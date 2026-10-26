//! The eleven sub-commands, one module each.
//!
//! **Eleven, not twelve.** `verify` is `goalcup-verify`, a separate binary
//! (`crates/goalcup-verifier/src/bin/goalcup-verify.rs`), and `goalcup --help` says so in one
//! line pointing at it. A `goalcup verify` that shelled out to that binary would be a second
//! place the verifier's flags and exit codes are spelled, and the first one to drift.

pub mod continue_run;
pub mod doctor;
pub mod entries;
pub mod login;
pub mod mcp;
pub mod prep;
pub mod run_repo;
pub mod seal;
pub mod start;
pub mod status;
pub mod time_left;
pub mod upload;

use crate::CliError;

/// One line to the participant, or the write error naming the stream.
pub(crate) fn say(out: &mut dyn std::io::Write, line: &str) -> Result<(), CliError> {
    writeln!(out, "{line}").map_err(|source| CliError::Io {
        path: "stdout".to_string(),
        source,
    })
}
