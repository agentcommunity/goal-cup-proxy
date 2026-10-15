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

/// What the daemon's own loop lost, counted rather than logged.
///
/// [`Daemon::run_forever`] cannot exit on a transient — a daemon that did would take the
/// capture with it — and this crate has no logger, deliberately: a component that can
/// write a request to a log cannot promise it never wrote a bearer token. So failures are
/// counted here and the last message rides out on the signed status document, path-free.
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
        self.tick.fetch_add(1, Ordering::Relaxed);
        let words = path_free(error);
        // `daemon.log` is where the CLI sends an operator; `last` is overwritten by the next error.
        eprintln!("goalcup-daemon: cycle error: {words}");
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(words);
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
