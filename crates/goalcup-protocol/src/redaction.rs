//! Redaction evidence (D14). A v5 capture `Segment` carries it beside the redacted bytes.
//! Offsets are absolute pre-redaction source offsets within the segment's stream.
//!
//! Nothing in this crate performs redaction: this is the shape the daemon fills and the
//! verifier reads.

use serde::{Deserialize, Serialize};

/// Hash-covered evidence carried beside the redacted bytes it describes.
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
