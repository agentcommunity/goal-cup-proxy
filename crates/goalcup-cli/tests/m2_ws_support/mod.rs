//! Test support for `m2_websocket_relay`, owned by that test alone.
//!
//! - [`Frame`]: a deterministic RFC 6455 frame encoder. It writes bytes; it never parses
//!   them, so it shares no code or assumption with the relay's parser. It is checked against
//!   the RFC's own byte examples (`tests/fixtures/m2-websocket/rfc-examples.json`).
//! - [`Deflater`]: permessage-deflate message compression (RFC 7692 §7.2.1) over `flate2`'s
//!   compressor, with or without context takeover.
//! - [`RecordingSink`]: a [`CaptureSink`] that logs every call in order and can be told to
//!   refuse frames or answer epoch admission a given way; [`check_c2`] and [`streams`] read
//!   that log independently of the emitter and the relay.
//! - [`Relay`]: `relay_websocket` between two in-memory duplex peers the test plays, with an
//!   explicit deadline on every wait.

#![allow(dead_code)]

use flate2::{Compress, Compression, FlushCompress};
use goalcup_cli::proxy::api::{CaptureEmitter, CapturePolicy, RelayContext, RelayPair, Upgrade};
use goalcup_cli::proxy::websocket::relay_websocket;
use goalcup_protocol::capture::{
    CaptureFrame, CaptureMeta, CaptureSink, CaptureSource, CaptureTime, ConnectionEvent, EndReason,
    EpochAdmission, FrameEvent, GapReason, MAX_SEGMENT_BYTES, StreamKey, SubmitResult,
    WireIdentity,
};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};
use tokio::task::JoinHandle;

pub const INITIAL: &str = "0123456789abcdef0123456789abcdef";
pub const CONN_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const CONN_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
pub const EXCHANGE: u64 = 7;
/// Every wait in these tests ends by this deadline or fails.
pub const DEADLINE: Duration = Duration::from_secs(30);
const DUPLEX_BYTES: usize = 64 * 1024;

pub const OP_CONT: u8 = 0x0;
pub const OP_TEXT: u8 = 0x1;
pub const OP_BINARY: u8 = 0x2;
pub const OP_CLOSE: u8 = 0x8;
pub const OP_PING: u8 = 0x9;
pub const OP_PONG: u8 = 0xa;
pub const RSV1: u8 = 0x40;
pub const RSV2: u8 = 0x20;
pub const RSV3: u8 = 0x10;

// ---------------------------------------------------------------------------------------
// Frame encoder
// ---------------------------------------------------------------------------------------

/// How a frame's payload length is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Len {
    /// The shortest form RFC 6455 §5.2 requires.
    Minimal,
    /// The 16-bit form, whatever the length.
    Force16,
    /// The 64-bit form, whatever the length.
    Force64,
    /// The 64-bit form carrying this value instead of the payload's length (malformed
    /// headers).
    Declared(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub fin: bool,
    pub rsv: u8,
    pub opcode: u8,
    pub mask: Option<[u8; 4]>,
    pub len: Len,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(opcode: u8, payload: impl Into<Vec<u8>>) -> Self {
        Frame {
            fin: true,
            rsv: 0,
            opcode,
            mask: None,
            len: Len::Minimal,
            payload: payload.into(),
        }
    }

    pub fn text(payload: impl Into<Vec<u8>>) -> Self {
        Frame::new(OP_TEXT, payload)
    }

    pub fn binary(payload: impl Into<Vec<u8>>) -> Self {
        Frame::new(OP_BINARY, payload)
    }

    pub fn cont(payload: impl Into<Vec<u8>>) -> Self {
        Frame::new(OP_CONT, payload)
    }

    pub fn ping(payload: impl Into<Vec<u8>>) -> Self {
        Frame::new(OP_PING, payload)
    }

    pub fn pong(payload: impl Into<Vec<u8>>) -> Self {
        Frame::new(OP_PONG, payload)
    }

    pub fn close(code: u16) -> Self {
        Frame::new(OP_CLOSE, code.to_be_bytes().to_vec())
    }

    pub fn fin(mut self, fin: bool) -> Self {
        self.fin = fin;
        self
    }

    pub fn rsv(mut self, bits: u8) -> Self {
        self.rsv = bits;
        self
    }

    pub fn masked(mut self, key: [u8; 4]) -> Self {
        self.mask = Some(key);
        self
    }

    pub fn len(mut self, len: Len) -> Self {
        self.len = len;
        self
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![if self.fin { 0x80 } else { 0 } | self.rsv | self.opcode];
        let mask_bit = if self.mask.is_some() { 0x80 } else { 0 };
        let n = self.payload.len() as u64;
        match self.len {
            Len::Minimal if n < 126 => out.push(mask_bit | n as u8),
            Len::Minimal if n <= 0xffff => {
                out.push(mask_bit | 126);
                out.extend_from_slice(&(n as u16).to_be_bytes());
            }
            Len::Force16 => {
                out.push(mask_bit | 126);
                out.extend_from_slice(&(n as u16).to_be_bytes());
            }
            Len::Minimal | Len::Force64 => {
                out.push(mask_bit | 127);
                out.extend_from_slice(&n.to_be_bytes());
            }
            Len::Declared(declared) => {
                out.push(mask_bit | 127);
                out.extend_from_slice(&declared.to_be_bytes());
            }
        }
        match self.mask {
            Some(key) => {
                out.extend_from_slice(&key);
                out.extend(
                    self.payload
                        .iter()
                        .enumerate()
                        .map(|(i, byte)| byte ^ key[i % 4]),
                );
            }
            None => out.extend_from_slice(&self.payload),
        }
        out
    }
}

/// A deterministic, varied masking key: frame masks differ so a phase error shows.
pub fn key(n: u32) -> [u8; 4] {
    let x = n.wrapping_mul(0x9e37_79b9).wrapping_add(0x7f4a_7c15);
    x.to_be_bytes()
}

pub fn hex(text: &str) -> Vec<u8> {
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(text.len().is_multiple_of(2), "odd hex length");
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).expect("hex digit"))
        .collect()
}

// ---------------------------------------------------------------------------------------
// permessage-deflate compression
// ---------------------------------------------------------------------------------------

/// RFC 7692 §7.2.1: raw DEFLATE, sync-flushed, with the trailing `00 00 ff ff` removed.
pub struct Deflater {
    compress: Compress,
    takeover: bool,
}

impl Deflater {
    pub fn new(takeover: bool) -> Self {
        Deflater {
            compress: Compress::new(Compression::default(), false),
            takeover,
        }
    }

    pub fn fast(takeover: bool) -> Self {
        Deflater {
            compress: Compress::new(Compression::fast(), false),
            takeover,
        }
    }

    /// Compress one whole message.
    pub fn message(&mut self, plain: &[u8]) -> Vec<u8> {
        if !self.takeover {
            self.compress.reset();
        }
        let mut out = Vec::with_capacity(plain.len() / 2 + 64);
        self.feed(plain, &mut out, FlushCompress::Sync);
        assert!(
            out.ends_with(&[0x00, 0x00, 0xff, 0xff]),
            "a sync flush ends with an empty stored block"
        );
        out.truncate(out.len() - 4);
        out
    }

    /// Compress part of a message without flushing (for building huge messages piecewise).
    pub fn partial(&mut self, plain: &[u8], out: &mut Vec<u8>) {
        self.feed(plain, out, FlushCompress::None);
    }

    /// End a message built with [`Deflater::partial`].
    pub fn finish(&mut self, out: &mut Vec<u8>) {
        self.feed(&[], out, FlushCompress::Sync);
        assert!(out.ends_with(&[0x00, 0x00, 0xff, 0xff]));
        out.truncate(out.len() - 4);
    }

    fn feed(&mut self, plain: &[u8], out: &mut Vec<u8>, flush: FlushCompress) {
        let start = self.compress.total_in();
        loop {
            if out.capacity() - out.len() < 1024 {
                out.reserve(out.capacity().max(4096));
            }
            let consumed = (self.compress.total_in() - start) as usize;
            self.compress
                .compress_vec(&plain[consumed..], out, flush)
                .expect("deflate");
            let consumed = (self.compress.total_in() - start) as usize;
            if consumed == plain.len() && out.len() < out.capacity() {
                return;
            }
        }
    }
}

/// Inflate one message with a fresh raw inflater (no history), for fixture sanity checks.
pub fn inflate_fresh(compressed: &[u8]) -> Result<Vec<u8>, flate2::DecompressError> {
    inflate_in_order(&[compressed]).pop().expect("one message")
}

/// Inflate messages in order through one inflater, as a takeover receiver would; each
/// message's own output. For fixture sanity checks.
pub fn inflate_in_order(messages: &[&[u8]]) -> Vec<Result<Vec<u8>, flate2::DecompressError>> {
    let mut inflater = flate2::Decompress::new(false);
    let mut outputs = Vec::new();
    for message in messages {
        let mut input = message.to_vec();
        input.extend_from_slice(&[0x00, 0x00, 0xff, 0xff]);
        let start = inflater.total_in();
        let mut out = Vec::with_capacity(input.len() * 8 + 1024);
        let result = loop {
            let consumed = (inflater.total_in() - start) as usize;
            if let Err(error) =
                inflater.decompress_vec(&input[consumed..], &mut out, flate2::FlushDecompress::Sync)
            {
                break Err(error);
            }
            let consumed = (inflater.total_in() - start) as usize;
            if consumed == input.len() && out.len() < out.capacity() {
                break Ok(out);
            }
            out.reserve(out.capacity());
        };
        outputs.push(result);
    }
    outputs
}

// ---------------------------------------------------------------------------------------
// Sinks and log readers
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Logged {
    Frame(Box<CaptureFrame>),
    Begin {
        producer: String,
        answer: EpochAdmission,
    },
    Close {
        producer: String,
        final_seq: Option<u64>,
    },
}

/// Logs every call in order. `refuse` makes `try_submit` answer `Disabled(reason)` without
/// logging; `admission` is what `try_begin_epoch` answers.
pub struct RecordingSink {
    log: Mutex<Vec<Logged>>,
    refuse: Mutex<Option<GapReason>>,
    admission: Mutex<EpochAdmission>,
}

impl RecordingSink {
    pub fn new() -> Arc<Self> {
        Arc::new(RecordingSink {
            log: Mutex::new(Vec::new()),
            refuse: Mutex::new(None),
            admission: Mutex::new(EpochAdmission::Ready),
        })
    }

    pub fn refusing(reason: GapReason, admission: EpochAdmission) -> Arc<Self> {
        let sink = RecordingSink::new();
        sink.refuse(Some(reason));
        sink.admit(admission);
        sink
    }

    pub fn refuse(&self, reason: Option<GapReason>) {
        *self.refuse.lock().unwrap() = reason;
    }

    pub fn admit(&self, answer: EpochAdmission) {
        *self.admission.lock().unwrap() = answer;
    }

    pub fn log(&self) -> Vec<Logged> {
        self.log.lock().unwrap().clone()
    }

    pub fn frames(&self) -> Vec<CaptureFrame> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter_map(|logged| match logged {
                Logged::Frame(frame) => Some((**frame).clone()),
                _ => None,
            })
            .collect()
    }

    pub fn begins(&self) -> Vec<EpochAdmission> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter_map(|logged| match logged {
                Logged::Begin { answer, .. } => Some(*answer),
                _ => None,
            })
            .collect()
    }
}

impl CaptureSink for RecordingSink {
    fn try_submit(&self, frame: CaptureFrame) -> SubmitResult {
        if let Some(reason) = *self.refuse.lock().unwrap() {
            return SubmitResult::Disabled(reason);
        }
        self.log
            .lock()
            .unwrap()
            .push(Logged::Frame(Box::new(frame)));
        SubmitResult::Queued
    }

    fn try_begin_epoch(&self, producer: &str) -> EpochAdmission {
        let answer = *self.admission.lock().unwrap();
        self.log.lock().unwrap().push(Logged::Begin {
            producer: producer.to_string(),
            answer,
        });
        answer
    }

    fn try_close_epoch(&self, producer: &str, final_seq: Option<u64>) -> SubmitResult {
        self.log.lock().unwrap().push(Logged::Close {
            producer: producer.to_string(),
            final_seq,
        });
        SubmitResult::Queued
    }
}

/// The producer rules, checked over a frame log without the emitter's help: per
/// producer, `seq` from zero without gaps, its own clock, valid frames, streams opened in
/// increasing order, payload only on open streams, `End` closing, `Recovery` first.
pub fn check_c2(frames: &[CaptureFrame]) -> Vec<String> {
    #[derive(Default)]
    struct View {
        next_seq: u64,
        high_water: u64,
        open: BTreeSet<u64>,
    }
    let mut views: BTreeMap<String, View> = BTreeMap::new();
    let mut problems = Vec::new();
    for frame in frames {
        let producer = &frame.stream.producer;
        let view = views.entry(producer.clone()).or_default();
        if frame.seq != view.next_seq {
            problems.push(format!(
                "seq {} where {} was next: {frame:?}",
                frame.seq, view.next_seq
            ));
        }
        view.next_seq = frame.seq + 1;
        if let Err(error) = frame.validate() {
            problems.push(format!("invalid frame ({error}): {frame:?}"));
        }
        if frame.observed.clock_id != *producer {
            problems.push(format!("frame not on its producer's clock: {frame:?}"));
        }
        let stream = frame.stream.stream;
        match &frame.event {
            FrameEvent::Recovery { .. } => {
                if frame.seq != 0 || producer == INITIAL {
                    problems.push(format!("Recovery out of place: {frame:?}"));
                }
            }
            FrameEvent::Connection { .. } | FrameEvent::Loss { .. } => {}
            FrameEvent::Open { .. } => {
                if stream <= view.high_water {
                    problems.push(format!("stream reused or out of order: {frame:?}"));
                }
                view.high_water = stream;
                view.open.insert(stream);
            }
            FrameEvent::Bytes { .. } | FrameEvent::Gap { .. } => {
                if !view.open.contains(&stream) {
                    problems.push(format!("payload on a stream that is not open: {frame:?}"));
                }
            }
            FrameEvent::End { .. } => {
                if !view.open.remove(&stream) {
                    problems.push(format!("End on a stream that is not open: {frame:?}"));
                }
            }
        }
    }
    problems
}

/// One payload stream as the log shows it.
#[derive(Debug, Clone)]
pub struct Captured {
    pub key: StreamKey,
    pub source: CaptureSource,
    pub meta: CaptureMeta,
    pub data: Vec<u8>,
    /// Every `Bytes` offset equalled the bytes already seen.
    pub contiguous: bool,
    pub bytes_frames: usize,
    pub largest_bytes_frame: usize,
    pub gaps: Vec<GapReason>,
    pub end: Option<EndReason>,
}

impl Captured {
    pub fn index(&self) -> u64 {
        self.meta
            .wire
            .as_ref()
            .and_then(|wire| wire.message_index)
            .expect("a websocket stream has a message index")
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.data).into_owned()
    }
}

/// Every payload stream, in `Open` order.
pub fn streams(frames: &[CaptureFrame]) -> Vec<Captured> {
    let mut out: Vec<Captured> = Vec::new();
    let mut at: BTreeMap<StreamKey, usize> = BTreeMap::new();
    for frame in frames {
        match &frame.event {
            FrameEvent::Open { source, metadata } => {
                at.insert(frame.stream.clone(), out.len());
                out.push(Captured {
                    key: frame.stream.clone(),
                    source: *source,
                    meta: metadata.clone(),
                    data: Vec::new(),
                    contiguous: true,
                    bytes_frames: 0,
                    largest_bytes_frame: 0,
                    gaps: Vec::new(),
                    end: None,
                });
            }
            FrameEvent::Bytes { offset, data } => {
                let stream = &mut out[at[&frame.stream]];
                if *offset != stream.data.len() as u64 {
                    stream.contiguous = false;
                }
                stream.data.extend_from_slice(data);
                stream.bytes_frames += 1;
                stream.largest_bytes_frame = stream.largest_bytes_frame.max(data.len());
            }
            FrameEvent::Gap { reason, .. } => out[at[&frame.stream]].gaps.push(*reason),
            FrameEvent::End { reason } => out[at[&frame.stream]].end = Some(*reason),
            FrameEvent::Connection { .. }
            | FrameEvent::Recovery { .. }
            | FrameEvent::Loss { .. } => {}
        }
    }
    out
}

pub fn assert_c2(sink: &RecordingSink) {
    let problems = check_c2(&sink.frames());
    assert!(problems.is_empty(), "C2 violations: {problems:#?}");
}

// ---------------------------------------------------------------------------------------
// Relay harness
// ---------------------------------------------------------------------------------------

pub fn time() -> CaptureTime {
    CaptureTime {
        wall: "2026-09-27T12:00:00Z".to_string(),
        mono_ns: 1,
        clock_id: "test".to_string(),
    }
}

pub fn emitter(sink: &Arc<RecordingSink>) -> Arc<CaptureEmitter> {
    Arc::new(CaptureEmitter::new(INITIAL.to_string(), sink.clone()))
}

/// What the proxy does before it calls the relay: the connection's `Open` and `UpgradedWebSocket`.
pub fn announce(emitter: &CaptureEmitter, connection: &str) {
    let open = ConnectionEvent::Open {
        alpn: Some("http/1.1".to_string()),
        already_open: false,
    };
    assert_eq!(
        emitter.connection_event(connection, time(), open),
        SubmitResult::Queued
    );
    assert_eq!(
        emitter.connection_event(connection, time(), ConnectionEvent::UpgradedWebSocket),
        SubmitResult::Queued
    );
}

/// The sanitized request metadata the HTTP relay hands over with a captured upgrade.
pub fn request_meta(connection: &str) -> CaptureMeta {
    CaptureMeta {
        wire: Some(WireIdentity {
            connection: connection.to_string(),
            exchange: EXCHANGE,
            message_index: None,
        }),
        label: "responses websocket request".to_string(),
        protocol_hint: Some("openai-responses".to_string()),
        method: Some("GET".to_string()),
        host: Some("api.example.test".to_string()),
        path: Some("/v1/responses".to_string()),
        status: Some(101),
        content_type: None,
    }
}

pub struct Setup {
    pub connection: String,
    pub client_prefix: Vec<u8>,
    pub upstream_prefix: Vec<u8>,
    /// Written into each socket before the relay starts.
    pub client_preload: Vec<u8>,
    pub upstream_preload: Vec<u8>,
    pub extensions: Option<String>,
    pub capture: Option<CaptureMeta>,
}

impl Setup {
    pub fn captured(connection: &str) -> Self {
        Setup {
            connection: connection.to_string(),
            client_prefix: Vec::new(),
            upstream_prefix: Vec::new(),
            client_preload: Vec::new(),
            upstream_preload: Vec::new(),
            extensions: None,
            capture: Some(request_meta(connection)),
        }
    }

    pub fn relay_only(connection: &str) -> Self {
        Setup {
            capture: None,
            ..Setup::captured(connection)
        }
    }

    pub fn extensions(mut self, value: &str) -> Self {
        self.extensions = Some(value.to_string());
        self
    }
}

/// One side the test plays.
pub struct Peer {
    pub rx: ReadHalf<DuplexStream>,
    pub tx: WriteHalf<DuplexStream>,
}

impl Peer {
    pub async fn send(&mut self, bytes: &[u8]) {
        tokio::time::timeout(DEADLINE, self.tx.write_all(bytes))
            .await
            .expect("write within the deadline")
            .expect("write");
    }

    /// Read exactly `bytes` and nothing else.
    pub async fn expect(&mut self, bytes: &[u8]) {
        let mut got = vec![0u8; bytes.len()];
        tokio::time::timeout(DEADLINE, self.rx.read_exact(&mut got))
            .await
            .expect("forwarded within the deadline")
            .expect("read");
        assert!(got == bytes, "forwarded bytes differ from the bytes sent");
    }

    pub async fn expect_eof(&mut self) {
        let mut extra = [0u8; 256];
        let read = tokio::time::timeout(DEADLINE, self.rx.read(&mut extra))
            .await
            .expect("EOF within the deadline")
            .expect("read");
        assert_eq!(read, 0, "unexpected extra bytes: {:02x?}", &extra[..read]);
    }
}

pub struct Relay {
    /// The test as the harness.
    pub client: Peer,
    /// The test as the provider.
    pub upstream: Peer,
    pub task: JoinHandle<io::Result<()>>,
}

pub async fn start(emitter: &Arc<CaptureEmitter>, setup: Setup) -> Relay {
    let (client_test, client_relay) = tokio::io::duplex(DUPLEX_BYTES);
    let (upstream_relay, upstream_test) = tokio::io::duplex(DUPLEX_BYTES);
    let (client_rx, mut client_tx) = tokio::io::split(client_test);
    let (upstream_rx, mut upstream_tx) = tokio::io::split(upstream_test);
    client_tx.write_all(&setup.client_preload).await.unwrap();
    upstream_tx
        .write_all(&setup.upstream_preload)
        .await
        .unwrap();
    let upgrade = Upgrade {
        exchange: EXCHANGE,
        pair: RelayPair {
            client: Box::new(client_relay),
            upstream: Box::new(upstream_relay),
        },
        context: RelayContext {
            connection: setup.connection,
            host: "api.example.test".to_string(),
            emitter: emitter.clone(),
            policy: CapturePolicy::default(),
        },
        client_prefix: setup.client_prefix,
        upstream_prefix: setup.upstream_prefix,
        extensions: setup.extensions,
        capture: setup.capture,
    };
    Relay {
        client: Peer {
            rx: client_rx,
            tx: client_tx,
        },
        upstream: Peer {
            rx: upstream_rx,
            tx: upstream_tx,
        },
        task: tokio::spawn(relay_websocket(upgrade)),
    }
}

impl Relay {
    /// Harness to provider: send, and wait until the provider has exactly these bytes.
    pub async fn pass_client(&mut self, bytes: &[u8]) {
        self.client.send(bytes).await;
        self.upstream.expect(bytes).await;
    }

    /// Provider to harness: send, and wait until the harness has exactly these bytes.
    pub async fn pass_server(&mut self, bytes: &[u8]) {
        self.upstream.send(bytes).await;
        self.client.expect(bytes).await;
    }

    /// Pass each byte on its own: every read the relay makes is one byte.
    pub async fn pass_client_bytewise(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.pass_client(std::slice::from_ref(byte)).await;
        }
    }

    pub async fn pass_server_bytewise(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.pass_server(std::slice::from_ref(byte)).await;
        }
    }

    /// A barrier: everything the client sent before has been observed by the relay (the
    /// ping is read only after the previous read was processed).
    pub async fn settle_client(&mut self) {
        self.pass_client(&Frame::ping(b"settle".to_vec()).masked(key(7)).encode())
            .await;
    }

    pub async fn settle_server(&mut self) {
        self.pass_server(&Frame::ping(b"settle".to_vec()).encode())
            .await;
    }

    /// Complete the closing handshake, end both sockets and return the relay's result.
    pub async fn close(mut self) -> io::Result<()> {
        self.pass_client(&Frame::close(1000).masked(key(1000)).encode())
            .await;
        self.pass_server(&Frame::close(1000).encode()).await;
        self.end().await
    }

    /// End both sockets as they stand and return the relay's result.
    pub async fn end(mut self) -> io::Result<()> {
        self.client.tx.shutdown().await.unwrap();
        self.upstream.tx.shutdown().await.unwrap();
        self.client.expect_eof().await;
        self.upstream.expect_eof().await;
        tokio::time::timeout(DEADLINE, self.task)
            .await
            .expect("the relay ends within the deadline")
            .expect("the relay task")
    }
}

/// The payload streams of one connection, in `Open` order.
pub fn of_connection(all: &[Captured], connection: &str) -> Vec<Captured> {
    all.iter()
        .filter(|stream| {
            stream
                .meta
                .wire
                .as_ref()
                .is_some_and(|wire| wire.connection == connection)
        })
        .cloned()
        .collect()
}

/// The bound every emitted segment respects.
pub fn assert_segment_bound(all: &[Captured]) {
    for stream in all {
        assert!(
            stream.contiguous,
            "offsets not contiguous: {:?}",
            stream.key
        );
        assert!(stream.largest_bytes_frame <= MAX_SEGMENT_BYTES);
    }
}
