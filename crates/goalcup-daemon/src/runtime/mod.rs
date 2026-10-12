//! The daemon's runtime: the process-level [`Daemon`] and the local captures it holds.
//!
//! The V1 per-entry runtime (the ordered poll, the tailers, the buffer and the uploader) was
//! removed with the rest of the V1 path (D48); a local capture owns its own work, which the
//! ingest listener and the control calls drive.

mod capture;
pub(crate) use capture::{ProfileDigests, capture_harness};
mod daemon;
mod termination;

pub use daemon::Daemon;

use crate::DaemonError;
use goalcup_protocol::control::CycleErrors;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Which of the daemon's timers a failure belonged to. The tick is the one left; the V1
/// upload, status and deadline cadences were removed (D48).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cadence {
    Tick,
}

/// What the daemon's own loop lost, counted rather than logged.
///
/// [`Daemon::run_forever`] cannot exit on a transient — a daemon that did would take the
/// capture with it — and this crate has no logger, deliberately: a component that can
/// write a request to a log cannot promise it never wrote a bearer token. So failures are
/// counted here and the last message rides out on the signed status document, path-free.
/// `upload`, `status` and `deadline` stay at zero: they are wire fields whose V1 cadences were
/// removed (D48).
#[derive(Debug, Default)]
pub(crate) struct CycleCounters {
    tick: AtomicU64,
    upload: AtomicU64,
    status: AtomicU64,
    deadline: AtomicU64,
    /// When any cadence last started a pass. Nothing stamps it since the V1 cadences were
    /// removed (D48), so it stays `None`; it is a wire field of the status document.
    last_pass_at: Mutex<Option<String>>,
    last: Mutex<Option<String>>,
}

impl CycleCounters {
    /// One cadence's pass failed, on this error. A `Busy` is not one.
    ///
    /// `DaemonError::Busy` is this daemon's own contention and nothing else: the entry was
    /// out of its slot for the length of one asynchronous call, and the next pass — a
    /// second later — takes it. Counted, it would put a permanent "cycle errors: tick 1 … busy:
    /// entry … is already held by another asynchronous call" on the status of every clean
    /// run, because `last` is never cleared and the counters never come down.
    ///
    /// Nothing is hidden by this: an error a participant could act on is never `Busy` —
    /// that variant exists precisely to mark the one code a client may retry on, and
    /// every failure with a cause outside this process's own scheduling keeps its own
    /// variant and is still counted and still named.
    fn note(&self, cadence: Cadence, error: &DaemonError) {
        if matches!(error, DaemonError::Busy(_)) {
            return;
        }
        let counter = match cadence {
            Cadence::Tick => &self.tick,
        };
        counter.fetch_add(1, Ordering::Relaxed);
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(path_free(error));
    }

    /// The wire shape, as the status document carries it.
    pub(crate) fn snapshot(&self) -> CycleErrors {
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
