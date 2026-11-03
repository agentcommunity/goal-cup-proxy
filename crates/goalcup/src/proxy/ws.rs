//! The passive WebSocket parser (PRD-03 section 8, PRD-01 section 4).
//!
//! The relay forwards every byte first; this parser only looks at a copy. One upgraded connection
//! is one `ws_up` and one `ws_down` stream. Every message (text, binary, close, ping, pong) goes out
//! through `StreamHandle::ws` as one or more pieces of at most 32 KiB, unmasked and inflated, with
//! `fin` on its last piece. Offsets run over the inflated bytes of every message of a direction.
//!
//! Framing is always tracked by length, so the relay is never touched. A frame that breaks RFC 6455
//! or RFC 7692 for its direction, or deflate data that does not decode, is a GAP `decode_failed`
//! for that message (unknown loss) and the next message parses. Under context takeover a lost
//! history cannot be reset mid-connection: every later compressed message of that direction is a
//! GAP too. Only a 64-bit length with its top bit set stops parsing for the connection (5a): the
//! length can no longer be trusted, so no later frame boundary can be.
//!
//! A control frame inside a fragmented data message is its own one-piece message; the data message
//! goes on whole with its own opcode (PRD-17 W8). Once the data message has handed over a piece,
//! such a control message (at most 125 bytes) is held and written right after the data message's
//! last piece, so the masker, which ends a message at each `fin`, sees the data message as one run
//! of bytes (PRD-05 section 7). A held message keeps its own read time, so its `t` can be earlier
//! than the piece written before it (the format's `dt` allows that).
//!
//! Inflation is raw DEFLATE with the fixed 15-bit window (`flate2` on `miniz_oxide`), which decodes
//! any smaller negotiated window. Memory per direction: the 32 KiB piece being filled, a 32 KiB
//! inflate output buffer, the inflater's 32 KiB window, the current frame header, and the held
//! control messages. Only the held messages are counted in PRD-02's guard (`Recorder::reserve`),
//! each until it is written.

use crate::pipeline::{
    EndReason, GapReason, Mono, OpenFlags, Recorder, Reserved, Sid, StreamHandle, StreamKind,
};
use bytes::Bytes;
use flate2::{Decompress, FlushDecompress, Status};
use serde_json::{Map, Value, json};

/// The largest piece handed to `StreamHandle::ws`.
const PIECE: usize = 32 * 1024;
/// The piece of payload unmasked at a time.
const UNMASK: usize = 16 * 1024;
/// RFC 7692 section 7.2.2: the sync-flush marker a sender strips and a receiver appends.
const DEFLATE_TAIL: [u8; 4] = [0x00, 0x00, 0xff, 0xff];

const OP_CONTINUATION: u8 = 0x0;
const OP_TEXT: u8 = 0x1;
const OP_BINARY: u8 = 0x2;
const OP_CLOSE: u8 = 0x8;
const OP_PING: u8 = 0x9;
const OP_PONG: u8 = 0xa;
const RSV1: u8 = 0x40;
const RSV2_RSV3: u8 = 0x30;

/// Which way bytes went: `Up` from the client (masked frames), `Down` from the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
}

/// The negotiated `permessage-deflate`, per sender.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deflate {
    /// The client keeps its compression history across messages.
    pub client_takeover: bool,
    /// The server keeps its compression history across messages.
    pub server_takeover: bool,
}

/// An extension answer the parser cannot follow. The caller stops parsing for the connection (5a).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unsupported;

/// Parsing stopped for the connection (5a): the caller writes the `parse_stopped` note and the
/// `conn` GAP. Holds the bytes of the chunk taken before the frame that stopped it, and the bytes
/// of that frame's header taken from earlier chunks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stopped(pub usize, pub usize);

/// Reads the `Sec-WebSocket-Extensions` of the upgrade request and of the `101`, each with its
/// header lines joined by `,`. No answer, or an empty one, is no extension. The answer must be at
/// most one `permessage-deflate` with only RFC 7692 parameters, each at most once, that the
/// request offered; anything else is `Unsupported`.
pub fn negotiate(
    request_ext: Option<&str>,
    response_ext: Option<&str>,
) -> Result<Option<Deflate>, Unsupported> {
    let listed = |value: Option<&str>| -> Vec<String> {
        value
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect()
    };
    let answer = listed(response_ext);
    let one = match answer.as_slice() {
        [] => return Ok(None),
        [one] => one,
        _ => return Err(Unsupported),
    };
    let deflate = permessage_deflate(one).ok_or(Unsupported)?;
    // A server may only answer an extension the client offered (RFC 6455 section 9.1).
    let offered = listed(request_ext).iter().any(|offer| {
        offer
            .split(';')
            .next()
            .is_some_and(|name| name.trim().eq_ignore_ascii_case("permessage-deflate"))
    });
    if offered {
        Ok(Some(deflate))
    } else {
        Err(Unsupported)
    }
}

/// One `permessage-deflate` extension with only RFC 7692 parameters, each at most once.
fn permessage_deflate(extension: &str) -> Option<Deflate> {
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
    Some(Deflate {
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

/// One upgraded connection: both directions' parsers and streams.
pub struct WsConn {
    up: Side,
    down: Side,
    stopped: bool,
}

impl WsConn {
    /// Opens `ws_up` then `ws_down` on connection `conn` for upgrade request `req`.
    pub fn open(rec: &Recorder, conn: Sid, req: Sid, deflate: Option<Deflate>) -> WsConn {
        let meta = json!({ "deflate": deflate.is_some() });
        let up = rec.open(
            StreamKind::WsUp,
            conn,
            req,
            0,
            OpenFlags::DECRYPTED,
            meta.clone(),
        );
        let down = rec.open(StreamKind::WsDown, conn, req, 0, OpenFlags::DECRYPTED, meta);
        WsConn {
            up: Side::new(up, rec, true, deflate.map(|d| d.client_takeover)),
            down: Side::new(down, rec, false, deflate.map(|d| d.server_takeover)),
            stopped: false,
        }
    }

    /// Takes relayed bytes of one direction, in any split, after they were forwarded.
    pub fn feed(&mut self, dir: Dir, t: Mono, bytes: &[u8]) -> Result<(), Stopped> {
        let side = match dir {
            Dir::Up => &mut self.up,
            Dir::Down => &mut self.down,
        };
        if let Err(stopped) = side.feed(t, bytes) {
            self.stopped = true;
            self.up.stop(t, dir == Dir::Up, None);
            self.down.stop(t, dir == Dir::Down, None);
            return Err(stopped);
        }
        Ok(())
    }

    /// Parsing stopped for the connection elsewhere (a parser panic): both streams get GAP
    /// `decode_failed` with `tail` and end `decode_error` at `end`.
    pub fn stop(&mut self, t: Mono, tail: Option<Value>) {
        if !std::mem::replace(&mut self.stopped, true) {
            self.up.stop(t, true, tail.clone());
            self.down.stop(t, true, tail);
        }
    }

    /// Ends both streams. `reason` is the connection's; a direction that ends inside a frame or a
    /// message ends `interrupted` instead of `complete`, and after a stop both end `decode_error`.
    /// `unsent_up`: client bytes never forwarded because the upstream had closed.
    pub fn end(self, t: Mono, reason: EndReason, unsent_up: u64) {
        let reason = if self.stopped {
            EndReason::DecodeError
        } else {
            reason
        };
        self.up.end(t, reason, unsent_up);
        self.down.end(t, reason, 0);
    }
}

/// A parsed frame header; `rsv` keeps the three reserved bits in their wire positions.
#[derive(Clone, Copy, Debug)]
struct Header {
    fin: bool,
    rsv: u8,
    opcode: u8,
    mask: Option<[u8; 4]>,
    len: u64,
}

/// XORs `bytes` with the masking `key`, `pos` being the payload offset of `bytes[0]`.
fn unmask(bytes: &mut [u8], key: [u8; 4], pos: u64) {
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte ^= key[((pos + i as u64) & 3) as usize];
    }
}

fn header_len(second: u8) -> usize {
    let extended = match second & 0x7f {
        126 => 2,
        127 => 8,
        _ => 0,
    };
    2 + extended + if second & 0x80 != 0 { 4 } else { 0 }
}

/// An incremental RFC 6455 frame header reader. The first two bytes always give the header's
/// size, so framing survives every protocol violation.
#[derive(Default)]
struct Framer {
    head: [u8; 14],
    have: usize,
}

impl Framer {
    /// Takes header bytes from `data`; the header once it is complete.
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

/// The frame whose payload is being read.
struct Frame {
    mask: Option<[u8; 4]>,
    pos: u64,
    len: u64,
    fin: bool,
    /// A control frame: its payload goes to `ctrl`, never to the data message.
    control: bool,
}

/// The data message a direction is inside.
struct Message {
    opcode: u8,
    compressed: bool,
    /// Compressed payload still goes through the inflater: false after the end of the DEFLATE
    /// stream (BFINAL).
    inflating: bool,
    /// When the message broke: its bytes from then on are a GAP `decode_failed`.
    broken: Option<Mono>,
    /// A piece without `fin` was handed over.
    handed: bool,
}

/// The control frame being read: at most 125 bytes, recorded as one message at its end.
struct Control {
    opcode: u8,
    t: Mono,
    /// The frame broke RFC 6455: a GAP at its end, no DATA.
    broken: bool,
    buf: Vec<u8>,
}

/// One direction: framing, unmasking, inflation and its stream.
struct Side {
    handle: StreamHandle,
    rec: Recorder,
    /// The client side: frames must be masked.
    client: bool,
    /// `Some(takeover)` when permessage-deflate was negotiated.
    takeover: Option<bool>,
    framer: Framer,
    frame: Option<Frame>,
    message: Option<Message>,
    control: Option<Control>,
    /// Control messages that ended inside a data message that already handed over a piece,
    /// written after that message; each counted in the guard while it waits.
    held: Vec<(Control, Reserved)>,
    inflater: Option<Decompress>,
    /// Takeover history lost: later compressed messages cannot be decoded faithfully.
    history_lost: bool,
    /// Inflated or plain bytes of the data message not yet handed over; at most `PIECE`.
    pending: Vec<u8>,
    /// The time the first byte of `pending` was read.
    pending_t: Mono,
    scratch: Vec<u8>,
    out: Vec<u8>,
    close_code: Option<u16>,
}

impl Side {
    fn new(handle: StreamHandle, rec: &Recorder, client: bool, takeover: Option<bool>) -> Side {
        Side {
            handle,
            rec: rec.clone(),
            client,
            takeover,
            framer: Framer::default(),
            frame: None,
            message: None,
            control: None,
            held: Vec::new(),
            inflater: takeover.map(|_| Decompress::new(false)),
            history_lost: false,
            pending: Vec::new(),
            pending_t: Mono::default(),
            scratch: Vec::new(),
            out: if takeover.is_some() {
                vec![0u8; PIECE]
            } else {
                Vec::new()
            },
            close_code: None,
        }
    }

    fn feed(&mut self, t: Mono, mut data: &[u8]) -> Result<(), Stopped> {
        let all = data.len();
        while !data.is_empty() {
            let at = all - data.len();
            if let Some(frame) = &mut self.frame {
                let take = (frame.len - frame.pos).min(data.len() as u64) as usize;
                let (mask, pos, control) = (frame.mask, frame.pos, frame.control);
                frame.pos += take as u64;
                let done = frame.pos == frame.len;
                self.payload(t, &data[..take], mask, pos, control);
                data = &data[take..];
                if done {
                    self.frame_end(t);
                }
            } else {
                // Header bytes buffered from earlier chunks: a header within one chunk is taken
                // whole or up to the chunk's end.
                let back = self.framer.have;
                let Some(header) = self.framer.take_header(&mut data) else {
                    continue;
                };
                if header.len >> 63 != 0 {
                    return Err(Stopped(at, back));
                }
                self.header(t, header);
                if header.len == 0 {
                    self.frame_end(t);
                }
            }
        }
        Ok(())
    }

    /// RFC 6455 sections 5.2 to 5.5 and RFC 7692 section 6, for one frame header.
    fn header(&mut self, t: Mono, h: Header) {
        let control = h.opcode & 0x8 != 0;
        let rsv1 = h.rsv & RSV1 != 0;
        let masking = h.mask.is_some() == self.client;
        let known = matches!(
            h.opcode,
            OP_CONTINUATION | OP_TEXT | OP_BINARY | OP_CLOSE | OP_PING | OP_PONG
        );
        let base = known && masking && h.rsv & RSV2_RSV3 == 0;
        self.frame = Some(Frame {
            mask: h.mask,
            pos: 0,
            len: h.len,
            fin: h.fin,
            control,
        });
        if control {
            self.control = Some(Control {
                opcode: h.opcode,
                t,
                broken: !(base && h.fin && h.len <= 125 && !rsv1),
                buf: Vec::new(),
            });
            return;
        }
        let starts = h.opcode != OP_CONTINUATION;
        if starts && self.message.is_some() {
            // A new message before the last one ended: the open one never finishes.
            self.mark_broken(t);
            self.finish_message(t);
        }
        if self.message.is_none() {
            let compressed = starts && rsv1 && self.takeover.is_some();
            let lost = compressed && self.history_lost;
            self.message = Some(Message {
                opcode: h.opcode,
                compressed,
                inflating: compressed && !lost,
                // A continuation with no message open belongs to no message.
                broken: (!starts || lost).then_some(t),
                handed: false,
            });
        }
        let rsv1_ok = if starts {
            !rsv1 || self.takeover.is_some()
        } else {
            !rsv1
        };
        if !(base && rsv1_ok) {
            self.mark_broken(t);
        }
    }

    fn payload(&mut self, t: Mono, data: &[u8], mask: Option<[u8; 4]>, pos: u64, control: bool) {
        if control {
            let Some(ctrl) = &mut self.control else {
                return;
            };
            if ctrl.broken {
                return;
            }
            let at = ctrl.buf.len();
            ctrl.buf.extend_from_slice(data);
            if let Some(key) = mask {
                unmask(&mut ctrl.buf[at..], key, pos);
            }
            return;
        }
        let mut scratch = std::mem::take(&mut self.scratch);
        let mut pos = pos;
        for piece in data.chunks(UNMASK) {
            let Some(message) = &self.message else {
                break;
            };
            if message.broken.is_some() || (message.compressed && !message.inflating) {
                break;
            }
            let compressed = message.compressed;
            // Server payload is not masked: no copy.
            let bytes = match mask {
                Some(key) => {
                    scratch.clear();
                    scratch.extend_from_slice(piece);
                    unmask(&mut scratch, key, pos);
                    &scratch[..]
                }
                None => piece,
            };
            pos += piece.len() as u64;
            if compressed {
                self.inflate(t, bytes);
            } else {
                self.deliver(t, bytes);
            }
        }
        self.scratch = scratch;
    }

    fn frame_end(&mut self, t: Mono) {
        let Some(frame) = self.frame.take() else {
            return;
        };
        if frame.control {
            if let Some(ctrl) = self.control.take() {
                if self.message.as_ref().is_some_and(|m| m.handed) {
                    let size = ctrl.buf.capacity() + std::mem::size_of::<Control>();
                    let counted = self.rec.reserve(size as u64);
                    self.held.push((ctrl, counted));
                } else {
                    self.control_out(ctrl, true);
                }
            }
            return;
        }
        if frame.fin {
            self.finish_message(t);
        }
    }

    /// Ends the data message: its last piece with `fin`, or its GAP when it broke.
    fn finish_message(&mut self, t: Mono) {
        let Some(message) = &self.message else {
            return;
        };
        if message.broken.is_none() && message.compressed && message.inflating {
            self.inflate(t, &DEFLATE_TAIL);
        }
        let Some(message) = self.message.take() else {
            return;
        };
        match message.broken {
            Some(start) => self.gap(start),
            None => {
                let at = if self.pending.is_empty() {
                    t
                } else {
                    self.pending_t
                };
                let bytes = Bytes::from(std::mem::take(&mut self.pending));
                self.handle.ws(at, message.opcode, true, bytes);
            }
        }
        self.release_held();
        if message.compressed {
            match self.takeover {
                Some(true) if message.broken.is_some() => self.history_lost = true,
                Some(false) => {
                    if let Some(inflater) = &mut self.inflater {
                        inflater.reset(false);
                    }
                }
                _ => {}
            }
        }
    }

    /// The data message broke at `t`: its decoded prefix is handed over, the rest is a GAP at its
    /// end. A compressed message under takeover takes the history with it.
    fn mark_broken(&mut self, t: Mono) {
        let Some(message) = &mut self.message else {
            return;
        };
        if message.broken.is_some() {
            return;
        }
        message.broken = Some(t);
        message.inflating = false;
        let opcode = message.opcode;
        if message.compressed && self.takeover == Some(true) {
            self.history_lost = true;
        }
        self.flush(opcode);
    }

    /// Hands over the pending bytes as a piece without `fin`.
    fn flush(&mut self, opcode: u8) {
        if self.pending.is_empty() {
            return;
        }
        if let Some(message) = &mut self.message {
            message.handed = true;
        }
        let bytes = Bytes::from(std::mem::take(&mut self.pending));
        self.handle.ws(self.pending_t, opcode, false, bytes);
    }

    /// Records a control message: its GAP when the frame broke RFC 6455, else its bytes, `fin`
    /// when the frame ended. A frame cut short with no bytes read writes nothing.
    fn control_out(&mut self, ctrl: Control, fin: bool) {
        if ctrl.broken {
            self.gap(ctrl.t);
        } else if fin || !ctrl.buf.is_empty() {
            if fin && ctrl.opcode == OP_CLOSE && ctrl.buf.len() >= 2 {
                self.close_code = Some(u16::from_be_bytes([ctrl.buf[0], ctrl.buf[1]]));
            }
            self.handle
                .ws(ctrl.t, ctrl.opcode, fin, Bytes::from(ctrl.buf));
        }
    }

    /// Writes the control messages held back while a data message was open.
    fn release_held(&mut self) {
        for (ctrl, _counted) in std::mem::take(&mut self.held) {
            self.control_out(ctrl, true);
        }
    }

    /// GAP `decode_failed` for the message that broke at `start` (unknown loss).
    fn gap(&mut self, start: Mono) {
        self.handle.gap(GapReason::DecodeFailed, start, None, None);
    }

    /// Adds decoded bytes to the pending piece, handing a full one over only when more follow, so
    /// a message's last piece always carries bytes unless the message is empty.
    fn deliver(&mut self, t: Mono, mut bytes: &[u8]) {
        let Some(opcode) = self.message.as_ref().map(|m| m.opcode) else {
            return;
        };
        while !bytes.is_empty() {
            if self.pending.len() == PIECE {
                self.flush(opcode);
            }
            if self.pending.is_empty() {
                self.pending_t = t;
            }
            let take = (PIECE - self.pending.len()).min(bytes.len());
            self.pending.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
        }
    }

    /// Pushes unmasked compressed payload through the inflater, delivering what it yields.
    fn inflate(&mut self, t: Mono, mut input: &[u8]) {
        let mut out = std::mem::take(&mut self.out);
        while let Some(inflater) = &mut self.inflater {
            let (before_in, before_out) = (inflater.total_in(), inflater.total_out());
            let result = inflater.decompress(input, &mut out, FlushDecompress::None);
            let consumed = (inflater.total_in() - before_in) as usize;
            let produced = (inflater.total_out() - before_out) as usize;
            input = &input[consumed..];
            // Bytes written before a failure were decoded from valid input: keep them.
            self.deliver(t, &out[..produced]);
            match result {
                Err(_) => {
                    self.mark_broken(t);
                    break;
                }
                Ok(Status::StreamEnd) => {
                    // A final DEFLATE block: this message decodes, but no takeover history can
                    // be continued past it.
                    if let Some(message) = &mut self.message {
                        message.inflating = false;
                    }
                    if self.takeover == Some(true) {
                        self.history_lost = true;
                    }
                    break;
                }
                Ok(Status::Ok | Status::BufError) => {
                    if input.is_empty() && produced < out.len() {
                        break;
                    }
                    if consumed == 0 && produced == 0 {
                        // No progress with input left: the rest of the message cannot decode.
                        self.mark_broken(t);
                        break;
                    }
                }
            }
        }
        self.out = out;
    }

    /// Parsing stopped for the connection at `t`: the open message, and on the faulting side the
    /// frame that broke it, get their GAP.
    fn stop(&mut self, t: Mono, faulted: bool, tail: Option<Value>) {
        let open = self.message.is_some() || self.control.is_some() || self.frame.is_some();
        if let Some(message) = &self.message {
            let opcode = message.opcode;
            self.flush(opcode);
        }
        let start = self.message.as_ref().and_then(|m| m.broken).unwrap_or(t);
        if faulted || open {
            self.handle.gap(GapReason::DecodeFailed, start, None, tail);
        }
        self.release_held();
        self.message = None;
        self.control = None;
        self.frame = None;
    }

    fn end(mut self, t: Mono, reason: EndReason, unsent: u64) {
        let mid = self.framer.have > 0 || self.frame.is_some() || self.message.is_some();
        // The data message's last bytes first, so they follow its earlier pieces.
        if let Some(message) = &self.message {
            match message.broken {
                Some(start) => self.gap(start),
                None => self.flush(message.opcode),
            }
        }
        self.message = None;
        self.release_held();
        if let Some(ctrl) = self.control.take() {
            self.control_out(ctrl, false);
        }
        let reason = if reason == EndReason::Complete && mid {
            EndReason::Interrupted
        } else {
            reason
        };
        let mut tail = Map::new();
        if let Some(code) = self.close_code {
            tail.insert("close_code".into(), code.into());
        }
        if unsent > 0 {
            tail.insert("unsent".into(), unsent.into());
        }
        let tail = (!tail.is_empty()).then_some(Value::Object(tail));
        self.handle.end(t, reason, tail);
    }
}
