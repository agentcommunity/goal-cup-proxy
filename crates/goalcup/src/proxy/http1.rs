//! The passive HTTP/1.1 parser (PRD-03 sections 5, 5a, 7 and 8; guards `http_head` and
//! `upgrade_copy` of section 11). The relay forwards every byte first; this parser reads a copy,
//! one direction at a time in any split, and records every request whatever its method, path or
//! Host (rule 2): one `http_req` per request, one `http_resp` per final response, paired in order.
//! A `1xx` head is a note. Headers go raw into the meta; PRD-02 masks them.
//!
//! Bodies are de-chunked and decoded through [`BodyDecoder`], each decoded piece written as it is
//! made; [`Step`] says which GAP a body gets. Framing that cannot be followed (a bad start line or
//! field, a bad chunk size, a conflicting `Content-Length`, a `101` that is not a WebSocket this
//! parser can follow, a 2xx to an inner `CONNECT`) stops parsing for the connection (5a): every
//! open stream gets GAP `decode_failed` and ends `decode_error`, and every later call answers the
//! same `Err`; the relay writes the `parse_stopped` note and the `conn` GAP.
//!
//! After a WebSocket upgrade request, client bytes are only copied (the `upgrade_copy` guard, then
//! 5a) until the answer: a `101` with the right `Sec-WebSocket-Accept` hands both directions to
//! [`WsConn`], the copy first; any other final answer replays the copy here. Server bytes that
//! overtake the feed of the request they answer wait, at most a head's worth, for it; past that,
//! or at the close, they are a response to no request (`req` 0).

use crate::pipeline::{
    EndReason, GapReason, Mono, OpenFlags, Recorder, Sid, StreamHandle, StreamKind,
};
use crate::proxy::decode::{BodyDecoder, Coding, Step, Unsupported, parse_codings};
use crate::proxy::ws::{self, Dir, WsConn};
use serde_json::{Value, json};
use std::collections::VecDeque;

/// The `http_head` guard: head bytes stored per head (and per chunked trailer section).
const HEAD_LIMIT: usize = 64 * 1024;
/// The `upgrade_copy` guard: client bytes copied while an upgrade waits for its answer.
const COPY_LIMIT: usize = 1 << 20;
/// The longest chunk-size line, extensions included (RFC 9112 section 7.1).
const CHUNK_LINE_MAX: usize = 4 * 1024;
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
enum Fault {
    /// Framing this parser cannot follow, or the WebSocket parser stopped (it wrote its GAPs).
    Framing,
    /// The `upgrade_copy` guard.
    Copy,
}

/// One decrypted HTTP/1.1 connection: both directions' parsers and the open streams.
pub struct Http1 {
    rec: Recorder,
    conn: Sid,
    up: Side,
    down: Side,
    /// Requests whose final response has not started, in order.
    queue: VecDeque<Asked>,
    /// Client bytes after a WebSocket upgrade request, held until its answer.
    copy: Option<Vec<u8>>,
    /// Server bytes that came before the request they answer.
    early: Vec<u8>,
    /// The response being parsed answers no request.
    orphan: bool,
    ws: Option<WsConn>,
    stopped: Option<GapReason>,
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
            early: Vec::new(),
            orphan: false,
            ws: None,
            stopped: None,
        }
    }

    /// Relayed bytes of one direction in any split, after they were forwarded. Err(reason): parsing
    /// stopped for the connection (5a): the open stream already has its own GAP (decode_failed or
    /// memory_guard) and ended decode_error; every later call returns the same Err.
    pub fn feed(&mut self, dir: Dir, t: Mono, bytes: &[u8]) -> Result<(), GapReason> {
        if let Some(reason) = self.stopped {
            return Err(reason);
        }
        self.take(dir, t, bytes).map_err(|fault| self.stop(t, fault))
    }

    /// The connection closed: end open streams (interrupted when inside a message, complete
    /// otherwise; decode_error after a stop), unsent_up goes to the open http_req (or ws_up) END tail.
    pub fn end(mut self, t: Mono, reason: EndReason, unsent_up: u64) {
        if self.stopped.is_none() && self.ws.is_none() {
            // An upgrade never answered: its copy is HTTP after all. Server bytes still waiting
            // for a request answer none.
            let copy = self.copy.take().unwrap_or_default();
            let mut fed = self.take(Dir::Up, t, &copy);
            if fed.is_ok() && !self.early.is_empty() {
                self.orphan = true;
                let early = std::mem::take(&mut self.early);
                fed = self.take(Dir::Down, t, &early);
            }
            if let Err(fault) = fed {
                self.stop(t, fault);
            }
        }
        if let Some(ws) = self.ws.take() {
            ws.end(t, reason, unsent_up);
            return;
        }
        if let Some(body) = self.up.body.take() {
            body.end(t, EndReason::Interrupted, unsent_up, None);
        }
        if let Some(body) = self.down.body.take() {
            let close = matches!(body.framing, Framing::Close);
            let reason = if close { reason } else { EndReason::Interrupted };
            body.end(t, reason, 0, None);
        }
    }

    /// 5a: every open stream gets the GAP and ends `decode_error`; nothing parses after this.
    fn stop(&mut self, t: Mono, fault: Fault) -> GapReason {
        let (reason, tail) = match fault {
            Fault::Framing => (GapReason::DecodeFailed, None),
            Fault::Copy => (
                GapReason::MemoryGuard,
                Some(json!({"guard": "upgrade_copy", "limit": COPY_LIMIT})),
            ),
        };
        for body in [self.up.body.take(), self.down.body.take()]
            .into_iter()
            .flatten()
        {
            body.end(t, EndReason::DecodeError, 0, Some((reason, tail.clone())));
        }
        self.copy = None;
        self.early = Vec::new();
        self.queue.clear();
        self.stopped = Some(reason);
        reason
    }

    fn take(&mut self, dir: Dir, t: Mono, mut bytes: &[u8]) -> Result<(), Fault> {
        while !bytes.is_empty() {
            if let Some(ws) = &mut self.ws {
                return ws.feed(dir, t, bytes).map_err(|_| Fault::Framing);
            }
            if dir == Dir::Up
                && let Some(copy) = &mut self.copy
            {
                if copy.len() + bytes.len() > COPY_LIMIT {
                    return Err(Fault::Copy);
                }
                copy.extend_from_slice(bytes);
                return Ok(());
            }
            if dir == Dir::Down && self.down.body.is_none() && self.queue.is_empty() && !self.orphan {
                if self.early.len() + bytes.len() <= HEAD_LIMIT {
                    self.early.extend_from_slice(bytes);
                    return Ok(());
                }
                // A head's worth and still no request: these bytes answer none.
                self.orphan = true;
                let mut all = std::mem::take(&mut self.early);
                all.extend_from_slice(bytes);
                return self.take(dir, t, &all);
            }
            let side = match dir {
                Dir::Up => &mut self.up,
                Dir::Down => &mut self.down,
            };
            let n = if let Some(body) = &mut side.body {
                let (n, done) = body.advance(t, bytes)?;
                body.raw += n as u64;
                if done && let Some(body) = side.body.take() {
                    body.end(t, EndReason::Complete, 0, None);
                }
                n
            } else {
                let (n, done) = side.head.push(t, bytes)?;
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
        let (method, target, http11) = request_line(&head.start).ok_or(Fault::Framing)?;
        let f = &head.frame;
        let framing = match f.chunked() {
            Some(true) => Some(Framing::Chunked(Chunk::Size(Vec::new()))),
            // Another final transfer coding cannot frame a request (RFC 9112 section 6.3).
            Some(false) => return Err(Fault::Framing),
            None => f.length()?.filter(|&n| n > 0).map(Framing::Length),
        };
        let keys: Vec<&str> = f.values("sec-websocket-key").collect();
        let ws = match keys.as_slice() {
            [key]
                if method == "GET"
                    && http11
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
            "ver": "1.1",
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
        if ws.is_some() {
            self.copy = Some(Vec::new());
        }
        self.queue.push_back(Asked {
            sid: h.sid(),
            head: method == "HEAD",
            connect: method == "CONNECT",
            ws,
        });
        self.up.body = Body::open(h, framing, codings, &head, t, &self.rec);
        if !self.early.is_empty() {
            let early = std::mem::take(&mut self.early);
            self.take(Dir::Down, t, &early)?;
        }
        Ok(())
    }

    fn response(&mut self, t: Mono, head: Head) -> Result<(), Fault> {
        let status = status_line(&head.start).ok_or(Fault::Framing)?;
        let f = &head.frame;
        if (100..200).contains(&status) {
            self.rec.event(json!({
                "ev": "note",
                "what": "informational_response",
                "conn": self.conn,
                "status": status,
                "headers": pairs(&head.fields),
            }));
            if status != 101 {
                return Ok(());
            }
            // The protocol switched: only a WebSocket this parser can follow goes on.
            let asked = self.queue.pop_front().unwrap_or_default();
            let Some((key, offered)) = asked.ws else {
                return Err(Fault::Framing);
            };
            let accepted = f.websocket()
                && f.values("sec-websocket-accept").eq([accept_key(&key).as_str()]);
            if !accepted {
                return Err(Fault::Framing);
            }
            let extensions = f.extensions();
            let deflate = ws::negotiate(offered.as_deref(), extensions.as_deref())
                .map_err(|_| Fault::Framing)?;
            let mut conn = WsConn::open(&self.rec, self.conn, asked.sid, deflate);
            let copy = self.copy.take().unwrap_or_default();
            let fed = conn.feed(Dir::Up, t, &copy);
            self.ws = Some(conn);
            return fed.map_err(|_| Fault::Framing);
        }
        let asked = self.queue.pop_front().unwrap_or_default();
        self.orphan = false;
        let framing = if asked.head || status == 204 || status == 304 {
            None
        } else if asked.connect && (200..300).contains(&status) {
            // A tunnel inside the connection: not HTTP after this head.
            return Err(Fault::Framing);
        } else {
            match (f.chunked(), f.length()?) {
                (Some(true), _) => Some(Framing::Chunked(Chunk::Size(Vec::new()))),
                (Some(false), _) | (None, None) => Some(Framing::Close),
                (None, Some(0)) => None,
                (None, Some(n)) => Some(Framing::Length(n)),
            }
        };
        let codings = f.codings();
        let meta = json!({
            "status": status,
            "ver": "1.1",
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
        self.down.body = Body::open(h, framing, codings, &head, t, &self.rec);
        if asked.ws.is_some()
            && let Some(copy) = self.copy.take()
        {
            // The upgrade was refused: the copied client bytes are HTTP.
            self.take(Dir::Up, t, &copy)?;
        }
        Ok(())
    }
}

/// `codings` for the OPEN meta: the list undone, or null when none is.
fn coding_meta(codings: &Result<Vec<Coding>, Unsupported>) -> Value {
    match codings {
        Ok(list) if !list.is_empty() => json!(list.iter().map(|c| c.name()).collect::<Vec<_>>()),
        _ => Value::Null,
    }
}

fn pairs(fields: &[(String, String)]) -> Value {
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

/// A request target reduced to its path without query and fragment (0.7's `safe_target`).
/// Origin form keeps `/p`, absolute form keeps the path after the authority; asterisk and
/// authority forms have no path (null).
fn safe_path(target: &str) -> Value {
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

fn is_http1(version: &str) -> bool {
    version == "HTTP/1.1" || version == "HTTP/1.0"
}

/// Method, target, and whether the version is 1.1.
fn request_line(line: &[u8]) -> Option<(&str, &str, bool)> {
    let line = std::str::from_utf8(line).ok()?;
    let mut parts = line.split(' ');
    let (method, target, version) = (parts.next()?, parts.next()?, parts.next()?);
    let valid = parts.next().is_none()
        && is_token(method)
        && !target.is_empty()
        && target.bytes().all(|b| b.is_ascii_graphic())
        && is_http1(version);
    valid.then_some((method, target, version == "HTTP/1.1"))
}

fn status_line(line: &[u8]) -> Option<u16> {
    let line = std::str::from_utf8(line).ok()?;
    let mut parts = line.splitn(3, ' ');
    let (version, code) = (parts.next()?, parts.next()?);
    let valid = is_http1(version) && code.len() == 3 && code.bytes().all(|b| b.is_ascii_digit());
    valid.then(|| code.parse().ok()).flatten()
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
struct Head {
    /// The start line is still to come (never, for trailers).
    want_start: bool,
    start: Vec<u8>,
    line: Vec<u8>,
    /// The rest of the current line is not kept.
    skip: bool,
    /// Field lines stored for the meta, raw, in wire order.
    fields: Vec<(String, String)>,
    frame: Fields,
    stored: usize,
    /// Once past the `http_head` guard: when, and the bytes not stored.
    over: Option<(Mono, u64)>,
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
            if self.skip {
                self.not_stored(t, piece.len());
            } else {
                self.line.extend_from_slice(piece);
                if !ends && self.stored + self.line.len() > HEAD_LIMIT {
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

    /// A line still open has passed the guard: keep it only while it can be a framing field.
    fn past_guard(&mut self, t: Mono) -> Result<(), Fault> {
        if self.want_start {
            // A start line past 64 KiB cannot be parsed (a garbage head, 5a).
            return Err(Fault::Framing);
        }
        let framing = match self.line.iter().position(|&b| b == b':') {
            Some(colon) => {
                let name = String::from_utf8_lossy(&self.line[..colon]).to_ascii_lowercase();
                FRAMING.contains(&name.as_str())
            }
            None => self.line.len() <= NAME_MAX,
        };
        if !framing {
            self.skip = true;
            let n = std::mem::take(&mut self.line).len();
            self.not_stored(t, n);
        }
        Ok(())
    }

    /// One complete line: `Ok(true)` at the empty line that ends the head.
    fn finish_line(&mut self, t: Mono) -> Result<bool, Fault> {
        if std::mem::take(&mut self.skip) {
            return Ok(false);
        }
        let raw = std::mem::take(&mut self.line);
        let line = strip_eol(&raw);
        if self.want_start {
            if raw.len() > HEAD_LIMIT {
                return Err(Fault::Framing);
            }
            if !line.is_empty() {
                self.want_start = false;
                self.stored += raw.len();
                self.start = line.to_vec();
            }
            return Ok(false);
        }
        if line.is_empty() {
            return Ok(true);
        }
        // Obsolete line folding and a bad name cannot be framed (RFC 9112 section 5).
        if matches!(line[0], b' ' | b'\t') {
            return Err(Fault::Framing);
        }
        let colon = line
            .iter()
            .position(|&b| b == b':')
            .ok_or(Fault::Framing)?;
        let name = std::str::from_utf8(&line[..colon])
            .ok()
            .filter(|n| is_token(n))
            .ok_or(Fault::Framing)?;
        let value = trim_ows(&line[colon + 1..]);
        if self.over.is_none() && self.stored + raw.len() <= HEAD_LIMIT {
            self.stored += raw.len();
            let value = String::from_utf8_lossy(value).into_owned();
            self.fields.push((name.to_owned(), value));
        } else {
            self.not_stored(t, raw.len());
        }
        self.frame.add(&name.to_ascii_lowercase(), value)?;
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
        let mut transfer: Vec<&str> = self
            .values("transfer-encoding")
            .flat_map(|v| v.split(','))
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .collect();
        if transfer.last().is_some_and(|t| t.eq_ignore_ascii_case("chunked")) {
            transfer.pop();
        }
        let all: Vec<&str> = self.values("content-encoding").chain(transfer).collect();
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
    let valid = !digits.is_empty() && digits.len() <= 19 && digits.bytes().all(|b| b.is_ascii_digit());
    valid.then(|| digits.parse().ok()).flatten()
}

/// `chunk-size [ BWS ; chunk-ext ]`: hexadecimal, at most 16 significant digits.
fn chunk_size(line: &[u8]) -> Option<u64> {
    let size = trim_ows(line.split(|&b| b == b';').next()?);
    let digits = std::str::from_utf8(size).ok()?;
    let significant = digits.trim_start_matches('0');
    let valid = !digits.is_empty()
        && significant.len() <= 16
        && digits.bytes().all(|b| b.is_ascii_hexdigit());
    if !valid {
        return None;
    }
    u64::from_str_radix(if significant.is_empty() { "0" } else { significant }, 16).ok()
}

/// How a body ends on the wire.
enum Framing {
    Length(u64),
    Chunked(Chunk),
    /// Runs to the close of the connection.
    Close,
}

enum Chunk {
    /// The chunk-size line read so far.
    Size(Vec<u8>),
    Data(u64),
    /// The line end after a chunk's data; `true` once its CR was seen.
    DataEnd(bool),
    Trailer(Box<Head>),
}

/// One open stream's body: framing, decoding and the END tail.
struct Body {
    framing: Framing,
    sink: Sink,
    /// Body bytes on the wire, chunk framing included (END tail `wire`).
    raw: u64,
}

/// Where a body's bytes go.
struct Sink {
    h: StreamHandle,
    dec: Result<BodyDecoder, Vec<String>>,
    /// Encoded body bytes, de-chunked (the GAP tails' `wire`).
    wire: u64,
    start: Mono,
    /// The first decoder answer other than `Ok`, and when it came.
    failed: Option<(Mono, Step)>,
    trailers: Vec<(String, String)>,
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

impl Body {
    /// Writes the `http_head` GAP when the head passed the guard; a message without a body ends
    /// at once and gives `None`.
    fn open(
        mut h: StreamHandle,
        framing: Option<Framing>,
        codings: Result<Vec<Coding>, Unsupported>,
        head: &Head,
        t: Mono,
        rec: &Recorder,
    ) -> Option<Body> {
        if let Some((at, n)) = head.over {
            let tail = json!({"guard": "http_head", "limit": HEAD_LIMIT, "head": n});
            h.gap(GapReason::MemoryGuard, at, None, Some(tail));
        }
        let Some(framing) = framing else {
            h.end(t, EndReason::Complete, Some(json!({"wire": 0})));
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
        })
    }

    /// Consumes what `input` holds of this body: `(consumed, done)`.
    fn advance(&mut self, t: Mono, input: &[u8]) -> Result<(usize, bool), Fault> {
        let sink = &mut self.sink;
        let chunk = match &mut self.framing {
            Framing::Length(left) => {
                let take = usize::try_from(*left).unwrap_or(usize::MAX).min(input.len());
                sink.data(t, &input[..take]);
                *left -= take as u64;
                return Ok((take, *left == 0));
            }
            Framing::Close => {
                sink.data(t, input);
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
                    line.extend_from_slice(piece);
                    if line.len() > CHUNK_LINE_MAX + 2 {
                        return Err(Fault::Framing);
                    }
                    if nl.is_some() {
                        let size = chunk_size(strip_eol(line)).ok_or(Fault::Framing)?;
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
                    if *left == 0 {
                        *chunk = Chunk::DataEnd(false);
                    }
                }
                Chunk::DataEnd(cr) => {
                    pos += 1;
                    match (rest[0], *cr) {
                        (b'\r', false) => *cr = true,
                        (b'\n', _) => *chunk = Chunk::Size(Vec::new()),
                        _ => return Err(Fault::Framing),
                    }
                }
                Chunk::Trailer(head) => {
                    let (n, done) = head.push(t, rest)?;
                    pos += n;
                    if done {
                        if let Some((at, n)) = head.over {
                            let tail = json!({"guard": "http_head", "limit": HEAD_LIMIT, "head": n});
                            sink.h.gap(GapReason::MemoryGuard, at, None, Some(tail));
                        }
                        sink.trailers = std::mem::take(&mut head.fields);
                        return Ok((pos, true));
                    }
                }
            }
        }
        Ok((pos, false))
    }

    /// Ends the stream: the decoder's GAP if it had one, then the 5a GAP if `stop` is set.
    fn end(self, t: Mono, reason: EndReason, unsent: u64, stop: Option<(GapReason, Option<Value>)>) {
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
        if unsent > 0 {
            tail["unsent"] = json!(unsent);
        }
        h.end(t, reason, Some(tail));
    }
}
