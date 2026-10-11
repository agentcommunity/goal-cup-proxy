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
use goalcup_protocol::manifest::KillStrategy;
use goalcup_protocol::process::ProcessHandle;
use std::path::{Path, PathBuf};

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
