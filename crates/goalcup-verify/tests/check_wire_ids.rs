//! PRD-09 point 2, wire ids (subagent design item A): a connector whose requests name their
//! calling thread and its parent in headers (or Codex's WebSocket `client_metadata`) gets its
//! threads, parent links and children from the wire alone, with no native reader. A connector
//! with no wire ids keeps no reader and no threads. Every value is an obvious fake.

mod common;

use common::{Entry, call_table, edit_event, mutate};
use goalcup_format::{EndReason, FLAG_FIN, StreamKind};
use goalcup_verify::usage::{CallRow, CallTable, Join, Kind, Leg, Reason, Thread};
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
    let dir = entry("wire-ids-none", "amp-cli", |e| {
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
