//! Shared support for the `proxy_*` tests that drive the real `proxy::serve`. A test binary
//! includes it as `mod proxy_fixtures;` after `mod common;`, so unused items are allowed here.

#![allow(dead_code)]

use crate::common::recorder::{ends, opens, records};
use crate::common::{HANG, within_async};
use goalcup::connector::HostList;
use goalcup::pipeline::{Calls, Recorder, StreamKind};
use goalcup::proxy::ca::RunCa;
use goalcup::proxy::{ProxyCtx, ProxyHandle, serve};
use goalcup_format::Record;
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
    (serve_with(listener, recorder, hosts, ca), calls)
}

/// The real `serve` on a clone of `listener` with `recorder`.
pub fn serve_with(
    listener: &TcpListener,
    recorder: Recorder,
    hosts: &str,
    ca: RunCa,
) -> ProxyHandle {
    let hosts = HostList::from_file(hosts.as_bytes(), "hosts.toml").unwrap();
    let ctx = ProxyCtx {
        recorder,
        hosts,
        ca,
    };
    serve(listener.try_clone().unwrap(), ctx)
}

/// The real `serve` on a held port with a 4,096 backlog, as the watchdog sets it, a fresh run CA
/// and `recorder`; `env` holds the test hooks, read once in `serve`.
pub fn proxy_with(
    recorder: Recorder,
    hosts: &str,
    env: &[(&str, String)],
) -> (ProxyHandle, SocketAddr, RunCa) {
    for (k, v) in env {
        // SAFETY: nextest runs each test in its own process; this runs before `serve` reads them.
        unsafe { std::env::set_var(k, v) };
    }
    let (listener, addr) = crate::common::ports::held();
    socket2::SockRef::from(&listener).listen(4096).unwrap();
    let ca = RunCa::mint(&["openai.com".to_string()]).unwrap();
    let h = serve_with(&listener, recorder, hosts, ca.clone());
    (h, addr, ca)
}

/// [`proxy_with`] and a test recorder with a room of 1 GiB.
pub fn proxy(hosts: &str, env: &[(&str, String)]) -> (ProxyHandle, SocketAddr, Calls, RunCa) {
    let (recorder, calls) = Recorder::for_tests(1 << 30);
    let (h, addr, ca) = proxy_with(recorder, hosts, env);
    (h, addr, calls, ca)
}

/// [`proxy_with`] for `api.openai.com` sent to the upstream at `up`, whose certificates `pki`
/// signs.
pub fn proxy_to(
    recorder: Recorder,
    up: SocketAddr,
    pki: &crate::common::pki::TestPki,
) -> (ProxyHandle, SocketAddr, RunCa) {
    // `serve` reads the roots file once, so the folder may go when this returns.
    let dir = tempfile::tempdir().unwrap();
    let roots = pki.roots_file(dir.path()).display().to_string();
    let env = [
        ("GOALCUP_TEST_EXTRA_ROOTS", roots),
        ("GOALCUP_TEST_UPSTREAM", format!("api.openai.com={up}")),
    ];
    proxy_with(recorder, HOSTS, &env)
}

/// Reads until `done` says the bytes so far are enough; gives them all.
pub async fn read_to<S: AsyncRead + Unpin>(t: &mut S, done: impl Fn(&[u8]) -> bool) -> Vec<u8> {
    within_async(HANG, async {
        let mut got = Vec::new();
        while !done(&got) {
            let mut b = vec![0u8; 64 * 1024];
            let n = t.read(&mut b).await.unwrap();
            assert!(n > 0, "the stream ended early");
            got.extend_from_slice(&b[..n]);
        }
        got
    })
    .await
}

/// Polls the entry in `dir` until `done` accepts its records; gives them.
pub async fn wait_records(dir: &std::path::Path, done: impl Fn(&[Record]) -> bool) -> Vec<Record> {
    within_async(HANG, async {
        loop {
            let recs = records(dir);
            if done(&recs) {
                return recs;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
}

/// PRD-17 Interfaces (PRD-08): runs `goalcup-verify check --live` on the entry folder and
/// asserts exit 0, so a lessons test that writes a part file proves the record whole. This crate
/// has no dev-dependency on the verifier (the scaffold is PRD-02's), so the binary is built and
/// run through `cargo run` from the workspace root; the entry is still recording, hence `--live`.
pub fn verify_exits_0(entry: &std::path::Path) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = std::process::Command::new(cargo)
        .current_dir(root)
        .args(["run", "-q", "-p", "goalcup-verify", "--", "check", "--live"])
        .arg(entry)
        .output()
        .expect("run goalcup-verify");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// At least one OPEN, and an END for every OPEN.
pub fn all_ended(recs: &[Record]) -> bool {
    let count = |f: fn(&Record) -> bool| recs.iter().filter(|r| f(r)).count();
    let open = count(|r| matches!(r, Record::Open { .. }));
    open > 0 && open == count(|r| matches!(r, Record::End { .. }))
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
/// A client through a decrypted CONNECT to `api.openai.com` that trusts the run CA and writes
/// `first` at once.
pub async fn tls_client(
    addr: SocketAddr,
    ca: &RunCa,
    alpn: &[&[u8]],
    first: &[u8],
) -> tokio_rustls::client::TlsStream<TcpStream> {
    let s = through(addr, "api.openai.com:443").await;
    let name = rustls::pki_types::ServerName::try_from("api.openai.com").unwrap();
    let mut t = trusting_run_ca(ca, alpn).connect(name, s).await.unwrap();
    t.write_all(first).await.unwrap();
    t
}

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
