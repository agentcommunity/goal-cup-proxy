//! The HTTP/1.1 relay, `relay_http(pair, context)`.
//!
//! **Forwarding is the product; capture is a side channel.** Each direction reads a bounded
//! chunk, finds the HTTP/1.1 message framing in it, writes those original bytes to the other
//! side unchanged and flushes, and only then hands the decoded body bytes of a selected
//! exchange to the synchronous, non-blocking [`CaptureEmitter`]. Nothing on the forwarding
//! path waits for the emitter, and whatever it answers, bytes keep flowing. Framing is parsed
//! only to know where a message ends (keep-alive, pipelining, the upgrade boundary); a request
//! is never reserialized.
//!
//! **What is captured.** Only an exchange whose Host names the admitted host and whose
//! query-free path the host profile selects ([`crate::proxy::metadata`]). A request whose Host
//! (or absolute-form authority) names any other host turns the connection into an opaque relay,
//! counted and chained as a loss: another host's traffic never rides uncaptured inside an
//! admitted connection. Its request body is a
//! `WireRequest` stream, its final response body a `WireResponse` stream; both carry the typed
//! [`goalcup_protocol::capture::WireIdentity`] of this physical connection and a per-connection
//! exchange counter (starting at one), with `message_index: None`. Informational (1xx) heads
//! are forwarded but open no stream. `Bytes` are emitted per read, at decoded offsets, after
//! removing chunked framing and at most one `gzip`/`x-gzip`/`deflate`/`br`/`zstd` coding; any
//! other coding records a named `UnsupportedEncoding` gap and no body bytes, and `host.log`
//! names the codings ([`unsupported_codings_line`]). A decode failure keeps the
//! decoded prefix, records `DecodeFailed` and ends `DecodeError`; no encoded byte is ever
//! submitted as captured content. EOF inside a body ends the stream `Interrupted` and the relay
//! returns `UnexpectedEof`.
//!
//! **Capture loss and recovery.** A refused emit abandons that stream (a local
//! `LimitExceeded` refusal is ended explicitly) while relaying
//! continues. Only at the head of a newly begun selected request does this relay call
//! [`CaptureEmitter::try_recover`]; the emitter owns rotation and re-observation. A response
//! whose request was not captured is never a capture boundary, and nothing is replayed.
//!
//! **Upgrades.** A request carrying `Upgrade` or a `Connection: upgrade` token pauses the
//! request direction until its final response. Only a `101` that answers a valid WebSocket
//! request for the admitted host, with the matching `Sec-WebSocket-Accept` (RFC 6455
//! section 4.2.2, [`websocket_accept_key`]), becomes [`HttpOutcome::WebSocket`]: every byte either direction had
//! read past the handshake is handed over as a prefix and neither forwarded nor captured here.
//! A non-`101` answer is ordinary HTTP evidence and parsing resumes; any other `101` (bad
//! accept, missing token, Host mismatch, non-WebSocket protocol) turns the connection into an
//! opaque byte relay with no further capture. So does any framing this relay cannot parse.
//! Either way the fallback is counted on the emitter when it happens, however the connection
//! then ends, and a relay that ends cleanly after it returns [`HttpOutcome::Opaque`].
//!
//! No header value, query, cookie or credential is retained, logged or put in an error: the
//! parsed heads keep only framing fields, and errors carry fixed text. The one exception is the
//! content-coding tokens of a body left undecoded, sanitized and bounded, in `host.log` only.

use crate::proxy::api::{
    BoxIo, CaptureEmitter, CapturePolicy, HttpOutcome, RelayContext, RelayPair, Upgrade,
};
use crate::proxy::metadata::{self, ExchangeMeta, SafeTarget};
use brotli_decompressor::{
    Allocator, BrotliDecompressStream, BrotliResult, BrotliState, SliceWrapper, SliceWrapperMut,
};
use flate2::{Crc, Decompress, FlushDecompress, Status};
use goalcup_protocol::capture::{
    CaptureMeta, CaptureSource, CaptureTime, EndReason, FrameEvent, GapReason,
    MAX_CAPTURE_QUEUE_BYTES, StreamKey, SubmitResult,
};
use std::io;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::{mpsc, oneshot};

/// The `Sec-WebSocket-Accept` value for a request's `Sec-WebSocket-Key` (RFC 6455 section
/// 4.2.2): base64 of the SHA-1 of the key followed by the protocol's fixed GUID.
fn websocket_accept_key(key: &[u8]) -> String {
    use base64::Engine as _;
    use sha1::{Digest, Sha1};
    const GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let mut hash = Sha1::new();
    hash.update(key);
    hash.update(GUID);
    base64::engine::general_purpose::STANDARD.encode(hash.finalize())
}

/// The most bytes one read asks for. A TLS record carries at most 16 KiB of plaintext.
const READ_CHUNK: usize = 16 * 1024;
/// The largest request or response head (and chunked trailer section) parsed.
const MAX_HEAD_BYTES: usize = 64 * 1024;
/// The longest chunk-size line, extensions included.
const MAX_CHUNK_LINE_BYTES: usize = 4 * 1024;
/// Requests the client may have in flight ahead of their responses.
const MAX_PIPELINED: usize = 16;
/// Decoded bytes produced per inflate call, and so per `Bytes` frame of a coded body.
const INFLATE_OUT_BYTES: usize = 32 * 1024;
/// The most decoded bytes one fed data range may expand into: the body data of one read, or
/// one chunk's share of it. More could never fit the capture queue, so decoding stops there
/// with a `LimitExceeded` gap instead of spending the work. Each range is at most one read,
/// and deflate expands by at most about 1032 to 1, so the work per range stays bounded.
const MAX_DECODED_PER_READ: usize = MAX_CAPTURE_QUEUE_BYTES;
/// The largest gzip member header (extra field, name and comment included).
const MAX_GZIP_HEADER_BYTES: usize = 64 * 1024;
/// The largest zstd window accepted, as a power of two: 8 MiB, the most an HTTP `zstd`
/// content coding may use (RFC 9659 section 3). A larger frame is a decode failure.
const ZSTD_WINDOW_LOG_MAX: u32 = 23;
/// What `host.log` says of an undecoded body's codings: at most this many tokens, each cut to
/// [`MAX_LOGGED_CODING_BYTES`].
const MAX_LOGGED_CODINGS: usize = 4;
const MAX_LOGGED_CODING_BYTES: usize = 32;

/// Relay one intercepted HTTP/1.1 connection until both sides are done, or until an accepted
/// WebSocket upgrade hands the connection to the WebSocket relay.
///
/// `Ok(Closed)` means both directions ended at message boundaries; `Ok(Opaque)` means both
/// ended but at least one had stopped parsing and relayed the rest as bytes. EOF inside a
/// message returns `UnexpectedEof`, and so does a request that arrives after the upstream
/// closed cleanly (it is not forwarded); other I/O errors are returned as they occurred. Dropping
/// the future ends every open capture stream `Interrupted`.
pub async fn relay_http(pair: RelayPair, context: RelayContext) -> io::Result<HttpOutcome> {
    let RelayPair { client, upstream } = pair;
    let (client_read, client_write) = tokio::io::split(client);
    let (upstream_read, upstream_write) = tokio::io::split(upstream);
    let (tx, rx) = mpsc::channel(MAX_PIPELINED);
    let shared = Shared {
        connection: &context.connection,
        host: &context.host,
        emitter: &context.emitter,
        policy: &context.policy,
        requests_opaque: AtomicBool::new(false),
        responses_closed: AtomicBool::new(false),
        fell_back: AtomicBool::new(false),
    };
    let (requests, responses) = tokio::try_join!(
        relay_requests(&shared, Inbound::new(client_read), upstream_write, tx),
        relay_responses(&shared, Inbound::new(upstream_read), client_write, rx),
    )?;
    match (requests, responses) {
        (RequestEnd::Closed, ResponseEnd::Closed) => Ok(HttpOutcome::Closed),
        (RequestEnd::Opaque, ResponseEnd::Closed | ResponseEnd::Opaque)
        | (RequestEnd::Closed, ResponseEnd::Opaque) => Ok(HttpOutcome::Opaque),
        (
            RequestEnd::Upgraded {
                client: harness_in,
                upstream,
            },
            ResponseEnd::Upgraded(hand),
        ) => {
            let Handoff {
                upstream: upstream_in,
                client: client_write,
                exchange,
                extensions,
                capture,
            } = *hand;
            Ok(HttpOutcome::WebSocket(Upgrade {
                exchange,
                pair: RelayPair {
                    client: harness_in.io.unsplit(client_write),
                    upstream: upstream_in.io.unsplit(upstream),
                },
                context,
                client_prefix: harness_in.buf,
                upstream_prefix: upstream_in.buf,
                extensions,
                capture,
            }))
        }
        _ => Err(io::Error::other(
            "the WebSocket hand-off lost one of its directions",
        )),
    }
}

/// What both directions of one connection read from the relay context.
struct Shared<'a> {
    connection: &'a str,
    host: &'a str,
    emitter: &'a CaptureEmitter,
    policy: &'a CapturePolicy,
    /// Set when the request direction stopped parsing: later responses cannot be paired.
    requests_opaque: AtomicBool,
    /// Set when the response direction ended cleanly: the upstream closed at a message boundary.
    responses_closed: AtomicBool,
    /// Set when either direction first stopped parsing.
    fell_back: AtomicBool,
}

impl Shared<'_> {
    /// Counts this connection's fallback to opaque bytes, once, when it happens: an error or
    /// a cancellation later on cannot hide it.
    fn fall_back(&self) {
        if !self.fell_back.swap(true, Ordering::SeqCst) {
            self.emitter
                .record_opaque_fallback(metadata::capture_time());
        }
    }
}

/// One request's facts the response direction needs, sent before the request is forwarded.
struct Pending {
    exchange: u64,
    /// A HEAD request's response has no body whatever its headers say.
    head: bool,
    /// A CONNECT request's 2xx response turns the connection into a tunnel.
    connect: bool,
    capture: Option<ExchangeCapture>,
    upgrade: Option<UpgradeAsk>,
}

/// A selected exchange: its retained metadata, and whether its request is being captured.
struct ExchangeCapture {
    meta: ExchangeMeta,
    request_content_type: Option<String>,
    /// True while the request's stream is open or ended normally; false if it never opened
    /// or its capture was lost. A response is captured only behind a captured request.
    request_ok: Arc<AtomicBool>,
}

/// A request that asked to switch protocols; the request direction waits on `decision`.
struct UpgradeAsk {
    /// The key of a well-formed WebSocket request for the admitted host; `None` for any
    /// other upgrade request, which can never be handed to the WebSocket relay.
    websocket_key: Option<String>,
    decision: oneshot::Sender<Decision>,
}

enum Decision {
    /// Not switched: keep parsing HTTP.
    Continue,
    /// A valid `101`: stop and hand every unread byte to the WebSocket relay.
    WebSocket,
    /// Switched to something this relay does not parse: relay bytes opaquely.
    Opaque,
}

enum RequestEnd {
    Closed,
    /// Stopped parsing and relayed the rest as bytes.
    Opaque,
    Upgraded {
        client: Inbound<ReadHalf<BoxIo>>,
        upstream: WriteHalf<BoxIo>,
    },
}

enum ResponseEnd {
    Closed,
    /// Stopped parsing and relayed the rest as bytes.
    Opaque,
    Upgraded(Box<Handoff>),
}

struct Handoff {
    upstream: Inbound<ReadHalf<BoxIo>>,
    client: WriteHalf<BoxIo>,
    exchange: u64,
    extensions: Option<String>,
    capture: Option<CaptureMeta>,
}

// ---------------------------------------------------------------------------------------
// The two directions.
// ---------------------------------------------------------------------------------------

async fn relay_requests(
    shared: &Shared<'_>,
    mut src: Inbound<ReadHalf<BoxIo>>,
    mut dst: WriteHalf<BoxIo>,
    tx: mpsc::Sender<Pending>,
) -> io::Result<RequestEnd> {
    let mut exchange: u64 = 0;
    loop {
        let end = match src.read_head().await? {
            HeadRead::Complete(end) => end,
            HeadRead::Eof => {
                half_close(&mut dst).await;
                return Ok(RequestEnd::Closed);
            }
            HeadRead::PartialEof => {
                forward(&mut dst, &src.buf).await?;
                half_close(&mut dst).await;
                return Err(unexpected_eof("the client closed inside a request head"));
            }
            HeadRead::TooLarge => return requests_opaque(shared, tx, src, dst).await,
        };
        let Some(request) = RequestHead::parse(&src.buf[..end]) else {
            return requests_opaque(shared, tx, src, dst).await;
        };
        let Some((mut framing, codings)) = request.framing() else {
            return requests_opaque(shared, tx, src, dst).await;
        };
        let target = metadata::safe_target(&request.target);
        let host_ok = target
            .as_ref()
            .is_some_and(|t| metadata::host_matches(shared.host, &request.fields.host, t));
        // A request for another host inside a connection admitted for this one would be relayed
        // uncaptured: the connection falls back to opaque, which is chained as a loss. So does a
        // target with no safe form (an authority-form CONNECT, `OPTIONS *`), fail-safe.
        if !host_ok {
            return requests_opaque(shared, tx, src, dst).await;
        }
        exchange += 1;
        shared.emitter.record_model_request();
        let at = src.at.clone();
        let capture = select(shared, &request, target.as_ref(), exchange);
        let request_ok = capture.as_ref().map(|c| c.request_ok.clone());
        let meta = capture
            .as_ref()
            .map(|c| c.meta.request(c.request_content_type.clone()));
        let (upgrade, decision) = if request.asks_upgrade() {
            let (sender, receiver) = oneshot::channel();
            let websocket_key = request.websocket_key();
            let ask = UpgradeAsk {
                websocket_key,
                decision: sender,
            };
            (Some(ask), Some(receiver))
        } else {
            (None, None)
        };
        let pending = Pending {
            exchange,
            head: request.method == "HEAD",
            connect: request.method == "CONNECT",
            capture,
            upgrade,
        };
        // Sent before the request is forwarded, so its response always finds it.
        if tx.send(pending).await.is_err() {
            // The upstream closed an idle keep-alive as this request arrived (the reuse race):
            // nothing was relayed or lost, and the client retries on a new connection. It ends
            // as a peer that went away, as it does when the close lands after the send.
            if shared.responses_closed.load(Ordering::SeqCst) {
                return Err(unexpected_eof("the upstream closed before this request"));
            }
            return requests_opaque(shared, tx, src, dst).await;
        }

        // Recovery only here, at the head of a newly begun selected request.
        let mut body = BodyCapture::none(shared.emitter);
        if let (Some(meta), Some(flag)) = (meta, request_ok) {
            if shared.emitter.try_recover(at.clone()) == SubmitResult::Queued {
                body = BodyCapture::open(
                    shared.emitter,
                    CaptureSource::WireRequest,
                    meta,
                    &codings,
                    &at,
                    Some(flag.clone()),
                );
            }
            flag.store(body.is_open(), Ordering::SeqCst);
        }

        forward(&mut dst, &src.buf[..end]).await?;
        src.consume(end);
        match relay_body(&mut src, &mut dst, &mut framing, &mut body).await {
            Ok(BodyEnd::Complete) => body.finish(true, &src.at),
            Ok(BodyEnd::Malformed) => {
                body.malformed(&src.at);
                return requests_opaque(shared, tx, src, dst).await;
            }
            Err(error) => {
                body.finish(false, &metadata::capture_time());
                if error.kind() == io::ErrorKind::UnexpectedEof {
                    half_close(&mut dst).await;
                }
                return Err(error);
            }
        }
        if let Some(decision) = decision {
            match decision.await {
                Ok(Decision::Continue) => {}
                Ok(Decision::WebSocket) => {
                    return Ok(RequestEnd::Upgraded {
                        client: src,
                        upstream: dst,
                    });
                }
                // A dropped decision means the response side is gone or stopped parsing:
                // raw forwarding is the one continuation that can never stall.
                Ok(Decision::Opaque) | Err(_) => {
                    return requests_opaque(shared, tx, src, dst).await;
                }
            }
        }
    }
}

/// Decide whether this exchange is captured: the admitted host, a retainable method and
/// path, and a path the host policy selects.
fn select(
    shared: &Shared<'_>,
    request: &RequestHead,
    target: Option<&SafeTarget>,
    exchange: u64,
) -> Option<ExchangeCapture> {
    let target = target.filter(|_| metadata::is_method(&request.method))?;
    let path = metadata::select(shared.policy, &target.path)?;
    Some(ExchangeCapture {
        meta: ExchangeMeta {
            connection: shared.connection.to_string(),
            exchange,
            host: shared.host.to_string(),
            method: request.method.clone(),
            path: target.path.clone(),
            protocol_hint: path.protocol_hint.clone(),
        },
        request_content_type: request.fields.content_type.clone(),
        request_ok: Arc::new(AtomicBool::new(false)),
    })
}

async fn requests_opaque(
    shared: &Shared<'_>,
    tx: mpsc::Sender<Pending>,
    src: Inbound<ReadHalf<BoxIo>>,
    dst: WriteHalf<BoxIo>,
) -> io::Result<RequestEnd> {
    shared.fall_back();
    shared.requests_opaque.store(true, Ordering::SeqCst);
    drop(tx);
    opaque(src, dst).await?;
    Ok(RequestEnd::Opaque)
}

async fn relay_responses(
    shared: &Shared<'_>,
    mut src: Inbound<ReadHalf<BoxIo>>,
    mut dst: WriteHalf<BoxIo>,
    mut rx: mpsc::Receiver<Pending>,
) -> io::Result<ResponseEnd> {
    // The request the next final response answers, once its first head has arrived.
    let mut current: Option<Pending> = None;
    loop {
        let end = match src.read_head().await? {
            HeadRead::Complete(end) => end,
            HeadRead::Eof => {
                let unanswered = current.is_some() || !rx.is_empty();
                half_close(&mut dst).await;
                if unanswered {
                    return Err(unexpected_eof("the upstream closed before answering"));
                }
                shared.responses_closed.store(true, Ordering::SeqCst);
                return Ok(ResponseEnd::Closed);
            }
            HeadRead::PartialEof => {
                forward(&mut dst, &src.buf).await?;
                half_close(&mut dst).await;
                return Err(unexpected_eof("the upstream closed inside a response head"));
            }
            HeadRead::TooLarge => return responses_opaque(shared, current, rx, src, dst).await,
        };
        let Some(response) = ResponseHead::parse(&src.buf[..end]) else {
            return responses_opaque(shared, current, rx, src, dst).await;
        };
        if current.is_none() {
            current = match rx.try_recv() {
                Ok(pending) => Some(pending),
                Err(_) if shared.requests_opaque.load(Ordering::SeqCst) => {
                    return responses_opaque(shared, None, rx, src, dst).await;
                }
                // Unsolicited (a request is always queued before it is forwarded).
                Err(_) => None,
            };
        }
        let status = response.status;
        if (100..200).contains(&status) && status != 101 {
            // Informational: forwarded, never a stream; the final response is still due.
            forward(&mut dst, &src.buf[..end]).await?;
            src.consume(end);
            continue;
        }
        let mut pending = current.take();
        let at = src.at.clone();
        let exchange_capture = pending
            .as_ref()
            .and_then(|p| p.capture.as_ref())
            .filter(|c| c.request_ok.load(Ordering::SeqCst));
        let response_meta = exchange_capture.map(|c| {
            c.meta
                .response(status, response.fields.content_type.clone())
        });

        if status == 101 {
            let ask = pending.as_mut().and_then(|p| p.upgrade.take());
            let accepted = ask
                .as_ref()
                .and_then(|a| a.websocket_key.as_deref())
                .is_some_and(|key| response.accepts_websocket(key));
            if let Some(meta) = response_meta {
                let mut stream = BodyCapture::open(
                    shared.emitter,
                    CaptureSource::WireResponse,
                    meta,
                    &[],
                    &at,
                    None,
                );
                stream.finish(true, &at);
            }
            forward(&mut dst, &src.buf[..end]).await?;
            src.consume(end);
            let (pending, ask) = match (pending, ask) {
                (Some(pending), Some(ask)) if accepted => (pending, ask),
                (pending, refused) => {
                    // Switched to something this relay will not parse: bytes only.
                    if let Some(ask) = refused {
                        let _ = ask.decision.send(Decision::Opaque);
                    }
                    return responses_opaque(shared, pending, rx, src, dst).await;
                }
            };
            if ask.decision.send(Decision::WebSocket).is_err() {
                return Err(io::Error::other("the request side of an upgrade is gone"));
            }
            let capture = pending
                .capture
                .map(|c| c.meta.request(c.request_content_type));
            return Ok(ResponseEnd::Upgraded(Box::new(Handoff {
                upstream: src,
                client: dst,
                exchange: pending.exchange,
                extensions: response.extensions(),
                capture,
            })));
        }

        let (head_request, connect) = pending
            .as_ref()
            .map_or((false, false), |p| (p.head, p.connect));
        let Some((mut framing, codings)) = response.framing(head_request, connect) else {
            forward(&mut dst, &src.buf[..end]).await?;
            src.consume(end);
            return responses_opaque(shared, pending, rx, src, dst).await;
        };
        let mut body = match response_meta {
            Some(meta) => BodyCapture::open(
                shared.emitter,
                CaptureSource::WireResponse,
                meta,
                &codings,
                &at,
                None,
            ),
            None => BodyCapture::none(shared.emitter),
        };
        forward(&mut dst, &src.buf[..end]).await?;
        src.consume(end);
        if let Some(ask) = pending.and_then(|p| p.upgrade) {
            // Refused: the upgrade request was ordinary HTTP after all.
            let _ = ask.decision.send(Decision::Continue);
        }
        let until_eof = matches!(framing, Framing::UntilEof);
        match relay_body(&mut src, &mut dst, &mut framing, &mut body).await {
            Ok(BodyEnd::Complete) => {
                body.finish(true, &src.at);
                if until_eof {
                    half_close(&mut dst).await;
                    // Only a close with nothing queued behind it is the keep-alive reuse race.
                    if rx.is_empty() {
                        shared.responses_closed.store(true, Ordering::SeqCst);
                    }
                    return Ok(ResponseEnd::Closed);
                }
            }
            Ok(BodyEnd::Malformed) => {
                body.malformed(&src.at);
                return responses_opaque(shared, None, rx, src, dst).await;
            }
            Err(error) => {
                body.finish(false, &metadata::capture_time());
                if error.kind() == io::ErrorKind::UnexpectedEof {
                    half_close(&mut dst).await;
                }
                return Err(error);
            }
        }
    }
}

/// Stop parsing responses: release every waiting upgrade request to raw forwarding, tell
/// the request direction by closing the queue, and copy bytes until EOF.
async fn responses_opaque(
    shared: &Shared<'_>,
    current: Option<Pending>,
    mut rx: mpsc::Receiver<Pending>,
    src: Inbound<ReadHalf<BoxIo>>,
    dst: WriteHalf<BoxIo>,
) -> io::Result<ResponseEnd> {
    shared.fall_back();
    let mut waiting = current.into_iter().collect::<Vec<_>>();
    while let Ok(pending) = rx.try_recv() {
        waiting.push(pending);
    }
    drop(rx);
    for ask in waiting.into_iter().filter_map(|p| p.upgrade) {
        let _ = ask.decision.send(Decision::Opaque);
    }
    opaque(src, dst).await?;
    Ok(ResponseEnd::Opaque)
}

/// Relay whatever arrives, unchanged and uncaptured, until EOF; then half-close.
async fn opaque<R, W>(mut src: Inbound<R>, mut dst: W) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    forward(&mut dst, &src.buf).await?;
    src.buf.clear();
    loop {
        let n = src.io.read(&mut src.scratch).await?;
        if n == 0 {
            break;
        }
        forward(&mut dst, &src.scratch[..n]).await?;
    }
    half_close(&mut dst).await;
    Ok(())
}

/// Pass an end of stream on. Every byte was already written and flushed, so a peer that is
/// already gone changes nothing this relay could report.
async fn half_close<W: AsyncWrite + Unpin>(dst: &mut W) {
    let _ = dst.shutdown().await;
}

async fn forward<W: AsyncWrite + Unpin>(dst: &mut W, bytes: &[u8]) -> io::Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    dst.write_all(bytes).await?;
    dst.flush().await
}

fn unexpected_eof(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, what)
}

// ---------------------------------------------------------------------------------------
// Reading.
// ---------------------------------------------------------------------------------------

/// One direction's source: bounded reads into a buffer of not-yet-forwarded bytes.
struct Inbound<R> {
    io: R,
    buf: Vec<u8>,
    scratch: Box<[u8]>,
    /// When the most recent read completed: the observation for the bytes it brought.
    at: CaptureTime,
}

enum HeadRead {
    /// A complete head occupies `buf[..end]`.
    Complete(usize),
    /// EOF with nothing buffered: a clean end between messages.
    Eof,
    /// EOF with part of a head buffered.
    PartialEof,
    /// No head end within [`MAX_HEAD_BYTES`].
    TooLarge,
}

impl<R: AsyncRead + Unpin> Inbound<R> {
    fn new(io: R) -> Self {
        Inbound {
            io,
            buf: Vec::new(),
            scratch: vec![0u8; READ_CHUNK].into_boxed_slice(),
            at: metadata::capture_time(),
        }
    }

    /// Read once, appending at most [`READ_CHUNK`] bytes. Zero means EOF.
    async fn fill(&mut self) -> io::Result<usize> {
        let n = self.io.read(&mut self.scratch).await?;
        if n > 0 {
            self.at = metadata::capture_time();
            self.buf.extend_from_slice(&self.scratch[..n]);
        }
        Ok(n)
    }

    fn consume(&mut self, n: usize) {
        self.buf.drain(..n);
    }

    async fn read_head(&mut self) -> io::Result<HeadRead> {
        let mut scan = HeadScan::default();
        loop {
            if let Some(end) = scan.find(&self.buf) {
                return Ok(if end <= MAX_HEAD_BYTES {
                    HeadRead::Complete(end)
                } else {
                    HeadRead::TooLarge
                });
            }
            if self.buf.len() >= MAX_HEAD_BYTES {
                return Ok(HeadRead::TooLarge);
            }
            if self.fill().await? == 0 {
                return Ok(if self.buf.is_empty() {
                    HeadRead::Eof
                } else {
                    HeadRead::PartialEof
                });
            }
        }
    }
}

/// Incremental search for the empty line ending a head; resumable, so a head arriving a byte
/// at a time is scanned once. Empty lines before the start line are tolerated (RFC 9112 §2.2).
#[derive(Default)]
struct HeadScan {
    line_start: usize,
    started: bool,
}

impl HeadScan {
    fn find(&mut self, buf: &[u8]) -> Option<usize> {
        while let Some(nl) = memchr(b'\n', &buf[self.line_start..]) {
            let line = strip_cr(&buf[self.line_start..self.line_start + nl]);
            self.line_start += nl + 1;
            if !line.is_empty() {
                self.started = true;
            } else if self.started {
                return Some(self.line_start);
            }
        }
        None
    }
}

fn memchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|&b| b == needle)
}

fn strip_cr(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

// ---------------------------------------------------------------------------------------
// Heads. Only framing and handshake fields are kept; every other header is skipped unread.
// ---------------------------------------------------------------------------------------

#[derive(Default)]
struct Fields {
    content_length: Option<u64>,
    transfer_encoding: Vec<String>,
    content_encoding: Vec<String>,
    content_type: Option<String>,
    connection: Vec<String>,
    upgrade: Vec<String>,
    host: Vec<String>,
    websocket_key: Vec<String>,
    websocket_version: Vec<String>,
    websocket_accept: Vec<String>,
    websocket_extensions: Vec<String>,
}

impl Fields {
    /// Parse header lines. `None` for what a recipient must reject: obsolete line folding, a
    /// malformed name, or a framing field (Content-Length, Transfer-Encoding) that is invalid,
    /// conflicting or not ASCII. Any other field this relay reads counts as absent when its
    /// value is not ASCII; every field it does not read is skipped unexamined.
    fn parse<'a>(lines: impl Iterator<Item = &'a [u8]>) -> Option<Fields> {
        let mut fields = Fields::default();
        let mut content_type_seen = false;
        for line in lines {
            if line.is_empty() {
                break;
            }
            if matches!(line[0], b' ' | b'\t') {
                return None;
            }
            let colon = memchr(b':', line)?;
            let name = std::str::from_utf8(&line[..colon]).ok()?;
            if !metadata::is_token(name) {
                return None;
            }
            let name = name.to_ascii_lowercase();
            let value = std::str::from_utf8(trim_ows(&line[colon + 1..]))
                .ok()
                .filter(|v| v.is_ascii());
            match (name.as_str(), value) {
                ("content-length" | "transfer-encoding", None) => return None,
                ("content-length", Some(value)) => {
                    for part in value.split(',') {
                        let length = parse_length(part.trim())?;
                        if fields.content_length.is_some_and(|seen| seen != length) {
                            return None;
                        }
                        fields.content_length = Some(length);
                    }
                }
                ("transfer-encoding", Some(value)) => {
                    fields.transfer_encoding.extend(tokens(value));
                }
                (_, None) => {}
                ("content-encoding", Some(value)) => fields.content_encoding.extend(tokens(value)),
                ("content-type", Some(value)) => {
                    if !content_type_seen {
                        content_type_seen = true;
                        fields.content_type = metadata::content_type_essence(value);
                    }
                }
                ("connection", Some(value)) => fields.connection.extend(tokens(value)),
                ("upgrade", Some(value)) => fields.upgrade.extend(tokens(value)),
                ("host", Some(value)) => fields.host.push(value.to_string()),
                ("sec-websocket-key", Some(value)) => fields.websocket_key.push(value.to_string()),
                ("sec-websocket-version", Some(value)) => {
                    fields.websocket_version.push(value.to_string());
                }
                ("sec-websocket-accept", Some(value)) => {
                    fields.websocket_accept.push(value.to_string());
                }
                ("sec-websocket-extensions", Some(value)) => {
                    fields.websocket_extensions.push(value.to_string());
                }
                _ => {}
            }
        }
        Some(fields)
    }

    fn has_connection_upgrade(&self) -> bool {
        self.connection.iter().any(|t| t == "upgrade")
    }

    fn upgrades_to_websocket(&self) -> bool {
        self.upgrade
            .iter()
            .any(|p| p.split('/').next() == Some("websocket"))
    }

    /// The codings to undo on the capture side, and the body framing: `Some(true)` for
    /// chunked, `Some(false)` for other transfer codings (no framing of their own), `None`
    /// without Transfer-Encoding.
    fn transfer(&self) -> (Option<bool>, Vec<String>) {
        let te = &self.transfer_encoding;
        if te.is_empty() {
            return (None, self.content_encoding.clone());
        }
        let chunked = te.last().is_some_and(|t| t == "chunked");
        let transfer = if chunked {
            &te[..te.len() - 1]
        } else {
            &te[..]
        };
        let mut codings = self.content_encoding.clone();
        codings.extend(transfer.iter().cloned());
        (Some(chunked), codings)
    }
}

fn trim_ows(value: &[u8]) -> &[u8] {
    let start = value
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t'))
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|b| !matches!(b, b' ' | b'\t'))
        .map_or(start, |i| i + 1);
    &value[start..end.max(start)]
}

fn tokens(value: &str) -> impl Iterator<Item = String> + '_ {
    value
        .split(',')
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty())
}

fn parse_length(digits: &str) -> Option<u64> {
    (!digits.is_empty() && digits.len() <= 19 && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}

fn head_lines(head: &[u8]) -> impl Iterator<Item = &[u8]> {
    head.split(|&b| b == b'\n')
        .map(strip_cr)
        .skip_while(|line| line.is_empty())
}

fn is_http1(version: &str) -> bool {
    version == "HTTP/1.1" || version == "HTTP/1.0"
}

struct RequestHead {
    method: String,
    /// The raw request target. Never retained: [`metadata::safe_target`] reduces it.
    target: String,
    http11: bool,
    fields: Fields,
}

impl RequestHead {
    fn parse(head: &[u8]) -> Option<RequestHead> {
        let mut lines = head_lines(head);
        let line = std::str::from_utf8(lines.next()?).ok()?;
        let mut parts = line.split(' ');
        let (method, target, version) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some()
            || !metadata::is_token(method)
            || target.is_empty()
            || !target.bytes().all(|b| b.is_ascii_graphic())
            || !is_http1(version)
        {
            return None;
        }
        Some(RequestHead {
            method: method.to_string(),
            target: target.to_string(),
            http11: version == "HTTP/1.1",
            fields: Fields::parse(lines)?,
        })
    }

    /// Request body framing (RFC 9112 §6.3): chunked last, else Content-Length, else none.
    /// A request with another final transfer coding cannot be framed.
    fn framing(&self) -> Option<(Framing, Vec<String>)> {
        match self.fields.transfer() {
            (Some(true), codings) => Some((Framing::Chunked(Chunked::default()), codings)),
            (Some(false), _) => None,
            (None, codings) => Some((
                match self.fields.content_length {
                    Some(length) => Framing::Length(length),
                    None => Framing::Empty,
                },
                codings,
            )),
        }
    }

    fn asks_upgrade(&self) -> bool {
        !self.fields.upgrade.is_empty() || self.fields.has_connection_upgrade()
    }

    /// The key of a well-formed WebSocket opening handshake (RFC 6455 §4.1), else `None`.
    fn websocket_key(&self) -> Option<String> {
        let f = &self.fields;
        let [key] = f.websocket_key.as_slice() else {
            return None;
        };
        (self.method == "GET"
            && self.http11
            && f.upgrades_to_websocket()
            && f.has_connection_upgrade()
            && f.websocket_version.iter().map(String::as_str).eq(["13"]))
        .then(|| key.clone())
    }
}

struct ResponseHead {
    status: u16,
    fields: Fields,
}

impl ResponseHead {
    fn parse(head: &[u8]) -> Option<ResponseHead> {
        let mut lines = head_lines(head);
        let line = std::str::from_utf8(lines.next()?).ok()?;
        let mut parts = line.splitn(3, ' ');
        let (version, code) = (parts.next()?, parts.next()?);
        if !is_http1(version) || code.len() != 3 || !code.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some(ResponseHead {
            status: code.parse().ok()?,
            fields: Fields::parse(lines)?,
        })
    }

    /// Response body framing (RFC 9112 §6.3). `None` for a CONNECT tunnel, which this
    /// relay does not parse.
    fn framing(&self, head_request: bool, connect: bool) -> Option<(Framing, Vec<String>)> {
        let (transfer, codings) = self.fields.transfer();
        if head_request || matches!(self.status, 100..=199 | 204 | 304) {
            return Some((Framing::Empty, codings));
        }
        if connect && (200..300).contains(&self.status) {
            return None;
        }
        let framing = match (transfer, self.fields.content_length) {
            (Some(true), _) => Framing::Chunked(Chunked::default()),
            (Some(false), _) => Framing::UntilEof,
            (None, Some(length)) => Framing::Length(length),
            (None, None) => Framing::UntilEof,
        };
        Some((framing, codings))
    }

    /// A `101` completing the WebSocket handshake whose request carried `key`.
    fn accepts_websocket(&self, key: &str) -> bool {
        let expected = websocket_accept_key(key.as_bytes());
        self.fields.upgrades_to_websocket()
            && self.fields.has_connection_upgrade()
            && self
                .fields
                .websocket_accept
                .iter()
                .map(String::as_str)
                .eq([expected.as_str()])
    }

    /// The negotiated extension text, exactly as the server stated it.
    fn extensions(&self) -> Option<String> {
        let all = &self.fields.websocket_extensions;
        (!all.is_empty()).then(|| all.join(", "))
    }
}

// ---------------------------------------------------------------------------------------
// Body framing: which buffered bytes belong to the current message, and which are data.
// ---------------------------------------------------------------------------------------

enum Framing {
    Empty,
    Length(u64),
    Chunked(Chunked),
    UntilEof,
}

enum BodyEnd {
    Complete,
    /// The framing broke; everything after the consumed prefix is not HTTP to this relay.
    Malformed,
}

struct Step {
    consumed: usize,
    done: bool,
    malformed: bool,
}

impl Step {
    fn more(consumed: usize) -> Step {
        Step {
            consumed,
            done: false,
            malformed: false,
        }
    }
    fn done(consumed: usize) -> Step {
        Step {
            consumed,
            done: true,
            malformed: false,
        }
    }
    fn malformed(consumed: usize) -> Step {
        Step {
            consumed,
            done: false,
            malformed: true,
        }
    }
}

impl Framing {
    /// Consume what `input` holds of this body; push the ranges that are body data.
    fn advance(&mut self, input: &[u8], data: &mut Vec<Range<usize>>) -> Step {
        match self {
            Framing::Empty => Step::done(0),
            Framing::Length(remaining) => {
                let take = usize::try_from(*remaining)
                    .unwrap_or(usize::MAX)
                    .min(input.len());
                if take > 0 {
                    data.push(0..take);
                }
                *remaining -= take as u64;
                if *remaining == 0 {
                    Step::done(take)
                } else {
                    Step::more(take)
                }
            }
            Framing::Chunked(chunked) => chunked.advance(input, data),
            Framing::UntilEof => {
                if !input.is_empty() {
                    data.push(0..input.len());
                }
                Step::more(input.len())
            }
        }
    }
}

#[derive(Default)]
struct Chunked {
    state: ChunkState,
    trailer_bytes: usize,
}

#[derive(Default)]
enum ChunkState {
    #[default]
    Size,
    Data(u64),
    DataEnd,
    Trailer,
}

impl Chunked {
    fn advance(&mut self, input: &[u8], data: &mut Vec<Range<usize>>) -> Step {
        let mut pos = 0;
        loop {
            let rest = &input[pos..];
            match self.state {
                ChunkState::Size => {
                    let Some(nl) = memchr(b'\n', rest) else {
                        return if rest.len() > MAX_CHUNK_LINE_BYTES {
                            Step::malformed(pos)
                        } else {
                            Step::more(pos)
                        };
                    };
                    let Some(size) = (nl <= MAX_CHUNK_LINE_BYTES)
                        .then(|| chunk_size(strip_cr(&rest[..nl])))
                        .flatten()
                    else {
                        return Step::malformed(pos);
                    };
                    pos += nl + 1;
                    self.state = if size == 0 {
                        ChunkState::Trailer
                    } else {
                        ChunkState::Data(size)
                    };
                }
                ChunkState::Data(remaining) => {
                    if rest.is_empty() {
                        return Step::more(pos);
                    }
                    let take = usize::try_from(remaining)
                        .unwrap_or(usize::MAX)
                        .min(rest.len());
                    data.push(pos..pos + take);
                    pos += take;
                    let left = remaining - take as u64;
                    self.state = if left == 0 {
                        ChunkState::DataEnd
                    } else {
                        ChunkState::Data(left)
                    };
                }
                ChunkState::DataEnd => {
                    match rest {
                        [] | [b'\r'] => return Step::more(pos),
                        [b'\r', b'\n', ..] => pos += 2,
                        [b'\n', ..] => pos += 1,
                        _ => return Step::malformed(pos),
                    }
                    self.state = ChunkState::Size;
                }
                ChunkState::Trailer => {
                    let Some(nl) = memchr(b'\n', rest) else {
                        return if self.trailer_bytes + rest.len() > MAX_HEAD_BYTES {
                            Step::malformed(pos)
                        } else {
                            Step::more(pos)
                        };
                    };
                    self.trailer_bytes += nl + 1;
                    if self.trailer_bytes > MAX_HEAD_BYTES {
                        return Step::malformed(pos);
                    }
                    let empty = strip_cr(&rest[..nl]).is_empty();
                    pos += nl + 1;
                    if empty {
                        return Step::done(pos);
                    }
                }
            }
        }
    }
}

/// `chunk-size [ BWS ; chunk-ext ]`: hexadecimal, at most 16 significant digits.
fn chunk_size(line: &[u8]) -> Option<u64> {
    let size = line.split(|&b| b == b';').next()?;
    let size = trim_ows(size);
    let digits = std::str::from_utf8(size).ok()?;
    let significant = digits.trim_start_matches('0');
    if digits.is_empty() || significant.len() > 16 || !digits.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    u64::from_str_radix(
        if significant.is_empty() {
            "0"
        } else {
            significant
        },
        16,
    )
    .ok()
}

/// Forward one message body and hand its data to capture, a read at a time: consumed bytes
/// are written and flushed first, then their data ranges are captured.
async fn relay_body<R, W>(
    src: &mut Inbound<R>,
    dst: &mut W,
    framing: &mut Framing,
    body: &mut BodyCapture<'_>,
) -> io::Result<BodyEnd>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut data = Vec::new();
    loop {
        data.clear();
        let step = framing.advance(&src.buf, &mut data);
        if step.consumed > 0 {
            forward(dst, &src.buf[..step.consumed]).await?;
            for range in &data {
                body.feed(&src.buf[range.clone()], &src.at);
            }
            src.consume(step.consumed);
        }
        if step.malformed {
            return Ok(BodyEnd::Malformed);
        }
        if step.done {
            return Ok(BodyEnd::Complete);
        }
        if src.fill().await? == 0 {
            return if matches!(framing, Framing::UntilEof) {
                Ok(BodyEnd::Complete)
            } else {
                Err(unexpected_eof("the peer closed inside a message body"))
            };
        }
    }
}

// ---------------------------------------------------------------------------------------
// The capture side channel for one body.
// ---------------------------------------------------------------------------------------

/// One body's capture stream. Never blocks and never fails the relay: every refusal just
/// ends or abandons the stream. Dropped while open (cancellation), it ends `Interrupted`.
struct BodyCapture<'e> {
    emitter: &'e CaptureEmitter,
    /// `Some` while this relay may still emit on the stream.
    key: Option<StreamKey>,
    decoder: Decoder,
    /// Decoded bytes emitted so far: the next `Bytes` offset.
    offset: u64,
    /// A gap made the rest of this body undecodable; no more `Bytes`.
    failed: bool,
    /// The request's capture flag, cleared if this stream's capture is lost.
    request_ok: Option<Arc<AtomicBool>>,
}

impl<'e> BodyCapture<'e> {
    fn none(emitter: &'e CaptureEmitter) -> Self {
        BodyCapture {
            emitter,
            key: None,
            decoder: Decoder::identity(),
            offset: 0,
            failed: false,
            request_ok: None,
        }
    }

    fn open(
        emitter: &'e CaptureEmitter,
        source: CaptureSource,
        meta: CaptureMeta,
        codings: &[String],
        at: &CaptureTime,
        request_ok: Option<Arc<AtomicBool>>,
    ) -> Self {
        let decoder = Decoder::for_codings(codings);
        let unsupported = decoder.is_none();
        let mut this = BodyCapture {
            emitter,
            key: emitter.open(source, meta, at.clone()).ok(),
            decoder: decoder.unwrap_or_else(Decoder::identity),
            offset: 0,
            failed: false,
            request_ok,
        };
        if unsupported {
            this.gap(GapReason::UnsupportedEncoding, at);
            // The trace's `Gap` record has no room for the coding (v5 is frozen), so the run's
            // log says which coding it was: the next unknown one is diagnosable.
            crate::host_log::log_only(&unsupported_codings_line(codings));
        }
        this
    }

    fn is_open(&self) -> bool {
        self.key.is_some()
    }

    fn feed(&mut self, data: &[u8], at: &CaptureTime) {
        let BodyCapture {
            emitter,
            key: Some(key),
            decoder,
            offset,
            failed: false,
            ..
        } = self
        else {
            return;
        };
        let mut refused = None;
        let result = decoder.feed(data, &mut |piece: &[u8]| {
            let event = FrameEvent::Bytes {
                offset: *offset,
                data: piece.to_vec(),
            };
            match emitter.emit(key, at.clone(), event) {
                SubmitResult::Queued => {
                    *offset += piece.len() as u64;
                    true
                }
                SubmitResult::Disabled(reason) => {
                    refused = Some(reason);
                    false
                }
            }
        });
        if let Some(reason) = refused {
            self.abandon(reason);
            return;
        }
        match result {
            Ok(()) => {}
            Err(DecodeError::Invalid) => self.gap(GapReason::DecodeFailed, at),
            Err(DecodeError::Budget) => self.gap(GapReason::LimitExceeded, at),
        }
    }

    /// Record why the rest of this body is not captured; its end will say `DecodeError`.
    fn gap(&mut self, reason: GapReason, at: &CaptureTime) {
        self.failed = true;
        let Some(key) = &self.key else {
            return;
        };
        let event = FrameEvent::Gap {
            reason,
            lost_bytes: None,
        };
        if let SubmitResult::Disabled(refused) = self.emitter.emit(key, at.clone(), event) {
            self.abandon(refused);
        }
    }

    /// The body's framing broke: what was decoded stays, the rest is unknown.
    fn malformed(&mut self, at: &CaptureTime) {
        if !self.failed {
            self.gap(GapReason::DecodeFailed, at);
        }
        self.end(EndReason::DecodeError, at);
    }

    /// End the stream: `complete` when the message's framing finished.
    fn finish(&mut self, complete: bool, at: &CaptureTime) {
        if self.key.is_none() {
            return;
        }
        let reason = if !complete {
            EndReason::Interrupted
        } else if self.failed {
            EndReason::DecodeError
        } else if self.decoder.finish().is_err() {
            self.gap(GapReason::DecodeFailed, at);
            EndReason::DecodeError
        } else {
            EndReason::Complete
        };
        self.end(reason, at);
    }

    fn end(&mut self, reason: EndReason, at: &CaptureTime) {
        if let Some(key) = self.key.take() {
            let _ = self
                .emitter
                .emit(&key, at.clone(), FrameEvent::End { reason });
        }
    }

    /// The emitter refused: capture of this body is over. A stale or failed epoch has
    /// already disowned the stream; a local refusal left it open, so it is ended here.
    /// Either way, nothing more is emitted and nothing is replayed.
    ///
    /// The end carries a fresh observation, not the refused one: a refusal of that
    /// observation itself would otherwise refuse its own gap and end too, leaving the
    /// stream open and the producer unable to close.
    fn abandon(&mut self, reason: GapReason) {
        if let Some(key) = self.key.take()
            && reason == GapReason::LimitExceeded
        {
            let at = metadata::capture_time();
            let gap = FrameEvent::Gap {
                reason,
                lost_bytes: None,
            };
            let _ = self.emitter.emit(&key, at.clone(), gap);
            let end = FrameEvent::End {
                reason: EndReason::Interrupted,
            };
            let _ = self.emitter.emit(&key, at, end);
        }
        if let Some(flag) = &self.request_ok {
            flag.store(false, Ordering::SeqCst);
        }
    }
}

impl Drop for BodyCapture<'_> {
    fn drop(&mut self) {
        if self.key.is_some() {
            self.end(EndReason::Interrupted, &metadata::capture_time());
        }
    }
}

// ---------------------------------------------------------------------------------------
// Capture-side decoding: bounded streaming inflate. Never touches forwarded bytes.
// ---------------------------------------------------------------------------------------

enum DecodeError {
    /// Not a valid stream of its coding (or incomplete at the end of the body).
    Invalid,
    /// One fed data range expanded past [`MAX_DECODED_PER_READ`].
    Budget,
}

/// `emit` receives each decoded piece and answers whether capture still wants more.
type Emit<'a> = dyn FnMut(&[u8]) -> bool + 'a;

struct Decoder {
    kind: DecoderKind,
    fed: bool,
}

enum DecoderKind {
    Identity,
    Gzip(Box<Gzip>),
    Deflate(Box<Deflate>),
    Brotli(Box<Brotli>),
    Zstd(Box<Zstd>),
}

impl Decoder {
    fn identity() -> Self {
        Decoder {
            kind: DecoderKind::Identity,
            fed: false,
        }
    }

    /// At most one non-identity coding is decoded; anything else is unsupported (`None`).
    fn for_codings(codings: &[String]) -> Option<Self> {
        let mut coded = codings.iter().filter(|c| c.as_str() != "identity");
        let kind = match (coded.next().map(String::as_str), coded.next()) {
            (None, _) => DecoderKind::Identity,
            (Some("gzip" | "x-gzip"), None) => DecoderKind::Gzip(Box::default()),
            (Some("deflate"), None) => DecoderKind::Deflate(Box::default()),
            (Some("br"), None) => DecoderKind::Brotli(Box::default()),
            (Some("zstd"), None) => DecoderKind::Zstd(Box::new(Zstd::new()?)),
            _ => return None,
        };
        Some(Decoder { kind, fed: false })
    }

    fn feed(&mut self, input: &[u8], emit: &mut Emit<'_>) -> Result<(), DecodeError> {
        if input.is_empty() {
            return Ok(());
        }
        self.fed = true;
        let mut budget = MAX_DECODED_PER_READ;
        match &mut self.kind {
            DecoderKind::Identity => {
                emit(input);
                Ok(())
            }
            DecoderKind::Gzip(gzip) => gzip.feed(input, &mut budget, emit),
            DecoderKind::Deflate(deflate) => deflate.feed(input, &mut budget, emit),
            DecoderKind::Brotli(brotli) => brotli.feed(input, &mut budget, emit),
            DecoderKind::Zstd(zstd) => zstd.feed(input, &mut budget, emit),
        }
    }

    /// Did the coded stream end exactly? An empty body needs no decoding.
    fn finish(&self) -> Result<(), DecodeError> {
        let complete = !self.fed
            || match &self.kind {
                DecoderKind::Identity => true,
                DecoderKind::Gzip(gzip) => gzip.complete(),
                DecoderKind::Deflate(deflate) => deflate.ended,
                DecoderKind::Brotli(brotli) => brotli.ended,
                DecoderKind::Zstd(zstd) => zstd.frame_done,
            };
        if complete {
            Ok(())
        } else {
            Err(DecodeError::Invalid)
        }
    }
}

enum Flow {
    NeedInput,
    Ended,
    Stop,
}

/// Inflate from `input` until it is used up, the stream ends, or capture stops wanting
/// output, emitting at most [`INFLATE_OUT_BYTES`] at a time.
fn inflate(
    z: &mut Decompress,
    out: &mut [u8],
    input: &mut &[u8],
    budget: &mut usize,
    mut crc: Option<&mut Crc>,
    emit: &mut Emit<'_>,
) -> Result<Flow, DecodeError> {
    loop {
        let (in_before, out_before) = (z.total_in(), z.total_out());
        let result = z.decompress(input, out, FlushDecompress::None);
        // Counted before the result is looked at: a call that meets corruption may still
        // have decoded valid output first, and that prefix is kept.
        let consumed = (z.total_in() - in_before) as usize;
        let produced = (z.total_out() - out_before) as usize;
        *input = &input[consumed..];
        if produced > 0 {
            *budget = budget.checked_sub(produced).ok_or(DecodeError::Budget)?;
            if let Some(crc) = crc.as_deref_mut() {
                crc.update(&out[..produced]);
            }
            if !emit(&out[..produced]) {
                return Ok(Flow::Stop);
            }
        }
        let status = result.map_err(|_| DecodeError::Invalid)?;
        if status == Status::StreamEnd {
            return Ok(Flow::Ended);
        }
        if consumed == 0 && produced == 0 {
            return if input.is_empty() {
                Ok(Flow::NeedInput)
            } else {
                Err(DecodeError::Invalid)
            };
        }
    }
}

/// RFC 1952 members, one after another: header, raw deflate, CRC-32 and size trailer.
struct Gzip {
    state: GzipState,
    z: Decompress,
    crc: Crc,
    out: Vec<u8>,
    members: u64,
}

enum GzipState {
    Header(Vec<u8>),
    Body,
    Trailer(Vec<u8>),
}

impl Default for Gzip {
    fn default() -> Self {
        Gzip {
            state: GzipState::Header(Vec::new()),
            z: Decompress::new(false),
            crc: Crc::new(),
            out: vec![0; INFLATE_OUT_BYTES],
            members: 0,
        }
    }
}

impl Gzip {
    fn feed(
        &mut self,
        mut input: &[u8],
        budget: &mut usize,
        emit: &mut Emit<'_>,
    ) -> Result<(), DecodeError> {
        while !input.is_empty() {
            match &mut self.state {
                GzipState::Header(header) => {
                    let before = header.len();
                    let take = input.len().min(MAX_GZIP_HEADER_BYTES + 1 - before);
                    header.extend_from_slice(&input[..take]);
                    match gzip_header_len(header)? {
                        Some(len) => {
                            input = &input[len - before..];
                            self.z.reset(false);
                            self.crc.reset();
                            self.state = GzipState::Body;
                        }
                        None if header.len() > MAX_GZIP_HEADER_BYTES => {
                            return Err(DecodeError::Invalid);
                        }
                        None => input = &input[take..],
                    }
                }
                GzipState::Body => {
                    match inflate(
                        &mut self.z,
                        &mut self.out,
                        &mut input,
                        budget,
                        Some(&mut self.crc),
                        emit,
                    )? {
                        Flow::Ended => self.state = GzipState::Trailer(Vec::with_capacity(8)),
                        Flow::NeedInput => {}
                        Flow::Stop => return Ok(()),
                    }
                }
                GzipState::Trailer(trailer) => {
                    let take = (8 - trailer.len()).min(input.len());
                    trailer.extend_from_slice(&input[..take]);
                    input = &input[take..];
                    if trailer.len() == 8 {
                        let crc =
                            u32::from_le_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
                        let size =
                            u32::from_le_bytes([trailer[4], trailer[5], trailer[6], trailer[7]]);
                        if crc != self.crc.sum() || size != self.crc.amount() {
                            return Err(DecodeError::Invalid);
                        }
                        self.members += 1;
                        self.state = GzipState::Header(Vec::new());
                    }
                }
            }
        }
        Ok(())
    }

    fn complete(&self) -> bool {
        self.members > 0 && matches!(&self.state, GzipState::Header(h) if h.is_empty())
    }
}

/// The length of a complete gzip member header at the start of `h`; `None` if more bytes
/// are needed.
fn gzip_header_len(h: &[u8]) -> Result<Option<usize>, DecodeError> {
    const MAGIC: [u8; 3] = [0x1f, 0x8b, 8];
    if h.iter().zip(MAGIC).any(|(got, want)| *got != want) {
        return Err(DecodeError::Invalid);
    }
    let Some(&flags) = h.get(3) else {
        return Ok(None);
    };
    if flags & 0xe0 != 0 {
        return Err(DecodeError::Invalid);
    }
    let mut pos = 10;
    if flags & 0x04 != 0 {
        let Some(len) = h.get(pos..pos + 2) else {
            return Ok(None);
        };
        pos += 2 + usize::from(u16::from_le_bytes([len[0], len[1]]));
    }
    for flag in [0x08, 0x10] {
        if flags & flag != 0 {
            let Some(zero) = h.get(pos..).and_then(|rest| memchr(0, rest)) else {
                return Ok(None);
            };
            pos += zero + 1;
        }
    }
    if flags & 0x02 != 0 {
        pos += 2;
    }
    Ok((h.len() >= pos).then_some(pos))
}

/// HTTP `deflate`: zlib (RFC 1950) as specified, or the raw deflate some servers send,
/// told apart by the zlib header check on the first two bytes.
struct Deflate {
    z: Option<Decompress>,
    first: Option<u8>,
    ended: bool,
    out: Vec<u8>,
}

impl Default for Deflate {
    fn default() -> Self {
        Deflate {
            z: None,
            first: None,
            ended: false,
            out: vec![0; INFLATE_OUT_BYTES],
        }
    }
}

impl Deflate {
    fn feed(
        &mut self,
        input: &[u8],
        budget: &mut usize,
        emit: &mut Emit<'_>,
    ) -> Result<(), DecodeError> {
        if self.z.is_none() {
            let (b0, b1) = match (self.first, input) {
                (Some(b0), [b1, ..]) => (b0, *b1),
                (None, [b0, b1, ..]) => (*b0, *b1),
                (None, [b0]) => {
                    self.first = Some(*b0);
                    return Ok(());
                }
                (_, _) => return Ok(()),
            };
            let zlib =
                b0 & 0x0f == 8 && b0 >> 4 <= 7 && (u16::from(b0) << 8 | u16::from(b1)) % 31 == 0;
            self.z = Some(Decompress::new(zlib));
            if let Some(b0) = self.first.take()
                && !self.inflate(&[b0], budget, emit)?
            {
                return Ok(());
            }
        }
        self.inflate(input, budget, emit).map(|_| ())
    }

    /// Returns whether capture still wants output.
    fn inflate(
        &mut self,
        mut input: &[u8],
        budget: &mut usize,
        emit: &mut Emit<'_>,
    ) -> Result<bool, DecodeError> {
        if self.ended {
            return if input.is_empty() {
                Ok(true)
            } else {
                Err(DecodeError::Invalid)
            };
        }
        let Some(z) = self.z.as_mut() else {
            return Ok(true);
        };
        match inflate(z, &mut self.out, &mut input, budget, None, emit)? {
            Flow::Ended => {
                self.ended = true;
                if input.is_empty() {
                    Ok(true)
                } else {
                    Err(DecodeError::Invalid)
                }
            }
            Flow::NeedInput => Ok(true),
            Flow::Stop => Ok(false),
        }
    }
}

/// Count `produced` decoded bytes against the read's budget and hand them to capture.
/// Answers whether capture still wants output.
fn emit_decoded(
    produced: &[u8],
    budget: &mut usize,
    emit: &mut Emit<'_>,
) -> Result<bool, DecodeError> {
    if produced.is_empty() {
        return Ok(true);
    }
    *budget = budget
        .checked_sub(produced.len())
        .ok_or(DecodeError::Budget)?;
    Ok(emit(produced))
}

/// HTTP `br` (RFC 7932), decoded to the standard's 16 MiB window at most (no large-window
/// extension). One stream; bytes after its end are a decode failure.
struct Brotli {
    state: BrotliState<BrotliHeap, BrotliHeap, BrotliHeap>,
    ended: bool,
    out: Vec<u8>,
}

/// The brotli decoder's memory: zeroed boxed slices from the global allocator, freed on drop.
#[derive(Default)]
struct BrotliHeap;

struct BrotliCells<T>(Box<[T]>);

impl<T> Default for BrotliCells<T> {
    fn default() -> Self {
        BrotliCells(Box::default())
    }
}

impl<T> SliceWrapper<T> for BrotliCells<T> {
    fn slice(&self) -> &[T] {
        &self.0
    }
}

impl<T> SliceWrapperMut<T> for BrotliCells<T> {
    fn slice_mut(&mut self) -> &mut [T] {
        &mut self.0
    }
}

impl<T: Clone + Default> Allocator<T> for BrotliHeap {
    type AllocatedMemory = BrotliCells<T>;

    fn alloc_cell(&mut self, len: usize) -> BrotliCells<T> {
        BrotliCells(vec![T::default(); len].into_boxed_slice())
    }

    fn free_cell(&mut self, _cells: BrotliCells<T>) {}
}

impl Default for Brotli {
    fn default() -> Self {
        Brotli {
            state: BrotliState::new_strict(BrotliHeap, BrotliHeap, BrotliHeap),
            ended: false,
            out: vec![0; INFLATE_OUT_BYTES],
        }
    }
}

impl Brotli {
    fn feed(
        &mut self,
        input: &[u8],
        budget: &mut usize,
        emit: &mut Emit<'_>,
    ) -> Result<(), DecodeError> {
        if self.ended {
            return if input.is_empty() {
                Ok(())
            } else {
                Err(DecodeError::Invalid)
            };
        }
        let (mut available_in, mut input_offset) = (input.len(), 0);
        loop {
            let (mut available_out, mut output_offset, mut total_out) = (self.out.len(), 0, 0);
            let result = BrotliDecompressStream(
                &mut available_in,
                &mut input_offset,
                input,
                &mut available_out,
                &mut output_offset,
                &mut self.out,
                &mut total_out,
                &mut self.state,
            );
            // Emitted before the result is looked at: output decoded before corruption is kept.
            if !emit_decoded(&self.out[..output_offset], budget, emit)? {
                return Ok(());
            }
            match result {
                BrotliResult::NeedsMoreOutput => {}
                BrotliResult::NeedsMoreInput => return Ok(()),
                BrotliResult::ResultSuccess => {
                    self.ended = true;
                    return if available_in == 0 {
                        Ok(())
                    } else {
                        Err(DecodeError::Invalid)
                    };
                }
                BrotliResult::ResultFailure => return Err(DecodeError::Invalid),
            }
        }
    }
}

/// HTTP `zstd` (RFC 8878 frames, RFC 9659 window limit), one frame or several in a row. The
/// body is complete when its last frame is.
struct Zstd {
    z: zstd::stream::raw::Decoder<'static>,
    frame_done: bool,
    out: Vec<u8>,
}

impl Zstd {
    fn new() -> Option<Self> {
        let mut z = zstd::stream::raw::Decoder::new().ok()?;
        z.set_parameter(zstd::stream::raw::DParameter::WindowLogMax(
            ZSTD_WINDOW_LOG_MAX,
        ))
        .ok()?;
        Some(Zstd {
            z,
            frame_done: false,
            out: vec![0; INFLATE_OUT_BYTES],
        })
    }

    fn feed(
        &mut self,
        input: &[u8],
        budget: &mut usize,
        emit: &mut Emit<'_>,
    ) -> Result<(), DecodeError> {
        use zstd::stream::raw::{InBuffer, Operation, OutBuffer};
        let mut source = InBuffer::around(input);
        loop {
            let before = source.pos();
            let mut sink = OutBuffer::around(&mut self.out[..]);
            let result = self.z.run(&mut source, &mut sink);
            let produced = sink.pos();
            if !emit_decoded(&self.out[..produced], budget, emit)? {
                return Ok(());
            }
            // Zero: a frame just ended and everything it decoded was flushed.
            self.frame_done = result.map_err(|_| DecodeError::Invalid)? == 0;
            let rest = source.pos() < input.len();
            if !rest && produced < self.out.len() {
                return Ok(());
            }
            if rest && produced == 0 && source.pos() == before {
                return Err(DecodeError::Invalid);
            }
        }
    }
}

/// The `host.log` line for a body whose codings are not decoded: each coding as a token (any
/// other byte is `?`), at most [`MAX_LOGGED_CODINGS`] of them, each cut to
/// [`MAX_LOGGED_CODING_BYTES`]. A coding name is not a secret, but it is still header text.
fn unsupported_codings_line(codings: &[String]) -> String {
    let named: Vec<String> = codings
        .iter()
        .take(MAX_LOGGED_CODINGS)
        .map(|coding| {
            coding
                .bytes()
                .take(MAX_LOGGED_CODING_BYTES)
                .map(|b| {
                    if b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b) {
                        char::from(b)
                    } else {
                        '?'
                    }
                })
                .collect()
        })
        .collect();
    let more = codings.len().saturating_sub(MAX_LOGGED_CODINGS);
    let more = if more > 0 {
        format!(" and {more} more")
    } else {
        String::new()
    };
    format!(
        "proxy: a body coded {}{more} is not decoded (unsupported_encoding gap)",
        named.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An undecoded body's log line names its codings, bounded and as tokens only.
    #[test]
    fn the_log_line_for_an_undecoded_body_names_its_codings() {
        let line = unsupported_codings_line(&["gzip".into(), "br".into()]);
        assert!(line.contains("gzip, br"), "{line}");
        assert!(line.contains("unsupported_encoding"), "{line}");

        let line = unsupported_codings_line(&["x\u{1b}[2J".into()]);
        assert!(line.contains("x??2J") && !line.contains('\u{1b}'), "{line}");

        let many: Vec<String> = (0..10)
            .map(|i| format!("c{i}{}", "z".repeat(100)))
            .collect();
        let line = unsupported_codings_line(&many);
        assert!(line.contains("c3") && !line.contains("c4"), "{line}");
        assert!(line.contains("and 6 more"), "{line}");
        assert!(
            !line.contains(&"z".repeat(MAX_LOGGED_CODING_BYTES)),
            "{line}"
        );
    }

    /// The worked example in RFC 6455 section 1.3.
    #[test]
    fn the_websocket_accept_key_matches_the_rfc_sample() {
        assert_eq!(
            websocket_accept_key(b"dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    fn run_chunked(input: &[u8]) -> (Step, Vec<u8>) {
        let mut framing = Chunked::default();
        let mut ranges = Vec::new();
        let step = framing.advance(input, &mut ranges);
        let data = ranges
            .iter()
            .flat_map(|r| input[r.clone()].to_vec())
            .collect();
        (step, data)
    }

    #[test]
    fn chunked_framing_stops_exactly_after_the_trailer_section() {
        let body = b"3;name=\"v\"\r\nabc\r\n1\r\nd\r\n0\r\nX-T: 1\r\n\r\nNEXT";
        let (step, data) = run_chunked(body);
        assert!(step.done && !step.malformed);
        assert_eq!(&body[step.consumed..], b"NEXT");
        assert_eq!(data, b"abcd");
    }

    #[test]
    fn chunked_framing_refuses_what_it_cannot_frame() {
        for bad in [&b"zz\r\n"[..], b"3\r\nabcX", b"11111111111111111\r\n"] {
            let (step, _) = run_chunked(bad);
            assert!(step.malformed, "{:?}", String::from_utf8_lossy(bad));
        }
        let (step, _) = run_chunked(&vec![b'1'; MAX_CHUNK_LINE_BYTES + 1]);
        assert!(step.malformed, "an unterminated over-long size line");
    }

    #[test]
    fn head_scan_resumes_and_skips_leading_empty_lines() {
        let head = b"\r\nGET / HTTP/1.1\r\nHost: h\r\n\r\nBODY";
        let mut scan = HeadScan::default();
        let mut found = None;
        for end in 1..=head.len() {
            if let Some(at) = scan.find(&head[..end]) {
                found = Some(at);
                break;
            }
        }
        assert_eq!(found, Some(head.len() - 4));
    }

    #[test]
    fn conflicting_content_lengths_are_refused() {
        assert!(RequestHead::parse(b"POST / HTTP/1.1\r\nContent-Length: 3, 3\r\n\r\n").is_some());
        assert!(
            RequestHead::parse(
                b"POST / HTTP/1.1\r\nContent-Length: 3\r\nContent-Length: 4\r\n\r\n"
            )
            .is_none()
        );
        assert!(RequestHead::parse(b"POST / HTTP/1.1\r\n Folded: x\r\n\r\n").is_none());
        assert!(RequestHead::parse(b"GET / HTTP/2.0\r\n\r\n").is_none());
    }

    /// Records every submitted frame and admits everything.
    #[derive(Default)]
    struct Recorder(std::sync::Mutex<Vec<goalcup_protocol::capture::CaptureFrame>>);

    impl goalcup_protocol::capture::CaptureSink for Recorder {
        fn try_submit(&self, frame: goalcup_protocol::capture::CaptureFrame) -> SubmitResult {
            self.0.lock().unwrap().push(frame);
            SubmitResult::Queued
        }
        fn try_begin_epoch(&self, _: &str) -> goalcup_protocol::capture::EpochAdmission {
            goalcup_protocol::capture::EpochAdmission::Ready
        }
        fn try_close_epoch(&self, _: &str, _: Option<u64>) -> SubmitResult {
            SubmitResult::Queued
        }
    }

    /// An emit the emitter refuses locally submits nothing and leaves the stream
    /// open. The relay must still end that stream, with an observation the emitter accepts,
    /// or the producer can never close cleanly.
    #[test]
    fn a_locally_refused_emit_is_ended_so_the_producer_can_close() {
        let sink = Arc::new(Recorder::default());
        let emitter = CaptureEmitter::new("0123456789abcdef0123456789abcdef".into(), sink.clone());
        let meta = ExchangeMeta {
            connection: "c0ffee00c0ffee00c0ffee00c0ffee00".into(),
            exchange: 1,
            host: "api.example.test".into(),
            method: "POST".into(),
            path: "/v1/x".into(),
            protocol_hint: None,
        }
        .request(None);
        let request_ok = Arc::new(AtomicBool::new(true));
        let at = metadata::capture_time();
        let mut body = BodyCapture::open(
            &emitter,
            CaptureSource::WireRequest,
            meta,
            &[],
            &at,
            Some(request_ok.clone()),
        );
        body.feed(b"kept", &at);
        let refused = CaptureTime {
            wall: "not a time".into(),
            ..at.clone()
        };
        body.feed(b"refused", &refused);
        assert!(!body.is_open());
        assert!(
            !request_ok.load(Ordering::SeqCst),
            "the request no longer counts as captured"
        );
        drop(body);
        assert_eq!(
            emitter.try_close(metadata::capture_time()),
            SubmitResult::Queued,
            "a stream was left open"
        );
        let events: Vec<FrameEvent> = sink
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|f| f.event.clone())
            .collect();
        assert!(matches!(events[0], FrameEvent::Open { .. }));
        assert_eq!(
            events[1..],
            [
                FrameEvent::Bytes {
                    offset: 0,
                    data: b"kept".to_vec()
                },
                FrameEvent::Gap {
                    reason: GapReason::LimitExceeded,
                    lost_bytes: None
                },
                FrameEvent::End {
                    reason: EndReason::Interrupted
                },
            ]
        );
    }
}
