//! Versioned workspace-submission documents.
//!
//! The manifest binds the logical workspace entries to a content-addressed object set.
//! It defines and validates bytes only; capture, filesystem traversal, and rule decisions
//! belong to the daemon and verifier.

use crate::provenance::{
    PortableRelativePath, ProvenanceError, Rfc3339Utc, Sha256Digest, require_time_order,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const MAX_IDENTIFIER_BYTES_V1: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionProfileV1 {
    WorkspaceSnapshotV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReasonV1 {
    Manual,
    Deadline,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionKindV1 {
    Regular,
    SymbolicLink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionEntryV1 {
    pub path: PortableRelativePath,
    pub kind: SubmissionKindV1,
    pub posix_owner_executable: bool,
    pub bytes: u64,
    pub content_sha256: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionManifestV1 {
    pub profile: SubmissionProfileV1,
    pub limit_profile: LimitProfileV1,
    pub effective_limits: EffectiveSubmissionLimitsV1,
    pub entry_id: String,
    pub receipt_sha256: Sha256Digest,
    pub cutoff_seq: u64,
    pub cutoff_chain_hash: Sha256Digest,
    pub finish_reason: FinishReasonV1,
    pub entries: Vec<SubmissionEntryV1>,
    pub excluded: Vec<String>,
    pub copy_started_at: Rfc3339Utc,
    pub copy_completed_at: Rfc3339Utc,
    pub readback_verified_at: Rfc3339Utc,
    /// Count of distinct content digests in `entries`.
    pub object_count: u64,
    /// Logical byte sum across every entry, including repeated content.
    pub total_bytes: u64,
}

impl SubmissionManifestV1 {
    pub fn validate(&self) -> Result<(), ProvenanceError> {
        validate_identifier("limit_profile.id", &self.limit_profile.id)?;
        validate_identifier("entry_id", &self.entry_id)?;
        validate_positive(
            "effective_limits.object_count",
            self.effective_limits.object_count,
        )?;
        validate_positive(
            "effective_limits.total_bytes",
            self.effective_limits.total_bytes,
        )?;
        validate_positive(
            "effective_limits.pre_cutoff_drain",
            self.effective_limits.pre_cutoff_drain,
        )?;
        validate_positive(
            "effective_limits.process_walk",
            self.effective_limits.process_walk,
        )?;

        let entry_count =
            u64::try_from(self.entries.len()).map_err(|_| ProvenanceError::IntegerOverflow {
                field: "entry_count",
            })?;
        if entry_count > self.effective_limits.object_count {
            return Err(invalid(
                "entries",
                "entry count exceeds the effective object-count limit",
            ));
        }

        for pair in self.entries.windows(2) {
            if pair[0].path >= pair[1].path {
                return Err(invalid("entries", "paths must be sorted and unique"));
            }
        }
        for pair in self.excluded.windows(2) {
            if pair[0] >= pair[1] {
                return Err(invalid("excluded", "values must be sorted and unique"));
            }
        }
        for excluded in &self.excluded {
            validate_identifier("excluded", excluded)?;
        }

        let mut logical_bytes = 0u64;
        let mut distinct = BTreeMap::<&Sha256Digest, u64>::new();
        for entry in &self.entries {
            if entry.kind == SubmissionKindV1::SymbolicLink && entry.posix_owner_executable {
                return Err(invalid(
                    "entries.posix_owner_executable",
                    "symbolic links must record false",
                ));
            }
            logical_bytes =
                logical_bytes
                    .checked_add(entry.bytes)
                    .ok_or(ProvenanceError::IntegerOverflow {
                        field: "total_bytes",
                    })?;
            match distinct.get(&entry.content_sha256) {
                Some(bytes) if *bytes != entry.bytes => {
                    return Err(invalid(
                        "entries.content_sha256",
                        "one digest cannot declare different lengths",
                    ));
                }
                Some(_) => {}
                None => {
                    distinct.insert(&entry.content_sha256, entry.bytes);
                }
            }
        }

        let distinct_count =
            u64::try_from(distinct.len()).map_err(|_| ProvenanceError::IntegerOverflow {
                field: "object_count",
            })?;
        if self.object_count != distinct_count {
            return Err(invalid(
                "object_count",
                "must equal the number of distinct content digests",
            ));
        }
        if distinct_count > self.effective_limits.object_count {
            return Err(invalid(
                "object_count",
                "distinct object count exceeds the effective limit",
            ));
        }
        if self.total_bytes != logical_bytes {
            return Err(invalid(
                "total_bytes",
                "must equal the logical byte sum across entries",
            ));
        }
        if logical_bytes > self.effective_limits.total_bytes {
            return Err(invalid(
                "total_bytes",
                "logical bytes exceed the effective limit",
            ));
        }
        // Each retained digest contributes bytes at most once physically, while
        // `logical_bytes` includes every entry that reuses it. Distinct physical object
        // bytes are therefore a subset of the checked logical sum and share its bound.

        require_time_order(
            &self.copy_started_at,
            &self.copy_completed_at,
            "submission copy",
        )?;
        require_time_order(
            &self.copy_completed_at,
            &self.readback_verified_at,
            "submission readback",
        )
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ProvenanceError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|_| ProvenanceError::Serialise)
    }

    pub fn digest(&self) -> Result<Sha256Digest, ProvenanceError> {
        Ok(Sha256Digest::from_bytes(
            Sha256::digest(self.canonical_bytes()?).into(),
        ))
    }
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

fn validate_identifier(field: &'static str, value: &str) -> Result<(), ProvenanceError> {
    if value.is_empty()
        || value.len() > MAX_IDENTIFIER_BYTES_V1
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        Err(invalid(field, "must be bounded non-control text"))
    } else {
        Ok(())
    }
}

fn validate_positive(field: &'static str, value: u64) -> Result<(), ProvenanceError> {
    if value == 0 {
        Err(invalid(field, "must be positive"))
    } else {
        Ok(())
    }
}

fn invalid(field: &'static str, reason: &'static str) -> ProvenanceError {
    ProvenanceError::InvalidField { field, reason }
}
