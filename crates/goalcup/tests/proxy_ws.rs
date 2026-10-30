//! PRD-03 acceptance for the passive WebSocket parser (`proxy_ws.rs`): the Codex-shaped fixture
//! inflates to its messages with and without context takeover, a fragmented message stays one
//! message around a control frame, control frames are recorded, client masking is removed, and a
//! close code reaches the END tail.

use bytes::Bytes;
use flate2::{Compress, Compression, FlushCompress};
use goalcup::pipeline::{Call, Calls, EndReason, Recorder, Sid, StreamKind};
use goalcup::proxy::ws::{Deflate, Dir, WsConn, negotiate};
use std::hash::BuildHasher;

const FIXTURE: &str = include_str!("proxy_fixtures/codex-ws.jsonl");
const PIECE: usize = 32 * 1024;

struct Rng(u64);

impl Rng {
    fn new() -> Rng {
        Rng(std::collections::hash_map::RandomState::new().hash_one(0u8) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn key(&mut self) -> [u8; 4] {
        (self.next() as u32).to_be_bytes()
    }
}

/// One frame on the wire; `mask` set for client frames.
fn frame(fin: bool, rsv1: bool, opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut out = vec![(u8::from(fin) << 7) | (u8::from(rsv1) << 6) | opcode];
    let bit = if mask.is_some() { 0x80 } else { 0 };
    match payload.len() {
        n if n < 126 => out.push(bit | n as u8),
        n if n <= 0xffff => {
            out.push(bit | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
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

/// Raw DEFLATE with a sync flush and the `00 00 ff ff` tail stripped (RFC 7692 section 7.2.1).
fn deflate(c: &mut Compress, msg: &[u8]) -> Vec<u8> {
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

/// The fixture's messages for one direction: `response.create` goes up, every event comes down.
fn fixture(dir: Dir) -> Vec<String> {
    FIXTURE
        .lines()
        .filter(|line| !line.is_empty())
        .filter(|line| line.starts_with(r#"{"type":"response.create","#) == (dir == Dir::Up))
        .map(str::to_string)
        .collect()
}

/// Every message of one direction compressed (one compressor for the direction under takeover,
/// a fresh one per message without), cut into one to three frames.
fn wire(msgs: &[String], takeover: bool, client: bool, rng: &mut Rng) -> Vec<u8> {
    let mut shared = Compress::new(Compression::default(), false);
    let mut out = Vec::new();
    for msg in msgs {
        let mut fresh = Compress::new(Compression::default(), false);
        let c = if takeover { &mut shared } else { &mut fresh };
        let body = deflate(c, msg.as_bytes());
        let cuts = 1 + rng.below(3) as usize;
        let size = body.len().div_ceil(cuts).max(1);
        let parts: Vec<&[u8]> = body.chunks(size).collect();
        for (i, part) in parts.iter().enumerate() {
            let mask = client.then(|| rng.key());
            let opcode = if i == 0 { 0x1 } else { 0x0 };
            out.extend(frame(i + 1 == parts.len(), i == 0, opcode, part, mask));
        }
    }
    out
}

/// Feeds one direction's bytes in irregular splits.
fn feed(ws: &mut WsConn, rec: &Recorder, dir: Dir, bytes: &[u8], rng: &mut Rng) {
    let mut rest = bytes;
    while !rest.is_empty() {
        let n = (1 + rng.below(700) as usize).min(rest.len());
        ws.feed(dir, rec.now(), &rest[..n])
            .expect("parsing goes on");
        rest = &rest[n..];
    }
}

/// The two streams opened, `(ws_up, ws_down)`; exactly one of each.
fn streams(calls: &[Call]) -> (Sid, Sid) {
    let opened = |kind: StreamKind| -> Vec<Sid> {
        calls
            .iter()
            .filter_map(|c| match c {
                Call::Open(sid, k, ..) if *k == kind => Some(*sid),
                _ => None,
            })
            .collect()
    };
    let (up, down) = (opened(StreamKind::WsUp), opened(StreamKind::WsDown));
    assert_eq!((up.len(), down.len()), (1, 1), "one ws_up and one ws_down");
    (up[0], down[0])
}

/// The messages of one stream, `(opcode, bytes)` in the order they end. Control pieces are whole
/// messages; data pieces join up to `fin`. Offsets must run on with no hole, every piece at most
/// 32 KiB.
fn messages(calls: &[Call], sid: Sid) -> Vec<(u8, Vec<u8>)> {
    let (mut off, mut data, mut out) = (0u64, Vec::new(), Vec::new());
    for call in calls {
        let Call::Ws(s, _, at, opcode, fin, bytes) = call else {
            continue;
        };
        if *s != sid {
            continue;
        }
        assert_eq!(*at, off, "offsets run over the inflated bytes");
        assert!(bytes.len() <= PIECE, "a piece is at most 32 KiB");
        off += bytes.len() as u64;
        if opcode & 0x8 != 0 {
            assert!(fin, "a control message is one piece");
            out.push((*opcode, bytes.to_vec()));
            continue;
        }
        data.extend_from_slice(bytes);
        if *fin {
            out.push((*opcode, std::mem::take(&mut data)));
        }
    }
    out
}

fn no_gaps(calls: &[Call]) {
    assert!(
        !calls.iter().any(|c| matches!(c, Call::Gap(..))),
        "no GAP on a clean connection"
    );
}

fn end_of(calls: &[Call], sid: Sid) -> (EndReason, Option<serde_json::Value>) {
    calls
        .iter()
        .find_map(|c| match c {
            Call::End(s, _, reason, _, tail) if *s == sid => Some((*reason, tail.clone())),
            _ => None,
        })
        .expect("the stream ended")
}

fn run_fixture(deflate: Deflate, takeover: bool) {
    let mut rng = Rng::new();
    let (rec, calls): (Recorder, Calls) = Recorder::for_tests(1 << 30);
    let (up_msgs, down_msgs) = (fixture(Dir::Up), fixture(Dir::Down));
    assert!(up_msgs.len() >= 2 && down_msgs.len() >= 3);
    let up_wire = wire(&up_msgs, takeover, true, &mut rng);
    let down_wire = wire(&down_msgs, takeover, false, &mut rng);

    let mut ws = WsConn::open(&rec, 1, 2, Some(deflate), rec.now());
    // Interleave the two directions in irregular rounds.
    let (mut u, mut d) = (&up_wire[..], &down_wire[..]);
    while !u.is_empty() || !d.is_empty() {
        let n = (1 + rng.below(900) as usize).min(u.len());
        feed(&mut ws, &rec, Dir::Up, &u[..n], &mut rng);
        u = &u[n..];
        let n = (1 + rng.below(900) as usize).min(d.len());
        feed(&mut ws, &rec, Dir::Down, &d[..n], &mut rng);
        d = &d[n..];
    }
    ws.end(rec.now(), EndReason::Complete, 0);

    let all = calls.all();
    no_gaps(&all);
    let (up, down) = streams(&all);
    for c in &all {
        if let Call::Open(_, _, _, _, _, _, meta) = c {
            assert_eq!(meta["deflate"], true);
        }
    }
    let got_up = messages(&all, up);
    let got_down = messages(&all, down);
    let text = |m: &[(u8, Vec<u8>)]| -> Vec<String> {
        m.iter()
            .map(|(op, b)| {
                assert_eq!(*op, 0x1, "text messages");
                String::from_utf8(b.clone()).expect("utf-8")
            })
            .collect()
    };
    assert_eq!(text(&got_up), up_msgs);
    assert_eq!(text(&got_down), down_msgs);
    for msg in text(&got_up) {
        serde_json::from_str::<serde_json::Value>(&msg).expect("every ws_up text message is JSON");
    }
    assert_eq!(end_of(&all, up).0, EndReason::Complete);
    assert_eq!(end_of(&all, down).0, EndReason::Complete);
}

#[test]
fn codex_fixture_inflates_with_context_takeover() {
    let deflate = negotiate(
        Some("permessage-deflate; client_max_window_bits"),
        Some("permessage-deflate"),
    )
    .expect("supported")
    .expect("deflate");
    assert!(deflate.client_takeover && deflate.server_takeover);
    run_fixture(deflate, true);
}

#[test]
fn codex_fixture_inflates_without_context_takeover() {
    let deflate = negotiate(
        Some("permessage-deflate; client_no_context_takeover"),
        Some("permessage-deflate; server_no_context_takeover; client_no_context_takeover"),
    )
    .expect("supported")
    .expect("deflate");
    assert!(!deflate.client_takeover && !deflate.server_takeover);
    run_fixture(deflate, false);
}

#[test]
fn fragmented_message_stays_whole_around_a_ping() {
    let mut rng = Rng::new();
    let (rec, calls) = Recorder::for_tests(1 << 30);
    let mut ws = WsConn::open(&rec, 1, 2, None, rec.now());
    let mut bytes = frame(false, false, 0x1, b"Hel", Some(rng.key()));
    bytes.extend(frame(true, false, 0x9, b"are you there", Some(rng.key())));
    bytes.extend(frame(false, false, 0x0, b"lo, ", Some(rng.key())));
    bytes.extend(frame(true, false, 0x0, b"world", Some(rng.key())));
    for b in bytes.chunks(3) {
        ws.feed(Dir::Up, rec.now(), b).expect("parsing goes on");
    }
    ws.end(rec.now(), EndReason::Complete, 0);

    let all = calls.all();
    no_gaps(&all);
    let (up, _) = streams(&all);
    let text: Vec<(bool, Bytes)> = all
        .iter()
        .filter_map(|c| match c {
            Call::Ws(s, _, _, 0x1, fin, b) if *s == up => Some((*fin, b.clone())),
            _ => None,
        })
        .collect();
    let joined: Vec<u8> = text.iter().flat_map(|(_, b)| b.to_vec()).collect();
    assert_eq!(joined, b"Hello, world");
    let fins: Vec<bool> = text.iter().map(|(fin, _)| *fin).collect();
    assert_eq!(fins.iter().filter(|f| **f).count(), 1);
    assert_eq!(fins.last(), Some(&true), "fin only on the last piece");
    let msgs = messages(&all, up);
    assert!(msgs.contains(&(0x9, b"are you there".to_vec())));
    assert!(msgs.contains(&(0x1, b"Hello, world".to_vec())));
    assert_eq!(msgs.len(), 2);
}

#[test]
fn rfc6455_examples_record_unmasked_payloads() {
    let (rec, calls) = Recorder::for_tests(1 << 30);
    let mut ws = WsConn::open(&rec, 1, 2, None, rec.now());
    // RFC 6455 section 5.7, byte for byte.
    let up: Vec<u8> = [
        // A single-frame masked text message: "Hello".
        &[
            0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58,
        ][..],
        // A masked pong: "Hello".
        &[
            0x8a, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58,
        ][..],
    ]
    .concat();
    let mut down: Vec<u8> = [
        // A single-frame unmasked text message.
        &[0x81, 0x05, 0x48, 0x65, 0x6c, 0x6c, 0x6f][..],
        // A fragmented unmasked text message.
        &[0x01, 0x03, 0x48, 0x65, 0x6c][..],
        &[0x80, 0x02, 0x6c, 0x6f][..],
        // An unmasked ping.
        &[0x89, 0x05, 0x48, 0x65, 0x6c, 0x6c, 0x6f][..],
        // 256 bytes binary in a single unmasked frame (16-bit length).
        &[0x82, 0x7e, 0x01, 0x00][..],
    ]
    .concat();
    let small: Vec<u8> = (0..256u32).map(|i| i as u8).collect();
    down.extend_from_slice(&small);
    // 64 KiB binary in a single unmasked frame (64-bit length).
    let big: Vec<u8> = (0..65536u32).map(|i| (i * 7) as u8).collect();
    down.extend_from_slice(&[0x82, 0x7f, 0, 0, 0, 0, 0, 1, 0, 0]);
    down.extend_from_slice(&big);
    ws.feed(Dir::Up, rec.now(), &up).expect("parsing goes on");
    ws.feed(Dir::Down, rec.now(), &down)
        .expect("parsing goes on");
    ws.end(rec.now(), EndReason::Complete, 0);

    let all = calls.all();
    no_gaps(&all);
    let (u, d) = streams(&all);
    let hello = b"Hello".to_vec();
    assert_eq!(
        messages(&all, u),
        vec![(0x1, hello.clone()), (0xa, hello.clone())]
    );
    assert_eq!(
        messages(&all, d),
        vec![
            (0x1, hello.clone()),
            (0x1, hello.clone()),
            (0x9, hello),
            (0x2, small),
            (0x2, big),
        ]
    );
}

#[test]
fn close_code_reaches_the_end_tail() {
    let mut rng = Rng::new();
    let (rec, calls) = Recorder::for_tests(1 << 30);
    let mut ws = WsConn::open(&rec, 1, 2, None, rec.now());
    let mut up_close = 1000u16.to_be_bytes().to_vec();
    up_close.extend_from_slice(b"done");
    ws.feed(
        Dir::Up,
        rec.now(),
        &frame(true, false, 0x8, &up_close, Some(rng.key())),
    )
    .expect("parsing goes on");
    ws.feed(
        Dir::Down,
        rec.now(),
        &frame(true, false, 0x8, &1001u16.to_be_bytes(), None),
    )
    .expect("parsing goes on");
    ws.end(rec.now(), EndReason::Complete, 7);

    let all = calls.all();
    no_gaps(&all);
    let (u, d) = streams(&all);
    assert_eq!(messages(&all, u), vec![(0x8, up_close)]);
    let (reason, tail) = end_of(&all, u);
    assert_eq!(reason, EndReason::Complete);
    let tail = tail.expect("tail");
    assert_eq!(tail["close_code"], 1000);
    assert_eq!(tail["unsent"], 7);
    let (_, tail) = end_of(&all, d);
    let tail = tail.expect("tail");
    assert_eq!(tail["close_code"], 1001);
    assert!(tail.get("unsent").is_none());
}
