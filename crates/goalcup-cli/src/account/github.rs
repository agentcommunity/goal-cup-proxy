//! The public GitHub repository a real run pushes to. Goal Cup holds no key for it (D69).
//!
//! `goalcup start` asks for a real run's repository (`commands::run_repo`), opens GitHub's "new
//! repository" page when it does not exist yet, and checks it with nothing of the participant's:
//! [`check_public_empty`] asks GitHub's open API, with no login, that it is public and empty.
//! Goal Cup makes no clone and runs no git: the agent works in the participant's project folder,
//! runs `git init` there and pushes with the token the participant put in that folder's
//! `.env.local`, for this one repository.
//!
//! # The older token flow, kept for old clones
//!
//! Versions before D69 stored a token in `<cfg>/github.json` and set each clone's credential
//! helper to `goalcup git-credential`. Those clones keep working: [`credential_helper`] still
//! answers git from that file ([`GithubSetup::load`]), for that one repository. Nothing writes
//! the file any more. Once the dry runs started on 0.6.1 are over, this half can go.

use crate::CliError;
use crate::upload::http::{ServerClient, ServerError};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

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
    /// In kilobytes; 0 for a repository with no commits, and often for one whose only commit is
    /// a README GitHub made, which [`check_public_empty`] asks about apart.
    size: u64,
    #[serde(default)]
    owner: Option<OwnerAnswer>,
}

#[derive(Deserialize)]
struct OwnerAnswer {
    id: u64,
}

/// What GitHub's open API says about a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    /// No such repository can be seen without a login: it does not exist yet, or it is private.
    Missing,
    /// It is public and has no commits. `owner_id` is its owner's GitHub id, for the owner's
    /// private commit address (`<id>+<owner>@users.noreply.github.com`), when GitHub said it.
    PublicEmpty { owner_id: Option<u64> },
    /// GitHub would not answer just now: its open API allows 60 calls an hour from one address
    /// (`403` or `429`). Not a sign the repository is used. The server checks that it is public
    /// and empty when the run opens.
    Unchecked,
}

/// Ask GitHub's API (`api_base`, [`GITHUB_API`] in production) about `repo_url` with no login
/// at all, so nothing of the participant's is sent. A public, empty repository is
/// [`Found::PublicEmpty`]; one that cannot be seen is [`Found::Missing`]; a rate limit is
/// [`Found::Unchecked`]. A repository that is not empty, or any other failure, is refused in a
/// sentence of its own.
///
/// Empty is asked twice. GitHub's `size` says 0 for a repository whose only commit is the README
/// it made, so its list of commits is asked too (`GET /repos/{owner}/{repo}/commits?per_page=1`,
/// as the server asks at open): `409` is an empty repository, `200` one with a commit, and any
/// other answer leaves the `size` answer standing (the server asks again at open).
pub fn check_public_empty(api_base: &str, repo_url: &str) -> Result<Found, CliError> {
    let (owner, repo) = parse_repo_url(repo_url)?;
    let github = ServerClient::anonymous(api_base);
    let answer: RepoAnswer =
        match github.json("GET", &format!("/repos/{owner}/{repo}"), None::<&()>) {
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
    let not_empty = |kb| {
        CliError::Refused(crate::participant::repository_not_empty(
            &format!("{owner}/{repo}"),
            kb,
        ))
    };
    if answer.size != 0 {
        return Err(not_empty(Some(answer.size)));
    }
    if github
        .json::<serde::de::IgnoredAny>(
            "GET",
            &format!("/repos/{owner}/{repo}/commits?per_page=1"),
            None::<&()>,
        )
        .is_ok()
    {
        return Err(not_empty(None));
    }
    Ok(Found::PublicEmpty {
        owner_id: answer.owner.map(|o| o.id),
    })
}

/// GitHub's page that creates a repository, with the name, the owner and public visibility
/// filled in (GitHub documents these query parameters; one it cannot use is ignored).
pub fn new_repo_page(owner: &str, repo: &str) -> String {
    format!("https://github.com/new?name={repo}&owner={owner}&visibility=public")
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
