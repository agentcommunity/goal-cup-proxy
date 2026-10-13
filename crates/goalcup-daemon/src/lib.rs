//! The /goal Cup capture daemon.
//!
//! The daemon records: it partitions what a harness wrote, redacts it, hashes it and
//! buffers it, and it emits observations about its own health. It interprets nothing
//! (DECISIONS D3) — a comparison against a constant a manifest carries is a comparison,
//! and every judgement about what the comparison *means* lives in `goalcup-verifier`.
//!
//! Every wire and store type comes from `goalcup-protocol`; this crate defines none of
//! them and extends none of them.

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
/// The authenticated, bounded capture ingest listener.
pub mod ingest;
/// Bounded, scoped native byte capture.
pub mod native_capture;
pub mod redact;
pub mod registry;
/// The process-level [`runtime::Daemon`] and the local captures it holds: the loopback
/// control listener, each capture's ingest and chain, and the finish a participant's seal
/// asks for. The V1 per-entry runtime (the ordered poll, the heartbeat, the tick and the
/// deadline seal) was removed by D48.
///
/// Where every module above is wired together.
pub mod runtime;
/// Building and signing the status document (D25).
pub mod status;
mod submission_types;
pub use submission_types::CaptureResourceLimitsV1;
/// Terminating the harness process tree at seal (D22).
///
/// Unix only, like `goalcup_protocol::process`, which owns the discovery half. A module
/// compiled everywhere and refusing at run time would put a platform this tree has not
/// designed for behind a call that looks available.
#[cfg(unix)]
pub mod terminate;
pub mod workspace;

/// This build's version, as the daemon reports it: health, `control.json` and status documents.
pub const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What `goalcup-daemon --version` prints.
pub const DAEMON_VERSION_TEXT: &str = env!("CARGO_PKG_VERSION");

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
    /// path non-deterministic and duplicate capture policy outside the V1 `submission`
    /// module (removed by D48).
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
    /// Its producer was the V1 `runtime::held_elsewhere`, raised while a runtime was out of
    /// the entry map for the length of a pump. D48 removed it, so no runtime path raises
    /// `Busy` now.
    #[error("busy: {0}")]
    Busy(String),
    /// The v5 local finish ran out of its one budget before `stage` (a `FinishStage` token),
    /// wrote no terminal and left the entry open. Distinct from [`Self::Busy`], which is the
    /// legacy pump hold a client retries, and from [`Self::Refused`], the entry's own state.
    #[error("finish_deadline: {stage}")]
    FinishDeadline { stage: &'static str },
}
