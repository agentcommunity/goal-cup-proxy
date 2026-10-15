//! The public GitHub repository a run pushes to, and the token that can push only there.
//!
//! `goalcup prep` asks the participant for an empty public repository and a GitHub token that
//! can push to that one repository. [`check`] refuses a repository that is private, not empty
//! or not pushable; [`clone_into`] makes the clone the agent works in; and
//! [`credential_helper`] is what `goalcup git-credential` runs when git asks for a password,
//! so the agent pushes with a plain `git push`.
//!
//! # The token stays here
//!
//! The token lives in one file, `<cfg>/github.json`, at mode 0600 under the tracker's own
//! configuration directory, and is handed out in exactly two places:
//!
//! - to GitHub's API, as the bearer of [`check`]'s one request;
//! - to git, on its standard input and output, by [`credential_helper`], and only for a
//!   request naming `https://github.com/<owner>/<repo>` for this very repository.
//!
//! It is never in the clone's configuration, a remote URL, a command line or an environment
//! variable (the helper is a command that reads the file when git runs it); never printed or
//! logged ([`GithubSetup`]'s `Debug` prints `<redacted>`, and no message in this module
//! formats the token); and never read from a command line (`goalcup prep` reads it from stdin
//! with echo off, [`GithubSetup::read_token`]).
//!
//! Mode 0600 keeps other users out, not the agent: the agent runs as the participant, so it
//! can read the file and can ask the helper for the token. What bounds the damage is the
//! token itself: it can push to one public, empty-at-the-start repository and nothing else,
//! and the participant can revoke it on GitHub (docs/CHEAT-CHECKLIST.md).

use crate::CliError;
use crate::config::write_secret_file;
use crate::upload::http::{ServerClient, ServerError};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// `<cfg>/github.json`.
pub const GITHUB_FILE_NAME: &str = "github.json";

/// GitHub's API. [`check`] takes the base as an argument so a test can point it at a fake.
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

/// The repository and the token for it, as `goalcup prep` stored them.
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
    /// `repo_url` and `token` checked and normalised ([`GithubSetup::read_token`] reads the
    /// token from a line instead).
    pub fn new(repo_url: &str, token: &str) -> Result<GithubSetup, CliError> {
        let (owner, repo) = parse_repo_url(repo_url)?;
        Ok(GithubSetup {
            repo_url: format!("{URL_PREFIX}{owner}/{repo}"),
            token: checked_token(token)?,
        })
    }

    /// `repo_url` checked, then ONE line of `input` as the token: bounded, so a paste of a
    /// whole file is not read into memory before it is refused. `goalcup prep` calls it with
    /// terminal echo off.
    pub fn read_token(repo_url: &str, input: &mut dyn BufRead) -> Result<GithubSetup, CliError> {
        // The URL first: a participant with a bad URL is not asked to paste a token twice.
        parse_repo_url(repo_url)?;
        let mut line = String::new();
        // Bounded: a token is at most TOKEN_MAX_LEN characters, and a paste of a whole file
        // must not be read into memory whole before it is refused.
        Read::take(&mut *input, 4 * TOKEN_MAX_LEN as u64)
            .read_line(&mut line)
            .map_err(|source| CliError::Io {
                path: "stdin".to_string(),
                source,
            })?;
        GithubSetup::new(repo_url, &line)
    }

    /// `(owner, repo)` of [`GithubSetup::repo_url`]. A `repo_url` that was set by hand to
    /// something that is not the normal form gives two empty strings, which no check or
    /// clone accepts.
    pub fn owner_repo(&self) -> (&str, &str) {
        parse_repo_url(&self.repo_url).unwrap_or(("", ""))
    }

    /// `(owner, repo)`, or the refusal for a `repo_url` that was set by hand.
    fn parts(&self) -> Result<(&str, &str), CliError> {
        parse_repo_url(&self.repo_url)
    }

    /// Store at `<cfg>/github.json`, mode 0600, replacing any earlier one.
    pub fn store(&self, cfg: &Path) -> Result<PathBuf, CliError> {
        let path = github_path(cfg);
        write_secret_file(&path, &serde_json::to_vec_pretty(self)?)?;
        Ok(path)
    }

    /// `<cfg>/github.json`.
    ///
    /// The file must be the owner's alone: one that the group or others can read is refused,
    /// naming the fix, because the token in it would no longer be the participant's alone.
    /// Its URL and token are checked again as [`GithubSetup::new`] checked them. No file is
    /// a refusal that says to run `goalcup prep`.
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
                    "no GitHub repository is set up; run `goalcup prep`".into(),
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
/// that names the form. `goalcup prep` checks the address it was given before it asks for a
/// token, so a mistyped address costs no paste.
pub fn checked_repo_url(url: &str) -> Result<String, CliError> {
    let (owner, repo) = parse_repo_url(url)?;
    Ok(format!("{URL_PREFIX}{owner}/{repo}"))
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
             address of the repository's page on github.com, with no user name, query or \
             extra path."
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
    /// Absent for a caller that cannot see what it may do: read as "cannot push".
    #[serde(default)]
    permissions: Option<Permissions>,
}

#[derive(Deserialize)]
struct Permissions {
    #[serde(default)]
    push: bool,
}

/// Ask GitHub (`api_base`, [`GITHUB_API`] in production) about the repository with the token,
/// and refuse unless it is public, empty and pushable: three different sentences, in that
/// order, each saying what to do. A wrong token, a repository the token cannot see, and any
/// other failure are refused in a sentence of their own.
pub fn check(api_base: &str, s: &GithubSetup) -> Result<(), CliError> {
    judge(api_base, s, true, true)
}

/// [`check`]'s access half: the repository is public and the token can push to it. This is
/// what must keep being true for the week of a run, and what `goalcup prep` asks on every run:
/// a token that expired or was revoked, or a repository made private, fails it.
pub fn check_access(api_base: &str, s: &GithubSetup) -> Result<(), CliError> {
    judge(api_base, s, true, false)
}

/// [`check`]'s other half: the repository has no commits. It is true only until the first push,
/// so it is asked until a repository has once passed, and never again for that repository.
pub fn check_empty(api_base: &str, s: &GithubSetup) -> Result<(), CliError> {
    judge(api_base, s, false, true)
}

/// One request to GitHub, judged on the halves asked for, in the order public, empty, pushable.
fn judge(api_base: &str, s: &GithubSetup, access: bool, empty: bool) -> Result<(), CliError> {
    let (owner, repo) = s.parts()?;
    let client = ServerClient::bearer(api_base, &s.token);
    let answer: RepoAnswer = client
        .json("GET", &format!("/repos/{owner}/{repo}"), None::<&()>)
        .map_err(|e| match e {
            ServerError::Status { status, .. } if status == 401 || status == 403 => {
                CliError::Refused(format!(
                    "GitHub refused the token (HTTP {status}). It may be mistyped, expired or \
                     revoked. Create a fine-grained token for {owner}/{repo} with Contents set \
                     to read and write, then run `goalcup prep` again."
                ))
            }
            ServerError::Status {
                status: 301 | 302 | 307 | 308,
                ..
            } => CliError::Refused(format!(
                "GitHub says {owner}/{repo} has moved or been renamed. Open the repository on \
                 GitHub, copy its current address, then run `goalcup prep` again with it."
            )),
            ServerError::Status { status: 404, .. } => CliError::Refused(format!(
                "the repository {owner}/{repo} was not found, or the token cannot see it. \
                 Check the address, and that the token was created for this repository, then \
                 run `goalcup prep` again."
            )),
            other => CliError::Refused(format!(
                "could not check {owner}/{repo} on GitHub: {other}. Try again in a minute."
            )),
        })?;
    if access && answer.private {
        return Err(CliError::Refused(format!(
            "the repository {owner}/{repo} is private, and Goal Cup needs a public one. Make it \
             public (Settings, then Danger Zone, then Change visibility) or create a new public \
             repository, then run `goalcup prep` again."
        )));
    }
    if empty && answer.size != 0 {
        return Err(CliError::Refused(format!(
            "the repository {owner}/{repo} is not empty ({} KB). Goal Cup needs a new \
             repository with no README, license or .gitignore: create one, then run `goalcup \
             prep` again.",
            answer.size
        )));
    }
    if access && !answer.permissions.is_some_and(|p| p.push) {
        return Err(CliError::Refused(format!(
            "the token cannot push to {owner}/{repo}. Create a fine-grained token for that one \
             repository with Contents set to read and write, then run `goalcup prep` again."
        )));
    }
    Ok(())
}

// ------------------------------------------------------------------ the clone

/// Clone the repository into `dest` and set the clone up to push with `goalcup git-credential`.
///
/// The clone itself carries no token: the repository is public, so `git clone` needs none.
/// Afterwards the clone's own configuration (never the participant's global one) gets:
///
/// - an empty `credential.https://github.com.helper`, git's documented reset, so the
///   participant's own helpers (a keychain, `gh auth git-credential`) are not asked about
///   this repository and cannot answer with someone else's login;
/// - `credential.https://github.com.helper = "!GOALCUP_CONFIG_DIR=<cfg> <goalcup_exe>
///   git-credential"`, both shell-quoted. The agent's git runs it in the harness's
///   environment, where `$HOME` and `$GOALCUP_CONFIG_DIR` may differ from the tracker's, so
///   the configuration directory is named in the command;
/// - `credential.useHttpPath = true`, so git sends the repository's path and the helper can
///   answer for this repository alone.
///
/// `goalcup_exe` and `cfg` are absolute: the helper runs from wherever the agent is.
pub fn clone_into(
    s: &GithubSetup,
    dest: &Path,
    goalcup_exe: &Path,
    cfg: &Path,
) -> Result<(), CliError> {
    clone_with(OsStr::new("git"), &s.repo_url, s, dest, goalcup_exe, cfg)
}

/// [`clone_into`] with this `git` program and a clone `source` that is not the repository: a
/// local bare repository, in a test. The clone's `origin` is then set to the repository's URL,
/// so it looks exactly as production's does. Shipped builds have no such door.
#[cfg(feature = "test-support")]
pub fn clone_from(
    git: &Path,
    source: &str,
    s: &GithubSetup,
    dest: &Path,
    goalcup_exe: &Path,
    cfg: &Path,
) -> Result<(), CliError> {
    clone_with(git.as_os_str(), source, s, dest, goalcup_exe, cfg)
}

fn clone_with(
    git: &OsStr,
    source: &str,
    s: &GithubSetup,
    dest: &Path,
    goalcup_exe: &Path,
    cfg: &Path,
) -> Result<(), CliError> {
    s.parts()?;
    for (what, path) in [("goalcup", goalcup_exe), ("configuration directory", cfg)] {
        if !path.is_absolute() {
            return Err(CliError::Refused(format!(
                "the {what} path {} is not absolute. The agent's git runs the credential \
                 helper from wherever it is, so the helper's paths must not depend on that.",
                path.display()
            )));
        }
    }
    let (Some(exe), Some(cfg)) = (goalcup_exe.to_str(), cfg.to_str()) else {
        return Err(CliError::Refused(
            "the goalcup path and the configuration directory must be valid UTF-8 to be put \
             in a git configuration."
                .into(),
        ));
    };
    let helper = format!(
        "!GOALCUP_CONFIG_DIR={} {} git-credential",
        shell_quote(cfg),
        shell_quote(exe)
    );

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
        &format!("git could not clone {}", s.repo_url),
    )?;
    let in_clone = |args: &[&str]| -> Result<(), CliError> {
        let mut full: Vec<&OsStr> = vec![OsStr::new("config"), OsStr::new("--local")];
        full.extend(args.iter().map(OsStr::new));
        run_git(
            git,
            full,
            Some(dest),
            CliError::Internal,
            "git could not configure the clone",
        )
    };
    if source != s.repo_url {
        run_git(
            git,
            [
                OsStr::new("remote"),
                OsStr::new("set-url"),
                OsStr::new("origin"),
                OsStr::new(&s.repo_url),
            ],
            Some(dest),
            CliError::Internal,
            "git could not set the clone's origin",
        )?;
    }
    in_clone(&["--add", HELPER_KEY, ""])?;
    in_clone(&["--add", HELPER_KEY, &helper])?;
    in_clone(&["credential.useHttpPath", "true"])
}

/// The configuration key of git's credential helpers for `https://github.com`.
const HELPER_KEY: &str = "credential.https://github.com.helper";

/// Run `git args` (in `dir` with `-C`, when given) with no terminal prompt and no stdin, and
/// turn a failure into `fail("<what>: <last lines of git's stderr>")`. Nothing passed here
/// carries the token, so nothing git says can.
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

/// `text` as one POSIX shell word: single-quoted, with each `'` as `'\''`.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

// ------------------------------------------------------------------ the credential helper

/// What `goalcup git-credential <op>` does: git's credential-helper protocol, for this
/// repository alone.
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
