//! The fast tier: every integration test of this crate that is neither slow nor
//! mutates process-global state. One binary, one module per former `tests/<name>.rs`.

#[path = "../common/mod.rs"]
mod common;

mod amp_child_secret;
#[cfg(unix)]
mod capture_gate;
mod capture_only_refusal;
#[cfg(unix)]
mod capture_stderr_tee;
#[cfg(unix)]
mod credential_stdout_identity;
mod daemon_client;
mod dependency_bans;
mod harness_kit_credential;
mod harness_launch_plan;
mod hermes_child_secret;
#[cfg(unix)]
mod hermes_offline_key;
#[cfg(unix)]
mod hermes_per_run_home;
mod key_scan;
mod key_scan_run;
#[cfg(unix)]
mod loader_script;
mod login;
mod m2_capture_sender;
mod m2_emitter;
mod m2_http_relay;
mod m2_ipc_compose;
mod m2_proxy_integration;
mod m2_tls_connect;
mod m2_websocket_relay;
mod process_handoff;
mod seal_scan;
mod start_launch;
mod status_views;
