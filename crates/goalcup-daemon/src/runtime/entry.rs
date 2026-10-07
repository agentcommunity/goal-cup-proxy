use super::{Cadence, CycleCounters};
use crate::arming::{ArmingGate, GateState};
use crate::buffer::{
    Buffer, OutOfWindowAdmissionV1, OutOfWindowBatchApplyV1, OutOfWindowBatchValidationV1,
    OutOfWindowLimitsV1,
};
use crate::chain_writer::ChainWriter;
use crate::config::DaemonConfig;
use crate::configwatch::ConfigWatch;
use crate::cursor::CursorState;
use crate::foreign::{
    ForeignDeclarationKey, ForeignScanPlan, ForeignSighting, MAX_FOREIGN_SIGHTINGS,
};
use crate::health::{self, HealthInputs, HealthPolicy};
use crate::heartbeat::Heartbeat;
use crate::partition::{self, DiscardReason, Outcome, Partitioner, str_at};
use crate::paths::PathVars;
use crate::receipt::{self, ReceiptClient, record_start};
use crate::redact::Redactor;
use crate::registry::{
    AdmissionWatermarksV1, ChainPositionV1, Entry, EntryState, MAX_FOREIGN_PENDING_SIGHTINGS,
    ProcessHandoffPhase, Registry, SealOutcome, SealingIntent, StartPathIdentity,
};
use crate::status::{self, StatusInputs};
use crate::tailer::{
    self, BoundTailer, CapturedRecord, DeclaredBinding, SourceBinding, SourceKind, TailBatch,
    TailNotice,
};
use crate::upload::{PumpOutcome, UploadClient, Uploader};
use crate::workspace::{AllowlistedFile, TrackerFile};
use crate::{
    DaemonError, MAX_FILE_BYTES, MAX_LINE_BYTES, MAX_POLL_BYTES, now_rfc3339, now_unix,
    unix_to_rfc3339,
};
use goalcup_platform::tracecap::{EntryArtifactRoot, TraceCapError};
use goalcup_protocol::control::{EventStatus, SignedStatus, StatusPayload};
use goalcup_protocol::cursor::{CursorUnit, StoreCursor};
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::events::{
    ForeignScanOutcome, HealthEvent, HealthState, HomeSeedRecord, VersionChange,
};
use goalcup_protocol::export::{
    CHAIN_FILE, FixedTraceDocument, PreparedTraceResume, TraceDescriptorDoc, TraceExport,
    TraceRoot, manifest_sha256,
};
use goalcup_protocol::manifest::{
    ConnectorManifest, ForeignScanRoot, LaunchMode, PartitionSpec, Store,
};
use goalcup_protocol::process::ProcessRegistration;
use goalcup_protocol::provenance::{PreCutoffDrainFailureV1, PreCutoffDrainOutcomeV1};
use goalcup_protocol::receipt::{StartReceipt, StartReceiptClaims};
use goalcup_protocol::seal::SealManifest;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::PathBuf;
#[cfg(not(any(test, feature = "test-hooks")))]
use std::sync::Arc;
#[cfg(any(test, feature = "test-hooks"))]
use std::sync::{Arc, Condvar, Mutex};
#[cfg(any(test, feature = "test-hooks"))]
use std::time::Duration;

/// How many tailer passes one tick may make before it yields, whatever the stores still
/// hold.
///
/// Sixty-four passes at [`crate::MAX_POLL_BYTES`] is 64 MiB of a 700 MB rollout per tick,
/// so a large backlog is caught up in seconds rather than in one unbounded call that
/// starves the uploader, the heartbeat and the deadline check. The two failures this number
/// sits between are both real: a single-pass tick reads one 1 MiB window per store per
/// 250 ms, which is 4 MiB a second and cannot keep up with a harness writing faster than
/// that; an unbounded loop hands one busy store the whole tick.
pub const MAX_POLLS_PER_TICK: usize = 64;

/// One live admission drain and one post-capture sweep may each consume the ordinary poll
/// bound. Recovery reuses already-committed transitions and fails closed rather than growing
/// this journal without limit.
const MAX_OOW_TRANSITIONS_PER_STORE: u64 = (MAX_POLLS_PER_TICK as u64) * 2;

/// Restart may inspect at most one ordinary poll window while rebuilding all
/// `FileFirstRecord` decisions. Exhaustion restores no cursor for the affected store, so
/// the existing witnessed, per-tick bounded reread path finishes the work without silently
/// classifying or dropping a file at this startup bound.
const MAX_PARTITION_RECONSTRUCTION_BYTES: u64 = MAX_POLL_BYTES;

/// A modified cursor document cannot make startup walk an unbounded path map. Like the byte
/// bound, exhaustion falls back to an ordinary visible store reread.
const MAX_PARTITION_RECONSTRUCTION_FILES: usize = 4_096;

/// The pseudo-store a heartbeat reports the write-ahead position under.
///
/// `gaps::cursor_for` looks a cursor up by store name and ignores every name it did not
/// ask for, so this rides along beside the real stores without displacing one. It is only
/// added when no declared store has taken the name.
const BUFFER_CURSOR: &str = "buffer";

/// `ForeignSession` predates the internal family discriminator and keeps the wire schema
/// unchanged. Rebuilt chain evidence therefore uses this impossible-on-the-wire sentinel
/// until a fresh scan supplies the trusted family again.
const UNKNOWN_FOREIGN_FAMILY: &str = "";

/// The largest number of same-tick foreign sightings the scanner can return. A failed
/// chain-to-buffer append retains this many identity-only repair markers at most, so a live
/// runtime can repair the exact committed chain events without rescanning the source files.
const MAX_FOREIGN_BUFFER_REPAIR_MARKERS: usize = MAX_FOREIGN_SIGHTINGS;

/// Test-only pause between writing the terminal link and finishing its registry promotion.
/// It is installed on one [`EntryRuntime`] rather than keyed by a display id in global
/// process state: two daemons may both own an `e1`, but neither may pause the other's
/// retained writer or artifact capability.
#[cfg(any(test, feature = "test-hooks"))]
pub struct LiveSealWritePause {
    state: LiveSealWritePauseState,
}

#[cfg(any(test, feature = "test-hooks"))]
#[derive(Clone)]
pub(super) struct LiveSealWritePauseState {
    state: Arc<(Mutex<LiveSealWritePauseStateInner>, Condvar)>,
}

#[cfg(any(test, feature = "test-hooks"))]
#[derive(Default)]
struct LiveSealWritePauseStateInner {
    paused: bool,
    released: bool,
}

/// The workspace-capture pause is reached about half a second after a finish starts (the
/// finalizer first terminates the harness), so the bound leaves a loaded machine ample room.
/// A test that expects a pause never to come waits the full bound.
#[cfg(any(test, feature = "test-hooks"))]
const LIVE_SEAL_WRITE_PAUSE_WAIT: Duration = Duration::from_secs(10);

#[cfg(any(test, feature = "test-hooks"))]
impl LiveSealWritePauseState {
    pub(super) fn new() -> Self {
        Self {
            state: Arc::new((
                Mutex::new(LiveSealWritePauseStateInner::default()),
                Condvar::new(),
            )),
        }
    }

    fn wait_until_paused(&self) -> Result<(), DaemonError> {
        let (lock, wake) = &*self.state;
        let state = lock.lock().unwrap_or_else(|e| e.into_inner());
        let (state, _) = wake
            .wait_timeout_while(state, LIVE_SEAL_WRITE_PAUSE_WAIT, |state| !state.paused)
            .unwrap_or_else(|e| e.into_inner());
        if state.paused {
            Ok(())
        } else {
            Err(DaemonError::Refused(
                "timed out waiting for live seal write pause".to_string(),
            ))
        }
    }

    fn release(&self) -> Result<(), DaemonError> {
        let (lock, wake) = &*self.state;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        if !state.paused {
            return Err(DaemonError::Refused(
                "live seal write pause is not currently paused".to_string(),
            ));
        }
        state.released = true;
        wake.notify_all();
        Ok(())
    }

    fn release_on_drop(&self) {
        let (lock, wake) = &*self.state;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.released = true;
        wake.notify_all();
    }

    pub(super) fn pause_after_write(&self) {
        let (lock, wake) = &*self.state;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.paused = true;
        wake.notify_all();
        while !state.released {
            state = wake.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }
}

#[cfg(any(test, feature = "test-hooks"))]
impl LiveSealWritePause {
    pub(super) fn new() -> (Self, LiveSealWritePauseState) {
        let state = LiveSealWritePauseState::new();
        (
            Self {
                state: state.clone(),
            },
            state,
        )
    }

    /// Wait for this exact retained runtime to reach its post-terminal pause. The bounded
    /// Condvar wait is diagnostic-only: successful tests are signalled by the writer, while
    /// a failed writer reports the missing successful terminal instead of hanging a worker.
    pub fn wait_until_paused(&self) -> Result<(), DaemonError> {
        self.state.wait_until_paused()
    }

    /// Release this exact paused runtime.  Releasing before its writer reaches the pause is
    /// a test fixture error rather than a latent permission to skip the containment window.
    pub fn release(&self) -> Result<(), DaemonError> {
        self.state.release()
    }
}

#[cfg(any(test, feature = "test-hooks"))]
impl Drop for LiveSealWritePause {
    fn drop(&mut self) {
        // The test often performs filesystem setup between `wait_until_paused` and its
        // explicit `release`. If that setup panics, this guard still releases the retained
        // sealing worker rather than leaving a test process with a permanently blocked task.
        // Under `std::thread::scope` the guard belongs inside the scope closure: the scope
        // joins its workers before a panic leaves it, so a guard declared outside is never
        // dropped and the join waits forever.
        self.state.release_on_drop();
    }
}

/// How many files [`EntryRuntime::sessions_by_path`] may remember.
///
/// One entry per tailed file that named a session, held so a record carrying no session id
/// is still attributed to the file's. A store like Claude Code's `{config_dir}/projects/
/// */*.jsonl` is a glob, so without a bound this grows with every file the glob has ever
/// matched, for the life of the run.
///
/// **Eviction first**: an entry whose file is gone is dropped once a tick, which is what
/// keeps the map the size of the store rather than the size of its history. The cap is the
/// backstop, and going over it costs nothing that is not recoverable — a record that names
/// its own session still answers from the record, and one that does not falls back to the
/// entry's bound session, which is what a first record of a file does.
const MAX_SESSIONS_BY_PATH: usize = 4_096;

/// The number of declarations whose first clean EOF a live entry may hold pending.
/// Exhaustion is fail-closed: the next eligible declaration emits its current identity
/// unavailable outcome rather than retaining another key.
const MAX_FOREIGN_IDENTITY_PENDING_DECLARATIONS: usize = 4_096;

/// What one tick did.
#[derive(Debug, Clone, PartialEq)]
pub struct PollReport {
    /// Records that passed the partitioner and reached the chain. Every pass of the drain
    /// counts into this, not just the first.
    pub captured: u64,
    /// Records the partitioner refused. A count is all a discard leaves behind (D20).
    pub discarded: u64,
    pub deviations: usize,
    pub arming: GateState,
    pub health: HealthState,
    pub notices: Vec<TailNotice>,
    /// The primary health state differs from the one the previous tick reported.
    pub health_changed: bool,
    /// Envelopes appended to the chain this tick — records with any inline redaction audit,
    /// notices, deviations, heartbeats and chained health.
    pub chained: u64,
    /// How many times **in this run** the drain stopped at [`MAX_POLLS_PER_TICK`] with a
    /// store still holding more.
    ///
    /// Cumulative and not per-tick: one capped tick is a busy moment and a hundred of them
    /// is a daemon that has been behind since the run started, and a supervisor watching a
    /// boolean that clears on the next quiet tick cannot tell the two apart. Nothing in the
    /// chain says a store is behind — a capped tick looks exactly like a quiet one from the
    /// outside — so this is the only place the fact surfaces.
    pub polls_capped: u64,
    /// How many stores began this run at offset 0 because their saved position did not come
    /// back (see [`EntryRuntime::refresh_descriptor`]'s neighbour,
    /// `chain_cursor_rereads`). Cumulative, and on a resumed entry it equals the number of
    /// `Reread` witnesses that were chained before the first poll.
    pub cursor_restore_failures: u64,
    /// How many records [`EntryRuntime::resume_skip_taken`] has spent the restart budget on.
    /// Cumulative, for the same reason `cursor_restore_failures` is: the budget is
    /// bookkeeping nothing else states, and a budget computed too wide is invisible from
    /// every other counter here — a record it eats silently is neither `captured` nor
    /// `discarded`. This is the one place that residue surfaces.
    pub skipped_replayed: u64,
    /// How many polls stopped at [`crate::tailer::dir::MAX_SEEN_FILES`] with files this run
    /// had not captured still in front of them.
    ///
    /// Cumulative, like the three above. The tailer's own report of it is a
    /// `TailNotice::StoreUnavailable`, which chains nothing and which a reader cannot tell
    /// from a store the daemon could not open at all — so without this, every file past the
    /// bound is evidence that will not exist and nothing anywhere says so.
    pub files_capped: u64,
}

/// The three counters one tick accumulates before there is a [`PollReport`] to put them in.
///
/// Locals rather than a half-built report: `arming` and `health` are read at the end of the
/// tick — after this tick's records have reached `ArmingGate::observe_leg1` and after the
/// ladder has run — and a report constructed at the top would have to state each of them
/// twice, once from a value nothing had computed yet.
#[derive(Debug, Default)]
struct Counts {
    captured: u64,
    discarded: u64,
    chained: u64,
}

enum Construction {
    Fresh(SealManifest),
    Rebuilt,
}

/// Chainless recovery owner for the one post-cutoff operation the ordinary recorder must
/// never perform: restore the exact admission cursors and retain unread tailer records as
/// encrypted OOW evidence. The terminal TraceExport remains owned by the finalization
/// reconciler in `seal.rs`.
pub(super) struct RecoveryTailerCutover {
    entry_id: String,
    manifest: ConnectorManifest,
    tailers: Vec<BoundTailer>,
    bindings: BTreeMap<String, SourceBinding>,
    partitioners: BTreeMap<String, Partitioner>,
    sessions_by_path: BTreeMap<String, String>,
    bound_session: Option<String>,
}

impl RecoveryTailerCutover {
    pub(super) fn open(
        entry: &Entry,
        manifest: ConnectorManifest,
        vars: PathVars,
        artifact: &EntryArtifactRoot,
        buffer: &Buffer,
    ) -> Result<Self, DaemonError> {
        let mut source_vars = vars;
        source_vars.session_id = entry.session_id.clone();
        let declared = tailer::binding_roster(&manifest)?;
        let parent_order: Vec<String> = poll_order(&manifest.store)
            .into_iter()
            .map(|store| store.name.clone())
            .collect();
        let mut tailers =
            tailer::build_bound_tailers(&declared, &manifest, &source_vars, &parent_order)?;
        let saved = load_finalization_cursors_strict(artifact)?;
        if saved.len() != tailers.len() {
            return Err(DaemonError::Refused(format!(
                "entry {} finalization cursor roster does not match the shipped tailers",
                entry.entry_id
            )));
        }
        for bound in &mut tailers {
            let name = bound.binding.name();
            let cursor = saved.get(name).ok_or_else(|| {
                DaemonError::Refused(format!(
                    "entry {} has no finalization cursor for {name}",
                    entry.entry_id
                ))
            })?;
            bound
                .reader
                .restore(cursor.cursor.clone())
                .map_err(|error| {
                    DaemonError::Refused(format!(
                        "entry {} cannot restore finalization cursor {name}: {error}",
                        entry.entry_id
                    ))
                })?;
        }
        let intent = entry.workspace_sealing_intent().ok_or_else(|| {
            DaemonError::Refused(format!(
                "entry {} has no workspace finalization watermarks",
                entry.entry_id
            ))
        })?;
        let current = unique_cursor_map(
            &tailers
                .iter()
                .map(|bound| bound.reader.cursor().to_store_cursor(bound.binding.name()))
                .collect::<Vec<_>>(),
        )?;
        let persisted = unique_cursor_map(&intent.admission_watermarks.tailers)?;
        if current != persisted {
            return Err(DaemonError::Refused(format!(
                "entry {} recovery tailers do not match the persisted admission watermarks",
                entry.entry_id
            )));
        }
        initialize_out_of_window_progress(buffer, &manifest, &tailers)?;
        let mut recovered = saved.clone();
        for bound in &mut tailers {
            let coordinate = out_of_window_source_coordinate(&manifest, &bound.binding)?;
            let cursor: CursorState =
                serde_json::from_slice(&buffer.out_of_window_current_cursor(&coordinate)?)?;
            bound.reader.restore(cursor.clone()).map_err(|error| {
                DaemonError::Refused(format!(
                    "entry {} cannot restore authenticated OOW cursor {}: {error}",
                    entry.entry_id,
                    bound.binding.name()
                ))
            })?;
            recovered
                .get_mut(bound.binding.name())
                .ok_or_else(|| {
                    DaemonError::Refused(format!(
                        "entry {} lost finalization cursor {} during recovery",
                        entry.entry_id,
                        bound.binding.name()
                    ))
                })?
                .cursor = cursor;
        }
        let BindingState {
            bindings,
            mut partitioners,
        } = binding_state(
            &tailers,
            &manifest,
            &entry.workspace,
            entry.session_id.as_deref(),
        )?;
        let reconstruction = reconstruct_file_first_decisions(
            &bindings,
            &manifest.store,
            &mut partitioners,
            &recovered,
            &source_vars,
            file_session_id_field(&manifest).as_deref(),
        );
        if !reconstruction.unsafe_restores.is_empty() {
            return Err(DaemonError::Refused(format!(
                "entry {} cannot reconstruct finalization partition authority for {:?}",
                entry.entry_id, reconstruction.unsafe_restores
            )));
        }
        Ok(Self {
            entry_id: entry.entry_id.clone(),
            manifest,
            tailers,
            bindings,
            partitioners,
            sessions_by_path: reconstruction.sessions_by_path,
            bound_session: entry.session_id.clone(),
        })
    }

    pub(super) fn retain<E>(
        &mut self,
        buffer: &Buffer,
        limits: OutOfWindowLimitsV1,
        mut expired: E,
    ) -> Result<(), DaemonError>
    where
        E: FnMut() -> bool,
    {
        let mut complete = true;
        let mut first_error = None;
        for _ in 0..MAX_POLLS_PER_TICK {
            if expired() {
                return Err(DaemonError::Refused(format!(
                    "entry {} recovery tailer cutover exceeded its bound",
                    self.entry_id
                )));
            }
            let mut more = false;
            for bound in &mut self.tailers {
                let before = bound.reader.cursor();
                let batch = match bound.reader.poll() {
                    Ok(batch) => batch,
                    Err(error) => {
                        complete = false;
                        first_error.get_or_insert(error);
                        continue;
                    }
                };
                more |= batch.more_available;
                let processed = process_out_of_window_batch(
                    &self.entry_id,
                    &batch,
                    &self.bindings,
                    &mut self.partitioners,
                    &self.manifest,
                    &mut self.sessions_by_path,
                    self.bound_session.as_deref(),
                    true,
                );
                complete &= processed.complete;
                if before == bound.reader.cursor()
                    && processed.canonical_records.is_empty()
                    && processed.validation == OutOfWindowBatchValidationV1::CLEAN
                {
                    continue;
                }
                let apply_result = apply_out_of_window_batch(
                    buffer,
                    &self.manifest,
                    bound,
                    &before,
                    &processed,
                    limits,
                );
                if first_error.is_none() {
                    first_error = processed.first_error;
                }
                match apply_result {
                    Ok(batch_complete) => complete &= batch_complete,
                    Err(error) => {
                        complete = false;
                        first_error.get_or_insert(error);
                    }
                }
            }
            if !more {
                if let Some(error) = first_error {
                    return Err(error);
                }
                return if complete {
                    Ok(())
                } else {
                    Err(DaemonError::Refused(format!(
                        "entry {} recovery tailer cutover was incomplete",
                        self.entry_id
                    )))
                };
            }
        }
        Err(DaemonError::Refused(format!(
            "entry {} recovery tailer cutover exceeded its poll cap",
            self.entry_id
        )))
    }
}

fn unique_cursor_map(
    cursors: &[StoreCursor],
) -> Result<BTreeMap<String, StoreCursor>, DaemonError> {
    let mut map = BTreeMap::new();
    for cursor in cursors {
        if map.insert(cursor.store.clone(), cursor.clone()).is_some() {
            return Err(DaemonError::Refused(format!(
                "duplicate finalization tailer watermark for {}",
                cursor.store
            )));
        }
    }
    Ok(map)
}

/// The largest single record the out-of-window partition takes. The limit is 8 MiB so that
/// the limits a finalization intent already persisted and the limits a new one derives are
/// the same.
const MAX_OUT_OF_WINDOW_RECORD_BYTES: u64 = 8 << 20;

pub(super) fn workspace_out_of_window_limits(
    intent: &crate::registry::WorkspaceSealingIntentV1,
) -> OutOfWindowLimitsV1 {
    OutOfWindowLimitsV1 {
        max_records: intent.limits.object_count,
        max_record_bytes: intent
            .limits
            .total_bytes
            .min(MAX_OUT_OF_WINDOW_RECORD_BYTES),
        max_total_bytes: intent.limits.total_bytes,
    }
}

/// A heartbeat may see a producer between valid JSONL records; a seal cannot leave that
/// observation unresolved because it closes the evidence chain immediately afterward.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ForeignSweepPhase {
    LiveHeartbeat,
    Seal,
}

enum WriterState {
    FreshPending,
    ResumePending(ChainWriter),
    Open(ChainWriter),
    Failed,
}

enum ReceiptMirrorState {
    NoAuthoritativeStart,
    Ready,
    Owed {
        receipt: Box<StartReceipt>,
        compact_bytes: Vec<u8>,
        mirror_ready: bool,
    },
}

/// The fallible part of adding distinct session-scoped readers. It is built before a start
/// becomes durable, then moved into the live runtime without another fallible operation.
struct DerivedActivation {
    vars: PathVars,
    tailers: Vec<BoundTailer>,
    bindings: BTreeMap<String, SourceBinding>,
    partitioners: BTreeMap<String, Partitioner>,
}

/// The explicit active authority and the independent state that admits records under it.
struct BindingState {
    bindings: BTreeMap<String, SourceBinding>,
    partitioners: BTreeMap<String, Partitioner>,
}

/// The exact chain envelope retained after its append succeeds but its buffer append fails.
/// The bounded identity is kept alongside it for seen-set reconciliation; live retries never
/// need to reopen or stream the chain to recover this event.
#[derive(Debug, Clone)]
struct ForeignBufferRepair {
    sighting: ForeignSighting,
    envelope: Envelope,
    keep_until_pending: bool,
}

/// One entry's tailers, partitioners, writer, buffer, uploader, config watch and gate.
pub struct EntryRuntime {
    entry: Entry,
    manifest: ConnectorManifest,
    registry: Arc<Registry>,
    /// The connector's declared stores, in manifest order. `partition::teach` reads them.
    stores: Vec<Store>,
    /// Fixed manifest declarations, including inactive session-scoped children.
    declared_bindings: Vec<DeclaredBinding>,
    /// Exact active source authority, keyed by the name chained on every emitted record.
    bindings: BTreeMap<String, SourceBinding>,
    /// The readers, in poll order: every store another store learns from comes first.
    tailers: Vec<BoundTailer>,
    partitioners: BTreeMap<String, Partitioner>,
    /// Construction intent and the one writer capability for this in-memory runtime.
    /// A failed opening is terminal, so a retry cannot reacquire the trace path or append a
    /// second restart after a partial first attempt.
    writer: WriterState,
    /// The one retained `configured-root/artifacts/entry` capability for every artifact
    /// operation this runtime performs. Registry paths are diagnostics, never reopen keys.
    artifact: EntryArtifactRoot,
    buffer: Arc<Buffer>,
    uploader: Uploader,
    watch: ConfigWatch,
    gate: ArmingGate,
    heartbeat: Heartbeat,
    receipts: ReceiptClient,
    policy: HealthPolicy,
    vars: PathVars,
    /// The workspace as [`std::fs::canonicalize`] returns it. **Every** tracked-value seed
    /// and the foreign scan's `directories` use this and not `entry.workspace`: the
    /// partitioner compares byte for byte, and on macOS a tempdir handed out as
    /// `/var/folders/…` is written by the harness as `/private/var/folders/…`.
    workspace: PathBuf,
    connector_dir: PathBuf,
    /// The exclusion union this entry's seal applies: the connector's
    /// `workspace_exclusions[]` plus every workspace-scoped tracker file, as
    /// [`crate::workspace::exclusion_union`] computed it over the files this runtime was
    /// built with.
    ///
    /// Held rather than recomputed at seal time from whatever is on disk, and rebuilt on a
    /// restart from the manifest and the rendered bytes rather than read back out of
    /// `baseline.json`: the union is what the walk applied, and `SealManifest.excluded`
    /// that carries a subset of it would make the tracker's own files read as
    /// unexplained sealed files, or as a silent hole, depending on which list a reader
    /// takes.
    excluded: Vec<String>,
    /// The connector manifest's name **inside the trace directory**, which is what
    /// `TraceDescriptorDoc::connector_manifest` states. `ChainWriter::copy_manifest` puts
    /// the file there and returns this name; a resumed run finds the copy the first run
    /// made, under the same constant.
    connector_manifest: String,
    baseline: SealManifest,
    pinned_server_key: String,
    event: EventStatus,

    // ---- what the health ladder is told, gathered as it is observed ----
    health: HealthState,
    /// Every state the previous evaluation asserted, so only a transition is chained.
    /// A `Vec` and not a set: `HealthState` is `PartialEq` and deliberately not `Ord` — the
    /// ladder's priority is not an ordering on the enum — and the list is at most three long.
    health_states: Vec<HealthState>,
    last_leg1_at: Option<f64>,
    /// Whether an app-wide file the tracker may not restore — or a shim reporting a stale
    /// pin — is deviating **now**. Recomputed on every poll from
    /// [`ConfigWatch::mismatched`], which is a comparison of digests and not a judgement
    /// about who moved them.
    ///
    /// Not latched: a file that matches its digest again stops reporting. The flag knows the
    /// bytes are the ones it recorded at launch, which is all it claims, and it does not have
    /// to know who moved them. The deviation itself is chained when it happens and is not
    /// withdrawn; this is the live state beside it. A file the tracker did write is put back,
    /// which is `config_restored` and is per-tick.
    config_mismatch: bool,
    arming_lost: bool,
    version_changed: bool,
    connected: bool,
    backing_off: bool,
    ack_unverified: bool,
    /// The last pump's `ReadChunk.gap_at`, held because a hole in the
    /// write-ahead log is permanent and a health surface polling through an outage must not
    /// watch it blink.
    buffer_gap_at: Option<u64>,
    clock_skew_seconds: Option<f64>,
    /// The terminal validator found a refusal, but its monotonic registry quarantine could
    /// not commit. The daemon uses this transient fact to withhold stale sealed status until
    /// a later validation either persists review or proves a clean terminal again.
    terminal_validation_persistence_failed: bool,
    /// `Some` only on the runtime that ran this entry's admission CAS, and then always the
    /// empty map. It is the "admission closed in this process" marker
    /// [`EntryRuntime::drain_finalization_admitted`] compares against the persisted intent;
    /// after a restart it is `None`, and the drain resolves as `RestartStateAmbiguous`.
    finalization_receiver_sources: Option<BTreeMap<String, u64>>,
    /// Test-only corruption staged after a real uploader return.  It exists solely to
    /// exercise the final sealed decision, rather than accidentally testing pump's
    /// pre-upload ceiling validation.
    #[cfg(any(test, feature = "test-hooks"))]
    post_pump_tail_for_test: Option<Envelope>,
    /// Test-only interposition immediately after a foreign chain append and before its
    /// write-ahead buffer append. This models the actual crash gap without changing the
    /// production ordering.
    #[cfg(any(test, feature = "test-hooks"))]
    fail_foreign_buffer_after_chain_for_test: bool,
    /// Test-only failures of the direct buffer repair retry, so repeated live retries can be
    /// proved to retain their exact envelope and marker.
    #[cfg(any(test, feature = "test-hooks"))]
    fail_foreign_buffer_repair_for_test: u32,
    /// Test-only count of paths that stream the chain to recover foreign envelopes.
    #[cfg(any(test, feature = "test-hooks"))]
    foreign_chain_scan_count_for_test: u64,
    /// The optional deterministic live-seal pause belongs to this runtime's retained
    /// writer and entry capability. It is absent from shipped builds.
    #[cfg(any(test, feature = "test-hooks"))]
    live_seal_write_pause: Option<LiveSealWritePauseState>,
    /// Deterministic proof that V4 capture/readback does not retain the slot mutex.
    #[cfg(any(test, feature = "test-hooks"))]
    workspace_capture_pause: Option<LiveSealWritePauseState>,
    #[cfg(any(test, feature = "test-hooks"))]
    workspace_intent_cas_pause: Option<LiveSealWritePauseState>,
    #[cfg(any(test, feature = "test-hooks"))]
    fail_before_workspace_intent_cas: bool,
    /// Set by [`EntryRuntime::fail_workspace_append_after_for_test`]; the finalization drain
    /// clears the writer fault it armed once the drain returns.
    #[cfg(any(test, feature = "test-hooks"))]
    clear_workspace_append_failure_after_drain: bool,
    /// Set by a resumed [`EntryRuntime::open_trace`] and never cleared: the run was
    /// interrupted, and a later quiet tick does not make that untrue.
    tracker_restarted: bool,
    /// The last session id each tailed file named, so a record that carries none is still
    /// attributed to the session its file belongs to (Codex names the session once, in the
    /// `session_meta` its file opens with).
    sessions_by_path: BTreeMap<String, String>,
    /// `(family, harness, session)` identities already chained as `EventBody::ForeignSession`, so a
    /// standing second session is one event and not one per sweep.
    foreign_seen: BTreeSet<(String, String, String)>,
    /// Same-harness identities found under an own-config root before this run is bound. The
    /// source may disappear before binding, so retain only the bounded identity projection
    /// needed to reconcile it later; no transcript bytes are held.
    foreign_pending: BTreeMap<(String, String, String), crate::foreign::ForeignSighting>,
    /// Immediate foreign sightings whose chain event committed before their buffer append
    /// failed. The source can disappear before the next tick, so retain the bounded exact
    /// envelope and identity projection needed to repair it without rescanning.
    foreign_buffer_repair_needed: BTreeMap<(String, String, String), ForeignBufferRepair>,
    foreign_plan: ForeignScanPlan,
    foreign_coverage_seen: BTreeMap<ForeignDeclarationKey, ForeignScanOutcome>,
    /// This set holds at most [`MAX_FOREIGN_IDENTITY_PENDING_DECLARATIONS`] declaration
    /// keys and retains neither a source path nor transcript bytes. A fresh runtime's
    /// eligible clean EOF enters it on the first heartbeat and becomes stable on the next
    /// eligible sweep.
    foreign_identity_pending: BTreeSet<ForeignDeclarationKey>,
    /// Only a genuinely fresh runtime may defer a clean EOF identity once. A reconstructed
    /// runtime has no durable pending state to recover, so it emits the existing closed
    /// unavailable coverage rather than granting another grace observation.
    foreign_identity_live_grace: bool,

    /// The trace mirror state for this entry's registry-authoritative start receipt.
    ///
    /// `write_receipt` puts the document the verifier reads beside the chain head it
    /// witnessed. It is a file write and it can fail — a full disk, a directory the
    /// participant moved — and when it does the receipt is already recorded in the registry
    /// and the row already `Started`: the run goes on and the write is retried.
    ///
    /// `NoAuthoritativeStart`, `Ready`, and `Owed` keep those facts distinct across fresh
    /// construction and restart. [`EntryRuntime::descriptor_doc`] names `receipt.json`
    /// only in `Ready`.
    receipt_mirror: ReceiptMirrorState,

    /// The next [`ChainWriter::write_receipt`] fails, so the retry above is reachable from
    /// a test. Producing this for real means a trace directory that is unwritable for one
    /// call and writable for the next.
    #[cfg(any(test, feature = "test-hooks"))]
    fail_next_receipt_write: bool,

    /// The stores whose saved position did not come back, in poll order, drained by
    /// [`EntryRuntime::open_trace`]. On a fresh entry this is every store and means
    /// nothing; on a resume each name is a store about to re-read a file it has already
    /// shipped, and `chain_cursor_rereads` turns each one into a witness (D19).
    unrestored_stores: Vec<String>,
    cursor_restore_failures: u64,
    polls_capped: u64,
    /// Mirrors [`PollReport::files_capped`]. Incremented once per poll whose batch came
    /// back with `TailBatch::files_capped` set.
    files_capped: u64,
    /// Mirrors [`PollReport::skipped_replayed`]. Incremented once per record
    /// [`EntryRuntime::resume_skip_taken`] spends the budget on.
    skipped_replayed: u64,

    /// Per store, how many records this run must read and not chain because the previous
    /// process already chained them.
    ///
    /// `cursors.json` is written after a batch is chained, so a kill part-way through one
    /// leaves the cursor at the batch's start while `k` of its records are in the chain.
    /// The restored tailer re-reads all of them and emits nothing — it went back from no
    /// offset it knew about — so the chain would take `k` second copies with no `Reread`
    /// above them, which V9 reads as a Fail on a run that did nothing wrong.
    ///
    /// `k` is **counted, off the chain**: the number of this store's `Leg1Record`s above
    /// the saved seq. Counting is independent of payload shape: a new line identical to the
    /// last chained one is not mistaken for a replay.
    ///
    /// Spent on records the partitioner keeps, not on records read: a batch mixes kept and
    /// discarded rows, only the kept ones took a chain link, and skipping by raw index
    /// would land the budget on the wrong records. Bounded by the store count, and each
    /// entry is gone the moment it reaches zero.
    resume_skip: BTreeMap<String, usize>,

    /// `FileFirstRecord` stores whose restart prefix resolved through a symbolic link or
    /// another non-regular source. Their original saved cursors are held so
    /// [`EntryRuntime::save_cursors`] cannot erase the evidence that makes the next
    /// process re-check the refusal. The ordinary tailer resolves paths by name and would
    /// otherwise follow the same unsafe ancestor that reconstruction rejected.
    blocked_file_first_sources: BTreeMap<String, crate::cursor::CursorState>,

    /// The digest of the `cursors.json` this runtime last wrote, so that "has anything
    /// moved?" costs no disk read. `None` until the first save. A digest and not
    /// the bytes: the document grows with `DirTailer`'s seen-set, and holding a second copy
    /// of it would trade a read for the memory bound.
    saved_cursor_digest: Option<String>,

    /// Per store, the seq of the last `Leg1Record` this entry has chained — what
    /// [`EntryRuntime::save_cursors`] writes beside each position. Seeded on a resume from
    /// the chain itself, so a process that chains nothing new does not save a seq behind
    /// the records already in the chain and make the next restart skip live records.
    last_leg1_seq: BTreeMap<String, u64>,

    /// The daemon's per-cadence failure counters, so the signed status can carry them.
    ///
    /// Its own until `Daemon::register` hands over the daemon's: a runtime driven by hand
    /// (the round-trip fixture, `tests/support/entry.rs`) has no loop around it and nothing
    /// to count, and a `None` here would make every status build ask a question with no
    /// answer.
    cycles: Arc<CycleCounters>,
}

impl EntryRuntime {
    /// See [`MAX_POLLS_PER_TICK`]; re-exported here so a caller holding a runtime does not
    /// have to name the module.
    pub const MAX_POLLS_PER_TICK: usize = MAX_POLLS_PER_TICK;

    /// Build the runtime for one entry. Opens the buffer and the config watch; does **not**
    /// open the chain, which is [`EntryRuntime::open_trace`]'s and has an order of its own.
    ///
    /// `tracker_files` and `allowlist` are the `Installation`'s — the bytes the tracker
    /// rendered from `hook_install[]`, not what happens to be on disk when this is called.
    /// A watch built from disk would adopt whatever an agent had already changed as the
    /// expected state and then report nothing for the rest of the run.
    ///
    /// **The registry is an argument** and not opened from `config.artifact_root`: redb
    /// takes a file lock, so a second `Registry::open` on the same path in the same process
    /// is a failure rather than a second handle, and the daemon already holds one.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        entry: Entry,
        manifest: ConnectorManifest,
        config: &DaemonConfig,
        vars: PathVars,
        tracker_files: Vec<TrackerFile>,
        allowlist: Vec<AllowlistedFile>,
        baseline: SealManifest,
        foreign_plan: ForeignScanPlan,
        registry: Arc<Registry>,
    ) -> Result<EntryRuntime, DaemonError> {
        Self::build(
            entry,
            manifest,
            config,
            vars,
            tracker_files,
            allowlist,
            Construction::Fresh(baseline),
            foreign_plan,
            registry,
            None,
        )
    }

    /// Construct a fresh runtime from the entry capability created by workspace setup.
    /// This is the launch path's one authority handoff: setup creates
    /// `root/artifacts/entry`, then the runtime retains that exact object for trace,
    /// buffer, cursors and status rather than resolving it again.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_artifact(
        entry: Entry,
        manifest: ConnectorManifest,
        config: &DaemonConfig,
        vars: PathVars,
        tracker_files: Vec<TrackerFile>,
        allowlist: Vec<AllowlistedFile>,
        baseline: SealManifest,
        foreign_plan: ForeignScanPlan,
        registry: Arc<Registry>,
        artifact: EntryArtifactRoot,
    ) -> Result<EntryRuntime, DaemonError> {
        Self::build(
            entry,
            manifest,
            config,
            vars,
            tracker_files,
            allowlist,
            Construction::Fresh(baseline),
            foreign_plan,
            registry,
            Some(artifact),
        )
    }

    /// Build a runtime from one coherent registry row and its existing trace. The trace is
    /// acquired once and completely validated against the row and baseline before the
    /// buffer, config watch, acknowledgement creation, or descriptor cleanup can occur.
    #[allow(clippy::too_many_arguments)]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn new_rebuilt(
        entry: Entry,
        manifest: ConnectorManifest,
        config: &DaemonConfig,
        vars: PathVars,
        tracker_files: Vec<TrackerFile>,
        allowlist: Vec<AllowlistedFile>,
        foreign_plan: ForeignScanPlan,
        registry: Arc<Registry>,
    ) -> Result<EntryRuntime, DaemonError> {
        Self::build(
            entry,
            manifest,
            config,
            vars,
            tracker_files,
            allowlist,
            Construction::Rebuilt,
            foreign_plan,
            registry,
            None,
        )
    }

    /// Production rebuilt constructor using the daemon-lifetime artifact authority.
    /// Tests that construct isolated runtimes may use [`Self::new_rebuilt`]; daemon restart
    /// must never reopen the configured artifact-root path after startup.
    #[allow(clippy::too_many_arguments)]
    pub fn new_rebuilt_with_artifact(
        entry: Entry,
        manifest: ConnectorManifest,
        config: &DaemonConfig,
        vars: PathVars,
        tracker_files: Vec<TrackerFile>,
        allowlist: Vec<AllowlistedFile>,
        foreign_plan: ForeignScanPlan,
        registry: Arc<Registry>,
        artifact: EntryArtifactRoot,
    ) -> Result<EntryRuntime, DaemonError> {
        Self::build(
            entry,
            manifest,
            config,
            vars,
            tracker_files,
            allowlist,
            Construction::Rebuilt,
            foreign_plan,
            registry,
            Some(artifact),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        entry: Entry,
        manifest: ConnectorManifest,
        config: &DaemonConfig,
        vars: PathVars,
        tracker_files: Vec<TrackerFile>,
        allowlist: Vec<AllowlistedFile>,
        construction: Construction,
        foreign_plan: ForeignScanPlan,
        registry: Arc<Registry>,
        acquired_artifact: Option<EntryArtifactRoot>,
    ) -> Result<EntryRuntime, DaemonError> {
        crate::registry::validate_foreign_pending(&entry)?;
        // Canonical, once, here. Everything downstream that compares a path compares this.
        let workspace =
            std::fs::canonicalize(&entry.workspace).unwrap_or_else(|_| entry.workspace.clone());
        let stores = manifest.store.clone();

        // The general path variables also render non-source declarations such as Codex's
        // launch-thread lock, which may legitimately carry a launch value before start.
        // Tailing authority is stricter: a fresh or unstarted rebuild must not turn that
        // value into a child reader. Only a rebuilt started row rehydrates its persisted,
        // corroborated session before child construction and cursor restoration.
        let mut source_vars = vars.clone();
        source_vars.session_id =
            if matches!(&construction, Construction::Rebuilt) && entry.started_at.is_some() {
                entry.session_id.clone()
            } else {
                None
            };

        let declared_bindings = tailer::binding_roster(&manifest)?;
        let parent_order: Vec<String> = poll_order(&stores)
            .into_iter()
            .map(|store| store.name.clone())
            .collect();
        let mut tailers = tailer::build_bound_tailers(
            &declared_bindings,
            &manifest,
            &source_vars,
            &parent_order,
        )?;
        let BindingState {
            bindings,
            mut partitioners,
        } = binding_state(
            &tailers,
            &manifest,
            &workspace,
            source_vars.session_id.as_deref(),
        )?;
        let artifact = match acquired_artifact {
            Some(artifact) => artifact,
            None => EntryArtifactRoot::open(&config.artifact_root, &entry.entry_id).map_err(
                |error| {
                    DaemonError::Refused(format!(
                        "entry {} artifact root cannot be acquired without following a swapped component: {error}",
                        entry.entry_id
                    ))
                },
            )?,
        };
        let saved_cursors = load_saved_cursors_from_artifact(&artifact)?;
        let reconstruction = reconstruct_file_first_decisions(
            &bindings,
            &stores,
            &mut partitioners,
            &saved_cursors,
            &source_vars,
            file_session_id_field(&manifest).as_deref(),
        );
        let Restored {
            unrestored_stores,
            saved_leg1_seq,
        } = restore_cursors(
            &mut tailers,
            &saved_cursors,
            &reconstruction.unsafe_restores,
        );

        if entry.foreign_pending.len() > MAX_FOREIGN_PENDING_SIGHTINGS {
            return Err(DaemonError::Refused(format!(
                "entry {} retains {} pending foreign identities, above the {}-identity bound",
                entry.entry_id,
                entry.foreign_pending.len(),
                MAX_FOREIGN_PENDING_SIGHTINGS
            )));
        }
        let foreign_pending = entry
            .foreign_pending
            .iter()
            .map(|pending| {
                if pending.family != manifest.family || pending.harness != manifest.id {
                    return Err(DaemonError::Refused(format!(
                        "entry {} retains pending foreign identity for {}::{}, not trusted active harness {}::{}",
                        entry.entry_id,
                        pending.family,
                        pending.harness,
                        manifest.family,
                        manifest.id
                    )));
                }
                let sighting = ForeignSighting::from_pending(pending);
                Ok((Self::foreign_sighting_key(&sighting), sighting))
            })
            .collect::<Result<BTreeMap<_, _>, DaemonError>>()?;
        let rebuilding = matches!(&construction, Construction::Rebuilt);
        let foreign_identity_live_grace = !rebuilding;
        let (baseline, writer, last_leg1_seq, resume_skip, receipt_mirror, foreign_seen) =
            match construction {
                Construction::Fresh(baseline) => (
                    baseline,
                    WriterState::FreshPending,
                    BTreeMap::new(),
                    BTreeMap::new(),
                    ReceiptMirrorState::NoAuthoritativeStart,
                    BTreeSet::new(),
                ),
                Construction::Rebuilt => {
                    let root = trace_root_from_artifact(&artifact, entry.trace_dir.clone())?;
                    let mut prepared = TraceExport::prepare_resume_from_root(root)?;
                    let facts = rebuilt_chain_facts(&entry, &saved_leg1_seq, &mut prepared)?;
                    let baseline_reader = prepared
                        .read_fixed(FixedTraceDocument::Baseline)?
                        .ok_or_else(|| {
                            DaemonError::Missing(format!(
                                "{} has no baseline.json for the chain's genesis commitment",
                                entry.trace_dir.display()
                            ))
                        })?;
                    let baseline: SealManifest = serde_json::from_reader(baseline_reader)?;
                    validate_rebuilt_baseline(&entry, &baseline, &facts)?;
                    let persisted = receipt::persisted_receipt(
                        &entry,
                        &facts.genesis_hash,
                        &config.server_public_key,
                    )?;
                    let receipt_mirror = rebuilt_receipt_mirror(&entry, persisted, &prepared)?;
                    let redactor = Redactor::for_connector(&manifest)?;
                    let writer =
                        ChainWriter::resume_prepared(entry.entry_id.clone(), prepared, redactor)?;
                    (
                        baseline,
                        WriterState::ResumePending(writer),
                        facts.last_leg1_seq,
                        facts.resume_skip,
                        receipt_mirror,
                        facts.foreign_seen,
                    )
                }
            };

        let buffer = Buffer::open_from_entry_artifact(&artifact, &entry.keys.buffer_key_hex)?;
        let uploader = Uploader::new(
            entry.entry_id.clone(),
            UploadClient::new(config.upload_url.clone()),
            config.upload_chunk_max,
            entry.keys.clone(),
            config.server_public_key.clone(),
            config.clock_skew_seconds,
        );
        // Through `Installation::from_files`, which is the one derivation of
        // `workspace_relative`, so the seal excludes exactly what `create_workspace`
        // excluded when it took the zero-entry baseline.
        let excluded = crate::workspace::exclusion_union(
            &manifest,
            &crate::workspace::Installation::from_files(tracker_files.clone()),
        );
        let watch = ConfigWatch::new(&tracker_files, &allowlist);
        // The arming-timeout precedence chain, stated once on `manifest::Arming`: the
        // connector's default, then the machine's, then this start's.
        let mut gate = ArmingGate::with_timeout(
            &manifest,
            entry.armed_at.unwrap_or_else(now_unix),
            entry
                .arming_timeout_seconds
                .or(config.arming_timeout_seconds),
        );
        // …and then told what the row already says, which on a fresh entry is nothing and
        // on a rebuilt one is the difference between a run that carries on and a run
        // whose first tick after a restart refuses it for an arming window that closed
        // hours ago. See [`ArmingGate::resume`].
        gate.resume(
            entry.start_signal_at,
            entry.session_id.clone(),
            entry.started_at.is_some(),
        );
        let heartbeat = Heartbeat::new(
            manifest
                .heartbeat_seconds
                .unwrap_or(config.heartbeat_seconds),
        );
        let policy = HealthPolicy {
            arming_timeout_seconds: gate.timeout_seconds() as f64,
            stall_after_seconds: config.stall_after_seconds,
            ack_lag_seconds: config.ack_lag_seconds,
            disk_reserve_bytes: config.disk_reserve_bytes,
        };
        let connector_dir = config.connectors_dir.join(&entry.connector_id);
        // The event's own dates belong to the round and come from the daemon config's `[event]`
        // section; until a construction site calls [`EntryRuntime::set_event_status`] this is the
        // entry's own timeline, which is what a participant reading the status cache before the
        // round opens has any right to be told. It has one definition, in `status.rs`.
        let event = crate::status::fallback_event_status(&entry);

        let armed_unstarted = entry.armed_at.is_some() && entry.started_at.is_none();
        let mut runtime = EntryRuntime {
            health: entry.health,
            health_states: Vec::new(),
            entry,
            manifest,
            registry,
            stores,
            declared_bindings,
            bindings,
            tailers,
            partitioners,
            writer,
            artifact,
            buffer: Arc::new(buffer),
            uploader,
            watch,
            gate,
            heartbeat,
            receipts: ReceiptClient::new(config.upload_url.clone()),
            policy,
            vars,
            workspace,
            connector_dir,
            excluded,
            connector_manifest: goalcup_protocol::export::MANIFEST_FILE.to_string(),
            baseline,
            pinned_server_key: config.server_public_key.clone(),
            event,
            last_leg1_at: None,
            config_mismatch: false,
            arming_lost: false,
            version_changed: false,
            connected: true,
            backing_off: false,
            ack_unverified: false,
            buffer_gap_at: None,
            clock_skew_seconds: None,
            terminal_validation_persistence_failed: false,
            finalization_receiver_sources: None,
            #[cfg(any(test, feature = "test-hooks"))]
            post_pump_tail_for_test: None,
            #[cfg(any(test, feature = "test-hooks"))]
            fail_foreign_buffer_after_chain_for_test: false,
            #[cfg(any(test, feature = "test-hooks"))]
            fail_foreign_buffer_repair_for_test: 0,
            #[cfg(any(test, feature = "test-hooks"))]
            foreign_chain_scan_count_for_test: 0,
            #[cfg(any(test, feature = "test-hooks"))]
            live_seal_write_pause: None,
            #[cfg(any(test, feature = "test-hooks"))]
            workspace_capture_pause: None,
            #[cfg(any(test, feature = "test-hooks"))]
            workspace_intent_cas_pause: None,
            #[cfg(any(test, feature = "test-hooks"))]
            fail_before_workspace_intent_cas: false,
            #[cfg(any(test, feature = "test-hooks"))]
            clear_workspace_append_failure_after_drain: false,
            tracker_restarted: false,
            sessions_by_path: reconstruction.sessions_by_path,
            foreign_seen,
            foreign_pending,
            foreign_buffer_repair_needed: BTreeMap::new(),
            foreign_plan,
            foreign_coverage_seen: BTreeMap::new(),
            foreign_identity_pending: BTreeSet::new(),
            foreign_identity_live_grace,
            receipt_mirror,
            #[cfg(any(test, feature = "test-hooks"))]
            fail_next_receipt_write: false,
            unrestored_stores,
            cursor_restore_failures: 0,
            polls_capped: 0,
            files_capped: 0,
            skipped_replayed: 0,
            saved_cursor_digest: None,
            resume_skip,
            blocked_file_first_sources: reconstruction.blocked_sources,
            last_leg1_seq,
            cycles: Arc::new(CycleCounters::default()),
        };
        // A row that is already armed and not yet started is a run whose pre-arm scan ran
        // before this process did. See `teach_direct_stores_on_first_sight`.
        if armed_unstarted {
            runtime.teach_direct_stores_on_first_sight();
        }
        if rebuilding {
            runtime.repair_foreign_chain_buffer()?;
        }
        Ok(runtime)
    }

    /// Opens the entry's chain, and the order is the contract.
    ///
    /// **Fresh entry** — explicit `FreshPending`: [`ChainWriter::create`], then
    /// [`ChainWriter::write_baseline`] as the very first call, so `EventBody::Baseline` is
    /// seq 0 and the start receipt's `chain_head` witnesses it. Anything appended before it
    /// makes the export answer `ExportError::BaselineNotFirst`, and V13 then reports
    /// `baseline_not_chained`. Only then `copy_manifest` and `write_descriptor`, neither of
    /// which chains a link.
    ///
    /// **Restart** — explicit `ResumePending` already owns the writer consumed from the
    /// fully validated prepared resume; then
    /// `EventBody::DaemonRestart { previous_head_seq, previous_head_hash }` from
    /// [`ChainWriter::head`], **before any tailer polls**. `write_baseline` is not called
    /// again — the export refuses it as `BaselineAlreadyChained`, which is the point: a
    /// second baseline is a second answer to what the workspace held at t0.
    ///
    /// **Why `DaemonRestart` must come before the re-read, and not merely be present.** A
    /// restarted tailer re-reads from offset 0 and emits `TailNotice::Reread` and, on a
    /// truncated store, `Truncated`. V9 only downgrades an unexplained truncation when a
    /// witness — a `DaemonRestart`, or a `Reread` for that store — sits at a **lower seq**
    /// than the notice **and no leg-1 record of that same store lies between them**. A
    /// restart chained after the first re-read poll is a witness above the notice, which
    /// explains nothing; one chained after the store's first record has already been spent.
    /// Either way the daemon's own restart reads as an unexplained truncation, at Fail, on an
    /// honest run.
    ///
    /// **The descriptor written here is not the final one.** No session is bound when a
    /// chain opens, so `tracked_sessions` is empty and `receipt` is `None` — a document the
    /// verifier loads and answers with a `no_session` Fail.
    /// [`EntryRuntime::refresh_descriptor`] rewrites it
    /// once there is something to say, which is what turns that Fail into a judgeable trace.
    ///
    /// The entry's `chain_seq`/`chain_head` are refreshed from [`ChainWriter::head`] here,
    /// never passed into it.
    pub fn open_trace(&mut self) -> Result<(), DaemonError> {
        // A rebuilt authoritative receipt must be reconciled while the retained writer is
        // still `ResumePending`, before the restart event or buffer can be changed. A
        // refusal leaves that capability and the debt intact for a later legitimate retry.
        let publish_receipt = self.reconcile_receipt_mirror()?;
        let state = std::mem::replace(&mut self.writer, WriterState::Failed);
        let result = match state {
            WriterState::FreshPending => self.open_fresh_trace(),
            WriterState::ResumePending(writer) => self.open_rebuilt_trace(writer),
            WriterState::Open(writer) => {
                self.writer = WriterState::Open(writer);
                if publish_receipt {
                    self.refresh_descriptor_and_commit_receipt()?;
                }
                return Ok(());
            }
            WriterState::Failed => {
                return Err(DaemonError::Refused(
                    "this runtime's trace opening already failed; it cannot reacquire the \
                     path or retry a partially completed opening"
                        .to_string(),
                ));
            }
        };
        if result.is_err() {
            self.writer = WriterState::Failed;
        }
        result
    }

    /// Resume an overdue V4 writer for immediate source-free finalization without adding a
    /// restart event after the signed cutoff. Startup calls this before the listener or
    /// any tailer poll; the subsequent admission CAS records restart ambiguity instead of
    /// backdating newly observed bytes into the qualifying chain.
    pub(super) fn open_trace_for_overdue_finalization(&mut self) -> Result<(), DaemonError> {
        let publish_receipt = self.reconcile_receipt_mirror()?;
        let state = std::mem::replace(&mut self.writer, WriterState::Failed);
        match state {
            WriterState::ResumePending(writer) => self.writer = WriterState::Open(writer),
            WriterState::Open(writer) => self.writer = WriterState::Open(writer),
            WriterState::FreshPending | WriterState::Failed => {
                return Err(DaemonError::Refused(
                    "overdue restart has no validated retained writer".to_string(),
                ));
            }
        }
        if publish_receipt {
            self.refresh_descriptor_and_commit_receipt()?;
        }
        self.sync_head()
    }

    fn open_rebuilt_trace(&mut self, mut writer: ChainWriter) -> Result<(), DaemonError> {
        let (previous_head_seq, previous_head_hash) = writer.head().ok_or_else(|| {
            DaemonError::Missing(format!(
                "the validated chain at {} has no head for a restart to name",
                self.entry.trace_dir.display()
            ))
        })?;
        let env = writer.event(EventBody::DaemonRestart {
            at: now_rfc3339(),
            previous_head_seq,
            previous_head_hash,
        })?;
        self.buffer.append(&env)?;
        self.writer = WriterState::Open(writer);
        self.tracker_restarted = true;
        self.chain_cursor_rereads()?;
        self.refresh_descriptor_and_commit_receipt()?;
        self.sync_head()
    }

    fn open_fresh_trace(&mut self) -> Result<(), DaemonError> {
        let redactor = Redactor::for_connector(&self.manifest)?;
        let trace = self.artifact.create_trace().map_err(|error| {
            DaemonError::Refused(format!(
                "fresh trace cannot be exclusively created through retained entry artifact authority: {error}"
            ))
        })?;
        let root = TraceRoot::from_acquired(trace, self.entry.trace_dir.clone());
        let mut writer =
            ChainWriter::create_from_root(self.entry.entry_id.clone(), root, redactor)?;
        let (_, link) = writer.write_baseline(&self.baseline)?;
        debug_assert_eq!(link.seq, 0, "the baseline is the genesis link");
        let diagnostic = writer.root().join(CHAIN_FILE);
        let reader = writer.independent_chain_reader()?;
        let env = chain_line_at(reader, &diagnostic, link.seq)?;
        self.buffer.append(&env)?;

        self.connector_manifest = writer.copy_manifest(&self.connector_dir)?;
        self.writer = WriterState::Open(writer);
        self.refresh_descriptor()?;
        self.unrestored_stores.clear();
        self.resume_skip.clear();
        self.sync_head()
    }

    /// Rewrite `trace.toml` from what the entry knows now.
    ///
    /// `write_descriptor` overwrites, and the document is a function of the run's state
    /// rather than of the moment the chain opened, so this is a rebuild and not a patch.
    /// Three fields move after `open_trace` has written its first version:
    ///
    /// - `tracked_sessions` — **the one that decides whether the trace loads at all**.
    ///   `TraceDescriptor::load` refuses an empty list, and a chain opens before any session
    ///   is bound, so a run whose descriptor was never refreshed is unverifiable no matter
    ///   what its chain holds.
    /// - `receipt` — `Some(receipt.json)` only once the registry-authoritative receipt's
    ///   exact compact bytes are ready beneath the retained root. V13 reports
    ///   `receipt_missing` for a started descriptor that names none.
    /// - `seal` — `Some(seal.json)` once [`EntryRuntime::seal`] has written it.
    ///
    /// Calling it twice writes the same bytes twice; it is not once-only.
    pub fn refresh_descriptor(&mut self) -> Result<(), DaemonError> {
        let doc = self.descriptor_doc();
        self.writer_mut()?.write_descriptor(&doc)?;
        Ok(())
    }

    /// Put the start receipt beside the chain, or hold it for the next tick.
    ///
    /// The only failure this returns is the one that means there is no writer at all, which
    /// [`EntryRuntime::bind_start`] has already met if it is going to. A write that fails
    /// leaves [`EntryRuntime::receipt_pending`] set and answers `Ok`: the run started, the
    /// registry holds the verified receipt, and the file is owed rather than lost.
    fn write_receipt_or_hold(&mut self, receipt: StartReceipt) -> Result<(), DaemonError> {
        self.receipt_writer_mut()?;
        self.receipt_mirror = ReceiptMirrorState::Owed {
            compact_bytes: serde_json::to_vec(&receipt)?,
            receipt: Box::new(receipt),
            mirror_ready: false,
        };
        let _ = self.reconcile_receipt_mirror();
        Ok(())
    }

    /// The receipt write this entry owes, tried again. Answers whether the descriptor now has a
    /// `receipt.json` to name that it did not name before.
    ///
    /// Called at the very top of a tick, before the config watch and the stores:
    /// the trace does not load without it, so nothing this tick chains is reachable by a
    /// verifier until the document is on disk.
    fn reconcile_receipt_mirror(&mut self) -> Result<bool, DaemonError> {
        let (receipt, compact_bytes) = match &self.receipt_mirror {
            ReceiptMirrorState::NoAuthoritativeStart | ReceiptMirrorState::Ready => {
                return Ok(false);
            }
            ReceiptMirrorState::Owed {
                receipt,
                compact_bytes,
                ..
            } => (receipt.clone(), compact_bytes.clone()),
        };
        // `mirror_ready` qualifies only the descriptor attempt immediately following an
        // exact observation. A failed publication leaves the authority in `Owed`, and the
        // next attempt must re-open the child rather than trust that earlier observation.
        if let ReceiptMirrorState::Owed { mirror_ready, .. } = &mut self.receipt_mirror {
            *mirror_ready = false;
        }
        let diagnostic = self
            .entry
            .trace_dir
            .join(goalcup_protocol::export::RECEIPT_FILE);
        if let Some(reader) = self
            .receipt_writer_mut()?
            .read_fixed(FixedTraceDocument::Receipt)?
        {
            let actual = bounded_receipt_bytes(reader, &diagnostic)?;
            if actual != compact_bytes {
                return Err(DaemonError::Refused(format!(
                    "{} does not exactly match the registry-authoritative start receipt",
                    diagnostic.display()
                )));
            }
            self.mark_receipt_mirror_ready();
            return Ok(true);
        }

        #[cfg(any(test, feature = "test-hooks"))]
        if std::mem::take(&mut self.fail_next_receipt_write) {
            return Err(DaemonError::Io {
                path: diagnostic.display().to_string(),
                source: std::io::Error::other("injected receipt write failure"),
            });
        }
        self.receipt_writer_mut()?.write_receipt(&receipt)?;
        self.mark_receipt_mirror_ready();
        Ok(true)
    }

    fn mark_receipt_mirror_ready(&mut self) {
        if let ReceiptMirrorState::Owed { mirror_ready, .. } = &mut self.receipt_mirror {
            *mirror_ready = true;
        }
    }

    /// Publish a descriptor that names the reconciled mirror, then retire its retry state.
    /// A descriptor error leaves `Owed { mirror_ready: true, .. }` intact, so the next poll
    /// retries publication before it processes any evidence.
    fn refresh_descriptor_and_commit_receipt(&mut self) -> Result<(), DaemonError> {
        self.refresh_descriptor()?;
        if matches!(
            self.receipt_mirror,
            ReceiptMirrorState::Owed {
                mirror_ready: true,
                ..
            }
        ) {
            self.receipt_mirror = ReceiptMirrorState::Ready;
        }
        Ok(())
    }

    /// Whether a verified start receipt is still owed to this entry's trace directory.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn receipt_pending(&self) -> bool {
        matches!(self.receipt_mirror, ReceiptMirrorState::Owed { .. })
    }

    /// Make the next [`ChainWriter::write_receipt`] fail. See the field.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_next_receipt_write(&mut self) {
        self.fail_next_receipt_write = true;
    }

    /// Lower one store's seen-file bound, so [`PollReport::files_capped`] is reachable with
    /// a handful of files instead of twenty thousand. A store that keeps no file set — both
    /// line-oriented readers — ignores it, and so does a name this entry does not declare.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn set_seen_cap(&mut self, store: &str, cap: usize) {
        for bound in &mut self.tailers {
            if bound.binding.name() == store {
                bound.reader.set_seen_cap(cap);
            }
        }
    }

    /// Report this entry's cadence failures through the daemon's own counters instead of
    /// this runtime's. Called once, by `Daemon::register`.
    ///
    /// They are the loop's facts rather than the run's, which is why they are handed over
    /// rather than kept: a runtime driven by hand (the round-trip fixture) has no loop
    /// around it and nothing to count, and a `None` here would make every status build ask
    /// a question with no answer.
    pub fn observe_cycles(&mut self, cycles: Arc<CycleCounters>) {
        self.cycles = cycles;
    }

    /// Drain exactly the window fixed by the finalization CAS: every tailer record not yet
    /// read at the admission instant is retained only in the encrypted out-of-window
    /// partition, never silently stranded or backdated.
    ///
    /// **The receiver watermark is compared** although nothing produces receiver
    /// sources. `finalization_receiver_sources` is `Some` only on the runtime that
    /// ran the admission CAS, so the comparison is also the "this is the process that
    /// closed admission" test: a runtime that did not is `RestartStateAmbiguous`, and that
    /// outcome is chained in the cutoff document's `pre_cutoff_drain`. A row persisted by
    /// an older daemon with a non-empty receiver watermark or queued posts is ambiguous for
    /// the same reason: those posts have no owner in this process.
    pub(super) fn drain_finalization_admitted(
        &mut self,
        watermarks: &AdmissionWatermarksV1,
        expired: &mut dyn FnMut() -> bool,
    ) -> PreCutoffDrainOutcomeV1 {
        let current_tailers = self.finalization_tailer_watermarks();
        let mut degraded = (current_tailers != watermarks.tailers)
            .then_some(PreCutoffDrainFailureV1::RestartStateAmbiguous);
        if self.finalization_receiver_sources.as_ref() != Some(&watermarks.receiver_sources)
            || watermarks.queued_receiver_posts != 0
        {
            degraded.get_or_insert(PreCutoffDrainFailureV1::RestartStateAmbiguous);
        }

        let head_matches = match (
            self.writer_mut().map(|writer| writer.head()),
            &watermarks.admitted_chain_predecessor,
        ) {
            (Ok(None), None) => true,
            (Ok(Some((seq, hash))), Some(position)) => {
                seq == position.seq && hash == position.hash.as_str()
            }
            _ => false,
        };
        if !head_matches {
            degraded.get_or_insert(PreCutoffDrainFailureV1::WriterWatermarkAmbiguous);
        }

        if !self.retain_out_of_window_tailers_until(expired) {
            degraded.get_or_insert(PreCutoffDrainFailureV1::WriterWatermarkAmbiguous);
        }

        #[cfg(any(test, feature = "test-hooks"))]
        if std::mem::take(&mut self.clear_workspace_append_failure_after_drain)
            && let Ok(writer) = self.writer_mut()
        {
            writer.clear_append_failure();
        }

        match degraded {
            Some(reason) => PreCutoffDrainOutcomeV1::Degraded { reason },
            None => PreCutoffDrainOutcomeV1::Complete,
        }
    }

    fn retain_out_of_window_tailers_until(&mut self, expired: &mut dyn FnMut() -> bool) -> bool {
        let mut complete = true;
        let mut passes = 0usize;
        let bound_session = self.entry.session_id.clone();
        let limits = match self.out_of_window_limits() {
            Ok(limits) => limits,
            Err(error) => {
                self.cycles.note(Cadence::Tick, &error);
                return false;
            }
        };
        loop {
            if expired() {
                return false;
            }
            let mut more = false;
            for index in 0..self.tailers.len() {
                let before = self.tailers[index].reader.cursor();
                let batch = match self.tailers[index].reader.poll() {
                    Ok(batch) => batch,
                    Err(error) => {
                        self.cycles.note(Cadence::Tick, &error);
                        complete = false;
                        continue;
                    }
                };
                more |= batch.more_available;
                let processed = process_out_of_window_batch(
                    &self.entry.entry_id,
                    &batch,
                    &self.bindings,
                    &mut self.partitioners,
                    &self.manifest,
                    &mut self.sessions_by_path,
                    bound_session.as_deref(),
                    false,
                );
                complete &= processed.complete;
                if let Some(error) = &processed.first_error {
                    self.cycles.note(Cadence::Tick, error);
                }
                if before == self.tailers[index].reader.cursor()
                    && processed.canonical_records.is_empty()
                    && processed.validation == OutOfWindowBatchValidationV1::CLEAN
                {
                    continue;
                }
                match apply_out_of_window_batch(
                    &self.buffer,
                    &self.manifest,
                    &self.tailers[index],
                    &before,
                    &processed,
                    limits,
                ) {
                    Ok(batch_complete) => complete &= batch_complete,
                    Err(error) => {
                        self.cycles.note(Cadence::Tick, &error);
                        complete = false;
                    }
                }
            }
            passes += 1;
            if !more {
                return complete;
            }
            if passes >= MAX_POLLS_PER_TICK {
                return false;
            }
        }
    }

    /// Catch tailer bytes written while blocking capture/readback owned this runtime.
    /// They stay private and outside the qualifying chain/submission. Any ambiguous bound
    /// becomes a durable terminal-review fact before the runtime returns to its slot.
    pub(super) fn retain_post_cutoff_tailers(
        &mut self,
        expired: &mut dyn FnMut() -> bool,
    ) -> Result<(), DaemonError> {
        if self.retain_out_of_window_tailers_until(expired) {
            return Ok(());
        }
        self.persist_out_of_window_nonclean()?;
        Err(DaemonError::Refused(format!(
            "entry {} could not retain its complete post-cutoff tailer window",
            self.entry.entry_id
        )))
    }

    /// The ordered data path, exactly once, **at a clock the caller supplies**.
    ///
    /// Every time-dependent decision inside one tick — the heartbeat, [`ArmingGate::tick`],
    /// [`health::evaluate`]'s `now`, the deadline comparison — reads `now` from this
    /// argument and never from [`crate::now_unix`], which is the seam a test moves.
    ///
    /// **It drains, and the drain is bounded.** After each tailer pass it loops while any
    /// store's `TailBatch.more_available` is still true, up to [`MAX_POLLS_PER_TICK`]
    /// passes, and then returns with whatever remains left for the next tick.
    /// `PollReport.captured` counts every record of every pass.
    ///
    /// **Bounded memory.** A batch is the tailer's own allocation; this loop borrows the
    /// records out of it one at a time and never takes a second copy, because a many-file
    /// batch can exceed 1 MiB on its own.
    pub fn poll_once_at(&mut self, now: f64) -> Result<PollReport, DaemonError> {
        if self.entry.state.capture_closed() {
            return Err(DaemonError::Refused(format!(
                "entry {} is {:?}; capture is closed while its terminal state is reconciled",
                self.entry.entry_id, self.entry.state
            )));
        }
        if self.reconcile_receipt_mirror()? {
            self.refresh_descriptor_and_commit_receipt()?;
        }
        // The arming window's own clock, ticked for its effect: a window that has run out
        // closes here, at the top, before anything in this tick reads the gate. The state
        // the report carries is read once, at the end — `observe_leg1` can move it while
        // the stores are draining, and a report stamped up here would be answering with
        // the gate the tick started with.
        self.gate.tick(now);
        let mut counts = Counts::default();
        let mut notices: Vec<TailNotice> = Vec::new();

        // The receipt this entry owes its own trace directory was reconciled at the top of the
        // tick, ahead of the config watch and the stores: until `receipt.json` is on disk the
        // descriptor cannot name it and the trace does not load, so every record this tick
        // chains is unreachable to a verifier.

        // 1. The config watch, before the stores: a hook file that was put back explains a
        //    gap in what the stores hold, and the deviation should be the earlier link.
        let deviations = self.watch.poll()?;
        let deviation_count = deviations.len();
        let mut config_restored = false;
        for deviation in deviations {
            if deviation.restored {
                config_restored = true;
            }
            if deviation.arming_lost {
                self.arming_lost = true;
            }
            let env = {
                let writer = self.writer_mut()?;
                writer.event(EventBody::ConfigDeviation(deviation))?
            };
            self.buffer.append(&env)?;
            counts.chained += 1;
        }
        // Read off the watch rather than latched here: `ConfigWatch::mismatched` answers
        // whether anything it has reported is still away from the digest it was pinned at,
        // and an app-wide file the participant put back matches again on the very next
        // poll. The deviation that raised it is in the chain and stays there; only the live
        // health state moves, and it moves as an ordinary transition.
        self.config_mismatch = self.watch.mismatched();

        // 2. The stores, drained.
        let mut store_replaced = false;
        let mut passes = 0usize;
        let mut blocked_reported = BTreeSet::new();
        loop {
            let mut more = false;
            for index in 0..self.tailers.len() {
                let store = self.tailers[index].binding.name().to_string();
                if self.blocked_file_first_sources.contains_key(&store) {
                    if blocked_reported.insert(store.clone()) {
                        notices.push(TailNotice::StoreUnavailable {
                            path: self.store_path(&store),
                            store,
                            reason: "restart source traverses a symbolic link or is not a regular file; restore the declared source and restart the daemon".to_string(),
                        });
                    }
                    continue;
                }
                // The batch is taken and the borrow released before anything is judged, so
                // the pipeline below can hold `&mut self`.
                let batch = { self.tailers[index].reader.poll()? };
                more |= batch.more_available;
                // A bound this daemon hit, counted where nothing else can see it. The notice the
                // tailer pushed beside this flag is a
                // `StoreUnavailable`, which chains nothing and reads as a store that could
                // not be opened; the files it refused are evidence that will not exist, and
                // this counter is the only thing that ever says so.
                if batch.files_capped {
                    self.files_capped += 1;
                }
                // **The batch's notices, before the batch's records.** A notice is
                // what explains the records under it: a `Truncated` or a `FileReplaced`
                // says the tailer went back and re-read, and the re-read records are the
                // thing it says that about. Chained after them, the notice sits below its
                // own evidence — and worse, on the first poll after a restart the re-read
                // records then lie between the `DaemonRestart`/`Reread` witness and the
                // notice, which is precisely what `read_since` spends the witness on: an
                // honest restart over a rotated transcript reads as an unexplained truncation, at
                // Fail.
                //
                // `open_trace` has already chained this store's `Reread` when there was one
                // to chain, before any tailer polled, so a witness and a notice cannot
                // fight over the same batch.
                for notice in batch.notices {
                    if notice.is_rewrite() {
                        store_replaced = true;
                        // A new generation at the same path has no inherited owner. Drop
                        // the old in-memory decision before this batch's records are
                        // admitted, so its first valid header decides from scratch.
                        if let Some(partitioner) = self.partitioners.get_mut(notice.store()) {
                            partitioner.forget_file(notice.path());
                        }
                        // …and the skip budget goes with the file it was measured against. It
                        // counts records the chain holds for a position in the old file; what a
                        // truncated or replaced store hands back is not what that count was about,
                        // and a budget left standing would swallow the first records of the
                        // rewritten file. The notice explains the re-read on its own, so the batch
                        // is left unskipped.
                        if let Some(bound) = self.tailers.get(index) {
                            let store = bound.binding.name().to_string();
                            self.resume_skip.remove(&store);
                        }
                    }
                    if let Some(body) = notice.to_event(unix_to_rfc3339(now)) {
                        let env = {
                            let writer = self.writer_mut()?;
                            writer.event(body)?
                        };
                        self.buffer.append(&env)?;
                        counts.chained += 1;
                    }
                    notices.push(notice);
                }
                for record in batch.records.iter() {
                    self.ingest_record(record, now, &mut counts)?;
                }
                // **A store's remaining skip budget does not survive its own re-read.** The budget
                // counts `Leg1Record`s the chain
                // holds above the saved position; it is spent only on records the
                // partitioner still keeps. A record kept before the kill can come back
                // judged differently the second time — most concretely, an
                // `AggregateLearned` store whose teaching row sits below the saved
                // position is never re-read, so a row this run once kept is now
                // `UnknownAggregate` and discarded before `resume_skip_taken` is ever
                // asked. That record spends nothing, the budget survives it unspent, and
                // every later record — genuinely new, chained for the first time — is
                // silently swallowed in its place until the budget runs out on its own.
                //
                // `more_available == false` is the fact that ends this cleanly: it says
                // the tailer has caught this store up to what it can currently see, so
                // whatever a stale budget still names is not coming back around on a
                // later pass. Dropped here and not before the records above ran, so a
                // re-read that spans more than one `MAX_POLL_BYTES` window (`more` still
                // true) keeps its budget across the passes that follow.
                if !batch.more_available
                    && let Some(bound) = self.tailers.get(index)
                {
                    let store = bound.binding.name().to_string();
                    self.resume_skip.remove(&store);
                }
                // **The cursor, here, and not once at the end of the tick.** A tick chains
                // every store's records and every pass's, and `cursors.json` written only
                // after all of it left a hard kill mid-tick with a cursor behind the chain
                // — the tailer that restored it re-read records already chained and had no
                // notice to explain the repeat.
                //
                // Per batch and not per record, because a batch is the finest position a
                // cursor can state: `Tailer::poll` advances to the end of the window it
                // read before this loop sees the first record, so a save between two
                // records of one batch would persist a position ahead of the chain, and a
                // kill there would lose the rest of the batch outright. Behind the chain
                // costs a re-read, which the saved chain seq beside the position then
                // counts exactly; ahead of it costs evidence, which nothing can recover.
                if !batch.records.is_empty() {
                    self.save_cursors()?;
                }
            }
            passes += 1;
            if !more {
                break;
            }
            if passes >= MAX_POLLS_PER_TICK {
                // Counted, because nothing else records it. A tick that yields with a store
                // still holding data looks exactly like a quiet tick from the chain, from
                // the heartbeat and from the health ladder — the cursor simply moves less
                // than it could have — so a daemon that has been behind since the run
                // started would otherwise be indistinguishable from one that is caught up.
                self.polls_capped += 1;
                break;
            }
        }

        // …and the per-file maps this tick may have grown lose the files that are gone.
        // Once a tick, after the drain, so a file rotated away between two ticks is
        // forgotten and the memo stays the size of the store rather than of its history.
        self.evict_missing_sessions();

        // Binding can become known from an ordinary store poll, long before the next
        // heartbeat. Reconcile durable pending own-config identities at that boundary so
        // a source that disappeared during the race is not left pending for a full cadence.
        if let Some(bound) = self.bound_session() {
            let family = self.manifest.family.clone();
            let harness = self.manifest.id.clone();
            counts.chained += self.flush_pending_foreign(
                &family,
                &harness,
                Some(bound.as_str()),
                ForeignSweepPhase::LiveHeartbeat,
            )?;
        }
        self.repair_needed_foreign_buffers()?;

        // 3. The heartbeat and, on the same slow cadence, the foreign sweep. Both are off
        //    the poll timer on purpose: a sweep of the participant's harness home on every
        //    250 ms tick is a walk this daemon has no business repeating.
        if self.heartbeat.due(now) {
            let env = {
                let stores_seen = self.store_cursors()?;
                let writer = self.writer_mut()?;
                writer.event(EventBody::Heartbeat {
                    ts: unix_to_rfc3339(now),
                    stores_seen,
                    // No hook receiver runs in this daemon, so none is up. The field is chained
                    // wire; `false` is the true value.
                    receiver_ok: false,
                })?
            };
            self.buffer.append(&env)?;
            counts.chained += 1;
            counts.chained += self.sweep_foreign(now, ForeignSweepPhase::LiveHeartbeat)?;
        }

        // 4. What the ladder is told, and what of it reaches the chain.
        let inputs = self.health_inputs(now, store_replaced, config_restored)?;
        let evaluated = health::evaluate(&inputs, &self.policy);
        let health_changed = evaluated.state != self.health;
        let detail = evaluated.detail();
        let states = evaluated.states();
        for state in &states {
            // Only the nine that have no richer event of their own, and only on
            // a transition — a state that was already true is not news, and a chain that
            // repeated it every 250 ms would bury the run.
            if !health::chains_as_event(*state) || self.health_states.contains(state) {
                continue;
            }
            let env = {
                let writer = self.writer_mut()?;
                writer.event(EventBody::Health(HealthEvent {
                    state: *state,
                    at: unix_to_rfc3339(now),
                    detail: detail.clone(),
                }))?
            };
            self.buffer.append(&env)?;
            counts.chained += 1;
        }
        self.health_states = states;
        self.health = evaluated.state;

        self.save_cursors()?;
        self.sync_head()?;
        // Built here and nowhere else, so every field is stated once, from the state the
        // tick actually ended in.
        Ok(PollReport {
            captured: counts.captured,
            discarded: counts.discarded,
            deviations: deviation_count,
            arming: self.gate.state(),
            health: evaluated.state,
            notices,
            health_changed,
            chained: counts.chained,
            polls_capped: self.polls_capped,
            cursor_restore_failures: self.cursor_restore_failures,
            skipped_replayed: self.skipped_replayed,
            files_capped: self.files_capped,
        })
    }

    /// One drain of the write-ahead buffer to the upload server.
    ///
    /// **[`Uploader::resume`] is not called from here**, and must never be: it drops an
    /// outstanding backoff, so a poll loop that called it would turn every failure into an
    /// immediate retry and the backoff into decoration. It belongs to an explicit resume —
    /// [`EntryRuntime::resume_upload`].
    pub async fn pump(&mut self) -> Result<PumpOutcome, DaemonError> {
        // A live runtime can remain in the map through the final acknowledgement.  An
        // unresolved sealing intent has staged a *plan*, not a proved terminal link: it
        // may upload only the predecessor the retained writer already validated.  Once the
        // registry is `Sealed`, its terminal head becomes the safe ceiling.
        let terminal_seq = match self.entry.state {
            EntryState::Sealing => self
                .entry
                .sealing_intent()
                .and_then(SealingIntent::pre_seal_seq),
            EntryState::Sealed => self.entry.chain_seq,
            _ => None,
        };
        if self.entry.state == EntryState::Sealing && terminal_seq.is_none() {
            return Err(DaemonError::Refused(format!(
                "entry {} has an unresolved seal with no proven predecessor to upload",
                self.entry.entry_id
            )));
        }
        let durable_entry = self.registry.get(&self.entry.entry_id)?.ok_or_else(|| {
            DaemonError::Missing(format!(
                "entry {} is not in the registry",
                self.entry.entry_id
            ))
        })?;
        let durable_ack = durable_entry.last_ack_seq;
        if let Some(terminal) = terminal_seq
            && (self
                .entry
                .last_ack_seq
                .is_some_and(|acked| acked > terminal)
                || durable_ack.is_some_and(|acked| acked > terminal))
        {
            // The durable row is an acknowledgement authority too.  Check it before
            // touching the writer or buffer: a corrupt registry watermark must not be
            // silently superseded by a lower buffer floor or reported as a live upload.
            return self.refuse_terminal_validation(DaemonError::Refused(format!(
                "entry {} acknowledges beyond terminal seq {terminal}",
                self.entry.entry_id
            )));
        }
        if let Some(terminal) = terminal_seq {
            let validation = (|| {
                if self.entry.state == EntryState::Sealed {
                    let floor = self.buffer.floor()?;
                    if floor.is_some_and(|acked| acked > terminal) {
                        return Err(DaemonError::Refused(format!(
                            "entry {} buffer acknowledgement exceeds terminal seq {terminal}",
                            self.entry.entry_id
                        )));
                    }
                    if durable_ack.is_some_and(|registry_ack| {
                        floor.is_none_or(|buffer_floor| buffer_floor < registry_ack)
                    }) {
                        return Err(DaemonError::Refused(format!(
                            "entry {} registry acknowledgement is ahead of durable buffer floor",
                            self.entry.entry_id
                        )));
                    }
                }
                self.buffer
                    .ensure_no_records_after(terminal)
                    .map_err(|error| {
                        DaemonError::Refused(format!(
                            "entry {} cannot drain beyond terminal: {error}",
                            self.entry.entry_id
                        ))
                    })
            })();
            if let Err(error) = validation {
                return self.refuse_terminal_validation(error);
            }
            // A transient failure to persist an earlier terminal refusal is not itself a
            // durable classification.  This fresh validation is clean; the daemon still
            // keeps its process-local withholding latch until it has successfully
            // republished a clean completion, but must be able to distinguish that retry
            // from another failed persistence attempt.
            if self.entry.state == EntryState::Sealed {
                self.terminal_validation_persistence_failed = false;
            }
        }
        let outcome = {
            let EntryRuntime {
                uploader,
                buffer,
                writer,
                ..
            } = self;
            let WriterState::Open(writer) = writer else {
                return Err(no_writer());
            };
            uploader.pump_through(buffer, writer, terminal_seq).await?
        };
        self.connected = !outcome.offline && outcome.rejected.is_none();
        self.backing_off = outcome.backing_off;
        self.ack_unverified = outcome.ack_unverified;
        self.buffer_gap_at = outcome.gap_at;
        if let Some(skew) = outcome.clock_skew_seconds {
            self.clock_skew_seconds = Some(skew);
        }
        if outcome.uploaded > 0 || outcome.acked_seq != self.entry.last_ack_seq {
            let acked = outcome.acked_seq;
            let acknowledged = super::seal::acknowledgement_delta(durable_ack, acked)?;
            let at = now_unix();
            self.entry = self.registry.update(&self.entry.entry_id, |e| {
                e.last_ack_seq = acked;
                e.last_ack_at = Some(at);
                // `Buffer::ack` is durable before this mirror transaction.  A retry can
                // therefore carry zero newly sent bytes while repairing a non-zero durable
                // acknowledgement watermark; status must count the acknowledged links, not
                // only this call's upload attempt.
                e.events_uploaded = e.events_uploaded.saturating_add(acknowledged);
            })?;
        }
        #[cfg(any(test, feature = "test-hooks"))]
        if let Some(envelope) = self.post_pump_tail_for_test.take() {
            self.buffer.append(&envelope)?;
        }
        // The uploader has returned, so the registry ACK and buffer floor may now differ
        // from the snapshot that admitted this pump.  Do not let a normal sealed status or
        // eviction escape this final durable terminal check.
        if self.entry.state == EntryState::Sealed {
            self.validate_sealed_terminal_after_pump()?;
        }
        Ok(outcome)
    }

    fn validate_sealed_terminal_after_pump(&mut self) -> Result<(), DaemonError> {
        let terminal = self.entry.chain_seq.ok_or_else(|| {
            DaemonError::Refused(format!(
                "sealed entry {} has no terminal sequence",
                self.entry.entry_id
            ))
        })?;
        let durable = self.registry.get(&self.entry.entry_id)?.ok_or_else(|| {
            DaemonError::Missing(format!(
                "entry {} is not in the registry",
                self.entry.entry_id
            ))
        })?;
        let validation = (|| {
            if self
                .entry
                .last_ack_seq
                .is_some_and(|acked| acked > terminal)
                || durable.last_ack_seq.is_some_and(|acked| acked > terminal)
            {
                return Err(DaemonError::Refused(format!(
                    "entry {} acknowledges beyond terminal seq {terminal}",
                    self.entry.entry_id
                )));
            }
            let floor = self.buffer.floor()?;
            if floor.is_some_and(|acked| acked > terminal) {
                return Err(DaemonError::Refused(format!(
                    "entry {} buffer acknowledgement exceeds terminal seq {terminal}",
                    self.entry.entry_id
                )));
            }
            if durable.last_ack_seq.is_some_and(|registry_ack| {
                floor.is_none_or(|buffer_floor| buffer_floor < registry_ack)
            }) {
                return Err(DaemonError::Refused(format!(
                    "entry {} registry acknowledgement is ahead of durable buffer floor",
                    self.entry.entry_id
                )));
            }
            self.buffer
                .ensure_no_records_after(terminal)
                .map_err(|error| {
                    DaemonError::Refused(format!(
                        "entry {} cannot drain beyond terminal: {error}",
                        self.entry.entry_id
                    ))
                })
        })();
        if let Err(error) = validation {
            return self.refuse_terminal_validation(error);
        }
        self.terminal_validation_persistence_failed = false;
        Ok(())
    }

    /// A sealed terminal validation refusal is a durable capture-closed outcome. Keep the
    /// runtime mirror in sync with the registry transaction so the immediate publisher and
    /// every later status cadence cannot overwrite review status with stale `Sealed` state.
    fn refuse_terminal_validation<T>(&mut self, error: DaemonError) -> Result<T, DaemonError> {
        if self.entry.state == EntryState::Sealed {
            match self
                .registry
                .record_terminal_drain_validation_failure(&self.entry.entry_id)
            {
                Ok(entry) => {
                    self.entry = entry;
                    self.terminal_validation_persistence_failed = false;
                }
                Err(persistence_error) => {
                    self.terminal_validation_persistence_failed = true;
                    return Err(persistence_error);
                }
            }
        }
        Err(error)
    }

    /// The first sequence the write-ahead buffer wanted and did not find, when something
    /// above it is buffered (`ReadChunk.gap_at`).
    ///
    /// Nothing refills a hole in a write-ahead log, so the run is capped at `gap_at - 1`
    /// whatever else happens. It reaches the health ladder the only way the ladder can take
    /// it: `pending_events` keeps counting above the hole while `last_ack_at` stops moving,
    /// which is `UploadLagging` and then `ConnectionLost`. There is no `HealthState` for "a
    /// hole in the buffer", and adding one is a protocol change.
    pub fn buffer_gap_at(&self) -> Option<u64> {
        self.buffer_gap_at
    }

    /// Say where the server is and try again now (the resync path).
    ///
    /// The one caller of [`Uploader::resume`] in this crate, and deliberately not on the
    /// poll path: see [`EntryRuntime::pump`].
    pub fn resume_upload(&mut self) {
        self.uploader.resume(self.entry.last_ack_seq);
    }

    /// Build every distinct child reader and its partition state before a receipt makes the
    /// start immutable. Primary readers stay where they are, retaining their first-sight
    /// state and cursor; these new readers deliberately begin at zero so bytes written
    /// before binding are captured once the exact session authorizes them.
    fn prepare_derived_activation(&self, session: &str) -> Result<DerivedActivation, DaemonError> {
        let mut vars = self.vars.clone();
        vars.session_id = Some(session.to_string());
        let parent_order: Vec<String> = poll_order(&self.stores)
            .into_iter()
            .map(|store| store.name.clone())
            .collect();
        let tailers: Vec<BoundTailer> = tailer::build_bound_tailers(
            &self.declared_bindings,
            &self.manifest,
            &vars,
            &parent_order,
        )?
        .into_iter()
        .filter(|bound| bound.binding.kind() != SourceKind::Primary)
        .collect();
        let BindingState {
            bindings,
            partitioners,
        } = binding_state(&tailers, &self.manifest, &self.workspace, Some(session))?;
        if bindings.keys().any(|name| self.bindings.contains_key(name)) {
            return Err(DaemonError::Refused(
                "a derived binding is already active before start".to_string(),
            ));
        }
        Ok(DerivedActivation {
            vars,
            tailers,
            bindings,
            partitioners,
        })
    }

    /// Commit a previously prepared activation. Every fallible operation happened before
    /// `record_start`; this is intentionally only moves into maps and vectors.
    fn activate_derived(&mut self, activation: DerivedActivation) {
        self.vars = activation.vars;
        self.tailers.extend(activation.tailers);
        self.bindings.extend(activation.bindings);
        self.partitioners.extend(activation.partitioners);
    }

    /// At `start_signal_seen`, request and verify the receipt, then stamp `started_at`,
    /// `deadline_at` and `deadline_unix` once.
    ///
    /// **The deadline is parsed once, here**: a deadline recomputed on every poll moves with
    /// the clock and is never passed, which would make `DeadlineReached` and the automatic
    /// seal unreachable.
    ///
    /// `session_id` is what the caller believes bound; the session the run is actually
    /// recorded under is the gate's — the one the daemon watched a leg-1 record arrive in.
    /// [`record_start`] enforces it, and a receipt naming any other session is refused.
    ///
    /// **This is the single caller of [`record_start`] for this entry** (see the module
    /// note on one thread per entry).
    pub async fn bind_start(&mut self, session_id: &str) -> Result<(), DaemonError> {
        if self.entry.started_at.is_some() {
            // `started_at` is immutable once stamped, so this is a no-op and not a refusal: a
            // second corroborating record arriving on a started run is the normal path.
            return Ok(());
        }
        if !self.entry.start_handoff_complete() {
            return Err(DaemonError::Guard(format!(
                "entry {} cannot request a start receipt before its process handoff",
                self.entry.entry_id
            )));
        }
        let session = self
            .gate
            .session_id()
            .map(|s| s.to_string())
            .unwrap_or_else(|| session_id.to_string());
        // **The genesis link, not the current head.** `chain_head` is the one
        // claim in a receipt that says *which chain* the server opened this entry over, and
        // both readers of it name seq 0: `verify_envelopes` refuses an entry's first chunk
        // whose seq 0 does not hash to it, and V13 reports `chain_head_mismatch` when it is
        // not `VerifiedChain::genesis_link`. `ChainWriter::head()` is the last link, which is
        // the baseline only on a chain nothing has been appended to, and a start is stamped on
        // the first *corroborated* record, so by the time this runs the head is that record.
        // Read off the chain rather than remembered, for the same reason `seal` reads its own
        // link back: the export owns what it wrote, and a second copy in this type is a second
        // answer.
        let chain_head = self.read_chain_line(0)?.link.hash;
        // The two the server owns are sent and then overwritten by its clock; a start a
        // participant could name is a deadline they could name.
        let started_at = now_rfc3339();
        let deadline_at = receipt::deadline_for(&started_at).unwrap_or_else(|| started_at.clone());
        let (run_public_key_claim, questionnaire_version) =
            receipt::binding_claims_for_entry(&self.entry)?;
        let claims = StartReceiptClaims {
            entry_id: self.entry.entry_id.clone(),
            connector_id: self.entry.connector_id.clone(),
            // The receipt binds the registry's durable identity spelling. `self.workspace`
            // is separately canonicalised for local tailing, so using it here would make a
            // coherent pre-canonicalisation row ask the service to sign a different identity
            // from the one `record_start` validates inside the registry transaction.
            workspace: self.entry.workspace.to_string_lossy().to_string(),
            session_id: session.clone(),
            started_at,
            deadline_at,
            manifest_sha256: self.entry.manifest_sha256.clone(),
            chain_head,
            // The directory digest this daemon took at workspace creation, over its own
            // `connectors_dir`. Sent as the daemon's own claim and signed by the server as
            // it stands — the server verifies the run key, not the connector — and read back
            // by V13 against the release allowlist. `None` only on a row written before the
            // field existed; `skip_serializing_if` keeps that out of the signed bytes
            // entirely, so such a receipt has exactly the layout it would have had.
            connector_dir_sha256: self.entry.connector_dir_sha256.clone(),
            run_public_key: run_public_key_claim,
            questionnaire_version,
        };
        let run_public_key = self.entry.keys.public_key_hex()?;
        // This is the runtime that built the claims and opened the trace. Capture its
        // non-receipt path identity before the network wait, because looking the row up
        // after the server answers would silently adopt a same-id replacement.
        let expected_paths = StartPathIdentity::from_entry(&self.entry);
        // Resolve session-scoped paths and allocate their readers while a failed start is
        // still retryable. No derived reader is visible to a poll until the receipt-backed
        // registry transition below succeeds.
        let activation = self.prepare_derived_activation(&session)?;
        let receipt = self.receipts.request(&claims, &run_public_key).await?;
        let stored = record_start(
            &self.registry,
            &self.entry.entry_id.clone(),
            &mut self.gate,
            &receipt,
            &claims,
            &expected_paths,
            &self.pinned_server_key,
        )?;
        // `record_start` is the irreversible success marker. Install the already prepared
        // readers immediately, before receipt/descriptor publication can fail, so a retry
        // never leaves a durable started session with its retained children unread.
        self.activate_derived(activation);
        self.entry = stored;
        // The verified receipt travels with the trace: the verifier reads it beside the
        // chain head it witnessed.
        //
        // **A failed write does not fail the start.** `record_start` has already
        // verified the receipt against the pinned key and recorded it in the registry, and
        // the row is already `Started`; returning here would leave a run going with a
        // receipt nothing on disk carries, and the descriptor naming a file that is not
        // there. The document is held instead and retried at the top of every tick, and the
        // descriptor does not name `receipt.json` until it is written.
        self.write_receipt_or_hold(receipt)?;
        debug_assert!(
            self.entry.state == EntryState::Started && self.entry.started_at.is_some(),
            "the state transition to Started happens exactly when `started_at` is stamped: \
             Task 10's length gate keys on `Armed | StartSignalSeen`"
        );

        // The session is bound, so a `Direct` partitioner that has been comparing against
        // an empty set now has its value (Codex's `history` store partitions on the session
        // id).
        //
        // **`Direct` and nothing else.** `set_tracked` clears the learned per-file
        // and per-aggregate caches, which is right for the store whose tracked value was
        // unknown until this moment and catastrophic for every other one: their values came
        // from a record that has already gone by, and `admit` judges once. On OpenCode the
        // teaching `session.created.1` lands before the first prompt, so clearing it here
        // would send every later `event`, `message` and `part` row to `UnknownAggregate` for
        // the rest of the run. On Codex it would throw away the rollout file→cwd map, and a
        // foreign thread's later records would then meet the `None` arm, which keeps: D20
        // inverted, at the moment the run starts.
        //
        // The taught id is read before `set_tracked` for clarity only: `set_tracked`
        // deliberately does not clear `taught`, so the value is available either side of it.
        let mut taught_direct: Vec<(String, String)> = Vec::new();
        for store in &self.stores {
            if !matches!(store.partition, Some(PartitionSpec::Direct { .. })) {
                continue;
            }
            if let Some(p) = self.partitioners.get_mut(&store.name) {
                if let Some(taught) = p.taught() {
                    taught_direct.push((store.name.clone(), taught.to_string()));
                }
                p.set_tracked(partition::tracked_for(
                    store.partition.as_ref(),
                    &self.workspace,
                    Some(&session),
                ));
            }
        }
        // …and the reconcile. A `Direct` store taught itself the first session id
        // it saw after arming, so that records written before the bind — Codex's blocked
        // prompt in `history.jsonl`, whose only leg-1 witness that file is — are captured
        // rather than judged against an empty set and dropped. The receipt is issued for
        // the session the gate corroborated, and where the two disagree the store had been
        // reading a thread that is not this run's. That is chained as a fact and never
        // absorbed quietly: `set_tracked` above has already put the bound id in its place,
        // so from here the taught one is foreign, which is exactly what this says.
        for (store, taught) in taught_direct {
            if taught == session {
                continue;
            }
            let harness = self.manifest.id.clone();
            if !self.foreign_seen.insert((
                self.manifest.family.clone(),
                harness.clone(),
                taught.clone(),
            )) {
                continue;
            }
            let store_path = self.store_path(&store);
            let env = self.writer_mut()?.event(EventBody::ForeignSession {
                harness,
                session_id: taught,
                store_path,
                first_seen: now_rfc3339(),
            })?;
            self.buffer.append(&env)?;
        }
        // The descriptor was written before any of this was true. Until it is rewritten,
        // `tracked_sessions` is empty and the trace does not load at all.
        self.refresh_descriptor_and_commit_receipt()
    }

    /// The unsigned status payload (D25) at a clock the caller supplies —
    /// `seconds_remaining` is a countdown and reads it.
    ///
    /// **Every health state reaches this.** Which states chain as events is about evidence,
    /// not about the participant surface.
    pub fn status_at(&self, now: f64) -> Result<StatusPayload, DaemonError> {
        Ok(status::build_payload(&StatusInputs {
            entry: &self.entry,
            manifest: Some(&self.manifest),
            arming: self.gate.state(),
            health: self.health,
            events_uploaded: self.entry.events_uploaded,
            events_pending: self.buffer.pending(None)?,
            last_sync_at: self.entry.last_ack_at.map(unix_to_rfc3339),
            now,
            event: self.event.clone(),
            cycles: self.cycles.snapshot(),
            polls_capped: self.polls_capped,
            cursor_restore_failures: self.cursor_restore_failures,
            skipped_replayed: self.skipped_replayed,
            buffer_gap_at: self.buffer_gap_at,
            files_capped: self.files_capped,
            // Off the durable row rather than a field of its own: the kill happens in
            // `Daemon::seal_entry`, after `seal()` has returned, and the row is where that
            // answer is written — so a status built by a later process reads the same fact
            // as the one published under the seal's own hold.
            harness_unverified: self
                .seal_outcome_for_status()?
                .is_some_and(|o| o.harness_unverified),
        }))
    }

    /// The `SealOutcome` [`EntryRuntime::status_at`] builds `harness_unverified` from.
    ///
    /// `Daemon::seal_entry` writes the registry row and only then calls
    /// [`EntryRuntime::note_seal_outcome`] to mirror it here — and swallows that second
    /// call's error when a concurrent pump has already evicted the drained entry out of the
    /// map it runs `with_entry` against. When that happens `self.entry.seal_outcome` stays
    /// `None` for the rest of this runtime's life while the row already carries the seal's
    /// real answer, and a status built from the in-memory copy alone would publish
    /// `harness_unverified: false` under a seal whose row says `true`.
    ///
    /// So the in-memory copy is read first — it is what a normal seal, with nothing racing
    /// it, already set — and the row is asked only when it is absent. The row cannot be
    /// staler than the in-memory copy (it is written first, at `Daemon::seal_entry`'s single
    /// write site), so falling back to it here can only ever correct the gap the swallowed
    /// error leaves, never overwrite a fresher in-memory answer with an older one.
    fn seal_outcome_for_status(&self) -> Result<Option<SealOutcome>, DaemonError> {
        if let Some(outcome) = self.entry.seal_outcome.clone() {
            return Ok(Some(outcome));
        }
        // Only a sealed row can carry an outcome this copy missed (the eviction race);
        // an unsealed entry publishes every tick and must not pay a registry read for it.
        if self.entry.state != EntryState::Sealed {
            return Ok(None);
        }
        Ok(self
            .registry
            .get(&self.entry.entry_id)?
            .and_then(|row| row.seal_outcome))
    }

    /// [`EntryRuntime::status_at`] at the wall clock.
    pub fn status(&self) -> Result<StatusPayload, DaemonError> {
        self.status_at(now_unix())
    }

    /// Sign the payload with this entry's own run key, mirror the two cache copies the MCP
    /// server reads, and write the one plain status line the harness `cat`s (D25).
    pub fn publish_status(
        &self,
        config_dir: &std::path::Path,
        artifact_root: &std::path::Path,
        now: f64,
    ) -> Result<SignedStatus, DaemonError> {
        let signed = status::sign_payload(
            self.status_at(now)?,
            unix_to_rfc3339(now),
            &self.entry.keys.signing_key()?,
        )?;
        status::mirror(&signed, config_dir)?;
        // Ordinary live publication retains the established writer boundary.  Terminal
        // review invalidation/replacement deliberately does not mutate a lexical
        // status-line path.
        let _ = artifact_root;
        status::write_entry_status_line_rooted(&signed, &self.artifact)?;
        // Returned, not re-signed by the caller: the document the page serves and the two
        // cache files on disk are then the same bytes with the same signature, and a
        // second `sign_payload` over "the same" payload a moment later would be a second
        // document nobody can tell apart from a stale one.
        Ok(signed)
    }

    /// The event status the round declares, from the daemon config's `[event]` section; until it is
    /// set the runtime reports the entry's own timeline.
    pub fn set_event_status(&mut self, event: EventStatus) {
        self.event = event;
    }

    pub fn entry(&self) -> &Entry {
        &self.entry
    }

    /// Snapshot admission while the caller holds both the admission latch and this
    /// runtime's slot. Tailer positions are their exact current cursors; bytes not yet read
    /// at this instant remain out-of-window evidence.
    ///
    /// `receiver_sources` and `queued_receiver_posts` are written empty and zero. They are
    /// fields of the persisted, `deny_unknown_fields` registry intent and part of its
    /// immutable recovery identity, so they stay on the row with the one value a daemon
    /// without a hook receiver can honestly give.
    pub(super) fn finalization_admission_watermarks(
        &mut self,
    ) -> Result<AdmissionWatermarksV1, DaemonError> {
        // The public StoreCursor is the signed/counting projection, not enough to restore a
        // file offset by itself. Persist the exact private cursor document first so restart
        // reconstructs the same position that the intent below names.
        self.save_cursors()?;
        let entry_id = self.entry.entry_id.clone();
        let tailers = self.finalization_tailer_watermarks();
        let admitted_chain_predecessor = match self.writer_mut()?.head() {
            Some((seq, hash)) => Some(ChainPositionV1 {
                seq,
                hash: goalcup_protocol::provenance::Sha256Digest::try_from(hash).map_err(
                    |error| {
                        DaemonError::Refused(format!(
                            "entry {} has a non-canonical chain head at admission: {error}",
                            entry_id
                        ))
                    },
                )?,
            }),
            None => None,
        };
        Ok(AdmissionWatermarksV1 {
            tailers,
            receiver_sources: BTreeMap::new(),
            queued_receiver_posts: 0,
            admitted_chain_predecessor,
        })
    }

    fn finalization_tailer_watermarks(&self) -> Vec<StoreCursor> {
        let mut watermarks: Vec<_> = self
            .tailers
            .iter()
            .map(|bound| bound.reader.cursor().to_store_cursor(bound.binding.name()))
            .collect();
        watermarks.sort_by(|left, right| left.store.cmp(&right.store));
        watermarks
    }

    /// Prepare the source-root journal and atomically close admission for one V4 run.
    /// The returned source capability exists only for the winning live invocation; a join
    /// or restart must reconcile the durable capture journal without reopening it.
    pub(super) fn begin_workspace_finalization(
        &mut self,
        requested_reason: goalcup_protocol::submission::FinishReasonV1,
        observe_now: impl FnOnce() -> f64,
        use_live_source: bool,
    ) -> Result<Option<goalcup_platform::sourcecap::SourceDir>, DaemonError> {
        if self.entry.workspace_sealing_intent().is_some() {
            return Ok(None);
        }
        self.finalization_receiver_sources = Some(BTreeMap::new());
        let (source, source_root) = if use_live_source {
            let source = self.open_workspace_submission_source()?;
            let journal = crate::submission::prepare_source_journal(&source, &self.artifact)?;
            (
                Some(source),
                crate::registry::WorkspaceSourceBindingV1::Available { journal },
            )
        } else {
            (
                None,
                crate::registry::WorkspaceSourceBindingV1::UnavailableAtCutoff {
                    reason: goalcup_protocol::submission::SubmissionFailureV1::RecoveryAmbiguous,
                },
            )
        };
        let trace_root_identity = self.workspace_writer_identity()?;
        let admission_watermarks = self.finalization_admission_watermarks()?;
        let excluded = self.excluded.clone();
        let source_date = [
            self.vars.date.0.clone(),
            self.vars.date.1.clone(),
            self.vars.date.2.clone(),
        ];
        let entry_id = self.entry.entry_id.clone();
        #[cfg(any(test, feature = "test-hooks"))]
        if std::mem::take(&mut self.fail_before_workspace_intent_cas) {
            return Err(DaemonError::Refused(
                "injected workspace finalization interruption before intent CAS".to_string(),
            ));
        }
        #[cfg(any(test, feature = "test-hooks"))]
        if let Some(pause) = self.workspace_intent_cas_pause.take() {
            pause.pause_after_write();
        }
        let committed = self
            .registry
            .begin_workspace_finalization(&entry_id, |row| {
                let observed_unix = row.observe_finalization_time(observe_now())?;
                let observed = goalcup_protocol::provenance::Rfc3339Utc::try_from(
                    goalcup_protocol::control::format_z(observed_unix),
                )
                .map_err(|error| {
                    DaemonError::Refused(format!(
                        "entry {entry_id} cannot timestamp finalization admission: {error}"
                    ))
                })?;
                let receipt = row.receipt().ok_or_else(|| {
                    DaemonError::Refused(format!(
                        "entry {entry_id} has no verified receipt for V4 finalization"
                    ))
                })?;
                if row.source_date() != Some(&source_date) {
                    return Err(DaemonError::Refused(format!(
                        "entry {entry_id} runtime source date does not match its persisted path authority"
                    )));
                }
                let deadline = goalcup_protocol::provenance::Rfc3339Utc::try_from(
                    receipt.claims.deadline_at.clone(),
                )
                .map_err(|error| {
                    DaemonError::Refused(format!(
                        "entry {entry_id} has an invalid signed deadline: {error}"
                    ))
                })?;
                let (reason, cutoff_at) = crate::registry::resolve_finish_boundary(
                    requested_reason,
                    &observed,
                    &deadline,
                    row.deadline_was_observed(),
                )?;
                let (
                    goalcup_protocol::submission::SubmissionProfileV1::WorkspaceSnapshotV1,
                    limit_profile,
                    limits,
                    capture_limits,
                ) = match row.receipt_binding() {
                    crate::config::ReceiptBindingPolicy::WorkspaceSubmission {
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
                    _ => {
                        return Err(DaemonError::Refused(format!(
                            "entry {entry_id} is not configured for workspace submission"
                        )));
                    }
                };
                Ok(crate::registry::WorkspaceSealingIntentV1 {
                    version: crate::registry::WorkspaceIntentVersionV1::WorkspaceSubmissionV1,
                    entry_id: entry_id.clone(),
                    generation: row.process_handoff_generation(),
                    reason,
                    receipt_sha256: receipt.canonical_digest()?,
                    cutoff_at,
                    trace_root_identity: trace_root_identity.clone(),
                    source_date: source_date.clone(),
                    source_root: source_root.clone(),
                    submission_profile:
                        goalcup_protocol::submission::SubmissionProfileV1::WorkspaceSnapshotV1,
                    limits,
                    limit_profile,
                    capture_limits,
                    excluded: excluded.clone(),
                    phase: crate::registry::FinalizationPhaseV1::AdmissionClosed,
                    admission_watermarks: admission_watermarks.clone(),
                    expected_cutoff_predecessor: None,
                    drain: None,
                    cutoff: None,
                    accepted_handoff: row.verified_registered_termination_registration()?,
                    termination: None,
                    termination_report: None,
                    submission: None,
                    finalization_at: None,
                    expected_finalization_predecessor: None,
                    finalization: None,
                    terminal_status_at: None,
                    status_mirror_complete: false,
                })
            })?;
        self.entry = committed;
        initialize_out_of_window_progress(&self.buffer, &self.manifest, &self.tailers)?;
        Ok(source)
    }

    /// The exact entry artifact authority acquired while this runtime was constructed.
    /// Terminal status paths borrow it rather than resolving the registry display path again.
    pub(super) fn artifact(&self) -> &EntryArtifactRoot {
        &self.artifact
    }

    /// Acquire the live workspace through the bounded no-follow source capability. This is
    /// called only before the finalization CAS; restart recovery uses the source-free
    /// journal reconciliation and never reopens this path as snapshot authority.
    pub(super) fn open_workspace_submission_source(
        &self,
    ) -> Result<goalcup_platform::sourcecap::SourceDir, DaemonError> {
        goalcup_platform::sourcecap::SourceDir::open_selected(
            &self.workspace,
            &goalcup_platform::handle_budget::NativeHandleBudgetV1::new(),
        )
        .map_err(|_| DaemonError::SubmissionCapture {
            reason: goalcup_protocol::submission::SubmissionFailureV1::Unreadable,
        })
    }

    pub(super) fn workspace_trace_root(
        &self,
    ) -> Result<goalcup_platform::tracecap::TraceDir, DaemonError> {
        self.artifact
            .open_trace()
            .map_err(|_| DaemonError::SubmissionCapture {
                reason: goalcup_protocol::submission::SubmissionFailureV1::StorageFailed,
            })
    }

    pub(super) fn workspace_writer_identity(&mut self) -> Result<String, DaemonError> {
        self.writer_mut()?.root_identity()
    }

    pub(super) fn workspace_writer_head(&mut self) -> Result<Option<(u64, String)>, DaemonError> {
        Ok(self.writer_mut()?.head())
    }

    fn out_of_window_limits(&self) -> Result<OutOfWindowLimitsV1, DaemonError> {
        let intent = self.entry.workspace_sealing_intent().ok_or_else(|| {
            DaemonError::Refused(format!(
                "entry {} has no workspace finalization limits for a late post",
                self.entry.entry_id
            ))
        })?;
        Ok(workspace_out_of_window_limits(intent))
    }

    fn persist_out_of_window_nonclean(&mut self) -> Result<(), DaemonError> {
        match self
            .registry
            .record_terminal_drain_validation_failure(&self.entry.entry_id)
        {
            Ok(entry) => {
                self.entry = entry;
                self.terminal_validation_persistence_failed = false;
                Ok(())
            }
            Err(error) => {
                self.terminal_validation_persistence_failed = true;
                Err(error)
            }
        }
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fn pause_before_workspace_capture_for_test(&mut self) {
        if let Some(pause) = self.workspace_capture_pause.take() {
            pause.pause_after_write();
        }
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fn fail_workspace_append_after_for_test(
        &mut self,
        appends_before_failure: u32,
    ) -> Result<(), DaemonError> {
        self.writer_mut()?.fail_append_after(appends_before_failure);
        self.clear_workspace_append_failure_after_drain = true;
        Ok(())
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fn fail_before_workspace_intent_cas_for_test(&mut self) {
        self.fail_before_workspace_intent_cas = true;
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fn out_of_window_stats_for_test(&self) -> Result<(u64, u64), DaemonError> {
        let stats = self
            .buffer
            .verify_out_of_window(self.out_of_window_limits()?)?;
        Ok((stats.records, stats.overflowed))
    }

    pub(super) fn read_workspace_chain_line(&mut self, seq: u64) -> Result<Envelope, DaemonError> {
        self.read_chain_line(seq)
    }

    pub(super) fn buffer_workspace_boundary(&self, envelope: &Envelope) -> Result<(), DaemonError> {
        self.buffer.append(envelope)
    }

    pub(super) fn write_workspace_cutoff(
        &mut self,
        recorded_at: &str,
        cutoff: goalcup_protocol::provenance::CutoffReachedV1,
    ) -> Result<goalcup_protocol::chain::Link, DaemonError> {
        self.writer_mut()?.write_cutoff(recorded_at, cutoff)
    }

    pub(super) fn write_workspace_finalization(
        &mut self,
        recorded_at: &str,
        finalization: goalcup_protocol::provenance::FinalizationV1,
    ) -> Result<goalcup_protocol::chain::Link, DaemonError> {
        self.writer_mut()?
            .write_finalization(recorded_at, finalization)
    }

    pub(super) fn advance_workspace_finalization<F>(
        &mut self,
        expected: crate::registry::FinalizationPhaseV1,
        next: crate::registry::FinalizationPhaseV1,
        update: F,
    ) -> Result<(), DaemonError>
    where
        F: FnOnce(&mut crate::registry::WorkspaceSealingIntentV1) -> Result<(), DaemonError>,
    {
        let entry = self.registry.advance_workspace_finalization(
            &self.entry.entry_id,
            expected,
            next,
            update,
        )?;
        self.entry = entry;
        Ok(())
    }

    pub(super) fn has_persisted_terminal_drain_validation_failure(&self) -> bool {
        self.entry.terminal_drain_validation_failed
    }

    pub(super) fn terminal_validation_persistence_failed(&self) -> bool {
        self.terminal_validation_persistence_failed
    }

    /// How many envelopes the write-ahead buffer holds above the server's watermark.
    ///
    /// Read through the runtime because the buffer is a redb database and redb takes a file
    /// lock: a second `Buffer::open` on the same path, in this process or another, is a
    /// failure and not a second handle. This type owns the entry's buffer, so it is the one
    /// that can answer.
    pub fn pending(&self) -> Result<u64, DaemonError> {
        self.buffer.pending(None)
    }

    pub fn manifest(&self) -> &ConnectorManifest {
        &self.manifest
    }

    pub fn gate_state(&self) -> GateState {
        self.gate.state()
    }

    /// How many stores this run started at offset 0 with a position it was supposed to have
    /// (D19). Zero on a fresh entry, which had none to lose; one per store on a resume that
    /// could not read `cursors.json`, and each of those is a `Reread` in the chain.
    pub fn cursor_restore_failures(&self) -> u64 {
        self.cursor_restore_failures
    }

    /// How many ticks stopped at [`MAX_POLLS_PER_TICK`] with a store still holding more.
    pub fn polls_capped(&self) -> u64 {
        self.polls_capped
    }

    /// How many polls stopped at [`crate::tailer::dir::MAX_SEEN_FILES`] with files this run
    /// never captured still in front of them. See [`PollReport::files_capped`].
    pub fn files_capped(&self) -> u64 {
        self.files_capped
    }

    /// How many records the restart skip budget has spent. See
    /// [`PollReport::skipped_replayed`].
    pub fn skipped_replayed(&self) -> u64 {
        self.skipped_replayed
    }

    pub fn signal_source(&self) -> Option<&'static str> {
        self.gate.signal_source()
    }

    /// The last pump reached the server.
    pub fn connected(&self) -> bool {
        self.connected
    }

    pub fn on_screen(&self) -> String {
        self.gate.on_screen()
    }

    /// The session this run is recorded under: the one the gate watched a leg-1 record
    /// arrive in, falling back to the one the registry stored.
    ///
    /// The gate's is the authority while the run is live, and the entry's is what a restarted
    /// daemon has before its first record.
    pub fn bound_session(&self) -> Option<String> {
        self.gate
            .session_id()
            .map(|s| s.to_string())
            .or_else(|| self.entry.session_id.clone())
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn sweep_foreign_live_for_test(&mut self, now: f64) -> Result<u64, DaemonError> {
        self.sweep_foreign(now, ForeignSweepPhase::LiveHeartbeat)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn sweep_foreign_seal_for_test(&mut self, now: f64) -> Result<u64, DaemonError> {
        self.sweep_foreign(now, ForeignSweepPhase::Seal)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn foreign_pending_capacity_for_test(&self) -> usize {
        MAX_FOREIGN_PENDING_SIGHTINGS
    }

    /// Test-only: stop after the foreign chain event commits and before its exact envelope
    /// reaches the buffer, leaving the real crash window for restart repair coverage.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_foreign_buffer_after_chain_for_test(&mut self) {
        self.fail_foreign_buffer_after_chain_for_test = true;
    }

    /// Test-only: fail the next `attempts` direct buffer repairs after their chain event is
    /// already retained, so retry durability and marker retention can be asserted.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_foreign_buffer_repair_for_test(&mut self, attempts: u32) {
        self.fail_foreign_buffer_repair_for_test = attempts;
    }

    /// Test-only: count the retained repair markers without exposing them to production code.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn foreign_buffer_repair_needed_for_test(&self) -> usize {
        self.foreign_buffer_repair_needed.len()
    }

    /// Test-only: count live/rebuild recovery paths that stream the chain for an envelope.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn foreign_chain_scan_count_for_test(&self) -> u64 {
        self.foreign_chain_scan_count_for_test
    }

    /// Test-only: inspect whether a complete envelope is present in the private buffer.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn buffer_contains_envelope_for_test(
        &self,
        envelope: &Envelope,
    ) -> Result<bool, DaemonError> {
        self.buffer_has_foreign_envelope(envelope)
    }

    /// When the gate saw the connector's declared start signal, if it has.
    pub fn start_signal_at(&self) -> Option<f64> {
        self.gate.start_signal_at()
    }

    /// The connector's `lock_paths[]`, resolved through this entry's own path variables.
    ///
    /// Reported at seal and read before a relaunch: a Codex resume
    /// that cannot take the thread-writer lock silently becomes a new thread, which is a run
    /// whose second half is chained under a session nobody bound.
    pub fn lock_paths(&self) -> Result<Vec<String>, DaemonError> {
        self.manifest
            .lock_paths
            .iter()
            .map(|t| crate::paths::resolve_template(t, &self.vars))
            .collect()
    }

    /// One question the daemon's tick asks after every poll: the declared signal has
    /// been seen, a leg-1 record has arrived since, and no `started_at` is stamped yet.
    ///
    /// Both facts are required; the record is the half that needs the harness to have written
    /// to its own store.
    pub fn may_start(&self) -> bool {
        self.entry.started_at.is_none()
            && self.entry.start_handoff_complete()
            && self.gate.may_stamp_started()
    }

    /// Arm the entry: refuse a harness home that already holds a tracked session, then move
    /// the entry to `Armed` and start the gate's window at `now`.
    ///
    /// The foreign scan is seeded with the canonical workspace, like every other tracked
    /// value in this type: the comparison is byte-exact and `/var/folders/…` is not
    /// `/private/var/folders/…`.
    ///
    /// **Refused once `started_at` is stamped.** Re-arming re-stamps `armed_at` and replaces
    /// the gate, and after dispatch both are facts the receipt witnessed; a second `POST
    /// /arm` on a started run would move the window a signed document already fixed. Before
    /// the start signal a re-arm is still allowed — that is a participant restarting a
    /// launch that never took.
    pub fn arm(&mut self, now: f64) -> Result<(), DaemonError> {
        if self.entry.state == EntryState::Refused {
            return Err(DaemonError::Refused(format!(
                "entry {} was terminally refused and cannot be armed again: the refusal \
                 remains the run's durable evidence",
                self.entry.entry_id
            )));
        }
        if let Some(at) = &self.entry.started_at {
            return Err(DaemonError::Guard(format!(
                "entry {} started at {at} and cannot be armed again: arming stamps \
                 `armed_at` and opens a new gate, and a started run's window is already \
                 witnessed by its receipt",
                self.entry.entry_id
            )));
        }
        if let Some(home) = self.entry.harness_home.clone() {
            crate::foreign::pre_arm_scan(
                &self.manifest,
                &self.vars,
                &self.registry,
                &home,
                &self.entry.entry_id,
            )?;
        }
        self.entry = self.registry.update(&self.entry.entry_id, |e| {
            e.state = EntryState::Armed;
            e.armed_at = Some(now);
            // The previous launch's start signal and the session it named are
            // not facts about this window. They must be cleared in the same update, so the
            // refreshed `self.entry` mirror below carries the cleared value and
            // `sync_head`'s `(None, Some(at))` stamp can fire for the next signal — clearing
            // the row afterwards would leave the mirror holding the old one, and clearing it
            // beforehand would erase the record if the pre-arm scan or the `started_at`
            // guard then refused the re-arm.
            e.start_signal_at = None;
            e.session_id = None;
            // A re-arm abandons the previous launch window. Clear its deferred own-config
            // identities in this same registry transaction; the in-memory mirror is cleared
            // only after this durable transition succeeds below.
            e.foreign_pending.clear();
        })?;
        self.foreign_pending.clear();
        for repair in self.foreign_buffer_repair_needed.values_mut() {
            repair.keep_until_pending = false;
        }
        // The generation is daemon-issued at creation but is not authority until this
        // armed transition has durably opened it.  Attach mode remains closed; its first
        // launchable resume is handled by the explicit resume CAS.
        if matches!(
            self.entry.expected_process_handoff(),
            crate::registry::ExpectedProcessHandoff::Required
        ) && matches!(
            self.entry.process_handoff_phase(),
            ProcessHandoffPhase::InitialClosed
        ) {
            self.entry = self
                .registry
                .open_initial_process_handoff(&self.entry.entry_id)?;
        }
        self.gate =
            ArmingGate::with_timeout(&self.manifest, now, self.entry.arming_timeout_seconds);
        // The premise a `Direct` store's teach-on-first-sight rests on is exactly the one
        // this method has just established: the workspace is the run's own and
        // `pre_arm_scan` has refused a harness home holding a session already working in it,
        // so between here and the bind the store's first session id is the participant's.
        self.teach_direct_stores_on_first_sight();
        Ok(())
    }

    /// Turn on [`Partitioner::teach_on_first_sight`] for every `Direct` store.
    ///
    /// **Two callers, and both of them have the premise.** [`EntryRuntime::arm`] has just
    /// run the pre-arm scan; [`EntryRuntime::new`] calls it only for a row that is already
    /// armed and not yet started, which is the rehydration path — the scan ran before the
    /// restart, `armed_at` is the fact that says so, and a rebuilt runtime that did not turn
    /// the mode back on would lose the pre-bind records of every restarted run.
    ///
    /// It is not armed at construction for a fresh entry, which is created before the scan
    /// has licensed anything, and the mode is never turned on for a started run: from the
    /// bind onwards the tracked set is the bound session's and a second teach would be this
    /// type choosing a session.
    fn teach_direct_stores_on_first_sight(&mut self) {
        for store in &self.stores {
            if matches!(store.partition, Some(PartitionSpec::Direct { .. }))
                && let Some(p) = self.partitioners.get_mut(&store.name)
            {
                p.teach_on_first_sight();
            }
        }
    }

    /// Prepare then register the exact daemon-issued process generation.  `Prepared` is
    /// durable before the event append; a restart can therefore prove which one request it
    /// is reconciling instead of accepting whichever retry reaches `/process` next.
    pub fn set_process(&mut self, registration: ProcessRegistration) -> Result<(), DaemonError> {
        let generation = registration.generation;
        self.entry = self
            .registry
            .prepare_process_handoff(&self.entry.entry_id, registration)?;
        if matches!(
            self.entry.process_handoff_phase(),
            ProcessHandoffPhase::Registered
        ) {
            return Ok(());
        }
        self.reconcile_prepared_process_handoff_for(generation)
    }

    /// Finish a durable `Prepared` handoff after this runtime has an open validated trace.
    /// It is called by the original `/process` request and during daemon rehydration before
    /// an entry can be sealed: a process accepted before a crash must become the seal's
    /// target, or an ambiguous marker must refuse the seal rather than orphan it.
    pub fn reconcile_prepared_process_handoff(&mut self) -> Result<(), DaemonError> {
        if !matches!(
            self.entry.process_handoff_phase(),
            ProcessHandoffPhase::Prepared
        ) {
            return Ok(());
        }
        self.reconcile_prepared_process_handoff_for(self.entry.process_handoff_generation())
    }

    fn reconcile_prepared_process_handoff_for(
        &mut self,
        generation: u64,
    ) -> Result<(), DaemonError> {
        let prepared = self
            .registry
            .prepared_process_handle(&self.entry.entry_id, generation)?;
        goalcup_protocol::process::validate_live_process_handle(&prepared).map_err(|reason| {
            DaemonError::Refused(format!(
                "entry {}'s prepared process could not be revalidated: {reason}",
                self.entry.entry_id
            ))
        })?;
        let Some((digest, mut seed)) = self
            .registry
            .prepared_home_seed(&self.entry.entry_id, generation)?
        else {
            self.entry = self
                .registry
                .register_prepared_process_handoff(&self.entry.entry_id, generation)?;
            return Ok(());
        };

        match self.home_seed_marker(generation, &digest)? {
            (0, None) => {
                seed.handoff_generation = Some(generation);
                seed.handoff_request_digest = Some(digest);
                self.chain_home_seed(seed)?;
            }
            (1, Some(envelope)) => {
                // A daemon-process crash can land the chain append before `Buffer::append`.
                // Re-inserting by seq is intentionally idempotent, so a restart repairs the
                // missing encrypted WAL/upload copy without writing a second chain event.
                // This is not a sudden-power-loss claim: neither store is fsync-ordered for
                // power-cut durability.
                self.buffer.append(&envelope)?;
            }
            (found, _) => {
                return Err(DaemonError::Refused(format!(
                    "entry {} has {found} HomeSeeded markers for process generation {generation}; \
                     crash recovery refuses to choose one",
                    self.entry.entry_id
                )));
            }
        }
        self.entry = self
            .registry
            .register_prepared_process_handoff(&self.entry.entry_id, generation)?;
        Ok(())
    }

    /// Return an issued resume permit before a replacement child has been accepted.  The
    /// registry keeps the prior `last_process` authority in place; if a request was already
    /// prepared, this method's caller first proves that exact child is gone.
    pub fn cancel_resume_process_handoff(
        &mut self,
        generation: u64,
        request_digest: Option<&str>,
    ) -> Result<Entry, DaemonError> {
        if let Some(target) = self.registry.prepared_resume_termination_for_cancel(
            &self.entry.entry_id,
            generation,
            request_digest,
        )? {
            super::seal::end_harness_verified(target.handle())?;
        }
        self.entry = self.registry.finish_cancel_resume_handoff(
            &self.entry.entry_id,
            generation,
            request_digest,
        )?;
        Ok(self.entry.clone())
    }

    /// Chain what this launch's `[home_seed]` carried into the per-run harness home, and
    /// buffer it like every other event.
    ///
    /// Raw and nothing else: the CLI measured it, the daemon writes it down, and no rule in
    /// this crate reads it (D3). It is written from `register_process`, which a start calls
    /// once and a resume calls once after the standing handle has been cleared — so a run
    /// resumed twice normally carries three of these, in launch order, and the chain says
    /// which settings each half of the run was given. At most one per launch that reached
    /// the registration: a relaunch whose own re-seed failed hands `None` and chains
    /// nothing, because a record of a seeding that did not happen would be a claim.
    pub fn chain_home_seed(&mut self, record: HomeSeedRecord) -> Result<(), DaemonError> {
        let env = self.writer_mut()?.event(EventBody::HomeSeeded(record))?;
        self.buffer.append(&env)?;
        Ok(())
    }

    /// Find this prepared handoff's correlation marker in a complete, independently
    /// revalidated chain.  The marker is generation plus a canonical *process* digest; the
    /// digest excludes seed content, so this scan does not derive a public identifier from a
    /// copied auth/config value.  The exact validated envelope is returned for the
    /// chain-before-buffer daemon-process crash repair; more than one is not guessed through.
    ///
    /// This reconciles ordinary process crash/restart only.  `ChainWriter::event` and the
    /// redb append are not power-cut/fsync ordered, so it intentionally makes no claim about
    /// a sudden loss of power between either durable write.
    fn home_seed_marker(
        &mut self,
        generation: u64,
        digest: &str,
    ) -> Result<(u8, Option<Envelope>), DaemonError> {
        use std::io::BufRead;

        let diagnostic = self.entry.trace_dir.join(CHAIN_FILE);
        let reader = self.writer_mut()?.independent_chain_reader()?;
        let mut expected_seq = 0u64;
        let mut previous = goalcup_protocol::chain::GENESIS.to_string();
        let mut count = 0u8;
        let mut unique = None;
        for line in std::io::BufReader::new(reader).lines() {
            let line = line.map_err(|source| DaemonError::Io {
                path: diagnostic.display().to_string(),
                source,
            })?;
            let env: Envelope = serde_json::from_str(&line)?;
            if env.entry_id != self.entry.entry_id
                || env.link.seq != expected_seq
                || env.link.prev != previous
                || env.link.hash != env.expected_hash(&previous)?
            {
                return Err(DaemonError::Refused(format!(
                    "{} is not a validated contiguous chain while reconciling process handoff",
                    diagnostic.display()
                )));
            }
            if let EventBody::HomeSeeded(seed) = &env.body
                && seed.handoff_generation == Some(generation)
                && seed.handoff_request_digest.as_deref() == Some(digest)
            {
                count = count.checked_add(1).ok_or_else(|| {
                    DaemonError::Refused("HomeSeeded marker count overflowed".to_string())
                })?;
                if count == 1 {
                    unique = Some(env.clone());
                }
            }
            expected_seq = expected_seq.checked_add(1).ok_or_else(|| {
                DaemonError::Refused(
                    "chain sequence overflowed while reconciling process handoff".to_string(),
                )
            })?;
            previous = env.link.hash;
        }
        Ok((count, unique))
    }

    /// Forget the registered handle, so the harness a resume is about to relaunch can be
    /// registered in its place.
    ///
    /// **One caller, and it is the resume.** `Daemon::register_process` refuses while a
    /// handle stands, which is what keeps a second one posted on the loopback from
    /// redirecting `harness_alive` and the seal's terminate target; a resume is the one
    /// moment the CLI legitimately replaces the process this entry names, so it is the one
    /// place the slot is opened. Cleared anywhere else, that refusal would be given away.
    ///
    /// What it costs until the relaunch registers: `same_process` answers `true` — with no
    /// handle there is nothing to contradict — and `agent_exited` is false, which is the
    /// reading an entry that never launched one already gets. Both are honest: the process
    /// this entry knew about is the one the participant is in the middle of replacing.
    ///
    /// The registry copy goes with it in the same fallible update. A true `attach_only`
    /// connector has no launch recipe to resume and is refused before that transaction;
    /// an attached invocation of a launchable `tui` connector is promoted to `Required`
    /// because resume relaunches its process.
    pub fn clear_process(&mut self, expected_generation: u64) -> Result<Entry, DaemonError> {
        self.ensure_resume_can_launch()?;
        self.entry = self
            .registry
            .open_resume_process_handoff(&self.entry.entry_id, expected_generation)?;
        Ok(self.entry.clone())
    }

    /// Refuse a resume only when the connector itself has no launch recipe. The persisted
    /// `NotRequired` value is insufficient: it also represents `goalcup start --attach` on
    /// a launchable `tui` connector, which a resume must relaunch.
    pub fn ensure_resume_can_launch(&self) -> Result<(), DaemonError> {
        if self.manifest.launch.mode == LaunchMode::AttachOnly {
            return Err(DaemonError::Refused(format!(
                "entry {} uses an attach_only connector and has no launch recipe to resume",
                self.entry.entry_id
            )));
        }
        Ok(())
    }

    /// Whether `now` is past the deadline that was parsed once and stored.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn deadline_passed(&self, now: f64) -> bool {
        self.entry.deadline_unix.is_some_and(|d| now >= d)
    }

    /// True when the registered process is alive and its program and argv still match what
    /// the CLI recorded. `ps` failing to answer is "unknown", never "mismatch".
    ///
    /// **No registered process is `true` only where none was owed** — attach mode, and an
    /// entry that has not started. There is nothing to contradict, and reporting a run that
    /// never launched a harness as "the harness is gone" would be an assertion about a
    /// measurement nobody took.
    ///
    /// Where one was owed it is `false`. A started `tui`/`server` entry with an
    /// empty slot is a handle that was lost — the resume cleared it and the relaunch never
    /// registered, a `set_process` registry write did not land, or the daemon died between
    /// the `HomeSeeded` record and the registration — and answering `true` there would make the
    /// health ladder report a harness it had stopped measuring as alive, which would put
    /// `HealthState::AgentExited`, the rung that tells the participant to run `goalcup
    /// resume`, out of reach for the rest of the run. `false` is not "the harness is dead";
    /// it is "this daemon cannot say it is the one we launched", which is what
    /// `health::evaluate` reads it as.
    pub fn same_process(&self) -> bool {
        match &self.entry.last_process {
            None => !self.owes_a_process_handle(),
            #[cfg(unix)]
            Some(handle) => crate::terminate::same_process(handle),
            // Windows process identity is not compared: this platform measures liveness and nothing
            // else.
            #[cfg(not(unix))]
            Some(handle) => is_alive(handle.pid),
        }
    }

    /// Should this entry have a registered process handle?
    ///
    /// A launch-mode connector that has started owes one: the CLI spawned a harness and
    /// registered it, and ending that harness is the daemon's promise. An
    /// `attach_only` connector spawns nothing, and an entry that has not started has not
    /// launched anything yet — for those, an empty slot is the expected state and not a
    /// loss. **No shipped v1 connector is `attach_only`**: both are `mode = "tui"`.
    pub fn owes_a_process_handle(&self) -> bool {
        if self.entry.started_at.is_none() {
            return false;
        }
        match self.entry.expected_process_handoff() {
            crate::registry::ExpectedProcessHandoff::NotRequired => false,
            crate::registry::ExpectedProcessHandoff::Required => true,
            crate::registry::ExpectedProcessHandoff::LegacyUnknown => {
                self.manifest.launch.mode != LaunchMode::AttachOnly
            }
        }
    }

    /// The entry's half of the seal, in the order the rule fixes: the workspace manifest, then
    /// the second foreign scan, then the chained seal and `seal.json`.
    ///
    /// After the authoritative receipt-mirror gate, **the manifest is taken before any
    /// other seal work**, because everything after it either reads somebody else's disk or
    /// ends the harness, and both race the files this walk is describing.
    ///
    /// **The scan is second, and it is the last thing chained under the seal.** A foreign
    /// session that appeared in the bound workspace during the run is evidence about the
    /// run, so it belongs *below* the seal link rather than after it — and after
    /// [`ChainWriter::write_seal`] there is no "after": every further `event` or `leg1` on
    /// this entry is `ExportError::Sealed`.
    ///
    /// **The seal itself is chained by the writer and not here.** `write_seal` appends
    /// `EventBody::Seal { manifest_sha256, sealed_at, file_count }` as the entry's last link
    /// and only then writes `seal.json`, so the seal is covered by the same link hash every
    /// record is and the last acknowledgement a server issues anchors it too. A `seal.json`
    /// with nothing over it in the chain is a file at a participant-named path.
    ///
    /// **The receipt that may still be owed is paid first.** The retry otherwise runs only
    /// at the top of a tick, and a seal is not a tick — nor is anything after one, because
    /// the chain's last link is already written. A run sealed with `receipt_pending` set
    /// would rewrite the descriptor below from a state that had no `receipt.json` in it and
    /// then close, and V13 would read that trace as `receipt_missing` for good, over a
    /// receipt this daemon had verified, recorded and merely failed to write once. Nothing
    /// is lost when there is no debt: the retry is a no-op.
    ///
    /// `Daemon::seal_entry` drives the rest of the shutdown: it terminates the harness tree
    /// after this returns, which is to say after the manifest is durable.
    pub fn seal(&mut self) -> Result<SealManifest, DaemonError> {
        // A previous in-process attempt can fail after the durable intent is written. Do
        // not reopen its pathname: finish only through the writer capability that staged
        // the intent, or leave capture closed for startup to acquire and validate a fresh
        // root.
        if self.entry.state == EntryState::Sealing {
            let staged = self.entry.sealing_intent().cloned().ok_or_else(|| {
                DaemonError::Refused(format!(
                    "entry {} is sealing without an exact staged manifest",
                    self.entry.entry_id
                ))
            })?;
            let manifest = staged.legacy()?.manifest.clone();
            self.entry = self.finish_live_sealing_intent(&staged)?;
            return Ok(manifest);
        }
        if self.reconcile_receipt_mirror()? {
            self.refresh_descriptor_and_commit_receipt()?;
        }
        let sealed_at = now_rfc3339();
        let manifest = crate::workspace::list_workspace(
            &self.workspace,
            &self.entry.entry_id,
            &sealed_at,
            &self.excluded,
        )?;
        self.sweep_foreign(now_unix(), ForeignSweepPhase::Seal)?;
        // This is the one pre-writer boundary every seal path shares, including direct
        // runtime callers used by recovery and lifecycle tests.  A crash on either side of
        // the terminal writer is therefore reconciled from a durable intent at startup.
        let mut descriptor = self.descriptor_doc();
        descriptor.seal = Some(goalcup_protocol::export::SEAL_FILE.to_string());
        // Take the writer's retained, validated identity before the registry reserves the
        // intent. A recovery path may only accept the one terminal link that extends this
        // exact predecessor, never a later path-swapped but self-consistent history.
        let (pre_seal_head, trace_root_identity) = {
            let writer = self.writer_mut()?;
            (writer.head(), writer.root_identity()?)
        };
        let intent = SealingIntent::new(
            manifest.clone(),
            descriptor,
            pre_seal_head,
            trace_root_identity,
        )?;
        self.begin_sealing(intent.clone())?;

        // `write_seal` can report after it has appended the terminal.  In both outcomes,
        // inspect that one expected link through the same root+chain capability; a failure
        // before it existed remains durably `Sealing` rather than reopening capture here.
        let write = self.writer_mut()?.write_seal(&manifest);
        #[cfg(any(test, feature = "test-hooks"))]
        if write.is_ok()
            && let Some(pause) = self.live_seal_write_pause.take()
        {
            pause.pause_after_write();
        }
        match self.finish_live_sealing_intent(&intent) {
            Ok(entry) => {
                self.entry = entry;
                Ok(manifest)
            }
            Err(recovery) => match write {
                Ok(_) => Err(recovery),
                Err(original) => Err(DaemonError::Refused(format!(
                    "seal writer failed and its staged terminal could not be finalized without reopening the trace path: {original}; {recovery}"
                ))),
            },
        }
    }

    /// Complete an already-staged seal only through this runtime's retained trace root and
    /// append-stream identity.  Startup owns no such capability and uses the separately
    /// staged root identity instead; a live runtime must never reacquire a path after a
    /// writer fault because that could turn a swapped directory into new capture authority.
    fn finish_live_sealing_intent(&mut self, intent: &SealingIntent) -> Result<Entry, DaemonError> {
        intent.validate(&self.entry.entry_id)?;
        intent.matches_trace_root(&self.writer_mut()?.root_identity()?)?;
        let terminal = self.read_chain_line(intent.expected_terminal_seq()?)?;
        intent.matches_terminal(&terminal)?;
        let legacy = intent.legacy()?;
        self.buffer.append(&terminal)?;
        self.writer_mut()?
            .repair_seal(&legacy.manifest, &terminal)?;
        self.writer_mut()?.write_descriptor(&legacy.descriptor)?;
        // The writer and the artifact owner still name the staged directory. Reopen its
        // final `trace` child through that retained entry capability, never by the
        // registry-visible spelling: an entry/ancestor replacement may strand later
        // restart recovery, but it cannot redirect this live promotion into the replacement.
        let reopened = TraceRoot::from_acquired(
            self.artifact.open_trace().map_err(|error| {
                DaemonError::Refused(format!(
                    "live sealing trace cannot be reopened through retained entry artifact authority: {error}"
                ))
            })?,
            self.entry.trace_dir.clone(),
        );
        intent.matches_trace_root(&reopened.identity()?)?;
        self.registry
            .record_sealing_terminal_head(&self.entry.entry_id, &terminal)?;
        self.registry
            .promote_sealing_terminal(&self.entry.entry_id, &terminal)
    }

    /// Persist the registry half of a seal before the terminal writer is touched.  If this
    /// process dies on either side of `write_seal`, startup reconciles the state from the
    /// validated chain rather than choosing between a live harness and a terminal row.
    pub fn begin_sealing(&mut self, intent: SealingIntent) -> Result<(), DaemonError> {
        self.entry = self.registry.begin_sealing(&self.entry.entry_id, intent)?;
        Ok(())
    }

    /// What ending the harness cost, into this runtime's own copy of the registry row.
    ///
    /// **In memory only, and after the caller has written the row.** The kill runs in
    /// `Daemon::seal_entry`, after [`EntryRuntime::seal`] has returned, so `seal()` cannot
    /// know the answer; the runtime meanwhile goes on building `status` from the copy of the
    /// row it read before the kill, and the document published under the seal's own hold is
    /// the one a participant reads about a finished run. Without this, a seal that could not
    /// vouch for the harness said so on disk and nowhere a reader looks.
    ///
    /// A registry write here would be a second write of a row the caller has just put there,
    /// and — because a concurrent pump may have evicted this entry by now — one more place
    /// for the seal to fail after the chain is already closed.
    pub fn note_seal_outcome(&mut self, outcome: SealOutcome) {
        self.entry.seal_outcome = Some(outcome);
    }

    // ---------------------------------------------------------------------------------
    // The pipeline, one record at a time.
    // ---------------------------------------------------------------------------------

    /// D20's order for one record: judge, teach, redact, hash, buffer.
    ///
    /// **Teaching happens whether or not the record was kept.** A `session.created.1` row
    /// naming another directory teaches the learners a value they then compare and discard
    /// against, which is the strategy working; skipping it would leave their aggregates
    /// unknown and the discard reason wrong.
    ///
    /// **…and so does everything else a record leaves behind on the way.** A record
    /// this process is re-reading because the previous one already chained it still runs
    /// the partitioner, the teach and `session_of`, because the state those build is what
    /// the records after it are judged and attributed by. What it does not do is take a
    /// second chain link.
    fn ingest_record(
        &mut self,
        record: &CapturedRecord,
        now: f64,
        counts: &mut Counts,
    ) -> Result<(), DaemonError> {
        let binding = self.bindings.get(&record.store).cloned().ok_or_else(|| {
            DaemonError::Refused(format!(
                "record from inactive or unknown binding `{}` cannot be admitted",
                record.store
            ))
        })?;
        let outcome = self
            .partitioners
            .get_mut(&record.store)
            .ok_or_else(|| {
                DaemonError::Refused(format!(
                    "active binding `{}` has no partitioner",
                    record.store
                ))
            })?
            .admit(record);
        // A derived child has an independent partitioner, but it never teaches aggregate
        // state into a primary store. The parent identity comes from the declaration, not
        // from the generated reader name.
        if binding.kind() == SourceKind::Primary {
            partition::teach(
                &mut self.partitioners,
                binding.parent_store(),
                record,
                &self.stores,
            );
        }
        if let Outcome::Discard(_) = outcome {
            // Includes `UndecidedFile`: pre-decision bytes leave one bounded count here
            // and no raw holding buffer, digest, log, chain link or upload envelope.
            counts.discarded += 1;
            return Ok(());
        }

        let session = self.session_of(record, &binding);
        // Already in the chain, from before the restart. Counted as neither captured nor
        // discarded: it is one record, chained once, by the process that read it first, and
        // the `Reread` witness `open_trace` chained says the tailer read it again.
        if self.resume_skip_taken(&record.store) {
            return Ok(());
        }
        self.sample_version(record, now)?;

        // Redaction and the link, in one call, over the redacted payload. `record.bytes`
        // are the redactor's input and reach no hash.
        let envelopes = {
            let writer = self.writer_mut()?;
            writer.leg1(record)?
        };
        let chained_seq = envelopes.first().map(|e| e.link.seq);
        for env in &envelopes {
            self.buffer.append(env)?;
        }
        counts.captured += 1;
        counts.chained += envelopes.len() as u64;
        self.last_leg1_at = Some(now);

        // Where this store's chain records now end, saved beside its position so that the
        // next restart can count what it re-reads.
        if let Some(seq) = chained_seq {
            self.last_leg1_seq.insert(record.store.clone(), seq);
        }

        // The gate is fed from the chained output and never from a raw read: a `started_at`
        // no chain link points back at is a claim about a record nobody can find again.
        if let (Some(seq), Some(session)) = (chained_seq, session) {
            self.gate.observe_leg1(&record.store, seq, &session, now);
        }
        Ok(())
    }

    /// The session a record belongs to, by the connector's own declaration.
    ///
    /// `foreign_scan[].session_id_field` for this harness is where a connector says which
    /// field names a session — the same field [`crate::foreign::pre_arm_scan`] reads, so
    /// the scan and the capture cannot disagree. Codex names the session once, in the
    /// `session_meta` that opens each rollout, so the value is remembered per file and the
    /// records that follow inherit it.
    ///
    /// **The lookup descends through a JSON-encoded TEXT column.** OpenCode's `event.data` is
    /// TEXT holding JSON and the session id lives inside it, so a dotted lookup that stops at
    /// the string node resolves nothing: `session_of` would answer `None` for every record of
    /// an OpenCode run, [`ArmingGate::observe_leg1`] would never be called, and every entry
    /// would time out at `arming.timeout_seconds` having captured a full transcript.
    fn session_of(&mut self, record: &CapturedRecord, binding: &SourceBinding) -> Option<String> {
        // A derived path reached only through a concrete session component has exactly that
        // session's authority. Do not infer ownership from `*_subagent` naming or trust a
        // child payload to reassign it.
        if binding.kind() != SourceKind::Primary {
            return self
                .entry
                .session_id
                .clone()
                .or_else(|| self.gate.session_id().map(str::to_string));
        }
        if let Some(field) = self.session_id_field()
            && let Some(found) = str_at(&record.value, &field).map(|s| s.into_owned())
        {
            // Bounded. At the cap the memo is not extended: the record in hand named
            // its own session and needs no memo, and a later record of this file that names
            // none falls back to the entry's bound session.
            if self.sessions_by_path.len() < MAX_SESSIONS_BY_PATH
                || self.sessions_by_path.contains_key(&record.source_path)
            {
                self.sessions_by_path
                    .insert(record.source_path.clone(), found.clone());
            }
            return Some(found);
        }
        self.sessions_by_path
            .get(&record.source_path)
            .cloned()
            .or_else(|| self.entry.session_id.clone())
    }

    /// Forget the per-file session memo for files that are no longer there.
    ///
    /// Once a tick, not once a record: it stats one file per remembered path, and the fact
    /// it is looking for — a transcript the harness rotated away — does not change inside a
    /// tick. A path that comes back is a new file and teaches the memo again from its own
    /// header, which is what a file this runtime had never seen already does.
    fn evict_missing_sessions(&mut self) {
        if self.sessions_by_path.is_empty() {
            return;
        }
        self.sessions_by_path
            .retain(|path, _| std::fs::metadata(path).is_ok());
    }

    /// How many files the per-file session memo remembers, readable.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn sessions_by_path_len(&self) -> usize {
        self.sessions_by_path.len()
    }

    fn session_id_field(&self) -> Option<String> {
        file_session_id_field(&self.manifest)
    }

    /// D17's signal 5, sampled only where the connector declares `version.self_updating`.
    /// A harness that cannot update itself mid-run has no version to re-read, and reading
    /// one anyway would chain a `VersionChange` for every connector that spells its version
    /// differently in two records.
    fn sample_version(&mut self, record: &CapturedRecord, now: f64) -> Result<(), DaemonError> {
        if !self.manifest.version.self_updating {
            return Ok(());
        }
        let Some(found) = tailer::dotted_json(&record.value, &self.manifest.version.record_field)
            .and_then(|v| v.as_str())
        else {
            return Ok(());
        };
        if found == self.entry.harness_version {
            return Ok(());
        }
        let change = VersionChange {
            component: self.manifest.id.clone(),
            from: self.entry.harness_version.clone(),
            to: found.to_string(),
            detected_at: unix_to_rfc3339(now),
        };
        let to = found.to_string();
        let env = {
            let writer = self.writer_mut()?;
            writer.event(EventBody::VersionChange(change))?
        };
        self.buffer.append(&env)?;
        self.version_changed = true;
        self.entry = self
            .registry
            .update(&self.entry.entry_id, |e| e.harness_version = to.clone())?;
        Ok(())
    }

    /// The `trace.toml` document as the run stands now. One builder, so `open_trace`'s
    /// first write and every [`EntryRuntime::refresh_descriptor`] after it cannot drift
    /// apart in what they declare.
    fn descriptor_doc(&self) -> TraceDescriptorDoc {
        TraceDescriptorDoc {
            connector: self.entry.connector_id.clone(),
            entry_id: self.entry.entry_id.clone(),
            workspace: self.workspace.display().to_string(),
            tracked_sessions: self.tracked_sessions(),
            harness_version: self.entry.harness_version.clone(),
            connector_manifest: self.connector_manifest.clone(),
            chain: Some(CHAIN_FILE.to_string()),
            // Named from the registry and not from the directory: the registry's checked
            // receipt commit is what verified it against the pinned server key, and a
            // file that merely exists beside the chain is not one anything verified.
            //
            // …and not named at all while the write is still owed. A descriptor that
            // names a `receipt.json` the disk does not hold is a trace that will not load,
            // which is a worse answer than a trace whose receipt has not arrived yet.
            receipt: matches!(
                self.receipt_mirror,
                ReceiptMirrorState::Ready
                    | ReceiptMirrorState::Owed {
                        mirror_ready: true,
                        ..
                    }
            )
            .then(|| goalcup_protocol::export::RECEIPT_FILE.to_string()),
            baseline: Some(goalcup_protocol::export::BASELINE_FILE.to_string()),
            acks: Some(goalcup_protocol::export::ACKS_FILE.to_string()),
            seal: self
                .entry
                .sealed_at
                .as_ref()
                .map(|_| goalcup_protocol::export::SEAL_FILE.to_string()),
            server_public_key: Some(self.pinned_server_key.clone()),
            expected_reread: false,
        }
    }

    /// The sessions this trace claims to cover, in the order they were learned.
    ///
    /// The gate's session is the one the daemon watched a leg-1 record arrive in, and
    /// [`record_start`] copies it onto the entry; both are read here because the entry is
    /// the durable half (it survives a restart) and the gate is the live half (it has the
    /// session before `record_start` runs). They name the same session on every path that
    /// reaches a start, and the list is deduplicated rather than assumed.
    fn tracked_sessions(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let candidates = [
            self.entry.session_id.clone(),
            self.gate.session_id().map(|s| s.to_string()),
        ];
        for session in candidates.into_iter().flatten() {
            if !out.contains(&session) {
                out.push(session);
            }
        }
        out
    }

    /// One `Reread` witness per store that is about to be read again (D19).
    ///
    /// Two ways a store gets one, and they are the same fact from the tailer's side —
    /// bytes it has already shipped are coming back — so they are one witness and not two
    /// kinds.
    ///
    /// **Its position did not come back.** A tailer with no cursor to restore starts at
    /// zero and emits nothing: it has no offset to have gone back from. The chain of a
    /// daemon that lost `cursors.json` would show a `DaemonRestart` followed by a second
    /// complete copy of every transcript and nothing saying why. V9 spends the restart on the first
    /// store's notice; every store after it reads as an unexplained truncation, at Fail, on a run
    /// that did nothing wrong. That store re-reads in full, so it is witnessed and nothing is
    /// skipped, and it is the one that counts a `cursor_restore_failure`.
    ///
    /// **Its position came back behind the chain.** `cursors.json` is saved per batch, so a
    /// kill part-way through one leaves it at the batch's start with `k` of that batch's
    /// records already chained. That re-read is just as silent, and `resume_skip` says
    /// exactly how many records it will replay — a number counted off the chain, so a
    /// non-empty budget is the statement that the store is about to be read again. Those
    /// `k` are dropped rather than chained twice, and the witness says the read happened.
    ///
    /// Chained from [`EntryRuntime::open_trace`]'s resume branch, after the `DaemonRestart`
    /// envelope and before any tailer polls, for the reason that method's doc gives: a
    /// witness above the thing it explains explains nothing, and one below it has already
    /// been spent.
    fn chain_cursor_rereads(&mut self) -> Result<(), DaemonError> {
        for store in std::mem::take(&mut self.unrestored_stores) {
            self.chain_reread_witness(&store)?;
            self.cursor_restore_failures += 1;
        }
        let behind: Vec<String> = self.resume_skip.keys().cloned().collect();
        for store in behind {
            self.chain_reread_witness(&store)?;
        }
        Ok(())
    }

    /// Whether this record is one the previous process already chained, and so is read but
    /// not chained again.
    ///
    /// Spent on records the partitioner kept and not on records read: a batch mixes kept
    /// and discarded rows, only the kept ones took a chain link, and a budget spent by raw
    /// index would land on the wrong records the moment one was discarded.
    ///
    /// **The subtraction is saturating, and the entry is removed the moment it reaches
    /// zero.** Nothing here can drive it negative — every taken
    /// unit was checked present first — but a saturating operator over a bare `-= 1` costs
    /// nothing and is the honest spelling of "never below zero" for a budget the run's
    /// health depends on reading right. Every unit actually spent is counted into
    /// [`EntryRuntime::skipped_replayed`], because a budget computed too wide is otherwise
    /// invisible: the record it swallows is neither `captured` nor `discarded`, and this is
    /// the one place its cost surfaces.
    fn resume_skip_taken(&mut self, store: &str) -> bool {
        let Some(remaining) = self.resume_skip.get_mut(store) else {
            return false;
        };
        *remaining = remaining.saturating_sub(1);
        if *remaining == 0 {
            self.resume_skip.remove(store);
        }
        self.skipped_replayed += 1;
        true
    }

    /// One `Reread` notice for `store`, chained and buffered.
    ///
    /// The one construction, read by both paths that witness a re-read — the lost cursor
    /// ([`EntryRuntime::chain_cursor_rereads`]) and the restored one that sat behind the
    /// chain — so the fields a verifier reads cannot come apart between them. `offset_before` is 0 for the same reason in both: what the
    /// tailer is about to hand back again begins at the start of what it reads.
    fn chain_reread_witness(&mut self, store: &str) -> Result<(), DaemonError> {
        let path = self.store_path(store);
        let Some(body) = (TailNotice::Reread {
            store: store.to_string(),
            path,
            offset_before: 0,
        })
        .to_event(now_rfc3339()) else {
            return Ok(());
        };
        let env = self.writer_mut()?.event(body)?;
        self.buffer.append(&env)?;
        Ok(())
    }

    /// Where a store's files are, as the connector declares them. The pattern itself when
    /// it will not render: a notice naming an unresolved template is still a notice naming
    /// the store, and refusing to chain the witness over a rendering failure would trade a
    /// cosmetic path for a Fail.
    fn store_path(&self, store: &str) -> String {
        if let Some(binding) = self.bindings.get(store) {
            return binding.resolved_pattern().to_string();
        }
        self.stores
            .iter()
            .find(|s| s.name == store)
            .map(|s| {
                crate::paths::resolve_template(&s.path_template, &self.vars)
                    .unwrap_or_else(|_| s.path_template.clone())
            })
            .unwrap_or_default()
    }

    /// Exactly one [`StoreCursor`] per store, plus the write-ahead position beside them.
    ///
    /// `gaps::cursor_for` takes the first match by name, so a second cursor for one store
    /// would be invisible — and if it were the first, it would displace the one that counts
    /// records.
    fn store_cursors(&self) -> Result<Vec<StoreCursor>, DaemonError> {
        let mut out: Vec<StoreCursor> = self
            .tailers
            .iter()
            .map(|bound| bound.reader.cursor().to_store_cursor(bound.binding.name()))
            .collect();
        if !self.stores.iter().any(|s| s.name == BUFFER_CURSOR) {
            let head = self.buffer.head()?.map(|(seq, _)| seq);
            let floor = self.buffer.floor()?;
            let pending = self.buffer.pending(None)?;
            out.push(StoreCursor {
                store: BUFFER_CURSOR.to_string(),
                // Opaque: a chain sequence is not a count of anything in a window, and
                // `records()` must never hand it to arithmetic that compares one.
                unit: CursorUnit::Opaque,
                position: head,
                describe: format!(
                    "buffered:{},acked:{},pending:{pending}",
                    head.map(|s| s.to_string()).unwrap_or_else(|| "none".into()),
                    floor
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "none".into()),
                ),
            });
        }
        Ok(out)
    }

    /// Sessions belonging to another installed harness whose directory is a workspace
    /// this daemon tracks. Identifiers and paths only, never content (D20), and each pair
    /// once for the run.
    fn foreign_sighting_key(sighting: &ForeignSighting) -> (String, String, String) {
        (
            sighting.family.clone(),
            sighting.harness.clone(),
            sighting.session_id.clone(),
        )
    }

    fn retain_pending_foreign(&mut self, sighting: &ForeignSighting) -> Result<(), DaemonError> {
        let key = Self::foreign_sighting_key(sighting);
        let pending = sighting.to_pending();
        let entry = self
            .registry
            .retain_foreign_pending(&self.entry.entry_id, pending)?;
        self.entry = entry;
        self.foreign_pending.insert(key, sighting.clone());
        Ok(())
    }

    fn clear_pending_foreign(&mut self, sighting: &ForeignSighting) -> Result<(), DaemonError> {
        let key = Self::foreign_sighting_key(sighting);
        let entry =
            self.registry
                .clear_foreign_pending(&self.entry.entry_id, &key.0, &key.1, &key.2)?;
        self.entry = entry;
        self.foreign_pending.remove(&key);
        self.foreign_buffer_repair_needed.remove(&key);
        Ok(())
    }

    /// Recover the exact envelope for a foreign event that reached the chain but did not yet
    /// reach the write-ahead buffer. The chain is authoritative for the event bytes; the
    /// pending projection supplies the fields needed to identify that one event without
    /// retaining transcript content in the registry.
    fn find_foreign_chain_envelope(
        &mut self,
        sighting: &ForeignSighting,
    ) -> Result<Option<Envelope>, DaemonError> {
        use std::io::BufRead;

        #[cfg(any(test, feature = "test-hooks"))]
        {
            self.foreign_chain_scan_count_for_test =
                self.foreign_chain_scan_count_for_test.saturating_add(1);
        }

        let diagnostic = self.entry.trace_dir.join(CHAIN_FILE);
        let reader = self.writer_mut()?.independent_chain_reader()?;
        let io = |source: std::io::Error| DaemonError::Io {
            path: diagnostic.display().to_string(),
            source,
        };
        for line in std::io::BufReader::new(reader).lines() {
            let line = line.map_err(io)?;
            if line.trim().is_empty() {
                continue;
            }
            let envelope: Envelope = serde_json::from_str(&line)?;
            if let EventBody::ForeignSession {
                harness,
                session_id,
                store_path,
                first_seen,
            } = &envelope.body
                && harness == &sighting.harness
                && session_id == &sighting.session_id
                && store_path == &sighting.store_path
                && first_seen == &sighting.first_seen
            {
                return Ok(Some(envelope));
            }
        }
        Ok(None)
    }

    /// On a rebuilt runtime, repair every foreign-session envelope that reached the chain but
    /// not the buffer. This also covers immediate sightings whose source disappeared before a
    /// pending projection could be retained; the wire event itself is enough to restore the
    /// exact buffered bytes.
    fn repair_foreign_chain_buffer(&mut self) -> Result<(), DaemonError> {
        use std::io::BufRead;

        #[cfg(any(test, feature = "test-hooks"))]
        {
            self.foreign_chain_scan_count_for_test =
                self.foreign_chain_scan_count_for_test.saturating_add(1);
        }

        let diagnostic = self.entry.trace_dir.join(CHAIN_FILE);
        let reader = self.chain_reader_mut()?.independent_chain_reader()?;
        let io = |source: std::io::Error| DaemonError::Io {
            path: diagnostic.display().to_string(),
            source,
        };
        for line in std::io::BufReader::new(reader).lines() {
            let line = line.map_err(io)?;
            if line.trim().is_empty() {
                continue;
            }
            let envelope: Envelope = serde_json::from_str(&line)?;
            let EventBody::ForeignSession {
                harness,
                session_id,
                store_path,
                first_seen,
            } = &envelope.body
            else {
                continue;
            };
            let pending = self
                .foreign_pending
                .values()
                .find(|candidate| {
                    candidate.harness == *harness
                        && candidate.session_id == *session_id
                        && candidate.store_path == *store_path
                        && candidate.first_seen == *first_seen
                })
                .cloned();
            let sighting = pending.clone().unwrap_or_else(|| ForeignSighting {
                harness: harness.clone(),
                family: UNKNOWN_FOREIGN_FAMILY.to_string(),
                session_id: session_id.clone(),
                store_path: store_path.clone(),
                directory: String::new(),
                first_seen: first_seen.clone(),
                scan_root: ForeignScanRoot::Home,
            });
            let key = Self::foreign_sighting_key(&sighting);
            if pending.is_none() && self.buffer_has_foreign_envelope(&envelope)? {
                continue;
            }
            if !self.foreign_buffer_repair_needed.contains_key(&key)
                && self.foreign_buffer_repair_needed.len() >= MAX_FOREIGN_BUFFER_REPAIR_MARKERS
            {
                return Err(DaemonError::Refused(format!(
                    "foreign buffer-repair marker bound reached at {} identities",
                    MAX_FOREIGN_BUFFER_REPAIR_MARKERS
                )));
            }
            let repair = ForeignBufferRepair {
                sighting,
                envelope,
                keep_until_pending: pending.is_some(),
            };
            self.foreign_buffer_repair_needed
                .insert(key.clone(), repair.clone());
            // A rebuilt runtime must stay usable when the buffer write is transiently
            // unavailable. The exact envelope is retained above, and the next ordinary tick
            // retries it directly without another chain scan.
            if let Err(_error) = self.repair_foreign_buffer_marker(&key, &repair) {
                // Keep the marker. Other chain rows still need their one-pass reconstruction.
            }
        }
        Ok(())
    }

    /// Test whether the exact chain envelope is already buffered. A row at or below the
    /// acknowledgement floor is already durable at the receiver and needs no local repair;
    /// otherwise compare the decrypted envelope, because a reused seq must be corrected too.
    fn buffer_has_foreign_envelope(&self, envelope: &Envelope) -> Result<bool, DaemonError> {
        let floor = self.buffer.floor()?;
        if floor.is_some_and(|floor| floor >= envelope.link.seq) {
            return Ok(true);
        }
        let after = envelope.link.seq.checked_sub(1);
        let read = self.buffer.read_from(after, 1)?;
        Ok(read.envelopes.first() == Some(envelope))
    }

    /// Repair immediate foreign events whose chain append committed before the buffer append
    /// failed. This runs on every ordinary tick, so a source that disappears cannot strand a
    /// live runtime until restart; the bounded marker retains the exact committed envelope
    /// and its identity projection, so retries do not rescan the chain.
    fn repair_needed_foreign_buffers(&mut self) -> Result<(), DaemonError> {
        let repairs: Vec<_> = self
            .foreign_buffer_repair_needed
            .values()
            .cloned()
            .collect();
        for repair in repairs {
            let key = Self::foreign_sighting_key(&repair.sighting);
            self.repair_foreign_buffer_marker(&key, &repair)?;
        }
        Ok(())
    }

    fn repair_foreign_buffer_marker(
        &mut self,
        key: &(String, String, String),
        repair: &ForeignBufferRepair,
    ) -> Result<(), DaemonError> {
        #[cfg(any(test, feature = "test-hooks"))]
        if self.fail_foreign_buffer_repair_for_test > 0 {
            self.fail_foreign_buffer_repair_for_test -= 1;
            return Err(DaemonError::Refused(
                "test-hooks: foreign buffer repair append failure".to_string(),
            ));
        }
        if !self.buffer_has_foreign_envelope(&repair.envelope)? {
            self.buffer.append(&repair.envelope)?;
        }
        self.foreign_seen.remove(&(
            UNKNOWN_FOREIGN_FAMILY.to_string(),
            repair.sighting.harness.clone(),
            repair.sighting.session_id.clone(),
        ));
        self.foreign_seen.insert(key.clone());
        if !repair.keep_until_pending || !self.foreign_pending.contains_key(key) {
            self.foreign_buffer_repair_needed.remove(key);
        }
        Ok(())
    }

    /// Complete a chain-before-buffer recovery without appending a second chain event.
    fn repair_foreign_buffer_from_chain(
        &mut self,
        sighting: &ForeignSighting,
    ) -> Result<Option<()>, DaemonError> {
        let key = Self::foreign_sighting_key(sighting);
        if let Some(repair) = self.foreign_buffer_repair_needed.get(&key).cloned() {
            self.repair_foreign_buffer_marker(&key, &repair)?;
            return Ok(Some(()));
        }
        let Some(envelope) = self.find_foreign_chain_envelope(sighting)? else {
            return Ok(None);
        };
        let repair = ForeignBufferRepair {
            sighting: sighting.clone(),
            envelope,
            keep_until_pending: self.foreign_pending.contains_key(&key),
        };
        if self.buffer_has_foreign_envelope(&repair.envelope)? {
            self.foreign_seen.remove(&(
                UNKNOWN_FOREIGN_FAMILY.to_string(),
                sighting.harness.clone(),
                sighting.session_id.clone(),
            ));
            self.foreign_seen.insert(key);
            return Ok(Some(()));
        }
        if self.foreign_buffer_repair_needed.len() >= MAX_FOREIGN_BUFFER_REPAIR_MARKERS {
            return Err(DaemonError::Refused(format!(
                "foreign buffer-repair marker bound reached at {} identities",
                MAX_FOREIGN_BUFFER_REPAIR_MARKERS
            )));
        }
        self.foreign_buffer_repair_needed
            .insert(key.clone(), repair.clone());
        self.repair_foreign_buffer_marker(&key, &repair)?;
        Ok(Some(()))
    }

    fn chain_foreign_sighting(&mut self, sighting: &ForeignSighting) -> Result<u64, DaemonError> {
        let key = Self::foreign_sighting_key(sighting);
        if self.foreign_seen.contains(&key) {
            return Ok(0);
        }
        let unknown = (
            UNKNOWN_FOREIGN_FAMILY.to_string(),
            sighting.harness.clone(),
            sighting.session_id.clone(),
        );
        if self.foreign_seen.contains(&unknown) {
            if self.repair_foreign_buffer_from_chain(sighting)?.is_some() {
                return Ok(0);
            }
            return Err(DaemonError::Refused(format!(
                "foreign chain marker for {}:{} has no matching envelope to repair",
                sighting.harness, sighting.session_id
            )));
        }
        if !self.foreign_buffer_repair_needed.contains_key(&key)
            && self.foreign_buffer_repair_needed.len() >= MAX_FOREIGN_BUFFER_REPAIR_MARKERS
        {
            return Err(DaemonError::Refused(format!(
                "foreign buffer-repair marker bound reached at {} identities",
                MAX_FOREIGN_BUFFER_REPAIR_MARKERS
            )));
        }
        let env = {
            let writer = self.writer_mut()?;
            writer.event(sighting.to_event())?
        };
        let repair = ForeignBufferRepair {
            sighting: sighting.clone(),
            envelope: env.clone(),
            keep_until_pending: self.foreign_pending.contains_key(&key),
        };
        #[cfg(any(test, feature = "test-hooks"))]
        if self.fail_foreign_buffer_after_chain_for_test {
            self.fail_foreign_buffer_after_chain_for_test = false;
            self.foreign_seen.insert(unknown);
            self.foreign_buffer_repair_needed.insert(key, repair);
            return Err(DaemonError::Refused(
                "test-hooks: chain-to-buffer foreign append gap".to_string(),
            ));
        }
        if let Err(error) = self.buffer.append(&env) {
            // The chain writer commits before Buffer::append. Retain an unknown-family
            // marker so a retry or rebuild searches for and repairs this exact envelope
            // rather than appending a second chain event.
            self.foreign_seen.insert(unknown);
            self.foreign_buffer_repair_needed.insert(key, repair);
            return Err(error);
        }
        self.foreign_seen.insert(key);
        Ok(1)
    }

    fn flush_pending_foreign(
        &mut self,
        family: &str,
        harness: &str,
        bound: Option<&str>,
        phase: ForeignSweepPhase,
    ) -> Result<u64, DaemonError> {
        if phase == ForeignSweepPhase::LiveHeartbeat && bound.is_none() {
            return Ok(0);
        }
        let keys: Vec<_> = self.foreign_pending.keys().cloned().collect();
        let mut chained = 0;
        for key in keys {
            let Some(sighting) = self.foreign_pending.get(&key).cloned() else {
                continue;
            };
            let is_exact_bound = sighting.family == family
                && sighting.harness == harness
                && bound == Some(sighting.session_id.as_str());
            if is_exact_bound {
                let unknown = (
                    UNKNOWN_FOREIGN_FAMILY.to_string(),
                    sighting.harness.clone(),
                    sighting.session_id.clone(),
                );
                if self.foreign_seen.contains(&unknown) {
                    self.repair_foreign_buffer_from_chain(&sighting)?
                        .ok_or_else(|| {
                            DaemonError::Refused(format!(
                                "foreign chain marker for {}:{} has no matching envelope to repair",
                                sighting.harness, sighting.session_id
                            ))
                        })?;
                }
                self.clear_pending_foreign(&sighting)?;
                continue;
            }
            chained += self.chain_foreign_sighting(&sighting)?;
            self.clear_pending_foreign(&sighting)?;
        }
        Ok(chained)
    }

    fn sweep_foreign(&mut self, now: f64, phase: ForeignSweepPhase) -> Result<u64, DaemonError> {
        let scan = crate::foreign::scan_once(
            &self.foreign_plan,
            &self.vars,
            std::slice::from_ref(&self.workspace),
        )?;
        // This run's own session is not a foreign one. A connector whose `foreign_scan[]`
        // glob covers the directory its tracked store lives in — Claude's second entry is
        // `{config_dir}/projects/*/*.jsonl`, and the tracked store is one file inside it —
        // returns the bound session on every sweep, and chaining that would report the run
        // as a second session in its own workspace on every heartbeat and again at the
        // seal. The scan cannot know which session is bound; this type does.
        let bound = self.bound_session();
        let own_family = self.manifest.family.clone();
        let own_harness = self.manifest.id.clone();
        let same_harness = |sighting: &crate::foreign::ForeignSighting| {
            sighting.family == own_family && sighting.harness == own_harness
        };
        let mut chained =
            self.flush_pending_foreign(&own_family, &own_harness, bound.as_deref(), phase)?;
        let at = unix_to_rfc3339(now);
        let eligible_identity = scan.eligible_identity;
        for coverage in scan.coverage {
            let key = coverage.declaration_key();
            if eligible_identity.contains(&key) {
                if phase == ForeignSweepPhase::LiveHeartbeat
                    && self.foreign_identity_live_grace
                    && !self.foreign_identity_pending.contains(&key)
                    && self.foreign_identity_pending.len()
                        < MAX_FOREIGN_IDENTITY_PENDING_DECLARATIONS
                {
                    self.foreign_identity_pending.insert(key.clone());
                    // A complete, valid prefix can be between writer records. The second
                    // consecutive observation falls through and emits the existing closed
                    // identity-unavailable outcome; source-byte changes do not restart it.
                    continue;
                }
                if phase == ForeignSweepPhase::Seal {
                    self.foreign_identity_pending.remove(&key);
                }
            } else {
                // Complete, absent, and every hard unavailable result break consecutive
                // eligibility. The next live clean EOF therefore has one bounded grace
                // observation of its own.
                self.foreign_identity_pending.remove(&key);
            }
            if self.foreign_coverage_seen.get(&key) == Some(&coverage.outcome) {
                continue;
            }
            let env = self.writer_mut()?.event(coverage.to_event(&at))?;
            self.buffer.append(&env)?;
            self.foreign_coverage_seen.insert(key, coverage.outcome);
            chained += 1;
        }
        for sighting in scan.sightings {
            let same_harness_own_config =
                same_harness(&sighting) && sighting.scan_root == ForeignScanRoot::OwnConfigDir;
            if same_harness_own_config {
                if bound.is_none() && phase == ForeignSweepPhase::LiveHeartbeat {
                    let key = Self::foreign_sighting_key(&sighting);
                    if self.foreign_seen.contains(&key) || self.foreign_pending.contains_key(&key) {
                        continue;
                    }
                    if self.foreign_pending.len() < MAX_FOREIGN_PENDING_SIGHTINGS {
                        self.retain_pending_foreign(&sighting)?;
                        continue;
                    }
                    // The retention bound is fail-closed: once it is full, publish the
                    // unresolved identity rather than silently dropping evidence.
                }
                if bound.as_deref() == Some(sighting.session_id.as_str()) {
                    continue;
                }
            }
            chained += self.chain_foreign_sighting(&sighting)?;
        }
        Ok(chained)
    }

    /// Persist where every tailer got to, so a restart continues rather than re-reading
    /// the run from zero (D19). Written only when a position moved: a JSON write on every
    /// 250 ms tick would be a cost the run pays for nothing.
    ///
    /// **"Has anything moved?" is answered in memory.** This runs after every
    /// batch that carried records — it has to, because a cursor written once a tick leaves
    /// a hard kill mid-tick behind the chain. Reading `cursors.json` back off the disk and
    /// comparing the whole document each time would get steadily more expensive on the hot
    /// path: that document grows, `DirTailer`'s seen-set holds one entry per file for the
    /// life of the run, and Claude Code's `raw-bodies` store is a directory of them. One
    /// digest of what this runtime last wrote answers the same question at a fixed cost,
    /// and holding a digest rather than the bytes keeps the memory bound this crate promises.
    ///
    /// The trade, stated: a `cursors.json` deleted from under a live daemon is not written
    /// again until a cursor actually moves. Until then a restart re-reads that store from
    /// zero with a `Reread` witness above it, which is D19's own fallback and loses no
    /// evidence.
    ///
    /// **And its size is bounded at the other end.** The document is
    /// still serialised in full whenever anything moved, because it is one document; what
    /// keeps it from growing with every file a store has ever held is
    /// [`crate::tailer::dir::MAX_SEEN_FILES`] and the eviction beside it, which is the
    /// tailer's own state to bound.
    fn save_cursors(&mut self) -> Result<(), DaemonError> {
        // The position and where the chain stood when it was taken, together, because a
        // restart cannot tell a re-read from a fresh record with only one of them.
        // Written in one document, so the two can never be saved a batch apart.
        let cursors: BTreeMap<&str, crate::cursor::SavedCursor> = self
            .tailers
            .iter()
            .map(|bound| {
                let name = bound.binding.name();
                (
                    name,
                    crate::cursor::SavedCursor {
                        cursor: self
                            .blocked_file_first_sources
                            .get(name)
                            .cloned()
                            .unwrap_or_else(|| bound.reader.cursor()),
                        last_leg1_seq: self.last_leg1_seq.get(name).copied(),
                    },
                )
            })
            .collect();
        let bytes = serde_json::to_vec(&cursors)?;
        let digest = crate::workspace::sha256_hex(&bytes);
        if self.saved_cursor_digest.as_deref() == Some(digest.as_str()) {
            return Ok(());
        }
        // The retained entry capability creates and renames the temporary under the same
        // held directory descriptor. A swapped `artifacts/<id>` pathname can therefore
        // neither receive a new cursor document nor redirect an old one.
        match self.artifact.replace_cursors(&bytes) {
            Ok(()) => {}
            // Preserve the cadence contract for a genuine local write failure while still
            // deriving the target from the retained authority, not from a mutable registry path.
            Err(TraceCapError::Io { source, .. }) => {
                return Err(DaemonError::Io {
                    path: self
                        .artifact
                        .display_path()
                        .join("cursors.json")
                        .display()
                        .to_string(),
                    source,
                });
            }
            Err(error) => {
                return Err(DaemonError::Refused(format!(
                    "cursor document cannot be replaced through retained entry artifact authority: {error}"
                )));
            }
        }
        // Recorded only after the rename: a write that failed left the previous document
        // on disk, and remembering this one would make the next save agree with a file that
        // was never written.
        self.saved_cursor_digest = Some(digest);
        Ok(())
    }

    /// Everything one evaluation is allowed to look at, gathered at one clock.
    fn health_inputs(
        &self,
        now: f64,
        store_replaced: bool,
        config_restored: bool,
    ) -> Result<HealthInputs, DaemonError> {
        let alive = self.same_process();
        Ok(HealthInputs {
            now,
            arming: self.gate.state(),
            harness_alive: alive,
            last_leg1_at: self.last_leg1_at,
            last_ack_at: self.entry.last_ack_at,
            pending_events: self.buffer.pending(None)?,
            connected: self.connected,
            config_mismatch: self.config_mismatch,
            arming_lost: self.arming_lost,
            version_changed: self.version_changed,
            deadline_at: self.entry.deadline_unix,
            agent_exited: self.entry.last_process.is_some() && !alive,
            store_replaced,
            disk_pressure: self.buffer.under_pressure(self.policy.disk_reserve_bytes),
            tracker_restarted: self.tracker_restarted,
            config_restored,
            clock_skew_seconds: self.clock_skew_seconds,
            backing_off: self.backing_off,
            ack_unverified: self.ack_unverified,
        })
    }

    /// The entry's mirror of the chain head, refreshed from the writer and never passed
    /// into it. Written to the registry only when it moved: a redb write transaction per
    /// 250 ms tick would be a cost the run pays for nothing.
    ///
    /// **`start_signal_at` rides along**, once, the first time the gate has one and the row
    /// does not. [`ArmingGate::resume`] is told the signal by the row, so a restart before
    /// the receipt was signed comes back with the window settled rather than reopened from
    /// `Armed`. It is not stamped anywhere else and
    /// it is never re-stamped: the start is the moment the run became observable, and a
    /// second write would move a number a resumed gate reads as the one it observed.
    fn sync_head(&mut self) -> Result<(), DaemonError> {
        let head = match &self.writer {
            WriterState::ResumePending(writer) | WriterState::Open(writer) => writer.head(),
            WriterState::FreshPending | WriterState::Failed => None,
        };
        let (seq, hash) = match head {
            Some((seq, hash)) => (Some(seq), hash),
            None => (None, goalcup_protocol::chain::GENESIS.to_string()),
        };
        let signal = match (self.entry.start_signal_at, self.gate.start_signal_at()) {
            (None, Some(at)) => Some(at),
            _ => None,
        };
        if self.entry.chain_seq == seq
            && self.entry.chain_head == hash
            && self.entry.health == self.health
            && signal.is_none()
        {
            return Ok(());
        }
        let health = self.health;
        self.entry = self.registry.update(&self.entry.entry_id, |e| {
            e.chain_seq = seq;
            e.chain_head = hash.clone();
            e.health = health;
            if let Some(at) = signal {
                e.start_signal_at = Some(at);
            }
        })?;
        Ok(())
    }

    fn writer_mut(&mut self) -> Result<&mut ChainWriter, DaemonError> {
        match &mut self.writer {
            WriterState::Open(writer) => Ok(writer),
            WriterState::FreshPending | WriterState::ResumePending(_) | WriterState::Failed => {
                Err(no_writer())
            }
        }
    }

    fn chain_reader_mut(&mut self) -> Result<&mut ChainWriter, DaemonError> {
        match &mut self.writer {
            WriterState::ResumePending(writer) | WriterState::Open(writer) => Ok(writer),
            WriterState::FreshPending | WriterState::Failed => Err(no_writer()),
        }
    }

    fn receipt_writer_mut(&mut self) -> Result<&mut ChainWriter, DaemonError> {
        match &mut self.writer {
            WriterState::ResumePending(writer) | WriterState::Open(writer) => Ok(writer),
            WriterState::FreshPending | WriterState::Failed => Err(no_writer()),
        }
    }

    fn read_chain_line(&mut self, seq: u64) -> Result<Envelope, DaemonError> {
        let diagnostic = self.entry.trace_dir.join(CHAIN_FILE);
        let reader = self.writer_mut()?.independent_chain_reader()?;
        chain_line_at(reader, &diagnostic, seq)
    }

    /// Test-only: this entry's chain writer, so a test can arm the injected failures
    /// `src/chain_writer.rs` keeps behind the same off-by-default `test-hooks` feature.
    /// There is no shipped caller and no shipped build that compiles it.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn chain_writer_mut(&mut self) -> Result<&mut ChainWriter, DaemonError> {
        match &mut self.writer {
            WriterState::ResumePending(writer) | WriterState::Open(writer) => Ok(writer),
            WriterState::FreshPending | WriterState::Failed => Err(no_writer()),
        }
    }

    /// Test-only: install the deterministic post-terminal pause on this exact runtime.
    /// The returned handle is the only object allowed to wait for or release it, so another
    /// daemon's same display id cannot intercept this runtime's retained artifact authority.
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn arm_live_seal_write_pause_for_test(
        &mut self,
    ) -> Result<LiveSealWritePause, DaemonError> {
        if self.live_seal_write_pause.is_some() {
            return Err(DaemonError::Refused(format!(
                "live seal write pause is already armed for {}",
                self.entry.entry_id
            )));
        }
        let (pause, state) = LiveSealWritePause::new();
        self.live_seal_write_pause = Some(state);
        Ok(pause)
    }

    /// Test-only: pause after the durable Terminated checkpoint and before submission
    /// filesystem capture/readback begins.
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn arm_workspace_capture_pause_for_test(
        &mut self,
    ) -> Result<LiveSealWritePause, DaemonError> {
        if self.workspace_capture_pause.is_some() {
            return Err(DaemonError::Refused(format!(
                "workspace capture pause is already armed for {}",
                self.entry.entry_id
            )));
        }
        let (pause, state) = LiveSealWritePause::new();
        self.workspace_capture_pause = Some(state);
        Ok(pause)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn arm_workspace_intent_cas_pause_for_test(
        &mut self,
    ) -> Result<LiveSealWritePause, DaemonError> {
        if self.workspace_intent_cas_pause.is_some() {
            return Err(DaemonError::Refused(format!(
                "workspace intent CAS pause is already armed for {}",
                self.entry.entry_id
            )));
        }
        let (pause, state) = LiveSealWritePause::new();
        self.workspace_intent_cas_pause = Some(state);
        Ok(pause)
    }

    /// Test-only: stage a durable buffer tail after an in-process terminal transition.
    /// Real capture cannot append after `Sealing`, but this is the exact corruption the
    /// upload ceiling must refuse before it can reach the server.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn append_buffer_for_test(&self, envelope: &Envelope) -> Result<(), DaemonError> {
        self.buffer.append(envelope)
    }

    /// Test-only: commit a buffer acknowledgement without its registry mirror.  The normal
    /// uploader has exactly this crash window — `Buffer::ack` commits before the registry
    /// transaction — but a test cannot open a second redb owner while this live runtime
    /// correctly holds the buffer lock.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn acknowledge_buffer_for_test(&self, up_to_seq: u64) -> Result<(), DaemonError> {
        self.buffer.ack(up_to_seq).map(|_| ())
    }

    /// Test-only: make the post-uploader terminal revalidation observe a new buffered tail.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn stage_post_pump_tail_for_test(&mut self, envelope: Envelope) {
        self.post_pump_tail_for_test = Some(envelope);
    }

    /// Test-only: reserve a real sealing intent but deliberately do not call the terminal
    /// writer.  This gives the upload path a deterministic crash-before-`write_seal`
    /// fixture without making any shipped caller manufacture a terminal envelope.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn begin_sealing_without_terminal_for_test(&mut self) -> Result<Envelope, DaemonError> {
        let sealed_at = now_rfc3339();
        let manifest = crate::workspace::list_workspace(
            &self.workspace,
            &self.entry.entry_id,
            &sealed_at,
            &self.excluded,
        )?;
        let mut descriptor = self.descriptor_doc();
        descriptor.seal = Some(goalcup_protocol::export::SEAL_FILE.to_string());
        let (pre_seal_head, trace_root_identity) = {
            let writer = self.writer_mut()?;
            (writer.head(), writer.root_identity()?)
        };
        let intent = SealingIntent::new(manifest, descriptor, pre_seal_head, trace_root_identity)?;
        let terminal = intent.expected_terminal_envelope()?;
        self.begin_sealing(intent)?;
        Ok(terminal)
    }
}

fn no_writer() -> DaemonError {
    DaemonError::Missing(
        "the entry's chain is not open: call `open_trace` before capturing, so the baseline \
         or the restart is the first link rather than whatever arrived first"
            .to_string(),
    )
}

/// The poll order: every store another store's `AggregateLearned` spec names as its
/// `teach_store` first, in manifest order, then the rest.
///
/// This puts the ordering in data rather than in a caller's discipline. On OpenCode it reads:
/// poll `event`, admit and teach each of its rows, and only then admit `message` and
/// `part`. A `message` row admitted ahead of the `event` row that teaches its aggregate is
/// discarded as `UnknownAggregate` and never offered again — `admit` judges once, and D20
/// forbids holding the bytes for a second look.
fn poll_order(stores: &[Store]) -> Vec<&Store> {
    let teachers: BTreeSet<&str> = stores
        .iter()
        .filter_map(|s| match s.partition.as_ref() {
            Some(PartitionSpec::AggregateLearned { teach_store, .. }) => teach_store.as_deref(),
            _ => None,
        })
        .collect();
    let mut order: Vec<&Store> = stores
        .iter()
        .filter(|s| teachers.contains(s.name.as_str()))
        .collect();
    order.extend(
        stores
            .iter()
            .filter(|s| !teachers.contains(s.name.as_str())),
    );
    order
}

/// Build the active authority and independent partition state for a concrete reader set.
///
/// The map is deliberately keyed by the record name the reader emits. A parent store is
/// carried inside each [`SourceBinding`], so no later caller has to recover it from a suffix
/// or a path basename.
fn binding_state(
    tailers: &[BoundTailer],
    manifest: &ConnectorManifest,
    workspace: &std::path::Path,
    session_id: Option<&str>,
) -> Result<BindingState, DaemonError> {
    let mut bindings = BTreeMap::new();
    let mut partitioners = BTreeMap::new();
    for bound in tailers {
        let binding = bound.binding.clone();
        let store = manifest
            .store_named(binding.parent_store())
            .ok_or_else(|| {
                DaemonError::Refused(format!(
                    "active binding `{}` names unknown parent store `{}`",
                    binding.name(),
                    binding.parent_store()
                ))
            })?;
        let name = binding.name().to_string();
        if bindings.insert(name.clone(), binding.clone()).is_some() {
            return Err(DaemonError::Refused(format!(
                "active binding `{name}` was constructed more than once"
            )));
        }
        let partitioner = Partitioner::for_binding(
            store.partition.as_ref(),
            partition::tracked_for(store.partition.as_ref(), workspace, session_id),
            binding.kind() == SourceKind::Spill,
            &name,
        )?;
        if partitioners.insert(name.clone(), partitioner).is_some() {
            return Err(DaemonError::Refused(format!(
                "active partitioner `{name}` was constructed more than once"
            )));
        }
    }
    Ok(BindingState {
        bindings,
        partitioners,
    })
}

fn file_session_id_field(manifest: &ConnectorManifest) -> Option<String> {
    manifest
        .foreign_scan
        .iter()
        .find(|scan| scan.harness == manifest.id)
        .map(|scan| scan.session_id_field.clone())
}

struct ProcessedOutOfWindowBatch {
    canonical_records: Vec<Vec<u8>>,
    validation: OutOfWindowBatchValidationV1,
    complete: bool,
    first_error: Option<DaemonError>,
}

fn out_of_window_source_coordinate(
    manifest: &ConnectorManifest,
    binding: &SourceBinding,
) -> Result<Vec<u8>, DaemonError> {
    Ok(serde_json::to_vec(&(
        "goalcup/tailer-binding/v1",
        manifest.id.as_str(),
        binding.name(),
    ))?)
}

fn initialize_out_of_window_progress(
    buffer: &Buffer,
    manifest: &ConnectorManifest,
    tailers: &[BoundTailer],
) -> Result<(), DaemonError> {
    let stores = tailers
        .iter()
        .map(|bound| {
            let coordinate = out_of_window_source_coordinate(manifest, &bound.binding)?;
            Ok((
                buffer.derive_out_of_window_store_key(&coordinate)?,
                serde_json::to_vec(&bound.reader.cursor())?,
            ))
        })
        .collect::<Result<Vec<_>, DaemonError>>()?;
    buffer.initialize_out_of_window_batch_progress(OutOfWindowAdmissionV1 {
        stores,
        max_transitions_per_store: MAX_OOW_TRANSITIONS_PER_STORE,
    })
}

fn incomplete_out_of_window_validation() -> OutOfWindowBatchValidationV1 {
    OutOfWindowBatchValidationV1 {
        flags: 1,
        reason_digest: Sha256::digest(b"goalcup/tailer-oow-incomplete/v1").into(),
    }
}

#[allow(clippy::too_many_arguments)]
fn process_out_of_window_batch(
    entry_id: &str,
    batch: &TailBatch,
    bindings: &BTreeMap<String, SourceBinding>,
    partitioners: &mut BTreeMap<String, Partitioner>,
    manifest: &ConnectorManifest,
    sessions_by_path: &mut BTreeMap<String, String>,
    bound_session: Option<&str>,
    recovery: bool,
) -> ProcessedOutOfWindowBatch {
    let mut complete = !batch.more_available && !batch.files_capped && batch.notices.is_empty();
    let mut first_error = None;
    let mut canonical_records = Vec::new();
    for notice in &batch.notices {
        if notice.is_rewrite() {
            if let Some(partitioner) = partitioners.get_mut(notice.store()) {
                partitioner.forget_file(notice.path());
            }
            sessions_by_path.remove(notice.path());
        }
    }
    for record in &batch.records {
        let Some(binding) = bindings.get(&record.store).cloned() else {
            complete = false;
            first_error.get_or_insert_with(|| {
                DaemonError::Refused(format!(
                    "entry {entry_id} received an OOW record from inactive binding {}",
                    record.store
                ))
            });
            continue;
        };
        match admit_out_of_window_record(
            record,
            &binding,
            partitioners,
            manifest,
            sessions_by_path,
            bound_session,
            recovery,
        ) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(error) => {
                complete = false;
                first_error.get_or_insert(error);
                continue;
            }
        }
        match crate::chain_writer::canonical_out_of_window_leg1(record, manifest) {
            Ok(bytes) => canonical_records.push(bytes),
            Err(error) => {
                complete = false;
                first_error.get_or_insert(error);
            }
        }
    }
    ProcessedOutOfWindowBatch {
        canonical_records,
        validation: if complete {
            OutOfWindowBatchValidationV1::CLEAN
        } else {
            incomplete_out_of_window_validation()
        },
        complete,
        first_error,
    }
}

fn apply_out_of_window_batch(
    buffer: &Buffer,
    manifest: &ConnectorManifest,
    bound: &BoundTailer,
    before: &CursorState,
    processed: &ProcessedOutOfWindowBatch,
    limits: OutOfWindowLimitsV1,
) -> Result<bool, DaemonError> {
    let coordinate = out_of_window_source_coordinate(manifest, &bound.binding)?;
    let outcome = buffer.apply_out_of_window_batch_with_validation(
        buffer.derive_out_of_window_store_key(&coordinate)?,
        &serde_json::to_vec(before)?,
        &serde_json::to_vec(&bound.reader.cursor())?,
        &processed.canonical_records,
        processed.validation,
        limits,
    )?;
    let (stats, validation) = match outcome {
        OutOfWindowBatchApplyV1::Applied {
            stats, validation, ..
        }
        | OutOfWindowBatchApplyV1::AlreadyApplied { stats, validation } => (stats, validation),
    };
    Ok(stats.overflowed == 0 && validation.flags == 0)
}

/// Apply the same partition authority to late evidence that ordinary ingestion applies to
/// qualifying records, with one additional privacy check for wildcard file-first stores:
/// the file header must bind the record to this exact root session, not merely its cwd.
fn admit_out_of_window_record(
    record: &CapturedRecord,
    binding: &SourceBinding,
    partitioners: &mut BTreeMap<String, Partitioner>,
    manifest: &ConnectorManifest,
    sessions_by_path: &mut BTreeMap<String, String>,
    bound_session: Option<&str>,
    recovery: bool,
) -> Result<bool, DaemonError> {
    let outcome = partitioners
        .get_mut(&record.store)
        .ok_or_else(|| {
            DaemonError::Refused(format!(
                "active OOW binding `{}` has no partitioner",
                record.store
            ))
        })?
        .admit(record);
    if binding.kind() == SourceKind::Primary {
        partition::teach(
            partitioners,
            binding.parent_store(),
            record,
            &manifest.store,
        );
    }
    match outcome {
        Outcome::Keep => {}
        Outcome::Discard(DiscardReason::UnknownAggregate) if recovery => {
            return Err(DaemonError::Refused(format!(
                "cannot reconstruct aggregate partition authority for OOW record {}",
                record.store
            )));
        }
        Outcome::Discard(_) => return Ok(false),
    }

    let store = manifest
        .store_named(binding.parent_store())
        .ok_or_else(|| {
            DaemonError::Refused(format!(
                "OOW binding `{}` names unknown parent store `{}`",
                binding.name(),
                binding.parent_store()
            ))
        })?;
    if binding.kind() == SourceKind::Primary
        && matches!(store.partition, Some(PartitionSpec::FileFirstRecord { .. }))
        && let Some(field) = file_session_id_field(manifest)
    {
        let explicit = str_at(&record.value, &field).map(|value| value.into_owned());
        if let Some(session) = &explicit
            && (sessions_by_path.len() < MAX_SESSIONS_BY_PATH
                || sessions_by_path.contains_key(&record.source_path))
        {
            sessions_by_path.insert(record.source_path.clone(), session.clone());
        }
        let observed = explicit.as_deref().or_else(|| {
            sessions_by_path
                .get(&record.source_path)
                .map(String::as_str)
        });
        match (bound_session, observed) {
            (Some(expected), Some(actual)) if expected == actual => {}
            (Some(_), Some(_)) => return Ok(false),
            _ => {
                return Err(DaemonError::Refused(format!(
                    "OOW record {} has no exact root-session authority",
                    record.store
                )));
            }
        }
    }
    Ok(true)
}

/// Parse the cursor document once for both source-prefix partition reconstruction and
/// ordinary tailer restore. Missing, malformed and old cursor documents retain the
/// existing empty-map fallback: every store re-reads from zero.
fn load_saved_cursors_from_artifact(
    artifact: &EntryArtifactRoot,
) -> Result<BTreeMap<String, crate::cursor::SavedCursor>, DaemonError> {
    let file = artifact.read_cursors().map_err(|error| {
        DaemonError::Refused(format!(
            "cursor document cannot be read through retained entry artifact authority: {error}"
        ))
    })?;
    Ok(file
        .and_then(|mut file| {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).ok().map(|_| bytes)
        })
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default())
}

fn load_finalization_cursors_strict(
    artifact: &EntryArtifactRoot,
) -> Result<BTreeMap<String, crate::cursor::SavedCursor>, DaemonError> {
    let mut file = artifact
        .read_cursors()
        .map_err(|error| {
            DaemonError::Refused(format!(
                "finalization cursor document cannot be read through retained authority: {error}"
            ))
        })?
        .ok_or_else(|| {
            DaemonError::Refused("finalization cursor document is missing".to_string())
        })?;
    let limit = MAX_FILE_BYTES.saturating_add(1);
    let mut bytes = Vec::new();
    file.by_ref()
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|source| DaemonError::Io {
            path: artifact
                .display_path()
                .join("cursors.json")
                .display()
                .to_string(),
            source,
        })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES {
        return Err(DaemonError::Refused(
            "finalization cursor document exceeds the fixed-document bound".to_string(),
        ));
    }
    serde_json::from_slice(&bytes).map_err(DaemonError::from)
}

fn trace_root_from_artifact(
    artifact: &EntryArtifactRoot,
    diagnostic: PathBuf,
) -> Result<TraceRoot, DaemonError> {
    let trace = artifact.open_trace().map_err(|error| {
        DaemonError::Refused(format!(
            "existing trace cannot be opened through retained entry artifact authority: {error}"
        ))
    })?;
    Ok(TraceRoot::from_acquired(trace, diagnostic))
}

/// Rebuild `FileFirstRecord` state from the source prefix the saved cursor already passed.
///
/// This is separate from replay: the real tailer's byte position and record count remain
/// unchanged, so reconstruction cannot spend a replay budget or suppress a truncation
/// notice. Only one line at a time is parsed, bounded by [`MAX_LINE_BYTES`], and no raw
/// bytes, value, digest or verdict is written anywhere. If a current source prefix cannot
/// be read completely, the whole store is marked unsafe to restore and will re-read from
/// zero through the ordinary, witnessed cursor-fallback path.
fn reconstruct_file_first_decisions(
    bindings: &BTreeMap<String, SourceBinding>,
    stores: &[Store],
    partitioners: &mut BTreeMap<String, Partitioner>,
    saved: &BTreeMap<String, crate::cursor::SavedCursor>,
    vars: &PathVars,
    session_id_field: Option<&str>,
) -> Reconstruction {
    let mut out = Reconstruction::default();
    let mut bytes_left = MAX_PARTITION_RECONSTRUCTION_BYTES;
    let mut files_left = MAX_PARTITION_RECONSTRUCTION_FILES;
    for binding in bindings.values() {
        let Some(store) = stores
            .iter()
            .find(|store| store.name == binding.parent_store())
        else {
            continue;
        };
        if !matches!(store.partition, Some(PartitionSpec::FileFirstRecord { .. })) {
            continue;
        }
        let Some(saved_store) = saved.get(binding.name()) else {
            continue;
        };
        if saved_store.cursor.kind() != store.cursor.kind {
            continue;
        }
        let files = match &saved_store.cursor {
            crate::cursor::CursorState::InodeOffset { files, .. }
            | crate::cursor::CursorState::Ordinal { files, .. } => files,
            _ => continue,
        };
        let Some(partitioner) = partitioners.get_mut(binding.name()) else {
            continue;
        };
        if files.len() > files_left {
            out.unsafe_restores.insert(binding.name().to_string());
            partitioner.clear_file_decisions();
            continue;
        }
        files_left -= files.len();
        let authorized_source =
            match AuthorizedSourcePattern::open(binding.resolved_pattern(), vars) {
                Ok(source) => source,
                Err(failure) => {
                    refuse_reconstruction(
                        &mut out,
                        partitioner,
                        binding.name(),
                        saved_store,
                        failure,
                    );
                    continue;
                }
            };
        for (path, cursor) in files {
            match reconstruct_file_prefix(
                partitioner,
                &authorized_source,
                path,
                *cursor,
                &mut bytes_left,
                session_id_field,
                &mut out.sessions_by_path,
            ) {
                Ok(PrefixState::Current) => {}
                // The real tailer will witness the replacement/truncation and read the
                // new generation from zero. It must meet an undecided partitioner first.
                Ok(PrefixState::Rewritten) => partitioner.forget_file(path),
                Err(failure) => {
                    refuse_reconstruction(
                        &mut out,
                        partitioner,
                        binding.name(),
                        saved_store,
                        failure,
                    );
                    break;
                }
            }
        }
    }
    out
}

#[derive(Default)]
struct Reconstruction {
    unsafe_restores: BTreeSet<String>,
    sessions_by_path: BTreeMap<String, String>,
    /// Only a source-identity safety refusal quarantines ordinary polling. Missing files,
    /// corrupt cursors and bounded-work exhaustion retain the existing witnessed reread.
    blocked_sources: BTreeMap<String, crate::cursor::CursorState>,
}

#[derive(Clone, Copy)]
enum ReconstructionFailure {
    /// The prefix was unavailable or the bounded reconstruction budget ended. Re-read the
    /// declared store from zero through its ordinary tailer.
    Fallback,
    /// The declared path resolved through a symbolic link or to a non-regular source. A
    /// string-based tailer must not be allowed to retry the same unsafe resolution.
    UnsafeSource,
}

fn refuse_reconstruction(
    out: &mut Reconstruction,
    partitioner: &mut Partitioner,
    binding_name: &str,
    saved: &crate::cursor::SavedCursor,
    failure: ReconstructionFailure,
) {
    out.unsafe_restores.insert(binding_name.to_string());
    if matches!(failure, ReconstructionFailure::UnsafeSource) {
        out.blocked_sources
            .insert(binding_name.to_string(), saved.cursor.clone());
    }
    partitioner.clear_file_decisions();
}

enum PrefixState {
    Current,
    Rewritten,
}

/// Read at most the exact prefix a cursor says was emitted. The fixed read buffer and the
/// one-line accumulator bound memory even for a very large rollout or an overlong line.
fn reconstruct_file_prefix(
    partitioner: &mut Partitioner,
    authorized_source: &AuthorizedSourcePattern,
    source_path: &str,
    cursor: crate::cursor::FileCursor,
    bytes_left: &mut u64,
    session_id_field: Option<&str>,
    sessions_by_path: &mut BTreeMap<String, String>,
) -> Result<PrefixState, ReconstructionFailure> {
    use std::io::Read;

    let mut file = authorized_source.open_file(std::path::Path::new(source_path))?;
    let metadata = file
        .metadata()
        .map_err(|_| ReconstructionFailure::Fallback)?;
    if !metadata.is_file() {
        return Err(ReconstructionFailure::UnsafeSource);
    }
    if cursor.offset == 0 {
        return Ok(PrefixState::Current);
    }
    if metadata.len() < cursor.offset || !cursor_inode_matches(&metadata, cursor.inode) {
        return Ok(PrefixState::Rewritten);
    }

    let mut remaining = cursor.offset;
    let mut chunk = [0_u8; 8 * 1024];
    let mut line = Vec::new();
    let mut overlong = false;
    while remaining > 0 {
        if *bytes_left == 0 {
            return Err(ReconstructionFailure::Fallback);
        }
        let wanted = usize::try_from(remaining.min(*bytes_left).min(chunk.len() as u64))
            .map_err(|_| ReconstructionFailure::Fallback)?;
        let read = file
            .read(&mut chunk[..wanted])
            .map_err(|_| ReconstructionFailure::Fallback)?;
        if read == 0 {
            return Err(ReconstructionFailure::Fallback);
        }
        remaining -= read as u64;
        *bytes_left -= read as u64;
        for byte in &chunk[..read] {
            if *byte == b'\n' {
                if !overlong {
                    let text = String::from_utf8_lossy(&line);
                    let value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
                    if let Some(field) = session_id_field
                        && let Some(session) = str_at(&value, field)
                        && (sessions_by_path.len() < MAX_SESSIONS_BY_PATH
                            || sessions_by_path.contains_key(source_path))
                    {
                        sessions_by_path.insert(source_path.to_string(), session.into_owned());
                    }
                    if partitioner.reconstruct_file_value(source_path, &value) {
                        return Ok(PrefixState::Current);
                    }
                }
                line.clear();
                overlong = false;
            } else if !overlong {
                if line.len() < MAX_LINE_BYTES as usize {
                    line.push(*byte);
                } else {
                    line.clear();
                    overlong = true;
                }
            }
        }
    }
    Ok(PrefixState::Current)
}

#[cfg(unix)]
struct AuthorizedSourcePattern {
    root_path: PathBuf,
    root: std::fs::File,
    relative_pattern: PathBuf,
}

#[cfg(unix)]
impl AuthorizedSourcePattern {
    fn open(pattern: &str, vars: &PathVars) -> Result<Self, ReconstructionFailure> {
        use std::os::unix::fs::OpenOptionsExt;

        let pattern = std::path::Path::new(pattern);
        let root_path =
            authorized_source_root(pattern, vars).ok_or(ReconstructionFailure::UnsafeSource)?;
        let relative_pattern = pattern
            .strip_prefix(&root_path)
            .map_err(|_| ReconstructionFailure::Fallback)?
            .to_path_buf();
        let root = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&root_path)
            .map_err(reconstruction_open_failure)?;
        if !root
            .metadata()
            .map_err(|_| ReconstructionFailure::Fallback)?
            .is_dir()
        {
            return Err(ReconstructionFailure::UnsafeSource);
        }
        Ok(Self {
            root_path,
            root,
            relative_pattern,
        })
    }

    fn open_file(
        &self,
        source_path: &std::path::Path,
    ) -> Result<std::fs::File, ReconstructionFailure> {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::ffi::OsStrExt;

        let relative = source_path
            .strip_prefix(&self.root_path)
            .map_err(|_| ReconstructionFailure::Fallback)?;
        if !path_matches_pattern(relative, &self.relative_pattern) {
            return Err(ReconstructionFailure::Fallback);
        }
        let components: Vec<&std::ffi::OsStr> = relative
            .components()
            .map(|component| match component {
                std::path::Component::Normal(name) => Ok(name),
                _ => Err(ReconstructionFailure::Fallback),
            })
            .collect::<Result<_, _>>()?;
        if components.is_empty() {
            return Err(ReconstructionFailure::Fallback);
        }

        let mut handle = self
            .root
            .try_clone()
            .map_err(|_| ReconstructionFailure::Fallback)?;
        for (index, component) in components.iter().enumerate() {
            let name = std::ffi::CString::new(component.as_bytes())
                .map_err(|_| ReconstructionFailure::Fallback)?;
            let last = index + 1 == components.len();
            let flags = libc::O_RDONLY
                | libc::O_NONBLOCK
                | libc::O_NOFOLLOW
                | libc::O_CLOEXEC
                | if last { 0 } else { libc::O_DIRECTORY };
            // SAFETY: `handle` is a live directory descriptor for the previously opened
            // component and `name` is NUL-terminated. `O_NOFOLLOW` makes each resolution
            // step fail when that exact child is a symlink.
            let fd = unsafe { libc::openat(handle.as_raw_fd(), name.as_ptr(), flags) };
            if fd < 0 {
                return Err(reconstruction_open_failure(std::io::Error::last_os_error()));
            }
            // SAFETY: a successful `openat` returns a new owned descriptor exactly once.
            handle = unsafe { std::fs::File::from_raw_fd(fd) };
        }
        Ok(handle)
    }
}

#[cfg(not(unix))]
struct AuthorizedSourcePattern;

#[cfg(not(unix))]
impl AuthorizedSourcePattern {
    fn open(_pattern: &str, _vars: &PathVars) -> Result<Self, ReconstructionFailure> {
        // The standard library cannot express a descriptor-relative, no-follow walk on
        // these platforms. Fail closed into the existing witnessed cursor reread instead
        // of reading a restart prefix through a path race.
        Err(ReconstructionFailure::UnsafeSource)
    }

    fn open_file(
        &self,
        _source_path: &std::path::Path,
    ) -> Result<std::fs::File, ReconstructionFailure> {
        Err(ReconstructionFailure::UnsafeSource)
    }
}

#[cfg(unix)]
fn reconstruction_open_failure(error: std::io::Error) -> ReconstructionFailure {
    match error.raw_os_error() {
        Some(libc::ELOOP) | Some(libc::ENOTDIR) => ReconstructionFailure::UnsafeSource,
        _ => ReconstructionFailure::Fallback,
    }
}

fn authorized_source_root(pattern: &std::path::Path, vars: &PathVars) -> Option<PathBuf> {
    [
        vars.workspace_canonical(),
        vars.workspace.clone(),
        vars.home.clone(),
        vars.config_dir.clone(),
        vars.artifact_root.clone(),
    ]
    .into_iter()
    .filter(|root| root.is_absolute() && pattern.strip_prefix(root).is_ok())
    .max_by_key(|root| root.components().count())
}

/// Match the current path template without walking it. Both inputs are already relative to
/// the pinned authorized root; wildcards retain `paths::expand_globs`' one-component
/// `*`/`?` semantics.
fn path_matches_pattern(path: &std::path::Path, pattern: &std::path::Path) -> bool {
    let mut path = path.components();
    let mut pattern = pattern.components();
    loop {
        match (path.next(), pattern.next()) {
            (None, None) => return true,
            (
                Some(std::path::Component::Normal(name)),
                Some(std::path::Component::Normal(want)),
            ) if component_matches(&name.to_string_lossy(), &want.to_string_lossy()) => {}
            _ => return false,
        }
    }
}

fn component_matches(name: &str, pattern: &str) -> bool {
    let name: Vec<char> = name.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    let (mut name_at, mut pattern_at) = (0_usize, 0_usize);
    let (mut star, mut matched) = (None, 0_usize);
    while name_at < name.len() {
        if pattern_at < pattern.len()
            && (pattern[pattern_at] == '?' || pattern[pattern_at] == name[name_at])
        {
            name_at += 1;
            pattern_at += 1;
        } else if pattern_at < pattern.len() && pattern[pattern_at] == '*' {
            star = Some(pattern_at);
            matched = name_at;
            pattern_at += 1;
        } else if let Some(star_at) = star {
            pattern_at = star_at + 1;
            matched += 1;
            name_at = matched;
        } else {
            return false;
        }
    }
    while pattern_at < pattern.len() && pattern[pattern_at] == '*' {
        pattern_at += 1;
    }
    pattern_at == pattern.len()
}

#[cfg(unix)]
fn cursor_inode_matches(metadata: &std::fs::Metadata, inode: u64) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.ino() == inode
}

#[cfg(not(unix))]
fn cursor_inode_matches(_metadata: &std::fs::Metadata, _inode: u64) -> bool {
    // The line tailer persists inode 0 on platforms without one and derives its decision
    // from the current source bytes too. Length remains the available rewrite signal.
    true
}

/// Put every tailer back where the last run left it, and answer with the stores that did
/// not come back.
///
/// **A cursor that will not restore is not an error.** The store may have changed its
/// declared type between builds, or the file may be from a daemon that wrote a different
/// shape; either way D19's stated fallback is to re-read that store from zero, which
/// duplicates records and loses none. Refusing to start the run instead would turn a
/// recoverable position into a run nobody can continue.
///
/// **It is not silent, either.** The names come back so that
/// `EntryRuntime::chain_cursor_rereads` can put a witness in the chain for each of them:
/// swallowing the failure would leave a restarted run re-reading every store with nothing
/// in the evidence saying it was about to. A missing file, a corrupt file, a file in a
/// shape this build does not read and a store the file never mentioned are one answer here
/// — from the tailer's side they are the same thing, which is starting at zero.
fn restore_cursors(
    tailers: &mut [BoundTailer],
    saved: &BTreeMap<String, crate::cursor::SavedCursor>,
    unsafe_file_first_restores: &BTreeSet<String>,
) -> Restored {
    let mut out = Restored::default();
    for bound in tailers.iter_mut() {
        let name = bound.binding.name();
        let restored = match saved
            .get(name)
            .filter(|_| !unsafe_file_first_restores.contains(name))
        {
            Some(saved) => bound.reader.restore(saved.cursor.clone()).is_ok(),
            None => false,
        };
        match restored {
            // The seq is kept only for a store whose position came back. For any other the
            // re-read is total and witnessed, and a budget over it would drop records the
            // chain does not hold.
            true => {
                out.saved_leg1_seq.insert(
                    name.to_string(),
                    saved.get(name).and_then(|s| s.last_leg1_seq),
                );
            }
            false => out.unrestored_stores.push(name.to_string()),
        }
    }
    out
}

/// What [`restore_cursors`] answers: which stores did not come back, and — for those that
/// did — where the chain stood when their position was taken.
#[derive(Default)]
struct Restored {
    unrestored_stores: Vec<String>,
    saved_leg1_seq: BTreeMap<String, Option<u64>>,
}

/// The envelope the export wrote at `seq`, read back off `chain.jsonl`.
///
/// `write_baseline` and `write_seal` return a `Link` and not an `Envelope` — the export
/// owns both bodies and builds them itself — and both links have to reach the
/// buffer, because the server refuses an entry's first chunk whose seq 0 is not the
/// baseline it witnessed. Reading the line back is how this crate gets the envelope
/// without building a second answer to what seq 0 is.
///
/// It **streams**, for the reason `upload::refill_gap` gives: one caller is the seal
/// (`EntryRuntime::seal`), and at that moment `chain.jsonl` is the whole run. A
/// `read_to_string` there puts the one allocation that can
/// fail between "the seal is on disk" and "the seal reached the buffer", so the entry that
/// captured the most would be the one that could not close. At most one line is held.
fn chain_line_at(
    reader: impl std::io::Read,
    diagnostic_path: &std::path::Path,
    seq: u64,
) -> Result<Envelope, DaemonError> {
    use std::io::BufRead;
    let io = |source: std::io::Error| DaemonError::Io {
        path: diagnostic_path.display().to_string(),
        source,
    };
    for line in std::io::BufReader::new(reader).lines() {
        let line = line.map_err(io)?;
        if line.trim().is_empty() {
            continue;
        }
        let env: Envelope = serde_json::from_str(&line)?;
        if env.link.seq == seq {
            return Ok(env);
        }
    }
    Err(DaemonError::Missing(format!(
        "{} has no link at seq {seq}, and the export said it wrote one",
        diagnostic_path.display()
    )))
}

/// What the chain says about one store's leg-1 records, per store.
struct Leg1Facts {
    /// The seq of the last `Leg1Record` this store has in the chain. What this process
    /// saves beside the store's position until it chains one of its own.
    last_seq: u64,
    /// How many of them sit above the seq `cursors.json` recorded — the records the
    /// restored tailer is about to hand back a second time.
    above_saved: usize,
}

struct RebuiltChainFacts {
    genesis_hash: String,
    baseline_manifest_sha256: String,
    baseline_file_count: u64,
    baseline_recorded_at: String,
    last_leg1_seq: BTreeMap<String, u64>,
    resume_skip: BTreeMap<String, usize>,
    foreign_seen: BTreeSet<(String, String, String)>,
}

fn rebuilt_receipt_mirror(
    entry: &Entry,
    persisted: receipt::PersistedReceipt,
    prepared: &PreparedTraceResume,
) -> Result<ReceiptMirrorState, DaemonError> {
    let diagnostic = entry.trace_dir.join(goalcup_protocol::export::RECEIPT_FILE);
    let reader = prepared.read_fixed(FixedTraceDocument::Receipt)?;
    match (persisted, reader) {
        (receipt::PersistedReceipt::NoAuthoritativeStart, None) => {
            Ok(ReceiptMirrorState::NoAuthoritativeStart)
        }
        (receipt::PersistedReceipt::NoAuthoritativeStart, Some(_)) => {
            Err(DaemonError::Refused(format!(
                "{} exists without an authoritative persisted start",
                diagnostic.display()
            )))
        }
        (
            receipt::PersistedReceipt::Authoritative {
                receipt,
                compact_bytes,
            },
            None,
        ) => Ok(ReceiptMirrorState::Owed {
            receipt,
            compact_bytes,
            mirror_ready: false,
        }),
        (receipt::PersistedReceipt::Authoritative { compact_bytes, .. }, Some(reader)) => {
            let actual = bounded_receipt_bytes(reader, &diagnostic)?;
            if actual != compact_bytes {
                return Err(DaemonError::Refused(format!(
                    "{} does not exactly match the registry-authoritative start receipt",
                    diagnostic.display()
                )));
            }
            Ok(ReceiptMirrorState::Ready)
        }
    }
}

fn bounded_receipt_bytes(
    reader: impl std::io::Read,
    diagnostic: &std::path::Path,
) -> Result<Vec<u8>, DaemonError> {
    use std::io::Read;
    let limit = MAX_FILE_BYTES
        .checked_add(1)
        .ok_or_else(|| DaemonError::Refused("receipt read bound does not fit u64".to_string()))?;
    let mut bytes = Vec::new();
    reader
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|source| DaemonError::Io {
            path: diagnostic.display().to_string(),
            source,
        })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES {
        return Err(DaemonError::Refused(format!(
            "{} exceeds the maximum fixed-document size",
            diagnostic.display()
        )));
    }
    Ok(bytes)
}

/// Check the registry as a prefix of the already protocol-validated chain, recover the
/// baseline commitment, and derive bounded replay counts from the same matched reader.
fn rebuilt_chain_facts(
    entry: &Entry,
    saved: &BTreeMap<String, Option<u64>>,
    prepared: &mut PreparedTraceResume,
) -> Result<RebuiltChainFacts, DaemonError> {
    use std::io::BufRead;
    let diagnostic = entry.trace_dir.join(CHAIN_FILE);
    let Some((chain_head_seq, _)) = prepared.head() else {
        return Err(DaemonError::Missing(format!(
            "{} is empty; a rebuilt entry requires its chained baseline",
            diagnostic.display()
        )));
    };
    if let Some(row_seq) = entry.chain_seq
        && row_seq > chain_head_seq
    {
        return Err(DaemonError::Refused(format!(
            "the registry head seq {row_seq} is ahead of the validated chain head {chain_head_seq}"
        )));
    }
    if entry.chain_seq.is_none() && entry.chain_head != goalcup_protocol::chain::GENESIS {
        return Err(DaemonError::Refused(
            "a registry row with no chain seq must retain the genesis hash".to_string(),
        ));
    }

    let reader = prepared.independent_chain_reader()?;
    let io = |source: std::io::Error| DaemonError::Io {
        path: diagnostic.display().to_string(),
        source,
    };
    let mut baseline_claim: Option<(String, u64, String)> = None;
    let mut genesis_hash: Option<String> = None;
    let mut prefix_found = entry.chain_seq.is_none();
    let mut leg1: BTreeMap<String, Leg1Facts> = BTreeMap::new();
    let mut foreign_seen = BTreeSet::new();
    for line in std::io::BufReader::new(reader).lines() {
        let line = line.map_err(io)?;
        let env: Envelope = serde_json::from_str(&line)?;
        if env.link.seq == 0 {
            if env.entry_id != entry.entry_id {
                return Err(DaemonError::Refused(format!(
                    "chain genesis belongs to entry {}, not registry entry {}",
                    env.entry_id, entry.entry_id
                )));
            }
            match &env.body {
                EventBody::Baseline {
                    manifest_sha256,
                    file_count,
                } => {
                    baseline_claim = Some((
                        manifest_sha256.clone(),
                        *file_count,
                        env.recorded_at.clone(),
                    ));
                }
                _ => {
                    return Err(DaemonError::Refused(
                        "the validated chain's seq 0 is not its baseline".to_string(),
                    ));
                }
            }
            genesis_hash = Some(env.link.hash.clone());
        }
        if entry.chain_seq == Some(env.link.seq) {
            if entry.chain_head != env.link.hash {
                return Err(DaemonError::Refused(format!(
                    "the registry hash at seq {} does not match the validated chain",
                    env.link.seq
                )));
            }
            prefix_found = true;
        }
        if matches!(
            &env.body,
            EventBody::CutoffReachedV1(_) | EventBody::Seal { .. } | EventBody::FinalizationV1(_)
        ) {
            return Err(DaemonError::Refused(format!(
                "the validated chain is terminal at seq {}; ordinary recorder restart is closed",
                env.link.seq
            )));
        }
        if let EventBody::ForeignSession {
            harness,
            session_id,
            store_path: _,
            first_seen: _,
        } = &env.body
        {
            // ForeignSession intentionally carries no family on the wire. Retain an
            // internal unknown-family marker; a fresh trusted scan supplies the family and
            // `chain_foreign_sighting` matches this marker before appending.
            foreign_seen.insert((
                UNKNOWN_FOREIGN_FAMILY.to_string(),
                harness.clone(),
                session_id.clone(),
            ));
        }
        if let EventBody::Leg1Record { store, .. } = env.body
            && let Some(floor) = saved.get(&store)
        {
            let floor = floor.unwrap_or(0);
            let facts = leg1.entry(store).or_insert(Leg1Facts {
                last_seq: env.link.seq,
                above_saved: 0,
            });
            facts.last_seq = env.link.seq;
            if env.link.seq > floor {
                facts.above_saved = facts.above_saved.checked_add(1).ok_or_else(|| {
                    DaemonError::Refused(
                        "the reconstructed per-store replay count exceeds usize".to_string(),
                    )
                })?;
            }
        }
    }
    if !prefix_found {
        return Err(DaemonError::Refused(format!(
            "the registry head seq {} is missing from the validated chain",
            entry.chain_seq.expect("the no-seq prefix starts matched")
        )));
    }
    let (baseline_manifest_sha256, baseline_file_count, baseline_recorded_at) = baseline_claim
        .ok_or_else(|| {
            DaemonError::Missing(
                "the validated nonempty chain has no seq-zero baseline".to_string(),
            )
        })?;
    let last_leg1_seq = leg1
        .iter()
        .map(|(store, facts)| (store.clone(), facts.last_seq))
        .collect();
    let resume_skip = leg1
        .into_iter()
        .filter(|(_, facts)| facts.above_saved > 0)
        .map(|(store, facts)| (store, facts.above_saved))
        .collect();
    Ok(RebuiltChainFacts {
        genesis_hash: genesis_hash.ok_or_else(|| {
            DaemonError::Missing("the validated nonempty chain has no seq-zero hash".to_string())
        })?,
        baseline_manifest_sha256,
        baseline_file_count,
        baseline_recorded_at,
        last_leg1_seq,
        resume_skip,
        foreign_seen,
    })
}

fn validate_rebuilt_baseline(
    entry: &Entry,
    baseline: &SealManifest,
    facts: &RebuiltChainFacts,
) -> Result<(), DaemonError> {
    let file_count = u64::try_from(baseline.files.len()).map_err(|_| {
        DaemonError::Refused("baseline file count does not fit the wire's u64".to_string())
    })?;
    let expected_workspace = entry.workspace.to_string_lossy().to_string();
    let digest = manifest_sha256(baseline)?;
    if baseline.entry_id != entry.entry_id
        || baseline.workspace != expected_workspace
        || baseline.sealed_at != facts.baseline_recorded_at
        || digest != facts.baseline_manifest_sha256
        || file_count != facts.baseline_file_count
    {
        return Err(DaemonError::Refused(
            "baseline.json does not match the registry identity and seq-zero chain commitment"
                .to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A manifest whose stores teach nobody keeps its own order, so a connector without an
    /// `AggregateLearned` spec is not silently reshuffled.
    #[test]
    fn a_connector_with_no_teacher_keeps_the_manifests_own_order() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../connectors/codex-cli/manifest.toml");
        let text = std::fs::read_to_string(&path).expect("the shipped manifest");
        let manifest =
            ConnectorManifest::from_toml_str(&text).expect("the shipped manifest parses");
        let order: Vec<&str> = poll_order(&manifest.store)
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(order, vec!["rollout", "history"]);
    }

    /// Production declarations are built only from a manifest store, so their parent is
    /// statically present. This test-only orphan crosses that otherwise unrepresentable
    /// invariant and proves the runtime's active-binding construction still refuses it.
    #[test]
    fn binding_state_refuses_an_orphan_parent_before_any_runtime_can_admit_it() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../connectors/codex-cli/manifest.toml");
        let text = std::fs::read_to_string(&path).expect("the shipped manifest");
        let manifest =
            ConnectorManifest::from_toml_str(&text).expect("the shipped manifest parses");
        let orphan = tailer::orphan_bound_tailer_for_runtime_test();
        match binding_state(
            &[orphan],
            &manifest,
            std::path::Path::new("/workspace"),
            None,
        ) {
            Err(DaemonError::Refused(message)) => {
                assert!(message.contains("unknown parent store"), "{message}");
            }
            Err(other) => panic!("expected orphan-parent refusal, got {other:?}"),
            Ok(_) => panic!("an orphan binding reached runtime admission state"),
        }
    }

    /// `chain_line_at` streams, so it answers over a chain far longer than any buffer a
    /// reader would allocate. The call that matters is the seal's: at that moment
    /// `chain.jsonl` is the whole run, and a reader that held the file would fail exactly
    /// on the entry that captured the most — after the seal was already appended.
    #[test]
    fn chain_line_at_finds_a_link_in_a_chain_far_longer_than_a_read_buffer() {
        use goalcup_protocol::chain::Link;
        use std::io::Write;

        const LINKS: u64 = 100_000;
        let tmp = tempfile::tempdir().expect("a temp dir");
        let path = tmp.path().join(CHAIN_FILE);
        let file = std::fs::File::create(&path).expect("the chain file");
        let mut out = std::io::BufWriter::new(file);
        for seq in 0..LINKS {
            let env = Envelope {
                protocol_version: goalcup_protocol::PROTOCOL_VERSION,
                entry_id: "e1".to_string(),
                source: "main".to_string(),
                recorded_at: "2026-09-04T00:00:00Z".to_string(),
                link: Link {
                    seq,
                    prev: format!("{:064x}", seq),
                    hash: format!("{:064x}", seq + 1),
                },
                body: EventBody::Leg1Record {
                    store: "main".to_string(),
                    payload: serde_json::Value::String(format!("{{\"n\":{seq}}}")),
                },
                redaction: None,
            };
            let line = serde_json::to_string(&env).expect("an envelope serialises");
            writeln!(out, "{line}").expect("the line is written");
        }
        out.flush().expect("the chain is on disk");

        let last = chain_line_at(
            std::fs::File::open(&path).expect("the chain opens"),
            &path,
            LINKS - 1,
        )
        .expect("the last link is found");
        assert_eq!(last.link.seq, LINKS - 1);
        assert_eq!(
            last.link.hash,
            format!("{:064x}", LINKS),
            "the line returned is the one at that seq, not a neighbour"
        );
        let first = chain_line_at(
            std::fs::File::open(&path).expect("the chain opens"),
            &path,
            0,
        )
        .expect("seq 0 is still found");
        assert_eq!(first.link.seq, 0);
        assert!(
            matches!(
                chain_line_at(
                    std::fs::File::open(&path).expect("the chain opens"),
                    &path,
                    LINKS,
                ),
                Err(DaemonError::Missing(_))
            ),
            "a seq the chain does not carry is Missing, not a wrong line"
        );
    }
}
