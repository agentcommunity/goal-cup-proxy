//! The run 1 shape builder (PRD-08 R1, R3, R4; PRD-12 K2): a Codex-shaped entry scaled to 120
//! minutes, a root thread and four children. `Shape` picks the losses it holds. All values fake.

use super::*;

#[derive(Clone, Copy, PartialEq)]
pub enum Shape {
    /// The wire stops at minute 49 while the rollouts run on, with run 1's other losses.
    Lossy,
    /// The same record with no loss.
    Clean,
    /// Clean, but the recorder is dead from minute 60 to 70.
    Killed,
    /// Clean, but the model host is a pinned tunnel: no wire call at all.
    Dodger,
}

const IDS: [&str; 5] = [
    "fake-thread-main",
    "fake-kid-1",
    "fake-kid-2",
    "fake-kid-3",
    "fake-kid-4",
];

/// Thread `id`'s call `k` as a POST on its own connection, answered over SSE. A rate-limited
/// call sends its error as an `error` event and as a `response.failed`, as run 1's did.
fn post(e: &mut Entry, id: &str, k: u64, limited: bool) {
    let ev = |what: &str, x: Value| {
        let key = if what == "failed" { "error" } else { "usage" };
        json!({"type": format!("response.{what}"), "response": {"id": format!("r_{k}"),
            "model": "fake-model", "status": what, key: x}})
    };
    let rl = json!({"code": "rate_limit_exceeded", "message": "fake"});
    let err = json!({"type": "error", "status": 429, "error": rl});
    let used = json!({"input_tokens": 9});
    let end = if limited {
        vec![err, ev("failed", rl)]
    } else {
        vec![ev("completed", used)]
    };
    let sse = |v: Value| format!("event: {}\ndata: {v}\n\n", v["type"].as_str().unwrap());
    let all = [vec![ev("created", Value::Null)], end].concat();
    let body: String = all.into_iter().map(sse).collect();
    let req = json!({"model": "fake-model", "input": "fake"}).to_string();
    let head = json!({"method": "POST", "path": "/v1/responses", "ver": "1.1", "masked": [],
        "coding": null, "headers": [["content-type", "application/json"], ["thread-id", id]]});
    let (c, done) = (e.conn(), EndReason::Complete);
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    e.data(q, 0, 0, req.as_bytes());
    e.end(q, done, req.len() as u64, Value::Null);
    let ctype = [["content-type", "text/event-stream"]];
    let head = json!({"status": 200, "ver": "1.1", "headers": ctype, "masked": [], "coding": null,
        "decoded": true});
    let p = e.open(StreamKind::HttpResp, c, q, head);
    e.data(p, 0, 0, body.as_bytes());
    e.end(p, done, body.len() as u64, Value::Null);
    e.end(c, done, 0, json!({"up": 900, "down": 900}));
}

/// A minute-long tunnel to the model host; only its GAP says it was there.
fn tunnel(e: &mut Entry) {
    let meta = json!({"host": HOST, "port": 443, "mode": "tunnel", "listed": "shared",
        "kind": "provider", "reason": "pinned", "alpn": null, "tls": null,
        "fingerprint": "0123456789abcdef"});
    let s = e.open(StreamKind::Conn, 0, 0, meta);
    e.gap(s, GapReason::Pinned, 58_000, 0, 2_561, Value::Null);
    e.end(s, EndReason::Complete, 0, json!({"up": 512, "down": 2048}));
}

/// A thread's rollout file: its stream, next offset and `token_count` calls so far.
struct Roll {
    id: &'static str,
    sid: u64,
    off: u64,
    n: u64,
}

impl Roll {
    fn new(id: &&'static str) -> Self {
        Self {
            id,
            sid: 0,
            off: 0,
            n: 0,
        }
    }

    /// Opens the stream (again after a restart) at the end of what was read.
    fn open(&mut self, e: &mut Entry) {
        let path = format!("sessions:2026/11/07/rollout-{}.jsonl", self.id);
        let meta = json!({"path": path, "gen": 0, "from": self.off});
        self.sid = e.open(StreamKind::Native, 0, 0, meta);
    }

    fn end(&self, e: &mut Entry) {
        e.end(self.sid, EndReason::Complete, self.off, Value::Null);
    }

    /// One rollout line, timed `ns` after the start; `v` holds its `type` and `payload`.
    fn line(&mut self, e: &mut Entry, ns: u64, mut v: Value) {
        let s = ns / 1_000_000_000;
        let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
        v["timestamp"] = format!("2026-11-07T{h:02}:{m:02}:{s:02}Z").into();
        let line = format!("{v}\n");
        e.data(self.sid, self.off, 0, line.as_bytes());
        self.off += line.len() as u64;
    }

    /// One native call; the running total grows with each.
    fn call(&mut self, e: &mut Entry, ns: u64) {
        self.n += 1;
        let last = json!({"input_tokens": 9, "output_tokens": 2, "total_tokens": 11});
        let total = json!({"total_tokens": 11 * self.n});
        let info = json!({"total_token_usage": total, "last_token_usage": last});
        let p = json!({"type": "event_msg", "payload": {"type": "token_count", "info": info}});
        self.line(e, ns, p);
    }
}

/// The sealed entry folder of `shape`.
pub fn run1(name: &str, shape: Shape) -> PathBuf {
    use Shape::*;
    let mut e = Entry::new(name);
    let n = if shape == Lossy { 4 } else { 5 };
    let mut rolls: Vec<Roll> = IDS[..n].iter().map(Roll::new).collect();
    for r in &mut rolls {
        r.open(&mut e);
        let kid = json!({"subagent": {"thread_spawn": {"parent_thread_id": IDS[0], "depth": 1}}});
        let id = json!({"id": r.id, "source": if r.id == IDS[0] { json!("cli") } else { kid }});
        r.line(&mut e, 0, json!({"type": "session_meta", "payload": id}));
    }
    // The root names every child, the fourth included, which Lossy never reads.
    let item = json!({"type": "CollabAgentToolCall", "tool": "spawn_agent",
        "receiver_thread_ids": &IDS[1..]});
    let spawn = json!({"type": "event_msg", "payload": {"type": "item_completed", "item": item}});
    rolls[0].line(&mut e, 0, spawn);
    let mut t0 = 0;
    for m in 1..=120u64 {
        let down = shape == Killed && (60..70).contains(&m);
        let on = !down && (m <= 49 || shape != Lossy) && shape != Dodger;
        if m == 60 && shape == Killed {
            rolls.iter().for_each(|r| r.end(&mut e));
            t0 = e.clock.ns;
            e.crash("killed", 600_000);
            rolls.iter_mut().for_each(|r| r.open(&mut e));
        } else if shape == Dodger {
            tunnel(&mut e);
        } else if !down {
            e.idle(59_000);
        }
        let ns = if down {
            t0 + (m - 59) * 60_000_000_000
        } else {
            e.clock.ns
        };
        if shape == Lossy && m % 10 == 0 && m <= 30 {
            let r = &rolls[m as usize / 10 - 1];
            e.gap(r.sid, GapReason::MemoryGuard, 0, r.off, 0, Value::Null);
        }
        if on && m % 5 == 0 && m <= 30 {
            post(&mut e, IDS[0], m * 10 + 9, true);
        }
        for (i, r) in rolls.iter_mut().enumerate() {
            if i == 0 || m % 12 == 3 * i as u64 % 12 {
                if on {
                    post(&mut e, r.id, m * 10 + i as u64, false);
                }
                r.call(&mut e, ns);
            }
        }
        if !down || m == 69 {
            e.count();
        }
    }
    // Lossy leaves the last two it opened with no END.
    let skip = if shape == Lossy { 2 } else { 0 };
    for r in rolls.iter().rev().skip(skip) {
        r.end(&mut e);
    }
    let dir = e.seal();
    let edit = |v: &mut Value| {
        v["connector"] = "codex-cli".into();
        v["hosts"]["shared"][0]["pinned"] = (shape == Dodger).into();
    };
    mutate(&dir, None, |r| {
        for n in 0..=usize::from(shape == Killed) {
            edit_event(r, "start", n, edit);
        }
    })
}
