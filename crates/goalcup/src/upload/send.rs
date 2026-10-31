//! The sender of one entry (PRD-06 sections 3, 4, 5, 8 and 11): rounds of chunks from the server's
//! position, the backoff that resets on progress (run 1's lag bug), the 409 re-read, `stuck`,
//! `refused`, `503 awaiting_final` and the confirmed rule. One [`Sender::tick`] makes at most one
//! HTTP call; the upload thread calls it on its 1 s tick and at once on [`Step::Again`].

use super::chunk::{self, Bound, Chunk, Cut};
use super::client::{RunPosition, ServerClient, ServerError};
use super::policy::{Backoff, Class, Policy, Timer, after, class_of_status};
use super::state::{Confirmed, Final, LastError, State, UploadJson, rfc3339};
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
    /// Whether a local line or the confirmed link matched `pos` when it was taken.
    held: bool,
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
    /// `(durable raw end, offset)` of the last measure; None with no live writer.
    measured_at: Option<(u64, u64)>,
    /// The last measure's `(raw end, waiting bytes)`.
    measured: (u64, u64),
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
            held: false,
            known: 0,
            target: None,
            conflict: None,
            inflight: None,
            refusals: 0,
            awaiting_final: false,
            measured_at: None,
            measured: (0, 0),
        }
    }

    /// One step. `bound` is `OnDisk` only when there is no live writer; a live writer with no
    /// durable point yet sends nothing (section 3). `wrap_up` runs rounds back to back (section 4).
    /// The clock is read again after a call.
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
        let Some((end, waiting)) = self.measure(bound, pos.offset) else {
            return self.fail(clock(), "chunk", UNREAD, None, j);
        };
        {
            let mut j = lock(j);
            (j.durable_raw, j.waiting_bytes) = (end, waiting);
        }
        if pos.offset > 0 && end <= pos.offset && !self.held {
            // The server holds bytes the local copy does not, or a different chain of the same
            // length: the copies disagree. A position below the end is checked by the cut.
            let reason = "the server's position does not match a local index line";
            return self.stuck(now, reason, pos.offset, j);
        }
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
            lock(j).state = State::Sending;
        }
        let confirmed = lock(j).confirmed.clone();
        match chunk::cut(&self.rec, &pos, bound, confirmed.as_ref()) {
            Ok(Cut::Chunk(c)) => self.put(&c, clock, j),
            Ok(Cut::Nothing) => {
                self.caught_up(j);
                Step::Idle
            }
            Ok(Cut::Stuck { reason, offset }) => self.stuck(now, reason, offset, j),
            Err(_) => self.fail(clock(), "chunk", UNREAD, None, j),
        }
    }

    /// Nothing is sent across a disagreement; the round stays open for the re-read.
    fn stuck(&mut self, now: Mono, reason: &str, offset: u64, j: &Mutex<UploadJson>) -> Step {
        self.pos = None;
        self.backoff.next_try = Some(after(now, self.p.stuck_reread));
        let why = (Class::Stuck, None, &*format!("{reason} (offset {offset})"));
        self.say(j, "chunk", State::Stuck, why, Some(self.p.stuck_reread));
        Step::Idle
    }

    /// Whether a local index line, or the confirmed link, matches the whole position.
    fn holds(&self, p: &RunPosition, j: &Mutex<UploadJson>) -> bool {
        let link = p.next_seq.checked_sub(1);
        let kept = lock(j)
            .confirmed
            .as_ref()
            .is_some_and(|c| c.raw == p.offset && Some(c.seq) == link && c.hash == p.head);
        kept || chunk::line_at(&self.rec, p.offset)
            .ok()
            .flatten()
            .is_some_and(|l| Some(l.seq) == link && hex(&l.hash) == p.head)
    }

    fn put(&mut self, c: &Chunk, clock: &dyn Fn() -> Mono, j: &Mutex<UploadJson>) -> Step {
        let len = c.c_end - c.c_start;
        let read = chunk::sha256(c)
            .and_then(|sha| Ok((chunk::headers(&self.entry, c, sha), chunk::body(c)?)));
        let Ok((h, body)) = read else {
            return self.fail(clock(), "chunk", UNREAD, None, j);
        };
        self.inflight = Some((c.raw_end, len));
        // Read before the PUT leaves: a final committed while it is in flight is the race that
        // `awaiting_final` waits on.
        let final_before = lock(j).final_;
        let answer = self.client.put_chunk(&self.run, &h, body, len);
        let now = clock();
        match answer {
            Ok(a) => {
                let ours =
                    a.offset == c.raw_end && a.next_seq == c.last_seq + 1 && a.head == h.head;
                if !ours && a.offset <= self.known {
                    // A 200 that stored nothing is never retried in a tight loop.
                    let why = (
                        Class::ServerBusy,
                        Some(200),
                        "the server did not store the chunk",
                    );
                    return self.fail(now, "chunk", why, None, j);
                }
                let p = RunPosition {
                    offset: a.offset,
                    next_seq: a.next_seq,
                    head: a.head,
                    sealed: a.sealed,
                    stop: false,
                    committed: None,
                };
                // An answer that is not ours is confirmed only by a local line.
                let confirm = ours || self.holds(&p, j);
                self.took(p, confirm, j);
                Step::Again
            }
            Err(ServerError::Status { status: 409, .. }) => {
                (self.conflict, self.pos) = (Some(self.known), None);
                Step::Again
            }
            // No failure counted: the wait ends when the final heartbeat is committed. A PUT that
            // left after the final was sent meets an ordinary busy server: it backs off, on the
            // ordinary backoff, since that Retry-After is not waited out once the final is sent.
            Err(ServerError::Status {
                status: 503, code, ..
            }) if code == "awaiting_final" => {
                if final_before != Final::None {
                    let why = (Class::ServerBusy, Some(503), "the server is busy");
                    return self.fail(now, "chunk", why, None, j);
                }
                self.awaiting_final = true;
                let why = (
                    Class::ServerBusy,
                    Some(503),
                    "the server waits for the final heartbeat",
                );
                self.say(j, "chunk", State::Waiting, why, None);
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
        let confirm = self.holds(&p, j);
        self.took(p, confirm, j);
        Step::Again
    }

    /// Takes a position the server answered: progress resets the backoff.
    fn took(&mut self, p: RunPosition, confirm: bool, j: &Mutex<UploadJson>) {
        // Only progress ends a row of refusals: a position read between refused PUTs does not.
        let moved = p.offset > self.known;
        if moved {
            self.backoff.progress();
            self.refusals = 0;
        }
        self.known = self.known.max(p.offset);
        self.held = confirm;
        let stored = self.inflight.take().filter(|i| moved && i.0 == p.offset);
        let at = rfc3339(SystemTime::now());
        let open = self.target.is_some_and(|t| p.offset < t);
        {
            let mut j = lock(j);
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
            // A re-read that did not move keeps `stuck`, `refused` or `waiting` as it was.
            if moved {
                (j.last_error, j.next_try) = (None, None);
                if open {
                    j.state = State::Sending;
                }
            }
        }
        self.pos = Some(p);
    }

    /// The round is over, or nothing waits: progress for the backoff.
    fn caught_up(&mut self, j: &Mutex<UploadJson>) {
        (self.target, self.refusals) = (None, 0);
        self.backoff.progress();
        let mut j = lock(j);
        j.state = if j.final_ == Final::Sealed {
            State::Done
        } else {
            State::CaughtUp
        };
        (j.failures, j.next_try, j.last_error) = (0, None, None);
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
        let state = if self.refusals >= self.p.refused_after {
            State::Refused
        } else {
            State::Waiting
        };
        self.say(j, what, state, why, Some(wait));
        Step::Idle
    }

    /// Why nothing moved: `state`, `last_error` in fixed words, `next_try` (none while nothing is
    /// timed), and one log line.
    fn say(
        &self,
        j: &Mutex<UploadJson>,
        what: &'static str,
        state: State,
        why: Why<'_>,
        wait: Option<Duration>,
    ) {
        let (class, status, text) = why;
        {
            let mut j = lock(j);
            j.state = state;
            j.last_error = Some(LastError::new(SystemTime::now(), class, status, text));
            (j.failures, j.next_try) = (self.backoff.failures, wait.and_then(wall_after));
        }
        let wait = wait.unwrap_or_default();
        log_failure(&self.entry_dir, what, class.word(), status, wait);
    }

    /// The bound's raw end and the waiting bytes, read again only when the bound or the offset
    /// moved (always with no live writer); None when an index could not be read.
    fn measure(&mut self, bound: &Bound, offset: u64) -> Option<(u64, u64)> {
        let key = match bound {
            Bound::Durable(d) => Some((d.line.raw_end, offset)),
            Bound::OnDisk => None,
        };
        if key.is_none() || key != self.measured_at {
            self.measured = chunk::waiting(&self.rec, offset, bound).ok()?;
            self.measured_at = key;
        }
        Some(self.measured)
    }
}

/// Locks `m`; a lock a panic left behind is taken as it is (the panic is the guard's).
pub(super) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn wall_after(wait: Duration) -> Option<String> {
    SystemTime::now().checked_add(wait).map(rfc3339)
}

/// A class, a status and fixed words; never a header, a token, a body or the server's text.
type Why<'a> = (Class, Option<u16>, &'a str);

const UNREAD: Why<'static> = (Class::Internal, None, "a part file could not be read");

/// The class, status and fixed words of a failed call.
pub(super) fn words(e: &ServerError) -> Why<'static> {
    let (class, status) = match e {
        ServerError::Status { status, .. } => (
            class_of_status(*status).unwrap_or(Class::Network),
            Some(*status),
        ),
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
pub(super) fn log_failure(
    entry_dir: &Path,
    what: &'static str,
    class: &'static str,
    status: Option<u16>,
    wait: Duration,
) {
    let status = status.map_or(Fact::Word("-"), |s| Fact::N(s.into()));
    let facts = [
        ("what", Fact::Word(what)),
        ("class", Fact::Word(class)),
        ("status", status),
        ("wait_s", Fact::N(wait.as_secs())),
    ];
    line(entry_dir, "upload", &facts);
}
