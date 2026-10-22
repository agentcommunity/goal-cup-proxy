//! A run whose harness writes no session file while the recording proxy records its model calls
//! (a silent native leg): `goalcup start` says so early, `goalcup status` keeps it as a
//! `PROBLEM:` line, and a session file clears it. Nothing is stopped or paused for it (D2).
//!
//! The rig's headless `m2-fake` connector: its native source is `{run_home}/native/*.jsonl`, which
//! the fake harness never writes, and its model host is a local upstream.

use crate::common::m2_cli_support::{Hooked, Rig, Upstream};
use std::time::{Duration, Instant};

const RESERVE: u64 = 4 * 1024 * 1024;

fn joined<T>(handle: std::thread::ScopedJoinHandle<'_, T>) -> T {
    handle.join().expect("the start thread panicked")
}

/// A harness that writes no session file while its model calls are recorded is told early: once
/// the wire has the calls and the time (here one call and no wait) and the native source has
/// admitted no file, `goalcup status` has a `PROBLEM:` line naming the calls, and `--json` has it
/// under `problems`. A session file clears it. Nothing is stopped or paused meanwhile.
#[test]
fn a_silent_native_leg_is_a_problem_until_a_session_file_appears() {
    use goalcup_cli::commands::status::{PROBLEMS_KEY, status_with};
    use goalcup_cli::run::{HarnessState, NativeSilence, read_state, run_dir};
    let rig = Rig::new();
    let upstream = Upstream::start(&rig.runtime);
    rig.tell("steps.sh", &upstream.curl(r#"{"n":1}"#));
    let goal = rig.run_folder();
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let mut opts = rig.options(&goal, RESERVE);
    opts.native_silence = NativeSilence {
        calls: 1,
        after: Duration::ZERO,
    };
    let status = |entry_id: &str, json: bool| {
        let mut out = Vec::new();
        let now = goalcup_protocol::control::format_z(goalcup_cli::run::now_unix());
        status_with(&http, &rig.cfg, Some(entry_id), json, &now, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    };
    let wait_state = |dir: &std::path::Path, silent: Option<u64>| {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(state) = read_state(dir)
                && state.native_silent == silent
            {
                return state;
            }
            assert!(Instant::now() < deadline, "native_silent never {silent:?}");
            std::thread::sleep(Duration::from_millis(25));
        }
    };
    let (started, silent, json, cleared, after) = std::thread::scope(|s| {
        let start = rig.start(s, &client, opts, Some(upstream.roots()));
        rig.wait_for("sleeper");
        let entry_id = client.created().entry_id;
        let dir = run_dir(&rig.cfg, &entry_id);
        wait_state(&dir, Some(1));
        let silent = status(&entry_id, false);
        let json = status(&entry_id, true);
        // The harness writes its session file under the native root (`{run_home}/native`).
        let env = rig.harness_file("env").unwrap();
        let home = env.lines().find_map(|l| l.strip_prefix("HOME=")).unwrap();
        let native = std::path::Path::new(home).join("native");
        std::fs::create_dir_all(&native).unwrap();
        std::fs::write(native.join("session.jsonl"), "{}\n").unwrap();
        let cleared = wait_state(&dir, None);
        let after = status(&entry_id, false);
        goalcup_cli::commands::seal::seal_with(
            &http,
            &rig.connectors,
            &rig.cfg,
            &entry_id,
            &mut Vec::new(),
        )
        .unwrap();
        (joined(start), silent, json, cleared, after)
    });
    let (result, out) = started;
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert_eq!(
        rig.harness_file("curl.err").unwrap_or_default(),
        "",
        "the call reached the upstream"
    );
    let problem = silent
        .lines()
        .find(|line| line.starts_with("PROBLEM:") && line.contains("session file"))
        .unwrap_or_else(|| panic!("{silent}"));
    assert!(problem.contains("1 model call"), "{problem}");
    let raw: serde_json::Value = serde_json::from_str(&json).unwrap();
    let problems = raw[PROBLEMS_KEY]
        .as_array()
        .unwrap_or_else(|| panic!("{json}"));
    assert!(
        problems
            .iter()
            .any(|p| p.as_str().is_some_and(|p| p.contains("session file"))),
        "{json}"
    );
    // The agent was never stopped for it, and the line is gone once a file is recorded.
    assert_eq!(cleared.harness, HarnessState::Running);
    assert!(!after.contains("session file"), "{after}");
}
