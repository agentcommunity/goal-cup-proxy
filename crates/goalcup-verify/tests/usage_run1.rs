//! PRD-09's usage test (ledger B10): a run 1 shaped entry, scaled down (PRD-09 section 8), read
//! through seam U1 only. These are the wire cases; the `native_only` silent window, the panic
//! that prints no record text (PRD-17 VR2) and the output files come with the native reader and
//! `usage::run`. Every value is an obvious fake.

mod common;

use common::{Entry, read_records};
use goalcup_format::{EndReason, FLAG_FIN, Record, StreamKind};
use goalcup_verify::usage::{CallRow, CallTable, Calls, Kind, Outcome, Tokens};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Planted in prompts, outputs and error messages; no row may hold it (point 4).
const MARK: &str = "zebra-lantern-planted-text";
const RL: &str = "rate_limit_exceeded";
/// The requested model of every call planted: 8 on the WebSocket, 4 over HTTP.
const CALLS: &str = "fake-ws-a fake-ws-b fake-ws-d fake-ws-r1 fake-ws-r2 fake-ws-r3 fake-ws-r4 \
    fake-ws-e fake-http-a fake-http-429 fake-http-cut fake-claude";

type Five = [Option<u64>; 5];

fn five(t: &Tokens) -> Five {
    [t.input, t.cached, t.cache_write, t.output, t.reasoning]
}

/// A Responses usage object and the five numbers the reader must give for it.
fn usage(i: u64, c: u64, o: u64, r: u64) -> (Value, Five) {
    let v = json!({"input_tokens": i, "input_tokens_details": {"cached_tokens": c},
        "output_tokens": o, "output_tokens_details": {"reasoning_tokens": r},
        "total_tokens": i + o});
    (v, [Some(i - c), Some(c), None, Some(o), Some(r)])
}

/// A request for model `fake-<k>`, following reply `r_<prev>`.
fn create(k: &str, prev: Option<&str>) -> Value {
    json!({"type": "response.create", "model": format!("fake-{k}"), "reasoning": {"effort": "low"},
        "previous_response_id": prev.map(|p| format!("r_{p}")), "store": false, "stream": true,
        "input": [{"type": "message", "role": "user",
                   "content": [{"type": "input_text", "text": MARK}]}]})
}

/// Event `response.<ev>` of reply `r_<k>`; `x` is its usage, or its error when it failed.
fn ev(what: &str, k: &str, x: &Value) -> Value {
    let key = if what == "failed" { "error" } else { "usage" };
    json!({"type": format!("response.{what}"), "response": {"id": format!("r_{k}"),
        "model": format!("fake-{k}-dated"), "status": what, key: x}})
}

fn created(k: &str) -> Value {
    ev("created", k, &Value::Null)
}

fn delta() -> Value {
    json!({"type": "response.output_text.delta", "delta": MARK})
}

/// SSE text of these events, each named by its `type`.
fn sse(evs: &[Value]) -> String {
    let ev = |v: &Value| format!("event: {}\ndata: {v}\n\n", v["type"].as_str().unwrap());
    evs.iter().map(ev).collect()
}

/// One POST on its own connection, from `[path, request, status, reply, ended complete]`; the
/// reply body (SSE text, else JSON) comes in two pieces.
fn post(e: &mut Entry, s: &Value) {
    let (c, req) = (e.conn(), s[1].to_string());
    let head = json!({"method": "POST", "path": s[0], "ver": "1.1", "masked": [], "coding": null,
        "headers": [["content-type", "application/json"]]});
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    e.data(q, 0, 0, req.as_bytes());
    e.end(q, EndReason::Complete, req.len() as u64, Value::Null);
    let (body, ctype) = match s[3].as_str() {
        Some(t) => (t.to_owned(), "text/event-stream"),
        None => (s[3].to_string(), "application/json"),
    };
    let head = json!({"status": s[2], "ver": "1.1", "headers": [["content-type", ctype]],
        "masked": [], "coding": null, "decoded": true});
    let p = e.open(StreamKind::HttpResp, c, q, head);
    let (x, y) = body.split_at(body.len() / 2);
    e.data(p, 0, 0, x.as_bytes());
    e.data(p, x.len() as u64, 0, y.as_bytes());
    let end = match s[4].as_bool() {
        Some(true) => EndReason::Complete,
        _ => EndReason::Interrupted,
    };
    e.end(p, end, body.len() as u64, Value::Null);
    e.end(c, EndReason::Complete, 0, json!({"up": 900, "down": 900}));
}

/// The entry's records and the five numbers planted per call with usage, by requested model.
fn run1(name: &str) -> (Vec<Record>, BTreeMap<&'static str, Five>) {
    let ((ua, ta), (ub, tb)) = (usage(1000, 600, 200, 50), usage(1300, 1100, 90, 0));
    let ((ue, te), (uh, th)) = (usage(2000, 1500, 300, 120), usage(700, 100, 90, 30));
    let (uw, rl) = (usage(500, 0, 0, 0).0, json!({"code": RL, "message": MARK}));
    let err = json!({"type": "error", "status": 429, "error": rl});
    let mut warm = create("warm", None);
    warm["generate"] = false.into();
    // Each step: a client message, then the server messages that answer it.
    let ws = json!([
        [warm, [created("warm"), ev("completed", "warm", &uw)]],
        [create("ws-a", None), [created("ws-a"), delta(), ev("completed", "ws-a", &ua)]],
        [create("ws-b", Some("ws-a")), [created("ws-b"), ev("completed", "ws-b", &ub)]],
        [warm, [created("warm"), ev("completed", "warm", &uw)]],
        [create("ws-d", None), [created("ws-d"), delta()]],
        [{"type": "response.interrupt"}, []],
        [create("ws-r1", None), [created("ws-r1"), err, ev("failed", "ws-r1", &rl)]],
        [create("ws-r2", None), [created("ws-r2"), ev("failed", "ws-r2", &rl), err]],
        [create("ws-r3", None), [created("ws-r3"), ev("failed", "ws-r3", &rl)]],
        // Refused before `response.created`, after the previous call failed.
        [create("ws-r4", None), [err]],
        [create("ws-e", Some("ws-b")), [created("ws-e"), ev("completed", "ws-e", &ue)]]
    ]);
    // Anthropic usage is cumulative: the largest value wins, never a sum.
    let au = |o: u64| {
        json!({"input_tokens": 300, "cache_read_input_tokens": 40,
        "cache_creation_input_tokens": 20, "output_tokens": o})
    };
    let claude = sse(&[
        json!({"type": "message_start",
            "message": {"id": "msg_fake_1", "model": "fake-claude-dated", "usage": au(1)}}),
        json!({"type": "ping"}),
        json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": MARK}}),
        json!({"type": "message_delta", "delta": {"stop_reason": null}, "usage": au(30)}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": au(75)}),
        json!({"type": "message_stop"}),
    ]);
    let tc = [Some(300), Some(40), Some(20), Some(75), None];
    let http = |k: &str| json!({"model": format!("fake-{k}"), "stream": true, "input": MARK});
    let ha = sse(&[created("http-a"), delta(), ev("completed", "http-a", &uh)]);
    let creq = json!({"model": "fake-claude", "messages": [{"role": "user", "content": MARK}]});
    let posts = json!([
        ["/v1/responses", http("http-a"), 200, ha, true],
        ["/v1/responses", http("http-429"), 429, {"error": rl}, true],
        ["/v1/responses", http("http-cut"), 200, sse(&[created("http-cut"), delta()]), false],
        ["/v1/messages?beta=true", creq, 200, claude, true],
        ["/v1/fake-events", {"event": MARK}, 200, {"ok": true}, true]
    ]);

    let mut e = Entry::new(name);
    let c = e.conn();
    let head = json!({"method": "GET", "path": "/backend-api/codex/responses", "ver": "1.1",
        "headers": [["upgrade", "websocket"]], "masked": [], "coding": null});
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    e.end(q, EndReason::Complete, 0, Value::Null);
    let up = e.open(StreamKind::WsUp, c, q, json!({"deflate": false}));
    let down = e.open(StreamKind::WsDown, c, q, json!({"deflate": false}));
    let (mut u, mut d) = (0, 0);
    for step in ws.as_array().unwrap() {
        let m = step[0].to_string();
        let (x, y) = m.split_at(m.len() / 2);
        e.data(up, u, 0x10, x.as_bytes());
        e.data(up, u + x.len() as u64, 0x10 | FLAG_FIN, y.as_bytes());
        u += m.len() as u64;
        for v in step[1].as_array().unwrap().iter().map(Value::to_string) {
            e.data(down, d, 0x10 | FLAG_FIN, v.as_bytes());
            d += v.len() as u64;
        }
    }
    e.end(up, EndReason::Complete, u, json!({"close_code": 1000}));
    e.end(down, EndReason::Complete, d, json!({"close_code": 1000}));
    e.end(c, EndReason::Complete, 0, json!({"up": 900, "down": 900}));
    for s in posts.as_array().unwrap() {
        post(&mut e, s);
    }
    let planted = [("fake-ws-a", ta), ("fake-ws-b", tb), ("fake-ws-e", te)];
    let planted = planted
        .into_iter()
        .chain([("fake-http-a", th), ("fake-claude", tc)]);
    (read_records(&e.seal().join("rec")), planted.collect())
}

fn table(recs: &[Record]) -> CallTable {
    let mut calls = Calls::new();
    recs.iter().for_each(|r| calls.push(r));
    calls.finish()
}

/// The one row whose request named model `m`.
fn row<'a>(t: &'a CallTable, m: &str) -> &'a CallRow {
    let named = |r: &&CallRow| r.model_requested.as_deref() == Some(m);
    let rows: Vec<_> = t.rows.iter().filter(named).collect();
    assert_eq!(rows.len(), 1, "{m}: {rows:?}");
    rows[0]
}

/// The status and code of a rate-limited row.
fn limited(r: &CallRow) -> Option<(Option<u16>, Option<&str>)> {
    match &r.outcome {
        Outcome::RateLimited { status, code } => Some((*status, code.as_deref())),
        _ => None,
    }
}

#[test]
fn warmups_and_interrupts_are_not_calls() {
    let t = table(&run1("u-kinds").0);
    let n = |k| t.rows.iter().filter(|r| r.kind == k).count();
    let kinds = [Kind::Warmup, Kind::Interrupt, Kind::Other, Kind::Call].map(n);
    assert_eq!(kinds, [2, 1, 1, CALLS.split_whitespace().count()]);
    for m in CALLS.split_whitespace() {
        assert_eq!(row(&t, m).kind, Kind::Call, "{m}");
    }
}

#[test]
fn each_rate_limited_call_is_one_row() {
    let (recs, planted) = run1("u-rl");
    let t = table(&recs);
    for m in ["fake-ws-r1", "fake-ws-r2", "fake-ws-r3", "fake-ws-r4"] {
        assert_eq!(limited(row(&t, m)).and_then(|l| l.1), Some(RL), "{m}");
    }
    let http = limited(row(&t, "fake-http-429"));
    assert_eq!(http.and_then(|l| l.0), Some(429));
    assert_eq!(t.rows.iter().filter(|r| r.outcome.is_error()).count(), 5);
    let after = row(&t, "fake-ws-e");
    assert_eq!(after.outcome, Outcome::Ok);
    assert_eq!(five(&after.tokens), planted["fake-ws-e"]);
}

/// Run 1 had 8 cut WebSocket client messages: a create the recorder wrote in part before it
/// died is still a call, not whole (PRD-01 section 7: cut by the `recorder_down` gap).
#[test]
fn a_create_cut_by_a_restart_is_a_call_not_whole() {
    let mut e = Entry::new("u-restart");
    let c = e.conn();
    let head = json!({"method": "GET", "path": "/v1/responses", "ver": "1.1",
        "headers": [["upgrade", "websocket"]], "masked": [], "coding": null});
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    e.end(q, EndReason::Complete, 0, Value::Null);
    let up = e.open(StreamKind::WsUp, c, q, json!({"deflate": false}));
    let down = e.open(StreamKind::WsDown, c, q, json!({"deflate": false}));
    let m = format!(r#"{{"type":"response.create","model":"fake-ws-x","input":"{MARK}"}}"#);
    e.data(up, 0, 0x10 | FLAG_FIN, m.as_bytes());
    let mut d = 0;
    let done = ev("completed", "ws-x", &usage(9, 0, 1, 0).0);
    for v in [created("ws-x"), done].map(|v| v.to_string()) {
        e.data(down, d, 0x10 | FLAG_FIN, v.as_bytes());
        d += v.len() as u64;
    }
    e.data(up, m.len() as u64, 0x10, &m.as_bytes()[..40]);
    e.crash("killed", 1_000);
    let t = table(&read_records(&e.seal().join("rec")));
    let calls: Vec<_> = t.rows.iter().filter(|r| r.kind == Kind::Call).collect();
    let got: Vec<_> = calls.iter().map(|r| (r.boot, r.whole, &r.outcome)).collect();
    assert_eq!(got, [(0, true, &Outcome::Ok), (0, false, &Outcome::Unknown)]);
}

#[test]
fn totals_equal_the_planted_sum_and_repeat() {
    let (recs, planted) = run1("u-sum");
    let t = table(&recs);
    let sum = |fives: Vec<Five>| -> [u64; 5] {
        std::array::from_fn(|i| fives.iter().map(|f| f[i].unwrap_or(0)).sum())
    };
    let calls = t.rows.iter().filter(|r| r.kind == Kind::Call);
    let got = sum(calls.map(|r| five(&r.tokens)).collect());
    assert_eq!(got, sum(planted.into_values().collect()));
    assert_eq!(table(&recs), t);
}

#[test]
fn no_row_holds_text_from_a_body() {
    let t = table(&run1("u-mark").0);
    assert!(!t.rows.is_empty());
    assert!(!format!("{t:?}").contains(MARK), "a row holds body text");
}
