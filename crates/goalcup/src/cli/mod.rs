//! The participant commands (PRD-10): environment, home and the run and entry paths.
//!
//! PRD-10 section 2. `GOALCUP_HOME` is the only home variable; `GOALCUP_CONFIG_DIR` is not read
//! (no 0.7 compatibility). Later slices add the command modules and pid liveness.

pub mod log;
pub mod text;

use std::env;
use std::path::{Path, PathBuf};

/// The hidden subcommand PRD-02's watchdog spawns as `goalcup recorder [--null]`; `main.rs`
/// declares the same name and dispatches it to `pipeline::run_recorder`.
pub const RECORDER_SUBCOMMAND: &str = "recorder";
/// The recorder's null-mode flag, `goalcup recorder --null` (PRD-02 watchdog, null mode).
pub const NULL_FLAG: &str = "--null";

/// The server a run talks to, from `GOALCUP_SERVER`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Server {
    /// The base address, without a trailing `/`.
    Url(String),
    /// `GOALCUP_SERVER=off`: a local-only run, nothing uploaded, parts kept.
    Off,
}

const DEFAULT_SERVER: &str = "https://ingest.agentcommunity.org";
const BAD_SERVER: &str = "GOALCUP_SERVER must be off, an https:// address, or an http:// address \
                          on this computer (127.0.0.1, ::1 or localhost).";

fn var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.is_empty())
}

/// Where runs live: `GOALCUP_HOME`, else `~/Library/Application Support/goalcup` on macOS, else
/// `$XDG_CONFIG_HOME/goalcup` or `~/.config/goalcup`.
pub fn home() -> PathBuf {
    if let Some(h) = var("GOALCUP_HOME") {
        return PathBuf::from(h);
    }
    let user = PathBuf::from(var("HOME").unwrap_or_default());
    if cfg!(target_os = "macos") {
        return user.join("Library/Application Support/goalcup");
    }
    match var("XDG_CONFIG_HOME") {
        Some(x) => PathBuf::from(x).join("goalcup"),
        None => user.join(".config/goalcup"),
    }
}

/// The server from `GOALCUP_SERVER`: unset gives the production address, `off` a local-only
/// run, an `https://` address is taken, an `http://` address only for a loopback host. An
/// address with a user part (`user@`) is refused, so no credential rides in it.
pub fn server() -> Result<Server, &'static str> {
    parse_server(var("GOALCUP_SERVER").as_deref())
}

fn parse_server(raw: Option<&str>) -> Result<Server, &'static str> {
    let Some(raw) = raw else {
        return Ok(Server::Url(DEFAULT_SERVER.to_owned()));
    };
    if raw == "off" {
        return Ok(Server::Off);
    }
    let (rest, https) = match (raw.strip_prefix("https://"), raw.strip_prefix("http://")) {
        (Some(r), _) => (r, true),
        (None, Some(r)) => (r, false),
        _ => return Err(BAD_SERVER),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') || raw.contains(char::is_whitespace) {
        return Err(BAD_SERVER);
    }
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    };
    if !https && !matches!(host, "127.0.0.1" | "::1" | "localhost") {
        return Err(BAD_SERVER);
    }
    Ok(Server::Url(raw.trim_end_matches('/').to_owned()))
}

/// `GOALCUP_KEEP_PARTS=1`: keep every record file (PRD-06 section 7).
pub fn keep_parts() -> bool {
    var("GOALCUP_KEEP_PARTS").as_deref() == Some("1")
}

/// `GOALCUP_NO_BROWSER=1`: print the approval address instead of opening a browser.
pub fn no_browser() -> bool {
    var("GOALCUP_NO_BROWSER").as_deref() == Some("1")
}

/// `$GOALCUP_HOME/runs/`.
pub fn runs_dir(home: &Path) -> PathBuf {
    home.join("runs")
}

/// `$GOALCUP_HOME/runs/<run>/`.
pub fn run_dir(home: &Path, run: &str) -> PathBuf {
    runs_dir(home).join(run)
}

/// `$GOALCUP_HOME/runs/<run>/<entry>/` (PRD-01 section 1).
pub fn entry_dir(home: &Path, run: &str, entry: &str) -> PathBuf {
    run_dir(home, run).join(entry)
}

/// `$GOALCUP_HOME/setup.json`: the saved harness and the values setup wrote.
pub fn setup_json(home: &Path) -> PathBuf {
    home.join("setup.json")
}

/// `$GOALCUP_HOME/record-hosts.txt`: hosts the participant registered, one per line.
pub fn record_hosts(home: &Path) -> PathBuf {
    home.join("record-hosts.txt")
}

/// `runs/<run>/server.json`: never holds the credential.
pub fn server_json(run_dir: &Path) -> PathBuf {
    run_dir.join("server.json")
}

/// `runs/<run>/credential`: the run credential, mode 0600.
pub fn credential(run_dir: &Path) -> PathBuf {
    run_dir.join("credential")
}

/// `runs/<run>/<entry>/goalcup.log`, written only through [`log::line`].
pub fn goalcup_log(entry_dir: &Path) -> PathBuf {
    entry_dir.join("goalcup.log")
}

#[cfg(test)]
mod tests {
    use super::{Server, parse_server};

    #[test]
    fn server_accepts_only_https_or_loopback_http() {
        let url = |s: &str| Ok(Server::Url(s.to_owned()));
        assert_eq!(parse_server(None), url("https://ingest.agentcommunity.org"));
        assert_eq!(parse_server(Some("off")), Ok(Server::Off));
        assert_eq!(
            parse_server(Some("https://s.example/")),
            url("https://s.example")
        );
        assert_eq!(
            parse_server(Some("http://127.0.0.1:8787")),
            url("http://127.0.0.1:8787")
        );
        assert_eq!(parse_server(Some("http://[::1]:1")), url("http://[::1]:1"));
        assert_eq!(
            parse_server(Some("http://localhost")),
            url("http://localhost")
        );
        for bad in [
            "http://s.example",
            "http://127.0.0.1@s.example",
            "https://user:pw@s.example",
            "ftp://s.example",
            "https://",
            "s.example",
        ] {
            assert!(parse_server(Some(bad)).is_err(), "{bad}");
        }
    }
}
