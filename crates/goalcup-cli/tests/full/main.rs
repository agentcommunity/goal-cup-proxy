//! The slow tier: real-binary campaigns and end-to-end runs. `sh scripts/gate.sh full` runs it
//! before a release.

#[path = "../common/mod.rs"]
mod common;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod capture_crash_campaign;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod capture_failure_campaign;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod connector_contract;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod keyed_lifecycle;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod launcher_sweep;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod m2_synthetic_vertical;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod run_scoped_daemon;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod upload_e2e;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod upload_worker;
