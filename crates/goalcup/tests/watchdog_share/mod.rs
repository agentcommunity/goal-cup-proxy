//! Helpers the `watchdog_*` test binaries share (PRD-02). Each includes it as
//! `mod watchdog_share;` next to `mod common;`, so unused items are allowed here.

#![allow(dead_code)]

use crate::common::pki::{HTTP11, provider};
use goalcup::connector::{HostList, NativeSources};
use goalcup::proxy::ca::{RUN_CA_FILE, certificates};
use goalcup::watchdog::StartFacts;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The longest wait of a `watchdog_*` test: a hang bound only.
pub const WAIT: Duration = Duration::from_secs(90);

/// Waits until `f` gives `Some`, at most [`WAIT`].
pub fn until<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The run and entry ids of a `watchdog_*` test entry.
pub const RUN: &str = "r0123456789abcdef";
pub const ENTRY: &str = "e0123456789abcdef";

/// Start facts for the test entry in `entry_dir`: a fake built-in connector, `hosts`, no mask
/// rules or values, the CA `ca` (a `CaPem` as JSON) and every optional fact left out.
pub fn facts(entry_dir: &Path, workspace: &Path, hosts: &HostList, ca: Value) -> StartFacts {
    serde_json::from_value(json!({"run": RUN, "entry": ENTRY, "segment": 0,
        "entry_dir": entry_dir, "local_wall": 0, "workspace": workspace, "keep": true,
        "connector": {"id": "fake", "name": "Fake", "sha256": "0".repeat(64), "source": "builtin"},
        "hosts": hosts, "hosts_sha256": hosts.sha256(), "native": NativeSources::default(),
        "mask_rules": [], "values": [], "ca": ca, "notes": []}))
    .unwrap()
}

/// L-C12: the one socket listening on `port` is bound to 127.0.0.1 (none on IPv6).
pub fn loopback_only(port: u16) {
    let tail = format!(":{port:04X}");
    let mut listening = Vec::new();
    for file in ["/proc/net/tcp", "/proc/net/tcp6"] {
        for l in std::fs::read_to_string(file)
            .unwrap_or_default()
            .lines()
            .skip(1)
        {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() > 3 && f[3] == "0A" && f[1].ends_with(&tail) {
                listening.push(f[1].to_string());
            }
        }
    }
    assert_eq!(listening, [format!("0100007F{tail}")], "port {port}");
}

/// The CONNECT a test sends to the proxy for the listed host.
pub const CONNECT: &[u8] =
    b"CONNECT api.openai.com:443 HTTP/1.1\r\nHost: api.openai.com:443\r\n\r\n";

/// After [`CONNECT`] was written on `tcp`: its `200` head, then TLS whose leaf must verify
/// against `pem` (a `run-ca.pem`) only.
pub fn tls_over(
    mut tcp: TcpStream,
    pem: &[u8],
) -> rustls::StreamOwned<rustls::ClientConnection, TcpStream> {
    tcp.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut b = [0u8];
        assert_eq!(tcp.read(&mut b).unwrap(), 1, "the proxy closed the CONNECT");
        head.push(b[0]);
    }
    assert!(head.starts_with(b"HTTP/1.1 200"), "{head:?}");
    let mut roots = rustls::RootCertStore::empty();
    for c in certificates(pem) {
        roots.add(c.into()).unwrap();
    }
    let mut config = rustls::ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![HTTP11.to_vec()];
    let name = rustls::pki_types::ServerName::try_from("api.openai.com").unwrap();
    let conn = rustls::ClientConnection::new(Arc::new(config), name).unwrap();
    rustls::StreamOwned::new(conn, tcp)
}

/// [`CONNECT`] and [`tls_over`] on a new connection to `port`, verified against the entry's `run-ca.pem`.
pub fn connect_tls(
    entry_dir: &Path,
    port: u16,
) -> rustls::StreamOwned<rustls::ClientConnection, TcpStream> {
    let pem = std::fs::read(entry_dir.join(RUN_CA_FILE)).unwrap();
    let mut tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
    tcp.write_all(CONNECT).unwrap();
    tls_over(tcp, &pem)
}

/// Writes `request` and reads until the answer ends with `tail` or the stream ends.
pub fn exchange(
    tls: &mut rustls::StreamOwned<rustls::ClientConnection, TcpStream>,
    request: &[u8],
    tail: &[u8],
) -> Vec<u8> {
    tls.write_all(request)
        .expect("the leaf verifies against run-ca.pem");
    let (mut out, mut buf) = (Vec::new(), [0u8; 4096]);
    while !out.ends_with(tail) {
        match tls.read(&mut buf) {
            Ok(n) if n > 0 => out.extend_from_slice(&buf[..n]),
            _ => break,
        }
    }
    out
}
