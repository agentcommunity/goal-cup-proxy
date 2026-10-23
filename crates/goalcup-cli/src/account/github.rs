//! The public GitHub repository a real run pushes to. Goal Cup holds no key for it (D69).
//!
//! `goalcup start` asks for a real run's repository (`commands::run_repo`), opens GitHub's "new
//! repository" page when it does not exist yet, and checks it with nothing of the participant's:
//! [`check_public_empty`] asks GitHub's open API, with no login, that it is public and empty, and
//! [`push_test`] asks the participant's own git for a dry-run push. [`clone_repo`] makes the
//! clone the agent works in, with the participant's own git and login, over SSH when
//! [`find_login`] saw SSH work. A practice run has no repository: [`init_local`]. Nothing here
//! reads, stores or passes a key: the agent pushes with the participant's own git login, as
//! their laptop already does.
//!
//! # The older token flow, kept for old clones
//!
//! Versions before D69 stored a token in `<cfg>/github.json` and set each clone's credential
//! helper to `goalcup git-credential`. Those clones keep working: [`credential_helper`] still
//! answers git from that file ([`GithubSetup::load`]), for that one repository. Nothing writes
//! the file any more. Once the dry runs of the week of 23 Oct are over, this half can go.

use crate::CliError;
use crate::upload::http::{ServerClient, ServerError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// `<cfg>/github.json`.
pub const GITHUB_FILE_NAME: &str = "github.json";

/// GitHub's API. [`check_public_empty`] takes the base as an argument so a test can point it at
/// a fake.
pub const GITHUB_API: &str = "https://api.github.com";

/// The most of `github.json` [`GithubSetup::load`] reads. The document is a URL and a token.
const GITHUB_FILE_MAX_BYTES: u64 = 16 * 1024;

/// The shortest and longest token accepted. GitHub's tokens are 40 characters (classic) to
/// about 100 (fine-grained); these bounds only catch a paste of the wrong thing.
pub const TOKEN_MIN_LEN: usize = 20;
pub const TOKEN_MAX_LEN: usize = 255;

/// The most of a credential request [`credential_helper`] reads. Git sends a few lines.
const HELPER_INPUT_MAX_BYTES: u64 = 64 * 1024;

/// What the URL of the repository looks like, for every refusal that names it.
const URL_FORM: &str = "https://github.com/<owner>/<repo>";
const URL_PREFIX: &str = "https://github.com/";

/// The username GitHub accepts with an access token as the password.
const HELPER_USERNAME: &str = "x-access-token";

/// The repository and the token for it, as `goalcup prep` stored them before D69.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GithubSetup {
    /// `https://github.com/<owner>/<repo>`: no `.git`, no trailing slash, case as given.
    pub repo_url: String,
    /// The access token. Never printed.
    token: String,
}

impl std::fmt::Debug for GithubSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GithubSetup")
            .field("repo_url", &self.repo_url)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl GithubSetup {
    /// `repo_url` and `token` checked and normalised.
    pub fn new(repo_url: &str, token: &str) -> Result<GithubSetup, CliError> {
        let (owner, repo) = parse_repo_url(repo_url)?;
        Ok(GithubSetup {
            repo_url: format!("{URL_PREFIX}{owner}/{repo}"),
            token: checked_token(token)?,
        })
    }

    /// `(owner, repo)`, or the refusal for a `repo_url` that was set by hand.
    fn parts(&self) -> Result<(&str, &str), CliError> {
        parse_repo_url(&self.repo_url)
    }

    /// Store at `<cfg>/github.json`, mode 0600, replacing any earlier one. Only a test writes
    /// the file now, to stand in for a clone an older version made (D69): no shipped build can.
    #[cfg(feature = "test-support")]
    pub fn store(&self, cfg: &Path) -> Result<PathBuf, CliError> {
        let path = github_path(cfg);
        crate::config::write_secret_file(&path, &serde_json::to_vec_pretty(self)?)?;
        Ok(path)
    }

    /// `<cfg>/github.json`.
    ///
    /// The file must be the owner's alone: one that the group or others can read is refused,
    /// naming the fix, because the token in it would no longer be the participant's alone.
    /// Its URL and token are checked again as [`GithubSetup::new`] checked them. No file is
    /// a refusal that says to run `goalcup setup`.
    pub fn load(cfg: &Path) -> Result<GithubSetup, CliError> {
        let path = github_path(cfg);
        let io = |source: std::io::Error| CliError::Io {
            path: path.display().to_string(),
            source,
        };
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Err(CliError::Refused(
                    "this clone was made by an older goalcup, and its GitHub token is gone. Push \
                     with your own git login instead: git remote set-url origin \
                     git@github.com:<owner>/<repo>.git"
                        .into(),
                ));
            }
            Err(source) => return Err(io(source)),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = file.metadata().map_err(io)?.permissions().mode();
            if mode & 0o077 != 0 {
                return Err(CliError::Refused(format!(
                    "{} can be read by other users (mode {:o}), so the token in it is no \
                     longer yours alone. Revoke that token on GitHub and run `goalcup prep` \
                     again with a new one, or `chmod 600` the file.",
                    path.display(),
                    mode & 0o777
                )));
            }
        }
        let mut text = String::new();
        let read = file
            .take(GITHUB_FILE_MAX_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(io)?;
        if read as u64 > GITHUB_FILE_MAX_BYTES {
            return Err(CliError::Refused(format!(
                "{} is larger than {GITHUB_FILE_MAX_BYTES} bytes. Run `goalcup prep` again.",
                path.display()
            )));
        }
        let stored: GithubSetup = serde_json::from_str(&text).map_err(|_| {
            CliError::Refused(format!(
                "{} is not a document `goalcup prep` wrote. Run `goalcup prep` again.",
                path.display()
            ))
        })?;
        GithubSetup::new(&stored.repo_url, &stored.token)
    }
}

/// `url` as `https://github.com/<owner>/<repo>` (no `.git`, no trailing slash), or the refusal
/// that names the form. `<owner>/<repo>` alone is read as that address.
pub fn checked_repo_url(url: &str) -> Result<String, CliError> {
    let trimmed = url.trim();
    let full;
    let url = if !trimmed.contains("://") && trimmed.matches('/').count() == 1 {
        full = format!("{URL_PREFIX}{trimmed}");
        full.as_str()
    } else {
        url
    };
    let (owner, repo) = parse_repo_url(url)?;
    Ok(format!("{URL_PREFIX}{owner}/{repo}"))
}

/// `(owner, repo)` of a repository address in [`checked_repo_url`]'s form.
pub fn owner_repo(url: &str) -> Result<(&str, &str), CliError> {
    parse_repo_url(url)
}

/// The two git remotes of a repository: over SSH, `git@github.com:<owner>/<repo>.git`, used when
/// the participant's SSH login to GitHub works ([`find_login`]); otherwise over HTTPS.
pub fn remote_url(owner: &str, repo: &str, ssh: bool) -> String {
    if ssh {
        format!("git@github.com:{owner}/{repo}.git")
    } else {
        format!("{URL_PREFIX}{owner}/{repo}.git")
    }
}

/// `<cfg>/github.json`.
pub fn github_path(cfg: &Path) -> PathBuf {
    cfg.join(GITHUB_FILE_NAME)
}

/// `(owner, repo)` of `url`, borrowed from it. The forms accepted are
/// `https://github.com/<owner>/<repo>`, with an optional `.git` and an optional trailing `/`.
/// The owner is 1 to 39 of `[A-Za-z0-9-]`; the repository is 1 to 100 of `[A-Za-z0-9._-]`,
/// is not `.` or `..`, and does not itself end in `.git`. A refusal names the expected form
/// and repeats the input only when it starts with `https://` and carries no user info.
fn parse_repo_url(url: &str) -> Result<(&str, &str), CliError> {
    // Echoed only when it looks like an address: a token pasted at the wrong prompt, or a
    // URL with user info, is not repeated.
    let shown = if url.contains('@') || !url.trim().starts_with("https://") {
        "given".to_string()
    } else {
        format!("{:?}", url.chars().take(100).collect::<String>())
    };
    let refused = || {
        CliError::Refused(format!(
            "the repository address {shown} is not of the form {URL_FORM}. Give the https \
             address of the repository's page on github.com, or <owner>/<repo>, with no user \
             name, query or extra path."
        ))
    };
    let rest = url.trim().strip_prefix(URL_PREFIX).ok_or_else(refused)?;
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, repo) = rest.split_once('/').ok_or_else(refused)?;
    let owner_ok = (1..=39).contains(&owner.len())
        && owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-');
    let repo_ok = (1..=100).contains(&repo.len())
        && repo
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && repo != "."
        && repo != ".."
        && !repo.ends_with(".git");
    if owner_ok && repo_ok {
        Ok((owner, repo))
    } else {
        Err(refused())
    }
}

/// `token` as pasted, checked: surrounding whitespace removed, then [`TOKEN_MIN_LEN`] to
/// [`TOKEN_MAX_LEN`] visible ASCII characters. The refusal never repeats the token.
fn checked_token(token: &str) -> Result<String, CliError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(CliError::Refused(
            "no token was given. Create a fine-grained GitHub token for the repository and \
             paste it."
                .into(),
        ));
    }
    if !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(CliError::Refused(
            "the token holds a space or a character that is not printable ASCII, so it is not \
             a GitHub token. Copy it again from GitHub."
                .into(),
        ));
    }
    if !(TOKEN_MIN_LEN..=TOKEN_MAX_LEN).contains(&token.len()) {
        return Err(CliError::Refused(format!(
            "the token is {} characters long, and a GitHub token is {TOKEN_MIN_LEN} to \
             {TOKEN_MAX_LEN}. Copy it again from GitHub.",
            token.len()
        )));
    }
    Ok(token.to_string())
}

// ------------------------------------------------------------------ the repository check

/// The parts of GitHub's `GET /repos/{owner}/{repo}` answer this check reads.
#[derive(Deserialize)]
struct RepoAnswer {
    private: bool,
    /// In kilobytes; 0 for a repository with no commits.
    size: u64,
}

/// What GitHub's open API says about a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    /// No such repository can be seen without a login: it does not exist yet, or it is private.
    Missing,
    /// It is public and has no commits.
    PublicEmpty,
    /// GitHub would not answer just now: its open API allows 60 calls an hour from one address
    /// (`403` or `429`). Not a sign the repository is used. The server checks that it is public
    /// when the run opens, and the clone that it is empty ([`clone_has_commits`]).
    Unchecked,
}

/// Ask GitHub's API (`api_base`, [`GITHUB_API`] in production) about `repo_url` with no login
/// at all, so nothing of the participant's is sent. A public, empty repository is
/// [`Found::PublicEmpty`]; one that cannot be seen is [`Found::Missing`]; a rate limit is
/// [`Found::Unchecked`]. A repository that is not empty, or any other failure, is refused in a
/// sentence of its own.
pub fn check_public_empty(api_base: &str, repo_url: &str) -> Result<Found, CliError> {
    let (owner, repo) = parse_repo_url(repo_url)?;
    let answer: RepoAnswer = match ServerClient::anonymous(api_base).json(
        "GET",
        &format!("/repos/{owner}/{repo}"),
        None::<&()>,
    ) {
        Ok(answer) => answer,
        Err(ServerError::Status { status: 404, .. }) => return Ok(Found::Missing),
        Err(ServerError::Status {
            status: 403 | 429, ..
        }) => return Ok(Found::Unchecked),
        Err(other) => {
            return Err(CliError::Refused(format!(
                "could not check {owner}/{repo} on GitHub: {other}. Try again in a minute."
            )));
        }
    };
    // GitHub answers a private repository with 404 to a caller with no login; this is a
    // belt-and-braces check of the answer.
    if answer.private {
        return Ok(Found::Missing);
    }
    if answer.size != 0 {
        return Err(CliError::Refused(crate::participant::repository_not_empty(
            &format!("{owner}/{repo}"),
            Some(answer.size),
        )));
    }
    Ok(Found::PublicEmpty)
}

/// GitHub's page that creates a repository, with the name, the owner and public visibility
/// filled in (GitHub documents these query parameters; one it cannot use is ignored).
pub fn new_repo_page(owner: &str, repo: &str) -> String {
    format!("https://github.com/new?name={repo}&owner={owner}&visibility=public")
}

/// Whether the clone at `clone` has a commit: `git rev-parse --verify --quiet HEAD` in it, on
/// this machine and exact. GitHub updates the `size` that [`check_public_empty`] reads with a
/// delay, so a repository pushed to a moment ago can still read as empty there; a competition or
/// test start asks this too, once it has cloned. A clone git cannot read has none.
pub fn clone_has_commits(clone: &Path) -> bool {
    run_git(
        OsStr::new("git"),
        [
            OsStr::new("rev-parse"),
            OsStr::new("--verify"),
            OsStr::new("--quiet"),
            OsStr::new("HEAD"),
        ],
        Some(clone),
        CliError::Internal,
        "git could not read the clone",
    )
    .is_ok()
}

// ------------------------------------------------------------------ the participant's own login

/// Who the participant is on GitHub, as their own tools say, and whether SSH works.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GithubLogin {
    /// The GitHub user name, when SSH or `gh` named one.
    pub user: Option<String>,
    /// `ssh git@github.com` signed in: the clone's remote is SSH.
    pub ssh: bool,
}

/// How long one look at the participant's login may take.
const LOOK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
/// How long the dry-run push may take.
const PUSH_TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// The participant's GitHub login as the agent will have it, found with their own tools and
/// nothing typed, in the agent's environment (`launch::agent_base_env`, with `scratch` as its
/// `HOME`): first `ssh -T git@github.com`, which signs in with their SSH key and answers
/// "Hi <user>!" (and exits 1, as GitHub gives no shell); then `gh api user --jq .login` when
/// `gh` is on `PATH`. Each is bounded by a timeout and a Ctrl-C, never asks a question, and none
/// of what they print is kept but the user name. A first SSH connection records GitHub's host
/// key (`accept-new`).
pub fn find_login(parent_env: &BTreeMap<String, String>, scratch: &Path) -> GithubLogin {
    let Some(env) = agent_env(parent_env, scratch) else {
        return GithubLogin::default();
    };
    let found = find_login_in(&env);
    let _ = std::fs::remove_dir_all(scratch);
    found
}

fn find_login_in(env: &BTreeMap<String, String>) -> GithubLogin {
    let ssh = [
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=accept-new",
        "-o",
        "ConnectTimeout=10",
        "git@github.com",
    ];
    if let Some((_, said)) = run_bounded(OsStr::new("ssh"), &ssh, env, LOOK_TIMEOUT)
        && let Some(user) = ssh_greeting_user(&said)
    {
        return GithubLogin {
            user: Some(user),
            ssh: true,
        };
    }
    if crate::preflight::resolve_on_path("gh", env).is_some() {
        let gh = ["api", "user", "--jq", ".login"];
        if let Some((true, said)) = run_bounded(OsStr::new("gh"), &gh, env, LOOK_TIMEOUT) {
            let user = said.lines().next().unwrap_or("").trim().to_string();
            if is_owner_name(&user) {
                return GithubLogin {
                    user: Some(user),
                    ssh: false,
                };
            }
        }
    }
    GithubLogin::default()
}

/// The agent's environment (`launch::agent_base_env`) with the new folder `scratch` as its
/// `HOME`, or `None` when the folder cannot be made or named.
fn agent_env(
    parent_env: &BTreeMap<String, String>,
    scratch: &Path,
) -> Option<BTreeMap<String, String>> {
    std::fs::create_dir_all(scratch).ok()?;
    let home = scratch.to_str()?;
    Some(crate::launch::agent_base_env(parent_env, home, true))
}

/// The user in GitHub's SSH greeting, `Hi <user>! You've successfully authenticated, ...`. A
/// deploy key's greeting names `<owner>/<repo>`, which is no user, so it is `None`.
pub fn ssh_greeting_user(said: &str) -> Option<String> {
    let after = said.split("Hi ").nth(1)?;
    let user = after.split('!').next()?;
    is_owner_name(user).then(|| user.to_string())
}

/// A GitHub user or organisation name: 1 to 39 of `[A-Za-z0-9-]`.
pub fn is_owner_name(name: &str) -> bool {
    (1..=39).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A repository name: 1 to 100 of `[A-Za-z0-9._-]`, not `.` or `..`, not ending in `.git`.
pub fn is_repo_name(name: &str) -> bool {
    (1..=100).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && name != "."
        && name != ".."
        && !name.ends_with(".git")
}

/// Whether the agent's git can push to `remote`: a dry-run push of a one-commit repository to a
/// branch the repository does not have, run in the agent's own environment
/// (`launch::agent_base_env`: the participant's `PATH` and the way to their git login, a scratch
/// `HOME`, nothing else), so a login only the participant's shell has (a `store` or `cache`
/// helper, an `include.path` under `~`, a `GH_TOKEN`) does not pass. Nothing is sent. A dry run
/// still opens git's push connection, which GitHub gives only to a login with write access; a
/// pass is git's exit 0 and nothing else. No hook or signing of theirs runs, and nothing may ask
/// a question (no terminal). `scratch` is a new folder, removed afterwards. `Err` holds git's
/// last lines.
pub fn push_test(
    git: &OsStr,
    remote: &str,
    scratch: &Path,
    parent_env: &BTreeMap<String, String>,
) -> Result<(), String> {
    let done = match agent_env(parent_env, &scratch.join("home")) {
        Some(env) => push_test_in(git, remote, &scratch.join("repo"), &env),
        None => Err(format!("{} could not be made", scratch.display())),
    };
    let _ = std::fs::remove_dir_all(scratch);
    done
}

fn push_test_in(
    git: &OsStr,
    remote: &str,
    repo: &Path,
    env: &BTreeMap<String, String>,
) -> Result<(), String> {
    let Some(dir) = repo.to_str() else {
        return Err(format!("{} is not UTF-8", repo.display()));
    };
    let quiet = |args: &[&str]| -> Result<(), String> {
        let mut full = vec![
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
        ];
        full.extend_from_slice(args);
        match run_bounded(git, &full, env, PUSH_TEST_TIMEOUT) {
            Some((true, _)) => Ok(()),
            Some((false, said)) => Err(last_lines(&said)),
            None if crate::arming::interrupted() => Err("interrupted".to_string()),
            None => Err("git did not finish within a minute".to_string()),
        }
    };
    quiet(&["init", "--quiet", dir])?;
    quiet(&[
        "-C",
        dir,
        "-c",
        "user.name=goalcup",
        "-c",
        "user.email=setup@goalcup.invalid",
        "commit",
        "--quiet",
        "--allow-empty",
        "-m",
        "goalcup setup push test",
    ])?;
    quiet(&[
        "-C",
        dir,
        "push",
        "--dry-run",
        "--quiet",
        remote,
        "HEAD:refs/heads/goalcup-setup-push-test",
    ])
}

/// Run `program args` (the participant's own `ssh`, `gh` or `git`) in exactly `env` and no
/// other variable, no stdin, no terminal prompt, in a session of its own with no controlling
/// terminal (so nothing can stop to ask for a passphrase), for at most `timeout`: `(exit 0,
/// stdout then stderr)`, or `None` when it could not start, did not finish, or a Ctrl-C came
/// ([`crate::arming::interrupted`]); its group is then killed. Nothing passed here is a key.
fn run_bounded(
    program: &OsStr,
    args: &[&str],
    env: &BTreeMap<String, String>,
    timeout: std::time::Duration,
) -> Option<(bool, String)> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .envs(env)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: `setsid` is async-signal-safe and touches nothing of this process's memory.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = command.spawn().ok()?;
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(pipe) = pipe {
                let _ = pipe.take(64 * 1024).read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let out = read(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = read(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let status = crate::harness_version::wait_or_end_group_unless(
        &mut child,
        timeout,
        &crate::arming::interrupted,
    );
    let status = status?;
    let mut said = out.join().unwrap_or_default();
    said.extend(err.join().unwrap_or_default());
    Some((
        status.success(),
        String::from_utf8_lossy(&said).into_owned(),
    ))
}

/// The last few lines of what a command said, on one line, without control characters.
fn last_lines(said: &str) -> String {
    let lines: Vec<&str> = said.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(3)..].join(" / ");
    tail.chars().filter(|c| !c.is_control()).take(400).collect()
}

// ------------------------------------------------------------------ the clone

/// Clone `source` into `dest` with the participant's own git and login, and name `remote` as
/// its `origin`. Nothing is written to the clone's configuration but that: no credential
/// helper, no token, no user name. Its `.env*` files are never committed ([`exclude_env_files`]).
/// In production `source` is `remote` itself; a test clones a local bare repository and names
/// the GitHub address after.
pub fn clone_repo(git: &OsStr, source: &str, remote: &str, dest: &Path) -> Result<(), CliError> {
    run_git(
        git,
        [
            OsStr::new("clone"),
            OsStr::new("--quiet"),
            OsStr::new("--"),
            OsStr::new(source),
            dest.as_os_str(),
        ],
        None,
        CliError::Refused,
        &format!("git could not clone {remote}"),
    )?;
    if source != remote {
        run_git(
            git,
            ["remote", "set-url", "origin", remote].map(OsStr::new),
            Some(dest),
            CliError::Internal,
            "git could not set the clone's origin",
        )?;
    }
    exclude_env_files(dest)
}

/// A practice run's work folder: a new, empty git repository at `dest` with no remote. A practice
/// run has no GitHub repository and pushes nowhere. Its agent reads no global or system git
/// config (`launch::agent_base_env`), so the repository names an author of its own,
/// [`PRACTICE_AUTHOR`], and the agent's commits work.
pub fn init_local(git: &OsStr, dest: &Path) -> Result<(), CliError> {
    run_git(
        git,
        [OsStr::new("init"), OsStr::new("--quiet"), dest.as_os_str()],
        None,
        CliError::Refused,
        "git could not make the practice folder",
    )?;
    for (key, value) in [
        ("user.name", PRACTICE_AUTHOR.0),
        ("user.email", PRACTICE_AUTHOR.1),
    ] {
        run_git(
            git,
            ["config", "--local", key, value].map(OsStr::new),
            Some(dest),
            CliError::Internal,
            "git could not configure the practice folder",
        )?;
    }
    exclude_env_files(dest)
}

/// The author a practice run's commits carry: no person's name.
pub const PRACTICE_AUTHOR: (&str, &str) = ("Goal Cup practice", "practice@goalcup.invalid");

/// Add `.env*` to the repository's own ignore list, `.git/info/exclude`, which is never
/// committed: a key file the participant gives their agent is not pushed by mistake.
fn exclude_env_files(dest: &Path) -> Result<(), CliError> {
    let info = dest.join(".git").join("info");
    let exclude = info.join("exclude");
    std::fs::create_dir_all(&info)
        .and_then(|()| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&exclude)
        })
        .and_then(|mut file| file.write_all(b"\n# goalcup: never commit a key file\n.env*\n"))
        .map_err(|source| CliError::Io {
            path: exclude.display().to_string(),
            source,
        })
}

/// The name the next real run suggests after `name`, whose repository is used: `<name>-2`, and
/// `<base>-<n+1>` for a name that already ends in `-<n>`.
pub fn next_name(name: &str) -> String {
    let numbered = name.rsplit_once('-').and_then(|(base, n)| {
        let digits = !n.is_empty() && n.len() <= 6 && n.bytes().all(|b| b.is_ascii_digit());
        (!base.is_empty() && digits).then(|| (base, n.parse::<u32>().unwrap_or(0)))
    });
    match numbered {
        Some((base, n)) if n >= 1 => format!("{base}-{}", n + 1),
        _ => format!("{name}-2"),
    }
}

/// Run `git args` (in `dir` with `-C`, when given) with no terminal prompt and no stdin, and
/// turn a failure into `fail("<what>: <last lines of git's stderr>")`. Nothing passed here
/// carries a key, so nothing git says can.
fn run_git<'a>(
    git: &OsStr,
    args: impl IntoIterator<Item = &'a OsStr>,
    dir: Option<&Path>,
    fail: fn(String) -> CliError,
    what: &str,
) -> Result<(), CliError> {
    let mut command = Command::new(git);
    if let Some(dir) = dir {
        command.arg("-C").arg(dir);
    }
    let output = command
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        // A caller's repository or configuration must not redirect `-C <dest> config --local`
        // into another repository, or add settings of its own to the clone.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                CliError::Refused(
                    "git is not installed or not on PATH. Install git (https://git-scm.com) \
                     and run the command again."
                        .into(),
                )
            } else {
                CliError::Io {
                    path: git.to_string_lossy().into_owned(),
                    source,
                }
            }
        })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(5)..].join(" / ");
    let tail: String = tail.chars().filter(|c| !c.is_control()).take(600).collect();
    Err(fail(format!("{what}: {tail}")))
}

// ------------------------------------------------------------------ the credential helper

/// What `goalcup git-credential <op>` does: git's credential-helper protocol, for this
/// repository alone. Only a clone made before D69 names it; no new clone does.
///
/// For `get`, with `protocol=https`, `host=github.com` (any case) and a `path` that is
/// `<owner>/<repo>` or `<owner>/<repo>.git` of the stored repository (any ASCII case, as
/// GitHub matches), print `username=x-access-token` and `password=<token>`. Anything else
/// prints nothing, and git moves on (to no password, and a failed push); `store`, `erase` and
/// any other operation are no-ops that do not even read the request.
///
/// `<cfg>/github.json` is read only once a request has matched on operation, protocol, host
/// and the presence of a `path`; if it cannot be read the error is returned, which git shows
/// to whoever ran it. A request that repeats a key with another value, or that runs past
/// 64 KiB, is not answered.
pub fn credential_helper(
    cfg: &Path,
    op: &str,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    if op != "get" {
        return Ok(());
    }
    let Some(request) = read_request(input)? else {
        return Ok(());
    };
    let (Some("https"), Some(host), Some(path)) = (
        request.protocol.as_deref(),
        request.host.as_deref(),
        request.path.as_deref(),
    ) else {
        return Ok(());
    };
    if !host.eq_ignore_ascii_case("github.com") {
        return Ok(());
    }
    let setup = GithubSetup::load(cfg)?;
    let (owner, repo) = setup.parts()?;
    let asked = match path.len().checked_sub(".git".len()) {
        Some(at) if path.is_char_boundary(at) && path[at..].eq_ignore_ascii_case(".git") => {
            &path[..at]
        }
        _ => path,
    };
    let ours = asked.len() == owner.len() + 1 + repo.len()
        && asked.is_char_boundary(owner.len())
        && asked[..owner.len()].eq_ignore_ascii_case(owner)
        && asked.as_bytes()[owner.len()] == b'/'
        && asked[owner.len() + 1..].eq_ignore_ascii_case(repo);
    if !ours {
        return Ok(());
    }
    write!(
        out,
        "username={HELPER_USERNAME}\npassword={}\n",
        setup.token
    )
    .and_then(|()| out.flush())
    .map_err(|source| CliError::Io {
        path: "stdout".to_string(),
        source,
    })
}

/// The three keys of a credential request this helper looks at.
#[derive(Default)]
struct Request {
    protocol: Option<String>,
    host: Option<String>,
    path: Option<String>,
}

/// `key=value` lines up to a blank line or the end of the input, at most
/// [`HELPER_INPUT_MAX_BYTES`] in all. `None` when the request ran past the bound or gave one
/// of the three keys twice with two values: such a request is not answered.
fn read_request(input: &mut dyn BufRead) -> Result<Option<Request>, CliError> {
    let mut limited = Read::take(&mut *input, HELPER_INPUT_MAX_BYTES);
    let mut request = Request::default();
    let mut total = 0u64;
    loop {
        let mut line = String::new();
        let read = limited
            .read_line(&mut line)
            .map_err(|source| CliError::Io {
                path: "stdin".to_string(),
                source,
            })?;
        total += read as u64;
        if total >= HELPER_INPUT_MAX_BYTES {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\n', '\r']);
        if read == 0 || line.is_empty() {
            return Ok(Some(request));
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let slot = match key {
            "protocol" => &mut request.protocol,
            "host" => &mut request.host,
            "path" => &mut request.path,
            _ => continue,
        };
        match slot {
            Some(earlier) if earlier != value => return Ok(None),
            _ => *slot = Some(value.to_string()),
        }
    }
}
