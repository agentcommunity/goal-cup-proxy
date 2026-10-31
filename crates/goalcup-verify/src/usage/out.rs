//! Seam S7 (PRD-09 section 7): the summary and the calls file, pure over a [`CallTable`]; only
//! fields it holds, never body text. A call counts once: a wire call, or a native call with no twin.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;

use super::{CallRow, CallTable, Join, Kind, Leg, Outcome, Tokens};
use goalcup_format::dump::wall;
use serde_json::{Value, json};

const KEYS: [&str; 5] = ["input", "cached", "cache_write", "output", "reasoning"];
const STATUS: [&str; 5] = ["ok", "rate_limited", "error", "cancelled", "unknown"];

/// `NativeOnly` as `native_only`. The S7 names come from the variants' `Debug` names, so renaming
/// a variant renames an output key.
fn snake(x: impl Debug) -> String {
    let mut s = String::new();
    for c in format!("{x:?}").chars() {
        if c.is_uppercase() && !s.is_empty() {
            s.push('_');
        }
        s.extend(c.to_lowercase());
    }
    s
}

fn status(o: &Outcome) -> &'static str {
    match o {
        Outcome::Ok => "ok",
        Outcome::RateLimited { .. } => "rate_limited",
        Outcome::Error { .. } => "error",
        Outcome::Cancelled => "cancelled",
        Outcome::Unknown => "unknown",
    }
}

/// The count of rows per status, every status present.
fn by_status<'a>(rows: impl Iterator<Item = &'a CallRow>) -> BTreeMap<&'static str, u64> {
    let mut m: BTreeMap<_, u64> = STATUS.into_iter().map(|k| (k, 0)).collect();
    rows.for_each(|r| *m.entry(status(&r.outcome)).or_default() += 1);
    m
}

fn five(t: &Tokens) -> [Option<u64>; 5] {
    [t.input, t.cached, t.cache_write, t.output, t.reasoning]
}

/// The five sums of these rows' own tokens; a sum of numbers all unsaid is null, never 0.
fn total<'a>(rows: impl Iterator<Item = &'a CallRow>) -> Value {
    sum(rows.map(|r| &r.tokens))
}

fn sum<'a>(tokens: impl Iterator<Item = &'a Tokens>) -> Value {
    let mut acc = [None; 5];
    for t in tokens {
        for (a, v) in acc.iter_mut().zip(five(t)) {
            let old: Option<u64> = *a;
            *a = old.zip(v).map(|(x, y)| x.saturating_add(y)).or(old).or(v);
        }
    }
    KEYS.iter().zip(acc).map(|(k, v)| (*k, json!(v))).collect()
}

pub(super) fn summary(t: &CallTable) -> Value {
    let rows = || t.rows.iter();
    let twin = |r: &&CallRow| r.leg == Leg::Native && r.join == Some(Join::Matched);
    let counted = |r: &&CallRow| r.kind == Kind::Call && !twin(r);
    let wire = |r: &&CallRow| r.leg == Leg::Wire && r.kind == Kind::Call;
    let has_usage = |r: &&CallRow| r.usage_from.is_some();
    let n = |j| rows().filter(counted).filter(|r| r.join == Some(j)).count();
    let mut st = json!(by_status(rows().filter(wire)));
    st["not_whole"] = rows().filter(wire).filter(|r| !r.whole).count().into();
    let none = rows().filter(counted).filter(|r| r.usage_from.is_none());
    let mut by: BTreeMap<Option<&str>, Vec<&CallRow>> = BTreeMap::new();
    for r in rows().filter(counted) {
        by.entry(r.model.as_deref()).or_default().push(r);
    }
    let models: Vec<_> = (by.into_iter())
        .map(|(m, rs)| {
            let req = rs.iter().filter_map(|r| r.model_requested.as_deref());
            let ths = rs.iter().filter_map(|r| r.thread.as_deref());
            json!({"model": m, "requested": req.collect::<BTreeSet<_>>(), "calls": rs.len(),
                "threads": ths.collect::<BTreeSet<_>>().len(),
                "tokens": total(rs.iter().copied().filter(has_usage))})
        })
        .collect();
    let mut by_th: BTreeMap<&str, Vec<&CallRow>> = BTreeMap::new();
    for (k, r) in rows().filter_map(|r| Some((r.thread.as_deref()?, r))) {
        by_th.entry(k).or_default().push(r);
    }
    let threads: Vec<_> = (t.threads.iter())
        .map(|th| {
            let rs: &[_] = by_th.get(th.id.as_str()).map_or(&[], Vec::as_slice);
            let times = rs.iter().flat_map(|r| [r.wall_start, r.wall_end]).flatten();
            let said = rs.iter().copied().filter(counted).filter(has_usage);
            let own = || rs.iter().copied().filter(counted);
            let mut models: BTreeMap<&str, u64> = BTreeMap::new();
            own()
                .filter_map(|r| r.model.as_deref())
                .for_each(|m| *models.entry(m).or_default() += 1);
            let wires = || rs.iter().copied().filter(wire);
            json!({"id": th.id, "parent": th.parent, "depth": th.depth, "label": th.label,
                "wire_calls": wires().count(), "native_calls": th.native_calls,
                "native_only": th.native_only, "native_total": th.native_total,
                "native_until": th.native_until.map(wall), "native_tokens": sum([&th.native_tokens].into_iter()),
                "matched": rs.iter().filter(|r| twin(r)).count(),
                "status": by_status(wires()), "tokens": total(said), "models": models,
                "effort": own().filter_map(|r| r.effort.as_deref()).collect::<BTreeSet<_>>(),
                "first": times.clone().min().map(wall), "last": times.max().map(wall),
                "readable": th.readable(), "reason": th.reason.map(snake)})
        })
        .collect();
    let native = || rows().filter(|r| r.leg == Leg::Native);
    json!({"v": 1, "reader": env!("CARGO_PKG_VERSION"), "run": t.run, "entry": t.entry,
        "connector": t.connector, "harness_version": t.harness_version, "sealed": t.sealed,
        "readable": t.readable(), "reason": t.reason.map(snake), "native_until": t.native_until.map(wall),
        "db_set": t.db_set, "harness_exited": t.harness_exited,
        "coverage": {"wire_calls": rows().filter(wire).count(), "native_calls": native().count(),
            "matched": n(Join::Matched), "native_only": n(Join::NativeOnly),
            "wire_only": n(Join::WireOnly), "unattributed": n(Join::Unattributed),
            "joined_by_time": t.joined_by_time, "repeats_skipped": t.repeats_skipped,
            "copied_skipped": t.copied_skipped},
        "status": st, "models": models, "threads": threads,
        "tokens": {"merged": total(rows().filter(counted).filter(has_usage)), "native": sum(native().map(|r| &r.tokens).chain(t.threads.iter().map(|x| &x.native_tokens))),
            "wire": total(rows().filter(counted).filter(|r| r.usage_from == Some(Leg::Wire))),
            "calls_without_usage": none.clone().count(),
            "calls_without_usage_by_status": by_status(none)}})
}

pub(super) fn calls_jsonl(t: &CallTable) -> String {
    t.rows
        .iter()
        .map(|r| {
            let mut v = json!({"k": snake(r.kind), "leg": snake(r.leg), "entry": r.entry,
                "boot": r.boot, "sid": r.sid, "msg": r.msg, "t0": r.wall_start.map(wall),
                "t1": r.wall_end.map(wall), "host": r.host, "host_kind": r.host_kind.map(snake),
                "shape": r.shape.map(snake), "via": r.via.map(snake), "model": r.model,
                "model_requested": r.model_requested, "effort": r.effort,
                "status": status(&r.outcome), "detail": r.detail, "whole": r.whole,
                "thread": r.thread, "parent": r.parent, "join": r.join.map(snake),
                "usage_from": r.usage_from.map(snake)});
            for (k, n) in KEYS.iter().zip(five(&r.tokens)) {
                v[*k] = json!(n);
            }
            if !r.extra.is_empty() {
                v["extra"] = json!(r.extra);
            }
            format!("{v}\n")
        })
        .collect()
}
