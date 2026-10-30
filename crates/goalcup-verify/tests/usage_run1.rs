//! PRD-09's usage test (ledger B10): a run 1 shaped entry, scaled down (PRD-09 section 8), read
//! through seam U1 only. These are the wire cases; the `native_only` silent window, the panic
//! that prints no record text (PRD-17 VR2) and the output files come with the native reader and
//! `usage::run`. Every value is an obvious fake.

mod common;

use common::{Entry, HOST, read_records};
use goalcup_format::{EndReason, FLAG_FIN, Record, StreamKind};
use goalcup_verify::usage::{
    CallRow, CallTable, Calls, HostKind, Kind, Outcome, Shape, Tokens, Via,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Planted in prompts, outputs and error messages; no row may hold it (point 4).
const MARK: &str = "zebra-lantern-planted-text";
const TEXT: u8 = 0x10;
const RL: &str = "rate_limit_exceeded";

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

fn create(model: &str, prev: Option<&str>) -> Value {
    json!({"type": "response.create", "model": model, "previous_response_id": prev,
        "reasoning": {"effort": "low"}, "store": false, "stream": true,
        "input": [{"type": "message", "role": "user",
                   "content": [{"type": "input_text", "text": MARK}]}]})
}

fn resp(ev: &str, id: &str, model: &str, extra: Value) -> Value {
    let mut r = json!({"id": id, "model": model, "status": "in_progress", "usage": null});
    r.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    json!({"type": ev, "response": r})
}

fn delta() -> Value {
    json!({"type": "response.output_text.delta", "delta": MARK})
}

/// SSE text of these events, each named by its `type`.
fn sse(evs: &[Value]) -> String {
    evs.iter()
        .map(|v| format!("event: {}\ndata: {v}\n\n", v["type"].as_str().unwrap()))
        .collect()
}

/// One WebSocket: client messages arrive in two pieces, server messages in one.
struct Ws(u64, u64, u64, u64);

impl Ws {
    fn send(&mut self, e: &mut Entry, v: Value) {
        let b = v.to_string();
        let (x, y) = b.split_at(b.len() / 2);
        e.data(self.0, self.2, TEXT, x.as_bytes());
        e.data(self.0, self.2 + x.len() as u64, TEXT | FLAG_FIN, y.as_bytes());
        self.2 += b.len() as u64;
    }
    fn recv(&mut self, e: &mut Entry, evs: &[Value]) {
        for v in evs {
            let b = v.to_string();
            e.data(self.1, self.3, TEXT | FLAG_FIN, b.as_bytes());
            self.3 += b.len() as u64;
        }
    }
}

/// One POST on its own connection; the reply body comes in two pieces and ends `end`.
fn post(e: &mut Entry, path: &str, req: Value, status: u16, ctype: &str, body: &str, end: EndReason) {
    let c = e.conn();
    let head = json!({"method": "POST", "path": path, "ver": "1.1", "masked": [], "coding": null,
        "headers": [["content-type", "application/json"]]});
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    let b = req.to_string();
    e.data(q, 0, 0, b.as_bytes());
    e.end(q, EndReason::Complete, b.len() as u64, json!({"wire": b.len()}));
    let head = json!({"status": status, "ver": "1.1", "headers": [["content-type", ctype]],
        "masked": [], "coding": null, "decoded": true});
    let p = e.open(StreamKind::HttpResp, c, q, head);
    let (x, y) = body.split_at(body.len() / 2);
    e.data(p, 0, 0, x.as_bytes());
    e.data(p, x.len() as u64, 0, y.as_bytes());
    e.end(p, end, body.len() as u64, json!({"wire": body.len()}));
    e.end(c, EndReason::Complete, 0, json!({"up": 900, "down": 900}));
}

/// The entry's records and the tokens planted per call, by requested model.
fn run1(name: &str) -> (Vec<Record>, BTreeMap<&'static str, Tokens>) {
    let mut planted = BTreeMap::new();
    let mut e = Entry::new(name);
    let c = e.conn();
    let head = json!({"method": "GET", "path": "/backend-api/codex/responses", "ver": "1.1",
        "headers": [["upgrade", "websocket"]], "masked": [], "coding": null});
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    e.end(q, EndReason::Complete, 0, Value::Null);
    let up = e.open(StreamKind::WsUp, c, q, json!({"deflate": false}));
    let down = e.open(StreamKind::WsDown, c, q, json!({"deflate": false}));
    let mut ws = Ws(up, down, 0, 0);
    let (zero, _) = usage(500, 0, 0, 0);
    let mut warm = create("fake-warm", None);
    warm["generate"] = json!(false);
    let calls = [("fake-ws-a", None, usage(1000, 600, 200, 50)), ("fake-ws-b", Some("r_a"), usage(1300, 1100, 90, 0))];
    for (i, (model, prev, (u, t))) in calls.into_iter().enumerate() {
        if i == 0 {
            ws.send(&mut e, warm.clone());
            let w = [resp("response.created", "r_w1", "fake-warm-dated", json!({})),
                resp("response.completed", "r_w1", "fake-warm-dated", json!({"status": "completed", "usage": zero}))];
            ws.recv(&mut e, &w);
        }
        let id = format!("r_{}", &model[8..]);
        let reply = format!("{model}-dated");
        ws.send(&mut e, create(model, prev));
        let done = resp("response.completed", &id, &reply, json!({"status": "completed", "usage": u}));
        ws.recv(&mut e, &[resp("response.created", &id, &reply, json!({})), delta(), done]);
        planted.insert(model, t);
    }
    ws.send(&mut e, warm);
    ws.recv(&mut e, &[resp("response.created", "r_w2", "fake-warm-dated", json!({})),
        resp("response.completed", "r_w2", "fake-warm-dated", json!({"status": "completed", "usage": zero}))]);
    // Interrupted in flight: no terminal event follows.
    ws.send(&mut e, create("fake-ws-d", None));
    ws.recv(&mut e, &[resp("response.created", "r_d", "fake-ws-d-dated", json!({})), delta()]);
    ws.send(&mut e, json!({"type": "response.interrupt"}));
    // Rate limited, sent both as `error` and `response.failed`, in both orders.
    let rl = json!({"code": RL, "message": MARK});
    let err = json!({"type": "error", "status": 429, "error": rl});
    for (model, id, error_first) in [("fake-ws-r1", "r_r1", true), ("fake-ws-r2", "r_r2", false)] {
        ws.send(&mut e, create(model, None));
        let failed = resp("response.failed", id, model, json!({"status": "failed", "error": rl}));
        let (x, y) = if error_first { (err.clone(), failed) } else { (failed, err.clone()) };
        ws.recv(&mut e, &[resp("response.created", id, model, json!({})), x, y]);
    }
    let (u, t) = usage(2000, 1500, 300, 120);
    ws.send(&mut e, create("fake-ws-e", Some("r_b")));
    ws.recv(&mut e, &[resp("response.created", "r_e", "fake-ws-e-dated", json!({})),
        resp("response.completed", "r_e", "fake-ws-e-dated", json!({"status": "completed", "usage": u}))]);
    planted.insert("fake-ws-e", t);
    e.end(up, EndReason::Complete, ws.2, json!({"close_code": 1000}));
    e.end(down, EndReason::Complete, ws.3, json!({"close_code": 1000}));
    e.end(c, EndReason::Complete, 0, json!({"up": 900, "down": 900}));

    let http = |m: &str| json!({"model": m, "stream": true, "reasoning": {"effort": "low"}, "input": MARK});
    let (u, t) = usage(700, 100, 90, 30);
    let body = sse(&[resp("response.created", "r_h1", "fake-http-a-dated", json!({})), delta(),
        resp("response.completed", "r_h1", "fake-http-a-dated", json!({"status": "completed", "usage": u}))]);
    post(&mut e, "/v1/responses", http("fake-http-a"), 200, "text/event-stream", &body, EndReason::Complete);
    planted.insert("fake-http-a", t);
    let body = json!({"error": rl}).to_string();
    post(&mut e, "/v1/responses", http("fake-http-429"), 429, "application/json", &body, EndReason::Complete);
    let body = sse(&[resp("response.created", "r_cut", "fake-http-cut", json!({})), delta()]);
    post(&mut e, "/v1/responses", http("fake-http-cut"), 200, "text/event-stream", &body, EndReason::Interrupted);
    let au = |out: u64| json!({"input_tokens": 300, "cache_read_input_tokens": 40,
        "cache_creation_input_tokens": 20, "output_tokens": out});
    let body = sse(&[
        json!({"type": "message_start", "message": {"id": "msg_fake_1", "model": "fake-claude-dated", "usage": au(1)}}),
        json!({"type": "ping"}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": MARK}}),
        json!({"type": "message_delta", "delta": {"stop_reason": null}, "usage": au(30)}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": au(75)}),
        json!({"type": "message_stop"}),
    ]);
    let req = json!({"model": "fake-claude", "max_tokens": 64, "stream": true,
        "messages": [{"role": "user", "content": MARK}]});
    post(&mut e, "/v1/messages?beta=true", req, 200, "text/event-stream", &body, EndReason::Complete);
    let t = Tokens { input: Some(300), cached: Some(40), cache_write: Some(20), output: Some(75), ..Tokens::default() };
    planted.insert("fake-claude", t);
    post(&mut e, "/v1/fake-events", json!({"event": MARK}), 200, "application/json", r#"{"ok":true}"#, EndReason::Complete);
    let dir = e.seal();
    (read_records(&dir.join("rec")), planted)
}

fn table(recs: &[Record]) -> CallTable {
    let mut calls = Calls::new();
    for r in recs {
        calls.push(r);
    }
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
        assert_eq!(r.kind, Kind::Call);
        assert!(matches!(&r.outcome, Outcome::RateLimited { code: Some(c), .. } if c == RL), "{m}: {r:?}");
    }
    let r = row(&t, "fake-http-429");
    assert!(matches!(r.outcome, Outcome::RateLimited { status: Some(429), .. }), "{r:?}");
    assert_eq!(t.rows.iter().filter(|r| r.outcome.is_error()).count(), 3);
    let after = row(&t, "fake-ws-e");
    assert_eq!(after.outcome, Outcome::Ok);
    assert_eq!(after.tokens, planted["fake-ws-e"]);
}

#[test]
fn tokens_and_outcomes_follow_the_shape() {
    let (recs, planted) = run1("u-shape");
    let t = table(&recs);
    let a = row(&t, "fake-ws-a");
    assert_eq!((a.kind, &a.outcome, a.whole), (Kind::Call, &Outcome::Ok, true));
    assert_eq!(a.tokens, planted["fake-ws-a"]);
    assert_eq!(a.model.as_deref(), Some("fake-ws-a-dated"));
    assert_eq!(a.effort.as_deref(), Some("low"));
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
        ts.iter().fold([0u64; 5], |mut s, x| {
            for (i, v) in [x.input, x.cached, x.cache_write, x.output, x.reasoning].iter().enumerate() {
                s[i] += v.unwrap_or(0);
            }
            s
        })
    };
    let calls: Vec<&Tokens> = t.rows.iter().filter(|r| r.kind == Kind::Call).map(|r| &r.tokens).collect();
    assert_eq!(sum(calls), sum(planted.values().collect()));
    assert_eq!(table(&recs), t);
}

#[test]
fn no_row_holds_text_from_a_body() {
    let t = table(&run1("u-mark").0);
    assert!(!t.rows.is_empty());
    assert!(!format!("{t:?}").contains(MARK), "a row holds body text");
}
