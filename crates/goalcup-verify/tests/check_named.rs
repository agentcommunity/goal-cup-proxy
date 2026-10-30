//! `goalcup-verify check` on losses that are named, not silent (PRD-08 section 3): a recorder
//! that died, ran in null mode or lost its watchdog leaves one `recorder_down` gap, and the
//! entry exits 1 with no `fail`; so does an `after_stop` GAP and an unsealed entry. With
//! `--live`, a stale COUNT is named in null mode and a failure otherwise. PRD-17 VR4: an
//! `interrupted` END is no loss. Counts follow the recorder's rules where a loss is named.

mod common;

use common::*;
use goalcup_format::{EndReason, FLAG_FIN, GapReason, StreamKind};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn no_fail(v: &Value) {
    assert!(
        ids_with(v, "fail").is_empty(),
        "failed: {:?}",
        ids_with(v, "fail")
    );
}

/// Checks a recovered entry: exit 1, `inv6` named with one gap of this cause and length.
fn assert_one_down_gap(dir: &Path, cause: &str, ms: u64) {
    let (code, v) = check(dir, &[]);
    no_fail(&v);
    assert_eq!(code, 1);
    assert_eq!(result(&v, "inv6"), "named");
    let gaps = item(&v, "inv6")["facts"]["gaps"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(gaps.len(), 1, "{gaps:?}");
    assert_eq!(gaps[0]["cause"], cause);
    assert_eq!(gaps[0]["ms"], ms);
    assert_eq!(v["entries"][0]["boots"], 2);
}

/// PRD-17 VR4.
#[test]
fn clean_record_with_interrupted_end_exits_0() {
    let mut e = Entry::new("vr4");
    e.standard();
    e.exchange(EndReason::Interrupted);
    e.count();
    let (code, v) = check(&e.seal(), &[]);
    no_fail(&v);
    assert_eq!(code, 0);
    assert_eq!(v["result"], "pass");
}

#[test]
fn killed_boot_recovered_into_a_second_boot_is_named() {
    let mut e = Entry::new("killed");
    e.standard();
    let ms = e.crash("killed", 5_000);
    e.exchange(EndReason::Complete);
    e.count();
    assert_one_down_gap(&e.seal(), "killed", ms);
}

/// PRD-01 section 12 step 3: the boot after a recovery dies before its first LINK, so the part
/// file closed by `R` takes a second frame with the recovered seal. Two named gaps, no fail.
#[test]
fn recovered_seal_in_a_second_recovery_frame_is_named() {
    let mut e = Entry::new("second-frame");
    e.standard();
    let first = e.crash("killed", 5_000);
    let dir = e.seal_recovered("killed", 60_000);
    let (code, v) = check(&dir, &[]);
    no_fail(&v);
    assert_eq!(code, 1);
    assert_eq!(result(&v, "inv6"), "named");
    assert_eq!(v["entries"][0]["seal_reason"], "recovered");
    assert_eq!(v["entries"][0]["boots"], 1);
    let gaps = item(&v, "inv6")["facts"]["gaps"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(gaps.len(), 2, "{gaps:?}");
    assert_eq!(gaps[0]["ms"], first);
    assert!(gaps[1]["ms"].as_u64().unwrap() >= 60_000);
}

/// PRD-01 section 12: both processes die before boot 0's first LINK, so recovery starts boot 0
/// afresh and the recovered seal follows `start` and the one `recorder_down` gap.
#[test]
fn recovered_seal_of_an_entry_with_no_index_line_is_named() {
    let dir = Entry::recovered_before_first_link("no-index", "killed", 30_000);
    let (code, v) = check(&dir, &[]);
    no_fail(&v);
    assert_eq!(code, 1);
    assert_eq!(result(&v, "inv6"), "named");
    assert_eq!(v["entries"][0]["seal_reason"], "recovered");
    let gaps = item(&v, "inv6")["facts"]["gaps"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(gaps.len(), 1, "{gaps:?}");
    assert_eq!(gaps[0]["cause"], "killed");
    assert!(gaps[0]["ms"].as_u64().unwrap() >= 30_000);
}

#[test]
fn null_mode_10_minutes_and_watchdog_lost_2_hours_are_named() {
    for (cause, down) in [("null_mode", 10 * 60_000), ("watchdog_lost", 2 * 3_600_000)] {
        let mut e = Entry::new(cause);
        e.standard();
        let ms = e.crash(cause, down);
        assert!(ms >= down);
        e.count();
        assert_one_down_gap(&e.seal(), cause, ms);
    }
}

#[test]
fn after_stop_gap_is_named() {
    let mut e = Entry::new("after-stop");
    e.standard();
    e.after_stop();
    let (code, v) = check(&e.seal(), &[]);
    no_fail(&v);
    assert_eq!(result(&v, "gaps_named"), "named");
    assert_eq!(code, 1);
}

#[test]
fn unsealed_crashed_entry_is_named() {
    let mut e = Entry::new("unsealed");
    e.standard();
    let meta = json!({"path": "fake:sessions/s2.jsonl", "gen": 0, "size_at_open": 0, "from": 0, "fid": "3c4d"});
    let s = e.open(StreamKind::Native, 0, 0, meta);
    e.data(s, 0, 0, b"{\"line\":1}\n");
    let (code, v) = check(&e.leave(), &[]);
    no_fail(&v);
    assert_eq!(v["entries"][0]["sealed"], false);
    assert_eq!(result(&v, "streams_closed"), "named");
    assert_eq!(code, 1);
}

/// An unsealed entry whose last COUNT is an hour old by the wall clock.
fn stale_live_entry(name: &str) -> std::path::PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    let mut e = Entry::at(name, now - 3_600 * 1_000_000_000);
    e.standard();
    e.leave()
}

#[test]
fn live_stale_count_is_named_in_null_mode() {
    let d = stale_live_entry("live-null");
    fs::write(d.join("status.json"), br#"{"mode":"null"}"#).unwrap();
    let (code, v) = check(&d, &["--live"]);
    no_fail(&v);
    assert_eq!(result(&v, "recorder_alive"), "named");
    assert_eq!(code, 1);
}

#[test]
fn live_stale_count_fails_without_null_mode() {
    let d = stale_live_entry("live-dead");
    let (code, v) = check(&d, &["--live"]);
    assert_eq!(result(&v, "recorder_alive"), "fail");
    assert_eq!(code, 2);
}

/// Point 6: the counts the participant sees equal the recount where PRD-02's counter decides the
/// rule. A listed tunnel is `listed` only once its GAP is written (a COUNT while it is open says
/// null); snapshot bytes are native bytes; a WebSocket message after a GAP and a ping has holes.
#[test]
fn counts_equal_the_recorder_rules_for_tunnels_snapshots_and_ws_gaps() {
    let mut e = Entry::new("count-rules");
    e.standard();
    let meta = json!({"host": "pinned.example.test", "port": 443, "mode": "tunnel",
        "listed": "shared", "kind": "provider", "reason": "pinned", "alpn": null, "tls": null,
        "fingerprint": "0123456789abcdef"});
    let tunnel = e.open(StreamKind::Conn, 0, 0, meta);
    e.count();
    e.gap(tunnel, GapReason::Pinned, 1_000, 0, 0, Value::Null);
    e.end(
        tunnel,
        EndReason::Complete,
        0,
        json!({"up": 512, "down": 2048}),
    );
    let meta = json!({"path": "fake:state.db", "gen": 0, "from": 0});
    let snap = e.open(StreamKind::Snapshot, 0, 0, meta);
    e.data(snap, 0, 0, b"fake database bytes");
    e.end(snap, EndReason::Complete, 19, Value::Null);
    let c = e.conn();
    let head = json!({"method": "GET", "path": "/v1/responses", "ver": "1.1",
        "headers": [["upgrade", "websocket"]], "masked": [], "coding": null});
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    e.end(q, EndReason::Complete, 0, Value::Null);
    let up = e.open(StreamKind::WsUp, c, q, json!({"deflate": false}));
    e.gap(
        up,
        GapReason::MemoryGuard,
        10,
        0,
        9,
        json!({"guard": "held_bytes"}),
    );
    e.data(up, 8, 0x90 | FLAG_FIN, b"p");
    e.data(up, 9, 0x10 | FLAG_FIN, br#"{"input":"hi"}"#);
    e.end(up, EndReason::Complete, 23, Value::Null);
    e.end(c, EndReason::Complete, 0, json!({"up": 64, "down": 32}));
    let (code, v) = check(&e.seal(), &[]);
    no_fail(&v);
    assert_eq!(result(&v, "inv7"), "pass");
    assert_eq!(result(&v, "counts_match_seal"), "pass");
    assert_eq!(v["entries"][0]["counts"]["req_holes"], 1);
    assert_eq!(code, 1);
}
