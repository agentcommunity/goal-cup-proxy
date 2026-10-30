//! The server copy check (PRD-08 section 3, seam S13): exit codes and results only.

mod common;

use common::*;
use goalcup_format::read::{part_files, read_index};
use goalcup_format::{EndReason, hex};
use goalcup_verify::server::IDS;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// Entry `d` as fetch-run lays it out: one chunk per index line, the entry row and the last
/// heartbeat from the seal's totals; `f` then changes the copy or its fetch.json.
fn fetch(d: &Path, name: &str, f: impl FnOnce(&Path, &mut Value)) -> PathBuf {
    let out = fetched(d, &tmp(name)).join(ENTRY);
    let (mut chunks, mut raw, mut prev) = (vec![], 0, "0".repeat(64));
    for p in part_files(&d.join("rec")).unwrap() {
        let (b, mut c0) = (fs::read(&p).unwrap(), 0);
        for l in read_index(&p.with_extension("").with_extension("idx")).unwrap() {
            let b = &b[c0..l.c_end as usize];
            chunks.push(
                json!({"offset": raw, "length": l.raw_end - raw, "bytes": b.len(),
                "sha256": hex(&Sha256::digest(b)), "first_seq": chunks.len(), "last_seq": l.seq,
                "prev": prev, "head": hex(&l.hash), "r2_key": "k", "received_at": null}),
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
    let hb = json!({"final": true, "recording": "ok", "model_requests": n, "counts": t});
    let mut doc = json!({"v": 1, "run_id": RUN, "entry_id": ENTRY, "upload_offset": raw,
        "next_seq": chunks.len(), "head": prev, "model_requests": n,
        "final": {"seq": chunks.len() - 1, "head": prev, "at": null}, "complete": true,
        "missing": null, "last_heartbeat": hb, "heartbeats": [hb], "chunks": chunks});
    f(&out, &mut doc);
    fs::write(out.join("fetch.json"), doc.to_string()).unwrap();
    out
}

/// A sealed entry with four links, fetched, changed by `f` and checked with `--local`.
fn server(name: &str, f: impl FnOnce(&Path, &mut Value)) -> (i32, Value) {
    let mut e = Entry::new(name);
    e.standard();
    e.count();
    e.count();
    let d = e.seal();
    let local = ["--server", "--local", d.to_str().unwrap()];
    check(&fetch(&d, name, f), &local)
}

/// Rewrites the fetched rec.gcr.zst.
fn bytes(out: &Path, f: impl FnOnce(&mut Vec<u8>)) {
    let mut b = fs::read(out.join("rec.gcr.zst")).unwrap();
    f(&mut b);
    fs::write(out.join("rec.gcr.zst"), b).unwrap();
}

#[test]
fn copy_equal_to_local_part_files_passes() {
    let (code, v) = server("equal", |_, _| {});
    assert_eq!(code, 0, "{:?}", ids_with(&v, "fail"));
    for id in IDS.split(' ') {
        assert_eq!(result(&v, id), "pass", "{id}");
    }
    for k in ["whole", "equals_local", "db_match"] {
        assert_eq!(v["server"][k], true, "{k}");
    }
}

#[test]
fn fw9_one_flipped_byte_fails_the_copy() {
    let (code, v) = server("flip", |out, _| bytes(out, |b| b[100] ^= 1));
    assert_eq!(code, 2);
    assert_eq!(result(&v, "server_chunks_whole"), "fail");
    assert_eq!(result(&v, "server_equals_local"), "fail");
}

#[test]
fn missing_middle_chunk_fails() {
    let (code, v) = server("hole", |out, doc| {
        let gone = doc["chunks"].as_array_mut().unwrap().remove(1);
        let at = doc["chunks"][0]["bytes"].as_u64().unwrap() as usize;
        let n = gone["bytes"].as_u64().unwrap() as usize;
        bytes(out, |b| drop(b.drain(at..at + n)));
    });
    assert_eq!((code, result(&v, "server_chunks_whole")), (2, "fail"));
}

#[test]
fn wrong_row_counts_fail_counts_match() {
    for at in ["/model_requests", "/last_heartbeat/counts/off_ms"] {
        let (code, v) = server("row", |_, doc| *doc.pointer_mut(at).unwrap() = 7.into());
        let r = result(&v, "server_counts_match");
        assert_eq!((code, r), (2, "fail"), "{at}");
    }
}

#[test]
fn open_end_is_named_and_counts_skip() {
    let (code, v) = server("open", |_, doc| doc["final"] = Value::Null);
    assert_eq!(code, 1, "{:?}", ids_with(&v, "fail"));
    assert_eq!(result(&v, "server_chunks_whole"), "named");
    assert_eq!(result(&v, "server_counts_match"), "skip");
}

/// L-FW3: a pruned entry is named on its own, and the server copy stands in for it.
#[test]
fn fw3_pruned_entry_is_named_and_server_copy_stands_in() {
    let mut e = Entry::new("pruned");
    e.standard();
    e.sleep(3_600_000);
    let now = e.clock.now();
    e.w.as_mut().unwrap().rotate_if_due(now).unwrap().unwrap();
    e.exchange(EndReason::Complete);
    e.count();
    let d = e.seal();
    let out = fetch(&d, "pruned", |_, _| {});
    fs::remove_file(&part_files(&d.join("rec")).unwrap()[0]).unwrap();
    let (code, v) = check(&d, &[]);
    assert_eq!((code, ids_with(&v, "fail").len()), (1, 0));
    assert_eq!(ids_with(&v, "named"), ["parts_pruned"]);
    let (_, v) = check(&out, &["--server", "--local", d.to_str().unwrap()]);
    assert_eq!(result(&v, "server_equals_local"), "skip");
}
