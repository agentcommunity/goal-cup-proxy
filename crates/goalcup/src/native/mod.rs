//! The session-file reader (PRD-04): one low-priority thread that walks each native row's root,
//! reads the harness's session files raw and copies its databases, with no symlink followed.

pub mod fsx;
pub mod pattern;
pub mod snapshot;
pub mod tail;
pub mod walk;

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::connector::{Form, NativeSources};
use crate::pipeline::{Mono, Recorder};
use snapshot::{Interval, Sets};
use tail::{Stop, Tail};
use walk::{Row, Walker};

const SLICE: u64 = 64 << 20;
const SETTLE: Duration = Duration::from_secs(1);
const ROOM_WAIT: Duration = Duration::from_millis(100);
const PASS: Duration = Duration::from_secs(1);
const MISSING_AFTER: Duration = Duration::from_secs(300);
const PERIODIC: Interval = Interval {
    base: Duration::from_secs(3600),
    per_2mib: Duration::from_secs(60),
};

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
    forms: Vec<Form>,
    walker: Walker,
    tail: Tail,
    sets: Sets,
    missing: Missing,
}

impl Reader {
    fn new(rec: Recorder, sources: &NativeSources) -> Reader {
        let since = sources.0.iter().map(|s| s.since).min();
        let since = since.unwrap_or(UNIX_EPOCH);
        let (mut rows, mut forms, mut missing) = (Vec::new(), Vec::new(), Missing::new(sources));
        for s in &sources.0 {
            // PRD-11's loader refuses a bad pattern; a row that still fails is named at once.
            let Ok(pattern) = pattern::parse(&s.pattern) else {
                rec.event(missing_note(&s.name));
                missing.took(&s.name);
                continue;
            };
            let (name, root) = (s.name.clone(), s.root.clone());
            rows.push(Row {
                name,
                root,
                pattern,
            });
            forms.push(s.form);
        }
        Reader {
            tail: Tail::new(since, rec.native_cursors()),
            sets: Sets::new(PERIODIC),
            missing,
            walker: Walker::default(),
            rec,
            rows,
            forms,
        }
    }

    /// A walk every max(1 s, 50 times its own time), so it uses at most 2% of one core; reads
    /// between walks until the next is due, so a new file never waits behind a large one.
    fn run(mut self, rx: &Receiver<Finish>) -> NativeFinish {
        loop {
            let began = self.rec.now();
            self.pass();
            let next = PASS.max(elapsed(began, self.rec.now()) * 50);
            loop {
                let read = (self.tail.read(&self.rec, SLICE, SETTLE), self.sets.read(&self.rec, SLICE));
                let wait = match read {
                    (Stop::Room, _) | (_, Stop::Room) => ROOM_WAIT,
                    (Stop::Slice, _) | (_, Stop::Slice) => Duration::ZERO,
                    _ => Duration::MAX,
                };
                let left = next.saturating_sub(elapsed(began, self.rec.now()));
                if let Some(f) = wait_for(rx, wait.min(left)) {
                    return self.finish(f);
                }
                if left <= wait {
                    break;
                }
            }
        }
    }

    /// One full pass: walk, settle each file under its path ID or its database set, sweep the gone.
    fn pass(&mut self) {
        let walked = self.walker.pass(&self.rec, &self.rows);
        for f in walked.found {
            let (row, form) = (&self.rows[f.row], self.forms[f.row]);
            let taken = self.tail.taken(&f.stat);
            if taken {
                self.missing.took(&row.name);
            }
            if form == Form::Final {
                self.sets.see(&row.name, &row.root, &f.rel, f.stat, taken);
                continue;
            }
            let id = pattern::path_id(&row.name, &f.rel);
            let snap = form == Form::Snapshot;
            self.tail
                .settle(&self.rec, id, &row.root, &f.rel, f.stat, snap);
        }
        self.sets.walked();
        self.tail.sweep(&self.rec, &walked.unlisted);
        let requests = self.rec.counts().model_requests();
        self.missing
            .check(&self.rec, requests, MISSING_AFTER, self.rec.now());
    }

    /// Section 8. Step 2 reads each snapshot as it is: a copy torn by the state the pass saw copies
    /// that state once; one torn by a newer state is not started again (its settle clock is past
    /// `seen`), so `close` names its loss. When the guard runs out, what is left is cut.
    fn finish(mut self, (harness_exited, guard): Finish) -> NativeFinish {
        let began = self.rec.now();
        self.pass();
        let (rec, seen) = (&self.rec, self.rec.now());
        let drain = |read: &mut dyn FnMut() -> Stop| loop {
            let left = guard.map(|g| g.saturating_sub(elapsed(began, rec.now())));
            if left.is_some_and(|l| l.is_zero()) {
                return true;
            }
            match read() {
                Stop::Done => return false,
                Stop::Slice => {}
                Stop::Room => std::thread::sleep(left.map_or(ROOM_WAIT, |l| l.min(ROOM_WAIT))),
            }
        };
        let cut = drain(&mut || self.tail.read(rec, SLICE, elapsed(seen, rec.now())));
        self.sets.seal(harness_exited);
        if !cut {
            drain(&mut || self.sets.read(rec, SLICE));
        }
        let (s, c) = (self.sets.close(rec), self.tail.close(rec, cut));
        NativeFinish {
            streams: c.streams,
            snapshots: s.streams,
            cut_files: c.cut_files + s.cut_files,
            cut_bytes: c.cut_bytes + s.cut_bytes,
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
        Missing {
            waiting,
            first: None,
        }
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
            self.waiting
                .drain(..)
                .for_each(|w| rec.event(missing_note(&w)));
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
