//! The whole gate, driven through the CONTROL API, against the real binaries.
//!
//! `roundtrip_verify.rs` beside this file proves that a trace the daemon writes verifies. It
//! drives the daemon's **library**: `workspace::prepare`, a hand-assembled `Entry`,
//! `daemon.register`, `.arm`, `.bind_start`, `poll_once_at`, `.seal_entry`.
//!
//! The gap that leaves is narrow and expensive. `POST /v0/workspaces` builds its entry
//! through `control::build_entry` and `control::prepared_workspace`, which the library
//! fixture bypasses entirely: one field set differently there — a `manifest_sha256`, an
//! `expected_config` — and every trace a real participant produces
//! fails V9 or V13 while the whole workspace suite stays green.
//!
//! So this drives the SHIPPED BINARIES over loopback and hands the result to the shipped
//! verifier:
//!
//!   `goalcup-devserver` → `goalcup-daemon --config` → `control.json` → POST /v0/workspaces
//!   → POST arm → POST process → the harness writes its transcript → GET start-signal → the
//!   daemon binds the receipt on its own cadence → POST seal → `goalcup-verify --dev`.
//!
//! Nothing here reaches into the library to move the daemon along. The polls, the uploads,
//! the acknowledgements and the receipt are the daemon's own four cadences running in
//! another process, which is the difference between this and the fixture next door.
//!
//! # Why the server's clock is set below the run
//!
//! A start receipt's `started_at` is the server's clock, and a client may not name its
//! own start. But the records that CAUSE a receipt to be requested — the harness launching,
//! the first transcript record — are chained before the request goes out, so their
//! `recorded_at` is at or barely below `started_at`, and V13 reports one second of rounding
//! as `outside_window`. `roundtrip_verify.rs` opens its window ten minutes below the run for
//! this reason; the same margin is set here through the binary's `--clock`. It is worth
//! saying plainly that this is the FIXTURE's compensation for a real property of the
//! protocol, not a property of the fixture.
//!
//! # Why the binaries are found as siblings
//!
//! `CARGO_BIN_EXE_goalcup-devserver` is the only one cargo sets for this package. The other
//! two are built by `cargo test --workspace --offline` — the gate command — and land beside
//! it, so they are resolved by name in the same directory. A missing one is a loud failure
//! naming the command that builds it rather than a silent skip.
#![cfg(feature = "v1-compat")]

// The shared plumbing, by path rather than through `tests/support/mod.rs`: that module
// pulls the verifier crate's fixture generator in with it, and this test wants the two
// child processes and the two clients and nothing else.
#[path = "support/gate.rs"]
mod gate;

use gate::{
    Api, CLOCK_MARGIN, DaemonProcess, Devserver, ROUND_CLOSES, ROUND_RULES, ROUND_T0, append,
    chain_head, now_unix, opts, parse_secs, sibling, status, wait_for, wait_for_chain,
    write_config, write_workspace_submission_config,
};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::control::{
    FinishReportV1, ROUTE_ENTRIES, ROUTE_WORKSPACES, StartSignal, WorkspaceFinalizationStatusV1,
    WorkspaceInfo, format_z,
};
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::manifest::KillStrategy;
use goalcup_protocol::process::{ProcessHandle, ProcessRegistration};
use goalcup_protocol::provenance::{TerminationFailureV1, TerminationOutcomeV1};
use goalcup_protocol::receipt::StartReceipt;
use goalcup_protocol::submission::SubmissionManifestV1;
use goalcup_protocol::submission::SubmissionOutcomeV1;
use serde_json::json;
use std::path::PathBuf;
use std::process::{Command, Stdio};
/// The goal, under `MAX_PROMPT_CHARS`.
const GOAL: &str = "write hello.py that prints the current date";
/// Every human message after the goal must be exactly this (V2, RULES.md).
const KEEP_GOING: &str = "keep going";
const WRITTEN_FILE: &str = "hello.py";
const WRITTEN_BODY: &str = "from datetime import date\nprint(date.today())\n";
/// Inside `tested_version_range = ">=2.1.259"`, so V5 reports no version drift.
const HARNESS_VERSION: &str = "2.1.259";

/// The API gates own a real stand-in until the daemon seals and reaps it.  This makes the
/// failure path executable: a panic before the seal must not leave its `/bin/sleep 600`
/// behind for a later PID-sensitive test.
#[test]
fn harness_child_guard_reaps_the_standin_during_unwind() {
    let child = Command::new("/bin/sleep")
        .arg("600")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the guard fixture starts");
    let pid = child.id();

    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = gate::ChildGuard::new(child);
        panic!("exercise the guard's unwind path");
    }));

    assert!(panicked.is_err(), "the fixture reached its forced unwind");
    wait_for("the unwind guard to reap the fixture", || {
        (!goalcup_protocol::process::is_alive(pid)).then_some(())
    });
}

#[test]
fn a_run_driven_entirely_through_the_control_api_passes_the_verifier() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    // Canonical from the start: on macOS a tempdir is handed out as `/var/folders/…` while a
    // harness records `/private/var/folders/…` in its own `cwd`, and the partitioner and V10
    // both compare the two byte for byte.
    let root = std::fs::canonicalize(tmp.path()).expect("the tempdir canonicalises");

    // ---- 1. the server the daemon uploads to, and the window it opens.
    let opened_at = now_unix() - CLOCK_MARGIN;
    let mut devserver = Devserver::start(&format_z(opened_at));

    // ---- 2. the daemon, configured and started as an operator would start it.
    let config = write_config(&root, &devserver.url);
    let mut daemon = DaemonProcess::start(&config);
    let token = daemon.bearer();
    let api = Api {
        addr: daemon.control_addr.clone(),
        token,
    };

    // ---- 3. POST /v0/workspaces. Everything the participant's CLI is handed comes back
    // from this one call: where to work and which session to write. `build_entry` is what fills them, and this is the field-for-field proof that
    // what it fills produces a trace that verifies.
    let info: WorkspaceInfo = api.post_json(ROUTE_WORKSPACES, &opts("claude-code-cli"));
    let workspace = PathBuf::from(&info.workspace);
    let harness_home = PathBuf::from(&info.harness_home);
    let trace_dir = PathBuf::from(&info.artifact_root).join("trace");
    let entry = format!("{ROUTE_ENTRIES}/{}", info.entry_id);

    // ---- 4. arm, which runs the pre-arm foreign scan first over an empty harness home.
    assert_eq!(api.post(&format!("{entry}/arm"), "").status, 204);

    // ---- 5. the process the CLI launched. A real child, so the seal at the end has a live
    // process tree to end rather than an empty target list.
    // Stdio is never inherited: a child holding this test's stdout keeps the pipe open long
    // after the test itself is finished, and a runner reading it waits for an EOF the child
    // will not send.
    let mut harness = gate::ChildGuard::new(
        Command::new("/bin/sleep")
            .arg("600")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("a stand-in for the harness"),
    );
    let handle = ProcessHandle {
        pid: harness.id(),
        pgid: None,
        native_pid: None,
        native_binary: None,
        native_process_start: None,
        process_start: goalcup_protocol::process::process_start_identity(harness.id()),
        strategy: KillStrategy::ProcessGroup,
        grace_seconds: 1,
        launched_at: format_z(now_unix()),
        program: "/bin/sleep".to_string(),
        argv: vec!["/bin/sleep".to_string(), "600".to_string()],
        env_keys: vec!["CLAUDE_CONFIG_DIR".to_string()],
        home_seed: None,
    };
    let registration = ProcessRegistration {
        generation: info.process_generation,
        handle,
    };
    let process = format!("{entry}/process");
    assert_eq!(api.post_body(&process, &registration).status, 204);
    assert_eq!(
        api.post_body(&process, &registration).status,
        204,
        "the exact typed registration replays under its canonical digest"
    );

    // ---- 6. what the harness writes: its transcript, in the order a real session produces
    // it, under the session id the API just handed over. Leg 2 is empty.
    let store = harness_home
        .join("projects")
        .join(goalcup_daemon::paths::claude_slug(&workspace))
        .join(format!("{}.jsonl", info.session_id));
    let ws = info.workspace.clone();
    let session = info.session_id.clone();

    append(&store, &typed_prompt(&ws, &session, "prompt-1", GOAL));

    append(&store, &assistant_write(&ws, &session, "toolu_01"));
    // The agent's own file. Written by the "harness", because it is a harness that writes
    // into a workspace and the seal's job is to notice.
    std::fs::write(workspace.join(WRITTEN_FILE), WRITTEN_BODY).expect("the agent's file");
    append(&store, &tool_result(&ws, &session, "toolu_01"));

    append(&store, &typed_prompt(&ws, &session, "prompt-2", KEEP_GOING));

    // ---- 7. the start signal, over the long poll that the on-screen text and the CLI both read.
    // Retried on the body code `busy`, which is not a refusal: an entry is out of the map
    // for the length of one pump, and with an upload cadence of 200 ms a caller will meet
    // that. `conflict` shares the status and is NOT retried. Every client has to do the
    // same, which is worth a test saying out loud.
    let path = format!("{entry}/start-signal?timeout_ms=5000");
    let signal: StartSignal = wait_for("the gate to see the harness start", || {
        let r = api.get(&path);
        match r.status {
            200 => Some(serde_json::from_str(&r.body).expect("the contract's type")),
            // 408 is the long poll's own timeout; 409 here is `busy` and nothing else —
            // a `conflict` would be a refusal this poll must report rather than wait out.
            408 => None,
            409 if r.is_busy() => None,
            other => panic!("the start-signal poll answered {other}: {}", r.body),
        }
    });
    assert_eq!(signal.entry_id, info.entry_id);
    assert_eq!(signal.session_id.as_deref(), Some(session.as_str()));

    // ---- 8. the receipt, which the daemon's OWN tick requests once the gate may start.
    // Nothing here asks for it: that is the point of driving the binary.
    let started = wait_for("the daemon to bind the start receipt", || {
        status(&api, &info).payload.started_at
    });
    assert!(
        parse_secs(&started) >= opened_at,
        "the receipt's window opens below the run: {started}"
    );

    // The `[event]` section `write_config` wrote, on the SIGNED document, off the public
    // route. `event_status_for` is reached from `control::build_entry` in the daemon's own
    // process, so a configured round that never made it onto the wire — the reading an
    // agent asking the MCP server for the time would get — fails here rather than at the
    // event. `publication_closes_at` is deliberately not asserted: it is the
    // deadline the receipt fixed, and that is the entry's, not the file's.
    let signed = status(&api, &info);
    assert_eq!(signed.payload.event.t0, ROUND_T0);
    assert_eq!(signed.payload.event.start_window_closes_at, ROUND_CLOSES);
    assert_eq!(signed.payload.event.rules_version, ROUND_RULES);
    assert_eq!(
        signed.payload.event_status_source,
        goalcup_protocol::control::EventStatusSource::Config,
        "a configured round must not be reported as the entry's own timeline"
    );

    // ---- 9. and then the daemon's own poll cadence catches up with the rest of what the
    // harness wrote. Waited for by CONTENT, not by a duration: sealing while records are
    // still on disk and not yet chained produces a trace with leg-1 records missing, which
    // reads as `unexplained_seal_file` — a failure of this test's pacing rather than of the
    // daemon.
    wait_for_chain(&trace_dir, 4, 0);

    // ---- 10. the seal, over the route `goalcup finish` calls. `POST` and not a deadline:
    // An entry has two endings and this is the participant's.
    let sealed = api.post(&format!("{entry}/seal"), "");
    assert_eq!(sealed.status, 200, "{}", sealed.body);
    // Reaped rather than waited on: the seal signals the tree and returns, so the exit is
    // observed on a deadline like every other cadence in this test.
    wait_for("the seal to end the process the CLI registered", || {
        harness.try_wait().ok().flatten()
    });
    harness.release_after_reap();

    // ---- 11. one more acknowledgement, so the seal link itself is anchored and no record
    // sits above the highest acknowledged sequence. The daemon's upload cadence does it; the
    // test waits for the evidence rather than for a duration.
    let acked = || -> Option<u64> {
        let text =
            std::fs::read_to_string(trace_dir.join(goalcup_protocol::export::ACKS_FILE)).ok()?;
        text.lines()
            .filter_map(|l| serde_json::from_str::<AckCheckpoint>(l).ok())
            .map(|ack| ack.seq)
            .max()
    };
    wait_for(
        &format!(
            "the seal link to be acknowledged (chain head {:?}, highest ack {:?})",
            chain_head(&trace_dir),
            acked()
        ),
        || {
            let head = chain_head(&trace_dir)?;
            (acked()? >= head).then_some(head)
        },
    );

    // ---- 12. the verdict, from the shipped verifier binary over the produced trace.
    let out = Command::new(sibling("goalcup-verify"))
        .arg(&trace_dir)
        .arg("--dev")
        .output()
        .expect("goalcup-verify runs");
    let verdict = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(
        out.status.code(),
        Some(2),
        "goalcup-verify did not give a run this API produced its transcript-only verdict.\nstderr:\n{}\nverdict:\n{verdict}",
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: serde_json::Value = serde_json::from_str(&verdict).expect("the verdict is JSON");
    assert_eq!(parsed["overall"], json!("fail"), "{verdict}");
    assert_eq!(
        review_queue(&parsed),
        vec![
            ("unwitnessed_keep_going", "flag"),
            ("leg1_without_leg2_post", "flag"),
            ("leg1_without_leg2_post", "flag"),
            ("leg1_call_without_leg2_post", "fail"),
        ],
        "{verdict}"
    );
    // The goal and one `keep going` have no leg-2 witness, so V1 holds both as unwitnessed
    // and the verdict lists no witnessed human message.
    assert_eq!(parsed["human_messages"].as_array().map(Vec::len), Some(0));

    daemon.stop();
    devserver.stop();
}

/// A real V4 workspace-submission run must preserve an unresolved process authority as a
/// non-clean terminal fact.  This is intentionally driven through the shipped binaries and
/// then through the shipped verifier: a library fixture could accidentally prove a different
/// receipt contract, a different process generation, or a different terminal projection.
/// The V4 gate end to end, twice: once over a stand-in this daemon can authenticate and
/// end, and once over one registered with an argv the launch never spawned.
///
/// **Read the positive case narrowly.** The stand-in is a single `/bin/sleep` with no
/// children, which on macOS is the one shape `terminate.rs` can bind to a process INSTANCE
/// — `terminate_darwin_singleton` acquires a Mach audit token and an `EVFILT_PROC` witness
/// for it, so its exit is observed exactly. A real Codex session is a tree (`goalcup` →
/// `node` → native `codex`, plus helpers in process groups of their own) and takes the
/// numeric descendant walk instead, which cannot make that claim. This test proves
/// the WIRING — cutoff, kill, record, verdict — and it is not evidence about Codex.
fn real_v4_workspace_run(owned: bool) {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let root = std::fs::canonicalize(tmp.path()).expect("the tempdir canonicalises");
    let opened_at = now_unix() - CLOCK_MARGIN;
    let devserver = Devserver::start_workspace(&format_z(opened_at));
    let config = write_workspace_submission_config(&root, &devserver.url);
    let mut daemon = DaemonProcess::start(&config);
    let api = Api {
        addr: daemon.control_addr.clone(),
        token: daemon.bearer(),
    };

    let workspace_opts = opts("claude-code-cli");
    let expected_manifest_sha256 = workspace_opts.manifest_sha256.clone();
    let info: WorkspaceInfo = api.post_json(ROUTE_WORKSPACES, &workspace_opts);
    let workspace = PathBuf::from(&info.workspace);
    let harness_home = PathBuf::from(&info.harness_home);
    let trace_dir = PathBuf::from(&info.artifact_root).join("trace");
    let entry = format!("{ROUTE_ENTRIES}/{}", info.entry_id);

    assert_eq!(api.post(&format!("{entry}/arm"), "").status, 204);
    let harness = gate::ChildGuard::new(
        Command::new("/bin/sleep")
            .arg("600")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("a stand-in for the harness"),
    );
    let pid = harness.id();
    let registration = ProcessRegistration {
        generation: info.process_generation,
        handle: ProcessHandle {
            pid,
            pgid: None,
            native_pid: None,
            native_binary: None,
            native_process_start: None,
            process_start: goalcup_protocol::process::process_start_identity(pid),
            strategy: KillStrategy::ProcessGroup,
            grace_seconds: 1,
            launched_at: format_z(now_unix()),
            program: "/bin/sleep".to_string(),
            // The negative run registers a program the pid is not running. The kill guard's
            // second question — what is this process ACTUALLY executing — refuses it, and a
            // refusal is the whole content of the outcome: nothing is signalled and the
            // seal may not say what became of the harness.
            argv: if owned {
                vec!["/bin/sleep".to_string(), "600".to_string()]
            } else {
                vec!["/usr/bin/true".to_string()]
            },
            env_keys: vec!["CLAUDE_CONFIG_DIR".to_string()],
            home_seed: None,
        },
    };
    let process = format!("{entry}/process");
    assert_eq!(api.post_body(&process, &registration).status, 204);

    let store = harness_home
        .join("projects")
        .join(goalcup_daemon::paths::claude_slug(&workspace))
        .join(format!("{}.jsonl", info.session_id));
    let session = info.session_id.clone();
    append(
        &store,
        &typed_prompt(&info.workspace, &session, "prompt-1", GOAL),
    );
    // The agent's own file, written through a TRACED tool call rather than behind the
    // trace's back. A file the chain does not explain is `unexplained_seal_file`, which is
    // a Fail about the WORKSPACE and would have hidden the thing this test is about — the
    // termination verdict — behind an unrelated one.
    append(
        &store,
        &assistant_write(&info.workspace, &session, "toolu_01"),
    );
    std::fs::write(workspace.join(WRITTEN_FILE), WRITTEN_BODY).expect("the agent's file");
    append(&store, &tool_result(&info.workspace, &session, "toolu_01"));

    let signal = wait_for("the V4 start signal", || {
        let response = api.get(&format!("{entry}/start-signal?timeout_ms=5000"));
        match response.status {
            200 => Some(serde_json::from_str::<StartSignal>(&response.body).expect("signal")),
            408 => None,
            409 if response.is_busy() => None,
            other => panic!(
                "the V4 start-signal poll answered {other}: {}",
                response.body
            ),
        }
    });
    assert_eq!(signal.entry_id, info.entry_id);
    wait_for("the V4 start receipt", || {
        status(&api, &info).payload.started_at
    });
    let receipt: StartReceipt = serde_json::from_slice(
        &std::fs::read(trace_dir.join(goalcup_protocol::export::RECEIPT_FILE))
            .expect("the signed V4 receipt"),
    )
    .expect("the receipt is valid JSON");
    receipt
        .verify_with(&goalcup_devserver::dev_server_public_key_hex())
        .expect("the receipt verifies with the expected devserver key");
    assert_eq!(receipt.claims.entry_id, info.entry_id);
    assert_eq!(receipt.claims.connector_id, "claude-code-cli");
    assert_eq!(receipt.claims.workspace, info.workspace);
    assert_eq!(receipt.claims.session_id, info.session_id);
    assert_eq!(receipt.claims.manifest_sha256, expected_manifest_sha256);
    assert_eq!(
        receipt.claims.questionnaire_version.as_deref(),
        Some("goal-cup-2026-v1")
    );
    assert_eq!(
        receipt.claims.run_public_key.as_deref(),
        Some(info.run_public_key.as_str())
    );
    wait_for_chain(&trace_dir, 3, 0);

    let sealed = api.post(&format!("{entry}/seal"), "");
    assert_eq!(sealed.status, 200, "{}", sealed.body);
    let first_report: FinishReportV1 =
        serde_json::from_str(&sealed.body).expect("V4 seal returns a FinishReportV1");
    if owned {
        let TerminationOutcomeV1::Confirmed {
            requested_at,
            confirmed_at,
            method,
        } = &first_report.termination
        else {
            panic!(
                "an owned, authenticated, observed-gone stand-in is a confirmed \
                 termination: {:?}",
                first_report.termination
            )
        };
        assert_eq!(
            method,
            &goalcup_protocol::provenance::TerminationMethodV1::ProcessSignal
        );
        assert!(
            goalcup_protocol::control::parse_z(requested_at.as_str())
                <= goalcup_protocol::control::parse_z(confirmed_at.as_str()),
            "a kill cannot be confirmed before it was requested: {requested_at:?} {confirmed_at:?}",
        );
    } else {
        assert!(
            matches!(
                first_report.termination,
                TerminationOutcomeV1::Unresolved {
                    reason: TerminationFailureV1::ProcessAuthorityUnavailable,
                    ..
                }
            ),
            "{:?}",
            first_report.termination
        );
    }
    assert!(matches!(
        first_report.submission,
        SubmissionOutcomeV1::Captured(_)
    ));
    let receipt_digest = receipt
        .canonical_digest()
        .expect("the receipt has a canonical digest");
    let (finalization_envelope, finalization) =
        std::fs::read_to_string(trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
            .expect("the V4 chain")
            .lines()
            .map(|line| serde_json::from_str::<Envelope>(line).expect("the V4 chain envelope"))
            .find_map(|envelope| {
                let finalization = match &envelope.body {
                    EventBody::FinalizationV1(finalization) => finalization.clone(),
                    _ => return None,
                };
                Some((envelope, finalization))
            })
            .expect("the V4 chain contains finalization");
    assert_eq!(finalization_envelope.entry_id, info.entry_id);
    assert_eq!(finalization.receipt_sha256(), &receipt_digest);
    let submission_manifest: SubmissionManifestV1 = serde_json::from_slice(
        &std::fs::read(trace_dir.join(goalcup_protocol::export::SUBMISSION_MANIFEST_FILE))
            .expect("the captured submission manifest"),
    )
    .expect("the submission manifest is valid JSON");
    assert_eq!(submission_manifest.entry_id, info.entry_id);
    assert_eq!(submission_manifest.receipt_sha256, receipt_digest);
    assert_eq!(
        goalcup_protocol::process::is_alive(pid),
        !owned,
        "the V4 path ends a harness it can authenticate and signals nothing it cannot"
    );
    let first_status = status(&api, &info);
    assert!(matches!(
        first_status.payload.finalization,
        Some(WorkspaceFinalizationStatusV1::Terminal { .. })
    ));

    daemon.stop();
    let mut restarted = DaemonProcess::start(&config);
    let restarted_api = Api {
        addr: restarted.control_addr.clone(),
        token: restarted.bearer(),
    };
    let replayed = restarted_api.post(&format!("{entry}/seal"), "");
    assert_eq!(replayed.status, 200, "{}", replayed.body);
    let replayed_report: FinishReportV1 =
        serde_json::from_str(&replayed.body).expect("restart replays the V4 report");
    assert_eq!(replayed_report, first_report);
    assert_eq!(
        goalcup_protocol::process::is_alive(pid),
        !owned,
        "a restart replays the durable outcome and neither revives nor re-signals anything"
    );
    let restarted_status = status(&restarted_api, &info);
    // The durable V4 finalization projection is byte-identical across restart.  The startup
    // sealed-harness reaper may additionally learn that its best-effort reap was unverified;
    // that independent sidecar is intentionally allowed to move while the terminal V4
    // outcomes remain immutable.
    assert_eq!(
        restarted_status.payload.finalization,
        first_status.payload.finalization
    );
    assert_eq!(
        restarted_status.payload.entry_id,
        first_status.payload.entry_id
    );
    assert_eq!(
        restarted_status.payload.workspace,
        first_status.payload.workspace
    );
    assert_eq!(
        restarted_status.payload.started_at,
        first_status.payload.started_at
    );
    assert_eq!(
        restarted_status.payload.deadline_at,
        first_status.payload.deadline_at
    );
    assert_eq!(
        restarted_status.payload.sealed_at,
        first_status.payload.sealed_at
    );

    // The seal link itself has to be anchored before the verdict, or V12 reports
    // `no_anchor` — a true statement about the server never having seen this chain, and one
    // that would sit between this test and the thing it is about.
    let acked = || -> Option<u64> {
        let text =
            std::fs::read_to_string(trace_dir.join(goalcup_protocol::export::ACKS_FILE)).ok()?;
        text.lines()
            .filter_map(|line| serde_json::from_str::<AckCheckpoint>(line).ok())
            .map(|ack| ack.seq)
            .max()
    };
    wait_for("the V4 seal link to be acknowledged", || {
        let head = chain_head(&trace_dir)?;
        (acked()? >= head).then_some(head)
    });

    let verified = Command::new(sibling("goalcup-verify"))
        .arg(&trace_dir)
        .arg("--dev")
        .output()
        .expect("goalcup-verify runs on the physical V4 trace");
    let verdict: serde_json::Value =
        serde_json::from_slice(&verified.stdout).expect("the verifier emits JSON");
    let v13 = verdict["checks"]
        .as_array()
        .and_then(|checks| checks.iter().find(|check| check["id"] == json!("V13")))
        .expect("V13 is present in the verifier result");
    let v13_findings = v13["findings"]
        .as_array()
        .expect("V13 findings are present");
    let termination_finding = v13_findings
        .iter()
        .find(|finding| finding["code"] == json!("workspace_submission_termination_not_confirmed"));
    if owned {
        assert_eq!(verified.status.code(), Some(2), "{verified:?}");
        assert_eq!(verdict["overall"], json!("fail"), "{verdict}");
        assert_eq!(
            review_queue(&verdict),
            vec![
                ("leg1_without_leg2_post", "flag"),
                ("leg1_call_without_leg2_post", "fail"),
            ],
            "{verdict}"
        );
        assert_eq!(v13["verdict"], json!("pass"), "{v13}");
        assert!(
            termination_finding.is_none(),
            "a confirmed termination leaves no termination finding: {v13}"
        );
        assert!(
            v13_findings
                .iter()
                .any(|finding| finding["code"] == json!("binding_checked")),
            "a clean terminal process outcome is what earns binding_checked: {v13}"
        );
    } else {
        assert_eq!(verified.status.code(), Some(2), "{verified:?}");
        assert_eq!(verdict["overall"], json!("fail"), "{verdict}");
        assert_eq!(v13["verdict"], json!("fail"), "{v13}");
        let termination_finding =
            termination_finding.expect("V13 preserves the unresolved termination finding");
        assert_eq!(termination_finding["severity"], json!("fail"), "{v13}");
        assert!(
            !v13_findings
                .iter()
                .any(|finding| finding["code"] == json!("binding_checked")),
            "a non-clean terminal process outcome cannot receive binding_checked: {v13}"
        );
    }

    restarted.stop();
}

/// A real workspace run, driven only
/// through the control API against the shipped binaries, ends its harness and verifies
/// clean. Read the caveat on `real_v4_workspace_run` before quoting this as evidence about
/// the reference harness.
#[test]
fn a_real_v4_workspace_run_confirms_termination_and_verifies_clean() {
    real_v4_workspace_run(true);
}

/// The same run with the process authority broken exactly one way: the handle names a
/// program the pid is not running. Nothing is signalled, the record says so with a reason,
/// and `goalcup-verify` withholds the pass. The negative case matters more than the
/// positive one — it is what stops a build that silently stopped killing anything from
/// looking identical to a build that kills correctly.
#[test]
fn a_real_v4_workspace_run_without_process_authority_stays_unresolved_and_is_not_a_pass() {
    real_v4_workspace_run(false);
}

fn typed_prompt(workspace: &str, session: &str, prompt_id: &str, text: &str) -> String {
    json!({
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

fn assistant_write(workspace: &str, session: &str, tool_use_id: &str) -> String {
    json!({
        "parentUuid": "uuid-prompt-1",
        "isSidechain": false,
        "message": {
            "model": "claude-opus-5",
            "id": "msg-1",
            "type": "message",
            "role": "assistant",
            "content": [
                { "type": "text", "text": "Writing hello.py." },
                {
                    "type": "tool_use",
                    "id": tool_use_id,
                    "name": "Write",
                    "input": {
                        "file_path": format!("{workspace}/{WRITTEN_FILE}"),
                        "content": WRITTEN_BODY,
                    },
                },
            ],
            "stop_reason": "tool_use",
            "usage": {
                "input_tokens": 1_200,
                "output_tokens": 340,
                "cache_read_input_tokens": 0,
                "cache_creation_input_tokens": 0,
            },
        },
        "requestId": "req-1",
        "type": "assistant",
        "uuid": "uuid-assistant-1",
        "timestamp": "2026-09-04T00:00:10.000Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": workspace,
        "sessionId": session,
        "version": HARNESS_VERSION,
    })
    .to_string()
}

fn tool_result(workspace: &str, session: &str, tool_use_id: &str) -> String {
    json!({
        "parentUuid": "uuid-assistant-1",
        "isSidechain": false,
        "promptId": "prompt-1",
        "type": "user",
        "message": {
            "role": "user",
            "content": [{
                "tool_use_id": tool_use_id,
                "type": "tool_result",
                "content": format!("Wrote {WRITTEN_FILE}"),
                "is_error": false,
            }],
        },
        "uuid": "uuid-tool-result-1",
        "timestamp": "2026-09-04T00:00:11.000Z",
        "sourceToolAssistantUUID": "uuid-assistant-1",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": workspace,
        "sessionId": session,
        "version": HARNESS_VERSION,
    })
    .to_string()
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
