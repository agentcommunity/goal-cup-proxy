//! Fixtures shared by the integration tests in this crate; it is compiled into every test
//! binary that declares `mod support`.
//!
//! What is here directly: a registry with one entry, the shipped manifests, a devserver on a
//! loopback socket and a daemon config file. The bigger harnesses are the submodules:
//! [`daemon`] (the process-level object), [`control`] (its control listener and a client) and
//! [`status`] (a status document and the reader that checks its signature).

#![allow(dead_code)]

// Health, status and page fixtures.
pub mod status;
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
// rather than through this module, so its binary does not also link the devserver
// fixture's axum and tokio for a file that spawns `sh` and `sleep`.
#[cfg(unix)]
pub mod tree;
// Re-exported so a test says `support::started_entry()`; the
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

// ---------------------------------------------------------------------------------------
// The fixture dev server.
// ---------------------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------------------
// Files on disk.
// ---------------------------------------------------------------------------------------

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

/// Write a valid `DaemonConfig` under `root`, with `edit` given the last word on the text,
/// so a test can write a config the daemon must refuse without a second copy of the valid
/// one. The control address is port 0: the OS picks, and the binary prints what it got.
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

/// Everything in `chain.jsonl`, in order.
pub fn read_chain(trace_dir: &std::path::Path) -> Vec<goalcup_protocol::envelope::Envelope> {
    let path = trace_dir.join("chain.jsonl");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("a chain line is an envelope"))
        .collect()
}
