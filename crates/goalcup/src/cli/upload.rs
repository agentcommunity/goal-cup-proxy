//! `goalcup upload [PART]` (PRD-10 section 1): send what the server does not hold yet, through
//! `upload::manual::run` (PRD-06 section 9). A part named is sent alone; a run named, or none (the
//! newest run), sends every part of the run in order. The words are `upload.json`'s own lines
//! (`text::upload_line`) plus the few cases that file cannot say. The run credential is not read here.

use std::process::ExitCode;

use super::{Server, fail, find, home, run_dir, say, server, text};
use crate::upload::manual::{Outcome, run as send};
use crate::upload::state;

/// 0 when every part is on the server (or sends itself, or this run has no server), 1 otherwise.
pub fn run(part: Option<String>) -> ExitCode {
    let home = match home() {
        Ok(h) => h,
        Err(t) => return fail(t),
    };
    match server() {
        Ok(Server::Off) => {
            say(text::SERVER_OFF);
            return ExitCode::SUCCESS;
        }
        Ok(Server::Url(_)) => {}
        Err(t) => return fail(t),
    }
    let Some(found) = find(&home, part.as_deref()) else {
        let line = part.map_or_else(|| text::NO_RUN_YET.to_owned(), |p| text::no_such_part(&p));
        return fail(&line);
    };
    let alone = part.as_deref().is_some_and(|p| !p.starts_with('r'));
    let parts = if alone {
        vec![found.entry.clone()]
    } else {
        found.entries.clone()
    };
    let mut worst = ExitCode::SUCCESS;
    for entry in parts {
        let dir = run_dir(&home, &found.run).join(&entry);
        let out = send(&dir, &found.run_dir, (&found.run, &entry), false);
        let upload = state::read(&dir);
        let line = match (&out, upload) {
            (
                Outcome::Done | Outcome::AlreadyThere | Outcome::Refused(_) | Outcome::Stuck,
                Some(u),
            )
            | (Outcome::Closed { .. }, Some(u)) => text::upload_line(&u, super::now(), true),
            _ => message(&out, &entry),
        };
        say(&line);
        if !matches!(
            out,
            Outcome::Done | Outcome::AlreadyThere | Outcome::StillRecording | Outcome::Local
        ) {
            worst = ExitCode::from(1);
        }
    }
    worst
}

/// The cases `upload.json` cannot word.
fn message(out: &Outcome, entry: &str) -> String {
    match out {
        Outcome::Done | Outcome::AlreadyThere => {
            "Upload: done. The server holds this part.".to_owned()
        }
        Outcome::StillRecording => {
            format!("Part {entry} is still recording; it sends itself, so there is nothing to do.")
        }
        Outcome::NotClosed => format!(
            "Part {entry} has no end yet, so it is not sent. Run goalcup seal first; nothing is lost."
        ),
        Outcome::Local => "Upload: none. This run stays on this computer.".to_owned(),
        Outcome::NoServer => format!(
            "This run's server file or credential is missing, so nothing can be sent. Your record is \
             kept on this computer. Write to {}.",
            text::SUPPORT_ADDRESS
        ),
        Outcome::Refused(s) => format!(
            "Upload: the server refuses this run ({s}). Nothing more is sent. Write to {}.",
            text::SUPPORT_ADDRESS
        ),
        Outcome::Closed { upload_by } => format!(
            "Upload: the server closed this run (it took uploads until {upload_by}). What was not sent stays on this computer."
        ),
        Outcome::Stuck => format!(
            "Upload: stopped. The server's copy and this computer's copy differ. Nothing more is sent. Write to {}.",
            text::SUPPORT_ADDRESS
        ),
    }
}
