//! PRD-09's usage test (ledger B10): a run 1 shaped entry, scaled down (PRD-09 section 8), read
//! through seam U1 only. These are the wire cases; the `native_only` silent window, the panic
//! that prints no record text (PRD-17 VR2) and the output files come with the native reader and
//! `usage::run`. Every value is an obvious fake.

mod common;

use common::{Entry, HOST, read_records};
use goalcup_format::{EndReason, FLAG_FIN, Record, StreamKind};
use goalcup_verify::usage::{CallRow, CallTable, Calls, HostKind, Kind, Outcome, Shape};
use goalcup_verify::usage::{Tokens, Via};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Planted in prompts, outputs and error messages; no row may hold it (point 4).
const MARK: &str = "zebra-lantern-planted-text";
const RL: &str = "rate_limit_exceeded";
const TEXT: u8 = 0x10;

/// A Responses usage object and the five numbers the reader must give for it.
fn usage(input: u64, cached: u64, output: u64, reasoning: u64) -> (Value, Tokens) {
    let v = json!({"input_tokens": input, "input_tokens_details": {"cached_tokens": cached},
        "output_tokens": output, "output_tokens_details": {"reasoning_tokens": reasoning},
        "total_tokens": input + output});
    let t = Tokens {
        input: Some(input - cached),
        cached: Some(cached),
        output: Some(output),
        reasoning: Some(reasoning),
        ..Tokens::default()
    };
    (v, t)
}

/// A request for model `fake-<k>`; its reply is `r_<k>`, model `fake-<k>-dated`.
fn create(k: &str, prev: Option<&str>) -> Value {
    json!({"type": "response.create", "model": format!("fake-{k}"), "reasoning": {"effort": "low"},
        "previous_response_id": prev.map(|p| format!("r_{p}")), "store": false, "stream": true,
        "input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": MARK}]}]})
}

fn resp(ev: &str, k: &str, status: &str, more: Value) -> Value {
    let mut r = json!({"id": format!("r_{k}"), "model": format!("fake-{k}-dated"), "status": status});
    r.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
    json!({"type": ev, "response": r})
}

fn created(k: &str) -> Value {
    resp("response.created", k, "in_progress", json!({"usage": null}))
}

fn done(k: &str, u: &Value) -> Value {
    resp("response.completed", k, "completed", json!({"usage": u}))
}

fn failed(k: &str, rl: &Value) -> Value {
    resp("response.failed", k, "failed", json!({"error": rl, "usage": null}))
}

fn delta() -> Value {
    json!({"type": "response.output_text.delta", "delta": MARK})
}

/// SSE text of these events, each named by its `type`.
fn sse(evs: &[Value]) -> String {
    let ev = |v: &Value| format!("event: {}\ndata: {v}\n\n", v["type"].as_str().unwrap());
    evs.iter().map(ev).collect()
}

/// One POST on its own connection; the reply body (SSE text, else JSON) comes in two pieces.
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

/// The entry's records and the tokens planted per call, by requested model.
fn run1(name: &str) -> (Vec<Record>, BTreeMap<&'static str, Tokens>) {
    let ((ua, ta), (ub, tb), (ue, te)) = (usage(1000, 600, 200, 50), usage(1300, 1100, 90, 0), usage(2000, 1500, 300, 120));
    let ((uw, _), (uh, th)) = (usage(500, 0, 0, 0), usage(700, 100, 90, 30));
    let rl = json!({"code": RL, "message": MARK});
    let err = json!({"type": "error", "status": 429, "error": rl});
    let mut warm = create("warm", None);
    warm["generate"] = false.into();
    // Each step: a client message, then the server messages that answer it.
    let ws = json!([
        [warm, [created("warm"), done("warm", &uw)]],
        [create("ws-a", None), [created("ws-a"), delta(), done("ws-a", &ua)]],
        [create("ws-b", Some("ws-a")), [created("ws-b"), done("ws-b", &ub)]],
        [warm, [created("warm"), done("warm", &uw)]],
        [create("ws-d", None), [created("ws-d"), delta()]],
        [{"type": "response.interrupt"}, []],
        [create("ws-r1", None), [created("ws-r1"), err, failed("ws-r1", &rl)]],
        [create("ws-r2", None), [created("ws-r2"), failed("ws-r2", &rl), err]],
        [create("ws-e", Some("ws-b")), [created("ws-e"), done("ws-e", &ue)]]
    ]);
    let au = |out: u64| json!({"input_tokens": 300, "cache_read_input_tokens": 40,
        "cache_creation_input_tokens": 20, "output_tokens": out});
    let claude = sse(&[
        json!({"type": "message_start", "message": {"id": "msg_fake_1", "model": "fake-claude-dated", "usage": au(1)}}),
        json!({"type": "ping"}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": MARK}}),
        json!({"type": "message_delta", "delta": {"stop_reason": null}, "usage": au(30)}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": au(75)}),
        json!({"type": "message_stop"}),
    ]);
    let tc = Tokens { input: Some(300), cached: Some(40), cache_write: Some(20), output: Some(75), ..Tokens::default() };
    let http = |k: &str| json!({"model": format!("fake-{k}"), "stream": true, "input": MARK});
    // Each POST: path, request body, status, reply body, whether the reply ended `complete`.
    let posts = json!([
        ["/v1/responses", http("http-a"), 200, sse(&[created("http-a"), delta(), done("http-a", &uh)]), true],
        ["/v1/responses", http("http-429"), 429, {"error": rl}, true],
        ["/v1/responses", http("http-cut"), 200, sse(&[created("http-cut"), delta()]), false],
        ["/v1/messages?beta=true", {"model": "fake-claude", "stream": true, "messages": [{"role": "user", "content": MARK}]}, 200, claude, true],
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
        e.data(up, u, TEXT, x.as_bytes());
        e.data(up, u + x.len() as u64, TEXT | FLAG_FIN, y.as_bytes());
        u += m.len() as u64;
        for v in step[1].as_array().unwrap() {
            e.data(down, d, TEXT | FLAG_FIN, v.to_string().as_bytes());
            d += v.to_string().len() as u64;
        }
    }
    e.end(up, EndReason::Complete, u, json!({"close_code": 1000}));
    e.end(down, EndReason::Complete, d, json!({"close_code": 1000}));
    e.end(c, EndReason::Complete, 0, json!({"up": 900, "down": 900}));
    posts.as_array().unwrap().iter().for_each(|s| post(&mut e, s));
    let planted = [("fake-ws-a", ta), ("fake-ws-b", tb), ("fake-ws-e", te), ("fake-http-a", th), ("fake-claude", tc)];
    (read_records(&e.seal().join("rec")), planted.into())
}

fn table(recs: &[Record]) -> CallTable {
    let mut calls = Calls::new();
    recs.iter().for_each(|r| calls.push(r));
    calls.finish()
}

/// The one row whose request named model `m`.
fn row<'a>(t: &'a CallTable, m: &str) -> &'a CallRow {
    let rows: Vec<_> = t.rows.iter().filter(|r| r.model_requested.as_deref() == Some(m)).collect();
    assert_eq!(rows.len(), 1, "{m}: {rows:?}");
    rows[0]
}

fn kinds(t: &CallTable, k: Kind) -> usize {
    t.rows.iter().filter(|r| r.kind == k).count()
}

#[test]
fn warmups_and_interrupts_are_rows_of_their_own_kind() {
    let t = table(&run1("u-kinds").0);
    assert_eq!(kinds(&t, Kind::Warmup), 2);
    assert_eq!(kinds(&t, Kind::Interrupt), 1);
    assert_eq!(kinds(&t, Kind::Other), 1);
    assert_eq!(kinds(&t, Kind::Call), 10, "6 WebSocket and 4 HTTP calls");
    let warm = |r: &&CallRow| r.model_requested.as_deref() == Some("fake-warm");
    assert!(t.rows.iter().filter(warm).all(|r| r.kind == Kind::Warmup));
    assert_eq!(row(&t, "fake-ws-d").outcome, Outcome::Cancelled);
}

#[test]
fn each_rate_limited_call_is_one_row() {
    let (recs, planted) = run1("u-rl");
    let t = table(&recs);
    for m in ["fake-ws-r1", "fake-ws-r2"] {
        let r = row(&t, m);
        let rl = matches!(&r.outcome, Outcome::RateLimited { code: Some(c), .. } if c == RL);
        assert!(r.kind == Kind::Call && rl, "{m}: {r:?}");
    }
    let r = row(&t, "fake-http-429");
    assert!(matches!(r.outcome, Outcome::RateLimited { status: Some(429), .. }), "{r:?}");
    assert_eq!(t.rows.iter().filter(|r| r.outcome.is_error()).count(), 3);
    let after = row(&t, "fake-ws-e");
    assert_eq!((&after.outcome, after.tokens), (&Outcome::Ok, planted["fake-ws-e"]));
}

#[test]
fn tokens_and_outcomes_follow_the_shape() {
    let (recs, planted) = run1("u-shape");
    let t = table(&recs);
    let a = row(&t, "fake-ws-a");
    assert_eq!((a.kind, &a.outcome, a.whole), (Kind::Call, &Outcome::Ok, true));
    assert_eq!(a.tokens, planted["fake-ws-a"], "input less cached, output with reasoning");
    assert_eq!((a.model.as_deref(), a.effort.as_deref()), (Some("fake-ws-a-dated"), Some("low")));
    assert_eq!((a.via, a.shape), (Some(Via::Ws), Some(Shape::Responses)));
    assert_eq!((a.host.as_deref(), a.host_kind), (Some(HOST), Some(HostKind::Provider)));
    let h = row(&t, "fake-http-a");
    assert_eq!((&h.outcome, h.via, h.tokens), (&Outcome::Ok, Some(Via::Http), planted["fake-http-a"]));
    let c = row(&t, "fake-claude");
    assert_eq!((c.kind, &c.outcome, c.shape), (Kind::Call, &Outcome::Ok, Some(Shape::Anthropic)));
    assert_eq!(c.tokens, planted["fake-claude"], "largest cumulative value, never a sum");
    let cut = row(&t, "fake-http-cut");
    assert_eq!((&cut.outcome, cut.whole), (&Outcome::Unknown, false));
}

#[test]
fn totals_equal_the_planted_sum_and_repeat() {
    let (recs, planted) = run1("u-sum");
    let t = table(&recs);
    let sum = |ts: Vec<&Tokens>| {
        let f = |x: &&Tokens| [x.input, x.cached, x.cache_write, x.output, x.reasoning];
        let add = |s: [u64; 5], v: [Option<u64>; 5]| -> [u64; 5] {
            std::array::from_fn(|i| s[i] + v[i].unwrap_or(0))
        };
        ts.iter().map(f).fold([0u64; 5], add)
    };
    let calls = t.rows.iter().filter(|r| r.kind == Kind::Call).map(|r| &r.tokens);
    assert_eq!(sum(calls.collect()), sum(planted.values().collect()));
    assert_eq!(table(&recs), t);
}

#[test]
fn no_row_holds_text_from_a_body() {
    let t = table(&run1("u-mark").0);
    assert!(!t.rows.is_empty());
    assert!(!format!("{t:?}").contains(MARK), "a row holds body text");
}
