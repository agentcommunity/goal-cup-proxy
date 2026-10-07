//! Redaction evidence (D14). V2 carries it inline on the altered envelope. The
//! legacy v1 log types remain readable so old chains and receipts keep verifying. Offsets
//! are byte offsets into the record's *pre*-redaction bytes. For a v2 `Leg2Post`, that
//! record is exactly the compact serialization of the complete adjacently tagged
//! `EventBody::Leg2Post { event, payload, path, receiver_meta }`, not the payload,
//! envelope, chain claims, or final JSONL line. Leg-1 offsets continue to index the
//! captured record bytes supplied by the tailer.
//!
//! Nothing in this crate performs redaction: this is the shape the daemon fills and the
//! verifier reads.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Hash-covered evidence carried on the same v2 envelope as altered leg bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedactionAudit {
    pub spans: Vec<RedactionAuditSpan>,
}

/// One altered interval, indexed into the pre-redaction bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedactionAuditSpan {
    pub kind: String,
    pub start: u64,
    pub end: u64,
}

/// The legacy v1 digest calculation: `hex(sha256(salt ++ removed))[..16]`.
///
/// Retained only to read and verify v1 fixtures. New writers must not publish this
/// secret-derived value; v2 publishes only the public kind and original offsets.
pub fn redaction_digest(salt: &str, removed: &str) -> String {
    let mut h = Sha256::new();
    h.update(salt.as_bytes());
    h.update(removed.as_bytes());
    hex::encode(h.finalize())[..16].to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedactionSpan {
    /// Pattern name, e.g. `anthropic_key`. The set is public (D14).
    pub kind: String,
    pub start: usize,
    pub end: usize,
    /// `redaction_digest(record.salt, removed)`.
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Legacy v1 second-link evidence. V2 rejects this body shape.
pub struct RedactionRecord {
    /// The store whose record was altered.
    pub store: String,
    /// The chain sequence of the record these spans belong to.
    pub target_seq: u64,
    /// The per-record salt every legacy `digest` below was taken under.
    pub salt: String,
    pub spans: Vec<RedactionSpan>,
}
