//! PRD-17 section 2.4, rows D1 to D4 and D6: the body-decoding edge cases the 0.7 relay learned
//! the hard way, driven through `proxy::decode` alone. The decoder writes no record itself: it
//! hands out decoded pieces and answers a `Step`, and the caller writes GAP `decode_failed` for
//! `Step::Fault`. So "the records" here are the pieces (the DATA prefix) and the step (the GAP).
//! PRD-03's own cases (each coding decodes, an unknown coding, a window over the room) are in
//! `proxy_decode.rs`; the bomb (D7) and the fake client (D8) wait for the HTTP/1 parser.
//!
//! D6's GAP and END records are asserted through `proxy::serve` in the last test. The coded
//! bytes of D1 to D4 stay pieces-only here; LESSONS.md keeps L-D1 to L-D4 open for that half.

mod common;
mod proxy_fixtures;

use bytes::Bytes;
use common::recorder::{data_of, ends, gaps, opens};
use common::upstream::{Answer, Body, Script, Upstream};
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use flate2::{Compression, GzBuilder};
use goalcup::pipeline::{Call, EndReason, GapReason, Recorder, StreamKind};
use goalcup::proxy::decode::{BodyDecoder, Step, parse_codings};
use std::io::Write;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

macro_rules! encoder {
    ($name:ident, $enc:ident) => {
        fn $name(data: &[u8]) -> Vec<u8> {
            let mut e = $enc::new(Vec::new(), Compression::default());
            e.write_all(data).unwrap();
            e.finish().unwrap()
        }
    };
}
encoder!(gzip, GzEncoder);
encoder!(zlib, ZlibEncoder);
encoder!(raw_deflate, DeflateEncoder);

/// 2,000 synthetic SSE events, the plaintext of `proxy_fixtures/anthropic-sse.br`.
fn sse() -> Vec<u8> {
    (0..2000)
        .map(|i| {
            format!(
                "event: content_block_delta\ndata: {{\"index\":{i},\"delta\":{{\"text\":\"token {i}\"}}}}\n\n"
            )
        })
        .collect::<String>()
        .into_bytes()
}

/// Made with `brotli -q 11` from [`sse`], not a real capture.
const BR: &[u8] = include_bytes!("proxy_fixtures/anthropic-sse.br");
const PIECE: usize = 32 * 1024;
const ROOM: u64 = 1 << 30;
const WHOLE_READ: &[usize] = &[usize::MAX];
const SPLIT_READS: &[usize] = &[7, 4096, 1000];

/// Feeds `wire` in reads of the given sizes (cycled); the pieces handed out and the last step.
fn run(codings: &[&str], wire: &[u8], reads: &[usize]) -> (Step, Vec<Bytes>) {
    let (rec, _calls) = Recorder::for_tests(ROOM);
    let mut pieces = Vec::new();
    let mut out = |b: Bytes| pieces.push(b);
    let mut dec = BodyDecoder::new(parse_codings(codings).expect("supported codings"), &rec);
    let mut step = Step::Ok;
    let (mut rest, mut i) = (wire, 0);
    while !rest.is_empty() {
        let take = reads[i % reads.len()].min(rest.len());
        step = dec.push(&rest[..take], &mut out);
        rest = &rest[take..];
        i += 1;
        if step != Step::Ok {
            // Every later call answers the same, and no piece follows.
            assert_eq!(dec.push(rest, &mut out), step);
            break;
        }
    }
    assert_eq!(dec.wire(), wire.len() as u64);
    if step == Step::Ok {
        step = dec.finish();
    }
    (step, pieces)
}

/// The body decodes to `plain` in pieces of at most 32 KiB, and the body is complete.
fn assert_complete(case: &str, codings: &[&str], wire: &[u8], reads: &[usize], plain: &[u8]) {
    let (step, pieces) = run(codings, wire, reads);
    assert_eq!(step, Step::Ok, "{case}");
    assert!(pieces.iter().all(|p| p.len() <= PIECE), "{case}");
    assert!(pieces.concat() == plain, "{case}: decoded body differs");
}

/// The body is a decode failure and everything handed out is a prefix of `plain`.
fn assert_fault(case: &str, codings: &[&str], wire: &[u8], reads: &[usize], plain: &[u8]) -> usize {
    let (step, pieces) = run(codings, wire, reads);
    assert_eq!(step, Step::Fault, "{case}");
    let got = pieces.concat();
    assert!(plain.starts_with(&got), "{case}: only a decoded prefix");
    got.len()
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

/// A body of `n` bytes that compresses well.
fn body(n: usize) -> Vec<u8> {
    (0..n)
        .map(|i| format!("event: delta\ndata: {{\"index\":{i},\"text\":\"token {i}\"}}\n\n"))
        .collect::<String>()
        .into_bytes()
}

/// D1: Anthropic answers `br` whenever the request offers it. A reply of 32 KiB or more decodes
/// to whole 32 KiB pieces with no gap, and bytes after the stream's end are invalid.
#[test]
fn brotli_whole_32k_pieces_and_trailing_bytes() {
    let plain = sse();
    for reads in [WHOLE_READ, SPLIT_READS] {
        assert_complete("fixture", &["br"], BR, reads, &plain);
    }

    let big = body(5000);
    for size in [32_768, 65_536, 98_304, 40_000] {
        let plain = &big[..size];
        let wire = brotli_stored(plain);
        for reads in [WHOLE_READ, SPLIT_READS] {
            assert_complete(&format!("{size} bytes"), &["br"], &wire, reads, plain);
        }
    }
    // One read: a whole number of pieces, each exactly 32 KiB.
    let (step, pieces) = run(&["br"], &brotli_stored(&big[..65_536]), WHOLE_READ);
    assert_eq!(step, Step::Ok);
    assert!(pieces.iter().all(|p| p.len() == PIECE));
    assert_eq!(pieces.len(), 2);

    // Bytes after the end of the stream: the whole body is decoded, then it is a failure.
    let plain = &big[..40_000];
    let mut wire = brotli_stored(plain);
    wire.push(0);
    for reads in [WHOLE_READ, SPLIT_READS] {
        let kept = assert_fault("trailing byte", &["br"], &wire, reads, plain);
        assert_eq!(kept, plain.len(), "the decoded body is kept");
    }
    let mut wire = BR.to_vec();
    wire.extend_from_slice(b"junk");
    assert_fault("fixture and junk", &["br"], &wire, SPLIT_READS, &sse());
}

/// D2 (review P4 M1): a zstd frame whose output ends exactly when the output buffer fills is
/// done, alone or followed by another frame.
#[test]
fn zstd_frame_ending_on_the_buffer_edge_is_complete() {
    let big = body(5000);
    for size in [32_768, 65_536, 98_304, 40_000] {
        let plain = &big[..size];
        let one = zstd::encode_all(plain, 3).unwrap();
        // The first frame ends on a buffer edge, and another follows it.
        let two = [
            zstd::encode_all(&plain[..32_768], 3).unwrap(),
            zstd::encode_all(&plain[32_768..], 3).unwrap(),
        ]
        .concat();
        for reads in [WHOLE_READ, SPLIT_READS] {
            let case = format!("{size} bytes, reads {reads:?}");
            assert_complete(&format!("one frame, {case}"), &["zstd"], &one, reads, plain);
            assert_complete(
                &format!("two frames, {case}"),
                &["zstd"],
                &two,
                reads,
                plain,
            );
        }
    }
    // Two frames of different levels over a body that is not a whole number of pieces.
    let plain = sse();
    let two = [
        zstd::encode_all(&plain[..1000], 3).unwrap(),
        zstd::encode_all(&plain[1000..], 19).unwrap(),
    ]
    .concat();
    assert_complete("levels 3 and 19", &["zstd"], &two, SPLIT_READS, &plain);
}

/// Where the first zstd block of `frame` ends (RFC 8878 sections 3.1.1.1 and 3.1.1.2), parsed by
/// hand: the frame header, then the first 3-byte block header.
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

/// D2: a frame cut after whole pieces is still a gap, never done. Two ways to get there: cut at
/// the end of a block, and a frame that decoded fully but lost its 4-byte content checksum.
#[test]
fn zstd_cut_after_whole_pieces_is_a_gap() {
    // Several 128 KiB blocks, each a whole number of 32 KiB pieces. The body must compress well,
    // or no read ever fills a piece from its last bytes.
    let plain = body(5000);
    assert!(plain.len() > 131_072 + PIECE, "more than one block");
    let frame = zstd::encode_all(plain.as_slice(), 3).unwrap();
    let first = first_zstd_block_end(&frame);
    assert!(first < frame.len(), "the cut is inside the frame");
    for reads in [WHOLE_READ, SPLIT_READS] {
        let kept = assert_fault(
            "cut at the first block",
            &["zstd"],
            &frame[..first],
            reads,
            &plain,
        );
        assert_eq!(kept, 131_072, "the block decodes and is kept");
    }

    for size in [32_768, 40_000, 131_072] {
        let plain = &plain[..size];
        let mut encoder = zstd::stream::Encoder::new(Vec::new(), 3).unwrap();
        encoder.include_checksum(true).unwrap();
        encoder.write_all(plain).unwrap();
        let whole = encoder.finish().unwrap();
        for reads in [WHOLE_READ, SPLIT_READS] {
            let case = format!("{size} bytes, reads {reads:?}");
            assert_complete(&case, &["zstd"], &whole, reads, plain);
            let cut = &whole[..whole.len() - 4];
            let kept = assert_fault(&case, &["zstd"], cut, reads, plain);
            assert_eq!(kept, size, "{case}: everything decoded is kept");
        }
    }
}

/// D3: gzip members in a row, CRC and size checked per member, every header field, reserved flags
/// refused, `x-gzip` is gzip.
#[test]
fn gzip_two_members_and_a_bad_crc() {
    let plain = b"{\"output\":\"a compressible body, a compressible body\"}".repeat(40);
    let two = [gzip(&plain[..100]), gzip(&plain[100..])].concat();
    for reads in [WHOLE_READ, SPLIT_READS] {
        assert_complete("two members", &["gzip"], &two, reads, &plain);
        assert_complete("x-gzip", &["x-gzip"], &gzip(&plain), reads, &plain);
        assert_complete("x-gzip, two members", &["X-Gzip"], &two, reads, &plain);
    }

    // FEXTRA, FNAME and FCOMMENT (60,000 header bytes), then FHCRC (skipped, two bytes).
    let (extra, name, comment) = (vec![1u8; 60_000], b"name".to_vec(), b"comment".to_vec());
    let mut e = GzBuilder::new()
        .extra(extra.clone())
        .filename(name.clone())
        .comment(comment.clone())
        .write(Vec::new(), Compression::default());
    e.write_all(&plain).unwrap();
    let mut fields = e.finish().unwrap();
    let at = 10 + 2 + extra.len() + name.len() + 1 + comment.len() + 1;
    fields[3] |= 0x02;
    fields.splice(at..at, [0x12, 0x34]);
    let mut fhcrc = gzip(&plain);
    fhcrc[3] |= 0x02;
    fhcrc.splice(10..10, [0x12, 0x34]);
    for reads in [WHOLE_READ, SPLIT_READS, &[1]] {
        assert_complete("all header fields", &["gzip"], &fields, reads, &plain);
        assert_complete("FHCRC", &["gzip"], &fhcrc, reads, &plain);
    }

    // A reserved flag bit is refused before any byte decodes.
    let mut reserved = gzip(&plain);
    reserved[3] |= 0x20;
    assert_eq!(
        assert_fault("reserved flag", &["gzip"], &reserved, SPLIT_READS, &plain),
        0
    );

    // A bad CRC, and a bad size, in the last member and in the first.
    for (case, at) in [("crc", 8), ("size", 4)] {
        let mut bad = gzip(&plain);
        let n = bad.len() - at;
        bad[n] ^= 0xff;
        for reads in [WHOLE_READ, SPLIT_READS] {
            assert_fault(case, &["gzip"], &bad, reads, &plain);
        }
    }
    let mut bad_first = two.clone();
    let n = gzip(&plain[..100]).len() - 8;
    bad_first[n] ^= 0xff;
    assert_fault(
        "first member crc",
        &["gzip"],
        &bad_first,
        SPLIT_READS,
        &plain,
    );

    // A body that stops before its trailer keeps what it decoded, and is a failure.
    let whole = gzip(&plain);
    let kept = assert_fault(
        "no trailer",
        &["gzip"],
        &whole[..whole.len() - 8],
        SPLIT_READS,
        &plain,
    );
    assert_eq!(kept, plain.len());
}

/// D4: HTTP `deflate` is zlib or raw, told apart by the first two bytes, and the first byte may
/// arrive alone.
#[test]
fn raw_and_zlib_deflate_split_after_byte_one() {
    let plain = sse();
    for (case, wire) in [("zlib", zlib(&plain)), ("raw", raw_deflate(&plain))] {
        for reads in [&[1usize, usize::MAX][..], &[1, 1, 1, 4096], WHOLE_READ] {
            assert_complete(case, &["deflate"], &wire, reads, &plain);
        }
        // A body of one byte is cut inside the choice, and is a failure.
        let (step, pieces) = run(&["deflate"], &wire[..1], WHOLE_READ);
        assert_eq!(step, Step::Fault, "{case}: one byte");
        assert!(pieces.is_empty());
    }
}

/// D6: a coding that breaks part way keeps the output decoded before the error, then the caller
/// writes GAP `decode_failed`: the pieces are the DATA prefix, the step is the GAP.
#[test]
fn corrupt_tail_keeps_the_prefix() {
    let plain = b"data: the part that decodes before the stream goes bad\n\n".repeat(20);
    let garbage = [0xffu8, 0xff, 0x00, 0x13, 0x37, 0xde, 0xad];

    let mut g = GzEncoder::new(Vec::new(), Compression::default());
    g.write_all(&plain).unwrap();
    g.flush().unwrap();
    let gz_prefix = g.get_ref().clone();
    let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
    z.write_all(&plain).unwrap();
    z.flush().unwrap();
    let zlib_prefix = z.get_ref().clone();

    for (coding, prefix) in [("gzip", gz_prefix), ("deflate", zlib_prefix)] {
        let wire = [prefix.clone(), garbage.to_vec()].concat();
        for reads in [WHOLE_READ, SPLIT_READS] {
            let (step, pieces) = run(&[coding], &wire, reads);
            assert_eq!(step, Step::Fault, "{coding}");
            let got = pieces.concat();
            assert!(got == plain, "{coding}: exactly the decoded prefix");
            let has = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).any(|w| w == needle);
            assert!(!has(&got, &garbage[..3]), "{coding}: no suffix bytes");
            assert!(!has(&got, &prefix[..8]), "{coding}: no compressed bytes");
        }
    }

    // Cut short, brotli and zstd keep a decoded prefix too.
    let sse = sse();
    let frame = zstd::encode_all(sse.as_slice(), 3).unwrap();
    assert_fault("br", &["br"], &BR[..BR.len() / 2], SPLIT_READS, &sse);
    assert_fault(
        "zstd",
        &["zstd"],
        &frame[..frame.len() / 2],
        SPLIT_READS,
        &sse,
    );
}

/// D6 through `proxy::serve`: a `br` reply whose tail is corrupt keeps the decoded prefix as DATA,
/// then one GAP `decode_failed` and an END `decode_error`; the client still gets every byte the
/// upstream sent, the bytes after the corruption included.
#[tokio::test]
async fn corrupt_br_tail_through_serve_keeps_prefix_gap_end_and_relays_everything() {
    let plain = body(2000); // two meta-blocks, so the first is out before the corrupt header
    let mut wire = brotli_stored(&plain);
    *wire.last_mut().unwrap() = 0xfe; // the closing meta-block header becomes invalid
    wire.extend_from_slice(b"bytes after the corruption");
    let head = vec![("content-encoding".to_string(), "br".to_string())];
    let answer = Answer::Http {
        status: 200,
        headers: head,
        body: Body::Whole(wire.clone()),
        coding: None,
    };
    let up = Upstream::start(
        Script {
            answers: vec![answer],
            ..Script::ok(b"")
        }
        .with_tls(&["api.openai.com"], &[common::pki::HTTP11]),
    );
    let dir = tempfile::tempdir().unwrap();
    let roots = up.pki().roots_file(dir.path()).display().to_string();
    let env = [
        (
            "GOALCUP_TEST_UPSTREAM",
            format!("api.openai.com={}", up.addr()),
        ),
        ("GOALCUP_TEST_EXTRA_ROOTS", roots),
    ];
    let (_h, addr, calls, ca) = proxy_fixtures::proxy(proxy_fixtures::HOSTS, &env);
    let name = rustls::pki_types::ServerName::try_from("api.openai.com").unwrap();
    let tcp = proxy_fixtures::through(addr, "api.openai.com:443").await;
    let mut t = proxy_fixtures::trusting_run_ca(&ca, &[b"http/1.1"])
        .connect(name, tcp)
        .await
        .unwrap();
    t.write_all(b"GET /v1/models HTTP/1.1\r\nHost: api.openai.com\r\nAccept-Encoding: br\r\n\r\n")
        .await
        .unwrap();
    let mut got = Vec::new();
    common::within_async(common::HANG, async {
        while !got.ends_with(&wire) {
            let mut b = [0u8; 4096];
            let n = t.read(&mut b).await.unwrap();
            assert!(n > 0, "the reply ended early");
            got.extend_from_slice(&b[..n]);
        }
    })
    .await;

    let [(resp, ..)] = opens(&calls)
        .into_iter()
        .filter(|o| o.1 == StreamKind::HttpResp)
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    let ended = common::within_async(common::HANG, async {
        loop {
            if let Some(e) = ends(&calls).into_iter().find(|e| e.0 == resp) {
                return e.1;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await;
    let kept = data_of(&calls, resp);
    assert!(
        !kept.is_empty() && plain.starts_with(&kept),
        "a decoded prefix, never a suffix byte"
    );
    let reasons: Vec<_> = gaps(&calls)
        .into_iter()
        .filter_map(|g| match g {
            Call::Gap(s, r, ..) if s == resp => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(reasons, [GapReason::DecodeFailed]);
    assert_eq!(ended, EndReason::DecodeError);
}
