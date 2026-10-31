//! PRD-09's usage test (ledger B10): a run 1 shaped entry, scaled down (PRD-09 section 8), read
//! through seam U1 only: wire cases and, under `codex-cli` and `claude-code-cli`, the native leg
//! and join, and the files `usage` writes. Every value is an obvious fake; the panic hook is the
//! binary's (PRD-08).

mod common;

use common::{ENTRY, Entry, WALL0, edit_event, read_records, tmp, write_raw};
use goalcup_format::read::part_files;
use goalcup_format::{EndReason, FLAG_FIN, GapReason, Record, StreamKind};
use goalcup_verify::usage::{
    CallRow, CallTable, Calls, Join, Kind, Leg, NativeForm, Outcome, Reason, Tokens,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path, path::PathBuf, process::Command};

/// Planted in prompts, outputs and error messages; no row may hold it (point 4).
const MARK: &str = "zebra-lantern-planted-text";
const RL: &str = "rate_limit_exceeded";
const MAIN: &str = "fake-thread-main";
const KID: &str = "fake-thread-kid";
/// The requested model of every call planted: 8 on the WebSocket, 4 over HTTP.
const CALLS: &str = "fake-ws-a fake-ws-b fake-ws-d fake-ws-r1 fake-ws-r2 fake-ws-r3 fake-ws-r4 \
    fake-ws-e fake-http-a fake-http-429 fake-http-cut fake-claude";

type Five = [Option<u64>; 5];

fn five(t: &Tokens) -> Five {
    [t.input, t.cached, t.cache_write, t.output, t.reasoning]
}

/// A Responses usage object and the five numbers the reader must give for it.
fn usage(i: u64, c: u64, o: u64, r: u64) -> (Value, Five) {
    let v = json!({"input_tokens": i, "input_tokens_details": {"cached_tokens": c},
        "output_tokens": o, "output_tokens_details": {"reasoning_tokens": r},
        "total_tokens": i + o});
    (v, [Some(i - c), Some(c), None, Some(o), Some(r)])
}

/// A request for model `fake-<k>`, following reply `r_<prev>`.
fn create(k: &str, prev: Option<&str>) -> Value {
    json!({"type": "response.create", "model": format!("fake-{k}"), "reasoning": {"effort": "low"},
        "previous_response_id": prev.map(|p| format!("r_{p}")), "store": false, "stream": true,
        "client_metadata": {"thread_id": MAIN},
        "input": [{"type": "message", "role": "user",
                   "content": [{"type": "input_text", "text": MARK}]}]})
}

/// Event `response.<ev>` of reply `r_<k>`; `x` is its usage, or its error when it failed.
fn ev(what: &str, k: &str, x: &Value) -> Value {
    let key = if what == "failed" { "error" } else { "usage" };
    json!({"type": format!("response.{what}"), "response": {"id": format!("r_{k}"),
        "model": format!("fake-{k}-dated"), "status": what, key: x}})
}

fn created(k: &str) -> Value {
    ev("created", k, &Value::Null)
}

fn delta() -> Value {
    json!({"type": "response.output_text.delta", "delta": MARK})
}

/// SSE text of these events, each named by its `type`.
fn sse(evs: &[Value]) -> String {
    let ev = |v: &Value| format!("event: {}\ndata: {v}\n\n", v["type"].as_str().unwrap());
    evs.iter().map(ev).collect()
}

/// A WebSocket on its own connection, upgraded at `path`: its conn, up and down sids.
fn socket(e: &mut Entry, path: &str) -> (u64, u64, u64) {
    let c = e.conn();
    let head = json!({"method": "GET", "path": path, "ver": "1.1",
        "headers": [["upgrade", "websocket"]], "masked": [], "coding": null});
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    e.end(q, EndReason::Complete, 0, Value::Null);
    let up = e.open(StreamKind::WsUp, c, q, json!({"deflate": false}));
    let down = e.open(StreamKind::WsDown, c, q, json!({"deflate": false}));
    (c, up, down)
}

/// One whole text message `v` on stream `sid` at `*at`, which moves past it.
fn send(e: &mut Entry, sid: u64, at: &mut u64, v: impl ToString) {
    let v = v.to_string();
    e.data(sid, *at, 0x10 | FLAG_FIN, v.as_bytes());
    *at += v.len() as u64;
}

/// One POST on its own connection, from `[path, request, status, reply, ended complete, thread
/// header or null]`; the reply body (SSE text, else JSON) comes in two pieces.
fn post(e: &mut Entry, s: &Value) {
    let (c, req) = (e.conn(), s[1].to_string());
    let mut h = vec![json!(["content-type", "application/json"])];
    h.extend(s[5].is_string().then(|| json!(["thread-id", s[5]])));
    let head = json!({"method": "POST", "path": s[0], "ver": "1.1", "masked": [], "coding": null,
        "headers": h});
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    e.data(q, 0, 0, req.as_bytes());
    e.end(q, EndReason::Complete, req.len() as u64, Value::Null);
    let (body, ctype) = match s[3].as_str() {
        Some(t) => (t.to_owned(), "text/event-stream"),
        None => (s[3].to_string(), "application/json"),
    };
    let head = json!({"status": s[2], "ver": "1.1", "headers": [["content-type", ctype]],
        "masked": [], "coding": null, "decoded": true});
    let p = e.open(StreamKind::HttpResp, c, q, head);
    let (x, y) = body.split_at(body.len() / 2);
    e.data(p, 0, 0, x.as_bytes());
    e.data(p, x.len() as u64, 0, y.as_bytes());
    let end = match s[4].as_bool() {
        Some(true) => EndReason::Complete,
        _ => EndReason::Interrupted,
    };
    e.end(p, end, body.len() as u64, Value::Null);
    e.end(c, EndReason::Complete, 0, json!({"up": 900, "down": 900}));
}

/// The entry's records and the five numbers planted per call with usage, by requested model.
fn run1(name: &str) -> (Vec<Record>, BTreeMap<&'static str, Five>) {
    let ((ua, ta), (ub, tb)) = (usage(1000, 600, 200, 50), usage(1300, 1100, 90, 0));
    let ((ue, te), (uh, th)) = (usage(2000, 1500, 300, 120), usage(700, 100, 90, 30));
    let (uw, rl) = (usage(500, 0, 0, 0).0, json!({"code": RL, "message": MARK}));
    let err = json!({"type": "error", "status": 429, "error": rl});
    let mut warm = create("warm", None);
    warm["generate"] = false.into();
    // Each step: a client message, then the server messages that answer it.
    let ws = json!([
        [warm, [created("warm"), ev("completed", "warm", &uw)]],
        [create("ws-a", None), [created("ws-a"), delta(), ev("completed", "ws-a", &ua)]],
        [create("ws-b", Some("ws-a")), [created("ws-b"), ev("completed", "ws-b", &ub)]],
        [warm, [created("warm"), ev("completed", "warm", &uw)]],
        [create("ws-d", None), [created("ws-d"), delta()]],
        [{"type": "response.interrupt"}, []],
        [create("ws-r1", None), [created("ws-r1"), err, ev("failed", "ws-r1", &rl)]],
        [create("ws-r2", None), [created("ws-r2"), ev("failed", "ws-r2", &rl), err]],
        [create("ws-r3", None), [created("ws-r3"), ev("failed", "ws-r3", &rl)]],
        // Refused before `response.created`, after the previous call failed.
        [create("ws-r4", None), [err]],
        [create("ws-e", Some("ws-b")), [created("ws-e"), ev("completed", "ws-e", &ue)]]
    ]);
    // Anthropic usage is cumulative: the largest value wins, never a sum.
    let au = |o: u64| {
        json!({"input_tokens": 300, "cache_read_input_tokens": 40,
        "cache_creation_input_tokens": 20, "output_tokens": o})
    };
    let claude = sse(&[
        json!({"type": "message_start",
            "message": {"id": "msg_fake_1", "model": "fake-claude-dated", "usage": au(1)}}),
        json!({"type": "ping"}),
        json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": MARK}}),
        json!({"type": "message_delta", "delta": {"stop_reason": null}, "usage": au(30)}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": au(75)}),
        json!({"type": "message_stop"}),
    ]);
    let tc = [Some(300), Some(40), Some(20), Some(75), None];
    let http = |k: &str| json!({"model": format!("fake-{k}"), "stream": true, "input": MARK});
    let ha = sse(&[created("http-a"), delta(), ev("completed", "http-a", &uh)]);
    let creq = json!({"model": "fake-claude", "messages": [{"role": "user", "content": MARK}]});
    let posts = json!([
        ["/v1/responses", http("http-a"), 200, ha, true, KID],
        ["/v1/responses", http("http-429"), 429, {"error": rl}, true, KID],
        ["/v1/responses", http("http-cut"), 200, sse(&[created("http-cut"), delta()]), false, KID],
        ["/v1/messages?beta=true", creq, 200, claude, true, null],
        ["/v1/fake-events", {"event": MARK}, 200, {"ok": true}, true, null]
    ]);

    let mut e = Entry::new(name);
    let (c, up, down) = socket(&mut e, "/backend-api/codex/responses");
    let (mut u, mut d) = (0, 0);
    for step in ws.as_array().unwrap() {
        let m = step[0].to_string();
        let (x, y) = m.split_at(m.len() / 2);
        e.data(up, u, 0x10, x.as_bytes());
        e.data(up, u + x.len() as u64, 0x10 | FLAG_FIN, y.as_bytes());
        u += m.len() as u64;
        for v in step[1].as_array().unwrap() {
            send(&mut e, down, &mut d, v);
        }
    }
    e.end(up, EndReason::Complete, u, json!({"close_code": 1000}));
    e.end(down, EndReason::Complete, d, json!({"close_code": 1000}));
    e.end(c, EndReason::Complete, 0, json!({"up": 900, "down": 900}));
    for s in posts.as_array().unwrap() {
        post(&mut e, s);
    }
    // A native call per ok wire call of its thread, near it, and 2 in the silence after.
    let main = ["00:00:01", "00:00:01", "00:00:01", "00:10:00", "00:10:00"];
    rollout(&mut e, MAIN, None, &main);
    rollout(&mut e, KID, Some(MAIN), &["00:00:01"]);
    let planted = [("fake-ws-a", ta), ("fake-ws-b", tb), ("fake-ws-e", te)];
    let planted = planted
        .into_iter()
        .chain([("fake-http-a", th), ("fake-claude", tc)]);
    e.count();
    (read_records(&e.seal().join("rec")), planted.collect())
}

/// A Codex rollout of thread `id` (spawned by `parent`, with a label too long to keep), a
/// message holding MARK, then a `token_count` at each time `at`.
fn rollout(e: &mut Entry, id: &str, parent: Option<&str>, at: &[&str]) {
    let p = MARK.repeat(6);
    let src = json!({"thread_spawn": {"parent_thread_id": parent, "depth": 1, "agent_path": p}});
    let meta = json!({"timestamp": "2026-11-07T00:00:00Z", "type": "session_meta", "payload":
        {"id": id, "source": parent.map_or(json!("cli"), |_| json!({"subagent": src}))}});
    let item = json!({"timestamp": "2026-11-07T00:00:00Z", "type": "response_item", "payload":
        {"type": "message", "role": "assistant", "content": MARK}});
    let mut f = format!("{meta}\n{item}\n");
    for (n, t) in (1..).zip(at) {
        let v = json!({"timestamp": format!("2026-11-07T{t}Z"), "type": "event_msg", "payload":
            {"type": "token_count", "info": {"total_token_usage": {"total_tokens": 11 * n},
            "last_token_usage": {"input_tokens": 9, "output_tokens": 2, "total_tokens": 11}}}});
        f += &format!("{v}\n");
    }
    // The folder holds MARK too: a session path can name the participant's folders (point 4).
    let path = format!("sessions:{MARK}/rollout-{id}.jsonl");
    stream(e, StreamKind::Native, &path, &f);
}

/// One stream of `skind` holding `body` as the file at `path`, ended complete.
fn stream(e: &mut Entry, skind: StreamKind, path: &str, body: &str) {
    let s = e.open(skind, 0, 0, json!({"path": path, "gen": 0, "from": 0}));
    e.data(s, 0, 0, body.as_bytes());
    e.end(s, EndReason::Complete, body.len() as u64, Value::Null);
}

fn table(recs: &[Record]) -> CallTable {
    let mut calls = Calls::new();
    recs.iter().for_each(|r| calls.push(r));
    calls.finish()
}

/// The one row whose request named model `m`.
fn row<'a>(t: &'a CallTable, m: &str) -> &'a CallRow {
    let named = |r: &&CallRow| r.model_requested.as_deref() == Some(m);
    let rows: Vec<_> = t.rows.iter().filter(named).collect();
    assert_eq!(rows.len(), 1, "{m}: {rows:?}");
    rows[0]
}

/// The status and code of a rate-limited row.
fn limited(r: &CallRow) -> Option<(Option<u16>, Option<&str>)> {
    match &r.outcome {
        Outcome::RateLimited { status, code } => Some((*status, code.as_deref())),
        _ => None,
    }
}

#[test]
fn warmups_and_interrupts_are_not_calls() {
    let t = table(&run1("u-kinds").0);
    let n = |k| t.rows.iter().filter(|r| r.kind == k).count();
    let kinds = [Kind::Warmup, Kind::Interrupt, Kind::Other, Kind::Call].map(n);
    assert_eq!(kinds, [2, 1, 1, CALLS.split_whitespace().count()]);
    for m in CALLS.split_whitespace() {
        assert_eq!(row(&t, m).kind, Kind::Call, "{m}");
    }
}

#[test]
fn each_rate_limited_call_is_one_row() {
    let (recs, planted) = run1("u-rl");
    let t = table(&recs);
    for m in ["fake-ws-r1", "fake-ws-r2", "fake-ws-r3", "fake-ws-r4"] {
        assert_eq!(limited(row(&t, m)).and_then(|l| l.1), Some(RL), "{m}");
    }
    let http = limited(row(&t, "fake-http-429"));
    assert_eq!(http.and_then(|l| l.0), Some(429));
    assert_eq!(t.rows.iter().filter(|r| r.outcome.is_error()).count(), 5);
    let after = row(&t, "fake-ws-e");
    assert_eq!(after.outcome, Outcome::Ok);
    assert_eq!(five(&after.tokens), planted["fake-ws-e"]);
}

/// Run 1 had 8 cut WebSocket client messages: a create the recorder wrote in part before it
/// died is still a call, not whole (PRD-01 section 7: cut by the `recorder_down` gap).
#[test]
fn a_create_cut_by_a_restart_is_a_call_not_whole() {
    let mut e = Entry::new("u-restart");
    let (_, up, down) = socket(&mut e, "/v1/responses");
    let m = format!(r#"{{"type":"response.create","model":"fake-ws-x","input":"{MARK}"}}"#);
    let (mut u, mut d) = (0, 0);
    send(&mut e, up, &mut u, &m);
    let done = ev("completed", "ws-x", &usage(9, 0, 1, 0).0);
    for v in [created("ws-x"), done] {
        send(&mut e, down, &mut d, v);
    }
    e.data(up, u, 0x10, &m.as_bytes()[..40]);
    e.crash("killed", 1_000);
    let want = [(0, true, Outcome::Ok), (0, false, Outcome::Unknown)];
    assert_eq!(whole(&e.seal()), want);
}

/// A known loss ending where a reply message starts may hold its head (PRD-01 section 7: the
/// stream goes on at off + lost - 1): that call is not whole, the one before it stays whole.
#[test]
fn a_loss_ending_at_a_reply_message_marks_that_call_not_whole() {
    let mut e = Entry::new("u-loss");
    let (_, up, down) = socket(&mut e, "/v1/responses");
    let (mut u, mut d) = (0, 0);
    let done = |k| ev("completed", k, &usage(9, 0, 1, 0).0);
    send(&mut e, up, &mut u, create("ws-y", None));
    for v in [created("ws-y"), done("ws-y")] {
        send(&mut e, down, &mut d, v);
    }
    send(&mut e, up, &mut u, create("ws-z", None));
    let lost = created("ws-z").to_string().len() as u64;
    e.gap(down, GapReason::MemoryGuard, 0, d, lost + 1, Value::Null);
    d += lost;
    send(&mut e, down, &mut d, done("ws-z"));
    let calls = whole(&e.seal());
    assert_eq!(calls, [(0, true, Outcome::Ok), (0, false, Outcome::Ok)]);
}

/// Boot, `whole` and outcome of each call of a sealed entry.
fn whole(dir: &std::path::Path) -> Vec<(u64, bool, Outcome)> {
    let t = table(&read_records(&dir.join("rec")));
    let calls = t.rows.into_iter().filter(|r| r.kind == Kind::Call);
    calls.map(|r| (r.boot, r.whole, r.outcome)).collect()
}

#[test]
fn totals_equal_the_planted_sum_and_repeat() {
    let (recs, planted) = run1("u-sum");
    let t = table(&recs);
    let sum = |fives: Vec<Five>| -> [u64; 5] {
        std::array::from_fn(|i| fives.iter().map(|f| f[i].unwrap_or(0)).sum())
    };
    let said = |r: &CallRow| r.usage_from.map(|_| five(&r.tokens));
    let calls = t.rows.iter().filter(|r| r.kind == Kind::Call);
    let got = sum(calls.filter_map(said).collect());
    assert_eq!(got, sum(planted.into_values().collect()));
    assert_eq!(table(&recs), t);
}

#[test]
fn native_leg_joins_codex_threads() {
    let mut recs = run1("u-native").0;
    edit_event(&mut recs, "start", 0, codex);
    let t = table(&recs);
    let th = |id| t.threads.iter().find(|x| x.id == id).unwrap();
    assert_eq!([th(MAIN).native_only, th(KID).native_only], [2, 0]);
    assert_eq!(th(KID).parent.as_deref(), Some(MAIN));
    assert_eq!(t.children.as_ref().unwrap()[MAIN], [KID]);
    // Under `codex-cli`, exactly the ok hinted wire calls join; an ok call with no hint shows.
    let ok = ["fake-ws-a", "fake-ws-b", "fake-ws-e", "fake-http-a"];
    assert_eq!(joined(&t, Join::Matched), ok);
    assert_eq!(joined(&t, Join::Unattributed), ["fake-claude"]);
    // Point 4: no row, and no thread, holds text from a body or a session file (`path` is U1's).
    let bare = t.rows.iter().map(|r| CallRow {
        path: None,
        ..r.clone()
    });
    assert!(!format!("{:?}", (bare.collect::<Vec<_>>(), &t.threads)).contains(MARK));
}

/// Point 2: a Claude Code subagent counts wherever its transcript sits under its session's
/// `subagents/` (a workflow agent too), with its `agentType` from the meta snapshot.
#[test]
fn every_claude_code_subagent_transcript_counts() {
    let mut e = Entry::new("u-claude");
    let files = "S S/subagents/agent-a S/subagents/workflows/w/agent-b";
    for (k, f) in files.split(' ').enumerate() {
        // The first file's year-2300 time is no real time, so its call joins the Claude time join.
        let at = ["2300-01-01T00:00:00Z", "2026-11-07T00:00:00Z"][k.min(1)];
        let v = json!({"type": "assistant", "timestamp": at, "message": {"id": k.to_string(),
            "usage": {"input_tokens": 3}, "content": MARK}});
        let (path, body) = (format!("transcripts:p/{f}.jsonl"), format!("{v}\n"));
        stream(&mut e, StreamKind::Native, &path, &body);
    }
    let meta = "meta:p/S/subagents/workflows/w/agent-b.meta.json";
    stream(&mut e, StreamKind::Snapshot, meta, r#"{"agentType":"x"}"#);
    let agent_c = "meta:p/S/subagents/agent-c.meta.json";
    stream(&mut e, StreamKind::Snapshot, agent_c, "{}");
    let mut recs = read_records(&e.seal().join("rec"));
    let claude = |v: &mut Value| v["connector"] = "claude-code-cli".into();
    edit_event(&mut recs, "start", 0, claude);
    let t = table(&recs);
    assert_eq!(t.rows.iter().filter(|r| r.thread.is_some()).count(), 3);
    let th = |id| t.threads.iter().find(|x| x.id == id).unwrap();
    let parent = |id| th(id).parent.as_deref();
    assert_eq!([parent("a"), parent("b")], [Some("S"); 2]);
    assert_eq!(th("b").label.as_deref(), Some("x"));
    assert!(th("a").readable() && !th("c").readable());
    assert!(!format!("{t:?}").contains(MARK));
}

fn joined(t: &CallTable, j: Join) -> Vec<&str> {
    let rows = t.rows.iter().filter(|r| r.join == Some(j));
    rows.flat_map(|r| r.model_requested.as_deref()).collect()
}

fn codex(v: &mut Value) {
    v["connector"] = "codex-cli".into();
}

fn json(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

/// The Codex run 1 entry as a folder; `torn` loses its seal LINK and the end of its last frame.
fn codex_entry(name: &str, torn: bool) -> PathBuf {
    let (mut recs, dir) = (run1(name).0, tmp(name));
    let rec = dir.join("rec");
    fs::create_dir_all(&rec).unwrap();
    edit_event(&mut recs, "start", 0, codex);
    recs.truncate(recs.len() - usize::from(torn));
    write_raw(&rec, &recs, None);
    let last = part_files(&rec).unwrap().pop().unwrap();
    let mut bytes = fs::read(&last).unwrap();
    bytes.truncate(bytes.len() - 8 * usize::from(torn));
    fs::write(last, bytes).unwrap();
    dir
}

/// What `usage <path> --json --out <new folder>` gave: exit code, stdout, stderr, the two files.
#[derive(Debug, PartialEq)]
struct Ran {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    sum: String,
    calls: String,
}

fn run_usage(path: &Path) -> Ran {
    let out = tmp("u-out");
    let (p, o) = (path.to_str().unwrap(), out.to_str().unwrap());
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_goalcup-verify"));
    let got = cmd
        .args(["usage", p, "--json", "--out", o])
        .output()
        .unwrap();
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    let file =
        |end: &str| fs::read_to_string(out.join(format!("{ENTRY}{end}"))).unwrap_or_default();
    Ran {
        code: got.status.code(),
        stdout: text(&got.stdout),
        stderr: text(&got.stderr),
        sum: file(".json"),
        calls: file(".calls.jsonl"),
    }
}

#[test]
fn usage_files_repeat_and_add_up() {
    let dir = codex_entry("u-files", false);
    let (a, b) = (run_usage(&dir), run_usage(&dir));
    assert_eq!(a, b);
    assert_eq!(a.code, Some(0));
    let (sum, lines): (Value, Vec<Value>) = (json(&a.sum), a.calls.lines().map(json).collect());
    let said = |l: &&Value| l["k"] == "call" && !l["usage_from"].is_null();
    for k in ["input", "cached", "cache_write", "output", "reasoning"] {
        let col = lines.iter().filter(said);
        let got = col.filter_map(|l| l[k].as_u64()).reduce(|a, b| a + b);
        assert_eq!(sum["tokens"]["merged"][k].as_u64(), got, "{k}");
    }
    let calls = CALLS.split_whitespace().count();
    assert_eq!(sum["coverage"]["wire_calls"], calls);
    // Which subagent used which model is in the summary's thread rows, not only in the calls file.
    let kid = sum["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == KID)
        .unwrap();
    assert_eq!(kid["models"]["fake-http-a-dated"], 1);
}

/// Point 3 in the reader: a decode error partway through an entry still gives the summary of what
/// came, marked `read_error` with a line on stderr, never a clean-looking partial count.
#[test]
fn a_corrupt_entry_is_marked_not_silent() {
    let dir = codex_entry("u-corrupt", false);
    let first = part_files(&dir.join("rec")).unwrap().remove(0);
    let mut bytes = fs::read(&first).unwrap();
    let at = bytes.len() * 3 / 8;
    bytes[at..at + 16].fill(0xA5);
    fs::write(first, bytes).unwrap();
    let ran = run_usage(&dir);
    let ok = ran.code == Some(0) && json(&ran.sum)["read_error"] == true;
    assert!(
        !ran.stderr.is_empty() && !ran.stderr.contains(MARK) && ok,
        "{ran:?}"
    );
}

/// Point 4 on every path: the whole entry, a torn copy of it (exit 0, not sealed), a missing path.
#[test]
fn no_record_text_in_any_output() {
    let check = |path: PathBuf, code: i32, sealed: bool| {
        let ran = run_usage(&path);
        let text = [&ran.stdout, &ran.stderr, &ran.sum, &ran.calls]
            .map(String::as_str)
            .concat();
        assert_eq!(ran.code, Some(code), "{text}");
        assert!(!text.is_empty() && !text.contains(MARK));
        if code == 0 {
            assert_eq!(json(&ran.sum)["sealed"], sealed);
        }
    };
    check(codex_entry("u-whole", false), 0, true);
    check(codex_entry("u-torn", true), 0, false);
    check(tmp("u-none").join("absent"), 3, false);
}

/// A Hermes `state.db` (WAL kept, connection open): a parent and a delegated child session, an
/// old session and one begun 10 minutes before the run, assistant `messages` rows at 1, 1.5 (that
/// one), 2 and 3 s of the run. Returns db and WAL bytes.
fn hermes_db(dir: &Path) -> (Vec<u8>, Vec<u8>) {
    let s = |ms: u64| (WALL0 / 1_000_000 + ms) as f64 / 1e3;
    let p = dir.join("state.db");
    let sql = format!(
        "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
        CREATE TABLE sessions(id TEXT, parent_session_id TEXT, model TEXT, started_at REAL,
            api_call_count INTEGER, title TEXT, input_tokens INTEGER, cache_read_tokens INTEGER,
            cache_write_tokens INTEGER, output_tokens INTEGER, reasoning_tokens INTEGER);
        CREATE TABLE messages(session_id TEXT, role TEXT, timestamp REAL, active INTEGER, content TEXT);
        INSERT INTO sessions VALUES ('s-main', NULL, 'fake-hermes', {a}, 2, '{MARK}', 100, 30, 0, 20, 5),
            ('s-kid', 's-main', 'fake-kid', {a}, 1, '{MARK}', 7, 0, 0, 3, 0),
            ('s-old', NULL, 'fake-old', 1.0, 1, 'x', 1, 1, 1, 1, 1),
            ('s-back', NULL, 'fake-back', {b}, 1, 'x', 9, 0, 0, 4, 0);
        INSERT INTO messages VALUES ('s-main', 'assistant', {t1}, 1, '{MARK}'),
            ('s-main', 'user', {t1}, 1, '{MARK}'), ('s-main', 'assistant', {t2}, 0, '{MARK}'),
            ('s-kid', 'assistant', {t3}, 1, '{MARK}'), ('s-old', 'assistant', 1.0, 1, 'x'),
            ('s-back', 'assistant', {t4}, 1, 'x');",
        a = s(100),
        b = s(100) - 600.0,
        t1 = s(1000),
        t2 = s(2000),
        t3 = s(3000),
        t4 = s(1500)
    );
    let conn = rusqlite::Connection::open(&p).unwrap();
    conn.execute_batch(&sql).unwrap();
    (
        fs::read(&p).unwrap(),
        fs::read(dir.join("state.db-wal")).unwrap(),
    )
}

/// A `hermes-cli` entry of that `segment`: four Chat Completions calls, then database copies
/// `(n, db, wal)` as snapshot streams of set `0.n`, copied at `n` x 10 s of the run (no WAL when
/// it is empty). Returns the table and the entry's folder.
fn hermes_entry(name: &str, sets: &[(u64, &[u8], &[u8])], segment: u64) -> (CallTable, PathBuf) {
    let mut e = Entry::new(name);
    for k in 1..=4 {
        let req = json!({"model": format!("fake-h{k}"), "messages": [{"content": MARK}]});
        let reply = json!({"id": format!("c{k}"), "choices": [{"finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 2}});
        post(
            &mut e,
            &json!(["/v1/chat/completions", req, 200, reply, true, null]),
        );
    }
    for &(n, db, wal) in sets {
        for (file, body) in [("state.db", db), ("state.db-wal", wal)] {
            if body.is_empty() {
                continue;
            }
            let mtime = WALL0 + n * 10_000_000_000;
            let meta = json!({"path": format!("state:{file}"), "gen": 0, "form": "final",
                "set": format!("0.{n}"), "harness_exited": true, "size": body.len(), "mtime_ns": mtime});
            let s = e.open(StreamKind::Snapshot, 0, 0, meta);
            e.data(s, 0, 0, body);
            e.end(s, EndReason::Complete, body.len() as u64, Value::Null);
        }
    }
    e.count();
    let dir = e.seal();
    let mut recs = read_records(&dir.join("rec"));
    edit_event(&mut recs, "start", 0, |v| {
        v["connector"] = "hermes-cli".into();
        v["segment"] = segment.into()
    });
    write_raw(&dir.join("rec"), &recs, None);
    (table(&recs), dir)
}

/// Points 1 and 2 for Hermes: a WAL copy with a masked page (its checksum fails) is repaired and
/// read by its set, each matched wire call takes the native thread, a newer broken set falls back
/// to the older one and the output names it, a `continue` entry keeps the session that began in
/// an earlier entry and its rows, and an entry with no readable set says so and keeps every wire
/// call.
#[test]
fn hermes_database_copies_read_by_set() {
    let (db, wal) = hermes_db(&tmp("u-hermes"));
    let at = wal
        .windows(MARK.len())
        .position(|w| w == MARK.as_bytes())
        .unwrap();
    let mut masked = wal.clone();
    masked[at..at + MARK.len()].fill(b'*');
    let mut bad = db.clone();
    bad[..16].fill(b'*');
    let (t, _) = hermes_entry("u-h1", &[(1, &db, &masked)], 0);
    let th = |t: &CallTable, id: &str| t.threads.iter().find(|x| x.id == id).cloned();
    let who = |t: &CallTable| {
        ["fake-h1", "fake-h2", "fake-h3", "fake-h4"].map(|m| row(t, m).thread.clone())
    };
    let want = [Some("s-main"), Some("s-main"), Some("s-kid"), None].map(|s| s.map(String::from));
    assert_eq!(who(&t), want);
    assert_eq!(row(&t, "fake-h4").join, Some(Join::WireOnly));
    let (main, kid) = (th(&t, "s-main").unwrap(), th(&t, "s-kid").unwrap());
    assert_eq!(
        (kid.parent.as_deref(), main.native_total),
        (Some("s-main"), Some(2))
    );
    // The session totals sit on the thread, never on the native call rows.
    let tok = |i, c, w, o, r| Tokens {
        input: Some(i),
        cached: Some(c),
        cache_write: Some(w),
        output: Some(o),
        reasoning: Some(r),
    };
    assert_eq!(main.native_tokens, tok(100, 30, 0, 20, 5));
    assert!(
        t.rows
            .iter()
            .filter(|r| r.leg == Leg::Native)
            .all(|r| r.tokens == Tokens::default())
    );
    assert_eq!(t.children.as_ref().unwrap()["s-main"], ["s-kid"]);
    let gone = ["s-old", "s-back"].map(|id| th(&t, id));
    assert!(t.readable() && gone == [None, None] && t.native_form == NativeForm::PerCall);
    assert!(!format!("{t:?}").contains(MARK));
    let (newer, dir) = hermes_entry("u-h2", &[(1, &db, &masked), (2, &bad, &[])], 0);
    let until = Some(WALL0 + 10_000_000_000);
    assert_eq!(th(&newer, "s-main").unwrap().native_until, until);
    assert_eq!(
        (
            newer.db_set.as_deref(),
            newer.native_until,
            newer.harness_exited
        ),
        (Some("0.1"), until, Some(true))
    );
    let sum = json(&run_usage(&dir).sum);
    assert_eq!(
        (
            &sum["db_set"],
            sum["native_until"].is_string(),
            &sum["harness_exited"]
        ),
        (&json!("0.1"), true, &json!(true))
    );
    assert_eq!(sum["tokens"]["native"]["input"], 107);
    // A `continue` entry: the session begun before this entry keeps its thread and its row; one
    // with no row inside the entry is an earlier entry's.
    let (next, next_dir) = hermes_entry("u-h4", &[(1, &db, &masked)], 1);
    assert_eq!(who(&next)[1].as_deref(), Some("s-back"));
    let back = th(&next, "s-back").unwrap();
    assert!(back.readable() && th(&next, "s-old").is_none());
    // Its totals span earlier entries: kept and marked, not compared and not summed here.
    assert!(back.resumed && back.native_total.is_none() && back.native_tokens.input == Some(9));
    let sum = json(&run_usage(&next_dir).sum);
    assert_eq!(
        (&sum["tokens"]["native"]["input"], &sum["tokens"]["native_partial"]),
        (&json!(107), &json!(true))
    );
    let (none, _) = hermes_entry("u-h3", &[(2, &bad, &[])], 0);
    let wire_only = none
        .rows
        .iter()
        .filter(|r| r.join == Some(Join::WireOnly))
        .count();
    assert_eq!(
        (none.reason, none.readable(), wire_only),
        (Some(Reason::SnapshotUnreadable), false, 4)
    );
}
