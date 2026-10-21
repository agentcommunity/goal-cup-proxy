//! `goalcup start --goal-file` and `goalcup seal` on a protocol-v5 local capture: the real
//! daemon in process, the real proxy and IPC sink inside `start_with`, and a fake headless
//! harness (`fixtures/m2-fake-harness.sh`) that records what it was started with.
//!
//! macOS and Linux (the trace folder must be on APFS, ext4 or XFS). Every test holds the
//! process-table lock: each runs a daemon, most spawn a harness, and one lowers this
//! process's open-file limit.

#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "common/m2_cli_support/mod.rs"]
mod m2_cli_support;
#[path = "common/m2_common/mod.rs"]
mod m2_common;
#[path = "common/support/mod.rs"]
mod support;

#[path = "../../goalcup-daemon/tests/common/support/table_lock.rs"]
mod table_lock;

use goalcup_protocol::capture::{CaptureRecord, CaptureSource, RunState};
use goalcup_protocol::capture_status::AnchorState;
use goalcup_protocol::control::{ControlError, DaemonClient};
use goalcup_protocol::provenance::TerminationOutcomeV1;
use m2_cli_support::{FAKE_ID, Hooked, LaunchFault, RegistrationFault, Rig, Upstream};
use std::time::{Duration, Instant};

const GOAL: &str = "write hello.txt, then stop";
const RESERVE: u64 = 4 * 1024 * 1024;

/// A test build's short wrap-up for a run without a server: two seconds, and the seal asked
/// again after 100 ms (doubling) instead of the shipped 30 minutes and 10 s.
const SHORT_WRAP_UP: (&str, &str) = (goalcup_cli::upload::policy::TEST_LOCAL_WRAP_UP, "2000,100");

fn joined<T>(handle: std::thread::ScopedJoinHandle<'_, T>) -> T {
    handle.join().expect("the start thread panicked")
}

/// Every `entries/*.json` this machine wrote, as one string.
fn records(rig: &Rig) -> String {
    let mut text = String::new();
    for entry in std::fs::read_dir(rig.cfg.join("entries"))
        .into_iter()
        .flatten()
        .flatten()
    {
        text.push_str(&std::fs::read_to_string(entry.path()).unwrap());
    }
    text
}

fn count(records: &[CaptureRecord], pick: impl Fn(&CaptureRecord) -> bool) -> usize {
    records.iter().filter(|r| pick(r)).count()
}

/// This process's soft and hard open-file limits.
fn open_files() -> (u64, u64) {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` writes the one struct it is handed.
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
        0
    );
    (limit.rlim_cur, limit.rlim_max)
}

/// Puts the soft open-file limit back where it was, however the test ends.
struct OpenFiles(u64);

impl Drop for OpenFiles {
    fn drop(&mut self) {
        let limit = libc::rlimit {
            rlim_cur: self.0,
            rlim_max: open_files().1,
        };
        // SAFETY: `setrlimit` reads the one struct it is handed.
        unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) };
    }
}

#[test]
fn the_proxy_and_the_admitted_producer_precede_the_spawn_and_the_goal_is_one_argument() {
    let _lock = table_lock::TableLock::acquire();
    // launchd's default, which leaves the proxy about 120 connections.
    let (soft, hard) = open_files();
    let _restore = OpenFiles(soft);
    let launchd = libc::rlimit {
        rlim_cur: 256,
        rlim_max: hard,
    };
    // SAFETY: `setrlimit` reads the one struct it is handed.
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &launchd) }, 0);
    let rig = Rig::new();
    rig.tell("stop", "");
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) =
        std::thread::scope(|s| joined(rig.start(s, &client, rig.options(&goal, RESERVE), None)));
    let outcome = result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(
        outcome.exit_status.expect("a harness ran").success(),
        "{out}"
    );
    // The participant is told how the harness ended, after the seal block; a run that names no
    // provider key has no key scan to report.
    assert!(out.contains("harness exited: exit code 0."), "{out}");
    assert!(!out.contains("key scan:"), "{out}");
    assert!(
        out.find("finished locally").unwrap() < out.find("harness exited:").unwrap(),
        "{out}"
    );
    // A clean run's proxy line is two counters, never the whole table of zeros.
    let admission = out
        .lines()
        .find(|line| line.starts_with("proxy admission:"))
        .unwrap_or_else(|| panic!("{out}"));
    assert!(
        admission.starts_with("proxy admission: accepted="),
        "{admission}"
    );
    assert!(admission.contains(" intercepted="), "{admission}");
    assert!(
        !admission.contains("auth_refused=") && !admission.contains("upstream_"),
        "{admission}"
    );
    assert!(!out.contains("proxy upstream outage"), "{out}");

    let calls = client.book().calls.clone();
    let at = |name: &str| {
        calls
            .iter()
            .position(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("no {name}: {calls:?}"))
    };
    assert_eq!(at("create_capture_workspace"), 0, "{calls:?}");
    assert!(at("status") < at("record_capture_goal"), "{calls:?}");
    assert!(
        at("record_capture_goal") < at("register_process"),
        "{calls:?}"
    );
    assert!(
        at("register_process") < at("record_capture_launch"),
        "{calls:?}"
    );
    assert!(
        at("record_capture_launch") < at("seal_capture"),
        "{calls:?}"
    );
    for (name, started) in &calls[..at("register_process")] {
        assert!(!started, "{name} came after the harness started: {calls:?}");
    }
    let harness_began = std::fs::metadata(rig.out.join("pid"))
        .unwrap()
        .modified()
        .unwrap();
    assert!(
        client.book().goal_answered.unwrap() < harness_began,
        "the goal was recorded before the harness began"
    );
    let admission = client.book().admission.clone();
    assert_eq!(
        admission.last().copied(),
        Some((Some(1), Some(0))),
        "the spawn followed a status showing the entry's own producer admitted"
    );
    assert!(admission.iter().all(|(_, refused)| *refused == Some(0)));
    assert_eq!(client.book().creates[0].reserve_bytes, RESERVE);
    let create = serde_json::to_value(&client.book().creates[0]).unwrap();
    assert_eq!(
        create["connector_dir_sha256"],
        goalcup_protocol::connector_dir::connector_dir_sha256(
            &rig.capture_connectors.join(FAKE_ID)
        )
        .unwrap(),
        "the request carries the directory digest from the connector the CLI loaded"
    );
    let wanted = (2 * goalcup_cli::proxy::MAX_CONNECTION_TASKS).min(hard);
    assert!(
        open_files().0 >= wanted,
        "the soft open-file limit was raised"
    );
    assert_eq!(rig.harness_file("proxy").as_deref(), Some("listening\n"));

    let argv = rig.harness_file("argv").unwrap();
    let argv: Vec<&str> = argv.lines().collect();
    assert_eq!(
        argv.last(),
        Some(&GOAL),
        "the goal is the last argument: {argv:?}"
    );
    assert_eq!(
        argv.iter().filter(|a| a.contains(GOAL)).count(),
        1,
        "{argv:?}"
    );
    assert_eq!(argv[..argv.len() - 1].last(), Some(&"--"), "{argv:?}");
    assert_eq!(
        rig.harness_file("stdin").as_deref(),
        Some("eof\n"),
        "no input"
    );

    let env = rig.harness_file("env").unwrap();
    let secret = client.created().ingest_secret;
    let bearer = goalcup_cli::config::read_control_file(&rig.cfg)
        .unwrap()
        .token;
    let proxy_url = env
        .lines()
        .find_map(|l| l.strip_prefix("HTTPS_PROXY="))
        .expect("the harness is pointed at the proxy");
    let credential = proxy_url
        .split_once('@')
        .and_then(|(userinfo, _)| userinfo.rsplit_once(':'))
        .map(|(_, credential)| credential)
        .unwrap();
    let argv = rig.harness_file("argv").unwrap();
    for (name, value) in [("ingest secret", &secret), ("daemon bearer", &bearer)] {
        assert!(!argv.contains(value.as_str()), "the {name} reached argv");
        assert!(
            !env.contains(value.as_str()),
            "the {name} reached the harness"
        );
    }
    let record = records(&rig);
    for (name, value) in [
        ("ingest secret", secret.as_str()),
        ("proxy credential", credential),
        ("goal", GOAL),
    ] {
        assert!(!out.contains(value), "the {name} was printed:\n{out}");
        assert!(
            !record.contains(value),
            "the {name} was written down: {record}"
        );
    }
}

#[test]
fn local_pilot_never_requests_receipt() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    rig.tell("stop", "");
    // No `grant.json`, the file the V1 `goalcup auth` wrote: the pilot runs without one.
    assert!(!rig.cfg.join("grant.json").exists(), "no grant");
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) =
        std::thread::scope(|s| joined(rig.start(s, &client, rig.options(&goal, RESERVE), None)));
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    // A run with no `goalcup login` is local: it leaves no server configuration and no upload
    // spool behind. (It cannot dial anything: a start only learns a server from `server.json`.)
    assert!(!rig.cfg.join("server.json").exists(), "a server appeared");
    assert!(
        !rig.cfg.join(goalcup_cli::upload::UPLOADS_DIR_NAME).exists(),
        "a local run made an upload spool"
    );

    let created = client.created();
    let entry_id = created.entry_id;
    let trace = rig.trace(&entry_id);
    assert!(!trace.join("receipt.json").exists());
    let (report, records) = rig.read_trace(&entry_id);
    let launch_clock = records.iter().find_map(|r| match r {
        CaptureRecord::Launch { observed, .. } => Some(observed.clock_id.clone()),
        _ => None,
    });
    assert_eq!(
        launch_clock,
        Some(created.producer_id),
        "the launch is observed on the clock the proxy's first producer stamps its frames with"
    );
    assert_eq!(report.anchor, AnchorState::LocalOnly);
    assert!(report.sealed, "{:?}", report.findings);
    assert_eq!(report.findings, Vec::<String>::new());
    assert_eq!(report.exit_code(), 0);
    assert!(out.contains("local only"), "{out}");
    assert!(
        out.contains(&format!(
            "goalcup-verify --capture-integrity {}",
            trace.display()
        )),
        "{out}"
    );
}

/// The reserve files in the artifact root `goalcup start` used for this rig.
fn reserve_files(rig: &Rig) -> Vec<std::fs::DirEntry> {
    let dir = goalcup_cli::run::reserve::artifact_root(&rig.cfg).join("reserve");
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .collect()
}

#[test]
fn a_run_holds_one_reserve_file_and_releases_it_at_the_end() {
    use std::os::unix::fs::MetadataExt;
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (started, held, record) = std::thread::scope(|s| {
        let start = rig.start(s, &client, rig.options(&goal, RESERVE), None);
        rig.wait_for("sleeper");
        let entry_id = client.created().entry_id;
        let held: Vec<(String, u64)> = reserve_files(&rig)
            .iter()
            .map(|f| {
                (
                    f.file_name().to_string_lossy().into_owned(),
                    f.metadata().unwrap().blocks() * 512,
                )
            })
            .collect();
        let record = goalcup_cli::run::read(&rig.cfg, &entry_id);
        // Ending the run through the daemon ends the harness.
        let mut sealed = Vec::new();
        goalcup_cli::commands::seal::seal_with(
            &http,
            &rig.connectors,
            &rig.cfg,
            &entry_id,
            &mut sealed,
        )
        .unwrap();
        (joined(start), held, record)
    });
    let (result, out) = started;
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    let entry_id = client.created().entry_id;
    // One file, named for the run (a local run's id is its entry's), holding its blocks.
    assert_eq!(held.len(), 1, "{held:?}");
    assert_eq!(held[0].0, entry_id);
    assert!(held[0].1 >= RESERVE, "{held:?}");
    // The record the run left: no server, no deadline, the entry and its trace folder.
    let record = record.expect("a run record while the run records");
    assert_eq!(record.run_id, entry_id);
    assert_eq!((record.server_url, record.deadline_at), (None, None));
    assert_eq!(record.connector_id, FAKE_ID);
    assert_eq!(record.entries.len(), 1);
    assert_eq!(record.entries[0].entry_id, entry_id);
    assert_eq!(
        goalcup_cli::run::of_entry(&rig.cfg, &entry_id)
            .unwrap()
            .map(|r| r.run_id),
        Some(entry_id.clone())
    );
    assert_eq!(
        goalcup_cli::run::newest(&rig.cfg)
            .unwrap()
            .map(|r| r.run_id),
        Some(entry_id)
    );
    // And at the run's end the reservation is gone.
    assert!(reserve_files(&rig).is_empty(), "{:?}", reserve_files(&rig));
}

/// D65: a full disk never ends the run. The disk always reads as full (a floor above any free
/// space) and a local run uploads nothing, which once stopped it; now the watch only resizes the
/// reserve and warns, and the run ends when it is sealed.
#[test]
fn a_full_disk_with_no_upload_never_ends_the_run() {
    use goalcup_cli::run::{HarnessState, read_state, run_dir};
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let mut opts = rig.options(&goal, RESERVE);
    opts.disk.free_floor = u64::MAX / 2;
    let (started, during) = std::thread::scope(|s| {
        let start = rig.start(s, &client, opts, None);
        rig.wait_for("sleeper");
        let entry_id = client.created().entry_id;
        // Twenty ticks of a full disk.
        std::thread::sleep(Duration::from_secs(1));
        let during = read_state(&run_dir(&rig.cfg, &entry_id));
        goalcup_cli::commands::seal::seal_with(
            &http,
            &rig.connectors,
            &rig.cfg,
            &entry_id,
            &mut Vec::new(),
        )
        .unwrap();
        (joined(start), during)
    });
    let (result, out) = started;
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    let during = during.expect("the run's state while it records");
    assert_eq!(during.harness, HarnessState::Running, "{out}");
    assert_eq!(during.stopped, None, "{out}");
    assert!(!out.contains("disk is nearly full"), "{out}");
    let entry_id = client.created().entry_id;
    let (report, _) = rig.read_trace(&entry_id);
    assert!(report.sealed, "{:?}", report.findings);
}

/// The run's `state.json` follows the run: the launch writes it, the tick fills in how the
/// recording is, and the host's last word says it ended. A sleep lock that cannot be taken is
/// said as a `PROBLEM:` line first of all by `goalcup status`, and never refuses or stops the
/// run.
#[test]
fn a_sleep_lock_that_cannot_be_taken_is_a_problem_line_and_the_run_goes_on() {
    use goalcup_cli::run::{HarnessState, Recording, read_state, run_dir};
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let mut opts = rig.options(&goal, RESERVE);
    opts.sleep_lock = |_| Err("no sleep inhibitor on this machine".to_string());
    let (started, running, status) = std::thread::scope(|s| {
        let start = rig.start(s, &client, opts, None);
        rig.wait_for("sleeper");
        let entry_id = client.created().entry_id;
        let dir = run_dir(&rig.cfg, &entry_id);
        // The tick asks the daemon at once and every tick after: the recording is known soon.
        let deadline = Instant::now() + Duration::from_secs(30);
        let running = loop {
            let state = read_state(&dir);
            if let Some(state) = state
                && state.recording == Recording::Ok
            {
                break state;
            }
            assert!(Instant::now() < deadline, "no recording state in {dir:?}");
            std::thread::sleep(Duration::from_millis(25));
        };
        let mut status = Vec::new();
        goalcup_cli::commands::status::status_with(
            &http,
            &rig.cfg,
            Some(&entry_id),
            false,
            &goalcup_protocol::control::format_z(goalcup_cli::run::now_unix()),
            &mut status,
        )
        .unwrap();
        // Ending the run through the daemon ends the harness.
        goalcup_cli::commands::seal::seal_with(
            &http,
            &rig.connectors,
            &rig.cfg,
            &entry_id,
            &mut Vec::new(),
        )
        .unwrap();
        (joined(start), running, String::from_utf8(status).unwrap())
    });
    let (result, out) = started;
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert_eq!(running.harness, HarnessState::Running);
    assert_eq!(
        running.sleep_lock,
        Err("no sleep inhibitor on this machine".to_string())
    );
    // The participant block comes first; the one problem line names the cause and the fix.
    let problem = status
        .lines()
        .find(|line| line.starts_with("PROBLEM:"))
        .unwrap_or_else(|| panic!("{status}"));
    assert!(problem.contains("may go to sleep"), "{status}");
    assert!(
        problem.contains("no sleep inhibitor on this machine"),
        "{status}"
    );
    assert!(problem.contains("power"), "{status}");
    assert_eq!(status.matches("PROBLEM:").count(), 1, "{status}");
    assert!(
        status.find("PROBLEM:").unwrap() > status.find("Problems: 1").unwrap(),
        "{status}"
    );
    // The host's last word: the agent's process is gone, the run is over.
    let entry_id = client.created().entry_id;
    let last = read_state(&run_dir(&rig.cfg, &entry_id)).expect("the final state");
    assert!(last.ended, "{last:?}");
    assert!(
        matches!(last.harness, HarnessState::Exited { .. }),
        "{last:?}"
    );
    assert!(reserve_files(&rig).is_empty());
}

#[test]
fn a_stale_reserve_is_released_by_the_next_start() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    let root = goalcup_cli::run::reserve::artifact_root(&rig.cfg);
    // An earlier server run, past its deadline by more than a day, still holding its reserve;
    // and one whose deadline has not come.
    let long_gone = goalcup_protocol::control::format_z(1_000);
    let ahead = goalcup_protocol::control::format_z(goalcup_cli::run::now_unix() + 3_600);
    for (id, deadline) in [
        ("r0000000000000001", long_gone),
        ("r0000000000000002", ahead),
    ] {
        goalcup_cli::run::write(
            &rig.cfg,
            &goalcup_cli::run::RunRecord {
                run_id: id.to_string(),
                server_url: Some("https://server.example".to_string()),
                practice: false,
                test: false,
                connector_id: FAKE_ID.to_string(),
                workspace: rig.root.join("old"),
                started_at_unix: 1,
                deadline_at: Some(deadline),
                entries: Vec::new(),
            },
        )
        .unwrap();
        goalcup_cli::run::reserve::RunReserve::take(
            &goalcup_cli::run::reserve::reserve_path(&root, id),
            &support::disk_of(RESERVE),
        )
        .unwrap();
    }
    // Two local runs: one whose `goalcup start` was killed (its file is there, nothing holds the
    // lock), and one still running (this test holds the lock).
    let mut held = Vec::new();
    for (id, killed) in [("r0000000000000003", true), ("r0000000000000004", false)] {
        goalcup_cli::run::write(
            &rig.cfg,
            &goalcup_cli::run::RunRecord {
                run_id: id.to_string(),
                server_url: None,
                practice: false,
                test: false,
                connector_id: FAKE_ID.to_string(),
                workspace: rig.root.join("old"),
                started_at_unix: 1,
                deadline_at: None,
                entries: Vec::new(),
            },
        )
        .unwrap();
        let reserve = goalcup_cli::run::reserve::RunReserve::take(
            &goalcup_cli::run::reserve::reserve_path(&root, id),
            &support::disk_of(RESERVE),
        )
        .unwrap();
        if !killed {
            held.push(reserve);
        }
    }
    assert_eq!(reserve_files(&rig).len(), 4);

    let goal = rig.goal(GOAL);
    rig.tell("stop", "");
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) =
        std::thread::scope(|s| joined(rig.start(s, &client, rig.options(&goal, RESERVE), None)));
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    let left: Vec<String> = reserve_files(&rig)
        .iter()
        .map(|f| f.file_name().to_string_lossy().into_owned())
        .collect();
    let mut left = left;
    left.sort();
    assert_eq!(
        left,
        vec![
            "r0000000000000002".to_string(),
            "r0000000000000004".to_string()
        ],
        "the server run before its deadline, and the local run that is still held"
    );
    drop(held);
}

#[test]
fn local_owned_stop_and_seal_need_no_anchor() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (started, sealed) = std::thread::scope(|s| {
        let start = rig.start(s, &client, rig.options(&goal, RESERVE), None);
        rig.wait_for("sleeper");
        support::wait_for("the launch", || {
            (client.count("record_capture_launch") == 1
                && http.status(&client.created().entry_id).is_ok_and(|s| {
                    s.payload
                        .capture
                        .is_some_and(|c| c.run == RunState::Running)
                }))
            .then_some(())
        });
        // A Ctrl-C typed at the terminal reaches the harness: it stays in the launcher's
        // process group.
        let pid = rig.harness_pid().unwrap() as i32;
        // SAFETY: two process-group reads of integers.
        assert_eq!(unsafe { libc::getpgid(pid) }, unsafe { libc::getpgrp() });
        let mut out = Vec::new();
        let sealed = goalcup_cli::commands::seal::seal_with(
            &http,
            &rig.connectors,
            &rig.cfg,
            &client.created().entry_id,
            &mut out,
        )
        .map(|()| String::from_utf8(out).unwrap());
        (joined(start), sealed)
    });
    let sealed = sealed.unwrap();
    let (result, out) = started;
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(
        rig.harness_file("done").is_none(),
        "the tracker ended the harness"
    );
    let pid = rig.harness_pid().unwrap();
    assert!(!goalcup_protocol::process::is_alive(pid));
    assert!(sealed.contains("local only"), "{sealed}");
    assert!(sealed.contains("--capture-integrity"), "{sealed}");

    let entry_id = client.created().entry_id;
    let (report, _) = rig.read_trace(&entry_id);
    assert!(report.sealed, "{:?}", report.findings);
    assert_eq!(report.anchor, AnchorState::LocalOnly);
    let terminal = report.terminal.expect("a local finish");
    assert!(
        matches!(
            terminal.termination,
            Some(TerminationOutcomeV1::Confirmed { .. })
        ),
        "the tracker ended its own harness and saw it gone: {:?}",
        terminal.termination
    );
}

#[test]
fn a_goal_file_without_a_capture_profile_refuses_before_anything_is_created() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::with_capture(false);
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) =
        std::thread::scope(|s| joined(rig.start(s, &client, rig.options(&goal, RESERVE), None)));
    let error = result.expect_err("no capture profile, no run");
    assert_eq!(error.exit_code(), 1, "{error}");
    assert!(
        format!("{error}").contains("no harness connector"),
        "{error}\n{out}"
    );
    assert!(client.names().is_empty(), "{:?}", client.names());
    assert!(rig.run_dirs().is_empty());
    assert!(!rig.harness_started());
}

#[test]
fn invalid_goals_refuse_before_any_capture_side_effect() {
    let _lock = table_lock::TableLock::acquire();
    let cases: [(&str, &[u8], &str); 4] = [
        ("empty", b"", "empty or only white space"),
        ("white space", b" \t\r\n", "empty or only white space"),
        ("nul", b"do one thing\0then another", "holds a NUL byte"),
        ("non UTF-8", b"goal \xff", "is not valid UTF-8"),
    ];
    for (name, bytes, reason) in cases {
        let rig = Rig::new();
        let goal = rig.root.join("runs").join("goal.txt");
        std::fs::create_dir_all(goal.parent().unwrap()).unwrap();
        std::fs::write(&goal, bytes).unwrap();
        let http = rig.client();
        let client = Hooked::new(&http, &rig);
        let (result, out) = std::thread::scope(|scope| {
            joined(rig.start(scope, &client, rig.options(&goal, RESERVE), None))
        });
        let error = result.expect_err(name);
        assert_eq!(error.exit_code(), 1, "{name}: {error}");
        assert!(error.to_string().contains(reason), "{name}: {error}\n{out}");
        assert!(client.names().is_empty(), "{name}: {:?}", client.names());
        assert!(rig.run_dirs().is_empty(), "{name}: a run home was created");
        assert!(!rig.harness_started(), "{name}: the harness was spawned");
        let homes = rig.cfg.join("capture-homes");
        assert!(!homes.exists(), "{name}: {} was created", homes.display());
        assert!(
            !homes.join(FAKE_ID).exists(),
            "{name}: the shared harness home was created"
        );
    }
}

/// A provider host that cannot be reached refuses the start before the daemon is asked for
/// anything: the refusal names the host, the failure subtype and this binary.
#[test]
fn an_unreachable_provider_host_refuses_the_start_before_anything_is_created() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    let manifest = rig.capture_connectors.join(FAKE_ID).join("harness.toml");
    let source = std::fs::read_to_string(&manifest).unwrap();
    assert!(source.contains("host = \"localhost\""));
    std::fs::write(
        &manifest,
        source.replacen(
            "host = \"localhost\"",
            "host = \"preflight-unreachable.invalid\"",
            1,
        ),
    )
    .unwrap();
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) =
        std::thread::scope(|s| joined(rig.start(s, &client, rig.options(&goal, RESERVE), None)));
    let error = result.expect_err("the provider host does not resolve");
    assert_eq!(error.exit_code(), 1, "{error}\n{out}");
    let said = error.to_string();
    assert!(said.contains("preflight-unreachable.invalid:443"), "{said}");
    assert!(said.contains("the DNS lookup failed"), "{said}");
    assert!(said.contains("Little Snitch"), "{said}");
    let binary = std::env::current_exe().unwrap();
    assert!(said.contains(&binary.display().to_string()), "{said}");
    assert!(client.names().is_empty(), "{:?}", client.names());
    assert!(rig.run_dirs().is_empty());
    assert!(!rig.harness_started());
    assert!(!rig.cfg.join("capture-homes").exists());
}

#[test]
fn an_unpinned_connector_without_dev_is_refused_before_anything_is_created() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    // Without `--dev` the directory is checked as a pinned one, and a pinned manifest may not
    // name a loopback host: the fixture's `localhost` would be refused by that rule (exit 3)
    // before the missing `--dev` is reached. A non-loopback test name leaves the pin refusal as
    // the only one this test can see.
    let manifest = rig.capture_connectors.join(FAKE_ID).join("harness.toml");
    let source = std::fs::read_to_string(&manifest).unwrap();
    assert!(source.contains("host = \"localhost\""));
    std::fs::write(
        &manifest,
        source.replacen(
            "host = \"localhost\"",
            "host = \"unpinned-fixture.invalid\"",
            1,
        ),
    )
    .unwrap();
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let mut opts = rig.options(&goal, RESERVE);
    opts.dev = false;
    let (result, out) = std::thread::scope(|s| joined(rig.start(s, &client, opts, None)));
    let error = result.expect_err("no build pins this connector, and `--dev` was not typed");
    assert_eq!(error.exit_code(), 1, "{error}\n{out}");
    assert!(format!("{error}").contains("--dev"), "{error}");
    assert!(client.names().is_empty(), "{:?}", client.names());
    assert!(rig.run_dirs().is_empty());
    assert!(!rig.harness_started());
}

/// Every shipped connector in place of the fake, the first of them in a folder named `alias`;
/// returns that connector's id.
fn replace_fake_with_aliased_shipped_connector(rig: &Rig, alter_bytes: bool) -> String {
    std::fs::remove_dir_all(rig.connectors.join(FAKE_ID)).unwrap();
    support::copy_connectors_into(&rig.connectors);
    let id = goalcup_protocol::pins::shipped_connector_ids()[0].clone();
    let alias = rig.connectors.join("alias");
    std::fs::rename(rig.connectors.join(&id), &alias).unwrap();
    if alter_bytes {
        let manifest = alias.join("harness.toml");
        let source = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(manifest, format!("{source}\n# altered pinned connector\n")).unwrap();
    }
    id
}

fn assert_aliased_shipped_connector_refuses_before_start_writes(alter_bytes: bool) {
    let rig = Rig::new();
    let id = replace_fake_with_aliased_shipped_connector(&rig, alter_bytes);
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let mut opts = rig.options(&goal, RESERVE);
    opts.harness = id;
    opts.dev = alter_bytes;

    let (result, out) = std::thread::scope(|s| joined(rig.start(s, &client, opts, None)));
    let error = result.expect_err("an aliased shipped identity is refused before start");
    assert_eq!(error.exit_code(), 1, "{error}\n{out}");
    assert!(error.to_string().contains("connector_folder"), "{error}");
    assert!(client.names().is_empty(), "{:?}", client.names());
    assert!(rig.run_dirs().is_empty(), "a run home was created");
    assert!(!rig.harness_started(), "the harness was spawned");
    assert!(
        !rig.cfg.join("capture-homes").exists(),
        "a shared capture home was created"
    );
}

#[test]
fn a_byte_identical_shipped_alias_is_exit_one_before_any_start_write() {
    let _lock = table_lock::TableLock::acquire();
    assert_aliased_shipped_connector_refuses_before_start_writes(false);
}

#[test]
fn an_altered_pinned_id_under_an_alias_is_exit_one_before_any_start_write() {
    let _lock = table_lock::TableLock::acquire();
    assert_aliased_shipped_connector_refuses_before_start_writes(true);
}

/// The proxy fails to start two ways, each before the goal is sent: its sender never finds
/// the ingest endpoint (a closed port, so the start deadline passes), or its CA certificate has
/// nowhere to go (a file already sits where it is written).
#[test]
fn a_proxy_that_cannot_start_refuses_before_the_goal_and_leaves_nothing() {
    let _lock = table_lock::TableLock::acquire();
    for (closed_ingest, said) in [(true, "proxy"), (false, "CA")] {
        let rig = Rig::new();
        let goal = rig.goal(GOAL);
        let http = rig.client();
        let client = Hooked::new(&http, &rig);
        if closed_ingest {
            let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            client.book().ingest_addr = Some(closed.local_addr().unwrap());
        } else {
            client.book().occupy_ca_file = true;
        }
        let (result, out) = std::thread::scope(|s| {
            joined(rig.start(s, &client, rig.options(&goal, RESERVE), None))
        });
        let error = result.expect_err("the proxy never started");
        assert!(format!("{error}").contains(said), "{error}\n{out}");
        assert_eq!(
            client.names(),
            vec!["create_capture_workspace", "abandon"],
            "no goal was sent"
        );
        let entry_id = client.created().entry_id;
        assert!(matches!(
            http.status(&entry_id),
            Err(ControlError::NotFound(_))
        ));
        assert!(rig.run_dirs().is_empty(), "{:?}", rig.run_dirs());
        assert!(records(&rig).is_empty());
        assert!(!rig.harness_started());
    }
}

#[test]
fn a_lost_goal_answer_never_spawns_and_is_never_retried() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    client.book().lose_goal_answer = true;
    let (result, out) =
        std::thread::scope(|s| joined(rig.start(s, &client, rig.options(&goal, RESERVE), None)));
    let error = result.expect_err("an ambiguous goal answer aborts the launch");
    assert!(
        matches!(client.book().second_goal, Some(ControlError::Conflict(_))),
        "the daemon refuses a second goal for one entry"
    );
    assert_eq!(
        client.count("record_capture_goal"),
        1,
        "not retried: {error}\n{out}"
    );
    assert_eq!(client.count("register_process"), 0);
    assert_eq!(client.count("abandon"), 1);
    assert!(!rig.harness_started());
    let entry_id = client.created().entry_id;
    assert!(matches!(
        http.status(&entry_id),
        Err(ControlError::NotFound(_))
    ));
    assert!(rig.run_dirs().is_empty(), "{:?}", rig.run_dirs());
    assert!(records(&rig).is_empty());
}

#[test]
fn refused_producer_never_spawns_the_harness() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    let goal = rig.goal(GOAL);
    let tmp = tempfile::tempdir().unwrap();
    let mock = goalcup_cli::daemon::MockDaemonClient::armed_ok(tmp.path(), m2_cli_support::FAKE_ID);
    mock.set_capture_status(
        goalcup_protocol::capture::M2Status {
            run: RunState::Starting,
            capture: goalcup_protocol::capture::CaptureState::Starting,
            delivery: goalcup_protocol::capture::DeliveryState::LocalOnly,
            analysis: goalcup_protocol::capture::AnalysisState::NotRun,
            anchor: goalcup_protocol::capture::AnchorState::LocalOnly,
            storage: goalcup_protocol::capture::CaptureStorage {
                quota_bytes: RESERVE,
                reserved_bytes: RESERVE,
                used_bytes: 0,
                durable: None,
                accounting_available: true,
            },
            gap_count: 0,
            assistance_observed: false,
        },
        "Local capture is starting.",
        1,
        1,
    );
    // Never accepted: the handshake lands in the backlog and nothing answers it.
    let receiver = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let client = Hooked::new(&mock, &rig);
    client.book().ingest_addr = Some(receiver.local_addr().unwrap());
    let (result, out) =
        std::thread::scope(|s| joined(rig.start(s, &client, rig.options(&goal, RESERVE), None)));
    let error = result.expect_err("a refused producer is no admission");
    assert!(format!("{error}").contains("admit"), "{error}\n{out}");
    assert_eq!(
        client.count("record_capture_goal"),
        0,
        "{:?}",
        client.names()
    );
    assert_eq!(client.count("abandon"), 1);
    assert!(!rig.harness_started());
    assert!(rig.run_dirs().is_empty(), "{:?}", rig.run_dirs());
}

#[test]
fn a_failed_registration_kills_the_harness_and_seals_instead_of_abandoning() {
    let _lock = table_lock::TableLock::acquire();
    for fault in [
        RegistrationFault::Refused,
        RegistrationFault::AdmittedThenMalformed,
    ] {
        let rig = Rig::new();
        let goal = rig.goal(GOAL);
        let http = rig.client();
        let client = Hooked::new(&http, &rig);
        client.book().registration = Some(fault);
        let (result, out) = std::thread::scope(|s| {
            joined(rig.start(s, &client, rig.options(&goal, RESERVE), None))
        });
        let error = result.expect_err("an unregistered harness is not a run");
        let pid = client.book().registered[0];
        support::wait_until_gone(pid);
        assert!(rig.harness_file("done").is_none(), "{error}\n{out}");
        assert_eq!(client.count("abandon"), 0, "{:?}", client.names());
        assert_eq!(client.count("seal_capture"), 1, "{:?}", client.names());
        let entry_id = client.created().entry_id;
        let (report, records) = rig.read_trace(&entry_id);
        assert!(report.sealed, "{:?}", report.findings);
        assert_eq!(
            count(&records, |r| matches!(r, CaptureRecord::Launch { .. })),
            0
        );
        assert!(report.findings.contains(&"launch_missing".to_string()));
        assert!(
            rig.run_dirs()[0].join("workspace").is_dir(),
            "the workspace stays"
        );
    }
}

/// H-10 4b (D40): a registration the daemon did not acknowledge, or one whose answer was lost,
/// never releases the parked child. Before the gate the harness was started first and killed
/// after, so for a moment it ran unrecorded; now it never runs a byte.
#[test]
fn capture_registration_refused_never_execs() {
    let _lock = table_lock::TableLock::acquire();
    for fault in [
        RegistrationFault::Refused,
        RegistrationFault::AdmittedThenMalformed,
    ] {
        let rig = Rig::new();
        let goal = rig.goal(GOAL);
        let http = rig.client();
        let client = Hooked::new(&http, &rig);
        client.book().registration = Some(fault);
        // A registration that takes 1.5 s: long enough for a harness started before it to
        // have run (it writes its pid in milliseconds), which is what the gate forbids.
        client.book().registration_delay = Some(Duration::from_millis(1_500));
        let (result, out) = std::thread::scope(|s| {
            joined(rig.start(s, &client, rig.options(&goal, RESERVE), None))
        });
        let error = result.expect_err("an unregistered harness is not a run");
        assert!(
            !rig.harness_started() && !rig.out.join("pid").exists(),
            "the harness never exec'd: {error}\n{out}"
        );
        let pid = client.book().registered[0];
        support::wait_until_gone(pid);
        assert!(
            rig.harness_file("argv").is_none() && rig.harness_file("env").is_none(),
            "no harness code ran"
        );
        assert_eq!(client.count("abandon"), 0, "{:?}", client.names());
        assert_eq!(client.count("seal_capture"), 1, "{:?}", client.names());
        assert_eq!(client.count("record_capture_launch"), 0);
        let (report, records) = rig.read_trace(&client.created().entry_id);
        assert!(report.sealed, "{:?}", report.findings);
        assert_eq!(
            count(&records, |r| matches!(r, CaptureRecord::Launch { .. })),
            0,
            "no Launch for a harness that never ran"
        );
        assert!(report.findings.contains(&"launch_missing".to_string()));
    }
}

/// The root ruling under D40: a seal that raced in between the registration and the release
/// cannot have ended a child that was still parked, so the launcher, which sees the entry
/// sealed right after the release, ends the child it just released, within the connector's
/// grace, and never records a Launch on the sealed entry.
#[test]
fn launch_refused_on_sealed_entry_ends_released_child_within_grace() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::with_grace(2);
    // A harness that ignores SIGTERM, so only the grace and a SIGKILL end it. The launcher's look
    // at the entry waits until the harness has said it ignores it (see `status_waits_for`).
    rig.tell("steps.sh", "trap '' TERM\ntouch \"$M2_FAKE_OUT/trapped\"\n");
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    client.book().registration = Some(RegistrationFault::AdmittedThenSealed);
    client.book().status_waits_for = Some("trapped");
    let began = Instant::now();
    let (result, out) =
        std::thread::scope(|s| joined(rig.start(s, &client, rig.options(&goal, RESERVE), None)));
    let took = began.elapsed();
    let error = result.expect_err("a harness on a sealed entry is not a run");
    assert_eq!(error.exit_code(), 1, "{error}\n{out}");
    assert!(
        error.to_string().contains("is no longer running"),
        "an ended harness is reported as such: {error}"
    );
    let pid = client.book().registered[0];
    // Released, so it ran, and ignored TERM; ended by this command and never by the daemon.
    assert!(rig.harness_started(), "the released child exec'd\n{out}");
    support::wait_until_gone(pid);
    assert!(
        out.contains("was sealed while its harness was being launched")
            && out.contains("ended the harness it had just started")
            && out.contains(&format!("asked to stop: {pid}; killed: {pid}"))
            && out.contains("The seal is the record; this ending is not in it."),
        "{out}"
    );
    // Within the termination bound of the connector's grace (2 s): its grace plus a kill sweep
    // plus the finish that answers the committed seal.
    assert!(
        took < Duration::from_secs(2) + Duration::from_secs(2) + Duration::from_secs(20),
        "{took:?}"
    );
    assert_eq!(
        client.count("record_capture_launch"),
        0,
        "no Launch on a sealed entry"
    );
    // The seal that raced in signalled nothing: the parked child's argv is not the harness's.
    let sealed = client.book().seal_report.clone().expect("the racing seal");
    assert!(
        matches!(sealed.termination, TerminationOutcomeV1::Unresolved { .. }),
        "{:?}",
        sealed.termination
    );
    // The launcher's own finish answers the committed report; one terminal, nothing after it.
    let (report, records) = rig.read_trace(&client.created().entry_id);
    assert!(report.sealed, "{:?}", report.findings);
    assert_eq!(
        count(&records, |r| matches!(r, CaptureRecord::LocalFinish { .. })),
        1
    );
    assert_eq!(
        count(&records, |r| matches!(r, CaptureRecord::Launch { .. })),
        0
    );
    assert!(report.findings.contains(&"launch_missing".to_string()));
}

/// The second trigger of the same ruling: the launcher's look right after the release could not
/// read the status, so the Launch is attempted, refused by the sealed entry, and the launcher looks
/// once more. (A seal that begins after the release finds an exec'd harness and ends it itself;
/// this trigger is for the seal that could not.)
#[test]
fn a_launch_error_on_a_sealed_entry_ends_the_released_child() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::with_grace(2);
    rig.tell("steps.sh", "trap '' TERM\ntouch \"$M2_FAKE_OUT/trapped\"\n");
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    client.book().registration = Some(RegistrationFault::AdmittedThenSealed);
    client.book().status_waits_for = Some("trapped");
    client.book().status_fault_once = true;
    let (result, out) =
        std::thread::scope(|s| joined(rig.start(s, &client, rig.options(&goal, RESERVE), None)));
    let error = result.expect_err("a harness on a sealed entry is not a run");
    assert_eq!(error.exit_code(), 1, "{error}\n{out}");
    let pid = client.book().registered[0];
    support::wait_until_gone(pid);
    assert!(
        out.contains("could not be read after the harness was released")
            && out.contains("ended the harness it had just started")
            && out.contains(&format!("asked to stop: {pid}; killed: {pid}")),
        "{out}"
    );
    assert_eq!(
        client.count("record_capture_launch"),
        1,
        "the Launch was attempted and refused by the sealed entry"
    );
    assert_eq!(
        client.book().sealed_seen.last(),
        Some(&true),
        "the second look saw the seal: {:?}",
        client.book().sealed_seen
    );
}

/// A `Conflict` from the daemon on an entry that is still open is a capture-side fault, not a
/// seal: the trace reads incomplete and the child is never touched.
#[test]
fn transient_launch_failure_on_open_entry_never_ends_child() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::with_grace(2);
    // The harness runs for two seconds and then ends by itself: a launcher that wrongly ended it
    // would leave no `done` marker.
    rig.tell("stop", "");
    rig.tell("steps.sh", "sleep 2\n");
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    client.book().launch_fault = Some(LaunchFault::Conflict);
    let (result, out) =
        std::thread::scope(|s| joined(rig.start(s, &client, rig.options(&goal, RESERVE), None)));
    let outcome = result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(
        outcome.exit_status.expect("a harness ran").success(),
        "{out}"
    );
    assert_eq!(
        rig.harness_file("done").as_deref(),
        Some("done\n"),
        "the harness was left to finish\n{out}"
    );
    assert!(
        out.contains("the launch could not be recorded, so the trace will read incomplete"),
        "{out}"
    );
    assert!(
        !out.contains("ended the harness it had just started"),
        "{out}"
    );
    let (report, records) = rig.read_trace(&client.created().entry_id);
    assert!(report.sealed, "{:?}", report.findings);
    assert_eq!(
        count(&records, |r| matches!(r, CaptureRecord::Launch { .. })),
        0
    );
    assert!(report.findings.contains(&"launch_missing".to_string()));
}

/// Residual 1 of the gate design (v2 section 4, kept by v3): a `stdout_json` harness whose root
/// exits while a descendant it forked still holds its stdout. The launcher's tee cannot see EOF,
/// so its drain expires at the one finish deadline and says the tail may be missing; the seal
/// finds the root already gone and signals nothing, and the descendant is never touched.
#[test]
fn descendant_holds_stdout_launcher_path() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::stdout_identity();
    rig.tell("stop", "");
    // The scope identity the harness prints first, then a descendant that inherits stdout.
    rig.tell(
        "steps.sh",
        "printf '%s\\n' '{\"type\":\"thread.started\",\"thread_id\":\"scope-1\"}'\n\
         sleep 300 &\n\
         echo $! > \"$M2_FAKE_OUT/descendant.pid\"\n",
    );
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let began = Instant::now();
    let (result, out) =
        std::thread::scope(|s| joined(rig.start(s, &client, rig.options(&goal, RESERVE), None)));
    let took = began.elapsed();
    let descendant: u32 = rig
        .harness_file("descendant.pid")
        .expect("the harness forked a descendant")
        .trim()
        .parse()
        .unwrap();
    let started =
        goalcup_protocol::process::process_start_identity(descendant).expect("a live descendant");
    // This test owns the descendant and ends it, however the assertions below go.
    struct Owned(u32, String);
    impl Drop for Owned {
        fn drop(&mut self) {
            if goalcup_protocol::process::process_start_identity(self.0).as_deref()
                == Some(self.1.as_str())
            {
                // SAFETY: two integers; the pid still has the start time this test recorded.
                unsafe { libc::kill(self.0 as i32, libc::SIGKILL) };
            }
        }
    }
    let _owned = Owned(descendant, started.clone());
    let outcome = result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(
        outcome.exit_status.expect("a harness ran").success(),
        "{out}"
    );
    assert!(
        out.contains("Final harness output did not finish within the 10-second finish budget"),
        "{out}"
    );
    assert!(!out.contains("drain bound"), "{out}");
    assert!(
        took >= Duration::from_secs(9),
        "the drain ran to its bound: {took:?}"
    );
    assert!(
        goalcup_protocol::process::is_alive(descendant)
            && goalcup_protocol::process::process_start_identity(descendant).as_deref()
                == Some(started.as_str()),
        "the descendant is the same live process: never signalled"
    );
    let (report, records) = rig.read_trace(&client.created().entry_id);
    assert!(report.sealed, "{:?}", report.findings);
    let terminal = records
        .iter()
        .find_map(|r| match r {
            CaptureRecord::LocalFinish { termination, .. } => Some(termination.clone()),
            _ => None,
        })
        .expect("a LocalFinish");
    assert!(
        matches!(terminal, TerminationOutcomeV1::AlreadyStopped { .. }),
        "the root was reaped before any signal: {terminal:?}"
    );
}

#[test]
fn automatic_sink_recovery_does_not_record_assistance() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    let upstream = Upstream::start(&rig.runtime);
    rig.tell("stop", "");
    rig.tell(
        "steps.sh",
        &format!(
            "{}echo > \"$M2_FAKE_OUT/first\"\n\
             while [ ! -f \"$M2_FAKE_OUT/cut\" ]; do sleep 0.05; done\n\
             i=0\n\
             while [ $i -lt 16 ]; do\n{}sleep 0.25\ni=$((i + 1))\ndone\n",
            upstream.curl(r#"{"n":1}"#),
            upstream.curl(r#"{"n":2}"#),
        ),
    );
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig).relaying(&rig.runtime);
    let (result, out) = std::thread::scope(|s| {
        let start = rig.start(
            s,
            &client,
            rig.options(&goal, RESERVE),
            Some(upstream.roots()),
        );
        rig.wait_for("first");
        client.relay_cut();
        rig.tell("cut", "");
        joined(start)
    });
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert_eq!(
        rig.harness_file("curl.err").unwrap_or_default(),
        "",
        "the agent never waited"
    );

    let (report, records) = rig.read_trace(&client.created().entry_id);
    assert!(report.sealed, "{:?}", report.findings);
    assert!(report.coverage_incomplete, "the loss stays visible");
    assert!(report.gap_count >= 1);
    let first = client.created().producer_id;
    assert_eq!(
        count(&records, |r| matches!(
            r,
            CaptureRecord::Recovery { previous_producer, .. } if *previous_producer == first
        )),
        1,
        "one recovery names the lost producer"
    );
    assert!(
        records.iter().any(|r| matches!(
            r,
            CaptureRecord::Open { stream, source: CaptureSource::WireRequest, .. }
                if stream.producer != first
        )),
        "capture resumed under the fresh producer"
    );
    assert_eq!(
        count(&records, |r| matches!(r, CaptureRecord::Assistance { .. })),
        0
    );
    assert_eq!(
        count(&records, |r| matches!(r, CaptureRecord::Launch { .. })),
        1
    );
    assert_eq!(
        count(&records, |r| matches!(
            r,
            CaptureRecord::Open {
                source: CaptureSource::Goal,
                ..
            }
        )),
        1,
        "the goal was never sent again"
    );
    assert_eq!(
        client.count("register_process"),
        1,
        "the harness was never restarted"
    );
}

#[test]
fn the_seal_carries_the_proxys_counters_into_the_chain() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    let upstream = Upstream::start(&rig.runtime);
    rig.tell("stop", "");
    rig.tell("steps.sh", &upstream.curl_misnamed());
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) = std::thread::scope(|s| {
        joined(rig.start(
            s,
            &client,
            rig.options(&goal, RESERVE),
            Some(upstream.roots()),
        ))
    });
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    let counters = client.book().seals[0]
        .proxy
        .expect("the launcher's seal has the proxy's report");
    assert_eq!(counters.tunneled_fail_open, 1, "{counters:?}");
    let (report, records) = rig.read_trace(&client.created().entry_id);
    assert!(
        records.iter().any(|r| matches!(
            r,
            CaptureRecord::Gap {
                stream: None,
                reason: goalcup_protocol::capture::GapReason::DecodeFailed,
                ..
            }
        )),
        "the uncaptured tunnel is a gap in the chain"
    );
    assert!(report.coverage_incomplete);
}

/// H-16. The launcher's seal request never reaches the daemon (a refused connection, the same
/// path as a busy daemon whose retries run out), so the proxy's counters are lost with it. The
/// fail-open tunnel is nevertheless in the chain, because the proxy chained it when it began,
/// and a later standalone seal (which carries no counters) commits an incomplete run.
#[test]
fn a_seal_that_never_reaches_the_daemon_keeps_the_proxys_loss() {
    use goalcup_cli::CliError;
    use goalcup_protocol::capture::GapReason;
    use goalcup_protocol::control::{CaptureSealError, CaptureSealRequest};

    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    let upstream = Upstream::start(&rig.runtime);
    rig.tell("stop", "");
    rig.tell("steps.sh", &upstream.curl_misnamed());
    let goal = rig.goal(GOAL);
    let http = rig.client();
    // No control.json to re-read after the refusal (H-14), so the refusal stands.
    std::fs::remove_file(rig.cfg.join("control.json")).unwrap();
    let client = Hooked::new(&http, &rig);
    client.book().seal_fault = Some(|| {
        CaptureSealError::NotAdmitted(ControlError::Unreachable("connection refused".into()))
    });
    // A seal that does not take is asked again until the wrap-up runs out; a test build gives a
    // run without a server two seconds of it, the first wait 100 ms.
    rig.with_env(SHORT_WRAP_UP.0, SHORT_WRAP_UP.1);
    let began = Instant::now();
    let (result, out) = std::thread::scope(|s| {
        joined(rig.start(
            s,
            &client,
            rig.options(&goal, RESERVE),
            Some(upstream.roots()),
        ))
    });
    let error = result.expect_err(&format!("the launcher's finish was refused\n{out}"));
    assert!(matches!(error, CliError::Refused(_)), "{error:?}");
    assert!(
        began.elapsed() < Duration::from_secs(20),
        "the wrap-up bounds the asking: {:?}",
        began.elapsed()
    );
    assert!(
        client.book().seals.len() > 1,
        "asked again, never delivered: {} requests",
        client.book().seals.len()
    );
    assert!(
        out.contains("fail_open=1"),
        "the proxy counted the tunnel: {out}"
    );

    // The standalone seal that follows any refusal sends no counters.
    let entry = client.created().entry_id;
    http.seal_capture(
        &entry,
        &CaptureSealRequest {
            proxy: None,
            remaining_finish_ms: None,
            native_scope: None,
        },
        Duration::from_secs(60),
    )
    .expect("the standalone seal commits");
    let (report, records) = rig.read_trace(&entry);
    assert!(
        records.iter().any(|r| matches!(
            r,
            CaptureRecord::Gap {
                stream: None,
                reason: GapReason::DecodeFailed,
                ..
            }
        )),
        "the uncaptured tunnel is a gap in the chain although no counter reached the daemon"
    );
    assert!(report.coverage_incomplete, "{:?}", report.findings);
}

#[test]
fn a_real_upstream_tls_failure_reaches_the_one_admission_summary() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    let upstream = Upstream::start(&rig.runtime);
    rig.tell("stop", "");
    rig.tell("steps.sh", &upstream.curl(r#"{"n":1}"#));
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    // The upstream presents the test CA, while this proxy uses only public roots.
    let (result, out) =
        std::thread::scope(|s| joined(rig.start(s, &client, rig.options(&goal, RESERVE), None)));
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    let summary = out
        .lines()
        .find(|line| line.starts_with("proxy admission: "))
        .unwrap_or_else(|| panic!("missing admission summary: {out}"));
    assert_eq!(out.matches("proxy admission: ").count(), 1, "{out}");
    // The fake harness also probes readiness with `nc -z`, which opens and closes one
    // connection before a CONNECT head; that can add a separate client-I/O failure.
    assert!(summary.contains(" upstream_tls=1"), "{summary}");
    assert!(summary.contains(" intercepted=0"), "{summary}");
    // Only the counters that are not zero are named.
    assert!(!summary.contains("auth_refused="), "{summary}");
    assert!(!summary.contains("alpn_incompatible="), "{summary}");
    assert!(!summary.contains("localhost") && !summary.contains("https://"));
    let (report, records) = rig.read_trace(&client.created().entry_id);
    assert!(report.sealed, "{:?}", report.findings);
    assert!(!records.iter().any(|r| matches!(
        r,
        CaptureRecord::Open {
            source: CaptureSource::WireRequest,
            ..
        }
    )));
}

/// `LOCAL_CAPTURE_FINISH_BUDGET`, the one local-finish budget (CONTRACTS.md, "C6 local finish
/// budget (H-10 4a)"). Its drain share is 8 s and its commit reserve 2 s, so a producer that
/// never settles is cut at the end of the drain share and the reserve still commits the seal.
const FINISH: Duration = Duration::from_secs(10);

/// With the relay in front of the daemon's ingest paused, the sender holds more than the
/// sockets between them can take, so its drain cannot finish, and the drain share waits out
/// its bound on the producer that never closes. That is a cut with a gap and never a refusal:
/// the commit reserve still seals. The sender gets what the one deadline has left, which is
/// nothing; a bound of its own would come on top. (Pin: green before and after H-10 4a.)
#[test]
fn a_blocked_evidence_queue_does_not_outlast_the_one_finish_deadline() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    let upstream = Upstream::start(&rig.runtime);
    rig.tell("stop", "");
    rig.tell(
        "steps.sh",
        &format!(
            "{}echo > \"$M2_FAKE_OUT/first\"\n\
             while [ ! -f \"$M2_FAKE_OUT/paused\" ]; do sleep 0.05; done\n{}",
            upstream.curl(r#"{"n":1}"#),
            upstream.curl_bytes(1536 * 1024),
        ),
    );
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig).relaying(&rig.runtime);
    let bound = FINISH + Duration::from_secs(3);
    let (result, out, finish) = std::thread::scope(|s| {
        let start = rig.start(
            s,
            &client,
            rig.options(&goal, RESERVE),
            Some(upstream.roots()),
        );
        rig.wait_for("first");
        client.relay_pause();
        rig.tell("paused", "");
        rig.wait_for("done");
        let exited = Instant::now();
        while !start.is_finished() && exited.elapsed() < bound {
            std::thread::sleep(Duration::from_millis(20));
        }
        // Past the bound, unblock the sender so the failure is reported rather than hung.
        client.relay_cut();
        let finish = exited.elapsed();
        let (result, out) = joined(start);
        (result, out, finish)
    });
    // Two endings are accepted. A release daemon seals in one shot: its owner stop is about
    // 140 ms of the ~1 s the commit reserve leaves. A debug daemon, or a loaded host, can
    // spend that second and refuse cleanly at `owner_drain` (no terminal, a sticky gap); one
    // `goalcup seal` then commits. The bound and the incompleteness below hold either way.
    match result {
        Ok(_) => eprintln!("F1: sealed in one shot"),
        Err(goalcup_cli::CliError::Refused(sentence)) if sentence.contains("owner_drain") => {
            eprintln!("F1: clean owner_drain refusal, then a second seal");
            let mut sealed = Vec::new();
            goalcup_cli::commands::seal::seal_with(
                &http,
                &rig.connectors,
                &rig.cfg,
                &client.created().entry_id,
                &mut sealed,
            )
            .unwrap_or_else(|e| panic!("the second seal: {e}"));
        }
        Err(e) => panic!("{e}\n{out}"),
    }
    assert_eq!(
        rig.harness_file("curl.err").unwrap_or_default(),
        "",
        "the agent never waited"
    );
    assert!(finish < bound, "the finish took {finish:?}\n{out}");

    let (report, records) = rig.read_trace(&client.created().entry_id);
    assert!(report.sealed, "{:?}", report.findings);
    assert!(report.coverage_incomplete, "{:?}", report.findings);
    assert!(
        records
            .iter()
            .any(|r| matches!(r, CaptureRecord::Gap { .. })),
        "the undelivered tail is a recorded gap"
    );
    assert_eq!(report.exit_code(), 1);
}

/// D56: a connection that outlives the harness (here a request nobody answers, held by a
/// process the harness left behind) must not spend the Git read's floor. The proxy stop waits
/// for it only until the drain share, which ends `GIT_READ_FLOOR` and the commit reserve before
/// the deadline, so the seal is asked with at least that much left. With the old bound (the
/// budget less the reserve) the stop ran to the end and the request stated about 2 s.
/// The harness ends only once the upstream holds the request: a harness that ended at once
/// could finish before its background `curl` reached the proxy (seen on a loaded Linux host).
#[test]
fn a_connection_held_open_at_the_finish_leaves_the_seal_the_git_floor() {
    use goalcup_protocol::control::{FINISH_COMMIT_RESERVE, GIT_READ_FLOOR};
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    // 34 s: the drain share is 34 - 2 - 30 = 2 s. (`Rig::new` names 10 s; the next rig resets it.)
    goalcup_cli::set_finish_budget(Duration::from_secs(34));
    let upstream = Upstream::start_holding_marked(&rig.runtime, &rig.out.join("held"));
    rig.tell("stop", "");
    rig.tell(
        "steps.sh",
        &format!(
            "{}{}",
            upstream.curl_waiting(r#"{"n":1}"#),
            upstream.wait_held()
        ),
    );
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let began = Instant::now();
    let (result, out) = std::thread::scope(|s| {
        joined(rig.start(
            s,
            &client,
            rig.options(&goal, RESERVE),
            Some(upstream.roots()),
        ))
    });
    let took = began.elapsed();
    goalcup_cli::set_finish_budget(Duration::from_secs(10));
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(
        rig.harness_file("held").is_some(),
        "the upstream never held the request (curl: {:?})\n{out}",
        rig.harness_file("curl.err")
    );

    // The request went out through the proxy and was never answered: it was open at the stop.
    let (_, records) = rig.read_trace(&client.created().entry_id);
    let opened = |wanted: CaptureSource| {
        records
            .iter()
            .any(|r| matches!(r, CaptureRecord::Open { source, .. } if *source == wanted))
    };
    assert!(
        opened(CaptureSource::WireRequest),
        "no request was held open\n{out}"
    );
    assert!(
        !opened(CaptureSource::WireResponse),
        "the request was answered\n{out}"
    );
    // Cut by the stop, and said so. (A Linux harness's leftover process is swept before the
    // finish, so there the connection has already ended.)
    if !cfg!(target_os = "linux") {
        assert!(out.contains(" cut=1"), "{out}");
    }

    // The seal was asked with the floor and the reserve still to run, less a small margin.
    let book = client.book();
    let ms = book
        .seals
        .first()
        .and_then(|seal| seal.remaining_finish_ms)
        .expect("the launcher states its remaining budget");
    let wanted = (GIT_READ_FLOOR + FINISH_COMMIT_RESERVE).as_millis() as u64 - 500;
    assert!(
        ms >= wanted,
        "the seal was asked with {ms} ms left of 34000; the Git floor needs {wanted}\n{out}"
    );
    assert!(
        took < Duration::from_secs(30),
        "the finish took {took:?}\n{out}"
    );
}

#[test]
fn a_test_build_trusts_the_upstream_roots_its_environment_names() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    let upstream = Upstream::start(&rig.runtime);
    rig.tell("stop", "");
    rig.tell("steps.sh", &upstream.curl(r#"{"n":1}"#));
    let pem = rig.root.join("upstream.pem");
    std::fs::write(&pem, upstream.root_pem()).unwrap();
    let goal = rig.goal(GOAL);
    let run = |roots: &std::path::Path| {
        std::process::Command::new(env!("CARGO_BIN_EXE_goalcup"))
            .args(["start", "--harness", FAKE_ID, "--dev", "--goal-file"])
            .arg(&goal)
            .env_clear()
            .env("GOALCUP_TEST_DISK", format!("{RESERVE},0,50"))
            .env("PATH", rig.path())
            .env("GOALCUP_CONFIG_DIR", &rig.cfg)
            .env("GOALCUP_CONNECTORS_DIR", &rig.capture_connectors)
            .env("GOALCUP_TEST_UPSTREAM_ROOTS", roots)
            .output()
            .unwrap()
    };

    let unreadable = run(&rig.root.join("absent.pem"));
    let said = String::from_utf8_lossy(&unreadable.stderr);
    assert_eq!(unreadable.status.code(), Some(1), "{said}");
    assert!(said.contains("GOALCUP_TEST_UPSTREAM_ROOTS"), "{said}");
    assert!(
        rig.run_dirs().is_empty(),
        "refused before anything was created"
    );

    let trusted = run(&pem);
    let said = String::from_utf8_lossy(&trusted.stdout);
    assert!(
        trusted.status.success(),
        "{said}{}",
        String::from_utf8_lossy(&trusted.stderr)
    );
    assert_eq!(rig.harness_file("curl.err").unwrap_or_default(), "");
    let entry_id = goalcup_cli::entries::latest_entry(&rig.cfg)
        .unwrap()
        .unwrap()
        .entry_id;
    let (report, records) = rig.read_trace(&entry_id);
    assert!(report.sealed, "{:?}", report.findings);
    assert!(
        records.iter().any(|r| matches!(
            r,
            CaptureRecord::Open {
                source: CaptureSource::WireRequest,
                ..
            }
        )),
        "the request to the upstream the roots name was intercepted and recorded"
    );
    assert_eq!(report.findings, Vec::<String>::new());
    assert_eq!(report.exit_code(), 0);
}

#[test]
fn stdout_tee_forwards_every_byte_unchanged() {
    use std::io::{Cursor, Read, Write};
    // Small reads split both JSON lines and the event identity across buffer boundaries.
    struct Chunked(Cursor<Vec<u8>>);
    impl Read for Chunked {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(17);
            self.0.read(&mut buf[..n])
        }
    }
    let mut bytes = b"non-json\n\xff\x00\n".to_vec();
    bytes.extend(vec![b'x'; 1024 * 1024 + 1]);
    bytes.extend(b"\n{\"type\":\"thread.started\",\"thread_id\":\"first-id\"}\n");
    bytes.extend(b"{\"type\":\"thread.started\",\"thread_id\":\"second-id\"}\npartial");
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut out = Vec::new();
    goalcup_cli::launch::stdout_tee_with_rule(
        Chunked(Cursor::new(bytes.clone())),
        &mut out,
        sender,
        &support::stdout_event_rule(),
    )
    .unwrap();
    assert_eq!(out, bytes);
    assert_eq!(receiver.recv().unwrap(), Some("first-id".into()));
    assert!(receiver.try_recv().is_err());

    struct Closed;
    impl Write for Closed {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut input = Cursor::new(bytes.clone());
    goalcup_cli::launch::stdout_tee_with_rule(
        &mut input,
        Closed,
        sender,
        &support::stdout_event_rule(),
    )
    .unwrap();
    assert_eq!(
        input.position(),
        bytes.len() as u64,
        "closed output still drains"
    );
    assert_eq!(receiver.recv().unwrap(), Some("first-id".into()));

    let (sender, receiver) = std::sync::mpsc::channel();
    let partial = b"{\"type\":\"thread.started\",\"thread_id\":\"partial\"}";
    let mut out = Vec::new();
    goalcup_cli::launch::stdout_tee_with_rule(
        &partial[..],
        &mut out,
        sender,
        &support::stdout_event_rule(),
    )
    .unwrap();
    assert_eq!(out, partial);
    assert_eq!(receiver.recv().unwrap(), None, "partial line cannot bind");
}

#[test]
fn native_scope_wait_ends_on_identity_eof_exit_or_deadline() {
    use goalcup_cli::launch::wait_native_scope;
    let _lock = table_lock::TableLock::acquire();
    let mut child = std::process::Command::new("/bin/sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    sender.send(Some("bound-id".into())).unwrap();
    assert_eq!(
        wait_native_scope(
            &mut child,
            &receiver,
            Instant::now() + Duration::from_secs(1)
        ),
        Some("bound-id".into())
    );
    sender.send(None).unwrap();
    assert_eq!(
        wait_native_scope(
            &mut child,
            &receiver,
            Instant::now() + Duration::from_secs(1)
        ),
        None
    );
    let before = Instant::now();
    assert_eq!(
        wait_native_scope(&mut child, &receiver, before + Duration::from_millis(40)),
        None
    );
    assert!(before.elapsed() < Duration::from_secs(1));
    child.kill().unwrap();
    child.wait().unwrap();
    // Keep the sender alive, as when a descendant holds the child's stdout open.
    assert_eq!(
        wait_native_scope(
            &mut child,
            &receiver,
            Instant::now() + Duration::from_secs(1)
        ),
        None
    );
    drop(sender);
}

#[test]
fn stdout_tee_completion_preserves_tail_after_child_exit_and_has_a_bound() {
    use goalcup_cli::launch::tee_capture_stdout_with_rule;
    use std::io::Write;
    use std::sync::{Arc, Condvar, Mutex};
    let _lock = table_lock::TableLock::acquire();
    struct SlowDestination {
        released: Arc<(Mutex<bool>, Condvar)>,
        bytes: Arc<Mutex<Vec<u8>>>,
    }
    impl Write for SlowDestination {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            let (lock, ready) = &*self.released;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = ready.wait(released).unwrap();
            }
            self.bytes.lock().unwrap().extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    // The child can exit after writing this small payload into its kernel pipe, while its
    // tee is still blocked forwarding the final bytes to an open participant destination.
    let payload = format!(
        "{{\"type\":\"thread.started\",\"thread_id\":\"tail-id\"}}\n{}final-tail\n",
        "x".repeat(512)
    );
    let mut child = std::process::Command::new("/usr/bin/printf")
        .arg("%s")
        .arg(&payload)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let released = Arc::new((Mutex::new(false), Condvar::new()));
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let stdout = tee_capture_stdout_with_rule(
        child.stdout.take().unwrap(),
        SlowDestination {
            released: released.clone(),
            bytes: bytes.clone(),
        },
        support::stdout_event_rule(),
    )
    .unwrap();
    assert_eq!(
        stdout.scope.recv_timeout(Duration::from_secs(1)).unwrap(),
        Some("tail-id".into())
    );
    assert!(child.wait().unwrap().success());
    let before = Instant::now();
    let incomplete = stdout.finish(before + Duration::from_millis(40)).unwrap();
    // Release even if an assertion fails, so the test never leaves its worker blocked.
    *released.0.lock().unwrap() = true;
    released.1.notify_all();
    assert!(!incomplete, "child exit alone is not forwarding completion");
    assert!(before.elapsed() < Duration::from_secs(1));
    assert!(
        stdout
            .finish(Instant::now() + Duration::from_secs(1))
            .unwrap()
    );
    assert_eq!(*bytes.lock().unwrap(), payload.as_bytes());

    // An otherwise idle pipe whose writer stays open must not cause an unbounded join.
    let (reader, held_writer) = std::os::unix::net::UnixStream::pair().unwrap();
    let stdout =
        tee_capture_stdout_with_rule(reader, std::io::sink(), support::stdout_event_rule())
            .unwrap();
    let before = Instant::now();
    let incomplete = stdout.finish(before + Duration::from_millis(40)).unwrap();
    drop(held_writer);
    assert!(!incomplete);
    assert!(before.elapsed() < Duration::from_secs(1));
    assert!(
        stdout
            .finish(Instant::now() + Duration::from_secs(1))
            .unwrap()
    );
}

#[test]
fn an_existing_isolated_home_preserves_its_exact_leaf_alias() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    let physical = rig.root.join("existing-shared-home");
    std::fs::create_dir_all(physical.join("projects/retained")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&physical, std::fs::Permissions::from_mode(0o700)).unwrap();
    let retained = physical.join("projects/retained/session.jsonl");
    let original = b"existing shared store must remain unchanged\n";
    std::fs::write(&retained, original).unwrap();
    let homes = rig.cfg.join("capture-homes");
    std::fs::create_dir_all(&homes).unwrap();
    let alias = homes.join(FAKE_ID);
    std::os::unix::fs::symlink(&physical, &alias).unwrap();
    // The controlled fixture reports whatever home variable its manifest names; its fake
    // executable invokes no auth API, copies no credentials and runs no model.
    let manifest_path = rig.capture_connectors.join(FAKE_ID).join("harness.toml");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap();
    let home_env = "env = \"CODEX_HOME\"";
    assert_eq!(manifest.matches(home_env).count(), 1);
    std::fs::write(
        &manifest_path,
        manifest.replace(home_env, "env = \"EXAMPLE_CONFIG_DIR\""),
    )
    .unwrap();
    rig.tell("stop", "");
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, output) = std::thread::scope(|scope| {
        joined(rig.start(scope, &client, rig.options(&goal, RESERVE), None))
    });
    let outcome = result.unwrap_or_else(|error| panic!("{error}\n{output}"));
    assert!(outcome.exit_status.unwrap().success(), "{output}");
    let expected = alias.to_str().unwrap();
    assert_eq!(client.book().creates[0].config_home, expected);
    let env = rig.harness_file("env").unwrap();
    assert_eq!(
        env.lines()
            .find_map(|line| line.strip_prefix("EXAMPLE_CONFIG_DIR=")),
        Some(expected)
    );
    let record = goalcup_cli::entries::read_entry(&rig.cfg, &outcome.entry_id).unwrap();
    assert_eq!(record.harness_home, expected);
    assert!(
        std::fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read(&retained).unwrap(), original);
}

#[test]
fn unsafe_shared_capture_home_leaves_refuse_before_a_request_or_child() {
    let _lock = table_lock::TableLock::acquire();
    for case in ["dangling", "file", "loop", "wide"] {
        let rig = Rig::new();
        let homes = rig.cfg.join("capture-homes");
        std::fs::create_dir_all(&homes).unwrap();
        let alias = homes.join(FAKE_ID);
        match case {
            "dangling" => std::os::unix::fs::symlink(rig.root.join("missing"), &alias).unwrap(),
            "file" => std::fs::write(&alias, b"not a directory").unwrap(),
            "loop" => std::os::unix::fs::symlink(&alias, &alias).unwrap(),
            "wide" => {
                std::fs::create_dir(&alias).unwrap();
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&alias, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            _ => unreachable!(),
        }
        let goal = rig.goal(GOAL);
        let http = rig.client();
        let client = Hooked::new(&http, &rig);
        let (result, out) = std::thread::scope(|scope| {
            joined(rig.start(scope, &client, rig.options(&goal, RESERVE), None))
        });
        let error = result.expect_err(case);
        assert_eq!(error.exit_code(), 1, "{case}: {error}\n{out}");
        assert!(
            error.to_string().contains("shared capture home"),
            "{case}: {error}\n{out}"
        );
        assert!(client.names().is_empty(), "{case}: {:?}", client.names());
        assert!(rig.run_dirs().is_empty(), "{case}: {:?}", rig.run_dirs());
        assert!(!rig.harness_started(), "{case}");
    }
}

#[test]
fn shared_capture_home_rejects_a_wrong_owner() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    let home = rig.root.join("wrong-owner-fixture");
    std::fs::create_dir(&home).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    let other_uid = unsafe { libc::geteuid() }.wrapping_add(1);
    let error =
        goalcup_cli::commands::start::validate_shared_capture_home_for_uid(&home, other_uid)
            .expect_err("a home not owned by the expected effective uid must be refused");
    assert!(error.to_string().contains("effective user"), "{error}");
}

struct PauseEnv;

impl Drop for PauseEnv {
    fn drop(&mut self) {
        // SAFETY: every test in this binary holds the process-table lock before changing this
        // test-only variable, and the guard restores the process environment before releasing it.
        unsafe { std::env::remove_var("GOALCUP_TEST_PAUSE_DIR") };
    }
}

#[test]
fn shared_capture_home_replacement_before_create_refuses_without_a_request_or_child() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::new();
    let homes = rig.cfg.join("capture-homes");
    std::fs::create_dir_all(&homes).unwrap();
    let alias = homes.join(FAKE_ID);
    let physical = rig.root.join("shared-home-target");
    std::fs::create_dir(&physical).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&physical, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(&physical, &alias).unwrap();
    let pause = rig.root.join("shared-home-pause");
    std::fs::create_dir(&pause).unwrap();
    std::fs::write(pause.join("capture-shared-home-pre-create.arm"), b"").unwrap();
    // SAFETY: this binary's tests take the same lock before changing this test-only variable.
    unsafe { std::env::set_var("GOALCUP_TEST_PAUSE_DIR", &pause) };
    let _restore = PauseEnv;

    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) = std::thread::scope(|scope| {
        let start = rig.start(scope, &client, rig.options(&goal, RESERVE), None);
        support::wait_for_file(&pause.join("capture-shared-home-pre-create.reached"));
        std::fs::rename(&physical, rig.root.join("replaced-shared-home-target")).unwrap();
        std::fs::create_dir(&physical).unwrap();
        std::fs::set_permissions(&physical, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(pause.join("capture-shared-home-pre-create.go"), b"").unwrap();
        joined(start)
    });
    let error = result.expect_err("the recorded shared-home leaf was replaced");
    assert!(error.to_string().contains("changed"), "{error}\n{out}");
    assert!(client.names().is_empty(), "{:?}", client.names());
    assert!(rig.run_dirs().is_empty(), "{:?}", rig.run_dirs());
    assert!(!rig.harness_started());
}

// ---------------------------------------------------------------- H-10 4a: the finish refusals

/// Each way the capture seal can fail is an exit-1 refusal with one fixed sentence, and the
/// request the launcher built carries what remained of the one budget and a read timeout
/// derived from it. The daemon is not asked: the refusal is the hook's.
#[test]
fn capture_finish_refusals_say_what_they_mean_and_carry_the_one_budget() {
    use goalcup_cli::CliError;
    use goalcup_protocol::control::CaptureSealError;

    type Fault = fn() -> CaptureSealError;
    let cases: [(&str, Fault, &[&str]); 6] = [
        (
            "finish_deadline",
            || {
                CaptureSealError::MaybeAdmitted(ControlError::FinishDeadline(
                    "blocking_permit".into(),
                ))
            },
            &[
                "did not finish within its 10-second local budget",
                "stopped before blocking_permit",
                "Nothing terminal was written",
            ],
        ),
        (
            "run_lock",
            || CaptureSealError::MaybeAdmitted(ControlError::FinishDeadline("run_lock".into())),
            &["Another finish of entry", "is still in progress"],
        ),
        (
            "ambiguous",
            || CaptureSealError::MaybeAdmitted(ControlError::Unreachable("read timed out".into())),
            &[
                "did not answer the finish of entry",
                "It may still complete it",
                "answered unchanged",
            ],
        ),
        (
            "unauthorized",
            || CaptureSealError::NotAdmitted(ControlError::Unauthorized("stale token".into())),
            &["was not finished", "before the finish began"],
        ),
        (
            "refused connection",
            || {
                CaptureSealError::NotAdmitted(ControlError::Unreachable(
                    "connection refused".into(),
                ))
            },
            &["was not finished", "before the finish began"],
        ),
        (
            "busy",
            || CaptureSealError::NotAdmitted(ControlError::Busy("held".into())),
            &["still busy with entry", "goalcup seal"],
        ),
    ];
    for (name, fault, wanted) in cases {
        let _lock = table_lock::TableLock::acquire();
        let rig = Rig::new();
        rig.tell("stop", "");
        let goal = rig.goal(GOAL);
        let http = rig.client();
        // No control.json to re-read after a refusal before the finish began (H-14), so the
        // refusal stands: `a_finish_refused_before_it_began_is_asked_again_...` covers the rest.
        std::fs::remove_file(rig.cfg.join("control.json")).unwrap();
        let client = Hooked::new(&http, &rig);
        client.book().seal_fault = Some(fault);
        rig.with_env(SHORT_WRAP_UP.0, SHORT_WRAP_UP.1);
        let began = Instant::now();
        let (result, out) = std::thread::scope(|s| {
            joined(rig.start(s, &client, rig.options(&goal, RESERVE), None))
        });
        let error = result.expect_err(&format!("{name} refuses the run's finish\n{out}"));
        let CliError::Refused(sentence) = &error else {
            panic!("{name}: an exit-1 refusal, got {error:?}");
        };
        for part in wanted {
            assert!(sentence.contains(part), "{name}: {part:?} in {sentence:?}");
        }
        assert!(
            sentence.contains("goalcup seal") || name == "run_lock",
            "{name}: the way forward is named: {sentence:?}"
        );
        assert!(began.elapsed() < Duration::from_secs(30), "{name}");

        // The one budget, as the launcher spent it: the drain share is 8 s, the first request
        // was built from what remained, and the transport timeout is that plus the launcher's
        // termination allowance (its root is reaped) plus the connect slack.
        let book = client.book();
        let first = book.seals.first().expect("a capture seal request");
        let ms = first
            .remaining_finish_ms
            .expect("the launcher states its remaining budget");
        assert!(
            (1_500..=10_000).contains(&ms),
            "{name}: remaining {ms} ms of the 10 s budget"
        );
        let timeout = book.seal_timeouts[0];
        assert_eq!(
            timeout,
            Duration::from_millis(ms) + Duration::from_millis(4_250) + Duration::from_secs(2),
            "{name}: read timeout = remaining + termination_bound(0) + CONNECT_TIMEOUT"
        );
        if name == "unauthorized" {
            assert_eq!(book.seals.len(), 1, "{name} is definitive, never replayed");
        } else {
            assert!(
                book.seals.len() > 1,
                "{name} is asked again until the wrap-up runs out: {} requests",
                book.seals.len()
            );
        }
    }
}

/// H-14, in process: the launcher's finish is refused before it began (`unauthorized`, or a
/// connection nobody accepted) because the daemon it built its client for has been restarted. With
/// the restarted daemon's control.json on disk the finish asks again there, once, and the run
/// finishes: `goalcup start` ends the run it began instead of leaving the participant a seal to
/// run by hand. The first client saw exactly one request.
#[test]
fn a_finish_refused_before_it_began_is_asked_again_after_rereading_control_json() {
    use goalcup_protocol::control::CaptureSealError;
    type Fault = fn() -> CaptureSealError;
    let faults: [(&str, Fault); 2] = [
        ("unauthorized", || {
            CaptureSealError::NotAdmitted(ControlError::Unauthorized("stale token".into()))
        }),
        ("refused connection", || {
            CaptureSealError::NotAdmitted(ControlError::Unreachable("connection refused".into()))
        }),
    ];
    for (name, fault) in faults {
        let _lock = table_lock::TableLock::acquire();
        let rig = Rig::new();
        rig.tell("stop", "");
        let goal = rig.goal(GOAL);
        let http = rig.client();
        let client = Hooked::new(&http, &rig);
        client.book().seal_fault = Some(fault);
        let (result, out) = std::thread::scope(|s| {
            joined(rig.start(s, &client, rig.options(&goal, RESERVE), None))
        });
        result.unwrap_or_else(|e| panic!("{name}: the finish follows the restart: {e}\n{out}"));
        assert!(out.contains("finished locally"), "{name}: {out}");
        assert_eq!(
            client.book().seals.len(),
            1,
            "{name}: one request to the first client"
        );
        let (_, records) = rig.read_trace(&client.created().entry_id);
        assert!(
            records
                .iter()
                .any(|r| matches!(r, CaptureRecord::LocalFinish { .. })),
            "{name}: the entry is sealed by the launcher's own finish"
        );
    }
}
