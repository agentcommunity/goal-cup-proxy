//! The native leg (PRD-09 sections 4 and 5): Codex rollouts and Claude Code transcripts read back
//! out of the record's `native` streams, Claude Code agent labels out of its `snapshot` streams.
//! Bytes are deduped by (path, gen, offset) across boots (PRD-01 section 15 point 5); only lines
//! that end in a newline are parsed, and a hole drops the partial line. Only ids, labels, model
//! names and numbers leave this file. Vendor pages: Codex `protocol.rs` at rust-v0.165.0; Claude
//! Code "Explore the .claude directory" and "Subagents".

use std::collections::{BTreeMap, BTreeSet, HashMap};

use goalcup_format::{Record, StreamKind};
use serde_json::{Map, Value};

use super::hints::{self, Reader};
use super::shapes::{model_name, safe_code};
use super::{CallRow, Ctx, Kind, Leg, Outcome, Thread, Tokens};

pub(super) struct NativeLeg {
    pub(super) rows: Vec<CallRow>,       // one per native call
    pub(super) ids: Vec<Option<String>>, // same length as rows: Claude Code requestId, else None
    pub(super) threads: Vec<Thread>,     // every thread read, reason None
    pub(super) children: Option<BTreeMap<String, Vec<String>>>, // Some when any file was read
    pub(super) read: bool, // a reader of this connector read at least one session file
}

/// A Claude Code call: its row, usage counts (largest per member) and `requestId`.
type Msg = (CallRow, BTreeMap<String, u64>, Option<String>);

/// The native sink. No `Debug`: it holds file bytes (PRD-17 VR2).
#[derive(Default)]
pub(super) struct Native {
    streams: HashMap<(u64, u64), Src>,
    files: BTreeMap<(String, u64), File>,
    codex: Vec<CallRow>,
    /// Claude Code calls by `message.id`.
    msgs: BTreeMap<String, Msg>,
    /// Claude Code agent id to its session and the last meta snapshot's `agentType`.
    agents: BTreeMap<String, (String, Option<String>)>,
}

/// What a stream carries: a session file (path, gen), or a meta snapshot (`None` after a hole).
enum Src {
    File((String, u64)),
    Snap(String, Option<Vec<u8>>),
}

type Meta = (
    Option<String>,
    Option<String>,
    Option<u32>,
    Option<String>,
    Option<u64>,
);

/// One generation of a session file: next expected offset, partial line (with the boot of its
/// first byte), and for Codex the `session_meta` (id, parent, depth, label, time), the latest
/// `turn_context`, the largest running total and the threads it spawned.
#[derive(Default)]
struct File {
    codex: bool,
    next: u64,
    line: Vec<u8>,
    lboot: u64,
    skip: bool,
    meta: Option<Meta>,
    turn: (Option<String>, Option<String>),
    top: Option<u64>,
    spawned: Vec<String>,
}

impl Native {
    /// Takes one record of the boot `ctx` names; reads only `native` and `snapshot` streams.
    pub(super) fn push(&mut self, ctx: &Ctx, rec: &Record) {
        let key = |sid: &u64| (ctx.boot, *sid);
        match rec {
            Record::Open {
                sid, skind, meta, ..
            } => {
                let m: Value = serde_json::from_slice(meta).unwrap_or(Value::Null);
                let n = |k: &str| m[k].as_u64().unwrap_or(0);
                let Some(p) = m["path"].as_str() else { return };
                let src = match (skind, hints::reader(ctx.connector.as_deref(), p)) {
                    (StreamKind::Snapshot, Some(Reader::ClaudeMeta)) => {
                        Src::Snap(p.to_string(), Some(Vec::new()))
                    }
                    (
                        StreamKind::Native,
                        Some(r @ (Reader::CodexRollout | Reader::ClaudeTranscript)),
                    ) => {
                        let (codex, next) = (r == Reader::CodexRollout, n("from"));
                        let f = File {
                            codex,
                            next,
                            ..File::default()
                        };
                        self.files.entry((p.to_string(), n("gen"))).or_insert(f);
                        Src::File((p.to_string(), n("gen")))
                    }
                    _ => return,
                };
                self.streams.insert(key(sid), src);
            }
            Record::Data {
                sid, off, bytes, ..
            } => match self.streams.get_mut(&key(sid)) {
                Some(Src::File(file)) => {
                    let Some(f) = self.files.get_mut(file) else {
                        return;
                    };
                    for (boot, line) in f.data(ctx.boot, *off, bytes) {
                        let Ok(v) = serde_json::from_slice::<Value>(&line) else {
                            continue;
                        };
                        let mut row =
                            CallRow::new(Leg::Native, ctx.entry.clone(), boot, Kind::Call);
                        (row.path, row.outcome, row.whole) =
                            (Some(file.0.clone()), Outcome::Ok, true);
                        row.wall_start = v["timestamp"].as_str().and_then(ts);
                        if f.codex {
                            self.codex.extend(f.rollout(row, &v));
                        } else {
                            claude(&mut self.msgs, row, &v);
                        }
                    }
                }
                Some(Src::Snap(_, buf)) => match buf {
                    Some(b) if *off == b.len() as u64 => b.extend_from_slice(bytes),
                    _ => *buf = None,
                },
                None => {}
            },
            Record::Gap { sid, lost, .. } if *lost != 1 => match self.streams.get_mut(&key(sid)) {
                Some(Src::File(file)) => self.files.get_mut(file).into_iter().for_each(File::hole),
                Some(Src::Snap(_, buf)) => *buf = None,
                None => {}
            },
            Record::End { sid, .. } => {
                if let Some(Src::Snap(path, Some(buf))) = self.streams.remove(&key(sid))
                    && let Some((id, session)) = agent(&path, ".meta.json")
                {
                    let v = serde_json::from_slice(&buf).unwrap_or(Value::Null);
                    self.agents.insert(id, (session, id_at(&v, "/agentType")));
                }
            }
            _ => {}
        }
    }

    /// The native rows (in time order), threads and children of the entry.
    pub(super) fn finish(self, _ctx: &Ctx) -> NativeLeg {
        let (mut owner, mut threads) = (HashMap::new(), BTreeMap::new());
        let mut kids: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for ((path, _), f) in self.files {
            let (id, parent, depth, label) = match (f.codex, f.meta) {
                (true, Some((id, p, d, l, _))) => (id.or_else(|| file_uuid(&path)), p, d, l),
                (true, None) => (file_uuid(&path), None, None, None),
                _ => match agent(&path, ".jsonl") {
                    Some((id, s)) => (Some(id), Some(s), Some(1), None),
                    None => (session_of(&path), None, None, None),
                },
            };
            let Some(id) = id else { continue };
            if let Some(p) = &parent {
                kids.entry(p.clone()).or_default().insert(id.clone());
            }
            kids.entry(id.clone()).or_default().extend(f.spawned);
            let new = Thread {
                reason: None,
                ..Thread::new(&id)
            };
            let t = threads.entry(id.clone()).or_insert(new);
            (t.parent, t.depth) = (t.parent.take().or(parent.clone()), t.depth.or(depth));
            t.label = t.label.take().or(label);
            owner.insert(path, (id, parent));
        }
        // A model of `<synthetic>` or an all-zero usage is not a call.
        let claude = self.msgs.into_values().filter_map(|(r, c, rid)| {
            let real = r.model.as_deref() != Some("<synthetic>") && c.values().any(|&n| n > 0);
            let (tokens, extra) = split(c, CLAUDE, 0);
            real.then_some((CallRow { tokens, extra, ..r }, rid))
        });
        let all = self.codex.into_iter().map(|r| (r, None)).chain(claude);
        let mut rows: Vec<_> = all
            .filter_map(|(r, rid)| {
                let (id, parent) = owner.get(r.path.as_deref()?)?.clone();
                threads.get_mut(&id)?.native_calls += 1;
                let at = r.wall_start;
                Some((
                    CallRow {
                        thread: Some(id),
                        parent,
                        wall_end: at,
                        ..r
                    },
                    rid,
                ))
            })
            .collect();
        for (id, (session, label)) in self.agents {
            if let Some(t) = threads.get_mut(&id) {
                t.label = label.or(t.label.take());
            }
            kids.entry(session).or_default().insert(id);
        }
        kids.retain(|_, v| !v.is_empty());
        rows.sort_by_key(|(r, _)| r.wall_start);
        let (rows, ids) = rows.into_iter().unzip();
        let read = !owner.is_empty();
        let list = |(k, v): (String, BTreeSet<String>)| (k, v.into_iter().collect());
        let children = read.then(|| kids.into_iter().map(list).collect());
        let threads = threads.into_values().collect();
        NativeLeg {
            rows,
            ids,
            threads,
            children,
            read,
        }
    }
}

impl File {
    fn hole(&mut self) {
        self.line.clear();
        self.skip = true;
    }

    /// Takes a piece at file offset `off`: trims bytes already seen, drops the partial line after
    /// a hole, and returns the lines it completes with the boot of their first byte.
    fn data(&mut self, b: u64, off: u64, bytes: &[u8]) -> Vec<(u64, Vec<u8>)> {
        let end = off.saturating_add(bytes.len() as u64);
        if end <= self.next {
            return Vec::new();
        }
        let mut piece = &bytes[usize::try_from(self.next.saturating_sub(off)).unwrap_or(0)..];
        if off > self.next {
            self.hole();
        }
        self.next = end;
        if self.skip {
            let Some(i) = piece.iter().position(|&c| c == b'\n') else {
                return Vec::new();
            };
            (piece, self.skip) = (&piece[i + 1..], false);
        }
        let mut out = Vec::new();
        for part in piece.split_inclusive(|&c| c == b'\n') {
            if self.line.is_empty() {
                self.lboot = b;
            }
            self.line.extend_from_slice(part);
            if part.ends_with(b"\n") {
                out.push((self.lboot, std::mem::take(&mut self.line)));
            }
        }
        out
    }

    /// One Codex `RolloutLine` (`timestamp`, `type`, `payload`); `Some` for a call. In a child
    /// file, lines timed before its own `session_meta` are copied parent records and are skipped.
    fn rollout(&mut self, row: CallRow, v: &Value) -> Option<CallRow> {
        let (p, at) = (&v["payload"], row.wall_start);
        let child = self.meta.as_ref().filter(|m| m.1.is_some());
        match (v["type"].as_str()?, p["type"].as_str()) {
            ("session_meta", _) if self.meta.is_none() => {
                let s = |k: &str| id_at(p, &format!("/source/subagent/thread_spawn/{k}"));
                let depth = p.pointer("/source/subagent/thread_spawn/depth");
                let depth = depth
                    .and_then(Value::as_u64)
                    .and_then(|d| u32::try_from(d).ok());
                self.meta = Some((
                    id_at(p, "/id"),
                    s("parent_thread_id"),
                    depth,
                    s("agent_path"),
                    at,
                ));
            }
            _ if child.is_some_and(|m| at < m.4) => {}
            ("turn_context", _) => {
                self.turn = (model_name(p.get("model")), safe_code(p.get("effort")))
            }
            ("event_msg", Some("token_count")) => {
                let total = p.pointer("/info/total_token_usage/total_tokens")?.as_u64();
                let last = p.pointer("/info/last_token_usage")?.as_object()?;
                // A repeat with the same or a smaller running total is no new call.
                if total > self.top {
                    self.top = total;
                    let (model, effort) = self.turn.clone();
                    let (tokens, extra) = split(counts(last), CODEX, 1);
                    return Some(CallRow {
                        model,
                        effort,
                        tokens,
                        extra,
                        ..row
                    });
                }
            }
            ("event_msg", Some("item_completed")) => {
                let item = &p["item"];
                if item["type"] == "CollabAgentToolCall" && item["tool"] == "spawn_agent" {
                    let ids = item["receiver_thread_ids"].as_array().into_iter().flatten();
                    self.spawned
                        .extend(ids.filter_map(|i| hints::id(i.as_str()?)));
                }
            }
            _ => {}
        }
        None
    }
}

/// One Claude Code transcript line: an `assistant` line is part of the call its `message.id`
/// names, whose usage it repeats; per member, the largest value wins.
fn claude(msgs: &mut BTreeMap<String, Msg>, row: CallRow, v: &Value) -> Option<()> {
    let m = (v["type"] == "assistant").then_some(&v["message"])?;
    let c = counts(m["usage"].as_object()?);
    let (at, rid) = (row.wall_start, id_at(v, "/requestId"));
    let (r, have, id) = msgs
        .entry(id_at(m, "/id")?)
        .or_insert((row, BTreeMap::new(), None));
    for (k, n) in c {
        let e = have.entry(k).or_insert(n);
        *e = (*e).max(n);
    }
    (r.wall_start, *id) = (r.wall_start.max(at), id.take().or(rid));
    r.model = model_name(m.get("model")).or(r.model.take());
    Some(())
}

/// A usage object's top-level integer members named `…tokens`, `…Tokens` or `…TokenCount`.
fn counts(u: &Map<String, Value>) -> BTreeMap<String, u64> {
    let named =
        |k: &str| k.ends_with("tokens") || k.ends_with("Tokens") || k.ends_with("TokenCount");
    let ok = |k: &String| named(k) && hints::id(k).is_some();
    u.iter()
        .filter(|(k, _)| ok(k))
        .filter_map(|(k, v)| Some((k.clone(), v.as_u64()?)))
        .collect()
}

/// Usage member names per the token table: input, cached, cache write, output, reasoning.
const CODEX: [&str; 5] = [
    "input_tokens",
    "cached_input_tokens",
    "",
    "output_tokens",
    "reasoning_output_tokens",
];
const CLAUDE: [&str; 5] = [
    "input_tokens",
    "cache_read_input_tokens",
    "cache_creation_input_tokens",
    "output_tokens",
    "",
];

/// The five numbers by member name per the token table ("" for one the source never gives),
/// the rest as `extra`; with `less` 1 the cached tokens are inside `input` and come out of it.
fn split(
    mut c: BTreeMap<String, u64>,
    names: [&str; 5],
    less: u64,
) -> (Tokens, BTreeMap<String, u64>) {
    let [input, cached, cache_write, output, reasoning] = names.map(|n| c.remove(n));
    let input = input.map(|i| i.saturating_sub(less * cached.unwrap_or(0)));
    (
        Tokens {
            input,
            cached,
            cache_write,
            output,
            reasoning,
        },
        c,
    )
}

/// An id or label at a JSON pointer (see [`hints::id`]).
fn id_at(v: &Value, ptr: &str) -> Option<String> {
    hints::id(v.pointer(ptr)?.as_str()?)
}

/// Claude Code `<p>/<session>/subagents/agent-<id><ext>`: (id, session).
fn agent(path: &str, ext: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = path.split_once(':')?.1.split('/').collect();
    let [_, session, "subagents", file] = parts[..] else {
        return None;
    };
    let id = hints::id(file.strip_prefix("agent-")?.strip_suffix(ext)?)?;
    Some((id, hints::id(session)?))
}

/// The session of a Claude Code main transcript `<p>/<session>.jsonl` (the stem before its first
/// dot, so a set-aside copy names the same session).
fn session_of(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split_once(':')?.1.split('/').collect();
    let [_, file] = parts[..] else { return None };
    hints::id(file.strip_suffix(".jsonl")?.split('.').next()?)
}

/// The thread UUID that ends a Codex rollout file name, for a file read with no `session_meta`.
fn file_uuid(path: &str) -> Option<String> {
    let stem = path.strip_suffix(".jsonl")?;
    let u = stem.get(stem.len().checked_sub(36)?..)?;
    let hex = u.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-');
    (hex && u.matches('-').count() == 4).then(|| u.to_string())
}

/// An RFC 3339 UTC time (`Z`, optional fraction) as ns since the Unix epoch.
fn ts(s: &str) -> Option<u64> {
    let num = |x: &str| {
        x.parse::<i64>()
            .ok()
            .filter(|_| x.bytes().all(|c| c.is_ascii_digit()))
    };
    let (date, time) = s.strip_suffix('Z')?.split_once('T')?;
    let (hms, frac) = time.split_once('.').unwrap_or((time, "0"));
    let f: Option<Vec<i64>> = date.split('-').chain(hms.split(':')).map(num).collect();
    let [y, m, d, h, mi, se] = f?[..] else {
        return None;
    };
    let ok = (1..=12).contains(&m) && (1..=31).contains(&d) && h < 24 && mi < 60 && se <= 60;
    let ns = num(frac).filter(|_| ok && frac.len() <= 9)? * 10i64.pow(9 - frac.len() as u32);
    // Days from civil (H. Hinnant): years start in March, eras of 400 years.
    let y = if m <= 2 { y - 1 } else { y };
    let (era, mp) = (y.div_euclid(400), (m + 9) % 12);
    let yoe = y - era * 400;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + (153 * mp + 2) / 5 + d - 1;
    let secs = (((days - 719_468) * 24 + h) * 60 + mi) * 60 + se;
    u64::try_from(secs * 1_000_000_000 + ns).ok()
}
