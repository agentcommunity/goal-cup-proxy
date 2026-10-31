//! `goalcup upload [PART]` (PRD-10 section 1): send what the server does not hold yet, through
//! `upload::manual::run` (PRD-06 section 9). A part named is sent alone; a run named, or none (the
//! newest run), sends every part of the run in order. The words are `upload.json`'s own lines
//! (`text::upload_line`) plus the few cases that file cannot say. The run credential is not read here.

use std::process::ExitCode;

use super::{fail, find, home, run_dir, text};
use crate::upload::manual::{Outcome, run as send};
use crate::upload::state;

/// 0 when every part is on the server (or sends itself, or this run has no server), 1 otherwise.
pub fn run(part: Option<String>) -> ExitCode {
    let home = match home() {
        Ok(h) => h,
        Err(t) => return fail(t),
    };
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
        let keep = super::var("GOALCUP_KEEP_PARTS").is_some_and(|v| v == "1");
        let out = send(&dir, &found.run_dir, (&found.run, &entry), keep);
        // `upload.json` words the outcomes that change it; the rest it cannot say.
        let said = !matches!(
            out,
            Outcome::StillRecording | Outcome::NotClosed | Outcome::Local | Outcome::NoServer
        );
        let line = match state::read(&dir) {
            Some(u) if said => text::upload_line(&u, super::now(), true),
            _ => text::upload_outcome(&out, &entry, &found.run),
        };
        println!("{line}");
        if !matches!(
            out,
            Outcome::Done | Outcome::AlreadyThere | Outcome::StillRecording | Outcome::Local
        ) {
            worst = ExitCode::from(1);
        }
    }
    worst
}
