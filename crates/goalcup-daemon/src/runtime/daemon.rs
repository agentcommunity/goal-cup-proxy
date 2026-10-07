use super::CycleCounters;
use super::capture::{CaptureEntry, fresh_capture_id};
#[cfg(feature = "v1-compat")]
use super::{Cadence, path_free};
use crate::config::DaemonConfig;
use crate::ingest::{IngestClock, IngestConfig, IngestListener};
use crate::manifests::TrustedManifests;
use crate::registry::Registry;
use crate::{DaemonError, now_unix};
use goalcup_platform::tracecap::ArtifactRoot;
use goalcup_protocol::control::{CycleErrors, EntrySummary, SignedStatus};
use goalcup_protocol::manifest::ConnectorManifest;
use goalcup_protocol::process::ProcessRegistration;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

#[cfg(all(unix, any(test, feature = "test-hooks")))]
static CONFIGURED_ROOT_REPLACEMENT_FOR_TEST: OnceLock<
    Mutex<Option<(std::path::PathBuf, std::path::PathBuf)>>,
> = OnceLock::new();

// What only the V1 runtime reads (D42): compiled with `v1-compat` alone.
#[cfg(feature = "v1-compat")]
use super::entry::RecoveryTailerCutover;
#[cfg(feature = "v1-compat")]
use super::seal::{
    SealedDrain, end_harness_verified, reap_sealed_harnesses, recover_sealing_entries_for_config,
    validate_sealed_drains_for_config,
};
#[cfg(feature = "v1-compat")]
use super::{EntryRuntime, SealReport, held_elsewhere};
#[cfg(feature = "v1-compat")]
use crate::arming::GateState;
#[cfg(feature = "v1-compat")]
use crate::foreign::ForeignScanPlan;
#[cfg(feature = "v1-compat")]
use crate::paths::PathVars;
#[cfg(feature = "v1-compat")]
use crate::registry::{Entry, EntryState};
#[cfg(feature = "v1-compat")]
use crate::status;
#[cfg(feature = "v1-compat")]
use crate::ui::UiState;
#[cfg(feature = "v1-compat")]
use goalcup_protocol::control::{ResumeInfo, ResumePreviewInfo};
#[cfg(feature = "v1-compat")]
use std::collections::BTreeSet;
#[cfg(feature = "v1-compat")]
use std::path::PathBuf;
#[cfg(all(feature = "v1-compat", any(test, feature = "test-hooks")))]
use std::sync::Condvar;

/// The most spawned units of an entry's own work — a blocking seal, a start binding — that
/// may be in flight at once, and so the number of permits [`Daemon::stop`] has to take back
/// before it can say the daemon is idle.
///
/// Both kinds hold an `Arc<Daemon>` and neither is abortable in any useful sense. A seal
/// walks the workspace, writes the chain and then sleeps out `grace_seconds` on a blocking
/// worker; a binding is mid-receipt, about to write `receipt.json` and a `started_at`. A
/// `stop` that returned over either would release nothing — the redb registry and buffer locks
/// would outlive it, so a restart in place would fail to open a database it had been told was free
/// — and would leave a tree half-killed or a start half-recorded.
///
/// 64 is far beyond any real daemon: one machine holds a handful of runs, and each of them
/// has at most one seal and at most one binding.
pub(super) const MAX_IN_FLIGHT: usize = 64;

/// What `{receiver_http}` and `receiver_http` answer: nothing, because the daemon runs no
/// hook receiver. The template variable and the protocol fields remain; an empty value is
/// the honest one.
#[cfg(feature = "v1-compat")]
pub(crate) const NO_RECEIVER: &str = "";

/// One entry's rehydration failure, said in words that name the entry.
///
/// The counters keep a count and one message ([`CycleCounters::snapshot`]), and a message
/// that did not say which run is no longer being captured would be a number a participant
/// cannot act on. `path_free` runs over it on the way into the counter, so the reason is
/// carried without the path it happened at.
#[cfg(feature = "v1-compat")]
fn not_rehydrated(entry_id: &str, error: &DaemonError) -> DaemonError {
    DaemonError::Missing(format!(
        "entry {entry_id} was not rebuilt at startup and is not being captured by this \
         daemon: {}",
        path_free(error)
    ))
}

/// A post-creation refusal is terminal participant evidence, not a missing runtime. Keep
/// its registry/trace explanation reachable while every mutating route fails closed.
#[cfg(feature = "v1-compat")]
fn terminal_refusal(entry_id: &str) -> DaemonError {
    DaemonError::Refused(format!(
        "entry {entry_id} was terminally refused and remains quarantined: its registry and \
         trace evidence are preserved, but it cannot be armed, resumed, handed a process, \
         sealed, or abandoned"
    ))
}

#[cfg(feature = "v1-compat")]
fn cleanup_pending_refusal(entry_id: &str) -> DaemonError {
    DaemonError::Refused(format!(
        "entry {entry_id} has durable cleanup pending: capture, arming, resume, process \
         handoff and sealing remain closed until abandon cleanup is retried"
    ))
}

// =========================================================================================
// The daemon: one per machine, entries routed by working directory (D4).
// =========================================================================================

/// One entry's runtime, behind its own lock.
///
/// **Per entry, not per daemon.** Two entries in one daemon share the registry and nothing
/// else: separate tailers, separate chains, separate buffers, separate gates. A single lock
/// over the map would also make one entry's 64-pass drain block the other's.
///
/// The runtime is an `Option` because some of the daemon's paths are `async` and the
/// runtime is not `Sync`: [`Daemon::pump_all`] and the start binding take it out, `await`,
/// and put it back, which is the reason a `std::sync::MutexGuard` is never held across an
/// `await` point in this file.
#[cfg(feature = "v1-compat")]
pub(super) struct EntrySlot {
    pub(super) runtime: Mutex<Option<EntryRuntime>>,
}

#[cfg(feature = "v1-compat")]
impl EntrySlot {
    /// Put the runtime back. Every return goes through here.
    fn return_runtime(&self, runtime: EntryRuntime) {
        *self.runtime.lock().unwrap_or_else(|e| e.into_inner()) = Some(runtime);
    }
}

/// One terminal row's upload-only recovery owner.
///
/// Terminal rows are deliberately not rebuilt as [`EntryRuntime`]s on restart: no tailer
/// may reopen after `EventBody::Seal`.  They still need the same one-at-a-time ownership
/// as a live runtime while their buffered terminal link is retried, otherwise two upload
/// cadence calls could sign and append competing acknowledgements for the same range.
#[cfg(feature = "v1-compat")]
pub(super) struct SealedDrainSlot {
    drain: Mutex<Option<SealedDrain>>,
}

/// The post-uploader terminal decision. Only a completed final validation is allowed to
/// publish an ordinary sealed status or release the retained owner. A durable quarantine gets
/// its signed review; an error before that persistence is held unavailable in this process.
#[cfg(feature = "v1-compat")]
enum RecoveredFinalValidation {
    Clean(bool),
    Quarantined,
    PersistFailed,
}

/// Deterministic test synchronization for the interval after the daemon has made a failed
/// terminal classification unavailable but before it returns the exclusive owner to its
/// slot.  Shipped builds contain no pause or scheduling behaviour.
#[cfg(feature = "v1-compat")]
#[cfg(any(test, feature = "test-hooks"))]
#[derive(Default)]
struct TerminalLatchPause {
    state: Mutex<TerminalLatchPauseState>,
    wake: Condvar,
}

#[cfg(feature = "v1-compat")]
#[cfg(any(test, feature = "test-hooks"))]
#[derive(Default)]
struct TerminalLatchPauseState {
    entry_id: Option<String>,
    paused: bool,
    released: bool,
}

#[cfg(feature = "v1-compat")]
struct BorrowedSealedDrain {
    slot: Arc<SealedDrainSlot>,
    drain: Option<SealedDrain>,
}

#[cfg(feature = "v1-compat")]
impl Drop for BorrowedSealedDrain {
    fn drop(&mut self) {
        if let Some(drain) = self.drain.take() {
            *self.slot.drain.lock().unwrap_or_else(|e| e.into_inner()) = Some(drain);
        }
    }
}

#[cfg(feature = "v1-compat")]
impl BorrowedSealedDrain {
    fn get(&mut self) -> Result<&mut SealedDrain, DaemonError> {
        self.drain.as_mut().ok_or_else(|| {
            DaemonError::Missing("the borrowed sealed drain was already returned".to_string())
        })
    }
}

/// A runtime borrowed out of the map for the length of one asynchronous call, returned to
/// its slot when the borrow ends — including when the future holding it is dropped.
///
/// Without the `Drop`, a cancelled `pump` would take the entry's runtime with it and every
/// later call on that entry would answer "mid-upload" for the rest of the process.
#[cfg(feature = "v1-compat")]
pub struct Borrowed {
    slot: Arc<EntrySlot>,
    /// Always `Some` until [`Borrowed::drop`] takes it back out.
    runtime: Option<EntryRuntime>,
}

#[cfg(feature = "v1-compat")]
impl Drop for Borrowed {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            self.slot.return_runtime(runtime);
        }
    }
}

#[cfg(feature = "v1-compat")]
impl Borrowed {
    /// The runtime itself.
    ///
    /// A `Result` and not a `Deref`: the field is `Option` so that `drop` can move the
    /// runtime back, `Deref` would have to answer with a reference it may not have, and the
    /// only ways to write that are a panic and a lie.
    pub fn get(&mut self) -> Result<&mut EntryRuntime, DaemonError> {
        self.runtime.as_mut().ok_or_else(|| {
            DaemonError::Missing("the borrowed runtime was already returned".to_string())
        })
    }

    /// Return the runtime while the caller still holds a surrounding admission latch.
    /// `Drop` remains the cancellation fallback; this explicit form closes the interval
    /// between draining the post-cutoff queue and making its owner available again.
    pub(super) fn return_to_slot(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            self.slot.return_runtime(runtime);
        }
    }
}

/// Takes one entry id back out of [`DaemonInner::binding`] however its task ended.
///
/// Clearing the id on the last line of the task is a line a panic and a cancellation
/// both skip — and [`Daemon::poll_pass`] steps over an entry whose id is in that set, so
/// one unwind would stop a run being polled for the life of the daemon while nothing was
/// binding it. A guard runs on every exit there is.
#[cfg(feature = "v1-compat")]
struct BindingGuard {
    daemon: Daemon,
    entry_id: String,
}

#[cfg(feature = "v1-compat")]
impl Drop for BindingGuard {
    fn drop(&mut self) {
        self.daemon.binding().remove(&self.entry_id);
    }
}

/// Everything the daemon owns, behind one `Arc` so the control router can share it.
pub(super) struct DaemonInner {
    pub(super) config: DaemonConfig,
    pub(super) registry: Arc<Registry>,
    /// Acquired once before the registry and retained for every later entry operation.
    /// `config.artifact_root` is display/validation data after this capability exists.
    pub(super) artifact_root: ArtifactRoot,
    /// The shipped connector manifests, by declared id. Behind a lock rather than an
    /// immutable `Arc`: `create_workspace` re-reads the connector
    /// directory, so a manifest edited on disk is seen without restarting the daemon.
    manifests: Mutex<Arc<TrustedManifests>>,
    /// entry id → its runtime.
    #[cfg(feature = "v1-compat")]
    entries: Mutex<BTreeMap<String, Arc<EntrySlot>>>,
    /// entry id → its v5 local capture. Never an entry of `entries`.
    pub(super) captures: Mutex<BTreeMap<String, Arc<CaptureEntry>>>,
    /// The loopback capture ingest listener, bound once at start; [`Daemon::stop`] takes it
    /// and shuts it down.
    pub(super) ingest: Mutex<Option<IngestListener>>,
    pub(super) ingest_addr: SocketAddr,
    /// The daemon's one observation clock for capture records and ingest outcomes.
    pub(super) capture_clock: IngestClock,
    /// Sealed rows evicted by a restart but not yet acknowledged.  This retains only the
    /// upload path and its retry state; the row remains the durable authority.
    #[cfg(feature = "v1-compat")]
    sealed_drains: Mutex<BTreeMap<String, Arc<SealedDrainSlot>>>,
    /// A post-pump terminal validation whose durable quarantine write failed. Until a later
    /// cadence validates or persists review, this process must withhold every stale status
    /// view instead of treating the pre-failure `Sealed` document as an all-clear.
    #[cfg(feature = "v1-compat")]
    withheld_terminal_status: Mutex<BTreeSet<String>>,
    /// Unsealed rows this process could not rebuild. Their prior on-disk status describes
    /// capture by an earlier daemon and must not be served as if this daemon were recording.
    #[cfg(feature = "v1-compat")]
    withheld_rehydration_status: Mutex<BTreeSet<String>>,
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    terminal_latch_pause: TerminalLatchPause,
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) workspace_finalization_fault: Mutex<Option<crate::registry::FinalizationPhaseV1>>,
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fail_after_workspace_capture: std::sync::atomic::AtomicBool,
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fail_after_workspace_cutoff_append: std::sync::atomic::AtomicBool,
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fail_after_workspace_finalization_append: std::sync::atomic::AtomicBool,
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fail_after_workspace_status_publish: std::sync::atomic::AtomicBool,
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) finalization_clock: Mutex<Option<f64>>,
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) finalization_clock_steps: Mutex<std::collections::VecDeque<f64>>,
    /// Process-local monotone wall-time authority for deadline/finalization decisions.
    /// `deadline_observed` carries the threshold latch across restarts; this high-water
    /// prevents a wall-clock rollback within one process from moving any phase backwards.
    pub(super) finalization_epoch_unix: f64,
    pub(super) finalization_high_water: Mutex<f64>,
    /// D4's routing key: the canonical workspace → the entry bound to it. One entry per
    /// workspace, so a directory reaches one run or none.
    #[cfg(feature = "v1-compat")]
    workspaces: Mutex<BTreeMap<PathBuf, String>>,
    #[cfg(feature = "v1-compat")]
    pub(super) seals: Mutex<BTreeMap<String, SealReport>>,
    /// What the four cadences lost, shared with every registered entry so the signed
    /// status can carry it.
    pub(super) cycles: Arc<CycleCounters>,
    #[cfg(feature = "v1-compat")]
    ui: UiState,
    /// Set once, after the listener bound: the OS chooses the port when the configured one
    /// is `0`, and nothing may read an address the daemon only asked for.
    control_addr: OnceLock<SocketAddr>,
    /// The control listener's task, held so [`Daemon::stop`] can take it down. Dropping a
    /// `JoinHandle` detaches its task, which is what the running daemon wants and what leaves
    /// a stopped one serving a port it no longer owns.
    listeners: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    /// One permit per in-flight blocking seal ([`Daemon::seal_entry_blocking`]) and per
    /// spawned start binding ([`Daemon::poll_all`]). [`Daemon::stop`] drains it by taking
    /// every permit, which it can only do once each of them has dropped its own. A
    /// semaphore rather than a `JoinSet` because the handler that started a seal is the one
    /// awaiting its result, and a `JoinHandle` can be awaited from exactly one place.
    pub(super) blocking: Arc<tokio::sync::Semaphore>,
    /// Entries whose start binding is on a task of its own right now, so
    /// [`Daemon::poll_all`] steps over them instead of meeting `held_elsewhere` four times
    /// a second for the length of a receipt round trip — and so one entry never has two
    /// receipt requests in flight.
    #[cfg(feature = "v1-compat")]
    binding: Mutex<BTreeSet<String>>,
    /// This daemon's admission boundary, held by the finalizer and by `register`. See
    /// [`super::AdmissionLatch`] for why it is per daemon and not a `static`.
    #[cfg(feature = "v1-compat")]
    admission: super::AdmissionLatch,
    /// Test-only: make the next spawned start binding panic, so [`BindingGuard`] is
    /// asserted rather than argued. See [`Daemon::panic_next_binding`].
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    panic_next_binding: std::sync::atomic::AtomicBool,
    /// Test-only: skip the one `note_seal_outcome` call [`Daemon::seal_entry`] makes, as if
    /// a concurrent pump had evicted the drained entry between the registry write and that
    /// call. See [`Daemon::skip_next_note_seal_outcome`].
    ///
    /// `pub(super)` and not private: `Daemon::seal_entry`'s read of it lives in
    /// `src/runtime/seal.rs`, a sibling module of this one.
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) skip_next_note_seal_outcome: std::sync::atomic::AtomicBool,
    pub(super) started: Instant,
}

/// The process-level daemon: the entry map, the loopback listener, the tick, and the
/// seal both endings share.
///
/// # One daemon per machine (D4)
///
/// The registry is opened once, at [`Registry::default_path`] under the artifact root, and
/// every entry's runtime is handed that one handle: redb takes a file lock, so a second
/// `Registry::open` on the same path is a failure and not a second handle. That is also why
/// [`Daemon::registry`] exists — a caller building an [`EntryRuntime`] to [`Daemon::register`]
/// has to be given the daemon's own.
///
/// # The listener is loopback and nothing else
///
/// `DaemonConfig::validate` refuses a non-loopback `control_addr`. Three control routes are
/// unauthenticated (`control::is_public_route`) and what makes that safe is that the socket is not
/// reachable off this machine.
///
/// # Sealing is one code path
///
/// The participant's early finish and the automatic seal at `deadline_at` are the same
/// call — [`Daemon::seal_entry`] — in the same order: the workspace manifest, the second
/// foreign scan, the chained `EventBody::Seal` and `seal.json`, and only then the kill.
pub struct Daemon {
    pub(super) inner: Arc<DaemonInner>,
}

impl Daemon {
    /// Test-only: replace this exact configured root with a symlink immediately after its
    /// successful `mkdir` and before private-mode normalization.
    #[cfg(all(unix, any(test, feature = "test-hooks")))]
    pub fn replace_new_configured_root_with_symlink_for_test(
        path: &std::path::Path,
        target: &std::path::Path,
    ) {
        *CONFIGURED_ROOT_REPLACEMENT_FOR_TEST
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) =
            Some((path.to_path_buf(), target.to_path_buf()));
    }

    /// Whether this process owns a runtime slot for the entry, without trying to acquire
    /// that slot. Exact Registered replay is a registry acknowledgement and must not turn
    /// a concurrent pump hold into an ambiguous response.
    #[cfg(feature = "v1-compat")]
    pub(crate) fn has_entry_runtime(&self, entry_id: &str) -> bool {
        self.has_capture_entry(entry_id)
            || self
                .inner
                .entries
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .contains_key(entry_id)
    }

    /// A default build holds no V1 runtime (D42): only a capture entry owns a slot.
    #[cfg(not(feature = "v1-compat"))]
    pub(crate) fn has_entry_runtime(&self, entry_id: &str) -> bool {
        self.has_capture_entry(entry_id)
    }

    /// Open the registry, sweep what a crash left behind, load the shipped manifests, bind
    /// the listener and put every unsealed run back into the entry map.
    ///
    /// A new entry arrives through [`Daemon::register`], which is what the control API's
    /// `POST /v0/workspaces` calls after `workspace::prepare`. An entry that was already
    /// running when the previous daemon stopped arrives through [`Daemon::rehydrate`],
    /// below, and by the same door: it builds the [`EntryRuntime`] and hands it to
    /// `register`, so the chain, the harness home and the first signed status are opened by
    /// exactly one method whichever way an entry got here.
    pub async fn start(config: DaemonConfig) -> Result<Daemon, DaemonError> {
        config.validate()?;
        create_configured_root(&config.artifact_root, "artifact_root")?;
        create_configured_root(&config.config_dir, "config_dir")?;
        let artifact_root = ArtifactRoot::open(&config.artifact_root).map_err(|error| {
            DaemonError::Refused(format!(
                "selected artifact root cannot be acquired without following a link or reparse point: {error}"
            ))
        })?;

        // **The registry first, because opening it is what makes this the only daemon.**
        // redb takes a file lock and `Registry::open` names the refusal, so a second daemon
        // on this machine stops here — before it has swept another daemon's live temp
        // files or reaped another daemon's harnesses.
        let registry = Arc::new(Registry::open_artifact(&artifact_root)?);
        // Service-free v5 startup cannot safely recover a legacy row: its runtime and
        // sealed drain construct the existing uploader/receipt clients. Refuse the whole
        // installation before cache sweeping, cleanup or rehydration changes its evidence.
        // A mixed registry requires restoration of its existing service pair too.
        #[cfg(feature = "v1-compat")]
        if !config.has_service() && registry.all()?.iter().any(|entry| entry.capture.is_none()) {
            config.require_service()?;
        }
        // D42: a default build has no V1 runtime. A V1 row that a running daemon would still owe
        // work to (an unsealed run, an unfinished drain, a harness still to reap) cannot be served
        // here, so the whole registry is refused before anything is swept or changed. A sealed,
        // fully acknowledged V1 row is inert evidence that this build never touches, and does not
        // block the capture product.
        #[cfg(not(feature = "v1-compat"))]
        if let Some(owing) = registry
            .all()?
            .iter()
            .find(|entry| entry.capture.is_none() && v1_row_owes_work(entry))
        {
            return Err(DaemonError::Refused(format!(
                "the registry holds a V1 (transcript-path) entry, {}, that is not finished: it is \
                 {:?} and this build does not include the V1 runtime that would drain and seal it. \
                 Finish it with a daemon built with the `v1-compat` cargo feature on this artifact \
                 root (once it is sealed and fully acknowledged it no longer blocks this daemon), \
                 or give the capture product its own `artifact_root` and `config_dir`",
                owing.entry_id, owing.state
            )));
        }
        // A status write is a temp file and a rename; a daemon killed between the two
        // leaves the temp file. Nothing else ever removes it, and the next `read_dir` of
        // the cache directory trips over a document that verifies as nothing.
        sweep_stale_temp_files(&config.config_dir.join("cache"))?;

        let manifests = Arc::new(TrustedManifests::load(&config.connectors_dir)?);
        let capture_clock = crate::ingest::monotonic_clock(fresh_capture_id());
        let ingest =
            IngestListener::bind_loopback(loopback(0), IngestConfig::new(capture_clock.clone()))
                .map_err(|error| {
                    DaemonError::Refused(format!(
                        "the capture ingest listener cannot bind: {error}"
                    ))
                })?;
        let ingest_addr = ingest.local_addr();
        let inner = Arc::new(DaemonInner {
            registry: registry.clone(),
            artifact_root,
            manifests: Mutex::new(manifests.clone()),
            #[cfg(feature = "v1-compat")]
            entries: Mutex::new(BTreeMap::new()),
            captures: Mutex::new(BTreeMap::new()),
            ingest: Mutex::new(Some(ingest)),
            ingest_addr,
            capture_clock,
            #[cfg(feature = "v1-compat")]
            sealed_drains: Mutex::new(BTreeMap::new()),
            #[cfg(feature = "v1-compat")]
            withheld_terminal_status: Mutex::new(BTreeSet::new()),
            #[cfg(feature = "v1-compat")]
            withheld_rehydration_status: Mutex::new(BTreeSet::new()),
            #[cfg(feature = "v1-compat")]
            #[cfg(any(test, feature = "test-hooks"))]
            terminal_latch_pause: TerminalLatchPause::default(),
            #[cfg(feature = "v1-compat")]
            #[cfg(any(test, feature = "test-hooks"))]
            workspace_finalization_fault: Mutex::new(None),
            #[cfg(feature = "v1-compat")]
            #[cfg(any(test, feature = "test-hooks"))]
            fail_after_workspace_capture: std::sync::atomic::AtomicBool::new(false),
            #[cfg(feature = "v1-compat")]
            #[cfg(any(test, feature = "test-hooks"))]
            fail_after_workspace_cutoff_append: std::sync::atomic::AtomicBool::new(false),
            #[cfg(feature = "v1-compat")]
            #[cfg(any(test, feature = "test-hooks"))]
            fail_after_workspace_finalization_append: std::sync::atomic::AtomicBool::new(false),
            #[cfg(feature = "v1-compat")]
            #[cfg(any(test, feature = "test-hooks"))]
            fail_after_workspace_status_publish: std::sync::atomic::AtomicBool::new(false),
            #[cfg(any(test, feature = "test-hooks"))]
            finalization_clock: Mutex::new(None),
            #[cfg(any(test, feature = "test-hooks"))]
            finalization_clock_steps: Mutex::new(std::collections::VecDeque::new()),
            finalization_epoch_unix: now_unix(),
            finalization_high_water: Mutex::new(0.0),
            #[cfg(feature = "v1-compat")]
            workspaces: Mutex::new(BTreeMap::new()),
            #[cfg(feature = "v1-compat")]
            seals: Mutex::new(BTreeMap::new()),
            cycles: Arc::new(CycleCounters::default()),
            #[cfg(feature = "v1-compat")]
            ui: UiState {
                cache: Arc::new(Mutex::new(BTreeMap::new())),
                current: Arc::new(Mutex::new(None)),
            },
            control_addr: OnceLock::new(),
            listeners: Mutex::new(Vec::new()),
            blocking: Arc::new(tokio::sync::Semaphore::new(MAX_IN_FLIGHT)),
            #[cfg(feature = "v1-compat")]
            binding: Mutex::new(BTreeSet::new()),
            #[cfg(feature = "v1-compat")]
            admission: super::AdmissionLatch::default(),
            #[cfg(feature = "v1-compat")]
            #[cfg(any(test, feature = "test-hooks"))]
            panic_next_binding: std::sync::atomic::AtomicBool::new(false),
            #[cfg(feature = "v1-compat")]
            #[cfg(any(test, feature = "test-hooks"))]
            skip_next_note_seal_outcome: std::sync::atomic::AtomicBool::new(false),
            started: Instant::now(),
            config,
        });

        let daemon = Daemon {
            inner: inner.clone(),
        };

        // The V1 startup recoveries (D42): abandon cleanup, sealing intents, sealed drains, the
        // harness reaper and the capture-closed status republish. A default build has no V1
        // rows (`start` refused them above), so it has nothing for these to recover.
        #[cfg(feature = "v1-compat")]
        {
            // Abandon cleanup is a durable transaction boundary. A predecessor may have
            // removed any subset of the artifact tree before its final unlink failed, so these
            // rows are completed before status publication or ordinary runtime rehydration can
            // mistake them for live entries. One failure belongs to one quarantined row and
            // does not prevent unrelated entries from starting.
            daemon.recover_pending_cleanups();

            // Seal crosses chain/buffer and registry stores. One corrupt intent must not take
            // down unrelated entries: each failure stays durably `Sealing` (capture-closed) and
            // is counted like every other per-entry startup failure. Only successfully promoted
            // rows reach the reaper.
            for (entry_id, error) in recover_sealing_entries_for_config(
                &daemon,
                &registry,
                &inner.config,
                &inner.artifact_root,
            )? {
                inner
                    .cycles
                    .note(Cadence::Tick, &not_rehydrated(&entry_id, &error));
            }
            // A durable `last_ack_seq == chain_seq` is not permission to skip the terminal
            // buffer.  Buffer::ack commits before its registry mirror, and a corrupt or swapped
            // floor beyond the terminal must be caught before this daemon starts serving a row
            // whose cache says it is safely complete.  Per-entry failure is quarantined like a
            // broken sealing intent; unrelated entries still start.
            let mut startup_terminal_failures = Vec::new();
            for (entry_id, error, persistence_failed) in
                validate_sealed_drains_for_config(&registry, &inner.config, &inner.artifact_root)?
            {
                inner.cycles.note(
                    Cadence::Upload,
                    &DaemonError::Refused(format!(
                        "sealed entry {entry_id} did not pass startup drain validation: {}",
                        path_free(&error)
                    )),
                );
                if persistence_failed {
                    // `validate_sealed_drains_for_config` has already best-effort invalidated
                    // the exact raw views while it still had the row/config roots. Keep this
                    // new daemon's API unavailable until a later recovery validation persists
                    // review or publishes a freshly verified clean completion.
                    if let Err(withhold_error) =
                        daemon.withhold_terminal_status_after_failed_validation(&entry_id)
                    {
                        inner
                            .cycles
                            .note(Cadence::Status, &not_rehydrated(&entry_id, &withhold_error));
                    }
                } else {
                    startup_terminal_failures.push(entry_id);
                }
            }
            // Startup validation is itself a terminal decision. Publish its durable review
            // (or install/invalidate its transient withholding) before any later reap or
            // rehydration step can fail and abort bootstrap over an old raw all-clear view.
            for entry_id in startup_terminal_failures {
                if let Err(status_error) = daemon.publish_terminal_review(&entry_id) {
                    inner
                        .cycles
                        .note(Cadence::Status, &not_rehydrated(&entry_id, &status_error));
                }
            }
            reap_sealed_harnesses(&registry, &inner.config)?;
            // A restarted daemon has no live runtime through which to correct the previous
            // cache for a promoted seal, a completed/failed reap, an already-ACKed terminal,
            // or an unreconciled intent.  Publish every durable capture-closed row before any
            // listener starts; this also makes a root-absent reap-pending outcome observable
            // rather than leaving an old Online/countdown document beside it.
            for entry in registry.all()? {
                if !entry.state.capture_closed() {
                    continue;
                }
                if entry.state == EntryState::CleanupPending {
                    continue;
                }
                if daemon.withheld_terminal_status().contains(&entry.entry_id) {
                    continue;
                }
                if entry.terminal_drain_needs_review() {
                    if let Err(status_error) = daemon.publish_terminal_review(&entry.entry_id) {
                        inner.cycles.note(
                            Cadence::Status,
                            &not_rehydrated(&entry.entry_id, &status_error),
                        );
                    }
                    continue;
                }
                match crate::status::compose_capture_closed(&entry, now_unix()) {
                    Ok(signed) => {
                        // Install the signed durable fact before retrying its filesystem views.
                        // In particular, a startup validation quarantine may not fall back to a
                        // stale `Sealed` all-clear merely because a cache/status-line write is
                        // temporarily unavailable.
                        daemon
                            .cache()?
                            .insert(entry.entry_id.clone(), signed.clone());
                        if let Err(status_error) =
                            crate::status::mirror(&signed, &inner.config.config_dir).and_then(|_| {
                                inner.artifact_root.open_entry(&entry.entry_id)
                                .map_err(|error| {
                                    DaemonError::Refused(format!(
                                        "startup status line cannot acquire entry artifact authority through the retained selected root: {error}"
                                    ))
                                })
                                .and_then(|artifact| {
                                    crate::status::write_entry_status_line_rooted(&signed, &artifact)
                                })
                            })
                        {
                            inner.cycles.note(
                                Cadence::Status,
                                &not_rehydrated(&entry.entry_id, &status_error),
                            );
                        }
                    }
                    Err(status_error) => inner.cycles.note(
                        Cadence::Status,
                        &not_rehydrated(&entry.entry_id, &status_error),
                    ),
                }
            }
        }

        // **Every run the previous daemon was capturing, back in the map first** — before
        // the control listener, because the first thing a CLI that finds `control.json` asks
        // is what this daemon is holding.
        #[cfg(feature = "v1-compat")]
        {
            daemon.close_overdue_workspace_entries_before_listeners()?;
            daemon.rehydrate();
        }
        daemon.reopen_captures();

        // The control listener, in three parts. The bearer is minted before the
        // router, because the auth layer holds it; `control.json` is written after the
        // bind, because the port is the OS's to choose when the configured one is `0` and
        // a file naming a port nothing listens on is worse than no file. Nothing between
        // the two can observe the token: the router is built and served in this function.
        let token = crate::control::mint_control_token();
        let state = crate::control::ControlState {
            daemon: Arc::new(Daemon {
                inner: inner.clone(),
            }),
            token: Arc::new(token.clone()),
            started_at: inner.started,
        };
        #[cfg(feature = "v1-compat")]
        let control = crate::control::control_router(state, inner.ui.clone());
        #[cfg(not(feature = "v1-compat"))]
        let control = crate::control::control_router(state);
        let control_addr = inner.config.control_addr;
        let listener = crate::control::serve_control(control, control_addr).await?;
        let bound = listener.addr();
        let _ = inner.control_addr.set(bound);
        // A `control.json.<pid>.tmp` a killed daemon left behind is removed once, here,
        // because the temp name carries the pid and the write's own `O_EXCL` would
        // otherwise refuse this start on a week-old file.
        crate::control::sweep_stale_control_temp(&inner.config.config_dir);
        // And if the file cannot be written, the listener comes back down before this
        // returns. `token` is a local of this function: a control listener left serving
        // after the failure would be an authenticated port whose bearer exists nowhere on
        // the machine once the frame is gone — unreachable by the CLI, unnoticed by the
        // operator, and up for the life of the process.
        if let Err(e) =
            crate::control::write_control_file(&inner.config.config_dir, bound.port(), &token)
        {
            listener.abort().await;
            return Err(e);
        }
        // The task is the daemon's from here, so [`Daemon::stop`] has something to take
        // down. Nothing else reads it.
        inner
            .listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(listener.into_task());

        Ok(Daemon { inner })
    }

    /// Bring this daemon out of service: the listener down, and every file lock it holds
    /// released.
    ///
    /// **What it is for.** The registry and every entry's buffer are redb databases, and
    /// redb takes a file lock — that is what makes one daemon per machine a fact rather
    /// than a convention (D4). A second [`Daemon::start`] over the same artifact root is
    /// therefore refused until the first daemon's handles are gone, and a `Daemon` value
    /// that is merely dropped does not release them: the control router holds an `Arc` of
    /// this one, and dropping a `JoinHandle` detaches its task rather than ending it. So
    /// the listener is aborted and awaited — awaited, because `abort` only marks a task
    /// and the socket is closed when the task is dropped — and only then is `self` gone,
    /// which is the last `Arc` and so the moment the locks are released.
    ///
    /// By value, because a stopped daemon has nothing left to answer with: the port is
    /// closed and the registry handle is released, and a caller holding one afterwards
    /// would be holding an object every method on which is about to fail.
    ///
    /// Entries are not sealed by this. A stop is the process ending, which is exactly the
    /// case `EventBody::DaemonRestart` and [`Daemon::rehydrate`] exist for; a seal is the
    /// run ending, and there is no second last link.
    ///
    /// **A seal already running is awaited.** It is not on `listeners` and it is
    /// not abortable: `tokio::task::spawn_blocking` runs a synchronous closure, and by the
    /// time a stop arrives that closure may be mid-`terminate`, still to write `seal.json`
    /// and the registry row, and still holding the `Arc<Daemon>` that owns the redb locks.
    /// A `stop` that returned over it would leave a restart-in-place unable to open the registry
    /// it had just been told was released — and a tree half-killed with a seal that
    /// never finished recording what it did.
    pub async fn stop(self) {
        let tasks: Vec<_> = std::mem::take(
            &mut *self
                .inner
                .listeners
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        for task in tasks {
            task.abort();
            let _ = task.await;
        }
        let ingest = self
            .inner
            .ingest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(listener) = ingest {
            listener
                .shutdown(tokio::time::Instant::now() + crate::ingest::DROP_SHUTDOWN_GRACE)
                .await;
        }
        self.inner
            .captures
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        // The listener is down before this, so no new request can start a seal; what is left
        // is what was already running. Every
        // permit back means every blocking seal has dropped its own, which happens when
        // the closure ends — including when it unwinds.
        let _ = self.inner.blocking.acquire_many(MAX_IN_FLIGHT as u32).await;
    }

    /// Put every unsealed registry row back into the entry map.
    ///
    /// **The rule.** A row that is not sealed is a run this machine is still capturing. Its
    /// [`EntryRuntime`] is rebuilt and handed to [`Daemon::register`], which opens the chain
    /// through [`EntryRuntime::open_trace`] — the retained prepared writer off the validated
    /// chain tail, `EventBody::DaemonRestart` before any tailer polls, a `Reread` witness for
    /// every store whose saved position did not come back, and the descriptor rewritten —
    /// so the deadline seal, the tailing, the status, `resume` and `seal` all carry on as if
    /// this process had been the one that started them.
    ///
    /// **A row that cannot be rebuilt is skipped and counted, never sealed.** A missing
    /// trace directory, a chain the export will not read, a connector no longer shipped: all
    /// of them are one entry's failure and none of them is a reason to close a run's chain.
    /// A seal is the last link and the daemon that writes one has to have walked the
    /// workspace to do it; a startup that sealed on a read error would be publishing a
    /// manifest for a run it could not open. The failure reaches the participant the way
    /// every other one this crate cannot log does — as a cycle error on the signed status
    /// document, naming the entry.
    ///
    /// Sealed rows are left alone. Their harnesses are [`reap_sealed_harnesses`]'s, which
    /// ran before this and under the registry lock.
    ///
    /// Infallible: every failure
    /// here belongs to one entry, and a daemon that refused to start because one of five
    /// runs had a bad chain would take the other four down with it.
    #[cfg(feature = "v1-compat")]
    fn rehydrate(&self) {
        let rows = match self.inner.registry.all() {
            Ok(rows) => rows,
            Err(e) => return self.inner.cycles.note(Cadence::Tick, &e),
        };
        for entry in rows {
            if entry.state.capture_closed() || entry.sealed_at.is_some() || entry.capture.is_some()
            {
                continue;
            }
            let entry_id = entry.entry_id.clone();
            if self
                .inner
                .withheld_rehydration_status
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .contains(&entry_id)
            {
                continue;
            }
            if let Err(e) = self.rebuild(entry).and_then(|r| self.register(r)) {
                self.inner
                    .withheld_rehydration_status
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .insert(entry_id.clone());
                self.inner
                    .cycles
                    .note(Cadence::Tick, &not_rehydrated(&entry_id, &e));
            }
        }
    }

    /// Close every V4 deadline that elapsed during downtime before a tailer can append a
    /// restart/read record and before the listener can admit work. The retained writer
    /// is opened without a post-deadline restart envelope, and submission recovery is
    /// source-free; ambiguous downtime bytes therefore cannot be backdated into the entry.
    #[cfg(feature = "v1-compat")]
    fn close_overdue_workspace_entries_before_listeners(&self) -> Result<(), DaemonError> {
        let now = self.finalization_now_unix();
        for entry in self.inner.registry.all()? {
            if entry.state != EntryState::Started
                || !matches!(
                    entry.receipt_binding(),
                    crate::config::ReceiptBindingPolicy::WorkspaceSubmission { .. }
                )
            {
                continue;
            }
            let entry_id = entry.entry_id.clone();
            let result = (|| {
                let observed_entry = self
                    .inner
                    .registry
                    .observe_signed_deadline(&entry_id, now)?;
                if !observed_entry.deadline_was_observed() {
                    return Ok(());
                }
                let mut runtime = self.rebuild(observed_entry)?;
                runtime.open_trace_for_overdue_finalization()?;
                self.finish_overdue_workspace_runtime(&mut runtime)?;
                Ok(())
            })();
            if let Err(error) = result {
                self.inner
                    .withheld_rehydration_status
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .insert(entry_id.clone());
                self.inner
                    .cycles
                    .note(Cadence::Tick, &not_rehydrated(&entry_id, &error));
            }
        }
        Ok(())
    }

    #[cfg(feature = "v1-compat")]
    fn recover_pending_cleanups(&self) {
        let rows = match self.inner.registry.all() {
            Ok(rows) => rows,
            Err(error) => return self.inner.cycles.note(Cadence::Tick, &error),
        };
        for entry in rows {
            if entry.state != EntryState::CleanupPending {
                continue;
            }
            if let Err(error) = self.finish_pending_cleanup(&entry.entry_id) {
                self.inner.cycles.note(
                    Cadence::Tick,
                    &DaemonError::Refused(format!(
                        "entry {} remains in cleanup quarantine after startup retry: {}",
                        entry.entry_id,
                        path_free(&error)
                    )),
                );
            }
        }
    }

    #[cfg(feature = "v1-compat")]
    fn finish_pending_cleanup(&self, entry_id: &str) -> Result<(), DaemonError> {
        let target = self.inner.registry.begin_unstarted_cleanup(entry_id)?;
        if let Some(target) = &target
            && !self
                .inner
                .registry
                .unstarted_termination_complete(entry_id)?
        {
            end_harness_verified(target.handle())?;
            // This write is deliberately between successful termination and recursive
            // artifact cleanup. A crash after it may resume deletion, but never mistakes
            // an already-gone PID for an unverified live process.
            self.inner
                .registry
                .mark_unstarted_termination_complete(entry_id, target)?;
        }
        let artifact = match self.inner.artifact_root.open_entry(entry_id) {
            Ok(artifact) => Some(artifact),
            Err(goalcup_platform::tracecap::TraceCapError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                None
            }
            Err(error) => {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} cleanup could not reacquire its surviving artifact \
                     through the retained selected root: {error}"
                )));
            }
        };
        if let Some(artifact) = artifact {
            artifact.remove_recursively().map_err(|error| {
                DaemonError::Refused(format!(
                    "entry {entry_id} artifact cleanup through its retained authority failed; \
                     the cleanup-pending registry row is retained: {error}"
                ))
            })?;
        }
        self.inner
            .registry
            .complete_unstarted_cleanup(entry_id, target.as_ref())
    }

    /// One row, back as the runtime it was — through the rebuilt-only constructor, with
    /// every identity claim taken from the row rather than from this process's memory.
    ///
    /// The tracker files and allowlist are not on the row, so each is recovered from
    /// something that is durable:
    ///
    /// - the **tracker files**, re-rendered from the shipped templates and the entry's own
    ///   variables by [`crate::workspace::rerender_tracker_files`], which writes nothing.
    ///   They are the config watch's expected bytes and the seal's exclusion union, and the
    ///   union is the one that must not be a subset.
    /// - the **allowlist**, re-hashed by `baseline_allowlist`. These are app-wide files the
    ///   tracker never wrote and never restores; the digests it takes here are today's and
    ///   not the launch's, so a change made to one while this machine had no daemon is not
    ///   reported. That is a real loss and it is the smaller one: the alternative is an
    ///   empty list, which stops watching them for the rest of the run.
    ///
    /// `EntryRuntime::new_rebuilt` then acquires the trace root once and validates the chain,
    /// registry prefix and typed baseline commitment before it opens any mutable store.
    #[cfg(feature = "v1-compat")]
    pub(super) fn rebuild(&self, entry: Entry) -> Result<EntryRuntime, DaemonError> {
        let config = &self.inner.config;
        let trusted = self.trusted_manifests();
        let manifest = trusted
            .manifests()
            .get(&entry.connector_id)
            .cloned()
            .ok_or_else(|| {
                DaemonError::Missing(format!(
                    "connector {} is not among the manifests this daemon shipped, so the \
                     stores to tail and the files to watch have no definition",
                    entry.connector_id
                ))
            })?;
        let harness_home = entry.harness_home.clone().ok_or_else(|| {
            DaemonError::Missing(
                "the row names no harness home, so there is no `{config_dir}` for a store \
                 template to hang off"
                    .to_string(),
            )
        })?;
        // A complete pre-upgrade Required row gets its explicit authority before anything
        // below can render tracker files, open a buffer, or build a runtime. A mixed row
        // refuses in the registry transaction, leaving those other stores untouched.
        let entry = self
            .inner
            .registry
            .materialize_legacy_required_handoff(&entry.entry_id)?;
        let (connector_dir, _) =
            crate::control::shipped_connector(&config.connectors_dir, &entry.connector_id)?;

        let roots = crate::workspace::Roots {
            workspace: entry.workspace.clone(),
            artifact_root: entry.artifact_dir.clone(),
            harness_home: harness_home.clone(),
        };
        let status_line_path =
            status::canonical_status_line_path(&entry.entry_id, &self.inner.config.artifact_root)?;
        let source_date = match entry.source_date() {
            Some(date) => (date[0].clone(), date[1].clone(), date[2].clone()),
            None if !matches!(
                entry.receipt_binding(),
                crate::config::ReceiptBindingPolicy::WorkspaceSubmission { .. }
            ) =>
            {
                crate::control::today()
            }
            None => {
                return Err(DaemonError::Refused(format!(
                    "entry {} has no persisted source date for V4 runtime reconstruction",
                    entry.entry_id
                )));
            }
        };
        let vars = PathVars::new(
            entry.workspace.clone(),
            // The participant's real home, as `prepared_workspace` resolves it: the
            // allowlist and the foreign scan are both about files this daemon did not write.
            crate::control::participant_home(&harness_home),
            harness_home,
            entry.artifact_dir.clone(),
            // **The bound session, not `None`.** A store's `path_template` renders
            // `{session_id}` as `*` when nothing is bound, and a resumed run that widened
            // its own glob would start tailing a session it never started.
            entry.session_id.clone(),
            source_date,
        );
        let installed = crate::workspace::rerender_tracker_files(
            &manifest,
            &connector_dir,
            &crate::control::tracker_vars(
                &roots,
                &entry.keys.receiver_token,
                &entry.entry_id,
                entry.session_id.as_deref().unwrap_or_default(),
                NO_RECEIVER,
                &status_line_path,
                None,
            ),
            &roots,
            false,
        )?;
        let allowlist = crate::workspace::baseline_allowlist(&manifest, &vars)?;
        let artifact = self
            .inner
            .artifact_root
            .open_entry(&entry.entry_id)
            .map_err(|error| {
                DaemonError::Refused(format!(
                    "entry {} artifact root cannot be acquired through the daemon's retained authority: {error}",
                    entry.entry_id
                ))
            })?;
        let mut runtime = EntryRuntime::new_rebuilt_with_artifact(
            entry,
            manifest.clone(),
            config,
            vars,
            installed.files,
            allowlist,
            trusted.plan_for(&manifest),
            self.inner.registry.clone(),
            artifact,
        )?;
        // The round's own dates, exactly as `control::build_entry` sets them on a fresh
        // entry. Unconditional: with no `[event]` section `event_status_for` answers
        // the entry's own timeline, which is the value the runtime already holds. Without
        // this line a rehydrated entry silently drops the configured round and its status
        // document reverts to `fallback` — a restart quietly changing what a signed document
        // says the deadline window is.
        runtime.set_event_status(crate::status::event_status_for(
            runtime.entry(),
            config.event.as_ref(),
        ));
        Ok(runtime)
    }

    #[cfg(feature = "v1-compat")]
    pub(super) fn recovery_tailer_cutover(
        &self,
        entry: &Entry,
        artifact: &goalcup_platform::tracecap::EntryArtifactRoot,
        buffer: &crate::buffer::Buffer,
    ) -> Result<RecoveryTailerCutover, DaemonError> {
        let trusted = self.trusted_manifests();
        let manifest = trusted
            .manifests()
            .get(&entry.connector_id)
            .cloned()
            .ok_or_else(|| {
                DaemonError::Missing(format!(
                    "connector {} is not among the shipped manifests",
                    entry.connector_id
                ))
            })?;
        let (_, manifest_bytes) = crate::control::shipped_connector(
            &self.inner.config.connectors_dir,
            &entry.connector_id,
        )?;
        if crate::workspace::sha256_hex(&manifest_bytes) != entry.manifest_sha256 {
            return Err(DaemonError::Refused(format!(
                "entry {} shipped manifest changed since finalization admission",
                entry.entry_id
            )));
        }
        let harness_home = entry.harness_home.clone().ok_or_else(|| {
            DaemonError::Missing(format!(
                "entry {} has no persisted harness home for finalization tailers",
                entry.entry_id
            ))
        })?;
        let intent = entry.workspace_sealing_intent().ok_or_else(|| {
            DaemonError::Refused(format!(
                "entry {} has no workspace finalization source authority",
                entry.entry_id
            ))
        })?;
        let vars = PathVars::new(
            entry.workspace.clone(),
            crate::control::participant_home(&harness_home),
            harness_home,
            entry.artifact_dir.clone(),
            entry.session_id.clone(),
            (
                intent.source_date[0].clone(),
                intent.source_date[1].clone(),
                intent.source_date[2].clone(),
            ),
        );
        RecoveryTailerCutover::open(entry, manifest, vars, artifact, buffer)
    }

    /// Where the CLI, the page and `goalcup doctor` connect. Loopback, always.
    pub fn control_addr(&self) -> SocketAddr {
        self.inner
            .control_addr
            .get()
            .copied()
            .unwrap_or_else(|| loopback(0))
    }

    /// The one registry handle in this process.
    ///
    /// [`EntryRuntime::new`] takes the registry as an argument precisely because redb
    /// takes a file lock, so a caller that has to build a runtime in order to
    /// [`Daemon::register`] it can only be handed this one.
    pub fn registry(&self) -> Arc<Registry> {
        self.inner.registry.clone()
    }

    pub(crate) fn artifact_root(&self) -> &ArtifactRoot {
        &self.inner.artifact_root
    }

    /// Take an entry into the map, creating its harness home and opening its chain.
    ///
    /// **The harness home is created here** when the workspace preparation did not create
    /// one (it writes only the files a connector declares, and a connector may declare
    /// none under its home). `CLAUDE_CONFIG_DIR` and `CODEX_HOME` are directories the
    /// harness is launched with; one that does not exist is a launch that writes its
    /// session somewhere this daemon is not watching.
    ///
    /// **`open_trace` is called here**, so `EventBody::Baseline` is seq 0 before anything
    /// else this daemon does can append, and the start receipt's `chain_head`
    /// witnesses it.
    #[cfg(feature = "v1-compat")]
    pub fn register(&self, mut runtime: EntryRuntime) -> Result<(), DaemonError> {
        let entry_id = runtime.entry().entry_id.clone();
        let workspace = canonical(&runtime.entry().workspace);
        {
            let entries = self.entry_map()?;
            if entries.contains_key(&entry_id) {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} is already open in this daemon"
                )));
            }
        }
        if let Some(held) = self.workspaces()?.get(&workspace) {
            return Err(DaemonError::Refused(format!(
                "workspace {} is already bound to entry {held}: this daemon routes hook \
                 posts by working directory (D4), and two entries in one directory make \
                 every post ambiguous",
                workspace.display()
            )));
        }
        // **Before any directory is created.** A sealed entry's chain is closed — every
        // `event`, `leg1` and `leg2` on it is `ExportError::Sealed` — so registering
        // one would open a run that cannot record anything, and the refusal has to come
        // before the side effects: `create_dir` on a harness home is a directory left
        // behind on a path this daemon then refuses to serve.
        if runtime.entry().state == EntryState::Refused {
            return Err(terminal_refusal(&entry_id));
        }
        if runtime.entry().state.capture_closed() || runtime.entry().sealed_at.is_some() {
            return Err(DaemonError::Refused(format!(
                "entry {entry_id} is already sealed: its chain's last link is the seal and \
                 every append after one is refused, so there is nothing left for this \
                 daemon to capture"
            )));
        }
        if let Some(home) = runtime.entry().harness_home.clone() {
            create_dir(&home)?;
        }
        runtime.open_trace()?;
        // `open_trace` gives recovery a validated writer and encrypted buffer.  A unique
        // Prepared marker is repaired/promoted here, before the runtime enters the map and
        // therefore before any seal can observe a missing termination target.
        runtime.reconcile_prepared_process_handoff()?;
        // From here on this entry's cadence failures are the daemon's, and its status
        // document carries them.
        runtime.observe_cycles(self.inner.cycles.clone());
        // A document from the moment the entry exists, so `goalcup status` and the page
        // have something signed to read before the first tick.
        let signed = runtime.publish_status(
            &self.inner.config.config_dir,
            &self.inner.config.artifact_root,
            now_unix(),
        )?;
        // The runtime that owns its entry becomes visible under the admission boundary, so a
        // finalizer never observes a half-registered slot.
        let admission = self.admission_guard();
        self.entry_map()?.insert(
            entry_id.clone(),
            Arc::new(EntrySlot {
                runtime: Mutex::new(Some(runtime)),
            }),
        );
        drop(admission);
        self.workspaces()?.insert(workspace, entry_id.clone());
        self.cache()?.insert(entry_id, signed);
        self.refresh_current()
    }

    /// One poll of every open entry, plus the start binding that the first corroborated
    /// record owes.
    ///
    /// **A sealed entry is skipped rather than polled.** Every chaining call on a sealed
    /// entry is `ExportError::Sealed`, so polling one would discover the refusal once
    /// per record; its remaining work is the final [`Daemon::pump_all`], which writes to
    /// `acks.jsonl` and not to the chain.
    ///
    /// **Nothing outside this method stamps a `started_at`.** The control API has no "start"
    /// route: the entry binds on the first submitted prompt, and this is where the daemon
    /// notices that both halves — the declared signal and a leg-1 record after it — are in.
    ///
    /// One entry's failure does not stop the others: the first error is returned and every
    /// remaining entry is still polled.
    #[cfg(feature = "v1-compat")]
    pub async fn tick(&self) -> Result<(), DaemonError> {
        let (binding, mut first) = self.poll_pass();
        // The bindings concurrently, so a receipt server stalling one entry is not also
        // the reason the next entry's start is not witnessed. Each on a task of its own,
        // because `EntryRuntime` is not `Sync` and the borrow that takes it out of its slot
        // has to belong to whoever awaits it.
        let mut bindings = tokio::task::JoinSet::new();
        for (entry_id, session) in binding {
            let daemon = self.clone_handle();
            bindings.spawn(async move { daemon.bind_start(&entry_id, &session).await });
        }
        while let Some(joined) = bindings.join_next().await {
            match joined {
                Ok(Err(e)) => {
                    first.get_or_insert(e);
                }
                Err(e) => {
                    first.get_or_insert(DaemonError::Missing(format!(
                        "a start binding did not run to completion: {e}"
                    )));
                }
                Ok(Ok(())) => {}
            }
        }
        self.finish(Cadence::Tick, first)
    }

    /// [`Daemon::tick`] without the wait: the poll pass, and the start bindings left
    /// running on tasks of their own.
    ///
    /// **This is what the cadence loop calls, and why.** The receipt round trip is a
    /// bounded HTTP call with `NETWORK_TIMEOUT` — ten seconds — on it, and with the receipt
    /// server down `may_start()` stays true, so a `run_forever` awaiting it would pay the full
    /// timeout on every beat: for the length of the arming window (120-300 s, per
    /// connector) nothing would be polled, nothing uploaded, no status published and no
    /// deadline checked. On a machine holding two runs, one unreachable server would stop
    /// the other run's capture entirely.
    ///
    /// Synchronous, so the loop's own future never waits here at all. What a binding
    /// produces still reaches the participant: its failure is counted on the tick cadence
    /// through the same [`CycleCounters`] every entry is handed, so a refusing receipt
    /// server is one count and one path-free message on the signed status document.
    ///
    /// [`Daemon::tick`] keeps the awaiting shape for a caller that drives the daemon step
    /// by step: a test, the round-trip fixture, a CLI that ticks once and reads the result.
    #[cfg(feature = "v1-compat")]
    pub fn poll_all(&self) -> Result<(), DaemonError> {
        let (binding, first) = self.poll_pass();
        for (entry_id, session) in binding {
            self.spawn_bind_start(entry_id, session);
        }
        self.finish(Cadence::Tick, first)
    }

    /// One poll of every open entry, and the entries that now owe a receipt.
    ///
    /// **Every poll happens before any binding**: the loop below is synchronous and holds no
    /// lock across an `await`, so one entry's stalled receipt cannot stand between every later
    /// entry in the sorted map and its own poll.
    ///
    /// A sealed entry is skipped rather than polled: every chaining call on one is
    /// `ExportError::Sealed`, so polling it would discover the refusal once per
    /// record. An entry already binding is skipped too — its runtime is out of the slot for
    /// the length of that call, and asking would be `held_elsewhere` four times a second.
    #[cfg(feature = "v1-compat")]
    fn poll_pass(&self) -> (Vec<(String, String)>, Option<DaemonError>) {
        let now = now_unix();
        let mut first: Option<DaemonError> = None;
        let mut binding = Vec::new();
        let ids = match self.entry_ids() {
            Ok(ids) => ids,
            Err(e) => return (binding, Some(e)),
        };
        for entry_id in ids {
            if self.is_sealed(&entry_id).unwrap_or(false) || self.is_binding(&entry_id) {
                continue;
            }
            let polled = self.with_entry(&entry_id, |r| {
                r.poll_once_at(now)?;
                Ok::<_, DaemonError>(r.may_start().then(|| r.bound_session()).flatten())
            });
            match polled.and_then(|inner| inner) {
                Ok(Some(session)) => binding.push((entry_id, session)),
                Ok(None) => {}
                Err(e) => {
                    first.get_or_insert(e);
                }
            }
        }
        (binding, first)
    }

    /// One entry's start binding on a task of its own, holding a permit [`Daemon::stop`]
    /// waits for. A no-op when one is already in flight for this entry.
    ///
    /// **The permit is taken here, before the spawn.** Acquired inside the task, it would
    /// leave a window — between this function returning and the runtime first polling that
    /// task — in which the semaphore was untouched: a `stop` arriving inside it would take
    /// all 64 permits, see an idle daemon and return over a binding that had not yet run.
    /// The permit is a synchronous `try_acquire_owned`, so there is no window at all, and a
    /// daemon already at [`MAX_IN_FLIGHT`] does not spawn the binding — the next poll asks
    /// again.
    ///
    /// **The id is removed by a guard and not by a line at the end.** A panicking or
    /// cancelled binding would otherwise leave the entry in `binding` forever, and
    /// [`Daemon::poll_pass`] steps over an entry that is in it — so one panic would stop that
    /// run being polled for the life of the daemon. [`BindingGuard`] runs on every exit,
    /// including an unwind and a dropped future, and it releases the id before the permit
    /// so a `stop` that has just been let through can never find a stale one.
    #[cfg(feature = "v1-compat")]
    fn spawn_bind_start(&self, entry_id: String, session_id: String) {
        if !self.binding().insert(entry_id.clone()) {
            return;
        }
        // Built before the permit, so a daemon at its ceiling gives the id back on the way
        // out of this function rather than pinning an entry nothing is binding.
        let guard = BindingGuard {
            daemon: self.clone_handle(),
            entry_id: entry_id.clone(),
        };
        let Ok(permit) = self.inner.blocking.clone().try_acquire_owned() else {
            return;
        };
        let daemon = self.clone_handle();
        tokio::spawn(async move {
            // Dropped last, and therefore after the guard: `stop` returns when the last
            // permit is back, and it must not return while an id is still in the set.
            let _permit = permit;
            let _guard = guard;
            #[cfg(any(test, feature = "test-hooks"))]
            if daemon
                .inner
                .panic_next_binding
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                panic!("test-hooks: this binding panics, so the drop guard is asserted");
            }
            if let Err(e) = daemon.bind_start(&entry_id, &session_id).await {
                daemon.inner.cycles.note(Cadence::Tick, &e);
            }
        });
    }

    /// Test-only: make the next spawned start binding panic instead of requesting its
    /// receipt.
    ///
    /// The thing under test is [`BindingGuard`] — that an entry leaves the `binding` set
    /// however its task ends — and nothing a caller can do to a healthy daemon produces an
    /// unwind on that path. Behind the same off-by-default `test-hooks` feature the chain
    /// writer's fault injection uses, so no shipped build has a way to reach it.
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn panic_next_binding(&self) {
        self.inner
            .panic_next_binding
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Test-only: make [`Daemon::seal_entry`] skip its `note_seal_outcome` call, the one
    /// time it is about to make one.
    ///
    /// The thing under test is [`EntryRuntime::status_at`]'s fallback to the registry row —
    /// that the document it builds still matches the row when this call is the one that
    /// gets swallowed, exactly as it is for real when a concurrent pump evicts the drained
    /// entry in the gap between `seal_entry`'s registry write and this call. Producing that
    /// gap with a genuine race is timing nothing in this crate controls; this seam makes it
    /// deterministic.
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn skip_next_note_seal_outcome(&self) {
        self.inner
            .skip_next_note_seal_outcome
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Test-only: inject a persisted buffer record into a live runtime.  This models a
    /// corrupt post-terminal write without needing a second redb owner while the daemon is
    /// still running.
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn append_live_buffer_for_test(
        &self,
        entry_id: &str,
        envelope: &goalcup_protocol::envelope::Envelope,
    ) -> Result<(), DaemonError> {
        self.with_entry(entry_id, |runtime| runtime.append_buffer_for_test(envelope))?
    }

    /// Test-only: expose the live runtime's buffer-before-registry crash window without
    /// opening a second redb owner.  The production uploader remains the only shipped
    /// caller that acknowledges a buffer.
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn ack_live_buffer_for_test(
        &self,
        entry_id: &str,
        up_to_seq: u64,
    ) -> Result<(), DaemonError> {
        self.with_entry(entry_id, |runtime| {
            runtime.acknowledge_buffer_for_test(up_to_seq)
        })?
    }

    /// Test-only: stage an impossible tail only after the live uploader has returned, so
    /// the final sealed validation is the decision under test.
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn stage_live_post_pump_tail_for_test(
        &self,
        entry_id: &str,
        envelope: goalcup_protocol::envelope::Envelope,
    ) -> Result<(), DaemonError> {
        self.with_entry(entry_id, |runtime| {
            runtime.stage_post_pump_tail_for_test(envelope);
            Ok(())
        })?
    }

    /// Test-only: inject a post-terminal record through the real recovered drain after it
    /// has been retained for uploader backoff. No second buffer owner is opened.
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn append_retained_buffer_for_test(
        &self,
        entry_id: &str,
        envelope: &goalcup_protocol::envelope::Envelope,
    ) -> Result<(), DaemonError> {
        let slot = self
            .inner
            .sealed_drains
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(entry_id)
            .cloned()
            .ok_or_else(|| {
                DaemonError::Missing(format!("sealed drain {entry_id} has not been retained"))
            })?;
        let held = slot.drain.lock().unwrap_or_else(|e| e.into_inner());
        let drain = held.as_ref().ok_or_else(|| held_elsewhere(entry_id))?;
        drain.append_buffer_for_test(envelope)
    }

    /// Test-only: inject a recovered buffer floor through the retained owner. The normal
    /// upload path remains the only shipped caller that acknowledges this buffer.
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn ack_retained_buffer_for_test(
        &self,
        entry_id: &str,
        up_to_seq: u64,
    ) -> Result<(), DaemonError> {
        let slot = self
            .inner
            .sealed_drains
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(entry_id)
            .cloned()
            .ok_or_else(|| {
                DaemonError::Missing(format!("sealed drain {entry_id} has not been retained"))
            })?;
        let held = slot.drain.lock().unwrap_or_else(|e| e.into_inner());
        let drain = held.as_ref().ok_or_else(|| held_elsewhere(entry_id))?;
        drain.acknowledge_buffer_for_test(up_to_seq)
    }

    /// Test-only: stage a buffer tail after the recovered uploader returns but before this
    /// daemon's final drain validation. The real retained owner keeps exclusive buffer
    /// access; this seam is ordering evidence, not an alternate production write path.
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn stage_retained_post_pump_tail_for_test(
        &self,
        entry_id: &str,
        envelope: goalcup_protocol::envelope::Envelope,
    ) -> Result<(), DaemonError> {
        let slot = self
            .inner
            .sealed_drains
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(entry_id)
            .cloned()
            .ok_or_else(|| {
                DaemonError::Missing(format!("sealed drain {entry_id} has not been retained"))
            })?;
        let mut held = slot.drain.lock().unwrap_or_else(|e| e.into_inner());
        let drain = held.as_mut().ok_or_else(|| held_elsewhere(entry_id))?;
        drain.stage_post_pump_tail_for_test(envelope);
        Ok(())
    }

    /// Test-only: fail one retained terminal quarantine persistence after a successful
    /// uploader return and before the final status/eviction decision.
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_next_retained_terminal_quarantine_for_test(
        &self,
        entry_id: &str,
    ) -> Result<(), DaemonError> {
        let slot = self.retained_drain_slot_for_test(entry_id)?;
        let mut held = slot.drain.lock().unwrap_or_else(|e| e.into_inner());
        let drain = held.as_mut().ok_or_else(|| held_elsewhere(entry_id))?;
        drain.fail_next_terminal_quarantine_persistence_for_test();
        Ok(())
    }

    /// Test-only: fail the one common registry transaction that persists a terminal-drain
    /// quarantine. Live runtimes, fully-ACKed equality drains, partial drains, and startup
    /// validation all take this same durable boundary.
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_next_terminal_drain_quarantine_persistence_for_test(&self) {
        self.inner
            .registry
            .fail_next_terminal_drain_validation_persistence_for_test();
    }

    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_workspace_append_after_for_test(
        &self,
        entry_id: &str,
        appends_before_failure: u32,
    ) -> Result<(), DaemonError> {
        self.with_entry(entry_id, |runtime| {
            runtime.fail_workspace_append_after_for_test(appends_before_failure)
        })?
    }

    /// Test-only: make the successor's sealed-harness reaper abort after startup terminal
    /// classification.  The root-scoped hook is consumed by that exact successor only.
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_next_startup_reap_for_test(&self) {
        super::seal::fail_next_reap_for_test(&self.inner.config.artifact_root);
    }

    /// Test-only: fail one retained durable refresh after a successful uploader return and
    /// before the final status/eviction decision.
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_next_retained_post_pump_refresh_for_test(
        &self,
        entry_id: &str,
    ) -> Result<(), DaemonError> {
        let slot = self.retained_drain_slot_for_test(entry_id)?;
        let mut held = slot.drain.lock().unwrap_or_else(|e| e.into_inner());
        let drain = held.as_mut().ok_or_else(|| held_elsewhere(entry_id))?;
        drain.fail_next_post_pump_refresh_for_test();
        Ok(())
    }

    /// Test-only: resume the retained uploader so a post-uploader validation test actually
    /// crosses the successful delivery/ACK transaction instead of observing its backoff.
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn resume_retained_upload_for_test(&self, entry_id: &str) -> Result<(), DaemonError> {
        let slot = self.retained_drain_slot_for_test(entry_id)?;
        let mut held = slot.drain.lock().unwrap_or_else(|e| e.into_inner());
        let drain = held.as_mut().ok_or_else(|| held_elsewhere(entry_id))?;
        drain.resume_upload_for_test();
        Ok(())
    }

    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    fn retained_drain_slot_for_test(
        &self,
        entry_id: &str,
    ) -> Result<Arc<SealedDrainSlot>, DaemonError> {
        self.inner
            .sealed_drains
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(entry_id)
            .cloned()
            .ok_or_else(|| {
                DaemonError::Missing(format!("sealed drain {entry_id} has not been retained"))
            })
    }

    /// Test-only: stage a real live seal intent without allowing its terminal writer to
    /// append, for deterministic crash-before-terminal upload provenance coverage.
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn begin_live_sealing_without_terminal_for_test(
        &self,
        entry_id: &str,
    ) -> Result<goalcup_protocol::envelope::Envelope, DaemonError> {
        self.with_entry(entry_id, |runtime| {
            runtime.begin_sealing_without_terminal_for_test()
        })?
    }

    /// Test-only: pause a live terminal immediately after its writer appended the exact
    /// seal link and before it reopens that trace to finish the durable promotion. The
    /// returned handle is installed on this daemon's exact retained runtime.
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn pause_after_live_seal_write_for_test(
        &self,
        entry_id: &str,
    ) -> Result<super::LiveSealWritePause, DaemonError> {
        self.with_entry(entry_id, |runtime| {
            runtime.arm_live_seal_write_pause_for_test()
        })?
    }

    /// Test-only: how many of [`MAX_IN_FLIGHT`] permits are free right now.
    ///
    /// What it exists for is the invariant `stop` rests on — that a spawned unit of work
    /// holds its permit from before it is spawned — which is otherwise invisible: a test
    /// can only watch `stop` take a long time, and that is also what a coincidence looks
    /// like.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn free_permits(&self) -> usize {
        self.inner.blocking.available_permits()
    }

    /// Whether this entry's start binding is on a spawned task right now.
    #[cfg(feature = "v1-compat")]
    pub fn is_binding(&self, entry_id: &str) -> bool {
        self.binding().contains(entry_id)
    }

    #[cfg(feature = "v1-compat")]
    fn binding(&self) -> std::sync::MutexGuard<'_, BTreeSet<String>> {
        self.inner.binding.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A second handle on the same daemon, for a task that outlives the caller's borrow.
    ///
    /// Every field is behind the one `Arc`, so this is a pointer copy and not a second
    /// daemon: the registry handle, the entry map and the listener are shared, and D4's
    /// "one daemon per machine" is unaffected — that is a property of the redb file lock
    /// this handle does not take again.
    ///
    /// [`Daemon::stop`] is by value and releases the locks only when the last handle is
    /// gone, which is why the spawned work that holds one of these is drained before it
    /// returns rather than merely aborted.
    pub fn clone_handle(&self) -> Daemon {
        Daemon {
            inner: self.inner.clone(),
        }
    }

    /// The start receipt, requested once the gate allows it.
    ///
    /// Public beside [`Daemon::tick`], which calls it, because a caller driving the daemon
    /// step by step — the round-trip fixture, a resumed run whose session is already known —
    /// has to be able to reach it. A second call on a started entry is a no-op, not a
    /// refusal.
    #[cfg(feature = "v1-compat")]
    pub async fn bind_start(&self, entry_id: &str, session_id: &str) -> Result<(), DaemonError> {
        let mut held = self.borrow_entry(entry_id)?;
        let published = {
            let runtime = held.get()?;
            let was_started = runtime.entry().started_at.is_some();
            runtime.bind_start(session_id).await?;
            // The receipt transaction is the point at which the public run became real.
            // Publish in this same successful call, rather than waiting for the status
            // cadence, so an immediate status read cannot return the pre-start cache whose
            // session is still absent.
            (!was_started)
                .then(|| {
                    runtime.publish_status(
                        &self.inner.config.config_dir,
                        &self.inner.config.artifact_root,
                        now_unix(),
                    )
                })
                .transpose()?
        };
        drop(held);
        if let Some(signed) = published {
            self.cache()?.insert(entry_id.to_string(), signed);
            self.refresh_current()?;
        }
        Ok(())
    }

    /// One drain of every entry's write-ahead buffer.
    ///
    /// `Uploader::resume` is not called from here — it drops an outstanding backoff, so a
    /// loop that called it would turn every failure into an immediate retry. That belongs to
    /// an explicit resume, which is [`Daemon::resume_upload`].
    #[cfg(feature = "v1-compat")]
    pub async fn pump_all(&self) -> Result<(), DaemonError> {
        let mut first: Option<DaemonError> = None;
        let live_entry_ids: BTreeSet<String> = self.entry_ids()?.into_iter().collect();
        for entry_id in &live_entry_ids {
            // Out for another asynchronous call — a start binding's receipt round trip, on
            // its own task. There is nothing this pump can do with an entry it
            // cannot hold, and the next upload interval asks again; counting it would put
            // a cycle error on the status document once per interval for the length of a
            // receipt timeout, about a run that is not in trouble.
            if self.is_borrowed(entry_id) {
                continue;
            }
            // A prior terminal refusal could not be durably classified.  Until a later
            // clean terminal validation can publish a brand-new completion, this cadence
            // must not let the ordinary runtime publisher resurrect its old all-clear.
            let status_was_withheld = self.withheld_terminal_status().contains(entry_id);
            let mut held = match self.borrow_entry(entry_id) {
                Ok(held) => held,
                Err(e) => {
                    first.get_or_insert(e);
                    continue;
                }
            };
            let (
                pumped,
                published,
                terminal_review,
                terminal_persist_failed,
                terminal_invalidation,
            ) = match held.get() {
                Ok(runtime) => {
                    let previous_ack = runtime.entry().last_ack_seq;
                    match runtime.pump().await {
                        Ok(outcome) => {
                            let terminal_review =
                                runtime.has_persisted_terminal_drain_validation_failure();
                            let terminal_persist_failed =
                                runtime.terminal_validation_persistence_failed();
                            // The owner has observed the failure and remains out of its
                            // slot.  Make all-clear publication unavailable *before* it can
                            // be returned for another cadence to inspect.
                            let latched = if terminal_persist_failed {
                                self.latch_terminal_status_before_owner_release(entry_id)
                            } else {
                                Ok(())
                            };
                            let terminal_invalidation =
                                if terminal_persist_failed && latched.is_ok() {
                                    self.invalidate_terminal_status_with_artifact(
                                        entry_id,
                                        runtime.artifact(),
                                    )
                                } else {
                                    Ok(())
                                };
                            let published = if latched.is_ok()
                                && !status_was_withheld
                                && !terminal_review
                                && !terminal_persist_failed
                                && (outcome.uploaded > 0 || outcome.acked_seq != previous_ack)
                            {
                                self.publish_live_status_unless_withheld(entry_id, runtime)
                            } else {
                                latched.map(|_| None)
                            };
                            (
                                Ok(()),
                                published,
                                terminal_review,
                                terminal_persist_failed,
                                terminal_invalidation,
                            )
                        }
                        Err(error) => {
                            let terminal_persist_failed =
                                runtime.terminal_validation_persistence_failed();
                            let latched = if terminal_persist_failed {
                                self.latch_terminal_status_before_owner_release(entry_id)
                            } else {
                                Ok(())
                            };
                            let terminal_invalidation =
                                if terminal_persist_failed && latched.is_ok() {
                                    self.invalidate_terminal_status_with_artifact(
                                        entry_id,
                                        runtime.artifact(),
                                    )
                                } else {
                                    Ok(())
                                };
                            (
                                Err(error),
                                latched.map(|_| None),
                                runtime.has_persisted_terminal_drain_validation_failure(),
                                terminal_persist_failed,
                                terminal_invalidation,
                            )
                        }
                    }
                }
                Err(e) => (Err(e), Ok(None), false, false, Ok(())),
            };
            // The runtime still owns the directory capability through which it observed the
            // terminal refusal. Publish review before returning that owner to its slot: a
            // later path reacquisition could follow a swapped `artifacts/<entry>` spelling.
            let terminal_review_publication = if terminal_review {
                held.get().and_then(|runtime| {
                    self.publish_terminal_review_with_artifact(entry_id, Some(runtime.artifact()))
                })
            } else {
                Ok(())
            };
            let clean_completion_publication = if status_was_withheld
                && !terminal_persist_failed
                && !terminal_review
                && self
                    .inner
                    .registry
                    .get(entry_id)
                    .ok()
                    .flatten()
                    .is_some_and(|entry| {
                        entry.state == EntryState::Sealed
                            && entry.chain_seq.is_some()
                            && !entry.terminal_drain_needs_review()
                            && entry.last_ack_seq == entry.chain_seq
                    }) {
                held.get()
                    .and_then(|runtime| {
                        self.publish_clean_terminal_completion_with_artifact(
                            entry_id,
                            runtime.artifact(),
                        )
                    })
                    .map(|_| true)
            } else {
                Ok(false)
            };
            // The runtime goes back into its slot here, which is what the eviction below
            // then takes it out of.
            drop(held);
            if let Err(e) = pumped {
                first.get_or_insert(e);
            }
            let mut completion_published = false;
            if terminal_persist_failed {
                if let Err(error) = terminal_invalidation {
                    first.get_or_insert(error);
                }
                if let Err(error) = self.finish_withholding_terminal_status_after_owner_release() {
                    first.get_or_insert(error);
                }
            } else if terminal_review {
                if let Err(error) = terminal_review_publication {
                    first.get_or_insert(error);
                }
            } else if status_was_withheld {
                match clean_completion_publication {
                    Ok(true) => {
                        if let Err(error) = self.refresh_current() {
                            first.get_or_insert(error);
                        } else {
                            completion_published = true;
                        }
                    }
                    Ok(false) => {}
                    Err(error) => {
                        first.get_or_insert(error);
                    }
                }
            } else {
                match published {
                    Ok(Some(_)) => {
                        if let Err(error) = self.refresh_current() {
                            first.get_or_insert(error);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        first.get_or_insert(error);
                    }
                }
            }
            if !terminal_persist_failed
                && (!status_was_withheld || completion_published)
                && let Err(e) = self.evict_if_drained(entry_id)
            {
                first.get_or_insert(e);
            }
        }
        if let Err(e) = self.pump_recovered_sealed_buffers(&live_entry_ids).await {
            first.get_or_insert(e);
        }
        self.finish(Cadence::Upload, first)
    }

    /// Drain terminal buffers left behind by a daemon restart.
    ///
    /// [`Daemon::rehydrate`] correctly leaves sealed rows out of the live capture map, but
    /// that cannot turn their encrypted write-ahead buffer into an archival dead end. A
    /// recovered `Sealing` intent first re-buffers its exact terminal envelope and promotes
    /// the row; this path retries it on every upload cadence until its verified
    /// acknowledgement reaches the terminal head. The smaller [`SealedDrain`] owner keeps
    /// the uploader's backoff without reopening tailers or a workspace binding.
    #[cfg(feature = "v1-compat")]
    async fn pump_recovered_sealed_buffers(
        &self,
        live_entry_ids: &BTreeSet<String>,
    ) -> Result<(), DaemonError> {
        let mut first: Option<DaemonError> = None;
        for entry in self.inner.registry.all()? {
            if entry.state != EntryState::Sealed
                || entry.chain_seq.is_none()
                || live_entry_ids.contains(&entry.entry_id)
            {
                continue;
            }
            let entry_id = entry.entry_id.clone();
            // Even an apparently complete terminal row must reopen its safe buffer owner
            // and check the durable floor/tail.  Otherwise `last_ack_seq == chain_seq`
            // turns the corruption check into a restart-only dead branch.
            if entry.last_ack_seq == entry.chain_seq {
                let slot = match self.sealed_drain_slot(entry) {
                    Ok(slot) => slot,
                    Err(error) => {
                        if let Err(quarantine_error) =
                            self.quarantine_terminal_drain_validation_failure(&entry_id)
                        {
                            first.get_or_insert(quarantine_error);
                        }
                        first.get_or_insert(error);
                        continue;
                    }
                };
                let taken = slot.drain.lock().unwrap_or_else(|e| e.into_inner()).take();
                let Some(drain) = taken else {
                    continue;
                };
                let mut held = BorrowedSealedDrain {
                    slot: slot.clone(),
                    drain: Some(drain),
                };
                held.get()?.begin_terminal_validation_cadence();
                let validated = held
                    .get()
                    .and_then(|drain| drain.is_drained(&self.inner.registry));
                let terminal_review = held
                    .get()
                    .is_ok_and(|drain| drain.has_persisted_validation_failure());
                let terminal_persist_failed = held
                    .get()
                    .is_ok_and(|drain| drain.terminal_validation_persistence_failed());
                let latch_error = terminal_persist_failed
                    .then(|| self.latch_terminal_status_before_owner_release(&entry_id))
                    .transpose()
                    .err();
                let terminal_invalidation = if terminal_persist_failed && latch_error.is_none() {
                    held.get().and_then(|drain| {
                        self.invalidate_terminal_status_with_artifact(&entry_id, drain.artifact())
                    })
                } else {
                    Ok(())
                };
                let terminal_review_publication = if terminal_review {
                    held.get().and_then(|drain| {
                        self.publish_terminal_review_with_artifact(
                            &entry_id,
                            Some(drain.artifact()),
                        )
                    })
                } else {
                    Ok(())
                };
                let clean_completion_publication = if !terminal_review
                    && !terminal_persist_failed
                    && matches!(&validated, Ok(true))
                {
                    held.get().and_then(|drain| {
                        self.publish_clean_terminal_completion_with_artifact(
                            &entry_id,
                            drain.artifact(),
                        )
                    })
                } else {
                    Ok(())
                };
                drop(held);
                if let Some(error) = latch_error {
                    first.get_or_insert(error);
                }
                let mut completion_published = false;
                if terminal_persist_failed {
                    if let Err(error) = terminal_invalidation {
                        first.get_or_insert(error);
                    }
                    if let Err(error) =
                        self.finish_withholding_terminal_status_after_owner_release()
                    {
                        first.get_or_insert(error);
                    }
                } else if terminal_review {
                    if let Err(error) = terminal_review_publication {
                        first.get_or_insert(error);
                    }
                } else if matches!(&validated, Ok(true)) {
                    match clean_completion_publication {
                        Ok(()) => match self.refresh_current() {
                            Ok(()) => completion_published = true,
                            Err(error) => {
                                first.get_or_insert(error);
                            }
                        },
                        Err(error) => {
                            first.get_or_insert(error);
                        }
                    }
                }
                match validated {
                    Ok(true)
                        if !terminal_review && !terminal_persist_failed && completion_published =>
                    {
                        self.inner
                            .sealed_drains
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .remove(&entry_id);
                    }
                    Ok(true) => {}
                    Ok(false) => {
                        first.get_or_insert(DaemonError::Refused(format!(
                            "sealed entry {entry_id} claims its terminal is acknowledged but \
                             its drain is not complete"
                        )));
                    }
                    Err(error) => {
                        first.get_or_insert(error);
                    }
                }
                continue;
            }
            if entry.started_at.is_none() {
                continue;
            }
            let previous_ack = entry.last_ack_seq;
            let status_was_withheld = self.withheld_terminal_status().contains(&entry_id);
            let slot = match self.sealed_drain_slot(entry) {
                Ok(slot) => slot,
                Err(error) => {
                    if let Err(quarantine_error) =
                        self.quarantine_terminal_drain_validation_failure(&entry_id)
                    {
                        first.get_or_insert(quarantine_error);
                    }
                    first.get_or_insert(error);
                    continue;
                }
            };
            let taken = slot.drain.lock().unwrap_or_else(|e| e.into_inner()).take();
            let Some(drain) = taken else {
                // Another upload cadence owns this drain. Like a live entry held by a
                // receipt round trip, that is transient rather than a cycle error.
                continue;
            };
            let mut held = BorrowedSealedDrain {
                slot: slot.clone(),
                drain: Some(drain),
            };
            held.get()?.begin_terminal_validation_cadence();
            let pumped = held.get()?.pump(&self.inner.registry).await;
            // Validate after the uploader returns and before choosing any success document.
            // The buffer can have acquired a terminal-invalid record in the narrow interval
            // the drain was exclusively borrowed; a clean cache written first would remain
            // an all-clear until another cadence noticed the durable quarantine.
            let drained = held
                .get()
                .and_then(|drain| drain.is_drained(&self.inner.registry));
            let terminal_review = held
                .get()
                .is_ok_and(|drain| drain.has_persisted_validation_failure());
            let terminal_persist_failed = held
                .get()
                .is_ok_and(|drain| drain.terminal_validation_persistence_failed());
            let final_validation = match &drained {
                // A persistence failure is more restrictive than an older durable marker:
                // this cadence's terminal conclusion is not safe to publish as clean, and
                // must retain the fail-closed latch until a later clean validation wins.
                _ if terminal_persist_failed => RecoveredFinalValidation::PersistFailed,
                Ok(_) if terminal_review => RecoveredFinalValidation::Quarantined,
                Ok(drained) => RecoveredFinalValidation::Clean(*drained),
                Err(_) if terminal_review => RecoveredFinalValidation::Quarantined,
                // The terminal refusal was not durably recorded. This must withhold stale
                // views, not fall through to a successful upload's clean status.
                Err(_) => RecoveredFinalValidation::PersistFailed,
            };
            let latch_error = matches!(final_validation, RecoveredFinalValidation::PersistFailed)
                .then(|| self.latch_terminal_status_before_owner_release(&entry_id))
                .transpose()
                .err();
            let terminal_invalidation =
                matches!(final_validation, RecoveredFinalValidation::PersistFailed)
                    && latch_error.is_none();
            let terminal_invalidation = if terminal_invalidation {
                held.get().and_then(|drain| {
                    self.invalidate_terminal_status_with_artifact(&entry_id, drain.artifact())
                })
            } else {
                Ok(())
            };
            // The drain is the only owner that can move an evicted terminal row's
            // acknowledgement watermark. Publish that durable movement synchronously,
            // rather than leaving the signed cache at its pre-restart value until a live
            // runtime happens to exist (it never will for this row). A terminal review
            // instead goes through the durable-marker path after the borrow is returned.
            let published = match (&pumped, &final_validation) {
                (Ok(outcome), RecoveredFinalValidation::Clean(false))
                    if !status_was_withheld
                        && (outcome.uploaded > 0 || outcome.acked_seq != previous_ack) =>
                {
                    self.publish_recovered_status_unless_withheld(&entry_id, held.get()?)
                }
                _ => Ok(None),
            };
            let terminal_review_publication =
                if matches!(&final_validation, RecoveredFinalValidation::Quarantined) {
                    held.get().and_then(|drain| {
                        self.publish_terminal_review_with_artifact(
                            &entry_id,
                            Some(drain.artifact()),
                        )
                    })
                } else {
                    Ok(())
                };
            let clean_completion_publication =
                if matches!(&final_validation, RecoveredFinalValidation::Clean(true)) {
                    held.get().and_then(|drain| {
                        self.publish_clean_terminal_completion_with_artifact(
                            &entry_id,
                            drain.artifact(),
                        )
                    })
                } else {
                    Ok(())
                };
            drop(held);
            if let Some(error) = latch_error {
                first.get_or_insert(error);
            }
            if let Err(error) = pumped {
                first.get_or_insert(error);
            }
            let mut clean_terminal_completion_published = false;
            match &final_validation {
                RecoveredFinalValidation::Clean(true) => match clean_completion_publication {
                    Ok(()) => match self.refresh_current() {
                        Ok(()) => clean_terminal_completion_published = true,
                        Err(error) => {
                            first.get_or_insert(error);
                        }
                    },
                    Err(error) => {
                        first.get_or_insert(error);
                    }
                },
                RecoveredFinalValidation::Clean(false) => {
                    // A retained row that is not yet terminally complete may publish an
                    // ordinary acknowledgement movement, except when an earlier failed
                    // terminal classification left its views withheld.
                    if !status_was_withheld {
                        match published {
                            Ok(Some(_)) => {
                                if let Err(error) = self.refresh_current() {
                                    first.get_or_insert(error);
                                }
                            }
                            Ok(None) => {}
                            Err(error) => {
                                first.get_or_insert(error);
                            }
                        }
                    }
                }
                RecoveredFinalValidation::Quarantined => {
                    if let Err(error) = terminal_review_publication {
                        first.get_or_insert(error);
                    }
                }
                RecoveredFinalValidation::PersistFailed => {
                    if let Err(error) = terminal_invalidation {
                        first.get_or_insert(error);
                    }
                    if let Err(error) =
                        self.finish_withholding_terminal_status_after_owner_release()
                    {
                        first.get_or_insert(error);
                    }
                }
            }
            match drained {
                Ok(true)
                    if matches!(final_validation, RecoveredFinalValidation::Clean(true))
                        && clean_terminal_completion_published =>
                {
                    self.inner
                        .sealed_drains
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&entry_id);
                }
                Ok(_) => {}
                Err(error) => {
                    first.get_or_insert(error);
                }
            }
        }
        match first {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Return the retained upload-only owner for an evicted terminal row, rebuilding it
    /// from the validated trace exactly once per daemon lifetime.
    #[cfg(feature = "v1-compat")]
    fn sealed_drain_slot(&self, entry: Entry) -> Result<Arc<SealedDrainSlot>, DaemonError> {
        if let Some(slot) = self
            .inner
            .sealed_drains
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&entry.entry_id)
            .cloned()
        {
            return Ok(slot);
        }
        let slot = Arc::new(SealedDrainSlot {
            drain: Mutex::new(Some(SealedDrain::rebuild(
                entry.clone(),
                &self.inner.config,
                &self.inner.registry,
                &self.inner.artifact_root,
            )?)),
        });
        let mut drains = self
            .inner
            .sealed_drains
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        Ok(drains
            .entry(entry.entry_id.clone())
            .or_insert_with(|| slot.clone())
            .clone())
    }

    /// A recovered terminal drain can fail validation while it is being rebuilt, before a
    /// [`SealedDrain`] exists to publish its own status. Persist and publish the same
    /// capture-closed quarantine in that path, so a later cadence cannot leave the last
    /// signed document claiming a successful sealed upload.
    #[cfg(feature = "v1-compat")]
    fn quarantine_terminal_drain_validation_failure(
        &self,
        entry_id: &str,
    ) -> Result<(), DaemonError> {
        match self
            .inner
            .registry
            .record_terminal_drain_validation_failure(entry_id)
        {
            Ok(_) => self.publish_terminal_review(entry_id),
            Err(error) => {
                // The registry could not establish review authority. Set the process-local
                // latch before any later cache/eviction path can read a stale all-clear.
                self.withhold_terminal_status_after_failed_validation(entry_id)?;
                Err(error)
            }
        }
    }

    /// Make a durable terminal-validation refusal the daemon/API's truth before attempting
    /// any fallible filesystem view. A stale cache can otherwise outlive a full-disk mirror
    /// failure and claim a zero-pending sealed all-clear after the registry has quarantined
    /// the entry. The per-entry replacement deliberately leaves every unrelated status
    /// document alone; later upload cadences retry the file views through this same method.
    #[cfg(feature = "v1-compat")]
    fn publish_terminal_review(&self, entry_id: &str) -> Result<(), DaemonError> {
        self.publish_terminal_review_with_artifact(entry_id, None)
    }

    /// Publish terminal review through the capability retained by the live runtime or
    /// recovered drain that observed it. Startup has no predecessor owner, so only that
    /// caller passes `None` and safely reacquires the configured entry spelling.
    #[cfg(feature = "v1-compat")]
    fn publish_terminal_review_with_artifact(
        &self,
        entry_id: &str,
        artifact: Option<&goalcup_platform::tracecap::EntryArtifactRoot>,
    ) -> Result<(), DaemonError> {
        let entry = self
            .inner
            .registry
            .get(entry_id)?
            .ok_or_else(|| DaemonError::Missing(format!("entry {entry_id}")))?;
        if !entry.terminal_drain_needs_review() {
            return Err(DaemonError::Refused(format!(
                "entry {entry_id} is not a terminal-drain review row"
            )));
        }
        let signed = status::compose_capture_closed(&entry, now_unix())?;
        // This comes first. The daemon control API reads UiState, while the MCP file cache
        // and the status line are retrying views that can independently fail. A composed
        // durable review is the one condition that releases a process-local withheld latch.
        let mut withheld = self.withheld_terminal_status();
        self.cache()?.insert(entry_id.to_string(), signed.clone());
        withheld.remove(entry_id);
        drop(withheld);
        let mut first = None;
        let replaced = match artifact {
            Some(artifact) => status::replace_terminal_views_with_artifact(
                &signed,
                &entry,
                &self.inner.config.config_dir,
                artifact,
            ),
            None => status::replace_terminal_views_with_root(
                &signed,
                &entry,
                &self.inner.config.config_dir,
                &self.inner.artifact_root,
            ),
        };
        if let Err(error) = replaced {
            first.get_or_insert(error);
        }
        if let Err(error) = self.refresh_current() {
            first.get_or_insert(error);
        }
        match first {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Publish a live runtime only while the terminal-withheld authority is held and still
    /// absent.  A check before borrowing is not enough: a terminal persistence failure can
    /// be discovered by the owner immediately before a status cadence reaches its cache and
    /// files.
    #[cfg(feature = "v1-compat")]
    fn publish_live_status_unless_withheld(
        &self,
        entry_id: &str,
        runtime: &EntryRuntime,
    ) -> Result<Option<SignedStatus>, DaemonError> {
        let withheld = self.withheld_terminal_status();
        if withheld.contains(entry_id) {
            return Ok(None);
        }
        let signed = runtime.publish_status(
            &self.inner.config.config_dir,
            &self.inner.config.artifact_root,
            now_unix(),
        )?;
        self.cache()?.insert(entry_id.to_string(), signed.clone());
        Ok(Some(signed))
    }

    /// The recovered-drain analogue of [`Self::publish_live_status_unless_withheld`].  A
    /// retained terminal is not in the live map, but it shares the same signed cache and
    /// must not write an ordinary acknowledgement status across a newly installed latch.
    #[cfg(feature = "v1-compat")]
    fn publish_recovered_status_unless_withheld(
        &self,
        entry_id: &str,
        drain: &SealedDrain,
    ) -> Result<Option<SignedStatus>, DaemonError> {
        let withheld = self.withheld_terminal_status();
        if withheld.contains(entry_id) {
            return Ok(None);
        }
        let signed = drain.publish_status(&self.inner.config.config_dir)?;
        self.cache()?.insert(entry_id.to_string(), signed.clone());
        Ok(Some(signed))
    }

    /// A previously withheld terminal row has later passed a clean, fully acknowledged
    /// validation. Publish its new sealed completion through every ordinary view before
    /// releasing the latch or removing its retained owner. A successful registry read alone
    /// cannot safely resurrect an old zero-pending cache after a failed quarantine write.
    #[cfg(feature = "v1-compat")]
    fn publish_clean_terminal_completion_with_artifact(
        &self,
        entry_id: &str,
        artifact: &goalcup_platform::tracecap::EntryArtifactRoot,
    ) -> Result<(), DaemonError> {
        let entry = self
            .inner
            .registry
            .get(entry_id)?
            .ok_or_else(|| DaemonError::Missing(format!("entry {entry_id}")))?;
        if entry.terminal_drain_needs_review()
            || entry.state != EntryState::Sealed
            || entry.last_ack_seq != entry.chain_seq
        {
            return Err(DaemonError::Refused(format!(
                "entry {entry_id} is not a clean fully acknowledged terminal row"
            )));
        }
        let signed = status::compose_capture_closed(&entry, now_unix())?;
        let mut withheld = self.withheld_terminal_status();
        status::mirror(&signed, &self.inner.config.config_dir)?;
        status::write_entry_status_line_rooted(&signed, artifact)?;
        self.cache()?.insert(entry_id.to_string(), signed);
        // The latch is cleared only after the new signed completion has reached the cache,
        // the per-entry/latest disk views and the status line.
        withheld.remove(entry_id);
        drop(withheld);
        Ok(())
    }

    /// Withhold status after a final terminal validation error that could not establish a
    /// durable review row. The in-process API becomes unavailable before filesystem work;
    /// exact old views are then invalidated best-effort. A storage failure can prevent both
    /// unlink and replacement, which this daemon cannot make atomic across files, but it
    /// never serves those bytes through its own public status API while the latch is held.
    #[cfg(feature = "v1-compat")]
    fn withhold_terminal_status_after_failed_validation(
        &self,
        entry_id: &str,
    ) -> Result<(), DaemonError> {
        self.latch_terminal_status_before_owner_release(entry_id)?;
        self.finish_withholding_terminal_status(entry_id)
    }

    /// Install the fail-closed state while the runtime/drain that made the terminal
    /// decision is still exclusively borrowed.  This is deliberately cache-only and
    /// in-process: filesystem invalidation belongs after the owner is returned, but no
    /// status reader or ordinary publisher may pass this point with stale truth.
    #[cfg(feature = "v1-compat")]
    pub(super) fn latch_terminal_status_before_owner_release(
        &self,
        entry_id: &str,
    ) -> Result<(), DaemonError> {
        self.withheld_terminal_status().insert(entry_id.to_string());
        self.cache()?.remove(entry_id);
        #[cfg(any(test, feature = "test-hooks"))]
        self.pause_after_terminal_latch_if_armed(entry_id);
        Ok(())
    }

    #[cfg(feature = "v1-compat")]
    pub(super) fn finish_withholding_terminal_status(
        &self,
        entry_id: &str,
    ) -> Result<(), DaemonError> {
        let entry = self
            .inner
            .registry
            .get(entry_id)?
            .ok_or_else(|| DaemonError::Missing(format!("entry {entry_id}")))?;
        let invalidated = status::invalidate_terminal_views_with_root(
            &entry,
            &self.inner.config.config_dir,
            &self.inner.artifact_root,
        );
        // The current-entry pointer is a separate UI view. It names only live entries, but
        // refresh it after removing the cache so no later reader can reuse an old pointer
        // while this entry's API/status views are deliberately withheld.
        let current = self.refresh_current();
        match (invalidated, current) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    /// Revoke stale terminal views while the runtime/drain that observed the persistence
    /// failure still owns the exact artifact entry.  In particular, this must happen before
    /// returning that owner to its slot: the registry spelling is only diagnostic and may
    /// have been swapped to an attacker-controlled directory by then.
    #[cfg(feature = "v1-compat")]
    fn invalidate_terminal_status_with_artifact(
        &self,
        entry_id: &str,
        artifact: &goalcup_platform::tracecap::EntryArtifactRoot,
    ) -> Result<(), DaemonError> {
        let entry = self
            .inner
            .registry
            .get(entry_id)?
            .ok_or_else(|| DaemonError::Missing(format!("entry {entry_id}")))?;
        status::invalidate_terminal_views_with_artifact(
            &entry,
            &self.inner.config.config_dir,
            artifact,
        )
    }

    /// The filesystem removal already happened through a retained artifact capability.
    /// Refresh the unrelated in-memory current-entry pointer only after returning that
    /// owner to its slot, because `current_entry` takes the ordinary runtime lock.
    #[cfg(feature = "v1-compat")]
    fn finish_withholding_terminal_status_after_owner_release(&self) -> Result<(), DaemonError> {
        self.refresh_current()
    }

    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    fn pause_after_terminal_latch_if_armed(&self, entry_id: &str) {
        let mut state = self
            .inner
            .terminal_latch_pause
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if state.entry_id.as_deref() != Some(entry_id) {
            return;
        }
        state.paused = true;
        self.inner.terminal_latch_pause.wake.notify_all();
        while !state.released {
            state = self
                .inner
                .terminal_latch_pause
                .wake
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
        state.entry_id = None;
    }

    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn pause_after_terminal_latch_for_test(&self, entry_id: &str) {
        let mut state = self
            .inner
            .terminal_latch_pause
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        state.entry_id = Some(entry_id.to_string());
        state.paused = false;
        state.released = false;
    }

    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn wait_for_terminal_latch_pause_for_test(&self, entry_id: &str) {
        let mut state = self
            .inner
            .terminal_latch_pause
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while !(state.paused && state.entry_id.as_deref() == Some(entry_id)) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "terminal latch pause for {entry_id} was never reached"
            );
            let (next, _) = self
                .inner
                .terminal_latch_pause
                .wake
                .wait_timeout(state, remaining)
                .unwrap_or_else(|e| e.into_inner());
            state = next;
        }
    }

    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn release_terminal_latch_pause_for_test(&self) {
        let mut state = self
            .inner
            .terminal_latch_pause
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        state.released = true;
        self.inner.terminal_latch_pause.wake.notify_all();
    }

    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn terminal_status_withheld_for_test(&self, entry_id: &str) -> bool {
        self.withheld_terminal_status().contains(entry_id)
    }

    #[cfg(feature = "v1-compat")]
    fn withheld_terminal_status(&self) -> std::sync::MutexGuard<'_, BTreeSet<String>> {
        self.inner
            .withheld_terminal_status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Drop a sealed entry's runtime once the acknowledgement watermark has reached the
    /// chain head.
    ///
    /// **What a sealed entry still owns.** Its redb `Buffer`, its registry handle, every
    /// tailer file descriptor and its chain writer. Left in the map, a machine that ran many
    /// short runs under one long-lived daemon would accumulate one full set per sealed run and
    /// exhaust its descriptors.
    ///
    /// **The condition is the last upload, not the seal.** `EventBody::Seal` is the chain's
    /// last link, so a sealed entry's remaining work is exactly one thing: draining the
    /// buffer to the server, which is `pump_all` and which writes to `acks.jsonl` rather
    /// than to the chain. When the verified watermark equals the head there is nothing left
    /// to send, and an entry evicted before that would strand its own last chunk.
    ///
    /// **The registry row stays**, which is what makes this safe: `is_sealed` falls back to
    /// the row for an entry with no runtime, `resume_lock_paths` reads `SealOutcome` off
    /// it. What does change is the
    /// live map — `GET /v0/entries` and `Daemon::entries` walk it, so they list the entries
    /// this daemon still holds open and not every run that ever sealed. The seal manifest,
    /// the chain and the row are where a finished run is read.
    ///
    /// **Everything this process held in memory for the entry goes with it, except its
    /// status document**. The map, the
    /// workspace binding and the [`SealReport`] go: each grows with the run and each has a
    /// durable copy on disk, so an eviction that left them behind would still grow by one
    /// run's worth per sealed run, which is the whole reason this exists. The cached
    /// `SignedStatus` has neither property — one frozen document per sealed entry, bounded
    /// by the entry count rather than by the run — so it stays, and
    /// `GET /v0/entries/{id}/status` on an evicted entry still answers with the last
    /// document the run published rather than a `404`. See the note on the removals below.
    ///
    /// What a participant reads about a finished run beyond that document is the durable
    /// copy — `seal.json`, `chain.jsonl` and the registry row, which `SealOutcome` is
    /// written into precisely because this process may not be the one that answers next.
    #[cfg(feature = "v1-compat")]
    fn evict_if_drained(&self, entry_id: &str) -> Result<(), DaemonError> {
        // A failed terminal quarantine write is a stronger stop than the durable review
        // bit (which is absent by definition). The runtime must stay owned for a later
        // clean validation and completion publication; evicting it would strand both.
        if self.withheld_terminal_status().contains(entry_id) {
            return Ok(());
        }
        let drained = self.with_entry(entry_id, |r| {
            let entry = r.entry();
            // `Sealing` is capture-closed but not terminal: its live writer may be
            // poisoned and its intent still needs a restart recovery.  Evicting an
            // unstarted one merely because it has no upload watermark would make that
            // quarantine invisible and strand the only runtime/status owner.
            let sealed = entry.state == EntryState::Sealed || entry.sealed_at.is_some();
            // **A sealed entry that never started is drained too.** Nothing
            // uploads before the start receipt — the devserver refuses a chunk without one,
            // by design — so its acknowledgement watermark can never reach its chain head
            // and the test below it would never be true. The entry would then sit in
            // `goalcup entries` for the life of the daemon, published once a second, with
            // no run in it. Its status document says what it is:
            // "sealed, never started — nothing to upload".
            let never_started = entry.started_at.is_none();
            // `chain_seq` is `None` only on a chain with no links, which a sealed entry
            // never has: the seal itself is a link.
            sealed
                && !entry.terminal_drain_needs_review()
                && (never_started
                    || (entry.chain_seq.is_some() && entry.last_ack_seq == entry.chain_seq))
        });
        // `held_elsewhere` is not a failure here — the entry is mid-call and the next pump
        // asks again — and neither is an entry another caller already evicted.
        if !matches!(drained, Ok(true)) {
            return Ok(());
        }
        // **The last document this entry will ever have, published before it goes.**
        // The seal writes one too, but the seal is not the end of the numbers: the
        // chain's last link has still to be uploaded and acknowledged, and it is that
        // acknowledgement that brings this call about. Without this, a run that ended cleanly would
        // freeze on "233 uploaded, 1 pending" while `goalcup entries` already listed nothing — the
        // pending link is the seal's own, and no cadence reaches an evicted entry to say so.
        // Published here, the frozen document is the one the run finished on.
        //
        // Ignored on failure, like every other line of this method: the entry is drained and
        // the eviction is a cost, not a run to fail.
        if let Ok(slot) = self.slot(entry_id) {
            let _ = self.publish_one(entry_id, &slot);
        }
        self.entry_map()?.remove(entry_id);
        self.unbind_workspace(entry_id)?;
        self.seals()?.remove(entry_id);
        // **The signed status document stays.** Everything else this process held for the entry
        // goes — the runtime with its buffer, its tailers and its registry handle, the workspace
        // binding, the `SealReport` — because each of those grows with the run and has a durable
        // copy on disk. The cached document has neither property: it is one frozen `SignedStatus`
        // per sealed entry, so it is bounded by the entry count rather than by the run, and
        // removing it would make `GET /v0/entries/{id}/status` answer `404` for a run that had
        // just finished cleanly. A finished run's last document is exactly the answer that
        // question wants, and it is frozen: no cadence reaches an evicted entry to re-sign it.
        self.refresh_current()
    }

    /// Take this entry out of the workspace routing table, by the id it is filed under.
    ///
    /// **By value and not by a re-derived key.** The key is
    /// [`canonical`] of the workspace as it stood at registration, and `canonical` is
    /// `fs::canonicalize` with the written path as its fallback — so an entry whose
    /// workspace directory has since been deleted derives a different key here than the one
    /// it was inserted under, the removal misses, and the binding outlives the entry: the
    /// next `register` in that directory is refused as "already bound to entry X" naming an
    /// entry this daemon no longer holds, for the life of the process.
    ///
    /// One entry per workspace (D4), so this removes at most one row.
    #[cfg(feature = "v1-compat")]
    fn unbind_workspace(&self, entry_id: &str) -> Result<(), DaemonError> {
        self.workspaces()?.retain(|_, held| held != entry_id);
        Ok(())
    }

    /// Say where the server is and try again now, for one entry (the resync path).
    #[cfg(feature = "v1-compat")]
    pub fn resume_upload(&self, entry_id: &str) -> Result<(), DaemonError> {
        self.with_entry(entry_id, |r| r.resume_upload())
    }

    /// Re-sign every open entry's status document, mirror the two cache files the MCP
    /// server reads, and rewrite the one plain line the harness `cat`s (D25).
    /// **A sealed entry is still published, and stops being here by being evicted.** Its
    /// document still moves until its last chunk is acknowledged — the uploaded and pending
    /// counts — and [`Daemon::evict_if_drained`] is where a sealed entry leaves the map: an
    /// entry whose last chunk has been acknowledged is gone from it, and this loop then never
    /// sees it again.
    ///
    /// An entry whose runtime is out of its slot is skipped, which is the ordinary case
    /// while a start binding is in flight.
    #[cfg(feature = "v1-compat")]
    pub fn refresh_statuses(&self) -> Result<(), DaemonError> {
        let mut first: Option<DaemonError> = None;
        for entry_id in self.entry_ids()? {
            if self.is_borrowed(&entry_id) {
                continue;
            }
            if self.withheld_terminal_status().contains(&entry_id) {
                continue;
            }
            match self.with_entry(&entry_id, |r| {
                self.publish_live_status_unless_withheld(&entry_id, r)
            }) {
                Ok(Ok(_)) => {}
                Ok(Err(e)) | Err(e) => {
                    first.get_or_insert(e);
                }
            }
        }
        if let Err(e) = self.refresh_current() {
            first.get_or_insert(e);
        }
        self.finish(Cadence::Status, first)
    }

    /// Sign and publish one entry's status document, sealed or not, through a slot the
    /// caller is already holding.
    ///
    /// The seal's own path. [`Daemon::refresh_statuses`] publishes sealed entries too — see
    /// its own note — but it is a cadence, so the document that says a run is sealed would
    /// otherwise be written whenever the next status interval came round, and a participant
    /// reading in between would be told a closed run is still open. This is that document.
    /// Everything else about it is the ordinary publication — the two mirrored cache files,
    /// the plain status line, and the signed copy in the map the page reads.
    ///
    /// **The slot is a parameter and not a lookup.** `Daemon::slot` reads the
    /// live entry map, and `evict_if_drained` takes a drained sealed entry out of that map —
    /// which it may do while the seal is sleeping out a termination grace. Looking the slot up here
    /// would answer `Missing`, the seal would swallow it, and the last document a participant could
    /// read would predate the seal it was published to announce. The caller took the `Arc` before
    /// any of that could happen, and the runtime inside it is still this entry's.
    #[cfg(feature = "v1-compat")]
    pub(super) fn publish_one(&self, entry_id: &str, slot: &EntrySlot) -> Result<(), DaemonError> {
        {
            let mut held = slot.runtime.lock().unwrap_or_else(|e| e.into_inner());
            match held.as_mut() {
                Some(runtime) => {
                    self.publish_live_status_unless_withheld(entry_id, runtime)?;
                }
                None => return Err(held_elsewhere(entry_id)),
            }
        }
        self.refresh_current()
    }

    /// The four cadences, each on its own timer, until the process is stopped.
    ///
    /// Timers and not sleeps between calls: a tick that took 300 ms must not push the next
    /// one 300 ms later, and `MissedTickBehavior::Delay` is what keeps a daemon that fell
    /// behind from spending the rest of the run catching up on ticks nobody needs.
    ///
    /// Errors do not end the loop, and they are not dropped either. A daemon that
    /// exited on the first transient would take the capture with it, so each cadence counts
    /// its own failures and the last message rides out on the signed status document
    /// (`StatusPayload::cycles`) — the counting happens inside [`Daemon::tick`],
    /// [`Daemon::pump_all`], [`Daemon::refresh_statuses`] and [`Daemon::seal_due_entries`]
    /// rather than in the `select!` below, so a caller driving the daemon step by step
    /// leaves the same record a loop does. There is no logger: the daemon handles
    /// per-run bearers, and a component that can write a request to a log cannot promise it
    /// never wrote one.
    #[cfg(feature = "v1-compat")]
    pub async fn run_forever(&self) -> Result<(), DaemonError> {
        let mut poll = interval_of(self.inner.config.poll_interval_ms);
        let mut upload = interval_of(self.inner.config.upload_interval_ms);
        let mut status = interval_of(self.inner.config.status_interval_ms);
        // The deadline is a wall-clock instant days away; a second is as often as it is
        // worth asking, and it costs one comparison per open entry.
        let mut deadline = interval_of(1_000);
        loop {
            tokio::select! {
                // **`poll_all` and not `tick`.** The poll pass is synchronous;
                // what is not is the start receipt round trip, which carries a ten-second
                // timeout and which this loop would otherwise await — so with the receipt server
                // down nothing would be polled, uploaded, published or checked for the length
                // of the arming window. The bindings run on tasks of their own and
                // count their own failures, and this arm returns in the time a poll takes.
                _ = poll.tick() => { let _ = self.poll_all(); }
                _ = upload.tick() => { let _ = self.pump_all().await; }
                _ = status.tick() => { let _ = self.refresh_statuses(); }
                _ = deadline.tick() => { let _ = self.seal_due_entries(); }
            }
        }
    }

    /// A default build's loop: nothing to drive.
    ///
    /// A local capture entry owns its own work (ingest and control calls drive it), so there is
    /// no poll, upload, status or deadline cadence to run (D42). It never returns, which is the
    /// contract the foreground binary relies on: the only way out is a signal.
    #[cfg(not(feature = "v1-compat"))]
    pub async fn run_forever(&self) -> Result<(), DaemonError> {
        std::future::pending().await
    }

    /// Arm an entry: [`crate::foreign::pre_arm_scan`] first, then the gate.
    ///
    /// The scan is inside [`EntryRuntime::arm`], which is where the entry's own manifest,
    /// path variables and canonical workspace are — and it runs before the entry moves to
    /// `Armed`, so a refused arming leaves nothing changed.
    #[cfg(feature = "v1-compat")]
    pub fn arm(&self, entry_id: &str) -> Result<(), DaemonError> {
        self.with_entry(entry_id, |r| r.arm(now_unix()))?
    }

    /// At `deadline_at` the entry seals itself. No finish command, no
    /// participant, no CLI.
    ///
    /// The deadline is read from the registry, which is where `bind_start` stamped it once
    /// and where a daemon that restarted mid-run finds it. The runtime's copy is a
    /// mirror of that row.
    #[cfg(feature = "v1-compat")]
    pub fn seal_due_entries(&self) -> Result<Vec<String>, DaemonError> {
        // The deadline sweep's own pass, counted here because this cadence answers a
        // `Vec<String>` rather than going through `finish`.
        self.inner.cycles.ran();
        let out = self.due_entries();
        if let Err(e) = &out {
            self.inner.cycles.note(Cadence::Deadline, e);
        }
        out
    }

    /// **One entry's failure is that entry's.** A `?` on every step would let the first entry
    /// whose seal did not go through — a `held_elsewhere` while `pump_all` had it borrowed, a
    /// workspace walk error, a full disk — return before any later entry in the sorted map was
    /// even considered, once a second, silently, for the rest of the run. A deadline is the one
    /// ending nobody is present for, and an entry that never reached it because a neighbour was
    /// busy has no second chance.
    ///
    /// Failures are collected and the first is returned, which is the shape every other
    /// cadence uses: [`Daemon::seal_due_entries`] counts it and names it on the status
    /// document, and the entries that did seal are in the answer either way.
    #[cfg(feature = "v1-compat")]
    fn due_entries(&self) -> Result<Vec<String>, DaemonError> {
        let now = self.finalization_now_unix();
        let mut sealed = Vec::new();
        let mut first: Option<DaemonError> = None;
        for entry_id in self.entry_ids()? {
            match self.is_sealed(&entry_id) {
                Ok(true) => continue,
                Ok(false) => {}
                Err(e) => {
                    first.get_or_insert(e);
                    continue;
                }
            }
            let row = match self.inner.registry.get(&entry_id) {
                Ok(Some(row)) => row,
                Ok(None) => continue,
                Err(e) => {
                    first.get_or_insert(e);
                    continue;
                }
            };
            let workspace = matches!(
                row.receipt_binding(),
                crate::config::ReceiptBindingPolicy::WorkspaceSubmission { .. }
            );
            let due = if workspace {
                match self.inner.registry.observe_signed_deadline(&entry_id, now) {
                    Ok(row) => row.deadline_was_observed(),
                    Err(error) => {
                        first.get_or_insert(error);
                        continue;
                    }
                }
            } else {
                row.deadline_unix.is_some_and(|deadline| now >= deadline)
            };
            if !due {
                continue;
            }
            let result = if workspace {
                self.finish_entry(
                    &entry_id,
                    goalcup_protocol::submission::FinishReasonV1::Deadline,
                )
                .map(|_| ())
            } else {
                self.seal_entry(&entry_id).map(|_| ())
            };
            match result {
                Ok(_) => sealed.push(entry_id),
                Err(e) => {
                    first.get_or_insert(e);
                }
            }
        }
        match first {
            Some(e) => Err(e),
            None => Ok(sealed),
        }
    }

    // -------------------------------------------------------------------------------
    // The accessors the control API calls. Written here, in the file that owns the private
    // maps they read.
    // -------------------------------------------------------------------------------

    pub fn config(&self) -> &DaemonConfig {
        &self.inner.config
    }

    /// The shipped connector manifests this daemon loaded, in id order.
    pub fn manifests(&self) -> Vec<ConnectorManifest> {
        self.loaded_manifests().values().cloned().collect()
    }

    /// The map as it stands, without touching the disk.
    fn loaded_manifests(&self) -> Arc<BTreeMap<String, ConnectorManifest>> {
        self.trusted_manifests().manifests_arc()
    }

    fn trusted_manifests(&self) -> Arc<TrustedManifests> {
        self.inner
            .manifests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    #[cfg(feature = "v1-compat")]
    pub(crate) fn foreign_plan_for(&self, manifest: &ConnectorManifest) -> ForeignScanPlan {
        self.trusted_manifests().plan_for(manifest)
    }

    /// Re-read `connectors/` and answer what is there now.
    ///
    /// A map loaded once at startup would leave a connector edited on disk invisible to a
    /// running daemon: `create_workspace` would compare the CLI's digest against a manifest
    /// this process had read minutes earlier and refuse the start, naming the wrong cause.
    ///
    /// Called by `create_workspace`, which is the one moment a manifest is chosen — a read
    /// of a few small files, once per run, against a decision the run is then judged by.
    /// The nearest cadence never reaches it. A directory whose `manifest.toml` does not
    /// parse fails here, naming the file and the parse error.
    pub fn reload_manifests(
        &self,
    ) -> Result<Arc<BTreeMap<String, ConnectorManifest>>, DaemonError> {
        let fresh = Arc::new(TrustedManifests::load(&self.inner.config.connectors_dir)?);
        let manifests = fresh.manifests_arc();
        *self
            .inner
            .manifests
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = fresh.clone();
        Ok(manifests)
    }

    /// The page's state: the signed cache per entry, and which entry a caller with no id
    /// gets. Cloned rather than borrowed — every field is an `Arc`, and the control router
    /// merges `ui_router` with it.
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn ui_state(&self) -> UiState {
        self.inner.ui.clone()
    }

    /// Where the arming gate is, and when it saw the declared signal. `None` for an entry
    /// whose gate has been refused — a refusal is not a wire state.
    #[cfg(feature = "v1-compat")]
    pub fn arming_of(
        &self,
        entry_id: &str,
    ) -> Result<Option<(GateState, Option<f64>)>, DaemonError> {
        self.with_entry(entry_id, |r| match r.gate_state() {
            GateState::Refused(_) => None,
            state => Some((state, r.start_signal_at())),
        })
    }

    /// `leg1` or `leg2`: which leg witnessed the start signal. Empty while none has.
    #[cfg(feature = "v1-compat")]
    pub fn start_signal_source(&self, entry_id: &str) -> Result<String, DaemonError> {
        self.with_entry(entry_id, |r| {
            r.signal_source().unwrap_or_default().to_string()
        })
    }

    /// The connector's `arming.signal`, echoed back for `StartSignal.signal`.
    #[cfg(feature = "v1-compat")]
    pub fn arming_signal(&self, entry_id: &str) -> Result<String, DaemonError> {
        self.with_entry(entry_id, |r| r.manifest().arming.signal.clone())
    }

    #[cfg(feature = "v1-compat")]
    pub fn session_id(&self, entry_id: &str) -> Result<Option<String>, DaemonError> {
        self.with_entry(entry_id, |r| r.bound_session())
    }

    /// The on-screen text: what the participant is waiting for, in the connector's own
    /// words, and what to do about it.
    #[cfg(feature = "v1-compat")]
    pub fn on_screen(&self, entry_id: &str) -> Result<String, DaemonError> {
        self.with_entry(entry_id, |r| r.on_screen())
    }

    /// The signed status document, as the last publication left it. The page and
    /// `goalcup status` read this one; nothing signs a second copy on the way out.
    ///
    /// **And the on-disk copy when this process has none.** The in-memory
    /// cache is a map this process filled, so a daemon that restarted after a run sealed
    /// would answer `404 not found: no status for that entry` about a run whose signed document
    /// is sitting in `<config_dir>/cache/status-<id>.json`. `status::mirror`
    /// wrote that file and this reads it back: same bytes, same signature, and the caller
    /// verifies it against the run key it pinned either way.
    ///
    /// The document is parsed and its `entry_id` compared, so a file that was replaced by
    /// another run's document answers about nobody. An unreadable or unparsable file is
    /// `None` — the honest "there is no document here" — rather than an error about a
    /// cache a caller never asked about. A terminal-drain refusal is durable registry
    /// authority over both of those retrying views: an older `Sealed` zero-pending document
    /// is rejected rather than returned while publication is recovering.
    pub fn signed_status(&self, entry_id: &str) -> Result<Option<SignedStatus>, DaemonError> {
        if let Some(signed) = self.capture_status(entry_id)? {
            return Ok(Some(signed));
        }
        // A default build serves capture entries alone (D42); the cache below is the V1 path's.
        #[cfg(not(feature = "v1-compat"))]
        return Ok(None);
        #[cfg(feature = "v1-compat")]
        self.signed_v1_status(entry_id)
    }

    /// The V1 half of [`Daemon::signed_status`]: the in-memory cache and the on-disk mirror.
    #[cfg(feature = "v1-compat")]
    fn signed_v1_status(&self, entry_id: &str) -> Result<Option<SignedStatus>, DaemonError> {
        if self.withheld_terminal_status().contains(entry_id)
            || self
                .inner
                .withheld_rehydration_status
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .contains(entry_id)
        {
            return Ok(None);
        }
        let terminal_review = self
            .inner
            .registry
            .get(entry_id)?
            .is_some_and(|entry| entry.terminal_drain_needs_review());
        let signed = self
            .cache()?
            .get(entry_id)
            .cloned()
            .or_else(|| cached_status_on_disk(&self.inner.config.config_dir, entry_id));
        Ok(signed.filter(|signed| {
            !terminal_review
                || (signed.payload.health
                    == goalcup_protocol::events::HealthState::SealingRecoveryNeeded
                    && signed.payload.events_pending > 0)
        }))
    }

    /// Catch the tail up, then say what a relaunch needs.
    ///
    /// The poll comes first and its result is in the answer: a resume that reported the
    /// paths without reading what the harness wrote before it died would relaunch over an
    /// un-captured tail, and those records are the ones a kill is most likely to have
    /// stranded.
    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn catch_up(&self, entry_id: &str) -> Result<ResumeInfo, DaemonError> {
        let artifact_root = self.inner.config.artifact_root.clone();
        self.with_entry(entry_id, |r| {
            let caught_up_events = r.poll_once_at(now_unix())?.chained;
            let entry = r.entry();
            Ok::<_, DaemonError>(ResumeInfo {
                entry_id: entry.entry_id.clone(),
                process_generation: entry.process_handoff_generation(),
                session_id: r.bound_session().unwrap_or_default(),
                caught_up_events,
                workspace: entry.workspace.to_string_lossy().to_string(),
                artifact_root: entry.artifact_dir.to_string_lossy().to_string(),
                harness_home: entry
                    .harness_home
                    .as_ref()
                    .map(|h| h.to_string_lossy().to_string())
                    .unwrap_or_default(),
                status_line_path: status::canonical_status_line_path(
                    &entry.entry_id,
                    &artifact_root,
                )?
                .to_string_lossy()
                .to_string(),
                server_port: entry.server_port,
                last_process: entry.last_process.clone(),
            })
        })?
    }

    /// Catch a committed run's tail up and then consume exactly one registered process
    /// generation while holding its runtime slot.  The durable transition is deliberately
    /// last: a capture error leaves the old registered handle and generation usable; once
    /// the CAS lands, a lost response is intentionally fail-closed in `OpenResume`.
    ///
    /// `Registry::open_resume_process_handoff` repeats the Started/session/generation
    /// checks in its write transaction.  The slot makes the poll and that transaction one
    /// lifecycle boundary with respect to seal and process registration.
    #[cfg(feature = "v1-compat")]
    pub fn catch_up_then_open_process_handoff(
        &self,
        entry_id: &str,
        expected_generation: u64,
    ) -> Result<ResumeInfo, DaemonError> {
        let artifact_root = self.inner.config.artifact_root.clone();
        self.with_entry(entry_id, |r| {
            let caught_up_events = r.poll_once_at(now_unix())?.chained;
            let previous = r.entry().last_process.clone();
            let opened = r.clear_process(expected_generation)?;
            Ok::<_, DaemonError>(ResumeInfo {
                entry_id: opened.entry_id.clone(),
                process_generation: opened.process_handoff_generation(),
                session_id: r.bound_session().unwrap_or_default(),
                caught_up_events,
                workspace: opened.workspace.to_string_lossy().to_string(),
                artifact_root: opened.artifact_dir.to_string_lossy().to_string(),
                harness_home: opened
                    .harness_home
                    .as_ref()
                    .map(|h| h.to_string_lossy().to_string())
                    .unwrap_or_default(),
                status_line_path: status::canonical_status_line_path(
                    &opened.entry_id,
                    &artifact_root,
                )?
                .to_string_lossy()
                .to_string(),
                server_port: opened.server_port,
                last_process: previous,
            })
        })?
    }

    /// Read the immutable pins needed to render a resume launch line.
    ///
    /// This deliberately does not call [`EntryRuntime::poll_once_at`] and does not open the
    /// process slot. `resume --print-launch` is a preview: capture and registry state must
    /// remain exactly as they were before its authenticated GET arrived.
    #[cfg(feature = "v1-compat")]
    pub fn resume_preview(&self, entry_id: &str) -> Result<ResumePreviewInfo, DaemonError> {
        let artifact_root = self.inner.config.artifact_root.clone();
        self.with_entry(entry_id, |r| {
            let entry = r.entry();
            Ok::<_, DaemonError>(ResumePreviewInfo {
                entry_id: entry.entry_id.clone(),
                process_generation: entry.process_handoff_generation(),
                process_handoff_phase: entry.process_handoff_phase().as_wire().to_string(),
                session_id: r.bound_session().unwrap_or_default(),
                workspace: entry.workspace.to_string_lossy().to_string(),
                artifact_root: entry.artifact_dir.to_string_lossy().to_string(),
                harness_home: entry
                    .harness_home
                    .as_ref()
                    .map(|h| h.to_string_lossy().to_string())
                    .unwrap_or_default(),
                status_line_path: status::canonical_status_line_path(
                    &entry.entry_id,
                    &artifact_root,
                )?
                .to_string_lossy()
                .to_string(),
                server_port: entry.server_port,
            })
        })?
    }

    /// Drop an entry that never started: failure before dispatch leaves no entry.
    ///
    /// Refused once `started_at` is stamped: a started run has a signed receipt, a chained
    /// baseline and a deadline the server witnessed, and none of that is the daemon's to
    /// withdraw. That run ends by sealing.
    /// **The check and the removal are one write transaction.** Reading `started_at` off the
    /// runtime's in-memory mirror and taking more locks before the row went would leave a window:
    /// [`Daemon::bind_start`] holds the runtime out of its slot for a receipt round trip of up to
    /// `NETWORK_TIMEOUT`, and a start committed inside that window would take the row with it,
    /// after which the control API's `remove_entry_files` would `remove_dir_all` the artifact
    /// directory — `chain.jsonl`, `baseline.json` and `receipt.json` of a run with a signed server
    /// receipt.
    ///
    /// Two things close it, in this order. The slot lock is taken and the runtime lifted
    /// out of it before anything else, so a `bind_start` that has not started yet meets an
    /// empty slot and a `bind_start` already in flight is what this call meets — refused as
    /// `held_elsewhere`, because a start may be committing right now. And
    /// [`Registry::begin_unstarted_cleanup`] re-asks the durable row while atomically moving
    /// it into `CleanupPending`, which is the check no race can pass twice.
    ///
    /// The quarantine is committed before recursive removal. A crash or late unlink failure
    /// therefore leaves a row that says cleanup debt—not a live runtime pointing at a partly
    /// removed tree—and startup retries that exact retained artifact authority.
    pub fn abandon(&self, entry_id: &str, reason: &str) -> Result<(), DaemonError> {
        if self.has_capture_entry(entry_id) {
            return self.abandon_capture(entry_id);
        }
        #[cfg(feature = "v1-compat")]
        return self.abandon_v1(entry_id, reason);
        #[cfg(not(feature = "v1-compat"))]
        {
            let _ = reason;
            Err(no_such_entry(entry_id))
        }
    }

    /// The V1 half of [`Daemon::abandon`]: a durable cleanup quarantine, then the runtime's
    /// artifact removal.
    #[cfg(feature = "v1-compat")]
    fn abandon_v1(&self, entry_id: &str, reason: &str) -> Result<(), DaemonError> {
        if self
            .inner
            .registry
            .get(entry_id)?
            .is_some_and(|entry| entry.state == EntryState::CleanupPending)
        {
            let result = self.finish_pending_cleanup(entry_id);
            if result.is_ok() {
                self.entry_map()?.remove(entry_id);
                self.unbind_workspace(entry_id)?;
                self.seals()?.remove(entry_id);
                self.cache()?.remove(entry_id);
                self.refresh_current()?;
            }
            return result.map_err(|error| match error {
                DaemonError::Refused(why) => DaemonError::Refused(format!("{why} ({reason})")),
                other => other,
            });
        }
        let slot = self.slot(entry_id)?;
        // Out of the slot for the length of this call: a `bind_start` already in flight has
        // it, and a start may be committing inside that very call, so an empty slot here is
        // a refusal and not a wait.
        let runtime = slot
            .runtime
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or_else(|| held_elsewhere(entry_id))?;
        let target = match self.inner.registry.begin_unstarted_cleanup(entry_id) {
            Ok(target) => target,
            Err(error) => {
                slot.return_runtime(runtime);
                return Err(match error {
                    DaemonError::Refused(why) => DaemonError::Refused(format!("{why} ({reason})")),
                    other => other,
                });
            }
        };
        if let Some(target) = &target
            && !self
                .inner
                .registry
                .unstarted_termination_complete(entry_id)?
        {
            if let Err(error) = end_harness_verified(target.handle()) {
                // The durable cleanup quarantine is intentional: the target was accepted
                // but could not be proved gone, so a retry/restart must fail closed rather
                // than reopen ordinary capture or forget the authority.
                return Err(match error {
                    DaemonError::Refused(why) => DaemonError::Refused(format!("{why} ({reason})")),
                    other => other,
                });
            }
            self.inner
                .registry
                .mark_unstarted_termination_complete(entry_id, target)?;
        }

        // From this durable point onward the runtime is never restored. Remove every
        // ordinary in-process route before process/artifact cleanup can fail late.
        self.entry_map()?.remove(entry_id);
        self.unbind_workspace(entry_id)?;
        self.seals()?.remove(entry_id);
        self.cache()?.remove(entry_id);
        let removal = (|| {
            runtime.artifact().remove_recursively().map_err(|error| {
                DaemonError::Refused(format!(
                    "entry {entry_id} artifact cleanup through its retained authority failed; \
                     the cleanup-pending registry row is retained: {error}"
                ))
            })?;
            self.inner
                .registry
                .complete_unstarted_cleanup(entry_id, target.as_ref())
        })();
        if let Err(e) = removal {
            return Err(match e {
                DaemonError::Refused(why) => DaemonError::Refused(format!("{why} ({reason})")),
                other => other,
            });
        }
        // The row is gone and the slot is empty, so nothing can reach this runtime again;
        // dropping it here releases the buffer's file lock before the maps are touched.
        drop(runtime);
        self.refresh_current()
    }

    /// The rows `goalcup entries` prints, in entry-id order.
    pub fn entries(&self) -> Result<Vec<EntrySummary>, DaemonError> {
        let mut out = Vec::new();
        #[cfg(feature = "v1-compat")]
        {
            out.extend(self.v1_entry_summaries()?);
        }
        out.extend(self.capture_summaries()?);
        out.sort_by(|left, right| left.entry_id.cmp(&right.entry_id));
        Ok(out)
    }

    /// The V1 rows of [`Daemon::entries`]: every live runtime, and every retained terminal
    /// owner or unreconciled sealing intent that has none.
    #[cfg(feature = "v1-compat")]
    fn v1_entry_summaries(&self) -> Result<Vec<EntrySummary>, DaemonError> {
        let mut out = Vec::new();
        let mut represented = BTreeSet::new();
        let withheld = self.withheld_terminal_status().clone();
        for entry_id in self.entry_ids()? {
            represented.insert(entry_id.clone());
            out.push(self.with_entry(&entry_id, |r| {
                let entry = r.entry();
                let health = if withheld.contains(&entry.entry_id)
                    || (entry.state == EntryState::Sealing && entry.sealed_at.is_none())
                {
                    goalcup_protocol::events::HealthState::SealingRecoveryNeeded
                } else {
                    entry.health
                };
                status::summary(entry, r.gate_state(), health)
            })?);
        }
        // A retained terminal owner and an unreconciled sealing intent can have no live
        // runtime.  They nevertheless remain an authenticated diagnostic: a failed
        // terminal marker must never make `/entries` call a sealed capture healthy simply
        // because the runtime was evicted or is intentionally not rebuilt.
        for entry in self.inner.registry.all()? {
            if !represented.contains(&entry.entry_id)
                && (entry.state == EntryState::Sealing || withheld.contains(&entry.entry_id))
            {
                out.push(status::summary(
                    &entry,
                    match (entry.started_at.is_some(), entry.start_signal_at.is_some()) {
                        (true, _) => GateState::Started,
                        (false, true) => GateState::StartSignalSeen,
                        (false, false) => GateState::Armed,
                    },
                    goalcup_protocol::events::HealthState::SealingRecoveryNeeded,
                ));
            }
        }
        Ok(out)
    }

    /// The single entry a caller with no id means: the one open run that is not sealed.
    /// `None` when there is none, and `None` when there is more than one — a daemon holding
    /// two live runs must not guess which one a bare `goalcup status` is about.
    pub fn current_entry(&self) -> Result<Option<String>, DaemonError> {
        #[cfg_attr(not(feature = "v1-compat"), allow(unused_mut))]
        let mut live = Vec::new();
        #[cfg(feature = "v1-compat")]
        for entry_id in self.entry_ids()? {
            if !self.is_sealed(&entry_id)? {
                live.push(entry_id);
            }
        }
        live.extend(
            self.capture_summaries()?
                .into_iter()
                .filter(|summary| summary.sealed_at.is_none())
                .map(|summary| summary.entry_id),
        );
        Ok(match live.len() {
            1 => live.pop(),
            _ => None,
        })
    }

    /// Record the process the CLI launched, so `harness_alive` has something to measure and
    /// the seal has something to end (D22, D9).
    ///
    /// **Refused while a handle stands, and for exactly as long as one does.** The handle is
    /// what `same_process` measures and what the seal terminates, so a second one posted on
    /// the loopback is a decoy that points both at a process the launch never made:
    /// `harness_alive` would then report the decoy's health and the seal would kill the
    /// decoy's group. The registered handle wins and everything else is a `409`.
    ///
    /// **It also chains what the launch's `[home_seed]` carried**, because
    /// this is the one call a launch makes exactly once — a start makes it, and a resume
    /// makes it after `clear_process` has opened the slot. The record is the CLI's
    /// measurement, written down and not interpreted (D3).
    ///
    /// It does not refuse after `started_at`: a resume has the CLI relaunch a harness on a run that
    /// started days ago, and the relaunched process has to be registered or the rest of the run
    /// would measure a pid that was gone and the seal would terminate nothing. The slot is
    /// opened by one thing —
    /// [`EntryRuntime::clear_process`], called by the resume route after its checks pass —
    /// so a cleared handle only ever comes from a resume, and the standing handle is the
    /// whole of what this refusal protects.
    pub fn register_process(
        &self,
        entry_id: &str,
        registration: ProcessRegistration,
    ) -> Result<(), DaemonError> {
        if self.has_capture_entry(entry_id) {
            return self.register_capture_process(entry_id, registration);
        }
        #[cfg(feature = "v1-compat")]
        return self.with_entry(entry_id, |r| r.set_process(registration))?;
        #[cfg(not(feature = "v1-compat"))]
        Err(no_such_entry(entry_id))
    }

    /// The native process identity the entry's already-loaded, signed connector manifest
    /// permits the CLI to bind into its process handoff. This is read through the runtime
    /// rather than reloading a connector by caller-supplied id: the launch must be bound to
    /// the same manifest the entry was created under.
    pub fn native_binary_for_process(&self, entry_id: &str) -> Result<Option<String>, DaemonError> {
        if self.has_capture_entry(entry_id) {
            // A sealed capture entry admits no process, and `capture_entry` refuses it.
            return self
                .capture_entry(entry_id)
                .map(|capture| capture.native_binary.clone());
        }
        #[cfg(feature = "v1-compat")]
        return self.with_entry(entry_id, |runtime| {
            Ok::<_, DaemonError>(runtime.manifest().kill.native_binary.clone())
        })?;
        #[cfg(not(feature = "v1-compat"))]
        Err(no_such_entry(entry_id))
    }

    /// Return an issued resume permit before a replacement process was accepted.  The
    /// generation remains advanced and the previous registered process remains the seal
    /// target; a later resume must use this generation and mint the next one.
    #[cfg(feature = "v1-compat")]
    pub fn cancel_resume_process_handoff(
        &self,
        entry_id: &str,
        generation: u64,
        request_digest: Option<&str>,
    ) -> Result<Entry, DaemonError> {
        self.with_entry(entry_id, |runtime| {
            runtime.cancel_resume_process_handoff(generation, request_digest)
        })?
    }

    /// Read-only acknowledgement for an exact `/process` replay whose entry was sealed and
    /// evicted after the original accepted response was lost.  This is intentionally not a
    /// general registry registration path: live and unsealed entries must retain the runtime
    /// slot's ordering with trace and encrypted-buffer reconciliation.
    pub fn acknowledge_evicted_sealed_process_replay(
        &self,
        entry_id: &str,
        registration: &ProcessRegistration,
    ) -> Result<(), DaemonError> {
        self.inner
            .registry
            .acknowledge_sealed_process_replay(entry_id, registration)
    }

    /// Forget the handle this entry registered, so a relaunch can register its own.
    ///
    /// Called by the resume route once its refusals have passed, and by nothing else — see
    /// [`EntryRuntime::clear_process`] for why that matters.
    #[cfg(feature = "v1-compat")]
    pub fn clear_process(
        &self,
        entry_id: &str,
        expected_generation: u64,
    ) -> Result<Entry, DaemonError> {
        self.with_entry(entry_id, |r| r.clear_process(expected_generation))?
    }

    /// Refuse an unresumable `attach_only` entry before `catch_up` can mutate capture
    /// state. The runtime repeats the check immediately before its durable process-slot
    /// transition.
    #[cfg(feature = "v1-compat")]
    pub fn ensure_resume_can_launch(&self, entry_id: &str) -> Result<(), DaemonError> {
        self.with_entry(entry_id, |r| r.ensure_resume_can_launch())?
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.inner.started.elapsed().as_secs()
    }

    /// Whether every open entry's last pump reached the server. Measured, not assumed: an
    /// entry that has not pumped yet has contradicted nothing and reports `true`.
    pub fn upload_ok(&self) -> bool {
        #[cfg(feature = "v1-compat")]
        {
            let Ok(ids) = self.entry_ids() else {
                return false;
            };
            ids.iter()
                .all(|id| self.with_entry(id, |r| r.connected()).unwrap_or(true))
        }
        // No upload exists in a default build (D42), so none is failing.
        #[cfg(not(feature = "v1-compat"))]
        true
    }

    // -------------------------------------------------------------------------------
    // The map, and the one place each lock is taken.
    // -------------------------------------------------------------------------------

    #[cfg(feature = "v1-compat")]
    fn entry_ids(&self) -> Result<Vec<String>, DaemonError> {
        Ok(self.entry_map()?.keys().cloned().collect())
    }

    #[cfg(feature = "v1-compat")]
    pub(super) fn slot(&self, entry_id: &str) -> Result<Arc<EntrySlot>, DaemonError> {
        let slot = { self.entry_map()?.get(entry_id).cloned() };
        if let Some(slot) = slot {
            return Ok(slot);
        }
        match self.inner.registry.get(entry_id)? {
            Some(entry) if entry.state == EntryState::Refused => Err(terminal_refusal(entry_id)),
            Some(entry) if entry.state == EntryState::CleanupPending => {
                Err(cleanup_pending_refusal(entry_id))
            }
            _ => Err(DaemonError::Missing(format!(
                "no entry {entry_id} in this daemon"
            ))),
        }
    }

    /// This daemon's admission boundary, for the finalizer that owns the entry and for
    /// [`Daemon::register`]. It is not a process-wide `static`, which would make two daemons
    /// in one process serialise each other's finalizations and put the boundary within
    /// reach of every module in the crate.
    #[cfg(feature = "v1-compat")]
    pub(super) fn admission_guard(&self) -> std::sync::MutexGuard<'_, ()> {
        super::admission_guard(&self.inner.admission)
    }

    /// Reach one entry's runtime. The lock is this daemon's only synchronous access to a
    /// runtime, and it is never held across an `await`.
    #[cfg(feature = "v1-compat")]
    pub(super) fn with_entry<T>(
        &self,
        entry_id: &str,
        f: impl FnOnce(&mut EntryRuntime) -> T,
    ) -> Result<T, DaemonError> {
        let slot = self.slot(entry_id)?;
        let mut held = slot.runtime.lock().unwrap_or_else(|e| e.into_inner());
        match held.as_mut() {
            Some(runtime) => Ok(f(runtime)),
            None => Err(held_elsewhere(entry_id)),
        }
    }

    #[cfg(feature = "v1-compat")]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn out_of_window_stats_for_test(&self, entry_id: &str) -> Result<(u64, u64), DaemonError> {
        if let Ok(stats) =
            self.with_entry(entry_id, |runtime| runtime.out_of_window_stats_for_test())
        {
            return stats;
        }
        let entry = self
            .inner
            .registry
            .get(entry_id)?
            .ok_or_else(|| DaemonError::Missing(format!("entry {entry_id}")))?;
        let slot = self.sealed_drain_slot(entry)?;
        let held = slot.drain.lock().unwrap_or_else(|e| e.into_inner());
        held.as_ref()
            .ok_or_else(|| held_elsewhere(entry_id))?
            .out_of_window_stats_for_test()
    }

    /// Whether this entry's runtime is currently out of its slot for an asynchronous call.
    ///
    /// A peek and not a claim: the caller may still lose the race to the borrow it is about
    /// to attempt, and then it meets the ordinary refusal. What this exists for is the
    /// steady state — a start binding on its own task for the length of a receipt timeout —
    /// where the ordinary refusal would be a cycle error per cadence about a healthy run.
    ///
    /// An entry that is not in the map at all is not borrowed; it is gone, and the caller's
    /// own lookup says so.
    #[cfg(feature = "v1-compat")]
    pub fn is_borrowed(&self, entry_id: &str) -> bool {
        match self.slot(entry_id) {
            Ok(slot) => slot
                .runtime
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_none(),
            Err(_) => false,
        }
    }

    /// Borrow the runtime for a caller that has to `await` while holding it.
    ///
    /// The runtime comes out of the map for the length of the call, because it is not
    /// `Sync` and a `std::sync::MutexGuard` held across an `await` point is a deadlock
    /// waiting for the runtime to schedule the other half. [`Borrowed`] puts it back on
    /// drop, so a cancelled future — a `run_forever` losing a `select!` race, a shutdown
    /// signal mid-pump — returns the entry rather than stranding it.
    #[cfg(feature = "v1-compat")]
    pub fn borrow_entry(&self, entry_id: &str) -> Result<Borrowed, DaemonError> {
        let slot = self.slot(entry_id)?;
        let taken = slot
            .runtime
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        match taken {
            Some(runtime) => Ok(Borrowed {
                slot,
                runtime: Some(runtime),
            }),
            // The same condition `held_elsewhere` names, and the same answer: transient,
            // and the one code a client may retry on.
            None => Err(DaemonError::Busy(format!(
                "entry {entry_id} is already held by another asynchronous call"
            ))),
        }
    }

    /// One cadence's answer, with the pass counted on the way out and its failure counted
    /// beside it.
    #[cfg(feature = "v1-compat")]
    fn finish(&self, cadence: Cadence, first: Option<DaemonError>) -> Result<(), DaemonError> {
        self.inner.cycles.ran();
        match first {
            Some(e) => {
                self.inner.cycles.note(cadence, &e);
                Err(e)
            }
            None => Ok(()),
        }
    }

    /// What the daemon's four timers have lost, as the status document carries it.
    pub fn cycle_errors(&self) -> CycleErrors {
        self.inner.cycles.snapshot()
    }

    #[cfg(feature = "v1-compat")]
    pub(super) fn refresh_current(&self) -> Result<(), DaemonError> {
        let current = self.current_entry()?;
        *self
            .inner
            .ui
            .current
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = current;
        Ok(())
    }

    #[cfg(feature = "v1-compat")]
    #[allow(clippy::type_complexity)]
    pub(super) fn entry_map(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, Arc<EntrySlot>>>, DaemonError> {
        Ok(self.inner.entries.lock().unwrap_or_else(|e| e.into_inner()))
    }

    #[cfg(feature = "v1-compat")]
    fn workspaces(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BTreeMap<PathBuf, String>>, DaemonError> {
        Ok(self
            .inner
            .workspaces
            .lock()
            .unwrap_or_else(|e| e.into_inner()))
    }

    #[cfg(feature = "v1-compat")]
    pub(super) fn cache(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, SignedStatus>>, DaemonError> {
        Ok(self
            .inner
            .ui
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner()))
    }
}

/// The refusal for an entry id this daemon does not hold. A default build keeps no V1 runtime
/// map, so every id that is not a local capture lands here.
/// Whether a V1 row still has work only the V1 runtime can do (D42): it is not sealed, its
/// terminal drain needs review, its acknowledgement watermark is short of the chain head, or a
/// sealed harness root is still alive and awaits the reaper. A refused row (never admitted) and a
/// sealed, drained, reaped row owe nothing.
#[cfg(not(feature = "v1-compat"))]
fn v1_row_owes_work(entry: &crate::registry::Entry) -> bool {
    use crate::registry::EntryState;
    match entry.state {
        EntryState::Refused => return false,
        EntryState::Sealed => {}
        _ => return true,
    }
    if entry.terminal_drain_needs_review() || entry.last_ack_seq != entry.chain_seq {
        return true;
    }
    #[cfg(unix)]
    if let Some(handle) = &entry.last_process {
        // "Could not ask" is not "gone": keep the row owing.
        return goalcup_protocol::process::liveness(handle.pid).unwrap_or(true);
    }
    false
}

#[cfg(not(feature = "v1-compat"))]
fn no_such_entry(entry_id: &str) -> DaemonError {
    DaemonError::Missing(format!("no entry {entry_id} in this daemon"))
}

fn loopback(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

#[cfg(feature = "v1-compat")]
fn interval_of(millis: u64) -> tokio::time::Interval {
    let mut timer = tokio::time::interval(std::time::Duration::from_millis(millis.max(1)));
    // A daemon that fell behind must not spend the next second catching up on ticks nobody
    // needs; the work each one does is the same either way.
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    timer
}

/// The signed status document `status::mirror` last wrote for one entry, or `None`.
///
/// The one reader of the cache file inside the daemon. The id becomes a file
/// name, so it is checked the way `status::mirror` checks it before writing: a value that is
/// not a plain file name names no file this daemon wrote, and nothing is opened for it.
#[cfg(feature = "v1-compat")]
pub(crate) fn cached_status_on_disk(
    config_dir: &std::path::Path,
    entry_id: &str,
) -> Option<SignedStatus> {
    if std::path::Path::new(entry_id).file_name() != Some(std::ffi::OsStr::new(entry_id)) {
        return None;
    }
    let path = config_dir
        .join("cache")
        .join(format!("status-{entry_id}.json"));
    let text = std::fs::read_to_string(path).ok()?;
    let signed: SignedStatus = serde_json::from_str(&text).ok()?;
    (signed.payload.entry_id == entry_id).then_some(signed)
}

/// Remove the `*.tmp` files an atomic write left behind when the process died between the
/// write and the rename.
///
/// A missing directory is not an error: the cache is created by the first status write.
fn sweep_stale_temp_files(dir: &std::path::Path) -> Result<(), DaemonError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(DaemonError::Io {
                path: dir.display().to_string(),
                source,
            });
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "tmp") {
            // Best effort: another daemon's live temp file is its business, and a sweep
            // that refused to start over one would be a daemon that cannot start twice.
            let _ = std::fs::remove_file(&path);
        }
    }
    Ok(())
}

/// Create one participant-selected daemon root privately, or leave an existing one alone.
///
/// The leaf `mkdir` is the created/existed decision: there is no `exists` check a concurrent
/// creator could race.  A new leaf asks for 0700 on that same operation, is opened without
/// following a link, and is normalized through that retained descriptor in case the process has
/// a more restrictive umask.  An existing directory is participant-owned policy, so wider bits
/// are reported but never changed or refused.
#[cfg(unix)]
fn create_configured_root(path: &std::path::Path, role: &str) -> Result<(), DaemonError> {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

    let io_at_root = |source| DaemonError::Io {
        path: path.display().to_string(),
        source,
    };

    if let Some(parent) = path.parent() {
        create_dir(parent).map_err(|error| match error {
            DaemonError::Io { source, .. } => io_at_root(source),
            other => other,
        })?;
    }

    let mut builder = std::fs::DirBuilder::new();
    builder.mode(0o700);
    match builder.create(path) {
        Ok(()) => {
            #[cfg(any(test, feature = "test-hooks"))]
            replace_created_root_with_symlink_for_test(path)?;
            let directory = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)
                .map_err(io_at_root)?;
            // SAFETY: `directory` owns a live descriptor opened as a directory without
            // following the configured leaf, and 0700 is a valid POSIX permission mode.
            if unsafe { libc::fchmod(directory.as_raw_fd(), 0o700) } == -1 {
                return Err(io_at_root(std::io::Error::last_os_error()));
            }
            Ok(())
        }
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = std::fs::metadata(path).map_err(io_at_root)?;
            if !metadata.is_dir() {
                return Err(io_at_root(source));
            }
            let mode = metadata.permissions().mode() & 0o7777;
            if mode & 0o077 != 0 {
                eprintln!(
                    "goalcup-daemon: permission warning: {role} {} has mode {mode:04o}; the existing directory is left unchanged",
                    path.display()
                );
            }
            Ok(())
        }
        Err(source) => Err(io_at_root(source)),
    }
}

#[cfg(all(unix, any(test, feature = "test-hooks")))]
fn replace_created_root_with_symlink_for_test(path: &std::path::Path) -> Result<(), DaemonError> {
    let replacement = {
        let mut hook = CONFIGURED_ROOT_REPLACEMENT_FOR_TEST
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if hook
            .as_ref()
            .is_some_and(|(configured, _)| configured == path)
        {
            hook.take()
        } else {
            None
        }
    };
    let Some((_, target)) = replacement else {
        return Ok(());
    };
    std::fs::remove_dir(path).map_err(|source| DaemonError::Io {
        path: path.display().to_string(),
        source,
    })?;
    std::os::unix::fs::symlink(target, path).map_err(|source| DaemonError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// Create one configured daemon root on a platform with no POSIX mode contract.
#[cfg(not(unix))]
fn create_configured_root(path: &std::path::Path, _role: &str) -> Result<(), DaemonError> {
    create_dir(path)
}

fn create_dir(path: &std::path::Path) -> Result<(), DaemonError> {
    std::fs::create_dir_all(path).map_err(|source| DaemonError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// The workspace as the filesystem spells it, which is the only spelling the partitioner
/// compares against. A path that cannot be resolved is kept as written.
#[cfg(feature = "v1-compat")]
fn canonical(path: &std::path::Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
