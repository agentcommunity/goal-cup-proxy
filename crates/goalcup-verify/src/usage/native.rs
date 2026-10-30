//! The native leg (PRD-09 sections 4 and 5): Codex rollouts and Claude Code transcripts read back
//! out of the record's `native` streams, Claude Code agent labels out of its `snapshot` streams.
//! Bytes are deduped by (path, gen, offset) across boots (PRD-01 section 15 point 5); only lines
//! that end in a newline are parsed, and a hole drops the partial line. Rows carry their session
//! file `path` (U1 allows it, but it can name the participant's folders, so section 7 keeps it out
//! of the output files); otherwise only ids, labels, model names and numbers leave this file.
//! Vendor pages: Codex `protocol.rs` at rust-v0.165.0; Claude Code "Explore the .claude
//! directory" and "Subagents".

use std::collections::{BTreeMap, BTreeSet, HashMap};

use goalcup_format::{Record, StreamKind};
use serde_json::Value;

use super::hints::{self, Reader};
use super::shapes::{extra, less, model_name, safe_code};
use super::{CallRow, Ctx, Kind, Leg, Outcome, Reason, Thread, Tokens};

pub(super) struct NativeLeg {
    /// One per native call, with its Claude Code `requestId` (else `None`).
    pub(super) rows: Vec<(CallRow, Option<String>)>,
    /// Every thread a file, a spawn or a meta snapshot names. Readable (reason `None`) only when it
    /// owns a read session file; the others keep their entry and parent with [`Reason::NoNative`].
    pub(super) threads: Vec<Thread>,
    /// `Some` when any session file named its thread.
    pub(super) children: Option<BTreeMap<String, Vec<String>>>,
    /// Codex `token_count` repeats (no larger running total), skipped.
    pub(super) repeats_skipped: u64,
    /// Child rollout lines timed before the child's own `session_meta` (copied parent records).
    pub(super) copied_skipped: u64,
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
    /// Claude Code agent id to its session and the last `agentType` a meta snapshot gave.
    agents: BTreeMap<String, (String, Option<String>)>,
}

/// What a stream carries: a session file (path, gen), or a meta snapshot (`None` after a hole).
enum Src {
    File((String, u64)),
    Snap(String, Option<Vec<u8>>),
}

/// A Codex `session_meta`: thread id, parent, depth, `agent_path` label and its time.
struct Meta {
    id: Option<String>,
    parent: Option<String>,
    depth: Option<u32>,
    label: Option<String>,
    at: Option<u64>,
}

/// One generation of a session file: next expected offset, partial line (with the boot of its
/// first byte), and for Codex the `session_meta`, the latest `turn_context` model and effort, the
/// largest running total, the threads it spawned and the lines it skipped.
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
    repeats: u64,
    copied: u64,
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
                let Some(p) = m["path"].as_str() else { return };
                let file = (p.to_string(), m["gen"].as_u64().unwrap_or(0));
                let src = match (skind, hints::reader(ctx.connector.as_deref(), p)) {
                    (StreamKind::Snapshot, Some(Reader::ClaudeMeta)) => {
                        Src::Snap(file.0, Some(Vec::new()))
                    }
                    (StreamKind::Native, Some(r)) if r != Reader::ClaudeMeta => {
                        let f = File {
                            codex: r == Reader::CodexRollout,
                            next: m["from"].as_u64().unwrap_or(0),
                            ..File::default()
                        };
                        self.files.entry(file.clone()).or_insert(f);
                        Src::File(file)
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
                        let at = v["timestamp"].as_str().and_then(ts);
                        let row = || {
                            let mut r = CallRow::new(Leg::Native, &ctx.entry, boot, Kind::Call);
                            (r.path, r.wall_start) = (Some(file.0.clone()), at);
                            (r.outcome, r.whole) = (Outcome::Ok, true);
                            r
                        };
                        if f.codex {
                            self.codex.extend(f.rollout(&v, at, row));
                        } else {
                            claude(&mut self.msgs, &v, at, row);
                        }
                    }
                }
                Some(Src::Snap(_, buf)) => match buf {
                    Some(b) if *off == b.len() as u64 => b.extend_from_slice(bytes),
                    _ => *buf = None,
                },
                None => {}
            },
            // A hole in a session file shows in `data` by its offset; a snapshot is dropped here.
            Record::Gap { sid, lost, .. } if *lost != 1 => {
                if let Some(Src::Snap(_, buf)) = self.streams.get_mut(&key(sid)) {
                    *buf = None;
                }
            }
            Record::End { sid, .. } => {
                if let Some(Src::Snap(path, Some(buf))) = self.streams.remove(&key(sid))
                    && let (Some(id), Some(session), _) = claude_thread(&path)
                {
                    // A later snapshot that gives no `agentType` keeps the earlier one.
                    let v = serde_json::from_slice(&buf).unwrap_or(Value::Null);
                    let a = self.agents.entry(id).or_insert((session, None));
                    a.1 = id_at(&v, "/agentType").or(a.1.take());
                }
            }
            _ => {}
        }
    }

    /// The native rows, threads and children of the entry. A file that names no thread keeps its
    /// rows, with `thread` `None`, so no native call leaves the count.
    pub(super) fn finish(self) -> NativeLeg {
        let (mut owner, mut threads) = (HashMap::new(), BTreeMap::new());
        let mut kids: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let (mut repeats_skipped, mut copied_skipped) = (0, 0);
        let named = |id: &String| Thread {
            reason: Some(Reason::NoNative),
            ..Thread::new(id)
        };
        for ((path, _), f) in self.files {
            repeats_skipped += f.repeats;
            copied_skipped += f.copied;
            let (id, parent, depth, label) = match f.meta {
                Some(m) => {
                    let depth = m.depth.or(m.parent.is_none().then_some(0));
                    (m.id.or_else(|| file_uuid(&path)), m.parent, depth, m.label)
                }
                None if f.codex => (file_uuid(&path), None, None, None),
                None => {
                    let (id, parent, depth) = claude_thread(&path);
                    (id, parent, depth, None)
                }
            };
            let Some(id) = id else { continue };
            if let Some(p) = &parent {
                kids.entry(p.clone()).or_default().insert(id.clone());
            }
            kids.entry(id.clone()).or_default().extend(f.spawned);
            let t = threads.entry(id.clone()).or_insert_with(|| named(&id));
            t.reason = None;
            (t.parent, t.depth) = (t.parent.take().or(parent.clone()), t.depth.or(depth));
            t.label = t.label.take().or(label);
            // Keyed by path: after a source_changed generation, older rows take the newest one's.
            owner.insert(path, (id, parent));
        }
        for (id, (session, label)) in self.agents {
            let t = threads.entry(id.clone()).or_insert_with(|| named(&id));
            (t.label, t.depth) = (label.or(t.label.take()), t.depth.or(Some(1)));
            kids.entry(session).or_default().insert(id);
        }
        // A thread only a spawn or a meta snapshot names still gets its entry and parent.
        for (p, cs) in &kids {
            for c in cs {
                let t = threads.entry(c.clone()).or_insert_with(|| named(c));
                t.parent.get_or_insert_with(|| p.clone());
            }
        }
        // A model of `<synthetic>` or an all-zero usage is not a call.
        let claude = self.msgs.into_values().filter_map(|(r, c, rid)| {
            let real = r.model.as_deref() != Some("<synthetic>") && c.values().any(|&n| n > 0);
            let (tokens, extra) = split(c, CLAUDE);
            real.then_some((CallRow { tokens, extra, ..r }, rid))
        });
        let all = self.codex.into_iter().map(|r| (r, None)).chain(claude);
        let rows = all.map(|(r, rid)| {
            let (thread, parent) = r.path.as_ref().and_then(|p| owner.get(p)).cloned().unzip();
            let (parent, wall_end) = (parent.flatten(), r.wall_start);
            let r = CallRow {
                thread,
                parent,
                wall_end,
                ..r
            };
            (r, rid)
        });
        kids.retain(|_, v| !v.is_empty());
        let list = |(k, v): (String, BTreeSet<String>)| (k, v.into_iter().collect());
        NativeLeg {
            rows: rows.collect(),
            children: (!owner.is_empty()).then(|| kids.into_iter().map(list).collect()),
            threads: threads.into_values().collect(),
            repeats_skipped,
            copied_skipped,
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

    /// One Codex `RolloutLine` (`timestamp`, `type`, `payload`) timed `at`; `Some` for a call. In
    /// a child file, lines timed before its own `session_meta` are copied parent records: skipped
    /// and counted.
    fn rollout(
        &mut self,
        v: &Value,
        at: Option<u64>,
        row: impl FnOnce() -> CallRow,
    ) -> Option<CallRow> {
        let p = &v["payload"];
        let child = self.meta.as_ref().filter(|m| m.parent.is_some());
        let copied = child.is_some_and(|m| at.zip(m.at).is_some_and(|(a, s)| a < s));
        match (v["type"].as_str()?, p["type"].as_str()) {
            ("session_meta", _) if self.meta.is_none() => {
                let s = |k: &str| id_at(p, &format!("/source/subagent/thread_spawn/{k}"));
                let d = p.pointer("/source/subagent/thread_spawn/depth");
                self.meta = Some(Meta {
                    id: id_at(p, "/id"),
                    parent: s("parent_thread_id"),
                    depth: d.and_then(|d| d.as_u64()?.try_into().ok()),
                    label: s("agent_path"),
                    at,
                });
            }
            _ if copied => self.copied += 1,
            ("turn_context", _) => {
                self.turn = (model_name(p.get("model")), safe_code(p.get("effort")))
            }
            ("event_msg", Some("token_count")) => {
                let total = p.pointer("/info/total_token_usage/total_tokens")?.as_u64();
                let last = p.pointer("/info/last_token_usage")?.as_object()?;
                // A repeat with the same or a smaller running total is no new call.
                if total <= self.top {
                    self.repeats += 1;
                    return None;
                }
                self.top = total;
                let (model, effort) = self.turn.clone();
                let (mut tokens, extra) = split(extra(last, &[]), CODEX);
                tokens.input = less(tokens.input, tokens.cached);
                return Some(CallRow {
                    model,
                    effort,
                    tokens,
                    extra,
                    ..row()
                });
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

/// One Claude Code transcript line timed `at`: an `assistant` line is part of the call its
/// `message.id` names, whose usage it repeats; per member, the largest value wins.
fn claude(
    msgs: &mut BTreeMap<String, Msg>,
    v: &Value,
    at: Option<u64>,
    row: impl FnOnce() -> CallRow,
) -> Option<()> {
    let m = (v["type"] == "assistant").then_some(&v["message"])?;
    let c = extra(m["usage"].as_object()?, &[]);
    let (r, have, id) = msgs
        .entry(id_at(m, "/id")?)
        .or_insert_with(|| (row(), BTreeMap::new(), None));
    for (k, n) in c {
        let e = have.entry(k).or_insert(n);
        *e = (*e).max(n);
    }
    (r.wall_start, *id) = (r.wall_start.max(at), id.take().or(id_at(v, "/requestId")));
    r.model = model_name(m.get("model")).or(r.model.take());
    Some(())
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

/// The five numbers by member name per the token table ("" for one the source never gives), the
/// rest as `extra`.
fn split(mut c: BTreeMap<String, u64>, names: [&str; 5]) -> (Tokens, BTreeMap<String, u64>) {
    let [input, cached, cache_write, output, reasoning] = names.map(|n| c.remove(n));
    let tokens = Tokens {
        input,
        cached,
        cache_write,
        output,
        reasoning,
    };
    (tokens, c)
}

/// An id or label at a JSON pointer (see [`hints::id`]).
fn id_at(v: &Value, ptr: &str) -> Option<String> {
    hints::id(v.pointer(ptr)?.as_str()?)
}

/// The Claude Code thread of a transcript or meta path `<row>:<p>/…`: (id, parent, depth). A file
/// `agent-<id>…` at any depth under `<session>/subagents/` (workflow agents included) is that
/// agent, a child of the session; any other file is a thread of its own, named by its stem
/// before the first dot (a main transcript `<p>/<session>.jsonl`, depth 0, or a set-aside copy).
fn claude_thread(path: &str) -> (Option<String>, Option<String>, Option<u32>) {
    let rel = path.split_once(':').map_or(path, |p| p.1);
    let parts: Vec<&str> = rel.split('/').collect();
    let (dirs, file) = parts.split_at(parts.len() - 1);
    let stem = file[0].split('.').next().unwrap_or_default();
    let session = dirs.windows(2).find(|w| w[1] == "subagents").map(|w| w[0]);
    match (session, stem.strip_prefix("agent-")) {
        (Some(s), Some(id)) => (hints::id(id), hints::id(s), Some(1)),
        _ => (hints::id(stem), None, (dirs.len() == 1).then_some(0)),
    }
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
    let ok = (1970..=2200).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&d) && h < 24 && mi < 60 && se <= 60;
    let ns = num(frac).filter(|_| ok && frac.len() <= 9)? * 10i64.pow(9 - frac.len() as u32);
    // Days from civil (H. Hinnant): years start in March, eras of 400 years.
    let y = if m <= 2 { y - 1 } else { y };
    let (era, mp) = (y.div_euclid(400), (m + 9) % 12);
    let yoe = y - era * 400;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + (153 * mp + 2) / 5 + d - 1;
    let secs = (((days - 719_468) * 24 + h) * 60 + mi) * 60 + se;
    u64::try_from(secs * 1_000_000_000 + ns).ok()
}
