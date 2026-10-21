//! The WebSocket relay, [`relay_websocket`].
//!
//! After an accepted `101`, the HTTP relay hands over both sockets and every byte either side
//! had already read ([`Upgrade`]). This relay forwards each read unchanged and at once, the
//! prefixes first, and only then looks at what it forwarded: a separate, bounded view of each
//! direction is unmasked and, where negotiated, inflated, and submitted to the shared
//! [`CaptureEmitter`] as one payload stream per logical message. Capture never holds a
//! forwarded byte back and never makes a peer wait for evidence.
//!
//! **Streams.** Every data message (text or binary, however many continuation frames it
//! spans) is one stream of source `WsClient` or `WsServer`. Its `wire` names the physical
//! connection and the upgrade's HTTP exchange, plus a `message_index` taken from one counter
//! shared by both directions at the observed start of each message, under a short
//! connection-local lock held through the stream's `Open` (recorder observation order, not
//! network causality). A message whose capture is skipped still consumes its index, as does
//! every message start of a direction whose capture was turned off, so a hole in the indexes
//! shows where messages went unrecorded. Control frames (ping, pong, close) are
//! forwarded and never captured, never consume an index and never disturb the message they
//! interrupt. `Bytes` offsets count decoded bytes. Each read is emitted before the next is
//! taken, at most `MAX_SEGMENT_BYTES` per frame, so a long message is emitted long before its
//! final frame and working memory is a few fixed buffers per direction. The metadata keeps the
//! handed-over request's sanitized `path`, `host` and `protocol_hint`; `method`, `status` and
//! `content_type` describe the HTTP exchange, not a message, and stay empty.
//! `Upgrade::capture == None` relays without any capture.
//!
//! **Extensions.** Supported: none, or exactly one `permessage-deflate` with any of
//! `server_no_context_takeover`, `client_no_context_takeover`, `server_max_window_bits=8..15`
//! and `client_max_window_bits[=8..15]`, each at most once. Inflation is raw DEFLATE with the
//! fixed 15-bit window (`flate2` on `miniz_oxide`), which decodes any smaller negotiated
//! window. Inflater state is per direction and per connection: kept across messages under
//! context takeover, reset after each message where that sender negotiated no takeover.
//! Anything else (another extension, several, an unknown or repeated parameter, a bad window
//! value) is unsupported: still relayed unchanged, and each direction records one explicit
//! `UnsupportedEncoding` stream ended `DecodeError` at its first data frame, then nothing more.
//!
//! **Loss is explicit, never complete.** A frame that breaks RFC 6455/7692 for its direction
//! (reserved RSV bits or opcode, wrong masking, a long or fragmented control frame, a 64-bit length
//! with its top bit set, broken continuation order, RSV1 without deflate) ends the message it hits,
//! or a stream opened to say so, with `Gap(DecodeFailed)` and `End(DecodeError)`, and turns that
//! direction's capture off. Framing is still tracked, so relaying, index counting and the close
//! outcome are unaffected. Non-minimal length encodings are accepted: their content decodes
//! identically. Invalid compressed data keeps the decoded prefix already emitted and ends the same
//! way; the encoded suffix is never retained. Under context takeover a lost history is never reset
//! mid-connection, and a final DEFLATE block (BFINAL) ends it too: every later compressed message
//! in that direction records an explicit decode loss instead of bytes.
//!
//! **Recovery.** A payload `emit` the emitter refuses abandons that message's capture
//! (`Gap` and `End(Interrupted)` are offered and are no-ops when the epoch is gone) while
//! relaying continues and the inflater keeps its history. Only at the start of a new message
//! is [`CaptureEmitter::try_recover`] called; the emitter re-observes the connection itself.
//! A continuation frame never reopens capture or recovers, even when it breaks the protocol;
//! nothing is replayed, and `Disabled` just skips that message.
//!
//! **Result.** `Ok` only when both directions carried a Close frame and ended at a frame
//! and message boundary. An end mid-frame or mid-message, or without a completed closing
//! handshake, is `UnexpectedEof`; read and write failures are returned as they are. A message
//! still open when the relay ends, fails or is cancelled is ended `Interrupted`.
//!
//! Observation times: one per read, all from the private `now()`; the emitter stamps
//! `clock_id` with its producer.

use crate::proxy::api::{CaptureEmitter, RelayPair, Upgrade};
use flate2::{Decompress, FlushDecompress, Status};
use goalcup_protocol::capture::{
    CaptureMeta, CaptureSource, CaptureTime, EndReason, FrameEvent, GapReason, MAX_SEGMENT_BYTES,
    StreamKey, SubmitResult, WireIdentity,
};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// One socket read: the unit of forwarding and of capture emission.
const READ_BYTES: usize = 16 * 1024;
/// The piece of payload unmasked at a time.
const UNMASK_BYTES: usize = 16 * 1024;
/// The inflater's output chunk.
const INFLATE_BYTES: usize = 32 * 1024;
/// RFC 7692 §7.2.2: the sync-flush marker a sender strips and a receiver appends.
const DEFLATE_TAIL: [u8; 4] = [0x00, 0x00, 0xff, 0xff];

const OP_CONTINUATION: u8 = 0x0;
const OP_TEXT: u8 = 0x1;
const OP_BINARY: u8 = 0x2;
const OP_CLOSE: u8 = 0x8;
const OP_PING: u8 = 0x9;
const OP_PONG: u8 = 0xa;
const RSV1: u8 = 0x40;
const RSV2_RSV3: u8 = 0x30;

/// Relay an accepted WebSocket upgrade until both directions end, capturing each data
/// message of `upgrade.capture`'s request through the connection's emitter.
///
/// Both prefixes are forwarded and observed before their socket is read, and forwarding is
/// byte-identical in both directions whatever capture does. See the module documentation for
/// the supported extensions, the loss semantics and the result mapping.
pub async fn relay_websocket(upgrade: Upgrade) -> io::Result<()> {
    let Upgrade {
        exchange,
        pair,
        context,
        client_prefix,
        upstream_prefix,
        extensions,
        capture,
    } = upgrade;
    let negotiated = negotiate(extensions.as_deref());
    let connection = capture.map(|base| Connection {
        emitter: context.emitter.clone(),
        id: context.connection.clone(),
        exchange,
        base,
        next_index: Mutex::new(0),
    });
    let emitter = &*context.emitter;
    let mut from_client =
        Direction::new(Side::Client, connection.as_ref(), negotiated, Some(emitter));
    let mut from_server = Direction::new(Side::Server, connection.as_ref(), negotiated, None);
    let RelayPair { client, upstream } = pair;
    let (client_read, client_write) = tokio::io::split(client);
    let (upstream_read, upstream_write) = tokio::io::split(upstream);
    let (client_end, server_end) = tokio::try_join!(
        pump(&mut from_client, client_prefix, client_read, upstream_write),
        pump(
            &mut from_server,
            upstream_prefix,
            upstream_read,
            client_write
        ),
    )?;
    if !(client_end.boundary && server_end.boundary) {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the websocket connection ended in the middle of a frame or message",
        ));
    }
    if !(client_end.close_seen && server_end.close_seen) {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the websocket connection ended without a completed closing handshake",
        ));
    }
    Ok(())
}

/// Forward one direction: its prefix, then every read, each written and flushed before it is
/// observed. At EOF the destination's write side is shut down (half-close).
async fn pump<R, W>(
    direction: &mut Direction<'_>,
    prefix: Vec<u8>,
    mut from: R,
    mut to: W,
) -> io::Result<Ended>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    if !prefix.is_empty() {
        let observed = direction.stamp();
        to.write_all(&prefix).await?;
        to.flush().await?;
        direction.observe(&prefix, observed.as_ref());
    }
    drop(prefix);
    let mut buffer = vec![0u8; READ_BYTES];
    loop {
        let read = from.read(&mut buffer).await?;
        let observed = direction.stamp();
        if read == 0 {
            let ended = direction.eof(observed.as_ref());
            // Every byte was already written and flushed; a peer gone before the half-close
            // changes nothing that was relayed.
            let _ = to.shutdown().await;
            return Ok(ended);
        }
        to.write_all(&buffer[..read]).await?;
        to.flush().await?;
        direction.observe(&buffer[..read], observed.as_ref());
    }
}

/// How one direction's byte stream ended.
#[derive(Debug, Clone, Copy)]
struct Ended {
    /// EOF fell between frames and outside any fragmented message.
    boundary: bool,
    /// A Close frame was seen in this direction.
    close_seen: bool,
}

/// The negotiated `Sec-WebSocket-Extensions`, as far as capture is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Negotiated {
    Plain,
    Deflate {
        client_takeover: bool,
        server_takeover: bool,
    },
    Unsupported,
}

fn negotiate(extensions: Option<&str>) -> Negotiated {
    let Some(value) = extensions else {
        return Negotiated::Plain;
    };
    let listed: Vec<&str> = value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect();
    match listed.as_slice() {
        [] => Negotiated::Plain,
        [one] => permessage_deflate(one).unwrap_or(Negotiated::Unsupported),
        _ => Negotiated::Unsupported,
    }
}

/// One `permessage-deflate` extension with only RFC 7692 parameters, each at most once.
fn permessage_deflate(extension: &str) -> Option<Negotiated> {
    let mut parts = extension.split(';').map(str::trim);
    if !parts.next()?.eq_ignore_ascii_case("permessage-deflate") {
        return None;
    }
    let (mut client_takeover, mut server_takeover) = (true, true);
    let mut seen = [false; 4];
    let window = |value: &str| matches!(value.parse::<u8>(), Ok(8..=15));
    for parameter in parts {
        let (name, value) = match parameter.split_once('=') {
            Some((name, value)) => (name.trim(), Some(unquote(value.trim())?)),
            None => (parameter, None),
        };
        let slot = match (name.to_ascii_lowercase().as_str(), value) {
            ("server_no_context_takeover", None) => {
                server_takeover = false;
                0
            }
            ("client_no_context_takeover", None) => {
                client_takeover = false;
                1
            }
            ("server_max_window_bits", Some(value)) if window(value) => 2,
            ("client_max_window_bits", None) => 3,
            ("client_max_window_bits", Some(value)) if window(value) => 3,
            _ => return None,
        };
        if std::mem::replace(&mut seen[slot], true) {
            return None;
        }
    }
    Some(Negotiated::Deflate {
        client_takeover,
        server_takeover,
    })
}

fn unquote(value: &str) -> Option<&str> {
    match value.strip_prefix('"') {
        Some(inner) => inner.strip_suffix('"'),
        None => (!value.contains('"')).then_some(value),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    /// Harness to provider: frames must be masked.
    Client,
    /// Provider to harness: frames must not be masked.
    Server,
}

impl Side {
    fn source(self) -> CaptureSource {
        match self {
            Side::Client => CaptureSource::WsClient,
            Side::Server => CaptureSource::WsServer,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Side::Client => "client",
            Side::Server => "server",
        }
    }
}

/// What a captured connection shares between its two directions.
struct Connection {
    emitter: Arc<CaptureEmitter>,
    id: String,
    exchange: u64,
    /// The handed-over request's sanitized metadata.
    base: CaptureMeta,
    /// The next message index, shared by both directions and held through each `Open`.
    next_index: Mutex<u64>,
}

impl Connection {
    fn metadata(&self, side: Side, index: u64, what: &str) -> CaptureMeta {
        CaptureMeta {
            wire: Some(WireIdentity {
                connection: self.id.clone(),
                exchange: self.exchange,
                message_index: Some(index),
            }),
            label: format!("websocket {} {what}", side.name()),
            protocol_hint: self.base.protocol_hint.clone(),
            method: None,
            host: self.base.host.clone(),
            path: self.base.path.clone(),
            status: None,
            content_type: None,
        }
    }
}

/// A parsed frame header; `rsv` keeps the three reserved bits in their wire positions.
#[derive(Debug, Clone, Copy)]
struct Header {
    fin: bool,
    rsv: u8,
    opcode: u8,
    mask: Option<[u8; 4]>,
    len: u64,
}

impl Header {
    fn control(&self) -> bool {
        self.opcode & 0x8 != 0
    }
}

/// Where the current frame's payload stands.
#[derive(Debug, Clone, Copy)]
struct Payload {
    mask: Option<[u8; 4]>,
    pos: u64,
    len: u64,
}

/// An incremental RFC 6455 frame delimiter. It never rejects a header: the first two bytes
/// always determine the header's size, so framing, and with it the close outcome, survives
/// every protocol violation. Judging a header is capture's business.
#[derive(Debug, Default)]
struct Framer {
    head: [u8; 14],
    have: usize,
    payload: Option<Payload>,
}

fn header_len(second: u8) -> usize {
    let extended = match second & 0x7f {
        126 => 2,
        127 => 8,
        _ => 0,
    };
    2 + extended + if second & 0x80 != 0 { 4 } else { 0 }
}

impl Framer {
    fn mid_frame(&self) -> bool {
        self.have > 0 || self.payload.is_some()
    }

    /// Take header bytes from `data`; the header once it is complete.
    fn take_header(&mut self, data: &mut &[u8]) -> Option<Header> {
        loop {
            let need = if self.have < 2 {
                2
            } else {
                header_len(self.head[1])
            };
            if self.have == need {
                self.have = 0;
                return Some(self.parse());
            }
            if data.is_empty() {
                return None;
            }
            let take = (need - self.have).min(data.len());
            self.head[self.have..self.have + take].copy_from_slice(&data[..take]);
            self.have += take;
            *data = &data[take..];
        }
    }

    fn parse(&self) -> Header {
        let (first, second) = (self.head[0], self.head[1]);
        let (len, at) = match second & 0x7f {
            126 => (
                u64::from(u16::from_be_bytes([self.head[2], self.head[3]])),
                4,
            ),
            127 => {
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&self.head[2..10]);
                (u64::from_be_bytes(bytes), 10)
            }
            short => (u64::from(short), 2),
        };
        let mask = (second & 0x80 != 0).then(|| {
            let mut key = [0u8; 4];
            key.copy_from_slice(&self.head[at..at + 4]);
            key
        });
        Header {
            fin: first & 0x80 != 0,
            rsv: first & 0x70,
            opcode: first & 0x0f,
            mask,
            len,
        }
    }
}

/// One direction of the relay: its framing, and its capture when the upgrade is captured.
struct Direction<'c> {
    /// Counts each data message this direction carries as a request of a captured host: the
    /// client's direction only.
    counting: Option<&'c CaptureEmitter>,
    framer: Framer,
    /// The last data frame did not carry FIN: a fragmented message is still open.
    in_message: bool,
    close_seen: bool,
    capture: Option<Capture<'c>>,
}

impl<'c> Direction<'c> {
    fn new(
        side: Side,
        connection: Option<&'c Connection>,
        negotiated: Negotiated,
        counting: Option<&'c CaptureEmitter>,
    ) -> Self {
        let mode = match negotiated {
            Negotiated::Plain => Mode::Plain,
            Negotiated::Unsupported => Mode::Unsupported,
            Negotiated::Deflate {
                client_takeover,
                server_takeover,
            } => Mode::Deflate {
                takeover: match side {
                    Side::Client => client_takeover,
                    Side::Server => server_takeover,
                },
            },
        };
        Direction {
            counting,
            framer: Framer::default(),
            in_message: false,
            close_seen: false,
            capture: connection.map(|connection| Capture::new(connection, side, mode)),
        }
    }

    /// The observation time of a read, taken only while this direction still captures.
    fn stamp(&self) -> Option<CaptureTime> {
        self.capture
            .as_ref()
            .filter(|capture| capture.active)
            .map(|_| now())
    }

    fn observe(&mut self, mut data: &[u8], observed: Option<&CaptureTime>) {
        while !data.is_empty() {
            if let Some(payload) = &mut self.framer.payload {
                let take = (payload.len - payload.pos).min(data.len() as u64) as usize;
                let (mask, pos) = (payload.mask, payload.pos);
                payload.pos += take as u64;
                let done = payload.pos == payload.len;
                if let (Some(capture), Some(observed)) = (&mut self.capture, observed) {
                    capture.payload(&data[..take], mask, pos, observed);
                }
                data = &data[take..];
                if done {
                    self.framer.payload = None;
                    self.frame_end(observed);
                }
            } else if let Some(header) = self.framer.take_header(&mut data) {
                if header.opcode == OP_CLOSE {
                    self.close_seen = true;
                }
                if !header.control() {
                    self.in_message = !header.fin;
                    if header.opcode != OP_CONTINUATION
                        && let Some(emitter) = self.counting
                    {
                        emitter.record_model_request();
                    }
                }
                if let Some(capture) = &mut self.capture {
                    capture.header(&header, observed);
                }
                if header.len == 0 {
                    self.frame_end(observed);
                } else {
                    self.framer.payload = Some(Payload {
                        mask: header.mask,
                        pos: 0,
                        len: header.len,
                    });
                }
            }
        }
        if let (Some(capture), Some(observed)) = (&mut self.capture, observed) {
            capture.flush(observed);
        }
    }

    fn frame_end(&mut self, observed: Option<&CaptureTime>) {
        if let (Some(capture), Some(observed)) = (&mut self.capture, observed) {
            capture.frame_end(observed);
        }
    }

    fn eof(&mut self, observed: Option<&CaptureTime>) -> Ended {
        if let (Some(capture), Some(observed)) = (&mut self.capture, observed) {
            capture.interrupt(observed);
        }
        Ended {
            boundary: !self.framer.mid_frame() && !self.in_message,
            close_seen: self.close_seen,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Plain,
    Deflate { takeover: bool },
    Unsupported,
}

/// An open payload stream and its next decoded offset.
struct Stream {
    key: StreamKey,
    offset: u64,
}

/// The data message a direction is inside.
struct Message {
    /// Taken at the message's observed start.
    index: u64,
    /// A stream was opened for this message, whatever became of it.
    opened: bool,
    compressed: bool,
    /// Compressed payload still goes through the inflater: false after a decode failure or
    /// the end of the DEFLATE stream.
    inflating: bool,
    /// `None` when this message is not, or no longer, captured.
    stream: Option<Stream>,
}

/// One direction's bounded capture view: unmasking, inflation and emission.
struct Capture<'c> {
    connection: &'c Connection,
    side: Side,
    mode: Mode,
    /// False once an unsupported extension or a protocol violation turned capture off.
    active: bool,
    message: Option<Message>,
    /// The current frame's payload belongs to `message`.
    frame_data: bool,
    /// The current frame is a data frame carrying FIN.
    frame_fin: bool,
    inflater: Option<Decompress>,
    /// Takeover history lost: later compressed messages cannot be decoded faithfully.
    history_lost: bool,
    scratch: Vec<u8>,
    output: Vec<u8>,
    /// Decoded bytes of the current read not yet emitted; at most `MAX_SEGMENT_BYTES`.
    pending: Vec<u8>,
}

impl<'c> Capture<'c> {
    fn new(connection: &'c Connection, side: Side, mode: Mode) -> Self {
        let deflate = matches!(mode, Mode::Deflate { .. });
        Capture {
            connection,
            side,
            mode,
            active: true,
            message: None,
            frame_data: false,
            frame_fin: false,
            inflater: deflate.then(|| Decompress::new(false)),
            history_lost: false,
            scratch: Vec::new(),
            output: if deflate {
                vec![0u8; INFLATE_BYTES]
            } else {
                Vec::new()
            },
            pending: Vec::new(),
        }
    }

    fn takeover(&self) -> bool {
        self.mode == Mode::Deflate { takeover: true }
    }

    /// Whether the current message's payload is needed at all: to be delivered, or to keep a
    /// takeover inflater's history.
    fn wants_payload(&self) -> bool {
        let Some(message) = &self.message else {
            return false;
        };
        if message.compressed {
            message.inflating && (message.stream.is_some() || self.takeover())
        } else {
            message.stream.is_some()
        }
    }

    fn header(&mut self, header: &Header, observed: Option<&CaptureTime>) {
        self.frame_data = false;
        self.frame_fin = false;
        let observed = match observed {
            Some(observed) if self.active => observed,
            _ => {
                // Capture is off for this direction, but its messages are still counted: the
                // index holes show how many went unrecorded.
                if !header.control() && header.opcode != OP_CONTINUATION {
                    self.skip_index();
                }
                return;
            }
        };
        if self.mode == Mode::Unsupported {
            if !header.control() {
                self.record_loss(
                    "messages (unsupported extension)",
                    GapReason::UnsupportedEncoding,
                    observed,
                );
                self.active = false;
            }
            return;
        }
        if !self.valid(header) {
            self.violation(observed);
            return;
        }
        if header.control() {
            return;
        }
        if header.opcode != OP_CONTINUATION {
            self.begin(header, observed);
        }
        self.frame_data = true;
        self.frame_fin = header.fin;
    }

    /// RFC 6455 §5.2/§5.4/§5.5 and RFC 7692 §6 for this direction and its message state.
    fn valid(&self, header: &Header) -> bool {
        let rsv1 = header.rsv & RSV1 != 0;
        let deflate = matches!(self.mode, Mode::Deflate { .. });
        let masking = header.mask.is_some() == (self.side == Side::Client);
        let known = matches!(
            header.opcode,
            OP_CONTINUATION | OP_TEXT | OP_BINARY | OP_CLOSE | OP_PING | OP_PONG
        );
        let sequenced = if header.control() {
            header.fin && header.len <= 125 && !rsv1
        } else if header.opcode == OP_CONTINUATION {
            self.message.is_some() && !rsv1
        } else {
            self.message.is_none() && (deflate || !rsv1)
        };
        header.rsv & RSV2_RSV3 == 0 && known && masking && header.len >> 63 == 0 && sequenced
    }

    fn begin(&mut self, header: &Header, observed: &CaptureTime) {
        let compressed = header.rsv & RSV1 != 0;
        let kind = if header.opcode == OP_TEXT {
            "text"
        } else {
            "binary"
        };
        let what = if compressed {
            format!("{kind} message (permessage-deflate)")
        } else {
            format!("{kind} message")
        };
        let (index, mut stream) = self.open(&what, observed);
        let opened = stream.is_some();
        let decodable = !compressed || !self.history_lost;
        if !decodable && let Some(lost) = stream.take() {
            self.fail(&lost.key, GapReason::DecodeFailed, observed);
        }
        self.message = Some(Message {
            index,
            opened,
            compressed,
            inflating: compressed && decodable,
            stream,
        });
    }

    fn payload(&mut self, data: &[u8], mask: Option<[u8; 4]>, pos: u64, observed: &CaptureTime) {
        if !self.active || !self.frame_data {
            return;
        }
        let mut scratch = std::mem::take(&mut self.scratch);
        let mut pos = pos;
        for piece in data.chunks(UNMASK_BYTES) {
            if !self.wants_payload() {
                break;
            }
            scratch.clear();
            scratch.extend_from_slice(piece);
            if let Some(key) = mask {
                for (i, byte) in scratch.iter_mut().enumerate() {
                    *byte ^= key[((pos + i as u64) & 3) as usize];
                }
            }
            pos += piece.len() as u64;
            if self
                .message
                .as_ref()
                .is_some_and(|message| message.compressed)
            {
                self.inflate(&scratch, observed);
            } else {
                self.deliver(&scratch, observed);
            }
        }
        self.scratch = scratch;
    }

    fn frame_end(&mut self, observed: &CaptureTime) {
        if !(self.active && self.frame_data && self.frame_fin) {
            return;
        }
        let compressed = self.message.as_ref().is_some_and(|m| m.compressed);
        if compressed && self.wants_payload() {
            self.inflate(&DEFLATE_TAIL, observed);
        }
        self.flush(observed);
        if let Some(stream) = self.message.take().and_then(|message| message.stream) {
            let _ = self.connection.emitter.emit(
                &stream.key,
                observed.clone(),
                FrameEvent::End {
                    reason: EndReason::Complete,
                },
            );
        }
        if compressed
            && !self.takeover()
            && let Some(inflater) = &mut self.inflater
        {
            inflater.reset(false);
        }
    }

    /// Push unmasked compressed payload through the inflater, delivering what it yields.
    fn inflate(&mut self, mut input: &[u8], observed: &CaptureTime) {
        let mut output = std::mem::take(&mut self.output);
        while let Some(inflater) = &mut self.inflater {
            let (before_in, before_out) = (inflater.total_in(), inflater.total_out());
            let result = inflater.decompress(input, &mut output, FlushDecompress::None);
            let consumed = (inflater.total_in() - before_in) as usize;
            let produced = (inflater.total_out() - before_out) as usize;
            input = &input[consumed..];
            // Bytes written before a failure were decoded from valid input: keep them.
            self.deliver(&output[..produced], observed);
            match result {
                Err(_) => {
                    self.decode_failed(observed);
                    break;
                }
                Ok(Status::StreamEnd) => {
                    // A final DEFLATE block: this message decodes, but no takeover history
                    // can be continued faithfully past it.
                    if let Some(message) = &mut self.message {
                        message.inflating = false;
                    }
                    if self.takeover() {
                        self.history_lost = true;
                    }
                    break;
                }
                Ok(Status::Ok | Status::BufError) => {
                    let drained = input.is_empty() && produced < output.len();
                    if drained || (consumed == 0 && produced == 0) {
                        break;
                    }
                }
            }
        }
        self.output = output;
    }

    /// Keep the decoded prefix, end the message `DecodeError` and stop inflating it.
    fn decode_failed(&mut self, observed: &CaptureTime) {
        if self.takeover() {
            self.history_lost = true;
        }
        self.flush(observed);
        let lost = self.message.as_mut().and_then(|message| {
            message.inflating = false;
            message.stream.take()
        });
        if let Some(lost) = lost {
            self.fail(&lost.key, GapReason::DecodeFailed, observed);
        }
    }

    /// Append decoded bytes to the pending segment, emitting each full one.
    fn deliver(&mut self, mut bytes: &[u8], observed: &CaptureTime) {
        while !bytes.is_empty() {
            let captured = self
                .message
                .as_ref()
                .is_some_and(|message| message.stream.is_some());
            if !captured {
                self.pending.clear();
                return;
            }
            let take = (MAX_SEGMENT_BYTES - self.pending.len()).min(bytes.len());
            self.pending.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.pending.len() == MAX_SEGMENT_BYTES {
                self.flush(observed);
            }
        }
    }

    /// Emit the pending decoded bytes. A refusal abandons this message's capture.
    fn flush(&mut self, observed: &CaptureTime) {
        if self.pending.is_empty() {
            return;
        }
        let data = std::mem::take(&mut self.pending);
        let emitter = &self.connection.emitter;
        let Some(message) = &mut self.message else {
            return;
        };
        let Some(stream) = &mut message.stream else {
            return;
        };
        let len = data.len() as u64;
        let event = FrameEvent::Bytes {
            offset: stream.offset,
            data,
        };
        match emitter.emit(&stream.key, observed.clone(), event) {
            SubmitResult::Queued => stream.offset += len,
            SubmitResult::Disabled(reason) => {
                let key = stream.key.clone();
                message.stream = None;
                // No-ops when the epoch is gone; a stream refused locally is still closed.
                let _ = emitter.emit(
                    &key,
                    observed.clone(),
                    FrameEvent::Gap {
                        reason,
                        lost_bytes: None,
                    },
                );
                let _ = emitter.emit(
                    &key,
                    observed.clone(),
                    FrameEvent::End {
                        reason: EndReason::Interrupted,
                    },
                );
            }
        }
    }

    /// Record a protocol violation and stop capturing this direction.
    ///
    /// Inside a message this never calls `try_recover`: the message's open stream ends
    /// `DecodeError`; a message that never had a stream is recorded under its own index only
    /// if the emitter's current epoch is live; a message whose stream already ended or was
    /// lost with its epoch is recorded already. Between messages the violation is a stream of
    /// its own at the next index, the one boundary where recovery is allowed.
    fn violation(&mut self, observed: &CaptureTime) {
        self.flush(observed);
        match self.message.take() {
            Some(Message {
                stream: Some(lost), ..
            }) => self.fail(&lost.key, GapReason::DecodeFailed, observed),
            Some(Message {
                opened: false,
                index,
                ..
            }) => {
                if let Some(stream) = self.open_at(index, "protocol violation", observed) {
                    self.fail(&stream.key, GapReason::DecodeFailed, observed);
                }
            }
            Some(_) => {}
            None => self.record_loss("protocol violation", GapReason::DecodeFailed, observed),
        }
        self.pending.clear();
        self.active = false;
    }

    /// At a message boundary: a stream that exists only to say its content could not be
    /// decoded.
    fn record_loss(&mut self, what: &str, reason: GapReason, observed: &CaptureTime) {
        if let (_, Some(stream)) = self.open(what, observed) {
            self.fail(&stream.key, reason, observed);
        }
    }

    fn fail(&self, key: &StreamKey, reason: GapReason, observed: &CaptureTime) {
        let emitter = &self.connection.emitter;
        let _ = emitter.emit(
            key,
            observed.clone(),
            FrameEvent::Gap {
                reason,
                lost_bytes: None,
            },
        );
        let _ = emitter.emit(
            key,
            observed.clone(),
            FrameEvent::End {
                reason: EndReason::DecodeError,
            },
        );
    }

    /// At an observed message start: take the next message index, restore a failed epoch if
    /// the emitter can, and open the stream, all under the connection's index lock so `Open`
    /// order is index order. The index is taken even when no stream opens.
    fn open(&mut self, what: &str, observed: &CaptureTime) -> (u64, Option<Stream>) {
        let connection = self.connection;
        let mut next = connection
            .next_index
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let index = *next;
        *next += 1;
        if let SubmitResult::Disabled(_) = connection.emitter.try_recover(observed.clone()) {
            return (index, None);
        }
        (index, self.open_at(index, what, observed))
    }

    /// Open a stream at `index` in the emitter's current epoch, without recovery.
    fn open_at(&self, index: u64, what: &str, observed: &CaptureTime) -> Option<Stream> {
        let metadata = self.connection.metadata(self.side, index, what);
        let key = self
            .connection
            .emitter
            .open(self.side.source(), metadata, observed.clone())
            .ok()?;
        Some(Stream { key, offset: 0 })
    }

    /// Count a message start that capture no longer observes.
    fn skip_index(&self) {
        *self
            .connection
            .next_index
            .lock()
            .unwrap_or_else(PoisonError::into_inner) += 1;
    }

    /// End an open message `Interrupted`: EOF, a failed peer, or cancellation.
    fn interrupt(&mut self, observed: &CaptureTime) {
        self.flush(observed);
        self.pending.clear();
        if let Some(stream) = self.message.take().and_then(|message| message.stream) {
            let _ = self.connection.emitter.emit(
                &stream.key,
                observed.clone(),
                FrameEvent::End {
                    reason: EndReason::Interrupted,
                },
            );
        }
    }
}

impl Drop for Capture<'_> {
    /// A relay that fails or is cancelled mid-message still ends that message's stream.
    fn drop(&mut self) {
        let open = self
            .message
            .as_ref()
            .is_some_and(|message| message.stream.is_some());
        if open {
            self.interrupt(&now());
        }
    }
}

/// The relay's one clock read: the recording proxy's shared process-wide clock,
/// so HTTP and WebSocket frames of one producer compare on one monotonic base. The emitter
/// stamps `clock_id` with its producer.
fn now() -> CaptureTime {
    super::metadata::capture_time()
}

#[cfg(test)]
mod tests {
    use super::{Negotiated, negotiate};

    #[test]
    fn extension_negotiation_accepts_only_rfc_7692_parameters() {
        let deflate = |client_takeover, server_takeover| Negotiated::Deflate {
            client_takeover,
            server_takeover,
        };
        for (value, expected) in [
            (None, Negotiated::Plain),
            (Some(" "), Negotiated::Plain),
            (Some("permessage-deflate"), deflate(true, true)),
            (
                Some("permessage-deflate; client_no_context_takeover"),
                deflate(false, true),
            ),
            (
                Some("Permessage-Deflate;server_no_context_takeover; server_max_window_bits=10"),
                deflate(true, false),
            ),
            (
                Some("permessage-deflate; client_max_window_bits"),
                deflate(true, true),
            ),
            (
                Some("permessage-deflate; client_max_window_bits=\"12\""),
                deflate(true, true),
            ),
            (
                Some("permessage-deflate; client_max_window_bits=15; client_max_window_bits"),
                Negotiated::Unsupported,
            ),
            (
                Some("permessage-deflate; server_max_window_bits=7"),
                Negotiated::Unsupported,
            ),
            (
                Some("permessage-deflate; server_max_window_bits"),
                Negotiated::Unsupported,
            ),
            (
                Some("permessage-deflate; server_no_context_takeover=1"),
                Negotiated::Unsupported,
            ),
            (
                Some("permessage-deflate; client_max_window_bits=\"9"),
                Negotiated::Unsupported,
            ),
            (Some("permessage-deflate; mystery"), Negotiated::Unsupported),
            (Some("x-webkit-deflate-frame"), Negotiated::Unsupported),
            (
                Some("permessage-deflate, permessage-deflate"),
                Negotiated::Unsupported,
            ),
        ] {
            assert_eq!(negotiate(value), expected, "{value:?}");
        }
    }
}
