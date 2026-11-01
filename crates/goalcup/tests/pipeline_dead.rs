//! PRD-02 section 9.4 and section 11 "In null mode" (`pipeline_dead`), points 1, 3, 5 and 6: a
//! dead entry is closed with one `recorder_down` gap and a seal inside the recovery frame, nothing
//! after its `R` line, a sealed `status.json` that names the dead recorder and carries the totals
//! of the files, and no run CA key; a live recorder's lock is never taken over, and a second close
//! changes nothing. The fake key is built from pieces.

mod common;

use bytes::Bytes;
use common::recorder::{entry_dir, real, records, wait_settled};
use goalcup::pipeline::status::Mode;
use goalcup::pipeline::{
    Closed, Cursors, EndReason, GapReason, OpenFlags, Status, StreamKind, close_dead_entry,
    entry_totals, seal_null_for_tests,
};
use goalcup::watchdog::{Cause, Exit, KEY_FILE, Prev, SealReason};
use goalcup_format::{IndexKind, Record};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const BODY: &[u8] = b"POST /v1/messages HTTP/1.1\r\n\r\n{\"a\":1}";
const DEAD_PID: u32 = 4242;

/// A dead entry: one request written and linked by the real pipeline, then the entry folder copied
/// as it stood at that durable point, unsealed (the pipeline's threads run on in the first folder,
/// never in the copy). Its `status.json` names a dead recorder, and a fake key is planted.
fn dead_entry() -> PathBuf {
    let live = entry_dir();
    let rec = real(live.path());
    let mut req = rec.open(StreamKind::HttpReq, 0, 0, 0, OpenFlags::POST, json!({}));
    req.data(rec.now(), Bytes::from_static(BODY));
    req.end(rec.now(), EndReason::Complete, None);
    wait_settled(&rec, |c| c.wire_bytes == BODY.len() as u64);
    let dead = entry_dir().path().join("e00000000000000d1");
    copy(live.path(), &dead);
    let mut s = Status::read(&dead).unwrap_or_default();
    (s.pid, s.start_id) = (DEAD_PID, 7);
    s.write(&dead).unwrap();
    let key = [
        "-----BEGIN ",
        "PRIVATE KEY-----\nZmFrZQ==\n-----END ",
        "PRIVATE KEY-----\n",
    ];
    std::fs::write(dead.join("rec").join(KEY_FILE), key.concat()).unwrap();
    dead
}

/// Copies the files of `from` into `to`, skipping temp files a live writer may rename away.
fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let name = e.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        if e.file_type().unwrap().is_dir() {
            copy(&e.path(), &to.join(&name));
        } else {
            std::fs::copy(e.path(), to.join(&name)).unwrap();
        }
    }
}

/// Every file under `dir` but the lock, with its bytes.
fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(files(&p));
        } else if p.file_name().is_some_and(|n| n != "lock") {
            out.insert(p.clone(), std::fs::read(&p).unwrap());
        }
    }
    out
}

fn event(json: &[u8]) -> Value {
    serde_json::from_slice(json).unwrap_or(Value::Null)
}

/// The `recorder_down` gaps' tails, each with start before end, and the seal event.
fn down_and_seal(dir: &Path) -> (Vec<Value>, Value) {
    let recs = records(dir);
    let mut down = Vec::new();
    for r in &recs {
        if let Record::Gap {
            sid: 0,
            reason: GapReason::RecorderDown,
            start_wall,
            end_wall,
            start_mono,
            end_mono,
            tail,
            ..
        } = r
        {
            assert!(start_mono < end_mono || start_wall < end_wall);
            down.push(event(tail));
        }
    }
    // Nothing follows the seal: the last part file read whole ends with the seal and its LINK,
    // at the index's last line, an `R` line.
    let parts = goalcup_format::read::part_files(&dir.join("rec")).unwrap();
    let last = parts.last().unwrap();
    let whole: Vec<Record> = goalcup_format::read::decode(std::fs::File::open(last).unwrap())
        .unwrap()
        .map(|i| i.unwrap().rec)
        .collect();
    assert!(matches!(whole.last(), Some(Record::Link { .. })));
    let Record::Event { json, .. } = &whole[whole.len() - 2] else {
        panic!("the seal event comes right before the last LINK");
    };
    let name = last.file_name().unwrap().to_str().unwrap();
    let idx = last.with_file_name(format!("{}.idx", name.trim_end_matches(".gcr.zst")));
    let line = *goalcup_format::read::read_index(&idx)
        .unwrap()
        .last()
        .unwrap();
    assert_eq!(line.kind, IndexKind::R);
    assert_eq!(std::fs::metadata(last).unwrap().len(), line.c_end);
    (down, event(json))
}

/// Point 3 (D14): a lock held by a live recorder is never taken over, and nothing is written.
#[test]
fn a_held_lock_is_still_recording_and_writes_nothing() {
    let dir = dead_entry();
    let lock = std::fs::File::create(dir.join("rec").join("lock")).unwrap();
    lock.lock().unwrap();
    let before = files(&dir);
    assert!(close_dead_entry(&dir).is_err());
    assert_eq!(files(&dir), before);
}

/// Points 1, 5 and 6: a dead entry closes `recovered` inside one recovery frame; the sealed
/// `status.json` keeps the dead recorder's pid and carries the files' totals; the key is gone; a
/// second close finds it sealed and changes nothing.
#[test]
fn a_dead_entry_closes_once_with_one_gap_and_the_files_totals() {
    let dir = dead_entry();
    let Ok(Closed::Sealed(facts)) = close_dead_entry(&dir) else {
        panic!("the dead entry is sealed");
    };
    let (down, seal) = down_and_seal(&dir);
    assert_eq!(down.len(), 1);
    assert_eq!(down[0]["cause"], "abandoned");
    assert_eq!(
        (&seal["reason"], &seal["harness"]),
        (&json!("recovered"), &json!("unknown"))
    );
    let totals = entry_totals(&dir);
    assert_eq!(
        seal["totals"],
        serde_json::to_value(&totals.counts).unwrap()
    );
    assert_eq!(facts.totals, totals);
    let st = Status::read(&dir).expect("status.json");
    assert_eq!(st.mode, Mode::Sealed);
    assert_eq!(st.seal.map(|s| s.reason), Some(SealReason::Recovered));
    assert_eq!(
        (st.written.counts, st.off.spans),
        (totals.counts, totals.off_spans)
    );
    assert_eq!((st.pid, st.start_id), (DEAD_PID, 7));
    assert!(!dir.join("rec").join(KEY_FILE).exists());

    let after = files(&dir);
    assert!(matches!(close_dead_entry(&dir), Ok(Closed::AlreadySealed)));
    assert_eq!(files(&dir), after);
}

/// Points 5 and 6 (section 11 "In null mode"): with `counts.jsonl` behind the index (the recorder
/// died between the index line and the append, or a power loss), the seal's totals are the
/// recovery COUNT's, which `entry_totals` and the closed entry's `status.json` also carry.
#[test]
fn a_counts_log_behind_the_index_still_seals_the_real_totals() {
    for null in [false, true] {
        let dir = dead_entry();
        std::fs::write(dir.join("counts.jsonl"), b"").unwrap();
        let facts = if null {
            let prev = Prev::Cause(Cause::NullMode {
                exits: 1,
                last: None,
            });
            seal_null_for_tests(&dir, SealReason::HarnessExit, prev).unwrap()
        } else {
            let Ok(Closed::Sealed(facts)) = close_dead_entry(&dir) else {
                panic!("the dead entry is sealed");
            };
            facts
        };
        let (_, seal) = down_and_seal(&dir);
        let totals = entry_totals(&dir);
        assert_eq!(totals.counts.wire_bytes, BODY.len() as u64);
        assert_eq!(
            seal["totals"],
            serde_json::to_value(&totals.counts).unwrap()
        );
        assert_eq!(facts.totals, totals);
        if !null {
            let st = Status::read(&dir).expect("status.json");
            assert_eq!(st.written.counts, totals.counts);
        }
    }
}

/// Points 3 and 6 (section 11 "In null mode"): the null recorder's seal writes one
/// `recorder_down` gap with its `null_mode` cause and the seal with the real reason.
#[test]
fn the_null_seal_names_null_mode_and_the_real_reason() {
    let dir = dead_entry();
    let last = Exit::Code(70);
    let prev = Prev::Cause(Cause::NullMode {
        exits: 3,
        last: Some(last),
    });
    let facts = seal_null_for_tests(&dir, SealReason::HarnessExit, prev).unwrap();
    let (down, seal) = down_and_seal(&dir);
    assert_eq!(down.len(), 1);
    assert_eq!(
        (&down[0]["cause"], &down[0]["exits"]),
        (&json!("null_mode"), &json!(3))
    );
    assert_eq!(
        (&seal["reason"], &seal["harness"]),
        (&json!("harness_exit"), &json!("exited"))
    );
    assert_eq!(facts.totals, entry_totals(&dir));
    assert!(!dir.join("rec").join(KEY_FILE).exists());
}

/// Point 5: after a reboot (`cursors.json` names another `boot_id`, so its durable index length
/// cuts the index first) the seal's totals still equal the files' totals.
#[test]
fn a_dead_entry_after_a_reboot_seals_the_files_totals() {
    let dir = dead_entry();
    let mut c = Cursors::read(&dir);
    c.boot_id = Some("0".repeat(32));
    c.write(&dir).unwrap();
    let Ok(Closed::Sealed(facts)) = close_dead_entry(&dir) else {
        panic!("the dead entry is sealed");
    };
    let (down, seal) = down_and_seal(&dir);
    assert_eq!(down.len(), 1);
    let totals = entry_totals(&dir);
    assert_eq!(
        seal["totals"],
        serde_json::to_value(&totals.counts).unwrap()
    );
    assert_eq!(facts.totals, totals);
}

/// Point 5 and a right count: a boot whose first part file is pruned after upload, with
/// `counts.jsonl` behind its last LINK (the append after the last compaction lost to a power
/// loss), seals the bytes it wrote. The recount starts from the line the prune left and adds the
/// records after it, DATA on the stream opened in the pruned part included, and a `fail_closed`
/// MASK after the line though the start event that names the rule is pruned. No `counts.jsonl`
/// line names a LINK that is not written, so a line never gives a LINK another LINK's count.
#[test]
fn a_pruned_boot_with_counts_behind_seals_the_bytes_written() {
    const MIB: usize = 1 << 20;
    // SAFETY: set before the recorder starts; only this test's own writer reads it.
    unsafe { std::env::set_var("GOALCUP_TEST_ROTATE_MB", "1") };
    let live = entry_dir();
    let rec = real(live.path());
    let flags = OpenFlags::POST | OpenFlags::DECRYPTED;
    let mut req = rec.open(StreamKind::HttpReq, 0, 0, 0, flags, json!({}));
    req.data(rec.now(), Bytes::from_static(BODY));
    // A WebSocket open across the prune: its OPEN is pruned, its messages go on after the line.
    let mut ws = rec.open(StreamKind::WsUp, 0, 0, 0, OpenFlags::NONE, json!({}));
    // Bytes that do not compress, so the part files reach the rotation size.
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let noise = (0..MIB).map(|_| {
        x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (x >> 56) as u8
    });
    let chunk = Bytes::from(noise.collect::<Vec<u8>>());
    let mut sent = BODY.len() as u64;
    let mut send = |ws: &mut goalcup::pipeline::StreamHandle, n: usize| {
        for _ in 0..n {
            ws.ws(rec.now(), 0x1, true, chunk.clone());
            sent += MIB as u64;
            let want = sent;
            wait_settled(&rec, move |c| c.wire_bytes == want);
        }
    };
    send(&mut ws, 7);
    let parts = goalcup_format::read::part_files(&live.path().join("rec")).unwrap();
    assert!(parts.len() >= 2, "the entry rotated");
    // What the prune leaves: `counts.jsonl` synced, its compacted line at the latest LINK.
    let log = std::fs::read(live.path().join("counts.jsonl")).unwrap();
    let line = log.split(|&b| b == b'\n').next().unwrap().to_vec();
    send(&mut ws, 3);
    // A message that fails closed: a 20 KiB key candidate the masker cannot decide.
    let closed = format!(
        r#"{{"type":"response.create","input":"sk-{}"}}"#,
        "A9".repeat(10 * 1024)
    );
    ws.ws(rec.now(), 0x1, true, Bytes::from(closed.clone()));
    sent += closed.len() as u64;
    req.end(rec.now(), EndReason::Complete, None);
    wait_settled(&rec, |c| c.http_post == 1 && c.ws_up_msgs == 11);
    let total = sent;
    let fail_closed = rec.durable().borrow().totals.counts.mask_failclosed_bytes;
    assert!(fail_closed > 0, "the message failed closed");

    let dead = entry_dir().path().join("e00000000000000d2");
    copy(live.path(), &dead);
    let rec_dir = dead.join("rec");
    let oldest = goalcup_format::read::part_files(&rec_dir).unwrap()[0].clone();
    let name = oldest.file_name().unwrap().to_str().unwrap().to_string();
    let idx = oldest.with_file_name(format!("{}.idx", name.trim_end_matches(".gcr.zst")));
    let oldest_end = goalcup_format::read::read_index(&idx)
        .unwrap()
        .last()
        .unwrap()
        .seq;
    let kept: Value = serde_json::from_slice(&line).unwrap();
    assert!(
        kept["seq"].as_u64().unwrap() >= oldest_end,
        "a prune syncs first"
    );
    // Every line's LINK is written (the copy was taken at rest, after the last sync).
    let parts = goalcup_format::read::part_files(&rec_dir).unwrap();
    let last = parts.last().unwrap().to_str().unwrap().to_string();
    let last_idx = PathBuf::from(format!("{}.idx", last.trim_end_matches(".gcr.zst")));
    let last_seq = goalcup_format::read::read_index(&last_idx)
        .unwrap()
        .last()
        .unwrap()
        .seq;
    for l in std::fs::read(dead.join("counts.jsonl"))
        .unwrap()
        .split(|&b| b == b'\n')
    {
        if let Ok(v) = serde_json::from_slice::<Value>(l) {
            assert!(
                v["seq"].as_u64().unwrap() <= last_seq,
                "a line with no LINK"
            );
        }
    }
    std::fs::remove_file(&oldest).unwrap();
    std::fs::remove_file(&idx).unwrap();
    std::fs::write(dead.join("counts.jsonl"), [line.as_slice(), b"\n"].concat()).unwrap();

    let Ok(Closed::Sealed(facts)) = close_dead_entry(&dead) else {
        panic!("the dead entry is sealed");
    };
    let (_, seal) = down_and_seal(&dead);
    assert_eq!(seal["totals"]["wire_bytes"], json!(total));
    assert_eq!(seal["totals"]["http_req"], json!(1));
    // A POST and WebSocket messages ended after the kept line, their OPENs pruned.
    assert_eq!(seal["totals"]["http_post"], json!(1));
    assert_eq!(seal["totals"]["ws_up_msgs"], json!(11));
    assert_eq!(seal["totals"]["req_whole"], json!(12));
    assert_eq!(seal["totals"]["mask_failclosed_bytes"], json!(fail_closed));
    assert_eq!(facts.totals.counts.wire_bytes, total);
    let totals = entry_totals(&dead);
    assert_eq!(facts.totals, totals);
    let st = Status::read(&dead).expect("status.json");
    assert_eq!(st.written.counts.wire_bytes, total);
    assert_eq!(st.written.counts.mask_failclosed_bytes, fail_closed);
}
