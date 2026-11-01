//! PRD-17 section 2.5, rows W2 to W7, W12 and W13: the WebSocket lessons of the 0.7 relay, driven
//! at the parser layer through `proxy::ws::{negotiate, WsConn}` over `Recorder::for_tests`
//! (`proxy_ws.rs` has the Codex fixture, fragmentation around a ping, the RFC 6455 examples and
//! the close code; none of that is repeated here). The serve halves (W2 and W7 relay, W9 to W11,
//! and K15, section 2.6) drive the real `proxy::serve` through a decrypted CONNECT, against the
//! scripted upstream or a raw TLS peer that misbehaves on purpose. Records are what
//! `Recorder::for_tests` shows before masking, except K15, which reads the part file of the real
//! pipeline. The frame and deflate encoders below are private to this file.

mod common;
mod proxy_fixtures;

use common::pki::{HTTP11, TestPki};
use common::recorder::{ends, entry_dir, gaps, opens, real};
use common::upstream::{Answer, Body, Script, Upstream, WsMsg};
use common::{HANG, within_async};
use flate2::{Compress, Compression, FlushCompress, Status};
use goalcup::pipeline::{Call, Calls, EndReason, GapReason, Mono, Recorder, Sid, StreamKind};
use goalcup::proxy::ProxyHandle;
use goalcup::proxy::ca::RunCa;
use goalcup::proxy::http1::Http1;
use goalcup::proxy::pump::{Parser, Stop};
use goalcup::proxy::ws::{Deflate, Dir, WsConn, negotiate};
use goalcup_format::{FLAG_FIN, Record};
use proxy_fixtures::{all_ended, conns, proxy_to, read_to, wait_records};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_rustls::TlsAcceptor;

/// The stream of this kind that was opened first.
fn sid_of(c: &Calls, kind: StreamKind) -> Sid {
    opens(c)
        .into_iter()
        .find_map(|(sid, k, _)| (k == kind).then_some(sid))
        .expect("a stream of this kind was opened")
}

/// The reason `sid` ended with.
fn end_reason(c: &Calls, sid: Sid) -> EndReason {
    ends(c)
        .into_iter()
        .find_map(|(s, reason, ..)| (s == sid).then_some(reason))
        .expect("the stream ended")
}

/// The reasons of the GAPs on `sid`, in order.
fn gap_reasons(c: &Calls, sid: Sid) -> Vec<GapReason> {
    gaps(c)
        .into_iter()
        .filter_map(|call| match call {
            Call::Gap(s, reason, ..) if s == sid => Some(reason),
            _ => None,
        })
        .collect()
}

/// The `lost` of the GAPs on `sid`, in order.
fn gap_lost(c: &Calls, sid: Sid) -> Vec<Option<u64>> {
    gaps(c)
        .into_iter()
        .filter_map(|call| match call {
            Call::Gap(s, .., lost, _) if s == sid => Some(lost),
            _ => None,
        })
        .collect()
}

const KEY: [u8; 4] = [0x37, 0xfa, 0x21, 0x3d];
const ROOM: u64 = 1 << 30;

/// One frame. `first` is the whole first byte (FIN, RSV and opcode); `form` picks the length
/// field (`None` for the shortest one, else 16 or 64 for a longer, non-minimal one).
fn raw_frame(first: u8, payload: &[u8], mask: Option<[u8; 4]>, form: Option<u8>) -> Vec<u8> {
    let mut out = vec![first];
    let bit = if mask.is_some() { 0x80 } else { 0 };
    let n = payload.len();
    match form {
        Some(16) => {
            out.push(bit | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        Some(64) => {
            out.push(bit | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
        _ if n < 126 => out.push(bit | n as u8),
        _ if n <= 0xffff => {
            out.push(bit | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        _ => {
            out.push(bit | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    match mask {
        Some(key) => {
            out.extend_from_slice(&key);
            out.extend(payload.iter().enumerate().map(|(i, b)| b ^ key[i & 3]));
        }
        None => out.extend_from_slice(payload),
    }
    out
}

fn frame(fin: bool, rsv1: bool, opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let first = (u8::from(fin) << 7) | (u8::from(rsv1) << 6) | opcode;
    raw_frame(first, payload, mask, None)
}

/// The mask a sender of this direction uses.
fn mask_for(dir: Dir) -> Option<[u8; 4]> {
    (dir == Dir::Up).then_some(KEY)
}

fn text(dir: Dir, msg: &str) -> Vec<u8> {
    frame(true, false, 0x1, msg.as_bytes(), mask_for(dir))
}

/// Raw DEFLATE of one message with a sync flush and the `00 00 ff ff` tail stripped (RFC 7692).
fn deflate_sync(c: &mut Compress, msg: &[u8]) -> Vec<u8> {
    let start = c.total_in();
    let mut out = Vec::with_capacity(msg.len() + 64);
    loop {
        if out.capacity() - out.len() < 64 {
            out.reserve(4096);
        }
        let done = (c.total_in() - start) as usize;
        c.compress_vec(&msg[done..], &mut out, FlushCompress::Sync)
            .expect("deflate");
        let done = (c.total_in() - start) as usize;
        if done == msg.len() && out.len() < out.capacity() {
            break;
        }
    }
    assert!(out.ends_with(&[0, 0, 0xff, 0xff]));
    out.truncate(out.len() - 4);
    out
}

/// Raw DEFLATE of one message ending in a final block (BFINAL set).
fn deflate_final(msg: &[u8]) -> Vec<u8> {
    let mut c = Compress::new(Compression::default(), false);
    let mut out = Vec::with_capacity(msg.len() + 64);
    loop {
        out.reserve(4096);
        let status = c
            .compress_vec(
                &msg[c.total_in() as usize..],
                &mut out,
                FlushCompress::Finish,
            )
            .expect("deflate");
        if status == Status::StreamEnd {
            return out;
        }
    }
}

fn compressed(dir: Dir, body: &[u8]) -> Vec<u8> {
    frame(true, true, 0x1, body, mask_for(dir))
}

fn both() -> Deflate {
    Deflate {
        client_takeover: true,
        server_takeover: true,
    }
}

fn open(room: u64, deflate: Option<Deflate>) -> (WsConn, Recorder, Calls) {
    let (rec, calls) = Recorder::for_tests(room);
    let ws = WsConn::open(&rec, 1, 2, deflate);
    (ws, rec, calls)
}

/// The stream a direction's bytes are recorded on.
fn stream(calls: &Calls, dir: Dir) -> Sid {
    sid_of(
        calls,
        if dir == Dir::Up {
            StreamKind::WsUp
        } else {
            StreamKind::WsDown
        },
    )
}

/// The messages of one stream, `(opcode, bytes)` in the order they end: data pieces join up to
/// `fin`, a control piece is a whole message.
fn messages(calls: &Calls, sid: Sid) -> Vec<(u8, Vec<u8>)> {
    let (mut data, mut out) = (Vec::new(), Vec::new());
    for call in calls.all() {
        let Call::Ws(s, _, _, opcode, fin, bytes) = call else {
            continue;
        };
        if s != sid {
            continue;
        }
        if opcode & 0x8 != 0 {
            out.push((opcode, bytes.to_vec()));
            continue;
        }
        data.extend_from_slice(&bytes);
        if fin {
            out.push((opcode, std::mem::take(&mut data)));
        }
    }
    out
}

fn texts(calls: &Calls, sid: Sid) -> Vec<String> {
    messages(calls, sid)
        .into_iter()
        .map(|(_, b)| String::from_utf8(b).expect("utf-8"))
        .collect()
}

/// A varied message of about `n` bytes whose repeats are all closer than 1 KiB.
fn near_repeats(seed: usize, n: usize) -> String {
    let mut s = String::new();
    let mut i = seed;
    while s.len() < n {
        s.push_str(&format!("{{\"k{i}\":\"v{}\"}},", i * 7919 % 10007));
        i += 1;
    }
    s
}

// W2: the parameters come from the 101, never from the request; anything that is not RFC 7692
// is refused, and a refusal is a named loss.

/// `Sec-WebSocket-Accept` for the RFC 6455 section 1.3 key the client below always sends.
const GOOD: &str = "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";

/// A `101` head, with an extension answer when given.
fn head101(ext: Option<&str>, accept: &str) -> Vec<u8> {
    let ext = ext.map_or(String::new(), |e| {
        format!("Sec-WebSocket-Extensions: {e}\r\n")
    });
    format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept}\r\n{ext}\r\n"
    )
    .into_bytes()
}

fn upgrade_req(path: &str, offer: bool) -> Vec<u8> {
    let ext = if offer {
        "Sec-WebSocket-Extensions: permessage-deflate\r\n"
    } else {
        ""
    };
    format!(
        "GET {path} HTTP/1.1\r\nHost: api.openai.com\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n{ext}\r\n"
    )
    .into_bytes()
}

/// The proxy in front of an upstream at `up` that `pki` signs for `api.openai.com`.
struct Wire {
    _h: ProxyHandle,
    addr: SocketAddr,
    ca: RunCa,
}

fn wire(up: SocketAddr, pki: &TestPki, recorder: Recorder) -> Wire {
    let (_h, addr, ca) = proxy_to(recorder, up, pki);
    Wire { _h, addr, ca }
}

/// A client through a decrypted CONNECT that writes `first` at once.
async fn client(w: &Wire, first: &[u8]) -> tokio_rustls::client::TlsStream<tokio::net::TcpStream> {
    proxy_fixtures::tls_client(w.addr, &w.ca, &[HTTP11], first).await
}

type Seen = Arc<Mutex<Vec<Vec<u8>>>>;

/// A TLS peer: for the i-th connection it reads the request head, answers `replies[i]` (the last
/// one repeats) in one write, then keeps what the client sends next in `seen[i]`.
fn raw_peer(pki: &TestPki, replies: Vec<Vec<u8>>) -> (SocketAddr, Seen) {
    let (l, addr) = common::ports::held();
    socket2::SockRef::from(&l).listen(4096).unwrap();
    l.set_nonblocking(true).unwrap();
    let l = tokio::net::TcpListener::from_std(l).unwrap();
    let acceptor = TlsAcceptor::from(pki.server_config(&["api.openai.com"], &[HTTP11]));
    let seen = Seen::default();
    let kept = seen.clone();
    tokio::spawn(async move {
        while let Ok((s, _)) = l.accept().await {
            let at = {
                let mut all = kept.lock().unwrap();
                all.push(Vec::new());
                all.len() - 1
            };
            let (acceptor, kept) = (acceptor.clone(), kept.clone());
            let reply = replies[at.min(replies.len() - 1)].clone();
            tokio::spawn(async move {
                let Ok(mut t) = acceptor.accept(s).await else {
                    return;
                };
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    let mut b = [0u8; 1];
                    if t.read(&mut b).await.unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(b[0]);
                }
                let _ = t.write_all(&reply).await;
                let mut b = [0u8; 4096];
                while let Ok(n @ 1..) = t.read(&mut b).await {
                    kept.lock().unwrap()[at].extend_from_slice(&b[..n]);
                }
            });
        }
    });
    (addr, seen)
}

/// The proxy in front of a raw peer that answers `replies`.
struct Raw {
    w: Wire,
    calls: Calls,
    seen: Seen,
    replies: Vec<Vec<u8>>,
}

fn raw(replies: Vec<Vec<u8>>) -> Raw {
    let pki = TestPki::new("lessons ws");
    let (addr, seen) = raw_peer(&pki, replies.clone());
    let (rec, calls) = Recorder::for_tests(ROOM);
    Raw {
        w: wire(addr, &pki, rec),
        calls,
        seen,
        replies,
    }
}

impl Raw {
    /// Connection `i`: an upgrade with `up` behind it in one write. The client must get the
    /// peer's reply byte for byte and the peer `up` byte for byte; gives the conn stream.
    async fn session(&self, i: usize, up: &[u8], offer: bool) -> Sid {
        let reply = &self.replies[i.min(self.replies.len() - 1)];
        let mut first = upgrade_req("/v1/responses", offer);
        first.extend_from_slice(up);
        let mut t = client(&self.w, &first).await;
        assert_eq!(&read_to(&mut t, |g| g.len() >= reply.len()).await, reply);
        within_async(HANG, async {
            while self
                .seen
                .lock()
                .unwrap()
                .get(i)
                .is_none_or(|s| s.len() < up.len())
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert_eq!(self.seen.lock().unwrap()[i], up);
        drop(t);
        conns(&self.calls, i + 1).await[i].0
    }

    fn no_ws_stream(&self) {
        let kinds = opens(&self.calls);
        assert!(
            kinds
                .iter()
                .all(|(_, k, _)| !matches!(k, StreamKind::WsUp | StreamKind::WsDown))
        );
    }
}

/// An upstream that upgrades every request and sends `messages`.
fn ws_upstream(messages: Vec<WsMsg>) -> Upstream {
    let mut script = Script::ok(b"").with_tls(&["api.openai.com"], &[HTTP11]);
    script.answers = vec![Answer::Ws {
        deflate: false,
        messages,
    }];
    Upstream::start(script)
}

/// W2, the relay half: the unknown extension in the `101` is relayed unchanged both ways and
/// the conn stream names the loss; no stream is decoded.
#[tokio::test]
async fn unknown_extension_through_serve() {
    let down = [text(Dir::Down, "down one"), text(Dir::Down, "down two")].concat();
    let up = text(Dir::Up, "up one");
    let answers = ["x-unknown-extension", "permessage-deflate; x_unknown=1"];
    let r = raw(answers
        .map(|a| [head101(Some(a), GOOD), down.clone()].concat())
        .to_vec());
    for i in 0..answers.len() {
        let conn = r.session(i, &up, true).await;
        assert_eq!(gap_reasons(&r.calls, conn), [GapReason::DecodeFailed]);
        // Every frame behind the 101, both ways, sits in that one loss.
        assert_eq!(
            gap_lost(&r.calls, conn),
            [Some((up.len() + down.len()) as u64)]
        );
    }
    r.no_ws_stream();
}

/// W7, the relay half: a violation mid-stream gives the 5a GAP on the WebSocket stream and every
/// byte is still relayed, each way.
#[tokio::test]
async fn violations_through_serve() {
    let down = [
        text(Dir::Down, "first"),
        raw_frame(0xa1, b"x", None, None),
        text(Dir::Down, "last"),
    ];
    let up = [
        text(Dir::Up, "first"),
        frame(true, false, 0x1, b"x", None),
        text(Dir::Up, "last"),
    ];
    let r = raw(vec![[head101(None, GOOD), down.concat()].concat()]);
    r.session(0, &up.concat(), false).await;
    for dir in [Dir::Up, Dir::Down] {
        let sid = stream(&r.calls, dir);
        let got = texts(&r.calls, sid);
        assert_eq!(got, ["first", "last"], "{dir:?}");
        let named = gap_reasons(&r.calls, sid);
        assert!(!named.is_empty() && named.iter().all(|g| *g == GapReason::DecodeFailed));
    }
}

#[tokio::test]
async fn upgrade_101_and_first_frame_in_one_write() {
    // W9: the peer writes the 101 and the first frame as one write; the client's first frame
    // follows its request at once.
    let reply = [head101(None, GOOD), text(Dir::Down, "first message")].concat();
    let r = raw(vec![reply]);
    r.session(0, &text(Dir::Up, "hello up"), false).await;
    assert_eq!(
        texts(&r.calls, stream(&r.calls, Dir::Down)),
        ["first message"]
    );
    assert_eq!(texts(&r.calls, stream(&r.calls, Dir::Up)), ["hello up"]);
    assert!(gaps(&r.calls).is_empty());
}

#[tokio::test]
async fn rejected_upgrade_stays_http() {
    // W10: a non-101 answer to an upgrade request is an HTTP exchange, nothing lost.
    let mut script = Script::ok(b"no").with_tls(&["api.openai.com"], &[HTTP11]);
    script.answers = vec![Answer::Http {
        status: 426,
        headers: Vec::new(),
        body: Body::Whole(b"no".to_vec()),
        coding: None,
    }];
    let up = Upstream::start(script);
    let (rec, calls) = Recorder::for_tests(ROOM);
    let w = wire(up.addr(), up.pki(), rec);
    let mut t = client(&w, &upgrade_req("/v1/responses", false)).await;
    let got = read_to(&mut t, |g| g.ends_with(b"no")).await;
    assert!(got.starts_with(b"HTTP/1.1 426"));
    drop(t);
    conns(&calls, 1).await;
    let kinds: Vec<_> = opens(&calls).into_iter().map(|o| o.1).collect();
    assert!(kinds.contains(&StreamKind::HttpReq) && kinds.contains(&StreamKind::HttpResp));
    assert!(
        !kinds
            .iter()
            .any(|k| matches!(k, StreamKind::WsUp | StreamKind::WsDown))
    );
    assert!(gaps(&calls).is_empty());
}

#[tokio::test]
async fn bad_accept_is_a_named_loss_not_a_websocket() {
    // W10: a 101 whose accept value does not match the key is relayed; parsing stops with one
    // named loss on the conn stream and no WebSocket stream opens.
    let reply = [
        head101(None, "AAAAAAAAAAAAAAAAAAAAAAAAAAA="),
        text(Dir::Down, "x"),
    ]
    .concat();
    let r = raw(vec![reply]);
    let (up, down) = (text(Dir::Up, "y"), text(Dir::Down, "x"));
    let conn = r.session(0, &up, false).await;
    assert_eq!(gap_reasons(&r.calls, conn), [GapReason::DecodeFailed]);
    assert_eq!(
        gap_lost(&r.calls, conn),
        [Some((up.len() + down.len()) as u64)]
    );
    r.no_ws_stream();
}

#[tokio::test]
async fn an_upgrade_on_any_path_is_recorded() {
    // W11: the path does not matter on a decrypted host.
    let up = ws_upstream(vec![WsMsg::Text("hello".into())]);
    let (rec, calls) = Recorder::for_tests(ROOM);
    let w = wire(up.addr(), up.pki(), rec);
    let paths = ["/v1/realtime", "/ws/telemetry", "/"];
    for path in paths {
        let mut t = client(&w, &upgrade_req(path, false)).await;
        read_to(&mut t, |g| g.ends_with(b"hello")).await;
    }
    conns(&calls, paths.len()).await;
    for kind in [StreamKind::WsUp, StreamKind::WsDown] {
        assert_eq!(
            opens(&calls).iter().filter(|o| o.1 == kind).count(),
            paths.len()
        );
    }
}

#[tokio::test]
async fn fail_closed_ends_at_message_end() {
    // K15: message 1 holds an undecided 20 KiB candidate; the fail-closed span ends with that
    // message and the next messages are whole and parse. The messages come down (`ws_down`)
    // from the scripted upstream; both directions run the same stream masker, so the
    // direction does not change the rule.
    let first = format!(
        r#"{{"type":"response.create","input":"sk-{}"}}"#,
        "A9".repeat(10 * 1024)
    );
    let plain: Vec<String> = (2..=3)
        .map(|i| format!(r#"{{"type":"response.create","n":{i},"input":"hello there"}}"#))
        .collect();
    let mut messages = vec![WsMsg::Text(first.clone())];
    messages.extend(plain.iter().cloned().map(WsMsg::Text));
    let up = ws_upstream(messages);
    let entry = entry_dir();
    let rec = real(entry.path());
    let w = wire(up.addr(), up.pki(), rec.clone());
    let mut t = client(&w, &upgrade_req("/v1/responses", false)).await;
    read_to(&mut t, |g| g.ends_with(plain[1].as_bytes())).await;
    drop(t);
    let recs = wait_records(entry.path(), all_ended).await;
    proxy_fixtures::verify_exits_0(&rec, entry.path());
    let sid = recs
        .iter()
        .find_map(|r| match r {
            Record::Open {
                sid,
                skind: StreamKind::WsDown,
                ..
            } => Some(*sid),
            _ => None,
        })
        .expect("a ws_down stream");
    let (mut bytes, mut fins) = (Vec::new(), Vec::new());
    for r in &recs {
        match r {
            Record::Data {
                sid: s,
                flags,
                bytes: b,
                ..
            } if *s == sid => {
                bytes.extend_from_slice(b);
                if flags & FLAG_FIN != 0 {
                    fins.push(bytes.len());
                }
            }
            Record::Mask {
                sid: s, off, len, ..
            } if *s == sid => {
                assert!(
                    off + len <= first.len() as u64,
                    "a mask span crosses a message end"
                );
            }
            _ => {}
        }
    }
    let at = first.len();
    assert_eq!(
        fins,
        [
            at,
            at + plain[0].len(),
            at + plain[0].len() + plain[1].len()
        ]
    );
    // The rule numbers index the start event's `mask_rules`.
    let fail_closed: Vec<u64> = recs
        .iter()
        .find_map(|r| match r {
            Record::Event { json, .. } => serde_json::from_slice::<serde_json::Value>(json)
                .ok()
                .filter(|v| v["ev"] == "start"),
            _ => None,
        })
        .and_then(|start| {
            let rules = start["mask_rules"].as_array()?.clone();
            let at =
                |(i, r): (usize, &serde_json::Value)| (*r == "fail_closed").then_some(i as u64);
            Some(rules.iter().enumerate().filter_map(at).collect())
        })
        .expect("a start event naming the mask rules");
    assert!(
        recs.iter()
            .any(|r| matches!(r, Record::Mask { sid: s, rule, .. }
            if *s == sid && fail_closed.contains(rule))),
        "the candidate is masked by the fail-closed rule"
    );
    assert_eq!(&bytes[at..], plain.concat().as_bytes());
    serde_json::from_slice::<serde_json::Value>(&bytes[at..at + plain[0].len()]).unwrap();
}

#[test]
fn negotiation_accepts_only_rfc7692() {
    let offer = Some("permessage-deflate; client_max_window_bits");
    // No answer is no extension, whatever the request offered.
    assert_eq!(negotiate(offer, None), Ok(None));
    assert_eq!(negotiate(offer, Some("")), Ok(None));
    // The 101 decides: the request's no-takeover offer does not bind a server that did not echo it.
    let asked = Some("permessage-deflate; client_no_context_takeover");
    assert_eq!(
        negotiate(asked, Some("permessage-deflate")),
        Ok(Some(both()))
    );
    // RFC 7692 parameters are all accepted, quoted values included, window bits 8 to 15.
    let ok = |answer: &str| {
        negotiate(offer, Some(answer))
            .expect("supported")
            .expect("deflate")
    };
    assert!(!ok("permessage-deflate; server_no_context_takeover").server_takeover);
    assert!(!ok("permessage-deflate; client_no_context_takeover").client_takeover);
    for bits in [8, 10, 15] {
        let answer = ok(&format!(
            "permessage-deflate; server_max_window_bits={bits}"
        ));
        assert_eq!(answer, both());
        ok(&format!(
            "permessage-deflate; client_max_window_bits={bits}"
        ));
    }
    assert_eq!(
        ok("permessage-deflate; server_max_window_bits=\"10\""),
        both()
    );
    assert_eq!(ok("permessage-deflate; client_max_window_bits"), both());
    assert!(ok("Permessage-Deflate; Server_No_Context_Takeover").client_takeover);
    // Everything else is Unsupported.
    let bad = [
        "permessage-deflate; server_max_window_bits=7",
        "permessage-deflate; server_max_window_bits=16",
        "permessage-deflate; server_max_window_bits",
        "permessage-deflate; server_max_window_bits=\"10",
        "permessage-deflate; server_no_context_takeover; server_no_context_takeover",
        "permessage-deflate; client_max_window_bits=9; client_max_window_bits=9",
        "permessage-deflate; made_up_parameter=1",
        "permessage-deflate; server_no_context_takeover=1",
        "x-webkit-deflate-frame",
        "permessage-deflate, permessage-deflate",
        "permessage-deflate, x-other",
    ];
    for answer in bad {
        assert!(negotiate(offer, Some(answer)).is_err(), "{answer}");
    }
    // A server may answer only what the client offered.
    assert!(negotiate(None, Some("permessage-deflate")).is_err());
    assert!(negotiate(Some("x-other"), Some("permessage-deflate")).is_err());
}

#[test]
fn unknown_extension_relays_inside_a_named_loss() {
    // The 101 is the RFC 6455 section 1.3 example: this key and its accept value.
    const UPGRADE: &[u8] = b"GET /v1/responses HTTP/1.1\r\nHost: api.example.com\r\nUpgrade: websocket\r\n\
        Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\
        Sec-WebSocket-Extensions: permessage-deflate\r\n\r\n";
    let up_frames = text(Dir::Up, "up one");
    let down_frames = [text(Dir::Down, "down one"), text(Dir::Down, "down two")].concat();
    for answer in ["x-unknown-extension", "permessage-deflate; x_unknown=1"] {
        assert!(
            negotiate(Some("permessage-deflate"), Some(answer)).is_err(),
            "{answer}"
        );
        // What the proxy does with that refusal: the HTTP/1 parser stops parsing for the
        // connection (5a) and never opens a WebSocket parser. The frames behind the 101, both
        // ways, are relayed (not seen here) and counted held; nothing of them is decoded.
        let (rec, calls) = Recorder::for_tests(ROOM);
        let mut p = Http1::new(&rec, 1000);
        let t = Mono::now();
        p.feed(Dir::Up, t, UPGRADE).expect("the request parses");
        p.feed(Dir::Up, t, &up_frames).expect("the copy is held");
        let mut answered = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\nSec-WebSocket-Extensions: {answer}\r\n\r\n"
        )
        .into_bytes();
        answered.extend_from_slice(&down_frames);
        let stop = p.feed(Dir::Down, t, &answered).expect_err("parsing stops");
        let held = (up_frames.len() + down_frames.len()) as u64;
        assert_eq!(
            stop,
            Stop {
                reason: GapReason::DecodeFailed,
                tail: None,
                held,
                since: stop.since,
                at_close: false,
            },
            "{answer}"
        );
        assert_eq!(Box::new(p).end(t, EndReason::Complete, false, &[]), None);
        let opened = opens(&calls);
        assert!(
            opened
                .iter()
                .all(|(_, kind, _)| !matches!(kind, StreamKind::WsUp | StreamKind::WsDown)),
            "{answer}: no WebSocket stream was opened"
        );
        assert!(!calls.all().iter().any(|c| matches!(c, Call::Ws(..))));
    }

    // Parsing that stops inside an open WebSocket: both streams name the loss and end
    // `decode_error`.
    let (mut ws, rec, calls) = open(ROOM, None);
    ws.stop(rec.now(), None);
    ws.end(rec.now(), EndReason::Complete, 0);
    for dir in [Dir::Up, Dir::Down] {
        let sid = stream(&calls, dir);
        assert_eq!(end_reason(&calls, sid), EndReason::DecodeError);
        assert_eq!(gap_reasons(&calls, sid), [GapReason::DecodeFailed]);
    }
}

// W3: the inflater always uses the full 15-bit window, which decodes any smaller one.

#[test]
fn server_max_window_bits_10_decodes() {
    let deflate = negotiate(
        Some("permessage-deflate; client_max_window_bits"),
        Some("permessage-deflate; server_max_window_bits=10; server_no_context_takeover"),
    )
    .expect("supported")
    .expect("deflate");
    assert!(!deflate.server_takeover);
    // Every back-reference is closer than 1 KiB, so these streams are valid with a 10-bit window.
    let msgs: Vec<String> = (0..3).map(|i| near_repeats(i * 40, 900)).collect();
    let (mut ws, rec, calls) = open(ROOM, Some(deflate));
    for msg in &msgs {
        let body = deflate_sync(
            &mut Compress::new(Compression::default(), false),
            msg.as_bytes(),
        );
        ws.feed(Dir::Down, rec.now(), &compressed(Dir::Down, &body))
            .expect("goes on");
    }
    ws.end(rec.now(), EndReason::Complete, 0);
    let down = stream(&calls, Dir::Down);
    assert_eq!(texts(&calls, down), msgs);
    assert!(gap_reasons(&calls, down).is_empty());
}

// W4: under takeover the inflater is fed whether or not a message's record survives. That is
// structural: `StreamHandle::ws` returns nothing, so the parser cannot know that the guard dropped
// a piece and keeps inflating every message as it arrives. `Recorder::for_tests` has no guard to
// drop with, so this case shows the other half: the history carries from one message to the next,
// and a second message that is only back-references to the first decodes. The guard's own GAP is
// PRD-02's `pipeline_guard.rs`.

#[test]
fn takeover_history_carries_between_messages() {
    let mut c = Compress::new(Compression::default(), false);
    let first = near_repeats(1, 20_000);
    // The second message repeats the first: it is a back-reference into history and only decodes
    // when the first message went through the inflater.
    let second = first.clone();
    let (mut ws, rec, calls) = open(ROOM, Some(both()));
    let body = deflate_sync(&mut c, first.as_bytes());
    ws.feed(Dir::Down, rec.now(), &compressed(Dir::Down, &body))
        .expect("goes on");
    let body = deflate_sync(&mut c, second.as_bytes());
    assert!(
        body.len() < second.len() / 20,
        "a repeat is a few back-references"
    );
    ws.feed(Dir::Down, rec.now(), &compressed(Dir::Down, &body))
        .expect("goes on");
    ws.end(rec.now(), EndReason::Complete, 0);

    let down = stream(&calls, Dir::Down);
    assert_eq!(texts(&calls, down), [first, second]);
    assert!(gap_reasons(&calls, down).is_empty());
}

// W5: a final DEFLATE block, or bytes that do not inflate, end the takeover history for good.

#[test]
fn bfinal_ends_takeover_history() {
    for bad_start in [false, true] {
        let (mut ws, rec, calls) = open(ROOM, Some(both()));
        let first: &[u8] = b"{\"before\":\"the final block\"}";
        let body = if bad_start {
            vec![0xff; 6]
        } else {
            deflate_final(first)
        };
        ws.feed(Dir::Down, rec.now(), &compressed(Dir::Down, &body))
            .expect("goes on");
        // Compressed messages after it cannot be trusted: a loss each, never garbage.
        let mut c = Compress::new(Compression::default(), false);
        for msg in ["{\"after\":1}", "{\"after\":2}"] {
            let body = deflate_sync(&mut c, msg.as_bytes());
            ws.feed(Dir::Down, rec.now(), &compressed(Dir::Down, &body))
                .expect("goes on");
        }
        // A plain message needs no history.
        ws.feed(Dir::Down, rec.now(), &text(Dir::Down, "plain"))
            .expect("goes on");
        ws.end(rec.now(), EndReason::Complete, 0);
        let down = stream(&calls, Dir::Down);
        let got = texts(&calls, down);
        if bad_start {
            assert_eq!(got, ["plain"]);
            assert_eq!(gap_reasons(&calls, down).len(), 3);
        } else {
            assert_eq!(got.len(), 2);
            assert_eq!(
                got[0].as_bytes(),
                first,
                "the final block's own message decodes"
            );
            assert_eq!(got[1], "plain");
            assert_eq!(gap_reasons(&calls, down).len(), 2);
        }
        assert!(
            gap_reasons(&calls, down)
                .iter()
                .all(|g| *g == GapReason::DecodeFailed)
        );
    }

    // Without takeover every message starts fresh, so a final block loses nothing.
    let none = Deflate {
        client_takeover: false,
        server_takeover: false,
    };
    let (mut ws, rec, calls) = open(ROOM, Some(none));
    for msg in ["{\"a\":1}", "{\"b\":2}"] {
        ws.feed(
            Dir::Down,
            rec.now(),
            &compressed(Dir::Down, &deflate_final(msg.as_bytes())),
        )
        .expect("goes on");
    }
    ws.end(rec.now(), EndReason::Complete, 0);
    let down = stream(&calls, Dir::Down);
    assert_eq!(texts(&calls, down), ["{\"a\":1}", "{\"b\":2}"]);
    assert!(gap_reasons(&calls, down).is_empty());
}

// W6: headers and mask keys split across reads, and lengths that are not the shortest form.

#[test]
fn one_byte_reads_decode_the_same() {
    for dir in [Dir::Up, Dir::Down] {
        let mut c = Compress::new(Compression::default(), false);
        let mask = mask_for(dir);
        let m1 = near_repeats(1, 300);
        let m2 = near_repeats(1, 300);
        let mut wire = Vec::new();
        wire.extend(compressed(dir, &deflate_sync(&mut c, m1.as_bytes())));
        // A fragmented message with a ping inside, the 16-bit and 64-bit length forms for short
        // payloads, an empty ping and a close with a code.
        wire.extend(frame(false, false, 0x1, b"Hel", mask));
        wire.extend(frame(true, false, 0x9, b"hi", mask));
        wire.extend(raw_frame(0x80, b"lo", mask, Some(16)));
        wire.extend(raw_frame(0x81, b"wide", mask, Some(64)));
        wire.extend(compressed(dir, &deflate_sync(&mut c, m2.as_bytes())));
        wire.extend(frame(true, false, 0xa, b"", mask));
        wire.extend(frame(true, false, 0x8, &[0x03, 0xe8], mask));

        let run = |split: usize| -> Vec<(u8, Vec<u8>)> {
            let (mut ws, rec, calls) = open(ROOM, Some(both()));
            for chunk in wire.chunks(split) {
                ws.feed(dir, rec.now(), chunk).expect("goes on");
            }
            ws.end(rec.now(), EndReason::Complete, 0);
            assert!(gap_reasons(&calls, stream(&calls, dir)).is_empty());
            messages(&calls, stream(&calls, dir))
        };
        let whole = run(wire.len());
        assert_eq!(whole.len(), 7, "{dir:?}");
        assert_eq!(whole[0].1, m1.as_bytes());
        assert_eq!(whole[1], (0x9, b"hi".to_vec()));
        assert_eq!(whole[2], (0x1, b"Hello".to_vec()));
        assert_eq!(whole[3], (0x1, b"wide".to_vec()));
        assert_eq!(whole[4].1, m2.as_bytes());
        assert_eq!(run(1), whole, "{dir:?}: one byte at a time");
        assert_eq!(run(3), whole, "{dir:?}: three bytes at a time");
    }
}

// W7: a frame that breaks RFC 6455 is a named loss for its message, the next message parses, and
// nothing panics. A length that cannot be trusted stops parsing for the connection.

#[test]
fn violations_are_named_and_relaying_goes_on() {
    let wrong_mask = |dir: Dir| if dir == Dir::Up { None } else { Some(KEY) };
    type Case = (&'static str, Dir, Box<dyn Fn(Dir) -> Vec<u8>>);
    let cases: Vec<Case> = vec![
        (
            "rsv2",
            Dir::Down,
            Box::new(|d| raw_frame(0xa1, b"x", mask_for(d), None)),
        ),
        (
            "rsv3",
            Dir::Up,
            Box::new(|d| raw_frame(0x91, b"x", mask_for(d), None)),
        ),
        (
            "rsv1 without deflate",
            Dir::Down,
            Box::new(|d| frame(true, true, 0x1, b"x", mask_for(d))),
        ),
        (
            "unmasked client frame",
            Dir::Up,
            Box::new(move |d| frame(true, false, 0x1, b"x", wrong_mask(d))),
        ),
        (
            "masked server frame",
            Dir::Down,
            Box::new(move |d| frame(true, false, 0x1, b"x", wrong_mask(d))),
        ),
        (
            "unknown data opcode",
            Dir::Down,
            Box::new(|d| frame(true, false, 0x3, b"x", mask_for(d))),
        ),
        (
            "unknown control opcode",
            Dir::Up,
            Box::new(|d| frame(true, false, 0xb, b"x", mask_for(d))),
        ),
        (
            "control without fin",
            Dir::Down,
            Box::new(|d| frame(false, false, 0x9, b"x", mask_for(d))),
        ),
        (
            "oversized control",
            Dir::Up,
            Box::new(|d| frame(true, false, 0x9, &[7; 126], mask_for(d))),
        ),
        (
            "continuation with nothing open",
            Dir::Down,
            Box::new(|d| frame(true, false, 0x0, b"x", mask_for(d))),
        ),
        (
            "new message inside a message",
            Dir::Up,
            Box::new(|d| {
                let mut v = frame(false, false, 0x1, b"half", mask_for(d));
                v.extend(frame(true, false, 0x1, b"whole", mask_for(d)));
                v
            }),
        ),
    ];
    for (name, dir, bad) in cases {
        let (mut ws, rec, calls) = open(ROOM, None);
        let mut wire = text(dir, "first");
        wire.extend(bad(dir));
        wire.extend(text(dir, "last"));
        // The relay never waits on the parser: every read is taken.
        for chunk in wire.chunks(5) {
            assert!(ws.feed(dir, rec.now(), chunk).is_ok(), "{name}");
        }
        ws.end(rec.now(), EndReason::Complete, 0);
        let sid = stream(&calls, dir);
        let got = texts(&calls, sid);
        assert_eq!(got.first().map(String::as_str), Some("first"), "{name}");
        assert_eq!(got.last().map(String::as_str), Some("last"), "{name}");
        assert!(!got.is_empty() && got.len() <= 3, "{name}");
        let named = gap_reasons(&calls, sid);
        assert!(!named.is_empty(), "{name}: the loss is named");
        assert!(
            named.iter().all(|g| *g == GapReason::DecodeFailed),
            "{name}"
        );
        assert_eq!(end_reason(&calls, sid), EndReason::Complete, "{name}");
    }

    // A 64-bit length with its top bit set: the connection stops parsing, both streams say so.
    for dir in [Dir::Up, Dir::Down] {
        let (mut ws, rec, calls) = open(ROOM, None);
        let mut wire = text(dir, "first");
        let mut head = vec![0x81, 127 | if dir == Dir::Up { 0x80 } else { 0 }];
        head.extend_from_slice(&(1u64 << 63).to_be_bytes());
        wire.extend(head);
        wire.extend(&[0; 4]);
        let stopped = ws.feed(dir, rec.now(), &wire);
        assert!(stopped.is_err(), "{dir:?}");
        ws.end(rec.now(), EndReason::Complete, 0);
        let sid = stream(&calls, dir);
        assert_eq!(texts(&calls, sid), ["first"]);
        assert!(!gap_reasons(&calls, sid).is_empty());
        assert_eq!(end_reason(&calls, sid), EndReason::DecodeError);
    }
}

// W12: a long message is handed over in pieces while it is still arriving.

#[test]
fn a_long_message_is_offered_before_its_last_frame() {
    const FRAMES: usize = 64;
    const MIB: usize = 1 << 20;
    let chunk = vec![b'x'; MIB];
    let (mut ws, rec, calls) = open(ROOM, None);
    let sid = stream(&calls, Dir::Down);
    let pieces = |calls: &Calls| -> (usize, usize) {
        calls.all().iter().fold((0, 0), |(open, fin), c| match c {
            Call::Ws(s, _, _, _, f, _) if *s == sid => {
                if *f {
                    (open, fin + 1)
                } else {
                    (open + 1, fin)
                }
            }
            _ => (open, fin),
        })
    };
    for i in 0..FRAMES {
        let last = i + 1 == FRAMES;
        let opcode = if i == 0 { 0x2 } else { 0x0 };
        ws.feed(
            Dir::Down,
            rec.now(),
            &frame(last, false, opcode, &chunk, None),
        )
        .expect("goes on");
        if i == 0 {
            // The first frame is in: pieces are out, the message is not finished.
            let (open, fin) = pieces(&calls);
            assert!(open > 0 && fin == 0, "pieces before the last frame");
        }
    }
    let (open, fin) = pieces(&calls);
    assert!(open > 0 && fin == 1);
    ws.end(rec.now(), EndReason::Complete, 0);
    let total: usize = messages(&calls, sid).iter().map(|(_, b)| b.len()).sum();
    assert_eq!(total, FRAMES * MIB);
    assert!(gap_reasons(&calls, sid).is_empty());
}

// W13: an end inside a frame or a message is `interrupted`, never `complete`.

#[test]
fn eof_mid_message_is_interrupted() {
    let cuts: [(&str, Vec<u8>); 3] = [
        ("inside a header", vec![0x81]),
        ("inside a payload", {
            let mut v = vec![0x81, 10];
            v.extend_from_slice(b"four");
            v
        }),
        ("between fragments", frame(false, false, 0x1, b"abc", None)),
    ];
    for (name, cut) in cuts {
        let (mut ws, rec, calls) = open(ROOM, None);
        ws.feed(Dir::Down, rec.now(), &text(Dir::Down, "whole"))
            .expect("goes on");
        ws.feed(Dir::Down, rec.now(), &cut).expect("goes on");
        ws.end(rec.now(), EndReason::Complete, 0);
        let (up, down) = (stream(&calls, Dir::Up), stream(&calls, Dir::Down));
        assert_eq!(end_reason(&calls, down), EndReason::Interrupted, "{name}");
        assert_eq!(
            end_reason(&calls, up),
            EndReason::Complete,
            "{name}: the quiet direction"
        );
        assert_eq!(
            texts(&calls, down).first().map(String::as_str),
            Some("whole"),
            "{name}"
        );
    }
    // A prefix that arrived is kept, as a piece without fin.
    let (mut ws, rec, calls) = open(ROOM, None);
    ws.feed(
        Dir::Down,
        rec.now(),
        &frame(false, false, 0x1, b"abc", None),
    )
    .expect("goes on");
    ws.end(rec.now(), EndReason::Complete, 0);
    let down = stream(&calls, Dir::Down);
    let kept: Vec<(u8, bool, Vec<u8>)> = calls
        .all()
        .iter()
        .filter_map(|c| match c {
            Call::Ws(s, _, _, op, fin, b) if *s == down => Some((*op, *fin, b.to_vec())),
            _ => None,
        })
        .collect();
    assert_eq!(kept, [(0x1, false, b"abc".to_vec())]);
}
