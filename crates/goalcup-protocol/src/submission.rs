//! Workspace-submission vocabulary that outlived the V4 submission writer: the outcome a
//! finish report (`control::FinishReportV1`) and a status document's finalization projection
//! can still name on the wire.
//!
//! The submission manifest, and the finalization arm that bound it, left with D54: no v5
//! writer produced either. The daemon-configuration profile and limit types left with the D54
//! amendment of 21 Oct 2026: nothing read them.

use crate::provenance::Sha256Digest;
use serde::{Deserialize, Serialize};

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
