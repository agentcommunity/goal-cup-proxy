//! H-10 slice 4a: the one local-finish budget of a v5 capture entry, through the real daemon and
//! its control API (CONTRACTS.md, "C6 local finish budget (H-10 4a)").
//!
//! Every request is built as raw JSON, so a row states the wire contract and not a Rust type:
//! `remaining_finish_ms` is the caller's remaining budget, a refusal is the `finish_deadline`
//! code with the refused stage's token as its message, and a refusal writes no terminal.
//!
//! The holds are the daemon's own `test-hooks` seams (`Daemon::hold_blocking_permits`,
//! `hold_capture_owner_drain`, `delay_capture_finish_stage`), the precedent being the registry's.
//! They exist in no shipped build. A real binary is not made to hold a permit or an owner; the
//! campaign rows (`capture_failure_campaign.rs`) substitute a whole-daemon SIGSTOP and say so.

use crate::common::capture_lifecycle_support;
use crate::common::support;
use crate::common::table_lock;

use capture_lifecycle_support::{
    Child, Rig, Wire, fresh_id, launch_request, local_finishes, native_rig, streamless,
};
use goalcup_protocol::capture::{CaptureRecord, GapReason};
use goalcup_protocol::control::{
    CaptureFinishReport, CaptureWorkspaceInfo, LocalSealOutcome, SUFFIX_SEAL, entry_route,
};
use goalcup_protocol::process::ProcessRegistration;
use goalcup_protocol::provenance::{TerminationFailureV1, TerminationOutcomeV1};
use serde_json::{Value, json};
use std::os::unix::process::CommandExt;
use std::time::{Duration, Instant};
use support::control::Response;
use table_lock::TableLock;

/// A seal request body: `proxy` counters and, when given, the caller's remaining budget.
fn request(proxy: Option<Value>, remaining_ms: Option<u64>) -> Value {
    let mut body = json!({ "proxy": proxy });
    if let Some(ms) = remaining_ms {
        body["remaining_finish_ms"] = json!(ms);
    }
    body
}

async fn seal(rig: &Rig, entry_id: &str, body: Value) -> (Response, Duration) {
    let began = Instant::now();
    let response = rig
        .client
        .post_body(&entry_route(entry_id, SUFFIX_SEAL), &body)
        .await;
    (response, began.elapsed())
}

/// The wire code and message of a failure body.
fn refusal(response: &Response) -> (String, String) {
    let body: Value = response.json().expect("a JSON failure body");
    (
        body["error"]["code"].as_str().unwrap_or("").to_string(),
        body["error"]["message"].as_str().unwrap_or("").to_string(),
    )
}

fn assert_finish_deadline(response: &Response, stage: &str) {
    assert_eq!(response.status(), 409, "{}", response.body());
    assert_eq!(
        refusal(response),
        ("finish_deadline".to_string(), stage.to_string()),
        "{}",
        response.body()
    );
}

fn position(records: &[CaptureRecord], pick: impl Fn(&CaptureRecord) -> bool) -> usize {
    records
        .iter()
        .position(pick)
        .expect("the record is in the chain")
}

/// The launcher's proxy counters after a full capture table: the one non-zero counter.
fn table_full_counters() -> Value {
    json!({
        "tunneled_fail_open": 0,
        "opaque_fallbacks": 0,
        "capture_table_full": 3,
        "cut_unsettled": 0,
        "unterminated": 0,
        "sink_refusals": 0
    })
}

/// A capture-dev entry with a live harness registered and its launch recorded.
async fn launched(rig: &Rig, child: &Child) -> CaptureWorkspaceInfo {
    let info = rig.created("capture-dev").await;
    let registered = rig
        .register(&info.entry_id, &child.registration(info.process_generation))
        .await;
    assert_eq!(registered.status(), 204, "{}", registered.body());
    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    let launch = rig.launch(&info.entry_id, &launch_request(&goal)).await;
    assert_eq!(launch.status(), 204, "{}", launch.body());
    info
}

fn alive(pid: u32) -> bool {
    goalcup_protocol::process::is_alive(pid)
}

#[tokio::test]
async fn blocked_permit_refuses_without_terminal_then_seals_with_gap() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let hold = rig.daemon.hold_blocking_permits().await;

    let (refused, took) = seal(
        &rig,
        &info.entry_id,
        request(Some(table_full_counters()), Some(3_000)),
    )
    .await;
    assert_finish_deadline(&refused, "blocking_permit");
    assert!(took < Duration::from_secs(6), "refused in {took:?}");
    assert_eq!(local_finishes(&rig.records(&info.entry_id)), 0);
    assert!(!rig.trace_dir(&info.entry_id).join("seal.json").exists());

    drop(hold);
    let (sealed, _) = seal(&rig, &info.entry_id, request(None, None)).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let report: CaptureFinishReport = sealed.json().unwrap();
    assert_eq!(report.seal, LocalSealOutcome::SealWritten);

    let records = rig.records(&info.entry_id);
    assert_eq!(local_finishes(&records), 1);
    assert_eq!(
        streamless(&records, GapReason::LimitExceeded),
        1,
        "the expired attempt's proxy counters are chained once, by the next finish"
    );
    assert_eq!(
        streamless(&records, GapReason::ShutdownDeadline),
        1,
        "the expired attempt leaves one sticky gap"
    );
    let counters = position(&records, |r| {
        matches!(
            r,
            CaptureRecord::Gap {
                reason: GapReason::LimitExceeded,
                ..
            }
        )
    });
    let latched = position(&records, |r| {
        matches!(
            r,
            CaptureRecord::Gap {
                reason: GapReason::ShutdownDeadline,
                ..
            }
        )
    });
    let finish = position(&records, |r| matches!(r, CaptureRecord::LocalFinish { .. }));
    assert!(counters < latched && latched < finish);
}

/// A finish whose caller goes away mid-handler (a daemon stalled past the CLI's read timeout, or
/// the launcher dying) has already set `finishing` and closed admission. The launcher's proxy
/// counters exist nowhere else, so the dropped attempt must latch them like a refusal does: the
/// next finish, a standalone `goalcup seal` that sends no counters, chains them and one
/// stream-less `ShutdownDeadline` gap, and the trace reads incomplete.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_finish_whose_caller_disconnects_still_latches_its_counters_and_a_gap() {
    use std::io::Write;
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let hold = rig.daemon.hold_blocking_permits().await;

    // The request, on a raw socket the test closes while the handler waits for its permit.
    let body = serde_json::to_string(&request(Some(table_full_counters()), Some(10_000))).unwrap();
    let head = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\n\
         x-goalcup-control-version: {}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        entry_route(&info.entry_id, SUFFIX_SEAL),
        rig.client.addr,
        rig.client.token,
        goalcup_protocol::control::CONTROL_API_VERSION,
        body.len()
    );
    let mut socket = std::net::TcpStream::connect(rig.client.addr).unwrap();
    socket.write_all(head.as_bytes()).unwrap();
    socket.write_all(body.as_bytes()).unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while rig.daemon.capture_finishing(&info.entry_id) != Some(true) {
        assert!(Instant::now() < until, "the handler never began the finish");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drop(socket);
    // Wait for hyper to notice the close and drop the handler's future: its drop guard latches
    // the carried counters, and that latch is the state the test needs before it lets go of the
    // permits. (Releasing them earlier lets the live handler finish the attempt normally.)
    let until = Instant::now() + Duration::from_secs(30);
    while rig.daemon.capture_finish_latched(&info.entry_id) != Some(true) {
        assert!(
            Instant::now() < until,
            "the dropped attempt never latched its counters"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drop(hold);

    let (sealed, _) = seal(&rig, &info.entry_id, request(None, None)).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let records = rig.records(&info.entry_id);
    assert_eq!(local_finishes(&records), 1);
    assert_eq!(
        streamless(&records, GapReason::LimitExceeded),
        1,
        "the dropped attempt's counters are chained by the next finish: {records:#?}"
    );
    assert_eq!(
        streamless(&records, GapReason::ShutdownDeadline),
        1,
        "and so is one gap for the interval nobody was listening"
    );
    let counters = position(&records, |r| {
        matches!(
            r,
            CaptureRecord::Gap {
                reason: GapReason::LimitExceeded,
                ..
            }
        )
    });
    let latched = position(&records, |r| {
        matches!(
            r,
            CaptureRecord::Gap {
                reason: GapReason::ShutdownDeadline,
                ..
            }
        )
    });
    let finish = position(&records, |r| matches!(r, CaptureRecord::LocalFinish { .. }));
    assert!(counters < latched && latched < finish);

    // Optional end to end: `GOALCUP_VERIFY_BIN` names a built `goalcup-verify`, and the sealed
    // trace must not read as valid (stream-less gaps make it incomplete, exit 1).
    if let Some(bin) = std::env::var_os("GOALCUP_VERIFY_BIN") {
        let out = std::process::Command::new(bin)
            .arg("--capture-integrity")
            .arg(rig.trace_dir(&info.entry_id))
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

#[tokio::test]
async fn slow_owner_refuses_without_terminal_and_never_appends_after_local_finish() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    rig.daemon.hold_capture_owner_drain(&info.entry_id, true);
    // A failed assertion below must not leave the owner parked: the daemon's stop joins it.
    struct Release<'a>(&'a goalcup_daemon::runtime::Daemon, String);
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            self.0.hold_capture_owner_drain(&self.1, false);
        }
    }
    let _release = Release(&rig.daemon, info.entry_id.clone());

    let (refused, took) = seal(&rig, &info.entry_id, request(None, Some(3_000))).await;
    assert_finish_deadline(&refused, "owner_drain");
    assert!(took < Duration::from_secs(6), "refused in {took:?}");
    assert_eq!(local_finishes(&rig.records(&info.entry_id)), 0);

    rig.daemon.hold_capture_owner_drain(&info.entry_id, false);
    let (sealed, _) = seal(&rig, &info.entry_id, request(None, None)).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let before = rig.chain(&info.entry_id).len();
    assert!(matches!(
        rig.records(&info.entry_id).last(),
        Some(CaptureRecord::LocalFinish { .. })
    ));
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        rig.chain(&info.entry_id).len(),
        before,
        "nothing is appended after the LocalFinish"
    );
    assert_eq!(local_finishes(&rig.records(&info.entry_id)), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refusal_after_termination_binds_first_outcome() {
    let (rig, _native, _connectors) = native_rig().await;
    let harness = Child::sleeping();
    let pid = harness.0.id();
    let info = launched(&rig, &harness).await;
    rig.daemon.delay_capture_finish_stage(
        &info.entry_id,
        "workspace_walk",
        Duration::from_millis(5_300),
    );

    let (refused, _) = seal(&rig, &info.entry_id, request(None, Some(6_000))).await;
    assert_finish_deadline(&refused, "workspace_walk");
    assert!(!alive(pid), "the first attempt ended the harness it owned");

    let (sealed, _) = seal(&rig, &info.entry_id, request(None, None)).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let report: CaptureFinishReport = sealed.json().unwrap();
    assert!(
        matches!(report.termination, TerminationOutcomeV1::Confirmed { .. }),
        "the next finish binds the first outcome, never `AlreadyStopped`: {:?}",
        report.termination
    );
}

/// A harness that is parked before its `exec`: its argv is a shell's until `go` appears, then
/// the registered binary's. Its pid and process-start identity do not change across the `exec`.
struct Parked {
    child: std::process::Child,
    dir: tempfile::TempDir,
    _lock: TableLock,
}

impl Parked {
    fn new() -> Parked {
        let lock = TableLock::acquire();
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("gatedharness");
        // A symlink, not a copy: a copied platform binary is killed by the code-signing check.
        std::os::unix::fs::symlink("/bin/sleep", &program).unwrap();
        let script = dir.path().join("parked.sh");
        std::fs::write(
            &script,
            format!(
                "while [ ! -f \"$1\" ]; do /bin/sleep 0.05; done\nexec {} 300\n",
                program.display()
            ),
        )
        .unwrap();
        let child = std::process::Command::new("/bin/sh")
            .arg(&script)
            .arg(dir.path().join("go"))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        Parked {
            child,
            dir,
            _lock: lock,
        }
    }

    fn program(&self) -> String {
        self.dir.path().join("gatedharness").display().to_string()
    }

    fn registration(&self, generation: u64) -> ProcessRegistration {
        let mut handle = support::handle_for_now(self.child.id());
        handle.program = "gatedharness".to_string();
        handle.native_binary = Some("codex".to_string());
        handle.argv = vec![self.program(), "300".to_string()];
        ProcessRegistration { generation, handle }
    }

    /// Release the harness and wait until the process table shows it running the binary.
    fn release(&self) {
        std::fs::write(self.dir.path().join("go"), b"").unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            let args = std::process::Command::new("/bin/ps")
                .args(["-o", "args=", "-p", &self.child.id().to_string()])
                .output()
                .unwrap();
            if String::from_utf8_lossy(&args.stdout).contains("gatedharness") {
                return;
            }
            assert!(Instant::now() < until, "the parked harness never exec'd");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Parked {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parked_refusal_then_expiry_then_seal_terminates_released_harness() {
    let (rig, _native, _connectors) = native_rig().await;
    let parked = Parked::new();
    let pid = parked.child.id();
    let info = rig.created("capture-dev").await;
    let registered = rig
        .register(
            &info.entry_id,
            &parked.registration(info.process_generation),
        )
        .await;
    assert_eq!(registered.status(), 204, "{}", registered.body());
    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    rig.launch(&info.entry_id, &launch_request(&goal)).await;
    rig.daemon.delay_capture_finish_stage(
        &info.entry_id,
        "workspace_walk",
        Duration::from_millis(5_300),
    );

    // The seal meets the parked child: the argv guard refuses it, nothing is signalled, and the
    // attempt then expires before its walk.
    let (refused, _) = seal(&rig, &info.entry_id, request(None, Some(6_000))).await;
    assert_finish_deadline(&refused, "workspace_walk");
    assert!(alive(pid), "a refused guard signals nothing");

    // The harness is released and now runs the registered binary. The next finish must not
    // reuse the inconclusive outcome: it terminates the harness it left running.
    parked.release();
    let (sealed, _) = seal(&rig, &info.entry_id, request(None, None)).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let report: CaptureFinishReport = sealed.json().unwrap();
    assert!(
        matches!(report.termination, TerminationOutcomeV1::Confirmed { .. }),
        "{:?}",
        report.termination
    );
    let until = Instant::now() + Duration::from_secs(5);
    while alive(pid) {
        assert!(
            Instant::now() < until,
            "the released harness is still running"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn an_inconclusive_first_outcome_is_not_reused_when_nothing_ended_the_harness() {
    // The same sequence with a harness the guard refuses for good: the second finish asks again
    // and binds what it finds, not the first attempt's text.
    let (rig, _native, _connectors) = native_rig().await;
    let parked = Parked::new();
    let info = rig.created("capture-dev").await;
    let registered = rig
        .register(
            &info.entry_id,
            &parked.registration(info.process_generation),
        )
        .await;
    assert_eq!(registered.status(), 204, "{}", registered.body());
    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    rig.launch(&info.entry_id, &launch_request(&goal)).await;
    rig.daemon.delay_capture_finish_stage(
        &info.entry_id,
        "workspace_walk",
        Duration::from_millis(5_300),
    );
    let (refused, _) = seal(&rig, &info.entry_id, request(None, Some(6_000))).await;
    assert_finish_deadline(&refused, "workspace_walk");

    let (sealed, _) = seal(&rig, &info.entry_id, request(None, None)).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let report: CaptureFinishReport = sealed.json().unwrap();
    assert!(
        matches!(
            report.termination,
            TerminationOutcomeV1::Unresolved {
                reason: TerminationFailureV1::ProcessAuthorityUnavailable,
                ..
            }
        ),
        "{:?}",
        report.termination
    );
    assert!(
        alive(parked.child.id()),
        "a parked child is never signalled"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drain_expiry_cuts_and_still_seals_within_partition() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    // A producer that never settles: it opens a stream and stays connected.
    let wired = info.clone();
    let wire = tokio::task::spawn_blocking(move || {
        let mut wire = Wire::connect(&wired, &wired.producer_id);
        let id = fresh_id();
        wire.send(0, support_connection(&id));
        wire
    })
    .await
    .unwrap();

    // Budget 4 s: the drain share ends at 2 s, then the producer is cut with a gap and the
    // commit reserve seals.
    let (sealed, took) = seal(&rig, &info.entry_id, request(None, Some(4_000))).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    assert!(
        took >= Duration::from_millis(1_800),
        "the drain share was spent first: {took:?}"
    );
    assert!(took < Duration::from_secs(7), "sealed in {took:?}");
    let records = rig.records(&info.entry_id);
    assert_eq!(local_finishes(&records), 1);
    assert!(
        records.iter().any(|r| matches!(
            r,
            CaptureRecord::Gap {
                reason: GapReason::ShutdownDeadline,
                ..
            }
        )),
        "the cut producer is a ShutdownDeadline gap: {records:#?}"
    );
    drop(wire);
}

fn support_connection(id: &str) -> goalcup_protocol::capture::FrameEvent {
    goalcup_protocol::capture::FrameEvent::Connection {
        connection: id.to_string(),
        event: goalcup_protocol::capture::ConnectionEvent::Open {
            alpn: None,
            already_open: false,
        },
    }
}

/// H-16: a producer that chained a `Loss` and then closed cleanly, followed by a standalone seal
/// that carries no proxy counters (`proxy: None`, as `goalcup seal` always sends). The loss is
/// already in the chain on the producer's own clock, so the seal is incomplete, not clean.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_standalone_seal_after_a_lossy_clean_close_is_incomplete() {
    use goalcup_protocol::capture::{ConnectionEvent, ConnectionOutcome, FrameEvent};
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let wired = info.clone();
    let producer = info.producer_id.clone();
    tokio::task::spawn_blocking(move || {
        let mut wire = Wire::connect(&wired, &wired.producer_id);
        let id = fresh_id();
        wire.send(0, support_connection(&id));
        wire.send(
            0,
            FrameEvent::Loss {
                reason: GapReason::DecodeFailed,
            },
        );
        wire.send(
            0,
            FrameEvent::Connection {
                connection: id,
                event: ConnectionEvent::Close {
                    outcome: ConnectionOutcome::Complete,
                },
            },
        );
        wire.close();
    })
    .await
    .unwrap();
    rig.records_when(&info.entry_id, |records| {
        records.iter().any(
            |r| matches!(r, CaptureRecord::ProducerClosed { producer: p, .. } if *p == producer),
        )
    })
    .await;

    let (sealed, _) = seal(&rig, &info.entry_id, request(None, Some(10_000))).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let records = rig.records(&info.entry_id);
    assert_eq!(local_finishes(&records), 1);
    assert_eq!(
        streamless(&records, GapReason::DecodeFailed),
        1,
        "the loss is chained once and nothing else added a DecodeFailed gap: {records:#?}"
    );
    let gap = position(&records, |r| {
        matches!(
            r,
            CaptureRecord::Gap { stream: None, reason: GapReason::DecodeFailed, observed, .. }
                if observed.clock_id == producer
        )
    });
    let closed = position(
        &records,
        |r| matches!(r, CaptureRecord::ProducerClosed { producer: p, .. } if *p == producer),
    );
    assert!(gap < closed, "the loss precedes the producer's close");

    // End to end, when a `goalcup-verify` is built beside the test binaries or named by
    // `GOALCUP_VERIFY_BIN`: the sealed trace is incomplete (exit 1), not valid (exit 0).
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("goalcup-verify")))
        .filter(|path| path.exists());
    if let Some(bin) = std::env::var_os("GOALCUP_VERIFY_BIN").or(sibling.map(Into::into)) {
        let out = std::process::Command::new(bin)
            .arg("--capture-integrity")
            .arg(rig.trace_dir(&info.entry_id))
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_launcher_finish_and_standalone_seal_share_one_commit() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    rig.daemon.delay_capture_finish_stage(
        &info.entry_id,
        "workspace_walk",
        Duration::from_millis(800),
    );

    let first = seal(
        &rig,
        &info.entry_id,
        request(Some(table_full_counters()), Some(10_000)),
    );
    let second = async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        seal(&rig, &info.entry_id, request(None, Some(10_000))).await
    };
    let ((first, _), (second, _)) = tokio::join!(first, second);
    assert_eq!(first.status(), 200, "{}", first.body());
    assert_eq!(second.status(), 200, "{}", second.body());
    let (a, b): (CaptureFinishReport, CaptureFinishReport) =
        (first.json().unwrap(), second.json().unwrap());
    assert_eq!(a.terminal_seq, b.terminal_seq);
    assert_eq!(a.seal, b.seal);
    assert_eq!(local_finishes(&rig.records(&info.entry_id)), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn run_lock_refusal_leaves_the_other_finish_alone_and_latches_nothing() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    rig.daemon.delay_capture_finish_stage(
        &info.entry_id,
        "workspace_walk",
        Duration::from_millis(4_000),
    );

    let first = seal(&rig, &info.entry_id, request(None, Some(10_000)));
    let second = async {
        tokio::time::sleep(Duration::from_millis(1_000)).await;
        seal(&rig, &info.entry_id, request(None, Some(1_500))).await
    };
    let ((first, _), (second, took)) = tokio::join!(first, second);
    assert_finish_deadline(&second, "run_lock");
    assert!(took < Duration::from_secs(5), "refused in {took:?}");
    assert_eq!(first.status(), 200, "{}", first.body());
    let records = rig.records(&info.entry_id);
    assert_eq!(local_finishes(&records), 1);
    assert_eq!(
        streamless(&records, GapReason::ShutdownDeadline),
        0,
        "a run-lock refusal is another finish's entry: it latches nothing"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_finish_then_restart_reads_ingest_lost_and_still_seals() {
    let (rig, _native, _connectors) = native_rig().await;
    let harness = Child::sleeping();
    let info = launched(&rig, &harness).await;
    let hold = rig.daemon.hold_blocking_permits().await;
    let (refused, _) = seal(&rig, &info.entry_id, request(None, Some(3_000))).await;
    assert_finish_deadline(&refused, "blocking_permit");
    drop(hold);

    let rig = rig.restart().await;
    let (sealed, _) = seal(&rig, &info.entry_id, request(None, None)).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let records = rig.records(&info.entry_id);
    assert_eq!(local_finishes(&records), 1);
    assert!(
        streamless(&records, GapReason::IngestLost) >= 1,
        "reopen keeps a launched run incomplete: {records:#?}"
    );
    assert_eq!(
        streamless(&records, GapReason::ShutdownDeadline),
        0,
        "the in-memory latch does not survive a restart; reopen's own gap does"
    );
}

#[tokio::test]
async fn every_finish_attempt_sets_finishing_and_nothing_clears_it() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    assert_eq!(rig.daemon.capture_finishing(&info.entry_id), Some(false));

    // A zero budget is refused before any finish step: it is not an attempt.
    let (zero, _) = seal(&rig, &info.entry_id, request(None, Some(0))).await;
    assert_finish_deadline(&zero, "not_started");
    assert_eq!(rig.daemon.capture_finishing(&info.entry_id), Some(false));

    let hold = rig.daemon.hold_blocking_permits().await;
    for _ in 0..2 {
        let (refused, _) = seal(&rig, &info.entry_id, request(None, Some(1_500))).await;
        assert_finish_deadline(&refused, "blocking_permit");
        assert_eq!(
            rig.daemon.capture_finishing(&info.entry_id),
            Some(true),
            "an expired attempt leaves the flag set"
        );
    }
    drop(hold);
}

#[tokio::test]
async fn remaining_finish_defaults_clamps_and_refuses_zero() {
    let rig = Rig::start().await;

    // Zero: refused at `not_started`, the entry untouched and still finishable.
    let zero = rig.created("codex-cli").await;
    let (refused, took) = seal(&rig, &zero.entry_id, request(None, Some(0))).await;
    assert_finish_deadline(&refused, "not_started");
    assert!(took < Duration::from_secs(1), "{took:?}");
    assert_eq!(local_finishes(&rig.records(&zero.entry_id)), 0);

    // Larger than the budget: clamped, and the finish succeeds.
    let (clamped, _) = seal(&rig, &zero.entry_id, request(None, Some(999_999_999))).await;
    assert_eq!(clamped.status(), 200, "{}", clamped.body());

    // Absent: the full budget, as an older CLI sends it.
    let absent = rig.created("codex-cli").await;
    let (sealed, _) = seal(&rig, &absent.entry_id, json!({ "proxy": null })).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());

    // Strict decoding is unchanged: an unknown field is still refused before any finish step.
    let strict = rig.created("codex-cli").await;
    let (unknown, _) = seal(
        &rig,
        &strict.entry_id,
        json!({ "proxy": null, "remaining_finish_s": 3 }),
    )
    .await;
    assert_eq!(unknown.status(), 400, "{}", unknown.body());
    assert_eq!(local_finishes(&rig.records(&strict.entry_id)), 0);
}

/// A `/bin/sh` that ignores SIGTERM and then becomes `sleep 300`, in its own process group. It
/// returns only once both are true: the shell writes a ready file after `trap`, and the process
/// table then names the pid `sleep` (the `exec` happened). Waiting for the condition replaces a
/// fixed sleep that a loaded machine can outrun, which kills the shell on the first SIGTERM and
/// makes the termination tests fail for the wrong reason.
fn spawn_sigterm_ignoring_sleep() -> std::process::Child {
    let dir = tempfile::tempdir().unwrap();
    let ready = dir.path().join("ready");
    // Write-then-rename, so the file the test sees is whole.
    let script = r#"trap '' TERM; : > "$1.tmp"; mv "$1.tmp" "$1"; exec /bin/sleep 300"#;
    let child = std::process::Command::new("/bin/sh")
        .args(["-c", script, "sh"])
        .arg(&ready)
        .process_group(0)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    let until = Instant::now() + Duration::from_secs(30);
    let mut exec_done = false;
    while !(ready.exists() && exec_done) {
        assert!(
            Instant::now() < until,
            "the shell (pid {pid}) never ignored SIGTERM and became sleep"
        );
        std::thread::sleep(Duration::from_millis(25));
        if ready.exists() {
            exec_done = goalcup_protocol::process::snapshot()
                .map(|table| table.iter().any(|p| p.pid == pid && p.comm == "sleep"))
                .unwrap_or(false);
        }
    }
    child
}

#[tokio::test]
async fn termination_time_is_outside_the_budget() {
    // A harness that ignores SIGTERM is ended only after the connector's whole grace. A caller
    // budget shorter than that grace must not cut it, and must not refuse the stages after it.
    let rig = Rig::start().await;
    let lock = TableLock::acquire();
    let info = rig.created("codex-cli").await;
    let mut stubborn = spawn_sigterm_ignoring_sleep();
    let mut handle = support::handle_for_now(stubborn.id());
    handle.argv = vec!["/bin/sleep".to_string(), "300".to_string()];
    handle.grace_seconds = 4;
    handle.native_binary = Some("codex".to_string());
    let registered = rig
        .register(
            &info.entry_id,
            &ProcessRegistration {
                generation: info.process_generation,
                handle,
            },
        )
        .await;
    assert_eq!(registered.status(), 204, "{}", registered.body());

    let (sealed, took) = seal(&rig, &info.entry_id, request(None, Some(3_000))).await;
    let _ = stubborn.kill();
    let _ = stubborn.wait();
    drop(lock);
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let report: CaptureFinishReport = sealed.json().unwrap();
    assert_eq!(report.seal, LocalSealOutcome::SealWritten);
    assert!(
        took >= Duration::from_millis(4_000),
        "the connector's 4 s grace was given in full, though the budget was 3 s: {took:?}"
    );
}

#[tokio::test]
async fn standalone_seal_ends_sigterm_ignoring_harness_with_full_grace() {
    // Pin: green before and after. The default budget never shortens the grace.
    let rig = Rig::start().await;
    let lock = TableLock::acquire();
    let info = rig.created("codex-cli").await;
    let mut stubborn = spawn_sigterm_ignoring_sleep();
    let pid = stubborn.id();
    let mut handle = support::handle_for_now(pid);
    handle.argv = vec!["/bin/sleep".to_string(), "300".to_string()];
    handle.grace_seconds = 2;
    handle.native_binary = Some("codex".to_string());
    let registered = rig
        .register(
            &info.entry_id,
            &ProcessRegistration {
                generation: info.process_generation,
                handle,
            },
        )
        .await;
    assert_eq!(registered.status(), 204, "{}", registered.body());

    let (sealed, took) = seal(&rig, &info.entry_id, json!({ "proxy": null })).await;
    let _ = stubborn.wait();
    drop(lock);
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let report: CaptureFinishReport = sealed.json().unwrap();
    assert!(
        matches!(report.termination, TerminationOutcomeV1::Confirmed { .. }),
        "{:?}",
        report.termination
    );
    assert!(
        took >= Duration::from_millis(2_000),
        "SIGKILL came before the 2 s grace ended: {took:?}"
    );
    assert!(!alive(pid), "the harness is gone");
}
