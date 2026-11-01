//! PRD-08's Gemini CLI parentage case (ledger B10): a subagent's session file lives at
//! `chats/<parent session id>/<subagent session id>.jsonl` (docs/harnesses/gemini-cli.md section
//! 4), and the native reader gives the subagent that parent through seam U1. Wire calls stay the
//! call source (points 1 and 2). Every value is an obvious fake.

mod common;

use common::{Entry, check, edit_event, mutate, read_records, result};
use goalcup_format::{EndReason, GapReason, Record, StreamKind};
use goalcup_verify::usage::{CallTable, Calls, Join, Kind, Leg};
use serde_json::{Value, json};
use std::path::PathBuf;

/// The fake main session id; its file name carries the first 8 characters.
const MAIN: &str = "d56586ab-0000-4000-8000-000000000001";
const KID: &str = "5ab0c0de-0000-4000-8000-000000000002";
const MAIN_FILE: &str = "chats:proj/chats/session-2026-11-01T08-42-d56586ab.jsonl";

fn kid_file() -> String {
    format!("chats:proj/chats/{MAIN}/{KID}.jsonl")
}

/// One native stream holding `body` as the file at `path`, ended complete.
fn file(e: &mut Entry, path: &str, body: &str) {
    let s = e.open(StreamKind::Native, 0, 0, json!({"path": path, "gen": 0, "from": 0}));
    e.data(s, 0, 0, body.as_bytes());
    e.end(s, EndReason::Complete, body.len() as u64, Value::Null);
}

/// A native stream whose bytes have a known hole after the first line.
fn holed_file(e: &mut Entry, path: &str, first: &str) {
    let s = e.open(StreamKind::Native, 0, 0, json!({"path": path, "gen": 0, "from": 0}));
    let n = first.len() as u64;
    e.data(s, 0, 0, first.as_bytes());
    e.gap(s, GapReason::MemoryGuard, 0, n, 7, Value::Null);
    e.data(s, n + 7, 0, b"{\"x\":1}\n");
    e.end(s, EndReason::Complete, n + 15, Value::Null);
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

fn gemini(v: &mut Value) {
    v["connector"] = "gemini-cli".into();
}

/// A sealed Gemini entry with two fake wire calls and these session files (path, body).
fn entry(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let mut e = Entry::new(name);
    wire(&mut e, "fake-gem-a");
    wire(&mut e, "fake-gem-b");
    for (p, b) in files {
        file(&mut e, p, b);
    }
    e.count();
    as_gemini(e.seal())
}

fn as_gemini(dir: PathBuf) -> PathBuf {
    mutate(&dir, None, |r| edit_event(r, "start", 0, gemini))
}

fn table(dir: &std::path::Path) -> CallTable {
    let recs: Vec<Record> = read_records(&dir.join("rec"));
    let mut calls = Calls::new();
    recs.iter().for_each(|r| calls.push(r));
    calls.finish()
}

fn main_line() -> String {
    format!("{}\n", json!({"sessionId": MAIN, "kind": "main"}))
}

fn kid_line() -> String {
    format!("{}\n", json!({"sessionId": KID, "kind": "subagent"}))
}

fn thread<'a>(t: &'a CallTable, id: &str) -> &'a goalcup_verify::usage::Thread {
    t.threads.iter().find(|x| x.id == id).unwrap_or_else(|| panic!("no thread {id}"))
}

#[test]
fn a_subagent_file_gives_its_parent_and_depth() {
    let t = table(&entry("gp-a", &[(MAIN_FILE, &main_line()), (&kid_file(), &kid_line())]));
    let (main, kid) = (thread(&t, MAIN), thread(&t, KID));
    assert_eq!((main.parent.as_deref(), main.depth), (None, Some(0)));
    assert_eq!((kid.parent.as_deref(), kid.depth), (Some(MAIN), Some(1)));
    let kids = t.children.as_ref().expect("parentage was read");
    assert_eq!(kids[MAIN], [KID]);
}

#[test]
fn a_main_file_with_no_id_still_gives_the_subagent_its_parent() {
    let d = entry("gp-b", &[(MAIN_FILE, "not json\n"), (&kid_file(), &kid_line())]);
    let t = table(&d);
    let kid = thread(&t, KID);
    // The folder id is matched to the main file by its 8-hex name suffix, so the child links to
    // that file's thread (named by its stem when the file holds no id), not a dangling id.
    let stem = "session-2026-11-01T08-42-d56586ab";
    assert_eq!((kid.parent.as_deref(), kid.depth), (Some(stem), Some(1)));
    assert!(t.threads.iter().any(|x| x.id == stem));
}

#[test]
fn a_hole_in_the_subagent_file_keeps_the_thread_and_its_parent() {
    let mut e = Entry::new("gp-c");
    wire(&mut e, "fake-gem-a");
    file(&mut e, MAIN_FILE, &main_line());
    holed_file(&mut e, &kid_file(), &kid_line());
    e.count();
    let t = table(&as_gemini(e.seal()));
    let kid = thread(&t, KID);
    assert_eq!((kid.parent.as_deref(), kid.depth), (Some(MAIN), Some(1)));
}

#[test]
fn wire_calls_stay_the_call_source() {
    let wire_only = table(&entry("gp-d1", &[]));
    let with_chats = table(&entry(
        "gp-d2",
        &[(MAIN_FILE, &main_line()), (&kid_file(), &kid_line())],
    ));
    for t in [&wire_only, &with_chats] {
        let calls: Vec<_> = t.rows.iter().filter(|r| r.kind == Kind::Call).collect();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|r| r.join == Some(Join::WireOnly)));
        assert!(t.rows.iter().all(|r| r.leg == Leg::Wire));
    }
    assert_eq!(wire_only.rows.len(), with_chats.rows.len());
}

#[test]
fn the_check_sees_a_thread_with_a_parent() {
    let d = entry("gp-check", &[(MAIN_FILE, &main_line()), (&kid_file(), &kid_line())]);
    let (_, v) = check(&d, &[]);
    // Every child here comes from a recorded file, so only the `--harness-files` manifest part
    // of this check can name a missing subagent; that part is PRD-08's manifest case, not this.
    assert_eq!(result(&v, "subagents_present"), "pass");
}

#[test]
fn a_message_id_never_names_the_main_thread() {
    // Line 1 is lost; a later line carries a message `id`, which is not the session id.
    let body = format!("not json\n{}\n", json!({"id": "11111111-0000-4000-8000-0000000000aa"}));
    let t = table(&entry("gp-e", &[(MAIN_FILE, &body)]));
    let stem = "session-2026-11-01T08-42-d56586ab";
    assert!(t.threads.iter().any(|x| x.id == stem));
    assert!(t.threads.iter().all(|x| !x.id.starts_with("11111111")));
}
