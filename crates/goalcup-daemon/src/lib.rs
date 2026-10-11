//! The /goal Cup capture daemon.
//!
//! The daemon records: it partitions what a harness wrote, redacts it, hashes it and
//! buffers it, and it emits observations about its own health. It interprets nothing
//! (DECISIONS D3) — a comparison against a constant a manifest carries is a comparison,
//! and every judgement about what the comparison *means* lives in `goalcup-verifier`.
//!
//! Every wire and store type comes from `goalcup-protocol`; this crate defines none of
//! them and extends none of them.

pub mod arming;
pub mod config;
/// The control API's server side: the state its handlers share, the one authentication
/// rule — `control::is_public_route` on the raw request path — the `control.json` the CLI
/// reads to find this daemon, and the router around all three.
pub mod control;
/// The file mechanics every writer in this crate shares: a mode, a private atomic
/// write through a fresh `O_EXCL` temp file, and the startup sweep for what a killed write
/// left behind. Crate-private: it is plumbing, not a surface the CLI or the verifier has any
/// business calling.
mod fsutil;
/// The five judgements of D17, as health state names.
pub mod health;
/// The authenticated, bounded capture ingest listener.
pub mod ingest;
/// Bounded, scoped native byte capture.
pub mod manifests;
pub mod native_capture;
pub mod paths;
pub mod redact;
pub mod registry;
/// One entry's runtime — the ordered poll, the notices, the heartbeat and the chained
/// health — and the process-level [`runtime::Daemon`] around it: the entry map D4 routes by
/// working directory, the loopback control listener, the tick, and the seal that the
/// deadline and a participant's early finish both take.
///
/// Where every module above is wired together.
pub mod runtime;
/// Building, signing and mirroring the signed status document (D25).
pub mod status;
mod submission_types;
pub use submission_types::{CaptureResourceLimitsV1, SourceRootJournalV1};
/// Terminating the harness process tree at seal (D22).
///
/// Unix only, like `goalcup_protocol::process`, which owns the discovery half. A module
/// compiled everywhere and refusing at run time would put a platform this tree has not
/// designed for behind a call that looks available.
#[cfg(unix)]
pub mod terminate;
pub mod workspace;

// The immutable-submission writer is a daemon API. Re-export its daemon-owned request and
// journal types together with the platform capabilities in its public signature so consumers
// such as the development server do not need a second, direct platform dependency.
pub use goalcup_platform::handle_budget::NativeHandleBudgetV1;
pub use goalcup_platform::sourcecap::SourceDir;
pub use goalcup_platform::tracecap::{EntryArtifactRoot, TraceDir};

/// This build's version, for `trace.toml` and for the `VersionChange` the daemon chains
/// when it observes its own binary change under a running entry.
pub const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What `goalcup-daemon --version` prints.
pub const DAEMON_VERSION_TEXT: &str = env!("CARGO_PKG_VERSION");

/// The most bytes the tailer reads from one file in one poll. A poll that hits this cap
/// carries what it read and sets `TailBatch.more_available`; the rest waits for the next
/// poll. Bounded memory is a constraint on the whole daemon, so no store reader ever holds
/// a whole file.
///
/// Hitting this cap chains no notice: `TailNoticeKind::OversizeFile` belongs to the whole-file
/// cap [`MAX_FILE_BYTES`], where bytes are genuinely dropped, and a poll at this cap drops
/// nothing — it reads the rest a moment later.
///
/// **One byte above [`MAX_LINE_BYTES`], written as that sum rather than as a round number.**
/// A line is overlong above the line cap — both comparisons in `jsonl.rs` are `>` — but a
/// line at the cap still needs its newline to be a line, so the poll window needs one byte of
/// headroom. With the two constants equal, a whole window would hold the bytes and not the
/// newline, and an exactly-1 MiB line would be discarded as `OverlongLine`.
pub const MAX_POLL_BYTES: u64 = MAX_LINE_BYTES + 1;

/// The per-line cap, distinct from the per-poll cap: a single line longer than this is
/// discarded and reported as `TailNotice::OverlongLine`. A line and a poll are different
/// bounds, so each has its own constant.
pub const MAX_LINE_BYTES: u64 = 1 << 20;

/// The largest whole-file capture the daemon will take in one piece — a config file it
/// baselines or restores, never a transcript it tails.
pub const MAX_FILE_BYTES: u64 = 8 << 20;

/// The most rows read from one SQLite store in one poll.
pub const MAX_POLL_ROWS: usize = 512;

/// Wall-clock seconds since the epoch, fractional. Used for intervals and for the
/// daemon's own bookkeeping; anything that reaches a chained envelope goes through
/// [`now_rfc3339`] instead, because `recorded_at` is a string in this protocol and a
/// float is the one timestamp that can serialise as `null`.
pub fn now_unix() -> f64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64(),
        // A clock before 1970 is a machine whose time is unusable, not a negative
        // instant to do arithmetic with.
        Err(_) => 0.0,
    }
}

/// Now, as the RFC-3339 `…Z` string every `recorded_at` carries.
pub fn now_rfc3339() -> String {
    unix_to_rfc3339(now_unix())
}

/// Seconds since the epoch as `YYYY-MM-DDTHH:MM:SSZ`, through the protocol's own
/// formatter so the daemon and the verifier read and write one spelling. Sub-second
/// precision is dropped by design: `parse_z` truncates, so a fractional part here would
/// claim a precision nothing downstream can check.
pub fn unix_to_rfc3339(t: f64) -> String {
    let secs = if t.is_finite() && t >= 0.0 {
        t as u64
    } else {
        0
    };
    goalcup_protocol::control::format_z(secs)
}

/// `Io` is a struct variant carrying `path` and deliberately has no `#[from]`, so `?` on a
/// `std::io::Result` does not compile in this crate: every `std::fs` call is written
/// `.map_err(|source| DaemonError::Io { path: p.display().to_string(), source })`. No error
/// message in this crate is ever a bare "No such file or directory" with no path in it.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    /// A typed capture outcome. Callers must map this enum value directly into the protocol
    /// finalization document; string matching an operating-system error would make a restart
    /// path non-deterministic and duplicate capture policy outside `submission`.
    #[error("submission capture incomplete: {reason:?}")]
    SubmissionCapture {
        reason: goalcup_protocol::submission::SubmissionFailureV1,
    },
    #[error("io {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// `String`, not `toml::de::Error`: the daemon reads TOML from two different toml
    /// APIs (deserialise and serialise) and the message is what a participant needs.
    #[error("toml: {0}")]
    Toml(String),
    #[error("protocol: {0}")]
    Protocol(#[from] goalcup_protocol::ProtocolError),
    #[error("export: {0}")]
    Export(#[from] goalcup_protocol::export::ExportError),
    /// Both redb stores share this variant — the entry registry and the capture buffer — so
    /// the message says which one it came from rather than the variant asserting one of them.
    #[error("db: {0}")]
    Db(String),
    #[error("missing: {0}")]
    Missing(String),
    #[error("sqlite: {0}")]
    Sqlite(String),
    #[error("http: {0}")]
    Http(String),
    #[error("crypto: {0}")]
    Crypto(String),
    #[error("config: {0}")]
    Config(String),
    #[error("refused: {0}")]
    Refused(String),
    #[error("guard: {0}")]
    Guard(String),
    /// The entry is held, and only for as long as one asynchronous call takes.
    ///
    /// Distinct from `Refused`, which is the entry itself saying no — already sealed, a
    /// lock still on disk, a second seal — and that distinction is the whole variant: both
    /// reach the control API as `409`, and a client cannot tell "try again in a moment"
    /// from "this will never work" out of a status code two opposite answers share. This
    /// one maps to pre-admission `ControlError::Busy`. The process route relabels the same
    /// runtime hold after durable Prepare as `ProcessAdmissionBusy`, so clients can retry
    /// without treating the child as definitely refused.
    ///
    /// Its producer is `runtime::held_elsewhere`, which is what
    /// `Daemon::with_entry` and `Daemon::seal_entry` raise when the runtime is out of the
    /// entry map for the length of a pump.
    #[error("busy: {0}")]
    Busy(String),
    /// The v5 local finish ran out of its one budget before `stage` (a `FinishStage` token),
    /// wrote no terminal and left the entry open. Distinct from [`Self::Busy`], which is the
    /// legacy pump hold a client retries, and from [`Self::Refused`], the entry's own state.
    #[error("finish_deadline: {stage}")]
    FinishDeadline { stage: &'static str },
}

impl DaemonError {
    /// The exact protocol failure class when this error came from immutable submission
    /// capture. All other daemon errors are deliberately unclassified here.
    pub fn submission_failure(&self) -> Option<goalcup_protocol::submission::SubmissionFailureV1> {
        match self {
            Self::SubmissionCapture { reason } => Some(*reason),
            _ => None,
        }
    }
}
