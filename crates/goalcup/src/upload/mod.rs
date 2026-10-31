//! The uploader (PRD-06): sends every part file to the server in small chunks soon after it is
//! written, tells the server the written counts in a heartbeat, and frees local disk once the
//! server holds the bytes. It runs on two plain threads with a blocking client, never on the
//! recorder's runtime, and never ends the process.
//!
//! Present: the seams ([`ServerWord`] S10, [`spawn`], [`UploadCtx`], [`SealFacts`] S18,
//! `upload.json` S9), the timing rules ([`policy`]), the client, the chunk cut ([`chunk`]), the
//! heartbeat ([`beat`]), the sender's rounds ([`send`]) and the two threads: `goalcup-upload` runs
//! the rounds and `goalcup-beat` the heartbeat, each on a 1 s tick on boot time, each whole
//! thread under [`guard`], and both write the shared `upload.json` at most once a second. In null
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
#[must_use = "dropping the handle stops the uploader"]
pub struct UploadHandle {
    word: watch::Receiver<Option<ServerWord>>,
    /// Keeps the channel open while the handle lives, also when no thread runs.
    _open: Arc<watch::Sender<Option<ServerWord>>>,
    /// None when no thread runs (a local run, or nothing to send with).
    run: Option<Run>,
}

/// The shared state, and each thread with the sender `finish` wakes it by.
type Run = (Arc<Shared>, Vec<(mpsc::Sender<()>, JoinHandle<()>)>);

/// What `finish` hands both threads.
type Wrap = (WrapUp, SealFacts);

/// The threads' tick, on boot time.
const TICK: Duration = Duration::from_secs(1);

/// The upload thread's name; the beat thread is `goalcup-beat`.
const UP: &str = "goalcup-upload";

/// What the two threads share.
struct Shared {
    p: Policy,
    client: ServerClient,
    ids: (String, String),
    dir: PathBuf,
    rec: Option<Recorder>,
    json: Mutex<UploadJson>,
    file: Mutex<state::Writer>,
    word: Arc<watch::Sender<Option<ServerWord>>>,
    /// Set by `finish`; kept here so a thread started again after a panic still has it.
    wrap: Mutex<Option<Wrap>>,
    /// The first final heartbeat returned, or the beat thread is gone: wrap-up chunks may go.
    tried: AtomicBool,
}

impl Shared {
    /// Writes `upload.json` at most once a second unless `force` (section 11), from a copy taken
    /// under the file lock; the shared state is not held across the disk write.
    fn put(&self, force: bool) {
        let mut file = lock(&self.file);
        let j = lock(&self.json).clone();
        let _ = file.put(Mono::now(), &j, force);
    }

    /// A new sender or beat (`Sender::new`, `Beat::new`).
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
    let word = Arc::new(word);
    UploadHandle {
        word: seen,
        _open: word.clone(),
        run: start(ctx, word),
    }
}

/// The threads of [`spawn`]; None when none starts, `upload.json` says why.
fn start(ctx: UploadCtx, word: Arc<watch::Sender<Option<ServerWord>>>) -> Option<Run> {
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
    let (server, url) = match (server, json.server_url.clone()) {
        (Some(s), Some(url)) if !cred.trim().is_empty() => (s, url),
        other => {
            match other {
                (Some(_), None) => (json.state, json.keep) = (State::Local, true),
                (Some(_), Some(_)) => json.last_error = internal("no run credential"),
                (None, _) => json.last_error = internal("server.json unreadable"),
            }
            let _ = state::write(&ctx.entry_dir, &json);
            return None;
        }
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
        wrap: Mutex::new(None),
        tried: AtomicBool::new(false),
    });
    let up = launch(&sh, true, upload_thread);
    let beat = launch(&sh, false, beat_thread);
    let threads: Vec<_> = up.into_iter().chain(beat).collect();
    sh.put(true);
    // With no thread, `finish` has nothing to wait for.
    (!threads.is_empty()).then_some((sh, threads))
}

/// The upload thread (`round`) or the beat thread on `body`, under [`guard`]; None when it could
/// not start, which `upload.json` says.
fn launch(
    sh: &Arc<Shared>,
    round: bool,
    body: fn(&Shared, &Receiver<()>),
) -> Option<(mpsc::Sender<()>, JoinHandle<()>)> {
    let (tx, rx) = mpsc::channel();
    let s = sh.clone();
    let name = if round { UP } else { "goalcup-beat" };
    let started = thread::Builder::new().name(name.into()).spawn(move || {
        pipeline::low_priority();
        guard(round, &s.dir, &s.json, &s.p, &rx, || body(&s, &rx));
        // A beat thread that is gone never holds the wrap-up chunks back.
        s.tried.store(true, SeqCst);
    });
    if started.is_err() {
        sh.tried.store(true, SeqCst);
        lock(&sh.json).last_error = internal("an uploader thread could not start");
    }
    started.ok().map(|h| (tx, h))
}

/// Waits one tick, or less when `finish` wakes the thread. False once `finish` let go.
fn next(rx: &Receiver<()>) -> bool {
    !matches!(rx.recv_timeout(TICK), Err(RecvTimeoutError::Disconnected))
}

/// Runs `body`, the whole thread, until it returns, which it does once `wake` is let go. A panic
/// anywhere in it never ends the process or the thread and is never raised again: a backoff fails,
/// one `goalcup.log` line says class `internal`, and for the upload thread (`round`) `upload.json`
/// says `waiting` with that class. Once the backoff is due `body` starts again and builds its
/// sender or beat anew, so no half-changed state is kept.
pub fn guard(
    round: bool,
    entry_dir: &Path,
    j: &Mutex<UploadJson>,
    p: &Policy,
    wake: &Receiver<()>,
    mut body: impl FnMut(),
) {
    let mut backoff = Backoff::new(p);
    while catch_unwind(AssertUnwindSafe(&mut body)).is_err() {
        let wait = backoff.fail(Mono::now(), None);
        if round {
            let mut j = lock(j);
            j.state = State::Waiting;
            j.last_error = internal("an internal error in the uploader");
        }
        let what = if round { "round" } else { "beat" };
        send::log_failure(entry_dir, what, Class::Internal.word(), None, wait);
        while !backoff.due(Mono::now()) {
            if !next(wake) {
                return;
            }
        }
    }
}

/// The upload thread: a round on each tick, bounded by the durable point (live) or by what is on
/// disk (null mode). After the seal it waits for the first final heartbeat, then sends back to
/// back.
fn upload_thread(sh: &Shared, rx: &Receiver<()>) {
    let mut s = sh.build(Sender::new);
    let durable = sh.rec.as_ref().map(Recorder::durable);
    while next(rx) {
        let wrap = lock(&sh.wrap).is_some();
        // Copied out, and the borrow dropped at once: the writer never waits on this thread.
        let bound = match &durable {
            Some(d) => d.borrow().point.clone().map(Bound::Durable),
            None => Some(Bound::OnDisk),
        };
        if let Some(b) = bound.filter(|_| !wrap || sh.tried.load(SeqCst)) {
            while s.tick(&Mono::now, &b, wrap, &sh.json) == Step::Again {}
        }
        sh.put(false);
    }
}

/// The beat thread: an ordinary beat when due; after the seal the final heartbeat, then the final
/// again whenever its backoff is due while it is not committed or once the last chunk is stored,
/// until sealed.
fn beat_thread(sh: &Shared, rx: &Receiver<()>) {
    let mut b = sh.build(Beat::new);
    let (durable, rec_dir) = (sh.rec.as_ref().map(Recorder::durable), sh.dir.join("rec"));
    let mut link = None::<IndexLine>;
    // Null mode: no writer runs, so the point and totals on disk are read once, when a beat is
    // first due (`entry_totals` decodes every part file).
    let mut on_disk = None::<Durable>;
    while next(rx) {
        let fin = lock(&sh.wrap).clone();
        let now = Mono::now();
        let (waiting, raw, final_) = {
            let j = lock(&sh.json);
            let raw = j.confirmed.as_ref().map(|c| c.raw);
            (j.waiting_bytes, raw, j.final_)
        };
        match (&fin, &sh.rec, &durable) {
            (None, Some(r), Some(d)) => {
                let d = d.borrow().clone();
                let h = health(Some(&r.live()), &sh.dir, &d.totals);
                b.tick(now, &d, &h, waiting, &sh.json, &sh.word);
            }
            // Null mode (ledger R7).
            (None, ..) => {
                let none = EntryTotals::default();
                if b.due(now, recording(&health(None, &sh.dir, &none), &none)) {
                    // Only the line and the totals are read by the beat; `part` and `index_len`
                    // are placeholders.
                    on_disk = on_disk.or_else(|| {
                        let line = last_on_disk(&rec_dir).ok().flatten()?;
                        let point = goalcup_format::Durable {
                            part: String::new(),
                            index_len: 0,
                            line,
                        };
                        let totals = pipeline::entry_totals(&sh.dir);
                        Some(Durable {
                            point: Some(point),
                            totals,
                            at: now,
                        })
                    });
                    if let Some(d) = &on_disk {
                        let h = health(None, &sh.dir, &d.totals);
                        b.tick(now, d, &h, waiting, &sh.json, &sh.word);
                    }
                }
            }
            (Some((_, seal)), ..) => {
                // After `finish` the writer has sealed and synced, so the last line on disk is the
                // seal LINK in both modes; read once.
                link = link.or_else(|| last_on_disk(&rec_dir).ok().flatten());
                if let Some(point) = &link {
                    let stored = raw.is_some_and(|raw| raw >= point.raw_end);
                    // A final not committed goes again on its backoff: past the run end the server
                    // takes no chunk before one is (503 awaiting_final).
                    let due = b.backoff.due(now) && (stored || final_ == Final::None);
                    if final_ != Final::Sealed && (!sh.tried.load(SeqCst) || due) {
                        let live = sh.rec.as_ref().map_or_else(Live::default, Recorder::live);
                        let f = FinalFacts {
                            point,
                            seal,
                            live: &live,
                            waiting_bytes: waiting,
                        };
                        b.final_beat(now, &f, &sh.json, &sh.word);
                    }
                }
            }
        }
        sh.tried.fetch_or(fin.is_some() && link.is_some(), SeqCst);
        sh.put(false);
    }
}

impl UploadHandle {
    /// The last heartbeat answer; None before the first one. With no uploader thread (a local
    /// run, or nothing to send with) the value stays None and `changed()` never returns while the
    /// handle lives.
    pub fn server_word(&self) -> watch::Receiver<Option<ServerWord>> {
        self.word.clone()
    }

    /// The seal is durable: final heartbeat, then the chunks left, then the final again until
    /// sealed (section 6), until `upload.json` says `done`, `wrap_up` is over or no upload thread
    /// runs. Both threads then stop at their next tick, waited for a few seconds at most (a thread
    /// inside a call ends alone), and `upload.json` is written a last time with what they did.
    pub fn finish(self, wrap_up: WrapUp, seal: SealFacts) {
        let Some((sh, threads)) = self.run else {
            return;
        };
        *lock(&sh.wrap) = Some((wrap_up, seal));
        for (tx, _) in &threads {
            let _ = tx.send(());
        }
        let done = |j: &UploadJson| j.final_ == Final::Sealed && j.state == State::Done;
        let up = |h: &JoinHandle<()>| h.thread().name() == Some(UP) && !h.is_finished();
        while !done(&lock(&sh.json))
            && !wrap_up.over(Mono::now())
            && threads.iter().any(|t| up(&t.1))
        {
            thread::sleep(TICK / 20);
        }
        // Dropping the senders stops the threads.
        let joins: Vec<_> = threads.into_iter().map(|t| t.1).collect();
        let until = policy::after(Mono::now(), 5 * TICK);
        while !joins.iter().all(JoinHandle::is_finished) && Mono::now() < until {
            thread::sleep(TICK / 100);
        }
        sh.put(true);
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
        let _h = spawn(ctx);
        let j = state::read(dir.path()).unwrap();
        // Valid JSON that is not a ServerJson: the unreadable branch, not the URL one.
        assert_eq!(j.server_url, None);
        assert_eq!((j.state, j.keep), (State::Waiting, false));
        assert_eq!(j.last_error.unwrap().class, policy::Class::Internal);
    }
}
