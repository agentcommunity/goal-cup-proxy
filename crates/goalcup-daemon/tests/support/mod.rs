//! Fixtures shared by the integration tests in this crate; it is compiled into every test
//! binary that declares `mod support`.
//!
//! What is here directly: a registry with one entry, the shipped manifests, a devserver
//! behind a recording proxy, and the `TrackerFile` builder the config-watch tests start from. The bigger
//! harnesses are the submodules — [`entry`] (one `EntryRuntime`, driven by hand), [`daemon`]
//! (the process-level object), [`control`] (its control listener and a client) and
//! [`status`] (a status document and the reader that checks its signature).

#![allow(dead_code)]

// The upload harness: a recording proxy in front of a real devserver.
#[cfg(feature = "v1-compat")]
pub mod mock_server;
// Re-exported so a test says `support::harness_with_devserver`; `allow` because every test
// binary in this directory compiles this module and only one of them uploads.
#[cfg(feature = "v1-compat")]
#[allow(unused_imports)]
pub use mock_server::*;

// Health, status and page fixtures.
pub mod status;
// The entry harness: one real entry, one real chain, one clock the test moves.
#[cfg(feature = "v1-compat")]
pub mod entry;
#[cfg(feature = "v1-compat")]
#[allow(unused_imports)]
pub use entry::*;
// The daemon harness: the process-level object, with the entry map inside it.
pub mod daemon;
#[allow(unused_imports)]
pub use daemon::*;
// The control harness: the control listener, its bearer, and a client that sends a
// request line byte for byte so the traversal cases can be written at all.
pub mod control;
#[allow(unused_imports)]
pub use control::*;
// The process-group escape helper (re-exec this test binary as a hard-linked `codex`
// under a pre-`main` role dispatch). Unix-only, matching `process_kill.rs`'s own gate;
// `process_kill.rs` itself pulls this file in directly via `#[path = "support/tree.rs"]`
// rather than through this module, so its binary does not also link `mock_server`'s axum
// and tokio for a file that spawns `sh` and `sleep`.
#[cfg(unix)]
pub mod tree;
// Re-exported so a test says `support::inputs()`; the
// module path stays available for a caller that prefers `support::status::payload_for`.
#[allow(unused_imports)]
pub use status::*;

use goalcup_daemon::registry::{Entry, ExpectedProcessHandoff, Registry, RunKeys};
use goalcup_protocol::manifest::ConnectorManifest;
use std::sync::Arc;

/// The shipped connector manifests, read from `connectors/<id>/manifest.toml`. Loading the
/// real file rather than a hand-written stub is the whole point of the block-shape test:
/// a manifest that changed its `response_shape` has to fail here.
pub fn manifest(id: &str) -> ConnectorManifest {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors")
        .join(id)
        .join("manifest.toml");
    let text = std::fs::read_to_string(&path).expect("the shipped manifest");
    ConnectorManifest::from_toml_str(&text).expect("the shipped manifest parses")
}

/// The template variables the tracker supplies, as `tests/workspace_layout.rs` enumerates
/// them.
///
/// `receiver_http` and `server_port` are empty, which is what `control::tracker_vars`
/// renders in production, where the daemon runs no hook receiver. It has to match: a
/// restart rebuilds these files from the templates and the entry's row, and a fixture whose
/// installed bytes differed from that re-render would meet it as a config deviation on the
/// first poll after one — a difference this harness invented, in a file the watch would
/// then put back.
pub fn template_vars(
    roots: &goalcup_daemon::workspace::Roots,
    token: &str,
    entry_id: &str,
    session: &str,
) -> goalcup_protocol::template::TemplateVars {
    let mut v = goalcup_protocol::template::TemplateVars::new();
    v.set("workspace", roots.workspace.to_string_lossy().to_string());
    v.set(
        "artifact_root",
        roots.artifact_root.to_string_lossy().to_string(),
    );
    v.set(
        "harness_home",
        roots.harness_home.to_string_lossy().to_string(),
    );
    v.set("receiver_http", "");
    v.set("receiver_token", token);
    v.set("entry_id", entry_id);
    v.set("run_id", entry_id);
    v.set("session_id", session);
    v.set("server_port", "");
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

/// A registry holding one created entry, bound to `workspace`.
///
/// Returns the `TempDir` as well as the registry, because the redb file lives in it and a
/// dropped `TempDir` takes the database with it. The directory is returned rather than
/// leaked, so a test run leaves nothing
/// behind.
pub fn registry_with_entry(id: &str, workspace: &str) -> (tempfile::TempDir, Registry, Entry) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let registry = Registry::open(&Registry::default_path(dir.path())).expect("the registry opens");
    let entry = entry_in(&registry, dir.path(), id, "claude-code-cli", workspace);
    (dir, registry, entry)
}

/// One entry, put into `registry` and returned as it was stored.
pub fn entry_in(
    registry: &Registry,
    root: &std::path::Path,
    id: &str,
    connector_id: &str,
    workspace: &str,
) -> Entry {
    let entry = Entry::new(
        id,
        connector_id,
        std::path::PathBuf::from(workspace),
        root.join("artifacts").join(id),
        &"ab".repeat(32),
        "0.0.0-test",
        goalcup_daemon::config::ReceiptBindingPolicy::Legacy,
        ExpectedProcessHandoff::NotRequired,
        RunKeys::generate(),
    );
    registry.put(&entry).expect("the entry is stored");
    entry
}

/// One envelope at `seq`, linked to the one before it.
///
/// Seq 0 is the baseline, and every seq above it is a leg-1 record. That is not
/// decoration: the devserver the uploader posts to refuses an entry's first chunk unless its
/// seq 0 is an `EventBody::Baseline` whose hash is the one the start receipt witnessed, so a
/// helper that stamped a leg-1 record at seq 0 would make every upload test assert against a
/// server that refused it. It is also what a real run produces: `TraceExport::write_baseline`
/// is the genesis link.
///
/// A real chain, not a shape: `prev` is the previous envelope's `hash` (`GENESIS` at
/// seq 0) and `hash` is `Envelope::expected_hash`, because the devserver the uploader posts
/// to acknowledges only chunks whose chain it verified. A helper that stamped plausible
/// hex would make every upload test assert against a server that refused it.
///
/// Deterministic all the way down — `entry_id`, `recorded_at` and the payload are
/// functions of `seq` alone — which is what lets envelope `n` be recomputed from the
/// genesis link without the caller threading a chain through the test.
pub fn envelope(seq: u64, store: &str) -> goalcup_protocol::envelope::Envelope {
    let mut prev = goalcup_protocol::chain::GENESIS.to_string();
    let mut env = unlinked(0, store);
    for n in 0..=seq {
        env = unlinked(n, store);
        env.link.prev = prev.clone();
        env.link.hash = env.expected_hash(&prev).expect("the envelope hashes");
        prev = env.link.hash.clone();
    }
    env
}

/// The envelope's content at `seq`, with the link left unstamped.
fn unlinked(seq: u64, store: &str) -> goalcup_protocol::envelope::Envelope {
    goalcup_protocol::envelope::Envelope {
        protocol_version: goalcup_protocol::PROTOCOL_VERSION,
        entry_id: "e1".to_string(),
        source: store.to_string(),
        // Off the clock on purpose: `envelope(n)` must be the same envelope every time it
        // is called, or the chain it recomputes would not be the chain it built before.
        recorded_at: goalcup_protocol::control::format_z(1_756_944_000 + seq),
        link: goalcup_protocol::chain::Link {
            seq,
            prev: String::new(),
            hash: String::new(),
        },
        body: match seq {
            0 => goalcup_protocol::envelope::EventBody::Baseline {
                manifest_sha256: "ba".repeat(32),
                file_count: 3,
            },
            _ => goalcup_protocol::envelope::EventBody::Leg1Record {
                store: store.to_string(),
                payload: serde_json::Value::String(format!(
                    r#"{{"type":"user","seq":{seq},"text":"record {seq}"}}"#
                )),
            },
        },
        redaction: None,
    }
}

// ---------------------------------------------------------------------------------------
// The arming gate, the pre-arm scan and the signed start receipt.
// ---------------------------------------------------------------------------------------

/// The nine path values, with `home` and `config_dir` both pointed at `home`.
///
/// One directory for both because a per-run harness home is the config dir the connector's
/// own stores hang off (`CODEX_HOME`, `CLAUDE_CONFIG_DIR`), and a foreign scan rooted at
/// `{home}` is looking at the same tree from the participant's side. `session_id` is
/// unbound, so `{session_id}` renders as `*` and the glob matches every session file.
pub fn path_vars(
    workspace: &std::path::Path,
    home: &std::path::Path,
) -> goalcup_daemon::paths::PathVars {
    goalcup_daemon::paths::PathVars::new(
        workspace.to_path_buf(),
        home.to_path_buf(),
        home.to_path_buf(),
        home.join("artifacts"),
        None,
        ("2026".to_string(), "09".to_string(), "04".to_string()),
    )
}

/// One Codex rollout in `home`, laid out the way `codex-cli`'s `rollout` store declares it
/// (`{config_dir}/sessions/{yyyy}/{mm}/{dd}/rollout-*.jsonl`), whose first record is the
/// `session_meta` the store's `file_first_record` partition decides the file by.
///
/// The date components are deliberately not today's: the pre-arm scan asks whether the home
/// holds a tracked session at all, not whether it holds one written this morning.
pub fn plant_rollout(home: &std::path::Path, workspace: &std::path::Path, session_id: &str) {
    let dir = home.join("sessions/2026/08/29");
    std::fs::create_dir_all(&dir).expect("the rollout directory");
    let line = serde_json::json!({
        "timestamp": "2026-08-29T09:12:33.401Z",
        "type": "session_meta",
        "payload": {
            "id": session_id,
            "cwd": workspace.display().to_string(),
            "cli_version": "0.153.0",
        },
    });
    let file = dir.join(format!("rollout-2026-08-29T09-12-33-{session_id}.jsonl"));
    std::fs::write(&file, format!("{line}\n")).expect("the rollout file");
}

/// One Claude Code transcript under `root`'s `.claude`, in the project slug Claude derives
/// from the working directory — which is what `claude-code-cli`'s first `foreign_scan`
/// glob (`{home}/.claude/projects/*/*.jsonl`) walks.
///
/// The first record carries `secret` as the prompt text, so a scan that publishes one byte
/// of it fails loudly rather than by inspection.
pub fn plant_claude_slug(
    root: &std::path::Path,
    workspace: &std::path::Path,
    session_id: &str,
    secret: &str,
) {
    let dir = root
        .join(".claude/projects")
        .join(goalcup_daemon::paths::claude_slug(workspace));
    std::fs::create_dir_all(&dir).expect("the projects directory");
    let line = serde_json::json!({
        "type": "user",
        "cwd": workspace.display().to_string(),
        "sessionId": session_id,
        "isSidechain": false,
        "message": { "role": "user", "content": secret },
    });
    std::fs::write(dir.join(format!("{session_id}.jsonl")), format!("{line}\n"))
        .expect("the transcript file");
}

/// The claims a daemon asserts, with the two timestamps the server owns handed in — a test
/// that pins the dev server's clock knows what it will overwrite them with.
pub fn claims(
    entry_id: &str,
    started_at: &str,
    deadline_at: &str,
) -> goalcup_protocol::receipt::StartReceiptClaims {
    goalcup_protocol::receipt::StartReceiptClaims {
        entry_id: entry_id.to_string(),
        connector_id: "claude-code-cli".to_string(),
        workspace: format!("/ws/{entry_id}"),
        session_id: "s1".to_string(),
        started_at: started_at.to_string(),
        deadline_at: deadline_at.to_string(),
        manifest_sha256: "ab".repeat(32),
        chain_head: "cd".repeat(32),
        // A digest no build pins, deliberately: these claims are asserted by a test playing
        // the daemon, and the devserver signs the connector claim exactly as it arrives, so
        // a made-up value is what proves it travelled rather than what proves it is shipped.
        connector_dir_sha256: Some("ef".repeat(32)),
        run_public_key: None,
        questionnaire_version: None,
    }
}

/// The per-run key an upload is signed with. Minted here, never in `src/` fixtures: a run
/// key is the participant's.
pub fn run_public_key() -> String {
    hex::encode(
        ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])
            .verifying_key()
            .to_bytes(),
    )
}

/// The instant this process pins the fixture dev server's clock to: one hour before the
/// first call, so the 168-hour window it derives always contains now.
///
/// The pin is relative to now. The window a
/// receipt carries is an absolute instant, `StatusPayload::seconds_remaining` counts down
/// to it off the wall clock, and `Daemon::run_forever`'s deadline cadence seals an entry
/// whose deadline has passed, so a fixed pin eventually hands every harness a window
/// already spent. A pin far enough out to be safe breaks the other assertion these fixtures
/// make, that a started run's countdown is no longer than the window itself.
///
/// Computed once per process so two calls agree: a test that asserts the spelling the
/// server used has to be able to name it, and a second reading a second later would not be
/// the same string.
pub fn pinned_start() -> String {
    goalcup_protocol::control::format_z(pinned_start_secs())
}

/// [`pinned_start`] plus the 168 hours the dev server adds, which is the `deadline_at` a
/// receipt asked for under this clock comes back with.
pub fn pinned_deadline() -> String {
    goalcup_protocol::control::format_z(
        pinned_start_secs().saturating_add(goalcup_devserver::ENTRY_WINDOW_SECONDS),
    )
}

fn pinned_start_secs() -> u64 {
    static PINNED: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *PINNED.get_or_init(|| (goalcup_daemon::now_unix() as u64).saturating_sub(3_600))
}

/// The dev signer on a loopback port the OS chose, in this process, with its clock pinned
/// to [`pinned_start`] so `started_at` is a string this process can name and `deadline_at`
/// is that plus 168 hours.
pub async fn devserver() -> (Arc<goalcup_devserver::DevServer>, String) {
    devserver_with_authority(goalcup_devserver::ReceiptAuthority::Legacy).await
}

/// The same real loopback signer under an explicit, independently configured receipt
/// contract.
pub async fn devserver_with_authority(
    authority: goalcup_devserver::ReceiptAuthority,
) -> (Arc<goalcup_devserver::DevServer>, String) {
    let server = Arc::new(
        goalcup_devserver::DevServer::with_authority(authority)
            .expect("the test receipt authority is valid"),
    );
    server.set_clock(&pinned_start());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port");
    let addr = listener.local_addr().expect("the bound address");
    let routed = server.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, goalcup_devserver::dev_router(routed)).await;
    });
    (server, format!("http://{addr}"))
}

/// An empty real daemon and its authenticated control client, wired to a separately
/// configured new-event receipt service. The test
/// creates the entry through `POST /v0/workspaces` rather than constructing an Entry.
pub async fn new_event_control_harness(
    entry_version: &str,
    server_version: &str,
) -> control::EmptyControlHarness {
    let authority = goalcup_devserver::ReceiptAuthority::new_event(server_version)
        .expect("the service questionnaire version is valid");
    let (server, upload_url) = devserver_with_authority(authority).await;
    control::empty_control_harness_on(server, &upload_url, |config| {
        config.receipt_binding = goalcup_daemon::config::ReceiptBindingPolicy::NewEvent {
            questionnaire_version: entry_version.to_string(),
        };
    })
    .await
}

// ---------------------------------------------------------------------------------------
// The config watch.
// ---------------------------------------------------------------------------------------

/// One rendered tracker file under `root`, written to disk, as `render_tracker_files`
/// would have left it.
///
/// It writes as well as describes because every config-watch test starts from a quiet poll,
/// and a `TrackerFile` whose bytes were never on disk would make the very first poll a
/// deviation for a reason no participant caused.
pub fn tracker_file(
    root: &std::path::Path,
    relative_path: &str,
    contents: &[u8],
    restore_invalidates_trust: bool,
) -> goalcup_daemon::workspace::TrackerFile {
    let contents = String::from_utf8(contents.to_vec()).expect("a tracker file is text");
    let absolute_path = root.join(relative_path);
    write(&absolute_path, contents.as_bytes());
    goalcup_daemon::workspace::TrackerFile {
        relative_path: relative_path.to_string(),
        absolute_path,
        scope: goalcup_protocol::manifest::InstallScope::Workspace,
        sha256: goalcup_daemon::workspace::sha256_hex(contents.as_bytes()),
        contents,
        mode: goalcup_daemon::workspace::DEFAULT_MODE,
        restore_invalidates_trust,
        harness_owned: false,
    }
}

/// Write `bytes` at `path`, creating the parent directories the way a participant's editor
/// would have.
pub fn write(path: &std::path::Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the parent directory");
    }
    std::fs::write(path, bytes).expect("the file is written");
}

// ---------------------------------------------------------------------------------------
// A configuration file on disk, for the tests that drive the binary.
// ---------------------------------------------------------------------------------------

/// Write a valid `DaemonConfig` under `root` and return its path.
///
/// Every root is absolute and `workspace_parent` is disjoint from `artifact_root`, because
/// `validate()` refuses both otherwise — an agent enumerates its workspace's parent on its
/// first tool call, so the run's own evidence must not be inside it. The control
/// address is port 0: the OS picks, and the binary prints what it got.
pub fn write_config(root: &std::path::Path) -> std::path::PathBuf {
    write_config_with(root, |text| text)
}

/// [`write_config`] with the last word on the text, so a test can write a config the
/// daemon must refuse without a second copy of the valid one.
pub fn write_config_with(
    root: &std::path::Path,
    edit: impl FnOnce(String) -> String,
) -> std::path::PathBuf {
    let connectors = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors");
    let text = format!(
        "artifact_root = {holder:?}\n\
         connectors_dir = {connectors:?}\n\
         config_dir = {config:?}\n\
         workspace_parent = {workspaces:?}\n\
         control_addr = \"127.0.0.1:0\"\n\
         upload_url = \"http://127.0.0.1:7788\"\n\
         server_public_key = \"{key}\"\n",
        holder = root.join("holder"),
        connectors = connectors,
        config = root.join("config"),
        workspaces = root.join("workspaces"),
        key = goalcup_devserver::dev_server_public_key_hex(),
    );
    let path = root.join("goalcup.toml");
    write(&path, edit(text).as_bytes());
    path
}

pub fn append_line(path: &std::path::Path, line: &str) {
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

/// Everything in `chain.jsonl`, in order.
pub fn read_chain(trace_dir: &std::path::Path) -> Vec<goalcup_protocol::envelope::Envelope> {
    let path = trace_dir.join("chain.jsonl");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("a chain line is an envelope"))
        .collect()
}
