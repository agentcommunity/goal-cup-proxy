//! The open-source verifier. Runs identically on the server and offline.
//!
//! Redaction is never applied here: the daemon redacts at capture and ships a log;
//! this crate reads the redacted bytes and never rewrites them.
//!
//! Everything it judges is a protocol-v5 capture trace, read by [`capture_analysis`].

pub mod capture_analysis;
mod trace_fs;

mod submission;

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
/// repository may be on it, because anybody could then sign a start and have it read as
/// `KeySource::Pinned`. With the list empty, a receipt or acknowledgement verifies only
/// under a key the caller supplies for one run (`goalcup-verify --dev-key`), which the
/// report records as `supplied`.
pub(crate) const SERVER_PUBLIC_KEYS: [&str; 0] = [];

/// Where the key that checked a signature came from.
///
/// `Pinned` is this build's own allowlist (`SERVER_PUBLIC_KEYS`), `Supplied` a key the
/// caller passed for this run (`goalcup-verify --dev-key`), and `None` no key at all. The
/// capture-integrity report maps `Descriptor` to `none`: a v5 trace carries no descriptor
/// key, so nothing produces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    /// The key is on the verifier's embedded allowlist.
    Pinned,
    /// A descriptor key accepted under `allow_descriptor_key`. No v5 trace has one.
    Descriptor,
    /// The caller supplied the key (`VerifyOptions::extra_server_keys`,
    /// `goalcup-verify --dev-key <hex>`).
    Supplied,
    /// No key was used.
    None,
}

/// Knobs a caller may turn. Every field defaults to the strict reading.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifyOptions {
    /// Accept a trace-named key that is not in `SERVER_PUBLIC_KEYS`. A v5 capture trace names
    /// none, so this has no effect; it stays because the research report records the
    /// options it ran with.
    pub allow_descriptor_key: bool,
    /// Server public keys this run will accept, hex, supplied by the caller rather than
    /// pinned into the build. `goalcup-verify --dev-key <hex>` is what fills it.
    ///
    /// This is how a development signer is honoured without its public seed entering
    /// `SERVER_PUBLIC_KEYS`, where nothing in the constant or in a verdict would
    /// distinguish it from a production anchor. A key that arrives this way is reported as
    /// `KeySource::Supplied`, so a reader of the verdict can see that the run borrowed its
    /// trust from whoever ran it.
    pub extra_server_keys: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum VerifierError {
    /// An attacker-controlled evidence document exceeded its declared verifier bound.
    #[error("evidence {path} exceeds the {limit}-byte verifier limit")]
    EvidenceTooLarge { path: String, limit: u64 },
    #[error("io {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("json {path} line {line}: {source}")]
    Json {
        path: String,
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("protocol: {0}")]
    Protocol(#[from] goalcup_protocol::ProtocolError),
    #[error("trace: {0}")]
    Trace(String),
}
