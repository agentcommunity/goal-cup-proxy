//! The GitHub repository of one real run, asked for by `goalcup start` before the approval (D69).
//!
//! A practice run has none. A real run (a dry run or the cup run) gets a new, empty, public
//! repository of its own: [`choose`] asks its name (or takes `--repo`), finds the participant's
//! GitHub user from their own login (and, with none, prints how to set one up and looks again),
//! opens GitHub's "new repository" page with the name filled
//! in when it does not exist yet, then checks that it is public and empty (GitHub's open API, no
//! login) and that the participant's own git can push to it (a dry-run push). Goal Cup reads,
//! stores and passes no key. The name is kept in `prep.json` only so that the next real run can
//! suggest one: `<name>-2` once this one is used.

use crate::CliError;
use crate::account::github::{self, Found, GithubLogin};
use std::io::Write;
use std::path::Path;

/// What [`choose`] reads from, or does to, the machine and the network, so that a test can state
/// them.
pub trait Doors {
    /// GitHub's API base, or `None` when it is not asked (a test build without a fake).
    fn api(&self) -> Option<String>;
    /// The participant's GitHub login, found with their own tools ([`github::find_login`]).
    fn login(&self) -> GithubLogin;
    /// A dry-run push to `remote` with the participant's own git ([`github::push_test`]).
    fn push_test(&self, remote: &str) -> Result<(), String>;
    /// One line typed by the participant, or `None` when there is no terminal or the input
    /// ended.
    fn read_line(&self) -> Option<String>;
    /// Open `url` in the browser; whether a program started.
    fn open(&self, url: &str) -> bool;
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
Press Enter when done, or Ctrl-C to stop.";

/// Help for a push that failed: one fix, one link.
const SSH_KEY_HELP: &str = "https://docs.github.com/en/authentication/connecting-to-github-with-ssh/adding-a-new-ssh-key-to-your-github-account";

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
    say(
        out,
        "Your run needs a new, empty, public GitHub repo. Each real run gets its own.",
    )?;
    let mut login = doors.login();
    if login.user.is_none() {
        // No SSH login and no `gh` login: one short guide, then one more look once the
        // participant says it is done. Goal Cup never makes an account for them.
        say(out, NO_LOGIN_GUIDE)?;
        if doors.read_line().is_some() {
            login = doors.login();
        }
    }
    let input = match given {
        Some(given) => given.trim().to_string(),
        None => {
            let last = crate::prep_file::read_prep_lenient(cfg).repo_url;
            let suggested = suggestion(doors, &last, login.user.as_deref());
            let shown = suggested
                .as_ref()
                .map(|s| format!(" [{s}]"))
                .unwrap_or_default();
            ask(out, &format!("  Name it{shown}: "))?;
            match doors.read_line().map(|l| l.trim().to_string()) {
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
            return Err(refused(
                "No GitHub login was found on this computer. Set one up as the steps above say \
                 and run the command again, or give --repo <your user name>/<repo name>."
                    .to_string(),
            ));
        };
        github::checked_repo_url(&format!("{owner}/{input}"))?
    };
    let (owner, repo) = github::owner_repo(&repo_url)?;
    if let Some(api) = doors.api() {
        let checked = |api: &str| {
            github::check_public_empty(api, &repo_url).map_err(|e| refused(e.to_string()))
        };
        if checked(&api)? == Found::Missing {
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
            let created = doors.read_line().is_some() && checked(&api)? == Found::PublicEmpty;
            if !created {
                return Err(refused(format!(
                    "GitHub does not show {owner}/{repo}: it is not created yet, or it is \
                     private. Create it as a public repository at {page}, then run the command \
                     again."
                )));
            }
        }
    }
    let remote = github::remote_url(owner, repo, login.ssh);
    if let Err(why) = doors.push_test(&remote) {
        let fix = if login.ssh {
            format!(
                "Check that your GitHub user can push to {owner}/{repo}. If your network \
                 blocks SSH, see https://docs.github.com/en/authentication/troubleshooting-ssh/using-ssh-over-the-https-port"
            )
        } else {
            format!(
                "Add an SSH key to GitHub ({SSH_KEY_HELP}), or run `gh auth login`, then run \
                 the command again"
            )
        };
        return Err(refused(format!(
            "Your git cannot push to {owner}/{repo} ({why}). {fix}."
        )));
    }
    let mut file = crate::prep_file::read_prep_lenient(cfg);
    if file.repo_url != repo_url {
        file.repo_url = repo_url.clone();
        crate::prep_file::write_prep(cfg, &file)?;
    }
    say(
        out,
        &format!(
            "  {owner}/{repo}: public, empty, and your git can push to it (a dry-run push; \
             nothing was sent)."
        ),
    )?;
    Ok(RunRepo { repo_url, remote })
}

/// The name to suggest: the last real run's repository while it is still empty (that run may
/// never have been approved), else the next name after it. Shown as `<name>` when it is the
/// participant's own, else `<owner>/<name>`.
fn suggestion(doors: &dyn Doors, last: &str, user: Option<&str>) -> Option<String> {
    let (owner, name) = github::owner_repo(last).ok()?;
    let used = doors
        .api()
        .is_some_and(|api| github::check_public_empty(&api, last).is_err());
    let name = if used {
        github::next_name(name)
    } else {
        name.to_string()
    };
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
