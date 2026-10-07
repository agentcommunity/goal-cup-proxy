//! The daemon's capture ingest receiver, driven over real loopback TCP.
//!
//! Every test binds a real `IngestListener` on 127.0.0.1 port zero and connects with real
//! Tokio TCP streams, the transport the sender uses. Nothing here calls a
//! private validator.
//!
//! The frame encoder, socket driver and recording sink are test-local. The encoder is
//! `serde_json::to_vec` on the capture types behind a big-endian `u32` prefix, an oracle
//! independent of the receiver's decoder. The sink records exactly what crossed the admission
//! boundary, so token and cross-entry assertions inspect what the receiver actually handed on.
//! Waits are bounded by `GUARD`; sequencing uses the receiver's own outcome latch and the
//! sink's item counter, never a sleep standing in for an event.

use goalcup_daemon::ingest::{
    DrainReport, EntryGrant, IngestConfig, IngestEntry, IngestListener, IngestSecret, LossCause,
    ProducerLoss, ProducerOutcome, ValidatedClose, monotonic_clock,
};
use goalcup_protocol::capture::{
    CaptureFrame, CaptureMeta, CaptureSink, CaptureSource, CaptureTime, ConnectionEvent, EndReason,
    EpochAdmission, FrameEvent, GapReason, IngestMessage, MAX_HANDSHAKE_BYTES,
    MAX_INGEST_FRAME_BYTES, MAX_OPEN_STREAMS, MAX_PRODUCERS, StreamKey, SubmitResult, WireIdentity,
};
use serde_json::json;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::time::{Instant, timeout};

/// Upper bound on any single wait. A test that needs it has already failed.
const GUARD: Duration = Duration::from_secs(10);
/// The daemon clock the listener stamps its own observations with.
const CLOCK_ID: &str = "goalcup-daemon-ingest-test";
const CONNECTION: &str = "5e1f0a2b3c4d5e6f708192a3b4c5d6e7";

// ---------------------------------------------------------------------------------------------
// Test sink: a small bounded FIFO with scripted epoch admission and a disabled-state latch.
// ---------------------------------------------------------------------------------------------

/// What crossed into the sink, in order. Shaped like `IngestMessage`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
enum Item {
    Frame(CaptureFrame),
    Close {
        producer: String,
        final_seq: Option<u64>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SinkProducer {
    Live,
    Closed,
    Lost(GapReason),
}

struct SinkState {
    items: Vec<Item>,
    /// Payload frames the bounded Vec admits; a close uses reserved control capacity.
    payload_capacity: usize,
    /// Answers to `try_begin_epoch`, in order; `Ready` once exhausted.
    script: VecDeque<EpochAdmission>,
    begin_calls: Vec<String>,
    submit_calls: usize,
    admitted: BTreeMap<String, SinkProducer>,
    disabled: Option<GapReason>,
}

struct RecordingSink {
    state: Mutex<SinkState>,
    changed: watch::Sender<usize>,
}

impl RecordingSink {
    fn new(payload_capacity: usize) -> Arc<Self> {
        Self::scripted(payload_capacity, Vec::new())
    }

    fn scripted(payload_capacity: usize, script: Vec<EpochAdmission>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(SinkState {
                items: Vec::new(),
                payload_capacity,
                script: script.into(),
                begin_calls: Vec::new(),
                submit_calls: 0,
                admitted: BTreeMap::new(),
                disabled: None,
            }),
            changed: watch::channel(0).0,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SinkState> {
        self.state.lock().unwrap()
    }

    fn items(&self) -> Vec<Item> {
        self.lock().items.clone()
    }

    fn frames(&self) -> Vec<CaptureFrame> {
        self.items()
            .into_iter()
            .filter_map(|item| match item {
                Item::Frame(frame) => Some(frame),
                Item::Close { .. } => None,
            })
            .collect()
    }

    fn begin_calls(&self) -> Vec<String> {
        self.lock().begin_calls.clone()
    }

    fn submit_calls(&self) -> usize {
        self.lock().submit_calls
    }

    /// Replace the answers to later `try_begin_epoch` calls (`Ready` once exhausted).
    fn set_script(&self, script: Vec<EpochAdmission>) {
        self.lock().script = script.into();
    }

    /// Drop queued payload, as the entry owner does when it drains.
    fn drain_payload(&self) {
        self.lock().items.clear();
        self.notify();
    }

    fn notify(&self) {
        self.changed.send_modify(|n| *n += 1);
    }

    /// Wait until at least `count` items are recorded.
    async fn wait_items(&self, count: usize) {
        let mut rx = self.changed.subscribe();
        timeout(GUARD, async {
            while self.lock().items.len() < count {
                rx.changed().await.unwrap();
            }
        })
        .await
        .unwrap_or_else(|_| panic!("the sink never reached {count} items"));
    }
}

impl CaptureSink for RecordingSink {
    fn try_submit(&self, frame: CaptureFrame) -> SubmitResult {
        let mut s = self.lock();
        s.submit_calls += 1;
        if let Some(reason) = s.disabled {
            return SubmitResult::Disabled(reason);
        }
        let producer = frame.stream.producer.clone();
        match s.admitted.get(&producer) {
            Some(SinkProducer::Live) => {}
            Some(SinkProducer::Lost(reason)) => return SubmitResult::Disabled(*reason),
            _ => return SubmitResult::Disabled(GapReason::ProducerLost),
        }
        let payload = s
            .items
            .iter()
            .filter(|item| matches!(item, Item::Frame(_)))
            .count();
        if payload >= s.payload_capacity {
            s.admitted
                .insert(producer, SinkProducer::Lost(GapReason::QueueFull));
            return SubmitResult::Disabled(GapReason::QueueFull);
        }
        s.items.push(Item::Frame(frame));
        drop(s);
        self.notify();
        SubmitResult::Queued
    }

    fn try_begin_epoch(&self, producer: &str) -> EpochAdmission {
        let mut s = self.lock();
        s.begin_calls.push(producer.to_string());
        if let Some(reason) = s.disabled {
            return EpochAdmission::Disabled(reason);
        }
        if s.admitted.contains_key(producer) {
            return EpochAdmission::RetryFresh;
        }
        let answer = s.script.pop_front().unwrap_or(EpochAdmission::Ready);
        if answer == EpochAdmission::Ready {
            s.admitted.insert(producer.to_string(), SinkProducer::Live);
        }
        answer
    }

    fn try_close_epoch(&self, producer: &str, final_seq: Option<u64>) -> SubmitResult {
        let mut s = self.lock();
        if let Some(reason) = s.disabled {
            return SubmitResult::Disabled(reason);
        }
        match s.admitted.get(producer) {
            Some(SinkProducer::Live) => {}
            Some(SinkProducer::Closed) => return SubmitResult::Queued,
            Some(SinkProducer::Lost(reason)) => return SubmitResult::Disabled(*reason),
            None => return SubmitResult::Disabled(GapReason::ProducerLost),
        }
        s.items.push(Item::Close {
            producer: producer.to_string(),
            final_seq,
        });
        s.admitted
            .insert(producer.to_string(), SinkProducer::Closed);
        drop(s);
        self.notify();
        SubmitResult::Queued
    }
}

// ---------------------------------------------------------------------------------------------
// Socket driver and encoder.
// ---------------------------------------------------------------------------------------------

type Conn = TcpStream;

struct Rig {
    listener: Option<IngestListener>,
}

fn test_config() -> IngestConfig {
    let mut config = IngestConfig::new(monotonic_clock(CLOCK_ID));
    config.setup_timeout = Duration::from_secs(3);
    config.close_grace = Duration::from_secs(3);
    config
}

impl Rig {
    fn new() -> Self {
        Self::with(|_| {})
    }

    /// A real listener on 127.0.0.1, port zero.
    fn with(adjust: impl FnOnce(&mut IngestConfig)) -> Self {
        let mut config = test_config();
        adjust(&mut config);
        let listener =
            IngestListener::bind_loopback("127.0.0.1:0".parse().unwrap(), config).unwrap();
        Self {
            listener: Some(listener),
        }
    }

    fn listener(&self) -> &IngestListener {
        self.listener.as_ref().unwrap()
    }

    fn authorize(&self, entry_id: &str, token: &str, sink: Arc<RecordingSink>) -> IngestEntry {
        self.authorize_prior(entry_id, token, sink, 0)
    }

    fn authorize_prior(
        &self,
        entry_id: &str,
        token: &str,
        sink: Arc<RecordingSink>,
        prior_producers: usize,
    ) -> IngestEntry {
        self.listener()
            .authorize(EntryGrant {
                entry_id: entry_id.to_string(),
                secret: IngestSecret::new(token.to_string()).unwrap(),
                sink,
                prior_producers,
            })
            .unwrap()
    }

    async fn connect(&self) -> Conn {
        TcpStream::connect(self.listener().local_addr())
            .await
            .unwrap()
    }

    /// Connect and send a handshake for `producer` on `entry` with `token`.
    async fn producer(&self, entry: &str, token: &str, producer: &str) -> Conn {
        let mut conn = self.connect().await;
        conn.write_all(&handshake(entry, producer, token))
            .await
            .unwrap();
        conn
    }

    async fn shutdown(mut self, within: Duration) -> goalcup_daemon::ingest::ShutdownReport {
        let listener = self.listener.take().unwrap();
        listener.shutdown(Instant::now() + within).await
    }
}

fn framed(body: &[u8]) -> Vec<u8> {
    let mut out = (body.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(body);
    out
}

fn handshake(entry: &str, producer: &str, token: &str) -> Vec<u8> {
    framed(
        &serde_json::to_vec(&json!({"entry_id": entry, "producer": producer, "token": token}))
            .unwrap(),
    )
}

fn item(frame: &CaptureFrame) -> Vec<u8> {
    framed(&serde_json::to_vec(&IngestMessage::Frame(frame.clone())).unwrap())
}

fn items(frames: &[CaptureFrame]) -> Vec<u8> {
    frames.iter().flat_map(item).collect()
}

fn close(producer: &str, final_seq: Option<u64>) -> Vec<u8> {
    framed(
        &serde_json::to_vec(&IngestMessage::Close {
            producer: producer.to_string(),
            final_seq,
        })
        .unwrap(),
    )
}

/// Write, ignoring failure: after a refusal the receiver may already have closed the socket.
async fn send_lossy(conn: &mut Conn, bytes: &[u8]) {
    let _ = conn.write_all(bytes).await;
}

async fn send(conn: &mut Conn, bytes: &[u8]) {
    conn.write_all(bytes).await.unwrap();
}

/// The receiver closes the socket and never writes a byte (no ACK, no rejection frame).
async fn expect_closed(conn: &mut Conn) {
    let mut buf = [0u8; 64];
    match timeout(GUARD, conn.read(&mut buf))
        .await
        .expect("the receiver closes the socket")
    {
        Ok(0) | Err(_) => {}
        Ok(n) => panic!("the receiver never writes, but sent {n} bytes"),
    }
}

/// The socket is still open after `window`: a read neither completes nor fails.
async fn expect_open(conn: &mut Conn, window: Duration) {
    let mut buf = [0u8; 1];
    assert!(
        timeout(window, conn.read(&mut buf)).await.is_err(),
        "the socket must still be open"
    );
}

// ---------------------------------------------------------------------------------------------
// Frame builders.
// ---------------------------------------------------------------------------------------------

fn pid(n: u32) -> String {
    format!("{n:032x}")
}

fn token(tag: &str) -> String {
    format!("{tag}-{}", "0".repeat(40))
}

fn at(producer: &str, seq: u64) -> CaptureTime {
    CaptureTime {
        wall: "2026-09-28T10:00:00.000Z".to_string(),
        mono_ns: 100 + seq,
        clock_id: producer.to_string(),
    }
}

fn frame(producer: &str, seq: u64, stream: u64, event: FrameEvent) -> CaptureFrame {
    CaptureFrame {
        stream: StreamKey {
            producer: producer.to_string(),
            stream,
        },
        seq,
        observed: at(producer, seq),
        event,
    }
}

fn connection_open(producer: &str, seq: u64) -> CaptureFrame {
    frame(
        producer,
        seq,
        0,
        FrameEvent::Connection {
            connection: CONNECTION.to_string(),
            event: ConnectionEvent::Open {
                alpn: Some("http/1.1".to_string()),
                already_open: false,
            },
        },
    )
}

fn native_meta(hint: &str) -> CaptureMeta {
    CaptureMeta {
        wire: None,
        label: "native:rollout.jsonl#g0".to_string(),
        protocol_hint: Some(hint.to_string()),
        method: None,
        host: None,
        path: None,
        status: None,
        content_type: None,
    }
}

fn open(producer: &str, seq: u64, stream: u64) -> CaptureFrame {
    frame(
        producer,
        seq,
        stream,
        FrameEvent::Open {
            source: CaptureSource::Native,
            metadata: native_meta("codex-jsonl"),
        },
    )
}

fn bytes(producer: &str, seq: u64, stream: u64, offset: u64, data: &[u8]) -> CaptureFrame {
    frame(
        producer,
        seq,
        stream,
        FrameEvent::Bytes {
            offset,
            data: data.to_vec(),
        },
    )
}

fn end(producer: &str, seq: u64, stream: u64) -> CaptureFrame {
    frame(
        producer,
        seq,
        stream,
        FrameEvent::End {
            reason: EndReason::Complete,
        },
    )
}

fn gap(producer: &str, seq: u64, stream: u64) -> CaptureFrame {
    frame(
        producer,
        seq,
        stream,
        FrameEvent::Gap {
            reason: GapReason::SourceChanged,
            lost_bytes: None,
        },
    )
}

fn recovery(producer: &str, seq: u64, previous: &str) -> CaptureFrame {
    frame(
        producer,
        seq,
        0,
        FrameEvent::Recovery {
            previous_producer: previous.to_string(),
            reason: GapReason::IngestLost,
        },
    )
}

/// A recovered epoch's re-observation of a socket that survived (`already_open: true`).
fn reobserve(producer: &str, seq: u64) -> CaptureFrame {
    frame(
        producer,
        seq,
        0,
        FrameEvent::Connection {
            connection: CONNECTION.to_string(),
            event: ConnectionEvent::Open {
                alpn: Some("http/1.1".to_string()),
                already_open: true,
            },
        },
    )
}

// ---------------------------------------------------------------------------------------------
// Outcome helpers.
// ---------------------------------------------------------------------------------------------

/// Collect exactly `count` outcomes from the entry's latch, waiting on the receiver.
async fn outcomes(entry: &IngestEntry, count: usize) -> Vec<ProducerOutcome> {
    let mut seen = Vec::new();
    timeout(GUARD, async {
        while seen.len() < count {
            seen.extend(entry.take_outcomes());
            if seen.len() < count {
                entry.wait_outcome().await;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("expected {count} outcomes, saw {seen:?}"));
    assert_eq!(
        seen.len(),
        count,
        "no more outcomes than expected: {seen:?}"
    );
    seen
}

async fn one_outcome(entry: &IngestEntry) -> ProducerOutcome {
    outcomes(entry, 1).await.remove(0)
}

async fn loss(entry: &IngestEntry) -> ProducerLoss {
    match one_outcome(entry).await {
        ProducerOutcome::Lost(loss) => loss,
        other => panic!("expected a loss, got {other:?}"),
    }
}

async fn clean_close(entry: &IngestEntry) -> ValidatedClose {
    match one_outcome(entry).await {
        ProducerOutcome::Closed(close) => close,
        other => panic!("expected a validated close, got {other:?}"),
    }
}

fn assert_loss(
    loss: &ProducerLoss,
    producer: &str,
    cause: LossCause,
    reason: GapReason,
    admitted_through: Option<u64>,
) {
    assert_eq!(loss.producer, producer, "{loss:?}");
    assert_eq!(loss.cause, cause, "{loss:?}");
    assert_eq!(loss.reason, reason, "{loss:?}");
    assert_eq!(loss.admitted_through, admitted_through, "{loss:?}");
    assert_eq!(
        loss.observed.clock_id, CLOCK_ID,
        "the daemon clock, not the producer's"
    );
    loss.observed.validate().unwrap();
}

/// Poll a synchronous condition under the guard (listener counters have no event stream).
async fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
    timeout(GUARD, async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("never observed: {what}"));
}

fn golden() -> (Vec<u8>, serde_json::Value) {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../goalcup-protocol/tests/fixtures/m2-ingest"
    );
    let hex_text = std::fs::read_to_string(format!("{dir}/golden-stream.hex")).unwrap();
    let bytes = hex::decode(hex_text.split_whitespace().collect::<String>()).unwrap();
    let decoded: serde_json::Value =
        serde_json::from_slice(&std::fs::read(format!("{dir}/golden-stream.json")).unwrap())
            .unwrap();
    (bytes, decoded)
}

// ---------------------------------------------------------------------------------------------
// 1. Conformance fixture and admission branches.
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn shared_handshake_fixture_reaches_only_bound_entry() {
    let (golden_bytes, decoded) = golden();
    let golden_token = decoded["handshake"]["token"].as_str().unwrap().to_string();
    let producer = decoded["handshake"]["producer"]
        .as_str()
        .unwrap()
        .to_string();
    let messages: Vec<IngestMessage> = serde_json::from_value(decoded["messages"].clone()).unwrap();
    let expected: Vec<Item> = messages
        .iter()
        .map(|message| match message {
            IngestMessage::Frame(frame) => Item::Frame(frame.clone()),
            IngestMessage::Close {
                producer,
                final_seq,
            } => Item::Close {
                producer: producer.clone(),
                final_seq: *final_seq,
            },
        })
        .collect();

    let rig = Rig::new();
    let bound = RecordingSink::new(16);
    let other = RecordingSink::new(16);
    let entry = rig.authorize("entry-m2-0001", &golden_token, bound.clone());
    let decoy = rig.authorize("entry-m2-0002", &token("decoy"), other.clone());

    let mut conn = rig.connect().await;
    send(&mut conn, &golden_bytes).await;
    conn.shutdown().await.unwrap();

    let closed = clean_close(&entry).await;
    assert_eq!(closed.producer, producer);
    assert_eq!(closed.final_seq, Some(3));
    assert_eq!(closed.observed.clock_id, CLOCK_ID);
    assert_eq!(
        bound.items(),
        expected,
        "exactly the golden frames, then close"
    );
    assert_eq!(bound.begin_calls(), vec![producer.clone()]);
    assert!(other.items().is_empty() && other.begin_calls().is_empty());
    assert!(decoy.take_outcomes().is_empty());
    expect_closed(&mut conn).await;

    // The same bytes under a different credential for that entry reach nothing.
    let rig = Rig::new();
    let sink = RecordingSink::new(16);
    let _entry = rig.authorize("entry-m2-0001", &token("not-the-golden"), sink.clone());
    let mut conn = rig.connect().await;
    send_lossy(&mut conn, &golden_bytes).await;
    expect_closed(&mut conn).await;
    assert!(sink.items().is_empty() && sink.begin_calls().is_empty());
    eventually("the refusal is counted", || {
        rig.listener().stats().handshakes_refused == 1
    })
    .await;
}

#[tokio::test]
async fn sink_admission_branches_are_honoured() {
    let rig = Rig::with(|c| {
        c.setup_timeout = Duration::from_millis(600);
    });
    let t = token("branches");

    // Pending twice, then Ready: the same ID is polled and then admitted.
    let pending =
        RecordingSink::scripted(8, vec![EpochAdmission::Pending, EpochAdmission::Pending]);
    let entry = rig.authorize("e-pending", &t, pending.clone());
    let p = pid(1);
    let mut conn = rig.producer("e-pending", &t, &p).await;
    send(&mut conn, &item(&connection_open(&p, 0))).await;
    pending.wait_items(1).await;
    assert_eq!(pending.begin_calls(), vec![p.clone(); 3]);
    assert_eq!(entry.snapshot().producers_unsettled, 1);

    // RetryFresh: refused and the ID is consumed; the same ID never reaches the sink again.
    let retry = RecordingSink::scripted(8, vec![EpochAdmission::RetryFresh]);
    let retry_entry = rig.authorize("e-retry", &t, retry.clone());
    let mut conn = rig.producer("e-retry", &t, &p).await;
    send_lossy(&mut conn, &item(&connection_open(&p, 0))).await;
    expect_closed(&mut conn).await;
    let mut again = rig.producer("e-retry", &t, &p).await;
    expect_closed(&mut again).await;
    assert_eq!(
        retry.begin_calls(),
        vec![p.clone()],
        "a consumed ID is not re-proposed"
    );
    assert!(retry.items().is_empty());
    assert_eq!(retry_entry.snapshot().admissions_refused, 1);
    assert_eq!(retry_entry.snapshot().producers_consumed, 1);
    assert!(
        retry_entry.take_outcomes().is_empty(),
        "never admitted, so no latch"
    );

    // Disabled: a locally known terminal answer, same treatment.
    let disabled =
        RecordingSink::scripted(8, vec![EpochAdmission::Disabled(GapReason::StorageFull)]);
    let disabled_entry = rig.authorize("e-disabled", &t, disabled.clone());
    let mut conn = rig.producer("e-disabled", &t, &p).await;
    expect_closed(&mut conn).await;
    assert_eq!(disabled_entry.snapshot().admissions_refused, 1);
    assert!(disabled.items().is_empty());

    // Pending past the setup deadline: refused, bounded, never admitted.
    let stuck = RecordingSink::scripted(8, vec![EpochAdmission::Pending; 1000]);
    let stuck_entry = rig.authorize("e-stuck", &t, stuck.clone());
    let started = Instant::now();
    let mut conn = rig.producer("e-stuck", &t, &p).await;
    expect_closed(&mut conn).await;
    assert!(started.elapsed() >= Duration::from_millis(500));
    assert!(stuck.begin_calls().len() > 1, "polled more than once");
    assert_eq!(stuck_entry.snapshot().admissions_refused, 1);
    assert!(stuck.items().is_empty());
}

#[tokio::test]
async fn unknown_provider_hint_reaches_sink() {
    let rig = Rig::new();
    let sink = RecordingSink::new(8);
    let t = token("hint");
    let entry = rig.authorize("e", &t, sink.clone());
    let p = pid(7);
    let wire_open = frame(
        &p,
        1,
        1,
        FrameEvent::Open {
            source: CaptureSource::WireRequest,
            metadata: CaptureMeta {
                wire: Some(WireIdentity {
                    connection: CONNECTION.to_string(),
                    exchange: 1,
                    message_index: None,
                }),
                label: "POST /v9/unheard-of".to_string(),
                protocol_hint: Some("a-provider-nobody-decodes-yet".to_string()),
                method: Some("POST".to_string()),
                host: Some("api.example.test".to_string()),
                path: Some("/v9/unheard-of".to_string()),
                status: None,
                content_type: Some("application/x-unknown".to_string()),
            },
        },
    );
    let sent = vec![
        connection_open(&p, 0),
        wire_open,
        bytes(&p, 2, 1, 0, b"opaque"),
        end(&p, 3, 1),
    ];
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &items(&sent)).await;
    send(&mut conn, &close(&p, Some(3))).await;
    conn.shutdown().await.unwrap();
    assert_eq!(clean_close(&entry).await.final_seq, Some(3));
    assert_eq!(
        sink.frames(),
        sent,
        "generic receiver: framing, not provider semantics"
    );
}

// ---------------------------------------------------------------------------------------------
// 2. Bounded parsing.
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn oversized_length_prefix_allocates_no_body() {
    // A long setup deadline: only an immediate refusal can close these sockets in time.
    let rig = Rig::with(|c| {
        c.setup_timeout = Duration::from_secs(60);
    });
    let sink = RecordingSink::new(8);
    let t = token("oversized");
    let entry = rig.authorize("e", &t, sink.clone());

    for declared in [MAX_HANDSHAKE_BYTES as u32 + 1, u32::MAX] {
        let mut conn = rig.connect().await;
        send(&mut conn, &declared.to_be_bytes()).await;
        // No body follows. Reading one would hold the socket until the setup deadline.
        expect_closed(&mut conn).await;
    }
    eventually("both handshake refusals are counted", || {
        rig.listener().stats().handshakes_refused == 2
    })
    .await;
    assert!(sink.begin_calls().is_empty());

    for (n, declared) in [MAX_INGEST_FRAME_BYTES as u32 + 1, u32::MAX]
        .into_iter()
        .enumerate()
    {
        let p = pid(10 + n as u32);
        let mut conn = rig.producer("e", &t, &p).await;
        send(&mut conn, &item(&connection_open(&p, 0))).await;
        send(&mut conn, &declared.to_be_bytes()).await;
        expect_closed(&mut conn).await;
        assert_loss(
            &loss(&entry).await,
            &p,
            LossCause::OversizedItem,
            GapReason::LimitExceeded,
            Some(0),
        );
    }
    assert_eq!(sink.frames().len(), 2, "only the two admitted controls");
}

#[tokio::test]
async fn slow_handshake_is_bounded() {
    let rig = Rig::with(|c| {
        c.setup_timeout = Duration::from_secs(2);
        c.max_pending_handshakes = 2;
    });
    let sink = RecordingSink::new(8);
    let t = token("slow");
    let entry = rig.authorize("e", &t, sink.clone());

    // Two silent sockets fill the unauthenticated bound; one sends a single prefix byte.
    let mut silent = rig.connect().await;
    let mut trickle = rig.connect().await;
    send(&mut trickle, &[0]).await;
    eventually("two pending handshakes", || {
        rig.listener().stats().pending_handshakes == 2
    })
    .await;

    // A third unauthenticated socket is closed on accept, not queued behind them.
    let mut third = rig.connect().await;
    expect_closed(&mut third).await;
    eventually("the third socket was refused at capacity", || {
        rig.listener().stats().rejected_at_capacity == 1
    })
    .await;
    expect_open(&mut silent, Duration::from_millis(50)).await;

    // The setup deadline closes both slow sockets and frees the bound.
    expect_closed(&mut silent).await;
    expect_closed(&mut trickle).await;
    eventually("both time out", || {
        let stats = rig.listener().stats();
        stats.handshakes_timed_out == 2 && stats.pending_handshakes == 0
    })
    .await;

    // A prompt, authenticated producer is admitted afterwards.
    let p = pid(1);
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &item(&connection_open(&p, 0))).await;
    sink.wait_items(1).await;
    assert_eq!(entry.snapshot().producers_consumed, 1);
}

#[tokio::test]
async fn authenticated_producers_do_not_hold_the_handshake_bound() {
    // One unauthenticated slot: live producers must release it once they authenticate.
    let rig = Rig::with(|c| c.max_pending_handshakes = 1);
    let (sink_a, sink_b) = (RecordingSink::new(8), RecordingSink::new(8));
    let (ta, tb) = (token("bound-a"), token("bound-b"));
    let _entry_a = rig.authorize("a", &ta, sink_a.clone());
    let _entry_b = rig.authorize("b", &tb, sink_b.clone());

    let (pa, pb, pc) = (pid(1), pid(2), pid(3));
    let mut conn_a = rig.producer("a", &ta, &pa).await;
    send(&mut conn_a, &item(&connection_open(&pa, 0))).await;
    sink_a.wait_items(1).await;
    eventually("the first producer released its handshake slot", || {
        rig.listener().stats().pending_handshakes == 0
    })
    .await;

    // Two more live producers, on another entry and on the same one, while A stays live.
    let mut conn_b = rig.producer("b", &tb, &pb).await;
    send(&mut conn_b, &item(&connection_open(&pb, 0))).await;
    sink_b.wait_items(1).await;
    let mut conn_c = rig.producer("a", &ta, &pc).await;
    send(&mut conn_c, &item(&connection_open(&pc, 0))).await;
    sink_a.wait_items(2).await;

    send(&mut conn_a, &item(&open(&pa, 1, 1))).await;
    sink_a.wait_items(3).await;
    let stats = rig.listener().stats();
    assert_eq!(
        (stats.pending_handshakes, stats.rejected_at_capacity),
        (0, 0)
    );
}

#[tokio::test]
async fn unknown_frame_field_rejected() {
    let rig = Rig::new();
    let sink = RecordingSink::new(8);
    let t = token("unknown-field");
    let entry = rig.authorize("e", &t, sink.clone());

    let mut top = serde_json::to_value(IngestMessage::Frame(connection_open(&pid(1), 0))).unwrap();
    top["extra"] = json!(1);
    let mut nested = serde_json::to_value(IngestMessage::Frame(open(&pid(2), 0, 1))).unwrap();
    nested["event"]["metadata"]["headers"] = json!({"authorization": "x"});
    let mut close_extra = serde_json::to_value(IngestMessage::Close {
        producer: pid(3),
        final_seq: None,
    })
    .unwrap();
    close_extra["ack"] = json!(true);

    for (n, value) in [top, nested, close_extra].into_iter().enumerate() {
        let p = pid(n as u32 + 1);
        let mut conn = rig.producer("e", &t, &p).await;
        send_lossy(&mut conn, &framed(&serde_json::to_vec(&value).unwrap())).await;
        expect_closed(&mut conn).await;
        assert_loss(
            &loss(&entry).await,
            &p,
            LossCause::Malformed,
            GapReason::ProducerLost,
            None,
        );
    }
    assert!(sink.items().is_empty());

    // An unknown handshake field is refused before any producer is consumed.
    let mut conn = rig.connect().await;
    let body = serde_json::to_vec(&json!({
        "entry_id": "e", "producer": pid(9), "token": t, "resume": true
    }))
    .unwrap();
    send_lossy(&mut conn, &framed(&body)).await;
    expect_closed(&mut conn).await;
    assert_eq!(entry.snapshot().producers_consumed, 3);
}

#[tokio::test]
async fn invalid_json_does_not_leak_payload() {
    const PLANTED: &str = "sk-live-PLANTED-0123456789";
    let rig = Rig::new();
    let sink = RecordingSink::new(8);
    let t = token("leak");
    let entry = rig.authorize("e", &t, sink.clone());

    let p = pid(1);
    let mut conn = rig.producer("e", &t, &p).await;
    let body = format!(r#"{{"kind":"frame","stream":"{PLANTED}""#);
    send_lossy(&mut conn, &framed(body.as_bytes())).await;
    expect_closed(&mut conn).await;
    let lost = loss(&entry).await;
    assert_loss(
        &lost,
        &p,
        LossCause::Malformed,
        GapReason::ProducerLost,
        None,
    );

    // A malformed handshake carrying the same text.
    let mut conn = rig.connect().await;
    send_lossy(
        &mut conn,
        &framed(format!(r#"{{"entry_id":"{PLANTED}"#).as_bytes()),
    )
    .await;
    expect_closed(&mut conn).await;
    eventually("handshake refusal counted", || {
        rig.listener().stats().handshakes_refused == 1
    })
    .await;

    let diagnostics = [
        format!("{lost:?}"),
        format!("{entry:?}"),
        format!("{:?}", entry.snapshot()),
        format!("{:?}", rig.listener()),
        format!("{:?}", rig.listener().stats()),
    ];
    for text in diagnostics {
        assert!(!text.contains(PLANTED), "{text}");
    }
    assert!(sink.items().is_empty());
}

// ---------------------------------------------------------------------------------------------
// 3. Authorization and one producer lifetime.
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn token_cannot_cross_entry() {
    let rig = Rig::new();
    let (sink_a, sink_b) = (RecordingSink::new(8), RecordingSink::new(8));
    let (ta, tb) = (token("entry-a"), token("entry-b"));
    let entry_a = rig.authorize("a", &ta, sink_a.clone());
    let entry_b = rig.authorize("b", &tb, sink_b.clone());

    for (entry, secret, p) in [("a", &tb, pid(1)), ("b", &ta, pid(2)), ("c", &ta, pid(3))] {
        let mut conn = rig.producer(entry, secret, &p).await;
        send_lossy(&mut conn, &item(&connection_open(&p, 0))).await;
        expect_closed(&mut conn).await;
    }
    assert!(sink_a.begin_calls().is_empty() && sink_b.begin_calls().is_empty());
    assert_eq!(entry_a.snapshot().producers_consumed, 0);
    assert_eq!(entry_b.snapshot().producers_consumed, 0);

    // The right pairing reaches only its own entry.
    let p = pid(4);
    let mut conn = rig.producer("a", &ta, &p).await;
    send(&mut conn, &item(&connection_open(&p, 0))).await;
    sink_a.wait_items(1).await;
    assert!(sink_b.items().is_empty());
}

#[tokio::test]
async fn producer_cannot_be_rebound() {
    let rig = Rig::new();
    let sink = RecordingSink::new(8);
    let t = token("rebind");
    let entry = rig.authorize("e", &t, sink.clone());

    // Frames must carry the handshake's producer.
    let (p, other) = (pid(1), pid(2));
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &item(&connection_open(&p, 0))).await;
    send_lossy(&mut conn, &item(&connection_open(&other, 1))).await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &p,
        LossCause::ForeignProducer,
        GapReason::ProducerLost,
        Some(0),
    );

    // So must its Close.
    let q = pid(3);
    let mut conn = rig.producer("e", &t, &q).await;
    send_lossy(&mut conn, &close(&other, None)).await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &q,
        LossCause::ForeignProducer,
        GapReason::ProducerLost,
        None,
    );
    assert_eq!(sink.frames(), vec![connection_open(&p, 0)]);
    assert!(sink.items().iter().all(|i| matches!(i, Item::Frame(_))));
}

#[tokio::test]
async fn second_connection_is_not_resume() {
    let rig = Rig::new();
    let sink = RecordingSink::new(16);
    let t = token("second");
    let entry = rig.authorize("e", &t, sink.clone());
    let p = pid(1);

    let mut first = rig.producer("e", &t, &p).await;
    send(&mut first, &item(&connection_open(&p, 0))).await;
    sink.wait_items(1).await;

    // A second connection for a live producer is refused; the first is untouched.
    let mut second = rig.producer("e", &t, &p).await;
    send_lossy(&mut second, &item(&open(&p, 1, 1))).await;
    expect_closed(&mut second).await;
    send(&mut first, &item(&open(&p, 1, 1))).await;
    sink.wait_items(2).await;

    // After that producer is lost, its ID still never reconnects.
    first.shutdown().await.unwrap();
    let lost = loss(&entry).await;
    assert_eq!(lost.admitted_through, Some(1));
    let mut third = rig.producer("e", &t, &p).await;
    expect_closed(&mut third).await;
    assert_eq!(
        sink.begin_calls(),
        vec![p.clone()],
        "one admission per ID, ever"
    );
    assert_eq!(sink.frames().len(), 2);
}

#[tokio::test]
async fn handshake_admitted_then_lost_before_payload_consumes_id() {
    let rig = Rig::new();
    let sink = RecordingSink::new(16);
    let t = token("zero-frames");
    let entry = rig.authorize("e", &t, sink.clone());
    let p = pid(1);

    let mut conn = rig.producer("e", &t, &p).await;
    conn.shutdown().await.unwrap();
    assert_loss(
        &loss(&entry).await,
        &p,
        LossCause::Disconnected,
        GapReason::IngestLost,
        None,
    );

    // No same-ID exception for zero frames.
    let mut again = rig.producer("e", &t, &p).await;
    expect_closed(&mut again).await;

    // A permitted fresh ID succeeds.
    let fresh = pid(2);
    let mut conn = rig.producer("e", &t, &fresh).await;
    send(&mut conn, &item(&connection_open(&fresh, 0))).await;
    sink.wait_items(1).await;
    assert_eq!(sink.begin_calls(), vec![p, fresh]);
}

#[tokio::test]
async fn credential_never_reaches_sink_or_diagnostics() {
    let secret = format!("TOKEN-SENTINEL-{}", "7".repeat(40));
    let wrong = format!("WRONG-SENTINEL-{}", "8".repeat(40));
    let rig = Rig::new();
    let sink = RecordingSink::new(16);
    let entry = rig.authorize("e", &secret, sink.clone());

    let p = pid(1);
    let mut conn = rig.producer("e", &secret, &p).await;
    send(
        &mut conn,
        &items(&[connection_open(&p, 0), open(&p, 1, 1), end(&p, 2, 1)]),
    )
    .await;
    send(&mut conn, &close(&p, Some(2))).await;
    conn.shutdown().await.unwrap();
    let closed = clean_close(&entry).await;

    let mut refused = rig.producer("e", &wrong, &pid(2)).await;
    expect_closed(&mut refused).await;

    let duplicate = rig
        .listener()
        .authorize(EntryGrant {
            entry_id: "e".to_string(),
            secret: IngestSecret::new(secret.clone()).unwrap(),
            sink: sink.clone(),
            prior_producers: 0,
        })
        .unwrap_err();
    let grant = EntryGrant {
        entry_id: "other".to_string(),
        secret: IngestSecret::new(secret.clone()).unwrap(),
        sink: sink.clone(),
        prior_producers: 0,
    };
    let generated = IngestSecret::generate();
    assert_eq!(generated.expose().len(), 64);

    let mut texts = vec![
        format!("{closed:?}"),
        format!("{entry:?}"),
        format!("{:?}", entry.snapshot()),
        format!("{:?}", rig.listener()),
        format!("{:?}", rig.listener().stats()),
        format!("{:?}", IngestSecret::new(secret.clone()).unwrap()),
        format!("{duplicate:?}"),
        duplicate.to_string(),
        format!("{grant:?}"),
        format!("{:?}", test_config()),
    ];
    texts.extend(
        sink.frames()
            .iter()
            .map(|f| serde_json::to_string(f).unwrap()),
    );
    texts.push(format!("{:?}", sink.items()));
    for text in &texts {
        assert!(
            !text.contains(&secret) && !text.contains("TOKEN-SENTINEL"),
            "{text}"
        );
        assert!(
            !text.contains(&wrong) && !text.contains("WRONG-SENTINEL"),
            "{text}"
        );
    }
    assert!(!format!("{generated:?}").contains(generated.expose()));
}

#[tokio::test]
async fn two_entries_keep_separate_queues_and_losses() {
    let rig = Rig::new();
    let small = RecordingSink::new(2);
    let roomy = RecordingSink::new(64);
    let (ta, tb) = (token("small"), token("roomy"));
    let entry_a = rig.authorize("a", &ta, small.clone());
    let entry_b = rig.authorize("b", &tb, roomy.clone());
    let (pa, pb) = (pid(1), pid(2));

    // Both producers at once; A overflows its own sink.
    let mut conn_a = rig.producer("a", &ta, &pa).await;
    let mut conn_b = rig.producer("b", &tb, &pb).await;
    send(
        &mut conn_b,
        &items(&[connection_open(&pb, 0), open(&pb, 1, 1)]),
    )
    .await;
    send_lossy(
        &mut conn_a,
        &items(&[
            connection_open(&pa, 0),
            open(&pa, 1, 1),
            bytes(&pa, 2, 1, 0, b"x"),
        ]),
    )
    .await;
    expect_closed(&mut conn_a).await;
    assert_loss(
        &loss(&entry_a).await,
        &pa,
        LossCause::SinkRefused,
        GapReason::QueueFull,
        Some(1),
    );

    // B keeps going: its queue, stream count and latch are its own.
    send(
        &mut conn_b,
        &items(&[bytes(&pb, 2, 1, 0, b"y"), end(&pb, 3, 1)]),
    )
    .await;
    send(&mut conn_b, &close(&pb, Some(3))).await;
    conn_b.shutdown().await.unwrap();
    assert_eq!(clean_close(&entry_b).await.final_seq, Some(3));
    assert_eq!(roomy.frames().len(), 4);
    assert_eq!(
        entry_a.snapshot().open_streams,
        1,
        "A's lost stream stays counted"
    );
    assert_eq!(entry_b.snapshot().open_streams, 0);
    assert!(entry_a.take_outcomes().is_empty() && entry_b.take_outcomes().is_empty());
}

// ---------------------------------------------------------------------------------------------
// 4. Sequence, stream state and source offsets.
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn interleaved_streams_use_one_producer_sequence() {
    let rig = Rig::new();
    let sink = RecordingSink::new(64);
    let t = token("interleave");
    let entry = rig.authorize("e", &t, sink.clone());

    let p = pid(1);
    let sent = vec![
        connection_open(&p, 0),
        open(&p, 1, 1),
        open(&p, 2, 2),
        bytes(&p, 3, 1, 0, b"ab"),
        bytes(&p, 4, 2, 0, b"xyz"),
        bytes(&p, 5, 1, 2, b"cd"),
        end(&p, 6, 2),
        end(&p, 7, 1),
    ];
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &items(&sent)).await;
    send(&mut conn, &close(&p, Some(7))).await;
    conn.shutdown().await.unwrap();
    assert_eq!(clean_close(&entry).await.final_seq, Some(7));
    assert_eq!(sink.frames(), sent);

    // A per-stream counter restarting at a stream is not the producer sequence.
    let q = pid(2);
    let mut conn = rig.producer("e", &t, &q).await;
    send_lossy(
        &mut conn,
        &items(&[
            open(&q, 0, 1),
            open(&q, 1, 2),
            bytes(&q, 2, 1, 0, b"a"),
            bytes(&q, 1, 2, 0, b"b"),
        ]),
    )
    .await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &q,
        LossCause::Sequence,
        GapReason::ProducerLost,
        Some(2),
    );
}

#[tokio::test]
async fn sequence_gap_latches_disabled() {
    let rig = Rig::new();
    let sink = RecordingSink::new(64);
    let t = token("seq-gap");
    let entry = rig.authorize("e", &t, sink.clone());
    let p = pid(1);

    let mut conn = rig.producer("e", &t, &p).await;
    send_lossy(
        &mut conn,
        &items(&[
            connection_open(&p, 0),
            open(&p, 1, 1),
            bytes(&p, 3, 1, 0, b"skipped two"),
        ]),
    )
    .await;
    // The missing sequence number arriving late does not repair the epoch.
    send_lossy(&mut conn, &item(&bytes(&p, 2, 1, 0, b"late"))).await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &p,
        LossCause::Sequence,
        GapReason::ProducerLost,
        Some(1),
    );
    assert_eq!(sink.frames(), vec![connection_open(&p, 0), open(&p, 1, 1)]);

    // A sequence that does not start at zero is refused the same way.
    let q = pid(2);
    let mut conn = rig.producer("e", &t, &q).await;
    send_lossy(&mut conn, &item(&connection_open(&q, 1))).await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &q,
        LossCause::Sequence,
        GapReason::ProducerLost,
        None,
    );
}

#[tokio::test]
async fn offset_overlap_latches_disabled() {
    let rig = Rig::new();
    let sink = RecordingSink::new(64);
    let t = token("overlap");
    let entry = rig.authorize("e", &t, sink.clone());

    // Overlap.
    let p = pid(1);
    let mut conn = rig.producer("e", &t, &p).await;
    send_lossy(
        &mut conn,
        &items(&[
            open(&p, 0, 1),
            bytes(&p, 1, 1, 0, b"0123456789"),
            bytes(&p, 2, 1, 5, b"56789abcde"),
        ]),
    )
    .await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &p,
        LossCause::Offset,
        GapReason::ProducerLost,
        Some(1),
    );

    // An unexplained hole: no Gap precedes the jump.
    let q = pid(2);
    let mut conn = rig.producer("e", &t, &q).await;
    send_lossy(
        &mut conn,
        &items(&[
            open(&q, 0, 1),
            bytes(&q, 1, 1, 0, b"0123"),
            bytes(&q, 2, 1, 9, b"9"),
        ]),
    )
    .await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &q,
        LossCause::Offset,
        GapReason::ProducerLost,
        Some(1),
    );

    // Bytes, End or Gap on a stream that is not open.
    for (n, stray) in [
        bytes(&pid(3), 0, 4, 0, b"x"),
        end(&pid(4), 0, 4),
        gap(&pid(5), 0, 4),
    ]
    .into_iter()
    .enumerate()
    {
        let r = pid(3 + n as u32);
        let mut conn = rig.producer("e", &t, &r).await;
        send_lossy(&mut conn, &item(&stray)).await;
        expect_closed(&mut conn).await;
        assert_loss(
            &loss(&entry).await,
            &r,
            LossCause::StreamState,
            GapReason::ProducerLost,
            None,
        );
    }
}

#[tokio::test]
async fn bytes_resume_past_offset_only_after_gap() {
    let rig = Rig::new();
    let sink = RecordingSink::new(64);
    let t = token("cp-a3");
    let entry = rig.authorize("e", &t, sink.clone());

    // Contiguous, then a Gap, then a forward jump, then contiguous again: all admitted.
    let p = pid(1);
    let sent = vec![
        open(&p, 0, 1),
        bytes(&p, 1, 1, 0, b"abcd"),
        gap(&p, 2, 1),
        bytes(&p, 3, 1, 100, b"wxyz"),
        bytes(&p, 4, 1, 104, b"!"),
        open(&p, 5, 2),
        gap(&p, 6, 2),
        bytes(&p, 7, 2, 50, b"first bytes after a gap"),
        end(&p, 8, 2),
        end(&p, 9, 1),
    ];
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &items(&sent)).await;
    send(&mut conn, &close(&p, Some(9))).await;
    conn.shutdown().await.unwrap();
    assert_eq!(clean_close(&entry).await.final_seq, Some(9));
    assert_eq!(sink.frames(), sent);

    // The first Bytes of a stream starts at zero unless a Gap came first.
    let q = pid(2);
    let mut conn = rig.producer("e", &t, &q).await;
    send_lossy(
        &mut conn,
        &items(&[open(&q, 0, 1), bytes(&q, 1, 1, 7, b"x")]),
    )
    .await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &q,
        LossCause::Offset,
        GapReason::ProducerLost,
        Some(0),
    );

    // A Gap permits a jump forward, never an overlap.
    let r = pid(3);
    let mut conn = rig.producer("e", &t, &r).await;
    send_lossy(
        &mut conn,
        &items(&[
            open(&r, 0, 1),
            bytes(&r, 1, 1, 0, b"abcd"),
            gap(&r, 2, 1),
            bytes(&r, 3, 1, 2, b"cd"),
        ]),
    )
    .await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &r,
        LossCause::Offset,
        GapReason::ProducerLost,
        Some(2),
    );

    // The Gap's permission is spent by the next Bytes.
    let s = pid(4);
    let mut conn = rig.producer("e", &t, &s).await;
    send_lossy(
        &mut conn,
        &items(&[
            open(&s, 0, 1),
            gap(&s, 1, 1),
            bytes(&s, 2, 1, 10, b"a"),
            bytes(&s, 3, 1, 20, b"b"),
        ]),
    )
    .await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &s,
        LossCause::Offset,
        GapReason::ProducerLost,
        Some(2),
    );

    // The boundary: after a Gap that lost nothing, Bytes at exactly the next offset is
    // admitted (any offset >= next), and so is a first Bytes at zero after a leading Gap.
    let u = pid(5);
    let boundary = vec![
        open(&u, 0, 1),
        bytes(&u, 1, 1, 0, b"abc"),
        gap(&u, 2, 1),
        bytes(&u, 3, 1, 3, b"def"),
        open(&u, 4, 2),
        gap(&u, 5, 2),
        bytes(&u, 6, 2, 0, b"x"),
        end(&u, 7, 2),
        end(&u, 8, 1),
    ];
    let mut conn = rig.producer("e", &t, &u).await;
    send(&mut conn, &items(&boundary)).await;
    send(&mut conn, &close(&u, Some(8))).await;
    conn.shutdown().await.unwrap();
    assert_eq!(clean_close(&entry).await.final_seq, Some(8));
}

#[tokio::test]
async fn offset_overflow_is_refused() {
    let rig = Rig::new();
    let sink = RecordingSink::new(64);
    let t = token("overflow");
    let entry = rig.authorize("e", &t, sink.clone());

    // After a Gap, a stream may resume right at the top of the offset space...
    let p = pid(1);
    let edge = vec![
        open(&p, 0, 1),
        gap(&p, 1, 1),
        bytes(&p, 2, 1, u64::MAX - 3, b"abc"),
    ];
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &items(&edge)).await;
    sink.wait_items(3).await;
    // ...but no byte can follow it: offset plus length would overflow.
    send_lossy(&mut conn, &item(&bytes(&p, 3, 1, u64::MAX, b"d"))).await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &p,
        LossCause::Invalid,
        GapReason::ProducerLost,
        Some(2),
    );

    // A single frame whose own range overflows never reaches the sink either.
    let q = pid(2);
    let mut conn = rig.producer("e", &t, &q).await;
    send_lossy(
        &mut conn,
        &items(&[
            open(&q, 0, 1),
            gap(&q, 1, 1),
            bytes(&q, 2, 1, u64::MAX - 1, b"four"),
        ]),
    )
    .await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &q,
        LossCause::Invalid,
        GapReason::ProducerLost,
        Some(1),
    );
    assert_eq!(sink.frames().len(), 5);
}

#[tokio::test]
async fn stream_limit_enforced() {
    let rig = Rig::new();
    let sink = RecordingSink::new(1024);
    let t = token("streams");
    let entry = rig.authorize("e", &t, sink.clone());

    let p = pid(1);
    let opens: Vec<CaptureFrame> = (0..MAX_OPEN_STREAMS as u64)
        .map(|n| open(&p, n, n + 1))
        .collect();
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &items(&opens)).await;
    sink.wait_items(MAX_OPEN_STREAMS).await;
    assert_eq!(entry.snapshot().open_streams, MAX_OPEN_STREAMS);
    let seq = MAX_OPEN_STREAMS as u64;
    send_lossy(&mut conn, &item(&open(&p, seq, seq + 1))).await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &p,
        LossCause::StreamLimit,
        GapReason::LimitExceeded,
        Some(seq - 1),
    );

    // Stream IDs only increase: no reuse after End, no going back.
    let rig = Rig::new();
    let sink = RecordingSink::new(64);
    let entry = rig.authorize("e", &t, sink.clone());
    for (n, bad) in [
        vec![open(&pid(2), 0, 1), end(&pid(2), 1, 1), open(&pid(2), 2, 1)],
        vec![open(&pid(3), 0, 2), open(&pid(3), 1, 1)],
    ]
    .into_iter()
    .enumerate()
    {
        let q = pid(2 + n as u32);
        let admitted = bad.len() as u64 - 2;
        let mut conn = rig.producer("e", &t, &q).await;
        send_lossy(&mut conn, &items(&bad)).await;
        expect_closed(&mut conn).await;
        assert_loss(
            &loss(&entry).await,
            &q,
            LossCause::StreamState,
            GapReason::ProducerLost,
            Some(admitted),
        );
    }
}

#[tokio::test]
async fn lost_producer_streams_count_until_recovery() {
    let rig = Rig::new();
    let sink = RecordingSink::new(1024);
    let t = token("cp-a1");
    let entry = rig.authorize("e", &t, sink.clone());

    // P opens 100 streams and is lost: they stay counted.
    let p = pid(1);
    let mut conn = rig.producer("e", &t, &p).await;
    send(
        &mut conn,
        &items(&(0..100).map(|n| open(&p, n, n + 1)).collect::<Vec<_>>()),
    )
    .await;
    sink.wait_items(100).await;
    conn.shutdown().await.unwrap();
    assert_eq!(loss(&entry).await.admitted_through, Some(99));
    assert_eq!(entry.snapshot().open_streams, 100);

    // Q starts without Recovery: only 28 slots remain for it.
    let q = pid(2);
    let mut conn = rig.producer("e", &t, &q).await;
    send_lossy(
        &mut conn,
        &items(&(0..29).map(|n| open(&q, n, n + 1)).collect::<Vec<_>>()),
    )
    .await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &q,
        LossCause::StreamLimit,
        GapReason::LimitExceeded,
        Some(27),
    );
    assert_eq!(entry.snapshot().open_streams, 128);

    // R's Recovery names P: P's 100 streams are released, Q's 28 stay counted.
    let r = pid(3);
    let mut conn = rig.producer("e", &t, &r).await;
    let mut recovered = vec![recovery(&r, 0, &p)];
    recovered.extend((1..=100).map(|n| open(&r, n, n)));
    send(&mut conn, &items(&recovered)).await;
    sink.wait_items(100 + 28 + 101).await;
    assert_eq!(entry.snapshot().open_streams, 128);
    send_lossy(&mut conn, &item(&open(&r, 101, 101))).await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &r,
        LossCause::StreamLimit,
        GapReason::LimitExceeded,
        Some(100),
    );
}

// ---------------------------------------------------------------------------------------------
// 5. Failed epochs, clean closure and bounded recovery.
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn queue_full_reports_loss_outside_full_queue() {
    let rig = Rig::new();
    let sink = RecordingSink::new(3);
    let t = token("queue-full");
    let entry = rig.authorize("e", &t, sink.clone());
    let p = pid(1);

    let mut conn = rig.producer("e", &t, &p).await;
    send_lossy(
        &mut conn,
        &items(&[
            connection_open(&p, 0),
            open(&p, 1, 1),
            bytes(&p, 2, 1, 0, b"fits"),
            bytes(&p, 3, 1, 4, b"does not fit"),
            bytes(&p, 4, 1, 16, b"never offered"),
        ]),
    )
    .await;
    expect_closed(&mut conn).await;

    // The loss is reported through the entry's latch while the sink's queue is still full.
    assert_loss(
        &loss(&entry).await,
        &p,
        LossCause::SinkRefused,
        GapReason::QueueFull,
        Some(2),
    );
    assert_eq!(
        sink.items().len(),
        3,
        "no Gap or loss item was pushed into the full queue"
    );
    assert_eq!(
        sink.submit_calls(),
        4,
        "nothing after the refused frame is offered"
    );

    // Once the owner drains, a fresh authorized epoch records Recovery before payload.
    sink.drain_payload();
    let q = pid(2);
    let mut conn = rig.producer("e", &t, &q).await;
    let sent = vec![recovery(&q, 0, &p), open(&q, 1, 1), end(&q, 2, 1)];
    send(&mut conn, &items(&sent)).await;
    send(&mut conn, &close(&q, Some(2))).await;
    conn.shutdown().await.unwrap();
    assert_eq!(clean_close(&entry).await.producer, q);
    assert_eq!(sink.frames(), sent);
}

#[tokio::test]
async fn disconnect_mid_frame_never_emits_partial_json() {
    let rig = Rig::new();
    let sink = RecordingSink::new(64);
    let t = token("mid-frame");
    let entry = rig.authorize("e", &t, sink.clone());

    // Partial body.
    let p = pid(1);
    let whole = item(&bytes(&p, 2, 1, 0, b"this frame is cut in half"));
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &items(&[connection_open(&p, 0), open(&p, 1, 1)])).await;
    send(&mut conn, &whole[..whole.len() / 2]).await;
    conn.shutdown().await.unwrap();
    assert_loss(
        &loss(&entry).await,
        &p,
        LossCause::PartialItem,
        GapReason::IngestLost,
        Some(1),
    );

    // Partial length prefix.
    let q = pid(2);
    let mut conn = rig.producer("e", &t, &q).await;
    send(&mut conn, &item(&connection_open(&q, 0))).await;
    send(&mut conn, &[0, 0]).await;
    conn.shutdown().await.unwrap();
    assert_loss(
        &loss(&entry).await,
        &q,
        LossCause::PartialItem,
        GapReason::IngestLost,
        Some(0),
    );

    assert_eq!(
        sink.frames(),
        vec![
            connection_open(&p, 0),
            open(&p, 1, 1),
            connection_open(&q, 0)
        ],
        "only whole, decoded frames crossed the boundary"
    );
}

#[tokio::test]
async fn old_epoch_never_recovers() {
    let rig = Rig::new();
    let sink = RecordingSink::new(64);
    let t = token("old-epoch");
    let entry = rig.authorize("e", &t, sink.clone());
    let other_sink = RecordingSink::new(8);
    let t2 = token("elsewhere");
    let other = rig.authorize("f", &t2, other_sink.clone());

    // P is lost.
    let p = pid(1);
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &item(&connection_open(&p, 0))).await;
    conn.shutdown().await.unwrap();
    loss(&entry).await;

    // P's ID never reconnects.
    let mut again = rig.producer("e", &t, &p).await;
    expect_closed(&mut again).await;

    // Q supersedes P with a Recovery at sequence zero.
    let q = pid(2);
    let mut conn_q = rig.producer("e", &t, &q).await;
    send(&mut conn_q, &item(&recovery(&q, 0, &p))).await;
    sink.wait_items(2).await;

    // A second Recovery naming P is refused: an epoch is superseded once.
    let r = pid(3);
    let mut conn = rig.producer("e", &t, &r).await;
    send_lossy(&mut conn, &item(&recovery(&r, 0, &p))).await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &r,
        LossCause::Recovery,
        GapReason::ProducerLost,
        None,
    );

    // Recovery only as the first frame.
    let s = pid(4);
    let mut conn = rig.producer("e", &t, &s).await;
    send_lossy(
        &mut conn,
        &items(&[connection_open(&s, 0), recovery(&s, 1, &q)]),
    )
    .await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &s,
        LossCause::Recovery,
        GapReason::ProducerLost,
        Some(0),
    );

    // A cleanly closed epoch cannot be recovered.
    send(&mut conn_q, &close(&q, Some(0))).await;
    conn_q.shutdown().await.unwrap();
    clean_close(&entry).await;
    let u = pid(5);
    let mut conn = rig.producer("e", &t, &u).await;
    send_lossy(&mut conn, &item(&recovery(&u, 0, &q))).await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &u,
        LossCause::Recovery,
        GapReason::ProducerLost,
        None,
    );

    // Nor can another entry's producer, or one this entry never admitted.
    let v = pid(6);
    let mut conn_v = rig.producer("f", &t2, &v).await;
    send(&mut conn_v, &item(&connection_open(&v, 0))).await;
    other_sink.wait_items(1).await;
    for (n, stranger) in [v.clone(), pid(99)].into_iter().enumerate() {
        let w = pid(7 + n as u32);
        let mut conn = rig.producer("e", &t, &w).await;
        send_lossy(&mut conn, &item(&recovery(&w, 0, &stranger))).await;
        expect_closed(&mut conn).await;
        assert_loss(
            &loss(&entry).await,
            &w,
            LossCause::Recovery,
            GapReason::ProducerLost,
            None,
        );
    }
    assert!(other.take_outcomes().is_empty());
    assert_eq!(
        sink.frames(),
        vec![
            connection_open(&p, 0),
            recovery(&q, 0, &p),
            connection_open(&s, 0)
        ],
        "no replay and no second recovery reached the sink"
    );
}

#[tokio::test]
async fn fresh_authorized_epoch_retains_gap() {
    let rig = Rig::new();
    let sink = RecordingSink::new(64);
    let t = token("retain-gap");
    let entry = rig.authorize("e", &t, sink.clone());

    let p = pid(1);
    let old = vec![
        connection_open(&p, 0),
        open(&p, 1, 1),
        bytes(&p, 2, 1, 0, b"interrupted bo"),
    ];
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &items(&old)).await;
    conn.shutdown().await.unwrap();
    let old_loss = loss(&entry).await;
    assert_loss(
        &old_loss,
        &p,
        LossCause::Disconnected,
        GapReason::IngestLost,
        Some(2),
    );
    assert_eq!(entry.snapshot().open_streams, 1);

    let q = pid(2);
    let reobserve = frame(
        &q,
        1,
        0,
        FrameEvent::Connection {
            connection: CONNECTION.to_string(),
            event: ConnectionEvent::Open {
                alpn: Some("http/1.1".to_string()),
                already_open: true,
            },
        },
    );
    let fresh = vec![
        recovery(&q, 0, &p),
        reobserve,
        open(&q, 2, 1),
        bytes(&q, 3, 1, 0, b"next message"),
        end(&q, 4, 1),
    ];
    let mut conn = rig.producer("e", &t, &q).await;
    send(&mut conn, &items(&fresh)).await;
    send(&mut conn, &close(&q, Some(4))).await;
    conn.shutdown().await.unwrap();
    assert_eq!(clean_close(&entry).await.final_seq, Some(4));

    let mut expected: Vec<Item> = old.into_iter().map(Item::Frame).collect();
    expected.extend(fresh.into_iter().map(Item::Frame));
    expected.push(Item::Close {
        producer: q.clone(),
        final_seq: Some(4),
    });
    assert_eq!(
        sink.items(),
        expected,
        "old payload appears once; nothing replayed"
    );
    assert_eq!(
        entry.snapshot().open_streams,
        0,
        "the recovery released P's stream"
    );
    assert!(
        entry.take_outcomes().is_empty(),
        "P's loss was latched once and stays history"
    );
}

#[tokio::test]
async fn recovery_supersedes_a_still_connected_epoch() {
    let rig = Rig::new();
    let sink = RecordingSink::new(64);
    let t = token("supersede");
    let entry = rig.authorize("e", &t, sink.clone());

    // P is still connected (the receiver has seen no failure) with one open stream.
    let p = pid(1);
    let old = vec![
        connection_open(&p, 0),
        open(&p, 1, 1),
        bytes(&p, 2, 1, 0, b"abc"),
    ];
    let mut conn_p = rig.producer("e", &t, &p).await;
    send(&mut conn_p, &items(&old)).await;
    sink.wait_items(3).await;
    assert_eq!(entry.snapshot().open_streams, 1);

    // Q's Recovery names P: P is latched lost as superseded, its socket is closed and its
    // stream released. The Recovery was queued behind every frame P had admitted.
    let q = pid(2);
    let mut conn_q = rig.producer("e", &t, &q).await;
    send(&mut conn_q, &item(&recovery(&q, 0, &p))).await;
    assert_loss(
        &loss(&entry).await,
        &p,
        LossCause::Superseded,
        GapReason::ProducerLost,
        Some(2),
    );
    expect_closed(&mut conn_p).await;
    assert_eq!(entry.snapshot().open_streams, 0);
    let mut expected = old.clone();
    expected.push(recovery(&q, 0, &p));
    assert_eq!(sink.frames(), expected);

    // Q carries on and closes cleanly.
    send(&mut conn_q, &items(&[open(&q, 1, 1), end(&q, 2, 1)])).await;
    send(&mut conn_q, &close(&q, Some(2))).await;
    conn_q.shutdown().await.unwrap();
    assert_eq!(clean_close(&entry).await.producer, q);
}

/// A sink that panics on one frame: stands in for any fault that kills a connection task.
struct PanickingSink(Arc<RecordingSink>);

impl CaptureSink for PanickingSink {
    fn try_submit(&self, frame: CaptureFrame) -> SubmitResult {
        if frame.seq == 1 {
            panic!("test sink fault");
        }
        self.0.try_submit(frame)
    }

    fn try_begin_epoch(&self, producer: &str) -> EpochAdmission {
        self.0.try_begin_epoch(producer)
    }

    fn try_close_epoch(&self, producer: &str, final_seq: Option<u64>) -> SubmitResult {
        self.0.try_close_epoch(producer, final_seq)
    }
}

#[tokio::test]
async fn a_task_that_dies_still_settles_its_producer() {
    let rig = Rig::new();
    let sink = RecordingSink::new(64);
    let t = token("dies");
    let entry = rig
        .listener()
        .authorize(EntryGrant {
            entry_id: "e".to_string(),
            secret: IngestSecret::new(t.clone()).unwrap(),
            sink: Arc::new(PanickingSink(sink.clone())),
            prior_producers: 0,
        })
        .unwrap();

    let p = pid(1);
    let mut conn = rig.producer("e", &t, &p).await;
    send_lossy(&mut conn, &items(&[connection_open(&p, 0), open(&p, 1, 1)])).await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &p,
        LossCause::Aborted,
        GapReason::IngestLost,
        Some(0),
    );
    assert_eq!(entry.snapshot().producers_unsettled, 0);

    // The entry keeps serving other producers.
    let q = pid(2);
    let mut conn = rig.producer("e", &t, &q).await;
    send(&mut conn, &item(&connection_open(&q, 0))).await;
    send(&mut conn, &close(&q, Some(0))).await;
    conn.shutdown().await.unwrap();
    assert_eq!(clean_close(&entry).await.producer, q);
}

#[tokio::test]
async fn non_loopback_bind_is_refused() {
    for address in ["0.0.0.0:0", "[::]:0", "192.0.2.1:0"] {
        let refused = IngestListener::bind_loopback(address.parse().unwrap(), test_config());
        assert!(
            matches!(
                refused,
                Err(goalcup_daemon::ingest::IngestError::NotLoopback)
            ),
            "{address}"
        );
    }
    let rig = Rig::new();
    let addr = rig.listener().local_addr();
    assert!(
        addr.ip().is_loopback() && addr.port() != 0,
        "port zero picked a real port"
    );
}

/// Sequence A: P, then Q lost in flight (its Recovery never admitted), then R.
#[tokio::test]
async fn recovery_naming_a_zero_frame_producer_releases_orphaned_wire_streams() {
    let rig = Rig::with(|c| c.setup_timeout = Duration::from_millis(400));
    let sink = RecordingSink::new(64);
    let t = token("cp-a4-a");
    let entry = rig.authorize("e", &t, sink.clone());

    // P, the first wire producer, is lost with one open stream.
    let p = pid(1);
    let old = vec![
        connection_open(&p, 0),
        open(&p, 1, 1),
        bytes(&p, 2, 1, 0, b"abc"),
    ];
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &items(&old)).await;
    sink.wait_items(3).await;
    conn.shutdown().await.unwrap();
    assert_eq!(loss(&entry).await.admitted_through, Some(2));
    assert_eq!(entry.snapshot().open_streams, 1);

    // Q's handshake is consumed, but the owner answers Pending past the setup deadline.
    // Its sender had already queued Recovery{P}; none of it is ever admitted.
    sink.set_script(vec![EpochAdmission::Pending; 1000]);
    let q = pid(2);
    let mut conn_q = rig.producer("e", &t, &q).await;
    send_lossy(
        &mut conn_q,
        &items(&[recovery(&q, 0, &p), reobserve(&q, 1)]),
    )
    .await;
    expect_closed(&mut conn_q).await;
    assert_eq!(entry.snapshot().admissions_refused, 1);
    assert!(
        entry.take_outcomes().is_empty(),
        "Q was never admitted: no latch"
    );

    // R names Q. Accepted, and P, the orphaned wire producer, is superseded with it.
    sink.set_script(Vec::new());
    let r = pid(3);
    let mut conn_r = rig.producer("e", &t, &r).await;
    send(
        &mut conn_r,
        &items(&[recovery(&r, 0, &q), reobserve(&r, 1)]),
    )
    .await;
    sink.wait_items(5).await;
    assert_eq!(entry.snapshot().open_streams, 0, "P's stream is released");
    assert!(
        entry.take_outcomes().is_empty(),
        "P's loss was reported once, at its end"
    );

    // P is superseded now, so nothing can name it again; R carries on and closes cleanly.
    let s = pid(4);
    let mut conn_s = rig.producer("e", &t, &s).await;
    send_lossy(&mut conn_s, &item(&recovery(&s, 0, &p))).await;
    expect_closed(&mut conn_s).await;
    assert_loss(
        &loss(&entry).await,
        &s,
        LossCause::Recovery,
        GapReason::ProducerLost,
        None,
    );
    send(&mut conn_r, &items(&[open(&r, 2, 1), end(&r, 3, 1)])).await;
    send(&mut conn_r, &close(&r, Some(3))).await;
    conn_r.shutdown().await.unwrap();
    assert_eq!(clean_close(&entry).await.producer, r);

    let mut expected = old;
    expected.extend([
        recovery(&r, 0, &q),
        reobserve(&r, 1),
        open(&r, 2, 1),
        end(&r, 3, 1),
    ]);
    assert_eq!(sink.frames(), expected, "nothing of Q, nothing replayed");
}

fn loss_frame(producer: &str, seq: u64, reason: GapReason) -> CaptureFrame {
    frame(producer, seq, 0, FrameEvent::Loss { reason })
}

/// H-16: a stream-zero `Loss` is an admitted control that consumes a sequence number, enters no
/// payload stream state, and marks its producer as wire, so a loss-only epoch is recoverable
/// like one that opened a connection.
#[tokio::test]
async fn a_stream_zero_loss_is_an_admitted_control_and_marks_the_epoch_wire() {
    let rig = Rig::new();
    let sink = RecordingSink::new(64);
    let t = token("h16-loss-control");
    let entry = rig.authorize("e", &t, sink.clone());

    // A loss-only epoch: one control frame, then it is lost with no close.
    let p = pid(1);
    let mut conn = rig.producer("e", &t, &p).await;
    send(
        &mut conn,
        &items(&[loss_frame(&p, 0, GapReason::DecodeFailed)]),
    )
    .await;
    sink.wait_items(1).await;
    assert_eq!(entry.snapshot().open_streams, 0, "a loss opens no stream");
    conn.shutdown().await.unwrap();
    assert_loss(
        &loss(&entry).await,
        &p,
        LossCause::Disconnected,
        GapReason::IngestLost,
        Some(0),
    );

    // Q is admitted and lost before any frame: zero admitted frames.
    let q = pid(2);
    let mut conn = rig.producer("e", &t, &q).await;
    conn.shutdown().await.unwrap();
    assert_loss(
        &loss(&entry).await,
        &q,
        LossCause::Disconnected,
        GapReason::IngestLost,
        None,
    );

    // R names Q, and the orphan rule sweeps P with it: P was a wire epoch only because its one
    // frame was a Loss (a non-wire epoch is untouched, see
    // `orphan_rule_never_touches_non_wire_producers`).
    let r = pid(3);
    let mut conn_r = rig.producer("e", &t, &r).await;
    send(&mut conn_r, &items(&[recovery(&r, 0, &q)])).await;
    sink.wait_items(2).await;
    assert!(entry.take_outcomes().is_empty());

    // So nothing can name P any more.
    let s2 = pid(4);
    let mut conn_s = rig.producer("e", &t, &s2).await;
    send_lossy(&mut conn_s, &item(&recovery(&s2, 0, &p))).await;
    expect_closed(&mut conn_s).await;
    assert_loss(
        &loss(&entry).await,
        &s2,
        LossCause::Recovery,
        GapReason::ProducerLost,
        None,
    );

    // R closes cleanly with the sequence its frames reached, before the next recovery: a
    // Recovery naming a loss-only epoch is an orphan recovery and would supersede it.
    send(&mut conn_r, &close(&r, Some(0))).await;
    conn_r.shutdown().await.unwrap();
    assert_eq!(clean_close(&entry).await.final_seq, Some(0));

    // A Recovery naming the loss-only epoch directly is admitted too.
    let u = pid(5);
    let mut conn_u = rig.producer("e", &t, &u).await;
    let w = pid(6);
    let mut conn_w = rig.producer("e", &t, &w).await;
    send(
        &mut conn_u,
        &items(&[loss_frame(&u, 0, GapReason::LimitExceeded)]),
    )
    .await;
    sink.wait_items(4).await; // R's clean close is an item too
    conn_u.shutdown().await.unwrap();
    assert_eq!(loss(&entry).await.producer, u);
    send(&mut conn_w, &items(&[recovery(&w, 0, &u)])).await;
    sink.wait_items(5).await;
    assert_eq!(
        sink.frames(),
        vec![
            loss_frame(&p, 0, GapReason::DecodeFailed),
            recovery(&r, 0, &q),
            loss_frame(&u, 0, GapReason::LimitExceeded),
            recovery(&w, 0, &u),
        ]
    );
}

/// H-16 / CP-A4: a loss-only epoch records nothing under its own name (a `Loss` chains only as a
/// stream-less gap), so the integrity reader reads a Recovery naming it as unrecorded, which
/// retires every other wire producer. The receiver adopts that definition: another live wire
/// producer is superseded with it, and its later frames are refused.
#[tokio::test]
async fn recovery_naming_a_loss_only_epoch_supersedes_every_other_wire_producer() {
    let rig = Rig::new();
    let sink = RecordingSink::new(64);
    let t = token("h16-loss-only-orphan");
    let entry = rig.authorize("e", &t, sink.clone());

    // P is loss-only and lost; L is a second, live wire producer with one open stream.
    let p = pid(1);
    let mut conn_p = rig.producer("e", &t, &p).await;
    send(
        &mut conn_p,
        &items(&[loss_frame(&p, 0, GapReason::LimitExceeded)]),
    )
    .await;
    sink.wait_items(1).await;
    conn_p.shutdown().await.unwrap();
    assert_eq!(loss(&entry).await.producer, p);
    let l = pid(2);
    let live = vec![connection_open(&l, 0), open(&l, 1, 1)];
    let mut conn_l = rig.producer("e", &t, &l).await;
    send(&mut conn_l, &items(&live)).await;
    sink.wait_items(3).await;
    assert_eq!(entry.snapshot().open_streams, 1);

    // R names P: an orphan recovery. L is superseded with P and its stream is released.
    let r = pid(3);
    let mut conn_r = rig.producer("e", &t, &r).await;
    send(&mut conn_r, &item(&recovery(&r, 0, &p))).await;
    assert_loss(
        &loss(&entry).await,
        &l,
        LossCause::Superseded,
        GapReason::ProducerLost,
        Some(1),
    );
    expect_closed(&mut conn_l).await;
    assert_eq!(entry.snapshot().open_streams, 0);
    let mut expected = vec![loss_frame(&p, 0, GapReason::LimitExceeded)];
    expected.extend(live);
    expected.push(recovery(&r, 0, &p));
    assert_eq!(sink.frames(), expected, "nothing of L follows the Recovery");

    // R carries on and closes cleanly.
    send(&mut conn_r, &close(&r, Some(0))).await;
    conn_r.shutdown().await.unwrap();
    assert_eq!(clean_close(&entry).await.producer, r);
}

/// H-16: a loss that breaks the representation rules loses its producer; a refused
/// loss means the close that would attest the stream is refused too.
#[tokio::test]
async fn a_loss_on_a_payload_stream_loses_the_producer() {
    let rig = Rig::new();
    let sink = RecordingSink::new(64);
    let t = token("h16-loss-misplaced");
    let entry = rig.authorize("e", &t, sink.clone());

    // On a payload stream, or any payload event on stream zero.
    let on_stream = frame(
        &pid(1),
        0,
        1,
        FrameEvent::Loss {
            reason: GapReason::DecodeFailed,
        },
    );
    let bytes_on_zero = frame(
        &pid(2),
        0,
        0,
        FrameEvent::Bytes {
            offset: 0,
            data: b"x".to_vec(),
        },
    );
    for (n, bad) in [on_stream, bytes_on_zero].into_iter().enumerate() {
        let p = pid(n as u32 + 1);
        let mut conn = rig.producer("e", &t, &p).await;
        send_lossy(&mut conn, &item(&bad)).await;
        expect_closed(&mut conn).await;
        assert_loss(
            &loss(&entry).await,
            &p,
            LossCause::Invalid,
            GapReason::ProducerLost,
            None,
        );
    }
    assert!(sink.items().is_empty(), "neither frame reached the sink");
}

/// H-16: the sink refuses a `Loss` (its queue is full). The producer is lost with
/// that reason, nothing after the refused frame is offered, and the close that follows is never
/// queued, so a producer whose loss could not be carried never closes clean.
#[tokio::test]
async fn a_refused_loss_makes_the_close_unanswerable() {
    let rig = Rig::new();
    let sink = RecordingSink::new(1);
    let t = token("h16-loss-refused");
    let entry = rig.authorize("e", &t, sink.clone());
    let p = pid(1);

    let mut conn = rig.producer("e", &t, &p).await;
    send_lossy(
        &mut conn,
        &items(&[
            connection_open(&p, 0),
            loss_frame(&p, 1, GapReason::DecodeFailed),
        ]),
    )
    .await;
    send_lossy(&mut conn, &close(&p, Some(1))).await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &p,
        LossCause::SinkRefused,
        GapReason::QueueFull,
        Some(0),
    );
    assert!(
        !sink.items().iter().any(|i| matches!(i, Item::Close { .. })),
        "no close was queued for a producer whose loss was refused"
    );
    assert_eq!(sink.frames(), vec![connection_open(&p, 0)]);
}

/// Sequence B: a Recovery naming a recorded producer supersedes that producer only.
#[tokio::test]
async fn recovery_naming_a_recorded_producer_supersedes_only_it() {
    let rig = Rig::new();
    let sink = RecordingSink::new(64);
    let t = token("cp-a4-b");
    let entry = rig.authorize("e", &t, sink.clone());

    // P is recorded and lost with one open stream.
    let p = pid(1);
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &items(&[connection_open(&p, 0), open(&p, 1, 1)])).await;
    sink.wait_items(2).await;
    conn.shutdown().await.unwrap();
    loss(&entry).await;

    // X is another wire producer, still live with an open stream.
    let x = pid(2);
    let mut conn_x = rig.producer("e", &t, &x).await;
    send(
        &mut conn_x,
        &items(&[connection_open(&x, 0), open(&x, 1, 1)]),
    )
    .await;
    sink.wait_items(4).await;
    assert_eq!(entry.snapshot().open_streams, 2);

    // Q's Recovery names P, which has admitted frames: only P is released.
    let q = pid(3);
    let mut conn_q = rig.producer("e", &t, &q).await;
    send(
        &mut conn_q,
        &items(&[recovery(&q, 0, &p), reobserve(&q, 1)]),
    )
    .await;
    sink.wait_items(6).await;
    assert_eq!(entry.snapshot().open_streams, 1, "X keeps its stream");
    assert!(entry.take_outcomes().is_empty(), "X is not superseded");

    // X finishes normally.
    send(&mut conn_x, &items(&[end(&x, 2, 1)])).await;
    send(&mut conn_x, &close(&x, Some(2))).await;
    conn_x.shutdown().await.unwrap();
    assert_eq!(clean_close(&entry).await.producer, x);
}

/// Sequence C: native (and goal) producers are never wire and never superseded.
#[tokio::test]
async fn orphan_rule_never_touches_non_wire_producers() {
    let rig = Rig::new();
    let sink = RecordingSink::new(64);
    let t = token("cp-a4-c");
    let entry = rig.authorize("e", &t, sink.clone());

    // N captures a native stream: no Connection frame, so not wire.
    let n = pid(1);
    let mut conn_n = rig.producer("e", &t, &n).await;
    send(
        &mut conn_n,
        &items(&[open(&n, 0, 1), bytes(&n, 1, 1, 0, b"{}")]),
    )
    .await;
    sink.wait_items(2).await;

    // P, a wire producer, is lost with an open stream.
    let p = pid(2);
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &items(&[connection_open(&p, 0), open(&p, 1, 1)])).await;
    sink.wait_items(4).await;
    conn.shutdown().await.unwrap();
    assert_eq!(loss(&entry).await.producer, p);

    // Q is admitted and lost before any frame: zero admitted frames.
    let q = pid(3);
    let mut conn = rig.producer("e", &t, &q).await;
    conn.shutdown().await.unwrap();
    assert_loss(
        &loss(&entry).await,
        &q,
        LossCause::Disconnected,
        GapReason::IngestLost,
        None,
    );
    assert_eq!(entry.snapshot().open_streams, 2);

    // R names Q: P is swept with it, N is untouched.
    let r = pid(4);
    let mut conn_r = rig.producer("e", &t, &r).await;
    send(
        &mut conn_r,
        &items(&[recovery(&r, 0, &q), reobserve(&r, 1)]),
    )
    .await;
    sink.wait_items(6).await;
    assert_eq!(entry.snapshot().open_streams, 1, "only N's stream remains");
    assert!(
        entry.take_outcomes().is_empty(),
        "N is neither lost nor superseded"
    );

    // N keeps capturing and closes cleanly; so does R.
    send(
        &mut conn_n,
        &items(&[bytes(&n, 2, 1, 2, b"[]"), end(&n, 3, 1)]),
    )
    .await;
    send(&mut conn_n, &close(&n, Some(3))).await;
    conn_n.shutdown().await.unwrap();
    assert_eq!(clean_close(&entry).await.producer, n);
    send(&mut conn_r, &close(&r, Some(1))).await;
    conn_r.shutdown().await.unwrap();
    assert_eq!(clean_close(&entry).await.producer, r);
}

#[tokio::test]
async fn fresh_epoch_after_startup_retry_needs_no_recovery() {
    let rig = Rig::new();
    let sink = RecordingSink::scripted(64, vec![EpochAdmission::RetryFresh]);
    let t = token("r1");
    let entry = rig.authorize("e", &t, sink.clone());

    // The startup: the first ID's handshake is consumed and refused.
    let first = pid(1);
    let mut conn = rig.producer("e", &t, &first).await;
    expect_closed(&mut conn).await;

    // The next ID legitimately starts with Connection at sequence zero.
    let second = pid(2);
    let mut conn = rig.producer("e", &t, &second).await;
    send(&mut conn, &item(&connection_open(&second, 0))).await;
    send(&mut conn, &close(&second, Some(0))).await;
    conn.shutdown().await.unwrap();
    assert_eq!(clean_close(&entry).await.producer, second);

    // A consumed-but-refused epoch may still be named by a Recovery: it was never closed.
    let third = pid(3);
    let mut conn = rig.producer("e", &t, &third).await;
    send(&mut conn, &item(&recovery(&third, 0, &first))).await;
    sink.wait_items(3).await;
    assert_eq!(entry.snapshot().producers_consumed, 3);
}

#[tokio::test]
async fn close_final_seq_matches_admitted_frames() {
    let rig = Rig::with(|c| {
        c.close_grace = Duration::from_millis(300);
    });
    let sink = RecordingSink::new(256);
    let t = token("final-seq");
    let entry = rig.authorize("e", &t, sink.clone());
    let three = |p: &str| items(&[connection_open(p, 0), open(p, 1, 1), end(p, 2, 1)]);

    // Every wrong final_seq is unknown loss, and no Close reaches the sink.
    for (n, wrong) in [Some(1), Some(3), None].into_iter().enumerate() {
        let p = pid(1 + n as u32);
        let mut conn = rig.producer("e", &t, &p).await;
        send_lossy(&mut conn, &three(&p)).await;
        send_lossy(&mut conn, &close(&p, wrong)).await;
        expect_closed(&mut conn).await;
        assert_loss(
            &loss(&entry).await,
            &p,
            LossCause::CloseMismatch,
            GapReason::ProducerLost,
            Some(2),
        );
    }
    let p = pid(4);
    let mut conn = rig.producer("e", &t, &p).await;
    send_lossy(&mut conn, &close(&p, Some(0))).await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &p,
        LossCause::CloseMismatch,
        GapReason::ProducerLost,
        None,
    );

    // The right one, then end of stream, is clean.
    let p = pid(5);
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &three(&p)).await;
    send(&mut conn, &close(&p, Some(2))).await;
    conn.shutdown().await.unwrap();
    assert_eq!(clean_close(&entry).await.final_seq, Some(2));

    // A frame after a valid Close undoes nothing already admitted, but the tail is not clean.
    let p = pid(6);
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &three(&p)).await;
    send(&mut conn, &close(&p, Some(2))).await;
    send_lossy(&mut conn, &item(&open(&p, 3, 2))).await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &p,
        LossCause::AfterClose,
        GapReason::ProducerLost,
        Some(2),
    );

    // A valid Close that is never followed by end of stream is not clean either.
    let p = pid(7);
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &three(&p)).await;
    send(&mut conn, &close(&p, Some(2))).await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &p,
        LossCause::CloseWithoutEof,
        GapReason::IngestLost,
        Some(2),
    );

    let closes: Vec<Item> = sink
        .items()
        .into_iter()
        .filter(|i| matches!(i, Item::Close { .. }))
        .collect();
    assert_eq!(
        closes,
        vec![Item::Close {
            producer: pid(5),
            final_seq: Some(2)
        }]
    );
}

#[tokio::test]
async fn zero_frame_close_is_clean() {
    let rig = Rig::new();
    let sink = RecordingSink::new(8);
    let t = token("zero");
    let entry = rig.authorize("e", &t, sink.clone());
    let p = pid(1);
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &close(&p, None)).await;
    conn.shutdown().await.unwrap();
    let closed = clean_close(&entry).await;
    assert_eq!(closed.producer, p);
    assert_eq!(closed.final_seq, None);
    assert_eq!(
        sink.items(),
        vec![Item::Close {
            producer: p,
            final_seq: None
        }]
    );
    expect_closed(&mut conn).await;
}

#[tokio::test]
async fn eof_without_close_is_unknown_loss() {
    let rig = Rig::new();
    let sink = RecordingSink::new(8);
    let t = token("eof");
    let entry = rig.authorize("e", &t, sink.clone());

    // The same frames as a clean run, then end of stream without a Close.
    let p = pid(1);
    let mut conn = rig.producer("e", &t, &p).await;
    send(
        &mut conn,
        &items(&[connection_open(&p, 0), open(&p, 1, 1), end(&p, 2, 1)]),
    )
    .await;
    conn.shutdown().await.unwrap();
    assert_loss(
        &loss(&entry).await,
        &p,
        LossCause::Disconnected,
        GapReason::IngestLost,
        Some(2),
    );

    // A crashed sender (socket dropped) is the same unknown tail.
    let q = pid(2);
    let mut conn = rig.producer("e", &t, &q).await;
    send(&mut conn, &item(&connection_open(&q, 0))).await;
    sink.wait_items(4).await;
    drop(conn);
    assert_loss(
        &loss(&entry).await,
        &q,
        LossCause::Disconnected,
        GapReason::IngestLost,
        Some(0),
    );
    assert!(
        sink.items().iter().all(|i| matches!(i, Item::Frame(_))),
        "end of stream never manufactures a close"
    );
}

#[tokio::test]
async fn close_with_open_stream_is_incomplete() {
    let rig = Rig::new();
    let sink = RecordingSink::new(8);
    let t = token("open-at-close");
    let entry = rig.authorize("e", &t, sink.clone());
    let p = pid(1);
    let mut conn = rig.producer("e", &t, &p).await;
    send(
        &mut conn,
        &items(&[open(&p, 0, 1), bytes(&p, 1, 1, 0, b"no end")]),
    )
    .await;
    send_lossy(&mut conn, &close(&p, Some(1))).await;
    expect_closed(&mut conn).await;
    assert_loss(
        &loss(&entry).await,
        &p,
        LossCause::CloseWithOpenStreams,
        GapReason::ProducerLost,
        Some(1),
    );
    assert!(sink.items().iter().all(|i| matches!(i, Item::Frame(_))));
    assert_eq!(entry.snapshot().open_streams, 1);
}

#[tokio::test]
async fn epoch_limit_refuses_recovery() {
    let rig = Rig::new();
    let sink = RecordingSink::new(1024);
    let t = token("epochs");
    let entry = rig.authorize("e", &t, sink.clone());

    // Every authenticated handshake consumes one of MAX_PRODUCERS, frames or not.
    for n in 0..MAX_PRODUCERS as u32 {
        let p = pid(n + 1);
        let mut conn = rig.producer("e", &t, &p).await;
        conn.shutdown().await.unwrap();
        let _ = loss(&entry).await;
    }
    assert_eq!(entry.snapshot().producers_consumed, MAX_PRODUCERS);

    // A fresh epoch naming the last one is refused at the handshake.
    let last = pid(MAX_PRODUCERS as u32);
    let fresh = pid(1000);
    let mut conn = rig.producer("e", &t, &fresh).await;
    send_lossy(&mut conn, &item(&recovery(&fresh, 0, &last))).await;
    expect_closed(&mut conn).await;
    assert_eq!(sink.begin_calls().len(), MAX_PRODUCERS);
    assert!(sink.items().is_empty());

    // Epochs consumed before this authorization count too.
    let rig = Rig::new();
    let sink = RecordingSink::new(8);
    let entry = rig.authorize_prior("e", &t, sink.clone(), MAX_PRODUCERS - 1);
    let mut conn = rig.producer("e", &t, &pid(1)).await;
    send(&mut conn, &item(&connection_open(&pid(1), 0))).await;
    sink.wait_items(1).await;
    let mut conn = rig.producer("e", &t, &pid(2)).await;
    expect_closed(&mut conn).await;
    assert_eq!(entry.snapshot().producers_consumed, MAX_PRODUCERS);
    let over = rig.listener().authorize(EntryGrant {
        entry_id: "g".to_string(),
        secret: IngestSecret::new(t.clone()).unwrap(),
        sink: RecordingSink::new(8),
        prior_producers: MAX_PRODUCERS + 1,
    });
    assert!(over.is_err());
}

#[tokio::test]
async fn validated_close_outcome_follows_admitted_frames() {
    let rig = Rig::new();
    let sink = RecordingSink::new(256);
    let t = token("ordered-close");
    let entry = rig.authorize("e", &t, sink.clone());
    let p = pid(1);

    let mut sent = vec![connection_open(&p, 0), open(&p, 1, 1)];
    sent.extend((0..40).map(|n| bytes(&p, 2 + n, 1, n * 8, b"12345678")));
    sent.push(end(&p, 42, 1));
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &items(&sent)).await;
    send(&mut conn, &close(&p, Some(42))).await;
    conn.shutdown().await.unwrap();

    let closed = clean_close(&entry).await;
    // By the time the outcome exists, every admitted frame precedes the sink's close.
    let recorded = sink.items();
    assert_eq!(recorded.len(), sent.len() + 1);
    assert_eq!(
        recorded.last(),
        Some(&Item::Close {
            producer: p.clone(),
            final_seq: Some(42)
        })
    );
    assert_eq!(sink.frames(), sent);
    assert_eq!(closed.final_seq, Some(42));
    // The receiver stamps its own observation on the daemon clock.
    assert_eq!(closed.observed.clock_id, CLOCK_ID);
    closed.observed.validate().unwrap();
    assert_eq!(entry.snapshot().frames_admitted, sent.len() as u64);
}

#[tokio::test]
async fn client_cannot_submit_producer_closed() {
    let rig = Rig::new();
    let sink = RecordingSink::new(8);
    let t = token("no-forgery");
    let entry = rig.authorize("e", &t, sink.clone());

    let forged_record = |p: &str| {
        json!({
            "kind": "producer_closed",
            "producer": p,
            "observed": at(p, 0),
            "final_seq": null
        })
    };
    let forged_event = |p: &str| {
        json!({
            "kind": "frame",
            "stream": {"producer": p, "stream": 0},
            "seq": 0,
            "observed": at(p, 0),
            "event": {"kind": "producer_closed", "producer": p, "final_seq": null}
        })
    };
    for (n, forge) in [forged_record, forged_event].into_iter().enumerate() {
        let p = pid(n as u32 + 1);
        let mut conn = rig.producer("e", &t, &p).await;
        send_lossy(&mut conn, &framed(&serde_json::to_vec(&forge(&p)).unwrap())).await;
        conn.shutdown().await.ok();
        expect_closed(&mut conn).await;
        assert_loss(
            &loss(&entry).await,
            &p,
            LossCause::Malformed,
            GapReason::ProducerLost,
            None,
        );
    }
    assert!(sink.items().is_empty(), "no close and no frame crossed");
}

// ---------------------------------------------------------------------------------------------
// 6. Bounded shutdown, drain and revocation.
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn shutdown_with_stalled_peer_is_bounded() {
    let rig = Rig::with(|c| {
        c.setup_timeout = Duration::from_secs(60);
    });
    let sink = RecordingSink::new(8);
    let t = token("stalled");
    let entry = rig.authorize("e", &t, sink.clone());

    // A silent unauthenticated socket, a producer stalled mid-frame and an idle producer.
    let mut silent = rig.connect().await;
    let (stalled, idle) = (pid(1), pid(2));
    let mut conn_stalled = rig.producer("e", &t, &stalled).await;
    send(&mut conn_stalled, &item(&connection_open(&stalled, 0))).await;
    let half = item(&open(&stalled, 1, 1));
    send(&mut conn_stalled, &half[..10]).await;
    let mut conn_idle = rig.producer("e", &t, &idle).await;
    send(&mut conn_idle, &item(&connection_open(&idle, 0))).await;
    sink.wait_items(2).await;

    let started = Instant::now();
    let report = rig.shutdown(Duration::from_millis(500)).await;
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "shutdown is bounded by its deadline, not by the peers"
    );
    assert_eq!(report.producers_cancelled, 2);
    for conn in [&mut silent, &mut conn_stalled, &mut conn_idle] {
        expect_closed(conn).await;
    }
    let mut lost = outcomes(&entry, 2).await;
    lost.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    for outcome in lost {
        let ProducerOutcome::Lost(loss) = outcome else {
            panic!("shutdown never manufactures a clean close");
        };
        assert_eq!(loss.cause, LossCause::Cancelled);
        assert_eq!(loss.reason, GapReason::ShutdownDeadline);
        assert_eq!(loss.admitted_through, Some(0));
    }
    assert_eq!(sink.frames().len(), 2, "the half frame never crossed");
}

#[tokio::test]
async fn shutdown_loss_is_not_double_counted() {
    let rig = Rig::new();
    let sink = RecordingSink::new(8);
    let t = token("once");
    let entry = rig.authorize("e", &t, sink.clone());

    // P is lost before shutdown and its outcome is taken.
    let p = pid(1);
    let mut conn = rig.producer("e", &t, &p).await;
    conn.shutdown().await.unwrap();
    assert_eq!(loss(&entry).await.producer, p);

    // Q is live at the entry drain deadline; the listener then shuts down as well.
    let q = pid(2);
    let mut conn_q = rig.producer("e", &t, &q).await;
    send(&mut conn_q, &item(&connection_open(&q, 0))).await;
    sink.wait_items(1).await;
    let drained: DrainReport = entry
        .drain(Instant::now() + Duration::from_millis(100))
        .await;
    assert_eq!(drained.producers_cancelled, 1);
    let report = rig.shutdown(Duration::from_millis(500)).await;
    assert_eq!(
        report.producers_cancelled, 0,
        "already settled by the drain"
    );

    let only = outcomes(&entry, 1).await;
    assert!(
        matches!(&only[0], ProducerOutcome::Lost(l) if l.producer == q && l.cause == LossCause::Cancelled)
    );
    expect_closed(&mut conn_q).await;
    assert!(
        entry.take_outcomes().is_empty(),
        "each latch is delivered once"
    );
}

/// The integration shape: bind, authorize, consume latches, drain, shut down.
#[tokio::test]
async fn t15_integration_example() {
    let listener = IngestListener::bind_loopback(
        "127.0.0.1:0".parse().unwrap(),
        IngestConfig::new(monotonic_clock("daemon-clock")),
    )
    .unwrap();
    // The proxy receives this address, the entry ID and the secret through its descriptor.
    let addr = listener.local_addr();
    // The entry owner's sink; the secret goes to the proxy through a private descriptor.
    let sink = RecordingSink::new(64);
    let secret = IngestSecret::generate();
    let entry = listener
        .authorize(EntryGrant {
            entry_id: "e0123456789abcdef".to_string(),
            secret: secret.clone(),
            sink: sink.clone(),
            prior_producers: 0,
        })
        .unwrap();

    // One producer closes cleanly, one stays open past the drain deadline.
    let (clean, idle) = (pid(1), pid(2));
    let mut a: Conn = TcpStream::connect(addr).await.unwrap();
    send(
        &mut a,
        &handshake("e0123456789abcdef", &clean, secret.expose()),
    )
    .await;
    send(&mut a, &items(&[open(&clean, 0, 1), end(&clean, 1, 1)])).await;
    send(&mut a, &close(&clean, Some(1))).await;
    a.shutdown().await.unwrap();
    let mut b: Conn = TcpStream::connect(addr).await.unwrap();
    send(
        &mut b,
        &handshake("e0123456789abcdef", &idle, secret.expose()),
    )
    .await;
    send(&mut b, &item(&connection_open(&idle, 0))).await;
    sink.wait_items(4).await;

    // The owner turns a Closed outcome into ProducerClosed after draining frames through
    // `final_seq`, and a Lost outcome into interrupted streams after `admitted_through`.
    let first = clean_close(&entry).await;
    assert_eq!(first.final_seq, Some(1));
    let report = entry
        .drain(Instant::now() + Duration::from_millis(200))
        .await;
    assert_eq!(report.producers_cancelled, 1);
    let ProducerOutcome::Lost(lost) = one_outcome(&entry).await else {
        panic!("the idle producer is cut at the drain deadline");
    };
    assert_eq!(
        (lost.producer.as_str(), lost.admitted_through),
        (idle.as_str(), Some(0))
    );
    assert!(!entry.snapshot().admission_open);

    // After drain, admission stays closed.
    let mut late: Conn = TcpStream::connect(addr).await.unwrap();
    send_lossy(
        &mut late,
        &handshake("e0123456789abcdef", &pid(3), secret.expose()),
    )
    .await;
    expect_closed(&mut late).await;
    expect_closed(&mut b).await;

    let report = listener
        .shutdown(Instant::now() + Duration::from_millis(500))
        .await;
    assert_eq!(
        report.tasks_aborted, 0,
        "every task joined within the deadline"
    );
}

#[tokio::test]
async fn dropping_the_entry_revokes_its_authorization() {
    let rig = Rig::new();
    let sink = RecordingSink::new(8);
    let t = token("revoke");
    let entry = rig.authorize("e", &t, sink.clone());
    let p = pid(1);
    let mut conn = rig.producer("e", &t, &p).await;
    send(&mut conn, &item(&connection_open(&p, 0))).await;
    sink.wait_items(1).await;

    drop(entry);
    expect_closed(&mut conn).await;
    let mut late = rig.producer("e", &t, &pid(2)).await;
    expect_closed(&mut late).await;

    // The entry ID can be authorized again with a fresh launch secret.
    let t2 = token("relaunch");
    let entry = rig.authorize("e", &t2, sink.clone());
    let q = pid(3);
    let mut conn = rig.producer("e", &t2, &q).await;
    send(&mut conn, &item(&connection_open(&q, 0))).await;
    sink.wait_items(2).await;
    assert_eq!(entry.snapshot().producers_consumed, 1);
}
