//! The Phase 1 recorder crash campaign, Task 3 slice of T25: four rows against the real
//! `goalcup`, `goalcup-daemon`, in-process proxy and `goalcup-verify`, on the synthetic workload
//! of `m2_synthetic_vertical`. A daemon killed under traffic, a genuine torn tail, a fresh
//! epoch with the receiver alive, and a recovery that never speaks to the harness.
//!
//! The faults are real processes and real operating-system conditions, nothing else:
//!
//! - the crash is `kill -9` of the one daemon the test spawned, while the recording proxy in
//!   `goalcup start` and the workload keep running; the restart is a second real
//!   `goalcup-daemon` over the same private registry;
//! - the torn tail is a real partial append: the daemon child runs under an `RLIMIT_FSIZE`
//!   (`capture_crash_support::Daemon::spawn`), so the production writer's own `write` is cut
//!   short by the kernel mid-line and the next one fails. No byte of the chain is written by
//!   the test;
//! - the receiver-alive fresh epoch is the owned daemon stopped and continued with `SIGSTOP` /
//!   `SIGCONT` (the T23 seam): the daemon process, its pid and its registry are never replaced.
//!
//! The daemon, protocol and sender suites own the protocol semantics (`capture_lifecycle.rs`,
//! `m2_capture_sender.rs`, `m2_emitter.rs`, `m2_ipc_compose.rs`); nothing here restates them.
//! These rows assert only what crosses a process boundary: the bytes on disk before and after
//! every inspection, the relay's delivery measured by the workload and the upstream endpoint,
//! the signed status, the control API's refusals, the CLI's outcomes and the verifier's literal
//! exit code. Power loss, a crashed relay, peer-initiated disconnect with a live receiver and
//! arbitrary v5 readmission are not measured here.
//!
//! macOS and Linux (the trace folder must be on APFS, ext4 or XFS). Each row runs real processes
//! for tens of seconds, so the campaign is in the `full` tier, one row at a time (the `serial` test group in
//! `.config/nextest.toml`); `sh scripts/gate.sh full` builds the binaries first and runs it. No
//! row needs a real harness or the network.

// Shared with the storage campaign; each campaign uses a subset.
use crate::common::capture_campaign_support;
use crate::common::capture_crash_support;
use crate::common::m2_vertical_support;

use crate::common::table_lock;

use capture_campaign_support::{
    OBSERVE, RUN, Row, capture, child_of, delivered, kill9_owned_child, kill9_owned_daemon,
    ps_command, ps_started, running, signal_owned_daemon, wait_capture, wait_delivered,
    wait_exited,
};
use capture_crash_support::{
    Daemon, cli, leaves_trace_unchanged, tail_after_last_newline, tree_hashes,
};
use goalcup_protocol::capture::{
    CaptureRecord, CaptureSource, CaptureState, GapReason, M2Status, MAX_ENVELOPE_BYTES, RunState,
};
use goalcup_protocol::envelope::Envelope;
use m2_vertical_support::{Chain, Launcher, RESERVE, Vertical, now, wait_until};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Duration;
use table_lock::TableLock;

/// Where this campaign's rows keep their evidence, under `target/`.
const EVIDENCE: &str = "phase1-task3-evidence";

/// The consecutive reconnect delays of one launch are 1, 2, 5 and 10 s (`RECONNECT_DELAYS`): a
/// window longer than their sum. What it shows is only that nothing moved: the restarted daemon
/// binds a new ingest port (`daemon.rs`, `bind_loopback(loopback(0), ..)`) and the surviving
/// sender keeps the first daemon's `ingest_addr` (`start.rs`), so it could not reach the new
/// daemon whatever the daemon's policy. The daemon's refusal to admit a producer after a
/// restart is proven only by the in-process `capture_lifecycle.rs` tests
/// (`a_restart_never_rebuilds_a_capture_row_as_a_legacy_runtime`,
/// `a_registered_run_reopened_before_its_launch_chains_the_loss`,
/// `a_restart_keeps_the_gap_history_through_the_seal`).
const READMISSION_WINDOW: Duration = Duration::from_secs(20);

/// How long the daemon is stopped in the fresh-epoch rows: the T23 value, long enough for the
/// sender's bounded queue to fill and cut the epoch.
const STALL: Duration = Duration::from_secs(25);

/// The file-size limit of the torn-tail row's daemon: above every file the daemon keeps but the
/// chain (the registry is about 2.6 MiB), and reached by the chain in a few seconds of the
/// `stream` workload (about 0.5 MiB/s).
const TORN_LIMIT: u64 = 6 * 1024 * 1024;

// ------------------------------------------------------------------ shared observations

fn gaps_of(chain: &Chain) -> Vec<(bool, GapReason, Option<u64>)> {
    chain
        .records()
        .filter_map(|r| match r {
            CaptureRecord::Gap {
                stream,
                reason,
                lost_bytes,
                ..
            } => Some((stream.is_some(), *reason, *lost_bytes)),
            _ => None,
        })
        .collect()
}

/// Exchanges the workload completed with `ok` true that began at or after `since`, by request
/// hash: the bytes the relay delivered, as the workload knows them.
fn echoes_since(v: &Vertical, since: f64) -> Vec<String> {
    v.events()
        .iter()
        .filter(|e| e["lane"] == "echo" && e["ok"] == true)
        .filter(|e| e["start"].as_f64().is_some_and(|t| t >= since))
        .filter_map(|e| e["req_sha256"].as_str().map(str::to_string))
        .collect()
}

/// Retained wire-request streams whose bytes hash to one of `hashes`.
fn retained_requests(chain: &Chain, hashes: &[String]) -> usize {
    chain
        .streams()
        .values()
        .filter(|s| s.source == CaptureSource::WireRequest && hashes.contains(&s.sha256()))
        .count()
}

/// The most times one request payload is retained: more than once would be a replay.
fn max_request_multiplicity(chain: &Chain) -> usize {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for stream in chain.streams().values() {
        if stream.source == CaptureSource::WireRequest && !stream.data.is_empty() {
            *seen.entry(stream.sha256()).or_default() += 1;
        }
    }
    seen.values().copied().max().unwrap_or(0)
}

fn launches(chain: &Chain) -> usize {
    chain.count(|r| matches!(r, CaptureRecord::Launch { .. }))
}

fn assistance(chain: &Chain) -> usize {
    chain.count(|r| matches!(r, CaptureRecord::Assistance { .. }))
}

fn recoveries(chain: &Chain) -> usize {
    chain.count(|r| matches!(r, CaptureRecord::Recovery { .. }))
}

fn local_finishes(chain: &Chain) -> usize {
    chain.count(|r| matches!(r, CaptureRecord::LocalFinish { .. }))
}

/// Run directories `goalcup start` made beside the goal file: a relaunch would make a second.
fn run_directories(v: &Vertical) -> usize {
    std::fs::read_dir(v.goal.parent().unwrap())
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .count()
}

/// What the upstream endpoint, counting outside the workload, the recording proxy and the recorder,
/// saw: every echo request and every WebSocket message exactly as many times as the workload sent
/// it. A request the relay replayed would be counted twice here.
fn assert_upstream_saw_each_once(v: &Vertical, row: &mut Row) {
    let counters = v.endpoint_counters();
    row.note(
        "endpoint_counters",
        serde_json::to_value(&counters).unwrap(),
    );
    let events = v.events();
    let sent = |lane: &str| events.iter().filter(|e| e["lane"] == lane).count() as u64;
    assert_eq!(
        counters.echo_requests,
        sent("echo"),
        "the upstream saw a different number of echo requests than the workload sent"
    );
    assert_eq!(
        counters.ws_messages_in,
        sent("ws"),
        "the upstream saw a different number of WebSocket messages than the workload sent"
    );
    // Nothing else reached the upstream, and nothing it saw was malformed. (A client closing an
    // idle keep-alive connection without a TLS close is counted as `interrupted_requests` and
    // says nothing about replay, so it is recorded but not asserted.)
    assert_eq!(
        counters.requests,
        counters.echo_requests + counters.ws_upgrades,
        "the upstream saw a request that is neither an echo nor the WebSocket upgrade"
    );
    assert_eq!((counters.bad_requests, counters.tls_failures), (0, 0));
}

/// The launcher outlived the daemon it had been talking to, so at its finish its control client
/// still names the first daemon's port and token. The finish re-reads `control.json` once (H-14),
/// reaches the restarted daemon, which permits status and seal, and seals the entry: `goalcup
/// start` exits 0 with "finished locally". Before H-14 it exited 1, "the daemon is not
/// reachable", having sealed nothing, and the participant ran their own `goalcup seal`.
fn assert_launcher_finished_on_the_restarted_daemon(start: &mut Launcher, row: &mut Row) {
    let exit = start.wait(RUN);
    row.note("start_exit", exit.code());
    row.note("start_output", start.output());
    assert_eq!(exit.code(), Some(0), "{}", start.output());
    assert!(
        start.output().contains("finished locally"),
        "{}",
        start.output()
    );
    assert!(
        !start.output().contains("the daemon is not reachable"),
        "{}",
        start.output()
    );
}

/// The launcher outlived a daemon that was restarted over a chain the restart could not reopen
/// (a torn tail), so it reaches the restarted daemon and is refused: nothing is sealed, and the
/// exit is a refusal, not the unreachable daemon it used to report.
fn assert_launcher_was_refused_by_the_restarted_daemon(start: &mut Launcher, row: &mut Row) {
    let exit = start.wait(RUN);
    row.note("start_exit", exit.code());
    row.note("start_output", start.output());
    assert_ne!(exit.code(), Some(0), "{}", start.output());
    assert!(
        !start.output().contains("the daemon is not reachable"),
        "{}",
        start.output()
    );
    assert!(
        !start.output().contains("finished locally"),
        "{}",
        start.output()
    );
}

/// What a daemon crash left, a restart over the same registry, and what the restart did.
struct Crash {
    killed_at: f64,
    dead: Chain,
    restarted: Daemon,
    after_restart: Chain,
    status: M2Status,
}

/// `kill -9` the world's own daemon under traffic, retain what it left, prove the relay and the
/// harness lived on and nothing wrote, restart a daemon over the same private registry, and
/// check that the restart only reopened the entry for status and seal.
fn crash_and_restart(v: &Vertical, entry: &str, row: &mut Row, start: &mut Launcher) -> Crash {
    let trace = v.trace(entry);
    let harness = v.harness_pid() as i32;
    let harness_started = ps_started(harness);
    let before = v.chain(entry);

    // The crash: `kill -9` of the one daemon this test started.
    let victim = v.daemon_pid();
    let killed_at = now();
    kill9_owned_daemon(v, victim);
    row.note("killed_daemon_pid", victim);

    // Retained before anything else touches it: the exact bytes the dead daemon left.
    let dead = v.chain(entry);
    row.keep_bytes("chain-after-kill.jsonl", &dead.bytes);
    row.note("chain_after_kill_bytes", dead.bytes.len());
    row.note("chain_after_kill_sha256", dead.sha256());
    row.note("chain_after_kill_lines", dead.lines.len());
    assert!(
        dead.bytes.starts_with(&before.bytes),
        "the prefix only grew"
    );
    assert!(
        tail_after_last_newline(&dead.bytes).is_empty(),
        "the kill landed between appends; a tail cut inside an append is the torn-tail row's"
    );

    // Only the daemon died: the launcher, its proxy and the harness are the same processes.
    assert!(!start.exited(), "goalcup start survived the daemon");
    assert!(running(harness), "the harness survived the daemon");
    assert_eq!(ps_started(harness), harness_started);

    // The relay, measured outside the recorder: the workload's own byte-checked exchanges that
    // began after the kill all passed, and a dead recorder wrote nothing.
    wait_delivered(v, killed_at, 10, 5, RUN);
    let after_kill = delivered(v, killed_at);
    row.note("delivered_after_kill", format!("{after_kill:?}"));
    assert_eq!(after_kill.failed, 0, "no exchange failed through the crash");
    assert_eq!(
        v.chain(entry).sha256(),
        dead.sha256(),
        "nothing wrote while the daemon was dead"
    );

    // A look at the dead daemon's trace, which must not change a byte of it. An honest
    // incomplete verdict: valid records, no terminal, no gap recorded yet.
    let (code, integrity) = leaves_trace_unchanged(&trace, "the verifier", || v.verify(entry));
    row.note("verifier_exit_unsealed_prefix", code);
    row.note("verifier_unsealed_prefix", integrity.clone());
    assert_eq!(code, 1, "{integrity}");
    assert_eq!(integrity["integrity"], "incomplete", "{integrity}");
    assert_eq!(integrity["sealed"], false, "{integrity}");
    assert_eq!(integrity["torn_tail"], false, "{integrity}");
    assert_eq!(integrity["terminal"], Value::Null, "{integrity}");
    assert_eq!(
        integrity["validated_bytes"].as_u64(),
        Some(dead.bytes.len() as u64),
        "{integrity}"
    );
    assert_eq!(
        integrity["validated_records"].as_u64(),
        Some(dead.lines.len() as u64),
        "{integrity}"
    );
    assert!(
        integrity["findings"]
            .as_array()
            .unwrap()
            .contains(&json!("chain_unsealed")),
        "{integrity}"
    );

    // The restart under the same private registry: a real second daemon over the same roots.
    let restarted = Daemon::spawn(v, "restart", None);
    row.note("restarted_daemon_pid", restarted.pid());
    let dead_gaps = gaps_of(&dead);
    let status = wait_capture(v, entry, "the reopened entry", OBSERVE, |s| {
        s.gap_count > dead_gaps.len() as u64
    });
    row.note("reopened_status", serde_json::to_value(&status).unwrap());
    let after_restart = v.chain(entry);
    row.keep_bytes("chain-after-restart.jsonl", &after_restart.bytes);
    row.note("chain_after_restart_sha256", after_restart.sha256());

    // Reopened for status and seal only: the dead daemon's bytes are a prefix, and the restart
    // added exactly one record, the persisted unknown ingest interval. Nothing was truncated,
    // rewritten or replayed.
    assert!(
        after_restart.bytes.starts_with(&dead.bytes),
        "the dead daemon's bytes are a prefix of the reopened chain"
    );
    assert_eq!(after_restart.lines.len(), dead.lines.len() + 1);
    assert_eq!(
        gaps_of(&after_restart),
        [
            dead_gaps.clone(),
            vec![(false, GapReason::IngestLost, None)]
        ]
        .concat(),
        "the gaps already chained, then one stream-less IngestLost gap with no invented byte count"
    );
    assert!(matches!(
        after_restart.lines.last().unwrap().record,
        Some(CaptureRecord::Gap {
            stream: None,
            reason: GapReason::IngestLost,
            lost_bytes: None,
            ..
        })
    ));
    assert_eq!(status.run, RunState::Running);
    assert_eq!(status.capture, CaptureState::Incomplete);
    assert_eq!(status.gap_count, dead_gaps.len() as u64 + 1);
    assert!(status.storage.accounting_available);
    assert_eq!(
        status.storage.durable.as_ref().map(|d| d.chain_bytes),
        Some(after_restart.bytes.len() as u64),
        "the reopen's gap is durable"
    );
    assert!(!status.assistance_observed);
    assert_eq!(launches(&after_restart), 1);
    assert_eq!(assistance(&after_restart), 0);

    Crash {
        killed_at,
        dead,
        restarted,
        after_restart,
        status,
    }
}

/// With the restarted daemon up and the relay still running: the relay keeps delivering, and
/// nothing moves
/// (chain, producers, status) across a window longer than the sender's whole reconnect
/// schedule. This observes no movement; it does not prove the daemon's readmission policy,
/// because the sender cannot reach the restarted daemon's new ingest port (see
/// `READMISSION_WINDOW`).
fn assert_no_readmission(v: &Vertical, entry: &str, row: &mut Row, crash: &Crash) {
    let window_began = now();
    std::thread::sleep(READMISSION_WINDOW);
    wait_delivered(v, window_began, 10, 5, RUN);
    let later = v.chain(entry);
    let status = capture(v, entry);
    row.note(
        "status_after_window",
        serde_json::to_value(&status).unwrap(),
    );
    assert_eq!(
        later.sha256(),
        crash.after_restart.sha256(),
        "the chain did not move in {READMISSION_WINDOW:?} (observed only; see READMISSION_WINDOW)"
    );
    assert_eq!(
        later.producers(),
        crash.dead.producers(),
        "no new producer has a record"
    );
    assert_eq!(status, crash.status, "the status did not move either");
    assert_eq!(delivered(v, window_began).failed, 0);
    assert!(running(crash.restarted.pid()));
}

// ------------------------------------------------------------------ rows

#[test]
fn daemon_exit_preserves_prefix_and_honest_recovery_limit() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("stream", json!({}));
    let mut row = Row::new(
        "daemon_exit_preserves_prefix_and_honest_recovery_limit",
        &v,
        EVIDENCE,
    );
    let mut start = v.start(RESERVE);
    let entry = v.entry_id();
    let harness = v.harness_pid() as i32;

    // A healthy capture with a durable frontier, traffic flowing and no loss.
    wait_delivered(&v, 0.0, 10, 5, RUN);
    let healthy = wait_capture(&v, &entry, "a durable frontier", OBSERVE, |s| {
        s.storage.durable.as_ref().is_some_and(|d| d.seq >= 8)
    });
    row.note("healthy_status", serde_json::to_value(&healthy).unwrap());
    assert_eq!(healthy.capture, CaptureState::Recording);
    assert_eq!(healthy.gap_count, 0);
    let before_hashes: Vec<String> = echoes_since(&v, 0.0);

    let crash = crash_and_restart(&v, &entry, &mut row, &mut start);
    assert!(
        retained_requests(&crash.dead, &before_hashes) > 0,
        "the retained prefix holds real exchanges"
    );
    assert_no_readmission(&v, &entry, &mut row, &crash);

    // The finish. The launcher's control client names the first daemon; it re-reads control.json
    // and seals on the restarted one (H-14).
    v.release();
    assert_launcher_finished_on_the_restarted_daemon(&mut start, &mut row);

    // The honest limit: what the relay delivered while no recorder ran is gone from the
    // evidence, reported as an unknown gap and never reconstructed.
    let chain = v.chain(&entry);
    row.keep_bytes("chain-sealed.jsonl", &chain.bytes);
    row.note("final_chain_sha256", chain.sha256());
    assert!(
        chain.bytes.starts_with(&crash.after_restart.bytes),
        "the seal only appended"
    );
    assert_eq!(chain.lines.len(), crash.after_restart.lines.len() + 1);
    assert_eq!(local_finishes(&chain), 1);
    assert!(matches!(
        chain.lines.last().unwrap().record,
        Some(CaptureRecord::LocalFinish { .. })
    ));
    assert_eq!(gaps_of(&chain), gaps_of(&crash.after_restart));
    assert_eq!(launches(&chain), 1);
    assert_eq!(assistance(&chain), 0);
    assert_eq!(
        recoveries(&chain),
        0,
        "no Recovery record: no producer epoch was recorded after the crash"
    );
    assert_eq!(run_directories(&v), 1, "one run, never relaunched");
    assert_eq!(
        v.harness_pid() as i32,
        harness,
        "the same harness, never relaunched"
    );
    let lost = echoes_since(&v, crash.killed_at);
    row.note("echoes_delivered_after_kill", lost.len());
    assert!(lost.len() >= 10);
    assert_eq!(
        retained_requests(&chain, &lost),
        0,
        "no exchange the relay delivered after the crash appears in the evidence"
    );
    assert_eq!(v.report()["ok"], true, "every exchange passed");
    assert_upstream_saw_each_once(&v, &mut row);

    let sealed = capture(&v, &entry);
    row.note("sealed_status", serde_json::to_value(&sealed).unwrap());
    assert_eq!(sealed.run, RunState::Sealed);
    assert_eq!(sealed.gap_count, 1, "the gap outlives the seal");
    assert!(!sealed.assistance_observed);
    let (code, integrity) = v.verify(&entry);
    row.note("verifier_exit", code);
    row.note("verifier", integrity.clone());
    assert_eq!(code, 1, "{integrity}");
    assert_eq!(integrity["integrity"], "incomplete");
    assert_eq!(integrity["sealed"], true);
    assert_eq!(integrity["coverage_incomplete"], true);
    assert_eq!(integrity["gap_count"], 1);
    assert_eq!(integrity["terminal"]["kind"], "local_finish");
    assert!(
        integrity["findings"]
            .as_array()
            .unwrap()
            .contains(&json!("gap_recorded")),
        "{integrity}"
    );
}

#[test]
fn torn_tail_is_preserved_and_nonresumable() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("stream", json!({}));
    let mut row = Row::new("torn_tail_is_preserved_and_nonresumable", &v, EVIDENCE);
    // The world's own daemon holds no entry yet: end it, and run the same registry's next
    // daemon under the file-size limit, so the only writer of the chain is the production one.
    let first = v.daemon_pid();
    signal_owned_daemon(&v, first, libc::SIGTERM);
    wait_exited(first, "the idle first daemon to exit");
    let mut limited = Daemon::spawn(&v, "limited", Some(TORN_LIMIT));
    row.note("limited_daemon_pid", limited.pid());
    row.note("file_size_limit", TORN_LIMIT);
    let mut start = v.start(16 * 1024 * 1024);
    let entry = v.entry_id();
    let trace = v.trace(&entry);
    let harness = v.harness_pid() as i32;

    // The kernel cuts the production writer's own append short mid-line, then refuses the
    // next: the writer fails, accounting is unknown, and the chain keeps a partial line.
    let failed = wait_capture(&v, &entry, "the append failure", RUN, |s| {
        s.capture == CaptureState::Failed
    });
    let failed_at = now();
    row.note("failed_status", serde_json::to_value(&failed).unwrap());
    assert_eq!(failed.run, RunState::Running);
    assert!(!failed.storage.accounting_available);
    let torn = v.chain(&entry);
    let tail = tail_after_last_newline(&torn.bytes).to_vec();
    row.keep_bytes("chain-torn.jsonl", &torn.bytes);
    row.keep_bytes("torn-tail.bin", &tail);
    row.note("torn_chain_bytes", torn.bytes.len());
    row.note("torn_chain_sha256", torn.sha256());
    row.note("torn_chain_complete_lines", torn.lines.len());
    row.note("torn_tail_bytes", tail.len());
    assert_eq!(
        torn.bytes.len() as u64,
        TORN_LIMIT,
        "the write stopped exactly at the operating-system limit"
    );
    assert!(!tail.is_empty() && tail.len() < MAX_ENVELOPE_BYTES);
    assert!(
        tail.starts_with(b"{\"protocol_version\":5,"),
        "the tail is the start of a real v5 envelope"
    );
    assert!(
        serde_json::from_slice::<Envelope>(&tail).is_err(),
        "and it is cut short"
    );
    assert_eq!(
        failed.storage.used_bytes,
        (torn.bytes.len() - tail.len()) as u64,
        "the daemon accounted only the complete lines"
    );
    assert!(
        failed
            .storage
            .durable
            .as_ref()
            .is_some_and(|d| d.chain_bytes <= failed.storage.used_bytes),
        "no durable frontier past the complete lines"
    );

    // The relay lives on, measured by the workload outside the recorder.
    wait_delivered(&v, failed_at, 10, 5, RUN);
    let delivered_after = delivered(&v, failed_at);
    row.note("delivered_after_failure", format!("{delivered_after:?}"));
    assert_eq!(delivered_after.failed, 0);
    assert!(!start.exited());
    assert!(running(harness));

    // The crash: `kill -9` of the limited daemon, which this test started.
    limited.kill9(&v);
    let dead_hashes = tree_hashes(&trace);
    assert_eq!(
        v.chain(&entry).bytes,
        torn.bytes,
        "the crash changed nothing"
    );

    // Integrity on the torn chain: invalid, with the validated prefix named, and every look at
    // the trace leaves each byte of it as it was.
    let (code, integrity) = leaves_trace_unchanged(&trace, "the verifier", || v.verify(&entry));
    row.note("verifier_exit_torn", code);
    row.note("verifier_torn", integrity.clone());
    assert_eq!(code, 2, "{integrity}");
    assert_eq!(integrity["integrity"], "invalid", "{integrity}");
    assert_eq!(integrity["torn_tail"], true, "{integrity}");
    assert_eq!(integrity["sealed"], false, "{integrity}");
    assert_eq!(integrity["terminal"], Value::Null, "{integrity}");
    assert_eq!(
        integrity["validated_bytes"].as_u64(),
        Some((torn.bytes.len() - tail.len()) as u64),
        "{integrity}"
    );
    assert_eq!(
        integrity["validated_records"].as_u64(),
        Some(torn.lines.len() as u64),
        "{integrity}"
    );
    assert!(
        integrity["findings"]
            .as_array()
            .unwrap()
            .contains(&json!("torn_tail")),
        "{integrity}"
    );
    let (again, repeated) = v.verify(&entry);
    assert_eq!((again, &repeated), (code, &integrity), "repeatable");

    // The restart under the same private registry. The torn chain cannot be reopened (a
    // resume validates every line and refuses a missing newline), so the daemon holds no
    // capture entry for it: no status, no seal, no resume, no gap record, no repair.
    let restarted = Daemon::spawn(&v, "restart", None);
    row.note("restarted_daemon_pid", restarted.pid());
    let (st, out, err) =
        leaves_trace_unchanged(&trace, "status", || cli(&v, &["status", &entry, "--json"]));
    row.note("status_exit", st.code());
    row.note("status_stderr", err.clone());
    // CLI ladder rung 1: refused, the daemon has no status for that entry.
    assert_eq!(st.code(), Some(1), "{out}{err}");
    assert!(err.contains("no status for that entry"), "{err}");
    let (st, out, err) =
        leaves_trace_unchanged(&trace, "seal", || cli(&v, &["seal", "--yes", &entry]));
    row.note("seal_exit", st.code());
    row.note("seal_stderr", err.clone());
    // CLI ladder rung 1 (H-15): a safe refusal. The daemon holds the row as a capture but has no
    // open capture to finish, so it answers the CLI's legacy seal `malformed`; the CLI says the
    // entry is torn, that nothing was changed, and how to read what survives. It used to exit 3
    // with "malformed control response".
    assert_eq!(st.code(), Some(1), "{out}{err}");
    assert!(
        err.contains(&format!("Part {entry} cannot be sealed")),
        "{err}"
    );
    assert!(err.contains("torn or unreadable"), "{err}");
    assert!(
        err.contains("Nothing was sealed, written or repaired"),
        "{err}"
    );
    assert!(err.contains("goalcup-verify --capture-integrity"), "{err}");
    assert!(!trace.join("seal.json").exists());

    // No repair and no movement across a window longer than the sender's whole reconnect
    // schedule, while the relay keeps delivering. (The restarted daemon holds no entry for this
    // chain at all, so there is no ingest listener for the sender to reach.)
    let window_began = now();
    std::thread::sleep(READMISSION_WINDOW);
    wait_delivered(&v, window_began, 10, 5, RUN);
    assert_eq!(
        tree_hashes(&trace),
        dead_hashes,
        "every byte of the trace as the crash left it"
    );
    assert_eq!(delivered(&v, window_began).failed, 0);

    // The finish: the launcher reaches the restarted daemon, which cannot seal this entry either,
    // and the torn bytes are still the only bytes.
    v.release();
    assert_launcher_was_refused_by_the_restarted_daemon(&mut start, &mut row);
    let final_chain = v.chain(&entry);
    assert_eq!(
        final_chain.bytes, torn.bytes,
        "byte-identical to the torn chain"
    );
    assert_eq!(tail_after_last_newline(&final_chain.bytes), &tail[..]);
    assert_eq!(tree_hashes(&trace), dead_hashes);
    assert_eq!(local_finishes(&final_chain), 0);
    assert_eq!(assistance(&final_chain), 0);
    assert_eq!(recoveries(&final_chain), 0);
    assert_eq!(launches(&final_chain), 1);
    assert_eq!(
        final_chain.count(|r| matches!(r, CaptureRecord::Gap { .. })),
        0,
        "no gap record: appending one to a torn chain would first require repairing it"
    );
    assert_eq!(run_directories(&v), 1);
    assert_eq!(v.report()["ok"], true, "every exchange passed");
    assert_upstream_saw_each_once(&v, &mut row);
    assert!(running(restarted.pid()));
    let (code, integrity) = v.verify(&entry);
    row.note("verifier_exit_final", code);
    assert_eq!(code, 2, "{integrity}");
    assert_eq!(integrity["integrity"], "invalid");
    assert_eq!(integrity["torn_tail"], true);
}

/// The fresh-epoch boundary with the receiver alive: the owned daemon is stopped and continued
/// (T23's seam), never killed or replaced. Returns the chain as it stood just after recovery.
fn stall_recovery(v: &Vertical, entry: &str, row: &mut Row) -> (f64, f64, Chain) {
    v.wait_for_echoes(20);
    let before = capture(v, entry);
    assert_eq!(before.capture, CaptureState::Recording);
    assert_eq!(before.gap_count, 0);
    let daemon = v.daemon_pid();
    let daemon_started = ps_started(daemon);
    let stopped = now();
    signal_owned_daemon(v, daemon, libc::SIGSTOP);
    std::thread::sleep(STALL);
    signal_owned_daemon(v, daemon, libc::SIGCONT);
    let continued = now();
    std::thread::sleep(Duration::from_secs(8));
    let after = capture(v, entry);
    row.note(
        "status_after_recovery",
        serde_json::to_value(&after).unwrap(),
    );
    assert_eq!(after.capture, CaptureState::Incomplete);
    assert!(after.gap_count >= 1);
    assert!(!after.assistance_observed);
    assert_eq!(
        (running(daemon), ps_started(daemon)),
        (true, daemon_started),
        "the receiver was the same live process throughout"
    );
    let chain = v.chain(entry);
    row.keep_bytes("chain-after-recovery.jsonl", &chain.bytes);
    row.note("chain_after_recovery_sha256", chain.sha256());
    (stopped, continued, chain)
}

#[test]
fn disconnect_reconnects_fresh_epoch_without_replay() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("stream", json!({}));
    let mut row = Row::new(
        "disconnect_reconnects_fresh_epoch_without_replay",
        &v,
        EVIDENCE,
    );
    let mut start = v.start(RESERVE);
    let entry = v.entry_id();
    let harness = v.harness_pid() as i32;
    let harness_started = ps_started(harness);
    let harness_command = ps_command(harness);
    let daemon = v.daemon_pid();
    let daemon_started = ps_started(daemon);

    let (stopped, continued, recovered) = stall_recovery(&v, &entry, &mut row);

    // The relay never waited on the recorder, measured by the workload.
    let events = v.events();
    assert!(events.iter().all(|e| e["ok"] == true), "an exchange failed");
    let stalled = events
        .iter()
        .filter(|e| {
            e["start"]
                .as_f64()
                .is_some_and(|t| t > stopped && t < continued)
        })
        .count();
    row.note("exchanges_during_stall", stalled);
    assert!(stalled > 50, "{stalled} exchanges during the stall");

    // A fresh epoch at a safe boundary: a Recovery names the lost producer, a different
    // producer's streams follow it, and the loss stays a gap.
    let lost = recovered.first_wire_producer();
    let fresh: Vec<String> = recovered
        .records()
        .filter_map(|r| match r {
            CaptureRecord::Recovery { producer, .. } => Some(producer.clone()),
            _ => None,
        })
        .collect();
    row.note("fresh_producers", fresh.clone());
    assert!(!fresh.is_empty(), "a fresh epoch was admitted");
    assert!(!fresh.contains(&lost));
    assert!(recovered.records().any(|r| matches!(
        r,
        CaptureRecord::Open { stream, source: CaptureSource::WireRequest, .. }
            if fresh.contains(&stream.producer)
    )));
    assert!(
        recovered
            .records()
            .any(|r| matches!(r, CaptureRecord::Gap { .. }))
    );
    wait_delivered(&v, continued, 10, 5, RUN);
    // Not a restart and not an agent resume: while it runs, the receiver and the harness are
    // the original processes, with the original command line.
    assert_eq!(
        (running(daemon), ps_started(daemon)),
        (true, daemon_started.clone())
    );
    assert_eq!(ps_started(harness), harness_started);
    assert_eq!(ps_command(harness), harness_command);

    v.release();
    let exit = start.wait(RUN);
    row.note("start_exit", exit.code());
    row.note("start_output", start.output());
    assert!(exit.success(), "{exit}\n{}", start.output());

    // Never replayed: the same daemon sealed it; the chain only appended to what stood after
    // recovery; no payload is retained twice; the upstream saw each request once.
    let chain = v.chain(&entry);
    row.keep_bytes("chain-sealed.jsonl", &chain.bytes);
    row.note("final_chain_sha256", chain.sha256());
    assert!(
        chain.bytes.starts_with(&recovered.bytes),
        "append-only across the finish: the gap is permanent"
    );
    assert!(gaps_of(&chain).len() >= gaps_of(&recovered).len());
    assert_eq!(
        max_request_multiplicity(&chain),
        1,
        "no request payload was replayed into the chain"
    );
    assert_upstream_saw_each_once(&v, &mut row);
    assert_eq!(v.report()["ok"], true);

    // Not a restart and not an agent resume: the receiver, harness and run are the originals.
    assert_eq!(
        (running(daemon), ps_started(daemon)),
        (true, daemon_started)
    );
    assert_eq!(
        v.harness_pid() as i32,
        harness,
        "the same harness, never relaunched"
    );
    assert_eq!(launches(&chain), 1);
    assert_eq!(run_directories(&v), 1);
    assert_eq!(assistance(&chain), 0);
    assert_eq!(local_finishes(&chain), 1);

    let sealed = capture(&v, &entry);
    row.note("sealed_status", serde_json::to_value(&sealed).unwrap());
    assert_eq!(sealed.run, RunState::Sealed);
    assert!(sealed.gap_count >= 1, "gap history outlives the finish");
    let (code, integrity) = v.verify(&entry);
    row.note("verifier_exit", code);
    row.note("verifier", integrity.clone());
    assert_eq!(code, 1, "{integrity}");
    assert_eq!(integrity["integrity"], "incomplete");
    assert_eq!(integrity["sealed"], true);
    assert_eq!(integrity["coverage_incomplete"], true);
    assert!(
        integrity["findings"]
            .as_array()
            .unwrap()
            .contains(&json!("producer_recovered")),
        "{integrity}"
    );
    assert_eq!(
        integrity["gap_count"].as_u64(),
        Some(sealed.gap_count),
        "the verifier and the status count the same loss"
    );
}

#[test]
fn capture_recovery_emits_no_assistance() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("stream", json!({}));
    let mut row = Row::new("capture_recovery_emits_no_assistance", &v, EVIDENCE);
    let mut start = v.start(RESERVE);
    let entry = v.entry_id();
    let harness = v.harness_pid() as i32;
    let harness_started = ps_started(harness);
    let harness_command = ps_command(harness);

    // Recovery one: a fresh epoch with the receiver alive.
    let (_stopped, _continued, recovered) = stall_recovery(&v, &entry, &mut row);
    assert!(recoveries(&recovered) >= 1, "a recorder recovery happened");
    assert_eq!(
        assistance(&recovered),
        0,
        "recorder recovery is not assistance"
    );
    assert!(!capture(&v, &entry).assistance_observed);

    // Recovery two: the receiver killed and a new one reopening the entry for status and seal.
    let crash = crash_and_restart(&v, &entry, &mut row, &mut start);
    assert!(
        crash.dead.bytes.starts_with(&recovered.bytes),
        "the first recovery's records survive the crash"
    );
    assert_eq!(assistance(&crash.after_restart), 0);
    assert_eq!(recoveries(&crash.after_restart), recoveries(&recovered));
    assert!(!crash.status.assistance_observed);
    assert_no_readmission(&v, &entry, &mut row, &crash);
    assert_eq!(ps_started(harness), harness_started);
    assert_eq!(ps_command(harness), harness_command);

    v.release();
    assert_launcher_finished_on_the_restarted_daemon(&mut start, &mut row);

    // No assistance, no relaunch, no harness input at either boundary or at the end: the
    // harness is the one process started once with its one command line, the launch is
    // recorded once, no assistance record exists and no status or finding claims one.
    let chain = v.chain(&entry);
    row.keep_bytes("chain-sealed.jsonl", &chain.bytes);
    row.note("final_chain_sha256", chain.sha256());
    assert!(chain.bytes.starts_with(&crash.after_restart.bytes));
    assert_eq!(assistance(&chain), 0);
    assert_eq!(launches(&chain), 1);
    assert_eq!(run_directories(&v), 1);
    assert_eq!(
        v.harness_pid() as i32,
        harness,
        "the same harness, never relaunched"
    );
    assert_eq!(local_finishes(&chain), 1);
    assert_eq!(v.report()["ok"], true);
    assert_eq!(max_request_multiplicity(&chain), 1, "nothing was replayed");
    assert_upstream_saw_each_once(&v, &mut row);
    let sealed = capture(&v, &entry);
    row.note("sealed_status", serde_json::to_value(&sealed).unwrap());
    assert_eq!(sealed.run, RunState::Sealed);
    assert!(!sealed.assistance_observed);
    let (code, integrity) = v.verify(&entry);
    row.note("verifier_exit", code);
    row.note("verifier", integrity.clone());
    assert_eq!(code, 1, "{integrity}");
    assert_eq!(integrity["integrity"], "incomplete");
    assert_eq!(integrity["sealed"], true);
    assert!(
        integrity["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| !f.as_str().unwrap_or("").contains("assist")),
        "{integrity}"
    );
}

// ------------------------------------------------------------------ H-10 4b: the spawn gate

// `goalcup start` forks its capture child and parks it before `exec` until the daemon has the
// child's registration durably (D40). The rows below hold the launcher at the gate's three
// points (`commands/start.rs::pause_at`, `test-support` builds only) and break one thing at
// each. The one fact every "never execs" row reads is the workload's own `out/pid`, which it
// writes as the first thing it does and which a parked child cannot have written.

/// A pause directory with `point` armed.
fn gate_pause(v: &Vertical, point: &str) -> std::path::PathBuf {
    let dir = v.root.join("pause");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{point}.arm")), b"").unwrap();
    dir
}

fn gate_reached(dir: &std::path::Path, point: &str) {
    let marker = dir.join(format!("{point}.reached"));
    wait_until(&format!("the launcher to reach {point}"), || {
        marker.exists()
    });
}

fn gate_go(dir: &std::path::Path, point: &str) {
    std::fs::write(dir.join(format!("{point}.go")), b"").unwrap();
}

/// The launcher's parked capture child: a fork of the launcher, still running the launcher's own
/// image, with the launcher's pid as its parent. Its start time is its identity.
fn parked_child(start: &Launcher, row: &mut Row) -> (i32, String) {
    let pid = child_of(start.pid(), "goalcup", "the parked capture child");
    let started = ps_started(pid);
    row.note("parked_pid", pid);
    row.note("parked_started", started.clone());
    row.note("parked_command", ps_command(pid));
    (pid, started)
}

/// Nothing the harness does has happened: its first act is its pid file.
fn assert_harness_never_ran(v: &Vertical, row: &mut Row) {
    // Long enough for a harness that was wrongly released to have written it.
    std::thread::sleep(Duration::from_secs(2));
    let ran = v.out.join("pid").exists();
    row.note("harness_pid_file_present", ran);
    assert!(!ran, "the harness exec'd and ran");
}

fn finding(integrity: &Value, name: &str) -> bool {
    integrity["findings"]
        .as_array()
        .unwrap()
        .contains(&json!(name))
}

/// A seal from a fresh `goalcup seal`, observed.
fn seal_observed(v: &Vertical, entry: &str, row: &mut Row, tag: &str) -> String {
    let (exit, out, took) = v.seal(entry);
    row.note(&format!("{tag}_exit"), exit.code());
    row.note(&format!("{tag}_output"), out.clone());
    row.note(&format!("{tag}_ms"), took.as_millis() as u64);
    assert!(exit.success(), "{exit}\n{out}");
    out
}

#[test]
fn capture_cli_killed_before_registration_never_execs() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("plain", json!({"hold": true}));
    let mut row = Row::new(
        "capture_cli_killed_before_registration_never_execs",
        &v,
        EVIDENCE,
    );
    let pause = gate_pause(&v, "capture-gate-forked");
    let mut start = v.start_paused(RESERVE, &pause);
    let entry = v.entry_id();
    gate_reached(&pause, "capture-gate-forked");
    let (parked, _) = parked_child(&start, &mut row);

    // The launcher dies with the child parked and nothing registered.
    start.kill9();
    wait_exited(parked, "the parked child to end when its launcher died");
    assert_harness_never_ran(&v, &mut row);
    assert_eq!(launches(&v.chain(&entry)), 0);

    // No registration reached the daemon, so a seal has no process to name: it commits an
    // honest incomplete trace and ends nothing.
    let out = seal_observed(&v, &entry, &mut row, "seal");
    assert!(!out.contains("Confirmed"), "{out}");
    let (code, integrity) = v.verify(&entry);
    row.note("verifier_exit", code);
    row.note("verifier", integrity.clone());
    assert_eq!(code, 1, "{integrity}");
    assert_eq!(integrity["sealed"], true);
    assert!(finding(&integrity, "launch_missing"), "{integrity}");
}

#[test]
fn capture_cli_killed_after_registration_before_release_never_execs() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("plain", json!({"hold": true}));
    let mut row = Row::new(
        "capture_cli_killed_after_registration_before_release_never_execs",
        &v,
        EVIDENCE,
    );
    let pause = gate_pause(&v, "capture-gate-registered");
    let mut start = v.start_paused(RESERVE, &pause);
    let entry = v.entry_id();
    gate_reached(&pause, "capture-gate-registered");
    let (parked, _) = parked_child(&start, &mut row);

    // Registered durably, never released, and then the launcher dies.
    start.kill9();
    wait_exited(parked, "the parked child to end when its launcher died");
    assert_harness_never_ran(&v, &mut row);

    // The row names a process that never exec'd and is gone: the seal observes that, signals
    // nothing, and never claims it ended anything.
    let out = seal_observed(&v, &entry, &mut row, "seal");
    assert!(out.contains("AlreadyStopped"), "{out}");
    assert!(!out.contains("Confirmed"), "{out}");
    let (code, integrity) = v.verify(&entry);
    row.note("verifier_exit", code);
    row.note("verifier", integrity.clone());
    assert_eq!(code, 1, "{integrity}");
    assert!(finding(&integrity, "launch_missing"), "{integrity}");
    assert_eq!(launches(&v.chain(&entry)), 0);
}

#[test]
fn capture_cli_killed_after_release_harness_is_owned() {
    let _lock = TableLock::acquire();
    // `stream` keeps going when its exchanges fail, so the harness outlives the launcher's
    // proxy; `plain` would end on its first refused connection and leave nothing to own.
    let v = Vertical::new("stream", json!({}));
    let mut row = Row::new(
        "capture_cli_killed_after_release_harness_is_owned",
        &v,
        EVIDENCE,
    );
    let pause = gate_pause(&v, "capture-gate-released");
    let mut start = v.start_paused(RESERVE, &pause);
    let entry = v.entry_id();
    gate_reached(&pause, "capture-gate-released");

    // Released and running; the launcher is held before it records the Launch.
    let harness = v.harness_pid() as i32;
    let harness_started = ps_started(harness);
    row.note("harness_pid", harness);
    row.note("harness_started", harness_started.clone());
    assert!(running(harness));
    start.kill9();
    assert!(
        running(harness) && ps_started(harness) == harness_started,
        "the launcher's death does not end the harness"
    );

    // Its exact instance was durably registered before it ran, so a seal ends it.
    let out = seal_observed(&v, &entry, &mut row, "seal");
    assert!(out.contains("Confirmed"), "{out}");
    wait_exited(harness, "the registered harness to be ended by the seal");
    let (code, integrity) = v.verify(&entry);
    row.note("verifier_exit", code);
    row.note("verifier", integrity.clone());
    assert_eq!(code, 1, "{integrity}");
    assert_eq!(integrity["sealed"], true);
    assert!(
        finding(&integrity, "launch_missing"),
        "the launcher died before it recorded the Launch: {integrity}"
    );
}

#[test]
fn capture_daemon_killed_during_gate_never_execs() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("plain", json!({"hold": true}));
    let mut row = Row::new(
        "capture_daemon_killed_during_gate_never_execs",
        &v,
        EVIDENCE,
    );
    let pause = gate_pause(&v, "capture-gate-forked");
    let mut start = v.start_paused(RESERVE, &pause);
    let entry = v.entry_id();
    gate_reached(&pause, "capture-gate-forked");
    let (parked, _) = parked_child(&start, &mut row);

    // The daemon dies before the registration is asked for. The launcher then retries it
    // against a dead daemon until the bounded retry gives up; the child stays parked and is
    // never released.
    let victim = v.daemon_pid();
    kill9_owned_daemon(&v, victim);
    row.note("killed_daemon_pid", victim);
    gate_go(&pause, "capture-gate-forked");
    let exit = start.wait(Duration::from_secs(120));
    row.note("start_exit", exit.code());
    row.note("start_output", start.output());
    assert!(!exit.success(), "{}", start.output());
    wait_exited(parked, "the never-released child to end with its launcher");
    assert_harness_never_ran(&v, &mut row);

    // A daemon that restarts over the registry finds no registration and no Launch: the trace
    // is incomplete for want of a launch, and the restart has no process to be lost.
    let restarted = Daemon::spawn(&v, "restart", None);
    row.note("restarted_daemon_pid", restarted.pid());
    let out = seal_observed(&v, &entry, &mut row, "seal");
    assert!(!out.contains("Confirmed"), "{out}");
    let chain = v.chain(&entry);
    assert_eq!(launches(&chain), 0);
    let gaps = gaps_of(&chain);
    row.note("gaps", format!("{gaps:?}"));
    assert!(
        !gaps
            .iter()
            .any(|(_, reason, _)| *reason == GapReason::IngestLost),
        "no registration was admitted, so there is no ingest the restart lost: {gaps:?}"
    );
    let (code, integrity) = v.verify(&entry);
    row.note("verifier_exit", code);
    row.note("verifier", integrity.clone());
    assert_eq!(code, 1, "{integrity}");
    assert!(finding(&integrity, "launch_missing"), "{integrity}");
}

#[test]
fn capture_daemon_killed_after_registered_harness_is_owned_across_restart() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("plain", json!({"hold": true}));
    let mut row = Row::new(
        "capture_daemon_killed_after_registered_harness_is_owned_across_restart",
        &v,
        EVIDENCE,
    );
    let pause = gate_pause(&v, "capture-gate-registered");
    let mut start = v.start_paused(RESERVE, &pause);
    let entry = v.entry_id();
    gate_reached(&pause, "capture-gate-registered");
    let (parked, parked_started) = parked_child(&start, &mut row);

    // The registration is durable and the child is still parked. The daemon dies and a second
    // one reopens the entry, which chains the unknown ingest interval of a registered run.
    let victim = v.daemon_pid();
    kill9_owned_daemon(&v, victim);
    row.note("killed_daemon_pid", victim);
    let restarted = Daemon::spawn(&v, "restart", None);
    row.note("restarted_daemon_pid", restarted.pid());
    wait_capture(&v, &entry, "the reopened entry", OBSERVE, |s| {
        s.gap_count >= 1
    });
    assert!(
        running(parked) && ps_started(parked) == parked_started,
        "the parked child outlived the daemon and is still parked"
    );
    assert!(!v.out.join("pid").exists());

    // Released after the restart: the registered instance is the harness that now runs.
    gate_go(&pause, "capture-gate-registered");
    let harness = v.harness_pid() as i32;
    assert_eq!(harness, parked, "the registered pid is the harness's pid");
    assert_eq!(ps_started(harness), parked_started, "and its instance");
    row.note("harness_pid", harness);

    // Only a fresh `goalcup seal` can reach the restarted daemon; the registered instance is
    // ended.
    let out = seal_observed(&v, &entry, &mut row, "seal");
    assert!(out.contains("Confirmed"), "{out}");
    wait_exited(harness, "the registered harness to be ended by the seal");
    let exit = start.wait(RUN);
    row.note("start_exit", exit.code());
    row.note("start_output", start.output());
    let chain = v.chain(&entry);
    let lost = gaps_of(&chain)
        .iter()
        .filter(|(_, reason, _)| *reason == GapReason::IngestLost)
        .count();
    row.note("ingest_lost_gaps", lost);
    assert_eq!(lost, 1, "the restart chained one IngestLost");
    assert_eq!(local_finishes(&chain), 1);
    let (code, integrity) = v.verify(&entry);
    row.note("verifier_exit", code);
    row.note("verifier", integrity.clone());
    assert_eq!(code, 1, "{integrity}");
    assert_eq!(integrity["sealed"], true);
}

/// #7: a Ctrl-C that reaches the launcher after the registration and before the release. It
/// is no longer swallowed: the parked child is ended without running, and the entry is sealed
/// with no Launch.
#[test]
fn capture_ctrl_c_after_registration_never_releases_the_harness() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("plain", json!({"hold": true}));
    let mut row = Row::new(
        "capture_ctrl_c_after_registration_never_releases_the_harness",
        &v,
        EVIDENCE,
    );
    let pause = gate_pause(&v, "capture-gate-registered");
    let mut start = v.start_paused(RESERVE, &pause);
    let entry = v.entry_id();
    gate_reached(&pause, "capture-gate-registered");

    // SAFETY: `kill` reads two integers; the pid is the launcher this test spawned.
    assert_eq!(unsafe { libc::kill(start.pid(), libc::SIGINT) }, 0);
    gate_go(&pause, "capture-gate-registered");
    let exit = start.wait(RUN);
    row.note("start_exit", exit.code());
    row.note("start_output", start.output());
    assert!(!exit.success(), "{}", start.output());
    assert!(
        start
            .output()
            .contains("interrupted before the harness started"),
        "{}",
        start.output()
    );
    assert_harness_never_ran(&v, &mut row);
    let chain = v.chain(&entry);
    assert_eq!(
        launches(&chain),
        0,
        "no Launch for a harness that never ran"
    );
    assert_eq!(local_finishes(&chain), 1, "the launcher sealed the entry");
}

#[test]
fn capture_gated_child_killed_before_registration_is_refused() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("plain", json!({"hold": true}));
    let mut row = Row::new(
        "capture_gated_child_killed_before_registration_is_refused",
        &v,
        EVIDENCE,
    );
    let pause = gate_pause(&v, "capture-gate-forked");
    let mut start = v.start_paused(RESERVE, &pause);
    let entry = v.entry_id();
    gate_reached(&pause, "capture-gate-forked");
    let (parked, parked_started) = parked_child(&start, &mut row);

    // The parked child dies on its own account while the launcher is held before it registers.
    kill9_owned_child(parked, "goalcup", &parked_started);
    wait_until("the killed child to die", || !running(parked));
    gate_go(&pause, "capture-gate-forked");
    let exit = start.wait(RUN);
    row.note("start_exit", exit.code());
    row.note("start_output", start.output());
    assert!(
        !exit.success(),
        "a launch whose harness never ran is not a successful run\n{}",
        start.output()
    );
    assert_harness_never_ran(&v, &mut row);

    // However far the registration got, no harness ran and no Launch was written; the entry is
    // sealed, honestly incomplete.
    let chain = v.chain(&entry);
    assert_eq!(
        launches(&chain),
        0,
        "no Launch for a harness that never ran"
    );
    assert_eq!(local_finishes(&chain), 1, "the launcher sealed the entry");
    let (code, integrity) = v.verify(&entry);
    row.note("verifier_exit", code);
    row.note("verifier", integrity.clone());
    assert_eq!(code, 1, "{integrity}");
    assert_eq!(integrity["sealed"], true);
    assert!(finding(&integrity, "launch_missing"), "{integrity}");
}

#[test]
fn seal_while_parked_signals_nothing_then_cli_ends_released_child() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("plain", json!({"hold": true}));
    let mut row = Row::new(
        "seal_while_parked_signals_nothing_then_cli_ends_released_child",
        &v,
        EVIDENCE,
    );
    let pause = gate_pause(&v, "capture-gate-registered");
    let mut start = v.start_paused(RESERVE, &pause);
    let entry = v.entry_id();
    gate_reached(&pause, "capture-gate-registered");
    let (parked, parked_started) = parked_child(&start, &mut row);

    // A seal lands between the registration and the release. The daemon's guard sees a process
    // whose command line is not the harness's, refuses to treat it as one and sends nothing.
    let out = seal_observed(&v, &entry, &mut row, "racing_seal");
    assert!(
        out.contains("Unresolved") && out.contains("ProcessAuthorityUnavailable"),
        "{out}"
    );
    assert!(
        running(parked) && ps_started(parked) == parked_started,
        "the daemon signalled nothing: the parked child is the same live process"
    );
    assert!(!v.out.join("pid").exists(), "and it has not run");

    // Released into a sealed entry: the launcher sees the seal right after the release and ends
    // the harness it just started, within the connector's grace (10 s) and a kill sweep.
    gate_go(&pause, "capture-gate-registered");
    let began = std::time::Instant::now();
    let exit = start.wait(Duration::from_secs(10 + 2 + 30));
    row.note("ending_ms", began.elapsed().as_millis() as u64);
    row.note("start_exit", exit.code());
    row.note("start_output", start.output());
    assert_eq!(exit.code(), Some(1), "{}", start.output());
    let printed = start.output();
    assert!(
        printed.contains("was sealed while its harness was being launched")
            && printed.contains("ended the harness it had just started")
            && printed.contains("The seal is the record; this ending is not in it.")
            && printed.contains(&format!("asked to stop: {parked}")),
        "{printed}"
    );
    wait_until("the released harness to be gone", || !running(parked));
    assert!(
        !running(parked) || ps_started(parked) != parked_started,
        "the released harness is gone"
    );

    // The seal is the record: one terminal, no Launch after it.
    let chain = v.chain(&entry);
    assert_eq!(local_finishes(&chain), 1);
    assert_eq!(launches(&chain), 0);
    assert!(matches!(
        chain.lines.last().unwrap().record,
        Some(CaptureRecord::LocalFinish { .. })
    ));
    let (code, integrity) = v.verify(&entry);
    row.note("verifier_exit", code);
    row.note("verifier", integrity.clone());
    assert_eq!(code, 1, "{integrity}");
    assert_eq!(integrity["sealed"], true);
    assert!(finding(&integrity, "launch_missing"), "{integrity}");
}

// ------------------------------------------------------------------ H-16: proxy loss in the stream

/// The launcher stops its proxy (`Close` queued, the harness gone) and is held there, before it
/// asks the daemon to seal. A standalone `goalcup seal` then commits the run with no proxy
/// counters (`proxy: None`), and the launcher is released into a sealed entry. Only the
/// producer's own stream can carry the fail-open tunnel the harness made; the verifier must
/// not read a clean pass.
#[test]
fn a_standalone_seal_that_commits_first_keeps_the_proxys_loss() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("fail-open", json!({}));
    let mut row = Row::new(
        "a_standalone_seal_that_commits_first_keeps_the_proxys_loss",
        &v,
        EVIDENCE,
    );
    let pause = gate_pause(&v, "capture-finish-proxy-stopped");
    let mut start = v.start_paused(RESERVE, &pause);
    let entry = v.entry_id();
    gate_reached(&pause, "capture-finish-proxy-stopped");

    // The harness has exited and the recording proxy has stopped; nothing has asked the daemon to
    // seal.
    let out = seal_observed(&v, &entry, &mut row, "standalone_seal");
    assert!(out.contains("finished locally"), "{out}");

    // The launcher's own request now meets a sealed row: it is answered from it before stage
    // zero (`runtime/capture/finish.rs`, the sealed-entry answer), so it exits 0 whatever its deadline
    // and its counters never reach the chain. A later reordering of that answer shows here.
    gate_go(&pause, "capture-finish-proxy-stopped");
    let exit = start.wait(RUN);
    row.note("start_exit", exit.code());
    row.note("start_output", start.output());
    assert!(exit.success(), "{}\n{}", exit, start.output());
    assert!(
        start.output().contains("fail_open=1"),
        "the recording proxy counted the tunnel: {}",
        start.output()
    );

    let chain = v.chain(&entry);
    row.keep_bytes("chain-sealed.jsonl", &chain.bytes);
    let producer = chain.first_wire_producer();
    row.note("first_wire_producer", producer.clone());
    assert_eq!(local_finishes(&chain), 1);

    // The verdict first: a run that lost a tunnel must not read as a clean pass.
    let (code, integrity) = v.verify(&entry);
    row.note("verifier_exit", code);
    row.note("verifier", integrity.clone());
    assert_eq!(code, 1, "{integrity}");
    assert_eq!(integrity["sealed"], true);
    assert_eq!(integrity["coverage_incomplete"], true, "{integrity}");
    assert!(finding(&integrity, "gap_recorded"), "{integrity}");

    // The loss is a stream-less gap on the producer's own clock, ahead of that producer's close.
    let mut gap_at = None;
    let mut closed_at = None;
    let mut daemon_clocked = 0;
    for (at, record) in chain.records().enumerate() {
        match record {
            CaptureRecord::Gap {
                stream: None,
                reason: GapReason::DecodeFailed,
                observed,
                ..
            } if observed.clock_id == producer => gap_at = gap_at.or(Some(at)),
            CaptureRecord::Gap {
                stream: None,
                reason: GapReason::DecodeFailed,
                ..
            } => daemon_clocked += 1,
            CaptureRecord::ProducerClosed { producer: p, .. } if *p == producer => {
                closed_at = Some(at)
            }
            _ => {}
        }
    }
    row.note("loss_gap_at", gap_at);
    row.note("producer_closed_at", closed_at);
    row.note("daemon_clocked_decode_failed_gaps", daemon_clocked);
    let (gap_at, closed_at) = (
        gap_at.expect("a producer-clocked stream-less DecodeFailed gap"),
        closed_at.expect("the producer closed cleanly"),
    );
    assert!(gap_at < closed_at, "the loss precedes the close");
    assert_eq!(
        daemon_clocked, 0,
        "the launcher's counters provably never arrived"
    );

    let lane_done = v
        .events()
        .iter()
        .any(|e| e["lane"] == "fail_open" && e["ok"] == true);
    row.note("fail_open_lane_completed", lane_done);
    assert!(lane_done, "the workload's tunnel completed");
}
