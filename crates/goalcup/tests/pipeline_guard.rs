//! PRD-02 P1, P2, P4 and P10 (`pipeline_guard`) and P3 (`pipeline_no_latch`), points 1 and 3:
//! with the writer held still, every S1 call returns, the guard drops only the offered pieces of
//! the streams that offer, and the record has one `memory_guard` GAP per drop episode per
//! stream, with DATA and GAP covering each stream's offsets exactly and COUNT equal to a recount.
//! Each test sets its env first, before any thread (nextest runs each test in its own process).

mod common;

use bytes::Bytes;
use common::recorder::{records, wait_settled};
use common::{HANG, within};
use goalcup::mask::Masker;
use goalcup::pipeline::{EndReason, GapReason, OpenFlags, Recorder, StreamHandle, StreamKind};
use goalcup::watchdog::SealReason;
use goalcup_format::Record;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

const BODY: usize = 1_500_000;
const POST: OpenFlags = OpenFlags::POST;
const MIB: u64 = 1 << 20;

/// One shared body: byte `i` is `i % 251`, so a byte's value follows from its stream offset.
fn body() -> Bytes {
    (0..BODY)
        .map(|i| (i % 251) as u8)
        .collect::<Vec<u8>>()
        .into()
}

fn set_env(stall: Option<&str>) {
    // SAFETY: the test's first statement, before any thread is started.
    unsafe {
        std::env::set_var("GOALCUP_TEST_GUARD_MB", "64");
        if let Some(s) = stall {
            std::env::set_var("GOALCUP_TEST_WRITER_STALL", s);
        }
    }
}

fn open(rec: &Recorder) -> StreamHandle {
    let flags = POST | OpenFlags::DECRYPTED;
    rec.open(StreamKind::HttpReq, 0, 0, 0, flags, json!({}))
}

/// Waits for a durable point that counts `posts` requests ended, with nothing held.
fn settle(rec: &Recorder, posts: u64) -> u64 {
    wait_settled(rec, move |c| c.http_post >= posts)
}

/// Checks every `http_req` stream: DATA bytes match the shared body at their offsets, DATA and
/// `memory_guard` GAPs cover `[0, seen)` exactly with no two GAPs in a row (one per episode),
/// each GAP has start before end. Returns the GAP count per stream.
fn check(recs: &[Record], body: &[u8], seen: u64) -> Vec<u64> {
    let mut at: BTreeMap<u64, (u64, bool, u64)> = BTreeMap::new();
    for r in recs {
        match r {
            Record::Open {
                sid,
                skind: StreamKind::HttpReq,
                ..
            } => {
                at.insert(*sid, (0, false, 0));
            }
            Record::Data {
                sid, off, bytes, ..
            } => {
                let Some(s) = at.get_mut(sid) else { continue };
                assert_eq!(*off, s.0, "DATA where the stream stands");
                let want = (0..bytes.len()).map(|j| body[(*off as usize + j) % BODY]);
                assert!(bytes.iter().copied().eq(want), "DATA bytes");
                (s.0, s.1) = (off + bytes.len() as u64, false);
            }
            Record::Gap {
                sid,
                reason,
                start_mono,
                end_mono,
                start_wall,
                end_wall,
                off,
                lost,
                ..
            } => {
                let s = at.get_mut(sid).expect("a GAP on an open stream");
                assert_eq!(*reason, GapReason::MemoryGuard);
                assert!(start_mono < end_mono && start_wall <= end_wall);
                assert!(!s.1, "one GAP per drop episode");
                assert_eq!(*off, s.0, "the GAP starts at the hole");
                assert!(*lost > 1);
                (s.0, s.1, s.2) = (off + lost - 1, true, s.2 + 1);
            }
            Record::End { sid, seen: n, .. } => {
                let s = at.get(sid).expect("an END on an open stream");
                assert_eq!((s.0, *n), (seen, seen), "DATA and GAP fill the stream");
            }
            _ => {}
        }
    }
    at.values().map(|s| s.2).collect()
}

/// The counts.jsonl line at `seq` against a recount of the records before that LINK.
fn check_count(dir: &Path, recs: &[Record], seq: u64) {
    let text = std::fs::read_to_string(dir.join("counts.jsonl")).unwrap();
    let line: Value = text
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|l| l["seq"] == seq)
        .expect("a counts.jsonl line at the durable seq");
    let mut n = BTreeMap::from([
        ("gaps", 0),
        ("lost_bytes", 0),
        ("wire_bytes", 0),
        ("http_req", 0),
    ]);
    for r in recs {
        match r {
            Record::Link { seq: s, .. } if *s == seq => break,
            Record::Open {
                skind: StreamKind::HttpReq,
                ..
            } => *n.get_mut("http_req").unwrap() += 1,
            Record::Data { bytes, .. } => *n.get_mut("wire_bytes").unwrap() += bytes.len() as u64,
            Record::Gap { lost, .. } => {
                *n.get_mut("gaps").unwrap() += 1;
                *n.get_mut("lost_bytes").unwrap() += lost - 1;
            }
            _ => {}
        }
    }
    for (k, v) in n {
        assert_eq!(line["count"][k], v, "COUNT {k} equals a recount");
    }
}

/// `n` streams each offer `bodies` bodies while the writer is held, then one small piece after
/// it is free, then END.
fn held_writer(n: usize, bodies: usize) {
    set_env(Some("0:25000"));
    let dir = common::recorder::entry_dir();
    let rec = Recorder::real_for_tests(dir.path(), Masker::passthrough_for_tests());
    let body = body();
    let handles: Vec<StreamHandle> = (0..n).map(|_| open(&rec)).collect();
    let (r, b) = (rec.clone(), body.clone());
    let mut handles = within(HANG, move || {
        let mut handles = handles;
        std::thread::scope(|s| {
            for h in handles.iter_mut() {
                let (r, b) = (&r, &b);
                s.spawn(move || {
                    for _ in 0..bodies {
                        h.data(r.now(), b.clone());
                    }
                });
            }
        });
        handles
    });
    // Every call returned while the writer was held.
    assert_eq!(rec.live().losing.streams_in_gap, n as u64);
    settle(&rec, 0);
    for h in handles.iter_mut() {
        h.data(rec.now(), body.slice(..1024));
    }
    for h in handles {
        h.end(rec.now(), EndReason::Complete, None);
    }
    let seq = settle(&rec, n as u64);
    let recs = records(dir.path());
    let gaps = check(&recs, &body, (bodies * BODY + 1024) as u64);
    // During the hold `held` only grows, so a stream that dropped never admits again before the
    // hold ends (it needs `held` under 3/4 of the guard): one episode per stream.
    assert_eq!(gaps, vec![1; n]);
    check_count(dir.path(), &recs, seq);
}

/// 40 streams, 2 GiB each in 1.5 MB bodies (PRD-02 `pipeline_guard`).
#[test]
fn forty_streams_with_the_writer_held() {
    held_writer(40, (2 << 30) / BODY);
}

/// 200 streams, 2 GiB each in 1.5 MB bodies (PRD-02 `pipeline_guard`).
#[test]
fn two_hundred_streams_with_the_writer_held() {
    held_writer(200, (2 << 30) / BODY);
}

/// P3: 1,000 guard episodes in a row on one stream; recording resumes after every one.
#[test]
fn a_thousand_guard_episodes_never_latch() {
    set_env(None);
    let dir = common::recorder::entry_dir();
    let rec = Recorder::real_for_tests(dir.path(), Masker::passthrough_for_tests());
    let body = body();
    let mut h = open(&rec);
    const PIECE: usize = 750;
    let mut off = 0;
    for _ in 0..1000 {
        let full = rec.reserve(64 * MIB);
        h.data(rec.now(), body.slice(off % BODY..off % BODY + PIECE));
        off += PIECE;
        drop(full);
        h.data(rec.now(), body.slice(off % BODY..off % BODY + PIECE));
        off += PIECE;
    }
    h.end(rec.now(), EndReason::Complete, None);
    let seq = settle(&rec, 1);
    let recs = records(dir.path());
    assert_eq!(check(&recs, &body, off as u64), vec![1000]);
    check_count(dir.path(), &recs, seq);
}

/// Section 3: the guard weighs what a piece costs (its queue slot as well as its bytes), so
/// 1-byte pieces reach the guard, and its GAP, long before their bytes alone would.
#[test]
fn tiny_pieces_reach_the_guard_by_their_cost() {
    // SAFETY: the test's first statement, before any thread is started.
    unsafe {
        std::env::set_var("GOALCUP_TEST_GUARD_MB", "1");
        std::env::set_var("GOALCUP_TEST_WRITER_STALL", "0:5000");
    }
    let dir = common::recorder::entry_dir();
    let rec = Recorder::real_for_tests(dir.path(), Masker::passthrough_for_tests());
    let body = body();
    let mut h = open(&rec);
    let mut n = 0;
    while rec.live().losing.dropped_bytes == 0 {
        h.data(rec.now(), body.slice(n..n + 1));
        n += 1;
        assert!((n as u64) < MIB / 64, "the guard counts a piece's cost");
    }
    h.end(rec.now(), EndReason::Complete, None);
    let seq = settle(&rec, 1);
    let recs = records(dir.path());
    assert_eq!(check(&recs, &body, n as u64), vec![1]);
    check_count(dir.path(), &recs, seq);
}

/// Point 3 and no drop without a GAP: 16 relay threads dropping under a full guard while the
/// wire stop lands. Every byte is in a guard GAP or in the seal's `after_stop` GAP, and the seal
/// leaves no stream in a gap.
#[test]
fn a_drop_racing_the_stop_is_in_a_gap_or_after_stop() {
    set_env(None);
    static PIECE: [u8; 64] = [b'r'; 64];
    for _ in 0..10 {
        let dir = common::recorder::entry_dir();
        let rec = Recorder::real_for_tests(dir.path(), Masker::passthrough_for_tests());
        let full = rec.reserve(64 * MIB);
        let mut handles: Vec<StreamHandle> = (0..16).map(|_| open(&rec)).collect();
        std::thread::scope(|s| {
            for h in handles.iter_mut() {
                let rec = &rec;
                s.spawn(move || {
                    for _ in 0..2000 {
                        h.data(rec.now(), Bytes::from_static(&PIECE));
                    }
                });
            }
            std::thread::yield_now();
            rec.stop_wire_for_tests();
        });
        rec.seal_for_tests(dir.path(), SealReason::SealCommand);
        drop((handles, full));
        let lost: u64 = records(dir.path())
            .iter()
            .map(|r| match r {
                Record::Gap { lost, .. } => lost.saturating_sub(1),
                Record::Data { sid, .. } if *sid != 0 => panic!("no piece fits a full guard"),
                _ => 0,
            })
            .sum();
        assert_eq!(lost, 16 * 2000 * 64);
        assert_eq!(rec.live().losing.streams_in_gap, 0);
    }
}
