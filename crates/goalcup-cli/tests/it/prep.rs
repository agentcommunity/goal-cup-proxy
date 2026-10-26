//! `goalcup setup` (and its old name, `goalcup prep`): four steps from install to ready,
//! re-runnable, and never starting a run. No GitHub here: a real run asks for its repository at
//! `goalcup start` (`account.rs`).
//!
//! The binary is run against a temporary configuration directory and the chat-mode development
//! connector (`Rig::chat_only`, whose harness is `fixtures/m2-fake-tui.sh`). The machine's facts
//! (mains power, free space, hosts, the sleep lock) are stated through the test door
//! `GOALCUP_TEST_PREP` rather than read from the machine the test runs on, and a run on a
//! terminal is a pseudo-terminal of the test's own.

use crate::common::m2_cli_support::{
    CHAT_CONFIGURE_ID, CHAT_HOME_ID, CHAT_ID, FAKE_ID, Launcher, OnPty, Rig,
};
use crate::common::upload_server::{ServerOptions, UploadServer};
use goalcup_cli::prep_file::{PrepFile, read_prep_lenient};
use goalcup_protocol::control::DaemonClient;
use std::path::Path;
use std::process::{Command, Stdio};

const GIB: u64 = 1 << 30;

// ------------------------------------------------------------------ the binary

/// The facts of a machine that is fine, plus anything the test changes (a later key wins).
fn facts(more: &[&str]) -> String {
    let mut parts = vec![
        "ac=on".to_string(),
        format!("free={}", 100 * GIB),
        "hosts=ok".to_string(),
        "awake=ok".to_string(),
        // Every folder on ext4 unless the test says otherwise, whatever this machine's are.
        "fs=/=ext4".to_string(),
    ];
    parts.extend(more.iter().map(|s| s.to_string()));
    // The parser takes the last of a repeated key.
    parts.join(",")
}

/// `goalcup setup --dev <args>` in `rig`'s world, with the machine as `facts` says.
fn prep_command(rig: &Rig, args: &[&str], facts: &str) -> Command {
    named_command(rig, "setup", args, facts)
}

/// [`prep_command`] under the command name `name`: `setup`, or its old name `prep`.
fn named_command(rig: &Rig, name: &str, args: &[&str], facts: &str) -> Command {
    let home = rig.root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_goalcup"));
    command
        .arg(name)
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

/// Answer the pause before the harness opens: setup asks for Enter, then takes the terminal.
fn press_enter(on_pty: &OnPty) {
    on_pty.wait_shown("Press Enter to open");
    on_pty.type_line("");
}

fn prep_file(cfg: &Path) -> PrepFile {
    read_prep_lenient(cfg)
}

// ------------------------------------------------------------------ the tests

#[test]
fn setup_reports_each_step_and_exits_1_until_all_are_ok() {
    let rig = Rig::chat_only();
    let facts = facts(&[]);

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
        line(&text, "machine").starts_with("[ok] machine:"),
        "{text}"
    );
    // The last line points at the next action: the first step that needs the person. What to
    // do is said once, on that step's own line.
    let last = text.lines().last().unwrap();
    assert_eq!(
        last, "goalcup setup: 2 of 4 steps need you. Next: the login step above.",
        "{text}"
    );
    let login_say = line(&text, "login")
        .split_once(": ")
        .map(|(_, say)| say)
        .unwrap();
    assert_eq!(text.matches(login_say).count(), 1, "{text}");
    // The four steps are printed in order; GitHub and practice are not steps of setup.
    let order: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with('['))
        .filter_map(|l| l.split(' ').nth(1))
        .collect();
    assert_eq!(
        order,
        ["login:", "harness:", "harness_home:", "machine:"],
        "{text}"
    );

    // The person logs in, then opens the harness once; each is what a later setup finds.
    log_in(&rig, "http://127.0.0.1:1");
    let (code, text) = prep(&rig, &[], &facts);
    assert_eq!(code, 1, "{text}");
    assert!(line(&text, "login").starts_with("[ok] login:"), "{text}");
    let mut file = prep_file(&rig.cfg);
    assert_eq!(file.harness, CHAT_ID);
    file.home_ready = true;
    // The person's word counts only while the home they set up is there.
    goalcup_cli::prep_file::write_prep(&rig.cfg, &file).unwrap();
    let (_, text) = prep(&rig, &[], &facts);
    assert!(line(&text, "harness_home").starts_with("[todo]"), "{text}");
    std::fs::create_dir_all(rig.chat_home()).unwrap();
    goalcup_cli::prep_file::write_prep(&rig.cfg, &file).unwrap();
    let (code, text) = prep(&rig, &[], &facts);
    assert_eq!(code, 0, "{text}");
    for step in ["login", "harness", "harness_home", "machine"] {
        assert!(line(&text, step).starts_with("[ok]"), "{step}: {text}");
    }
    // Ready, with the optional practice run and the real run, in that order.
    let tail: Vec<&str> = text.lines().rev().take(3).collect();
    assert!(
        tail[2] == "Ready."
            && tail[1].contains("mktemp -d ~/goalcup-practice-XXXX")
            && tail[1].ends_with("goalcup start --harness m2-fake-tui --practice")
            && tail[0].contains("folder step")
            && tail[0].ends_with("goalcup start --harness m2-fake-tui --repo <you>/<name>"),
        "{text}"
    );
    // And what was settled is what a run on a server reads.
    let kept = goalcup_cli::prep_file::read_prep(&rig.cfg).unwrap();
    assert_eq!(kept.harness, CHAT_ID);
}

#[test]
fn setup_logs_in_inline_when_this_computer_is_not_connected() {
    let rig = Rig::chat_only();
    let server = UploadServer::start(&rig.root.join("server"), ServerOptions::default());
    // A server is named but no account: `goalcup login`'s device flow runs inside setup.
    goalcup_cli::server::write_server_config(
        &rig.cfg,
        &goalcup_cli::server::ServerConfig {
            url: server.url(),
            account: None,
        },
    )
    .unwrap();
    let mut on_pty = OnPty::spawn(prep_command(&rig, &[], &facts(&[])), None);
    on_pty.wait_shown("[ok] login:");
    press_enter(&on_pty);
    on_pty.wait_shown("tui: ready");
    on_pty.type_line("/exit");
    on_pty.wait();
    let shown = on_pty.shown();
    assert!(shown.contains("not connected to Goal Cup yet"), "{shown}");
    let stored = goalcup_cli::server::read_server_config(&rig.cfg)
        .unwrap()
        .unwrap();
    assert!(stored.account.is_some(), "{shown}");
}

/// Old docs and players say `goalcup prep`: it is a hidden alias that runs setup itself.
#[test]
fn the_old_name_prep_runs_setup() {
    let rig = Rig::chat_only();
    let facts = facts(&[]);
    let report = |name: &str| {
        let out = named_command(&rig, name, &["--json"], &facts)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (out.status.code(), String::from_utf8(out.stdout).unwrap())
    };
    let (code, text) = report("prep");
    assert_eq!(code, Some(1), "{text}");
    assert_eq!((code, text), report("setup"));
}

/// A version before D69 stored a GitHub token in `<cfg>/github.json`. Setup says where it is and
/// what to do with it, every time, and never prints the token.
#[test]
fn setup_names_an_old_github_token_file_and_never_prints_the_token() {
    let rig = Rig::chat_only();
    let facts = facts(&[]);
    let token = format!("github_pat_{}", "Z".repeat(60));
    let stored = rig.cfg.join("github.json");
    std::fs::create_dir_all(&rig.cfg).unwrap();
    std::fs::write(
        &stored,
        format!(r#"{{"repo_url":"https://github.com/you/old","token":"{token}"}}"#),
    )
    .unwrap();
    let (_, out) = prep(&rig, &[], &facts);
    let want = format!(
        "An old GitHub token from an earlier goalcup is stored at {}. Goal Cup no longer uses \
         it: delete the file and revoke the token on GitHub once your older dry run is over.",
        stored.display()
    );
    assert!(out.lines().any(|l| l == want), "{out}");
    assert!(!out.contains(&token), "{out}");
    // With no such file, nothing is said about one.
    std::fs::remove_file(&stored).unwrap();
    let (_, out) = prep(&rig, &[], &facts);
    assert!(!out.contains("old GitHub token"), "{out}");
}

#[test]
fn a_second_setup_skips_passed_steps() {
    let rig = Rig::chat_only();
    let facts = facts(&[]);
    log_in(&rig, "http://127.0.0.1:1");

    // The first setup, on a terminal: it opens the harness.
    let mut first = OnPty::spawn(prep_command(&rig, &[], &facts), None);
    press_enter(&first);
    first.wait_shown("tui: ready");
    first.type_line("/exit");
    let status = first.wait();
    let shown = first.shown();
    assert_eq!(status.code(), Some(0), "{shown}");
    assert!(shown.contains("[ok] harness_home:"), "{shown}");
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
    assert!(prep_file(&rig.cfg).home_ready);

    // The second setup, on a terminal again, does not open the harness again.
    std::fs::remove_file(rig.out.join("tui.pid")).unwrap();
    std::fs::remove_file(rig.out.join("tui.ready")).unwrap();
    let mut second = OnPty::spawn(prep_command(&rig, &[], &facts), None);
    let status = second.wait();
    let shown = second.shown();
    assert_eq!(status.code(), Some(0), "{shown}");
    assert!(shown.contains("[ok] harness_home:"), "{shown}");
    assert!(
        !shown.contains("tui: ready"),
        "the harness was opened again:\n{shown}"
    );
    assert!(
        !rig.out.join("tui.pid").exists(),
        "the harness started again"
    );
}

#[test]
fn setup_waits_for_enter_before_it_opens_the_harness() {
    let rig = Rig::chat_only();
    let facts = facts(&[]);
    log_in(&rig, "http://127.0.0.1:1");

    let mut on_pty = OnPty::spawn(prep_command(&rig, &[], &facts), None);
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
    on_pty.wait();
    assert!(prep_file(&rig.cfg).home_ready);
}

#[test]
fn setup_does_not_open_the_harness_when_the_input_ends_at_the_pause() {
    let rig = Rig::chat_only();
    let facts = facts(&[]);
    log_in(&rig, "http://127.0.0.1:1");

    let mut on_pty = OnPty::spawn(prep_command(&rig, &[], &facts), None);
    on_pty.wait_shown("Press Enter to open");
    // Ctrl-D at the start of a line: the input ends with no Enter. Only that one byte is
    // written, so nothing is left in the terminal's queue for the question that follows.
    on_pty.type_bytes("\u{4}");
    on_pty.wait();
    let shown = on_pty.shown();
    // The terminal echoes the typed Ctrl-D (`^D`, then backspaces on macOS) and no newline.
    // Setup ends that line, so the report line is a line of its own, word for word.
    let lines: Vec<&str> = shown.lines().map(|l| l.trim_end_matches('\r')).collect();
    let at = lines
        .iter()
        .position(|l| l.contains("harness_home:"))
        .unwrap_or_else(|| panic!("no harness_home line in:\n{shown}"));
    assert_eq!(
        lines[at],
        format!(
            "[todo] harness_home: {CHAT_ID} was not opened. Run `goalcup setup` again, then \
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
fn prep_on_wsl2_the_machine_step_is_ok_without_a_sleep_lock() {
    let rig = Rig::chat_only();
    let (_, text) = prep(&rig, &[], &facts(&["wsl=2", "awake=no_inhibitor"]));
    let machine = line(&text, "machine");
    assert!(
        machine.starts_with("[ok] machine:")
            && machine.contains("Windows Settings, System, Power, set sleep to Never"),
        "{text}"
    );
    // On battery the step still waits for the plug, and the sleep sentence is still said.
    let (_, text) = prep(
        &rig,
        &[],
        &facts(&["wsl=2", "awake=no_inhibitor", "ac=battery"]),
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
    let artifacts = format!("{}-artifacts", rig.cfg.display());

    let on_artifacts = facts(&["wsl=2", &format!("fs={artifacts}=9p")]);
    let (code, text) = prep(&rig, &[], &on_artifacts);
    assert_eq!(code, 1, "{text}");
    let machine = line(&text, "machine");
    assert!(
        machine.starts_with("[fail] machine:")
            && machine.contains(&format!("artifact root {artifacts}"))
            && !machine.contains("config folder"),
        "{text}"
    );

    let on_config = facts(&["wsl=2", &format!("fs={}=virtiofs", rig.cfg.display())]);
    let (_, text) = prep(&rig, &[], &on_config);
    let machine = line(&text, "machine");
    assert!(
        machine.contains("config folder") && machine.contains("(virtiofs)"),
        "{text}"
    );
}

/// A run keeps its records on APFS, ext4 or XFS only: prep says so for the artifact root and the
/// configuration folder, with the way to move them, on any machine.
#[test]
fn prep_fails_a_folder_on_an_unsupported_filesystem_and_says_how_to_move_it() {
    let rig = Rig::chat_only();
    let on_overlay = facts(&[&format!("fs={}=overlay", rig.cfg.display())]);
    let (code, text) = prep(&rig, &[], &on_overlay);
    assert_eq!(code, 1, "{text}");
    let machine = line(&text, "machine");
    assert!(
        machine.starts_with("[fail] machine:")
            && machine.contains("config folder")
            && machine.contains("overlayfs")
            && machine.contains("GOALCUP_CONFIG_DIR"),
        "{text}"
    );
}

#[test]
fn prep_fails_wsl1_in_one_sentence() {
    let rig = Rig::chat_only();
    let (code, text) = prep(&rig, &[], &facts(&["wsl=1"]));
    assert_eq!(code, 1, "{text}");
    let machine = line(&text, "machine");
    assert!(
        machine.starts_with("[fail] machine:")
            && machine.contains("WSL1 cannot run Goal Cup: convert this distribution to WSL2 with")
            && machine.contains("`wsl --set-version <distro> 2`"),
        "{text}"
    );
}

#[test]
fn prep_does_not_start_a_run() {
    let rig = Rig::chat_only();
    let server = UploadServer::start(&rig.root.join("server"), ServerOptions::default());
    log_in(&rig, &server.url());
    let facts = facts(&[]);

    // With no terminal, with --json, and on a terminal.
    let (_, text) = prep(&rig, &[], &facts);
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
    let facts = facts(&[]);

    let (code, text) = prep(&rig, &["--json"], &facts);
    assert_eq!(code, 1);
    let steps = json(&text);
    let ids: Vec<&str> = steps.iter().map(|s| s["step"].as_str().unwrap()).collect();
    assert_eq!(ids, ["login", "harness", "harness_home", "machine"]);
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
            ("machine", false),
        ]
    );
    let states: Vec<&str> = steps.iter().map(|s| s["state"].as_str().unwrap()).collect();
    assert_eq!(states, ["todo", "ok", "todo", "ok"]);
    // Nothing but the array is printed.
    assert!(text.trim_start().starts_with('[') && text.trim_end().ends_with(']'));

    // A step that has passed needs nobody.
    log_in(&rig, "http://127.0.0.1:1");
    let (_, text) = prep(&rig, &["--json"], &facts);
    let steps = json(&text);
    assert_eq!(steps[0]["state"], "ok");
    assert_eq!(steps[0]["human"], false);

    // On a terminal, --json still asks nothing and opens nothing.
    let mut on_pty = OnPty::spawn(prep_command(&rig, &["--json"], &facts), None);
    let status = on_pty.wait();
    let shown = on_pty.shown();
    assert_eq!(status.code(), Some(1), "{shown}");
    assert!(
        !shown.contains("Press Enter") && !shown.contains("tui: ready"),
        "{shown}"
    );
    assert!(
        !rig.out.join("tui.pid").exists(),
        "--json launched the harness"
    );
    let steps = json(shown.trim());
    assert_eq!(steps[2]["state"], "todo");
    assert_eq!(steps[2]["human"], true);
}

#[test]
fn the_machine_step_judges_power_space_git_and_the_network() {
    let rig = Rig::chat_only();
    let machine = |more: &[&str]| {
        let (code, text) = prep(&rig, &[], &facts(more));
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
    let out = prep_command(&rig, &[], &facts(&[]))
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

/// A key in `~/.ssh` with no passphrase is a `[warn]` line after the machine step, which keeps
/// its state: a warning, never a refusal (D73).
#[test]
fn setup_warns_once_of_an_ssh_key_with_no_passphrase() {
    let rig = Rig::chat_only();
    if !Path::new("/usr/bin/ssh-keygen").exists() {
        return;
    }
    let home = rig.root.join("ssh-home");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    let key = home.join(".ssh/id_ed25519");
    let made = Command::new("/usr/bin/ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "test", "-f"])
        .arg(&key)
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(made.success());
    let out = prep_command(&rig, &[], &facts(&[]))
        .env("GOALCUP_TEST_SSH_HOME", &home)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(line(&text, "machine").starts_with("[ok]"), "{text}");
    let warned: Vec<&str> = text.lines().filter(|l| l.starts_with("[warn]")).collect();
    assert_eq!(warned.len(), 1, "{text}");
    assert!(warned[0].contains(&key.display().to_string()), "{text}");
}

/// With more than one harness and none chosen, the harness step lists the installed ones, the
/// other supported ones with where to read how to install them, and how to add one's own; in a
/// terminal it asks which one, the first installed one by default.
#[test]
fn the_harness_step_lists_every_supported_harness_and_asks_which_one() {
    let rig = Rig::chat_with_shared_run_home();
    // Two chat-mode harnesses with homes to set up: the rig's own and the shared-home one.
    std::fs::remove_dir_all(rig.connectors.join(FAKE_ID)).unwrap();
    let facts = facts(&[]);
    let (_, text) = prep(&rig, &[], &facts);
    let step = text
        .split("[todo] harness:")
        .nth(1)
        .and_then(|rest| rest.split("\n[").next())
        .unwrap_or_else(|| panic!("{text}"));
    assert!(
        step.contains("Installed:")
            && step.contains(CHAT_ID)
            && step.contains(CHAT_HOME_ID)
            && step.contains("Your own harness?")
            && step.contains(
                "https://github.com/agentcommunity/goal-cup-proxy/blob/main/docs/HARNESS-ONBOARDING.md"
            ),
        "the link to adding a harness is in this repository, under its name:\n{text}"
    );
    // None on the PATH: every one is "also supported", with where to read how to install it.
    let out = prep_command(&rig, &[], &facts)
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains("Installed: none yet")
            && text.contains("Also supported:")
            && text.contains("HARNESS-SETUP.md"),
        "{text}"
    );

    // In a terminal it asks, and Enter takes the default it shows.
    log_in(&rig, "http://127.0.0.1:1");
    let mut on_pty = OnPty::spawn(prep_command(&rig, &[], &facts), None);
    on_pty.wait_shown("Which one? [");
    let shown = on_pty.shown();
    let default = shown
        .split("Which one? [")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .unwrap()
        .to_string();
    on_pty.type_line("");
    on_pty.wait_shown(&format!("[ok] harness: {default}"));
    on_pty.wait_shown("Press Enter to open");
    on_pty.type_bytes("\u{4}");
    on_pty.wait();
    assert_eq!(prep_file(&rig.cfg).harness, default);
}

#[test]
fn the_harness_step_names_what_it_cannot_use() {
    let rig = Rig::chat_only();
    let facts = facts(&[]);

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

/// What a participant who has done everything but open the harness has: a login.
fn all_but_the_harness(rig: &Rig) {
    log_in(rig, "http://127.0.0.1:1");
}

#[test]
fn prep_opens_a_harness_that_needs_its_interpreter_with_the_participants_environment() {
    let rig = Rig::chat_only();
    let facts = facts(&[]);
    all_but_the_harness(&rig);
    let path = npm_style_harness(&rig, true);

    let mut command = prep_command(&rig, &[], &facts);
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
    let facts = facts(&[]);

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
    let facts = facts(&[]);
    all_but_the_harness(&rig);
    let home = rig
        .cfg
        .join("capture-homes")
        .join(format!("{CHAT_HOME_ID}.run-home"));

    // Not set up yet: the step names the run home, and a prep with no terminal opens nothing.
    let (_, text) = prep(&rig, &["--harness", CHAT_HOME_ID], &facts);
    let step = line(&text, "harness_home");
    assert!(
        step.starts_with("[todo] harness_home:") && step.contains(".run-home"),
        "{text}"
    );

    // On a terminal, the harness opens with its HOME the kept run home, and writes there.
    let mut on_pty = OnPty::spawn(
        prep_command(&rig, &["--harness", CHAT_HOME_ID], &facts),
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
    let facts = facts(&[]);
    all_but_the_harness(&rig);
    let home = rig.cfg.join("capture-homes").join(CHAT_CONFIGURE_ID);

    let mut on_pty = OnPty::spawn(
        prep_command(&rig, &["--harness", CHAT_CONFIGURE_ID], &facts),
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
    let facts = facts(&[]);
    all_but_the_harness(&rig);
    std::fs::write(rig.out.join("tui.configure.fail"), "").unwrap();

    let mut on_pty = OnPty::spawn(
        prep_command(&rig, &["--harness", CHAT_CONFIGURE_ID], &facts),
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
            && shown.contains(&format!("goalcup setup --harness {CHAT_CONFIGURE_ID}")),
        "{shown}"
    );
    assert!(!shown.contains("tui: ready"), "{shown}");
}

// ------------------------------------------------------------------ a home someone else holds

/// Take the flock `goalcup start` and `goalcup prep` take on
/// `<cfg>/capture-homes/<id>.goalcup.lock`, as another process: the lock is on this open file, so
/// the prep (a process of its own) is refused. `holder`, when given, is the run id a recording run
/// writes into the file.
fn hold_home_lock(rig: &Rig, id: &str, holder: Option<&str>) -> std::fs::File {
    use std::os::fd::AsRawFd;
    let homes = rig.cfg.join("capture-homes");
    std::fs::create_dir_all(&homes).unwrap();
    let path = homes.join(format!("{id}.goalcup.lock"));
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    if let Some(run) = holder {
        std::fs::write(&path, run).unwrap();
    }
    // SAFETY: `lock` owns a live descriptor; `flock` reads nothing else.
    let taken = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(taken, 0, "{}", std::io::Error::last_os_error());
    lock
}

#[test]
fn prep_refuses_a_home_another_process_holds_and_opens_nothing() {
    let facts = facts(&[]);
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

        let mut on_pty = OnPty::spawn(prep_command(&rig, &["--harness", id], &facts), None);
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
            None => assert!(home.contains("another `goalcup setup`"), "{what}"),
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

/// The setup guide (fix plan v8), as prep links to it.
const SETUP_GUIDE: &str =
    "https://github.com/agentcommunity/goal-cup-proxy/blob/main/docs/HARNESS-SETUP.md";

/// Fix plan v8: prep's harness step points to the setup guide on every prep, with nothing
/// missing too, at the section the connector names, else at the guide; the step stays ok.
#[test]
fn prep_points_the_harness_step_to_the_setup_guide() {
    let rig = Rig::chat_only();
    let facts = facts(&[]);
    all_but_the_harness(&rig);

    // The connector names no section: the guide itself.
    let (_, out) = prep(&rig, &[], &facts);
    let step = line(&out, "harness");
    assert!(step.starts_with("[ok] harness:"), "{out}");
    assert!(
        step.ends_with(&format!("Before a long run, read {SETUP_GUIDE}")),
        "{out}"
    );

    // It names one: that section, in the text and in `--json`.
    let path = rig.connectors.join(CHAT_ID).join("harness.toml");
    let text = std::fs::read_to_string(&path).unwrap().replacen(
        "[capture]\n",
        "[participant]\ndisplay_name = \"Fake Agent\"\nsetup_section = \"fake-agent\"\n\n\
         [capture]\n",
        1,
    );
    std::fs::write(&path, text).unwrap();
    let want = format!("Before a long run, read {SETUP_GUIDE}#fake-agent");
    let (_, out) = prep(&rig, &[], &facts);
    let step = line(&out, "harness");
    assert!(
        step.starts_with("[ok] harness:") && step.ends_with(&want),
        "{out}"
    );
    let (_, out) = prep(&rig, &["--json"], &facts);
    let steps = json(&out);
    let harness = steps.iter().find(|s| s["step"] == "harness").unwrap();
    assert_eq!(harness["state"], "ok", "{out}");
    assert!(harness["say"].as_str().unwrap().ends_with(&want), "{out}");
}

/// Fix plan v9: a connector that names where its MCP servers are listed gets, in prep's harness
/// step, the lines that add `goalcup mcp` there, with this binary's absolute path and the file in
/// the harness's Goal Cup home; prep writes nothing there, and the step stays ok.
#[test]
fn prep_prints_the_time_left_tool_lines_with_the_absolute_path_and_writes_nothing() {
    let rig = Rig::chat_only();
    let path = rig.connectors.join(CHAT_ID).join("harness.toml");
    let text = std::fs::read_to_string(&path).unwrap().replacen(
        "[capture]\n",
        "[participant]\ndisplay_name = \"Fake Agent\"\n\n\
         [participant.time_left_tool]\nfile = \"config.toml\"\nkey = \"mcp_servers\"\n\
         pass_env = \"env_vars\"\n\n[capture]\n",
        1,
    );
    std::fs::write(&path, text).unwrap();
    let facts = facts(&[]);
    all_but_the_harness(&rig);
    let home = rig.cfg.join("capture-homes").join(CHAT_ID);
    let file = home.join("config.toml");
    let goalcup = env!("CARGO_BIN_EXE_goalcup");

    for json_mode in [false, true] {
        let mut args = Vec::<&str>::new();
        if json_mode {
            args.push("--json");
        }
        let (_, out) = prep(&rig, &args, &facts);
        let say = if json_mode {
            let steps = json(&out);
            let harness = steps.iter().find(|s| s["step"] == "harness").unwrap();
            assert_eq!(harness["state"], "ok", "{out}");
            harness["say"].as_str().unwrap().to_string()
        } else {
            assert!(line(&out, "harness").starts_with("[ok] harness:"), "{out}");
            out.clone()
        };
        assert!(
            say.contains(&format!("`mcp_servers` in {}", file.display())),
            "{say}"
        );
        let lines: Vec<&str> = say.lines().map(str::trim).collect();
        for want in [
            "[mcp_servers.goalcup]".to_string(),
            format!("command = {}", serde_json::to_string(goalcup).unwrap()),
            "args = [\"mcp\"]".to_string(),
            "env_vars = [\"GOALCUP_TIME_LEFT_FILE\"]".to_string(),
        ] {
            assert!(lines.contains(&want.as_str()), "{want} in:\n{say}");
        }
    }
    assert!(!file.exists(), "prep wrote the harness's config");
}

/// Review P4 L3: the time-left lines prep prints, copied exactly as shown from between its two
/// marker lines and put where it says, give the harness's file a `goalcup` server at the top
/// level, next to what the file already had. In YAML the indentation is the structure: lines
/// printed with spaces in front would nest under the key above them. TOML and JSON too.
#[test]
fn the_time_left_lines_pasted_where_prep_says_add_the_server_at_the_top_level() {
    let goalcup = env!("CARGO_BIN_EXE_goalcup");
    let var = "GOALCUP_TIME_LEFT_FILE";
    type Paste = fn(&str, &str) -> String;
    let at_the_end: Paste = |had, lines| format!("{had}{lines}\n");
    let in_the_braces: Paste = |had, lines| {
        let inside = had.trim_end().strip_suffix('}').unwrap().trim_end();
        format!("{inside},\n{lines}\n}}\n")
    };
    // The file, how the harness passes the variable, what the file had, and where prep says the
    // lines go.
    let cases: [(&str, &str, &str, Paste); 3] = [
        (
            "config.yaml",
            "env_template",
            "model:\n  base_url: \"https://api.example.test\"\n  default: m\n",
            at_the_end,
        ),
        (
            "config.toml",
            "env_vars",
            "[model]\nbase_url = \"https://api.example.test\"\n",
            at_the_end,
        ),
        (
            "settings.json",
            "inherited",
            "{\n  \"model\": { \"base_url\": \"https://api.example.test\" }\n}\n",
            in_the_braces,
        ),
    ];
    for (name, pass_env, had, paste) in cases {
        let rig = Rig::chat_only();
        let path = rig.connectors.join(CHAT_ID).join("harness.toml");
        let text = std::fs::read_to_string(&path).unwrap().replacen(
            "[capture]\n",
            &format!(
                "[participant]\ndisplay_name = \"Fake Agent\"\n\n\
                 [participant.time_left_tool]\nfile = \"{name}\"\nkey = \"mcp_servers\"\n\
                 pass_env = \"{pass_env}\"\n\n[capture]\n"
            ),
            1,
        );
        std::fs::write(&path, text).unwrap();
        let facts = facts(&[]);
        all_but_the_harness(&rig);
        let (_, out) = prep(&rig, &[], &facts);
        let mut printed = out.lines();
        printed
            .find(|l| *l == "  ----- start of the lines to copy -----")
            .unwrap_or_else(|| panic!("{name}: no start marker in:\n{out}"));
        let lines: Vec<&str> = printed
            .by_ref()
            .take_while(|l| *l != "  ----- end of the lines to copy -----")
            .collect();
        if name == "config.yaml" {
            let command = format!("    command: {}", serde_json::to_string(goalcup).unwrap());
            let env = format!("      {var}: \"${{{var}}}\"");
            assert_eq!(
                lines,
                [
                    "mcp_servers:",
                    "  goalcup:",
                    &command,
                    "    args: [\"mcp\"]",
                    "    env:",
                    &env,
                ],
                "the YAML lines, as they go in the file"
            );
        }
        let file = rig.cfg.join(name);
        std::fs::write(&file, paste(had, &lines.join("\n"))).unwrap();
        let read = |key: &str| goalcup_cli::model_host::read_setting(&file, name, key);
        let pasted = std::fs::read_to_string(&file).unwrap();
        assert_eq!(
            read("mcp_servers.goalcup.command").as_deref(),
            Some(goalcup),
            "{name}:\n{pasted}"
        );
        assert_eq!(
            read("model.base_url").as_deref(),
            Some("https://api.example.test"),
            "{name}: what the file had is unchanged:\n{pasted}"
        );
        if pass_env == "env_template" {
            assert_eq!(
                read(&format!("mcp_servers.goalcup.env.{var}")),
                Some(format!("${{{var}}}")),
                "{name}:\n{pasted}"
            );
        }
    }
}

/// D67: the chat connector lists two `participant.no_prompts` settings. Prep reads them in the
/// harness's Goal Cup home and recommends each one that is not set, with the setup guide's
/// link, in the harness home step; it never changes the step's state, the exit code or the files.
#[test]
fn prep_recommends_the_no_prompt_settings_and_never_writes_them() {
    let rig = Rig::chat_only();
    let path = rig.connectors.join(CHAT_ID).join("harness.toml");
    let text = std::fs::read_to_string(&path).unwrap().replacen(
        "[capture]\n",
        "[participant]\ndisplay_name = \"Fake Agent\"\nsetup_section = \"fake-agent\"\n\n\
         [[participant.no_prompts]]\nfile = \"config.toml\"\nkey = \"approval_policy\"\n\
         values = [\"never\"]\n\n\
         [[participant.no_prompts]]\nfile = \"settings.json\"\nkey = \"permissions.skip\"\n\
         values = [\"true\"]\n\n[capture]\n",
        1,
    );
    std::fs::write(&path, text).unwrap();
    let facts = facts(&[]);
    all_but_the_harness(&rig);
    let home = rig.cfg.join("capture-homes").join(CHAT_ID);
    const ASKS: &str = "Fake Agent may stop to ask your permission";
    let link = format!("{SETUP_GUIDE}#fake-agent");

    // Not set up yet: the step is still todo for the set-up itself, and names both settings.
    let (code, out) = prep(&rig, &[], &facts);
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
    let (_, out) = prep(&rig, &["--json"], &facts);
    let steps = json(&out);
    let home_step = steps.iter().find(|s| s["step"] == "harness_home").unwrap();
    assert_eq!(home_step["state"], "ok", "{out}");
    assert!(home_step["say"].as_str().unwrap().contains(ASKS), "{out}");

    // One set: only the other is named.
    std::fs::write(home.join("config.toml"), "approval_policy = \"never\"\n").unwrap();
    let (_, out) = prep(&rig, &[], &facts);
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
    let (_, out) = prep(&rig, &[], &facts);
    let step = line(&out, "harness_home");
    assert!(
        step.contains("`approval_policy` to `never`") && !step.contains("`permissions.skip`"),
        "{out}"
    );

    // All set: no recommendation, and prep wrote neither file.
    std::fs::write(home.join("config.toml"), "approval_policy = \"never\"\n").unwrap();
    let (_, out) = prep(&rig, &[], &facts);
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
    let facts = facts(&[]);
    all_but_the_harness(&rig);
    let set = |url: &str| crate::common::support::write_model_host_config(&home, &from, url);
    let open_and_exit = |fix: Option<&str>| {
        let mut on_pty = OnPty::spawn(prep_command(&rig, &[], &facts), None);
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
        step.contains(LOCAL) && step.contains("goalcup setup"),
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
    let (_, out) = prep(&rig, &[], &facts);
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
    let (_, out) = prep(&rig, &[], &facts);
    assert!(
        line(&out, "machine").contains("the server and the model service are reachable"),
        "{out}"
    );
    assert!(line(&out, "harness_home").starts_with("[ok]"), "{out}");
}
