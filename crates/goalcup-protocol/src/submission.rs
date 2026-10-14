//! Workspace-submission vocabulary that outlived the V4 submission writer: the daemon
//! configuration's profile and limits, and the outcome a status document reports.
//!
//! The submission manifest, and the finalization arm that bound it, left with D51: no v5
//! writer produced either.

use crate::provenance::Sha256Digest;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionProfileV1 {
    WorkspaceSnapshotV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitProfileV1 {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveSubmissionLimitsV1 {
    pub object_count: u64,
    pub total_bytes: u64,
    /// Milliseconds available to drain admitted pre-cutoff records.
    pub pre_cutoff_drain: u64,
    /// Maximum owned process identities/targets that one termination attempt may
    /// discover, retain, and signal. Timeout/grace policy is carried separately.
    pub process_walk: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionCapturedV1 {
    pub manifest_sha256: Sha256Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionFailureV1 {
    SourceChanged,
    UnsupportedEntry,
    Unreadable,
    LimitExceeded,
    StorageFailed,
    ReadbackFailed,
    RecoveryAmbiguous,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum SubmissionOutcomeV1 {
    Captured(SubmissionCapturedV1),
    Incomplete { reason: SubmissionFailureV1 },
}
