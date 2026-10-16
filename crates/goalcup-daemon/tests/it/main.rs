//! The fast tier of goalcup-daemon's integration tests, folded into one binary.
//!
//! Slow files go in `tests/full/`. `dir_modes` stays a standalone binary because it changes
//! process state (umask); `health_status` is a standalone binary too.

// Each test binary uses a different subset of the shared support.
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;

mod binary_surface;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod capture_deferred_scope;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod capture_harness_load;
mod capture_ingest;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod capture_lifecycle;
mod capture_native;
mod capture_redaction;
mod capture_redaction_extra;
mod capture_redaction_run_secrets;
mod config_load;
mod control_api;
mod harness_contract_golden;
mod redaction;
mod registry_roundtrip;
#[cfg(unix)]
mod start_race;
#[cfg(unix)]
mod table_lock_group;
#[cfg(unix)]
mod terminate_sim;
mod workspace_layout;
