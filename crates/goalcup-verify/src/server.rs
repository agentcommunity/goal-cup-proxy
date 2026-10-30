//! The server copy (PRD-08 section 3, "Server copy"): a fetched entry (seam S13,
//! `scripts/fetch-run.sh`) against its chunk list, the local part files and the seal. A chunk's
//! `offset` and `length` are raw GCR1 bytes, its `bytes` the compressed size (docs/UPLOAD-API.md).

use crate::input::{self, Entry, FETCHED, Input};
use crate::integrity::{Check, Outcome, Res};
use goalcup_format::{Record, hex};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read};

/// The three checks, in report order.
pub const IDS: [&str; 3] = ["server_chunks_whole", "server_equals_local", "server_counts_match"];

/// `<entry>/fetch.json`, or the result all three checks take without it: fetch-run always writes
/// it beside the copy (S13), so a copy without one has no chunk list to judge it whole by.
pub type Fetched = Result<Value, (Res, String)>;

pub fn read(e: &Entry) -> Fetched {
    let b = std::fs::read(e.dir.join("fetch.json"));
    let b = b.map_err(|_| (Res::Fail, "no fetch.json beside the copy".to_string()))?;
    serde_json::from_slice(&b).map_err(|x| (Res::Fail, format!("fetch.json is not JSON: {x}")))
}

fn chunks(f: &Value) -> &[Value] {
    f["chunks"].as_array().map_or(&[], Vec::as_slice)
}

/// The link seqs whose hashes the checks need: each chunk's end.
pub fn wanted(f: &Fetched) -> HashSet<u64> {
    let all = f.iter().flat_map(chunks);
    all.filter_map(|c| c["last_seq"].as_u64()).collect()
}

/// The three checks for one fetched entry; `local` is the `--local` input.
pub fn checks(e: &Entry, f: &Fetched, o: &Outcome, local: Option<&Input>) -> Vec<Check> {
    let f = match f {
        Ok(f) => f,
        Err((r, why)) => return IDS.map(|id| Check::new(id, None, *r, why)).into(),
    };
    let mut whole = Check::new("server_chunks_whole", None, Res::Pass, "");
    // fetch-run stops the copy before a bad or missing object or a chain break, and names it.
    let (cut, at) = (&f["missing"]["why"], f["missing"]["offset"].as_u64().unwrap_or(0));
    let cut = cut.as_str().map(|w| (format!("the copy stops at byte {at}: {w}"), at));
    if let Some((why, at)) = cut.or_else(|| broken(e, f, o)) {
        (whole.result, whole.why) = (Res::Fail, why);
        whole.first = Some(json!({"o": at, "sid": 0, "wall": null}));
    } else if f["final"].is_null() {
        let why = "the server copy has an open end (no final heartbeat)";
        (whole.result, whole.why) = (Res::Named, why.into());
    }
    whole.facts.insert("chunks".into(), chunks(f).len().into());
    vec![whole, equal(e, o, local), counts(f, o)]
}

/// The first break in the chunk list, with the raw offset where it is.
fn broken(e: &Entry, f: &Value, o: &Outcome) -> Option<(String, u64)> {
    let mut file = File::open(e.dir.join(FETCHED)).map(io::BufReader::new);
    let (mut off, mut seq, mut prev, mut sum) = (0u64, 0u64, "0".repeat(64), 0u64);
    for c in chunks(f) {
        let n = |k: &str| c[k].as_u64().unwrap_or(u64::MAX);
        let (head, last) = (c["head"].as_str().unwrap_or("-"), n("last_seq"));
        let sha = digest([file.as_mut()], n("bytes"));
        let why = if n("offset") != off {
            format!("a chunk starts at byte {} where {off} was due", n("offset"))
        } else if n("first_seq") != seq || last < seq {
            format!("the chunk at byte {off} does not run on from seq {seq}")
        } else if c["prev"] != prev.as_str() {
            format!("the chunk at byte {off} does not follow the previous head")
        } else if o.heads.get(&last).map(|h| hex(h)).as_deref() != Some(head) {
            format!("the record holds another head at seq {last}")
        } else if sha.is_none() || sha.as_deref() != c["sha256"].as_str() {
            format!("the chunk at byte {off} is not the bytes its SHA-256 names")
        } else {
            (off, seq) = (off.saturating_add(n("length")), last.saturating_add(1));
            (prev, sum) = (head.into(), sum.saturating_add(n("bytes")));
            continue;
        };
        return Some((why, off));
    }
    let (len, fin) = (input::compressed(e), &f["final"]);
    if sum != len {
        return Some((format!("the copy holds {len} bytes, the chunks {sum}"), off));
    }
    let (seq, end) = (&fin["seq"], chunks(f).last());
    let why = format!("the chunks do not end at the final link, seq {seq}");
    let short = end.map(|c| (&c["last_seq"], &c["head"])) != Some((seq, &fin["head"]));
    (!fin.is_null() && short).then_some((why, off))
}

/// The SHA-256 of the first `n` bytes of `rs` read in order, in hex; `None` when one fails.
fn digest<R: Read, E>(rs: impl IntoIterator<Item = Result<R, E>>, mut n: u64) -> Option<String> {
    let mut h = Sha256::new();
    for r in rs {
        n -= io::copy(&mut r.ok()?.take(n), &mut h).ok()?;
    }
    Some(hex(&h.finalize()))
}

/// `server_equals_local`: the copy and the local part files, over the bytes both hold.
fn equal(e: &Entry, o: &Outcome, local: Option<&Input>) -> Check {
    let c = |res, why: &str| Check::new("server_equals_local", None, res, why);
    // The first PART of each local entry: its entry id, and whether it starts the entry (PRD-01).
    let first = |l: &Entry| Some((l.clone(), input::open(l).ok()?.0.next()?.ok()?.rec));
    let id = o.entry.as_deref().unwrap_or("-");
    let ours = |f: &(Entry, Record)| matches!(&f.1, Record::Part { entry, .. } if entry == id);
    let mut found = local.into_iter().flat_map(|i| &i.entries).filter_map(first);
    let pruned = "local part files were pruned; use the server copy";
    let Some((l, rec)) = found.find(ours) else {
        // A `--local` folder whose part files were all pruned resolves to no entry (main.rs).
        let gone = local.is_some_and(|i| i.entries.is_empty());
        return c(Res::Skip, if gone { pruned } else { "no --local entry with this entry id" });
    };
    if !matches!(rec, Record::Part { raw_offset: 0, .. }) {
        return c(Res::Skip, pruned);
    }
    let n = input::compressed(e).min(input::compressed(&l));
    let all = |x: &Entry| digest(x.files.iter().map(|p| File::open(&p.0)), n);
    match (all(e), all(&l)) {
        (Some(a), Some(b)) if a == b => c(Res::Pass, ""),
        (Some(_), Some(_)) => c(Res::Fail, "the copy differs from the local part files"),
        _ => c(Res::Fail, "the copy or the local part files cannot be read"),
    }
}

/// `server_counts_match`: the server's entry row and last heartbeat against the seal's totals.
fn counts(f: &Value, o: &Outcome) -> Check {
    let c = |res, why: &str| Check::new("server_counts_match", None, res, why);
    let (fin, hb) = (&f["final"], &f["last_heartbeat"]);
    let (hc, last) = (&hb["counts"], chunks(f).last().unwrap_or(&Value::Null));
    if fin.is_null() {
        return c(Res::Skip, "not sealed");
    }
    let Some(seal) = &o.seal else {
        return c(Res::Fail, "the server copy holds no seal");
    };
    let t = |k: &str| seal["totals"][k].as_u64().unwrap_or(0);
    let l = |k: &str| last[k].as_u64().unwrap_or(0);
    let requests = t("http_post").saturating_add(t("ws_up_msgs"));
    // PRD-06 section 6. `streams_in_gap` is the live recorder's count and not in the totals; at
    // the seal no stream is open, so it is taken as 0 here.
    let lost = ["gaps", "lost_bytes", "lost_unknown", "off_ms"].map(t);
    let lossy = seal["reason"] == "recovered" || lost.iter().any(|n| *n > 0);
    let recording = if lossy { "gaps" } else { "ok" };
    let mut row = json!({"model_requests": f["model_requests"], "upload_offset": f["upload_offset"],
        "final.seq": fin["seq"], "last_heartbeat.recording": hb["recording"],
        "last_heartbeat.model_requests": hb["model_requests"], "counts.gaps": hc["gaps"],
        "counts.lost_bytes": hc["lost_bytes"], "counts.off_ms": hc["off_ms"]});
    let mut want = json!({"model_requests": requests,
        "upload_offset": l("offset").saturating_add(l("length")),
        "final.seq": l("last_seq"), "last_heartbeat.recording": recording,
        "last_heartbeat.model_requests": requests, "counts.gaps": t("gaps"),
        "counts.lost_bytes": t("lost_bytes"), "counts.off_ms": t("off_ms")});
    if let Some(v) = hc.get("lost_unknown") {
        row["counts.lost_unknown"] = v.clone();
        want["counts.lost_unknown"] = t("lost_unknown").into();
    }
    let (mut d, row) = (vec![], row.as_object().into_iter().flatten());
    for (k, v) in row.filter(|(k, v)| want[k] != **v) {
        d.push(format!("{k} {v} against {}", want[k]));
    }
    if d.is_empty() {
        return c(Res::Pass, "");
    }
    let why = format!("the server's row differs from the seal: {}", d.join(", "));
    c(Res::Fail, &why)
}

/// S6 `server` from the checks of every entry: the chunk count; `whole` when every copy's chunks
/// pass; `equals_local` and `db_match` true when every entry passes, false on a `fail`, else null.
pub fn summary(all: &[Check]) -> Value {
    // The least and the most severe result of one check over the entries.
    let span = |id: &str| {
        let r = all.iter().filter(|c| c.id == id).map(|c| c.result);
        (r.clone().min(), r.max())
    };
    let tri = |id| match span(id) {
        (_, Some(Res::Fail)) => json!(false),
        (Some(Res::Pass), Some(Res::Pass)) => json!(true),
        _ => Value::Null,
    };
    let n = all.iter().filter_map(|c| c.facts.get("chunks")?.as_u64());
    let chunks: u64 = n.sum();
    let whole = span("server_chunks_whole").1 == Some(Res::Pass);
    json!({"chunks": chunks, "whole": whole, "equals_local": tri("server_equals_local"),
        "db_match": tri("server_counts_match")})
}
