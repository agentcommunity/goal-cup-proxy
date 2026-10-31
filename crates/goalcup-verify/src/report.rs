//! The `check --json` report (seam S6, PRD-08 section 4), its exit code, and the text form with
//! the coverage line (section 5). `why` is one plain line; no record byte is ever printed here.

use crate::history;
use crate::input::{self, Entry, Input};
use crate::integrity::{self, Check, Outcome, Recount, Res, has_extent};
use crate::legs::{self, Streams};
use crate::secrets::{self, Rules};
use crate::server;
use goalcup_format::dump::wall;
use goalcup_format::{GapReason, Record, StreamKind};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// A timeline bucket, ns. A caller of [`Seen::new`] may ask for another.
pub const BUCKET: u64 = 600_000_000_000;

/// One timeline row: harness calls, wire calls, wire errors, native bytes, gaps, off ms.
type Row = [u64; 6];

/// A second read of one entry's record for what the integrity pass does not keep: native
/// bytes and GAPs by time, the off spans, and the streams by file path. Rows are sparse, keyed
/// by bucket index from the entry's first wall time; an off span is clamped to the entry's own
/// first and last record time, so a forged span cannot add buckets past them.
#[derive(Default)]
struct Seen {
    bucket: u64,
    first: Option<u64>,
    last: u64,
    /// The current PART's wall and record clock, and the boot it belongs to.
    base: (u64, u64),
    boot: Option<u64>,
    /// Open native and snapshot streams: path and whether native.
    open: HashMap<u64, (String, bool)>,
    rows: BTreeMap<u64, Row>,
    off: Vec<(u64, u64)>,
    streams: Streams,
}

impl Seen {
    fn new(e: &Entry, bucket: u64) -> Self {
        let mut s = Seen {
            bucket: bucket.max(1),
            ..Default::default()
        };
        if let Ok((rd, _)) = input::open(e) {
            rd.flatten().for_each(|it| s.item(&it.rec));
        }
        s
    }

    fn row(&mut self, w: u64) -> Option<&mut Row> {
        let i = w.saturating_sub(self.first?) / self.bucket;
        Some(self.rows.entry(i).or_default())
    }

    fn item(&mut self, r: &Record) {
        let (pw, pm) = self.base;
        let at = |t: u64| {
            (i128::from(pw) + i128::from(t) - i128::from(pm)).clamp(0, u64::MAX.into()) as u64
        };
        let w = match r {
            Record::Part { wall_ns, .. } | Record::Link { wall_ns, .. } => *wall_ns,
            Record::Open { t, .. }
            | Record::Data { t, .. }
            | Record::End { t, .. }
            | Record::Event { t, .. } => at(*t),
            _ => 0,
        };
        self.last = self.last.max(w);
        match r {
            Record::Part {
                boot,
                wall_ns,
                mono_ns,
                ..
            } => {
                if self.boot.replace(*boot) != Some(*boot) {
                    self.open.clear();
                }
                self.base = (*wall_ns, *mono_ns);
                self.first.get_or_insert(*wall_ns);
            }
            Record::Open {
                sid, skind, meta, ..
            } if matches!(skind, StreamKind::Native | StreamKind::Snapshot) => {
                let m: Value = serde_json::from_slice(meta).unwrap_or_default();
                let path = m["path"].as_str().unwrap_or_default().to_string();
                self.open.insert(*sid, (path, *skind == StreamKind::Native));
            }
            Record::Data { sid, bytes, .. } if self.open.contains_key(sid) => {
                if let Some(x) = self.row(w) {
                    x[3] = x[3].saturating_add(bytes.len() as u64);
                }
            }
            Record::End { sid, seen, .. } => {
                if let Some((p, native)) = self.open.remove(sid) {
                    if native {
                        self.streams.native.insert(p, *seen);
                    } else {
                        self.streams.snapshots.insert(p);
                    }
                }
            }
            Record::Gap {
                sid,
                reason,
                start_wall,
                end_wall,
                start_mono,
                end_mono,
                lost,
                tail,
                ..
            } => {
                if let Some(x) = self.row(*start_wall) {
                    x[4] = x[4].saturating_add(1);
                }
                if let Some((p, _)) = self.open.get(sid) {
                    self.streams.gapped.insert(p.clone());
                }
                let tail = serde_json::from_slice(tail).unwrap_or_default();
                let (wl, mo) = ((*start_wall, *end_wall), (*start_mono, *end_mono));
                if *sid == 0
                    && *reason == GapReason::RecorderDown
                    && has_extent(&tail, wl, mo, *lost)
                {
                    self.off.push(wl);
                }
            }
            _ => {}
        }
    }

    /// Adds the calls and spreads each off span over the buckets it overlaps.
    fn finish(&mut self, calls: &[(bool, bool, u64)]) {
        for &(native, err, t) in calls {
            if let Some(x) = self.row(t) {
                let k = usize::from(!native);
                x[k] = x[k].saturating_add(1);
                x[2] = x[2].saturating_add(u64::from(err && !native));
            }
        }
        let Some(first) = self.first else { return };
        for (s, e) in std::mem::take(&mut self.off) {
            let (s, e) = (s.max(first), e.min(self.last));
            if e <= s {
                continue;
            }
            for i in (s - first) / self.bucket..=(e - 1 - first) / self.bucket {
                let lo = first + i * self.bucket;
                let ns = e.min(lo.saturating_add(self.bucket)) - s.max(lo);
                let x = self.rows.entry(i).or_default();
                x[5] = x[5].saturating_add(ns / 1_000_000);
            }
        }
    }

    /// The report's `timeline`: the buckets with anything in them.
    fn timeline(&self) -> Value {
        let first = self.first.unwrap_or(0);
        let rows = self.rows.iter().filter(|(_, r)| r.iter().any(|&n| n > 0));
        rows.map(|(&i, r)| {
            let start = first.saturating_add(i.saturating_mul(self.bucket));
            json!({"bucket": i, "start": wall(start), "harness_calls": r[0], "wire_calls": r[1],
                "wire_errors": r[2], "native_bytes": r[3], "gaps": r[4], "off_ms": r[5]})
        })
        .collect()
    }
}

/// The report as JSON, the exit code, and the text for stderr.
pub struct Report {
    pub json: Value,
    pub exit: u8,
    pub text: String,
}

/// Checks every entry of the input. `now_ns` is set for `--live`; `local` is `--local`, the
/// local entries a server copy is compared with; `known` (`--known`) adds the `secrets` check.
pub fn build(
    input: &Input,
    now_ns: Option<u64>,
    local: Option<&Input>,
    known: Option<&Rules>,
    files: Option<&[(String, u64)]>,
) -> Report {
    let is_server = input.kind == "server";
    let (mut exit, mut text, mut entries) = (0u8, String::new(), Vec::new());
    let (mut run, mut all) = (Value::Null, Vec::new());
    for e in &input.entries {
        let f = is_server.then(|| server::read(e, input.server));
        let want = f.as_ref().map(server::wanted).unwrap_or_default();
        let mut o = integrity::check(e, now_ns, &want);
        let mut checks = std::mem::take(&mut o.checks);
        if let Some(f) = &f {
            checks.extend(server::checks(e, f, &o, local));
        }
        checks.extend(known.map(|k| secrets::check(k, e)));
        let mut seen = Seen::new(e, BUCKET);
        seen.finish(&o.legs.timed);
        if let (Some(f), Some(c)) = (
            files,
            checks.iter_mut().find(|c| c.id == "subagents_present"),
        ) {
            legs::manifest(c, f, &seen.streams, &o.legs.covers);
        }
        let worst = checks.iter().map(|c| c.result).max().unwrap_or(Res::Pass);
        let partial = !o.sealed || input.kind == "live" || o.pruned;
        exit = exit.max(match worst {
            Res::Fail => 2,
            Res::Named => 1,
            _ => u8::from(partial),
        });
        if run.is_null() {
            run = o.start.get("run").cloned().unwrap_or(Value::Null);
        }
        let name = o.entry.clone().unwrap_or_else(|| folder(&e.dir));
        text += &self::text(&name, &o, &checks);
        let mut en = entry(name, &o, &checks);
        en["timeline"] = seen.timeline();
        entries.push(en);
        all.extend(checks);
    }
    let server = is_server.then(|| server::summary(&all));
    let result = ["pass", "named", "fail"][usize::from(exit)];
    let json = json!({"verifier": env!("CARGO_PKG_VERSION"), "input": input.kind, "run": run,
        "result": result, "exit": exit, "entries": entries, "server": server});
    Report { json, exit, text }
}

fn folder(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Model requests written (PRD-01 section 9): what the participant, status and heartbeat show.
fn requests(c: &Recount) -> u64 {
    c.get("http_post").saturating_add(c.get("ws_up_msgs"))
}

/// One S6 entry.
fn entry(name: String, o: &Outcome, checks: &[Check]) -> Value {
    let c = &o.counts;
    let requests = requests(c);
    let label = match (requests > 0, c.get("native_bytes") > 0) {
        (true, true) => "wire_and_native",
        (true, false) => "wire_only",
        (false, true) => "native_only",
        (false, false) => "none",
    };
    let gaps: Map<String, Value> = c
        .by_reason
        .iter()
        .map(|(r, (n, _, _))| (r.clone(), (*n).into()))
        .collect();
    let counts = json!({"model_requests": requests, "req_whole": c.get("req_whole"),
        "req_holes": c.get("req_holes"), "gaps": gaps, "lost_bytes": c.get("lost_bytes"),
        "lost_unknown": c.get("lost_unknown"), "after_stop_bytes": c.after_stop,
        "mask_failclosed_bytes": c.get("mask_failclosed_bytes"), "off_ms": c.get("off_ms"),
        "tunnels": c.get("tunnels"), "native_files": c.get("native_files"),
        "threads": o.legs.threads.len()});
    // The history check's facts are the S6 `history` object; its verdicts fill the thread rows.
    let hist = checks.iter().find(|k| k.id == "history_complete");
    let verdicts = hist.and_then(|k| k.facts.get("threads"));
    let threads: Vec<Value> = (o.legs.threads.iter())
        .map(|r| {
            let v = r["thread"].as_str().and_then(|t| verdicts?.get(t));
            let mut r = r.clone();
            r["history"] = v.cloned().unwrap_or_default();
            r
        })
        .collect();
    let checks: Vec<Value> = checks
        .iter()
        .map(|k| {
            json!({"id": k.id, "inv": k.inv, "result": k.result.name(), "why": k.why,
            "first": k.first, "facts": k.facts})
        })
        .collect();
    let s = |k: &str| o.start.get(k).cloned().unwrap_or(Value::Null);
    let reason = o.seal.as_ref().map(|s| &s["reason"]);
    json!({"entry": name, "sealed": o.sealed, "seal_reason": reason, "boots": o.boots,
        "connector": s("connector"), "keep": s("keep"), "label": label, "harness_log": o.legs.harness_log,
        "provider_errors": o.legs.provider_errors,
        "bytes": {"raw": o.raw, "compressed": o.compressed, "framing": o.raw.saturating_sub(o.data_bytes)},
        "links": o.links, "checks": checks, "counts": counts, "history": hist.map(|k| &k.facts), "threads": threads})
}

/// The text form of one entry: `why` per failed or named check, then the coverage line.
fn text(name: &str, o: &Outcome, checks: &[Check]) -> String {
    let c = &o.counts;
    let mut t = format!("goalcup-verify: entry {name}\n");
    let hist = (checks.iter().find(|k| k.id == "history_complete")).map(history::line);
    for k in checks.iter().filter(|k| k.result >= Res::Named) {
        t += &format!("  {} {}: {}\n", k.id, k.result.name(), k.why);
    }
    if !o.sealed {
        t += "  The entry is not sealed.\n";
    }
    t += &format!(
        "Recorded: {} requests, {} refused by the provider. {} {} Recording off: {} times, {} s.\n",
        requests(c),
        o.legs.refused,
        hist.unwrap_or_default(),
        o.legs.coverage,
        c.off_spans,
        c.get("off_ms") / 1000
    );
    t
}
