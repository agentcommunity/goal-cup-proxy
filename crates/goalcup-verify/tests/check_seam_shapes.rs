//! The two record shapes PRD-02's mask worker writes (PRD-08 slice 7; points 1, 2 and 3): a
//! `mask_failed` GAP for metadata (lost 1, tail names the part, on the stream or on sid 0 for an
//! EVENT) and an empty DATA with fin (an empty WebSocket message). A real entry holding them
//! passes the format checks and the recount; the run 1 shape still fails `gap_has_end`.

mod common;

use common::*;
use goalcup_format::{EndReason, FLAG_FIN, GapReason, StreamKind};
use serde_json::{Value, json};

fn no_fail(v: &Value) {
    assert!(
        ids_with(v, "fail").is_empty(),
        "failed: {:?}",
        ids_with(v, "fail")
    );
}

/// A POST on a decrypted connection with a metadata GAP at its OPEN and one before its END, an
/// EVENT GAP on sid 0, and a WebSocket whose client and server each send an empty message.
fn entry() -> Entry {
    let mut e = Entry::new("seam-shapes");
    e.event(json!({"ev": "launch", "pid": 1, "program": "fake-harness"}));
    let c = e.conn();
    let head = json!({"method": "POST", "path": "/v1/responses", "ver": "1.1",
        "headers": [], "masked": [], "coding": null});
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    e.gap(q, GapReason::MaskFailed, 0, 0, 1, json!({"meta": "open"}));
    let body = br#"{"model":"m1","input":"hi"}"#;
    e.data(q, 0, 0, body);
    e.gap(q, GapReason::MaskFailed, 0, body.len() as u64, 1, json!({"meta": "tail"}));
    e.end(q, EndReason::Complete, body.len() as u64, json!({"wire": body.len()}));
    e.gap(0, GapReason::MaskFailed, 0, 0, 1, json!({"meta": "event"}));
    e.end(c, EndReason::Complete, 0, json!({"up": 9, "down": 9, "plain_up": 9, "plain_down": 9}));
    let c = e.conn();
    let head = json!({"method": "GET", "path": "/v1/responses", "ver": "1.1",
        "headers": [["upgrade", "websocket"]], "masked": [], "coding": null});
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    e.end(q, EndReason::Complete, 0, Value::Null);
    let up = e.open(StreamKind::WsUp, c, q, json!({"deflate": false}));
    let down = e.open(StreamKind::WsDown, c, q, json!({"deflate": false}));
    e.data(up, 0, 0x10 | FLAG_FIN, b"");
    e.data(down, 0, 0x10 | FLAG_FIN, b"");
    e.end(up, EndReason::Complete, 0, json!({"close_code": 1000}));
    e.end(down, EndReason::Complete, 0, json!({"close_code": 1000}));
    e.end(c, EndReason::Complete, 0, json!({"up": 9, "down": 9, "plain_up": 9, "plain_down": 9}));
    e.count();
    e
}

#[test]
fn mask_failed_meta_gaps_and_empty_messages_pass_and_recount() {
    let (code, v) = check(&entry().seal(), &[]);
    no_fail(&v);
    assert_ne!(code, 2, "{v}");
    for id in ["inv3", "inv4", "inv7", "gap_has_end", "counts_match_seal"] {
        assert_ne!(result(&v, id), "fail", "{id}");
    }
    assert_eq!(result(&v, "inv4"), "pass");
    assert_eq!(result(&v, "gap_has_end"), "pass");
    assert_ne!(result(&v, "recorder_alive"), "fail");
    let c = &v["entries"][0]["counts"];
    assert_eq!(c["gaps"]["mask_failed"], 3);
    assert_eq!(c["lost_bytes"], 0);
    assert_eq!(c["lost_unknown"], 0);
    assert_eq!(c["off_ms"], 0);
    // The POST is holed by its GAP, the empty client message is one whole request.
    assert_eq!(c["model_requests"], 2);
    assert_eq!(c["req_holes"], 1);
    assert_eq!(c["req_whole"], 1);
}

#[test]
fn extent_less_gaps_without_the_meta_shape_still_fail() {
    for (reason, lost, tail) in [
        (GapReason::MaskFailed, 1, Value::Null),
        (GapReason::MaskFailed, 2, json!({"meta": "tail"})),
        (GapReason::Pinned, 1, Value::Null),
    ] {
        let mut e = Entry::new("seam-shapes-bad");
        e.standard();
        e.gap(0, reason, 0, 0, lost, tail);
        let (_, v) = check(&e.seal(), &[]);
        assert_eq!(result(&v, "gap_has_end"), "fail", "{reason:?} {lost}");
    }
}
