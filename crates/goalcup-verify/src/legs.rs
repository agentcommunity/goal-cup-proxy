//! Legs (PRD-08 section 3): `wire_keeps_up`, `threads_match` and `subagents_present` over U1's call
//! table, the provider error counts and the legs part of the coverage line. [`Legs`] takes the
//! records of integrity's one pass (a few words per open stream, one table row per call);
//! [`judge`] is pure over the table and the covers. It reads `native_form`, threads and
//! `children`, and names no harness.

use crate::integrity::{Check, Res};
use crate::usage::{CallRow, CallTable, Calls, Kind, Leg, NativeForm, Outcome, Shape};
use goalcup_format::dump::wall;
use goalcup_format::{GapReason, Item, Record, StreamKind};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

const MIN: u64 = 60_000_000_000;
/// A harness call may reach the wire this long after the harness logged it.
const LAG: u64 = 2 * MIN;
const WIRE_ENDED: u64 = 10 * MIN;
const NOT_READ: &str = "the harness's own log is not read";
const UNREAD: &str = "could not read the harness's own log";

/// Wall time (ns since the epoch) in which a loss is named: a recorder-down GAP, or a GAP on a
/// connection to a listed host.
#[derive(Clone, Copy, Debug)]
pub struct Cover {
    pub start_wall: u64,
    pub end_wall: u64,
}

pub struct Out {
    pub checks: Vec<Check>,
    pub harness_log: &'static str,
    pub provider_errors: Value,
    pub threads: Vec<Value>,
    pub n_threads: u64,
    /// The legs part of the coverage line; it never claims everything was recorded.
    pub coverage: String,
}

/// The collector held by integrity's pass: feeds U1 and gathers the covers.
#[derive(Default)]
pub struct Legs {
    calls: Calls,
    covers: Vec<Cover>,
    boot: Option<u64>,
    /// Open connections and their streams, by sid: whether the connection is to a listed host.
    listed: HashMap<u64, bool>,
}

impl Legs {
    pub(crate) fn item(&mut self, it: &Item) {
        self.calls.push(&it.rec);
        match &it.rec {
            Record::Part { boot, .. } if self.boot != Some(*boot) => {
                (self.boot, self.listed) = (Some(*boot), HashMap::new());
            }
            Record::Open {
                sid,
                skind,
                conn,
                meta,
                ..
            } => {
                let m: Value = serde_json::from_slice(meta).unwrap_or(Value::Null);
                let own = !m["listed"].is_null();
                let on = if *skind == StreamKind::Conn {
                    own
                } else {
                    self.listed.get(conn) == Some(&true)
                };
                self.listed.insert(*sid, on);
            }
            Record::End { sid, .. } => {
                self.listed.remove(sid);
            }
            Record::Gap {
                sid,
                reason,
                start_wall,
                end_wall,
                lost,
                ..
            } => {
                let on = *sid == 0 || self.listed.get(sid) == Some(&true);
                // A GAP with no extent (gap_has_end) or after the stop covers nothing.
                let none = end_wall < start_wall || (end_wall == start_wall && *lost <= 1);
                if on && !none && *reason != GapReason::AfterStop {
                    let (start_wall, end_wall) = (*start_wall, *end_wall);
                    self.covers.push(Cover {
                        start_wall,
                        end_wall,
                    });
                }
            }
            _ => {}
        }
    }

    pub fn finish(self) -> Out {
        judge(&self.calls.finish(), &self.covers)
    }
}

/// The table with what both leg checks read: the covers, the thread cut-offs and call counts.
struct Ctx<'a> {
    t: &'a CallTable,
    covers: &'a [Cover],
    log: &'static str,
    /// Each thread's `native_until`: its native calls after it are not known, so not compared.
    until: HashMap<&'a str, u64>,
    native: u64,
    wire: u64,
}

impl<'a> Ctx<'a> {
    fn new(t: &'a CallTable, covers: &'a [Cover]) -> Self {
        let log = match (t.readable(), t.native_form) {
            (false, _) => "none",
            (_, NativeForm::Totals) => "totals",
            _ => "per_call",
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

    /// Why a per-call comparison cannot run, or "".
    fn unread(&self) -> &'static str {
        match self.log {
            "none" => NOT_READ,
            "per_call" if self.native == 0 => ["no calls", UNREAD][usize::from(self.wire > 0)],
            _ => "",
        }
    }
}

/// The three checks, the error counts and the coverage text of one table.
pub fn judge(t: &CallTable, covers: &[Cover]) -> Out {
    let x = Ctx::new(t, covers);
    let (keeps, line) = wire_keeps_up(&x);
    let (matched, threads) = threads_match(&x);
    let wire = t
        .rows
        .iter()
        .filter(|r| r.leg == Leg::Wire && r.kind == Kind::Call);
    let refused = wire.clone().filter(|r| r.outcome.is_error()).count();
    let coverage = format!(
        "Recorded: {} requests, {refused} refused by the provider. History check: skipped, not \
         built in this slice. {line}",
        wire.count()
    );
    Out {
        checks: vec![keeps, matched, subagents(&x)],
        harness_log: x.log,
        provider_errors: errors(t),
        n_threads: threads.len() as u64,
        threads,
        coverage,
    }
}

fn at(r: &CallRow) -> Option<u64> {
    r.wall_end.or(r.wall_start)
}

/// More than max(3, 2%) harness calls above the wire's.
fn bad(h: u64, w: u64) -> bool {
    h > 0 && (w == 0 || h > w.saturating_add((h / 50).max(3)))
}

/// `fail` when the excess stands with the covered calls taken out, `named` when only they hold it.
fn grade(raw: bool, adjusted: bool) -> Res {
    [Res::Pass, Res::Named, Res::Fail][if adjusted { 2 } else { usize::from(raw) }]
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
            if ex > (n / 50).max(3) {
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
    let (mut h, mut w, mut last_wire) = (Vec::new(), Vec::new(), None);
    for r in x.calls() {
        if r.leg == Leg::Native {
            h.extend(at(r).filter(|_| x.known(r)));
        } else {
            last_wire = last_wire.max(at(r));
            w.extend(at(r).filter(|_| !r.outcome.is_error()));
        }
    }
    let why = match x.unread() {
        "" if x.log == "totals" || h.is_empty() => "no per-call times",
        why => why,
    };
    if !why.is_empty() {
        let line = format!(
            "The harness's own log {}.",
            if x.log == "none" {
                "is not read for this harness (optional)".to_string()
            } else {
                format!("was not compared: {why}")
            }
        );
        return (skip(c, why), line);
    }
    h.sort_unstable();
    w.sort_unstable();
    let mut open: Vec<u64> = (x.calls())
        .filter(|r| r.leg == Leg::Native && x.known(r) && !x.covered(r))
        .filter_map(at)
        .collect();
    open.sort_unstable();
    let (raw, adj) = (Sweep::new(&h, &w), Sweep::new(&open, &w));
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
    err: u64,
    total: Option<u64>,
}

/// The check and the per-thread rows of the report.
fn threads_match(x: &Ctx) -> (Check, Vec<Value>) {
    let c = Check::new("threads_match", None, Res::Pass, "");
    let totals = x.log == "totals";
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
            *(if r.outcome.is_error() {
                &mut a.err
            } else {
                &mut a.ok
            }) += 1;
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
            Acc {
                total: Some(total),
                ok: chat,
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
            let res = grade(bad(h, a.ok), bad(open, a.ok));
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
    let named = (x.t.threads.iter())
        .filter(|t| t.native_calls > 0)
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
    // A child never seen has no time, so any cover in the entry names it.
    let res = grade(!gone.is_empty(), !gone.is_empty() && x.covers.is_empty());
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

/// Provider errors by code, one per error row: the wire apart from the harness's own log.
fn errors(t: &CallTable) -> Value {
    let mut m = [BTreeMap::new(), BTreeMap::new()];
    for r in &t.rows {
        let (Outcome::RateLimited { status, code } | Outcome::Error { status, code }) = &r.outcome
        else {
            continue;
        };
        let key = code.clone().or_else(|| status.map(|s| format!("http_{s}")));
        let n: &mut u64 = m[usize::from(r.leg == Leg::Native)]
            .entry(key.unwrap_or_else(|| "error".into()))
            .or_default();
        *n = n.saturating_add(1);
    }
    json!({"wire": m[0], "native": m[1]})
}
