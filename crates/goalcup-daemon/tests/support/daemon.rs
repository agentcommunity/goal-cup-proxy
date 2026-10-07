//! A real [`Daemon`], one `claude-code-cli` entry inside it, one file
//! the agent wrote and one live child process for the seal to end.
//!
//! Where `support::entry` builds a single [`EntryRuntime`] and drives it by hand, this
//! builds the process-level object: the registry the daemon opened, the two loopback
//! listeners it bound, and the entry map it routes by workspace. Nothing here sleeps and
//! nothing waits on a background task — `Daemon::seal_entry` and `Daemon::seal_due_entries`
//! are synchronous, and the one clock they read is the wall clock, which is why the
//! deadline is moved in the registry rather than by waiting for one.
//!
//! # The connector is `claude-code-cli`, and that is not arbitrary
//!
//! The seal's exclusion union is the assertion this harness exists for, and Claude is the
//! connector whose tracker writes a file into the workspace (`.claude/settings.json`).
//! Codex writes none, so a codex entry could not tell "the union the daemon applied" apart
//! from "the connector's `workspace_exclusions[]`" — which is exactly the subset the protocol
//! refuses.

#![allow(dead_code)]

use goalcup_daemon::config::DaemonConfig;
use goalcup_daemon::paths::{PathVars, claude_slug};
#[cfg(feature = "v1-compat")]
use goalcup_daemon::registry::RunKeys;
use goalcup_daemon::registry::{Entry, EntryState};
use goalcup_daemon::runtime::Daemon;
#[cfg(feature = "v1-compat")]
use goalcup_daemon::runtime::EntryRuntime;
#[cfg(feature = "v1-compat")]
use goalcup_daemon::workspace::{self, Roots};
#[cfg(feature = "v1-compat")]
use goalcup_platform::tracecap::EntryArtifactRoot;
use goalcup_protocol::envelope::EventBody;
use goalcup_protocol::manifest::{ConnectorManifest, KillStrategy};
use goalcup_protocol::process::ProcessHandle;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The connector every daemon case runs on. See the module note.
pub const DAEMON_CONNECTOR: &str = "claude-code-cli";

/// The session the run is recorded under.
pub const DAEMON_SESSION: &str = "s1";

/// A second session, in the same harness home and the same workspace, that this run did
/// not start, which the seal records. Its prompt text is the thing no sighting may carry.
pub const FOREIGN_SESSION: &str = "a9-second";

/// The prompt the foreign session typed. Every assertion about what a sighting says is
/// written against this word: a scan that published one byte of the transcript fails
/// loudly rather than by inspection.
pub const FOREIGN_PROMPT: &str = "a secret prompt nobody may publish";

/// A daemon, one started entry, one file in the workspace and one live child.
pub struct DaemonHarness {
    pub daemon: Daemon,
    pub entry_id: String,
    pub workspace: PathBuf,
    pub home: PathBuf,
    pub artifact_root: PathBuf,
    pub artifact_dir: PathBuf,
    pub trace_dir: PathBuf,
    /// The tracked transcript: `{config_dir}/projects/{slug}/{session_id}.jsonl`.
    pub store_path: PathBuf,
    pub manifest: ConnectorManifest,
    pub config: DaemonConfig,
    pub vars: PathVars,
    /// The harness the seal has to end.
    pub child_pid: u32,
    child: Arc<ChildGuard>,
    _server: Arc<goalcup_devserver::DevServer>,
    _dir: Arc<tempfile::TempDir>,
}

/// Keeps the restart fixture's temporary tree and mock server alive after an intentionally
/// failed successor bootstrap, so a test can inspect the exact raw offline views it left.
pub struct RestartFailureHarness {
    pub error: goalcup_daemon::DaemonError,
    pub config: DaemonConfig,
    pub entry_id: String,
    child: Arc<ChildGuard>,
    server: Arc<goalcup_devserver::DevServer>,
    dir: Arc<tempfile::TempDir>,
}

/// The spawned harness, reaped when the last holder of it goes.
///
/// The `Drop` lives here rather than on [`DaemonHarness`] so that harness can be taken
/// apart field by field — [`restart_daemon`] moves the tempdir, the dev server and this guard into
/// a second harness built around a second daemon, and a type with a `Drop` impl cannot be
/// destructured at all.
pub struct ChildGuard(Mutex<Option<std::process::Child>>);

impl Drop for ChildGuard {
    /// Reap the child. `terminate` never reaps — the daemon is not the harness's parent,
    /// the CLI was — so the process this harness spawned is this harness's to collect.
    fn drop(&mut self) {
        if let Ok(mut held) = self.0.lock()
            && let Some(mut child) = held.take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl DaemonHarness {
    pub fn entry(&self) -> Entry {
        self.daemon
            .registry()
            .get(&self.entry_id)
            .expect("the entry reads")
            .expect("it is there")
    }

    /// Keep a recovered terminal drain alive without granting it a successful upload.  This
    /// is a test harness control over the real devserver, not a daemon test hook: the first
    /// cadence still constructs the real retained `SealedDrain` and takes its ordinary
    /// offline path.
    pub fn set_upload_offline(&self, offline: bool) {
        self._server.set_offline(offline);
    }
}

/// Stop this daemon and start a second one over the same configuration — a restart
/// as a participant's machine performs it.
///
/// Everything on disk is kept: the same artifact root, the same registry file, the same
/// workspace, the same trace, and the same harness process still running. What is not kept
/// is anything the first daemon held in memory — the entry map above all — which is the
/// whole point: a second `Daemon::start` over a live registry has to rehydrate the rows it
/// finds.
///
/// `Daemon::stop` is awaited before the second `start` because it has to be: redb takes a
/// file lock on the registry and on every entry's buffer, so a second daemon cannot open
/// them until the first one's handles are released.
pub async fn restart_daemon(h: DaemonHarness) -> DaemonHarness {
    restart_daemon_after_stop(h, |_| {}).await
}

/// The same restart boundary with one test-only opportunity after the first daemon has
/// released its registry/buffer locks and before the successor opens them. This models a
/// durable on-disk crash artifact without introducing a second live redb owner.
pub async fn restart_daemon_after_stop(
    h: DaemonHarness,
    mutate: impl FnOnce(&goalcup_daemon::registry::Entry),
) -> DaemonHarness {
    let DaemonHarness {
        daemon,
        entry_id,
        workspace,
        home,
        artifact_root,
        artifact_dir,
        trace_dir,
        store_path,
        manifest,
        config,
        vars,
        child_pid,
        child,
        _server,
        _dir,
    } = h;
    daemon.stop().await;
    let row = goalcup_daemon::registry::Registry::open(
        &goalcup_daemon::registry::Registry::default_path(&config.artifact_root),
    )
    .expect("the stopped daemon's registry reopens for crash setup")
    .get(&entry_id)
    .expect("the crash-setup row reads")
    .expect("the crash-setup entry exists");
    mutate(&row);
    let daemon = Daemon::start(config.clone())
        .await
        .expect("a second daemon starts over the same tree");
    DaemonHarness {
        daemon,
        entry_id,
        workspace,
        home,
        artifact_root,
        artifact_dir,
        trace_dir,
        store_path,
        manifest,
        config,
        vars,
        child_pid,
        child,
        _server,
        _dir,
    }
}

/// The same restart boundary where the successor is expected to abort.  It is intentionally
/// narrow: callers retain the pre-restart config/entry identity to inspect raw offline views
/// after the failed bootstrap, while this helper still releases every old runtime and buffer
/// before opening the successor.
pub async fn restart_daemon_after_stop_expect_error(
    h: DaemonHarness,
    mutate: impl FnOnce(&goalcup_daemon::registry::Entry),
) -> RestartFailureHarness {
    let DaemonHarness {
        daemon,
        entry_id,
        config,
        child,
        _server,
        _dir,
        ..
    } = h;
    daemon.stop().await;
    let row = goalcup_daemon::registry::Registry::open(
        &goalcup_daemon::registry::Registry::default_path(&config.artifact_root),
    )
    .expect("the stopped daemon's registry reopens for crash setup")
    .get(&entry_id)
    .expect("the crash-setup row reads")
    .expect("the crash-setup entry exists");
    mutate(&row);
    let error = match goalcup_daemon::runtime::Daemon::start(config.clone()).await {
        Ok(_) => panic!("the successor reaper is injected to abort"),
        Err(error) => error,
    };
    RestartFailureHarness {
        error,
        config,
        entry_id,
        child,
        server: _server,
        dir: _dir,
    }
}

/// Rewrite the one started fixture as a raw pre-upgrade Required/no-phase row, then start a
/// fresh daemon over it. The fixture uses the daemon's feature-gated raw-row seam so its
/// real receipt and trace remain intact while the on-disk legacy shape is planted exactly.
pub async fn restart_as_raw_legacy_required<T>(
    h: DaemonHarness,
    state: EntryState,
    last_process: Option<ProcessHandle>,
    mutate: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>),
    while_stopped: impl FnOnce(&Entry, &[u8]) -> T,
) -> (DaemonHarness, T) {
    let DaemonHarness {
        daemon,
        entry_id,
        workspace,
        home,
        artifact_root,
        artifact_dir,
        trace_dir,
        store_path,
        manifest,
        config,
        vars,
        child_pid,
        child,
        _server,
        _dir,
    } = h;
    let registry = daemon.registry();
    let original = registry
        .get(&entry_id)
        .expect("the entry reads")
        .expect("the started fixture is stored");
    daemon.stop().await;

    let mut raw = serde_json::to_value(original).expect("the old row serializes");
    let fields = raw
        .as_object_mut()
        .expect("the old row serializes as an object");
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
        serde_json::to_value(last_process).expect("the legacy process serializes"),
    );
    mutate(fields);
    let raw_bytes = serde_json::to_vec(&raw).expect("the historical row is compact JSON");
    registry
        .replace_raw_json_for_test(&entry_id, &raw_bytes)
        .expect("the historical row replaces the fixture without lifecycle rewriting");
    let persisted_raw = registry
        .raw_json_for_test(&entry_id)
        .expect("the historical raw row reads")
        .expect("the historical raw row remains stored");
    assert_eq!(
        persisted_raw, raw_bytes,
        "the raw fixture bytes are not decoded and re-serialized before restart"
    );
    let raw: Entry =
        serde_json::from_slice(&persisted_raw).expect("the historical row deserializes");
    let before_restart = while_stopped(&raw, &persisted_raw);
    drop(registry);

    let daemon = Daemon::start(config.clone())
        .await
        .expect("a second daemon starts over the raw legacy row");
    (
        DaemonHarness {
            daemon,
            entry_id,
            workspace,
            home,
            artifact_root,
            artifact_dir,
            trace_dir,
            store_path,
            manifest,
            config,
            vars,
            child_pid,
            child,
            _server,
            _dir,
        },
        before_restart,
    )
}

fn connector_dir(id: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors")
        .join(id)
}

/// The daemon's own configuration for a run rooted at `root`.
///
/// `workspace_parent` and `artifact_root` are disjoint trees, which `DaemonConfig::validate`
/// enforces and `Daemon::start` therefore does too: an agent enumerates its workspace's
/// parent directory on its first tool call.
pub fn config_at(root: &Path, upload_url: &str) -> DaemonConfig {
    DaemonConfig {
        artifact_root: root.join("holder"),
        connectors_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors"),
        config_dir: root.join("config"),
        workspace_parent: root.join("workspaces"),
        control_addr: "127.0.0.1:0".parse().expect("a loopback address"),
        upload_url: upload_url.to_string(),
        server_public_key: goalcup_devserver::dev_server_public_key_hex(),
        receipt_binding: goalcup_daemon::config::ReceiptBindingPolicy::Legacy,
        poll_interval_ms: 250,
        upload_interval_ms: 2_000,
        upload_chunk_max: 256,
        status_interval_ms: 1_000,
        stall_after_seconds: 900.0,
        silence_window_seconds: 60.0,
        ack_lag_seconds: 300.0,
        arming_timeout_seconds: None,
        heartbeat_seconds: 60,
        clock_skew_seconds: 120.0,
        disk_reserve_bytes: 0,
        // No round configured: the signed status carries the entry's own timeline and
        // says so, through `event_status_source: "fallback"`.
        event: None,
    }
}

/// Where Claude writes one session's transcript: the per-run config dir, the project slug
/// it derives from the working directory, and the session id.
pub fn transcript_path(home: &Path, workspace: &Path, session: &str) -> PathBuf {
    home.join("projects")
        .join(claude_slug(workspace))
        .join(format!("{session}.jsonl"))
}

/// One transcript record, as Claude writes it: the session it belongs to, the directory it
/// was typed in, and the prompt.
pub fn transcript_line(workspace: &Path, session: &str, text: &str) -> String {
    serde_json::json!({
        "type": "user",
        "cwd": workspace.display().to_string(),
        "sessionId": session,
        "isSidechain": false,
        "version": "2.1.259",
        "message": { "role": "user", "content": text },
    })
    .to_string()
}

/// A daemon holding one started entry: armed through `Daemon::arm`, corroborated by a real
/// transcript record, its start receipt signed by a live devserver — plus the two things
/// the seal is about, a file the agent wrote and a harness process that is still running.
#[cfg(feature = "v1-compat")]
pub async fn started_entry_with_work() -> DaemonHarness {
    let (server, upload_url) = super::devserver().await;
    let dir = tempfile::tempdir().expect("a temporary directory");
    // Canonical from the start: on macOS a tempdir is handed out as `/var/folders/…` while
    // the harness records `/private/var/folders/…`, and the partitioner compares the two
    // byte for byte.
    let root = std::fs::canonicalize(dir.path()).expect("the tempdir canonicalises");

    // `workspace_parent` and `artifact_root` are disjoint trees, which `DaemonConfig`
    // validates and `Daemon::start` therefore enforces: an agent enumerates its
    // workspace's parent on its first tool call.
    let artifact_root = root.join("holder");
    let workspace = root.join("workspaces/ws-one");
    let home = root.join("home");
    let artifact_dir = artifact_root.join("artifacts/e1");

    let manifest = super::manifest(DAEMON_CONNECTOR);
    let config = config_at(&root, &upload_url);
    let daemon = Daemon::start(config.clone())
        .await
        .expect("the daemon starts");

    // The harness home is deliberately not created here: Claude's `hook_install[]` writes
    // nothing under it, so `workspace::prepare` leaves it absent and the daemon is what
    // creates it.
    let runtime = build_runtime(&daemon, &manifest, &config, "e1", &workspace, &home).await;
    daemon.register(runtime).expect("the entry is registered");
    assert!(
        home.is_dir(),
        "the daemon creates the entry's harness home at registration: CLAUDE_CONFIG_DIR is          a directory the harness is LAUNCHED with, and one that does not exist is a launch          that writes its session somewhere this daemon is not watching"
    );
    daemon.arm("e1").expect("an empty harness home arms");

    // The launch, in the order the CLI does it: arm, launch, register the handle, and only
    // then the harness's first record. The handle is registered before the start because
    // that is the one window `register_process` accepts one in — after the start it names a
    // process the launch did not make, and the daemon refuses it.
    let child = spawn_harness();
    let child_pid = child.id();
    daemon
        .register_process(
            "e1",
            super::process_registration(1, handle_for_now(child_pid)),
        )
        .expect("the process is registered");

    // Leg 1 is Claude's whole arming signal (`launch+first_transcript_record`, D16): the
    // first record the harness writes is the signal, so one poll arms and corroborates it.
    let store_path = transcript_path(&home, &workspace, DAEMON_SESSION);
    super::append_line(
        &store_path,
        &transcript_line(&workspace, DAEMON_SESSION, "the goal"),
    );
    // One tick arms, corroborates and starts: nothing outside the daemon stamps a
    // `started_at`, so `tick` is where the start receipt is requested once the gate allows it.
    daemon.tick().await.expect("the first tick runs");

    // The agent's work, and the second session that was not this run's. Both after the
    // start, so the tracked store is bound to one session and the foreign transcript is
    // never tailed: `{session_id}` renders as `*` until the session binds.
    std::fs::write(workspace.join("main.py"), b"print('hello')\n").expect("the agent's file");
    super::append_line(
        &transcript_path(&home, &workspace, FOREIGN_SESSION),
        &transcript_line(&workspace, FOREIGN_SESSION, FOREIGN_PROMPT),
    );

    DaemonHarness {
        daemon,
        entry_id: "e1".to_string(),
        trace_dir: artifact_dir.join("trace"),
        workspace,
        home,
        artifact_root,
        artifact_dir,
        store_path,
        manifest,
        config,
        vars: PathVars::new(
            root.join("workspaces/ws-one"),
            root.join("home"),
            root.join("home"),
            root.join("holder/artifacts/e1"),
            Some(DAEMON_SESSION.to_string()),
            ("2026".to_string(), "09".to_string(), "04".to_string()),
        ),
        child_pid,
        child: Arc::new(ChildGuard(Mutex::new(Some(child)))),
        _server: server,
        _dir: Arc::new(dir),
    }
}

/// A second entry in the same harness home, ready to be registered and armed. A home
/// holds one run.
#[cfg(feature = "v1-compat")]
pub async fn runtime_sharing_harness_home(h: &DaemonHarness) -> EntryRuntime {
    let workspace = h.workspace.parent().expect("a parent").join("ws-two");
    build_runtime(&h.daemon, &h.manifest, &h.config, "e2", &workspace, &h.home).await
}

/// A second entry in the same daemon, in its own workspace and its own harness home.
///
/// Not [`runtime_sharing_harness_home`], which deliberately shares the home so arming
/// refuses it. This is two runs on one machine as a participant really has
/// them, which is what the cadence cases are about: one entry's stalled receipt must
/// not be the reason the other is not being read.
#[cfg(feature = "v1-compat")]
pub async fn second_entry(h: &DaemonHarness, entry_id: &str) -> EntryRuntime {
    let parent = h.workspace.parent().expect("a parent");
    let workspace = parent.join(format!("ws-{entry_id}"));
    let home = h
        .home
        .parent()
        .expect("a parent")
        .join(format!("home-{entry_id}"));
    build_runtime(
        &h.daemon,
        &h.manifest,
        &h.config,
        entry_id,
        &workspace,
        &home,
    )
    .await
}

/// One entry: its workspace prepared, its row in the daemon's own registry, and the
/// runtime around it.
///
/// The registry comes from the daemon rather than being opened a second time: redb takes a
/// file lock, so a second `Registry::open` on one path is a failure and not a second
/// handle — which is why `EntryRuntime::new` takes the handle as an argument.
#[cfg(feature = "v1-compat")]
async fn build_runtime(
    daemon: &Daemon,
    manifest: &ConnectorManifest,
    config: &DaemonConfig,
    entry_id: &str,
    workspace: &Path,
    home: &Path,
) -> EntryRuntime {
    build_runtime_for(
        daemon,
        DAEMON_CONNECTOR,
        manifest,
        config,
        entry_id,
        workspace,
        home,
        None,
        |_| {},
        |_| {},
    )
    .await
}

/// [`build_runtime`] for any shipped connector, so a case that needs one Claude cannot
/// answer — `lock_paths[]`, which Claude ships empty — can build a codex entry beside it.
#[cfg(feature = "v1-compat")]
#[allow(clippy::too_many_arguments)]
async fn build_runtime_for(
    daemon: &Daemon,
    connector_id: &str,
    manifest: &ConnectorManifest,
    config: &DaemonConfig,
    entry_id: &str,
    workspace: &Path,
    home: &Path,
    // The thread this run is bound to, where the case needs one. `None` is an entry whose
    // gate has not bound yet, which is what `PathVars` renders as `*`; a `lock_paths[]` or a
    // store template naming `{session_id}` then matches nothing, which is correct for a run
    // that has no thread. `codex-cli`'s lock is the per-thread file, so a case
    // about a lock a harness is holding has to say which thread.
    session_id: Option<&str>,
    // The last word on the row, applied before it is stored, so the registry and the
    // runtime's own copy of the entry cannot disagree about what state it is in.
    finish: impl FnOnce(&mut Entry),
    // The last word on the manifest the RUNTIME holds (the workspace is prepared from the
    // shipped one), for a case about a manifest the shipped workspace path would refuse.
    runtime_manifest: impl FnOnce(&mut ConnectorManifest),
) -> EntryRuntime {
    let artifact_dir = config.artifact_root.join("artifacts").join(entry_id);
    let roots = Roots {
        workspace: workspace.to_path_buf(),
        artifact_root: artifact_dir.clone(),
        harness_home: home.to_path_buf(),
    };
    let keys = RunKeys::generate();
    let vars = PathVars::new(
        workspace.to_path_buf(),
        home.to_path_buf(),
        home.to_path_buf(),
        artifact_dir.clone(),
        session_id.map(str::to_string),
        ("2026".to_string(), "09".to_string(), "04".to_string()),
    );
    // Mirror the production launch handoff: the configured root creates this final entry
    // exactly once, then the runtime owns the very same capability after workspace setup.
    let artifact = EntryArtifactRoot::create(&config.artifact_root, entry_id)
        .expect("the fresh entry artifact capability is created");
    let prepared = workspace::prepare(
        manifest,
        &connector_dir(connector_id),
        &super::template_vars(&roots, &keys.receiver_token, entry_id, DAEMON_SESSION),
        &vars,
        &roots,
        entry_id,
    )
    .expect("the workspace is prepared");

    let manifest_bytes = std::fs::read(connector_dir(connector_id).join("manifest.toml"))
        .expect("the shipped manifest");
    let mut entry = Entry::new(
        entry_id,
        connector_id,
        workspace.to_path_buf(),
        artifact_dir.clone(),
        &workspace::sha256_hex(&manifest_bytes),
        "2.1.259",
        config.receipt_binding.clone(),
        goalcup_daemon::registry::ExpectedProcessHandoff::Required,
        keys,
    );
    entry.harness_home = Some(home.to_path_buf());
    entry.expected_config = prepared.expected_config.clone();
    entry.status_line_path = artifact_dir.join("status-line.txt");
    entry.state = EntryState::Created;
    finish(&mut entry);
    let registry = daemon.registry();
    registry.put(&entry).expect("the entry is stored");

    let mut held = manifest.clone();
    runtime_manifest(&mut held);
    EntryRuntime::new_with_artifact(
        entry,
        held,
        config,
        vars,
        prepared.installed.files.clone(),
        prepared.allowlist.clone(),
        prepared.baseline.clone(),
        goalcup_daemon::foreign::TrustedManifests::load(&config.connectors_dir)
            .expect("the shipped manifests are trusted")
            .plan_for(manifest),
        registry,
        artifact,
    )
    .expect("the runtime builds")
}

/// A codex entry beside the Claude one, in a harness home of its own.
///
/// The connector is not decoration: Claude ships `lock_paths = []`, so an assertion that
/// the seal reports resolved lock paths passes vacuously on it. Codex declares
/// `{config_dir}/thread-writer-locks/{session_id}.lock`: the per-thread
/// file, not the directory it sits in, which the harness makes at startup and leaves behind
/// — so this runtime binds `DAEMON_SESSION` and the path resolves under this entry's
/// harness home. That is the path a resume has to be able to take before it relaunches,
/// because a codex resume that cannot take the thread-writer lock silently becomes a new
/// thread.
#[cfg(feature = "v1-compat")]
pub async fn codex_runtime(h: &DaemonHarness) -> (EntryRuntime, PathBuf) {
    let workspace = h.workspace.parent().expect("a parent").join("ws-codex");
    let home = h.artifact_root.parent().expect("a root").join("home-codex");
    let manifest = super::manifest("codex-cli");
    let runtime = build_runtime_for(
        &h.daemon,
        "codex-cli",
        &manifest,
        &h.config,
        "e-codex",
        &workspace,
        &home,
        // A codex entry whose thread is bound, which is what a run with a lock on disk is.
        Some(DAEMON_SESSION),
        |_| {},
        |_| {},
    )
    .await;
    (runtime, home)
}

/// An entry whose manifest names a `lock_paths[]` template this daemon cannot render.
///
/// A packaging mistake, not an attack: the seal reads these paths off the manifest as
/// advice for a later relaunch, and a template naming a variable the tracker does not
/// supply must not be able to leave an entry sealed with its harness still running.
#[cfg(feature = "v1-compat")]
pub async fn runtime_with_unresolvable_lock_paths(h: &DaemonHarness) -> EntryRuntime {
    let workspace = h.workspace.parent().expect("a parent").join("ws-lock");
    let home = h.artifact_root.parent().expect("a root").join("home-lock");
    let mut manifest = h.manifest.clone();
    manifest.lock_paths = vec!["{no_such_variable}/locks/".to_string()];
    build_runtime_for(
        &h.daemon,
        DAEMON_CONNECTOR,
        &manifest,
        &h.config,
        "e-lock",
        &workspace,
        &home,
        None,
        |_| {},
        |_| {},
    )
    .await
}

/// A runtime for an entry the registry already calls `Sealed`, and the harness home that
/// does not exist yet.
///
/// What a daemon meets after a restart: the row survived, the run's chain is closed, and
/// registering it would create directories for a run that can no longer record anything.
#[cfg(feature = "v1-compat")]
pub async fn sealed_runtime(h: &DaemonHarness) -> (EntryRuntime, PathBuf) {
    let workspace = h.workspace.parent().expect("a parent").join("ws-sealed");
    let home = h
        .artifact_root
        .parent()
        .expect("a root")
        .join("home-sealed");
    let runtime = build_runtime_for(
        &h.daemon,
        DAEMON_CONNECTOR,
        &h.manifest,
        &h.config,
        "e-sealed",
        &workspace,
        &home,
        None,
        |e| {
            e.state = EntryState::Sealed;
            e.sealed_at = Some("2026-09-04T00:00:00Z".to_string());
        },
        |_| {},
    )
    .await;
    assert!(!home.exists(), "the harness home is the daemon's to create");
    (runtime, home)
}

/// Replace the entry's saved cursor file with a directory of the same name.
///
/// The one deterministic way to make a tick fail from outside the daemon: `save_cursors`
/// writes a temp file and renames it over `cursors.json`, and a rename onto a directory
/// cannot succeed. What it produces is an `io` error carrying an absolute path, which is
/// also what the path-free rule on the status document is about.
pub fn break_the_cursor_file(h: &DaemonHarness) {
    let path = h.artifact_dir.join("cursors.json");
    let _ = std::fs::remove_file(&path);
    std::fs::create_dir_all(&path).expect("a directory where the cursor file goes");
}

/// Put one store's saved position back to `offset`, `records_back` records short of what it
/// chained.
///
/// The state a hard kill between "the record is in the chain" and "the cursor covering it
/// is on disk" leaves behind — the case `cursors.json` cannot be
/// made to reach by any legal sequence of calls, because the daemon writes it in the safe
/// order. `records_back` is how far behind: one for the kill between two batches, and the
/// count already chained for a kill part-way through a batch, which is the state the cursor
/// is saved per batch to bound.
///
/// Both halves of the line move. `cursors.json` carries the position and the
/// chain seq that position covered, and rolling back only the position would describe a
/// state no kill can produce: the daemon writes the pair in one document. The seq is taken
/// from the chain itself — the record `records_back` back from this store's last one — so
/// the number of `Leg1Record`s above it is exactly `records_back`, which is what the
/// restart counts.
pub fn roll_cursor_back(
    h: &DaemonHarness,
    store: &str,
    file: &Path,
    offset: u64,
    records_back: u64,
) {
    let seqs: Vec<u64> = super::read_chain(&h.trace_dir)
        .iter()
        .filter(|e| matches!(&e.body, EventBody::Leg1Record { store: s, .. } if s == store))
        .map(|e| e.link.seq)
        .collect();
    // `None` when the chain holds no more than `records_back` records for this store: the
    // saved position then covered nothing at all, which is what the daemon writes before a
    // store's first record.
    let floor = seqs
        .len()
        .checked_sub(records_back as usize + 1)
        .and_then(|i| seqs.get(i).copied());

    let path = h.artifact_dir.join("cursors.json");
    let text = std::fs::read_to_string(&path).expect("the cursor file reads");
    let mut doc: serde_json::Value = serde_json::from_str(&text).expect("cursors.json is JSON");
    let line = doc
        .get_mut(store)
        .expect("the store has a line in the cursor file");
    line["last_leg1_seq"] = serde_json::json!(floor);
    let state = line
        .get_mut("cursor")
        .and_then(|s| s.get_mut("inode_offset"))
        .expect("the store keeps an inode/offset cursor");
    let records = state["records"].as_u64().expect("a record count");
    state["records"] = serde_json::json!(records.saturating_sub(records_back));
    let key = file.display().to_string();
    state["files"][&key]["offset"] = serde_json::json!(offset);
    std::fs::write(
        &path,
        serde_json::to_vec(&doc).expect("the cursor file serialises"),
    )
    .expect("the rolled-back cursor is written");
}

/// Whether anything is listening at `addr`, as a bare connect — so a test can wait for the
/// gap between two daemons without sending a request to whichever one is up.
pub async fn listener_is_up(addr: String) -> bool {
    tokio::task::spawn_blocking(move || std::net::TcpStream::connect(&addr).is_ok())
        .await
        .expect("the blocking connect finishes")
}

/// The handle a launch that recorded no argv would have registered.
///
/// `terminate`'s guard has two halves — the pid's age, and the program `ps` names among
/// its arguments — and an empty argv leaves only the first. The kill still runs; the report
/// says it ran on the weaker guard.
pub fn handle_without_argv(pid: u32) -> ProcessHandle {
    ProcessHandle {
        argv: Vec::new(),
        ..handle_for_now(pid)
    }
}

/// Move `deadline_unix` behind us, which is the only way to reach the automatic seal in
/// a test: the deadline is `started_at + 168h`, parsed once, and the daemon reads
/// the wall clock.
pub fn set_deadline_in_the_past(h: &DaemonHarness) {
    h.daemon
        .registry()
        .update(&h.entry_id, |e| {
            e.deadline_unix = Some(1.0);
            e.deadline_at = Some(goalcup_protocol::control::format_z(1));
        })
        .expect("the deadline moves");
}

/// Everything a chained `ForeignSession` says, as strings — so an assertion about what a
/// sighting does not carry is asserted over the whole event and not over one field of it.
pub fn chained_foreign_sessions(trace_dir: &Path) -> Vec<String> {
    super::read_chain(trace_dir)
        .into_iter()
        .filter_map(|e| match e.body {
            EventBody::ForeignSession {
                harness,
                session_id,
                store_path,
                first_seen,
            } => Some(format!("{harness} {session_id} {store_path} {first_seen}")),
            _ => None,
        })
        .collect()
}

/// A receipt server that answers nothing for `delay`, then refuses.
///
/// A real round trip and not a held borrow: `ReceiptClient::request` opens a socket on a
/// blocking worker and reads until the peer answers or `NETWORK_TIMEOUT` expires, and the
/// case the cadence split is about is precisely that wait. A borrow held by the test is the
/// same shape — the runtime is out of its slot either way — but it exercises none of the
/// client, so nothing in it would notice a binding awaited on the loop's own future.
///
/// `std::net` and a thread, not tokio: this workspace's tokio has `net` and no `io-util`,
/// and a stall wants a socket that is accepted and then ignored, which needs no async at
/// all. It answers `503` at the end so the binding finishes as a counted failure rather
/// than by timing out at ten seconds.
pub fn stalling_receipt_server(delay: std::time::Duration) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let addr = listener.local_addr().expect("the bound address");
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            std::thread::spawn(move || {
                use std::io::{BufRead, Write};
                // The request line decides. Only the start receipt is stalled: an uploader
                // pointed at this stub pumps on the same url, `pump_all` holds the runtime
                // out of its slot for the length of a pump, and a stub that stalled those
                // too would stop the entry being polled at all — which is the very thing
                // under test, arrived at by the test's own fixture.
                let mut line = String::new();
                let peer = stream.try_clone().ok();
                if let Some(peer) = peer {
                    let _ = std::io::BufReader::new(peer).read_line(&mut line);
                }
                if line.contains("/receipt") {
                    std::thread::sleep(delay);
                }
                let _ = stream.write_all(
                    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            });
        }
    });
    format!("http://{addr}")
}

/// A second entry that is armed, corroborated and therefore owed the start receipt, which it
/// asks of `upload_url` rather than of the harness's devserver.
///
/// The whole arming signal for Claude is `launch+first_transcript_record` (D16), so this is
/// the same four steps `started_entry_with_work` takes: register, arm, register the launch,
/// and write the first record. The next poll finds `may_start()` and spawns the binding.
///
/// The returned `Child` is the stand-in harness; the caller ends it.
#[cfg(feature = "v1-compat")]
pub async fn entry_owed_a_receipt(
    h: &DaemonHarness,
    entry_id: &str,
    upload_url: &str,
) -> std::process::Child {
    let workspace = h
        .workspace
        .parent()
        .expect("a parent")
        .join(format!("ws-{entry_id}"));
    let home = h
        .home
        .parent()
        .expect("a parent")
        .join(format!("home-{entry_id}"));
    // The receipt url is the runtime's, taken from the config it is built with, so one
    // entry can be pointed at a stalling stub while every other entry in this daemon keeps
    // the live devserver.
    let mut config = h.config.clone();
    config.upload_url = upload_url.to_string();
    let runtime = build_runtime(&h.daemon, &h.manifest, &config, entry_id, &workspace, &home).await;
    h.daemon.register(runtime).expect("the entry opens");
    h.daemon.arm(entry_id).expect("an empty harness home arms");
    let child = spawn_harness();
    h.daemon
        .register_process(
            entry_id,
            super::process_registration(1, handle_for_now(child.id())),
        )
        .expect("the handle is registered");
    super::append_line(
        &transcript_path(&home, &workspace, DAEMON_SESSION),
        &transcript_line(&workspace, DAEMON_SESSION, "the goal"),
    );
    child
}

/// A stand-in for the harness the CLI launched: a process that is alive, is not this test,
/// and dies on SIGTERM.
pub fn spawn_harness() -> std::process::Child {
    std::process::Command::new("/bin/sleep")
        .arg("300")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a sleeper starts")
}

/// The handle the CLI would have registered for it, launched now.
///
/// `launched_at` is the wall clock and not a fixed instant: `terminate`'s age guard refuses
/// a pid younger than its own registered launch, and a handle dated last year would be
/// refused as a recycled number rather than signalled.
pub fn handle_for_now(pid: u32) -> ProcessHandle {
    ProcessHandle {
        pid,
        pgid: None,
        native_pid: None,
        native_binary: None,
        native_process_start: None,
        process_start: goalcup_protocol::process::process_start_identity(pid),
        strategy: KillStrategy::NativePid,
        grace_seconds: 5,
        launched_at: goalcup_protocol::control::format_z(goalcup_daemon::now_unix() as u64),
        program: "sleep".to_string(),
        argv: vec!["/bin/sleep".to_string(), "300".to_string()],
        env_keys: Vec::new(),
        home_seed: None,
    }
}

/// One request against a loopback listener, answered as a status code.
///
/// Twenty lines of HTTP/1.1 over `std::net::TcpStream`, for the same reason
/// `support::mod` has its own: this workspace has no client crate, and a dependency added
/// so a test can speak the protocol it already serves is a dependency the workspace then
/// carries. `Connection: close` so the body ends at EOF and this needs no chunked decoder.
async fn http(
    addr: String,
    method: &'static str,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
) -> u16 {
    try_http(addr, method, path, headers, body)
        .await
        .expect("connect to the daemon")
}

/// [`http`] for a caller that expects the listener to be down some of the time: `None` when
/// the connection was refused rather than a panic.
async fn try_http(
    addr: String,
    method: &'static str,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
) -> Option<u16> {
    tokio::task::spawn_blocking(move || {
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect(&addr).ok()?;
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        );
        for (k, v) in &headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes()).expect("write the head");
        let _ = stream.write_all(body.as_bytes());
        let mut raw = Vec::new();
        let _ = stream.read_to_end(&mut raw);
        Some(
            String::from_utf8_lossy(&raw)
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        )
    })
    .await
    .expect("the blocking request finishes")
}

/// `GET path` against the control listener, with no bearer — which is what a browser
/// opening the status page sends.
pub async fn control_get(h: &DaemonHarness, path: &str) -> u16 {
    http(
        h.daemon.control_addr().to_string(),
        "GET",
        path.to_string(),
        Vec::new(),
        String::new(),
    )
    .await
}
