//! The run-scoped daemon, through the real binaries: `goalcup` starts `goalcup-daemon` when a
//! run needs one and no daemon answers, and that daemon exits once nothing is unsealed.
//!
//! Every row runs the `goalcup` binary of this test build against [`Rig::run_scoped`]: the
//! `m2-fake` development connector, its fake harness, and no daemon until the CLI starts one.
//! The daemon a row observes is found by its command line (`--config <cfg>/daemon.toml`), which
//! is unique to the row's temporary directory. Each row ends every daemon it started.
//!
//! macOS only: the daemon reserves a capture chain on APFS alone. Slow (a row waits out the
//! daemon's 30-second idle grace), so it is in the `full` tier.

use crate::common::m2_cli_support;
use crate::common::support;

use goalcup_protocol::capture::{CaptureRecord, GapReason, RunState};
use goalcup_protocol::control::SignedStatus;
use m2_cli_support::{FAKE_ID, Rig};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const RESERVE: u64 = 4 * 1024 * 1024;

/// `goalcup-daemon`'s `IDLE_GRACE`, and how much longer than it an exit may take to be seen
/// (one 500 ms idle poll, `stop`, and this file's own 100 ms poll).
const GRACE: Duration = Duration::from_secs(30);
const GRACE_SLACK: Duration = Duration::from_secs(10);

/// Ends every daemon started over this rig's configuration, however the row ends, so a failed
/// row leaves nothing behind for its 30-second grace.
struct Reaper(PathBuf);

impl Drop for Reaper {
    fn drop(&mut self) {
        for pid in daemons_for(&self.0) {
            // SAFETY: two integers; the pid was just seen running this row's daemon.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
}

fn reaper(rig: &Rig) -> Reaper {
    Reaper(rig.cfg.join(goalcup_cli::daemon::spawn::DAEMON_CONFIG_NAME))
}

/// The pids of every `goalcup-daemon --config <config>` on this machine.
fn daemons_for(config: &Path) -> Vec<i32> {
    let table = Command::new("/bin/ps")
        .args(["-axww", "-o", "pid=,command="])
        .output()
        .expect("ps");
    let needle = format!("--config {} ", config.display());
    String::from_utf8_lossy(&table.stdout)
        .lines()
        .filter(|line| line.contains("goalcup-daemon") && line.contains(&needle))
        .filter_map(|line| line.split_whitespace().next()?.parse().ok())
        .collect()
}

fn daemons(rig: &Rig) -> Vec<i32> {
    daemons_for(&rig.cfg.join(goalcup_cli::daemon::spawn::DAEMON_CONFIG_NAME))
}

/// The one daemon of this rig, waited for.
fn the_daemon(rig: &Rig) -> i32 {
    support::wait_for("the rig's one daemon", || match daemons(rig).as_slice() {
        [pid] => Some(*pid),
        _ => None,
    })
}

/// Wait until `pid` is gone, and say how long that took.
fn wait_gone(pid: i32, within: Duration) -> Duration {
    let began = Instant::now();
    while goalcup_protocol::process::is_alive(pid as u32) {
        assert!(
            began.elapsed() < within,
            "daemon {pid} is still running after {within:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    began.elapsed()
}

/// `goalcup` with only this rig's directories in its environment, and `HOME` a throwaway one.
fn goalcup(rig: &Rig, binary: &Path) -> Command {
    let home = rig.root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("PATH", rig.path())
        .env("HOME", home)
        .env("GOALCUP_CONFIG_DIR", &rig.cfg)
        .env("GOALCUP_CONNECTORS_DIR", &rig.capture_connectors)
        .stdin(Stdio::null());
    command
}

fn cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_goalcup"))
}

/// A goal file in a directory of its own (`goalcup start` creates the run directory beside it).
fn goal_in(rig: &Rig, dir: &str) -> PathBuf {
    let dir = rig.root.join(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let goal = dir.join("goal.txt");
    std::fs::write(&goal, "write hello.txt, then stop").unwrap();
    goal
}

fn start_command(rig: &Rig, binary: &Path, goal: &Path) -> Command {
    let mut command = goalcup(rig, binary);
    command
        .args(["start", "--harness", FAKE_ID, "--dev", "--goal-file"])
        .arg(goal)
        .args(["--capture-reserve-bytes", &RESERVE.to_string()]);
    command
}

fn said(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// `goalcup start` in the background, its output collected when it ends.
fn start_in_background(rig: &Rig, goal: &Path) -> Child {
    start_command(rig, &cli(), goal)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn latest_entry(rig: &Rig) -> String {
    support::wait_for("the entry record", || {
        goalcup_cli::entries::latest_entry(&rig.cfg).ok().flatten()
    })
    .entry_id
}

/// `goalcup status ENTRY --json`, parsed.
fn status(rig: &Rig, entry: &str) -> SignedStatus {
    let output = goalcup(rig, &cli())
        .args(["status", entry, "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", said(&output));
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The fake harness, launched and recording, with its one `sleep` running.
fn wait_running(rig: &Rig, entry: &str) {
    rig.wait_for("sleeper");
    support::wait_for("the run to be recording", || {
        status(rig, entry)
            .payload
            .capture
            .is_some_and(|c| c.run == RunState::Running)
            .then_some(())
    });
}

/// `goalcup start` on a machine with no daemon and no `daemon.toml`: it writes the default
/// configuration, starts a daemon, the run is recorded and sealed, and the daemon exits within
/// its grace of the seal, withdrawing `control.json`. Afterwards `status` and `entries` say no
/// daemon is running and start none, although `daemon.toml` would let them.
#[test]
fn start_with_no_daemon_starts_one_that_exits_after_the_seal() {
    let rig = Rig::run_scoped();
    let _reaper = reaper(&rig);
    assert!(daemons(&rig).is_empty());
    rig.tell("stop", "");

    let output = start_command(&rig, &cli(), &goal_in(&rig, "runs"))
        .output()
        .unwrap();
    let sealed_at = Instant::now();
    assert!(output.status.success(), "{}", said(&output));
    assert!(
        said(&output).contains("finished locally"),
        "{}",
        said(&output)
    );
    assert!(
        rig.cfg.join("daemon.toml").is_file(),
        "the default configuration was written"
    );
    let entry = latest_entry(&rig);
    let (report, _) = rig.read_trace(&entry);
    assert!(report.sealed, "{:?}", report.findings);

    // Still up just after the seal, then gone within the grace.
    let daemon = the_daemon(&rig);
    let took = wait_gone(daemon, GRACE + GRACE_SLACK);
    let after_seal = sealed_at.elapsed();
    assert!(
        after_seal >= GRACE - Duration::from_secs(2),
        "the daemon left {after_seal:?} after the seal, before its grace ({took:?} observed)"
    );
    assert!(
        !goalcup_cli::config::control_path(&rig.cfg).exists(),
        "an idle exit withdraws control.json"
    );

    for args in [
        vec!["status"],
        vec!["status", entry.as_str()],
        vec!["entries"],
    ] {
        let output = goalcup(&rig, &cli()).args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}: {}", said(&output));
        let text = String::from_utf8_lossy(&output.stderr);
        assert!(
            text.contains("no daemon is running")
                && text.contains(
                    &goalcup_cli::config::control_path(&rig.cfg)
                        .display()
                        .to_string()
                ),
            "{args:?} says so and names control.json: {text}"
        );
        assert!(daemons(&rig).is_empty(), "{args:?} started a daemon");
    }
}

/// Two `goalcup start` at the same moment, with no daemon: both runs are recorded and sealed,
/// and exactly one daemon holds them. The loser of a start race refuses at the registry lock
/// and its command uses the winner.
#[test]
fn two_concurrent_starts_end_with_exactly_one_daemon() {
    let rig = Rig::run_scoped();
    let _reaper = reaper(&rig);
    rig.tell("stop", "");
    let first = start_in_background(&rig, &goal_in(&rig, "runs-a"));
    let second = start_in_background(&rig, &goal_in(&rig, "runs-b"));
    for child in [first, second] {
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{}", said(&output));
    }
    assert_eq!(daemons(&rig).len(), 1, "one daemon: {:?}", daemons(&rig));
    let output = goalcup(&rig, &cli())
        .args(["entries", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", said(&output));
    let entries: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let entries = entries.as_array().unwrap();
    assert_eq!(
        entries.len(),
        2,
        "both runs are in the one daemon: {entries:?}"
    );
    assert!(
        entries.iter().all(|e| !e["sealed_at"].is_null()),
        "and both are sealed: {entries:?}"
    );
}

/// The daemon `goalcup` starts receives none of the CLI's environment: not a provider key, not a
/// proxy, not a `GOALCUP_*` variable. Observed by putting a shim in place of the daemon binary,
/// beside a copy of `goalcup`, that writes the environment it was given and then becomes the
/// real daemon, so the run goes on as usual.
#[test]
fn the_started_daemon_inherits_none_of_the_clis_secrets() {
    let rig = Rig::run_scoped();
    let _reaper = reaper(&rig);
    rig.tell("stop", "");
    let bin = rig.root.join("shim-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let copied = bin.join("goalcup");
    std::fs::copy(cli(), &copied).unwrap();
    let dump = rig.root.join("daemon-env");
    let shim = bin.join("goalcup-daemon");
    std::fs::write(
        &shim,
        format!(
            "#!/bin/sh\n/usr/bin/env > '{}'\nexec '{}' \"$@\"\n",
            dump.display(),
            support::daemon_binary().display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();

    let planted = [
        ("DEEPSEEK_API_KEY", "sk-planted-deepseek-0123456789abcdef"),
        ("AMP_API_KEY", "sgamp-planted-0123456789abcdef"),
        ("OPENAI_API_KEY", "sk-planted-openai-0123456789abcdef"),
        ("ANTHROPIC_API_KEY", "sk-ant-planted-0123456789abcdef"),
        ("HTTPS_PROXY", "http://planted-proxy.invalid:3128"),
        ("GOALCUP_PLANTED", "planted-goalcup-variable"),
    ];
    let mut command = start_command(&rig, &copied, &goal_in(&rig, "runs"));
    command.env("USER", "participant").env("TMPDIR", "/tmp/");
    for (name, value) in planted {
        command.env(name, value);
    }
    let output = command.output().unwrap();
    assert!(output.status.success(), "{}", said(&output));

    let env = std::fs::read_to_string(&dump).expect("the shim ran as the daemon");
    for (name, value) in planted {
        assert!(
            !env.contains(value),
            "{name}'s value reached the daemon:\n{env}"
        );
    }
    let names: std::collections::BTreeSet<&str> = env
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(n, _)| n)
        .collect();
    // What the CLI passes, plus what `/bin/sh` adds to its own environment before `env` runs.
    let allowed = [
        "HOME", "USER", "LOGNAME", "TMPDIR", "PATH", "PWD", "SHLVL", "_", "OLDPWD",
    ];
    assert!(
        names.iter().all(|n| allowed.contains(n)),
        "only the allowlist reached the daemon: {names:?}"
    );
    assert!(env.contains(&format!(
        "PATH={}\n",
        goalcup_cli::daemon::spawn::DAEMON_PATH
    )));
    assert!(env.contains(&format!("HOME={}\n", rig.root.join("home").display())));
    assert!(env.contains("USER=participant\n"));
    assert!(env.contains("PWD=/\n"), "it runs in `/`: {env}");
}

/// While a run is open, `goalcup status` and `goalcup seal` from a second terminal reach the
/// daemon the run started and start no other; the seal ends the run.
#[test]
fn status_and_seal_while_a_run_is_open_use_its_daemon() {
    let rig = Rig::run_scoped();
    let _reaper = reaper(&rig);
    let start = start_in_background(&rig, &goal_in(&rig, "runs"));
    let entry = latest_entry(&rig);
    wait_running(&rig, &entry);
    let daemon = the_daemon(&rig);

    let output = goalcup(&rig, &cli())
        .args(["seal", entry.as_str()])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", said(&output));
    let output = start.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", said(&output));
    assert_eq!(
        daemons(&rig),
        vec![daemon],
        "the same one daemon throughout"
    );
    let sealed = status(&rig, &entry);
    assert_eq!(
        sealed.payload.capture.map(|c| c.run),
        Some(RunState::Sealed)
    );
}

/// The crash campaign's documented outcome without a service to restart the daemon: a daemon
/// killed mid-run is started again by the launcher's own finish (H-14), which reopens the run
/// for status and seal only and seals it. `goalcup start` exits 0 "finished locally"; the chain
/// carries exactly one stream-less `IngestLost` gap, so it verifies sealed and incomplete.
#[test]
fn a_daemon_killed_mid_run_is_started_again_by_the_finish_and_the_run_seals_with_one_gap() {
    let rig = Rig::run_scoped();
    let _reaper = reaper(&rig);
    let start = start_in_background(&rig, &goal_in(&rig, "runs"));
    let entry = latest_entry(&rig);
    wait_running(&rig, &entry);
    let killed = the_daemon(&rig);
    // SAFETY: two integers; the pid was just seen running this row's daemon.
    assert_eq!(unsafe { libc::kill(killed, libc::SIGKILL) }, 0);
    wait_gone(killed, Duration::from_secs(10));

    // The harness ends on its own: its one `sleep` is killed, and it exits without `stop`.
    let sleeper: i32 = rig.harness_file("sleeper").unwrap().trim().parse().unwrap();
    // SAFETY: two integers; the fake harness wrote its own sleeper's pid.
    unsafe {
        libc::kill(sleeper, libc::SIGKILL);
    }
    let output = start.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", said(&output));
    assert!(
        said(&output).contains("finished locally"),
        "{}",
        said(&output)
    );

    let restarted = the_daemon(&rig);
    assert_ne!(restarted, killed, "a second daemon sealed the run");
    let (report, records) = rig.read_trace(&entry);
    assert!(report.sealed, "{:?}", report.findings);
    let lost = records
        .iter()
        .filter(|r| {
            matches!(
                r,
                CaptureRecord::Gap {
                    stream: None,
                    reason: GapReason::IngestLost,
                    lost_bytes: None,
                    ..
                }
            )
        })
        .count();
    assert_eq!(lost, 1, "one reopen, one IngestLost gap");
    assert_eq!(
        report.exit_code(),
        1,
        "sealed and incomplete: {:?}",
        report.findings
    );
}
