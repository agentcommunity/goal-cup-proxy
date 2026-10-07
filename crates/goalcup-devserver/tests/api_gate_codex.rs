//! End to end through the CONTROL API: the record written before the bind is
//! chained, and the thread that follows it is not.
//!
//! `api_gate.rs` beside this file drives the same binaries on Claude Code, whose stores are
//! partitioned by a workspace path the daemon knows at arming time. Codex's `history` store
//! is not: it partitions `direct` on the SESSION id, which nothing knows until the harness
//! names its session, and `Partitioner::admit` judges once with D20 leaving nothing to
//! re-offer.
//!
//! The failure mode is total loss. Codex's `SessionStart` fires at the FIRST PROMPT, which
//! is the same moment that prompt is written to `history.jsonl` — and `history.jsonl` is,
//! by D2 and the manifest's own comment, the ONLY place a hook-blocked prompt survives. The
//! daemon's tick drains the stores and binds afterwards, so without the teaching that
//! record meets an empty tracked set, is discarded as `ForeignValue` and is never offered
//! again: submitting a goal over the length gate would delete its own leg-1 copy, for free.
//!
//! Both halves are asserted here, because a fix that kept the first record by keeping
//! everything would be worse than the bug:
//!
//!   * the pre-bind `history` record reaches the chain (the store taught itself the first
//!     session id it saw after arming), and
//!   * a SECOND thread writing to the same `history.jsonl` after the bind is judged foreign
//!     and reaches nothing.
//!
//! The shipped verifier IS run over the trace at the end, and it is asked to reach a
//! verdict and to read the run's two human messages — which is the assertion that the
//! capture produced a loadable trace rather than a directory of files. Which verdict is not
//! asserted: what a verifier makes of a Codex trace is `checks_v6_v11.rs`'s question, over
//! the committed M0 fixture, and this run's first human message is a goal the hook blocked.
//! This test is about what the daemon captures, which is the half no fixture can answer.
#![cfg(feature = "v1-compat")]

// The shared plumbing — the two child processes, the two loopback clients, the waits. By
// path rather than through `tests/support/mod.rs`, which pulls the verifier crate's fixture
// generator in with it.
#[path = "support/gate.rs"]
mod gate;

use gate::{
    Api, CLOCK_MARGIN, DaemonProcess, Devserver, append, chain_head, leg1_records, now_unix, opts,
    sibling, status, today, wait_for, write_config,
};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::control::{ROUTE_ENTRIES, ROUTE_WORKSPACES, WorkspaceInfo, format_z};
use goalcup_protocol::manifest::KillStrategy;
use goalcup_protocol::process::{ProcessHandle, ProcessRegistration};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The goal the length gate refused. Its ONLY leg-1 witness is `history.jsonl`: the
/// blocking hook aborted the turn, so no rollout record was ever written for it.
const BLOCKED_GOAL: &str = "write hello.py that prints the current date";
/// The second thread on the same machine — a participant's other Codex window, writing to
/// the one `history.jsonl` that harness home has.
const OTHER_THREAD: &str = "sess_someone_elses_thread";
/// Inside `tested_version_range`, so nothing here is about a version drift.
const HARNESS_VERSION: &str = "0.55.0";

#[test]
fn a_history_record_written_before_the_bind_is_chained_and_the_next_thread_is_not() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    // Canonical from the start: on macOS a tempdir is handed out as `/var/folders/…` while
    // a harness records `/private/var/folders/…` in its own `cwd`, and the partitioner
    // compares the two byte for byte.
    let root = std::fs::canonicalize(tmp.path()).expect("the tempdir canonicalises");

    let opened_at = now_unix() - CLOCK_MARGIN;
    let mut devserver = Devserver::start(&format_z(opened_at));
    let config = write_config(&root, &devserver.url);
    let mut daemon = DaemonProcess::start(&config);
    let api = Api {
        addr: daemon.control_addr.clone(),
        token: daemon.bearer(),
    };

    // ---- 1. the workspace, on the connector whose `history` store is the case.
    let info: WorkspaceInfo = api.post_json(ROUTE_WORKSPACES, &opts("codex-cli"));
    let config_dir = PathBuf::from(&info.harness_home);
    let trace_dir = PathBuf::from(&info.artifact_root).join("trace");
    let entry = format!("{ROUTE_ENTRIES}/{}", info.entry_id);
    let ws = info.workspace.clone();
    let session = info.session_id.clone();

    // ---- 2. arm. The pre-arm scan runs over an EMPTY harness home, which is the premise
    // the teaching rests on: from here to the bind there is no session in scope but the one
    // about to start, so the first id the `history` store sees is the participant's.
    assert_eq!(api.post(&format!("{entry}/arm"), "").status, 204);

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
        // The real CLI always carries the manifest-pinned termination identity, even when
        // its launch-time descendant scan has not found a native pid yet.
        native_binary: Some("codex".to_string()),
        native_process_start: None,
        process_start: goalcup_protocol::process::process_start_identity(harness.id()),
        strategy: KillStrategy::ProcessGroup,
        grace_seconds: 1,
        launched_at: format_z(now_unix()),
        program: "/bin/sleep".to_string(),
        argv: vec!["/bin/sleep".to_string(), "600".to_string()],
        env_keys: vec!["CODEX_HOME".to_string()],
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

    // ---- 3. the first prompt, in the order the harness produces it: written to
    // `history.jsonl` before anything has bound a session. The rollout's `session_meta` is
    // the first chained transcript record, and it is what arms the gate.
    let history = config_dir.join("history.jsonl");
    append(&history, &history_line(&session, BLOCKED_GOAL));
    append(&rollout_path(&config_dir), &session_meta(&ws, &session));

    // ---- 4. the daemon's own cadences: the tick captures, the gate clears, and the
    // receipt binds the session. Nothing here asks for any of it.
    let started = wait_for("the daemon to bind the start receipt", || {
        status(&api, &info).payload.started_at
    });
    assert!(!started.is_empty());

    // ---- 5. the pre-bind record, polled against an EMPTY
    // tracked set before the bind, is in the chain.
    wait_for("the pre-bind history record to reach the chain", || {
        (leg1_records(&trace_dir, "history") >= 1).then_some(())
    });
    assert_eq!(
        leg1_records(&trace_dir, "history"),
        1,
        "the blocked goal's only leg-1 witness, chained exactly once"
    );
    // …and it is THAT record and not a later one. A count alone cannot tell the two apart:
    // without the teaching the pre-bind record is discarded and the first history record
    // to reach the chain is whatever the harness writes AFTER the bind, which is a run that
    // has lost its goal and a count that has not noticed.
    assert!(
        std::fs::read_to_string(trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
            .expect("the chain reads")
            .contains(BLOCKED_GOAL),
        "the chained history record must be the blocked goal itself"
    );

    // ---- 6. …and the half a "keep everything" fix would have broken. A second Codex
    // window on the same machine writes to the same `history.jsonl`. The bind has put the
    // run's own session in the tracked set, so this one is judged and discarded — and the
    // tracked thread's NEXT record is what proves the foreign one was seen and passed over
    // rather than merely not yet read.
    append(
        &history,
        &history_line(OTHER_THREAD, "someone else's prompt"),
    );
    append(&history, &history_line(&session, "keep going"));
    wait_for(
        "the tracked thread's second record to reach the chain",
        || (leg1_records(&trace_dir, "history") >= 2).then_some(()),
    );
    assert_eq!(
        leg1_records(&trace_dir, "history"),
        2,
        "the second thread's record is judged foreign: D20 discards it before a hash is \
         taken over it, so it appears nowhere in the chain"
    );
    let chain = std::fs::read_to_string(trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
        .expect("the chain reads");
    assert!(
        !chain.contains(OTHER_THREAD),
        "the foreign thread's id must not appear anywhere in this run's chain"
    );

    // ---- 7. and the seal, so the run ends the way a participant ends it.
    let sealed = api.post(&format!("{entry}/seal"), "");
    assert_eq!(sealed.status, 200, "{}", sealed.body);
    wait_for("the seal to end the process the CLI registered", || {
        harness.try_wait().ok().flatten()
    });
    harness.release_after_reap();

    // ---- 8. one more acknowledgement, so the seal link itself is anchored before the
    // trace is read.
    let acked = || -> Option<u64> {
        let text =
            std::fs::read_to_string(trace_dir.join(goalcup_protocol::export::ACKS_FILE)).ok()?;
        text.lines()
            .filter_map(|l| serde_json::from_str::<AckCheckpoint>(l).ok())
            .map(|ack| ack.seq)
            .max()
    };
    wait_for("the seal link to be acknowledged", || {
        let head = chain_head(&trace_dir)?;
        (acked()? >= head).then_some(head)
    });

    // ---- 9. and the verdict, RUN rather than asserted to be installed:
    // `sibling("goalcup-verify").is_file()` says only that cargo built a neighbour, and
    // holds just as well over a trace the verifier cannot open at all, which is the one
    // outcome a capture test needs to rule out.
    //
    // **The verdict this trace earns is `fail`, and that is the assertion.** This run has
    // no leg-2 `UserPromptSubmit` — Codex's blocked turn is exactly the case where there is
    // none — so V1 recovers no human message and reports `goal_missing`. What the OTHER
    // twelve checks say is the thing worth having: the chain loads, its anchors and its
    // start binding check out, and the stores are partitioned. A capture that produced a
    // directory of files rather than a readable trace fails those, not this one.
    //
    // Which verdict a COMPLETE Codex run earns is `checks_v6_v11.rs`'s question, over the
    // committed M0 fixture; this test is about what the daemon captured.
    let out = Command::new(sibling("goalcup-verify"))
        .arg(&trace_dir)
        .arg("--dev")
        .output()
        .expect("goalcup-verify runs");
    let verdict = String::from_utf8_lossy(&out.stdout).to_string();
    let parsed: serde_json::Value = serde_json::from_str(&verdict).unwrap_or_else(|e| {
        panic!(
            "the verifier answered with no verdict about this trace: {e}\nstderr:\n{}\nstdout:\n{verdict}",
            String::from_utf8_lossy(&out.stderr)
        )
    });
    let check = |id: &str| -> String {
        parsed["checks"]
            .as_array()
            .expect("the verdict lists its checks")
            .iter()
            .find(|c| c["id"] == json!(id))
            .unwrap_or_else(|| panic!("{id} is in the verdict: {verdict}"))["verdict"]
            .as_str()
            .expect("a check states a verdict")
            .to_string()
    };
    for id in ["V4", "V10", "V12", "V13"] {
        assert_eq!(
            check(id),
            "pass",
            "{id} is what says the daemon produced a READABLE trace — one tracked session, \
             partitioned stores, anchored chain, checked start binding: {verdict}"
        );
    }
    assert_eq!(
        parsed["overall"],
        json!("fail"),
        "and the run itself is not a passing one, for the single reason below: {verdict}"
    );
    assert_eq!(
        parsed["review_queue"]
            .as_array()
            .expect("a review queue")
            .iter()
            .filter(|f| f["severity"] == json!("fail"))
            .map(|f| f["code"].as_str().expect("a code"))
            .collect::<Vec<_>>(),
        vec!["goal_missing"],
        "the blocked turn posted no `UserPromptSubmit`, so V1 recovers no human message — \
         and that is the ONLY thing wrong with this trace: {verdict}"
    );

    daemon.stop();
    devserver.stop();
}

/// One `history.jsonl` line: a flat record whose session is at the
/// top level, which is why the store's partition is `direct` on `session_id`.
fn history_line(session: &str, text: &str) -> String {
    json!({
        "session_id": session,
        "ts": now_unix(),
        "text": text,
    })
    .to_string()
}

/// `{config_dir}/sessions/{yyyy}/{mm}/{dd}/rollout-*.jsonl`, with today's date, because
/// that is the glob the `rollout` store renders and the daemon renders it from its own
/// clock.
fn rollout_path(config_dir: &Path) -> PathBuf {
    let (y, m, d) = today();
    config_dir
        .join("sessions")
        .join(y)
        .join(m)
        .join(d)
        .join("rollout-2026-01-01T00-00-00-run.jsonl")
}

/// The rollout's opening record: the `session_meta` whose `payload.cwd` decides the whole
/// file (`file_first_record`) and whose `payload.session_id` is the session `session_of`
/// reads. Codex 0.153.0 writes both `session_id` and `id` here, equal, and only here — every
/// `response_item` after it carries `id` alone, and that one is the ITEM's.
fn session_meta(workspace: &str, session: &str) -> String {
    json!({
        "timestamp": "2026-09-04T09:12:33.401Z",
        "type": "session_meta",
        "payload": {
            "session_id": session,
            "id": session,
            "cwd": workspace,
            "cli_version": HARNESS_VERSION,
            "ordinal": 0,
        },
    })
    .to_string()
}
