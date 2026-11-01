//! Rebuilds the selftest fixtures k7-part, k7-fetched, k11-d-part and k11-d-fetched, which the
//! check_run1_shape and check_history helpers cannot write. Not compiled in place: a box job copies it to
//! crates/goalcup-verify/tests/regen_k7_k11d.rs and runs it (see the scripts/pad/selftest docstring).
//! GOALCUP_FIXTURES_IN is scripts/pad/fixtures (k7's raw records are read from k7-part there);
//! GOALCUP_FIXTURES_OUT receives the four folders.
mod common;
use common::*;
use goalcup_format::codec::Record;
use goalcup_format::part::new_encoder;
use goalcup_format::read::{open_dir, part_files, read_index};
use goalcup_format::{EndReason, StreamKind, hex};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::PathBuf;
use std::{env, fs};

fn fx() -> PathBuf { PathBuf::from(env::var("GOALCUP_FIXTURES_IN").unwrap()) }
fn out() -> PathBuf { PathBuf::from(env::var("GOALCUP_FIXTURES_OUT").unwrap()) }

/// k7: the same raw records, re-compressed as the writer does (one frame, flushed at every LINK,
/// an L line per link and an E line at the last), and the fetched copy as three chunks.
#[test]
fn regen_k7() {
    let rec = fx().join("k7-part/rec");
    let items: Vec<_> = open_dir(&rec).unwrap().map(|i| i.unwrap()).collect();
    let mut raw = Vec::new();
    for p in part_files(&rec).unwrap() {
        raw.extend(zstd::decode_all(fs::File::open(p).unwrap()).unwrap());
    }
    let mut links = vec![];
    for (i, it) in items.iter().enumerate() {
        if let Record::Link { seq, hash, .. } = &it.rec {
            let end = items.get(i + 1).map_or(raw.len() as u64, |n| n.off);
            links.push((*seq, end, *hash));
        }
    }
    assert_eq!(links.last().unwrap().1, raw.len() as u64, "nothing after the last link");
    let mut enc = new_encoder(Vec::new(), 20).unwrap();
    let (mut at, mut idx, mut ends) = (0u64, String::new(), vec![]);
    let n = links.len();
    for (k, (seq, end, hash)) in links.iter().enumerate() {
        enc.write_all(&raw[at as usize..*end as usize]).unwrap();
        at = *end;
        if k + 1 < n {
            enc.flush().unwrap();
            let c = enc.get_ref().len() as u64;
            idx += &format!("L {seq} {end} {c} {}\n", hex(hash));
            ends.push((*seq, *end, c, *hash));
        }
    }
    let z = enc.finish().unwrap();
    let (seq, end, hash) = links[n - 1];
    idx += &format!("E {seq} {end} {} {}\n", z.len(), hex(&hash));
    ends.push((seq, end, z.len() as u64, hash));
    let o = out();
    fs::create_dir_all(o.join("k7-part/rec")).unwrap();
    fs::create_dir_all(o.join("k7-fetched")).unwrap();
    fs::write(o.join("k7-part/rec/rec-00000000000000000000.gcr.zst"), &z).unwrap();
    fs::write(o.join("k7-part/rec/rec-00000000000000000000.idx"), idx).unwrap();
    fs::write(o.join("k7-fetched/rec.gcr.zst"), &z).unwrap();
    // Three chunks, each from one index line to a later one, cut at links 40 and 80.
    let cuts = [40u64, 80, seq];
    let (mut chunks, mut r0, mut c0, mut first, mut prev) = (vec![], 0u64, 0u64, 0u64, "0".repeat(64));
    for cut in cuts {
        let (s, e, c, h) = *ends.iter().find(|x| x.0 == cut).unwrap();
        let b = &z[c0 as usize..c as usize];
        chunks.push(json!({"offset": r0, "length": e - r0, "bytes": b.len(), "sha256": hex(&Sha256::digest(b)),
            "first_seq": first, "last_seq": s, "prev": prev, "head": hex(&h)}));
        (r0, c0, first, prev) = (e, c, s + 1, hex(&h));
    }
    let mut doc: Value = serde_json::from_slice(&fs::read(fx().join("k7-fetched/fetch.json")).unwrap()).unwrap();
    assert_eq!(doc["upload_offset"], json!(r0));
    assert_eq!(doc["final"]["head"], json!(prev));
    doc["chunks"] = json!(chunks);
    fs::write(o.join("k7-fetched/fetch.json"), doc.to_string()).unwrap();
}

const MAIN: &str = "fake-thread-main";
const KID: &str = "fake-kid-1";

/// Thread `id`'s call `k`, resending `seen`, answered with a tool call and usage (run 1's post).
fn post(e: &mut Entry, id: &str, k: u64, seen: &[u64]) {
    let call = |k: u64| json!({"type": "function_call", "call_id": format!("call_{k}"), "name": "shell", "arguments": "{}"});
    let ev = |what: &str, x: Value| json!({"type": format!("response.{what}"), "response": {"id": format!("r_{k}"),
        "model": "fake-model", "status": what, "usage": x}});
    let all = [ev("created", Value::Null), json!({"type": "response.output_item.done", "item": call(k)}),
        ev("completed", json!({"input_tokens": 9, "output_tokens": 2}))];
    let body: String = all.iter().map(|v| format!("event: {}\ndata: {v}\n\n", v["type"].as_str().unwrap())).collect();
    let done = |j: u64| json!({"type": "function_call_output", "call_id": format!("call_{j}"), "output": "fake"});
    let mut input: Vec<Value> = seen.iter().flat_map(|&j| [call(j), done(j)]).collect();
    input.push(json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": "fake turn"}]}));
    let req = json!({"model": "fake-model", "input": input}).to_string();
    let head = json!({"method": "POST", "path": "/v1/responses", "ver": "1.1", "masked": [], "coding": null,
        "headers": [["content-type", "application/json"], ["thread-id", id]]});
    let (c, ok) = (e.conn(), EndReason::Complete);
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    e.data(q, 0, 0, req.as_bytes());
    e.end(q, ok, req.len() as u64, Value::Null);
    let head = json!({"status": 200, "ver": "1.1", "headers": [["content-type", "text/event-stream"]], "masked": [],
        "coding": null, "decoded": true});
    let p = e.open(StreamKind::HttpResp, c, q, head);
    e.data(p, 0, 0, body.as_bytes());
    e.end(p, ok, body.len() as u64, Value::Null);
    e.end(c, ok, 0, json!({"up": 900, "down": 900}));
}

/// k11-d: Codex-shaped; the calls (with usage) and history are on the wire only. The one native
/// stream is the root's rollout holding its session_meta and the spawn that names the child: the
/// parentage, and no native call (no token_count line), so there is no harness call log to compare.
#[test]
fn regen_k11d() {
    let mut e = Entry::new("k11d");
    let s = e.open(StreamKind::Native, 0, 0,
        json!({"path": format!("sessions:2026/11/07/rollout-{MAIN}.jsonl"), "gen": 0, "from": 0}));
    let item = json!({"type": "CollabAgentToolCall", "tool": "spawn_agent", "receiver_thread_ids": [KID]});
    let lines = [
        json!({"type": "session_meta", "payload": {"id": MAIN, "source": "cli"}, "timestamp": "2026-11-07T00:00:00Z"}),
        json!({"type": "event_msg", "payload": {"type": "item_completed", "item": item}, "timestamp": "2026-11-07T00:00:00Z"}),
    ];
    let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
    e.data(s, 0, 0, body.as_bytes());
    e.end(s, EndReason::Complete, body.len() as u64, Value::Null);
    let (mut a, mut b) = (vec![], vec![]);
    for k in 0..4u64 {
        e.idle(1_000);
        post(&mut e, MAIN, k, &a);
        a.push(k);
        if k % 2 == 1 {
            post(&mut e, KID, 100 + k, &b);
            b.push(100 + k);
        }
        e.count();
    }
    let dir = e.seal();
    let dir = mutate(&dir, None, |r| edit_event(r, "start", 0, |v| v["connector"] = "codex-cli".into()));
    // The fetched copy, one chunk per index line (check_run1_shape's honest fetch), plus S13's final fields.
    let f = fetched(&dir, &tmp("k11d-fetched")).join(ENTRY);
    let (mut chunks, mut raw, mut prev) = (vec![], 0, "0".repeat(64));
    for p in part_files(&dir.join("rec")).unwrap() {
        let (bb, mut c0) = (fs::read(&p).unwrap(), 0);
        for l in read_index(&p.with_extension("").with_extension("idx")).unwrap() {
            let x = &bb[c0..l.c_end as usize];
            chunks.push(json!({"offset": raw, "length": l.raw_end - raw, "bytes": x.len(), "sha256": hex(&Sha256::digest(x)),
                "first_seq": if chunks.is_empty() { 0 } else { chunks.len() as u64 }, "last_seq": l.seq, "prev": prev, "head": hex(&l.hash)}));
            (raw, prev, c0) = (l.raw_end, hex(&l.hash), l.c_end as usize);
        }
    }
    assert_eq!(chunks.len(), 1);
    let mut seal = Value::Null;
    edit_event(&mut read_records(&dir.join("rec")), "seal", 0, |v| seal = v.clone());
    let t = &seal["totals"];
    let n = t["http_post"].as_u64().unwrap() + t["ws_up_msgs"].as_u64().unwrap();
    let hb = json!({"recording": "ok", "model_requests": n, "counts": t});
    let doc = json!({"upload_offset": raw, "model_requests": n, "last_heartbeat": hb,
        "final": {"seq": chunks[0]["last_seq"], "head": prev}, "chunks": chunks, "complete": true, "missing": null});
    fs::write(f.join("fetch.json"), doc.to_string()).unwrap();
    let o = out();
    let _ = fs::remove_dir_all(o.join("k11-d-part"));
    let _ = fs::remove_dir_all(o.join("k11-d-fetched"));
    export(&dir, &o.join("k11-d-part"));
    export(&f, &o.join("k11-d-fetched"));
}
