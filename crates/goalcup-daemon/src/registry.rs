//! The entry registry: every run this daemon knows about, and what it knows about it.
//!
//! An entry holds a workspace path, harness, manifest hash, session binding, signed receipt,
//! per-run keys and health. The receipt is here as an `Option<StartReceipt>` that is written
//! only after the document verified against the key this build pins, so a stored receipt is
//! a verified one by construction.
//!
//! D4 makes entries records inside one daemon, routed by working directory.
//!
//! **This is not the capture buffer.** Two redb databases, not one: a buffer corrupted
//! mid-run must still leave the daemon able to say what it was running, whose keys it
//! held and which receipt it had.
//!
//! **The file is a secret store.** It holds [`RunKeys`] in plaintext — the buffer key, the
//! per-run bearer and the signing seed that is
//! the run key itself. On unix the file is created `0o600` and a file found looser than
//! that is tightened on open; the directory [`Registry::default_path`] gives it is
//! `0o700`, and a directory the registry merely happens to sit in is left alone.
//! `RunKeys` redacts its own `Debug`, the way `control::WorkspaceInfo` already does, so
//! the same secrets cannot reach a log through an error message either.

use crate::config::ReceiptBindingPolicy;
use crate::submission_types::{CaptureResourceLimitsV1, SourceRootJournalV1};
use crate::{DaemonError, now_rfc3339};
use goalcup_platform::tracecap::ArtifactRoot;
use goalcup_protocol::chain::GENESIS;
use goalcup_protocol::chain::Link;
use goalcup_protocol::control::{EventStatus, WorkspaceFinalizationStatusV1};
use goalcup_protocol::cursor::StoreCursor;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::events::{HealthState, HomeSeedRecord, SeededKey};
use goalcup_protocol::export::{SEAL_FILE, TraceDescriptorDoc, manifest_sha256};
use goalcup_protocol::manifest::ForeignScanRoot;
use goalcup_protocol::process::{ProcessHandle, ProcessRegistration};
use goalcup_protocol::provenance::{
    PreCutoffDrainOutcomeV1, Rfc3339Utc, Sha256Digest, TerminationOutcomeV1,
};
use goalcup_protocol::receipt::StartReceipt;
#[cfg(feature = "v1-compat")]
use goalcup_protocol::receipt::StartReceiptClaims;
use goalcup_protocol::seal::SealManifest;
use goalcup_protocol::submission::{
    EffectiveSubmissionLimitsV1, FinishReasonV1, LimitProfileV1, SubmissionFailureV1,
    SubmissionOutcomeV1, SubmissionProfileV1,
};
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
#[cfg(any(test, feature = "test-hooks"))]
use std::sync::atomic::{AtomicBool, Ordering};

/// One JSON document per entry, keyed by `entry_id`. JSON rather than a hand-rolled
/// encoding so an added field cannot silently shift the meaning of the bytes already on
/// disk.
const ENTRIES: TableDefinition<&str, &[u8]> = TableDefinition::new("entries");

/// Daemon-wide scalars that are not any one entry's. The table is created so a registry
/// written by an older daemon opens unchanged.
const META: TableDefinition<&str, u16> = TableDefinition::new("meta");

/// Where an entry is in its life, from created to sealed.
///
/// `Refused` is terminal and distinct from removal: a start refused before dispatch leaves
/// no entry ([`Registry::remove`]), and this state is for a run refused *after*
/// it existed, which the participant is owed an explanation for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryState {
    Created,
    Armed,
    StartSignalSeen,
    Started,
    /// Recursive abandon cleanup has begun and must finish before this row can disappear.
    /// This is capture-closed and restart-recoverable; it is never restored to an ordinary
    /// runtime after any artifact may already have been removed.
    CleanupPending,
    /// A seal has reserved the durable state transition but has not yet been reconciled
    /// against the terminal chain link.  Startup either restores `sealing_from_state` when
    /// no link exists or promotes this to `Sealed` after re-buffering that exact link.
    Sealing,
    Sealed,
    Refused,
}

/// Durable, monotone workspace-finalization checkpoints.
///
/// Every phase names the inclusive fact already committed. In particular,
/// `SubmissionResolved` covers both a captured submission and an explicit incomplete
/// outcome; calling that checkpoint "artifact verified" would overstate negative evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalizationPhaseV1 {
    AdmissionClosed,
    DrainResolved,
    CutoffAppendPending,
    CutoffBound,
    HandoffReconciled,
    Terminated,
    SubmissionResolved,
    FinalizationAppendPending,
    FinalizationBound,
    TerminalStatusPending,
    Terminal,
}

impl FinalizationPhaseV1 {
    /// Whether `next` is this checkpoint or a later checkpoint. Recovery may replay one
    /// committed phase but may never move an intent backward.
    pub const fn permits(self, next: Self) -> bool {
        self.rank() <= next.rank()
    }

    const fn rank(self) -> u8 {
        match self {
            Self::AdmissionClosed => 0,
            Self::DrainResolved => 1,
            Self::CutoffAppendPending => 2,
            Self::CutoffBound => 3,
            Self::HandoffReconciled => 4,
            Self::Terminated => 5,
            Self::SubmissionResolved => 6,
            Self::FinalizationAppendPending => 7,
            Self::FinalizationBound => 8,
            Self::TerminalStatusPending => 9,
            Self::Terminal => 10,
        }
    }
}

/// Exact V1 discriminator for the workspace arm of the untagged registry intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceIntentVersionV1 {
    WorkspaceSubmissionV1,
}

/// Positions that were admitted when the registry transaction closed capture.
///
/// `tailers` are the record-count positions for every declared leg-1 store. Receiver
/// sources are kept separately because their counter spaces are independent. The chain
/// predecessor binds both legs to the exact durable history they had produced at the
/// boundary; none of these values contain unredacted record bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionWatermarksV1 {
    pub tailers: Vec<StoreCursor>,
    pub receiver_sources: BTreeMap<String, u64>,
    pub queued_receiver_posts: u64,
    pub admitted_chain_predecessor: Option<ChainPositionV1>,
}

/// The identity of a chain head already validated by the retained writer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChainPositionV1 {
    pub seq: u64,
    pub hash: Sha256Digest,
}

/// A planned append predecessor must distinguish "not prepared yet" from genesis.
/// The containing `Option` carries the former; this enum carries the latter or the exact
/// link already read from the validated chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainPredecessorV1 {
    Genesis,
    Position(ChainPositionV1),
}

/// The registered launch authority latched by the same transaction as finalization.
/// `registration` uses the protocol's strict nested decoder; callers construct it from a
/// seed-stripped handle so private home-seed values never enter this registry document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedProcessHandoffV1 {
    pub request_digest: Sha256Digest,
    pub registration: ProcessRegistration,
}

/// Source authority fixed by the same transaction that closes V4 admission. A daemon
/// present at cutoff retains the writer's opaque source journal; a restart that first
/// observes the signed cutoff can only persist that no source capability survived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "availability", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkspaceSourceBindingV1 {
    Available { journal: SourceRootJournalV1 },
    UnavailableAtCutoff { reason: SubmissionFailureV1 },
}

impl WorkspaceSourceBindingV1 {
    fn validate(&self) -> bool {
        match self {
            Self::Available { journal } => !journal.identity.is_empty(),
            Self::UnavailableAtCutoff { reason } => {
                *reason == SubmissionFailureV1::RecoveryAmbiguous
            }
        }
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn journal(&self) -> Option<&SourceRootJournalV1> {
        match self {
            Self::Available { journal } => Some(journal),
            Self::UnavailableAtCutoff { .. } => None,
        }
    }
}

/// Exact V4 workspace finalization authority persisted in the existing sealing slot.
///
/// Every optional field is controlled by `phase`; a restart reads the committed phase and
/// reconciles only the immediately corresponding side effect. No field is inferred from a
/// live pathname, a missing process, or the current wall clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSealingIntentV1 {
    pub version: WorkspaceIntentVersionV1,
    pub entry_id: String,
    pub generation: u64,
    pub reason: FinishReasonV1,
    pub receipt_sha256: Sha256Digest,
    pub cutoff_at: Rfc3339Utc,
    /// Opaque identity of the retained trace-root capability that owned admission.
    pub trace_root_identity: String,
    /// Exact date tokens used to render the admitted tailer paths. Recovery reuses these
    /// bytes and never substitutes its current calendar day.
    #[serde(default)]
    pub source_date: [String; 3],
    pub source_root: WorkspaceSourceBindingV1,
    pub submission_profile: SubmissionProfileV1,
    pub limits: EffectiveSubmissionLimitsV1,
    pub limit_profile: LimitProfileV1,
    pub capture_limits: CaptureResourceLimitsV1,
    pub excluded: Vec<String>,
    pub phase: FinalizationPhaseV1,
    pub admission_watermarks: AdmissionWatermarksV1,
    pub expected_cutoff_predecessor: Option<ChainPredecessorV1>,
    pub drain: Option<PreCutoffDrainOutcomeV1>,
    pub cutoff: Option<Link>,
    pub accepted_handoff: Option<AcceptedProcessHandoffV1>,
    pub termination: Option<TerminationOutcomeV1>,
    /// What the kill actually did, in one line, written in the same transaction as
    /// `termination`.
    ///
    /// The typed outcome is a verdict with three values; this is the measurement it was
    /// read from — which pids were owned, which were still alive at the end, how long the
    /// escalation took, and whether containment or the walk budget held. The daemon records
    /// facts and the verifier judges them, so an `Unresolved` record that named no survivor
    /// would be a judgement with its evidence thrown away.
    ///
    /// `default` because rows written before this field existed are still valid rows: an
    /// absent measurement is an older daemon, not a damaged authority.
    #[serde(default)]
    pub termination_report: Option<String>,
    pub submission: Option<SubmissionOutcomeV1>,
    pub finalization_at: Option<Rfc3339Utc>,
    pub expected_finalization_predecessor: Option<ChainPredecessorV1>,
    pub finalization: Option<Link>,
    pub terminal_status_at: Option<Rfc3339Utc>,
    pub status_mirror_complete: bool,
}

fn valid_source_date(date: &[String; 3]) -> bool {
    date[0].len() == 4
        && date[1].len() == 2
        && date[2].len() == 2
        && date
            .iter()
            .all(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
        && date[1]
            .parse::<u8>()
            .is_ok_and(|month| (1..=12).contains(&month))
        && date[2]
            .parse::<u8>()
            .is_ok_and(|day| (1..=31).contains(&day))
}

impl WorkspaceSealingIntentV1 {
    fn validate(&self, entry_id: &str) -> Result<(), DaemonError> {
        if self.entry_id != entry_id {
            return Err(DaemonError::Refused(format!(
                "workspace finalization intent belongs to {}, not entry {entry_id}",
                self.entry_id
            )));
        }
        if self.trace_root_identity.is_empty()
            || !valid_source_date(&self.source_date)
            || !self.source_root.validate()
            || self.capture_limits.validate_for_config().is_err()
            || self
                .excluded
                .iter()
                .any(|path| path.is_empty() || path.contains('\0'))
            || self.limit_profile.id.is_empty()
            || self.limit_profile.id.len() > 256
            || self
                .limit_profile
                .id
                .bytes()
                .any(|byte| byte.is_ascii_control())
            || [
                self.limits.object_count,
                self.limits.total_bytes,
                self.limits.pre_cutoff_drain,
                self.limits.process_walk,
            ]
            .contains(&0)
        {
            return Err(DaemonError::Refused(format!(
                "workspace finalization intent for entry {entry_id} has invalid source/profile limits"
            )));
        }
        if let Some(authority) = &self.accepted_handoff {
            let expected = authority
                .registration
                .canonical_handle_digest(&self.entry_id);
            if authority.registration.generation != self.generation
                || authority.request_digest.as_str() != expected
            {
                return Err(DaemonError::Refused(format!(
                    "workspace finalization intent for entry {entry_id} has mismatched handoff authority"
                )));
            }
        }
        self.validate_phase_presence(entry_id)?;
        if let Some(cutoff) = &self.cutoff {
            validate_successor(
                &self.expected_cutoff_predecessor,
                cutoff,
                "cutoff",
                entry_id,
            )?;
        }
        if let Some(expected) = &self.expected_finalization_predecessor
            && !matches!(
                expected,
                ChainPredecessorV1::Position(position)
                    if self.cutoff.as_ref().is_some_and(|cutoff|
                        cutoff.seq == position.seq && cutoff.hash == position.hash.as_str())
            )
        {
            return Err(DaemonError::Refused(format!(
                "workspace finalization intent for entry {entry_id} does not bind finalization to its cutoff"
            )));
        }
        if let Some(finalization) = &self.finalization {
            validate_successor(
                &self.expected_finalization_predecessor,
                finalization,
                "finalization",
                entry_id,
            )?;
        }
        Ok(())
    }

    #[cfg(feature = "v1-compat")]
    fn validate_admission_authority(&self, entry: &Entry) -> Result<(), DaemonError> {
        self.validate_bound_authority(entry, true)
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn validate_recovery_authority(&self, entry: &Entry) -> Result<(), DaemonError> {
        self.validate_bound_authority(entry, false)
    }

    #[cfg(feature = "v1-compat")]
    fn validate_bound_authority(
        &self,
        entry: &Entry,
        require_admission_head: bool,
    ) -> Result<(), DaemonError> {
        let entry_id = &entry.entry_id;
        if (require_admission_head && entry.state != EntryState::Started)
            || entry.started_at.as_deref().is_none_or(str::is_empty)
            || entry.session_id.as_deref().is_none_or(str::is_empty)
        {
            return Err(DaemonError::Refused(format!(
                "entry {entry_id} is not a durably started run and cannot enter V4 finalization"
            )));
        }
        let receipt = entry.receipt().ok_or_else(|| {
            DaemonError::Refused(format!(
                "entry {entry_id} has no verified start receipt for V4 finalization"
            ))
        })?;
        if receipt.canonical_digest()? != self.receipt_sha256
            || entry.source_date() != Some(&self.source_date)
            || entry.deadline_at.as_deref() != Some(receipt.claims.deadline_at.as_str())
            || receipt.claims.started_at.as_str() != entry.started_at.as_deref().unwrap()
            || receipt.claims.session_id.as_str() != entry.session_id.as_deref().unwrap()
            || receipt.claims.run_public_key.as_deref()
                != Some(entry.keys.public_key_hex()?.as_str())
        {
            return Err(DaemonError::Refused(format!(
                "entry {entry_id} V4 finalization does not match its verified receipt authority"
            )));
        }
        let deadline =
            Rfc3339Utc::try_from(receipt.claims.deadline_at.clone()).map_err(|error| {
                DaemonError::Refused(format!(
                    "entry {entry_id} V4 receipt has an invalid deadline: {error}"
                ))
            })?;
        match self.reason {
            FinishReasonV1::Deadline if self.cutoff_at != deadline => {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} deadline finalization did not latch the signed deadline"
                )));
            }
            FinishReasonV1::Manual
                if goalcup_protocol::control::parse_z(self.cutoff_at.as_str())
                    >= goalcup_protocol::control::parse_z(deadline.as_str()) =>
            {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} manual finalization is not earlier than the signed deadline"
                )));
            }
            FinishReasonV1::Manual | FinishReasonV1::Deadline => {}
        }
        let ReceiptBindingPolicy::WorkspaceSubmission {
            questionnaire_version,
            submission_profile,
            limit_profile,
            effective_limits,
            capture_limits,
        } = entry.receipt_binding()
        else {
            return Err(DaemonError::Refused(format!(
                "entry {entry_id} is not configured for workspace-submission finalization"
            )));
        };
        if receipt.claims.questionnaire_version.as_deref() != Some(questionnaire_version)
            || self.submission_profile != *submission_profile
            || self.limit_profile != *limit_profile
            || self.limits != *effective_limits
            || self.capture_limits != *capture_limits
            || self.generation != entry.process_handoff_generation()
        {
            return Err(DaemonError::Refused(format!(
                "entry {entry_id} V4 finalization tried to replace its latched profile, limits, or generation"
            )));
        }

        let accepted = entry.verified_registered_termination_registration()?;
        if self.accepted_handoff != accepted
            || (matches!(
                entry.expected_process_handoff(),
                ExpectedProcessHandoff::Required
            ) && accepted.is_none())
            || matches!(
                entry.expected_process_handoff(),
                ExpectedProcessHandoff::LegacyUnknown
            )
        {
            return Err(DaemonError::Refused(format!(
                "entry {entry_id} V4 finalization does not latch its exact accepted process authority"
            )));
        }

        if require_admission_head {
            let admitted_head = match entry.chain_seq {
                Some(seq) => Some(ChainPositionV1 {
                    seq,
                    hash: Sha256Digest::try_from(entry.chain_head.as_str()).map_err(|error| {
                        DaemonError::Refused(format!(
                            "entry {entry_id} has a non-canonical chain head at admission: {error}"
                        ))
                    })?,
                }),
                None if entry.chain_head == GENESIS => None,
                None => {
                    return Err(DaemonError::Refused(format!(
                        "entry {entry_id} has no chain position but does not retain genesis"
                    )));
                }
            };
            if self.admission_watermarks.admitted_chain_predecessor != admitted_head {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} V4 finalization admission watermark does not match the durable chain mirror"
                )));
            }
        }
        Ok(())
    }

    fn validate_phase_presence(&self, entry_id: &str) -> Result<(), DaemonError> {
        let exact = [
            (
                FinalizationPhaseV1::DrainResolved,
                self.drain.is_some(),
                "drain",
            ),
            (
                FinalizationPhaseV1::CutoffAppendPending,
                self.expected_cutoff_predecessor.is_some(),
                "cutoff predecessor",
            ),
            (
                FinalizationPhaseV1::CutoffBound,
                self.cutoff.is_some(),
                "cutoff",
            ),
            (
                FinalizationPhaseV1::Terminated,
                self.termination.is_some(),
                "termination outcome",
            ),
            (
                FinalizationPhaseV1::SubmissionResolved,
                self.submission.is_some(),
                "submission outcome",
            ),
            (
                FinalizationPhaseV1::FinalizationAppendPending,
                self.finalization_at.is_some(),
                "finalization timestamp",
            ),
            (
                FinalizationPhaseV1::FinalizationAppendPending,
                self.expected_finalization_predecessor.is_some(),
                "finalization predecessor",
            ),
            (
                FinalizationPhaseV1::FinalizationBound,
                self.finalization.is_some(),
                "finalization link",
            ),
            (
                FinalizationPhaseV1::TerminalStatusPending,
                self.terminal_status_at.is_some(),
                "terminal status timestamp",
            ),
            (
                FinalizationPhaseV1::Terminal,
                self.status_mirror_complete,
                "status mirror",
            ),
        ];
        for (phase, present, fact) in exact {
            let expected_present = phase.permits(self.phase);
            if present != expected_present {
                return Err(DaemonError::Refused(format!(
                    "workspace finalization intent for entry {entry_id} has {fact} presence {present} at phase {:?}, expected {expected_present}",
                    self.phase,
                )));
            }
        }
        Ok(())
    }

    fn status_projection(&self) -> WorkspaceFinalizationStatusV1 {
        if matches!(
            self.phase,
            FinalizationPhaseV1::TerminalStatusPending | FinalizationPhaseV1::Terminal
        ) {
            return match (&self.drain, &self.termination, &self.submission) {
                (Some(pre_cutoff_drain), Some(termination), Some(submission)) => {
                    WorkspaceFinalizationStatusV1::Terminal {
                        pre_cutoff_drain: pre_cutoff_drain.clone(),
                        termination: termination.clone(),
                        submission: submission.clone(),
                    }
                }
                _ => WorkspaceFinalizationStatusV1::Incomplete {
                    pre_cutoff_drain: self.drain.clone(),
                    termination: self.termination.clone(),
                    submission: self.submission.clone(),
                },
            };
        }

        let drain_incomplete = matches!(self.drain, Some(PreCutoffDrainOutcomeV1::Degraded { .. }));
        let termination_incomplete = self
            .termination
            .as_ref()
            .is_some_and(|outcome| !matches!(outcome, TerminationOutcomeV1::Confirmed { .. }));
        let submission_incomplete = matches!(
            self.submission,
            Some(SubmissionOutcomeV1::Incomplete { .. })
        );
        if drain_incomplete || termination_incomplete || submission_incomplete {
            WorkspaceFinalizationStatusV1::Incomplete {
                pre_cutoff_drain: self.drain.clone(),
                termination: self.termination.clone(),
                submission: self.submission.clone(),
            }
        } else {
            WorkspaceFinalizationStatusV1::Finishing {}
        }
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn matches_terminal_envelope(&self, terminal: &Envelope) -> Result<(), DaemonError> {
        let expected_link = self.finalization.as_ref().ok_or_else(|| {
            DaemonError::Refused(format!(
                "workspace finalization for entry {} has no terminal link",
                self.entry_id
            ))
        })?;
        let expected_at = self.finalization_at.as_ref().ok_or_else(|| {
            DaemonError::Refused(format!(
                "workspace finalization for entry {} has no terminal timestamp",
                self.entry_id
            ))
        })?;
        let expected_body = EventBody::FinalizationV1(
            goalcup_protocol::provenance::FinalizationV1::WorkspaceSubmission {
                receipt_sha256: self.receipt_sha256.clone(),
                termination: self.termination.clone().ok_or_else(|| {
                    DaemonError::Refused(format!(
                        "workspace finalization for entry {} has no termination outcome",
                        self.entry_id
                    ))
                })?,
                submission: self.submission.clone().ok_or_else(|| {
                    DaemonError::Refused(format!(
                        "workspace finalization for entry {} has no submission outcome",
                        self.entry_id
                    ))
                })?,
            },
        );
        if terminal.entry_id != self.entry_id
            || terminal.source != "daemon"
            || terminal.recorded_at != expected_at.as_str()
            || terminal.link != *expected_link
            || terminal.redaction.is_some()
            || terminal.body != expected_body
        {
            return Err(DaemonError::Refused(format!(
                "entry {} terminal envelope does not match its durable workspace finalization",
                self.entry_id
            )));
        }
        Ok(())
    }
}

/// Resolve the one organizer-authoritative cutoff decision. The caller invokes this from
/// the registry admission transaction: at/after the signed deadline the receipt wins over
/// a racing manual request, while an early deadline request cannot shorten organizer time.
#[cfg(feature = "v1-compat")]
pub(crate) fn resolve_finish_boundary(
    requested: FinishReasonV1,
    observed_at: &Rfc3339Utc,
    deadline_at: &Rfc3339Utc,
    deadline_was_observed: bool,
) -> Result<(FinishReasonV1, Rfc3339Utc), DaemonError> {
    let observed = goalcup_protocol::control::parse_z(observed_at.as_str()).ok_or_else(|| {
        DaemonError::Refused("finalization observed_at is not a protocol timestamp".to_string())
    })?;
    let deadline = goalcup_protocol::control::parse_z(deadline_at.as_str()).ok_or_else(|| {
        DaemonError::Refused("verified receipt deadline is not a protocol timestamp".to_string())
    })?;
    if deadline_was_observed || observed >= deadline {
        return Ok((FinishReasonV1::Deadline, deadline_at.clone()));
    }
    match requested {
        FinishReasonV1::Manual => Ok((FinishReasonV1::Manual, observed_at.clone())),
        FinishReasonV1::Deadline => Err(DaemonError::Refused(
            "deadline finalization was requested before the signed deadline".to_string(),
        )),
    }
}

#[cfg(feature = "v1-compat")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceIntentImmutableV1 {
    version: WorkspaceIntentVersionV1,
    entry_id: String,
    generation: u64,
    reason: FinishReasonV1,
    receipt_sha256: Sha256Digest,
    cutoff_at: Rfc3339Utc,
    trace_root_identity: String,
    source_root: WorkspaceSourceBindingV1,
    submission_profile: SubmissionProfileV1,
    limits: EffectiveSubmissionLimitsV1,
    limit_profile: LimitProfileV1,
    capture_limits: CaptureResourceLimitsV1,
    excluded: Vec<String>,
    admission_watermarks: AdmissionWatermarksV1,
    accepted_handoff: Option<AcceptedProcessHandoffV1>,
}

#[cfg(feature = "v1-compat")]
impl From<&WorkspaceSealingIntentV1> for WorkspaceIntentImmutableV1 {
    fn from(intent: &WorkspaceSealingIntentV1) -> Self {
        Self {
            version: intent.version,
            entry_id: intent.entry_id.clone(),
            generation: intent.generation,
            reason: intent.reason,
            receipt_sha256: intent.receipt_sha256.clone(),
            cutoff_at: intent.cutoff_at.clone(),
            trace_root_identity: intent.trace_root_identity.clone(),
            source_root: intent.source_root.clone(),
            submission_profile: intent.submission_profile,
            limits: intent.limits.clone(),
            limit_profile: intent.limit_profile.clone(),
            capture_limits: intent.capture_limits.clone(),
            excluded: intent.excluded.clone(),
            admission_watermarks: intent.admission_watermarks.clone(),
            accepted_handoff: intent.accepted_handoff.clone(),
        }
    }
}

fn validate_successor(
    predecessor: &Option<ChainPredecessorV1>,
    actual: &Link,
    kind: &str,
    entry_id: &str,
) -> Result<(), DaemonError> {
    let (seq, hash) = match predecessor {
        Some(ChainPredecessorV1::Position(predecessor)) => (
            predecessor.seq.checked_add(1).ok_or_else(|| {
                DaemonError::Refused(format!(
                    "workspace finalization intent for entry {entry_id} cannot append {kind} after u64::MAX"
                ))
            })?,
            predecessor.hash.as_str(),
        ),
        Some(ChainPredecessorV1::Genesis) => (0, GENESIS),
        None => {
            return Err(DaemonError::Refused(format!(
                "workspace finalization intent for entry {entry_id} has no prepared {kind} predecessor"
            )));
        }
    };
    if actual.seq != seq
        || actual.prev != hash
        || Sha256Digest::try_from(actual.hash.as_str()).is_err()
    {
        return Err(DaemonError::Refused(format!(
            "workspace finalization intent for entry {entry_id} has an invalid {kind} link"
        )));
    }
    Ok(())
}

impl EntryState {
    /// No capture, delivery, or reopening may cross a durable terminal boundary.  `Sealing`
    /// is deliberately included: the terminal chain link may already exist even when the
    /// registry promotion has not happened yet, so treating it as an ordinary open row
    /// would race evidence after its intended last link.
    pub const fn capture_closed(self) -> bool {
        matches!(
            self,
            Self::CleanupPending | Self::Sealing | Self::Sealed | Self::Refused
        )
    }
}

/// The immutable terminal payload staged before a seal can append its last chain link.
///
/// The registry owns this transient copy because a terminal link can reach `chain.jsonl`
/// before `seal.json`, `trace.toml`, the encrypted buffer, or the row's terminal state. A
/// restart repairs those stores only when the validated link matches every staged fact.
/// The manifest is already terminal evidence once recovery succeeds; retaining it privately
/// while `Sealing` is the smallest way to rebuild its missing fixed document without
/// guessing from a digest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LegacySealingIntent {
    pub manifest: SealManifest,
    pub descriptor: TraceDescriptorDoc,
    /// Opaque identity of the acquired trace-root capability. The display path is not an
    /// authority: a restart must refuse if that path now resolves to another directory.
    #[serde(default)]
    trace_root_identity: String,
    /// The validated head the live writer held before it was permitted to append the
    /// terminal link. It binds recovery to this history rather than to a merely
    /// self-consistent replacement chain at the path.
    #[serde(default)]
    pre_seal_seq: Option<u64>,
    #[serde(default)]
    pre_seal_head: String,
    /// The complete link identity the staged manifest may append, computed before the
    /// registry enters `Sealing`. Body equality alone is insufficient: another chain can
    /// carry the same body at a different predecessor.
    #[serde(default)]
    terminal_seq: u64,
    #[serde(default)]
    terminal_prev: String,
    #[serde(default)]
    terminal_hash: String,
}

impl LegacySealingIntent {
    /// Stage a seal from the live writer's already-validated head. The expected terminal
    /// link is computed here, before the registry state changes, so recovery never infers
    /// an authority-bearing predecessor from a path it opens after a crash.
    pub fn new(
        manifest: SealManifest,
        descriptor: TraceDescriptorDoc,
        pre_seal_head: Option<(u64, String)>,
        trace_root_identity: String,
    ) -> Result<Self, DaemonError> {
        let (pre_seal_seq, pre_seal_head) = match pre_seal_head {
            Some((seq, head)) => (Some(seq), head),
            None => (None, GENESIS.to_string()),
        };
        let terminal_seq = match pre_seal_seq {
            Some(seq) => seq.checked_add(1).ok_or_else(|| {
                DaemonError::Refused(
                    "a chain at u64::MAX cannot stage another terminal link".to_string(),
                )
            })?,
            None => 0,
        };
        let digest = manifest_sha256(&manifest)?;
        let mut envelope = Envelope {
            protocol_version: goalcup_protocol::PROTOCOL_VERSION,
            entry_id: manifest.entry_id.clone(),
            source: "daemon".to_string(),
            recorded_at: manifest.sealed_at.clone(),
            link: goalcup_protocol::chain::Link {
                seq: terminal_seq,
                prev: pre_seal_head.clone(),
                hash: String::new(),
            },
            body: EventBody::Seal {
                manifest_sha256: digest,
                sealed_at: manifest.sealed_at.clone(),
                file_count: manifest.files.len() as u64,
            },
            redaction: None,
        };
        envelope.link.hash = envelope.expected_hash(&pre_seal_head)?;
        Ok(Self {
            manifest,
            descriptor,
            trace_root_identity,
            pre_seal_seq,
            pre_seal_head,
            terminal_seq,
            terminal_prev: envelope.link.prev.clone(),
            terminal_hash: envelope.link.hash,
        })
    }

    pub(crate) fn validate(&self, entry_id: &str) -> Result<(), DaemonError> {
        if self.manifest.entry_id != entry_id || self.descriptor.entry_id != entry_id {
            return Err(DaemonError::Refused(format!(
                "sealing intent belongs to {} / {}, not entry {entry_id}",
                self.manifest.entry_id, self.descriptor.entry_id
            )));
        }
        if self.descriptor.workspace != self.manifest.workspace
            || self.descriptor.seal.as_deref() != Some(SEAL_FILE)
        {
            return Err(DaemonError::Refused(format!(
                "sealing intent for entry {entry_id} does not stage the exact sealed descriptor"
            )));
        }
        let expected_seq = self
            .pre_seal_seq
            .map(|seq| seq.checked_add(1))
            .unwrap_or(Some(0));
        if self.trace_root_identity.is_empty()
            || expected_seq.is_none()
            || self.pre_seal_head.is_empty()
            || self.terminal_seq != expected_seq.expect("checked above")
            || self.terminal_prev != self.pre_seal_head
        {
            return Err(DaemonError::Refused(format!(
                "sealing intent for entry {entry_id} has an invalid staged predecessor"
            )));
        }
        let expected = Self::new(
            self.manifest.clone(),
            self.descriptor.clone(),
            self.pre_seal_seq
                .map(|seq| (seq, self.pre_seal_head.clone())),
            self.trace_root_identity.clone(),
        )?;
        if self.terminal_hash != expected.terminal_hash {
            return Err(DaemonError::Refused(format!(
                "sealing intent for entry {entry_id} has an invalid staged terminal hash"
            )));
        }
        Ok(())
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn matches_terminal(&self, terminal: &Envelope) -> Result<(), DaemonError> {
        let EventBody::Seal {
            manifest_sha256: actual_digest,
            sealed_at,
            file_count,
        } = &terminal.body
        else {
            return Err(DaemonError::Refused(
                "sealing recovery was not given a seal terminal envelope".to_string(),
            ));
        };
        let expected_digest = manifest_sha256(&self.manifest)?;
        if terminal.entry_id != self.manifest.entry_id
            || terminal.recorded_at != self.manifest.sealed_at
            || actual_digest != &expected_digest
            || sealed_at != &self.manifest.sealed_at
            || *file_count != self.manifest.files.len() as u64
            || terminal.link.seq != self.terminal_seq
            || terminal.link.prev != self.terminal_prev
            || terminal.link.hash != self.terminal_hash
        {
            return Err(DaemonError::Refused(format!(
                "sealing terminal for entry {} does not match its staged manifest identity",
                self.manifest.entry_id
            )));
        }
        Ok(())
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn expected_terminal_seq(&self) -> u64 {
        self.terminal_seq
    }

    /// The highest already-proven chain position while this intent is unresolved.  Its
    /// planned terminal is not evidence until a retained writer or restart validator has
    /// read back the exact staged envelope, so a live `Sealing` runtime may upload only
    /// this predecessor.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn pre_seal_seq(&self) -> Option<u64> {
        self.pre_seal_seq
    }

    #[cfg(all(feature = "v1-compat", any(test, feature = "test-hooks")))]
    pub(crate) fn expected_terminal_envelope(&self) -> Result<Envelope, DaemonError> {
        Ok(Envelope {
            protocol_version: goalcup_protocol::PROTOCOL_VERSION,
            entry_id: self.manifest.entry_id.clone(),
            source: "daemon".to_string(),
            recorded_at: self.manifest.sealed_at.clone(),
            link: goalcup_protocol::chain::Link {
                seq: self.terminal_seq,
                prev: self.terminal_prev.clone(),
                hash: self.terminal_hash.clone(),
            },
            body: EventBody::Seal {
                manifest_sha256: manifest_sha256(&self.manifest)?,
                sealed_at: self.manifest.sealed_at.clone(),
                file_count: self.manifest.files.len() as u64,
            },
            redaction: None,
        })
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn matches_trace_root(&self, actual: &str) -> Result<(), DaemonError> {
        if actual != self.trace_root_identity {
            return Err(DaemonError::Refused(format!(
                "sealing trace root for entry {} is not the root that staged the intent",
                self.manifest.entry_id
            )));
        }
        Ok(())
    }

    /// A no-terminal recovery may reopen capture only when the currently validated trace
    /// still ends at the predecessor this intent staged.  A merely valid shorter or longer
    /// history at the same directory is not the writer history this daemon reserved a seal
    /// against, so it stays capture-closed for manual recovery.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn matches_pre_seal_head(
        &self,
        actual: Option<(u64, String)>,
    ) -> Result<(), DaemonError> {
        let expected = self
            .pre_seal_seq
            .map(|seq| (seq, self.pre_seal_head.clone()));
        if actual != expected {
            return Err(DaemonError::Refused(format!(
                "sealing trace for entry {} no longer ends at its staged predecessor",
                self.manifest.entry_id
            )));
        }
        Ok(())
    }
}

/// The one durable sealing authority stored on an entry.
///
/// The workspace arm is deliberately first and closed. The historical arm stays an
/// unwrapped object with exactly its old fields, so reading and rewriting a V1-V3 row does
/// not add a tag or otherwise reinterpret its recovery authority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum SealingIntent {
    Workspace(Box<WorkspaceSealingIntentV1>),
    Legacy(Box<LegacySealingIntent>),
}

impl SealingIntent {
    /// Preserve the legacy constructor and its serialized bytes for every existing caller.
    pub fn new(
        manifest: SealManifest,
        descriptor: TraceDescriptorDoc,
        pre_seal_head: Option<(u64, String)>,
        trace_root_identity: String,
    ) -> Result<Self, DaemonError> {
        LegacySealingIntent::new(manifest, descriptor, pre_seal_head, trace_root_identity)
            .map(Box::new)
            .map(Self::Legacy)
    }

    pub(crate) fn validate(&self, entry_id: &str) -> Result<(), DaemonError> {
        match self {
            Self::Workspace(intent) => intent.validate(entry_id),
            Self::Legacy(intent) => intent.validate(entry_id),
        }
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn legacy(&self) -> Result<&LegacySealingIntent, DaemonError> {
        match self {
            Self::Legacy(intent) => Ok(intent),
            Self::Workspace(_) => Err(DaemonError::Refused(
                "a workspace finalization intent cannot be reconciled as a legacy seal".to_string(),
            )),
        }
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn matches_terminal(&self, terminal: &Envelope) -> Result<(), DaemonError> {
        self.legacy()?.matches_terminal(terminal)
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn expected_terminal_seq(&self) -> Result<u64, DaemonError> {
        Ok(self.legacy()?.expected_terminal_seq())
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn pre_seal_seq(&self) -> Option<u64> {
        match self {
            Self::Legacy(intent) => intent.pre_seal_seq(),
            Self::Workspace(_) => None,
        }
    }

    #[cfg(all(feature = "v1-compat", any(test, feature = "test-hooks")))]
    pub(crate) fn expected_terminal_envelope(&self) -> Result<Envelope, DaemonError> {
        self.legacy()?.expected_terminal_envelope()
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn matches_trace_root(&self, actual: &str) -> Result<(), DaemonError> {
        self.legacy()?.matches_trace_root(actual)
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn matches_pre_seal_head(
        &self,
        actual: Option<(u64, String)>,
    ) -> Result<(), DaemonError> {
        self.legacy()?.matches_pre_seal_head(actual)
    }
}

/// Whether the daemon must receive the process the CLI launched before it can make this
/// entry `Started`.
///
/// This is persisted with the entry rather than inferred from a connector manifest: a
/// `tui` connector can be entered through `goalcup start --attach`, which launches no
/// child. Rows written before this field existed have no trustworthy provenance, so their
/// serde default is deliberately conservative at start time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedProcessHandoff {
    /// A pre-upgrade row. It may start only after a handle arrives; once already started,
    /// the legacy manifest-based health/seal behavior remains in force.
    #[default]
    LegacyUnknown,
    /// This launch path spawned a child and must hand its handle to the daemon first.
    Required,
    /// This run was attached to an existing harness, so no daemon-owned child exists.
    NotRequired,
}

/// The durable authority state of the process route.
///
/// `last_process` answers health and termination.  It is deliberately not the authority:
/// an old response can name a perfectly plausible process after a resume has cleared that
/// field.  The generation and phase below are the one state machine that decides whether a
/// registration belongs to this launch opening.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessHandoffPhase {
    /// Rows created before durable handoff authority existed.  They may never be opened by
    /// a process request; a `Started` legacy row gets one fresh `OpenResume` generation.
    #[default]
    LegacyUnknown,
    /// A new launch exists but has not been armed, or is attached and will never open an
    /// initial process authority.
    InitialClosed,
    /// The armed initial launch may prepare its process request for this generation.
    OpenInitial,
    /// The started resume has atomically cleared the predecessor and may prepare once.
    OpenResume,
    /// The request identity and safe seed record are durable; the event must be reconciled
    /// before this becomes `Registered`.
    Prepared,
    /// The one accepted process is the active health/termination target.
    Registered,
    /// A resume permit was issued but the CLI returned it before a replacement handoff was
    /// accepted.  The generation is deliberately retained: a later resume must open a new
    /// one, never reuse an observed permit.  `last_process` remains the previous, still
    /// authoritative seal target until a later `Registered` replacement commits.
    ResumeCancelled,
}

impl ProcessHandoffPhase {
    /// The authenticated read-only spelling used by resume preview.  Keep this beside the
    /// serde enum: a CLI may display/diagnose the authoritative phase but never interpret a
    /// cached spelling as authority.
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::LegacyUnknown => "legacy_unknown",
            Self::InitialClosed => "initial_closed",
            Self::OpenInitial => "open_initial",
            Self::OpenResume => "open_resume",
            Self::Prepared => "prepared",
            Self::Registered => "registered",
            Self::ResumeCancelled => "resume_cancelled",
        }
    }
}

/// The exact accepted child an unstarted entry must prove gone before it can be abandoned.
/// This is intentionally private-to-the-daemon lifecycle: arbitrary registry callers may
/// not turn a `Prepared` or `Registered` authority back into an ordinary removable row.
#[cfg(feature = "v1-compat")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnstartedTermination {
    state: EntryState,
    phase: ProcessHandoffPhase,
    generation: u64,
    digest: Option<String>,
    handle: ProcessHandle,
    other_handle: Option<ProcessHandle>,
}

#[cfg(feature = "v1-compat")]
impl UnstartedTermination {
    #[cfg(feature = "v1-compat")]
    pub(crate) fn handle(&self) -> &ProcessHandle {
        &self.handle
    }
}

/// The prepared child a resume cancellation must terminate before it may restore a standing
/// generation.  It never carries a seed: copied home evidence is not termination authority.
#[cfg(feature = "v1-compat")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedResumeTermination {
    handle: ProcessHandle,
}

#[cfg(feature = "v1-compat")]
impl PreparedResumeTermination {
    #[cfg(feature = "v1-compat")]
    pub(crate) fn handle(&self) -> &ProcessHandle {
        &self.handle
    }
}

const MAX_PREPARED_HOME_SEED_BYTES: usize = 64 * 1024;

/// The three per-run secrets.
///
/// They are minted once per entry and never leave this machine: the buffer key encrypts
/// the write-ahead store, the signing seed is the run key every `SignedStatus` and
/// `Chunk` is signed with, and the receiver token is the per-run bearer a rendered tracker
/// file may carry.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunKeys {
    /// 32 random bytes, hex: the age passphrase recipient.
    pub buffer_key_hex: String,
    /// 32 random bytes, hex: the Ed25519 seed, not the expanded key.
    pub signing_seed_hex: String,
    /// 16 random bytes, hex (32 characters). D24's per-run bearer.
    pub receiver_token: String,
}

/// Nothing but the shape. All three fields are secrets, and a derived `Debug` would put
/// them in every log line and every interpolated error that carries an `Entry` — the same
/// leak `control::WorkspaceInfo` hand-writes its `Debug` to close.
impl std::fmt::Debug for RunKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunKeys")
            .field("buffer_key_hex", &"<redacted>")
            .field("signing_seed_hex", &"<redacted>")
            .field("receiver_token", &"<redacted>")
            .finish()
    }
}

impl RunKeys {
    /// Three fresh secrets from the operating system's entropy source.
    ///
    /// `fill_bytes` rather than `try_fill_bytes` because the signature returns
    /// `RunKeys` and not a `Result`: on every platform this daemon targets, a failing
    /// `OsRng` is a machine that cannot keep a secret at all, and continuing with a
    /// predictable run key would be worse than the abort `rand` raises.
    pub fn generate() -> RunKeys {
        use rand::RngCore;
        let mut buffer = [0u8; 32];
        let mut signing = [0u8; 32];
        let mut token = [0u8; 16];
        let mut rng = rand::rngs::OsRng;
        rng.fill_bytes(&mut buffer);
        rng.fill_bytes(&mut signing);
        rng.fill_bytes(&mut token);
        RunKeys::from_seeds(buffer, signing, token)
    }

    /// The same keys from bytes a caller chose. Tests use it for reproducible runs; no
    /// production path does.
    pub fn from_seeds(buffer: [u8; 32], signing: [u8; 32], token: [u8; 16]) -> RunKeys {
        RunKeys {
            buffer_key_hex: hex::encode(buffer),
            signing_seed_hex: hex::encode(signing),
            receiver_token: hex::encode(token),
        }
    }

    pub fn signing_key(&self) -> Result<ed25519_dalek::SigningKey, DaemonError> {
        let seed: [u8; 32] = hex::decode(&self.signing_seed_hex)
            .map_err(|_| Self::bad_seed())?
            .try_into()
            .map_err(|_| Self::bad_seed())?;
        Ok(ed25519_dalek::SigningKey::from_bytes(&seed))
    }

    /// Hex verifying key. `WorkspaceInfo.run_public_key`, the key `SignedStatus::verify_with`
    /// and `Chunk::verify` are pinned against, and the key the receipt request carries.
    pub fn public_key_hex(&self) -> Result<String, DaemonError> {
        Ok(hex::encode(self.signing_key()?.verifying_key().to_bytes()))
    }

    /// The seed is a secret, so the message describes its shape and never shows it.
    fn bad_seed() -> DaemonError {
        DaemonError::Crypto(
            "the run signing seed is not 64 hex characters decoding to 32 bytes".to_string(),
        )
    }
}

/// What one seal left behind, kept on the entry so a restarted daemon can read it.
///
/// [`crate::runtime::SealReport`] is the live answer `Daemon::seal_entry` hands its caller;
/// this is the durable half. A daemon killed between `write_seal` and `terminate` leaves a
/// sealed entry with its harness still running and nothing in memory saying so, and the next
/// process has to be able to finish the job: it reaps the survivor and writes what that
/// cost here.
///
/// A **summary** of the termination rather than the report itself, because
/// `TerminationReport` is unix-only and this row is read on every platform. What a reader
/// needs from it is whether the tree is gone and what was signalled, and both are words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealOutcome {
    /// The connector's `lock_paths[]`, resolved through the entry's own path variables.
    /// Empty when the templates would not resolve — see `lock_paths_error`.
    pub lock_paths: Vec<String>,
    /// Why the paths above are empty, when they are. `None` on a clean resolution.
    pub lock_paths_error: Option<String>,
    /// The kill guard could ask the age question and not the identity one.
    pub guard_degraded: bool,
    /// The guard refused the registered pid, so nothing in the tree was signalled by number
    /// and neither this seal nor any later reader of this row may say the harness is gone.
    ///
    /// The durable half of [`crate::runtime::SealReport::harness_unverified`], and what
    /// `StatusPayload::harness_unverified` is built from. `#[serde(default)]` because rows
    /// written before this field existed are read by this build, and `false` is what they
    /// meant: a field they did not have claimed nothing.
    #[serde(default)]
    pub harness_unverified: bool,
    /// What the kill did, in one line. `None` on an entry that registered no process.
    pub termination: Option<String>,
    /// Why the harness was not terminated, when it was not.
    pub termination_error: Option<String>,
}

/// The strict bound on pending foreign identities retained in the entry registry.
///
/// Pending rows carry only the bounded identity projection below; they never retain
/// transcript bytes. The bound is shared by the registry validator and the runtime's
/// in-memory retention so a malformed or stale row cannot turn restart recovery into an
/// unbounded allocation.
pub const MAX_FOREIGN_PENDING_SIGHTINGS: usize = 256;
/// Durable pending identities are projections of bounded scan fields, not an alternate
/// transcript store. This is a retained-state bound applied after serde decodes the registry
/// row; it is deliberately not described as a pre-deserialization allocation guard.
const MAX_FOREIGN_PENDING_FIELD_BYTES: usize = 16 * 1024;

/// The crash-recovery projection of one deferred own-config foreign sighting.
///
/// This is registry state, not protocol/wire state. It deliberately contains identifiers,
/// the store path, first-seen timestamp and trusted scan-root provenance only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingForeignSighting {
    pub family: String,
    pub harness: String,
    pub session_id: String,
    pub store_path: String,
    pub first_seen: String,
    #[serde(default)]
    pub scan_root: ForeignScanRoot,
}

/// Everything the daemon knows about one run.
///
/// `chain_seq`/`chain_head` are a **mirror of the export's own head, never an input to
/// it**: `TraceExport::resume` takes the trace root and nothing else, and reads its
/// position off `chain.jsonl`. These two fields exist so a status, a `DaemonRestart`
/// body or a receipt claim can be produced without opening the export. `chain_seq` is an
/// `Option` for the same reason `TraceExport::head()` is: "nothing written" and "seq 0
/// written" are two states.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub entry_id: String,
    pub connector_id: String,
    pub workspace: PathBuf,
    pub artifact_dir: PathBuf,
    /// `artifact_dir.join("trace")` — the one directory `TraceExport` owns. On the
    /// entry rather than derived at each call site, because several call sites open it and a
    /// second derivation is a second answer.
    pub trace_dir: PathBuf,
    pub manifest_sha256: String,
    /// The digest of the whole connector directory this daemon rendered the run's tracker
    /// files out of, taken once at workspace creation and claimed on the start receipt.
    ///
    /// Not a second copy of `manifest_sha256`: that one covers `manifest.toml`, the file a
    /// trace carries; this one covers the hook bodies under `files/` beside it, which no
    /// trace carries and which are what the daemon renders, records as the expected config,
    /// and then guards. Held on the row rather than recomputed in `bind_start` so the
    /// receipt names the bytes the workspace was actually built from.
    ///
    /// `#[serde(default)]`, so a row written before this field existed still reads: the
    /// registry is a file that outlives an upgrade, and an entry a daemon cannot open is a
    /// run it cannot finish. Such a row claims nothing on its receipt, and V13 says so as
    /// `connector_dir_unbound`.
    #[serde(default)]
    pub connector_dir_sha256: Option<String>,
    pub harness_version: String,
    pub state: EntryState,
    pub health: HealthState,
    pub session_id: Option<String>,
    pub armed_at: Option<f64>,
    pub start_signal_at: Option<f64>,
    pub started_at: Option<String>,
    pub deadline_at: Option<String>,
    /// The signed start receipt.
    ///
    /// **Private on purpose.** The registry's checked receipt writers are the only code
    /// that can change it, and both verify against the key this build pins. There is no
    /// setter; [`Entry::new`] leaves it `None`; a struct literal outside this module does
    /// not compile; [`Registry::put`] carries the stored receipt forward rather than
    /// taking the caller's; and [`Registry::update`] restores it after the closure has
    /// run. So "a stored receipt is a verified receipt" is a property of the type, not a
    /// convention each caller has to remember. Read it with [`Entry::receipt`].
    receipt: Option<StartReceipt>,
    /// The receipt contract selected when this entry was created.
    ///
    /// Private and preserved by every generic replacement/update so ambient configuration
    /// cannot upgrade, downgrade, or retarget an existing run. Old rows are legacy.
    #[serde(default)]
    receipt_binding: ReceiptBindingPolicy,
    /// `None` until the first envelope is appended.
    pub chain_seq: Option<u64>,
    /// The genesis link until the first append.
    pub chain_head: String,
    pub last_ack_seq: Option<u64>,
    pub last_ack_at: Option<f64>,
    pub events_uploaded: u64,
    /// A capture-closed terminal buffer or trace failed its recovery validation.  The chain
    /// cannot carry a post-seal health event, so this durable quarantine is the authority
    /// the signed status reads until an operator resolves the evidence failure.
    #[serde(default)]
    pub terminal_drain_validation_failed: bool,
    pub expected_config: BTreeMap<String, String>,
    pub deadline_unix: Option<f64>,
    /// Exact UTC date tokens used when this entry's store templates were first rendered.
    /// Missing legacy rows may continue ordinary capture, but V4 finalization refuses
    /// rather than guessing a recovery pathname.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_date: Option<[String; 3]>,
    /// Forward-only observation that this daemon has reached the signed deadline. Private
    /// and omitted until true so legacy row bytes remain unchanged.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    deadline_observed: bool,
    /// Forward-only time authority for choosing a V4 cutoff. Persisting the high-water,
    /// rather than only the at-deadline bit, survives daemon restart plus wall rollback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    finalization_observed_unix: Option<u64>,
    pub sealed_at: Option<String>,
    /// The open lifecycle state a durable seal intent must restore if the daemon dies before
    /// `write_seal` reaches its terminal chain link.  It is never a caller-supplied state
    /// transition: only the dedicated sealing transactions below write it.
    #[serde(default)]
    pub sealing_from_state: Option<EntryState>,
    /// The exact manifest and descriptor staged with `Sealing`. Generic writers cannot
    /// replace or discard it; only the dedicated recovery transaction consumes it.
    #[serde(default)]
    sealing_intent: Option<SealingIntent>,
    /// The root capability identity that produced the terminal chain. A sealed drain
    /// rechecks it before it resumes through the display path, so a post-promotion root
    /// swap cannot make a later daemon upload another directory's bytes.
    #[serde(default)]
    sealed_trace_root_identity: Option<String>,
    pub status_line_path: PathBuf,
    /// The `HOME` (or `CODEX_HOME`) this run's harness is given, and `None` until the
    /// workspace setup has actually placed one.
    ///
    /// An `Option` rather than a derived default, because [`Registry::in_harness_home`]
    /// is the pre-arm scan: it asks "does another entry already live in this
    /// home?", and a guessed-but-plausible default would have every unplaced entry claim
    /// a directory nothing is in, so the scan would answer "nobody" confidently and
    /// wrongly. An entry with `None` occupies no home and matches no scan.
    pub harness_home: Option<PathBuf>,
    /// Same-harness own-config foreign identities observed before this entry bound a
    /// session. This is intentionally a bounded identity projection for crash recovery:
    /// no transcript bytes or scan directory are retained, and an empty vector is omitted
    /// so legacy rows keep their serialized shape.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub foreign_pending: Vec<PendingForeignSighting>,
    /// The port `create` reserved for a connector whose launch or resume recipe names
    /// `{server_port}`, and `None` for one that names it nowhere. The launch mode does not
    /// decide this: a `Tui` connector may pin the port of a server it embeds.
    ///
    /// Remembered on the row rather than only handed to the `WorkspaceInfo` the create
    /// answered with, because `Daemon::catch_up` has to give a relaunch the same port: a
    /// resume that answered `None` would render `--port ""`, which is a launch the CLI
    /// refuses.
    ///
    /// `#[serde(default)]`, so a row written before this field existed still reads. The
    /// registry is a file that outlives an upgrade, and an entry a daemon cannot open is a
    /// run it cannot finish.
    #[serde(default)]
    pub server_port: Option<u16>,
    pub arming_timeout_seconds: Option<u64>,
    /// The actual workspace-creation invocation determines this, not the connector's
    /// launch mode: `--attach` is possible on a normal `tui` connector.
    #[serde(default)]
    expected_process_handoff: ExpectedProcessHandoff,
    /// The daemon-issued, monotonically increasing process authority.  Its default is zero
    /// solely so a historical started row can mint generation one during a guarded resume.
    #[serde(default)]
    process_handoff_generation: u64,
    /// Missing on an old row is deliberately `LegacyUnknown`, never an implicit open slot.
    #[serde(default)]
    process_handoff_phase: ProcessHandoffPhase,
    /// Canonical, domain-separated digest of entry+generation+termination identity.  It
    /// excludes home-seed evidence and is private registry state, not a receipt claim.
    #[serde(default)]
    process_handoff_digest: Option<String>,
    /// The termination identity awaiting/holding registration.  It is always stripped of
    /// `home_seed`; copied account settings never live on the ordinary process handle.
    #[serde(default)]
    prepared_process_handle: Option<ProcessHandle>,
    /// The bounded, already-sanitized seed record only a crash replay needs to append its
    /// one event.  It is private registry state and has no auth-file bytes or paths.
    #[serde(default)]
    prepared_home_seed: Option<HomeSeedRecord>,
    /// The exact Prepared identity an idempotent cancellation returned.  Unlike the live
    /// handoff digest this is not termination authority; it only prevents a lost cancel
    /// response from turning a later caller's different digest into a success.
    #[serde(default)]
    cancelled_process_handoff_digest: Option<String>,
    /// A verified unstarted-handoff termination committed before artifact removal. This
    /// closes the crash window where the process is gone but a restarted daemon would
    /// otherwise attempt to verify the now-absent PID again.
    #[serde(default)]
    cleanup_termination_complete: bool,
    pub last_process: Option<ProcessHandle>,
    /// What the seal left behind, once there has been one.
    ///
    /// `#[serde(default)]` so every row written before this field existed still reads: the
    /// registry is a file that outlives an upgrade, and an entry a daemon cannot open is a
    /// run it cannot finish.
    #[serde(default)]
    pub seal_outcome: Option<SealOutcome>,
    /// The round this run was told about, resolved once at creation from `[event]`.
    ///
    /// Held on the row because the terminal composers have no config and no runtime: the
    /// runtime that knew the round is evicted at seal, and `Daemon::start`'s republication
    /// sweep runs before any runtime exists. Without this a sealed document silently reverts
    /// to the entry's own timeline and is re-signed that way on every restart.
    ///
    /// `#[serde(default)]` so a row written before this field existed still reads: the
    /// registry is a file that outlives an upgrade, and an entry a daemon cannot open is a
    /// run it cannot finish. `None` reproduces the fallback exactly.
    ///
    /// Only the three configurable fields are honoured from here. `publication_closes_at` is
    /// always recomputed, because the deadline is fixed by the receipt after this is written
    /// — see [`crate::status::event_status_from`].
    #[serde(default)]
    pub event: Option<EventStatus>,
    /// `Some` for a protocol-v5 local capture entry, which is never armed, tailed, uploaded
    /// or rebuilt as a legacy runtime. Absent from every legacy row byte-for-byte.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<CaptureRowV1>,
    pub created_at: String,
    pub keys: RunKeys,
}

/// What a v5 local capture entry keeps on its row beyond the legacy fields: the profile is
/// `connector_id`, its digest `manifest_sha256`, and the chain is under `trace_dir`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureRowV1 {
    /// The serialized chain bytes reserved at creation, which a reopened chain reserves
    /// again (a remeasurement, not a second allocation).
    pub reserve_bytes: u64,
    /// The committed local finish, which a retried seal answers after a restart too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished: Option<goalcup_protocol::control::CaptureFinishReport>,
    /// The capture status a finish that wrote its seal left. Such an entry's chain is closed:
    /// its status is answered from here, and a restart does not reopen it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed_status: Option<goalcup_protocol::capture::M2Status>,
}

pub(crate) fn validate_foreign_pending(entry: &Entry) -> Result<(), DaemonError> {
    if entry.foreign_pending.len() > MAX_FOREIGN_PENDING_SIGHTINGS {
        return Err(DaemonError::Refused(format!(
            "entry {} retains {} pending foreign identities, above the {}-identity bound",
            entry.entry_id,
            entry.foreign_pending.len(),
            MAX_FOREIGN_PENDING_SIGHTINGS
        )));
    }
    let mut identities = std::collections::BTreeSet::new();
    for pending in &entry.foreign_pending {
        if pending.scan_root != ForeignScanRoot::OwnConfigDir {
            return Err(DaemonError::Refused(format!(
                "entry {} retains pending foreign identity without own-config provenance",
                entry.entry_id
            )));
        }
        if pending.harness != entry.connector_id {
            return Err(DaemonError::Refused(format!(
                "entry {} retains pending foreign identity for harness {}, not its trusted connector {}",
                entry.entry_id, pending.harness, entry.connector_id
            )));
        }
        for (field, value) in [
            ("family", pending.family.as_str()),
            ("harness", pending.harness.as_str()),
            ("session_id", pending.session_id.as_str()),
            ("store_path", pending.store_path.as_str()),
            ("first_seen", pending.first_seen.as_str()),
        ] {
            if value.is_empty()
                || value.len() > MAX_FOREIGN_PENDING_FIELD_BYTES
                || value.bytes().any(|byte| byte.is_ascii_control())
            {
                return Err(DaemonError::Refused(format!(
                    "entry {} has an invalid pending foreign {field} field",
                    entry.entry_id
                )));
            }
        }
        if !Path::new(&pending.store_path).is_absolute() {
            return Err(DaemonError::Refused(format!(
                "entry {} has a relative pending foreign store path",
                entry.entry_id
            )));
        }
        if Rfc3339Utc::try_from(pending.first_seen.clone()).is_err() {
            return Err(DaemonError::Refused(format!(
                "entry {} has an invalid pending foreign first-seen timestamp",
                entry.entry_id
            )));
        }
        let identity = (
            pending.family.clone(),
            pending.harness.clone(),
            pending.session_id.clone(),
        );
        if !identities.insert(identity) {
            return Err(DaemonError::Refused(format!(
                "entry {} contains duplicate pending foreign identity",
                entry.entry_id
            )));
        }
    }
    Ok(())
}

/// The non-receipt path identity of the runtime that requested a start receipt.
///
/// Captured from the runtime's [`Entry`] before the outbound request, then compared with
/// the row re-read inside [`Registry::commit_start`]. These paths do not belong in the
/// signed receipt, but a receipt built from one trace must never be installed after the
/// same entry id is replaced with another artifact or trace directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartPathIdentity {
    artifact_dir: PathBuf,
    trace_dir: PathBuf,
}

impl StartPathIdentity {
    pub fn from_entry(entry: &Entry) -> StartPathIdentity {
        StartPathIdentity {
            artifact_dir: entry.artifact_dir.clone(),
            trace_dir: entry.trace_dir.clone(),
        }
    }
}

impl Entry {
    /// A created entry: bound to a workspace and a connector, holding its keys, and
    /// carrying nothing that only a later stage can know.
    ///
    /// `health` starts [`HealthState::Online`] — the daemon is up and has observed
    /// nothing yet. Arming is what turns it into `ArmingPending`, and arming has not
    /// happened: `state` is [`EntryState::Created`].
    ///
    /// `status_line_path` defaults inside `artifact_dir`; workspace setup owns its final value
    /// and overwrites it. `harness_home` gets no such
    /// default — see the field — and `receipt` cannot be given one at all: it is `None`
    /// here and only the registry's checked receipt writers can change that.
    // The persisted receipt policy is intentionally explicit at every birth site; hiding
    // it in a default would let a new production constructor silently create legacy rows.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        entry_id: &str,
        connector_id: &str,
        workspace: PathBuf,
        artifact_dir: PathBuf,
        manifest_sha256: &str,
        harness_version: &str,
        receipt_binding: ReceiptBindingPolicy,
        expected_process_handoff: ExpectedProcessHandoff,
        keys: RunKeys,
    ) -> Entry {
        Entry {
            entry_id: entry_id.to_string(),
            connector_id: connector_id.to_string(),
            workspace,
            trace_dir: artifact_dir.join("trace"),
            status_line_path: artifact_dir.join("status-line.txt"),
            harness_home: None,
            artifact_dir,
            manifest_sha256: manifest_sha256.to_string(),
            // Not a constructor argument: the directory digest is taken by the control
            // handler that also renders the tracker files out of that directory, and a
            // second place to pass it is a second place to pass the wrong one.
            connector_dir_sha256: None,
            harness_version: harness_version.to_string(),
            state: EntryState::Created,
            health: HealthState::Online,
            session_id: None,
            armed_at: None,
            start_signal_at: None,
            started_at: None,
            deadline_at: None,
            receipt: None,
            receipt_binding,
            chain_seq: None,
            chain_head: GENESIS.to_string(),
            last_ack_seq: None,
            last_ack_at: None,
            events_uploaded: 0,
            terminal_drain_validation_failed: false,
            expected_config: BTreeMap::new(),
            deadline_unix: None,
            source_date: None,
            deadline_observed: false,
            finalization_observed_unix: None,
            sealed_at: None,
            sealing_from_state: None,
            sealing_intent: None,
            sealed_trace_root_identity: None,
            server_port: None,
            foreign_pending: Vec::new(),
            arming_timeout_seconds: None,
            expected_process_handoff,
            process_handoff_generation: 0,
            process_handoff_phase: ProcessHandoffPhase::InitialClosed,
            process_handoff_digest: None,
            prepared_process_handle: None,
            prepared_home_seed: None,
            cancelled_process_handoff_digest: None,
            cleanup_termination_complete: false,
            last_process: None,
            seal_outcome: None,
            event: None,
            capture: None,
            created_at: now_rfc3339(),
            keys,
        }
    }

    /// The verified start receipt, or `None` while the run has none.
    ///
    /// Read-only by design: there is no matching setter, because a receipt this returns
    /// was checked by the registry against the pinned server key.
    pub fn receipt(&self) -> Option<&StartReceipt> {
        self.receipt.as_ref()
    }

    /// The immutable receipt policy captured for this entry.
    pub fn receipt_binding(&self) -> &ReceiptBindingPolicy {
        &self.receipt_binding
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn bind_source_date(&mut self, date: [String; 3]) -> Result<(), DaemonError> {
        if !valid_source_date(&date) {
            return Err(DaemonError::Refused(format!(
                "entry {} has invalid source date tokens",
                self.entry_id
            )));
        }
        match &self.source_date {
            Some(existing) if existing != &date => Err(DaemonError::Refused(format!(
                "entry {} source date is already bound",
                self.entry_id
            ))),
            Some(_) => Ok(()),
            None => {
                self.source_date = Some(date);
                Ok(())
            }
        }
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn source_date(&self) -> Option<&[String; 3]> {
        self.source_date.as_ref()
    }

    /// The handoff expectation fixed when the workspace was created (or the conservative
    /// default read from a pre-upgrade row).
    pub fn expected_process_handoff(&self) -> ExpectedProcessHandoff {
        self.expected_process_handoff
    }

    /// The current server-issued handoff generation.  This is sent to the CLI only through
    /// the create/resume replies; it is not receipt material.
    pub fn process_handoff_generation(&self) -> u64 {
        self.process_handoff_generation
    }

    /// The exact durable seal plan. It is exposed read-only solely to the sealing
    /// reconciler; broad registry writes cannot fabricate or discard it.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn sealing_intent(&self) -> Option<&SealingIntent> {
        self.sealing_intent.as_ref()
    }

    pub(crate) fn workspace_sealing_intent(&self) -> Option<&WorkspaceSealingIntentV1> {
        match self.sealing_intent.as_ref() {
            Some(SealingIntent::Workspace(intent)) => Some(intent),
            Some(SealingIntent::Legacy(_)) | None => None,
        }
    }

    /// A signed status sees only the read-only projection of the durable workspace
    /// finalization authority. Legacy rows omit the additive status field entirely.
    pub(crate) fn workspace_finalization_status(&self) -> Option<WorkspaceFinalizationStatusV1> {
        self.workspace_sealing_intent()
            .map(WorkspaceSealingIntentV1::status_projection)
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) const fn deadline_was_observed(&self) -> bool {
        self.deadline_observed
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn observe_finalization_time(&mut self, candidate: f64) -> Result<u64, DaemonError> {
        let deadline = self.deadline_unix.ok_or_else(|| {
            DaemonError::Refused(format!(
                "entry {} has no signed deadline to observe",
                self.entry_id
            ))
        })?;
        let candidate = candidate.max(0.0) as u64;
        let observed = self.finalization_observed_unix.unwrap_or(0).max(candidate);
        self.finalization_observed_unix = Some(observed);
        if observed as f64 >= deadline {
            self.deadline_observed = true;
        }
        Ok(observed)
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn matches_sealed_trace_root(&self, actual: &str) -> Result<(), DaemonError> {
        let expected = self.sealed_trace_root_identity.as_deref().ok_or_else(|| {
            DaemonError::Refused(format!(
                "sealed entry {} has no persisted trace-root identity",
                self.entry_id
            ))
        })?;
        if expected != actual {
            return Err(DaemonError::Refused(format!(
                "sealed entry {} trace root no longer matches the terminal writer root",
                self.entry_id
            )));
        }
        Ok(())
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn has_legacy_sealed_trace_root_identity(&self) -> bool {
        self.state == EntryState::Sealed && self.sealed_trace_root_identity.is_none()
    }

    /// A capture-closed terminal record must never be rendered as a clean completed upload
    /// when either recovery persisted a buffer/trace validation failure or its own durable
    /// acknowledgement watermark exceeds the immutable terminal head.  The latter is an
    /// explicit corrupt-row outcome already present on the entry, so it needs no second
    /// write merely for the status builder to tell the truth.
    pub(crate) fn terminal_drain_needs_review(&self) -> bool {
        self.terminal_drain_validation_failed
            || self.last_ack_seq.is_some_and(|acked| match self.chain_seq {
                Some(terminal) => acked > terminal,
                None => true,
            })
    }

    /// The persisted handoff phase, exposed for lifecycle diagnostics and direct registry
    /// tests but changed only by the dedicated authority transactions below.
    pub fn process_handoff_phase(&self) -> ProcessHandoffPhase {
        self.process_handoff_phase
    }

    /// The only persisted process identity a restarted seal may act on. `last_process` is
    /// public for diagnostics, but a reaper must not turn a caller-controlled-looking copy
    /// into signal authority without the matching registered generation and canonical
    /// digest that the handoff transaction wrote together.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn verified_registered_termination_handle(
        &self,
    ) -> Result<Option<ProcessHandle>, DaemonError> {
        Ok(self
            .verified_registered_termination_registration()?
            .map(|authority| authority.registration.handle))
    }

    /// The complete generation/digest/handle authority a finalization intent must latch
    /// before it prepares a termination attempt. This is the same validation as the
    /// legacy handle accessor, but it does not throw away the generation boundary.
    pub(crate) fn verified_registered_termination_registration(
        &self,
    ) -> Result<Option<AcceptedProcessHandoffV1>, DaemonError> {
        if self.process_handoff_phase != ProcessHandoffPhase::Registered {
            return Ok(None);
        }
        let handle = self.last_process.clone().ok_or_else(|| {
            DaemonError::Refused(format!(
                "entry {} is registered without a persisted termination handle",
                self.entry_id
            ))
        })?;
        let registration = ProcessRegistration {
            generation: self.process_handoff_generation,
            handle: handle.clone(),
        };
        let expected = registration.canonical_handle_digest(&self.entry_id);
        if self.process_handoff_digest.as_deref() != Some(expected.as_str()) {
            return Err(DaemonError::Refused(format!(
                "entry {} has a registered process without its matching canonical authority digest",
                self.entry_id
            )));
        }
        let request_digest = Sha256Digest::try_from(expected).map_err(|error| {
            DaemonError::Refused(format!(
                "entry {} has a non-canonical process authority digest: {error}",
                self.entry_id
            ))
        })?;
        Ok(Some(AcceptedProcessHandoffV1 {
            request_digest,
            registration,
        }))
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn is_registered_without_termination_handle(&self) -> bool {
        self.process_handoff_phase == ProcessHandoffPhase::Registered && self.last_process.is_none()
    }

    /// A start may proceed only after any required or unknown launch has supplied its
    /// durable process handle. `LegacyUnknown` fails closed because its absent field says
    /// nothing about whether the old invocation used `--attach`.
    pub fn start_handoff_complete(&self) -> bool {
        matches!(
            self.expected_process_handoff,
            ExpectedProcessHandoff::NotRequired
        ) || matches!(self.process_handoff_phase, ProcessHandoffPhase::Registered)
    }

    /// The process route may receive a handle only from a launch path that owes one.
    pub fn accepts_process_handoff(&self) -> bool {
        !matches!(
            self.expected_process_handoff,
            ExpectedProcessHandoff::NotRequired
        )
    }
}

fn handoff_conflict(entry_id: &str, why: &str) -> DaemonError {
    DaemonError::Guard(format!("entry {entry_id} process handoff conflict: {why}"))
}

/// Accept only the already-public, bounded `HomeSeedRecord` shape for crash recovery.  This
/// is defense in depth against a direct loopback caller: the actual CLI builds this record
/// through `SeededKey::new`, but a registry row must not become a second place to retain an
/// arbitrary copied auth file or an oversized secret-shaped value.
fn safe_seed_for_recovery(
    seed: Option<HomeSeedRecord>,
) -> Result<Option<HomeSeedRecord>, DaemonError> {
    let Some(mut seed) = seed else {
        return Ok(None);
    };
    let bytes = serde_json::to_vec(&seed)?;
    if bytes.len() > MAX_PREPARED_HOME_SEED_BYTES {
        return Err(DaemonError::Refused(format!(
            "home-seed recovery record is {} bytes, above the {MAX_PREPARED_HOME_SEED_BYTES}-byte bound",
            bytes.len()
        )));
    }
    let key_is_safe = |key: &SeededKey| {
        key.value
            .as_ref()
            .map(|value| SeededKey::new(&key.name, value).value.as_ref() == Some(value))
            .unwrap_or(true)
    };
    if !seed.keys.iter().all(key_is_safe) || !seed.written.iter().all(key_is_safe) {
        return Err(DaemonError::Refused(
            "home-seed recovery record carries an unsafe copied value shape".to_string(),
        ));
    }
    if !safe_manifest_home_spelling(&seed.from) {
        return Err(DaemonError::Refused(
            "home-seed recovery record names a resolved or unsafe source path".to_string(),
        ));
    }
    let safe_file_name = |name: &str| {
        !name.is_empty()
            && !name.starts_with('/')
            && !name.split('/').any(|segment| segment == "..")
            && !name.contains('{')
            && !name.contains('}')
    };
    if !seed.files.iter().all(|file| safe_file_name(&file.name))
        || !seed.files_kept.iter().all(|file| safe_file_name(file))
        || !seed.files_missing.iter().all(|file| safe_file_name(file))
    {
        return Err(DaemonError::Refused(
            "home-seed recovery record names an unsafe copied file path".to_string(),
        ));
    }
    // Only the daemon writes correlation markers.  A caller-provided marker could make an
    // untrusted body impersonate a chain event after a crash.
    seed.handoff_generation = None;
    seed.handoff_request_digest = None;
    Ok(Some(seed))
}

/// `HomeSeedRecord::from` is public evidence, so it has the connector manifest's stable
/// `{home}/…` spelling rather than the participant's resolved account path.  The manifest
/// validates the same shape before the CLI can construct a normal record; repeat the
/// narrow check at the durable direct-caller boundary, where the registry has no manifest
/// to consult.
fn safe_manifest_home_spelling(path: &str) -> bool {
    path.starts_with("{home}/")
        && !path["{home}/".len()..].is_empty()
        && !path.split('/').any(|segment| segment == "..")
        && !path["{home}/".len()..].contains('{')
        && !path["{home}/".len()..].contains('}')
}

/// The entries, on disk, in their own redb database.
pub struct Registry {
    db: Database,
    /// Integration tests need one deterministic representation of the storage failure that
    /// makes terminal review persistence impossible. It is strictly process-local and never
    /// compiled into the shipped registry.
    #[cfg(any(test, feature = "test-hooks"))]
    fail_next_terminal_drain_validation_persistence: AtomicBool,
}

impl Registry {
    /// Where the registry belongs: `<artifact_root>/registry/registry.redb`.
    ///
    /// A directory of its own, and not the shared `config_dir` the CLI and the MCP server
    /// both read, because [`Registry::open`] tightens the directory it owns to `0o700`
    /// and a directory shared with other readers is not the registry's to close.
    pub fn default_path(artifact_root: &Path) -> PathBuf {
        artifact_root.join("registry").join("registry.redb")
    }

    /// Open the production registry through the daemon's retained artifact-root
    /// capability. The configured path remains diagnostic only after root acquisition.
    pub fn open_artifact(artifact_root: &ArtifactRoot) -> Result<Registry, DaemonError> {
        let path = Self::default_path(artifact_root.display_path());
        let file = artifact_root.open_registry_file().map_err(|error| {
            DaemonError::Refused(format!(
                "registry parent or leaf cannot be acquired through retained artifact authority: {error}"
            ))
        })?;
        Self::from_file(&path, file)
    }

    /// Open the registry at `path`, creating it and its directory if either is missing.
    ///
    /// On unix the file is `0o600`, set on creation and **re-applied on every open**: a
    /// registry that was created loose, or loosened afterwards, is tightened rather than
    /// trusted. The parent directory is set to `0o700` only when it is the registry's
    /// own — one this call created, or one holding nothing but the registry file (which
    /// is what [`Registry::default_path`] gives it). A directory that holds anybody
    /// else's files is left exactly as it was found; the daemon is not entitled to close
    /// a directory it shares. Both tables are created here, so no later read transaction
    /// has to distinguish "empty" from "never written".
    pub fn open(path: &Path) -> Result<Registry, DaemonError> {
        let file_name = path.file_name().ok_or_else(|| {
            DaemonError::Refused(format!(
                "the registry path {} does not name a file",
                path.display()
            ))
        })?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            let ours = if parent.is_dir() {
                holds_only(parent, file_name)?
            } else {
                std::fs::create_dir_all(parent).map_err(|source| DaemonError::Io {
                    path: parent.display().to_string(),
                    source,
                })?;
                true
            };
            if ours {
                restrict(parent, 0o700)?;
            }
        }

        let mut opts = std::fs::OpenOptions::new();
        opts.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let file = opts.open(path).map_err(|source| DaemonError::Io {
            path: path.display().to_string(),
            source,
        })?;
        // `mode` above only applies to a file this call created; an existing one keeps
        // whatever it had, so repair it explicitly.
        restrict(path, 0o600)?;

        Self::from_file(path, file)
    }

    fn from_file(path: &Path, file: std::fs::File) -> Result<Registry, DaemonError> {
        // The one place D4 is enforced rather than asserted: redb takes a file lock, so a
        // second daemon on this machine fails here, by name, instead of racing the first
        // one over the entry map and the sweep below.
        let db = Database::builder().create_file(file).map_err(|e| match e {
            redb::DatabaseError::DatabaseAlreadyOpen => DaemonError::Refused(format!(
                "another daemon holds the registry at {}: one daemon per machine (D4), \
                 because entries are routed by working directory and two routers over one \
                 set of workspaces make every hook post ambiguous",
                path.display()
            )),
            other => db_err(path, "opening the registry", other),
        })?;
        {
            let txn = db
                .begin_write()
                .map_err(|e| db_err(path, "opening the registry", e))?;
            txn.open_table(ENTRIES)
                .map_err(|e| db_err(path, "creating the entries table", e))?;
            txn.open_table(META)
                .map_err(|e| db_err(path, "creating the meta table", e))?;
            txn.commit()
                .map_err(|e| db_err(path, "creating the registry tables", e))?;
        }
        Ok(Registry {
            db,
            #[cfg(any(test, feature = "test-hooks"))]
            fail_next_terminal_drain_validation_persistence: AtomicBool::new(false),
        })
    }

    /// Write an entry, replacing any entry with the same id.
    ///
    /// Two things the caller does not get to decide, both checked inside the write
    /// transaction so a concurrent put cannot slip between them:
    ///
    /// - **One entry per workspace (D4).** Entries are routed by working directory, so a
    ///   second entry bound to a directory another entry already holds would make the
    ///   route a coin toss. It is refused, and nothing is written.
    /// - **The receipt is not the caller's.** Whatever the value handed in says, the put
    ///   carries the *stored* receipt forward: a new entry starts with none and an
    ///   existing one keeps the one the registry verified.
    pub fn put(&self, entry: &Entry) -> Result<(), DaemonError> {
        validate_foreign_pending(entry)?;
        let txn = self.db.begin_write().map_err(|e| plain(e, "put"))?;
        {
            let mut table = txn.open_table(ENTRIES).map_err(|e| plain(e, "put"))?;
            let mut to_write = entry.clone();
            to_write.receipt = None;
            for row in table.iter().map_err(|e| plain(e, "put"))? {
                let (id, bytes) = row.map_err(|e| plain(e, "put"))?;
                let stored: Entry = serde_json::from_slice(bytes.value())?;
                if id.value() == entry.entry_id {
                    to_write.receipt = stored.receipt;
                    to_write.receipt_binding = stored.receipt_binding;
                    to_write.expected_process_handoff = stored.expected_process_handoff;
                    to_write.process_handoff_generation = stored.process_handoff_generation;
                    to_write.process_handoff_phase = stored.process_handoff_phase;
                    to_write.process_handoff_digest = stored.process_handoff_digest;
                    to_write.prepared_process_handle = stored.prepared_process_handle;
                    to_write.prepared_home_seed = stored.prepared_home_seed;
                    to_write.cancelled_process_handoff_digest =
                        stored.cancelled_process_handoff_digest;
                    to_write.sealing_from_state = stored.sealing_from_state;
                    to_write.sealing_intent = stored.sealing_intent;
                    to_write.sealed_trace_root_identity = stored.sealed_trace_root_identity;
                    // Terminal-drain validation is capture-closed evidence. A broad row
                    // replacement cannot erase an elevated-review outcome; only the
                    // dedicated monotonic validation transaction may set it.
                    to_write.terminal_drain_validation_failed =
                        stored.terminal_drain_validation_failed;
                    to_write.deadline_observed = stored.deadline_observed;
                    to_write.finalization_observed_unix = stored.finalization_observed_unix;
                    to_write.source_date = stored.source_date;
                    to_write.foreign_pending = stored.foreign_pending;
                    // `last_process` is the public, termination-facing half of the
                    // accepted handoff.  A broad `put` must not replace it independently
                    // of the private authority fields above.
                    to_write.last_process = stored.last_process;
                } else if stored.workspace == entry.workspace {
                    return Err(DaemonError::Refused(format!(
                        "workspace {} is already bound to entry {}, and one workspace \
                         holds one entry",
                        entry.workspace.display(),
                        stored.entry_id
                    )));
                }
            }
            let bytes = serde_json::to_vec(&to_write)?;
            table
                .insert(entry.entry_id.as_str(), bytes.as_slice())
                .map_err(|e| plain(e, "put"))?;
        }
        txn.commit().map_err(|e| plain(e, "put"))
    }

    /// Plant exact JSON bytes for integration coverage of upgrade compatibility.
    ///
    /// This bypasses all ordinary lifecycle guarantees and is available only to this
    /// crate's unit tests and integration tests compiled with the off-by-default
    /// `test-hooks` feature. Production callers must use [`Registry::put`] or one of the
    /// checked authority transitions above.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn replace_raw_json_for_test(
        &self,
        entry_id: &str,
        bytes: &[u8],
    ) -> Result<(), DaemonError> {
        let txn = self
            .db
            .begin_write()
            .map_err(|e| plain(e, "replace_raw_json_for_test"))?;
        {
            let mut table = txn
                .open_table(ENTRIES)
                .map_err(|e| plain(e, "replace_raw_json_for_test"))?;
            table
                .insert(entry_id, bytes)
                .map_err(|e| plain(e, "replace_raw_json_for_test"))?;
        }
        txn.commit()
            .map_err(|e| plain(e, "replace_raw_json_for_test"))
    }

    /// Read the exact stored bytes after [`Registry::replace_raw_json_for_test`].
    ///
    /// This is deliberately feature-gated with the writer above: normal callers must only
    /// observe decoded entries, while an upgrade fixture needs to prove absent historical
    /// keys remained absent on disk rather than being restored by serde defaults.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn raw_json_for_test(&self, entry_id: &str) -> Result<Option<Vec<u8>>, DaemonError> {
        let txn = self
            .db
            .begin_read()
            .map_err(|e| plain(e, "raw_json_for_test"))?;
        let table = txn
            .open_table(ENTRIES)
            .map_err(|e| plain(e, "raw_json_for_test"))?;
        let found = table
            .get(entry_id)
            .map_err(|e| plain(e, "raw_json_for_test"))?;
        Ok(found.map(|bytes| bytes.value().to_vec()))
    }

    pub fn get(&self, entry_id: &str) -> Result<Option<Entry>, DaemonError> {
        let txn = self.db.begin_read().map_err(|e| plain(e, "get"))?;
        let table = txn.open_table(ENTRIES).map_err(|e| plain(e, "get"))?;
        let found = table.get(entry_id).map_err(|e| plain(e, "get"))?;
        match found {
            Some(bytes) => {
                let entry: Entry = serde_json::from_slice(bytes.value())?;
                validate_foreign_pending(&entry)?;
                Ok(Some(entry))
            }
            None => Ok(None),
        }
    }

    /// Every entry, in `entry_id` order.
    ///
    /// Bounded by the number of concurrent runs on one machine — a handful — not by the
    /// size of any capture. No caller may use it to walk captured bytes.
    pub fn all(&self) -> Result<Vec<Entry>, DaemonError> {
        self.matching(|_| true)
    }

    /// Remove an entry, refusing inside the same write transaction if the row has a
    /// `started_at`. Removing an id that is not there is not an error: the contract is about the
    /// end state, and a refusal that unwinds twice must not turn into a second failure.
    ///
    /// **The check has to be in the transaction.** Reading `started_at` off the runtime's
    /// in-memory mirror and removing the row later would race `bind_start`, which commits a
    /// start from a receipt round trip that holds the runtime out of its slot for up to
    /// `NETWORK_TIMEOUT`: a start committed between the read and the removal would take the
    /// row with it, and the control API's file removal that follows `remove_dir_all`s the
    /// artifact directory: `chain.jsonl`, `baseline.json` and `receipt.json` of a run with a
    /// signed server receipt. redb serialises write transactions, so a check on the row the
    /// same transaction removes is a check the winner of that race cannot pass twice.
    ///
    /// The caller removes files only after this returns `Ok` — the row is the tombstone,
    /// and evidence outlives a directory entry.
    pub fn remove_if_unstarted(&self, entry_id: &str) -> Result<(), DaemonError> {
        let txn = self
            .db
            .begin_write()
            .map_err(|e| plain(e, "remove_if_unstarted"))?;
        {
            let mut table = txn
                .open_table(ENTRIES)
                .map_err(|e| plain(e, "remove_if_unstarted"))?;
            // Scoped so the read borrow ends before the removal takes a mutable one.
            let protected = {
                let found = table
                    .get(entry_id)
                    .map_err(|e| plain(e, "remove_if_unstarted"))?;
                // A row that is not there is already gone, which is what the caller wanted.
                match found {
                    Some(bytes) => {
                        let row = serde_json::from_slice::<Entry>(bytes.value())?;
                        Some((row.state, row.started_at, row.process_handoff_phase))
                    }
                    None => None,
                }
            };
            if let Some((state, started_at, phase)) = protected {
                if state.capture_closed() {
                    return Err(DaemonError::Refused(format!(
                        "entry {entry_id} is capture-closed and cannot be removed through the \
                         ordinary unstarted path: its registry row is durable lifecycle state"
                    )));
                }
                if let Some(at) = started_at {
                    return Err(DaemonError::Refused(format!(
                        "entry {entry_id} started at {at} and cannot be abandoned: a started \
                         run has a signed receipt and a witnessed deadline, and it ends by \
                         sealing"
                    )));
                }
                if matches!(
                    phase,
                    ProcessHandoffPhase::Prepared | ProcessHandoffPhase::Registered
                ) {
                    return Err(DaemonError::Refused(format!(
                        "entry {entry_id} has accepted an unstarted process handoff and cannot be abandoned \
                         without a verified termination path"
                    )));
                }
            }
            table
                .remove(entry_id)
                .map_err(|e| plain(e, "remove_if_unstarted"))?;
        }
        txn.commit().map_err(|e| plain(e, "remove_if_unstarted"))
    }

    #[cfg(feature = "v1-compat")]
    fn unstarted_termination_from_entry(
        entry_id: &str,
        entry: &Entry,
    ) -> Result<Option<UnstartedTermination>, DaemonError> {
        let target = match entry.process_handoff_phase {
            ProcessHandoffPhase::Prepared => UnstartedTermination {
                state: entry.state,
                phase: ProcessHandoffPhase::Prepared,
                generation: entry.process_handoff_generation,
                digest: entry.process_handoff_digest.clone(),
                handle: entry.prepared_process_handle.clone().ok_or_else(|| {
                    DaemonError::Refused(format!(
                        "entry {entry_id} has a prepared process handoff without a termination handle"
                    ))
                })?,
                other_handle: entry.last_process.clone(),
            },
            ProcessHandoffPhase::Registered => UnstartedTermination {
                state: entry.state,
                phase: ProcessHandoffPhase::Registered,
                generation: entry.process_handoff_generation,
                digest: entry.process_handoff_digest.clone(),
                handle: entry.last_process.clone().ok_or_else(|| {
                    DaemonError::Refused(format!(
                        "entry {entry_id} has a registered process handoff without a termination handle"
                    ))
                })?,
                other_handle: entry.prepared_process_handle.clone(),
            },
            _ => return Ok(None),
        };
        let valid_digest = target.digest.as_deref().is_some_and(|digest| {
            ProcessRegistration {
                generation: target.generation,
                handle: target.handle.clone(),
            }
            .canonical_handle_digest(entry_id)
                == digest
        });
        if !valid_digest {
            return Err(DaemonError::Refused(format!(
                "entry {entry_id} has accepted process authority without a matching canonical digest"
            )));
        }
        Ok(Some(target))
    }

    /// Durably enter cleanup quarantine before ending the accepted process or removing any
    /// artifact. Repeating it for an existing quarantine returns the same authority so a
    /// restart can finish the bounded recovery path.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn begin_unstarted_cleanup(
        &self,
        entry_id: &str,
    ) -> Result<Option<UnstartedTermination>, DaemonError> {
        let txn = self
            .db
            .begin_write()
            .map_err(|e| plain(e, "begin_unstarted_cleanup"))?;
        let target;
        {
            let mut table = txn
                .open_table(ENTRIES)
                .map_err(|e| plain(e, "begin_unstarted_cleanup"))?;
            let mut entry: Entry = table
                .get(entry_id)
                .map_err(|e| plain(e, "begin_unstarted_cleanup"))?
                .map(|bytes| serde_json::from_slice(bytes.value()))
                .transpose()?
                .ok_or_else(|| DaemonError::Missing(format!("entry {entry_id}")))?;
            if entry.started_at.is_some() {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} has started and cannot be abandoned"
                )));
            }
            if entry.state != EntryState::CleanupPending {
                if entry.state.capture_closed() {
                    return Err(DaemonError::Refused(format!(
                        "entry {entry_id} is already capture-closed and cannot be abandoned"
                    )));
                }
                entry.state = EntryState::CleanupPending;
            }
            target = Self::unstarted_termination_from_entry(entry_id, &entry)?;
            let bytes = serde_json::to_vec(&entry)?;
            table
                .insert(entry_id, bytes.as_slice())
                .map_err(|e| plain(e, "begin_unstarted_cleanup"))?;
        }
        txn.commit()
            .map_err(|e| plain(e, "begin_unstarted_cleanup"))?;
        Ok(target)
    }

    /// Persist the fact that the exact cleanup authority was successfully terminated before
    /// moving on to artifact removal. A restart may trust only this committed fact; old
    /// cleanup rows without it remain fail-closed and must re-verify their process.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn mark_unstarted_termination_complete(
        &self,
        entry_id: &str,
        expected: &UnstartedTermination,
    ) -> Result<(), DaemonError> {
        let txn = self
            .db
            .begin_write()
            .map_err(|e| plain(e, "mark_unstarted_termination_complete"))?;
        {
            let mut table = txn
                .open_table(ENTRIES)
                .map_err(|e| plain(e, "mark_unstarted_termination_complete"))?;
            let mut entry: Entry = table
                .get(entry_id)
                .map_err(|e| plain(e, "mark_unstarted_termination_complete"))?
                .map(|bytes| serde_json::from_slice(bytes.value()))
                .transpose()?
                .ok_or_else(|| DaemonError::Missing(format!("entry {entry_id}")))?;
            if entry.state != EntryState::CleanupPending || entry.started_at.is_some() {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} is not in unstarted cleanup quarantine"
                )));
            }
            let actual = Self::unstarted_termination_from_entry(entry_id, &entry)?;
            if actual.as_ref() != Some(expected) {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} cleanup authority changed; preserving its quarantine"
                )));
            }
            entry.cleanup_termination_complete = true;
            let bytes = serde_json::to_vec(&entry)?;
            table
                .insert(entry_id, bytes.as_slice())
                .map_err(|e| plain(e, "mark_unstarted_termination_complete"))?;
        }
        txn.commit()
            .map_err(|e| plain(e, "mark_unstarted_termination_complete"))
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn unstarted_termination_complete(
        &self,
        entry_id: &str,
    ) -> Result<bool, DaemonError> {
        Ok(self
            .get(entry_id)?
            .ok_or_else(|| DaemonError::Missing(format!("entry {entry_id}")))?
            .cleanup_termination_complete)
    }

    #[cfg(all(feature = "v1-compat", any(test, feature = "test-hooks")))]
    pub fn cleanup_termination_complete_for_test(
        &self,
        entry_id: &str,
    ) -> Result<bool, DaemonError> {
        self.unstarted_termination_complete(entry_id)
    }

    /// Remove a cleanup-quarantined row only if its termination authority is unchanged.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn complete_unstarted_cleanup(
        &self,
        entry_id: &str,
        expected: Option<&UnstartedTermination>,
    ) -> Result<(), DaemonError> {
        let txn = self
            .db
            .begin_write()
            .map_err(|e| plain(e, "complete_unstarted_cleanup"))?;
        {
            let mut table = txn
                .open_table(ENTRIES)
                .map_err(|e| plain(e, "complete_unstarted_cleanup"))?;
            let Some(entry) = table
                .get(entry_id)
                .map_err(|e| plain(e, "complete_unstarted_cleanup"))?
                .map(|bytes| serde_json::from_slice::<Entry>(bytes.value()))
                .transpose()?
            else {
                return Ok(());
            };
            if entry.state != EntryState::CleanupPending || entry.started_at.is_some() {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} is not in unstarted cleanup quarantine"
                )));
            }
            let actual = Self::unstarted_termination_from_entry(entry_id, &entry)?;
            if actual.as_ref() != expected {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} cleanup authority changed; preserving its quarantine"
                )));
            }
            table
                .remove(entry_id)
                .map_err(|e| plain(e, "complete_unstarted_cleanup"))?;
        }
        txn.commit()
            .map_err(|e| plain(e, "complete_unstarted_cleanup"))
    }

    pub fn remove(&self, entry_id: &str) -> Result<(), DaemonError> {
        let txn = self.db.begin_write().map_err(|e| plain(e, "remove"))?;
        {
            let mut table = txn.open_table(ENTRIES).map_err(|e| plain(e, "remove"))?;
            table.remove(entry_id).map_err(|e| plain(e, "remove"))?;
        }
        txn.commit().map_err(|e| plain(e, "remove"))
    }

    /// Every entry whose harness home is this one. The pre-arm scan asks the registry
    /// before it reads a byte of anyone's transcript.
    ///
    /// An entry whose `harness_home` is `None` has not been placed in a home yet and so
    /// occupies none: it is skipped rather than compared against a guessed path.
    pub fn in_harness_home(&self, home: &Path) -> Result<Vec<Entry>, DaemonError> {
        self.matching(|e| e.harness_home.as_deref() == Some(home))
    }

    /// Read, modify, write, in one transaction, returning what was stored.
    ///
    /// The closure may not rename the entry: the id is the key, and an id changed under
    /// it would leave the old key holding the new document. It cannot touch the receipt or
    /// process-handoff contract either — there is no setter to reach the private fields
    /// with, and the values the entry arrived with are restored afterwards regardless.
    /// `update` therefore cannot bypass receipt verification or launch authority.
    pub fn update<F>(&self, entry_id: &str, f: F) -> Result<Entry, DaemonError>
    where
        F: FnOnce(&mut Entry),
    {
        self.modify(entry_id, |e| {
            let verified = e.receipt.clone();
            let receipt_binding = e.receipt_binding.clone();
            let expected_process_handoff = e.expected_process_handoff;
            let process_handoff_generation = e.process_handoff_generation;
            let process_handoff_phase = e.process_handoff_phase;
            let process_handoff_digest = e.process_handoff_digest.clone();
            let prepared_process_handle = e.prepared_process_handle.clone();
            let prepared_home_seed = e.prepared_home_seed.clone();
            let cancelled_process_handoff_digest = e.cancelled_process_handoff_digest.clone();
            let sealing_from_state = e.sealing_from_state;
            let sealing_intent = e.sealing_intent.clone();
            let sealed_trace_root_identity = e.sealed_trace_root_identity.clone();
            let terminal_drain_validation_failed = e.terminal_drain_validation_failed;
            let deadline_observed = e.deadline_observed;
            let finalization_observed_unix = e.finalization_observed_unix;
            let source_date = e.source_date.clone();
            let last_process = e.last_process.clone();
            f(e);
            e.receipt = verified;
            e.receipt_binding = receipt_binding;
            e.expected_process_handoff = expected_process_handoff;
            e.process_handoff_generation = process_handoff_generation;
            e.process_handoff_phase = process_handoff_phase;
            e.process_handoff_digest = process_handoff_digest;
            e.prepared_process_handle = prepared_process_handle;
            e.prepared_home_seed = prepared_home_seed;
            e.cancelled_process_handoff_digest = cancelled_process_handoff_digest;
            e.sealing_from_state = sealing_from_state;
            e.sealing_intent = sealing_intent;
            e.sealed_trace_root_identity = sealed_trace_root_identity;
            e.terminal_drain_validation_failed = terminal_drain_validation_failed;
            e.deadline_observed = deadline_observed;
            e.finalization_observed_unix = finalization_observed_unix;
            e.source_date = source_date;
            e.last_process = last_process;
        })
    }

    /// Durably retain one bounded foreign identity before a live sweep returns. The
    /// transaction is idempotent by the identity tuple so a retry cannot grow the row.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn retain_foreign_pending(
        &self,
        entry_id: &str,
        pending: PendingForeignSighting,
    ) -> Result<Entry, DaemonError> {
        self.modify_handoff(entry_id, "retain_foreign_pending", |entry| {
            if entry.foreign_pending.iter().any(|existing| {
                existing.family == pending.family
                    && existing.harness == pending.harness
                    && existing.session_id == pending.session_id
            }) {
                return Ok(());
            }
            if entry.foreign_pending.len() >= MAX_FOREIGN_PENDING_SIGHTINGS {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} reached the {}-identity pending foreign bound",
                    MAX_FOREIGN_PENDING_SIGHTINGS
                )));
            }
            entry.foreign_pending.push(pending);
            Ok(())
        })
    }

    /// Clear one pending identity after its event, if any, has been chained. Clearing is
    /// idempotent so a crash after the chain append is recovered by the chain's identity set
    /// before this transaction is retried.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn clear_foreign_pending(
        &self,
        entry_id: &str,
        family: &str,
        harness: &str,
        session_id: &str,
    ) -> Result<Entry, DaemonError> {
        self.update(entry_id, |entry| {
            entry.foreign_pending.retain(|pending| {
                !(pending.family == family
                    && pending.harness == harness
                    && pending.session_id == session_id)
            });
        })
    }

    /// Persist the one terminal-drain outcome the trace cannot chain after its seal.  This
    /// is intentionally monotonic: a later upload cadence may retry, but it may never turn
    /// an evidence-validation failure back into an all-clear completed status.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn record_terminal_drain_validation_failure(
        &self,
        entry_id: &str,
    ) -> Result<Entry, DaemonError> {
        #[cfg(any(test, feature = "test-hooks"))]
        if self
            .fail_next_terminal_drain_validation_persistence
            .swap(false, Ordering::SeqCst)
        {
            return Err(DaemonError::Refused(
                "test-hooks: terminal quarantine persistence failed".to_string(),
            ));
        }
        self.modify_handoff(
            entry_id,
            "record_terminal_drain_validation_failure",
            |entry| {
                if !entry.state.capture_closed() {
                    return Err(DaemonError::Refused(format!(
                        "entry {entry_id} is not capture-closed"
                    )));
                }
                entry.terminal_drain_validation_failed = true;
                Ok(())
            },
        )
    }

    /// Test-only: fail the exact monotonic terminal-quarantine transaction once. This is
    /// narrower than a generic registry write failure so live, retained, and startup callers
    /// can prove their common fail-closed boundary without weakening unrelated persistence.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_next_terminal_drain_validation_persistence_for_test(&self) {
        self.fail_next_terminal_drain_validation_persistence
            .store(true, Ordering::SeqCst);
    }

    /// Reserve the cross-store seal transition before a terminal envelope is written.  The
    /// private predecessor is the only state startup may restore when the trace has no seal;
    /// `Prepared` is deliberately refused because its accepted child is not yet in
    /// `last_process` and sealing it could orphan the authority.
    pub fn begin_sealing(
        &self,
        entry_id: &str,
        intent: SealingIntent,
    ) -> Result<Entry, DaemonError> {
        if matches!(&intent, SealingIntent::Workspace(_)) {
            return Err(DaemonError::Refused(format!(
                "entry {entry_id} workspace finalization must use the admission-closing CAS"
            )));
        }
        intent.validate(entry_id)?;
        self.modify_handoff(entry_id, "begin_sealing", |entry| {
            if matches!(
                entry.state,
                EntryState::CleanupPending | EntryState::Sealed | EntryState::Refused
            ) || entry.sealed_at.is_some()
            {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} is already terminal and cannot begin another seal"
                )));
            }
            if matches!(entry.state, EntryState::Sealing) {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} already has an unreconciled sealing intent"
                )));
            }
            if matches!(entry.process_handoff_phase, ProcessHandoffPhase::Prepared) {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} has a prepared process handoff; reconcile it before sealing"
                )));
            }
            let previous = entry.state;
            entry.state = EntryState::Sealing;
            entry.sealing_from_state = Some(previous);
            entry.sealing_intent = Some(intent);
            entry.sealed_trace_root_identity = None;
            Ok(())
        })
    }

    /// Close admission and install the V4 intent under the same redb serialization domain
    /// as process handoff prepare/register/cancel.
    ///
    /// `build` runs inside the write transaction so it derives the reason, deadline-bound
    /// cutoff, receipt digest, generation and accepted handoff from the one row it closes.
    /// A concurrent finisher joins the existing workspace intent and cannot replace any of
    /// those facts.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn begin_workspace_finalization<F>(
        &self,
        entry_id: &str,
        build: F,
    ) -> Result<Entry, DaemonError>
    where
        F: FnOnce(&mut Entry) -> Result<WorkspaceSealingIntentV1, DaemonError>,
    {
        self.modify_handoff(entry_id, "begin_workspace_finalization", |entry| {
            if let Some(intent) = &entry.sealing_intent {
                return match intent {
                    SealingIntent::Workspace(intent) => {
                        intent.validate(entry_id)?;
                        if !matches!(entry.state, EntryState::Sealing | EntryState::Sealed) {
                            return Err(DaemonError::Refused(format!(
                                "entry {entry_id} retains a workspace finalization intent in invalid state {:?}",
                                entry.state
                            )));
                        }
                        Ok(())
                    }
                    SealingIntent::Legacy(_) => Err(DaemonError::Refused(format!(
                        "entry {entry_id} already has an unreconciled legacy sealing intent"
                    ))),
                };
            }
            if matches!(
                entry.state,
                EntryState::CleanupPending
                    | EntryState::Sealing
                    | EntryState::Sealed
                    | EntryState::Refused
            ) || entry.sealed_at.is_some()
            {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} is already terminal and cannot begin workspace finalization"
                )));
            }
            if matches!(entry.process_handoff_phase, ProcessHandoffPhase::Prepared) {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} has a prepared process handoff; reconcile it before finalization"
                )));
            }
            let intent = build(entry)?;
            intent.validate(entry_id)?;
            intent.validate_admission_authority(entry)?;
            if intent.phase != FinalizationPhaseV1::AdmissionClosed {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} must begin workspace finalization at admission_closed"
                )));
            }
            let previous = entry.state;
            entry.state = EntryState::Sealing;
            entry.sealing_from_state = Some(previous);
            entry.sealing_intent = Some(SealingIntent::Workspace(Box::new(intent)));
            entry.sealed_trace_root_identity = None;
            Ok(())
        })
    }

    /// Latch a signed deadline observation once. A later wall-clock rollback cannot clear
    /// this bit through generic row replacement, so manual and automatic finishers share
    /// the same forward-only authority.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn observe_signed_deadline(
        &self,
        entry_id: &str,
        observed_unix: f64,
    ) -> Result<Entry, DaemonError> {
        self.modify_handoff(entry_id, "observe_signed_deadline", |entry| {
            entry.observe_finalization_time(observed_unix)?;
            Ok(())
        })
    }

    /// Commit one monotone workspace checkpoint under the shared finalization/handoff CAS.
    /// Replaying an already-committed or later phase is a read-only join; moving backward,
    /// skipping the caller's expected phase, or changing immutable admission facts refuses.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn advance_workspace_finalization<F>(
        &self,
        entry_id: &str,
        expected: FinalizationPhaseV1,
        next: FinalizationPhaseV1,
        update: F,
    ) -> Result<Entry, DaemonError>
    where
        F: FnOnce(&mut WorkspaceSealingIntentV1) -> Result<(), DaemonError>,
    {
        if expected.rank().checked_add(1) != Some(next.rank()) {
            return Err(DaemonError::Refused(format!(
                "entry {entry_id} workspace finalization must advance exactly one checkpoint from {expected:?} to {next:?}"
            )));
        }
        self.modify_handoff(entry_id, "advance_workspace_finalization", |entry| {
            let terminal_status_completion = expected
                == FinalizationPhaseV1::TerminalStatusPending
                && next == FinalizationPhaseV1::Terminal;
            if (!terminal_status_completion
                && (entry.state != EntryState::Sealing || entry.sealed_at.is_some()))
                || (terminal_status_completion
                    && (entry.state != EntryState::Sealed || entry.sealed_at.is_none()))
            {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} has no active workspace finalization intent"
                )));
            }
            let intent = match entry.sealing_intent.as_mut() {
                Some(SealingIntent::Workspace(intent)) => intent,
                Some(SealingIntent::Legacy(_)) | None => {
                    return Err(DaemonError::Refused(format!(
                        "entry {entry_id} has no workspace finalization intent"
                    )));
                }
            };
            intent.validate(entry_id)?;
            if next.permits(intent.phase) {
                return Ok(());
            }
            if intent.phase != expected {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} workspace finalization is at {:?}, not expected {expected:?}",
                    intent.phase
                )));
            }
            let immutable = WorkspaceIntentImmutableV1::from(intent.as_ref());
            update(intent)?;
            intent.phase = next;
            if WorkspaceIntentImmutableV1::from(intent.as_ref()) != immutable {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} workspace finalization attempted to replace immutable admission facts"
                )));
            }
            intent.validate(entry_id)?;
            if matches!(
                next,
                FinalizationPhaseV1::CutoffBound | FinalizationPhaseV1::FinalizationBound
            ) {
                let link = if next == FinalizationPhaseV1::CutoffBound {
                    intent.cutoff.as_ref()
                } else {
                    intent.finalization.as_ref()
                }
                .ok_or_else(|| {
                    DaemonError::Refused(format!(
                        "entry {entry_id} bound a workspace chain phase without its link"
                    ))
                })?;
                entry.chain_seq = Some(link.seq);
                entry.chain_head = link.hash.clone();
                if next == FinalizationPhaseV1::FinalizationBound {
                    entry.sealed_trace_root_identity = Some(intent.trace_root_identity.clone());
                }
            }
            if next == FinalizationPhaseV1::TerminalStatusPending {
                let sealed_at = intent.finalization_at.as_ref().ok_or_else(|| {
                    DaemonError::Refused(format!(
                        "entry {entry_id} cannot promote without a finalization timestamp"
                    ))
                })?;
                entry.state = EntryState::Sealed;
                entry.sealed_at = Some(sealed_at.as_str().to_string());
            }
            Ok(())
        })
    }

    /// Reopen an interrupted sealing intent only after recovery established that the
    /// validated chain has no terminal envelope.  This does not decrement process
    /// generation or otherwise mutate launch authority.
    pub fn restore_unfinished_sealing(&self, entry_id: &str) -> Result<Entry, DaemonError> {
        self.modify_handoff(entry_id, "restore_unfinished_sealing", |entry| {
            if entry.state != EntryState::Sealing || entry.sealed_at.is_some() {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} has no unfinished sealing intent to restore"
                )));
            }
            let previous = entry.sealing_from_state.ok_or_else(|| {
                DaemonError::Refused(format!(
                    "entry {entry_id} is sealing without an open predecessor state"
                ))
            })?;
            if matches!(
                &entry.sealing_intent,
                Some(SealingIntent::Workspace(_))
            ) {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} has a workspace finalization intent and can never reopen capture"
                )));
            }
            if matches!(
                previous,
                EntryState::CleanupPending
                    | EntryState::Sealing
                    | EntryState::Sealed
                    | EntryState::Refused
            ) {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} has an invalid sealing predecessor {previous:?}"
                )));
            }
            entry.state = previous;
            entry.sealing_from_state = None;
            entry.sealing_intent = None;
            Ok(())
        })
    }

    /// Record the exact, validated terminal head before the row may become `Sealed`.
    /// This is intentionally its own durable step: promotion without the matching head
    /// would publish a sealed registry row whose descriptor repair could still name a
    /// different terminal history after a crash.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn record_sealing_terminal_head(
        &self,
        entry_id: &str,
        terminal: &Envelope,
    ) -> Result<Entry, DaemonError> {
        self.modify_handoff(entry_id, "record_sealing_terminal_head", |entry| {
            if entry.state != EntryState::Sealing || entry.sealed_at.is_some() {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} no longer holds the sealing intent being staged"
                )));
            }
            let intent = entry.sealing_intent.as_ref().ok_or_else(|| {
                DaemonError::Refused(format!(
                    "entry {entry_id} is sealing without an exact staged manifest"
                ))
            })?;
            intent.matches_terminal(terminal)?;
            entry.chain_seq = Some(terminal.link.seq);
            entry.chain_head = terminal.link.hash.clone();
            Ok(())
        })
    }

    /// Promote a sealing intent only from the exact terminal envelope recovery read from a
    /// protocol-validated chain.  The buffer repair happens before this transaction, so a
    /// crash either leaves `Sealing` recoverable or leaves a sealed row whose terminal link
    /// can be uploaded.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn promote_sealing_terminal(
        &self,
        entry_id: &str,
        terminal: &Envelope,
    ) -> Result<Entry, DaemonError> {
        let EventBody::Seal { sealed_at, .. } = &terminal.body else {
            return Err(DaemonError::Refused(
                "sealing recovery was not given a terminal seal envelope".to_string(),
            ));
        };
        if terminal.entry_id != entry_id {
            return Err(DaemonError::Refused(format!(
                "sealing recovery terminal belongs to {}, not entry {entry_id}",
                terminal.entry_id
            )));
        }
        self.modify_handoff(entry_id, "promote_sealing_terminal", |entry| {
            if entry.state != EntryState::Sealing || entry.sealed_at.is_some() {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} no longer holds the sealing intent being promoted"
                )));
            }
            if entry.sealing_from_state.is_none() {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} is sealing without an open predecessor state"
                )));
            }
            let intent = entry.sealing_intent.as_ref().ok_or_else(|| {
                DaemonError::Refused(format!(
                    "entry {entry_id} is sealing without an exact staged manifest"
                ))
            })?;
            intent.matches_terminal(terminal)?;
            let trace_root_identity = intent.legacy()?.trace_root_identity.clone();
            if entry.chain_seq != Some(terminal.link.seq) || entry.chain_head != terminal.link.hash
            {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} has not durably staged this terminal chain head"
                )));
            }
            entry.state = EntryState::Sealed;
            entry.sealed_at = Some(sealed_at.clone());
            entry.sealing_from_state = None;
            entry.sealing_intent = None;
            entry.sealed_trace_root_identity = Some(trace_root_identity);
            Ok(())
        })
    }

    /// A V1 sealed row written before root capabilities were persisted may drain only
    /// after its validated terminal trace has been tied to the registry's exact head.  This
    /// narrow migration is deliberately unavailable to generic updates: it never changes a
    /// terminal link, and it cannot bless an open or mismatched row.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn record_legacy_sealed_trace_root_identity(
        &self,
        entry_id: &str,
        actual_root_identity: &str,
        terminal: &Envelope,
    ) -> Result<Entry, DaemonError> {
        if actual_root_identity.is_empty() {
            return Err(DaemonError::Refused(format!(
                "sealed entry {entry_id} has an empty trace-root identity"
            )));
        }
        let EventBody::Seal { .. } = terminal.body else {
            return Err(DaemonError::Refused(format!(
                "sealed entry {entry_id} legacy root migration requires a seal terminal"
            )));
        };
        self.modify_handoff(
            entry_id,
            "record_legacy_sealed_trace_root_identity",
            |entry| {
                if entry.state != EntryState::Sealed
                    || entry.sealing_intent.is_some()
                    || entry.sealing_from_state.is_some()
                    || entry.sealed_trace_root_identity.is_some()
                    || entry.chain_seq != Some(terminal.link.seq)
                    || entry.chain_head != terminal.link.hash
                {
                    return Err(DaemonError::Refused(format!(
                        "sealed entry {entry_id} no longer matches the legacy trace-root migration"
                    )));
                }
                entry.sealed_trace_root_identity = Some(actual_root_identity.to_string());
                Ok(())
            },
        )
    }

    /// Test-only fixture seam for the historical V1 sealed-row shape, which did not carry
    /// a trace-root identity.  Production writes cannot clear this field.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn clear_sealed_trace_root_identity_for_test(
        &self,
        entry_id: &str,
    ) -> Result<Entry, DaemonError> {
        self.modify_handoff(
            entry_id,
            "clear_sealed_trace_root_identity_for_test",
            |entry| {
                if entry.state != EntryState::Sealed {
                    return Err(DaemonError::Refused(format!(
                        "entry {entry_id} is not sealed for the legacy-root fixture"
                    )));
                }
                entry.sealed_trace_root_identity = None;
                Ok(())
            },
        )
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn sealed_trace_root_identity_for_test(
        &self,
        entry_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        Ok(self
            .get(entry_id)?
            .and_then(|entry| entry.sealed_trace_root_identity))
    }

    /// Test-only corrupt-row fixture: production handoff writers never make a Registered
    /// phase without a target.  The reaper needs an executable regression for refusing to
    /// silently skip such a durable contradiction after a crash.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn mark_registered_without_termination_handle_for_test(
        &self,
        entry_id: &str,
    ) -> Result<Entry, DaemonError> {
        self.modify_handoff(
            entry_id,
            "mark_registered_without_termination_handle_for_test",
            |entry| {
                entry.process_handoff_generation = 1;
                entry.process_handoff_phase = ProcessHandoffPhase::Registered;
                entry.process_handoff_digest = Some("test-corrupt-registered-row".to_string());
                entry.last_process = None;
                Ok(())
            },
        )
    }

    /// Open generation one after arming.  A request received while `Created`, attached,
    /// terminal, or already opened has no authority and changes nothing.
    pub fn open_initial_process_handoff(&self, entry_id: &str) -> Result<Entry, DaemonError> {
        self.modify_handoff(entry_id, "open_initial_process_handoff", |entry| {
            if !matches!(entry.state, EntryState::Armed | EntryState::StartSignalSeen) {
                return Err(DaemonError::Guard(format!(
                    "entry {entry_id} is {:?}; an initial process handoff opens only after arm",
                    entry.state
                )));
            }
            if !matches!(
                entry.expected_process_handoff,
                ExpectedProcessHandoff::Required
            ) || !matches!(
                entry.process_handoff_phase,
                ProcessHandoffPhase::InitialClosed
            ) || entry.process_handoff_generation != 0
            {
                return Err(DaemonError::Guard(format!(
                    "entry {entry_id} has no unopened daemon-launched initial process authority"
                )));
            }
            entry.process_handoff_generation = 1;
            entry.process_handoff_phase = ProcessHandoffPhase::OpenInitial;
            Ok(())
        })
    }

    /// Materialize a historical `Required` row before a rebuilt runtime can expose it to a
    /// modern route.  Old rows had no phase, so retaining that absence after a V2 daemon has
    /// opened the registry would let legacy health/termination facts and durable authority
    /// drift apart.  This transaction is deliberately narrow: started legacy rows retain
    /// their guarded resume migration, while pre-start rows become explicit state.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn materialize_legacy_required_handoff(
        &self,
        entry_id: &str,
    ) -> Result<Entry, DaemonError> {
        let txn = self
            .db
            .begin_write()
            .map_err(|e| plain(e, "materialize_legacy_required_handoff"))?;
        let materialized;
        {
            let mut table = txn
                .open_table(ENTRIES)
                .map_err(|e| plain(e, "materialize_legacy_required_handoff"))?;
            let mut entry: Entry = {
                let found = table
                    .get(entry_id)
                    .map_err(|e| plain(e, "materialize_legacy_required_handoff"))?;
                match found {
                    Some(bytes) => serde_json::from_slice(bytes.value())?,
                    None => return Err(DaemonError::Missing(format!("entry {entry_id}"))),
                }
            };

            // Historical attached/unknown rows and terminal rows remain on their existing
            // guarded paths.  Return before serialization: this migration is neither an
            // acknowledgement nor an opportunity to rewrite a row it does not own.
            if !matches!(
                entry.expected_process_handoff,
                ExpectedProcessHandoff::Required
            ) || !matches!(
                entry.process_handoff_phase,
                ProcessHandoffPhase::LegacyUnknown
            ) || entry.state.capture_closed()
            {
                return Ok(entry);
            }

            // The legacy handoff fields were absent, not the lifecycle facts around them.
            // Do not let a malformed state/receipt/timestamp combination choose an open
            // modern authority shape: it is not evidence that a process may be registered.
            // This check intentionally precedes every materialization write and every
            // auxiliary-field diagnostic, so a refusal leaves the exact historical bytes
            // intact for repair.
            if !Self::legacy_required_handoff_lifecycle_is_coherent(&entry) {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} has an impossible legacy state/receipt/timestamp tuple and cannot be materialized"
                )));
            }

            // A complete old row had none of V1's authority evidence.  Any one of these
            // fields says the row is a partial/mixed upgrade or an impossible interrupted
            // write; guessing its intended state would make a later process request look
            // authorized.  Refusal drops this transaction without writing any bytes.
            if entry.process_handoff_generation != 0
                || entry.process_handoff_digest.is_some()
                || entry.prepared_process_handle.is_some()
                || entry.prepared_home_seed.is_some()
                || entry.cancelled_process_handoff_digest.is_some()
            {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} has a partial legacy process handoff and cannot be materialized"
                )));
            }

            match (&entry.state, entry.last_process.clone()) {
                (EntryState::Created, None) => {
                    entry.process_handoff_phase = ProcessHandoffPhase::InitialClosed;
                }
                (EntryState::Armed | EntryState::StartSignalSeen, None) => {
                    entry.process_handoff_generation = 1;
                    entry.process_handoff_phase = ProcessHandoffPhase::OpenInitial;
                }
                (
                    EntryState::Armed | EntryState::StartSignalSeen | EntryState::Started,
                    Some(mut handle),
                ) => {
                    // A legacy `last_process` is termination identity, not a prepared
                    // handoff.  Preserve every termination field, discard only seed
                    // evidence, and derive the canonical generation-one identity from that
                    // stripped handle.
                    handle.home_seed = None;
                    let registration = ProcessRegistration {
                        generation: 1,
                        handle: handle.clone(),
                    };
                    entry.process_handoff_generation = 1;
                    entry.process_handoff_phase = ProcessHandoffPhase::Registered;
                    entry.process_handoff_digest =
                        Some(registration.canonical_handle_digest(entry_id));
                    entry.last_process = Some(handle);
                }
                (EntryState::Started, None) => {
                    return Err(DaemonError::Refused(format!(
                        "entry {entry_id} started without a legacy termination handle and cannot be materialized"
                    )));
                }
                (EntryState::Created, Some(_)) => {
                    return Err(DaemonError::Refused(format!(
                        "entry {entry_id} was created with a legacy termination handle and cannot be materialized"
                    )));
                }
                // Every `EntryState` is covered above or excluded by `capture_closed`.
                (
                    EntryState::CleanupPending
                    | EntryState::Sealing
                    | EntryState::Sealed
                    | EntryState::Refused,
                    _,
                ) => {
                    unreachable!("terminal rows returned before materialization")
                }
            }

            let bytes = serde_json::to_vec(&entry)?;
            table
                .insert(entry_id, bytes.as_slice())
                .map_err(|e| plain(e, "materialize_legacy_required_handoff"))?;
            materialized = entry;
        }
        txn.commit()
            .map_err(|e| plain(e, "materialize_legacy_required_handoff"))?;
        Ok(materialized)
    }

    #[cfg(feature = "v1-compat")]
    fn legacy_required_handoff_lifecycle_is_coherent(entry: &Entry) -> bool {
        let no_start_facts = entry.receipt.is_none()
            && entry.started_at.is_none()
            && entry.deadline_at.is_none()
            && entry.deadline_unix.is_none();
        match entry.state {
            EntryState::Created | EntryState::Armed | EntryState::StartSignalSeen => no_start_facts,
            EntryState::Started => {
                let (Some(receipt), Some(started_at), Some(deadline_at), Some(deadline_unix)) = (
                    entry.receipt.as_ref(),
                    entry.started_at.as_ref(),
                    entry.deadline_at.as_ref(),
                    entry.deadline_unix,
                ) else {
                    return false;
                };
                receipt.claims.started_at == *started_at
                    && receipt.claims.deadline_at == *deadline_at
                    && goalcup_protocol::control::parse_z(deadline_at)
                        .is_some_and(|expected| expected as f64 == deadline_unix)
            }
            // Terminal rows returned before this helper. Keeping this total makes the
            // invariant explicit if the call order changes later.
            EntryState::CleanupPending
            | EntryState::Sealing
            | EntryState::Sealed
            | EntryState::Refused => true,
        }
    }

    /// Persist the accepted request identity before the trace event.  The retry-safe record
    /// holds only a stripped termination handle and a bounded public seed description; it
    /// never retains copied authentication-file bytes, a resolved home path, or a digest of
    /// seed contents.
    pub fn prepare_process_handoff(
        &self,
        entry_id: &str,
        registration: ProcessRegistration,
    ) -> Result<Entry, DaemonError> {
        let digest = registration.canonical_handle_digest(entry_id);
        let handle = registration.stripped_handle();
        self.modify_handoff(entry_id, "prepare_process_handoff", |entry| {
            // A successful registration is a pure acknowledgement on every retry, even if
            // a concurrent seal has made the row terminal.  It must never re-open or mutate.
            if matches!(entry.process_handoff_phase, ProcessHandoffPhase::Registered)
                && entry.process_handoff_generation == registration.generation
            {
                return if entry.process_handoff_digest.as_deref() == Some(digest.as_str()) {
                    Ok(())
                } else {
                    Err(handoff_conflict(
                        entry_id,
                        "a different process identity already registered",
                    ))
                };
            }
            if !matches!(
                entry.state,
                EntryState::Armed | EntryState::StartSignalSeen | EntryState::Started
            ) || !entry.accepts_process_handoff()
            {
                return Err(DaemonError::Guard(format!(
                    "entry {entry_id} does not currently accept a daemon-launched process handoff"
                )));
            }
            if registration.generation != entry.process_handoff_generation {
                return Err(handoff_conflict(
                    entry_id,
                    "the handoff generation is stale or was not issued",
                ));
            }
            match entry.process_handoff_phase {
                ProcessHandoffPhase::OpenInitial | ProcessHandoffPhase::OpenResume => {
                    // Only a new Open -> Prepared transition inspects seed material.  An
                    // exact replay's original safe seed already won; an alternate or even
                    // malformed seed must not turn that idempotent acknowledgement into a
                    // new failure or overwrite recovery evidence.
                    let seed = safe_seed_for_recovery(registration.handle.home_seed.clone())?;
                    entry.process_handoff_phase = ProcessHandoffPhase::Prepared;
                    entry.process_handoff_digest = Some(digest.clone());
                    entry.prepared_process_handle = Some(handle.clone());
                    entry.prepared_home_seed = seed.clone();
                    entry.cancelled_process_handoff_digest = None;
                    Ok(())
                }
                ProcessHandoffPhase::Prepared => {
                    if entry.process_handoff_digest.as_deref() == Some(digest.as_str()) {
                        Ok(())
                    } else {
                        Err(handoff_conflict(
                            entry_id,
                            "a different process identity already prepared",
                        ))
                    }
                }
                ProcessHandoffPhase::LegacyUnknown
                | ProcessHandoffPhase::InitialClosed
                | ProcessHandoffPhase::Registered
                | ProcessHandoffPhase::ResumeCancelled => Err(handoff_conflict(
                    entry_id,
                    "no matching open process handoff exists",
                )),
            }
        })
    }

    /// Whether this exact already-admitted request may be replayed without treating its
    /// historical fields as new authority.
    ///
    /// Older durable rows lack additive process-instance fields. They are still valid
    /// evidence of the authority accepted by the previous daemon, but must never be
    /// admitted afresh through an Open handoff. The route uses this read-only predicate to
    /// bypass current-live validation only for an exact `Prepared`/`Registered` replay;
    /// `prepare_process_handoff` repeats the digest check before it writes anything.
    pub fn is_exact_accepted_process_replay(
        &self,
        entry_id: &str,
        registration: &ProcessRegistration,
    ) -> Result<bool, DaemonError> {
        Ok(self
            .exact_accepted_process_replay_phase(entry_id, registration)?
            .is_some())
    }

    /// The durable phase of an exact accepted process replay. The route needs the phase,
    /// not only a boolean: Prepared still requires reconciliation and preserves transient
    /// Busy, while Registered may be acknowledged without a runtime only after seal.
    pub fn exact_accepted_process_replay_phase(
        &self,
        entry_id: &str,
        registration: &ProcessRegistration,
    ) -> Result<Option<ProcessHandoffPhase>, DaemonError> {
        let Some(entry) = self.get(entry_id)? else {
            return Ok(None);
        };
        let digest = registration.canonical_handle_digest(entry_id);
        Ok((matches!(
            entry.process_handoff_phase,
            ProcessHandoffPhase::Prepared | ProcessHandoffPhase::Registered
        ) && entry.process_handoff_generation == registration.generation
            && entry.process_handoff_digest.as_deref() == Some(digest.as_str()))
        .then_some(entry.process_handoff_phase))
    }

    /// Acknowledge the one `/process` retry whose exact Registered runtime is unavailable.
    /// This is read-only: it neither reopens authority nor changes the termination handle.
    /// Only a sealed row qualifies. An unsealed Registered row without a runtime would
    /// acknowledge an unmonitored live harness, which is not a successful recovery.
    pub fn acknowledge_sealed_process_replay(
        &self,
        entry_id: &str,
        registration: &ProcessRegistration,
    ) -> Result<(), DaemonError> {
        let entry = self
            .get(entry_id)?
            .ok_or_else(|| DaemonError::Missing(format!("entry {entry_id}")))?;
        let digest = registration.canonical_handle_digest(entry_id);
        if entry.state == EntryState::Sealed
            && entry.sealed_at.is_some()
            && matches!(entry.process_handoff_phase, ProcessHandoffPhase::Registered)
            && entry.process_handoff_generation == registration.generation
            && entry.process_handoff_digest.as_deref() == Some(digest.as_str())
        {
            Ok(())
        } else {
            Err(handoff_conflict(
                entry_id,
                "the sealed entry does not hold this exact registered process handoff",
            ))
        }
    }

    /// Complete a prepared handoff only after the runtime has reconciled its seed marker in
    /// the validated chain.  This transaction is the sole writer of `last_process`.
    pub fn register_prepared_process_handoff(
        &self,
        entry_id: &str,
        generation: u64,
    ) -> Result<Entry, DaemonError> {
        self.modify_handoff(entry_id, "register_prepared_process_handoff", |entry| {
            if entry.process_handoff_generation != generation {
                return Err(handoff_conflict(
                    entry_id,
                    "the prepared generation is no longer current",
                ));
            }
            if matches!(entry.process_handoff_phase, ProcessHandoffPhase::Registered) {
                return Ok(());
            }
            if !matches!(entry.process_handoff_phase, ProcessHandoffPhase::Prepared)
                || matches!(
                    entry.state,
                    EntryState::CleanupPending | EntryState::Sealed | EntryState::Refused
                )
            {
                return Err(handoff_conflict(
                    entry_id,
                    "the process handoff is not prepared for registration",
                ));
            }
            let handle = entry.prepared_process_handle.clone().ok_or_else(|| {
                DaemonError::Refused(format!(
                    "entry {entry_id} has a prepared handoff without a handle"
                ))
            })?;
            goalcup_protocol::process::validate_live_process_handle(&handle).map_err(|reason| {
                DaemonError::Refused(format!(
                    "entry {entry_id}'s prepared process could not be revalidated: {reason}"
                ))
            })?;
            entry.last_process = Some(handle);
            entry.process_handoff_phase = ProcessHandoffPhase::Registered;
            entry.prepared_process_handle = None;
            entry.prepared_home_seed = None;
            Ok(())
        })
    }

    /// Clone the exact handle a runtime is about to reconcile.  The runtime validates it
    /// before writing a HomeSeeded marker; the registry validates again in the promotion
    /// transaction so neither replay nor restart can install stale authority.
    pub(crate) fn prepared_process_handle(
        &self,
        entry_id: &str,
        generation: u64,
    ) -> Result<ProcessHandle, DaemonError> {
        let entry = self
            .get(entry_id)?
            .ok_or_else(|| DaemonError::Missing(format!("entry {entry_id}")))?;
        if entry.process_handoff_generation != generation
            || !matches!(entry.process_handoff_phase, ProcessHandoffPhase::Prepared)
        {
            return Err(handoff_conflict(
                entry_id,
                "the process handoff is no longer prepared",
            ));
        }
        entry.prepared_process_handle.ok_or_else(|| {
            DaemonError::Refused(format!(
                "entry {entry_id} has a prepared handoff without a handle"
            ))
        })
    }

    /// Read the private prepared evidence for the one runtime that owns this entry slot.
    /// A clone prevents the caller from mutating the registry's copy before reconciliation.
    pub fn prepared_home_seed(
        &self,
        entry_id: &str,
        generation: u64,
    ) -> Result<Option<(String, HomeSeedRecord)>, DaemonError> {
        let entry = self
            .get(entry_id)?
            .ok_or_else(|| DaemonError::Missing(format!("entry {entry_id}")))?;
        if entry.process_handoff_generation != generation
            || !matches!(entry.process_handoff_phase, ProcessHandoffPhase::Prepared)
        {
            return Err(handoff_conflict(
                entry_id,
                "the process handoff is no longer prepared",
            ));
        }
        Ok(entry
            .prepared_home_seed
            .map(|seed| (entry.process_handoff_digest.unwrap_or_default(), seed)))
    }

    /// Inspect an issued resume cancellation while the runtime slot is held.  A bare cancel
    /// may close only an `OpenResume`; once a child is `Prepared`, the caller must prove it
    /// is cancelling that exact canonical request before it may terminate it.  Registered
    /// authority is never rolled back by this route.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn prepared_resume_termination_for_cancel(
        &self,
        entry_id: &str,
        generation: u64,
        request_digest: Option<&str>,
    ) -> Result<Option<PreparedResumeTermination>, DaemonError> {
        let entry = self
            .get(entry_id)?
            .ok_or_else(|| DaemonError::Missing(format!("entry {entry_id}")))?;
        if !matches!(entry.state, EntryState::Started)
            || entry.session_id.as_deref().is_none_or(str::is_empty)
            || entry.process_handoff_generation != generation
        {
            return Err(handoff_conflict(
                entry_id,
                "the resume cancellation generation is no longer current",
            ));
        }
        match entry.process_handoff_phase {
            ProcessHandoffPhase::OpenResume => Ok(None),
            ProcessHandoffPhase::ResumeCancelled => {
                if entry.cancelled_process_handoff_digest.as_deref() == request_digest {
                    Ok(None)
                } else {
                    Err(handoff_conflict(
                        entry_id,
                        "the cancelled resume belongs to a different request",
                    ))
                }
            }
            ProcessHandoffPhase::Prepared => {
                if entry.process_handoff_digest.as_deref() != request_digest {
                    return Err(handoff_conflict(
                        entry_id,
                        "the prepared process belongs to a different request",
                    ));
                }
                Ok(Some(PreparedResumeTermination {
                    handle: entry.prepared_process_handle.clone().ok_or_else(|| {
                        DaemonError::Refused(format!(
                            "entry {entry_id} has a prepared resume without a termination handle"
                        ))
                    })?,
                }))
            }
            _ => Err(handoff_conflict(
                entry_id,
                "a registered or different process authority cannot be cancelled",
            )),
        }
    }

    /// Commit a cancellation only after [`Self::prepared_resume_termination_for_cancel`]'s
    /// child, if any, was verified gone.  The advanced generation remains standing so no
    /// observer can ever reuse it; a subsequent resume opens `generation + 1`.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn finish_cancel_resume_handoff(
        &self,
        entry_id: &str,
        generation: u64,
        request_digest: Option<&str>,
    ) -> Result<Entry, DaemonError> {
        self.modify_handoff(entry_id, "finish_cancel_resume_handoff", |entry| {
            if !matches!(entry.state, EntryState::Started)
                || entry.session_id.as_deref().is_none_or(str::is_empty)
                || entry.process_handoff_generation != generation
            {
                return Err(handoff_conflict(
                    entry_id,
                    "the resume cancellation generation is no longer current",
                ));
            }
            match entry.process_handoff_phase {
                ProcessHandoffPhase::OpenResume => {
                    // A definite client-side registration failure may have happened before
                    // or after the server prepared.  If it was before, the supplied intended
                    // digest is merely retained as this cancellation's replay identity.
                }
                ProcessHandoffPhase::Prepared => {
                    if entry.process_handoff_digest.as_deref() != request_digest {
                        return Err(handoff_conflict(
                            entry_id,
                            "the prepared process belongs to a different request",
                        ));
                    }
                }
                ProcessHandoffPhase::ResumeCancelled
                    if entry.cancelled_process_handoff_digest.as_deref() == request_digest =>
                {
                    return Ok(());
                }
                _ => {
                    return Err(handoff_conflict(
                        entry_id,
                        "a registered or different process authority cannot be cancelled",
                    ));
                }
            }
            entry.process_handoff_phase = ProcessHandoffPhase::ResumeCancelled;
            entry.cancelled_process_handoff_digest = request_digest.map(str::to_string);
            entry.process_handoff_digest = None;
            entry.prepared_process_handle = None;
            entry.prepared_home_seed = None;
            Ok(())
        })
    }

    /// Atomically consume an exactly named `Registered` generation and clear its old
    /// process.  There is intentionally no idempotent success: a lost resume response is a
    /// stranded pending launch in V1, not permission for a second caller to launch.
    pub fn open_resume_process_handoff(
        &self,
        entry_id: &str,
        expected_generation: u64,
    ) -> Result<Entry, DaemonError> {
        self.modify_handoff(entry_id, "open_resume_process_handoff", |entry| {
            if !matches!(entry.state, EntryState::Started) {
                return Err(handoff_conflict(
                    entry_id,
                    "only a started entry may open a resume handoff",
                ));
            }
            if entry.session_id.as_deref().is_none_or(str::is_empty) {
                return Err(handoff_conflict(
                    entry_id,
                    "only an entry with a committed session may open a resume handoff",
                ));
            }
            let legacy = matches!(
                entry.process_handoff_phase,
                ProcessHandoffPhase::LegacyUnknown
            );
            let attached_origin = matches!(
                entry.expected_process_handoff,
                ExpectedProcessHandoff::NotRequired
            ) && matches!(
                entry.process_handoff_phase,
                ProcessHandoffPhase::InitialClosed
            );
            if !legacy
                && !attached_origin
                && (!matches!(
                    entry.process_handoff_phase,
                    ProcessHandoffPhase::Registered | ProcessHandoffPhase::ResumeCancelled
                ) || entry.process_handoff_generation != expected_generation)
            {
                return Err(handoff_conflict(
                    entry_id,
                    "the resume generation is stale or already consumed",
                ));
            }
            if attached_origin && entry.process_handoff_generation != expected_generation {
                return Err(handoff_conflict(
                    entry_id,
                    "the attached resume generation is stale",
                ));
            }
            if legacy && expected_generation != 0 {
                return Err(handoff_conflict(
                    entry_id,
                    "the legacy resume must begin from generation zero",
                ));
            }
            if legacy
                && matches!(
                    entry.expected_process_handoff,
                    ExpectedProcessHandoff::Required
                )
                && entry.last_process.is_none()
            {
                return Err(handoff_conflict(
                    entry_id,
                    "a legacy required entry has no durable termination handle and cannot resume",
                ));
            }
            entry.process_handoff_generation = entry
                .process_handoff_generation
                .checked_add(1)
                .ok_or_else(|| {
                    DaemonError::Refused(format!("entry {entry_id} process generation overflowed"))
                })?;
            entry.process_handoff_phase = ProcessHandoffPhase::OpenResume;
            entry.expected_process_handoff = ExpectedProcessHandoff::Required;
            entry.process_handoff_digest = None;
            entry.prepared_process_handle = None;
            entry.prepared_home_seed = None;
            entry.cancelled_process_handoff_digest = None;
            // `last_process` remains the active termination authority throughout an issued
            // resume permit.  A failed local relaunch can cancel this generation without
            // leaving a seal with nothing safe to end; only `Registered` replacement swaps
            // it for the new child.
            Ok(())
        })
    }

    /// Read/modify/write a process-authority transition in one redb write transaction.
    /// Keeping these transitions out of generic `update` is what gives a resume its CAS
    /// semantics instead of a best-effort clear followed by an unrelated write.
    fn modify_handoff<F>(&self, entry_id: &str, operation: &str, f: F) -> Result<Entry, DaemonError>
    where
        F: FnOnce(&mut Entry) -> Result<(), DaemonError>,
    {
        let txn = self.db.begin_write().map_err(|e| plain(e, operation))?;
        let stored;
        {
            let mut table = txn.open_table(ENTRIES).map_err(|e| plain(e, operation))?;
            let mut entry: Entry = {
                let found = table.get(entry_id).map_err(|e| plain(e, operation))?;
                match found {
                    Some(bytes) => serde_json::from_slice(bytes.value())?,
                    None => return Err(DaemonError::Missing(format!("entry {entry_id}"))),
                }
            };
            f(&mut entry)?;
            validate_foreign_pending(&entry)?;
            let bytes = serde_json::to_vec(&entry)?;
            table
                .insert(entry_id, bytes.as_slice())
                .map_err(|e| plain(e, operation))?;
            stored = entry;
        }
        txn.commit().map_err(|e| plain(e, operation))?;
        Ok(stored)
    }

    /// `update` without the receipt guard. Receipt writers perform their own validation
    /// before reaching this helper.
    fn modify<F>(&self, entry_id: &str, f: F) -> Result<Entry, DaemonError>
    where
        F: FnOnce(&mut Entry),
    {
        let txn = self.db.begin_write().map_err(|e| plain(e, "update"))?;
        let stored;
        {
            let mut table = txn.open_table(ENTRIES).map_err(|e| plain(e, "update"))?;
            let mut entry: Entry = {
                let found = table.get(entry_id).map_err(|e| plain(e, "update"))?;
                match found {
                    Some(bytes) => serde_json::from_slice(bytes.value())?,
                    None => return Err(DaemonError::Missing(format!("entry {entry_id}"))),
                }
            };
            f(&mut entry);
            if entry.entry_id != entry_id {
                return Err(DaemonError::Refused(format!(
                    "an update to entry {entry_id} renamed it to {}; the id is the key",
                    entry.entry_id
                )));
            }
            validate_foreign_pending(&entry)?;
            let bytes = serde_json::to_vec(&entry)?;
            table
                .insert(entry_id, bytes.as_slice())
                .map_err(|e| plain(e, "update"))?;
            stored = entry;
        }
        txn.commit().map_err(|e| plain(e, "update"))?;
        Ok(stored)
    }

    /// Store a start receipt **after** it verifies against the key this build pins.
    ///
    /// This compatibility helper is limited to legacy receipt-only rows. Bound receipts
    /// must use [`Registry::commit_start`] so the receipt and start state are one durable
    /// transition. `verify_with` compares the document's own key to the pinned one first,
    /// so a receipt a participant signed themselves is refused before any cryptography
    /// runs, and a refusal writes nothing at all.
    ///
    /// Test-only: no shipped caller remains, so a shipped daemon does not compile it.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn store_receipt(
        &self,
        entry_id: &str,
        receipt: &StartReceipt,
        server_public_key_hex: &str,
    ) -> Result<Entry, DaemonError> {
        if receipt.claims.entry_id != entry_id {
            return Err(DaemonError::Refused(format!(
                "the start receipt names entry {}, and this is entry {entry_id}",
                receipt.claims.entry_id
            )));
        }
        receipt.verify_with(server_public_key_hex).map_err(|e| {
            DaemonError::Refused(format!(
                "the start receipt for entry {entry_id} was not stored: {e}"
            ))
        })?;
        let txn = self
            .db
            .begin_write()
            .map_err(|e| plain(e, "store_receipt"))?;
        let stored;
        {
            let mut table = txn
                .open_table(ENTRIES)
                .map_err(|e| plain(e, "store_receipt"))?;
            let mut entry: Entry = {
                let found = table.get(entry_id).map_err(|e| plain(e, "store_receipt"))?;
                match found {
                    Some(bytes) => serde_json::from_slice(bytes.value())?,
                    None => return Err(DaemonError::Missing(format!("entry {entry_id}"))),
                }
            };
            if !matches!(entry.receipt_binding, ReceiptBindingPolicy::Legacy) {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} uses bound receipts, so its receipt and start state \
                     must be committed together"
                )));
            }
            if receipt.claims.run_public_key.is_some()
                || receipt.claims.questionnaire_version.is_some()
            {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} uses legacy receipts, so a bound receipt was not stored"
                )));
            }
            entry.receipt = Some(receipt.clone());
            let bytes = serde_json::to_vec(&entry)?;
            table
                .insert(entry_id, bytes.as_slice())
                .map_err(|e| plain(e, "store_receipt"))?;
            stored = entry;
        }
        txn.commit().map_err(|e| plain(e, "store_receipt"))?;
        Ok(stored)
    }

    /// Atomically commit the only durable transition into [`EntryState::Started`].
    ///
    /// The row is re-read and every row-owned receipt claim is re-derived inside this
    /// write transaction. The artifact and trace paths are compared with the runtime
    /// identity captured before the receipt request. A receipt checked against an earlier
    /// in-memory entry therefore cannot be installed after the row was replaced, started,
    /// sealed, or refused. Every refusal leaves the receipt, session, timestamps, deadline
    /// scalar, and state unchanged.
    #[cfg(feature = "v1-compat")]
    pub fn commit_start(
        &self,
        entry_id: &str,
        receipt: &StartReceipt,
        asserted: &StartReceiptClaims,
        expected_paths: &StartPathIdentity,
        corroborated_session: &str,
        server_public_key_hex: &str,
    ) -> Result<Entry, DaemonError> {
        let txn = self
            .db
            .begin_write()
            .map_err(|e| plain(e, "commit_start"))?;
        let stored;
        {
            let mut table = txn
                .open_table(ENTRIES)
                .map_err(|e| plain(e, "commit_start"))?;
            let mut entry: Entry = {
                let found = table.get(entry_id).map_err(|e| plain(e, "commit_start"))?;
                match found {
                    Some(bytes) => serde_json::from_slice(bytes.value())?,
                    None => return Err(DaemonError::Missing(format!("entry {entry_id}"))),
                }
            };
            if !matches!(entry.state, EntryState::Armed | EntryState::StartSignalSeen) {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} is {:?} in the registry, so its start was not committed",
                    entry.state
                )));
            }
            if entry.receipt.is_some() {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} already has a start receipt, so its start is immutable"
                )));
            }
            if !entry.start_handoff_complete() {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} has not durably received the process its launch owes \
                     before start"
                )));
            }
            if receipt.claims.entry_id != entry_id {
                return Err(DaemonError::Refused(format!(
                    "the start receipt names entry {}, and this is entry {entry_id}",
                    receipt.claims.entry_id
                )));
            }
            if entry.artifact_dir != expected_paths.artifact_dir {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} has a different artifact directory than the runtime \
                     that requested its start receipt"
                )));
            }
            if entry.trace_dir != expected_paths.trace_dir {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} has a different trace directory than the runtime \
                     that requested its start receipt"
                )));
            }

            let started_at = receipt.claims.started_at.clone();
            let deadline_at = receipt.claims.deadline_at.clone();
            let deadline_unix =
                goalcup_protocol::control::parse_z(&deadline_at).ok_or_else(|| {
                    DaemonError::Refused(format!(
                        "the start receipt for entry {entry_id} has no usable deadline instant"
                    ))
                })? as f64;
            let (run_public_key, questionnaire_version) =
                crate::receipt::binding_claims_for_entry(&entry)?;
            let expected = StartReceiptClaims {
                entry_id: entry.entry_id.clone(),
                connector_id: entry.connector_id.clone(),
                workspace: entry.workspace.display().to_string(),
                session_id: corroborated_session.to_string(),
                started_at: started_at.clone(),
                deadline_at: deadline_at.clone(),
                manifest_sha256: entry.manifest_sha256.clone(),
                // Seq 0's genesis link belongs to the receipt request. `entry.chain_head`
                // is a moving mirror of the latest chain link and must not replace it.
                chain_head: asserted.chain_head.clone(),
                connector_dir_sha256: entry.connector_dir_sha256.clone(),
                run_public_key,
                questionnaire_version,
            };
            crate::receipt::accept_receipt(receipt, &expected, server_public_key_hex)?;

            entry.receipt = Some(receipt.clone());
            entry.session_id = Some(corroborated_session.to_string());
            entry.started_at = Some(started_at);
            entry.deadline_at = Some(deadline_at);
            entry.deadline_unix = Some(deadline_unix);
            entry.state = EntryState::Started;
            let bytes = serde_json::to_vec(&entry)?;
            table
                .insert(entry_id, bytes.as_slice())
                .map_err(|e| plain(e, "commit_start"))?;
            stored = entry;
        }
        txn.commit().map_err(|e| plain(e, "commit_start"))?;
        Ok(stored)
    }

    /// One scan, in key order, deserialising each stored document and keeping the ones
    /// the predicate accepts. Every lookup above is this with a different predicate, so
    /// there is one place that knows how an entry is stored.
    fn matching<F>(&self, keep: F) -> Result<Vec<Entry>, DaemonError>
    where
        F: Fn(&Entry) -> bool,
    {
        let txn = self.db.begin_read().map_err(|e| plain(e, "scan"))?;
        let table = txn.open_table(ENTRIES).map_err(|e| plain(e, "scan"))?;
        let mut found = Vec::new();
        for row in table.iter().map_err(|e| plain(e, "scan"))? {
            let (_, bytes) = row.map_err(|e| plain(e, "scan"))?;
            let entry: Entry = serde_json::from_slice(bytes.value())?;
            validate_foreign_pending(&entry)?;
            if keep(&entry) {
                found.push(entry);
            }
        }
        Ok(found)
    }
}

/// A redb failure that happened while touching a known file, so the message carries the
/// path the way every `std::fs` error in this crate does.
fn db_err(path: &Path, what: &str, e: impl std::fmt::Display) -> DaemonError {
    DaemonError::Db(format!("registry {}: {what}: {e}", path.display()))
}

/// A redb failure inside an already-open database, named by the operation that hit it.
/// The path is not repeated: the daemon runs one registry, and the operation is the part
/// the reader does not already know.
fn plain(e: impl std::fmt::Display, op: &str) -> DaemonError {
    DaemonError::Db(format!("registry {op}: {e}"))
}

/// Whether `dir` holds nothing but the registry file — the test for "this directory is
/// the registry's own", and so the test for whether its mode is the registry's to set.
/// An empty directory passes: it is a directory the registry is about to be alone in.
fn holds_only(dir: &Path, file_name: &std::ffi::OsStr) -> Result<bool, DaemonError> {
    let io = |source| DaemonError::Io {
        path: dir.display().to_string(),
        source,
    };
    for row in std::fs::read_dir(dir).map_err(io)? {
        if row.map_err(io)?.file_name() != file_name {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Tighten a path to exactly `mode`. A no-op off unix, where this process cannot express
/// "owner only" in the same terms.
///
/// `pub(crate)` because [`crate::buffer::Buffer`] owns a second owner-only file and needs
/// the same repair on open. One spelling of "owner only" in this crate, not two.
#[cfg(unix)]
pub(crate) fn restrict(path: &Path, mode: u32) -> Result<(), DaemonError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(|source| {
        DaemonError::Io {
            path: path.display().to_string(),
            source,
        }
    })
}

#[cfg(not(unix))]
pub(crate) fn restrict(_path: &Path, _mode: u32) -> Result<(), DaemonError> {
    Ok(())
}

#[cfg(test)]
#[cfg(feature = "v1-compat")]
mod legacy_required_handoff_materialization_tests {
    use super::*;
    use goalcup_protocol::events::HomeSeedRecord;
    use goalcup_protocol::manifest::KillStrategy;

    type RawMutation = Box<dyn Fn(&mut Entry)>;
    type RefusalCase = (&'static str, EntryState, Option<ProcessHandle>, RawMutation);

    fn seeded_handle() -> ProcessHandle {
        ProcessHandle {
            pid: 42,
            pgid: Some(41),
            native_pid: Some(40),
            native_binary: Some("codex".to_string()),
            native_process_start: Some("native-fixture-start".to_string()),
            process_start: Some("fixture-start".to_string()),
            strategy: KillStrategy::ProcessGroup,
            grace_seconds: 7,
            launched_at: "2026-09-10T00:00:00Z".to_string(),
            program: "/usr/local/bin/codex".to_string(),
            argv: vec!["codex".to_string(), "resume".to_string()],
            env_keys: vec!["CODEX_HOME".to_string()],
            home_seed: Some(HomeSeedRecord {
                wire_layout: Default::default(),
                handoff_generation: None,
                handoff_request_digest: None,
                connector: "codex-cli".to_string(),
                from: "{home}/.codex/config.toml".to_string(),
                keys: Vec::new(),
                keys_unchanged: Vec::new(),
                written: Vec::new(),
                files: Vec::new(),
                files_kept: Vec::new(),
                files_missing: Vec::new(),
            }),
        }
    }

    fn legacy_required_entry(
        registry: &Registry,
        root: &Path,
        entry_id: &str,
        state: EntryState,
        last_process: Option<ProcessHandle>,
    ) -> Entry {
        let base = Entry::new(
            entry_id,
            "codex-cli",
            root.join(format!("workspace-{entry_id}")),
            root.join(format!("artifact-{entry_id}")),
            &"ab".repeat(32),
            "0.0.0-test",
            ReceiptBindingPolicy::Legacy,
            ExpectedProcessHandoff::Required,
            RunKeys::from_seeds([1; 32], [2; 32], [3; 16]),
        );
        let mut raw = serde_json::to_value(base).expect("an entry serializes");
        let fields = raw
            .as_object_mut()
            .expect("an entry serializes as an object");
        fields.insert(
            "state".to_string(),
            serde_json::to_value(state).expect("the state serializes"),
        );
        fields.insert(
            "expected_process_handoff".to_string(),
            serde_json::json!("required"),
        );
        fields.remove("process_handoff_generation");
        fields.remove("process_handoff_phase");
        fields.remove("process_handoff_digest");
        fields.remove("prepared_process_handle");
        fields.remove("prepared_home_seed");
        fields.remove("cancelled_process_handoff_digest");
        fields.insert(
            "last_process".to_string(),
            serde_json::to_value(last_process).expect("the process serializes"),
        );
        let entry: Entry = serde_json::from_value(raw).expect("the old row deserializes");
        registry.put(&entry).expect("the raw row is stored");
        entry
    }

    fn stored_bytes(registry: &Registry, entry_id: &str) -> Vec<u8> {
        let txn = registry.db.begin_read().expect("the registry reads");
        let table = txn.open_table(ENTRIES).expect("the entries table opens");
        table
            .get(entry_id)
            .expect("the row reads")
            .expect("the row exists")
            .value()
            .to_vec()
    }

    #[test]
    fn a_legacy_armed_seeded_handle_materializes_once_as_a_stripped_registered_generation_one() {
        let temp = tempfile::tempdir().expect("a temporary registry root");
        let registry =
            Registry::open(&Registry::default_path(temp.path())).expect("the registry opens");
        let seeded = seeded_handle();
        legacy_required_entry(
            &registry,
            temp.path(),
            "armed-seeded",
            EntryState::Armed,
            Some(seeded.clone()),
        );

        let materialized = registry
            .materialize_legacy_required_handoff("armed-seeded")
            .expect("the complete legacy row materializes");
        let expected = ProcessRegistration {
            generation: 1,
            handle: seeded.clone(),
        };
        assert_eq!(
            materialized.process_handoff_phase(),
            ProcessHandoffPhase::Registered
        );
        assert_eq!(materialized.process_handoff_generation(), 1);
        assert_eq!(materialized.last_process, Some(expected.stripped_handle()));
        assert_eq!(
            materialized.process_handoff_digest.as_deref(),
            Some(expected.canonical_handle_digest("armed-seeded").as_str())
        );
        assert!(
            materialized
                .last_process
                .as_ref()
                .and_then(|handle| handle.home_seed.as_ref())
                .is_none(),
            "old seed evidence is not reclassified as a prepared seed or retained on termination authority"
        );
        assert!(materialized.prepared_process_handle.is_none());
        assert!(materialized.prepared_home_seed.is_none());

        let before_second_materialization = stored_bytes(&registry, "armed-seeded");
        let repeated = registry
            .materialize_legacy_required_handoff("armed-seeded")
            .expect("a materialized row is a no-op on restart");
        assert_eq!(repeated, materialized);
        assert_eq!(
            stored_bytes(&registry, "armed-seeded"),
            before_second_materialization,
            "a second materialization serializes no registry mutation"
        );
    }

    #[test]
    fn complete_legacy_rows_materialize_to_their_only_allowed_authority_shape() {
        let temp = tempfile::tempdir().expect("a temporary registry root");
        let registry =
            Registry::open(&Registry::default_path(temp.path())).expect("the registry opens");
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("a live process supplies real instance authority");

        for (entry_id, state, last_process, phase, generation) in [
            (
                "created",
                EntryState::Created,
                None,
                ProcessHandoffPhase::InitialClosed,
                0,
            ),
            (
                "armed",
                EntryState::Armed,
                None,
                ProcessHandoffPhase::OpenInitial,
                1,
            ),
            (
                "signal",
                EntryState::StartSignalSeen,
                None,
                ProcessHandoffPhase::OpenInitial,
                1,
            ),
        ] {
            legacy_required_entry(&registry, temp.path(), entry_id, state, last_process);
            let actual = registry
                .materialize_legacy_required_handoff(entry_id)
                .expect("the documented complete shape materializes");
            assert_eq!(actual.process_handoff_phase(), phase, "{entry_id}");
            assert_eq!(
                actual.process_handoff_generation(),
                generation,
                "{entry_id}"
            );
            if phase == ProcessHandoffPhase::OpenInitial {
                let mut handle = seeded_handle();
                handle.home_seed = None;
                handle.pid = child.id();
                handle.process_start =
                    goalcup_protocol::process::process_start_identity(child.id());
                handle.native_pid = None;
                handle.native_binary = None;
                handle.native_process_start = None;
                assert!(
                    registry
                        .prepare_process_handoff(
                            entry_id,
                            ProcessRegistration {
                                generation: 0,
                                handle: handle.clone(),
                            }
                        )
                        .is_err(),
                    "{entry_id} never accepts the historical generation zero"
                );
                registry
                    .prepare_process_handoff(
                        entry_id,
                        ProcessRegistration {
                            generation: 1,
                            handle,
                        },
                    )
                    .expect("only the materialized generation registers");
                let registered = registry
                    .register_prepared_process_handoff(entry_id, 1)
                    .expect("the generation-one handoff becomes termination authority");
                assert_eq!(
                    registered.process_handoff_phase(),
                    ProcessHandoffPhase::Registered,
                    "{entry_id}"
                );
            }
        }
        let _ = child.kill();
        let _ = child.wait();

        for (entry_id, state) in [
            ("armed-process", EntryState::Armed),
            ("signal-process", EntryState::StartSignalSeen),
        ] {
            let process = seeded_handle();
            legacy_required_entry(
                &registry,
                temp.path(),
                entry_id,
                state,
                Some(process.clone()),
            );
            let actual = registry
                .materialize_legacy_required_handoff(entry_id)
                .expect("the documented preserved-process shape materializes");
            let expected = ProcessRegistration {
                generation: 1,
                handle: process,
            };
            assert_eq!(
                actual.process_handoff_phase(),
                ProcessHandoffPhase::Registered,
                "{entry_id}"
            );
            assert_eq!(actual.process_handoff_generation(), 1, "{entry_id}");
            assert_eq!(
                actual.last_process,
                Some(expected.stripped_handle()),
                "{entry_id}"
            );
            assert_eq!(
                actual.process_handoff_digest.as_deref(),
                Some(expected.canonical_handle_digest(entry_id).as_str()),
                "{entry_id}"
            );
        }
    }

    #[test]
    fn partial_or_impossible_legacy_required_rows_refuse_without_rewriting_the_row() {
        let temp = tempfile::tempdir().expect("a temporary registry root");
        let registry =
            Registry::open(&Registry::default_path(temp.path())).expect("the registry opens");

        let mut cases: Vec<RefusalCase> = vec![
            (
                "nonzero-generation",
                EntryState::Armed,
                None,
                Box::new(|entry| entry.process_handoff_generation = 1),
            ),
            (
                "digest",
                EntryState::Armed,
                None,
                Box::new(|entry| entry.process_handoff_digest = Some("old-digest".to_string())),
            ),
            (
                "prepared-handle",
                EntryState::Armed,
                None,
                Box::new(|entry| entry.prepared_process_handle = Some(seeded_handle())),
            ),
            (
                "prepared-seed",
                EntryState::Armed,
                None,
                Box::new(|entry| {
                    entry.prepared_home_seed = seeded_handle().home_seed;
                }),
            ),
            (
                "cancelled-digest",
                EntryState::Armed,
                None,
                Box::new(|entry| {
                    entry.cancelled_process_handoff_digest = Some("cancelled".to_string());
                }),
            ),
            (
                "armed-with-started-at",
                EntryState::Armed,
                None,
                Box::new(|entry| {
                    entry.started_at = Some("2026-09-10T00:00:00Z".to_string());
                }),
            ),
            (
                "started-without-process",
                EntryState::Started,
                None,
                Box::new(|_| {}),
            ),
            (
                "created-with-process",
                EntryState::Created,
                Some(seeded_handle()),
                Box::new(|_| {}),
            ),
        ];

        for (entry_id, state, last_process, mutate) in cases.drain(..) {
            legacy_required_entry(&registry, temp.path(), entry_id, state, last_process);
            registry
                .modify_handoff(entry_id, "plant_malformed_legacy_handoff", |entry| {
                    mutate(entry);
                    Ok(())
                })
                .expect("the malformed raw row is stored");
            let before = stored_bytes(&registry, entry_id);

            assert!(
                registry
                    .materialize_legacy_required_handoff(entry_id)
                    .is_err(),
                "{entry_id} is an incomplete or impossible legacy shape"
            );
            assert_eq!(
                stored_bytes(&registry, entry_id),
                before,
                "{entry_id} refuses without changing its persisted bytes"
            );
            assert_eq!(
                serde_json::to_vec(
                    &registry
                        .get(entry_id)
                        .expect("the registry reads")
                        .expect("the refused row remains")
                )
                .expect("the refused row serializes"),
                before,
                "{entry_id} remains byte-identical after deserialization too"
            );
        }
    }
}

#[cfg(test)]
#[cfg(feature = "v1-compat")]
mod workspace_finalization_admission_tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    #[test]
    fn source_date_is_persisted_and_survives_generic_updates() {
        let temp = tempfile::tempdir().unwrap();
        let registry = Registry::open(&Registry::default_path(temp.path())).unwrap();
        let mut entry = started_entry(temp.path(), "source-date");
        entry
            .bind_source_date(["2026".into(), "09".into(), "14".into()])
            .unwrap();
        registry.put(&entry).unwrap();
        registry
            .update(&entry.entry_id, |row| row.armed_at = Some(9.0))
            .unwrap();
        assert_eq!(
            registry
                .get(&entry.entry_id)
                .unwrap()
                .unwrap()
                .source_date(),
            Some(&["2026".to_string(), "09".to_string(), "14".to_string()]),
        );
    }

    fn policy() -> ReceiptBindingPolicy {
        ReceiptBindingPolicy::WorkspaceSubmission {
            questionnaire_version: "goal-cup-2026-v1".to_string(),
            submission_profile: SubmissionProfileV1::WorkspaceSnapshotV1,
            limit_profile: LimitProfileV1 {
                id: "goal-cup-v1-local".to_string(),
            },
            effective_limits: EffectiveSubmissionLimitsV1 {
                object_count: 4096,
                total_bytes: 1_073_741_824,
                pre_cutoff_drain: 5000,
                process_walk: 64,
            },
            capture_limits: CaptureResourceLimitsV1 {
                version: 1,
                namespace_nodes: 8192,
                inventory_bytes: 8_388_608,
                manifest_bytes: 8_388_608,
            },
        }
    }

    #[cfg(feature = "v1-compat")]
    fn started_entry(root: &Path, entry_id: &str) -> Entry {
        let mut entry = Entry::new(
            entry_id,
            "codex-cli",
            root.join(format!("workspace-{entry_id}")),
            root.join(format!("artifact-{entry_id}")),
            &"ab".repeat(32),
            "0.0.0-test",
            policy(),
            ExpectedProcessHandoff::NotRequired,
            RunKeys::from_seeds([1; 32], [2; 32], [3; 16]),
        );
        let started_at = "2026-09-14T00:00:00Z";
        let deadline_at = "2026-09-14T00:00:02Z";
        entry.state = EntryState::Started;
        entry.session_id = Some("session-1".to_string());
        entry.started_at = Some(started_at.to_string());
        entry.deadline_at = Some(deadline_at.to_string());
        entry
            .bind_source_date(["2026".into(), "09".into(), "14".into()])
            .unwrap();
        entry.deadline_unix = goalcup_protocol::control::parse_z(deadline_at).map(|v| v as f64);
        let claims = StartReceiptClaims {
            entry_id: entry_id.to_string(),
            connector_id: entry.connector_id.clone(),
            workspace: entry.workspace.display().to_string(),
            session_id: "session-1".to_string(),
            started_at: started_at.to_string(),
            deadline_at: deadline_at.to_string(),
            manifest_sha256: entry.manifest_sha256.clone(),
            chain_head: entry.chain_head.clone(),
            connector_dir_sha256: None,
            run_public_key: Some(entry.keys.public_key_hex().unwrap()),
            questionnaire_version: Some("goal-cup-2026-v1".to_string()),
        };
        entry.receipt =
            Some(StartReceipt::sign(claims, &SigningKey::from_bytes(&[9; 32])).unwrap());
        entry
    }

    #[cfg(feature = "v1-compat")]
    fn intent(entry: &Entry) -> WorkspaceSealingIntentV1 {
        let (submission_profile, limit_profile, limits, capture_limits) =
            match entry.receipt_binding() {
                ReceiptBindingPolicy::WorkspaceSubmission {
                    submission_profile,
                    limit_profile,
                    effective_limits,
                    capture_limits,
                    ..
                } => (
                    *submission_profile,
                    limit_profile.clone(),
                    effective_limits.clone(),
                    capture_limits.clone(),
                ),
                _ => panic!("fixture must carry the V4 policy"),
            };
        WorkspaceSealingIntentV1 {
            version: WorkspaceIntentVersionV1::WorkspaceSubmissionV1,
            entry_id: entry.entry_id.clone(),
            generation: entry.process_handoff_generation(),
            reason: FinishReasonV1::Deadline,
            receipt_sha256: entry.receipt().unwrap().canonical_digest().unwrap(),
            cutoff_at: Rfc3339Utc::try_from(entry.deadline_at.clone().unwrap()).unwrap(),
            trace_root_identity: "trace-root-v1:4:2".to_string(),
            source_date: entry.source_date().unwrap().clone(),
            source_root: WorkspaceSourceBindingV1::Available {
                journal: SourceRootJournalV1 {
                    identity: "source-root-v1:17:9".to_string(),
                },
            },
            submission_profile,
            limits,
            limit_profile,
            capture_limits,
            excluded: vec![".goalcup".to_string()],
            phase: FinalizationPhaseV1::AdmissionClosed,
            admission_watermarks: AdmissionWatermarksV1 {
                tailers: Vec::new(),
                receiver_sources: BTreeMap::new(),
                queued_receiver_posts: 0,
                admitted_chain_predecessor: None,
            },
            expected_cutoff_predecessor: None,
            drain: None,
            cutoff: None,
            accepted_handoff: None,
            termination: None,
            termination_report: None,
            submission: None,
            finalization_at: None,
            expected_finalization_predecessor: None,
            finalization: None,
            terminal_status_at: None,
            status_mirror_complete: false,
        }
    }

    #[test]
    fn verified_deadline_wins_at_the_boundary_and_manual_uses_the_observed_instant() {
        let deadline = Rfc3339Utc::try_from("2026-09-14T00:00:02Z".to_string()).unwrap();
        let before = Rfc3339Utc::try_from("2026-09-14T00:00:01Z".to_string()).unwrap();
        let at = deadline.clone();
        let after = Rfc3339Utc::try_from("2026-09-14T00:00:03Z".to_string()).unwrap();

        assert_eq!(
            resolve_finish_boundary(FinishReasonV1::Manual, &before, &deadline, false).unwrap(),
            (FinishReasonV1::Manual, before.clone()),
        );
        assert_eq!(
            resolve_finish_boundary(FinishReasonV1::Manual, &at, &deadline, false).unwrap(),
            (FinishReasonV1::Deadline, deadline.clone()),
        );
        assert_eq!(
            resolve_finish_boundary(FinishReasonV1::Manual, &after, &deadline, false).unwrap(),
            (FinishReasonV1::Deadline, deadline.clone()),
        );
        assert_eq!(
            resolve_finish_boundary(FinishReasonV1::Deadline, &after, &deadline, false).unwrap(),
            (FinishReasonV1::Deadline, deadline.clone()),
        );
        assert!(
            resolve_finish_boundary(
                FinishReasonV1::Deadline,
                &Rfc3339Utc::try_from("2026-09-14T00:00:01Z".to_string()).unwrap(),
                &Rfc3339Utc::try_from("2026-09-14T00:00:02Z".to_string()).unwrap(),
                false,
            )
            .is_err(),
            "a participant-side early deadline request cannot shorten organizer time",
        );
        assert_eq!(
            resolve_finish_boundary(FinishReasonV1::Manual, &before, &deadline, true).unwrap(),
            (FinishReasonV1::Deadline, deadline),
            "a durable deadline observation survives wall-clock rollback",
        );
    }

    #[test]
    fn admission_rejects_a_reason_or_cutoff_not_derived_from_the_receipt_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = Registry::open(&Registry::default_path(tmp.path())).unwrap();

        let manual = started_entry(tmp.path(), "manual");
        store_fixture(&registry, &manual);
        let mut manual_intent = intent(&manual);
        manual_intent.reason = FinishReasonV1::Manual;
        manual_intent.cutoff_at = Rfc3339Utc::try_from("2026-09-14T00:00:01Z".to_string()).unwrap();
        registry
            .begin_workspace_finalization("manual", |_| Ok(manual_intent))
            .expect("an early manual instant is accepted");

        let forged = started_entry(tmp.path(), "forged");
        store_fixture(&registry, &forged);
        let mut forged_intent = intent(&forged);
        forged_intent.reason = FinishReasonV1::Manual;
        registry
            .begin_workspace_finalization("forged", |_| Ok(forged_intent))
            .expect_err("manual cannot remain the reason at the signed deadline");

        let early_deadline = started_entry(tmp.path(), "early-deadline");
        store_fixture(&registry, &early_deadline);
        let mut early_deadline_intent = intent(&early_deadline);
        early_deadline_intent.cutoff_at =
            Rfc3339Utc::try_from("2026-09-14T00:00:01Z".to_string()).unwrap();
        registry
            .begin_workspace_finalization("early-deadline", |_| Ok(early_deadline_intent))
            .expect_err("deadline finalization must latch the signed deadline exactly");
    }

    fn store_fixture(registry: &Registry, entry: &Entry) {
        registry.put(entry).unwrap();
        registry
            .replace_raw_json_for_test(&entry.entry_id, &serde_json::to_vec(entry).unwrap())
            .unwrap();
    }

    #[test]
    fn admission_cas_latches_only_the_started_rows_exact_persisted_authority() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = Registry::open(&Registry::default_path(tmp.path())).unwrap();

        let mut unstarted = started_entry(tmp.path(), "unstarted");
        unstarted.state = EntryState::Armed;
        store_fixture(&registry, &unstarted);
        registry
            .begin_workspace_finalization("unstarted", |row| Ok(intent(row)))
            .expect_err("pre-start abandonment cannot become qualifying V4 finalization");

        let mut legacy = started_entry(tmp.path(), "legacy");
        legacy.receipt_binding = ReceiptBindingPolicy::Legacy;
        store_fixture(&registry, &legacy);
        registry
            .begin_workspace_finalization("legacy", |_| {
                Ok(intent(&started_entry(tmp.path(), "legacy")))
            })
            .expect_err("a legacy row cannot be upgraded by the finalizer");

        let exact = started_entry(tmp.path(), "exact");
        store_fixture(&registry, &exact);
        let mut wrong = intent(&exact);
        wrong.limits.process_walk += 1;
        registry
            .begin_workspace_finalization("exact", |_| Ok(wrong))
            .expect_err("ambient limits cannot replace the entry's latched V4 profile");
        let preserved = registry.get("exact").unwrap().unwrap();
        assert_eq!(preserved.state, EntryState::Started);
        assert!(preserved.sealing_intent().is_none());

        let committed = registry
            .begin_workspace_finalization("exact", |row| Ok(intent(row)))
            .expect("the exact started authority closes admission once");
        assert_eq!(committed.state, EntryState::Sealing);
        assert!(matches!(
            committed.sealing_intent(),
            Some(SealingIntent::Workspace(_))
        ));
        assert_eq!(
            registry
                .begin_workspace_finalization("exact", |_| panic!(
                    "a retry must join before rebuilding facts"
                ))
                .unwrap(),
            committed,
        );
        registry
            .restore_unfinished_sealing("exact")
            .expect_err("workspace admission closure is irreversible");
    }

    #[test]
    fn admission_cas_serializes_manual_and_deadline_races_at_the_exact_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let registry =
            std::sync::Arc::new(Registry::open(&Registry::default_path(tmp.path())).unwrap());
        let deadline = Rfc3339Utc::try_from("2026-09-14T00:00:02Z".to_string()).unwrap();
        let before = Rfc3339Utc::try_from("2026-09-14T00:00:01Z".to_string()).unwrap();
        let cases = [
            (
                "manual-manual",
                FinishReasonV1::Manual,
                FinishReasonV1::Manual,
                before,
                FinishReasonV1::Manual,
            ),
            (
                "deadline-deadline",
                FinishReasonV1::Deadline,
                FinishReasonV1::Deadline,
                deadline.clone(),
                FinishReasonV1::Deadline,
            ),
            (
                "manual-deadline",
                FinishReasonV1::Manual,
                FinishReasonV1::Deadline,
                deadline.clone(),
                FinishReasonV1::Deadline,
            ),
        ];

        for (entry_id, left_reason, right_reason, observed, expected_reason) in cases {
            let entry = started_entry(tmp.path(), entry_id);
            store_fixture(&registry, &entry);
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
            let run = |requested: FinishReasonV1| {
                let registry = std::sync::Arc::clone(&registry);
                let barrier = std::sync::Arc::clone(&barrier);
                let observed = observed.clone();
                let deadline = deadline.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    registry.begin_workspace_finalization(entry_id, |row| {
                        let mut candidate = intent(row);
                        let (reason, cutoff_at) = resolve_finish_boundary(
                            requested,
                            &observed,
                            &deadline,
                            row.deadline_was_observed(),
                        )?;
                        candidate.reason = reason;
                        candidate.cutoff_at = cutoff_at;
                        Ok(candidate)
                    })
                })
            };
            let left = run(left_reason);
            let right = run(right_reason);
            barrier.wait();
            let left = left.join().unwrap().unwrap();
            let right = right.join().unwrap().unwrap();
            assert_eq!(left, right, "both callers join one durable winner");
            let SealingIntent::Workspace(winner) = left.sealing_intent().unwrap() else {
                panic!("race installed the legacy arm")
            };
            assert_eq!(winner.reason, expected_reason);
            assert_eq!(
                winner.cutoff_at,
                if expected_reason == FinishReasonV1::Deadline {
                    deadline.clone()
                } else {
                    observed
                }
            );
            assert_eq!(winner.generation, entry.process_handoff_generation());
        }
    }
}

#[cfg(test)]
mod foreign_pending_registry_tests {
    use super::*;

    fn pending(path: &str) -> PendingForeignSighting {
        PendingForeignSighting {
            family: "codex".to_string(),
            harness: "codex-cli".to_string(),
            session_id: "session-1".to_string(),
            store_path: path.to_string(),
            first_seen: "2026-09-04T00:00:00Z".to_string(),
            scan_root: ForeignScanRoot::OwnConfigDir,
        }
    }

    fn entry_with_pending(temp: &tempfile::TempDir, row: PendingForeignSighting) -> Entry {
        let mut entry = Entry::new(
            "pending",
            "codex-cli",
            temp.path().join("workspace"),
            temp.path().join("artifact"),
            &"ab".repeat(32),
            "0.0.0-test",
            ReceiptBindingPolicy::Legacy,
            ExpectedProcessHandoff::NotRequired,
            RunKeys::from_seeds([1; 32], [2; 32], [3; 16]),
        );
        entry.foreign_pending = vec![row];
        entry
    }

    #[test]
    fn pending_projection_defaults_empty_omits_empty_and_enforces_bound() {
        let temp = tempfile::tempdir().unwrap();
        let registry = Registry::open(&Registry::default_path(temp.path())).unwrap();
        let entry = Entry::new(
            "pending",
            "codex-cli",
            temp.path().join("workspace"),
            temp.path().join("artifact"),
            &"ab".repeat(32),
            "0.0.0-test",
            ReceiptBindingPolicy::Legacy,
            ExpectedProcessHandoff::NotRequired,
            RunKeys::from_seeds([1; 32], [2; 32], [3; 16]),
        );
        let mut old_row = serde_json::to_value(&entry).unwrap();
        assert!(
            old_row
                .as_object()
                .unwrap()
                .get("foreign_pending")
                .is_none(),
            "empty pending state is omitted"
        );
        let decoded: Entry = serde_json::from_value(old_row.clone()).unwrap();
        assert!(decoded.foreign_pending.is_empty());

        registry.put(&entry).unwrap();
        let mut over = entry;
        over.foreign_pending = (0..=MAX_FOREIGN_PENDING_SIGHTINGS)
            .map(|index| PendingForeignSighting {
                family: "cli".to_string(),
                harness: "codex-cli".to_string(),
                session_id: format!("session-{index}"),
                store_path: "/tmp/store.jsonl".to_string(),
                first_seen: "2026-09-04T00:00:00Z".to_string(),
                scan_root: ForeignScanRoot::OwnConfigDir,
            })
            .collect();
        assert!(
            registry.put(&over).is_err(),
            "the registry refuses rows over the strict pending bound"
        );

        old_row["foreign_pending"] = serde_json::json!([]);
        assert!(serde_json::from_value::<Entry>(old_row).is_ok());
    }

    #[test]
    fn pending_rows_validate_provenance_identity_path_timestamp_and_duplicates() {
        let cases = [
            ("home provenance", {
                let mut row = pending("/tmp/store.jsonl");
                row.scan_root = ForeignScanRoot::Home;
                row
            }),
            ("mismatched harness", {
                let mut row = pending("/tmp/store.jsonl");
                row.harness = "other-harness".to_string();
                row
            }),
            ("relative path", pending("relative/store.jsonl")),
            ("nul path", pending("/tmp/store\0.jsonl")),
            ("invalid timestamp", {
                let mut row = pending("/tmp/store.jsonl");
                row.first_seen = "not-a-timestamp".to_string();
                row
            }),
            ("oversized family", {
                let mut row = pending("/tmp/store.jsonl");
                row.family = "x".repeat(MAX_FOREIGN_PENDING_FIELD_BYTES + 1);
                row
            }),
        ];
        for (name, row) in cases {
            let temp = tempfile::tempdir().unwrap();
            let registry = Registry::open(&Registry::default_path(temp.path())).unwrap();
            let entry = entry_with_pending(&temp, row);
            assert!(
                registry.put(&entry).is_err(),
                "malformed pending row: {name}"
            );
        }

        let temp = tempfile::tempdir().unwrap();
        let registry = Registry::open(&Registry::default_path(temp.path())).unwrap();
        let row = pending("/tmp/store.jsonl");
        let mut entry = entry_with_pending(&temp, row.clone());
        entry.foreign_pending.push(row);
        assert!(
            registry.put(&entry).is_err(),
            "duplicate durable identity rows are refused rather than collapsed"
        );
    }

    #[test]
    fn pending_rows_missing_scan_root_are_rejected_but_legacy_empty_rows_decode() {
        let temp = tempfile::tempdir().unwrap();
        let registry = Registry::open(&Registry::default_path(temp.path())).unwrap();
        let entry = Entry::new(
            "pending",
            "codex-cli",
            temp.path().join("workspace"),
            temp.path().join("artifact"),
            &"ab".repeat(32),
            "0.0.0-test",
            ReceiptBindingPolicy::Legacy,
            ExpectedProcessHandoff::NotRequired,
            RunKeys::from_seeds([1; 32], [2; 32], [3; 16]),
        );
        let mut raw = serde_json::to_value(&entry).unwrap();
        raw["foreign_pending"] = serde_json::json!([{
            "family": "codex",
            "harness": "codex-cli",
            "session_id": "legacy",
            "store_path": "/tmp/legacy.jsonl",
            "first_seen": "2026-09-04T00:00:00Z"
        }]);
        let decoded: Entry = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(decoded.foreign_pending[0].scan_root, ForeignScanRoot::Home);
        registry
            .replace_raw_json_for_test("pending", &serde_json::to_vec(&raw).unwrap())
            .unwrap();
        assert!(
            registry.get("pending").is_err(),
            "a legacy pending row without provenance fails closed"
        );

        raw["foreign_pending"] = serde_json::json!([]);
        registry
            .replace_raw_json_for_test("pending", &serde_json::to_vec(&raw).unwrap())
            .unwrap();
        assert!(registry.get("pending").unwrap().is_some());
    }
}
