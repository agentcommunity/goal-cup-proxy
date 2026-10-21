//! Test-local fixtures for `m2_proxy_integration`, all loopback-only.
//!
//! - [`DirectSink`]: a byte-bounded direct `CaptureSink`. Submission only enqueues (no disk, no
//!   network, no await); a worker thread validates what it dequeues against the submission rules
//!   (sequence, stream transitions, ordered close) and appends it to an in-memory retained list.
//!   Only that worker turns a producer close into [`Retained::ProducerClosed`]: a close the sink
//!   merely queued is not retained until the worker validates it. Raw frames live only in test
//!   memory. Every call is logged in order ([`Call`]).
//! - [`TestPki`], the rustls client configuration, the byte helpers and the RFC 6455
//!   upgrade bytes come from `m2_common` and are re-exported here.
//! - [`Upstream`]: a real TLS server on 127.0.0.1 that hands each handshaken connection to
//!   the test, which scripts the provider side byte by byte.
//! - [`Rig`]: the real proxy (`goalcup_cli::proxy::start`) over a pre-bound loopback listener,
//!   a direct sink and a private CA directory.
//! - Clients: an authenticated CONNECT and a rustls client that trusts exactly the roots it
//!   is given (the recording proxy's per-launch CA for interception).
//! - An oracle over retained frames that re-checks the frame rules without relying on the
//!   emitter.
//!
//! Every wait is bounded by `m2_common::DEADLINE`; spawned tasks and threads end with their
//! owners.

pub use super::m2_common::{
    ClientTls, HTTP11, TestPki, UpstreamTls, client_config, close_both, contains, eventually,
    provider, read_at_least, read_head, read_to_end, send, within, ws_101, ws_upgrade_request,
};
use goalcup_cli::proxy::api::{CapturePath, CapturePolicy};
use goalcup_cli::proxy::connect::{ApprovedHost, PROXY_USERNAME, ProxyCredential};
use goalcup_cli::proxy::tls::UpstreamRoots;
use goalcup_cli::proxy::{ProxyConfig, ProxyHandle, ProxyStopReport, StartError};
use goalcup_protocol::capture::{
    CaptureFrame, CaptureMeta, CaptureSink, CaptureSource, ConnectionEvent, EndReason,
    EpochAdmission, FrameEvent, GapReason, StreamKey, SubmitResult,
};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, ServerName};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_rustls::{TlsAcceptor, TlsConnector};

/// The configured initial producer ID.
pub const PRODUCER: &str = "7e130000000000000000000000c0ffee";

/// The proxy credential's secret: also a sentinel no retained byte may contain.
pub const SECRET: &str = "5ec2e75ec2e75ec2e75ec2e75ec2e7aa";

/// A generous direct-sink bound for tests that do not exercise pressure.
pub const QUEUE_BYTES: usize = 8 * 1024 * 1024;

pub fn count(haystack: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() || haystack.len() < needle.len() {
        return 0;
    }
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

// ---------------------------------------------------------------------------------------
// The direct sink.
// ---------------------------------------------------------------------------------------

/// One call the recording proxy made on the sink, in call order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    Begin {
        producer: String,
        answer: EpochAdmission,
    },
    Submit {
        producer: String,
        seq: u64,
        answer: SubmitResult,
    },
    Close {
        producer: String,
        final_seq: Option<u64>,
        answer: SubmitResult,
    },
}

/// What the worker appended to the retained list, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
// Test memory only: an unboxed frame keeps the matches in the tests plain.
#[allow(clippy::large_enum_variant)]
pub enum Retained {
    Frame(CaptureFrame),
    /// A sticky loss for a producer whose submission this sink refused.
    Gap {
        producer: String,
        reason: GapReason,
    },
    /// A close the worker validated: exact final sequence, no open payload stream.
    ProducerClosed {
        producer: String,
        final_seq: Option<u64>,
    },
    /// Anything the worker refused to retain as valid.
    Invalid {
        producer: String,
        why: String,
    },
}

// Test memory only, as for `Retained`.
#[allow(clippy::large_enum_variant)]
enum Item {
    Frame(CaptureFrame, usize),
    Lost {
        producer: String,
        reason: GapReason,
    },
    Close {
        producer: String,
        final_seq: Option<u64>,
    },
}

struct Check {
    next_seq: u64,
    last_seq: Option<u64>,
    next_stream: u64,
    open: BTreeSet<u64>,
    closed: bool,
}

impl Default for Check {
    fn default() -> Self {
        Check {
            next_seq: 0,
            last_seq: None,
            next_stream: 1,
            open: BTreeSet::new(),
            closed: false,
        }
    }
}

#[derive(Default)]
struct SinkState {
    queue: VecDeque<Item>,
    queued_bytes: usize,
    script: VecDeque<EpochAdmission>,
    hold_admission: bool,
    pressure: bool,
    blocked: bool,
    stall_submits: bool,
    stalled: usize,
    stopping: bool,
    calls: Vec<Call>,
    retained: Vec<Retained>,
    admitted: BTreeSet<String>,
    lost: BTreeSet<String>,
    checks: BTreeMap<String, Check>,
}

type SinkShared = Arc<(Mutex<SinkState>, Condvar)>;

fn lock(shared: &SinkShared) -> MutexGuard<'_, SinkState> {
    shared.0.lock().unwrap_or_else(|p| p.into_inner())
}

/// See the module documentation.
pub struct DirectSink {
    shared: SinkShared,
    max_queue_bytes: usize,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl DirectSink {
    pub fn new(max_queue_bytes: usize) -> Arc<Self> {
        let shared: SinkShared = Arc::new((Mutex::new(SinkState::default()), Condvar::new()));
        let worker_shared = Arc::clone(&shared);
        let worker = std::thread::spawn(move || worker(worker_shared));
        Arc::new(DirectSink {
            shared,
            max_queue_bytes,
            worker: Mutex::new(Some(worker)),
        })
    }

    /// Answers for the next `try_begin_epoch` calls (after them: `Ready`).
    pub fn script(&self, answers: impl IntoIterator<Item = EpochAdmission>) {
        lock(&self.shared).script.extend(answers);
    }

    /// While held, every admission is `Pending`.
    pub fn hold_admission(&self, hold: bool) {
        lock(&self.shared).hold_admission = hold;
    }

    /// While on, every submission is refused `QueueFull` (with one sticky loss per producer)
    /// and every fresh epoch stays `Pending`.
    pub fn set_pressure(&self, on: bool) {
        lock(&self.shared).pressure = on;
    }

    /// While blocked, the worker validates nothing: accepted items only wait in the queue.
    pub fn block_worker(&self, blocked: bool) {
        lock(&self.shared).blocked = blocked;
        self.shared.1.notify_all();
    }

    /// While on, `try_submit` blocks inside the call: a sink that breaks the submission contract's
    /// promise of never waiting. Only for proving the recording proxy never invents a drain around
    /// it.
    pub fn stall_submits(&self, stall: bool) {
        lock(&self.shared).stall_submits = stall;
        self.shared.1.notify_all();
    }

    /// How many `try_submit` calls are blocked right now.
    pub fn stalled(&self) -> usize {
        lock(&self.shared).stalled
    }

    pub fn calls(&self) -> Vec<Call> {
        lock(&self.shared).calls.clone()
    }

    pub fn retained(&self) -> Vec<Retained> {
        lock(&self.shared).retained.clone()
    }

    /// The retained frames only, in retained order.
    pub fn frames(&self) -> Vec<CaptureFrame> {
        self.retained()
            .into_iter()
            .filter_map(|r| match r {
                Retained::Frame(frame) => Some(frame),
                _ => None,
            })
            .collect()
    }

    pub fn queued(&self) -> usize {
        lock(&self.shared).queue.len()
    }

    /// Waits until the worker has validated everything queued so far.
    pub async fn settle(&self) {
        eventually("direct sink worker drains its queue", || {
            lock(&self.shared).queue.is_empty()
        })
        .await;
    }
}

impl Drop for DirectSink {
    fn drop(&mut self) {
        lock(&self.shared).stopping = true;
        self.shared.1.notify_all();
        if let Some(worker) = self.worker.lock().unwrap().take() {
            let _ = worker.join();
        }
    }
}

fn frame_size(frame: &CaptureFrame) -> usize {
    serde_json::to_vec(frame).map_or(usize::MAX, |bytes| bytes.len())
}

impl CaptureSink for DirectSink {
    fn try_submit(&self, frame: CaptureFrame) -> SubmitResult {
        let size = frame_size(&frame);
        let mut state = lock(&self.shared);
        if state.stall_submits {
            state.stalled += 1;
            while state.stall_submits && !state.stopping {
                state = self.shared.1.wait(state).unwrap_or_else(|p| p.into_inner());
            }
            state.stalled -= 1;
        }
        let producer = frame.stream.producer.clone();
        let seq = frame.seq;
        let answer = if !state.admitted.contains(&producer) {
            SubmitResult::Disabled(GapReason::IngestLost)
        } else if state.pressure
            || state.lost.contains(&producer)
            || state.queued_bytes.saturating_add(size) > self.max_queue_bytes
        {
            if state.lost.insert(producer.clone()) {
                state.queue.push_back(Item::Lost {
                    producer: producer.clone(),
                    reason: GapReason::QueueFull,
                });
            }
            SubmitResult::Disabled(GapReason::QueueFull)
        } else {
            state.queued_bytes += size;
            state.queue.push_back(Item::Frame(frame, size));
            SubmitResult::Queued
        };
        state.calls.push(Call::Submit {
            producer,
            seq,
            answer,
        });
        drop(state);
        self.shared.1.notify_all();
        answer
    }

    fn try_begin_epoch(&self, producer: &str) -> EpochAdmission {
        let mut state = lock(&self.shared);
        let answer = if state.hold_admission || state.pressure {
            EpochAdmission::Pending
        } else {
            state.script.pop_front().unwrap_or(EpochAdmission::Ready)
        };
        if answer == EpochAdmission::Ready {
            state.admitted.insert(producer.to_string());
        }
        state.calls.push(Call::Begin {
            producer: producer.to_string(),
            answer,
        });
        answer
    }

    fn try_close_epoch(&self, producer: &str, final_seq: Option<u64>) -> SubmitResult {
        let mut state = lock(&self.shared);
        let answer = if !state.admitted.contains(producer) || state.lost.contains(producer) {
            SubmitResult::Disabled(GapReason::IngestLost)
        } else if state.pressure {
            SubmitResult::Disabled(GapReason::QueueFull)
        } else {
            state.queue.push_back(Item::Close {
                producer: producer.to_string(),
                final_seq,
            });
            SubmitResult::Queued
        };
        state.calls.push(Call::Close {
            producer: producer.to_string(),
            final_seq,
            answer,
        });
        drop(state);
        self.shared.1.notify_all();
        answer
    }
}

fn worker(shared: SinkShared) {
    let (mutex, wake) = &*shared;
    let mut state = mutex.lock().unwrap_or_else(|p| p.into_inner());
    loop {
        if state.stopping {
            return;
        }
        if !state.blocked
            && let Some(item) = state.queue.pop_front()
        {
            validate(&mut state, item);
            continue;
        }
        state = wake.wait(state).unwrap_or_else(|p| p.into_inner());
    }
}

fn validate(state: &mut SinkState, item: Item) {
    match item {
        Item::Lost { producer, reason } => state.retained.push(Retained::Gap { producer, reason }),
        Item::Close {
            producer,
            final_seq,
        } => {
            let check = state.checks.entry(producer.clone()).or_default();
            let valid = !check.closed && check.open.is_empty() && check.last_seq == final_seq;
            check.closed = true;
            state.retained.push(if valid {
                Retained::ProducerClosed {
                    producer,
                    final_seq,
                }
            } else {
                Retained::Invalid {
                    producer,
                    why: format!(
                        "close final_seq {final_seq:?} vs last {:?}, open streams {}",
                        check.last_seq,
                        check.open.len()
                    ),
                }
            });
        }
        Item::Frame(frame, size) => {
            state.queued_bytes -= size;
            let producer = frame.stream.producer.clone();
            let check = state.checks.entry(producer.clone()).or_default();
            let why = frame_problem(check, &frame);
            match why {
                None => {
                    check.last_seq = Some(frame.seq);
                    check.next_seq = frame.seq + 1;
                    state.retained.push(Retained::Frame(frame));
                }
                Some(why) => state.retained.push(Retained::Invalid { producer, why }),
            }
        }
    }
}

fn frame_problem(check: &mut Check, frame: &CaptureFrame) -> Option<String> {
    if check.closed {
        return Some("frame after close".into());
    }
    if frame.seq != check.next_seq {
        return Some(format!("seq {} expected {}", frame.seq, check.next_seq));
    }
    if frame.observed.clock_id != frame.stream.producer {
        return Some("clock_id is not the producer".into());
    }
    if frame.validate().is_err() {
        return Some("frame fails C1 validation".into());
    }
    let stream = frame.stream.stream;
    match &frame.event {
        FrameEvent::Connection { .. } | FrameEvent::Recovery { .. } | FrameEvent::Loss { .. } => {
            (stream != 0).then(|| "control off stream zero".into())
        }
        FrameEvent::Open { .. } => {
            if stream != check.next_stream {
                return Some(format!("open {stream} expected {}", check.next_stream));
            }
            check.next_stream += 1;
            check.open.insert(stream);
            None
        }
        FrameEvent::Bytes { .. } | FrameEvent::Gap { .. } => {
            (!check.open.contains(&stream)).then(|| format!("payload on closed stream {stream}"))
        }
        FrameEvent::End { .. } => {
            (!check.open.remove(&stream)).then(|| format!("end on closed stream {stream}"))
        }
    }
}

/// Whether any retained record carries `needle`, in its payload bytes or its JSON text.
pub fn retained_contains(retained: &[Retained], needle: &[u8]) -> bool {
    retained.iter().any(|record| match record {
        Retained::Frame(frame) => {
            let in_bytes =
                matches!(&frame.event, FrameEvent::Bytes { data, .. } if contains(data, needle));
            in_bytes || contains(&serde_json::to_vec(frame).unwrap(), needle)
        }
        other => contains(format!("{other:?}").as_bytes(), needle),
    })
}

// ---------------------------------------------------------------------------------------
// An oracle over retained frames.
// ---------------------------------------------------------------------------------------

/// One payload stream reassembled from retained frames.
#[derive(Debug, Clone)]
pub struct StreamView {
    pub key: StreamKey,
    pub source: CaptureSource,
    pub meta: CaptureMeta,
    pub bytes: Vec<u8>,
    pub gaps: Vec<GapReason>,
    pub end: Option<EndReason>,
    /// Positions in the retained frame list.
    pub open_at: usize,
    pub end_at: Option<usize>,
}

impl StreamView {
    pub fn connection(&self) -> &str {
        &self.meta.wire.as_ref().expect("a wire stream").connection
    }

    pub fn exchange(&self) -> u64 {
        self.meta.wire.as_ref().expect("a wire stream").exchange
    }

    pub fn message_index(&self) -> Option<u64> {
        self.meta.wire.as_ref().and_then(|w| w.message_index)
    }
}

/// One `Connection` control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlView {
    pub at: usize,
    pub producer: String,
    pub connection: String,
    pub event: ConnectionEvent,
}

pub fn streams(frames: &[CaptureFrame]) -> Vec<StreamView> {
    let mut views: Vec<StreamView> = Vec::new();
    let mut index: BTreeMap<StreamKey, usize> = BTreeMap::new();
    for (at, frame) in frames.iter().enumerate() {
        match &frame.event {
            FrameEvent::Open { source, metadata } => {
                index.insert(frame.stream.clone(), views.len());
                views.push(StreamView {
                    key: frame.stream.clone(),
                    source: *source,
                    meta: metadata.clone(),
                    bytes: Vec::new(),
                    gaps: Vec::new(),
                    end: None,
                    open_at: at,
                    end_at: None,
                });
            }
            FrameEvent::Bytes { data, .. } => {
                views[index[&frame.stream]].bytes.extend_from_slice(data);
            }
            FrameEvent::Gap { reason, .. } => views[index[&frame.stream]].gaps.push(*reason),
            FrameEvent::End { reason } => {
                let view = &mut views[index[&frame.stream]];
                view.end = Some(*reason);
                view.end_at = Some(at);
            }
            FrameEvent::Connection { .. }
            | FrameEvent::Recovery { .. }
            | FrameEvent::Loss { .. } => {}
        }
    }
    views
}

/// `(position, reason)` of each stream-zero `Loss`, in frame order.
pub fn losses(frames: &[CaptureFrame]) -> Vec<(usize, GapReason)> {
    frames
        .iter()
        .enumerate()
        .filter_map(|(at, frame)| match &frame.event {
            FrameEvent::Loss { reason } => Some((at, *reason)),
            _ => None,
        })
        .collect()
}

pub fn controls(frames: &[CaptureFrame]) -> Vec<ControlView> {
    frames
        .iter()
        .enumerate()
        .filter_map(|(at, frame)| match &frame.event {
            FrameEvent::Connection { connection, event } => Some(ControlView {
                at,
                producer: frame.stream.producer.clone(),
                connection: connection.clone(),
                event: event.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// `(position, previous_producer, producer, reason)` of each `Recovery`.
pub fn recoveries(frames: &[CaptureFrame]) -> Vec<(usize, String, String, GapReason)> {
    frames
        .iter()
        .enumerate()
        .filter_map(|(at, frame)| match &frame.event {
            FrameEvent::Recovery {
                previous_producer,
                reason,
            } => Some((
                at,
                previous_producer.clone(),
                frame.stream.producer.clone(),
                *reason,
            )),
            _ => None,
        })
        .collect()
}

/// Re-checks the frame rules over the retained frames, independently of the emitter: per producer a
/// sequence from zero with no hole, streams numbered from one in `Open` order, contiguous
/// `Bytes` offsets, at most one `End`, stream zero exactly for controls, the producer as the
/// clock, typed wire identity per source, a payload stream only on a connection that producer
/// observed open, and no `Close` of a connection while one of its streams is open.
pub fn check_frames(frames: &[CaptureFrame]) {
    #[derive(Default)]
    struct P {
        next_seq: u64,
        next_stream: u64,
        live: BTreeSet<String>,
        open: BTreeMap<u64, (u64, Option<String>)>,
    }
    let mut producers: BTreeMap<String, P> = BTreeMap::new();
    for (at, frame) in frames.iter().enumerate() {
        let p = producers
            .entry(frame.stream.producer.clone())
            .or_insert_with(|| P {
                next_stream: 1,
                ..P::default()
            });
        assert_eq!(
            frame.seq, p.next_seq,
            "frame {at}: sequence hole or reorder"
        );
        p.next_seq += 1;
        assert_eq!(
            frame.observed.clock_id, frame.stream.producer,
            "frame {at}: clock is not the producer"
        );
        assert!(frame.validate().is_ok(), "frame {at} fails C1 validation");
        let stream = frame.stream.stream;
        match &frame.event {
            FrameEvent::Connection { connection, event } => {
                assert_eq!(stream, 0, "frame {at}: control off stream zero");
                match event {
                    ConnectionEvent::Open { .. } => {
                        p.live.insert(connection.clone());
                    }
                    ConnectionEvent::UpgradedWebSocket => {
                        assert!(p.live.contains(connection), "frame {at}: upgrade unseen");
                    }
                    ConnectionEvent::Close { .. } => {
                        assert!(
                            !p.open
                                .values()
                                .any(|(_, c)| c.as_deref() == Some(connection.as_str())),
                            "frame {at}: connection closed with a payload stream open"
                        );
                        p.live.remove(connection);
                    }
                }
            }
            FrameEvent::Recovery { .. } => assert_eq!(stream, 0, "frame {at}: recovery"),
            FrameEvent::Loss { .. } => assert_eq!(stream, 0, "frame {at}: loss"),
            FrameEvent::Open { source, metadata } => {
                assert_eq!(stream, p.next_stream, "frame {at}: stream numbering");
                p.next_stream += 1;
                let connection = match source {
                    CaptureSource::WireRequest | CaptureSource::WireResponse => {
                        let wire = metadata.wire.as_ref().expect("HTTP stream without wire");
                        assert!(wire.message_index.is_none(), "frame {at}: HTTP index");
                        Some(wire.connection.clone())
                    }
                    CaptureSource::WsClient | CaptureSource::WsServer => {
                        let wire = metadata.wire.as_ref().expect("WS stream without wire");
                        assert!(wire.message_index.is_some(), "frame {at}: WS index");
                        Some(wire.connection.clone())
                    }
                    CaptureSource::Native | CaptureSource::Goal => None,
                };
                if let Some(connection) = &connection {
                    assert!(
                        p.live.contains(connection),
                        "frame {at}: stream on a connection this producer never saw open"
                    );
                }
                p.open.insert(stream, (0, connection));
            }
            FrameEvent::Bytes { offset, data } => {
                let (next, _) = p
                    .open
                    .get_mut(&stream)
                    .unwrap_or_else(|| panic!("frame {at}: bytes on a closed stream"));
                assert_eq!(*offset, *next, "frame {at}: offsets not contiguous");
                *next += data.len() as u64;
            }
            FrameEvent::Gap { .. } => {
                assert!(
                    p.open.contains_key(&stream),
                    "frame {at}: gap on closed stream"
                );
            }
            FrameEvent::End { .. } => {
                assert!(
                    p.open.remove(&stream).is_some(),
                    "frame {at}: end on a closed stream"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// The synthetic upstream.
// ---------------------------------------------------------------------------------------

/// One handshaken upstream connection, handed to the test.
pub struct UpstreamConn {
    pub tls: UpstreamTls,
    pub sni: Option<String>,
}

/// A loopback TLS server offering `http/1.1`; see the module documentation.
pub struct Upstream {
    pub addr: SocketAddr,
    conns: mpsc::Receiver<UpstreamConn>,
    task: Option<JoinHandle<()>>,
}

impl Upstream {
    pub async fn start(pki: &TestPki, names: &[&str]) -> Self {
        let (chain, key) = pki.leaf(names);
        let mut config = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .unwrap();
        config.alpn_protocols = vec![HTTP11.to_vec()];
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, conns) = mpsc::channel(64);
        let task = tokio::spawn(async move {
            let mut handshakes = tokio::task::JoinSet::new();
            while let Ok((tcp, _)) = listener.accept().await {
                let _ = tcp.set_nodelay(true);
                let acceptor = acceptor.clone();
                let tx = tx.clone();
                handshakes.spawn(async move {
                    if let Ok(tls) = acceptor.accept(tcp).await {
                        let sni = tls.get_ref().1.server_name().map(str::to_owned);
                        let _ = tx.send(UpstreamConn { tls, sni }).await;
                    }
                });
            }
        });
        Self {
            addr,
            conns,
            task: Some(task),
        }
    }

    pub async fn accept(&mut self) -> UpstreamConn {
        within("upstream connection", self.conns.recv())
            .await
            .expect("upstream server ended")
    }

    pub fn try_accept(&mut self) -> Option<UpstreamConn> {
        self.conns.try_recv().ok()
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

// ---------------------------------------------------------------------------------------
// The recording proxy under test.
// ---------------------------------------------------------------------------------------

pub fn policy(paths: &[(&str, bool, Option<&str>)]) -> CapturePolicy {
    CapturePolicy {
        paths: paths
            .iter()
            .map(|(path, prefix, hint)| CapturePath {
                path: path.to_string(),
                prefix: *prefix,
                protocol_hint: hint.map(str::to_string),
            })
            .collect(),
    }
}

/// The capture policy most tests use for the approved host.
pub fn default_policy() -> CapturePolicy {
    policy(&[
        ("/v1/responses", false, Some("openai-responses")),
        ("/v1/realtime", false, Some("openai-realtime")),
        ("/v1/unknown", false, Some("vendor-unknown-v9")),
    ])
}

pub fn approved(host: &str, policy: CapturePolicy) -> ApprovedHost {
    ApprovedHost {
        host: host.to_string(),
        policy,
    }
}

/// What a [`Rig`] starts with. Defaults: `localhost` approved with [`default_policy`], no
/// authentication host, [`PRODUCER`], 5 s startup and 3 s shutdown deadlines, 32 connections.
pub struct RigConfig {
    pub approved: Vec<ApprovedHost>,
    pub auth_hosts: Vec<String>,
    pub roots: Vec<CertificateDer<'static>>,
    pub producer: String,
    pub startup: Duration,
    pub shutdown: Duration,
    pub max_connections: usize,
}

impl RigConfig {
    pub fn new(pki: &TestPki) -> Self {
        Self {
            approved: vec![approved("localhost", default_policy())],
            auth_hosts: Vec::new(),
            roots: vec![pki.root()],
            producer: PRODUCER.to_string(),
            startup: Duration::from_secs(5),
            shutdown: Duration::from_secs(3),
            max_connections: 32,
        }
    }
}

/// A private CA directory and the [`ProxyConfig`] for `listener`.
pub fn proxy_config(
    listener: std::net::TcpListener,
    dir: &std::path::Path,
    config: &RigConfig,
    sink: Arc<DirectSink>,
) -> ProxyConfig {
    ProxyConfig {
        listener,
        ca_dir: dir.to_path_buf(),
        approved: config.approved.clone(),
        auth_hosts: config.auth_hosts.clone(),
        credential: ProxyCredential::from_secret(SECRET.to_string()).unwrap(),
        upstream_roots: UpstreamRoots::from_certificates(config.roots.clone()).unwrap(),
        producer: config.producer.clone(),
        sink,
        startup_deadline: config.startup,
        shutdown_deadline: config.shutdown,
        admission_timeout: Duration::from_secs(5),
        connect_attempt_timeout: Duration::from_secs(2),
        max_connections: config.max_connections,
    }
}

pub fn loopback_listener() -> std::net::TcpListener {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap()
}

/// A running proxy with its sink and CA directory.
pub struct Rig {
    pub handle: Option<ProxyHandle>,
    pub sink: Arc<DirectSink>,
    pub dir: tempfile::TempDir,
    pub addr: SocketAddr,
    pub ca: CertificateDer<'static>,
}

impl Rig {
    pub async fn start(config: RigConfig, sink: Arc<DirectSink>) -> Self {
        Self::try_start(config, sink)
            .await
            .unwrap_or_else(|e| panic!("proxy did not start: {e}"))
    }

    pub async fn try_start(config: RigConfig, sink: Arc<DirectSink>) -> Result<Self, StartError> {
        let dir = tempfile::tempdir().unwrap();
        let listener = loopback_listener();
        let config = proxy_config(listener, dir.path(), &config, Arc::clone(&sink));
        let handle = within("proxy start", goalcup_cli::proxy::start(config)).await?;
        Ok(Self {
            addr: handle.local_addr(),
            ca: handle.ca_certificate_der().clone(),
            handle: Some(handle),
            sink,
            dir,
        })
    }

    pub fn handle(&self) -> &ProxyHandle {
        self.handle.as_ref().expect("proxy already stopped")
    }

    pub async fn stop(&mut self) -> ProxyStopReport {
        let handle = self.handle.take().expect("proxy already stopped");
        within("proxy stop", handle.stop()).await
    }

    /// Waits until `n` intercepted connections have closed.
    pub async fn wait_closed(&self, n: u64) {
        let handle = self.handle();
        eventually("intercepted connections close", || {
            let h = handle.health();
            h.closed_complete + h.closed_interrupted + h.closed_transport_error >= n
        })
        .await;
    }

    /// An intercepted client for `localhost:<port>` trusting only this proxy's CA.
    pub async fn client(&self, port: u16) -> ClientTls {
        let tcp = connect_via(self.addr, &format!("localhost:{port}")).await;
        tls_over(tcp, std::slice::from_ref(&self.ca), "localhost").await
    }
}

// ---------------------------------------------------------------------------------------
// Clients.
// ---------------------------------------------------------------------------------------

pub fn proxy_authorization() -> String {
    use base64::Engine as _;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{PROXY_USERNAME}:{SECRET}"))
    )
}

pub fn connect_head(target: &str) -> Vec<u8> {
    format!(
        "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nProxy-Authorization: {}\r\n\r\n",
        proxy_authorization()
    )
    .into_bytes()
}

/// Sends an authenticated CONNECT for `target` and returns the socket after its `200`.
pub async fn connect_via(proxy: SocketAddr, target: &str) -> TcpStream {
    let mut tcp = TcpStream::connect(proxy).await.unwrap();
    tcp.set_nodelay(true).unwrap();
    tcp.write_all(&connect_head(target)).await.unwrap();
    let (head, rest) = within("CONNECT response", read_head(&mut tcp)).await;
    assert!(
        head.starts_with(b"HTTP/1.1 200 "),
        "CONNECT refused: {}",
        String::from_utf8_lossy(&head)
    );
    assert!(rest.is_empty(), "bytes after the 200 head");
    tcp
}

/// A rustls client over `tcp` for `name`, trusting exactly `roots`.
pub async fn tls_over(tcp: TcpStream, roots: &[CertificateDer<'static>], name: &str) -> ClientTls {
    try_tls_over(tcp, roots, name)
        .await
        .unwrap_or_else(|e| panic!("client TLS for {name} failed: {e}"))
}

pub async fn try_tls_over(
    tcp: TcpStream,
    roots: &[CertificateDer<'static>],
    name: &str,
) -> std::io::Result<ClientTls> {
    let name = ServerName::try_from(name.to_owned()).unwrap();
    within(
        "client TLS handshake",
        TlsConnector::from(client_config(roots)).connect(name, tcp),
    )
    .await
}

// ---------------------------------------------------------------------------------------
// Byte helpers.
// ---------------------------------------------------------------------------------------

/// Reads into `buf` until it contains `needle`.
pub async fn read_until<S: AsyncRead + Unpin>(stream: &mut S, buf: &mut Vec<u8>, needle: &[u8]) {
    let mut chunk = vec![0u8; 16 * 1024];
    within("read_until", async {
        while !contains(buf, needle) {
            let read = stream.read(&mut chunk).await.expect("read failed");
            assert!(read > 0, "EOF before {:?}", String::from_utf8_lossy(needle));
            buf.extend_from_slice(&chunk[..read]);
        }
    })
    .await;
}

/// Reads one request: head and a `Content-Length` body. Returns `(head, body)`.
pub async fn read_request<S: AsyncRead + Unpin>(stream: &mut S) -> (Vec<u8>, Vec<u8>) {
    let (head, mut body) = within("request head", read_head(stream)).await;
    assert!(!head.is_empty(), "EOF before a request");
    let length = content_length(&head);
    read_at_least(stream, &mut body, length).await;
    assert_eq!(body.len(), length, "bytes beyond the body arrived early");
    (head, body)
}

pub fn content_length(head: &[u8]) -> usize {
    let text = String::from_utf8_lossy(head).to_ascii_lowercase();
    text.lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .map_or(0, |value| value.trim().parse().unwrap())
}

pub fn post(path: &str, body: &[u8], extra: &str) -> Vec<u8> {
    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    request.extend_from_slice(body);
    request
}

pub fn ok_response(content_type: &str, body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

/// One ordinary request/response on an intercepted keep-alive connection: the client sends
/// `POST path` with `body`, the upstream answers `reply`. Both ends are checked byte for
/// byte.
pub async fn exchange(
    client: &mut ClientTls,
    upstream: &mut UpstreamTls,
    path: &str,
    body: &[u8],
    reply: &[u8],
) {
    let request = post(path, body, "");
    send(client, &request).await;
    let (head, got) = read_request(upstream).await;
    assert_eq!([head, got].concat(), request, "upstream saw other bytes");
    let response = ok_response("application/json", reply);
    send(upstream, &response).await;
    let mut seen = Vec::new();
    read_at_least(client, &mut seen, response.len()).await;
    assert_eq!(seen, response, "client saw other bytes");
}

// ---------------------------------------------------------------------------------------
// WebSocket bytes (RFC 6455), written here independently of the relay.
// ---------------------------------------------------------------------------------------

fn frame(opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut out = vec![0x80 | opcode];
    let masked = if mask.is_some() { 0x80 } else { 0 };
    match payload.len() {
        n if n < 126 => out.push(masked | n as u8),
        n if n <= u16::MAX as usize => {
            out.push(masked | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(masked | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    match mask {
        Some(key) => {
            out.extend_from_slice(&key);
            out.extend(payload.iter().enumerate().map(|(i, b)| b ^ key[i % 4]));
        }
        None => out.extend_from_slice(payload),
    }
    out
}

/// A final, masked client text frame.
pub fn ws_client_text(payload: &[u8]) -> Vec<u8> {
    frame(0x1, payload, Some([0x37, 0xfa, 0x21, 0x3d]))
}

/// A final, unmasked server text frame.
pub fn ws_server_text(payload: &[u8]) -> Vec<u8> {
    frame(0x1, payload, None)
}

pub fn ws_client_close() -> Vec<u8> {
    frame(0x8, &[0x03, 0xe8], Some([1, 2, 3, 4]))
}

pub fn ws_server_close() -> Vec<u8> {
    frame(0x8, &[0x03, 0xe8], None)
}

/// One parsed WebSocket frame (payload unmasked).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsFrame {
    pub fin: bool,
    pub opcode: u8,
    pub payload: Vec<u8>,
}

/// Parses one complete frame from the front of `buf`: `(frame, bytes used)`, or `None` if
/// `buf` does not hold a whole frame yet.
pub fn parse_ws_frame(buf: &[u8]) -> Option<(WsFrame, usize)> {
    if buf.len() < 2 {
        return None;
    }
    let (fin, opcode) = (buf[0] & 0x80 != 0, buf[0] & 0x0f);
    let masked = buf[1] & 0x80 != 0;
    let (len, mut at) = match buf[1] & 0x7f {
        126 => (
            u16::from_be_bytes(buf.get(2..4)?.try_into().ok()?) as usize,
            4,
        ),
        127 => (
            u64::from_be_bytes(buf.get(2..10)?.try_into().ok()?) as usize,
            10,
        ),
        n => (n as usize, 2),
    };
    let key = if masked {
        let key: [u8; 4] = buf.get(at..at + 4)?.try_into().ok()?;
        at += 4;
        Some(key)
    } else {
        None
    };
    let body = buf.get(at..at + len)?;
    let payload = match key {
        Some(key) => body
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ key[i % 4])
            .collect(),
        None => body.to_vec(),
    };
    Some((
        WsFrame {
            fin,
            opcode,
            payload,
        },
        at + len,
    ))
}

/// One line per retained record, for evidence logs: position, producer prefix, sequence,
/// stream and a summary of the event (payload shown as a length, never as bytes).
pub fn trace(retained: &[Retained]) -> String {
    let mut out = String::new();
    for (at, record) in retained.iter().enumerate() {
        let line = match record {
            Retained::Frame(frame) => {
                let event = match &frame.event {
                    FrameEvent::Connection { connection, event } => {
                        format!("connection {} {event:?}", &connection[..8])
                    }
                    FrameEvent::Recovery {
                        previous_producer,
                        reason,
                    } => format!("recovery from {} ({reason:?})", &previous_producer[..8]),
                    FrameEvent::Open { source, metadata } => {
                        let wire = metadata.wire.as_ref().map(|w| {
                            format!(
                                " conn {} exch {} idx {:?}",
                                &w.connection[..8],
                                w.exchange,
                                w.message_index
                            )
                        });
                        format!(
                            "open {source:?}{} label {:?} status {:?} hint {:?}",
                            wire.unwrap_or_default(),
                            metadata.label,
                            metadata.status,
                            metadata.protocol_hint
                        )
                    }
                    FrameEvent::Bytes { offset, data } => {
                        format!("bytes @{offset} len {}", data.len())
                    }
                    FrameEvent::End { reason } => format!("end {reason:?}"),
                    FrameEvent::Gap { reason, lost_bytes } => {
                        format!("gap {reason:?} lost {lost_bytes:?}")
                    }
                    FrameEvent::Loss { reason } => format!("loss {reason:?}"),
                };
                format!(
                    "{at:3} {} seq {:3} stream {:2} {event}",
                    &frame.stream.producer[..8],
                    frame.seq,
                    frame.stream.stream
                )
            }
            Retained::Gap { producer, reason } => {
                format!("{at:3} {} sticky gap {reason:?}", &producer[..8])
            }
            Retained::ProducerClosed {
                producer,
                final_seq,
            } => format!(
                "{at:3} {} producer closed final_seq {final_seq:?}",
                &producer[..8]
            ),
            Retained::Invalid { producer, why } => {
                format!("{at:3} {} INVALID {why}", &producer[..8])
            }
        };
        out.push_str(&line);
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------------------------
// The one capture clock.
// ---------------------------------------------------------------------------------------

/// Opens a window on the recording proxy's one process clock, `proxy::metadata::capture_time()`:
/// its start lies strictly after that clock's zero, so an observation stamped zero, or read from
/// another monotonic base started later, falls outside it.
pub fn clock_start() -> u64 {
    let _ = goalcup_cli::proxy::metadata::capture_time();
    std::thread::sleep(Duration::from_millis(2));
    goalcup_cli::proxy::metadata::capture_time().mono_ns
}

/// The process clock now: the end of a window opened by [`clock_start`].
pub fn clock_now() -> u64 {
    goalcup_cli::proxy::metadata::capture_time().mono_ns
}

/// Every retained frame was observed on the recording proxy's process clock inside `[start, end]`,
/// and, per physical connection, in causal order: the root's `Open` (stamped before the relay
/// starts) precedes every observation of that connection's streams; the root's
/// `UpgradedWebSocket` (stamped after `relay_http` returned, before `relay_websocket`) follows
/// every HTTP observation and precedes every WebSocket one; and the root's `Close` (stamped
/// after the relay ended) follows all of them. Only same-connection pairs are compared, which
/// program order fixes; different connections stamp concurrently.
pub fn check_clock(frames: &[CaptureFrame], start: u64, end: u64) {
    #[derive(Default)]
    struct Span {
        open: Option<u64>,
        upgrade: Option<u64>,
        close: Option<u64>,
        http: Vec<u64>,
        ws: Vec<u64>,
    }
    let mut spans: BTreeMap<String, Span> = BTreeMap::new();
    let mut owner: BTreeMap<StreamKey, (String, bool)> = BTreeMap::new();
    for (at, frame) in frames.iter().enumerate() {
        let mono = frame.observed.mono_ns;
        assert!(
            (start..=end).contains(&mono),
            "frame {at}: mono_ns {mono} is not on the process clock window [{start}, {end}]"
        );
        match &frame.event {
            FrameEvent::Connection { connection, event } => {
                let span = spans.entry(connection.clone()).or_default();
                match event {
                    ConnectionEvent::Open {
                        already_open: false,
                        ..
                    } => span.open = Some(mono),
                    ConnectionEvent::Open { .. } => {}
                    ConnectionEvent::UpgradedWebSocket => span.upgrade = Some(mono),
                    ConnectionEvent::Close { .. } => span.close = Some(mono),
                }
            }
            FrameEvent::Recovery { .. } | FrameEvent::Loss { .. } => {}
            FrameEvent::Open { source, metadata } => {
                if let Some(wire) = &metadata.wire {
                    let ws = matches!(source, CaptureSource::WsClient | CaptureSource::WsServer);
                    owner.insert(frame.stream.clone(), (wire.connection.clone(), ws));
                    let span = spans.entry(wire.connection.clone()).or_default();
                    if ws { &mut span.ws } else { &mut span.http }.push(mono);
                }
            }
            FrameEvent::Bytes { .. } | FrameEvent::Gap { .. } | FrameEvent::End { .. } => {
                if let Some((connection, ws)) = owner.get(&frame.stream) {
                    let span = spans.entry(connection.clone()).or_default();
                    if *ws { &mut span.ws } else { &mut span.http }.push(mono);
                }
            }
        }
    }
    for (connection, span) in &spans {
        let observed = span.http.iter().chain(&span.ws);
        if let Some(open) = span.open {
            assert!(
                observed.clone().all(|&m| m >= open),
                "{connection}: a stream observation precedes the connection's Open"
            );
        }
        if let Some(close) = span.close {
            assert!(
                observed.clone().all(|&m| m <= close),
                "{connection}: a stream observation follows the connection's Close"
            );
            if let Some(open) = span.open {
                assert!(open <= close, "{connection}: Close precedes Open");
            }
        }
        if let Some(upgrade) = span.upgrade {
            assert!(
                span.http.iter().all(|&m| m <= upgrade),
                "{connection}: an HTTP observation follows UpgradedWebSocket"
            );
            assert!(
                span.ws.iter().all(|&m| m >= upgrade),
                "{connection}: a WebSocket observation precedes UpgradedWebSocket"
            );
        }
    }
}
