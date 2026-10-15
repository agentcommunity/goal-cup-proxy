//! The local production vertical on a synthetic workload: the shipped `goalcup-daemon`, this
//! build's `goalcup start --goal-file` with its in-process proxy and IPC sender, the physically
//! reserved chain, native capture, the local finish and `goalcup-verify`, with no service.
//! Only the workload (`fixtures/m2-controlled-workload.py`) and its TLS upstream (the
//! controlled TLS endpoint) are fixtures; `m2_vertical_support` says how they are wired.
//!
//! Each test holds evidence against evidence: what the workload reports it sent, against what
//! the chain file retained and what the daemon's status and the verifier say.
//!
//! macOS and Linux (the trace folder must be on APFS, ext4 or XFS), ignored by default because
//! each test runs real processes for seconds to minutes. Build the binaries first, then run by name:
//!
//! ```text
//! cargo build --offline
//! cargo test -p goalcup-cli --offline --features test-support --test full m2_synthetic_vertical:: -- --ignored --test-threads=1
//! ```

use crate::common::m2_common;
use crate::common::m2_vertical_support;
use crate::common::support;

use crate::common::table_lock;

use goalcup_protocol::capture::{
    CaptureRecord, CaptureSource, CaptureState, ConnectionEvent, DeliveryState, EndReason,
    GapReason, RunState,
};
use m2_common::contains;
use m2_vertical_support::{
    GOAL, RESERVE, Stream, Vertical, files_under, holds_proxy_credential, now, sentinel_prefix,
    wall,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use table_lock::TableLock;

/// Room for a workload that ends on its own, then the finish.
const RUN: Duration = Duration::from_secs(60);

/// The clean stream in `streams` whose retained bytes hash to `sha256`, from `source`.
fn retained<'a>(
    streams: &'a std::collections::BTreeMap<goalcup_protocol::capture::StreamKey, Stream>,
    source: CaptureSource,
    sha256: &Value,
) -> Option<&'a Stream> {
    streams
        .values()
        .find(|s| s.source == source && s.clean() && *sha256 == s.sha256())
}

fn items(report: &Value, lane: &str) -> Vec<Value> {
    report["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["lane"] == lane)
        .cloned()
        .collect()
}

fn assert_incomplete(v: &Vertical, entry: &str) -> Value {
    let (code, integrity) = v.verify(entry);
    assert_eq!(code, 1, "{integrity}");
    assert_eq!(integrity["integrity"], "incomplete");
    assert_eq!(integrity["sealed"], true);
    assert_eq!(integrity["coverage_incomplete"], true);
    integrity
}

#[test]
#[ignore = "real processes and sockets; run by name with --ignored"]
fn local_finish_without_service_is_complete() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("plain", json!({"hold": true}));
    let mut start = v.start(RESERVE);
    let entry = v.entry_id();

    // Status while the workload's traffic is live, until it holds.
    let mut samples = Vec::new();
    while !v.out.join("holding").exists() {
        samples.push((now(), v.status(&entry)));
        assert!(samples.len() < 600, "the workload never held");
        std::thread::sleep(Duration::from_millis(100));
    }
    let sse = v
        .events()
        .into_iter()
        .find(|e| e["lane"] == "sse")
        .expect("the SSE exchange");
    let (sse_start, sse_end) = (sse["start"].as_f64().unwrap(), sse["end"].as_f64().unwrap());
    let live: Vec<_> = samples
        .iter()
        .filter(|(at, _)| *at > sse_start && *at < sse_end)
        .map(|(_, s)| s.payload.capture.clone().expect("a capture status"))
        .collect();
    assert!(
        !live.is_empty(),
        "no status was read while the SSE response streamed"
    );
    // The launch post arrives after the child has exec'ed and the daemon hashes the build
    // before it writes `Launch`, so a slow disk can leave the run `Starting` while the
    // workload's traffic is already captured. Wait, bounded, for `Running` before asking for it.
    let until = Instant::now() + Duration::from_secs(30);
    let mut now_running = v.status(&entry).payload.capture.expect("a capture status");
    while now_running.run != RunState::Running {
        assert!(
            Instant::now() < until,
            "the run never became Running within 30 s; last state {:?}",
            now_running.run
        );
        std::thread::sleep(Duration::from_millis(100));
        now_running = v.status(&entry).payload.capture.expect("a capture status");
    }
    for capture in live.iter().chain(std::iter::once(&now_running)) {
        // Before the `Launch` record the run and its capture are both still `Starting`.
        let (run, state) = match capture.run {
            RunState::Starting => (RunState::Starting, CaptureState::Starting),
            _ => (RunState::Running, CaptureState::Recording),
        };
        assert_eq!((capture.run, capture.capture), (run, state), "{capture:?}");
        assert_eq!(capture.delivery, DeliveryState::LocalOnly);
        if run == RunState::Running {
            assert!(capture.storage.durable.is_some(), "{capture:?}");
        }
    }

    v.release();
    let exit = start.wait(RUN);
    assert!(exit.success(), "{exit}\n{}", start.output());
    let printed = start.output();
    assert!(printed.contains("local only"), "{printed}");
    assert!(printed.contains("--capture-integrity"), "{printed}");

    // What the workload did, from its own report and the run directory.
    let report = v.report();
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(
        report["goal"]["sha256"],
        goalcup_daemon::workspace::sha256_hex(GOAL.as_bytes()),
        "the goal reached the workload whole, as its one argument"
    );
    let run = v.run_dir().unwrap();
    assert_eq!(
        std::fs::read_to_string(run.join("workspace/result.txt")).unwrap(),
        "m2-controlled-workload ok\n"
    );

    // The chain: v5 throughout, one Launch naming the one Goal stream, and every exchange
    // retained byte for byte.
    let chain = v.chain(&entry);
    assert!(chain.lines.iter().all(|l| l.protocol_version == 5));
    let streams = chain.streams();
    let goals: Vec<_> = streams
        .iter()
        .filter(|(_, s)| s.source == CaptureSource::Goal)
        .collect();
    assert_eq!(goals.len(), 1);
    assert_eq!(goals[0].1.data, GOAL.as_bytes());
    let launches: Vec<_> = chain
        .records()
        .filter_map(|r| match r {
            CaptureRecord::Launch {
                goal_stream,
                profile_id,
                ..
            } => Some((goal_stream.clone(), profile_id.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        launches,
        vec![(goals[0].0.clone(), "m2-controlled-workload".to_string())]
    );
    for item in items(&report, "echo") {
        assert!(retained(&streams, CaptureSource::WireRequest, &item["req_sha256"]).is_some());
        assert!(retained(&streams, CaptureSource::WireResponse, &item["resp_sha256"]).is_some());
    }
    for item in items(&report, "sse") {
        assert!(retained(&streams, CaptureSource::WireResponse, &item["resp_sha256"]).is_some());
    }
    for item in items(&report, "ws") {
        assert!(retained(&streams, CaptureSource::WsClient, &item["req_sha256"]).is_some());
        assert!(retained(&streams, CaptureSource::WsServer, &item["resp_sha256"]).is_some());
    }
    // Unknown native and provider identities: the bytes are kept, whole.
    let natives: Vec<_> = streams
        .values()
        .filter(|s| s.source == CaptureSource::Native)
        .collect();
    assert_eq!(natives.len(), 1);
    assert_eq!(natives[0].data, v.native_file());
    assert_eq!(natives[0].meta.protocol_hint, None);
    assert!(
        streams
            .values()
            .filter(|s| s.meta.wire.is_some())
            .all(|s| s.meta.protocol_hint.as_deref() == Some("e00-controlled"))
    );

    // The verifier: complete, sealed, local only, with a local finish and no receipt.
    let trace = v.trace(&entry);
    assert!(!trace.join("receipt.json").exists());
    assert_eq!(
        std::fs::read(trace.join("acks.jsonl")).unwrap(),
        b"",
        "no acknowledgement"
    );
    let (code, integrity) = v.verify(&entry);
    assert_eq!(code, 0, "{integrity}");
    assert_eq!(integrity["integrity"], "valid");
    assert_eq!(integrity["anchor"], "local_only");
    assert_eq!(integrity["sealed"], true);
    assert_eq!(integrity["findings"], json!([]));
    assert_eq!(integrity["terminal"]["kind"], "local_finish");
    assert_eq!(integrity["receipt_clock"], Value::Null);
    assert_eq!(integrity["verified_ack"], Value::Null);
    assert!(
        integrity["terminal"]["termination"]
            .get("already_stopped")
            .is_some(),
        "the workload ended on its own: {integrity}"
    );
    let (code, research) = v.research(&entry);
    assert_eq!(code, 0, "{research}");
    assert_eq!(research["analysis"]["state"], "unavailable", "{research}");
    assert_eq!(research["delivery"]["state"], "local_only");

    // Finished once: a second seal answers the same terminal and appends nothing.
    let status = v.status(&entry).payload;
    let capture = status.capture.unwrap();
    assert_eq!(capture.run, RunState::Sealed);
    assert_eq!(capture.gap_count, 0);
    assert_eq!(status.events_uploaded, 0);
    assert_eq!(status.deadline_at, None);
    let before = chain.sha256();
    let (sealed, said, _) = v.seal(&entry);
    assert!(sealed.success(), "{said}");
    let terminal = integrity["terminal"]["seq"].as_u64().unwrap();
    assert!(
        said.contains(&format!("its terminal record is {terminal}")),
        "{said}"
    );
    assert_eq!(v.chain(&entry).sha256(), before);
}

#[test]
#[ignore = "real processes and sockets; run by name with --ignored"]
fn chain_exists_before_production_launch() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("plain", json!({"hold": true}));
    let mut start = v.start(RESERVE);
    // The workload's first act: the chain already exists, holding the pre-launch records only.
    v.harness_pid();
    let entry = v.entry_id();
    let meta = std::fs::metadata(v.trace(&entry).join("chain.jsonl")).unwrap();
    assert!(meta.len() > 0, "the chain precedes the workload");
    assert!(
        meta.len() < RESERVE / 64,
        "the chain is logical growth, not an allocation"
    );
    let storage = v.status(&entry).payload.capture.unwrap().storage;
    assert_eq!(storage.quota_bytes, RESERVE);
    v.release();
    assert!(start.wait(RUN).success(), "{}", start.output());
}

#[test]
#[ignore = "real processes and sockets; run by name with --ignored"]
fn sparse_stream_has_incremental_durable_evidence() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("sparse", json!({"events": 10, "interval_ms": 10_000}));
    let mut start = v.start(RESERVE);
    let entry = v.entry_id();
    let mut samples = Vec::new();
    let deadline = Instant::now() + RUN + Duration::from_secs(120);
    while !start.exited() {
        assert!(
            Instant::now() < deadline,
            "the sparse launcher never exited"
        );
        let capture = v.status(&entry).payload.capture.unwrap();
        samples.push((now(), capture.storage.durable.map(|d| d.seq)));
        std::thread::sleep(Duration::from_millis(500));
    }
    assert!(start.wait(RUN).success(), "{}", start.output());
    let report = v.report();
    let sse = &items(&report, "sse")[0];
    let (begun, ended) = (sse["start"].as_f64().unwrap(), sse["end"].as_f64().unwrap());
    assert!(ended - begun > 90.0, "{sse}");

    let chain = v.chain(&entry);
    let streams = chain.streams();
    let response = retained(&streams, CaptureSource::WireResponse, &sse["resp_sha256"])
        .expect("the SSE response, retained whole");
    let firsts: Vec<f64> = response
        .segments
        .iter()
        .map(|(_, first, _)| wall(&first.wall))
        .collect();
    assert!(
        response.segments.len() >= 10 && firsts.last().unwrap() - firsts[0] > 80.0,
        "one segment per event, as it arrived: {firsts:?}"
    );
    // Each event is appended soon after its capture, and durable while the stream is open.
    let mut worst_append = 0f64;
    let mut worst_durable = 0f64;
    for (seq, first, recorded_at) in &response.segments {
        let appended = wall(recorded_at);
        worst_append = worst_append.max(appended - wall(&first.wall));
        if appended > ended - 2.0 {
            continue;
        }
        let durable = samples
            .iter()
            .find(|(at, frontier)| *at < ended && frontier.is_some_and(|f| f >= *seq))
            .unwrap_or_else(|| panic!("segment {seq} was not durable before the stream ended"));
        worst_durable = worst_durable.max(durable.0 - appended);
    }
    eprintln!(
        "sparse SSE: worst capture-to-append {:.1} ms (target 250), worst append-to-durable \
         seen by 0.5 s status polls {:.0} ms (sync target 1 s)",
        worst_append * 1e3,
        worst_durable * 1e3
    );
    assert!(worst_append < 2.0, "{worst_append}");
    assert!(worst_durable < 5.0, "{worst_durable}");
    let advances = samples
        .windows(2)
        .filter(|w| w[0].0 < ended && w[1].1 > w[0].1)
        .count();
    assert!(advances >= 5, "the frontier advanced {advances} times");
}

#[test]
#[ignore = "real processes and sockets; run by name with --ignored"]
fn boundary_secret_is_absent_from_chain_and_logs() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("split-secret", json!({}));
    let mut start = v.start(RESERVE);
    let entry = v.entry_id();
    assert!(start.wait(RUN).success(), "{}", start.output());
    let report = v.report();
    assert_eq!(
        report["ok"], true,
        "the relay carried every secret intact: {report}"
    );
    let prefix = sentinel_prefix();
    let chain = v.chain(&entry);
    assert!(!contains(&chain.bytes, &prefix));
    assert!(!holds_proxy_credential(&chain.bytes));

    let streams = chain.streams();
    let redacted = |source: CaptureSource, len: u64| {
        streams.values().any(|s| {
            s.source == source
                && s.source_len == len
                && s.audit.iter().any(|k| k == "anthropic_key")
                && !contains(&s.data, &prefix)
        })
    };
    // Across two request chunks, and back in the response.
    assert!(redacted(CaptureSource::WireRequest, 16384));
    assert!(redacted(CaptureSource::WireResponse, 16384));
    // Across a 4096-byte frame boundary, both ways.
    assert!(redacted(CaptureSource::WsClient, 16384));
    assert!(redacted(CaptureSource::WsServer, 16384));
    // Across two native writes 300 ms apart.
    let native = v.native_file();
    assert!(contains(&native, &prefix), "the source holds it, by design");
    assert!(redacted(CaptureSource::Native, native.len() as u64));
    // Cut off inside the secret: every byte sent is accounted for, the undecided tail
    // replaced rather than released, and the stream ends interrupted.
    let sent = items(&report, "cut")[0]["req_len"].as_u64().unwrap();
    let cut: Vec<_> = streams
        .values()
        .filter(|s| s.source == CaptureSource::WireRequest && !s.clean())
        .collect();
    assert_eq!(cut.len(), 1, "one interrupted request");
    assert_eq!(cut[0].end, Some(EndReason::Interrupted));
    assert_eq!(cut[0].source_len, sent);
    assert!(cut[0].audit.iter().any(|k| k == "anthropic_key"));

    // Nowhere else either: the daemon's and the CLI's output, the machine's records, the
    // trace's other files, the workload's own report and the run directory (the native
    // source file aside); status is also written to the scanned logs.
    let (code, integrity) = v.verify(&entry);
    assert_eq!(code, 0, "nothing sent was lost: {integrity}");
    v.status(&entry);
    let native_path = v.run_dir().unwrap().join("home/native/e02-native.jsonl");
    let mut scanned = 0;
    for (path, bytes) in files_under(&v.root) {
        if path == native_path || path.ends_with("chain.jsonl") {
            continue;
        }
        scanned += 1;
        assert!(!contains(&bytes, &prefix), "{}", path.display());
        assert!(!holds_proxy_credential(&bytes), "{}", path.display());
    }
    assert!(scanned >= 10, "{scanned} files scanned");
}

#[test]
#[ignore = "real processes and sockets; run by name with --ignored"]
fn native_partial_line_survives_local_finish() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("partial-native", json!({"hold": true}));
    let mut start = v.start(RESERVE);
    let entry = v.entry_id();
    v.wait_for("holding");
    v.release();
    assert!(start.wait(RUN).success(), "{}", start.output());
    let native = v.native_file();
    assert_eq!(native.len(), 4 * 1024 + 500);
    assert_ne!(native.last(), Some(&b'\n'), "the file ends inside a line");

    let chain = v.chain(&entry);
    let streams = chain.streams();
    let retained: Vec<_> = streams
        .values()
        .filter(|s| s.source == CaptureSource::Native)
        .collect();
    assert_eq!(retained.len(), 1);
    let stream = retained[0];
    assert_eq!(stream.data, native, "every byte, the partial tail included");
    assert_eq!(stream.source_len, native.len() as u64);
    assert!(stream.audit.is_empty());
    assert_eq!(
        stream.end,
        Some(EndReason::Complete),
        "framing ended at the finish boundary, the tail read"
    );
    assert!(stream.gaps.is_empty());
    let (code, integrity) = v.verify(&entry);
    assert_eq!(code, 0, "{integrity}");
    assert_eq!(integrity["findings"], json!([]));
}

#[test]
#[ignore = "real processes and sockets; run by name with --ignored"]
fn clean_producer_close_and_unexpected_eof_differ() {
    let _lock = TableLock::acquire();
    // A launcher that ends with its harness: the proxy's producer closes in order.
    let v = Vertical::new("plain", json!({}));
    let mut start = v.start(RESERVE);
    let entry = v.entry_id();
    assert!(start.wait(RUN).success(), "{}", start.output());
    let chain = v.chain(&entry);
    let wire = chain.first_wire_producer();
    let of_wire = |r: &CaptureRecord| match r {
        CaptureRecord::Open { stream, .. } | CaptureRecord::End { stream, .. } => {
            stream.producer == wire
        }
        CaptureRecord::Connection { producer, .. } => *producer == wire,
        _ => false,
    };
    let framed = chain.count(of_wire) as u64;
    let closes: Vec<_> = chain
        .lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| match &l.record {
            Some(CaptureRecord::ProducerClosed {
                producer,
                final_seq,
                ..
            }) if *producer == wire => Some((i, *final_seq)),
            _ => None,
        })
        .collect();
    assert_eq!(closes.len(), 1, "one ordered close");
    let (at, final_seq) = closes[0];
    let final_seq = final_seq.expect("a close after frames names its last");
    assert!(
        final_seq + 1 >= framed,
        "final_seq {final_seq} counts every frame: at least {framed}"
    );
    let last_of_wire = chain
        .lines
        .iter()
        .rposition(|l| l.record.as_ref().is_some_and(of_wire))
        .unwrap();
    assert!(at > last_of_wire, "the close follows every frame it counts");
    assert_eq!(chain.count(|r| matches!(r, CaptureRecord::Gap { .. })), 0);
    let (code, integrity) = v.verify(&entry);
    assert_eq!(code, 0, "{integrity}");

    // `kill -9` of the launcher mid-stream: its proxy dies with it, the harness lives on,
    // owned by the daemon, and the producer ends in an unknown tail.
    let v = Vertical::new("stream", json!({}));
    let mut start = v.start(RESERVE);
    let entry = v.entry_id();
    let harness = v.harness_pid();
    v.wait_for_echoes(10);
    let killed_at = now();
    start.kill9();
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        goalcup_protocol::process::is_alive(harness),
        "the harness outlives its launcher"
    );
    assert!(
        v.events()
            .iter()
            .any(|e| e["start"].as_f64().is_some_and(|t| t > killed_at) && e["ok"] == false),
        "the proxy is gone with the launcher"
    );
    let live = v.status(&entry).payload.capture.unwrap();
    assert_eq!(
        live.run,
        RunState::Running,
        "the daemon still owns the harness"
    );
    let (sealed, said, took) = v.seal(&entry);
    assert!(sealed.success(), "{said}");
    assert!(said.contains("Confirmed"), "{said}");
    assert!(took < Duration::from_secs(25), "{took:?}");
    support::wait_until_gone(harness);

    let chain = v.chain(&entry);
    let wire = chain.first_wire_producer();
    assert_eq!(
        chain.count(
            |r| matches!(r, CaptureRecord::ProducerClosed { producer, .. } if *producer == wire)
        ),
        0,
        "no close was ever sent"
    );
    assert!(
        chain.count(|r| matches!(
            r,
            CaptureRecord::Gap {
                reason: GapReason::IngestLost,
                stream,
                ..
            } if stream.as_ref().is_none_or(|s| s.producer == wire)
        )) >= 1,
        "the unknown tail is recorded"
    );
    let integrity = assert_incomplete(&v, &entry);
    assert!(
        integrity["terminal"]["termination"]
            .get("confirmed")
            .is_some(),
        "{integrity}"
    );
}

#[test]
#[ignore = "real processes and sockets; run by name with --ignored"]
fn production_recovery_keeps_relay_and_gap_history() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("stream", json!({}));
    let mut start = v.start(RESERVE);
    let entry = v.entry_id();
    v.wait_for_echoes(20);
    let before = v.status(&entry).payload.capture.unwrap();
    assert_eq!(before.capture, CaptureState::Recording);
    assert_eq!(before.gap_count, 0);

    let daemon = v.daemon_pid();
    let stopped = now();
    // SAFETY: two integers, the daemon this test started.
    assert_eq!(unsafe { libc::kill(daemon, libc::SIGSTOP) }, 0);
    std::thread::sleep(Duration::from_secs(25));
    // SAFETY: as above.
    assert_eq!(unsafe { libc::kill(daemon, libc::SIGCONT) }, 0);
    let continued = now();
    std::thread::sleep(Duration::from_secs(8));
    let after = v.status(&entry).payload.capture.unwrap();
    assert_eq!(after.capture, CaptureState::Incomplete);
    assert!(after.gap_count >= 1);
    v.release();
    assert!(start.wait(RUN).success(), "{}", start.output());

    // The relay never waited on the recorder: every exchange passed, the stalled ones too.
    let events = v.events();
    assert!(events.iter().all(|e| e["ok"] == true), "an exchange failed");
    let stalled: Vec<_> = events
        .iter()
        .filter(|e| {
            e["start"]
                .as_f64()
                .is_some_and(|t| t > stopped && t < continued)
        })
        .collect();
    assert!(
        stalled.len() > 50,
        "{} exchanges during the stall",
        stalled.len()
    );
    let slowest = stalled
        .iter()
        .map(|e| e["end"].as_f64().unwrap() - e["start"].as_f64().unwrap())
        .fold(0f64, f64::max);
    assert!(
        slowest < 5.0,
        "an exchange took {slowest:.1} s during the stall"
    );

    let chain = v.chain(&entry);
    let lost = chain.first_wire_producer();
    let recoveries: Vec<(String, String, GapReason)> = chain
        .records()
        .filter_map(|r| match r {
            CaptureRecord::Recovery {
                previous_producer,
                producer,
                reason,
                ..
            } => Some((previous_producer.clone(), producer.clone(), *reason)),
            _ => None,
        })
        .collect();
    let (named, _, reason) = recoveries.first().expect("a Recovery");
    assert_eq!(*reason, GapReason::QueueFull);
    // The first Recovery names the lost producer, or a fresh epoch lost before any of its
    // records reached the chain, whose Recovery supersedes the lost producer as well.
    let recorded = chain.producers();
    assert!(
        *named == lost || !recorded.contains(named),
        "the first Recovery names {named}, a producer with records that is not {lost}"
    );
    let fresh: Vec<String> = recoveries.iter().map(|(_, p, _)| p.clone()).collect();
    assert!(!fresh.contains(&lost));
    let opened_by_lost: Vec<String> = chain
        .records()
        .filter_map(|r| match r {
            CaptureRecord::Connection {
                producer,
                connection,
                event:
                    ConnectionEvent::Open {
                        already_open: false,
                        ..
                    },
                ..
            } if *producer == lost => Some(connection.clone()),
            _ => None,
        })
        .collect();
    assert!(
        chain.records().any(|r| matches!(
            r,
            CaptureRecord::Connection {
                producer,
                connection,
                event: ConnectionEvent::Open { already_open: true, .. },
                ..
            } if fresh.contains(producer) && opened_by_lost.contains(connection)
        )),
        "a connection opened under the lost producer is re-observed in a fresh epoch"
    );
    assert!(
        chain.records().any(|r| matches!(
            r,
            CaptureRecord::Open { stream, source: CaptureSource::WireRequest, .. }
                if stream.producer != lost
        )),
        "capture resumed in a fresh epoch"
    );
    assert_eq!(
        chain.count(|r| matches!(r, CaptureRecord::Assistance { .. })),
        0,
        "recorder recovery is not assistance"
    );
    assert_eq!(
        chain.count(|r| matches!(r, CaptureRecord::Launch { .. })),
        1
    );
    let integrity = assert_incomplete(&v, &entry);
    assert!(integrity["gap_count"].as_u64().unwrap() >= 1);
    assert!(
        integrity["findings"]
            .as_array()
            .unwrap()
            .contains(&json!("producer_recovered")),
        "{integrity}"
    );
    let sealed = v.status(&entry).payload.capture.unwrap();
    assert_eq!(sealed.run, RunState::Sealed);
    assert!(sealed.gap_count >= 1, "gap history outlives the finish");

    // The loss, reported and not bounded: sent echo bytes with no clean retained stream.
    let streams = chain.streams();
    let unretained: u64 = events
        .iter()
        .filter(|e| e["lane"] == "echo")
        .filter(|e| retained(&streams, CaptureSource::WireRequest, &e["req_sha256"]).is_none())
        .map(|e| e["req_len"].as_u64().unwrap() + e["resp_len"].as_u64().unwrap())
        .sum();
    eprintln!(
        "recovery: {} Recovery record(s); {unretained} echo payload bytes sent and not \
         retained across the {:.1} s stall",
        fresh.len(),
        continued - stopped
    );
}

#[test]
#[ignore = "real processes and sockets; run by name with --ignored"]
fn finish_drains_once_and_never_appends_after_terminal() {
    let _lock = TableLock::acquire();
    // One SSE response, an event every 250 ms for a minute, open when the seal comes.
    let v = Vertical::new("sparse", json!({"events": 240, "interval_ms": 250}));
    let mut start = v.start(RESERVE);
    let entry = v.entry_id();
    let harness = v.harness_pid();
    let streaming = || {
        v.chain(&entry)
            .streams()
            .values()
            .any(|s| s.source == CaptureSource::WireResponse && s.segments.len() >= 4)
    };
    m2_vertical_support::wait_until("the SSE response, streaming", streaming);

    // The seal while traffic flows: one drain deadline, then the harness is ended.
    let (sealed, said, took) = v.seal(&entry);
    assert!(sealed.success(), "{said}");
    assert!(said.contains("Confirmed"), "{said}");
    assert!(took < Duration::from_secs(25), "the seal took {took:?}");
    let at_finish = v.chain(&entry);
    // The launcher sees its harness gone and finishes too: the daemon answers the committed
    // finish, and nothing its proxy still sends is appended.
    let exit = start.wait(RUN);
    assert!(exit.success(), "{exit}\n{}", start.output());
    support::wait_until_gone(harness);
    let (again, said_again, _) = v.seal(&entry);
    assert!(again.success(), "{said_again}");
    let after = v.chain(&entry);
    assert_eq!(
        after.sha256(),
        at_finish.sha256(),
        "nothing after the terminal"
    );

    let last = after.lines.last().unwrap();
    assert!(matches!(
        last.record,
        Some(CaptureRecord::LocalFinish { .. })
    ));
    assert_eq!(
        after.count(|r| matches!(r, CaptureRecord::LocalFinish { .. })),
        1
    );
    assert!(
        said_again.contains(&format!("its terminal record is {}", last.seq)),
        "{said_again}"
    );
    assert!(after.lines.iter().all(|l| l.protocol_version == 5));
    assert!(v.trace(&entry).join("seal.json").is_file());

    // The proxy's producer was cut at the drain deadline: no close, and its partial tails
    // stay visible.
    let wire = after.first_wire_producer();
    assert_eq!(
        after.count(
            |r| matches!(r, CaptureRecord::ProducerClosed { producer, .. } if *producer == wire)
        ),
        0
    );
    let streams = after.streams();
    let tails: Vec<_> = streams
        .iter()
        .filter(|(k, s)| k.producer == wire && s.source == CaptureSource::WireResponse)
        .map(|(_, s)| (s.end, s.gaps.clone(), s.source_len))
        .collect();
    assert!(
        matches!(tails.as_slice(), [(end, gaps, len)]
            if *end != Some(EndReason::Complete) && !gaps.is_empty() && *len > 0),
        "the open response keeps its retained prefix and a recorded tail: {tails:?}"
    );
    let integrity = assert_incomplete(&v, &entry);
    assert_eq!(integrity["torn_tail"], false);
    assert_eq!(integrity["terminal"]["seq"].as_u64(), Some(last.seq));
    assert!(
        integrity["terminal"]["termination"]
            .get("confirmed")
            .is_some(),
        "{integrity}"
    );
}
