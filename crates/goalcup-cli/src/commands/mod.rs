//! The eight sub-commands, one module each.
//!
//! **Eight, not nine.** `verify` is `goalcup-verify`, a separate binary
//! (`crates/goalcup-verifier/src/bin/goalcup-verify.rs`), and `goalcup --help` says so in one
//! line pointing at it. A `goalcup verify` that shelled out to that binary would be a second
//! place the verifier's flags and exit codes are spelled, and the first one to drift.

#[cfg(feature = "v1-compat")]
pub mod auth;
pub mod doctor;
pub mod entries;
#[cfg(feature = "v1-compat")]
pub mod resume;
pub mod seal;
pub mod start;
pub mod status;
#[cfg(feature = "v1-compat")]
pub mod ui;
