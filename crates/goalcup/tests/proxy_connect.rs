//! PRD-03 `proxy_connect.rs`, admission building blocks (core slice part 1). The serve-level
//! cases (2,000 connections, `may_accept`, reset passing, decisions) land with `serve`.

use bytes::Bytes;
use goalcup::pipeline::{Call, Recorder};
use goalcup::proxy::connect::{
    CLIENT_HELLO_GUARD, First, Hello, NoHead, Refused, RefusalNotes, connect_upstream,
    first_bytes, peek_client_hello, read_head,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

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
    assert_eq!(&info.bytes[..], &sent[..], "every byte read is kept for replay");
}

#[tokio::test]
async fn fingerprint_ignores_order_and_grease_and_sees_ciphers() {
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
    let (fa, fb, fc) = (fp(parse(&a).await), fp(parse(&b).await), fp(parse(&c).await));
    assert_eq!(fa, fb);
    assert_ne!(fa, fc);
}

#[tokio::test]
async fn client_hello_past_64_kib_is_too_big_and_keeps_every_byte() {
    // Extensions hold at most 64 KiB, so a long cipher list takes the hello past the guard.
    let suites: Vec<u16> = (0..1_000).map(|i| 0x1301 + i).collect();
    let sent = hello(
        &suites,
        &[sni_ext("api.openai.com"), (21, vec![0; 64_000])],
    );
    assert!(sent.len() > CLIENT_HELLO_GUARD);
    let mut io = &sent[..];
    let Hello::TooBig(kept) = peek_client_hello(&mut io, Bytes::new()).await else {
        panic!("a ClientHello past the guard must be TooBig");
    };
    assert!(!kept.is_empty() && kept.len() <= CLIENT_HELLO_GUARD);
    let mut replay = kept.to_vec();
    replay.extend_from_slice(io);
    assert_eq!(replay, sent, "the bytes read plus the unread rest are the whole hello");
}

#[tokio::test]
async fn server_speaks_first_is_seen_at_once() {
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut s, _) = upstream.accept().await.unwrap();
        s.write_all(b"220 ready\r\n").await.unwrap();
        let mut sink = [0u8; 16];
        let _ = s.read(&mut sink).await;
    });
    let front = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let _silent_client = TcpStream::connect(front.local_addr().unwrap())
        .await
        .unwrap();
    let (mut client_leg, _) = front.accept().await.unwrap();
    let mut upstream_leg = connect_upstream("server-first.test", 25, true, Some(upstream_addr))
        .await
        .unwrap();

    let started = Instant::now();
    let first = tokio::time::timeout(
        Duration::from_secs(10),
        first_bytes(&mut client_leg, &mut upstream_leg),
    )
    .await
    .expect("first_bytes waited for the silent client");
    assert_eq!(first, First::Server(Bytes::from_static(b"220 ready\r\n")));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn non_connect_gets_405_and_one_note_per_minute() {
    let (rec, calls) = Recorder::for_tests(1 << 20);
    let notes = RefusalNotes::new();
    for _ in 0..2 {
        let mut io: &[u8] = b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let got = read_head(&mut io, &rec).await;
        assert_eq!(
            got,
            Err(NoHead::Refused(Refused {
                status: 405,
                host: None
            }))
        );
        notes.note(&rec, None, 405);
    }
    let all = calls.all();
    let refused: Vec<_> = all
        .iter()
        .filter_map(|c| match c {
            Call::Event(v) if v["what"] == "connect_refused" => Some(v.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0]["ev"], "note");
    assert_eq!(refused[0]["status"], 405);
    assert!(refused[0].get("host").is_none());
    assert!(
        !all.iter().any(|c| matches!(c, Call::Open(..))),
        "a refusal opens no stream"
    );
}
