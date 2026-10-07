//! D22: what the CLI hands the daemon after a launch, and the one thing it never does with
//! the process it launched.

mod support;

// Not `#[cfg(unix)]`: this file's four spawning tests below are not themselves cfg(unix), and
// table_lock.rs is pure std, so a cfg'd module here would break a non-unix build.
#[path = "../../goalcup-daemon/tests/support/table_lock.rs"]
mod table_lock;

#[cfg(feature = "v1-compat")]
use goalcup_cli::daemon::MockDaemonClient;
#[cfg(feature = "v1-compat")]
use goalcup_cli::launch::{LaunchKind, handle, plan, spawn};
#[cfg(feature = "v1-compat")]
use goalcup_protocol::control::DaemonClient;
#[cfg(feature = "v1-compat")]
use goalcup_protocol::manifest::{Kill, KillStrategy, LaunchMode};
use goalcup_protocol::process::{descendants, find_named, snapshot};

#[test]
fn discovery_finds_the_grandchild_the_npm_wrapper_hides() {
    // M0 §2.5: `kill -9` on the wrapper left the native binary holding the thread-writer
    // lock, and the next resume silently became a new thread.
    //
    // The pid file is a path inside a temporary DIRECTORY rather than a `NamedTempFile`,
    // because `support::wait_for_file` waits for a path to exist and a `NamedTempFile`
    // exists from the moment it is created — the wait would return on an empty file and the
    // parse below would be a race this test lost on a busy machine.
    let _lock = table_lock::TableLock::acquire();
    let tmp = tempfile::tempdir().unwrap();
    let pidfile = tmp.path().join("grandchild.pid");
    let mut child = std::process::Command::new("sh")
        .arg(support::fixture("wrapper-fork.sh"))
        .arg(&pidfile)
        .spawn()
        .unwrap();
    support::wait_for_file(&pidfile);
    let grandchild: u32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let table = snapshot().unwrap();
    assert!(descendants(child.id(), &table).contains(&grandchild));
    assert_eq!(find_named(child.id(), "sleep", &table), Some(grandchild));
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(feature = "v1-compat")]
#[test]
fn a_tui_handle_records_no_pgid_and_a_server_handle_records_one() {
    let _lock = table_lock::TableLock::acquire();
    let info = support::workspace_info();
    let v = goalcup_cli::vars::from_workspace(&info);
    let tmp = tempfile::tempdir().unwrap();

    let mut c = support::connector("claude-code-cli");
    c.manifest.launch.program = "sh".into();
    c.manifest.launch.args = vec!["-c".into(), "sleep 5".into()];
    let p = plan(
        &c,
        &v,
        tmp.path(),
        LaunchKind::Start,
        &support::launch_env(),
    )
    .unwrap();
    assert_eq!(p.mode, LaunchMode::Tui);
    let mut s = spawn(&p).unwrap();
    let h = handle(&p, &s, support::NOW).unwrap();
    assert_eq!(
        h.pgid, None,
        "a TUI in its own process group loses the terminal on the first read (SIGTTIN)"
    );
    let _ = s.child.kill();
    let _ = s.child.wait();

    // No connector ships in `Server` mode. The mode still decides the one thing asserted below,
    // so this case declares it — which is what a manifest for a genuinely headless harness
    // would do.
    let mut c = support::connector("codex-cli");
    c.manifest.launch.mode = LaunchMode::Server;
    c.manifest.launch.program = "sh".into();
    c.manifest.launch.args = vec!["-c".into(), "sleep 5".into()];
    let p = plan(
        &c,
        &v,
        tmp.path(),
        LaunchKind::Start,
        &support::launch_env(),
    )
    .unwrap();
    assert_eq!(p.mode, LaunchMode::Server);
    let mut s = spawn(&p).unwrap();
    let h = handle(&p, &s, support::NOW).unwrap();
    assert!(
        h.pgid.is_some(),
        "a server owns its group: there is no tty to lose"
    );
    assert_eq!(
        h.pgid,
        Some(h.pid),
        "and the group it owns is its own: `process_group(0)` and nothing else"
    );
    let _ = s.child.kill();
    let _ = s.child.wait();
}

#[cfg(feature = "v1-compat")]
#[test]
fn the_handle_records_what_the_daemon_needs_to_check_identity_and_no_secret_at_all() {
    let _lock = table_lock::TableLock::acquire();
    let info = support::workspace_info();
    let v = goalcup_cli::vars::from_workspace(&info);
    let tmp = tempfile::tempdir().unwrap();
    let mut c = support::connector("claude-code-cli");
    c.manifest.launch.program = "sh".into();
    c.manifest.launch.args = vec!["-c".into(), "sleep 5".into()];
    c.manifest.kill = Kill {
        strategy: KillStrategy::NativePid,
        native_binary: Some("sleep".into()),
        grace_seconds: 10,
    };
    let p = plan(
        &c,
        &v,
        tmp.path(),
        LaunchKind::Start,
        &support::launch_env(),
    )
    .unwrap();
    let mut s = spawn(&p).unwrap();
    let h = handle(&p, &s, support::NOW).unwrap();

    assert_eq!(
        h.argv, p.argv,
        "the daemon compares a live command line against this"
    );
    assert!(
        h.program.starts_with('/'),
        "the resolved path, so a different binary is visible"
    );
    assert_eq!(h.grace_seconds, 10);
    assert_eq!(h.strategy, KillStrategy::NativePid);
    assert!(
        h.process_start.is_some(),
        "the launch captures the spawned process instance before handing off authority"
    );
    assert_eq!(
        h.native_binary.as_deref(),
        Some("sleep"),
        "the native identity is manifest-pinned and distinct from the sh launcher"
    );
    assert_eq!(h.launched_at, support::NOW);
    for k in ["CLAUDE_CONFIG_DIR", "OTEL_LOG_RAW_API_BODIES"] {
        assert!(
            h.env_keys.iter().any(|e| e == k),
            "{k} is pinned and recorded by name"
        );
    }
    let serialised = serde_json::to_string(&h).unwrap();
    assert!(
        !serialised.contains(&info.harness_home),
        "a value is not a key"
    );

    // The hand-off itself. Nothing here reads anything back out of the mock — the assertion is
    // that a `ProcessHandle` goes over the `DaemonClient` trait as `register_process` — so
    // `armed_ok` is used.
    let m = MockDaemonClient::armed_ok(tmp.path(), "claude-code-cli");
    m.register_process(
        &info.entry_id,
        &goalcup_protocol::process::ProcessRegistration {
            generation: info.process_generation,
            handle: h.clone(),
        },
    )
    .unwrap();
    assert!(m.calls().contains(&"register_process".to_string()));
    let _ = s.child.kill();
    let _ = s.child.wait();
}

/// The program on the handle is the program the child was exec'd from, and nothing that
/// happens after the spawn changes it.
///
/// The window between `spawn` and `handle` is short and it is not empty: a version manager
/// can rewrite a shim, an installer can finish, a `PATH` can be reordered. A `handle` that
/// resolved the name a second time would record a program the child never ran, and the
/// daemon's `same_process` guard would then reject a harness that is doing exactly what it
/// was told to — a run that dies at its first check for a reason nobody can see. So `spawn`
/// hands the resolution back and `handle` takes it.
#[cfg(feature = "v1-compat")]
#[test]
fn the_handle_names_the_program_that_was_exec_d_even_when_the_path_moves_under_it() {
    let _lock = table_lock::TableLock::acquire();
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let shim = bin.join("goalcup-fake-harness");
    std::fs::write(&shim, "#!/bin/sh\nexec sleep 30\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let info = support::workspace_info();
    let v = goalcup_cli::vars::from_workspace(&info);
    let mut env = support::launch_env();
    let inherited = env.get("PATH").cloned().unwrap_or_default();
    env.insert("PATH".to_string(), format!("{}:{inherited}", bin.display()));

    let mut c = support::connector("claude-code-cli");
    c.manifest.launch.program = "goalcup-fake-harness".into();
    c.manifest.launch.args = Vec::new();
    let mut p = plan(&c, &v, tmp.path(), LaunchKind::Start, &env).unwrap();
    let mut s = spawn(&p).unwrap();

    // The world moves under the launch, between the two calls: the shim is deleted and the
    // environment the launch was planned in no longer names the directory it was in. The
    // child is still running the file that was there.
    std::fs::remove_file(&shim).unwrap();
    p.env_base
        .insert("PATH".to_string(), "/nonexistent".to_string());

    let h = handle(&p, &s, support::NOW).unwrap();
    assert_eq!(
        h.program,
        shim.display().to_string(),
        "the handle named a program the child never exec'd"
    );
    let _ = s.child.kill();
    let _ = s.child.wait();
}

#[test]
fn this_crate_never_terminates_anything() {
    // The one invariant whose violation is silent: two killers with different grace
    // periods racing at the deadline. The kill is `goalcup_daemon::terminate`.
    //
    // `support::walk` takes a path relative to the repository root, so the crate's own source
    // directory is spelled out.
    //
    // The signal check carries three exemptions, and each is written down rather than
    // glossed.
    //
    // `preflight/probe.rs`: the version probe spawns the harness with `--version` in a group
    // of its own and `killpg`s that group at its own ten-second deadline. That child is the
    // probe's, it is dead before `start` has decided anything, and it is not the launched
    // harness — which is the process this rule is about.
    //
    // `commands/start.rs`: the abandon path signals the group of a harness the daemon was
    // never successfully told about, on a start that is failing. Nothing will measure that
    // process, nothing will seal what it writes, and the workspace it is running in is about
    // to be deleted — `commands::start::launch`'s own header is where that reasoning lives.
    // It is not a deadline, it has no grace period and it reads no clock: the terminator this
    // rule is about is `goalcup_daemon::terminate`, at 168 hours, and a second one racing it
    // is the failure whose symptom is silence.
    //
    // `released_child.rs` (H-10 4b, the root ruling under D40): when a seal raced in between
    // a capture child's durable registration and its release, the launcher ends the child it
    // has just released, with a grace and a SIGKILL, so that no harness runs on a sealed entry.
    // It is the one exemption that reads a clock, which is why it is its own file and the
    // exemption is by name: it signals only this process's own unreaped `Child` and that
    // child's native descendant bound by creation identity, never anything recorded, and
    // `goalcup_daemon::terminate` stays the only terminator of a recorded run.
    //
    // The `start.rs` exemption stays ONE file, which is why `commands::start::kill` is `pub(crate)`:
    // `goalcup resume` reaches the same state — a relaunched harness the daemon would not
    // accept — and calls that function rather than growing a second copy of the group walk
    // in `commands/resume.rs`, which this sweep would then have had to exempt as well.
    //
    // A NEW file that signals is still caught, which is the whole point of the sweep.
    //
    // The sweep asserts what it walked before it asserts anything about the contents:
    // `support::walk` answers an empty vector for a directory that is not there, and a
    // `for` over nothing passes every assertion inside it. A rename that moved this crate's
    // source would otherwise turn this test green and silent.
    let sources = support::walk("crates/goalcup-cli/src");
    assert!(
        !sources.is_empty(),
        "the sweep walked no file at all: `crates/goalcup-cli/src` is not where this \
         crate's source lives any more"
    );
    assert!(
        sources
            .iter()
            .any(|p| p.file_name().and_then(|n| n.to_str()) == Some("launch.rs")),
        "the sweep did not reach `launch.rs`, which is the file that spawns the harness: \
         {sources:?}"
    );
    for entry in sources {
        let s = std::fs::read_to_string(&entry).unwrap();
        assert!(!s.contains("/bin/kill"), "{entry:?} signals a process");
        assert!(
            !s.contains("fn terminate"),
            "{entry:?} defines a terminator"
        );
        // The three files that signal, and the reason each one is allowed to. Only the probe half
        // of `preflight/{mod,gates,probe}.rs` kills anything, so the exemption is matched on the
        // tail of the path, and `preflight/gates.rs` is NOT exempt — a `SIGKILL` appearing there
        // is the sweep doing its job.
        let exempt = [
            "preflight/probe.rs",
            "commands/start.rs",
            "released_child.rs",
        ]
        .iter()
        .any(|tail| entry.ends_with(tail));
        if exempt {
            continue;
        }
        assert!(
            !s.contains("SIGKILL") && !s.contains("SIGTERM"),
            "{entry:?}"
        );
    }
}
