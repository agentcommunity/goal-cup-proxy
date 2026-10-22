//! `proxy::ingest`, the bounded IPC `CaptureSink`, against a test-local ingest endpoint
//! emulator over real loopback TCP (`m2_sender_support`). The emulator reads what crossed the
//! socket with the protocol's `decode_ingest_message` and never answers, so every assertion
//! about order, loss, close and recovery is about bytes that actually left the sender. Nothing
//! here uses the daemon's receiver.
//!
//! Reconnect delays are scaled (`FAST`, the same 1:2:5:10 shape) except where a test checks
//! the contract schedule itself.

use crate::common::m2_sender_support;

use goalcup_cli::proxy::api::CaptureEmitter;
use goalcup_cli::proxy::ingest::{
    CLOSE_RESERVE_BYTES, CONNECT_ATTEMPT_TIMEOUT, CONTROL_RESERVE_BYTES, IngestSecret, IngestSink,
    MAX_CONNECTION_FAILURES, PAYLOAD_QUEUE_BYTES, RECONNECT_DELAYS, SenderOutcome, SinkConfig,
    SinkConfigError, connect_sink,
};
use goalcup_protocol::capture::{
    CaptureFrame, CaptureMeta, CaptureSink, CaptureSource, CaptureTime, ConnectionEvent,
    ConnectionOutcome, EndReason, EpochAdmission, FrameEvent, GapReason, IngestMessage,
    MAX_CAPTURE_QUEUE_BYTES, MAX_HANDSHAKE_BYTES, MAX_INGEST_FRAME_BYTES, MAX_OPEN_CONNECTIONS,
    MAX_SEGMENT_BYTES, StreamKey, SubmitResult, WireIdentity, encode_ingest_message,
};
use m2_sender_support::{Emulator, End, Policy, RefusingPort, ReservedPort, Verdict, from_hex};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const ENTRY: &str = "entry-t12-0001";
const TOKEN: &str = "t12-synthetic-launch-token-00000000";
const WAIT: Duration = Duration::from_secs(10);

const fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// The contract schedule's shape at 1/50 scale.
const FAST: [Duration; 4] = [ms(20), ms(40), ms(100), ms(200)];

/// A distinct, deterministic 128-bit ID.
fn id(n: u128) -> String {
    format!("{n:032x}")
}

fn at(mono_ns: u64) -> CaptureTime {
    CaptureTime {
        wall: "2026-09-28T10:00:00.000Z".to_string(),
        mono_ns,
        clock_id: "t12-test-clock".to_string(),
    }
}

fn meta(connection: &str, exchange: u64) -> CaptureMeta {
    CaptureMeta {
        wire: Some(WireIdentity {
            connection: connection.to_string(),
            exchange,
            message_index: None,
        }),
        label: "POST /v1/responses".to_string(),
        protocol_hint: Some("openai-responses".to_string()),
        method: Some("POST".to_string()),
        host: Some("api.example.test".to_string()),
        path: Some("/v1/responses".to_string()),
        status: None,
        content_type: Some("application/json".to_string()),
    }
}

fn frame(producer: &str, seq: u64, stream: u64, event: FrameEvent) -> CaptureFrame {
    CaptureFrame {
        stream: StreamKey {
            producer: producer.to_string(),
            stream,
        },
        seq,
        observed: at(seq),
        event,
    }
}

fn open(connection: &str) -> FrameEvent {
    FrameEvent::Open {
        source: CaptureSource::WireRequest,
        metadata: meta(connection, 1),
    }
}

fn bytes(offset: u64, data: Vec<u8>) -> FrameEvent {
    FrameEvent::Bytes { offset, data }
}

fn end() -> FrameEvent {
    FrameEvent::End {
        reason: EndReason::Complete,
    }
}

fn conn_open(connection: &str) -> FrameEvent {
    FrameEvent::Connection {
        connection: connection.to_string(),
        event: ConnectionEvent::Open {
            alpn: Some("http/1.1".to_string()),
            already_open: false,
        },
    }
}

fn conn_close(connection: &str) -> FrameEvent {
    FrameEvent::Connection {
        connection: connection.to_string(),
        event: ConnectionEvent::Close {
            outcome: ConnectionOutcome::Complete,
        },
    }
}

/// The encoded (framed) length `encode_ingest_message` gives this frame: the test's own
/// accounting.
fn encoded_len(frame: &CaptureFrame) -> usize {
    encode_ingest_message(&IngestMessage::Frame(frame.clone()))
        .unwrap()
        .len()
}

fn config(addr: std::net::SocketAddr, producer: &str) -> SinkConfig {
    SinkConfig::new(
        addr,
        ENTRY.to_string(),
        IngestSecret::new(TOKEN.to_string()),
        producer.to_string(),
        Duration::from_secs(5),
    )
    .with_reconnect_delays(FAST)
}

/// Polls admission the way the recording proxy's startup does: `Pending` keeps the same ID, no busy
/// loop.
async fn admit(sink: &IngestSink, producer: &str) -> EpochAdmission {
    let deadline = Instant::now() + WAIT;
    loop {
        match sink.try_begin_epoch(producer) {
            EpochAdmission::Pending if Instant::now() < deadline => {
                tokio::time::sleep(ms(5)).await;
            }
            other => return other,
        }
    }
}

/// Waits (without blocking the runtime) until `ready` holds.
async fn eventually(what: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(ms(5)).await;
    }
}

/// Drives `try_recover` at successive boundaries until the emitter records its recovery;
/// returns how many polls it took.
async fn recover(emitter: &CaptureEmitter) -> usize {
    let deadline = Instant::now() + WAIT;
    let mut polls = 0;
    loop {
        polls += 1;
        match emitter.try_recover(at(1_000_000 + polls as u64)) {
            SubmitResult::Queued => return polls,
            SubmitResult::Disabled(GapReason::QueueFull | GapReason::IngestLost) => {}
            other => panic!("recovery refused: {other:?}"),
        }
        assert!(Instant::now() < deadline, "recovery never became ready");
        tokio::time::sleep(ms(10)).await;
    }
}

fn frames(messages: &[IngestMessage]) -> Vec<&CaptureFrame> {
    messages
        .iter()
        .filter_map(|m| match m {
            IngestMessage::Frame(frame) => Some(frame),
            IngestMessage::Close { .. } => None,
        })
        .collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// The private handshake codec and the frame/close encoding reproduce the shared golden
/// byte for byte, over a real socket.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handshake_and_frames_match_the_shared_golden() {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../goalcup-protocol/tests/fixtures/m2-ingest"
    );
    let decoded: serde_json::Value =
        serde_json::from_slice(&std::fs::read(format!("{dir}/golden-stream.json")).unwrap())
            .unwrap();
    let golden = from_hex(&std::fs::read_to_string(format!("{dir}/golden-stream.hex")).unwrap());
    let handshake = &decoded["handshake"];
    let producer = handshake["producer"].as_str().unwrap();
    let messages: Vec<IngestMessage> = serde_json::from_value(decoded["messages"].clone()).unwrap();

    let emulator = Emulator::start();
    let sink = connect_sink(SinkConfig::new(
        emulator.addr(),
        handshake["entry_id"].as_str().unwrap().to_string(),
        IngestSecret::new(handshake["token"].as_str().unwrap().to_string()),
        producer.to_string(),
        Duration::from_secs(5),
    ))
    .unwrap();
    assert_eq!(admit(&sink, producer).await, EpochAdmission::Ready);
    for message in messages {
        let answer = match message {
            IngestMessage::Frame(frame) => sink.try_submit(frame),
            IngestMessage::Close {
                producer,
                final_seq,
            } => sink.try_close_epoch(&producer, final_seq),
        };
        assert_eq!(answer, SubmitResult::Queued);
    }
    let report = sink.shutdown().await;
    assert_eq!(
        report.outcome,
        SenderOutcome::Closed {
            producer: producer.to_string(),
            final_seq: Some(3)
        }
    );
    assert!(report.is_clean(), "{report:?}");
    let conn = emulator.wait_ended(0, WAIT);
    let first_difference = conn.raw.iter().zip(&golden).position(|(a, b)| a != b);
    assert!(
        conn.raw == golden,
        "the sender's bytes differ from the golden: {} vs {} bytes, first difference at {:?}",
        conn.raw.len(),
        golden.len(),
        first_difference
    );
    assert_eq!(conn.verdict(), Verdict::Clean { final_seq: Some(3) });
}

/// The receiver never answers; the sender must not wait for it to. Frames arrive while the
/// producer is still open, and the close drains without any response.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sender_never_waits_for_local_ack() {
    let emulator = Emulator::start();
    let producer = id(0xa1);
    let sink = connect_sink(config(emulator.addr(), &producer)).unwrap();
    assert_eq!(admit(&sink, &producer).await, EpochAdmission::Ready);
    let conn = id(0xc1);
    let events = [
        (0, conn_open(&conn)),
        (1, open(&conn)),
        (1, bytes(0, b"first".to_vec())),
        (1, bytes(5, b"second".to_vec())),
        (1, end()),
    ];
    for (seq, (stream, event)) in events.into_iter().enumerate() {
        assert_eq!(
            sink.try_submit(frame(&producer, seq as u64, stream, event)),
            SubmitResult::Queued
        );
    }
    // Delivered with the producer still open and nothing ever sent back.
    assert!(
        emulator.wait_until(WAIT, |s| s
            .conns
            .first()
            .is_some_and(|c| c.messages.len() == 5)),
        "frames reached the endpoint without any acknowledgement"
    );
    assert_eq!(emulator.conn(0).end, None, "the connection is still open");
    assert_eq!(
        sink.try_close_epoch(&producer, Some(4)),
        SubmitResult::Queued
    );
    let started = Instant::now();
    let report = sink.shutdown().await;
    assert!(started.elapsed() < ms(1000), "{:?}", started.elapsed());
    assert!(report.is_clean(), "{report:?}");
    assert_eq!(report.stats.items_written, 6);
    let conn = emulator.wait_ended(0, WAIT);
    assert_eq!(conn.verdict(), Verdict::Clean { final_seq: Some(4) });
}

/// The endpoint stops reading; submissions keep returning promptly from a
/// relay thread, the sink holds at most its bound (measured independently from what the
/// endpoint later receives), overflow latches `QueueFull` for that epoch, the socket is closed
/// without `Close`, and only a fresh epoch that publishes its Recovery first restores capture.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_full_latches_without_blocking_relay() {
    let emulator = Emulator::start();
    emulator.stall();
    let old = id(0xa2);
    let sink = connect_sink(config(emulator.addr(), &old)).unwrap();
    assert_eq!(admit(&sink, &old).await, EpochAdmission::Ready);
    let dyn_sink: Arc<dyn CaptureSink> = sink.clone();
    let emitter = Arc::new(CaptureEmitter::new(old.clone(), dyn_sink));
    let conn = id(0xc2);

    // A relay thread: synchronous submissions only, guarded so a blocking sink fails the test
    // instead of hanging it.
    let relay = {
        let emitter = Arc::clone(&emitter);
        let old = old.clone();
        let conn = conn.clone();
        tokio::task::spawn_blocking(move || {
            let observed = at(7);
            let stamped = CaptureTime {
                clock_id: old.clone(),
                ..observed.clone()
            };
            let mut accepted: u64 = 0;
            let mut slowest = Duration::ZERO;
            let started = Instant::now();
            let key = emitter
                .open(CaptureSource::WireRequest, meta(&conn, 1), observed.clone())
                .unwrap();
            accepted += encoded_len(&CaptureFrame {
                stream: key.clone(),
                seq: 0,
                observed: stamped.clone(),
                event: open(&conn),
            }) as u64;
            slowest = slowest.max(started.elapsed());
            let data = vec![b'a'; MAX_SEGMENT_BYTES];
            let mut seq = 1u64;
            let mut offset = 0u64;
            loop {
                assert!(seq < 5_000, "the queue never filled");
                let event = bytes(offset, data.clone());
                let len = encoded_len(&CaptureFrame {
                    stream: key.clone(),
                    seq,
                    observed: stamped.clone(),
                    event: event.clone(),
                });
                let t0 = Instant::now();
                let answer = emitter.emit(&key, observed.clone(), event);
                slowest = slowest.max(t0.elapsed());
                match answer {
                    SubmitResult::Queued => {
                        accepted += len as u64;
                        seq += 1;
                        offset += data.len() as u64;
                    }
                    SubmitResult::Disabled(reason) => {
                        return (reason, accepted, len as u64, seq, slowest, key);
                    }
                }
            }
        })
    };
    let (reason, accepted, refused_len, refused_seq, slowest, key) =
        tokio::time::timeout(Duration::from_secs(30), relay)
            .await
            .expect("the relay thread blocked on a full queue")
            .unwrap();
    assert_eq!(reason, GapReason::QueueFull);
    assert!(slowest < ms(250), "a submission took {slowest:?}");

    // Latched for this epoch: nothing more is admitted, and it cannot close cleanly.
    let t0 = Instant::now();
    assert_eq!(
        emitter.emit(&key, at(8), bytes(0, b"late".to_vec())),
        SubmitResult::Disabled(GapReason::QueueFull)
    );
    assert_eq!(
        sink.try_submit(frame(&old, refused_seq, 1, bytes(0, b"late".to_vec()))),
        SubmitResult::Disabled(GapReason::QueueFull)
    );
    assert_eq!(
        sink.try_close_epoch(&old, Some(refused_seq - 1)),
        SubmitResult::Disabled(GapReason::QueueFull)
    );
    assert!(t0.elapsed() < ms(250));
    // The writer stops at the latch and releases even the buffer it was writing.
    eventually("the latched epoch's buffers to be released", || {
        sink.stats().queued_bytes == 0
    })
    .await;
    let stats = sink.stats();
    assert_eq!((stats.losses, stats.queue_full, stats.failures), (1, 1, 0));
    assert!(
        stats.high_water_bytes <= PAYLOAD_QUEUE_BYTES as u64,
        "{stats:?}"
    );

    // The endpoint resumes: it drains the kernel buffers, then EOF with no Close.
    emulator.resume();
    let first = emulator.wait_ended(0, WAIT);
    assert!(!first.has_close(), "a latched epoch never writes Close");
    assert!(
        matches!(first.end, Some(End::Eof | End::TornTail)),
        "{:?}",
        first.end
    );
    assert!(matches!(first.verdict(), Verdict::UnknownLoss(_)));
    let received = frames(&first.messages);
    for (i, frame) in received.iter().enumerate() {
        assert_eq!(frame.seq, i as u64, "a contiguous prefix, nothing replayed");
    }
    // Independent bound: what was accepted but never arrived as a complete item is exactly
    // what the sink held (queued plus in flight) when it latched.
    let held = accepted - first.complete_item_bytes();
    assert!(
        held <= PAYLOAD_QUEUE_BYTES as u64 && held > PAYLOAD_QUEUE_BYTES as u64 - refused_len,
        "held {held}, payload bound {PAYLOAD_QUEUE_BYTES}, refused item {refused_len}"
    );
    assert!(held <= MAX_CAPTURE_QUEUE_BYTES as u64);
    assert!(stats.high_water_bytes >= held, "{stats:?} held {held}");
    eprintln!(
        "T12 evidence queue_full: accepted={accepted} delivered_complete={} held_at_latch={held} \
         high_water={} payload_bound={PAYLOAD_QUEUE_BYTES} refused_item={refused_len} \
         slowest_submit={slowest:?}",
        first.complete_item_bytes(),
        stats.high_water_bytes
    );

    // Only a fresh epoch restores capture, and its Recovery comes first.
    let polls = recover(&emitter).await;
    assert!(polls >= 1);
    let key = emitter
        .open(CaptureSource::WireRequest, meta(&conn, 2), at(9))
        .unwrap();
    assert_ne!(key.producer, old);
    assert_eq!(
        emitter.emit(&key, at(10), bytes(0, b"NEW-payload".to_vec())),
        SubmitResult::Queued
    );
    // A stale frame of the old epoch is refused and never reaches the new one.
    assert_eq!(
        sink.try_submit(frame(&old, refused_seq, 1, bytes(0, b"OLD-stale".to_vec()))),
        SubmitResult::Disabled(GapReason::ProducerLost)
    );
    assert_eq!(emitter.emit(&key, at(11), end()), SubmitResult::Queued);
    assert_eq!(emitter.try_close(at(12)), SubmitResult::Queued);
    let report = sink.shutdown().await;
    assert!(matches!(report.outcome, SenderOutcome::Closed { .. }));
    assert!(!report.is_clean(), "the earlier loss stays in the report");

    let second = emulator.wait_ended(1, WAIT);
    assert_eq!(second.producer(), key.producer);
    let second_frames = frames(&second.messages);
    assert!(matches!(
        &second_frames[0].event,
        FrameEvent::Recovery { previous_producer, reason: GapReason::QueueFull }
            if *previous_producer == old
    ));
    assert!(
        second_frames
            .iter()
            .all(|f| f.stream.producer == key.producer)
    );
    assert!(!contains(&second.raw, b"OLD-stale"));
    assert_eq!(second.verdict(), Verdict::Clean { final_seq: Some(3) });
    assert_eq!(
        emulator.max_open(),
        1,
        "one writer, one connection at a time"
    );
}

/// The endpoint drops the connection in the middle of an item. The next epoch is a new
/// connection with a new ID whose Recovery precedes its payload; nothing old is resent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnect_reconnects_fresh_epoch_without_replay() {
    let emulator = Emulator::start();
    emulator.set_policy(Policy::CloseMidItem(3));
    let old = id(0xa3);
    let sink = connect_sink(config(emulator.addr(), &old)).unwrap();
    assert_eq!(admit(&sink, &old).await, EpochAdmission::Ready);
    let dyn_sink: Arc<dyn CaptureSink> = sink.clone();
    let emitter = CaptureEmitter::new(old.clone(), dyn_sink);
    let conn = id(0xc3);
    let key = emitter
        .open(CaptureSource::WireRequest, meta(&conn, 1), at(1))
        .unwrap();
    assert_eq!(
        emitter.emit(&key, at(2), bytes(0, b"OLD-1".to_vec())),
        SubmitResult::Queued
    );
    assert_eq!(
        emitter.emit(&key, at(3), bytes(5, b"OLD-2".to_vec())),
        SubmitResult::Queued
    );
    let mut long = b"OLD-3-".to_vec();
    long.extend(std::iter::repeat_n(b'x', 4096));
    assert_eq!(
        emitter.emit(&key, at(4), bytes(10, long)),
        SubmitResult::Queued
    );
    let first = emulator.wait_ended(0, WAIT);
    assert_eq!(first.end, Some(End::ClosedByEndpoint));
    assert_eq!(
        first.messages.len(),
        3,
        "three items, then half of the fourth"
    );
    eventually("the sender to see the loss", || sink.stats().failures == 1).await;
    emulator.set_policy(Policy::Serve);
    assert_eq!(
        emitter.emit(&key, at(5), bytes(4112, b"OLD-4".to_vec())),
        SubmitResult::Disabled(GapReason::IngestLost)
    );

    recover(&emitter).await;
    let fresh = emitter
        .open(CaptureSource::WireRequest, meta(&conn, 2), at(6))
        .unwrap();
    assert_eq!(
        emitter.emit(&fresh, at(7), bytes(0, b"NEW-1".to_vec())),
        SubmitResult::Queued
    );
    assert_eq!(emitter.emit(&fresh, at(8), end()), SubmitResult::Queued);
    assert!(emulator.wait_until(WAIT, |s| {
        s.conns.get(1).is_some_and(|c| c.messages.len() == 4)
    }));

    let second = emulator.conn(1);
    assert_eq!(emulator.accepted(), 2);
    assert_eq!(
        emulator.max_open(),
        1,
        "the new connection opened after the old one ended"
    );
    assert!(second.accepted_at >= first.ended_at.unwrap());
    assert_eq!(second.producer(), fresh.producer);
    assert_ne!(fresh.producer, old);
    let got = frames(&second.messages);
    assert_eq!(got[0].seq, 0);
    assert!(matches!(
        &got[0].event,
        FrameEvent::Recovery { previous_producer, reason: GapReason::IngestLost }
            if *previous_producer == old
    ));
    assert!(got.iter().all(|f| f.stream.producer == fresh.producer));
    assert!(
        !contains(&second.raw, b"OLD-"),
        "no old data is retransmitted"
    );
    assert_eq!(
        String::from_utf8_lossy(&second.raw)
            .matches(old.as_str())
            .count(),
        1,
        "the old ID appears only as Recovery's previous_producer"
    );
    assert_eq!(sink.stats().handshakes_sent, 2);
}

/// Only a drained Close followed by EOF is clean. A crash after identical payload frames, a
/// wrong final_seq and a None close after frames are not; a None close of zero frames is.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn producer_close_distinguishes_normal_eof() {
    let emulator = Emulator::start();
    let payload = |producer: &str| {
        let conn = id(0xc4);
        vec![
            frame(producer, 0, 1, open(&conn)),
            frame(producer, 1, 1, bytes(0, b"body".to_vec())),
            frame(producer, 2, 1, end()),
        ]
    };
    let events = |conn: &m2_sender_support::Conn| -> Vec<(u64, FrameEvent)> {
        frames(&conn.messages)
            .iter()
            .map(|f| (f.seq, f.event.clone()))
            .collect()
    };

    // Clean: frames, ordered Close, EOF.
    let clean = id(0xa4);
    let sink = connect_sink(config(emulator.addr(), &clean)).unwrap();
    assert_eq!(admit(&sink, &clean).await, EpochAdmission::Ready);
    for f in payload(&clean) {
        assert_eq!(sink.try_submit(f), SubmitResult::Queued);
    }
    assert_eq!(sink.try_close_epoch(&clean, Some(2)), SubmitResult::Queued);
    let report = sink.shutdown().await;
    assert_eq!(
        report.outcome,
        SenderOutcome::Closed {
            producer: clean.clone(),
            final_seq: Some(2)
        }
    );
    let clean_conn = emulator.wait_ended(0, WAIT);
    assert_eq!(clean_conn.verdict(), Verdict::Clean { final_seq: Some(2) });
    assert!(matches!(
        clean_conn.messages.last(),
        Some(IngestMessage::Close {
            final_seq: Some(2),
            ..
        })
    ));

    // Crash: the identical frames arrive, then the process loses the sink without a close.
    let crashed = id(0xa5);
    let sink = connect_sink(config(emulator.addr(), &crashed)).unwrap();
    assert_eq!(admit(&sink, &crashed).await, EpochAdmission::Ready);
    for f in payload(&crashed) {
        assert_eq!(sink.try_submit(f), SubmitResult::Queued);
    }
    assert!(emulator.wait_until(WAIT, |s| {
        s.conns.get(1).is_some_and(|c| c.messages.len() == 3)
    }));
    drop(sink);
    let crash_conn = emulator.wait_ended(1, WAIT);
    assert_eq!(crash_conn.end, Some(End::Eof));
    assert_eq!(events(&crash_conn), events(&clean_conn)[..3].to_vec());
    assert!(matches!(crash_conn.verdict(), Verdict::UnknownLoss(_)));

    // A close naming the wrong sequence is refused locally and never written.
    let wrong = id(0xa6);
    let sink = connect_sink(config(emulator.addr(), &wrong)).unwrap();
    assert_eq!(admit(&sink, &wrong).await, EpochAdmission::Ready);
    for f in payload(&wrong) {
        assert_eq!(sink.try_submit(f), SubmitResult::Queued);
    }
    assert_eq!(
        sink.try_close_epoch(&wrong, Some(1)),
        SubmitResult::Disabled(GapReason::LimitExceeded)
    );
    assert_eq!(
        sink.try_submit(frame(&wrong, 3, 0, conn_open(&id(0xc5)))),
        SubmitResult::Disabled(GapReason::LimitExceeded)
    );
    let report = sink.shutdown().await;
    assert_eq!(
        report.outcome,
        SenderOutcome::CaptureLost(GapReason::LimitExceeded)
    );
    let wrong_conn = emulator.wait_ended(2, WAIT);
    assert!(!wrong_conn.has_close());
    assert!(matches!(wrong_conn.verdict(), Verdict::UnknownLoss(_)));

    // None after frames is the same lie.
    let none_after = id(0xa7);
    let sink = connect_sink(config(emulator.addr(), &none_after)).unwrap();
    assert_eq!(admit(&sink, &none_after).await, EpochAdmission::Ready);
    for f in payload(&none_after) {
        assert_eq!(sink.try_submit(f), SubmitResult::Queued);
    }
    assert_eq!(
        sink.try_close_epoch(&none_after, None),
        SubmitResult::Disabled(GapReason::LimitExceeded)
    );
    drop(sink);
    assert!(!emulator.wait_ended(3, WAIT).has_close());

    // Shutdown without a close: the frames are written, then EOF without Close.
    let unclosed = id(0xa9a);
    let sink = connect_sink(config(emulator.addr(), &unclosed)).unwrap();
    assert_eq!(admit(&sink, &unclosed).await, EpochAdmission::Ready);
    for f in payload(&unclosed) {
        assert_eq!(sink.try_submit(f), SubmitResult::Queued);
    }
    let report = sink.shutdown().await;
    assert_eq!(report.outcome, SenderOutcome::NotClosed);
    assert!(!report.is_clean());
    let unclosed_conn = emulator.wait_ended(4, WAIT);
    assert_eq!(events(&unclosed_conn), events(&clean_conn)[..3].to_vec());
    assert!(matches!(unclosed_conn.verdict(), Verdict::UnknownLoss(_)));

    // None for a producer that admitted nothing is a clean zero-frame close.
    let empty = id(0xa8);
    let sink = connect_sink(config(emulator.addr(), &empty)).unwrap();
    assert_eq!(admit(&sink, &empty).await, EpochAdmission::Ready);
    assert_eq!(sink.try_close_epoch(&empty, None), SubmitResult::Queued);
    let report = sink.shutdown().await;
    assert!(report.is_clean(), "{report:?}");
    let empty_conn = emulator.wait_ended(5, WAIT);
    assert_eq!(empty_conn.verdict(), Verdict::Clean { final_seq: None });
    assert_eq!(empty_conn.messages.len(), 1);
}

/// Forwards to the IPC sink. Once armed, it holds its next `Ready` answer until the endpoint
/// has closed that epoch's connection and the sink has seen the loss, so the emitter's
/// Recovery is submitted into an epoch already known dead, deterministically. It counts every
/// admission poll.
struct KillAfterReady {
    inner: Arc<IngestSink>,
    emulator: Arc<Emulator>,
    armed: AtomicBool,
    fired: AtomicBool,
    begins: AtomicUsize,
}

impl CaptureSink for KillAfterReady {
    fn try_submit(&self, frame: CaptureFrame) -> SubmitResult {
        self.inner.try_submit(frame)
    }

    fn try_begin_epoch(&self, producer: &str) -> EpochAdmission {
        self.begins.fetch_add(1, Ordering::SeqCst);
        let answer = self.inner.try_begin_epoch(producer);
        if answer == EpochAdmission::Ready && self.armed.swap(false, Ordering::SeqCst) {
            let failures = self.inner.stats().failures;
            // `Ready` means the handshake was written, not that the endpoint has accepted the
            // connection yet: the one to close is the one whose handshake names this producer.
            assert!(self.emulator.wait_until(WAIT, |s| {
                s.conns.iter().any(|c| c.producer() == producer)
            }));
            let index = self
                .emulator
                .conns()
                .iter()
                .position(|c| c.producer() == producer)
                .unwrap();
            self.emulator.close_connection(index);
            let deadline = Instant::now() + WAIT;
            while self.inner.stats().failures == failures {
                assert!(Instant::now() < deadline, "the sender never saw the close");
                std::thread::sleep(ms(2));
            }
            self.fired.store(true, Ordering::SeqCst);
        }
        answer
    }

    fn try_close_epoch(&self, producer: &str, final_seq: Option<u64>) -> SubmitResult {
        self.inner.try_close_epoch(producer, final_seq)
    }
}

/// When the socket of a just-`Ready` epoch is already known dead, the emitter's
/// Recovery is refused (`IngestLost`), never answered `Queued` and dropped, so the emitter
/// ends `Disabled` and never names that unpersisted producer in a later Recovery.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_into_a_dead_ready_epoch_disables_the_emitter() {
    let emulator = Arc::new(Emulator::start());
    let first = id(0xbd);
    let sink = connect_sink(config(emulator.addr(), &first)).unwrap();
    assert_eq!(admit(&sink, &first).await, EpochAdmission::Ready);
    let gate = Arc::new(KillAfterReady {
        inner: Arc::clone(&sink),
        emulator: Arc::clone(&emulator),
        armed: AtomicBool::new(false),
        fired: AtomicBool::new(false),
        begins: AtomicUsize::new(0),
    });
    let dyn_sink: Arc<dyn CaptureSink> = gate.clone();
    let emitter = Arc::new(CaptureEmitter::new(first.clone(), dyn_sink));
    let conn = id(0xc9);
    assert_eq!(
        emitter.connection_event(
            &conn,
            at(1),
            ConnectionEvent::Open {
                alpn: Some("http/1.1".to_string()),
                already_open: false
            }
        ),
        SubmitResult::Queued
    );
    assert!(emulator.wait_until(WAIT, |s| {
        s.conns.first().is_some_and(|c| c.messages.len() == 1)
    }));
    emulator.close_connection(0);
    eventually("the sender to see the loss", || sink.stats().failures == 1).await;
    assert_eq!(
        emitter.open(CaptureSource::WireRequest, meta(&conn, 1), at(2)),
        Err(GapReason::IngestLost),
        "the first loss is recoverable"
    );

    gate.armed.store(true, Ordering::SeqCst);
    let answer = {
        let emitter = Arc::clone(&emitter);
        let gate = Arc::clone(&gate);
        tokio::task::spawn_blocking(move || {
            let deadline = Instant::now() + WAIT;
            loop {
                let answer = emitter.try_recover(at(3));
                if gate.fired.load(Ordering::SeqCst) {
                    return answer;
                }
                assert!(
                    Instant::now() < deadline,
                    "the fresh epoch never became ready"
                );
                std::thread::sleep(ms(10));
            }
        })
        .await
        .unwrap()
    };
    assert_eq!(answer, SubmitResult::Disabled(GapReason::IngestLost));

    // The emitter is disabled for good: no further admission poll, nothing recovers.
    let polls = gate.begins.load(Ordering::SeqCst);
    let attempts = sink.stats().connection_attempts;
    assert_eq!(
        emitter.try_recover(at(4)),
        SubmitResult::Disabled(GapReason::IngestLost)
    );
    assert_eq!(
        emitter.open(CaptureSource::WireRequest, meta(&conn, 2), at(5)),
        Err(GapReason::IngestLost)
    );
    tokio::time::sleep(ms(300)).await;
    assert_eq!(gate.begins.load(Ordering::SeqCst), polls);
    assert_eq!(sink.stats().connection_attempts, attempts);
    // The refused Recovery never reached the wire, and was never counted as queued.
    let second = emulator.wait_ended(1, WAIT);
    assert!(second.messages.is_empty(), "{:?}", second.messages);
    let stats = sink.stats();
    assert_eq!(
        (stats.frames_queued, stats.losses, stats.failures),
        (1, 2, 2)
    );
    assert_eq!(emulator.accepted(), 2);
}

/// While the fresh attempt waits out the contract's first reconnect delay (1 s), many
/// readiness polls keep one prospective ID and start no second attempt; exactly one Recovery
/// opens the single accepted fresh epoch, and no stream ID was consumed meanwhile.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_epoch_id_survives_readiness_polls() {
    let emulator = Emulator::start();
    let old = id(0xa9);
    let config = SinkConfig::new(
        emulator.addr(),
        ENTRY.to_string(),
        IngestSecret::new(TOKEN.to_string()),
        old.clone(),
        Duration::from_secs(5),
    );
    assert_eq!(config.reconnect_delays(), RECONNECT_DELAYS);
    let sink = connect_sink(config).unwrap();
    assert_eq!(admit(&sink, &old).await, EpochAdmission::Ready);
    let dyn_sink: Arc<dyn CaptureSink> = sink.clone();
    let emitter = CaptureEmitter::new(old.clone(), dyn_sink);
    let conn = id(0xc9);
    assert_eq!(
        emitter.connection_event(
            &conn,
            at(1),
            ConnectionEvent::Open {
                alpn: Some("http/1.1".to_string()),
                already_open: false
            }
        ),
        SubmitResult::Queued
    );
    assert!(emulator.wait_until(WAIT, |s| {
        s.conns.first().is_some_and(|c| c.messages.len() == 1)
    }));
    emulator.close_connection(0);
    eventually("the sender to see the loss", || sink.stats().failures == 1).await;
    assert_eq!(
        emitter.open(CaptureSource::WireRequest, meta(&conn, 1), at(2)),
        Err(GapReason::IngestLost)
    );

    let polls = recover(&emitter).await;
    eprintln!(
        "T12 evidence pending: {polls} readiness polls, {:?}",
        sink.stats()
    );
    assert!(
        polls >= 10,
        "only {polls} polls across a 1 s reconnect delay"
    );
    let first = emulator.conn(0);
    assert!(emulator.wait_until(WAIT, |s| {
        s.conns.get(1).is_some_and(|c| c.messages.len() == 2)
    }));
    let second = emulator.conn(1);
    assert!(
        second.accepted_at.duration_since(first.ended_at.unwrap()) >= ms(990),
        "the first reconnect waits the contract's 1 s"
    );
    let stats = sink.stats();
    assert_eq!(emulator.accepted(), 2);
    assert_eq!((stats.connection_attempts, stats.handshakes_sent), (2, 2));
    let got = frames(&second.messages);
    assert_eq!(
        got.iter()
            .filter(|f| matches!(f.event, FrameEvent::Recovery { .. }))
            .count(),
        1
    );
    assert!(
        matches!(&got[0].event, FrameEvent::Recovery { previous_producer, .. } if *previous_producer == old)
    );
    assert!(matches!(
        &got[1].event,
        FrameEvent::Connection { connection, event: ConnectionEvent::Open { already_open: true, .. } }
            if *connection == conn
    ));
    let key = emitter
        .open(CaptureSource::WireRequest, meta(&conn, 2), at(3))
        .unwrap();
    assert_eq!(key.stream, 1, "no stream ID was consumed by the polls");
    assert_eq!(key.producer, second.producer());
}

/// The endpoint consumes the handshake and closes before the caller heard `Ready`: that ID
/// is spent (`RetryFresh`, every time), and a fresh ID is `Pending` only for its own attempt.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handshake_ambiguity_requires_fresh_id() {
    let emulator = Emulator::start();
    emulator.set_policy(Policy::CloseAfterHandshake);
    let first = id(0xaa);
    let sink = connect_sink(config(emulator.addr(), &first).with_reconnect_delays([
        ms(300),
        ms(300),
        ms(300),
        ms(300),
    ]))
    .unwrap();
    let ended = emulator.wait_ended(0, WAIT);
    assert_eq!(
        ended.producer(),
        first,
        "the handshake reached the endpoint"
    );
    eventually("the sender to see the close", || sink.stats().failures == 1).await;
    emulator.set_policy(Policy::Serve);

    assert_eq!(sink.try_begin_epoch(&first), EpochAdmission::RetryFresh);
    assert_eq!(sink.try_begin_epoch(&first), EpochAdmission::RetryFresh);
    let fresh = id(0xab);
    let mut answers = Vec::new();
    loop {
        let answer = sink.try_begin_epoch(&fresh);
        answers.push(answer);
        if answer != EpochAdmission::Pending {
            break;
        }
        tokio::time::sleep(ms(10)).await;
    }
    assert_eq!(answers.last(), Some(&EpochAdmission::Ready));
    assert!(answers.len() >= 5, "{answers:?}");
    assert!(
        answers[..answers.len() - 1]
            .iter()
            .all(|a| *a == EpochAdmission::Pending)
    );
    assert_eq!(sink.try_begin_epoch(&first), EpochAdmission::RetryFresh);
    assert!(emulator.wait_until(WAIT, |s| s.conns.len() == 2
        && s.conns[1].handshake.is_some()));
    assert_eq!(emulator.conn(1).producer(), fresh);
    let stats = sink.stats();
    assert_eq!(
        (
            stats.handshakes_sent,
            stats.failures,
            stats.epochs_ready,
            stats.losses
        ),
        (2, 1, 2, 0)
    );
}

/// A failure before any handshake byte keeps the same ID: polls stay `Pending`, never
/// `RetryFresh`, and the same ID becomes ready once the endpoint appears. Here the endpoint
/// silently drops the connect, so the attempt is bounded by `CONNECT_ATTEMPT_TIMEOUT`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pre_handshake_connect_failure_keeps_the_same_id() {
    let port = ReservedPort::new();
    let producer = id(0xac);
    let started = Instant::now();
    let sink = connect_sink(config(port.addr(), &producer).with_reconnect_delays([
        ms(50),
        ms(100),
        ms(250),
        ms(500),
    ]))
    .unwrap();
    while sink.stats().failures == 0 {
        assert_eq!(sink.try_begin_epoch(&producer), EpochAdmission::Pending);
        assert!(started.elapsed() < WAIT, "a hung connect was never bounded");
        tokio::time::sleep(ms(20)).await;
    }
    let bounded = started.elapsed();
    assert!(
        bounded >= CONNECT_ATTEMPT_TIMEOUT - ms(20) && bounded < CONNECT_ATTEMPT_TIMEOUT + ms(500),
        "the hung connect failed after {bounded:?}"
    );
    assert_eq!(sink.try_begin_epoch(&producer), EpochAdmission::Pending);
    let emulator = Emulator::from_listener(port.listen());
    assert_eq!(admit(&sink, &producer).await, EpochAdmission::Ready);
    let stats = sink.stats();
    assert_eq!(
        stats.handshakes_sent, 1,
        "only the successful attempt wrote a handshake"
    );
    assert_eq!(stats.failures + 1, stats.connection_attempts);
    assert!(emulator.wait_until(WAIT, |s| {
        s.conns.first().is_some_and(|c| c.handshake.is_some())
    }));
    assert_eq!(emulator.accepted(), 1);
    assert_eq!(emulator.conn(0).producer(), producer);
}

/// A receiver that refuses the credential just closes. The sender reports unknown loss
/// (`IngestLost`) and nothing else, and stops at the retry cap.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_eof_does_not_invent_rejection_reason() {
    let emulator = Emulator::start();
    emulator.expect_token("a-different-token");
    let mut producer = id(0xb00);
    let sink = connect_sink(config(emulator.addr(), &producer)).unwrap();
    let mut reasons = Vec::new();
    let mut next = 0xb01u128;
    let deadline = Instant::now() + WAIT;
    let terminal = loop {
        assert!(Instant::now() < deadline, "the retry budget never ran out");
        match sink.try_begin_epoch(&producer) {
            EpochAdmission::Pending => tokio::time::sleep(ms(5)).await,
            EpochAdmission::RetryFresh => {
                producer = id(next);
                next += 1;
            }
            EpochAdmission::Ready => {
                // Like an emitter: keep submitting until a refusal says the epoch is gone.
                let mut seq = 0;
                loop {
                    assert!(Instant::now() < deadline, "the refusal was never seen");
                    match sink.try_submit(frame(&producer, seq, 0, conn_open(&id(0xc6)))) {
                        SubmitResult::Queued => seq += 1,
                        SubmitResult::Disabled(reason) => {
                            reasons.push(reason);
                            break;
                        }
                    }
                    tokio::time::sleep(ms(5)).await;
                }
                producer = id(next);
                next += 1;
            }
            EpochAdmission::Disabled(reason) => break reason,
        }
    };
    assert_eq!(terminal, GapReason::IngestLost);
    assert!(
        reasons.iter().all(|r| *r == GapReason::IngestLost),
        "{reasons:?}"
    );
    let stats = sink.stats();
    assert_eq!(stats.failures, MAX_CONNECTION_FAILURES);
    assert_eq!(emulator.accepted() as u64, MAX_CONNECTION_FAILURES);
    assert!(emulator.conns().iter().all(|c| c.end == Some(End::Refused)));
    let report = sink.shutdown().await;
    assert_eq!(
        report.outcome,
        SenderOutcome::CaptureLost(GapReason::IngestLost)
    );
    let text = format!("{report:?}").to_lowercase();
    for fabricated in ["auth", "storage", "token", "credential", "reject"] {
        assert!(!text.contains(fabricated), "{fabricated} in {text}");
    }
}

/// Every epoch's handshake is written and then its socket is lost before it is handed a single
/// item. The tenth failure in a row disables the sink across all producer IDs; a handshake
/// alone (`Ready`) never reset the count.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ready_then_remote_eof_cannot_reset_retry_budget() {
    let emulator = Emulator::start();
    emulator.set_policy(Policy::CloseAfterHandshake);
    let mut n = 0xd00u128;
    let sink = connect_sink(config(emulator.addr(), &id(n))).unwrap();
    let mut epochs = 0u64;
    let terminal = loop {
        let producer = id(n);
        match admit(&sink, &producer).await {
            // The endpoint may close before the poll that would have said `Ready`: either way
            // the epoch was one handshake and one loss.
            EpochAdmission::Ready | EpochAdmission::RetryFresh => {
                epochs += 1;
                eventually("the endpoint's close", || sink.stats().failures >= epochs).await;
                n += 1;
            }
            EpochAdmission::Disabled(reason) => break reason,
            EpochAdmission::Pending => unreachable!(),
        }
    };
    assert_eq!(terminal, GapReason::IngestLost);
    assert_eq!(
        sink.try_begin_epoch(&id(0xdff)),
        EpochAdmission::Disabled(GapReason::IngestLost)
    );
    let stats = sink.stats();
    assert_eq!(
        (stats.epochs_ready, stats.failures, stats.items_written),
        (10, 10, 0)
    );
    // The schedule held between attempts: 20, 40, 100, then 200 ms.
    let conns = emulator.conns();
    assert_eq!(conns.len(), 10);
    for pair in 1..conns.len() {
        let gap = conns[pair]
            .accepted_at
            .duration_since(conns[pair - 1].ended_at.unwrap());
        // The endpoint stamps its close a moment before the sender can see it.
        assert!(
            gap + ms(2) >= FAST[(pair - 1).min(3)],
            "attempt {pair} came {gap:?} after the loss"
        );
    }
    tokio::time::sleep(ms(400)).await;
    assert_eq!(
        emulator.accepted(),
        10,
        "no attempt after the budget ran out"
    );
}

/// Every epoch takes one item and is closed at once. That is not a connection that worked
/// (the epoch did not stay up `STABLE_EPOCH`): each still counts, and the tenth in a row
/// disables the sink.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_epoch_that_takes_one_item_and_closes_at_once_still_runs_out_at_ten() {
    let emulator = Emulator::start();
    emulator.set_policy(Policy::CloseAfterItems(1));
    let mut n = 0xf00u128;
    let sink = connect_sink(config(emulator.addr(), &id(n))).unwrap();
    let mut readies = 0u64;
    let terminal = loop {
        let producer = id(n);
        match admit(&sink, &producer).await {
            EpochAdmission::Ready => {
                readies += 1;
                let failures = sink.stats().failures;
                assert_eq!(
                    sink.try_submit(frame(&producer, 0, 0, conn_open(&id(0xc9)))),
                    SubmitResult::Queued
                );
                eventually("the endpoint's close", || sink.stats().failures > failures).await;
                n += 1;
            }
            EpochAdmission::RetryFresh => n += 1,
            EpochAdmission::Disabled(reason) => break reason,
            EpochAdmission::Pending => unreachable!(),
        }
    };
    assert_eq!(terminal, GapReason::IngestLost);
    let stats = sink.stats();
    assert_eq!(readies, MAX_CONNECTION_FAILURES);
    assert_eq!(
        (stats.epochs_ready, stats.failures, stats.items_written),
        (10, 10, 10),
        "every epoch took its item"
    );
}

/// Nine failures, one connection that takes an item, nine more failures: the count restarted
/// at the hand-off, so capture is still on (a week of short stalls must not switch the wire
/// leg off for good). Only the tenth in a row ends it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_successful_connection_resets_the_failure_count() {
    let emulator = Emulator::start();
    let mut n = 0xe00u128;
    let sink = connect_sink(config(emulator.addr(), &id(n)).with_stable_after(ms(150))).unwrap();
    // Drive epochs, each lost as soon as its handshake is read, until `failures` failures were
    // counted. An epoch is one handshake and one loss, whether or not the poll that would have
    // said `Ready` came before the loss.
    async fn lose_until(sink: &IngestSink, n: &mut u128, failures: u64) -> Option<GapReason> {
        let (base, mut epochs) = (sink.stats().failures, 0);
        while sink.stats().failures < failures {
            let producer = id(*n);
            *n += 1;
            match admit(sink, &producer).await {
                EpochAdmission::Ready | EpochAdmission::RetryFresh => {
                    epochs += 1;
                    eventually("the endpoint's close", || {
                        sink.stats().failures >= base + epochs
                    })
                    .await;
                }
                EpochAdmission::Disabled(reason) => return Some(reason),
                EpochAdmission::Pending => unreachable!(),
            }
        }
        None
    }
    emulator.set_policy(Policy::CloseAfterHandshake);
    assert_eq!(lose_until(&sink, &mut n, 9).await, None);
    assert_eq!(sink.stats().failures, 9);

    // The connection that works: it takes an item, stays up, and then drops (one more failure,
    // the first since the success).
    emulator.set_policy(Policy::Serve);
    let producer = id(n);
    n += 1;
    assert_eq!(admit(&sink, &producer).await, EpochAdmission::Ready);
    assert_eq!(
        sink.try_submit(frame(&producer, 0, 0, conn_open(&id(0xc8)))),
        SubmitResult::Queued
    );
    eventually("the item", || sink.stats().items_written == 1).await;
    tokio::time::sleep(ms(300)).await;
    emulator.close_connection(emulator.accepted() - 1);
    eventually("the drop", || sink.stats().failures == 10).await;

    // Nine more failures since the success: the sink is still on, and a receiver that stays
    // up is served.
    emulator.set_policy(Policy::CloseAfterHandshake);
    assert_eq!(lose_until(&sink, &mut n, 18).await, None);
    emulator.set_policy(Policy::Serve);
    let producer = id(n);
    n += 1;
    assert_eq!(
        admit(&sink, &producer).await,
        EpochAdmission::Ready,
        "capture is still on after {} failures in all",
        sink.stats().failures
    );

    // The tenth failure since the success is the tenth in a row, and it is over.
    emulator.set_policy(Policy::CloseAfterHandshake);
    emulator.close_connection(emulator.accepted() - 1);
    eventually("the served connection's loss", || {
        sink.stats().failures >= 19
    })
    .await;
    let terminal = lose_until(&sink, &mut n, 100).await;
    assert_eq!(terminal, Some(GapReason::IngestLost));
    let stats = sink.stats();
    assert!(
        (19..=20).contains(&stats.failures),
        "failures: {}",
        stats.failures
    );
}

/// With the endpoint unavailable throughout, the sink retries the same ID on the bounded
/// schedule (no busy loop), stops after the tenth failure, leaves no retry task behind and
/// reports an incomplete capture.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_budget_exhaustion_stays_incomplete() {
    let port = RefusingPort::new();
    let producer = id(0xae);
    let started = Instant::now();
    let sink = connect_sink(config(port.addr(), &producer)).unwrap();
    let terminal = loop {
        match sink.try_begin_epoch(&producer) {
            EpochAdmission::Pending => tokio::time::sleep(ms(5)).await,
            other => break other,
        }
        assert!(started.elapsed() < WAIT, "the budget never ran out");
    };
    let elapsed = started.elapsed();
    eprintln!(
        "T12 evidence budget: disabled after {elapsed:?}, {:?}",
        sink.stats()
    );
    assert_eq!(terminal, EpochAdmission::Disabled(GapReason::IngestLost));
    // Nine pauses between ten attempts: 20 + 40 + 100 + 6 × 200 ms.
    let schedule: Duration = (0..9).map(|i| FAST[i.min(3)]).sum();
    assert!(elapsed >= schedule, "{elapsed:?} < {schedule:?}");
    let stats = sink.stats();
    assert_eq!(
        (
            stats.connection_attempts,
            stats.failures,
            stats.handshakes_sent
        ),
        (10, 10, 0)
    );
    tokio::time::sleep(ms(500)).await;
    assert_eq!(
        sink.stats().connection_attempts,
        10,
        "no retry task outlived the budget"
    );
    let t0 = Instant::now();
    let report = sink.shutdown().await;
    assert!(t0.elapsed() < ms(200), "the task had already ended");
    assert_eq!(
        report.outcome,
        SenderOutcome::CaptureLost(GapReason::IngestLost)
    );
    assert!(!report.is_clean());
    assert_eq!(report.stats.bytes_written, 0);
}

/// Encoded length `target` for a `Bytes` frame, built from 1-digit and 2-digit byte values
/// so any length above the empty frame's is reachable; checked with `encode_ingest_message`.
fn filler(producer: &str, seq: u64, offset: u64, target: usize) -> CaptureFrame {
    let empty = encoded_len(&frame(producer, seq, 1, bytes(offset, Vec::new())));
    let need = target + 1 - empty; // 2 chars per `9,`, 3 per `97,`, minus the last comma
    let nines = (0..3).find(|k| (need - 2 * k).is_multiple_of(3)).unwrap();
    let mut data = vec![9u8; nines];
    data.extend(std::iter::repeat_n(97u8, (need - 2 * nines) / 3));
    let built = frame(producer, seq, 1, bytes(offset, data));
    assert_eq!(encoded_len(&built), target);
    built
}

/// Payload pressure never starves the control lane: with payload exactly at its bound,
/// stream-zero controls and the final Close are still admitted, in order, by the same writer,
/// and nothing follows Close. After a `QueueFull`, the Recovery and the re-observation of
/// every tracked connection are all admitted before new payload.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_controls_survive_payload_pressure() {
    // (a) A clean close with the payload lane full.
    let emulator = Emulator::start();
    emulator.stall();
    let producer = id(0xaf);
    let sink = connect_sink(config(emulator.addr(), &producer)).unwrap();
    assert_eq!(admit(&sink, &producer).await, EpochAdmission::Ready);
    let conn = id(0xc8);
    assert_eq!(
        sink.try_submit(frame(&producer, 0, 1, open(&conn))),
        SubmitResult::Queued
    );
    let big = vec![97u8; MAX_SEGMENT_BYTES];
    let (mut seq, mut offset) = (1u64, 0u64);
    let min_filler = encoded_len(&frame(&producer, 999_999, 1, bytes(999_999_999, vec![9])));
    let end_len = encoded_len(&frame(&producer, 999_999, 1, end()));
    let budget = PAYLOAD_QUEUE_BYTES - end_len;
    loop {
        let queued = sink.stats().queued_bytes as usize;
        let next = frame(&producer, seq, 1, bytes(offset, big.clone()));
        let len = encoded_len(&next);
        if queued + len + min_filler <= budget {
            assert_eq!(sink.try_submit(next), SubmitResult::Queued);
            seq += 1;
            offset += big.len() as u64;
            continue;
        }
        tokio::time::sleep(ms(100)).await;
        if sink.stats().queued_bytes as usize == queued {
            break; // the writer is blocked: the queue no longer drains
        }
    }
    let mut room = budget - sink.stats().queued_bytes as usize;
    let most = encoded_len(&frame(&producer, seq, 1, bytes(offset, big.clone())));
    while room > 0 {
        let take = if room > most {
            (room - min_filler).min(most)
        } else {
            room
        };
        let f = filler(&producer, seq, offset, take);
        let FrameEvent::Bytes { data, .. } = &f.event else {
            unreachable!()
        };
        offset += data.len() as u64;
        assert_eq!(sink.try_submit(f), SubmitResult::Queued);
        seq += 1;
        room -= take;
    }
    assert_eq!(
        sink.try_submit(frame(&producer, seq, 1, end())),
        SubmitResult::Queued
    );
    // Full to within the few digits the End's upper-bound length overestimated.
    let full = sink.stats().queued_bytes as usize;
    assert!(
        full <= PAYLOAD_QUEUE_BYTES && full > PAYLOAD_QUEUE_BYTES - 16,
        "the payload lane is full: {full}"
    );
    // Controls above the payload bound, then the close above the control bound's reserve.
    assert_eq!(
        sink.try_submit(frame(&producer, seq + 1, 0, conn_open(&id(0xca)))),
        SubmitResult::Queued
    );
    assert_eq!(
        sink.try_submit(frame(&producer, seq + 2, 0, conn_close(&id(0xca)))),
        SubmitResult::Queued
    );
    let controls = sink.stats().queued_bytes as usize;
    assert!(
        controls > PAYLOAD_QUEUE_BYTES && controls <= MAX_CAPTURE_QUEUE_BYTES - CLOSE_RESERVE_BYTES,
        "controls use the reserve above payload, short of the close reserve: {controls}"
    );
    assert_eq!(
        sink.try_close_epoch(&producer, Some(seq + 2)),
        SubmitResult::Queued
    );
    let stats = sink.stats();
    assert!(
        stats.high_water_bytes as usize <= MAX_CAPTURE_QUEUE_BYTES,
        "{stats:?}"
    );
    assert_eq!(
        sink.try_submit(frame(&producer, seq + 3, 0, conn_open(&id(0xcb)))),
        SubmitResult::Disabled(GapReason::ShutdownDeadline),
        "nothing follows Close"
    );
    emulator.resume();
    let report = sink.shutdown().await;
    assert!(report.is_clean(), "{report:?}");
    let first = emulator.wait_ended(0, WAIT);
    assert_eq!(
        first.verdict(),
        Verdict::Clean {
            final_seq: Some(seq + 2)
        }
    );
    let n = first.messages.len();
    assert!(matches!(&first.messages[n - 3], IngestMessage::Frame(f) if f.stream.stream == 0));
    assert!(matches!(&first.messages[n - 2], IngestMessage::Frame(f) if f.stream.stream == 0));
    assert!(matches!(first.messages[n - 1], IngestMessage::Close { .. }));

    // (b) After QueueFull, Recovery plus every re-observation fits before new payload.
    let emulator = Emulator::start();
    let old = id(0xb1);
    let sink = connect_sink(config(emulator.addr(), &old)).unwrap();
    assert_eq!(admit(&sink, &old).await, EpochAdmission::Ready);
    let dyn_sink: Arc<dyn CaptureSink> = sink.clone();
    let emitter = CaptureEmitter::new(old.clone(), dyn_sink);
    let tracked: Vec<String> = (0..MAX_OPEN_CONNECTIONS as u128)
        .map(|i| id(0xe000 + i))
        .collect();
    for (i, connection) in tracked.iter().enumerate() {
        assert_eq!(
            emitter.connection_event(
                connection,
                at(i as u64),
                ConnectionEvent::Open {
                    alpn: Some("http/1.1".to_string()),
                    already_open: false
                }
            ),
            SubmitResult::Queued
        );
    }
    assert!(emulator.wait_until(WAIT, |s| {
        s.conns
            .first()
            .is_some_and(|c| c.messages.len() == MAX_OPEN_CONNECTIONS)
    }));
    emulator.stall();
    let key = emitter
        .open(CaptureSource::WireRequest, meta(&tracked[0], 1), at(500))
        .unwrap();
    let mut offset = 0u64;
    let refused = loop {
        match emitter.emit(&key, at(501), bytes(offset, big.clone())) {
            SubmitResult::Queued => offset += big.len() as u64,
            SubmitResult::Disabled(reason) => break reason,
        }
    };
    assert_eq!(refused, GapReason::QueueFull);
    emulator.resume();
    emulator.wait_ended(0, WAIT);
    recover(&emitter).await;
    let fresh = emitter
        .open(CaptureSource::WireRequest, meta(&tracked[1], 2), at(600))
        .unwrap();
    assert_eq!(emitter.emit(&fresh, at(601), end()), SubmitResult::Queued);
    assert_eq!(emitter.try_close(at(602)), SubmitResult::Queued);
    let report = sink.shutdown().await;
    assert!(
        matches!(report.outcome, SenderOutcome::Closed { .. }),
        "{report:?}"
    );
    let second = emulator.wait_ended(1, WAIT);
    let got = frames(&second.messages);
    assert!(matches!(
        &got[0].event,
        FrameEvent::Recovery { previous_producer, reason: GapReason::QueueFull }
            if *previous_producer == old
    ));
    for (i, connection) in tracked.iter().enumerate() {
        assert!(matches!(
            &got[1 + i].event,
            FrameEvent::Connection { connection: c, event: ConnectionEvent::Open { already_open: true, .. } }
                if c == connection
        ));
    }
    assert!(matches!(
        got[1 + MAX_OPEN_CONNECTIONS].event,
        FrameEvent::Open { .. }
    ));
    assert_eq!(
        second.verdict(),
        Verdict::Clean {
            final_seq: Some(1 + MAX_OPEN_CONNECTIONS as u64 + 1)
        }
    );
}

/// A frame over `MAX_INGEST_FRAME_BYTES` and over-limit handshake values are refused before
/// anything is written, and no error or debug text repeats the input.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_frame_and_handshake_are_refused_before_write() {
    let marker = b"t12-oversize-marker";
    let emulator = Emulator::start();
    let producer = id(0xb2);
    let sink = connect_sink(config(emulator.addr(), &producer)).unwrap();
    assert_eq!(admit(&sink, &producer).await, EpochAdmission::Ready);
    let mut data = marker.repeat(MAX_INGEST_FRAME_BYTES / marker.len() + 1);
    data.truncate(MAX_INGEST_FRAME_BYTES + 1);
    let answer = sink.try_submit(frame(&producer, 0, 0, conn_open(&id(0xcc))));
    assert_eq!(answer, SubmitResult::Queued);
    // Written before the refusal: a terminal latch discards whatever is still queued.
    assert!(emulator.wait_until(WAIT, |s| {
        s.conns.first().is_some_and(|c| c.messages.len() == 1)
    }));
    let huge = frame(&producer, 1, 1, bytes(0, data));
    assert_eq!(
        sink.try_submit(huge),
        SubmitResult::Disabled(GapReason::LimitExceeded)
    );
    assert_eq!(
        sink.try_submit(frame(&producer, 1, 0, conn_close(&id(0xcc)))),
        SubmitResult::Disabled(GapReason::LimitExceeded),
        "locally invalid framing is terminal"
    );
    assert_eq!(
        sink.try_begin_epoch(&id(0xb3)),
        EpochAdmission::Disabled(GapReason::LimitExceeded)
    );
    let report = sink.shutdown().await;
    assert!(!format!("{report:?}").contains("t12-oversize-marker"));
    let conn = emulator.wait_ended(0, WAIT);
    assert_eq!(
        conn.messages.len(),
        1,
        "only the valid frame before it was written"
    );
    assert!(!contains(&conn.raw, marker));
    assert!(!conn.has_close());

    // Handshake values: over the limit is refused with no connection; exactly at it fits.
    let before = emulator.accepted();
    let sentinel = "t12-handshake-sentinel";
    let too_long = sentinel.repeat(MAX_HANDSHAKE_BYTES / sentinel.len() + 1);
    for (entry, token) in [
        (ENTRY.to_string(), too_long.clone()),
        (too_long.clone(), TOKEN.to_string()),
    ] {
        let error = connect_sink(SinkConfig::new(
            emulator.addr(),
            entry,
            IngestSecret::new(token),
            id(0xb4),
            Duration::from_secs(1),
        ))
        .unwrap_err();
        assert_eq!(
            error,
            SinkConfigError::HandshakeTooLarge {
                limit: MAX_HANDSHAKE_BYTES
            }
        );
        assert!(!error.to_string().contains(sentinel) && !format!("{error:?}").contains(sentinel));
    }
    let fixed = r#"{"entry_id":"","producer":"","token":""}"#.len() + ENTRY.len() + 32;
    let exact = "k".repeat(MAX_HANDSHAKE_BYTES - fixed);
    let sink = connect_sink(SinkConfig::new(
        emulator.addr(),
        ENTRY.to_string(),
        IngestSecret::new(exact),
        id(0xb5),
        Duration::from_secs(1),
    ))
    .unwrap();
    assert_eq!(admit(&sink, &id(0xb5)).await, EpochAdmission::Ready);
    drop(sink);
    let conn = emulator.wait_ended(before, WAIT);
    assert_eq!(
        emulator.accepted(),
        before + 1,
        "the refused configurations never connected"
    );
    assert_eq!(conn.raw.len(), 4 + MAX_HANDSHAKE_BYTES);
}

/// Concurrent relays share one emitter: the single writer delivers one producer's sequence,
/// in order, never renumbered, and the close names its last frame.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_emitter_preserves_one_producer_sequence() {
    let emulator = Emulator::start();
    let producer = id(0xb6);
    let sink = connect_sink(config(emulator.addr(), &producer)).unwrap();
    assert_eq!(admit(&sink, &producer).await, EpochAdmission::Ready);
    let dyn_sink: Arc<dyn CaptureSink> = sink.clone();
    let emitter = CaptureEmitter::new(producer.clone(), dyn_sink);
    const THREADS: u128 = 8;
    const ROUNDS: u64 = 40;
    std::thread::scope(|scope| {
        for t in 0..THREADS {
            let emitter = &emitter;
            scope.spawn(move || {
                let conn = id(0xf000 + t);
                for round in 0..ROUNDS {
                    let key = emitter
                        .open(
                            CaptureSource::WireRequest,
                            meta(&conn, round + 1),
                            at(round),
                        )
                        .unwrap();
                    let body = format!("t{t}-r{round}").into_bytes();
                    assert_eq!(
                        emitter.emit(&key, at(round), bytes(0, body)),
                        SubmitResult::Queued
                    );
                    assert_eq!(emitter.emit(&key, at(round), end()), SubmitResult::Queued);
                }
            });
        }
    });
    assert_eq!(emitter.try_close(at(9_999)), SubmitResult::Queued);
    let report = sink.shutdown().await;
    assert!(report.is_clean(), "{report:?}");
    let total = THREADS as u64 * ROUNDS * 3;
    let conn = emulator.wait_ended(0, WAIT);
    assert_eq!(
        conn.verdict(),
        Verdict::Clean {
            final_seq: Some(total - 1)
        }
    );
    let got = frames(&conn.messages);
    assert_eq!(got.len() as u64, total);
    assert!(got.iter().all(|f| f.stream.producer == producer));
    let opened: Vec<u64> = got
        .iter()
        .filter(|f| matches!(f.event, FrameEvent::Open { .. }))
        .map(|f| f.stream.stream)
        .collect();
    assert_eq!(opened, (1..=THREADS as u64 * ROUNDS).collect::<Vec<_>>());
}

/// No diagnostic surface of the sink shows the secret; only the handshake carries it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_diagnostic_state_exposes_the_secret() {
    let sentinel = "T12-SECRET-SENTINEL-5f0c9a";
    let emulator = Emulator::start();
    let producer = id(0xb7);
    let secret = || IngestSecret::new(format!("tok-{sentinel}"));
    let config = SinkConfig::new(
        emulator.addr(),
        ENTRY.to_string(),
        secret(),
        producer.clone(),
        Duration::from_secs(1),
    );
    let mut texts = vec![format!("{:?}", secret()), format!("{config:?}")];
    let sink = connect_sink(config).unwrap();
    assert_eq!(admit(&sink, &producer).await, EpochAdmission::Ready);
    assert_eq!(
        sink.try_submit(frame(&producer, 0, 0, conn_open(&id(0xcd)))),
        SubmitResult::Queued
    );
    texts.push(format!("{sink:?}"));
    texts.push(format!("{:?}", sink.stats()));
    assert_eq!(
        sink.try_close_epoch(&producer, Some(0)),
        SubmitResult::Queued
    );
    texts.push(format!("{:?}", sink.shutdown().await));
    let loopback = std::net::SocketAddr::from(([10, 0, 0, 1], 9));
    for bad in [
        SinkConfig::new(loopback, ENTRY.into(), secret(), producer.clone(), ms(1)),
        SinkConfig::new(
            emulator.addr(),
            String::new(),
            secret(),
            producer.clone(),
            ms(1),
        ),
        SinkConfig::new(
            emulator.addr(),
            ENTRY.into(),
            secret(),
            sentinel.into(),
            ms(1),
        ),
        SinkConfig::new(
            emulator.addr(),
            ENTRY.into(),
            secret(),
            producer.clone(),
            Duration::ZERO,
        ),
        SinkConfig::new(
            emulator.addr(),
            ENTRY.into(),
            IngestSecret::new(sentinel.repeat(64)),
            producer.clone(),
            ms(1),
        ),
    ] {
        let error = connect_sink(bad).unwrap_err();
        texts.push(error.to_string());
        texts.push(format!("{error:?}"));
    }
    for text in &texts {
        assert!(!text.contains(sentinel), "the secret leaked into {text}");
    }
    let conn = emulator.wait_ended(0, WAIT);
    let raw = String::from_utf8_lossy(&conn.raw);
    assert_eq!(
        raw.matches(sentinel).count(),
        1,
        "the handshake carries it exactly once"
    );
    assert_eq!(conn.handshake.unwrap().token, format!("tok-{sentinel}"));
    assert!(std::env::vars().all(|(k, v)| !k.contains(sentinel) && !v.contains(sentinel)));
    assert!(std::env::args().all(|a| !a.contains(sentinel)));
}

/// Shutdown with a queued backlog behind a blocked writer: one deadline, an incomplete
/// outcome, the socket closed without Close, the task gone, and no reconnect afterwards.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_uses_one_deadline() {
    let emulator = Emulator::start();
    emulator.stall();
    let producer = id(0xb8);
    let deadline = ms(600);
    let sink = connect_sink(
        SinkConfig::new(
            emulator.addr(),
            ENTRY.to_string(),
            IngestSecret::new(TOKEN.to_string()),
            producer.clone(),
            deadline,
        )
        .with_reconnect_delays(FAST),
    )
    .unwrap();
    assert_eq!(admit(&sink, &producer).await, EpochAdmission::Ready);
    assert_eq!(
        sink.try_submit(frame(&producer, 0, 1, open(&id(0xce)))),
        SubmitResult::Queued
    );
    let big = vec![97u8; MAX_SEGMENT_BYTES];
    let mut seq = 1u64;
    let mut stable = 0;
    while stable < 3 {
        let queued = sink.stats().queued_bytes as usize;
        let next = frame(
            &producer,
            seq,
            1,
            bytes((seq - 1) * big.len() as u64, big.clone()),
        );
        if queued < 2 * 1024 * 1024 && queued + encoded_len(&next) <= PAYLOAD_QUEUE_BYTES {
            assert_eq!(sink.try_submit(next), SubmitResult::Queued);
            seq += 1;
            stable = 0;
        } else {
            tokio::time::sleep(ms(50)).await;
            stable += 1;
        }
    }
    assert!(sink.stats().queued_bytes >= 2 * 1024 * 1024 - 300_000);
    assert_eq!(
        sink.try_submit(frame(&producer, seq, 1, end())),
        SubmitResult::Queued
    );
    assert_eq!(
        sink.try_close_epoch(&producer, Some(seq)),
        SubmitResult::Queued
    );

    let started = Instant::now();
    let report = sink.shutdown().await;
    let took = started.elapsed();
    eprintln!("T12 evidence shutdown: deadline={deadline:?} took={took:?} {report:?}");
    assert!(
        took >= deadline - ms(20),
        "gave up before the deadline: {took:?}"
    );
    assert!(
        took < deadline + ms(250),
        "overran the one deadline: {took:?}"
    );
    let SenderOutcome::DeadlineExpired { unwritten_bytes } = report.outcome else {
        panic!("{report:?}")
    };
    assert!(unwritten_bytes > 0);
    assert!(!report.is_clean());
    assert_eq!(report.stats.queued_bytes, 0);

    // The socket is closed: the endpoint drains what the kernel held, then EOF, no Close.
    emulator.resume();
    let conn = emulator.wait_ended(0, WAIT);
    assert!(!conn.has_close());
    assert!(matches!(conn.verdict(), Verdict::UnknownLoss(_)));
    // The task is gone: nothing reconnects, a second shutdown does not wait.
    assert_eq!(
        sink.try_begin_epoch(&id(0xb9)),
        EpochAdmission::Disabled(GapReason::ShutdownDeadline)
    );
    let t0 = Instant::now();
    assert_eq!(sink.shutdown().await.outcome, report.outcome);
    assert!(t0.elapsed() < ms(50));
    tokio::time::sleep(ms(300)).await;
    assert_eq!(emulator.accepted(), 1);
}

/// Cancelling `shutdown()` aborts the writer instead of detaching it to drain unobserved:
/// once the endpoint reads again it finds the backlog cut off and no Close.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_shutdown_aborts_the_writer() {
    let emulator = Emulator::start();
    emulator.stall();
    let producer = id(0xbf);
    let sink = connect_sink(config(emulator.addr(), &producer)).unwrap();
    assert_eq!(admit(&sink, &producer).await, EpochAdmission::Ready);
    assert_eq!(
        sink.try_submit(frame(&producer, 0, 1, open(&id(0xd1)))),
        SubmitResult::Queued
    );
    let big = vec![97u8; MAX_SEGMENT_BYTES];
    let mut seq = 1u64;
    while (sink.stats().queued_bytes as usize) < 1024 * 1024 {
        let next = frame(
            &producer,
            seq,
            1,
            bytes((seq - 1) * big.len() as u64, big.clone()),
        );
        assert!(sink.stats().queued_bytes as usize + encoded_len(&next) <= PAYLOAD_QUEUE_BYTES);
        assert_eq!(sink.try_submit(next), SubmitResult::Queued);
        seq += 1;
        tokio::time::sleep(ms(5)).await;
    }
    assert_eq!(
        sink.try_submit(frame(&producer, seq, 1, end())),
        SubmitResult::Queued
    );
    assert_eq!(
        sink.try_close_epoch(&producer, Some(seq)),
        SubmitResult::Queued
    );
    assert!(
        tokio::time::timeout(ms(100), sink.shutdown())
            .await
            .is_err(),
        "the drain cannot finish while the endpoint is stalled"
    );
    emulator.resume();
    let conn = emulator.wait_ended(0, WAIT);
    assert!(
        !conn.has_close(),
        "a detached writer drained and closed after cancellation"
    );
    assert!(matches!(conn.verdict(), Verdict::UnknownLoss(_)));
    assert_eq!(emulator.accepted(), 1);
}

/// A socket lost while the queued Close is still waiting behind frames is an unexpected
/// write failure in the local outcome, never a clean close.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_failure_during_drain_is_not_a_clean_close() {
    let emulator = Emulator::start();
    emulator.stall();
    let producer = id(0xba);
    let sink = connect_sink(config(emulator.addr(), &producer)).unwrap();
    assert_eq!(admit(&sink, &producer).await, EpochAdmission::Ready);
    assert_eq!(
        sink.try_submit(frame(&producer, 0, 1, open(&id(0xcf)))),
        SubmitResult::Queued
    );
    let big = vec![97u8; MAX_SEGMENT_BYTES];
    let mut seq = 1u64;
    loop {
        let queued = sink.stats().queued_bytes as usize;
        if queued >= 1024 * 1024 {
            break;
        }
        let next = frame(
            &producer,
            seq,
            1,
            bytes((seq - 1) * big.len() as u64, big.clone()),
        );
        assert!(queued + encoded_len(&next) <= PAYLOAD_QUEUE_BYTES);
        assert_eq!(sink.try_submit(next), SubmitResult::Queued);
        seq += 1;
        tokio::time::sleep(ms(5)).await;
    }
    assert_eq!(
        sink.try_submit(frame(&producer, seq, 1, end())),
        SubmitResult::Queued
    );
    assert_eq!(
        sink.try_close_epoch(&producer, Some(seq)),
        SubmitResult::Queued
    );
    emulator.close_connection(0);
    emulator.resume();
    eventually("the sender to see the loss", || sink.stats().failures == 1).await;
    let report = sink.shutdown().await;
    assert_eq!(
        report.outcome,
        SenderOutcome::CaptureLost(GapReason::IngestLost)
    );
    assert_eq!((report.stats.losses, report.stats.failures), (1, 1));
    assert!(!emulator.wait_ended(0, WAIT).has_close());
}

/// Configuration is checked before any task or connection exists, and the default schedule
/// is the contract's.
#[test]
fn configuration_is_checked_before_anything_starts() {
    assert_eq!(
        RECONNECT_DELAYS,
        [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(5),
            Duration::from_secs(10)
        ]
    );
    assert_eq!(MAX_CONNECTION_FAILURES, 10);
    assert_eq!(
        PAYLOAD_QUEUE_BYTES + CONTROL_RESERVE_BYTES,
        MAX_CAPTURE_QUEUE_BYTES
    );
    let port = RefusingPort::new();
    let addr = port.addr();
    let good = || config(addr, &id(0xbb));
    // Outside a runtime there is nowhere to run the writer.
    assert_eq!(
        connect_sink(good()).unwrap_err(),
        SinkConfigError::NoRuntime
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _guard = runtime.enter();
    let cases = [
        (
            SinkConfig::new(
                "192.0.2.1:9".parse().unwrap(),
                ENTRY.into(),
                IngestSecret::new(TOKEN.into()),
                id(1),
                ms(1),
            ),
            SinkConfigError::NotLoopback,
        ),
        (
            SinkConfig::new(
                "127.0.0.1:0".parse().unwrap(),
                ENTRY.into(),
                IngestSecret::new(TOKEN.into()),
                id(1),
                ms(1),
            ),
            SinkConfigError::NotLoopback,
        ),
        (
            SinkConfig::new(
                addr,
                ENTRY.into(),
                IngestSecret::new(String::new()),
                id(1),
                ms(1),
            ),
            SinkConfigError::EmptyCredential,
        ),
        (
            SinkConfig::new(
                addr,
                ENTRY.into(),
                IngestSecret::new(TOKEN.into()),
                "ABC".into(),
                ms(1),
            ),
            SinkConfigError::InvalidProducer,
        ),
        (
            good().with_reconnect_delays([ms(1), Duration::ZERO, ms(1), ms(1)]),
            SinkConfigError::InvalidTiming,
        ),
    ];
    for (config, expected) in cases {
        assert_eq!(connect_sink(config).unwrap_err(), expected);
    }
    let sink = connect_sink(good()).unwrap();
    assert_eq!(
        sink.try_begin_epoch("not-an-id"),
        EpochAdmission::Disabled(GapReason::LimitExceeded)
    );
    // A different ID while the configured one's attempt is in flight is a configuration
    // mismatch between the sink and its caller: terminal, never a second attempt.
    assert_eq!(
        sink.try_begin_epoch(&id(0xbc)),
        EpochAdmission::Disabled(GapReason::LimitExceeded)
    );
    drop(sink);
}
