//! A run whose harness writes no session file while the recording proxy records its model
//! requests (a silent native leg): `goalcup start` says so once, in `host.log` while the harness
//! owns the terminal, `goalcup status` keeps it as a `PROBLEM:` line, and a session file clears
//! it. Nothing is stopped or paused for it (D2).
//!
//! The rig's fixture connectors: the headless `m2-fake`, whose native source is
//! `{run_home}/native/*.jsonl` and which never writes there, and the chat-mode `m2-fake-tui`,
//! which writes its session at the first line typed. Their model host is a local upstream.

use crate::common::m2_cli_support::{Hooked, Rig, Upstream};
use goalcup_cli::participant::NATIVE_SILENT_MARK;
use goalcup_cli::run::read_state;
use std::path::Path;
use std::time::{Duration, Instant};

const RESERVE: u64 = 4 * 1024 * 1024;

fn joined<T>(handle: std::thread::ScopedJoinHandle<'_, T>) -> T {
    handle.join().expect("the start thread panicked")
}

/// Wait until the run's state at `dir` says `silent`, and return that state.
fn wait_silent(dir: &Path, silent: Option<u64>) -> goalcup_cli::run::RunState {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(state) = read_state(dir)
            && state.native_silent == silent
        {
            return state;
        }
        assert!(Instant::now() < deadline, "native_silent never {silent:?}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Once the wire has recorded the requests and the time has passed (here one request and no
/// wait) and the native reader has taken no file, `goalcup status` has the `PROBLEM:` line and
/// `--json` has it under `problems`. A session file clears both. The agent is never stopped.
#[test]
fn a_silent_native_leg_is_a_problem_until_a_session_file_appears() {
    use goalcup_cli::commands::status::{PROBLEMS_KEY, status_with};
    use goalcup_cli::run::{HarnessState, NativeSilence, run_dir};
    let rig = Rig::new();
    let upstream = Upstream::start(&rig.runtime);
    rig.tell("steps.sh", &upstream.curl(r#"{"n":1}"#));
    let goal = rig.run_folder();
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let mut opts = rig.options(&goal, RESERVE);
    opts.native_silence = NativeSilence {
        requests: 1,
        after: Duration::ZERO,
    };
    let status = |entry_id: &str, json: bool| {
        let mut out = Vec::new();
        let now = goalcup_protocol::control::format_z(goalcup_cli::run::now_unix());
        status_with(&http, &rig.cfg, Some(entry_id), json, &now, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    };
    let (started, silent, text, json, cleared, after) = std::thread::scope(|s| {
        let start = rig.start(s, &client, opts, Some(upstream.roots()));
        rig.wait_for("sleeper");
        let entry_id = client.created().entry_id;
        let dir = run_dir(&rig.cfg, &entry_id);
        let silent = wait_silent(&dir, Some(1));
        let text = status(&entry_id, false);
        let json = status(&entry_id, true);
        // The harness writes its session file under the native root (`{run_home}/native`).
        let env = rig.harness_file("env").unwrap();
        let home = env.lines().find_map(|l| l.strip_prefix("HOME=")).unwrap();
        let native = Path::new(home).join("native");
        std::fs::create_dir_all(&native).unwrap();
        std::fs::write(native.join("session.jsonl"), "{}\n").unwrap();
        let cleared = wait_silent(&dir, None);
        let after = status(&entry_id, false);
        goalcup_cli::commands::seal::seal_with(
            &http,
            &rig.connectors,
            &rig.cfg,
            &entry_id,
            &mut Vec::new(),
        )
        .unwrap();
        (joined(start), silent, text, json, cleared, after)
    });
    let (result, out) = started;
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert_eq!(
        rig.harness_file("curl.err").unwrap_or_default(),
        "",
        "the request reached the upstream"
    );
    assert_eq!(silent.harness, HarnessState::Running);
    assert!(
        text.lines()
            .any(|line| line.starts_with("PROBLEM:") && line.contains(NATIVE_SILENT_MARK)),
        "{text}"
    );
    let raw: serde_json::Value = serde_json::from_str(&json).unwrap();
    let problems = raw[PROBLEMS_KEY]
        .as_array()
        .unwrap_or_else(|| panic!("{json}"));
    assert!(
        problems
            .iter()
            .any(|p| p.as_str().is_some_and(|p| p.contains(NATIVE_SILENT_MARK))),
        "{json}"
    );
    // The agent was never stopped for it, and the line is gone once a file is recorded.
    assert_eq!(cleared.harness, HarnessState::Running);
    assert!(!after.contains(NATIVE_SILENT_MARK), "{after}");
}

/// The chat-mode run on its own terminal: a model request before any session file makes the line,
/// which goes to `host.log` once and never onto the terminal the harness draws on. The first
/// typed line makes the session file and clears it.
#[test]
fn the_silent_native_leg_is_said_in_the_log_while_the_harness_owns_the_terminal() {
    let rig = Rig::chat();
    let upstream = Upstream::start(&rig.runtime);
    rig.tell("upstream.pem", &upstream.root_pem());
    rig.tell("tui.call.sh", &upstream.curl(r#"{"n":1}"#));
    let pem = rig.out.join("upstream.pem").display().to_string();
    let mut run = rig.start_on_pty(&[
        ("GOALCUP_TEST_DISK", "4194304,0,50"),
        ("GOALCUP_TEST_LOCAL_WRAP_UP", "2000,100"),
        ("GOALCUP_TEST_UPSTREAM_ROOTS", &pem),
        ("GOALCUP_TEST_NATIVE_SILENCE", "1,0"),
    ]);
    run.wait_shown("tui: ready");
    let record = goalcup_cli::run::newest(&rig.cfg).unwrap().unwrap();
    let dir = goalcup_cli::run::run_dir(&rig.cfg, &record.run_id);
    run.type_line("/call");
    run.wait_shown("tui: called");
    let silent = wait_silent(&dir, Some(1));
    run.type_line("hello");
    run.wait_shown("tui: noted hello");
    wait_silent(&dir, None);
    run.type_line("/exit");
    let status = run.wait();
    let shown = run.shown();
    assert!(status.success(), "{status:?}\n{shown}");
    assert_eq!(
        rig.harness_file("curl.err").unwrap_or_default(),
        "",
        "the request reached the upstream"
    );
    assert_eq!(silent.harness, goalcup_cli::run::HarnessState::Running);
    let log = std::fs::read_to_string(dir.join(goalcup_cli::host_log::HOST_LOG_NAME)).unwrap();
    assert_eq!(log.matches(NATIVE_SILENT_MARK).count(), 1, "{log}");
    assert!(!shown.contains(NATIVE_SILENT_MARK), "{shown}");
}
