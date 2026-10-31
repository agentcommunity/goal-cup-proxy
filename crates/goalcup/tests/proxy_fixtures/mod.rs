//! Shared support for the `proxy_*` tests that drive the real `proxy::serve`. A test binary
//! includes it as `mod proxy_fixtures;` after `mod common;`, so unused items are allowed here.

#![allow(dead_code)]

use crate::common::recorder::{ends, opens};
use crate::common::{HANG, within_async};
use goalcup::connector::HostList;
use goalcup::pipeline::{Calls, Recorder, StreamKind};
use goalcup::proxy::ca::RunCa;
use goalcup::proxy::{ProxyCtx, ProxyHandle, serve};
use rustls::pki_types::CertificateDer;
use rustls::{ClientConfig, RootCertStore};
use serde_json::Value;
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

/// `api.openai.com` as a listed provider.
pub const HOSTS: &str = "provider = [\"api.openai.com\"]\n";

/// The real `serve` on a clone of `listener` with a test recorder.
pub fn serve_on(listener: &TcpListener, hosts: &str, ca: RunCa) -> (ProxyHandle, Calls) {
    let (recorder, calls) = Recorder::for_tests(1 << 30);
    let hosts = HostList::from_file(hosts.as_bytes(), "hosts.toml").unwrap();
    let ctx = ProxyCtx {
        recorder,
        hosts,
        ca,
    };
    (serve(listener.try_clone().unwrap(), ctx), calls)
}

/// The real `serve` on a held port with a 4,096 backlog, as the watchdog sets it, and a fresh
/// run CA; `env` holds the test hooks, read once in `serve`.
pub fn proxy(hosts: &str, env: &[(&str, String)]) -> (ProxyHandle, SocketAddr, Calls, RunCa) {
    for (k, v) in env {
        // SAFETY: nextest runs each test in its own process; this runs before `serve` reads them.
        unsafe { std::env::set_var(k, v) };
    }
    let (listener, addr) = crate::common::ports::held();
    socket2::SockRef::from(&listener).listen(4096).unwrap();
    let ca = RunCa::mint(&["openai.com".to_string()]).unwrap();
    let (h, calls) = serve_on(&listener, hosts, ca.clone());
    (h, addr, calls, ca)
}

/// Reads an answer's head and gives its status.
pub async fn status(s: &mut TcpStream) -> u16 {
    let mut got = Vec::new();
    while !got.ends_with(b"\r\n\r\n") {
        let mut b = [0u8; 1];
        let n = s.read(&mut b).await.unwrap();
        assert_eq!(n, 1, "the proxy closed mid-answer");
        got.push(b[0]);
    }
    String::from_utf8_lossy(&got[9..12]).parse().unwrap()
}

/// Sends `head` and reads the answer's head; gives the stream and the status.
pub async fn ask(proxy: SocketAddr, head: &str) -> (TcpStream, u16) {
    let mut s = TcpStream::connect(proxy).await.unwrap();
    s.write_all(head.as_bytes()).await.unwrap();
    let code = status(&mut s).await;
    (s, code)
}

/// A CONNECT to `target`; gives the stream and the status.
pub async fn connect(proxy: SocketAddr, target: &str) -> (TcpStream, u16) {
    let head = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n");
    ask(proxy, &head).await
}

/// A CONNECT to `target` answered `200`.
pub async fn through(proxy: SocketAddr, target: &str) -> TcpStream {
    let (s, code) = connect(proxy, target).await;
    assert_eq!(code, 200);
    s
}

/// A client config trusting `roots` and offering `alpn`.
pub fn client_trusting(roots: Vec<CertificateDer<'static>>, alpn: &[&[u8]]) -> Arc<ClientConfig> {
    let mut store = RootCertStore::empty();
    for root in roots {
        store.add(root).unwrap();
    }
    let mut config = ClientConfig::builder_with_provider(crate::common::pki::provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(store)
        .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Arc::new(config)
}

/// A connector trusting the run CA and offering `alpn`.
pub fn trusting_run_ca(ca: &RunCa, alpn: &[&[u8]]) -> TlsConnector {
    TlsConnector::from(client_trusting(vec![ca.cert_der().clone()], alpn))
}

/// One request; reads until the fake upstream's `hi`.
pub async fn get<S: AsyncRead + AsyncWrite + Unpin>(t: &mut S) {
    t.write_all(b"GET /v1/models HTTP/1.1\r\nHost: api.openai.com\r\n\r\n")
        .await
        .unwrap();
    let mut got = Vec::new();
    while !got.ends_with(b"hi") {
        let mut b = [0u8; 256];
        let n = t.read(&mut b).await.unwrap();
        assert!(n > 0, "the response ended early");
        got.extend_from_slice(&b[..n]);
    }
}

/// Waits for `n` conn ENDs, then gives (sid, OPEN meta, END tail) per ended conn in open order.
pub async fn conns(calls: &Calls, n: usize) -> Vec<(u64, Value, Value)> {
    within_async(HANG, async {
        loop {
            let ended = ends(calls);
            let got: Vec<_> = (opens(calls).into_iter())
                .filter(|o| o.1 == StreamKind::Conn)
                .filter_map(|(sid, _, meta)| {
                    let end = ended.iter().find(|e| e.0 == sid)?;
                    Some((sid, meta, end.3.clone().unwrap_or_default()))
                })
                .collect();
            if got.len() >= n {
                return got;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
}
