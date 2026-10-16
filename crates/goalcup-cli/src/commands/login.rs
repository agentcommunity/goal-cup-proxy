//! `goalcup login` — approve the participant's account in the browser and store the upload
//! server and the account handle.
//!
//! [`run`] is the login: the participant approves in the browser ([`crate::account::device`])
//! and nothing secret comes back, only the account handle. What is stored and where:
//! [`crate::server`].

use crate::CliError;
use crate::account::device::{Approved, Purpose, approve};
use crate::server::{
    DEFAULT_SERVER_URL, ServerConfig, checked_server_url, read_server_config, write_server_config,
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
        },
    )?;
    // The server is named only when it is not the default one: organizers on staging need it,
    // and a participant on the default server does not.
    let named = (url.trim_end_matches('/') != DEFAULT_SERVER_URL).then_some(url.as_str());
    for line in crate::participant::login_done(&handle, named) {
        writeln!(out, "{line}").map_err(|source| CliError::Io {
            path: "stdout".to_string(),
            source,
        })?;
    }
    Ok(())
}
