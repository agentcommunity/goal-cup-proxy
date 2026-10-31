//! PRD-17 FW1 and R2: a close or a loss is never what the memory guard drops, and one stream's
//! drop never takes another stream's OPEN or close. With the writer held, A fills the guard, then
//! B and C open (`pipeline_guard.rs` opens its streams before the guard fills).

mod common;

use bytes::Bytes;
use common::recorder::{records, wait_settled};
use goalcup::mask::Masker;
use goalcup::pipeline::{EndReason, GapReason, OpenFlags, Recorder, StreamHandle, StreamKind};
use goalcup_format::Record;
use serde_json::json;
use std::collections::BTreeMap;

const PIECE: usize = 1_500_000;

#[test]
fn open_and_end_survive_a_full_guard() {
    // SAFETY: first statement, before any thread starts.
    unsafe {
        std::env::set_var("GOALCUP_TEST_GUARD_MB", "64");
        std::env::set_var("GOALCUP_TEST_WRITER_STALL", "0:10000");
    }
    let dir = tempfile::tempdir().unwrap();
    let rec = Recorder::real_for_tests(dir.path(), Masker::passthrough_for_tests());
    let piece = Bytes::from(vec![7u8; PIECE]);
    let open = || -> StreamHandle {
        let flags = OpenFlags::POST | OpenFlags::DECRYPTED;
        rec.open(StreamKind::HttpReq, 0, 0, 0, flags, json!({}))
    };
    let mut a = open();
    let mut offered = BTreeMap::from([(a.sid(), 0u64)]);
    // The writer is held: offer to A until the guard has put it in a gap (bounded, no clock).
    for _ in 0..200 {
        if rec.live().losing.streams_in_gap >= 1 {
            break;
        }
        a.data(rec.now(), piece.clone());
        *offered.get_mut(&a.sid()).unwrap() += PIECE as u64;
    }
    assert!(rec.live().losing.streams_in_gap >= 1, "A reached a gap");
    let (mut b, mut c) = (open(), open());
    for h in [&mut b, &mut c] {
        h.data(rec.now(), piece.clone());
        offered.insert(h.sid(), PIECE as u64);
    }
    for h in [a, b, c] {
        h.end(rec.now(), EndReason::Complete, None);
    }
    wait_settled(&rec, |n| n.http_post >= 3);

    let (mut opens, mut ends) = (Vec::new(), BTreeMap::new());
    let mut at: BTreeMap<_, (u64, u64)> = BTreeMap::new(); // sid -> (covered offset, GAP count)
    for r in records(dir.path()) {
        match r {
            Record::Open { sid, .. } => opens.push(sid),
            Record::Data {
                sid, off, bytes, ..
            } => {
                let s = at.entry(sid).or_default();
                assert_eq!(off, s.0, "DATA where the stream stands");
                s.0 += bytes.len() as u64;
            }
            Record::Gap {
                sid,
                reason,
                off,
                lost,
                ..
            } => {
                let s = at.entry(sid).or_default();
                assert_eq!(
                    (reason, off),
                    (GapReason::MemoryGuard, s.0),
                    "one drop, on its own stream"
                );
                s.0 += lost - 1;
                s.1 += 1;
            }
            Record::End { sid, seen, .. } => {
                ends.insert(sid, seen);
            }
            _ => {}
        }
    }
    for (sid, want) in offered {
        assert!(opens.contains(&sid), "OPEN of every stream is durable");
        assert_eq!(
            ends.get(&sid),
            Some(&want),
            "END of every stream, with what was offered"
        );
        let (covered, gaps) = at[&sid];
        assert_eq!(
            covered, want,
            "DATA and GAP cover the stream up to END.seen"
        );
        assert!(
            gaps >= 1,
            "the full guard dropped a piece of this stream, as a GAP"
        );
    }
}
