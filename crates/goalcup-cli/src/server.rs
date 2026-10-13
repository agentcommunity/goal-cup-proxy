//! The upload server this tracker sends its trace to, and the participant's token for it.
//!
//! `goalcup login --server <URL>` writes `<cfg>/server.json`; nothing else does. With no such
//! file the tracker is local only, exactly as before it existed.
//!
//! # The token stays here
//!
//! The token is the participant's credential for the upload server (docs/SERVER-DESIGN.md,
//! section 3, step 2). It lives in this one file, at mode 0600 under the tracker's own
//! configuration directory, and nowhere else:
//!
//! - never in a harness's environment: the harness environment is built from nothing
//!   (`launch::process_env` and the launch plan), and nothing reads this file into it;
//! - never on a command line: `goalcup login` reads it from stdin;
//! - never printed or logged: [`ServerConfig`]'s `Debug` prints `<redacted>`, and no message
//!   in this crate formats the token;
//! - never in a trace: the daemon, which writes the trace, never reads this file.

use crate::CliError;
use crate::config::write_secret_file;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// `<cfg>/server.json`.
pub const SERVER_FILE_NAME: &str = "server.json";

/// The most of `server.json` [`read_server_config`] reads. The document is a URL and a token.
const SERVER_FILE_MAX_BYTES: u64 = 16 * 1024;

/// The shortest and longest token accepted. Tokens are minted by the server; these bounds only
/// catch a paste of the wrong thing (an empty line, a whole page).
pub const TOKEN_MIN_LEN: usize = 16;
pub const TOKEN_MAX_LEN: usize = 512;

/// The upload server and the token for it, as `goalcup login` stored them.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// The server's base URL, without a trailing slash: `https://…`, or `http://` to a
    /// loopback address for a local test server.
    pub url: String,
    /// The bearer token. Never printed.
    pub token: String,
}

impl std::fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerConfig")
            .field("url", &self.url)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// `<cfg>/server.json`.
pub fn server_path(cfg: &Path) -> PathBuf {
    cfg.join(SERVER_FILE_NAME)
}

/// `url` checked and normalised: an `https://` URL, or `http://` to `127.0.0.1`, `[::1]` or
/// `localhost` (a local test server), with no user info, query or fragment, and the trailing
/// slashes removed. Anything else is refused naming the rule.
pub fn checked_server_url(url: &str) -> Result<String, CliError> {
    let refused = |why: &str| {
        CliError::Refused(format!(
            "the server URL {url:?} {why}. Give the https:// address you were told to use."
        ))
    };
    if url.contains(['@', '?', '#']) || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(refused(
            "carries user info, a query, a fragment or whitespace",
        ));
    }
    let (secure, rest) = match (url.strip_prefix("https://"), url.strip_prefix("http://")) {
        (Some(rest), _) => (true, rest),
        (None, Some(rest)) => (false, rest),
        (None, None) => return Err(refused("does not start with https://")),
    };
    let authority = rest.split('/').next().unwrap_or_default();
    // The host, and what follows it in the authority: nothing, or `:<port>`.
    let (host, port) = match authority.strip_prefix('[') {
        Some(inner) => match inner.split_once(']') {
            Some((host, port)) => (host, port),
            None => return Err(refused("has an unclosed [ in its host")),
        },
        None => match authority.split_once(':') {
            Some((host, _)) => (host, &authority[host.len()..]),
            None => (authority, ""),
        },
    };
    if host.is_empty() {
        return Err(refused("names no host"));
    }
    let port_ok = port.is_empty()
        || port.strip_prefix(':').is_some_and(|p| {
            !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit())
        });
    if !port_ok {
        return Err(refused("has a port that is not a number"));
    }
    if !secure && !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return Err(refused(
            "is plain http to a host that is not this machine; only https is accepted",
        ));
    }
    Ok(url.trim_end_matches('/').to_string())
}

/// `token` as pasted, checked: surrounding whitespace removed, then [`TOKEN_MIN_LEN`] to
/// [`TOKEN_MAX_LEN`] visible ASCII characters. The refusal never repeats the token.
pub fn checked_token(token: &str) -> Result<String, CliError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(CliError::Refused(
            "no token was given. Copy the token from the Goal Cup site and paste it.".into(),
        ));
    }
    if !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(CliError::Refused(
            "the token holds a space or a character that is not printable ASCII, so it is not \
             a Goal Cup token. Copy it again from the Goal Cup site."
                .into(),
        ));
    }
    if !(TOKEN_MIN_LEN..=TOKEN_MAX_LEN).contains(&token.len()) {
        return Err(CliError::Refused(format!(
            "the token is {} characters long, and a Goal Cup token is {TOKEN_MIN_LEN} to \
             {TOKEN_MAX_LEN}. Copy it again from the Goal Cup site.",
            token.len()
        )));
    }
    Ok(token.to_string())
}

/// Store `config` at `<cfg>/server.json`, mode 0600, replacing any earlier one.
pub fn write_server_config(cfg: &Path, config: &ServerConfig) -> Result<PathBuf, CliError> {
    let path = server_path(cfg);
    write_secret_file(&path, &serde_json::to_vec_pretty(config)?)?;
    Ok(path)
}

/// `<cfg>/server.json`, or `None` when there is none (local-only mode).
///
/// The file must be the owner's alone: a `server.json` that the group or others can read is
/// refused, naming the fix, because the token in it would no longer be the participant's
/// alone. Its URL and token are checked again as `goalcup login` checked them.
pub fn read_server_config(cfg: &Path) -> Result<Option<ServerConfig>, CliError> {
    use std::io::Read;

    let path = server_path(cfg);
    let io = |source: std::io::Error| CliError::Io {
        path: path.display().to_string(),
        source,
    };
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(io(source)),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = file.metadata().map_err(io)?.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(CliError::Refused(format!(
                "{} can be read by other users (mode {:o}), so the token in it is no longer \
                 yours alone. Run `goalcup login` again with a new token, or `chmod 600` it.",
                path.display(),
                mode & 0o777
            )));
        }
    }
    let mut text = String::new();
    let read = file
        .take(SERVER_FILE_MAX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(io)?;
    if read as u64 > SERVER_FILE_MAX_BYTES {
        return Err(CliError::Refused(format!(
            "{} is larger than {SERVER_FILE_MAX_BYTES} bytes. Run `goalcup login` again.",
            path.display()
        )));
    }
    let stored: ServerConfig = serde_json::from_str(&text).map_err(|_| {
        CliError::Refused(format!(
            "{} is not a document `goalcup login` wrote. Run `goalcup login` again.",
            path.display()
        ))
    })?;
    Ok(Some(ServerConfig {
        url: checked_server_url(&stored.url)?,
        token: checked_token(&stored.token)?,
    }))
}
