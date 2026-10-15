//! Support shared by this crate's `it` and `full` test binaries. The standalone binaries
//! (`tests/<name>.rs`) include the modules they need from here by `#[path]`.
//!
//! Each binary uses a different subset, so an item one binary leaves unused is expected, and
//! this allow is the one place that says so for every module below. The four modules a
//! standalone binary also includes by `#[path]` (`support`, `m2_common`, `m2_cli_support`,
//! `m2_profile_support`) repeat it for that binary. A helper that no binary uses is deleted.
#![allow(dead_code, unused_imports)]

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod capture_campaign_support;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod capture_crash_support;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod capture_failure_support;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod m2_cli_support;
pub mod m2_common;
pub mod m2_http_support;
pub mod m2_profile_support;
pub mod m2_proxy_support;
pub mod m2_sender_support;
pub mod m2_tls_support;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod m2_vertical_support;
pub mod m2_ws_support;
#[cfg(unix)]
pub mod real_home_guard;
pub mod secret_support;
pub mod support;
pub mod upload_server;

// Shared with goalcup-daemon's process-table tests; one mutex per test binary.
#[path = "../../../goalcup-daemon/tests/common/support/table_lock.rs"]
pub mod table_lock;
