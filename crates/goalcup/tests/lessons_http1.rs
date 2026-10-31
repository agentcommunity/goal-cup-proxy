//! PRD-17 section 2.3, rows H1 to H6 and H8, plus the parser half of H2: the HTTP/1.1 edge cases
//! the 0.7 relay learned the hard way, driven through the public parser `proxy::http1::Http1`
//! with `Recorder::for_tests`, so what is asserted is the records (DATA bytes, GAP reasons, END
//! reasons and tails) and the `Stop` the relay would act on. Whole-path cases wait for
//! `proxy::serve`: H9 (a head over 64 KiB relays on), H10 (credentials reach the upstream
//! unchanged), H11 (every request on a decrypted host is recorded) and the relay half of H2 (the
//! client gets both answers byte for byte, the later request sits inside the `conn` GAP).
//!
//! What PRD-03's `proxy_http1.rs` already holds at this layer is not repeated here: each coding,
//! an unknown coding, trailers in the END tail, pipelining, `HEAD` and `304`, the 64 KiB head,
//! the generic framing fault, the `101` hand-off, bytes held at the close, a panic, long lines.

mod common;

use common::encode::gzip;
use common::recorder::{data_of, ends, gaps, opens as all_opens};
use goalcup::pipeline::{Call, Calls, EndReason, GapReason, Mono, Recorder, Sid, StreamKind};
use goalcup::proxy::http1::Http1;
use goalcup::proxy::pump::{Parser, Stop};
use goalcup::proxy::ws::Dir;
use serde_json::{Value, json};

const CONN: Sid = 1000;
const ROOM: u64 = 1 << 30;

/// How the bytes of one direction reach the parser.
#[derive(Clone, Copy)]
enum Cut {
    Whole,
    /// One byte at a time.
    Bytes,
    /// Two pieces, split at this offset.
    At(usize),
}

fn pieces(bytes: &[u8], cut: Cut) -> Vec<&[u8]> {
    let all: Vec<&[u8]> = match cut {
        Cut::Whole => vec![bytes],
        Cut::Bytes => bytes.chunks(1).collect(),
        Cut::At(k) => {
            let k = k.min(bytes.len());
            vec![&bytes[..k], &bytes[k..]]
        }
    };
    all.into_iter().filter(|p| !p.is_empty()).collect()
}

/// What one connection left behind.
struct Run {
    calls: Calls,
    /// The stop a `feed` answered (the first one; nothing is fed after it).
    fed: Option<Stop>,
    /// What `end` answered.
    closed: Option<Stop>,
}

/// The client bytes, then the server bytes, then the close. `up_cut` and `down_cut` shape the
/// feed of each direction; `how` is the close: its reason, whether the server's close was cut,
/// and the client bytes never forwarded.
fn drive(
    up: &[u8],
    down: &[u8],
    (up_cut, down_cut): (Cut, Cut),
    how: (EndReason, bool, &[u8]),
) -> Run {
    let (rec, calls) = Recorder::for_tests(ROOM);
    let mut p = Http1::new(&rec, CONN);
    let mut fed = None;
    'feeding: for (dir, bytes, cut) in [(Dir::Up, up, up_cut), (Dir::Down, down, down_cut)] {
        for piece in pieces(bytes, cut) {
            if let Err(stop) = p.feed(dir, Mono::now(), piece) {
                fed = Some(stop);
                break 'feeding;
            }
        }
    }
    let (reason, cut_close, unsent) = how;
    let closed = Box::new(p).end(Mono::now(), reason, cut_close, unsent);
    Run { calls, fed, closed }
}

/// A clean exchange: nothing stops, nothing is held at the close.
fn exchange(up: &[u8], down: &[u8], cut: Cut) -> Calls {
    let run = drive(up, down, (cut, cut), (EndReason::Complete, false, &[]));
    assert!(run.fed.is_none(), "parsing goes on");
    assert!(run.closed.is_none(), "nothing held at the close");
    run.calls
}

/// `(sid, req, meta)` of every stream of `kind`, in order.
fn of_kind(calls: &Calls, kind: StreamKind) -> Vec<(Sid, Sid, Value)> {
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
    let (_, reason, _, tail) = ends(calls)
        .into_iter()
        .find(|e| e.0 == sid)
        .expect("the stream ended");
    (reason, tail.unwrap_or_default())
}

/// Every stream as (meta, DATA bytes, END reason, END tail): what a record reader sees, without
/// the stream IDs and times, so two feeds of the same bytes compare equal.
fn summary(calls: &Calls) -> Vec<(Value, Vec<u8>, EndReason, Value)> {
    all_opens(calls)
        .into_iter()
        .map(|(sid, _, meta)| {
            let (reason, tail) = end_of(calls, sid);
            (meta, data_of(calls, sid), reason, tail)
        })
        .collect()
}

const CHUNKED_HEAD: &[u8] = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
const TWO_GETS: &[u8] = b"GET /c HTTP/1.1\r\n\r\nGET /d HTTP/1.1\r\n\r\n";
const OK_RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";

/// H1: empty lines before a start line are skipped (and are not held at the close), and a bare
/// LF ends a line. Header values are kept as sent, in every feed.
#[test]
fn head_scan_skips_blank_lines_and_takes_bare_lf() {
    let up = b"\r\n\n\r\nPOST /a HTTP/1.1\nHost: h\nContent-Length: 2\n\nhiGET /b HTTP/1.1\n\n";
    let down = b"\n\r\nHTTP/1.1 200 OK\nContent-Length: 2\n\nok\r\nHTTP/1.1 204 No Content\n\n";
    // The empty line after `ok` is one of the next head's leading blank lines.
    let mut want = None;
    for cut in [Cut::Whole, Cut::Bytes, Cut::At(5), Cut::At(40)] {
        let calls = exchange(up, down, cut);
        assert!(gaps(&calls).is_empty());
        let reqs = of_kind(&calls, StreamKind::HttpReq);
        let paths: Vec<_> = reqs.iter().map(|r| r.2["path"].clone()).collect();
        assert_eq!(paths, [json!("/a"), json!("/b")]);
        assert_eq!(
            reqs[0].2["headers"],
            json!([["Host", "h"], ["Content-Length", "2"]])
        );
        assert_eq!(reqs[0].2["ver"], "1.1");
        assert_eq!(data_of(&calls, reqs[0].0), b"hi");
        let resps = of_kind(&calls, StreamKind::HttpResp);
        let status: Vec<_> = resps.iter().map(|r| (r.1, r.2["status"].clone())).collect();
        assert_eq!(
            status,
            [(reqs[0].0, json!(200)), (reqs[1].0, json!(204))],
            "paired in order"
        );
        assert_eq!(data_of(&calls, resps[0].0), b"ok");
        let got = summary(&calls);
        match &want {
            Some(first) => assert_eq!(first, &got, "same in every feed"),
            None => want = Some(got),
        }
    }
}

/// H3: `Transfer-Encoding: gzip, chunked` undoes the gzip after the de-chunking.
#[test]
fn transfer_gzip_then_chunked_decodes() {
    let plain = b"data: {\"delta\":\"token\"}\n\n".repeat(200);
    let coded = gzip(&plain);
    let mut down = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip, chunked\r\n\r\n".to_vec();
    let mut chunk_bytes = 0;
    for piece in coded.chunks(37) {
        let line = format!("{:x}\r\n", piece.len());
        down.extend_from_slice(line.as_bytes());
        down.extend_from_slice(piece);
        down.extend_from_slice(b"\r\n");
        chunk_bytes += line.len() + piece.len() + 2;
    }
    down.extend_from_slice(b"0\r\n\r\n");
    chunk_bytes += 5;
    for cut in [Cut::Whole, Cut::Bytes] {
        let calls = exchange(b"GET /t HTTP/1.1\r\n\r\n", &down, cut);
        assert!(gaps(&calls).is_empty());
        let [(resp, _, meta)] = of_kind(&calls, StreamKind::HttpResp).try_into().unwrap();
        assert_eq!(
            (&meta["coding"], &meta["decoded"]),
            (&json!(["gzip"]), &json!(true))
        );
        assert!(data_of(&calls, resp) == plain, "the gzip is undone");
        let (reason, tail) = end_of(&calls, resp);
        assert_eq!(
            (reason, &tail["wire"]),
            (EndReason::Complete, &json!(chunk_bytes))
        );
    }
}

/// H4: a chunked body split anywhere (one byte at a time, or in two pieces at every offset) gives
/// the same records, and ends exactly once: the response after it is its own stream.
#[test]
fn chunked_split_at_every_byte_ends_exactly() {
    let body = b"3;a=b\r\nabc\r\n0A\r\n0123456789\n2\r\nxy\r\n0\r\nX-T: 1\r\n\r\n";
    let down = [CHUNKED_HEAD, body, OK_RESPONSE].concat();
    let whole = exchange(TWO_GETS, &down, Cut::Whole);
    let want = summary(&whole);
    assert_eq!(want.len(), 4, "two requests, two responses");
    assert_eq!(want[2].1, b"abc0123456789xy");
    assert_eq!(want[2].2, EndReason::Complete);
    let trailers = json!([["X-T", "1"]]);
    assert_eq!(
        want[2].3,
        json!({"wire": body.len(), "trailers": trailers}),
        "the framing is counted, the trailer is the tail's"
    );
    assert_eq!(want[3].1, b"ok");
    let mut cuts = vec![Cut::Bytes];
    cuts.extend((1..down.len()).map(Cut::At));
    for cut in cuts {
        let calls = exchange(TWO_GETS, &down, cut);
        assert_eq!(summary(&calls), want);
        assert_eq!(ends(&calls).len(), all_opens(&calls).len(), "one END each");
    }
}

/// H4: what a chunk-size line may hold, and what stops parsing.
#[test]
fn chunk_size_rules() {
    let long_ext = format!("5;{}\r\nhello\r\n0\r\n\r\n", "e".repeat(4096));
    let padded = format!("{}5\r\nhello\r\n0\r\n\r\n", "0".repeat(5000));
    let ok: [(&str, Vec<u8>, &[u8]); 8] = [
        (
            "lower hex",
            b"a\r\n0123456789\r\n0\r\n\r\n".to_vec(),
            b"0123456789",
        ),
        (
            "upper hex",
            b"A\r\n0123456789\r\n0\r\n\r\n".to_vec(),
            b"0123456789",
        ),
        (
            "extensions",
            b"5;x=1;y=\"q;r\"\r\nhello\r\n0\r\n\r\n".to_vec(),
            b"hello",
        ),
        (
            "space before the extension",
            b"5 ; x\r\nhello\r\n0\r\n\r\n".to_vec(),
            b"hello",
        ),
        ("a 4 KiB extension", long_ext.into_bytes(), b"hello"),
        (
            "leading zeros are not digits",
            padded.into_bytes(),
            b"hello",
        ),
        ("bare LF everywhere", b"5\nhello\n0\n\n".to_vec(), b"hello"),
        (
            "last chunk with an extension",
            b"5\r\nhello\r\n0;done\r\n\r\n".to_vec(),
            b"hello",
        ),
    ];
    for (label, body, want) in ok {
        let down = [CHUNKED_HEAD, &body, OK_RESPONSE].concat();
        for cut in [Cut::Whole, Cut::Bytes] {
            let calls = exchange(TWO_GETS, &down, cut);
            assert!(gaps(&calls).is_empty(), "{label}");
            let resps = of_kind(&calls, StreamKind::HttpResp);
            assert_eq!(resps.len(), 2, "{label}");
            assert_eq!(data_of(&calls, resps[0].0), want, "{label}");
            let (reason, tail) = end_of(&calls, resps[0].0);
            assert_eq!(
                (reason, &tail["wire"]),
                (EndReason::Complete, &json!(body.len())),
                "{label}"
            );
            assert_eq!(data_of(&calls, resps[1].0), b"ok", "{label}");
        }
    }

    let bad: [(&str, &[u8], &[u8]); 7] = [
        ("not hex", b"3\r\nabc\r\nzz\r\n", b"abc"),
        ("an empty size line", b"3\r\nabc\r\n\r\n", b"abc"),
        ("a sign", b"-1\r\n", b""),
        ("0x prefix", b"0x5\r\nhello\r\n", b""),
        ("17 significant digits", b"11111111111111111\r\n", b""),
        ("overflow", b"FFFFFFFFFFFFFFFFFFFF\r\n", b""),
        (
            "junk after the data",
            b"5\r\nhelloXX\r\n0\r\n\r\n",
            b"hello",
        ),
    ];
    for (label, body, kept) in bad {
        let down = [CHUNKED_HEAD, body, OK_RESPONSE].concat();
        let run = drive(
            TWO_GETS,
            &down,
            (Cut::Whole, Cut::Bytes),
            (EndReason::Complete, false, &[]),
        );
        let stop = run.fed.unwrap_or_else(|| panic!("{label}: parsing stops"));
        assert_eq!(stop.reason, GapReason::DecodeFailed, "{label}");
        let resps = of_kind(&run.calls, StreamKind::HttpResp);
        assert_eq!(resps.len(), 1, "{label}: nothing parses after the fault");
        assert_eq!(data_of(&run.calls, resps[0].0), kept, "{label}");
        assert_eq!(
            end_of(&run.calls, resps[0].0).0,
            EndReason::DecodeError,
            "{label}"
        );
        let gap_sids: Vec<_> = gaps(&run.calls)
            .into_iter()
            .map(|g| match g {
                Call::Gap(sid, reason, ..) => (sid, reason),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(gap_sids, [(resps[0].0, GapReason::DecodeFailed)], "{label}");
    }
}

/// H5: how a body is framed. `Content-Length` and `Transfer-Encoding` together, equal duplicate
/// lengths, a length that does not parse or does not agree, no body for 204 and for a length of
/// 0, a body that runs to the close, and a 2xx answer to an inner `CONNECT`.
#[test]
fn framing_matrix() {
    let two = |a: &[u8]| [a, OK_RESPONSE].concat();

    // Chunked wins over a length (RFC 9112 section 6.3); 204 has no body whatever it says.
    let ok_cases: [(&str, Vec<u8>, &[u8]); 6] = [
        (
            "chunked beats a length",
            two(b"HTTP/1.1 200 OK\r\nContent-Length: 99\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n"),
            b"hello",
        ),
        (
            "204 has no body",
            two(b"HTTP/1.1 204 No Content\r\nContent-Length: 5\r\n\r\n"),
            b"",
        ),
        (
            "a length of 0 has no body",
            two(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"),
            b"",
        ),
        (
            "equal duplicate lengths",
            two(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\nhello"),
            b"hello",
        ),
        (
            "an equal list",
            two(b"HTTP/1.1 200 OK\r\nContent-Length: 5, 5\r\n\r\nhello"),
            b"hello",
        ),
        (
            "a refused CONNECT has a body",
            two(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 5\r\n\r\nhello"),
            b"hello",
        ),
    ];
    for (label, down, want) in ok_cases {
        let up: &[u8] = if label.starts_with("a refused") {
            b"CONNECT h:443 HTTP/1.1\r\n\r\nGET /d HTTP/1.1\r\n\r\n"
        } else {
            TWO_GETS
        };
        for cut in [Cut::Whole, Cut::Bytes] {
            let calls = exchange(up, &down, cut);
            assert!(gaps(&calls).is_empty(), "{label}");
            let resps = of_kind(&calls, StreamKind::HttpResp);
            assert_eq!(resps.len(), 2, "{label}");
            assert_eq!(data_of(&calls, resps[0].0), want, "{label}");
            assert_eq!(end_of(&calls, resps[0].0).0, EndReason::Complete, "{label}");
            assert_eq!(data_of(&calls, resps[1].0), b"ok", "{label}");
        }
    }

    // A length that cannot be trusted stops parsing for the connection.
    for length in [
        "Content-Length: 2\r\nContent-Length: 3",
        "Content-Length: 2, 3",
        "Content-Length: +2",
        "Content-Length: 0x2",
        "Content-Length: -1",
        "Content-Length: 1.5",
        "Content-Length:",
    ] {
        let down = two(format!("HTTP/1.1 200 OK\r\n{length}\r\n\r\nok").as_bytes());
        let run = drive(
            TWO_GETS,
            &down,
            (Cut::Whole, Cut::Whole),
            (EndReason::Complete, false, &[]),
        );
        let stop = run.fed.unwrap_or_else(|| panic!("{length}: parsing stops"));
        assert_eq!(stop.reason, GapReason::DecodeFailed, "{length}");
        assert!(
            of_kind(&run.calls, StreamKind::HttpResp).is_empty(),
            "{length}"
        );
    }

    // No length and not chunked: the body is whatever comes until the server closes.
    let down = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\ndata: 1\n\ndata: 2\n\n";
    for cut in [Cut::Whole, Cut::Bytes] {
        let run = drive(
            b"GET /s HTTP/1.1\r\n\r\n",
            down,
            (cut, cut),
            (EndReason::Complete, false, &[]),
        );
        assert!(run.fed.is_none() && run.closed.is_none());
        let [(resp, _, _)] = of_kind(&run.calls, StreamKind::HttpResp)
            .try_into()
            .unwrap();
        assert_eq!(data_of(&run.calls, resp), b"data: 1\n\ndata: 2\n\n");
        let (reason, tail) = end_of(&run.calls, resp);
        assert_eq!((reason, &tail["wire"]), (EndReason::Complete, &json!(18)));
        // A server close that was cut short is not a finished body.
        let run = drive(
            b"GET /s HTTP/1.1\r\n\r\n",
            down,
            (cut, cut),
            (EndReason::Complete, true, &[]),
        );
        let [(resp, _, _)] = of_kind(&run.calls, StreamKind::HttpResp)
            .try_into()
            .unwrap();
        assert_eq!(end_of(&run.calls, resp).0, EndReason::Interrupted);
        assert_eq!(data_of(&run.calls, resp), b"data: 1\n\ndata: 2\n\n");
    }

    // A 2xx to an inner CONNECT is a tunnel, not HTTP: parsing stops there.
    let run = drive(
        b"CONNECT h:443 HTTP/1.1\r\n\r\n",
        b"HTTP/1.1 200 Connection established\r\n\r\n\x16\x03\x01\x02\x00",
        (Cut::Whole, Cut::Whole),
        (EndReason::Complete, false, &[]),
    );
    let stop = run.fed.expect("parsing stops at the tunnel");
    assert_eq!(stop.reason, GapReason::DecodeFailed);
    assert!(of_kind(&run.calls, StreamKind::HttpResp).is_empty());
}

/// H6: `100`, `103` and other informational heads before the final answer are notes. They carry
/// no body whatever they say, and the final answer pairs with the request it answers.
#[test]
fn informational_then_final_keeps_the_pair() {
    let up = b"POST /a HTTP/1.1\r\nExpect: 100-continue\r\nContent-Length: 2\r\n\r\n{}GET /b HTTP/1.1\r\n\r\n";
    let down = b"HTTP/1.1 100 Continue\r\nContent-Length: 7\r\n\r\n\
        HTTP/1.1 103 Early Hints\r\nLink: </s.css>; rel=preload\r\n\r\n\
        HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nA\
        HTTP/1.1 102 Processing\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nB";
    for cut in [Cut::Whole, Cut::Bytes] {
        let calls = exchange(up, down, cut);
        assert!(gaps(&calls).is_empty());
        let reqs = of_kind(&calls, StreamKind::HttpReq);
        let resps = of_kind(&calls, StreamKind::HttpResp);
        assert_eq!(reqs.len(), 2);
        let pairs: Vec<_> = resps.iter().map(|r| (r.1, r.2["status"].clone())).collect();
        assert_eq!(pairs, [(reqs[0].0, json!(200)), (reqs[1].0, json!(200))]);
        assert_eq!(data_of(&calls, resps[0].0), b"A");
        assert_eq!(data_of(&calls, resps[1].0), b"B");
        let notes: Vec<_> = common::recorder::events(&calls)
            .into_iter()
            .map(|e| (e["what"].clone(), e["status"].clone()))
            .collect();
        let note = json!("informational_response");
        assert_eq!(
            notes,
            [
                (note.clone(), json!(100)),
                (note.clone(), json!(103)),
                (note, json!(102))
            ]
        );
    }
}

/// A label, the client bytes, the server bytes, and the DATA the cut body keeps.
type Eof<'a> = (&'a str, &'a [u8], &'a [u8], &'a [u8]);

/// H8: the connection ends inside a body. The stream ends `interrupted`, never `complete`, and
/// what arrived is in the record.
#[test]
fn eof_inside_a_body_is_interrupted() {
    let cases: [Eof; 4] = [
        (
            "a response of known length",
            b"GET /a HTTP/1.1\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabcd",
            b"abcd",
        ),
        (
            "a chunked response",
            b"GET /a HTTP/1.1\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n5\r\nwor",
            b"hellowor",
        ),
        (
            "a chunked response cut before its trailer section ends",
            b"GET /a HTTP/1.1\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\nX-T: 1\r\n",
            b"hello",
        ),
        (
            "a request body",
            b"POST /a HTTP/1.1\r\nContent-Length: 10\r\n\r\nabc",
            b"",
            b"abc",
        ),
    ];
    for (label, up, down, want) in cases {
        for cut in [Cut::Whole, Cut::Bytes] {
            // The peers closed cleanly, so the close itself says `complete`: it is the body that
            // is unfinished.
            let run = drive(up, down, (cut, cut), (EndReason::Complete, false, &[]));
            assert!(run.fed.is_none(), "{label}");
            assert!(
                run.closed.is_none(),
                "{label}: every byte is in some stream"
            );
            assert!(gaps(&run.calls).is_empty(), "{label}");
            let kind = if down.is_empty() {
                StreamKind::HttpReq
            } else {
                StreamKind::HttpResp
            };
            let [(sid, _, _)] = of_kind(&run.calls, kind).try_into().unwrap();
            assert_eq!(data_of(&run.calls, sid), want, "{label}");
            assert_eq!(end_of(&run.calls, sid).0, EndReason::Interrupted, "{label}");
        }
    }
}

/// H8: client bytes read and never forwarded because the upstream had closed are recorded, and
/// the stream they make says how many were never sent (END tail `unsent`).
#[test]
fn request_after_upstream_close_is_marked_unsent() {
    let first = b"GET /a HTTP/1.1\r\n\r\n";
    let down = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
    let cases: [(&str, &[u8], EndReason, &[u8]); 3] = [
        (
            "a bare GET",
            b"GET /late HTTP/1.1\r\n\r\n",
            EndReason::Complete,
            b"",
        ),
        (
            "a POST with its body",
            b"POST /late HTTP/1.1\r\nContent-Length: 3\r\n\r\nabc",
            EndReason::Complete,
            b"abc",
        ),
        (
            "a POST cut inside its body",
            b"POST /late HTTP/1.1\r\nContent-Length: 10\r\n\r\nabc",
            EndReason::Interrupted,
            b"abc",
        ),
    ];
    for (label, unsent, reason, body) in cases {
        let run = drive(
            first,
            down,
            (Cut::Whole, Cut::Whole),
            (EndReason::Interrupted, false, unsent),
        );
        assert!(run.fed.is_none(), "{label}");
        assert!(
            run.closed.is_none(),
            "{label}: recorded, so nothing is held"
        );
        assert!(gaps(&run.calls).is_empty(), "{label}");
        let reqs = of_kind(&run.calls, StreamKind::HttpReq);
        assert_eq!(reqs.len(), 2, "{label}");
        assert_eq!(reqs[1].2["path"], "/late", "{label}");
        assert_eq!(data_of(&run.calls, reqs[1].0), body, "{label}");
        let (got, tail) = end_of(&run.calls, reqs[1].0);
        assert_eq!(got, reason, "{label}");
        assert_eq!(tail["unsent"], unsent.len(), "{label}");
        // The request that was forwarded is not marked.
        let (_, tail) = end_of(&run.calls, reqs[0].0);
        assert!(tail.get("unsent").is_none(), "{label}");
    }
}

/// H2, the parser half: a head this parser cannot frame stops parsing for the connection, and the
/// `Stop` counts every byte relayed and recorded in no stream. `Content-Length: 5, 5` frames.
/// (The relay half, the client getting both answers and a later request sitting inside the `conn`
/// GAP, is `proxy::serve`'s and waits for it.)
#[test]
fn unframeable_heads_stop_parsing_and_count_their_bytes() {
    let heads: [(&str, &[u8]); 9] = [
        (
            "obsolete line folding",
            b"GET / HTTP/1.1\r\nA: b\r\n folded\r\n\r\n",
        ),
        (
            "a space in a header name",
            b"GET / HTTP/1.1\r\nBad Name: x\r\n\r\n",
        ),
        (
            "a header line with no colon",
            b"GET / HTTP/1.1\r\nno colon here\r\n\r\n",
        ),
        (
            "a non-ASCII Content-Length",
            b"POST / HTTP/1.1\r\nContent-Length: 5\xc3\xa9\r\n\r\nhello",
        ),
        (
            "conflicting Content-Length lines",
            b"POST / HTTP/1.1\r\nContent-Length: 3\r\nContent-Length: 4\r\n\r\nabc",
        ),
        (
            "a final transfer coding that is not chunked",
            b"POST / HTTP/1.1\r\nTransfer-Encoding: gzip\r\n\r\nabc",
        ),
        (
            "a non-ASCII Transfer-Encoding",
            b"POST / HTTP/1.1\r\nTransfer-Encoding: chunk\xc3\xa9d\r\n\r\n",
        ),
        ("an HTTP/2 request line", b"GET / HTTP/2.0\r\n\r\n"),
        ("a request line with no target", b"GET HTTP/1.1\r\n\r\n"),
    ];
    for (label, head) in heads {
        let run = drive(
            head,
            b"",
            (Cut::Whole, Cut::Whole),
            (EndReason::Complete, false, &[]),
        );
        let stop = run.fed.unwrap_or_else(|| panic!("{label}: parsing stops"));
        assert_eq!(stop.reason, GapReason::DecodeFailed, "{label}");
        assert_eq!(
            stop.held,
            head.len() as u64,
            "{label}: every byte is counted"
        );
        assert!(!stop.at_close, "{label}");
        assert!(
            of_kind(&run.calls, StreamKind::HttpReq).is_empty(),
            "{label}"
        );
        assert!(run.closed.is_none(), "{label}: the close adds nothing");
    }

    let calls = exchange(
        b"POST / HTTP/1.1\r\nContent-Length: 5, 5\r\n\r\nhello",
        b"",
        Cut::Bytes,
    );
    let [(req, _, _)] = of_kind(&calls, StreamKind::HttpReq).try_into().unwrap();
    assert_eq!(data_of(&calls, req), b"hello");
    assert!(gaps(&calls).is_empty());
}
