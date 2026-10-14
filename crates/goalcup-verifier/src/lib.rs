//! The open-source verifier. Runs identically on the server and offline.
//!
//! Redaction is never applied here: the daemon redacts at capture and ships a log;
//! this crate reads the redacted bytes and never rewrites them.
//!
//! Everything it judges is a protocol-v5 capture trace, read by [`capture_analysis`].

pub mod capture_analysis;
mod trace_fs;

/// `goalcup-verify --capture-integrity TRACE`: the protocol-v5 integrity route.
pub use capture_analysis::reader::{
    CaptureIntegrityReport, capture_integrity_document, scan_capture_trace,
};
/// `goalcup-verify --research-json TRACE --output REPORT.ndjson`: the private research report.
pub use capture_analysis::report::{
    ResearchResult, write_research_report, write_research_report_with,
};

pub const VERIFIER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The Ed25519 public keys this build will accept a receipt or acknowledgement signature
/// from. The allowlist lives in the binary: a key the participant could name is not an
/// anchor.
///
/// **Empty until the organiser's real key exists.** No key whose seed is written in this
/// repository may be on it, because anybody could then sign a start and have it reported as
/// `key_source: pinned`. With the list empty, a receipt or acknowledgement verifies only
/// under a key the caller supplies for one run (`goalcup-verify --dev-key`), which the
/// report records as `supplied`.
pub(crate) const SERVER_PUBLIC_KEYS: [&str; 0] = [];

/// Knobs a caller may turn. Every field defaults to the strict reading.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifyOptions {
    /// Server public keys this run will accept, hex, supplied by the caller rather than
    /// pinned into the build. `goalcup-verify --dev-key <hex>` is what fills it.
    ///
    /// This is how a development signer is honoured without its public seed entering
    /// `SERVER_PUBLIC_KEYS`, where nothing in the constant or in a verdict would
    /// distinguish it from a production anchor. A key that arrives this way is reported as
    /// `key_source: supplied`, so a reader of the verdict can see that the run borrowed its
    /// trust from whoever ran it.
    pub extra_server_keys: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum VerifierError {
    #[error("io {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("protocol: {0}")]
    Protocol(#[from] goalcup_protocol::ProtocolError),
    #[error("trace: {0}")]
    Trace(String),
}
