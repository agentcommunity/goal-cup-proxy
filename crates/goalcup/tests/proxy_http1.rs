//! PRD-03 acceptance for the passive HTTP/1.1 parser (`proxy_http1.rs`), driven directly with
//! `Recorder::for_tests`: each coding decodes to the plaintext, an unknown coding writes no body
//! byte, chunked trailers reach the END tail, pipelined responses pair with their requests, `HEAD`
//! and `304` have no body, a request target's query and fragment never reach the meta, a body
//! whose window does not fit `room()` is not decoded, a head past 64 KiB is named and parsing goes
//! on, a `101` hands the connection to the WebSocket parser, and a framing fault stops parsing
//! (5a) and counts what it could not record, at a fault and at the close. The relay side of 5a
//! (the `conn` GAP, the `parse_stopped` note) is in `proxy_tunnel.rs`.

mod common;

use common::recorder::{data_of, gaps};
use flate2::write::{GzEncoder, ZlibEncoder};
use flate2::{Compress, Compression, FlushCompress};
use goalcup::pipeline::{Call, Calls, EndReason, GapReason, Mono, Recorder, Sid, StreamKind};
use goalcup::proxy::http1::Http1;
use goalcup::proxy::pump::{Parser, Stop};
use goalcup::proxy::ws::Dir;
use serde_json::{Value, json};
use std::io::Write;

/// Made with `brotli -q 11` from [`text`], not a real capture.
const BR: &[u8] = include_bytes!("proxy_fixtures/anthropic-sse.br");
/// [`text`] flushed every 2 KiB with a 4 MiB window (see `proxy_decode.rs`).
const BR_FLUSHED: &[u8] = include_bytes!("proxy_fixtures/sse-flushed.br");
const CONN: Sid = 1000;
const ROOM: u64 = 1 << 30;

/// 2,000 synthetic SSE events, the plaintext of [`BR`].
fn text() -> Vec<u8> {
    (0..2000)
        .map(|i| {
            format!(
                "event: content_block_delta\ndata: {{\"index\":{i},\"delta\":{{\"text\":\"token {i}\"}}}}\n\n"
            )
        })
        .collect::<String>()
        .into_bytes()
}

/// Feeds `bytes` in irregular splits (every split when `step` is 1).
fn feed(p: &mut Http1, dir: Dir, bytes: &[u8], step: usize) {
    let sizes = [1, 7, 2, 4096, 3, 70_000, 13, 600];
    let (mut rest, mut i) = (bytes, 0);
    while !rest.is_empty() {
        let n = if step == 1 { 1 } else { sizes[i % 8] }.min(rest.len());
        p.feed(dir, Mono::now(), &rest[..n])
            .expect("parsing goes on");
        rest = &rest[n..];
        i += 1;
    }
}

/// One connection: the client bytes, then the server bytes, then a clean close.
fn exchange(room: u64, up: &[u8], down: &[u8], step: usize) -> Calls {
    let (rec, calls) = Recorder::for_tests(room);
    let mut p = Http1::new(&rec, CONN);
    feed(&mut p, Dir::Up, up, step);
    feed(&mut p, Dir::Down, down, step);
    let late = Box::new(p).end(Mono::now(), EndReason::Complete, 0);
    assert_eq!(late, None, "nothing held at the close");
    calls
}

/// `(sid, req, meta)` of every stream of `kind`, in order.
fn opens(calls: &Calls, kind: StreamKind) -> Vec<(Sid, Sid, Value)> {
    calls
        .all()
        .into_iter()
        .filter_map(|c| match c {
            Call::Open(sid, k, conn, req, _, _, meta) if k == kind => {
                assert_eq!(conn, CONN);
                Some((sid, req, meta))
            }
            _ => None,
        })
        .collect()
}

fn end_of(calls: &Calls, sid: Sid) -> (EndReason, Value) {
    calls
        .all()
        .into_iter()
        .find_map(|c| match c {
            Call::End(s, _, reason, _, tail) if s == sid => {
                Some((reason, tail.unwrap_or_default()))
            }
            _ => None,
        })
        .expect("the stream ended")
}

/// The only GAP.
fn one_gap(calls: &Calls) -> [Call; 1] {
    gaps(calls).try_into().expect("one GAP")
}

fn chunked(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for piece in body.chunks(5000) {
        out.extend_from_slice(format!("{:x};ext=1\r\n", piece.len()).as_bytes());
        out.extend_from_slice(piece);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"0\r\n\r\n");
    out
}

/// A `POST` with a small body, answered with `wire` under `coding`, chunked or by length.
fn coded(coding: &str, wire: &[u8], chunk: bool) -> (Vec<u8>, Vec<u8>) {
    let up = b"POST /v1/messages HTTP/1.1\r\nHost: api.example.com\r\nContent-Length: 2\r\n\r\n{}";
    let mut down = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Encoding: {coding}\r\n"
    );
    let body = if chunk {
        down += "Transfer-Encoding: chunked\r\n\r\n";
        chunked(wire)
    } else {
        down += &format!("Content-Length: {}\r\n\r\n", wire.len());
        wire.to_vec()
    };
    (up.to_vec(), [down.into_bytes(), body].concat())
}

#[test]
fn each_coding_decodes_to_the_plaintext() {
    let plain = text();
    let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
    gzip.write_all(&plain).unwrap();
    let mut zlib = ZlibEncoder::new(Vec::new(), Compression::default());
    zlib.write_all(&plain).unwrap();
    let cases: [(&str, Vec<u8>, bool, usize); 5] = [
        ("gzip", gzip.finish().unwrap(), true, 0),
        ("deflate", zlib.finish().unwrap(), false, 0),
        ("br", BR.to_vec(), true, 0),
        ("zstd", zstd::encode_all(&plain[..], 3).unwrap(), false, 0),
        ("br", BR.to_vec(), false, 1),
    ];
    for (coding, wire, chunk, step) in cases {
        let (up, down) = coded(coding, &wire, chunk);
        let calls = exchange(ROOM, &up, &down, step);
        assert!(gaps(&calls).is_empty(), "{coding}");
        let [(req, _, req_meta)] = opens(&calls, StreamKind::HttpReq).try_into().unwrap();
        assert_eq!(data_of(&calls, req), b"{}");
        assert_eq!(req_meta["method"], "POST");
        let [(resp, paired, meta)] = opens(&calls, StreamKind::HttpResp).try_into().unwrap();
        assert_eq!(paired, req);
        assert_eq!(
            (&meta["coding"], &meta["decoded"]),
            (&json!([coding]), &json!(true))
        );
        assert!(
            data_of(&calls, resp) == plain,
            "{coding} decodes to the plaintext"
        );
        // The END tail counts body bytes on the wire, chunk framing included.
        let on_wire = if chunk {
            chunked(&wire).len()
        } else {
            wire.len()
        };
        let (reason, tail) = end_of(&calls, resp);
        assert_eq!(
            (reason, &tail["wire"]),
            (EndReason::Complete, &json!(on_wire))
        );
    }
}

#[test]
fn an_unknown_coding_writes_no_body_byte() {
    let (up, down) = coded("gzip, foo", b"not decodable by us", true);
    let calls = exchange(ROOM, &up, &down, 0);
    let [(resp, _, meta)] = opens(&calls, StreamKind::HttpResp).try_into().unwrap();
    assert_eq!(
        (&meta["coding"], &meta["decoded"]),
        (&Value::Null, &json!(false))
    );
    assert!(data_of(&calls, resp).is_empty());
    let [Call::Gap(sid, reason, _, _, _, lost, tail)] = one_gap(&calls) else {
        panic!("one GAP");
    };
    assert_eq!(
        (sid, reason, lost),
        (resp, GapReason::UnsupportedEncoding, None)
    );
    assert_eq!(tail, Some(json!({"codings": ["gzip", "foo"], "wire": 19})));
}

#[test]
fn chunked_trailers_reach_the_end_tail() {
    let body = b"5\r\nhello\r\n0\r\nX-Checksum: abc\r\nx-n:  1 \r\n\r\n";
    let down = [
        &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n"[..],
        body,
    ]
    .concat();
    let calls = exchange(ROOM, b"GET /t HTTP/1.1\r\n\r\n", &down, 1);
    let [(resp, _, _)] = opens(&calls, StreamKind::HttpResp).try_into().unwrap();
    assert_eq!(data_of(&calls, resp), b"hello");
    let (reason, tail) = end_of(&calls, resp);
    assert_eq!(reason, EndReason::Complete);
    let trailers = json!([["X-Checksum", "abc"], ["x-n", "1"]]);
    assert_eq!(tail, json!({"wire": body.len(), "trailers": trailers}));
}

#[test]
fn pipelined_requests_pair_their_responses() {
    let up = b"GET /a HTTP/1.1\r\n\r\nPOST /b HTTP/1.1\r\nContent-Length: 3\r\n\r\nabcGET /c HTTP/1.1\r\n\r\n";
    let down = b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nAHTTP/1.1 100 Continue\r\n\r\n\
        HTTP/1.1 201 Created\r\nContent-Length: 2\r\n\r\nBBHTTP/1.1 204 No Content\r\n\r\n";
    let calls = exchange(ROOM, up, down, 0);
    let reqs = opens(&calls, StreamKind::HttpReq);
    let resps = opens(&calls, StreamKind::HttpResp);
    let paths: Vec<_> = reqs.iter().map(|(_, _, m)| m["path"].clone()).collect();
    assert_eq!(paths, [json!("/a"), json!("/b"), json!("/c")]);
    let pairs: Vec<_> = resps
        .iter()
        .map(|(_, req, m)| (*req, m["status"].clone()))
        .collect();
    assert_eq!(
        pairs,
        [
            (reqs[0].0, json!(200)),
            (reqs[1].0, json!(201)),
            (reqs[2].0, json!(204))
        ]
    );
    assert_eq!(data_of(&calls, resps[1].0), b"BB");
    let notes: Vec<_> = calls
        .all()
        .into_iter()
        .filter_map(|c| match c {
            Call::Event(ev) => Some(ev),
            _ => None,
        })
        .collect();
    assert_eq!(notes.len(), 1);
    assert_eq!(
        (&notes[0]["what"], &notes[0]["status"], &notes[0]["conn"]),
        (&json!("informational_response"), &json!(100), &json!(CONN))
    );
}

#[test]
fn head_and_304_have_no_body() {
    let up = b"HEAD /h HTTP/1.1\r\n\r\nGET /c HTTP/1.1\r\n\r\nGET /d HTTP/1.1\r\n\r\n";
    let down = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nContent-Encoding: gzip\r\n\r\n\
        HTTP/1.1 304 Not Modified\r\nContent-Length: 10\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
    let calls = exchange(ROOM, up, down, 1);
    let resps = opens(&calls, StreamKind::HttpResp);
    assert_eq!(resps.len(), 3);
    for (sid, _, _) in &resps[..2] {
        assert!(data_of(&calls, *sid).is_empty());
        assert_eq!(
            end_of(&calls, *sid),
            (EndReason::Complete, json!({"wire": 0}))
        );
    }
    assert_eq!(data_of(&calls, resps[2].0), b"ok");
    assert!(gaps(&calls).is_empty());
}

#[test]
fn a_target_query_and_fragment_never_reach_the_meta() {
    let up = b"GET /v1/models?api_key=sk-secret-1#frag HTTP/1.1\r\n\r\n\
        GET https://api.example.com/v1/x?token=abc HTTP/1.1\r\n\r\nGET http://h?q=1 HTTP/1.1\r\n\r\n";
    let calls = exchange(ROOM, up, b"", 0);
    let reqs = opens(&calls, StreamKind::HttpReq);
    let paths: Vec<_> = reqs.iter().map(|(_, _, m)| m["path"].clone()).collect();
    assert_eq!(paths, [json!("/v1/models"), json!("/v1/x"), json!("/")]);
    let metas = serde_json::to_string(&reqs.iter().map(|r| &r.2).collect::<Vec<_>>()).unwrap();
    for secret in ["sk-secret-1", "frag", "token=abc", "q=1"] {
        assert!(!metas.contains(secret));
    }
}

#[test]
fn a_window_over_room_is_not_decoded() {
    let (up, down) = coded("br", BR_FLUSHED, true);
    let calls = exchange(32 * 1024, &up, &down, 0);
    let [(resp, _, _)] = opens(&calls, StreamKind::HttpResp).try_into().unwrap();
    assert!(data_of(&calls, resp).is_empty());
    let [Call::Gap(sid, reason, _, _, _, lost, Some(tail))] = one_gap(&calls) else {
        panic!("one GAP with a tail");
    };
    assert_eq!((sid, reason, lost), (resp, GapReason::MemoryGuard, None));
    assert_eq!(
        (&tail["guard"], &tail["wire"]),
        (&json!("decoder_window"), &json!(BR_FLUSHED.len()))
    );
    assert!(tail["limit"].as_u64().unwrap() <= 32 * 1024);
    assert_eq!(end_of(&calls, resp).0, EndReason::Complete);
}

#[test]
fn a_head_past_64k_is_named_and_parsing_goes_on() {
    let mut up = b"POST /big HTTP/1.1\r\n".to_vec();
    for i in 0..2000 {
        up.extend_from_slice(format!("x-pad-{i:04}: {}\r\n", "a".repeat(50)).as_bytes());
    }
    up.extend_from_slice(b"Content-Length: 5\r\n\r\nhelloGET /next HTTP/1.1\r\n\r\n");
    let calls = exchange(ROOM, &up, b"", 0);
    let reqs = opens(&calls, StreamKind::HttpReq);
    assert_eq!(reqs.len(), 2);
    let (big, _, meta) = &reqs[0];
    let stored = meta["headers"].as_array().unwrap().len();
    assert!(stored > 0 && stored < 2000);
    let [Call::Gap(sid, reason, _, _, _, lost, Some(tail))] = one_gap(&calls) else {
        panic!("one GAP with a tail");
    };
    assert_eq!((sid, reason, lost), (*big, GapReason::MemoryGuard, None));
    assert_eq!(
        (&tail["guard"], &tail["limit"]),
        (&json!("http_head"), &json!(65536))
    );
    assert!(tail["head"].as_u64().unwrap() > 60_000);
    assert_eq!(data_of(&calls, *big), b"hello");
    assert_eq!(end_of(&calls, *big).0, EndReason::Complete);
    assert_eq!(reqs[1].2["path"], "/next");
}

/// One text frame, `rsv1` set when compressed, masked with `mask` (client frames).
fn frame(rsv1: bool, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut out = vec![0x81 | if rsv1 { 0x40 } else { 0 }, payload.len() as u8];
    match mask {
        Some(key) => {
            out[1] |= 0x80;
            out.extend_from_slice(&key);
            out.extend(payload.iter().enumerate().map(|(i, b)| b ^ key[i & 3]));
        }
        None => out.extend_from_slice(payload),
    }
    out
}

/// Raw DEFLATE with a sync flush and the `00 00 ff ff` tail stripped (RFC 7692 section 7.2.1).
fn deflate(c: &mut Compress, msg: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(msg.len() + 64);
    c.compress_vec(msg, &mut out, FlushCompress::Sync).unwrap();
    assert!(out.ends_with(&[0, 0, 0xff, 0xff]));
    out.truncate(out.len() - 4);
    out
}

#[test]
fn a_101_hands_the_copied_frames_to_the_websocket_parser() {
    let (mut cup, mut cdown) = (
        Compress::new(Compression::default(), false),
        Compress::new(Compression::default(), false),
    );
    let key = Some([1, 2, 3, 4]);
    // The RFC 6455 section 1.3 example key and its accept value.
    let mut up = b"GET /v1/responses?session=s1 HTTP/1.1\r\nHost: api.example.com\r\nUpgrade: websocket\r\n\
        Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\
        Sec-WebSocket-Extensions: permessage-deflate; client_max_window_bits\r\n\r\n"
        .to_vec();
    up.extend(frame(true, &deflate(&mut cup, b"first up"), key));
    let mut down = b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
        Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\nSec-WebSocket-Extensions: permessage-deflate\r\n\r\n"
        .to_vec();
    down.extend(frame(true, &deflate(&mut cdown, b"down one"), None));

    let (rec, calls) = Recorder::for_tests(ROOM);
    let mut p = Http1::new(&rec, CONN);
    feed(&mut p, Dir::Up, &up, 0);
    feed(&mut p, Dir::Down, &down, 1);
    feed(
        &mut p,
        Dir::Up,
        &frame(true, &deflate(&mut cup, b"second up"), key),
        1,
    );
    assert_eq!(Box::new(p).end(Mono::now(), EndReason::Complete, 0), None);

    assert!(gaps(&calls).is_empty());
    let [(req, _, _)] = opens(&calls, StreamKind::HttpReq).try_into().unwrap();
    assert_eq!(end_of(&calls, req).0, EndReason::Complete);
    assert!(opens(&calls, StreamKind::HttpResp).is_empty());
    let [(ws_up, up_req, meta)] = opens(&calls, StreamKind::WsUp).try_into().unwrap();
    let [(ws_down, down_req, _)] = opens(&calls, StreamKind::WsDown).try_into().unwrap();
    assert_eq!(
        (up_req, down_req, &meta["deflate"]),
        (req, req, &json!(true))
    );
    let messages = |sid: Sid| -> Vec<Vec<u8>> {
        calls
            .all()
            .into_iter()
            .filter_map(|c| match c {
                Call::Ws(s, _, _, 0x1, true, bytes) if s == sid => Some(bytes.to_vec()),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        messages(ws_up),
        [b"first up".to_vec(), b"second up".to_vec()]
    );
    assert_eq!(messages(ws_down), [b"down one".to_vec()]);
}

#[test]
fn a_framing_fault_stops_parsing_for_the_connection() {
    let (rec, calls) = Recorder::for_tests(ROOM);
    let mut p = Http1::new(&rec, CONN);
    let t = Mono::now();
    let head = b"POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n";
    p.feed(Dir::Up, t, head).expect("a valid prefix parses");
    let stop = p.feed(Dir::Up, t, b"zz\r\n").unwrap_err();
    assert_eq!((stop.reason, stop.held), (GapReason::DecodeFailed, 4));
    let [(req, _, _)] = opens(&calls, StreamKind::HttpReq).try_into().unwrap();
    assert_eq!(data_of(&calls, req), b"abc");
    let [Call::Gap(sid, reason, ..)] = one_gap(&calls) else {
        panic!("one GAP");
    };
    assert_eq!((sid, reason), (req, GapReason::DecodeFailed));
    assert_eq!(end_of(&calls, req).0, EndReason::DecodeError);
    let later = b"GET /later HTTP/1.1\r\n\r\n";
    assert_eq!(
        p.feed(Dir::Up, t, later).unwrap_err().held,
        later.len() as u64
    );
    assert_eq!(Box::new(p).end(t, EndReason::Complete, 0), None);
    assert_eq!(opens(&calls, StreamKind::HttpReq).len(), 1);
    assert_eq!(gaps(&calls).len(), 1);
}

/// 5a counts the rest of the faulting chunk: a bad head followed, in the same chunk, by a whole
/// request is held whole.
#[test]
fn a_fault_counts_the_rest_of_its_chunk() {
    let (rec, calls) = Recorder::for_tests(ROOM);
    let mut p = Http1::new(&rec, CONN);
    let t = Mono::now();
    p.feed(Dir::Up, t, b"GET /a HTTP/1.1\r\n\r\n").unwrap();
    let mut chunk = b"BAD\r\n\r\nPOST /c HTTP/1.1\r\nContent-Length: 8000\r\n\r\n".to_vec();
    chunk.extend_from_slice(&[b'x'; 8000]);
    let stop = p.feed(Dir::Up, t, &chunk).unwrap_err();
    assert_eq!(
        (stop.reason, stop.held),
        (GapReason::DecodeFailed, chunk.len() as u64)
    );
    assert_eq!(opens(&calls, StreamKind::HttpReq).len(), 1);
}

/// 5a under the `upgrade_copy` guard: the GAP names the guard, and the copy, relayed but never
/// recorded, counts as held from the upgrade request on.
#[test]
fn the_upgrade_copy_guard_counts_the_copy() {
    let (rec, calls) = Recorder::for_tests(ROOM);
    let mut p = Http1::new(&rec, CONN);
    let t0 = Mono::now();
    let up = b"GET /ws HTTP/1.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
        Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";
    p.feed(Dir::Up, t0, up).unwrap();
    let piece = [7u8; 16 * 1024];
    let stop = (0..65)
        .find_map(|_| p.feed(Dir::Up, Mono::now(), &piece).err())
        .expect("the guard stops parsing");
    let tail = json!({"guard": "upgrade_copy", "limit": 1 << 20});
    assert_eq!(
        stop,
        Stop {
            reason: GapReason::MemoryGuard,
            tail: Some(tail),
            held: 65 * 16 * 1024,
            since: t0
        }
    );
    let [(req, _, _)] = opens(&calls, StreamKind::HttpReq).try_into().unwrap();
    assert_eq!(end_of(&calls, req).0, EndReason::Complete);
}

/// Bytes the close leaves unrecorded are a stop at the close: heads cut short on both sides, and
/// the copy of an upgrade that was never answered.
#[test]
fn bytes_held_at_the_close_are_named() {
    let (rec, calls) = Recorder::for_tests(ROOM);
    let mut p = Http1::new(&rec, CONN);
    let t = Mono::now();
    p.feed(Dir::Up, t, b"GET /a HTTP/1.1\r\n\r\n").unwrap();
    p.feed(
        Dir::Down,
        t,
        b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
    )
    .unwrap();
    let (resp, req) = (
        &b"HTTP/1.1 200 OK\r\nContent-Type: te"[..],
        &b"POST /v1/m HTTP/1.1\r\nCont"[..],
    );
    p.feed(Dir::Down, t, resp).unwrap();
    p.feed(Dir::Up, t, req).unwrap();
    let late = Box::new(p)
        .end(Mono::now(), EndReason::Complete, 0)
        .expect("a stop");
    let held = (resp.len() + req.len()) as u64;
    assert_eq!(
        (late.reason, late.held, late.since),
        (GapReason::DecodeFailed, held, t)
    );
    assert_eq!(opens(&calls, StreamKind::HttpReq).len(), 1);

    let (rec, _calls) = Recorder::for_tests(ROOM);
    let mut p = Http1::new(&rec, CONN);
    let up = b"GET /ws HTTP/1.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
        Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";
    p.feed(Dir::Up, t, up).unwrap();
    p.feed(Dir::Up, Mono::now(), &[0x81; 790]).unwrap();
    let late = Box::new(p)
        .end(Mono::now(), EndReason::Complete, 0)
        .expect("a stop");
    assert_eq!((late.held, late.since), (790, t));
}

/// A panic (rule 4): the open stream gets its own GAP with the panic tail and ends decode_error.
#[test]
fn a_panic_gives_the_open_stream_its_gap() {
    let (rec, calls) = Recorder::for_tests(ROOM);
    let mut p = Http1::new(&rec, CONN);
    let t = Mono::now();
    p.feed(
        Dir::Up,
        t,
        b"POST /x HTTP/1.1\r\nContent-Length: 10\r\n\r\nabc",
    )
    .unwrap();
    let stop = Box::new(p).panicked(t);
    assert_eq!((stop.reason, stop.held), (GapReason::DecodeFailed, 0));
    let [(req, _, _)] = opens(&calls, StreamKind::HttpReq).try_into().unwrap();
    let [Call::Gap(sid, GapReason::DecodeFailed, _, _, _, _, tail)] = one_gap(&calls) else {
        panic!("one GAP decode_failed");
    };
    assert_eq!((sid, tail), (req, Some(json!({"panic": true}))));
    assert_eq!(end_of(&calls, req).0, EndReason::DecodeError);
}

/// Past the head guard, a long start line keeps its method and path (GAP `http_head`), a chunk
/// extension of any length is skipped, and a framing field that keeps growing stops parsing.
#[test]
fn long_lines_past_the_guard() {
    let mut up = format!("GET /long?q={} HTTP/1.1\r\n\r\n", "a".repeat(100_000)).into_bytes();
    up.extend_from_slice(b"POST /next HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n");
    up.extend_from_slice(format!("2;ext={}\r\nok\r\n0\r\n\r\n", "e".repeat(10_000)).as_bytes());
    let calls = exchange(ROOM, &up, b"", 0);
    let reqs = opens(&calls, StreamKind::HttpReq);
    let meta = |i: usize| (reqs[i].2["path"].clone(), reqs[i].2["ver"].clone());
    assert_eq!(meta(0), (json!("/long"), Value::Null));
    assert_eq!(meta(1), (json!("/next"), json!("1.1")));
    assert_eq!(data_of(&calls, reqs[1].0), b"ok");
    let [Call::Gap(sid, GapReason::MemoryGuard, _, _, _, _, Some(tail))] = one_gap(&calls) else {
        panic!("one GAP memory_guard");
    };
    assert_eq!((sid, &tail["guard"]), (reqs[0].0, &json!("http_head")));

    let (rec, _calls) = Recorder::for_tests(ROOM);
    let mut p = Http1::new(&rec, CONN);
    let mut head = b"GET / HTTP/1.1\r\n".to_vec();
    for i in 0..80 {
        head.extend(format!("x-pad-{i:02}: {}\r\n", "p".repeat(900)).bytes());
    }
    head.extend_from_slice(b"Upgrade: ");
    p.feed(Dir::Up, Mono::now(), &head).unwrap();
    let grow = |p: &mut Http1| p.feed(Dir::Up, Mono::now(), &[b'w'; 1024]);
    let stop = (0..8)
        .find_map(|_| grow(&mut p).err())
        .expect("parsing stops");
    assert_eq!(stop.reason, GapReason::DecodeFailed);
}
