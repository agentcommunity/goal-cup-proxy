//! Fixtures shared by the proxy-path tests (`m2_ipc_compose`, and `m2_proxy_integration` and
//! `m2_tls_connect` through their support modules), all loopback-only and independent of the
//! code under test.
//!
//! - [`TestPki`]: an rcgen CA independent of the recording proxy's own CA code, for synthetic
//!   upstream servers.
//! - A rustls client configuration offering `http/1.1`, and byte helpers over any stream.
//! - RFC 6455 upgrade bytes, written here independently of the relay.
//!
//! Every wait is bounded by [`DEADLINE`] through [`within`].

#![allow(dead_code)]

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ClientConfig, RootCertStore};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

/// The bound on every fixture wait.
pub const DEADLINE: Duration = Duration::from_secs(10);

pub const HTTP11: &[u8] = b"http/1.1";

/// Awaits `fut` or panics naming `what` once [`DEADLINE`] passes.
pub async fn within<F: Future>(what: &str, fut: F) -> F::Output {
    tokio::time::timeout(DEADLINE, fut)
        .await
        .unwrap_or_else(|_| panic!("fixture deadline passed: {what}"))
}

/// Polls `condition` every 5 ms until it holds, bounded by [`DEADLINE`].
pub async fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
    within(what, async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
}

/// Whether `needle` occurs in `haystack`.
pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

// ---------------------------------------------------------------------------------------
// PKI and the client.
// ---------------------------------------------------------------------------------------

pub fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// A test-only certificate authority, independent of `proxy::tls`.
pub struct TestPki {
    params: CertificateParams,
    key: KeyPair,
    root: CertificateDer<'static>,
    root_pem: String,
}

impl TestPki {
    pub fn new(common_name: &str) -> Self {
        let mut params = CertificateParams::default();
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, common_name);
        params.distinguished_name = name;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        let key = KeyPair::generate().unwrap();
        let certificate = params.self_signed(&key).unwrap();
        let (root, root_pem) = (certificate.der().clone(), certificate.pem());
        Self {
            params,
            key,
            root,
            root_pem,
        }
    }

    pub fn root(&self) -> CertificateDer<'static> {
        self.root.clone()
    }

    /// The root as a PEM file's text.
    pub fn root_pem(&self) -> String {
        self.root_pem.clone()
    }

    /// A server leaf for `names`, signed by this authority.
    pub fn leaf(&self, names: &[&str]) -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
        let mut params =
            CertificateParams::new(names.iter().map(|n| n.to_string()).collect::<Vec<_>>())
                .unwrap();
        params.use_authority_key_identifier_extension = true;
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let key = KeyPair::generate().unwrap();
        let issuer = Issuer::from_params(&self.params, &self.key);
        let cert = params.signed_by(&key, &issuer).unwrap();
        (
            vec![cert.der().clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
        )
    }
}

pub type UpstreamTls = tokio_rustls::server::TlsStream<TcpStream>;
pub type ClientTls = tokio_rustls::client::TlsStream<TcpStream>;

/// A rustls client configuration trusting exactly `roots`, offering `http/1.1`.
pub fn client_config(roots: &[CertificateDer<'static>]) -> Arc<ClientConfig> {
    let mut store = RootCertStore::empty();
    for root in roots {
        store.add(root.clone()).unwrap();
    }
    let mut config = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(store)
        .with_no_client_auth();
    config.alpn_protocols = vec![HTTP11.to_vec()];
    Arc::new(config)
}

// ---------------------------------------------------------------------------------------
// Byte helpers.
// ---------------------------------------------------------------------------------------

/// Reads until the end of a head; returns the head (with its blank line) and what followed.
/// A peer that closes first yields what arrived and no leftover.
pub async fn read_head<S: AsyncRead + Unpin>(stream: &mut S) -> (Vec<u8>, Vec<u8>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(end + 4);
            return (buf, rest);
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return (buf, Vec::new()),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
}

/// Reads into `buf` until it holds at least `n` bytes.
pub async fn read_at_least<S: AsyncRead + Unpin>(stream: &mut S, buf: &mut Vec<u8>, n: usize) {
    let mut chunk = vec![0u8; 16 * 1024];
    within("read_at_least", async {
        while buf.len() < n {
            let read = stream.read(&mut chunk).await.expect("read failed");
            assert!(read > 0, "EOF after {} of {n} bytes", buf.len());
            buf.extend_from_slice(&chunk[..read]);
        }
    })
    .await;
}

/// Reads until EOF or an error; returns what arrived and whether it ended cleanly.
pub async fn read_to_end<S: AsyncRead + Unpin>(stream: &mut S) -> (Vec<u8>, bool) {
    let mut all = Vec::new();
    let mut chunk = vec![0u8; 16 * 1024];
    let clean = within("read to EOF", async {
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) => break true,
                Err(_) => break false,
                Ok(n) => all.extend_from_slice(&chunk[..n]),
            }
        }
    })
    .await;
    (all, clean)
}

/// Writes all of `bytes` and flushes.
pub async fn send<S: AsyncWrite + Unpin>(stream: &mut S, bytes: &[u8]) {
    stream.write_all(bytes).await.unwrap();
    stream.flush().await.unwrap();
}

/// Ends a relayed connection from both ends cleanly: the client shuts down, the upstream
/// sees EOF and shuts down, the client sees EOF.
pub async fn close_both(mut client: ClientTls, mut upstream: UpstreamTls) {
    client.shutdown().await.unwrap();
    let (rest, clean) = read_to_end(&mut upstream).await;
    assert!(
        rest.is_empty() && clean,
        "upstream end: {rest:?} clean={clean}"
    );
    upstream.shutdown().await.unwrap();
    let (rest, clean) = read_to_end(&mut client).await;
    assert!(
        rest.is_empty() && clean,
        "client end: {rest:?} clean={clean}"
    );
}

// ---------------------------------------------------------------------------------------
// WebSocket bytes (RFC 6455), written here independently of the relay.
// ---------------------------------------------------------------------------------------

pub const WS_KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";

pub fn ws_upgrade_request(path: &str) -> Vec<u8> {
    format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {WS_KEY}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    )
    .into_bytes()
}

/// RFC 6455 section 4.2.2, written out here so the test oracle is not the recording proxy's own
/// function.
fn derive_accept_key(key: &[u8]) -> String {
    use base64::Engine as _;
    use sha1::{Digest, Sha1};
    let mut hash = Sha1::new();
    hash.update(key);
    hash.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64::engine::general_purpose::STANDARD.encode(hash.finalize())
}

pub fn ws_101() -> Vec<u8> {
    let accept = derive_accept_key(WS_KEY.as_bytes());
    format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    )
    .into_bytes()
}
