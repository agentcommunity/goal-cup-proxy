//! `goalcup start` steps 3 to 5 (PRD-10 section 3): the repository pre-check, the approval in the
//! browser (device flow, `docs/UPLOAD-API.md`) and the open run. [`begin`] does all three and
//! writes `runs/<run>/server.json` and `runs/<run>/credential` (0600, never in `server.json`,
//! never printed). Nothing is created on disk before the server has opened the run, so a denial,
//! an expired code, an absent server and Ctrl-C each leave nothing behind (PRD-17 SC2). The only
//! timer is the approval code's own life (at most 15 minutes, PRD-17 P14). The device code lives
//! in memory and in request bodies only.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{Value, json};

use super::{ConnectorSource, EntryFacts, RunMode, Server, ServerJson, text};
use super::{credential, parse_server, run_dir, server_json};
use crate::pipeline::Mono;
use crate::upload::client::{ServerClient, ServerError, Token};

/// GitHub's API, for the repository pre-check.
pub const GITHUB_API: &str = "https://api.github.com";

/// The longest an approval is waited for, whatever the server says (the code's life).
const MAX_WAIT: u64 = 15 * 60;
/// The poll interval bounds, and what a `429` adds.
const INTERVAL: (u64, u64) = (1, 60);
const SLOW_DOWN: u64 = 5;
const USER_CODE_ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";

/// Why a start ended here: one plain line and the exit code (1: refused, fixable; 3: the command
/// could not do its job).
#[derive(Debug)]
pub struct Stop {
    pub text: String,
    pub code: u8,
}

fn refuse(text: String) -> Stop {
    Stop { text, code: 1 }
}

fn broke(text: String) -> Stop {
    Stop { text, code: 3 }
}

/// What `start` knows when it asks for the run.
pub struct Ask<'a> {
    pub mode: RunMode,
    pub connector: &'a ConnectorSource,
    pub connector_sha256: &'a str,
    pub workspace: &'a Path,
    pub session_id: &'a str,
    pub harness_version: Option<&'a str>,
    /// `<owner>/<name>`; none for a practice run.
    pub repo: Option<&'a str>,
    pub github_api: &'a str,
    pub hosts_file: Option<&'a Path>,
    /// A flat object of machine facts (`os`, `cpus`, `ac_power`).
    pub machine: Value,
}

/// An opened run. `deadline` is the server's `seconds_left` added to the boot clock at the moment
/// of its answer, so a wall clock set back cannot lengthen the run (PRD-17 S7); none for a
/// local-only run.
#[derive(Debug)]
pub struct Opened {
    pub run: String,
    pub entry: String,
    pub deadline: Option<Mono>,
    pub token: Option<Token>,
}

/// Steps 3 to 5. `GOALCUP_SERVER=off` asks for no approval and opens nothing.
pub fn begin(home: &Path, server: &Server, ask: &Ask, out: &mut dyn Write) -> Result<Opened, Stop> {
    let Server::Url(base) = server else {
        let (run, entry) = (random_id('r'), random_id('e'));
        let sj = server_record(None, ask, &run, &entry, None);
        write_run(home, &sj, None)?;
        return Ok(Opened {
            run,
            entry,
            deadline: None,
            token: None,
        });
    };
    let repo_url = ask
        .repo
        .map(|r| check_repo(ask.github_api, r))
        .transpose()?;
    let _watch = Ctrl::new();
    let secret = approve(base, ask, out)?;
    if interrupted() {
        return Err(stopped(text::WHY_INTERRUPTED));
    }
    let client = ServerClient::new(base, Some(Token::new(secret.clone())));
    let body = json!({
        "connector_id": ask.connector.id, "connector_sha256": ask.connector_sha256,
        "workspace": ask.workspace.to_string_lossy(), "session_id": ask.session_id,
        "harness_version": ask.harness_version, "proxy_version": env!("CARGO_PKG_VERSION"),
        "repo_url": repo_url, "machine": ask.machine,
    });
    // The participant has approved: a blip, a GitHub outage (502) or a 429 is waited out, with the
    // same `session_id`, so the open is idempotent and the approval is not spent for nothing.
    // A Ctrl-C that lands once the server has opened the run is ignored: the run exists, so it is
    // written and `start` handles the stop with the run on disk.
    let opened = send(&client, "/v1/runs", &body, true);
    if opened.is_err() && interrupted() {
        return Err(stopped(text::WHY_INTERRUPTED));
    }
    let open: Open = opened.map_err(|e| server_stop(&e, text::open_refused))?;
    let deadline = Mono::now()
        .ns()
        .saturating_add(open.seconds_left.saturating_mul(1_000_000_000));
    if !is_id(&open.run_id, 'r') || !is_id(&open.entry_id, 'e') {
        return Err(broke(text::bad_answer()));
    }
    let sj = server_record(
        Some(base),
        ask,
        &open.run_id,
        &open.entry_id,
        Some(open.deadline_at),
    );
    write_run(home, &sj, Some(&secret))?;
    Ok(Opened {
        run: open.run_id,
        entry: open.entry_id,
        deadline: Some(Mono(deadline)),
        token: Some(Token::new(secret)),
    })
}

/// `POST path`, again while the server says to wait for the code's life: a `429` always, and with
/// `outage` a transport failure or a 5xx too. A `Retry-After` is obeyed (1 to 60 s).
fn send<T: serde::de::DeserializeOwned>(
    client: &ServerClient,
    path: &str,
    body: &Value,
    outage: bool,
) -> Result<T, ServerError> {
    let until = Instant::now() + Duration::from_secs(MAX_WAIT);
    loop {
        let e = match client.json("POST", path, Some(body)) {
            Err(e) => e,
            done => return done,
        };
        let again = match &e {
            ServerError::Status { status: 429, .. } => true,
            ServerError::Status { status, .. } => outage && *status >= 500,
            ServerError::Transport(_) => outage,
            ServerError::Malformed(_) => false,
        };
        let now = Instant::now();
        if !again || interrupted() || now >= until {
            return Err(e);
        }
        let wait = e.retry_after().unwrap_or(5).clamp(INTERVAL.0, INTERVAL.1);
        nap(Duration::from_secs(wait).min(until - now));
    }
}

#[derive(Deserialize)]
struct Open {
    run_id: String,
    entry_id: String,
    deadline_at: String,
    seconds_left: u64,
}

fn server_record(
    base: Option<&str>,
    ask: &Ask,
    run: &str,
    entry: &str,
    deadline: Option<String>,
) -> ServerJson {
    ServerJson {
        v: 1,
        server: base.map(str::to_owned),
        run: run.to_owned(),
        mode: ask.mode,
        deadline,
        repo: ask.repo.map(str::to_owned),
        connector: ask.connector.clone(),
        hosts_file: ask.hosts_file.map(Path::to_path_buf),
        entries: vec![EntryFacts {
            id: entry.to_owned(),
            harness_version: ask.harness_version.map(str::to_owned),
            resumed: false,
        }],
    }
}

/// The run folder: the credential first, `server.json` last, so a run is listed only whole. A
/// failure takes the folder away again.
fn write_run(home: &Path, sj: &ServerJson, secret: Option<&str>) -> Result<(), Stop> {
    let dir = run_dir(home, &sj.run);
    let put = |path: &Path, bytes: &[u8], mode: u32| {
        let tmp = path.with_extension("tmp");
        let _ = fs::remove_file(&tmp);
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    };
    let written = fs::create_dir_all(&dir)
        .and_then(|()| secret.map_or(Ok(()), |s| put(&credential(&dir), s.as_bytes(), 0o600)))
        .and_then(|()| {
            put(
                &server_json(&dir),
                &serde_json::to_vec(sj).unwrap_or_default(),
                0o600,
            )
        });
    written.map_err(|_| {
        let _ = fs::remove_dir_all(&dir);
        broke(text::not_written(&dir))
    })
}

/// `r` or `e` and 16 hex: the ids become folder names, so the server's word is checked.
fn is_id(id: &str, first: char) -> bool {
    id.strip_prefix(first).is_some_and(|h| {
        h.len() == 16
            && h.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn random_id(first: char) -> String {
    let mut b = [0u8; 8];
    let got = File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    if got.is_err() {
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        b = (t.as_nanos() as u64).to_le_bytes();
    }
    format!("{first}{}", hex::encode(b))
}

/// Step 3: the repository is public and empty, asked of GitHub with no token. Returns the address
/// the server is told. A GitHub that cannot answer does not stop the start: the server asks again
/// when it opens the run.
pub fn check_repo(api: &str, repo: &str) -> Result<String, Stop> {
    let ok = |s: &str, extra: &str| {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.bytes().all(|b| {
                b.is_ascii_alphanumeric() || b"-".contains(&b) || extra.as_bytes().contains(&b)
            })
    };
    let named = repo
        .split_once('/')
        .is_some_and(|(o, n)| ok(o, "") && ok(n, "_."));
    if !named {
        return Err(refuse(text::repo_name(repo)));
    }
    let github = ServerClient::new(api, None);
    let get = |tail: &str| github.json::<Value, ()>("GET", &format!("/repos/{repo}{tail}"), None);
    match get("") {
        Err(ServerError::Status { status: 404, .. }) => {
            return Err(refuse(text::repo_missing(repo)));
        }
        Ok(v) if v["private"] == true => return Err(refuse(text::repo_missing(repo))),
        _ => {}
    }
    if get("/commits?per_page=1").is_ok_and(|v| v.as_array().is_some_and(|a| !a.is_empty())) {
        return Err(refuse(text::repo_not_empty(repo)));
    }
    Ok(format!("https://github.com/{repo}"))
}

/// Step 4: ask for the code, show it, and wait for the answer. Returns the run credential.
fn approve(base: &str, ask: &Ask, out: &mut dyn Write) -> Result<String, Stop> {
    let server = ServerClient::new(base, None);
    let wire = serde_json::to_value(ask.mode).unwrap_or_default();
    let body = json!({"purpose": "run", "mode": wire, "connector_id": ask.connector.id,
        "proxy_version": env!("CARGO_PKG_VERSION")});
    let made: Made = send(&server, "/v1/device", &body, false)
        .map_err(|e| server_stop(&e, text::approval_refused))?;
    let wait = made.expires_in.unwrap_or(MAX_WAIT).min(MAX_WAIT);
    let until = Instant::now() + Duration::from_secs(wait);
    let address = &made.verification_uri_complete;
    if !good_code(&made) || parse_server(address.split('?').next()).is_err() {
        return Err(broke(text::bad_answer()));
    }
    let mut say = |line: String| {
        let _ = writeln!(out, "{line}");
    };
    say(text::approval_intro(ask.mode, &ask.connector.name));
    say(text::approval_address(browser(address), address));
    say(text::approval_code(&made.user_code));
    say(text::approval_waiting(wait.div_ceil(60).max(1)));
    let mut every = made.interval.unwrap_or(5).clamp(INTERVAL.0, INTERVAL.1);
    loop {
        if interrupted() {
            return Err(stopped(text::WHY_INTERRUPTED));
        }
        let poll = json!({"device_code": made.device_code});
        match server.json::<Polled, Value>("POST", "/v1/device/token", Some(&poll)) {
            Ok(Polled {
                status,
                credential: Some(c),
            }) if status == "approved" => {
                let shaped = c
                    .strip_prefix("gcr_")
                    .is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()));
                if !shaped {
                    return Err(broke(text::bad_answer()));
                }
                say(text::APPROVED.to_owned());
                return Ok(c);
            }
            Ok(Polled { status, .. }) if status == "pending" => {}
            Ok(_) => return Err(broke(text::bad_answer())),
            Err(ServerError::Status { status: 403, .. }) => return Err(stopped(text::WHY_DENIED)),
            Err(ServerError::Status { status: 410, .. }) => return Err(stopped(text::WHY_EXPIRED)),
            Err(ServerError::Status { status: 429, .. }) => {
                every = (every + SLOW_DOWN).min(INTERVAL.1)
            }
            // The server may be away for a moment: keep asking until the code runs out.
            Err(ServerError::Transport(_)) => {}
            Err(ServerError::Status { status, .. }) if status >= 500 => {}
            Err(e) => return Err(server_stop(&e, text::approval_refused)),
        }
        let now = Instant::now();
        if now >= until {
            return Err(stopped(text::WHY_EXPIRED));
        }
        nap(Duration::from_secs(every).min(until - now));
    }
}

#[derive(Deserialize)]
struct Made {
    device_code: String,
    user_code: String,
    verification_uri_complete: String,
    interval: Option<u64>,
    expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct Polled {
    status: String,
    credential: Option<String>,
}

/// The codes the server made are the shape the site and this program expect, checked before any
/// of it is shown or sent.
fn good_code(m: &Made) -> bool {
    let user = m.user_code.as_bytes();
    let user_ok = user.len() == 9
        && user.iter().enumerate().all(|(i, b)| {
            if i == 4 {
                *b == b'-'
            } else {
                USER_CODE_ALPHABET.contains(b)
            }
        });
    let device_ok =
        m.device_code.len() == 64 && m.device_code.bytes().all(|b| b.is_ascii_hexdigit());
    let query_ok = !m
        .verification_uri_complete
        .contains(|c: char| c.is_control() || c.is_whitespace() || c == '#' || c == '@');
    user_ok && device_ok && query_ok
}

fn stopped(why: &str) -> Stop {
    refuse(text::approval_stopped(why))
}

/// A failed call to the server: nobody home or a broken answer ends exit 3, a refusal exit 1.
fn server_stop(e: &ServerError, refused: fn(&str) -> String) -> Stop {
    match e {
        ServerError::Status {
            status,
            message,
            code,
            ..
        } if *status < 500 => refuse(refused(if message.is_empty() { code } else { message })),
        _ => broke(format!(
            "{} {}",
            text::server_unreachable(e),
            text::NOTHING_CREATED
        )),
    }
}

/// Opens `url` in the browser unless `GOALCUP_NO_BROWSER=1`; whether a program started.
pub fn browser(url: &str) -> bool {
    if std::env::var("GOALCUP_NO_BROWSER").is_ok_and(|v| v == "1") {
        return false;
    }
    let tool = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(tool)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

/// Waits `d` in short steps, ending early on Ctrl-C.
fn nap(d: Duration) {
    let until = Instant::now() + d;
    while Instant::now() < until && !interrupted() {
        std::thread::sleep(Duration::from_millis(100).min(until - Instant::now()));
    }
}

static INT: AtomicBool = AtomicBool::new(false);

fn interrupted() -> bool {
    INT.load(Relaxed)
}

extern "C" fn on_int(_: libc::c_int) {
    INT.store(true, Relaxed);
}

/// Ctrl-C becomes a flag while this lives, and the old handler comes back after it.
struct Ctrl(libc::sighandler_t);

impl Ctrl {
    fn new() -> Ctrl {
        INT.store(false, Relaxed);
        // SAFETY: the handler only stores into an atomic, which is async-signal-safe.
        Ctrl(unsafe { libc::signal(libc::SIGINT, on_int as *const () as libc::sighandler_t) })
    }
}

impl Drop for Ctrl {
    fn drop(&mut self) {
        // SAFETY: puts back the handler `signal` returned.
        unsafe { libc::signal(libc::SIGINT, self.0) };
    }
}
