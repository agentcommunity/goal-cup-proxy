//! PRD-02 section 11 (`pipeline_seal`), points 1, 3, 5 and 6 (PRD-17 L-P8, L-P15, L-FW6): the
//! seal over the real pipeline. What was open ends `cut_at_seal` with every byte in DATA or a
//! GAP, a call after the stop is counted in the `after_stop` GAP, native calls are admitted until
//! the native finish, nothing follows the seal's LINK, and the sealed totals equal a recount.
//! Each test checks the last two through `seal` below. Fake keys are built from pieces.

mod common;

use bytes::Bytes;
use common::recorder::{real, records};
use goalcup::pipeline::{
    Counts, EndReason, GapReason, OpenFlags, Recorder, StreamKind, entry_totals,
};
use goalcup::watchdog::SealReason;
use goalcup_format::{IndexKind, Record};
use serde_json::{Value, json};
use std::io::Read;
use std::path::Path;

const REQ: OpenFlags = OpenFlags::POST;

/// Seals, then checks points 5 and 6 on every entry: the part file ends with the seal event and
/// its LINK, read whole (nothing follows it), the index ends with `E`, and the sealed totals
/// equal a recount of the part files. Returns the records.
fn seal(rec: &Recorder, dir: &Path) -> Vec<Record> {
    let sealed: Counts = rec.seal_for_tests(dir, SealReason::SealCommand);
    let parts = goalcup_format::read::part_files(&dir.join("rec")).unwrap();
    let last = parts.last().expect("a part file");
    let whole: Vec<Record> = goalcup_format::read::decode(std::fs::File::open(last).unwrap())
        .unwrap()
        .map(|i| i.unwrap().rec)
        .collect();
    assert!(matches!(whole.last(), Some(Record::Link { .. })), "the seal's LINK is last");
    let before = &whole[whole.len() - 2];
    assert!(matches!(before, Record::Event { json, .. } if event(json)["ev"] == "seal"));
    let name = last.file_name().unwrap().to_str().unwrap();
    let idx = last.with_file_name(format!("{}.idx", name.trim_end_matches(".gcr.zst")));
    let lines = goalcup_format::read::read_index(&idx).unwrap();
    assert_eq!(lines.last().map(|l| l.kind), Some(IndexKind::E));
    assert_eq!(sealed, entry_totals(dir).counts, "the sealed totals equal a recount");
    records(dir)
}

fn event(json: &[u8]) -> Value {
    serde_json::from_slice(json).unwrap_or(Value::Null)
}

/// Each stream's DATA bytes and GAP losses, and its END as (reason, seen).
fn stream(recs: &[Record], want: u64) -> (Vec<u8>, u64, Option<(EndReason, u64)>) {
    let (mut data, mut lost, mut end) = (Vec::new(), 0, None);
    for r in recs {
        match r {
            Record::Data { sid, bytes, .. } if *sid == want => data.extend_from_slice(bytes),
            Record::Gap { sid, lost: l, .. } if *sid == want => lost += l.saturating_sub(1),
            Record::End {
                sid, reason, seen, ..
            } if *sid == want => end = Some((*reason, *seen)),
            _ => {}
        }
    }
    (data, lost, end)
}

/// The `after_stop` GAP on `sid 0`, as (start, end, lost, tail).
fn after_stop(recs: &[Record]) -> Option<(u64, u64, u64, Value)> {
    recs.iter().find_map(|r| match r {
        Record::Gap {
            sid: 0,
            reason: GapReason::AfterStop,
            start_mono,
            end_mono,
            lost,
            tail,
            ..
        } => Some((*start_mono, *end_mono, *lost, event(tail))),
        _ => None,
    })
}

/// Points 1 and 5 (L-P8): streams open at the seal end `cut_at_seal`, and every byte offered
/// before it is in DATA or in a GAP, bytes the masker held back for a match included.
#[test]
fn open_streams_end_cut_at_seal_with_every_byte_written() {
    let dir = tempfile::tempdir().unwrap();
    let rec = real(dir.path());
    let conn = rec.open(StreamKind::Conn, 0, 0, 0, OpenFlags::NONE, json!({"host": "a.test"}));
    let mut req = rec.open(StreamKind::HttpReq, conn.sid(), 0, 0, REQ, json!({}));
    let mut offered = b"POST / HTTP/1.1\r\n\r\n".to_vec();
    // The masker holds this tail back for a match that may go on.
    offered.extend_from_slice(b"data: {\"delta\": \"partial\"}");
    req.data(rec.now(), Bytes::from(offered.clone()));
    let recs = seal(&rec, dir.path());
    let (data, lost, end) = stream(&recs, req.sid());
    assert_eq!(data.len() as u64 + lost, offered.len() as u64);
    assert_eq!(end, Some((EndReason::CutAtSeal, offered.len() as u64)));
    let (_, _, end) = stream(&recs, conn.sid());
    assert_eq!(end.map(|e| e.0), Some(EndReason::CutAtSeal));
}

/// Points 1 and 3 (L-P15): a wire call after the stop reaches no record, and its bytes are in
/// the `after_stop` GAP's `lost`, with start before end.
#[test]
fn a_wire_call_after_the_stop_is_in_the_after_stop_gap() {
    let dir = tempfile::tempdir().unwrap();
    let rec = real(dir.path());
    let mut req = rec.open(StreamKind::HttpReq, 0, 0, 0, REQ, json!({}));
    req.data(rec.now(), Bytes::from_static(b"before the stop"));
    rec.stop_wire_for_tests();
    let late = b"zq-after-the-stop-zq".repeat(50);
    req.data(rec.now(), Bytes::from(late.clone()));
    let mut other = rec.open(StreamKind::HttpReq, 0, 0, 0, REQ, json!({}));
    other.data(rec.now(), Bytes::from_static(b"zq-after-the-stop-zq"));
    let recs = seal(&rec, dir.path());
    let (start, end, lost, _) = after_stop(&recs).expect("an after_stop GAP");
    assert!(start < end);
    assert_eq!(lost - 1, late.len() as u64 + 20);
    for r in &recs {
        if let Record::Data { bytes, .. } = r {
            assert!(!bytes.windows(8).any(|w| w == b"zq-after"));
        }
    }
    assert!(!recs.iter().any(|r| matches!(r, Record::Open { sid, .. } if *sid == other.sid())));
}

/// Points 1 and 3 (R3-3): a native stream offered between the wire stop and the native finish is
/// in the record before the seal event.
#[test]
fn a_native_stream_after_the_wire_stop_is_recorded_before_the_seal() {
    let dir = tempfile::tempdir().unwrap();
    let rec = real(dir.path());
    rec.stop_wire_for_tests();
    let meta = json!({"path": "t:a.jsonl", "gen": 0});
    let mut h = rec.open(StreamKind::Native, 0, 0, 0, OpenFlags::NONE, meta);
    let line = b"{\"turn\":\"after the wire stop\"}\n";
    h.data(rec.now(), Bytes::from_static(line));
    let sid = h.sid();
    h.end(rec.now(), EndReason::Complete, None);
    let recs = seal(&rec, dir.path());
    let whole = Some((EndReason::Complete, line.len() as u64));
    assert_eq!(stream(&recs, sid), (line.to_vec(), 0, whole));
    let at = |f: &dyn Fn(&Record) -> bool| recs.iter().position(f).unwrap();
    let data = at(&|r| matches!(r, Record::Data { sid: s, .. } if *s == sid));
    let seal_ev = at(&|r| matches!(r, Record::Event { json, .. } if event(json)["ev"] == "seal"));
    assert!(data < seal_ev);
}

/// Point 3 (L-FW6): a guard gap still open at the seal is closed by it, with start before end and
/// its bytes in `lost`, before the stream's `cut_at_seal` END.
#[test]
fn a_guard_gap_open_at_the_seal_is_closed() {
    // SAFETY: the test's first statement, before any thread is started.
    unsafe { std::env::set_var("GOALCUP_TEST_GUARD_MB", "64") };
    let dir = tempfile::tempdir().unwrap();
    let rec = real(dir.path());
    let mut req = rec.open(StreamKind::HttpReq, 0, 0, 0, REQ, json!({}));
    let full = rec.reserve(64 << 20);
    req.data(rec.now(), Bytes::from(vec![b'x'; 4096]));
    drop(full);
    let recs = seal(&rec, dir.path());
    let gap = recs.iter().position(|r| {
        matches!(r, Record::Gap { sid, reason: GapReason::MemoryGuard, start_mono, end_mono, lost: 4097, .. }
            if *sid == req.sid() && start_mono < end_mono)
    });
    let (_, _, end) = stream(&recs, req.sid());
    assert_eq!(end, Some((EndReason::CutAtSeal, 4096)));
    let end_at = recs.iter().position(|r| matches!(r, Record::End { sid, .. } if *sid == req.sid()));
    assert!(gap.expect("the guard GAP") < end_at.unwrap());
}

/// Points 4 and 6 (ledger R3b, PRD-05): the run CA key is gone after the seal, and a host shaped
/// like a key that opened after the stop reaches no part file, index or `status.json` unmasked.
#[test]
fn the_key_file_goes_and_an_after_stop_host_is_masked() {
    let dir = tempfile::tempdir().unwrap();
    let rec = real(dir.path());
    let key_file = dir.path().join("rec/run-ca.key");
    std::fs::write(&key_file, b"fake key for the test").unwrap();
    rec.stop_wire_for_tests();
    let key = ["gh", "p_", &"a1B2".repeat(9)].concat();
    let mut c = rec.open(StreamKind::Conn, 0, 0, 0, OpenFlags::NONE, json!({"host": key}));
    c.data(rec.now(), Bytes::from_static(b"relayed after the stop"));
    let recs = seal(&rec, dir.path());
    assert!(!key_file.exists());
    let (_, _, _, tail) = after_stop(&recs).expect("an after_stop GAP");
    assert_eq!(tail["hosts"].as_array().map(Vec::len), Some(1));
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let mut all = Vec::new();
    for f in std::fs::read_dir(dir.path().join("rec")).unwrap() {
        let p = f.unwrap().path();
        let Ok(bytes) = std::fs::read(&p) else { continue };
        all.extend(bytes);
        if p.to_string_lossy().ends_with(".gcr.zst") {
            let file = std::fs::File::open(&p).unwrap();
            zstd::stream::read::Decoder::new(file).unwrap().read_to_end(&mut all).unwrap();
        }
    }
    all.extend(std::fs::read(dir.path().join("status.json")).unwrap());
    assert!(!all.windows(key.len()).any(|w| w == key.as_bytes()));
}
