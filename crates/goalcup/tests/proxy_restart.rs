//! PRD-03 section 10: a second boot serves the listener the watchdog holds, with the same CA, and
//! `quiet_since` follows relayed bytes only.

mod common;

use common::pki::HTTP11;
use common::upstream::{Script, Upstream};
use common::{HANG, within_async};
use goalcup::connector::HostList;
use goalcup::pipeline::Recorder;
use goalcup::proxy::ca::{RUN_CA_FILE, RunCa, certificates};
use goalcup::proxy::{ProxyCtx, ProxyHandle, serve};
use rustls::pki_types::{CertificateDer, ServerName};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;

const HOSTS: &str = "provider = [\"api.openai.com\"]\n";

fn boot(listener: &TcpListener, ca: RunCa) -> ProxyHandle {
    let (recorder, _calls) = Recorder::for_tests(1 << 30);
    let hosts = HostList::from_file(HOSTS.as_bytes(), "hosts.toml").unwrap();
    let ctx = ProxyCtx {
        recorder,
        hosts,
        ca,
    };
    serve(listener.try_clone().unwrap(), ctx)
}

const CONNECT: &[u8] = b"CONNECT api.openai.com:443 HTTP/1.1\r\nHost: api.openai.com\r\n\r\n";
const ANSWER: &[u8] = b"HTTP/1.1 200 Connection established\r\n\r\n";

async fn answered(s: &mut TcpStream) {
    let mut head = [0u8; ANSWER.len()];
    s.read_exact(&mut head).await.unwrap();
    assert_eq!(head, ANSWER);
}

/// A connection queued in the backlog while no boot runs is served by the next boot, whose
/// imported CA signs a leaf the first boot's `run-ca.pem` trusts.
#[tokio::test]
async fn the_next_boot_serves_the_backlog_with_the_same_ca() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[HTTP11]));
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: nextest runs each test in its own process; set before either `serve` reads them.
    unsafe {
        std::env::set_var("GOALCUP_TEST_UPSTREAM", format!("api.openai.com={}", up.addr()));
        std::env::set_var("GOALCUP_TEST_EXTRA_ROOTS", up.pki().roots_file(dir.path()));
    }
    let (listener, addr) = common::ports::held();
    let first = RunCa::mint(&["openai.com".to_string()]).unwrap();
    first.write_files(dir.path()).unwrap();
    drop(boot(&listener, first.clone()));

    let mut queued = TcpStream::connect(addr).await.unwrap();
    queued.write_all(CONNECT).await.unwrap();
    let _second = boot(&listener, RunCa::import(&first.export()).unwrap());
    within_async(HANG, answered(&mut queued)).await;

    let mut roots = rustls::RootCertStore::empty();
    let pem = std::fs::read(dir.path().join(RUN_CA_FILE)).unwrap();
    for der in certificates(&pem) {
        roots.add(CertificateDer::from(der)).unwrap();
    }
    let mut config = rustls::ClientConfig::builder_with_provider(common::pki::provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![HTTP11.to_vec()];
    let name = ServerName::try_from("api.openai.com").unwrap();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let mut t = connector.connect(name, queued).await.expect("the leaf verifies");
    t.write_all(b"GET / HTTP/1.1\r\nHost: api.openai.com\r\n\r\n").await.unwrap();
    let mut got = Vec::new();
    while !got.ends_with(b"hi") {
        let mut b = [0u8; 256];
        let n = t.read(&mut b).await.unwrap();
        assert!(n > 0, "the response ended early");
        got.extend_from_slice(&b[..n]);
    }
}

async fn through(proxy: std::net::SocketAddr, target: &str) -> TcpStream {
    let mut s = TcpStream::connect(proxy).await.unwrap();
    let head = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n");
    s.write_all(head.as_bytes()).await.unwrap();
    answered(&mut s).await;
    s
}

/// `quiet_since` moves when a relay moves bytes; an open connection that relays nothing leaves it.
#[tokio::test]
async fn quiet_since_moves_with_relayed_bytes_only() {
    let (echo, target) = common::ports::held();
    std::thread::spawn(move || {
        for s in echo.incoming().flatten() {
            std::thread::spawn(move || {
                let mut r = s;
                let mut w = r.try_clone().unwrap();
                let _ = std::io::copy(&mut r, &mut w);
            });
        }
    });
    let (listener, addr) = common::ports::held();
    let h = boot(&listener, RunCa::mint(&["openai.com".to_string()]).unwrap());
    let before = h.quiet_since();
    let mut busy = through(addr, &target.to_string()).await;
    busy.write_all(b"ping").await.unwrap();
    let mut back = [0u8; 4];
    busy.read_exact(&mut back).await.unwrap();
    // The relay counts a chunk just after writing it on.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let moved = h.quiet_since();
    assert!(moved > before);

    let _idle = through(addr, &target.to_string()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(h.quiet_since(), moved, "an open silent connection is not traffic");
}
