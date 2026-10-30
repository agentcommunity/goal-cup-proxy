//! The one blocking HTTP client for the upload server, and the request and answer documents of
//! seam S5 (`docs/UPLOAD-API.md`). One request, sent once, its answer read with a bound;
//! retrying is the caller's.
//!
//! TLS is this crate's own rustls with the `ring` provider and the `webpki-roots` anchors,
//! plugged into `ureq` through its connector hook: the workspace builds `ureq` without TLS.
//! `http://` works too, for a local Worker under `wrangler dev` and the fake Worker.
//!
//! Every answer type ignores fields it does not know, so the same client reads a Worker that
//! still sends `ack` and `last_ack` and one that no longer does.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a connection may take to open.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a read or a write may go without moving a byte.
pub const STALL_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a chunk upload may take as a whole.
pub const PUT_TIMEOUT: Duration = Duration::from_secs(600);
/// How long a JSON call may take as a whole.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);
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

/// Why a call did not return the answer it asked for. Never holds a header value or a body
/// byte of the request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ServerError {
    /// No answer: not reached, the connection broke, or a timeout.
    #[error("the server could not be reached ({0})")]
    Transport(String),
    /// An answer that is not a success, with the server's `error` code and sentence when it
    /// sent them (control characters dropped, bounded), and its `Retry-After` in seconds.
    #[error("the server answered HTTP {status} {code}: {message}")]
    Status {
        status: u16,
        code: String,
        message: String,
        retry_after: Option<u64>,
    },
    /// A success whose body this client could not read.
    #[error("the server's answer is unreadable ({0})")]
    Malformed(String),
    /// The TLS handshake failed. Asking again does not change a certificate.
    #[error("the server's TLS connection was refused ({0})")]
    Tls(String),
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

/// `GET /v1/runs/{run}?entry_id=`: where the server is, in the fields the uploader reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    agent: ureq::Agent,
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
    /// A client for `base` (no trailing slash needed) that sends `token` on the run calls.
    pub fn new(base: &str, token: Option<Token>) -> ServerClient {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map(|b| Arc::new(b.with_root_certificates(roots).with_no_client_auth()));
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
        if let Ok(tls) = tls {
            agent = agent.tls_connector(Arc::new(RustlsConnector(tls)));
        }
        ServerClient {
            base: base.trim_end_matches('/').to_string(),
            token,
            agent: agent.build(),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// `method path` with a JSON body or none, sending `token` if given, answered by JSON, all
    /// within [`CALL_TIMEOUT`]. PRD-10's device, open and continue calls use it.
    pub fn json_timed<T, B>(
        &self,
        method: &str,
        path: &str,
        token: Option<&Token>,
        body: Option<&B>,
    ) -> Result<T, ServerError>
    where
        T: serde::de::DeserializeOwned,
        B: Serialize + ?Sized,
    {
        let until = Instant::now() + CALL_TIMEOUT;
        let request = self
            .request(method, path, token)
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
        parse(answer(sent, until)?, until)
    }

    /// `GET /v1/runs/{run}?entry_id={entry}`.
    pub fn position(&self, run: &str, entry: &str) -> Result<RunPosition, ServerError> {
        let path = format!("/v1/runs/{run}?entry_id={entry}");
        self.json_timed::<_, ()>("GET", &path, self.token.as_ref(), None)
    }

    /// `POST /v1/runs/{run}/heartbeat`.
    pub fn heartbeat(
        &self,
        run: &str,
        beat: &HeartbeatRequest,
    ) -> Result<HeartbeatAnswer, ServerError> {
        let path = format!("/v1/runs/{run}/heartbeat");
        self.json_timed("POST", &path, self.token.as_ref(), Some(beat))
    }

    /// `PUT /v1/runs/{run}/chunks/{offset}`, the body streamed from `body`, which yields exactly
    /// `content_length` bytes; it is never held in memory.
    pub fn put_chunk(
        &self,
        run: &str,
        h: &ChunkHeaders,
        body: impl Read,
        content_length: u64,
    ) -> Result<ChunkAnswer, ServerError> {
        // ureq 2 drops the agent's read and write timeouts on a request with its own deadline,
        // so the whole-PUT bound is kept here, between reads and writes, and the stall bound
        // stays on the socket.
        let until = Instant::now() + PUT_TIMEOUT;
        let request = self
            .request(
                "PUT",
                &format!("/v1/runs/{run}/chunks/{}", h.offset),
                self.token.as_ref(),
            )
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
        let sent = request.send(Deadline { inner: body, until });
        parse(answer(sent, until)?, until)
    }

    fn request(&self, method: &str, path: &str, token: Option<&Token>) -> ureq::Request {
        let request = self.agent.request(method, &format!("{}{path}", self.base));
        match token {
            Some(Token(secret)) => request.set("Authorization", &format!("Bearer {secret}")),
            None => request,
        }
    }
}

/// A reader that fails once `until` has passed.
struct Deadline<R> {
    inner: R,
    until: Instant,
}

impl<R: Read> Read for Deadline<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if Instant::now() >= self.until {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "the call ran past its time",
            ));
        }
        self.inner.read(buf)
    }
}

/// A success, or the error the answer describes.
fn answer(
    sent: Result<ureq::Response, ureq::Error>,
    until: Instant,
) -> Result<ureq::Response, ServerError> {
    match sent {
        Ok(response) => Ok(response),
        Err(ureq::Error::Status(status, response)) => {
            let retry_after = response
                .header("Retry-After")
                .and_then(|v| v.trim().parse::<u64>().ok());
            let body = read_bounded(response, until).unwrap_or_default();
            let error: ErrorBody = serde_json::from_slice(&body).unwrap_or_default();
            Err(ServerError::Status {
                status,
                code: printable(&error.error, 40),
                message: printable(&error.message, 300),
                retry_after,
            })
        }
        // A transport error names the URL and the cause, never a header.
        Err(ureq::Error::Transport(t)) => Err(match tls_failure(&t) {
            Some(why) => ServerError::Tls(why),
            None => ServerError::Transport(t.to_string()),
        }),
    }
}

/// The server's words, safe for a terminal: no control characters, at most `max` characters.
fn printable(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(max).collect()
}

/// The rustls error under a transport failure, if it was the handshake.
fn tls_failure(t: &ureq::Transport) -> Option<String> {
    let mut source = std::error::Error::source(t);
    while let Some(error) = source {
        let tls = error.downcast_ref::<rustls::Error>().or_else(|| {
            error
                .downcast_ref::<std::io::Error>()
                .and_then(|io| io.get_ref())
                .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        });
        if let Some(tls) = tls {
            return Some(tls.to_string());
        }
        source = error.source();
    }
    None
}

fn parse<T: serde::de::DeserializeOwned>(
    response: ureq::Response,
    until: Instant,
) -> Result<T, ServerError> {
    let body = read_bounded(response, until)?;
    serde_json::from_slice(&body).map_err(|e| ServerError::Malformed(e.to_string()))
}

fn read_bounded(response: ureq::Response, until: Instant) -> Result<Vec<u8>, ServerError> {
    let mut body = Vec::new();
    let reader = Deadline {
        inner: response.into_reader(),
        until,
    };
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

/// TLS for `ureq`: rustls on ring, checked against the web roots and the host name.
struct RustlsConnector(Arc<rustls::ClientConfig>);

impl ureq::TlsConnector for RustlsConnector {
    fn connect(
        &self,
        dns_name: &str,
        mut io: Box<dyn ureq::ReadWrite>,
    ) -> Result<Box<dyn ureq::ReadWrite>, ureq::Error> {
        let host = dns_name.trim_start_matches('[').trim_end_matches(']');
        let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{host:?}: {e}"))
        })?;
        let mut session = rustls::ClientConnection::new(self.0.clone(), name)
            .map_err(|e| std::io::Error::other(format!("TLS setup: {e}")))?;
        session.complete_io(&mut io)?;
        Ok(Box::new(TlsStream(rustls::StreamOwned::new(session, io))))
    }
}

struct TlsStream(rustls::StreamOwned<rustls::ClientConnection, Box<dyn ureq::ReadWrite>>);

impl ureq::ReadWrite for TlsStream {
    fn socket(&self) -> Option<&std::net::TcpStream> {
        self.0.get_ref().socket()
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

    /// Answers from a Worker before PRD-07 step 3 (`ack`, `last_ack`) and with a key this
    /// client does not know still parse.
    #[test]
    fn answers_with_old_and_unknown_fields_parse() {
        let head = "a".repeat(64);
        let chunk = format!(
            r#"{{"offset":10,"next_seq":3,"head":"{head}","received_at":"2026-10-30T12:00:00Z",
                "sealed":false,"ack":{{"seq":2}},"later":1}}"#
        );
        let chunk: ChunkAnswer = serde_json::from_str(&chunk).unwrap();
        assert_eq!((chunk.offset, chunk.next_seq), (10, 3));
        let position = format!(
            r#"{{"run_id":"r0123456789abcdef","entry_id":"e0123456789abcdef","status":"open",
                "offset":10,"next_seq":3,"head":"{head}","last_ack":{{"seq":2}},"final":null,
                "later":[1]}}"#
        );
        let position: RunPosition = serde_json::from_str(&position).unwrap();
        assert_eq!((position.offset, position.next_seq), (10, 3));
        let beat: HeartbeatAnswer =
            serde_json::from_str(r#"{"stop":true,"reason":"deadline","later":{}}"#).unwrap();
        assert!(beat.stop);
    }

    /// Point 4: a refused call never shows the token, in the error, its text or its Debug.
    #[test]
    fn a_refused_call_never_shows_the_token() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut seen = Vec::new();
            let mut buf = [0u8; 1024];
            while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                seen.extend_from_slice(&buf[..n]);
            }
            let body = r#"{"error":"unauthorized","message":"no such credential"}"#;
            let answer = format!(
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(answer.as_bytes()).unwrap();
            String::from_utf8_lossy(&seen).into_owned()
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
}
