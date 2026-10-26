//! `proxy::http::relay_http` between two in-memory peers, against a test-local recording
//! sink. Every expectation is the literal bytes a peer wrote, or a value the test derives
//! independently of the relay (flate2's and zstd's encoders, a fixture from the reference brotli
//! encoder, RFC 6455's sample key, the frame oracle in `m2_http_support`).

use crate::common::m2_http_support;

use goalcup_cli::proxy::api::{HttpOutcome, Upgrade};
use goalcup_protocol::capture::{
    CaptureFrame, CaptureMeta, CaptureSource, ConnectionEvent, EndReason, FrameEvent, GapReason,
    MAX_CAPTURE_QUEUE_BYTES, MAX_METADATA_STRING_BYTES, MAX_SEGMENT_BYTES, WireIdentity,
};
use m2_http_support::*;
use std::io;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const WIRE_REQUEST: CaptureSource = CaptureSource::WireRequest;
const WIRE_RESPONSE: CaptureSource = CaptureSource::WireResponse;

/// RFC 6455 §1.3's sample handshake.
const WS_KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";
const WS_ACCEPT: &str = "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";

fn request(method: &str, target: &str, headers: &[&str], body: &[u8]) -> Vec<u8> {
    head_with_host(method, target, HOST, headers, body)
}

fn head_with_host(
    method: &str,
    target: &str,
    host: &str,
    headers: &[&str],
    body: &[u8],
) -> Vec<u8> {
    let mut out = format!("{method} {target} HTTP/1.1\r\nHost: {host}\r\n");
    for header in headers {
        out.push_str(header);
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
    let mut out = out.into_bytes();
    out.extend_from_slice(body);
    out
}

fn response(status_line: &str, headers: &[&str], body: &[u8]) -> Vec<u8> {
    let mut out = format!("{status_line}\r\n");
    for header in headers {
        out.push_str(header);
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
    let mut out = out.into_bytes();
    out.extend_from_slice(body);
    out
}

fn length(body: &[u8]) -> String {
    format!("Content-Length: {}", body.len())
}

/// The client sends `req`; the upstream must see exactly it. The upstream answers `resp`;
/// the client must see exactly it.
async fn round_trip(h: &mut Harness, req: &[u8], resp: &[u8]) {
    write(&mut h.client, req).await;
    assert_eq!(
        read_exact(&mut h.upstream, req.len()).await,
        req,
        "request changed in transit"
    );
    write(&mut h.upstream, resp).await;
    assert_eq!(
        read_exact(&mut h.client, resp.len()).await,
        resp,
        "response changed in transit"
    );
}

/// Both peers end cleanly; the relay reports `Closed`.
async fn close_both(h: Harness) {
    let outcome = end_both(h).await;
    assert!(matches!(outcome, HttpOutcome::Closed), "{outcome:?}");
}

/// Both peers end cleanly; the relay reports `Opaque`: it stopped parsing along the way.
async fn close_opaque(h: Harness) {
    let outcome = end_both(h).await;
    assert!(matches!(outcome, HttpOutcome::Opaque), "{outcome:?}");
}

async fn end_both(mut h: Harness) -> HttpOutcome {
    h.client.shutdown().await.unwrap();
    assert_eq!(
        read_to_end(&mut h.upstream).await,
        b"",
        "no stray bytes upstream"
    );
    h.upstream.shutdown().await.unwrap();
    assert_eq!(
        read_to_end(&mut h.client).await,
        b"",
        "no stray bytes to the client"
    );
    finish(h.relay).await.expect("relay failed")
}

fn find(views: &[StreamView], source: CaptureSource, exchange: u64) -> &StreamView {
    let mut found = views
        .iter()
        .filter(|v| v.source == source && exchange_of(v) == exchange);
    let view = found
        .next()
        .unwrap_or_else(|| panic!("no {source:?} stream for exchange {exchange}"));
    assert!(
        found.next().is_none(),
        "two {source:?} streams for exchange {exchange}"
    );
    view
}

fn has(views: &[StreamView], source: CaptureSource, exchange: u64) -> bool {
    views
        .iter()
        .any(|v| v.source == source && exchange_of(v) == exchange)
}

fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/tests/fixtures/m2-http/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path)
        .unwrap()
        .replace('\n', "\r\n")
        .into_bytes()
}

// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn sse_prefix_reaches_client_before_eof() {
    let sink = RecordingSink::new();
    let mut h = start(
        emitter_for(sink.clone()),
        policy(&[("/v1/responses", false, Some("openai-responses"))]),
    );
    let body = b"{\"stream\":true}";
    let req = request(
        "POST",
        "/v1/responses",
        &["Content-Type: application/json", &length(body)],
        body,
    );
    write(&mut h.client, &req).await;
    assert_eq!(read_exact(&mut h.upstream, req.len()).await, req);

    let events: [&[u8]; 3] = [
        b"event: response.created\ndata: {\"n\":1}\n\n",
        b"event: response.output_text.delta\ndata: {\"n\":2}\n\n",
        b"data: [DONE]\n\n",
    ];
    let mut first = response(
        "HTTP/1.1 200 OK",
        &[
            "Content-Type: text/event-stream; charset=utf-8",
            "Transfer-Encoding: chunked",
        ],
        b"",
    );
    first.extend(chunk(events[0]));
    write(&mut h.upstream, &first).await;
    // The head and the first event reach the client while the upstream has sent nothing more.
    assert_eq!(read_exact(&mut h.client, first.len()).await, first);
    // Capture already holds that event, before the body ends.
    let response_bytes = |sink: &RecordingSink| {
        streams(&sink.frames())
            .into_iter()
            .find(|v| v.source == WIRE_RESPONSE)
            .map(|v| (v.bytes, v.end))
    };
    wait_until("the first event is captured", || {
        response_bytes(&sink).is_some_and(|(bytes, _)| bytes == events[0])
    })
    .await;
    assert_eq!(
        response_bytes(&sink).unwrap().1,
        None,
        "not ended before the body is"
    );

    for event in &events[1..] {
        let c = chunk(event);
        write(&mut h.upstream, &c).await;
        assert_eq!(read_exact(&mut h.client, c.len()).await, c);
    }
    write(&mut h.upstream, b"0\r\n\r\n").await;
    assert_eq!(read_exact(&mut h.client, 5).await, b"0\r\n\r\n");
    close_both(h).await;

    let views = streams(&sink.frames());
    let resp = find(&views, WIRE_RESPONSE, 1);
    assert_eq!(
        resp.bytes,
        events.concat(),
        "decoded SSE bytes, chunk framing removed"
    );
    assert_eq!(resp.byte_frames[0].0, 0);
    assert_eq!(
        resp.byte_frames[0].1,
        events[0].len(),
        "the first event is its own frame"
    );
    let monos: Vec<u64> = resp.byte_frames.iter().map(|f| f.2).collect();
    assert!(
        monos.windows(2).all(|w| w[0] < w[1]),
        "observed as each read arrived: {monos:?}"
    );
    assert_eq!(resp.end, Some(EndReason::Complete));
    assert_eq!(resp.meta.status, Some(200));
    assert_eq!(resp.meta.content_type.as_deref(), Some("text/event-stream"));
    assert_eq!(resp.meta.protocol_hint.as_deref(), Some("openai-responses"));
    let req_view = find(&views, WIRE_REQUEST, 1);
    assert_eq!(req_view.bytes, body);
    assert!(req_view.open_at < resp.open_at);
}

#[tokio::test]
async fn framing_matrix_relays_unchanged_and_captures_decoded_bodies() {
    let sink = RecordingSink::new();
    let mut h = start(emitter_for(sink.clone()), policy(&[("/v1/", true, None)]));

    // 1: Content-Length.
    let json = b"{\"ok\":true}";
    round_trip(
        &mut h,
        &request("GET", "/v1/cl", &[], b""),
        &response(
            "HTTP/1.1 200 OK",
            &["Content-Type: application/json", &length(json)],
            json,
        ),
    )
    .await;
    // 2: chunked both ways, with a chunk extension and a trailer.
    let mut chunked_req = request("POST", "/v1/chunked", &["Transfer-Encoding: chunked"], b"");
    chunked_req.extend(chunk(b"hel"));
    chunked_req.extend(b"2;ext=1\r\nlo\r\n0\r\n\r\n");
    let mut chunked_resp = response("HTTP/1.1 200 OK", &["Transfer-Encoding: chunked"], b"");
    chunked_resp.extend(chunk(b"wor"));
    chunked_resp.extend(chunk(b"ld"));
    chunked_resp.extend(b"0\r\nX-Trailer: t\r\n\r\n");
    round_trip(&mut h, &chunked_req, &chunked_resp).await;
    // 3: HEAD: no body whatever the headers say.
    round_trip(
        &mut h,
        &request("HEAD", "/v1/head", &[], b""),
        &response(
            "HTTP/1.1 200 OK",
            &["Content-Length: 1000", "Content-Encoding: gzip"],
            b"",
        ),
    )
    .await;
    // 4, 5: 204 and 304 carry no body.
    round_trip(
        &mut h,
        &request("GET", "/v1/nocontent", &[], b""),
        &response("HTTP/1.1 204 No Content", &[], b""),
    )
    .await;
    round_trip(
        &mut h,
        &request("GET", "/v1/notmodified", &[], b""),
        &response("HTTP/1.1 304 Not Modified", &["Content-Length: 50"], b""),
    )
    .await;
    // 6: close-delimited: the body ends at EOF.
    let req6 = request("GET", "/v1/close", &[], b"");
    write(&mut h.client, &req6).await;
    assert_eq!(read_exact(&mut h.upstream, req6.len()).await, req6);
    let resp6 = response(
        "HTTP/1.1 200 OK",
        &["Content-Type: text/plain"],
        b"until-eof-body",
    );
    write(&mut h.upstream, &resp6).await;
    h.upstream.shutdown().await.unwrap();
    assert_eq!(
        read_to_end(&mut h.client).await,
        resp6,
        "the body, then EOF"
    );
    h.client.shutdown().await.unwrap();
    assert_eq!(read_to_end(&mut h.upstream).await, b"");
    assert!(matches!(
        finish(h.relay).await.unwrap(),
        HttpOutcome::Closed
    ));

    let views = streams(&sink.frames());
    // (method, path, request body, status, response body)
    type Expected<'a> = (&'a str, &'a str, &'a [u8], u16, &'a [u8]);
    let expect: [Expected; 6] = [
        ("GET", "/v1/cl", b"", 200, json),
        ("POST", "/v1/chunked", b"hello", 200, b"world"),
        ("HEAD", "/v1/head", b"", 200, b""),
        ("GET", "/v1/nocontent", b"", 204, b""),
        ("GET", "/v1/notmodified", b"", 304, b""),
        ("GET", "/v1/close", b"", 200, b"until-eof-body"),
    ];
    for (i, (method, path, req_body, status, resp_body)) in expect.into_iter().enumerate() {
        let exchange = i as u64 + 1;
        let req = find(&views, WIRE_REQUEST, exchange);
        let resp = find(&views, WIRE_RESPONSE, exchange);
        assert_eq!(req.meta.method.as_deref(), Some(method));
        assert_eq!(req.meta.path.as_deref(), Some(path));
        assert_eq!(req.meta.label, format!("{method} {path}"));
        assert_eq!(req.meta.host.as_deref(), Some(HOST));
        assert_eq!(req.bytes, req_body, "exchange {exchange} request body");
        assert_eq!(req.end, Some(EndReason::Complete));
        assert_eq!(resp.meta.status, Some(status));
        assert_eq!(resp.meta.path.as_deref(), Some(path));
        assert_eq!(resp.meta.label, format!("{status} {path}"));
        assert_eq!(resp.bytes, resp_body, "exchange {exchange} response body");
        assert_eq!(resp.end, Some(EndReason::Complete), "exchange {exchange}");
        assert!(resp.gaps.is_empty(), "exchange {exchange}: {:?}", resp.gaps);
        assert!(req.open_at < resp.open_at);
        assert_ne!(
            req.key, resp.key,
            "request and response are distinct streams"
        );
    }
    assert_eq!(views.len(), 12);
}

#[tokio::test]
async fn large_body_streams_in_bounded_contiguous_segments() {
    let sink = RecordingSink::new();
    let mut offered = None;
    let mut h = start_with(
        emitter_for(sink.clone()),
        policy(&[("/v1/big", false, None)]),
        1 << 20,
        |io| {
            let choppy = ChoppyIo::new(io, &[1 << 20]);
            offered = Some(choppy.max_offered.clone());
            Box::new(choppy)
        },
    );
    let body = noise(5 * 65536 + 123, 7);
    let req = request("GET", "/v1/big", &[], b"");
    write(&mut h.client, &req).await;
    assert_eq!(read_exact(&mut h.upstream, req.len()).await, req);
    let head = response("HTTP/1.1 200 OK", &[&length(&body)], b"");
    write(&mut h.upstream, &head).await;
    assert_eq!(read_exact(&mut h.client, head.len()).await, head);
    // Lockstep: each piece reaches the client before the next is sent, so the relay cannot be
    // holding the body back until it is complete.
    for piece in body.chunks(10_000) {
        write(&mut h.upstream, piece).await;
        assert_eq!(read_exact(&mut h.client, piece.len()).await, piece);
    }
    close_both(h).await;

    let views = streams(&sink.frames());
    let resp = find(&views, WIRE_RESPONSE, 1);
    assert_eq!(resp.bytes, body);
    assert_eq!(resp.end, Some(EndReason::Complete));
    assert!(
        resp.byte_frames.len() > 5,
        "{} frames",
        resp.byte_frames.len()
    );
    assert!(resp.byte_frames.iter().all(|f| f.1 <= MAX_SEGMENT_BYTES));
    let offered = offered.unwrap().load(Ordering::SeqCst);
    assert!(
        offered > 0 && offered <= MAX_SEGMENT_BYTES,
        "read size {offered}"
    );
}

#[tokio::test]
async fn chunk_framing_split_at_every_byte_ends_exactly() {
    let sink = RecordingSink::new();
    let mut h = start_with(
        emitter_for(sink.clone()),
        policy(&[("/v1/", true, None)]),
        1 << 20,
        |io| Box::new(ChoppyIo::new(io, &[1])),
    );
    let req1 = request("GET", "/v1/one", &[], b"");
    let req2 = request("GET", "/v1/two", &[], b"");
    write(&mut h.client, &[req1.clone(), req2.clone()].concat()).await;
    assert_eq!(
        read_exact(&mut h.upstream, req1.len() + req2.len()).await,
        [req1, req2].concat()
    );
    let data17 = b"seventeen bytes!!";
    let mut resp1 = response("HTTP/1.1 200 OK", &["Transfer-Encoding: chunked"], b"");
    resp1.extend(b"1\r\na\r\n");
    resp1.extend(b"11 ;name=\"quoted;value\"\r\n");
    resp1.extend(data17);
    resp1.extend(b"\r\n000A\r\n0123456789\r\n");
    resp1.extend(b"0\r\nTrailer-A: x\r\nTrailer-B: y\r\n\r\n");
    let resp2 = response("HTTP/1.1 200 OK", &["Content-Length: 2"], b"OK");
    let both = [resp1, resp2].concat();
    write(&mut h.upstream, &both).await;
    assert_eq!(read_exact(&mut h.client, both.len()).await, both);
    close_both(h).await;

    let views = streams(&sink.frames());
    let one = find(&views, WIRE_RESPONSE, 1);
    assert_eq!(one.bytes, [&b"a"[..], data17, b"0123456789"].concat());
    assert_eq!(one.end, Some(EndReason::Complete));
    let two = find(&views, WIRE_RESPONSE, 2);
    assert_eq!(
        two.bytes, b"OK",
        "the next message starts exactly after the trailers"
    );
    assert_eq!(two.end, Some(EndReason::Complete));
}

#[tokio::test]
async fn large_chunked_body_with_split_lines_keeps_contiguous_ranges() {
    let sink = RecordingSink::new();
    let mut offered = None;
    let mut h = start_with(
        emitter_for(sink.clone()),
        policy(&[("/v1/big", false, None)]),
        1 << 20,
        |io| {
            let choppy = ChoppyIo::new(io, &[1, 3, 7, 4096, 2, 65536, 5, 11]);
            offered = Some(choppy.max_offered.clone());
            Box::new(choppy)
        },
    );
    let sizes = [70_000, 1, 65_536, 100_000, 13, 30_000];
    let mut data = Vec::new();
    let mut resp = response("HTTP/1.1 200 OK", &["Transfer-Encoding: chunked"], b"");
    for (i, size) in sizes.into_iter().enumerate() {
        let piece = noise(size, i as u64 + 11);
        resp.extend(chunk(&piece));
        data.extend(piece);
    }
    resp.extend(b"0\r\n\r\n");
    let req = request("GET", "/v1/big", &[], b"");
    round_trip(&mut h, &req, &resp).await;
    close_both(h).await;

    let views = streams(&sink.frames());
    let body = find(&views, WIRE_RESPONSE, 1);
    assert!(data.len() > 4 * MAX_SEGMENT_BYTES);
    assert_eq!(body.bytes.len(), data.len());
    assert!(body.bytes == data, "decoded bytes differ");
    assert_eq!(body.end, Some(EndReason::Complete));
    let offered = offered.unwrap().load(Ordering::SeqCst);
    assert!(offered <= MAX_SEGMENT_BYTES, "read size {offered}");
}

/// A response whose body is `coded`, sent with `headers`; returns the response stream.
async fn capture_coded_response(headers: &[&str], coded: &[u8]) -> StreamView {
    capture_coded_response_in(headers, coded, &[usize::MAX]).await
}

/// As [`capture_coded_response`], with the relay reading the upstream in pieces of at most
/// `reads` bytes, in turn.
async fn capture_coded_response_in(headers: &[&str], coded: &[u8], reads: &[usize]) -> StreamView {
    let sink = RecordingSink::new();
    let mut h = start_with(
        emitter_for(sink.clone()),
        policy(&[("/v1/", true, None)]),
        1 << 20,
        |io| Box::new(ChoppyIo::new(io, reads)),
    );
    let resp = response("HTTP/1.1 200 OK", headers, coded);
    round_trip(&mut h, &request("GET", "/v1/coded", &[], b""), &resp).await;
    close_both(h).await;
    find(&streams(&sink.frames()), WIRE_RESPONSE, 1).clone()
}

#[tokio::test]
async fn late_decode_failure_keeps_partial_prefix_only() {
    let plain = b"data: the part that decodes before the stream goes bad\n\n".repeat(20);
    let garbage = [0xffu8, 0xff, 0x00, 0x13, 0x37, 0xde, 0xad];
    for (coding, prefix) in [
        ("gzip", gzip_flushed_prefix(&plain)),
        ("deflate", zlib_flushed_prefix(&plain)),
    ] {
        let coded = [prefix.clone(), garbage.to_vec()].concat();
        let view = capture_coded_response(
            &[&format!("Content-Encoding: {coding}"), &length(&coded)],
            &coded,
        )
        .await;
        println!(
            "late-decode {coding}: {} coded bytes relayed; captured {} decoded bytes; gaps {:?}; end {:?}",
            coded.len(),
            view.bytes.len(),
            view.gaps,
            view.end
        );
        assert_eq!(view.bytes, plain, "{coding}: exactly the decoded prefix");
        assert_eq!(view.gaps, vec![GapReason::DecodeFailed], "{coding}");
        assert_eq!(view.end, Some(EndReason::DecodeError), "{coding}");
        assert!(
            !contains(&view.bytes, &garbage[..3]),
            "{coding}: no suffix bytes"
        );
        assert!(
            !contains(&view.bytes, &prefix[..8]),
            "{coding}: no compressed bytes"
        );
    }
}

#[tokio::test]
async fn transport_codings_are_decoded_on_the_capture_side_only() {
    let plain = b"{\"output\":\"a compressible body, a compressible body\"}".repeat(40);
    let cases: Vec<(&str, Vec<&str>, Vec<u8>)> = vec![
        ("gzip", vec!["Content-Encoding: gzip"], gzip(&plain)),
        ("x-gzip", vec!["Content-Encoding: x-gzip"], gzip(&plain)),
        (
            "zlib deflate",
            vec!["Content-Encoding: deflate"],
            zlib(&plain),
        ),
        (
            "raw deflate",
            vec!["Content-Encoding: deflate"],
            raw_deflate(&plain),
        ),
        (
            "identity",
            vec!["Content-Encoding: identity"],
            plain.clone(),
        ),
        (
            "two gzip members",
            vec!["Content-Encoding: gzip"],
            [gzip(&plain[..100]), gzip(&plain[100..])].concat(),
        ),
    ];
    for (name, mut headers, coded) in cases {
        let len = length(&coded);
        headers.push(&len);
        let view = capture_coded_response(&headers, &coded).await;
        assert!(view.bytes == plain, "{name}: decoded body differs");
        assert!(view.gaps.is_empty(), "{name}: {:?}", view.gaps);
        assert_eq!(view.end, Some(EndReason::Complete), "{name}");
    }

    // Chunked framing around gzip, and gzip as a transfer coding before chunked.
    let coded = gzip(&plain);
    let mut chunked = Vec::new();
    for piece in coded.chunks(37) {
        chunked.extend(chunk(piece));
    }
    chunked.extend(b"0\r\n\r\n");
    for te in [
        vec!["Content-Encoding: gzip", "Transfer-Encoding: chunked"],
        vec!["Transfer-Encoding: gzip, chunked"],
    ] {
        let view = capture_coded_response(&te, &chunked).await;
        assert!(view.bytes == plain, "{te:?}: decoded body differs");
        assert_eq!(view.end, Some(EndReason::Complete), "{te:?}");
    }

    // A gzip body that stops before its trailer keeps everything it decoded, and says so.
    let truncated = &coded[..coded.len() - 8];
    let view =
        capture_coded_response(&["Content-Encoding: gzip", &length(truncated)], truncated).await;
    assert!(view.bytes == plain, "the decoded part is kept");
    assert_eq!(view.gaps, vec![GapReason::DecodeFailed]);
    assert_eq!(view.end, Some(EndReason::DecodeError));

    // A corrupted trailer is a decode failure too.
    let mut bad_crc = coded.clone();
    let at = bad_crc.len() - 8;
    bad_crc[at] ^= 0xff;
    let view =
        capture_coded_response(&["Content-Encoding: gzip", &length(&bad_crc)], &bad_crc).await;
    assert_eq!(view.gaps, vec![GapReason::DecodeFailed]);
    assert_eq!(view.end, Some(EndReason::DecodeError));
}

/// The body `tests/fixtures/m2-http/sse-body.br` holds, brotli-coded by the reference encoder
/// (`brotli -q 11`): a streamed reply's shape, long enough for several decoded pieces.
fn sse_body() -> Vec<u8> {
    (0..2000)
        .map(|i| {
            format!(
                "event: content_block_delta\ndata: {{\"index\":{i},\"delta\":{{\"text\":\"token {i}\"}}}}\n\n"
            )
        })
        .collect::<String>()
        .into_bytes()
}

#[tokio::test]
async fn brotli_and_zstd_replies_decode_with_no_gap() {
    let plain = sse_body();
    let br = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2-http/sse-body.br"),
    )
    .unwrap();
    let zstd_one = zstd::encode_all(plain.as_slice(), 3).unwrap();
    let zstd_two = [
        zstd::encode_all(&plain[..1000], 3).unwrap(),
        zstd::encode_all(&plain[1000..], 19).unwrap(),
    ]
    .concat();
    let mut chunked = Vec::new();
    for piece in br.chunks(37) {
        chunked.extend(chunk(piece));
    }
    chunked.extend(b"0\r\n\r\n");
    let cases: Vec<(&str, Vec<String>, &[u8])> = vec![
        ("br", vec!["Content-Encoding: br".into(), length(&br)], &br),
        (
            "br, chunked",
            vec![
                "Content-Encoding: br".into(),
                "Transfer-Encoding: chunked".into(),
            ],
            &chunked,
        ),
        (
            "zstd",
            vec!["Content-Encoding: zstd".into(), length(&zstd_one)],
            &zstd_one,
        ),
        (
            "zstd, two frames",
            vec!["Content-Encoding: zstd".into(), length(&zstd_two)],
            &zstd_two,
        ),
    ];
    for (name, headers, coded) in cases {
        let headers: Vec<&str> = headers.iter().map(String::as_str).collect();
        let view = capture_coded_response(&headers, coded).await;
        assert!(view.bytes == plain, "{name}: decoded body differs");
        assert!(view.gaps.is_empty(), "{name}: {:?}", view.gaps);
        assert_eq!(view.end, Some(EndReason::Complete), "{name}");
    }

    // Cut short, either keeps what it decoded and says the rest failed to decode.
    for (name, coded) in [
        ("br", &br[..br.len() / 2]),
        ("zstd", &zstd_one[..zstd_one.len() / 2]),
    ] {
        let headers = [&format!("Content-Encoding: {name}") as &str, &length(coded)];
        let view = capture_coded_response(&headers, coded).await;
        assert!(
            plain.starts_with(&view.bytes),
            "{name}: only a decoded prefix"
        );
        assert_eq!(view.gaps, vec![GapReason::DecodeFailed], "{name}");
        assert_eq!(view.end, Some(EndReason::DecodeError), "{name}");
    }
}

/// `data` as a brotli stream of uncompressed meta-blocks (RFC 7932 sections 9.1 and 9.2),
/// written by hand: no brotli encoder is in the build, and this one shares nothing with the
/// decoder.
fn brotli_stored(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut bits, mut used) = (0u64, 0u32);
    let mut put = |out: &mut Vec<u8>, value: u64, n: u32, pad: bool| {
        bits |= value << used;
        used += n;
        while used >= 8 || (pad && used > 0) {
            out.push(bits as u8);
            bits >>= 8;
            used = used.saturating_sub(8);
        }
    };
    put(&mut out, 0, 1, false); // WBITS: 16
    for piece in data.chunks(1 << 16) {
        // ISLAST 0, MNIBBLES 4, MLEN - 1, ISUNCOMPRESSED 1, then the bytes from a byte boundary.
        put(&mut out, 0, 3, false);
        put(&mut out, (piece.len() - 1) as u64, 16, false);
        put(&mut out, 1, 1, true);
        out.extend_from_slice(piece);
    }
    put(&mut out, 0b11, 2, true); // ISLAST 1, ISLASTEMPTY 1
    out
}

/// Review P4 M1: a reply whose decoded size is a whole number of the decoder's 32 KiB output
/// pieces ends complete, with no gap, whether its coded bytes arrive in one read or in many.
/// zstd once lost track of its finished frame there and called the reply undecodable. Only the
/// zstd cases guard that fix; the brotli case passes without it and guards brotli regressions.
/// A zstd frame cut short at that size must still be a gap: see
/// `a_zstd_frame_cut_after_whole_pieces_is_still_a_gap`. (A stream cut exactly between two whole
/// frames still reads as complete. That is inherent to concatenated frames; the HTTP framing,
/// Content-Length or the chunked end, is the backstop there.)
#[tokio::test]
async fn a_reply_that_decodes_to_whole_32_kib_pieces_has_no_gap() {
    let body = sse_body();
    for size in [32_768, 65_536, 98_304, 40_000] {
        let plain = &body[..size];
        let zstd_one = zstd::encode_all(plain, 3).unwrap();
        // The first frame ends on the boundary, and another follows it.
        let zstd_two = [
            zstd::encode_all(&plain[..32_768], 3).unwrap(),
            zstd::encode_all(&plain[32_768..], 3).unwrap(),
        ]
        .concat();
        // Brotli never had this bug: its case passes without the zstd fix and only guards
        // brotli regressions at the same sizes.
        let br = brotli_stored(plain);
        let cases = [
            ("zstd", "zstd", &zstd_one),
            ("zstd, two frames", "zstd", &zstd_two),
            ("brotli regression guard", "br", &br),
        ];
        for (name, coding, coded) in cases {
            for reads in [&[usize::MAX][..], &[7, 4096, 1000]] {
                let headers = [
                    &format!("Content-Encoding: {coding}") as &str,
                    &length(coded),
                ];
                let view = capture_coded_response_in(&headers, coded, reads).await;
                let case = format!("{name}, {size} bytes, reads {reads:?}");
                assert!(view.bytes == plain, "{case}: decoded body differs");
                assert!(view.gaps.is_empty(), "{case}: {:?}", view.gaps);
                assert_eq!(view.end, Some(EndReason::Complete), "{case}");
            }
        }

        // Not a guard for the fix: these bodies are one zstd block, so cutting 3 bytes decodes
        // nothing and the decoder never holds whole pieces with an unfinished frame. It only
        // checks that a truncated small frame is a gap. The boundary the fix decides is covered
        // by the next test.
        let cut = &zstd_one[..zstd_one.len() - 3];
        let view = capture_coded_response(&["Content-Encoding: zstd", &length(cut)], cut).await;
        assert!(
            plain.starts_with(&view.bytes),
            "{size}: only a decoded prefix"
        );
        assert_eq!(view.gaps, vec![GapReason::DecodeFailed], "{size}");
        assert_eq!(view.end, Some(EndReason::DecodeError), "{size}");
    }
}

/// Where the first zstd block of `frame` ends (RFC 8878 sections 3.1.1.1 and 3.1.1.2),
/// parsed by hand: the frame header, then the first 3-byte block header.
fn first_zstd_block_end(frame: &[u8]) -> usize {
    assert_eq!(frame[..4], [0x28, 0xb5, 0x2f, 0xfd], "a zstd frame");
    let descriptor = frame[4];
    let single_segment = descriptor & 0x20 != 0;
    let dictionary = [0, 1, 2, 4][usize::from(descriptor & 3)];
    let content_size = match descriptor >> 6 {
        0 => usize::from(single_segment),
        1 => 2,
        2 => 4,
        _ => 8,
    };
    let at = 5 + usize::from(!single_segment) + dictionary + content_size;
    let header = u32::from_le_bytes([frame[at], frame[at + 1], frame[at + 2], 0]);
    let size = (header >> 3) as usize;
    let end = at + 3 + if (header >> 1) & 3 == 1 { 1 } else { size };
    assert!(header & 1 == 0, "more than one block expected");
    end
}

/// The state the zstd fix decides: the decoder has handed back a whole 32 KiB piece with no coded
/// input left, and the frame has not ended. Such a reply is a gap, never complete. Two ways to
/// get there: a frame cut at the end of a block, and a frame that decoded fully but lost its
/// 4-byte content checksum.
///
/// The body must compress well. The relay reads at most 16 KiB at a time, so a body that does not
/// compress (noise) never fills a 32 KiB piece from a read's last bytes, and passes without the
/// fix too.
#[tokio::test]
async fn a_zstd_frame_cut_after_whole_pieces_is_still_a_gap() {
    // Several 128 KiB blocks, each a whole number of 32 KiB pieces.
    let plain: Vec<u8> = (0..5000)
        .map(|i| format!("event: delta\ndata: {{\"index\":{i},\"text\":\"token {i}\"}}\n\n"))
        .collect::<String>()
        .into_bytes();
    assert!(plain.len() > 131_072 + 32_768, "more than one block");
    let frame = zstd::encode_all(plain.as_slice(), 3).unwrap();
    let first = first_zstd_block_end(&frame);
    assert!(first < frame.len(), "the cut is inside the frame");
    for reads in [&[usize::MAX][..], &[7, 4096, 1000]] {
        let cut = &frame[..first];
        let headers = ["Content-Encoding: zstd", &length(cut)];
        let view = capture_coded_response_in(&headers, cut, reads).await;
        let case = format!("cut at the first block, reads {reads:?}");
        assert_eq!(view.bytes.len(), 131_072, "{case}: the block decodes");
        assert!(
            plain.starts_with(&view.bytes),
            "{case}: only a decoded prefix"
        );
        assert_eq!(view.gaps, vec![GapReason::DecodeFailed], "{case}");
        assert_eq!(view.end, Some(EndReason::DecodeError), "{case}");
    }

    // Written with a content checksum, then cut so only the 4 checksum bytes are missing:
    // everything decodes, and the frame has still not ended.
    for size in [32_768, 40_000, 131_072] {
        let plain = &plain[..size];
        let mut encoder = zstd::stream::Encoder::new(Vec::new(), 3).unwrap();
        encoder.include_checksum(true).unwrap();
        io::Write::write_all(&mut encoder, plain).unwrap();
        let whole = encoder.finish().unwrap();
        for reads in [&[usize::MAX][..], &[7, 4096, 1000]] {
            let headers = ["Content-Encoding: zstd", &length(&whole)];
            let view = capture_coded_response_in(&headers, &whole, reads).await;
            let case = format!("checksummed, {size} bytes, reads {reads:?}");
            assert!(view.bytes == plain, "{case}: the whole frame decodes");
            assert!(view.gaps.is_empty(), "{case}: {:?}", view.gaps);
            assert_eq!(view.end, Some(EndReason::Complete), "{case}");

            let cut = &whole[..whole.len() - 4];
            let headers = ["Content-Encoding: zstd", &length(cut)];
            let view = capture_coded_response_in(&headers, cut, reads).await;
            let case = format!("checksum cut off, {size} bytes, reads {reads:?}");
            assert!(view.bytes == plain, "{case}: everything decoded is kept");
            assert_eq!(view.gaps, vec![GapReason::DecodeFailed], "{case}");
            assert_eq!(view.end, Some(EndReason::DecodeError), "{case}");
        }
    }
}

#[tokio::test]
async fn unsupported_encoding_yields_a_named_gap_and_no_body_bytes() {
    let opaque = noise(3000, 5);
    for coding in ["gzip, br", "br, zstd", "compress", "x-unknown"] {
        let view = capture_coded_response(
            &[&format!("Content-Encoding: {coding}"), &length(&opaque)],
            &opaque,
        )
        .await;
        assert!(view.bytes.is_empty(), "{coding}: no opaque fallback");
        assert_eq!(view.gaps, vec![GapReason::UnsupportedEncoding], "{coding}");
        assert_eq!(view.end, Some(EndReason::DecodeError), "{coding}");
        assert_eq!(
            view.meta.status,
            Some(200),
            "{coding}: the metadata is still retained"
        );
    }

    // The same on a request body.
    let sink = RecordingSink::new();
    let mut h = start(emitter_for(sink.clone()), policy(&[("/v1/", true, None)]));
    let req = request(
        "POST",
        "/v1/responses",
        &["Content-Encoding: compress", &length(&opaque)],
        &opaque,
    );
    round_trip(
        &mut h,
        &req,
        &response("HTTP/1.1 200 OK", &["Content-Length: 0"], b""),
    )
    .await;
    close_both(h).await;
    let views = streams(&sink.frames());
    let req_view = find(&views, WIRE_REQUEST, 1);
    assert!(req_view.bytes.is_empty());
    assert_eq!(req_view.gaps, vec![GapReason::UnsupportedEncoding]);
    assert_eq!(
        find(&views, WIRE_RESPONSE, 1).end,
        Some(EndReason::Complete)
    );
}

#[tokio::test]
async fn selected_upload_bytes_use_the_ordinary_capture_path() {
    let sink = RecordingSink::new();
    let mut h = start(
        emitter_for(sink.clone()),
        policy(&[("/v1/files", true, Some("openai-files"))]),
    );
    let mut upload = b"--XYZ\r\nContent-Disposition: form-data; name=\"file\"\r\n\r\n".to_vec();
    upload.extend(noise(100_000, 3));
    upload.extend([0, 0, 0, 13, 10]);
    upload.extend(b"\r\n--XYZ--\r\n");
    let req = request(
        "POST",
        "/v1/files/upload?purpose=assistants",
        &[
            "Content-Type: multipart/form-data; boundary=XYZ",
            &length(&upload),
        ],
        &upload,
    );
    round_trip(
        &mut h,
        &req,
        &response("HTTP/1.1 201 Created", &["Content-Length: 2"], b"{}"),
    )
    .await;
    close_both(h).await;
    let views = streams(&sink.frames());
    let req_view = find(&views, WIRE_REQUEST, 1);
    assert!(req_view.bytes == upload, "upload bytes differ");
    assert_eq!(req_view.end, Some(EndReason::Complete));
    assert_eq!(
        req_view.meta.content_type.as_deref(),
        Some("multipart/form-data")
    );
    assert_eq!(req_view.meta.path.as_deref(), Some("/v1/files/upload"));
    assert_eq!(req_view.meta.protocol_hint.as_deref(), Some("openai-files"));
}

#[tokio::test]
async fn credentials_reach_upstream_but_never_capture() {
    let sink = RecordingSink::new();
    let mut h = start(
        emitter_for(sink.clone()),
        policy(&[("/v1/responses", false, Some("openai-responses"))]),
    );
    let head = fixture("sentinel-request.http");
    let req = [head.clone(), b"{}".to_vec()].concat();
    write(&mut h.client, &req).await;
    let upstream_saw = read_exact(&mut h.upstream, req.len()).await;
    assert_eq!(
        upstream_saw, req,
        "forwarded unchanged, credentials included"
    );
    assert!(contains(
        &upstream_saw,
        b"Authorization: Bearer SENTINEL-AUTH-1b2c3d"
    ));
    let resp = [fixture("sentinel-response.http"), b"{}".to_vec()].concat();
    write(&mut h.upstream, &resp).await;
    assert_eq!(read_exact(&mut h.client, resp.len()).await, resp);

    // The same credentials on a request that breaks mid-body: the error names none of them.
    let broken = String::from_utf8(head)
        .unwrap()
        .replace("Content-Length: 2", "Content-Length: 50");
    write(&mut h.client, broken.as_bytes()).await;
    write(&mut h.client, b"{\"partial\":").await;
    h.client.shutdown().await.unwrap();
    let mut rest = Vec::new();
    h.upstream.read_to_end(&mut rest).await.unwrap();
    assert_eq!(rest, [broken.as_bytes(), b"{\"partial\":"].concat());
    let error = finish(h.relay).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);

    let frames = sink.frames();
    let mut evidence = format!("{error} {error:?} {:?}", h.emitter);
    for frame in &frames {
        evidence.push_str(&serde_json::to_string(frame).unwrap());
        evidence.push_str(&format!("{frame:?}"));
    }
    assert!(
        !evidence.contains("SENTINEL"),
        "a sentinel reached evidence: {evidence}"
    );
    assert!(!evidence.contains("sig="), "query text reached evidence");

    let views = streams(&frames);
    let req_view = find(&views, WIRE_REQUEST, 1);
    assert_eq!(req_view.meta.path.as_deref(), Some("/v1/responses"));
    assert_eq!(req_view.meta.label, "POST /v1/responses");
    assert_eq!(
        req_view.meta.content_type.as_deref(),
        Some("application/json")
    );
    assert_eq!(req_view.bytes, b"{}");
    assert_eq!(find(&views, WIRE_RESPONSE, 1).bytes, b"{}");
    let broken_view = find(&views, WIRE_REQUEST, 2);
    assert_eq!(broken_view.bytes, b"{\"partial\":");
    assert_eq!(broken_view.end, Some(EndReason::Interrupted));
}

#[tokio::test]
async fn capture_disabled_does_not_block_forwarding() {
    let sink = Arc::new(DisabledSink::default());
    let h = start_with(
        refused_emitter(sink.clone()),
        policy(&[("/v1/", true, None)]),
        8 * 1024,
        |io| Box::new(io),
    );
    let bodies: Vec<Vec<u8>> = (0..3).map(|i| noise(256 * 1024, 100 + i)).collect();
    let requests: Vec<u8> = bodies
        .iter()
        .enumerate()
        .flat_map(|(i, b)| request("POST", &format!("/v1/upload/{i}"), &[&length(b)], b))
        .collect();
    let responses: Vec<u8> = bodies
        .iter()
        .rev()
        .flat_map(|b| response("HTTP/1.1 200 OK", &[&length(b)], b))
        .collect();
    let (mut client_read, mut client_write) = tokio::io::split(h.client);
    let (mut upstream_read, mut upstream_write) = tokio::io::split(h.upstream);
    let (started, begin) = tokio::sync::oneshot::channel::<()>();
    let everything = async {
        tokio::join!(
            async {
                client_write.write_all(&requests).await.unwrap();
                client_write.shutdown().await.unwrap();
            },
            async {
                // Answer as soon as the first request head is in, while requests still stream.
                let mut first = vec![0u8; 64];
                upstream_read.read_exact(&mut first).await.unwrap();
                started.send(()).unwrap();
                let mut rest = Vec::new();
                upstream_read.read_to_end(&mut rest).await.unwrap();
                [first, rest].concat()
            },
            async {
                begin.await.unwrap();
                upstream_write.write_all(&responses).await.unwrap();
                upstream_write.shutdown().await.unwrap();
            },
            async {
                let mut got = Vec::new();
                client_read.read_to_end(&mut got).await.unwrap();
                got
            },
        )
    };
    let ((), upstream_got, (), client_got) = tokio::time::timeout(WAIT, everything)
        .await
        .expect("forwarding waited on capture");
    assert!(upstream_got == requests, "requests changed in transit");
    assert!(client_got == responses, "responses changed in transit");
    assert!(matches!(
        finish(h.relay).await.unwrap(),
        HttpOutcome::Closed
    ));
    println!(
        "capture-disabled: relayed {} request bytes and {} response bytes; sink refused {} submissions, {} recovery polls",
        requests.len(),
        responses.len(),
        sink.submits.load(Ordering::SeqCst),
        sink.admissions.load(Ordering::SeqCst)
    );
    assert!(
        sink.submits.load(Ordering::SeqCst) >= 1,
        "capture was attempted"
    );
    assert!(
        sink.admissions.load(Ordering::SeqCst) >= 1,
        "recovery was polled, not awaited"
    );
}

#[tokio::test]
async fn queue_pressure_recovers_at_next_http_request() {
    let sink = RecordingSink::new();
    let mut h = start(
        emitter_for(sink.clone()),
        policy(&[("/v1/responses", false, Some("openai-responses"))]),
    );
    let (part1, part2, part3) = (
        &b"{\"input\":\"part one, "[..],
        &b"part two, "[..],
        &b"part three\"}"[..],
    );
    let whole = [part1, part2, part3].concat();
    let head1 = request("POST", "/v1/responses", &[&length(&whole)], b"");
    write(&mut h.client, &[&head1[..], part1].concat()).await;
    read_exact(&mut h.upstream, head1.len() + part1.len()).await;
    wait_until("the first part is captured", || {
        streams(&sink.frames())
            .iter()
            .any(|v| v.source == WIRE_REQUEST && v.bytes == part1)
    })
    .await;

    // Pressure during the body: the rest of it is forwarded, not captured.
    sink.set_pressure(true);
    write(&mut h.client, part2).await;
    assert_eq!(read_exact(&mut h.upstream, part2.len()).await, part2);
    wait_until("the sink refused", || sink.refused() >= 1).await;

    // Pressure is released while the body is still arriving. The rest of this body is not a
    // boundary: no recovery is attempted inside it and its suffix is never captured.
    sink.set_pressure(false);
    write(&mut h.client, part3).await;
    assert_eq!(read_exact(&mut h.upstream, part3.len()).await, part3);
    let no_recovery = |sink: &RecordingSink| {
        !sink
            .frames()
            .iter()
            .any(|f| matches!(f.event, FrameEvent::Recovery { .. }))
    };
    assert!(no_recovery(&sink), "recovered inside a body");
    assert_eq!(
        sink.begun().len(),
        1,
        "an epoch was requested inside a body"
    );
    assert!(
        streams(&sink.frames())
            .iter()
            .all(|v| !contains(&v.bytes, part3)),
        "the suffix of an interrupted body was captured"
    );

    // A response to an uncaptured request is not a safe boundary either: nothing recovers
    // there and the response is not captured.
    let resp1 = response("HTTP/1.1 200 OK", &["Content-Length: 8"], b"resp-one");
    write(&mut h.upstream, &resp1).await;
    assert_eq!(read_exact(&mut h.client, resp1.len()).await, resp1);
    assert_quiet(&mut h.client).await;
    assert!(no_recovery(&sink), "recovered at a response");
    assert_eq!(
        sink.begun(),
        vec![PRODUCER.to_string()],
        "no epoch requested at a response"
    );

    // The next newly begun request is captured under a fresh epoch.
    let body2 = b"{\"input\":\"again\"}";
    let req2 = request("POST", "/v1/responses", &[&length(body2)], body2);
    let resp2 = response("HTTP/1.1 200 OK", &["Content-Length: 8"], b"resp-two");
    round_trip(&mut h, &req2, &resp2).await;
    close_both(h).await;

    let frames = sink.frames();
    let views = streams(&frames);
    let lost = find(&views, WIRE_REQUEST, 1);
    assert_eq!(lost.key.producer, PRODUCER);
    assert_eq!(
        lost.bytes, part1,
        "only what was captured before the pressure"
    );
    assert_eq!(
        lost.end, None,
        "the lost body is not resumed or ended by the relay"
    );
    assert!(
        !has(&views, WIRE_RESPONSE, 1),
        "the response to an uncaptured request"
    );

    let begun = sink.begun();
    assert_eq!(begun.len(), 2);
    let fresh = &begun[1];
    assert_ne!(fresh, PRODUCER);
    let at = frames
        .iter()
        .position(|f| matches!(f.event, FrameEvent::Recovery { .. }))
        .expect("a recovery");
    assert_eq!(
        frames[at].event,
        FrameEvent::Recovery {
            previous_producer: PRODUCER.to_string(),
            reason: GapReason::QueueFull
        }
    );
    assert_eq!((&frames[at].stream.producer, frames[at].seq), (fresh, 0));
    for exchange_stream in [
        find(&views, WIRE_REQUEST, 2),
        find(&views, WIRE_RESPONSE, 2),
    ] {
        assert_eq!(&exchange_stream.key.producer, fresh);
        assert!(exchange_stream.open_at > at);
        assert_eq!(exchange_stream.end, Some(EndReason::Complete));
    }
    assert_eq!(find(&views, WIRE_REQUEST, 2).bytes, body2);
    assert_eq!(find(&views, WIRE_RESPONSE, 2).bytes, b"resp-two");
}

#[tokio::test]
async fn http_recovery_keeps_connection_exchange_identity() {
    let sink = RecordingSink::new();
    let mut h = start(emitter_for(sink.clone()), policy(&[("/v1/", true, None)]));
    // Exchange 1: captured whole.
    round_trip(
        &mut h,
        &request("POST", "/v1/one", &["Content-Length: 3"], b"one"),
        &response("HTTP/1.1 200 OK", &["Content-Length: 5"], b"uno!!"),
    )
    .await;
    // Exchange 2: capture is lost inside the response body.
    let req2 = request("POST", "/v1/two", &["Content-Length: 3"], b"two");
    write(&mut h.client, &req2).await;
    read_exact(&mut h.upstream, req2.len()).await;
    let mut first = response("HTTP/1.1 200 OK", &["Transfer-Encoding: chunked"], b"");
    first.extend(chunk(b"dos-part-1"));
    write(&mut h.upstream, &first).await;
    read_exact(&mut h.client, first.len()).await;
    wait_until("the first part is captured", || {
        streams(&sink.frames())
            .iter()
            .any(|v| v.source == WIRE_RESPONSE && v.bytes == b"dos-part-1")
    })
    .await;
    sink.set_pressure(true);
    let second = chunk(b"dos-part-2");
    write(&mut h.upstream, &second).await;
    assert_eq!(read_exact(&mut h.client, second.len()).await, second);
    wait_until("the sink refused", || sink.refused() >= 1).await;
    sink.set_pressure(false);
    write(&mut h.upstream, b"0\r\n\r\n").await;
    read_exact(&mut h.client, 5).await;
    // Exchange 3: a new request, a new epoch, the same connection.
    round_trip(
        &mut h,
        &request("POST", "/v1/three", &["Content-Length: 5"], b"three"),
        &response("HTTP/1.1 200 OK", &["Content-Length: 4"], b"tres"),
    )
    .await;
    close_both(h).await;

    let frames = sink.frames();
    let views = streams(&frames);
    let fresh = sink.begun()[1].clone();
    let recovery = frames
        .iter()
        .position(|f| matches!(f.event, FrameEvent::Recovery { .. }))
        .expect("a recovery");
    assert_eq!(frames[recovery].stream.producer, fresh);
    // Ordered controls first: Recovery, then this connection re-observed as already open.
    assert_eq!(
        frames[recovery + 1].event,
        FrameEvent::Connection {
            connection: CONN.to_string(),
            event: ConnectionEvent::Open {
                alpn: Some("http/1.1".into()),
                already_open: true
            }
        }
    );
    assert_eq!(frames[recovery + 1].seq, 1);

    for (exchange, producer) in [(1, PRODUCER), (2, PRODUCER), (3, fresh.as_str())] {
        let req = find(&views, WIRE_REQUEST, exchange);
        assert_eq!(req.key.producer, producer, "exchange {exchange}");
        assert_eq!(req.end, Some(EndReason::Complete));
        let wire = req.meta.wire.clone().unwrap();
        assert_eq!(
            wire,
            WireIdentity {
                connection: CONN.to_string(),
                exchange,
                message_index: None
            }
        );
    }
    let lost = find(&views, WIRE_RESPONSE, 2);
    assert_eq!(lost.bytes, b"dos-part-1");
    assert_eq!(lost.end, None);
    let three = find(&views, WIRE_RESPONSE, 3);
    assert_eq!(three.key.producer, fresh);
    assert_eq!(three.bytes, b"tres");
    // Stream and sequence numbering restart with the epoch; identity does not.
    assert_eq!(find(&views, WIRE_REQUEST, 3).key.stream, 1);
    assert!(find(&views, WIRE_REQUEST, 3).open_at > recovery + 1);
    // Recorder-only recovery: nothing but observation controls and payload in the new epoch.
    for frame in frames.iter().filter(|f| f.stream.producer == fresh) {
        assert!(
            matches!(
                frame.event,
                FrameEvent::Recovery { .. }
                    | FrameEvent::Connection { .. }
                    | FrameEvent::Open { .. }
                    | FrameEvent::Bytes { .. }
                    | FrameEvent::End { .. }
            ),
            "{frame:?}"
        );
    }
    assert_eq!(
        frames
            .iter()
            .filter(|f| matches!(f.event, FrameEvent::Recovery { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn pipelined_requests_and_informational_responses_keep_their_exchange() {
    let sink = RecordingSink::new();
    let mut h = start(emitter_for(sink.clone()), policy(&[("/v1/", true, None)]));
    let reqs = [
        request("GET", "/v1/a", &[], b""),
        request(
            "POST",
            "/v1/b",
            &["Content-Length: 3", "Expect: 100-continue"],
            b"abc",
        ),
        request("GET", "/v1/c?x=1", &[], b""),
    ]
    .concat();
    write(&mut h.client, &reqs).await;
    assert_eq!(read_exact(&mut h.upstream, reqs.len()).await, reqs);
    let resps = [
        response(
            "HTTP/1.1 103 Early Hints",
            &["Link: </style.css>; rel=preload"],
            b"",
        ),
        response("HTTP/1.1 200 OK", &["Content-Length: 1"], b"A"),
        response("HTTP/1.1 100 Continue", &[], b""),
        response("HTTP/1.1 201 Created", &["Content-Length: 2"], b"BB"),
        response("HTTP/1.1 200 OK", &["Content-Length: 3"], b"CCC"),
    ]
    .concat();
    write(&mut h.upstream, &resps).await;
    assert_eq!(read_exact(&mut h.client, resps.len()).await, resps);
    close_both(h).await;

    let views = streams(&sink.frames());
    assert_eq!(views.len(), 6, "interim responses open no stream");
    for (exchange, path, req_body, status, resp_body) in [
        (1, "/v1/a", &b""[..], 200, &b"A"[..]),
        (2, "/v1/b", b"abc", 201, b"BB"),
        (3, "/v1/c", b"", 200, b"CCC"),
    ] {
        let req = find(&views, WIRE_REQUEST, exchange);
        let resp = find(&views, WIRE_RESPONSE, exchange);
        assert_eq!(req.meta.path.as_deref(), Some(path));
        assert_eq!(req.bytes, req_body);
        assert_eq!(resp.meta.label, format!("{status} {path}"));
        assert_eq!(resp.meta.status, Some(status));
        assert_eq!(resp.bytes, resp_body);
        assert_eq!(resp.end, Some(EndReason::Complete));
    }
}

#[tokio::test]
async fn eof_inside_a_body_is_unexpected_eof_and_interrupted() {
    // Upstream closes inside a Content-Length body.
    let sink = RecordingSink::new();
    let mut h = start(emitter_for(sink.clone()), policy(&[("/v1/", true, None)]));
    let req = request("GET", "/v1/x", &[], b"");
    write(&mut h.client, &req).await;
    read_exact(&mut h.upstream, req.len()).await;
    let partial = response("HTTP/1.1 200 OK", &["Content-Length: 100"], b"only-ten!!");
    write(&mut h.upstream, &partial).await;
    drop(h.upstream);
    assert_eq!(
        read_to_end(&mut h.client).await,
        partial,
        "what arrived, then EOF"
    );
    let error = finish(h.relay).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    let views = streams(&sink.frames());
    let resp = find(&views, WIRE_RESPONSE, 1);
    assert_eq!(resp.bytes, b"only-ten!!");
    assert_eq!(resp.end, Some(EndReason::Interrupted));

    // The client closes inside a chunk.
    let sink = RecordingSink::new();
    let mut h = start(emitter_for(sink.clone()), policy(&[("/v1/", true, None)]));
    let mut req = request("POST", "/v1/y", &["Transfer-Encoding: chunked"], b"");
    req.extend(b"a\r\nhalf");
    write(&mut h.client, &req).await;
    h.client.shutdown().await.unwrap();
    assert_eq!(read_to_end(&mut h.upstream).await, req);
    let error = finish(h.relay).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    let views = streams(&sink.frames());
    let req_view = find(&views, WIRE_REQUEST, 1);
    assert_eq!(req_view.bytes, b"half");
    assert_eq!(req_view.end, Some(EndReason::Interrupted));
}

/// The keep-alive reuse race: the upstream closes an idle connection cleanly just as the
/// client sends its next request. Nothing was lost (the client retries on a new connection),
/// so the request is not forwarded, the relay ends `UnexpectedEof`, and no loss is chained.
#[tokio::test]
async fn a_request_after_a_clean_upstream_close_is_not_a_loss() {
    let sink = RecordingSink::new();
    let mut h = start(emitter_for(sink.clone()), policy(&[("/v1/", true, None)]));
    round_trip(
        &mut h,
        &request("GET", "/v1/models", &[], b""),
        &response("HTTP/1.1 200 OK", &["Content-Length: 0"], b""),
    )
    .await;
    h.upstream.shutdown().await.unwrap();
    // The client sees the close, so the response direction has ended before the next request.
    assert_eq!(read_to_end(&mut h.client).await, b"");
    write(
        &mut h.client,
        &request("POST", "/v1/x", &[&length(b"next")], b"next"),
    )
    .await;
    h.client.shutdown().await.unwrap();
    let error = finish(h.relay).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    assert_eq!(
        read_to_end(&mut h.upstream).await,
        b"",
        "the late request is not forwarded"
    );
    let frames = sink.frames();
    assert!(
        !frames
            .iter()
            .any(|f| matches!(f.event, FrameEvent::Loss { .. })),
        "a clean upstream close is not a loss"
    );
    assert!(!has(&streams(&frames), WIRE_REQUEST, 2));
}

/// Each request relayed on an intercepted connection is one model request, however many share
/// the connection; a request for another host is not the captured host's.
#[tokio::test]
async fn two_requests_on_one_connection_count_two() {
    let sink = RecordingSink::new();
    let mut h = start(emitter_for(sink.clone()), policy(&[("/v1/", true, None)]));
    assert_eq!(h.emitter.model_requests(), 0);
    let ok = response("HTTP/1.1 200 OK", &["Content-Length: 0"], b"");
    round_trip(&mut h, &request("GET", "/v1/models", &[], b""), &ok).await;
    assert_eq!(h.emitter.model_requests(), 1);
    // The second request is on the same connection, and one the policy does not capture: it is
    // still a request of the captured host.
    round_trip(&mut h, &request("GET", "/health", &[], b""), &ok).await;
    assert_eq!(h.emitter.model_requests(), 2);
    let counter = h.emitter.model_requests_counter();
    close_both(h).await;
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "the shared counter keeps its value when the connection ends"
    );
}

/// The run's "no model calls" line counts model calls, not every recorded request: a model list
/// under the same base path (a `model_host_from` host records the whole base, without a hint) is
/// recorded but is no call, and a request on a hinted path is both.
#[tokio::test]
async fn only_a_request_on_a_hinted_path_counts_as_a_model_call() {
    let sink = RecordingSink::new();
    let mut h = start(
        emitter_for(sink.clone()),
        policy(&[
            (
                "/v1/chat/completions",
                false,
                Some("openai-chat-completions"),
            ),
            ("/v1/", true, None),
        ]),
    );
    let ok = response("HTTP/1.1 200 OK", &["Content-Length: 0"], b"");
    round_trip(&mut h, &request("GET", "/v1/models", &[], b""), &ok).await;
    assert_eq!(h.emitter.recorded_requests(), 1);
    assert_eq!(
        h.emitter.generation_requests(),
        0,
        "a model list is no call"
    );
    round_trip(
        &mut h,
        &request("POST", "/v1/chat/completions", &[&length(b"{}")], b"{}"),
        &ok,
    )
    .await;
    assert_eq!(h.emitter.recorded_requests(), 2);
    assert_eq!(h.emitter.generation_requests(), 1);
    close_both(h).await;
}

// ---------------------------------------------------------------------------------------
// Upgrades.
// ---------------------------------------------------------------------------------------

fn ws_request(target: &str, host: &str, connection: &str) -> Vec<u8> {
    head_with_host(
        "GET",
        target,
        host,
        &[
            "Upgrade: websocket",
            &format!("Connection: {connection}"),
            &format!("Sec-WebSocket-Key: {WS_KEY}"),
            "Sec-WebSocket-Version: 13",
            "Sec-WebSocket-Extensions: permessage-deflate; client_max_window_bits",
        ],
        b"",
    )
}

fn ws_101(accept: &str) -> Vec<u8> {
    response(
        "HTTP/1.1 101 Switching Protocols",
        &[
            "Upgrade: websocket",
            "Connection: Upgrade",
            &format!("Sec-WebSocket-Accept: {accept}"),
            "Sec-WebSocket-Extensions: permessage-deflate; client_max_window_bits=15",
        ],
        b"",
    )
}

/// A masked text frame and the start of a second one, as a client might send right away.
const CLIENT_SURPLUS: &[u8] = &[
    0x81,
    0x85,
    1,
    2,
    3,
    4,
    b'h' ^ 1,
    b'e' ^ 2,
    b'l' ^ 3,
    b'l' ^ 4,
    b'o' ^ 1,
    0x82,
    0xfe,
];
/// An unmasked text frame and the start of a second, coalesced with the 101.
const UPSTREAM_SURPLUS: &[u8] = &[0x81, 0x03, b'h', b'e', b'y', 0x81];

/// Handshake with both sides coalescing WebSocket bytes, and return the outcome.
async fn coalesced_upgrade(h: &mut Harness, req: &[u8], resp_head: &[u8]) {
    write(&mut h.client, &[req, CLIENT_SURPLUS].concat()).await;
    assert_eq!(read_exact(&mut h.upstream, req.len()).await, req);
    assert_quiet(&mut h.upstream).await;
    write(&mut h.upstream, &[resp_head, UPSTREAM_SURPLUS].concat()).await;
    assert_eq!(read_exact(&mut h.client, resp_head.len()).await, resp_head);
}

async fn upgraded(h: Harness) -> (Upgrade, tokio::io::DuplexStream, tokio::io::DuplexStream) {
    match finish(h.relay).await.expect("relay failed") {
        HttpOutcome::WebSocket(upgrade) => (upgrade, h.client, h.upstream),
        other => panic!("not handed off: {other:?}"),
    }
}

#[tokio::test]
async fn accepted_upgrade_hands_off_both_prefixes_once() {
    let sink = RecordingSink::new();
    let mut h = start(
        emitter_for(sink.clone()),
        policy(&[("/v1/realtime", false, Some("openai-realtime"))]),
    );
    let req = ws_request(
        "/v1/realtime?model=SENTINEL-MODEL",
        HOST,
        "keep-alive, Upgrade",
    );
    coalesced_upgrade(&mut h, &req, &ws_101(WS_ACCEPT)).await;
    // Nothing past the 101 head was forwarded by the HTTP relay.
    assert_quiet(&mut h.client).await;
    let (mut upgrade, mut client, mut upstream) = upgraded(h).await;

    println!(
        "upgrade prefixes: client {} bytes (expected {}), upstream {} bytes (expected {}), extensions {:?}",
        upgrade.client_prefix.len(),
        CLIENT_SURPLUS.len(),
        upgrade.upstream_prefix.len(),
        UPSTREAM_SURPLUS.len(),
        upgrade.extensions
    );
    assert_eq!(upgrade.client_prefix, CLIENT_SURPLUS);
    assert_eq!(upgrade.upstream_prefix, UPSTREAM_SURPLUS);
    assert_eq!(
        upgrade.extensions.as_deref(),
        Some("permessage-deflate; client_max_window_bits=15")
    );
    assert_eq!(upgrade.exchange, 1);
    assert_eq!(upgrade.context.connection, CONN);
    assert_eq!(
        upgrade.capture,
        Some(CaptureMeta {
            wire: Some(WireIdentity {
                connection: CONN.to_string(),
                exchange: 1,
                message_index: None
            }),
            label: "GET /v1/realtime".into(),
            protocol_hint: Some("openai-realtime".into()),
            method: Some("GET".into()),
            host: Some(HOST.into()),
            path: Some("/v1/realtime".into()),
            status: None,
            content_type: None,
        })
    );

    // The pair is intact and unread past the prefixes: bytes written now arrive next.
    write(&mut upstream, b"late-upstream").await;
    assert_eq!(
        read_exact(&mut upgrade.pair.upstream, 13).await,
        b"late-upstream"
    );
    write(&mut upgrade.pair.client, b"to-client").await;
    assert_eq!(
        read_exact(&mut client, 9).await,
        b"to-client",
        "no surplus was forwarded"
    );
    write(&mut upgrade.pair.upstream, b"to-upstream").await;
    assert_eq!(
        read_exact(&mut upstream, 11).await,
        b"to-upstream",
        "no surplus was forwarded"
    );

    // The handshake is HTTP evidence; the surplus is not captured here.
    let frames = sink.frames();
    let views = streams(&frames);
    assert_eq!(views.len(), 2);
    let (req_view, resp_view) = (
        find(&views, WIRE_REQUEST, 1),
        find(&views, WIRE_RESPONSE, 1),
    );
    assert!(req_view.bytes.is_empty() && resp_view.bytes.is_empty());
    assert_eq!(resp_view.meta.status, Some(101));
    assert_eq!(req_view.end, Some(EndReason::Complete));
    assert_eq!(resp_view.end, Some(EndReason::Complete));
    assert!(
        !frames
            .iter()
            .any(|f| matches!(f.event, FrameEvent::Bytes { .. }))
    );
}

#[tokio::test]
async fn excluded_upgrade_relays_without_capture() {
    for (path, selected) in [("/ws/telemetry", false), ("/v1/realtime", true)] {
        let sink = RecordingSink::new();
        let mut h = start(
            emitter_for(sink.clone()),
            policy(&[("/v1/realtime", false, Some("openai-realtime"))]),
        );
        let req = ws_request(&format!("{path}?session=1"), HOST, "Upgrade");
        coalesced_upgrade(&mut h, &req, &ws_101(WS_ACCEPT)).await;
        let (upgrade, _, _) = upgraded(h).await;
        assert_eq!(upgrade.client_prefix, CLIENT_SURPLUS, "{path}");
        assert_eq!(upgrade.upstream_prefix, UPSTREAM_SURPLUS, "{path}");
        let payload_frames = sink
            .frames()
            .into_iter()
            .filter(|f| f.stream.stream != 0)
            .count();
        if selected {
            let capture = upgrade.capture.expect("selected path keeps its metadata");
            assert_eq!(capture.path.as_deref(), Some(path));
            assert_eq!(capture.protocol_hint.as_deref(), Some("openai-realtime"));
            assert_eq!(capture.wire.map(|w| w.exchange), Some(1));
            assert_eq!(payload_frames, 4, "handshake request and response");
        } else {
            assert_eq!(upgrade.capture, None, "relay-only");
            assert_eq!(payload_frames, 0, "no evidence for an unselected path");
        }
    }
}

#[tokio::test]
async fn rejected_upgrade_stays_http() {
    let sink = RecordingSink::new();
    let mut h = start(
        emitter_for(sink.clone()),
        policy(&[("/v1/realtime", false, Some("openai-realtime"))]),
    );
    let req = ws_request("/v1/realtime", HOST, "Upgrade");
    let refusal = response(
        "HTTP/1.1 426 Upgrade Required",
        &["Content-Type: text/plain", "Content-Length: 5"],
        b"nope!",
    );
    round_trip(&mut h, &req, &refusal).await;
    // HTTP parsing resumes on the same connection.
    round_trip(
        &mut h,
        &request("GET", "/v1/realtime", &[], b""),
        &response("HTTP/1.1 200 OK", &["Content-Length: 2"], b"ok"),
    )
    .await;
    close_both(h).await;
    let views = streams(&sink.frames());
    let refused = find(&views, WIRE_RESPONSE, 1);
    assert_eq!(refused.meta.status, Some(426));
    assert_eq!(
        refused.bytes, b"nope!",
        "the rejection body is HTTP evidence"
    );
    assert_eq!(refused.end, Some(EndReason::Complete));
    assert_eq!(find(&views, WIRE_RESPONSE, 2).bytes, b"ok");
}

/// A `101` this relay must not hand to the WebSocket relay: the connection becomes an opaque
/// byte relay, both surpluses are forwarded as bytes, and the outcome is `Opaque`.
async fn assert_not_handed_off(
    req: Vec<u8>,
    resp_head: Vec<u8>,
    policy_paths: &[(&str, bool, Option<&str>)],
) -> Vec<CaptureFrame> {
    let sink = RecordingSink::new();
    let mut h = start(emitter_for(sink.clone()), policy(policy_paths));
    coalesced_upgrade(&mut h, &req, &resp_head).await;
    assert_eq!(
        read_exact(&mut h.client, UPSTREAM_SURPLUS.len()).await,
        UPSTREAM_SURPLUS
    );
    assert_eq!(
        read_exact(&mut h.upstream, CLIENT_SURPLUS.len()).await,
        CLIENT_SURPLUS
    );
    round_trip(&mut h, b"raw-client-bytes", b"raw-upstream-bytes").await;
    close_opaque(h).await;
    sink.frames()
}

#[tokio::test]
async fn malformed_accept_and_missing_token_do_not_hand_off() {
    let paths = [("/v1/realtime", false, Some("openai-realtime"))];
    // A wrong accept hash.
    let frames = assert_not_handed_off(
        ws_request("/v1/realtime", HOST, "Upgrade"),
        ws_101("AAAAAAAAAAAAAAAAAAAAAAAAAAA="),
        &paths,
    )
    .await;
    let views = streams(&frames);
    assert_eq!(find(&views, WIRE_RESPONSE, 1).meta.status, Some(101));
    assert!(
        !frames
            .iter()
            .any(|f| matches!(f.event, FrameEvent::Bytes { .. }))
    );
    // A request without the Connection upgrade token.
    assert_not_handed_off(
        ws_request("/v1/realtime", HOST, "keep-alive"),
        ws_101(WS_ACCEPT),
        &paths,
    )
    .await;
    // A 101 without its own Connection upgrade token.
    let no_token = response(
        "HTTP/1.1 101 Switching Protocols",
        &[
            "Upgrade: websocket",
            &format!("Sec-WebSocket-Accept: {WS_ACCEPT}"),
        ],
        b"",
    );
    assert_not_handed_off(
        ws_request("/v1/realtime", HOST, "Upgrade"),
        no_token,
        &paths,
    )
    .await;
}

/// A request whose Host names another host turns the connection opaque, which is a loss: it
/// is never relayed as ordinary uncaptured traffic inside a connection admitted for this host.
/// What came before it on the connection stays captured; what comes after is opaque too.
#[tokio::test]
async fn host_mismatch_turns_the_connection_opaque() {
    let paths = [("/v1/", true, None)];
    // An upgrade whose Host names another host is neither captured nor handed off.
    let sink = RecordingSink::new();
    let mut h = start(emitter_for(sink.clone()), policy(&paths));
    round_trip(
        &mut h,
        &request("GET", "/v1/models", &[], b""),
        &response("HTTP/1.1 200 OK", &["Content-Length: 2"], b"[]"),
    )
    .await;
    // The relay goes opaque at the mismatched head, so the surplus is not held back for a 101.
    let req = ws_request("/v1/realtime", "evil.example", "Upgrade");
    write(&mut h.client, &[&req[..], CLIENT_SURPLUS].concat()).await;
    assert_eq!(read_exact(&mut h.upstream, req.len()).await, req);
    let resp = ws_101(WS_ACCEPT);
    write(&mut h.upstream, &[&resp[..], UPSTREAM_SURPLUS].concat()).await;
    assert_eq!(read_exact(&mut h.client, resp.len()).await, resp);
    assert_eq!(
        read_exact(&mut h.client, UPSTREAM_SURPLUS.len()).await,
        UPSTREAM_SURPLUS
    );
    assert_eq!(
        read_exact(&mut h.upstream, CLIENT_SURPLUS.len()).await,
        CLIENT_SURPLUS
    );
    close_opaque(h).await;
    let views = streams(&sink.frames());
    assert_eq!(
        find(&views, WIRE_RESPONSE, 1).bytes,
        b"[]",
        "the unrelated exchange stays"
    );
    assert!(!has(&views, WIRE_REQUEST, 2) && !has(&views, WIRE_RESPONSE, 2));

    // A mismatched ordinary request, then a matching one on the same connection: both are
    // relayed, neither is captured, and the fallback is the connection's one loss.
    let sink = RecordingSink::new();
    let mut h = start(emitter_for(sink.clone()), policy(&paths));
    round_trip(
        &mut h,
        &head_with_host(
            "POST",
            "/v1/x",
            "evil.example:443",
            &["Content-Length: 5"],
            b"wrong",
        ),
        &response("HTTP/1.1 200 OK", &["Content-Length: 1"], b"1"),
    )
    .await;
    round_trip(
        &mut h,
        &request("POST", "/v1/x", &["Content-Length: 5"], b"right"),
        &response("HTTP/1.1 200 OK", &["Content-Length: 1"], b"2"),
    )
    .await;
    close_opaque(h).await;
    let frames = sink.frames();
    assert!(
        streams(&frames).is_empty(),
        "nothing after the mismatch is captured"
    );
    assert_eq!(
        frames
            .iter()
            .filter(|f| matches!(f.event, FrameEvent::Loss { .. }))
            .count(),
        1,
        "the fallback is chained as one loss"
    );
}

#[tokio::test]
async fn a_response_to_a_lost_request_stays_uncaptured_after_another_connection_recovers() {
    let sink = RecordingSink::new();
    let emitter = emitter_for(sink.clone());
    emitter.connection_event(
        CONN_B,
        now(),
        ConnectionEvent::Open {
            alpn: Some("http/1.1".into()),
            already_open: false,
        },
    );
    let paths = policy(&[("/v1/responses", false, None)]);
    let mut a = start(emitter.clone(), paths.clone());
    let mut b = start_on(emitter.clone(), paths, 1 << 20, CONN_B, |io| Box::new(io));

    // Connection A loses capture inside its request body.
    let head = request("POST", "/v1/responses", &["Content-Length: 8"], b"");
    write(&mut a.client, &[&head[..], b"abcd"].concat()).await;
    read_exact(&mut a.upstream, head.len() + 4).await;
    wait_until("A's first half is captured", || {
        streams(&sink.frames()).iter().any(|v| v.bytes == b"abcd")
    })
    .await;
    sink.set_pressure(true);
    write(&mut a.client, b"efgh").await;
    assert_eq!(read_exact(&mut a.upstream, 4).await, b"efgh");
    wait_until("the sink refused", || sink.refused() >= 1).await;
    sink.set_pressure(false);

    // Connection B begins a new request: capture recovers there, at B's boundary.
    round_trip(
        &mut b,
        &request("POST", "/v1/responses", &["Content-Length: 1"], b"b"),
        &response("HTTP/1.1 200 OK", &["Content-Length: 2"], b"bb"),
    )
    .await;
    assert_eq!(
        sink.begun().len(),
        2,
        "B's request recovered the shared emitter"
    );

    // A's response arrives under the recovered epoch. Its request was not captured, so it is
    // not a capture boundary and stays uncaptured.
    let resp_a = response("HTTP/1.1 200 OK", &["Content-Length: 2"], b"aa");
    write(&mut a.upstream, &resp_a).await;
    assert_eq!(read_exact(&mut a.client, resp_a.len()).await, resp_a);
    // A's next request is a new boundary and is captured.
    round_trip(
        &mut a,
        &request("POST", "/v1/responses", &["Content-Length: 1"], b"c"),
        &response("HTTP/1.1 200 OK", &["Content-Length: 2"], b"cc"),
    )
    .await;
    close_both(a).await;
    close_both(b).await;

    let frames = sink.frames();
    let views = streams(&frames);
    let on = |connection: &str, source: CaptureSource, exchange: u64| {
        views
            .iter()
            .filter(|v| v.source == source && wire_of(v) == (connection.to_string(), exchange))
            .collect::<Vec<_>>()
    };
    assert!(
        on(CONN, WIRE_RESPONSE, 1).is_empty(),
        "a response to a lost request was captured"
    );
    assert_eq!(on(CONN, WIRE_REQUEST, 1)[0].bytes, b"abcd");
    let fresh = sink.begun()[1].clone();
    for (connection, exchange, req_body, resp_body) in
        [(CONN_B, 1, b"b", b"bb"), (CONN, 2, b"c", b"cc")]
    {
        let (req, resp) = (
            on(connection, WIRE_REQUEST, exchange),
            on(connection, WIRE_RESPONSE, exchange),
        );
        assert_eq!(
            (req.len(), resp.len()),
            (1, 1),
            "{connection} exchange {exchange}"
        );
        assert_eq!(req[0].key.producer, fresh);
        assert_eq!(req[0].bytes, req_body);
        assert_eq!(resp[0].bytes, resp_body);
    }
    // One recovery, re-observing both surviving connections before any new payload.
    let is_recovery = |f: &&CaptureFrame| matches!(f.event, FrameEvent::Recovery { .. });
    assert_eq!(frames.iter().filter(is_recovery).count(), 1);
    let at = frames.iter().position(|f| is_recovery(&f)).unwrap();
    let reobserved: Vec<&str> = frames[at + 1..at + 3]
        .iter()
        .map(|f| match &f.event {
            FrameEvent::Connection {
                connection,
                event:
                    ConnectionEvent::Open {
                        already_open: true, ..
                    },
            } => connection.as_str(),
            other => panic!("not a re-observation: {other:?}"),
        })
        .collect();
    assert_eq!(reobserved, [CONN, CONN_B]);
}

#[tokio::test]
async fn cancelling_the_relay_ends_open_streams_interrupted() {
    let sink = RecordingSink::new();
    let mut h = start(emitter_for(sink.clone()), policy(&[("/v1/", true, None)]));
    let req = request("GET", "/v1/stream", &[], b"");
    write(&mut h.client, &req).await;
    read_exact(&mut h.upstream, req.len()).await;
    let partial = response(
        "HTTP/1.1 200 OK",
        &["Transfer-Encoding: chunked"],
        &chunk(b"first"),
    );
    write(&mut h.upstream, &partial).await;
    read_exact(&mut h.client, partial.len()).await;
    wait_until("the first chunk is captured", || {
        streams(&sink.frames()).iter().any(|v| v.bytes == b"first")
    })
    .await;
    // The composing owner drops the relay future (cancellation, a shutdown deadline).
    h.relay.abort();
    assert!(h.relay.await.unwrap_err().is_cancelled());
    let views = streams(&sink.frames());
    assert_eq!(find(&views, WIRE_REQUEST, 1).end, Some(EndReason::Complete));
    let resp = find(&views, WIRE_RESPONSE, 1);
    assert_eq!(resp.bytes, b"first");
    assert_eq!(
        resp.end,
        Some(EndReason::Interrupted),
        "never left open, never Complete"
    );
}

#[tokio::test]
async fn decompression_bomb_is_bounded_with_an_explicit_gap() {
    // 32 gzip members of 1 MiB of zeros each: about 1 KiB coded apiece, so a single 16 KiB
    // read would inflate to about 16 MiB, well past what one read may expand into.
    let member = gzip(&vec![0u8; 1 << 20]);
    let coded = member.repeat(32);
    assert!(coded.len() < 64 * 1024, "{} coded bytes", coded.len());
    let view = capture_coded_response(&["Content-Encoding: gzip", &length(&coded)], &coded).await;
    // capture_coded_response has already checked that the client got the coded bytes unchanged.
    println!(
        "decompression bomb: {} coded bytes relayed; captured {} decoded bytes; gaps {:?}; end {:?}",
        coded.len(),
        view.bytes.len(),
        view.gaps,
        view.end
    );
    assert_eq!(view.gaps, vec![GapReason::LimitExceeded]);
    assert_eq!(view.end, Some(EndReason::DecodeError));
    assert!(!view.bytes.is_empty(), "the decoded prefix is kept");
    assert!(
        view.bytes.len() <= MAX_CAPTURE_QUEUE_BYTES + 32 * 1024,
        "{} decoded bytes",
        view.bytes.len()
    );
    assert!(view.bytes.len() < 32 << 20, "decoding stopped");
    assert!(view.bytes.iter().all(|&b| b == 0));
}

#[tokio::test]
async fn oversized_heads_fall_back_to_an_opaque_relay() {
    let big_field = |start: &[u8]| {
        let mut head = start.to_vec();
        head.extend(b"X-Big: ");
        head.extend(vec![b'a'; 70 * 1024]);
        head
    };

    // A request head with no end within 64 KiB.
    let sink = RecordingSink::new();
    let mut h = start(emitter_for(sink.clone()), policy(&[("/v1/", true, None)]));
    let big = big_field(b"GET /v1/x HTTP/1.1\r\nHost: api.example.test\r\n");
    write(&mut h.client, &big).await;
    // Relayed as bytes once the bound is reached, without waiting for the head to end.
    assert_eq!(read_exact(&mut h.upstream, big.len()).await, big);
    round_trip(
        &mut h,
        b"\r\n\r\nraw client bytes",
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
    )
    .await;
    close_opaque(h).await;
    assert!(
        sink.frames().iter().all(|f| f.stream.stream == 0),
        "nothing captured past the bound"
    );

    // A response head with no end within 64 KiB.
    let sink = RecordingSink::new();
    let mut h = start(emitter_for(sink.clone()), policy(&[("/v1/", true, None)]));
    let req = request("GET", "/v1/y", &[], b"");
    write(&mut h.client, &req).await;
    assert_eq!(read_exact(&mut h.upstream, req.len()).await, req);
    let big = big_field(b"HTTP/1.1 200 OK\r\n");
    write(&mut h.upstream, &big).await;
    assert_eq!(read_exact(&mut h.client, big.len()).await, big);
    write(&mut h.upstream, b"\r\n\r\nraw body").await;
    h.upstream.shutdown().await.unwrap();
    assert_eq!(read_to_end(&mut h.client).await, b"\r\n\r\nraw body");
    h.client.shutdown().await.unwrap();
    assert_eq!(read_to_end(&mut h.upstream).await, b"");
    assert!(matches!(
        finish(h.relay).await.unwrap(),
        HttpOutcome::Opaque
    ));
    let views = streams(&sink.frames());
    assert_eq!(find(&views, WIRE_REQUEST, 1).end, Some(EndReason::Complete));
    assert!(
        !has(&views, WIRE_RESPONSE, 1),
        "no response head was parsed"
    );
}

/// H-16: a relay that stops parsing records the uncaptured rest of its connection as one
/// stream-zero `Loss { DecodeFailed }` when it falls back, and a second fallback of the same
/// emitter (another connection, or the other direction) adds none: a reason is sent once.
#[tokio::test]
async fn an_opaque_fallback_records_one_loss() {
    let big_head = {
        let mut head = b"GET /v1/x HTTP/1.1\r\nHost: api.example.test\r\n".to_vec();
        head.extend(b"X-Big: ");
        head.extend(vec![b'a'; 70 * 1024]);
        head
    };
    let sink = RecordingSink::new();
    let emitter = emitter_for(sink.clone());
    let losses = |sink: &RecordingSink| -> Vec<(u64, GapReason)> {
        sink.frames()
            .iter()
            .filter_map(|f| match f.event {
                FrameEvent::Loss { reason } => {
                    assert_eq!(f.stream.stream, 0, "a loss is a stream-zero control");
                    Some((f.seq, reason))
                }
                _ => None,
            })
            .collect()
    };
    for round in 0..2 {
        let mut h = start(Arc::clone(&emitter), policy(&[("/v1/", true, None)]));
        write(&mut h.client, &big_head).await;
        assert_eq!(read_exact(&mut h.upstream, big_head.len()).await, big_head);
        // The loss is in the stream as soon as the relay stops parsing, before the connection ends.
        let before = losses(&sink).len();
        assert_eq!(
            before, 1,
            "round {round}: one loss, recorded at the fallback"
        );
        round_trip(
            &mut h,
            b"\r\n\r\nraw client bytes",
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
        )
        .await;
        close_opaque(h).await;
        assert_eq!(
            losses(&sink).len(),
            1,
            "round {round}: a reason is sent once"
        );
    }
    assert_eq!(losses(&sink)[0].1, GapReason::DecodeFailed);
    let _ = streams(&sink.frames()); // the oracle over every frame, the loss included
}

#[tokio::test]
async fn long_selected_paths_are_captured_with_bounded_metadata() {
    let sink = RecordingSink::new();
    let mut h = start(
        emitter_for(sink.clone()),
        policy(&[("/v1/", true, Some("openai-responses"))]),
    );
    // "POST {path}" is exactly one metadata string.
    let fits = format!("/v1/{}", "f".repeat(1015));
    // Fits one metadata string as a path, but not inside "POST {path}".
    let near = format!("/v1/{}", "a".repeat(1016));
    // Longer than one metadata string may be.
    let over = format!("/v1/{}", "b".repeat(2000));
    // A key-shaped secret from byte 1,010, across where a label cut at the bound would fall:
    // the part left after such a cut is too short for any scrub rule to recognize.
    let secret = format!("sk-ant-api03-{}", "k".repeat(90));
    let straddling = format!("/v1/{}{secret}", "c".repeat(1006));
    assert_eq!(straddling.find("sk-ant"), Some(1010));
    for path in [&fits, &near, &over, &straddling] {
        round_trip(
            &mut h,
            &request(
                "POST",
                &format!("{path}?q=1"),
                &["Content-Length: 4"],
                b"body",
            ),
            &response("HTTP/1.1 200 OK", &["Content-Length: 2"], b"ok"),
        )
        .await;
    }
    close_both(h).await;

    let views = streams(&sink.frames());
    for view in &views {
        let label = &view.meta.label;
        assert!(
            !label.contains("sk-ant"),
            "a label keeps part of the secret: {:?}",
            &label[label.len().saturating_sub(24)..]
        );
    }
    // Each label is whole or names no path at all; "200 {near}" is exactly one string.
    let long = (
        "POST <long path>".to_string(),
        "200 <long path>".to_string(),
    );
    for (exchange, (request_label, response_label)) in [
        (1, (format!("POST {fits}"), format!("200 {fits}"))),
        (2, (long.0.clone(), format!("200 {near}"))),
        (3, long.clone()),
        (4, long),
    ] {
        let (req, resp) = (
            find(&views, WIRE_REQUEST, exchange),
            find(&views, WIRE_RESPONSE, exchange),
        );
        assert_eq!(
            req.bytes, b"body",
            "exchange {exchange} is captured, not dropped"
        );
        assert_eq!(resp.bytes, b"ok");
        assert_eq!(req.end, Some(EndReason::Complete));
        assert_eq!(req.meta.label, request_label, "exchange {exchange}");
        assert_eq!(resp.meta.label, response_label, "exchange {exchange}");
        for view in [req, resp] {
            assert!(view.meta.label.len() <= MAX_METADATA_STRING_BYTES);
            assert_eq!(view.meta.protocol_hint.as_deref(), Some("openai-responses"));
        }
    }
    for (exchange, path) in [(1, Some(&fits)), (2, Some(&near)), (3, None), (4, None)] {
        assert_eq!(
            find(&views, WIRE_REQUEST, exchange).meta.path.as_ref(),
            path,
            "exchange {exchange}: a path longer than one metadata string is not retained"
        );
    }
}
