//! The slow tier: real-binary campaigns, the harness contract kit and goldens, and
//! the doctor's preflight. `sh scripts/gate.sh full` runs it before a release.

#[path = "../common/mod.rs"]
mod common;

#[cfg(target_os = "macos")]
mod capture_crash_campaign;
#[cfg(target_os = "macos")]
mod capture_failure_campaign;
#[cfg(unix)]
mod harness_contract_golden;
#[cfg(target_os = "macos")]
mod harness_contract_kit;
#[cfg(target_os = "macos")]
mod m2_hermes_lifecycle;
#[cfg(target_os = "macos")]
mod m2_synthetic_vertical;
mod preflight_doctor;
