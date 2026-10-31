//! Legs (PRD-08 section 3): `wire_keeps_up`, `threads_match` and `subagents_present` over U1's call
//! table, the provider error counts and the legs part of the coverage line. Integrity's one pass
//! feeds U1's `Calls` (one table row per call); [`judge`] is pure over the table and the covers,
//! which honesty collects. It reads `native_form`, threads and `children`, and names no harness.

use crate::integrity::{Check, Res};
use crate::timeline::Streams;
use crate::usage::{CallRow, CallTable, Kind, Leg, NativeForm, Outcome, Reason, Shape};
use goalcup_format::dump::wall;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

const MIN: u64 = 60_000_000_000;
/// A harness call may reach the wire this long after the harness logged it.
const LAG: u64 = 2 * MIN;
const WIRE_ENDED: u64 = 10 * MIN;
const NOT_READ: &str = "the harness's own log is not read";
const UNREAD: &str = "could not read the harness's own log";

/// Wall time (ns since the epoch) in which a loss is named: a recorder-down GAP, or a conn GAP
/// of a connection to a model host (a listed host of kind provider or gateway, or one the
/// participant or config named). Honesty collects them.
#[derive(Clone, Copy, Debug)]
pub struct Cover {
    pub start_wall: u64,
    pub end_wall: u64,
    /// A recorder-down GAP: the only cause that removes both legs of a subagent.
    pub down: bool,
}

/// A timed call for the report's timeline.
pub struct Timed {
    pub native: bool,
    pub err: bool,
    pub wall: u64,
}

pub struct Out {
    pub harness_log: &'static str,
    pub provider_errors: Value,
    pub threads: Vec<Value>,
    /// Wire calls the provider refused.
    pub refused: u64,
    /// The harness-log sentence of the coverage line; it never claims everything was recorded.
    pub coverage: String,
    /// The covers, and the timed calls the two checks count (native?, error?, wall) for the
    /// report's timeline.
    pub covers: Vec<Cover>,
    pub timed: Vec<Timed>,
}

/// What the harness's own log gave, as the report's `harness_log`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Log {
    None,
    Totals,
    PerCall,
}

impl Log {
    fn name(self) -> &'static str {
        match self {
            Log::None => "none",
            Log::Totals => "totals",
            Log::PerCall => "per_call",
        }
    }
}

/// The table with what both leg checks read: the covers, the thread cut-offs and call counts.
struct Ctx<'a> {
    t: &'a CallTable,
    covers: &'a [Cover],
    log: Log,
    /// Each thread's `native_until`: its native calls after it are not known, so not compared.
    until: HashMap<&'a str, u64>,
    native: u64,
    wire: u64,
}

impl<'a> Ctx<'a> {
    fn new(t: &'a CallTable, covers: &'a [Cover]) -> Self {
        let log = match (t.readable(), t.native_form) {
            (false, _) => Log::None,
            (_, NativeForm::Totals) => Log::Totals,
            _ => Log::PerCall,
        };
        let until = (t.threads.iter())
            .filter_map(|x| Some((x.id.as_str(), x.native_until?)))
            .collect();
        let mut x = Ctx {
            t,
            covers,
            log,
            until,
            native: 0,
            wire: 0,
        };
        let native = x.calls().filter(|r| r.leg == Leg::Native).count() as u64;
        (x.native, x.wire) = (native, x.calls().count() as u64 - native);
        x
    }

    fn calls(&self) -> impl Iterator<Item = &'a CallRow> {
        self.t.rows.iter().filter(|r| r.kind == Kind::Call)
    }

    /// A harness call is covered when its time lies in a cover or within the lag after it.
    fn covered(&self, r: &CallRow) -> bool {
        let hit = |c: &Cover| {
            at(r).is_some_and(|t| c.start_wall <= t && t <= c.end_wall.saturating_add(LAG))
        };
        self.covers.iter().any(hit)
    }

    fn known(&self, r: &CallRow) -> bool {
        let cut = r.thread.as_deref().and_then(|id| self.until.get(id));
        at(r).zip(cut).is_none_or(|(x, &c)| x <= c)
    }

    /// Why a per-call comparison cannot run, or "". A reader that found nothing while the wire
    /// had calls is not the same as no reader: the participant is told it could not read.
    fn unread(&self) -> &'static str {
        match (self.log, self.t.reason) {
            (Log::None, Some(Reason::NoNative)) => UNREAD,
            (Log::None, _) => NOT_READ,
            (Log::PerCall, _) if self.native == 0 && self.wire > 0 => UNREAD,
            (Log::PerCall, _) if self.native == 0 => "no calls",
            _ => "",
        }
    }
}

/// The three checks, the error counts and the coverage text of one table.
pub fn judge(t: &CallTable, covers: &[Cover]) -> (Vec<Check>, Out) {
    let x = Ctx::new(t, covers);
    let (keeps, line) = wire_keeps_up(&x);
    let (matched, threads) = threads_match(&x);
    let refused = (x.calls())
        .filter(|r| r.leg == Leg::Wire && r.outcome.is_error())
        .count() as u64;
    let out = Out {
        harness_log: x.log.name(),
        provider_errors: errors(t),
        threads,
        refused,
        coverage: line,
        covers: covers.to_vec(),
        timed: (x.calls().filter(|r| r.leg == Leg::Wire || x.known(r)))
            .filter_map(|r| {
                let native = r.leg == Leg::Native;
                Some(Timed {
                    native,
                    err: r.outcome.is_error(),
                    wall: at(r)?,
                })
            })
            .collect(),
    };
    (vec![keeps, matched, subagents(&x)], out)
}

fn at(r: &CallRow) -> Option<u64> {
    r.wall_end.or(r.wall_start)
}

/// The tolerance: max(3, 2%) calls.
fn tol(n: u64) -> u64 {
    (n / 50).max(3)
}

/// More than the tolerance of harness calls above the wire's.
fn bad(h: u64, w: u64) -> bool {
    h > 0 && (w == 0 || h > w.saturating_add(tol(h)))
}

/// `fail` when the excess stands with the covered calls taken out, `named` when only they hold
/// it. Taking calls out never turns a pass into a fail.
fn grade(raw: bool, adjusted: bool) -> Res {
    if !raw {
        Res::Pass
    } else if adjusted {
        Res::Fail
    } else {
        Res::Named
    }
}

fn hms(ns: u64) -> String {
    let w = wall(ns);
    format!("{}Z", w.get(11..19).unwrap_or(&w))
}

/// A finding's place: a time, a thread or both.
fn found(w: Option<u64>, thread: Option<&str>) -> Value {
    json!({"o": null, "sid": null, "wall": w.map(wall), "thread": thread})
}

/// Sets what a check found; a pass keeps its facts only.
fn done(mut c: Check, res: Res, why: String, first: Value, facts: Value) -> Check {
    c.facts = facts.as_object().cloned().unwrap_or_default();
    if res > Res::Pass {
        (c.result, c.why, c.first) = (res, why, Some(first).filter(|f| !f.is_null()));
    }
    c
}

fn skip(c: Check, why: &str) -> Check {
    done(c, Res::Skip, why.into(), Value::Null, Value::Null)
}

/// Where harness calls ran ahead of the wire's: the top excess, and the crossings of the
/// tolerance (the first, the worst and its time).
#[derive(Default)]
struct Sweep {
    top: u64,
    worst: u64,
    first: Option<u64>,
    at: u64,
}

impl Sweep {
    /// Harness calls up to each harness time T against wire calls up to T plus the lag.
    fn new(h: &[u64], w: &[u64]) -> Self {
        let mut s = Sweep::default();
        for (j, &t) in h.iter().enumerate() {
            let n = j as u64 + 1;
            let ex = n.saturating_sub(w.partition_point(|&x| x <= t.saturating_add(LAG)) as u64);
            s.top = s.top.max(ex);
            if ex > tol(n) {
                s.first.get_or_insert(t);
                if ex >= s.worst {
                    (s.worst, s.at) = (ex, t);
                }
            }
        }
        s
    }

    fn span(&self) -> String {
        let from = self.first.unwrap_or(self.at);
        let min = self.at.saturating_sub(from) / MIN;
        format!("from {} to {} ({min} min)", hms(from), hms(self.at))
    }
}

/// The check and the harness's part of the coverage line.
fn wire_keeps_up(x: &Ctx) -> (Check, String) {
    let c = Check::new("wire_keeps_up", None, Res::Pass, "");
    // `w_open` leaves out the wire calls inside a cover: they answer covered harness calls and
    // must not offset a loss outside it.
    let (mut h, mut w, mut w_open, mut last_wire) = (Vec::new(), Vec::new(), Vec::new(), None);
    for r in x.calls() {
        if r.leg == Leg::Native {
            h.extend(at(r).filter(|_| x.known(r)));
        } else {
            last_wire = last_wire.max(at(r));
            let ok = at(r).filter(|_| !r.outcome.is_error());
            w.extend(ok);
            w_open.extend(ok.filter(|_| !x.covered(r)));
        }
    }
    let why = match x.unread() {
        "" if x.log == Log::Totals || h.is_empty() => "no per-call times",
        why => why,
    };
    if !why.is_empty() {
        let line = format!(
            "The harness's own log {}.",
            if why == NOT_READ {
                "is not read for this harness (optional)".to_string()
            } else {
                format!("was not compared: {why}")
            }
        );
        return (skip(c, why), line);
    }
    h.sort_unstable();
    w.sort_unstable();
    w_open.sort_unstable();
    let mut open: Vec<u64> = (x.calls())
        .filter(|r| r.leg == Leg::Native && x.known(r) && !x.covered(r))
        .filter_map(at)
        .collect();
    open.sort_unstable();
    let (raw, adj) = (Sweep::new(&h, &w), Sweep::new(&open, &w_open));
    // The harness kept calling long after the wire's last call, whatever the wire's outcome.
    let ended = |l: Option<&u64>| {
        l.is_some_and(|&l| last_wire.is_none_or(|v| l > v.saturating_add(WIRE_ENDED)))
    };
    let (e_raw, e_adj) = (ended(h.last()), ended(open.last()));
    let s = if adj.first.is_some() { &adj } else { &raw };
    let res = grade(raw.first.is_some() || e_raw, adj.first.is_some() || e_adj);
    let mut why = match (s.first, h.last()) {
        (Some(_), _) => format!("harness above wire by {} calls {}", s.worst, s.span()),
        (_, last) => format!(
            "the wire's last call was {}, the harness kept calling to {}",
            last_wire.map_or("never".into(), hms),
            last.map_or(String::new(), |&l| hms(l))
        ),
    };
    if res == Res::Named {
        why += "; a named gap covers it";
    }
    let facts = json!({"harness": h.len(), "wire_ok": w.len(), "excess": s.top,
        "from": s.first.map(hms), "to": s.first.map(|_| hms(s.at)),
        "wire_above": w.len().saturating_sub(h.len())});
    let (n, range) = match raw.first {
        Some(_) => (raw.worst, format!(", {}", raw.span())),
        None => (raw.top, String::new()),
    };
    let line = format!(
        "The harness's own log: {} calls; harness calls with no recording: {n}{range}.",
        x.native
    );
    (
        done(
            c,
            res,
            why,
            found(s.first.or(h.last().copied()), None),
            facts,
        ),
        line,
    )
}

#[derive(Default)]
struct Acc {
    parent: Option<String>,
    /// Native calls of the thread, and those not inside a cover.
    h: u64,
    open: u64,
    ok: u64,
    /// The `ok` wire calls not inside a cover.
    ok_open: u64,
    err: u64,
    total: Option<u64>,
}

/// The check and the per-thread rows of the report.
fn threads_match(x: &Ctx) -> (Check, Vec<Value>) {
    let c = Check::new("threads_match", None, Res::Pass, "");
    let totals = x.log == Log::Totals;
    let mut by: BTreeMap<&str, Acc> = BTreeMap::new();
    let (mut threaded, mut chat) = (false, 0u64);
    for r in x.calls() {
        let nat = r.leg == Leg::Native;
        chat += u64::from(!nat && matches!(r.shape, Some(Shape::Chat | Shape::Anthropic)));
        let Some(a) = r.thread.as_deref().map(|id| by.entry(id).or_default()) else {
            continue;
        };
        a.parent = a.parent.take().or_else(|| r.parent.clone());
        if nat && x.known(r) {
            (a.h, a.open) = (a.h + 1, a.open + u64::from(!x.covered(r)));
        } else if !nat {
            threaded = true;
            if r.outcome.is_error() {
                a.err += 1;
            } else {
                (a.ok, a.ok_open) = (a.ok + 1, a.ok_open + u64::from(!x.covered(r)));
            }
        }
    }
    for th in &x.t.threads {
        let a = if totals {
            Some(by.entry(&th.id).or_default())
        } else {
            by.get_mut(th.id.as_str())
        };
        if let Some(a) = a {
            (a.parent, a.total) = (th.parent.clone().or(a.parent.take()), th.native_total);
        }
    }
    let why = match x.unread() {
        "" if totals && !by.values().any(|a| a.total.is_some()) => "no per-session totals",
        "" if by.is_empty() => "no call carries a thread id",
        why => why,
    };
    if !why.is_empty() {
        return (skip(c, why), Vec::new());
    }
    if totals && !threaded {
        // No wire call names a thread: the entry's totals against its chat calls (PRD-09 section 6).
        let total = by
            .values()
            .filter_map(|a| a.total)
            .fold(0, u64::saturating_add);
        by = BTreeMap::from([(
            "",
            // The key `wire_ok` holds every chat call of the entry here, refused ones included
            // (PRD-08 section 3.1), and `wire_err` stays 0.
            Acc {
                total: Some(total),
                ok: chat,
                ok_open: chat,
                ..Acc::default()
            },
        )]);
    }
    let any = !x.covers.is_empty();
    let rows: Vec<(Value, Res)> = (by.iter())
        .map(|(id, a)| {
            // The totals form cannot say when: any cover in the entry names the excess.
            let tot = a.total.unwrap_or(0);
            let (h, open) = if totals {
                (tot, if any { 0 } else { tot })
            } else {
                (a.h, a.open)
            };
            let res = grade(bad(h, a.ok), bad(open, a.ok_open));
            let thread = Some(*id).filter(|i| !i.is_empty());
            let v = json!({"thread": thread, "parent": a.parent, "harness": h, "wire_ok": a.ok,
                "wire_err": a.err, "result": res.name(), "history": null});
            (v, res)
        })
        .collect();
    let worst = rows.iter().map(|r| r.1).max().unwrap_or(Res::Pass);
    let n = rows.iter().filter(|r| r.1 > Res::Pass).count();
    let (null, first) = (Value::Null, rows.iter().find(|r| r.1 == worst));
    let v = first.map_or(&null, |r| &r.0);
    let id = v["thread"].as_str();
    let who = id.map_or("the entry".into(), |i| format!("thread {i}"));
    let why = format!(
        "{n} of {} threads have more harness calls than recorded ones; first: {who}, {} harness \
         calls against {} recorded",
        rows.len(),
        v["harness"],
        v["wire_ok"]
    );
    let check = done(
        c,
        worst,
        why,
        found(None, id),
        json!({"threads": rows.len(), "bad": n}),
    );
    (check, rows.into_iter().map(|r| r.0).collect())
}

fn subagents(x: &Ctx) -> Check {
    let c = Check::new("subagents_present", None, Res::Pass, "");
    let Some(kids) = &x.t.children else {
        return skip(c, "parentage was not read");
    };
    // A thread whose own log was read has a native stream, however few calls it completed.
    let named = (x.t.threads.iter())
        .filter(|t| t.readable())
        .map(|t| t.id.as_str());
    let seen: HashSet<&str> = x
        .calls()
        .filter_map(|r| r.thread.as_deref())
        .chain(named)
        .collect();
    let all: BTreeSet<&String> = kids.values().flatten().collect();
    let gone: Vec<&String> = all
        .iter()
        .copied()
        .filter(|k| !seen.contains(k.as_str()))
        .collect();
    // A child with neither leg has no time and no connection: only the recorder being down
    // removes both legs, so only a recorder-down cover names it. A conn GAP explains wire calls
    // of its own connection, not a native stream.
    let down = x.covers.iter().any(|c| c.down);
    let res = grade(!gone.is_empty(), !gone.is_empty() && !down);
    let first = gone.first().map(|k| k.as_str());
    let why = format!(
        "{} of {} subagents have no call on either leg; first: thread {}",
        gone.len(),
        all.len(),
        first.unwrap_or_default()
    );
    done(
        c,
        res,
        why,
        found(None, first),
        json!({"children": all.len(), "missing": gone}),
    )
}

/// Provider errors by code, one per error call: the wire apart from the harness's own log.
fn errors(t: &CallTable) -> Value {
    let (mut wire, mut native) = (BTreeMap::new(), BTreeMap::new());
    for r in t.rows.iter().filter(|r| r.kind == Kind::Call) {
        let (Outcome::RateLimited { status, code } | Outcome::Error { status, code }) = &r.outcome
        else {
            continue;
        };
        let key = code.clone().or_else(|| status.map(|s| format!("http_{s}")));
        let m = if r.leg == Leg::Native {
            &mut native
        } else {
            &mut wire
        };
        let n: &mut u64 = m.entry(key.unwrap_or_else(|| "error".into())).or_default();
        *n = n.saturating_add(1);
    }
    json!({"wire": wire, "native": native})
}

/// Reads the manifest, `[{"path": "<source>:<path>", "size": n}]`, as (path, size) pairs. The
/// error is a plain line for exit 3.
pub fn read_manifest(p: &std::path::Path) -> Result<Vec<(String, u64)>, String> {
    let bad =
        || "--harness-files: expected [{\"path\": \"<source>:<path>\", \"size\": n}]".to_string();
    let v: Value = std::fs::read(p)
        .map_err(|e| format!("{}: {e}", p.display()))
        .and_then(|b| serde_json::from_slice(&b).map_err(|_| bad()))?;
    let each = v.as_array().ok_or_else(bad)?.iter();
    let pair = |f: &Value| Some((f["path"].as_str()?.to_string(), f["size"].as_u64()?));
    each.map(|f| pair(f).ok_or_else(bad)).collect()
}

/// The manifest part of `subagents_present`: every file on disk has a native stream whose last
/// END `seen` equals its size, or a snapshot stream (a database keeps growing, so its size is
/// not compared). A file with neither is `named` by a GAP on its stream or by a recorder-down
/// cover, else `fail`. Never lowers the check's result; the first finding names a path only.
pub fn manifest(c: &mut Check, files: &[(String, u64)], s: &Streams, covers: &[Cover]) {
    let down = covers.iter().any(|c| c.down);
    let ok = |(p, n): &&(String, u64)| s.snapshots.contains(p) || s.native.get(p) == Some(n);
    let lost: Vec<(&String, bool)> = (files.iter().filter(|f| !ok(f)))
        .map(|(p, _)| (p, down || s.gapped.contains(p)))
        .collect();
    let unnamed = lost.iter().find(|l| !l.1);
    let res = match (lost.first(), unnamed) {
        (None, _) => Res::Pass,
        (_, Some(_)) => Res::Fail,
        _ => Res::Named,
    };
    let named_by_down = lost
        .iter()
        .filter(|l| l.1 && !s.gapped.contains(l.0))
        .count();
    c.facts.insert("files".into(), files.len().into());
    c.facts
        .insert("files_without_stream".into(), lost.len().into());
    c.facts
        .insert("files_named_by_recorder_down".into(), named_by_down.into());
    // A pass or a skip stays as it is: the manifest part never raises a skip (PRD-12 section 8).
    let Some((f, _)) = unnamed.or(lost.first()) else {
        return;
    };
    if res > c.result {
        let why = "harness files have no stream of their size";
        c.why = format!("{} of {} {why}; first: {f}", lost.len(), files.len());
        c.first = Some(json!({"o": null, "sid": null, "wall": null, "thread": null, "file": f}));
        c.result = res;
    }
}
