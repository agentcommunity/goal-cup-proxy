//! Wire and store types shared by the /goal Cup daemon and verifier.
//!
//! This crate carries no rule logic (DECISIONS D3). It defines the hash chain, the shipped
//! envelope, the signed start receipt, the acknowledgement checkpoint, the health states, the
//! seal manifest, the v5 capture records, and the connector contract. It carries protocol v5
//! only ([`capture::M2_PROTOCOL_VERSION`], D54):
//!
//! * [`harness`]: the harness manifest (`harness.toml`, schema 1) and its one loader;
//! * [`capture_policy`]: the reviewed trust tables a harness manifest is checked against;
//! * [`connector_dir`]: the connector-directory digest;
//! * [`pins`]: the directory pins this build ships, as data;
//! * [`hints`]: the protocol-hint grammar;
//! * [`manifest`]: the kill, launch-mode, scan-root and redaction-pattern vocabulary the
//!   launch, daemon and registry share. No connector ships or loads a `manifest.toml` (OS-29).

pub mod ack;
pub mod capture;
pub mod capture_policy;
pub mod capture_status;
pub mod chain;
pub mod clock;
pub mod connector_dir;
pub mod constant_time;
pub mod control;
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
pub mod run_secret;
pub mod seal;
pub mod template;

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
