//! PRD-03 `proxy_connect.rs`: the ClientHello building blocks, then admission through the real
//! `proxy::serve`.

mod common;
mod proxy_fixtures;

use bytes::Bytes;
use common::pki::HTTP11;
use common::recorder::{events, gaps, opens};
use common::upstream::{Script, Upstream};
use common::{HANG, within_async};
use goalcup::pipeline::{Call, GapReason, StreamKind};
use goalcup::proxy::connect::{CLIENT_HELLO_GUARD, Hello, peek_client_hello};
use proxy_fixtures::{HOSTS, ask, connect, conns, get, proxy, status, through, trusting_run_ca};
use rustls::pki_types::ServerName;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;

/// A ClientHello as rustls writes it.
fn rustls_hello(sni: &str, alpn: &[&[u8]]) -> Vec<u8> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|a| a.to_vec()).collect();
    let name = rustls::pki_types::ServerName::try_from(sni.to_owned()).unwrap();
    let mut conn = rustls::ClientConnection::new(Arc::new(config), name).unwrap();
    let mut out = Vec::new();
    conn.write_tls(&mut out).unwrap();
    out
}

fn u16s(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}

fn with_len16(body: Vec<u8>) -> Vec<u8> {
    let mut out = (body.len() as u16).to_be_bytes().to_vec();
    out.extend(body);
    out
}

fn sni_ext(name: &str) -> (u16, Vec<u8>) {
    let mut entry = vec![0u8];
    entry.extend(with_len16(name.as_bytes().to_vec()));
    (0, with_len16(entry))
}

fn groups_ext(groups: &[u16]) -> (u16, Vec<u8>) {
    (10, with_len16(u16s(groups)))
}

fn alpn_ext(names: &[&[u8]]) -> (u16, Vec<u8>) {
    let mut list = Vec::new();
    for name in names {
        list.push(name.len() as u8);
        list.extend_from_slice(name);
    }
    (16, with_len16(list))
}

/// A hand-made ClientHello, split into TLS records of at most 16 KiB.
fn hello(suites: &[u16], exts: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut body = vec![0x03, 0x03];
    body.extend([7u8; 32]);
    body.push(0);
    body.extend(with_len16(u16s(suites)));
    body.extend([1, 0]);
    let mut block = Vec::new();
    for (kind, data) in exts {
        block.extend(kind.to_be_bytes());
        block.extend((data.len() as u16).to_be_bytes());
        block.extend(data);
    }
    body.extend(with_len16(block));
    let mut handshake = vec![1u8];
    handshake.extend(&(body.len() as u32).to_be_bytes()[1..]);
    handshake.extend(body);
    let mut out = Vec::new();
    for record in handshake.chunks(16 * 1024) {
        out.extend([22, 3, 1]);
        out.extend((record.len() as u16).to_be_bytes());
        out.extend(record);
    }
    out
}

async fn parse(bytes: &[u8]) -> Hello {
    let mut io = bytes;
    peek_client_hello(&mut io, Bytes::new()).await
}

#[tokio::test]
async fn rustls_client_hello_gives_sni_and_alpn() {
    let sent = rustls_hello("api.openai.com", &[b"h2", b"http/1.1"]);
    // Handed over in two pieces: what the head carried, then the socket.
    let (already, rest) = sent.split_at(7);
    let mut io = rest;
    let Hello::Parsed(info) = peek_client_hello(&mut io, Bytes::copy_from_slice(already)).await
    else {
        panic!("the ClientHello did not parse");
    };
    assert_eq!(info.sni.as_deref(), Some("api.openai.com"));
    assert_eq!(info.alpn, vec![b"h2".to_vec(), b"http/1.1".to_vec()]);
    assert_eq!(
        &info.bytes[..],
        &sent[..],
        "every byte read is kept for replay"
    );
}

#[tokio::test]
async fn fingerprint_ignores_order_grease_and_resumption_and_sees_ciphers() {
    let a = hello(
        &[0x0a0a, 0x1301, 0x1302, 0xc02b],
        &[
            (0x1a1a, vec![]),
            sni_ext("api.anthropic.com"),
            groups_ext(&[0x2a2a, 0x001d, 0x0017]),
            alpn_ext(&[b"http/1.1"]),
            (43, vec![2, 0x03, 0x04]),
            (0x3a3a, vec![0]),
        ],
    );
    let b = hello(
        &[0xc02b, 0x1302, 0x4a4a, 0x1301],
        &[
            (43, vec![2, 0x03, 0x04]),
            (0x5a5a, vec![0]),
            alpn_ext(&[b"http/1.1"]),
            groups_ext(&[0x0017, 0x6a6a, 0x001d]),
            sni_ext("api.anthropic.com"),
            // Padding and a resumption's early_data and pre_shared_key: same client.
            (21, vec![0; 40]),
            (42, vec![]),
            (41, vec![0; 8]),
        ],
    );
    let c = hello(
        &[0x1301, 0x1303, 0xc02b],
        &[
            sni_ext("api.anthropic.com"),
            groups_ext(&[0x001d, 0x0017]),
            alpn_ext(&[b"http/1.1"]),
            (43, vec![2, 0x03, 0x04]),
        ],
    );
    let fp = |h: Hello| match h {
        Hello::Parsed(info) => info.fingerprint,
        other => panic!("not parsed: {other:?}"),
    };
    let (fa, fb, fc) = (
        fp(parse(&a).await),
        fp(parse(&b).await),
        fp(parse(&c).await),
    );
    assert_eq!(fa, fb);
    assert_ne!(fa, fc);
}

#[tokio::test]
async fn client_hello_past_64_kib_is_too_big_and_keeps_every_byte() {
    // Extensions hold at most 64 KiB, so a long cipher list takes the hello past the guard.
    let suites: Vec<u16> = (0..1_000).map(|i| 0x1301 + i).collect();
    let sent = hello(&suites, &[sni_ext("api.openai.com"), (21, vec![0; 64_000])]);
    assert!(sent.len() > CLIENT_HELLO_GUARD);
    let mut io = &sent[..];
    let Hello::TooBig(kept) = peek_client_hello(&mut io, Bytes::new()).await else {
        panic!("a ClientHello past the guard must be TooBig");
    };
    assert!(!kept.is_empty() && kept.len() <= CLIENT_HELLO_GUARD);
    let mut replay = kept.to_vec();
    replay.extend_from_slice(io);
    assert_eq!(
        replay, sent,
        "the bytes read plus the unread rest are the whole hello"
    );
}

/// A loopback upstream that writes `banner` to each connection, then echoes it.
fn echo(banner: &'static [u8]) -> SocketAddr {
    let (l, addr) = common::ports::held();
    socket2::SockRef::from(&l).listen(4096).unwrap();
    l.set_nonblocking(true).unwrap();
    let l = tokio::net::TcpListener::from_std(l).unwrap();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = l.accept().await {
            tokio::spawn(async move {
                let _ = s.write_all(banner).await;
                let (mut r, mut w) = s.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    addr
}

/// No connection cap: 2,000 at once to an unlisted host, all open together, are all served.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_thousand_concurrent_connects_are_all_served() {
    common::fds::raise(10_000);
    let (_h, addr, calls, _) = proxy(HOSTS, &[]);
    let target = format!("{}", echo(b""));
    let all_open = Arc::new(tokio::sync::Barrier::new(2_000));
    let tasks: Vec<_> = (0..2_000)
        .map(|i| {
            let (target, all_open) = (target.clone(), all_open.clone());
            tokio::spawn(async move {
                let mut s = through(addr, &target).await;
                let ping = format!("ping {i:04}");
                s.write_all(ping.as_bytes()).await.unwrap();
                let mut back = vec![0u8; ping.len()];
                s.read_exact(&mut back).await.unwrap();
                assert_eq!(back, ping.as_bytes());
                all_open.wait().await;
            })
        })
        .collect();
    within_async(Duration::from_secs(120), async {
        for t in tasks {
            t.await.unwrap();
        }
    })
    .await;
    assert_eq!(conns(&calls, 2_000).await.len(), 2_000);
}

fn soft_limit(soft: Option<u64>) -> u64 {
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit and setrlimit only touch the struct we own.
    unsafe {
        assert_eq!(libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim), 0);
        let old = lim.rlim_cur;
        if let Some(soft) = soft {
            lim.rlim_cur = soft;
            assert_eq!(libc::setrlimit(libc::RLIMIT_NOFILE, &lim), 0);
        }
        old
    }
}

/// Rule 7: while `may_accept()` is false a new CONNECT waits in the backlog, never refused, and
/// the episode writes one `fd_limit` note; it is served once a connection closes. Connections still
/// before their decision (here: waiting for the first bytes) count.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepts_wait_while_descriptors_are_short_and_never_refuse() {
    // The reserve reads the soft limit once per process (nextest: one process per test). It reads
    // 200 here, then the test gets its own limit back, so only the reserve holds back:
    // 2 x 68 + 64 = 200 is not under 200, so the 69th waits.
    let real = soft_limit(Some(200));
    assert!(goalcup::pipeline::fds::may_accept());
    soft_limit(Some(real));
    let (_h, addr, calls, _) = proxy(HOSTS, &[]);
    let target = echo(b"").to_string();
    let mut held = Vec::new();
    for _ in 0..68 {
        held.push(through(addr, &target).await);
    }
    let mut waiting = TcpStream::connect(addr).await.unwrap();
    let head = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n");
    waiting.write_all(head.as_bytes()).await.unwrap();
    let mut b = [0u8; 1];
    let early = tokio::time::timeout(Duration::from_millis(500), waiting.read(&mut b)).await;
    assert!(
        early.is_err(),
        "no answer and no close while descriptors are short"
    );
    let note = json!({"ev": "note", "what": "fd_limit", "limit": 200, "open": 68});
    within_async(HANG, async {
        while !events(&calls).contains(&note) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    let notes = events(&calls)
        .into_iter()
        .filter(|v| v["what"] == "fd_limit");
    assert_eq!(notes.count(), 1, "one note per episode");
    // Serving the 69th starts a new episode at 68 open, with a note of its own.
    drop(held.pop());
    assert_eq!(within_async(HANG, status(&mut waiting)).await, 200);
}

/// A client that closes after the `200` without a byte, as a cancelled request does, loses
/// nothing: its conn on a listed host carries no GAP.
#[tokio::test]
async fn a_connect_abandoned_before_any_byte_names_no_loss() {
    let up = echo(b"");
    let (_h, addr, calls, _) = proxy(
        HOSTS,
        &[("GOALCUP_TEST_UPSTREAM", format!("api.openai.com={up}"))],
    );
    drop(through(addr, "api.openai.com:443").await);
    let got = conns(&calls, 1).await;
    assert_eq!(
        (got[0].2["up"].as_u64(), got[0].2["down"].as_u64()),
        (Some(0), Some(0))
    );
    assert!(gaps(&calls).is_empty(), "no loss to name");
}

/// C9: a server-speaks-first protocol tunnels at once (the old proxy waited 30 s for the client).
#[tokio::test]
async fn a_server_first_protocol_tunnels_with_no_wait() {
    let (_h, addr, calls, _) = proxy(HOSTS, &[]);
    let mut s = through(addr, &format!("{}", echo(b"220 ready\r\n"))).await;
    let mut banner = [0u8; 11];
    let read = tokio::time::timeout(Duration::from_secs(10), s.read_exact(&mut banner));
    read.await
        .expect("the banner came before the client spoke")
        .unwrap();
    assert_eq!(&banner, b"220 ready\r\n");
    drop(s);
    let got = conns(&calls, 1).await;
    assert_eq!(
        (got[0].1["mode"].clone(), got[0].1["reason"].clone()),
        (json!("tunnel"), json!("not_tls"))
    );
}

/// Rule 3: a non-CONNECT request gets `405` and, for two tries within a minute, one note.
#[tokio::test]
async fn a_non_connect_request_gets_405_and_one_note() {
    let (_h, addr, calls, _) = proxy(HOSTS, &[]);
    for _ in 0..2 {
        let head = "GET http://someone@Example.com:80/x?q=1 HTTP/1.1\r\nHost: example.com\r\n\r\n";
        assert_eq!(ask(addr, head).await.1, 405);
    }
    let notes: Vec<_> = (events(&calls).into_iter())
        .filter(|v| v["what"] == "connect_refused")
        .collect();
    assert_eq!(
        notes,
        [json!({"ev": "note", "what": "connect_refused", "status": 405, "host": "example.com"})]
    );
    assert!(opens(&calls).is_empty(), "a refusal opens no stream");
}

/// Rule 3: an upstream that cannot be reached gets `502`, as the harness would see without us.
#[tokio::test]
async fn an_unreachable_upstream_gets_502() {
    let (_h, addr, _calls, _) = proxy(HOSTS, &[]);
    let (_refusing, dead) = common::ports::refusing();
    assert_eq!(connect(addr, &format!("{dead}")).await.1, 502);
}

/// GAP 1: a ClientHello past 64 KiB to a listed CONNECT host is a `hello_too_big` tunnel whose
/// `memory_guard` GAP covers every byte each way.
#[tokio::test]
async fn a_client_hello_past_64_kib_tunnels_with_its_guard_gap() {
    let up = echo(b"");
    let (_h, addr, calls, _) = proxy(
        HOSTS,
        &[("GOALCUP_TEST_UPSTREAM", format!("api.openai.com={up}"))],
    );
    let mut s = through(addr, "api.openai.com:443").await;
    let suites: Vec<u16> = (0..1_000).map(|i| 0x1301 + i).collect();
    let wire = hello(&suites, &[sni_ext("api.openai.com"), (21, vec![0; 64_000])]);
    s.write_all(&wire).await.unwrap();
    s.shutdown().await.unwrap();
    let mut back = Vec::new();
    s.read_to_end(&mut back).await.unwrap();
    assert_eq!(back, wire, "every byte relayed");
    let got = conns(&calls, 1).await;
    let (sid, meta, tail) = &got[0];
    assert_eq!(meta["reason"], "hello_too_big");
    let lost = tail["up"].as_u64().unwrap() + tail["down"].as_u64().unwrap();
    assert_eq!(lost, 2 * wire.len() as u64);
    let guard = json!({"guard": "client_hello", "limit": CLIENT_HELLO_GUARD});
    assert!(
        matches!(&gaps(&calls)[..], [Call::Gap(s, GapReason::MemoryGuard, _, _, 0, Some(l), Some(t))]
        if s == sid && *l == lost && *t == guard)
    );
}

/// C10 and the decrypted HTTP/1.1 path: a listed SNI behind an IP-literal CONNECT is decrypted,
/// checked upstream against the SNI name, and recorded with its byte counts.
#[tokio::test]
async fn a_listed_sni_behind_an_ip_literal_is_decrypted_and_recorded() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[HTTP11]));
    let dir = tempfile::tempdir().unwrap();
    let roots = up.pki().roots_file(dir.path()).display().to_string();
    let (_h, addr, calls, ca) = proxy(HOSTS, &[("GOALCUP_TEST_EXTRA_ROOTS", roots)]);
    let s = through(addr, &up.addr().to_string()).await;
    let name = ServerName::try_from("api.openai.com").unwrap();
    let mut t = trusting_run_ca(&ca, &[HTTP11])
        .connect(name, s)
        .await
        .unwrap();
    get(&mut t).await;
    t.shutdown().await.unwrap();
    drop(t);
    let got = conns(&calls, 1).await;
    let (_, meta, tail) = &got[0];
    assert_eq!(
        (&meta["mode"], &meta["alpn"]),
        (&json!("decrypt"), &json!("http/1.1"))
    );
    let kinds: Vec<_> = opens(&calls).into_iter().map(|o| o.1).collect();
    assert!(kinds.contains(&StreamKind::HttpReq) && kinds.contains(&StreamKind::HttpResp));
    for key in ["up", "down", "plain_up", "plain_down"] {
        assert!(tail[key].as_u64().unwrap() > 0, "{key}");
    }
}
