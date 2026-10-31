//! PRD-17 section 2.1, rows C1 to C8, C10 and T3: what the 0.7 CONNECT path learned, through the
//! real `proxy::serve` over the scripted upstream. Assertions are on answers, conn records, GAP
//! reasons and event order, never on wording. Cases PRD-03's `proxy_connect.rs` holds (the
//! 64 KiB hello guard, 2,000 connections, a server-first banner) are not repeated here, and the
//! OpenSSL ClientHello fixtures of C4 are not copied: rustls hellos cut two ways stand in.

mod common;
mod proxy_fixtures;

use common::pki::HTTP11;
use common::recorder::{ends, gaps};
use common::upstream::{Script, Upstream};
use common::{HANG, within_async};
use goalcup::pipeline::{Call, Calls, GapReason};
use goalcup::proxy::ProxyHandle;
use goalcup::proxy::ca::RunCa;
use proxy_fixtures::{HOSTS, client_trusting, connect, conns, get, proxy, through, trusting_run_ca};
use rustls::pki_types::{CertificateDer, ServerName};
use serde_json::json;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_rustls::TlsConnector;

/// The proxy in front of a TLS upstream for `api.openai.com` and `example.com`. `trust` puts the
/// upstream's CA in the proxy's roots; `map` sends the listed name to the upstream's port.
struct World {
    up: Upstream,
    _dir: tempfile::TempDir,
    _h: ProxyHandle,
    addr: SocketAddr,
    calls: Calls,
    ca: RunCa,
}

fn world(trust: bool, map: bool) -> World {
    let script = Script::ok(b"hi").with_tls(&["api.openai.com", "example.com"], &[HTTP11]);
    let up = Upstream::start(script);
    let dir = tempfile::tempdir().unwrap();
    let mut env = Vec::new();
    if trust {
        let roots = up.pki().roots_file(dir.path()).display().to_string();
        env.push(("GOALCUP_TEST_EXTRA_ROOTS", roots));
    }
    if map {
        env.push(("GOALCUP_TEST_UPSTREAM", format!("api.openai.com={}", up.addr())));
    }
    let (_h, addr, calls, ca) = proxy(HOSTS, &env);
    World { up, _dir: dir, _h, addr, calls, ca }
}

/// A TCP server that accepts and holds every connection; a CONNECT to it is answered `200`.
fn sink(echo: bool) -> SocketAddr {
    let (l, addr) = common::ports::held();
    socket2::SockRef::from(&l).listen(4096).unwrap();
    l.set_nonblocking(true).unwrap();
    let l = tokio::net::TcpListener::from_std(l).unwrap();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = l.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = s.split();
                if echo {
                    let _ = tokio::io::copy(&mut r, &mut w).await;
                } else {
                    let _ = tokio::io::copy(&mut r, &mut tokio::io::sink()).await;
                }
            });
        }
    });
    addr
}

/// How the CONNECT head and the hello reach the proxy.
#[derive(Clone, Copy)]
enum Cut {
    /// Head and hello in one write.
    One,
    /// One byte per write.
    Bytes,
    /// The hello cut into two TLS records, then one byte per write.
    Records,
}

/// A TLS client through the proxy to `target` that sends its hello with the head, as a client
/// that does not wait for the `200` does, then asks for `hi`.
fn tls_via(w: &World, target: &str, sni: &str, root: CertificateDer<'static>, cut: Cut) {
    let name = ServerName::try_from(sni.to_owned()).unwrap();
    let mut conn = rustls::ClientConnection::new(client_trusting(vec![root], &[HTTP11]), name).unwrap();
    let mut hello = Vec::new();
    conn.write_tls(&mut hello).unwrap();
    if let Cut::Records = cut {
        let record = |part: &[u8]| [&hello[..3], &(part.len() as u16).to_be_bytes(), part].concat();
        let mid = 5 + (hello.len() - 5) / 2;
        hello = [record(&hello[5..mid]), record(&hello[mid..])].concat();
    }
    let mut wire = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").into_bytes();
    wire.extend(hello);
    let mut sock = std::net::TcpStream::connect(w.addr).unwrap();
    sock.set_nodelay(true).unwrap();
    sock.set_read_timeout(Some(HANG)).unwrap();
    match cut {
        Cut::One => sock.write_all(&wire).unwrap(),
        _ => wire.iter().for_each(|b| sock.write_all(&[*b]).unwrap()),
    }
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut b = [0u8; 1];
        assert_eq!(sock.read(&mut b).unwrap(), 1, "the proxy closed before the 200");
        head.push(b[0]);
    }
    assert!(head.starts_with(b"HTTP/1.1 200"));
    let mut tls = rustls::StreamOwned::new(conn, sock);
    tls.write_all(b"GET /v1/models HTTP/1.1\r\nHost: api.openai.com\r\n\r\n").unwrap();
    let mut got = Vec::new();
    while !got.ends_with(b"hi") {
        let mut b = [0u8; 256];
        let n = tls.read(&mut b).unwrap();
        assert!(n > 0, "the response ended early");
        got.extend_from_slice(&b[..n]);
    }
}

#[tokio::test]
async fn connect_and_client_hello_in_one_write_are_decrypted() {
    let w = world(true, false);
    tls_via(&w, &w.up.addr().to_string(), "api.openai.com", w.ca.cert_der().clone(), Cut::One);
    assert_eq!(conns(&w.calls, 1).await[0].1["mode"], "decrypt");
}

#[tokio::test]
async fn connect_head_split_over_one_byte_reads_is_decrypted() {
    let w = world(true, false);
    tls_via(&w, &w.up.addr().to_string(), "api.openai.com", w.ca.cert_der().clone(), Cut::Bytes);
    assert_eq!(conns(&w.calls, 1).await[0].1["mode"], "decrypt");
}

#[tokio::test]
async fn client_hello_in_two_records_and_one_byte_reads_decides_by_sni() {
    let w = world(true, false);
    let target = w.up.addr().to_string();
    tls_via(&w, &target, "api.openai.com", w.ca.cert_der().clone(), Cut::Records);
    // An unlisted name is tunnelled: the client meets the upstream's own certificate.
    tls_via(&w, &target, "example.com", w.up.pki().root(), Cut::Records);
    let got = conns(&w.calls, 2).await;
    assert_eq!(got[0].1["mode"], "decrypt");
    assert_eq!((&got[1].1["mode"], &got[1].1["reason"]), (&json!("tunnel"), &json!("not_listed")));
}

/// C2/K14: a broken head gets a fixed answer, and nothing the client sent comes back in it or
/// into a record.
#[tokio::test]
async fn bad_heads_get_fixed_answers_without_echo() {
    const MARK: &str = "marker-5b1e";
    let (_h, addr, calls, _) = proxy(HOSTS, &[]);
    let start = "CONNECT example.com:443 HTTP/1.1\r\nX: ";
    // Exactly the 8 KiB guard with no blank line: nothing is left unread to turn the answer into a reset.
    let big = format!("{start}{}", MARK.repeat(8192).split_at(8192 - start.len()).0);
    let cases = [
        (format!("CONNECT {MARK} HTTP/1.1\r\nX: {MARK}\r\n\r\n"), "400"),
        (format!("{start}a\u{1}{MARK}\r\n\r\n"), "400"),
        (big, "431"),
        (format!("GET http://u:{MARK}@example.com/{MARK}?q={MARK} HTTP/1.1\r\nX: {MARK}\r\n\r\n"), "405"),
    ];
    for (head, code) in cases {
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(head.as_bytes()).await.unwrap();
        let mut reply = Vec::new();
        within_async(HANG, s.read_to_end(&mut reply)).await.unwrap();
        assert_eq!(String::from_utf8_lossy(&reply[9..12]), code);
        assert!(!String::from_utf8_lossy(&reply).contains(MARK));
    }
    assert!(!format!("{:?}", calls.all()).contains(MARK));
}

#[tokio::test]
async fn authority_forms() {
    let up = sink(false);
    let hooks = format!("example.com={up},::1={up},a_b.example.com={up}");
    let (_h, addr, calls, _) = proxy(HOSTS, &[("GOALCUP_TEST_UPSTREAM", hooks)]);
    let accepted = [
        ("example.com:443", "example.com", 443),
        ("Example.COM.:8443", "example.com", 8443),
        ("[::1]:443", "::1", 443),
        ("a_b.example.com:443", "a_b.example.com", 443),
    ];
    for (target, ..) in accepted {
        drop(through(addr, target).await);
    }
    let mut seen: Vec<_> = conns(&calls, accepted.len()).await.iter()
        .map(|c| (c.1["host"].as_str().unwrap().to_owned(), c.1["port"].as_u64().unwrap()))
        .collect();
    seen.sort();
    let mut want: Vec<_> = accepted.iter().map(|a| (a.1.to_owned(), a.2)).collect();
    want.sort();
    assert_eq!(seen, want);
    let refused = ["example.com", "example.com:0", "u@example.com:443", "example.com:443/x",
        "example.com:443?q=1", "example.com:99999", "example.com:", "[::1]"];
    for target in refused {
        assert_eq!(connect(addr, target).await.1, 400, "{target}");
    }
}

/// C5: a client that closes before its hello is whole costs nothing and sends the upstream no
/// request. The upstream leg is dialled before the `200`, so only its bytes can be asserted.
#[tokio::test]
async fn close_before_hello_opens_no_upstream() {
    let w = world(true, true);
    drop(through(w.addr, "api.openai.com:443").await);
    assert_eq!(conns(&w.calls, 1).await.len(), 1);
    assert!(gaps(&w.calls).is_empty(), "no byte moved, no loss to name");
    let mut s = through(w.addr, "api.openai.com:443").await;
    s.write_all(&[22, 3, 1, 2, 0]).await.unwrap(); // a record header that promises 512 bytes
    drop(s);
    assert_eq!(conns(&w.calls, 2).await.len(), 2);
    assert!(w.up.received().iter().all(Vec::is_empty));
}

/// C6: an unlisted host (the agent's own pip, npm, git) gets one round: the refusal is answered
/// at once. A listed host is retried; the bound only guards a hang.
#[tokio::test]
async fn unlisted_refusal_is_one_round() {
    let (_h, addr, _calls, _) = proxy(HOSTS, &[]);
    let (_refusing, dead) = common::ports::refusing();
    let target = dead.to_string();
    let asked = tokio::time::timeout(Duration::from_secs(10), connect(addr, &target));
    assert_eq!(asked.await.expect("a listed host would still be retrying").1, 502);
}

#[tokio::test]
async fn listed_refusal_is_retried() {
    let (sock, dead) = common::ports::refusing();
    let (_h, addr, _calls, _) = proxy(HOSTS, &[("GOALCUP_TEST_UPSTREAM", format!("api.openai.com={dead}"))]);
    let mut call = tokio::spawn(connect(addr, "api.openai.com:443"));
    // A proxy that gave up would have answered `502` at once; this one is still on its rounds.
    assert!(tokio::time::timeout(Duration::from_secs(6), &mut call).await.is_err());
    sock.listen(16).unwrap();
    let (_s, code) = within_async(HANG, async { call.await.unwrap() }).await;
    assert_eq!(code, 200, "the next round connected");
}

/// C7, full tier: a tunnel idle for two minutes is still a tunnel. Proof is the echo after the
/// idle and the absence of an END before it, not a stopwatch.
#[tokio::test]
#[ignore = "full tier: the one 120 s test (PRD-14 section 4)"]
async fn idle_tunnel_survives_120s() {
    let (_h, addr, calls, _) = proxy(HOSTS, &[]);
    let mut s = through(addr, &sink(true).to_string()).await;
    tokio::time::sleep(Duration::from_secs(120)).await;
    assert!(ends(&calls).is_empty(), "nothing ended while the tunnel was idle");
    s.write_all(b"ping").await.unwrap();
    let mut back = [0u8; 4];
    within_async(HANG, s.read_exact(&mut back)).await.unwrap();
    assert_eq!(&back, b"ping");
    drop(s);
    assert_eq!(conns(&calls, 1).await.len(), 1);
    assert!(gaps(&calls).is_empty());
}

/// C8: a handshake the client refuses closes only that connection's two sockets.
#[tokio::test]
async fn one_bad_handshake_leaves_others_relaying() {
    let w = world(true, true);
    let name = || ServerName::try_from("api.openai.com").unwrap();
    let listed = || through(w.addr, "api.openai.com:443");
    let mut good = trusting_run_ca(&w.ca, &[HTTP11]).connect(name(), listed().await).await.unwrap();
    let wrong = TlsConnector::from(client_trusting(vec![w.up.pki().root()], &[HTTP11]));
    assert!(wrong.connect(name(), listed().await).await.is_err(), "the run CA's leaf is refused");
    get(&mut good).await;
    let later = listed().await; // the listener goes on
    drop((good, later));
    let got = conns(&w.calls, 3).await;
    let refused = got.iter().filter(|c| c.1["reason"] == "tls_refused").count();
    let decrypted = got.iter().filter(|c| c.1["mode"] == "decrypt").count();
    assert_eq!((refused, decrypted), (1, 1));
}

/// A listed name whose upstream certificate is not in the proxy's roots is tunnelled with a GAP:
/// a client trusting the run CA refuses what it sees, one trusting the upstream's CA is served.
async fn untrusted(via_ip: bool) {
    let w = world(false, !via_ip);
    let target = if via_ip { w.up.addr().to_string() } else { "api.openai.com:443".into() };
    let name = || ServerName::try_from("api.openai.com").unwrap();
    let by_run_ca = trusting_run_ca(&w.ca, &[HTTP11]).connect(name(), through(w.addr, &target).await);
    assert!(by_run_ca.await.is_err(), "a fake is never trusted as a listed host");
    let own = TlsConnector::from(client_trusting(vec![w.up.pki().root()], &[HTTP11]));
    let mut t = own.connect(name(), through(w.addr, &target).await).await.unwrap();
    get(&mut t).await;
    drop(t);
    let got = conns(&w.calls, 2).await;
    assert!(got.iter().all(|c| c.1["mode"] == "tunnel" && c.1["reason"] == "upstream_untrusted"));
    assert!(gaps(&w.calls).iter().any(|g| matches!(g, Call::Gap(_, GapReason::UpstreamUntrusted, ..))));
}

#[tokio::test]
async fn listed_sni_to_a_local_fake_server_is_never_trusted() {
    untrusted(true).await;
}

#[tokio::test]
async fn untrusted_upstream_reaches_client_unchanged() {
    untrusted(false).await;
}
