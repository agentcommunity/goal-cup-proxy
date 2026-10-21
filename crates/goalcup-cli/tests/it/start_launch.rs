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
            "any-harness",
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
/// daemon is not asked, there is no run directory in the run's folder, no run record, and no
/// reserve file. (A chat-mode run: no goal file, its folder is the one it is run from.)
#[test]
fn a_run_that_cannot_take_its_reserve_is_refused_before_anything_is_created() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("cfg");
    std::fs::create_dir(&cfg).unwrap();
    let beside = tmp.path().join("run-folder");
    std::fs::create_dir(&beside).unwrap();
    let m = support::mock_armed(&tmp, "chat-shared-home");
    let mut o = support::start_options("chat-shared-home");
    o.dev = true;
    o.run_folder = Some(beside.clone());
    // More than any volume has, and a margin on top.
    o.disk.reserve_bytes = 1 << 60;
    o.disk.start_margin = 1 << 30;
    let err = goalcup_cli::commands::start::start_with(
        &m,
        &support::fixture_connectors_dir(),
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
    let made: Vec<_> = std::fs::read_dir(&beside).unwrap().flatten().collect();
    assert!(made.is_empty(), "nothing in the run's folder: {made:?}");
    assert!(!cfg.join("runs").exists(), "no run record");
    let reserve = goalcup_cli::run::reserve::artifact_root(&cfg).join("reserve");
    assert!(!reserve.exists(), "no reserve file or folder");
}

/// What `start` does on a stated WSL machine: the refusal, and that nothing was made or asked.
fn start_on_wsl(wsl: &str, with_work_folder: Option<&str>) -> String {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("cfg");
    std::fs::create_dir(&cfg).unwrap();
    let beside = tmp.path().join("run-folder");
    std::fs::create_dir(&beside).unwrap();
    if let Some(work) = with_work_folder {
        goalcup_cli::server::write_server_config(
            &cfg,
            &goalcup_cli::server::ServerConfig {
                url: "http://127.0.0.1:1".to_string(),
                account: Some("member-1".to_string()),
            },
        )
        .unwrap();
        goalcup_cli::prep_file::write_prep(
            &cfg,
            &goalcup_cli::prep_file::PrepFile {
                work_root: work.to_string(),
                ..Default::default()
            },
        )
        .unwrap();
    }
    let m = support::mock_armed(&tmp, "codex-cli");
    let mut o = support::start_options("codex-cli");
    o.run_folder = Some(beside.clone());
    let mut env = support::clean_env();
    env.insert(
        goalcup_cli::wsl_gate::Host::VAR.to_string(),
        wsl.replace("{cfg}", &cfg.display().to_string())
            .replace("{beside}", &beside.display().to_string()),
    );
    let err = goalcup_cli::commands::start::start_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &env,
        &o,
        &mut Vec::new(),
    )
    .expect_err("a WSL machine this run cannot use");
    assert_eq!(err.exit_code(), 1, "{err}");
    assert!(err.to_string().ends_with("Nothing was created."), "{err}");
    assert!(
        m.calls().is_empty(),
        "the daemon was asked: {:?}",
        m.calls()
    );
    let made: Vec<_> = std::fs::read_dir(&beside).unwrap().flatten().collect();
    assert!(
        made.is_empty(),
        "nothing was made in the run folder: {made:?}"
    );
    assert!(!cfg.join("runs").exists(), "no run record");
    let reserve = goalcup_cli::run::reserve::artifact_root(&cfg).join("reserve");
    assert!(!reserve.exists(), "no reserve file or folder");
    err.to_string()
}

/// WSL1 cannot run Goal Cup: one sentence, and nothing created.
#[test]
fn start_refuses_wsl1_before_anything_is_created() {
    let said = start_on_wsl("wsl=1", None);
    assert!(
        said.starts_with("WSL1 cannot run Goal Cup: convert this distribution to WSL2 with")
            && said.contains("`wsl --set-version <distro> 2`"),
        "{said}"
    );
}

/// On WSL2 each folder the run writes to is judged by its filesystem type.
#[test]
fn start_refuses_each_run_folder_on_a_windows_host_filesystem_under_wsl2() {
    let said = start_on_wsl("wsl=2,fs={beside}=9p", None);
    assert!(
        said.contains("run folder") && said.contains("(9p)"),
        "{said}"
    );
    let said = start_on_wsl("wsl=2,fs={cfg}=drvfs", None);
    assert!(
        said.contains("config folder") && said.contains("(drvfs)"),
        "{said}"
    );
    // The artifact root is beside the configuration folder, not inside it.
    let said = start_on_wsl("wsl=2,fs={cfg}-artifacts=virtiofs", None);
    assert!(
        said.contains("artifact root") && said.contains("(virtiofs)"),
        "{said}"
    );
    assert!(!said.contains("config folder"), "{said}");
    // The work folder `goalcup prep` kept (a run on the server has one).
    let said = start_on_wsl("wsl=2,fs=/odd/place=9p", Some("/odd/place/work"));
    assert!(said.contains("work folder /odd/place/work"), "{said}");
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
    assert!(start.contains("--test"), "{start}");
    assert!(!start.contains("--repo"), "{start}");
    // `--test` and `--practice` are two different kinds of run: clap refuses both together.
    let both = std::process::Command::new(goalcup)
        .args(["start", "--harness", "any-harness", "--test", "--practice"])
        .output()
        .unwrap();
    assert!(!both.status.success(), "{both:?}");
    let refusal = String::from_utf8_lossy(&both.stderr);
    assert!(
        refusal.contains("--test") && refusal.contains("--practice"),
        "{refusal}"
    );
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
    use crate::common::m2_cli_support::{CHAT_HOME_ID, CHAT_ID, Launcher, OnPty, Rig};
    use crate::common::upload_server::{ServerOptions, UploadServer};
    use goalcup_protocol::capture::{CaptureRecord, CaptureSource};
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

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

    /// The window is closed while the harness runs, as a participant closes it: the terminal's
    /// master side goes, the kernel hangs up the shell that leads the session, and then the
    /// foreground group (the host and the harness). The harness ends, and the host, which cannot
    /// print any more, still seals the run, sends the final heartbeat and uploads it. What it
    /// would have said is in `host.log`.
    #[test]
    fn closing_the_window_still_seals_and_uploads_the_segment() {
        let rig = Rig::chat();
        let server = UploadServer::start(&rig.root.join("server"), ServerOptions::default());
        rig.prepare_for(&server.url());
        let mut run = rig.start_under_a_shell_on_pty(&[DISK, BEATING]);
        run.wait_shown("tui: ready");
        let shell = run.child.id();
        let host = OnPty::descendant(shell, "goalcup").expect("goalcup below the shell");
        run.type_line("hello");
        run.wait_shown("tui: noted hello");
        run.close();
        run.wait();
        crate::common::support::wait_until_gone(host);
        // The hangup may reach the harness's read before its signal does.
        let end = harness(&rig, "tui.end");
        assert!(end == "hup" || end == "eof", "{end}");

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

    /// A Ctrl-\ reaches the foreground group: the harness ends on it, the host rides it out and
    /// seals the run.
    #[test]
    fn a_quit_from_the_terminal_ends_the_harness_and_the_run_still_seals() {
        let rig = Rig::chat();
        let mut run = rig.start_on_pty(&[DISK, LOCAL_WRAP_UP]);
        run.wait_shown("tui: ready");
        run.signal_foreground(libc::SIGQUIT);
        let status = run.wait();
        assert!(status.success(), "{status:?}\n{}", run.shown());
        assert_eq!(harness(&rig, "tui.end"), "quit");
        let (report, _) = rig.read_trace(&newest_entry(&rig));
        assert!(report.sealed, "{:?}", report.findings);
    }

    /// The terminal's Ctrl-C, Ctrl-\ and a hangup reach the harness, and neither the host nor the
    /// lock that keeps the machine awake: the inhibitor leads a process group of its own.
    #[test]
    fn the_terminals_signals_spare_the_host_and_the_sleep_lock() {
        let rig = Rig::chat();
        rig.tell("tui.endure", "");
        let mut run = rig.start_on_pty(&[DISK, LOCAL_WRAP_UP]);
        run.wait_shown("tui: ready");
        let host = run.child.id();
        let inhibitor = OnPty::descendant(host, "caffeinate")
            .or_else(|| OnPty::descendant(host, "systemd-inhibit"));
        if cfg!(target_os = "macos") {
            assert!(inhibitor.is_some(), "macOS ships caffeinate");
        }
        for (signal, said) in [
            (libc::SIGINT, "tui: endured int"),
            (libc::SIGQUIT, "tui: endured quit"),
            (libc::SIGHUP, "tui: endured hup"),
        ] {
            // The fake harness is a shell script, and a signal that lands between its last
            // output and its next blocking read is held until that read returns: it is not
            // lost, but nothing happens until the next input. Its own "waiting" line cannot
            // close that gap (it is printed before the read starts). So send the signal, and
            // send it again when the harness has not answered within a second. A signal that
            // was only slow is handled twice, which the harness reports twice and this test
            // does not mind: each stage looks for its own line.
            let before = run.shown().matches(said).count();
            let mut sent = None::<std::time::Instant>;
            wait_until(said, || {
                if run.shown().matches(said).count() > before {
                    return true;
                }
                // Never a resend into a host that has gone: that is the failure, said as such.
                assert!(matches!(run.child.try_wait(), Ok(None)), "the host lives");
                if sent.is_none_or(|at| at.elapsed() > std::time::Duration::from_secs(1)) {
                    run.signal_foreground(signal);
                    sent = Some(std::time::Instant::now());
                }
                false
            });
            assert!(matches!(run.child.try_wait(), Ok(None)), "the host lives");
            if let Some(inhibitor) = inhibitor {
                assert!(
                    goalcup_protocol::process::is_alive(inhibitor),
                    "the sleep lock lives after {said}"
                );
            }
        }
        // A signal sent twice can reach the harness while it is part way through reading this
        // line, and the half it had read is gone. Type the line again until the harness has
        // read it (it says `tui: bye`), then wait for the run to end: the host's wrap-up after
        // that reads no terminal.
        let mut typed = None::<std::time::Instant>;
        wait_until("the run to end on /exit", || {
            if matches!(run.child.try_wait(), Ok(Some(_))) {
                return true;
            }
            if run.shown().contains("tui: bye") {
                return false;
            }
            if typed.is_none_or(|at| at.elapsed() > std::time::Duration::from_secs(2)) {
                run.type_line("/exit");
                typed = Some(std::time::Instant::now());
            }
            false
        });
        let status = run.wait();
        assert!(status.success(), "{status:?}\n{}", run.shown());
        let (report, _) = rig.read_trace(&newest_entry(&rig));
        assert!(report.sealed, "{:?}", report.findings);
    }

    /// A harness that suspends itself (its own Ctrl-Z handling, alone or with its whole group) is
    /// continued at once: a run cannot be suspended, and a stopped harness would hold the
    /// participant's terminal frozen behind a host still waiting for it. The run still seals.
    #[test]
    fn a_harness_that_suspends_itself_is_continued_and_the_run_seals() {
        let rig = Rig::chat();
        // Under a job-control shell: a process group with no parent in its session is orphaned,
        // and the kernel discards a stop signal sent to one.
        let mut run = rig.start_under_a_shell_on_pty(&[DISK, LOCAL_WRAP_UP]);
        run.wait_shown("tui: ready");
        run.type_line("/suspend");
        run.wait_shown("tui: resumed");
        run.type_line("/suspend-group");
        wait_until("the second resume", || {
            run.shown().matches("tui: resumed").count() == 2
        });
        run.type_line("/exit");
        run.wait_shown("shell: ");
        assert!(run.shown().contains("shell: 0"), "{}", run.shown());
        run.wait();
        assert_eq!(harness(&rig, "tui.end"), "exit");
        let log = std::fs::read_to_string(run_dir(&rig).join("host.log")).unwrap();
        assert_eq!(
            log.matches("the harness stopped itself").count(),
            2,
            "{log}"
        );
        let (report, _) = rig.read_trace(&newest_entry(&rig));
        assert!(report.sealed, "{:?}", report.findings);
    }

    /// A panic on one of the host's threads while it is quiet is a message like any other: it is
    /// in `host.log`, and the terminal does not show it.
    #[test]
    fn a_panic_while_quiet_goes_to_the_log_not_the_terminal() {
        let rig = Rig::chat();
        let mut run =
            rig.start_on_pty(&[DISK, LOCAL_WRAP_UP, ("GOALCUP_TEST_PANIC_WHILE_QUIET", "1")]);
        run.wait_shown("tui: ready");
        run.type_line("/exit");
        let status = run.wait();
        let shown = run.shown();
        assert!(status.success(), "{status:?}\n{shown}");
        assert!(!shown.contains("panicked"), "{shown}");
        let log = std::fs::read_to_string(run_dir(&rig).join("host.log")).unwrap();
        assert!(
            log.contains("panicked") && log.contains("a test panic while the host is quiet"),
            "{log}"
        );
    }

    /// A chat-mode harness gets the participant's terminal and tools: their PATH (the fake
    /// harness is a `#!/usr/bin/env node` script, and the `node` it finds is only on that PATH),
    /// their TERM, COLORTERM, LANG and LC_*; the connector's own SHELL wins over theirs. It gets
    /// none of their proxy, base-URL or credential variables, and none of goalcup's own: its
    /// proxy is the run's loopback recorder.
    #[test]
    fn the_harness_gets_the_participants_terminal_and_tools_and_nothing_else() {
        let rig = Rig::chat();
        let participant_home = rig.root.join("participant-home");
        std::fs::create_dir_all(&participant_home).unwrap();
        let participant_home = participant_home.display().to_string();
        let mut run = rig.start_on_pty(&[
            ("HOME", participant_home.as_str()),
            ("CODEX_HOME", "/participant/codex-home"),
            ("NO_PROXY", "*"),
            ("LC_API_TOKEN", "not-a-real-token-0123456789"),
            DISK,
            LOCAL_WRAP_UP,
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
            ("LANG", "en_US.UTF-8"),
            ("LC_CTYPE", "en_US.UTF-8"),
            ("SHELL", "/bin/bash"),
            ("HTTPS_PROXY", "http://proxy.example:3128"),
            ("https_proxy", "http://proxy.example:3128"),
            ("ALL_PROXY", "socks5://proxy.example:1080"),
            ("OPENAI_API_KEY", "sk-not-a-real-key-0123456789"),
            ("ANTHROPIC_BASE_URL", "https://elsewhere.example"),
            ("GITHUB_TOKEN", "ghp_not_a_real_token_0123456789"),
        ]);
        run.wait_shown("tui: ready");
        run.type_line("/exit");
        let status = run.wait();
        assert!(status.success(), "{status:?}\n{}", run.shown());

        let env: BTreeMap<String, String> = harness(&rig, "tui.env")
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(env.get("PATH"), Some(&rig.path()), "{env:?}");
        for (key, value) in [
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
            ("LANG", "en_US.UTF-8"),
            ("LC_CTYPE", "en_US.UTF-8"),
            ("SHELL", "/bin/zsh"),
        ] {
            assert_eq!(
                env.get(key).map(String::as_str),
                Some(value),
                "{key}: {env:?}"
            );
        }
        for key in ["HTTPS_PROXY", "https_proxy"] {
            let proxy = env.get(key).expect("the run's own proxy");
            assert!(
                proxy.starts_with("http://goalcup:") && proxy.contains("@127.0.0.1:"),
                "{key} is the loopback recorder, not the participant's: {proxy}"
            );
        }
        // The launch's own values win over the participant's: the run's HOME, the connector's
        // isolated home, and the loopback proxy's exemptions.
        let home = env.get("HOME").expect("a HOME");
        assert_ne!(home, &participant_home, "{env:?}");
        assert!(
            home.starts_with(&rig.root.join("runs").display().to_string()),
            "{home}"
        );
        assert_eq!(
            env.get("CODEX_HOME"),
            Some(&rig.chat_home().display().to_string()),
            "{env:?}"
        );
        for key in ["NO_PROXY", "no_proxy"] {
            assert_eq!(
                env.get(key).map(String::as_str),
                Some(goalcup_cli::launch::CAPTURE_NO_PROXY),
                "{key}: {env:?}"
            );
        }
        for key in [
            "ALL_PROXY",
            "OPENAI_API_KEY",
            "ANTHROPIC_BASE_URL",
            "GITHUB_TOKEN",
            "LC_API_TOKEN",
        ] {
            assert!(!env.contains_key(key), "{key} reached the harness: {env:?}");
        }
        // Of goalcup's own variables only the launch's two reach it (D66): the harness marker
        // and the run's time-left file, under the configuration directory. None of the
        // participant's (`GOALCUP_CONFIG_DIR` among them).
        let own: Vec<&str> = env
            .keys()
            .map(String::as_str)
            .filter(|k| k.starts_with("GOALCUP_"))
            .collect();
        assert_eq!(
            own,
            ["GOALCUP_IN_HARNESS", "GOALCUP_TIME_LEFT_FILE"],
            "goalcup's own variables reached the harness: {env:?}"
        );
        assert!(
            env["GOALCUP_TIME_LEFT_FILE"]
                .starts_with(&rig.cfg.join("time-left").display().to_string()),
            "{env:?}"
        );
    }

    /// A connector whose run `HOME` is kept across runs (D59, `run_home = "shared"`): the run's
    /// `HOME` is `<cfg>/capture-homes/<id>.run-home`, a file the harness writes there is still
    /// there for the run's next segment, and `goalcup continue` takes the same home (and its
    /// lock) and restarts the harness in it.
    #[test]
    fn a_shared_run_home_is_kept_across_segments_and_continue_uses_it() {
        let rig = Rig::chat_with_shared_run_home();
        let server = UploadServer::start(&rig.root.join("server"), ServerOptions::default());
        rig.prepare_for(&server.url());
        let home = rig
            .cfg
            .join("capture-homes")
            .join(format!("{CHAT_HOME_ID}.run-home"));

        let mut first = OnPty::spawn(
            rig.chat_command_for(CHAT_HOME_ID, &[DISK, BEATING], Launcher::Direct),
            None,
        );
        first.wait_shown("tui: ready");
        assert_eq!(harness(&rig, "tui.home"), home.display().to_string());
        first.type_line("hello");
        first.wait_shown("tui: noted hello");
        first.type_line("/exit");
        let status = first.wait();
        assert!(status.success(), "{status:?}\n{}", first.shown());
        assert_eq!(
            std::fs::read_to_string(home.join("tui-runs.txt")).unwrap(),
            "run\n"
        );
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&home).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let entry = newest_entry(&rig);
        let (_, records) = rig.read_trace(&entry);
        assert!(native_bytes(&records).contains(r#""text":"hello""#));

        let mut second = rig.continue_on_pty(&[DISK, BEATING], None);
        second.wait_shown("tui: ready");
        assert_eq!(harness(&rig, "tui.home"), home.display().to_string());
        assert_eq!(
            std::fs::read_to_string(home.join("tui-runs.txt")).unwrap(),
            "run\nrun\n",
            "what the harness wrote in its HOME survived into the next segment"
        );
        let argv = harness(&rig, "tui.argv");
        assert!(argv.ends_with("resume\n--last"), "{argv}");
        second.type_line("again");
        second.wait_shown("tui: noted again");
        second.type_line("/exit");
        let status = second.wait();
        assert!(status.success(), "{status:?}\n{}", second.shown());
        let run_id = &server.run_ids()[0];
        assert_eq!(server.entry_ids(run_id).len(), 2, "two segments");
        assert!(home.is_dir(), "the shared run home is never removed");
    }

    /// A harness that closed before the first message (a trust question answered "no", a setup
    /// that ends the program) left no session. `goalcup continue` opens it the normal way, with
    /// no resume arguments, and says so in one line instead of promising the last session. Once
    /// the run has a session, the next continue picks it up.
    #[test]
    fn a_continue_of_a_run_with_no_session_opens_the_harness_the_normal_way() {
        let rig = Rig::chat();
        let server = UploadServer::start(&rig.root.join("server"), ServerOptions::default());
        rig.prepare_for(&server.url());
        let new_session = "with a new session: no saved session was found for this run.";

        // The first part: the harness opens and closes, and no message is sent.
        let mut first = rig.start_on_pty(&[DISK, BEATING]);
        first.wait_shown("tui: ready");
        first.type_line("/exit");
        let status = first.wait();
        assert!(status.success(), "{status:?}\n{}", first.shown());

        let mut second = rig.continue_on_pty(&[DISK, BEATING], None);
        second.wait_shown("tui: ready");
        let argv = harness(&rig, "tui.argv");
        assert!(!argv.contains("resume"), "no resume arguments: {argv}");
        let shown = second.shown();
        assert_eq!(shown.matches(new_session).count(), 1, "{shown}");
        assert!(!shown.contains("picks up its last session"), "{shown}");
        assert!(shown.contains("Type your one prompt"), "{shown}");
        second.type_line("hello");
        second.wait_shown("tui: noted hello");
        second.type_line("/exit");
        let status = second.wait();
        assert!(status.success(), "{status:?}\n{}", second.shown());

        // Now the run has a session: the next part resumes it.
        let mut third = rig.continue_on_pty(&[DISK, BEATING], None);
        third.wait_shown("tui: ready");
        let argv = harness(&rig, "tui.argv");
        assert!(argv.ends_with("resume\n--last"), "{argv}");
        let shown = third.shown();
        assert!(shown.contains("picks up its last session"), "{shown}");
        assert!(!shown.contains(new_session), "{shown}");
        third.type_line("/exit");
        let status = third.wait();
        assert!(status.success(), "{status:?}\n{}", third.shown());
        let run_id = &server.run_ids()[0];
        assert_eq!(server.entry_ids(run_id).len(), 3, "three segments");
    }

    /// A chat-mode run needs its terminal for input and for output: from a pipe, or with its
    /// output in a file, it is refused before anything is created.
    #[test]
    fn a_chat_run_needs_the_terminal_for_its_input_and_its_output() {
        let rig = Rig::chat();
        let out = rig
            .chat_command(&[DISK], Launcher::Direct)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{said}");
        assert!(said.contains("foreground of a terminal"), "{said}");
        assert!(rig.run_dirs().is_empty(), "{:?}", rig.run_dirs());

        let file = std::fs::File::create(rig.root.join("stdout.txt")).unwrap();
        let mut run = rig.start_on_pty_writing_to(&[DISK], file);
        let status = run.wait();
        assert_eq!(status.code(), Some(1), "{}", run.shown());
        assert!(
            run.shown().contains("foreground of a terminal"),
            "{}",
            run.shown()
        );
        assert!(rig.run_dirs().is_empty(), "{:?}", rig.run_dirs());
        assert!(rig.harness_file("tui.pid").is_none(), "no harness ran");
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
        // The verifier's reading of a chat-mode trace (lane R owns the verifier: findings are
        // reported, not changed here).
        let verified = std::process::Command::new(crate::common::support::verify_binary())
            .arg("--capture-integrity")
            .arg(rig.trace(&newest_entry(&rig)))
            .output()
            .unwrap();
        let report: serde_json::Value = serde_json::from_slice(&verified.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&verified.stdout)));
        println!("goalcup-verify on a chat-mode trace: {report:#}");
        assert_eq!(report["findings"], serde_json::json!([]), "{report:#}");
        assert_eq!(verified.status.code(), Some(0), "{report:#}");
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

    /// Headless mode is gone, and its `--goal-file` with it: the flag is a usage error, said
    /// before anything is created.
    #[test]
    fn the_goal_file_flag_is_gone() {
        let rig = Rig::chat();
        let folder = rig.run_folder();
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup"))
            .args(["start", "--harness", CHAT_ID, "--dev", "--goal-file"])
            .arg(folder.join("goal.txt"))
            .env_clear()
            .env("PATH", rig.path())
            .env("GOALCUP_CONFIG_DIR", &rig.cfg)
            .env("GOALCUP_CONNECTORS_DIR", &rig.capture_connectors)
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{said}");
        assert!(said.contains("--goal-file"), "{said}");
        assert!(said.contains("unexpected argument"), "{said}");
        assert!(rig.run_dirs().is_empty());
        let help = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup"))
            .args(["start", "--help"])
            .output()
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&help.stdout).contains("goal-file"),
            "{}",
            String::from_utf8_lossy(&help.stdout)
        );
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
