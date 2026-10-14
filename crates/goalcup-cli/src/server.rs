//! The upload server this tracker sends its trace to, and the participant's account on it.
//!
//! `goalcup login` writes `<cfg>/server.json`; nothing else does. With no such file the tracker
//! is local only, exactly as before it existed.
//!
//! # What the file holds
//!
//! The file holds the server's URL and the participant's account handle, which `goalcup login`
//! learns from the approval in the browser (`account::device`). Neither is a secret. A run's
//! credential is a different thing: the server mints it when the participant approves that run,
//! and it lives in the run's own directory (`account::credential`), never here.
//!
//! The pasted token is legacy until Task T5 removes its callers. A `server.json` an older
//! `goalcup login` wrote still holds one; while it does, it is treated as it always was:
//!
//! - it lives in this one file, at mode 0600 under the tracker's own configuration directory;
//! - never in a harness's environment or on its command line: the harness environment is
//!   built from nothing (`launch::process_env` and the launch plan), and nothing reads this
//!   file into it;
//! - never printed or logged: [`ServerConfig`]'s `Debug` prints `<redacted>`, and no message
//!   in this crate formats the token;
//! - never in a trace: the daemon, which writes the trace, never reads this file.
//!
//! Mode 0600 keeps other users out, not the agent: the harness runs as the participant and can
//! read any file they can. The token only uploads this member's runs and the member can revoke
//! it on the site (docs/CHEAT-CHECKLIST.md).

use crate::CliError;
use crate::config::write_secret_file;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The upload server a `goalcup login` without `--server` talks to.
pub const DEFAULT_SERVER_URL: &str = "https://ingest.agentcommunity.org";

/// The longest account handle accepted. Handles are chosen on the site; the bound only catches
/// a damaged file.
pub const ACCOUNT_MAX_LEN: usize = 64;

/// `<cfg>/server.json`.
pub const SERVER_FILE_NAME: &str = "server.json";

/// The most of `server.json` [`read_server_config`] reads. The document is a URL, a handle and, in an older file, a token.
const SERVER_FILE_MAX_BYTES: u64 = 16 * 1024;

/// The shortest and longest token accepted. Tokens are minted by the server; these bounds only
/// catch a paste of the wrong thing (an empty line, a whole page).
pub const TOKEN_MIN_LEN: usize = 16;
pub const TOKEN_MAX_LEN: usize = 512;

/// The upload server and the account on it, as `goalcup login` stored them.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// The server's base URL, without a trailing slash: `https://…`, or `http://` to a
    /// loopback address for a local test server.
    pub url: String,
    /// The account handle the last approval named, when `goalcup login` has run since the
    /// device flow. Not a secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The pasted bearer token: legacy, empty for a device-flow login, removed with its callers
    /// (T5). Never printed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token: String,
}

impl std::fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerConfig")
            .field("url", &self.url)
            .field("account", &self.account)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// `<cfg>/server.json`.
pub fn server_path(cfg: &Path) -> PathBuf {
    cfg.join(SERVER_FILE_NAME)
}

/// `url` checked and normalised: an `https://` URL, or `http://` to `127.0.0.1`, `[::1]` or
/// `localhost` (a local test server). The host is letters, digits, `.` and `-`, or a bracketed
/// IPv6 address; a port, if given, is 1 to 65535; there is no user info, query or fragment.
/// The scheme and host are lowercased and trailing slashes removed. Anything else is refused
/// naming the rule, and a refusal never repeats a URL that carries user info (a password).
pub fn checked_server_url(url: &str) -> Result<String, CliError> {
    let shown = if url.contains('@') {
        "given".to_string()
    } else {
        format!("{url:?}")
    };
    let refused = |why: &str| {
        CliError::Refused(format!(
            "the server URL {shown} {why}. Give the https:// address you were told to use."
        ))
    };
    if url.contains(['@', '?', '#', '%', '\\'])
        || url.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(refused(
            "carries user info, a query, a fragment, an escape or whitespace",
        ));
    }
    let lower = url.to_ascii_lowercase();
    let (secure, scheme_len) = if lower.starts_with("https://") {
        (true, "https://".len())
    } else if lower.starts_with("http://") {
        (false, "http://".len())
    } else {
        return Err(refused("does not start with https://"));
    };
    let rest = &url[scheme_len..];
    let authority_len = rest.find('/').unwrap_or(rest.len());
    let authority = rest[..authority_len].to_ascii_lowercase();
    let path = &rest[authority_len..];
    // The host, and what follows it in the authority: nothing, or `:<port>`.
    let (host, port, bracketed) = match authority.strip_prefix('[') {
        Some(inner) => match inner.split_once(']') {
            Some((host, port)) => (host, port, true),
            None => return Err(refused("has an unclosed [ in its host")),
        },
        None => match authority.split_once(':') {
            Some((host, _)) => (host, &authority[host.len()..], false),
            None => (authority.as_str(), "", false),
        },
    };
    let host_ok = if bracketed {
        host.contains(':') && host.bytes().all(|b| b.is_ascii_hexdigit() || b == b':')
    } else {
        !host.is_empty()
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    };
    if !host_ok {
        return Err(refused("does not name a host"));
    }
    let port_ok = port.is_empty()
        || port
            .strip_prefix(':')
            .and_then(|p| p.parse::<u16>().ok())
            .is_some_and(|p| p != 0);
    if !port_ok {
        return Err(refused("has a port that is not 1 to 65535"));
    }
    if !secure && !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return Err(refused(
            "is plain http to a host that is not this machine; only https is accepted",
        ));
    }
    let scheme = if secure { "https://" } else { "http://" };
    Ok(format!("{scheme}{authority}{path}")
        .trim_end_matches('/')
        .to_string())
}

/// `handle` checked: 1 to [`ACCOUNT_MAX_LEN`] characters, not only whitespace, none of them a
/// control character or an invisible or direction-changing one.
pub fn checked_account(handle: &str) -> Result<String, CliError> {
    // Zero-width and direction-control characters print as nothing, or reorder what is around
    // them: a handle holding one could read as another account's.
    let invisible = |c: char| {
        matches!(
            c,
            '\u{AD}' | '\u{61C}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2069}' | '\u{FEFF}'
        )
    };
    if handle.trim().is_empty()
        || handle.chars().count() > ACCOUNT_MAX_LEN
        || handle.chars().any(|c| c.is_control() || invisible(c))
    {
        return Err(CliError::Refused(format!(
            "the account handle is not 1 to {ACCOUNT_MAX_LEN} printable characters. Run \
             `goalcup login` again."
        )));
    }
    Ok(handle.to_string())
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
/// alone. Its URL, its account handle if it has one, and its legacy token if it has one are
/// checked again as `goalcup login` checked them.
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
        account: stored.account.as_deref().map(checked_account).transpose()?,
        token: if stored.token.is_empty() {
            String::new()
        } else {
            checked_token(&stored.token)?
        },
    }))
}
