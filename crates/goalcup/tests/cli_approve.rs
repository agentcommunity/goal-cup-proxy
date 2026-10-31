//! `goalcup start` steps 3 to 5 (PRD-10 section 3, PRD-17 SC2) against the fake Worker: the
//! approval line, a denial, an expired code, an absent server and Ctrl-C each end with one line
//! and create nothing; the credential is 0600 and nowhere else; a non-empty repository is refused
//! before any approval request. Facts and the contract words only.

mod common;

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use common::ports;
use common::worker::{CREDENTIAL, FakeWorker, Inject, Route};
use goalcup::cli::approve::{Ask, Stop, begin};
use goalcup::cli::{ConnectorSource, RunMode, Server};
use goalcup::pipeline::Mono;
use serde_json::{Value, json};

/// Ctrl-C is process-wide, so the cases run one at a time.
static ONE: Mutex<()> = Mutex::new(());

const SHA: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn connector() -> ConnectorSource {
    ConnectorSource {
        id: "agent".into(),
        name: "Agent".into(),
        source: "builtin".into(),
    }
}

fn ask<'a>(c: &'a ConnectorSource, repo: Option<&'a str>, api: &'a str) -> Ask<'a> {
    Ask {
        mode: RunMode::Competition,
        connector: c,
        connector_sha256: SHA,
        workspace: Path::new("/work"),
        session_id: "s",
        harness_version: Some("1.2.3"),
        repo,
        github_api: api,
        hosts_file: None,
        machine: json!({"os": "Linux"}),
    }
}

/// Runs `begin` against `server`; the home must hold nothing if it stopped.
fn start(
    server: &Server,
    repo: Option<&str>,
    api: &str,
) -> (
    tempfile::TempDir,
    Result<goalcup::cli::approve::Opened, Stop>,
    String,
) {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let (c, mut out) = (connector(), Vec::new());
    let got = begin(home.path(), server, &ask(&c, repo, api), &mut out);
    (home, got, String::from_utf8(out).unwrap())
}

fn url(w: &FakeWorker) -> Server {
    Server::Url(w.url())
}

fn empty(dir: &Path) -> bool {
    std::fs::read_dir(dir).unwrap().next().is_none()
}

fn stops(w: &FakeWorker, route: Route, status: u16) -> Stop {
    w.inject(
        route,
        Inject {
            answer: Some((status, json!({"error": "x"}))),
            ..Inject::default()
        },
    );
    let (home, got, _) = start(&url(w), None, "");
    assert!(empty(home.path()), "a stopped start created something");
    got.unwrap_err()
}

#[test]
fn the_code_line_ends_with_the_contract_words_and_the_run_opens() {
    let w = FakeWorker::start();
    let (home, got, out) = start(&url(&w), None, "");
    let opened = got.unwrap();
    assert!(
        out.lines()
            .any(|l| l.ends_with("shows the code BCDF-GHJK.")),
        "{out}"
    );
    assert!(opened.run.starts_with('r') && opened.entry.starts_with('e'));
    // The deadline is the server's seconds_left on the boot clock (PRD-17 S7).
    let left = opened.deadline.unwrap().ns() - Mono::now().ns();
    assert!(
        (86_300 * 1_000_000_000..=86_400 * 1_000_000_000).contains(&left),
        "{left}"
    );
    assert!(
        home.path()
            .join("runs")
            .join(&opened.run)
            .join("server.json")
            .is_file()
    );
}

#[test]
fn the_credential_is_0600_and_nowhere_else() {
    let w = FakeWorker::start();
    let (home, got, out) = start(&url(&w), None, "");
    let opened = got.unwrap();
    let dir = home.path().join("runs").join(&opened.run);
    let file = dir.join("credential");
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), CREDENTIAL);
    assert_eq!(
        std::fs::metadata(dir.join("server.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let record = std::fs::read_to_string(dir.join("server.json")).unwrap();
    assert!(!record.contains(CREDENTIAL) && !out.contains(CREDENTIAL));
    assert!(!format!("{opened:?}").contains(CREDENTIAL));
    assert_eq!(
        serde_json::from_str::<Value>(&record).unwrap()["server"],
        w.url()
    );
}

#[test]
fn a_denial_an_expired_code_and_an_absent_server_create_nothing() {
    let w = FakeWorker::start();
    assert_eq!(stops(&w, Route::DeviceToken, 403).code, 1);
    assert_eq!(stops(&w, Route::DeviceToken, 410).code, 1);
    let (_held, addr) = ports::refusing();
    let (home, got, _) = start(&Server::Url(format!("http://{addr}")), None, "");
    assert_eq!(got.unwrap_err().code, 3);
    assert!(empty(home.path()));
}

#[test]
fn an_outage_or_429_after_approval_is_waited_out_and_the_run_opens() {
    let w = FakeWorker::start();
    for (status, retry_after) in [(502, None), (429, Some(1))] {
        w.inject(
            Route::Open,
            Inject {
                answer: Some((status, json!({"error": "upstream_error"}))),
                retry_after,
                ..Inject::default()
            },
        );
    }
    let (home, got, _) = start(&url(&w), None, "");
    let opened = got.unwrap();
    assert!(
        home.path()
            .join("runs")
            .join(&opened.run)
            .join("credential")
            .is_file()
    );
}

#[test]
fn ctrl_c_while_waiting_creates_nothing() {
    let w = FakeWorker::start();
    w.inject(
        Route::DeviceToken,
        Inject {
            answer: Some((202, json!({"status": "pending"}))),
            ..Inject::default()
        },
    );
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(300));
        // SAFETY: signals this test process, whose handler `begin` has installed.
        unsafe { libc::kill(libc::getpid(), libc::SIGINT) };
    });
    let (home, got, _) = start(&url(&w), None, "");
    assert_eq!(got.unwrap_err().code, 1);
    assert!(empty(home.path()));
}

#[test]
fn a_repository_with_commits_is_refused_before_any_approval() {
    // GitHub's fake: public, and a first commit. The server is a closed port: if the approval were
    // asked first the stop would be exit 3.
    let (listener, addr) = ports::held();
    std::thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let mut buf = [0u8; 2048];
            let n = s.read(&mut buf).unwrap_or(0);
            let body = if String::from_utf8_lossy(&buf[..n]).contains("/commits") {
                "[{}]"
            } else {
                r#"{"private":false}"#
            };
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    let (_held, closed) = ports::refusing();
    let (home, got, out) = start(
        &Server::Url(format!("http://{closed}")),
        Some("o/r"),
        &format!("http://{addr}"),
    );
    assert_eq!(got.unwrap_err().code, 1);
    assert!(out.is_empty() && empty(home.path()));
}

#[test]
fn a_local_only_run_asks_for_nothing() {
    let (home, got, out) = start(&Server::Off, Some("o/r"), "");
    let opened = got.unwrap();
    assert!(out.is_empty() && opened.deadline.is_none() && opened.token.is_none());
    let dir = home.path().join("runs").join(&opened.run);
    assert!(dir.join("server.json").is_file() && !dir.join("credential").exists());
}
