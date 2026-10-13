//! The slow tier: real-binary campaigns, the harness contract kit and goldens, and
//! the doctor's preflight. `sh scripts/gate.sh full` runs it before a release.

#[path = "../common/mod.rs"]
mod common;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod capture_crash_campaign;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod capture_failure_campaign;
#[cfg(unix)]
mod harness_contract_golden;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod harness_contract_kit;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod launcher_sweep;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod m2_hermes_lifecycle;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod m2_synthetic_vertical;
mod preflight_doctor;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod run_scoped_daemon;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod upload_e2e;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod upload_worker;
