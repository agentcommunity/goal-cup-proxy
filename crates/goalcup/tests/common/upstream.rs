//! The fake upstream (PRD-14 section 5): TLS with a chosen ALPN or plain TCP, HTTP/1.1 (whole,
//! chunked, SSE, a content coding) and WebSocket with permessage-deflate, from a script. It records
//! every byte it receives as the bytes arrive, so a test can read them while a connection is open.
//!
//! One tokio runtime on its own thread serves a held port; dropping the [`Upstream`] stops it on
//! that thread, so a `#[tokio::test]` may drop it. HTTP/2 over the `h2` crate comes in PRD-14
//! slice 3: today a connection whose TLS chose `h2` panics its task with a clear line.

use super::pki::{self, TestPki};
use super::ports;
use base64::Engine as _;
use flate2::{Compress, Compression, FlushCompress};
use sha1::{Digest, Sha1};
use std::io::{self, Read as _};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::oneshot;

/// TLS or not, and the ALPN protocols offered (`pki::HTTP11`, `pki::H2`).
#[derive(Clone, Debug)]
pub enum Tls {
    Off,
    On {
        /// DNS names or IP addresses on the leaf.
        names: Vec<String>,
        alpn: Vec<Vec<u8>>,
    },
}

/// How a response body is sent.
#[derive(Clone, Debug)]
pub enum Body {
    /// With `Content-Length`.
    Whole(Vec<u8>),
    /// `Transfer-Encoding: chunked`, one chunk per piece (an empty piece is skipped, since an
    /// empty chunk ends the body).
    Chunked(Vec<Vec<u8>>),
    /// `text/event-stream`, each piece flushed on its own (chunked on HTTP/1.1).
    Sse(Vec<Vec<u8>>),
}

/// A `Content-Encoding` the body is compressed with before sending. The whole body (every piece
/// joined) is compressed once, and for `Chunked` and `Sse` the compressed bytes are split again
/// over the same number of pieces. `Deflate` is the zlib format (RFC 9110 section 8.4.1.2); `Br`
/// is a valid brotli stream of uncompressed meta-blocks, since no brotli encoder is a dependency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coding {
    Gzip,
    Deflate,
    Br,
    Zstd,
}

/// One WebSocket message the upstream sends.
#[derive(Clone, Debug)]
pub enum WsMsg {
    Text(String),
    Binary(Vec<u8>),
    Close(u16),
}

/// What the upstream answers one request with.
#[derive(Clone, Debug)]
pub enum Answer {
    /// HTTP/1.1, or HTTP/2 when the connection chose `h2`.
    Http {
        status: u16,
        headers: Vec<(String, String)>,
        body: Body,
        coding: Option<Coding>,
    },
    /// `101`, then these messages, compressed with permessage-deflate when `deflate` and the
    /// client offered it (context takeover kept both ways). The upstream then reads the client's
    /// frames until the client closes, and answers a client Close with a Close.
    Ws { deflate: bool, messages: Vec<WsMsg> },
}

/// The upstream's script: one answer per request, in order across every connection (the last
/// repeats).
#[derive(Clone, Debug)]
pub struct Script {
    pub tls: Tls,
    pub answers: Vec<Answer>,
    /// Hold the TLS handshake (or, without TLS, the first read) this long.
    pub hold_handshake: Option<Duration>,
    /// Hold each answer this long after its request arrived.
    pub hold_answer: Option<Duration>,
    /// Close the connection once this many answer bytes are sent on it, counting from its first
    /// answer byte.
    pub close_after: Option<u64>,
}

impl Script {
    /// Plain TCP, one `200` with `body`.
    pub fn ok(body: &[u8]) -> Script {
        Script {
            tls: Tls::Off,
            answers: vec![Answer::Http {
                status: 200,
                headers: Vec::new(),
                body: Body::Whole(body.to_vec()),
                coding: None,
            }],
            hold_handshake: None,
            hold_answer: None,
            close_after: None,
        }
    }

    /// The same script over TLS: a leaf from the upstream's test CA ([`Upstream::pki`]) for
    /// `names`, offering `alpn` in order.
    pub fn with_tls(mut self, names: &[&str], alpn: &[&[u8]]) -> Script {
        self.tls = Tls::On {
            names: names.iter().map(|n| n.to_string()).collect(),
            alpn: alpn.iter().map(|p| p.to_vec()).collect(),
        };
        self
    }
}

/// What one connection sent the upstream, as raw bytes after TLS.
#[derive(Clone, Debug, Default)]
pub struct Received {
    pub conn: u64,
    pub bytes: Vec<u8>,
}

/// A running fake upstream on a held loopback port; it stops when dropped.
pub struct Upstream {
    addr: SocketAddr,
    received: Arc<Mutex<Vec<Received>>>,
    pki: TestPki,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Upstream {
    /// Starts serving `script` on a held port (`ports::held`).
    pub fn start(script: Script) -> Upstream {
        assert!(
            !script.answers.is_empty(),
            "an upstream script needs an answer"
        );
        let (listener, addr) = ports::held();
        listener
            .set_nonblocking(true)
            .expect("a non-blocking listener");
        let pki = TestPki::new("goalcup test upstream CA");
        let tls = match &script.tls {
            Tls::Off => None,
            Tls::On { names, alpn } => {
                let names: Vec<&str> = names.iter().map(String::as_str).collect();
                let alpn: Vec<&[u8]> = alpn.iter().map(Vec::as_slice).collect();
                Some(tokio_rustls::TlsAcceptor::from(
                    pki.server_config(&names, &alpn),
                ))
            }
        };
        let received = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::new(Shared {
            script,
            tls,
            received: received.clone(),
            next: AtomicUsize::new(0),
        });
        let (stop, stopped) = oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("fake-upstream".into())
            .spawn(move || serve(listener, shared, stopped))
            .expect("start the upstream thread");
        Upstream {
            addr,
            received,
            pki,
            stop: Some(stop),
            thread: Some(thread),
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The test CA behind the TLS leaf: trust `pki().root()` (or `client_config`, or
    /// `roots_file` for `GOALCUP_TEST_EXTRA_ROOTS`) to reach this upstream over TLS.
    pub fn pki(&self) -> &TestPki {
        &self.pki
    }

    /// Every connection so far, in accept order.
    pub fn received(&self) -> Vec<Received> {
        lock(&self.received).clone()
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Shared {
    script: Script,
    tls: Option<tokio_rustls::TlsAcceptor>,
    received: Arc<Mutex<Vec<Received>>>,
    next: AtomicUsize,
}

impl Shared {
    fn next_answer(&self) -> &Answer {
        let i = self.next.fetch_add(1, Ordering::SeqCst);
        &self.script.answers[i.min(self.script.answers.len() - 1)]
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The runtime lives and dies on this thread, never inside the caller's async context.
fn serve(listener: std::net::TcpListener, shared: Arc<Shared>, stopped: oneshot::Receiver<()>) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("the upstream runtime");
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::from_std(listener).expect("a tokio listener");
        let accept = async {
            let mut conn = 0u64;
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    continue;
                };
                lock(&shared.received).push(Received {
                    conn,
                    bytes: Vec::new(),
                });
                tokio::spawn(connection(shared.clone(), tcp, conn));
                conn += 1;
            }
        };
        tokio::select! {
            _ = accept => {}
            _ = stopped => {}
        }
    });
    rt.shutdown_background();
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

async fn connection(shared: Arc<Shared>, tcp: tokio::net::TcpStream, conn: u64) {
    if let Some(hold) = shared.script.hold_handshake {
        tokio::time::sleep(hold).await;
    }
    let io: Box<dyn Io> = match &shared.tls {
        None => Box::new(tcp),
        Some(acceptor) => {
            let Ok(tls) = acceptor.accept(tcp).await else {
                return;
            };
            if tls.get_ref().1.alpn_protocol() == Some(pki::H2) {
                panic!("fake upstream: the connection chose h2; HTTP/2 comes in PRD-14 slice 3");
            }
            Box::new(tls)
        }
    };
    let mut c = Conn {
        io,
        buf: Vec::new(),
        shared,
        conn: conn as usize,
        written: 0,
    };
    let _ = c.http1().await;
    let _ = c.io.shutdown().await;
}

/// The request head, with header names in lower case.
struct Request {
    headers: Vec<(String, String)>,
}

impl Request {
    fn parse(head: &[u8]) -> Request {
        let text = String::from_utf8_lossy(head);
        let headers = text
            .split("\r\n")
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        Request { headers }
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn has(&self, name: &str, token: &str) -> bool {
        self.header(name)
            .is_some_and(|v| v.to_ascii_lowercase().contains(token))
    }
}

struct Conn {
    io: Box<dyn Io>,
    buf: Vec<u8>,
    shared: Arc<Shared>,
    conn: usize,
    written: u64,
}

impl Conn {
    /// Reads what is there, records it, and fails at the end of the stream.
    async fn fill(&mut self) -> io::Result<()> {
        let mut tmp = [0u8; 16 * 1024];
        let n = self.io.read(&mut tmp).await?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        lock(&self.shared.received)[self.conn]
            .bytes
            .extend_from_slice(&tmp[..n]);
        self.buf.extend_from_slice(&tmp[..n]);
        Ok(())
    }

    async fn take(&mut self, n: usize) -> io::Result<Vec<u8>> {
        while self.buf.len() < n {
            self.fill().await?;
        }
        Ok(self.buf.drain(..n).collect())
    }

    /// Everything up to and including `end`.
    async fn until(&mut self, end: &[u8]) -> io::Result<Vec<u8>> {
        loop {
            if let Some(i) = self.buf.windows(end.len()).position(|w| w == end) {
                return Ok(self.buf.drain(..i + end.len()).collect());
            }
            self.fill().await?;
        }
    }

    /// Writes answer bytes and closes the connection once `close_after` of them are out.
    async fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        let limit = self.shared.script.close_after;
        let room = limit.map_or(u64::MAX, |n| n.saturating_sub(self.written));
        let n = (bytes.len() as u64).min(room) as usize;
        self.io.write_all(&bytes[..n]).await?;
        self.io.flush().await?;
        self.written += n as u64;
        if limit.is_some_and(|limit| self.written >= limit) {
            self.io.shutdown().await?;
            return Err(io::Error::other("closed after the scripted byte count"));
        }
        Ok(())
    }

    async fn http1(&mut self) -> io::Result<()> {
        loop {
            let req = Request::parse(&self.until(b"\r\n\r\n").await?);
            if let Some(len) = req.header("content-length") {
                self.take(len.parse().unwrap_or(0)).await?;
            } else if req.has("transfer-encoding", "chunked") {
                self.skip_chunked().await?;
            }
            let shared = self.shared.clone();
            let answer = shared.next_answer();
            if let Some(hold) = shared.script.hold_answer {
                tokio::time::sleep(hold).await;
            }
            match answer {
                Answer::Http {
                    status,
                    headers,
                    body,
                    coding,
                } => self.answer(*status, headers, body, *coding).await?,
                Answer::Ws { deflate, messages } => {
                    return self.websocket(&req, *deflate, messages).await;
                }
            }
            if req.has("connection", "close") {
                return Ok(());
            }
        }
    }

    async fn skip_chunked(&mut self) -> io::Result<()> {
        loop {
            let line = self.until(b"\r\n").await?;
            let text = String::from_utf8_lossy(&line);
            let size = text.split(';').next().unwrap_or("").trim();
            let size = usize::from_str_radix(size, 16).unwrap_or(0);
            if size == 0 {
                while self.until(b"\r\n").await?.len() > 2 {}
                return Ok(());
            }
            self.take(size + 2).await?;
        }
    }

    async fn answer(
        &mut self,
        status: u16,
        headers: &[(String, String)],
        body: &Body,
        coding: Option<Coding>,
    ) -> io::Result<()> {
        let pieces = match body {
            Body::Whole(b) => vec![b.clone()],
            Body::Chunked(p) | Body::Sse(p) => p.clone(),
        };
        let pieces = match coding {
            None => pieces,
            Some(c) => {
                let coded = encode(c, &pieces.concat());
                let size = coded.len().div_ceil(pieces.len().max(1)).max(1);
                coded.chunks(size).map(<[u8]>::to_vec).collect()
            }
        };
        let reason = if status == 200 { "OK" } else { "Answer" };
        let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
        for (k, v) in headers {
            head += &format!("{k}: {v}\r\n");
        }
        if let Some(c) = coding {
            // The Debug names are the tokens in lower case: gzip, deflate, br, zstd.
            let name = format!("{c:?}").to_lowercase();
            head += &format!("content-encoding: {name}\r\n");
        }
        if let Body::Whole(_) = body {
            let len: usize = pieces.iter().map(Vec::len).sum();
            head += &format!("content-length: {len}\r\n\r\n");
            let mut out = head.into_bytes();
            out.extend(pieces.concat());
            return self.send(&out).await;
        }
        let typed = headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"));
        if matches!(body, Body::Sse(_)) && !typed {
            head += "content-type: text/event-stream\r\n";
        }
        head += "transfer-encoding: chunked\r\n\r\n";
        self.send(head.as_bytes()).await?;
        for piece in pieces.iter().filter(|p| !p.is_empty()) {
            let mut chunk = format!("{:x}\r\n", piece.len()).into_bytes();
            chunk.extend_from_slice(piece);
            chunk.extend_from_slice(b"\r\n");
            self.send(&chunk).await?;
        }
        self.send(b"0\r\n\r\n").await
    }

    async fn websocket(
        &mut self,
        req: &Request,
        deflate: bool,
        messages: &[WsMsg],
    ) -> io::Result<()> {
        const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
        let key = req.header("sec-websocket-key").unwrap_or("");
        let accept = base64::engine::general_purpose::STANDARD
            .encode(&Sha1::digest(format!("{key}{GUID}").as_bytes())[..]);
        let deflate = deflate && req.has("sec-websocket-extensions", "permessage-deflate");
        let mut head = format!(
            "HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nconnection: Upgrade\r\n\
             sec-websocket-accept: {accept}\r\n"
        );
        if deflate {
            head += "sec-websocket-extensions: permessage-deflate\r\n";
        }
        head += "\r\n";
        self.send(head.as_bytes()).await?;

        let mut compress = Compress::new(Compression::default(), false);
        let mut closed = false;
        for msg in messages {
            let (opcode, payload) = match msg {
                WsMsg::Text(t) => (1, t.as_bytes().to_vec()),
                WsMsg::Binary(b) => (2, b.clone()),
                WsMsg::Close(code) => (8, code.to_be_bytes().to_vec()),
            };
            let frame = if deflate && opcode != 8 {
                ws_frame(opcode, true, &deflate_message(&mut compress, &payload))
            } else {
                ws_frame(opcode, false, &payload)
            };
            self.send(&frame).await?;
            closed |= opcode == 8;
        }

        // The client's frames: masked, maybe compressed and fragmented. `fill` has already
        // recorded their raw bytes; only a Close is looked at.
        loop {
            let h = self.take(2).await?;
            let len = match h[1] & 0x7f {
                126 => u16::from_be_bytes(self.take(2).await?.try_into().expect("2 bytes")) as u64,
                127 => u64::from_be_bytes(self.take(8).await?.try_into().expect("8 bytes")),
                n => n as u64,
            };
            let mask = if h[1] & 0x80 != 0 {
                self.take(4).await?
            } else {
                vec![0; 4]
            };
            let mut payload = self.take(len as usize).await?;
            if h[0] & 0x0f == 8 {
                if !closed {
                    for (i, b) in payload.iter_mut().enumerate() {
                        *b ^= mask[i % 4];
                    }
                    payload.truncate(2);
                    self.send(&ws_frame(8, false, &payload)).await?;
                }
                return Ok(());
            }
        }
    }
}

fn encode(coding: Coding, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let level = Compression::default();
    let read = match coding {
        Coding::Gzip => flate2::read::GzEncoder::new(body, level).read_to_end(&mut out),
        Coding::Deflate => flate2::read::ZlibEncoder::new(body, level).read_to_end(&mut out),
        Coding::Zstd => return zstd::stream::encode_all(body, 0).expect("zstd"),
        Coding::Br => return brotli_stored(body),
    };
    read.expect("compress a body");
    out
}

/// A brotli stream (RFC 7932 section 9.2) of uncompressed meta-blocks: WBITS 16 (one 0 bit); per
/// block of at most 64 KiB, ISLAST 0, MNIBBLES 4, MLEN - 1 in 16 bits, ISUNCOMPRESSED 1, zero bits
/// to the byte boundary and the bytes; then ISLAST 1 and ISLASTEMPTY 1.
fn brotli_stored(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + body.len() / 65536 * 4 + 8);
    let (mut acc, mut n) = (0u32, 0u32);
    let mut put = |out: &mut Vec<u8>, v: u32, bits: u32, align: bool| {
        acc |= v << n;
        n += bits;
        while n >= 8 || (align && n > 0) {
            out.push(acc as u8);
            acc >>= 8;
            n = n.saturating_sub(8);
        }
    };
    put(&mut out, 0, 1, false);
    for block in body.chunks(1 << 16) {
        put(&mut out, 0, 3, false);
        put(&mut out, block.len() as u32 - 1, 16, false);
        put(&mut out, 1, 1, true);
        out.extend_from_slice(block);
    }
    put(&mut out, 0b11, 2, true);
    out
}

/// One unmasked frame from the server, FIN set.
fn ws_frame(opcode: u8, rsv1: bool, payload: &[u8]) -> Vec<u8> {
    let mut f = vec![0x80 | if rsv1 { 0x40 } else { 0 } | opcode];
    match payload.len() {
        n if n < 126 => f.push(n as u8),
        n if n <= 0xffff => {
            f.push(126);
            f.extend((n as u16).to_be_bytes());
        }
        n => {
            f.push(127);
            f.extend((n as u64).to_be_bytes());
        }
    }
    f.extend_from_slice(payload);
    f
}

/// permessage-deflate (RFC 7692): raw deflate on the one stream of this direction, a sync flush,
/// and the `00 00 ff ff` tail stripped.
fn deflate_message(c: &mut Compress, data: &[u8]) -> Vec<u8> {
    let start = c.total_in();
    let mut out = Vec::with_capacity(data.len() / 2 + 64);
    loop {
        if out.len() == out.capacity() {
            out.reserve(out.capacity().max(64));
        }
        let used = (c.total_in() - start) as usize;
        c.compress_vec(&data[used..], &mut out, FlushCompress::Sync)
            .expect("deflate a message");
        if (c.total_in() - start) as usize == data.len() && out.len() < out.capacity() {
            break;
        }
    }
    assert!(
        out.ends_with(&[0, 0, 0xff, 0xff]),
        "a sync flush ends in 00 00 ff ff"
    );
    out.truncate(out.len() - 4);
    out
}
