use super::CycleCounters;
use super::capture::{CaptureEntry, fresh_capture_id};
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

/// The most spawned units of an entry's own blocking work that may be in flight at once, and
/// so the number of permits [`Daemon::stop`] has to take back before it can say the daemon is
/// idle.
///
/// Each unit holds an `Arc<Daemon>` and is not abortable in any useful sense. A `stop` that
/// returned over one would release nothing — the redb registry lock would outlive it, so a
/// restart in place would fail to open a database it had been told was free.
///
/// 64 is far beyond any real daemon: one machine holds a handful of runs.
pub(super) const MAX_IN_FLIGHT: usize = 64;

// =========================================================================================
// The daemon: one per machine, entries routed by working directory (D4).
// =========================================================================================

/// Everything the daemon owns, behind one `Arc` so the control router can share it.
pub(super) struct DaemonInner {
    pub(super) config: DaemonConfig,
    pub(super) registry: Arc<Registry>,
    /// Acquired once before the registry and retained for every later entry operation.
    /// `config.artifact_root` is display/validation data after this capability exists.
    pub(super) artifact_root: ArtifactRoot,
    /// The shipped connector manifests, by declared id, loaded once at startup. Nothing
    /// re-reads the connector directory while the daemon runs: the V1 `create_workspace`
    /// route that did was removed by D48.
    manifests: Arc<TrustedManifests>,
    /// entry id → its v5 local capture. Never an entry of `entries`.
    pub(super) captures: Mutex<BTreeMap<String, Arc<CaptureEntry>>>,
    /// The loopback capture ingest listener, bound once at start; [`Daemon::stop`] takes it
    /// and shuts it down.
    pub(super) ingest: Mutex<Option<IngestListener>>,
    pub(super) ingest_addr: SocketAddr,
    /// The daemon's one observation clock for capture records and ingest outcomes.
    pub(super) capture_clock: IngestClock,
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) finalization_clock: Mutex<Option<f64>>,
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) finalization_clock_steps: Mutex<std::collections::VecDeque<f64>>,
    /// Process-local monotone wall-time authority for deadline/finalization decisions.
    /// `deadline_observed` carries the threshold latch across restarts; this high-water
    /// prevents a wall-clock rollback within one process from moving any phase backwards.
    pub(super) finalization_epoch_unix: f64,
    pub(super) finalization_high_water: Mutex<f64>,
    /// What the daemon's loop lost, so the signed status can carry it.
    pub(super) cycles: Arc<CycleCounters>,
    /// Set once, after the listener bound: the OS chooses the port when the configured one
    /// is `0`, and nothing may read an address the daemon only asked for.
    control_addr: OnceLock<SocketAddr>,
    /// The control listener's task, held so [`Daemon::stop`] can take it down. Dropping a
    /// `JoinHandle` detaches its task, which is what the running daemon wants and what leaves
    /// a stopped one serving a port it no longer owns.
    listeners: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    /// One permit per in-flight unit of blocking work. [`Daemon::stop`] drains it by taking
    /// every permit, which it can only do once each of them has dropped its own. A
    /// semaphore rather than a `JoinSet` because the handler that started the work is the one
    /// awaiting its result, and a `JoinHandle` can be awaited from exactly one place.
    pub(super) blocking: Arc<tokio::sync::Semaphore>,
    pub(super) started: Instant,
}

/// The process-level daemon: the capture map, the loopback listeners and the registry.
///
/// # One daemon per machine (D4)
///
/// The registry is opened once, at [`Registry::default_path`] under the artifact root: redb
/// takes a file lock, so a second `Registry::open` on the same path is a failure and not a
/// second handle.
///
/// # The listener is loopback and nothing else
///
/// `DaemonConfig::validate` refuses a non-loopback `control_addr`. Three control routes are
/// unauthenticated (`control::is_public_route`) and what makes that safe is that the socket is not
/// reachable off this machine.
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

    /// Only a capture entry owns a slot: the V1 runtime was removed (D48).
    pub(crate) fn has_entry_runtime(&self, entry_id: &str) -> bool {
        self.has_capture_entry(entry_id)
    }

    /// Open the registry, sweep what a crash left behind, load the shipped manifests, bind
    /// the listeners and reopen every retained local capture.
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
        // D48: this tree has no V1 runtime. A V1 row that a running daemon would still owe work
        // to (an unsealed run, an unfinished drain, a harness still to reap) cannot be served
        // here, so the whole registry is refused before anything is swept or changed. A sealed,
        // fully acknowledged V1 row is inert evidence that this build never touches, and does not
        // block the capture product.
        if let Some(owing) = registry
            .all()?
            .iter()
            .find(|entry| entry.capture.is_none() && v1_row_owes_work(entry))
        {
            return Err(DaemonError::Refused(format!(
                "the registry holds a V1 (transcript-path) entry, {}, that is not finished: it is \
                 {:?} and this build does not include the V1 runtime that would drain and seal it. \
                 Finish it with a v0.5.0 daemon built with `--features v1-compat` on this \
                 artifact root (once it is sealed and fully acknowledged it no longer blocks this \
                 daemon), or give the capture product its own `artifact_root` and `config_dir`",
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
            manifests: manifests.clone(),
            captures: Mutex::new(BTreeMap::new()),
            ingest: Mutex::new(Some(ingest)),
            ingest_addr,
            capture_clock,
            #[cfg(any(test, feature = "test-hooks"))]
            finalization_clock: Mutex::new(None),
            #[cfg(any(test, feature = "test-hooks"))]
            finalization_clock_steps: Mutex::new(std::collections::VecDeque::new()),
            finalization_epoch_unix: now_unix(),
            finalization_high_water: Mutex::new(0.0),
            cycles: Arc::new(CycleCounters::default()),
            control_addr: OnceLock::new(),
            listeners: Mutex::new(Vec::new()),
            blocking: Arc::new(tokio::sync::Semaphore::new(MAX_IN_FLIGHT)),
            started: Instant::now(),
            config,
        });

        let daemon = Daemon {
            inner: inner.clone(),
        };

        // **Every run the previous daemon was capturing, back in the map first** — before
        // the control listener, because the first thing a CLI that finds `control.json` asks
        // is what this daemon is holding.
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
    /// **What it is for.** The registry is a redb database, and redb takes a file lock — that is what makes one daemon per machine a fact rather
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
    /// Entries are not sealed by this. A stop is the process ending; a seal is the run ending.
    ///
    /// **Blocking work already running is awaited.** It is not on `listeners` and it is not
    /// abortable: `tokio::task::spawn_blocking` runs a synchronous closure, still holding the
    /// `Arc<Daemon>` that owns the redb lock. A `stop` that returned over it would leave a
    /// restart-in-place unable to open the registry it had just been told was released.
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
        // The listener is down before this, so no new request can start blocking work; what
        // is left is what was already running. Every permit back means every unit has dropped
        // its own, which happens when the closure ends — including when it unwinds.
        let _ = self.inner.blocking.acquire_many(MAX_IN_FLIGHT as u32).await;
    }

    /// Where the CLI and `goalcup doctor` connect. Loopback, always.
    pub fn control_addr(&self) -> SocketAddr {
        self.inner
            .control_addr
            .get()
            .copied()
            .unwrap_or_else(|| loopback(0))
    }

    /// The one registry handle in this process: redb takes a file lock, so there is no
    /// second one.
    pub fn registry(&self) -> Arc<Registry> {
        self.inner.registry.clone()
    }

    pub(crate) fn artifact_root(&self) -> &ArtifactRoot {
        &self.inner.artifact_root
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

    /// The daemon's loop: nothing to drive.
    ///
    /// A local capture entry owns its own work (ingest and control calls drive it), so there is
    /// no poll, upload, status or deadline cadence to run. It never returns, which is the
    /// contract the foreground binary relies on: the only way out is a signal.
    pub async fn run_forever(&self) -> Result<(), DaemonError> {
        std::future::pending().await
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
        self.inner.manifests.clone()
    }

    /// The signed status document of a local capture, as `goalcup status` reads it. `None`
    /// for an id that is not a capture this daemon holds.
    pub fn signed_status(&self, entry_id: &str) -> Result<Option<SignedStatus>, DaemonError> {
        self.capture_status(entry_id)
    }

    /// Drop a local capture that never started: failure before dispatch leaves no entry.
    /// `_reason` is the caller's sentence; a capture abandon records none.
    pub fn abandon(&self, entry_id: &str, _reason: &str) -> Result<(), DaemonError> {
        if self.has_capture_entry(entry_id) {
            return self.abandon_capture(entry_id);
        }
        Err(no_such_entry(entry_id))
    }

    /// The rows `goalcup entries` prints, in entry-id order.
    pub fn entries(&self) -> Result<Vec<EntrySummary>, DaemonError> {
        let mut out = Vec::new();
        out.extend(self.capture_summaries()?);
        out.sort_by(|left, right| left.entry_id.cmp(&right.entry_id));
        Ok(out)
    }

    /// The single entry a caller with no id means: the one open run that is not sealed.
    /// `None` when there is none, and `None` when there is more than one — a daemon holding
    /// two live runs must not guess which one a bare `goalcup status` is about.
    pub fn current_entry(&self) -> Result<Option<String>, DaemonError> {
        let mut live: Vec<String> = self
            .capture_summaries()?
            .into_iter()
            .filter(|summary| summary.sealed_at.is_none())
            .map(|summary| summary.entry_id)
            .collect();
        Ok(match live.len() {
            1 => live.pop(),
            _ => None,
        })
    }

    /// Record the process the CLI launched, so the seal has something to end (D22, D9).
    ///
    /// **Refused while a handle stands, and for exactly as long as one does.** The handle is
    /// what the seal terminates, so a second one posted on the loopback is a decoy that
    /// points it at a process the launch never made. The
    /// registered handle wins and everything else is a `409`.
    pub fn register_process(
        &self,
        entry_id: &str,
        registration: ProcessRegistration,
    ) -> Result<(), DaemonError> {
        if self.has_capture_entry(entry_id) {
            return self.register_capture_process(entry_id, registration);
        }
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
        Err(no_such_entry(entry_id))
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

    /// Whether every open entry's last upload reached the server. Always `true`: this tree has
    /// no uploader (D48), so none is failing.
    pub fn upload_ok(&self) -> bool {
        true
    }

    // -------------------------------------------------------------------------------
    // The map, and the one place each lock is taken.
    // -------------------------------------------------------------------------------

    /// What the daemon's loop has lost, as the status document carries it.
    pub fn cycle_errors(&self) -> CycleErrors {
        self.inner.cycles.snapshot()
    }
}

/// Whether a V1 row still has work only the removed V1 runtime could do (D48): it is not sealed, its
/// terminal drain needs review, its acknowledgement watermark is short of the chain head, or a
/// sealed harness root is still alive and awaits the reaper. A refused row (never admitted) and a
/// sealed, drained, reaped row owe nothing.
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

/// The refusal for an entry id this daemon does not hold: every id that is not a local
/// capture lands here.
fn no_such_entry(entry_id: &str) -> DaemonError {
    DaemonError::Missing(format!("no entry {entry_id} in this daemon"))
}

fn loopback(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
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
