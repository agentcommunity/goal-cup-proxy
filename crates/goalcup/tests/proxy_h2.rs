//! PRD-03 section 6, the passive HTTP/2 parser, driven directly with `Recorder::for_tests` as
//! `proxy_http1.rs` drives `Http1`: streams with pseudo-headers and the query dropped, the HPACK
//! dynamic table and size updates (RFC 7541 appendix C), CONTINUATION, padding and priority,
//! trailers, `RST_STREAM`, `GOAWAY`, body codings, an extended CONNECT stream, and the stops for
//! `PUSH_PROMISE`, an HPACK error and the `hpack_table` guard. The relay side of 5a is in
//! `proxy_tunnel.rs`.

mod common;

use common::recorder::{data_of, gaps};
use flate2::Compression;
use flate2::write::GzEncoder;
use goalcup::pipeline::{Call, Calls, EndReason, GapReason, Mono, Recorder, Sid, StreamKind};
use goalcup::proxy::h2::H2;
use goalcup::proxy::pump::{Parser, Stop};
use goalcup::proxy::ws::Dir;
use serde_json::{Value, json};
use std::io::Write;

const CONN: Sid = 2000;
const ROOM: u64 = 1 << 30;
const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
// Frame types and flags (RFC 9113 section 6).
const DATA: u8 = 0;
const HEADERS: u8 = 1;
const PRIORITY: u8 = 2;
const RST: u8 = 3;
const SETTINGS: u8 = 4;
const PUSH: u8 = 5;
const PING: u8 = 6;
const GOAWAY: u8 = 7;
const WINDOW: u8 = 8;
const CONT: u8 = 9;
const END_STREAM: u8 = 0x1;
const END_HEADERS: u8 = 0x4;
const PADDED: u8 = 0x8;
const PRIO: u8 = 0x20;

/// One frame: the 9-byte header, then the payload.
fn frame(ty: u8, flags: u8, sid: u32, payload: &[u8]) -> Vec<u8> {
    let n = (payload.len() as u32).to_be_bytes();
    [&n[1..], &[ty, flags], &sid.to_be_bytes(), payload].concat()
}

/// A literal header field without indexing, new name (RFC 7541 section 6.2.2).
fn lit(name: &str, value: &str) -> Vec<u8> {
    let (n, v) = (name.as_bytes(), value.as_bytes());
    [&[0, n.len() as u8][..], n, &[v.len() as u8], v].concat()
}

fn block(fields: &[(&str, &str)]) -> Vec<u8> {
    fields.iter().flat_map(|(n, v)| lit(n, v)).collect()
}

/// A complete header block in one HEADERS frame.
fn head(sid: u32, end: bool, fields: &[(&str, &str)]) -> Vec<u8> {
    let flags = END_HEADERS | if end { END_STREAM } else { 0 };
    frame(HEADERS, flags, sid, &block(fields))
}

/// A request head; a GET ends its stream.
fn req(sid: u32, method: &str, path: &str) -> Vec<u8> {
    let f = [(":method", method), (":scheme", "https"), (":authority", "api.example.com")];
    head(sid, method == "GET", &[&f[..], &[(":path", path)]].concat())
}

fn ok(sid: u32, extra: &[(&str, &str)]) -> Vec<u8> {
    head(sid, false, &[&[(":status", "200")][..], extra].concat())
}

fn body(sid: u32, end: bool, bytes: &[u8]) -> Vec<u8> {
    frame(DATA, if end { END_STREAM } else { 0 }, sid, bytes)
}

fn up(frames: &[Vec<u8>]) -> (Dir, Vec<u8>) {
    let settings = frame(SETTINGS, 0, 0, &[]);
    (Dir::Up, [&[PREFACE.to_vec(), settings][..], frames].concat().concat())
}

/// The server's frames, after its SETTINGS.
fn down(frames: &[Vec<u8>]) -> (Dir, Vec<u8>) {
    let settings = frame(SETTINGS, 0, 0, &[]);
    (Dir::Down, [&[settings][..], frames].concat().concat())
}

/// Feeds each step in irregular splits until a stop, then closes. The stop, if any, is returned.
fn run(steps: &[(Dir, Vec<u8>)]) -> (Calls, Option<Stop>) {
    let (rec, calls) = Recorder::for_tests(ROOM);
    let mut p = H2::new(&rec, CONN);
    let sizes = [1, 7, 2, 4096, 3, 13, 600, 9];
    let mut stop = None;
    'all: for (dir, bytes) in steps {
        let (mut rest, mut i) = (&bytes[..], 0);
        while !rest.is_empty() {
            let n = sizes[i % sizes.len()].min(rest.len());
            if let Err(s) = p.feed(*dir, Mono::now(), &rest[..n]) {
                stop = Some(s);
                break 'all;
            }
            rest = &rest[n..];
            i += 1;
        }
    }
    let late = Box::new(p).end(Mono::now(), EndReason::Complete, false, &[]);
    assert_eq!(late, None, "nothing held at the close");
    (calls, stop)
}

/// `(sid, req, meta)` of every stream of `kind`, in order.
fn opens(calls: &Calls, kind: StreamKind) -> Vec<(Sid, Sid, Value)> {
    let all = calls.all().into_iter();
    all.filter_map(|c| match c {
        Call::Open(sid, k, conn, req, _, _, meta) if k == kind => {
            assert_eq!(conn, CONN);
            Some((sid, req, meta))
        }
        _ => None,
    })
    .collect()
}

fn end_of(calls: &Calls, sid: Sid) -> (EndReason, Value) {
    let all = calls.all().into_iter();
    all.find_map(|c| match c {
        Call::End(s, _, reason, _, tail) if s == sid => Some((reason, tail.unwrap_or_default())),
        _ => None,
    })
    .expect("the stream ended")
}

/// `(reason, lost)` of each GAP on `sid`.
fn gaps_on(calls: &Calls, sid: Sid) -> Vec<(GapReason, Option<u64>)> {
    let all = gaps(calls).into_iter();
    all.filter_map(|c| match c {
        Call::Gap(s, reason, _, _, _, lost, _) if s == sid => Some((reason, lost)),
        _ => None,
    })
    .collect()
}

fn pairs(fields: &[(&str, &str)]) -> Value {
    json!(fields.iter().map(|(n, v)| [n, v]).collect::<Vec<_>>())
}

#[test]
fn a_get_and_its_response_with_data() {
    let steps = [
        up(&[req(1, "GET", "/v1/models?page=2")]),
        down(&[
            ok(1, &[("content-type", "text/plain")]),
            body(1, false, b"hello "),
            frame(WINDOW, 0, 0, &[0, 0, 1, 0]),
            body(1, true, b"world"),
        ]),
    ];
    let (calls, stop) = run(&steps);
    assert_eq!(stop, None);
    assert!(gaps(&calls).is_empty());
    let [(req, _, meta)] = opens(&calls, StreamKind::HttpReq).try_into().unwrap();
    assert_eq!((&meta["ver"], &meta["h2"]), (&json!("2"), &json!(1)));
    assert_eq!(meta["path"], "/v1/models");
    let wire = [(":method", "GET"), (":scheme", "https"), (":authority", "api.example.com")];
    assert_eq!(meta["headers"], pairs(&[&wire[..], &[(":path", "/v1/models")]].concat()));
    let [(resp, paired, meta)] = opens(&calls, StreamKind::HttpResp).try_into().unwrap();
    assert_eq!((paired, &meta["ver"], &meta["h2"]), (req, &json!("2"), &json!(1)));
    assert_eq!(meta["headers"], pairs(&[(":status", "200"), ("content-type", "text/plain")]));
    assert_eq!(data_of(&calls, resp), b"hello world");
    let (reason, tail) = end_of(&calls, resp);
    assert_eq!((reason, &tail["wire"]), (EndReason::Complete, &json!(11)));
}

/// RFC 7541 C.3 (no Huffman) and C.4 (Huffman): three requests on one connection share the
/// dynamic table; a fourth and fifth add size updates (to 0, then back to 4096).
#[test]
fn hpack_dynamic_table_and_size_updates() {
    let c3 = [
        [&[0x82, 0x86, 0x84, 0x41, 0x0f][..], b"www.example.com"].concat(),
        [&[0x82, 0x86, 0x84, 0xbe, 0x58, 0x08][..], b"no-cache"].concat(),
        [&[0x82, 0x87, 0x85, 0xbf, 0x40, 0x0a][..], b"custom-key\x0ccustom-value"].concat(),
    ];
    let c4: [&[u8]; 3] = [
        &[
            0x82, 0x86, 0x84, 0x41, 0x8c, 0xf1, 0xe3, 0xc2, 0xe5, 0xf2, 0x3a, 0x6b, 0xa0, 0xab,
            0x90, 0xf4, 0xff,
        ],
        &[0x82, 0x86, 0x84, 0xbe, 0x58, 0x86, 0xa8, 0xeb, 0x10, 0x64, 0x9c, 0xbf],
        &[
            0x82, 0x87, 0x85, 0xbf, 0x40, 0x88, 0x25, 0xa8, 0x49, 0xe9, 0x5b, 0xa9, 0x7d, 0x7f,
            0x89, 0x25, 0xa8, 0x49, 0xe9, 0x5b, 0xb8, 0xe8, 0xb4, 0xbf,
        ],
    ];
    let base = [(":method", "GET"), (":scheme", "http"), (":path", "/")];
    let site = (":authority", "www.example.com");
    let mut want = vec![
        pairs(&[base[0], base[1], base[2], site]),
        pairs(&[base[0], base[1], base[2], site, ("cache-control", "no-cache")]),
        pairs(&[
            base[0],
            (":scheme", "https"),
            (":path", "/index.html"),
            site,
            ("custom-key", "custom-value"),
        ]),
    ];
    let other = (":authority", "b.example.org");
    let after = pairs(&[base[0], (":scheme", "https"), base[2], other]);
    want.extend([after.clone(), after]);
    let resize = [&[0x20, 0x3f, 0xe1, 0x1f, 0x82, 0x87, 0x84, 0x41, 0x0d][..], b"b.example.org"];
    for blocks in [c3.iter().map(|b| &b[..]).collect::<Vec<_>>(), c4.to_vec()] {
        let mut frames: Vec<Vec<u8>> = (0..3)
            .map(|i| frame(HEADERS, END_HEADERS | END_STREAM, 1 + 2 * i, blocks[i as usize]))
            .collect();
        frames.push(frame(HEADERS, END_HEADERS | END_STREAM, 7, &resize.concat()));
        frames.push(frame(HEADERS, END_HEADERS | END_STREAM, 9, &[0x82, 0x87, 0x84, 0xbe]));
        let (calls, stop) = run(&[up(&frames)]);
        assert_eq!(stop, None);
        let got: Vec<_> = opens(&calls, StreamKind::HttpReq)
            .into_iter()
            .map(|(_, _, m)| m["headers"].clone())
            .collect();
        assert_eq!(got, want);
    }
}

/// A request block over HEADERS (padded, with priority) and two CONTINUATION frames, a padded
/// DATA, frames skipped by length in between, and response trailers.
#[test]
fn continuation_padding_priority_and_trailers() {
    let blk = block(&[
        (":method", "POST"),
        (":scheme", "https"),
        (":authority", "api.example.com"),
        (":path", "/v1/m"),
        ("content-type", "application/json"),
    ]);
    let pad = |n: u8, inner: &[u8]| [&[n][..], inner, &vec![0; n as usize]].concat();
    let first = pad(3, &[&[0x80, 0, 0, 0, 15][..], &blk[..10]].concat());
    let steps = [
        up(&[
            frame(HEADERS, PADDED | PRIO, 1, &first),
            frame(CONT, 0, 1, &blk[10..31]),
            frame(CONT, END_HEADERS, 1, &blk[31..]),
            frame(PRIORITY, 0, 1, &[0, 0, 0, 0, 7]),
            frame(PING, 0, 0, &[1; 8]),
            frame(0xfa, 0, 0, b"unknown"),
            frame(DATA, PADDED | END_STREAM, 1, &pad(5, b"{\"a\":1}")),
        ]),
        down(&[
            ok(1, &[("content-type", "application/grpc")]),
            frame(DATA, PADDED, 1, &pad(2, b"reply")),
            head(1, true, &[("grpc-status", "0"), ("grpc-message", "fine")]),
        ]),
    ];
    let (calls, stop) = run(&steps);
    assert_eq!(stop, None);
    assert!(gaps(&calls).is_empty());
    let [(req, _, meta)] = opens(&calls, StreamKind::HttpReq).try_into().unwrap();
    assert_eq!(meta["headers"].as_array().unwrap().len(), 5);
    assert_eq!(meta["headers"][4], json!(["content-type", "application/json"]));
    assert_eq!(data_of(&calls, req), b"{\"a\":1}");
    let [(resp, _, _)] = opens(&calls, StreamKind::HttpResp).try_into().unwrap();
    assert_eq!(data_of(&calls, resp), b"reply");
    let (reason, tail) = end_of(&calls, resp);
    assert_eq!(reason, EndReason::Complete);
    assert_eq!(tail["trailers"], pairs(&[("grpc-status", "0"), ("grpc-message", "fine")]));
}

#[test]
fn rst_stream_and_goaway_end_streams_interrupted() {
    let steps = [
        up(&[req(1, "POST", "/v1/m"), body(1, false, b"par")]),
        down(&[ok(1, &[]), body(1, false, b"x")]),
        (Dir::Up, frame(RST, 0, 1, &8u32.to_be_bytes())),
    ];
    let (calls, _) = run(&steps);
    let [(req, ..)] = opens(&calls, StreamKind::HttpReq).try_into().unwrap();
    let [(resp, ..)] = opens(&calls, StreamKind::HttpResp).try_into().unwrap();
    for sid in [req, resp] {
        let (reason, tail) = end_of(&calls, sid);
        assert_eq!((reason, &tail["h2_error"]), (EndReason::Interrupted, &json!(8)));
    }

    let away = [&1u32.to_be_bytes()[..], &0u32.to_be_bytes()].concat();
    let steps = [
        up(&[req(1, "GET", "/a"), req(3, "POST", "/v1/m"), body(3, false, b"cut")]),
        down(&[ok(1, &[]), frame(GOAWAY, 0, 0, &away), body(1, true, b"ok")]),
    ];
    let (calls, stop) = run(&steps);
    assert_eq!(stop, None);
    let reqs = opens(&calls, StreamKind::HttpReq);
    let [(resp, paired, _)] = opens(&calls, StreamKind::HttpResp).try_into().unwrap();
    assert_eq!((reqs.len(), paired), (2, reqs[0].0));
    assert_eq!(data_of(&calls, resp), b"ok");
    assert_eq!(end_of(&calls, resp).0, EndReason::Complete);
    assert_eq!(end_of(&calls, reqs[0].0).0, EndReason::Complete);
    assert_eq!(end_of(&calls, reqs[1].0).0, EndReason::Interrupted);
}

/// `content-encoding: gzip` decodes; `grpc-encoding: gzip` is a GAP and no body byte.
#[test]
fn body_codings() {
    let plain = b"data: {\"text\":\"decoded\"}\n\n".repeat(50);
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&plain).unwrap();
    let gz = gz.finish().unwrap();
    let steps = [
        up(&[req(1, "GET", "/a"), req(3, "GET", "/b")]),
        down(&[
            ok(1, &[("content-encoding", "gzip")]),
            ok(3, &[("grpc-encoding", "gzip")]),
            body(1, false, &gz[..9]),
            body(3, true, b"\x01\0\0\0\x05zzzzz"),
            body(1, true, &gz[9..]),
        ]),
    ];
    let (calls, stop) = run(&steps);
    assert_eq!(stop, None);
    let resps = opens(&calls, StreamKind::HttpResp);
    let (gzip, grpc) = (&resps[0], &resps[1]);
    assert_eq!((&gzip.2["h2"], &grpc.2["h2"]), (&json!(1), &json!(3)));
    assert_eq!((&gzip.2["coding"], &gzip.2["decoded"]), (&json!(["gzip"]), &json!(true)));
    assert!(data_of(&calls, gzip.0) == plain);
    assert!(gaps_on(&calls, gzip.0).is_empty());
    assert!(data_of(&calls, grpc.0).is_empty());
    let [(reason, _)] = gaps_on(&calls, grpc.0).try_into().unwrap();
    assert_eq!(reason, GapReason::UnsupportedEncoding);
}

/// Extended CONNECT (RFC 8441): that stream gets its GAP and no DATA; the next is whole.
#[test]
fn an_extended_connect_stream_is_named_and_others_go_on() {
    let connect = head(
        1,
        false,
        &[
            (":method", "CONNECT"),
            (":protocol", "websocket"),
            (":scheme", "https"),
            (":path", "/ws"),
            (":authority", "api.example.com"),
        ],
    );
    let steps = [
        up(&[connect, body(1, false, &[0x82; 10]), req(3, "GET", "/b"), body(1, true, &[0x82; 5])]),
        down(&[ok(1, &[]), ok(3, &[]), body(3, true, b"whole"), body(1, true, b"")]),
    ];
    let (calls, stop) = run(&steps);
    assert_eq!(stop, None);
    let reqs = opens(&calls, StreamKind::HttpReq);
    assert_eq!(reqs.len(), 2);
    assert_eq!(gaps_on(&calls, reqs[0].0), [(GapReason::H2Unsupported, Some(16))]);
    assert!(data_of(&calls, reqs[0].0).is_empty());
    assert!(gaps_on(&calls, reqs[1].0).is_empty());
    let resps = opens(&calls, StreamKind::HttpResp);
    let (resp, ..) = resps.iter().find(|r| r.1 == reqs[1].0).expect("the second response");
    assert_eq!(data_of(&calls, *resp), b"whole");
    assert!(resps.iter().all(|r| data_of(&calls, r.0).is_empty() || r.0 == *resp));
    assert_eq!(end_of(&calls, *resp).0, EndReason::Complete);
}

/// `PUSH_PROMISE` and an HPACK error stop parsing (5a); every open stream gets its GAP.
#[test]
fn push_promise_and_hpack_errors_stop_parsing() {
    let promise = [&[0, 0, 0, 2][..], &block(&[(":method", "GET"), (":path", "/p")])].concat();
    let push = [
        up(&[req(1, "GET", "/a")]),
        down(&[ok(1, &[]), frame(PUSH, END_HEADERS, 1, &promise)]),
    ];
    let bad_index = [up(&[req(1, "POST", "/v1/m"), frame(HEADERS, END_HEADERS | END_STREAM, 3, &[0xbe])])];
    for steps in [&push[..], &bad_index[..]] {
        let (calls, stop) = run(steps);
        assert_eq!(stop.expect("parsing stops").reason, GapReason::H2Unsupported);
        let open: Vec<Sid> = [StreamKind::HttpReq, StreamKind::HttpResp]
            .into_iter()
            .flat_map(|k| opens(&calls, k))
            .map(|(sid, ..)| sid)
            .filter(|sid| end_of(&calls, *sid).0 != EndReason::Complete)
            .collect();
        assert_eq!(open.len(), 1);
        for sid in open {
            assert_eq!(gaps_on(&calls, sid).len(), 1);
            assert_eq!(end_of(&calls, sid).0, EndReason::DecodeError);
        }
    }
}

/// A table size update past 1 MiB (the server allowed 2 MiB) is the `hpack_table` guard.
#[test]
fn a_table_past_1_mib_is_the_hpack_table_guard() {
    let big = [0, 1, 0, 0x20, 0, 0]; // SETTINGS_HEADER_TABLE_SIZE = 2 MiB
    let steps = [
        down(&[frame(SETTINGS, 0, 0, &big)]),
        up(&[frame(HEADERS, END_HEADERS | END_STREAM, 1, &[0x3f, 0xe2, 0xff, 0x3f, 0x82])]),
    ];
    let (_, stop) = run(&steps);
    let stop = stop.expect("parsing stops");
    assert_eq!(stop.reason, GapReason::MemoryGuard);
    let tail = stop.tail.expect("a tail");
    assert_eq!((&tail["guard"], &tail["limit"]), (&json!("hpack_table"), &json!(1048576)));
}
