//! Immutable, handle-rooted workspace retention for submission.
//!
//! This module records source bytes and native facts. It does not decide whether those bytes
//! satisfy a challenge rule: that belongs exclusively to `goalcup-verifier`.

use crate::{DaemonError, now_rfc3339};
use goalcup_platform::sourcecap::{
    SourceCapError, SourceDir, SourceFactsV1, SourceKind, SourceLookup, SourceName,
    SourceObservation,
};
use goalcup_platform::tracecap::{EntryArtifactRoot, TraceCapError, TraceDir};
use goalcup_protocol::chain::Link;
use goalcup_protocol::provenance::{PortableRelativePath, Rfc3339Utc, Sha256Digest};
use goalcup_protocol::seal::is_excluded;
use goalcup_protocol::submission::{
    EffectiveSubmissionLimitsV1, FinishReasonV1, LimitProfileV1, SubmissionCapturedV1,
    SubmissionEntryV1, SubmissionFailureV1, SubmissionKindV1, SubmissionManifestV1,
    SubmissionOutcomeV1, SubmissionProfileV1,
};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read as _;

const SUBMISSION: &str = "submission";
const CAPTURE_JOURNAL: &str = ".submission-capture-v1";
const STAGING_PREFIX: &str = "submission-staging-v1-";
const OWNER_RECORD: &str = "owner.json";
const INVENTORY_RECORD: &str = "inventory.json";
const STAGE_RECORD: &str = "stage.json";
const COPY_RECORD: &str = "copy.json";
const OBJECTS_RECORD: &str = "objects-verified.json";
const MANIFEST_RECORD: &str = "manifest-planned.json";
const STAGED_RECORD: &str = "staged-verified.json";
const PROMOTED_RECORD: &str = "promoted.json";
const RESOLUTION_RECORD: &str = "resolution.json";
const MANIFEST: &str = "manifest.json";
const OBJECTS: &str = "objects";
const SHA256: &str = "sha256";
const COPY_BUFFER: usize = 64 * 1024;
const MAX_SOURCE_JOURNAL_BYTES: u64 = 4 * 1024;
const MAX_CAPTURE_JOURNAL_BYTES: u64 = 64 * 1024;
pub use crate::submission_types::{CaptureResourceLimitsV1, SourceRootJournalV1};
#[cfg(test)]
use crate::submission_types::{
    MAX_CAPTURE_INVENTORY_BYTES, MAX_CAPTURE_MANIFEST_BYTES, MAX_CAPTURE_NAMESPACE_NODES,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureCheckpoint {
    OwnerCommitted,
    FirstInventoryComplete,
    InventoryCommitted,
    StageCommitted,
    ObjectChunkWritten,
    CopyComplete,
    CopyCommitted,
    ObjectsVerified,
    ObjectsCommitted,
    ManifestCommitted,
    ManifestSynced,
    StagedCommitted,
    Promoted,
    FinalReadbackComplete,
    ResolutionCommitted,
}

/// The finalization owner supplies values already bound to the receipt and cutoff chain.
#[derive(Debug, Clone)]
pub struct SubmissionRequestV1 {
    /// Finalization generation that owns this one immutable capture attempt.
    pub generation: u64,
    pub entry_id: String,
    pub receipt_sha256: Sha256Digest,
    pub cutoff: Link,
    pub reason: FinishReasonV1,
    pub excluded: Vec<String>,
    pub limit_profile: LimitProfileV1,
    pub limits: EffectiveSubmissionLimitsV1,
    /// Versioned writer-resource bounds which are separate from manifest object semantics.
    pub capture_limits: CaptureResourceLimitsV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InventoryEntry {
    path: String,
    facts: InventoryFactsV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StableInventory {
    version: u8,
    source_identity: String,
    entries: Vec<InventoryEntry>,
    directories: Vec<InventoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InventoryFactsV1 {
    identity: String,
    kind: InventoryKindV1,
    mode: Option<u32>,
    length: u64,
    posix_owner_executable: Option<bool>,
    modified_seconds: i64,
    modified_nanos: i64,
    changed_seconds: i64,
    changed_nanos: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum InventoryKindV1 {
    Directory,
    Regular,
    SymbolicLink,
    Special,
}

impl From<SourceFactsV1> for InventoryFactsV1 {
    fn from(value: SourceFactsV1) -> Self {
        Self {
            identity: value.identity,
            kind: match value.kind {
                SourceKind::Directory => InventoryKindV1::Directory,
                SourceKind::Regular => InventoryKindV1::Regular,
                SourceKind::SymbolicLink => InventoryKindV1::SymbolicLink,
                SourceKind::Special => InventoryKindV1::Special,
            },
            mode: value.mode,
            length: value.length,
            posix_owner_executable: value.posix_owner_executable,
            modified_seconds: value.modified_seconds,
            modified_nanos: value.modified_nanos,
            changed_seconds: value.changed_seconds,
            changed_nanos: value.changed_nanos,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureOwnerBindingsV1 {
    version: u8,
    generation: u64,
    entry_id: String,
    receipt_sha256: Sha256Digest,
    cutoff_seq: u64,
    cutoff_prev: String,
    cutoff_chain_hash: String,
    reason: FinishReasonV1,
    excluded: BTreeSet<String>,
    limit_profile: LimitProfileV1,
    limits: EffectiveSubmissionLimitsV1,
    capture_limits: CaptureResourceLimitsV1,
    source_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureOwnerRecordV1 {
    version: u8,
    capture_id: Sha256Digest,
    journal_identity: String,
    staging_name: String,
    bindings: CaptureOwnerBindingsV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureInventoryRecordV1 {
    version: u8,
    capture_id: Sha256Digest,
    prior_sha256: Sha256Digest,
    inventory: StableInventory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureStageRecordV1 {
    version: u8,
    capture_id: Sha256Digest,
    prior_sha256: Sha256Digest,
    staging_name: String,
    staging_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureCopyRecordV1 {
    version: u8,
    capture_id: Sha256Digest,
    prior_sha256: Sha256Digest,
    entries: Vec<SubmissionEntryV1>,
    copy_started_at: Rfc3339Utc,
    copy_completed_at: Rfc3339Utc,
    object_count: u64,
    total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureObjectsRecordV1 {
    version: u8,
    capture_id: Sha256Digest,
    prior_sha256: Sha256Digest,
    readback_verified_at: Rfc3339Utc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureManifestRecordV1 {
    version: u8,
    capture_id: Sha256Digest,
    prior_sha256: Sha256Digest,
    manifest_sha256: Sha256Digest,
    manifest_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureStagedRecordV1 {
    version: u8,
    capture_id: Sha256Digest,
    prior_sha256: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapturePromotedRecordV1 {
    version: u8,
    capture_id: Sha256Digest,
    prior_sha256: Sha256Digest,
    submission_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureResolutionRecordV1 {
    version: u8,
    capture_id: Sha256Digest,
    prior_phase: CaptureResolutionPriorV1,
    prior_sha256: Sha256Digest,
    outcome: SubmissionOutcomeV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CaptureResolutionPriorV1 {
    Owner,
    Inventory,
    Stage,
    Copy,
    Objects,
    Manifest,
    Staged,
    Promoted,
}

pub fn prepare_source_journal(
    root: &SourceDir,
    artifact: &EntryArtifactRoot,
) -> Result<SourceRootJournalV1, DaemonError> {
    let journal = SourceRootJournalV1 {
        identity: root.identity().journal_token(),
    };
    let bytes = canonical_json_bounded(
        &journal,
        MAX_SOURCE_JOURNAL_BYTES,
        SubmissionFailureV1::LimitExceeded,
    )?;
    match artifact.install_sealing_source_once(&bytes) {
        Ok(()) => Ok(journal),
        Err(TraceCapError::Refused {
            reason: goalcup_platform::tracecap::RefusalKind::EntryAlreadyExists,
            ..
        }) => {
            let retained = artifact
                .read_sealing_source()
                .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?
                .ok_or_else(|| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
            let retained_bytes = read_private_exact(
                retained,
                u64::try_from(bytes.len())
                    .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?,
                SubmissionFailureV1::RecoveryAmbiguous,
            )?;
            let prior: SourceRootJournalV1 = serde_json::from_slice(&retained_bytes)
                .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
            if retained_bytes != bytes || prior != journal {
                return Err(failure(SubmissionFailureV1::SourceChanged));
            }
            Ok(prior)
        }
        Err(_) => Err(storage_failure()),
    }
}

/// Retain a complete, stable source inventory under a private staging directory, read it back
/// through capabilities, then expose it with one no-replacement directory promotion.
pub fn capture_submission(
    root: &SourceDir,
    trace: &TraceDir,
    journal: &SourceRootJournalV1,
    request: &SubmissionRequestV1,
) -> Result<SubmissionCapturedV1, DaemonError> {
    capture_submission_inner(root, trace, journal, request, true, &mut |_| Ok(()))
}

#[cfg(test)]
fn capture_submission_with_checkpoints(
    root: &SourceDir,
    trace: &TraceDir,
    journal: &SourceRootJournalV1,
    request: &SubmissionRequestV1,
    checkpoint: &mut impl FnMut(CaptureCheckpoint) -> Result<(), DaemonError>,
) -> Result<SubmissionCapturedV1, DaemonError> {
    capture_submission_inner(root, trace, journal, request, false, checkpoint)
}

fn capture_submission_inner(
    root: &SourceDir,
    trace: &TraceDir,
    journal: &SourceRootJournalV1,
    request: &SubmissionRequestV1,
    record_failure: bool,
    checkpoint: &mut impl FnMut(CaptureCheckpoint) -> Result<(), DaemonError>,
) -> Result<SubmissionCapturedV1, DaemonError> {
    validate_capture_limits(request)?;
    let excluded = normalised_exclusions(&request.excluded)?;
    let bindings = capture_owner_bindings(journal, request, &excluded);
    if let Ok(retained) = trace.open_child_dir(CAPTURE_JOURNAL) {
        let _capture_lock = retained
            .acquire_capture_lock()
            .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
        let result = recover_committed_capture(trace, &retained, journal, request, &excluded);
        return conclude_capture_attempt(&retained, &bindings, result, record_failure);
    }
    let capture_id = digest_bytes(&canonical_json_bounded(
        &bindings,
        MAX_CAPTURE_JOURNAL_BYTES,
        SubmissionFailureV1::LimitExceeded,
    )?);
    let staging_name = format!("{STAGING_PREFIX}{}", capture_id.as_str());
    let init_name = format!(".submission-capture-init-v1-{}", capture_id.as_str());
    let init = match trace.create_child_dir(&init_name) {
        Ok(created) => created,
        Err(TraceCapError::Refused {
            reason: goalcup_platform::tracecap::RefusalKind::EntryAlreadyExists,
            ..
        }) => trace
            .open_child_dir(&init_name)
            .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?,
        Err(_) => return Err(storage_failure()),
    };
    let init_lock = init
        .acquire_capture_lock()
        .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
    if let Ok(retained) = trace.open_child_dir(CAPTURE_JOURNAL) {
        drop(init_lock);
        let _capture_lock = retained
            .acquire_capture_lock()
            .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
        let result = recover_committed_capture(trace, &retained, journal, request, &excluded);
        return conclude_capture_attempt(&retained, &bindings, result, record_failure);
    }
    let owner = CaptureOwnerRecordV1 {
        version: 1,
        capture_id: capture_id.clone(),
        journal_identity: init.identity().map_err(|_| storage_failure())?,
        staging_name: staging_name.clone(),
        bindings: bindings.clone(),
    };
    if let Err(error) = write_record_once(&init, OWNER_RECORD, &owner) {
        return conclude_capture_attempt(&init, &bindings, Err(error), record_failure);
    }
    if init.sync().is_err() {
        return conclude_capture_attempt(&init, &bindings, Err(storage_failure()), record_failure);
    }
    match trace.promote_child_dir_exclusive(&init_name, CAPTURE_JOURNAL) {
        Ok(()) => {}
        Err(_) => match trace.open_child_dir(CAPTURE_JOURNAL) {
            Ok(retained) => {
                drop(init_lock);
                let _capture_lock = retained
                    .acquire_capture_lock()
                    .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
                let result =
                    recover_committed_capture(trace, &retained, journal, request, &excluded);
                return conclude_capture_attempt(&retained, &bindings, result, record_failure);
            }
            Err(_) => {
                return conclude_capture_attempt(
                    &init,
                    &bindings,
                    Err(storage_failure()),
                    record_failure,
                );
            }
        },
    }
    let capture_journal = match trace.open_child_dir(CAPTURE_JOURNAL) {
        Ok(retained) => retained,
        Err(_) => {
            return conclude_capture_attempt(
                &init,
                &bindings,
                Err(failure(SubmissionFailureV1::RecoveryAmbiguous)),
                record_failure,
            );
        }
    };
    let retained_journal_identity = match capture_journal.identity() {
        Ok(identity) => identity,
        Err(_) => {
            return conclude_capture_attempt(
                &init,
                &bindings,
                Err(storage_failure()),
                record_failure,
            );
        }
    };
    if retained_journal_identity != owner.journal_identity {
        return conclude_capture_attempt(
            &init,
            &bindings,
            Err(failure(SubmissionFailureV1::RecoveryAmbiguous)),
            record_failure,
        );
    }
    let _capture_lock = init_lock;
    let owner_bytes = match read_record_exact::<CaptureOwnerRecordV1>(
        &capture_journal,
        OWNER_RECORD,
        MAX_CAPTURE_JOURNAL_BYTES,
        SubmissionFailureV1::RecoveryAmbiguous,
    ) {
        Ok((_, bytes)) => bytes,
        Err(error) => {
            return conclude_capture_attempt(
                &capture_journal,
                &bindings,
                Err(error),
                record_failure,
            );
        }
    };
    let result = (|| {
        checkpoint(CaptureCheckpoint::OwnerCommitted)?;
        if journal.identity != root.identity().journal_token() {
            return Err(failure(SubmissionFailureV1::SourceChanged));
        }

        let first = inventory(root, &excluded, &request.limits, &request.capture_limits)?;
        checkpoint(CaptureCheckpoint::FirstInventoryComplete)?;
        let second = inventory(root, &excluded, &request.limits, &request.capture_limits)?;
        if first != second {
            return Err(failure(SubmissionFailureV1::SourceChanged));
        }
        let inventory_record = CaptureInventoryRecordV1 {
            version: 1,
            capture_id: capture_id.clone(),
            prior_sha256: digest_bytes(&owner_bytes),
            inventory: first,
        };
        let inventory_bytes = write_record_once_bounded(
            &capture_journal,
            INVENTORY_RECORD,
            &inventory_record,
            request.capture_limits.inventory_bytes,
        )?;
        checkpoint(CaptureCheckpoint::InventoryCommitted)?;
        let staging = trace
            .create_child_dir(&staging_name)
            .map_err(|_| storage_failure())?;
        staging.sync().map_err(|_| storage_failure())?;
        trace.sync().map_err(|_| storage_failure())?;
        let stage = CaptureStageRecordV1 {
            version: 1,
            capture_id: capture_id.clone(),
            prior_sha256: digest_bytes(&inventory_bytes),
            staging_name: staging_name.clone(),
            staging_identity: staging.identity().map_err(|_| storage_failure())?,
        };
        let stage_bytes = write_record_once(&capture_journal, STAGE_RECORD, &stage)?;
        checkpoint(CaptureCheckpoint::StageCommitted)?;
        let objects = staging
            .create_child_dir(OBJECTS)
            .map_err(|_| storage_failure())?;
        let object_store = objects
            .create_child_dir(SHA256)
            .map_err(|_| storage_failure())?;
        object_store.sync().map_err(|_| storage_failure())?;
        objects.sync().map_err(|_| storage_failure())?;
        staging.sync().map_err(|_| storage_failure())?;

        let started = timestamp()?;
        let mut copied = BTreeMap::<Sha256Digest, u64>::new();
        let mut entries = Vec::new();
        let mut total_bytes = 0u64;
        let mut copy_namespace_count = 1u64;
        let mut manifest_bytes_used = 1_024u64;
        copy_dir(
            root,
            "",
            &excluded,
            &request.limits,
            request.capture_limits.namespace_nodes,
            &object_store,
            &mut copied,
            &mut entries,
            &mut total_bytes,
            &mut copy_namespace_count,
            &mut manifest_bytes_used,
            request.capture_limits.manifest_bytes,
            checkpoint,
        )?;
        checkpoint(CaptureCheckpoint::CopyComplete)?;

        let after_copy = inventory(root, &excluded, &request.limits, &request.capture_limits)?;
        if inventory_record.inventory != after_copy {
            return Err(failure(SubmissionFailureV1::SourceChanged));
        }

        entries.sort_by(|left, right| left.path.cmp(&right.path));
        let copy_completed_at = timestamp()?;
        object_store.sync().map_err(|_| storage_failure())?;
        objects.sync().map_err(|_| storage_failure())?;
        staging.sync().map_err(|_| storage_failure())?;
        let copy_record = CaptureCopyRecordV1 {
            version: 1,
            capture_id: capture_id.clone(),
            prior_sha256: digest_bytes(&stage_bytes),
            entries,
            copy_started_at: started,
            copy_completed_at,
            object_count: u64::try_from(copied.len())
                .map_err(|_| failure(SubmissionFailureV1::LimitExceeded))?,
            total_bytes,
        };
        validate_copy_record(
            &owner,
            &inventory_record,
            &stage,
            &stage_bytes,
            &copy_record,
        )?;
        let copy_bytes = write_record_once_bounded(
            &capture_journal,
            COPY_RECORD,
            &copy_record,
            copy_record_limit(&request.capture_limits)?,
        )?;
        checkpoint(CaptureCheckpoint::CopyCommitted)?;
        finish_capture_from_copy(
            trace,
            &capture_journal,
            &owner,
            &stage,
            copy_record,
            copy_bytes,
            checkpoint,
        )
    })();
    conclude_capture_attempt(&capture_journal, &bindings, result, record_failure)
}

fn finish_capture_from_copy(
    trace: &TraceDir,
    capture_journal: &TraceDir,
    owner: &CaptureOwnerRecordV1,
    stage: &CaptureStageRecordV1,
    copy: CaptureCopyRecordV1,
    copy_bytes: Vec<u8>,
    checkpoint: &mut impl FnMut(CaptureCheckpoint) -> Result<(), DaemonError>,
) -> Result<SubmissionCapturedV1, DaemonError> {
    let expected_prior_sha256 = digest_bytes(&copy_bytes);
    let (objects_record, objects_bytes) = match read_record_optional::<CaptureObjectsRecordV1>(
        capture_journal,
        OBJECTS_RECORD,
        MAX_CAPTURE_JOURNAL_BYTES,
        SubmissionFailureV1::RecoveryAmbiguous,
    )? {
        Some((retained, bytes)) => {
            if retained.version != 1
                || retained.capture_id != owner.capture_id
                || retained.prior_sha256 != expected_prior_sha256
            {
                return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
            }
            (retained, bytes)
        }
        None => {
            let staging = open_owned_stage(trace, stage)?;
            verify_entry_objects(&staging, &copy.entries, owner.bindings.limits.object_count)?;
            checkpoint(CaptureCheckpoint::ObjectsVerified)?;
            let expected_objects = CaptureObjectsRecordV1 {
                version: 1,
                capture_id: owner.capture_id.clone(),
                prior_sha256: expected_prior_sha256,
                readback_verified_at: timestamp()?,
            };
            let bytes = write_record_once(capture_journal, OBJECTS_RECORD, &expected_objects)?;
            checkpoint(CaptureCheckpoint::ObjectsCommitted)?;
            (expected_objects, bytes)
        }
    };

    let manifest = SubmissionManifestV1 {
        profile: SubmissionProfileV1::WorkspaceSnapshotV1,
        limit_profile: owner.bindings.limit_profile.clone(),
        effective_limits: owner.bindings.limits.clone(),
        entry_id: owner.bindings.entry_id.clone(),
        receipt_sha256: owner.bindings.receipt_sha256.clone(),
        cutoff_seq: owner.bindings.cutoff_seq,
        cutoff_chain_hash: Sha256Digest::try_from(owner.bindings.cutoff_chain_hash.as_str())
            .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?,
        finish_reason: owner.bindings.reason,
        entries: copy.entries,
        excluded: owner.bindings.excluded.iter().cloned().collect(),
        copy_started_at: copy.copy_started_at,
        copy_completed_at: copy.copy_completed_at,
        readback_verified_at: objects_record.readback_verified_at,
        object_count: copy.object_count,
        total_bytes: copy.total_bytes,
    };
    let manifest_bytes =
        canonical_manifest_bounded(&manifest, owner.bindings.capture_limits.manifest_bytes)?;
    let manifest_len = u64::try_from(manifest_bytes.len())
        .map_err(|_| failure(SubmissionFailureV1::LimitExceeded))?;
    let captured = SubmissionCapturedV1 {
        manifest_sha256: digest_bytes(&manifest_bytes),
    };
    let manifest_record = CaptureManifestRecordV1 {
        version: 1,
        capture_id: owner.capture_id.clone(),
        prior_sha256: digest_bytes(&objects_bytes),
        manifest_sha256: captured.manifest_sha256.clone(),
        manifest_bytes: manifest_len,
    };
    let manifest_record_bytes =
        write_record_once(capture_journal, MANIFEST_RECORD, &manifest_record)?;
    checkpoint(CaptureCheckpoint::ManifestCommitted)?;

    let expected_staged = CaptureStagedRecordV1 {
        version: 1,
        capture_id: owner.capture_id.clone(),
        prior_sha256: digest_bytes(&manifest_record_bytes),
    };
    let staged_bytes = match read_record_optional::<CaptureStagedRecordV1>(
        capture_journal,
        STAGED_RECORD,
        MAX_CAPTURE_JOURNAL_BYTES,
        SubmissionFailureV1::RecoveryAmbiguous,
    )? {
        Some((retained, bytes)) => {
            if retained != expected_staged {
                return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
            }
            bytes
        }
        None => {
            let staging = open_owned_stage(trace, stage)?;
            install_bytes_once(
                &staging,
                MANIFEST,
                &manifest_bytes,
                SubmissionFailureV1::RecoveryAmbiguous,
            )?;
            staging.sync().map_err(|_| storage_failure())?;
            checkpoint(CaptureCheckpoint::ManifestSynced)?;
            readback_from_submission(&staging, &captured, manifest_len)
                .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
            write_record_once(capture_journal, STAGED_RECORD, &expected_staged)?
        }
    };
    checkpoint(CaptureCheckpoint::StagedCommitted)?;

    let expected_promoted_prior = digest_bytes(&staged_bytes);
    let (promoted, promoted_bytes) = match read_record_optional::<CapturePromotedRecordV1>(
        capture_journal,
        PROMOTED_RECORD,
        MAX_CAPTURE_JOURNAL_BYTES,
        SubmissionFailureV1::RecoveryAmbiguous,
    )? {
        Some((retained, bytes)) => {
            if retained.version != 1
                || retained.capture_id != owner.capture_id
                || retained.prior_sha256 != expected_promoted_prior
            {
                return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
            }
            (retained, bytes)
        }
        None => {
            let promoted = promote_or_join_owned_stage(trace, stage)?;
            checkpoint(CaptureCheckpoint::Promoted)?;
            readback_from_submission(&promoted, &captured, manifest_len)
                .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
            checkpoint(CaptureCheckpoint::FinalReadbackComplete)?;
            let record = CapturePromotedRecordV1 {
                version: 1,
                capture_id: owner.capture_id.clone(),
                prior_sha256: expected_promoted_prior,
                submission_identity: promoted
                    .identity()
                    .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?,
            };
            let bytes = write_record_once(capture_journal, PROMOTED_RECORD, &record)?;
            (record, bytes)
        }
    };
    readback_promoted(trace, stage, &manifest_record, &promoted, &captured)?;

    let resolution = CaptureResolutionRecordV1 {
        version: 1,
        capture_id: owner.capture_id.clone(),
        prior_phase: CaptureResolutionPriorV1::Promoted,
        prior_sha256: digest_bytes(&promoted_bytes),
        outcome: SubmissionOutcomeV1::Captured(captured.clone()),
    };
    write_record_once(capture_journal, RESOLUTION_RECORD, &resolution)?;
    checkpoint(CaptureCheckpoint::ResolutionCommitted)?;
    Ok(captured)
}

fn open_owned_stage(
    trace: &TraceDir,
    stage: &CaptureStageRecordV1,
) -> Result<TraceDir, DaemonError> {
    let staging = trace
        .open_child_dir(&stage.staging_name)
        .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
    if staging
        .identity()
        .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?
        != stage.staging_identity
    {
        return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
    }
    Ok(staging)
}

fn promote_or_join_owned_stage(
    trace: &TraceDir,
    stage: &CaptureStageRecordV1,
) -> Result<TraceDir, DaemonError> {
    if let Ok(existing) = trace.open_child_dir(SUBMISSION) {
        if existing
            .identity()
            .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?
            == stage.staging_identity
        {
            return Ok(existing);
        }
        return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
    }
    let staging = open_owned_stage(trace, stage)?;
    match trace.promote_child_dir_matching_exclusive(&stage.staging_name, &staging, SUBMISSION) {
        Ok(()) => {}
        Err(_) => {
            let winner = trace
                .open_child_dir(SUBMISSION)
                .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
            if winner
                .identity()
                .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?
                != stage.staging_identity
            {
                return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
            }
            return Ok(winner);
        }
    }
    let promoted = trace
        .open_child_dir(SUBMISSION)
        .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
    let expected = staging
        .identity()
        .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
    if promoted
        .identity()
        .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?
        != expected
        || expected != stage.staging_identity
    {
        return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
    }
    Ok(promoted)
}

/// Resume only from the immutable capture journal and digest-committed staged bytes.
/// This path never requires or reopens the participant workspace.
pub fn recover_submission(
    trace: &TraceDir,
    journal: &SourceRootJournalV1,
    request: &SubmissionRequestV1,
) -> Result<SubmissionCapturedV1, DaemonError> {
    let excluded = normalised_exclusions(&request.excluded)?;
    let capture_journal = trace
        .open_child_dir(CAPTURE_JOURNAL)
        .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
    let _capture_lock = capture_journal
        .acquire_capture_lock()
        .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
    let bindings = capture_owner_bindings(journal, request, &excluded);
    let result = recover_committed_capture(trace, &capture_journal, journal, request, &excluded);
    conclude_capture_attempt(&capture_journal, &bindings, result, true)
}

pub fn readback_submission(
    trace: &TraceDir,
    captured: &SubmissionCapturedV1,
) -> Result<SubmissionManifestV1, DaemonError> {
    let capture_journal = trace
        .open_child_dir(CAPTURE_JOURNAL)
        .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?;
    let prefix = load_capture_prefix(&capture_journal)
        .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?;
    let (objects, objects_bytes): (CaptureObjectsRecordV1, Vec<u8>) = read_record_exact(
        &capture_journal,
        OBJECTS_RECORD,
        MAX_CAPTURE_JOURNAL_BYTES,
        SubmissionFailureV1::ReadbackFailed,
    )?;
    let (manifest_record, manifest_record_bytes): (CaptureManifestRecordV1, Vec<u8>) =
        read_record_exact(
            &capture_journal,
            MANIFEST_RECORD,
            MAX_CAPTURE_JOURNAL_BYTES,
            SubmissionFailureV1::ReadbackFailed,
        )?;
    let (staged, staged_bytes): (CaptureStagedRecordV1, Vec<u8>) = read_record_exact(
        &capture_journal,
        STAGED_RECORD,
        MAX_CAPTURE_JOURNAL_BYTES,
        SubmissionFailureV1::ReadbackFailed,
    )?;
    let (promoted, promoted_bytes): (CapturePromotedRecordV1, Vec<u8>) = read_record_exact(
        &capture_journal,
        PROMOTED_RECORD,
        MAX_CAPTURE_JOURNAL_BYTES,
        SubmissionFailureV1::ReadbackFailed,
    )?;
    let (resolution, _): (CaptureResolutionRecordV1, Vec<u8>) = read_record_exact(
        &capture_journal,
        RESOLUTION_RECORD,
        MAX_CAPTURE_JOURNAL_BYTES,
        SubmissionFailureV1::ReadbackFailed,
    )?;
    if objects.version != 1
        || objects.capture_id != prefix.owner.capture_id
        || objects.prior_sha256 != digest_bytes(&prefix.copy_bytes)
        || manifest_record.version != 1
        || manifest_record.capture_id != prefix.owner.capture_id
        || manifest_record.prior_sha256 != digest_bytes(&objects_bytes)
        || manifest_record.manifest_bytes > prefix.owner.bindings.capture_limits.manifest_bytes
        || staged.version != 1
        || staged.capture_id != prefix.owner.capture_id
        || staged.prior_sha256 != digest_bytes(&manifest_record_bytes)
        || promoted.version != 1
        || promoted.capture_id != prefix.owner.capture_id
        || promoted.prior_sha256 != digest_bytes(&staged_bytes)
        || resolution.version != 1
        || resolution.capture_id != prefix.owner.capture_id
        || resolution.prior_phase != CaptureResolutionPriorV1::Promoted
        || resolution.prior_sha256 != digest_bytes(&promoted_bytes)
        || resolution.outcome != SubmissionOutcomeV1::Captured(captured.clone())
        || manifest_record.manifest_sha256 != captured.manifest_sha256
    {
        return Err(failure(SubmissionFailureV1::ReadbackFailed));
    }
    let submission = trace
        .open_child_dir(SUBMISSION)
        .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?;
    let identity = submission
        .identity()
        .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?;
    if promoted.submission_identity != identity || prefix.stage.staging_identity != identity {
        return Err(failure(SubmissionFailureV1::ReadbackFailed));
    }
    let manifest = readback_from_submission(&submission, captured, manifest_record.manifest_bytes)?;
    validate_manifest_bindings(&manifest, &prefix, &objects)?;
    Ok(manifest)
}

#[allow(clippy::too_many_arguments)]
fn copy_dir(
    directory: &SourceDir,
    prefix: &str,
    excluded: &BTreeSet<String>,
    limits: &EffectiveSubmissionLimitsV1,
    namespace_limit: u64,
    object_store: &TraceDir,
    copied: &mut BTreeMap<Sha256Digest, u64>,
    entries: &mut Vec<SubmissionEntryV1>,
    total_bytes: &mut u64,
    namespace_count: &mut u64,
    manifest_bytes_used: &mut u64,
    manifest_limit: u64,
    checkpoint: &mut impl FnMut(CaptureCheckpoint) -> Result<(), DaemonError>,
) -> Result<(), DaemonError> {
    let before = directory.facts().map_err(source_failure)?;
    let remaining = namespace_limit.saturating_sub(*namespace_count);
    let children = listed(directory, remaining)?;
    for (name, observed) in children {
        *namespace_count = namespace_count
            .checked_add(1)
            .ok_or_else(|| failure(SubmissionFailureV1::LimitExceeded))?;
        if *namespace_count > namespace_limit {
            return Err(failure(SubmissionFailureV1::LimitExceeded));
        }
        let component = name
            .as_os_str()
            .to_str()
            .ok_or_else(|| failure(SubmissionFailureV1::UnsupportedEntry))?;
        let path = join_path(prefix, component);
        if is_excluded(&path, &excluded.iter().cloned().collect::<Vec<_>>()) {
            continue;
        }
        match observed.kind() {
            SourceKind::Directory => {
                let child = directory
                    .open_dir_matching(&name, &observed)
                    .map_err(source_failure)?;
                copy_dir(
                    &child,
                    &path,
                    excluded,
                    limits,
                    namespace_limit,
                    object_store,
                    copied,
                    entries,
                    total_bytes,
                    namespace_count,
                    manifest_bytes_used,
                    manifest_limit,
                    checkpoint,
                )?;
            }
            SourceKind::Regular => copy_regular(
                directory,
                &name,
                &observed,
                &path,
                limits,
                object_store,
                copied,
                entries,
                total_bytes,
                manifest_bytes_used,
                manifest_limit,
                checkpoint,
            )?,
            SourceKind::SymbolicLink => copy_symlink(
                directory,
                &name,
                &observed,
                &path,
                limits,
                object_store,
                copied,
                entries,
                total_bytes,
                manifest_bytes_used,
                manifest_limit,
            )?,
            SourceKind::Special => return Err(failure(SubmissionFailureV1::UnsupportedEntry)),
        }
    }
    if directory.facts().map_err(source_failure)? != before {
        return Err(failure(SubmissionFailureV1::SourceChanged));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn copy_regular(
    directory: &SourceDir,
    name: &SourceName,
    observed: &SourceObservation,
    path: &str,
    limits: &EffectiveSubmissionLimitsV1,
    object_store: &TraceDir,
    copied: &mut BTreeMap<Sha256Digest, u64>,
    entries: &mut Vec<SubmissionEntryV1>,
    total_bytes: &mut u64,
    manifest_bytes_used: &mut u64,
    manifest_limit: u64,
    checkpoint: &mut impl FnMut(CaptureCheckpoint) -> Result<(), DaemonError>,
) -> Result<(), DaemonError> {
    let facts = observed.facts();
    admit_entry(facts.length, limits, entries, total_bytes)?;
    let mut source = directory
        .open_regular_matching(name, observed, facts.length)
        .map_err(source_failure)?;
    let (digest, bytes) = stream_regular(&mut source, object_store, facts.length, checkpoint)?;
    source.finish().map_err(source_failure)?;
    retain_entry(
        path,
        SubmissionKindV1::Regular,
        facts.posix_owner_executable.unwrap_or(false),
        digest,
        bytes,
        limits,
        copied,
        entries,
        total_bytes,
        manifest_bytes_used,
        manifest_limit,
    )
}

#[allow(clippy::too_many_arguments)]
fn copy_symlink(
    directory: &SourceDir,
    name: &SourceName,
    observed: &SourceObservation,
    path: &str,
    limits: &EffectiveSubmissionLimitsV1,
    object_store: &TraceDir,
    copied: &mut BTreeMap<Sha256Digest, u64>,
    entries: &mut Vec<SubmissionEntryV1>,
    total_bytes: &mut u64,
    manifest_bytes_used: &mut u64,
    manifest_limit: u64,
) -> Result<(), DaemonError> {
    let facts = observed.facts();
    admit_entry(facts.length, limits, entries, total_bytes)?;
    let capacity = usize::try_from(
        facts
            .length
            .checked_add(1)
            .ok_or_else(|| failure(SubmissionFailureV1::LimitExceeded))?,
    )
    .map_err(|_| failure(SubmissionFailureV1::LimitExceeded))?;
    let mut bytes = vec![0u8; capacity];
    let count = directory
        .read_symlink_into_matching(name, observed, &mut bytes)
        .map_err(source_failure)?;
    bytes.truncate(count);
    let digest = Sha256Digest::from_bytes(Sha256::digest(&bytes).into());
    retain_object(object_store, &digest, &bytes)?;
    retain_entry(
        path,
        SubmissionKindV1::SymbolicLink,
        false,
        digest,
        u64::try_from(bytes.len()).map_err(|_| failure(SubmissionFailureV1::LimitExceeded))?,
        limits,
        copied,
        entries,
        total_bytes,
        manifest_bytes_used,
        manifest_limit,
    )
}

fn stream_regular(
    source: &mut impl std::io::Read,
    object_store: &TraceDir,
    expected: u64,
    checkpoint: &mut impl FnMut(CaptureCheckpoint) -> Result<(), DaemonError>,
) -> Result<(Sha256Digest, u64), DaemonError> {
    let mut buffer = [0u8; COPY_BUFFER];
    let mut bytes = 0u64;
    let mut hasher = Sha256::new();
    // Hash into a private temporary object first. Its final digest name is not known until EOF.
    let (temporary, mut output) = create_unique_private(object_store, "copy")?;
    loop {
        let read = source
            .read(&mut buffer)
            .map_err(|_| failure(SubmissionFailureV1::Unreadable))?;
        if read == 0 {
            break;
        }
        bytes = bytes
            .checked_add(u64::try_from(read).unwrap_or(u64::MAX))
            .ok_or_else(|| failure(SubmissionFailureV1::LimitExceeded))?;
        if bytes > expected {
            return Err(failure(SubmissionFailureV1::SourceChanged));
        }
        hasher.update(&buffer[..read]);
        write_all_storage(&mut output, &buffer[..read])?;
        checkpoint(CaptureCheckpoint::ObjectChunkWritten)?;
    }
    if bytes != expected {
        return Err(failure(SubmissionFailureV1::SourceChanged));
    }
    output.sync_all().map_err(|_| storage_failure())?;
    drop(output);
    let digest = Sha256Digest::from_bytes(hasher.finalize().into());
    match object_store.promote_child_exclusive(&temporary, digest.as_str()) {
        Ok(()) => Ok((digest, bytes)),
        Err(TraceCapError::Refused {
            reason: goalcup_platform::tracecap::RefusalKind::EntryAlreadyExists,
            ..
        }) => {
            let mut retained = object_store
                .read_regular(digest.as_str())
                .map_err(|_| storage_failure())?
                .ok_or_else(storage_failure)?;
            if !is_private_regular(&retained).map_err(|_| storage_failure())? {
                return Err(storage_failure());
            }
            verify_object(&mut retained, &digest, bytes)?;
            // Only this attempt owns the unpromoted temporary. Its full revalidation above
            // permits reuse of the digest-named object; remove no foreign name.
            object_store
                .remove_regular(&temporary)
                .map_err(|_| storage_failure())?;
            Ok((digest, bytes))
        }
        Err(_) => Err(storage_failure()),
    }
}

fn retain_object(
    object_store: &TraceDir,
    digest: &Sha256Digest,
    bytes: &[u8],
) -> Result<(), DaemonError> {
    install_bytes_once(
        object_store,
        digest.as_str(),
        bytes,
        SubmissionFailureV1::StorageFailed,
    )
}

#[allow(clippy::too_many_arguments)]
fn retain_entry(
    path: &str,
    kind: SubmissionKindV1,
    executable: bool,
    digest: Sha256Digest,
    bytes: u64,
    limits: &EffectiveSubmissionLimitsV1,
    copied: &mut BTreeMap<Sha256Digest, u64>,
    entries: &mut Vec<SubmissionEntryV1>,
    total_bytes: &mut u64,
    manifest_bytes_used: &mut u64,
    manifest_limit: u64,
) -> Result<(), DaemonError> {
    if let Some(previous) = copied.get(&digest) {
        if *previous != bytes {
            return Err(failure(SubmissionFailureV1::StorageFailed));
        }
    } else {
        copied.insert(digest.clone(), bytes);
        if u64::try_from(copied.len()).map_or(true, |count| count > limits.object_count) {
            return Err(failure(SubmissionFailureV1::LimitExceeded));
        }
    }
    *total_bytes = total_bytes
        .checked_add(bytes)
        .ok_or_else(|| failure(SubmissionFailureV1::LimitExceeded))?;
    if *total_bytes > limits.total_bytes {
        return Err(failure(SubmissionFailureV1::LimitExceeded));
    }
    let entry = SubmissionEntryV1 {
        path: PortableRelativePath::try_from(path)
            .map_err(|_| failure(SubmissionFailureV1::UnsupportedEntry))?,
        kind,
        posix_owner_executable: executable,
        bytes,
        content_sha256: digest,
    };
    charge_json(&entry, manifest_bytes_used, manifest_limit)?;
    entries.push(entry);
    Ok(())
}

fn admit_entry(
    bytes: u64,
    limits: &EffectiveSubmissionLimitsV1,
    entries: &[SubmissionEntryV1],
    total: &u64,
) -> Result<(), DaemonError> {
    if u64::try_from(entries.len()).map_or(true, |count| count >= limits.object_count)
        || bytes > limits.total_bytes.saturating_sub(*total)
    {
        return Err(failure(SubmissionFailureV1::LimitExceeded));
    }
    Ok(())
}

fn listed(
    directory: &SourceDir,
    namespace_limit: u64,
) -> Result<Vec<(SourceName, SourceObservation)>, DaemonError> {
    let mut names = Vec::new();
    let mut cursor = directory.entries().map_err(source_failure)?;
    while let Some(name) = cursor.next_entry().map_err(source_failure)? {
        if u64::try_from(names.len()).map_or(true, |count| count >= namespace_limit) {
            return Err(failure(SubmissionFailureV1::LimitExceeded));
        }
        names.push(name);
    }
    drop(cursor);
    names.sort_by(|left, right| left.as_os_str().cmp(right.as_os_str()));
    let mut observed = Vec::with_capacity(names.len());
    for name in names {
        match directory.observe(&name).map_err(source_failure)? {
            SourceLookup::Present(value) => observed.push((name, value)),
            SourceLookup::Missing => return Err(failure(SubmissionFailureV1::SourceChanged)),
        }
    }
    Ok(observed)
}

fn inventory(
    directory: &SourceDir,
    excluded: &BTreeSet<String>,
    limits: &EffectiveSubmissionLimitsV1,
    capture_limits: &CaptureResourceLimitsV1,
) -> Result<StableInventory, DaemonError> {
    let mut entries = Vec::new();
    let mut directories = Vec::new();
    let mut namespace_count = 1u64;
    let mut inventory_bytes = 512u64;
    if namespace_count > capture_limits.namespace_nodes {
        return Err(failure(SubmissionFailureV1::LimitExceeded));
    }
    inventory_dir(
        directory,
        "",
        excluded,
        limits.object_count,
        capture_limits.namespace_nodes,
        &mut namespace_count,
        &mut inventory_bytes,
        capture_limits.inventory_bytes,
        &mut entries,
        &mut directories,
    )?;
    Ok(StableInventory {
        version: 1,
        source_identity: directory.identity().journal_token(),
        entries,
        directories,
    })
}

#[allow(clippy::too_many_arguments)]
fn inventory_dir(
    directory: &SourceDir,
    prefix: &str,
    excluded: &BTreeSet<String>,
    entry_limit: u64,
    namespace_limit: u64,
    namespace_count: &mut u64,
    inventory_bytes: &mut u64,
    inventory_limit: u64,
    entries: &mut Vec<InventoryEntry>,
    directories: &mut Vec<InventoryEntry>,
) -> Result<(), DaemonError> {
    let before = directory.facts().map_err(source_failure)?;
    let retained_directory = InventoryEntry {
        path: prefix.to_string(),
        facts: before.clone().into(),
    };
    charge_json(&retained_directory, inventory_bytes, inventory_limit)?;
    directories.push(retained_directory);
    let remaining = namespace_limit.saturating_sub(*namespace_count);
    let children = listed(directory, remaining)?;
    for (name, observed) in children {
        let component = name
            .as_os_str()
            .to_str()
            .ok_or_else(|| failure(SubmissionFailureV1::UnsupportedEntry))?;
        let path = join_path(prefix, component);
        *namespace_count = namespace_count
            .checked_add(1)
            .ok_or_else(|| failure(SubmissionFailureV1::LimitExceeded))?;
        if *namespace_count > namespace_limit {
            return Err(failure(SubmissionFailureV1::LimitExceeded));
        }
        if is_excluded(&path, &excluded.iter().cloned().collect::<Vec<_>>()) {
            continue;
        }
        if observed.kind() == SourceKind::Directory {
            let child = directory
                .open_dir_matching(&name, &observed)
                .map_err(source_failure)?;
            inventory_dir(
                &child,
                &path,
                excluded,
                entry_limit,
                namespace_limit,
                namespace_count,
                inventory_bytes,
                inventory_limit,
                entries,
                directories,
            )?;
        } else {
            if u64::try_from(entries.len()).map_or(true, |count| count >= entry_limit) {
                return Err(failure(SubmissionFailureV1::LimitExceeded));
            }
            PortableRelativePath::try_from(path.as_str())
                .map_err(|_| failure(SubmissionFailureV1::UnsupportedEntry))?;
            let retained_entry = InventoryEntry {
                path,
                facts: observed.facts().into(),
            };
            charge_json(&retained_entry, inventory_bytes, inventory_limit)?;
            entries.push(retained_entry);
        }
    }
    if directory.facts().map_err(source_failure)? != before {
        return Err(failure(SubmissionFailureV1::SourceChanged));
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    directories.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(())
}

fn capture_owner_bindings(
    source: &SourceRootJournalV1,
    request: &SubmissionRequestV1,
    excluded: &BTreeSet<String>,
) -> CaptureOwnerBindingsV1 {
    CaptureOwnerBindingsV1 {
        version: 1,
        generation: request.generation,
        entry_id: request.entry_id.clone(),
        receipt_sha256: request.receipt_sha256.clone(),
        cutoff_seq: request.cutoff.seq,
        cutoff_prev: request.cutoff.prev.clone(),
        cutoff_chain_hash: request.cutoff.hash.clone(),
        reason: request.reason,
        excluded: excluded.clone(),
        limit_profile: request.limit_profile.clone(),
        limits: request.limits.clone(),
        capture_limits: request.capture_limits.clone(),
        source_identity: source.identity.clone(),
    }
}

fn validate_capture_limits(request: &SubmissionRequestV1) -> Result<(), DaemonError> {
    request
        .capture_limits
        .validate_for_config()
        .map_err(|_| failure(SubmissionFailureV1::LimitExceeded))?;
    if request.generation == 0
        || request.limits.object_count == 0
        || request.limits.total_bytes == 0
    {
        return Err(failure(SubmissionFailureV1::LimitExceeded));
    }
    Ok(())
}

fn validate_retained_capture_limits(limits: &CaptureResourceLimitsV1) -> Result<(), DaemonError> {
    limits
        .validate_for_config()
        .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))
}

struct BoundedBytes {
    bytes: Vec<u8>,
    limit: usize,
}

impl BoundedBytes {
    fn new(limit: u64, failure_reason: SubmissionFailureV1) -> Result<Self, DaemonError> {
        let limit = usize::try_from(limit).map_err(|_| failure(failure_reason))?;
        Ok(Self {
            bytes: Vec::with_capacity(limit.min(8 * 1024)),
            limit,
        })
    }
}

impl std::io::Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::FileTooLarge,
                "canonical bytes exceed capture bound",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn canonical_json_bounded(
    value: &impl Serialize,
    limit: u64,
    failure_reason: SubmissionFailureV1,
) -> Result<Vec<u8>, DaemonError> {
    let mut output = BoundedBytes::new(limit, failure_reason)?;
    serde_json::to_writer(&mut output, value).map_err(|_| failure(failure_reason))?;
    Ok(output.bytes)
}

fn canonical_manifest_bounded(
    manifest: &SubmissionManifestV1,
    limit: u64,
) -> Result<Vec<u8>, DaemonError> {
    manifest
        .validate()
        .map_err(|_| failure(SubmissionFailureV1::StorageFailed))?;
    canonical_json_bounded(manifest, limit, SubmissionFailureV1::LimitExceeded)
}

fn charge_json(value: &impl Serialize, used: &mut u64, limit: u64) -> Result<(), DaemonError> {
    let remaining = limit.saturating_sub(*used);
    let bytes = canonical_json_bounded(value, remaining, SubmissionFailureV1::LimitExceeded)?;
    *used = used
        .checked_add(
            u64::try_from(bytes.len()).map_err(|_| failure(SubmissionFailureV1::LimitExceeded))?,
        )
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| failure(SubmissionFailureV1::LimitExceeded))?;
    if *used > limit {
        return Err(failure(SubmissionFailureV1::LimitExceeded));
    }
    Ok(())
}

fn digest_bytes(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
}

fn recover_committed_capture(
    trace: &TraceDir,
    capture_journal: &TraceDir,
    source: &SourceRootJournalV1,
    request: &SubmissionRequestV1,
    excluded: &BTreeSet<String>,
) -> Result<SubmissionCapturedV1, DaemonError> {
    validate_capture_limits(request)?;
    let (owner, _) = load_capture_owner(capture_journal)?;
    let expected_bindings = capture_owner_bindings(source, request, excluded);
    if owner.bindings != expected_bindings {
        return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
    }
    if let Some((resolution, _)) = read_record_optional::<CaptureResolutionRecordV1>(
        capture_journal,
        RESOLUTION_RECORD,
        MAX_CAPTURE_JOURNAL_BYTES,
        SubmissionFailureV1::RecoveryAmbiguous,
    )? {
        validate_resolution_record(capture_journal, &owner, &resolution)?;
        if let SubmissionOutcomeV1::Incomplete { reason } = resolution.outcome {
            return Err(failure(reason));
        }
    }
    let prefix = load_capture_prefix(capture_journal)?;
    finish_capture_from_copy(
        trace,
        capture_journal,
        &prefix.owner,
        &prefix.stage,
        prefix.copy,
        prefix.copy_bytes,
        &mut |_| Ok(()),
    )
}

struct CapturePrefix {
    owner: CaptureOwnerRecordV1,
    stage: CaptureStageRecordV1,
    copy: CaptureCopyRecordV1,
    copy_bytes: Vec<u8>,
}

fn load_capture_owner(
    capture_journal: &TraceDir,
) -> Result<(CaptureOwnerRecordV1, Vec<u8>), DaemonError> {
    let (owner, owner_bytes): (CaptureOwnerRecordV1, Vec<u8>) = read_record_exact(
        capture_journal,
        OWNER_RECORD,
        MAX_CAPTURE_JOURNAL_BYTES,
        SubmissionFailureV1::ReadbackFailed,
    )?;
    validate_retained_capture_limits(&owner.bindings.capture_limits)?;
    let expected_id = capture_id_for_bindings(&owner.bindings)?;
    if owner.version != 1
        || owner.bindings.version != 1
        || owner.bindings.generation == 0
        || owner.capture_id != expected_id
        || owner.journal_identity
            != capture_journal
                .identity()
                .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?
        || owner.staging_name != format!("{STAGING_PREFIX}{}", expected_id.as_str())
    {
        return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
    }
    Ok((owner, owner_bytes))
}

fn load_capture_prefix(capture_journal: &TraceDir) -> Result<CapturePrefix, DaemonError> {
    let (owner, owner_bytes) = load_capture_owner(capture_journal)?;
    let (inventory, inventory_bytes): (CaptureInventoryRecordV1, Vec<u8>) = read_record_exact(
        capture_journal,
        INVENTORY_RECORD,
        owner.bindings.capture_limits.inventory_bytes,
        SubmissionFailureV1::RecoveryAmbiguous,
    )?;
    validate_inventory_record(&owner, &owner_bytes, &inventory)?;
    let (stage, stage_bytes): (CaptureStageRecordV1, Vec<u8>) = read_record_exact(
        capture_journal,
        STAGE_RECORD,
        MAX_CAPTURE_JOURNAL_BYTES,
        SubmissionFailureV1::RecoveryAmbiguous,
    )?;
    if stage.version != 1
        || stage.capture_id != owner.capture_id
        || stage.prior_sha256 != digest_bytes(&inventory_bytes)
        || stage.staging_name != owner.staging_name
        || stage.staging_identity.is_empty()
    {
        return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
    }
    let (copy, copy_bytes): (CaptureCopyRecordV1, Vec<u8>) = read_record_exact(
        capture_journal,
        COPY_RECORD,
        copy_record_limit(&owner.bindings.capture_limits)?,
        SubmissionFailureV1::RecoveryAmbiguous,
    )?;
    validate_copy_record(&owner, &inventory, &stage, &stage_bytes, &copy)?;
    Ok(CapturePrefix {
        owner,
        stage,
        copy,
        copy_bytes,
    })
}

fn capture_id_for_bindings(bindings: &CaptureOwnerBindingsV1) -> Result<Sha256Digest, DaemonError> {
    let bytes = canonical_json_bounded(
        bindings,
        MAX_CAPTURE_JOURNAL_BYTES,
        SubmissionFailureV1::RecoveryAmbiguous,
    )?;
    Ok(digest_bytes(&bytes))
}

fn conclude_capture_attempt(
    capture_journal: &TraceDir,
    expected_bindings: &CaptureOwnerBindingsV1,
    result: Result<SubmissionCapturedV1, DaemonError>,
    record_failure: bool,
) -> Result<SubmissionCapturedV1, DaemonError> {
    match result {
        Err(error) if record_failure => {
            let Some(reason) = error.submission_failure() else {
                return Err(error);
            };
            if !record_incomplete_resolution(capture_journal, expected_bindings, reason)? {
                return Err(error);
            }
            Err(error)
        }
        result => result,
    }
}

fn record_incomplete_resolution(
    capture_journal: &TraceDir,
    expected_bindings: &CaptureOwnerBindingsV1,
    reason: SubmissionFailureV1,
) -> Result<bool, DaemonError> {
    let (owner, _) = load_capture_owner(capture_journal)?;
    if owner.bindings != *expected_bindings {
        return Ok(false);
    }
    let (prior_phase, prior_sha256) = capture_journal_head(capture_journal, &owner)?;
    let resolution = CaptureResolutionRecordV1 {
        version: 1,
        capture_id: owner.capture_id,
        prior_phase,
        prior_sha256,
        outcome: SubmissionOutcomeV1::Incomplete { reason },
    };
    write_record_once(capture_journal, RESOLUTION_RECORD, &resolution)?;
    Ok(true)
}

fn validate_resolution_record(
    capture_journal: &TraceDir,
    owner: &CaptureOwnerRecordV1,
    resolution: &CaptureResolutionRecordV1,
) -> Result<(), DaemonError> {
    let (prior_phase, prior_sha256) = capture_journal_head(capture_journal, owner)?;
    if resolution.version != 1
        || resolution.capture_id != owner.capture_id
        || resolution.prior_phase != prior_phase
        || resolution.prior_sha256 != prior_sha256
        || matches!(
            resolution.outcome,
            SubmissionOutcomeV1::Captured(_) if prior_phase != CaptureResolutionPriorV1::Promoted
        )
    {
        return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
    }
    Ok(())
}

fn capture_journal_head(
    capture_journal: &TraceDir,
    owner: &CaptureOwnerRecordV1,
) -> Result<(CaptureResolutionPriorV1, Sha256Digest), DaemonError> {
    let records = [
        (
            CaptureResolutionPriorV1::Owner,
            OWNER_RECORD,
            MAX_CAPTURE_JOURNAL_BYTES,
        ),
        (
            CaptureResolutionPriorV1::Inventory,
            INVENTORY_RECORD,
            owner.bindings.capture_limits.inventory_bytes,
        ),
        (
            CaptureResolutionPriorV1::Stage,
            STAGE_RECORD,
            MAX_CAPTURE_JOURNAL_BYTES,
        ),
        (
            CaptureResolutionPriorV1::Copy,
            COPY_RECORD,
            copy_record_limit(&owner.bindings.capture_limits)?,
        ),
        (
            CaptureResolutionPriorV1::Objects,
            OBJECTS_RECORD,
            MAX_CAPTURE_JOURNAL_BYTES,
        ),
        (
            CaptureResolutionPriorV1::Manifest,
            MANIFEST_RECORD,
            MAX_CAPTURE_JOURNAL_BYTES,
        ),
        (
            CaptureResolutionPriorV1::Staged,
            STAGED_RECORD,
            MAX_CAPTURE_JOURNAL_BYTES,
        ),
        (
            CaptureResolutionPriorV1::Promoted,
            PROMOTED_RECORD,
            MAX_CAPTURE_JOURNAL_BYTES,
        ),
    ];
    let mut head = None;
    let mut missing = false;
    for (phase, name, limit) in records {
        let file = capture_journal
            .read_regular(name)
            .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
        match file {
            Some(file) if !missing => {
                let bytes =
                    read_private_bounded(file, limit, SubmissionFailureV1::RecoveryAmbiguous)?;
                head = Some((phase, digest_bytes(&bytes)));
            }
            Some(_) => return Err(failure(SubmissionFailureV1::RecoveryAmbiguous)),
            None => missing = true,
        }
    }
    head.ok_or_else(|| failure(SubmissionFailureV1::RecoveryAmbiguous))
}

fn copy_record_limit(limits: &CaptureResourceLimitsV1) -> Result<u64, DaemonError> {
    limits
        .manifest_bytes
        .checked_add(MAX_CAPTURE_JOURNAL_BYTES)
        .ok_or_else(|| failure(SubmissionFailureV1::LimitExceeded))
}

fn validate_inventory_record(
    owner: &CaptureOwnerRecordV1,
    owner_bytes: &[u8],
    inventory: &CaptureInventoryRecordV1,
) -> Result<(), DaemonError> {
    let entry_count = u64::try_from(inventory.inventory.entries.len())
        .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
    let directory_count = u64::try_from(inventory.inventory.directories.len())
        .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
    if inventory.version != 1
        || inventory.capture_id != owner.capture_id
        || inventory.prior_sha256 != digest_bytes(owner_bytes)
        || inventory.inventory.version != 1
        || inventory.inventory.source_identity != owner.bindings.source_identity
        || entry_count > owner.bindings.limits.object_count
        || entry_count
            .checked_add(directory_count)
            .is_none_or(|count| count > owner.bindings.capture_limits.namespace_nodes)
        || inventory
            .inventory
            .directories
            .first()
            .is_none_or(|entry| !entry.path.is_empty())
        || !strictly_sorted_inventory(&inventory.inventory.entries)
        || !strictly_sorted_inventory(&inventory.inventory.directories)
    {
        return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
    }
    for entry in &inventory.inventory.entries {
        if entry.facts.kind == InventoryKindV1::Directory
            || PortableRelativePath::try_from(entry.path.as_str()).is_err()
        {
            return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
        }
    }
    for directory in &inventory.inventory.directories {
        if directory.facts.kind != InventoryKindV1::Directory
            || (!directory.path.is_empty()
                && PortableRelativePath::try_from(directory.path.as_str()).is_err())
        {
            return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
        }
    }
    Ok(())
}

fn strictly_sorted_inventory(entries: &[InventoryEntry]) -> bool {
    entries.windows(2).all(|pair| pair[0].path < pair[1].path)
}

fn validate_copy_record(
    owner: &CaptureOwnerRecordV1,
    inventory: &CaptureInventoryRecordV1,
    stage: &CaptureStageRecordV1,
    stage_bytes: &[u8],
    copy: &CaptureCopyRecordV1,
) -> Result<(), DaemonError> {
    if copy.version != 1
        || copy.capture_id != owner.capture_id
        || copy.prior_sha256 != digest_bytes(stage_bytes)
        || copy.entries.len() != inventory.inventory.entries.len()
        || copy.entries.len() as u64 > owner.bindings.limits.object_count
    {
        return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
    }
    let mut total_bytes = 0u64;
    let mut objects = BTreeMap::<&Sha256Digest, u64>::new();
    for (entry, retained) in copy.entries.iter().zip(inventory.inventory.entries.iter()) {
        let kind_matches = matches!(
            (&entry.kind, retained.facts.kind),
            (SubmissionKindV1::Regular, InventoryKindV1::Regular)
                | (
                    SubmissionKindV1::SymbolicLink,
                    InventoryKindV1::SymbolicLink
                )
        );
        if entry.path.as_str() != retained.path
            || !kind_matches
            || entry.bytes != retained.facts.length
            || entry.posix_owner_executable
                != retained.facts.posix_owner_executable.unwrap_or(false)
            || (entry.kind == SubmissionKindV1::SymbolicLink && entry.posix_owner_executable)
        {
            return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
        }
        total_bytes = total_bytes
            .checked_add(entry.bytes)
            .ok_or_else(|| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
        match objects.get(&entry.content_sha256) {
            Some(bytes) if *bytes != entry.bytes => {
                return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
            }
            Some(_) => {}
            None => {
                objects.insert(&entry.content_sha256, entry.bytes);
            }
        }
    }
    if copy.total_bytes != total_bytes
        || total_bytes > owner.bindings.limits.total_bytes
        || copy.object_count != objects.len() as u64
        || copy.object_count > owner.bindings.limits.object_count
        || stage.capture_id != copy.capture_id
    {
        return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
    }
    Ok(())
}

fn validate_manifest_bindings(
    manifest: &SubmissionManifestV1,
    prefix: &CapturePrefix,
    objects: &CaptureObjectsRecordV1,
) -> Result<(), DaemonError> {
    if manifest.profile != SubmissionProfileV1::WorkspaceSnapshotV1
        || manifest.limit_profile != prefix.owner.bindings.limit_profile
        || manifest.effective_limits != prefix.owner.bindings.limits
        || manifest.entry_id != prefix.owner.bindings.entry_id
        || manifest.receipt_sha256 != prefix.owner.bindings.receipt_sha256
        || manifest.cutoff_seq != prefix.owner.bindings.cutoff_seq
        || manifest.cutoff_chain_hash.as_str() != prefix.owner.bindings.cutoff_chain_hash
        || manifest.finish_reason != prefix.owner.bindings.reason
        || manifest.entries != prefix.copy.entries
        || manifest.excluded
            != prefix
                .owner
                .bindings
                .excluded
                .iter()
                .cloned()
                .collect::<Vec<_>>()
        || manifest.copy_started_at != prefix.copy.copy_started_at
        || manifest.copy_completed_at != prefix.copy.copy_completed_at
        || manifest.readback_verified_at != objects.readback_verified_at
        || manifest.object_count != prefix.copy.object_count
        || manifest.total_bytes != prefix.copy.total_bytes
    {
        return Err(failure(SubmissionFailureV1::ReadbackFailed));
    }
    Ok(())
}

fn readback_promoted(
    trace: &TraceDir,
    stage: &CaptureStageRecordV1,
    manifest: &CaptureManifestRecordV1,
    promoted: &CapturePromotedRecordV1,
    captured: &SubmissionCapturedV1,
) -> Result<SubmissionCapturedV1, DaemonError> {
    let submission = trace
        .open_child_dir(SUBMISSION)
        .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
    let identity = submission
        .identity()
        .map_err(|_| failure(SubmissionFailureV1::RecoveryAmbiguous))?;
    if identity != stage.staging_identity || identity != promoted.submission_identity {
        return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
    }
    readback_from_submission(&submission, captured, manifest.manifest_bytes)?;
    Ok(captured.clone())
}

fn verify_entry_objects(
    submission: &TraceDir,
    entries: &[SubmissionEntryV1],
    object_limit: u64,
) -> Result<(), DaemonError> {
    let objects = submission
        .open_child_dir(OBJECTS)
        .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?;
    let store = objects
        .open_child_dir(SHA256)
        .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?;
    let expected = entries
        .iter()
        .map(|entry| (entry.content_sha256.clone(), entry.bytes))
        .collect::<BTreeMap<_, _>>();
    let names = store
        .child_names(object_limit)
        .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?;
    if names.len() != expected.len() {
        return Err(failure(SubmissionFailureV1::ReadbackFailed));
    }
    for (digest, bytes) in expected {
        let mut object = store
            .read_regular(digest.as_str())
            .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?
            .ok_or_else(|| failure(SubmissionFailureV1::ReadbackFailed))?;
        require_private_regular(&object)?;
        verify_object(&mut object, &digest, bytes)?;
    }
    Ok(())
}

fn write_record_once<T: Serialize + serde::de::DeserializeOwned + PartialEq>(
    parent: &TraceDir,
    name: &str,
    record: &T,
) -> Result<Vec<u8>, DaemonError> {
    write_record_once_bounded(parent, name, record, MAX_CAPTURE_JOURNAL_BYTES)
}

fn write_record_once_bounded<T: Serialize + serde::de::DeserializeOwned + PartialEq>(
    parent: &TraceDir,
    name: &str,
    record: &T,
    max_bytes: u64,
) -> Result<Vec<u8>, DaemonError> {
    let bytes = canonical_json_bounded(record, max_bytes, SubmissionFailureV1::LimitExceeded)?;
    if let Some((retained, retained_bytes)) = read_record_optional::<T>(
        parent,
        name,
        u64::try_from(bytes.len()).map_err(|_| storage_failure())?,
        SubmissionFailureV1::RecoveryAmbiguous,
    )? {
        return if retained == *record && retained_bytes == bytes {
            Ok(retained_bytes)
        } else {
            Err(failure(SubmissionFailureV1::RecoveryAmbiguous))
        };
    }
    install_bytes_once(parent, name, &bytes, SubmissionFailureV1::RecoveryAmbiguous)?;
    let (retained, retained_bytes) = read_record_exact::<T>(
        parent,
        name,
        u64::try_from(bytes.len()).map_err(|_| storage_failure())?,
        SubmissionFailureV1::RecoveryAmbiguous,
    )?;
    if retained != *record || retained_bytes != bytes {
        return Err(failure(SubmissionFailureV1::RecoveryAmbiguous));
    }
    Ok(retained_bytes)
}

fn install_bytes_once(
    parent: &TraceDir,
    name: &str,
    bytes: &[u8],
    mismatch: SubmissionFailureV1,
) -> Result<(), DaemonError> {
    let expected = u64::try_from(bytes.len()).map_err(|_| failure(mismatch))?;
    if let Some(retained) = parent.read_regular(name).map_err(|_| failure(mismatch))? {
        return if read_private_exact(retained, expected, mismatch)? == bytes {
            Ok(())
        } else {
            Err(failure(mismatch))
        };
    }
    let temporary_prefix = format!(".{name}.{}", digest_bytes(bytes).as_str());
    let (temporary, mut output) = create_unique_private(parent, &temporary_prefix)?;
    write_all_storage(&mut output, bytes)?;
    output.sync_all().map_err(|_| storage_failure())?;
    drop(output);
    let promoted = match parent.promote_child_exclusive(&temporary, name) {
        Ok(()) => true,
        Err(TraceCapError::Refused {
            reason: goalcup_platform::tracecap::RefusalKind::EntryAlreadyExists,
            ..
        }) => false,
        Err(_) => return Err(storage_failure()),
    };
    parent.sync().map_err(|_| storage_failure())?;
    let retained = parent
        .read_regular(name)
        .map_err(|_| failure(mismatch))?
        .ok_or_else(|| failure(mismatch))?;
    if read_private_exact(retained, expected, mismatch)? != bytes {
        return Err(failure(mismatch));
    }
    if !promoted {
        parent
            .remove_regular(&temporary)
            .map_err(|_| storage_failure())?;
        parent.sync().map_err(|_| storage_failure())?;
    }
    Ok(())
}

fn create_unique_private(parent: &TraceDir, prefix: &str) -> Result<(String, File), DaemonError> {
    for _ in 0..16 {
        let name = format!("{prefix}.{:016x}.tmp", rand::rngs::OsRng.next_u64());
        match parent.create_regular_exclusive(&name) {
            Ok(file) => return Ok((name, file)),
            Err(TraceCapError::Refused {
                reason: goalcup_platform::tracecap::RefusalKind::EntryAlreadyExists,
                ..
            }) => {}
            Err(_) => return Err(storage_failure()),
        }
    }
    Err(storage_failure())
}

fn read_record_optional<T: serde::de::DeserializeOwned + Serialize>(
    parent: &TraceDir,
    name: &str,
    max_bytes: u64,
    failure_reason: SubmissionFailureV1,
) -> Result<Option<(T, Vec<u8>)>, DaemonError> {
    let Some(file) = parent
        .read_regular(name)
        .map_err(|_| failure(failure_reason))?
    else {
        return Ok(None);
    };
    let bytes = read_private_bounded(file, max_bytes, failure_reason)?;
    let record: T = serde_json::from_slice(&bytes).map_err(|_| failure(failure_reason))?;
    if serde_json::to_vec(&record).map_err(|_| failure(failure_reason))? != bytes {
        return Err(failure(failure_reason));
    }
    Ok(Some((record, bytes)))
}

fn read_record_exact<T: serde::de::DeserializeOwned + Serialize>(
    parent: &TraceDir,
    name: &str,
    max_bytes: u64,
    failure_reason: SubmissionFailureV1,
) -> Result<(T, Vec<u8>), DaemonError> {
    read_record_optional(parent, name, max_bytes, failure_reason)?
        .ok_or_else(|| failure(failure_reason))
}

fn read_private_bounded(
    file: File,
    max_bytes: u64,
    failure_reason: SubmissionFailureV1,
) -> Result<Vec<u8>, DaemonError> {
    let length = file.metadata().map_err(|_| failure(failure_reason))?.len();
    if length > max_bytes {
        return Err(failure(failure_reason));
    }
    read_private_exact(file, length, failure_reason)
}

fn read_private_exact(
    file: File,
    expected_bytes: u64,
    failure_reason: SubmissionFailureV1,
) -> Result<Vec<u8>, DaemonError> {
    require_private_regular(&file).map_err(|_| failure(failure_reason))?;
    if file.metadata().map_err(|_| failure(failure_reason))?.len() != expected_bytes {
        return Err(failure(failure_reason));
    }
    let capacity = usize::try_from(expected_bytes).map_err(|_| failure(failure_reason))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(expected_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| failure(failure_reason))?;
    if bytes.len() != capacity {
        return Err(failure(failure_reason));
    }
    Ok(bytes)
}

fn write_all_storage(output: &mut impl std::io::Write, bytes: &[u8]) -> Result<(), DaemonError> {
    output.write_all(bytes).map_err(|_| storage_failure())
}

fn readback_from_submission(
    submission: &TraceDir,
    captured: &SubmissionCapturedV1,
    manifest_bytes: u64,
) -> Result<SubmissionManifestV1, DaemonError> {
    if submission
        .child_names(3)
        .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?
        != [MANIFEST.to_string(), OBJECTS.to_string()]
    {
        return Err(failure(SubmissionFailureV1::ReadbackFailed));
    }
    let file = submission
        .read_regular(MANIFEST)
        .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?
        .ok_or_else(|| failure(SubmissionFailureV1::ReadbackFailed))?;
    let retained_bytes =
        read_private_exact(file, manifest_bytes, SubmissionFailureV1::ReadbackFailed)?;
    if digest_bytes(&retained_bytes) != captured.manifest_sha256 {
        return Err(failure(SubmissionFailureV1::ReadbackFailed));
    }
    let manifest: SubmissionManifestV1 = serde_json::from_slice(&retained_bytes)
        .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?;
    manifest
        .validate()
        .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?;
    let canonical = manifest
        .canonical_bytes()
        .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?;
    if canonical != retained_bytes {
        return Err(failure(SubmissionFailureV1::ReadbackFailed));
    }
    let objects = submission
        .open_child_dir(OBJECTS)
        .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?;
    if objects
        .child_names(2)
        .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?
        != [SHA256.to_string()]
    {
        return Err(failure(SubmissionFailureV1::ReadbackFailed));
    }
    let store = objects
        .open_child_dir(SHA256)
        .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?;
    let expected = manifest
        .entries
        .iter()
        .map(|entry| (entry.content_sha256.clone(), entry.bytes))
        .collect::<BTreeMap<_, _>>();
    let names = store
        .child_names(manifest.effective_limits.object_count)
        .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?;
    if names.len() != expected.len() {
        return Err(failure(SubmissionFailureV1::ReadbackFailed));
    }
    for name in names {
        let digest = Sha256Digest::try_from(name.as_str())
            .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?;
        if !expected.contains_key(&digest) {
            return Err(failure(SubmissionFailureV1::ReadbackFailed));
        }
    }
    for (digest, bytes) in &expected {
        let mut object = store
            .read_regular(digest.as_str())
            .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?
            .ok_or_else(|| failure(SubmissionFailureV1::ReadbackFailed))?;
        require_private_regular(&object)?;
        verify_object(&mut object, digest, *bytes)?;
    }
    Ok(manifest)
}

fn verify_object(
    file: &mut File,
    expected_digest: &Sha256Digest,
    expected_bytes: u64,
) -> Result<(), DaemonError> {
    let mut buffer = [0u8; COPY_BUFFER];
    let mut bytes = 0u64;
    let mut hasher = Sha256::new();
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(u64::try_from(count).unwrap_or(u64::MAX))
            .ok_or_else(|| failure(SubmissionFailureV1::ReadbackFailed))?;
        hasher.update(&buffer[..count]);
    }
    if bytes != expected_bytes
        || Sha256Digest::from_bytes(hasher.finalize().into()) != *expected_digest
    {
        return Err(failure(SubmissionFailureV1::ReadbackFailed));
    }
    Ok(())
}

fn require_private_regular(file: &File) -> Result<(), DaemonError> {
    if !is_private_regular(file)? {
        return Err(failure(SubmissionFailureV1::ReadbackFailed));
    }
    Ok(())
}

fn is_private_regular(file: &File) -> Result<bool, DaemonError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        Ok(file
            .metadata()
            .map_err(|_| failure(SubmissionFailureV1::ReadbackFailed))?
            .mode()
            & 0o7777
            == 0o600)
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        Ok(true)
    }
}

fn normalised_exclusions(values: &[String]) -> Result<BTreeSet<String>, DaemonError> {
    let values = values.iter().cloned().collect::<BTreeSet<_>>();
    if values
        .iter()
        .any(|value| value.is_empty() || value.contains('\0'))
    {
        return Err(failure(SubmissionFailureV1::UnsupportedEntry));
    }
    Ok(values)
}

fn timestamp() -> Result<Rfc3339Utc, DaemonError> {
    Rfc3339Utc::try_from(now_rfc3339()).map_err(|_| failure(SubmissionFailureV1::StorageFailed))
}

fn join_path(prefix: &str, component: &str) -> String {
    if prefix.is_empty() {
        component.to_string()
    } else {
        format!("{prefix}/{component}")
    }
}

fn failure(reason: SubmissionFailureV1) -> DaemonError {
    DaemonError::SubmissionCapture { reason }
}

fn storage_failure() -> DaemonError {
    failure(SubmissionFailureV1::StorageFailed)
}

fn source_failure(error: SourceCapError) -> DaemonError {
    match error {
        SourceCapError::Refused {
            reason:
                goalcup_platform::sourcecap::RefusalKind::IdentityChanged
                | goalcup_platform::sourcecap::RefusalKind::MetadataChanged,
            ..
        } => failure(SubmissionFailureV1::SourceChanged),
        SourceCapError::Refused {
            reason:
                goalcup_platform::sourcecap::RefusalKind::WrongKind
                | goalcup_platform::sourcecap::RefusalKind::SpecialFile
                | goalcup_platform::sourcecap::RefusalKind::UnsupportedSymlinkPayload,
            ..
        } => failure(SubmissionFailureV1::UnsupportedEntry),
        SourceCapError::Refused {
            reason:
                goalcup_platform::sourcecap::RefusalKind::LengthLimit
                | goalcup_platform::sourcecap::RefusalKind::BufferLimit
                | goalcup_platform::sourcecap::RefusalKind::CursorActive
                | goalcup_platform::sourcecap::RefusalKind::PoisonedCursor
                | goalcup_platform::sourcecap::RefusalKind::PoisonedStream,
            ..
        } => failure(SubmissionFailureV1::LimitExceeded),
        SourceCapError::Budget(_) => failure(SubmissionFailureV1::LimitExceeded),
        SourceCapError::Io { .. } => failure(SubmissionFailureV1::Unreadable),
        SourceCapError::Refused { .. } => failure(SubmissionFailureV1::SourceChanged),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use goalcup_platform::handle_budget::NativeHandleBudgetV1;
    use std::io::{Error, Write};
    use std::path::PathBuf;
    use std::sync::mpsc::{TryRecvError, sync_channel};
    use std::time::Duration;

    fn request() -> SubmissionRequestV1 {
        SubmissionRequestV1 {
            generation: 9,
            entry_id: "entry-0001".to_string(),
            receipt_sha256: Sha256Digest::try_from("01".repeat(32)).unwrap(),
            cutoff: Link {
                seq: 11,
                prev: "02".repeat(32),
                hash: "03".repeat(32),
            },
            reason: FinishReasonV1::Manual,
            excluded: vec![".goalcup".to_string(), ".git".to_string()],
            limit_profile: LimitProfileV1 {
                id: "test-snapshot-v1".to_string(),
            },
            limits: EffectiveSubmissionLimitsV1 {
                object_count: 16,
                total_bytes: 1 << 20,
                pre_cutoff_drain: 1_000,
                process_walk: 64,
            },
            capture_limits: CaptureResourceLimitsV1 {
                version: 1,
                namespace_nodes: 1_024,
                inventory_bytes: 2 * 1024 * 1024,
                manifest_bytes: 2 * 1024 * 1024,
            },
        }
    }

    #[test]
    fn capture_resource_limit_validation_owns_version_positive_and_hard_maxima() {
        let valid = CaptureResourceLimitsV1 {
            version: 1,
            namespace_nodes: MAX_CAPTURE_NAMESPACE_NODES,
            inventory_bytes: MAX_CAPTURE_INVENTORY_BYTES,
            manifest_bytes: MAX_CAPTURE_MANIFEST_BYTES,
        };
        assert!(valid.validate_for_config().is_ok());

        for invalid in [
            CaptureResourceLimitsV1 {
                version: 2,
                ..valid.clone()
            },
            CaptureResourceLimitsV1 {
                namespace_nodes: 0,
                ..valid.clone()
            },
            CaptureResourceLimitsV1 {
                namespace_nodes: MAX_CAPTURE_NAMESPACE_NODES + 1,
                ..valid.clone()
            },
            CaptureResourceLimitsV1 {
                inventory_bytes: 0,
                ..valid.clone()
            },
            CaptureResourceLimitsV1 {
                inventory_bytes: MAX_CAPTURE_INVENTORY_BYTES + 1,
                ..valid.clone()
            },
            CaptureResourceLimitsV1 {
                manifest_bytes: 0,
                ..valid.clone()
            },
            CaptureResourceLimitsV1 {
                manifest_bytes: MAX_CAPTURE_MANIFEST_BYTES + 1,
                ..valid.clone()
            },
        ] {
            assert!(invalid.validate_for_config().is_err(), "{invalid:?}");
        }
    }

    fn fixture() -> (
        tempfile::TempDir,
        PathBuf,
        SourceDir,
        TraceDir,
        SourceRootJournalV1,
    ) {
        let temporary = tempfile::tempdir().unwrap();
        let workspace = temporary.path().join("workspace");
        let artifact_root = temporary.path().join("artifact-root");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&artifact_root).unwrap();
        std::fs::write(workspace.join("answer.txt"), b"pre-cutoff bytes").unwrap();
        let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
        let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();
        let trace = artifact.create_trace().unwrap();
        let journal = prepare_source_journal(&source, &artifact).unwrap();
        (temporary, workspace, source, trace, journal)
    }

    fn replacement_source(workspace: &PathBuf) -> SourceDir {
        let detached = workspace.with_file_name("detached-workspace");
        std::fs::rename(workspace, detached).unwrap();
        std::fs::create_dir(workspace).unwrap();
        std::fs::write(workspace.join("answer.txt"), b"post-cutoff replacement").unwrap();
        SourceDir::open_selected(workspace, &NativeHandleBudgetV1::new()).unwrap()
    }

    #[test]
    fn incomplete_crash_phases_never_resnapshot_after_the_source_handle_is_lost() {
        for target in [
            CaptureCheckpoint::OwnerCommitted,
            CaptureCheckpoint::FirstInventoryComplete,
            CaptureCheckpoint::InventoryCommitted,
            CaptureCheckpoint::StageCommitted,
            CaptureCheckpoint::ObjectChunkWritten,
            CaptureCheckpoint::CopyComplete,
        ] {
            let (_temporary, workspace, source, trace, journal) = fixture();
            let error = capture_submission_with_checkpoints(
                &source,
                &trace,
                &journal,
                &request(),
                &mut |checkpoint| {
                    if checkpoint == target {
                        Err(storage_failure())
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
            assert_eq!(
                error.submission_failure(),
                Some(SubmissionFailureV1::StorageFailed),
                "fault phase {target:?}"
            );

            drop(source);
            let _replacement = replacement_source(&workspace);
            let retry = recover_submission(&trace, &journal, &request()).unwrap_err();
            assert_eq!(
                retry.submission_failure(),
                Some(SubmissionFailureV1::RecoveryAmbiguous),
                "retry phase {target:?}"
            );
            assert!(trace.open_child_dir(SUBMISSION).is_err());
        }
    }

    #[test]
    fn digest_committed_crash_phases_recover_from_storage_only() {
        for target in [
            CaptureCheckpoint::CopyCommitted,
            CaptureCheckpoint::ObjectsVerified,
            CaptureCheckpoint::ObjectsCommitted,
            CaptureCheckpoint::ManifestCommitted,
            CaptureCheckpoint::ManifestSynced,
            CaptureCheckpoint::StagedCommitted,
            CaptureCheckpoint::Promoted,
            CaptureCheckpoint::FinalReadbackComplete,
            CaptureCheckpoint::ResolutionCommitted,
        ] {
            let (_temporary, workspace, source, trace, journal) = fixture();
            capture_submission_with_checkpoints(
                &source,
                &trace,
                &journal,
                &request(),
                &mut |checkpoint| {
                    if checkpoint == target {
                        Err(storage_failure())
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();

            drop(source);
            let _replacement = replacement_source(&workspace);
            let captured = recover_submission(&trace, &journal, &request()).unwrap();
            let manifest = readback_submission(&trace, &captured).unwrap();
            assert_eq!(manifest.entries.len(), 1, "recovery phase {target:?}");
            assert_eq!(manifest.entries[0].bytes, 16, "recovery phase {target:?}");
        }
    }

    #[test]
    fn concurrent_same_owner_waits_for_and_reuses_the_durable_resolution() {
        let (temporary, workspace, first_source, first_trace, journal) = fixture();
        let artifact_root = temporary.path().join("artifact-root");
        let second_source =
            SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
        let second_trace = EntryArtifactRoot::open(&artifact_root, "entry-0001")
            .unwrap()
            .open_trace()
            .unwrap();
        let second_journal = journal.clone();
        let request = request();
        let second_request = request.clone();
        let (owner_tx, owner_rx) = sync_channel(0);
        let (release_tx, release_rx) = sync_channel(0);
        let (second_started_tx, second_started_rx) = sync_channel(0);
        let (second_result_tx, second_result_rx) = sync_channel(0);

        std::thread::scope(|scope| {
            let first = scope.spawn(move || {
                capture_submission_with_checkpoints(
                    &first_source,
                    &first_trace,
                    &journal,
                    &request,
                    &mut |checkpoint| {
                        if checkpoint == CaptureCheckpoint::OwnerCommitted {
                            owner_tx.send(()).unwrap();
                            release_rx.recv().unwrap();
                        }
                        Ok(())
                    },
                )
            });
            owner_rx.recv().unwrap();
            let second = scope.spawn(move || {
                second_started_tx.send(()).unwrap();
                second_result_tx
                    .send(capture_submission(
                        &second_source,
                        &second_trace,
                        &second_journal,
                        &second_request,
                    ))
                    .unwrap();
            });
            second_started_rx.recv().unwrap();
            std::thread::sleep(Duration::from_millis(100));
            assert!(matches!(
                second_result_rx.try_recv(),
                Err(TryRecvError::Empty)
            ));
            release_tx.send(()).unwrap();

            let first_result = first.join().unwrap().unwrap();
            let second_result = second_result_rx.recv().unwrap().unwrap();
            second.join().unwrap();
            assert_eq!(second_result, first_result);
        });
    }

    #[test]
    fn mismatched_request_never_terminalizes_the_retained_owner_journal() {
        let (temporary, _workspace, source, trace, journal) = fixture();
        capture_submission_with_checkpoints(
            &source,
            &trace,
            &journal,
            &request(),
            &mut |checkpoint| {
                if checkpoint == CaptureCheckpoint::OwnerCommitted {
                    Err(storage_failure())
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();

        let mut mismatched = request();
        mismatched.generation += 1;
        let error = capture_submission(&source, &trace, &journal, &mismatched).unwrap_err();
        assert_eq!(
            error.submission_failure(),
            Some(SubmissionFailureV1::RecoveryAmbiguous)
        );
        assert!(
            !temporary
                .path()
                .join("artifact-root/artifacts/entry-0001/trace/.submission-capture-v1")
                .join(RESOLUTION_RECORD)
                .exists()
        );
    }

    #[test]
    fn insert_delete_and_hardlink_replacement_races_are_source_changes() {
        for race in ["insert", "delete", "hardlink"] {
            let (_temporary, workspace, source, trace, journal) = fixture();
            std::fs::write(workspace.join("donor.txt"), b"donor-content-01").unwrap();
            let error = capture_submission_with_checkpoints(
                &source,
                &trace,
                &journal,
                &request(),
                &mut |checkpoint| {
                    if checkpoint != CaptureCheckpoint::FirstInventoryComplete {
                        return Ok(());
                    }
                    match race {
                        "insert" => std::fs::write(workspace.join("inserted.txt"), b"new").unwrap(),
                        "delete" => std::fs::remove_file(workspace.join("answer.txt")).unwrap(),
                        "hardlink" => {
                            std::fs::remove_file(workspace.join("answer.txt")).unwrap();
                            std::fs::hard_link(
                                workspace.join("donor.txt"),
                                workspace.join("answer.txt"),
                            )
                            .unwrap();
                        }
                        _ => unreachable!(),
                    }
                    Ok(())
                },
            )
            .unwrap_err();
            assert_eq!(
                error.submission_failure(),
                Some(SubmissionFailureV1::SourceChanged),
                "race {race}"
            );
            assert!(trace.open_child_dir(SUBMISSION).is_err());
        }
    }

    #[test]
    fn manifest_timestamp_is_not_committed_before_object_readback_succeeds() {
        let (temporary, _workspace, source, trace, journal) = fixture();
        let trace_path = temporary
            .path()
            .join("artifact-root/artifacts/entry-0001/trace");
        let error = capture_submission_with_checkpoints(
            &source,
            &trace,
            &journal,
            &request(),
            &mut |checkpoint| {
                if checkpoint != CaptureCheckpoint::CopyComplete {
                    return Ok(());
                }
                let stage = std::fs::read_dir(&trace_path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| {
                        path.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with(STAGING_PREFIX)
                    })
                    .unwrap();
                let object = std::fs::read_dir(stage.join("objects/sha256"))
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path();
                std::fs::write(object, b"corrupted-object").unwrap();
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(
            error.submission_failure(),
            Some(SubmissionFailureV1::ReadbackFailed)
        );
        assert!(
            !trace_path
                .join(CAPTURE_JOURNAL)
                .join(MANIFEST_RECORD)
                .exists()
        );
        assert!(trace.open_child_dir(SUBMISSION).is_err());
    }

    #[test]
    fn canonical_manifest_check_is_independent_of_raw_digest_authentication() {
        let (temporary, _workspace, source, trace, journal) = fixture();
        let captured = capture_submission(&source, &trace, &journal, &request()).unwrap();
        let manifest_path = temporary
            .path()
            .join("artifact-root/artifacts/entry-0001/trace/submission/manifest.json");
        let canonical = std::fs::read(&manifest_path).unwrap();
        let mut noncanonical = b" \n".to_vec();
        noncanonical.extend_from_slice(&canonical);
        std::fs::write(&manifest_path, &noncanonical).unwrap();
        let submission = trace.open_child_dir(SUBMISSION).unwrap();
        let raw_authenticated = SubmissionCapturedV1 {
            manifest_sha256: digest_bytes(&noncanonical),
        };

        assert_eq!(
            readback_from_submission(
                &submission,
                &raw_authenticated,
                u64::try_from(noncanonical.len()).unwrap(),
            )
            .unwrap_err()
            .submission_failure(),
            Some(SubmissionFailureV1::ReadbackFailed)
        );
        assert_ne!(captured.manifest_sha256, raw_authenticated.manifest_sha256);
    }

    #[test]
    fn hardlink_alias_mutation_during_streaming_is_a_source_change() {
        let (_temporary, workspace, _source, trace, journal) = fixture();
        let answer = workspace.join("answer.txt");
        let alias = workspace.join("alias.txt");
        std::fs::write(&answer, vec![0x11; COPY_BUFFER * 2]).unwrap();
        std::fs::hard_link(&answer, &alias).unwrap();
        let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
        let mut mutated = false;

        let error = capture_submission_with_checkpoints(
            &source,
            &trace,
            &journal,
            &request(),
            &mut |checkpoint| {
                if checkpoint == CaptureCheckpoint::ObjectChunkWritten && !mutated {
                    std::fs::write(&alias, vec![0x22; COPY_BUFFER * 2]).unwrap();
                    mutated = true;
                }
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(
            error.submission_failure(),
            Some(SubmissionFailureV1::SourceChanged)
        );
        assert!(trace.open_child_dir(SUBMISSION).is_err());
    }

    #[derive(Default)]
    struct ChunkedWriter {
        bytes: Vec<u8>,
    }

    impl Write for ChunkedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let count = bytes.len().min(2);
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct EnospcWriter;

    impl Write for EnospcWriter {
        fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
            Err(Error::from_raw_os_error(28))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct ZeroWriter;

    impl Write for ZeroWriter {
        fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
            Ok(0)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn short_writes_complete_and_enospc_or_write_zero_fail_closed() {
        let expected = b"complete private record";
        let mut chunked = ChunkedWriter::default();
        write_all_storage(&mut chunked, expected).unwrap();
        assert_eq!(chunked.bytes, expected);

        for mut writer in [
            Box::new(EnospcWriter) as Box<dyn Write>,
            Box::new(ZeroWriter) as Box<dyn Write>,
        ] {
            assert_eq!(
                write_all_storage(&mut writer, expected)
                    .unwrap_err()
                    .submission_failure(),
                Some(SubmissionFailureV1::StorageFailed)
            );
        }
    }
}
