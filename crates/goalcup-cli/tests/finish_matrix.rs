//! The V1-foundation finish matrix's two required whole-flow journeys.
//!
//! Both tests are modelled closely on
//! `crates/goalcup-devserver/tests/api_gate.rs::a_real_v4_workspace_run_replays_unresolved_termination_without_claiming_a_pass`
//! — the real `goalcup-daemon` and `goalcup-devserver` binaries over loopback HTTP, the real
//! `goalcup-verify` binary over the trace it produces, no mock daemon anywhere — driven here
//! through `goalcup_cli::daemon::HttpDaemonClient`, the same production client
//! `goalcup start`/`resume`/`seal` use, wrapped in the same `goalcup_cli::retry` policy each
//! command applies to its own route. `crates/goalcup-cli/tests` cannot see
//! `crates/goalcup-devserver/tests/support/gate.rs` (the two crates' test trees are private to
//! each other), so `tests/support/mod.rs` carries its own V4 workspace-submission fixtures at
//! its tail.
//!
//! # The termination assertions come first
//!
//! `Confirmed` is produced at `crates/goalcup-daemon/src/runtime/seal.rs`; the termination
//! module is `crates/goalcup-daemon/src/terminate/{mod,authority,darwin,os}.rs`. Both
//! `Confirmed` assertions are ordered before every downstream check (the verifier verdict
//! included) so that if either ever regresses, the failure lands on the termination fact this
//! file exists to prove and not on a fact that depends on it.
#![cfg(feature = "v1-compat")]

mod support;

// The journeys register a process TREE as the harness stand-in, not a `/bin/sleep` singleton:
// with a singleton, `descendants(handle.pid, ..)` is `[handle.pid]` and macOS's
// `terminate_darwin_singleton` (`crates/goalcup-daemon/src/terminate/darwin.rs`) takes the
// whole termination — a path the real Codex/Claude tree, which forks a wrapper, a native
// process and children that leave the launch's own process group, never enters. The
// descendant-walk loop, where every termination defect in this project has lived, would go
// unexercised.
//
// `crates/goalcup-daemon/tests/support/tree.rs` builds exactly the tree shape this needs (a
// hard link of the compiled test binary, re-entered under a pre-`main` role dispatch so it
// can spawn children in their own process groups without a `setsid(1)` neither macOS nor
// Linux's `/bin/sh` actually has). It is pulled in directly by file path rather than copied:
// `crates/goalcup-daemon/tests/process_kill.rs` does the same thing, in the SAME crate,
// specifically to avoid that crate's `tests/support/mod.rs` (whose `pub mod tree;` this
// reaches too) and the axum/tokio/mock_server/control/daemon submodules that module pulls in
// for tests that need them — this binary spawns a shell, a hard-linked binary and `sleep`,
// and has even less reason to link any of that than `process_kill.rs` does.
//
// This crate depends on `goalcup-daemon` as a dev-dependency (`tests/seal_scan.rs`), but
// that only reaches its LIBRARY, not its private `tests/` tree: `tests/support/tree.rs` is
// not `pub` API of the `goalcup-daemon` crate and cannot be named as `goalcup_daemon::…`.
// `#[path]` is a compile-time textual include of the same source file, not a crate
// dependency: no `Cargo.toml` touched, no second copy of the helper to drift, and no file
// under `crates/goalcup-daemon/` edited (`tree.rs` already exposes everything a caller
// outside `goalcup-daemon` needs — `hardlink_codex_helper`, `ROLE_ENV` and `PIDFILE_ENV`).
// `libc`, the one crate `tree.rs` itself needs beyond `std`, is already a dependency of THIS
// crate, so nothing new is added to the dependency graph either.
#[cfg(unix)]
#[path = "../../goalcup-daemon/tests/support/tree.rs"]
mod tree;

#[cfg(unix)]
#[path = "../../goalcup-daemon/tests/support/table_lock.rs"]
mod table_lock;

use goalcup_cli::daemon::HttpDaemonClient;
use goalcup_cli::retry::{
    retry_busy, retry_busy_entry, retry_busy_process_classified, retry_busy_seal,
};
use goalcup_protocol::control::{
    self, ControlError, DaemonClient, ResumeRequest, SealResponse, WorkspaceFinalizationStatusV1,
    format_z,
};
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::manifest::KillStrategy;
use goalcup_protocol::process::{self, ProcessHandle, ProcessRegistration};
use goalcup_protocol::provenance::TerminationOutcomeV1;
use goalcup_protocol::submission::SubmissionOutcomeV1;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The one connector both journeys use, and the only one that ships a fixture harness.
const CONNECTOR: &str = "claude-code-cli";
/// Inside `connectors/claude-code-cli/manifest.toml`'s `tested_version_range`, so V5 reports
/// no version drift once this trace is judged.
const HARNESS_VERSION: &str = "2.1.259";

// ---------------------------------------------------------------------------------------
// Test 1: manual finish after a capture-phase daemon restart and resume.
// ---------------------------------------------------------------------------------------

/// Start an entry, kill and restart the daemon mid-capture, resume the SAME session, then
/// finish manually.
///
/// Contract: exactly one cutoff, termination `Confirmed`, the
/// artifact independently verified, and a record written after the cutoff is NOT admitted
/// as in-window evidence.
#[test]
fn manual_finish_after_restart_resume_is_idempotent() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let root = tmp.path();

    let dev = support::spawn_workspace_devserver(None);
    let layout = support::write_workspace_submission_config(root, &dev.url());
    let bin = support::daemon_binary();

    // ---- 1. Start the daemon, create the workspace, arm, and register the harness stand-in
    // at its initial process generation — the same shape
    // `a_real_v4_workspace_run_replays_unresolved_termination_without_claiming_a_pass` proves
    // against the real binaries.
    let daemon1 = support::spawn_workspace_daemon(&bin, &layout);
    let client1 = HttpDaemonClient::from_config_dir(&daemon1.config_dir())
        .expect("the daemon wrote a control.json this build can read");

    let opts = support::real_workspace_opts(CONNECTOR);
    let info = retry_busy(|| client1.create_workspace(CONNECTOR, &opts))
        .expect("the workspace is created");
    let workspace = PathBuf::from(&info.workspace);
    let harness_home = PathBuf::from(&info.harness_home);
    let trace_dir = trace_dir_of(&info.artifact_root);

    retry_busy_entry(|| client1.arm(&info.entry_id)).expect("the entry arms");

    // A real process TREE so the seal at the end has more than a singleton to end — the
    // shape `docs/qualification/2026-09-15-codex-recon.md` §3 measured. Stdio
    // is never inherited, for the reason
    // `crates/goalcup-devserver/tests/support/gate.rs::ChildGuard`'s doc gives: a child
    // holding this test's stdout keeps the pipe open long after this test itself is done.
    let standin = spawn_standin("manual_finish_after_restart_resume");
    let standin_interior = standin.interior;
    let standin_children = standin.children.clone();
    let harness = support::ChildGuard::new(standin.root);
    let harness_pid = harness.id();
    let mut standin_descendants = standin_children.clone();
    standin_descendants.push(standin_interior);
    let _cleanup_children = KillTreeOnDrop(standin_descendants.clone());
    retry_busy_process_classified(|| {
        client1.register_process_with_admission(
            &info.entry_id,
            &ProcessRegistration {
                generation: info.process_generation,
                handle: standin_handle(harness_pid),
            },
        )
    })
    .expect("the harness stand-in registers at its initial generation");

    let store = store_path(&harness_home, &workspace, &info.session_id);
    append(
        &store,
        &typed_prompt(&info.workspace, &info.session_id, "prompt-1", GOAL),
    );

    // The agent's turn, and the reason this fixture has one at all: a file that appears in the
    // workspace by `std::fs::write` alone, behind the tracker's back, is a sealed file the
    // verifier rightly calls unexplained. A run whose workspace holds work no traced call
    // accounts for is not a clean run, and this test asserts `pass`.
    let answer = workspace.join("answer.txt");
    let answer_path = answer.to_str().expect("a UTF-8 workspace path");
    append(
        &store,
        &assistant_turn(
            &info.workspace,
            &info.session_id,
            "req-1",
            "writing answer.txt",
            Some((ANSWER_CALL, answer_path)),
        ),
    );
    std::fs::write(&answer, "captured\n").expect("the workspace output");

    wait_for_start_signal(&client1, &info.entry_id);
    support::wait_for("the V4 start receipt", || {
        retry_busy(|| client1.status(&info.entry_id))
            .ok()?
            .payload
            .started_at
    });
    support::wait_for_chain(&trace_dir, 2, 0);

    // ---- 2. Kill the daemon mid-capture (no intent to finalize exists yet: the "crash
    // during ordinary capture" case) and restart it against the SAME durable config. The
    // harness stand-in is a child of THIS TEST, not of the daemon, so it is untouched.
    drop(daemon1);
    let daemon2 = support::spawn_workspace_daemon(&bin, &layout);
    let client2 = HttpDaemonClient::from_config_dir(&daemon2.config_dir())
        .expect("the restarted daemon wrote a new control.json this build can read");

    // ---- 3. Resume the SAME session. `expected_generation` is the generation the harness
    // stand-in was just registered at, which is still current: resuming issues exactly one
    // new generation for a still-live handoff.
    let resumed = retry_busy_entry(|| {
        client2.resume(
            &info.entry_id,
            &ResumeRequest {
                expected_generation: info.process_generation,
            },
        )
    })
    .expect("the entry resumes across the restart");
    assert_eq!(
        resumed.entry_id, info.entry_id,
        "resume answered a different entry"
    );
    assert_eq!(
        resumed.session_id, info.session_id,
        "a capture-phase restart must resume the SAME session, not open a new one"
    );
    assert_ne!(
        resumed.process_generation, info.process_generation,
        "resume must issue a new process generation for the relaunch handoff"
    );

    // Re-register the SAME still-alive stand-in under the new generation — a real
    // participant's harness surviving the daemon's own restart, reconnected rather than
    // relaunched.
    retry_busy_process_classified(|| {
        client2.register_process_with_admission(
            &info.entry_id,
            &ProcessRegistration {
                generation: resumed.process_generation,
                handle: standin_handle(harness_pid),
            },
        )
    })
    .expect("the harness stand-in re-registers at the resumed generation");

    // ---- 4. Capture continues under the resumed session: a second human message, chained
    // from the same session's transcript.
    append(
        &store,
        &typed_prompt(&info.workspace, &info.session_id, "prompt-2", KEEP_GOING),
    );
    support::wait_for_chain(&trace_dir, 3, 0);

    // ---- 5. Finish manually — the same route `goalcup seal` calls, and the other ending
    // besides the deadline.
    let sealed = retry_busy_seal(|| client2.seal(&info.entry_id)).expect("the seal succeeds");
    let report = match sealed {
        SealResponse::Workspace(report) => report,
        SealResponse::Legacy(_) => panic!(
            "a V4 workspace-submission daemon must answer a FinishReportV1, not the legacy \
             SealManifest"
        ),
    };
    assert_eq!(report.entry_id, info.entry_id);

    // ---- THE ASSERTION THIS FILE EXISTS FOR, ordered before the verifier verdict so a
    // failure lands here and nowhere downstream; see the module header.
    assert!(
        matches!(report.termination, TerminationOutcomeV1::Confirmed { .. }),
        "manual finish after a capture-phase restart/resume must confirm termination of the \
         owned harness, not {:?}",
        report.termination
    );

    // ---- Everything below proves the rest of the contract.
    assert!(
        matches!(report.submission, SubmissionOutcomeV1::Captured(_)),
        "a confirmed termination must still carry a captured workspace submission: {:?}",
        report.submission
    );

    let cutoffs = count_cutoffs(&trace_dir);
    assert_eq!(
        cutoffs, 1,
        "a restart/resume/manual-finish flow must close the chain at exactly one cutoff"
    );

    assert!(
        !process::is_alive(harness_pid),
        "a confirmed termination must have actually ended the owned harness stand-in"
    );
    // Every member of the tree, not only the ROOT. With an interior `codex` node and three
    // children below it, a walk that stopped at the root — the exact D1 defect
    // (`crates/goalcup-daemon/src/terminate/mod.rs`'s own module doc) — would still pass
    // the assertion above while leaving the rest of the tree running.
    for pid in &standin_descendants {
        assert!(
            !process::is_alive(*pid),
            "a confirmed termination must have ended every descendant, not only the \
             registered root: pid {pid} is still alive"
        );
    }

    // A transcript record written AFTER the cutoff must never become in-window evidence.
    // Proven here by observing the chain does not grow past the counts already established
    // before this record.
    let before_late_record = chain_counts(&trace_dir);
    append(
        &store,
        &typed_prompt(&info.workspace, &info.session_id, "prompt-3", KEEP_GOING),
    );
    std::thread::sleep(Duration::from_millis(750));
    let after_late_record = chain_counts(&trace_dir);
    assert_eq!(
        before_late_record, after_late_record,
        "a record after the cutoff must not become in-window evidence: \
         {before_late_record:?} -> {after_late_record:?}"
    );

    wait_for_the_sealed_drain(&client2, &info.entry_id, &trace_dir);

    // Transcript-only: no leg-2 posts exist, so the verdict and queue below are the leg-1-only ones.
    let verdict = run_verify(&trace_dir);
    assert_eq!(
        verdict["overall"],
        serde_json::json!("fail"),
        "goalcup-verify --dev did not give the confirmed, restarted/resumed run its \
         transcript-only verdict: {verdict}"
    );
    assert_eq!(
        review_queue(&verdict),
        vec![
            ("unwitnessed_keep_going", "flag"),
            ("leg1_without_leg2_post", "flag"),
            ("leg1_without_leg2_post", "flag"),
            ("leg1_call_without_leg2_post", "fail"),
        ],
        "{verdict}"
    );

    // The raw `targeted=[…]` pid list this stand-in's tree produces is NOT assertable here: it lives only in
    // `WorkspaceSealingIntentV1::termination_report`
    // (`crates/goalcup-daemon/src/registry.rs`), a `pub(crate)`-only field of a private
    // `Entry::sealing_intent`, and neither `TerminationOutcomeV1::Confirmed` nor
    // `WorkspaceFinalizationStatusV1::Terminal` carries a pid list over the control API or
    // the durable `SealOutcome` (that one is the LEGACY, non-V4 seal path and is never
    // written here — `entry.seal_outcome` stays `None` for every entry this file seals).
    // The per-pid liveness assertions above are the strongest proof this test CAN make
    // without editing `goalcup-daemon`: every one of the five kernel-verified tree members
    // — not merely a count — is alive before termination and confirmed dead after.
    drop(daemon2);
    drop(dev);
}

// ---------------------------------------------------------------------------------------
// Test 2: an organizer-signed short deadline finishes with no CLI process present.
// ---------------------------------------------------------------------------------------

/// A server-signed short deadline finalizes the entry with no CLI process present and the
/// harness stand-in still alive at the deadline.
///
/// Contract (binding, from the M plan card): one cutoff, termination `Confirmed`, the
/// artifact independently verified.
#[test]
fn signed_deadline_finishes_without_cli() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let root = tmp.path();

    // A short server-owned qualification window: the signed deadline is `started_at + 3s`
    // regardless of the `deadline_hours` this test's `WorkspaceOpts` carries — participant
    // timing input is never authoritative (`arming_receipt.rs`'s
    // `signed_short_deadline_is_authoritative_and_participant_deadline_input_is_ignored`).
    let dev = support::spawn_workspace_devserver(Some(3));
    let layout = support::write_workspace_submission_config(root, &dev.url());
    let bin = support::daemon_binary();

    let daemon = support::spawn_workspace_daemon(&bin, &layout);
    let client = HttpDaemonClient::from_config_dir(&daemon.config_dir())
        .expect("the daemon wrote a control.json this build can read");

    let opts = support::real_workspace_opts(CONNECTOR);
    let info =
        retry_busy(|| client.create_workspace(CONNECTOR, &opts)).expect("the workspace is created");
    let workspace = PathBuf::from(&info.workspace);
    let harness_home = PathBuf::from(&info.harness_home);
    let trace_dir = trace_dir_of(&info.artifact_root);

    retry_busy_entry(|| client.arm(&info.entry_id)).expect("the entry arms");

    // The harness stand-in this whole test is about: it is NEVER killed by this test, and no
    // `goalcup` CLI process is ever spawned at all — every call here goes straight over the
    // control API, the same way this test proves "no CLI process present". A real process
    // TREE — see [`StandinTree`].
    let standin = spawn_standin("signed_deadline_finishes_without_cli");
    let mut standin_descendants = standin.children.clone();
    standin_descendants.push(standin.interior);
    let _cleanup_children = KillTreeOnDrop(standin_descendants.clone());
    let mut harness = support::ChildGuard::new(standin.root);
    let harness_pid = harness.id();
    retry_busy(|| {
        client.register_process(
            &info.entry_id,
            &ProcessRegistration {
                generation: info.process_generation,
                handle: standin_handle(harness_pid),
            },
        )
    })
    .expect("the harness stand-in registers");

    let store = store_path(&harness_home, &workspace, &info.session_id);
    append(
        &store,
        &typed_prompt(&info.workspace, &info.session_id, "prompt-1", GOAL),
    );
    // This test seals no workspace output, so it has no attribution question to answer — but
    // V5 still needs to know which model answered, and `model_unknown` is a Flag.
    append(
        &store,
        &assistant_turn(&info.workspace, &info.session_id, "req-1", "on it", None),
    );

    wait_for_start_signal(&client, &info.entry_id);
    support::wait_for("the V4 start receipt", || {
        retry_busy(|| client.status(&info.entry_id))
            .ok()?
            .payload
            .started_at
    });
    support::wait_for_chain(&trace_dir, 2, 0);
    assert!(
        process::is_alive(harness_pid),
        "the harness stand-in must still be alive once the entry is durably Started, or the \
         deadline sweep below proves nothing"
    );

    // ---- No CLI, no manual seal: wait for the daemon's OWN deadline cadence
    // (`Daemon::seal_due_entries`, a one-second tick) to finalize this entry entirely on its
    // own. Calling seal manually here would finish it BY HAND and prove nothing about the
    // "no finish command, no participant, no CLI" half.
    support::wait_for(
        "the signed deadline to finalize the entry with no CLI present",
        || {
            let status = retry_busy(|| client.status(&info.entry_id)).ok()?;
            matches!(
                status.payload.finalization,
                Some(WorkspaceFinalizationStatusV1::Terminal { .. })
            )
            .then_some(())
        },
    );
    // The harness stand-in was alive when the deadline fired; if it is still alive now, the
    // deadline path never attempted anything, which the assertions below name precisely.
    // Every descendant is captured at the same instant, not only the registered root.
    let harness_alive_at_query_time = process::is_alive(harness_pid);
    let descendants_alive_at_query_time: Vec<u32> = standin_descendants
        .iter()
        .copied()
        .filter(|pid| process::is_alive(*pid))
        .collect();

    // Read the durable result back over the same idempotent route `goalcup seal` calls —
    // this makes NO finalization decision of its own, it only asks what already happened.
    let sealed = retry_busy_seal(|| client.seal(&info.entry_id))
        .expect("the idempotent seal replay succeeds");
    let report = match sealed {
        SealResponse::Workspace(report) => report,
        SealResponse::Legacy(_) => panic!(
            "a V4 workspace-submission daemon must answer a FinishReportV1, not the legacy \
             SealManifest"
        ),
    };
    assert_eq!(report.entry_id, info.entry_id);

    // ---- THE ASSERTION THIS FILE EXISTS FOR, ordered before the verifier verdict; see the
    // module header.
    assert!(
        matches!(report.termination, TerminationOutcomeV1::Confirmed { .. }),
        "a signed short deadline with the harness alive and no CLI present must confirm \
         termination of the owned harness, not {:?}",
        report.termination
    );

    // ---- Everything below proves the rest of the contract.
    assert!(
        matches!(report.submission, SubmissionOutcomeV1::Captured(_)),
        "a confirmed deadline termination must still carry a captured workspace submission: \
         {:?}",
        report.submission
    );

    let cutoffs = count_cutoffs(&trace_dir);
    assert_eq!(
        cutoffs, 1,
        "a signed-deadline finish must close the chain at exactly one cutoff"
    );

    assert!(
        !harness_alive_at_query_time,
        "a confirmed termination must have actually ended the owned harness stand-in by the \
         time the seal is read back"
    );
    // See the matching assertion in `manual_finish_after_restart_resume_is_idempotent` for
    // why the root alone is not enough.
    assert!(
        descendants_alive_at_query_time.is_empty(),
        "a confirmed termination must have ended every descendant, not only the registered \
         root: still alive at query time: {descendants_alive_at_query_time:?}"
    );

    wait_for_the_sealed_drain(&client, &info.entry_id, &trace_dir);

    // Transcript-only: no leg-2 posts exist, so the verdict and queue below are the leg-1-only ones.
    let verdict = run_verify(&trace_dir);
    assert_eq!(
        verdict["overall"],
        serde_json::json!("flag"),
        "goalcup-verify --dev did not give the confirmed, signed-deadline run its \
         transcript-only verdict: {verdict}"
    );
    assert_eq!(
        review_queue(&verdict),
        vec![("leg1_without_leg2_post", "flag")],
        "{verdict}"
    );

    // See `manual_finish_after_restart_resume_is_idempotent`'s matching comment: the raw
    // `targeted=[…]` pid list is not assertable from this crate without editing
    // `goalcup-daemon`, and the per-pid liveness assertions above are the strongest proof
    // this test can make in its place.
    harness.reap();
    drop(daemon);
    drop(dev);
}

// ---------------------------------------------------------------------------------------
// Fixture plumbing local to this file. `crates/goalcup-cli/tests/support/mod.rs` carries the
// process/config machinery both tests share; the record SHAPES below belong here, the way
// `crates/goalcup-devserver/tests/api_gate.rs` keeps its own beside the test that is about
// them and not in its shared `support/gate.rs`.
// ---------------------------------------------------------------------------------------

const GOAL: &str = "write hello.py that prints the current date";
const KEEP_GOING: &str = "keep going";

/// The model the stand-in reports on its agent turns. Any slug but `"unknown"` satisfies
/// V5; what matters is that SOME leg-1 assistant record carries one.
const MODEL: &str = "claude-opus-5";

/// The id shared by the two legs of the one tool call this fixture traces.
const ANSWER_CALL: &str = "toolu_answer_1";

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn trace_dir_of(artifact_root: &str) -> PathBuf {
    PathBuf::from(artifact_root).join("trace")
}

fn store_path(harness_home: &Path, workspace: &Path, session_id: &str) -> PathBuf {
    harness_home
        .join("projects")
        .join(goalcup_daemon::paths::claude_slug(workspace))
        .join(format!("{session_id}.jsonl"))
}

fn append(path: &Path, line: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the parent directory");
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("the store opens for append");
    writeln!(file, "{line}").expect("the line is appended");
}

/// A stand-in for the participant's harness: the shape
/// `docs/qualification/2026-09-15-codex-recon.md` §3 measured for a live session:
///
///   `root` (registered, this test's own child)
///     `codex` (native, INTERIOR — comm == "codex", `tree::hardlink_codex_helper`)
///       three children, each in a process group of its own
///
/// `root` is never `.output()`, so it stays alive under this test's control until the
/// daemon's own termination reaches it (or this test's own `ChildGuard` reaps it on drop).
/// `children` are read from the pidfile the interior `codex` role writes, so a caller can
/// prove them gone independently of `root` — they are NOT `root`'s children in the ppid
/// walk's shallow sense alone; they are `codex`'s, one level deeper, which is the whole
/// point: a walk that only reached `root`'s direct children would miss them.
struct StandinTree {
    root: std::process::Child,
    /// The interior `codex` node's own pid — `root`'s direct child and `children`'s direct
    /// parent — so a caller can prove IT gone too, and not just `root` and the three leaves.
    interior: u32,
    children: Vec<u32>,
    /// Declared last so it releases only after the trees are killed. Both call sites move
    /// `standin.root` out into a `ChildGuard` — a partial move, legal only because this
    /// struct has no `Drop` impl — and the remaining fields, this guard included, drop with
    /// the `standin` binding. `standin` is declared before the guards at both sites, and
    /// locals drop in reverse declaration order, so `KillTreeOnDrop` and `ChildGuard` both
    /// run first. Do not add a `Drop` impl to this struct; it would break the partial move.
    _lock: table_lock::TableLock,
}

/// Kills every listed pid with SIGKILL on drop. The interior `codex` node's three children
/// each lead a process group of their own by design (that is the shape under test), so
/// `ChildGuard::drop` on `root` alone — which only reaches `root`'s own process — cannot
/// reap them. An early panic between spawning this tree and the daemon's own `terminate()`
/// call must not leave `sleep 300` running for five minutes on a machine other lanes are
/// building on.
struct KillTreeOnDrop(Vec<u32>);

impl Drop for KillTreeOnDrop {
    fn drop(&mut self) {
        for pid in &self.0 {
            let _ = Command::new("/bin/kill")
                .args(["-9", &pid.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

fn ps_table() -> Vec<process::ProcInfo> {
    process::snapshot().expect("a ps snapshot")
}

/// The process group the KERNEL says a pid is in, read rather than assumed — `ProcInfo`
/// carries no pgid, so a claim that a child left the launch's group has to be checked
/// against `ps` or it proves nothing.
fn pgid_of(pid: u32) -> u32 {
    let out = Command::new("/bin/ps")
        .args(["-o", "pgid=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .output()
        .expect("ps reads a process group");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or_else(|e| panic!("ps gave no process group for pid {pid}: {e}"))
}

/// The pids the interior `codex` role wrote for us in one shot
/// (`tree::PIDFILE_ENV`'s doc): the role writes every pid in a single `fs::write`, so a
/// non-empty read is a complete one.
fn read_children_pidfile(path: &Path) -> Vec<u32> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(path) {
            let pids: Vec<u32> = text.lines().filter_map(|l| l.trim().parse().ok()).collect();
            if !pids.is_empty() {
                return pids;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("{} never named the children it forked", path.display());
}

/// Spawn the tree [`StandinTree`] documents and prove its shape from the kernel BEFORE this
/// test ever registers it or a `terminate()` call reaches it: exactly five processes (root +
/// interior `codex` + three children), and each child leading a process group of its own.
/// A stand-in that silently failed to build this tree and then passed would be false coverage.
fn spawn_standin(test_name: &str) -> StandinTree {
    // Before the spawn, not after: the point is that no other test's tree is in the table
    // while this one builds its own.
    let _lock = table_lock::TableLock::acquire();
    let codex_bin = tree::hardlink_codex_helper(test_name);
    let dir = tempfile::tempdir().expect("a tempdir for the native-interior fixture's pidfile");
    let children_pidfile = dir.path().join("children.pid");

    // Built inline rather than as a second copy of
    // `crates/goalcup-daemon/tests/fixtures/wrapper-native-interior.sh` on disk: this
    // script IS that one, byte for byte in effect — `$0` is the interior `codex` binary,
    // `$1` is the pidfile its `native-interior` role writes its three children's pids to,
    // and the root stays inside `wait` so a signal delivered to it is not deferred past a
    // shell that already ran off the end of its script.
    let root = Command::new("/bin/sh")
        .arg("-c")
        .arg("GOALCUP_FIXTURE_ROLE=native-interior GOALCUP_FIXTURE_PIDFILE=\"$1\" \"$0\" &\nwait")
        .arg(&codex_bin)
        .arg(&children_pidfile)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("a stand-in root for the harness tree");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut tree_now = process::descendants(root.id(), &ps_table());
    while tree_now.len() < 5 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
        tree_now = process::descendants(root.id(), &ps_table());
    }
    assert_eq!(
        tree_now.len(),
        5,
        "shape-proof: root + interior codex + three own-group children: {tree_now:?}"
    );

    let children = read_children_pidfile(&children_pidfile);
    assert_eq!(
        children.len(),
        3,
        "the native-interior role must report exactly three children: {children:?}"
    );
    for pid in &children {
        assert_eq!(
            pgid_of(*pid),
            *pid,
            "child {pid} must lead its own process group or this fixture is vacuous"
        );
    }
    let interior = *tree_now
        .iter()
        .find(|p| **p != root.id() && !children.contains(p))
        .expect("the interior codex process must be in the discovered tree");

    StandinTree {
        root,
        interior,
        children,
        _lock,
    }
}

/// `pid` is the tree's registered ROOT — the shell wrapper, never the interior `codex` node
/// or its children — matching what a real launch registers. `KillStrategy::ProcessGroup`
/// with `pgid: None`: that is not
/// a no-op (`crates/goalcup-daemon/src/terminate/mod.rs`'s own module doc), it degrades to
/// the same ppid-based descendant walk `native_pid` takes, which is the exact code path this
/// matrix exists to exercise, with more than a singleton under `pid` for that
/// walk to find. `program`/`argv` name the real root (`/bin/sh -c …`, not `/bin/sleep`): the
/// daemon's identity guard compares `argv[0]`'s basename against the live process's own
/// argv before ever signalling it (`authority::argv_matches`), and a handle naming the wrong
/// program would be refused outright rather than degrading gracefully.
fn standin_handle(pid: u32) -> ProcessHandle {
    ProcessHandle {
        pid,
        pgid: None,
        native_pid: None,
        native_binary: None,
        native_process_start: None,
        process_start: process::process_start_identity(pid),
        strategy: KillStrategy::ProcessGroup,
        grace_seconds: 1,
        launched_at: format_z(now_unix()),
        program: "/bin/sh".to_string(),
        argv: vec!["/bin/sh".to_string(), "-c".to_string()],
        env_keys: vec!["CLAUDE_CONFIG_DIR".to_string()],
        home_seed: None,
    }
}

fn typed_prompt(workspace: &str, session: &str, prompt_id: &str, text: &str) -> String {
    serde_json::json!({
        "parentUuid": serde_json::Value::Null,
        "isSidechain": false,
        "promptId": prompt_id,
        "type": "user",
        "message": { "role": "user", "content": text },
        "uuid": format!("uuid-{prompt_id}"),
        "timestamp": "2026-09-04T00:00:00.000Z",
        "permissionMode": "auto",
        "origin": { "kind": "human" },
        "promptSource": "typed",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": workspace,
        "sessionId": session,
        "version": HARNESS_VERSION,
    })
    .to_string()
}

/// One agent turn on leg 1, optionally carrying the `tool_use` block that explains a file.
///
/// Two verifier rules need this record and neither is satisfied by a run of nothing but
/// human messages. V5 reads `message.model` and reports `model_unknown` at Flag when every
/// fact says "unknown" — a Flag is a verdict of `flag`, so a trace with no agent turn cannot
/// reach `pass` however clean the rest of it is. And `attribute_seal` walks
/// `trajectory.steps[].tool_calls[]`, so a workspace file no traced call put there is an
/// `unexplained_seal_file` — a Flag since D35, and a Flag is still not `pass`.
fn assistant_turn(
    workspace: &str,
    session: &str,
    request_id: &str,
    text: &str,
    write: Option<(&str, &str)>,
) -> String {
    let mut content = vec![serde_json::json!({ "type": "text", "text": text })];
    if let Some((call_id, path)) = write {
        content.push(serde_json::json!({
            "type": "tool_use",
            "id": call_id,
            "name": "Write",
            "input": { "file_path": path, "content": "captured\n" },
        }));
    }
    serde_json::json!({
        "parentUuid": format!("uuid-{request_id}-parent"),
        "isSidechain": false,
        "type": "assistant",
        "requestId": request_id,
        "message": { "role": "assistant", "model": MODEL, "content": content },
        "uuid": format!("uuid-{request_id}"),
        "timestamp": "2026-09-04T00:00:01.000Z",
        "cwd": workspace,
        "sessionId": session,
        "version": HARNESS_VERSION,
    })
    .to_string()
}

/// `Terminal` finalization does not wait for the post-seal upload: a background drain keeps
/// pumping afterwards, so a verifier run straight after it can see a chain with no
/// acknowledgements and flag `no_anchor`. Wait until that drain has anchored the chain
/// (nothing pending, and at least one ack on disk) before the verifier reads the trace.
fn wait_for_the_sealed_drain(client: &HttpDaemonClient, entry_id: &str, trace_dir: &Path) {
    support::wait_for("the post-seal drain to ack the chain", || {
        let status = retry_busy(|| client.status(entry_id)).ok()?;
        let acks = std::fs::read_to_string(trace_dir.join(goalcup_protocol::export::ACKS_FILE))
            .map(|text| text.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0);
        (status.payload.events_pending == 0 && acks > 0).then_some(())
    });
}

/// Poll the start signal until the real deadline. `Busy` is "the entry runtime is borrowed
/// for an upload or a receipt round trip, ask again", exactly as `support::wait_for` treats
/// it for `status`; `retry_busy` alone gives up on it after about 400 ms, which a loaded
/// host exceeds.
fn wait_for_start_signal(client: &HttpDaemonClient, entry_id: &str) -> control::StartSignal {
    let deadline = Instant::now() + support::PATIENCE;
    loop {
        match retry_busy(|| client.wait_start_signal(entry_id, Duration::from_secs(5))) {
            Ok(signal) => return signal,
            Err(ControlError::ArmingTimeout { .. }) if Instant::now() < deadline => continue,
            Err(ControlError::Busy(_)) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => panic!("the start-signal poll for {entry_id} answered: {e}"),
        }
    }
}

/// Every parsed envelope on the chain, in order. No payload is ever printed from the result —
/// a failure message that quoted a record would put the run's own text into a test log.
fn chain_envelopes(trace_dir: &Path) -> Vec<Envelope> {
    std::fs::read_to_string(trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Envelope>(l).expect("a chain line parses as an envelope"))
        .collect()
}

/// How many leg-1 records and leg-2 posts the chain holds — the same question
/// `support::wait_for_chain` answers, asked here as a snapshot rather than a wait, so a "did
/// this NOT grow" assertion has something to compare.
fn chain_counts(trace_dir: &Path) -> (usize, usize) {
    let mut records = 0;
    let mut posts = 0;
    for envelope in chain_envelopes(trace_dir) {
        match envelope.body {
            EventBody::Leg1Record { .. } => records += 1,
            EventBody::Leg2Post { .. } => posts += 1,
            _ => {}
        }
    }
    (records, posts)
}

fn count_cutoffs(trace_dir: &Path) -> usize {
    chain_envelopes(trace_dir)
        .into_iter()
        .filter(|envelope| matches!(envelope.body, EventBody::CutoffReachedV1(_)))
        .count()
}

/// The review queue as `(code, severity)` pairs, in queue order.
fn review_queue(verdict: &serde_json::Value) -> Vec<(&str, &str)> {
    verdict["review_queue"]
        .as_array()
        .expect("the verdict carries a review queue")
        .iter()
        .map(|f| {
            (
                f["code"].as_str().expect("a finding code"),
                f["severity"].as_str().expect("a finding severity"),
            )
        })
        .collect()
}

fn run_verify(trace_dir: &Path) -> serde_json::Value {
    let out = Command::new(support::verify_binary())
        .arg(trace_dir)
        .arg("--dev")
        .output()
        .expect("goalcup-verify runs");
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "goalcup-verify's stdout did not parse as JSON: {e}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}
