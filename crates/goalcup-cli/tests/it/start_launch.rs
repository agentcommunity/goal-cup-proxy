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

/// `goalcup git-credential` is what a run's clone calls; it is not for the participant, so
/// `--help` does not list it, and `start` has no `--repo` any more (the repository comes from
/// `goalcup prep`) but has `--practice`.
#[test]
fn the_git_credential_command_is_hidden_and_start_takes_practice_not_repo() {
    let goalcup = env!("CARGO_BIN_EXE_goalcup");
    let help = std::process::Command::new(goalcup)
        .arg("--help")
        .output()
        .unwrap();
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(!help.contains("git-credential"), "{help}");
    for shown in [
        "start", "status", "entries", "seal", "doctor", "login", "upload",
    ] {
        assert!(help.contains(shown), "{shown}: {help}");
    }
    let start = std::process::Command::new(goalcup)
        .args(["start", "--help"])
        .output()
        .unwrap();
    let start = String::from_utf8_lossy(&start.stdout);
    assert!(start.contains("--practice"), "{start}");
    assert!(!start.contains("--repo"), "{start}");
    // The command still answers git, and only for the stored repository.
    let tmp = tempfile::tempdir().unwrap();
    let token = "github_pat_FAKE0123456789abcdefghijkl";
    goalcup_cli::account::github::GithubSetup::new("https://github.com/goal-cup/demo", token)
        .unwrap()
        .store(tmp.path())
        .unwrap();
    let ask = |path: &str, op: &str| {
        use std::io::Write;
        let mut child = std::process::Command::new(goalcup)
            .args(["git-credential", op])
            .env("GOALCUP_CONFIG_DIR", tmp.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(format!("protocol=https\nhost=github.com\npath={path}\n\n").as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8(out.stdout).unwrap()
    };
    let answer = ask("goal-cup/demo.git", "get");
    assert!(answer.contains("username=x-access-token"), "{answer}");
    assert!(answer.contains(&format!("password={token}")), "{answer}");
    assert_eq!(ask("someone/else.git", "get"), "");
    assert_eq!(ask("goal-cup/demo.git", "store"), "");
    // An operation git does not use is a usage error.
    let odd = std::process::Command::new(goalcup)
        .args(["git-credential", "frobnicate"])
        .env("GOALCUP_CONFIG_DIR", tmp.path())
        .output()
        .unwrap();
    assert_eq!(odd.status.code(), Some(1));
}

// ------------------------------------------------------------- chat mode (D57)

/// The harness in chat mode in the participant's terminal (D57): `goalcup start` shares the
/// terminal with it, stays silent while it runs, survives what the terminal sends the harness,
/// and still seals and uploads when the window is closed.
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod chat {
    use crate::common::m2_cli_support::{CHAT_ID, Hooked, Rig};
    use crate::common::upload_server::{ServerOptions, UploadServer};
    use goalcup_protocol::capture::{CaptureRecord, CaptureSource};
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    const RESERVE: u64 = 4 * 1024 * 1024;
    /// What `goalcup start` prints last before the harness owns the terminal.
    const LAUNCH_LINE: &str = "Recording in chat mode";
    /// A test build's disk sizes: the reservation, no margin, a 50 ms watch.
    const DISK: (&str, &str) = ("GOALCUP_TEST_DISK", "4194304,0,50");
    /// A short wrap-up for a run without a server.
    const LOCAL_WRAP_UP: (&str, &str) = ("GOALCUP_TEST_LOCAL_WRAP_UP", "2000,100");
    /// The uploader's cadence a test can watch, with a heartbeat every 100 ms.
    const BEATING: (&str, &str) = (
        "GOALCUP_TEST_UPLOAD_POLICY",
        "200,25,100,400,10000,268435456,3000,100",
    );

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !done() {
            assert!(
                std::time::Instant::now() < deadline,
                "{what} never happened"
            );
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }

    fn harness(rig: &Rig, name: &str) -> String {
        rig.harness_file(name)
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    /// The newest run's own directory (`<cfg>/runs/<run_id>`), where `host.log` is.
    fn run_dir(rig: &Rig) -> PathBuf {
        let record = goalcup_cli::run::newest(&rig.cfg)
            .expect("the run records are readable")
            .expect("a run was recorded");
        goalcup_cli::run::run_dir(&rig.cfg, &record.run_id)
    }

    fn newest_entry(rig: &Rig) -> String {
        goalcup_cli::entries::latest_entry(&rig.cfg)
            .unwrap()
            .expect("an entry was recorded")
            .entry_id
    }

    /// Every byte of the trace's native streams, joined.
    fn native_bytes(records: &[CaptureRecord]) -> String {
        use base64::Engine;
        let natives: Vec<_> = records
            .iter()
            .filter_map(|r| match r {
                CaptureRecord::Open {
                    stream,
                    source: CaptureSource::Native,
                    ..
                } => Some(stream.clone()),
                _ => None,
            })
            .collect();
        let mut bytes = Vec::new();
        for r in records {
            if let CaptureRecord::Segment {
                stream, data_b64, ..
            } = r
                && natives.contains(stream)
            {
                bytes.extend(
                    base64::engine::general_purpose::STANDARD
                        .decode(data_b64)
                        .unwrap(),
                );
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// The lines the terminal showed after the launch line and before `end`.
    fn between_launch_and(shown: &str, end: &str) -> Vec<String> {
        let launch = shown
            .find(LAUNCH_LINE)
            .unwrap_or_else(|| panic!("no launch line:\n{shown}"));
        let after = &shown[launch..];
        let after = &after[after.find('\n').map_or(after.len(), |i| i + 1)..];
        let window = &after[..after
            .find(end)
            .unwrap_or_else(|| panic!("never showed {end:?}:\n{shown}"))];
        window
            .split('\n')
            .map(|l| l.trim_matches('\r').to_string())
            .collect()
    }

    /// Every file under `root` with its bytes, by path relative to it, `skip` left out.
    fn tree(root: &Path, skip: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(dir: &Path, root: &Path, skip: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.starts_with(skip) {
                    continue;
                }
                if path.is_dir() {
                    walk(&path, root, skip, out);
                } else {
                    out.insert(
                        path.strip_prefix(root).unwrap().to_path_buf(),
                        std::fs::read(&path).unwrap(),
                    );
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(root, root, skip, &mut out);
        out
    }

    fn private_dir(dir: &Path) {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .unwrap();
    }

    /// A typed Ctrl-C reaches the whole foreground group. The harness is in it and ends; the
    /// host is in it and rides it out. From the launch line to the harness's exit the terminal
    /// shows the harness's bytes and nothing of the host's, and once the harness is gone the
    /// host seals the run.
    #[test]
    fn the_host_is_silent_and_survives_ctrl_c() {
        let rig = Rig::chat();
        let mut run = rig.start_on_pty(&[DISK, LOCAL_WRAP_UP]);
        run.wait_shown("tui: ready");
        assert_eq!(harness(&rig, "tui.stdin"), "tty", "{}", run.shown());
        assert_eq!(harness(&rig, "tui.argv").lines().next(), Some("chat"));
        // The harness is in the terminal's foreground group, or its first read would stop it.
        let tui: i32 = harness(&rig, "tui.pid").parse().unwrap();
        // SAFETY: an integer; the pid is the fake harness's own.
        assert_eq!(unsafe { libc::getpgid(tui) }, run.foreground_group());

        run.type_line("hello");
        run.wait_shown("tui: noted hello");
        run.signal_foreground(libc::SIGINT);
        let status = run.wait();
        let shown = run.shown();
        assert!(status.success(), "{status:?}\n{shown}");
        assert_eq!(harness(&rig, "tui.end"), "int", "{shown}");

        for line in between_launch_and(&shown, "tui: interrupted") {
            assert!(
                line.is_empty() || line.starts_with("tui:") || line == "hello",
                "the host wrote {line:?} to the terminal while the harness ran:\n{shown}"
            );
        }
        let after = &shown[shown.find("tui: interrupted").unwrap()..];
        assert!(after.contains("harness exited"), "{shown}");

        let (report, records) = rig.read_trace(&newest_entry(&rig));
        assert!(report.sealed, "{:?}", report.findings);
        assert!(
            native_bytes(&records).contains(r#""text":"hello""#),
            "{shown}"
        );
        let log = run_dir(&rig).join("host.log");
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&log)
            .unwrap_or_else(|e| panic!("{}: {e}", log.display()))
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "{}", log.display());
    }

    /// The window is closed while the harness runs: the harness ends, and the host, which
    /// cannot print any more, still seals the run, sends the final heartbeat and uploads it.
    /// What it would have said is in `host.log`.
    #[test]
    fn closing_the_window_still_seals_and_uploads_the_segment() {
        let rig = Rig::chat();
        let server = UploadServer::start(&rig.root.join("server"), ServerOptions::default());
        rig.prepare_for(&server.url());
        let mut run = rig.start_on_pty(&[DISK, BEATING]);
        run.wait_shown("tui: ready");
        run.type_line("hello");
        run.wait_shown("tui: noted hello");
        run.signal_foreground(libc::SIGHUP);
        run.close();
        let status = run.wait();
        assert!(status.success(), "{status:?}\n{}", run.shown());
        assert_eq!(harness(&rig, "tui.end"), "hup");

        let runs = server.run_ids();
        assert_eq!(runs.len(), 1);
        let run_id = &runs[0];
        assert_eq!(server.status(run_id).as_deref(), Some("sealed"));
        assert!(
            server.heartbeats(run_id).iter().any(|h| h.final_),
            "a final heartbeat"
        );
        assert!(!server.chunks(run_id).is_empty(), "chunks were uploaded");
        let entry = newest_entry(&rig);
        let trace = rig.trace(&entry);
        let (seal, _) = server.seal(run_id).expect("the seal was sent");
        assert_eq!(
            seal.as_bytes(),
            std::fs::read(trace.join("seal.json")).unwrap()
        );
        let log = std::fs::read_to_string(run_dir(&rig).join("host.log")).unwrap();
        assert!(log.contains("uploaded:"), "{log}");
    }

    /// The prompt is typed, not passed: it reaches the record through the session file the
    /// harness writes in its home, which is read because it was made after the run started.
    #[test]
    fn the_typed_prompt_is_recorded_from_the_session_file() {
        let rig = Rig::chat();
        // An old session in the shared home, from before this run: it is not this run's.
        let sessions = rig.chat_home().join("sessions");
        private_dir(&sessions);
        let old = sessions.join("session-old.jsonl");
        std::fs::write(&old, "{\"type\":\"user\",\"text\":\"an older run\"}\n").unwrap();
        let yesterday = std::time::SystemTime::now() - std::time::Duration::from_secs(86_400);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(yesterday)
            .unwrap();
        // The change time cannot be set back: let it fall behind the run's start.
        std::thread::sleep(std::time::Duration::from_millis(1100));

        let mut run = rig.start_on_pty(&[DISK, LOCAL_WRAP_UP]);
        run.wait_shown("tui: ready");
        run.type_line("hello");
        run.wait_shown("tui: noted hello");
        run.type_line("/exit");
        let status = run.wait();
        assert!(status.success(), "{status:?}\n{}", run.shown());
        assert_eq!(harness(&rig, "tui.end"), "exit");

        let (report, records) = rig.read_trace(&newest_entry(&rig));
        assert!(report.sealed, "{:?}", report.findings);
        let native = native_bytes(&records);
        assert!(native.contains(r#""text":"hello""#), "{native}");
        assert!(!native.contains("an older run"), "{native}");
        // No goal was handed to the harness: its argv is the connector's args alone.
        assert_eq!(
            harness(&rig, "tui.argv").lines().collect::<Vec<_>>().len(),
            3,
            "{}",
            harness(&rig, "tui.argv")
        );
    }

    /// The uploader's heartbeats run from the launch, before the participant has typed anything
    /// and so before the harness has written a session file. What the uploader has to say
    /// meanwhile (a chunk the server refused) goes to `host.log`, never to the terminal.
    #[test]
    fn heartbeats_flow_before_the_first_session_file_appears() {
        let rig = Rig::chat();
        let server = UploadServer::start(&rig.root.join("server"), ServerOptions::default());
        rig.prepare_for(&server.url());
        server.fail_next_chunks(2);
        let mut run = rig.start_on_pty(&[DISK, BEATING]);
        run.wait_shown("tui: ready");
        wait_until("two heartbeats", || {
            server
                .run_ids()
                .first()
                .is_some_and(|run| server.heartbeats(run).len() >= 2)
        });
        let sessions: Vec<_> = std::fs::read_dir(rig.chat_home().join("sessions"))
            .map(|entries| entries.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        assert!(sessions.is_empty(), "no session file yet: {sessions:?}");
        let log = run_dir(&rig).join("host.log");
        wait_until("the uploader to say a chunk failed", || {
            std::fs::read_to_string(&log).is_ok_and(|said| said.contains("goalcup: upload: "))
        });
        run.type_line("/exit");
        let status = run.wait();
        let shown = run.shown();
        assert!(status.success(), "{status:?}\n{shown}");
        for line in between_launch_and(&shown, "tui: bye") {
            assert!(
                line.is_empty() || line.starts_with("tui:") || line == "/exit",
                "the host wrote {line:?} to the terminal while the harness ran:\n{shown}"
            );
        }
        let run_id = &server.run_ids()[0];
        assert_eq!(server.status(run_id).as_deref(), Some("sealed"));
    }

    /// A chat-mode connector takes no goal file, and a headless one needs one: both are
    /// refused before anything is created, with a sentence that says which.
    #[test]
    fn a_goal_file_with_an_interactive_connector_is_refused() {
        let rig = Rig::chat();
        let goal = rig.goal("write hello.txt");
        let http = rig.client();
        let client = Hooked::new(&http, &rig);
        let mut opts = rig.chat_options(RESERVE);
        opts.goal_file = Some(goal.clone());
        let (result, out) =
            std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
        let refused = result.expect_err("a goal file with a chat-mode connector");
        let said = refused.to_string();
        assert_eq!(refused.exit_code(), 1, "{said}");
        assert!(
            said.contains(CHAT_ID) && said.contains("chat mode"),
            "{said}"
        );
        assert!(said.contains("--goal-file"), "{said}");
        assert!(client.book().calls.is_empty(), "{out}");
        assert!(rig.run_dirs().is_empty(), "{:?}", rig.run_dirs());
        assert!(!rig.chat_home().exists(), "the shared home was not claimed");

        // The headless connector still needs its goal file.
        let mut opts = rig.options(&goal, RESERVE);
        opts.goal_file = None;
        let (result, _) = std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
        let refused = result.expect_err("a headless connector with no goal file");
        assert!(refused.to_string().contains("--goal-file"), "{refused}");
        assert!(client.book().calls.is_empty());

        // The binary says the same, before anything is created.
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup"))
            .args(["start", "--harness", CHAT_ID, "--dev", "--goal-file"])
            .arg(&goal)
            .env_clear()
            .env("PATH", rig.path())
            .env("GOALCUP_CONFIG_DIR", &rig.cfg)
            .env("GOALCUP_CONNECTORS_DIR", &rig.capture_connectors)
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{said}");
        assert!(said.contains("chat mode"), "{said}");
        assert!(rig.run_dirs().is_empty());
    }

    /// Before the launch the goal-cup skill is put in the harness's skills folder, replacing
    /// only its own folder: the participant's other skills and settings are untouched.
    #[test]
    fn the_goal_cup_skill_is_in_the_harness_home_and_nothing_else_changed() {
        let rig = Rig::chat();
        let home = rig.chat_home();
        private_dir(&home.join("skills/theirs"));
        private_dir(&home.join("skills/goal-cup"));
        std::fs::write(home.join("skills/theirs/SKILL.md"), "the participant's own").unwrap();
        std::fs::write(home.join("skills/goal-cup/stale.md"), "an older copy").unwrap();
        std::fs::write(home.join("settings.json"), "{\"theme\":\"dark\"}").unwrap();
        let ours = home.join("skills/goal-cup");
        let before = tree(&home, &ours);

        let mut run = rig.start_on_pty(&[DISK, LOCAL_WRAP_UP]);
        run.wait_shown("tui: ready");
        run.type_line("/exit");
        let status = run.wait();
        assert!(status.success(), "{status:?}\n{}", run.shown());

        // The harness found the skill at its start, and only it.
        assert_eq!(harness(&rig, "tui.skills"), "SKILL.md");
        let shipped =
            std::fs::read(crate::common::support::repo_root().join("skills/goal-cup/SKILL.md"))
                .unwrap();
        assert_eq!(std::fs::read(ours.join("SKILL.md")).unwrap(), shipped);
        let names: Vec<_> = std::fs::read_dir(&ours)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, ["SKILL.md"]);
        assert_eq!(
            tree(&home, &ours),
            before,
            "nothing else in the home changed"
        );
    }
}
