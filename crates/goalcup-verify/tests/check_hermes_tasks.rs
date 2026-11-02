//! PRD-08's Hermes subagent case (ledger B10): with no state.db, a subagent is a delegation task
//! log (`delegation:<delegation>/task-<n>.log`, plain text) and the manifest snapshot names the
//! tasks that should exist (Hermes `tools/delegation_live_log.py`). Wire calls stay the call
//! source. Every value is an obvious fake.

mod common;

use common::{Entry, call_table, check, edit_event, gemini_wire, mutate, result};
use goalcup_format::{EndReason, StreamKind};
use goalcup_verify::legs::judge;
use goalcup_verify::usage::Reason;
use serde_json::{Value, json};
use std::path::PathBuf;

const D: &str = "deleg_aaaaaaaa";

fn task_path(n: u64) -> String {
    format!("delegation:{D}/task-{n}.log")
}

/// A task log stream opened again in a later boot: no bytes, ended complete.
fn reopened(e: &mut Entry, path: &str) {
    let meta = json!({"path": path, "gen": 0, "from": 0});
    let s = e.open(StreamKind::Native, 0, 0, meta);
    e.end(s, EndReason::Complete, 0, Value::Null);
}

/// The manifest snapshot naming `count` tasks.
fn manifest(e: &mut Entry, count: u64) {
    let path = format!("delegation-manifest:{D}/manifest.json");
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

fn hermes(v: &mut Value) {
    v["connector"] = "hermes-cli".into();
}

/// A sealed Hermes entry with two fake wire calls and what `f` adds.
fn entry(name: &str, f: impl FnOnce(&mut Entry)) -> PathBuf {
    let mut e = Entry::new(name);
    gemini_wire(&mut e, "fake-her-a");
    gemini_wire(&mut e, "fake-her-b");
    f(&mut e);
    e.count();
    mutate(&e.seal(), None, |r| edit_event(r, "start", 0, hermes))
}

fn task_ids(n: u64) -> Vec<String> {
    (0..n).map(|i| format!("{D}/task-{i}")).collect()
}

#[test]
fn manifest_and_two_logs_give_two_children() {
    let d = entry("ht-a", |e| {
        manifest(e, 2);
        for n in 0..2 {
            e.native(&task_path(n), 0, 0);
        }
    });
    let t = call_table(&d);
    for id in task_ids(2) {
        let th = t.threads.iter().find(|x| x.id == id).expect("task thread");
        assert_eq!((th.parent.as_deref(), th.depth), (Some(D), Some(1)));
        assert!(th.readable());
    }
    assert_eq!(t.children.as_ref().expect("read")[D], task_ids(2));
    let (_, v) = check(&d, &[]);
    assert_eq!(result(&v, "subagents_present"), "pass");
    assert_eq!(result(&v, "threads_match"), "skip");
    assert_eq!(result(&v, "wire_keeps_up"), "skip");
}

#[test]
fn a_task_the_manifest_names_with_no_log_is_not_a_pass() {
    let d = entry("ht-b", |e| {
        manifest(e, 3);
        for n in 0..2 {
            e.native(&task_path(n), 0, 0);
        }
    });
    let t = call_table(&d);
    assert_eq!(t.children.as_ref().expect("read")[D].len(), 3);
    let (_, v) = check(&d, &[]);
    assert_ne!(result(&v, "subagents_present"), "pass");
}

#[test]
fn logs_with_no_manifest_still_give_children() {
    let d = entry("ht-c", |e| {
        for n in 0..2 {
            e.native(&task_path(n), 0, 0);
        }
    });
    let t = call_table(&d);
    assert_eq!(t.children.as_ref().expect("read")[D], task_ids(2));
    let (_, v) = check(&d, &[]);
    assert_eq!(result(&v, "subagents_present"), "pass");
}

/// One delegation of five tasks, each log reopened with no bytes in later boots: still five.
#[test]
fn five_tasks_reopened_across_boots_stay_five() {
    let d = entry("ht-d", |e| {
        manifest(e, 5);
        for n in 0..5 {
            e.native(&task_path(n), 0, 0);
        }
        for _ in 0..2 {
            (0..5).for_each(|n| reopened(e, &task_path(n)));
        }
    });
    let t = call_table(&d);
    assert_eq!(t.children.as_ref().expect("read")[D], task_ids(5));
    assert_eq!(t.children.as_ref().unwrap().values().flatten().count(), 5);
    let (_, v) = check(&d, &[]);
    assert_eq!(result(&v, "subagents_present"), "pass");
}

/// A dodger row: the model host is pinned, so its one connection is a tunnel and no wire call is
/// read; the task logs give threads but no calls. The harness log is not readable per call.
#[test]
fn task_logs_with_no_calls_and_no_wire_call_are_not_readable() {
    let mut e = Entry::new("ht-e");
    e.event(json!({"ev": "launch", "pid": 4242, "program": "fake-harness"}));
    let tunnel = json!({"host": "pinned.example.test", "port": 443, "mode": "tunnel",
        "listed": "shared", "kind": "provider", "reason": "pinned", "alpn": null, "tls": null,
        "fingerprint": "0123456789abcdef"});
    e.open(StreamKind::Conn, 0, 0, tunnel);
    for n in 0..2 {
        e.native(&task_path(n), 0, 0);
    }
    e.count();
    let d = mutate(&e.seal(), None, |r| edit_event(r, "start", 0, hermes));
    let t = call_table(&d);
    assert!(t.threads.iter().any(|x| x.id == task_ids(2)[1]), "task threads");
    assert_eq!(t.not_on_wire.len(), 1);
    assert_eq!(t.reason, Some(Reason::NoNative));
    assert_eq!(judge(&t, &[]).1.harness_log, "none");
}
