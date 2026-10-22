//! `goalcup prep`: six steps from install to a practice run, re-runnable, and never starting a
//! run.
//!
//! The binary is run against a temporary configuration directory and the chat-mode development
//! connector (`Rig::chat_only`, whose harness is `fixtures/m2-fake-tui.sh`). The machine's facts
//! (mains power, free space, hosts, the sleep lock) are stated through the test door
//! `GOALCUP_TEST_PREP` rather than read from the machine the test runs on, GitHub is a local
//! fake, and a run on a terminal is a pseudo-terminal of the test's own.

use crate::common::m2_cli_support::{
    CHAT_CONFIGURE_ID, CHAT_HOME_ID, CHAT_ID, Launcher, OnPty, Rig,
};
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
        // Every folder on ext4 unless the test says otherwise, whatever this machine's are.
        "fs=/=ext4".to_string(),
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
    kept_run(rig, run_id, entries, true, false);
}

/// A kept run of the chat connector: practice, test or (neither) competition.
fn kept_run(rig: &Rig, run_id: &str, entries: &[(&str, bool)], practice: bool, test: bool) {
    let record = RunRecord {
        run_id: run_id.to_string(),
        server_url: Some("http://127.0.0.1:1".to_string()),
        practice,
        test,
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

/// Answer the pause before the harness opens: prep asks for Enter, then takes the terminal.
fn press_enter(on_pty: &OnPty) {
    on_pty.wait_shown("Press Enter to open");
    on_pty.type_line("");
}

fn prep_file(cfg: &Path) -> PrepFile {
    read_prep_lenient(cfg)
}

// ------------------------------------------------------------------ the tests

#[test]
fn prep_reports_each_step_and_exits_1_until_all_are_ok() {
    let rig = Rig::chat_only();
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
    // The last line points at the next action: the first step that needs the person. What to
    // do is said once, on that step's own line.
    let last = text.lines().last().unwrap();
    assert_eq!(
        last, "goalcup prep: 4 of 6 steps need you. Next: the login step above.",
        "{text}"
    );
    let login_say = line(&text, "login")
        .split_once(": ")
        .map(|(_, say)| say)
        .unwrap();
    assert_eq!(text.matches(login_say).count(), 1, "{text}");
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

    // A fully uploaded organizer test run (and a competition run) is not the practice run.
    kept_run(&rig, "run-test-1", &[("entry-test", true)], false, true);
    kept_run(&rig, "run-comp-1", &[("entry-comp", true)], false, false);
    let (code, text) = prep(&rig, &[], &facts);
    assert_eq!(code, 1, "{text}");
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
    let last = text.lines().last().unwrap();
    assert!(
        last.starts_with("goalcup prep: ready.")
            && last.contains("goalcup start --harness m2-fake-tui"),
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
    let rig = Rig::chat_only();
    let github = FakeGithub::good();
    let facts = facts(&github.base, &[]);
    log_in(&rig, "http://127.0.0.1:1");

    // The first prep, on a terminal: it opens the harness and asks for the token.
    let mut first = OnPty::spawn(prep_command(&rig, &["--repo", REPO], &facts), None);
    press_enter(&first);
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
fn prep_waits_for_enter_before_it_opens_the_harness() {
    let rig = Rig::chat_only();
    let github = FakeGithub::good();
    let facts = facts(&github.base, &[]);
    log_in(&rig, "http://127.0.0.1:1");

    let mut on_pty = OnPty::spawn(prep_command(&rig, &["--repo", REPO], &facts), None);
    on_pty.wait_shown("Press Enter to open");
    // The pause is real: the harness has not started, and nothing is recorded as set up.
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(
        !rig.out.join("tui.pid").exists(),
        "the harness opened before Enter:\n{}",
        on_pty.shown()
    );
    assert!(!prep_file(&rig.cfg).home_ready);
    on_pty.type_line("");
    on_pty.wait_shown("tui: ready");
    on_pty.type_line("/exit");
    on_pty.wait_shown("GitHub token for");
    on_pty.type_line(&token());
    on_pty.wait();
    assert!(prep_file(&rig.cfg).home_ready);
}

#[test]
fn prep_does_not_open_the_harness_when_the_input_ends_at_the_pause() {
    let rig = Rig::chat_only();
    let github = FakeGithub::good();
    let facts = facts(&github.base, &[]);
    log_in(&rig, "http://127.0.0.1:1");

    let mut on_pty = OnPty::spawn(prep_command(&rig, &["--repo", REPO], &facts), None);
    on_pty.wait_shown("Press Enter to open");
    // Ctrl-D at the start of a line: the input ends with no Enter. Only that one byte is
    // written. An Enter after it would sit in the terminal's queue, and the token prompt that
    // follows would read it as an empty token whenever it arrived after the prompt cleared the
    // queue.
    on_pty.type_bytes("\u{4}");
    on_pty.wait_shown("GitHub token for");
    on_pty.type_line(&token());
    on_pty.wait();
    let shown = on_pty.shown();
    // The terminal echoes the typed Ctrl-D (`^D`, then backspaces on macOS) and no newline.
    // Prep ends that line, so the report line is a line of its own, word for word.
    let lines: Vec<&str> = shown.lines().map(|l| l.trim_end_matches('\r')).collect();
    let at = lines
        .iter()
        .position(|l| l.contains("harness_home:"))
        .unwrap_or_else(|| panic!("no harness_home line in:\n{shown}"));
    assert_eq!(
        lines[at],
        format!(
            "[todo] harness_home: {CHAT_ID} was not opened. Run `goalcup prep` again, then \
             press Enter"
        ),
        "{shown}"
    );
    // Only the echo of the Ctrl-D is on the line between the question and the report line.
    assert!(
        at >= 2
            && lines[at - 1]
                .chars()
                .all(|c| matches!(c, '^' | 'D' | '\u{8}')),
        "{shown}"
    );
    assert!(lines[at - 2].starts_with("Press Enter to open"), "{shown}");
    assert!(
        !rig.out.join("tui.pid").exists(),
        "the harness opened:\n{shown}"
    );
    assert!(!prep_file(&rig.cfg).home_ready);
}

#[test]
fn prep_refuses_a_work_folder_inside_a_git_repository_and_names_the_fix() {
    let rig = Rig::chat_only();
    let github = FakeGithub::good();
    let facts = facts(&github.base, &[]);
    log_in(&rig, "http://127.0.0.1:1");
    let repo = rig.root.join("some-repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let inside = repo.join("work");

    let (code, text) = prep(
        &rig,
        &["--repo", REPO, "--work-root", inside.to_str().unwrap()],
        &facts,
    );
    assert_eq!(code, 1, "{text}");
    let machine = line(&text, "machine");
    assert!(
        machine.starts_with("[fail] machine:")
            && machine.contains(&inside.display().to_string())
            && machine.contains(&repo.display().to_string())
            && machine.contains("goalcup prep --work-root ~/goalcup-work"),
        "{text}"
    );
    // A refused folder is not kept, so a later `goalcup start` cannot use it.
    assert!(prep_file(&rig.cfg).work_root.is_empty());

    // A `.git` file (a linked worktree) counts too, and a folder outside any repository passes.
    let linked = rig.root.join("linked");
    std::fs::create_dir_all(&linked).unwrap();
    std::fs::write(linked.join(".git"), "gitdir: /elsewhere\n").unwrap();
    let (_, text) = prep(
        &rig,
        &["--work-root", linked.join("w").to_str().unwrap()],
        &facts,
    );
    assert!(line(&text, "machine").starts_with("[fail]"), "{text}");
    let outside = rig.root.join("outside/work");
    let (_, text) = prep(&rig, &["--work-root", outside.to_str().unwrap()], &facts);
    assert!(line(&text, "machine").starts_with("[ok]"), "{text}");
    assert_eq!(prep_file(&rig.cfg).work_root, outside.display().to_string());
}

#[test]
fn prep_fails_a_wsl_work_root_on_a_windows_host_filesystem() {
    let rig = Rig::chat_only();
    let github = FakeGithub::good();
    // The filesystem is judged by its type, not by where it is mounted: a Windows drive mounted
    // at an odd place is refused, and a Linux mount under `/mnt/c` is not.
    let wsl = facts(
        &github.base,
        &[
            "wsl=2",
            "fs=/odd/place=9p",
            "fs=/mnt/c/linux=ext4",
            "fs=/mnt/c=drvfs",
        ],
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

    // On WSL2 the machine cannot be kept awake by the tracker: Windows decides. That alone never
    // holds the step back; prep says what to set in Windows.
    let (_, text) = prep(
        &rig,
        &["--work-root", "/home/me/goalcup-work"],
        &facts(&github.base, &["wsl=2", "awake=no_inhibitor"]),
    );
    let machine = line(&text, "machine");
    assert!(
        machine.starts_with("[ok] machine:")
            && machine.contains("Windows decides")
            && machine.contains("set sleep to Never"),
        "{text}"
    );
}

#[test]
fn prep_on_wsl2_the_machine_step_is_ok_without_a_sleep_lock() {
    let rig = Rig::chat_only();
    let github = FakeGithub::good();
    let (_, text) = prep(
        &rig,
        &["--work-root", "/home/me/goalcup-work"],
        &facts(&github.base, &["wsl=2", "awake=no_inhibitor"]),
    );
    let machine = line(&text, "machine");
    assert!(
        machine.starts_with("[ok] machine:")
            && machine.contains("Windows Settings, System, Power, set sleep to Never"),
        "{text}"
    );
    // On battery the step still waits for the plug, and the sleep sentence is still said.
    let (_, text) = prep(
        &rig,
        &["--work-root", "/home/me/goalcup-work"],
        &facts(&github.base, &["wsl=2", "awake=no_inhibitor", "ac=battery"]),
    );
    let machine = line(&text, "machine");
    assert!(
        machine.starts_with("[todo] machine:")
            && machine.contains("on battery")
            && machine.contains("Windows decides"),
        "{text}"
    );
}

#[test]
fn prep_fails_wsl_artifact_root_and_config_folder_on_a_windows_host_filesystem() {
    let rig = Rig::chat_only();
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

/// A run keeps its records on APFS, ext4 or XFS only: prep says so for the work folder, the
/// artifact root and the configuration folder, with the way to move each, on any machine.
#[test]
fn prep_fails_a_folder_on_an_unsupported_filesystem_and_says_how_to_move_it() {
    let rig = Rig::chat_only();
    let github = FakeGithub::good();
    let work = rig.root.join("btrfs").join("goalcup-work");
    let on_btrfs = facts(
        &github.base,
        &[&format!("fs={}=btrfs", rig.root.join("btrfs").display())],
    );
    let (code, text) = prep(&rig, &["--work-root", work.to_str().unwrap()], &on_btrfs);
    assert_eq!(code, 1, "{text}");
    let machine = line(&text, "machine");
    assert!(
        machine.starts_with("[fail] machine:")
            && machine.contains(&format!("work folder {}", work.display()))
            && machine.contains("btrfs")
            && machine.contains("ext4, XFS or APFS")
            && machine.contains("goalcup prep --work-root"),
        "{text}"
    );
    // A refused work folder is not kept for `goalcup start` to find.
    assert_ne!(prep_file(&rig.cfg).work_root, work.display().to_string());

    let elsewhere = rig.root.join("goalcup-work");
    let on_overlay = facts(
        &github.base,
        &[&format!("fs={}=overlay", rig.cfg.display())],
    );
    let (code, text) = prep(
        &rig,
        &["--work-root", elsewhere.to_str().unwrap()],
        &on_overlay,
    );
    assert_eq!(code, 1, "{text}");
    let machine = line(&text, "machine");
    assert!(
        machine.contains("config folder")
            && machine.contains("overlayfs")
            && machine.contains("GOALCUP_CONFIG_DIR")
            && !machine.contains("work folder"),
        "{text}"
    );
    assert_eq!(
        prep_file(&rig.cfg).work_root,
        elsewhere.display().to_string()
    );
}

#[test]
fn prep_fails_wsl1_in_one_sentence() {
    let rig = Rig::chat_only();
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
    let rig = Rig::chat_only();
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
    press_enter(&on_pty);
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
    let rig = Rig::chat_only();
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
    let rig = Rig::chat_only();
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
    let rig = Rig::chat_only();
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
    // A sleep lock that cannot be taken is said, and never holds the step back.
    let no_lock = machine(&["awake=no_inhibitor"]);
    assert!(
        no_lock.starts_with("[ok]") && no_lock.contains("keep it awake yourself"),
        "{no_lock}"
    );

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
    let rig = Rig::chat_only();
    let github = FakeGithub::good();
    let facts = facts(&github.base, &[]);

    let (_, text) = prep(&rig, &["--harness", "no-such-harness"], &facts);
    let harness = line(&text, "harness");
    assert!(
        harness.starts_with("[fail]") && harness.contains(CHAT_ID),
        "{text}"
    );
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
    let rig = Rig::chat_only();
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
    let rig = Rig::chat_only();
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

// ------------------------------------------------------------------ the participant's environment

/// The harness as an npm one is: a `#!/usr/bin/env toynode` script, found on a PATH of the
/// participant's own (the rig's directory is not on it). `toynode` is on that PATH, or not.
/// Returns the PATH.
fn npm_style_harness(rig: &Rig, with_interpreter: bool) -> String {
    use std::os::unix::fs::PermissionsExt;
    let bin = rig.root.join("participant-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let fake = std::fs::read_to_string(rig.root.join("bin").join(CHAT_ID)).unwrap();
    let (_, body) = fake.split_once('\n').unwrap();
    let harness = bin.join(CHAT_ID);
    std::fs::write(&harness, format!("#!/usr/bin/env toynode\n{body}")).unwrap();
    std::fs::set_permissions(&harness, std::fs::Permissions::from_mode(0o755)).unwrap();
    if with_interpreter {
        let node = bin.join("toynode");
        std::fs::write(&node, "#!/bin/sh\nexec /bin/sh \"$@\"\n").unwrap();
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    format!(
        "{}:{}",
        bin.display(),
        crate::common::support::clean_env()["PATH"]
    )
}

/// What a participant who has done everything but open the harness has: a login, the repository
/// and a token that passes.
fn all_but_the_harness(rig: &Rig) {
    log_in(rig, "http://127.0.0.1:1");
    GithubSetup::new(REPO, &token())
        .unwrap()
        .store(&rig.cfg)
        .unwrap();
}

#[test]
fn prep_opens_a_harness_that_needs_its_interpreter_with_the_participants_environment() {
    let rig = Rig::chat_only();
    let github = FakeGithub::good();
    let facts = facts(&github.base, &[]);
    all_but_the_harness(&rig);
    let path = npm_style_harness(&rig, true);

    let mut command = prep_command(&rig, &["--repo", REPO], &facts);
    command
        .env("PATH", &path)
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        // Things a participant's shell may hold that the harness must not be handed: a proxy, a
        // model base URL, a token. A prep launch records nothing and sets none of them.
        .env("HTTPS_PROXY", "http://goalcup:secret@127.0.0.1:1")
        .env("https_proxy", "http://goalcup:secret@127.0.0.1:1")
        .env("NO_PROXY", "example.org")
        .env("OPENAI_BASE_URL", "https://llm.example.org/v1")
        .env("ANTHROPIC_BASE_URL", "https://llm.example.org")
        .env("GITHUB_TOKEN", "ghp_participantsownshelltoken")
        .env("LC_API_TOKEN", "lc-token-value");
    let mut on_pty = OnPty::spawn(command, None);
    // With the old fixed PATH the script's interpreter is not found and the harness never draws.
    press_enter(&on_pty);
    on_pty.wait_shown("tui: ready");
    on_pty.type_line("/exit");
    on_pty.wait();
    let shown = on_pty.shown();
    assert!(shown.contains("[ok] harness_home:"), "{shown}");
    let env = rig
        .harness_file("tui.env")
        .expect("the harness reported its environment");
    for want in [
        format!("PATH={path}"),
        "TERM=xterm-256color".to_string(),
        "COLORTERM=truecolor".to_string(),
        "LANG=C.UTF-8".to_string(),
        "LC_ALL=C.UTF-8".to_string(),
    ] {
        assert!(env.lines().any(|l| l == want), "{want} is not in:\n{env}");
    }
    // Not a recorded launch, and not a leak: none of what the shell held reaches the harness,
    // by name or by value.
    for key in [
        "HTTPS_PROXY",
        "https_proxy",
        "NO_PROXY",
        "no_proxy",
        "OPENAI_BASE_URL",
        "ANTHROPIC_BASE_URL",
        "GITHUB_TOKEN",
        "LC_API_TOKEN",
        "CODEX_CA_CERTIFICATE",
    ] {
        assert!(
            !env.lines().any(|l| l.starts_with(&format!("{key}="))),
            "{key} reached the harness:\n{env}"
        );
    }
    for value in [
        "goalcup:secret",
        "llm.example.org",
        "ghp_participantsownshelltoken",
        "lc-token-value",
    ] {
        assert!(!env.contains(value), "{value} reached the harness:\n{env}");
    }
    assert!(
        !env.lines().any(|l| l.starts_with("GOALCUP_")),
        "goalcup's own variables reached the harness:\n{env}"
    );
}

#[test]
fn the_harness_step_checks_the_interpreter_on_the_participants_path() {
    let rig = Rig::chat_only();
    let github = FakeGithub::good();
    let facts = facts(&github.base, &[]);

    let path = npm_style_harness(&rig, false);
    let out = prep_command(&rig, &[], &facts)
        .env("PATH", &path)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let harness = line(&text, "harness");
    assert!(
        harness.starts_with("[fail] harness:")
            && harness.contains("toynode")
            && harness.contains("not on the PATH"),
        "{text}"
    );

    let path = npm_style_harness(&rig, true);
    let out = prep_command(&rig, &[], &facts)
        .env("PATH", &path)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        line(&text, "harness").starts_with("[ok] harness:"),
        "{text}"
    );
}

// ------------------------------------------------------------------ a kept run HOME (D59)

#[test]
fn prep_logs_in_to_the_run_home_a_later_start_uses() {
    use std::os::unix::fs::PermissionsExt;
    let rig = Rig::chat_with_shared_run_home();
    let github = FakeGithub::good();
    let facts = facts(&github.base, &[]);
    all_but_the_harness(&rig);
    let home = rig
        .cfg
        .join("capture-homes")
        .join(format!("{CHAT_HOME_ID}.run-home"));

    // Not set up yet: the step names the run home, and a prep with no terminal opens nothing.
    let (_, text) = prep(&rig, &["--harness", CHAT_HOME_ID, "--repo", REPO], &facts);
    let step = line(&text, "harness_home");
    assert!(
        step.starts_with("[todo] harness_home:") && step.contains(".run-home"),
        "{text}"
    );

    // On a terminal, the harness opens with its HOME the kept run home, and writes there.
    let mut on_pty = OnPty::spawn(
        prep_command(&rig, &["--harness", CHAT_HOME_ID, "--repo", REPO], &facts),
        None,
    );
    press_enter(&on_pty);
    on_pty.wait_shown("tui: ready");
    on_pty.type_line("/exit");
    on_pty.wait();
    let shown = on_pty.shown();
    assert!(shown.contains("[ok] harness_home:"), "{shown}");
    assert_eq!(
        std::fs::read_to_string(home.join("tui-runs.txt")).unwrap(),
        "run\n"
    );
    assert_eq!(
        std::fs::metadata(&home).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert!(
        home.is_dir(),
        "the kept home is not removed with the prep's scratch folder"
    );
    assert!(prep_file(&rig.cfg).home_ready);

    // A later `goalcup start` of that connector runs in the same HOME, and sees what prep left.
    let server = UploadServer::start(&rig.root.join("server"), ServerOptions::default());
    rig.prepare_for(&server.url());
    let mut start = OnPty::spawn(
        rig.chat_command_for(
            CHAT_HOME_ID,
            &[
                ("GOALCUP_TEST_DISK", "4194304,0,50"),
                (
                    "GOALCUP_TEST_UPLOAD_POLICY",
                    "200,25,100,400,10000,268435456,3000,100",
                ),
            ],
            Launcher::Direct,
        ),
        None,
    );
    start.wait_shown("tui: ready");
    assert_eq!(
        rig.harness_file("tui.home").unwrap().trim(),
        home.display().to_string()
    );
    assert_eq!(
        std::fs::read_to_string(home.join("tui-runs.txt")).unwrap(),
        "run\nrun\n",
        "the run's HOME is the one prep set up"
    );
    start.type_line("/exit");
    let status = start.wait();
    assert!(status.success(), "{status:?}\n{}", start.shown());
}

// ------------------------------------------------------------------ the connector's settings (D61)

/// The test doors a chat start on the stand-in server needs: a disk it accepts, and an upload
/// cadence a test can wait out.
const CHAT_START_ENV: [(&str, &str); 2] = [
    ("GOALCUP_TEST_DISK", "4194304,0,50"),
    (
        "GOALCUP_TEST_UPLOAD_POLICY",
        "200,25,100,400,10000,268435456,3000,100",
    ),
];

/// A connector's `capture.configure` command is run in its home after the participant's own
/// setup, and again before each launch (D61): with the harness's environment and no proxy, no
/// terminal for its input, and its output in `prep.log` and the run's `host.log`, never on the
/// screen.
#[test]
fn prep_and_start_put_the_connectors_settings_into_its_home() {
    let rig = Rig::chat_with_configure();
    let github = FakeGithub::good();
    let facts = facts(&github.base, &[]);
    all_but_the_harness(&rig);
    let home = rig.cfg.join("capture-homes").join(CHAT_CONFIGURE_ID);

    let mut on_pty = OnPty::spawn(
        prep_command(
            &rig,
            &["--harness", CHAT_CONFIGURE_ID, "--repo", REPO],
            &facts,
        ),
        None,
    );
    press_enter(&on_pty);
    on_pty.wait_shown("tui: ready");
    // Not yet: the setting follows the participant's own setup.
    assert!(rig.harness_file("tui.configure").is_none());
    on_pty.type_line("/exit");
    on_pty.wait();
    let shown = on_pty.shown();
    assert!(shown.contains("[ok] harness_home:"), "{shown}");
    assert!(prep_file(&rig.cfg).home_ready, "{shown}");
    assert_eq!(
        std::fs::read_to_string(home.join("settings.txt")).unwrap(),
        "snapshots=on\n",
        "{shown}"
    );
    assert_eq!(
        rig.harness_file("tui.configure").unwrap(),
        "configure snapshots on not-a-tty\n"
    );
    assert!(!shown.contains("tui: configured"), "{shown}");
    let log = std::fs::read_to_string(rig.cfg.join("prep.log")).unwrap();
    assert!(log.contains("tui: configured snapshots"), "{log}");
    // The harness's own environment, and nothing of a recording launch.
    let env = rig.harness_file("tui.configure.env").unwrap();
    assert!(
        env.lines()
            .any(|l| l == format!("CODEX_HOME={}", home.display())),
        "{env}"
    );
    for key in ["HTTPS_PROXY", "https_proxy", "CODEX_CA_CERTIFICATE"] {
        assert!(
            !env.lines().any(|l| l.starts_with(&format!("{key}="))),
            "{key}:\n{env}"
        );
    }

    // `goalcup start` runs it again before the harness opens, and logs it in the run's log.
    let server = UploadServer::start(&rig.root.join("server"), ServerOptions::default());
    rig.prepare_for(&server.url());
    let mut start = OnPty::spawn(
        rig.chat_command_for(CHAT_CONFIGURE_ID, &CHAT_START_ENV, Launcher::Direct),
        None,
    );
    start.wait_shown("tui: ready");
    assert_eq!(
        std::fs::read_to_string(home.join("settings.txt")).unwrap(),
        "snapshots=on\nsnapshots=on\n"
    );
    start.type_line("/exit");
    let status = start.wait();
    let shown = start.shown();
    assert!(status.success(), "{status:?}\n{shown}");
    assert!(!shown.contains("tui: configured"), "{shown}");
    let record = goalcup_cli::run::newest(&rig.cfg).unwrap().unwrap();
    let host_log = std::fs::read_to_string(
        goalcup_cli::run::run_dir(&rig.cfg, &record.run_id).join("host.log"),
    )
    .unwrap();
    assert!(host_log.contains("tui: configured snapshots"), "{host_log}");
}

/// A `capture.configure` command that fails leaves the home not ready, and `goalcup start`
/// launches nothing: the harness would otherwise run without the setting its capture needs.
#[test]
fn a_failing_configure_command_keeps_the_home_not_ready_and_launches_nothing() {
    let rig = Rig::chat_with_configure();
    let github = FakeGithub::good();
    let facts = facts(&github.base, &[]);
    all_but_the_harness(&rig);
    std::fs::write(rig.out.join("tui.configure.fail"), "").unwrap();

    let mut on_pty = OnPty::spawn(
        prep_command(
            &rig,
            &["--harness", CHAT_CONFIGURE_ID, "--repo", REPO],
            &facts,
        ),
        None,
    );
    press_enter(&on_pty);
    on_pty.wait_shown("tui: ready");
    on_pty.type_line("/exit");
    on_pty.wait();
    let shown = on_pty.shown();
    let step = line(&shown, "harness_home");
    assert!(
        step.starts_with("[fail] harness_home:")
            && step.contains("could not be set up for recording")
            && step.contains("configure snapshots on")
            && step.contains("exit code 3")
            && step.contains("prep.log"),
        "{shown}"
    );
    assert!(!prep_file(&rig.cfg).home_ready, "{shown}");

    let server = UploadServer::start(&rig.root.join("server"), ServerOptions::default());
    rig.prepare_for(&server.url());
    let mut start = OnPty::spawn(
        rig.chat_command_for(CHAT_CONFIGURE_ID, &CHAT_START_ENV, Launcher::Direct),
        None,
    );
    let status = start.wait();
    let shown = start.shown();
    assert!(!status.success(), "{shown}");
    assert!(
        shown.contains("could not be set up for recording")
            && shown.contains("Nothing was launched")
            && shown.contains(&format!("goalcup prep --harness {CHAT_CONFIGURE_ID}")),
        "{shown}"
    );
    assert!(!shown.contains("tui: ready"), "{shown}");
}

// ------------------------------------------------------------------ a home someone else holds

/// Take the flock `goalcup start` and `goalcup prep` take on `<cfg>/capture-homes/<id>.lock`,
/// as another process: the lock is on this open file, so the prep (a process of its own) is
/// refused. `holder`, when given, is the run id a recording run writes into the file.
fn hold_home_lock(rig: &Rig, id: &str, holder: Option<&str>) -> std::fs::File {
    use std::os::fd::AsRawFd;
    let homes = rig.cfg.join("capture-homes");
    std::fs::create_dir_all(&homes).unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(homes.join(format!("{id}.lock")))
        .unwrap();
    if let Some(run) = holder {
        std::fs::write(homes.join(format!("{id}.lock")), run).unwrap();
    }
    // SAFETY: `lock` owns a live descriptor; `flock` reads nothing else.
    let taken = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(taken, 0, "{}", std::io::Error::last_os_error());
    lock
}

#[test]
fn prep_refuses_a_home_another_process_holds_and_opens_nothing() {
    let github = FakeGithub::good();
    let facts = facts(&github.base, &[]);
    // The config home kept across runs (D39) and the run HOME kept across runs (D59), each
    // held by a recording run that has said who it is, and by a holder that has not.
    for (id, kind, holder) in [
        (CHAT_ID, "config home", Some("r0123456789abcdef")),
        (CHAT_ID, "config home", None),
        (CHAT_HOME_ID, "run HOME", Some("r0123456789abcdef")),
        (CHAT_HOME_ID, "run HOME", None),
    ] {
        let rig = Rig::chat_with_shared_run_home();
        all_but_the_harness(&rig);
        let _held = hold_home_lock(&rig, id, holder);

        let mut on_pty = OnPty::spawn(
            prep_command(&rig, &["--harness", id, "--repo", REPO], &facts),
            None,
        );
        let status = on_pty.wait();
        let shown = on_pty.shown();
        let what = format!("{kind}, holder {holder:?}:\n{shown}");
        assert_eq!(status.code(), Some(1), "{what}");
        // The home is looked at before the pause: a held home is said at once, and prep never
        // asks for an Enter it cannot use.
        assert!(!shown.contains("Press Enter to open"), "{what}");
        let home = line(&shown, "harness_home");
        assert!(home.starts_with("[fail] harness_home:"), "{what}");
        match holder {
            Some(run) => assert!(home.contains(run), "the holder is named: {what}"),
            None => assert!(home.contains("another `goalcup prep`"), "{what}"),
        }
        assert!(
            home.contains(&format!("share one {id} home")) || home.contains(&format!("{id} run")),
            "{what}"
        );
        // Nothing was opened, nothing was made in the held home, and nothing is recorded as
        // set up.
        assert!(
            !rig.out.join("tui.pid").exists(),
            "the harness started: {what}"
        );
        assert!(!prep_file(&rig.cfg).home_ready, "{what}");
        let leaf = if id == CHAT_ID {
            id.to_string()
        } else {
            format!("{id}.run-home")
        };
        assert!(!rig.cfg.join("capture-homes").join(leaf).exists(), "{what}");
    }
}

// ------------------------------------------------------------------ the model service (D62)

/// D67: the chat connector lists two `participant.no_prompts` settings. Prep reads them in the
/// harness's Goal Cup home and recommends each one that is not set, with the doc's link, in the
/// harness home step; it never changes the step's state, the exit code or the files.
#[test]
fn prep_recommends_the_no_prompt_settings_and_never_writes_them() {
    let rig = Rig::chat_only();
    let path = rig.connectors.join(CHAT_ID).join("harness.toml");
    let text = std::fs::read_to_string(&path).unwrap().replacen(
        "[capture]\n",
        "[participant]\ndisplay_name = \"Fake Agent\"\n\n\
         [[participant.no_prompts]]\nfile = \"config.toml\"\nkey = \"approval_policy\"\n\
         values = [\"never\"]\n\n\
         [[participant.no_prompts]]\nfile = \"settings.json\"\nkey = \"permissions.skip\"\n\
         values = [\"true\"]\n\n[capture]\n",
        1,
    );
    std::fs::write(&path, text).unwrap();
    let github = FakeGithub::good();
    let facts = facts(&github.base, &[]);
    all_but_the_harness(&rig);
    let home = rig.cfg.join("capture-homes").join(CHAT_ID);
    const ASKS: &str = "Fake Agent may stop to ask your permission";
    let link = format!(
        "https://github.com/agentcommunity/goal-cup-tracer/blob/main/docs/harnesses/{CHAT_ID}.md\
         #run-without-permission-prompts"
    );

    // Not set up yet: the step is still todo for the set-up itself, and names both settings.
    let (code, out) = prep(&rig, &["--repo", REPO], &facts);
    let step = line(&out, "harness_home");
    assert!(step.starts_with("[todo] harness_home:"), "{out}");
    assert!(step.contains(ASKS) && step.contains(&link), "{out}");
    assert!(
        step.contains("`approval_policy` to `never`")
            && step.contains("`permissions.skip` to `true`"),
        "{out}"
    );
    assert_eq!(code, 1);

    // Set up, nothing set: the step is ok and still says what to set.
    std::fs::create_dir_all(&home).unwrap();
    let mut file = prep_file(&rig.cfg);
    file.harness = CHAT_ID.to_string();
    file.home_ready = true;
    goalcup_cli::prep_file::write_prep(&rig.cfg, &file).unwrap();
    let (_, out) = prep(&rig, &["--repo", REPO, "--json"], &facts);
    let steps = json(&out);
    let home_step = steps.iter().find(|s| s["step"] == "harness_home").unwrap();
    assert_eq!(home_step["state"], "ok", "{out}");
    assert!(home_step["say"].as_str().unwrap().contains(ASKS), "{out}");

    // One set: only the other is named.
    std::fs::write(home.join("config.toml"), "approval_policy = \"never\"\n").unwrap();
    let (_, out) = prep(&rig, &["--repo", REPO], &facts);
    let step = line(&out, "harness_home");
    assert!(step.starts_with("[ok] harness_home:"), "{out}");
    assert!(
        !step.contains("`approval_policy`") && step.contains("`permissions.skip` to `true`"),
        "{out}"
    );

    // Another value is not an accepted one; a JSON boolean is read as `true`.
    std::fs::write(
        home.join("config.toml"),
        "approval_policy = \"on-request\"\n",
    )
    .unwrap();
    std::fs::write(
        home.join("settings.json"),
        r#"{"permissions":{"skip":true}}"#,
    )
    .unwrap();
    let (_, out) = prep(&rig, &["--repo", REPO], &facts);
    let step = line(&out, "harness_home");
    assert!(
        step.contains("`approval_policy` to `never`") && !step.contains("`permissions.skip`"),
        "{out}"
    );

    // All set: no recommendation, and prep wrote neither file.
    std::fs::write(home.join("config.toml"), "approval_policy = \"never\"\n").unwrap();
    let (_, out) = prep(&rig, &["--repo", REPO], &facts);
    let step = line(&out, "harness_home");
    assert!(
        step.starts_with("[ok] harness_home:") && !step.contains(ASKS),
        "{out}"
    );
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).unwrap(),
        "approval_policy = \"never\"\n"
    );
    assert_eq!(
        std::fs::read_to_string(home.join("settings.json")).unwrap(),
        r#"{"permissions":{"skip":true}}"#
    );
}

/// A connector that reads its model service from the harness's config: a home whose harness
/// names one Goal Cup cannot record (here a local `http://localhost` server) is not ready. Prep
/// says why, clears the mark and opens the harness again, so the participant can choose another
/// service there; once the config names an https service on the network, the home is ready.
#[test]
fn prep_keeps_the_home_not_ready_while_the_model_service_cannot_be_recorded() {
    let rig = Rig::chat_only();
    let from = crate::model_host_from::reads_model_host(&rig, CHAT_ID);
    let home = rig.cfg.join("capture-homes").join(CHAT_ID);
    let github = FakeGithub::good();
    let facts = facts(&github.base, &[]);
    all_but_the_harness(&rig);
    let set = |url: &str| crate::common::support::write_model_host_config(&home, &from, url);
    let open_and_exit = |fix: Option<&str>| {
        let mut on_pty = OnPty::spawn(prep_command(&rig, &["--repo", REPO], &facts), None);
        press_enter(&on_pty);
        on_pty.wait_shown("tui: ready");
        if let Some(url) = fix {
            set(url);
        }
        on_pty.type_line("/exit");
        on_pty.wait();
        on_pty.shown()
    };
    const LOCAL: &str = "is set to use http://localhost:11434, which Goal Cup cannot record: \
                         it is plain http";

    // The first set-up picks a local server: the harness exits normally, the home is not ready.
    set("http://localhost:11434/v1");
    let shown = open_and_exit(None);
    let step = line(&shown, "harness_home");
    assert!(step.starts_with("[todo] harness_home:"), "{shown}");
    assert!(
        step.contains(LOCAL) && step.contains("goalcup prep"),
        "{shown}"
    );
    assert!(!prep_file(&rig.cfg).home_ready, "{shown}");
    // Its machine line says only what it probed: no model service was read.
    assert!(
        line(&shown, "machine").contains("the server is reachable"),
        "{shown}"
    );

    // Chosen again in the harness: ready.
    let shown = open_and_exit(Some("https://models.example.test/v1"));
    assert!(shown.contains("[ok] harness_home:"), "{shown}");
    assert!(prep_file(&rig.cfg).home_ready, "{shown}");

    // Changed back to the local server later: a prep with no terminal says why and clears the
    // mark; one on a terminal says why and opens the harness again.
    set("http://localhost:11434/v1");
    let (_, out) = prep(&rig, &["--repo", REPO], &facts);
    let step = line(&out, "harness_home");
    assert!(
        step.starts_with("[todo] harness_home:") && step.contains(LOCAL),
        "{out}"
    );
    assert!(!prep_file(&rig.cfg).home_ready, "{out}");
    let shown = open_and_exit(Some("https://models.example.test/v1"));
    assert!(
        shown.contains(LOCAL),
        "the reason is said before the harness opens:\n{shown}"
    );
    assert!(shown.contains("[ok] harness_home:"), "{shown}");
    assert!(prep_file(&rig.cfg).home_ready, "{shown}");

    // With a service it can read, the machine step probes it too, and says so.
    let (_, out) = prep(&rig, &["--repo", REPO], &facts);
    assert!(
        line(&out, "machine").contains("the server and the model service are reachable"),
        "{out}"
    );
    assert!(line(&out, "harness_home").starts_with("[ok]"), "{out}");
}
