//! `goalcup prep`: six steps from install to a practice run, re-runnable, and never starting a
//! run.
//!
//! The binary is run against a temporary configuration directory and the chat-mode development
//! connector (`Rig::chat`, whose harness is `fixtures/m2-fake-tui.sh`). The machine's facts
//! (mains power, free space, hosts, the sleep lock) are stated through the test door
//! `GOALCUP_TEST_PREP` rather than read from the machine the test runs on, GitHub is a local
//! fake, and a run on a terminal is a pseudo-terminal of the test's own.

use crate::common::m2_cli_support::{CHAT_ID, OnPty, Rig};
use crate::common::upload_server::{ServerOptions, UploadServer};
use goalcup_cli::account::github::GithubSetup;
use goalcup_cli::prep_file::{PrepFile, prep_path, read_prep_lenient};
use goalcup_cli::run::{EntryRef, RunRecord};
use goalcup_protocol::control::DaemonClient;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const REPO: &str = "https://github.com/acme/empty-repo";
const GIB: u64 = 1 << 30;

/// A made-up token: the shape of a fine-grained one, and nothing GitHub would accept.
fn token() -> String {
    format!("github_pat_{}", "A".repeat(60))
}

// ------------------------------------------------------------------ the fake GitHub API

struct FakeGithub {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    port: u16,
}

impl FakeGithub {
    /// Answers every request as a repository that is public, empty and pushable.
    fn good() -> FakeGithub {
        FakeGithub::start(
            r#"{"private":false,"size":0,"permissions":{"admin":false,"push":true,"pull":true}}"#,
        )
    }

    fn start(body: &'static str) -> FakeGithub {
        FakeGithub::serve(move |_| (200, body))
    }

    /// Answers each request as `answer` says for the bearer token it carried.
    fn serve(answer: impl Fn(&str) -> (u16, &'static str) + Send + 'static) -> FakeGithub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (log, halt) = (requests.clone(), stop.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if halt.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = stream else { continue };
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && head.len() < 16 * 1024 {
                    match stream.read(&mut byte) {
                        Ok(1) => head.push(byte[0]),
                        _ => break,
                    }
                }
                let head = String::from_utf8_lossy(&head).into_owned();
                let path = head
                    .lines()
                    .next()
                    .and_then(|l| l.split(' ').nth(1))
                    .unwrap_or("")
                    .to_string();
                log.lock().unwrap().push(path);
                let bearer = head
                    .lines()
                    .filter_map(|l| l.split_once(':'))
                    .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
                    .map(|(_, v)| v.trim().trim_start_matches("Bearer ").to_string())
                    .unwrap_or_default();
                let (status, body) = answer(&bearer);
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
                let _ = stream.shutdown(Shutdown::Both);
            }
        });
        FakeGithub {
            base: format!("http://127.0.0.1:{port}"),
            requests,
            stop,
            port,
        }
    }

    fn asked(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl Drop for FakeGithub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

// ------------------------------------------------------------------ the binary

/// The facts of a machine that is fine, plus anything the test changes (a later key wins).
fn facts(github: &str, more: &[&str]) -> String {
    let mut parts = vec![
        "ac=on".to_string(),
        format!("free={}", 100 * GIB),
        "hosts=ok".to_string(),
        "awake=ok".to_string(),
        format!("github={github}"),
    ];
    parts.extend(more.iter().map(|s| s.to_string()));
    // The parser takes the last of a repeated key.
    parts.join(",")
}

/// `goalcup prep --dev <args>` in `rig`'s world, with the machine as `facts` says.
fn prep_command(rig: &Rig, args: &[&str], facts: &str) -> Command {
    let home = rig.root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_goalcup"));
    command
        .arg("prep")
        .arg("--dev")
        .args(args)
        .current_dir(&rig.root)
        .env_clear()
        .env("PATH", rig.path())
        .env("HOME", &home)
        .env("GOALCUP_CONFIG_DIR", &rig.cfg)
        .env("GOALCUP_CONNECTORS_DIR", &rig.connectors)
        .env("GOALCUP_TEST_NO_BROWSER", "1")
        .env("GOALCUP_TEST_PREP", facts)
        .env("TERM", "dumb");
    command
}

/// Run it with no terminal: `(exit code, stdout)`.
fn prep(rig: &Rig, args: &[&str], facts: &str) -> (i32, String) {
    let out = prep_command(rig, args, facts)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
    )
}

/// The report line of `step`, which starts `[state] step:`.
fn line<'a>(text: &'a str, step: &str) -> &'a str {
    text.lines()
        .find(|l| l.split(' ').nth(1) == Some(format!("{step}:").as_str()))
        .unwrap_or_else(|| panic!("no line for {step} in:\n{text}"))
}

fn json(text: &str) -> Vec<serde_json::Value> {
    serde_json::from_str::<Vec<serde_json::Value>>(text)
        .unwrap_or_else(|e| panic!("not a JSON array ({e}):\n{text}"))
}

fn log_in(rig: &Rig, url: &str) {
    goalcup_cli::server::write_server_config(
        &rig.cfg,
        &goalcup_cli::server::ServerConfig {
            url: url.to_string(),
            account: Some("member-1".to_string()),
        },
    )
    .unwrap();
}

/// A practice run of the chat connector with `entries`, each `(entry id, sent)`.
fn practice_run(rig: &Rig, run_id: &str, entries: &[(&str, bool)]) {
    let record = RunRecord {
        run_id: run_id.to_string(),
        server_url: Some("http://127.0.0.1:1".to_string()),
        practice: true,
        connector_id: CHAT_ID.to_string(),
        workspace: rig.root.join("work").join(run_id),
        started_at_unix: 1_700_000_000,
        deadline_at: Some("2026-10-14T10:10:00Z".to_string()),
        entries: entries
            .iter()
            .map(|(id, _)| EntryRef {
                entry_id: id.to_string(),
                trace_dir: rig.root.join("trace").join(id),
            })
            .collect(),
    };
    goalcup_cli::run::write(&rig.cfg, &record).unwrap();
    let uploads = rig.cfg.join(goalcup_cli::upload::UPLOADS_DIR_NAME);
    std::fs::create_dir_all(&uploads).unwrap();
    for (id, sent) in entries {
        if *sent {
            std::fs::write(uploads.join(format!("{id}.sent")), "").unwrap();
        }
    }
}

fn prep_file(cfg: &Path) -> PrepFile {
    read_prep_lenient(cfg)
}

// ------------------------------------------------------------------ the tests

#[test]
fn prep_reports_each_step_and_exits_1_until_all_are_ok() {
    let rig = Rig::chat();
    let github = FakeGithub::good();
    let facts = facts(&github.base, &[]);

    // A machine that has done nothing: the harness is there, the machine is fine, the rest wait.
    let (code, text) = prep(&rig, &[], &facts);
    assert_eq!(code, 1, "{text}");
    assert_eq!(line(&text, "login"), "[todo] login: run `goalcup login`");
    assert!(
        line(&text, "harness").starts_with("[ok] harness: m2-fake-tui"),
        "{text}"
    );
    assert!(
        line(&text, "harness_home").starts_with("[todo] harness_home:"),
        "{text}"
    );
    assert!(
        line(&text, "github").starts_with("[todo] github:"),
        "{text}"
    );
    assert!(
        line(&text, "machine").starts_with("[ok] machine:"),
        "{text}"
    );
    let practice = line(&text, "practice");
    assert!(
        practice.starts_with("[todo] practice:")
            && practice.contains("goalcup start --harness m2-fake-tui --practice"),
        "{text}"
    );
    // The six steps are printed in order.
    let order: Vec<&str> = text
        .lines()
        .filter_map(|l| l.split(' ').nth(1))
        .filter(|w| w.ends_with(':') && *w != "prep:")
        .collect();
    assert_eq!(
        order,
        [
            "login:",
            "harness:",
            "harness_home:",
            "github:",
            "machine:",
            "practice:"
        ],
        "{text}"
    );

    // The person logs in, opens the harness once, gives the repository and its token, and a
    // practice run is uploaded; each of those is what a later prep finds, one at a time.
    log_in(&rig, "http://127.0.0.1:1");
    let (code, text) = prep(&rig, &["--repo", REPO], &facts);
    assert_eq!(code, 1, "{text}");
    assert!(line(&text, "login").starts_with("[ok] login:"), "{text}");
    assert!(
        line(&text, "github").starts_with("[todo] github:"),
        "{text}"
    );

    GithubSetup::new(REPO, &token())
        .unwrap()
        .store(&rig.cfg)
        .unwrap();
    let (code, text) = prep(&rig, &[], &facts);
    assert_eq!(code, 1, "{text}");
    assert!(line(&text, "github").starts_with("[ok] github:"), "{text}");
    assert!(line(&text, "harness_home").starts_with("[todo]"), "{text}");

    let mut file = prep_file(&rig.cfg);
    assert_eq!(file.repo_url, REPO);
    assert_eq!(file.harness, CHAT_ID);
    file.home_ready = true;
    // The person's word counts only while the home they set up is there.
    let (_, text) = {
        goalcup_cli::prep_file::write_prep(&rig.cfg, &file).unwrap();
        prep(&rig, &[], &facts)
    };
    assert!(line(&text, "harness_home").starts_with("[todo]"), "{text}");
    std::fs::create_dir_all(rig.chat_home()).unwrap();
    goalcup_cli::prep_file::write_prep(&rig.cfg, &file).unwrap();
    let (code, text) = prep(&rig, &[], &facts);
    assert_eq!(code, 1, "{text}");
    assert!(line(&text, "harness_home").starts_with("[ok]"), "{text}");
    assert!(line(&text, "practice").starts_with("[todo]"), "{text}");

    // A practice run that is not all uploaded does not count, and says which entry to send.
    practice_run(
        &rig,
        "run-practice-1",
        &[("entry-one", true), ("entry-two", false)],
    );
    let (code, text) = prep(&rig, &[], &facts);
    assert_eq!(code, 1, "{text}");
    let practice = line(&text, "practice");
    assert!(
        practice.starts_with("[todo]") && practice.contains("goalcup upload entry-two"),
        "{text}"
    );

    practice_run(
        &rig,
        "run-practice-1",
        &[("entry-one", true), ("entry-two", true)],
    );
    let (code, text) = prep(&rig, &[], &facts);
    assert_eq!(code, 0, "{text}");
    for step in [
        "login",
        "harness",
        "harness_home",
        "github",
        "machine",
        "practice",
    ] {
        assert!(line(&text, step).starts_with("[ok]"), "{step}: {text}");
    }
    assert!(
        text.contains("goalcup start --harness m2-fake-tui"),
        "{text}"
    );
    // And what was settled is what a run on a server reads.
    let kept = goalcup_cli::prep_file::read_prep(&rig.cfg).unwrap();
    assert_eq!(kept.repo_url, REPO);
    assert_eq!(kept.harness, CHAT_ID);
    assert!(
        Path::new(&kept.work_root).is_absolute() && kept.work_root.ends_with("home/goalcup-work"),
        "{kept:?}"
    );
}

#[test]
fn a_second_prep_skips_passed_steps() {
    let rig = Rig::chat();
    let github = FakeGithub::good();
    let facts = facts(&github.base, &[]);
    log_in(&rig, "http://127.0.0.1:1");

    // The first prep, on a terminal: it opens the harness and asks for the token.
    let mut first = OnPty::spawn(prep_command(&rig, &["--repo", REPO], &facts), None);
    first.wait_shown("tui: ready");
    first.type_line("/exit");
    first.wait_shown("GitHub token for");
    let secret = token();
    first.type_line(&secret);
    let status = first.wait();
    let shown = first.shown();
    assert_eq!(
        status.code(),
        Some(1),
        "the practice step is still to do:\n{shown}"
    );
    assert!(
        !shown.contains(&secret),
        "the token the person typed was shown on the terminal:\n{shown}"
    );
    assert!(shown.contains("[ok] harness_home:"), "{shown}");
    assert!(shown.contains("[ok] github:"), "{shown}");
    // The token the person gave passed the whole check in one request.
    assert_eq!(github.asked(), 1);
    // The harness ran with no goal, on the terminal, in the home a run uses, with no proxy.
    assert_eq!(
        rig.harness_file("tui.stdin").as_deref().map(str::trim),
        Some("tty")
    );
    assert!(
        rig.harness_file("tui.argv").unwrap().contains("chat"),
        "{:?}",
        rig.harness_file("tui.argv")
    );
    assert!(rig.chat_home().is_dir(), "the harness home was made");
    let file = prep_file(&rig.cfg);
    assert!(file.home_ready, "{file:?}");
    assert_eq!(file.repo_url, REPO);
    // The token is stored where git's helper reads it, and nowhere the report shows.
    assert!(GithubSetup::load(&rig.cfg).is_ok());

    // The second prep, on a terminal again, repeats neither: the harness is not opened, the
    // token is not asked for, and GitHub is not asked again.
    std::fs::remove_file(rig.out.join("tui.pid")).unwrap();
    std::fs::remove_file(rig.out.join("tui.ready")).unwrap();
    let mut second = OnPty::spawn(prep_command(&rig, &[], &facts), None);
    let status = second.wait();
    let shown = second.shown();
    assert_eq!(status.code(), Some(1), "{shown}");
    assert!(shown.contains("[ok] harness_home:"), "{shown}");
    assert!(shown.contains("[ok] github:"), "{shown}");
    assert!(
        !shown.contains("tui: ready"),
        "the harness was opened again:\n{shown}"
    );
    assert!(!shown.contains("GitHub token for"), "{shown}");
    assert!(
        !rig.out.join("tui.pid").exists(),
        "the harness started again"
    );
    // GitHub is asked again, for all three (public and pushable, then empty), and no token is
    // wanted because the stored one passes.
    assert_eq!(github.asked(), 3, "the repository is checked on every prep");
}

#[test]
fn prep_fails_a_wsl_work_root_on_a_windows_host_filesystem() {
    let rig = Rig::chat();
    let github = FakeGithub::good();
    // The filesystem is judged by its type, not by where it is mounted: a Windows drive mounted
    // at an odd place is refused, and a Linux mount under `/mnt/c` is not.
    let wsl = facts(
        &github.base,
        &["wsl=2", "fs=/odd/place=9p", "fs=/mnt/c/linux=ext4", "fs=/mnt/c=drvfs"],
    );
    let on_drive = "/odd/place/goalcup-work";

    let (code, text) = prep(&rig, &["--work-root", on_drive], &wsl);
    assert_eq!(code, 1, "{text}");
    let machine = line(&text, "machine");
    assert!(
        machine.starts_with("[fail] machine:")
            && machine.contains("work folder /odd/place/goalcup-work")
            && machine.contains("Windows-host filesystem (9p)"),
        "{text}"
    );
    // A refused folder is not kept for `goalcup start` to find.
    assert_ne!(prep_file(&rig.cfg).work_root, on_drive);

    let (_, text) = prep(&rig, &["--work-root", "/mnt/c/Users/me/work"], &wsl);
    assert!(line(&text, "machine").contains("(drvfs)"), "{text}");
    let (_, text) = prep(&rig, &["--work-root", "/mnt/c/linux/work"], &wsl);
    assert!(!line(&text, "machine").contains("Windows-host"), "{text}");

    // The Linux home is fine on WSL2; the same folder is fine on a machine that is not WSL.
    let (_, text) = prep(&rig, &["--work-root", "/home/me/goalcup-work"], &wsl);
    assert!(!line(&text, "machine").contains("Windows-host"), "{text}");
    let (_, text) = prep(
        &rig,
        &["--work-root", on_drive],
        &facts(&github.base, &["wsl=0", "fs=/odd/place=9p"]),
    );
    assert!(line(&text, "machine").starts_with("[ok]"), "{text}");

    // On WSL2 the machine is not asked to be kept awake: Windows decides, and prep says so.
    let (_, text) = prep(
        &rig,
        &["--work-root", "/home/me/goalcup-work"],
        &facts(&github.base, &["wsl=2", "awake=no_inhibitor"]),
    );
    let machine = line(&text, "machine");
    assert!(
        machine.starts_with("[todo] machine:") && machine.contains("Windows decides"),
        "{text}"
    );
}

#[test]
fn prep_fails_wsl_artifact_root_and_config_folder_on_a_windows_host_filesystem() {
    let rig = Rig::chat();
    let github = FakeGithub::good();
    let work = "/home/me/goalcup-work";
    let artifacts = format!("{}-artifacts", rig.cfg.display());

    let on_artifacts = facts(&github.base, &["wsl=2", &format!("fs={artifacts}=9p")]);
    let (code, text) = prep(&rig, &["--work-root", work], &on_artifacts);
    assert_eq!(code, 1, "{text}");
    let machine = line(&text, "machine");
    assert!(
        machine.starts_with("[fail] machine:")
            && machine.contains(&format!("artifact root {artifacts}"))
            && !machine.contains("config folder"),
        "{text}"
    );
    // The work folder is fine, and is kept.
    assert_eq!(prep_file(&rig.cfg).work_root, work);

    let on_config = facts(
        &github.base,
        &["wsl=2", &format!("fs={}=virtiofs", rig.cfg.display())],
    );
    let (_, text) = prep(&rig, &["--work-root", work], &on_config);
    let machine = line(&text, "machine");
    assert!(
        machine.contains("config folder") && machine.contains("(virtiofs)"),
        "{text}"
    );
}

#[test]
fn prep_fails_wsl1_in_one_sentence() {
    let rig = Rig::chat();
    let github = FakeGithub::good();
    let (code, text) = prep(
        &rig,
        &["--work-root", "/home/me/goalcup-work"],
        &facts(&github.base, &["wsl=1"]),
    );
    assert_eq!(code, 1, "{text}");
    let machine = line(&text, "machine");
    assert!(
        machine.starts_with("[fail] machine:")
            && machine.contains("WSL1 cannot run Goal Cup: convert this distribution to WSL2 with")
            && machine.contains("`wsl --set-version <distro> 2`"),
        "{text}"
    );
    assert_ne!(
        prep_file(&rig.cfg).work_root,
        "/home/me/goalcup-work",
        "a work folder is not kept on WSL1"
    );
}

#[test]
fn prep_does_not_start_a_run() {
    let rig = Rig::chat();
    let server = UploadServer::start(&rig.root.join("server"), ServerOptions::default());
    let github = FakeGithub::good();
    log_in(&rig, &server.url());
    GithubSetup::new(REPO, &token())
        .unwrap()
        .store(&rig.cfg)
        .unwrap();
    let facts = facts(&github.base, &[]);

    // With no terminal, with --json, and on a terminal.
    let (_, text) = prep(&rig, &["--repo", REPO], &facts);
    assert!(line(&text, "login").starts_with("[ok]"), "{text}");
    let (_, text) = prep(&rig, &["--json"], &facts);
    json(&text);
    let mut on_pty = OnPty::spawn(prep_command(&rig, &[], &facts), None);
    on_pty.wait_shown("tui: ready");
    on_pty.type_line("/exit");
    on_pty.wait();

    // Nothing of a run exists: no run directory or record, no entry in the daemon, no entry
    // file, no run opened on the server, no credential.
    assert!(!goalcup_cli::run::runs_dir(&rig.cfg).exists());
    assert!(!goalcup_cli::entries::entries_dir(&rig.cfg).exists());
    assert_eq!(rig.client().health().unwrap().entries_open, 0);
    assert!(rig.run_dirs().is_empty());
    assert!(rig.clones().is_empty());
    let calls = server.calls();
    assert!(
        !calls.iter().any(|c| c.starts_with("POST /v1/runs")),
        "prep opened a run: {calls:?}"
    );
    assert!(server.run_ids().is_empty());
    // The scratch home it opened the harness in is gone.
    let leftover = std::fs::read_dir(rig.cfg.join("prep-launch"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(leftover, 0);
}

#[test]
fn prep_json_marks_the_human_steps() {
    let rig = Rig::chat();
    let github = FakeGithub::good();
    let facts = facts(&github.base, &[]);

    let (code, text) = prep(&rig, &["--json"], &facts);
    assert_eq!(code, 1);
    let steps = json(&text);
    let ids: Vec<&str> = steps.iter().map(|s| s["step"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        [
            "login",
            "harness",
            "harness_home",
            "github",
            "machine",
            "practice"
        ]
    );
    for s in &steps {
        let mut keys: Vec<&str> = s.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["human", "say", "state", "step"], "{s}");
        assert!(s["say"].as_str().is_some_and(|say| !say.is_empty()), "{s}");
    }
    let human: Vec<(&str, bool)> = steps
        .iter()
        .map(|s| (s["step"].as_str().unwrap(), s["human"].as_bool().unwrap()))
        .collect();
    assert_eq!(
        human,
        [
            ("login", true),
            ("harness", false),
            ("harness_home", true),
            ("github", true),
            ("machine", false),
            ("practice", true),
        ]
    );
    let states: Vec<&str> = steps.iter().map(|s| s["state"].as_str().unwrap()).collect();
    assert_eq!(states, ["todo", "ok", "todo", "todo", "ok", "todo"]);
    // Nothing but the array is printed.
    assert!(text.trim_start().starts_with('[') && text.trim_end().ends_with(']'));

    // A step that has passed needs nobody.
    log_in(&rig, "http://127.0.0.1:1");
    let (_, text) = prep(&rig, &["--json"], &facts);
    let steps = json(&text);
    assert_eq!(steps[0]["state"], "ok");
    assert_eq!(steps[0]["human"], false);

    // On a terminal, --json still asks nothing and opens nothing.
    let mut on_pty = OnPty::spawn(
        prep_command(&rig, &["--json", "--repo", REPO], &facts),
        None,
    );
    let status = on_pty.wait();
    let shown = on_pty.shown();
    assert_eq!(status.code(), Some(1), "{shown}");
    assert!(
        !shown.contains("not shown):") && !shown.contains("tui: ready"),
        "{shown}"
    );
    assert!(
        !rig.out.join("tui.pid").exists(),
        "--json launched the harness"
    );
    let steps = json(shown.trim());
    assert_eq!(steps[3]["state"], "todo");
    assert_eq!(steps[3]["human"], true);
}

#[test]
fn prep_refuses_what_the_flags_get_wrong_before_it_writes_anything() {
    let rig = Rig::chat();
    let github = FakeGithub::good();
    let facts = facts(&github.base, &[]);
    for args in [
        &["--work-root", "relative/goalcup-work"][..],
        &["--repo", "https://example.com/acme/empty-repo"][..],
        &["--repo", "acme/empty-repo"][..],
    ] {
        let out = prep_command(&rig, args, &facts)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        assert!(!prep_path(&rig.cfg).exists(), "{args:?} wrote a prep file");
    }
}

#[test]
fn the_machine_step_judges_power_space_git_and_the_network() {
    let rig = Rig::chat();
    let github = FakeGithub::good();
    let machine = |more: &[&str]| {
        let (code, text) = prep(&rig, &[], &facts(&github.base, more));
        assert_eq!(code, 1, "{text}");
        line(&text, "machine").to_string()
    };
    assert!(machine(&[]).starts_with("[ok]"));
    assert!(machine(&["ac=battery"]).starts_with("[todo]"));
    // Below 4 GiB a run cannot start; between that and 8 GiB it starts with a warning.
    assert!(machine(&[&format!("free={}", 3 * GIB)]).starts_with("[fail]"));
    assert!(machine(&[&format!("free={}", 6 * GIB)]).starts_with("[todo]"));
    assert!(machine(&[&format!("free={}", 8 * GIB)]).starts_with("[ok]"));
    let unreachable = machine(&["hosts=ingest.example.org:443_could_not_be_reached"]);
    assert!(
        unreachable.starts_with("[fail]") && unreachable.contains("ingest.example.org"),
        "{unreachable}"
    );
    assert!(machine(&["awake=no_inhibitor"]).starts_with("[todo]"));

    // No git on PATH: the harness is still found (the rig's directory is on it), git is not.
    let out = prep_command(&rig, &[], &facts(&github.base, &[]))
        .env("PATH", rig.root.join("bin"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let machine = line(&text, "machine");
    assert!(
        machine.starts_with("[fail]") && machine.contains("git is not on PATH"),
        "{text}"
    );
}

#[test]
fn the_harness_step_names_what_it_cannot_use() {
    let rig = Rig::chat();
    let github = FakeGithub::good();
    let facts = facts(&github.base, &[]);

    let (_, text) = prep(&rig, &["--harness", "no-such-harness"], &facts);
    let harness = line(&text, "harness");
    assert!(
        harness.starts_with("[fail]") && harness.contains(CHAT_ID),
        "{text}"
    );
    // The headless connector of the same rig is no chat harness.
    let (_, text) = prep(&rig, &["--harness", "m2-fake"], &facts);
    assert!(line(&text, "harness").starts_with("[fail]"), "{text}");
    // A harness that is not installed is a failure that says so.
    let out = prep_command(&rig, &[], &facts)
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(line(&text, "harness").starts_with("[fail]"), "{text}");
    // And the choice is kept for the next prep.
    let (_, _) = prep(&rig, &["--harness", CHAT_ID], &facts);
    assert_eq!(prep_file(&rig.cfg).harness, CHAT_ID);
}

/// The repository answers as public, pushable and empty (`size` 0) or not, for the one token
/// `good` and with a refusal for any other.
fn github_that_only_takes(good: &'static str, size: u32) -> FakeGithub {
    FakeGithub::serve(move |bearer| {
        if bearer == good {
            (
                200,
                Box::leak(
                    format!(
                        r#"{{"private":false,"size":{size},"permissions":{{"admin":false,"push":true,"pull":true}}}}"#
                    )
                    .into_boxed_str(),
                ),
            )
        } else {
            (401, r#"{"message":"Bad credentials"}"#)
        }
    })
}

#[test]
fn a_revoked_token_is_found_on_the_next_prep_and_replaced_and_replaced() {
    let rig = Rig::chat();
    let new_token = format!("github_pat_{}", "B".repeat(60));
    let new_token: &'static str = Box::leak(new_token.into_boxed_str());
    let github = github_that_only_takes(new_token, 0);
    let facts = facts(&github.base, &[]);
    log_in(&rig, "http://127.0.0.1:1");
    // Everything settled earlier, and the token stored since has been revoked.
    GithubSetup::new(REPO, &token())
        .unwrap()
        .store(&rig.cfg)
        .unwrap();
    std::fs::create_dir_all(rig.chat_home()).unwrap();
    goalcup_cli::prep_file::write_prep(
        &rig.cfg,
        &PrepFile {
            harness: CHAT_ID.to_string(),
            repo_url: REPO.to_string(),
            work_root: rig.root.join("work").display().to_string(),
            home_ready: true,
        },
    )
    .unwrap();

    // Without a terminal it is a failure that says what GitHub said, not an ok.
    let (code, text) = prep(&rig, &[], &facts);
    assert_eq!(code, 1, "{text}");
    let github_line = line(&text, "github");
    assert!(
        github_line.starts_with("[fail] github:") && github_line.contains("refused the token"),
        "{text}"
    );

    // On a terminal the person is asked for a new token, which passes the whole check and
    // replaces the old one.
    let mut on_pty = OnPty::spawn(prep_command(&rig, &[], &facts), None);
    on_pty.wait_shown("not shown):");
    on_pty.type_line(new_token);
    on_pty.wait();
    let shown = on_pty.shown();
    assert!(shown.contains("[ok] github:"), "{shown}");
    assert!(!shown.contains(new_token), "the token was shown:\n{shown}");
    assert_eq!(
        GithubSetup::load(&rig.cfg).unwrap(),
        GithubSetup::new(REPO, new_token).unwrap()
    );

    // The next prep is not asked for anything: the stored token passes.
    let (_, text) = prep(&rig, &[], &facts);
    assert!(line(&text, "github").starts_with("[ok] github:"), "{text}");
    let mut again = OnPty::spawn(prep_command(&rig, &[], &facts), None);
    again.wait();
    assert!(!again.shown().contains("not shown):"), "{}", again.shown());
}

#[test]
fn a_repository_that_is_not_empty_is_a_failure_on_every_prep_and_no_reason_for_a_new_token() {
    let rig = Rig::chat();
    let token_in_use: &'static str = Box::leak(token().into_boxed_str());
    let github = github_that_only_takes(token_in_use, 12);
    log_in(&rig, "http://127.0.0.1:1");
    GithubSetup::new(REPO, token_in_use)
        .unwrap()
        .store(&rig.cfg)
        .unwrap();
    let facts = facts(&github.base, &[]);
    std::fs::create_dir_all(rig.chat_home()).unwrap();
    goalcup_cli::prep_file::write_prep(
        &rig.cfg,
        &PrepFile {
            harness: CHAT_ID.to_string(),
            home_ready: true,
            ..PrepFile::default()
        },
    )
    .unwrap();
    // Every prep says so, with the sentence of the repository check, however often it is run.
    for _ in 0..2 {
        let (code, text) = prep(&rig, &["--repo", REPO], &facts);
        assert_eq!(code, 1, "{text}");
        let github_line = line(&text, "github");
        assert!(
            github_line.starts_with("[fail] github:")
                && github_line.contains("is not empty (12 KB)")
                && github_line.contains("no README, license or .gitignore"),
            "{text}"
        );
    }
    // On a terminal too: the token is fine, so none is asked for.
    let mut on_pty = OnPty::spawn(prep_command(&rig, &[], &facts), None);
    on_pty.wait_shown("[fail] github:");
    on_pty.wait();
    let shown = on_pty.shown();
    assert!(shown.contains("is not empty"), "{shown}");
    assert!(!shown.contains("not shown):"), "{shown}");
}
