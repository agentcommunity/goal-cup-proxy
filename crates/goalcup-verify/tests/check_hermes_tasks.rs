//! PRD-08's Hermes subagent case (ledger B10): with no state.db, a subagent is a delegation task
//! log (`delegation:<...>/<delegation>/task-<n>.log`, plain text) and the manifest snapshot names
//! the tasks that should exist (Hermes `tools/delegation_live_log.py`). Wire calls stay the call
//! source. Every value is an obvious fake.

mod common;

use common::{Entry, check, edit_event, mutate, read_records, result};
use goalcup_format::{EndReason, GapReason, StreamKind};
use goalcup_verify::usage::{CallTable, Calls};
use serde_json::{Value, json};
use std::path::PathBuf;

const D: &str = "deleg_aaaaaaaa";

fn task_path(n: u64) -> String {
    format!("delegation:cache/delegation/live/{D}/task-{n}.log")
}

/// One native stream holding a plain-text log as the file at `path`, ended complete.
fn log(e: &mut Entry, path: &str) {
    let s = e.open(
        StreamKind::Native,
        0,
        0,
        json!({"path": path, "gen": 0, "from": 0}),
    );
    let body = b"=== fake transcript ===\nfake line without newline";
    e.data(s, 0, 0, body);
    e.end(s, EndReason::Complete, body.len() as u64, Value::Null);
}

/// A log with a plain-text body and a known hole in the middle.
fn holed_log(e: &mut Entry, path: &str) {
    let s = e.open(
        StreamKind::Native,
        0,
        0,
        json!({"path": path, "gen": 0, "from": 0}),
    );
    e.data(s, 0, 0, b"fake head\n");
    e.gap(s, GapReason::MemoryGuard, 0, 10, 5, Value::Null);
    e.data(s, 15, 0, b"fake tail\n");
    e.end(s, EndReason::Complete, 25, Value::Null);
}

/// The manifest snapshot naming `count` tasks.
fn manifest(e: &mut Entry, count: u64) {
    let path = format!("delegation-manifest:cache/delegation/live/{D}/manifest.json");
    let s = e.open(
        StreamKind::Snapshot,
        0,
        0,
        json!({"path": path, "gen": 0, "from": 0}),
    );
    let body = json!({"delegation_id": D, "task_count": count}).to_string();
    e.data(s, 0, 0, body.as_bytes());
    e.end(s, EndReason::Complete, body.len() as u64, Value::Null);
}

/// A Gemini `generateContent` call on its own connection.
fn wire(e: &mut Entry, model: &str) {
    let c = e.conn();
    let path = format!("/v1beta/models/{model}:generateContent");
    let head = json!({"method": "POST", "path": path, "ver": "1.1", "masked": [], "coding": null,
        "headers": [["content-type", "application/json"]]});
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    e.data(q, 0, 0, b"{}");
    e.end(q, EndReason::Complete, 2, Value::Null);
    let body = json!({"modelVersion": model, "responseId": format!("resp-{model}"),
        "candidates": [{"finishReason": "STOP"}],
        "usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 3}})
    .to_string();
    let head = json!({"status": 200, "ver": "1.1", "decoded": true, "masked": [], "coding": null,
        "headers": [["content-type", "application/json"]]});
    let p = e.open(StreamKind::HttpResp, c, q, head);
    e.data(p, 0, 0, body.as_bytes());
    e.end(p, EndReason::Complete, body.len() as u64, Value::Null);
    e.end(c, EndReason::Complete, 0, json!({"up": 90, "down": 90}));
}

fn hermes(v: &mut Value) {
    v["connector"] = "hermes-cli".into();
}

/// A sealed Hermes entry with two fake wire calls and what `f` adds.
fn entry(name: &str, f: impl FnOnce(&mut Entry)) -> PathBuf {
    let mut e = Entry::new(name);
    wire(&mut e, "fake-her-a");
    wire(&mut e, "fake-her-b");
    f(&mut e);
    e.count();
    mutate(&e.seal(), None, |r| edit_event(r, "start", 0, hermes))
}

fn table(dir: &std::path::Path) -> CallTable {
    let mut calls = Calls::new();
    read_records(&dir.join("rec"))
        .iter()
        .for_each(|r| calls.push(r));
    calls.finish()
}

fn task_ids() -> [String; 2] {
    [format!("{D}/task-0"), format!("{D}/task-1")]
}

#[test]
fn manifest_and_two_logs_give_two_children() {
    let d = entry("ht-a", |e| {
        manifest(e, 2);
        log(e, &task_path(0));
        log(e, &task_path(1));
    });
    let t = table(&d);
    for id in task_ids() {
        let th = t.threads.iter().find(|x| x.id == id).expect("task thread");
        assert_eq!((th.parent.as_deref(), th.depth), (Some(D), Some(1)));
        assert!(th.readable());
    }
    assert_eq!(t.children.as_ref().expect("read")[D], task_ids());
    let (_, v) = check(&d, &[]);
    assert_eq!(result(&v, "subagents_present"), "pass");
    assert_eq!(result(&v, "threads_match"), "skip");
    assert_eq!(result(&v, "wire_keeps_up"), "skip");
}

#[test]
fn a_task_the_manifest_names_with_no_log_is_not_a_pass() {
    let d = entry("ht-b", |e| {
        manifest(e, 3);
        log(e, &task_path(0));
        log(e, &task_path(1));
    });
    let t = table(&d);
    assert_eq!(t.children.as_ref().expect("read")[D].len(), 3);
    let (_, v) = check(&d, &[]);
    assert_ne!(result(&v, "subagents_present"), "pass");
}

#[test]
fn logs_with_no_manifest_still_give_children() {
    let d = entry("ht-c", |e| {
        log(e, &task_path(0));
        log(e, &task_path(1));
    });
    let t = table(&d);
    assert_eq!(t.children.as_ref().expect("read")[D], task_ids());
    let (_, v) = check(&d, &[]);
    assert_eq!(result(&v, "subagents_present"), "pass");
}

#[test]
fn a_hole_in_a_task_log_keeps_the_thread() {
    let d = entry("ht-d", |e| holed_log(e, &task_path(0)));
    let t = table(&d);
    let th = t.threads.iter().find(|x| x.id == task_ids()[0]);
    let th = th.expect("task thread");
    assert_eq!(th.parent.as_deref(), Some(D));
    assert!(th.readable());
}
