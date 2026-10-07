//! `proxy::websocket::relay_websocket` between two in-memory peers the test plays.
//!
//! Every forwarded byte is checked as it arrives (`Peer::expect`) and every run ends with an
//! EOF check on both peers, so "byte-identical" means exactly the bytes sent, once, in order,
//! and nothing added. Capture is read back from a recording sink through `check_c2` and
//! `streams`, which reimplement the frame rules instead of trusting the emitter. The harness
//! spawns the relay with `tokio::spawn`, so these tests only compile while its future is
//! `Send`.

mod m2_ws_support;

use goalcup_protocol::capture::{
    CaptureFrame, CaptureMeta, CaptureSink, CaptureSource, ConnectionEvent, EndReason,
    EpochAdmission, FrameEvent, GapReason, MAX_SEGMENT_BYTES, SubmitResult, WireIdentity,
};
use m2_ws_support::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// ---------------------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct Fixture {
    examples: Vec<Example>,
    headers: Vec<HeaderExample>,
}

#[derive(serde::Deserialize)]
struct Example {
    name: String,
    side: String,
    extensions: Option<String>,
    frames: Vec<FixtureFrame>,
    messages: Vec<String>,
}

#[derive(serde::Deserialize)]
struct FixtureFrame {
    hex: String,
    fin: bool,
    rsv1: bool,
    opcode: u8,
    mask: Option<String>,
    payload_hex: String,
}

#[derive(serde::Deserialize)]
struct HeaderExample {
    name: String,
    hex: String,
    opcode: u8,
    payload_len: usize,
}

fn fixture() -> Fixture {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/m2-websocket/rfc-examples.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("fixture json")
}

fn pattern(len: usize, seed: u32) -> Vec<u8> {
    (0..len as u32)
        .map(|i| (i.wrapping_add(seed).wrapping_mul(2_654_435_761) >> 24) as u8)
        .collect()
}

fn unexpected_eof(result: io::Result<()>) {
    let error = result.expect_err("the relay must not report a clean close");
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof, "{error}");
}

fn by_source(all: &[Captured], source: CaptureSource) -> Vec<Captured> {
    all.iter()
        .filter(|stream| stream.source == source)
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------------------
// The encoder and the RFC examples
// ---------------------------------------------------------------------------------------

#[test]
fn encoder_reproduces_the_rfc_byte_examples() {
    let fixture = fixture();
    for example in &fixture.examples {
        for frame in &example.frames {
            let mut built = Frame::new(frame.opcode, hex(&frame.payload_hex))
                .fin(frame.fin)
                .rsv(if frame.rsv1 { RSV1 } else { 0 });
            if let Some(mask) = &frame.mask {
                built = built.masked(hex(mask).try_into().expect("four-byte key"));
            }
            assert_eq!(built.encode(), hex(&frame.hex), "{}", example.name);
        }
    }
    for header in &fixture.headers {
        let expected = hex(&header.hex);
        let encoded = Frame::new(header.opcode, vec![0u8; header.payload_len]).encode();
        assert_eq!(&encoded[..expected.len()], &expected[..], "{}", header.name);
        assert_eq!(encoded.len(), expected.len() + header.payload_len);
    }
    // The compressor helper round-trips, and RFC 7692 7.2.3.2's second message really
    // depends on the first one's window: without that history it does not decode to "Hello".
    assert_eq!(inflate_fresh(&hex("f248cdc9c90700")).unwrap(), b"Hello");
    assert_eq!(
        inflate_fresh(&Deflater::new(false).message(b"Hello")).unwrap(),
        b"Hello"
    );
    assert_ne!(
        inflate_fresh(&hex("f200110000")).ok(),
        Some(b"Hello".to_vec())
    );
}

#[tokio::test]
async fn rfc_examples_relay_byte_identically_and_capture_the_stated_messages() {
    for example in fixture().examples {
        let sink = RecordingSink::new();
        let emitter = emitter(&sink);
        announce(&emitter, CONN_A);
        let mut setup = Setup::captured(CONN_A);
        setup.extensions = example.extensions.clone();
        let mut relay = start(&emitter, setup).await;
        for frame in &example.frames {
            let bytes = hex(&frame.hex);
            match example.side.as_str() {
                "client" => relay.pass_client(&bytes).await,
                _ => relay.pass_server(&bytes).await,
            }
        }
        relay
            .close()
            .await
            .unwrap_or_else(|error| panic!("{}: {error}", example.name));
        assert_c2(&sink);
        let all = streams(&sink.frames());
        let texts: Vec<String> = all.iter().map(Captured::text).collect();
        assert_eq!(texts, example.messages, "{}", example.name);
        let source = match example.side.as_str() {
            "client" => CaptureSource::WsClient,
            _ => CaptureSource::WsServer,
        };
        for stream in &all {
            assert_eq!(stream.source, source, "{}", example.name);
            assert_eq!(stream.end, Some(EndReason::Complete), "{}", example.name);
            assert!(stream.gaps.is_empty(), "{}", example.name);
        }
    }
}

// ---------------------------------------------------------------------------------------
// Hand-off, relay-only and metadata
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn prefixes_are_forwarded_and_observed_once_before_socket_bytes() {
    let sink = RecordingSink::new();
    let emitter = emitter(&sink);
    announce(&emitter, CONN_A);
    let c1 = Frame::text("first client message").masked(key(1)).encode();
    let c2 = Frame::text("second client message").masked(key(2)).encode();
    let s1 = Frame::text("first server message").encode();
    let s2 = Frame::binary(vec![0x5a; 300]).encode();
    let mut setup = Setup::captured(CONN_A);
    // A complete frame, then a partial one: c2 breaks inside its masking key, s2 inside its
    // 16-bit length. The rest of each is already waiting on the socket.
    setup.client_prefix = [c1.clone(), c2[..5].to_vec()].concat();
    setup.client_preload = c2[5..].to_vec();
    setup.upstream_prefix = [s1.clone(), s2[..3].to_vec()].concat();
    setup.upstream_preload = s2[3..].to_vec();
    let mut relay = start(&emitter, setup).await;
    relay.upstream.expect(&[c1, c2].concat()).await;
    relay.client.expect(&[s1, s2].concat()).await;
    relay.close().await.expect("clean close");

    assert_c2(&sink);
    let all = streams(&sink.frames());
    let client: Vec<String> = by_source(&all, CaptureSource::WsClient)
        .iter()
        .map(Captured::text)
        .collect();
    assert_eq!(client, ["first client message", "second client message"]);
    let server = by_source(&all, CaptureSource::WsServer);
    assert_eq!(server.len(), 2);
    assert_eq!(server[0].text(), "first server message");
    assert_eq!(server[1].data, vec![0x5a; 300]);
    assert!(all.iter().all(|s| s.end == Some(EndReason::Complete)));
}

#[tokio::test]
async fn relay_only_upgrade_forwards_everything_and_captures_nothing() {
    let sink = RecordingSink::new();
    let emitter = emitter(&sink);
    let mut relay = start(
        &emitter,
        Setup::relay_only(CONN_A).extensions("permessage-deflate"),
    )
    .await;
    let compressed = Deflater::new(true).message(b"compressed request");
    relay
        .pass_client(&Frame::text(compressed).rsv(RSV1).masked(key(1)).encode())
        .await;
    relay
        .pass_server(&Frame::text("plain").fin(false).encode())
        .await;
    relay.pass_server(&Frame::ping("ping").encode()).await;
    relay.pass_server(&Frame::cont(" reply").encode()).await;
    relay.close().await.expect("clean close");
    assert!(sink.log().is_empty(), "relay-only touched the sink");
}

#[tokio::test]
async fn message_streams_keep_the_handed_over_path_and_protocol_hint_in_both_directions() {
    let bases = [
        request_meta(CONN_A),
        CaptureMeta {
            path: Some("/backend-api/codex/responses".to_string()),
            protocol_hint: None,
            host: Some("chatgpt.example.test".to_string()),
            ..request_meta(CONN_A)
        },
    ];
    for base in bases {
        let sink = RecordingSink::new();
        let emitter = emitter(&sink);
        announce(&emitter, CONN_A);
        let mut setup = Setup::captured(CONN_A);
        setup.capture = Some(base.clone());
        let mut relay = start(&emitter, setup).await;
        relay
            .pass_client(&Frame::text("hi").masked(key(1)).encode())
            .await;
        relay.pass_server(&Frame::text("hello").encode()).await;
        relay
            .pass_client(&Frame::binary(vec![1, 2, 3]).masked(key(2)).encode())
            .await;
        relay.close().await.expect("clean close");

        assert_c2(&sink);
        let all = streams(&sink.frames());
        let expected = [
            (CaptureSource::WsClient, 0, "client"),
            (CaptureSource::WsServer, 1, "server"),
            (CaptureSource::WsClient, 2, "client"),
        ];
        assert_eq!(all.len(), expected.len());
        for (stream, (source, index, side)) in all.iter().zip(expected) {
            assert_eq!(stream.source, source);
            assert_eq!(
                stream.meta.wire,
                Some(WireIdentity {
                    connection: CONN_A.to_string(),
                    exchange: EXCHANGE,
                    message_index: Some(index),
                })
            );
            assert_eq!(stream.meta.path, base.path);
            assert_eq!(stream.meta.protocol_hint, base.protocol_hint);
            assert_eq!(stream.meta.host, base.host);
            assert_eq!(stream.meta.method, None);
            assert_eq!(stream.meta.status, None);
            assert_eq!(stream.meta.content_type, None);
            assert!(stream.meta.label.contains(side), "{}", stream.meta.label);
        }
    }
}

// ---------------------------------------------------------------------------------------
// Framing
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn frame_headers_and_mask_keys_split_across_one_byte_reads() {
    let sink = RecordingSink::new();
    let emitter = emitter(&sink);
    announce(&emitter, CONN_A);
    let mut relay = start(&emitter, Setup::captured(CONN_A)).await;

    let three_hundred = pattern(300, 1);
    let client = [
        Frame::text("tiny").masked(key(11)),
        Frame::binary(three_hundred.clone()).masked(key(12)),
        Frame::text("forced sixteen")
            .len(Len::Force16)
            .masked(key(13)),
        Frame::text("forced sixty-four")
            .len(Len::Force64)
            .masked(key(14)),
        Frame::text("frag-").fin(false).masked(key(15)),
        Frame::cont("mented").masked(key(16)),
    ];
    for frame in &client {
        relay.pass_client_bytewise(&frame.encode()).await;
    }
    let seventy_thousand = pattern(70_000, 2);
    let big = Frame::binary(seventy_thousand.clone()).encode();
    relay.pass_server_bytewise(&big[..14]).await;
    relay.pass_server(&big[14..]).await;
    relay
        .pass_server_bytewise(&Frame::text("server").len(Len::Force64).encode())
        .await;
    relay.close().await.expect("clean close");

    assert_c2(&sink);
    let all = streams(&sink.frames());
    assert_segment_bound(&all);
    let client_data: Vec<Vec<u8>> = by_source(&all, CaptureSource::WsClient)
        .into_iter()
        .map(|stream| stream.data)
        .collect();
    assert_eq!(
        client_data,
        [
            b"tiny".to_vec(),
            three_hundred,
            b"forced sixteen".to_vec(),
            b"forced sixty-four".to_vec(),
            b"frag-mented".to_vec(),
        ]
    );
    let server = by_source(&all, CaptureSource::WsServer);
    assert_eq!(server[0].data, seventy_thousand);
    assert_eq!(server[1].text(), "server");
    assert!(all.iter().all(|s| s.end == Some(EndReason::Complete)));
}

#[tokio::test]
async fn a_long_message_is_emitted_before_its_final_frame_with_contiguous_offsets() {
    const FRAGMENT: usize = 5_000;
    const FRAGMENTS: usize = 40;
    let sink = RecordingSink::new();
    let emitter = emitter(&sink);
    announce(&emitter, CONN_A);
    let mut relay = start(&emitter, Setup::captured(CONN_A)).await;

    // One client message over 40 continuation frames, 200 000 bytes: several segments.
    let message = pattern(FRAGMENT * FRAGMENTS, 3);
    for (i, chunk) in message.chunks(FRAGMENT).enumerate() {
        let frame = if i == 0 {
            Frame::binary(chunk.to_vec())
        } else {
            Frame::cont(chunk.to_vec())
        };
        relay
            .pass_client(&frame.fin(i + 1 == FRAGMENTS).masked(key(i as u32)).encode())
            .await;
        if i == 29 {
            relay.settle_client().await;
            let all = streams(&sink.frames());
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].data, message[..30 * FRAGMENT], "emitted before FIN");
            assert!(all[0].contiguous);
            assert_eq!(all[0].end, None);
        }
    }

    // One server frame of 150 000 bytes arriving in 10 000-byte pieces: its bytes are
    // emitted read by read, long before the frame ends.
    let big = pattern(150_000, 4);
    let encoded = Frame::binary(big.clone()).encode();
    let header = encoded.len() - big.len();
    let pieces: Vec<&[u8]> = encoded[header..].chunks(10_000).collect();
    relay.pass_server(&encoded[..header]).await;
    for (i, piece) in pieces.iter().enumerate() {
        relay.pass_server(piece).await;
        if i == 8 {
            // Every piece before this one was read, forwarded and observed before this one
            // was read; this one may still be in progress.
            let emitted = by_source(&streams(&sink.frames()), CaptureSource::WsServer)[0]
                .data
                .len();
            assert!(emitted >= 8 * 10_000, "only {emitted} bytes emitted");
            assert!(emitted <= 9 * 10_000);
        }
    }
    relay.close().await.expect("clean close");

    assert_c2(&sink);
    let all = streams(&sink.frames());
    assert_segment_bound(&all);
    assert_eq!(all[0].data, message);
    assert!(all[0].bytes_frames >= message.len().div_ceil(MAX_SEGMENT_BYTES));
    assert_eq!(all[1].data, big);
    assert!(all.iter().all(|s| s.end == Some(EndReason::Complete)));
}

#[tokio::test]
async fn control_frames_interleave_without_disturbing_the_message_they_interrupt() {
    let sink = RecordingSink::new();
    let emitter = emitter(&sink);
    announce(&emitter, CONN_A);
    let mut relay = start(&emitter, Setup::captured(CONN_A)).await;

    relay
        .pass_client(&Frame::text("part one, ").fin(false).masked(key(1)).encode())
        .await;
    // Each control frame reaches the other peer while the message is still open.
    relay
        .pass_client(&Frame::ping("are you there").masked(key(2)).encode())
        .await;
    relay
        .pass_server(&Frame::pong("are you there").encode())
        .await;
    relay.pass_server(&Frame::ping("keepalive").encode()).await;
    relay
        .pass_client(&Frame::pong("keepalive").masked(key(3)).encode())
        .await;
    relay
        .pass_client(&Frame::cont("part two, ").fin(false).masked(key(4)).encode())
        .await;
    relay
        .pass_client(&Frame::cont("the end").masked(key(5)).encode())
        .await;
    relay
        .pass_client(&Frame::text("next").masked(key(6)).encode())
        .await;
    relay
        .pass_server(&Frame::text("server a, ").fin(false).encode())
        .await;
    relay.pass_server(&Frame::ping("").encode()).await;
    relay.pass_server(&Frame::cont("server b").encode()).await;
    relay.close().await.expect("clean close");

    assert_c2(&sink);
    let all = streams(&sink.frames());
    let seen: Vec<(CaptureSource, u64, String)> = all
        .iter()
        .map(|s| (s.source, s.index(), s.text()))
        .collect();
    assert_eq!(
        seen,
        [
            (
                CaptureSource::WsClient,
                0,
                "part one, part two, the end".to_string()
            ),
            (CaptureSource::WsClient, 1, "next".to_string()),
            (CaptureSource::WsServer, 2, "server a, server b".to_string()),
        ]
    );
    assert!(all.iter().all(|s| s.end == Some(EndReason::Complete)));
}

// ---------------------------------------------------------------------------------------
// permessage-deflate
// ---------------------------------------------------------------------------------------

fn history(connection: &str, side: &str, n: usize) -> Vec<u8> {
    let mut text = String::new();
    for round in 0..6 {
        text.push_str(&format!(
            "{connection} {side}: the quick brown fox jumps over the lazy dog ({round}); "
        ));
    }
    text.push_str(&format!("message {n} of {connection} {side}"));
    text.into_bytes()
}

#[tokio::test]
async fn permessage_deflate_decodes_per_direction_and_connection_with_and_without_takeover() {
    for extensions in [
        "permessage-deflate",
        "permessage-deflate; client_no_context_takeover",
        "permessage-deflate; server_no_context_takeover; server_max_window_bits=12",
        "permessage-deflate; client_no_context_takeover; server_no_context_takeover; client_max_window_bits=15",
    ] {
        let client_takeover = !extensions.contains("client_no_context_takeover");
        let server_takeover = !extensions.contains("server_no_context_takeover");
        let sink = RecordingSink::new();
        let emitter = emitter(&sink);
        announce(&emitter, CONN_A);
        announce(&emitter, CONN_B);
        // Two live connections with distinct histories, interleaved message by message: any
        // inflater state shared across connections or directions corrupts one of them.
        let mut relays = [
            start(&emitter, Setup::captured(CONN_A).extensions(extensions)).await,
            start(&emitter, Setup::captured(CONN_B).extensions(extensions)).await,
        ];
        let mut deflaters = [
            (
                Deflater::new(client_takeover),
                Deflater::new(server_takeover),
            ),
            (
                Deflater::new(client_takeover),
                Deflater::new(server_takeover),
            ),
        ];
        let connections = [CONN_A, CONN_B];
        for n in 0..4 {
            for (at, relay) in relays.iter_mut().enumerate() {
                let (client, server) = &mut deflaters[at];
                let request = client.message(&history(connections[at], "client", n));
                let response = server.message(&history(connections[at], "server", n));
                if n == 1 && client_takeover {
                    // The fixture really exercises takeover: this message needs history.
                    assert_ne!(
                        inflate_fresh(&request).ok(),
                        Some(history(connections[at], "client", n))
                    );
                }
                if n % 2 == 1 {
                    let half = request.len() / 2;
                    relay
                        .pass_client(
                            &Frame::text(request[..half].to_vec())
                                .rsv(RSV1)
                                .fin(false)
                                .masked(key(n as u32))
                                .encode(),
                        )
                        .await;
                    relay
                        .pass_client(
                            &Frame::cont(request[half..].to_vec())
                                .masked(key(n as u32 + 100))
                                .encode(),
                        )
                        .await;
                } else {
                    relay
                        .pass_client(
                            &Frame::text(request)
                                .rsv(RSV1)
                                .masked(key(n as u32))
                                .encode(),
                        )
                        .await;
                }
                relay
                    .pass_server(&Frame::text(response).rsv(RSV1).encode())
                    .await;
            }
        }
        for relay in relays {
            relay.close().await.expect("clean close");
        }

        assert_c2(&sink);
        let all = streams(&sink.frames());
        for connection in connections {
            let mine = of_connection(&all, connection);
            assert_eq!(mine.len(), 8, "{extensions}");
            for (n, pair) in mine.chunks(2).enumerate() {
                assert_eq!(pair[0].source, CaptureSource::WsClient);
                assert_eq!(
                    pair[0].data,
                    history(connection, "client", n),
                    "{extensions}"
                );
                assert_eq!(pair[1].source, CaptureSource::WsServer);
                assert_eq!(
                    pair[1].data,
                    history(connection, "server", n),
                    "{extensions}"
                );
            }
            for stream in &mine {
                assert_eq!(stream.end, Some(EndReason::Complete), "{extensions}");
                assert!(stream.gaps.is_empty());
            }
        }
    }
}

#[tokio::test]
async fn invalid_compressed_data_keeps_the_decoded_prefix_and_ends_decode_error() {
    const NOT_DEFLATE: [u8; 6] = [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc];
    for (extensions, server_takeover) in [
        ("permessage-deflate", true),
        ("permessage-deflate; server_no_context_takeover", false),
    ] {
        let sink = RecordingSink::new();
        let emitter = emitter(&sink);
        announce(&emitter, CONN_A);
        let mut relay = start(&emitter, Setup::captured(CONN_A).extensions(extensions)).await;

        let first = Deflater::new(server_takeover).message(b"first server message");
        relay
            .pass_server(&Frame::text(first).rsv(RSV1).encode())
            .await;
        let prefix = Deflater::new(false).message(b"valid prefix text");
        relay
            .pass_server(&Frame::text(prefix.clone()).rsv(RSV1).fin(false).encode())
            .await;
        relay.settle_server().await;
        let emitted = by_source(&streams(&sink.frames()), CaptureSource::WsServer);
        assert_eq!(
            emitted[1].text(),
            "valid prefix text",
            "the prefix is emitted first"
        );
        assert_eq!(emitted[1].end, None);

        relay
            .pass_server(&Frame::cont(NOT_DEFLATE.to_vec()).encode())
            .await;
        // A standalone compressed message a fresh inflater would decode correctly.
        let third = Deflater::new(false).message(b"third server message");
        relay
            .pass_server(&Frame::text(third).rsv(RSV1).encode())
            .await;
        relay
            .pass_server(&Frame::text("plain server message").encode())
            .await;
        let request = Deflater::new(true).message(b"the client still decodes");
        relay
            .pass_client(&Frame::text(request).rsv(RSV1).masked(key(3)).encode())
            .await;
        relay
            .close()
            .await
            .expect("decode failure does not end the transport");

        assert_c2(&sink);
        let all = streams(&sink.frames());
        let server = by_source(&all, CaptureSource::WsServer);
        assert_eq!(server.len(), 4, "{extensions}");
        assert_eq!(server[0].text(), "first server message");
        assert_eq!(server[0].end, Some(EndReason::Complete));
        assert_eq!(server[1].text(), "valid prefix text", "{extensions}");
        assert_eq!(server[1].gaps, [GapReason::DecodeFailed]);
        assert_eq!(server[1].end, Some(EndReason::DecodeError));
        if server_takeover {
            // The shared window is gone: never reset it and claim complete bytes.
            assert!(server[2].data.is_empty(), "{extensions}");
            assert_eq!(server[2].gaps, [GapReason::DecodeFailed]);
            assert_eq!(server[2].end, Some(EndReason::DecodeError));
        } else {
            assert_eq!(server[2].text(), "third server message", "{extensions}");
            assert_eq!(server[2].end, Some(EndReason::Complete));
        }
        assert_eq!(server[3].text(), "plain server message");
        assert_eq!(server[3].end, Some(EndReason::Complete));
        let client = by_source(&all, CaptureSource::WsClient);
        assert_eq!(client[0].text(), "the client still decodes");
        assert_eq!(client[0].end, Some(EndReason::Complete));
        for stream in &all {
            for window in [&NOT_DEFLATE[..4], &prefix[..4]] {
                assert!(
                    !stream.data.windows(4).any(|w| w == window),
                    "encoded bytes were retained: {extensions}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Unsupported extensions and protocol violations
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn unsupported_extensions_relay_unchanged_and_record_explicit_loss() {
    for extensions in [
        "x-webkit-deflate-frame",
        "permessage-deflate; mystery=1",
        "permessage-deflate, x-custom-extension",
    ] {
        let sink = RecordingSink::new();
        let emitter = emitter(&sink);
        announce(&emitter, CONN_A);
        let mut relay = start(&emitter, Setup::captured(CONN_A).extensions(extensions)).await;
        let request = Deflater::new(true).message(b"compressed request");
        relay
            .pass_client(&Frame::text(request).rsv(RSV1).masked(key(1)).encode())
            .await;
        relay
            .pass_client(&Frame::text("plain request").masked(key(2)).encode())
            .await;
        relay
            .pass_server(&Frame::binary(vec![9; 40]).rsv(RSV1 | RSV2).encode())
            .await;
        relay
            .pass_server(&Frame::text("plain reply").encode())
            .await;
        relay.close().await.expect("clean close");

        assert_c2(&sink);
        let all = streams(&sink.frames());
        assert_eq!(
            all.len(),
            2,
            "one explicit record per direction: {extensions}"
        );
        for (stream, source) in all
            .iter()
            .zip([CaptureSource::WsClient, CaptureSource::WsServer])
        {
            assert_eq!(stream.source, source);
            assert!(stream.data.is_empty());
            assert_eq!(stream.gaps, [GapReason::UnsupportedEncoding]);
            assert_eq!(stream.end, Some(EndReason::DecodeError));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Client,
    Server,
}

async fn pass(relay: &mut Relay, bytes: &[u8], from: Side) {
    match from {
        Side::Client => relay.pass_client(bytes).await,
        Side::Server => relay.pass_server(bytes).await,
    }
}

/// One protocol-violation case: the frames that break the rules on `side`, and the decoded
/// bytes the stream they end keeps.
struct Violation {
    name: &'static str,
    side: Side,
    extensions: Option<&'static str>,
    frames: Vec<Frame>,
    kept: &'static [u8],
}

fn violation(name: &'static str, side: Side, frames: Vec<Frame>) -> Violation {
    Violation {
        name,
        side,
        extensions: None,
        frames,
        kept: b"",
    }
}

#[tokio::test]
async fn a_protocol_violation_ends_capture_of_that_direction_explicitly() {
    let compressed = Deflater::new(true).message(b"compressed start");
    let cases = vec![
        violation(
            "client-unmasked",
            Side::Client,
            vec![Frame::text("unmasked")],
        ),
        violation(
            "server-masked",
            Side::Server,
            vec![Frame::text("masked").masked(key(9))],
        ),
        violation(
            "client-rsv2",
            Side::Client,
            vec![Frame::text("x").rsv(RSV2)],
        ),
        violation(
            "server-rsv3",
            Side::Server,
            vec![Frame::text("x").rsv(RSV3)],
        ),
        violation(
            "server-reserved-data-opcode",
            Side::Server,
            vec![Frame::new(0x3, "x")],
        ),
        violation(
            "client-reserved-control-opcode",
            Side::Client,
            vec![Frame::new(0xb, "x")],
        ),
        violation(
            "server-rsv1-without-deflate",
            Side::Server,
            vec![Frame::text("x").rsv(RSV1)],
        ),
        violation(
            "client-long-control",
            Side::Client,
            vec![Frame::ping(vec![1; 126])],
        ),
        violation(
            "server-fragmented-control",
            Side::Server,
            vec![Frame::ping("x").fin(false)],
        ),
        violation(
            "client-orphan-continuation",
            Side::Client,
            vec![Frame::cont("x")],
        ),
        Violation {
            kept: b"open",
            ..violation(
                "client-new-message-inside-a-fragmented-one",
                Side::Client,
                vec![Frame::text("open").fin(false), Frame::text("again")],
            )
        },
        Violation {
            extensions: Some("permessage-deflate"),
            kept: b"compressed start",
            ..violation(
                "server-rsv1-on-a-continuation",
                Side::Server,
                vec![
                    Frame::text(compressed.clone()).rsv(RSV1).fin(false),
                    Frame::cont("more").rsv(RSV1),
                ],
            )
        },
    ];
    for Violation {
        name,
        side,
        extensions,
        frames,
        kept,
    } in cases
    {
        let sink = RecordingSink::new();
        let emitter = emitter(&sink);
        announce(&emitter, CONN_A);
        let mut setup = Setup::captured(CONN_A);
        setup.extensions = extensions.map(str::to_string);
        let mut relay = start(&emitter, setup).await;
        let mut masks = 0;
        // Client frames are masked, except the violating frame of the masking case itself.
        let mut on_side = |frame: Frame, violating: bool| match side {
            Side::Client if !(violating && name == "client-unmasked") => {
                masks += 1;
                frame.masked(key(masks)).encode()
            }
            _ => frame.encode(),
        };
        let before = on_side(Frame::text("before"), false);
        let violating: Vec<Vec<u8>> = frames
            .into_iter()
            .map(|frame| on_side(frame, true))
            .collect();
        let after = on_side(Frame::text("after"), false);
        pass(&mut relay, &before, side).await;
        for bytes in &violating {
            pass(&mut relay, bytes, side).await;
        }
        pass(&mut relay, &after, side).await;
        let (other, other_frame) = match side {
            Side::Client => (Side::Server, Frame::text("other side").encode()),
            Side::Server => (
                Side::Client,
                Frame::text("other side").masked(key(77)).encode(),
            ),
        };
        pass(&mut relay, &other_frame, other).await;
        relay
            .close()
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        assert_c2(&sink);
        let all = streams(&sink.frames());
        let (mine, theirs) = match side {
            Side::Client => (CaptureSource::WsClient, CaptureSource::WsServer),
            Side::Server => (CaptureSource::WsServer, CaptureSource::WsClient),
        };
        let mine = by_source(&all, mine);
        assert_eq!(
            mine.len(),
            2,
            "{name}: before, then the violation; never 'after'"
        );
        assert_eq!(mine[0].text(), "before", "{name}");
        assert_eq!(mine[0].end, Some(EndReason::Complete), "{name}");
        assert_eq!(mine[1].data, kept, "{name}");
        assert_eq!(mine[1].gaps, [GapReason::DecodeFailed], "{name}");
        assert_eq!(mine[1].end, Some(EndReason::DecodeError), "{name}");
        let theirs = by_source(&all, theirs);
        assert_eq!(theirs.len(), 1, "{name}");
        assert_eq!(theirs[0].text(), "other side", "{name}");
        assert_eq!(theirs[0].end, Some(EndReason::Complete), "{name}");
    }
}

// ---------------------------------------------------------------------------------------
// Capture pressure and interruption
// ---------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disabled_or_saturated_sink_never_holds_back_a_large_exchange() {
    const MESSAGES: usize = 32;
    const MESSAGE_BYTES: usize = 128 * 1024;
    const FRAME_BYTES: usize = 16 * 1024;
    let encode = |client: bool| -> Vec<u8> {
        let mut out = Vec::new();
        for m in 0..MESSAGES {
            let body = pattern(MESSAGE_BYTES, m as u32 + if client { 0 } else { 1000 });
            for (f, chunk) in body.chunks(FRAME_BYTES).enumerate() {
                let frame = if f == 0 {
                    Frame::binary(chunk.to_vec())
                } else {
                    Frame::cont(chunk.to_vec())
                }
                .fin((f + 1) * FRAME_BYTES >= MESSAGE_BYTES);
                let frame = if client {
                    frame.masked(key((m * 100 + f) as u32))
                } else {
                    frame
                };
                out.extend(frame.encode());
            }
        }
        out
    };
    let client_bytes = [encode(true), Frame::close(1000).masked(key(1)).encode()].concat();
    let server_bytes = [encode(false), Frame::close(1000).encode()].concat();

    for (sink, terminal) in [
        (
            RecordingSink::refusing(GapReason::StorageFull, EpochAdmission::Ready),
            true,
        ),
        (
            RecordingSink::refusing(GapReason::QueueFull, EpochAdmission::Pending),
            false,
        ),
    ] {
        let emitter = emitter(&sink);
        let relay = start(&emitter, Setup::captured(CONN_A)).await;
        let Relay {
            mut client,
            mut upstream,
            task,
        } = relay;
        let exchange = async {
            let send_client = async {
                client.tx.write_all(&client_bytes).await.unwrap();
                client.tx.shutdown().await.unwrap();
            };
            let send_server = async {
                upstream.tx.write_all(&server_bytes).await.unwrap();
                upstream.tx.shutdown().await.unwrap();
            };
            let mut at_upstream = Vec::new();
            let mut at_client = Vec::new();
            let receive_upstream = upstream.rx.read_to_end(&mut at_upstream);
            let receive_client = client.rx.read_to_end(&mut at_client);
            let (_, _, up, down) =
                tokio::join!(send_client, send_server, receive_upstream, receive_client);
            up.unwrap();
            down.unwrap();
            (at_upstream, at_client)
        };
        let (at_upstream, at_client) = tokio::time::timeout(DEADLINE, exchange)
            .await
            .expect("the exchange never waits for the evidence queue");
        assert!(
            at_upstream == client_bytes,
            "client bytes forwarded identically"
        );
        assert!(
            at_client == server_bytes,
            "server bytes forwarded identically"
        );
        tokio::time::timeout(DEADLINE, task)
            .await
            .unwrap()
            .unwrap()
            .expect("clean close");

        assert!(sink.frames().is_empty());
        let begins = sink.begins();
        if terminal {
            assert!(begins.is_empty(), "a terminal refusal is never retried");
        } else {
            // Every later message start polled admission once, without waiting on it.
            assert_eq!(begins.len(), 2 * MESSAGES - 1);
            assert!(begins.iter().all(|b| *b == EpochAdmission::Pending));
        }
    }
}

#[tokio::test]
async fn ending_mid_frame_or_mid_message_is_interruption_never_completion() {
    // Mid-frame: a 100-byte frame cut after 10 payload bytes.
    {
        let sink = RecordingSink::new();
        let emitter = emitter(&sink);
        announce(&emitter, CONN_A);
        let mut relay = start(&emitter, Setup::captured(CONN_A)).await;
        let payload = pattern(100, 5);
        let frame = Frame::binary(payload.clone()).masked(key(1)).encode();
        relay.pass_client(&frame[..2 + 4 + 10]).await;
        unexpected_eof(relay.end().await);
        let all = streams(&sink.frames());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].data, payload[..10]);
        assert_eq!(all[0].end, Some(EndReason::Interrupted));
    }
    // Mid-message at a frame boundary, compressed: the decoded prefix stays.
    {
        let sink = RecordingSink::new();
        let emitter = emitter(&sink);
        announce(&emitter, CONN_A);
        let mut relay = start(
            &emitter,
            Setup::captured(CONN_A).extensions("permessage-deflate"),
        )
        .await;
        let first = Deflater::new(true).message(b"half of a reply");
        relay
            .pass_server(&Frame::text(first).rsv(RSV1).fin(false).encode())
            .await;
        unexpected_eof(relay.end().await);
        let all = streams(&sink.frames());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].text(), "half of a reply");
        assert_eq!(all[0].end, Some(EndReason::Interrupted));
    }
    // A 64-bit length with its top bit set: an explicit violation, and a frame that never
    // ends.
    {
        let sink = RecordingSink::new();
        let emitter = emitter(&sink);
        announce(&emitter, CONN_A);
        let mut relay = start(&emitter, Setup::captured(CONN_A)).await;
        relay
            .pass_server(
                &Frame::text("abc")
                    .len(Len::Declared((1 << 63) | 3))
                    .encode(),
            )
            .await;
        unexpected_eof(relay.end().await);
        let all = streams(&sink.frames());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].gaps, [GapReason::DecodeFailed]);
        assert_eq!(all[0].end, Some(EndReason::DecodeError));
    }
    // Both Close frames seen, yet a message was cut (the server's Close interrupts its own
    // fragmented message) or a frame was cut after the handshake: still not a clean close.
    for cut_frame in [false, true] {
        let sink = RecordingSink::new();
        let emitter = emitter(&sink);
        announce(&emitter, CONN_A);
        let mut relay = start(&emitter, Setup::captured(CONN_A)).await;
        if !cut_frame {
            relay
                .pass_server(&Frame::text("cut short").fin(false).encode())
                .await;
        }
        relay
            .pass_client(&Frame::close(1000).masked(key(2)).encode())
            .await;
        relay.pass_server(&Frame::close(1000).encode()).await;
        if cut_frame {
            relay
                .pass_client(&Frame::text("late").masked(key(3)).encode()[..3])
                .await;
        }
        unexpected_eof(relay.end().await);
        let all = streams(&sink.frames());
        if cut_frame {
            assert!(all.is_empty());
        } else {
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].text(), "cut short");
            assert_eq!(all[0].end, Some(EndReason::Interrupted));
        }
    }
    // Whole messages, but no closing handshake, or only half of one.
    for close_client in [false, true] {
        let sink = RecordingSink::new();
        let emitter = emitter(&sink);
        announce(&emitter, CONN_A);
        let mut relay = start(&emitter, Setup::captured(CONN_A)).await;
        relay
            .pass_client(&Frame::text("done").masked(key(1)).encode())
            .await;
        relay.pass_server(&Frame::text("done").encode()).await;
        if close_client {
            relay
                .pass_client(&Frame::close(1000).masked(key(2)).encode())
                .await;
        }
        unexpected_eof(relay.end().await);
        let all = streams(&sink.frames());
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|s| s.end == Some(EndReason::Complete)));
    }
}

#[tokio::test]
async fn the_relay_reads_the_proxy_wide_capture_clock() {
    // WebSocket and HTTP observations of one producer share one monotonic base.
    let sink = RecordingSink::new();
    let emitter = emitter(&sink);
    announce(&emitter, CONN_A);
    let mut relay = start(&emitter, Setup::captured(CONN_A)).await;
    let _ = goalcup_cli::proxy::metadata::capture_time();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let before = goalcup_cli::proxy::metadata::capture_time();
    relay
        .pass_client(&Frame::text("timed").masked(key(1)).encode())
        .await;
    relay.settle_client().await;
    let after = goalcup_cli::proxy::metadata::capture_time();
    relay.close().await.expect("clean close");

    let payload: Vec<CaptureFrame> = sink
        .frames()
        .into_iter()
        .filter(|f| f.stream.stream != 0)
        .collect();
    assert_eq!(payload.len(), 3, "Open, Bytes, End");
    for frame in &payload {
        let mono = frame.observed.mono_ns;
        assert!(
            (before.mono_ns..=after.mono_ns).contains(&mono),
            "{mono} is not on the shared clock between {} and {}",
            before.mono_ns,
            after.mono_ns
        );
        // The shared clock writes nanoseconds: "YYYY-MM-DDTHH:MM:SS.nnnnnnnnnZ".
        assert_eq!(frame.observed.wall.len(), 30, "{}", frame.observed.wall);
    }
}

#[tokio::test]
async fn cancelling_the_relay_ends_an_open_message_interrupted() {
    let sink = RecordingSink::new();
    let emitter = emitter(&sink);
    announce(&emitter, CONN_A);
    let mut relay = start(&emitter, Setup::captured(CONN_A)).await;
    relay
        .pass_client(&Frame::text("unfinished").fin(false).masked(key(1)).encode())
        .await;
    relay.settle_client().await;
    relay.task.abort();
    assert!(relay.task.await.expect_err("aborted").is_cancelled());
    assert_c2(&sink);
    let all = streams(&sink.frames());
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].text(), "unfinished");
    assert_eq!(all[0].end, Some(EndReason::Interrupted));
}

// ---------------------------------------------------------------------------------------
// Recovery
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn ws_recovery_waits_for_next_message_not_continuation() {
    let sink = RecordingSink::new();
    let emitter = emitter(&sink);
    announce(&emitter, CONN_A);
    let mut relay = start(&emitter, Setup::captured(CONN_A)).await;

    relay
        .pass_client(&Frame::text("alpha-").fin(false).masked(key(1)).encode())
        .await;
    relay.settle_client().await;
    // Capture is lost inside the fragmented message.
    sink.refuse(Some(GapReason::QueueFull));
    relay
        .pass_client(&Frame::cont("beta-").fin(false).masked(key(2)).encode())
        .await;
    relay.settle_client().await;
    let before_heal = sink.frames();
    // The sink is ready again, but these are continuations of the lost message.
    sink.refuse(None);
    sink.admit(EpochAdmission::Ready);
    relay
        .pass_client(&Frame::cont("gamma-").fin(false).masked(key(3)).encode())
        .await;
    relay
        .pass_client(&Frame::cont("delta").masked(key(4)).encode())
        .await;
    relay.settle_client().await;
    assert!(
        sink.begins().is_empty(),
        "no recovery attempt inside a message"
    );
    assert_eq!(sink.frames(), before_heal, "continuations reopened nothing");

    // A newly begun message is the first safe boundary.
    relay
        .pass_client(&Frame::text("fresh").masked(key(5)).encode())
        .await;
    relay.pass_server(&Frame::text("reply").encode()).await;
    relay.close().await.expect("clean close");

    assert_eq!(sink.begins(), [EpochAdmission::Ready]);
    assert_c2(&sink);
    let frames = sink.frames();
    let all = streams(&frames);
    let texts: Vec<(String, u64, String)> = all
        .iter()
        .map(|s| (s.key.producer.clone(), s.index(), s.text()))
        .collect();
    let fresh = all[1].key.producer.clone();
    assert_ne!(fresh, INITIAL);
    assert_eq!(
        texts,
        [
            (INITIAL.to_string(), 0, "alpha-".to_string()),
            (fresh.clone(), 1, "fresh".to_string()),
            (fresh.clone(), 2, "reply".to_string()),
        ]
    );
    assert_eq!(all[0].end, None, "the lost message is never completed");
    for stream in &all {
        for lost in ["beta", "gamma", "delta"] {
            assert!(!stream.text().contains(lost), "{lost} was captured");
        }
    }
    // Recovery precedes everything the fresh producer emits.
    let first_fresh = frames
        .iter()
        .find(|f| f.stream.producer == fresh)
        .expect("fresh producer");
    assert!(matches!(
        first_fresh.event,
        FrameEvent::Recovery {
            reason: GapReason::QueueFull,
            ..
        }
    ));
}

#[tokio::test]
async fn recovery_preserves_connection_identity_and_sticky_gap() {
    let sink = RecordingSink::new();
    let emitter = emitter(&sink);
    announce(&emitter, CONN_A);
    let mut relay = start(&emitter, Setup::captured(CONN_A)).await;

    relay
        .pass_client(&Frame::text("c0").masked(key(1)).encode())
        .await;
    relay.pass_server(&Frame::text("s1").encode()).await;
    relay
        .pass_server(&Frame::text("s2 part one, ").fin(false).encode())
        .await;
    relay.settle_server().await;
    sink.refuse(Some(GapReason::QueueFull));
    relay
        .pass_client(&Frame::text("c3").masked(key(2)).encode())
        .await;
    relay.settle_client().await;
    relay
        .pass_server(&Frame::cont("s2 part two").encode())
        .await;
    relay.settle_server().await;
    sink.refuse(None);
    sink.admit(EpochAdmission::Pending);
    relay.pass_server(&Frame::text("s4").encode()).await;
    relay.settle_server().await;
    sink.admit(EpochAdmission::Ready);
    relay
        .pass_client(&Frame::text("c5").masked(key(3)).encode())
        .await;
    relay.pass_server(&Frame::text("s6").encode()).await;
    relay.close().await.expect("clean close");

    assert_eq!(
        sink.begins(),
        [EpochAdmission::Pending, EpochAdmission::Ready]
    );
    assert_c2(&sink);
    let frames = sink.frames();
    let all = streams(&frames);
    let producers: Vec<&str> = all.iter().map(|s| s.key.producer.as_str()).collect();
    let fresh = producers[3].to_string();
    assert_ne!(fresh, INITIAL);
    assert_eq!(producers, [INITIAL, INITIAL, INITIAL, &fresh, &fresh]);
    // Stable physical connection and handshake exchange, increasing indexes; 3 and 4 were
    // observed but not recorded, and the hole says so.
    for stream in &all {
        let wire = stream.meta.wire.as_ref().unwrap();
        assert_eq!(
            (wire.connection.as_str(), wire.exchange),
            (CONN_A, EXCHANGE)
        );
    }
    let indexes: Vec<u64> = all.iter().map(Captured::index).collect();
    assert_eq!(indexes, [0, 1, 2, 5, 6]);
    let texts: Vec<String> = all.iter().map(Captured::text).collect();
    assert_eq!(texts, ["c0", "s1", "s2 part one, ", "c5", "s6"]);
    // The message in flight at the loss stays incomplete; nothing after it claims its bytes.
    assert_eq!(all[2].end, None);
    for (i, stream) in all.iter().enumerate() {
        if i != 2 {
            assert_eq!(stream.end, Some(EndReason::Complete));
        }
    }
    // The fresh producer starts with Recovery, then re-observes the connection, then payload.
    let fresh_frames: Vec<&CaptureFrame> = frames
        .iter()
        .filter(|f| f.stream.producer == fresh)
        .collect();
    assert_eq!(
        fresh_frames[0].event,
        FrameEvent::Recovery {
            previous_producer: INITIAL.to_string(),
            reason: GapReason::QueueFull,
        }
    );
    assert_eq!(
        fresh_frames[1].event,
        FrameEvent::Connection {
            connection: CONN_A.to_string(),
            event: ConnectionEvent::Open {
                alpn: Some("http/1.1".to_string()),
                already_open: true,
            },
        }
    );
    assert_eq!(
        fresh_frames[2].event,
        FrameEvent::Connection {
            connection: CONN_A.to_string(),
            event: ConnectionEvent::UpgradedWebSocket,
        }
    );
    assert!(matches!(fresh_frames[3].event, FrameEvent::Open { .. }));
    // Recorder-only recovery sent nothing to either peer: each `pass_*` read exactly what
    // the other side wrote, and `close` saw EOF with nothing extra (no replay, no input).
}

#[tokio::test]
async fn a_final_deflate_block_ends_the_takeover_history_explicitly() {
    // RFC 7692 7.2.3.4: "Hello" in a DEFLATE block with BFINAL set.
    let example = fixture()
        .examples
        .into_iter()
        .find(|example| example.name == "rfc7692-7.2.3.4-deflate-block-with-bfinal-set")
        .expect("the BFINAL example");
    let bfinal = hex(&example.frames[0].hex);
    for (extensions, takeover) in [
        ("permessage-deflate", true),
        ("permessage-deflate; server_no_context_takeover", false),
    ] {
        let sink = RecordingSink::new();
        let emitter = emitter(&sink);
        announce(&emitter, CONN_A);
        let mut relay = start(&emitter, Setup::captured(CONN_A).extensions(extensions)).await;
        relay.pass_server(&bfinal).await;
        let next = Deflater::new(false).message(b"after the final block");
        relay
            .pass_server(&Frame::text(next).rsv(RSV1).encode())
            .await;
        relay.close().await.expect("clean close");

        assert_c2(&sink);
        let all = streams(&sink.frames());
        assert_eq!(all.len(), 2, "{extensions}");
        assert_eq!(all[0].text(), "Hello", "{extensions}");
        assert_eq!(all[0].end, Some(EndReason::Complete));
        if takeover {
            // Past the final block there is no history to continue: never an empty Complete.
            assert!(all[1].data.is_empty(), "{extensions}");
            assert_eq!(all[1].gaps, [GapReason::DecodeFailed], "{extensions}");
            assert_eq!(all[1].end, Some(EndReason::DecodeError), "{extensions}");
        } else {
            assert_eq!(all[1].text(), "after the final block", "{extensions}");
            assert_eq!(all[1].end, Some(EndReason::Complete), "{extensions}");
        }
    }
}

#[tokio::test]
async fn a_violation_inside_a_lost_message_recovers_only_at_the_next_message() {
    let sink = RecordingSink::new();
    let emitter = emitter(&sink);
    announce(&emitter, CONN_A);
    let mut relay = start(&emitter, Setup::captured(CONN_A)).await;

    relay
        .pass_client(&Frame::text("lost-").fin(false).masked(key(1)).encode())
        .await;
    relay.settle_client().await;
    sink.refuse(Some(GapReason::QueueFull));
    relay
        .pass_client(&Frame::cont("more-").fin(false).masked(key(2)).encode())
        .await;
    relay.settle_client().await;
    let before_heal = sink.frames();
    sink.refuse(None);
    sink.admit(EpochAdmission::Ready);
    // A continuation of the lost message that also breaks the protocol (RSV2).
    relay
        .pass_client(
            &Frame::cont("bad")
                .rsv(RSV2)
                .fin(false)
                .masked(key(3))
                .encode(),
        )
        .await;
    relay.settle_client().await;
    assert!(sink.begins().is_empty(), "no recovery inside a message");
    assert_eq!(sink.frames(), before_heal, "nothing recorded mid-message");
    // The client direction is off now; its next message recovers nothing either.
    relay
        .pass_client(&Frame::text("after").masked(key(4)).encode())
        .await;
    relay.settle_client().await;
    assert!(sink.begins().is_empty());
    // The first new message of a capturing direction is the boundary.
    relay.pass_server(&Frame::text("reply").encode()).await;
    relay.close().await.expect("clean close");

    assert_eq!(sink.begins(), [EpochAdmission::Ready]);
    assert_c2(&sink);
    let all = streams(&sink.frames());
    let seen: Vec<(bool, u64, String, Option<EndReason>)> = all
        .iter()
        .map(|s| (s.key.producer == INITIAL, s.index(), s.text(), s.end))
        .collect();
    // Index 1 is the client's "after": observed, never recorded.
    assert_eq!(
        seen,
        [
            (true, 0, "lost-".to_string(), None),
            (false, 2, "reply".to_string(), Some(EndReason::Complete)),
        ]
    );
}

#[tokio::test]
async fn a_violation_in_a_never_recorded_message_keeps_that_messages_index() {
    let sink = RecordingSink::new();
    let emitter = emitter(&sink);
    announce(&emitter, CONN_A);
    announce(&emitter, CONN_B);
    let mut a = start(&emitter, Setup::captured(CONN_A)).await;
    let mut b = start(&emitter, Setup::captured(CONN_B)).await;

    a.pass_client(&Frame::text("a0").masked(key(1)).encode())
        .await;
    sink.refuse(Some(GapReason::QueueFull));
    b.pass_server(&Frame::text("b0").encode()).await;
    b.settle_server().await;
    sink.refuse(None);
    sink.admit(EpochAdmission::Pending);
    // Starts while the epoch is down: index 1 of A, never recorded.
    a.pass_client(&Frame::text("a1-").fin(false).masked(key(2)).encode())
        .await;
    a.settle_client().await;
    // Another connection restores the epoch at its own message boundary.
    sink.admit(EpochAdmission::Ready);
    b.pass_server(&Frame::text("b1").encode()).await;
    b.settle_server().await;
    // A continuation of A's unrecorded message breaks the protocol: recorded under that
    // message's own index in the live epoch, with no recovery of its own.
    a.pass_client(
        &Frame::cont("bad")
            .rsv(RSV2)
            .fin(false)
            .masked(key(3))
            .encode(),
    )
    .await;
    a.pass_client(&Frame::cont("end").masked(key(4)).encode())
        .await;
    a.close().await.expect("clean close");
    b.close().await.expect("clean close");

    assert_eq!(
        sink.begins(),
        [EpochAdmission::Pending, EpochAdmission::Ready]
    );
    assert_c2(&sink);
    let all = streams(&sink.frames());
    // (initial producer, message index, text, gaps, end)
    type Seen = (bool, u64, String, Vec<GapReason>, Option<EndReason>);
    let seen = |connection| -> Vec<Seen> {
        of_connection(&all, connection)
            .iter()
            .map(|s| {
                let initial = s.key.producer == INITIAL;
                (initial, s.index(), s.text(), s.gaps.clone(), s.end)
            })
            .collect()
    };
    assert_eq!(
        seen(CONN_A),
        [
            (true, 0, "a0".to_string(), vec![], Some(EndReason::Complete)),
            (
                false,
                1,
                String::new(),
                vec![GapReason::DecodeFailed],
                Some(EndReason::DecodeError)
            ),
        ]
    );
    assert_eq!(
        seen(CONN_B),
        [(
            false,
            1,
            "b1".to_string(),
            vec![],
            Some(EndReason::Complete)
        )]
    );
}

#[tokio::test]
async fn a_direction_whose_capture_is_off_still_consumes_message_indexes() {
    let sink = RecordingSink::new();
    let emitter = emitter(&sink);
    announce(&emitter, CONN_A);
    let mut relay = start(&emitter, Setup::captured(CONN_A)).await;

    relay
        .pass_client(&Frame::text("before").masked(key(1)).encode())
        .await;
    // Unmasked: the client direction's capture ends here.
    relay.pass_client(&Frame::text("unmasked").encode()).await;
    relay.pass_server(&Frame::text("s1").encode()).await;
    relay
        .pass_client(&Frame::text("c-after-1").masked(key(2)).encode())
        .await;
    relay.pass_server(&Frame::text("s2").encode()).await;
    // A fragmented message with a ping inside counts once; control frames never count.
    relay
        .pass_client(&Frame::text("c-after-2 ").fin(false).masked(key(3)).encode())
        .await;
    relay
        .pass_client(&Frame::ping("ping").masked(key(4)).encode())
        .await;
    relay
        .pass_client(&Frame::cont("end").masked(key(5)).encode())
        .await;
    relay.pass_server(&Frame::text("s3").encode()).await;
    relay.close().await.expect("clean close");

    assert_c2(&sink);
    let all = streams(&sink.frames());
    let seen: Vec<(CaptureSource, u64, Option<EndReason>)> =
        all.iter().map(|s| (s.source, s.index(), s.end)).collect();
    assert_eq!(
        seen,
        [
            (CaptureSource::WsClient, 0, Some(EndReason::Complete)),
            (CaptureSource::WsClient, 1, Some(EndReason::DecodeError)),
            (CaptureSource::WsServer, 2, Some(EndReason::Complete)),
            (CaptureSource::WsServer, 4, Some(EndReason::Complete)),
            (CaptureSource::WsServer, 6, Some(EndReason::Complete)),
        ],
        "holes at 3 and 5 are the client messages capture no longer recorded"
    );
}

#[tokio::test]
async fn takeover_history_survives_messages_whose_capture_was_lost() {
    let sink = RecordingSink::new();
    let emitter = emitter(&sink);
    announce(&emitter, CONN_A);
    let mut relay = start(
        &emitter,
        Setup::captured(CONN_A).extensions("permessage-deflate"),
    )
    .await;
    // Message 3 repeats tokens that exist only in the lost tail of message 1 and in the
    // never-captured message 2, so only a receiver that inflated both can decode it.
    let texts: Vec<Vec<u8>> = [
        "message zero: a common opening line that every message shares. ",
        "message one opens normally and ends with a token found nowhere else: \
         QUEBEC-7731-violet-harbor-QUEBEC-7731-violet-harbor. ",
        "message two is never captured; it alone carries XRAY-4096-crimson-lantern-XRAY.",
        "three: XRAY-4096-crimson-lantern-XRAY then QUEBEC-7731-violet-harbor-QUEBEC-7731-\
         violet-harbor, a common opening line that every message shares.",
    ]
    .iter()
    .map(|text| text.as_bytes().to_vec())
    .collect();
    let mut server = Deflater::new(true);
    let compressed: Vec<Vec<u8>> = texts.iter().map(|text| server.message(text)).collect();
    let with_all: Vec<&[u8]> = compressed.iter().map(Vec::as_slice).collect();
    assert_eq!(
        inflate_in_order(&with_all).pop().unwrap().unwrap(),
        texts[3],
        "the fixture decodes with the whole history"
    );
    let without_two = [&compressed[0][..], &compressed[1][..], &compressed[3][..]];
    assert_ne!(
        inflate_in_order(&without_two).pop().unwrap().ok().as_ref(),
        Some(&texts[3]),
        "the fixture needs message two's history"
    );

    relay
        .pass_server(&Frame::text(compressed[0].clone()).rsv(RSV1).encode())
        .await;
    // Lost in the middle of message 1, and message 2 is never captured at all.
    let half = compressed[1].len() / 2;
    relay
        .pass_server(
            &Frame::text(compressed[1][..half].to_vec())
                .rsv(RSV1)
                .fin(false)
                .encode(),
        )
        .await;
    relay.settle_server().await;
    sink.refuse(Some(GapReason::QueueFull));
    relay
        .pass_server(&Frame::cont(compressed[1][half..].to_vec()).encode())
        .await;
    relay.settle_server().await;
    sink.refuse(None);
    sink.admit(EpochAdmission::Pending);
    relay
        .pass_server(&Frame::text(compressed[2].clone()).rsv(RSV1).encode())
        .await;
    relay.settle_server().await;
    // Recovered: message 3 references the window messages 1 and 2 built.
    sink.admit(EpochAdmission::Ready);
    relay
        .pass_server(&Frame::text(compressed[3].clone()).rsv(RSV1).encode())
        .await;
    relay.close().await.expect("clean close");

    assert_c2(&sink);
    let all = streams(&sink.frames());
    let seen: Vec<(u64, Option<EndReason>)> = all.iter().map(|s| (s.index(), s.end)).collect();
    assert_eq!(
        seen,
        [
            (0, Some(EndReason::Complete)),
            (1, None),
            (3, Some(EndReason::Complete)),
        ]
    );
    assert_eq!(all[0].data, texts[0]);
    assert_eq!(
        all[2].data, texts[3],
        "decoded with the full, uninterrupted history"
    );
}

#[tokio::test]
async fn two_connections_with_same_payload_keep_distinct_parents() {
    let sink = RecordingSink::new();
    let emitter = emitter(&sink);
    announce(&emitter, CONN_A);
    announce(&emitter, CONN_B);
    let mut a = start(&emitter, Setup::captured(CONN_A)).await;
    let mut b = start(&emitter, Setup::captured(CONN_B)).await;
    let first = Frame::text("same ").fin(false).masked(key(1)).encode();
    let second = Frame::cont("payload").masked(key(2)).encode();
    a.pass_client(&first).await;
    b.pass_client(&first).await;
    b.pass_client(&second).await;
    a.pass_client(&second).await;
    b.pass_server(&Frame::text("same reply").encode()).await;
    a.pass_server(&Frame::text("same reply").encode()).await;
    a.close().await.expect("clean close");
    b.close().await.expect("clean close");

    assert_c2(&sink);
    let all = streams(&sink.frames());
    assert_eq!(all.len(), 4);
    for connection in [CONN_A, CONN_B] {
        let mine = of_connection(&all, connection);
        let seen: Vec<(CaptureSource, u64, String)> = mine
            .iter()
            .map(|s| (s.source, s.index(), s.text()))
            .collect();
        assert_eq!(
            seen,
            [
                (CaptureSource::WsClient, 0, "same payload".to_string()),
                (CaptureSource::WsServer, 1, "same reply".to_string()),
            ],
            "{connection}"
        );
        assert!(mine.iter().all(|s| s.end == Some(EndReason::Complete)));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_and_websocket_relays_share_one_sequence_and_stream_allocator() {
    const HTTP_EXCHANGES: u64 = 300;
    const WS_ROUNDS: usize = 100;
    let sink = RecordingSink::new();
    let emitter = emitter(&sink);
    announce(&emitter, CONN_A);
    let http_emitter = emitter.clone();
    // A stand-in for the HTTP relay on another thread, sharing the same emitter.
    let http = std::thread::spawn(move || {
        for exchange in 0..HTTP_EXCHANGES {
            let metadata = CaptureMeta {
                wire: Some(WireIdentity {
                    connection: CONN_B.to_string(),
                    exchange,
                    message_index: None,
                }),
                label: "http request".to_string(),
                protocol_hint: None,
                method: Some("POST".to_string()),
                host: Some("api.example.test".to_string()),
                path: Some("/v1/responses".to_string()),
                status: None,
                content_type: None,
            };
            let key = http_emitter
                .open(CaptureSource::WireRequest, metadata, time())
                .expect("open");
            let bytes = FrameEvent::Bytes {
                offset: 0,
                data: b"{\"http\":true}".to_vec(),
            };
            assert_eq!(http_emitter.emit(&key, time(), bytes), SubmitResult::Queued);
            let end = FrameEvent::End {
                reason: EndReason::Complete,
            };
            assert_eq!(http_emitter.emit(&key, time(), end), SubmitResult::Queued);
        }
    });
    let mut relay = start(&emitter, Setup::captured(CONN_A)).await;
    for round in 0..WS_ROUNDS {
        relay
            .pass_client(
                &Frame::text(format!("request {round}, "))
                    .fin(false)
                    .masked(key(round as u32))
                    .encode(),
            )
            .await;
        relay
            .pass_client(&Frame::cont("done").masked(key(7)).encode())
            .await;
        relay
            .pass_server(&Frame::text(format!("reply {round}")).encode())
            .await;
    }
    relay.close().await.expect("clean close");
    http.join().expect("http thread");

    // One producer: its seq and stream IDs are unique and contiguous across both relays.
    assert_c2(&sink);
    let all = streams(&sink.frames());
    let http_streams = by_source(&all, CaptureSource::WireRequest);
    assert_eq!(http_streams.len() as u64, HTTP_EXCHANGES);
    let ws: Vec<&Captured> = all
        .iter()
        .filter(|s| matches!(s.source, CaptureSource::WsClient | CaptureSource::WsServer))
        .collect();
    assert_eq!(ws.len(), 2 * WS_ROUNDS);
    let indexes: Vec<u64> = ws.iter().map(|s| s.index()).collect();
    assert_eq!(indexes, (0..2 * WS_ROUNDS as u64).collect::<Vec<_>>());
    assert!(all.iter().all(|s| s.end == Some(EndReason::Complete)));
    let mut keys: Vec<u64> = all.iter().map(|s| s.key.stream).collect();
    keys.dedup();
    assert_eq!(keys.len(), all.len());
}

// ---------------------------------------------------------------------------------------
// Bounded working memory
// ---------------------------------------------------------------------------------------

/// Counts heap bytes per thread. The memory test runs the relay and both peers on its own
/// thread (current-thread runtime), so that thread's peak is the relay's working set plus the
/// test's own fixed buffers.
struct CountingAllocator;

thread_local! {
    static LIVE: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
}

fn account(delta: isize) {
    let _ = LIVE.try_with(|live| {
        let now = live.get() + delta;
        live.set(now);
        let _ = PEAK.try_with(|peak| peak.set(peak.get().max(now)));
    });
}

// SAFETY: every call forwards to `System` unchanged; only a thread-local counter is added.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            account(layout.size() as isize);
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            account(layout.size() as isize);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        account(-(layout.size() as isize));
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let moved = unsafe { System.realloc(ptr, layout, new_size) };
        if !moved.is_null() {
            account(new_size as isize - layout.size() as isize);
        }
        moved
    }
}

#[global_allocator]
static COUNTING: CountingAllocator = CountingAllocator;

/// Checks each streamed byte against the known source and keeps nothing.
#[derive(Default)]
struct StreamingState {
    sources: BTreeMap<u64, CaptureSource>,
    received: BTreeMap<u64, u64>,
    mismatched: u64,
    ends: Vec<(CaptureSource, EndReason)>,
}

struct StreamingSink {
    state: Mutex<StreamingState>,
    client_pattern: Vec<u8>,
}

fn server_byte(offset: u64) -> u8 {
    (offset.wrapping_mul(31) >> 3) as u8
}

impl CaptureSink for StreamingSink {
    fn try_submit(&self, frame: CaptureFrame) -> SubmitResult {
        let mut state = self.state.lock().unwrap();
        let stream = frame.stream.stream;
        match frame.event {
            FrameEvent::Open { source, .. } => {
                state.sources.insert(stream, source);
                state.received.insert(stream, 0);
            }
            FrameEvent::Bytes { offset, data } => {
                let source = state.sources[&stream];
                let expected_offset = state.received[&stream];
                let mut bad = u64::from(offset != expected_offset);
                for (i, byte) in data.iter().enumerate() {
                    let at = offset + i as u64;
                    let want = match source {
                        CaptureSource::WsClient => {
                            self.client_pattern[(at % self.client_pattern.len() as u64) as usize]
                        }
                        _ => server_byte(at),
                    };
                    bad += u64::from(*byte != want);
                }
                state.mismatched += bad;
                *state.received.get_mut(&stream).unwrap() += data.len() as u64;
            }
            FrameEvent::End { reason } => {
                let source = state.sources[&stream];
                state.ends.push((source, reason));
            }
            _ => {}
        }
        SubmitResult::Queued
    }

    fn try_begin_epoch(&self, _producer: &str) -> EpochAdmission {
        EpochAdmission::Ready
    }

    fn try_close_epoch(&self, _producer: &str, _final_seq: Option<u64>) -> SubmitResult {
        SubmitResult::Queued
    }
}

#[tokio::test(flavor = "current_thread")]
async fn working_memory_stays_bounded_while_huge_messages_stream_through() {
    const CLIENT_DECODED: usize = 12 * 1024 * 1024;
    const SERVER_BYTES: usize = 8 * 1024 * 1024;
    const FRAME_BYTES: usize = 16 * 1024;
    const LIMIT: isize = 2 * 1024 * 1024;

    // Prepared before the measurement: one compressed 12 MiB client message.
    let client_pattern = pattern(4096, 9);
    let mut deflater = Deflater::fast(true);
    let mut compressed = Vec::new();
    for _ in 0..CLIENT_DECODED / client_pattern.len() {
        deflater.partial(&client_pattern, &mut compressed);
    }
    deflater.finish(&mut compressed);
    let mut client_bytes = Vec::new();
    for (i, chunk) in compressed.chunks(8 * 1024).enumerate() {
        let frame = if i == 0 {
            Frame::text(chunk.to_vec()).rsv(RSV1)
        } else {
            Frame::cont(chunk.to_vec())
        };
        let last = (i + 1) * 8 * 1024 >= compressed.len();
        client_bytes.extend(frame.fin(last).masked(key(i as u32)).encode());
    }
    client_bytes.extend(Frame::close(1000).masked(key(1)).encode());
    let server_close = Frame::close(1000).encode();

    let sink = Arc::new(StreamingSink {
        state: Mutex::new(StreamingState::default()),
        client_pattern: client_pattern.clone(),
    });
    let emitter = Arc::new(goalcup_cli::proxy::api::CaptureEmitter::new(
        INITIAL.to_string(),
        sink.clone(),
    ));
    let relay = start(
        &emitter,
        Setup::captured(CONN_A).extensions("permessage-deflate"),
    )
    .await;
    let Relay {
        mut client,
        mut upstream,
        task,
    } = relay;
    let mut buffers = (
        vec![0u8; FRAME_BYTES],
        vec![0u8; FRAME_BYTES],
        vec![0u8; FRAME_BYTES],
    );

    let baseline = LIVE.with(Cell::get);
    PEAK.with(|peak| peak.set(baseline));
    let (read_up, read_down, server_frame) = (&mut buffers.0, &mut buffers.1, &mut buffers.2);
    let exchange = async {
        let send_client = async {
            for piece in client_bytes.chunks(8 * 1024) {
                client.tx.write_all(piece).await.unwrap();
            }
            client.tx.shutdown().await.unwrap();
        };
        // The server message is generated frame by frame into one reused buffer.
        let send_server = async {
            let mut offset = 0u64;
            let mut first = true;
            while (offset as usize) < SERVER_BYTES {
                let len = FRAME_BYTES.min(SERVER_BYTES - offset as usize);
                let fin = offset as usize + len == SERVER_BYTES;
                let opcode = if first { OP_BINARY } else { OP_CONT };
                let header = [
                    if fin { 0x80 } else { 0 } | opcode,
                    126,
                    (len >> 8) as u8,
                    len as u8,
                ];
                for (i, byte) in server_frame[..len].iter_mut().enumerate() {
                    *byte = server_byte(offset + i as u64);
                }
                upstream.tx.write_all(&header).await.unwrap();
                upstream.tx.write_all(&server_frame[..len]).await.unwrap();
                offset += len as u64;
                first = false;
            }
            upstream.tx.write_all(&server_close).await.unwrap();
            upstream.tx.shutdown().await.unwrap();
        };
        let receive_upstream = async {
            let mut at = 0usize;
            loop {
                let read = upstream.rx.read(read_up).await.unwrap();
                if read == 0 {
                    break;
                }
                assert!(
                    read_up[..read] == client_bytes[at..at + read],
                    "client bytes differ"
                );
                at += read;
            }
            assert_eq!(at, client_bytes.len());
        };
        let receive_client = async {
            let mut at = 0usize;
            loop {
                let read = client.rx.read(read_down).await.unwrap();
                if read == 0 {
                    break;
                }
                at += read;
            }
            // Frame headers every 16 KiB plus the close; content is checked by the sink.
            assert!(at > SERVER_BYTES);
        };
        tokio::join!(send_client, send_server, receive_upstream, receive_client);
    };
    tokio::time::timeout(DEADLINE * 4, exchange)
        .await
        .expect("the exchange completes");
    tokio::time::timeout(DEADLINE, task)
        .await
        .unwrap()
        .unwrap()
        .expect("clean close");
    let growth = PEAK.with(Cell::get) - baseline;
    eprintln!(
        "working-memory growth {growth} bytes; {} compressed client bytes relayed",
        client_bytes.len()
    );

    let state = sink.state.lock().unwrap();
    assert_eq!(state.mismatched, 0);
    let totals: Vec<(CaptureSource, u64)> = state
        .received
        .iter()
        .map(|(stream, bytes)| (state.sources[stream], *bytes))
        .collect();
    assert_eq!(
        totals,
        [
            (CaptureSource::WsClient, CLIENT_DECODED as u64),
            (CaptureSource::WsServer, SERVER_BYTES as u64),
        ]
    );
    assert_eq!(
        state.ends,
        [
            (CaptureSource::WsClient, EndReason::Complete),
            (CaptureSource::WsServer, EndReason::Complete),
        ]
    );
    assert!(
        growth < LIMIT,
        "working memory grew by {growth} bytes while {CLIENT_DECODED} + {SERVER_BYTES} bytes streamed"
    );
}
