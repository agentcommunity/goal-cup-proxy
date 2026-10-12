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
use goalcup_protocol::export::{TraceDescriptorDoc, manifest_sha256};
use goalcup_protocol::manifest::ForeignScanRoot;
use goalcup_protocol::process::{ProcessHandle, ProcessRegistration};
use goalcup_protocol::provenance::{
    PreCutoffDrainOutcomeV1, Rfc3339Utc, Sha256Digest, TerminationOutcomeV1,
};
use goalcup_protocol::receipt::StartReceipt;
use goalcup_protocol::seal::SealManifest;
use goalcup_protocol::submission::{
    EffectiveSubmissionLimitsV1, FinishReasonV1, LimitProfileV1, SubmissionFailureV1,
    SubmissionOutcomeV1, SubmissionProfileV1,
};
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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

impl WorkspaceSealingIntentV1 {
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

const MAX_PREPARED_HOME_SEED_BYTES: usize = 64 * 1024;

/// The three per-run secrets.
///
/// They are minted once per entry and never leave this machine: the buffer key encrypts
/// the write-ahead store, the signing seed is the run key every `SignedStatus` is
/// signed with, and the receiver token is the per-run bearer a rendered tracker
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
    /// is pinned against, and the key the receipt request carries.
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
/// The V1 seal's live answer (`runtime::SealReport`, removed by D48) went to its caller;
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
    /// The durable half of the V1 `SealReport::harness_unverified` (removed by D48), and what
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
pub(crate) const MAX_FOREIGN_PENDING_SIGHTINGS: usize = 256;
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
    /// and then guards. Held on the row rather than recomputed at start binding (the V1
    /// `bind_start`, removed by D48) so the receipt names the bytes the workspace was
    /// actually built from.
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
    /// convention each caller has to remember.
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
    /// An `Option` rather than a derived default, because the V1 pre-arm scan (removed by
    /// D48 and OS-31) asked "does another entry already live in this home?", and a
    /// guessed-but-plausible default would have every unplaced entry claim a directory
    /// nothing is in. Rows written then still read back with `None` for an unplaced entry.
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
    /// — see `crate::status::event_status_from`.
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

    /// The immutable receipt policy captured for this entry.
    pub fn receipt_binding(&self) -> &ReceiptBindingPolicy {
        &self.receipt_binding
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

    /// The process route may receive a handle only from a launch path that owes one.
    pub(crate) fn accepts_process_handoff(&self) -> bool {
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
}

impl Registry {
    /// Where the registry belongs: `<artifact_root>/registry/registry.redb`.
    ///
    /// A directory of its own, and not the shared `config_dir` the CLI reads,
    /// because [`Registry::open`] tightens the directory it owns to `0o700`
    /// and a directory shared with other readers is not the registry's to close.
    pub fn default_path(artifact_root: &Path) -> PathBuf {
        artifact_root.join("registry").join("registry.redb")
    }

    /// Open the production registry through the daemon's retained artifact-root
    /// capability. The configured path remains diagnostic only after root acquisition.
    pub(crate) fn open_artifact(artifact_root: &ArtifactRoot) -> Result<Registry, DaemonError> {
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
        Ok(Registry { db })
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

    pub fn remove(&self, entry_id: &str) -> Result<(), DaemonError> {
        let txn = self.db.begin_write().map_err(|e| plain(e, "remove"))?;
        {
            let mut table = txn.open_table(ENTRIES).map_err(|e| plain(e, "remove"))?;
            table.remove(entry_id).map_err(|e| plain(e, "remove"))?;
        }
        txn.commit().map_err(|e| plain(e, "remove"))
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
    pub(crate) fn exact_accepted_process_replay_phase(
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
    pub(crate) fn acknowledge_sealed_process_replay(
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
/// `pub(crate)` so any second owner-only file in this crate takes the same repair on open.
/// The V1 buffer was one until D48 removed it. One spelling of "owner only", not two.
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
