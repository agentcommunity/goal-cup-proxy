//! The composed capture chain over real loopback TCP.
//!
//! The recording proxy (`proxy::start`, with its one `CaptureEmitter`) → the IPC sender
//! (`proxy::ingest::connect_sink`) → the daemon's receiver (`goalcup_daemon::ingest`
//! `IngestListener`/`IngestEntry`) → a test-local entry-owner sink. The other suites test one
//! side against an emulator of the other; this file composes the real ones in this order:
//! `connect_sink`, then `proxy::start` with the same producer and the same sink, then
//! `ProxyHandle::stop`, then `IngestSink::shutdown`.
//!
//! Every client, upstream, listener and link is real loopback TCP, with real TLS on both proxy
//! legs. The PKI, the client configuration and the byte helpers come from `m2_common`; the
//! test-local pieces are:
//!
//! - `Owner`: the entry owner's sink behind the receiver. It admits epochs `Ready` unless held
//!   (`Pending`, the owner-pressure answer), keeps what the receiver offers in order, and can
//!   refuse the next frame as a full queue would.
//! - `Tee`: a transparent recorder between the emitter and the sender (tests 2 and 3 only). Every
//!   call passes through unchanged; it logs the call with the sender's answer, so a test can tell
//!   what the sender accepted from what the receiver admitted.
//! - `Link`: a loopback TCP forwarder between the sender and the receiver (tests 2 and 4 only). It
//!   copies bytes unchanged both ways, can hold a connection before it reaches the receiver, and
//!   can cut one.
//! - A TLS upstream, a CONNECT/TLS client, and a frame oracle over what the owner received
//!   that does not rely on either side's validator.
//!
//! Every wait is bounded by `DEADLINE`, and sequencing waits on observable state (the receiver's
//! outcome latch and snapshot, the sender's counters), never on a sleep standing in for an event.

use crate::common::m2_common;

use goalcup_cli::proxy::api::{CapturePath, CapturePolicy};
use goalcup_cli::proxy::connect::{ApprovedHost, PROXY_USERNAME, ProxyCredential};
use goalcup_cli::proxy::ingest::{
    IngestSecret as SenderSecret, IngestSink, SenderOutcome, SinkConfig, connect_sink,
};
use goalcup_cli::proxy::tls::UpstreamRoots;
use goalcup_cli::proxy::{ProxyConfig, ProxyHandle, ProxyStopReport};
use goalcup_daemon::ingest::{
    DEFAULT_SETUP_TIMEOUT, EntryGrant, IngestConfig, IngestEntry, IngestListener, IngestSecret,
    LossCause, ProducerLoss, ProducerOutcome, ShutdownReport, ValidatedClose, monotonic_clock,
};
use goalcup_protocol::capture::{
    CaptureFrame, CaptureMeta, CaptureSink, CaptureSource, ConnectionEvent, ConnectionOutcome,
    EndReason, EpochAdmission, FrameEvent, GapReason, StreamKey, SubmitResult,
};
use m2_common::{
    ClientTls, HTTP11, TestPki, UpstreamTls, client_config, close_both, contains, eventually,
    provider, read_at_least, read_head, send, within, ws_101, ws_upgrade_request,
};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, ServerName};
use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio::task::{AbortHandle, JoinHandle};
use tokio::time::Instant;
use tokio_rustls::{TlsAcceptor, TlsConnector};

const ENTRY: &str = "entry-cp2-0001";
/// The configured initial producer ID (32 lowercase hex).
const PRODUCER: &str = "c2c20000000000000000000000000001";
/// The proxy credential (32 lowercase hex).
const PROXY_SECRET: &str = "c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2aa";
/// The daemon clock the receiver stamps its own observations with.
const DAEMON_CLOCK: &str = "goalcup-daemon-cp2";
/// The sender's reconnect schedule, scaled down through its test-only knob.
const RECONNECT: Duration = Duration::from_millis(50);
const SENDER_DEADLINE: Duration = Duration::from_secs(5);
/// The most request boundaries a recovery may take: at most one to learn the loss, one to
/// propose a fresh ID (`Pending`), one to be answered `Ready`.
const MAX_BOUNDARIES: usize = 6;
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------------------------------
// The entry owner behind the receiver.
// ---------------------------------------------------------------------------------------

/// What the receiver handed the entry owner, in call order.
#[derive(Debug, Clone, PartialEq, Eq)]
// Test memory only: an unboxed frame keeps the matches plain.
#[allow(clippy::large_enum_variant)]
enum Owned {
    Frame(CaptureFrame),
    Close {
        producer: String,
        final_seq: Option<u64>,
    },
}

#[derive(Default)]
struct OwnerState {
    hold: bool,
    refuse_next: Option<GapReason>,
    begins: Vec<(String, EpochAdmission)>,
    items: Vec<Owned>,
    admitted: BTreeSet<String>,
    lost: BTreeSet<String>,
}

/// The entry owner's sink: synchronous and non-blocking, as the submission contract requires
/// of it.
#[derive(Default)]
struct Owner(Mutex<OwnerState>);

impl Owner {
    /// While held, every new epoch is answered `Pending`.
    fn hold(&self, on: bool) {
        lock(&self.0).hold = on;
    }

    /// Refuses the next frame with `reason` and loses that producer, as a full queue does.
    fn refuse_next(&self, reason: GapReason) {
        lock(&self.0).refuse_next = Some(reason);
    }

    fn items(&self) -> Vec<Owned> {
        lock(&self.0).items.clone()
    }

    fn frames(&self) -> Vec<CaptureFrame> {
        self.items()
            .into_iter()
            .filter_map(|item| match item {
                Owned::Frame(frame) => Some(frame),
                Owned::Close { .. } => None,
            })
            .collect()
    }

    fn begins(&self) -> Vec<(String, EpochAdmission)> {
        lock(&self.0).begins.clone()
    }
}

impl CaptureSink for Owner {
    fn try_submit(&self, frame: CaptureFrame) -> SubmitResult {
        let mut state = lock(&self.0);
        let producer = frame.stream.producer.clone();
        if !state.admitted.contains(&producer) || state.lost.contains(&producer) {
            return SubmitResult::Disabled(GapReason::ProducerLost);
        }
        if let Some(reason) = state.refuse_next.take() {
            state.lost.insert(producer);
            return SubmitResult::Disabled(reason);
        }
        state.items.push(Owned::Frame(frame));
        SubmitResult::Queued
    }

    fn try_begin_epoch(&self, producer: &str) -> EpochAdmission {
        let mut state = lock(&self.0);
        let answer = if state.hold {
            EpochAdmission::Pending
        } else {
            state.admitted.insert(producer.to_string());
            EpochAdmission::Ready
        };
        state.begins.push((producer.to_string(), answer));
        answer
    }

    fn try_close_epoch(&self, producer: &str, final_seq: Option<u64>) -> SubmitResult {
        let mut state = lock(&self.0);
        if !state.admitted.contains(producer) || state.lost.contains(producer) {
            return SubmitResult::Disabled(GapReason::ProducerLost);
        }
        state.items.push(Owned::Close {
            producer: producer.to_string(),
            final_seq,
        });
        SubmitResult::Queued
    }
}

// ---------------------------------------------------------------------------------------
// The transparent recorder between the emitter and the sender.
// ---------------------------------------------------------------------------------------

/// One call the emitter made on the sender, with the sender's answer.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
enum Sent {
    Begin {
        producer: String,
        answer: EpochAdmission,
    },
    Submit {
        frame: CaptureFrame,
        answer: SubmitResult,
    },
    Close {
        answer: SubmitResult,
    },
}

/// Passes every call straight to the sender and logs it in call order (the emitter holds its lock
/// across each call, so call order is sequence order). It changes no argument or answer.
struct Tee {
    inner: Arc<IngestSink>,
    log: Mutex<Vec<Sent>>,
}

impl Tee {
    fn begins(&self) -> Vec<(String, EpochAdmission)> {
        lock(&self.log)
            .iter()
            .filter_map(|sent| match sent {
                Sent::Begin { producer, answer } => Some((producer.clone(), *answer)),
                _ => None,
            })
            .collect()
    }

    /// Every `Recovery` the emitter submitted: `(producer, previous, reason, the sender's answer)`.
    fn recoveries(&self) -> Vec<(String, String, GapReason, SubmitResult)> {
        lock(&self.log)
            .iter()
            .filter_map(|sent| match sent {
                Sent::Submit {
                    frame:
                        CaptureFrame {
                            stream,
                            event:
                                FrameEvent::Recovery {
                                    previous_producer,
                                    reason,
                                },
                            ..
                        },
                    answer,
                } => Some((
                    stream.producer.clone(),
                    previous_producer.clone(),
                    *reason,
                    *answer,
                )),
                _ => None,
            })
            .collect()
    }

    /// The frames the sender answered `Queued` for `producer`, in order.
    fn queued(&self, producer: &str) -> Vec<CaptureFrame> {
        lock(&self.log)
            .iter()
            .filter_map(|sent| match sent {
                Sent::Submit {
                    frame,
                    answer: SubmitResult::Queued,
                } if frame.stream.producer == producer => Some(frame.clone()),
                _ => None,
            })
            .collect()
    }

    fn closes(&self) -> Vec<SubmitResult> {
        lock(&self.log)
            .iter()
            .filter_map(|sent| match sent {
                Sent::Close { answer } => Some(*answer),
                _ => None,
            })
            .collect()
    }
}

impl CaptureSink for Tee {
    fn try_submit(&self, frame: CaptureFrame) -> SubmitResult {
        let copy = frame.clone();
        let answer = self.inner.try_submit(frame);
        lock(&self.log).push(Sent::Submit {
            frame: copy,
            answer,
        });
        answer
    }

    fn try_begin_epoch(&self, producer: &str) -> EpochAdmission {
        let answer = self.inner.try_begin_epoch(producer);
        lock(&self.log).push(Sent::Begin {
            producer: producer.to_string(),
            answer,
        });
        answer
    }

    fn try_close_epoch(&self, producer: &str, final_seq: Option<u64>) -> SubmitResult {
        let answer = self.inner.try_close_epoch(producer, final_seq);
        lock(&self.log).push(Sent::Close { answer });
        answer
    }
}

// ---------------------------------------------------------------------------------------
// The loopback link between the sender and the receiver.
// ---------------------------------------------------------------------------------------

/// Accepts the sender's connections on its own loopback port and forwards each, byte for byte both
/// ways, to the receiver. While closed, an accepted connection waits before it reaches the
/// receiver; `cut(n)` drops both sockets of the n-th connection at once, as a dead daemon link
/// would.
struct Link {
    addr: SocketAddr,
    gate: watch::Sender<bool>,
    conns: Arc<Mutex<Vec<AbortHandle>>>,
    accept: JoinHandle<()>,
}

impl Link {
    async fn start(target: SocketAddr, open: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let gate = watch::channel(open).0;
        let conns: Arc<Mutex<Vec<AbortHandle>>> = Arc::default();
        let accept = tokio::spawn({
            let gate = gate.subscribe();
            let conns = Arc::clone(&conns);
            async move {
                while let Ok((from, _)) = listener.accept().await {
                    let _ = from.set_nodelay(true);
                    let task = tokio::spawn(forward(from, target, gate.clone()));
                    lock(&conns).push(task.abort_handle());
                }
            }
        });
        Self {
            addr,
            gate,
            conns,
            accept,
        }
    }

    fn release(&self) {
        self.gate.send_replace(true);
    }

    fn cut(&self, n: usize) {
        lock(&self.conns)[n].abort();
    }

    fn connections(&self) -> usize {
        lock(&self.conns).len()
    }
}

async fn forward(mut from: TcpStream, target: SocketAddr, mut gate: watch::Receiver<bool>) {
    if gate.wait_for(|open| *open).await.is_err() {
        return;
    }
    let Ok(mut to) = TcpStream::connect(target).await else {
        return;
    };
    let _ = to.set_nodelay(true);
    let _ = tokio::io::copy_bidirectional(&mut from, &mut to).await;
}

impl Drop for Link {
    fn drop(&mut self) {
        self.accept.abort();
        for conn in lock(&self.conns).iter() {
            conn.abort();
        }
    }
}

// ---------------------------------------------------------------------------------------
// The synthetic upstream and the client.
// ---------------------------------------------------------------------------------------

/// A loopback TLS server offering `http/1.1` that hands each handshaken connection to the
/// test, which scripts the provider side byte by byte.
struct Upstream {
    addr: SocketAddr,
    conns: mpsc::Receiver<UpstreamTls>,
    task: JoinHandle<()>,
}

impl Upstream {
    async fn start(pki: &TestPki) -> Self {
        let (chain, key) = pki.leaf(&["localhost"]);
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
        let (tx, conns) = mpsc::channel(16);
        let task = tokio::spawn(async move {
            let mut handshakes = tokio::task::JoinSet::new();
            while let Ok((tcp, _)) = listener.accept().await {
                let _ = tcp.set_nodelay(true);
                let (acceptor, tx) = (acceptor.clone(), tx.clone());
                handshakes.spawn(async move {
                    if let Ok(tls) = acceptor.accept(tcp).await {
                        let _ = tx.send(tls).await;
                    }
                });
            }
        });
        Self { addr, conns, task }
    }

    async fn accept(&mut self) -> UpstreamTls {
        within("upstream connection", self.conns.recv())
            .await
            .expect("upstream server ended")
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// An authenticated CONNECT to `target` through `proxy`, then TLS for `localhost` trusting
/// only `ca` (the recording proxy's per-launch CA).
async fn intercepted_client(
    proxy: SocketAddr,
    target: &str,
    ca: &CertificateDer<'static>,
) -> ClientTls {
    use base64::Engine as _;
    let mut tcp = TcpStream::connect(proxy).await.unwrap();
    tcp.set_nodelay(true).unwrap();
    let credential = base64::engine::general_purpose::STANDARD
        .encode(format!("{PROXY_USERNAME}:{PROXY_SECRET}"));
    let head = format!(
        "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nProxy-Authorization: Basic {credential}\r\n\r\n"
    );
    tcp.write_all(head.as_bytes()).await.unwrap();
    let (head, rest) = within("CONNECT response", read_head(&mut tcp)).await;
    assert!(head.starts_with(b"HTTP/1.1 200 "), "CONNECT refused");
    assert!(rest.is_empty(), "bytes after the 200 head");
    let name = ServerName::try_from("localhost".to_string()).unwrap();
    within(
        "client TLS handshake",
        TlsConnector::from(client_config(std::slice::from_ref(ca))).connect(name, tcp),
    )
    .await
    .expect("client TLS through the recording proxy")
}

// ---------------------------------------------------------------------------------------
// Byte helpers.
// ---------------------------------------------------------------------------------------

/// Writes `bytes` on `from` and asserts `to` receives exactly them, and nothing before.
async fn relay_exact<A, B>(from: &mut A, to: &mut B, bytes: &[u8])
where
    A: AsyncWrite + Unpin,
    B: AsyncRead + Unpin,
{
    send(from, bytes).await;
    let mut got = Vec::new();
    read_at_least(to, &mut got, bytes.len()).await;
    assert_eq!(got, bytes, "the relay changed, lost or added bytes");
}

fn post(path: &str, body: &[u8]) -> Vec<u8> {
    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    request.extend_from_slice(body);
    request
}

fn ok_response(body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

/// One request/response on an intercepted keep-alive connection, checked byte for byte at
/// both ends.
async fn exchange(client: &mut ClientTls, up: &mut UpstreamTls, body: &[u8], reply: &[u8]) {
    relay_exact(client, up, &post("/v1/responses", body)).await;
    relay_exact(up, client, &ok_response(reply)).await;
}

// RFC 6455 bytes, written here independently of the relay.
fn ws_frame(opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    assert!(payload.len() < 126, "short frames only");
    let mut out = vec![0x80 | opcode, payload.len() as u8];
    match mask {
        Some(key) => {
            out[1] |= 0x80;
            out.extend_from_slice(&key);
            out.extend(payload.iter().enumerate().map(|(i, b)| b ^ key[i % 4]));
        }
        None => out.extend_from_slice(payload),
    }
    out
}

const CLIENT_MASK: Option<[u8; 4]> = Some([0x37, 0xfa, 0x21, 0x3d]);

// ---------------------------------------------------------------------------------------
// An oracle over what the owner received.
// ---------------------------------------------------------------------------------------

/// Re-checks the frame rules over the frames the receiver handed the owner, relying on neither
/// side: per producer a sequence from zero with no hole, the producer as the clock, `Recovery` only
/// at sequence zero, controls exactly on stream zero, streams numbered from one in `Open` order,
/// contiguous `Bytes` offsets from zero, `End` only on an open stream, and a wire stream only on a
/// connection that producer observed open (or re-observed).
fn check_chain(frames: &[CaptureFrame]) {
    #[derive(Default)]
    struct P {
        next_seq: u64,
        next_stream: u64,
        live: BTreeSet<String>,
        open: BTreeMap<u64, u64>,
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
            "frame {at}: the clock is not the producer"
        );
        let stream = frame.stream.stream;
        match &frame.event {
            FrameEvent::Recovery { .. } => {
                assert_eq!((frame.seq, stream), (0, 0), "frame {at}: recovery position");
            }
            FrameEvent::Loss { .. } => {
                assert_eq!(stream, 0, "frame {at}: control off stream zero");
            }
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
                        assert!(p.live.remove(connection), "frame {at}: close unseen");
                    }
                }
            }
            FrameEvent::Open { metadata, .. } => {
                assert_eq!(stream, p.next_stream, "frame {at}: stream numbering");
                p.next_stream += 1;
                let wire = metadata.wire.as_ref().expect("a wire stream");
                assert!(
                    p.live.contains(&wire.connection),
                    "frame {at}: a stream on a connection this producer never saw open"
                );
                p.open.insert(stream, 0);
            }
            FrameEvent::Bytes { offset, data } => {
                let next = p
                    .open
                    .get_mut(&stream)
                    .unwrap_or_else(|| panic!("frame {at}: bytes on a closed stream"));
                assert_eq!(*offset, *next, "frame {at}: offsets not contiguous");
                *next += data.len() as u64;
            }
            FrameEvent::Gap { .. } => {
                assert!(
                    p.open.contains_key(&stream),
                    "frame {at}: gap on a closed stream"
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

/// One payload stream reassembled from the owner's frames.
#[derive(Debug, Clone)]
struct StreamView {
    key: StreamKey,
    source: CaptureSource,
    meta: CaptureMeta,
    bytes: Vec<u8>,
    end: Option<EndReason>,
}

impl StreamView {
    fn exchange(&self) -> u64 {
        self.meta.wire.as_ref().expect("a wire stream").exchange
    }
}

fn streams(frames: &[CaptureFrame]) -> Vec<StreamView> {
    let mut views: Vec<StreamView> = Vec::new();
    let mut index: BTreeMap<StreamKey, usize> = BTreeMap::new();
    for frame in frames {
        match &frame.event {
            FrameEvent::Open { source, metadata } => {
                index.insert(frame.stream.clone(), views.len());
                views.push(StreamView {
                    key: frame.stream.clone(),
                    source: *source,
                    meta: metadata.clone(),
                    bytes: Vec::new(),
                    end: None,
                });
            }
            FrameEvent::Bytes { data, .. } => {
                views[index[&frame.stream]].bytes.extend_from_slice(data)
            }
            FrameEvent::End { reason } => views[index[&frame.stream]].end = Some(*reason),
            _ => {}
        }
    }
    views
}

/// The streams whose captured bytes carry `marker`.
fn carrying<'a>(views: &'a [StreamView], marker: &[u8]) -> Vec<&'a StreamView> {
    views
        .iter()
        .filter(|v| contains(&v.bytes, marker))
        .collect()
}

/// `(producer, connection, event)` of every `Connection` control, in order.
fn controls(frames: &[CaptureFrame]) -> Vec<(String, String, ConnectionEvent)> {
    frames
        .iter()
        .filter_map(|frame| match &frame.event {
            FrameEvent::Connection { connection, event } => Some((
                frame.stream.producer.clone(),
                connection.clone(),
                event.clone(),
            )),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------
// The composed chain.
// ---------------------------------------------------------------------------------------

struct Options {
    /// Route the sender through a `Link`; `Some(false)` holds its connections until released.
    link: Option<bool>,
    /// Put the `Tee` between the emitter and the sender.
    tee: bool,
    /// Configure the sender with a secret the receiver never issued for the entry.
    wrong_secret: bool,
    /// The receiver's bound from accept to an admitted epoch.
    setup_timeout: Duration,
}

impl Options {
    /// The sender connects straight to the receiver, and the recording proxy gets the sender's sink
    /// itself.
    fn direct() -> Self {
        Options {
            link: None,
            tee: false,
            wrong_secret: false,
            setup_timeout: DEFAULT_SETUP_TIMEOUT,
        }
    }
}

struct Chain {
    listener: Option<IngestListener>,
    entry: IngestEntry,
    owner: Arc<Owner>,
    link: Option<Link>,
    sender: Arc<IngestSink>,
    tee: Option<Arc<Tee>>,
    proxy: Option<ProxyHandle>,
    proxy_addr: SocketAddr,
    ca: CertificateDer<'static>,
    _ca_dir: tempfile::TempDir,
}

impl Chain {
    async fn start(pki: &TestPki, options: Options) -> Self {
        // The daemon's receiver and one authorized entry with its owner's sink.
        let mut config = IngestConfig::new(monotonic_clock(DAEMON_CLOCK));
        config.setup_timeout = options.setup_timeout;
        let listener =
            IngestListener::bind_loopback("127.0.0.1:0".parse().unwrap(), config).unwrap();
        let secret = IngestSecret::generate();
        let owner = Arc::new(Owner::default());
        let entry = listener
            .authorize(EntryGrant {
                entry_id: ENTRY.to_string(),
                secret: secret.clone(),
                sink: owner.clone(),
                prior_producers: 0,
            })
            .unwrap();
        let link = match options.link {
            Some(open) => Some(Link::start(listener.local_addr(), open).await),
            None => None,
        };
        let endpoint = link
            .as_ref()
            .map_or(listener.local_addr(), |link| link.addr);
        let presented = if options.wrong_secret {
            IngestSecret::generate()
        } else {
            secret
        };

        // The sender, then the recording proxy with the same producer and the same sink.
        let sender = connect_sink(
            SinkConfig::new(
                endpoint,
                ENTRY.to_string(),
                SenderSecret::new(presented.expose().to_string()),
                PRODUCER.to_string(),
                SENDER_DEADLINE,
            )
            .with_reconnect_delays([RECONNECT; 4]),
        )
        .unwrap();
        let tee = options.tee.then(|| {
            Arc::new(Tee {
                inner: Arc::clone(&sender),
                log: Mutex::default(),
            })
        });
        let sink: Arc<dyn CaptureSink> = match &tee {
            Some(tee) => Arc::clone(tee) as Arc<dyn CaptureSink>,
            None => Arc::clone(&sender) as Arc<dyn CaptureSink>,
        };
        let ca_dir = tempfile::tempdir().unwrap();
        let policy = CapturePolicy {
            paths: [
                ("/v1/responses", "openai-responses"),
                ("/v1/realtime", "openai-realtime"),
            ]
            .into_iter()
            .map(|(path, hint)| CapturePath {
                path: path.to_string(),
                prefix: false,
                protocol_hint: Some(hint.to_string()),
            })
            .collect(),
        };
        let proxy = within(
            "proxy start",
            goalcup_cli::proxy::start(ProxyConfig {
                listener: std::net::TcpListener::bind("127.0.0.1:0").unwrap(),
                ca_dir: ca_dir.path().to_path_buf(),
                approved: vec![ApprovedHost {
                    host: "localhost".to_string(),
                    policy,
                }],
                auth_hosts: Vec::new(),
                credential: ProxyCredential::from_secret(PROXY_SECRET.to_string()).unwrap(),
                upstream_roots: UpstreamRoots::from_certificates(vec![pki.root()]).unwrap(),
                producer: PRODUCER.to_string(),
                sink,
                startup_deadline: Duration::from_secs(5),
                shutdown_deadline: Duration::from_secs(3),
                admission_timeout: Duration::from_secs(5),
                connect_attempt_timeout: Duration::from_secs(2),
                max_connections: 16,
            }),
        )
        .await
        .expect("the recording proxy starts once T12 has written its handshake");
        assert_eq!(proxy.producer(), PRODUCER);
        Self {
            listener: Some(listener),
            entry,
            owner,
            link,
            sender,
            tee,
            proxy_addr: proxy.local_addr(),
            ca: proxy.ca_certificate_der().clone(),
            proxy: Some(proxy),
            _ca_dir: ca_dir,
        }
    }

    fn listener(&self) -> &IngestListener {
        self.listener.as_ref().expect("listener already shut down")
    }

    fn link(&self) -> &Link {
        self.link.as_ref().expect("a chain with a link")
    }

    fn tee(&self) -> &Tee {
        self.tee.as_ref().expect("a chain with a tee")
    }

    async fn client(&self, upstream: &Upstream) -> ClientTls {
        let target = format!("localhost:{}", upstream.addr.port());
        intercepted_client(self.proxy_addr, &target, &self.ca).await
    }

    /// Waits until `n` intercepted connections have closed.
    async fn wait_closed(&self, n: u64) {
        let proxy = self.proxy.as_ref().expect("proxy already stopped");
        eventually("intercepted connections close", || {
            let h = proxy.health();
            h.closed_complete + h.closed_interrupted + h.closed_transport_error >= n
        })
        .await;
    }

    /// Waits until every frame the sender queued has been admitted by the receiver: nothing is in
    /// flight.
    async fn settled(&self) {
        eventually("T06 admits everything T12 queued", || {
            self.entry.snapshot().frames_admitted == self.sender.stats().frames_queued
        })
        .await;
    }

    async fn stop_proxy(&mut self) -> ProxyStopReport {
        let proxy = self.proxy.take().expect("proxy already stopped");
        within("proxy stop", proxy.stop()).await
    }

    /// Waits for exactly `n` more latched receiver outcomes.
    async fn outcomes(&self, n: usize) -> Vec<ProducerOutcome> {
        let mut got = Vec::new();
        within("T06 outcomes", async {
            while got.len() < n {
                self.entry.wait_outcome().await;
                got.extend(self.entry.take_outcomes());
            }
        })
        .await;
        assert_eq!(got.len(), n, "{got:?}");
        got
    }

    async fn shutdown_listener(&mut self) -> ShutdownReport {
        let listener = self.listener.take().expect("listener already shut down");
        within(
            "listener shutdown",
            listener.shutdown(Instant::now() + Duration::from_secs(2)),
        )
        .await
    }
}

fn loss(outcome: ProducerOutcome) -> ProducerLoss {
    match outcome {
        ProducerOutcome::Lost(loss) => loss,
        other => panic!("expected a loss, got {other:?}"),
    }
}

fn validated(outcome: ProducerOutcome) -> ValidatedClose {
    match outcome {
        ProducerOutcome::Closed(close) => close,
        other => panic!("expected a validated close, got {other:?}"),
    }
}

/// Drives request boundaries on one connection, each relayed byte for byte, until the emitter's
/// `Recovery` naming `previous` has been answered by the sender. Returns the index of the boundary
/// that carried it and the fresh producer. A fresh proposal answered `Pending` starts the sender's
/// one background attempt; the next boundary waits until its handshake is written, so it is
/// answered `Ready` rather than racing the attempt.
async fn exchange_until_recovered(
    chain: &Chain,
    client: &mut ClientTls,
    up: &mut UpstreamTls,
    tag: &str,
    previous: &str,
) -> (usize, String) {
    let tee = chain.tee();
    for i in 0..MAX_BOUNDARIES {
        let begins = tee.begins().len();
        let ready = chain.sender.stats().epochs_ready;
        exchange(client, up, format!("{tag}-{i}").as_bytes(), b"{}").await;
        let recovered = tee
            .recoveries()
            .into_iter()
            .find(|(_, named, _, _)| named == previous);
        if let Some((producer, _, _, answer)) = recovered {
            assert_eq!(
                answer,
                SubmitResult::Queued,
                "T12 refused the Recovery naming {previous}"
            );
            return (i, producer);
        }
        if tee.begins()[begins..]
            .iter()
            .any(|(_, answer)| *answer == EpochAdmission::Pending)
        {
            eventually("T12 writes the fresh epoch's handshake", || {
                chain.sender.stats().epochs_ready > ready
            })
            .await;
        }
    }
    panic!("no Recovery naming {previous} within {MAX_BOUNDARIES} request boundaries");
}

// ---------------------------------------------------------------------------------------
// (1) The clean path.
// ---------------------------------------------------------------------------------------

/// An HTTP exchange and a WebSocket exchange through the real proxy reach the real receiver over
/// real loopback TCP, in order and complete; the ordered stop and shutdown end in the receiver's
/// validated close with the exact final sequence.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn clean_http_and_websocket_capture_closes_validated_at_the_receiver() {
    let pki = TestPki::new("cp2 upstream CA");
    let mut upstream = Upstream::start(&pki).await;
    let mut chain = Chain::start(&pki, Options::direct()).await;

    let mut client = chain.client(&upstream).await;
    let mut up = upstream.accept().await;
    exchange(
        &mut client,
        &mut up,
        b"cp2-http-request",
        b"{\"cp2\":\"http-reply\"}",
    )
    .await;
    close_both(client, up).await;

    let mut client = chain.client(&upstream).await;
    let mut up = upstream.accept().await;
    relay_exact(&mut client, &mut up, &ws_upgrade_request("/v1/realtime")).await;
    relay_exact(&mut up, &mut client, &ws_101()).await;
    relay_exact(
        &mut client,
        &mut up,
        &ws_frame(0x1, b"cp2-ws-client", CLIENT_MASK),
    )
    .await;
    relay_exact(&mut up, &mut client, &ws_frame(0x1, b"cp2-ws-server", None)).await;
    relay_exact(&mut client, &mut up, &ws_frame(0x8, &[3, 232], CLIENT_MASK)).await;
    relay_exact(&mut up, &mut client, &ws_frame(0x8, &[3, 232], None)).await;
    close_both(client, up).await;
    chain.wait_closed(2).await;

    let report = chain.stop_proxy().await;
    assert!(
        report.is_clean_drain() && !report.capture_loss() && report.transport_failures() == 0,
        "{report:?}"
    );
    let sent = within("T12 shutdown", chain.sender.shutdown()).await;
    let outcomes = chain.outcomes(1).await;
    eprintln!("evidence clean path: {report:?}\n{sent:?}\n{outcomes:?}");

    // What the owner received: every frame, in sequence order, then exactly one close last.
    let items = chain.owner.items();
    let frames = chain.owner.frames();
    check_chain(&frames);
    let n = frames.len() as u64;
    let last = frames.last().expect("frames reached the owner").seq;
    assert_eq!(last, n - 1);
    assert!(frames.iter().all(|f| f.stream.producer == PRODUCER));
    assert_eq!(
        items.last(),
        Some(&Owned::Close {
            producer: PRODUCER.to_string(),
            final_seq: Some(last),
        })
    );
    assert_eq!(items.len() as u64, n + 1, "one close, after every frame");

    // The receiver: a validated close with the exact final sequence, on the daemon clock.
    let close = validated(outcomes.into_iter().next().unwrap());
    assert_eq!(
        (close.producer.as_str(), close.final_seq),
        (PRODUCER, Some(last))
    );
    assert_eq!(close.observed.clock_id, DAEMON_CLOCK);
    let snapshot = chain.entry.snapshot();
    assert_eq!(
        (
            snapshot.frames_admitted,
            snapshot.producers_consumed,
            snapshot.producers_unsettled,
            snapshot.open_streams,
            snapshot.admissions_refused,
        ),
        (n, 1, 0, 0, 0)
    );
    assert_eq!(chain.listener().stats().handshakes_refused, 0);
    assert_eq!(
        chain.owner.begins(),
        vec![(PRODUCER.to_string(), EpochAdmission::Ready)]
    );

    // The sender: the same close, and exactly the frames the receiver admitted, all written.
    assert_eq!(
        sent.outcome,
        SenderOutcome::Closed {
            producer: PRODUCER.to_string(),
            final_seq: Some(last),
        }
    );
    assert!(sent.is_clean(), "{sent:?}");
    let stats = &sent.stats;
    assert_eq!(
        (
            stats.frames_queued,
            stats.items_written,
            stats.connection_attempts,
            stats.handshakes_sent,
            stats.losses,
            stats.failures,
            stats.discarded_bytes,
        ),
        (n, n + 1, 1, 1, 0, 0, 0)
    );

    // The content arrived as relayed: both connections' lifecycles, and every body once.
    let controls = controls(&frames);
    let events: Vec<ConnectionEvent> = controls.iter().map(|c| c.2.clone()).collect();
    let open = ConnectionEvent::Open {
        alpn: Some("http/1.1".into()),
        already_open: false,
    };
    let complete = ConnectionEvent::Close {
        outcome: ConnectionOutcome::Complete,
    };
    assert_eq!(
        events,
        vec![
            open.clone(),
            complete.clone(),
            open,
            ConnectionEvent::UpgradedWebSocket,
            complete
        ]
    );
    assert_ne!(controls[0].1, controls[2].1, "two physical connections");
    let views = streams(&frames);
    assert!(views.iter().all(|v| v.end == Some(EndReason::Complete)));
    for (marker, source) in [
        (&b"cp2-http-request"[..], CaptureSource::WireRequest),
        (b"{\"cp2\":\"http-reply\"}", CaptureSource::WireResponse),
        (b"cp2-ws-client", CaptureSource::WsClient),
        (b"cp2-ws-server", CaptureSource::WsServer),
    ] {
        let found = carrying(&views, marker);
        assert_eq!(found.len(), 1, "{:?}", String::from_utf8_lossy(marker));
        assert_eq!(
            (found[0].source, found[0].bytes.as_slice()),
            (source, marker)
        );
    }
    assert_eq!(
        views
            .iter()
            .filter(|v| matches!(v.source, CaptureSource::WsClient | CaptureSource::WsServer))
            .count(),
        2
    );

    assert_eq!(
        chain.shutdown_listener().await,
        ShutdownReport {
            producers_cancelled: 0,
            tasks_aborted: 0
        }
    );
}

// ---------------------------------------------------------------------------------------
// (2) Receiver loss mid-run.
// ---------------------------------------------------------------------------------------

/// The daemon link dies mid-run. The receiver records the loss at the item boundary, the sender
/// latches `IngestLost`, the relay keeps forwarding every byte unchanged, and at a later request
/// boundary the emitter recovers into a fresh epoch whose `Recovery` the receiver accepts.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn receiver_loss_mid_run_recovers_into_a_fresh_epoch_the_receiver_accepts() {
    let pki = TestPki::new("cp2 upstream CA");
    let mut upstream = Upstream::start(&pki).await;
    let mut chain = Chain::start(
        &pki,
        Options {
            link: Some(true),
            tee: true,
            ..Options::direct()
        },
    )
    .await;
    let mut client = chain.client(&upstream).await;
    let mut up = upstream.accept().await;
    exchange(&mut client, &mut up, b"cp2-before-loss", b"{}").await;
    chain.settled().await;
    let admitted = chain.entry.snapshot().frames_admitted;

    // Nothing in flight: the link dies at an item boundary.
    chain.link().cut(0);
    let lost = loss(chain.outcomes(1).await.remove(0));
    assert_eq!(
        (
            lost.producer.as_str(),
            lost.cause,
            lost.reason,
            lost.admitted_through
        ),
        (
            PRODUCER,
            LossCause::Disconnected,
            GapReason::IngestLost,
            Some(admitted - 1)
        )
    );
    assert_eq!(lost.observed.clock_id, DAEMON_CLOCK);
    eventually("T12 latches the lost epoch", || {
        chain.sender.stats().losses == 1
    })
    .await;
    let stats = chain.sender.stats();
    assert_eq!((stats.queue_full, stats.failures), (0, 1));

    // The relay is unaffected; capture comes back under a fresh producer.
    let (at, fresh) =
        exchange_until_recovered(&chain, &mut client, &mut up, "cp2-loss", PRODUCER).await;
    exchange(&mut client, &mut up, b"cp2-after-recovery", b"{}").await;
    close_both(client, up).await;
    chain.wait_closed(1).await;
    let report = chain.stop_proxy().await;
    let sent = within("T12 shutdown", chain.sender.shutdown()).await;
    let closed = validated(chain.outcomes(1).await.remove(0));
    eprintln!(
        "evidence receiver loss: recovered at boundary {at}\n{report:?}\n{sent:?}\n{closed:?}"
    );

    // The owner holds P's frames, then the fresh epoch's, and nothing else.
    let frames = chain.owner.frames();
    check_chain(&frames);
    let (p, r): (Vec<&CaptureFrame>, Vec<&CaptureFrame>) =
        frames.iter().partition(|f| f.stream.producer == PRODUCER);
    assert_eq!(p.len() as u64, admitted);
    assert!(r.iter().all(|f| f.stream.producer == fresh));
    assert!(
        frames[..p.len()]
            .iter()
            .all(|f| f.stream.producer == PRODUCER),
        "P's frames precede the fresh epoch's"
    );
    let connection = controls(&frames)[0].1.clone();
    assert_eq!(
        r[0].event,
        FrameEvent::Recovery {
            previous_producer: PRODUCER.to_string(),
            reason: GapReason::IngestLost,
        }
    );
    assert_eq!(
        r[1].event,
        FrameEvent::Connection {
            connection,
            event: ConnectionEvent::Open {
                alpn: Some("http/1.1".into()),
                already_open: true,
            },
        },
        "the surviving connection is re-observed before any payload"
    );
    // Equal, frame for frame, to what the sender accepted for each epoch: nothing dropped, added or
    // reordered on either side of the loss.
    assert_eq!(
        p.into_iter().cloned().collect::<Vec<_>>(),
        chain.tee().queued(PRODUCER)
    );
    assert_eq!(
        r.into_iter().cloned().collect::<Vec<_>>(),
        chain.tee().queued(&fresh)
    );

    // Nothing observed during the loss reached the receiver; the recovering boundary did, as
    // exchange 2 + at on the same connection.
    let views = streams(&frames);
    assert_eq!(
        carrying(&views, b"cp2-before-loss")[0].key.producer,
        PRODUCER
    );
    for i in 0..at {
        let marker = format!("cp2-loss-{i}");
        assert!(carrying(&views, marker.as_bytes()).is_empty(), "{marker}");
    }
    let recovered = carrying(&views, format!("cp2-loss-{at}").as_bytes());
    assert_eq!(recovered.len(), 1);
    assert_eq!(
        (recovered[0].key.producer.as_str(), recovered[0].exchange()),
        (fresh.as_str(), 2 + at as u64)
    );
    assert_eq!(
        carrying(&views, b"cp2-after-recovery")[0].key.producer,
        fresh
    );

    // The receiver closed the fresh epoch with its exact final sequence; the sender agrees but
    // reports the earlier loss; the recording proxy saw the refusal and two admitted epochs.
    let last = frames.last().unwrap().seq;
    assert_eq!(
        (closed.producer.as_str(), closed.final_seq),
        (fresh.as_str(), Some(last))
    );
    assert_eq!(
        sent.outcome,
        SenderOutcome::Closed {
            producer: fresh.clone(),
            final_seq: Some(last),
        }
    );
    assert!(!sent.is_clean(), "a launch with a loss is never clean");
    assert_eq!(
        (
            sent.stats.losses,
            sent.stats.failures,
            sent.stats.epochs_ready,
            sent.stats.handshakes_sent
        ),
        (1, 1, 2, 2)
    );
    assert!(
        report.is_clean_drain() && report.capture_loss(),
        "{report:?}"
    );
    assert_eq!(report.transport_failures(), 0);
    assert_eq!(report.health.producers_admitted, 2);
    assert_eq!(chain.tee().closes(), vec![SubmitResult::Queued]);
    let snapshot = chain.entry.snapshot();
    assert_eq!(
        (
            snapshot.producers_consumed,
            snapshot.producers_unsettled,
            snapshot.open_streams,
            snapshot.admissions_refused
        ),
        (2, 0, 0, 0)
    );
    assert_eq!(chain.link().connections(), 2);
    chain.shutdown_listener().await;
}

// ---------------------------------------------------------------------------------------
// (3) A recovery lost in flight, over the IPC path.
// ---------------------------------------------------------------------------------------

/// P is lost with a stream open (the owner's queue refuses its frame). Q's handshake is written, so
/// the sender answers `Ready` and queues Q's `Recovery` naming P, but the receiver holds Q in
/// admission (owner `Pending`) past its setup deadline: Q is lost in flight and the chain holds
/// nothing of it. R's `Recovery` then names the unrecorded Q; the receiver accepts it and releases
/// the orphaned wire lineage, so P's stream stops counting.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recovery_lost_in_flight_is_named_by_the_next_and_the_receiver_releases_the_lineage() {
    let pki = TestPki::new("cp2 upstream CA");
    let mut upstream = Upstream::start(&pki).await;
    let mut chain = Chain::start(
        &pki,
        Options {
            tee: true,
            setup_timeout: Duration::from_secs(2),
            ..Options::direct()
        },
    )
    .await;
    let mut client = chain.client(&upstream).await;
    let mut up = upstream.accept().await;
    exchange(&mut client, &mut up, b"cp2-p-recorded", b"{}").await;

    // A second exchange pauses mid-response: P's response stream is open at the receiver.
    relay_exact(&mut client, &mut up, &post("/v1/responses", b"cp2-p-open")).await;
    let (first, second) = (
        &b"{\"cp2\":\"first-half\","[..],
        &b"\"cp2\":\"second-half\"}"[..],
    );
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        first.len() + second.len()
    );
    relay_exact(&mut up, &mut client, &[head.as_bytes(), first].concat()).await;
    eventually("the first half reaches the owner", || {
        streams(&chain.owner.frames())
            .iter()
            .any(|v| v.source == CaptureSource::WireResponse && v.bytes == first)
    })
    .await;
    chain.settled().await;
    let admitted = chain.entry.snapshot().frames_admitted;
    assert_eq!(chain.entry.snapshot().open_streams, 1);

    // The owner's queue is full: it refuses P's next frame, and answers every new epoch
    // `Pending` while the pressure lasts.
    chain.owner.refuse_next(GapReason::QueueFull);
    chain.owner.hold(true);
    relay_exact(&mut up, &mut client, second).await;
    let lost = loss(chain.outcomes(1).await.remove(0));
    assert_eq!(
        (
            lost.producer.as_str(),
            lost.cause,
            lost.reason,
            lost.admitted_through
        ),
        (
            PRODUCER,
            LossCause::SinkRefused,
            GapReason::QueueFull,
            Some(admitted - 1)
        )
    );
    assert_eq!(
        chain.entry.snapshot().open_streams,
        1,
        "a lost producer's open stream stays counted"
    );
    eventually("T12 learns P's epoch is gone", || {
        chain.sender.stats().losses == 1
    })
    .await;

    // Q: the sender answers Ready and queues Q's Recovery naming P; the receiver never admits Q.
    let (q_at, q) = exchange_until_recovered(&chain, &mut client, &mut up, "cp2-q", PRODUCER).await;
    eventually("T06 refuses Q at its setup deadline", || {
        chain.entry.snapshot().admissions_refused == 1
    })
    .await;
    eventually("T12 learns Q's epoch is gone", || {
        chain.sender.stats().losses == 2
    })
    .await;
    assert!(
        chain.entry.take_outcomes().is_empty(),
        "Q was never admitted, so nothing is latched for it"
    );
    let q_begins: Vec<EpochAdmission> = chain
        .owner
        .begins()
        .into_iter()
        .filter(|(producer, _)| *producer == q)
        .map(|(_, answer)| answer)
        .collect();
    assert!(
        !q_begins.is_empty() && q_begins.iter().all(|a| *a == EpochAdmission::Pending),
        "{q_begins:?}"
    );
    let q_queued = chain.tee().queued(&q);
    assert_eq!(
        q_queued[0].event,
        FrameEvent::Recovery {
            previous_producer: PRODUCER.to_string(),
            reason: GapReason::IngestLost,
        },
        "Q's Recovery was queued by T12 (volatile acceptance), never persisted"
    );

    // The pressure clears. R's Recovery names Q, which the chain never recorded.
    chain.owner.hold(false);
    let (r_at, r) = exchange_until_recovered(&chain, &mut client, &mut up, "cp2-r", &q).await;
    exchange(&mut client, &mut up, b"cp2-r-after", b"{}").await;
    close_both(client, up).await;
    chain.wait_closed(1).await;
    let report = chain.stop_proxy().await;
    let sent = within("T12 shutdown", chain.sender.shutdown()).await;
    let closed = validated(chain.outcomes(1).await.remove(0));
    eprintln!(
        "evidence CP-A4 A: Q at boundary {q_at}, R at boundary {r_at}\n{report:?}\n{sent:?}\n{closed:?}"
    );

    // The receiver accepted R's Recovery naming the unrecorded Q and released the orphaned P: no
    // stream is counted any more (P's would still count if only Q had been superseded).
    let snapshot = chain.entry.snapshot();
    assert_eq!(snapshot.open_streams, 0, "P's orphaned stream was released");
    assert_eq!(
        (
            snapshot.producers_consumed,
            snapshot.producers_unsettled,
            snapshot.admissions_refused
        ),
        (3, 0, 1)
    );
    let frames = chain.owner.frames();
    check_chain(&frames);
    assert!(
        !frames.iter().any(|f| f.stream.producer == q),
        "nothing of Q reached the owner"
    );
    let (p_frames, r_frames): (Vec<&CaptureFrame>, Vec<&CaptureFrame>) =
        frames.iter().partition(|f| f.stream.producer == PRODUCER);
    assert_eq!(p_frames.len() as u64, admitted);
    // The sender accepted more of P than the receiver admitted: the owner refused exactly the
    // second half.
    let p_queued = chain.tee().queued(PRODUCER);
    assert_eq!(
        p_frames.iter().map(|f| (*f).clone()).collect::<Vec<_>>(),
        p_queued[..p_frames.len()]
    );
    assert!(matches!(
        &p_queued[p_frames.len()].event,
        FrameEvent::Bytes { data, .. } if data == second
    ));
    assert!(r_frames.iter().all(|f| f.stream.producer == r));
    assert_eq!(
        r_frames[0].event,
        FrameEvent::Recovery {
            previous_producer: q.clone(),
            reason: GapReason::IngestLost,
        }
    );
    assert!(matches!(
        &r_frames[1].event,
        FrameEvent::Connection {
            event: ConnectionEvent::Open {
                already_open: true,
                ..
            },
            ..
        }
    ));
    assert_eq!(
        r_frames.into_iter().cloned().collect::<Vec<_>>(),
        chain.tee().queued(&r)
    );
    let recoveries: Vec<(String, String, SubmitResult)> = chain
        .tee()
        .recoveries()
        .into_iter()
        .map(|(producer, previous, _, answer)| (producer, previous, answer))
        .collect();
    assert_eq!(
        recoveries,
        vec![
            (q.clone(), PRODUCER.to_string(), SubmitResult::Queued),
            (r.clone(), q.clone(), SubmitResult::Queued),
        ]
    );
    let views = streams(&frames);
    let p_open: Vec<&StreamView> = views
        .iter()
        .filter(|v| v.key.producer == PRODUCER && v.end.is_none())
        .collect();
    assert_eq!(
        p_open.len(),
        1,
        "P's response stream never ended in the chain"
    );
    assert_eq!(p_open[0].bytes, first);
    assert!(carrying(&views, format!("cp2-q-{q_at}").as_bytes()).is_empty());
    assert_eq!(
        carrying(&views, format!("cp2-r-{r_at}").as_bytes())[0]
            .key
            .producer,
        r
    );
    assert_eq!(carrying(&views, b"cp2-r-after")[0].key.producer, r);

    let last = frames.last().unwrap().seq;
    assert_eq!(
        (closed.producer.as_str(), closed.final_seq),
        (r.as_str(), Some(last))
    );
    assert_eq!(
        sent.outcome,
        SenderOutcome::Closed {
            producer: r.clone(),
            final_seq: Some(last),
        }
    );
    assert_eq!(
        (
            sent.stats.losses,
            sent.stats.failures,
            sent.stats.epochs_ready
        ),
        (2, 2, 3)
    );
    assert!(
        report.is_clean_drain() && report.capture_loss(),
        "{report:?}"
    );
    assert_eq!(report.health.producers_admitted, 3);
    assert_eq!(report.transport_failures(), 0);
    chain.shutdown_listener().await;
}

// ---------------------------------------------------------------------------------------
// (4) A wrong secret.
// ---------------------------------------------------------------------------------------

/// The sender presents a secret the receiver never issued. The receiver refuses the handshake
/// before consulting the owner: no ID is consumed and no frame admitted. The recording proxy, which
/// started on the sender's written handshake (the link held it back from the receiver until then),
/// relays every byte unchanged.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wrong_secret_is_refused_before_admission_and_the_relay_is_unaffected() {
    let pki = TestPki::new("cp2 upstream CA");
    let mut upstream = Upstream::start(&pki).await;
    let mut chain = Chain::start(
        &pki,
        Options {
            link: Some(false),
            wrong_secret: true,
            ..Options::direct()
        },
    )
    .await;

    // The handshake now reaches the receiver, which refuses the token and closes the socket.
    chain.link().release();
    eventually("T06 refuses the handshake", || {
        chain.listener().stats().handshakes_refused >= 1
    })
    .await;
    eventually("T12 learns its epoch is gone", || {
        chain.sender.stats().losses == 1
    })
    .await;

    // The connection's first control is refused, so the first request boundary proposes a fresh
    // epoch; the sender connects for it with the same secret and the receiver refuses that one too.
    let mut client = chain.client(&upstream).await;
    let mut up = upstream.accept().await;
    for i in 0..4 {
        let body = format!("cp2-refused-{i}");
        exchange(&mut client, &mut up, body.as_bytes(), b"{\"refused\":true}").await;
        if i == 1 {
            eventually("T06 refuses the fresh epoch's handshake too", || {
                chain.listener().stats().handshakes_refused >= 2
            })
            .await;
        }
    }
    close_both(client, up).await;
    chain.wait_closed(1).await;
    let report = chain.stop_proxy().await;
    let sent = within("T12 shutdown", chain.sender.shutdown()).await;
    // Every connection the sender ever made reached the receiver through the link and was refused
    // there.
    eventually("every connection T12 made is refused", || {
        let stats = chain.listener().stats();
        stats.connections_accepted == chain.link().connections() as u64
            && stats.handshakes_refused == stats.connections_accepted
    })
    .await;
    eprintln!(
        "evidence wrong secret: {report:?}\n{sent:?}\n{:?}",
        chain.listener().stats()
    );

    // Nothing was admitted: the owner was never consulted and no producer ID was consumed.
    assert!(chain.owner.begins().is_empty());
    assert!(chain.owner.items().is_empty());
    let snapshot = chain.entry.snapshot();
    assert_eq!(
        (
            snapshot.producers_consumed,
            snapshot.frames_admitted,
            snapshot.admissions_refused,
            snapshot.outcomes_pending
        ),
        (0, 0, 0, 0)
    );
    assert!(chain.entry.take_outcomes().is_empty());
    let stats = chain.listener().stats();
    assert!(stats.handshakes_refused >= 2, "{stats:?}");
    assert_eq!(stats.handshakes_timed_out, 0);

    // The relay was unaffected; the capture loss is visible on both sender-side reports.
    assert!(report.drained && report.unterminated == 0, "{report:?}");
    assert_eq!(report.transport_failures(), 0);
    assert!(report.capture_loss() && report.health.sink_refusals > 0);
    assert!(!sent.is_clean(), "{sent:?}");
    assert!(sent.stats.losses >= 1 && sent.stats.handshakes_sent >= 2);
    chain.shutdown_listener().await;
}
