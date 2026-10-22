//! `goalcup time-left`: how much of the run is left, for the agent to ask (D66), and for the
//! participant in their own terminal.
//!
//! The agent asks; Goal Cup Proxy never tells it anything unasked. Inside the harness the
//! command reads only the file the launch names in [`FILE_ENV`], which the run's watch tick
//! writes from the server's count ([`crate::run::time_left::keep_file`]), so it works there,
//! where the configuration directory cannot be found. It asks no server and no daemon.
//!
//! In a person's own terminal there is no such file: the newest open run's state on this
//! computer is read instead, the way `goalcup status` counts its time left
//! ([`time_left_of_open_run`]).
//!
//! [`answer`] makes that choice and finds the line. `goalcup time-left` prints it, and the
//! `time_left` tool of `goalcup mcp` (fix plan v9) returns it, so the two cannot differ.

use crate::CliError;
use crate::run::time_left::{file_says_practice, left_line, sentence};
use std::io::Read;
use std::path::Path;

/// The most of the file read: four short lines.
const MAX_BYTES: u64 = 4096;

/// The time-left line at `now` (unix seconds), or its refusal: from the file the environment
/// names ([`crate::run::time_left::file_from_env`]), as inside the harness; without one, from the
/// newest open run in the configuration directory, as in a person's own terminal. It only reads.
pub fn answer(now: u64) -> Result<String, CliError> {
    match crate::run::time_left::file_from_env() {
        Some(file) => line_from_file(&file, now),
        None => line_of_open_run(&crate::config::config_dir()?, now),
    }
}

/// `goalcup time-left`: print [`answer`].
pub fn time_left(now: u64, out: &mut dyn std::io::Write) -> Result<(), CliError> {
    put(out, &answer(now)?)
}

/// Print the time left from `file` (the one [`crate::run::time_left::file_from_env`] names) at
/// `now` (unix seconds), and answer `Ok`. A file that is missing, unreadable or stale is a
/// refusal (exit 1) that says the time is unknown right now.
pub fn time_left_with(file: &Path, now: u64, out: &mut dyn std::io::Write) -> Result<(), CliError> {
    put(out, &line_from_file(file, now)?)
}

/// The line [`time_left_with`] prints, or its refusal.
fn line_from_file(file: &Path, now: u64) -> Result<String, CliError> {
    let mut text = String::new();
    std::fs::File::open(file)
        .and_then(|f| f.take(MAX_BYTES).read_to_string(&mut text))
        .ok()
        .and_then(|_| sentence(&text, now))
        .ok_or_else(|| {
            CliError::Refused(crate::participant::time_left_unknown(file_says_practice(
                &text,
            )))
        })
}

/// Print the time left of the newest run on this computer that is still open (on a server, and
/// not over), counted as `goalcup status` counts it, in the line [`time_left_with`] prints. No
/// open run is a refusal (exit 1).
pub fn time_left_of_open_run(
    cfg: &Path,
    now: u64,
    out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    put(out, &line_of_open_run(cfg, now)?)
}

/// The line [`time_left_of_open_run`] prints, or its refusal.
fn line_of_open_run(cfg: &Path, now: u64) -> Result<String, CliError> {
    let mut runs = crate::run::all(cfg);
    runs.sort_by_key(|run| std::cmp::Reverse(run.started_at_unix));
    runs.into_iter()
        .filter(|run| run.server_url.is_some())
        .find_map(|run| {
            let state = crate::run::read_state(&crate::run::run_dir(cfg, &run.run_id));
            if crate::run::is_over(&run, state.as_ref(), now) {
                return None;
            }
            let left = crate::participant::run_seconds_left(&run, state.as_ref(), now)?;
            let deadline = run
                .deadline_at
                .as_deref()
                .and_then(goalcup_protocol::control::parse_z);
            Some(left_line(left, deadline, run.practice))
        })
        .ok_or_else(|| CliError::Refused(crate::participant::no_open_run_time_left().to_string()))
}

fn put(out: &mut dyn std::io::Write, line: &str) -> Result<(), CliError> {
    writeln!(out, "{line}").map_err(|source| CliError::Io {
        path: "stdout".into(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In a person's own terminal (no time-left file) the newest open run's state answers, in
    /// the line the agent gets; with no open run it says so and exits 1.
    #[test]
    fn without_the_file_the_newest_open_run_says_its_time_left() {
        let cfg = tempfile::tempdir().unwrap();
        let none = time_left_of_open_run(cfg.path(), 1_000, &mut Vec::new()).unwrap_err();
        assert_eq!(none.exit_code(), 1);
        assert!(none.to_string().contains("No run is open"), "{none}");

        let now = crate::run::now_unix();
        let deadline = now + 2 * 3600 + 30 * 60 + 30;
        crate::run::tests_support::continuable_run(cfg.path(), "run-open", "part-1", deadline);
        // The state holds no count: the server's deadline against this machine's date.
        let mut out = Vec::new();
        time_left_of_open_run(cfg.path(), now, &mut out).unwrap();
        let said = String::from_utf8(out).unwrap();
        assert!(said.starts_with("About 2 h 30 min left"), "{said}");
        assert!(said.contains(crate::run::time_left::PUSH_OFTEN), "{said}");

        // A run that is over is not the one asked about.
        let over = tempfile::tempdir().unwrap();
        crate::run::tests_support::continuable_run(over.path(), "run-over", "part-2", now - 1);
        assert!(time_left_of_open_run(over.path(), now, &mut Vec::new()).is_err());
    }
}
