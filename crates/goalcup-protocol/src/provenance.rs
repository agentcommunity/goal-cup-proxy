//! Versioned provenance documents a v5 chain can carry: the cutoff, the signed-mode
//! finalization and the termination outcome, with the primitives they are built from.
//!
//! These types validate and bind declarations. They do not inspect a repository, read
//! captured objects or run a publisher.
//!
//! **What is left of the Git vocabulary.** `FinalizationV1::CaptureUnavailable` names the
//! repository selection that did not happen (`RepositoryNonSelectionV1`,
//! `SelectedRepositoryV1`, `SnapshotFailedV1`) and the publisher that was not run. The frozen v5
//! corpus case `signed-receipt-ack-finalization` carries it, so its types stay exactly as they
//! were, every enum variant included: dropping one would change how a hostile line reads. The
//! repository observations, snapshot, history, commit, publication and recovery documents were
//! never produced by any writer and left with D54.

use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Deserializer, Serialize};
use std::cmp::Ordering;

const MAX_PORTABLE_PATH_BYTES_V1: usize = 4_096;

const MAX_REF_BYTES_V1: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProvenanceError {
    #[error("invalid lowercase SHA-256 digest")]
    InvalidSha256,
    #[error("invalid lowercase Git object id")]
    InvalidGitObjectId,
    #[error("Git object id width does not match the declared object format")]
    GitObjectFormatMismatch,
    #[error("invalid portable relative path")]
    InvalidPortablePath,
    #[error("invalid refs/heads publication target")]
    InvalidHeadsRef,
    #[error("invalid normalized GitHub remote")]
    InvalidRemote,
    #[error("invalid canonical UTC timestamp")]
    InvalidTimestamp,
    #[error("invalid {field}: {reason}")]
    InvalidField {
        field: &'static str,
        reason: &'static str,
    },
    #[error("public key is not a usable Ed25519 key")]
    PublicKeyMalformed,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(hex::encode(bytes))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for Sha256Digest {
    type Error = ProvenanceError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if is_lower_hex(value, 64) {
            Ok(Self(value.to_string()))
        } else {
            Err(ProvenanceError::InvalidSha256)
        }
    }
}

impl TryFrom<String> for Sha256Digest {
    type Error = ProvenanceError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitObjectFormat {
    Sha1,
    Sha256,
}

impl GitObjectFormat {
    fn hex_len(self) -> usize {
        match self {
            Self::Sha1 => 40,
            Self::Sha256 => 64,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct GitObjectId(String);

impl GitObjectId {
    pub fn validate_for(&self, format: GitObjectFormat) -> Result<(), ProvenanceError> {
        if self.0.len() == format.hex_len() {
            Ok(())
        } else {
            Err(ProvenanceError::GitObjectFormatMismatch)
        }
    }
}

impl TryFrom<&str> for GitObjectId {
    type Error = ProvenanceError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if (value.len() == 40 || value.len() == 64)
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            Ok(Self(value.to_string()))
        } else {
            Err(ProvenanceError::InvalidGitObjectId)
        }
    }
}

impl TryFrom<String> for GitObjectId {
    type Error = ProvenanceError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl<'de> Deserialize<'de> for GitObjectId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct PortableRelativePathOrRoot(String);

impl TryFrom<&str> for PortableRelativePathOrRoot {
    type Error = ProvenanceError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        validate_portable_path(value, true)?;
        Ok(Self(value.to_string()))
    }
}

impl TryFrom<String> for PortableRelativePathOrRoot {
    type Error = ProvenanceError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl<'de> Deserialize<'de> for PortableRelativePathOrRoot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ValidatedHeadsRef(String);

impl TryFrom<&str> for ValidatedHeadsRef {
    type Error = ProvenanceError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        validate_heads_ref(value)?;
        Ok(Self(value.to_string()))
    }
}

impl TryFrom<String> for ValidatedHeadsRef {
    type Error = ProvenanceError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl<'de> Deserialize<'de> for ValidatedHeadsRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Rfc3339Utc(String);

impl Rfc3339Utc {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn chronological_cmp(&self, other: &Self) -> Ordering {
        let whole = self.0[..19].cmp(&other.0[..19]);
        if whole != Ordering::Equal {
            return whole;
        }
        fractional_digits(&self.0).cmp(&fractional_digits(&other.0))
    }
}

impl TryFrom<&str> for Rfc3339Utc {
    type Error = ProvenanceError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        validate_rfc3339_utc(value)?;
        Ok(Self(value.to_string()))
    }
}

impl TryFrom<String> for Rfc3339Utc {
    type Error = ProvenanceError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl<'de> Deserialize<'de> for Rfc3339Utc {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(serde::de::Error::custom)
    }
}

pub(crate) type ChainHash = Sha256Digest;

/// Validate the shared questionnaire binding grammar without changing its receipt wire type.
pub fn validate_questionnaire_version(value: &str) -> Result<(), ProvenanceError> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 128
        || !bytes[0].is_ascii_alphanumeric()
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(invalid(
            "questionnaire_version",
            "must use the bounded version-token grammar",
        ));
    }
    Ok(())
}

/// Return the canonical lowercase spelling of a usable, non-weak Ed25519 run key.
pub fn canonical_run_public_key(value: &str) -> Result<String, ProvenanceError> {
    if value.len() != 64 {
        return Err(ProvenanceError::PublicKeyMalformed);
    }
    let bytes: [u8; 32] = hex::decode(value)
        .map_err(|_| ProvenanceError::PublicKeyMalformed)?
        .try_into()
        .map_err(|_| ProvenanceError::PublicKeyMalformed)?;
    let key = VerifyingKey::from_bytes(&bytes).map_err(|_| ProvenanceError::PublicKeyMalformed)?;
    if key.is_weak() {
        return Err(ProvenanceError::PublicKeyMalformed);
    }
    Ok(hex::encode(bytes))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitRemoteTransportV1 {
    Https,
    Ssh,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedGitHubRemote {
    pub host: String,
    pub owner: String,
    pub repository: String,
    pub transport: GitRemoteTransportV1,
}

impl NormalizedGitHubRemote {
    pub fn validate(&self) -> Result<(), ProvenanceError> {
        if self.host.is_empty()
            || self.host.len() > 253
            || self
                .host
                .bytes()
                .any(|b| !(b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-'))
            || self.host.starts_with(['.', '-'])
            || self.host.ends_with(['.', '-'])
            || self.host.contains("..")
            || self
                .host
                .split('.')
                .any(|label| label.is_empty() || label.starts_with('-') || label.ends_with('-'))
            || !valid_github_component(&self.owner)
            || !valid_github_component(&self.repository)
            || self.repository.to_ascii_lowercase().ends_with(".git")
        {
            return Err(ProvenanceError::InvalidRemote);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RepositoryNonSelectionV1 {
    None {
        reason: NoRepositoryReasonV1,
    },
    Ambiguous {
        candidate_count: u32,
    },
    Unsupported {
        reason: RepositoryUnsupportedReasonV1,
    },
}

impl RepositoryNonSelectionV1 {
    pub(crate) fn validate(&self) -> Result<(), ProvenanceError> {
        if let Self::Ambiguous { candidate_count } = self
            && *candidate_count < 2
        {
            return Err(invalid(
                "candidate_count",
                "an ambiguous selection requires at least two candidates",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoRepositoryReasonV1 {
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryUnsupportedReasonV1 {
    EligibleOutputOutsideSelectedRepository,
    BareRepository,
    LinkedWorktree,
    NestedRepository,
    Gitlink,
    SplitIndex,
    SparseIndex,
    AlternateObjectDirectory,
    UnsupportedRepositoryFormat,
    UnsupportedObjectFormat,
    UnsupportedIndex,
    UnsupportedLaunchEnvironment,
    UnsupportedConfigAuthority,
    DetachedHead,
    UnsafeRepositoryPath,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedRepositoryV1 {
    pub workspace_relative_root: PortableRelativePathOrRoot,
    pub remote: NormalizedGitHubRemote,
    pub target_ref: ValidatedHeadsRef,
    pub object_format: GitObjectFormat,
    pub pre_seal_head: PreSealHeadV1,
    pub selection_basis_chain_hash: ChainHash,
}

impl SelectedRepositoryV1 {
    pub(crate) fn validate(&self) -> Result<(), ProvenanceError> {
        self.remote.validate()?;
        match &self.pre_seal_head {
            PreSealHeadV1::Unborn => {}
            PreSealHeadV1::Existing { commit, tree } => {
                commit.validate_for(self.object_format)?;
                tree.validate_for(self.object_format)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PreSealHeadV1 {
    Unborn,
    Existing {
        commit: GitObjectId,
        tree: GitObjectId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotPhaseV1 {
    Eligibility,
    Enumerate,
    Copy,
    Protect,
    Verify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotFailureReasonV1 {
    UnsupportedAuthority,
    IncompleteAuthority,
    UnsupportedEntryKind,
    UnsupportedPath,
    BoundExceeded,
    RaceDetected,
    Io,
    IntegrityMismatch,
    ProtectionFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotFailedV1 {
    pub phase: SnapshotPhaseV1,
    pub reason: SnapshotFailureReasonV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotNotAttemptedV1 {
    pub selection: RepositoryNonSelectionV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreCutoffDrainFailureV1 {
    AdmissionStateUnavailable,
    WriterUnavailable,
    WriterWatermarkAmbiguous,
    ChainSyncFailed,
    RestartStateAmbiguous,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PreCutoffDrainOutcomeV1 {
    Complete,
    Degraded { reason: PreCutoffDrainFailureV1 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CutoffReachedV1 {
    pub receipt_sha256: Sha256Digest,
    pub deadline_at: Rfc3339Utc,
    pub pre_cutoff_drain: PreCutoffDrainOutcomeV1,
}

impl CutoffReachedV1 {
    pub(crate) fn validate(&self) -> Result<(), ProvenanceError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminationMethodV1 {
    GracefulRequest,
    ProcessSignal,
    JobObjectClose,
    AlreadyExited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminationFailureV1 {
    ProcessAuthorityUnavailable,
    RequestFailed,
    ConfirmationTimedOut,
    ProcessStillRunning,
    UnsupportedPlatform,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminationOutcomeV1 {
    AlreadyStopped {
        checked_at: Rfc3339Utc,
    },
    Confirmed {
        requested_at: Rfc3339Utc,
        confirmed_at: Rfc3339Utc,
        method: TerminationMethodV1,
    },
    Unresolved {
        requested_at: Rfc3339Utc,
        stopped_waiting_at: Rfc3339Utc,
        reason: TerminationFailureV1,
    },
}

impl TerminationOutcomeV1 {
    /// Check the outcome's own time ordering: a stop is not confirmed, and a wait does not
    /// end, before it was requested. Public so the v5 local terminal writer and readers apply
    /// exactly the rule finalizations already use; the semantics are unchanged.
    pub(crate) fn validate(&self) -> Result<(), ProvenanceError> {
        match self {
            Self::AlreadyStopped { .. } => Ok(()),
            Self::Confirmed {
                requested_at,
                confirmed_at,
                ..
            } => require_time_order(requested_at, confirmed_at, "termination"),
            Self::Unresolved {
                requested_at,
                stopped_waiting_at,
                ..
            } => require_time_order(requested_at, stopped_waiting_at, "termination wait"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum CaptureUnavailableV1 {
    NotAttempted(SnapshotNotAttemptedV1),
    Failed {
        selected: Box<SelectedRepositoryV1>,
        failure: SnapshotFailedV1,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublisherNotAttemptedV1 {
    pub reason: PublisherNotAttemptedReasonV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublisherNotAttemptedReasonV1 {
    SelectionFailed,
    EligibilityFailed,
    SnapshotFailed,
    ProjectionFailed,
    RequiredHistoryFailed,
    CommitConstructionFailed,
    CredentialUnavailable,
    PublisherSetupFailed,
}

/// The signed-mode terminal bound to the preceding cutoff. One arm: the frozen corpus case
/// `signed-receipt-ack-finalization` carries it. The `captured` and `workspace_submission`
/// arms left with D54; no v5 writer ever produced either.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum FinalizationV1 {
    CaptureUnavailable {
        receipt_sha256: Sha256Digest,
        termination: TerminationOutcomeV1,
        capture: CaptureUnavailableV1,
        publisher: PublisherNotAttemptedV1,
    },
}

impl FinalizationV1 {
    pub(crate) fn receipt_sha256(&self) -> &Sha256Digest {
        match self {
            Self::CaptureUnavailable { receipt_sha256, .. } => receipt_sha256,
        }
    }

    /// The termination outcome this finalization records.
    pub fn termination(&self) -> &TerminationOutcomeV1 {
        match self {
            Self::CaptureUnavailable { termination, .. } => termination,
        }
    }

    pub fn validate(&self) -> Result<(), ProvenanceError> {
        match self {
            Self::CaptureUnavailable {
                termination,
                capture,
                publisher,
                ..
            } => {
                termination.validate()?;
                let expected = match capture {
                    CaptureUnavailableV1::NotAttempted(not_attempted) => {
                        not_attempted.selection.validate()?;
                        PublisherNotAttemptedReasonV1::SelectionFailed
                    }
                    CaptureUnavailableV1::Failed { selected, failure } => {
                        selected.validate()?;
                        if failure.phase == SnapshotPhaseV1::Eligibility {
                            PublisherNotAttemptedReasonV1::EligibilityFailed
                        } else {
                            PublisherNotAttemptedReasonV1::SnapshotFailed
                        }
                    }
                };
                require_publisher_reason(publisher, expected)
            }
        }
    }
}

fn require_publisher_reason(
    publisher: &PublisherNotAttemptedV1,
    expected: PublisherNotAttemptedReasonV1,
) -> Result<(), ProvenanceError> {
    if publisher.reason == expected {
        Ok(())
    } else {
        Err(invalid(
            "publisher.reason",
            "does not match the causal failure stage",
        ))
    }
}

fn is_lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_portable_path(value: &str, allow_root: bool) -> Result<(), ProvenanceError> {
    if value.is_empty() {
        return if allow_root {
            Ok(())
        } else {
            Err(ProvenanceError::InvalidPortablePath)
        };
    }
    if value.len() > MAX_PORTABLE_PATH_BYTES_V1
        || value.starts_with('/')
        || value.contains(['\0', '\\'])
        || (value.len() >= 2
            && value.as_bytes()[0].is_ascii_alphabetic()
            && value.as_bytes()[1] == b':')
        || value
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(ProvenanceError::InvalidPortablePath);
    }
    Ok(())
}

fn validate_heads_ref(value: &str) -> Result<(), ProvenanceError> {
    let Some(suffix) = value.strip_prefix("refs/heads/") else {
        return Err(ProvenanceError::InvalidHeadsRef);
    };
    if suffix.is_empty()
        || value.len() > MAX_REF_BYTES_V1
        || value.contains("..")
        || value.contains("@{")
        || value.ends_with('.')
        || value.bytes().any(|byte| {
            byte <= b' '
                || byte == 0x7f
                || matches!(byte, b'~' | b'^' | b':' | b'?' | b'*' | b'[' | b'\\')
        })
        || suffix.split('/').any(|component| {
            component.is_empty()
                || component.starts_with('.')
                || component.ends_with(".lock")
                || component.ends_with('.')
        })
    {
        return Err(ProvenanceError::InvalidHeadsRef);
    }
    Ok(())
}

fn valid_github_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value != "."
        && value != ".."
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
}

fn validate_rfc3339_utc(value: &str) -> Result<(), ProvenanceError> {
    if value.len() < 20
        || value.len() > 30
        || !value.ends_with('Z')
        || value.as_bytes().get(4) != Some(&b'-')
        || value.as_bytes().get(7) != Some(&b'-')
        || value.as_bytes().get(10) != Some(&b'T')
        || value.as_bytes().get(13) != Some(&b':')
        || value.as_bytes().get(16) != Some(&b':')
    {
        return Err(ProvenanceError::InvalidTimestamp);
    }
    let digit = |range: std::ops::Range<usize>| -> Result<u32, ProvenanceError> {
        let bytes = value
            .as_bytes()
            .get(range)
            .ok_or(ProvenanceError::InvalidTimestamp)?;
        if !bytes.iter().all(u8::is_ascii_digit) {
            return Err(ProvenanceError::InvalidTimestamp);
        }
        std::str::from_utf8(bytes)
            .ok()
            .and_then(|text| text.parse().ok())
            .ok_or(ProvenanceError::InvalidTimestamp)
    };
    let year = digit(0..4)?;
    let month = digit(5..7)?;
    let day = digit(8..10)?;
    let hour = digit(11..13)?;
    let minute = digit(14..16)?;
    let second = digit(17..19)?;
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return Err(ProvenanceError::InvalidTimestamp),
    };
    if year == 0 || day == 0 || day > days || hour > 23 || minute > 59 || second > 59 {
        return Err(ProvenanceError::InvalidTimestamp);
    }
    match value.as_bytes().get(19..value.len() - 1) {
        Some([]) => {}
        Some(fraction)
            if fraction.first() == Some(&b'.')
                && (1..=9).contains(&(fraction.len() - 1))
                && fraction[1..].iter().all(u8::is_ascii_digit) => {}
        _ => return Err(ProvenanceError::InvalidTimestamp),
    }
    Ok(())
}

fn fractional_digits(value: &str) -> [u8; 9] {
    let mut result = [b'0'; 9];
    if value.as_bytes().get(19) == Some(&b'.') {
        let digits = &value.as_bytes()[20..value.len() - 1];
        result[..digits.len()].copy_from_slice(digits);
    }
    result
}

pub(crate) fn require_time_order(
    first: &Rfc3339Utc,
    second: &Rfc3339Utc,
    field: &'static str,
) -> Result<(), ProvenanceError> {
    if first.chronological_cmp(second) == Ordering::Greater {
        Err(invalid(field, "timestamps are reversed"))
    } else {
        Ok(())
    }
}

fn invalid(field: &'static str, reason: &'static str) -> ProvenanceError {
    ProvenanceError::InvalidField { field, reason }
}
