//! Private fixtures for `m2_tls_connect.rs`, all loopback-only.
//!
//! - [`TestPki`], [`within`], [`contains`] and [`provider`] come from `m2_common` and are
//!   re-exported here.
//! - [`TlsUpstream`]: a real TLS server on 127.0.0.1 that reports each handshake (SNI, ALPN,
//!   or the error) and then echoes lines back prefixed with `echo:`.
//! - [`Recorder`]: a plain TCP peer that records every byte until EOF, then answers a fixed
//!   reply and closes.
//! - [`ProxyRun`]: one loopback listener that hands exactly one accepted connection to the
//!   real `admit`.
//! - CONNECT, ClientHello and record builders, and [`tls_via_shim`], which drives a real
//!   rustls client while pipelining its first TLS bytes into the CONNECT head and
//!   re-fragmenting its ClientHello into several records and TCP writes.
//!
//! Every wait is bounded by `m2_common::DEADLINE` through [`within`]; every spawned task is
//! joined or aborted (the `Drop` impls abort what a failing test leaves behind).

#![allow(dead_code)]

pub use crate::m2_common::{TestPki, contains, provider, within};
use goalcup_cli::proxy::connect::{Admission, AdmissionConfig, PROXY_USERNAME, admit};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_rustls::{TlsAcceptor, TlsConnector};

/// The configured proxy secret: also the sentinel no diagnostic may contain.
pub const SECRET: &str = "5e17e1a15ec2e7c0ffee5e17e1a15ec2";

/// Decodes a committed hex fixture (whitespace ignored).
pub fn hex_fixture(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    assert!(digits.len().is_multiple_of(2), "odd hex fixture");
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

/// What the upstream server observed for one accepted connection.
#[derive(Debug, Clone)]
pub struct UpstreamSeen {
    pub sni: Option<String>,
    pub alpn: Option<Vec<u8>>,
    pub handshake: Result<(), String>,
}

/// A loopback TLS server; see the module documentation.
pub struct TlsUpstream {
    pub addr: SocketAddr,
    seen: mpsc::Receiver<UpstreamSeen>,
    task: Option<JoinHandle<()>>,
}

impl TlsUpstream {
    pub async fn start(
        chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
        alpn: &[&[u8]],
    ) -> Self {
        let mut config = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .unwrap();
        config.alpn_protocols = alpn.iter().map(|a| a.to_vec()).collect();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, seen) = mpsc::channel(16);
        let task = tokio::spawn(async move {
            // Dropped with this task, which aborts every connection task it owns.
            let mut connections = tokio::task::JoinSet::new();
            while let Ok((tcp, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let tx = tx.clone();
                connections.spawn(async move {
                    match acceptor.accept(tcp).await {
                        Err(e) => {
                            let _ = tx
                                .send(UpstreamSeen {
                                    sni: None,
                                    alpn: None,
                                    handshake: Err(e.to_string()),
                                })
                                .await;
                        }
                        Ok(tls) => {
                            let (_, conn) = tls.get_ref();
                            let seen = UpstreamSeen {
                                sni: conn.server_name().map(str::to_owned),
                                alpn: conn.alpn_protocol().map(<[u8]>::to_vec),
                                handshake: Ok(()),
                            };
                            let _ = tx.send(seen).await;
                            echo_lines(tls).await;
                        }
                    }
                });
            }
        });
        Self {
            addr,
            seen,
            task: Some(task),
        }
    }

    /// The next connection's handshake report.
    pub async fn next_seen(&mut self) -> UpstreamSeen {
        within("upstream handshake report", self.seen.recv())
            .await
            .expect("upstream server task ended")
    }

    /// Whether a handshake report is already waiting (none means no connection reached it).
    pub fn try_seen(&mut self) -> Option<UpstreamSeen> {
        self.seen.try_recv().ok()
    }

    /// Stops the server and every connection task; returns once they are gone.
    pub async fn shutdown(mut self) {
        let task = self.task.take().unwrap();
        task.abort();
        let joined = within("upstream shutdown", task).await;
        assert!(
            joined.as_ref().is_err_and(|e| e.is_cancelled()),
            "upstream task ended other than by cancellation: {joined:?}"
        );
    }
}

impl Drop for TlsUpstream {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn echo_lines<S: AsyncRead + AsyncWrite + Unpin>(stream: S) {
    let (read, mut write) = tokio::io::split(stream);
    let mut read = BufReader::new(read);
    let mut line = String::new();
    loop {
        line.clear();
        match read.read_line(&mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if write
                    .write_all(format!("echo:{line}").as_bytes())
                    .await
                    .is_err()
                    || write.flush().await.is_err()
                {
                    break;
                }
            }
        }
    }
}

/// A plain TCP byte recorder; see the module documentation.
pub struct Recorder {
    pub addr: SocketAddr,
    accepted: Arc<AtomicUsize>,
    received: watch::Receiver<usize>,
    task: Option<JoinHandle<Vec<u8>>>,
}

impl Recorder {
    /// Accepts one connection, records until EOF (at most 1 MiB), then writes `reply` and
    /// closes.
    pub async fn start(reply: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accepted = Arc::new(AtomicUsize::new(0));
        let (progress, received) = watch::channel(0);
        let counter = accepted.clone();
        let task = tokio::spawn(async move {
            let (mut tcp, _) = listener.accept().await.unwrap();
            counter.fetch_add(1, Ordering::SeqCst);
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                match tcp.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        bytes.extend_from_slice(&chunk[..n]);
                        assert!(bytes.len() <= 1 << 20, "recorder bound exceeded");
                        let _ = progress.send(bytes.len());
                    }
                }
            }
            let _ = tcp.write_all(&reply).await;
            let _ = tcp.shutdown().await;
            bytes
        });
        Self {
            addr,
            accepted,
            received,
            task: Some(task),
        }
    }

    /// How many connections reached this peer so far.
    pub fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }

    /// Waits until at least `n` bytes arrived.
    pub async fn wait_received(&mut self, n: usize) {
        within(
            "recorder progress",
            self.received.wait_for(|&received| received >= n),
        )
        .await
        .expect("recorder task ended");
    }

    /// Everything received, once the peer saw EOF.
    pub async fn bytes(mut self) -> Vec<u8> {
        within("recorder EOF", self.task.take().unwrap())
            .await
            .expect("recorder task panicked")
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// One loopback listener handing exactly one accepted connection to the real `admit`.
pub struct ProxyRun {
    pub addr: SocketAddr,
    task: Option<JoinHandle<Admission>>,
}

pub async fn serve_one(config: Arc<AdmissionConfig>) -> ProxyRun {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        admit(tcp, &config).await
    });
    ProxyRun {
        addr,
        task: Some(task),
    }
}

impl ProxyRun {
    /// The admission outcome.
    pub async fn outcome(mut self) -> Admission {
        within("admission outcome", self.task.take().unwrap())
            .await
            .expect("admission task panicked")
    }

    /// Cancels the admission task (as a caller's shutdown would) and waits until it is gone.
    pub async fn cancel(mut self) {
        let task = self.task.take().unwrap();
        task.abort();
        let joined = within("admission cancel", task).await;
        assert!(
            joined.as_ref().is_err_and(|e| e.is_cancelled()),
            "admission task ended other than by cancellation: {joined:?}"
        );
    }
}

impl Drop for ProxyRun {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// A `Proxy-Authorization` value, encoded with the `base64` crate (independent of the proxy).
pub fn basic(user: &str, secret: &str) -> String {
    use base64::Engine as _;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{secret}"))
    )
}

/// The correct credential for [`SECRET`].
pub fn good_auth() -> String {
    basic(PROXY_USERNAME, SECRET)
}

pub fn connect_head(target: &str, proxy_authorization: Option<&str>) -> Vec<u8> {
    let mut head = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nUser-Agent: m2-tls\r\n");
    if let Some(value) = proxy_authorization {
        head.push_str(&format!("Proxy-Authorization: {value}\r\n"));
    }
    head.push_str("\r\n");
    head.into_bytes()
}

/// Reads until the end of a response head; returns the head and any bytes after it. If the
/// peer closes (or fails) first, returns what arrived as the head and no leftover.
pub async fn read_head<S: AsyncRead + Unpin>(stream: &mut S) -> (Vec<u8>, Vec<u8>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let leftover = buf.split_off(end + 4);
            return (buf, leftover);
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return (buf, Vec::new()),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
}

/// Reads until EOF (or an error), bounded by `m2_common::DEADLINE`.
pub async fn read_to_end<S: AsyncRead + Unpin>(stream: &mut S) -> Vec<u8> {
    let mut all = Vec::new();
    let mut chunk = [0u8; 4096];
    within("read to EOF", async {
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => all.extend_from_slice(&chunk[..n]),
            }
        }
    })
    .await;
    all
}

/// Connects, sends an authenticated CONNECT for `target`, and returns the socket after a
/// `200` with nothing after it.
pub async fn open_tunnel(proxy: SocketAddr, target: &str) -> TcpStream {
    let mut tcp = TcpStream::connect(proxy).await.unwrap();
    tcp.set_nodelay(true).unwrap();
    tcp.write_all(&connect_head(target, Some(&good_auth())))
        .await
        .unwrap();
    let (head, leftover) = within("CONNECT response", read_head(&mut tcp)).await;
    assert!(
        head.starts_with(b"HTTP/1.1 200 "),
        "CONNECT refused: {}",
        String::from_utf8_lossy(&head)
    );
    assert!(leftover.is_empty(), "bytes after the 200 head");
    tcp
}

/// A rustls client configuration trusting exactly `roots`, offering `alpn`, with SNI on or
/// off.
pub fn client_config(
    roots: &[CertificateDer<'static>],
    alpn: &[&[u8]],
    sni: bool,
) -> Arc<ClientConfig> {
    let mut store = RootCertStore::empty();
    for root in roots {
        store.add(root.clone()).unwrap();
    }
    let mut config = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(store)
        .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|a| a.to_vec()).collect();
    config.enable_sni = sni;
    Arc::new(config)
}

pub fn server_name(name: &str) -> ServerName<'static> {
    ServerName::try_from(name.to_owned()).unwrap()
}

/// The ClientHello record(s) a real rustls client emits for `name`, without any socket.
pub fn client_hello(config: Arc<ClientConfig>, name: &str) -> Vec<u8> {
    let mut conn = rustls::ClientConnection::new(config, server_name(name)).unwrap();
    let mut out = Vec::new();
    while conn.wants_write() {
        conn.write_tls(&mut out).unwrap();
    }
    out
}

/// Re-frames TLS handshake records so each carries at most `payload` handshake bytes.
pub fn refragment(records: &[u8], payload: usize) -> Vec<u8> {
    let mut handshake = Vec::new();
    let mut version = [3, 1];
    let mut offset = 0;
    while offset < records.len() {
        assert_eq!(records[offset], 22, "not a handshake record");
        version = [records[offset + 1], records[offset + 2]];
        let len = u16::from_be_bytes([records[offset + 3], records[offset + 4]]) as usize;
        handshake.extend_from_slice(&records[offset + 5..offset + 5 + len]);
        offset += 5 + len;
    }
    let mut out = Vec::new();
    for fragment in handshake.chunks(payload) {
        out.push(22);
        out.extend_from_slice(&version);
        out.extend_from_slice(&(fragment.len() as u16).to_be_bytes());
        out.extend_from_slice(fragment);
    }
    out
}

/// Writes `bytes` in `piece`-sized writes with a short pause after each.
pub async fn write_in_pieces<S: AsyncWrite + Unpin>(stream: &mut S, bytes: &[u8], piece: usize) {
    for part in bytes.chunks(piece) {
        stream.write_all(part).await.unwrap();
        stream.flush().await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// How [`tls_via_shim`] sends the CONNECT head and the client's first TLS bytes.
pub struct ShimPlan {
    /// The complete CONNECT head.
    pub head: Vec<u8>,
    /// Offsets at which the head is split into separate writes.
    pub head_splits: Vec<usize>,
    /// How many leading ClientHello bytes ride in the same write as the head's last piece.
    pub glue: usize,
    /// Re-fragment the ClientHello into records of at most this many handshake bytes.
    pub record_payload: Option<usize>,
    /// The size of each later write of the remaining ClientHello bytes.
    pub write_piece: usize,
}

/// Runs a real rustls client whose transport is a duplex; a shim task between that duplex
/// and the proxy socket sends the CONNECT head in pieces, glues the first ClientHello bytes
/// onto its last piece, sends the rest (optionally re-fragmented), strips the proxy's `200`
/// head, then relays both ways. Returns the client's handshake result and the shim task.
pub async fn tls_via_shim(
    proxy: SocketAddr,
    plan: ShimPlan,
    config: Arc<ClientConfig>,
    name: &str,
) -> (
    std::io::Result<tokio_rustls::client::TlsStream<tokio::io::DuplexStream>>,
    JoinHandle<()>,
) {
    let (client_end, mut shim_end) = tokio::io::duplex(64 * 1024);
    let mut tcp = TcpStream::connect(proxy).await.unwrap();
    tcp.set_nodelay(true).unwrap();
    let shim = tokio::spawn(async move {
        let mut header = [0u8; 5];
        shim_end.read_exact(&mut header).await.unwrap();
        let len = u16::from_be_bytes([header[3], header[4]]) as usize;
        let mut hello = header.to_vec();
        hello.resize(5 + len, 0);
        shim_end.read_exact(&mut hello[5..]).await.unwrap();
        let hello = match plan.record_payload {
            Some(payload) => refragment(&hello, payload),
            None => hello,
        };
        let mut start = 0;
        for &split in &plan.head_splits {
            tcp.write_all(&plan.head[start..split]).await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
            start = split;
        }
        let glue = plan.glue.min(hello.len());
        let mut last = plan.head[start..].to_vec();
        last.extend_from_slice(&hello[..glue]);
        tcp.write_all(&last).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        write_in_pieces(&mut tcp, &hello[glue..], plan.write_piece).await;
        let (head, leftover) = read_head(&mut tcp).await;
        assert!(
            head.starts_with(b"HTTP/1.1 200 "),
            "CONNECT refused: {}",
            String::from_utf8_lossy(&head)
        );
        shim_end.write_all(&leftover).await.unwrap();
        let _ = tokio::io::copy_bidirectional(&mut shim_end, &mut tcp).await;
    });
    let tls = TlsConnector::from(config)
        .connect(server_name(name), client_end)
        .await;
    (tls, shim)
}

/// Writes one line over an established client TLS stream and reads the echoed line.
pub async fn echo_round_trip<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    line: &str,
) -> String {
    stream.write_all(line.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();
    let mut answer = Vec::new();
    let mut byte = [0u8; 1];
    within("echo line", async {
        loop {
            let n = stream.read(&mut byte).await.unwrap();
            assert!(n == 1, "EOF before the echoed line ended");
            answer.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }
    })
    .await;
    String::from_utf8(answer).unwrap()
}

/// Drives an intercepted pair: copy both ways until either side ends.
pub fn relay(pair: goalcup_cli::proxy::api::RelayPair) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut pair = pair;
        let _ = tokio::io::copy_bidirectional(&mut pair.client, &mut pair.upstream).await;
    })
}
