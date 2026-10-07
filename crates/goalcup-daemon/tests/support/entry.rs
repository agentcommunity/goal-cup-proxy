//! One real entry, on a real disk, with a real chain.
//!
//! Everything an `EntryRuntime` needs is built here the way the daemon builds it —
//! `workspace::prepare` writes the tracker files, `Registry` holds the entry, a devserver
//! signs the start receipt — so a test asserts against the wiring rather than against a
//! stub that agrees with it.
//!
//! # The default connector is `codex-cli`, and that is not arbitrary
//!
//! A foreign row must never reach the chain, and
//! `claude-code-cli` has nothing to discard: its `main` store carries an
//! `unpartitioned_reason` (the per-run `CLAUDE_CONFIG_DIR` is a process-scoped pin, so the
//! file holds no foreign rows). Codex's `rollout` store partitions `file_first_record` on
//! `payload.cwd`, which is exactly the strategy a second thread in another directory
//! trips, and its `history` store partitions `direct` on the session id — the pair
//! partition fidelity is about.
//!
//! # The clock
//!
//! The harness owns one `f64` and `h.now()` reads it. `advance_clock` adds to it. Nothing
//! here sleeps, and nothing waits on a background task: `poll_once_at(h.now())` is the
//! whole of the daemon's tick.

#![allow(dead_code)]

use super::{append_line, read_chain};
use goalcup_daemon::config::DaemonConfig;
use goalcup_daemon::paths::PathVars;
use goalcup_daemon::registry::{Entry, EntryState, Registry, RunKeys};
use goalcup_daemon::runtime::EntryRuntime;
use goalcup_daemon::workspace::{self, Roots};
use goalcup_platform::tracecap::EntryArtifactRoot;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::events::{HealthState, TailNoticeKind};
use goalcup_protocol::manifest::KillStrategy;
use goalcup_protocol::manifest::{ConnectorManifest, LaunchMode};
use goalcup_protocol::process::ProcessHandle;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The connector an entry case runs on unless it names another. See the module note.
pub const CONNECTOR: &str = "codex-cli";

/// The session id the planted rollout declares and the run is recorded under.
pub const SESSION: &str = "s1";

/// The moment every harness starts at. Fixed, because the arming window, the stall window
/// and the deadline are all measured from it and a test that read the wall clock would be
/// asserting against a number that changes overnight.
pub const CLOCK0: f64 = 1_756_944_000.0;

/// What the app-wide `config_allowlist[]` file holds when the daemon baselines it. Named
/// once, because a test that puts the file back has to write the bytes the baseline was
/// taken over, and a second literal would drift from this one.
pub const ALLOWLISTED_BYTES: &[u8] = b"model = \"o3\"\n";

pub fn connector_dir(id: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors")
        .join(id)
}

/// One entry, armed, its trace open, its stores empty.
///
/// Armed and not started: the gate is where `goalcup start` leaves it, so a test that
/// wants a started run says so with [`start_entry`]. Every store is empty, because
/// "an entry starts empty" is the property the first poll is asserted against.
pub struct EntryHarness {
    runtime: Mutex<Option<EntryRuntime>>,
    pub registry: Arc<Registry>,
    /// Which connector this harness was built on, so a restart re-derives the tracker files
    /// from the same one rather than from the default.
    pub connector: String,
    pub entry_id: String,
    pub workspace: PathBuf,
    pub home: PathBuf,
    pub artifact_dir: PathBuf,
    pub trace_dir: PathBuf,
    /// The tracked rollout: its first record is a `session_meta` naming this workspace.
    pub store_path: PathBuf,
    /// A second rollout in the same directory whose `session_meta` names another
    /// directory. Codex writes one file per thread, so this is what a second session in
    /// the same harness home actually looks like.
    pub foreign_store_path: PathBuf,
    /// The app-wide file the tracker baselined and may never restore.
    pub allowlisted_path: PathBuf,
    pub manifest: ConnectorManifest,
    pub config: DaemonConfig,
    pub vars: PathVars,
    pub upload_url: String,
    clock: Mutex<f64>,
    _server: Arc<goalcup_devserver::DevServer>,
    _dir: tempfile::TempDir,
}

impl EntryHarness {
    /// The only clock any test reads, and the one every `poll_once_at` is handed.
    pub fn now(&self) -> f64 {
        *self.clock.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn poll(&self) -> goalcup_daemon::runtime::PollReport {
        let now = self.now();
        self.with(|r| r.poll_once_at(now)).expect("the tick runs")
    }

    /// Reach into the runtime. It lives behind an `Option` so [`EntryHarness::close`] can
    /// drop it: the buffer is a redb database and redb takes a file lock, so a restarted
    /// daemon cannot open the same buffer until the first one has let go.
    pub fn with<T>(&self, f: impl FnOnce(&mut EntryRuntime) -> T) -> T {
        let mut held = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        f(held.as_mut().expect("the runtime is open"))
    }

    /// Take the runtime out, for a caller that has to `await` while holding it.
    pub fn take(&self) -> EntryRuntime {
        self.runtime
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .expect("the runtime is open")
    }

    /// Put it back.
    pub fn give_back(&self, runtime: EntryRuntime) {
        *self.runtime.lock().unwrap_or_else(|e| e.into_inner()) = Some(runtime);
    }

    /// Drop the runtime, releasing the buffer and the chain writer. What a restarting
    /// daemon finds on disk is exactly what this left behind.
    pub fn close(&self) {
        let mut held = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        *held = None;
    }

    /// How many envelopes are waiting in the write-ahead buffer.
    pub fn buffer_pending(&self) -> u64 {
        self.with(|r| r.pending()).expect("the buffer counts")
    }

    /// The chain head, as the runtime holds it.
    pub fn head(&self) -> (u64, String) {
        let chain = read_chain(&self.trace_dir);
        let last = chain.last().expect("the chain has links");
        (last.link.seq, last.link.hash.clone())
    }

    pub fn entry(&self) -> Entry {
        self.registry
            .get(&self.entry_id)
            .expect("the entry reads")
            .expect("it is there")
    }
}

/// Add `seconds` to the harness's own clock. There is nothing else to move: `EntryRuntime`
/// holds no clock, and every timer inside a tick reads the `now` the caller handed it.
pub fn advance_clock(h: &EntryHarness, seconds: f64) {
    let mut clock = h.clock.lock().unwrap_or_else(|e| e.into_inner());
    *clock += seconds;
}

/// One armed entry on the default connector. See [`one_armed_entry_on`].
pub async fn one_armed_entry() -> EntryHarness {
    one_armed_entry_on(CONNECTOR).await
}

/// A Claude entry whose participant home and per-run config home are distinct, as they are
/// in production. Foreign-scan provenance tests need both roots so an own-config sighting
/// cannot accidentally look like a participant-home sighting in the fixture.
pub async fn one_armed_entry_with_distinct_config_home() -> EntryHarness {
    one_armed_entry_on_with_binding_and_trace_and_config(
        "claude-code-cli",
        goalcup_daemon::config::ReceiptBindingPolicy::Legacy,
        goalcup_devserver::ReceiptAuthority::Legacy,
        goalcup_daemon::registry::ExpectedProcessHandoff::NotRequired,
        None,
        true,
        true,
    )
    .await
}

/// A fresh `EntryRuntime` with its artifact capability and registry row, but before its
/// `FreshPending` writer creates the trace root.  This is deliberately narrower than the
/// ordinary harness: a caller can plant an attacker-owned empty final `trace` and exercise
/// the shipped `open_fresh_trace` refusal without manufacturing a second artifact owner.
pub async fn fresh_pending_entry() -> EntryHarness {
    one_armed_entry_on_with_binding_and_trace(
        CONNECTOR,
        goalcup_daemon::config::ReceiptBindingPolicy::Legacy,
        goalcup_devserver::ReceiptAuthority::Legacy,
        goalcup_daemon::registry::ExpectedProcessHandoff::NotRequired,
        None,
        false,
    )
    .await
}

/// An otherwise ordinary launch-mode entry whose spawned process has not yet been handed
/// over. Kept separate from the general runtime fixture: most lifecycle tests exercise
/// capture/receipt behavior rather than the CLI launch contract.
pub async fn one_armed_entry_requiring_process_handoff() -> EntryHarness {
    one_armed_entry_on_with_binding(
        CONNECTOR,
        goalcup_daemon::config::ReceiptBindingPolicy::Legacy,
        goalcup_devserver::ReceiptAuthority::Legacy,
        goalcup_daemon::registry::ExpectedProcessHandoff::Required,
        None,
    )
    .await
}

/// A pre-upgrade row whose absent persisted handoff field deserialised as
/// `LegacyUnknown`.
pub async fn one_armed_legacy_handoff_entry() -> EntryHarness {
    one_armed_entry_on_with_binding(
        CONNECTOR,
        goalcup_daemon::config::ReceiptBindingPolicy::Legacy,
        goalcup_devserver::ReceiptAuthority::Legacy,
        goalcup_daemon::registry::ExpectedProcessHandoff::LegacyUnknown,
        None,
    )
    .await
}

/// A connector with no launch recipe. This is a focused runtime fixture; no shipped
/// manifest is attach-only, so production creation continues to use the pinned manifests.
pub async fn one_armed_attach_only_entry() -> EntryHarness {
    one_armed_entry_on_with_binding(
        CONNECTOR,
        goalcup_daemon::config::ReceiptBindingPolicy::Legacy,
        goalcup_devserver::ReceiptAuthority::Legacy,
        goalcup_daemon::registry::ExpectedProcessHandoff::NotRequired,
        Some(LaunchMode::AttachOnly),
    )
    .await
}

/// The harness version the entry is created with — the one the connector says it was
/// measured against, so a `VersionChange` fires only when a planted record disagrees with
/// what the spike actually ran.
fn pinned_version(connector: &str) -> &'static str {
    let _ = connector;
    "0.153.0"
}

/// The session id whose name the tracker files are rendered with.
fn harness_session(connector: &str) -> &'static str {
    let _ = connector;
    SESSION
}

/// One armed entry on `connector`: workspace prepared, registry holding it, trace open at
/// its baseline, every store empty.
pub async fn one_armed_entry_on(connector: &str) -> EntryHarness {
    one_armed_entry_on_with_binding(
        connector,
        goalcup_daemon::config::ReceiptBindingPolicy::Legacy,
        goalcup_devserver::ReceiptAuthority::Legacy,
        goalcup_daemon::registry::ExpectedProcessHandoff::NotRequired,
        None,
    )
    .await
}

/// One armed Codex entry whose persisted policy and independently configured real
/// loopback receipt service can agree or disagree. The latter is deliberate in the
/// refusal case: neither side learns its contract from the other.
pub async fn one_armed_entry_with_receipt_binding(
    entry_version: &str,
    server_version: &str,
) -> EntryHarness {
    one_armed_entry_on_with_binding(
        CONNECTOR,
        goalcup_daemon::config::ReceiptBindingPolicy::NewEvent {
            questionnaire_version: entry_version.to_string(),
        },
        goalcup_devserver::ReceiptAuthority::new_event(server_version)
            .expect("the service questionnaire version is valid"),
        goalcup_daemon::registry::ExpectedProcessHandoff::NotRequired,
        None,
    )
    .await
}

async fn one_armed_entry_on_with_binding(
    connector: &str,
    receipt_binding: goalcup_daemon::config::ReceiptBindingPolicy,
    authority: goalcup_devserver::ReceiptAuthority,
    expected_process_handoff: goalcup_daemon::registry::ExpectedProcessHandoff,
    launch_mode: Option<LaunchMode>,
) -> EntryHarness {
    one_armed_entry_on_with_binding_and_trace(
        connector,
        receipt_binding,
        authority,
        expected_process_handoff,
        launch_mode,
        true,
    )
    .await
}

async fn one_armed_entry_on_with_binding_and_trace(
    connector: &str,
    receipt_binding: goalcup_daemon::config::ReceiptBindingPolicy,
    authority: goalcup_devserver::ReceiptAuthority,
    expected_process_handoff: goalcup_daemon::registry::ExpectedProcessHandoff,
    launch_mode: Option<LaunchMode>,
    open_trace: bool,
) -> EntryHarness {
    one_armed_entry_on_with_binding_and_trace_and_config(
        connector,
        receipt_binding,
        authority,
        expected_process_handoff,
        launch_mode,
        open_trace,
        false,
    )
    .await
}

async fn one_armed_entry_on_with_binding_and_trace_and_config(
    connector: &str,
    receipt_binding: goalcup_daemon::config::ReceiptBindingPolicy,
    authority: goalcup_devserver::ReceiptAuthority,
    expected_process_handoff: goalcup_daemon::registry::ExpectedProcessHandoff,
    launch_mode: Option<LaunchMode>,
    open_trace: bool,
    distinct_config_home: bool,
) -> EntryHarness {
    let (server, upload_url) = super::devserver_with_authority(authority).await;
    let dir = tempfile::tempdir().expect("a temporary directory");
    // Canonical from the start: on macOS a tempdir is handed out as `/var/folders/…` and
    // the harness records `/private/var/folders/…` in its own `cwd`, and the partitioner
    // compares the two byte for byte.
    let root = std::fs::canonicalize(dir.path()).expect("the tempdir canonicalises");

    let workspace = root.join("holder/ws-one");
    let home = root.join("home");
    let config_home = if distinct_config_home {
        root.join("config-home")
    } else {
        home.clone()
    };
    let artifact_dir = root.join("holder/artifacts/e1");
    // Match Daemon::start's configured-root bootstrap. Everything below this root is then
    // capability-created, including `artifacts/e1` below.
    std::fs::create_dir(root.join("holder")).expect("the configured artifact root exists");
    std::fs::create_dir_all(&home).expect("the harness home");

    let mut manifest = super::manifest(connector);
    if let Some(launch_mode) = launch_mode {
        manifest.launch.mode = launch_mode;
    }
    let roots = Roots {
        workspace: workspace.clone(),
        artifact_root: artifact_dir.clone(),
        harness_home: config_home.clone(),
    };
    let keys = RunKeys::generate();
    let vars = PathVars::new(
        workspace.clone(),
        home.clone(),
        config_home.clone(),
        artifact_dir.clone(),
        None,
        ("2026".to_string(), "09".to_string(), "04".to_string()),
    );
    // The app-wide file the tracker baselines and never restores — the connector's
    // own first `config_allowlist[]` entry, rendered, rather than a path copied out of one
    // manifest and reused for the others.
    let allowlisted_path = PathBuf::from(
        goalcup_daemon::paths::resolve_template(&manifest.config_allowlist[0], &vars)
            .expect("the connector's first allowlisted path renders"),
    );
    if let Some(parent) = allowlisted_path.parent() {
        std::fs::create_dir_all(parent).expect("the allowlisted file's directory");
    }
    std::fs::write(&allowlisted_path, ALLOWLISTED_BYTES).expect("the allowlisted file");

    // Mirror control setup: the final artifact entry is created once through the configured
    // root and handed to the fresh runtime. `workspace::prepare` renders files below it but
    // is not authority to create it lexically.
    let artifact = EntryArtifactRoot::create(&root.join("holder"), "e1")
        .expect("the fixture creates its entry capability");

    let prepared = workspace::prepare(
        &manifest,
        &connector_dir(connector),
        &super::template_vars(
            &roots,
            &keys.receiver_token,
            "e1",
            harness_session(connector),
        ),
        &vars,
        &roots,
        "e1",
    )
    .expect("the workspace is prepared");

    let manifest_bytes = std::fs::read(connector_dir(connector).join("manifest.toml"))
        .expect("the shipped manifest");
    let registry = Arc::new(
        Registry::open(&Registry::default_path(&root.join("holder"))).expect("the registry opens"),
    );
    let mut entry = Entry::new(
        "e1",
        connector,
        workspace.clone(),
        artifact_dir.clone(),
        &workspace::sha256_hex(&manifest_bytes),
        pinned_version(connector),
        receipt_binding.clone(),
        expected_process_handoff,
        keys,
    );
    entry.harness_home = Some(config_home.clone());
    entry.expected_config = prepared.expected_config.clone();
    entry.armed_at = Some(CLOCK0);
    entry.state = EntryState::Armed;
    entry.status_line_path = artifact_dir.join("status-line.txt");
    registry.put(&entry).expect("the entry is stored");
    // This fixture reconstructs an already-armed row instead of calling
    // `EntryRuntime::arm`.  Keep that reconstruction faithful to the durable
    // transition: a required launch handoff is issued when (and only when) the
    // entry is armed, not merely because a test constructed an `Entry`.
    if matches!(
        expected_process_handoff,
        goalcup_daemon::registry::ExpectedProcessHandoff::Required
    ) {
        entry = registry
            .open_initial_process_handoff(&entry.entry_id)
            .expect("arming issues the first launch handoff generation");
    }

    let config = DaemonConfig {
        artifact_root: root.join("holder"),
        connectors_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors"),
        config_dir: root.join("config"),
        workspace_parent: root.join("holder"),
        control_addr: "127.0.0.1:0".parse().expect("a loopback address"),
        upload_url: upload_url.clone(),
        server_public_key: goalcup_devserver::dev_server_public_key_hex(),
        receipt_binding,
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
    };

    let mut runtime = EntryRuntime::new_with_artifact(
        entry,
        manifest.clone(),
        &config,
        vars.clone(),
        prepared.installed.files.clone(),
        prepared.allowlist.clone(),
        prepared.baseline.clone(),
        goalcup_daemon::foreign::TrustedManifests::load(&config.connectors_dir)
            .expect("the shipped manifests are trusted")
            .plan_for(&manifest),
        registry.clone(),
        artifact,
    )
    .expect("the runtime builds");
    if open_trace {
        runtime.open_trace().expect("the trace opens");
    }

    // The tracked store and the second thread beside it.
    let sessions = config_home.join("sessions/2026/09/04");
    std::fs::create_dir_all(&sessions).expect("the rollout directory");
    let (store_path, foreign_store_path) = (
        sessions.join(format!("rollout-2026-09-04T09-12-33-{SESSION}.jsonl")),
        sessions.join("rollout-2026-09-04T09-14-01-other.jsonl"),
    );

    EntryHarness {
        runtime: Mutex::new(Some(runtime)),
        registry,
        connector: connector.to_string(),
        entry_id: "e1".to_string(),
        trace_dir: artifact_dir.join("trace"),
        workspace,
        home,
        artifact_dir,
        store_path,
        foreign_store_path,
        allowlisted_path,
        manifest,
        config,
        vars,
        upload_url,
        clock: Mutex::new(CLOCK0),
        _server: server,
        _dir: dir,
    }
}

/// The tracked thread's opening record: the `session_meta` whose `payload.cwd` is this
/// workspace, which is what `file_first_record` decides the whole file by.
///
/// It carries both `payload.session_id` and `payload.id`, equal, because that is what Codex
/// 0.153.0 writes on this one record type. The
/// connector binds the session from `payload.session_id`; `payload.id` stays here because a
/// fixture that dropped it would stop being what the harness writes.
pub fn session_meta_line(h: &EntryHarness) -> String {
    serde_json::json!({
        "timestamp": "2026-09-04T09:12:33.401Z",
        "type": "session_meta",
        "payload": { "session_id": SESSION, "id": SESSION,
                     "cwd": h.workspace.display().to_string(),
                     "cli_version": "0.153.0" },
    })
    .to_string()
}

/// How many leg-1 records of `store` are in the chain.
pub fn leg1_count(trace_dir: &Path, store: &str) -> usize {
    read_chain(trace_dir)
        .into_iter()
        .filter(|e| matches!(&e.body, EventBody::Leg1Record { store: s, .. } if s == store))
        .count()
}

/// A second thread's opening record, working somewhere this tracker knows nothing about.
pub fn foreign_meta_line() -> String {
    serde_json::json!({
        "timestamp": "2026-09-04T09:14:01.002Z",
        "type": "session_meta",
        "payload": { "id": "other", "cwd": "/somewhere/else", "cli_version": "0.153.0" },
    })
    .to_string()
}

/// One line of a filler batch. Deliberately not a `session_meta`: the file's verdict is
/// already decided by its first record, and these are the ordinary records that follow.
fn filler_line(n: usize) -> String {
    let pad = "x".repeat(900);
    serde_json::json!({ "type": "response_item", "n": n, "text": pad }).to_string()
}

/// Roughly `n` × [`goalcup_daemon::MAX_POLL_BYTES`] of records, plus a short tail line, so
/// the tailer reports `more_available` on the first `n` passes and nothing on the next.
pub fn append_batches(path: &Path, n: usize) {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("the store opens for append");
    for i in 0..lines_in_batches(n) {
        writeln!(file, "{}", filler_line(i as usize)).expect("a filler line");
    }
    file.flush().expect("the batch reaches the disk");
}

/// How many lines [`append_batches`] wrote, so a test asserts a count rather than a range.
pub fn lines_in_batches(n: usize) -> u64 {
    // Each filler line is comfortably under a kilobyte; 1,400 of them clears a 1 MiB
    // window with room to spare, so `n` of these is more than `n` polls' worth.
    (n as u64) * 1_400
}

pub fn chained_tail_notice_kinds(trace_dir: &Path) -> Vec<TailNoticeKind> {
    read_chain(trace_dir)
        .into_iter()
        .filter_map(|e| match e.body {
            EventBody::TailNotice { kind, .. } => Some(kind),
            _ => None,
        })
        .collect()
}

pub fn chained_health_states(trace_dir: &Path) -> Vec<HealthState> {
    read_chain(trace_dir)
        .into_iter()
        .filter_map(|e| match e.body {
            EventBody::Health(h) => Some(h.state),
            _ => None,
        })
        .collect()
}

pub fn chained_config_deviations(
    trace_dir: &Path,
) -> Vec<goalcup_protocol::events::ConfigDeviation> {
    read_chain(trace_dir)
        .into_iter()
        .filter_map(|e| match e.body {
            EventBody::ConfigDeviation(d) => Some(d),
            _ => None,
        })
        .collect()
}

pub fn last_heartbeat(trace_dir: &Path) -> Option<HeartbeatFacts> {
    read_chain(trace_dir)
        .into_iter()
        .rev()
        .find_map(|e| match e.body {
            EventBody::Heartbeat {
                stores_seen,
                receiver_ok,
                ..
            } => Some(HeartbeatFacts {
                stores_seen,
                receiver_ok,
            }),
            _ => None,
        })
}

pub struct HeartbeatFacts {
    pub stores_seen: Vec<goalcup_protocol::cursor::StoreCursor>,
    pub receiver_ok: bool,
}

/// `trace.toml`, read back as the document the export wrote.
///
/// Parsed into `goalcup_protocol::export::TraceDescriptorDoc` and not into the verifier's
/// `TraceDescriptor`: `goalcup-daemon` does not depend on `goalcup-verifier` (D3), and a
/// test that reached for it would make the capture crate's tests depend on the crate that
/// judges it. The fields asserted are the ones `TraceDescriptor::load` refuses a trace for.
pub fn read_descriptor(trace_dir: &Path) -> goalcup_protocol::export::TraceDescriptorDoc {
    let path = trace_dir.join("trace.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    toml::from_str(&text).expect("trace.toml is a trace descriptor")
}

/// Lose the saved tailing positions, the way a wiped artifact directory or a half-written
/// file does. Every store then re-reads from zero, and D19 says that is expected — the
/// point of the test is that the chain says so.
pub fn delete_cursors(h: &EntryHarness) {
    let path = h.artifact_dir.join("cursors.json");
    std::fs::remove_file(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

/// Put a line store's saved byte position back to `offset`, `records_back` records short
/// of what it has actually chained — the `EntryHarness` twin of
/// `support::daemon::roll_cursor_back`, for a test that drives a single `EntryRuntime`
/// directly rather than a full `Daemon`.
///
/// Both line cursor shapes carry the same per-file byte offsets: `inode_offset` directly,
/// and `ordinal` beside its per-file high-water marks. This helper changes only the byte
/// position and emitted-record count; a caller replaying a line without an ordinal leaves
/// the high-water map untouched.
///
/// See that function's doc for why both halves of the line move together: the position and
/// the chain seq it covered are one document, and a kill can only ever leave them agreeing
/// with each other, never with a hand-picked position alone.
pub fn roll_back_file_cursor(
    h: &EntryHarness,
    store: &str,
    file: &Path,
    offset: u64,
    records_back: u64,
) {
    let seqs: Vec<u64> = read_chain(&h.trace_dir)
        .iter()
        .filter(|e| matches!(&e.body, EventBody::Leg1Record { store: s, .. } if s == store))
        .map(|e| e.link.seq)
        .collect();
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
    let cursor = line
        .get_mut("cursor")
        .and_then(serde_json::Value::as_object_mut)
        .expect("the store keeps a line cursor");
    let state = if cursor.contains_key("inode_offset") {
        cursor
            .get_mut("inode_offset")
            .expect("the inode-offset state exists")
    } else {
        cursor
            .get_mut("ordinal")
            .expect("the store keeps an inode-offset or ordinal line cursor")
    };
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

/// Replace one saved line cursor's source-path key. This models a modified bookkeeping
/// file without changing the declared connector source set.
pub fn replace_file_cursor_path(h: &EntryHarness, store: &str, from: &Path, to: &Path) {
    let path = h.artifact_dir.join("cursors.json");
    let text = std::fs::read_to_string(&path).expect("the cursor file reads");
    let mut doc: serde_json::Value = serde_json::from_str(&text).expect("cursors.json is JSON");
    let cursor = doc
        .get_mut(store)
        .and_then(|line| line.get_mut("cursor"))
        .and_then(serde_json::Value::as_object_mut)
        .expect("the store keeps a cursor");
    let state = if cursor.contains_key("inode_offset") {
        cursor
            .get_mut("inode_offset")
            .expect("the inode-offset state exists")
    } else {
        cursor
            .get_mut("ordinal")
            .expect("the store keeps an inode-offset or ordinal line cursor")
    };
    let files = state["files"]
        .as_object_mut()
        .expect("the line cursor has a file map");
    let saved = files
        .remove(&from.display().to_string())
        .expect("the original source path is saved");
    files.insert(to.display().to_string(), saved);
    std::fs::write(
        &path,
        serde_json::to_vec(&doc).expect("the cursor file serialises"),
    )
    .expect("the cursor path substitution is written");
}

/// Replace the persisted identity and offset for one saved line source. A restart-security
/// test uses this to make a modified cursor internally consistent with a planted target.
pub fn set_file_cursor_facts(h: &EntryHarness, store: &str, file: &Path, inode: u64, offset: u64) {
    let path = h.artifact_dir.join("cursors.json");
    let text = std::fs::read_to_string(&path).expect("the cursor file reads");
    let mut doc: serde_json::Value = serde_json::from_str(&text).expect("cursors.json is JSON");
    let cursor = doc
        .get_mut(store)
        .and_then(|line| line.get_mut("cursor"))
        .and_then(serde_json::Value::as_object_mut)
        .expect("the store keeps a cursor");
    let state = if cursor.contains_key("inode_offset") {
        cursor
            .get_mut("inode_offset")
            .expect("the inode-offset state exists")
    } else {
        cursor
            .get_mut("ordinal")
            .expect("the store keeps an inode-offset or ordinal line cursor")
    };
    let key = file.display().to_string();
    state["files"][&key]["inode"] = serde_json::json!(inode);
    state["files"][&key]["offset"] = serde_json::json!(offset);
    std::fs::write(
        &path,
        serde_json::to_vec(&doc).expect("the cursor file serialises"),
    )
    .expect("the modified cursor facts are written");
}

/// The seq of the first chained `Reread` notice for `store`, if there is one.
pub fn seq_of_reread(chain: &[Envelope], want: &str) -> Option<u64> {
    chain
        .iter()
        .find(|e| {
            matches!(&e.body, EventBody::TailNotice { kind, store, .. }
                     if *kind == TailNoticeKind::Reread && store == want)
        })
        .map(|e| e.link.seq)
}

/// The seq of the first leg-1 record of `store` above `above` — the re-read record a
/// witness has to precede.
pub fn first_leg1_seq_above(chain: &[Envelope], store: &str, above: u64) -> Option<u64> {
    chain
        .iter()
        .find(|e| {
            e.link.seq > above
                && matches!(&e.body, EventBody::Leg1Record { store: s, .. } if s == store)
        })
        .map(|e| e.link.seq)
}

pub fn seq_of_restart(chain: &[Envelope]) -> u64 {
    chain
        .iter()
        .find(|e| matches!(e.body, EventBody::DaemonRestart { .. }))
        .map(|e| e.link.seq)
        .expect("a restart is chained")
}

pub fn seq_of_tail_notice(chain: &[Envelope], want: TailNoticeKind) -> u64 {
    chain
        .iter()
        .find(|e| matches!(&e.body, EventBody::TailNotice { kind, .. } if *kind == want))
        .map(|e| e.link.seq)
        .unwrap_or_else(|| panic!("a {want:?} notice is chained"))
}

/// Is there a leg-1 record of `store` between two seqs? V9 spends a re-read witness on one,
/// so an honest restart must chain its `DaemonRestart` before the store's first record.
pub fn any_leg1_between(chain: &[Envelope], store: &str, from: u64, to: u64) -> bool {
    chain.iter().any(|e| {
        e.link.seq > from
            && e.link.seq < to
            && matches!(&e.body, EventBody::Leg1Record { store: s, .. } if s == store)
    })
}

/// The same entry, the same trace directory, a second runtime — a restarted daemon.
///
/// The buffer and the registry are the ones on disk; only the process is new.
pub async fn reopen_runtime(h: &EntryHarness) -> EntryRuntime {
    try_reopen_runtime(h)
        .await
        .expect("the runtime builds again")
}

/// The fallible form of [`reopen_runtime`], for recovery refusal controls.
pub async fn try_reopen_runtime(
    h: &EntryHarness,
) -> Result<EntryRuntime, goalcup_daemon::DaemonError> {
    // The old process is gone before the new one opens the buffer: redb takes a file lock.
    h.close();
    let prepared_files = tracker_files_of(h);
    EntryRuntime::new_rebuilt(
        h.entry(),
        h.manifest.clone(),
        &h.config,
        h.vars.clone(),
        prepared_files.0,
        prepared_files.1,
        goalcup_daemon::foreign::TrustedManifests::load(&h.config.connectors_dir)
            .expect("the shipped manifests are trusted")
            .plan_for(&h.manifest),
        h.registry.clone(),
    )
}

/// The tracker files and the allowlist, re-derived from the connector the way a restarting
/// daemon does: from the manifest and the bytes it renders, never from what is on disk.
fn tracker_files_of(
    h: &EntryHarness,
) -> (Vec<workspace::TrackerFile>, Vec<workspace::AllowlistedFile>) {
    let roots = Roots {
        workspace: h.workspace.clone(),
        artifact_root: h.artifact_dir.clone(),
        harness_home: h.vars.config_dir.clone(),
    };
    let token = h.entry().keys.receiver_token.clone();
    let installed = workspace::render_tracker_files(
        &h.manifest,
        &connector_dir(&h.connector),
        &super::template_vars(&roots, &token, "e1", harness_session(&h.connector)),
        &roots,
        false,
    )
    .expect("the tracker files render");
    let allowlist =
        workspace::baseline_allowlist(&h.manifest, &h.vars).expect("the allowlist baselines");
    (installed.files, allowlist)
}

/// The baseline this entry was created with, read back out of `baseline.json`.
fn baseline_of(h: &EntryHarness) -> goalcup_protocol::seal::SealManifest {
    let path = h.trace_dir.join("baseline.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).expect("baseline.json is a seal manifest")
}

/// The transcript shrank: the on-disk signature of a rewrite.
///
/// The file it leaves behind is not empty. An empty one hands the tick a batch
/// with a notice and no records, and every ordering assertion about a record lying between
/// the witness and the notice is then vacuous. One short record is what a rewrite actually
/// leaves: the harness rewrote the transcript, it did not delete it.
pub fn truncate_store(h: &EntryHarness) {
    std::fs::write(&h.store_path, b"{\"type\":\"response_item\",\"n\":0}\n")
        .expect("the store is truncated");
}

/// Nothing has been written for longer than `stall_after_seconds`, while the harness is
/// still alive. Signal 1 of D17.
pub fn stall_capture(h: &EntryHarness) {
    advance_clock(h, h.config.stall_after_seconds + 1.0);
}

/// The app-wide file the tracker baselined and may never restore, changed by the
/// participant: reported as a `ConfigDeviation` with `restored = false`,
/// which is what `HealthState::ConfigMismatch` is the state name for.
pub fn deviate_tracker_config(h: &EntryHarness) {
    std::fs::write(&h.allowlisted_path, b"model = \"something-else\"\n")
        .expect("the allowlisted file is changed");
}

/// The same file put back the way the daemon found it at launch — by the participant, since
/// this daemon never writes it. The digest matches the baseline
/// again, which is the fact `ConfigWatch::mismatched` reads and the reason a mismatch is a
/// state rather than a latch.
pub fn restore_tracker_config(h: &EntryHarness) {
    std::fs::write(&h.allowlisted_path, ALLOWLISTED_BYTES)
        .expect("the allowlisted file is put back");
}

/// Take the entry all the way to `Started`: the first leg-1 record, which is the start
/// signal on every connector, and the verified start receipt
/// that makes `started_at` a fact.
pub async fn start_entry(h: &EntryHarness) {
    append_line(&h.store_path, &session_meta_line(h));
    h.poll();
    // The runtime is taken out of the harness for the await and put back afterwards: a
    // `std::sync::MutexGuard` held across an await point is a deadlock waiting for a
    // multi-threaded runtime to schedule the other half.
    let mut runtime = h.take();
    runtime.bind_start(SESSION).await.expect("the run starts");
    h.give_back(runtime);
}

/// A process that is emphatically not the tracked harness.
pub fn spawn_unrelated() -> std::process::Child {
    std::process::Command::new("/bin/sleep")
        .arg("30")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a sleeper starts")
}

/// The handle the CLI recorded for the tracked harness, pointing at `child`'s pid.
///
/// That pairing: the participant killed the harness the tracker launched and
/// started their own by hand, and the number the daemon holds now belongs to something the
/// launch never registered. `program` and `argv` are what the CLI wrote down; the pid is
/// what is alive.
pub fn handle_for(child: std::process::Child) -> ProcessHandle {
    ProcessHandle {
        pid: child.id(),
        pgid: None,
        native_pid: None,
        native_binary: None,
        native_process_start: None,
        process_start: goalcup_protocol::process::process_start_identity(child.id()),
        strategy: KillStrategy::NativePid,
        grace_seconds: 10,
        launched_at: goalcup_protocol::control::format_z(1_756_944_000),
        program: "codex".to_string(),
        argv: vec!["codex".to_string()],
        env_keys: Vec::new(),
        home_seed: None,
    }
}

/// A structurally valid, deliberately non-live handle for handoff/registry tests. The
/// process route records CLI proof; health is responsible for testing whether that process
/// is later alive, so a deterministic test need not spawn one.
pub fn recorded_launch_handle() -> ProcessHandle {
    ProcessHandle {
        pid: 42,
        pgid: None,
        native_pid: None,
        native_binary: None,
        native_process_start: None,
        process_start: None,
        strategy: KillStrategy::NativePid,
        grace_seconds: 10,
        launched_at: goalcup_protocol::control::format_z(1_756_944_000),
        program: "codex".to_string(),
        argv: vec!["codex".to_string()],
        env_keys: Vec::new(),
        home_seed: None,
    }
}
