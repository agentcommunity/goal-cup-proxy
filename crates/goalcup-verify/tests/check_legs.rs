//! The optional log checks (PRD-08 section 3.1) over hand-built call tables. A skip is never a
//! pass; a loss inside a recorder GAP window makes a fail `named`.

mod common;

use common::{Entry as Rec, check, item, tmp};
use goalcup_format::{EndReason, GapReason, StreamKind};
use goalcup_verify::legs::{Cover, judge};
use goalcup_verify::usage::*;
use serde_json::{Value, json};
use std::{fs, path::Path};

const MIN: u64 = 60_000_000_000;
const T0: u64 = 1_700_000_000_000_000_000;
const K: [&str; 3] = ["wire_keeps_up", "threads_match", "subagents_present"];

fn call(leg: Leg, thread: &str, t: u64) -> CallRow {
    let mut r = CallRow::new(leg, "e", 1, Kind::Call);
    r.thread = (!thread.is_empty()).then_some(thread.into());
    (r.wall_end, r.outcome, r.shape) = (Some(t), Outcome::Ok, Some(Shape::Chat));
    r
}

/// A readable per-call table: `h` harness and `w` wire calls of a thread, a minute apart.
fn tab(id: &str, h: u64, w: u64) -> CallTable {
    let legs = [(Leg::Native, h), (Leg::Wire, w)];
    let each = legs.iter().flat_map(|&(l, n)| (0..n).map(move |i| (l, i)));
    let rows = each.map(|(l, i)| call(l, id, T0 + i * MIN)).collect();
    let mut t = CallTable::default();
    (t.rows, t.reason, t.native_form) = (rows, None, NativeForm::PerCall);
    t
}

fn thread(id: &str, calls: u64, total: Option<u64>) -> Thread {
    let mut t = Thread::new(id);
    (t.reason, t.native_calls, t.native_total) = (None, calls, total);
    t
}

/// Hermes-shaped: `n` wire calls naming no thread; the threads carry the harness totals.
fn totals(n: u64) -> CallTable {
    let mut t = tab("", 0, n);
    t.threads = vec![thread("s1", 0, Some(6)), thread("s2", 0, Some(4))];
    t.native_form = NativeForm::Totals;
    t
}

/// A parent `p` with the child `k`: `wire` wire calls of it, and its own log read or not.
fn parent(read: bool, wire: u64) -> CallTable {
    let mut t = tab("k", 0, wire);
    t.children = Some([("p".to_string(), vec!["k".to_string()])].into());
    t.threads = vec![thread("k", 0, None)]
        .into_iter()
        .filter(|_| read)
        .collect();
    t
}

/// One check's result; the cover is (from, to) in minutes after `T0`, from a recorder-down
/// GAP when `down` is set and from a conn GAP of a model host otherwise.
fn go_by(t: &CallTable, cover: Option<(u64, u64)>, down: bool, id: &str) -> &'static str {
    let at = |m: u64| T0 + m * MIN;
    let c = cover.map(|(a, b)| Cover {
        start_wall: at(a),
        end_wall: at(b),
        down,
    });
    let (checks, _) = judge(t, c.as_slice());
    checks.iter().find(|x| x.id == id).unwrap().result.name()
}

fn go(t: &CallTable, cover: Option<(u64, u64)>, id: &str) -> &'static str {
    go_by(t, cover, true, id)
}

#[test]
fn legs_grade_the_excess_and_a_gap_names_it() {
    let [k, m, s] = K;
    let cases = [
        (tab("a", 15, 5), None, k, "fail"),
        (tab("a", 15, 5), Some((5, 14)), k, "named"),
        (tab("a", 8, 5), None, k, "pass"),
        (tab("a", 3, 6), None, k, "pass"),
        (tab("b", 2, 0), None, m, "fail"),
        (tab("b", 2, 0), Some((0, 2)), m, "named"),
        (totals(10), None, m, "pass"),
        (totals(6), None, m, "fail"),
        (totals(6), Some((100, 101)), m, "named"),
        (CallTable::default(), None, s, "skip"),
        (parent(false, 0), None, s, "fail"),
        (parent(false, 0), Some((0, 1)), s, "named"),
        (parent(false, 2), None, s, "pass"),
        (parent(true, 0), None, s, "pass"),
        // Wire calls inside the cover answer covered harness calls only: the loss at minutes
        // 12 to 19 has no cover and no wire call.
        (tab("a", 20, 10), Some((0, 9)), k, "fail"),
        (tab("a", 20, 10), Some((0, 9)), m, "fail"),
        (tab("a", 0, 3), None, k, "skip"),
        (tab("a", 0, 3), None, m, "skip"),
        (totals(3), None, k, "skip"),
    ];
    for (t, cover, id, want) in cases {
        assert_eq!(go(&t, cover, id), want, "{id}");
    }
}

#[test]
fn a_conn_gap_does_not_name_a_subagent_with_neither_leg() {
    // The child has no wire call and no native stream: a loss on one connection cannot explain
    // it, only a recorder that was down can (point 2).
    let t = parent(false, 0);
    assert_eq!(go_by(&t, Some((0, 1)), false, K[2]), "fail");
    assert_eq!(go_by(&t, Some((0, 1)), true, K[2]), "named");
}

#[test]
fn no_reader_skips_and_a_wire_only_thread_is_reported() {
    let mut none = tab("a", 0, 3);
    none.reason = Some(Reason::NoReader);
    assert_eq!(judge(&none, &[]).1.harness_log, "none");
    assert_eq!(go(&none, None, K[0]), "skip");
    assert_eq!(go(&none, None, K[1]), "skip");
    let mut t = tab("a", 5, 5);
    t.rows.extend(tab("w", 0, 3).rows);
    assert_eq!(go(&t, None, K[1]), "pass");
    let (_, o) = judge(&t, &[]);
    assert_eq!(o.threads.len(), 2);
    let w = o.threads.iter().find(|r| r["thread"] == "w").unwrap();
    assert_eq!((&w["harness"], &w["wire_ok"]), (&json!(0), &json!(3)));
}

#[test]
fn provider_errors_count_by_code_and_leg() {
    let err = |leg, limited, code: &str| {
        let (status, code) = (None, Some(code.to_string()));
        let mut r = call(leg, "", T0);
        r.outcome = match limited {
            true => Outcome::RateLimited { status, code },
            false => Outcome::Error { status, code },
        };
        r
    };
    let mut t = tab("", 0, 1);
    t.rows.push(err(Leg::Wire, true, "rate_limit"));
    t.rows.push(err(Leg::Wire, false, "server_error"));
    t.rows.push(err(Leg::Native, false, "overloaded"));
    let e = judge(&t, &[]).1.provider_errors;
    assert_eq!(e["wire"], json!({"rate_limit": 1, "server_error": 1}));
    assert_eq!(e["native"], json!({"overloaded": 1}));
}

const S1: &str = "fake:sessions/s1.jsonl";

/// A sealed entry with `s1` (22 bytes) as a native stream and `state.db` as a snapshot stream;
/// after a recorder crash when `down`.
fn rec(name: &str, down: bool) -> std::path::PathBuf {
    let mut e = Rec::new(name);
    e.standard();
    let meta = json!({"path": "fake:state.db", "gen": 0, "from": 0});
    let s = e.open(StreamKind::Snapshot, 0, 0, meta);
    e.data(s, 0, 0, b"SQLite");
    e.end(s, EndReason::Complete, 6, Value::Null);
    if down {
        e.crash("killed", 5_000);
        e.count();
    }
    e.seal()
}

/// `subagents_present` for the files `list` (path, size) on the entry: exit code, result and
/// the first finding's file.
fn files(dir: &Path, list: &[(&str, u64)]) -> (i32, String, Value) {
    let f: Vec<Value> = list
        .iter()
        .map(|(p, n)| json!({"path": p, "size": n}))
        .collect();
    let m = tmp("manifest").join("m.json");
    fs::write(&m, Value::Array(f).to_string()).unwrap();
    let (code, v) = check(dir, &["--harness-files", m.to_str().unwrap()]);
    let c = item(&v, "subagents_present");
    (
        code,
        c["result"].as_str().unwrap().into(),
        c["first"]["file"].clone(),
    )
}

#[test]
fn a_harness_file_needs_a_stream_of_its_size_or_a_snapshot() {
    let d = rec("manifest-whole", false);
    let db = ("fake:state.db", 123_456);
    // A pass of the manifest part never raises a skip: no parentage was read here.
    assert_eq!(files(&d, &[(S1, 22), db]).1, "skip");
    // No stream: fail, and the first finding names the path. A wrong size is no stream of its size.
    let (code, res, first) = files(&d, &[(S1, 22), ("fake:sessions/s2.jsonl", 5)]);
    assert_eq!(
        (code, res.as_str(), first),
        (2, "fail", json!("fake:sessions/s2.jsonl"))
    );
    for bad in [&[(S1, 30)][..], &[("fake:other.db", 1)][..]] {
        assert_eq!(files(&d, bad).1, "fail");
    }
    // Without the flag the check is as before.
    let (_, v) = check(&d, &[]);
    assert_eq!(item(&v, "subagents_present")["result"], "skip");
}

#[test]
fn a_file_with_no_stream_is_named_when_the_recorder_was_down() {
    let d = rec("manifest-down", true);
    let (code, res, first) = files(&d, &[(S1, 22), ("fake:sessions/s2.jsonl", 5)]);
    assert_eq!((code, res.as_str()), (1, "named"));
    assert_eq!(first, json!("fake:sessions/s2.jsonl"));
}

#[test]
fn a_stream_with_a_gap_names_its_file() {
    let mut e = Rec::new("manifest-gap");
    let meta = json!({"path": S1, "gen": 0, "from": 0});
    let s = e.open(StreamKind::Native, 0, 0, meta);
    e.gap(s, GapReason::MemoryGuard, 10, 0, 6, Value::Null);
    e.data(s, 5, 0, b"abcd");
    e.end(s, EndReason::Complete, 9, Value::Null);
    e.count();
    assert_eq!(files(&e.seal(), &[(S1, 40)]).1, "named");
}

#[test]
fn a_manifest_of_another_shape_is_no_report() {
    let m = tmp("manifest-bad").join("m.json");
    fs::write(&m, r#"{"path": "x"}"#).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup-verify"))
        .args(["check", rec("manifest-shape", false).to_str().unwrap()])
        .args(["--harness-files", m.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
}
