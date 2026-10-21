//! `proxy::api::CaptureEmitter` against a recording sink that checks every call against the
//! submission contract independently of the emitter.

use goalcup_cli::proxy::api::{CaptureEmitter, ProxyLoss};
use goalcup_protocol::capture::{
    CaptureFrame, CaptureMeta, CaptureSink, CaptureSource, CaptureTime, ConnectionEvent,
    ConnectionOutcome, EndReason, EpochAdmission, FrameEvent, GapReason, MAX_OPEN_CONNECTIONS,
    MAX_OPEN_STREAMS, MAX_PRODUCERS, MAX_SEGMENT_BYTES, StreamKey, SubmitResult, WireIdentity,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

const INITIAL: &str = "0123456789abcdef0123456789abcdef";
const CONN_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CONN_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/// One producer's state as the oracle sees it.
#[derive(Debug, Default)]
struct ProducerView {
    /// Admitted by a `Ready` from `try_begin_epoch`.
    admitted: bool,
    /// Admitted through recovery rather than as the initial producer.
    recovered: bool,
    next_seq: u64,
    last_seq: Option<u64>,
    open: BTreeSet<u64>,
    high_water: u64,
    closed: bool,
    failed: bool,
}

#[derive(Debug, Default)]
struct Oracle {
    producers: BTreeMap<String, ProducerView>,
    /// IDs retired by `RetryFresh`: proposing one again is a violation.
    retired: BTreeSet<String>,
    violations: Vec<String>,
    initial_admitted: bool,
}

impl Oracle {
    fn check_frame(&mut self, frame: &CaptureFrame) -> Result<(), String> {
        let view = self
            .producers
            .get_mut(&frame.stream.producer)
            .filter(|v| v.admitted)
            .ok_or_else(|| format!("frame from unadmitted producer {:?}", frame.stream))?;
        if view.closed || view.failed {
            return Err(format!("frame after close/failure: {frame:?}"));
        }
        if frame.seq != view.next_seq {
            return Err(format!(
                "seq {} where {} was next: {frame:?}",
                frame.seq, view.next_seq
            ));
        }
        if view.recovered && frame.seq == 0 && !matches!(frame.event, FrameEvent::Recovery { .. }) {
            return Err(format!(
                "a recovered producer's first frame is not Recovery: {frame:?}"
            ));
        }
        frame
            .validate()
            .map_err(|e| format!("invalid frame {e}: {frame:?}"))?;
        if frame.observed.clock_id != frame.stream.producer {
            return Err(format!(
                "C1: a producer's frame names its own clock, not {:?}: {frame:?}",
                frame.observed.clock_id
            ));
        }
        let stream = frame.stream.stream;
        match &frame.event {
            FrameEvent::Connection { .. } => {}
            FrameEvent::Loss { .. } => {
                if stream != 0 {
                    return Err(format!("a Loss is a stream-zero control: {frame:?}"));
                }
            }
            FrameEvent::Recovery { .. } => {
                if frame.seq != 0 || !view.recovered {
                    return Err(format!("Recovery out of place: {frame:?}"));
                }
            }
            FrameEvent::Open { .. } => {
                if stream <= view.high_water {
                    return Err(format!("stream ID reused or out of order: {frame:?}"));
                }
                view.high_water = stream;
                view.open.insert(stream);
            }
            FrameEvent::Bytes { .. } | FrameEvent::Gap { .. } => {
                if !view.open.contains(&stream) {
                    return Err(format!("payload on a stream that is not open: {frame:?}"));
                }
            }
            FrameEvent::End { .. } => {
                if !view.open.remove(&stream) {
                    return Err(format!("End on a stream that is not open: {frame:?}"));
                }
            }
        }
        view.last_seq = Some(frame.seq);
        view.next_seq += 1;
        Ok(())
    }
}

/// A task-local sink: `Mutex<Vec<CaptureFrame>>` plus scripted answers, recording every
/// transition against the oracle.
#[derive(Default)]
struct RecordingSink {
    frames: Mutex<Vec<CaptureFrame>>,
    oracle: Mutex<Oracle>,
    /// Answers for `try_begin_epoch`, in order; `Ready` once exhausted.
    admissions: Mutex<VecDeque<EpochAdmission>>,
    /// Refusals for `try_submit`, by call index (0-based over all submit calls).
    refusals: Mutex<BTreeMap<usize, GapReason>>,
    /// Refuse every submission with this reason.
    always_refuse: Mutex<Option<GapReason>>,
    submit_calls: Mutex<usize>,
    begun: Mutex<Vec<String>>,
    closes: Mutex<Vec<(String, Option<u64>)>>,
}

impl RecordingSink {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Admit the initial producer ID before the emitter is constructed.
    fn admit_initial(self: &Arc<Self>, producer: &str) -> Arc<CaptureEmitter> {
        assert_eq!(self.try_begin_epoch(producer), EpochAdmission::Ready);
        self.oracle.lock().unwrap().initial_admitted = true;
        let sink: Arc<dyn CaptureSink> = self.clone();
        Arc::new(CaptureEmitter::new(producer.to_string(), sink))
    }

    fn script_admissions(&self, answers: &[EpochAdmission]) {
        self.admissions
            .lock()
            .unwrap()
            .extend(answers.iter().copied());
    }

    fn refuse_call(&self, index: usize, reason: GapReason) {
        self.refusals.lock().unwrap().insert(index, reason);
    }

    fn frames(&self) -> Vec<CaptureFrame> {
        self.frames.lock().unwrap().clone()
    }

    fn begun(&self) -> Vec<String> {
        self.begun.lock().unwrap().clone()
    }

    fn submit_calls(&self) -> usize {
        *self.submit_calls.lock().unwrap()
    }

    fn assert_no_violation(&self) {
        let oracle = self.oracle.lock().unwrap();
        assert!(
            oracle.violations.is_empty(),
            "C2 violations: {:#?}",
            oracle.violations
        );
    }
}

impl CaptureSink for RecordingSink {
    fn try_submit(&self, frame: CaptureFrame) -> SubmitResult {
        let call = {
            let mut calls = self.submit_calls.lock().unwrap();
            *calls += 1;
            *calls - 1
        };
        // Widen the window between an emitter's ID allocation and this enqueue, so an
        // allocator that does not hold its ordering lock through `try_submit` is caught.
        std::thread::yield_now();
        let refusal = (*self.always_refuse.lock().unwrap())
            .or_else(|| self.refusals.lock().unwrap().get(&call).copied());
        let mut oracle = self.oracle.lock().unwrap();
        if let Some(reason) = refusal {
            if let Some(view) = oracle.producers.get_mut(&frame.stream.producer) {
                view.failed = true;
            }
            return SubmitResult::Disabled(reason);
        }
        if let Err(violation) = oracle.check_frame(&frame) {
            oracle.violations.push(violation);
        }
        // The frame is pushed while the oracle lock is held, so the vector order is the
        // order the sink admitted frames in.
        self.frames.lock().unwrap().push(frame);
        SubmitResult::Queued
    }

    fn try_begin_epoch(&self, producer: &str) -> EpochAdmission {
        self.begun.lock().unwrap().push(producer.to_string());
        let answer = self
            .admissions
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(EpochAdmission::Ready);
        let mut oracle = self.oracle.lock().unwrap();
        if oracle.retired.contains(producer) {
            oracle
                .violations
                .push(format!("retired producer ID {producer} proposed again"));
        }
        if oracle.producers.get(producer).is_some_and(|v| v.admitted) {
            oracle
                .violations
                .push(format!("admitted producer ID {producer} proposed again"));
        }
        match answer {
            EpochAdmission::Ready => {
                let recovered = oracle.initial_admitted;
                oracle.producers.insert(
                    producer.to_string(),
                    ProducerView {
                        admitted: true,
                        recovered,
                        ..ProducerView::default()
                    },
                );
            }
            EpochAdmission::RetryFresh => {
                oracle.retired.insert(producer.to_string());
            }
            EpochAdmission::Pending | EpochAdmission::Disabled(_) => {}
        }
        answer
    }

    fn try_close_epoch(&self, producer: &str, final_seq: Option<u64>) -> SubmitResult {
        let mut oracle = self.oracle.lock().unwrap();
        let violation = match oracle.producers.get_mut(producer) {
            None => Some(format!("close for unknown producer {producer}")),
            Some(view) if view.last_seq != final_seq => Some(format!(
                "close final_seq {final_seq:?} but last admitted {:?}",
                view.last_seq
            )),
            Some(view) if !view.open.is_empty() => {
                Some(format!("close with open streams {:?}", view.open))
            }
            Some(view) if view.closed => Some(format!("second close for {producer}")),
            Some(view) => {
                view.closed = true;
                None
            }
        };
        if let Some(v) = violation {
            oracle.violations.push(v);
        }
        self.closes
            .lock()
            .unwrap()
            .push((producer.to_string(), final_seq));
        SubmitResult::Queued
    }
}

fn now(clock: &str) -> CaptureTime {
    CaptureTime {
        wall: "2026-09-27T10:00:00Z".to_string(),
        mono_ns: 1,
        clock_id: clock.to_string(),
    }
}

fn request_meta(connection: &str, exchange: u64) -> CaptureMeta {
    CaptureMeta {
        wire: Some(WireIdentity {
            connection: connection.to_string(),
            exchange,
            message_index: None,
        }),
        label: format!("POST /v1/responses #{exchange}"),
        protocol_hint: Some("openai-responses".to_string()),
        method: Some("POST".to_string()),
        host: Some("api.example.test".to_string()),
        path: Some("/v1/responses".to_string()),
        status: None,
        content_type: Some("application/json".to_string()),
    }
}

fn open_request(emitter: &CaptureEmitter, exchange: u64) -> Result<StreamKey, GapReason> {
    emitter.open(
        CaptureSource::WireRequest,
        request_meta(CONN_A, exchange),
        now(INITIAL),
    )
}

fn bytes(offset: u64, data: &[u8]) -> FrameEvent {
    FrameEvent::Bytes {
        offset,
        data: data.to_vec(),
    }
}

fn end() -> FrameEvent {
    FrameEvent::End {
        reason: EndReason::Complete,
    }
}

fn open_conn(alpn: Option<&str>) -> ConnectionEvent {
    ConnectionEvent::Open {
        alpn: alpn.map(str::to_string),
        already_open: false,
    }
}

/// Fail the current epoch with `reason` by refusing the next submission.
fn fail_epoch(sink: &RecordingSink, emitter: &CaptureEmitter, reason: GapReason) {
    sink.refuse_call(sink.submit_calls(), reason);
    assert_eq!(
        open_request(emitter, 99),
        Err(reason),
        "the refused submission latches the epoch"
    );
}

#[test]
fn concurrent_emitter_sequences_match_queue_order() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    let workers: Vec<_> = (0..8u64)
        .map(|worker| {
            let emitter = Arc::clone(&emitter);
            std::thread::spawn(move || {
                let key = emitter
                    .open(
                        CaptureSource::WireRequest,
                        request_meta(CONN_A, worker + 1),
                        now(INITIAL),
                    )
                    .expect("admitted");
                let payload = format!("worker-{worker}-body");
                assert_eq!(
                    emitter.emit(&key, now(INITIAL), bytes(0, payload.as_bytes())),
                    SubmitResult::Queued
                );
                assert_eq!(
                    emitter.emit(&key, now(INITIAL), end()),
                    SubmitResult::Queued
                );
                (key, payload)
            })
        })
        .collect();
    let results: Vec<(StreamKey, String)> =
        workers.into_iter().map(|w| w.join().unwrap()).collect();

    let frames = sink.frames();
    assert_eq!(frames.len(), 24, "eight Open/Bytes/End triples");
    for (index, frame) in frames.iter().enumerate() {
        assert_eq!(
            frame.seq, index as u64,
            "a frame's seq equals its position in the queue"
        );
        assert_eq!(frame.stream.producer, INITIAL);
    }
    let keys: BTreeSet<u64> = results.iter().map(|(k, _)| k.stream).collect();
    assert_eq!(
        keys,
        (1..=8).collect::<BTreeSet<u64>>(),
        "stream keys never collide"
    );
    // Open stream IDs are allocated in Open emission order.
    let opened: Vec<u64> = frames
        .iter()
        .filter(|f| matches!(f.event, FrameEvent::Open { .. }))
        .map(|f| f.stream.stream)
        .collect();
    assert_eq!(opened, (1..=8).collect::<Vec<u64>>());
    // Each stream holds exactly its own worker's Open, Bytes at offset 0, End.
    for (key, payload) in &results {
        let own: Vec<&CaptureFrame> = frames.iter().filter(|f| f.stream == *key).collect();
        assert_eq!(own.len(), 3);
        assert!(matches!(own[0].event, FrameEvent::Open { .. }));
        match &own[1].event {
            FrameEvent::Bytes { offset, data } => {
                assert_eq!(*offset, 0, "offsets are per stream");
                assert_eq!(data, payload.as_bytes());
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(own[2].event, FrameEvent::End { .. }));
    }
    sink.assert_no_violation();
    assert_eq!(emitter.try_close(now(INITIAL)), SubmitResult::Queued);
    assert_eq!(
        sink.closes.lock().unwrap().clone(),
        vec![(INITIAL.to_string(), Some(23))]
    );
    sink.assert_no_violation();
}

#[test]
fn try_recover_on_a_live_epoch_is_idempotent() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    let key = open_request(&emitter, 1).unwrap();
    for _ in 0..3 {
        assert_eq!(emitter.try_recover(now(INITIAL)), SubmitResult::Queued);
    }
    assert_eq!(sink.begun(), vec![INITIAL.to_string()], "no admission poll");
    assert_eq!(sink.frames().len(), 1, "no frame");
    assert_eq!(
        emitter.emit(&key, now(INITIAL), bytes(0, b"x")),
        SubmitResult::Queued,
        "the live stream is untouched"
    );
    sink.assert_no_violation();
}

#[test]
fn recovery_precedes_payload_and_reobserves_connections_in_order() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    assert_eq!(
        emitter.connection_event(CONN_A, now(INITIAL), open_conn(Some("http/1.1"))),
        SubmitResult::Queued
    );
    assert_eq!(
        emitter.connection_event(CONN_B, now(INITIAL), open_conn(None)),
        SubmitResult::Queued
    );
    assert_eq!(
        emitter.connection_event(CONN_B, now(INITIAL), ConnectionEvent::UpgradedWebSocket),
        SubmitResult::Queued
    );
    let stale = open_request(&emitter, 1).unwrap();
    fail_epoch(&sink, &emitter, GapReason::QueueFull);

    // While failed: no payload and no frame, but connection tracking continues: CONN_A
    // closes (released) and conn_c opens and stays open (tracked).
    assert_eq!(open_request(&emitter, 2), Err(GapReason::QueueFull));
    let conn_c = "cccccccccccccccccccccccccccccccc";
    let during_failure = sink.frames().len();
    assert_eq!(
        emitter.connection_event(
            CONN_A,
            now(INITIAL),
            ConnectionEvent::Close {
                outcome: ConnectionOutcome::Complete
            }
        ),
        SubmitResult::Disabled(GapReason::QueueFull)
    );
    assert_eq!(
        emitter.connection_event(conn_c, now(INITIAL), open_conn(None)),
        SubmitResult::Disabled(GapReason::QueueFull)
    );
    assert_eq!(
        sink.frames().len(),
        during_failure,
        "nothing is submitted while failed"
    );

    let before = sink.frames().len();
    // The caller names some other clock; the emitter stamps the new producer's own.
    assert_eq!(
        emitter.try_recover(now("caller-clock")),
        SubmitResult::Queued
    );
    let begun = sink.begun();
    assert_eq!(begun.len(), 2);
    let fresh = begun[1].clone();
    assert_ne!(fresh, INITIAL);
    let fresh_key = open_request(&emitter, 3).unwrap();
    assert_eq!(fresh_key.producer, fresh);
    assert_eq!(
        fresh_key.stream, 1,
        "stream IDs restart for the new producer"
    );

    let recovered: Vec<CaptureFrame> = sink.frames()[before..].to_vec();
    let shape: Vec<String> = recovered
        .iter()
        .map(|f| match &f.event {
            FrameEvent::Recovery {
                previous_producer,
                reason,
            } => format!("recovery {previous_producer} {reason:?}"),
            FrameEvent::Connection { connection, event } => format!("{connection} {event:?}"),
            FrameEvent::Open { .. } => format!("open {}", f.stream.stream),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        shape,
        vec![
            format!("recovery {INITIAL} QueueFull"),
            format!("{CONN_B} Open {{ alpn: None, already_open: true }}"),
            format!("{CONN_B} UpgradedWebSocket"),
            format!("{conn_c} Open {{ alpn: None, already_open: true }}"),
            "open 1".to_string(),
        ],
        "Recovery first, then every surviving connection in opening order (CONN_A closed \
         while failed, conn_c opened while failed), then payload"
    );
    for (index, frame) in recovered.iter().enumerate() {
        assert_eq!(frame.stream.producer, fresh);
        assert_eq!(
            frame.seq, index as u64,
            "the new producer's seq restarts at zero"
        );
        assert_eq!(
            frame.observed.clock_id, fresh,
            "Recovery, re-observed connections and payload name the new producer's clock"
        );
    }
    for frame in &sink.frames()[..before] {
        assert_eq!(
            frame.observed.clock_id, INITIAL,
            "the old epoch's frames keep its clock"
        );
    }
    // The old epoch's key stays invalid.
    assert_eq!(
        emitter.emit(&stale, now(INITIAL), bytes(0, b"late")),
        SubmitResult::Disabled(GapReason::ProducerLost)
    );
    sink.assert_no_violation();
}

#[test]
fn stale_stream_key_is_rejected_after_rotation() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    let old = open_request(&emitter, 1).unwrap();
    fail_epoch(&sink, &emitter, GapReason::IngestLost);
    assert_eq!(emitter.try_recover(now(INITIAL)), SubmitResult::Queued);
    let new = open_request(&emitter, 2).unwrap();
    assert_eq!(
        new.stream, old.stream,
        "the same stream number under a new producer"
    );
    assert_ne!(new.producer, old.producer);
    let frames_before = sink.frames().len();
    for event in [bytes(0, b"suffix"), end()] {
        assert_eq!(
            emitter.emit(&old, now(INITIAL), event),
            SubmitResult::Disabled(GapReason::ProducerLost),
            "no interrupted suffix is resumed under an old key"
        );
    }
    assert_eq!(sink.frames().len(), frames_before, "nothing was submitted");
    assert_eq!(
        emitter.emit(&new, now(INITIAL), bytes(0, b"fresh")),
        SubmitResult::Queued
    );
    sink.assert_no_violation();
}

#[test]
fn pending_id_is_reused_and_retry_fresh_id_never_is() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    fail_epoch(&sink, &emitter, GapReason::IngestLost);
    sink.script_admissions(&[
        EpochAdmission::Pending,
        EpochAdmission::Pending,
        EpochAdmission::RetryFresh,
        EpochAdmission::Pending,
        EpochAdmission::Ready,
    ]);
    for _ in 0..3 {
        assert_eq!(
            emitter.try_recover(now(INITIAL)),
            SubmitResult::Disabled(GapReason::IngestLost),
            "no capture until an epoch is Ready"
        );
        assert_eq!(open_request(&emitter, 5), Err(GapReason::IngestLost));
    }
    assert_eq!(
        emitter.try_recover(now(INITIAL)),
        SubmitResult::Disabled(GapReason::IngestLost)
    );
    assert_eq!(emitter.try_recover(now(INITIAL)), SubmitResult::Queued);
    let begun = sink.begun();
    assert_eq!(begun.len(), 6);
    let (a, b) = (&begun[1], &begun[4]);
    assert_eq!(&begun[2], a, "a Pending ID is kept across polls");
    assert_eq!(&begun[3], a, "and proposed until it resolves");
    assert_ne!(b, a, "a RetryFresh ID is never proposed again");
    assert_eq!(&begun[5], b);
    assert_eq!(open_request(&emitter, 6).unwrap().producer, *b);
    sink.assert_no_violation();
}

#[test]
fn terminal_admission_disables_recovery_for_good() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    fail_epoch(&sink, &emitter, GapReason::QueueFull);
    sink.script_admissions(&[EpochAdmission::Disabled(GapReason::StorageFull)]);
    assert_eq!(
        emitter.try_recover(now(INITIAL)),
        SubmitResult::Disabled(GapReason::StorageFull)
    );
    let polls = sink.begun().len();
    assert_eq!(
        emitter.try_recover(now(INITIAL)),
        SubmitResult::Disabled(GapReason::StorageFull)
    );
    assert_eq!(
        sink.begun().len(),
        polls,
        "a terminal failure is not retried"
    );
    sink.assert_no_violation();
}

#[test]
fn failed_recovery_admission_leaves_the_new_epoch_disabled() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    fail_epoch(&sink, &emitter, GapReason::QueueFull);
    // The next submission is the new producer's Recovery; refuse it.
    sink.refuse_call(sink.submit_calls(), GapReason::StorageFull);
    assert_eq!(
        emitter.try_recover(now(INITIAL)),
        SubmitResult::Disabled(GapReason::StorageFull)
    );
    assert_eq!(open_request(&emitter, 7), Err(GapReason::StorageFull));
    assert_eq!(
        emitter.try_recover(now(INITIAL)),
        SubmitResult::Disabled(GapReason::StorageFull)
    );
    sink.assert_no_violation();
}

/// A refused Recovery frame disables the epoch for good, even when
/// the refusal reason (QueueFull/IngestLost) would ordinarily be recoverable. The candidate
/// producer whose Recovery frame was refused was never persisted, so it must never be named
/// again and no second `try_begin_epoch` may be attempted.
#[test]
fn failed_recovery_frame_disables_the_epoch_even_for_a_recoverable_reason() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    let stale = open_request(&emitter, 1).unwrap();
    fail_epoch(&sink, &emitter, GapReason::IngestLost);
    // The next submission is the new producer's Recovery frame itself; refuse it with a
    // reason that `latch` ordinarily treats as recoverable.
    sink.refuse_call(sink.submit_calls(), GapReason::QueueFull);
    assert_eq!(
        emitter.try_recover(now(INITIAL)),
        SubmitResult::Disabled(GapReason::QueueFull)
    );
    let polls = sink.begun().len();
    for _ in 0..3 {
        assert_eq!(
            emitter.try_recover(now(INITIAL)),
            SubmitResult::Disabled(GapReason::QueueFull),
            "a refused Recovery frame is terminal: no further recovery is ever attempted"
        );
    }
    assert_eq!(
        sink.begun().len(),
        polls,
        "no second try_begin_epoch after the Recovery frame itself was refused"
    );
    assert_eq!(open_request(&emitter, 7), Err(GapReason::QueueFull));
    assert_eq!(
        emitter.emit(&stale, now(INITIAL), bytes(0, b"late")),
        SubmitResult::Disabled(GapReason::QueueFull),
        "every later emit sees the disabled result too"
    );
    sink.assert_no_violation();
}

/// Same as above with `IngestLost`, the other reason `latch` would otherwise treat as
/// recoverable.
#[test]
fn failed_recovery_frame_disables_the_epoch_on_ingest_lost() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    fail_epoch(&sink, &emitter, GapReason::QueueFull);
    sink.refuse_call(sink.submit_calls(), GapReason::IngestLost);
    assert_eq!(
        emitter.try_recover(now(INITIAL)),
        SubmitResult::Disabled(GapReason::IngestLost)
    );
    let polls = sink.begun().len();
    assert_eq!(
        emitter.try_recover(now(INITIAL)),
        SubmitResult::Disabled(GapReason::IngestLost),
        "IngestLost refusing the Recovery frame itself is terminal too"
    );
    assert_eq!(
        sink.begun().len(),
        polls,
        "no second try_begin_epoch after the Recovery frame itself was refused"
    );
    assert_eq!(open_request(&emitter, 7), Err(GapReason::IngestLost));
    sink.assert_no_violation();
}

/// Companion to the two tests above: once the Recovery frame itself is *admitted*, a
/// refusal of the re-observation frame that follows it keeps the recoverable latch behaviour
/// (only the Recovery frame itself is special-cased).
#[test]
fn refused_reobservation_after_admitted_recovery_stays_recoverable() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    assert_eq!(
        emitter.connection_event(CONN_A, now(INITIAL), open_conn(None)),
        SubmitResult::Queued
    );
    fail_epoch(&sink, &emitter, GapReason::IngestLost);
    // Let the Recovery frame itself succeed; refuse the re-observation frame right after it.
    sink.refuse_call(sink.submit_calls() + 1, GapReason::QueueFull);
    assert_eq!(
        emitter.try_recover(now(INITIAL)),
        SubmitResult::Disabled(GapReason::QueueFull)
    );
    let polls = sink.begun().len();
    // This refusal is still recoverable, so a further try_recover proposes a fresh producer
    // again (unlike a refused Recovery frame).
    assert_eq!(emitter.try_recover(now(INITIAL)), SubmitResult::Queued);
    assert!(
        sink.begun().len() > polls,
        "a re-observation refusal (unlike the Recovery frame itself) stays recoverable"
    );
    sink.assert_no_violation();
}

/// The reader and receiver rely on this: a sink's `Queued` is not persistence. Q's Recovery
/// and re-observation are answered `Queued`, then the ingest path is lost before anything of
/// Q is delivered, which the emitter only learns as an `IngestLost` refusal of a later frame.
/// The emitter cannot know Q was never persisted, so the next epoch R names the immediately
/// previous producer Q, never the last one known to have delivered (P), and recovery stays
/// available.
#[test]
fn an_undelivered_recovery_is_still_named_by_the_next_recovery() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    assert_eq!(
        emitter.connection_event(CONN_A, now(INITIAL), open_conn(None)),
        SubmitResult::Queued
    );
    fail_epoch(&sink, &emitter, GapReason::IngestLost);

    // Q: Recovery{previous: P} and the re-observation are both answered Queued.
    let q_start = sink.frames().len();
    assert_eq!(emitter.try_recover(now(INITIAL)), SubmitResult::Queued);
    let q = sink.begun()[1].clone();
    assert!(matches!(
        &sink.frames()[q_start].event,
        FrameEvent::Recovery { previous_producer, .. } if previous_producer == INITIAL
    ));
    assert_eq!(
        sink.frames().len(),
        q_start + 2,
        "Recovery and one re-observation"
    );

    // Q's next frame is refused IngestLost: the ingest path closed under it.
    fail_epoch(&sink, &emitter, GapReason::IngestLost);

    // R names Q, and recovery is still recoverable, not disabled.
    let r_start = sink.frames().len();
    assert_eq!(emitter.try_recover(now(INITIAL)), SubmitResult::Queued);
    let begun = sink.begun();
    assert_eq!(begun.len(), 3);
    let r = begun[2].clone();
    assert_ne!(r, q);
    let r_frames = sink.frames()[r_start..].to_vec();
    assert_eq!(
        r_frames[0].event,
        FrameEvent::Recovery {
            previous_producer: q.clone(),
            reason: GapReason::IngestLost,
        },
        "the next Recovery names the producer whose Recovery was only Queued"
    );
    assert_eq!(r_frames[0].stream.producer, r);
    assert_eq!(
        r_frames[1].event,
        FrameEvent::Connection {
            connection: CONN_A.to_string(),
            event: ConnectionEvent::Open {
                alpn: None,
                already_open: true,
            },
        },
        "the surviving connection is re-observed again under R"
    );
    assert_eq!(open_request(&emitter, 2).unwrap().producer, r);
    sink.assert_no_violation();
}

#[test]
fn producer_budget_is_bounded() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    for _ in 1..MAX_PRODUCERS {
        fail_epoch(&sink, &emitter, GapReason::IngestLost);
        assert_eq!(emitter.try_recover(now(INITIAL)), SubmitResult::Queued);
    }
    fail_epoch(&sink, &emitter, GapReason::IngestLost);
    let polls = sink.begun().len();
    assert_eq!(polls, MAX_PRODUCERS);
    assert_eq!(
        emitter.try_recover(now(INITIAL)),
        SubmitResult::Disabled(GapReason::LimitExceeded),
        "the {MAX_PRODUCERS}th producer was the last"
    );
    assert_eq!(
        sink.begun().len(),
        polls,
        "no ID beyond the budget is proposed"
    );
    sink.assert_no_violation();
}

#[test]
fn close_declares_final_seq_and_requires_no_open_stream() {
    // Zero frames: the close declares None.
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    assert_eq!(emitter.try_close(now(INITIAL)), SubmitResult::Queued);
    assert_eq!(
        sink.closes.lock().unwrap().clone(),
        vec![(INITIAL.to_string(), None)]
    );

    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    let key = open_request(&emitter, 1).unwrap();
    assert_eq!(
        emitter.emit(&key, now(INITIAL), bytes(0, b"abc")),
        SubmitResult::Queued
    );
    assert_eq!(
        emitter.try_close(now(INITIAL)),
        SubmitResult::Disabled(GapReason::ShutdownDeadline),
        "an open payload stream refuses a clean close"
    );
    assert!(
        sink.closes.lock().unwrap().is_empty(),
        "the sink never saw it"
    );
    assert_eq!(
        emitter.emit(&key, now(INITIAL), end()),
        SubmitResult::Queued
    );
    assert_eq!(emitter.try_close(now(INITIAL)), SubmitResult::Queued);
    assert_eq!(emitter.try_close(now(INITIAL)), SubmitResult::Queued);
    assert_eq!(
        sink.closes.lock().unwrap().clone(),
        vec![(INITIAL.to_string(), Some(2))],
        "one close naming the last admitted seq; a retry sends nothing"
    );
    assert_eq!(
        open_request(&emitter, 2),
        Err(GapReason::ShutdownDeadline),
        "nothing follows a close"
    );
    sink.assert_no_violation();

    // A failed epoch never closes cleanly.
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    fail_epoch(&sink, &emitter, GapReason::QueueFull);
    assert_eq!(
        emitter.try_close(now(INITIAL)),
        SubmitResult::Disabled(GapReason::QueueFull)
    );
    assert!(sink.closes.lock().unwrap().is_empty());
}

#[test]
fn a_disabled_sink_stops_capture_without_retries() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    *sink.always_refuse.lock().unwrap() = Some(GapReason::StorageFull);
    assert_eq!(open_request(&emitter, 1), Err(GapReason::StorageFull));
    assert_eq!(sink.submit_calls(), 1);
    assert_eq!(open_request(&emitter, 2), Err(GapReason::StorageFull));
    assert_eq!(
        emitter.connection_event(CONN_A, now(INITIAL), open_conn(None)),
        SubmitResult::Disabled(GapReason::StorageFull)
    );
    assert_eq!(
        emitter.try_recover(now(INITIAL)),
        SubmitResult::Disabled(GapReason::StorageFull)
    );
    assert_eq!(
        emitter.try_close(now(INITIAL)),
        SubmitResult::Disabled(GapReason::StorageFull)
    );
    assert_eq!(
        sink.submit_calls(),
        1,
        "a terminal refusal is latched, not retried"
    );
    assert_eq!(sink.begun().len(), 1, "no recovery attempt");
    assert!(sink.closes.lock().unwrap().is_empty());
}

#[test]
fn stream_and_connection_limits_disable_capture_instead_of_growing() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    for exchange in 0..MAX_OPEN_STREAMS as u64 {
        open_request(&emitter, exchange).expect("within the open-stream bound");
    }
    assert_eq!(open_request(&emitter, 999), Err(GapReason::LimitExceeded));
    assert_eq!(
        emitter.try_recover(now(INITIAL)),
        SubmitResult::Disabled(GapReason::LimitExceeded),
        "exhaustion is not recoverable"
    );
    assert_eq!(sink.frames().len(), MAX_OPEN_STREAMS);
    sink.assert_no_violation();

    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    for n in 0..MAX_OPEN_CONNECTIONS {
        let id = format!("{n:032x}");
        assert_eq!(
            emitter.connection_event(&id, now(INITIAL), open_conn(None)),
            SubmitResult::Queued
        );
    }
    assert_eq!(
        emitter.connection_event(CONN_A, now(INITIAL), open_conn(None)),
        SubmitResult::Disabled(GapReason::LimitExceeded)
    );
    assert_eq!(open_request(&emitter, 1), Err(GapReason::LimitExceeded));
    assert_eq!(sink.frames().len(), MAX_OPEN_CONNECTIONS);
    sink.assert_no_violation();
}

#[test]
fn contract_violations_are_refused_before_submission() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    // Metadata over a limit: refused without serializing or submitting, epoch still live.
    let mut huge = request_meta(CONN_A, 1);
    huge.label = "x".repeat(4 * 1024 * 1024);
    assert_eq!(
        emitter.open(CaptureSource::WireRequest, huge, now(INITIAL)),
        Err(GapReason::LimitExceeded)
    );
    // The open itself submits nothing, but the relay goes on uncaptured, so the loss is the
    // epoch's one stream-zero `Loss` (H-16, `an_open_refused_for_its_metadata_records_a_loss`).
    assert_eq!(sink.submit_calls(), 1);
    assert!(matches!(
        sink.frames()[0].event,
        FrameEvent::Loss {
            reason: GapReason::LimitExceeded
        }
    ));
    let key = open_request(&emitter, 1).expect("the epoch stays live");
    assert_eq!(key.stream, 1, "the refused open consumed no stream ID");
    // emit carries payload only.
    assert_eq!(
        emitter.emit(
            &key,
            now(INITIAL),
            FrameEvent::Open {
                source: CaptureSource::WireRequest,
                metadata: request_meta(CONN_A, 2),
            }
        ),
        SubmitResult::Disabled(GapReason::LimitExceeded)
    );
    // An unknown stream of the current producer, and stream zero, are not open streams.
    for stream in [0, 7] {
        let other = StreamKey {
            producer: INITIAL.to_string(),
            stream,
        };
        assert_eq!(
            emitter.emit(&other, now(INITIAL), bytes(0, b"x")),
            SubmitResult::Disabled(GapReason::ProducerLost)
        );
    }
    // A connection event outside the limits is refused and never tracked.
    assert_eq!(
        emitter.connection_event(CONN_B, now(INITIAL), open_conn(Some("h2"))),
        SubmitResult::Disabled(GapReason::LimitExceeded)
    );
    assert_eq!(
        emitter.connection_event("not-hex", now(INITIAL), open_conn(None)),
        SubmitResult::Disabled(GapReason::LimitExceeded)
    );
    assert_eq!(
        emitter.connection_event(CONN_B, now(INITIAL), ConnectionEvent::UpgradedWebSocket),
        SubmitResult::Disabled(GapReason::ProducerLost),
        "an untracked connection cannot be upgraded"
    );
    assert_eq!(sink.frames().len(), 2, "the Loss and the one real Open");
    sink.assert_no_violation();

    // An emitter built on an ID that is not a fresh 128-bit hex ID submits nothing.
    let sink = RecordingSink::new();
    let bad: Arc<dyn CaptureSink> = sink.clone();
    let emitter = CaptureEmitter::new("producer-1".to_string(), bad);
    assert_eq!(
        emitter.open(
            CaptureSource::WireRequest,
            request_meta(CONN_A, 1),
            now(INITIAL)
        ),
        Err(GapReason::LimitExceeded)
    );
    assert_eq!(sink.submit_calls(), 0);
}

#[test]
fn retry_fresh_answers_count_against_the_producer_budget() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    fail_epoch(&sink, &emitter, GapReason::IngestLost);
    let retries = MAX_PRODUCERS - 1;
    sink.script_admissions(&vec![EpochAdmission::RetryFresh; retries]);
    for _ in 0..retries {
        assert_eq!(
            emitter.try_recover(now(INITIAL)),
            SubmitResult::Disabled(GapReason::IngestLost)
        );
    }
    let polls = sink.begun().len();
    assert_eq!(polls, 1 + retries, "one fresh ID per RetryFresh");
    let begun = sink.begun();
    let distinct: BTreeSet<&String> = begun[1..].iter().collect();
    assert_eq!(distinct.len(), retries, "no retired ID was proposed again");
    assert_eq!(
        emitter.try_recover(now(INITIAL)),
        SubmitResult::Disabled(GapReason::LimitExceeded),
        "every possibly-delivered ID counts: the initial one plus {retries} retired ones"
    );
    assert_eq!(
        sink.begun().len(),
        polls,
        "no ID beyond MAX_PRODUCERS is proposed"
    );
    sink.assert_no_violation();
}

#[test]
fn oversized_bytes_are_split_losslessly_into_contiguous_segments() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    let key = open_request(&emitter, 1).unwrap();
    let payload: Vec<u8> = (0..3 * MAX_SEGMENT_BYTES + 5)
        .map(|i| (i % 251) as u8)
        .collect();
    let start = 1_000u64;
    assert_eq!(
        emitter.emit(&key, now(INITIAL), bytes(start, &payload)),
        SubmitResult::Queued
    );
    assert_eq!(
        emitter.emit(
            &key,
            now(INITIAL),
            bytes(start + payload.len() as u64, b"tail")
        ),
        SubmitResult::Queued
    );
    assert_eq!(
        emitter.emit(&key, now(INITIAL), end()),
        SubmitResult::Queued
    );
    let frames = sink.frames();
    let mut rebuilt = Vec::new();
    let mut expected_offset = start;
    let mut sizes = Vec::new();
    for frame in frames
        .iter()
        .filter(|f| matches!(f.event, FrameEvent::Bytes { .. }))
    {
        let FrameEvent::Bytes { offset, data } = &frame.event else {
            unreachable!()
        };
        assert_eq!(*offset, expected_offset, "offsets are contiguous");
        assert!(data.len() <= MAX_SEGMENT_BYTES);
        expected_offset += data.len() as u64;
        sizes.push(data.len());
        rebuilt.extend_from_slice(data);
    }
    assert_eq!(
        sizes,
        vec![
            MAX_SEGMENT_BYTES,
            MAX_SEGMENT_BYTES,
            MAX_SEGMENT_BYTES,
            5,
            4
        ]
    );
    assert_eq!(
        &rebuilt[..payload.len()],
        payload.as_slice(),
        "nothing lost"
    );
    for (index, frame) in frames.iter().enumerate() {
        assert_eq!(
            frame.seq, index as u64,
            "the split frames take consecutive seqs"
        );
    }
    assert_eq!(emitter.try_close(now(INITIAL)), SubmitResult::Queued);
    sink.assert_no_violation();

    // A refusal part-way keeps the queued prefix and latches the epoch.
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    let key = open_request(&emitter, 1).unwrap();
    sink.refuse_call(sink.submit_calls() + 2, GapReason::QueueFull);
    assert_eq!(
        emitter.emit(&key, now(INITIAL), bytes(0, &payload)),
        SubmitResult::Disabled(GapReason::QueueFull)
    );
    assert_eq!(
        sink.frames().len(),
        3,
        "Open plus the two admitted segments"
    );
    assert_eq!(open_request(&emitter, 2), Err(GapReason::QueueFull));
    sink.assert_no_violation();
}

#[test]
fn a_local_emit_refusal_leaves_the_stream_open_for_the_caller_to_end() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    let bad_clock = CaptureTime {
        wall: "not a timestamp".to_string(),
        ..now(INITIAL)
    };
    // More cycles than there are open-stream slots: a slot released by End is reusable.
    for exchange in 0..(2 * MAX_OPEN_STREAMS as u64) {
        let key = open_request(&emitter, exchange).expect("no slot has leaked");
        let submitted = sink.submit_calls();
        assert_eq!(
            emitter.emit(&key, bad_clock.clone(), bytes(0, b"x")),
            SubmitResult::Disabled(GapReason::LimitExceeded),
            "an invalid observation is a local refusal"
        );
        assert_eq!(
            emitter.emit(
                &key,
                now(INITIAL),
                FrameEvent::Open {
                    source: CaptureSource::WireRequest,
                    metadata: request_meta(CONN_A, exchange),
                }
            ),
            SubmitResult::Disabled(GapReason::LimitExceeded),
            "a non-payload event kind is a local refusal"
        );
        assert_eq!(
            sink.submit_calls(),
            submitted,
            "a local refusal submits nothing"
        );
        assert_eq!(
            emitter.try_close(now(INITIAL)),
            SubmitResult::Disabled(GapReason::ShutdownDeadline),
            "the refused stream is still open"
        );
        assert_eq!(
            emitter.emit(
                &key,
                now(INITIAL),
                FrameEvent::End {
                    reason: EndReason::Interrupted
                }
            ),
            SubmitResult::Queued,
            "the caller ends the stream it could not capture"
        );
    }
    assert_eq!(emitter.try_close(now(INITIAL)), SubmitResult::Queued);
    let last = sink.frames().last().unwrap().seq;
    assert_eq!(
        sink.closes.lock().unwrap().clone(),
        vec![(INITIAL.to_string(), Some(last))]
    );
    sink.assert_no_violation();
}

// ------------------------------------------------------------------ H-16: Loss

fn loss_reasons(sink: &RecordingSink) -> Vec<GapReason> {
    sink.frames()
        .iter()
        .filter_map(|f| match f.event {
            FrameEvent::Loss { reason } => Some(reason),
            _ => None,
        })
        .collect()
}

/// The recording proxy's own losses and the one reason each is chained as (H-16). A fail-open
/// tunnel and an opaque fallback share a reason, so between them the emitter chains one `Loss`;
/// that collapse is the documented one, not an accident of two call sites.
#[test]
fn each_proxy_loss_is_chained_as_its_one_documented_reason() {
    assert_eq!(
        ProxyLoss::FailOpenTunnel.gap_reason(),
        GapReason::DecodeFailed
    );
    assert_eq!(
        ProxyLoss::OpaqueFallback.gap_reason(),
        GapReason::DecodeFailed
    );
    assert_eq!(
        ProxyLoss::CaptureTableFull.gap_reason(),
        GapReason::LimitExceeded
    );
    assert_eq!(
        ProxyLoss::CutUnsettled.gap_reason(),
        GapReason::ShutdownDeadline
    );

    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    for loss in [
        ProxyLoss::FailOpenTunnel,
        ProxyLoss::OpaqueFallback,
        ProxyLoss::CaptureTableFull,
        ProxyLoss::CutUnsettled,
    ] {
        assert_eq!(
            emitter.loss(now(INITIAL), loss.gap_reason()),
            SubmitResult::Queued
        );
    }
    assert_eq!(
        loss_reasons(&sink),
        [
            GapReason::DecodeFailed,
            GapReason::LimitExceeded,
            GapReason::ShutdownDeadline
        ],
        "a fail-open tunnel and an opaque fallback leave one Loss between them"
    );
}

#[test]
fn a_loss_is_a_stream_zero_control_sent_once_per_reason() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    for _ in 0..3 {
        assert_eq!(
            emitter.loss(now(INITIAL), GapReason::DecodeFailed),
            SubmitResult::Queued
        );
    }
    assert_eq!(sink.frames().len(), 1, "a reason is sent once per emitter");
    let frame = &sink.frames()[0];
    assert_eq!(
        frame.stream,
        StreamKey {
            producer: INITIAL.to_string(),
            stream: 0
        }
    );
    assert_eq!(frame.seq, 0);
    assert_eq!(frame.observed.clock_id, INITIAL, "the producer's own clock");
    assert_eq!(
        frame.event,
        FrameEvent::Loss {
            reason: GapReason::DecodeFailed
        }
    );
    // A second reason is a second frame; the sequence is the producer's, controls included.
    assert_eq!(
        emitter.loss(now(INITIAL), GapReason::LimitExceeded),
        SubmitResult::Queued
    );
    assert_eq!(
        emitter.loss(now(INITIAL), GapReason::ShutdownDeadline),
        SubmitResult::Queued
    );
    assert_eq!(
        loss_reasons(&sink),
        vec![
            GapReason::DecodeFailed,
            GapReason::LimitExceeded,
            GapReason::ShutdownDeadline
        ]
    );
    // A loss takes no stream ID: the first payload stream is still stream one, at seq 3.
    let key = open_request(&emitter, 1).unwrap();
    assert_eq!(key.stream, 1);
    assert_eq!(sink.frames().last().unwrap().seq, 3);
    assert_eq!(
        emitter.emit(&key, now(INITIAL), end()),
        SubmitResult::Queued
    );
    sink.assert_no_violation();
}

#[test]
fn close_after_a_loss_names_its_sequence() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    // A loss-only epoch: the loss is the producer's only frame, so the close names it.
    assert_eq!(
        emitter.loss(now(INITIAL), GapReason::DecodeFailed),
        SubmitResult::Queued
    );
    assert_eq!(emitter.try_close(now(INITIAL)), SubmitResult::Queued);
    assert_eq!(
        sink.closes.lock().unwrap().clone(),
        vec![(INITIAL.to_string(), Some(0))],
        "the close is chained after the loss and attests it"
    );
    sink.assert_no_violation();
}

#[test]
fn a_loss_on_a_failed_or_disabled_epoch_submits_nothing() {
    // Failed: recoverable, followed by a Recovery, so nothing is queued and nothing remembered.
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    fail_epoch(&sink, &emitter, GapReason::QueueFull);
    let calls = sink.submit_calls();
    assert_eq!(
        emitter.loss(now(INITIAL), GapReason::DecodeFailed),
        SubmitResult::Disabled(GapReason::QueueFull)
    );
    assert_eq!(
        sink.submit_calls(),
        calls,
        "a failed epoch is not offered a frame"
    );
    assert_eq!(
        emitter.try_close(now(INITIAL)),
        SubmitResult::Disabled(GapReason::QueueFull),
        "and it can never close cleanly"
    );
    // The reason was not remembered, so the recovered epoch carries it.
    assert_eq!(emitter.try_recover(now(INITIAL)), SubmitResult::Queued);
    assert_eq!(
        emitter.loss(now(INITIAL), GapReason::DecodeFailed),
        SubmitResult::Queued
    );
    assert_eq!(loss_reasons(&sink), vec![GapReason::DecodeFailed]);
    sink.assert_no_violation();

    // Disabled: terminal, never closes.
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    fail_epoch(&sink, &emitter, GapReason::StorageFull);
    let calls = sink.submit_calls();
    assert_eq!(
        emitter.loss(now(INITIAL), GapReason::DecodeFailed),
        SubmitResult::Disabled(GapReason::StorageFull)
    );
    assert_eq!(sink.submit_calls(), calls);
    assert_eq!(
        emitter.try_close(now(INITIAL)),
        SubmitResult::Disabled(GapReason::StorageFull)
    );
    assert!(sink.closes.lock().unwrap().is_empty());
    sink.assert_no_violation();
}

#[test]
fn a_refused_loss_latches_the_epoch_so_it_never_closes_cleanly() {
    // The sink refuses the loss itself: the epoch latches like any refusal, the close is not
    // queued, and the reason is not remembered, so a recovered epoch records it again.
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    sink.refuse_call(sink.submit_calls(), GapReason::QueueFull);
    assert_eq!(
        emitter.loss(now(INITIAL), GapReason::DecodeFailed),
        SubmitResult::Disabled(GapReason::QueueFull)
    );
    assert_eq!(
        emitter.try_close(now(INITIAL)),
        SubmitResult::Disabled(GapReason::QueueFull)
    );
    assert!(sink.closes.lock().unwrap().is_empty());
    assert_eq!(emitter.try_recover(now(INITIAL)), SubmitResult::Queued);
    assert_eq!(
        emitter.loss(now(INITIAL), GapReason::DecodeFailed),
        SubmitResult::Queued
    );
    assert_eq!(loss_reasons(&sink), vec![GapReason::DecodeFailed]);
    sink.assert_no_violation();

    // A terminal refusal disables for good.
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    sink.refuse_call(sink.submit_calls(), GapReason::StorageIo);
    assert_eq!(
        emitter.loss(now(INITIAL), GapReason::DecodeFailed),
        SubmitResult::Disabled(GapReason::StorageIo)
    );
    assert_eq!(
        emitter.try_recover(now(INITIAL)),
        SubmitResult::Disabled(GapReason::StorageIo)
    );
    assert_eq!(
        emitter.try_close(now(INITIAL)),
        SubmitResult::Disabled(GapReason::StorageIo)
    );
}

#[test]
fn a_loss_that_fails_validation_disables_the_epoch() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    let bad_clock = CaptureTime {
        wall: "not a timestamp".to_string(),
        ..now(INITIAL)
    };
    assert_eq!(
        emitter.loss(bad_clock, GapReason::DecodeFailed),
        SubmitResult::Disabled(GapReason::LimitExceeded)
    );
    assert_eq!(
        sink.submit_calls(),
        0,
        "nothing the receiver would refuse is sent"
    );
    assert_eq!(
        emitter.try_close(now(INITIAL)),
        SubmitResult::Disabled(GapReason::LimitExceeded),
        "an epoch that could not carry its loss never closes cleanly"
    );
    assert!(sink.closes.lock().unwrap().is_empty());
}

#[test]
fn an_open_refused_for_its_metadata_records_a_loss() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    let mut huge = request_meta(CONN_A, 1);
    huge.label = "x".repeat(4 * 1024 * 1024);
    for _ in 0..2 {
        assert_eq!(
            emitter.open(CaptureSource::WireRequest, huge.clone(), now(INITIAL)),
            Err(GapReason::LimitExceeded)
        );
    }
    assert_eq!(
        loss_reasons(&sink),
        vec![GapReason::LimitExceeded],
        "the epoch stays live and carries the loss once"
    );
    // The epoch is still live and closes cleanly after the loss it carries.
    let key = open_request(&emitter, 2).expect("live");
    assert_eq!(key.stream, 1);
    assert_eq!(
        emitter.emit(&key, now(INITIAL), end()),
        SubmitResult::Queued
    );
    assert_eq!(emitter.try_close(now(INITIAL)), SubmitResult::Queued);
    assert_eq!(
        sink.closes.lock().unwrap().clone(),
        vec![(INITIAL.to_string(), Some(2))]
    );
    sink.assert_no_violation();
}

#[test]
fn a_loss_after_close_is_refused_and_counted() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    assert_eq!(emitter.try_close(now(INITIAL)), SubmitResult::Queued);
    let calls = sink.submit_calls();
    assert_eq!(
        emitter.loss(now(INITIAL), GapReason::DecodeFailed),
        SubmitResult::Disabled(GapReason::ShutdownDeadline)
    );
    assert_eq!(sink.submit_calls(), calls, "nothing follows a close");
    assert!(sink.frames().is_empty());
}

#[test]
fn emit_refuses_a_loss_on_a_payload_stream() {
    let sink = RecordingSink::new();
    let emitter = sink.admit_initial(INITIAL);
    let key = open_request(&emitter, 1).unwrap();
    let calls = sink.submit_calls();
    assert_eq!(
        emitter.emit(
            &key,
            now(INITIAL),
            FrameEvent::Loss {
                reason: GapReason::DecodeFailed
            }
        ),
        SubmitResult::Disabled(GapReason::LimitExceeded),
        "a loss has its own entry point and its own stream, zero"
    );
    assert_eq!(sink.submit_calls(), calls);
    assert_eq!(
        emitter.try_close(now(INITIAL)),
        SubmitResult::Disabled(GapReason::ShutdownDeadline),
        "the stream is still open, as for any local refusal"
    );
    assert_eq!(
        emitter.emit(&key, now(INITIAL), end()),
        SubmitResult::Queued
    );
    assert_eq!(emitter.try_close(now(INITIAL)), SubmitResult::Queued);
    sink.assert_no_violation();
}
