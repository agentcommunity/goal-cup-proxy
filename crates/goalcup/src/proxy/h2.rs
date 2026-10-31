//! The narrow passive HTTP/2 parser (PRD-03 sections 6, 5a and 11, guard `hpack_table`). Both TLS
//! legs carry the same frames, so it only observes: the client preface, then frames read from
//! their 9-byte headers in any split (RFC 9113 section 4.1). `HEADERS` plus `CONTINUATION` form
//! one header block, decoded by one HPACK decoder per direction (RFC 7541) that follows the
//! sender's table size updates; every block is decoded, so the table stays in step. A client
//! stream opens one `http_req`, its final server `HEADERS` one `http_resp` (meta `ver` "2" and
//! `h2`, pseudo-headers in wire order, `:path` without its query); a `1xx` is a note. `DATA`
//! payloads, padding stripped, go through http1's [`Body`] (decoding, the decoder guard, the
//! `wire` tail) as they arrive. `END_STREAM` ends a body, a second `HEADERS` carries trailers,
//! `RST_STREAM` and a server's `GOAWAY` end streams interrupted with `h2_error` (a client's
//! `GOAWAY` names server streams, of which there are none). `PRIORITY`, `PING`, `WINDOW_UPDATE`,
//! unknown types and `SETTINGS` are skipped by length, but for a server's
//! `SETTINGS_ENABLE_CONNECT_PROTOCOL` (RFC 8441 section 3). Decoded fields past 64 KiB are not
//! stored and give GAP `memory_guard` `http_head`, as in HTTP/1.1; decoding goes on, and the
//! fields the parser reads ([`READ`]) are still kept for it, at most [`FIELD_MAX`] bytes of them.
//!
//! Server frames from a `HEADERS` for a client stream whose request is not parsed yet wait,
//! unparsed, until it is, so the response names its request; as in HTTP/1.1 they wait at most a
//! head's worth. Past that, or at the close, they are parsed on, and a response whose request
//! is still not parsed answers none (`req` 0). Its `DATA` is recorded as any response's, unless
//! the server set `SETTINGS_ENABLE_CONNECT_PROTOCOL` to 1: then its request may be an extended
//! `CONNECT`, so it records no `DATA` and gets GAP `h2_unsupported` counting it, as a tunnel side.
//!
//! `DATA` or a header block for a stream already ended (frames in flight after `RST_STREAM`) is
//! recorded in no stream: it counts as held, and the close names it in the `conn` GAP.
//!
//! Not followed, never silent: an extended `CONNECT` stream (RFC 8441) records no `DATA` and each
//! side gets GAP `h2_unsupported`; a `grpc-encoding` or `connect-content-encoding` other than
//! `identity` gives the body GAP `unsupported_encoding` (D14). A bad preface or malformed frame
//! (`decode_failed`), `PUSH_PROMISE`, an HPACK error or a header block past 16 MiB
//! (`h2_unsupported`), or a table size update past 1 MiB (`memory_guard`) stops parsing for the
//! connection (5a). A frame length cannot pass 16 MiB: its field has 24 bits.
//!
//! Client bytes never forwarded (taken at the close) are attributed per stream: the `DATA`
//! payloads and header block of a stream count as its `http_req` END tail `unsent`; frame
//! headers, padding and connection frames count on no stream.

use crate::pipeline::{
    EndReason, GapReason, Mono, OpenFlags, Recorder, Sid, StreamHandle, StreamKind,
};
use crate::proxy::decode::{Coding, Unsupported, parse_codings, unsupported};
use crate::proxy::http1::{
    Body, FIELD_MAX, Framing, HEAD_LIMIT, Head, coding_meta, head_gap, pairs, safe_path,
};
use crate::proxy::pump::{Parser, Stop};
use crate::proxy::ws::Dir;
use loona_hpack::decoder::{Decoder, DecoderError};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const PREFACE: &[u8; 24] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
/// A header block past this is not followed (5a `h2_unsupported`).
const BLOCK_MAX: usize = 16 << 20;
/// The `hpack_table` guard.
const TABLE_LIMIT: usize = 1 << 20;
/// The fields the parser reads, kept for it past the `http_head` guard.
const READ: [&str; 6] = [
    ":method",
    ":protocol",
    ":status",
    "content-encoding",
    "grpc-encoding",
    "connect-content-encoding",
];

const DATA: u8 = 0;
const HEADERS: u8 = 1;
const RST_STREAM: u8 = 3;
const SETTINGS: u8 = 4;
const PUSH_PROMISE: u8 = 5;
const GOAWAY: u8 = 7;
const CONTINUATION: u8 = 9;
const END_STREAM: u8 = 0x1;
const END_HEADERS: u8 = 0x4;
const PADDED: u8 = 0x8;
const PRIORITY: u8 = 0x20;
/// `SETTINGS_ENABLE_CONNECT_PROTOCOL` (RFC 8441 section 3).
const ENABLE_CONNECT: [u8; 2] = [0, 8];

/// Why parsing stopped for the connection.
#[derive(Clone, Copy)]
enum Fault {
    /// A bad preface or a frame that breaks RFC 9113.
    Frame,
    Unsupported,
    Table,
    Panic,
}

impl Fault {
    fn gap(self) -> (GapReason, Option<Value>) {
        match self {
            Fault::Frame => (GapReason::DecodeFailed, None),
            Fault::Unsupported => (GapReason::H2Unsupported, None),
            Fault::Table => (
                GapReason::MemoryGuard,
                Some(json!({"guard": "hpack_table", "limit": TABLE_LIMIT})),
            ),
            Fault::Panic => (GapReason::DecodeFailed, Some(json!({"panic": true}))),
        }
    }
}

/// One decrypted HTTP/2 connection: both directions' frame readers and the open streams.
pub struct H2 {
    rec: Recorder,
    conn: Sid,
    sides: [Side; 2],
    streams: BTreeMap<u32, Stream>,
    stopped: bool,
    /// Bytes fed, and those taken by a frame other than a header block's or by a decoded block
    /// (5a counts the difference).
    seen: u64,
    kept: u64,
    /// The client bytes being taken were never forwarded.
    unsent: bool,
    /// The highest client stream whose request was parsed.
    last_req: u32,
    /// Server bytes from a `HEADERS` whose request is not parsed yet, and when they came.
    wait: Vec<u8>,
    wait_at: Mono,
    /// The waiting bytes passed a head's worth, or the connection closed: a `HEADERS` read now
    /// opens its stream, answering no request, instead of waiting.
    orphan: bool,
    /// When the first frame for an ended stream came (its bytes are held).
    late: Option<Mono>,
    /// The server allows extended `CONNECT`; a server `SETTINGS` entry read in part.
    connect: bool,
    setting: Vec<u8>,
}

/// One direction's reader.
struct Side {
    hpack: Decoder<'static>,
    /// Preface bytes still to come.
    preface: usize,
    /// A frame header read in part, and when its first byte came.
    hdr: Vec<u8>,
    hdr_at: Mono,
    frame: Option<Frame>,
    block: Option<Block>,
}

/// The frame being read: `pre` holds the leading fields read before the payload (pad length,
/// priority, error codes), then `body` payload bytes and `pad` padding follow.
struct Frame {
    ty: u8,
    flags: u8,
    id: u32,
    len: usize,
    pre: Vec<u8>,
    need: usize,
    ready: bool,
    body: usize,
    pad: usize,
}

/// A header block being gathered until `END_HEADERS`; `raw` counts its frames' bytes.
struct Block {
    id: u32,
    end: bool,
    raw: u64,
    bytes: Vec<u8>,
    at: Mono,
}

struct Stream {
    req: Sid,
    /// Extended `CONNECT`: no `DATA` is recorded.
    tunnel: bool,
    /// By [`Dir`]: the request, the response.
    halves: [Half; 2],
}

impl Stream {
    /// A stream whose request is not parsed yet: its response half waits.
    fn new() -> Stream {
        let halves = [Half::Done, Half::Waiting];
        Stream {
            req: 0,
            tunnel: false,
            halves,
        }
    }
}

enum Half {
    Waiting,
    Open(Box<Body>),
    /// An extended `CONNECT` side, or a response that may answer one: its handle, the `DATA`
    /// bytes not recorded, its open time.
    Raw(StreamHandle, u64, Mono),
    Done,
}

impl Half {
    fn end(
        self,
        t: Mono,
        mut reason: EndReason,
        stop: Option<(GapReason, Option<Value>)>,
        extra: Option<(&str, Value)>,
    ) {
        match self {
            Half::Open(body) => (*body).end(t, reason, stop, extra),
            Half::Raw(mut h, n, at) => {
                h.gap(GapReason::H2Unsupported, at, Some(n), None);
                if let Some((gap, tail)) = stop {
                    h.gap(gap, t, None, tail);
                    reason = EndReason::DecodeError;
                }
                let mut tail = json!({"wire": n});
                if let Some((key, value)) = extra {
                    tail[key] = value;
                }
                h.end(t, reason, Some(tail));
            }
            Half::Waiting | Half::Done => {}
        }
    }
}

impl H2 {
    pub fn new(rec: &Recorder, conn: Sid) -> H2 {
        let side = |preface| {
            let mut hpack = Decoder::new();
            hpack.set_max_allowed_table_size(TABLE_LIMIT);
            Side {
                hpack,
                preface,
                hdr: Vec::new(),
                hdr_at: Mono::default(),
                frame: None,
                block: None,
            }
        };
        H2 {
            rec: rec.clone(),
            conn,
            sides: [side(PREFACE.len()), side(0)],
            streams: BTreeMap::new(),
            stopped: false,
            seen: 0,
            kept: 0,
            unsent: false,
            last_req: 0,
            wait: Vec::new(),
            wait_at: Mono::default(),
            orphan: false,
            late: None,
            connect: false,
            setting: Vec::new(),
        }
    }

    /// The bytes fed and recorded nowhere, and the earliest time one of them came, else `t`.
    fn held(&self, t: Mono) -> (u64, Mono) {
        let at = self.sides.iter().flat_map(|s| {
            let hdr = (!s.hdr.is_empty()).then_some(s.hdr_at);
            [hdr, s.block.as_ref().map(|b| b.at)]
        });
        let wait = (!self.wait.is_empty()).then_some(self.wait_at);
        let since = at.chain([wait, self.late]).flatten().fold(t, Mono::min);
        (self.seen.saturating_sub(self.kept), since)
    }

    /// 5a: every open stream gets the GAP and ends `decode_error`; nothing parses after this.
    fn stop(&mut self, t: Mono, fault: Fault) -> Stop {
        let (reason, tail) = fault.gap();
        for s in std::mem::take(&mut self.streams).into_values() {
            for half in s.halves {
                half.end(
                    t,
                    EndReason::DecodeError,
                    Some((reason, tail.clone())),
                    None,
                );
            }
        }
        self.stopped = true;
        let (held, since) = self.held(t);
        Stop {
            reason,
            tail,
            held,
            since,
            at_close: false,
        }
    }

    fn take(&mut self, dir: Dir, t: Mono, mut bytes: &[u8]) -> Result<(), Fault> {
        let d = dir as usize;
        if dir == Dir::Down && !self.wait.is_empty() {
            self.wait.extend_from_slice(bytes);
            if self.wait.len() <= HEAD_LIMIT {
                return Ok(());
            }
            return self.replay(t);
        }
        while !bytes.is_empty() {
            let side = &mut self.sides[d];
            if side.preface > 0 {
                let (at, n) = (PREFACE.len() - side.preface, side.preface.min(bytes.len()));
                if bytes[..n] != PREFACE[at..at + n] {
                    return Err(Fault::Frame);
                }
                side.preface -= n;
                self.kept += n as u64;
                bytes = &bytes[n..];
                continue;
            }
            let Some(f) = &mut side.frame else {
                if side.hdr.is_empty() {
                    side.hdr_at = t;
                }
                let n = (9 - side.hdr.len()).min(bytes.len());
                side.hdr.extend_from_slice(&bytes[..n]);
                bytes = &bytes[n..];
                if side.hdr.len() == 9 {
                    let h = std::mem::take(&mut side.hdr);
                    let id = sid(&h);
                    // A response before its request: wait for it (client streams are odd).
                    let early = h[3] == HEADERS && id % 2 == 1 && id > self.last_req;
                    if dir == Dir::Down && early && !self.streams.contains_key(&id) {
                        if !self.orphan {
                            (self.wait, self.wait_at) = ([&h[..], bytes].concat(), side.hdr_at);
                            return Ok(());
                        }
                        // Waited long enough: the stream opens now, answering no request.
                        self.streams.insert(id, Stream::new());
                    }
                    self.open_frame(d, &h)?;
                    self.settle(dir, t)?;
                }
                continue;
            };
            let (n, body) = if f.pre.len() < f.need {
                let n = (f.need - f.pre.len()).min(bytes.len());
                f.pre.extend_from_slice(&bytes[..n]);
                (n, false)
            } else if f.body > 0 {
                let n = f.body.min(bytes.len());
                f.body -= n;
                (n, true)
            } else {
                let n = f.pad.min(bytes.len());
                f.pad -= n;
                (n, false)
            };
            let (ty, id, piece) = (f.ty, f.id, &bytes[..n]);
            bytes = &bytes[n..];
            // A pending block means this frame is its HEADERS or CONTINUATION.
            if let Some(b) = side.block.as_mut() {
                b.raw += n as u64;
                if body {
                    b.bytes.extend_from_slice(piece);
                    if b.bytes.len() > BLOCK_MAX {
                        return Err(Fault::Unsupported);
                    }
                }
            } else if body && ty == DATA {
                self.data(dir, t, id, piece);
            } else {
                if body && ty == SETTINGS && dir == Dir::Down {
                    self.settings(piece);
                }
                self.kept += n as u64;
            }
            self.settle(dir, t)?;
        }
        Ok(())
    }

    /// Server frames that waited for a request now parsed go on.
    fn resume(&mut self, t: Mono) -> Result<(), Fault> {
        if self.wait.is_empty() || sid(&self.wait) > self.last_req {
            return Ok(());
        }
        let wait = std::mem::take(&mut self.wait);
        self.take(Dir::Down, t, &wait)
    }

    /// Server frames that waited a head's worth, or to the close: parsed on, no `HEADERS`
    /// among them waits, and a response whose request is not parsed answers none.
    fn replay(&mut self, t: Mono) -> Result<(), Fault> {
        let wait = std::mem::take(&mut self.wait);
        self.orphan = true;
        let fed = self.take(Dir::Down, t, &wait);
        self.orphan = false;
        fed
    }

    /// A whole frame header: the frame to read, or a stop.
    fn open_frame(&mut self, d: usize, h: &[u8]) -> Result<(), Fault> {
        let len = u32::from_be_bytes([0, h[0], h[1], h[2]]) as usize;
        let (ty, flags, id) = (h[3], h[4], sid(h));
        let side = &mut self.sides[d];
        // A header block admits only its own CONTINUATION frames (RFC 9113 section 6.10).
        if side.block.is_some() != (ty == CONTINUATION) {
            return Err(Fault::Frame);
        }
        match ty {
            PUSH_PROMISE => return Err(Fault::Unsupported),
            HEADERS => {
                let (end, at) = (flags & END_STREAM != 0, side.hdr_at);
                let bytes = Vec::new();
                side.block = Some(Block {
                    id,
                    end,
                    raw: 9,
                    bytes,
                    at,
                });
            }
            CONTINUATION => match side.block.as_mut() {
                Some(b) if b.id == id => b.raw += 9,
                _ => return Err(Fault::Frame),
            },
            SETTINGS => {
                self.setting.clear();
                self.kept += 9;
            }
            _ => self.kept += 9,
        }
        let padded = usize::from(flags & PADDED != 0);
        let need = match ty {
            DATA => padded,
            HEADERS => padded + 5 * usize::from(flags & PRIORITY != 0),
            RST_STREAM => 4,
            GOAWAY => 8,
            _ => 0,
        };
        if need > len {
            return Err(Fault::Frame);
        }
        side.frame = Some(Frame {
            ty,
            flags,
            id,
            len,
            pre: Vec::new(),
            need,
            ready: false,
            body: 0,
            pad: 0,
        });
        Ok(())
    }

    /// Once the leading fields are in, sizes the payload; a frame read whole is acted on.
    fn settle(&mut self, dir: Dir, t: Mono) -> Result<(), Fault> {
        let side = &mut self.sides[dir as usize];
        let Some(f) = &mut side.frame else {
            return Ok(());
        };
        if !f.ready && f.pre.len() == f.need {
            let padded = f.flags & PADDED != 0 && matches!(f.ty, DATA | HEADERS);
            let pad = if padded { usize::from(f.pre[0]) } else { 0 };
            if f.need + pad > f.len {
                return Err(Fault::Frame);
            }
            (f.body, f.pad, f.ready) = (f.len - f.need - pad, pad, true);
        }
        let Some(f) = side.frame.take_if(|f| f.ready && f.body == 0 && f.pad == 0) else {
            return Ok(());
        };
        let word = |i: usize| u32::from_be_bytes([0, 1, 2, 3].map(|k| f.pre[i + k]));
        match f.ty {
            DATA if f.flags & END_STREAM != 0 => self.finish(f.id, dir, t),
            HEADERS | CONTINUATION if f.flags & END_HEADERS != 0 => {
                let Some(b) = side.block.take() else {
                    return Ok(());
                };
                self.headers(dir, t, b)?;
            }
            RST_STREAM => {
                reset(self.streams.remove(&f.id).into_iter(), t, word(0));
            }
            // A client's GOAWAY names server streams (RFC 9113 section 6.8): none here.
            GOAWAY if dir == Dir::Down => {
                let last = word(0) & 0x7fff_ffff;
                let above = self.streams.split_off(&last.saturating_add(1));
                reset(above.into_values(), t, word(4));
            }
            _ => {}
        }
        Ok(())
    }

    /// Server `SETTINGS` payload bytes, in 6-byte entries (RFC 9113 section 6.5.1).
    fn settings(&mut self, piece: &[u8]) {
        for &byte in piece {
            self.setting.push(byte);
            if self.setting.len() == 6 {
                let entry = std::mem::take(&mut self.setting);
                if entry[..2] == ENABLE_CONNECT {
                    self.connect = entry[2..] == [0, 0, 0, 1];
                }
            }
        }
    }

    /// `END_STREAM`: one side of the stream ends complete. A stream whose request is not
    /// parsed yet stays, so that request opens no response half.
    fn finish(&mut self, id: u32, dir: Dir, t: Mono) {
        let Some(s) = self.streams.get_mut(&id) else {
            return;
        };
        let half = std::mem::replace(&mut s.halves[dir as usize], Half::Done);
        half.end(t, EndReason::Complete, None, None);
        if s.req != 0 && s.halves.iter().all(|h| matches!(h, Half::Done)) {
            self.streams.remove(&id);
        }
    }

    fn data(&mut self, dir: Dir, t: Mono, id: u32, piece: &[u8]) {
        let unsent = self.unsent && dir == Dir::Up;
        let half = self
            .streams
            .get_mut(&id)
            .map(|s| &mut s.halves[dir as usize]);
        match half {
            Some(Half::Open(body)) => {
                let _ = body.advance(t, piece);
                if unsent {
                    body.unsent += piece.len() as u64;
                }
            }
            Some(Half::Raw(_, n, ..)) => *n += piece.len() as u64,
            // Recorded in no stream: held, so the close's `conn` GAP counts it.
            _ => {
                self.late.get_or_insert(t);
                return;
            }
        }
        self.kept += piece.len() as u64;
    }

    /// A whole header block: opens that side of its stream, or is its trailers.
    /// The block's raw bytes count as kept unless it is for an ended stream.
    fn headers(&mut self, dir: Dir, t: Mono, b: Block) -> Result<(), Fault> {
        let (mut fields, mut read, mut stored, mut over) = (Vec::new(), Vec::new(), 0, 0);
        let mut kept_past = 0;
        let hpack = &mut self.sides[dir as usize].hpack;
        let lossy = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        // The `http_head` guard: fields past it are decoded, so the table stays in step, and
        // not stored; those the parser reads are kept for it, as HTTP/1.1 keeps its framing.
        let decoded = hpack.decode_with_cb(&b.bytes, |n, v| {
            let size = n.len() + v.len();
            let keep = over == 0 && stored + size <= HEAD_LIMIT;
            if READ.iter().any(|r| r.as_bytes() == &n[..]) {
                kept_past += if keep { 0 } else { size };
                read.push((lossy(&n), lossy(&v)));
            }
            if keep {
                stored += size;
                fields.push((lossy(&n), lossy(&v)));
            } else {
                over += size as u64;
            }
        });
        decoded.map_err(|e| match e {
            DecoderError::InvalidMaxDynamicSize => Fault::Table,
            _ => Fault::Unsupported,
        })?;
        // No valid head has that much of them (5a, as HTTP/1.1's framing fields).
        if kept_past > FIELD_MAX {
            return Err(Fault::Frame);
        }
        let head = Head::past_guard_at((over > 0).then_some((b.at, over)));
        let d = dir as usize;
        if dir == Dir::Up && b.id > self.last_req {
            self.last_req = b.id;
            self.streams.entry(b.id).or_insert_with(Stream::new).halves[0] = Half::Waiting;
        }
        let Some(s) = self.streams.get_mut(&b.id) else {
            // Frames in flight after the stream ended: recorded in no stream.
            self.late.get_or_insert(b.at);
            return Ok(());
        };
        match &mut s.halves[d] {
            Half::Open(body) => body.sink.set_trailers(fields, &head),
            Half::Waiting if dir == Dir::Up => {
                let unsent = if self.unsent { b.raw } else { 0 };
                self.request(t, b.id, fields, &read, &head, unsent);
            }
            Half::Waiting => self.response(t, b.id, fields, &read, &head),
            // Trailers on an extended `CONNECT` side: counted in its `h2_unsupported` GAP.
            Half::Raw(_, n, ..) => *n += b.raw,
            // A header block after that side ended: recorded in no stream, as a late frame.
            Half::Done => {
                self.late.get_or_insert(b.at);
                return Ok(());
            }
        }
        self.kept += b.raw;
        if b.end {
            self.finish(b.id, dir, t);
        }
        Ok(())
    }

    fn request(
        &mut self,
        t: Mono,
        id: u32,
        mut fields: Vec<Field>,
        read: &[Field],
        head: &Head,
        unsent: u64,
    ) {
        let method = field(read, ":method").unwrap_or_default().to_owned();
        let tunnel = method == "CONNECT" && field(read, ":protocol").is_some();
        let path = field(&fields, ":path").map_or(Value::Null, safe_path);
        // Point 4: the query leaves the `:path` header too.
        for f in fields.iter_mut().filter(|f| f.0 == ":path") {
            f.1.truncate(f.1.find(['?', '#']).unwrap_or(f.1.len()));
        }
        let codings = codings(read);
        let meta = json!({
            "method": method,
            "path": path,
            "ver": "2",
            "h2": id,
            "headers": pairs(&fields),
            "coding": coding_meta(&codings),
        });
        let flags = if method == "POST" {
            OpenFlags::DECRYPTED | OpenFlags::POST
        } else {
            OpenFlags::DECRYPTED
        };
        let h = self
            .rec
            .open(StreamKind::HttpReq, self.conn, 0, 0, flags, meta);
        let req = h.sid();
        let half = self.half(h, tunnel, codings, head, unsent, t);
        if let Some(s) = self.streams.get_mut(&id) {
            (s.req, s.tunnel, s.halves[0]) = (req, tunnel, half);
        }
    }

    fn response(&mut self, t: Mono, id: u32, fields: Vec<Field>, read: &[Field], head: &Head) {
        let status = field(read, ":status").and_then(|s| s.parse::<u16>().ok());
        if let Some(status @ 100..200) = status {
            self.rec.event(json!({
                "ev": "note",
                "what": "informational_response",
                "conn": self.conn,
                "status": status,
                "headers": pairs(&fields),
            }));
            return;
        }
        let Some(s) = self.streams.get(&id) else {
            return;
        };
        // A request not parsed yet may be an extended `CONNECT` if the server allows one.
        let (req, tunnel) = (s.req, s.tunnel || (s.req == 0 && self.connect));
        let codings = codings(read);
        let meta = json!({
            "status": status,
            "ver": "2",
            "h2": id,
            "headers": pairs(&fields),
            "coding": coding_meta(&codings),
            "decoded": codings.is_ok(),
        });
        let flags = OpenFlags::DECRYPTED;
        let h = self
            .rec
            .open(StreamKind::HttpResp, self.conn, req, 0, flags, meta);
        let half = self.half(h, tunnel, codings, head, 0, t);
        if let Some(s) = self.streams.get_mut(&id) {
            s.halves[1] = half;
        }
    }

    fn half(
        &self,
        mut h: StreamHandle,
        tunnel: bool,
        codings: Result<Vec<Coding>, Unsupported>,
        head: &Head,
        unsent: u64,
        t: Mono,
    ) -> Half {
        if tunnel {
            head_gap(&mut h, head);
            return Half::Raw(h, 0, t);
        }
        let framing = Some(Framing::Close);
        let body = Body::open(h, framing, codings, head, unsent, t, &self.rec);
        body.map_or(Half::Done, |b| Half::Open(Box::new(b)))
    }
}

type Field = (String, String);

/// A frame header's 31-bit stream id.
fn sid(h: &[u8]) -> u32 {
    u32::from_be_bytes([h[5], h[6], h[7], h[8]]) & 0x7fff_ffff
}

fn field<'a>(fields: &'a [Field], name: &str) -> Option<&'a str> {
    fields.iter().find(|f| f.0 == name).map(|f| f.1.as_str())
}

/// `RST_STREAM` or `GOAWAY`: the streams end interrupted with the error code.
fn reset(streams: impl Iterator<Item = Stream>, t: Mono, code: u32) {
    for s in streams {
        for half in s.halves {
            half.end(
                t,
                EndReason::Interrupted,
                None,
                Some(("h2_error", json!(code))),
            );
        }
    }
}

/// The content codings to undo; a message-level coding other than `identity` (gRPC, Connect)
/// cannot be undone as one body (D14).
fn codings(fields: &[Field]) -> Result<Vec<Coding>, Unsupported> {
    fn named<'a>(fields: &'a [Field], names: &[&str]) -> Vec<&'a str> {
        let named = fields.iter().filter(|f| names.contains(&f.0.as_str()));
        named.map(|f| f.1.trim()).collect()
    }
    let message = named(fields, &["grpc-encoding", "connect-content-encoding"]);
    if message.iter().any(|v| !v.eq_ignore_ascii_case("identity")) {
        return Err(unsupported(message.into_iter()));
    }
    parse_codings(&named(fields, &["content-encoding"]))
}

impl Parser for H2 {
    fn feed(&mut self, dir: Dir, t: Mono, bytes: &[u8]) -> Result<(), Stop> {
        self.seen += bytes.len() as u64;
        let fed = self.take(dir, t, bytes);
        fed.and_then(|()| self.resume(t))
            .map_err(|fault| self.stop(t, fault))
    }

    /// Takes the client bytes never forwarded; streams still open end interrupted.
    fn end(mut self: Box<Self>, t: Mono, _: EndReason, _: bool, unsent: &[u8]) -> Option<Stop> {
        let mut late = None;
        if !self.stopped {
            self.seen += unsent.len() as u64;
            self.unsent = true;
            let fed = self.take(Dir::Up, t, unsent).and_then(|()| self.resume(t));
            // Server frames still waiting for their request: parsed on, answering none.
            let fed = fed.and_then(|()| {
                if self.wait.is_empty() {
                    Ok(())
                } else {
                    self.replay(t)
                }
            });
            late = match fed {
                Err(fault) => Some(self.stop(t, fault)),
                // A frame or header block the close cut short: recorded in no stream.
                Ok(()) => Some(self.held(t))
                    .filter(|h| h.0 > 0)
                    .map(|(held, since)| Stop {
                        reason: GapReason::DecodeFailed,
                        tail: None,
                        held,
                        since,
                        at_close: true,
                    }),
            };
        }
        for s in std::mem::take(&mut self.streams).into_values() {
            for half in s.halves {
                half.end(t, EndReason::Interrupted, None, None);
            }
        }
        late
    }

    fn panicked(mut self: Box<Self>, t: Mono) -> Stop {
        self.stop(t, Fault::Panic)
    }
}
