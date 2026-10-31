//! The uploader (PRD-06): sends every part file to the server in small chunks soon after it is
//! written, tells the server the written counts in a heartbeat, and frees local disk once the
//! server holds the bytes. It runs on two plain threads with a blocking client, never on the
//! recorder's runtime, and never ends the process.
//!
//! Present: the seams ([`ServerWord`] S10, [`spawn`], [`UploadCtx`], [`SealFacts`] S18,
//! `upload.json` S9), the timing rules ([`policy`]), the client, the chunk cut ([`chunk`]), the
//! heartbeat ([`beat`]), the sender's rounds ([`send`]) and the two threads: `goalcup-upload` runs
//! the rounds and `goalcup-beat` the heartbeat, each on a 1 s tick on boot time, each round and
//! beat under [`guard`], and both write the shared `upload.json` at most once a second. In null
//! mode (no live writer) they send what is on disk and beat `failed` (ledger R7).
//! [`UploadHandle::finish`] runs the wrap-up: final heartbeat, then chunks, then the final again
//! until sealed.
//!
//! Still open: `prune.rs`, `upload.lock`, the queue of older entries, `manual.rs` (`goalcup
//! upload`) and the full-tier `upload_worker.rs`.

pub mod beat;
pub mod chunk;
pub mod client;
pub mod policy;
pub mod send;
pub mod state;

pub use policy::WrapUp;

use crate::cli::{self, ServerJson};
use crate::pipeline::{self, Durable, EntryTotals, Live, Mono, Recorder};
use crate::watchdog::SealReason;
use beat::{Beat, FinalFacts, health, recording};
use chunk::{Bound, last_on_disk};
use client::{ServerClient, Token};
use goalcup_format::IndexLine;
use policy::{Backoff, Class, Policy};
use send::{Sender, Step, lock};
use serde::{Deserialize, Serialize};
use state::{Final, LastError, State, UploadJson};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

/// The last heartbeat answer (seam S10). The recorder forwards it to the watchdog as the `server`
/// control line; the watchdog resets its deadline timer from `seconds_left` and seals with reason
/// `server_stop` on `stop`. The beat publishes it with `send_replace`, which keeps the value while
/// no receiver exists, so a subscriber that comes later still sees the last word.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerWord {
    pub stop: bool,
    pub reason: Option<String>,
    pub seconds_left: Option<u64>,
    /// Boot time, taken before the heartbeat call, so a wall clock set back never lengthens the
    /// deadline (PRD-17 S7).
    pub asked_at: Mono,
}

/// What the uploader needs to start (seam S18).
pub struct UploadCtx {
    /// `$GOALCUP_HOME`, for the queue of older entries.
    pub home: PathBuf,
    /// `runs/<run>/`, with `server.json` and the run credential (PRD-10).
    pub run_dir: PathBuf,
    /// The entry folder: `upload.json`, `upload.lock`, `rec/`.
    pub entry_dir: PathBuf,
    pub run: String,
    pub entry: String,
    /// None in null mode.
    pub recorder: Option<Recorder>,
    /// `GOALCUP_KEEP_PARTS` from the start facts: nothing is pruned.
    pub keep: bool,
}

/// What the seal hands over (seam S18).
#[derive(Clone, Debug)]
pub struct SealFacts {
    pub reason: SealReason,
    pub workspace_head: Option<String>,
    pub totals: EntryTotals,
}

/// The running uploader. Dropping it without `finish` stops both threads at their next tick.
pub struct UploadHandle {
    word: watch::Receiver<Option<ServerWord>>,
    /// None when no thread runs (a local run, or nothing to send with).
    run: Option<Run>,
}

/// The shared state, and each thread with the sender `finish` wakes it by.
type Run = (Arc<Shared>, Vec<(mpsc::Sender<Wrap>, JoinHandle<()>)>);

/// What `finish` hands both threads.
type Wrap = (WrapUp, SealFacts);

/// A thread's body.
type Body = fn(&Shared, Receiver<Wrap>);

/// The threads' tick, on boot time.
const TICK: Duration = Duration::from_secs(1);

/// What the two threads share.
struct Shared {
    p: Policy,
    client: ServerClient,
    ids: (String, String),
    dir: PathBuf,
    rec: Option<Recorder>,
    json: Mutex<UploadJson>,
    file: Mutex<state::Writer>,
    word: watch::Sender<Option<ServerWord>>,
    /// The first final heartbeat returned, answered or failed: wrap-up chunks may go.
    tried: AtomicBool,
}

impl Shared {
    /// Writes `upload.json` at most once a second unless `force` (section 11).
    fn put(&self, force: bool) {
        let mut file = lock(&self.file);
        let _ = file.put(Mono::now(), &lock(&self.json).clone(), force);
    }

    /// A new sender or beat (`Sender::new`, `Beat::new`), also after a panic.
    fn build<T>(&self, f: fn(&Policy, ServerClient, &str, &str, &Path) -> T) -> T {
        let (run, entry) = &self.ids;
        f(&self.p, self.client.clone(), run, entry, &self.dir)
    }
}

/// A `last_error` of class `internal` in fixed words.
fn internal(text: &str) -> Option<LastError> {
    let class = Class::Internal;
    Some(LastError::new(SystemTime::now(), class, None, text))
}

/// Starts the uploader for the entry in `ctx`. Never panics and never fails: an unwritable folder
/// is ignored. A restart keeps what an earlier `upload.json` confirmed, sent and pruned.
pub fn spawn(ctx: UploadCtx) -> UploadHandle {
    let (word, seen) = watch::channel(None);
    UploadHandle {
        word: seen,
        run: start(ctx, word),
    }
}

/// The threads of [`spawn`]; None when none starts, `upload.json` says why.
fn start(ctx: UploadCtx, word: watch::Sender<Option<ServerWord>>) -> Option<Run> {
    let old = state::read(&ctx.entry_dir).unwrap_or_default();
    let mut json = UploadJson {
        keep: ctx.keep,
        upload_by: old.upload_by,
        confirmed: old.confirmed,
        sent: old.sent,
        final_: old.final_,
        pruned: old.pruned,
        server_word: old.server_word,
        ..UploadJson::new(ctx.run.clone(), ctx.entry.clone())
    };
    // Only a readable `server.json` that says off makes the run local; an unreadable one is not
    // read as "no server".
    let server = cli::read_json::<ServerJson>(&cli::server_json(&ctx.run_dir));
    json.server_url = server.as_ref().and_then(|s| s.server.clone());
    let cred = std::fs::read_to_string(cli::credential(&ctx.run_dir)).unwrap_or_default();
    let ready = server.as_ref().zip(json.server_url.clone());
    let Some((server, url)) = ready.filter(|_| !cred.trim().is_empty()) else {
        if server.is_some() && json.server_url.is_none() {
            (json.state, json.keep) = (State::Local, true);
        } else if server.is_some() {
            json.last_error = internal("no run credential");
        } else {
            json.last_error = internal("server.json unreadable");
        }
        let _ = state::write(&ctx.entry_dir, &json);
        return None;
    };
    // The server's intake ends a day after the run's deadline (section 9).
    let tail = |t: u64| state::rfc3339(UNIX_EPOCH + Duration::from_secs(t.saturating_add(86_400)));
    let deadline = server.deadline.as_deref().and_then(cli::unix_of);
    json.upload_by = json.upload_by.or_else(|| deadline.map(tail));
    let sh = Arc::new(Shared {
        p: Policy::from_env(),
        client: ServerClient::new(&url, Some(Token::new(cred.trim()))),
        ids: (ctx.run, ctx.entry),
        dir: ctx.entry_dir.clone(),
        rec: ctx.recorder,
        json: Mutex::new(json),
        file: Mutex::new(state::Writer::new(ctx.entry_dir)),
        word,
        tried: AtomicBool::new(false),
    });
    let mut run: Run = (sh.clone(), Vec::new());
    let bodies: [(&str, Body); 2] = [
        ("goalcup-upload", upload_thread),
        ("goalcup-beat", beat_thread),
    ];
    for (name, body) in bodies {
        let (tx, rx) = mpsc::channel();
        let s = sh.clone();
        let started = thread::Builder::new().name(name.into()).spawn(move || {
            pipeline::low_priority();
            body(&s, rx);
        });
        match started {
            Ok(j) => run.1.push((tx, j)),
            Err(_) => lock(&sh.json).last_error = internal("an uploader thread could not start"),
        }
    }
    sh.put(true);
    Some(run)
}

/// Waits one tick, or less when `finish` hands over the wrap-up. None once `finish` let go.
fn next(rx: &Receiver<Wrap>) -> Option<Option<Wrap>> {
    match rx.recv_timeout(TICK) {
        Ok(w) => Some(Some(w)),
        Err(RecvTimeoutError::Timeout) => Some(None),
        Err(RecvTimeoutError::Disconnected) => None,
    }
}

/// Runs one round or one beat. A panic in it never ends the process and is never raised again:
/// `backoff` fails, `upload.json` says `waiting` with class `internal`, and one `goalcup.log` line
/// says so. None after a panic; the caller then builds its sender or beat again with `backoff`, so
/// no half-changed state is kept.
pub fn guard<T>(
    what: &'static str,
    entry_dir: &Path,
    j: &Mutex<UploadJson>,
    backoff: &mut Backoff,
    f: impl FnOnce() -> T,
) -> Option<T> {
    let out = catch_unwind(AssertUnwindSafe(f)).ok();
    if out.is_none() {
        let wait = backoff.fail(Mono::now(), None);
        let mut j = lock(j);
        j.state = State::Waiting;
        j.last_error = internal("an internal error in the uploader");
        send::log_failure(entry_dir, what, Class::Internal.word(), None, wait);
    }
    out
}

/// The upload thread: a round on each tick, bounded by the durable point (live) or by what is on
/// disk (null mode). After the seal it waits for the first final heartbeat, then sends back to
/// back.
fn upload_thread(sh: &Shared, rx: Receiver<Wrap>) {
    let mut s = sh.build(Sender::new);
    let durable = sh.rec.as_ref().map(Recorder::durable);
    let mut wrap = false;
    while let Some(w) = next(&rx) {
        wrap |= w.is_some();
        // Copied out, and the borrow dropped at once: the writer never waits on this thread.
        let bound = match &durable {
            Some(d) => d.borrow().point.clone().map(Bound::Durable),
            None => Some(Bound::OnDisk),
        };
        if let Some(b) = bound.filter(|_| !wrap || sh.tried.load(SeqCst)) {
            let mut backoff = s.backoff;
            let round = || while s.tick(&Mono::now, &b, wrap, &sh.json) == Step::Again {};
            if guard("round", &sh.dir, &sh.json, &mut backoff, round).is_none() {
                (s, s.backoff) = (sh.build(Sender::new), backoff);
            }
        }
        sh.put(false);
    }
}

/// The beat thread: an ordinary beat when due; after the seal the final heartbeat, then the final
/// again whenever its backoff is due once the last chunk is stored, until sealed.
fn beat_thread(sh: &Shared, rx: Receiver<Wrap>) {
    let mut b = sh.build(Beat::new);
    let (durable, rec_dir) = (sh.rec.as_ref().map(Recorder::durable), sh.dir.join("rec"));
    let (mut fin, mut link) = (None::<Wrap>, None::<IndexLine>);
    while let Some(w) = next(&rx) {
        fin = w.or(fin);
        let (now, j) = (Mono::now(), lock(&sh.json).clone());
        let mut backoff = b.backoff;
        let one = || match (&fin, &sh.rec, &durable) {
            (None, Some(r), Some(d)) => {
                let d = d.borrow().clone();
                let h = health(Some(&r.live()), &sh.dir, &d.totals);
                b.tick(now, &d, &h, j.waiting_bytes, &sh.json, &sh.word);
            }
            // Null mode (ledger R7): the point and totals on disk, read only when a beat is due.
            (None, ..) => {
                let none = EntryTotals::default();
                if b.due(now, recording(&health(None, &sh.dir, &none), &none)) {
                    let mut d = Durable::default();
                    (d.totals, d.at) = (pipeline::entry_totals(&sh.dir), now);
                    d.point = last_on_disk(&rec_dir).ok().flatten().map(|line| {
                        let part = String::new();
                        goalcup_format::Durable {
                            part,
                            index_len: 0,
                            line,
                        }
                    });
                    let h = health(None, &sh.dir, &d.totals);
                    b.tick(now, &d, &h, j.waiting_bytes, &sh.json, &sh.word);
                }
            }
            (Some((_, seal)), ..) => {
                // The seal LINK, the last `E` or `R` line, read once.
                link = link.or_else(|| match &durable {
                    Some(d) => d.borrow().point.as_ref().map(|p| p.line),
                    None => last_on_disk(&rec_dir).ok().flatten(),
                });
                let Some(point) = &link else { return };
                let stored = j.confirmed.as_ref().is_some_and(|c| c.raw >= point.raw_end);
                let again = !sh.tried.load(SeqCst) || (stored && b.backoff.due(now));
                if j.final_ != Final::Sealed && again {
                    let live = sh.rec.as_ref().map_or_else(Live::default, Recorder::live);
                    let (live, waiting_bytes) = (&live, j.waiting_bytes);
                    let f = FinalFacts {
                        point,
                        seal,
                        live,
                        waiting_bytes,
                    };
                    b.final_beat(now, &f, &sh.json, &sh.word);
                }
            }
        };
        if guard("beat", &sh.dir, &sh.json, &mut backoff, one).is_none() {
            (b, b.backoff) = (sh.build(Beat::new), backoff);
        }
        sh.tried.fetch_or(fin.is_some(), SeqCst);
        sh.put(false);
    }
}

impl UploadHandle {
    /// The last heartbeat answer; None before the first one.
    pub fn server_word(&self) -> watch::Receiver<Option<ServerWord>> {
        self.word.clone()
    }

    /// The seal is durable: final heartbeat, then the chunks left, then the final again until
    /// sealed (section 6), until `upload.json` says `done` or `wrap_up` is over. Then a last write
    /// of `upload.json`; both threads stop at their next tick, waited for a few seconds at most (a
    /// thread inside a call ends alone).
    pub fn finish(self, wrap_up: WrapUp, seal: SealFacts) {
        let Some((sh, threads)) = self.run else {
            return;
        };
        for (tx, _) in &threads {
            let _ = tx.send((wrap_up, seal.clone()));
        }
        let done = |j: &UploadJson| j.final_ == Final::Sealed && j.state == State::Done;
        while !done(&lock(&sh.json)) && !wrap_up.over(Mono::now()) {
            thread::sleep(TICK / 20);
        }
        sh.put(true);
        // Dropping the senders stops the threads.
        let joins: Vec<_> = threads.into_iter().map(|t| t.1).collect();
        let until = policy::after(Mono::now(), 5 * TICK);
        while !joins.iter().all(JoinHandle::is_finished) && Mono::now() < until {
            thread::sleep(TICK / 100);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Point 6: a `server.json` that cannot be read never tells the participant the run is local.
    #[test]
    fn an_unreadable_server_json_is_not_local() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(cli::server_json(dir.path()), br#"{"server":"https://x"}"#).unwrap();
        let ctx = UploadCtx {
            home: dir.path().into(),
            run_dir: dir.path().into(),
            entry_dir: dir.path().into(),
            run: "r".into(),
            entry: "e".into(),
            recorder: None,
            keep: false,
        };
        spawn(ctx);
        let j = state::read(dir.path()).unwrap();
        // Valid JSON that is not a ServerJson: the unreadable branch, not the URL one.
        assert_eq!(j.server_url, None);
        assert_eq!((j.state, j.keep), (State::Waiting, false));
        assert_eq!(j.last_error.unwrap().class, policy::Class::Internal);
    }
}
