//! A whole run, driven through `goalcup_daemon`'s public API, against a
//! live `DevServer`, on a clock the test moves.
//!
//! Nothing here writes a trace file. Every byte in the directory this returns was written
//! by `TraceExport` under the daemon's own `ChainWriter`, and every server signature in it
//! was produced by the `DevServer` this crate ships. That is the difference between this
//! and `support::compliant`, which builds the same shape by hand: the hand-written fixture
//! proves the RULES are satisfiable, and this one proves the DAEMON satisfies them.
//!
//! # Three clocks, and which is which
//!
//! * **The wall clock.** `ChainWriter` stamps every `recorded_at` with `now_rfc3339()`, so
//!   every leg-1 record and leg-2 post in the chain is dated by the moment the test ran.
//!   That is not a seam a test can move, and it does not need to be: V13 measures records
//!   against `[started_at, sealed_at]`, and both of those are produced during the same run.
//! * **The server's clock**, which `DevServer::set_clock` moves. It dates `started_at` (and
//!   `deadline_at`, a week later) and every `acked_at`. It is set to the wall clock at the
//!   moment the receipt is requested — which is AFTER the arming records were chained, and
//!   is what a real server's clock does — and then stepped forward an hour at a time, so
//!   V12's `anchor_timeline` measures a real series instead of one anchor.
//!
//!   It is not set below the run so that `started_at` opens under every record: a
//!   connector's arming signal is the harness's own first output, so the records that cause
//!   the receipt to be requested are always chained before it is issued, and no real
//!   participant can arrange a start that precedes them. This fixture produces the
//!   pre-start records an honest run produces, and V13 reports them as `pre_start_records`
//!   at Pass — which `roundtrip_verify.rs` asserts.
//! * **The poll clock**, the `now` `EntryRuntime::poll_once_at` takes. It carries the
//!   heartbeat cadence, so stepping it an hour at a time is what puts a heartbeat an hour
//!   apart in the chain — a hole, by the connector's own `heartbeat_seconds = 60`, and a
//!   CLEAN one, because nothing is appended inside it and an acknowledgement covers its far
//!   edge.
//!
//! # Why the work happens before the first heartbeat
//!
//! `gaps::classify` calls the stretch between two heartbeats a hole, and a hole with leg-1
//! records inside it is `RecordsAppended` — Dirty, at Fail. `poll_once_at` drains the
//! stores and *then* beats, so the first poll of the run chains the records it captured
//! below its own heartbeat. Every later record therefore has to be captured by that same
//! first poll, or it lands between two beats and turns an honest run into a dirty gap. So
//! the fixture writes the whole transcript **before** the first poll,
//! which is also how a real run reads: the harness writes at its own pace and the daemon
//! catches up.

#![allow(dead_code)]

use goalcup_daemon::config::DaemonConfig;
use goalcup_daemon::paths::{PathVars, claude_slug};
use goalcup_daemon::registry::{Entry, EntryState, RunKeys};
use goalcup_daemon::runtime::{Daemon, EntryRuntime};
use goalcup_daemon::workspace::{self, Roots};
use goalcup_devserver::{DevServer, dev_router};
use goalcup_protocol::control::format_z;
use goalcup_protocol::manifest::ConnectorManifest;
use goalcup_protocol::template::TemplateVars;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The entry every run in this module is recorded under.
pub const ENTRY_ID: &str = "roundtrip-0001";
/// The session the harness writes its transcript in, and the one the receipt signs.
pub const SESSION_ID: &str = "roundtrip-session-0001";
/// Inside `tested_version_range = ">=2.1.259"`, so V5 reports no version drift.
pub const HARNESS_VERSION: &str = "2.1.259";
/// The goal, under `MAX_PROMPT_CHARS`.
pub const GOAL: &str = "write hello.py that prints the current date";
/// Every human message after the goal must be exactly this (V2, RULES.md).
pub const KEEP_GOING: &str = "keep going";
/// The file the agent writes, named in full by the traced `Write` call so the seal
/// attributes it and `unexplained_seal_file` stays silent.
pub const WRITTEN_FILE: &str = "hello.py";
/// What the agent puts in it.
pub const WRITTEN_BODY: &str = "from datetime import date\nprint(date.today())\n";

/// How many hourly acknowledgements the run carries.
///
/// Enough that `anchor_timeline` measures a series — a first anchor, twelve spans between
/// consecutive ones, and a tail — rather than the single anchor before the deadline.
/// Every span is one hour, comfortably inside V12's
/// `MAX_UNANCHORED_SPAN_SECONDS`.
pub const HOURS: u64 = 12;

/// One second of the poll clock, in seconds.
const HOUR: f64 = 3_600.0;

/// A run this daemon captured, and the directory a verifier reads it out of.
pub struct CompliantRun {
    pub trace_dir: PathBuf,
    pub entry_id: String,
    pub workspace: PathBuf,
    /// The key that signed the receipt and every acknowledgement, hex. The placeholder for
    /// [`compliant_run`]; whatever seed was asked for in [`compliant_run_signed_by`].
    pub server_public_key: String,
    /// Dropped last, so the trace directory outlives every `verify_trace` the caller runs.
    _tmp: tempfile::TempDir,
}

/// Drives a whole run through the daemon's own code against a live `goalcup-devserver` on
/// its DEFAULT key — the placeholder, which `goalcup_verifier::SERVER_PUBLIC_KEYS` pins —
/// on a compressed clock, and returns the trace directory.
pub async fn compliant_run() -> CompliantRun {
    drive(DevServer::new()).await
}

/// The same run against `DevServer::with_seed`, for the one unpinned-anchor case. Same
/// steps; only the seed and `DaemonConfig.server_public_key` differ.
pub async fn compliant_run_signed_by(seed: &[u8; 32]) -> CompliantRun {
    drive(DevServer::with_seed(*seed)).await
}

async fn drive(server: DevServer) -> CompliantRun {
    let server = Arc::new(server);
    let upload_url = serve(server.clone()).await;

    let tmp = tempfile::tempdir().expect("a temporary directory");
    // Canonical from the start: on macOS a tempdir is handed out as `/var/folders/…` while
    // a harness records `/private/var/folders/…` in its own `cwd`, and the partitioner and
    // V10 both compare the two byte for byte.
    let root = std::fs::canonicalize(tmp.path()).expect("the tempdir canonicalises");
    let workspace = root.join("workspaces/ws-one");
    let home = root.join("home");
    let artifact_dir = root.join("holder/artifacts").join(ENTRY_ID);
    let trace_dir = artifact_dir.join("trace");

    let manifest = manifest("claude-code-cli");
    let config = config_at(&root, &upload_url, &server.public_key_hex());
    let daemon = Daemon::start(config.clone())
        .await
        .expect("the daemon starts");

    // ---- 2. the workspace, the entry and the runtime. `register` calls `open_trace`,
    // which chains the baseline as seq 0 before anything else this daemon does can
    // append — the link the start receipt witnesses.
    let runtime = build_runtime(&daemon, &manifest, &config, &workspace, &home).await;
    daemon.register(runtime).expect("the entry is registered");

    // ---- 3. arm, which runs `foreign::pre_arm_scan` first, over an empty harness home.
    daemon.arm(ENTRY_ID).expect("an empty harness home arms");

    // ---- 4-7. the whole of what the harness wrote, BEFORE the first poll. See the module
    // header: everything captured after the first heartbeat lands inside a hole. There is
    // no hook leg, so nothing is posted: the transcript is the only capture leg.
    let store_path = transcript_path(&home, &workspace, SESSION_ID);
    let ws = workspace.display().to_string();

    append_line(&store_path, &typed_prompt(&ws, "prompt-1", GOAL));

    append_line(&store_path, &assistant_write(&ws, "toolu_01"));
    // The agent's own file. Written here rather than by the daemon, because it is the
    // participant's harness that writes into a workspace and the seal's job is to notice.
    std::fs::write(workspace.join(WRITTEN_FILE), WRITTEN_BODY).expect("the agent's file");
    append_line(&store_path, &tool_result(&ws, "toolu_01"));

    append_line(&store_path, &typed_prompt(&ws, "prompt-2", KEEP_GOING));

    // One poll: leg 1 has grown, the gate has its signal and its corroboration, and this
    // tick's heartbeat closes the work below it.
    let t0 = goalcup_daemon::now_unix();
    poll_at(&daemon, t0);

    // ---- 5. the receipt. The server's clock dates it; the daemon's claims are overwritten.
    //
    // Read HERE, and not before the run: `started_at` is the server's clock at the moment
    // the receipt is issued, and by this line the baseline, the arming record, the work and
    // the first heartbeat are already chained. So this fixture produces what every honest
    // run produces — records dated below `started_at` — rather than a margin that hid them.
    cross_a_second_boundary().await;
    let opened_at = goalcup_daemon::now_unix() as u64;
    server.set_clock(&format_z(opened_at));
    daemon
        .bind_start(ENTRY_ID, SESSION_ID)
        .await
        .expect("the receipt is issued, verified and stamped");

    // ---- 8. the week, an hour at a time: a heartbeat per step, and an acknowledgement
    // over it. This is what makes `anchor_timeline` a real check rather than one anchor
    // before the deadline.
    for hour in 1..=HOURS {
        poll_at(&daemon, t0 + hour as f64 * HOUR);
        server.set_clock(&format_z(opened_at + hour * 3_600));
        pump(&daemon).await;
    }

    // ---- 9. the seal: the workspace manifest, the second foreign scan, the chained
    // `EventBody::Seal` and `seal.json`, then termination — nothing was spawned here, so
    // the kill has an empty target list.
    daemon.seal_entry(ENTRY_ID).expect("the entry seals");

    // ---- 10. one last acknowledgement, so the seal link itself is anchored,
    // and no record sits above the highest acknowledged sequence.
    server.set_clock(&format_z(opened_at + (HOURS + 1) * 3_600));
    pump(&daemon).await;

    CompliantRun {
        trace_dir,
        entry_id: ENTRY_ID.to_string(),
        workspace,
        server_public_key: server.public_key_hex(),
        _tmp: tmp,
    }
}

/// Wait until the wall clock has passed the next whole second, and no longer.
///
/// The one wait in this fixture, and it buys the property the whole window check is
/// about. A `recorded_at` is stamped at whole-second granularity and so is `started_at`,
/// and the fixture reaches its receipt in under a millisecond — so without this, the
/// arming records and the receipt land in the SAME second and `recorded_at < started_at`
/// is false by an accident of speed. A real participant's start takes a launch, a first
/// transcript record and an HTTP round trip, and its arming records are below `started_at`
/// every time.
///
/// It waits to the boundary and 50ms past it rather than a flat second: the cost is half a
/// second on average, the crossing is guaranteed, and it is a condition rather than a
/// duration — the thing being waited for is the tick of the clock the two timestamps are
/// rounded to.
async fn cross_a_second_boundary() {
    let now = goalcup_daemon::now_unix();
    let remaining = 1.0 - now.fract() + 0.05;
    tokio::time::sleep(std::time::Duration::from_secs_f64(remaining)).await;
}

// ---------------------------------------------------------------------------------------
// Driving the daemon step by step.
// ---------------------------------------------------------------------------------------

/// One poll at a clock the fixture chooses.
///
/// `Daemon::tick` reads the wall clock and would also bind the start on its own; this
/// fixture wants both of those in its own hands, so it borrows the entry and calls the
/// runtime — which `Daemon::borrow_entry` exists for.
fn poll_at(daemon: &Daemon, now: f64) {
    let mut held = daemon.borrow_entry(ENTRY_ID).expect("the entry is free");
    held.get()
        .expect("the runtime is in its slot")
        .poll_once_at(now)
        .expect("the poll runs");
}

/// One drain of the write-ahead buffer, and the acknowledgement it earns.
///
/// The outcome is asserted rather than dropped: an offline or refused pump leaves
/// `acks.jsonl` short, and a fixture that swallowed it would hand the verifier a trace with
/// a hole in its anchor series and blame the verifier for noticing.
async fn pump(daemon: &Daemon) {
    let mut held = daemon.borrow_entry(ENTRY_ID).expect("the entry is free");
    let out = held
        .get()
        .expect("the runtime is in its slot")
        .pump()
        .await
        .expect("the pump runs");
    assert!(
        !out.offline && out.rejected.is_none() && !out.ack_unverified && !out.ack_mismatched,
        "the devserver acknowledged nothing: {out:?}"
    );
}

// ---------------------------------------------------------------------------------------
// The entry, its workspace and its configuration.
// ---------------------------------------------------------------------------------------

fn connector_dir(id: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors")
        .join(id)
}

/// The shipped connector manifest, read from `connectors/<id>/manifest.toml`. The same file
/// `ChainWriter::copy_manifest` puts beside the chain, so the digest pin resolves to
/// `ManifestPin::Pinned`.
pub fn manifest(id: &str) -> ConnectorManifest {
    let text = std::fs::read_to_string(connector_dir(id).join("manifest.toml"))
        .expect("the shipped manifest");
    ConnectorManifest::from_toml_str(&text).expect("the shipped manifest parses")
}

/// The daemon's own configuration for a run rooted at `root`.
///
/// `workspace_parent` and `artifact_root` are disjoint trees, which `DaemonConfig::validate`
/// enforces: an agent enumerates its workspace's parent directory on its first tool call.
///
/// The three health thresholds are set past any clock this fixture reaches. They decide
/// nothing a verifier reads — no check in the crate looks at `EventBody::Health` — and
/// leaving them at their defaults would chain a `CaptureStalled` the moment the poll clock
/// stepped an hour, which is noise in a fixture whose whole value is that a reader can look
/// at its chain and see a compliant run.
fn config_at(root: &Path, upload_url: &str, server_public_key: &str) -> DaemonConfig {
    DaemonConfig {
        artifact_root: root.join("holder"),
        connectors_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors"),
        config_dir: root.join("config"),
        workspace_parent: root.join("workspaces"),
        control_addr: "127.0.0.1:0".parse().expect("a loopback address"),
        upload_url: upload_url.to_string(),
        server_public_key: server_public_key.to_string(),
        receipt_binding: goalcup_daemon::config::ReceiptBindingPolicy::Legacy,
        poll_interval_ms: 250,
        upload_interval_ms: 2_000,
        upload_chunk_max: 256,
        status_interval_ms: 1_000,
        stall_after_seconds: f64::MAX,
        silence_window_seconds: f64::MAX,
        ack_lag_seconds: f64::MAX,
        arming_timeout_seconds: None,
        heartbeat_seconds: 60,
        clock_skew_seconds: f64::MAX,
        disk_reserve_bytes: 0,
        // No round configured: the signed status carries the entry's own timeline and
        // says so, through `event_status_source: "fallback"`.
        event: None,
    }
}

/// The template variables the tracker supplies when it renders a connector's hook files.
fn template_vars(roots: &Roots, token: &str, receiver_http: &str) -> TemplateVars {
    let mut v = TemplateVars::new();
    v.set("workspace", roots.workspace.to_string_lossy().to_string());
    v.set(
        "artifact_root",
        roots.artifact_root.to_string_lossy().to_string(),
    );
    v.set(
        "harness_home",
        roots.harness_home.to_string_lossy().to_string(),
    );
    v.set("receiver_http", receiver_http.to_string());
    v.set("receiver_token", token);
    v.set("entry_id", ENTRY_ID);
    v.set("run_id", ENTRY_ID);
    v.set("session_id", SESSION_ID);
    v.set("server_port", "7788");
    v.set(
        "status_line_path",
        roots
            .artifact_root
            .join("status-line.txt")
            .to_string_lossy()
            .to_string(),
    );
    v
}

/// One entry: its workspace prepared, its row in the daemon's OWN registry, and the runtime
/// around it.
///
/// The registry comes from the daemon rather than being opened a second time: redb takes a
/// file lock, so a second `Registry::open` on one path is a failure and not a second handle
/// — which is why `EntryRuntime::new` takes the handle as an argument.
async fn build_runtime(
    daemon: &Daemon,
    manifest: &ConnectorManifest,
    config: &DaemonConfig,
    workspace: &Path,
    home: &Path,
) -> EntryRuntime {
    let artifact_dir = config.artifact_root.join("artifacts").join(ENTRY_ID);
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
        None,
        ("2026".to_string(), "09".to_string(), "04".to_string()),
    );
    // Empty: the daemon runs no hook receiver.
    let receiver_http = String::new();
    let prepared = workspace::prepare(
        manifest,
        &connector_dir(&manifest.id),
        &template_vars(&roots, &keys.receiver_token, &receiver_http),
        &vars,
        &roots,
        ENTRY_ID,
    )
    .expect("the workspace is prepared");

    let manifest_bytes = std::fs::read(connector_dir(&manifest.id).join("manifest.toml"))
        .expect("the shipped manifest");
    let mut entry = Entry::new(
        ENTRY_ID,
        &manifest.id,
        workspace.to_path_buf(),
        artifact_dir.clone(),
        &workspace::sha256_hex(&manifest_bytes),
        HARNESS_VERSION,
        goalcup_daemon::config::ReceiptBindingPolicy::Legacy,
        goalcup_daemon::registry::ExpectedProcessHandoff::NotRequired,
        keys,
    );
    entry.harness_home = Some(home.to_path_buf());
    // What `control::handlers::build_entry` sets from its own preflight: the digest of the
    // connector directory the workspace was just rendered from. Taken over the same
    // directory `workspace::prepare` read, so the receipt this run asks for claims the
    // shipped connector and V13 raises neither `connector_dir_unbound` nor
    // `connector_dir_unpinned`.
    entry.connector_dir_sha256 = Some(
        goalcup_protocol::connector_dir::connector_dir_sha256(&connector_dir(&manifest.id))
            .expect("the shipped connector directory hashes"),
    );
    entry.expected_config = prepared.expected_config.clone();
    entry.status_line_path = artifact_dir.join("status-line.txt");
    entry.state = EntryState::Created;
    let registry = daemon.registry();
    registry.put(&entry).expect("the entry is stored");

    EntryRuntime::new(
        entry,
        manifest.clone(),
        config,
        vars,
        prepared.installed.files.clone(),
        prepared.allowlist.clone(),
        prepared.baseline.clone(),
        goalcup_daemon::foreign::TrustedManifests::load(&config.connectors_dir)
            .expect("the shipped manifests are trusted")
            .plan_for(manifest),
        registry,
    )
    .expect("the runtime builds")
}

// ---------------------------------------------------------------------------------------
// The synthetic harness: the records and the posts.
// ---------------------------------------------------------------------------------------

/// Where Claude writes one session's transcript: the per-run config dir, the project slug
/// it derives from the working directory, and the session id.
pub fn transcript_path(home: &Path, workspace: &Path, session: &str) -> PathBuf {
    home.join("projects")
        .join(claude_slug(workspace))
        .join(format!("{session}.jsonl"))
}

/// Append one line to a store the harness owns, creating it the way the harness would.
pub fn append_line(path: &Path, line: &str) {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the parent directory");
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("the store opens for append");
    writeln!(file, "{line}").expect("the line is appended");
}

/// A prompt the participant typed, in the shape `spikes/claude-code-cli/fixtures/
/// main-session.jsonl` record 5 carries: `origin.kind` and `promptSource` are the
/// connector's own positive test, `cwd` is what V10 measures the tracked session's
/// directory by, and `promptId` is the `prompt_id` join both legs stamp.
fn typed_prompt(workspace: &str, prompt_id: &str, text: &str) -> String {
    json!({
        "parentUuid": serde_json::Value::Null,
        "isSidechain": false,
        "promptId": prompt_id,
        "type": "user",
        "message": { "role": "user", "content": text },
        "uuid": format!("uuid-{prompt_id}"),
        "timestamp": "2026-09-04T00:00:00.000Z",
        "permissionMode": "auto",
        "origin": { "kind": "human" },
        "promptSource": "typed",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": workspace,
        "sessionId": SESSION_ID,
        "version": HARNESS_VERSION,
    })
    .to_string()
}

/// The agent's one turn: a `Write` naming the full path of the file the seal will hold, and
/// the token usage V11 reads.
fn assistant_write(workspace: &str, tool_use_id: &str) -> String {
    json!({
        "parentUuid": "uuid-prompt-1",
        "isSidechain": false,
        "message": {
            "model": "claude-opus-5",
            "id": "msg-1",
            "type": "message",
            "role": "assistant",
            "content": [
                { "type": "text", "text": "Writing hello.py." },
                {
                    "type": "tool_use",
                    "id": tool_use_id,
                    "name": "Write",
                    "input": {
                        "file_path": format!("{workspace}/{WRITTEN_FILE}"),
                        "content": WRITTEN_BODY,
                    },
                },
            ],
            "stop_reason": "tool_use",
            "usage": {
                "input_tokens": 1_200,
                "output_tokens": 340,
                "cache_read_input_tokens": 0,
                "cache_creation_input_tokens": 0,
            },
        },
        "requestId": "req-1",
        "type": "assistant",
        "uuid": "uuid-assistant-1",
        "timestamp": "2026-09-04T00:00:10.000Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": workspace,
        "sessionId": SESSION_ID,
        "version": HARNESS_VERSION,
    })
    .to_string()
}

/// The tool's answer, in the user slot and carrying a `tool_result` block — which is how
/// the parser tells a continuation of a turn from a new one, so this occupies no user slot
/// and V6 has nothing to exclude.
fn tool_result(workspace: &str, tool_use_id: &str) -> String {
    json!({
        "parentUuid": "uuid-assistant-1",
        "isSidechain": false,
        "promptId": "prompt-1",
        "type": "user",
        "message": {
            "role": "user",
            "content": [{
                "tool_use_id": tool_use_id,
                "type": "tool_result",
                "content": format!("Wrote {WRITTEN_FILE}"),
                "is_error": false,
            }],
        },
        "uuid": "uuid-tool-result-1",
        "timestamp": "2026-09-04T00:00:11.000Z",
        "sourceToolAssistantUUID": "uuid-assistant-1",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": workspace,
        "sessionId": SESSION_ID,
        "version": HARNESS_VERSION,
    })
    .to_string()
}

// ---------------------------------------------------------------------------------------
// The two loopback clients this fixture needs.
// ---------------------------------------------------------------------------------------

/// The devserver, on a loopback port the OS chose, in this process.
async fn serve(server: Arc<DevServer>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port");
    let addr = listener.local_addr().expect("the bound address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, dev_router(server)).await;
    });
    format!("http://{addr}")
}

// ---------------------------------------------------------------------------------------
// Reading a finished trace back, and the two edits the failure cases make.
// ---------------------------------------------------------------------------------------

/// Every link in the chain, in file order — which is seq order.
pub fn read_chain(trace_dir: &Path) -> Vec<goalcup_protocol::envelope::Envelope> {
    let path = trace_dir.join(goalcup_protocol::export::CHAIN_FILE);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("a chain line is an envelope"))
        .collect()
}

/// Rewrite the body of the link at `index` without touching its stamped `hash`, which is
/// what a participant editing their own evidence in place would leave behind.
///
/// The `recorded_at` is what moves: it is one string on a line the link hash covers, so the
/// recomputed hash no longer matches and `chain_broken` is the only honest reading.
pub fn rewrite_one_chain_line(trace_dir: &Path, index: usize) {
    let path = trace_dir.join(goalcup_protocol::export::CHAIN_FILE);
    let text = std::fs::read_to_string(&path).expect("the chain");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut env: serde_json::Value =
        serde_json::from_str(&lines[index]).expect("a chain line is JSON");
    env["recorded_at"] = json!("2001-01-01T00:00:00Z");
    lines[index] = env.to_string();
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).expect("the edited chain");
}

/// Add a planted file to `baseline.json` after the fact.
///
/// `baseline.json` alone is a file the participant's own daemon wrote, so a participant who
/// pre-populated the workspace could list every planted file in it and have attribution
/// exempt each one. The chain covers its digest at seq 0, so this moves the head the server
/// signed and `baseline_not_chained` says so.
pub fn rewrite_baseline_json(trace_dir: &Path) {
    let path = trace_dir.join(goalcup_protocol::export::BASELINE_FILE);
    let mut baseline: goalcup_protocol::seal::SealManifest =
        serde_json::from_slice(&std::fs::read(&path).expect("the baseline")).expect("it parses");
    baseline.files.push(goalcup_protocol::seal::SealedFile {
        path: "planted.py".to_string(),
        bytes: 4,
        sha256: "aa".repeat(32),
    });
    std::fs::write(
        &path,
        serde_json::to_vec(&baseline.sorted()).expect("serialise the baseline"),
    )
    .expect("the edited baseline");
}
