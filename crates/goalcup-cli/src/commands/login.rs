//! `goalcup login` — approve the participant's account in the browser and store the upload
//! server and the account handle.
//!
//! [`run`] is the device-flow login: the participant approves in the browser
//! ([`crate::account::device`]) and nothing secret comes back, only the account handle.
//! [`login_with`] is the older half, which pastes a token read from stdin, never from the
//! command line (a shell history and `ps` both show argv) and never printed; `main.rs` turns
//! terminal echo off while it is pasted. It stays until Task T5 moves `main.rs` to [`run`].
//! What is stored and where: [`crate::server`].

use crate::CliError;
use crate::account::device::{Approved, Purpose, approve};
use crate::server::{
    DEFAULT_SERVER_URL, ServerConfig, checked_server_url, checked_token, read_server_config,
    write_server_config,
};
use crate::upload::http::ServerClient;
use std::path::Path;

/// What `goalcup login` was given.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoginArgs {
    /// `--server <URL>`. Without it the server the last login stored, else
    /// [`DEFAULT_SERVER_URL`].
    pub server: Option<String>,
}

/// The device-flow login: ask the server for approval of this account, wait for the
/// participant to give it in the browser, and only then store `<cfg>/server.json` (mode 0600)
/// with the server and the account handle. A denied or expired approval, or a server that
/// cannot be reached, leaves `cfg` as it was.
pub fn run(
    args: &LoginArgs,
    cfg: &Path,
    open: &dyn Fn(&str) -> bool,
    out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    let url = match &args.server {
        Some(url) => url.clone(),
        // An unreadable `server.json` is not repaired by logging in the same way again: say
        // which file, and how to start over with an address.
        None => match read_server_config(cfg) {
            Ok(Some(stored)) => stored.url,
            Ok(None) => DEFAULT_SERVER_URL.to_string(),
            Err(_) => {
                return Err(CliError::Refused(format!(
                    "{} cannot be used (it is unreadable, or other users can read it). Run \
                     `goalcup login --server <URL>` to replace it, or delete it.",
                    crate::server::server_path(cfg).display()
                )));
            }
        },
    };
    let url = checked_server_url(&url)?;
    let handle = match approve(
        &ServerClient::anonymous(&url),
        Purpose::Login,
        "none",
        open,
        out,
    )? {
        Approved::Login { handle } => handle,
        Approved::Run(_) => {
            return Err(CliError::Internal(
                "the server approved a run when a login was asked for. Nothing was stored; \
                 run `goalcup login` again, and report it if it keeps happening."
                    .into(),
            ));
        }
    };
    write_server_config(
        cfg,
        &ServerConfig {
            url: url.clone(),
            account: Some(handle.clone()),
            token: String::new(),
        },
    )?;
    writeln!(out, "Logged in as {handle}. Runs are uploaded to {url}.").map_err(|source| {
        CliError::Io {
            path: "stdout".to_string(),
            source,
        }
    })
}

/// The testable half: check `server` and the first line of `token_input`, store both at
/// `<cfg>/server.json` (mode 0600), and say where, without the token.
pub fn login_with(
    cfg: &Path,
    server: &str,
    token_input: &mut dyn std::io::BufRead,
    out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    let url = checked_server_url(server)?;
    use std::io::{BufRead, Read};
    // Bounded: a token is at most TOKEN_MAX_LEN characters, and a paste of a whole file must
    // not be read into memory whole before it is refused.
    let mut line = String::new();
    token_input
        .take(4 * crate::server::TOKEN_MAX_LEN as u64)
        .read_line(&mut line)
        .map_err(|source| CliError::Io {
            path: "stdin".to_string(),
            source,
        })?;
    let token = checked_token(&line)?;
    let path = write_server_config(
        cfg,
        &ServerConfig {
            url: url.clone(),
            account: None,
            token,
        },
    )?;
    let say = |out: &mut dyn std::io::Write, line: String| {
        writeln!(out, "{line}").map_err(|source| CliError::Io {
            path: "stdout".to_string(),
            source,
        })
    };
    say(out, format!("Logged in. Runs are uploaded to {url}."))?;
    say(
        out,
        format!(
            "The token is in {}, readable by your user alone. It is never shown, never put in \
             the agent's environment or on its command line, and never written into a trace.",
            path.display()
        ),
    )
}
