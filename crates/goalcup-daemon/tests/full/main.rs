//! The slow tier of goalcup-daemon's integration tests, folded into one binary: run before a
//! release, not on every change. `cargo test -p goalcup-daemon --test full`.

// Each test binary uses a different subset of the shared support.
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;

#[cfg(target_os = "macos")]
mod capture_finish_budget;
mod harness_contract_golden;
#[cfg(unix)]
mod process_kill;
