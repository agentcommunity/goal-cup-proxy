//! The GitHub repository of one real run, asked for by `goalcup start` before the approval (D69).
//!
//! A practice run has none. A real run (a dry run or the cup run) gets a new, empty, public
//! repository of its own: [`choose`] asks its name (or takes `--repo`), finds the participant's
//! GitHub user from their own login (and, with none, prints how to set one up and looks again),
//! opens GitHub's "new repository" page with the name filled in when it does not exist yet, then
//! checks that it is public and empty (GitHub's open API, no login) and that the agent's git sees
//! no branch in it and can push to it (`git ls-remote` and a dry-run push in the agent's own
//! environment). Goal Cup reads, stores and passes
//! no key. Once the run has opened on the server ([`keep_for_next_run`]), the name is kept in
//! `prep.json` only so that the next real run can suggest one, `<name>-2`; a denied approval or a
//! Ctrl-C keeps nothing. A Ctrl-C at any question or check ends it, with nothing created.

use crate::CliError;
use crate::account::github::{self, Found, GithubLogin, PushRefused};
use std::io::Write;
use std::path::{Path, PathBuf};

/// What [`choose`] reads from, or does to, the machine and the network, so that a test can state
/// them.
pub trait Doors {
    /// GitHub's API base, or `None` when it is not asked (a test build without a fake).
    fn api(&self) -> Option<String>;
    /// The participant's GitHub login as the agent will have it ([`github::find_login`]).
    fn login(&self) -> GithubLogin;
    /// Whether `remote` has no branch and the agent's git can push to it ([`github::push_test`]).
    fn push_test(&self, remote: &str) -> Result<(), PushRefused>;
    /// One line typed by the participant, or `None` when there is no terminal, the input ended
    /// or a Ctrl-C came.
    fn read_line(&self) -> Option<String>;
    /// Whether a Ctrl-C came ([`crate::arming::interrupted`]).
    fn interrupted(&self) -> bool;
    /// Open `url` in the browser; whether a program started.
    fn open(&self, url: &str) -> bool;
    /// The participant's git config file the agent does not get, when they have two
    /// ([`crate::launch::unused_git_config`]).
    fn unused_git_config(&self) -> Option<PathBuf>;
}

/// The repository a real run pushes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRepo {
    /// `https://github.com/<owner>/<repo>`, what the server is told.
    pub repo_url: String,
    /// The git remote the clone is made from and pushes to: SSH when the participant's SSH
    /// login to GitHub works, else HTTPS ([`github::remote_url`]).
    pub remote: String,
}

/// What `goalcup start` prints when neither SSH nor `gh` names a GitHub login.
pub const NO_LOGIN_GUIDE: &str = "\
No GitHub login found on this computer. To set one up (about 5 minutes):
  1. No account yet? Make one: https://github.com/signup
  2. Install GitHub's command line tool: https://cli.github.com
  3. Run: gh auth login   (choose GitHub.com, HTTPS, \"Login with a web browser\",
     and say Yes to \"Authenticate Git with your GitHub credentials\")
  Or type <your-user>/<name> as the repo name if your git already pushes to GitHub over HTTPS.
Press Enter when done, or Ctrl-C to stop.";

/// The one fix for a push test that failed.
pub const PUSH_FIX: &str = "Your git login is not one the agent can use. Easiest: run `gh auth \
login` (choose HTTPS and say Yes to Authenticate Git), or use an SSH key: \
https://docs.github.com/en/authentication/connecting-to-github-with-ssh/adding-a-new-ssh-key-to-your-github-account";

/// Said when GitHub's open API would not answer (its limit is 60 calls an hour per address).
const UNCHECKED: &str = "  GitHub's API is busy, so it was not asked. The server checks that the \
repo is public when the run opens, and goalcup that it is empty once it has cloned it.";

/// Ask for, and check, the repository of a new real run. `given` is `--repo`: a name,
/// `<owner>/<name>` or the repository's address. Every refusal says what to do and that nothing
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
    let mut login = doors.login();
    if let Some(refusal) = stop() {
        return Err(refusal);
    }
    if login.user.is_none() {
        // No SSH login and no `gh` login: one short guide, then one more look once the
        // participant says it is done. Goal Cup never makes an account for them.
        say(out, NO_LOGIN_GUIDE)?;
        if read()?.is_some() {
            login = doors.login();
        }
    }
    if let Some(unused) = doors.unused_git_config() {
        say(
            out,
            &format!(
                "  Your agent's git reads ~/.gitconfig; {} is not used.",
                unused.display()
            ),
        )?;
    }
    let mut input = match given {
        Some(given) => given.trim().to_string(),
        None => {
            // A suggestion only when an earlier real run named one: its repository is used.
            let last = crate::prep_file::read_prep_lenient(cfg).repo_url;
            let suggested = suggestion(&last, login.user.as_deref());
            let shown = suggested
                .as_ref()
                .map(|s| format!(" [{s}]"))
                .unwrap_or_default();
            ask(out, &format!("  Name it{shown}: "))?;
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
    if !input.contains('/') && login.user.is_none() {
        // A name alone cannot be placed without a user: asked once more, with the user.
        ask(out, &format!("  Type <your-user>/{input}: "))?;
        if let Some(typed) = read()?.filter(|t| t.contains('/')) {
            input = typed;
        }
    }
    let repo_url = if input.contains('/') {
        github::checked_repo_url(&input)?
    } else {
        if !github::is_repo_name(&input) {
            return Err(refused(format!(
                "{input:?} is not a GitHub repository name: use letters, digits, `.`, `_` and \
                 `-`."
            )));
        }
        let Some(owner) = login.user.clone() else {
            return Err(refused(format!(
                "No GitHub login was found on this computer. Set one up as the steps above say, \
                 or give --repo <your user name>/{input}."
            )));
        };
        github::checked_repo_url(&format!("{owner}/{input}"))?
    };
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
    let remote = github::remote_url(owner, repo, login.ssh);
    let pushed = doors.push_test(&remote);
    if let Some(refusal) = stop() {
        return Err(refusal);
    }
    match pushed {
        Ok(()) => {}
        // GitHub's `size` can read 0 right after the repository was created with a README.
        Err(PushRefused::HasBranches) => {
            return Err(refused(crate::participant::repository_not_empty(
                &format!("{owner}/{repo}"),
                None,
            )));
        }
        Err(PushRefused::Failed(why)) => {
            return Err(refused(format!(
                "The agent's git cannot push to {owner}/{repo} ({why}). {PUSH_FIX}"
            )));
        }
    }
    say(
        out,
        &format!(
            "  {owner}/{repo}: public, empty, and your agent's git can push to it (a dry-run \
             push; nothing was sent)."
        ),
    )?;
    Ok(RunRepo { repo_url, remote })
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

/// The name to suggest after the last real run's repository, `<name>-2`: that one has been used.
/// Shown as `<name>` when it is the participant's own, else `<owner>/<name>`. GitHub is not
/// asked: its open API allows few calls.
fn suggestion(last: &str, user: Option<&str>) -> Option<String> {
    let (owner, name) = github::owner_repo(last).ok()?;
    let name = github::next_name(name);
    Some(if user.is_some_and(|u| u.eq_ignore_ascii_case(owner)) {
        name
    } else {
        format!("{owner}/{name}")
    })
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
