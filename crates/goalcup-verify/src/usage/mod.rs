//! The usage reader (PRD-09). A placeholder until PRD-09's first merge replaces this file; the
//! signature is the seam: the arguments after the word `usage`, and the process exit code.

use std::ffi::OsString;
use std::process::ExitCode;

/// Runs `goalcup-verify usage <args>`. Not built yet: says so and exits 3 (no usable report).
pub fn run(args: Vec<OsString>) -> ExitCode {
    let _ = args;
    eprintln!("goalcup-verify: usage is not built yet");
    ExitCode::from(3)
}
