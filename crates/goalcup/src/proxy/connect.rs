//! Admission building blocks (PRD-03 section 2, section 1 rules 3, 8 and 9): the CONNECT head,
//! the fixed answers, refusal notes, the upstream TCP leg, the first-bytes race, the ClientHello
//! (SNI, ALPN, fingerprint) and the replaying stream. `serve` puts them together; nothing here
//! decides a host, holds a deadline after the head or refuses a connection on its own.
//!
//! Nothing here logs. Errors are fixed text: never a header value, a body byte or a target.

use crate::pipeline::{Mono, Recorder};
use bytes::{Bytes, BytesMut};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, ReadBuf};
use tokio::net::TcpStream;

/// The largest CONNECT head (request line, header lines, blank line) before a `431`.
pub const HEAD_GUARD: usize = 8 * 1024;

/// How long the CONNECT head may take to arrive. The only deadline of admission.
pub const HEAD_DEADLINE: Duration = Duration::from_secs(30);

/// Memory guard `client_hello`: the most bytes held while reading a ClientHello.
pub const CLIENT_HELLO_GUARD: usize = 64 * 1024;

/// A listed target is retried until this long after the first attempt.
pub const LISTED_RETRY_FOR: Duration = Duration::from_secs(25);
/// The first pause between two connect rounds; it doubles up to [`RETRY_PAUSE_MAX`].
pub const RETRY_PAUSE_FIRST: Duration = Duration::from_millis(250);
/// The longest pause between two connect rounds.
pub const RETRY_PAUSE_MAX: Duration = Duration::from_secs(2);
/// One upstream address attempt, so one dead route cannot hold the others back.
pub const CONNECT_ATTEMPT: Duration = Duration::from_secs(5);

/// Upstream TCP keepalive: idle time, probe interval, probes.
pub const KEEPALIVE_IDLE: Duration = Duration::from_secs(60);
pub const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
pub const KEEPALIVE_PROBES: u32 = 4;

/// One `connect_refused` note per host and status in this window (rule 3).
pub const REFUSAL_NOTE_WINDOW_NS: u64 = 60_000_000_000;

/// A parsed CONNECT head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Head {
    /// Lowercase, one trailing dot dropped; an IPv6 literal without brackets.
    pub host: String,
    pub port: u16,
    /// Every byte the client sent past the blank line, replayed into the chosen path (PRD-17 C1).
    pub rest: Bytes,
}

/// A head that gets a non-`200` answer. `host` is set only when the head parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    /// `400` broken line or a head that did not arrive in time, `405` a method other than
    /// CONNECT, `431` a head past [`HEAD_GUARD`].
    pub status: u16,
    pub host: Option<String>,
}

/// Why no head came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoHead {
    /// Answer `status` with [`answer`] and note it with [`RefusalNotes`].
    Refused(Refused),
    /// The client closed or its socket failed before a whole head: nothing to answer.
    Closed,
}

impl NoHead {
    fn status(status: u16) -> NoHead {
        NoHead::Refused(Refused { status, host: None })
    }
}

/// Reads the CONNECT head into at most [`HEAD_GUARD`] bytes within [`HEAD_DEADLINE`], timed by
/// `rec.now()`. A broken head is never echoed: no error carries a byte of it.
pub async fn read_head<R: AsyncRead + Unpin>(io: &mut R, rec: &Recorder) -> Result<Head, NoHead> {
    let start = rec.now();
    let mut buf = BytesMut::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    loop {
        let spent = Duration::from_nanos(rec.now().ns().saturating_sub(start.ns()));
        let Some(left) = HEAD_DEADLINE.checked_sub(spent).filter(|d| !d.is_zero()) else {
            return Err(NoHead::status(400));
        };
        let room = (HEAD_GUARD - buf.len()).min(chunk.len());
        let n = match tokio::time::timeout(left, io.read(&mut chunk[..room])).await {
            Err(_) => return Err(NoHead::status(400)),
            Ok(Err(_)) | Ok(Ok(0)) => return Err(NoHead::Closed),
            Ok(Ok(n)) => n,
        };
        let from = buf.len().saturating_sub(3);
        buf.extend_from_slice(&chunk[..n]);
        if let Some(at) = buf[from..].windows(4).position(|w| w == b"\r\n\r\n") {
            let end = from + at + 4;
            let rest = buf.split_off(end).freeze();
            let (host, port) = parse_head(&buf)?;
            return Ok(Head { host, port, rest });
        }
        if buf.len() >= HEAD_GUARD {
            return Err(NoHead::status(431));
        }
    }
}

/// A complete head ending in the blank line: structure first, then the method, then the target.
fn parse_head(head: &[u8]) -> Result<(String, u16), NoHead> {
    let broken = || NoHead::status(400);
    let body = &head[..head.len() - 2];
    let mut lines = body
        .split_inclusive(|&b| b == b'\n')
        .map(|line| line.strip_suffix(b"\r\n"));
    let request_line = lines.next().flatten().ok_or_else(broken)?;
    let mut parts = request_line.split(|&b| b == b' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(broken());
    };
    if method.is_empty() || target.is_empty() || !matches!(version, b"HTTP/1.1" | b"HTTP/1.0") {
        return Err(broken());
    }
    for line in lines {
        let line = line.ok_or_else(broken)?;
        let colon = line.iter().position(|&b| b == b':').ok_or_else(broken)?;
        let (name, value) = (&line[..colon], &line[colon + 1..]);
        if name.is_empty()
            || !name.iter().all(|&b| is_token_byte(b))
            || value.iter().any(|&b| (b < 0x20 && b != b'\t') || b == 0x7f)
        {
            return Err(broken());
        }
    }
    if method != b"CONNECT" {
        return Err(NoHead::status(405));
    }
    parse_authority(target).ok_or_else(broken)
}

/// RFC 9110 `tchar`.
fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// `host:port` or `[ipv6]:port`, nothing else: no userinfo, path, query or fragment.
fn parse_authority(target: &[u8]) -> Option<(String, u16)> {
    let target = std::str::from_utf8(target).ok()?;
    let (host, port) = if let Some(rest) = target.strip_prefix('[') {
        let (literal, port) = rest.split_once("]:")?;
        (
            literal.parse::<std::net::Ipv6Addr>().ok()?.to_string(),
            port,
        )
    } else {
        let (host, port) = target.rsplit_once(':')?;
        (normalize_host(host)?, port)
    };
    if port.is_empty() || port.len() > 5 || !port.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let port: u16 = port.parse().ok()?;
    (port != 0).then_some((host, port))
}

/// Lowercases a name and drops one trailing dot. DNS-style labels (letters, digits, hyphen,
/// underscore) and IPv4 literals only.
fn normalize_host(host: &str) -> Option<String> {
    let host = host.to_ascii_lowercase();
    let host = host.strip_suffix('.').unwrap_or(&host);
    let valid = !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        });
    valid.then(|| host.to_owned())
}

/// Writes one fixed answer: `200`, `400`, `405`, `431` or `502` (any other status is a `400`).
pub async fn answer<W: AsyncWrite + Unpin>(io: &mut W, status: u16) -> io::Result<()> {
    let text: &[u8] = match status {
        200 => b"HTTP/1.1 200 Connection established\r\n\r\n",
        405 => b"HTTP/1.1 405 Method Not Allowed\r\nAllow: CONNECT\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        431 => b"HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        502 => b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        _ => b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    };
    io.write_all(text).await?;
    io.flush().await
}

/// At most one `connect_refused` note per host and status per minute (rule 3), timed by
/// `Recorder::now()` (rule 9). No `conn` stream: no connection exists.
#[derive(Default)]
pub struct RefusalNotes {
    last: Mutex<HashMap<(Option<String>, u16), Mono>>,
}

impl RefusalNotes {
    pub fn new() -> RefusalNotes {
        RefusalNotes::default()
    }

    /// Writes the note unless one for this host and status was written within the minute.
    pub fn note(&self, rec: &Recorder, host: Option<&str>, status: u16) {
        let now = rec.now();
        let fresh = |t: &Mono| now.ns().saturating_sub(t.ns()) < REFUSAL_NOTE_WINDOW_NS;
        {
            let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
            let key = (host.map(str::to_owned), status);
            if last.get(&key).is_some_and(fresh) {
                return;
            }
            last.retain(|_, t| fresh(t));
            last.insert(key, now);
        }
        let mut ev = json!({"ev": "note", "what": "connect_refused", "status": status});
        if let Some(host) = host {
            ev["host"] = json!(host);
        }
        rec.event(ev);
    }
}

/// Connects the upstream leg. Each round re-resolves `name` (or takes `test_override`) and tries
/// every address, IPv4 first, each for at most [`CONNECT_ATTEMPT`]. A `listed` target is retried
/// until [`LISTED_RETRY_FOR`], pausing 250 ms doubling to 2 s; any other gets one round, since
/// retrying the agent's own traffic changes the run (PRD-17 C6). Nothing has been sent upstream,
/// so a retry replays nothing. The socket gets `TCP_NODELAY` and keepalive; no idle timeout and
/// no `TCP_USER_TIMEOUT`.
pub async fn connect_upstream(
    name: &str,
    port: u16,
    listed: bool,
    test_override: Option<SocketAddr>,
) -> io::Result<TcpStream> {
    let until = tokio::time::Instant::now() + LISTED_RETRY_FOR;
    let mut pause = RETRY_PAUSE_FIRST;
    loop {
        let round = match test_override {
            Some(addr) => connect_addresses(vec![addr]).await,
            None => match tokio::net::lookup_host((name, port)).await {
                Ok(found) => {
                    let mut addrs: Vec<SocketAddr> = found.collect();
                    addrs.sort_by_key(SocketAddr::is_ipv6);
                    connect_addresses(addrs).await
                }
                Err(_) => Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "upstream name did not resolve",
                )),
            },
        };
        let err = match round {
            Ok(stream) => return Ok(stream),
            Err(e) => e,
        };
        if !listed || tokio::time::Instant::now() + pause >= until {
            return Err(err);
        }
        tokio::time::sleep(pause).await;
        pause = (pause * 2).min(RETRY_PAUSE_MAX);
    }
}

async fn connect_addresses(addrs: Vec<SocketAddr>) -> io::Result<TcpStream> {
    let mut last = io::Error::new(io::ErrorKind::NotFound, "upstream name has no address");
    for addr in addrs {
        match tokio::time::timeout(CONNECT_ATTEMPT, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => {
                let _ = stream.set_nodelay(true);
                let keepalive = socket2::TcpKeepalive::new()
                    .with_time(KEEPALIVE_IDLE)
                    .with_interval(KEEPALIVE_INTERVAL)
                    .with_retries(KEEPALIVE_PROBES);
                let _ = socket2::SockRef::from(&stream).set_tcp_keepalive(&keepalive);
                return Ok(stream);
            }
            Ok(Err(e)) => last = e,
            Err(_) => {
                last = io::Error::new(io::ErrorKind::TimedOut, "upstream connect timed out");
            }
        }
    }
    Err(last)
}

/// Closes a socket with a TCP reset (`SO_LINGER` 0). Only for the harness leg when the upstream
/// leg failed (rule 8); a clean upstream close stays a clean close.
pub fn reset(stream: TcpStream) {
    let _ = socket2::SockRef::from(&stream).set_linger(Some(Duration::ZERO));
    drop(stream);
}

/// Who spoke first after the `200`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum First {
    Client(Bytes),
    /// The caller tunnels with reason `not_tls`, replaying these bytes to the client.
    Server(Bytes),
    ClientClosed,
    ServerClosed,
}

/// Races the first bytes of both sides, with no deadline. Call it only when the head carried no
/// bytes past the blank line (those are the client's first bytes). A read error counts as a close.
pub async fn first_bytes<C, U>(client: &mut C, upstream: &mut U) -> First
where
    C: AsyncRead + Unpin,
    U: AsyncRead + Unpin,
{
    let mut a = [0u8; 16 * 1024];
    let mut b = [0u8; 16 * 1024];
    tokio::select! {
        r = client.read(&mut a) => match r {
            Ok(n) if n > 0 => First::Client(Bytes::copy_from_slice(&a[..n])),
            _ => First::ClientClosed,
        },
        r = upstream.read(&mut b) => match r {
            Ok(n) if n > 0 => First::Server(Bytes::copy_from_slice(&b[..n])),
            _ => First::ServerClosed,
        },
    }
}

/// What the ClientHello says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientHelloInfo {
    /// The `host_name`, lowercase, one trailing dot dropped.
    pub sni: Option<String>,
    /// The ALPN names in the client's order.
    pub alpn: Vec<Vec<u8>>,
    /// SHA-256 over the legacy version and the sorted cipher suites, extension types and
    /// supported groups, GREASE values (RFC 8701) removed.
    pub fingerprint: [u8; 32],
    /// Every byte read, for replay.
    pub bytes: Bytes,
}

/// The outcome of reading a ClientHello. Every variant keeps every byte read, for replay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hello {
    Parsed(ClientHelloInfo),
    /// Past the `client_hello` guard ([`CLIENT_HELLO_GUARD`]): a tunnel, reason `hello_too_big`.
    TooBig(Bytes),
    /// Not a TLS handshake, or not a well-formed ClientHello.
    NotTls(Bytes),
    /// The client closed (or its socket failed) before the ClientHello was complete.
    Closed(Bytes),
}

/// Reads until the ClientHello is complete, starting from `already` (bytes the caller holds).
/// Joins a ClientHello split across TLS records and reads.
pub async fn peek_client_hello<R: AsyncRead + Unpin>(io: &mut R, already: Bytes) -> Hello {
    let mut buf = BytesMut::from(&already[..]);
    let mut chunk = [0u8; 4096];
    loop {
        match parse_client_hello(&buf) {
            Parse::NeedMore if buf.len() < CLIENT_HELLO_GUARD => {}
            Parse::NeedMore | Parse::TooBig => return Hello::TooBig(buf.freeze()),
            Parse::NotTls => return Hello::NotTls(buf.freeze()),
            Parse::Done(f) => {
                return Hello::Parsed(ClientHelloInfo {
                    sni: f.sni,
                    alpn: f.alpn,
                    fingerprint: f.fingerprint,
                    bytes: buf.freeze(),
                });
            }
        }
        let room = (CLIENT_HELLO_GUARD - buf.len()).min(chunk.len());
        match io.read(&mut chunk[..room]).await {
            Ok(n) if n > 0 => buf.extend_from_slice(&chunk[..n]),
            _ => return Hello::Closed(buf.freeze()),
        }
    }
}

/// The first 16 hex digits of a fingerprint, for `conn` meta `fingerprint`.
pub fn fingerprint_hex16(fp: &[u8; 32]) -> String {
    fp[..8].iter().map(|b| format!("{b:02x}")).collect()
}

struct Facts {
    sni: Option<String>,
    alpn: Vec<Vec<u8>>,
    fingerprint: [u8; 32],
}

enum Parse {
    NeedMore,
    TooBig,
    NotTls,
    Done(Facts),
}

const TLS_HANDSHAKE: u8 = 22;
const TLS_MAX_RECORD: usize = 16 * 1024;
const CLIENT_HELLO: u8 = 1;
const EXT_SERVER_NAME: u16 = 0;
const EXT_SUPPORTED_GROUPS: u16 = 10;
const EXT_ALPN: u16 = 16;

/// Joins consecutive handshake records and parses the ClientHello once it is whole. Pure:
/// callable on any prefix.
fn parse_client_hello(bytes: &[u8]) -> Parse {
    let mut handshake = Vec::new();
    let mut offset = 0;
    loop {
        let Some(header) = bytes.get(offset..offset + 5) else {
            if offset == 0 && bytes.first().is_some_and(|&b| b != TLS_HANDSHAKE) {
                return Parse::NotTls;
            }
            return Parse::NeedMore;
        };
        if header[0] != TLS_HANDSHAKE || header[1] != 3 {
            return Parse::NotTls;
        }
        let len = usize::from(u16::from_be_bytes([header[3], header[4]]));
        if len == 0 || len > TLS_MAX_RECORD {
            return Parse::NotTls;
        }
        let Some(fragment) = bytes.get(offset + 5..offset + 5 + len) else {
            return Parse::NeedMore;
        };
        handshake.extend_from_slice(fragment);
        offset += 5 + len;
        if handshake.len() >= 4 {
            if handshake[0] != CLIENT_HELLO {
                return Parse::NotTls;
            }
            let body = (usize::from(handshake[1]) << 16)
                | (usize::from(handshake[2]) << 8)
                | usize::from(handshake[3]);
            if body + 4 > CLIENT_HELLO_GUARD {
                return Parse::TooBig;
            }
            if let Some(body) = handshake.get(4..4 + body) {
                return parse_body(body).map_or(Parse::NotTls, Parse::Done);
            }
        }
    }
}

/// RFC 8701: both bytes equal, each of the form `0x?A`.
fn is_grease(v: u16) -> bool {
    (v & 0x0f0f) == 0x0a0a && (v >> 8) == (v & 0xff)
}

/// Version, random, session ID, cipher suites, compression, then the extensions.
fn parse_body(body: &[u8]) -> Option<Facts> {
    let mut r = Reader(body);
    let version = r.u16()?;
    r.take(32)?;
    let session = r.u8()?;
    if session > 32 {
        return None;
    }
    r.take(usize::from(session))?;
    let suites_len = r.u16()?;
    if suites_len == 0 || suites_len % 2 != 0 {
        return None;
    }
    let mut suites = u16_list(r.take(usize::from(suites_len))?);
    let compression = r.u8()?;
    if compression == 0 {
        return None;
    }
    r.take(usize::from(compression))?;
    let mut sni = None;
    let mut alpn = Vec::new();
    let mut types = Vec::new();
    let mut groups = Vec::new();
    if !r.0.is_empty() {
        let extensions = usize::from(r.u16()?);
        if extensions != r.0.len() {
            return None;
        }
        while !r.0.is_empty() {
            let kind = r.u16()?;
            let len = r.u16()?;
            let data = r.take(usize::from(len))?;
            types.push(kind);
            match kind {
                EXT_SERVER_NAME => sni = parse_server_name(data)?,
                EXT_ALPN => alpn = parse_alpn(data)?,
                EXT_SUPPORTED_GROUPS => {
                    let mut g = Reader(data);
                    let n = usize::from(g.u16()?);
                    if n % 2 != 0 || n != g.0.len() {
                        return None;
                    }
                    groups = u16_list(g.0);
                }
                _ => {}
            }
        }
    }
    let mut seen = types.clone();
    seen.sort_unstable();
    if seen.windows(2).any(|w| w[0] == w[1] && !is_grease(w[0])) {
        return None;
    }
    let mut hash = Sha256::new();
    hash.update(version.to_be_bytes());
    for list in [&mut suites, &mut types, &mut groups] {
        list.retain(|&v| !is_grease(v));
        list.sort_unstable();
        hash.update((list.len() as u32).to_be_bytes());
        for v in list.iter() {
            hash.update(v.to_be_bytes());
        }
    }
    Some(Facts {
        sni,
        alpn,
        fingerprint: hash.finalize().into(),
    })
}

fn u16_list(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect()
}

/// A `server_name` body (RFC 6066): one `host_name` at most, non-empty ASCII.
fn parse_server_name(data: &[u8]) -> Option<Option<String>> {
    let mut r = Reader(data);
    let list = usize::from(r.u16()?);
    if list == 0 || list != r.0.len() {
        return None;
    }
    let mut host = None;
    while !r.0.is_empty() {
        let kind = r.u8()?;
        let len = r.u16()?;
        let name = r.take(usize::from(len))?;
        if kind == 0 {
            if host.is_some() || name.is_empty() || !name.is_ascii() {
                return None;
            }
            let name = std::str::from_utf8(name).ok()?.to_ascii_lowercase();
            host = Some(name.strip_suffix('.').map(str::to_owned).unwrap_or(name));
        }
    }
    Some(host)
}

/// An ALPN body (RFC 7301): a list of non-empty names.
fn parse_alpn(data: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut r = Reader(data);
    let list = usize::from(r.u16()?);
    if list != r.0.len() {
        return None;
    }
    let mut names = Vec::new();
    while !r.0.is_empty() {
        let len = usize::from(r.u8()?);
        if len == 0 {
            return None;
        }
        names.push(r.take(len)?.to_vec());
    }
    Some(names)
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }

    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
}

/// A stream that first yields the bytes admission already read, then `T`. Writes go to `T`.
pub struct PrefixedIo<T> {
    prefix: Bytes,
    inner: T,
}

impl<T> PrefixedIo<T> {
    pub fn new(prefix: Bytes, inner: T) -> PrefixedIo<T> {
        PrefixedIo { prefix, inner }
    }

    pub fn get_ref(&self) -> &T {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// The inner stream; prefix bytes not yet read are dropped.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for PrefixedIo<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if !this.prefix.is_empty() {
            let n = this.prefix.len().min(buf.remaining());
            buf.put_slice(&this.prefix.split_to(n));
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for PrefixedIo<T> {
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

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
}
