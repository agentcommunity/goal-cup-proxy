//! `goalcup-verify check` on losses that are named, not silent (PRD-08 section 3): a recorder
//! that died, ran in null mode or lost its watchdog leaves one `recorder_down` gap, and the
//! entry exits 1 with no `fail`; so does an `after_stop` GAP and an unsealed entry. With
//! `--live`, a stale COUNT is named in null mode and a failure otherwise. PRD-17 VR4: an
//! `interrupted` END is no loss. Counts follow the recorder's rules where a loss is named.

mod common;

use common::*;
use goalcup_format::codec::Record;
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

/// A listed tunnel of 512 up and 2048 down bytes, then its GAP (`lost` = up + down + 1) if any.
fn tunnel(e: &mut Entry, host: &str, reason: &str, gap: Option<(GapReason, Value)>) {
    let meta = json!({"host": host, "port": 443, "mode": "tunnel", "listed": "shared",
        "kind": "provider", "reason": reason, "alpn": null, "tls": null,
        "fingerprint": "0123456789abcdef"});
    let s = e.open(StreamKind::Conn, 0, 0, meta);
    if let Some((cause, tail)) = gap {
        e.gap(s, cause, 1_000, 0, 2_561, tail);
    }
    e.end(s, EndReason::Complete, 0, json!({"up": 512, "down": 2048}));
}

fn sealed(mut e: Entry) -> (i32, Value) {
    e.count();
    check(&e.seal(), &[])
}

#[test]
fn listed_tunnels_with_a_named_reason_are_named_and_without_a_gap_fail() {
    use GapReason::*;
    let guard = json!({"guard": "client_hello", "limit": 65536});
    for (reason, cause, tail) in [
        ("pinned", Pinned, Value::Null),
        ("tls_refused", TlsRefused, Value::Null),
        ("upstream_untrusted", UpstreamUntrusted, Value::Null),
        ("alpn_other", ProtocolOther, Value::Null),
        ("hello_too_big", MemoryGuard, guard),
    ] {
        let mut e = Entry::new("listed-tunnel");
        tunnel(&mut e, "t.example.test", reason, Some((cause, tail)));
        let (code, v) = sealed(e);
        assert_eq!(result(&v, "listed_hosts_decrypted"), "named", "{reason}");
        assert_eq!(code, 1, "{reason}");
    }
    for (reason, res) in [("pinned", "fail"), ("sign_in", "pass")] {
        let mut e = Entry::new("listed-tunnel-no-gap");
        tunnel(&mut e, "t.example.test", reason, None);
        let (code, v) = sealed(e);
        assert_eq!(result(&v, "listed_hosts_decrypted"), res, "{reason}");
        assert_eq!(code == 2, res == "fail", "{reason}");
    }
}

#[test]
fn decrypted_conn_with_a_parser_fault_gap_is_named() {
    let mut e = Entry::new("conn-gap");
    let c = e.conn();
    e.gap(c, GapReason::DecodeFailed, 10, 0, 1_000_001, Value::Null);
    let tail = json!({"up": 1_000_064, "down": 64, "plain_up": 1_000_000, "plain_down": 0});
    e.end(c, EndReason::Complete, 0, tail);
    let (code, v) = sealed(e);
    assert_eq!(result(&v, "listed_hosts_decrypted"), "named");
    assert_eq!(code, 1);
}

/// A tunnel needs a GAP of its own reason, and a host the `start` list holds is listed whatever
/// the conn says: a mislabelled blind relay is a `fail`.
#[test]
fn a_tunnel_with_the_wrong_gap_or_a_mislabelled_listed_host_fails() {
    let mut e = Entry::new("wrong-gap");
    let gap = Some((GapReason::MemoryGuard, json!({"guard": "held_bytes"})));
    tunnel(&mut e, "t.example.test", "pinned", gap);
    let (code, v) = sealed(e);
    assert_eq!(result(&v, "listed_hosts_decrypted"), "fail");
    assert_eq!(code, 2);
    let mut e = Entry::new("mislabelled");
    let meta = json!({"host": HOST, "port": 443, "mode": "tunnel", "listed": null,
        "kind": null, "reason": "not_listed", "alpn": null, "tls": null, "fingerprint": null});
    let s = e.open(StreamKind::Conn, 0, 0, meta);
    e.end(s, EndReason::Complete, 0, json!({"up": 512, "down": 2048}));
    let (code, v) = sealed(e);
    assert_eq!(result(&v, "listed_hosts_decrypted"), "fail");
    assert_eq!(code, 2);
}

/// A refusal of a host and fingerprint that was trusted earlier is a hint, unless no alert came.
#[test]
fn refusal_after_trust_lists_a_learned_refusal_but_not_a_silent_one() {
    for (alert, listed) in [(true, 1), (false, 0)] {
        let mut e = Entry::new("refusal");
        e.exchange(EndReason::Complete);
        let gap = Some((GapReason::TlsRefused, json!({"alert": alert})));
        tunnel(&mut e, HOST, "tls_refused", gap);
        let (code, v) = sealed(e);
        let facts = &item(&v, "listed_hosts_decrypted")["facts"]["refusal_after_trust"];
        let n = facts.as_array().map_or(0, Vec::len);
        assert_eq!(n, listed, "alert {alert}");
        assert_eq!(result(&v, "listed_hosts_decrypted"), "named");
        assert_eq!(code, 1);
    }
}

/// One request stream of `n` bytes, all `*` and all under one `fail_closed` MASK (rule 2).
fn fail_closed_span(n: u64) -> (i32, Value) {
    let mut e = Entry::new("fail-closed");
    let c = e.conn();
    let head = json!({"method": "GET", "path": "/v1/files", "ver": "1.1", "headers": [],
        "masked": [], "coding": null});
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    for off in (0..n).step_by(65_536) {
        e.data(q, off, 0, &vec![b'*'; (n - off).min(65_536) as usize]);
    }
    e.put(Record::Mask {
        sid: q,
        off: 0,
        len: n,
        rule: 2,
    });
    e.end(q, EndReason::Complete, n, json!({"wire": n}));
    let tail = json!({"up": n, "down": 0, "plain_up": n, "plain_down": 0});
    e.end(c, EndReason::Complete, 0, tail);
    sealed(e)
}

#[test]
fn fail_closed_masking_over_1_mib_in_one_stream_warns() {
    let (code, v) = fail_closed_span(1_048_577);
    assert_eq!(result(&v, "mask_failclosed"), "warn");
    assert_eq!(item(&v, "mask_failclosed")["facts"]["streams"], 1);
    assert_eq!(code, 0);
}
