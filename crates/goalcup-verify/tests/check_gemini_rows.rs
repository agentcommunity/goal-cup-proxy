//! Gemini CLI native call rows (PRD-09 sections 4 and 6): every `type: "gemini"` message with
//! `tokens` in a chat file is one native call, the last copy per message `id` kept (Gemini writes
//! each message twice). The rows join the wire by time, the wire call whose usage equals theirs
//! first, so the threads of the main and its subagent own their wire calls and a utility call
//! that no chat file holds stays wire-only. Every value is an obvious fake.

mod common;

use common::{Entry, call_table, edit_event, mutate};
use goalcup_format::{EndReason, StreamKind};
use goalcup_verify::usage::{CallRow, CallTable, Join, Kind, Leg, Thread};
use serde_json::{Value, json};

const MAIN: &str = "d56586ab-0000-4000-8000-000000000001";
const KID: &str = "5ab0c0de-0000-4000-8000-000000000002";
const MAIN_FILE: &str = "chats:proj/chats/session-2026-11-07T00-00-d56586ab.jsonl";

/// One fake `generateContent` call with this usage on its own connection.
fn wire(e: &mut Entry, model: &str, prompt: u64, cached: u64, out: u64) {
    let c = e.conn();
    let path = format!("/v1beta/models/{model}:generateContent");
    let head = json!({"method": "POST", "path": path, "ver": "1.1", "masked": [], "coding": null,
        "headers": [["content-type", "application/json"]]});
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    e.data(q, 0, 0, b"{}");
    e.end(q, EndReason::Complete, 2, Value::Null);
    let mut usage = json!({"promptTokenCount": prompt, "candidatesTokenCount": out});
    if cached > 0 {
        usage["cachedContentTokenCount"] = cached.into();
    }
    let body = json!({"modelVersion": model, "responseId": format!("resp-{prompt}"),
        "candidates": [{"finishReason": "STOP"}], "usageMetadata": usage})
    .to_string();
    let head = json!({"status": 200, "ver": "1.1", "decoded": true, "masked": [], "coding": null,
        "headers": [["content-type", "application/json"]]});
    let p = e.open(StreamKind::HttpResp, c, q, head);
    e.data(p, 0, 0, body.as_bytes());
    e.end(p, EndReason::Complete, body.len() as u64, Value::Null);
    e.end(c, EndReason::Complete, 0, json!({"up": 90, "down": 90}));
}

/// One native stream holding `lines` as the file at `path`, ended complete.
fn file(e: &mut Entry, path: &str, lines: &[Value]) {
    let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let s = e.open(
        StreamKind::Native,
        0,
        0,
        json!({"path": path, "gen": 0, "from": 0}),
    );
    e.data(s, 0, 0, body.as_bytes());
    e.end(s, EndReason::Complete, body.len() as u64, Value::Null);
}

/// A model message line: id, time (seconds after the test clock's start) and token counts.
fn msg(id: &str, sec: u64, input: u64, cached: u64, out: u64) -> Value {
    json!({"id": id, "timestamp": format!("2026-11-07T00:00:0{sec}.000Z"), "type": "gemini",
        "model": "fake-gem", "tokens": {"input": input, "output": out, "cached": cached,
        "thoughts": 0, "tool": 0, "total": input + out}})
}

/// Wire calls A (main), U (a utility call no file holds) and B (the subagent), in that order,
/// and the two chat files; the main message is written twice, the first copy with a smaller
/// output.
fn table(name: &str) -> CallTable {
    let mut e = Entry::new(name);
    wire(&mut e, "fake-gem", 100, 0, 7);
    wire(&mut e, "fake-gem", 300, 0, 2);
    wire(&mut e, "fake-gem", 200, 50, 9);
    let user = json!({"id": "u-0001", "timestamp": "2026-11-07T00:00:01.000Z", "type": "user"});
    let mut first = msg("m-0001", 2, 100, 0, 6);
    first["tokens"]["total"] = 106.into();
    let main = [
        json!({"sessionId": MAIN, "kind": "main"}),
        user,
        json!({"$set": {"lastUpdated": "2026-11-07T00:00:01.000Z"}}),
        first,
        msg("m-0001", 2, 100, 0, 7),
        json!({"$rewindTo": "u-0001"}),
    ];
    file(&mut e, MAIN_FILE, &main);
    let kid = [
        json!({"sessionId": KID, "kind": "subagent"}),
        msg("k-0001", 3, 200, 50, 9),
    ];
    file(
        &mut e,
        &format!("chats:proj/chats/{MAIN}/{KID}.jsonl"),
        &kid,
    );
    e.count();
    let dir = e.seal();
    let gemini = |v: &mut Value| v["connector"] = "gemini-cli".into();
    call_table(&mutate(&dir, None, |r| edit_event(r, "start", 0, gemini)))
}

fn wire_call(t: &CallTable, prompt: u64) -> &CallRow {
    let mut all = t
        .rows
        .iter()
        .filter(|r| r.leg == Leg::Wire && r.kind == Kind::Call);
    let full = |r: &&CallRow| r.tokens.input.unwrap_or(0) + r.tokens.cached.unwrap_or(0);
    all.find(|r| full(r) == prompt)
        .unwrap_or_else(|| panic!("no wire call of {prompt}"))
}

fn thread<'a>(t: &'a CallTable, id: &str) -> &'a Thread {
    let found = t.threads.iter().find(|x| x.id == id);
    found.unwrap_or_else(|| panic!("no thread {id}"))
}

#[test]
fn one_native_row_per_message_id() {
    let t = table("gr-a");
    let nat: Vec<_> = t.rows.iter().filter(|r| r.leg == Leg::Native).collect();
    assert_eq!(nat.len(), 2);
    let main = nat.iter().find(|r| r.thread.as_deref() == Some(MAIN));
    let main = main.expect("a main row");
    // The last copy wins; input is the part not read from a cache, as on the wire.
    assert_eq!(
        (main.tokens.input, main.tokens.output),
        (Some(100), Some(7))
    );
    assert_eq!(main.model.as_deref(), Some("fake-gem"));
    let kid = nat.iter().find(|r| r.thread.as_deref() == Some(KID));
    let kid = kid.expect("a subagent row");
    assert_eq!((kid.tokens.input, kid.tokens.cached), (Some(150), Some(50)));
    assert_eq!(kid.parent.as_deref(), Some(MAIN));
    assert!(nat.iter().all(|r| r.join == Some(Join::Matched)));
}

#[test]
fn the_rows_take_their_wire_calls_and_the_utility_call_stays_wire_only() {
    let t = table("gr-b");
    let (a, u, b) = (wire_call(&t, 100), wire_call(&t, 300), wire_call(&t, 200));
    assert_eq!(
        (a.join, a.thread.as_deref()),
        (Some(Join::Matched), Some(MAIN))
    );
    assert_eq!(
        (b.join, b.thread.as_deref()),
        (Some(Join::Matched), Some(KID))
    );
    assert_eq!(b.parent.as_deref(), Some(MAIN));
    assert_eq!((u.join, u.thread.as_deref()), (Some(Join::WireOnly), None));
    let lost = t.rows.iter().filter(|r| r.join == Some(Join::Unattributed));
    assert_eq!(lost.count(), 0);
}

#[test]
fn both_threads_are_readable_with_their_calls() {
    let t = table("gr-c");
    assert!(t.readable());
    let (main, kid) = (thread(&t, MAIN), thread(&t, KID));
    assert_eq!((main.native_calls, main.native_only), (1, 0));
    assert_eq!((kid.native_calls, kid.native_only), (1, 0));
    assert_eq!(kid.parent.as_deref(), Some(MAIN));
    let kids = t.children.as_ref().expect("parentage was read");
    assert_eq!(kids[MAIN], [KID]);
}
