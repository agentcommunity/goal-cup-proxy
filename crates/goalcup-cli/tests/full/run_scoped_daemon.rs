//! The run-scoped daemon, through the real binaries: `goalcup` starts `goalcup-daemon` when a
//! run needs one and no daemon answers, and that daemon exits once nothing is unsealed.
//!
//! Every row runs the `goalcup` binary of this test build against [`Rig::run_scoped`]: the
//! `m2-fake` development connector, its fake harness, and no daemon until the CLI starts one.
//! The daemon a row observes is found by its command line (`--config <cfg>/daemon.toml`), which
//! is unique to the row's temporary directory. Each row ends every daemon it started.
//!
//! Runs on macOS (APFS) and on Linux (ext4 or XFS); the daemon refuses to reserve a capture
//! chain anywhere else. Slow (a row waits out the daemon's 30-second idle grace), so it is in
//! the `full` tier.

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
/// its grace of the seal, withdrawing `control.json`.
///
/// Afterwards `goalcup status` starts a daemon again and shows that run, sealed, out of the
/// registry; it creates no entry and changes nothing in the sealed trace; and the daemon it
/// started exits on the same grace.
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

    let trace_before = files_under(&rig.trace(&entry));
    let sealed = status(&rig, &entry);
    assert_eq!(
        sealed.payload.capture.as_ref().map(|c| c.run),
        Some(RunState::Sealed),
        "status shows the sealed run after its daemon exited"
    );
    let asked = Instant::now();
    let respawned = the_daemon(&rig);
    assert_ne!(respawned, daemon, "status started a daemon of its own");

    // The bare command and the text rendering name the same run, and say it is sealed.
    let output = goalcup(&rig, &cli()).arg("status").output().unwrap();
    assert!(output.status.success(), "{}", said(&output));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains(&entry) && text.contains("sealed"),
        "the newest run, sealed: {text}"
    );
    let output = goalcup(&rig, &cli())
        .args(["entries", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", said(&output));
    let listed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let listed = listed.as_array().unwrap();
    assert_eq!(listed.len(), 1, "no entry was created: {listed:?}");
    assert_eq!(listed[0]["entry_id"], entry.as_str());
    assert!(
        !listed[0]["sealed_at"].is_null(),
        "still sealed: {listed:?}"
    );
    assert_eq!(
        files_under(&rig.trace(&entry)),
        trace_before,
        "a look changes nothing in a sealed trace"
    );
    assert_eq!(
        daemons(&rig),
        vec![respawned],
        "one daemon answered all three"
    );

    wait_gone(respawned, GRACE + GRACE_SLACK);
    assert!(
        asked.elapsed() >= GRACE - Duration::from_secs(2),
        "the status daemon left before its grace"
    );
    assert!(
        !goalcup_cli::config::control_path(&rig.cfg).exists(),
        "an idle exit withdraws control.json"
    );
}

/// `goalcup status` and `goalcup entries` on a machine where no run was ever started: a plain
/// answer, and nothing written or started for it: no `daemon.toml`, no daemon.
#[test]
fn status_and_entries_with_no_runs_ever_answer_without_starting_anything() {
    let rig = Rig::run_scoped();
    let _reaper = reaper(&rig);
    let config = rig.cfg.join(goalcup_cli::daemon::spawn::DAEMON_CONFIG_NAME);
    assert!(daemons(&rig).is_empty());
    assert!(!config.exists());

    for args in [vec!["status"], vec!["status", "e0123456789abcdef"]] {
        let output = goalcup(&rig, &cli()).args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}: {}", said(&output));
        assert!(
            said(&output).contains("no run has been started from this machine")
                || said(&output).contains("e0123456789abcdef"),
            "{args:?}: a clear answer: {}",
            said(&output)
        );
    }
    let output = goalcup(&rig, &cli())
        .args(["entries", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", said(&output));
    let listed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(listed, serde_json::json!([]), "no entries");
    let output = goalcup(&rig, &cli()).arg("entries").output().unwrap();
    assert!(output.status.success(), "{}", said(&output));
    assert!(String::from_utf8_lossy(&output.stdout).contains("(none)"));

    // Long enough for a daemon a command had started to have shown itself.
    std::thread::sleep(Duration::from_millis(500));
    assert!(daemons(&rig).is_empty(), "no daemon was started");
    assert!(!config.exists(), "no daemon.toml was written");
    assert!(!goalcup_cli::config::control_path(&rig.cfg).exists());
}

/// The one write path a look may take. A run whose daemon was killed mid-run is unfinished, so
/// the daemon `goalcup status` starts reopens it for status and seal only and chains the lost
/// interval as exactly one `IngestLost` gap. That daemon then stays up past its 30-second grace,
/// because an unfinished capture is work, and `goalcup seal` meets the same daemon, finishes the
/// run and chains no second gap.
#[test]
fn status_after_a_mid_run_crash_reopens_the_run_keeps_its_daemon_up_and_seal_adds_no_gap() {
    let rig = Rig::run_scoped();
    let _reaper = reaper(&rig);
    let start = start_in_background(&rig, &goal_in(&rig, "runs"));
    let entry = latest_entry(&rig);
    wait_running(&rig, &entry);
    let killed = the_daemon(&rig);
    // SAFETY: two integers; the pid was just seen running this row's daemon.
    assert_eq!(unsafe { libc::kill(killed, libc::SIGKILL) }, 0);
    wait_gone(killed, Duration::from_secs(10));
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        daemons(&rig).is_empty(),
        "nothing but the look below restarts it while the harness runs"
    );

    let reopened = status(&rig, &entry);
    let capture = reopened.payload.capture.expect("a capture status");
    assert_ne!(capture.run, RunState::Sealed, "status does not seal it");
    let respawned = the_daemon(&rig);
    assert_ne!(respawned, killed, "status started a daemon");

    std::thread::sleep(GRACE + GRACE_SLACK);
    assert_eq!(
        daemons(&rig),
        vec![respawned],
        "the daemon holding the unfinished run is still up after its grace"
    );

    let output = goalcup(&rig, &cli())
        .args(["seal", entry.as_str()])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", said(&output));
    let _ = start.wait_with_output().unwrap();
    assert_eq!(daemons(&rig), vec![respawned], "seal met the same daemon");
    let sealed = status(&rig, &entry);
    assert_eq!(
        sealed.payload.capture.map(|c| c.run),
        Some(RunState::Sealed)
    );
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
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        lost, 1,
        "one reopen, one IngestLost gap, and none from the seal"
    );
}

/// Every file under `dir`, by relative path, with its bytes.
fn files_under(dir: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(here) = stack.pop() {
        for item in std::fs::read_dir(&here).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.insert(
                    path.strip_prefix(dir).unwrap().to_path_buf(),
                    std::fs::read(&path).unwrap(),
                );
            }
        }
    }
    out
}

/// Two `goalcup start` at the same moment, with no daemon: exactly one daemon holds whatever
/// they record. The loser of a start race refuses at the registry lock and its command uses the
/// winner. The fake connector shares one harness home, so the two runs go one at a time (D39):
/// a start that meets the other still recording is refused plainly and leaves no entry.
#[test]
fn two_concurrent_starts_end_with_exactly_one_daemon() {
    let rig = Rig::run_scoped();
    let _reaper = reaper(&rig);
    rig.tell("stop", "");
    let first = start_in_background(&rig, &goal_in(&rig, "runs-a"));
    let second = start_in_background(&rig, &goal_in(&rig, "runs-b"));
    let mut recorded = 0;
    for child in [first, second] {
        let output = child.wait_with_output().unwrap();
        if output.status.success() {
            recorded += 1;
        } else {
            assert!(
                said(&output).contains("already recording"),
                "the only refusal is the shared home's: {}",
                said(&output)
            );
        }
    }
    assert!(recorded >= 1, "at least one run is recorded");
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
        recorded,
        "every recorded run is in the one daemon: {entries:?}"
    );
    assert!(
        entries.iter().all(|e| !e["sealed_at"].is_null()),
        "and every one is sealed: {entries:?}"
    );
}

/// A daemon that refuses its configuration exits at once, and so does the command that started
/// it: refused (1), naming the daemon's exit status and `daemon.log`, well before the 10-second
/// wait for a daemon to answer runs out.
#[test]
fn a_daemon_that_refuses_its_configuration_fails_the_command_at_once() {
    let rig = Rig::run_scoped();
    let _reaper = reaper(&rig);
    std::fs::create_dir_all(&rig.cfg).unwrap();
    let config = rig.cfg.join(goalcup_cli::daemon::spawn::DAEMON_CONFIG_NAME);
    std::fs::write(&config, "this is not = = toml\n").unwrap();

    let began = Instant::now();
    let output = goalcup(&rig, &cli()).arg("seal").output().unwrap();
    let took = began.elapsed();
    let text = said(&output);
    assert_eq!(output.status.code(), Some(1), "refused: {text}");
    assert!(text.contains("exit status: 1"), "its exit status: {text}");
    let log = rig.cfg.join(goalcup_cli::daemon::spawn::DAEMON_LOG_NAME);
    assert!(
        text.contains(&log.display().to_string()),
        "and where its words are: {text}"
    );
    assert!(
        took < goalcup_cli::daemon::spawn::SPAWN_WAIT / 2,
        "without waiting for an answer that cannot come: {took:?}"
    );
}

/// The one early exit that is waited out: another process holds the registry lock (a daemon
/// starting in a race, or one stopping). The started daemon refuses at the lock and is started
/// again until the lock comes free, and then the run goes on.
#[test]
fn the_registry_lock_held_by_another_process_is_waited_out() {
    let rig = Rig::run_scoped();
    let _reaper = reaper(&rig);
    rig.tell("stop", "");
    let held = goalcup_daemon::registry::Registry::open(
        &goalcup_daemon::registry::Registry::default_path(&rig.config.artifact_root),
    )
    .expect("the registry lock, held by this test");
    let mut start = start_in_background(&rig, &goal_in(&rig, "runs"));
    let log = rig.cfg.join(goalcup_cli::daemon::spawn::DAEMON_LOG_NAME);
    support::wait_for("a started daemon to meet the held lock", || {
        std::fs::read_to_string(&log)
            .ok()
            .filter(|said| said.contains("another daemon holds the registry"))
    });
    // Long enough for the command to have seen that daemon exit and started another.
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        start.try_wait().unwrap().is_none(),
        "the command is still waiting for the lock, not refused"
    );
    drop(held);

    let output = start.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", said(&output));
}

/// The daemon `goalcup` starts receives none of the CLI's environment: not a provider key, not a
/// proxy, not a `GOALCUP_*` variable. Observed by putting a shim in place of the daemon binary,
/// beside a copy of `goalcup`, that writes the environment it was given and then becomes the
/// real daemon, so the run goes on as usual.
#[test]
fn the_started_daemon_inherits_none_of_the_clis_secrets_or_descriptors() {
    let rig = Rig::run_scoped();
    let _reaper = reaper(&rig);
    rig.tell("stop", "");
    let bin = rig.root.join("shim-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let copied = bin.join("goalcup");
    std::fs::copy(cli(), &copied).unwrap();
    let dump = rig.root.join("daemon-env");
    let fds = rig.root.join("daemon-fds");
    let shim = bin.join("goalcup-daemon");
    std::fs::write(
        &shim,
        format!(
            "#!/bin/sh\n/bin/ls /dev/fd > '{}'\n/usr/bin/env > '{}'\nexec '{}' \"$@\"\n",
            fds.display(),
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
    // A descriptor the CLI inherits without close-on-exec, as a careless parent would hand one
    // over: `dup2` onto PLANTED_FD in the CLI's child clears the flag.
    const PLANTED_FD: i32 = 47;
    let held = std::fs::File::create(rig.root.join("planted-descriptor")).unwrap();
    let raw = std::os::fd::AsRawFd::as_raw_fd(&held);
    // SAFETY: the closure runs in the forked child before `exec` and calls only `dup2`.
    unsafe {
        std::os::unix::process::CommandExt::pre_exec(&mut command, move || {
            if libc::dup2(raw, PLANTED_FD) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = command.output().unwrap();
    drop(held);
    assert!(output.status.success(), "{}", said(&output));
    let open: Vec<String> = std::fs::read_to_string(&fds)
        .expect("the shim listed its descriptors")
        .lines()
        .map(str::to_string)
        .collect();
    assert!(
        !open.contains(&PLANTED_FD.to_string()),
        "the CLI's inherited descriptor {PLANTED_FD} reached the daemon: {open:?}"
    );

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

/// The default `daemon.toml` names the connectors directory of the `goalcup start` that wrote
/// it, and binaries move. Here the first run's connectors directory is moved away:
///
/// - a `start` from the new directory while the first daemon still runs rewrites the unedited
///   default to name the new directory and refuses, naming the file, because that daemon reads
///   the old one;
/// - once that daemon is gone, the next `start` starts one from the rewritten file and the run
///   is recorded and sealed;
/// - a file someone edited is never rewritten: a `start` it disagrees with is refused naming
///   the file, and no daemon is asked anything.
#[test]
fn a_moved_connectors_directory_rewrites_the_default_config_and_refuses_an_edited_one() {
    let rig = Rig::run_scoped();
    let _reaper = reaper(&rig);
    rig.tell("stop", "");
    let config = rig.cfg.join(goalcup_cli::daemon::spawn::DAEMON_CONFIG_NAME);
    let output = start_command(&rig, &cli(), &goal_in(&rig, "runs-a"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", said(&output));
    let first = the_daemon(&rig);
    let named = |path: &Path| {
        std::fs::read_to_string(&config)
            .unwrap()
            .contains(&serde_json::to_string(path.to_str().unwrap()).unwrap())
    };
    assert!(
        named(&rig.capture_connectors),
        "the default names the first directory"
    );

    // The binaries' connectors moved: the old directory is gone.
    let moved = rig.root.join("connectors-moved");
    std::fs::rename(&rig.capture_connectors, &moved).unwrap();
    let from_moved = |goal: &str| {
        let mut command = start_command(&rig, &cli(), &goal_in(&rig, goal));
        command.env("GOALCUP_CONNECTORS_DIR", &moved);
        command.output().unwrap()
    };

    let output = from_moved("runs-b");
    assert_eq!(output.status.code(), Some(1), "{}", said(&output));
    assert!(
        said(&output).contains(&config.display().to_string()),
        "the refusal names the file: {}",
        said(&output)
    );
    assert!(
        named(&moved),
        "the unedited default now names the moved directory"
    );
    assert_eq!(
        daemons(&rig),
        vec![first],
        "and the running daemon was left alone"
    );

    // SAFETY: two integers; the pid was just seen running this row's daemon.
    assert_eq!(unsafe { libc::kill(first, libc::SIGTERM) }, 0);
    wait_gone(first, Duration::from_secs(15));
    let output = from_moved("runs-c");
    assert!(output.status.success(), "{}", said(&output));
    let (report, _) = rig.read_trace(&latest_entry(&rig));
    assert!(report.sealed, "{:?}", report.findings);
    let second = the_daemon(&rig);

    // Edited by hand: a comment, and a connectors_dir that is neither directory.
    let text = std::fs::read_to_string(&config).unwrap();
    let edited: String = text
        .lines()
        .map(|line| {
            if line.starts_with("connectors_dir") {
                "connectors_dir = \"/nonexistent/goalcup-connectors\" # mine".to_string()
            } else {
                line.to_string()
            }
        })
        .map(|line| line + "\n")
        .collect();
    std::fs::write(&config, &edited).unwrap();
    let output = from_moved("runs-d");
    assert_eq!(output.status.code(), Some(1), "{}", said(&output));
    let text = said(&output);
    assert!(
        text.contains(&config.display().to_string())
            && text.contains("/nonexistent/goalcup-connectors"),
        "the refusal names the file and what it says: {text}"
    );
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        edited,
        "an edited file is never rewritten"
    );
    assert_eq!(daemons(&rig), vec![second]);
}

// ------------------------------------------------------------------ slow is not dead

/// A `goalcup-daemon` that runs `script` first, in a directory of its own, then (if the script
/// reaches it) the real one: a daemon that is slow to answer, or that dies.
fn daemon_shim(rig: &Rig, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = rig.root.join("daemon-shim");
    std::fs::create_dir_all(&bin).unwrap();
    let shim = bin.join("goalcup-daemon");
    std::fs::write(
        &shim,
        format!(
            "#!/bin/sh\n{script}\nexec '{}' \"$@\"\n",
            support::daemon_binary().display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    shim
}

/// The waits of a command, scaled down: a dead daemon for 300 ms, a live one for 30 s, said
/// every 400 ms.
const QUICK_WAIT: goalcup_cli::daemon::spawn::Wait = goalcup_cli::daemon::spawn::Wait {
    dead: Duration::from_millis(300),
    reopening: Duration::from_secs(30),
    report_every: Duration::from_millis(400),
};

fn ensure_with(
    rig: &Rig,
    shim: &Path,
    wait: goalcup_cli::daemon::spawn::Wait,
    said: &mut Vec<String>,
) -> Result<goalcup_cli::daemon::HttpDaemonClient, goalcup_cli::CliError> {
    goalcup_cli::daemon::spawn::ensure_daemon_waiting(
        &rig.cfg,
        goalcup_cli::daemon::MissingConfig::WriteDefault {
            connectors: &rig.capture_connectors,
        },
        Some(shim),
        wait,
        &mut |line| said.push(line.to_string()),
    )
}

/// A daemon that is alive but has not yet written `control.json` (it is reopening a long run)
/// is waited for well past the wait a dead one gets, and the wait says so.
#[test]
fn a_daemon_still_reopening_is_waited_for() {
    let rig = Rig::run_scoped();
    let _reaper = reaper(&rig);
    let shim = daemon_shim(&rig, "sleep 2");
    let mut said = Vec::new();
    let began = Instant::now();
    let client = ensure_with(&rig, &shim, QUICK_WAIT, &mut said);
    let took = began.elapsed();
    assert!(client.is_ok(), "{:?}", client.err());
    assert!(
        took >= Duration::from_millis(1_500),
        "the 300 ms a dead daemon gets was not all it got: {took:?}"
    );
    assert!(
        said.len() >= 2
            && said
                .iter()
                .all(|line| line.starts_with("the daemon is reopening the run (")),
        "{said:?}"
    );
}

/// A daemon that is gone is not waited for: one that dies at once fails the command at once,
/// and one that dies while being waited for ends the wait there, not at the long budget.
#[test]
fn a_dead_daemon_is_not_waited_for() {
    let rig = Rig::run_scoped();
    let _reaper = reaper(&rig);

    let dead = daemon_shim(&rig, "exit 3");
    let mut said = Vec::new();
    let began = Instant::now();
    // Starting a process takes longer on a busy machine than a short report interval, so this
    // one is said no more than every five seconds: a report here would be a daemon waited for.
    let patient = goalcup_cli::daemon::spawn::Wait {
        report_every: Duration::from_secs(5),
        ..QUICK_WAIT
    };
    let error =
        ensure_with(&rig, &dead, patient, &mut said).expect_err("a dead daemon is an error");
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "{:?}",
        began.elapsed()
    );
    assert!(error.to_string().contains("exited"), "{error}");
    assert!(
        said.is_empty(),
        "nothing to wait for, nothing said: {said:?}"
    );

    // Alive for a while, then gone.
    let dies = daemon_shim(&rig, "sleep 1; exit 3");
    let mut said = Vec::new();
    let began = Instant::now();
    let error = ensure_with(&rig, &dies, QUICK_WAIT, &mut said)
        .expect_err("a daemon that dies is an error");
    let took = began.elapsed();
    assert!(
        took >= Duration::from_millis(900),
        "it was waited for while alive: {took:?}"
    );
    assert!(
        took < Duration::from_secs(10),
        "and not once it was gone: {took:?}"
    );
    assert!(error.to_string().contains("exited"), "{error}");
    assert!(!said.is_empty(), "the wait said it was waiting: {said:?}");
}
