//! `goalcup login --server <URL>` — store the upload server and the participant's token.
//!
//! The token is read from stdin, never from the command line (a shell history and `ps` both
//! show argv), and it is never printed. `main.rs` turns terminal echo off while it is pasted.
//! What is stored and where: [`crate::server`].

use crate::CliError;
use crate::server::{ServerConfig, checked_server_url, checked_token, write_server_config};
use std::path::Path;

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
            "The token is stored in {}, readable by you alone. It is never shown, never given \
             to the agent, and never written into a trace.",
            path.display()
        ),
    )
}
