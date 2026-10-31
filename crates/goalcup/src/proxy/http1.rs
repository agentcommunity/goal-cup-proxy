//! The passive HTTP/1.1 parser (PRD-03 sections 5, 5a, 7 and 8; guards `http_head` and
//! `upgrade_copy` of section 11). The relay forwards every byte first; this parser reads a copy,
//! one direction at a time in any split, and records every request whatever its method, path or
//! Host (rule 2): one `http_req` per request, one `http_resp` per final response, paired in order.
//! A `1xx` head is a note. Headers go raw into the meta; PRD-02 masks them.
//!
//! Bodies are de-chunked and decoded through [`BodyDecoder`], each decoded piece written as it is
//! made; [`Step`] says which GAP a body gets. Framing that cannot be followed stops parsing for
//! the connection (5a): a bad start line or field, a bad chunk size (more than 16 significant
//! digits included), a conflicting `Content-Length`, framing field lines kept past the head guard
//! that grow past [`FIELD_MAX`] in all (no valid head has that much), a `101` that is not a
//! WebSocket this parser can follow, a 2xx to an inner `CONNECT`. Every open stream then gets its
//! GAP and ends `decode_error`, and the [`Stop`] counts the relayed bytes recorded in no stream;
//! the relay writes the `parse_stopped` note and the `conn` GAP. Bytes held unrecorded at the close
//! (a head cut short) are a [`Stop`] too, marked `at_close`: the relay writes only the GAP. A start
//! line past the head guard keeps its first 64 KiB (method and path, or version and status) under
//! GAP `http_head`; a request then has no `ver`. Client bytes read and never forwarded (the
//! upstream had gone) are taken at the close like the rest, and each stream they reach counts
//! its share, head included, as END tail `unsent`.
//!
//! After a WebSocket upgrade request, client bytes are only copied (the `upgrade_copy` guard, then
//! 5a) until the answer: a `101` with the right `Sec-WebSocket-Accept` hands both directions to
//! [`WsConn`], the copy first; any other final answer replays the copy here. Server bytes that
//! overtake the feed of the request they answer wait, at most a head's worth, for it; past that,
//! or at the close, they are a response to no request (`req` 0), and so are later server bytes
//! until a request waits again. So a response that overtakes its request's feed in that window
//! answers none, and each later response on the connection pairs with the request before its
//! own; the bytes are all recorded, only the `req` link is off.

use crate::pipeline::{
    EndReason, GapReason, Mono, OpenFlags, Recorder, Sid, StreamHandle, StreamKind,
};
use crate::proxy::decode::{BodyDecoder, Coding, Step, Unsupported, parse_codings};
use crate::proxy::pump::{Parser, Stop};
use crate::proxy::ws::{self, Dir, WsConn};
use serde_json::{Value, json};
use std::collections::VecDeque;

/// The `http_head` guard: head bytes stored per head (and per chunked trailer section).
const HEAD_LIMIT: usize = 64 * 1024;
/// The `upgrade_copy` guard: client bytes copied while an upgrade waits for its answer.
const COPY_LIMIT: usize = 1 << 20;
/// Past the head guard, framing field lines longer than this in all cannot be followed (5a).
const FIELD_MAX: usize = 4 * 1024;
/// Past the head guard, a field line whose name has not ended within this many bytes is not a
/// field framing needs, so the rest of it is skipped unread.
const NAME_MAX: usize = 64;
/// The fields read past the head guard: what framing and the WebSocket handshake need.
const FRAMING: [&str; 9] = [
    "content-length",
    "transfer-encoding",
    "content-encoding",
    "connection",
    "upgrade",
    "sec-websocket-key",
    "sec-websocket-version",
    "sec-websocket-accept",
    "sec-websocket-extensions",
];

/// Why parsing stopped for the connection.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Fault {
    /// Framing this parser cannot follow, or the WebSocket parser stopped (it wrote its GAPs).
    Framing,
    /// The `upgrade_copy` guard.
    Copy,
    /// A parser panic (rule 4).
    Panic,
}

impl Fault {
    fn gap(self) -> (GapReason, Option<Value>) {
        match self {
            Fault::Framing => (GapReason::DecodeFailed, None),
            Fault::Copy => (
                GapReason::MemoryGuard,
                Some(json!({"guard": "upgrade_copy", "limit": COPY_LIMIT})),
            ),
            Fault::Panic => (GapReason::DecodeFailed, Some(json!({"panic": true}))),
        }
    }
}

/// One decrypted HTTP/1.1 connection: both directions' parsers and the open streams.
pub struct Http1 {
    rec: Recorder,
    conn: Sid,
    up: Side,
    down: Side,
    /// Requests whose final response has not started, in order.
    queue: VecDeque<Asked>,
    /// Client bytes after a WebSocket upgrade request, held until its answer, and the request's
    /// time.
    copy: Option<(Mono, Vec<u8>)>,
    /// Server bytes that came before the request they answer, and when the first came.
    early: (Mono, Vec<u8>),
    /// The response being parsed answers no request.
    orphan: bool,
    ws: Option<WsConn>,
    stopped: bool,
    /// Bytes fed, and those a stream or note accounts for: heads, bodies on the wire, WebSocket
    /// bytes (5a counts the difference).
    seen: u64,
    kept: u64,
    /// Client bytes being taken were never forwarded (fed at the close); each stream counts its
    /// share as END tail `unsent`: the head being read, a body, the WebSocket's.
    unsent: bool,
    head_unsent: u64,
    ws_unsent: u64,
}

/// One direction: the head being read, or the body of the open stream.
struct Side {
    head: Head,
    body: Option<Body>,
}

/// A request waiting for its final response.
#[derive(Default)]
struct Asked {
    sid: Sid,
    head: bool,
    connect: bool,
    /// A WebSocket upgrade: the key and the offered extensions.
    ws: Option<(String, Option<String>)>,
}

impl Http1 {
    pub fn new(rec: &Recorder, conn: Sid) -> Http1 {
        let side = || Side {
            head: Head::new(true),
            body: None,
        };
        Http1 {
            rec: rec.clone(),
            conn,
            up: side(),
            down: side(),
            queue: VecDeque::new(),
            copy: None,
            early: (Mono::default(), Vec::new()),
            orphan: false,
            ws: None,
            stopped: false,
            seen: 0,
            kept: 0,
            unsent: false,
            head_unsent: 0,
            ws_unsent: 0,
        }
    }

    /// The bytes fed and recorded in no stream (blank lines before a start line aside), and the
    /// earliest time one of those still held was taken, else `t`.
    fn held(&self, t: Mono) -> (u64, Mono) {
        let blank = self.up.head.blank + self.down.head.blank;
        let early = (!self.early.1.is_empty()).then_some(self.early.0);
        let copy = self.copy.as_ref().map(|c| c.0);
        let at = [self.up.head.at, self.down.head.at, early, copy];
        let since = at.into_iter().flatten().fold(t, Mono::min);
        (self.seen.saturating_sub(self.kept + blank), since)
    }

    /// 5a: every open stream gets the GAP and ends `decode_error`; nothing parses after this.
    fn stop(&mut self, t: Mono, fault: Fault) -> Stop {
        let (reason, tail) = fault.gap();
        for body in [self.up.body.take(), self.down.body.take()]
            .into_iter()
            .flatten()
        {
            body.end(t, EndReason::DecodeError, Some((reason, tail.clone())), None);
        }
        if let Some(ws) = &mut self.ws {
            ws.stop(t, tail.clone());
        }
        let (held, since) = self.held(t);
        self.copy = None;
        self.early.1 = Vec::new();
        self.queue.clear();
        self.stopped = true;
        Stop {
            reason,
            tail,
            held,
            since,
            at_close: false,
        }
    }

    /// Feeds the WebSocket parser. On its stop it wrote its GAPs, and only the bytes it took before
    /// the frame that stopped it count as kept.
    fn ws_feed(&mut self, dir: Dir, t: Mono, bytes: &[u8]) -> Result<(), Fault> {
        let Some(ws) = &mut self.ws else {
            return Ok(());
        };
        let fed = ws.feed(dir, t, bytes);
        let took = match fed {
            Ok(()) => bytes.len(),
            // The bad frame's header bytes from earlier chunks were counted kept then.
            Err(ws::Stopped(n, back)) => {
                self.kept = self.kept.saturating_sub(back as u64);
                n
            }
        };
        self.kept += took as u64;
        if dir == Dir::Up && self.unsent {
            self.ws_unsent += took as u64;
        }
        fed.map_err(|_| Fault::Framing)
    }

    fn take(&mut self, dir: Dir, t: Mono, mut bytes: &[u8]) -> Result<(), Fault> {
        let unsent = self.unsent && dir == Dir::Up;
        while !bytes.is_empty() {
            if self.ws.is_some() {
                return self.ws_feed(dir, t, bytes);
            }
            if dir == Dir::Up
                && let Some((_, copy)) = &mut self.copy
            {
                if copy.len() + bytes.len() > COPY_LIMIT {
                    return Err(Fault::Copy);
                }
                copy.extend_from_slice(bytes);
                return Ok(());
            }
            if dir == Dir::Down && self.down.body.is_none() && self.queue.is_empty() && !self.orphan
            {
                let early = &mut self.early;
                if early.1.len() + bytes.len() <= HEAD_LIMIT {
                    if early.1.is_empty() {
                        early.0 = t;
                    }
                    early.1.extend_from_slice(bytes);
                    return Ok(());
                }
                // A head's worth and still no request: these bytes answer none.
                self.orphan = true;
                let mut all = std::mem::take(&mut early.1);
                all.extend_from_slice(bytes);
                return self.take(dir, t, &all);
            }
            let side = match dir {
                Dir::Up => &mut self.up,
                Dir::Down => &mut self.down,
            };
            let n = if let Some(body) = &mut side.body {
                let before = body.raw;
                let step = body.advance(t, bytes);
                self.kept += body.raw - before;
                if unsent {
                    body.unsent += body.raw - before;
                }
                let (n, done) = step?;
                if done && let Some(body) = side.body.take() {
                    body.end(t, EndReason::Complete, None, None);
                }
                n
            } else {
                let (n, done) = side.head.push(t, bytes)?;
                if unsent {
                    self.head_unsent += n as u64;
                }
                if done {
                    let head = std::mem::replace(&mut side.head, Head::new(true));
                    match dir {
                        Dir::Up => self.request(t, head)?,
                        Dir::Down => self.response(t, head)?,
                    }
                }
                n
            };
            bytes = &bytes[n..];
        }
        Ok(())
    }

    fn request(&mut self, t: Mono, head: Head) -> Result<(), Fault> {
        let (method, target, ver) = request_line(&head.start, head.cut).ok_or(Fault::Framing)?;
        let f = &head.frame;
        let framing = match f.chunked() {
            Some(true) => Some(Framing::Chunked(Chunk::Size(SizeLine::default()))),
            // Another final transfer coding cannot frame a request (RFC 9112 section 6.3).
            Some(false) => return Err(Fault::Framing),
            None => f.length()?.filter(|&n| n > 0).map(Framing::Length),
        };
        let keys: Vec<&str> = f.values("sec-websocket-key").collect();
        let ws = match keys.as_slice() {
            [key]
                if method == "GET"
                    && ver == Some("1.1")
                    && framing.is_none()
                    && f.websocket()
                    && f.values("sec-websocket-version").eq(["13"]) =>
            {
                Some((key.to_string(), f.extensions()))
            }
            _ => None,
        };
        let codings = f.codings();
        let meta = json!({
            "method": method,
            "path": safe_path(target),
            "ver": ver,
            "headers": pairs(&head.fields),
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
        self.kept += head.size;
        if ws.is_some() {
            self.copy = Some((t, Vec::new()));
        }
        self.queue.push_back(Asked {
            sid: h.sid(),
            head: method == "HEAD",
            connect: method == "CONNECT",
            ws,
        });
        let unsent = std::mem::take(&mut self.head_unsent);
        self.up.body = Body::open(h, framing, codings, &head, unsent, t, &self.rec);
        if !self.early.1.is_empty() {
            let early = std::mem::take(&mut self.early.1);
            self.take(Dir::Down, t, &early)?;
        }
        Ok(())
    }

    fn response(&mut self, t: Mono, head: Head) -> Result<(), Fault> {
        let (status, ver) = status_line(&head.start).ok_or(Fault::Framing)?;
        let f = &head.frame;
        if (100..200).contains(&status) {
            self.rec.event(json!({
                "ev": "note",
                "what": "informational_response",
                "conn": self.conn,
                "status": status,
                "headers": pairs(&head.fields),
            }));
            self.kept += head.size;
            if status != 101 {
                return Ok(());
            }
            // The protocol switched: only a WebSocket this parser can follow goes on.
            let asked = self.queue.pop_front().unwrap_or_default();
            let Some((key, offered)) = asked.ws else {
                return Err(Fault::Framing);
            };
            let accepted = f.websocket()
                && f.values("sec-websocket-accept")
                    .eq([accept_key(&key).as_str()]);
            if !accepted {
                return Err(Fault::Framing);
            }
            let extensions = f.extensions();
            let deflate = ws::negotiate(offered.as_deref(), extensions.as_deref())
                .map_err(|_| Fault::Framing)?;
            self.ws = Some(WsConn::open(&self.rec, self.conn, asked.sid, deflate));
            let copy = self.copy.take().map(|c| c.1).unwrap_or_default();
            return self.ws_feed(Dir::Up, t, &copy);
        }
        // A response to no request: later server bytes answer none either until a request waits.
        let asked = self.queue.pop_front();
        self.orphan = asked.is_none();
        let asked = asked.unwrap_or_default();
        let framing = if asked.head || status == 204 || status == 304 {
            None
        } else if asked.connect && (200..300).contains(&status) {
            // A tunnel inside the connection: not HTTP after this head.
            return Err(Fault::Framing);
        } else {
            match (f.chunked(), f.length()?) {
                (Some(true), _) => Some(Framing::Chunked(Chunk::Size(SizeLine::default()))),
                (Some(false), _) | (None, None) => Some(Framing::Close),
                (None, Some(0)) => None,
                (None, Some(n)) => Some(Framing::Length(n)),
            }
        };
        let codings = f.codings();
        // `decoded` is what the codings allow at open; a body the decoder guard later refuses
        // says so in its `decoder_window` GAP (lost 0, tail `wire`), which overrides it.
        let meta = json!({
            "status": status,
            "ver": ver,
            "headers": pairs(&head.fields),
            "coding": coding_meta(&codings),
            "decoded": codings.is_ok(),
        });
        let h = self.rec.open(
            StreamKind::HttpResp,
            self.conn,
            asked.sid,
            0,
            OpenFlags::DECRYPTED,
            meta,
        );
        self.kept += head.size;
        self.down.body = Body::open(h, framing, codings, &head, 0, t, &self.rec);
        // The upgrade was refused: the copied client bytes are HTTP.
        if asked.ws.is_some()
            && let Some((_, copy)) = self.copy.take()
        {
            self.take(Dir::Up, t, &copy)?;
        }
        Ok(())
    }
}

impl Parser for Http1 {
    /// Relayed bytes of one direction in any split, after they were forwarded.
    fn feed(&mut self, dir: Dir, t: Mono, bytes: &[u8]) -> Result<(), Stop> {
        self.seen += bytes.len() as u64;
        self.take(dir, t, bytes)
            .map_err(|fault| self.stop(t, fault))
    }

    /// Takes the client bytes never forwarded, then ends open streams (interrupted when inside
    /// a message, complete otherwise; decode_error after a stop).
    fn end(
        mut self: Box<Self>,
        t: Mono,
        reason: EndReason,
        down_cut: bool,
        unsent: &[u8],
    ) -> Option<Stop> {
        let since = self.held(t).1;
        let mut late = None;
        if !self.stopped {
            self.seen += unsent.len() as u64;
            // An upgrade never answered: its copy is HTTP after all, and came before the bytes
            // never forwarded. Server bytes still waiting for a request answer none.
            let copy = self.copy.take().map(|c| c.1).unwrap_or_default();
            let mut fed = self.take(Dir::Up, t, &copy);
            self.unsent = true;
            fed = fed.and_then(|()| self.take(Dir::Up, t, unsent));
            self.unsent = false;
            if fed.is_ok() && self.ws.is_none() && !self.early.1.is_empty() {
                self.orphan = true;
                let early = std::mem::take(&mut self.early.1);
                fed = self.take(Dir::Down, t, &early);
            }
            late = match fed {
                Err(fault) => Some(self.stop(t, fault)),
                // A head the close cut short: recorded in no stream.
                Ok(()) => Some(self.held(t))
                    .filter(|h| h.0 > 0)
                    .map(|(held, _)| Stop {
                        reason: GapReason::DecodeFailed,
                        tail: None,
                        held,
                        since,
                        at_close: true,
                    }),
            };
        }
        if let Some(ws) = self.ws.take() {
            ws.end(t, reason, self.ws_unsent);
        }
        if let Some(body) = self.up.body.take() {
            body.end(t, EndReason::Interrupted, None, None);
        }
        if let Some(body) = self.down.body.take() {
            // Only the server's close ends this body, and a cut close may come early.
            let close = matches!(body.framing, Framing::Close);
            let reason = if close && !down_cut {
                reason
            } else {
                EndReason::Interrupted
            };
            body.end(t, reason, None, None);
        }
        late.map(|s| Stop {
            since: s.since.min(since),
            ..s
        })
    }

    fn panicked(mut self: Box<Self>, t: Mono) -> Stop {
        let stop = self.stop(t, Fault::Panic);
        let _ = self.end(t, EndReason::DecodeError, false, &[]);
        stop
    }
}

/// `codings` for the OPEN meta: the list undone, or null when none is.
pub(crate) fn coding_meta(codings: &Result<Vec<Coding>, Unsupported>) -> Value {
    match codings {
        Ok(list) if !list.is_empty() => json!(list.iter().map(|c| c.name()).collect::<Vec<_>>()),
        _ => Value::Null,
    }
}

pub(crate) fn pairs(fields: &[(String, String)]) -> Value {
    json!(fields.iter().map(|(n, v)| [n, v]).collect::<Vec<_>>())
}

/// The `Sec-WebSocket-Accept` value for a request's `Sec-WebSocket-Key` (RFC 6455 section
/// 4.2.2): base64 of the SHA-1 of the key followed by the protocol's fixed GUID.
fn accept_key(key: &str) -> String {
    use base64::Engine as _;
    use sha1::{Digest, Sha1};
    const GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let mut hash = Sha1::new();
    hash.update(key.as_bytes());
    hash.update(GUID);
    base64::engine::general_purpose::STANDARD.encode(hash.finalize())
}

/// A request target reduced to its path without query and fragment.
/// Origin form keeps `/p`, absolute form keeps the path after the authority; asterisk and
/// authority forms have no path (null).
pub(crate) fn safe_path(target: &str) -> Value {
    let rest = if target.starts_with('/') {
        target
    } else {
        let lower = target.get(..8).map(str::to_ascii_lowercase);
        let skip = match lower.as_deref() {
            Some(s) if s.starts_with("http://") => 7,
            Some("https://") => 8,
            _ => return Value::Null,
        };
        let after = &target[skip..];
        &after[after.find(['/', '?', '#']).unwrap_or(after.len())..]
    };
    let path = &rest[..rest.find(['?', '#']).unwrap_or(rest.len())];
    json!(if path.is_empty() { "/" } else { path })
}

fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// The meta `ver` of an HTTP/1 version.
fn version(v: &str) -> Option<&'static str> {
    match v {
        "HTTP/1.1" => Some("1.1"),
        "HTTP/1.0" => Some("1.0"),
        _ => None,
    }
}

/// Method, target and `ver`. A line `cut` at the head guard has lost its version (`None`).
fn request_line(line: &[u8], cut: bool) -> Option<(&str, &str, Option<&'static str>)> {
    let line = std::str::from_utf8(line).ok()?;
    let mut parts = line.split(' ');
    let (method, target) = (parts.next()?, parts.next()?);
    let ver = if cut {
        None
    } else {
        Some(version(parts.next()?)?)
    };
    let valid = (cut || parts.next().is_none())
        && is_token(method)
        && !target.is_empty()
        && target.bytes().all(|b| b.is_ascii_graphic());
    valid.then_some((method, target, ver))
}

fn status_line(line: &[u8]) -> Option<(u16, &'static str)> {
    let line = std::str::from_utf8(line).ok()?;
    let mut parts = line.splitn(3, ' ');
    let (ver, code) = (version(parts.next()?)?, parts.next()?);
    let valid = code.len() == 3 && code.bytes().all(|b| b.is_ascii_digit());
    Some((code.parse().ok().filter(|_| valid)?, ver))
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
    &value[start..end]
}

fn strip_eol(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// A head, or a chunked trailer section, read line by line in any split. Empty lines before
/// the start line are skipped and bare LF line ends are taken (RFC 9112 section 2.2).
#[derive(Default)]
pub(crate) struct Head {
    /// The start line is still to come (never, for trailers).
    want_start: bool,
    start: Vec<u8>,
    /// The start line passed the guard and keeps only its first [`HEAD_LIMIT`] bytes.
    cut: bool,
    line: Vec<u8>,
    /// The rest of the current line is not kept.
    skip: bool,
    /// Field lines stored for the meta, raw, in wire order.
    fields: Vec<(String, String)>,
    frame: Fields,
    stored: usize,
    /// Once past the `http_head` guard: when, and the bytes not stored.
    over: Option<(Mono, u64)>,
    /// Bytes of framing field lines read but not stored (at most [`FIELD_MAX`]).
    framed: usize,
    /// Bytes taken, the blank lines before the start line among them, and when the first byte
    /// after those came.
    size: u64,
    blank: u64,
    at: Option<Mono>,
}

impl Head {
    fn new(want_start: bool) -> Head {
        Head {
            want_start,
            ..Head::default()
        }
    }

    /// Consumes bytes up to the end of the head: `(consumed, done)`.
    fn push(&mut self, t: Mono, input: &[u8]) -> Result<(usize, bool), Fault> {
        let mut pos = 0;
        while pos < input.len() {
            let rest = &input[pos..];
            let (piece, ends) = match rest.iter().position(|&b| b == b'\n') {
                Some(i) => (&rest[..=i], true),
                None => (rest, false),
            };
            pos += piece.len();
            self.size += piece.len() as u64;
            self.at.get_or_insert(t);
            if self.skip {
                self.not_stored(t, piece.len());
            } else {
                self.line.extend_from_slice(piece);
                if self.stored + self.line.len() > HEAD_LIMIT && (!ends || self.want_start) {
                    self.past_guard(t)?;
                }
            }
            if ends && self.finish_line(t)? {
                return Ok((pos, true));
            }
        }
        Ok((pos, false))
    }

    fn not_stored(&mut self, t: Mono, n: usize) {
        let over = self.over.get_or_insert((t, 0));
        over.1 += n as u64;
    }

    /// A line has passed the guard. A start line keeps its first [`HEAD_LIMIT`] bytes; a field
    /// line is kept only while it can be a framing field, at most [`FIELD_MAX`] bytes.
    fn past_guard(&mut self, t: Mono) -> Result<(), Fault> {
        if self.want_start {
            let rest = self.line.len().saturating_sub(HEAD_LIMIT);
            self.line.truncate(HEAD_LIMIT);
            self.not_stored(t, rest);
            (self.skip, self.cut) = (true, true);
            return Ok(());
        }
        let framing = match self.line.iter().position(|&b| b == b':') {
            Some(colon) => {
                let name = String::from_utf8_lossy(&self.line[..colon]).to_ascii_lowercase();
                FRAMING.contains(&name.as_str())
            }
            None => self.line.len() <= NAME_MAX,
        };
        if framing && self.framed + self.line.len() > FIELD_MAX {
            return Err(Fault::Framing);
        }
        if !framing {
            self.skip = true;
            let n = std::mem::take(&mut self.line).len();
            self.not_stored(t, n);
        }
        Ok(())
    }

    /// One complete line: `Ok(true)` at the empty line that ends the head.
    fn finish_line(&mut self, t: Mono) -> Result<bool, Fault> {
        let skipped = std::mem::take(&mut self.skip);
        let raw = std::mem::take(&mut self.line);
        if self.want_start {
            // A cut line was stored without its end.
            let line = if skipped { &raw[..] } else { strip_eol(&raw) };
            if line.is_empty() {
                (self.blank, self.at) = (self.size, None);
            } else {
                self.want_start = false;
                self.stored += raw.len();
                self.start = line.to_vec();
            }
            return Ok(false);
        }
        let line = strip_eol(&raw);
        if skipped {
            return Ok(false);
        }
        if line.is_empty() {
            return Ok(true);
        }
        // Obsolete line folding and a bad name cannot be framed (RFC 9112 section 5).
        if matches!(line[0], b' ' | b'\t') {
            return Err(Fault::Framing);
        }
        let colon = line.iter().position(|&b| b == b':').ok_or(Fault::Framing)?;
        let name = std::str::from_utf8(&line[..colon])
            .ok()
            .filter(|n| is_token(n))
            .ok_or(Fault::Framing)?;
        let value = trim_ows(&line[colon + 1..]);
        let lower = name.to_ascii_lowercase();
        if self.over.is_none() && self.stored + raw.len() <= HEAD_LIMIT {
            self.stored += raw.len();
            let value = String::from_utf8_lossy(value).into_owned();
            self.fields.push((name.to_owned(), value));
        } else {
            self.not_stored(t, raw.len());
            if FRAMING.contains(&lower.as_str()) {
                self.framed += raw.len();
                if self.framed > FIELD_MAX {
                    return Err(Fault::Framing);
                }
            }
        }
        self.frame.add(&lower, value)?;
        Ok(false)
    }
}

/// The field lines framing and the WebSocket handshake read: a [`FRAMING`] name and its value.
#[derive(Default)]
struct Fields(Vec<(&'static str, String)>);

impl Fields {
    /// A framing field (`Content-Length`, `Transfer-Encoding`) that is not ASCII cannot be framed;
    /// another field read here counts as absent when it is not ASCII.
    fn add(&mut self, name: &str, value: &[u8]) -> Result<(), Fault> {
        let Some(name) = FRAMING.iter().find(|f| **f == name) else {
            return Ok(());
        };
        match std::str::from_utf8(value).ok().filter(|v| v.is_ascii()) {
            Some(v) => self.0.push((name, v.to_owned())),
            None if FRAMING[..2].contains(name) => return Err(Fault::Framing),
            None => {}
        }
        Ok(())
    }

    fn values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.0
            .iter()
            .filter(move |(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }

    fn tokens(&self, name: &str) -> Vec<String> {
        self.values(name)
            .flat_map(|v| v.split(','))
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect()
    }

    /// Every `Content-Length` value and list member must be the same valid number.
    fn length(&self) -> Result<Option<u64>, Fault> {
        let mut seen = None;
        for part in self.values("content-length").flat_map(|v| v.split(',')) {
            let n = length(part.trim()).ok_or(Fault::Framing)?;
            if seen.is_some_and(|s| s != n) {
                return Err(Fault::Framing);
            }
            seen = Some(n);
        }
        Ok(seen)
    }

    /// `Some(true)` when `chunked` is the final transfer coding, `Some(false)` for another one,
    /// `None` without `Transfer-Encoding`.
    fn chunked(&self) -> Option<bool> {
        self.tokens("transfer-encoding")
            .last()
            .map(|last| last == "chunked")
    }

    /// The codings to undo, in order of application: content codings, then transfer codings
    /// other than a final `chunked`.
    fn codings(&self) -> Result<Vec<Coding>, Unsupported> {
        let mut te = self.tokens("transfer-encoding");
        if self.chunked() == Some(true) {
            te.pop();
        }
        let mut all: Vec<&str> = self.values("content-encoding").collect();
        all.extend(te.iter().map(String::as_str));
        parse_codings(&all)
    }

    fn websocket(&self) -> bool {
        self.tokens("connection").iter().any(|t| t == "upgrade")
            && self
                .tokens("upgrade")
                .iter()
                .any(|p| p.split('/').next() == Some("websocket"))
    }

    /// `Sec-WebSocket-Extensions` lines joined by `,`, or `None` without one.
    fn extensions(&self) -> Option<String> {
        let all: Vec<&str> = self.values("sec-websocket-extensions").collect();
        (!all.is_empty()).then(|| all.join(", "))
    }
}

fn length(digits: &str) -> Option<u64> {
    let valid =
        !digits.is_empty() && digits.len() <= 19 && digits.bytes().all(|b| b.is_ascii_digit());
    valid.then(|| digits.parse().ok()).flatten()
}

/// A chunk-size line, read without a buffer: `chunk-size [ BWS ; chunk-ext ]` and its line end
/// (RFC 9112 section 7.1). Extensions are never recorded, so their bytes are only skipped.
#[derive(Default)]
pub(crate) struct SizeLine {
    size: u64,
    /// Significant hex digits so far: past 16 the size fits no `u64` (5a).
    digits: u8,
    any: bool,
    /// The size ended (whitespace seen); `ext` once `;` was.
    after: bool,
    ext: bool,
}

impl SizeLine {
    /// Takes a piece that ends at the line's LF or before it: the size once the line ended.
    fn push(&mut self, piece: &[u8]) -> Result<Option<u64>, Fault> {
        for &b in piece {
            match b {
                b'\n' if self.any => return Ok(Some(self.size)),
                _ if self.ext && b != b'\n' => {}
                b';' => self.ext = true,
                b' ' | b'\t' | b'\r' => self.after = self.any,
                _ if b.is_ascii_hexdigit() && !self.after => {
                    let v = u64::from((b as char).to_digit(16).unwrap_or(0));
                    self.digits += u8::from(self.digits > 0 || v > 0);
                    if self.digits > 16 {
                        return Err(Fault::Framing);
                    }
                    (self.size, self.any) = ((self.size << 4) | v, true);
                }
                _ => return Err(Fault::Framing),
            }
        }
        Ok(None)
    }
}

/// How a body ends on the wire.
pub(crate) enum Framing {
    Length(u64),
    Chunked(Chunk),
    /// Runs to the close of the connection.
    Close,
}

pub(crate) enum Chunk {
    Size(SizeLine),
    Data(u64),
    /// The line end after a chunk's data; `true` once its CR was seen.
    DataEnd(bool),
    Trailer(Box<Head>),
}

/// One open stream's body: framing, decoding and the END tail.
pub(crate) struct Body {
    framing: Framing,
    pub(crate) sink: Sink,
    /// Body bytes on the wire, chunk framing included (END tail `wire`).
    raw: u64,
    /// Of the message's bytes on the wire, head included, those never forwarded (END tail
    /// `unsent`).
    pub(crate) unsent: u64,
}

/// Where a body's bytes go.
pub(crate) struct Sink {
    h: StreamHandle,
    dec: Result<BodyDecoder, Vec<String>>,
    /// Encoded body bytes, de-chunked (the GAP tails' `wire`).
    wire: u64,
    start: Mono,
    /// The first decoder answer other than `Ok`, and when it came.
    failed: Option<(Mono, Step)>,
    pub(crate) trailers: Vec<(String, String)>,
}

impl Sink {
    fn data(&mut self, t: Mono, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.wire += bytes.len() as u64;
        if let Ok(dec) = &mut self.dec {
            let h = &mut self.h;
            let step = dec.push(bytes, &mut |piece| h.data(t, piece));
            if step != Step::Ok && self.failed.is_none() {
                self.failed = Some((t, step));
            }
        }
    }
}

/// The `http_head` GAP, when a head or a trailer section passed the guard.
fn head_gap(h: &mut StreamHandle, head: &Head) {
    if let Some((at, n)) = head.over {
        let tail = json!({"guard": "http_head", "limit": HEAD_LIMIT, "head": n});
        h.gap(GapReason::MemoryGuard, at, None, Some(tail));
    }
}

impl Body {
    /// Writes the `http_head` GAP when the head passed the guard; a message without a body ends
    /// at once and gives `None`. `unsent`: the head's bytes never forwarded.
    pub(crate) fn open(
        mut h: StreamHandle,
        framing: Option<Framing>,
        codings: Result<Vec<Coding>, Unsupported>,
        head: &Head,
        unsent: u64,
        t: Mono,
        rec: &Recorder,
    ) -> Option<Body> {
        head_gap(&mut h, head);
        let Some(framing) = framing else {
            let mut tail = json!({"wire": 0});
            if unsent > 0 {
                tail["unsent"] = json!(unsent);
            }
            h.end(t, EndReason::Complete, Some(tail));
            return None;
        };
        let dec = match codings {
            Ok(codings) => Ok(BodyDecoder::new(codings, rec)),
            Err(Unsupported { codings }) => Err(codings),
        };
        let sink = Sink {
            h,
            dec,
            wire: 0,
            start: t,
            failed: None,
            trailers: Vec::new(),
        };
        Some(Body {
            framing,
            sink,
            raw: 0,
            unsent,
        })
    }

    /// Consumes what `input` holds of this body: `(consumed, done)`. `raw` grows by each piece
    /// taken, also when a later piece faults.
    pub(crate) fn advance(&mut self, t: Mono, input: &[u8]) -> Result<(usize, bool), Fault> {
        let Body {
            framing, sink, raw, ..
        } = self;
        let chunk = match framing {
            Framing::Length(left) => {
                let take = usize::try_from(*left)
                    .unwrap_or(usize::MAX)
                    .min(input.len());
                sink.data(t, &input[..take]);
                *left -= take as u64;
                *raw += take as u64;
                return Ok((take, *left == 0));
            }
            Framing::Close => {
                sink.data(t, input);
                *raw += input.len() as u64;
                return Ok((input.len(), false));
            }
            Framing::Chunked(chunk) => chunk,
        };
        let mut pos = 0;
        while pos < input.len() {
            let rest = &input[pos..];
            match chunk {
                Chunk::Size(line) => {
                    let nl = rest.iter().position(|&b| b == b'\n');
                    let piece = &rest[..nl.map_or(rest.len(), |i| i + 1)];
                    pos += piece.len();
                    let size = line.push(piece)?;
                    *raw += piece.len() as u64;
                    if let Some(size) = size {
                        *chunk = match size {
                            0 => Chunk::Trailer(Box::new(Head::new(false))),
                            n => Chunk::Data(n),
                        };
                    }
                }
                Chunk::Data(left) => {
                    let take = usize::try_from(*left).unwrap_or(usize::MAX).min(rest.len());
                    sink.data(t, &rest[..take]);
                    pos += take;
                    *left -= take as u64;
                    *raw += take as u64;
                    if *left == 0 {
                        *chunk = Chunk::DataEnd(false);
                    }
                }
                Chunk::DataEnd(cr) => {
                    pos += 1;
                    match (rest[0], *cr) {
                        (b'\r', false) => *cr = true,
                        (b'\n', _) => *chunk = Chunk::Size(SizeLine::default()),
                        _ => return Err(Fault::Framing),
                    }
                    *raw += 1;
                }
                Chunk::Trailer(head) => {
                    let (n, done) = head.push(t, rest)?;
                    pos += n;
                    *raw += n as u64;
                    if done {
                        head_gap(&mut sink.h, head);
                        sink.trailers = std::mem::take(&mut head.fields);
                        return Ok((pos, true));
                    }
                }
            }
        }
        Ok((pos, false))
    }

    /// Ends the stream: the decoder's GAP if it had one, then the 5a GAP if `stop` is set.
    /// `extra`: one more END tail key (HTTP/2's `h2_error`).
    pub(crate) fn end(
        self,
        t: Mono,
        reason: EndReason,
        stop: Option<(GapReason, Option<Value>)>,
        extra: Option<(&str, Value)>,
    ) {
        let Sink {
            mut h,
            dec,
            wire,
            start,
            failed,
            trailers,
        } = self.sink;
        let mut reason = reason;
        let (at, step) = match (dec, failed) {
            (Err(codings), _) => {
                if wire > 0 {
                    let tail = json!({"codings": codings, "wire": wire});
                    h.gap(GapReason::UnsupportedEncoding, start, None, Some(tail));
                }
                (t, Step::Ok)
            }
            (Ok(_), Some(failed)) => failed,
            (Ok(dec), None) if reason == EndReason::Complete && wire > 0 => (t, dec.finish()),
            (Ok(_), None) => (t, Step::Ok),
        };
        match step {
            Step::Ok => {}
            Step::Fault => {
                h.gap(GapReason::DecodeFailed, at, None, None);
                reason = EndReason::DecodeError;
            }
            Step::NotDecoded { limit } => {
                let tail = json!({"guard": "decoder_window", "limit": limit, "wire": wire});
                h.gap(GapReason::MemoryGuard, at, None, Some(tail));
            }
        }
        if let Some((gap, tail)) = stop {
            h.gap(gap, t, None, tail);
            reason = EndReason::DecodeError;
        }
        let mut tail = json!({"wire": self.raw});
        if !trailers.is_empty() {
            tail["trailers"] = pairs(&trailers);
        }
        if self.unsent > 0 {
            tail["unsent"] = json!(self.unsent);
        }
        if let Some((key, value)) = extra {
            tail[key] = value;
        }
        h.end(t, reason, Some(tail));
    }
}
