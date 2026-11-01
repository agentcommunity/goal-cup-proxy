//! `conn_bytes` (PRD-08 section 3, "Honesty"): per decrypted connection and direction, the
//! plaintext the END tail claims against what its streams account for. A request lost on a
//! reused connection with no `conn` GAP is a `fail` (exit 2), even beside a small GAP on another
//! stream; a `conn` GAP covers it (`named`, exit 1); 10% and 1 MiB unaccounted is a `warn` that
//! never moves the exit; no plain tails is a `skip`; a sized WebSocket stream GAP names its bytes.

mod common;

use common::*;
use goalcup_format::{EndReason, FLAG_FIN, GapReason, StreamKind};
use serde_json::{Value, json};

const MIB: u64 = 1_048_576;

/// One decrypted listed connection that claims `plain` (up, down) plaintext bytes. Each pair in
/// `wire` is a request and its response with that many body bytes on the wire (`conn_bytes`
/// trusts END `wire`; `seen` stays equal to the tiny DATA). `lost` adds a `conn` GAP.
fn conn_with(plain: (u64, u64), wire: &[(u64, u64)], lost: Option<u64>) -> (i32, Value) {
    conn_gapped(plain, wire, lost, None)
}

/// As `conn_with`, plus a small `memory_guard` GAP on the last response stream (`stream_lost`).
fn conn_gapped(
    plain: (u64, u64),
    wire: &[(u64, u64)],
    lost: Option<u64>,
    stream_lost: Option<u64>,
) -> (i32, Value) {
    let mut e = Entry::new("conn-bytes");
    let c = e.conn();
    for (i, &(up, down)) in wire.iter().enumerate() {
        let head = json!({"method": "POST", "path": "/v1/responses", "ver": "1.1",
            "headers": [], "masked": [], "coding": null});
        let q = e.open(StreamKind::HttpReq, c, 0, head);
        e.data(q, 0, 0, b"{}");
        e.end(q, EndReason::Complete, 2, json!({"wire": up}));
        let head = json!({"status": 200, "ver": "1.1", "headers": [], "masked": [],
            "coding": null, "decoded": true});
        let p = e.open(StreamKind::HttpResp, c, q, head);
        e.data(p, 0, 0, b"{}");
        let mut seen = 2;
        if let (Some(lost), true) = (stream_lost, i == wire.len() - 1) {
            let tail = json!({"guard": "held_bytes"});
            e.gap(p, GapReason::MemoryGuard, 10, 2, lost, tail);
            seen += lost.saturating_sub(1);
        }
        e.end(p, EndReason::Complete, seen, json!({"wire": down}));
    }
    if let Some(lost) = lost {
        e.gap(c, GapReason::DecodeFailed, 10, 0, lost, Value::Null);
    }
    let tail = json!({"up": plain.0 + 64, "down": plain.1 + 64,
        "plain_up": plain.0, "plain_down": plain.1});
    e.end(c, EndReason::Complete, 0, tail);
    e.count();
    check(&e.seal(), &[])
}

#[test]
fn normal_framing_passes() {
    let (code, v) = conn_with((3 * MIB, 3 * MIB), &[(3 * MIB - 600, 3 * MIB - 600)], None);
    assert_eq!(result(&v, "conn_bytes"), "pass");
    assert_eq!(code, 0);
}

/// The 1.0 parser fault: the second request on a reused connection vanished, no `conn` GAP.
#[test]
fn a_lost_request_without_a_conn_gap_fails() {
    let (code, v) = conn_with((3 * MIB, MIB), &[(MIB, MIB)], None);
    assert_eq!(result(&v, "conn_bytes"), "fail");
    assert_eq!(code, 2);
}

/// A small named GAP on one response must not hide a request the parser lost on the same
/// connection: the loss stays silent, so it stays a `fail`.
#[test]
fn a_small_stream_gap_does_not_cover_a_silently_lost_request() {
    let (code, v) = conn_gapped((3 * MIB, MIB), &[(MIB, MIB)], None, Some(11));
    assert_eq!(ids_with(&v, "fail"), ["conn_bytes"]);
    assert_eq!(code, 2);
}

#[test]
fn the_same_loss_under_a_conn_gap_is_named() {
    let (code, v) = conn_with((3 * MIB, MIB), &[(MIB, MIB)], Some(2 * MIB + 1));
    assert_ne!(result(&v, "conn_bytes"), "fail");
    assert_eq!(result(&v, "listed_hosts_decrypted"), "named");
    assert_eq!(code, 1);
}

#[test]
fn fifteen_percent_over_1_mib_warns_and_the_exit_stays_0() {
    let (code, v) = conn_with((MIB, 10 * MIB), &[(MIB, 17 * MIB / 2)], None);
    assert_eq!(result(&v, "conn_bytes"), "warn");
    assert_eq!(code, 0);
}

/// PRD-01 section 3 makes the tails required: without them nothing was judged, so no `pass`.
#[test]
fn a_decrypted_conn_without_plain_tails_is_skipped_not_passed() {
    let mut e = Entry::new("no-tails");
    let c = e.conn();
    e.end(c, EndReason::Complete, 0, json!({"up": 64, "down": 64}));
    e.count();
    let (code, v) = check(&e.seal(), &[]);
    assert_eq!(result(&v, "conn_bytes"), "skip");
    assert_eq!(code, 0);
}

/// A WebSocket stream has no END `wire`; a GAP on it with its loss known names those bytes.
#[test]
fn a_ws_stream_gap_names_its_lost_bytes() {
    let mut e = Entry::new("ws-gap");
    let c = e.conn();
    let head = json!({"method": "GET", "path": "/v1/responses", "ver": "1.1",
        "headers": [["upgrade", "websocket"]], "masked": [], "coding": null});
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    e.end(q, EndReason::Complete, 0, Value::Null);
    let up = e.open(StreamKind::WsUp, c, q, json!({"deflate": false}));
    e.data(up, 0, FLAG_FIN, &[b'a'; 1000]);
    e.gap(
        up,
        GapReason::MemoryGuard,
        10,
        1000,
        2 * MIB + 1,
        json!({"guard": "held_bytes"}),
    );
    e.end(up, EndReason::Complete, 1000 + 2 * MIB, Value::Null);
    let tail = json!({"up": 3 * MIB, "down": 64, "plain_up": 2 * MIB + 1000, "plain_down": 0});
    e.end(c, EndReason::Complete, 0, tail);
    e.count();
    let (code, v) = check(&e.seal(), &[]);
    assert_eq!(result(&v, "conn_bytes"), "pass");
    assert_eq!(code, 1, "{:?}", ids_with(&v, "fail"));
}
