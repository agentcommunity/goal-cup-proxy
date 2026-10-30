//! Seam S7 (PRD-09 section 7): the summary and the calls file, pure over a [`CallTable`]; only
//! fields it holds, never body text. A call counts once: a wire call, or a native call with no twin.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;

use super::{CallRow, CallTable, Join, Kind, Leg, Outcome, Tokens};
use goalcup_format::dump::wall;
use serde_json::{Value, json};

const KEYS: [&str; 5] = ["input", "cached", "cache_write", "output", "reasoning"];

/// `NativeOnly` as `native_only`.
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
    let all = ["ok", "rate_limited", "error", "cancelled", "unknown"];
    let mut m: BTreeMap<_, u64> = all.into_iter().map(|k| (k, 0)).collect();
    rows.for_each(|r| *m.entry(status(&r.outcome)).or_default() += 1);
    m
}

fn five(t: &Tokens) -> [Option<u64>; 5] {
    [t.input, t.cached, t.cache_write, t.output, t.reasoning]
}

/// The five sums of these rows' own tokens; a sum of numbers all unsaid is null, never 0.
fn total<'a>(rows: impl Iterator<Item = &'a CallRow>) -> Value {
    let mut acc = [None; 5];
    for r in rows {
        for (a, v) in acc.iter_mut().zip(five(&r.tokens)) {
            let old: Option<u64> = *a;
            *a = old.zip(v).map(|(x, y)| x.saturating_add(y)).or(old).or(v);
        }
    }
    KEYS.iter().zip(acc).map(|(k, v)| (*k, json!(v))).collect()
}

/// The structured status and code of a refusal or error, then the row's own token.
fn detail(r: &CallRow) -> Option<String> {
    let (s, c) = match &r.outcome {
        Outcome::RateLimited { status, code } | Outcome::Error { status, code } => (*status, code),
        _ => (None, &None),
    };
    let own = [s.map(|s| s.to_string()), c.clone(), r.detail.clone()];
    let parts: Vec<_> = own.into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

pub(super) fn summary(t: &CallTable) -> Value {
    let rows = || t.rows.iter();
    let twin = |r: &&CallRow| r.leg == Leg::Native && r.join == Some(Join::Matched);
    let counted = |r: &&CallRow| r.kind == Kind::Call && !twin(r);
    let wire = |r: &&CallRow| r.leg == Leg::Wire && r.kind == Kind::Call;
    let said = |r: &&CallRow| r.usage_from.is_some();
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
                "tokens": total(rs.iter().copied().filter(said))})
        })
        .collect();
    let threads: Vec<_> = (t.threads.iter())
        .map(|th| {
            let mine = |r: &&CallRow| r.thread.as_deref() == Some(th.id.as_str());
            let at = |r: &CallRow| [r.wall_start, r.wall_end];
            let times = rows().filter(mine).flat_map(at).flatten();
            json!({"id": th.id, "parent": th.parent, "depth": th.depth, "label": th.label,
                "wire_calls": rows().filter(mine).filter(wire).count(),
                "native_calls": th.native_calls, "native_only": th.native_only,
                "native_total": th.native_total, "matched": rows().filter(mine).filter(twin).count(),
                "status": by_status(rows().filter(mine).filter(wire)),
                "tokens": total(rows().filter(mine).filter(counted).filter(said)),
                "first": times.clone().min().map(wall), "last": times.max().map(wall),
                "readable": th.readable(), "reason": th.reason.map(snake)})
        })
        .collect();
    let native = || rows().filter(|r| r.leg == Leg::Native);
    json!({"v": 1, "reader": env!("CARGO_PKG_VERSION"), "run": t.run, "entry": t.entry,
        "connector": t.connector, "harness_version": t.harness_version, "sealed": t.sealed,
        "readable": t.readable(), "reason": t.reason.map(snake), "native_until": null, "db_set": null,
        "coverage": {"wire_calls": rows().filter(wire).count(), "native_calls": native().count(),
            "matched": n(Join::Matched), "native_only": n(Join::NativeOnly),
            "wire_only": n(Join::WireOnly), "unattributed": n(Join::Unattributed),
            "joined_by_time": t.joined_by_time},
        "status": st, "models": models, "threads": threads,
        "tokens": {"merged": total(rows().filter(counted).filter(said)), "native": total(native()),
            "wire": total(rows().filter(counted).filter(|r| r.usage_from == Some(Leg::Wire))),
            "calls_without_usage": none.clone().count(),
            "calls_without_usage_by_status": by_status(none)}})
}

pub(super) fn calls_jsonl(t: &CallTable) -> String {
    let rows = t.rows.iter();
    rows.map(|r| {
        let k = r.tokens;
        let mut v = json!({"k": snake(r.kind), "leg": snake(r.leg), "entry": r.entry, "boot": r.boot,
            "sid": r.sid, "msg": r.msg, "path": r.path, "t0": r.wall_start.map(wall),
            "t1": r.wall_end.map(wall), "host": r.host, "host_kind": r.host_kind.map(snake),
            "shape": r.shape.map(snake), "via": r.via.map(snake), "model": r.model,
            "model_requested": r.model_requested, "effort": r.effort, "status": status(&r.outcome),
            "detail": detail(r), "whole": r.whole, "thread": r.thread, "parent": r.parent,
            "join": r.join.map(snake), "usage_from": r.usage_from.map(snake),
            "input": k.input, "cached": k.cached, "cache_write": k.cache_write,
            "output": k.output, "reasoning": k.reasoning});
        if !r.extra.is_empty() {
            v["extra"] = json!(r.extra);
        }
        format!("{v}\n")
    })
    .collect()
}
