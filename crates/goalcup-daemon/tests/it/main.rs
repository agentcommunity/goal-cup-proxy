//! The fast tier of goalcup-daemon's integration tests, folded into one binary.
//!
//! Slow files go in `tests/full/`. `dir_modes` and `health_status` stay standalone binaries
//! because they change process state (umask, a re-exec by exact test name).

#[path = "../common/mod.rs"]
mod common;

mod binary_surface;
#[cfg(target_os = "macos")]
mod capture_deferred_scope;
#[cfg(target_os = "macos")]
mod capture_harness_load;
mod capture_ingest;
#[cfg(target_os = "macos")]
mod capture_lifecycle;
mod capture_native;
mod capture_redaction;
mod capture_redaction_extra;
mod config_load;
mod contract_present;
mod control_api;
mod placeholder_notice;
mod redaction;
mod registry_roundtrip;
#[cfg(unix)]
mod terminate_sim;
mod workspace_layout;
