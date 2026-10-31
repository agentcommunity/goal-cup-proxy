//! The uploader (PRD-06): sends every part file to the server in small chunks soon after it is
//! written, tells the server the written counts in a heartbeat, and frees local disk once the
//! server holds the bytes. It runs on two plain threads with a blocking client, never on the
//! recorder's runtime, and never ends the process.
//!
//! Present: the seams ([`ServerWord`] S10, [`spawn`], [`UploadCtx`], [`SealFacts`] S18,
//! `upload.json` S9), the timing rules ([`policy`]), the client, the chunk cut ([`chunk`]), the
//! heartbeat ([`beat`]), the sender's rounds ([`send`]) and the two threads: `goalcup-upload` runs
//! the rounds and `goalcup-beat` the heartbeat, each on a 1 s tick on boot time, each round and beat
//! under [`guard`], and both write the shared `upload.json` at most once a second. In null mode
//! (no live writer) they send what is on disk and beat `failed` (ledger R7). [`UploadHandle::finish`]
//! runs the wrap-up: final heartbeat, then chunks, then the final again until sealed.
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

/// The running uploader.
pub struct UploadHandle {
    word: watch::Receiver<Option<ServerWord>>,
    /// None when no thread runs (a local run, or nothing to send with).
    run: Option<(Arc<Shared>, Vec<mpsc::Sender<Wrap>>, Vec<JoinHandle<()>>)>,
}

/// What `finish` hands both threads.
type Wrap = (WrapUp, SealFacts);

/// A thread's body.
type Body = fn(&Shared, &Make, Option<Recorder>, Receiver<Wrap>);

/// The threads' tick, on boot time.
const TICK: Duration = Duration::from_secs(1);

/// What the two threads share.
struct Shared {
    dir: PathBuf,
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
        let j = lock(&self.json).clone();
        let _ = file.put(Mono::now(), &j, force);
    }
}

/// How a thread builds its sender or beat again after a panic.
#[derive(Clone)]
struct Make(Policy, ServerClient, String, String, PathBuf);

impl Make {
    fn build<T>(&self, f: fn(&Policy, ServerClient, &str, &str, &Path) -> T) -> T {
        f(&self.0, self.1.clone(), &self.2, &self.3, &self.4)
    }
}

/// Starts the uploader for the entry in `ctx`. Never panics and never fails: an unwritable folder
/// is ignored. A restart keeps what an earlier `upload.json` confirmed, sent and pruned.
pub fn spawn(ctx: UploadCtx) -> UploadHandle {
    let (word, seen) = watch::channel(None);
    let mut handle = UploadHandle { word: seen, run: None };
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
    if server.as_ref().is_some_and(|s| s.server.is_none()) {
        (json.state, json.keep) = (State::Local, true);
        let _ = state::write(&ctx.entry_dir, &json);
        return handle;
    }
    let cred = std::fs::read_to_string(cli::credential(&ctx.run_dir)).unwrap_or_default();
    let ready = server.zip(json.server_url.clone());
    let Some((server, url)) = ready.filter(|_| !cred.trim().is_empty()) else {
        let text = match json.server_url {
            Some(_) => "the run credential is missing",
            None => "server.json unreadable",
        };
        json.last_error = Some(LastError::new(SystemTime::now(), Class::Internal, None, text));
        let _ = state::write(&ctx.entry_dir, &json);
        return handle;
    };
    // The server's intake ends a day after the run's deadline (section 9).
    let tail = |t: u64| state::rfc3339(UNIX_EPOCH + Duration::from_secs(t.saturating_add(86_400)));
    json.upload_by = json
        .upload_by
        .or_else(|| server.deadline.as_deref().and_then(cli::unix_of).map(tail));
    let client = ServerClient::new(&url, Some(Token::new(cred.trim())));
    let make = Make(Policy::from_env(), client, ctx.run, ctx.entry, ctx.entry_dir.clone());
    let sh = Arc::new(Shared {
        dir: ctx.entry_dir.clone(),
        json: Mutex::new(json),
        file: Mutex::new(state::Writer::new(ctx.entry_dir)),
        word,
        tried: AtomicBool::new(false),
    });
    let (mut txs, mut joins) = (Vec::new(), Vec::new());
    let threads: [(&str, Body); 2] = [("goalcup-upload", upload_thread), ("goalcup-beat", beat_thread)];
    for (name, body) in threads {
        let (tx, rx) = mpsc::channel();
        let (s, make, rec) = (sh.clone(), make.clone(), ctx.recorder.clone());
        let started = thread::Builder::new().name(name.into()).spawn(move || {
            pipeline::low_priority();
            body(&s, &make, rec, rx);
        });
        if let Ok(j) = started {
            (txs.push(tx), joins.push(j));
        } else {
            let text = "a thread of the uploader could not start";
            lock(&sh.json).last_error = Some(LastError::new(SystemTime::now(), Class::Internal, None, text));
        }
    }
    sh.put(true);
    handle.run = Some((sh, txs, joins));
    handle
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
        let text = "the uploader met an internal error";
        let mut j = lock(j);
        j.state = State::Waiting;
        j.last_error = Some(LastError::new(SystemTime::now(), Class::Internal, None, text));
        send::log_failure(entry_dir, what, Class::Internal.word(), None, wait);
    }
    out
}

/// The upload thread: a round on each tick, bounded by the durable point (live) or by what is on
/// disk (null mode). After the seal it waits for the first final heartbeat, then sends back to
/// back.
fn upload_thread(sh: &Shared, make: &Make, rec: Option<Recorder>, rx: Receiver<Wrap>) {
    let mut s = make.build(Sender::new);
    let durable = rec.as_ref().map(Recorder::durable);
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
                s = make.build(Sender::new);
                s.backoff = backoff;
            }
        }
        sh.put(false);
    }
}

/// The beat thread: an ordinary beat when due; after the seal the final heartbeat, then the final
/// again whenever its backoff is due once the last chunk is stored, until sealed.
fn beat_thread(sh: &Shared, make: &Make, rec: Option<Recorder>, rx: Receiver<Wrap>) {
    let mut b = make.build(Beat::new);
    let (durable, rec_dir) = (rec.as_ref().map(Recorder::durable), sh.dir.join("rec"));
    let (mut fin, mut link) = (None::<Wrap>, None::<IndexLine>);
    while let Some(w) = next(&rx) {
        fin = w.or(fin);
        let now = Mono::now();
        let j = lock(&sh.json).clone();
        let mut backoff = b.backoff;
        let one = || match &fin {
            None => match (&rec, &durable) {
                (Some(r), Some(d)) => {
                    let d = d.borrow().clone();
                    let h = health(Some(&r.live()), &sh.dir, &d.totals);
                    b.tick(now, &d, &h, j.waiting_bytes, &sh.json, &sh.word);
                }
                _ => {
                    // Null mode (ledger R7): the point and totals on disk, read only when due.
                    let none = EntryTotals::default();
                    if !b.due(now, recording(&health(None, &sh.dir, &none), &none)) {
                        return;
                    }
                    let totals = pipeline::entry_totals(&sh.dir);
                    let h = health(None, &sh.dir, &totals);
                    let point = last_on_disk(&rec_dir).ok().flatten();
                    let point = point.map(|line| goalcup_format::Durable {
                        part: String::new(),
                        index_len: 0,
                        line,
                    });
                    let d = Durable { point, totals, at: now };
                    b.tick(now, &d, &h, j.waiting_bytes, &sh.json, &sh.word);
                }
            },
            Some((_, seal)) => {
                // The seal LINK, the last `E` or `R` line, read once.
                link = link.or_else(|| match &durable {
                    Some(d) => d.borrow().point.as_ref().map(|p| p.line),
                    None => last_on_disk(&rec_dir).ok().flatten(),
                });
                let Some(point) = &link else { return };
                let stored = j.confirmed.as_ref().is_some_and(|c| c.raw >= point.raw_end);
                let again = !sh.tried.load(SeqCst) || (stored && b.backoff.due(now));
                if j.final_ != Final::Sealed && again {
                    let live = rec.as_ref().map_or_else(Live::default, Recorder::live);
                    let waiting_bytes = j.waiting_bytes;
                    let f = FinalFacts { point, seal, live: &live, waiting_bytes };
                    b.final_beat(now, &f, &sh.json, &sh.word);
                }
            }
        };
        if guard("beat", &sh.dir, &sh.json, &mut backoff, one).is_none() {
            b = make.build(Beat::new);
            b.backoff = backoff;
        }
        if fin.is_some() {
            sh.tried.store(true, SeqCst);
        }
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
    /// of `upload.json`; both threads stop at their next tick.
    pub fn finish(self, wrap_up: WrapUp, seal: SealFacts) {
        let Some((sh, txs, joins)) = self.run else {
            return;
        };
        for tx in &txs {
            let _ = tx.send((wrap_up, seal.clone()));
        }
        let done = |j: &UploadJson| j.final_ == Final::Sealed && j.state == State::Done;
        while !done(&lock(&sh.json)) && !wrap_up.over(Mono::now()) {
            thread::sleep(TICK / 20);
        }
        sh.put(true);
        drop(txs);
        // A thread in a call is left to end alone: the bound guards the harness's exit.
        let until = policy::after(Mono::now(), 5 * TICK);
        for j in joins {
            while !j.is_finished() && Mono::now() < until {
                thread::sleep(TICK / 100);
            }
            if j.is_finished() {
                let _ = j.join();
            }
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
