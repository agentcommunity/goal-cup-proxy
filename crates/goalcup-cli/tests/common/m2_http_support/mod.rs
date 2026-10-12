//! Test-local helpers for `m2_http_relay`: a real `relay_http` task between two in-memory peers,
//! a recording `CaptureSink` whose pressure a test switches on and off, a sink that refuses
//! everything, an I/O wrapper that chops reads and records their size, and an independent
//! frame oracle over the frames a sink recorded.
//!
//! Loopback only: every byte travels through `tokio::io::duplex`. Nothing here parses HTTP
//! the way the relay does; expectations are the literal bytes a test wrote.

use goalcup_cli::proxy::api::{
    BoxIo, CaptureEmitter, CapturePath, CapturePolicy, HttpOutcome, RelayContext, RelayPair,
};
use goalcup_protocol::capture::{
    CaptureFrame, CaptureMeta, CaptureSink, CaptureSource, CaptureTime, ConnectionEvent, EndReason,
    EpochAdmission, FrameEvent, GapReason, MAX_SEGMENT_BYTES, StreamKey, SubmitResult,
};
use std::collections::BTreeMap;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};
use tokio::task::JoinHandle;

pub const PRODUCER: &str = "0123456789abcdef0123456789abcdef";
pub const CONN: &str = "c0ffee00c0ffee00c0ffee00c0ffee00";
pub const CONN_B: &str = "b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0";
pub const HOST: &str = "api.example.test";
pub const WAIT: Duration = Duration::from_secs(10);

/// Records every admitted frame. While `pressure` is on, every submission is refused with
/// `QueueFull` and a fresh epoch stays `Pending`; switching it off admits the next epoch.
#[derive(Default)]
pub struct RecordingSink {
    frames: Mutex<Vec<CaptureFrame>>,
    pressure: AtomicBool,
    refused: AtomicUsize,
    begun: Mutex<Vec<String>>,
}

impl RecordingSink {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn frames(&self) -> Vec<CaptureFrame> {
        self.frames.lock().unwrap().clone()
    }

    pub fn set_pressure(&self, on: bool) {
        self.pressure.store(on, Ordering::SeqCst);
    }

    pub fn refused(&self) -> usize {
        self.refused.load(Ordering::SeqCst)
    }

    pub fn begun(&self) -> Vec<String> {
        self.begun.lock().unwrap().clone()
    }
}

impl CaptureSink for RecordingSink {
    fn try_submit(&self, frame: CaptureFrame) -> SubmitResult {
        if self.pressure.load(Ordering::SeqCst) {
            self.refused.fetch_add(1, Ordering::SeqCst);
            return SubmitResult::Disabled(GapReason::QueueFull);
        }
        self.frames.lock().unwrap().push(frame);
        SubmitResult::Queued
    }

    fn try_begin_epoch(&self, producer: &str) -> EpochAdmission {
        if self.pressure.load(Ordering::SeqCst) {
            return EpochAdmission::Pending;
        }
        self.begun.lock().unwrap().push(producer.to_string());
        EpochAdmission::Ready
    }

    fn try_close_epoch(&self, _producer: &str, _final_seq: Option<u64>) -> SubmitResult {
        SubmitResult::Queued
    }
}

/// A sink that never admits a frame: every submission is `QueueFull` and no fresh epoch is
/// ever ready. It counts how often it was asked, so a test can show capture was attempted.
#[derive(Default)]
pub struct DisabledSink {
    pub submits: AtomicUsize,
    pub admissions: AtomicUsize,
}

impl CaptureSink for DisabledSink {
    fn try_submit(&self, _frame: CaptureFrame) -> SubmitResult {
        self.submits.fetch_add(1, Ordering::SeqCst);
        SubmitResult::Disabled(GapReason::QueueFull)
    }

    fn try_begin_epoch(&self, _producer: &str) -> EpochAdmission {
        self.admissions.fetch_add(1, Ordering::SeqCst);
        EpochAdmission::Pending
    }

    fn try_close_epoch(&self, _producer: &str, _final_seq: Option<u64>) -> SubmitResult {
        SubmitResult::Disabled(GapReason::QueueFull)
    }
}

pub fn now() -> CaptureTime {
    CaptureTime {
        wall: "2026-09-27T10:00:00Z".into(),
        mono_ns: 0,
        clock_id: "test".into(),
    }
}

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

/// Wraps the relay's upstream end: each read is offered at most the next size of `pattern`
/// (cycling), and the largest buffer the relay ever offered is recorded.
pub struct ChoppyIo<T> {
    inner: T,
    pattern: Vec<usize>,
    next: usize,
    pub max_offered: Arc<AtomicUsize>,
}

impl<T> ChoppyIo<T> {
    pub fn new(inner: T, pattern: &[usize]) -> Self {
        assert!(pattern.iter().all(|&n| n > 0));
        ChoppyIo {
            inner,
            pattern: pattern.to_vec(),
            next: 0,
            max_offered: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for ChoppyIo<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        this.max_offered
            .fetch_max(buf.remaining(), Ordering::SeqCst);
        let limit = this.pattern[this.next % this.pattern.len()].min(buf.remaining());
        let mut small = vec![0u8; limit];
        let mut small_buf = ReadBuf::new(&mut small);
        match Pin::new(&mut this.inner).poll_read(cx, &mut small_buf) {
            Poll::Ready(Ok(())) => {
                let filled = small_buf.filled();
                if !filled.is_empty() {
                    this.next += 1;
                }
                buf.put_slice(filled);
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for ChoppyIo<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// One running relay: the harness-facing peer, the provider-facing peer and the task.
pub struct Harness {
    pub client: DuplexStream,
    pub upstream: DuplexStream,
    pub relay: JoinHandle<io::Result<HttpOutcome>>,
    pub emitter: Arc<CaptureEmitter>,
}

/// Admit the initial producer, as the proxy does before constructing the emitter, and report
/// the physical connection open, as the proxy does before handing the pair to `relay_http`.
pub fn emitter_for(sink: Arc<dyn CaptureSink>) -> Arc<CaptureEmitter> {
    assert_eq!(sink.try_begin_epoch(PRODUCER), EpochAdmission::Ready);
    let emitter = Arc::new(CaptureEmitter::new(PRODUCER.to_string(), sink));
    emitter.connection_event(
        CONN,
        now(),
        ConnectionEvent::Open {
            alpn: Some("http/1.1".into()),
            already_open: false,
        },
    );
    emitter
}

/// An emitter over a sink that refuses everything: constructed directly, since its epoch is
/// admitted by construction and the refusing sink is the point.
pub fn refused_emitter(sink: Arc<dyn CaptureSink>) -> Arc<CaptureEmitter> {
    Arc::new(CaptureEmitter::new(PRODUCER.to_string(), sink))
}

pub fn start(emitter: Arc<CaptureEmitter>, policy: CapturePolicy) -> Harness {
    start_with(emitter, policy, 1 << 20, |io| Box::new(io))
}

/// Start `relay_http` with duplex buffers of `capacity` bytes and the relay's upstream end
/// wrapped by `wrap_upstream`.
pub fn start_with(
    emitter: Arc<CaptureEmitter>,
    policy: CapturePolicy,
    capacity: usize,
    wrap_upstream: impl FnOnce(DuplexStream) -> BoxIo,
) -> Harness {
    start_on(emitter, policy, capacity, CONN, wrap_upstream)
}

/// As [`start_with`], on the physical connection `connection`.
pub fn start_on(
    emitter: Arc<CaptureEmitter>,
    policy: CapturePolicy,
    capacity: usize,
    connection: &str,
    wrap_upstream: impl FnOnce(DuplexStream) -> BoxIo,
) -> Harness {
    let (client, relay_client) = tokio::io::duplex(capacity);
    let (relay_upstream, upstream) = tokio::io::duplex(capacity);
    let context = RelayContext {
        connection: connection.to_string(),
        host: HOST.to_string(),
        emitter: emitter.clone(),
        policy,
    };
    let pair = RelayPair {
        client: Box::new(relay_client),
        upstream: wrap_upstream(relay_upstream),
    };
    let relay = tokio::spawn(goalcup_cli::proxy::http::relay_http(pair, context));
    Harness {
        client,
        upstream,
        relay,
        emitter,
    }
}

pub async fn read_exact(io: &mut (impl AsyncRead + Unpin), n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    tokio::time::timeout(WAIT, io.read_exact(&mut buf))
        .await
        .expect("peer did not receive the expected bytes in time")
        .expect("peer read failed");
    buf
}

pub async fn read_to_end(io: &mut (impl AsyncRead + Unpin)) -> Vec<u8> {
    let mut buf = Vec::new();
    tokio::time::timeout(WAIT, io.read_to_end(&mut buf))
        .await
        .expect("peer did not see EOF in time")
        .expect("peer read failed");
    buf
}

pub async fn write(io: &mut (impl AsyncWrite + Unpin), bytes: &[u8]) {
    tokio::time::timeout(WAIT, io.write_all(bytes))
        .await
        .expect("peer write did not complete in time")
        .expect("peer write failed");
}

/// Nothing more arrives within a short window.
pub async fn assert_quiet(io: &mut (impl AsyncRead + Unpin)) {
    let mut one = [0u8; 1];
    let got = tokio::time::timeout(Duration::from_millis(150), io.read(&mut one)).await;
    assert!(got.is_err(), "unexpected bytes or EOF: {got:?}");
}

pub async fn finish(relay: JoinHandle<io::Result<HttpOutcome>>) -> io::Result<HttpOutcome> {
    tokio::time::timeout(WAIT, relay)
        .await
        .expect("relay did not finish in time")
        .expect("relay task panicked")
}

pub async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !cond() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// One payload stream as recorded, reassembled independently of the relay.
#[derive(Debug, Clone)]
pub struct StreamView {
    pub key: StreamKey,
    pub source: CaptureSource,
    pub meta: CaptureMeta,
    pub bytes: Vec<u8>,
    pub byte_frames: Vec<(u64, usize, u64)>, // (offset, len, mono_ns)
    pub gaps: Vec<GapReason>,
    pub end: Option<EndReason>,
    /// Position (index into the frame list) of each event, for ordering assertions.
    pub open_at: usize,
    pub end_at: Option<usize>,
}

/// Check the frames against the frame rules independently of the emitter and reassemble each
/// payload stream: per-producer seq from zero without holes, clock named by the producer, stream
/// zero only for controls, fresh increasing stream IDs, contiguous decoded offsets from zero, no
/// event after End, every `Bytes` within one segment.
pub fn streams(frames: &[CaptureFrame]) -> Vec<StreamView> {
    let mut seqs: BTreeMap<String, u64> = BTreeMap::new();
    let mut high: BTreeMap<String, u64> = BTreeMap::new();
    let mut views: Vec<StreamView> = Vec::new();
    for (at, frame) in frames.iter().enumerate() {
        frame
            .validate()
            .unwrap_or_else(|e| panic!("invalid frame {e}: {frame:?}"));
        assert_eq!(
            frame.observed.clock_id, frame.stream.producer,
            "C1 clock domain"
        );
        let next = seqs.entry(frame.stream.producer.clone()).or_insert(0);
        assert_eq!(frame.seq, *next, "seq hole or reorder at {frame:?}");
        *next += 1;
        let find = |views: &mut Vec<StreamView>| {
            views
                .iter_mut()
                .rposition(|v| v.key == frame.stream)
                .unwrap_or_else(|| panic!("payload on an unopened stream: {frame:?}"))
        };
        match &frame.event {
            FrameEvent::Connection { .. }
            | FrameEvent::Recovery { .. }
            | FrameEvent::Loss { .. } => {}
            FrameEvent::Open { source, metadata } => {
                let hw = high.entry(frame.stream.producer.clone()).or_insert(0);
                assert!(frame.stream.stream > *hw, "stream ID reused: {frame:?}");
                *hw = frame.stream.stream;
                views.push(StreamView {
                    key: frame.stream.clone(),
                    source: *source,
                    meta: metadata.clone(),
                    bytes: Vec::new(),
                    byte_frames: Vec::new(),
                    gaps: Vec::new(),
                    end: None,
                    open_at: at,
                    end_at: None,
                });
            }
            FrameEvent::Bytes { offset, data } => {
                let i = find(&mut views);
                let v = &mut views[i];
                assert!(v.end.is_none(), "bytes after End: {frame:?}");
                assert!(data.len() <= MAX_SEGMENT_BYTES);
                assert!(!data.is_empty(), "empty Bytes frame");
                assert_eq!(*offset, v.bytes.len() as u64, "non-contiguous offset");
                v.bytes.extend_from_slice(data);
                v.byte_frames
                    .push((*offset, data.len(), frame.observed.mono_ns));
            }
            FrameEvent::Gap { reason, .. } => {
                let i = find(&mut views);
                assert!(views[i].end.is_none(), "gap after End: {frame:?}");
                views[i].gaps.push(*reason);
            }
            FrameEvent::End { reason } => {
                let i = find(&mut views);
                assert!(views[i].end.is_none(), "second End: {frame:?}");
                views[i].end = Some(*reason);
                views[i].end_at = Some(at);
            }
        }
    }
    views
}

pub fn exchange_of(view: &StreamView) -> u64 {
    let (connection, exchange) = wire_of(view);
    assert_eq!(connection, CONN);
    exchange
}

/// The typed (connection, exchange) of an HTTP stream.
pub fn wire_of(view: &StreamView) -> (String, u64) {
    let wire = view.meta.wire.as_ref().expect("wire identity");
    assert_eq!(
        wire.message_index, None,
        "HTTP streams have no message index"
    );
    (wire.connection.clone(), wire.exchange)
}

/// `size-in-hex CRLF data CRLF`.
pub fn chunk(data: &[u8]) -> Vec<u8> {
    let mut out = format!("{:x}\r\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\r\n");
    out
}

pub fn gzip(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

/// A gzip stream prefix: header plus everything needed to decode `data`, sync-flushed, with
/// no final block and no trailer.
pub fn gzip_flushed_prefix(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data).unwrap();
    enc.flush().unwrap();
    enc.get_ref().clone()
}

pub fn zlib(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

pub fn raw_deflate(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

/// A zlib stream prefix, sync-flushed, with no final block and no checksum.
pub fn zlib_flushed_prefix(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data).unwrap();
    enc.flush().unwrap();
    enc.get_ref().clone()
}

/// Deterministic pseudo-random bytes (xorshift), so large bodies are not trivially
/// compressible or periodic.
pub fn noise(len: usize, mut seed: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        out.extend_from_slice(&seed.to_le_bytes());
    }
    out.truncate(len);
    out
}

pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
