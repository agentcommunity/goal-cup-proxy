//! The `check --json` report (seam S6, PRD-08 section 4), its exit code, and the text form with
//! the coverage line (section 5). `why` is one plain line; no record byte is ever printed here.

use crate::input::{Input, Kind};
use crate::integrity::{self, Check, Outcome, Res};
use serde_json::{Map, Value, json};
use std::path::Path;

/// Checks a later slice builds: in the report as `skip`, so readers see every id now.
const LATER: &[&str] = &[
    "listed_hosts_decrypted",
    "conn_bytes",
    "mask_failclosed",
    "history_complete",
    "wire_keeps_up",
    "threads_match",
    "subagents_present",
];
const LATER_SERVER: &[&str] = &[
    "server_chunks_whole",
    "server_equals_local",
    "server_counts_match",
];
const NOT_BUILT: &str = "not built in this slice";

/// The report as JSON, the exit code, and the text for stderr.
pub struct Report {
    pub json: Value,
    pub exit: u8,
    pub text: String,
}

/// Checks every entry of the input. `now_ns` is set for `--live`.
pub fn build(input: &Input, server: bool, now_ns: Option<u64>) -> Report {
    let (mut exit, mut text, mut entries) = (0u8, String::new(), Vec::new());
    let mut run = Value::Null;
    for e in &input.entries {
        let o = integrity::check(e, now_ns);
        let mut checks = o.checks.clone();
        let later = LATER.iter().chain(if server { LATER_SERVER } else { &[] });
        checks.extend(later.map(|id| Check::new(id, None, Res::Skip, NOT_BUILT)));
        let worst = checks.iter().map(|c| c.result).max().unwrap_or(Res::Pass);
        let partial = !o.sealed || input.kind == Kind::Live || o.pruned;
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
        entries.push(entry(name, &o, &checks));
    }
    let result = ["pass", "named", "fail"][usize::from(exit)];
    let json = json!({"verifier": env!("CARGO_PKG_VERSION"), "input": input.kind.name(), "run": run,
        "result": result, "exit": exit, "entries": entries, "server": null});
    Report { json, exit, text }
}

fn folder(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// One S6 entry.
fn entry(name: String, o: &Outcome, checks: &[Check]) -> Value {
    let c = &o.counts;
    let requests = c.get("http_post") + c.get("ws_up_msgs");
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
        "tunnels": c.get("tunnels"), "native_files": c.get("native_files"), "threads": null});
    let checks: Vec<Value> = checks
        .iter()
        .map(|k| {
            json!({"id": k.id, "inv": k.inv, "result": k.result.name(), "why": k.why,
            "first": k.first, "facts": k.facts})
        })
        .collect();
    let s = |k: &str| o.start.get(k).cloned().unwrap_or(Value::Null);
    json!({"entry": name, "sealed": o.sealed, "seal_reason": o.seal_reason, "boots": o.boots,
        "connector": s("connector"), "keep": s("keep"), "label": label, "harness_log": null,
        "provider_errors": null,
        "bytes": {"raw": o.raw, "compressed": o.compressed, "framing": o.raw.saturating_sub(o.data_bytes)},
        "links": o.links, "checks": checks, "counts": counts, "history": null, "threads": []})
}

/// The text form of one entry: `why` per failed or named check, then the coverage line.
fn text(name: &str, o: &Outcome, checks: &[Check]) -> String {
    let c = &o.counts;
    let mut t = format!("goalcup-verify: entry {name}\n");
    for k in checks.iter().filter(|k| k.result >= Res::Named) {
        t += &format!("  {} {}: {}\n", k.id, k.result.name(), k.why);
    }
    if !o.sealed {
        t += "  The entry is not sealed.\n";
    }
    t += &format!(
        "Recorded: {} requests. Recording off: {} times, {} s. The harness's own log is not read \
         for this harness (optional). History check: skipped, {NOT_BUILT}.\n",
        c.get("http_post") + c.get("ws_up_msgs"),
        c.off_spans,
        c.get("off_ms") / 1000
    );
    t
}
