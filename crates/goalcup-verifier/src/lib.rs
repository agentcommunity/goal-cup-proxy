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
/// **M1 placeholder.** The single entry is `PLACEHOLDER_SERVER_PUBLIC_KEY`, whose seed is
/// written in this repository. Read that constant before trusting anything this list
/// resolves to `KeySource::Pinned`.
pub(crate) const SERVER_PUBLIC_KEYS: [&str; 1] = [PLACEHOLDER_SERVER_PUBLIC_KEY];

/// The M1 placeholder anchor: the Ed25519 verifying key of the seed `[5u8; 32]`, which is
/// written in this repository so development fixtures can sign receipts and
/// acknowledgements with it. The unit test below re-derives it, so the constant cannot drift
/// from what the tests sign with.
///
/// **The seed is public, so anybody can mint a receipt or an acknowledgement that this
/// build reads as `KeySource::Pinned`** — the strongest word a verdict has for the
/// provenance of a signature. It stays on `SERVER_PUBLIC_KEYS` so the committed fixtures
/// keep working, and the capture-integrity report says out loud, as `placeholder_server_key`,
/// whenever the key that verified a receipt or an acknowledgement was this one.
///
/// **Release gate.** Replacing this constant with the real organiser key is a prerequisite
/// to judging any entry: until it is replaced, `KeySource::Pinned` on a submitted trace
/// means the participant could have signed their own start. Nothing in this crate can
/// enforce that — the Flag is what makes it impossible to miss in a verdict.
pub use goalcup_protocol::receipt::PLACEHOLDER_SERVER_PUBLIC_KEY_HEX as PLACEHOLDER_SERVER_PUBLIC_KEY;

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
    /// This is a development run (`goalcup-verify --dev`): `placeholder_server_key` is
    /// suppressed, because signing with the published placeholder seed is the expected thing
    /// to do while developing.
    ///
    /// It does not suppress `server_key_unpinned`: a run that borrowed its trust from the
    /// invocation says so at every severity, in every mode.
    pub dev_mode: bool,
}

#[cfg(test)]
mod key_tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    /// The placeholder in `SERVER_PUBLIC_KEYS` is exactly the key the test suite signs
    /// with. Re-derived rather than re-typed, so the constant and the tests cannot drift.
    #[test]
    fn the_placeholder_key_is_the_one_the_tests_sign_with() {
        let signer = SigningKey::from_bytes(&[5u8; 32]);
        assert_eq!(
            SERVER_PUBLIC_KEYS[0],
            hex::encode(signer.verifying_key().to_bytes())
        );
    }
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
