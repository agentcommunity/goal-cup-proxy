//! Ctrl-C inside the abandon window.
//!
//! # Why this is a test binary of its own, holding one test
//!
//! The interrupt flag is **process-global** — a signal handler has nowhere else to put what
//! it saw — and `cargo test` runs the `#[test]`s of one binary on a pool of threads. A test
//! that raised `SIGINT` beside a sibling would set that sibling's flag too, and the sibling
//! would abandon an entry nobody interrupted. So the whole of this file is one test, run in
//! a process of its own, and it raises a real `SIGINT` at this process rather than reaching
//! for the flag directly: the handler being installed at all is half of what is under test.
//!
//! The phases cover the shapes the window has and the one shape it does not. Attach mode
//! stands at the gate with nothing spawned. A launch whose full native authority was
//! captured can be stopped and abandoned; an unresolved group-less native process instead
//! leaves a durable manual-cleanup latch, so nothing can relaunch over it.
//!
//! Phase 3 is past `register_process`, where the window is closed. A launched TUI shares
//! this terminal, so an ordinary Ctrl-C inside the harness sets this same flag — and reading
//! it there would report an armed, recorded, ordinary run as "interrupted before the run
//! began" and ask the daemon to abandon a stamped entry.
//!
//! The flag is put back by `start_with` itself, so each phase raises its own `SIGINT`
//! rather than living off the first one — which is the same reason a participant who runs
//! `goalcup start` twice in one process is not refused by the first run's Ctrl-C.
#![cfg(feature = "v1-compat")]
#![cfg(unix)]

mod support;

#[test]
fn a_ctrl_c_before_handoff_never_leaves_an_automatically_relaunchable_child() {
    // The handler first, or the raise below ends this process the way an uninstalled
    // SIGINT ends every process.
    goalcup_cli::arming::watch_for_interrupt();
    goalcup_cli::arming::raise_sigint_for_tests();
    assert!(
        goalcup_cli::arming::interrupted(),
        "a raised SIGINT is delivered to the raising thread before `raise` returns"
    );

    // Phase 1 — attach mode: the gate is where the participant is standing when they press
    // it, and there is no child.
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    let mut o = support::start_options("codex-cli");
    o.attach = true;
    let err = goalcup_cli::commands::start::start_with(
        &m,
        &support::connectors_dir(),
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(
        err.exit_code(),
        1,
        "an interrupt is the participant's own doing, not a gate's refusal: {err}"
    );
    assert_eq!(err.to_string().lines().count(), 1, "one line: {err}");
    assert_eq!(m.abandoned().len(), 1, "the entry was unmade");
    assert!(
        goalcup_cli::entries::read_entry(tmp.path(), &m.info().entry_id).is_err(),
        "and so was the local record"
    );

    assert!(
        !goalcup_cli::arming::interrupted(),
        "`start` puts the flag back on its way out, so the next run in this process is not \
         refused over this one's Ctrl-C"
    );

    // Phase 2 — launch mode: a child is already running and the daemon has not been told
    // about it. It must not be handed over, and it must not be left behind.
    //
    // Use the TUI wrapper/native shape here. It owns no private process group, so killing
    // only the retained wrapper would orphan `sleep`; the captured native instance must be
    // stopped before the wrapper even though no `/process` request is made.
    goalcup_cli::arming::raise_sigint_for_tests();
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    let mut o = support::start_options("codex-cli");
    o.dev = true;
    let connectors = support::connectors_dir_with_a_tui_native_child("codex-cli");
    let pidfile = std::path::Path::new(&m.info().workspace).join("pid.txt");
    let err = goalcup_cli::commands::start::start_with(
        &m,
        &connectors,
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), 1, "{err}");
    assert!(
        err.to_string().contains("interrupted before the run began"),
        "the refusal under test is the interrupt's and not the seed's: {err}"
    );
    assert!(
        !m.calls().contains(&"register_process".to_string()),
        "an interrupted start hands the daemon nothing to terminate later"
    );
    assert_eq!(m.abandoned().len(), 1);
    let native_pid: u32 = std::fs::read_to_string(pidfile)
        .expect("native pid was exposed before handle construction completed")
        .trim()
        .parse()
        .unwrap();
    support::wait_until_gone(native_pid);
    assert!(
        goalcup_cli::entries::read_entry(tmp.path(), &m.info().entry_id).is_err(),
        "and the local record went with it"
    );

    // Phase 2b — the same pre-POST interrupt when the group-less wrapper's native child
    // exists but has not yet exec'd into its pinned name. The CLI cannot signal that child
    // by a later name lookup, and wrapper exit is not proof it disappeared. The durable
    // unresolved-native latch therefore keeps the entry and blocks all automatic recovery.
    goalcup_cli::arming::raise_sigint_for_tests();
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    let mut o = support::start_options("codex-cli");
    o.dev = true;
    let connectors = support::connectors_dir_with_an_unresolved_tui_native_child("codex-cli");
    let pidfile = std::path::Path::new(&m.info().workspace).join("pid.txt");
    goalcup_cli::commands::start::start_with(
        &m,
        &connectors,
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut Vec::new(),
    )
    .expect_err("the pre-POST interrupt is reported");
    assert!(!m.calls().contains(&"register_process".to_string()));
    let native_pid: u32 = std::fs::read_to_string(pidfile)
        .expect("the as-yet unnamed native process exists")
        .trim()
        .parse()
        .unwrap();
    assert!(goalcup_protocol::process::is_alive(native_pid));
    assert!(m.abandoned().is_empty());
    let path = goalcup_cli::entries::entry_path(tmp.path(), &m.info().entry_id);
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        raw.get("pending_unresolved_native_cleanup")
            .and_then(|value| value.get("native_binary")),
        Some(&serde_json::Value::String("sleep".to_string()))
    );
    assert!(raw.get("pending_process_cleanup").is_some());
    std::process::Command::new("kill")
        .args(["-KILL", &native_pid.to_string()])
        .status()
        .unwrap();
    support::wait_until_gone(native_pid);

    // Phase 3 — past the hand-off: the harness raises the `SIGINT` itself, a beat after it
    // starts, which is where a participant's Ctrl-C at a TUI lands. The run is stamped and
    // was recorded, so this is an ordinary run that ends ordinarily.
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    // The harness waits for this file; the watched daemon creates it once `register_process`
    // has been accepted, so the signal lands past the hand-off however long discovery took.
    let watched = support::watched(&m);
    watched.touch_after_register(&std::path::Path::new(&m.info().workspace).join("registered"));
    let mut o = support::start_options("codex-cli");
    o.dev = true;
    let connectors = support::connectors_dir_with_a_harness_that_interrupts_its_parent("codex-cli");
    let mut out = Vec::new();
    let outcome = goalcup_cli::commands::start::start_with(
        &watched,
        &connectors,
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut out,
    )
    .expect("a Ctrl-C inside the harness is not this command's refusal");
    assert!(
        matches!(outcome.gate, goalcup_cli::arming::GateOutcome::NotRequired),
        "the gate is the one the daemon answered: {:?}",
        outcome.gate
    );
    assert!(
        m.calls().contains(&"register_process".to_string()),
        "the harness was handed over"
    );
    assert!(
        m.abandoned().is_empty(),
        "and a stamped, recorded run is not abandoned over the terminal's Ctrl-C"
    );
    assert!(
        goalcup_cli::entries::read_entry(tmp.path(), &m.info().entry_id).is_ok(),
        "nor is this machine's note of it removed"
    );
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("nothing was abandoned"),
        "the interrupt reaches the last line and nothing else — and this text is also the \
         proof the signal was delivered at all: {text:?}"
    );

    goalcup_cli::arming::clear_interrupt_for_tests();
}
