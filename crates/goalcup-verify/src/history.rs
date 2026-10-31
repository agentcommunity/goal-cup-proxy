//! The history check (PRD-08 section 3.2, ledger T4): an earlier assistant answer resent inside a
//! later model request must exist as a recorded response, or the call that made it was missed.
//! One streaming pass keeps answer keys (a tool-call id, or the hash of the answer text) and, per
//! chain of requests, only the turns new since the chain's previous request. [`judge`] then needs
//! only the call table's thread ids and times. Requests are found by body (`messages`, `input` or
//! `contents`), never by path or harness. No text, id or byte of a body leaves this file: findings
//! name a thread, a stream, a time and a position.

use crate::integrity::{At, Check, Res};
use crate::legs::Cover;
use crate::secrets::answer_text;
use crate::usage::{CallTable, Calls, Leg};
use goalcup_format::dump::wall;
use goalcup_format::{FLAG_FIN, Record, StreamKind};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};

/// The first 16 bytes of a SHA-256.
type Key = [u8; 16];
/// (boot, stream, WebSocket message number): the call table's row key.
type Id = (u64, u64, Option<u64>);
/// One answer's keys: (key, whether it is a tool-call id).
type Turn = Vec<(Key, bool)>;

fn hash(b: &[u8]) -> Key {
    let mut k = [0; 16];
    k.copy_from_slice(&Sha256::digest(b)[..16]);
    k
}

/// The answer as one assistant turn: its tool-call ids, else (Gemini function calls, no id, and
/// text) the hash of its whitespace-collapsed text. `None` when it has neither.
fn turn(m: &Map<String, Value>) -> Option<Turn> {
    let s = |k: &str| m.get(k).and_then(Value::as_str);
    let id = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(String::from);
    let mut ids: Vec<String> = Vec::new();
    match s("type") {
        Some("tool_use") => ids.extend(s("id").map(String::from)),
        Some("function_call") => ids.extend(s("call_id").map(String::from)),
        _ => {}
    }
    let calls = m.get("tool_calls").and_then(Value::as_array);
    ids.extend(calls.into_iter().flatten().filter_map(|t| id(t, "id")));
    let (mut text, mut fcs) = (String::new(), Vec::new());
    if matches!(s("role"), Some("assistant" | "model")) {
        let parts = m.get("parts").and_then(Value::as_array);
        let blocks = m.get("content").and_then(Value::as_array);
        text.extend(s("content"));
        for p in blocks.into_iter().chain(parts).flatten() {
            if p.get("type").and_then(Value::as_str) == Some("tool_use") {
                ids.extend(id(p, "id"));
            }
            text.extend(p.get("text").and_then(Value::as_str));
            if let Some(f) = p.get("functionCall") {
                fcs.push(hash(format!("{}{}", f["name"], f["args"]).as_bytes()));
            }
        }
    }
    if !ids.is_empty() {
        return Some(ids.iter().map(|i| (hash(i.as_bytes()), true)).collect());
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = (!text.is_empty()).then(|| vec![(hash(text.as_bytes()), false)]);
    Some(fcs.into_iter().map(|k| (k, false)).collect::<Turn>())
        .filter(|f| !f.is_empty())
        .or(t)
}

/// Adds the answer keys of every object in a response event or body.
fn keys_of(v: &Value, keys: &mut HashSet<Key>) {
    match v {
        Value::Object(m) => {
            keys.extend(turn(m).into_iter().flatten().map(|k| k.0));
            m.values().for_each(|c| keys_of(c, keys));
        }
        Value::Array(a) => a.iter().for_each(|c| keys_of(c, keys)),
        _ => {}
    }
}

/// One response event: its answer keys, and its streamed answer text joined into `acc` by the
/// secret scan's joiner ([`answer_text`]); the scan and this check never keep two copies. The
/// answer leaves out thinking, reasoning and tool arguments, which a resent turn does not carry.
fn event(v: &Value, acc: &mut String, keys: &mut HashSet<Key>) {
    keys_of(v, keys);
    answer_text(v, acc);
}

/// The joined text of a whole response is one answer key.
fn texts(acc: &mut String, keys: &mut HashSet<Key>) {
    let t = std::mem::take(acc);
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if !t.is_empty() {
        keys.insert(hash(t.as_bytes()));
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Role {
    Req,
    /// A response to a model request; true when it is server-sent events.
    Resp(bool),
    Up,
    Down,
}

/// One open stream: the request body, WebSocket message or response carry being read.
struct St {
    role: Role,
    buf: Vec<u8>,
    next: u64,
    hole: bool,
    /// A body or message is being read (a request that looks like JSON; an open message).
    pend: bool,
    msgs: u64,
    acc: String,
}

/// One model request: where it ended, its chain, and its turns new to that chain.
struct Rq {
    id: Id,
    off: u64,
    chain: usize,
    first: bool,
    /// How many empty chains there were when this request opened its chain with no earlier
    /// match; [`judge`] may join it to the one of its own thread.
    waiting: usize,
    turns: Vec<(usize, Turn)>,
}

struct Ch {
    /// The digests of the last request's turns.
    last: Vec<u64>,
    /// Its first request resent no earlier answer.
    anchored: bool,
}

/// What the pass kept. Plain data, no `Debug`: the keys come from bodies (PRD-17 VR2).
#[derive(Default)]
pub struct Hist {
    reqs: Vec<Rq>,
    chains: Vec<Ch>,
    /// The anchored (empty) chains, in the order they began.
    zeros: Vec<usize>,
    answers: HashSet<Key>,
    /// Wall spans of GAPs on model streams.
    gaps: Vec<(u64, u64)>,
    /// Requests skipped: previous-response id only, and undecoded. The compacted turns are
    /// counted at judge.
    prev_response: u64,
    undecoded: u64,
    seen: u64,
}

/// U1's `Calls` plus the history state, fed the same records. No `Debug`: it holds body bytes.
#[derive(Default)]
pub struct Sink {
    calls: Calls,
    boot: u64,
    open: HashMap<(u64, u64), St>,
    /// Every HTTP and WebSocket stream of the record, by boot and sid.
    net: HashSet<(u64, u64)>,
    /// Candidate requests whose response is still to come.
    cand: HashSet<(u64, u64)>,
    by_last: HashMap<u64, Vec<usize>>,
    h: Hist,
}

fn mix(p: u64, d: u64) -> u64 {
    (p ^ d).wrapping_mul(0x0100_0000_01b3).rotate_left(13)
}

/// Complete lines of `buf` that carry a `data:` JSON event; the rest stays.
fn lines(buf: &mut Vec<u8>, f: &mut dyn FnMut(&Value)) {
    let mut at = 0;
    while let Some(i) = buf[at..].iter().position(|&b| b == b'\n') {
        let line = buf[at..at + i].trim_ascii();
        if let Some(d) = line.strip_prefix(b"data:")
            && let Ok(v) = serde_json::from_slice(d.trim_ascii())
        {
            f(&v);
        }
        at += i + 1;
    }
    buf.drain(..at);
}

impl Sink {
    /// Takes one record; `off` is its raw offset.
    pub fn push(&mut self, rec: &Record, off: u64) {
        self.calls.push(rec);
        match rec {
            Record::Part { boot, .. } if *boot != self.boot => {
                // The dead boot's open requests and messages are cut.
                let cut = std::mem::take(&mut self.open);
                let up = |s: &&St| s.pend && matches!(s.role, Role::Req | Role::Up);
                self.h.undecoded += cut.values().filter(up).count() as u64;
                self.boot = *boot;
            }
            Record::Open {
                sid,
                skind,
                req,
                meta,
                ..
            } => self.open(*sid, *skind, *req, meta),
            Record::Data {
                sid,
                off: o,
                flags,
                bytes,
                ..
            } => self.data(*sid, off, (*o, *flags), bytes),
            Record::End { sid, .. } => self.end(*sid, off),
            Record::Gap {
                sid,
                start_wall: a,
                end_wall: b,
                ..
            } if *a > 0 && b >= a && self.net.contains(&(self.boot, *sid)) => {
                self.h.gaps.push((*a, *b));
            }
            _ => {}
        }
    }

    fn open(&mut self, sid: u64, skind: StreamKind, req: u64, meta: &[u8]) {
        let b = self.boot;
        if matches!(
            skind,
            StreamKind::HttpReq | StreamKind::HttpResp | StreamKind::WsUp | StreamKind::WsDown
        ) {
            // Any GAP on these covers a missed call, also over a body the pass never opened.
            self.net.insert((b, sid));
        }
        let m: Value = serde_json::from_slice(meta).unwrap_or_default();
        let role = match skind {
            StreamKind::HttpReq if m["method"] == "POST" => Role::Req,
            StreamKind::HttpResp if m["decoded"] != false => {
                let pending = self
                    .open
                    .get(&(b, req))
                    .is_some_and(|s| s.role == Role::Req);
                if !(self.cand.remove(&(b, req)) || pending) {
                    return;
                }
                let sse = |h: &Value| {
                    let ct = h[0]
                        .as_str()
                        .is_some_and(|n| n.eq_ignore_ascii_case("content-type"));
                    ct && h[1]
                        .as_str()
                        .is_some_and(|v| v.to_lowercase().contains("event-stream"))
                };
                Role::Resp(m["headers"].as_array().is_some_and(|h| h.iter().any(sse)))
            }
            StreamKind::WsUp => Role::Up,
            StreamKind::WsDown => Role::Down,
            _ => return,
        };
        let pend = role == Role::Req;
        let st = St {
            role,
            buf: Vec::new(),
            next: 0,
            hole: false,
            pend,
            msgs: 0,
            acc: String::new(),
        };
        self.open.insert((b, sid), st);
    }

    fn data(&mut self, sid: u64, rec_off: u64, at: (u64, u8), bytes: &[u8]) {
        let (off, flags) = at;
        let Some(st) = self.open.get_mut(&(self.boot, sid)) else {
            return;
        };
        if matches!(st.role, Role::Up | Role::Down) && !st.pend {
            // A message starts where its first byte is; a hole before it is not its own.
            (st.pend, st.next, st.hole) = (true, off, false);
        }
        let on = off == st.next;
        st.next = off.saturating_add(bytes.len() as u64);
        if !on {
            st.hole = true;
            st.buf.clear();
        }
        match st.role {
            Role::Req if st.pend && !st.hole => {
                st.buf.extend_from_slice(bytes);
                if st
                    .buf
                    .trim_ascii_start()
                    .first()
                    .is_some_and(|&b| b != b'{')
                {
                    (st.pend, st.buf) = (false, Vec::new());
                }
            }
            Role::Resp(true) => {
                st.buf.extend_from_slice(bytes);
                let (acc, keys) = (&mut st.acc, &mut self.h.answers);
                lines(&mut st.buf, &mut |v| event(v, acc, keys));
            }
            Role::Resp(false) | Role::Up | Role::Down if !st.hole => {
                st.buf.extend_from_slice(bytes)
            }
            _ => {}
        }
        if flags & FLAG_FIN != 0 && matches!(st.role, Role::Up | Role::Down) {
            let (body, hole, n) = (std::mem::take(&mut st.buf), st.hole, st.msgs);
            (st.pend, st.msgs, st.hole) = (false, n + 1, false);
            let (role, op) = (st.role, flags >> 4);
            if role == Role::Down && op == 1 && !hole {
                self.reply(&body);
            } else if role == Role::Up && op == 1 {
                let id = (self.boot, sid, Some(n));
                self.message(id, rec_off, (!hole).then_some(&body));
            }
        }
    }

    fn end(&mut self, sid: u64, off: u64) {
        let k = (self.boot, sid);
        let Some(mut st) = self.open.remove(&k) else {
            return;
        };
        match st.role {
            Role::Req if st.pend => {
                let body = (!st.hole).then_some(&st.buf);
                if self.message((k.0, sid, None), off, body) {
                    self.cand.insert(k);
                }
            }
            Role::Resp(true) => {
                st.buf.push(b'\n');
                let (acc, keys) = (&mut st.acc, &mut self.h.answers);
                lines(&mut st.buf, &mut |v| event(v, acc, keys));
                texts(&mut st.acc, keys);
            }
            Role::Resp(false) if !st.hole => self.reply(&st.buf),
            Role::Up if st.pend => self.h.undecoded += 1,
            _ => {}
        }
    }

    /// A whole response body or server message: its answers go in the one set.
    fn reply(&mut self, body: &[u8]) {
        let (mut acc, keys) = (String::new(), &mut self.h.answers);
        match serde_json::from_slice::<Value>(body) {
            Ok(Value::Array(a)) => a.iter().for_each(|v| event(v, &mut acc, keys)),
            Ok(v) => event(&v, &mut acc, keys),
            Err(_) => {}
        }
        texts(&mut acc, keys);
    }

    /// A finished request body (`None`: it has a hole). True when its reply is to be read: a
    /// model request, or a body that looks like JSON but could not be decoded (the call is real
    /// and its answer is still an answer).
    fn message(&mut self, id: Id, off: u64, body: Option<&Vec<u8>>) -> bool {
        let parsed = body.and_then(|b| serde_json::from_slice::<Value>(b).ok());
        let Some(Value::Object(m)) = parsed else {
            let looks = body.is_none_or(|b| b.trim_ascii_start().first() == Some(&b'{'));
            self.h.undecoded += u64::from(looks);
            return looks;
        };
        let Some(items) = ["messages", "input", "contents"]
            .iter()
            .find_map(|k| m.get(*k)?.as_array())
        else {
            return false;
        };
        let turns: Vec<Turn> = items.iter().filter_map(|t| turn(t.as_object()?)).collect();
        let ds: Vec<u64> = turns
            .iter()
            .map(|t| {
                (t.iter()).fold(0, |d, k| {
                    mix(
                        d,
                        u64::from_le_bytes(k.0[..8].try_into().unwrap_or_default()),
                    )
                })
            })
            .collect();
        self.h.seen += 1;
        if ds.is_empty() && m.get("previous_response_id").is_some_and(|v| !v.is_null()) {
            self.h.prev_response += 1;
            return true;
        }
        let (c, k, first, waiting) = self.attach(&ds);
        let turns = turns.into_iter().enumerate().skip(k).collect();
        self.h.reqs.push(Rq {
            id,
            off,
            chain: c,
            first,
            waiting,
            turns,
        });
        true
    }

    /// The chain a request with these turn digests joins, how many of its turns that chain
    /// already had, whether the request opened the chain, and (for a chain opened with turns
    /// already resent) how many empty chains were waiting then.
    ///
    /// Worked example. Request 1 resends nothing: it opens chain 0 (empty, anchored). Request 2
    /// resends [A1]: no chain's last list is a prefix of it, so it opens chain 1 and may join chain
    /// 0. Which empty chain, if any, is decided in [`judge`], where the thread ids are known: the
    /// one of its own thread. Request 3 resends [A1, A2]: chain 1's last list [A1] is a prefix, so
    /// it joins chain 1 and only A2 is new. The prefix is matched by a rolling hash of the turn
    /// digests, longest first, so a request may extend its chain by more than one answer (a
    /// skipped request in between).
    fn attach(&mut self, ds: &[u64]) -> (usize, usize, bool, usize) {
        let ph: Vec<u64> = std::iter::once(0)
            .chain(ds.iter().scan(0, |p, &d| {
                *p = mix(*p, d);
                Some(*p)
            }))
            .collect();
        let hit = (1..=ds.len()).rev().find_map(|k| {
            let list = self.by_last.get(&ph[k])?;
            list.iter()
                .rev()
                .find(|&&c| self.h.chains[c].last == ds[..k])
                .map(|&c| (c, k))
        });
        let (c, k, first, waiting) = match hit {
            Some((c, k)) => (c, k, false, 0),
            None => {
                let anchored = ds.is_empty();
                let waiting = self.h.zeros.len();
                self.h.chains.push(Ch {
                    last: Vec::new(),
                    anchored,
                });
                let c = self.h.chains.len() - 1;
                if anchored {
                    self.h.zeros.push(c);
                }
                (c, 0, true, if anchored { 0 } else { waiting })
            }
        };
        if !ds.is_empty() {
            if k > 0 {
                self.by_last.entry(ph[k]).or_default().retain(|&x| x != c);
            }
            self.h.chains[c].last = ds.to_vec();
            self.by_last.entry(ph[ds.len()]).or_default().push(c);
        }
        (c, k, first, waiting)
    }

    /// The call table and what the history pass kept.
    pub fn finish(self) -> (CallTable, Hist) {
        (self.calls.finish(), self.h)
    }
}

/// The verdict: every new turn against the whole answer set.
pub fn judge(h: &Hist, t: &CallTable, covers: &[Cover]) -> Check {
    let rows: HashMap<Id, (Option<&str>, Option<u64>)> = (t.rows.iter())
        .filter(|r| r.leg == Leg::Wire)
        .filter_map(|r| Some(((r.boot, r.sid?, r.msg), (r.thread.as_deref(), r.wall_start))))
        .collect();
    let info = |r: &Rq| rows.get(&r.id).copied().unwrap_or_default();
    // A chain that opened with turns already resent continues the empty chain of its own thread
    // (the same id, or no id on both), the oldest one not yet continued; else it stays its own.
    let n = h.chains.len();
    let (mut home, mut free): (Vec<usize>, Vec<bool>) = ((0..n).collect(), vec![true; n]);
    let mut thread: Vec<Option<&str>> = vec![None; n];
    for r in h.reqs.iter().filter(|r| r.first) {
        thread[r.chain] = info(r).0;
        let mine = |z: &&usize| free[**z] && thread[**z] == thread[r.chain];
        if let Some(&z) = h.zeros[..r.waiting].iter().find(mine) {
            (free[z], home[r.chain]) = (false, z);
        }
    }
    let mut names = vec![String::new(); n];
    for r in h
        .reqs
        .iter()
        .filter(|r| r.first && home[r.chain] == r.chain)
    {
        let own = format!("chain-{}", r.chain + 1);
        names[r.chain] = thread[r.chain].map_or(own, String::from);
    }
    // A second chain of a thread that starts unanchored is rewritten history (compaction).
    let (mut seen, mut cut): (HashSet<&str>, HashMap<&str, usize>) = Default::default();
    for (c, name) in names.iter().enumerate().filter(|(c, _)| home[*c] == *c) {
        if !seen.insert(name) && !h.chains[c].anchored {
            cut.entry(name).or_insert(c);
        }
    }
    let mut chk = Check::new("history_complete", None, Res::Pass, "");
    let (mut checked, mut comp) = (0u64, 0u64);
    let (mut missing_call, mut missing_text, mut unanchored) = (0u64, 0u64, 0u64);
    let mut th: BTreeMap<&str, (Res, u64)> = BTreeMap::new();
    let mut last: HashMap<usize, u64> = HashMap::new();
    for r in &h.reqs {
        let c = home[r.chain];
        let (name, at) = (names[c].as_str(), info(r).1);
        let prev = last.get(&c).copied();
        last.extend(at.map(|a| (c, a)));
        let e = th.entry(name).or_insert((Res::Pass, 0));
        if cut.get(name).is_some_and(|&x| c >= x) {
            comp += r.turns.len() as u64;
            continue;
        }
        let first = r.first && c == r.chain;
        for (pos, keys) in &r.turns {
            e.1 += 1;
            checked += 1;
            let miss: Vec<bool> = (keys.iter())
                .filter(|k| !h.answers.contains(&k.0))
                .map(|k| k.1)
                .collect();
            if miss.is_empty() {
                continue;
            }
            // The time between the chain's previous request and this one (this one's own time
            // when the chain has no earlier one with a time).
            let (lo, hi) = (prev.or(at).unwrap_or(0), at.unwrap_or(u64::MAX));
            let overlaps = |(a, b): (u64, u64)| a <= hi && b >= lo;
            let named = (covers.iter().any(|c| overlaps((c.start_wall, c.end_wall))))
                || h.gaps.iter().any(|&g| overlaps(g));
            let (res, what) = if first && !h.chains[c].anchored {
                unanchored += 1;
                (Res::Warn, "history from before the record")
            } else if miss.contains(&true) {
                missing_call += 1;
                let res = if named { Res::Named } else { Res::Fail };
                (res, "never recorded as a response")
            } else {
                missing_text += 1;
                (Res::Warn, "text not found in any response")
            };
            e.0 = e.0.max(res);
            let kind = if miss.contains(&true) {
                "tool call"
            } else {
                "text"
            };
            let w = at.map_or("an unknown time".into(), wall);
            let why = format!(
                "thread {name}: request on stream {} at {w} resends answer {} ({kind}): {what}",
                r.id.1,
                pos + 1
            );
            chk.raise(res, &why, Some(At(r.off, r.id.1, at.unwrap_or(0))));
        }
    }
    let label = |(res, n): &(Res, u64)| match (*res, *n) {
        (Res::Pass, 0) => "skip",
        (Res::Pass, _) => "pass",
        (res, _) => res.name(),
    };
    let threads: Map<String, Value> = th
        .iter()
        .map(|(k, v)| (k.to_string(), label(v).into()))
        .collect();
    let live = threads.values().filter(|v| *v != "skip").count();
    let (prev, bad) = (h.prev_response, h.undecoded);
    let skipped = json!({"previous_response_id": prev, "compacted": comp, "undecoded": bad});
    let tail: Vec<String> = [
        (prev, "previous_response_id"),
        (comp, "compacted"),
        (bad, "undecoded"),
    ]
    .iter()
    .filter(|s| s.0 > 0)
    .map(|s| format!("{} {}", s.0, s.1))
    .collect();
    let tail = if tail.is_empty() {
        String::new()
    } else {
        format!(" (skipped: {})", tail.join(", "))
    };
    if checked == 0 {
        let at = cut
            .values()
            .min()
            .and_then(|&c| h.reqs.iter().find(|r| r.chain == c && r.first));
        let when = at
            .and_then(|r| info(r).1)
            .map_or("an unknown time".into(), wall);
        let why = if h.seen == 0 && prev == 0 && bad == 0 {
            "nothing decrypted to the model host".into()
        } else if comp > 0 {
            format!("history rewritten at {when} (compaction)")
        } else if prev > 0 {
            "history held by the provider".into()
        } else if bad > 0 {
            format!("{bad} request bodies could not be read")
        } else {
            "no request resent an earlier answer".to_string()
        };
        chk = Check::new("history_complete", None, Res::Skip, &why);
    } else if chk.result == Res::Pass {
        chk.why = format!("{checked} answers checked in {live} threads{tail}");
    } else {
        chk.why += &tail;
    }
    let facts = json!({"checked": checked, "missing_call": missing_call,
        "missing_text": missing_text, "unanchored": unanchored, "skipped": skipped,
        "threads": threads, "chains": home.iter().enumerate().filter(|(i, c)| i == *c).count()});
    chk.facts = facts.as_object().cloned().unwrap_or_default();
    chk
}

/// The coverage sentence. It never says everything was recorded.
pub fn line(c: &Check) -> String {
    if c.result == Res::Skip {
        return format!("History check: skipped, {}.", c.why);
    }
    let n = |k: &str| c.facts.get(k).and_then(Value::as_u64).unwrap_or(0);
    let threads = c.facts.get("threads").and_then(Value::as_object);
    let live = threads.map_or(0, |t| t.values().filter(|v| *v != "skip").count());
    format!(
        "History check: {} answers checked in {live} threads, {} missed call(s).",
        n("checked"),
        n("missing_call")
    )
}
