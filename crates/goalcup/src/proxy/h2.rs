//! The narrow passive HTTP/2 parser (PRD-03 sections 6, 5a and 11, guard `hpack_table`). Both TLS
//! legs carry the same frames, so it only observes: the client preface, then frames read from
//! their 9-byte headers in any split (RFC 9113 section 4.1). `HEADERS` plus `CONTINUATION` form
//! one header block, decoded by one HPACK decoder per direction (RFC 7541) that follows the
//! sender's table size updates; every block is decoded, so the table stays in step. A client
//! stream opens one `http_req`, its final server `HEADERS` one `http_resp` (meta `ver` "2" and
//! `h2`, pseudo-headers in wire order, `:path` without its query); a `1xx` is a note. `DATA`
//! payloads, padding stripped, go through http1's [`Body`] (decoding, the decoder guard, the
//! `wire` tail) as they arrive. `END_STREAM` ends a body, a second `HEADERS` carries trailers,
//! `RST_STREAM` and `GOAWAY` end streams interrupted with `h2_error`. `PRIORITY`, `PING`,
//! `SETTINGS`, `WINDOW_UPDATE` and unknown types are skipped by length.
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
use crate::proxy::decode::{Coding, Unsupported, parse_codings};
use crate::proxy::http1::{Body, Framing, Head, coding_meta, pairs, safe_path};
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

const DATA: u8 = 0;
const HEADERS: u8 = 1;
const RST_STREAM: u8 = 3;
const PUSH_PROMISE: u8 = 5;
const GOAWAY: u8 = 7;
const CONTINUATION: u8 = 9;
const END_STREAM: u8 = 0x1;
const END_HEADERS: u8 = 0x4;
const PADDED: u8 = 0x8;
const PRIORITY: u8 = 0x20;

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

enum Half {
    Waiting,
    Open(Box<Body>),
    /// An extended `CONNECT` side: its handle, the `DATA` bytes not recorded, its open time.
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
                h.gap(GapReason::H2Unsupported, at, Some(n + 1), None);
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
        }
    }

    /// The bytes fed and recorded nowhere, and the earliest time one of them came, else `t`.
    fn held(&self, t: Mono) -> (u64, Mono) {
        let at = self.sides.iter().flat_map(|s| {
            let hdr = (!s.hdr.is_empty()).then_some(s.hdr_at);
            [hdr, s.block.as_ref().map(|b| b.at)]
        });
        let since = at.flatten().fold(t, Mono::min);
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
            } else {
                self.kept += n as u64;
                if body && ty == DATA {
                    self.data(dir, t, id, piece);
                }
            }
            self.settle(dir, t)?;
        }
        Ok(())
    }

    /// A whole frame header: the frame to read, or a stop.
    fn open_frame(&mut self, d: usize, h: &[u8]) -> Result<(), Fault> {
        let len = u32::from_be_bytes([0, h[0], h[1], h[2]]) as usize;
        let (ty, flags) = (h[3], h[4]);
        let id = u32::from_be_bytes([h[5], h[6], h[7], h[8]]) & 0x7fff_ffff;
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
                let raw = b.raw;
                self.headers(dir, t, b)?;
                self.kept += raw;
            }
            RST_STREAM => {
                reset(self.streams.remove(&f.id).into_iter(), t, word(0));
            }
            GOAWAY => {
                let last = word(0) & 0x7fff_ffff;
                let above = self.streams.split_off(&last.saturating_add(1));
                reset(above.into_values(), t, word(4));
            }
            _ => {}
        }
        Ok(())
    }

    /// `END_STREAM`: one side of the stream ends complete.
    fn finish(&mut self, id: u32, dir: Dir, t: Mono) {
        let Some(s) = self.streams.get_mut(&id) else {
            return;
        };
        let half = std::mem::replace(&mut s.halves[dir as usize], Half::Done);
        half.end(t, EndReason::Complete, None, None);
        if s.halves.iter().all(|h| matches!(h, Half::Done)) {
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
            Some(Half::Raw(_, n, _)) => *n += piece.len() as u64,
            _ => {}
        }
    }

    /// A whole header block: opens that side of its stream, or is its trailers.
    fn headers(&mut self, dir: Dir, t: Mono, b: Block) -> Result<(), Fault> {
        let mut fields = Vec::new();
        let hpack = &mut self.sides[dir as usize].hpack;
        let lossy = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        let decoded = hpack.decode_with_cb(&b.bytes, |n, v| fields.push((lossy(&n), lossy(&v))));
        decoded.map_err(|e| match e {
            DecoderError::InvalidMaxDynamicSize => Fault::Table,
            _ => Fault::Unsupported,
        })?;
        let d = dir as usize;
        let s = self.streams.entry(b.id).or_insert_with(|| Stream {
            req: 0,
            tunnel: false,
            halves: [Half::Waiting, Half::Waiting],
        });
        match &mut s.halves[d] {
            Half::Open(body) => body.sink.trailers = fields,
            Half::Waiting if dir == Dir::Up => {
                let unsent = if self.unsent { b.raw } else { 0 };
                self.request(t, b.id, fields, unsent);
            }
            Half::Waiting => self.response(t, b.id, fields),
            _ => {}
        }
        if b.end {
            self.finish(b.id, dir, t);
        }
        Ok(())
    }

    fn request(&mut self, t: Mono, id: u32, mut fields: Vec<(String, String)>, unsent: u64) {
        let method = field(&fields, ":method").unwrap_or_default().to_owned();
        let tunnel = method == "CONNECT" && field(&fields, ":protocol").is_some();
        let path = field(&fields, ":path").map_or(Value::Null, safe_path);
        // Point 4: the query leaves the `:path` header too.
        for f in fields.iter_mut().filter(|f| f.0 == ":path") {
            f.1.truncate(f.1.find(['?', '#']).unwrap_or(f.1.len()));
        }
        let codings = codings(&fields);
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
        let half = self.half(h, tunnel, codings, unsent, t);
        if let Some(s) = self.streams.get_mut(&id) {
            (s.req, s.tunnel, s.halves[0]) = (req, tunnel, half);
        }
    }

    fn response(&mut self, t: Mono, id: u32, fields: Vec<(String, String)>) {
        let status = field(&fields, ":status").and_then(|s| s.parse::<u16>().ok());
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
        let (req, tunnel) = (s.req, s.tunnel);
        let codings = codings(&fields);
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
        let half = self.half(h, tunnel, codings, 0, t);
        if let Some(s) = self.streams.get_mut(&id) {
            s.halves[1] = half;
        }
    }

    fn half(
        &self,
        h: StreamHandle,
        tunnel: bool,
        codings: Result<Vec<Coding>, Unsupported>,
        unsent: u64,
        t: Mono,
    ) -> Half {
        if tunnel {
            return Half::Raw(h, 0, t);
        }
        let framing = Some(Framing::Close);
        let body = Body::open(h, framing, codings, &Head::default(), unsent, t, &self.rec);
        body.map_or(Half::Done, |b| Half::Open(Box::new(b)))
    }
}

fn field<'a>(fields: &'a [(String, String)], name: &str) -> Option<&'a str> {
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
fn codings(fields: &[(String, String)]) -> Result<Vec<Coding>, Unsupported> {
    fn named<'a>(fields: &'a [(String, String)], names: &[&str]) -> Vec<&'a str> {
        let named = fields.iter().filter(|f| names.contains(&f.0.as_str()));
        named.map(|f| f.1.trim()).collect()
    }
    let message = named(fields, &["grpc-encoding", "connect-content-encoding"]);
    if message.iter().any(|v| !v.eq_ignore_ascii_case("identity")) {
        // As decode.rs cuts tokens: at most 4, each at most 32 bytes on a character boundary.
        let cut = |v: &&str| -> String {
            let fits = v.char_indices().take_while(|(i, c)| i + c.len_utf8() <= 32);
            fits.map(|c| c.1).collect()
        };
        let codings = message.iter().take(4).map(cut).collect();
        return Err(Unsupported { codings });
    }
    parse_codings(&named(fields, &["content-encoding"]))
}

impl Parser for H2 {
    fn feed(&mut self, dir: Dir, t: Mono, bytes: &[u8]) -> Result<(), Stop> {
        self.seen += bytes.len() as u64;
        self.take(dir, t, bytes)
            .map_err(|fault| self.stop(t, fault))
    }

    /// Takes the client bytes never forwarded; streams still open end interrupted.
    fn end(mut self: Box<Self>, t: Mono, _: EndReason, _: bool, unsent: &[u8]) -> Option<Stop> {
        let mut late = None;
        if !self.stopped {
            self.seen += unsent.len() as u64;
            self.unsent = true;
            late = match self.take(Dir::Up, t, unsent) {
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
