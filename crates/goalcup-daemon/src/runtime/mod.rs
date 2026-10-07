//! One entry's runtime: the ordered poll, the notices, the heartbeat and the chained
//! health.
//!
//! Every module in this crate is a piece built on its own. This one is where they
//! are wired together, and the wiring — not any single piece — is what the constraints are
//! about. Five orderings are the whole content of this file:
//!
//! 1. **partition → redact → hash → buffer, per record, never optional** (D20).
//!    [`Partitioner::admit`] borrows the record and answers; a discard is a
//!    counter and the bytes are dropped on the spot, so a foreign row cannot reach a hash.
//!    [`ChainWriter::leg1`] does the redaction and the link in one call, over the redacted
//!    payload, and only what came back goes to the [`Buffer`].
//! 2. **Teach before admitting the learners, every cycle.** A store that
//!    another store's `AggregateLearned` spec names as its `teach_store` is polled first,
//!    and every one of its records runs through [`partition::teach`] before any learner is
//!    admitted. `admit` judges once and D20 forbids holding the bytes for a second look, so
//!    a row admitted ahead of the record that teaches its aggregate is lost for the run.
//! 3. **`write_baseline` is the first chain call on a fresh entry.** Seq 0 is the
//!    baseline, which is the genesis link a start receipt's `chain_head` witnesses.
//! 4. **`DaemonRestart` before the first re-reading poll** (V9). A witness above the
//!    notice explains nothing, and a leg-1 record of the same store between the witness and
//!    the notice spends it — either way an honest restart reads as an unexplained
//!    truncation, at Fail.
//! 5. **A batch's notices before that batch's records.** The notice is what
//!    explains the records under it, and it is the other half of ordering 4: records
//!    chained first sit between the restart witness and the `Truncated`/`FileReplaced`
//!    that would have explained them, which is the same Fail reached from the other side.
//!
//! # There is no clock in this type
//!
//! [`EntryRuntime::poll_once_at`] takes `now` and every time-dependent decision inside the
//! tick reads it: the heartbeat, [`ArmingGate::tick`], [`health::evaluate`] and the
//! deadline comparison. That is the seam a test moves.
//!
//! # One thread per entry
//!
//! An `EntryRuntime` is not `Sync` in spirit even where the compiler would allow it: it
//! owns the entry's tailers (`Tailer` is `Send` and deliberately not `Sync`), its writer
//! and its arming gate, and it is **the single caller of [`record_start`] for its entry**.
//! The registry check inside `record_start` sits outside the write transaction, so two
//! runtimes racing on one entry could both pass it; the daemon gives each entry one owner
//! instead, by holding each runtime behind its own mutex.

mod capture;
pub(crate) use capture::{ProfileDigests, capture_harness};
mod daemon;
#[cfg(feature = "v1-compat")]
mod entry;
#[cfg(feature = "v1-compat")]
mod seal;
mod termination;

#[cfg(feature = "v1-compat")]
pub use daemon::Borrowed;
pub use daemon::Daemon;
#[cfg(feature = "v1-compat")]
pub(crate) use daemon::NO_RECEIVER;
#[cfg(all(feature = "v1-compat", any(test, feature = "test-hooks")))]
pub use entry::LiveSealWritePause;
#[cfg(feature = "v1-compat")]
pub use entry::{EntryRuntime, MAX_POLLS_PER_TICK, PollReport};
#[cfg(feature = "v1-compat")]
pub use seal::SealReport;

use crate::DaemonError;
#[cfg(feature = "v1-compat")]
use crate::now_rfc3339;
use goalcup_protocol::control::CycleErrors;
#[cfg(feature = "v1-compat")]
use std::sync::MutexGuard;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The daemon's admission boundary: the registry transaction that closes finalization
/// admission, and the registration that makes a runtime routable, are serialised on it.
///
/// The finalizer and `Daemon::register` hold it.
///
/// **One per daemon, not one per process.** A process-wide latch would make two daemons in
/// one process — every integration test that restarts one — serialise each other's
/// admissions and each other's finalizations for no reason at all. The latch lives on
/// `DaemonInner`; `Daemon::admission_guard` is the finalizer's end of it.
pub type AdmissionLatch = Arc<Mutex<()>>;

/// Take the boundary, waiting as long as it takes. The finalizer's end: it is already the
/// entry's owner and has nothing to answer inside a budget.
///
/// A poisoned latch is recovered rather than panicked on: a boundary another thread died
/// holding is still the boundary.
#[cfg(feature = "v1-compat")]
pub(crate) fn admission_guard(latch: &AdmissionLatch) -> MutexGuard<'_, ()> {
    latch.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// Which of the daemon's four timers a failure belonged to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cadence {
    Tick,
    #[cfg(feature = "v1-compat")]
    Upload,
    #[cfg(feature = "v1-compat")]
    Status,
    #[cfg(feature = "v1-compat")]
    Deadline,
}

/// What the daemon's own loop lost, counted rather than logged.
///
/// [`Daemon::run_forever`] cannot exit on a transient — a daemon that did would take the
/// capture with it — and this crate has no logger, deliberately: a component that can
/// write a request to a log cannot promise it never wrote a bearer token. So the four
/// cadences count their failures here and the last message rides out
/// on the signed status document, path-free.
#[derive(Debug, Default)]
pub struct CycleCounters {
    tick: AtomicU64,
    upload: AtomicU64,
    status: AtomicU64,
    deadline: AtomicU64,
    /// When any cadence last started a pass. The four counters above count failures, and on
    /// a healthy daemon they are four zeroes, so this is the one reading that says the loops
    /// are turning: a stamp from a second ago is a live daemon and an old one is a stopped one.
    /// It moves on every pass, whatever the pass found and whether or not it failed.
    last_pass_at: Mutex<Option<String>>,
    last: Mutex<Option<String>>,
}

impl CycleCounters {
    /// One cadence started a pass. Stamped whether or not the pass found anything and
    /// whether or not it failed: a failed pass is still a pass the loop made, and
    /// [`CycleCounters::note`] is what says it failed. "Started", because `seal_due_entries`
    /// stamps before its work rather than after it, and a stamp a reader uses to tell a
    /// turning daemon from a stopped one answers the same either way.
    ///
    /// It takes no cadence. Four cadences share one stamp because they are one loop, and
    /// what a reader does with it is tell a turning daemon from a stopped one — an argument
    /// nothing reads is surface a caller has to get right for no reason.
    #[cfg(feature = "v1-compat")]
    fn ran(&self) {
        *self.last_pass_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(now_rfc3339());
    }

    /// One cadence's pass failed, on this error. A `Busy` is not one.
    ///
    /// `DaemonError::Busy` is this daemon's own contention and nothing else: the entry was
    /// out of its slot for the length of one asynchronous call, and the next pass — a
    /// second later — takes it. Counted, it would put a permanent "cycle errors: tick 1 … busy:
    /// entry … is already held by another asynchronous call" on the status of every clean
    /// run, because `last` is never cleared and the four counters never come down.
    ///
    /// Nothing is hidden by this: an error a participant could act on is never `Busy` —
    /// that variant exists precisely to mark the one code a client may retry on, and
    /// every failure with a cause outside this process's own scheduling keeps its own
    /// variant and is still counted and still named.
    ///
    /// **What the narrowing costs.** A wedge stops leg 1 advancing, so
    /// `health::CaptureStalled` fires once `now - last_leg1_at > stall_after_seconds`
    /// (`health.rs`), which is a participant-facing rung.
    /// `cycles last ran` is not that rung: `ran()` is stamped before the outcome is
    /// inspected, so the stamp keeps moving under permanent `Busy` — it tells a stopped
    /// daemon from a turning one and nothing more.
    ///
    /// The corner nothing catches is a wedge from before the first leg-1 record:
    /// `last_leg1_at` is `None`, so `stalled` is `false`, leaving a just-armed run reading
    /// healthy.
    fn note(&self, cadence: Cadence, error: &DaemonError) {
        if matches!(error, DaemonError::Busy(_)) {
            return;
        }
        let counter = match cadence {
            Cadence::Tick => &self.tick,
            #[cfg(feature = "v1-compat")]
            Cadence::Upload => &self.upload,
            #[cfg(feature = "v1-compat")]
            Cadence::Status => &self.status,
            #[cfg(feature = "v1-compat")]
            Cadence::Deadline => &self.deadline,
        };
        counter.fetch_add(1, Ordering::Relaxed);
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(path_free(error));
    }

    /// The wire shape, as the status document carries it.
    pub fn snapshot(&self) -> CycleErrors {
        CycleErrors {
            last_pass_at: self
                .last_pass_at
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
            tick: self.tick.load(Ordering::Relaxed),
            upload: self.upload.load(Ordering::Relaxed),
            status: self.status.load(Ordering::Relaxed),
            deadline: self.deadline.load(Ordering::Relaxed),
            last: self.last.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        }
    }
}

/// A `DaemonError`'s words with every filesystem path removed.
///
/// The status document is unauthenticated and the status line is rendered into a
/// terminal a screen recording may catch, so nothing that reaches either may carry a path
/// out of the participant's home. `DaemonError::Io` states its path by construction — that
/// is the point of the variant — so it is reported as the io error alone, and every
/// remaining word that starts a path is replaced rather than trusted.
fn path_free(error: &DaemonError) -> String {
    let text = match error {
        DaemonError::Io { source, .. } => format!("io: {source}"),
        other => other.to_string(),
    };
    text.split(' ')
        .map(|word| {
            let lead: usize = word
                .chars()
                .take_while(|c| !c.is_ascii_alphanumeric() && *c != '/')
                .map(char::len_utf8)
                .sum();
            match word[lead..].starts_with('/') {
                true => format!("{}<path>", &word[..lead]),
                false => word.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The runtime is out of its slot for the length of an asynchronous call.
///
/// Not a failure of the entry: `Borrowed` puts it back when the call ends, including when
/// the future is dropped.
///
/// **`Busy` and not `Refused`.** Both reach the control API as `409`, and a client
/// cannot tell "try again in a moment" from "this will never work" out of a status code the
/// two share. This is the transient one, it is the only code a client may retry on, and it
/// is the whole reason `ControlError::Busy` exists: `Refused` here would make a retry
/// loop key on the status, and so retry a genuinely refused call.
#[cfg(feature = "v1-compat")]
fn held_elsewhere(entry_id: &str) -> DaemonError {
    DaemonError::Busy(format!(
        "entry {entry_id} is mid-upload: its runtime is out of the map for the length of \
         one pump, and this call would have to hold a lock across it"
    ))
}

#[cfg(test)]
#[cfg(feature = "v1-compat")]
mod admission_tests {
    use super::{AdmissionLatch, admission_guard};
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn receiver_delivery_and_finalization_share_one_admission_latch() {
        let latch: AdmissionLatch = Default::default();
        let first = admission_guard(&latch);
        let (entered, receive) = mpsc::channel();
        let second_latch = latch.clone();
        let waiter = std::thread::spawn(move || {
            let _second = admission_guard(&second_latch);
            entered.send(()).unwrap();
        });
        assert!(
            receive.recv_timeout(Duration::from_millis(25)).is_err(),
            "the second admission path must wait behind the boundary owner"
        );
        drop(first);
        receive
            .recv_timeout(Duration::from_secs(1))
            .expect("releasing the boundary admits the waiting path");
        waiter.join().unwrap();
    }
}
