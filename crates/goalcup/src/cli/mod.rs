//! The participant commands (PRD-10): environment, home and the run and entry paths.
//!
//! PRD-10 section 2. `GOALCUP_HOME` is the only home variable; `GOALCUP_CONFIG_DIR` is not read
//! (no 0.7 compatibility). Section 4: pid liveness, `server.json` and the run and part lookup
//! that `goalcup status` and the end screen read.

pub mod finish;
pub mod log;
pub mod status;
pub mod text;

use std::env;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

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
/// ([`text::NO_HOME`]; [`text::NO_CWD`] when the current folder is gone). A relative home would put runs and the credential in the project folder,
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
        return std::path::absolute(h).map_err(|_| text::NO_CWD);
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
        _ => return Err(text::BAD_SERVER),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') || raw.contains(char::is_whitespace) {
        return Err(text::BAD_SERVER);
    }
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    };
    if !https && !matches!(host, "127.0.0.1" | "::1" | "localhost") {
        return Err(text::BAD_SERVER);
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

/// `runs/<run>/<entry>/status.json` (PRD-02 section 6, seam S4).
pub fn status_json(entry_dir: &Path) -> PathBuf {
    entry_dir.join("status.json")
}

/// `runs/<run>/<entry>/upload.json` (PRD-06, seam S9). Status passes it through, unread.
pub fn upload_json(entry_dir: &Path) -> PathBuf {
    entry_dir.join("upload.json")
}

/// One line `goalcup: <text>` on stderr. A closed stderr is not worth a panic.
pub fn say(text: &str) {
    let _ = writeln!(std::io::stderr(), "goalcup: {text}");
}

/// `runs/<run>/server.json` (section 2): written by `start` before the watchdog starts. It never
/// holds the credential (that is `runs/<run>/credential`), so `status --json` prints it whole.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerJson {
    pub v: u32,
    /// The server address; null for `GOALCUP_SERVER=off`.
    pub server: Option<String>,
    /// `r` and 16 hex.
    pub run: String,
    pub mode: RunMode,
    /// RFC 3339 UTC; null when no server set one.
    pub deadline: Option<String>,
    /// `<owner>/<name>`; null for a practice run.
    pub repo: Option<String>,
    pub connector: ConnectorSource,
    /// The `--hosts` file, when one was given.
    pub hosts_file: Option<PathBuf>,
    /// The run's parts in the order they started.
    pub entries: Vec<EntryFacts>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    Practice,
    Test,
    Competition,
}

/// The connector a run uses: `source` is `builtin` or the `--connector` file path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorSource {
    pub id: String,
    /// The harness name shown to the participant.
    pub name: String,
    pub source: String,
}

/// One part of a run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryFacts {
    /// `e` and 16 hex.
    pub id: String,
    pub harness_version: Option<String>,
    /// Whether `continue` resumed the harness's session.
    pub resumed: bool,
}

/// `server.json` of a run folder, when it is there and parses.
pub fn read_server_json(run_dir: &Path) -> Option<ServerJson> {
    serde_json::from_slice(&fs::read(server_json(run_dir)).ok()?).ok()
}

/// A run and one of its parts, as `goalcup status [PART]` picks them.
#[derive(Clone, Debug)]
pub struct Found {
    pub run: String,
    pub run_dir: PathBuf,
    pub server: Option<ServerJson>,
    /// Every part of the run, oldest first.
    pub entries: Vec<String>,
    /// The part shown: the one named, or the newest.
    pub entry: String,
}

impl Found {
    pub fn entry_dir(&self) -> PathBuf {
        self.run_dir.join(&self.entry)
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// The folders in `dir` whose name starts with `first`, oldest first by `when`.
fn dirs(dir: &Path, first: char, when: impl Fn(&Path) -> Option<SystemTime>) -> Vec<PathBuf> {
    let mut found: Vec<(Option<SystemTime>, PathBuf)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(first))
        })
        .map(|p| (when(&p), p))
        .collect();
    found.sort();
    found.into_iter().map(|(_, p)| p).collect()
}

/// The run and part for `goalcup status [PART]`: no argument is the newest run (by `server.json`
/// time, else the folder's) and its newest part; `r…` is that run; `e…` is that part in whichever
/// run holds it. None when there is no such run or it has no part yet.
pub fn find(home: &Path, arg: Option<&str>) -> Option<Found> {
    let runs = dirs(&runs_dir(home), 'r', |p| {
        mtime(&server_json(p)).or_else(|| mtime(p))
    });
    let (run_dir, entry) = match arg {
        None => (runs.last()?.clone(), None),
        Some(a) if a.starts_with('r') => (runs.into_iter().find(|p| p.ends_with(a))?, None),
        Some(a) => (
            runs.into_iter().find(|p| p.join(a).is_dir())?,
            Some(a.to_owned()),
        ),
    };
    let server = read_server_json(&run_dir);
    let mut entries: Vec<String> = match &server {
        Some(s) if !s.entries.is_empty() => s.entries.iter().map(|e| e.id.clone()).collect(),
        _ => dirs(&run_dir, 'e', mtime)
            .iter()
            .filter_map(|p| Some(p.file_name()?.to_string_lossy().into_owned()))
            .collect(),
    };
    if let Some(e) = entry.as_ref().filter(|e| !entries.contains(e)) {
        entries.push(e.clone());
    }
    let entry = entry.or_else(|| entries.last().cloned())?;
    let run = run_dir.file_name()?.to_string_lossy().into_owned();
    Some(Found {
        run,
        run_dir,
        server,
        entries,
        entry,
    })
}

/// Now, in whole seconds since 1970.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// An RFC 3339 time (`2026-11-07T10:00:01Z`, a fraction and an offset allowed) in whole seconds
/// since 1970. None when it does not parse.
pub fn unix_of(t: &str) -> Option<u64> {
    let num = |at: usize, len: usize| -> Option<i64> {
        let s = t.get(at..at + len)?;
        s.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| s.parse().ok())?
    };
    let sep = |at: usize, c: &[u8]| t.as_bytes().get(at).is_some_and(|b| c.contains(b));
    if !(sep(4, b"-") && sep(7, b"-") && sep(10, b"Tt ") && sep(13, b":") && sep(16, b":")) {
        return None;
    }
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (h, mi, s) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let mut rest = &t[19..];
    if let Some(f) = rest.strip_prefix('.') {
        rest = f.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let off = match rest {
        "Z" | "z" => 0,
        _ if rest.len() == 6 && rest.as_bytes()[3] == b':' => {
            let part = |r: std::ops::Range<usize>| rest.get(r)?.parse::<i64>().ok();
            let v = part(1..3)? * 3600 + part(4..6)? * 60;
            match rest.as_bytes()[0] {
                b'+' => v,
                b'-' => -v,
                _ => return None,
            }
        }
        _ => return None,
    };
    // Days since 1970-01-01 from the civil date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((mo + 9) % 12) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    u64::try_from(days * 86_400 + h * 3600 + mi * 60 + s - off).ok()
}

/// This computer's offset from UTC at `unix` in seconds east, and the zone's short name.
pub fn local_offset(unix: u64) -> Option<(i64, String)> {
    unsafe extern "C" {
        fn tzset();
    }
    let t: libc::time_t = unix.try_into().ok()?;
    // SAFETY: `tm` is plain data, valid when all zero.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `tzset` takes no arguments and sets libc's own zone state. `localtime_r` writes only
    // into `tm`, which outlives the call.
    let done = unsafe {
        tzset();
        libc::localtime_r(&t, &mut tm)
    };
    if done.is_null() {
        return None;
    }
    let zone = if tm.tm_zone.is_null() {
        String::new()
    } else {
        // SAFETY: a non-null `tm_zone` from `localtime_r` points at a NUL-terminated name in
        // libc's static zone table, which lives as long as the process.
        unsafe { std::ffi::CStr::from_ptr(tm.tm_zone) }
            .to_string_lossy()
            .into_owned()
    };
    #[allow(clippy::useless_conversion)] // `c_long` is 32 bits on some targets.
    let offset = i64::from(tm.tm_gmtoff);
    Some((offset, zone))
}

// Pid liveness (section 4). A pid is checked with `kill(pid, 0)` and its OS start time, so a
// reused pid is never taken for the process that wrote it down.

fn answers(pid: u32) -> bool {
    // `kill(0, 0)` and negative pids address process groups: never ask for those.
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 sends nothing; it only checks that the pid exists and may be signalled.
    pid > 0 && unsafe { libc::kill(pid, 0) } == 0
}

/// The OS start time of `pid`: on Linux field 22 of `/proc/<pid>/stat` (clock ticks after boot),
/// on macOS the BSD start time in microseconds. The same number PRD-02 writes as `start_id` in
/// `status.json` and `rec/watchdog.pid`. None when the process is gone.
#[cfg(target_os = "linux")]
pub fn proc_start_id(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The name in brackets may hold spaces and `)`; the fields after the last `)` start at 3.
    stat[stat.rfind(')')? + 1..]
        .split_whitespace()
        .nth(22 - 3)?
        .parse()
        .ok()
}

#[cfg(target_os = "macos")]
pub fn proc_start_id(pid: u32) -> Option<u64> {
    let pid = libc::c_int::try_from(pid).ok()?;
    // SAFETY: `proc_bsdinfo` is plain data, valid when all zero.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = libc::c_int::try_from(std::mem::size_of::<libc::proc_bsdinfo>()).ok()?;
    // SAFETY: the buffer is `info`, of exactly `size` bytes, and outlives the call.
    let n =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
    (n == size).then(|| info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn proc_start_id(_pid: u32) -> Option<u64> {
    None
}

/// The file name of the program `pid` runs.
#[cfg(target_os = "linux")]
fn exe_name(pid: u32) -> Option<String> {
    let exe = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    Some(exe.file_name()?.to_string_lossy().into_owned())
}

#[cfg(target_os = "macos")]
fn exe_name(pid: u32) -> Option<String> {
    let pid = libc::c_int::try_from(pid).ok()?;
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let len = u32::try_from(buf.len()).ok()?;
    // SAFETY: `buf` is a writable buffer of `len` bytes that outlives the call.
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), len) };
    buf.truncate(usize::try_from(n).ok().filter(|n| *n > 0)?);
    let path = PathBuf::from(String::from_utf8_lossy(&buf).into_owned());
    Some(path.file_name()?.to_string_lossy().into_owned())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn exe_name(_pid: u32) -> Option<String> {
    None
}

/// The pid answers and runs a `goalcup` binary.
pub fn goalcup_alive(pid: u32) -> bool {
    answers(pid) && exe_name(pid).is_some_and(|n| n.starts_with("goalcup"))
}

/// The harness (or any process written down with its start id) still runs: the pid answers and
/// its start time still matches.
pub fn harness_alive(pid: u32, start_id: u64) -> bool {
    answers(pid) && proc_start_id(pid) == Some(start_id)
}

/// `rec/watchdog.pid`: one line `<pid> <start_id>`, written by PRD-02's watchdog.
pub fn watchdog_pid(entry_dir: &Path) -> Option<(u32, u64)> {
    let text = fs::read_to_string(entry_dir.join("rec").join("watchdog.pid")).ok()?;
    let mut it = text.split_whitespace();
    let pid = it.next()?.parse().ok()?;
    let start_id = it.next()?.parse().ok()?;
    it.next().is_none().then_some((pid, start_id))
}

/// The watchdog of `entry_dir` lives: its pid is a goalcup binary with the start id written down.
/// A missing or odd `rec/watchdog.pid` means no.
pub fn watchdog_alive(entry_dir: &Path) -> bool {
    watchdog_pid(entry_dir)
        .is_some_and(|(pid, start_id)| goalcup_alive(pid) && proc_start_id(pid) == Some(start_id))
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
            assert!(
                pick_home(s("rel"), None, None, macos)
                    .unwrap()
                    .is_absolute()
            );
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
