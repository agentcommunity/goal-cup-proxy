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
const API: &str = ":scheme=https :authority=api.example.com";

/// One frame: the 9-byte header, then the payload.
fn frame(ty: u8, flags: u8, sid: u32, payload: &[u8]) -> Vec<u8> {
    let n = (payload.len() as u32).to_be_bytes();
    [&n[1..], &[ty, flags], &sid.to_be_bytes(), payload].concat()
}

/// `name=value` fields separated by spaces.
fn fields(s: &str) -> Vec<(&str, &str)> {
    s.split(' ').map(|f| f.split_once('=').unwrap()).collect()
}

/// Literal fields without indexing, new name (RFC 7541 section 6.2.2).
fn block(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for (n, v) in fields(s) {
        out.extend([0, n.len() as u8].iter().chain(n.as_bytes()));
        out.extend([v.len() as u8].iter().chain(v.as_bytes()));
    }
    out
}

/// The meta or tail form of `s`.
fn pairs(s: &str) -> Value {
    json!(fields(s).iter().map(|(n, v)| [n, v]).collect::<Vec<_>>())
}

/// A complete header block in one HEADERS frame.
fn head(sid: u32, flags: u8, s: &str) -> Vec<u8> {
    frame(HEADERS, END_HEADERS | flags, sid, &block(s))
}

/// A request head; a GET ends its stream.
fn req(sid: u32, method: &str, path: &str) -> Vec<u8> {
    let end = if method == "GET" { END_STREAM } else { 0 };
    head(sid, end, &format!(":method={method} {API} :path={path}"))
}

fn ok(sid: u32, extra: &str) -> Vec<u8> {
    head(sid, 0, format!(":status=200 {extra}").trim_end())
}

fn body(sid: u32, end: bool, bytes: &[u8]) -> Vec<u8> {
    frame(DATA, if end { END_STREAM } else { 0 }, sid, bytes)
}

/// The client's frames, after its preface and SETTINGS.
fn up(frames: &[Vec<u8>]) -> (Dir, Vec<u8>) {
    let start = [PREFACE.to_vec(), frame(SETTINGS, 0, 0, &[])];
    (Dir::Up, [&start[..], frames].concat().concat())
}

/// The server's frames, after its SETTINGS.
fn down(frames: &[Vec<u8>]) -> (Dir, Vec<u8>) {
    let start = [frame(SETTINGS, 0, 0, &[])];
    (Dir::Down, [&start[..], frames].concat().concat())
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

#[test]
fn a_get_and_its_response_with_data() {
    let steps = [
        up(&[req(1, "GET", "/v1/models?page=2")]),
        down(&[
            ok(1, "content-type=text/plain"),
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
    let want = format!(":method=GET {API} :path=/v1/models");
    assert_eq!(meta["headers"], pairs(&want));
    let [(resp, paired, meta)] = opens(&calls, StreamKind::HttpResp).try_into().unwrap();
    assert_eq!((paired, &meta["h2"]), (req, &json!(1)));
    let want = ":status=200 content-type=text/plain";
    assert_eq!((&meta["ver"], &meta["headers"]), (&json!("2"), &pairs(want)));
    assert_eq!(data_of(&calls, resp), b"hello world");
    let (reason, tail) = end_of(&calls, resp);
    assert_eq!((reason, &tail["wire"]), (EndReason::Complete, &json!(11)));
}

/// RFC 7541 C.3 (no Huffman) and C.4 (Huffman): three requests on one connection share the
/// dynamic table; a fourth and fifth add size updates (to 0, then back to 4096).
#[test]
fn hpack_dynamic_table_and_size_updates() {
    let c3: [&[u8]; 3] = [
        b"\x82\x86\x84\x41\x0fwww.example.com",
        b"\x82\x86\x84\xbe\x58\x08no-cache",
        b"\x82\x87\x85\xbf\x40\x0acustom-key\x0ccustom-value",
    ];
    let c4: [&[u8]; 3] = [
        b"\x82\x86\x84\x41\x8c\xf1\xe3\xc2\xe5\xf2\x3a\x6b\xa0\xab\x90\xf4\xff",
        b"\x82\x86\x84\xbe\x58\x86\xa8\xeb\x10\x64\x9c\xbf",
        b"\x82\x87\x85\xbf\x40\x88\x25\xa8\x49\xe9\x5b\xa9\x7d\x7f\
          \x89\x25\xa8\x49\xe9\x5b\xb8\xe8\xb4\xbf",
    ];
    let resize: &[u8] = b"\x20\x3f\xe1\x1f\x82\x87\x84\x41\x0db.example.org";
    let reuse: &[u8] = b"\x82\x87\x84\xbe";
    let www = ":authority=www.example.com";
    let after = ":method=GET :scheme=https :path=/ :authority=b.example.org";
    let want = [
        format!(":method=GET :scheme=http :path=/ {www}"),
        format!(":method=GET :scheme=http :path=/ {www} cache-control=no-cache"),
        format!(":method=GET :scheme=https :path=/index.html {www} custom-key=custom-value"),
        after.to_string(),
        after.to_string(),
    ];
    let want: Vec<Value> = want.iter().map(|s| pairs(s)).collect();
    for blocks in [c3, c4] {
        let all = blocks.into_iter().chain([resize, reuse]);
        let frames: Vec<_> = (1..)
            .step_by(2)
            .zip(all)
            .map(|(sid, b)| frame(HEADERS, END_HEADERS | END_STREAM, sid, b))
            .collect();
        let (calls, stop) = run(&[up(&frames)]);
        assert_eq!(stop, None);
        let reqs = opens(&calls, StreamKind::HttpReq);
        let got: Vec<_> = reqs.into_iter().map(|r| r.2["headers"].clone()).collect();
        assert_eq!(got, want);
    }
}

/// A request block over HEADERS (padded, with priority) and two CONTINUATION frames, a padded
/// DATA, frames skipped by length in between, and response trailers.
#[test]
fn continuation_padding_priority_and_trailers() {
    let heads = format!(":method=POST {API} :path=/v1/m content-type=application/json");
    let blk = block(&heads);
    let pad = |n: u8, inner: &[u8]| [&[n][..], inner, &vec![0; n as usize]].concat();
    let first = pad(3, &[&[0x80, 0, 0, 0, 15][..], &blk[..10]].concat());
    let trailers = "grpc-status=0 grpc-message=fine";
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
            ok(1, "content-type=application/grpc"),
            frame(DATA, PADDED, 1, &pad(2, b"reply")),
            head(1, END_STREAM, trailers),
        ]),
    ];
    let (calls, stop) = run(&steps);
    assert_eq!(stop, None);
    assert!(gaps(&calls).is_empty());
    let [(req, _, meta)] = opens(&calls, StreamKind::HttpReq).try_into().unwrap();
    assert_eq!(meta["headers"], pairs(&heads));
    assert_eq!(data_of(&calls, req), b"{\"a\":1}");
    let [(resp, ..)] = opens(&calls, StreamKind::HttpResp).try_into().unwrap();
    assert_eq!(data_of(&calls, resp), b"reply");
    let (reason, tail) = end_of(&calls, resp);
    assert_eq!(reason, EndReason::Complete);
    assert_eq!(tail["trailers"], pairs(trailers));
}

#[test]
fn rst_stream_and_goaway_end_streams_interrupted() {
    let steps = [
        up(&[req(1, "POST", "/v1/m"), body(1, false, b"par")]),
        down(&[ok(1, ""), body(1, false, b"x")]),
        (Dir::Up, frame(RST, 0, 1, &8u32.to_be_bytes())),
    ];
    let (calls, _) = run(&steps);
    let [(req, ..)] = opens(&calls, StreamKind::HttpReq).try_into().unwrap();
    let [(resp, ..)] = opens(&calls, StreamKind::HttpResp).try_into().unwrap();
    for sid in [req, resp] {
        let (reason, tail) = end_of(&calls, sid);
        let want = (EndReason::Interrupted, &json!(8));
        assert_eq!((reason, &tail["h2_error"]), want);
    }

    let away = [0, 0, 0, 1, 0, 0, 0, 0];
    let steps = [
        up(&[
            req(1, "GET", "/a"),
            req(3, "POST", "/v1/m"),
            body(3, false, b"cut"),
        ]),
        down(&[ok(1, ""), frame(GOAWAY, 0, 0, &away), body(1, true, b"ok")]),
    ];
    let (calls, stop) = run(&steps);
    assert_eq!(stop, None);
    let reqs = opens(&calls, StreamKind::HttpReq);
    let [(resp, paired, _)] = opens(&calls, StreamKind::HttpResp).try_into().unwrap();
    assert_eq!((reqs.len(), paired), (2, reqs[0].0));
    assert_eq!(data_of(&calls, resp), b"ok");
    let reasons = [resp, reqs[0].0, reqs[1].0].map(|s| end_of(&calls, s).0);
    let (done, cut) = (EndReason::Complete, EndReason::Interrupted);
    assert_eq!(reasons, [done, done, cut]);
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
            ok(1, "content-encoding=gzip"),
            ok(3, "grpc-encoding=gzip"),
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
    let coding = (&gzip.2["coding"], &gzip.2["decoded"]);
    assert_eq!(coding, (&json!(["gzip"]), &json!(true)));
    assert!(data_of(&calls, gzip.0) == plain);
    assert!(gaps_on(&calls, gzip.0).is_empty());
    assert!(data_of(&calls, grpc.0).is_empty());
    let [(reason, _)] = gaps_on(&calls, grpc.0).try_into().unwrap();
    assert_eq!(reason, GapReason::UnsupportedEncoding);
}

/// Extended CONNECT (RFC 8441): that stream gets its GAP and no DATA; the next is whole.
#[test]
fn an_extended_connect_stream_is_named_and_others_go_on() {
    let connect = format!(":method=CONNECT :protocol=websocket {API} :path=/ws");
    let steps = [
        up(&[
            head(1, 0, &connect),
            body(1, false, &[0x82; 10]),
            req(3, "GET", "/b"),
            body(1, true, &[0x82; 5]),
        ]),
        down(&[ok(1, ""), ok(3, ""), body(3, true, b"whole"), body(1, true, b"")]),
    ];
    let (calls, stop) = run(&steps);
    assert_eq!(stop, None);
    let reqs = opens(&calls, StreamKind::HttpReq);
    let [(ws, ..), (next, ..)] = reqs.try_into().unwrap();
    let want = [(GapReason::H2Unsupported, Some(16))];
    assert_eq!(gaps_on(&calls, ws), want);
    assert!(data_of(&calls, ws).is_empty());
    assert!(gaps_on(&calls, next).is_empty());
    let resps = opens(&calls, StreamKind::HttpResp);
    let resp = resps.iter().find(|r| r.1 == next).expect("its response").0;
    assert_eq!(data_of(&calls, resp), b"whole");
    let mut rest = resps.iter().filter(|r| r.0 != resp);
    assert!(rest.all(|r| data_of(&calls, r.0).is_empty()));
    assert_eq!(end_of(&calls, resp).0, EndReason::Complete);
}

/// `PUSH_PROMISE` and an HPACK error stop parsing (5a); every open stream gets its GAP.
#[test]
fn push_promise_and_hpack_errors_stop_parsing() {
    let promise = [&[0, 0, 0, 2][..], &block(":method=GET :path=/p")].concat();
    let push = [
        up(&[req(1, "GET", "/a")]),
        down(&[ok(1, ""), frame(PUSH, END_HEADERS, 1, &promise)]),
    ];
    // Index 62 with an empty dynamic table.
    let bad = frame(HEADERS, END_HEADERS | END_STREAM, 3, &[0xbe]);
    let bad_index = [up(&[req(1, "POST", "/v1/m"), bad])];
    for steps in [&push[..], &bad_index[..]] {
        let (calls, stop) = run(steps);
        let stop = stop.expect("parsing stops");
        assert_eq!(stop.reason, GapReason::H2Unsupported);
        let kinds = [StreamKind::HttpReq, StreamKind::HttpResp];
        let sids = kinds.into_iter().flat_map(|k| opens(&calls, k));
        let open: Vec<Sid> = sids
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
    // SETTINGS_HEADER_TABLE_SIZE 2 MiB; then a size update to 1 MiB + 1, then `:method GET`.
    let big = [0, 1, 0, 0x20, 0, 0];
    let update = [0x3f, 0xe2, 0xff, 0x3f, 0x82];
    let steps = [
        down(&[frame(SETTINGS, 0, 0, &big)]),
        up(&[frame(HEADERS, END_HEADERS | END_STREAM, 1, &update)]),
    ];
    let (_, stop) = run(&steps);
    let stop = stop.expect("parsing stops");
    assert_eq!(stop.reason, GapReason::MemoryGuard);
    let tail = stop.tail.expect("a tail");
    let want = (&json!("hpack_table"), &json!(1048576));
    assert_eq!((&tail["guard"], &tail["limit"]), want);
}
