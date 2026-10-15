//! The daemon's own configuration for a run rooted in a temporary directory, and a stand-in
//! for the harness process the CLI launched, with the process handle that names it.

use goalcup_daemon::config::DaemonConfig;
use goalcup_protocol::manifest::KillStrategy;
use goalcup_protocol::process::ProcessHandle;
use std::path::{Path, PathBuf};

/// The daemon's own configuration for a run rooted at `root`.
///
/// `workspace_parent` and `artifact_root` are disjoint trees, which `DaemonConfig::validate`
/// enforces and `Daemon::start` therefore does too: an agent enumerates its workspace's
/// parent directory on its first tool call.
pub fn config_at(root: &Path) -> DaemonConfig {
    DaemonConfig {
        artifact_root: root.join("holder"),
        connectors_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors"),
        config_dir: root.join("config"),
        workspace_parent: root.join("workspaces"),
        control_addr: "127.0.0.1:0".parse().expect("a loopback address"),
        // No round configured: the signed status carries the entry's own timeline and
        // says so, through `event_status_source: "fallback"`.
        event: None,
    }
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
    }
}
