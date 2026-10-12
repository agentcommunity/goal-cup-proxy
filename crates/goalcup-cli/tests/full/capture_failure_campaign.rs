//! The Phase 1 storage and accounting fault campaign, Task 2 slice of T25: four rows against the
//! real `goalcup`, `goalcup-daemon`, in-process proxy and `goalcup-verify`, on the synthetic
//! workload of `m2_synthetic_vertical`. A control, a refused reservation, an exhausted
//! reservation and a refused metadata append.
//!
//! The faults are the production boundaries and nothing else:
//!
//! - the reservation is `goalcup start --capture-reserve-bytes`: an impossible size is refused
//!   by the platform allocator before the goal, the Launch or the harness exist;
//! - exhaustion is logical: a small reserve is spent by the `stream` workload, whose traffic
//!   keeps flowing through the relay; no filesystem is filled;
//! - the metadata fault is a second hard link to the live `chain.jsonl`, made outside the trace
//!   directory and removed once the refusal has been observed. The append capability refuses a
//!   file with two names at its next validation, before it writes.
//!
//! The unit and protocol suites own the byte arithmetic (`capture_reserved_storage.rs`,
//! `capture_terminals.rs`, the daemon's `capacity_exhaustion_uses_bounded_terminal_records`);
//! nothing here restates them. These rows assert only what crosses the process boundary: launch
//! or no launch, later traffic delivered after storage loss, bounded retained records, the signed
//! status, the CLI's outcome and the verifier's literal exit code. Literal `ENOSPC` on a mounted
//! volume, power loss, other filesystems and RSS are not measured here.
//!
//! macOS and Linux (the trace folder must be on APFS, ext4 or XFS), ignored by default because
//! each row runs real processes for seconds. Build the binaries first, then run by name and serially:
//!
//! ```text
//! cargo build --offline
//! cargo test -p goalcup-cli --offline --features test-support --test full capture_failure_campaign:: -- --ignored --test-threads=1
//! ```

// Shared with the crash campaign; each campaign uses a subset.
use crate::common::capture_campaign_support;
use crate::common::capture_failure_support;
use crate::common::m2_vertical_support;

use crate::common::table_lock;

use capture_campaign_support::{
    OBSERVE, RUN, Row, capture, delivered, ps_started, running, signal_owned_daemon, wait_capture,
    wait_delivered,
};
use capture_failure_support::{Alias, bytes_through, line_at};
use goalcup_protocol::capture::{
    CaptureRecord, CaptureState, DeliveryState, GapReason, MAX_ENVELOPE_BYTES, RunState,
    TERMINAL_RESERVE_BYTES,
};
use m2_vertical_support::{GOAL, RESERVE, Vertical, now, wait_until};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use table_lock::TableLock;

/// Where this campaign's rows keep their evidence, under `target/`.
const EVIDENCE: &str = "phase1-task2-evidence";

/// The small reserve of the exhaustion row: two MiB, of which one MiB is the protected terminal
/// allowance. Qualified from the observed baseline, not assumed: the chain through the Launch
/// record is a few KiB (see `BASELINE_LIMIT`), so about 1 MiB of payload room is spent by the
/// `stream` workload's roughly 0.5 MiB/s within a few seconds, and nothing close to the host's
/// free space is ever allocated.
const SMALL_RESERVE: u64 = 2 * 1024 * 1024;

/// The most the pre-traffic chain (Baseline, Goal stream, Launch) may weigh for `SMALL_RESERVE`
/// to leave the payload room the row means to spend on real wire traffic.
const BASELINE_LIMIT: u64 = (SMALL_RESERVE - TERMINAL_RESERVE_BYTES) / 8;

/// The metadata row's reserve: far more than the row's few seconds of traffic can reach.
const HEADROOM: u64 = 16 * 1024 * 1024;

fn assert_verifier_incomplete(v: &Vertical, entry: &str) -> Value {
    let (code, integrity) = v.verify(entry);
    assert_eq!(code, 1, "{integrity}");
    assert_eq!(integrity["integrity"], "incomplete", "{integrity}");
    assert_eq!(integrity["sealed"], true, "{integrity}");
    assert_eq!(integrity["coverage_incomplete"], true, "{integrity}");
    integrity
}

#[test]
#[ignore = "real processes and sockets; run by name with --ignored"]
fn campaign_control_local_capture_finishes_and_verifies() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("plain", json!({"hold": true}));
    let mut row = Row::new(
        "campaign_control_local_capture_finishes_and_verifies",
        &v,
        EVIDENCE,
    );
    let mut start = v.start(RESERVE);
    let entry = v.entry_id();
    v.wait_for("holding");

    // While the workload holds: the physical reservation reported measured, recording,
    // local-only, with a durable frontier and no loss.
    let live = capture(&v, &entry);
    row.note("live_status", serde_json::to_value(&live).unwrap());
    assert_eq!(live.run, RunState::Running);
    assert_eq!(live.capture, CaptureState::Recording);
    assert_eq!(live.delivery, DeliveryState::LocalOnly);
    assert_eq!(live.storage.quota_bytes, RESERVE);
    assert_eq!(
        live.storage.reserved_bytes, RESERVE,
        "measured physical reservation"
    );
    assert!(live.storage.accounting_available);
    assert_eq!(live.gap_count, 0);
    assert!(live.storage.durable.is_some(), "{live:?}");

    v.release();
    let exit = start.wait(RUN);
    row.note("start_exit", exit.code());
    row.note("start_output", start.output());
    assert!(exit.success(), "{exit}\n{}", start.output());
    assert!(start.output().contains("local only"), "{}", start.output());
    assert_eq!(v.upload_dials(), 0, "no service was dialled");

    let chain = v.chain(&entry);
    row.note("chain_lines", chain.lines.len());
    row.note("chain_bytes", chain.bytes.len());
    row.note("chain_sha256", chain.sha256());
    assert_eq!(
        chain.count(|r| matches!(r, CaptureRecord::LocalFinish { .. })),
        1
    );
    assert!(matches!(
        chain.lines.last().unwrap().record,
        Some(CaptureRecord::LocalFinish { .. })
    ));
    assert_eq!(chain.count(|r| matches!(r, CaptureRecord::Gap { .. })), 0);
    assert_eq!(
        chain.count(|r| matches!(r, CaptureRecord::Launch { .. })),
        1
    );
    assert!(chain.lines.iter().all(|l| l.protocol_version == 5));
    assert_eq!(
        v.report()["goal"]["sha256"],
        goalcup_daemon::workspace::sha256_hex(GOAL.as_bytes())
    );

    let sealed = capture(&v, &entry);
    row.note("sealed_status", serde_json::to_value(&sealed).unwrap());
    assert_eq!(sealed.run, RunState::Sealed);
    assert_eq!(sealed.delivery, DeliveryState::LocalOnly);
    assert_eq!(sealed.gap_count, 0);
    assert!(sealed.storage.accounting_available);

    let (code, integrity) = v.verify(&entry);
    row.note("verifier_exit", code);
    row.note("verifier", integrity.clone());
    assert_eq!(code, 0, "{integrity}");
    assert_eq!(integrity["integrity"], "valid");
    assert_eq!(integrity["anchor"], "local_only");
    assert_eq!(integrity["sealed"], true);
    assert_eq!(integrity["terminal"]["kind"], "local_finish");
    assert_eq!(integrity["findings"], json!([]));
}

#[test]
#[ignore = "real processes and sockets; run by name with --ignored"]
fn reservation_failure_refuses_before_goal_launch() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("plain", json!({}));
    let mut row = Row::new(
        "reservation_failure_refuses_before_goal_launch",
        &v,
        EVIDENCE,
    );
    // A reservation no volume can hold: refused by the platform allocator, so no goal, no
    // Launch and no harness. The allocator's own byte accounting is `capture_reserved_storage`'s.
    let mut start = v.start(1 << 60);
    let exit = start.wait(RUN);
    row.note("start_exit", exit.code());
    row.note("start_output", start.output());
    assert_eq!(exit.code(), Some(1), "{}", start.output());
    assert!(
        start.output().contains("cannot reserve"),
        "{}",
        start.output()
    );

    // Nothing ran and nothing was left behind.
    assert!(!v.out.join("pid").exists(), "the workload never ran");
    assert_eq!(v.run_dir(), None, "no run directory");
    let entries = std::fs::read_dir(v.cfg.join("entries")).map_or(0, |d| d.count());
    assert_eq!(entries, 0, "no entry record");
    let left: Vec<_> = m2_vertical_support::files_under(&v.artifacts.join("artifacts"))
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    row.note("artifact_files_left", left.len());
    assert!(left.is_empty(), "no trace was left: {left:?}");
    assert_eq!(v.upload_dials(), 0);
}

#[test]
#[ignore = "real processes and sockets; run by name with --ignored"]
fn chain_budget_exhaustion_preserves_bounded_terminal_gap() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("stream", json!({}));
    let mut row = Row::new(
        "chain_budget_exhaustion_preserves_bounded_terminal_gap",
        &v,
        EVIDENCE,
    );
    let mut start = v.start(SMALL_RESERVE);
    let entry = v.entry_id();
    let room = SMALL_RESERVE - TERMINAL_RESERVE_BYTES;

    // Exhaustion arrives by itself: the first loss the daemon reports.
    let lost = wait_capture(&v, &entry, "storage loss", RUN, |s| s.gap_count >= 1);
    let lost_at = now();
    row.note("first_loss_status", serde_json::to_value(&lost).unwrap());
    assert_eq!(lost.storage.quota_bytes, SMALL_RESERVE);
    assert_eq!(lost.storage.reserved_bytes, SMALL_RESERVE);
    assert_eq!(lost.capture, CaptureState::Incomplete);
    assert!(
        lost.storage.accounting_available,
        "a refused payload is an exact, accounted loss"
    );

    // The reserve was qualified: a small baseline, then real wire traffic filled the room.
    let chain = v.chain(&entry);
    let baseline = bytes_through(&chain, |r| matches!(r, CaptureRecord::Launch { .. }));
    row.note("baseline_bytes_through_launch", baseline);
    assert!(
        baseline > 0 && baseline < BASELINE_LIMIT,
        "baseline {baseline} against limit {BASELINE_LIMIT}"
    );

    // The relay keeps delivering after the storage loss: HTTP and WebSocket exchanges that
    // began after the loss was first seen, byte for byte.
    let before_loss = delivered(&v, 0.0);
    wait_delivered(&v, lost_at, 10, 5, RUN);
    row.note("delivered_before_loss_seen", format!("{before_loss:?}"));

    // Settle: every stream open at exhaustion has been closed out or reported once.
    // A fixed settle: if it were too short the row could only fail (a later gap between the two
    // samples), never pass falsely.
    std::thread::sleep(Duration::from_secs(3));
    let first = capture(&v, &entry);
    let first_chain = v.chain(&entry);
    let first_delivered = delivered(&v, 0.0);
    // And more traffic after that, with the bounded state not moving.
    let settled_at = now();
    wait_delivered(&v, settled_at, 10, 5, RUN);
    std::thread::sleep(Duration::from_secs(3));
    let second = capture(&v, &entry);
    let second_chain = v.chain(&entry);
    let second_delivered = delivered(&v, 0.0);
    row.note("settled_status", serde_json::to_value(&first).unwrap());
    row.note("later_status", serde_json::to_value(&second).unwrap());
    row.note("settled_chain_bytes", first_chain.bytes.len());
    row.note("later_chain_bytes", second_chain.bytes.len());
    row.note(
        "settled_gap_records",
        first_chain.count(|r| matches!(r, CaptureRecord::Gap { .. })),
    );
    row.note("delivered_settled", format!("{first_delivered:?}"));
    row.note("delivered_later", format!("{second_delivered:?}"));
    assert!(
        second_delivered.http >= first_delivered.http + 10
            && second_delivered.ws >= first_delivered.ws + 5,
        "traffic continued: {first_delivered:?} then {second_delivered:?}"
    );
    assert_eq!(
        second_delivered.failed, 0,
        "no exchange failed through the loss"
    );

    // Sticky, bounded, and not growing: the loss is chained once for the entry and once per
    // stream open at exhaustion, never once per discarded frame.
    for status in [&first, &second] {
        assert_eq!(
            status.capture,
            CaptureState::Incomplete,
            "sticky: {status:?}"
        );
        assert!(status.storage.accounting_available);
        assert!(status.storage.used_bytes <= status.storage.quota_bytes);
        assert!(
            status.storage.used_bytes + MAX_ENVELOPE_BYTES as u64 >= room,
            "exhaustion was real, not an earlier failure: {status:?}"
        );
    }
    assert_eq!(
        second.gap_count, first.gap_count,
        "the gap count stopped growing"
    );
    assert_eq!(second.storage.used_bytes, first.storage.used_bytes);
    assert_eq!(
        second_chain.bytes.len(),
        first_chain.bytes.len(),
        "the retained chain stopped growing while traffic continued"
    );
    let gaps: Vec<_> = second_chain
        .records()
        .filter_map(|r| match r {
            CaptureRecord::Gap { stream, reason, .. } => Some((stream.clone(), *reason)),
            _ => None,
        })
        .collect();
    assert!(
        gaps.iter()
            .all(|(_, reason)| *reason == GapReason::StorageFull),
        "{gaps:?}"
    );
    assert_eq!(
        gaps.iter().filter(|(s, _)| s.is_none()).count(),
        1,
        "one entry gap"
    );
    let streams = second_chain.streams();
    assert!(
        streams.values().all(|s| s.gaps.len() <= 1),
        "at most one gap per stream"
    );
    assert_eq!(second.gap_count as usize, gaps.len());
    assert!(
        gaps.len() <= 1 + streams.len(),
        "{} gaps against {} streams",
        gaps.len(),
        streams.len()
    );

    // The finish: the terminal is written after exhaustion, inside the quota, and is last. This
    // run's gaps and terminal fit in payload room that was left over (the sealed chain is under
    // `room`), so it does not show the protected 1 MiB allowance being spent; the protocol and
    // daemon suites own that arithmetic.
    v.release();
    let exit = start.wait(RUN);
    row.note("start_exit", exit.code());
    row.note("start_output", start.output());
    let events = delivered(&v, 0.0);
    row.note("delivered_total", format!("{events:?}"));
    assert_eq!(events.failed, 0, "every exchange passed, the lost ones too");
    assert_eq!(v.report()["ok"], true);

    let chain = v.chain(&entry);
    row.note("final_chain_bytes", chain.bytes.len());
    row.note("final_chain_lines", chain.lines.len());
    row.note("final_chain_sha256", chain.sha256());
    assert!(
        chain.bytes.len() as u64 <= SMALL_RESERVE,
        "{} bytes in a {SMALL_RESERVE}-byte reservation",
        chain.bytes.len()
    );
    assert!(matches!(
        chain.lines.last().unwrap().record,
        Some(CaptureRecord::LocalFinish { .. })
    ));
    assert_eq!(
        chain.count(|r| matches!(r, CaptureRecord::LocalFinish { .. })),
        1,
        "one terminal, last"
    );
    assert_eq!(
        chain.count(|r| matches!(r, CaptureRecord::Assistance { .. })),
        0
    );
    assert_eq!(
        chain.count(|r| matches!(r, CaptureRecord::Recovery { .. })),
        0
    );
    assert_eq!(
        chain.count(|r| matches!(r, CaptureRecord::Launch { .. })),
        1
    );
    let final_gaps = chain.count(|r| matches!(r, CaptureRecord::Gap { .. }));
    assert_eq!(final_gaps, gaps.len(), "the terminal added no loss records");

    let integrity = assert_verifier_incomplete(&v, &entry);
    row.note("verifier_exit", 1);
    row.note("verifier", integrity.clone());
    assert_eq!(integrity["terminal"]["kind"], "local_finish");
    assert_eq!(
        integrity["terminal"]["seq"].as_u64(),
        Some(chain.lines.last().unwrap().seq)
    );
    assert_eq!(integrity["gap_count"].as_u64(), Some(gaps.len() as u64));
    let sealed = capture(&v, &entry);
    row.note("sealed_status", serde_json::to_value(&sealed).unwrap());
    assert_eq!(sealed.run, RunState::Sealed);
    assert_eq!(
        sealed.gap_count as usize,
        gaps.len(),
        "gap history outlives the finish"
    );
    assert!(sealed.storage.used_bytes <= sealed.storage.quota_bytes);
    assert!(exit.success(), "{exit}\n{}", start.output());
}

#[test]
#[ignore = "real processes and sockets; run by name with --ignored"]
fn metadata_write_failure_reports_accounting_unavailable() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("stream", json!({}));
    let mut row = Row::new(
        "metadata_write_failure_reports_accounting_unavailable",
        &v,
        EVIDENCE,
    );
    let mut start = v.start(HEADROOM);
    let entry = v.entry_id();
    let chain_path = v.trace(&entry).join("chain.jsonl");

    // A healthy capture with a known durable frontier, and traffic flowing.
    wait_delivered(&v, 0.0, 10, 5, RUN);
    let healthy = wait_capture(&v, &entry, "a durable frontier", OBSERVE, |s| {
        s.storage.durable.as_ref().is_some_and(|d| d.seq >= 8)
    });
    row.note("healthy_status", serde_json::to_value(&healthy).unwrap());
    assert_eq!(healthy.capture, CaptureState::Recording);
    assert!(healthy.storage.accounting_available);
    assert_eq!(healthy.gap_count, 0);
    let frontier_before = healthy.storage.durable.clone().unwrap();

    // The real append boundary: the chain file gains a second name. The capability's next
    // validation refuses it before writing.
    let mut alias = Alias::create(&chain_path, &v.root);
    row.note("alias_links", alias.links());
    assert_eq!(alias.links(), 2);
    let aliased_at = now();
    let failed = wait_capture(&v, &entry, "the append refusal", OBSERVE, |s| {
        !s.storage.accounting_available
    });
    let at_failure = v.chain(&entry);
    row.note("failure_status", serde_json::to_value(&failed).unwrap());
    row.note("chain_bytes_at_failure", at_failure.bytes.len());
    row.note("chain_sha256_at_failure", at_failure.sha256());
    // Not exact loss, not healthy: the writer failed, the byte count is unknown.
    assert_eq!(failed.capture, CaptureState::Failed, "{failed:?}");
    assert_eq!(failed.run, RunState::Running);
    assert!(!failed.storage.accounting_available);

    // Only now is the alias removed. The writer stays poisoned, so nothing is retried.
    alias.remove();
    assert_eq!(alias.links(), 0);
    wait_delivered(&v, aliased_at, 5, 3, RUN);
    std::thread::sleep(Duration::from_secs(4));
    let later = capture(&v, &entry);
    let later_chain = v.chain(&entry);
    row.note("later_status", serde_json::to_value(&later).unwrap());
    row.note("chain_bytes_later", later_chain.bytes.len());
    assert!(
        delivered(&v, aliased_at).http >= 5,
        "the relay kept delivering after the metadata fault"
    );

    // No later append: the chain is byte-identical after the alias is gone and traffic ran.
    assert_eq!(
        later_chain.sha256(),
        at_failure.sha256(),
        "nothing appended after the fault"
    );
    // No false durable advance: the frontier is the one the daemon reported, it names a real
    // chain line with the hash and byte count it claims, and it did not move past the fault.
    let frontier = failed.storage.durable.clone().expect("a frontier");
    assert_eq!(
        later.storage.durable, failed.storage.durable,
        "the frontier is frozen"
    );
    assert!(frontier.seq >= frontier_before.seq);
    assert!(frontier.chain_bytes >= frontier_before.chain_bytes);
    let (hash, through) = line_at(&later_chain, frontier.seq).expect("the frontier's line");
    assert_eq!(
        hash, frontier.hash,
        "the frontier names the chain's own link"
    );
    assert_eq!(through, frontier.chain_bytes, "and the bytes through it");
    assert!(frontier.chain_bytes <= later_chain.bytes.len() as u64);
    row.note(
        "appended_not_durable_bytes",
        later_chain.bytes.len() as u64 - frontier.chain_bytes,
    );
    // Unknown stays unknown: still failed, accounting still unavailable, the byte count and
    // gap history untouched, and no loss quantity was ever chained for it.
    assert_eq!(later.capture, CaptureState::Failed);
    assert!(!later.storage.accounting_available);
    assert_eq!(later.storage.used_bytes, failed.storage.used_bytes);
    assert_eq!(later.gap_count, failed.gap_count);
    assert!(
        later_chain.records().all(|r| !matches!(
            r,
            CaptureRecord::Gap {
                lost_bytes: Some(_),
                ..
            }
        )),
        "no exact loss was manufactured"
    );
    assert_eq!(
        later_chain.count(|r| matches!(r, CaptureRecord::Gap { .. })) as u64,
        later.gap_count
    );

    // The finish cannot append its terminal: no LocalFinish, no seal, not Sealed.
    v.release();
    let exit = start.wait(RUN);
    row.note("start_exit", exit.code());
    row.note("start_output", start.output());
    let final_chain = v.chain(&entry);
    row.note("final_chain_sha256", final_chain.sha256());
    assert_eq!(
        final_chain.sha256(),
        later_chain.sha256(),
        "the finish appended nothing"
    );
    assert_eq!(
        final_chain.count(|r| matches!(r, CaptureRecord::LocalFinish { .. })),
        0
    );
    assert!(
        !v.trace(&entry).join("seal.json").exists(),
        "no seal document"
    );
    let ended = capture(&v, &entry);
    row.note("final_status", serde_json::to_value(&ended).unwrap());
    assert_ne!(ended.run, RunState::Sealed);
    assert_eq!(ended.capture, CaptureState::Failed);
    assert!(!ended.storage.accounting_available);
    assert_eq!(ended.gap_count, failed.gap_count);
    assert_eq!(ended.storage.durable, failed.storage.durable);
    // `goalcup start` exits per the CLI ladder (`goalcup-cli/src/lib.rs`, "The exit ladder"):
    // 0 means the command did what it was asked. `main.rs` drops the harness's own exit, and an
    // unsealable finish is a reported outcome (`SealWriteFailed`, `seal.rs`), not an error.
    // Capture failure is carried by the status (`Failed`, asserted above) and by the verifier's
    // exit, asserted below (CONTRACTS.md: the 2026-09-29 local-finish amendment).
    assert_eq!(exit.code(), Some(0), "{}", start.output());
    // The CLI has one wording for `SealWriteFailed` (`seal.rs`, `finish_locally`), used whether
    // the terminal append or seal.json failed. Pinned only on the parts that matter here: no
    // terminal was written and the trace reads incomplete.
    let printed = start.output();
    assert!(
        printed.contains("no terminal record was written") && printed.contains("reads incomplete"),
        "{printed}"
    );
    let (code, integrity) = v.verify(&entry);
    row.note("verifier_exit", code);
    row.note("verifier", integrity.clone());
    assert_eq!(code, 1, "{integrity}");
    assert_ne!(integrity["integrity"], "valid", "{integrity}");
    assert_ne!(integrity["sealed"], true, "{integrity}");
}

// ------------------------------------------------------------------ H-10 4a: one finish budget

/// A pause directory for `goalcup start`'s test-support points, with `point` armed.
fn armed(v: &Vertical, point: &str) -> std::path::PathBuf {
    let dir = v.root.join("pause");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{point}.arm")), b"").unwrap();
    dir
}

/// Wait, bounded, for the launcher to reach `point`; the instant it did, on this test's clock.
fn reached(dir: &std::path::Path, point: &str) -> Instant {
    let marker = dir.join(format!("{point}.reached"));
    wait_until(&format!("the launcher to reach {point}"), || {
        marker.exists()
    });
    Instant::now()
}

fn go(dir: &std::path::Path, point: &str) {
    std::fs::write(dir.join(format!("{point}.go")), b"").unwrap();
}

/// Ends the test's own descendant when the test ends, however it ends: only while the pid still
/// has the start time the test recorded, so a reused pid is never signalled.
struct OwnedProcess {
    pid: i32,
    started: String,
}

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        if running(self.pid) && ps_started(self.pid) == self.started {
            // SAFETY: two integers; the pid was just seen with the start time this test recorded.
            unsafe { libc::kill(self.pid, libc::SIGKILL) };
        }
    }
}

fn count_in(chain: &m2_vertical_support::Chain, pick: impl Fn(&CaptureRecord) -> bool) -> usize {
    chain.count(pick)
}

/// The plan's `local_finish_under_backlog_has_one_deadline`, as a declared substitution
/// (CONTRACTS.md, H-10 4a ¶10): a whole-daemon SIGSTOP, not a held owner append or a delayed
/// blocking permit (those two holds are observed by the daemon's `capture_finish_budget`
/// test-hook rows only). The real parts are the sustained `stream` backlog, the real binaries,
/// and a real descendant that forked from the harness and holds its stdout (on Linux, until the
/// launcher's sweep ends it, before the finish).
///
/// The one observer is this test, on its own clock: the launcher is held at its
/// `capture-finish-started` point (before its deadline is taken), the daemon is frozen, the
/// launcher is released, and every later fact is stamped here.
#[test]
#[ignore = "real processes and sockets; run by name with --ignored"]
fn local_finish_under_backlog_has_one_deadline() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("stream", json!({"descendant": true}));
    let mut row = Row::new("local_finish_under_backlog_has_one_deadline", &v, EVIDENCE);
    let pause = armed(&v, "capture-finish-started");
    let mut start = v.start_paused(RESERVE, &pause);
    let entry = v.entry_id();
    v.wait_for("pid");
    v.wait_for_echoes(5);
    v.wait_for("descendant.pid");
    let descendant: i32 = std::fs::read_to_string(v.out.join("descendant.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let descendant_started = ps_started(descendant);
    row.note("descendant_pid", descendant);
    row.note("descendant_started", descendant_started.clone());
    let _owned = OwnedProcess {
        pid: descendant,
        started: descendant_started.clone(),
    };
    assert!(
        running(descendant),
        "a real descendant holds the harness's stdout"
    );

    // End the stream: the harness exits, and the launcher is held just before it takes its
    // deadline. Freeze the daemon, then let the launcher finish.
    v.release();
    let finish_started = reached(&pause, "capture-finish-started");
    let daemon = v.daemon_pid();
    signal_owned_daemon(&v, daemon, libc::SIGSTOP);
    go(&pause, "capture-finish-started");
    let frozen_at = Instant::now();

    let exit = start.wait(Duration::from_secs(45));
    let returned = finish_started.elapsed();
    row.note(
        "launcher_return_after_finish_start_ms",
        returned.as_millis() as u64,
    );
    row.note("start_exit", exit.code());
    row.note("start_output", start.output());
    // The one deadline: 10 s of budget, the launcher's termination allowance (its root is
    // reaped), the connect slack and one second of scheduler tolerance.
    let ceiling = Duration::from_millis(10_000 + 4_250 + 2_000 + 1_000);
    assert!(
        returned <= ceiling,
        "returned after {returned:?}, ceiling {ceiling:?}"
    );
    assert_eq!(exit.code(), Some(1), "{}", start.output());
    assert!(
        start
            .output()
            .contains("did not answer the finish of entry"),
        "{}",
        start.output()
    );
    let chain_at_return = v.chain(&entry);
    assert_eq!(
        count_in(&chain_at_return, |r| matches!(
            r,
            CaptureRecord::LocalFinish { .. }
        )),
        0,
        "a frozen daemon wrote nothing"
    );

    // macOS: the descendant is outside the owned set, still the same live process and never
    // signalled. Linux: the launcher is a child subreaper and swept it before the finish.
    if cfg!(target_os = "linux") {
        assert!(!running(descendant), "{}", start.output());
        assert!(
            start
                .output()
                .contains("swept 1 process(es) the harness left behind after it exited."),
            "{}",
            start.output()
        );
    } else {
        assert!(running(descendant));
        assert_eq!(ps_started(descendant), descendant_started);
    }

    // Hold the daemon frozen for 20 s in all, then let it go and watch what it does alone.
    if let Some(rest) = Duration::from_secs(20).checked_sub(frozen_at.elapsed()) {
        std::thread::sleep(rest);
    }
    signal_owned_daemon(&v, daemon, libc::SIGCONT);
    let thawed = Instant::now();
    std::thread::sleep(Duration::from_secs(12));
    let alone = v.chain(&entry);
    let committed_alone = count_in(&alone, |r| matches!(r, CaptureRecord::LocalFinish { .. }));
    row.note(
        "local_finish_committed_by_the_daemon_alone",
        committed_alone,
    );
    row.note(
        "observed_after_thaw_ms",
        thawed.elapsed().as_millis() as u64,
    );
    assert!(
        committed_alone <= 1,
        "at most one LocalFinish: {committed_alone}"
    );

    // A fresh `goalcup seal` answers the committed finish unchanged, or commits it now; either
    // way there is exactly one LocalFinish, nothing after it, and a second seal is identical.
    let (first, first_out, _) = v.seal(&entry);
    row.note("fresh_seal_output", first_out.clone());
    assert!(first.success(), "{first_out}");
    let sealed = v.chain(&entry);
    assert_eq!(
        count_in(&sealed, |r| matches!(r, CaptureRecord::LocalFinish { .. })),
        1
    );
    assert!(matches!(
        sealed.lines.last().unwrap().record,
        Some(CaptureRecord::LocalFinish { .. })
    ));
    let (second, second_out, _) = v.seal(&entry);
    assert!(second.success(), "{second_out}");
    assert_eq!(first_out, second_out, "the same report, byte for byte");
    assert_eq!(
        v.chain(&entry).sha256(),
        sealed.sha256(),
        "the second seal wrote nothing"
    );
    assert!(
        first_out.contains("AlreadyStopped"),
        "the root was reaped before any signal; nothing was ended: {first_out}"
    );
    assert!(!first_out.contains("Confirmed"), "{first_out}");
    if !cfg!(target_os = "linux") {
        assert!(
            running(descendant),
            "a seal never signals an orphaned descendant"
        );
        assert_eq!(ps_started(descendant), descendant_started);
    }

    let (code, integrity) = v.verify(&entry);
    row.note("verifier_exit", code);
    row.note("verifier", integrity.clone());
    assert!(code == 0 || code == 1, "{integrity}");
    assert_eq!(integrity["sealed"], true, "{integrity}");
    assert_eq!(v.upload_dials(), 0);

    // The test owns this descendant and ends it.
    drop(_owned);
    wait_until("the test's descendant to end", || !running(descendant));
}

/// The same finish without the freeze, under the `stream` backlog with a tight reserve: the
/// launcher returns by the one budget, the entry is sealed with a recorded gap, and the verifier
/// reads it as incomplete (exit 1).
#[test]
#[ignore = "real processes and sockets; run by name with --ignored"]
fn local_finish_under_stream_backlog_returns_by_the_deadline() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("stream", json!({}));
    let mut row = Row::new(
        "local_finish_under_stream_backlog_returns_by_the_deadline",
        &v,
        EVIDENCE,
    );
    let pause = armed(&v, "capture-finish-started");
    let mut start = v.start_paused(SMALL_RESERVE, &pause);
    let entry = v.entry_id();
    wait_capture(&v, &entry, "storage loss", RUN, |s| s.gap_count >= 1);

    v.release();
    let finish_started = reached(&pause, "capture-finish-started");
    go(&pause, "capture-finish-started");
    let exit = start.wait(RUN);
    let returned = finish_started.elapsed();
    row.note(
        "launcher_return_after_finish_start_ms",
        returned.as_millis() as u64,
    );
    row.note("start_exit", exit.code());
    row.note("start_output", start.output());
    assert!(
        returned <= Duration::from_secs(10) + Duration::from_secs(1),
        "returned after {returned:?}"
    );
    assert!(exit.success(), "{}", start.output());

    let chain = v.chain(&entry);
    assert_eq!(
        count_in(&chain, |r| matches!(r, CaptureRecord::LocalFinish { .. })),
        1
    );
    assert!(count_in(&chain, |r| matches!(r, CaptureRecord::Gap { .. })) >= 1);
    let sealed = capture(&v, &entry);
    assert_eq!(sealed.run, RunState::Sealed);
    let integrity = assert_verifier_incomplete(&v, &entry);
    row.note("verifier", integrity);
    assert_eq!(v.upload_dials(), 0);
}
