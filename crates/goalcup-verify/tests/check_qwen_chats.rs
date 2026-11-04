//! PRD-09 point 2, Qwen Code chats (subagent design item C): a main session file at
//! `chats/<session id>.jsonl` is a thread of depth 0, a subagent's file at
//! `subagents/<session id>/agent-<id>.jsonl` is a child of that session (depth 1), and each
//! assistant row with `usageMetadata` is one native call joined to the wire by time. The wire
//! carries no thread, so the threads come from these files alone. Every value is an obvious fake.

mod common;

use common::{Entry, call_table, check, edit_event, mutate, result};
use goalcup_format::{EndReason, StreamKind};
use goalcup_verify::usage::{CallRow, CallTable, Join, Kind, Leg, Reason, Thread};
use serde_json::{Value, json};
use std::path::PathBuf;

const MAIN: &str = "f00dfeed-0000-4000-8000-000000000001";
const KID: &str = "general-purpose-call_00_fake0000kid1";
const MAIN_FILE: &str = "chats:-fake-project/chats/f00dfeed-0000-4000-8000-000000000001.jsonl";

fn kid_file() -> String {
    format!("chats:-fake-project/subagents/{MAIN}/agent-{KID}.jsonl")
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

/// One native stream holding `body` as the file at `path`, ended complete.
fn file(e: &mut Entry, path: &str, body: &str) {
    let s = e.open(
        StreamKind::Native,
        0,
        0,
        json!({"path": path, "gen": 0, "from": 0}),
    );
    e.data(s, 0, 0, body.as_bytes());
    e.end(s, EndReason::Complete, body.len() as u64, Value::Null);
}

/// A user row, an assistant row with no usage, and one assistant call timed `at` (seconds after
/// the test clock's start) with made-up counts.
fn chat_file(session: &str, at: u64, prompt: u64, cached: u64) -> String {
    let t = format!("2026-11-07T00:00:{at:02}.000Z");
    let rows = [
        json!({"uuid": "u-1", "sessionId": session, "timestamp": t, "type": "user"}),
        json!({"uuid": "u-2", "sessionId": session, "timestamp": t, "type": "assistant",
            "model": "fake-model"}),
        json!({"uuid": "u-3", "sessionId": session, "timestamp": t, "type": "assistant",
            "model": "fake-model", "usageMetadata": {"promptTokenCount": prompt,
            "cachedContentTokenCount": cached, "candidatesTokenCount": 40,
            "thoughtsTokenCount": 7, "totalTokenCount": prompt + 40}}),
    ];
    rows.iter().map(|r| format!("{r}\n")).collect()
}

/// A sealed Qwen Code entry with two wire calls and a main and a subagent chat file.
fn entry(name: &str) -> PathBuf {
    entry_with(name, "", |_| {})
}

/// [`entry`] with `tail` appended to the main chat file and `f` applied to the start event.
fn entry_with(name: &str, tail: &str, f: impl FnOnce(&mut Value)) -> PathBuf {
    let mut e = Entry::new(name);
    chat(&mut e);
    chat(&mut e);
    // The subagent row carries the parent's session id, as Qwen writes it.
    let main = chat_file(MAIN, 1, 1200, 1000) + tail;
    file(&mut e, MAIN_FILE, &main);
    file(&mut e, &kid_file(), &chat_file(MAIN, 2, 900, 500));
    e.count();
    let dir = e.seal();
    let set = |v: &mut Value| {
        v["connector"] = "qwen-cli".into();
        f(v);
    };
    mutate(&dir, None, |r| edit_event(r, "start", 0, set))
}

fn thread<'a>(t: &'a CallTable, id: &str) -> &'a Thread {
    t.threads
        .iter()
        .find(|x| x.id == id)
        .unwrap_or_else(|| panic!("no thread {id}"))
}

fn rows(t: &CallTable, leg: Leg) -> Vec<&CallRow> {
    let of = |r: &&CallRow| r.leg == leg && r.kind == Kind::Call;
    t.rows.iter().filter(of).collect()
}

#[test]
fn a_subagent_file_gives_its_parent_and_depth() {
    let t = call_table(&entry("qc-a"));
    assert_eq!(t.reason, None);
    let (main, kid) = (thread(&t, MAIN), thread(&t, KID));
    assert_eq!((main.parent.as_deref(), main.depth), (None, Some(0)));
    assert_eq!((kid.parent.as_deref(), kid.depth), (Some(MAIN), Some(1)));
    assert_eq!(
        t.children.as_ref().expect("parentage was read")[MAIN],
        [KID]
    );
    assert!(t.threads.iter().all(|x| x.reason != Some(Reason::NoReader)));
}

#[test]
fn each_assistant_row_with_usage_is_one_call_with_the_cached_part_split_out() {
    let t = call_table(&entry("qc-b"));
    let nat = rows(&t, Leg::Native);
    assert_eq!(nat.len(), 2);
    let kid = nat
        .iter()
        .find(|r| r.thread.as_deref() == Some(KID))
        .expect("kid row");
    let k = &kid.tokens;
    let got = (k.input, k.cached, k.output, k.reasoning, k.cache_write);
    assert_eq!(got, (Some(400), Some(500), Some(40), Some(7), None));
    assert_eq!(
        (kid.parent.as_deref(), kid.model.as_deref()),
        (Some(MAIN), Some("fake-model"))
    );
    assert!(nat.iter().all(|r| r.join == Some(Join::Matched)));
}

#[test]
fn wire_calls_take_their_thread_by_time() {
    let t = call_table(&entry("qc-c"));
    let wire = rows(&t, Leg::Wire);
    assert_eq!(wire.len(), 2);
    assert!(wire.iter().all(|r| r.join == Some(Join::Matched)));
    let ids: Vec<_> = wire
        .iter()
        .map(|r| (r.thread.as_deref(), r.parent.as_deref()))
        .collect();
    assert!(ids.contains(&(Some(MAIN), None)));
    assert!(ids.contains(&(Some(KID), Some(MAIN))));
}

#[test]
fn the_check_sees_a_thread_with_a_parent() {
    let (_, v) = check(&entry("qc-check"), &[]);
    assert_eq!(result(&v, "subagents_present"), "pass");
}

#[test]
fn a_qwen_record_names_build_rules_and_unread_lines() {
    let t = call_table(&entry_with("qc-v", "not json\n\n  \n", |v| {
        v["harness_semver"] = "0.27.0".into();
        v["version_row"] = Value::Null;
    }));
    assert_eq!(t.harness_semver.as_deref(), Some("0.27.0"));
    assert_eq!(t.version_row, None);
    assert_eq!(t.native_checked_on.as_deref(), Some("0.26.4"));
    assert_eq!(t.lines_unread, 1);
    let base = call_table(&entry("qc-v0"));
    let calls = |t: &CallTable| -> Vec<_> {
        let r = rows(t, Leg::Native);
        r.iter()
            .map(|r| (r.thread.clone(), r.parent.clone(), r.tokens, r.join))
            .collect()
    };
    assert_eq!(calls(&t), calls(&base));
    assert_eq!((base.harness_semver, base.version_row), (None, None));
    assert_eq!(base.lines_unread, 0);
}
