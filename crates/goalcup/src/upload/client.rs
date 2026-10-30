//! The one blocking HTTP client for the upload server and the documents of seam S5
//! (`docs/UPLOAD-API.md`): one request, sent once, its answer read with a bound; retrying is the
//! caller's. TLS is this crate's rustls (ring, webpki roots) through ureq's connector hook; the
//! base address comes from `cli::server()`. Answer types ignore unknown fields (`ack`, `last_ack`).

use super::policy::after;
use crate::pipeline::Mono;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

/// How long a connection may take to open.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a read or a write may go without moving a byte.
const STALL_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a chunk upload may take as a whole, on boot time.
const PUT_TIMEOUT: Duration = Duration::from_secs(600);
/// How long a JSON call may take as a whole.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// The most of an answer read. Every answer in docs/UPLOAD-API.md is a few kilobytes.
const MAX_ANSWER_BYTES: u64 = 256 * 1024;

/// The run credential (`gcr_<64 hex>`). `Debug` never prints it, and there is no `Display`.
#[derive(Clone)]
pub struct Token(String);

impl Token {
    pub fn new(secret: impl Into<String>) -> Token {
        Token(secret.into())
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

/// Why a call did not return the answer it asked for. Never holds a URL, a header value or a
/// body byte of the request. Every kind is retried by a background upload (PRD-06 section 5,
/// D64); only `goalcup upload` by hand gives up.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ServerError {
    /// No answer: not reached, the connection broke, a TLS handshake failed, or a timeout.
    /// The text is the failure's kind and its OS or TLS cause.
    #[error("the server could not be reached ({0})")]
    Transport(String),
    /// An answer that is not a success, with the server's `error` code and sentence when it
    /// sent them (control characters dropped, bounded), and its `Retry-After` in seconds.
    #[error("the server answered HTTP {status}{}", said(.code, .message))]
    Status {
        status: u16,
        code: String,
        message: String,
        retry_after: Option<u64>,
    },
    /// A success whose body this client could not read.
    #[error("the server's answer is unreadable ({0})")]
    Malformed(String),
}

impl ServerError {
    /// The server's `Retry-After`, in seconds, if it sent one.
    pub fn retry_after(&self) -> Option<u64> {
        match self {
            ServerError::Status { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

/// ` code: message`, leaving out what the server did not send.
fn said(code: &str, message: &str) -> String {
    let mut s = String::new();
    if !code.is_empty() {
        s = format!(" {code}");
    }
    if !message.is_empty() {
        s.push_str(": ");
        s.push_str(message);
    }
    s
}

/// `GET /v1/runs/{run}?entry_id=`: where the server is, in the fields the uploader reads.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RunPosition {
    /// Raw GCR1 bytes of the entry the server holds.
    pub offset: u64,
    /// The LINK `seq` the next chunk starts at.
    pub next_seq: u64,
    /// The link hash at `offset`; 64 zeros before any chunk.
    pub head: String,
    #[serde(default)]
    pub sealed: bool,
    /// The run is over (the server's clock is at or past `ends_at`).
    #[serde(default)]
    pub stop: bool,
    /// The end a final heartbeat committed, if one has.
    #[serde(rename = "final")]
    pub committed: Option<ChainEnd>,
}

/// One LINK: its `seq` and hash.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ChainEnd {
    pub seq: u64,
    pub head: String,
}

/// The headers of `PUT /v1/runs/{run}/chunks/{offset}`. Hashes are lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkHeaders {
    pub entry_id: String,
    /// The path offset: the raw offset of the chunk's first byte.
    pub offset: u64,
    /// `Goalcup-Length`: the raw bytes the chunk covers.
    pub raw_len: u64,
    pub first_seq: u64,
    pub last_seq: u64,
    pub head: String,
    pub prev: String,
    /// SHA-256 of the compressed body.
    pub sha256: String,
}

/// The answer to a stored chunk.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ChunkAnswer {
    pub offset: u64,
    pub next_seq: u64,
    pub head: String,
    #[serde(default)]
    pub received_at: String,
    #[serde(default)]
    pub sealed: bool,
}

/// The recording state a heartbeat reports (PRD-06 section 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Recording {
    Ok,
    Gaps,
    Failed,
}

/// `POST /v1/runs/{run}/heartbeat`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HeartbeatRequest {
    pub entry_id: String,
    /// The durable point: the last LINK on disk, its hash, and the raw offset just past it.
    pub seq: u64,
    pub head: String,
    pub offset_local: u64,
    #[serde(rename = "final")]
    pub is_final: bool,
    /// The seal's `workspace_head` on the final heartbeat, else `None`.
    pub repo_head: Option<String>,
    pub recording: Recording,
    pub model_requests: u64,
    /// One flat object of written counts and off time, at most 2 KiB.
    pub counts: serde_json::Map<String, serde_json::Value>,
}

/// The answer to a heartbeat.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HeartbeatAnswer {
    /// The run is over: stop now.
    pub stop: bool,
    #[serde(default)]
    pub deadline_at: String,
    /// Whole seconds from the server's clock to the deadline when it answered.
    pub seconds_left: Option<u64>,
    /// `deadline` or `replaced` while `stop`, else `None`.
    pub reason: Option<String>,
    #[serde(default)]
    pub sealed: bool,
}

/// An error answer's body. Missing fields read as empty: a proxy's HTML page is still an error.
#[derive(Default, Deserialize)]
#[serde(default)]
struct ErrorBody {
    error: String,
    message: String,
}

/// A client for one server. `Debug` prints the URL only.
#[derive(Clone)]
pub struct ServerClient {
    base: String,
    token: Option<Token>,
    /// JSON calls: a TLS connection lives CALL_TIMEOUT; chunk uploads: PUT_TIMEOUT.
    calls: ureq::Agent,
    puts: ureq::Agent,
}

impl std::fmt::Debug for ServerClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerClient")
            .field("base", &self.base)
            .field("token", &self.token)
            .finish()
    }
}

impl ServerClient {
    /// A client for `base` (no trailing slash needed) that sends `token` on every call: `None`
    /// for the device flow, the run credential after it. `base` comes from `cli::server()`.
    pub fn new(base: &str, token: Option<Token>) -> ServerClient {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map(|b| Arc::new(b.with_root_certificates(roots).with_no_client_auth()));
        let agent = |whole| {
            let mut agent = ureq::AgentBuilder::new()
                // One connection per call, so each call resolves the name again.
                .max_idle_connections(0)
                .max_idle_connections_per_host(0)
                // A redirect would carry the token to an address nobody configured.
                .redirects(0)
                // The token never goes through a proxy.
                .try_proxy_from_env(false)
                .timeout_connect(CONNECT_TIMEOUT)
                .timeout_read(STALL_TIMEOUT)
                .timeout_write(STALL_TIMEOUT);
            // The ring provider takes both default versions, so this holds; if not, https fails.
            if let Ok(tls) = &tls {
                agent = agent.tls_connector(Arc::new(RustlsConnector(tls.clone(), whole)));
            }
            agent.build()
        };
        ServerClient {
            base: base.trim_end_matches('/').to_string(),
            token,
            calls: agent(CALL_TIMEOUT),
            puts: agent(PUT_TIMEOUT),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// `method path` with a JSON body or none, answered by JSON. The request's own 30 s bounds
    /// the connect and a plain call; a TLS connection also ends 30 s after it opened, a peer that
    /// trickles bytes included (`Bounded`). PRD-10's device, open and continue calls use it.
    pub fn json<T, B>(&self, method: &str, path: &str, body: Option<&B>) -> Result<T, ServerError>
    where
        T: serde::de::DeserializeOwned,
        B: Serialize + ?Sized,
    {
        let request = self
            .request(&self.calls, method, path)
            .timeout(CALL_TIMEOUT)
            .set("Accept", "application/json");
        let sent = match body {
            Some(body) => {
                let text = serde_json::to_string(body)
                    .map_err(|e| ServerError::Malformed(format!("request body: {e}")))?;
                request
                    .set("Content-Type", "application/json")
                    .send_string(&text)
            }
            None => request.call(),
        };
        parse(answer(sent)?.into_reader())
    }

    /// `GET /v1/runs/{run}?entry_id={entry}`.
    pub fn position(&self, run: &str, entry: &str) -> Result<RunPosition, ServerError> {
        self.json::<_, ()>("GET", &format!("/v1/runs/{run}?entry_id={entry}"), None)
    }

    /// `POST /v1/runs/{run}/heartbeat`.
    pub fn heartbeat(
        &self,
        run: &str,
        beat: &HeartbeatRequest,
    ) -> Result<HeartbeatAnswer, ServerError> {
        self.json("POST", &format!("/v1/runs/{run}/heartbeat"), Some(beat))
    }

    /// `PUT /v1/runs/{run}/chunks/{offset}`, the body streamed from `body` (`content_length`
    /// bytes), never held in memory. ureq writes the whole body before it reads an answer, so a
    /// refusal sent before the body arrives as a broken connection; a broken PUT therefore reads
    /// the position once and answers with the server's word: its refusal, the chunk's answer when
    /// the server holds it, 409 when it stands elsewhere, 503 `awaiting_final` when the run is
    /// over with no final committed. Only a server waiting on this very chunk gives `Transport`.
    pub fn put_chunk(
        &self,
        run: &str,
        h: &ChunkHeaders,
        body: impl Read,
        content_length: u64,
    ) -> Result<ChunkAnswer, ServerError> {
        // No request timeout: ureq 2 drops the agent's 60 s stall on a request with a deadline.
        // The whole-PUT bound is the TLS connection's (`Bounded`); a local `http://` Worker has
        // none.
        let path = format!("/v1/runs/{run}/chunks/{}", h.offset);
        let request = self
            .request(&self.puts, "PUT", &path)
            .set("Accept", "application/json")
            .set("Content-Type", "application/zstd")
            .set("Content-Length", &content_length.to_string())
            .set("Goalcup-Entry-Id", &h.entry_id)
            .set("Goalcup-Length", &h.raw_len.to_string())
            .set("Goalcup-Sha256", &h.sha256)
            .set("Goalcup-First-Seq", &h.first_seq.to_string())
            .set("Goalcup-Last-Seq", &h.last_seq.to_string())
            .set("Goalcup-Prev", &h.prev)
            .set("Goalcup-Head", &h.head);
        match answer(request.send(body)) {
            Ok(response) => parse(response.into_reader()),
            Err(ServerError::Transport(why)) => self.after_broken_put(run, h, why),
            Err(e) => Err(e),
        }
    }

    /// What the server says about a PUT whose connection broke (see [`Self::put_chunk`]).
    fn after_broken_put(
        &self,
        run: &str,
        h: &ChunkHeaders,
        why: String,
    ) -> Result<ChunkAnswer, ServerError> {
        let at = self.position(run, &h.entry_id)?;
        let read = |status, code: &str| ServerError::Status {
            status,
            code: code.to_string(),
            message: format!("read from the position after the upload broke ({why})"),
            retry_after: None,
        };
        if at.offset == h.offset + h.raw_len && at.head == h.head && at.next_seq == h.last_seq + 1 {
            Ok(ChunkAnswer {
                offset: at.offset,
                next_seq: at.next_seq,
                head: at.head,
                received_at: String::new(),
                sealed: at.sealed,
            })
        } else if at.offset != h.offset || at.head != h.prev || at.next_seq != h.first_seq {
            Err(read(409, "conflict"))
        } else if at.stop && at.committed.is_none() {
            Err(read(503, "awaiting_final"))
        } else {
            Err(ServerError::Transport(why))
        }
    }

    fn request(&self, agent: &ureq::Agent, method: &str, path: &str) -> ureq::Request {
        let request = agent.request(method, &format!("{}{path}", self.base));
        match &self.token {
            Some(Token(secret)) => request.set("Authorization", &format!("Bearer {secret}")),
            None => request,
        }
    }
}

/// A success, or the error the answer describes.
fn answer(sent: Result<ureq::Response, ureq::Error>) -> Result<ureq::Response, ServerError> {
    match sent {
        Ok(response) => Ok(response),
        Err(ureq::Error::Status(status, response)) => {
            // Whole seconds only, as the Worker sends it; an HTTP-date reads as absent.
            let retry_after = response
                .header("Retry-After")
                .and_then(|v| v.trim().parse::<u64>().ok());
            let body = read_bounded(response.into_reader()).unwrap_or_default();
            let error: ErrorBody = serde_json::from_slice(&body).unwrap_or_default();
            Err(ServerError::Status {
                status,
                code: printable(&error.error, 40),
                message: printable(&error.message, 300),
                retry_after,
            })
        }
        // The kind and the OS or TLS cause; ureq's own text names the URL, which holds the run
        // and entry ids.
        Err(ureq::Error::Transport(t)) => {
            let cause = std::error::Error::source(&t)
                .and_then(|e| e.downcast_ref::<std::io::Error>())
                .map(|io| format!(": {io}"));
            Err(ServerError::Transport(format!(
                "{}{}",
                t.kind(),
                cause.unwrap_or_default()
            )))
        }
    }
}

/// The server's words, safe for a terminal: no control characters, at most `max` characters.
fn printable(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(max).collect()
}

fn parse<T: serde::de::DeserializeOwned>(reader: impl Read) -> Result<T, ServerError> {
    let body = read_bounded(reader)?;
    serde_json::from_slice(&body).map_err(|e| ServerError::Malformed(e.to_string()))
}

fn read_bounded(reader: impl Read) -> Result<Vec<u8>, ServerError> {
    let mut body = Vec::new();
    reader
        .take(MAX_ANSWER_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|e| ServerError::Transport(format!("reading the answer: {e}")))?;
    if body.len() as u64 > MAX_ANSWER_BYTES {
        return Err(ServerError::Malformed(format!(
            "an answer larger than {MAX_ANSWER_BYTES} bytes"
        )));
    }
    Ok(body)
}

/// TLS for `ureq`: rustls on ring, checked against the web roots and the host name, on a
/// connection that lives at most the given time.
struct RustlsConnector(Arc<rustls::ClientConfig>, Duration);

impl ureq::TlsConnector for RustlsConnector {
    fn connect(
        &self,
        dns_name: &str,
        io: Box<dyn ureq::ReadWrite>,
    ) -> Result<Box<dyn ureq::ReadWrite>, ureq::Error> {
        // Each call has its own connection: the handshake within CALL_TIMEOUT, the rest within
        // the agent's time, both from the same start.
        let start = Mono::now();
        let mut io = Bounded {
            inner: io,
            until: after(start, CALL_TIMEOUT),
        };
        let host = dns_name.trim_start_matches('[').trim_end_matches(']');
        let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("server name: {e}"),
            )
        })?;
        let mut session = rustls::ClientConnection::new(Arc::clone(&self.0), name)
            .map_err(|e| std::io::Error::other(format!("TLS setup: {e}")))?;
        // complete_io may return mid-handshake after a partial read; finish it within the bound.
        while session.is_handshaking() {
            session.complete_io(&mut io)?;
        }
        io.until = after(start, self.1);
        Ok(Box::new(TlsStream(rustls::StreamOwned::new(session, io))))
    }
}

struct TlsStream(rustls::StreamOwned<rustls::ClientConnection, Bounded>);

impl ureq::ReadWrite for TlsStream {
    fn socket(&self) -> Option<&std::net::TcpStream> {
        self.0.get_ref().inner.socket()
    }
}

/// A connection whose reads and writes end by `until` (boot time): each one waits at most the
/// time left, or STALL_TIMEOUT if less, so a peer that sends a byte now and then cannot hold it.
struct Bounded {
    inner: Box<dyn ureq::ReadWrite>,
    until: Mono,
}

impl Bounded {
    fn left(&self) -> std::io::Result<Duration> {
        match self.until.0.checked_sub(Mono::now().0) {
            Some(n) if n > 0 => Ok(Duration::from_nanos(n).min(STALL_TIMEOUT)),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "the call ran past its time",
            )),
        }
    }
}

impl Read for Bounded {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.left()?;
        if let Some(socket) = self.inner.socket() {
            socket.set_read_timeout(Some(left))?;
        }
        self.inner.read(buf)
    }
}

impl std::io::Write for Bounded {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let left = self.left()?;
        if let Some(socket) = self.inner.socket() {
            socket.set_write_timeout(Some(left))?;
        }
        self.inner.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl Read for TlsStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl std::io::Write for TlsStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl std::fmt::Debug for TlsStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TlsStream")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;

    /// Reads a request up to the end of its headers, never its body.
    fn read_head(stream: &mut TcpStream) -> String {
        let mut seen = Vec::new();
        let mut buf = [0u8; 1024];
        while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            seen.extend_from_slice(&buf[..n]);
        }
        String::from_utf8_lossy(&seen).into_owned()
    }

    fn http(status: &str, extra: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{extra}\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    /// Point 4: a refused call never shows the token, in the error, its text or its Debug.
    #[test]
    fn a_refused_call_never_shows_the_token() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let seen = read_head(&mut stream);
            let body = r#"{"error":"unauthorized","message":"no such credential"}"#;
            let answer = http("401 Unauthorized", "", body);
            stream.write_all(answer.as_bytes()).unwrap();
            seen
        });
        let secret = format!("gcr_{}", "5e".repeat(32));
        let client = ServerClient::new(
            &format!("http://127.0.0.1:{port}/"),
            Some(Token::new(secret.clone())),
        );
        let error = client
            .position("r0123456789abcdef", "e0123456789abcdef")
            .unwrap_err();
        let request = server.join().unwrap();
        assert!(request.contains(&secret), "the token was sent");
        assert!(matches!(error, ServerError::Status { status: 401, .. }));
        for shown in [
            format!("{error}"),
            format!("{error:?}"),
            format!("{client:?}"),
        ] {
            assert!(!shown.contains(&secret));
        }
    }

    /// PRD-06 section 5: a PUT the server refuses before it reads the body breaks the connection
    /// (ureq writes the whole body first), so the client reads the position at once and answers
    /// with the server's word, not a network failure.
    #[test]
    fn a_put_refused_before_its_body_answers_with_the_servers_word() {
        let b = "b".repeat(64);
        let elsewhere = format!(r#"{{"offset":4096,"next_seq":9,"head":"{b}"}}"#);
        let conflict = http("409 Conflict", "", "{}");
        let busy = http("429 Too Many Requests", "Retry-After: 7\r\n", "{}");
        let cases = [
            (conflict, http("200 OK", "", &elsewhere), 409, None),
            (busy.clone(), busy, 429, Some(7)),
        ];
        for (put_answer, position_answer, status, retry_after) in cases {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let (seen, heads) = mpsc::channel();
            std::thread::spawn(move || {
                for answer in [put_answer, position_answer] {
                    let Ok((mut stream, _)) = listener.accept() else {
                        return;
                    };
                    let head = read_head(&mut stream);
                    let _ = stream.write_all(answer.as_bytes());
                    let _ = seen.send(head.lines().next().unwrap_or_default().to_string());
                    // Dropped with the body unread: the kernel resets the connection.
                }
            });
            let client = ServerClient::new(&format!("http://127.0.0.1:{port}"), None);
            let h = ChunkHeaders {
                entry_id: "e0123456789abcdef".into(),
                offset: 0,
                raw_len: 100,
                first_seq: 0,
                last_seq: 1,
                head: "a".repeat(64),
                prev: "0".repeat(64),
                sha256: "c".repeat(64),
            };
            let size = 32 << 20;
            let error = client
                .put_chunk("r0123456789abcdef", &h, std::io::repeat(0).take(size), size)
                .unwrap_err();
            let wait = Duration::from_secs(10);
            assert!(heads.recv_timeout(wait).unwrap().starts_with("PUT "));
            assert!(heads.recv_timeout(wait).unwrap().starts_with("GET "));
            let ServerError::Status { status: s, .. } = error else {
                panic!("{error:?}");
            };
            assert_eq!((s, error.retry_after()), (status, retry_after));
        }
    }
}
