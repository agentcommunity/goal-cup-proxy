//! The session-file reader (PRD-04): one low-priority thread that walks each native row's root,
//! reads the harness's session files raw and copies its databases, with no symlink followed.
//! Database sets (`snapshot.rs`) come in the next slice.

pub mod fsx;
pub mod pattern;
pub mod tail;
pub mod walk;

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::connector::{Form, NativeSources};
use crate::pipeline::{Mono, Recorder};
use tail::{Stop, Tail};
use walk::{Row, Walker};

const SLICE: u64 = 64 << 20;
const SETTLE: Duration = Duration::from_secs(1);
const ROOM_WAIT: Duration = Duration::from_millis(100);
const PASS: Duration = Duration::from_secs(1);
const MISSING_AFTER: Duration = Duration::from_secs(300);

/// What `finish` sends the thread: `harness_exited` and the seal's time guard.
type Finish = (bool, Option<Duration>);

pub struct NativeReader {
    tx: Sender<Finish>,
    thread: Option<JoinHandle<NativeFinish>>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NativeFinish {
    pub streams: u64,
    pub snapshots: u64,
    pub cut_files: u64,
    pub cut_bytes: u64,
    pub took_ms: u64,
}

/// Spawns the reader thread (section 1).
pub fn start(rec: Recorder, sources: NativeSources) -> NativeReader {
    let (tx, rx) = mpsc::channel();
    let notes: Vec<Value> = sources.0.iter().map(|s| missing_note(&s.name)).collect();
    let r = rec.clone();
    let body = move || {
        crate::pipeline::low_priority();
        Reader::new(rec, &sources).run(&rx)
    };
    let thread = std::thread::Builder::new().spawn(body).ok();
    if thread.is_none() {
        // No thread reads anything: every row is named, never a silent loss.
        notes.into_iter().for_each(|n| r.event(n));
    }
    NativeReader { tx, thread }
}

impl NativeReader {
    /// Section 8, run on the native thread. `guard`: `Some(180 s)` for the seal reasons
    /// `deadline`, `server_stop` and `signal`, else `None`. A panicked thread gives the default.
    pub fn finish(self, harness_exited: bool, guard: Option<Duration>) -> NativeFinish {
        let _ = self.tx.send((harness_exited, guard));
        let joined = self.thread.map(JoinHandle::join);
        joined.and_then(Result::ok).unwrap_or_default()
    }
}

struct Reader {
    rec: Recorder,
    rows: Vec<Row>,
    /// Per row: read as a snapshot (`final` rows are read as single snapshot files until
    /// `snapshot.rs` copies database sets).
    snap: Vec<bool>,
    walker: Walker,
    tail: Tail,
    since: i64,
    missing: Missing,
}

impl Reader {
    fn new(rec: Recorder, sources: &NativeSources) -> Reader {
        let since = sources.0.iter().map(|s| s.since).min();
        let since = since.unwrap_or(UNIX_EPOCH);
        let (mut rows, mut snap, mut missing) = (Vec::new(), Vec::new(), Missing::new(sources));
        for s in &sources.0 {
            // PRD-11's loader refuses a bad pattern; a row that still fails is named at once.
            let Ok(pattern) = pattern::parse(&s.pattern) else {
                rec.event(missing_note(&s.name));
                missing.took(&s.name);
                continue;
            };
            let (name, root) = (s.name.clone(), s.root.clone());
            rows.push(Row { name, root, pattern });
            snap.push(s.form != Form::Append);
        }
        let secs = since.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        Reader {
            tail: Tail::new(since, rec.native_cursors()),
            since: i64::try_from(secs).unwrap_or(i64::MAX),
            missing,
            walker: Walker::default(),
            rec,
            rows,
            snap,
        }
    }

    /// A walk every max(1 s, 50 times its own time), so it uses at most 2% of one core; reads
    /// between walks until the next is due, so a new file never waits behind a large one.
    fn run(mut self, rx: &Receiver<Finish>) -> NativeFinish {
        loop {
            let began = self.rec.now();
            self.pass();
            let next = PASS.max(elapsed(began, self.rec.now()) * 50);
            let left = |rec: &Recorder| next.saturating_sub(elapsed(began, rec.now()));
            loop {
                let wait = match self.tail.read(&self.rec, SLICE, SETTLE) {
                    Stop::Done => break,
                    Stop::Slice => Duration::ZERO,
                    Stop::Room => ROOM_WAIT,
                };
                if left(&self.rec).is_zero() {
                    break;
                }
                if let Some(f) = wait_for(rx, wait.min(left(&self.rec))) {
                    return self.finish(f);
                }
            }
            if let Some(f) = wait_for(rx, left(&self.rec)) {
                return self.finish(f);
            }
        }
    }

    /// One full pass: walk, settle each file under its path ID, sweep the gone.
    fn pass(&mut self) {
        let walked = self.walker.pass(&self.rec, &self.rows);
        for f in walked.found {
            let row = &self.rows[f.row];
            if tail::taken(&f.stat, self.since) {
                self.missing.took(&row.name);
            }
            let id = pattern::path_id(&row.name, &f.rel);
            let snap = self.snap[f.row];
            self.tail.settle(&self.rec, id, &row.root, &f.rel, f.stat, snap);
        }
        self.tail.sweep(&self.rec, &walked.unlisted);
        let requests = self.rec.counts().model_requests();
        self.missing.check(&self.rec, requests, MISSING_AFTER, self.rec.now());
    }

    /// Section 8 steps 1, 2, 4 and 5. `harness_exited` is for step 3, the seal's database copies
    /// in `snapshot.rs` (next slice); until then `snapshots` stays 0.
    fn finish(mut self, (_harness_exited, guard): Finish) -> NativeFinish {
        let began = self.rec.now();
        let left = |rec: &Recorder| guard.map(|g| g.saturating_sub(elapsed(began, rec.now())));
        self.pass();
        let cut = loop {
            if left(&self.rec).is_some_and(|l| l.is_zero()) {
                break true;
            }
            match self.tail.read(&self.rec, SLICE, Duration::ZERO) {
                Stop::Done => break false,
                Stop::Slice => {}
                Stop::Room => std::thread::sleep(left(&self.rec).map_or(ROOM_WAIT, |l| l.min(ROOM_WAIT))),
            }
        };
        let c = self.tail.close(&self.rec, cut);
        NativeFinish {
            streams: c.streams,
            snapshots: 0,
            cut_files: c.cut_files,
            cut_bytes: c.cut_bytes,
            took_ms: elapsed(began, self.rec.now()).as_millis() as u64,
        }
    }
}

/// Waits up to `d` for `finish`, holding nothing. A reader dropped without `finish` is finished
/// with no time left, so what is unread is named by a GAP.
fn wait_for(rx: &Receiver<Finish>, d: Duration) -> Option<Finish> {
    match rx.recv_timeout(d) {
        Ok(f) => Some(f),
        Err(RecvTimeoutError::Timeout) => None,
        Err(RecvTimeoutError::Disconnected) => Some((false, Some(Duration::ZERO))),
    }
}

fn elapsed(from: Mono, to: Mono) -> Duration {
    Duration::from_nanos(to.ns().saturating_sub(from.ns()))
}

fn missing_note(row: &str) -> Value {
    json!({"ev": "note", "what": "native_missing", "row": row})
}

/// Rows that find nothing (section 2): each `warn_missing` row that has taken no file `after` the
/// first time `requests` was seen above 0 is named once. The reader checks each pass with
/// `rec.counts().model_requests()`, 5 minutes and `rec.now()`.
pub struct Missing {
    waiting: Vec<String>,
    first: Option<Mono>,
}

impl Missing {
    pub fn new(sources: &NativeSources) -> Missing {
        let warn = sources.0.iter().filter(|s| s.warn_missing);
        let waiting = warn.map(|s| s.name.clone()).collect();
        Missing { waiting, first: None }
    }

    /// A file of row `name` was taken: the row is never named.
    pub fn took(&mut self, name: &str) {
        self.waiting.retain(|w| w != name);
    }

    pub fn check(&mut self, rec: &Recorder, requests: u64, after: Duration, now: Mono) {
        if requests > 0 {
            self.first.get_or_insert(now);
        }
        if self.first.is_some_and(|f| elapsed(f, now) >= after) {
            self.waiting.drain(..).for_each(|w| rec.event(missing_note(&w)));
        }
    }
}

/// The notes already written, by event text: each note is written once per boot.
#[derive(Default)]
pub(crate) struct Noted(HashSet<String>);

impl Noted {
    pub(crate) fn once(&mut self, rec: &Recorder, ev: Value) {
        if self.0.insert(ev.to_string()) {
            rec.event(ev);
        }
    }
}
