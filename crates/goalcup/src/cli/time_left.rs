//! `goalcup time-left` (PRD-10 section 10, D66): the time left of the run, from the server's last
//! word in `upload.json` (seam S9). It asks no server: a word older than 11 minutes, or none, falls
//! back to the run's deadline; with no deadline either it is "unknown right now" (exit 1).

use std::path::Path;
use std::process::ExitCode;

use super::{RunMode, ServerJson, fail, find, home, read_json, text, unix_of};
use crate::upload::state::{self as up, UploadJson};

/// Set in the harness's environment by `start`: the part's `upload.json`.
pub const FILE_ENV: &str = "GOALCUP_TIME_LEFT_FILE";

/// Two 5-minute heartbeats and a minute.
const FRESH_SECS: u64 = 660;

pub fn run() -> ExitCode {
    let now = super::now();
    let answer = match super::var(FILE_ENV) {
        Some(file) => in_harness(Path::new(&file), now),
        None => in_terminal(now),
    };
    match answer {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(line) => fail(&line),
    }
}

/// Inside the harness, whose `HOME` is the isolated home: only the file named and its run's
/// `server.json` are read.
fn in_harness(file: &Path, now: u64) -> Result<String, String> {
    let run_dir = file.parent().and_then(Path::parent);
    let run = run_dir.and_then(|r| read_json::<ServerJson>(&super::server_json(r)));
    let upload = read_json::<UploadJson>(file);
    let deadline = run
        .as_ref()
        .and_then(|r| r.deadline.as_deref().and_then(unix_of));
    line(upload.as_ref(), practice(run.as_ref()), deadline, now)
}

/// In the person's own terminal: the newest open run's newest part. A run is open when it has a
/// server and its deadline, if any, has not passed.
fn in_terminal(now: u64) -> Result<String, String> {
    let no_run = || text::NO_OPEN_RUN_TIME_LEFT.to_owned();
    let home = home().map_err(str::to_owned)?;
    let open = |r: ServerJson| {
        let deadline = r.deadline.as_deref().and_then(unix_of);
        r.server.is_some() && deadline.is_none_or(|d| d > now)
    };
    let mut runs = super::runs(&home).into_iter().rev();
    let dir = runs.find(|d| read_json(&super::server_json(d)).is_some_and(open));
    let name = dir
        .as_deref()
        .and_then(Path::file_name)
        .and_then(|n| n.to_str());
    let found = name.and_then(|n| find(&home, Some(n))).ok_or_else(no_run)?;
    let practice = practice(found.server.as_ref());
    let deadline = found
        .server
        .as_ref()
        .and_then(|r| r.deadline.as_deref().and_then(unix_of));
    line(
        up::read(&found.entry_dir()).as_ref(),
        practice,
        deadline,
        now,
    )
}

fn practice(run: Option<&ServerJson>) -> bool {
    run.is_some_and(|r| r.mode == RunMode::Practice)
}

/// The line at `now` (seconds since 1970). The server's word counts down from its `asked_at` when
/// it was asked within 11 minutes of now (`stop` is "time is up"); without such a word the run's
/// own `deadline` answers; with neither, the "unknown right now" line.
pub fn line(
    upload: Option<&UploadJson>,
    practice: bool,
    deadline: Option<u64>,
    now: u64,
) -> Result<String, String> {
    let word = upload.and_then(|u| u.server_word.as_ref());
    let fresh = word.and_then(|w| {
        Some((
            w,
            unix_of(&w.asked_at).filter(|a| a.abs_diff(now) <= FRESH_SECS)?,
        ))
    });
    if let Some((word, asked)) = fresh {
        if word.stop {
            return Ok(text::TIME_IS_UP.to_owned());
        }
        if let Some(left) = word.seconds_left {
            return Ok(text::time_left(asked.saturating_add(left), now, practice));
        }
    }
    match deadline {
        Some(d) if d <= now => Ok(text::TIME_IS_UP.to_owned()),
        Some(d) => Ok(text::time_left(d, now, practice)),
        None => Err(text::time_left_unknown(practice)),
    }
}
