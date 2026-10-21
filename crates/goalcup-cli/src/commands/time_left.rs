//! `goalcup time-left` — how much of the run is left, for the agent to ask (D66).
//!
//! The agent asks; Goal Cup Proxy never tells it anything unasked. The command reads only the
//! file the launch names in [`FILE_ENV`], which the run's watch tick writes from the server's
//! count ([`crate::run::time_left::keep_file`]), so it works from inside the harness, where the
//! configuration directory cannot be found. It asks no server and no daemon.

use crate::CliError;
use crate::run::time_left::{FILE_ENV, PUSH_OFTEN, sentence};
use std::ffi::OsStr;
use std::io::Read;

/// The most of the file read: three short lines.
const MAX_BYTES: u64 = 4096;

/// Print the time left from the file `file` names (the value of [`FILE_ENV`]) at `now` (unix
/// seconds), and answer `Ok`. A file that is missing, unreadable or stale is a refusal (exit 1)
/// that says the time is unknown right now.
pub fn time_left_with(
    file: Option<&OsStr>,
    now: u64,
    out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    let line = file
        .filter(|file| !file.is_empty())
        .and_then(|file| {
            let mut text = String::new();
            std::fs::File::open(file)
                .and_then(|f| f.take(MAX_BYTES).read_to_string(&mut text))
                .ok()?;
            sentence(&text, now)
        })
        .ok_or_else(|| {
            CliError::Refused(format!(
                "the time left is unknown right now (no fresh ${FILE_ENV} file). {PUSH_OFTEN}"
            ))
        })?;
    writeln!(out, "{line}").map_err(|source| CliError::Io {
        path: "stdout".into(),
        source,
    })
}
