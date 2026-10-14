//! The one HTTP client for the upload server: one request, sent once, its answer read with a
//! bound. Retrying is the caller's.
//!
//! TLS is this crate's own rustls with the `ring` provider and the `webpki-roots` anchors, the
//! stack the recording proxy already links, plugged into `ureq` through its connector hook.
//! The workspace builds `ureq` without its TLS feature, so this adds no crate.

use super::api::ErrorBody;
use crate::server::ServerConfig;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

/// How long a connection may take to open.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a JSON call may take as a whole.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// The most of a response body read. Every answer in docs/UPLOAD-API.md is a few kilobytes.
const MAX_RESPONSE_BYTES: u64 = 256 * 1024;

/// Why a call did not return the answer it asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerError {
    /// No answer: the server could not be reached, or the connection broke. Safe to retry.
    Transport(String),
    /// An answer that is not a success, with the server's `error` code and sentence when it
    /// sent them, and its `Retry-After` in seconds.
    Status {
        status: u16,
        code: String,
        message: String,
        retry_after: Option<u64>,
    },
    /// A success whose body this client could not read.
    Malformed(String),
    /// The TLS handshake failed: the certificate is not trusted, not for this host, or the
    /// server speaks no TLS this client accepts. Asking again does not change a certificate.
    Tls(String),
}

impl ServerError {
    /// Whether asking again later can succeed: no answer, a server error, a rate limit, the
    /// maintenance switch, or a body that arrived damaged (`422 digest_mismatch`).
    pub fn is_retryable(&self) -> bool {
        match self {
            ServerError::Transport(_) => true,
            ServerError::Status { status, .. } => {
                *status >= 500 || *status == 429 || *status == 422
            }
            ServerError::Malformed(_) | ServerError::Tls(_) => false,
        }
    }

    /// The server's `Retry-After`, in seconds, if it sent one.
    pub fn retry_after(&self) -> Option<u64> {
        match self {
            ServerError::Status { retry_after, .. } => *retry_after,
            _ => None,
        }
    }

    /// Whether this is the server's `409 conflict`.
    pub fn is_conflict(&self) -> bool {
        matches!(self, ServerError::Status { status: 409, .. })
    }
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerError::Transport(why) => write!(f, "the server could not be reached ({why})"),
            ServerError::Status {
                status,
                code,
                message,
                ..
            } => {
                if message.is_empty() {
                    write!(f, "the server answered HTTP {status} {code}")
                } else {
                    write!(f, "the server answered HTTP {status} {code}: {message}")
                }
            }
            ServerError::Malformed(why) => write!(f, "the server's answer is unreadable ({why})"),
            ServerError::Tls(why) => write!(
                f,
                "the server's TLS connection was refused ({why}); check the address `goalcup \
                 login` stored"
            ),
        }
    }
}

/// A client for one server, holding the token. `Debug` prints the URL only.
#[derive(Clone)]
pub struct ServerClient {
    base: String,
    token: String,
    agent: ureq::Agent,
}

impl std::fmt::Debug for ServerClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerClient")
            .field("base", &self.base)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl ServerClient {
    /// A client for `config`, trusting the public web roots.
    pub fn new(config: &ServerConfig) -> ServerClient {
        let mut store = rustls::RootCertStore::empty();
        store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        ServerClient::with_store(config, store)
    }

    /// A client trusting exactly `roots`, for a test server with a certificate of its own. A
    /// shipped build has no such door.
    #[cfg(feature = "test-support")]
    pub fn with_roots(
        config: &ServerConfig,
        roots: impl IntoIterator<Item = rustls::pki_types::CertificateDer<'static>>,
    ) -> ServerClient {
        let mut store = rustls::RootCertStore::empty();
        for root in roots {
            let _ = store.add(root);
        }
        ServerClient::with_store(config, store)
    }

    fn with_store(config: &ServerConfig, store: rustls::RootCertStore) -> ServerClient {
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        // The ring provider supports both default protocol versions; this cannot fail, and
        // nothing in `src/` panics, so a failure falls back to an agent that refuses https.
        .map(|builder| Arc::new(builder.with_root_certificates(store).with_no_client_auth()));
        let mut agent = ureq::AgentBuilder::new()
            // One connection per call: calls are minutes apart, and a kept connection to a
            // server that has since restarted is one more way for a call to hang.
            .max_idle_connections(0)
            .max_idle_connections_per_host(0)
            // A redirect would carry the bearer token to an address nobody configured.
            .redirects(0)
            .timeout_connect(CONNECT_TIMEOUT)
            // Proxies are not consulted: the tracker never routes the token through one.
            .try_proxy_from_env(false);
        if let Ok(tls) = tls {
            agent = agent.tls_connector(Arc::new(RustlsConnector(tls)));
        }
        ServerClient {
            base: config.url.clone(),
            token: config.token.clone(),
            agent: agent.build(),
        }
    }

    /// The server's base URL, for messages.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// `method path` with a JSON body (or none), answered by JSON of type `T`.
    pub fn json<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<&impl serde::Serialize>,
    ) -> Result<T, ServerError> {
        self.json_timed(method, path, body, CALL_TIMEOUT)
    }

    /// [`ServerClient::json`] within `timeout` for the whole call.
    pub fn json_timed<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<&impl serde::Serialize>,
        timeout: Duration,
    ) -> Result<T, ServerError> {
        let request = self
            .request(method, path)
            .timeout(timeout)
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
        parse(answer(sent)?)
    }

    /// `PUT path` with `body` streamed from a reader of exactly `length` bytes, `headers`
    /// beside the token, answered by JSON of type `T`. `timeout` bounds the whole call.
    pub fn put_stream<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        headers: &[(&str, String)],
        length: u64,
        body: impl Read,
        timeout: Duration,
    ) -> Result<T, ServerError> {
        let mut request = self
            .request("PUT", path)
            .timeout(timeout)
            .set("Accept", "application/json")
            .set("Content-Length", &length.to_string());
        for (name, value) in headers {
            request = request.set(name, value);
        }
        parse(answer(request.send(body))?)
    }

    fn request(&self, method: &str, path: &str) -> ureq::Request {
        self.agent
            .request(method, &format!("{}{path}", self.base))
            .set("Authorization", &format!("Bearer {}", self.token))
    }
}

/// A success response, or the error the answer describes.
fn answer(sent: Result<ureq::Response, ureq::Error>) -> Result<ureq::Response, ServerError> {
    match sent {
        Ok(response) => Ok(response),
        Err(ureq::Error::Status(status, response)) => {
            let retry_after = response
                .header("Retry-After")
                .and_then(|value| value.trim().parse::<u64>().ok());
            let body = read_bounded(response).unwrap_or_default();
            let error: ErrorBody = serde_json::from_slice(&body).unwrap_or_default();
            Err(ServerError::Status {
                status,
                code: printable(&error.error, 40),
                message: printable(&error.message, 300),
                retry_after,
            })
        }
        // The transport error's text names the host and the cause, never a header.
        Err(ureq::Error::Transport(t)) => Err(match tls_failure(&t) {
            Some(why) => ServerError::Tls(why),
            None => ServerError::Transport(t.to_string()),
        }),
    }
}

/// The server's own words, safe for a terminal: control characters (an escape sequence among
/// them) dropped, and at most `max` characters.
fn printable(text: &str, max: usize) -> String {
    let mut kept: String = text.chars().filter(|c| !c.is_control()).take(max).collect();
    if text.chars().filter(|c| !c.is_control()).count() > max {
        kept.push('…');
    }
    kept
}

/// The rustls error under a transport failure, if the failure was the TLS handshake.
fn tls_failure(t: &ureq::Transport) -> Option<String> {
    let mut source = std::error::Error::source(t);
    while let Some(error) = source {
        if let Some(tls) = error.downcast_ref::<rustls::Error>() {
            return Some(tls.to_string());
        }
        if let Some(tls) = error
            .downcast_ref::<std::io::Error>()
            .and_then(|io| io.get_ref())
            .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        {
            return Some(tls.to_string());
        }
        source = error.source();
    }
    None
}

fn parse<T: serde::de::DeserializeOwned>(response: ureq::Response) -> Result<T, ServerError> {
    let body = read_bounded(response)?;
    serde_json::from_slice(&body).map_err(|e| ServerError::Malformed(e.to_string()))
}

fn read_bounded(response: ureq::Response) -> Result<Vec<u8>, ServerError> {
    let mut body = Vec::new();
    response
        .into_reader()
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|e| ServerError::Transport(format!("reading the answer: {e}")))?;
    if body.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(ServerError::Malformed(format!(
            "an answer larger than {MAX_RESPONSE_BYTES} bytes"
        )));
    }
    Ok(body)
}

/// TLS for `ureq`: rustls with the ring provider, validating against the configured roots and
/// the server's host name.
struct RustlsConnector(Arc<rustls::ClientConfig>);

impl ureq::TlsConnector for RustlsConnector {
    fn connect(
        &self,
        dns_name: &str,
        mut io: Box<dyn ureq::ReadWrite>,
    ) -> Result<Box<dyn ureq::ReadWrite>, ureq::Error> {
        let host = dns_name.trim_start_matches('[').trim_end_matches(']');
        let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{host:?} is not a TLS server name: {e}"),
            )
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
