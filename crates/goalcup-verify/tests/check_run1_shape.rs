//! PRD-08 R1(a), R3 and R4 at record level (ledger B10; points 1, 2 and 3): the run 1 shape
//! fails exactly the checks that must fail it, and the same record without its losses, with a
//! killed recorder or with a pinned model host does not.

mod common;

use common::run1::{IDS, Shape, TURN, run1};
use common::*;
use goalcup_format::read::{part_files, read_index};
use goalcup_format::{EndReason, StreamKind, hex};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

/// What fails run 1's shape: the mid-run drop shows as `history_complete`.
const FAILS: [&str; 6] = [
    "gap_has_end",
    "streams_closed",
    "wire_keeps_up",
    "threads_match",
    "subagents_present",
    "history_complete",
];

fn sorted(ids: impl IntoIterator<Item = impl ToString>) -> Vec<String> {
    let mut ids: Vec<String> = ids.into_iter().map(|s| s.to_string()).collect();
    ids.sort();
    ids
}

/// The entry as fetch-run lays it out, one chunk per index line. Unless `honest` (a record without
/// losses), it claims one model request more than the seal's totals say.
fn fetch(d: &Path, honest: bool) -> PathBuf {
    let out = fetched(d, &tmp("r1-fetched")).join(ENTRY);
    let (mut chunks, mut raw, mut prev) = (vec![], 0, "0".repeat(64));
    for p in part_files(&d.join("rec")).unwrap() {
        let (b, mut c0) = (fs::read(&p).unwrap(), 0);
        for l in read_index(&p.with_extension("").with_extension("idx")).unwrap() {
            let b = &b[c0..l.c_end as usize];
            chunks.push(
                json!({"offset": raw, "length": l.raw_end - raw, "bytes": b.len(),
                "sha256": hex(&Sha256::digest(b)), "first_seq": chunks.len(), "last_seq": l.seq,
                "prev": prev, "head": hex(&l.hash)}),
            );
            (raw, prev, c0) = (l.raw_end, hex(&l.hash), l.c_end as usize);
        }
    }
    let mut seal = Value::Null;
    edit_event(&mut read_records(&d.join("rec")), "seal", 0, |v| {
        seal = v.clone()
    });
    let t = &seal["totals"];
    let n = t["http_post"].as_u64().unwrap() + t["ws_up_msgs"].as_u64().unwrap();
    let recording = if honest { "ok" } else { "gaps" };
    let hb = json!({"recording": recording, "model_requests": n, "counts": t});
    let asked = n + u64::from(!honest);
    let doc = json!({"upload_offset": raw, "model_requests": asked, "last_heartbeat": hb,
        "final": {"seq": chunks[chunks.len() - 1]["last_seq"], "head": prev}, "chunks": chunks});
    fs::write(out.join("fetch.json"), doc.to_string()).unwrap();
    out
}

/// Copies a folder: the K2 and K7 exports of PRD-12 (section 12). With `GOALCUP_FIXTURES_OUT`
/// set, the run 1 test writes `k2-part`, `k2-fetched` (lying `fetch.json`), `k7-part` and
/// `k7-fetched` (honest, from the Clean shape) there, to be committed under
/// `scripts/pad/fixtures/`.
fn export(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for f in fs::read_dir(from).unwrap().map(|f| f.unwrap().path()) {
        match f.is_dir() {
            true => export(&f, &to.join(f.file_name().unwrap())),
            false => drop(fs::copy(&f, to.join(f.file_name().unwrap())).unwrap()),
        }
    }
}

#[test]
fn run1_shape_fails_exactly_its_checks_and_reads_six_wire_errors() {
    let (d, k7) = (run1("r1-lossy", Shape::Lossy), run1("r1-pair", Shape::Clean));
    let k7f = fetch(&k7, true);
    if let Ok(out) = env::var("GOALCUP_FIXTURES_OUT") {
        let out = Path::new(&out);
        for (from, to) in [(&d, "k2-part"), (&fetch(&d, false), "k2-fetched")] {
            export(from, &out.join(to));
        }
        export(&k7, &out.join("k7-part"));
        export(&k7f, &out.join("k7-fetched"));
    }
    let (code, v) = check(&k7f, &["--server", "--local", k7.to_str().unwrap()]);
    assert_eq!((code, ids_with(&v, "fail")), (0, Vec::<String>::new()));
    assert_eq!(result(&v, "server_counts_match"), "pass");
    let (code, v) = check(&d, &[]);
    assert_eq!(code, 2);
    assert_eq!(sorted(ids_with(&v, "fail")), sorted(FAILS));
    // The root thread's row fails its history; a child's does not; no finding quotes a turn.
    let hist = |t: &str| {
        let rows = v["entries"][0]["threads"].as_array().unwrap();
        rows.iter().find(|r| r["thread"] == t).unwrap()["history"].clone()
    };
    assert_eq!(hist(IDS[0]), "fail");
    assert_ne!(hist(IDS[1]), "fail");
    assert!(!item(&v, "history_complete")["why"].as_str().unwrap().contains(TURN));
    let wire = v["entries"][0]["provider_errors"]["wire"]
        .as_object()
        .unwrap();
    assert_eq!(wire.values().map(|n| n.as_u64().unwrap()).sum::<u64>(), 6);
}

#[test]
fn run1_shape_as_a_server_copy_fails_the_count_check_too() {
    let d = run1("r1-server", Shape::Lossy);
    let (code, v) = check(
        &fetch(&d, false),
        &["--server", "--local", d.to_str().unwrap()],
    );
    assert_eq!(code, 2);
    let want = sorted(FAILS.iter().chain(&["server_counts_match"]));
    assert_eq!(sorted(ids_with(&v, "fail")), want);
}

#[test]
fn a_vendor_tunnel_does_not_name_the_loss_of_model_calls() {
    let (code, v) = check(&run1("r1-vendor", Shape::Vendor), &[]);
    assert_eq!(code, 2);
    assert_eq!(sorted(ids_with(&v, "fail")), sorted(FAILS));
}

#[test]
fn the_coverage_line_counts_every_request_written() {
    let mut e = Entry::new("r1-other");
    e.standard();
    // A POST to a listed host that is no model call: written as a request, not a call.
    let c = e.conn();
    let head = json!({"method": "POST", "path": "/v1/other", "ver": "1.1", "headers": [],
        "masked": [], "coding": null});
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    e.end(q, EndReason::Complete, 0, json!({"wire": 0}));
    e.end(
        c,
        EndReason::Complete,
        0,
        json!({"up": 10, "down": 0, "plain_up": 0, "plain_down": 0}),
    );
    e.count();
    let d = e.seal();
    let args = ["check", d.to_str().unwrap(), "--json"];
    let out = Command::new(env!("CARGO_BIN_EXE_goalcup-verify"))
        .args(args)
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let n = v["entries"][0]["counts"]["model_requests"]
        .as_u64()
        .unwrap();
    assert_eq!(n, 3);
    assert!(String::from_utf8_lossy(&out.stderr).contains(&format!(" {n} requests")));
}

#[test]
fn the_same_record_without_its_losses_exits_0() {
    let (code, v) = check(&run1("r1-clean", Shape::Clean), &[]);
    assert_eq!((code, ids_with(&v, "fail")), (0, Vec::<String>::new()));
    assert_eq!(result(&v, "history_complete"), "pass");
}

#[test]
fn a_killed_recorder_is_named_not_failed() {
    let (code, v) = check(&run1("r3-killed", Shape::Killed), &[]);
    assert_eq!((code, ids_with(&v, "fail")), (1, Vec::<String>::new()));
    assert_eq!(result(&v, "wire_keeps_up"), "named");
    assert_eq!(result(&v, "threads_match"), "named");
    assert_eq!(result(&v, "history_complete"), "named");
}

#[test]
fn a_pinned_model_host_exits_1_with_the_log_check_named() {
    // The shared list's host, and the same host as the participant's own (kind `other`).
    for (name, shape) in [("r4-dodger", Shape::Dodger), ("r4-own", Shape::DodgerOwn)] {
        let (code, v) = check(&run1(name, shape), &[]);
        assert_eq!(
            (code, ids_with(&v, "fail")),
            (1, Vec::<String>::new()),
            "{name}"
        );
        assert_eq!(result(&v, "listed_hosts_decrypted"), "named", "{name}");
        assert_eq!(result(&v, "wire_keeps_up"), "named", "{name}");
        assert_eq!(result(&v, "history_complete"), "skip", "{name}");
    }
}
