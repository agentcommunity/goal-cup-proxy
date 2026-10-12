//! Versioned provenance documents for repository capture and publication.
//!
//! These types validate and bind declarations. They do not inspect a repository, read
//! captured objects, construct Git objects, run a publisher, or establish remote state.
//!
//! **Reserved for V1.1 (D42).** Nothing in this workspace produces the Git vocabulary below:
//! no daemon, CLI or publisher writes `FinalizationV1::Captured`, `FinalizationV1::CaptureUnavailable`
//! or `EventBody::RepositoryObservedV1` (only the workspace-submission finalization is ever
//! written), and the `Repository*`, `Snapshot*`, `Publisher*`, `PublicationRecovery*` and
//! `MAX_GIT_*_V1` items exist so that continuous Git provenance and publication can land
//! additively on the same wire. They are deserialised and validated, never emitted, and they are
//! not part of the v0.5 product. Removing them would be a wire change, so they stay until V1.1
//! decides.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;

use crate::submission::SubmissionOutcomeV1;

pub const SNAPSHOT_MANIFEST_DOMAIN: &[u8] = b"goalcup:pinned-eligible-repository-snapshot:v1\0";
pub const REPOSITORY_MANIFEST_DOMAIN: &[u8] = b"goalcup:sealed-repository-manifest:v1\0";
pub const REQUIRED_GIT_OBJECT_HISTORY_DOMAIN: &[u8] = b"goalcup:required-git-object-history:v1\0";
pub const RECOVERY_SIGNATURE_DOMAIN: &[u8] = b"goalcup:publication-recovery-signature:v1\0";
pub const RECOVERY_RECORD_DIGEST_DOMAIN: &[u8] = b"goalcup:publication-recovery-record-digest:v1\0";
pub const PUBLICATION_OBSERVATION_SIGNATURE_DOMAIN: &[u8] =
    b"goalcup:publication-observation-signature:v1\0";

pub const MAX_ACTIVE_GIT_CONFIG_SCOPES_V1: u64 = 4;
pub const MAX_GIT_CONFIG_BYTES_V1: u64 = 1_048_576;
pub const MAX_GIT_INDEX_BYTES_V1: u64 = 512 * 1024 * 1024;
pub const MAX_GIT_INDEX_ENTRIES_V1: u64 = 2_000_000;
pub const MAX_GITIGNORE_FILES_V1: u64 = 16_384;
pub const MAX_GITIGNORE_BYTES_V1: u64 = 1_048_576;
pub const MAX_IGNORE_BYTES_AGGREGATE_V1: u64 = 64 * 1024 * 1024;
pub const MAX_IGNORE_RULES_V1: u64 = 1_000_000;
pub const MAX_INFO_EXCLUDE_BYTES_V1: u64 = 16 * 1024 * 1024;
pub const MAX_GLOBAL_EXCLUDES_BYTES_V1: u64 = 16 * 1024 * 1024;
pub const MAX_REQUIRED_HISTORY_OBJECTS_V1: u64 = 1_000_000;
pub const MAX_REQUIRED_HISTORY_BYTES_V1: u64 = 16 * 1024 * 1024 * 1024;
pub const MAX_REQUIRED_HISTORY_OBJECT_BYTES_V1: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_REQUIRED_HISTORY_PARENT_EDGES_V1: u64 = 1_000_000;

const MAX_PORTABLE_PATH_BYTES_V1: usize = 4_096;
const MAX_REF_BYTES_V1: usize = 1_024;
const MAX_IDENTIFIER_BYTES_V1: usize = 256;
const MAX_OBSERVATION_SET_ITEMS_V1: usize = 1_000_000;

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
    #[error("integer overflow while validating {field}")]
    IntegerOverflow { field: &'static str },
    #[error("canonical JSON serialization failed")]
    Serialise,
    #[error("document key does not match the independently pinned key")]
    KeyMismatch,
    #[error("public key is not a usable Ed25519 key")]
    PublicKeyMalformed,
    #[error("signature is not canonical lowercase 128-character hex")]
    SignatureMalformed,
    #[error("signature does not verify against the independently pinned key")]
    SignatureInvalid,
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
pub struct PortableRelativePath(String);

impl PortableRelativePath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for PortableRelativePath {
    type Error = ProvenanceError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        validate_portable_path(value, false)?;
        Ok(Self(value.to_string()))
    }
}

impl TryFrom<String> for PortableRelativePath {
    type Error = ProvenanceError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl<'de> Deserialize<'de> for PortableRelativePath {
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

pub type ChainHash = Sha256Digest;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitObjectKindV1 {
    Commit,
    Tree,
    Blob,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitFileModeV1 {
    #[serde(rename = "100644")]
    Regular,
    #[serde(rename = "100755")]
    Executable,
    #[serde(rename = "120000")]
    Symlink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryTransitionV1 {
    Appeared,
    Updated,
    Replaced,
    Disappeared,
    Reconciled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryObservationUnavailableReasonV1 {
    BoundExceeded,
    RepositoryUnavailable,
    UnsupportedState,
    ReadFailed,
    Raced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum CompleteOrUnavailableV1<T> {
    Complete(Vec<T>),
    Unavailable {
        reason: RepositoryObservationUnavailableReasonV1,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ObservedHeadV1 {
    Unborn,
    Branch {
        target_ref: ValidatedHeadsRef,
        commit: GitObjectId,
        tree: GitObjectId,
    },
    Detached {
        commit: GitObjectId,
        tree: GitObjectId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ObservedRepositoryStateV1 {
    Present {
        workspace_relative_root: PortableRelativePathOrRoot,
        object_format: GitObjectFormat,
        head: ObservedHeadV1,
        parents: CompleteOrUnavailableV1<GitObjectId>,
        remotes: CompleteOrUnavailableV1<NormalizedGitHubRemote>,
        commits: CompleteOrUnavailableV1<GitObjectId>,
    },
    Disappeared {
        workspace_relative_root: PortableRelativePathOrRoot,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryObservedV1 {
    pub transition: RepositoryTransitionV1,
    pub state: ObservedRepositoryStateV1,
}

impl RepositoryObservedV1 {
    pub fn validate(&self) -> Result<(), ProvenanceError> {
        match (&self.transition, &self.state) {
            (
                RepositoryTransitionV1::Disappeared,
                ObservedRepositoryStateV1::Disappeared { .. },
            ) => Ok(()),
            (
                RepositoryTransitionV1::Appeared
                | RepositoryTransitionV1::Updated
                | RepositoryTransitionV1::Replaced
                | RepositoryTransitionV1::Reconciled,
                ObservedRepositoryStateV1::Present {
                    object_format,
                    head,
                    parents,
                    remotes,
                    commits,
                    ..
                },
            ) => {
                validate_observed_head(head, *object_format)?;
                validate_complete_ids(parents, *object_format)?;
                validate_complete(remotes, |remote| remote.validate())?;
                validate_complete_ids(commits, *object_format)
            }
            _ => Err(invalid("transition", "does not match repository state")),
        }
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
    pub fn validate(&self) -> Result<(), ProvenanceError> {
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
    pub fn validate(&self) -> Result<(), ProvenanceError> {
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

    fn expected_history_root(&self) -> Option<&GitObjectId> {
        match &self.pre_seal_head {
            PreSealHeadV1::Unborn => None,
            PreSealHeadV1::Existing { commit, .. } => Some(commit),
        }
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
pub enum RepositoryEligibilityPolicyV1 {
    OrdinaryGitTrackedPlusUntrackedNonIgnoredV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryEligibilityReleasePolicyV1 {
    GoalCupV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinnedEligibleRepositorySnapshotFormatV1 {
    PinnedEligibleRepositorySnapshotV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum CapturedEntryKindV1 {
    Regular {
        posix_owner_executable: Option<bool>,
    },
    SymbolicLink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotEntryV1 {
    pub repository_path: PortableRelativePath,
    pub observed_kind: CapturedEntryKindV1,
    pub bytes: u64,
    pub content_sha256: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedEligibleRepositorySnapshotManifestV1 {
    pub format: PinnedEligibleRepositorySnapshotFormatV1,
    pub entries: Vec<SnapshotEntryV1>,
}

impl PinnedEligibleRepositorySnapshotManifestV1 {
    pub fn validate(&self) -> Result<(), ProvenanceError> {
        if self.entries.len() as u64 > MAX_GIT_INDEX_ENTRIES_V1 {
            return Err(invalid("entries", "snapshot row bound exceeded"));
        }
        validate_sorted_unique_by(&self.entries, |entry| &entry.repository_path)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ProvenanceError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn digest(&self) -> Result<Sha256Digest, ProvenanceError> {
        Ok(domain_digest(
            SNAPSHOT_MANIFEST_DOMAIN,
            &self.canonical_bytes()?,
        ))
    }

    fn totals(&self) -> Result<(u64, u64), ProvenanceError> {
        let bytes = self.entries.iter().try_fold(0u64, |sum, entry| {
            sum.checked_add(entry.bytes)
                .ok_or(ProvenanceError::IntegerOverflow {
                    field: "snapshot object bytes",
                })
        })?;
        Ok((self.entries.len() as u64, bytes))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompleteEnumerationV1 {
    pub policy: RepositoryEligibilityPolicyV1,
    pub release_policy: RepositoryEligibilityReleasePolicyV1,
    pub eligibility_basis_sha256: Sha256Digest,
    pub included: u64,
    pub tracked_absent: u64,
    pub ignored: u64,
    pub release_excluded: u64,
    pub pruned_ignored_directories: u64,
}

impl CompleteEnumerationV1 {
    pub fn validate(&self) -> Result<(), ProvenanceError> {
        self.included
            .checked_add(self.tracked_absent)
            .and_then(|value| value.checked_add(self.ignored))
            .and_then(|value| value.checked_add(self.release_excluded))
            .and_then(|value| value.checked_add(self.pruned_ignored_directories))
            .ok_or(ProvenanceError::IntegerOverflow {
                field: "enumeration counts",
            })?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotCompletedV1 {
    pub eligibility: RepositoryEligibilityPolicyV1,
    pub eligibility_basis_sha256: Sha256Digest,
    pub enumeration: CompleteEnumerationV1,
    pub copy_started_at: Rfc3339Utc,
    pub copy_completed_at: Rfc3339Utc,
    pub snapshot_manifest_sha256: Sha256Digest,
    pub integrity_verified_at: Rfc3339Utc,
    pub object_count: u64,
    pub object_bytes: u64,
}

impl SnapshotCompletedV1 {
    pub fn validate(&self) -> Result<(), ProvenanceError> {
        self.enumeration.validate()?;
        if self.object_count > MAX_GIT_INDEX_ENTRIES_V1 {
            return Err(invalid("object_count", "snapshot row bound exceeded"));
        }
        if self.eligibility != self.enumeration.policy
            || self.eligibility_basis_sha256 != self.enumeration.eligibility_basis_sha256
        {
            return Err(invalid(
                "eligibility",
                "snapshot and enumeration authorities differ",
            ));
        }
        if self.object_count != self.enumeration.included {
            return Err(invalid(
                "object_count",
                "does not equal the complete included count",
            ));
        }
        require_time_order(
            &self.copy_started_at,
            &self.copy_completed_at,
            "snapshot copy",
        )?;
        require_time_order(
            &self.copy_completed_at,
            &self.integrity_verified_at,
            "snapshot verification",
        )
    }

    pub fn validate_manifest(
        &self,
        manifest: &PinnedEligibleRepositorySnapshotManifestV1,
    ) -> Result<(), ProvenanceError> {
        self.validate()?;
        let (count, bytes) = manifest.totals()?;
        if self.snapshot_manifest_sha256 != manifest.digest()?
            || self.object_count != count
            || self.object_bytes != bytes
        {
            return Err(invalid(
                "snapshot_manifest",
                "digest, count, or byte sum differs from the descriptor",
            ));
        }
        Ok(())
    }
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
pub enum RepositoryManifestVersion {
    GitTreeV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealedRepositoryEntryV1 {
    pub repository_path: PortableRelativePath,
    pub bytes: u64,
    pub content_sha256: Sha256Digest,
    pub mode: GitFileModeV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealedRepositoryManifestV1 {
    pub format: RepositoryManifestVersion,
    pub entries: Vec<SealedRepositoryEntryV1>,
}

impl SealedRepositoryManifestV1 {
    pub fn validate(&self) -> Result<(), ProvenanceError> {
        if self.entries.len() as u64 > MAX_GIT_INDEX_ENTRIES_V1 {
            return Err(invalid("entries", "repository row bound exceeded"));
        }
        validate_sorted_unique_by(&self.entries, |entry| &entry.repository_path)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ProvenanceError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn digest(&self) -> Result<Sha256Digest, ProvenanceError> {
        Ok(domain_digest(
            REPOSITORY_MANIFEST_DOMAIN,
            &self.canonical_bytes()?,
        ))
    }

    pub fn validate_against_snapshot(
        &self,
        snapshot: &PinnedEligibleRepositorySnapshotManifestV1,
    ) -> Result<(), ProvenanceError> {
        self.validate()?;
        snapshot.validate()?;
        if self.entries.len() != snapshot.entries.len() {
            return Err(invalid(
                "repository_manifest",
                "row count differs from the snapshot",
            ));
        }
        for (repository, captured) in self.entries.iter().zip(&snapshot.entries) {
            if repository.repository_path != captured.repository_path
                || repository.bytes != captured.bytes
                || repository.content_sha256 != captured.content_sha256
            {
                return Err(invalid(
                    "repository_manifest",
                    "path, digest, or byte length differs from the snapshot",
                ));
            }
            match (&captured.observed_kind, repository.mode) {
                (CapturedEntryKindV1::SymbolicLink, GitFileModeV1::Symlink) => {}
                (
                    CapturedEntryKindV1::Regular {
                        posix_owner_executable: Some(true),
                    },
                    GitFileModeV1::Executable,
                ) => {}
                (
                    CapturedEntryKindV1::Regular {
                        posix_owner_executable: Some(false),
                    },
                    GitFileModeV1::Regular,
                ) => {}
                (
                    CapturedEntryKindV1::Regular {
                        posix_owner_executable: None,
                    },
                    GitFileModeV1::Regular | GitFileModeV1::Executable,
                ) => {}
                _ => {
                    return Err(invalid(
                        "repository_manifest",
                        "Git mode does not match the observed entry kind",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequiredGitObjectHistoryFormatV1 {
    RequiredGitObjectHistoryV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredGitObjectV1 {
    pub object_id: GitObjectId,
    pub kind: GitObjectKindV1,
    pub uncompressed_bytes: u64,
    pub content_sha256: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredGitObjectHistoryManifestV1 {
    pub format: RequiredGitObjectHistoryFormatV1,
    pub object_format: GitObjectFormat,
    pub root_commit: Option<GitObjectId>,
    pub objects: Vec<RequiredGitObjectV1>,
}

impl RequiredGitObjectHistoryManifestV1 {
    pub fn validate(&self) -> Result<(), ProvenanceError> {
        if self.objects.len() as u64 > MAX_REQUIRED_HISTORY_OBJECTS_V1 {
            return Err(invalid("objects", "required-history object bound exceeded"));
        }
        if let Some(root) = &self.root_commit {
            root.validate_for(self.object_format)?;
            if !self
                .objects
                .iter()
                .any(|object| object.object_id == *root && object.kind == GitObjectKindV1::Commit)
            {
                return Err(invalid(
                    "root_commit",
                    "must name a commit row in the complete history",
                ));
            }
        } else if !self.objects.is_empty() {
            return Err(invalid(
                "objects",
                "an unborn history must be the canonical empty set",
            ));
        }
        validate_sorted_unique_by(&self.objects, |object| &object.object_id)?;
        let mut sum = 0u64;
        for object in &self.objects {
            object.object_id.validate_for(self.object_format)?;
            if object.uncompressed_bytes > MAX_REQUIRED_HISTORY_OBJECT_BYTES_V1 {
                return Err(invalid(
                    "uncompressed_bytes",
                    "required-history object byte bound exceeded",
                ));
            }
            sum = sum.checked_add(object.uncompressed_bytes).ok_or(
                ProvenanceError::IntegerOverflow {
                    field: "required-history bytes",
                },
            )?;
        }
        if sum > MAX_REQUIRED_HISTORY_BYTES_V1 {
            return Err(invalid(
                "uncompressed_object_bytes",
                "required-history aggregate byte bound exceeded",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ProvenanceError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn digest(&self) -> Result<Sha256Digest, ProvenanceError> {
        Ok(domain_digest(
            REQUIRED_GIT_OBJECT_HISTORY_DOMAIN,
            &self.canonical_bytes()?,
        ))
    }

    fn total_bytes(&self) -> Result<u64, ProvenanceError> {
        self.objects.iter().try_fold(0u64, |sum, object| {
            sum.checked_add(object.uncompressed_bytes)
                .ok_or(ProvenanceError::IntegerOverflow {
                    field: "required-history bytes",
                })
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredGitObjectHistoryV1 {
    pub root_commit: Option<GitObjectId>,
    pub manifest_sha256: Sha256Digest,
    pub object_count: u64,
    pub uncompressed_object_bytes: u64,
}

impl RequiredGitObjectHistoryV1 {
    pub fn validate_against_selection(
        &self,
        selection: &SelectedRepositoryV1,
    ) -> Result<(), ProvenanceError> {
        selection.validate()?;
        if self.object_count > MAX_REQUIRED_HISTORY_OBJECTS_V1
            || self.uncompressed_object_bytes > MAX_REQUIRED_HISTORY_BYTES_V1
        {
            return Err(invalid(
                "required_history",
                "declared history bounds are exceeded",
            ));
        }
        if self.root_commit.as_ref() != selection.expected_history_root() {
            return Err(invalid(
                "root_commit",
                "does not match the frozen pre-seal head",
            ));
        }
        if let Some(root) = &self.root_commit {
            root.validate_for(selection.object_format)?;
        }
        Ok(())
    }

    pub fn validate_manifest(
        &self,
        manifest: &RequiredGitObjectHistoryManifestV1,
        selection: &SelectedRepositoryV1,
    ) -> Result<(), ProvenanceError> {
        self.validate_against_selection(selection)?;
        manifest.validate()?;
        if manifest.object_format != selection.object_format
            || manifest.root_commit != self.root_commit
            || self.manifest_sha256 != manifest.digest()?
            || self.object_count != manifest.objects.len() as u64
            || self.uncompressed_object_bytes != manifest.total_bytes()?
        {
            return Err(invalid(
                "required_history",
                "manifest and descriptor bindings differ",
            ));
        }
        Ok(())
    }
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
    pub fn validate(&self) -> Result<(), ProvenanceError> {
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
    pub fn validate(&self) -> Result<(), ProvenanceError> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryProjectionFailureV1 {
    SnapshotManifestInvalid,
    SnapshotObjectUnavailable,
    SnapshotObjectMismatch,
    ModeUnavailable,
    TreeConstructionUnavailable,
    InternalConsistency,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequiredGitHistoryFailureV1 {
    MissingObject,
    CorruptObject,
    UnsupportedObjectStorage,
    Raced,
    BoundExceeded,
    ContainsReleaseExcludedPath,
    UnsupportedHistoricalPath,
    DanglingReference,
    MismatchedObjectId,
    ShallowRepository,
    PromisorRepository,
    ReplacementRefs,
    Grafts,
    Alternates,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum CommitConstructionFailureV1 {
    RequiredHistory { reason: RequiredGitHistoryFailureV1 },
    RepositoryManifestMismatch,
    ObjectIdMismatch,
    UnsupportedObjectFormat,
    InvalidDeadline,
    InternalConsistency,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RepositoryFinalizationV1 {
    ProjectionFailed {
        failure: RepositoryProjectionFailureV1,
        publisher: PublisherNotAttemptedV1,
    },
    ProjectionComplete {
        repository_manifest_sha256: Sha256Digest,
        commit: Box<CommitConstructionOutcomeV1>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum CommitConstructionOutcomeV1 {
    Resolved {
        required_history: RequiredGitObjectHistoryV1,
        disposition: CommitDispositionV1,
        publication: Box<InitialPublicationOutcomeV1>,
    },
    Failed {
        failure: CommitConstructionFailureV1,
        publisher: PublisherNotAttemptedV1,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum CommitDispositionV1 {
    ReusedHead {
        commit: GitObjectId,
        tree: GitObjectId,
    },
    CreatedSealCommit {
        commit: GitObjectId,
        tree: GitObjectId,
        parent: Option<GitObjectId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum InitialPublicationOutcomeV1 {
    Attempted(PublisherAttemptV1),
    NotAttempted(PublisherNotAttemptedV1),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublisherAttemptTriggerV1 {
    InitialCutoff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublisherPhaseV1 {
    Prepare,
    Spawn,
    Negotiate,
    Transfer,
    UpdateRef,
    Confirm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublisherFailureReasonV1 {
    CredentialUnavailable,
    IsolationFailed,
    ProtocolFailed,
    TransportFailed,
    RemoteFailed,
    ProcessFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PublisherAttemptOutcomeV1 {
    PushReported {
        commit: GitObjectId,
        tree: GitObjectId,
        target_ref: ValidatedHeadsRef,
    },
    TimedOut {
        phase: PublisherPhaseV1,
    },
    RejectedRemote,
    Failed {
        phase: PublisherPhaseV1,
        reason: PublisherFailureReasonV1,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublisherAttemptV1 {
    pub attempt_id: String,
    pub trigger: PublisherAttemptTriggerV1,
    pub required_history_manifest_sha256: Sha256Digest,
    pub started_at: Rfc3339Utc,
    pub finished_at: Rfc3339Utc,
    pub outcome: PublisherAttemptOutcomeV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum FinalizationV1 {
    Captured {
        receipt_sha256: Sha256Digest,
        termination: TerminationOutcomeV1,
        selected: Box<SelectedRepositoryV1>,
        snapshot: SnapshotCompletedV1,
        repository: Box<RepositoryFinalizationV1>,
    },
    CaptureUnavailable {
        receipt_sha256: Sha256Digest,
        termination: TerminationOutcomeV1,
        capture: CaptureUnavailableV1,
        publisher: PublisherNotAttemptedV1,
    },
    WorkspaceSubmission {
        receipt_sha256: Sha256Digest,
        termination: TerminationOutcomeV1,
        submission: SubmissionOutcomeV1,
    },
}

impl FinalizationV1 {
    pub fn receipt_sha256(&self) -> &Sha256Digest {
        match self {
            Self::Captured { receipt_sha256, .. }
            | Self::CaptureUnavailable { receipt_sha256, .. }
            | Self::WorkspaceSubmission { receipt_sha256, .. } => receipt_sha256,
        }
    }

    pub fn validate(&self) -> Result<(), ProvenanceError> {
        match self {
            Self::WorkspaceSubmission { termination, .. } => termination.validate(),
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
            Self::Captured {
                termination,
                selected,
                snapshot,
                repository,
                ..
            } => {
                termination.validate()?;
                selected.validate()?;
                snapshot.validate()?;
                match repository.as_ref() {
                    RepositoryFinalizationV1::ProjectionFailed { publisher, .. } => {
                        require_publisher_reason(
                            publisher,
                            PublisherNotAttemptedReasonV1::ProjectionFailed,
                        )
                    }
                    RepositoryFinalizationV1::ProjectionComplete { commit, .. } => {
                        validate_commit_outcome(commit, selected)
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryPreviousV1 {
    Terminal(ChainHash),
    Attempt(Sha256Digest),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryTriggerV1 {
    AutomaticRestart,
    OperatorRequested,
}

pub type RecoveryAttemptOutcomeV1 = PublisherAttemptOutcomeV1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationRecoveryAttemptClaimsV1 {
    pub entry_id: String,
    pub session_id: String,
    pub receipt_sha256: Sha256Digest,
    pub terminal_chain_hash: ChainHash,
    pub snapshot_manifest_sha256: Sha256Digest,
    pub repository_manifest_sha256: Sha256Digest,
    pub required_history_manifest_sha256: Sha256Digest,
    pub remote: NormalizedGitHubRemote,
    pub target_ref: ValidatedHeadsRef,
    pub object_format: GitObjectFormat,
    pub commit: GitObjectId,
    pub tree: GitObjectId,
    pub attempt_number: u32,
    pub previous: RecoveryPreviousV1,
    pub trigger: RecoveryTriggerV1,
    pub started_at: Rfc3339Utc,
    pub finished_at: Rfc3339Utc,
    pub outcome: RecoveryAttemptOutcomeV1,
}

impl PublicationRecoveryAttemptClaimsV1 {
    pub fn validate(&self) -> Result<(), ProvenanceError> {
        validate_identifier("entry_id", &self.entry_id)?;
        validate_identifier("session_id", &self.session_id)?;
        self.remote.validate()?;
        self.commit.validate_for(self.object_format)?;
        self.tree.validate_for(self.object_format)?;
        if self.attempt_number == 0 {
            return Err(invalid("attempt_number", "must start at one"));
        }
        match (self.attempt_number, &self.previous) {
            (1, RecoveryPreviousV1::Terminal(hash)) if hash == &self.terminal_chain_hash => {}
            (1, _) => {
                return Err(invalid(
                    "previous",
                    "attempt one must bind the terminal chain hash",
                ));
            }
            (_, RecoveryPreviousV1::Attempt(_)) => {}
            (_, RecoveryPreviousV1::Terminal(_)) => {
                return Err(invalid(
                    "previous",
                    "later attempts must bind the prior signed record",
                ));
            }
        }
        require_time_order(&self.started_at, &self.finished_at, "recovery attempt")?;
        validate_push_outcome(
            &self.outcome,
            self.object_format,
            &self.commit,
            &self.tree,
            &self.target_ref,
        )
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProvenanceError> {
        self.validate()?;
        domain_bytes(RECOVERY_SIGNATURE_DOMAIN, self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedPublicationRecoveryAttemptV1 {
    pub claims: PublicationRecoveryAttemptClaimsV1,
    pub run_public_key: String,
    pub signature: String,
}

impl SignedPublicationRecoveryAttemptV1 {
    pub fn sign(
        claims: PublicationRecoveryAttemptClaimsV1,
        key: &SigningKey,
    ) -> Result<Self, ProvenanceError> {
        let signature = key.sign(&claims.signing_bytes()?);
        Ok(Self {
            claims,
            run_public_key: hex::encode(key.verifying_key().to_bytes()),
            signature: hex::encode(signature.to_bytes()),
        })
    }

    pub fn verify_with(&self, pinned_run_public_key: &str) -> Result<(), ProvenanceError> {
        verify_signed(
            &self.run_public_key,
            pinned_run_public_key,
            &self.signature,
            &self.claims.signing_bytes()?,
        )
    }

    pub fn complete_digest(&self) -> Result<Sha256Digest, ProvenanceError> {
        self.claims.validate()?;
        validate_signature_hex(&self.signature)?;
        validate_public_key_hex(&self.run_public_key)?;
        Ok(domain_digest(
            RECOVERY_RECORD_DIGEST_DOMAIN,
            &canonical_json(self)?,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationObservationClaimsV1 {
    pub entry_id: String,
    pub session_id: String,
    pub receipt_sha256: Sha256Digest,
    pub terminal_chain_hash: ChainHash,
    pub snapshot_manifest_sha256: Sha256Digest,
    pub repository_manifest_sha256: Sha256Digest,
    pub remote: NormalizedGitHubRemote,
    pub target_ref: ValidatedHeadsRef,
    pub object_format: GitObjectFormat,
    pub commit: GitObjectId,
    pub tree: GitObjectId,
    pub first_observed_at: Rfc3339Utc,
}

impl PublicationObservationClaimsV1 {
    pub fn validate(&self) -> Result<(), ProvenanceError> {
        validate_identifier("entry_id", &self.entry_id)?;
        validate_identifier("session_id", &self.session_id)?;
        self.remote.validate()?;
        self.commit.validate_for(self.object_format)?;
        self.tree.validate_for(self.object_format)
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProvenanceError> {
        self.validate()?;
        domain_bytes(PUBLICATION_OBSERVATION_SIGNATURE_DOMAIN, self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationObservationReceiptV1 {
    pub claims: PublicationObservationClaimsV1,
    pub public_key: String,
    pub signature: String,
}

impl PublicationObservationReceiptV1 {
    pub fn sign(
        claims: PublicationObservationClaimsV1,
        key: &SigningKey,
    ) -> Result<Self, ProvenanceError> {
        let signature = key.sign(&claims.signing_bytes()?);
        Ok(Self {
            claims,
            public_key: hex::encode(key.verifying_key().to_bytes()),
            signature: hex::encode(signature.to_bytes()),
        })
    }

    pub fn verify_with(&self, pinned_public_key: &str) -> Result<(), ProvenanceError> {
        verify_signed(
            &self.public_key,
            pinned_public_key,
            &self.signature,
            &self.claims.signing_bytes()?,
        )
    }
}

fn validate_observed_head(
    head: &ObservedHeadV1,
    format: GitObjectFormat,
) -> Result<(), ProvenanceError> {
    match head {
        ObservedHeadV1::Unborn => Ok(()),
        ObservedHeadV1::Branch { commit, tree, .. } | ObservedHeadV1::Detached { commit, tree } => {
            commit.validate_for(format)?;
            tree.validate_for(format)
        }
    }
}

fn validate_complete_ids(
    value: &CompleteOrUnavailableV1<GitObjectId>,
    format: GitObjectFormat,
) -> Result<(), ProvenanceError> {
    validate_complete(value, |id| id.validate_for(format))
}

fn validate_complete<T: Ord>(
    value: &CompleteOrUnavailableV1<T>,
    validate: impl Fn(&T) -> Result<(), ProvenanceError>,
) -> Result<(), ProvenanceError> {
    let CompleteOrUnavailableV1::Complete(values) = value else {
        return Ok(());
    };
    if values.len() > MAX_OBSERVATION_SET_ITEMS_V1 {
        return Err(invalid("complete_set", "item bound exceeded"));
    }
    validate_sorted_unique(values)?;
    for item in values {
        validate(item)?;
    }
    Ok(())
}

fn validate_commit_outcome(
    outcome: &CommitConstructionOutcomeV1,
    selected: &SelectedRepositoryV1,
) -> Result<(), ProvenanceError> {
    match outcome {
        CommitConstructionOutcomeV1::Failed { failure, publisher } => {
            let expected = if matches!(failure, CommitConstructionFailureV1::RequiredHistory { .. })
            {
                PublisherNotAttemptedReasonV1::RequiredHistoryFailed
            } else {
                PublisherNotAttemptedReasonV1::CommitConstructionFailed
            };
            require_publisher_reason(publisher, expected)
        }
        CommitConstructionOutcomeV1::Resolved {
            required_history,
            disposition,
            publication,
        } => {
            required_history.validate_against_selection(selected)?;
            let (commit, tree) = validate_disposition(disposition, selected)?;
            match publication.as_ref() {
                InitialPublicationOutcomeV1::NotAttempted(publisher) => {
                    if !matches!(
                        publisher.reason,
                        PublisherNotAttemptedReasonV1::CredentialUnavailable
                            | PublisherNotAttemptedReasonV1::PublisherSetupFailed
                    ) {
                        return Err(invalid(
                            "publisher.reason",
                            "does not name a post-construction cause",
                        ));
                    }
                    Ok(())
                }
                InitialPublicationOutcomeV1::Attempted(attempt) => {
                    validate_identifier("attempt_id", &attempt.attempt_id)?;
                    require_time_order(
                        &attempt.started_at,
                        &attempt.finished_at,
                        "publisher attempt",
                    )?;
                    if attempt.required_history_manifest_sha256 != required_history.manifest_sha256
                    {
                        return Err(invalid(
                            "required_history_manifest_sha256",
                            "does not match the resolved history",
                        ));
                    }
                    validate_push_outcome(
                        &attempt.outcome,
                        selected.object_format,
                        commit,
                        tree,
                        &selected.target_ref,
                    )
                }
            }
        }
    }
}

fn validate_disposition<'a>(
    disposition: &'a CommitDispositionV1,
    selected: &SelectedRepositoryV1,
) -> Result<(&'a GitObjectId, &'a GitObjectId), ProvenanceError> {
    match disposition {
        CommitDispositionV1::ReusedHead { commit, tree } => {
            commit.validate_for(selected.object_format)?;
            tree.validate_for(selected.object_format)?;
            match &selected.pre_seal_head {
                PreSealHeadV1::Existing {
                    commit: expected_commit,
                    tree: expected_tree,
                } if commit == expected_commit && tree == expected_tree => Ok((commit, tree)),
                _ => Err(invalid(
                    "disposition",
                    "reused head does not equal the frozen existing head",
                )),
            }
        }
        CommitDispositionV1::CreatedSealCommit {
            commit,
            tree,
            parent,
        } => {
            commit.validate_for(selected.object_format)?;
            tree.validate_for(selected.object_format)?;
            if parent.as_ref() != selected.expected_history_root() {
                return Err(invalid(
                    "parent",
                    "does not equal the frozen pre-seal commit",
                ));
            }
            if parent.as_ref() == Some(commit) {
                return Err(invalid("commit", "cannot be its own parent"));
            }
            if let PreSealHeadV1::Existing {
                tree: existing_tree,
                ..
            } = &selected.pre_seal_head
                && tree == existing_tree
            {
                return Err(invalid(
                    "tree",
                    "an unchanged pre-seal tree must use reused_head",
                ));
            }
            Ok((commit, tree))
        }
    }
}

fn validate_push_outcome(
    outcome: &PublisherAttemptOutcomeV1,
    format: GitObjectFormat,
    expected_commit: &GitObjectId,
    expected_tree: &GitObjectId,
    expected_ref: &ValidatedHeadsRef,
) -> Result<(), ProvenanceError> {
    if let PublisherAttemptOutcomeV1::PushReported {
        commit,
        tree,
        target_ref,
    } = outcome
    {
        commit.validate_for(format)?;
        tree.validate_for(format)?;
        if commit != expected_commit || tree != expected_tree || target_ref != expected_ref {
            return Err(invalid(
                "push_reported",
                "does not match the frozen commit, tree, and target ref",
            ));
        }
    }
    Ok(())
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

fn validate_sorted_unique<T: Ord>(values: &[T]) -> Result<(), ProvenanceError> {
    if values.windows(2).any(|pair| pair[0] >= pair[1]) {
        Err(invalid(
            "rows",
            "must be strictly sorted and duplicate-free",
        ))
    } else {
        Ok(())
    }
}

fn validate_sorted_unique_by<T, K: Ord>(
    values: &[T],
    key: impl Fn(&T) -> &K,
) -> Result<(), ProvenanceError> {
    if values.windows(2).any(|pair| key(&pair[0]) >= key(&pair[1])) {
        Err(invalid(
            "rows",
            "must be strictly sorted and duplicate-free",
        ))
    } else {
        Ok(())
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

fn validate_public_key_hex(value: &str) -> Result<(), ProvenanceError> {
    if is_lower_hex(value, 64) {
        Ok(())
    } else {
        Err(ProvenanceError::PublicKeyMalformed)
    }
}

fn validate_signature_hex(value: &str) -> Result<(), ProvenanceError> {
    if is_lower_hex(value, 128) {
        Ok(())
    } else {
        Err(ProvenanceError::SignatureMalformed)
    }
}

fn verify_signed(
    document_key: &str,
    pinned_key: &str,
    signature: &str,
    bytes: &[u8],
) -> Result<(), ProvenanceError> {
    validate_public_key_hex(document_key)?;
    if !document_key.eq_ignore_ascii_case(pinned_key) {
        return Err(ProvenanceError::KeyMismatch);
    }
    validate_signature_hex(signature)?;
    let public_key: [u8; 32] = hex::decode(pinned_key)
        .map_err(|_| ProvenanceError::PublicKeyMalformed)?
        .try_into()
        .map_err(|_| ProvenanceError::PublicKeyMalformed)?;
    let signature: [u8; 64] = hex::decode(signature)
        .map_err(|_| ProvenanceError::SignatureMalformed)?
        .try_into()
        .map_err(|_| ProvenanceError::SignatureMalformed)?;
    let verifying_key =
        VerifyingKey::from_bytes(&public_key).map_err(|_| ProvenanceError::PublicKeyMalformed)?;
    if verifying_key.is_weak() {
        return Err(ProvenanceError::PublicKeyMalformed);
    }
    verifying_key
        .verify_strict(bytes, &Signature::from_bytes(&signature))
        .map_err(|_| ProvenanceError::SignatureInvalid)
}

fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ProvenanceError> {
    serde_json::to_vec(value).map_err(|_| ProvenanceError::Serialise)
}

fn domain_bytes<T: Serialize>(domain: &[u8], value: &T) -> Result<Vec<u8>, ProvenanceError> {
    let mut bytes = domain.to_vec();
    bytes.extend_from_slice(&canonical_json(value)?);
    Ok(bytes)
}

fn domain_digest(domain: &[u8], bytes: &[u8]) -> Sha256Digest {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    Sha256Digest::from_bytes(hasher.finalize().into())
}

fn invalid(field: &'static str, reason: &'static str) -> ProvenanceError {
    ProvenanceError::InvalidField { field, reason }
}
