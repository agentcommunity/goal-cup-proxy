//! `history_complete` (PRD-08 section 3.2, ledger T4; points 1, 2 and 3): an answer resent inside
//! a later request but never recorded as a response is a missed call. One small record per case,
//! all values fake. The last test also writes PRD-12's K11 fixtures when `GOALCUP_FIXTURES_OUT`
//! is set.

mod common;

use common::*;
use goalcup_format::{EndReason, FLAG_FIN, GapReason, Record, StreamKind};
use serde_json::{Value, json};
use std::env;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq)]
enum Sh {
    Chat,
    Anthropic,
    Responses,
    Gemini,
}
use Sh::*;

/// One assistant answer: a tool call, or a text.
#[derive(Clone, Copy)]
enum A {
    Call(u64),
    Text(u64),
}
use A::*;

/// The thread of every request, unless a case names others; the text of the one new user turn in
/// every request body, which no finding may carry.
const T: &str = "t-main";
const TURN: &str = "fake turn text";

fn tid(s: Sh, k: u64) -> String {
    format!("{}_{k}", if s == Anthropic { "toolu" } else { "call" })
}

fn say(n: u64) -> String {
    format!("fake answer {n}")
}

/// Tool call `k` as Chat lists it in `tool_calls`, or Anthropic as a `tool_use` block.
fn part(s: Sh, k: u64) -> Value {
    match s {
        Anthropic => json!({"type": "tool_use", "id": tid(s, k), "name": "f", "input": {}}),
        _ => json!({"id": tid(s, k), "type": "function", "function": {"name": "f"}}),
    }
}

/// Answer `a` as a Responses output item.
fn out(a: A) -> Value {
    match a {
        Call(k) => json!({"type": "function_call", "call_id": tid(Responses, k), "name": "f"}),
        Text(n) => json!({"type": "message", "role": "assistant",
            "content": [{"type": "output_text", "text": say(n)}]}),
    }
}

fn user(s: Sh) -> Value {
    match s {
        Chat | Anthropic => json!({"role": "user", "content": TURN}),
        Responses => json!({"type": "message", "role": "user",
            "content": [{"type": "input_text", "text": TURN}]}),
        Gemini => json!({"role": "user", "parts": [{"text": TURN}]}),
    }
}

/// Answer `a` as shape `s` resends it, and the turn that follows it.
fn ans(s: Sh, a: A) -> [Value; 2] {
    let tool = |k| json!({"role": "tool", "tool_call_id": tid(s, k), "content": "ok"});
    match (s, a) {
        (Chat, Call(k)) => [
            json!({"role": "assistant", "content": null, "tool_calls": [part(s, k)]}),
            tool(k),
        ],
        (Anthropic, Call(k)) => [
            json!({"role": "assistant", "content": [part(s, k)]}),
            json!({"role": "user", "content": [{"type": "tool_result",
                "tool_use_id": tid(s, k), "content": "ok"}]}),
        ],
        (Responses, Call(k)) => [
            out(a),
            json!({"type": "function_call_output", "call_id": tid(s, k), "output": "ok"}),
        ],
        (Responses, Text(_)) => [out(a), user(s)],
        (Gemini, Text(n)) => [
            json!({"role": "model", "parts": [{"text": say(n)}]}),
            user(s),
        ],
        (_, Text(n)) => [json!({"role": "assistant", "content": say(n)}), user(s)],
        (Gemini, Call(_)) => unreachable!("Gemini function calls carry no id"),
    }
}

/// The turns of a request that resends `hist`, then one new user turn.
fn turns(s: Sh, hist: &[A]) -> Vec<Value> {
    let mut v: Vec<Value> = hist.iter().flat_map(|&a| ans(s, a)).collect();
    v.push(user(s));
    v
}

/// The reply that carries answer `a`: its content type and body.
fn reply(s: Sh, a: A) -> (&'static str, String) {
    let sse = |evs: &[Value]| {
        evs.iter()
            .map(|e| format!("data: {e}\n\n"))
            .collect::<String>()
    };
    let delta = |d: Value| json!({"choices": [{"index": 0, "delta": d}]});
    let text = match a {
        Text(n) | Call(n) => say(n),
    };
    let (stream, body) = match (s, a) {
        (Chat, Call(k)) => (true, sse(&[delta(json!({"tool_calls": [part(s, k)]}))])),
        (Chat, Text(_)) => {
            let two = [
                delta(json!({"content": &text[..5]})),
                delta(json!({"content": &text[5..]})),
            ];
            (true, sse(&two))
        }
        (Responses, _) => {
            let done = json!({"type": "response.output_item.done", "item": out(a)});
            (
                true,
                sse(&[done, json!({"type": "response.completed", "response": {}})]),
            )
        }
        (Gemini, _) => {
            // Streamed, with a thought part before the answer; the resent turn has no thought.
            let parts = json!([{"text": "fake thought", "thought": true}, {"text": text}]);
            let c = json!({"candidates": [{"content": {"role": "model", "parts": parts}}]});
            (true, sse(&[c]))
        }
        (Anthropic, Text(_)) => {
            // Streamed, with a thinking block before the answer; the resent turn has no thinking.
            let d =
                |i: u64, d: Value| json!({"type": "content_block_delta", "index": i, "delta": d});
            let evs = [
                d(
                    0,
                    json!({"type": "thinking_delta", "thinking": "fake thought"}),
                ),
                d(1, json!({"type": "text_delta", "text": &text[..5]})),
                d(1, json!({"type": "text_delta", "text": &text[5..]})),
                json!({"type": "message_stop"}),
            ];
            (true, sse(&evs))
        }
        (Anthropic, _) => (false, json!(ans(s, a)[0]).to_string()),
    };
    (
        if stream {
            "text/event-stream"
        } else {
            "application/json"
        },
        body,
    )
}

/// One decrypted POST of `body` with its reply, on its own connection. Returns the request's sid.
/// With `lose` 1, 10 bytes in the middle of the request body are lost to a `memory_guard` GAP;
/// with 2, the response is undecoded (`decoded: false`) and sits under an `unsupported_encoding` GAP.
fn send(e: &mut Entry, path: &str, thread: &str, body: &Value, rep: (&str, String)) -> u64 {
    send_with(e, path, thread, body, rep, 0)
}

fn send_with(
    e: &mut Entry,
    path: &str,
    thread: &str,
    body: &Value,
    rep: (&str, String),
    lose: u8,
) -> u64 {
    let body = body.to_string();
    let head = json!({"method": "POST", "path": path, "ver": "1.1", "masked": [], "coding": null,
        "headers": [["content-type", "application/json"], ["thread-id", thread]]});
    let (c, done) = (e.conn(), EndReason::Complete);
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    if lose == 1 {
        let half = body.len() / 2;
        e.data(q, 0, 0, &body.as_bytes()[..half]);
        let tail = json!({"guard": "held_bytes"});
        e.gap(q, GapReason::MemoryGuard, 10, half as u64, 10, tail);
        e.data(q, half as u64 + 10, 0, &body.as_bytes()[half + 10..]);
    } else {
        e.data(q, 0, 0, body.as_bytes());
    }
    e.end(q, done, body.len() as u64, Value::Null);
    let head = json!({"status": 200, "ver": "1.1", "headers": [["content-type", rep.0]],
        "masked": [], "coding": null, "decoded": lose != 2});
    let p = e.open(StreamKind::HttpResp, c, q, head);
    if lose == 2 {
        e.gap(p, GapReason::UnsupportedEncoding, 10, 0, 0, Value::Null);
        e.end(p, done, 0, Value::Null);
    } else {
        e.data(p, 0, 0, rep.1.as_bytes());
        e.end(p, done, rep.1.len() as u64, Value::Null);
    }
    e.end(c, done, 0, json!({"up": 900, "down": 900}));
    e.idle(1_000);
    q
}

/// A request of shape `s` that resends `hist`, answered with `a`.
fn call(e: &mut Entry, s: Sh, thread: &str, hist: &[A], a: A) -> u64 {
    call_with(e, s, thread, hist, a, 0)
}

fn call_with(e: &mut Entry, s: Sh, thread: &str, hist: &[A], a: A, lose: u8) -> u64 {
    let (path, key) = match s {
        Chat => ("/v1/chat/completions", "messages"),
        Anthropic => ("/v1/messages", "messages"),
        Responses => ("/v1/responses", "input"),
        Gemini => ("/v1beta/models/fake-model:generateContent", "contents"),
    };
    let mut b = json!({"model": "fake-model"});
    b[key] = turns(s, hist).into();
    send_with(e, path, thread, &b, reply(s, a), lose)
}

/// `n` requests: request i resends answers 0..i and is answered with answer i (a text when `text`
/// or Gemini, else a tool call). `lost` is a request never written; with `true` the recorder
/// crashes in its place and comes back (a `recorder_down` GAP). Returns the sids written.
fn chain(e: &mut Entry, s: Sh, text: bool, n: u64, lost: Option<(usize, bool)>) -> Vec<u64> {
    let mk: fn(u64) -> A = if text || s == Gemini { Text } else { Call };
    let all: Vec<A> = (0..n).map(mk).collect();
    let mut sids = vec![];
    for (i, &a) in all.iter().enumerate() {
        match lost {
            Some((j, true)) if j == i => {
                e.crash("killed", 60_000);
            }
            Some((j, false)) if j == i => {}
            _ => sids.push(call(e, s, T, &all[..i], a)),
        }
    }
    sids
}

/// One WebSocket connection of Responses requests on `thread`. Message 0 is no request (a ping);
/// each of `msgs` is a `response.create` resending the history, answered with the answer, if any.
/// Offsets run through each stream. Returns the client stream's sid.
fn ws(e: &mut Entry, thread: &str, msgs: &[(&[A], Option<A>)]) -> u64 {
    fn put(e: &mut Entry, sid: u64, off: &mut u64, m: &Value) {
        e.data(sid, *off, 0x10 | FLAG_FIN, m.to_string().as_bytes());
        *off += m.to_string().len() as u64;
    }
    let c = e.conn();
    let head = json!({"method": "GET", "path": "/v1/responses", "ver": "1.1",
        "headers": [["upgrade", "websocket"]], "masked": [], "coding": null});
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    e.end(q, EndReason::Complete, 0, Value::Null);
    let up = e.open(StreamKind::WsUp, c, q, json!({"deflate": false}));
    let down = e.open(StreamKind::WsDown, c, q, json!({"deflate": false}));
    let (mut u, mut d) = (0, 0);
    put(e, up, &mut u, &json!({"type": "ping"}));
    for (hist, a) in msgs {
        let create = json!({"type": "response.create", "input": turns(Responses, hist),
            "client_metadata": {"thread_id": thread}});
        put(e, up, &mut u, &create);
        if let Some(a) = a {
            put(
                e,
                down,
                &mut d,
                &json!({"type": "response.output_item.done", "item": out(*a)}),
            );
        }
        put(
            e,
            down,
            &mut d,
            &json!({"type": "response.completed", "response": {}}),
        );
    }
    let close = json!({"close_code": 1000});
    e.end(up, EndReason::Complete, u, close.clone());
    e.end(down, EndReason::Complete, d, close);
    e.end(c, EndReason::Complete, 0, json!({"up": 400, "down": 300}));
    up
}

/// The sealed folder of the record `f` writes. The connector is Codex's, so the wire requests
/// carry their thread (`thread-id` header, `client_metadata.thread_id`).
fn rec(name: &str, f: impl FnOnce(&mut Entry)) -> PathBuf {
    let mut e = Entry::new(name);
    f(&mut e);
    e.count();
    mutate(&e.seal(), None, |recs| {
        for r in recs {
            if let Record::Event { json, .. } = r {
                let mut v: Value = serde_json::from_slice(json).unwrap();
                if v["ev"] == "start" {
                    v["connector"] = "codex-cli".into();
                    *json = v.to_string().into_bytes();
                }
            }
        }
    })
}

/// Checks `dir`, asserts the history result and the exit code (naming every failing id on a
/// miss) and returns the history check.
fn verdict(dir: &Path, res: &str, exit: i32) -> Value {
    let (code, v) = check(dir, &[]);
    let h = item(&v, "history_complete").clone();
    let fails = ids_with(&v, "fail");
    assert_eq!(
        (h["result"].as_str().unwrap(), code),
        (res, exit),
        "{fails:?} {h}"
    );
    h
}

fn n(h: &Value, k: &str) -> u64 {
    h["facts"][k].as_u64().unwrap()
}

/// (missing_call, missing_text, unanchored) of a history check.
fn missing(h: &Value) -> (u64, u64, u64) {
    (
        n(h, "missing_call"),
        n(h, "missing_text"),
        n(h, "unanchored"),
    )
}

#[test]
fn a_clean_chain_passes_in_every_request_shape() {
    for s in [Chat, Anthropic, Responses, Gemini] {
        let h = verdict(
            &rec("h-clean", |e| drop(chain(e, s, false, 3, None))),
            "pass",
            0,
        );
        assert_eq!((n(&h, "checked"), missing(&h)), (2, (0, 0, 0)));
    }
    // Answers streamed with a thinking block (Gemini: a thought part) still match the text a request resends.
    for s in [Anthropic, Chat, Gemini] {
        let d = rec("h-clean-text", |e| drop(chain(e, s, true, 3, None)));
        assert_eq!(missing(&verdict(&d, "pass", 0)), (0, 0, 0));
    }
}

#[test]
fn an_undecoded_response_under_its_gap_names_the_missed_call() {
    for s in [Chat, Anthropic, Responses] {
        let d = rec("h-undecoded", |e| {
            call_with(e, s, T, &[], Call(0), 2);
            call(e, s, T, &[Call(0)], Call(1));
            call(e, s, T, &[Call(0), Call(1)], Call(2));
        });
        assert_eq!(missing(&verdict(&d, "named", 1)).0, 1);
    }
}

#[test]
fn a_call_dropped_with_no_gap_fails_and_a_named_gap_over_it_names_it() {
    for (s, j) in [
        (Chat, 1),
        (Anthropic, 1),
        (Responses, 1),
        (Chat, 2),
        (Anthropic, 2),
    ] {
        let mut sids = vec![];
        let d = rec("h-drop", |e| sids = chain(e, s, false, 5, Some((j, false))));
        let h = verdict(&d, "fail", 2);
        // The request after the dropped one resends its answer; it is written at place j.
        assert_eq!(h["first"]["sid"], sids[j]);
        assert_eq!(missing(&h), (1, 0, 0));
        let d = rec("h-down", |e| drop(chain(e, s, false, 5, Some((j, true)))));
        assert_eq!(missing(&verdict(&d, "named", 1)), (1, 0, 0));
    }
}

/// A GAP on a stream that is no model call (a download) lies between two requests; a call
/// dropped there with no GAP of its own is still a `fail`.
#[test]
fn a_gap_on_an_unrelated_stream_does_not_cover_a_dropped_call() {
    let d = rec("h-unrelated", |e| {
        let all: Vec<A> = (0..5).map(Call).collect();
        for (i, &a) in all.iter().enumerate() {
            if i != 1 {
                call(e, Chat, T, &all[..i], a);
                continue;
            }
            let (c, done) = (e.conn(), EndReason::Complete);
            let head = json!({"method": "GET", "path": "/file", "ver": "1.1", "masked": [],
                "coding": null, "headers": []});
            let q = e.open(StreamKind::HttpReq, c, 0, head);
            e.end(q, done, 0, Value::Null);
            let head = json!({"status": 200, "ver": "1.1", "headers": [], "masked": [],
                "coding": null, "decoded": true});
            let p = e.open(StreamKind::HttpResp, c, q, head);
            e.gap(
                p,
                GapReason::MemoryGuard,
                10,
                0,
                0,
                json!({"guard": "held_bytes"}),
            );
            e.end(p, done, 0, Value::Null);
            e.end(c, done, 0, json!({"up": 90, "down": 90}));
            e.idle(1_000);
        }
    });
    assert_eq!(missing(&verdict(&d, "fail", 2)), (1, 0, 0));
}

#[test]
fn a_missing_text_answer_only_warns() {
    for s in [Anthropic, Chat] {
        let d = rec("h-text", |e| drop(chain(e, s, true, 5, Some((2, false)))));
        assert_eq!(missing(&verdict(&d, "warn", 0)), (0, 1, 0));
    }
}

#[test]
fn history_from_before_the_record_warns_and_never_fails() {
    let d = rec("h-unanchored", |e| {
        call(e, Chat, T, &[Call(99)], Call(0));
        call(e, Chat, T, &[Call(99), Call(0)], Call(1));
    });
    let h = verdict(&d, "warn", 0);
    assert_eq!((n(&h, "checked"), missing(&h)), (2, (0, 0, 1)));
}

/// A one-shot request on another thread leaves an empty chain that nothing continues; a request
/// of a resumed session must not take it (it is that session's first request, never a `fail`).
#[test]
fn a_stray_empty_chain_of_another_thread_is_not_taken() {
    let d = rec("h-stray", |e| {
        call(e, Chat, "t-side", &[], Call(500));
        call(e, Chat, "t-res", &[Call(99)], Call(0));
    });
    let h = verdict(&d, "warn", 0);
    assert_eq!(missing(&h), (0, 0, 1));
}

/// A request body lost to a GAP is skipped and counted; its whole reply is still an answer and
/// the thread goes on being checked.
#[test]
fn a_request_body_lost_to_a_gap_does_not_switch_off_its_thread() {
    let d = rec("h-hole", |e| {
        let all: Vec<A> = (0..4).map(Call).collect();
        for (i, &a) in all.iter().enumerate() {
            call_with(e, Chat, T, &all[..i], a, u8::from(i == 1));
        }
    });
    let (_, v) = check(&d, &[]);
    let h = item(&v, "history_complete");
    assert_eq!(h["result"], "pass", "{h}");
    assert_eq!(
        (
            h["facts"]["skipped"]["undecoded"].as_u64(),
            h["facts"]["skipped"]["compacted"].as_u64()
        ),
        (Some(1), Some(0))
    );
    assert_eq!((n(h, "checked"), missing(h)), (3, (0, 0, 0)));
}

#[test]
fn requests_with_only_a_previous_response_id_are_skipped() {
    let d = rec("h-prev", |e| {
        for i in 0..3 {
            let b = json!({"input": turns(Responses, &[]), "previous_response_id": "resp_0"});
            send(e, "/v1/responses", T, &b, reply(Responses, Call(i)));
        }
    });
    let h = verdict(&d, "skip", 0);
    assert_eq!(h["facts"]["skipped"]["previous_response_id"], 3);
}

/// The history is rewritten in the same thread (compaction): the old chain is still judged, the
/// rest of the thread is skipped and counted.
#[test]
fn compaction_skips_the_rest_of_the_thread_and_keeps_the_earlier_turns() {
    for (lost, res, exit) in [(None, "pass", 0), (Some((2, false)), "fail", 2)] {
        let d = rec("h-compact", |e| {
            chain(e, Chat, false, 4, lost);
            call(e, Chat, T, &[Text(900)], Call(4));
            call(e, Chat, T, &[Text(900), Call(4)], Call(5));
        });
        let h = verdict(&d, res, exit);
        assert!(h["facts"]["skipped"]["compacted"].as_u64().unwrap() > 0);
        assert_eq!(
            (missing(&h).0, missing(&h).2),
            (u64::from(lost.is_some()), 0)
        );
    }
}

#[test]
fn an_answer_recorded_in_another_thread_matches() {
    let d = rec("h-other", |e| {
        call(e, Chat, "t-a", &[], Call(100));
        call(e, Chat, "t-b", &[Call(100)], Call(200));
    });
    let h = verdict(&d, "pass", 0);
    assert_eq!((n(&h, "checked"), missing(&h)), (1, (0, 0, 0)));
}

/// The lost call is found by (sid, message number): the thread is the one of the second message
/// on its stream, after a message that is no request, so only a count of every message names it.
#[test]
fn a_call_dropped_on_a_websocket_fails_and_names_the_thread() {
    let mut up = 0;
    let d = rec("h-ws", |e| {
        up = ws(e, "t-ws", &[(&[], None), (&[Call(0)], Some(Call(1)))]);
    });
    let h = verdict(&d, "fail", 2);
    assert_eq!(h["first"]["sid"], up);
    assert_eq!(missing(&h), (1, 0, 0));
}

#[test]
fn a_record_with_only_a_pinned_tunnel_skips() {
    let d = rec("h-tunnel", |e| {
        let meta = json!({"host": HOST, "port": 443, "mode": "tunnel", "listed": "shared",
            "kind": "provider", "reason": "pinned", "alpn": null, "tls": null,
            "fingerprint": "0123456789abcdef"});
        let s = e.open(StreamKind::Conn, 0, 0, meta);
        e.gap(s, GapReason::Pinned, 1_000, 0, 2_561, Value::Null);
        e.end(s, EndReason::Complete, 0, json!({"up": 512, "down": 2048}));
    });
    assert_eq!(n(&verdict(&d, "skip", 1), "checked"), 0);
}

/// Five requests that never resend history.
fn every_call_alone(e: &mut Entry, s: Sh) {
    for i in 0..5 {
        call(e, s, T, &[], Call(i));
    }
}

/// PRD-12 K11: (a) a response left out, later requests resend its answer; (b) the same with a
/// named gap; (c) requests that never resend history: `skip`, not a pass with 0 checked; (d) a
/// clean wire-only record. With `GOALCUP_FIXTURES_OUT` set the four go to `<out>/k11-<x>-part`.
#[test]
fn the_k11_records_give_their_four_results() {
    let (r, c) = (Responses, Chat);
    let k11 = [
        (
            "a",
            rec("k11-a", |e| drop(chain(e, r, false, 5, Some((2, false))))),
            "fail",
            2,
        ),
        (
            "b",
            rec("k11-b", |e| drop(chain(e, r, false, 5, Some((2, true))))),
            "named",
            1,
        ),
        ("c", rec("k11-c", |e| every_call_alone(e, c)), "skip", 0),
        (
            "d",
            rec("k11-d", |e| drop(chain(e, c, false, 5, None))),
            "pass",
            0,
        ),
    ];
    for (x, d, res, exit) in k11 {
        let h = verdict(&d, res, exit);
        assert_eq!(n(&h, "missing_call"), u64::from(exit > 0));
        if let Ok(out) = env::var("GOALCUP_FIXTURES_OUT") {
            export(&d, &Path::new(&out).join(format!("k11-{x}-part")));
        }
    }
}
