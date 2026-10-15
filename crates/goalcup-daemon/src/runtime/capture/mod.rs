//! Protocol-v5 local capture entries.
//!
//! A capture entry is created without a server, grant or receipt. Its registry row is marked
//! [`CaptureRowV1`]; its chain is a v5 [`TraceExport`] with a logical quota and no
//! preallocation, written inline with no redb payload buffer; and one authorization on the
//! daemon's ingest listener carries its launch-scoped secret. It lives in its own map beside
//! the legacy entries, so no tick, arming gate, upload pump or deadline reaches it.
//!
//! # One owner per entry
//!
//! Every producer submits to the entry's [`OwnerSink`]: wire producers through the ingest
//! receiver, the native adapter directly. The sink only checks and queues, under its own
//! short lock, and never calls back into the receiver. One owner thread drains it in
//! admission order, runs each producer's frames through that producer's own streaming
//! redactor, and appends the records to the [`ChainWriter`]. The same thread polls the native
//! adapter, applies the receiver's loss outcomes, flushes every redactor at least every
//! [`FLUSH_EVERY`] and then syncs the chain at least every [`SYNC_EVERY`], in that order. A
//! validated close becomes `ProducerClosed` on the daemon clock; a loss is interrupted and
//! chained as a gap and never closed.
//!
//! The goal is accepted once: redacted by the profile's streaming redactor, appended as one
//! `Goal` stream, closed, and synced before its key is answered. The launch record names that
//! stream and is appended after the harness is spawned. A schema-1 profile whose identity binds
//! at launch binds its one scoped native source there, or a stream-less `SourceChanged` gap
//! records its unavailability; a profile whose identity binding is deferred binds it at finish
//! instead and records no gap at launch (CONTRACTS.md,
//! "Amendment 2026-10-02 (C6 local finish budget, H-10 4a)" ¶9). Controlled-workload stores
//! retain run-HOME capture. Records are durable from the next sync.
//!
//! # Local finish
//!
//! The v5 seal runs inside one local-finish budget (`LOCAL_CAPTURE_FINISH_BUDGET`, CONTRACTS.md
//! "Amendment 2026-10-02 (C6 local finish budget, H-10 4a)"), split into a drain share and a
//! commit reserve ([`FINISH_COMMIT_RESERVE`]). It closes admission, lets live wire producers
//! drain until the end of the drain share, stops the owner (native finish, remaining producers
//! cut as losses, flush, sync), chains the proxy's counters as gaps, ends the registered process
//! tree and records its actual outcome (termination time is outside the budget), seals the
//! workspace (a clone's Git state or why it could not be read, else the walk; D56), and calls `write_local_finish` once. The
//! drain share expiring is the existing cut-with-a-gap, never a refusal. A stage that waits on a permit, the
//! entry's lock or the owner, or that is about to begin with less than [`FINISH_RESPONSE_MARGIN`]
//! left, is refused with [`DaemonError::FinishDeadline`]: no terminal is written, the entry stays
//! open, and a sticky [`ExpiredFinish`] makes the next finish record a `ShutdownDeadline` gap. A
//! failed terminal write is reported, never retried or re-synced, and the entry stays open to
//! say so. A finish whose seal is written records the entry's final
//! status on its row and lets the entry go: its ingest grant and its chain's descriptors are
//! dropped, its status is answered from the row, and a restart does not reopen its chain.
//!
//! # Who decides what a `Recovery` supersedes
//!
//! Four layers each track which producers are wire, and they differ on purpose (CONTRACTS.md,
//! H-16 ¶4). The receiver judges per connection, the sink entry-wide across wire, native and goal
//! producers, the writer chain validity under quota, and the reader is the authority. A
//! predecessor is an *orphan* when the chain holds nothing under its name; a `Loss` chains only as
//! a stream-less gap, so it names no producer.
//!
//! | Predecessor named by a `Recovery`      | Receiver, sink                        | Writer                              | Reader                               |
//! |----------------------------------------|---------------------------------------|-------------------------------------|--------------------------------------|
//! | no frame admitted (zero-record)        | orphan: supersedes every other wire   | orphan (unrecorded): same           | unrecorded: same                     |
//! | only `Loss` frames (loss-only epoch)   | orphan (`!named`): same               | orphan (unrecorded): same           | unrecorded: same                     |
//! | a named frame (`Connection`, payload)  | supersedes only the named producer    | wire: only the named producer;      | recorded wire: only the named        |
//! |                                        | (non-wire native/goal: refused)       | a recorded non-wire is refused      | producer; non-wire is invalid        |
//!
//! "Same" means every other wire producer not yet closed or lost. The sink and receiver test the
//! admission-side stand-in `!named`; it equals the writer's and reader's "unrecorded" while the
//! writer retains what the sink admitted, and where it does not the writer's `lost_producers`
//! keeps the chain valid. A named predecessor that is not wire in the writer (a payload `Open` after
//! a `Loss`, no `Connection`) is not reachable from the proxy's single lineage.
//!
//! A daemon restart reopens every other entry for status and seal only: no producer is admitted
//! again, and the gap count is read back from the chain, so gap history outlives the daemon. A
//! run whose harness was registered or launched and that did not finish gets one stream-less
//! `IngestLost` gap at reopen, since whatever it did while no daemon admitted its producers is
//! lost. A chain that already ends in its local finish takes nothing: the next finish only
//! commits that terminal to the registry.

use super::{CycleCounters, Daemon};

use crate::ingest::{
    EntryGrant, EntrySnapshot, IngestClock, IngestEntry, IngestSecret, Offsets, ProducerLoss,
    ProducerOutcome, lock,
};

use crate::native_capture::{
    NativeCapture, NativeCaptureConfig, NativeCaptureError, NativeCaptureLimits,
    NativeFinishBudget, NativeHint, NativeSourceSpec, NativeStoreForm, SystemNativeClock,
};

use crate::redact::stream::StreamRedactor;

use crate::registry::{
    CaptureRowV1, Entry, EntryState, ExpectedProcessHandoff, ProcessHandoffPhase, RunKeys,
};

use crate::workspace::{self, Roots};

use crate::{DAEMON_VERSION, DaemonError, now_rfc3339, status};

use goalcup_platform::tracecap::EntryArtifactRoot;

use goalcup_protocol::capture::{
    AnalysisState, AnchorState, CaptureFrame, CaptureMeta, CaptureRecord, CaptureSink,
    CaptureSource, CaptureState, CaptureTime, ConnectionEvent, DeliveryState, DurableFrontier,
    EndReason, EpochAdmission, FrameEvent, GapReason, M2_PROTOCOL_VERSION, M2Status,
    MAX_CAPTURE_QUEUE_BYTES, MAX_LIVE_PRODUCERS, MAX_OPEN_CONNECTIONS, MAX_OPEN_STREAMS,
    MAX_PRODUCERS, MAX_SEGMENT_BYTES, RunState, StreamKey, SubmitResult, is_capture_id,
};

use goalcup_protocol::chain::Link;

use goalcup_protocol::control::{
    ArmingState, CaptureFinishReport, CaptureLaunchRequest, CaptureSealRequest,
    CaptureWorkspaceInfo, CycleErrors, FINISH_COMMIT_RESERVE, LocalSealOutcome, ProxyCounters,
    SignedStatus,
};

use goalcup_protocol::envelope::{Envelope, EventBody};

use goalcup_protocol::events::HealthState;

use goalcup_protocol::export::{
    ExportError, FixedTraceDocument, TraceExport, TraceRoot, manifest_sha256,
};
use goalcup_protocol::seal::SealManifest;

use goalcup_protocol::harness::{CaptureSpec, HarnessManifest};
use goalcup_protocol::manifest::RedactionPattern;

use goalcup_protocol::process::{ProcessRegistration, validate_live_process_handle};

use goalcup_protocol::provenance::{TerminationFailureV1, TerminationOutcomeV1};

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use std::path::{Path, PathBuf};

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use std::thread::JoinHandle;

use std::time::{Duration, Instant};

mod finish;
mod harness;
mod harness_build;
mod native_scope;
mod owner;
mod sink;
mod writer;

use self::finish::*;
use self::harness::recorded_capture_harness;
pub(crate) use self::harness::{ProfileDigests, capture_harness};
use self::native_scope::*;
use self::owner::*;
use self::sink::*;
use self::writer::*;

/// The longest the owner waits for a queue item before checking its timers.
const IDLE_WAIT: Duration = Duration::from_millis(25);

/// What reading a clone's Git state may take at the baseline (D56): inside the CLI's 30 s
/// create call. A clone not read in it is admitted with the reason in place of the state, never
/// walked.
const BASELINE_GIT_BUDGET: Duration = Duration::from_secs(20);

/// One v5 capture entry of this daemon.
pub(crate) struct CaptureEntry {
    /// Creation owns the capture mutators. Reopen deliberately retains only status and seal.
    authority: CaptureAuthority,
    /// The profile's carried patterns, compiled into each producer's own redactor.
    extra_patterns: Vec<RedactionPattern>,
    /// The schema-1 termination binary a registered harness handle may name.
    pub(super) native_binary: Option<String>,
    /// Schema-declared scoped source, enabled once its session identity is bound: at launch or
    /// at finish according to `capture.identity.binding`.
    scoped_native: Option<NativeSourceSpec>,
    /// Where that source stands; see [`NativeBinding`]. Taken after `run`, never before it.
    native_binding: Mutex<NativeBinding>,
    /// The daemon's observation clock.
    clock: IngestClock,
    /// Where the entry notes a cause its chain has no room for; the daemon's own, once created.
    cycles: Arc<CycleCounters>,
    /// `None` for an entry reopened after a restart: it admits no producer again.
    ingest: Option<Arc<IngestEntry>>,
    sink: Arc<OwnerSink>,
    writer: Arc<Mutex<ChainWriter>>,
    /// The baseline recorded the workspace's Git state, read or not: the seal reads it again and
    /// never walks (D56). Fixed at creation: a finish reads it without the run lock, which
    /// another finish may hold. Taken from `baseline.json`, at the create and at a reopen alike; a
    /// trace without a readable one asks the workspace.
    clone: bool,
    /// Taken before `writer` whenever both are held.
    run: Mutex<CaptureRun>,
    owner: Arc<OwnerControl>,
    thread: Mutex<Option<JoinHandle<()>>>,
    /// Set by every finish attempt before it closes admission; nothing clears it. It is the
    /// "a finish has begun" signal a late native-scope bind is refused on (CONTRACTS.md, H-10
    /// 4a ¶9), and it is true for an attempt that expired as well as for one that sealed.
    finishing: AtomicBool,
    /// What an expired finish left for the next one to chain; see [`ExpiredFinish`].
    expired: Mutex<Option<ExpiredFinish>>,
    /// When this daemon created or reopened the entry, on a clock that does not count sleep: a
    /// laptop asleep mid-start does not age it toward [`super::UNLAUNCHED_LIMIT`].
    opened: std::time::Instant,
    /// The local finish once it is chained, before and after its registry row says so.
    pub(super) chained_finish: Mutex<Option<ChainedFinish>>,
    /// Test seam: a pause taken just before the named finish stage's admission check.
    #[cfg(any(test, feature = "test-hooks"))]
    stage_delay: Mutex<Option<(FinishStage, Duration)>>,
    /// Test seam: the launch parks before it reads the harness build, until cleared.
    #[cfg(any(test, feature = "test-hooks"))]
    hold_build: AtomicBool,
}

/// The authority retained by an in-memory capture runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptureAuthority {
    Live,
    RecoveredStatusSealOnly,
}

/// What the entry recorded outside the owner's stream of frames: its registry row as last
/// written, which says whether the run launched and how it finished, and its goal.
struct CaptureRun {
    entry: Entry,
    goal: Goal,
}

impl CaptureRun {
    fn launched(&self) -> bool {
        self.entry.state == EntryState::Started
    }

    /// The committed local finish, which a retried seal answers again.
    fn finished(&self) -> Option<&CaptureFinishReport> {
        self.entry.capture.as_ref()?.finished.as_ref()
    }

    /// Refused once the entry is finished: its chain takes nothing more, and its trace stays.
    fn refuse_if_finished(&self) -> Result<(), DaemonError> {
        match self.finished() {
            Some(_) => Err(DaemonError::Refused(format!(
                "entry {} is finished locally",
                self.entry.entry_id
            ))),
            None => Ok(()),
        }
    }
}

enum Goal {
    Open,
    /// Submitted and not accepted: a goal is never recorded twice, even after a failure.
    Attempted,
    Accepted(StreamKey),
    /// Its `Launch` is appended. A launch retried after the row update failed appends nothing.
    Launched(StreamKey),
}

/// The owner thread's stop request and how it ended.
#[derive(Default)]
struct OwnerControl {
    stop: AtomicBool,
    exited: AtomicBool,
    /// Set only by an exit that drained the queue into the chain first.
    drained: AtomicBool,
    /// What the native adapter may still spend when the owner stops.
    finish_budget_ns: AtomicU64,
    /// One bounded launch-to-owner handoff; no native poll precedes Launch.
    pending_native: Mutex<Option<NativeCapture>>,
    #[cfg(test)]
    fault: AtomicBool,
    /// Test seam: the owner parks after its stop request, before it drains, until cleared.
    #[cfg(any(test, feature = "test-hooks"))]
    hold_drain: AtomicBool,
}

impl OwnerControl {
    /// The owner is gone without having drained its queue into the chain.
    fn failed(&self) -> bool {
        self.exited.load(Ordering::Acquire) && !self.drained.load(Ordering::Acquire)
    }
}

impl Drop for CaptureEntry {
    fn drop(&mut self) {
        self.stop_owner(Duration::ZERO, None);
    }
}

/// A fresh random 128-bit identifier as 32 lowercase hex digits: a producer, or the
/// daemon's capture clock.
pub(super) fn fresh_capture_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

impl Daemon {
    /// Create a v5 local capture entry for `manifest`'s capture profile: preflight the
    /// profile's redaction, set the chain's quota, chain the v5 baseline of the fresh workspace,
    /// write the row, authorize ingest and start the owner. A refusal leaves no artifact, row
    /// or created workspace behind.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create_capture(
        &self,
        entry_id: String,
        workspace: &Path,
        run_home: &Path,
        config_home: &Path,
        manifest: &HarnessManifest,
        digests: &ProfileDigests,
        every_loaded_real_dir: &[String],
        reserve_bytes: u64,
        native_since: Option<u64>,
    ) -> Result<CaptureWorkspaceInfo, DaemonError> {
        let config_home = isolated_config_home(config_home, every_loaded_real_dir)?;
        let run_home = &isolated_run_home(
            manifest.capture.home.run_home,
            run_home,
            every_loaded_real_dir,
            &self.config().config_dir,
        )?;
        // The run's start, or else this entry's create time, which its row keeps (D57).
        let created_at = now_rfc3339();
        let since = match native_since {
            Some(since) => since,
            None => create_time_floor(&created_at)?,
        };
        let plan = native_plan(&manifest.capture, run_home, &config_home, since)?;
        let extra_patterns = crate::redact::extra_patterns_of(manifest);
        StreamRedactor::with_extra_patterns(&extra_patterns)?;
        let artifact_dir = self
            .config()
            .artifact_root
            .join("artifacts")
            .join(&entry_id);
        workspace::check_workspace_available(&Roots {
            workspace: workspace.to_path_buf(),
            artifact_root: artifact_dir.clone(),
        })?;
        workspace::check_machine_roots(
            workspace,
            &[
                ("artifact_root", &self.config().artifact_root),
                ("config_dir", &self.config().config_dir),
            ],
        )?;
        // After the cheap checks, so a create they refuse makes no directory; before anything
        // opens a root (D57).
        create_home_roots(&manifest.capture, run_home, &config_home)?;
        let artifact = self
            .artifact_root()
            .create_entry(&entry_id)
            .map_err(|error| {
                DaemonError::Refused(format!(
                    "fresh entry artifact authority cannot be acquired: {error}"
                ))
            })?;
        let workspace_existed = workspace.exists();
        let opened = self.open_capture(
            &entry_id,
            workspace,
            run_home,
            &config_home,
            manifest,
            digests,
            reserve_bytes,
            extra_patterns,
            &artifact,
            &artifact_dir,
            plan,
            created_at,
        );
        if let Err(error) = &opened {
            let created = (!workspace_existed).then_some(workspace);
            if let Err(cleanup) = self.discard_capture(&entry_id, &artifact, created) {
                return Err(DaemonError::Refused(format!(
                    "capture setup failed ({error}); its cleanup also failed: {cleanup}"
                )));
            }
        }
        opened
    }

    #[allow(clippy::too_many_arguments)]
    fn open_capture(
        &self,
        entry_id: &str,
        workspace: &Path,
        run_home: &Path,
        config_home: &Path,
        manifest: &HarnessManifest,
        digests: &ProfileDigests,
        reserve_bytes: u64,
        extra_patterns: Vec<RedactionPattern>,
        artifact: &EntryArtifactRoot,
        artifact_dir: &Path,
        plan: NativePlan,
        created_at: String,
    ) -> Result<CaptureWorkspaceInfo, DaemonError> {
        let trace = artifact.create_trace().map_err(|error| {
            DaemonError::Refused(format!(
                "the capture trace directory cannot be created: {error}"
            ))
        })?;
        let mut export = TraceExport::create_from_root(TraceRoot::from_acquired(
            trace,
            artifact_dir.join("trace"),
        ))?;
        // Selects v5 on the empty chain, before the baseline. A quota only: the run host holds
        // the run's one disk reservation.
        export
            .configure_capture_quota(reserve_bytes)
            .map_err(|error| {
                DaemonError::Config(format!(
                    "the capture chain cannot take a {reserve_bytes} byte quota: {error}"
                ))
            })?;
        std::fs::create_dir_all(workspace).map_err(|source| DaemonError::Io {
            path: workspace.display().to_string(),
            source,
        })?;
        let workspace = std::fs::canonicalize(workspace).map_err(|source| DaemonError::Io {
            path: workspace.display().to_string(),
            source,
        })?;
        let mut baseline = workspace::seal_workspace(
            &workspace,
            workspace::is_clone_root(&workspace),
            entry_id,
            &now_rfc3339(),
            BASELINE_GIT_BUDGET,
        )?;
        baseline.home = listed_home(&manifest.capture, run_home, config_home);
        export.write_baseline_for_protocol(M2_PROTOCOL_VERSION, &baseline)?;
        export.sync_capture()?;
        let reserved_bytes = export.capture_storage().reserved_bytes;
        let chain_head = export
            .head()
            .map(|(_, hash)| hash)
            .ok_or_else(|| DaemonError::Refused("the baseline was not chained".into()))?;

        let keys = RunKeys::generate();
        let run_public_key = keys.public_key_hex()?;
        let mut entry = Entry::new(
            entry_id,
            &manifest.id,
            workspace,
            artifact_dir.to_path_buf(),
            &digests.profile_sha256,
            "",
            ExpectedProcessHandoff::Required,
            keys,
        );
        entry.connector_dir_sha256 = Some(digests.connector_dir_sha256.clone());
        // The instant the native floor was taken from, when the request named no run start.
        entry.created_at = created_at;
        entry.capture = Some(CaptureRowV1 {
            reserve_bytes,
            finished: None,
            sealed_status: None,
        });
        // No arming gate: the entry is created ready for its one daemon-launched process.
        entry.state = EntryState::Armed;
        // One entry holds a workspace (D4), except that a capture entry that routes nothing
        // lets it go: a sealed one, and one this daemon failed to reopen at start. An entry
        // merely absent from the captures map is not that: a create in flight is stored before
        // it is inserted. That is how a continued run makes its next segment in the workspace
        // its last one had.
        let reopen_failed = lock(&self.inner.reopen_failed).clone();
        self.registry().put_letting_go(&entry, &|stored| {
            stored.capture.as_ref().is_some_and(|row| {
                row.sealed_status.is_some() || reopen_failed.contains(&stored.entry_id)
            })
        })?;
        let entry = self.registry().open_initial_process_handoff(entry_id)?;
        let process_generation = entry.process_handoff_generation();

        #[cfg(any(test, feature = "test-hooks"))]
        if let Some((bytes, age)) = *lock(&self.inner.part_limits) {
            export.set_part_limits(bytes, age);
        }
        let clock = self.inner.capture_clock.clone();
        let writer = ChainWriter::new(export, entry_id.to_string());
        let mut capture = CaptureEntry::new_with_authority(
            entry,
            Goal::Open,
            writer,
            clock.clone(),
            CaptureAuthority::Live,
        );
        // Prepare the fixed isolated source root before spawn: the leaf opens it once.
        if let Some(source) = &plan.scoped {
            // A `{config_home}` root, or a `{run_home}` root under a shared run `HOME`, was
            // created above, and a failure there refused the create. For a per-run `{run_home}`
            // root, an unavailable root must not stop the harness: the Launch
            // constructor reports the required-source gap if this could not make it available.
            let _ = std::fs::create_dir_all(&source.root);
        }
        capture.cycles = self.inner.cycles.clone();
        capture.scoped_native = plan.scoped;
        *lock(&capture.native_binding) = plan.initial_binding;
        let native = if plan.controlled.is_empty() {
            None
        } else {
            Some(
                native_capture(plan.controlled, capture.sink.clone())
                    .map_err(|error| DaemonError::Config(format!("native capture: {error}")))?,
            )
        };
        let secret = IngestSecret::generate();
        let ingest = Arc::new(self.authorize_ingest(EntryGrant {
            entry_id: entry_id.to_string(),
            secret: secret.clone(),
            sink: capture.sink.clone(),
            prior_producers: 0,
        })?);
        let owner = Owner::new(
            capture.sink.clone(),
            capture.writer.clone(),
            clock,
            &extra_patterns,
            Some(ingest.clone()),
            native,
        )
        .noting_to(self.inner.cycles.clone());
        let thread = owner
            .spawn(capture.owner.clone())
            .map_err(|source| DaemonError::Io {
                path: "the capture owner thread".to_string(),
                source,
            })?;
        capture.thread = Mutex::new(Some(thread));
        capture.extra_patterns = extra_patterns;
        capture.native_binary = manifest.termination().native_binary;
        capture.ingest = Some(ingest);
        self.captures()
            .insert(entry_id.to_string(), Arc::new(capture));
        Ok(CaptureWorkspaceInfo {
            entry_id: entry_id.to_string(),
            process_generation,
            reserved_bytes,
            ingest_addr: self.inner.ingest_addr,
            ingest_secret: secret.expose().to_string(),
            producer_id: fresh_capture_id(),
            run_public_key,
            chain_head,
            trace_dir: artifact_dir.join("trace").display().to_string(),
        })
    }

    /// Remove what a v5 entry holds on disk: its artifact tree with the reserved chain, then
    /// its row, then `workspace` if that directory is still empty.
    fn discard_capture(
        &self,
        entry_id: &str,
        artifact: &EntryArtifactRoot,
        workspace: Option<&Path>,
    ) -> Result<(), DaemonError> {
        artifact.remove_recursively().map_err(|error| {
            DaemonError::Refused(format!(
                "entry {entry_id}'s artifacts cannot be removed through its retained authority: \
                 {error}"
            ))
        })?;
        self.registry().remove(entry_id)?;
        if let Some(workspace) = workspace {
            let _ = std::fs::remove_dir(workspace);
        }
        Ok(())
    }

    /// Abandon a v5 entry before its launch and before any process handoff is prepared: its
    /// reservation, row, artifacts and still-empty workspace go, as for a refused create, and
    /// only then its owner and ingest grant. After a launch or a handoff, it is sealed instead.
    ///
    /// A recovered entry may be abandoned too: what makes an abandon safe is that nothing was
    /// handed off, which the checks below prove, and a never-launched entry the CLI can no
    /// longer finish would otherwise hold its reservation and keep a run-scoped daemon up for
    /// good (review R4 I1).
    pub(super) fn abandon_capture(&self, entry_id: &str) -> Result<(), DaemonError> {
        let capture = self.capture_entry(entry_id)?;
        let run = capture.lock();
        run.refuse_if_finished()?;
        let row = self
            .registry()
            .get(entry_id)?
            .ok_or_else(|| DaemonError::Missing(format!("entry {entry_id}")))?;
        if run.launched() || row.process_handoff_phase() != ProcessHandoffPhase::OpenInitial {
            return Err(DaemonError::Refused(format!(
                "entry {entry_id} has a launched or registered harness, so it is not abandoned: \
                 seal it instead"
            )));
        }
        let artifact = self.artifact_root().open_entry(entry_id).map_err(|error| {
            DaemonError::Refused(format!(
                "entry {entry_id}'s artifact authority cannot be acquired: {error}"
            ))
        })?;
        self.discard_capture(entry_id, &artifact, Some(&row.workspace))?;
        self.captures().remove(entry_id);
        Ok(())
    }

    /// Abandon every entry that never launched, has handed nothing off (its registry row is
    /// still `OpenInitial`) and was opened by this daemon more than `older_than` ago. A
    /// `goalcup start` goes from create to launch in seconds, so such an entry's CLI died (it
    /// was killed, or its terminal closed): nothing will ever seal or abandon it. An entry with
    /// a registered harness is left for `goalcup seal`. The idle loop calls this; a failure is
    /// noted and retried at the next poll.
    pub fn abandon_stale_unlaunched(&self, older_than: std::time::Duration) {
        let candidates: Vec<String> = self
            .captures()
            .values()
            .filter(|capture| capture.opened.elapsed() >= older_than)
            .filter_map(|capture| {
                let run = capture.run.try_lock().ok()?;
                (!run.launched() && run.finished().is_none()).then(|| run.entry.entry_id.clone())
            })
            .collect();
        // Registration writes only the registry, so the row is what says nothing was handed off.
        let stale = candidates.into_iter().filter(|entry_id| {
            self.registry()
                .get(entry_id)
                .ok()
                .flatten()
                .is_some_and(|row| row.process_handoff_phase() == ProcessHandoffPhase::OpenInitial)
        });
        for entry_id in stale {
            if let Err(error) = self.abandon_capture(&entry_id) {
                self.inner.cycles.note(&error);
            }
        }
    }

    /// Admit a v5 entry's harness process: at the generation its create answered, once, and
    /// before its launch is recorded. A capture launch carries no home seed.
    pub(super) fn register_capture_process(
        &self,
        entry_id: &str,
        registration: ProcessRegistration,
    ) -> Result<(), DaemonError> {
        let capture = self.capture_entry(entry_id)?;
        capture.require_live_capture_authority()?;
        let run = capture.lock();
        run.refuse_if_finished()?;
        if run.launched() {
            return Err(DaemonError::Refused(format!(
                "entry {entry_id}'s launch is recorded, and its process is registered before it"
            )));
        }
        let generation = registration.generation;
        let registry = self.registry();
        let entry = registry.prepare_process_handoff(entry_id, registration)?;
        if entry.process_handoff_phase() == ProcessHandoffPhase::Registered {
            return Ok(());
        }
        let prepared = registry.prepared_process_handle(entry_id, generation)?;
        validate_live_process_handle(&prepared).map_err(|reason| {
            DaemonError::Refused(format!(
                "entry {entry_id}'s prepared process could not be revalidated: {reason}"
            ))
        })?;
        registry.register_prepared_process_handoff(entry_id, generation)?;
        Ok(())
    }

    /// Whether this daemon holds a v5 capture entry `entry_id`: open in its map, or sealed (its
    /// row says so). A row that failed to reopen is neither; the control layer's
    /// `is_capture_row` reads the registry row alone.
    pub(super) fn has_capture_entry(&self, entry_id: &str) -> bool {
        self.captures().contains_key(entry_id)
            || matches!(self.sealed_capture(entry_id), Ok(Some(_)))
    }

    /// Refuse a mutating control route when `entry_id` is an unfinished capture reopened after
    /// restart. Non-capture entries pass through to their own runtime. A capture row whose live
    /// authority cannot be found fails closed; a sealed row may still use its existing read-only
    /// exact-replay acknowledgement path.
    pub(crate) fn require_live_capture_mutation_authority(
        &self,
        entry_id: &str,
    ) -> Result<(), DaemonError> {
        let Some(row) = self.registry().get(entry_id)? else {
            return Ok(());
        };
        let Some(capture_row) = row.capture.as_ref() else {
            return Ok(());
        };
        if capture_row.sealed_status.is_some() {
            return Ok(());
        }
        let capture = self.captures().get(entry_id).cloned().ok_or_else(|| {
            DaemonError::Refused(format!(
                "entry {entry_id} is a recovered capture restricted to status and seal; the \
                 daemon cannot prove live capture mutation authority"
            ))
        })?;
        capture.require_live_capture_authority()
    }

    /// Record the entry's goal once and answer its stream key after the chain is synced.
    /// The caller has checked the goal's size and encoding.
    pub(crate) fn record_capture_goal(
        &self,
        entry_id: &str,
        goal: &[u8],
    ) -> Result<StreamKey, DaemonError> {
        self.capture_entry(entry_id)?.record_goal(goal)
    }

    /// Append the entry's `Launch` record for the harness the caller spawned, then mark the
    /// row started, which is how the entry (and a reopened one) knows it launched.
    pub(crate) fn record_capture_launch(
        &self,
        entry_id: &str,
        request: &CaptureLaunchRequest,
    ) -> Result<(), DaemonError> {
        let capture = self.capture_entry(entry_id)?;
        capture.require_live_capture_authority()?;
        // D53: which build ran, read before the run lock is taken, because it reads whole
        // binaries. The launch post follows the release, so the registered process has `exec`ed.
        // A retried post of a launch already chained is answered below without a second read.
        let already_launched = matches!(
            &capture.lock().goal,
            Goal::Launched(key) if *key == request.goal_stream
        );
        let harness_build = if already_launched {
            None
        } else {
            self.registry()
                .get(entry_id)?
                .and_then(|row| row.last_process)
                .map(|handle| {
                    #[cfg(any(test, feature = "test-hooks"))]
                    while capture.hold_build.load(Ordering::Acquire) {
                        std::thread::sleep(IDLE_WAIT);
                    }
                    Box::new(harness_build::observe(&handle))
                })
        };
        let mut run = capture.lock();
        capture.record_launch(&mut run, request, harness_build)?;
        run.entry = self.registry().update(entry_id, |entry| {
            entry.state = EntryState::Started;
            entry.harness_version = request.harness_version.clone();
        })?;
        Ok(())
    }

    /// Test seam: park (or release) the entry's next launch just before it reads the harness
    /// build, as a slow read of a large binary would hold it.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn hold_capture_harness_build(&self, entry_id: &str, hold: bool) {
        if let Some(capture) = self.captures().get(entry_id) {
            capture.hold_build.store(hold, Ordering::Release);
        }
    }

    /// Reopen every v5 row but the sealed ones for status and seal after a restart. A row that
    /// cannot be reopened is counted as a cycle error and has no status.
    pub(super) fn reopen_captures(&self) {
        let rows = match self.registry().all() {
            Ok(rows) => rows,
            Err(error) => return self.inner.cycles.note(&error),
        };
        let unsealed = |row: &Entry| {
            row.capture
                .as_ref()
                .is_some_and(|kept| kept.sealed_status.is_none())
        };
        for row in rows.into_iter().filter(unsealed) {
            let entry_id = row.entry_id.clone();
            if let Err(error) = self.reopen_capture(row) {
                lock(&self.inner.reopen_failed).insert(entry_id.clone());
                self.inner.cycles.note(&DaemonError::Refused(format!(
                    "capture entry {entry_id} was not reopened: {}",
                    super::path_free(&error)
                )));
            }
        }
    }

    /// Ids of the entries [`Self::reopen_captures`] failed to reopen, in id order.
    pub fn reopen_failed_ids(&self) -> Vec<String> {
        lock(&self.inner.reopen_failed).iter().cloned().collect()
    }

    fn reopen_capture(&self, row: Entry) -> Result<(), DaemonError> {
        let reserve_bytes = row
            .capture
            .as_ref()
            .map(|kept| kept.reserve_bytes)
            .ok_or_else(|| DaemonError::Missing(format!("entry {}", row.entry_id)))?;
        let refused = |error: goalcup_platform::tracecap::TraceCapError| {
            DaemonError::Refused(format!("the capture trace cannot be acquired: {error}"))
        };
        let artifact = self
            .artifact_root()
            .open_entry(&row.entry_id)
            .map_err(refused)?;
        let trace = artifact.open_trace().map_err(refused)?;
        // One read of the chain: the resume validates every line and hands it to `facts`.
        let mut chained = ChainFacts::default();
        let mut export = TraceExport::resume_from_root_visiting(
            TraceRoot::from_acquired(trace, row.trace_dir.clone()),
            &mut |envelope| chained.observe(envelope),
        )?;
        export.configure_capture_quota(reserve_bytes)?;
        export.sync_capture()?;
        if let (Some(finish), Some(digest)) = (chained.finish.as_mut(), &chained.seal_digest) {
            bind_seal(&export, finish, digest);
        }
        let mut writer = ChainWriter::new(export, row.entry_id.clone());
        writer.gaps = chained.gaps;
        // A chain that already ends in its local finish takes nothing more, whatever the row
        // says: the next finish only commits that terminal to the registry.
        let unfinished = chained.finish.is_none()
            && row
                .capture
                .as_ref()
                .is_some_and(|kept| kept.finished.is_none());
        // A registered harness is running before its launch is appended, and the row is marked
        // started only after that append.
        let launched = row.state == EntryState::Started || row.last_process.is_some();
        if launched && unfinished {
            // The run went on while no daemon admitted its producers, and nothing is admitted
            // from here on: that ingest is lost, and a stream-less gap says so, whether or not
            // a stop chained one before.
            writer.streamless_gap(GapReason::IngestLost, &(self.inner.capture_clock)());
            // A failed sync is latched by the writer and reported by status.
            let _ = writer.sync();
        }
        // The binding is in memory: a reopened entry has no source to install, and a launched
        // Hermes run still owes the finish's deferred-scope gap.
        // Unless the chain already holds it: a finish that chained the gap and was refused
        // after it must not chain a second one when the next daemon finishes.
        let deferred = launched
            && unfinished
            && self.recorded_harness_defers_scope(
                &row.connector_id,
                row.connector_dir_sha256.as_deref(),
            )
            && !chained.launch_source_gap;
        let entry_id = row.entry_id.clone();
        let capture = CaptureEntry::new_with_authority(
            row,
            Goal::Attempted,
            writer,
            self.inner.capture_clock.clone(),
            CaptureAuthority::RecoveredStatusSealOnly,
        );
        if deferred {
            *lock(&capture.native_binding) = NativeBinding::Deferred;
        }
        *lock(&capture.chained_finish) = chained.finish;
        self.captures().insert(entry_id, Arc::new(capture));
        Ok(())
    }

    /// Every v5 entry's summary, for the entry list: the open entries, then the sealed rows.
    pub(super) fn capture_summaries(
        &self,
    ) -> Result<Vec<goalcup_protocol::control::EntrySummary>, DaemonError> {
        let captures: BTreeMap<_, _> = self.captures().clone();
        let mut summaries: Vec<_> = captures
            .values()
            .map(|capture| {
                let run = capture.lock();
                let (gate, health) = match (run.finished(), run.launched()) {
                    (Some(_), _) => (ArmingState::Started, HealthState::Sealed),
                    (None, true) => (ArmingState::Started, HealthState::Online),
                    (None, false) => (ArmingState::Armed, HealthState::Online),
                };
                status::summary(&run.entry, gate, health)
            })
            .collect();
        for row in self.registry().all()? {
            let sealed = row
                .capture
                .as_ref()
                .is_some_and(|kept| kept.sealed_status.is_some());
            if sealed && !captures.contains_key(&row.entry_id) {
                summaries.push(status::summary(
                    &row,
                    ArmingState::Started,
                    HealthState::Sealed,
                ));
            }
        }
        Ok(summaries)
    }

    /// Whether any capture this daemon holds has not finished: what keeps a run-scoped daemon
    /// alive (`Daemon::run_until_idle`). A run whose lock is held right now counts as
    /// unfinished, because its holder may be finishing it or may not, and the poll that asks
    /// must not block a runtime thread waiting to find out.
    pub(crate) fn holds_unfinished_capture(&self) -> bool {
        self.captures()
            .values()
            .any(|capture| match capture.run.try_lock() {
                Ok(run) => run.finished().is_none(),
                Err(std::sync::TryLockError::Poisoned(run)) => {
                    run.into_inner().finished().is_none()
                }
                Err(std::sync::TryLockError::WouldBlock) => true,
            })
    }

    /// A freshly signed status for a v5 entry of this daemon, or `None` for any other entry.
    pub(crate) fn capture_status(
        &self,
        entry_id: &str,
    ) -> Result<Option<SignedStatus>, DaemonError> {
        let cycles = self.cycle_errors();
        let capture = self.captures().get(entry_id).cloned();
        if let Some(capture) = capture {
            return capture.signed_status(cycles).map(Some);
        }
        match self.sealed_capture(entry_id)? {
            Some((row, sealed)) => sign_capture_status(&row, sealed, None, cycles).map(Some),
            None => Ok(None),
        }
    }

    /// An open v5 entry. A sealed one is refused, as an open entry refuses once finished.
    pub(super) fn capture_entry(&self, entry_id: &str) -> Result<Arc<CaptureEntry>, DaemonError> {
        let open = self.captures().get(entry_id).cloned();
        if let Some(capture) = open {
            return Ok(capture);
        }
        Err(match self.sealed_capture(entry_id)? {
            Some(_) => DaemonError::Refused(format!("entry {entry_id} is finished locally")),
            None => DaemonError::Missing(format!("no capture entry {entry_id} in this daemon")),
        })
    }

    /// A v5 row whose finish wrote its seal, with the status that finish left.
    fn sealed_capture(&self, entry_id: &str) -> Result<Option<(Entry, M2Status)>, DaemonError> {
        let Some(row) = self.registry().get(entry_id)? else {
            return Ok(None);
        };
        let sealed = row
            .capture
            .as_ref()
            .and_then(|kept| kept.sealed_status.clone());
        Ok(sealed.map(|sealed| (row, sealed)))
    }

    fn captures(&self) -> MutexGuard<'_, BTreeMap<String, Arc<CaptureEntry>>> {
        lock(&self.inner.captures)
    }

    fn authorize_ingest(&self, grant: EntryGrant) -> Result<IngestEntry, DaemonError> {
        let listener = lock(&self.inner.ingest);
        listener
            .as_ref()
            .ok_or_else(|| DaemonError::Refused("the capture ingest listener is shut down".into()))?
            .authorize(grant)
            .map_err(|error| DaemonError::Refused(format!("capture ingest authorization: {error}")))
    }
}

/// What a reopen needs from the chain, gathered during the resume's own validation pass.
#[derive(Default)]
struct ChainFacts {
    /// Gap records: gap history outlives the daemon.
    gaps: u64,
    /// A stream-less `SourceChanged` gap after the `Launch`: the one gap a deferred native scope
    /// owes the run, already recorded.
    launch_source_gap: bool,
    launched: bool,
    /// Whether any envelope was visited yet.
    visited: bool,
    /// The local finish, when the chain already ends in one, and the `seal.json` digest it
    /// binds.
    finish: Option<ChainedFinish>,
    seal_digest: Option<String>,
}

/// Read `seal.json` back and mark `finish` written only when it is there whole and is the
/// manifest the terminal binds; its `sealed_at` is then the seal's own. A terminal chained
/// without its `seal.json` (a crash between the two, or a full disk) stays a failed seal.
fn bind_seal(export: &TraceExport, finish: &mut ChainedFinish, digest: &str) {
    let Some(manifest) = read_manifest(export, FixedTraceDocument::Seal) else {
        return;
    };
    if manifest_sha256(&manifest).is_ok_and(|bound| bound == digest) {
        finish.sealed_at = manifest.sealed_at;
        finish.seal_written = true;
    }
}

/// `seal.json` or `baseline.json` as written, when it is there whole and parses.
fn read_manifest(export: &TraceExport, document: FixedTraceDocument) -> Option<SealManifest> {
    use std::io::Read;
    const MANIFEST_MAX_BYTES: u64 = 16 * 1024 * 1024;
    let file = export.read_fixed(document).ok()??;
    let mut bytes = Vec::new();
    file.take(MANIFEST_MAX_BYTES).read_to_end(&mut bytes).ok()?;
    serde_json::from_slice(&bytes).ok()
}

impl ChainFacts {
    fn observe(&mut self, envelope: &Envelope) {
        // A chain read from above seq 0 lost its first parts after their acknowledgement (D55),
        // and with them the `Launch`: such a chain launched long ago.
        if !self.visited && envelope.link.seq > 0 {
            self.launched = true;
        }
        self.visited = true;
        match &envelope.body {
            EventBody::Capture {
                record: CaptureRecord::Launch { .. },
            } => self.launched = true,
            EventBody::Capture {
                record: CaptureRecord::Gap { stream, reason, .. },
            } => {
                self.gaps += 1;
                if self.launched && stream.is_none() && *reason == GapReason::SourceChanged {
                    self.launch_source_gap = true;
                }
            }
            EventBody::Capture {
                record:
                    CaptureRecord::LocalFinish {
                        termination,
                        seal_manifest_sha256,
                        ..
                    },
            } => {
                self.finish = Some(ChainedFinish {
                    seq: envelope.link.seq,
                    termination: termination.clone(),
                    // Until `seal.json` is read back and binds the terminal: see `bind_seal`.
                    sealed_at: envelope.recorded_at.clone(),
                    seal_written: false,
                });
                self.seal_digest = Some(seal_manifest_sha256.clone());
            }
            _ => {}
        }
    }
}

/// A local finish already chained and synced, whose registry row may not say so yet: a
/// registry write that failed after the terminal, or a daemon that died between the two. A
/// later finish commits it to the registry and appends nothing after the terminal.
#[derive(Clone)]
pub(super) struct ChainedFinish {
    pub(super) seq: u64,
    pub(super) termination: TerminationOutcomeV1,
    pub(super) sealed_at: String,
    /// Whether its `seal.json` is written and binds it; a finish without one is a failed seal.
    pub(super) seal_written: bool,
}

/// Sign a v5 entry's status with `capture` as its capture projection and the receiver's
/// `admission` counts, which only an entry still holding its ingest grant has.
fn sign_capture_status(
    entry: &Entry,
    capture: M2Status,
    admission: Option<EntrySnapshot>,
    cycles: CycleErrors,
) -> Result<SignedStatus, DaemonError> {
    let mut payload = status::build_payload(&status::StatusInputs {
        entry,
        arming: if entry.state == EntryState::Started {
            ArmingState::Started
        } else {
            ArmingState::Armed
        },
        cycles,
        capture,
    });
    payload.producers_consumed = admission.map(|a| a.producers_consumed as u64);
    payload.admissions_refused = admission.map(|a| a.admissions_refused);
    status::sign_payload(payload, now_rfc3339(), &entry.keys.signing_key()?)
}

impl CaptureEntry {
    /// An entry over `writer` with no ingest grant, owner or native adapter yet: a reopened
    /// entry stays that way, and a created one is given them.
    fn new_with_authority(
        entry: Entry,
        goal: Goal,
        writer: ChainWriter,
        clock: IngestClock,
        authority: CaptureAuthority,
    ) -> CaptureEntry {
        CaptureEntry {
            authority,
            extra_patterns: Vec::new(),
            native_binary: None,
            scoped_native: None,
            native_binding: Mutex::new(NativeBinding::NotRequired),
            clock,
            cycles: Arc::new(CycleCounters::default()),
            ingest: None,
            sink: Arc::new(OwnerSink::default()),
            clone: read_manifest(&writer.export, FixedTraceDocument::Baseline).map_or_else(
                || workspace::is_clone_root(&entry.workspace),
                |b| b.git.is_some(),
            ),
            writer: Arc::new(Mutex::new(writer)),
            run: Mutex::new(CaptureRun { entry, goal }),
            owner: Arc::new(OwnerControl::default()),
            thread: Mutex::new(None),
            finishing: AtomicBool::new(false),
            expired: Mutex::new(None),
            opened: std::time::Instant::now(),
            chained_finish: Mutex::new(None),
            #[cfg(any(test, feature = "test-hooks"))]
            stage_delay: Mutex::new(None),
            #[cfg(any(test, feature = "test-hooks"))]
            hold_build: AtomicBool::new(false),
        }
    }

    /// Unit fixtures construct a fresh in-process entry and therefore hold creation-time Live
    /// authority. Production create and reopen call [`Self::new_with_authority`] explicitly.
    #[cfg(test)]
    fn new(entry: Entry, goal: Goal, writer: ChainWriter, clock: IngestClock) -> CaptureEntry {
        Self::new_with_authority(entry, goal, writer, clock, CaptureAuthority::Live)
    }

    fn lock(&self) -> MutexGuard<'_, CaptureRun> {
        lock(&self.run)
    }

    pub(crate) fn require_live_capture_authority(&self) -> Result<(), DaemonError> {
        match self.authority {
            CaptureAuthority::Live => Ok(()),
            CaptureAuthority::RecoveredStatusSealOnly => {
                let entry_id = self.lock().entry.entry_id.clone();
                Err(DaemonError::Refused(format!(
                    "entry {entry_id} is a recovered capture restricted to status and seal"
                )))
            }
        }
    }

    fn record_goal(&self, goal: &[u8]) -> Result<StreamKey, DaemonError> {
        self.require_live_capture_authority()?;
        let mut run = self.lock();
        run.refuse_if_finished()?;
        if !matches!(run.goal, Goal::Open) {
            return Err(DaemonError::Refused(
                "this entry's goal was already submitted, and a goal is recorded once".into(),
            ));
        }
        run.goal = Goal::Attempted;
        let key = StreamKey {
            producer: fresh_capture_id(),
            stream: 1,
        };
        // The goal producer counts toward the entry's producer bound like any other.
        if !self.sink.reserve_closed(&key.producer) {
            return Err(DaemonError::Refused(
                "this entry has no producer left for its goal".into(),
            ));
        }
        let events = [
            FrameEvent::Open {
                source: CaptureSource::Goal,
                metadata: CaptureMeta {
                    wire: None,
                    label: "goal".to_string(),
                    protocol_hint: None,
                    method: None,
                    host: None,
                    path: None,
                    status: None,
                    content_type: None,
                },
            },
            FrameEvent::Bytes {
                offset: 0,
                data: goal.to_vec(),
            },
            FrameEvent::End {
                reason: EndReason::Complete,
            },
        ];
        let mut redactor = StreamRedactor::with_extra_patterns(&self.extra_patterns)?;
        let mut writer = lock(&self.writer);
        let mut final_seq = None;
        for (seq, event) in (0u64..).zip(events) {
            let observed = (self.clock)();
            let frame = CaptureFrame {
                stream: key.clone(),
                seq,
                observed: observed.clone(),
                event,
            };
            let records = redactor.push(frame).map_err(|error| {
                DaemonError::Refused(format!("the goal redactor refused a frame: {error}"))
            })?;
            for record in records {
                writer.require(record, &observed)?;
            }
            final_seq = Some(seq);
        }
        // The goal producer is complete: every frame is in and its stream ended.
        let observed = (self.clock)();
        let closed = CaptureRecord::ProducerClosed {
            producer: key.producer.clone(),
            observed: observed.clone(),
            final_seq,
        };
        writer.require(closed, &observed)?;
        writer.sync()?;
        drop(writer);
        run.goal = Goal::Accepted(key.clone());
        Ok(key)
    }

    fn record_launch(
        &self,
        run: &mut CaptureRun,
        request: &CaptureLaunchRequest,
        harness_build: Option<Box<goalcup_protocol::capture::HarnessBuildV1>>,
    ) -> Result<(), DaemonError> {
        run.refuse_if_finished()?;
        match &run.goal {
            Goal::Launched(key) if *key == request.goal_stream => return Ok(()),
            Goal::Accepted(key) if *key == request.goal_stream => {}
            Goal::Accepted(_) | Goal::Launched(_) => {
                return Err(DaemonError::Refused(
                    "the launch names a stream that is not this entry's goal".into(),
                ));
            }
            Goal::Open | Goal::Attempted => {
                return Err(DaemonError::Refused(
                    "this entry has no accepted goal to launch with".into(),
                ));
            }
        }
        let record = CaptureRecord::Launch {
            observed: request.spawned_at.clone(),
            goal_stream: request.goal_stream.clone(),
            profile_id: run.entry.connector_id.clone(),
            profile_digest: run.entry.manifest_sha256.clone(),
            harness_version: request.harness_version.clone(),
            recorder_revision: DAEMON_VERSION.to_string(),
            continuation: request.continuation.clone(),
            // The tracer no longer registers a tool allowance (D52); the v5 fields stay in the
            // record shape so traces that carry them still verify.
            allowed_tool_rule: None,
            process_handoff_digest: None,
            // The key the daemon minted at create, never one the CLI supplied (contract S1).
            run_public_key: Some(run.entry.keys.public_key_hex()?),
            // Forwarded from the row. `create_capture` stores the directory digest carried by
            // the authoritative schema-1 `LoadedConnector`/`ProfileDigests` read (design F3).
            connector_dir_sha256: run
                .entry
                .connector_dir_sha256
                .as_ref()
                .map(|digest| digest.to_ascii_lowercase()),
            harness_build,
        };
        let mut writer = lock(&self.writer);
        let now = (self.clock)();
        writer.require(record, &now)?;
        run.goal = Goal::Launched(request.goal_stream.clone());
        self.bind_launch_scope(request.native_scope.as_deref(), &mut writer, &now);
        Ok(())
    }

    /// Sign the status from the entry's state now. The receiver's own admission counts are
    /// read before any capture lock, which a sink call never waits on.
    fn signed_status(&self, cycles: CycleErrors) -> Result<SignedStatus, DaemonError> {
        let admission = self.ingest.as_ref().map(|ingest| ingest.snapshot());
        let run = self.lock();
        let snapshot = self.snapshot(run.finished(), run.launched());
        sign_capture_status(&run.entry, snapshot, admission, cycles)
    }

    /// The capture projection of the entry now, given its committed finish, if any.
    fn snapshot(&self, finished: Option<&CaptureFinishReport>, launched: bool) -> M2Status {
        let writer = lock(&self.writer);
        let storage = writer.export.capture_storage();
        let sealed = finished.map(|done| done.seal == LocalSealOutcome::SealWritten);
        let capture = if writer.failure.is_some() || self.owner.failed() || sealed == Some(false) {
            CaptureState::Failed
        } else if writer.gaps > 0 || !storage.accounting_available {
            CaptureState::Incomplete
        } else if launched {
            CaptureState::Recording
        } else {
            CaptureState::Starting
        };
        let stopped = finished.is_some_and(|done| {
            matches!(
                done.termination,
                TerminationOutcomeV1::Confirmed { .. }
                    | TerminationOutcomeV1::AlreadyStopped { .. }
            )
        });
        M2Status {
            // A finish whose seal was not written ended the run without sealing it.
            run: match (sealed, stopped, launched) {
                (Some(true), _, _) => RunState::Sealed,
                (_, true, _) => RunState::Exited,
                (_, _, true) => RunState::Running,
                (_, _, false) => RunState::Starting,
            },
            capture,
            delivery: DeliveryState::LocalOnly,
            analysis: AnalysisState::NotRun,
            anchor: AnchorState::LocalOnly,
            storage,
            gap_count: writer.gaps,
            assistance_observed: false,
        }
    }
}

#[cfg(test)]
mod tests;
