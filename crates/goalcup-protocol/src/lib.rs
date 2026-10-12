//! Wire and store types shared by the /goal Cup daemon and verifier.
//!
//! This crate carries no rule logic (DECISIONS D3). It defines the hash chain, the shipped
//! envelope, the signed start receipt, the acknowledgement checkpoint, the run events, the seal
//! manifest, the v5 capture records, and the connector contract:
//!
//! * [`harness`]: the harness manifest (`harness.toml`, schema 1) and its one loader;
//! * [`capture_policy`]: the reviewed trust tables a harness manifest is checked against;
//! * [`connector_dir`]: the connector-directory digest;
//! * [`pins`]: the directory pins this build ships, as data;
//! * [`hints`]: the protocol-hint grammar;
//! * [`manifest`]: the V1 manifest types live code still borrows (`RedactionPattern`, the kill
//!   and path-token vocabulary). No connector ships or loads a `manifest.toml` (OS-29).

pub mod ack;
pub mod capture;
pub mod capture_policy;
pub mod capture_status;
pub mod chain;
pub mod connector_dir;
pub mod constant_time;
pub mod control;
pub mod cursor;
pub mod envelope;
pub mod events;
pub mod export;
pub mod harness;
pub mod hints;
pub mod manifest;
pub mod pins;
pub mod process;
pub mod provenance;
pub mod receipt;
pub mod redaction;
pub mod seal;
pub mod submission;
pub mod template;

/// Version every newly appended envelope uses.
pub const PROTOCOL_VERSION: u32 = 4;

/// First version whose envelope may carry inline redaction evidence.
pub const INLINE_REDACTION_PROTOCOL_VERSION: u32 = 2;

/// First version whose `HomeSeeded` body may carry handoff markers.
pub const HOME_SEED_HANDOFF_PROTOCOL_VERSION: u32 = 3;

/// First version whose finalization may bind a workspace submission manifest.
pub const WORKSPACE_SUBMISSION_PROTOCOL_VERSION: u32 = 4;

/// The V2 compatibility label retained by historical fixtures and consumers.
///
/// This is deliberately not used as a feature floor or as arithmetic "current minus one":
/// V3 added HomeSeeded handoff markers and V4 added workspace submissions, while V2 remains
/// a readable historical format.
pub const PREVIOUS_PROTOCOL_VERSION: u32 = 2;

/// The original envelope version, retained for trace and receipt compatibility.
pub const LEGACY_PROTOCOL_VERSION: u32 = 1;

/// The protocol versions this build can read. A trace outside it is refused by the
/// verifier as `protocol_version_unsupported`, never silently parsed.
///
/// This is the LEGACY set, and it deliberately stops at 4. Protocol v5
/// ([`capture::M2_PROTOCOL_VERSION`]) is admitted only by a writer and reader written for it;
/// the legacy writer, the legacy export reader and the legacy semantic verifier refuse it.
#[allow(clippy::manual_range_patterns)] // A closed wire-version set must stay explicit.
pub fn is_supported_protocol_version(v: u32) -> bool {
    matches!(v, 1 | 2 | 3 | 4)
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("signature: {0}")]
    Signature(String),
    #[error("field {field}: {reason}")]
    Field { field: String, reason: String },
    /// Reading bytes this crate was asked to hash. `connector_dir::connector_dir_sha256` walks a
    /// whole directory, so the path that failed is carried: "io error" over a tree of files
    /// names nothing a caller can act on.
    #[error("io {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// A directory being hashed holds something that is neither a regular file nor a
    /// subdirectory — a symlink, a device node, a socket — or a name that is not UTF-8.
    ///
    /// Refused rather than skipped or followed. A skipped entry is a byte the digest does
    /// not cover, which is the hole `connector_dir_sha256` exists to close; a *followed*
    /// symlink pins bytes that live outside the directory, so one digest could then be
    /// produced by two directories whose own contents differ.
    #[error("{path}: {reason}")]
    UnhashableEntry { path: String, reason: String },
}
