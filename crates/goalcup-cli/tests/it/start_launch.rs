//! `goalcup start`'s launch half: the argv each connector's recipe renders to, the pins that
//! reach the child, and the two refusals that happen before anything is spawned.
//!
//! The second half of the file drives the command that uses the plan.

use crate::common::support;

// ------------------------------------------------------------- the command

/// Every daemon call this crate makes goes through the one `busy` retry.
///
/// A grep and not a type, because the rule is about call sites: `retry::retry_busy` is a
/// helper a command can simply forget to wrap a call in, and the failure mode of forgetting
/// is a command that dies on a transient the daemon expects every client to ride out.
///
/// **The sweep is over the whole of `src/`**, not `src/commands/` alone: a command is where
/// a daemon call is usually made, and a future unwrapped call outside it would pass a guard
/// whose name says "every". The wider walk costs nothing: every `client.` in this crate that is not in a
/// comment is a call site, wherever the file sits.
///
/// **It reads one line at a time, so the convention it enforces is one line per call site**:
/// a `client.` and the `retry_busy` wrapping it are written on the same line, `rustfmt`
/// permitting, and a call split across lines would pass this sweep while wrapped in nothing
/// at all. That is a real constraint on how these commands are written, and it is cheap —
/// every daemon call in this crate is a closure short enough to fit.
#[test]
fn no_daemon_call_in_this_crate_is_made_outside_the_busy_retry() {
    let mut seen = 0;
    for entry in support::walk("crates/goalcup-cli/src") {
        let source = std::fs::read_to_string(&entry).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        for (n, line) in lines.iter().copied().enumerate() {
            if !line.contains("client.") || line.trim_start().starts_with("//") {
                continue;
            }
            seen += 1;
            // The capture seal's own retry (`retry_capture_seal`, H-10 4a) has a closure that
            // builds its request from what remains of the budget before it calls, so its one
            // `client.seal_capture(` call sits inside a block that `retry_capture_seal(` opens
            // a few lines above: still exactly one call site, still inside the retry, and
            // nothing else may use this form.
            let capture_seal = line.trim_start().starts_with("client.seal_capture(")
                && lines[n.saturating_sub(12)..n]
                    .iter()
                    .any(|above| above.contains("retry_capture_seal("));
            assert!(
                line.contains("retry_busy") || capture_seal,
                "{}:{} calls the daemon outside the retry: {}",
                entry.display(),
                n + 1,
                line.trim()
            );
        }
    }
    assert!(seen > 0, "the sweep found no daemon call at all to check");
    // The walk stays over the whole of `src/`: the V1 arming gate that held the daemon calls
    // outside `src/commands/` left with D48, and a future call outside it is still checked.
}

/// The entry cache on a machine that has never started a run: `entries.rs` is shared with the
/// capture `status` and `seal`, which choose their entry through it.
#[test]
fn the_entry_cache_says_so_when_there_is_none() {
    let cfg = support::empty_cfg();
    assert!(
        goalcup_cli::entries::latest_entry(&cfg).unwrap().is_none(),
        "a machine that has never started a run is not an error"
    );
    let err = goalcup_cli::entries::read_entry(&cfg, "entry-nope").unwrap_err();
    assert_eq!(
        err.exit_code(),
        1,
        "naming an entry this machine never started is a refusal, not the CLI failing"
    );
}

/// The run's disk is one reservation held by the run, not a number the participant passes: the
/// flag that used to size each chain's preallocation is gone, and `--help` does not offer it.
#[test]
fn capture_reserve_bytes_is_no_longer_a_flag() {
    let goalcup = env!("CARGO_BIN_EXE_goalcup");
    let out = std::process::Command::new(goalcup)
        .args([
            "start",
            "--harness",
            "codex-cli",
            "--goal-file",
            "/nonexistent/goal.txt",
            "--capture-reserve-bytes",
            "1",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "a usage error");
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("--capture-reserve-bytes"), "{said}");
    assert!(said.contains("unexpected argument"), "{said}");
    let help = std::process::Command::new(goalcup)
        .args(["start", "--help"])
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&help.stdout).contains("capture-reserve"),
        "{}",
        String::from_utf8_lossy(&help.stdout)
    );
}

/// A run whose reservation the volume cannot hold is refused before anything is created: the
/// daemon is not asked, there is no run directory beside the goal file, no run record, and no
/// reserve file.
#[test]
fn a_run_that_cannot_take_its_reserve_is_refused_before_anything_is_created() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("cfg");
    std::fs::create_dir(&cfg).unwrap();
    let beside = tmp.path().join("goal-folder");
    std::fs::create_dir(&beside).unwrap();
    let goal = beside.join("goal.txt");
    std::fs::write(&goal, "write result.txt").unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    let mut o = support::start_options("codex-cli");
    o.goal_file = Some(goal);
    // More than any volume has, and a margin on top.
    o.disk.reserve_bytes = 1 << 60;
    o.disk.start_margin = 1 << 30;
    let err = goalcup_cli::commands::start::start_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &support::clean_env(),
        &o,
        &mut Vec::new(),
    )
    .expect_err("no volume holds that");
    assert_eq!(err.exit_code(), 1, "{err}");
    assert!(err.to_string().contains("this run needs"), "{err}");
    assert!(
        m.calls().is_empty(),
        "the daemon was asked: {:?}",
        m.calls()
    );
    let beside_goal: Vec<_> = std::fs::read_dir(&beside).unwrap().flatten().collect();
    assert_eq!(beside_goal.len(), 1, "only the goal file: {beside_goal:?}");
    assert!(!cfg.join("runs").exists(), "no run record");
    let reserve = goalcup_cli::run::reserve::artifact_root(&cfg).join("reserve");
    assert!(!reserve.exists(), "no reserve file or folder");
}
