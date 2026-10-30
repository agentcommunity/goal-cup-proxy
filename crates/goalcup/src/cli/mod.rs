//! The participant commands (PRD-10): environment, home and the run and entry paths.
//!
//! PRD-10 section 2. `GOALCUP_HOME` is the only home variable; `GOALCUP_CONFIG_DIR` is not read
//! (no 0.7 compatibility). Later slices add the command modules and pid liveness.

pub mod log;
pub mod text;

use std::env;
use std::path::{Path, PathBuf};

use text::BAD_SERVER;

/// The hidden subcommand PRD-02's watchdog spawns as `goalcup recorder [--null]`. `main.rs`
/// takes its subcommand and flag names from these two, so the spawn and the parser cannot drift.
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

fn var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.is_empty())
}

/// Where runs live: `GOALCUP_HOME`, else `~/Library/Application Support/goalcup` on macOS, else
/// `$XDG_CONFIG_HOME/goalcup` or `~/.config/goalcup`. Always an absolute path: a relative
/// `GOALCUP_HOME` is taken against the current folder, a relative `HOME` or `XDG_CONFIG_HOME` is
/// ignored (XDG base directory spec), and with no absolute home left it refuses
/// ([`text::NO_HOME`]). A relative home would put runs and the credential in the project folder,
/// which is pushed to a public repository (point 4).
pub fn home() -> Result<PathBuf, &'static str> {
    pick_home(
        var("GOALCUP_HOME"),
        var("HOME"),
        var("XDG_CONFIG_HOME"),
        cfg!(target_os = "macos"),
    )
}

fn pick_home(
    goalcup: Option<String>,
    user: Option<String>,
    xdg: Option<String>,
    macos: bool,
) -> Result<PathBuf, &'static str> {
    if let Some(h) = goalcup {
        return std::path::absolute(h).map_err(|_| text::NO_HOME);
    }
    let full = |v: Option<String>| v.map(PathBuf::from).filter(|p| p.is_absolute());
    let user = full(user);
    let found = match (macos, full(xdg), user) {
        (true, _, Some(u)) => Some(u.join("Library/Application Support/goalcup")),
        (false, Some(x), _) => Some(x.join("goalcup")),
        (false, None, Some(u)) => Some(u.join(".config/goalcup")),
        _ => None,
    };
    found.ok_or(text::NO_HOME)
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
    use super::{parse_server, pick_home};

    /// Point 4: runs and the credential never land in a folder relative to the project.
    #[test]
    fn home_is_never_relative() {
        let s = |v: &str| Some(v.to_owned());
        for macos in [false, true] {
            assert!(pick_home(None, None, None, macos).is_err());
            assert!(pick_home(None, s("rel"), s("rel/x"), macos).is_err());
            let home = pick_home(None, s("/u"), s("rel"), macos).unwrap();
            assert!(home.starts_with("/u"), "{home:?}");
            assert!(pick_home(s("rel"), None, None, macos).unwrap().is_absolute());
        }
    }

    /// Point 4: no credential in the address, and none sent in the clear off this computer.
    #[test]
    fn server_refuses_a_user_part_and_remote_http() {
        for bad in [
            "http://s.example",
            "http://127.0.0.1@s.example",
            "https://user:pw@s.example",
        ] {
            assert!(parse_server(Some(bad)).is_err(), "{bad}");
        }
    }
}
