//! PRD-03 acceptance for body decoding (section 7 and the `decoder_window` guard of section 11):
//! each coding decodes to the plaintext (a gzip header of any length included: no cap of our own),
//! an unknown coding writes no body byte, and a window that does not fit `Recorder::room()` leaves
//! the body undecoded, a flushed brotli stream (the SSE shape) included. The cases PRD-17 owns
//! (gzip members and a bad CRC, deflate split after byte one, the zstd buffer edge, a corrupt
//! tail, a bomb) are in `lessons_decode.rs`.

use bytes::Bytes;
use flate2::Compression;
use flate2::GzBuilder;
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use goalcup::pipeline::Recorder;
use goalcup::proxy::decode::{BodyDecoder, Step, Unsupported, parse_codings};
use std::io::Write;

/// Made with `brotli -q 11` from [`text`], not a real capture.
const BR: &[u8] = include_bytes!("proxy_fixtures/anthropic-sse.br");
/// [`text`] as a server streams it: Python `brotli.Compressor(quality=5, lgwin=22)`, `process`
/// then `flush` every 2 KiB, so a window of `1 << 22` (4 MiB) over many meta-blocks.
const BR_FLUSHED: &[u8] = include_bytes!("proxy_fixtures/sse-flushed.br");
const PIECE: usize = 32 * 1024;
const LARGE_ROOM: u64 = 1 << 30;

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

/// Feeds `wire` in irregular splits and answers the last step with every piece handed out.
fn decode(codings: &[&str], wire: &[u8], room: u64) -> (Recorder, Step, Vec<Bytes>) {
    let (rec, _calls) = Recorder::for_tests(room);
    let mut pieces = Vec::new();
    let mut out = |b: Bytes| pieces.push(b);
    let mut dec = BodyDecoder::new(parse_codings(codings).expect("supported codings"), &rec);
    let mut step = Step::Ok;
    let (mut rest, mut i) = (wire, 0);
    while !rest.is_empty() && step == Step::Ok {
        let take = [1, 7, 2, 4096, 3, 70_000, 13, 600][i % 8].min(rest.len());
        step = dec.push(&rest[..take], &mut out);
        rest = &rest[take..];
        i += 1;
    }
    if !rest.is_empty() {
        assert_eq!(dec.push(rest, &mut out), step);
    }
    assert_eq!(dec.wire(), wire.len() as u64);
    if step == Step::Ok {
        step = dec.finish();
    }
    (rec, step, pieces)
}

fn assert_plain(codings: &[&str], wire: &[u8], plain: &[u8]) {
    let (_, step, pieces) = decode(codings, wire, LARGE_ROOM);
    assert_eq!(step, Step::Ok, "{codings:?}");
    assert!(pieces.iter().all(|p| p.len() <= PIECE), "{codings:?}");
    assert!(
        pieces.concat() == plain,
        "{codings:?} decodes to the plaintext"
    );
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

/// A gzip member whose extra field, name and comment take its header past 128 KiB.
fn gzip_long_header(data: &[u8]) -> Vec<u8> {
    let mut e = GzBuilder::new()
        .extra(vec![1; 65_000])
        .filename(vec![b'n'; 40_000])
        .comment(vec![b'c'; 40_000])
        .write(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn raw_deflate(data: &[u8]) -> Vec<u8> {
    let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

#[test]
fn each_coding_decodes_to_the_plaintext() {
    let plain = text();
    let text = plain.repeat(2);
    assert_plain(&["br"], BR, &plain);
    assert_plain(&["br"], BR_FLUSHED, &plain);
    assert_plain(&["gzip"], &gzip(&text), &text);
    assert_plain(&["gzip"], &gzip_long_header(&text), &text);
    assert_plain(&["identity", "X-Gzip"], &gzip(&text), &text);
    assert_plain(&["deflate"], &zlib(&text), &text);
    assert_plain(&["deflate"], &raw_deflate(&text), &text);
    assert_plain(&["zstd"], &zstd::encode_all(&text[..], 3).unwrap(), &text);
    // Applied br first, then gzip: undone gzip first, then br.
    assert_plain(&["br, gzip"], &gzip(BR_FLUSHED), &plain);
    assert_plain(&[], &text, &text);
}

#[test]
fn an_unknown_coding_writes_no_body_byte() {
    assert_eq!(
        parse_codings(&["gzip, foo"]),
        Err(Unsupported {
            codings: vec!["gzip".into(), "foo".into()]
        })
    );
    let long = "x".repeat(40);
    let Err(Unsupported { codings }) = parse_codings(&["br", long.as_str(), "a, b"]) else {
        panic!("an unknown coding is unsupported");
    };
    assert_eq!(codings, ["br", &long[..32], "a", "b"]);
}

/// The step of a body refused by the `decoder_window` guard, with the room it saw.
fn refused(step: Step) -> u64 {
    match step {
        Step::NotDecoded { limit } => limit,
        other => panic!("not refused: {other:?}"),
    }
}

#[test]
fn a_window_over_room_is_not_decoded() {
    let room = 32 * 1024;
    let (rec, step, pieces) = decode(&["br"], BR_FLUSHED, room);
    assert!(refused(step) <= room);
    assert!(pieces.is_empty());
    assert_eq!(rec.room(), room);

    // `br, gzip`: the gzip stage holds its 32 KiB when the brotli window is refused, and the
    // limit is the room at that moment.
    let room = 80 * 1024;
    let (rec, step, pieces) = decode(&["br, gzip"], &gzip(BR_FLUSHED), room);
    assert!(refused(step) <= room - 32 * 1024);
    assert!(pieces.is_empty());
    assert_eq!(rec.room(), room);
}

/// The SSE shape: a flushed brotli stream holds its whole window from its first meta-block, so
/// the guard counts the window at the first push and refuses a body whose window does not fit.
#[test]
fn a_flushed_brotli_body_counts_its_whole_window() {
    let (rec, _calls) = Recorder::for_tests(LARGE_ROOM);
    let mut pieces = Vec::new();
    let mut out = |b: Bytes| pieces.push(b);
    let mut dec = BodyDecoder::new(parse_codings(&["br"]).unwrap(), &rec);
    assert_eq!(dec.push(&BR_FLUSHED[..1024], &mut out), Step::Ok);
    assert!(!pieces.is_empty());
    assert!(
        LARGE_ROOM - rec.room() >= 1 << 22,
        "the whole window is counted"
    );
    drop(dec);
    assert_eq!(rec.room(), LARGE_ROOM);

    let room = 1 << 20;
    let (rec, step, pieces) = decode(&["br"], BR_FLUSHED, room);
    assert!(refused(step) <= room);
    assert!(pieces.is_empty());
    assert_eq!(rec.room(), room);
}
