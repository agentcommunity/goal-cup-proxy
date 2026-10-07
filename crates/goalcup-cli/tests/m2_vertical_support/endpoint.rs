//! The controlled TLS upstream: one loopback listener serving an API
//! host (HTTP echo, SSE stream, WebSocket echo/stream, an unknown binary protocol, bulk
//! transfer) and an authentication host (always an opaque tunnel at the proxy).
//!
//! Both names resolve to loopback (`*.localhost`), so the real proxy reaches this endpoint by
//! its ordinary CONNECT-target resolution: no library change and no resolver override. The
//! endpoint has its own CA; the proxy is told to trust it as its *upstream* roots, exactly as
//! production trusts the public roots.
//!
//! The endpoint counts decoded payload bytes and messages on its own, independently of the
//! recorder. Those counters are bounded observations about traffic, never captured evidence.
//! Startup and shutdown are bounded: [`ControlledEndpoint::start`] returns once the listener
//! is bound and serving, and [`ControlledEndpoint::shutdown`] stops accepting, asks every
//! connection to finish at its next boundary and aborts whatever is left at the deadline.

use super::wire::{
    self, Conn, Det, Frame, Head, OP_BINARY, OP_CLOSE, OP_CONTINUATION, OP_PING, OP_PONG, OP_TEXT,
    encode_frame,
};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::Serialize;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinSet;

/// The captured API host of the controlled endpoint.
pub const API_HOST: &str = "api.e00.localhost";
/// The authentication host: always tunnelled, never intercepted.
pub const AUTH_HOST: &str = "auth.e00.localhost";

/// Largest request body the echo route holds (it echoes with a Content-Length).
const MAX_ECHO_BYTES: u64 = 4 * 1024 * 1024;
/// Largest body any route accepts.
const MAX_BODY_BYTES: u64 = 256 * 1024 * 1024;
/// Largest WebSocket frame payload or reassembled message.
const MAX_WS_MESSAGE: usize = 8 * 1024 * 1024;
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// The endpoint's independent observations.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EndpointCounters {
    pub connections: u64,
    pub tls_failures: u64,
    pub requests: u64,
    pub bad_requests: u64,
    pub host_mismatches: u64,
    pub test_id_mismatches: u64,
    pub interrupted_requests: u64,
    pub renewals_requested: u64,
    pub echo_requests: u64,
    pub echo_bytes_in: u64,
    pub echo_bytes_out: u64,
    pub sse_requests: u64,
    pub sse_events: u64,
    pub sse_bytes_out: u64,
    pub unknown_requests: u64,
    pub unknown_bytes_in: u64,
    pub unknown_bytes_out: u64,
    pub bulk_requests: u64,
    pub bulk_bytes_in: u64,
    pub bulk_bytes_out: u64,
    pub auth_requests: u64,
    pub auth_bytes_in: u64,
    pub auth_bytes_out: u64,
    pub ws_upgrades: u64,
    pub ws_messages_in: u64,
    pub ws_bytes_in: u64,
    pub ws_messages_out: u64,
    pub ws_bytes_out: u64,
    pub ws_clean_closes: u64,
    pub ws_interrupted: u64,
}

macro_rules! counters {
    ($($field:ident),* $(,)?) => {
        #[derive(Default)]
        struct Counters { $($field: AtomicU64,)* }
        impl Counters {
            fn snapshot(&self) -> EndpointCounters {
                EndpointCounters { $($field: self.$field.load(Ordering::SeqCst),)* }
            }
        }
    };
}

counters!(
    connections,
    tls_failures,
    requests,
    bad_requests,
    host_mismatches,
    test_id_mismatches,
    interrupted_requests,
    renewals_requested,
    echo_requests,
    echo_bytes_in,
    echo_bytes_out,
    sse_requests,
    sse_events,
    sse_bytes_out,
    unknown_requests,
    unknown_bytes_in,
    unknown_bytes_out,
    bulk_requests,
    bulk_bytes_in,
    bulk_bytes_out,
    auth_requests,
    auth_bytes_in,
    auth_bytes_out,
    ws_upgrades,
    ws_messages_in,
    ws_bytes_in,
    ws_messages_out,
    ws_bytes_out,
    ws_clean_closes,
    ws_interrupted,
);

fn add(counter: &AtomicU64, n: u64) {
    counter.fetch_add(n, Ordering::SeqCst);
}

/// The endpoint's own certificate authority and one leaf for both hosts.
pub struct EndpointPki {
    pub ca_der: CertificateDer<'static>,
    pub ca_pem: String,
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
}

impl EndpointPki {
    pub fn generate(hosts: &[&str]) -> Result<Self, rcgen::Error> {
        let mut params = CertificateParams::default();
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, "Goal Cup E00 controlled endpoint CA");
        params.distinguished_name = name;
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let ca_key = KeyPair::generate()?;
        let ca = params.self_signed(&ca_key)?;
        let mut leaf =
            CertificateParams::new(hosts.iter().map(|h| h.to_string()).collect::<Vec<_>>())?;
        leaf.use_authority_key_identifier_extension = true;
        leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let leaf_key = KeyPair::generate()?;
        let issuer = Issuer::from_params(&params, &ca_key);
        let leaf_cert = leaf.signed_by(&leaf_key, &issuer)?;
        Ok(Self {
            ca_der: ca.der().clone(),
            ca_pem: ca.pem(),
            chain: vec![leaf_cert.der().clone()],
            key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
        })
    }
}

/// The configuration the peers share: host names and the test ID both sides derive content
/// from.
#[derive(Debug, Clone)]
pub struct EndpointConfig {
    pub api_host: String,
    pub auth_host: String,
    pub test_id: String,
}

/// How shutdown went.
#[derive(Debug, Clone, Serialize)]
pub struct EndpointShutdown {
    pub within_deadline: bool,
    pub aborted_connections: u64,
}

pub struct ControlledEndpoint {
    pub addr: SocketAddr,
    pub ca_der: CertificateDer<'static>,
    pub ca_pem: String,
    counters: Arc<Counters>,
    stop: watch::Sender<bool>,
    accept: tokio::task::JoinHandle<JoinSet<()>>,
}

impl ControlledEndpoint {
    /// Bind `127.0.0.1:0` and start serving. Returns once the listener is accepting.
    pub async fn start(config: EndpointConfig) -> io::Result<Self> {
        let pki = EndpointPki::generate(&[&config.api_host, &config.auth_host])
            .map_err(|_| io::Error::other("endpoint certificate generation failed"))?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut tls = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|_| io::Error::other("endpoint TLS configuration failed"))?
            .with_no_client_auth()
            .with_single_cert(pki.chain.clone(), pki.key.clone_key())
            .map_err(|_| io::Error::other("endpoint TLS configuration failed"))?;
        tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let counters = Arc::new(Counters::default());
        let (stop, stop_rx) = watch::channel(false);
        let config = Arc::new(config);
        let accept = {
            let counters = counters.clone();
            let mut stop_rx = stop_rx.clone();
            tokio::spawn(async move {
                let mut connections = JoinSet::new();
                loop {
                    tokio::select! {
                        _ = stop_rx.changed() => break,
                        accepted = listener.accept() => {
                            let Ok((tcp, _)) = accepted else { continue };
                            add(&counters.connections, 1);
                            let acceptor = acceptor.clone();
                            let counters = counters.clone();
                            let config = config.clone();
                            let stop_rx = stop_rx.clone();
                            connections.spawn(async move {
                                let tls = tokio::time::timeout(
                                    TLS_HANDSHAKE_TIMEOUT,
                                    acceptor.accept(tcp),
                                )
                                .await;
                                match tls {
                                    Ok(Ok(stream)) => {
                                        serve(Conn::new(stream), &config, &counters, stop_rx).await;
                                    }
                                    _ => add(&counters.tls_failures, 1),
                                }
                            });
                        }
                    }
                    // Reap finished connections so the set stays bounded by live ones.
                    while connections.try_join_next().is_some() {}
                }
                connections
            })
        };
        Ok(Self {
            addr,
            ca_der: pki.ca_der,
            ca_pem: pki.ca_pem,
            counters,
            stop,
            accept,
        })
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn counters(&self) -> EndpointCounters {
        self.counters.snapshot()
    }

    /// Stop accepting, let connections finish at their next boundary, abort the rest at the
    /// deadline.
    pub async fn shutdown(self, deadline: Duration) -> EndpointShutdown {
        let until = tokio::time::Instant::now() + deadline;
        let _ = self.stop.send(true);
        let mut connections = match tokio::time::timeout_at(until, self.accept).await {
            Ok(Ok(set)) => set,
            _ => {
                return EndpointShutdown {
                    within_deadline: false,
                    aborted_connections: 0,
                };
            }
        };
        let drained = tokio::time::timeout_at(until, async {
            while connections.join_next().await.is_some() {}
        })
        .await
        .is_ok();
        let aborted = connections.len() as u64;
        connections.abort_all();
        while connections.join_next().await.is_some() {}
        EndpointShutdown {
            within_deadline: drained,
            aborted_connections: aborted,
        }
    }
}

/// The deterministic SSE event `i` of a stream: both peers compute it from the test ID.
pub fn sse_event(test_id: &str, i: u64, size: usize) -> Vec<u8> {
    let pad = Det::new(test_id, 1_000 + i).text(size);
    format!("data: {{\"i\":{i},\"pad\":\"{pad}\"}}\n\n").into_bytes()
}

/// The deterministic WebSocket stream message `i`.
pub fn ws_stream_message(test_id: &str, i: u64, size: usize) -> Vec<u8> {
    Det::new(test_id, 2_000 + i).text(size).into_bytes()
}

/// The deterministic unknown-protocol response body.
pub fn unknown_body(test_id: &str, size: usize) -> Vec<u8> {
    Det::new(test_id, 3_000).high_bytes(size)
}

/// The deterministic bulk response body.
pub fn bulk_piece(test_id: &str, piece: u64, size: usize) -> Vec<u8> {
    Det::new(test_id, 4_000 + piece).text(size).into_bytes()
}

fn query(target: &str) -> (&str, Vec<(&str, &str)>) {
    let (path, q) = target.split_once('?').unwrap_or((target, ""));
    let pairs = q
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| p.split_once('=').unwrap_or((p, "")))
        .collect();
    (path, pairs)
}

fn param(pairs: &[(&str, &str)], key: &str, default: u64) -> u64 {
    pairs
        .iter()
        .find(|(k, _)| *k == key)
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(default)
}

fn host_name(value: &str) -> &str {
    value.split(':').next().unwrap_or("")
}

enum Next {
    KeepAlive,
    Close,
}

async fn serve<S: AsyncRead + AsyncWrite + Unpin>(
    mut conn: Conn<S>,
    config: &EndpointConfig,
    counters: &Counters,
    mut stop: watch::Receiver<bool>,
) {
    loop {
        if *stop.borrow() {
            return;
        }
        let head = tokio::select! {
            _ = stop.changed() => return,
            head = conn.read_head() => head,
        };
        let head = match head {
            Ok(Some(head)) => head,
            Ok(None) => {
                // The client finished: answer its close with ours (TLS close_notify).
                let _ = tokio::io::AsyncWriteExt::shutdown(&mut conn.io).await;
                return;
            }
            Err(e) => {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    add(&counters.interrupted_requests, 1);
                } else {
                    add(&counters.bad_requests, 1);
                }
                return;
            }
        };
        add(&counters.requests, 1);
        match handle(&mut conn, &head, config, counters, &mut stop).await {
            Ok(Next::KeepAlive) => {}
            Ok(Next::Close) => {
                let _ = tokio::io::AsyncWriteExt::shutdown(&mut conn.io).await;
                return;
            }
            Err(e) => {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    add(&counters.interrupted_requests, 1);
                } else {
                    add(&counters.bad_requests, 1);
                }
                return;
            }
        }
    }
}

async fn read_request_body<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Conn<S>,
    head: &Head,
    limit: u64,
    mut sink: impl FnMut(&[u8]),
) -> io::Result<u64> {
    let mut total = 0u64;
    let mut count = |piece: &[u8]| {
        total += piece.len() as u64;
        sink(piece);
    };
    if head.is_chunked() {
        conn.read_chunked(&mut count).await?;
    } else if let Some(len) = head.content_length()? {
        if len > limit {
            return Err(wire::invalid("body too large"));
        }
        conn.read_body(len, &mut count).await?;
    }
    if total > limit {
        return Err(wire::invalid("body too large"));
    }
    Ok(total)
}

fn response_head(status: &str, content_type: &str, length: Option<u64>, close: bool) -> Vec<u8> {
    let mut head = format!("HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\n");
    match length {
        Some(n) => head.push_str(&format!("content-length: {n}\r\n")),
        None => head.push_str("transfer-encoding: chunked\r\n"),
    }
    if close {
        head.push_str("connection: close\r\n");
    }
    head.push_str("\r\n");
    head.into_bytes()
}

async fn handle<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Conn<S>,
    head: &Head,
    config: &EndpointConfig,
    counters: &Counters,
    stop: &mut watch::Receiver<bool>,
) -> io::Result<Next> {
    let mut parts = head.start.split(' ');
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let host = host_name(head.header("host").unwrap_or(""));
    if host != config.api_host && host != config.auth_host {
        add(&counters.host_mismatches, 1);
        return Ok(Next::Close);
    }
    if host == config.api_host && head.header("x-e00-test-id") != Some(config.test_id.as_str()) {
        add(&counters.test_id_mismatches, 1);
    }
    let close = head.has_token("connection", "close");
    if close {
        add(&counters.renewals_requested, 1);
    }
    let next = if close { Next::Close } else { Next::KeepAlive };
    let (path, pairs) = query(target);
    if host == config.auth_host {
        add(&counters.auth_requests, 1);
        let n = read_request_body(conn, head, MAX_ECHO_BYTES, |_| {}).await?;
        add(&counters.auth_bytes_in, n);
        let body = format!(
            "{{\"grant\":\"e00-synthetic\",\"id\":\"{}\"}}",
            Det::new(&config.test_id, 5_000).text(24)
        );
        conn.write_all(&response_head(
            "200 OK",
            "application/json",
            Some(body.len() as u64),
            close,
        ))
        .await?;
        conn.write_all(body.as_bytes()).await?;
        add(&counters.auth_bytes_out, body.len() as u64);
        return Ok(next);
    }
    match (method, path) {
        ("POST", "/v1/echo") => {
            add(&counters.echo_requests, 1);
            let mut body = Vec::new();
            read_request_body(conn, head, MAX_ECHO_BYTES, |p| body.extend_from_slice(p)).await?;
            add(&counters.echo_bytes_in, body.len() as u64);
            let content_type = head
                .header("content-type")
                .unwrap_or("application/octet-stream");
            conn.write_all(&response_head(
                "200 OK",
                content_type,
                Some(body.len() as u64),
                close,
            ))
            .await?;
            conn.write_all(&body).await?;
            add(&counters.echo_bytes_out, body.len() as u64);
            Ok(next)
        }
        ("POST", "/v1/sse") => {
            add(&counters.sse_requests, 1);
            let mut body = Vec::new();
            read_request_body(conn, head, MAX_ECHO_BYTES, |p| body.extend_from_slice(p)).await?;
            let events = param(&pairs, "events", 4);
            let size = param(&pairs, "size", 64) as usize;
            let interval = Duration::from_millis(param(&pairs, "interval_ms", 20));
            conn.write_all(&response_head("200 OK", "text/event-stream", None, close))
                .await?;
            for i in 0..events {
                if *stop.borrow() {
                    return Ok(Next::Close);
                }
                let event = sse_event(&config.test_id, i, size);
                conn.write_all(&wire::chunk(&event)).await?;
                add(&counters.sse_events, 1);
                add(&counters.sse_bytes_out, event.len() as u64);
                tokio::time::sleep(interval).await;
            }
            // The request body comes back as one last event, split across two chunks at the
            // requested byte so the recorder sees it in two reads.
            if let Some(split) = head
                .header("x-e00-split-at")
                .and_then(|v| v.parse::<usize>().ok())
            {
                let mut event = b"data: ".to_vec();
                event.extend_from_slice(&body);
                event.extend_from_slice(b"\n\n");
                let split = (split + 6).min(event.len());
                conn.write_all(&wire::chunk(&event[..split])).await?;
                tokio::time::sleep(Duration::from_millis(60)).await;
                conn.write_all(&wire::chunk(&event[split..])).await?;
                add(&counters.sse_events, 1);
                add(&counters.sse_bytes_out, event.len() as u64);
            }
            conn.write_all(&wire::chunk(&[])).await?;
            Ok(next)
        }
        ("POST", "/v1/unknown") => {
            add(&counters.unknown_requests, 1);
            let n = read_request_body(conn, head, MAX_BODY_BYTES, |_| {}).await?;
            add(&counters.unknown_bytes_in, n);
            let body = unknown_body(&config.test_id, param(&pairs, "size", 1024) as usize);
            conn.write_all(&response_head(
                "200 OK",
                "application/x-e00-unknown",
                Some(body.len() as u64),
                close,
            ))
            .await?;
            conn.write_all(&body).await?;
            add(&counters.unknown_bytes_out, body.len() as u64);
            Ok(next)
        }
        ("POST", "/v1/bulk") => {
            add(&counters.bulk_requests, 1);
            let n = read_request_body(conn, head, MAX_BODY_BYTES, |_| {}).await?;
            add(&counters.bulk_bytes_in, n);
            let size = param(&pairs, "size", 0);
            conn.write_all(&response_head("200 OK", "text/plain", Some(size), close))
                .await?;
            let mut sent = 0u64;
            let mut piece = 0u64;
            while sent < size {
                let n = (size - sent).min(16 * 1024) as usize;
                let bytes = bulk_piece(&config.test_id, piece, n);
                conn.write_all(&bytes).await?;
                sent += n as u64;
                piece += 1;
            }
            add(&counters.bulk_bytes_out, sent);
            Ok(next)
        }
        ("GET", "/v1/ws") if head.has_token("upgrade", "websocket") => {
            let Some(key) = head.header("sec-websocket-key") else {
                add(&counters.bad_requests, 1);
                return Ok(Next::Close);
            };
            let accept = derive_accept_key(key.as_bytes());
            let response = format!(
                "HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-accept: {accept}\r\n\r\n"
            );
            conn.write_all(response.as_bytes()).await?;
            add(&counters.ws_upgrades, 1);
            let fragment = param(&pairs, "frag", 1024) as usize;
            websocket(conn, config, counters, fragment, stop).await?;
            Ok(Next::Close)
        }
        _ => {
            read_request_body(conn, head, MAX_ECHO_BYTES, |_| {}).await?;
            conn.write_all(&response_head(
                "404 Not Found",
                "text/plain",
                Some(0),
                close,
            ))
            .await?;
            Ok(next)
        }
    }
}

/// Send one server message fragmented into `fragment`-byte frames, with a ping between the
/// first two fragments so a control frame interleaves the message.
async fn send_message<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Conn<S>,
    opcode: u8,
    payload: &[u8],
    fragment: usize,
) -> io::Result<()> {
    let frames = wire::fragment_message(opcode, payload, fragment, || None);
    for (i, frame) in frames.iter().enumerate() {
        conn.write_all(frame).await?;
        if i == 0 && frames.len() > 1 {
            conn.write_all(&encode_frame(true, OP_PING, b"e00", None))
                .await?;
        }
    }
    Ok(())
}

async fn websocket<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Conn<S>,
    config: &EndpointConfig,
    counters: &Counters,
    fragment: usize,
    stop: &mut watch::Receiver<bool>,
) -> io::Result<()> {
    let mut message: Option<(u8, Vec<u8>)> = None;
    loop {
        let frame = tokio::select! {
            _ = stop.changed() => return Ok(()),
            frame = conn.read_frame(MAX_WS_MESSAGE) => frame,
        };
        let frame: Frame = match frame {
            Ok(Some(frame)) => frame,
            Ok(None) | Err(_) => {
                add(&counters.ws_interrupted, 1);
                return Ok(());
            }
        };
        match frame.opcode {
            OP_PING => {
                conn.write_all(&encode_frame(true, OP_PONG, &frame.payload, None))
                    .await?;
            }
            OP_PONG => {}
            OP_CLOSE => {
                conn.write_all(&encode_frame(true, OP_CLOSE, &frame.payload, None))
                    .await?;
                add(&counters.ws_clean_closes, 1);
                return Ok(());
            }
            OP_TEXT | OP_BINARY | OP_CONTINUATION => {
                let (opcode, mut data) = match (frame.opcode, message.take()) {
                    (OP_CONTINUATION, Some(open)) => open,
                    (OP_CONTINUATION, None) => return Err(wire::invalid("stray continuation")),
                    (op, None) => (op, Vec::new()),
                    (_, Some(_)) => return Err(wire::invalid("interleaved data message")),
                };
                data.extend_from_slice(&frame.payload);
                if data.len() > MAX_WS_MESSAGE {
                    return Err(wire::invalid("message too large"));
                }
                if !frame.fin {
                    message = Some((opcode, data));
                    continue;
                }
                add(&counters.ws_messages_in, 1);
                add(&counters.ws_bytes_in, data.len() as u64);
                let text = std::str::from_utf8(&data).unwrap_or("");
                if opcode == OP_TEXT && text.starts_with("stream ") {
                    let mut args = text["stream ".len()..].split(' ');
                    let count: u64 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1);
                    let size: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(64);
                    for i in 0..count {
                        let payload = ws_stream_message(&config.test_id, i, size);
                        send_message(conn, OP_TEXT, &payload, fragment).await?;
                        add(&counters.ws_messages_out, 1);
                        add(&counters.ws_bytes_out, payload.len() as u64);
                    }
                } else {
                    send_message(conn, opcode, &data, fragment).await?;
                    add(&counters.ws_messages_out, 1);
                    add(&counters.ws_bytes_out, data.len() as u64);
                }
            }
            _ => return Err(wire::invalid("unknown opcode")),
        }
    }
}

/// RFC 6455 section 4.2.2: the controlled endpoint answers a handshake without the proxy's code.
fn derive_accept_key(key: &[u8]) -> String {
    use base64::Engine as _;
    use sha1::{Digest, Sha1};
    let mut hash = Sha1::new();
    hash.update(key);
    hash.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64::engine::general_purpose::STANDARD.encode(hash.finalize())
}
