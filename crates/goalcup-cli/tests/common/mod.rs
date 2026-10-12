//! Support shared by this crate's `it` and `full` test binaries. The standalone binaries
//! (`tests/<name>.rs`) include the modules they need from here by `#[path]`.
//!
//! Each binary uses a different subset, so unused items and re-exports are expected.
#![allow(dead_code, unused_imports)]

#[cfg(target_os = "macos")]
pub mod capture_campaign_support;
#[cfg(target_os = "macos")]
pub mod capture_crash_support;
#[cfg(target_os = "macos")]
pub mod capture_failure_support;
#[cfg(target_os = "macos")]
pub mod m2_cli_support;
pub mod m2_common;
pub mod m2_http_support;
pub mod m2_profile_support;
pub mod m2_proxy_support;
pub mod m2_sender_support;
pub mod m2_tls_support;
#[cfg(target_os = "macos")]
pub mod m2_vertical_support;
pub mod m2_ws_support;
pub mod secret_support;
pub mod support;

// Shared with goalcup-daemon's process-table tests; one mutex per test binary.
#[path = "../../../goalcup-daemon/tests/support/table_lock.rs"]
pub mod table_lock;
