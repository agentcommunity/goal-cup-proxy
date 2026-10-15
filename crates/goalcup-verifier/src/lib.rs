//! The open-source verifier. Runs identically on the server and offline.
//!
//! Redaction is never applied here: the daemon redacts at capture and ships a log;
//! this crate reads the redacted bytes and never rewrites them.
//!
//! Everything it judges is a protocol-v5 capture trace, read by [`capture_analysis`].

pub mod capture_analysis;

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
/// One key: the production upload Worker's (`goalcup-ingest` on ingest.agentcommunity.org,
/// deployed 2026-10-15). Its seed was generated offline and is held by the organiser, never in
/// this repository: no key whose seed is written here may be on this list, because anybody could
/// then sign a start and have it reported as `key_source: pinned`. Any other key (the staging
/// Worker's, a test signer's) verifies only when the caller supplies it for one run
/// (`goalcup-verify --dev-key`), which the report records as `supplied`.
pub(crate) const SERVER_PUBLIC_KEYS: [&str; 1] =
    ["6fadf49c804f89248de400d17eb0a8c8aa657b82676d81e026f10e6f7bf9079c"];

/// Knobs a caller may turn. Every field defaults to the strict reading.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifyOptions {
    /// Accept a trace-named key that is not in `SERVER_PUBLIC_KEYS`. A v5 capture trace names
    /// none, so this has no effect. It stays because the research report records the options
    /// it ran with, and because callers build these options with a struct update.
    pub allow_descriptor_key: bool,
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
