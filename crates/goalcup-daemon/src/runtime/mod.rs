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
use std::io::Write as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// What the daemon's own loop lost, counted, and written once to stderr.
///
/// [`Daemon::run_forever`] cannot exit on a transient — a daemon that did would take the
/// capture with it — and this crate has no request logger, deliberately: a component that can
/// write a request to a log cannot promise it never wrote a bearer token. So failures are
/// counted here and the last message rides out on the signed status document, path-free.
/// The same path-free words are also written to stderr (`daemon.log`) when they differ from
/// the last ones written, so a failure that repeats every poll costs one line. Nothing from
/// a request is ever logged.
/// The wire document also carries `upload`, `status`, `deadline` and `last_pass_at`; their
/// V1 cadences were removed (D48), so [`Self::snapshot`] writes them as zero and `None`.
#[derive(Debug, Default)]
pub(crate) struct CycleCounters {
    tick: AtomicU64,
    last: Mutex<Option<String>>,
}

impl CycleCounters {
    /// One tick's pass failed, on this error.
    fn note(&self, error: &DaemonError) {
        let words = path_free(error);
        // `daemon.log` is where the CLI sends an operator; `last` is overwritten by the next
        // error. A write that fails (a full disk) must not panic the thread that noted it.
        if self.record(words.clone()) {
            let _ = writeln!(std::io::stderr(), "goalcup-daemon: cycle error: {words}");
        }
    }

    /// Count one failure and keep its words. `true` when the words differ from the last
    /// ones kept, which is when they are worth a log line: a pass that fails the same way
    /// on every poll is counted every time and written once.
    fn record(&self, words: String) -> bool {
        self.tick.fetch_add(1, Ordering::Relaxed);
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        let changed = last.as_deref() != Some(words.as_str());
        *last = Some(words);
        changed
    }

    /// The wire shape, as the status document carries it.
    pub(crate) fn snapshot(&self) -> CycleErrors {
        CycleErrors {
            last_pass_at: None,
            tick: self.tick.load(Ordering::Relaxed),
            upload: 0,
            status: 0,
            deadline: 0,
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

#[cfg(test)]
mod cycle_counter_tests {
    use super::*;

    #[test]
    fn a_repeating_failure_is_counted_every_time_and_logged_once() {
        let cycles = CycleCounters::default();
        assert!(
            cycles.record("abandon failed".into()),
            "the first is logged"
        );
        assert!(!cycles.record("abandon failed".into()), "a repeat is not");
        assert!(cycles.record("reopen failed".into()), "a change is");
        let counted = cycles.snapshot();
        assert_eq!(counted.tick, 3, "every failure is still counted");
        assert_eq!(counted.last.as_deref(), Some("reopen failed"));
    }
}
