//! The entry registry: every run this daemon knows about, and what it knows about it.
//!
//! An entry holds a workspace path, harness, manifest hash, signed receipt,
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
//! **The file is a secret store.** It holds [`RunKeys`] in plaintext — the signing seed that
//! is the run key itself. On unix the file is created `0o600` and a file found looser than
//! that is tightened on open; the directory [`Registry::default_path`] gives it is
//! `0o700`, and a directory the registry merely happens to sit in is left alone.
//! `RunKeys` redacts its own `Debug`, the way `control::WorkspaceInfo` already does, so
//! the same secret cannot reach a log through an error message either.

use crate::config::ReceiptBindingPolicy;
use crate::{DaemonError, now_rfc3339};
use goalcup_platform::tracecap::ArtifactRoot;
use goalcup_protocol::chain::GENESIS;
use goalcup_protocol::control::EventStatus;
use goalcup_protocol::events::HealthState;
use goalcup_protocol::process::{ProcessHandle, ProcessRegistration};
use goalcup_protocol::provenance::Sha256Digest;
use goalcup_protocol::receipt::StartReceipt;
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One JSON document per entry, keyed by `entry_id`. JSON rather than a hand-rolled
/// encoding so an added field cannot silently shift the meaning of the bytes already on
/// disk.
const ENTRIES: TableDefinition<&str, &[u8]> = TableDefinition::new("entries");

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
    /// A V1 seal had reserved the durable state transition but not yet reconciled it against
    /// the terminal chain link. No current path enters it; `Daemon::start` refuses a registry
    /// that holds such a V1 row.
    Sealing,
    Sealed,
    Refused,
}

/// The registered launch authority latched by the same transaction as finalization.
/// `registration` uses the protocol's strict nested decoder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedProcessHandoffV1 {
    pub request_digest: Sha256Digest,
    pub registration: ProcessRegistration,
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
    /// The request identity is durable; the event must be reconciled
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

/// The per-run secret: the signing seed every `SignedStatus` is signed with.
///
/// Minted once per entry and never leaves this machine.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunKeys {
    /// 32 random bytes, hex: the Ed25519 seed, not the expanded key.
    pub signing_seed_hex: String,
    /// The V1 buffer key and receiver token. Nothing in this build reads them, and a new
    /// entry writes them empty: no secret is minted for them. They stay on the row because
    /// a v0.5.0 daemon, which `Daemon::start` names as the way to finish a V1 entry on the
    /// same artifact root, requires both keys to read any row.
    #[serde(default)]
    buffer_key_hex: String,
    #[serde(default)]
    receiver_token: String,
}

/// Nothing but the shape. The seed is a secret, and a derived `Debug` would put it in every
/// log line and every interpolated error that carries an `Entry` — the same leak
/// `control::WorkspaceInfo` hand-writes its `Debug` to close.
impl std::fmt::Debug for RunKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunKeys")
            .field("signing_seed_hex", &"<redacted>")
            .finish()
    }
}

impl RunKeys {
    /// A fresh seed from the operating system's entropy source.
    ///
    /// `fill_bytes` rather than `try_fill_bytes` because the signature returns
    /// `RunKeys` and not a `Result`: on every platform this daemon targets, a failing
    /// `OsRng` is a machine that cannot keep a secret at all, and continuing with a
    /// predictable run key would be worse than the abort `rand` raises.
    pub fn generate() -> RunKeys {
        use rand::RngCore;
        let mut signing = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut signing);
        RunKeys::from_seed(signing)
    }

    /// The same key from bytes a caller chose. Tests use it for reproducible runs; no
    /// production path does.
    pub fn from_seed(signing: [u8; 32]) -> RunKeys {
        RunKeys {
            signing_seed_hex: hex::encode(signing),
            buffer_key_hex: String::new(),
            receiver_token: String::new(),
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

/// Everything the daemon knows about one run.
///
/// `chain_seq`/`chain_head` are a **mirror of the export's own head, never an input to
/// it**: `TraceExport::resume` takes the trace root and nothing else, and reads its
/// position off `chain.jsonl`. These two fields exist so a status or a receipt
/// claim can be produced without opening the export. `chain_seq` is an
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
    /// A capture-closed terminal buffer or trace failed its recovery validation.  The chain
    /// cannot carry a post-seal health event, so this durable quarantine is the authority
    /// the signed status reads until an operator resolves the evidence failure.
    #[serde(default)]
    pub terminal_drain_validation_failed: bool,
    pub sealed_at: Option<String>,
    /// Three V1 fields nothing in this build reads. They stay on the row, at the values a new
    /// entry always had, because a v0.5.0 daemon (which `Daemon::start` names as the way to
    /// finish a V1 entry on the same artifact root) requires them to read any row.
    #[serde(default)]
    events_uploaded: u64,
    #[serde(default)]
    expected_config: BTreeMap<String, String>,
    #[serde(default)]
    status_line_path: PathBuf,
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
    /// is private registry state, not a receipt claim.
    #[serde(default)]
    process_handoff_digest: Option<String>,
    /// The termination identity awaiting/holding registration.
    #[serde(default)]
    prepared_process_handle: Option<ProcessHandle>,
    /// The exact Prepared identity an idempotent cancellation returned.  Unlike the live
    /// handoff digest this is not termination authority; it only prevents a lost cancel
    /// response from turning a later caller's different digest into a success.
    #[serde(default)]
    cancelled_process_handoff_digest: Option<String>,
    pub last_process: Option<ProcessHandle>,
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

impl Entry {
    /// A created entry: bound to a workspace and a connector, holding its keys, and
    /// carrying nothing that only a later stage can know.
    ///
    /// `health` starts [`HealthState::Online`] — the daemon is up and has observed
    /// nothing yet. Arming is what turns it into `ArmingPending`, and arming has not
    /// happened: `state` is [`EntryState::Created`].
    ///
    /// `receipt` cannot be given a value here: it is `None` and only the registry's checked
    /// receipt writers can change that.
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
            artifact_dir,
            manifest_sha256: manifest_sha256.to_string(),
            // Not a constructor argument: the directory digest is taken by the control
            // handler that also renders the tracker files out of that directory, and a
            // second place to pass it is a second place to pass the wrong one.
            connector_dir_sha256: None,
            harness_version: harness_version.to_string(),
            state: EntryState::Created,
            health: HealthState::Online,
            receipt: None,
            receipt_binding,
            chain_seq: None,
            chain_head: GENESIS.to_string(),
            last_ack_seq: None,
            terminal_drain_validation_failed: false,
            sealed_at: None,
            events_uploaded: 0,
            expected_config: BTreeMap::new(),
            expected_process_handoff,
            process_handoff_generation: 0,
            process_handoff_phase: ProcessHandoffPhase::InitialClosed,
            process_handoff_digest: None,
            prepared_process_handle: None,
            cancelled_process_handoff_digest: None,
            last_process: None,
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
        self.put_letting_go(entry, &|_| false)
    }

    /// [`Registry::put`], except that an entry `lets_go` says is finished with its workspace no
    /// longer holds it. A run that is continued (`goalcup continue`) makes its next segment, a
    /// new entry, in the workspace its last one had: the last one is sealed, or this daemon
    /// could not reopen it, and routes nothing.
    pub fn put_letting_go(
        &self,
        entry: &Entry,
        lets_go: &dyn Fn(&Entry) -> bool,
    ) -> Result<(), DaemonError> {
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
                    to_write.cancelled_process_handoff_digest =
                        stored.cancelled_process_handoff_digest;
                    // Terminal-drain validation is capture-closed evidence. A broad row
                    // replacement cannot erase an elevated-review outcome; only the
                    // dedicated monotonic validation transaction may set it.
                    to_write.terminal_drain_validation_failed =
                        stored.terminal_drain_validation_failed;
                    // `last_process` is the public, termination-facing half of the
                    // accepted handoff.  A broad `put` must not replace it independently
                    // of the private authority fields above.
                    to_write.last_process = stored.last_process;
                } else if stored.workspace == entry.workspace && !lets_go(&stored) {
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
            let cancelled_process_handoff_digest = e.cancelled_process_handoff_digest.clone();
            let terminal_drain_validation_failed = e.terminal_drain_validation_failed;
            let last_process = e.last_process.clone();
            f(e);
            e.receipt = verified;
            e.receipt_binding = receipt_binding;
            e.expected_process_handoff = expected_process_handoff;
            e.process_handoff_generation = process_handoff_generation;
            e.process_handoff_phase = process_handoff_phase;
            e.process_handoff_digest = process_handoff_digest;
            e.prepared_process_handle = prepared_process_handle;
            e.cancelled_process_handoff_digest = cancelled_process_handoff_digest;
            e.terminal_drain_validation_failed = terminal_drain_validation_failed;
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
    /// holds only the termination handle.
    pub fn prepare_process_handoff(
        &self,
        entry_id: &str,
        registration: ProcessRegistration,
    ) -> Result<Entry, DaemonError> {
        let digest = registration.canonical_handle_digest(entry_id);
        let handle = registration.handle.clone();
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
                    entry.process_handoff_phase = ProcessHandoffPhase::Prepared;
                    entry.process_handoff_digest = Some(digest.clone());
                    entry.prepared_process_handle = Some(handle.clone());
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
            Ok(())
        })
    }

    /// Clone the exact handle a runtime is about to reconcile. The registry validates it again
    /// in the promotion transaction so neither replay nor restart can install stale authority.
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
