//! The GitHub repository of one real run, asked for by `goalcup start` before the approval (D69).
//!
//! A practice run has none. A real run (a dry run or the cup run) gets a new, empty, public
//! repository of its own: [`choose`] asks its name as `<owner>/<name>` (or takes `--repo`), opens
//! GitHub's "new repository" page with the name filled in when it does not exist yet, then checks
//! that it is public and empty (GitHub's open API, no login). It does not look for the
//! participant's GitHub login and makes no test push: the agent pushes with the token the
//! participant put in the project folder's `.env.local`. Goal Cup reads, stores and passes no
//! key. Once the run has opened on the server ([`keep_for_next_run`]), the name is kept in
//! `prep.json` only so that the next real run can suggest one, `<name>-2`; a denied approval or a
//! Ctrl-C keeps nothing. A Ctrl-C at any question or check ends it, with nothing created.

use crate::CliError;
use crate::account::github::{self, Found};
use std::io::Write;
use std::path::Path;

/// What [`choose`] reads from, or does to, the machine and the network, so that a test can state
/// them.
pub trait Doors {
    /// GitHub's API base, or `None` when it is not asked (a test build without a fake).
    fn api(&self) -> Option<String>;
    /// One line typed by the participant, or `None` when there is no terminal, the input ended
    /// or a Ctrl-C came.
    fn read_line(&self) -> Option<String>;
    /// Whether a Ctrl-C came ([`crate::arming::interrupted`]).
    fn interrupted(&self) -> bool;
    /// Open `url` in the browser; whether a program started.
    fn open(&self, url: &str) -> bool;
}

/// The repository a real run pushes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRepo {
    /// `https://github.com/<owner>/<repo>`, what the server is told.
    pub repo_url: String,
}

/// Said when GitHub's open API would not answer (its limit is 60 calls an hour per address).
const UNCHECKED: &str = "  GitHub's API is busy, so it was not asked. The server checks that the \
repo is public when the run opens.";

/// Ask for, and check, the repository of a new real run. `given` is `--repo`: `<owner>/<name>`
/// or the repository's address. Every refusal says what to do and that nothing
/// was created.
pub fn choose(
    doors: &dyn Doors,
    cfg: &Path,
    given: Option<&str>,
    out: &mut dyn Write,
) -> Result<RunRepo, CliError> {
    let refused = |why: String| CliError::Refused(format!("{why} Nothing was created."));
    // A Ctrl-C at a question or during a check: the same refusal as any Ctrl-C before the run.
    let stop = || {
        doors
            .interrupted()
            .then(crate::commands::start::interrupted_refusal)
    };
    let read = || -> Result<Option<String>, CliError> {
        let line = doors.read_line().map(|l| l.trim().to_string());
        match stop() {
            Some(refusal) => Err(refusal),
            None => Ok(line),
        }
    };
    say(
        out,
        "Your run needs a new, empty, public GitHub repo. Each real run gets its own.",
    )?;
    // No look for the participant's GitHub login and no test push: how the agent pushes is the
    // participant's to set up (their key in `.env.local`, their own git login). Only the name is
    // asked here, always with its owner.
    let input = match given {
        Some(given) => given.trim().to_string(),
        None => {
            // A suggestion only when an earlier real run named one: its repository is used.
            let last = crate::prep_file::read_prep_lenient(cfg).repo_url;
            let suggested = suggestion(&last);
            let shown = suggested
                .as_ref()
                .map(|s| format!(" [{s}]"))
                .unwrap_or_default();
            ask(out, &format!("  Name it, as <your-user>/<name>{shown}: "))?;
            match read()? {
                Some(typed) if !typed.is_empty() => typed,
                Some(_) if suggested.is_some() => suggested.unwrap_or_default(),
                _ => {
                    return Err(refused(
                        "A real run needs a new, empty, public GitHub repository. Run goalcup \
                         start in a terminal and name it, or add --repo <owner>/<name>."
                            .to_string(),
                    ));
                }
            }
        }
    };
    if !input.contains('/') {
        return Err(refused(format!(
            "Give the repository with its owner: <your-user>/{input}, or add --repo \
             <owner>/<name>."
        )));
    }
    let repo_url = github::checked_repo_url(&input)?;
    let (owner, repo) = github::owner_repo(&repo_url)?;
    if let Some(api) = doors.api() {
        let checked = |api: &str| {
            github::check_public_empty(api, &repo_url).map_err(|e| refused(e.to_string()))
        };
        let mut found = checked(&api)?;
        if found == Found::Missing {
            let page = github::new_repo_page(owner, repo);
            let opened = doors.open(&page);
            say(
                out,
                &format!(
                    "  {} github.com/new with that name: {page}\n  Click \"Create repository\" \
                     (keep it public, add no README), then press Enter.",
                    if opened { "Opening" } else { "Open" }
                ),
            )?;
            found = match read()? {
                Some(_) => checked(&api)?,
                None => Found::Missing,
            };
            if found == Found::Missing {
                return Err(refused(format!(
                    "GitHub does not show {owner}/{repo}: it is not created yet, or it is \
                     private. Create it as a public repository at {page}, then run the command \
                     again."
                )));
            }
        }
        if found == Found::Unchecked {
            say(out, UNCHECKED)?;
        }
    }
    say(out, &format!("  {owner}/{repo}: public and empty."))?;
    Ok(RunRepo { repo_url })
}

/// Keep `repo_url` in `prep.json` as the last real run's repository, for [`suggestion`]: called
/// once a run has opened with it, never before, so a start that ends at the approval suggests
/// the same name again.
pub fn keep_for_next_run(cfg: &Path, repo_url: &str) -> Result<(), CliError> {
    let mut file = crate::prep_file::read_prep_lenient(cfg);
    if file.repo_url == repo_url {
        return Ok(());
    }
    file.repo_url = repo_url.to_string();
    crate::prep_file::write_prep(cfg, &file)
}

/// The name to suggest after the last real run's repository, `<owner>/<name>-2`: that one has
/// been used. GitHub is not asked: its open API allows few calls.
fn suggestion(last: &str) -> Option<String> {
    let (owner, name) = github::owner_repo(last).ok()?;
    Some(format!("{owner}/{}", github::next_name(name)))
}

fn say(out: &mut dyn Write, line: &str) -> Result<(), CliError> {
    super::say(out, line)
}

/// A question, with no line break after it, flushed so it is on the screen before the answer.
fn ask(out: &mut dyn Write, question: &str) -> Result<(), CliError> {
    out.write_all(question.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|source| CliError::Io {
            path: "stdout".to_string(),
            source,
        })
}
