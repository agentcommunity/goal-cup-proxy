//! Fixtures shared by the integration tests in this crate; it is compiled into every test
//! binary that declares `mod support`.
//!
//! What is here directly: a registry with one entry, a loopback upload
//! target and a daemon config file. The bigger harnesses are the submodules:
//! [`daemon`] (the process-level object), [`control`] (its control listener and a client) and
//! [`status`] (a status document and the reader that checks its signature).

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
// under a pre-`main` role dispatch). Unix-only, matching `process_kill.rs`'s own gate.
#[cfg(unix)]
pub mod tree;
// Re-exported so a test says `support::started_entry()`; the
// module path stays available for a caller that prefers `support::status::payload_for`.
#[allow(unused_imports)]
pub use status::*;

use goalcup_daemon::registry::{Entry, ExpectedProcessHandoff, Registry, RunKeys};

/// A test-only server public key: the Ed25519 verifying key of the test seed `[0x42; 32]`.
/// The verifier trusts no key, this one included; it only has to be a well-formed key.
pub const TEST_SERVER_PUBLIC_KEY_HEX: &str =
    "2152f8d19b791d24453242e15f2eab6cb7cffa7b6a5ed30097960e069881db12";

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

/// An upload URL on a loopback port the OS chose, with a listener held open behind it so
/// nothing else can take the port while the test runs. Nothing connects: this tree has no
/// uploader (D48), and `DaemonConfig` still parses the V1 `upload_url`.
pub fn upload_target() -> (std::net::TcpListener, String) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let url = format!(
        "http://{}",
        listener.local_addr().expect("the bound address")
    );
    (listener, url)
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
        key = TEST_SERVER_PUBLIC_KEY_HEX,
    );
    let path = root.join("goalcup.toml");
    write(&path, edit(text).as_bytes());
    path
}

/// Everything in the chain, in order: every part file that is on disk (D55), from
/// `chain.jsonl` or the first part that survived pruning.
pub fn read_chain(trace_dir: &std::path::Path) -> Vec<goalcup_protocol::envelope::Envelope> {
    let parts = goalcup_protocol::export::chain_parts(trace_dir)
        .unwrap_or_else(|e| panic!("{}: {e}", trace_dir.display()));
    assert!(!parts.is_empty(), "{}: no chain part", trace_dir.display());
    parts
        .iter()
        .flat_map(|part| {
            let text = std::fs::read_to_string(&part.path)
                .unwrap_or_else(|e| panic!("{}: {e}", part.path.display()));
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| serde_json::from_str(l).expect("a chain line is an envelope"))
                .collect::<Vec<_>>()
        })
        .collect()
}
