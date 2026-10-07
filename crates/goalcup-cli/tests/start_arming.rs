//! D16's arming gate, as the participant meets it: the ordinary case, where the first
//! transcript record arms the entry and there is nothing to wait for, and the gate the daemon
//! has already settled.
//!
//! The whole of this file is driven against `MockDaemonClient`, which can put the status
//! payload into `ArmingState::Refused`. It can also serve a status document that does not
//! verify, or one about another entry: the two shapes a real harness never produces and the
//! unauthenticated status route can.
//!
//! Every connector arms on its own transcript, so no test here waits on a manifest-required
//! start signal.
#![cfg(feature = "v1-compat")]

mod support;

use goalcup_cli::arming::{GateOutcome, START_SIGNAL_IN_WORDS, refusal_text, wait_foreground};
use goalcup_protocol::control::ControlError;
use std::time::{Duration, Instant};

#[test]
fn the_gate_does_not_block_and_does_not_say_anything_is_wrong() {
    for id in ["claude-code-cli", "codex-cli"] {
        let tmp = tempfile::tempdir().unwrap();
        let m = support::mock_armed(&tmp, id);
        let c = support::connector(id);
        let mut out = Vec::new();
        let started = std::time::Instant::now();
        let r = wait_foreground(&m, &c, &m.info().entry_id, &m.run_public_key(), &mut out);
        assert!(matches!(r.outcome, GateOutcome::NotRequired), "{id}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{id}: it must not block"
        );
        let text = String::from_utf8(out).unwrap().to_lowercase();
        assert!(text.contains("first transcript record"), "{id}: {text:?}");
        for alarming in ["error", "failed", "not recording", "problem"] {
            assert!(
                !text.contains(alarming),
                "{id}: nothing is wrong, and this said {alarming:?}: {text:?}"
            );
        }
        assert!(
            !m.calls().contains(&"wait_start_signal".to_string()),
            "{id}: there is nothing to long-poll for"
        );
    }
}

/// The fourth arming state: a gate the daemon has already refused is not a gate
/// to wait longer for.
#[test]
fn a_gate_the_daemon_refused_is_refused_here_and_never_reported_as_a_timeout() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    m.set_arming_refused("this harness home already holds a tracked entry");
    let c = support::connector("codex-cli");
    let mut out = Vec::new();
    let r = wait_foreground(&m, &c, &m.info().entry_id, &m.run_public_key(), &mut out);
    match r.outcome {
        GateOutcome::Refused(detail) => assert!(detail.contains("already holds")),
        other => panic!("a refused gate is not a timeout: {other:?}"),
    }
    let text = String::from_utf8(out).unwrap().to_lowercase();
    assert!(
        !text.contains("wait"),
        "it will never arm; do not ask for patience: {text:?}"
    );
    assert!(
        text.contains("already holds"),
        "and it carries the daemon's own reason: {text:?}"
    );
}

/// A refusal is believed off a document that VERIFIES, and off no other.
///
/// The status route is unauthenticated, so a local process that bound the control
/// port first can serve a document of its own — and a `Refused` read off one would become
/// `GateOutcome::Refused`, which `commands::start::verdict` turns into exit 4 and an
/// abandoned entry. The one document this gate acts on is checked against the run key this
/// run was handed, and a document that does not check out is "no answer".
#[test]
fn a_settled_refusal_whose_document_does_not_verify_is_no_answer_and_never_an_abandon() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    m.set_arming_refused("this harness home already holds a tracked entry");
    // The bytes the daemon signed, edited without re-signing.
    m.tamper_status_payload();
    let c = support::connector("codex-cli");
    let mut out = Vec::new();
    let r = wait_foreground(&m, &c, &m.info().entry_id, &m.run_public_key(), &mut out);
    assert!(
        matches!(r.outcome, GateOutcome::NotRequired),
        "an unverifiable document decided nothing: {:?}",
        r.outcome
    );
    let text = String::from_utf8(out).unwrap();
    assert!(
        !text.contains("already holds"),
        "and not one word of it reached the screen in the daemon's voice: {text:?}"
    );
    assert!(
        !text.contains(&m.info().workspace),
        "nor the path it named, which is the path the sentence that matters most points at: \
         {text:?}"
    );
}

/// The half a signature cannot cover: one run key signs every document of the run, so a
/// payload about ANOTHER entry verifies exactly as well as this one's.
///
/// `refusal_text` prints that payload's `workspace` inside "do not type your goal into a
/// session in that folder" — so a document that verifies and is about the wrong run named
/// the wrong folder in the one sentence a participant most needs to be right.
#[test]
fn a_settled_refusal_about_another_entry_is_no_answer_even_though_it_verifies() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    m.set_arming_refused("this harness home already holds a tracked entry");
    m.set_status_entry_id("entry-somebody-elses");
    let c = support::connector("codex-cli");
    let mut out = Vec::new();
    let r = wait_foreground(&m, &c, &m.info().entry_id, &m.run_public_key(), &mut out);
    assert!(
        matches!(r.outcome, GateOutcome::NotRequired),
        "a document about another entry decided nothing: {:?}",
        r.outcome
    );
    let text = String::from_utf8(out).unwrap();
    assert!(
        !text.contains("already holds") && !text.contains("entry-somebody-elses"),
        "and neither its reason nor its id reached the screen: {text:?}"
    );
}

/// The refusal text keeps every promise the participant needs and adds the daemon's reason,
/// and it never invites the participant to be patient with a gate that has been settled.
#[test]
fn the_refusal_text_for_a_settled_gate_carries_the_reason_and_asks_for_no_patience() {
    let t = refusal_text(
        "entry-0001",
        "/tmp/ws-0001",
        "this harness home already holds a tracked entry",
    );
    assert!(t.contains("did not start your clock"), "{t:?}");
    assert!(t.contains("already holds a tracked entry"), "{t:?}");
    assert!(t.contains("/tmp/ws-0001"), "{t:?}");
    assert!(
        t.to_lowercase().contains("do not type your goal"),
        "M0: a session opened before arming runs blind and silent for its whole life: {t:?}"
    );
    // The daemon's wording, not the manifest's leftover `arming.signal` (`SessionStart`),
    // which no daemon waits for.
    assert!(t.contains(START_SIGNAL_IN_WORDS), "{t:?}");
    assert!(!t.contains("SessionStart"), "{t:?}");
    assert!(!t.to_lowercase().contains("wait"), "{t:?}");
    assert!(
        !t.contains("test-grant"),
        "no secret is ever on this screen"
    );
}

/// A gate the daemon settled leaves under 4, never 2, with the daemon's own words in the
/// refusal and the entry abandoned for the same reason.
#[test]
fn a_gate_the_daemon_refused_also_leaves_under_four_and_abandons() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    m.set_arming_refused("this harness home already holds a tracked entry");
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
    assert_eq!(err.exit_code(), 4);
    assert_ne!(err.exit_code(), 2);
    assert!(
        err.to_string().contains("already holds a tracked entry"),
        "the refusal carries the daemon's reason: {err}"
    );
    assert_eq!(m.abandoned().len(), 1);
    assert!(
        goalcup_cli::entries::read_entry(tmp.path(), &m.info().entry_id).is_err(),
        "and the local record went with it"
    );
}

/// Attach mode with nothing to wait for leaves the entry standing: nothing is abandoned.
#[test]
fn an_attach_start_leaves_the_entry_standing() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    let mut o = support::start_options("codex-cli");
    o.attach = true;
    let mut out = Vec::new();
    let outcome = goalcup_cli::commands::start::start_with(
        &m,
        &support::connectors_dir(),
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut out,
    )
    .unwrap();
    assert!(matches!(outcome.gate, GateOutcome::NotRequired));
    assert!(m.abandoned().is_empty(), "nothing was unmade");
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("first transcript record"), "{text:?}");
}

/// Launch mode: the gate is read beside the harness, the harness is handed over, and the
/// gate's account is printed only once the TUI lets go of the screen.
///
/// The harness is the shell fixture that writes a file and exits, for the reason
/// `support`'s header gives — the shipped `codex-cli` recipe spawns `codex`, and a test
/// that ran one would be about the machine it ran on. `--dev` is on because that fixture
/// edits the connector directory, which is the door behaving exactly as it should.
#[test]
fn a_launch_reports_the_gate_once_the_harness_has_let_go_of_the_screen() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    let mut o = support::start_options("codex-cli");
    o.dev = true;
    let connectors = support::connectors_dir_with_a_harness_that_exits("codex-cli");
    let mut out = Vec::new();
    let outcome = goalcup_cli::commands::start::start_with(
        &m,
        &connectors,
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut out,
    )
    .unwrap();
    assert!(matches!(outcome.gate, GateOutcome::NotRequired));
    assert!(m.abandoned().is_empty());
    assert!(
        m.calls().contains(&"register_process".to_string()),
        "the harness was handed over"
    );
    let text = String::from_utf8(out).unwrap();
    let gate = text
        .lines()
        .position(|l| l.contains("first transcript record"))
        .expect("the gate's account is on the screen");
    let recording = text
        .lines()
        .position(|l| l.contains("Recording. Leave this running."))
        .expect("and so is the line the launch printed before it");
    assert!(
        recording < gate,
        "the gate does not print over a TUI that still owns the terminal: {text:?}"
    );
}

/// Launch mode's other outcome for the same gate: the daemon has already refused it. The
/// harness still runs to its own exit — `register_process` handed it over, and this command
/// does not cut a TUI off early merely because the answer waiting on the other side of its
/// screen is bad news. Only once the harness has let go does `verdict`'s `Window::Closed`
/// branch judge the gate, leave under 4, and abandon the entry (abandon happens either
/// side of `register_process`, never skipped for a settled refusal).
///
/// Comparing output-line order against "Recording..." does not prove this.
/// `start.rs` reads the gate through `watch_beside` *before* `child.wait()` — it already
/// holds `report` (a settled `Refused`, here) while the harness is still running — and only
/// the printing of that report and the resulting `abandon` are supposed to wait for
/// `wait()`. "Recording..." is written before either happens, so a regression that acted on
/// `report` immediately, without ever waiting for the child, would print the very same two
/// lines in the very same order.
///
/// The fixture instead SLEEPS (`connectors_dir_with_a_harness_that_sleeps`, written for
/// exactly this discrimination) and the test watches the mock's own record of what the
/// daemon was told, not the screen. Once `register_process` names a live pid, this polls for
/// several hundred milliseconds — utterly negligible next to the fixture's 300-second sleep,
/// but far longer than an in-process, no-I/O `abandon()` call would take to land if one ran
/// immediately — asserting the pid stays alive and `m.abandoned()` stays empty throughout.
/// Only after that window does the test kill the harness itself, and only then does it check
/// the same outcome the text-order version checked.
/// Kills the sleeping harness unconditionally once its pid is known, so a panic from any
/// assertion below — including the one this test exists to trip — still stops it promptly
/// instead of leaving `thread::scope` blocked in the spawned thread's `child.wait()` for the
/// fixture's full 300-second sleep.
///
/// SIGTERM first, then SIGKILL if it is still alive after a short wait: this is a test
/// fixture, not a participant's real process, so there is no reason to wait longer for a
/// graceful exit. Modeled on `finish_matrix.rs`'s `KillTreeOnDrop`, kept local here because
/// this test only ever has one pid and no dependency on that file's process-group logic.
struct KillHarnessOnDrop(u32);

impl Drop for KillHarnessOnDrop {
    fn drop(&mut self) {
        let pid = self.0;
        if !goalcup_protocol::process::is_alive(pid) {
            return;
        }
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if !goalcup_protocol::process::is_alive(pid) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

#[test]
fn a_launch_whose_gate_never_armed_leaves_under_four_after_the_harness_exits() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    m.set_arming_refused("this harness home already holds a tracked entry");
    let mut o = support::start_options("codex-cli");
    o.dev = true;
    let connectors = support::connectors_dir_with_a_harness_that_sleeps("codex-cli");
    let env = support::clean_env_with_no_harness_installed();
    let mut out = Vec::new();

    let result = std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            goalcup_cli::commands::start::start_with(
                &m,
                &connectors,
                tmp.path(),
                &env,
                &o,
                &mut out,
            )
        });

        // The one real event this test can wait on: `register_process` hands the daemon a
        // live pid, and nothing here can ask about the child before that.
        let registration_deadline = Instant::now() + Duration::from_secs(10);
        let pid = loop {
            if let Some(handed) = m.last_registered_handle() {
                break handed.pid;
            }
            assert!(
                Instant::now() < registration_deadline,
                "register_process never landed inside the window"
            );
            std::thread::sleep(Duration::from_millis(10));
        };

        // Armed as soon as the pid is known, so a panic from any assertion below — the one
        // this test exists to trip included — still kills the harness instead of leaving the
        // spawned thread blocked in `child.wait()` on a 300-second sleep.
        let _kill_harness_on_drop = KillHarnessOnDrop(pid);

        // The discriminator: nothing has told the sleeping harness to stop, so a build that
        // genuinely waits on `child.wait()` before acting on the gate cannot have printed
        // the refusal or abandoned anything yet, for as long as this keeps checking.
        let watch_deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < watch_deadline {
            assert!(
                goalcup_protocol::process::is_alive(pid),
                "the sleeping harness (pid {pid}) exited on its own before this test killed it"
            );
            assert!(
                m.abandoned().is_empty(),
                "the abandon ran while the harness (pid {pid}) the daemon owns was still \
                 alive: the gate was judged before the harness exited, not after"
            );
            std::thread::sleep(Duration::from_millis(20));
        }

        std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .unwrap();
        support::wait_until_gone(pid);

        handle.join().unwrap()
    });

    let err = result.unwrap_err();
    assert_eq!(err.exit_code(), 4, "{err}");
    assert!(
        err.to_string().contains("already holds a tracked entry"),
        "the refusal carries the daemon's reason: {err}"
    );
    assert!(
        m.calls().contains(&"register_process".to_string()),
        "the harness was handed over before the gate was judged"
    );
    assert_eq!(
        m.abandoned().len(),
        1,
        "a run the tracker's gate never armed is a run there is no entry for, even past \
         register_process — and only once the harness had actually exited"
    );
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("already holds a tracked entry"),
        "and the gate's refusal reaches the screen too: {text:?}"
    );
}

/// Second half: the local record is this machine's only note of an entry the
/// tracker may still be holding, so an abandon the daemon refused leaves it standing.
#[test]
fn an_abandon_the_daemon_refused_keeps_this_machines_note_of_the_entry_and_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    m.set_arming_refused("this harness home already holds a tracked entry");
    m.fail_next(
        "abandon",
        ControlError::Conflict("this entry has already started".to_string()),
    );
    let mut o = support::start_options("codex-cli");
    o.attach = true;
    let mut out = Vec::new();
    let err = goalcup_cli::commands::start::start_with(
        &m,
        &support::connectors_dir(),
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut out,
    )
    .unwrap_err();
    assert_eq!(
        err.exit_code(),
        4,
        "the cleanup does not change what refused the run: {err}"
    );
    assert!(
        m.calls().contains(&"abandon".to_string()),
        "the abandon was asked for"
    );
    assert!(m.abandoned().is_empty(), "and refused");
    assert!(
        goalcup_cli::entries::read_entry(tmp.path(), &m.info().entry_id).is_ok(),
        "so the record is still there for `goalcup entries` to name"
    );
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("already started"),
        "the daemon's own words for the refusal: {text:?}"
    );
    assert!(
        text.contains("has been kept"),
        "and the screen says the note was kept: {text:?}"
    );
}

#[test]
fn a_busy_abandon_retries_then_removes_both_daemon_and_local_state() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    m.set_arming_refused("this harness home already holds a tracked entry");
    m.busy_once("abandon");
    let mut options = support::start_options("codex-cli");
    options.attach = true;

    let error = goalcup_cli::commands::start::start_with(
        &m,
        &support::connectors_dir(),
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &options,
        &mut Vec::new(),
    )
    .expect_err("the arming gate still refuses after cleanup succeeds");

    assert_eq!(error.exit_code(), 4);
    assert_eq!(
        m.calls().iter().filter(|call| *call == "abandon").count(),
        2,
        "the same cleanup is retried once"
    );
    assert_eq!(m.abandoned().len(), 1, "the daemon removed the entry once");
    assert!(
        goalcup_cli::entries::read_entry(tmp.path(), &m.info().entry_id).is_err(),
        "the acknowledged abandon also removes the local cache"
    );
}
