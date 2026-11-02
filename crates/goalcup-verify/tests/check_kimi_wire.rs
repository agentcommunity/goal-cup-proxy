//! PRD-09 point 2, Kimi Code subagents (subagent design item D): one `wire.jsonl` per agent under
//! `sessions/<workspace>/<session>/agents/<name>/`. `main` is the root; an `agent-<n>` is a child
//! of the session's `main`, unless a `subagent.spawned` record in a file names another parent
//! agent. Each `usage.record` is one native call, timed by the `llm.request` before it, and joins
//! the wire by time. Every value is an obvious fake.

mod common;

use common::{Entry, WALL0, call_table, check, edit_event, mutate, result};
use goalcup_format::{EndReason, StreamKind};
use goalcup_verify::usage::{CallRow, CallTable, Join, Kind, Leg, Thread};
use serde_json::{Value, json};
use std::path::PathBuf;

const SID: &str = "session_fake-0000-0001";
const MAIN: &str = "session_fake-0000-0001/main";
const KID: &str = "session_fake-0000-0001/agent-0";
const NESTED: &str = "session_fake-0000-0001/agent-1";

/// The fake session's file of agent `name`.
fn path(name: &str) -> String {
    format!("wire:wd_fake_000000000000/{SID}/agents/{name}/wire.jsonl")
}

/// Wall time in epoch ms, `s` seconds after the entry's wall clock starts.
fn ms(s: u64) -> u64 {
    WALL0 / 1_000_000 + s * 1000
}

/// One `POST /chat/completions` with no thread header, answered whole and ok.
fn chat(e: &mut Entry) {
    let c = e.conn();
    let head = json!({"method": "POST", "path": "/v1/chat/completions", "ver": "1.1",
        "headers": [["content-type", "application/json"]], "masked": [], "coding": null});
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    let body = br#"{"model":"fake-model"}"#;
    e.data(q, 0, 0, body);
    e.end(q, EndReason::Complete, body.len() as u64, Value::Null);
    let body = json!({"id": "fake-reply", "model": "fake-model",
        "choices": [{"finish_reason": "stop"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 3}})
    .to_string();
    let head = json!({"status": 200, "ver": "1.1", "decoded": true, "masked": [], "coding": null,
        "headers": [["content-type", "application/json"]]});
    let p = e.open(StreamKind::HttpResp, c, q, head);
    e.data(p, 0, 0, body.as_bytes());
    e.end(p, EndReason::Complete, body.len() as u64, Value::Null);
    e.end(c, EndReason::Complete, 0, json!({"up": 90, "down": 90}));
}

/// One native stream holding these lines as the file at `path`, ended complete.
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

/// An `llm.request` and its `usage.record` for agent `agent`, at `at` epoch ms.
fn call(agent: &str, at: u64, usage: Value) -> [Value; 2] {
    call_until(agent, at, at + 400, usage)
}

/// The same, with the `usage.record` (the call's end) at `end` epoch ms.
fn call_until(agent: &str, at: u64, end: u64, usage: Value) -> [Value; 2] {
    [
        json!({"type": "llm.request", "agentId": agent, "kind": "loop", "provider": "fake",
            "model": "fake-kimi", "messageCount": 1, "time": at}),
        json!({"type": "usage.record", "agentId": agent, "model": "fake-kimi", "usage": usage,
            "usageScope": "turn", "time": end}),
    ]
}

fn main_lines() -> Vec<Value> {
    let mut v = call(
        "main",
        ms(10),
        json!({"inputOther": 11, "output": 5, "inputCacheRead": 100, "inputCacheCreation": 0}),
    )
    .to_vec();
    let spawn = json!({"type": "subagent.spawned", "subagentId": "agent-0",
        "subagentName": "coder", "parentToolCallId": "call_fake0", "parentAgentId": "main",
        "runInBackground": false, "time": ms(11)});
    // A record another agent's id names is not this file's call.
    let other = json!({"type": "usage.record", "agentId": "agent-0", "model": "fake-kimi",
        "usage": {"inputOther": 1, "output": 1}, "time": ms(12)});
    v.extend([spawn, other]);
    v
}

/// The `agent-0` file; `nested` adds its spawn of a custom `agent-1` that has no file here.
fn kid_lines(nested: bool) -> Vec<Value> {
    let spawn = json!({"type": "subagent.spawned", "subagentId": "agent-1",
        "subagentName": "custom", "parentToolCallId": "call_fake1", "parentAgentId": "agent-0",
        "runInBackground": false, "time": ms(13)});
    let mut v: Vec<Value> = nested.then_some(spawn).into_iter().collect();
    v.extend(call(
        "agent-0",
        ms(14),
        json!({"inputOther": 7, "output": 3, "inputCacheRead": 50, "inputCacheCreation": 2}),
    ));
    v
}

/// A sealed Kimi Code entry with two wire calls and the main and `agent-0` files.
fn entry(name: &str, nested: bool) -> PathBuf {
    let mut e = Entry::new(name);
    chat(&mut e);
    chat(&mut e);
    file(&mut e, &path("main"), &main_lines());
    file(&mut e, &path("agent-0"), &kid_lines(nested));
    e.count();
    let dir = e.seal();
    let set = |v: &mut Value| v["connector"] = "kimi-code-cli".into();
    mutate(&dir, None, |r| edit_event(r, "start", 0, set))
}

fn thread<'a>(t: &'a CallTable, id: &str) -> &'a Thread {
    t.threads
        .iter()
        .find(|x| x.id == id)
        .unwrap_or_else(|| panic!("no thread {id}"))
}

fn rows(t: &CallTable, leg: Leg) -> Vec<&CallRow> {
    let call = |r: &&CallRow| r.leg == leg && r.kind == Kind::Call;
    t.rows.iter().filter(call).collect()
}

#[test]
fn each_agent_file_is_a_thread_under_its_parent() {
    let t = call_table(&entry("kw-a", true));
    let (main, kid, nested) = (thread(&t, MAIN), thread(&t, KID), thread(&t, NESTED));
    assert_eq!((main.parent.as_deref(), main.depth), (None, Some(0)));
    assert_eq!((kid.parent.as_deref(), kid.depth), (Some(MAIN), Some(1)));
    // Named only by a spawn record, with the parent agent it gives.
    assert_eq!(nested.parent.as_deref(), Some(KID));
    assert_eq!((main.native_calls, kid.native_calls), (1, 1));
    let kids = t.children.as_ref().expect("parentage was read");
    assert_eq!(kids[MAIN], [KID]);
    assert_eq!(kids[KID], [NESTED]);
}

#[test]
fn each_usage_record_is_one_call_with_its_tokens() {
    let t = call_table(&entry("kw-b", true));
    let native = rows(&t, Leg::Native);
    assert_eq!(native.len(), 2);
    let kid = native
        .iter()
        .find(|r| r.thread.as_deref() == Some(KID))
        .expect("agent-0 call");
    let k = &kid.tokens;
    let got = (k.input, k.cached, k.cache_write, k.output, k.reasoning);
    assert_eq!(got, (Some(7), Some(50), Some(2), Some(3), None));
    assert_eq!(kid.parent.as_deref(), Some(MAIN));
    assert_eq!(kid.model.as_deref(), Some("fake-kimi"));
    // Timed by the `llm.request` before it, epoch ms as ns.
    assert_eq!(kid.wall_start, Some(ms(14) * 1_000_000));
}

#[test]
fn native_calls_join_the_wire_by_time() {
    let t = call_table(&entry("kw-c", true));
    let wire = rows(&t, Leg::Wire);
    assert_eq!(wire.len(), 2);
    assert!(wire.iter().all(|r| r.join == Some(Join::Matched)));
    let mut threads: Vec<_> = wire.iter().filter_map(|r| r.thread.as_deref()).collect();
    threads.sort_unstable();
    // Sorted: `<session>/agent-0` comes before `<session>/main`.
    assert_eq!(threads, [KID, MAIN]);
    let native = rows(&t, Leg::Native);
    assert!(native.iter().all(|r| r.join == Some(Join::Matched)));
}

#[test]
fn the_check_sees_a_thread_with_a_parent() {
    // With no nested spawn: a child with no file and no call is a missing subagent.
    let (_, v) = check(&entry("kw-check", false), &[]);
    assert_eq!(result(&v, "subagents_present"), "pass");
}

#[test]
fn a_long_call_joins_its_own_wire_call_by_its_end() {
    // The main call opens at 10 s and ends at 40 s; agent-0's call runs from 20 s to 22 s in
    // between. Each wire reply ends just before its `usage.record`, so only the end time pairs
    // each native call with its own wire call.
    let mut e = Entry::new("kw-long");
    e.idle(21_900);
    chat(&mut e);
    e.idle(17_900);
    chat(&mut e);
    let main = json!({"inputOther": 11, "output": 5});
    file(
        &mut e,
        &path("main"),
        &call_until("main", ms(10), ms(40), main),
    );
    let kid = json!({"inputOther": 7, "output": 3});
    file(
        &mut e,
        &path("agent-0"),
        &call_until("agent-0", ms(20), ms(22), kid),
    );
    e.count();
    let dir = e.seal();
    let set = |v: &mut Value| v["connector"] = "kimi-code-cli".into();
    let t = call_table(&mutate(&dir, None, |r| edit_event(r, "start", 0, set)));
    let mut wire = rows(&t, Leg::Wire);
    assert_eq!(wire.len(), 2);
    assert!(wire.iter().all(|r| r.join == Some(Join::Matched)));
    wire.sort_by_key(|r| r.wall_end);
    let threads: Vec<_> = wire.iter().map(|r| r.thread.as_deref()).collect();
    assert_eq!(threads, [Some(KID), Some(MAIN)]);
    assert_eq!(wire[0].parent.as_deref(), Some(MAIN));
    let native = rows(&t, Leg::Native);
    assert!(native.iter().all(|r| r.join == Some(Join::Matched)));
    let main = native
        .iter()
        .find(|r| r.thread.as_deref() == Some(MAIN))
        .expect("main call");
    // Opened at the `llm.request`, ended at the `usage.record`.
    let got = (main.wall_start, main.wall_end);
    assert_eq!(got, (Some(ms(10) * 1_000_000), Some(ms(40) * 1_000_000)));
}
