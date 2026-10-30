//! Seam U1 (PRD-09): the usage call table, one row per model call, and the `usage` subcommand.
//!
//! [`Calls`] takes every record of an entry in order and gives the [`CallTable`]: the wire leg
//! (`wire.rs`), the native leg (`native.rs`, by the connector's reader in `hints.rs`) and the join.
//! Build rows, threads and tables from [`CallRow::new`], [`Thread::new`] or
//! [`CallTable::default`], never a full struct literal, so a field added later breaks no caller.
//! Wall times are ns since the Unix epoch (LINK `wall_ns`); a token count of `None` means not said.

mod hints;
mod native;
mod shapes;
mod sse;
mod wire;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::OsString;
use std::process::ExitCode;

use goalcup_format::Record;
use serde_json::Value;

/// Which log a row comes from, the wire or the harness's own session log; also `usage_from`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Leg {
    #[default]
    Wire,
    Native,
}

/// What a row is (section 3): a model `Call`, a `Warmup` (`generate: false`), an `Interrupt`, or
/// `Other` POST to a listed host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Call,
    Warmup,
    Interrupt,
    Other,
}

/// How a call ended (PRD-09 section 3); `code` is a safe error code token, never run text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    RateLimited {
        status: Option<u16>,
        code: Option<String>,
    },
    Error {
        status: Option<u16>,
        code: Option<String>,
    },
    /// The client interrupted it.
    Cancelled,
    /// No terminal event in the record and no named cause.
    #[default]
    Unknown,
}

impl Outcome {
    /// PRD-08's "error": true for `RateLimited` and `Error` only.
    pub fn is_error(&self) -> bool {
        matches!(self, Outcome::RateLimited { .. } | Outcome::Error { .. })
    }
}

/// The join result of one call (PRD-09 sections 5 and 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    /// A wire call and a native call of the same call.
    Matched,
    /// The harness made it; the wire has no `ok` copy.
    NativeOnly,
    /// On the wire with no harness entry (refused retries, side calls, auxiliary threads).
    WireOnly,
    /// An `ok` wire call of a harness with a wire thread hint whose request carries none.
    Unattributed,
}

/// The `conn` meta `kind`, else the `start` hosts' (section 7); `Other` for participant and config.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostKind {
    Provider,
    Gateway,
    Vendor,
    Other,
}

/// The wire shape the request path picks (PRD-09 sections 3 and 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Responses,
    Anthropic,
    Chat,
    Gemini,
}

/// How a wire call travelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Http,
    Ws,
}

/// What the native leg gives: per-call rows with times, per-session totals, or none (PRD-08 3.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeForm {
    PerCall,
    Totals,
    None,
}

/// Why a native leg is not readable (PRD-09 section 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// No reader for this connector's session format.
    NoReader,
    /// No database copy opened, even after the WAL repair.
    SnapshotUnreadable,
    /// A reader found zero native calls while the wire had calls.
    NoNative,
}

/// The five token numbers of one call, `None` meaning not said (never 0): input not read from a
/// cache, `cached` input, `cache_write` input, `output` (reasoning included) and its `reasoning`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: Option<u64>,
    pub cached: Option<u64>,
    pub cache_write: Option<u64>,
    pub output: Option<u64>,
    pub reasoning: Option<u64>,
}

/// One wire request (call, warm-up, interrupt, other) or native call; wire-only fields are `None`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallRow {
    pub leg: Leg,
    /// The entry id, `e` and 16 hex.
    pub entry: String,
    pub boot: u64,
    /// The request stream id.
    pub sid: Option<u64>,
    /// The session file path.
    pub path: Option<String>,
    /// The WebSocket message number on its stream.
    pub msg: Option<u64>,
    pub thread: Option<String>,
    pub parent: Option<String>,
    pub wall_start: Option<u64>,
    /// For a wire call, the reply's end.
    pub wall_end: Option<u64>,
    pub host: Option<String>,
    pub host_kind: Option<HostKind>,
    pub shape: Option<Shape>,
    pub via: Option<Via>,
    pub kind: Kind,
    pub outcome: Outcome,
    /// A safe reason token beside `outcome`, such as a `response.incomplete` reason.
    pub detail: Option<String>,
    /// The request, the reply and, for WebSocket, the message ended `complete` with no GAP.
    pub whole: bool,
    /// The model the reply named, else the requested one.
    pub model: Option<String>,
    /// The request body's `model`.
    pub model_requested: Option<String>,
    pub effort: Option<String>,
    pub tokens: Tokens,
    /// Other numeric usage members whose name ends in `tokens`, `Tokens` or `TokenCount`.
    pub extra: BTreeMap<String, u64>,
    /// The leg counted for this call, never both: a merged total sums rows where this is `Some`.
    pub usage_from: Option<Leg>,
    pub join: Option<Join>,
}

impl CallRow {
    /// A row with every optional field empty, `outcome` `Unknown`, `whole` false.
    pub fn new(leg: Leg, entry: impl Into<String>, boot: u64, kind: Kind) -> Self {
        Self {
            leg,
            entry: entry.into(),
            boot,
            kind,
            ..Self::default()
        }
    }
}

/// One agent or subagent thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thread {
    pub id: String,
    pub parent: Option<String>,
    /// The harness's own agent label; bounded, printable.
    pub label: Option<String>,
    pub depth: Option<u32>,
    /// Native calls read for this thread.
    pub native_calls: u64,
    /// The harness's own call total where it keeps one.
    pub native_total: Option<u64>,
    /// Native calls with no `ok` wire copy.
    pub native_only: u64,
    /// Native calls after this time (the database copy's time) are not known.
    pub native_until: Option<u64>,
    /// Why the native leg is not readable; `None` means readable.
    pub reason: Option<Reason>,
}

impl Thread {
    /// True when the native leg was read.
    pub fn readable(&self) -> bool {
        self.reason.is_none()
    }

    /// A thread with nothing read: not readable, [`Reason::NoReader`].
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            parent: None,
            label: None,
            depth: None,
            native_calls: 0,
            native_total: None,
            native_only: 0,
            native_until: None,
            reason: Some(Reason::NoReader),
        }
    }
}

/// The call table of one entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallTable {
    pub rows: Vec<CallRow>,
    /// The threads read.
    pub threads: Vec<Thread>,
    /// Parent thread id to its children; `None` when no parentage was read (independent of
    /// `reason`), and then `subagents_present` skips.
    pub children: Option<BTreeMap<String, Vec<String>>>,
    /// Why the native leg is not readable; `None` means readable.
    pub reason: Option<Reason>,
    pub native_form: NativeForm,
    /// Calls that fell back to the time join.
    pub joined_by_time: u64,
    /// Codex `token_count` repeats skipped (no larger running total; PRD-09 section 4).
    pub repeats_skipped: u64,
    /// Child rollout lines skipped as copied parent records (timed before the child's own
    /// `session_meta`).
    pub copied_skipped: u64,
}

impl Default for CallTable {
    /// The table of an entry with no reader: [`Reason::NoReader`], children not read.
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            threads: Vec::new(),
            children: None,
            reason: Some(Reason::NoReader),
            native_form: NativeForm::None,
            joined_by_time: 0,
            repeats_skipped: 0,
            copied_skipped: 0,
        }
    }
}

impl CallTable {
    /// True when the native leg was read.
    pub fn readable(&self) -> bool {
        self.reason.is_none()
    }
}

/// A host kind by its record name.
fn host_kind(s: Option<&str>) -> Option<HostKind> {
    match s? {
        "provider" => Some(HostKind::Provider),
        "gateway" => Some(HostKind::Gateway),
        "vendor" => Some(HostKind::Vendor),
        "other" => Some(HostKind::Other),
        _ => None,
    }
}

/// What the sink knows of the boot being read: entry, boot, wall base and the `start` event.
#[derive(Debug, Default)]
struct Ctx {
    entry: String,
    connector: Option<String>,
    boot: u64,
    /// (wall_ns, mono_ns) of the last PART or LINK of this boot.
    base: Option<(u64, u64)>,
    /// The `start` event's hosts and kinds, wildcards as given.
    hosts: Vec<(String, Option<HostKind>)>,
}

impl Ctx {
    /// A record time of this boot as wall ns, by the nearest PART or LINK before it.
    fn wall(&self, t: u64) -> Option<u64> {
        let (w, m) = self.base?;
        u64::try_from(i128::from(w) + i128::from(t) - i128::from(m)).ok()
    }

    /// `Some(kind)` when the `start` hosts list `host` (`*.x` matches by suffix).
    fn listed(&self, host: &str) -> Option<Option<HostKind>> {
        let h = host.to_ascii_lowercase();
        let hit = |p: &str| match p.strip_prefix('*') {
            Some(end) => end.starts_with('.') && h.ends_with(end),
            None => p == h,
        };
        self.hosts.iter().find(|(p, _)| hit(p)).map(|(_, k)| *k)
    }

    fn start(&mut self, ev: &Value) {
        self.connector = ev["connector"].as_str().map(str::to_string);
        let lists = ev
            .get("hosts")
            .and_then(Value::as_object)
            .into_iter()
            .flatten();
        self.hosts = lists
            .filter_map(|(_, l)| l.as_array())
            .flatten()
            .filter_map(|e| {
                let host = e.get("host")?.as_str()?.to_ascii_lowercase();
                Some((host, host_kind(e.get("kind").and_then(Value::as_str))))
            })
            .collect();
    }
}

/// The U1 sink. No `Debug`: it holds body bytes, which no panic or log may print (PRD-17 VR2).
#[derive(Default)]
pub struct Calls {
    ctx: Ctx,
    wire: wire::Wire,
    native: native::Native,
}

impl Calls {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes one record.
    pub fn push(&mut self, rec: &Record) {
        match rec {
            Record::Part {
                entry,
                boot,
                wall_ns,
                mono_ns,
                ..
            } => {
                if *boot != self.ctx.boot {
                    // The dead boot's open messages are cut by its gap (PRD-01 section 7).
                    self.wire.cut(&self.ctx);
                }
                (self.ctx.entry, self.ctx.boot) = (entry.clone(), *boot);
                self.ctx.base = Some((*wall_ns, *mono_ns));
            }
            Record::Link {
                wall_ns, mono_ns, ..
            } => self.ctx.base = Some((*wall_ns, *mono_ns)),
            Record::Event { json, .. } => {
                let ev = serde_json::from_slice::<Value>(json).unwrap_or(Value::Null);
                if ev.get("ev").and_then(Value::as_str) == Some("start") {
                    self.ctx.start(&ev);
                }
            }
            _ => {
                self.wire.push(&self.ctx, rec);
                self.native.push(&self.ctx, rec);
            }
        }
    }

    /// The table: wire rows in (boot, sid, msg) order, then native rows in (path, time) order,
    /// joined by reply id or by time (PRD-09 section 6).
    pub fn finish(self) -> CallTable {
        let (mut rows, replies) = self.wire.finish(&self.ctx);
        let ok = |r: &CallRow| r.kind == Kind::Call && r.outcome == Outcome::Ok;
        let c = self.ctx.connector.as_deref();
        let Some((per_thread, by_id)) = hints::join_of(c) else {
            for r in rows.iter_mut().filter(|r| r.kind == Kind::Call) {
                r.join = Some(Join::WireOnly);
            }
            return CallTable {
                rows,
                ..CallTable::default()
            };
        };
        let leg = self.native.finish();
        let mut nat = leg.rows;
        nat.sort_by(|a, b| (&a.0.path, a.0.wall_end).cmp(&(&b.0.path, b.0.wall_end)));
        let (mut pair, mut taken) = (vec![None; nat.len()], vec![false; rows.len()]);
        // By reply id: each native `requestId` takes the first unmatched ok wire call with it.
        let mut by_reply: HashMap<&str, Vec<usize>> = HashMap::new();
        for w in (0..rows.len()).rev().filter(|&w| by_id && ok(&rows[w])) {
            if let Some(id) = replies[w].as_deref() {
                by_reply.entry(id).or_default().push(w);
            }
        }
        for (n, (_, id)) in nat.iter().enumerate() {
            if let Some(w) = id.as_deref().and_then(|i| by_reply.get_mut(i)?.pop()) {
                (taken[w], pair[n]) = (true, Some(w));
            }
        }
        // Each native call, in time order, takes the earliest unmatched ok wire call that ends
        // 120 s before to 5 s after it; of its own thread where the wire names threads.
        let mut wires: Vec<usize> = (0..rows.len()).filter(|&w| ok(&rows[w])).collect();
        wires.sort_by_key(|&w| rows[w].wall_end);
        let mut order: Vec<usize> = (0..nat.len()).filter(|&n| pair[n].is_none()).collect();
        order.sort_by_key(|&n| nat[n].0.wall_end);
        let mut joined_by_time = 0;
        for n in order {
            let (nr, Some(t)) = (&nat[n].0, nat[n].0.wall_end) else {
                continue;
            };
            let lo = t.saturating_sub(120_000_000_000);
            let hi = t.saturating_add(5_000_000_000);
            let early = |&w: &usize| rows[w].wall_end.is_none_or(|e| e < lo);
            let near = |&&w: &&usize| rows[w].wall_end <= Some(hi);
            let free = |&&w: &&usize| {
                !taken[w] && (!per_thread || nr.thread.is_some() && rows[w].thread == nr.thread)
            };
            let from = wires.partition_point(early);
            if let Some(&w) = wires[from..].iter().take_while(near).find(free) {
                (taken[w], pair[n]) = (true, Some(w));
                joined_by_time += u64::from(by_id);
            }
        }
        for (nr, w) in nat.iter_mut().map(|x| &mut x.0).zip(pair) {
            let said = nr.tokens != Tokens::default();
            let Some(w) = w else {
                (nr.join, nr.usage_from) = (Some(Join::NativeOnly), said.then_some(Leg::Native));
                continue;
            };
            let r = &mut rows[w];
            r.thread = r.thread.take().or_else(|| nr.thread.clone());
            r.parent = r.parent.take().or_else(|| nr.parent.clone());
            if r.usage_from.is_none() && said {
                (r.tokens, r.extra) = (nr.tokens, nr.extra.clone());
                r.usage_from = Some(Leg::Native);
            }
            (r.join, nr.join) = (Some(Join::Matched), Some(Join::Matched));
        }
        for r in rows.iter_mut().filter(|r| r.kind == Kind::Call) {
            let lost = (per_thread && ok(r) && r.thread.is_none()).then_some(Join::Unattributed);
            r.join = r.join.or(lost).or(Some(Join::WireOnly));
        }
        let reason = (nat.is_empty() && rows.iter().any(ok)).then_some(Reason::NoNative);
        let mut threads = leg.threads;
        let mut per: HashMap<&str, (u64, u64)> = HashMap::new();
        for x in &nat {
            if let Some(id) = x.0.thread.as_deref() {
                let e = per.entry(id).or_default();
                (e.0, e.1) = (e.0 + 1, e.1 + u64::from(x.0.join == Some(Join::NativeOnly)));
            }
        }
        for t in &mut threads {
            (t.native_calls, t.native_only) = per.get(t.id.as_str()).copied().unwrap_or_default();
        }
        let known: HashSet<String> = threads.iter().map(|t| t.id.clone()).collect();
        let wire_only: BTreeSet<&String> = rows.iter().filter_map(|r| r.thread.as_ref()).collect();
        for id in wire_only.into_iter().filter(|id| !known.contains(*id)) {
            threads.push(Thread {
                reason,
                ..Thread::new(id)
            });
        }
        // An unmatched wire row of a child thread takes the parent its thread entry gives.
        let parents: HashMap<&str, &String> = threads
            .iter()
            .filter_map(|t| Some((t.id.as_str(), t.parent.as_ref()?)))
            .collect();
        for r in rows.iter_mut().filter(|r| r.parent.is_none()) {
            let p = r.thread.as_deref().and_then(|t| parents.get(t));
            r.parent = p.map(|p| (*p).clone());
        }
        rows.extend(nat.into_iter().map(|x| x.0));
        CallTable {
            rows,
            threads,
            children: leg.children,
            reason,
            native_form: reason.map_or(NativeForm::PerCall, |_| NativeForm::None),
            joined_by_time,
            repeats_skipped: leg.repeats_skipped,
            copied_skipped: leg.copied_skipped,
        }
    }
}

/// Runs `goalcup-verify usage <args>` (PRD-09 section 7); not built yet, so exits 3 (no report).
pub fn run(args: Vec<OsString>) -> ExitCode {
    let _ = args;
    eprintln!("goalcup-verify: the usage reader is not built yet");
    ExitCode::from(3)
}
