//! The sender of one entry (PRD-06 sections 3, 4, 5, 8 and 11): rounds of chunks from the server's
//! position, the backoff that resets on progress (run 1's lag bug), the 409 re-read, `stuck`,
//! `refused`, `503 awaiting_final` and the confirmed rule. One [`Sender::tick`] makes at most one
//! HTTP call; the upload thread (slice 4) calls it on its 1 s tick and at once on [`Step::Again`].

use super::chunk::{self, Bound, Chunk, Cut};
use super::client::{RunPosition, ServerClient, ServerError};
use super::policy::{Backoff, Class, Policy, Timer, after, class_of_status};
use super::state::{self, Confirmed, Final, LastError, State, UploadJson, rfc3339};
use crate::cli::log::{Fact, line};
use crate::pipeline::Mono;
use goalcup_format::hex;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// What the caller does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Call again at once: a round is open below its target, or a re-read is due.
    Again,
    /// Wait for the next tick.
    Idle,
}

/// The sender of one entry, with its own backoff.
pub struct Sender {
    p: Policy,
    client: ServerClient,
    run: String,
    entry: String,
    entry_dir: PathBuf,
    rec: PathBuf,
    timer: Timer,
    pub backoff: Backoff,
    /// The server's position; None until read, and after a failure or a 409.
    pos: Option<RunPosition>,
    /// The furthest offset the server has answered.
    known: u64,
    /// The raw end the open round runs to.
    target: Option<u64>,
    /// The known offset when a 409 came; its re-read must move past it.
    conflict: Option<u64>,
    /// The last chunk put: its raw end and compressed length, counted in `sent` once stored.
    inflight: Option<(u64, u64)>,
    refusals: u32,
    awaiting_final: bool,
    /// `(durable raw end, offset)` of the last measure, and its `(raw end, waiting bytes)`.
    measured: (Option<(u64, u64)>, (u64, u64)),
}

impl Sender {
    pub fn new(p: &Policy, client: ServerClient, run: &str, entry: &str, entry_dir: &Path) -> Self {
        Sender {
            p: *p,
            client,
            run: run.to_owned(),
            entry: entry.to_owned(),
            entry_dir: entry_dir.to_path_buf(),
            rec: entry_dir.join("rec"),
            timer: Timer::new(p.round, p.spread_first),
            backoff: Backoff::new(p),
            pos: None,
            known: 0,
            target: None,
            conflict: None,
            inflight: None,
            refusals: 0,
            awaiting_final: false,
            measured: (None, (0, 0)),
        }
    }

    /// One step. `bound` is `Durable` only while the live writer has a point, else `OnDisk`;
    /// `wrap_up` runs rounds back to back (section 4). The clock is read again after a call.
    pub fn tick(
        &mut self,
        clock: &dyn Fn() -> Mono,
        bound: &Bound,
        wrap_up: bool,
        j: &Mutex<UploadJson>,
    ) -> Step {
        let now = clock();
        if self.awaiting_final {
            // Retried at once once the final is committed; its Retry-After is never waited out.
            if lock(j).final_ == Final::None {
                return Step::Idle;
            }
            self.awaiting_final = false;
        } else if !self.backoff.due(now) {
            return Step::Idle;
        }
        let Some(pos) = self.pos.clone() else {
            return self.read_position(clock, j);
        };
        let (end, waiting) = self.measure(bound, pos.offset);
        self.update(j, |j| (j.durable_raw, j.waiting_bytes) = (end, waiting));
        if self.target.is_some_and(|t| pos.offset >= t) {
            self.caught_up(j);
        }
        if self.target.is_none() {
            if end <= pos.offset {
                self.caught_up(j);
                return Step::Idle;
            }
            if !(wrap_up || waiting >= self.p.trigger_bytes || self.timer.due(now)) {
                return Step::Idle;
            }
            self.timer.started(now);
            self.target = Some(end);
            self.update(j, |j| j.state = State::Sending);
        }
        let confirmed = lock(j).confirmed.clone();
        match chunk::cut(&self.rec, &pos, bound, confirmed.as_ref()) {
            Ok(Cut::Chunk(c)) => self.put(&c, clock, j),
            Ok(Cut::Nothing) => {
                self.caught_up(j);
                Step::Idle
            }
            Ok(Cut::Stuck { reason, offset }) => {
                // Nothing is sent across it; the round stays open for the re-read.
                self.pos = None;
                self.backoff.next_try = Some(after(now, self.p.stuck_reread));
                let why = (Class::Stuck, None, &*format!("{reason} (offset {offset})"));
                self.say(j, "chunk", State::Stuck, why, self.p.stuck_reread);
                Step::Idle
            }
            Err(_) => self.fail(clock(), "chunk", UNREAD, None, j),
        }
    }

    fn put(&mut self, c: &Chunk, clock: &dyn Fn() -> Mono, j: &Mutex<UploadJson>) -> Step {
        let len = c.c_end - c.c_start;
        let read = chunk::sha256(c)
            .and_then(|sha| Ok((chunk::headers(&self.entry, c, sha), chunk::body(c)?)));
        let Ok((h, body)) = read else {
            return self.fail(clock(), "chunk", UNREAD, None, j);
        };
        self.inflight = Some((c.raw_end, len));
        let answer = self.client.put_chunk(&self.run, &h, body, len);
        let now = clock();
        match answer {
            Ok(a) => {
                let ours =
                    a.offset == c.raw_end && a.next_seq == c.last_seq + 1 && a.head == h.head;
                let (offset, next_seq, head, sealed) = (a.offset, a.next_seq, a.head, a.sealed);
                let p = RunPosition {
                    offset,
                    next_seq,
                    head,
                    sealed,
                    stop: false,
                    committed: None,
                };
                self.took(p, ours, j);
                Step::Again
            }
            Err(ServerError::Status { status: 409, .. }) => {
                (self.conflict, self.pos) = (Some(self.known), None);
                Step::Again
            }
            Err(ServerError::Status {
                status: 503, code, ..
            }) if code == "awaiting_final" => {
                // No failure counted: the wait ends when the final heartbeat is committed.
                self.awaiting_final = true;
                let why = (Class::ServerBusy, Some(503), "the server waits for the final heartbeat");
                self.say(j, "chunk", State::Waiting, why, Duration::ZERO);
                Step::Idle
            }
            Err(e) => self.fail(now, "chunk", words(&e), e.retry_after(), j),
        }
    }

    fn read_position(&mut self, clock: &dyn Fn() -> Mono, j: &Mutex<UploadJson>) -> Step {
        let answer = self.client.position(&self.run, &self.entry);
        let now = clock();
        let p = match answer {
            Ok(p) => p,
            Err(e) => return self.fail(now, "position", words(&e), e.retry_after(), j),
        };
        if self.conflict.take().is_some_and(|k| p.offset <= k) {
            let text = "the server's position did not move after a conflict";
            return self.fail(
                now,
                "position",
                (Class::ServerBusy, Some(409), text),
                None,
                j,
            );
        }
        // Confirmed only when a local link, or the confirmed one, matches the whole position.
        let link = p.next_seq.checked_sub(1);
        let line = chunk::line_at(&self.rec, p.offset).ok().flatten();
        let local = line.is_some_and(|l| Some(l.seq) == link && hex(&l.hash) == p.head);
        let kept = lock(j)
            .confirmed
            .as_ref()
            .is_some_and(|c| c.raw == p.offset && Some(c.seq) == link && c.hash == p.head);
        self.took(p, local || kept, j);
        Step::Again
    }

    /// Takes a position the server answered: progress resets the backoff.
    fn took(&mut self, p: RunPosition, confirm: bool, j: &Mutex<UploadJson>) {
        let moved = p.offset > self.known;
        if moved {
            self.backoff.progress();
        }
        (self.known, self.refusals) = (self.known.max(p.offset), 0);
        let stored = self.inflight.take().filter(|i| moved && i.0 == p.offset);
        let at = rfc3339(SystemTime::now());
        let open = self.target.is_some_and(|t| p.offset < t);
        self.update(j, |j| {
            if p.sealed {
                j.final_ = Final::Sealed;
            }
            if confirm && let Some(seq) = p.next_seq.checked_sub(1) {
                let hash = p.head.clone();
                j.confirmed = Some(Confirmed {
                    raw: p.offset,
                    seq,
                    hash,
                    at: at.clone(),
                });
            }
            if let Some((_, bytes)) = stored {
                (j.sent.chunks, j.sent.bytes) = (j.sent.chunks + 1, j.sent.bytes + bytes);
            }
            (j.last_ok, j.failures) = (Some(at), self.backoff.failures);
            if moved {
                (j.last_error, j.next_try) = (None, None);
            }
            if open {
                j.state = State::Sending;
            }
        });
        self.pos = Some(p);
    }

    /// The round is over, or nothing waits: progress for the backoff.
    fn caught_up(&mut self, j: &Mutex<UploadJson>) {
        self.target = None;
        self.backoff.progress();
        self.update(j, |j| {
            j.state = if j.final_ == Final::Sealed {
                State::Done
            } else {
                State::CaughtUp
            };
            (j.failures, j.next_try, j.last_error) = (0, None, None);
        });
    }

    /// A failed call at `now` (read after it): back off, say so in `upload.json` and one log line.
    fn fail(
        &mut self,
        now: Mono,
        what: &'static str,
        why: Why<'_>,
        retry_after: Option<u64>,
        j: &Mutex<UploadJson>,
    ) -> Step {
        self.pos = None;
        let refused = why.0 == Class::Refused;
        self.refusals = if refused { self.refusals + 1 } else { 0 };
        let wait = self.backoff.fail(now, retry_after.map(Duration::from_secs));
        let state = match self.refusals >= self.p.refused_after {
            true => State::Refused,
            false => State::Waiting,
        };
        self.say(j, what, state, why, wait);
        Step::Idle
    }

    /// Why nothing moved: `state`, `last_error` in fixed words, `next_try`, and one log line.
    fn say(&self, j: &Mutex<UploadJson>, what: &'static str, state: State, why: Why<'_>, wait: Duration) {
        let (class, status, text) = why;
        self.update(j, |j| {
            j.state = state;
            j.last_error = Some(LastError::new(SystemTime::now(), class, status, text));
            (j.failures, j.next_try) = (self.backoff.failures, wall_after(wait));
        });
        log_failure(&self.entry_dir, what, class, status, wait);
    }

    /// The bound's raw end and the waiting bytes, read again only when the bound or the offset
    /// moved (always with no live writer).
    fn measure(&mut self, bound: &Bound, offset: u64) -> (u64, u64) {
        let key = match bound {
            Bound::Durable(d) => Some((d.line.raw_end, offset)),
            Bound::OnDisk => None,
        };
        if (key.is_none() || key != self.measured.0)
            && let Ok(m) = chunk::waiting(&self.rec, offset, bound)
        {
            self.measured = (key, m);
        }
        self.measured.1
    }

    /// Changes `upload.json` and writes it when anything changed (slice 4: `state::Writer`).
    fn update(&self, j: &Mutex<UploadJson>, f: impl FnOnce(&mut UploadJson)) {
        let mut j = lock(j);
        let before = j.clone();
        f(&mut j);
        if *j != before {
            let _ = state::write(&self.entry_dir, &j);
        }
    }
}

fn lock(j: &Mutex<UploadJson>) -> std::sync::MutexGuard<'_, UploadJson> {
    j.lock().unwrap_or_else(|e| e.into_inner())
}

fn wall_after(wait: Duration) -> Option<String> {
    SystemTime::now().checked_add(wait).map(rfc3339)
}

/// A class, a status and fixed words; never a header, a token, a body or the server's text.
type Why<'a> = (Class, Option<u16>, &'a str);

const UNREAD: Why<'static> = (Class::Internal, None, "a part file could not be read");

/// The class, status and fixed words of a failed call.
fn words(e: &ServerError) -> Why<'static> {
    let (class, status) = match e {
        ServerError::Status { status, .. } => {
            (class_of_status(*status).unwrap_or(Class::Network), Some(*status))
        }
        ServerError::Transport(_) => (Class::Network, None),
        ServerError::Malformed(_) => (Class::Internal, None),
    };
    let text = match class {
        Class::Network => "the server could not be reached",
        Class::ServerBusy => "the server is busy",
        Class::Refused => "the server refused the upload",
        _ => "the server's answer is unreadable",
    };
    (class, status, text)
}

/// `upload what=<what> class=<class> status=<n or -> wait_s=<n>`: fixed words and numbers only.
/// Slice 4 has the beat call this one and drops its own copy.
pub fn log_failure(
    entry_dir: &Path,
    what: &'static str,
    class: Class,
    status: Option<u16>,
    wait: Duration,
) {
    let status = status.map_or(Fact::Word("-"), |s| Fact::N(s.into()));
    let facts = [
        ("what", Fact::Word(what)),
        ("class", Fact::Word(class.word())),
        ("status", status),
        ("wait_s", Fact::N(wait.as_secs())),
    ];
    line(entry_dir, "upload", &facts);
}
