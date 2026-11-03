//! PRD-09 point 2, wire ids (subagent design item A): a connector whose requests name their
//! calling thread and its parent in headers (or Codex's WebSocket `client_metadata`) gets its
//! threads, parent links and children from the wire alone, with no native reader. A connector
//! with no wire ids keeps no reader and no threads. Every value is an obvious fake.

mod common;

use common::{Entry, call_table, check, edit_event, item, mutate, result};
use goalcup_format::{EndReason, FLAG_FIN, StreamKind};
use goalcup_verify::usage::{CallRow, CallTable, Join, Kind, Leg, Outcome, Reason, Thread};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;

const MAIN: &str = "ses_fake0000main";
const KID: &str = "ses_fake0000kid1";

/// One `POST /chat/completions` with these request headers, answered whole and ok.
fn chat(e: &mut Entry, headers: &[(&str, &str)]) {
    let c = e.conn();
    let mut hs = vec![json!(["content-type", "application/json"])];
    hs.extend(headers.iter().map(|(k, v)| json!([k, v])));
    let head = json!({"method": "POST", "path": "/v1/chat/completions", "ver": "1.1",
        "headers": hs, "masked": [], "coding": null});
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

/// A sealed entry of this connector, its calls written by `calls`.
fn entry(name: &str, connector: &str, calls: impl FnOnce(&mut Entry)) -> PathBuf {
    let mut e = Entry::new(name);
    calls(&mut e);
    e.count();
    let dir = e.seal();
    let set = |v: &mut Value| v["connector"] = connector.into();
    mutate(&dir, None, |r| edit_event(r, "start", 0, set))
}

fn thread<'a>(t: &'a CallTable, id: &str) -> &'a Thread {
    t.threads.iter().find(|x| x.id == id).expect("thread")
}

fn calls(t: &CallTable) -> Vec<&CallRow> {
    let wire = |r: &&CallRow| r.leg == Leg::Wire && r.kind == Kind::Call;
    t.rows.iter().filter(wire).collect()
}

/// The children map of one parent with one child.
fn kids(parent: &str, kid: &str) -> Option<BTreeMap<String, Vec<String>>> {
    Some(BTreeMap::from([(
        parent.to_string(),
        vec![kid.to_string()],
    )]))
}

#[test]
fn opencode_threads_and_parent_come_from_the_wire() {
    let dir = entry("wire-ids-opencode", "opencode-cli", |e| {
        chat(e, &[("x-opencode-session-id", MAIN)]);
        let kid = [
            ("x-opencode-session-id", KID),
            ("x-opencode-parent-session-id", MAIN),
        ];
        chat(e, &kid);
    });
    let t = call_table(&dir);
    assert_eq!(t.threads.len(), 2);
    assert_eq!(thread(&t, KID).parent.as_deref(), Some(MAIN));
    assert_eq!(thread(&t, MAIN).parent, None);
    assert_eq!(t.children, kids(MAIN, KID));
    let rows = calls(&t);
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.join == Some(Join::WireOnly)));
    assert_eq!(rows[1].thread.as_deref(), Some(KID));
    assert_eq!(rows[1].parent.as_deref(), Some(MAIN));
    let unattributed = rows.iter().filter(|r| r.join == Some(Join::Unattributed));
    assert_eq!(unattributed.count(), 0);
    // No native reader: the participant is told there is none, not that it could not read.
    assert_eq!(t.reason, Some(Reason::NoReader));
    assert!(t.threads.iter().all(|x| x.reason == Some(Reason::NoReader)));
}

#[test]
fn a_parent_only_the_wire_names_is_a_parent_not_a_thread() {
    let dir = entry("wire-ids-kilo", "kilo-cli", |e| {
        chat(e, &[("x-session-id", KID), ("x-parent-session-id", MAIN)]);
    });
    let t = call_table(&dir);
    // The parent's calls may sit in another entry: here it has none, so it is no thread, and
    // the children map still names it.
    assert_eq!(t.threads.len(), 1);
    assert!(t.threads.iter().all(|x| x.id != MAIN));
    assert_eq!(thread(&t, KID).parent.as_deref(), Some(MAIN));
    assert_eq!(t.children, kids(MAIN, KID));
}

#[test]
fn a_connector_with_no_wire_ids_reads_no_threads() {
    let dir = entry("wire-ids-none", "cline-cli", |e| {
        chat(e, &[("x-session-id", KID), ("x-parent-session-id", MAIN)]);
    });
    let t = call_table(&dir);
    assert_eq!(t.reason, Some(Reason::NoReader));
    assert!(t.threads.is_empty());
    assert_eq!(t.children, None);
    let rows = calls(&t);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].thread.as_ref(), rows[0].parent.as_ref()),
        (None, None)
    );
    assert_eq!(rows[0].join, Some(Join::WireOnly));
}

#[test]
fn codex_websocket_client_metadata_names_the_parent() {
    let dir = entry("wire-ids-codex-ws", "codex-cli", |e| {
        let c = e.conn();
        let head = json!({"method": "GET", "path": "/v1/responses", "ver": "1.1",
            "headers": [["upgrade", "websocket"]], "masked": [], "coding": null});
        let q = e.open(StreamKind::HttpReq, c, 0, head);
        e.end(q, EndReason::Complete, 0, Value::Null);
        let up = e.open(StreamKind::WsUp, c, q, json!({"deflate": false}));
        let down = e.open(StreamKind::WsDown, c, q, json!({"deflate": false}));
        let meta = json!({"thread_id": KID, "x-codex-parent-thread-id": MAIN});
        let msg = json!({"type": "response.create", "model": "fake-model", "input": "hi",
            "client_metadata": meta})
        .to_string();
        e.data(up, 0, 0x10 | FLAG_FIN, msg.as_bytes());
        let done = br#"{"type":"response.completed","response":{}}"#;
        e.data(down, 0, 0x10 | FLAG_FIN, done);
        e.end(
            up,
            EndReason::Complete,
            msg.len() as u64,
            json!({"close_code": 1000}),
        );
        e.end(
            down,
            EndReason::Complete,
            done.len() as u64,
            json!({"close_code": 1000}),
        );
        e.end(c, EndReason::Complete, 0, json!({"up": 400, "down": 300}));
    });
    let t = call_table(&dir);
    let rows = calls(&t);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].thread.as_deref(), Some(KID));
    assert_eq!(rows[0].parent.as_deref(), Some(MAIN));
    assert_eq!(thread(&t, KID).parent.as_deref(), Some(MAIN));
    assert_eq!(t.children, kids(MAIN, KID));
    // Codex has a native reader: with no rollout it reads none, which is not "no reader".
    assert_eq!(t.reason, Some(Reason::NoNative));
}

const AMP: &str = "T-fake0000-main";
const TASK1: &str = "TU-fake0000task1";
const TASK2: &str = "TU-fake0000task2";
const TASK3: &str = "TU-fake0000task3";

/// An Amp thread socket: a replayed old turn, a user turn, the main turn sent in progress then
/// complete with its `Task` calls, three subagent turns (one with two tool calls), the same main
/// turn sent again, and a last main turn. `idle` adds a `Task` whose subagent leased no tool.
fn amp(name: &str, idle: bool) -> PathBuf {
    entry(name, "amp-cli", |e| {
        let c = e.conn();
        let head = json!({"method": "GET", "path": "/actors/gateway/threadActor/websocket/",
            "ver": "1.1", "headers": [["upgrade", "websocket"]], "masked": [], "coding": null});
        let q = e.open(StreamKind::HttpReq, c, 0, head);
        e.end(q, EndReason::Complete, 0, Value::Null);
        let up = e.open(StreamKind::WsUp, c, q, json!({"deflate": false}));
        let down = e.open(StreamKind::WsDown, c, q, json!({"deflate": false}));
        let mut tasks = vec![TASK1, TASK2];
        tasks.extend(idle.then_some(TASK3));
        let added = |id: &str, role: &str, out: u64, state: &str, at: Option<&str>| {
            let done = state == "complete" && role == "assistant";
            let usage = json!({"model": "fake-model", "inputTokens": 2, "outputTokens": out,
                "cacheReadInputTokens": 100, "cacheCreationInputTokens": 10});
            let calls: Vec<_> = (id == "M-fake-main1")
                .then_some(&tasks)
                .into_iter()
                .flatten()
                .map(|t| json!({"type": "tool_use", "name": "Task", "id": t}))
                .collect();
            json!({"jsonrpc": "2.0", "method": "message_added", "params": {"message": {
                "threadId": AMP, "role": role, "messageId": id, "state": {"type": state},
                "createdAt": at, "content": calls, "usage": done.then_some(usage)}}})
        };
        let lease = |task: &str, id: &str| {
            json!({"jsonrpc": "2.0", "method": "tool_lease", "params": {"toolCallId": "TU-fake",
                "parentToolCallId": task, "messageId": id}})
        };
        let frames = [
            added(
                "M-fake-old",
                "assistant",
                1000,
                "complete",
                Some("2020-01-01T00:00:00Z"),
            ),
            added("M-fake-user", "user", 0, "complete", None),
            added("M-fake-main1", "assistant", 30, "streaming", None),
            added("M-fake-main1", "assistant", 30, "complete", None),
            lease(TASK1, "M-fake-kid1a"),
            lease(TASK1, "M-fake-kid1a"),
            lease(TASK2, "M-fake-kid2a"),
            lease(TASK1, "M-fake-kid1b"),
            added("M-fake-main1", "assistant", 30, "complete", None),
            added("M-fake-main2", "assistant", 5, "complete", None),
        ];
        let mut off = 0;
        for f in frames.map(|f| f.to_string()) {
            e.data(down, off, 0x10 | FLAG_FIN, f.as_bytes());
            off += f.len() as u64;
        }
        e.end(up, EndReason::Complete, 0, json!({"close_code": 1000}));
        e.end(down, EndReason::Complete, off, json!({"close_code": 1000}));
        e.end(c, EndReason::Complete, 0, json!({"up": 300, "down": 3000}));
    })
}

#[test]
fn amp_thread_socket_links_subagents_and_counts_usage() {
    let dir = amp("wire-ids-amp", false);
    assert_eq!(result(&check(&dir, &[]).1, "subagents_present"), "pass");
    let t = call_table(&dir);
    let kids = BTreeMap::from([(AMP.to_string(), vec![TASK1.into(), TASK2.into()])]);
    assert_eq!(t.children, Some(kids));
    let rows = calls(&t);
    let of = |id: &str| {
        rows.iter()
            .filter(|r| r.thread.as_deref() == Some(id))
            .count()
    };
    assert_eq!((of(AMP), of(TASK1), of(TASK2)), (2, 2, 1));
    let kid = |r: &&&CallRow| r.thread.as_deref() != Some(AMP);
    assert!(
        rows.iter()
            .filter(kid)
            .all(|r| r.parent.as_deref() == Some(AMP))
    );
    // Each main turn's usage is counted once, the replayed turn not; a subagent turn says none.
    let said: Vec<_> = rows
        .iter()
        .filter(|r| r.usage_from == Some(Leg::Wire))
        .collect();
    assert_eq!(said.len(), 2);
    assert!(
        said.iter()
            .all(|r| r.model.as_deref() == Some("fake-model"))
    );
    let sum = |f: fn(&CallRow) -> Option<u64>| said.iter().filter_map(|r| f(r)).sum::<u64>();
    assert_eq!(sum(|r| r.tokens.output), 35);
    assert_eq!(sum(|r| r.tokens.cached), 200);
    assert_eq!(sum(|r| r.tokens.cache_write), 20);
    assert_eq!(sum(|r| r.tokens.input), 4);
    assert!(rows.iter().all(|r| r.outcome == Outcome::Ok && r.whole));
}

#[test]
fn amp_task_with_no_lease_is_a_missing_subagent() {
    let dir = amp("wire-ids-amp-idle", true);
    let kids: Vec<String> = vec![TASK1.into(), TASK2.into(), TASK3.into()];
    assert_eq!(call_table(&dir).children.expect("read")[AMP], kids);
    let v = check(&dir, &[]).1;
    assert_ne!(result(&v, "subagents_present"), "pass");
    assert_eq!(
        item(&v, "subagents_present")["facts"]["missing"],
        json!([TASK3])
    );
}
