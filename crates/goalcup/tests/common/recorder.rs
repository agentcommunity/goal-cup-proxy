//! Helpers over the calls of `Recorder::for_tests(room)` (seam S1), the recorder that collects
//! every call before masking, with `room()` set by the test. Tests call that seam directly.
//!
//! `real(dir)` is the pipeline with masking (the built-in shapes) and the writer in a temp entry
//! folder (PRD-02's `Recorder::real_for_tests`), `wait_settled` waits for a quiet durable point,
//! and `records` reads back what is durable.

use super::{HANG, within};
use goalcup::mask::Masker;
use goalcup::pipeline::{Call, Calls, Counts, EndReason, Recorder, Sid, StreamKind};
use goalcup_format::Record;
use serde_json::Value;
use std::io::Read;
use std::path::Path;

/// The real pipeline over `dir` with the built-in masking shapes and no connector rules. A test
/// that needs another masker (for example a passthrough one) calls `Recorder::real_for_tests`.
pub fn real(dir: &Path) -> Recorder {
    let masker = Masker::new(&[], Vec::new()).expect("built-in shapes");
    Recorder::real_for_tests(dir, masker)
}

/// Waits (bounded by `HANG`, never a speed check) for a durable point whose counts satisfy `done`
/// with nothing held, and returns that point's index line number.
pub fn wait_settled(rec: &Recorder, done: impl Fn(&Counts) -> bool + Send + 'static) -> u64 {
    let rec = rec.clone();
    within(HANG, move || {
        loop {
            let d = rec.durable().borrow().clone();
            if let Some(p) = &d.point
                && done(&d.totals.counts)
                && rec.live().held_bytes == 0
            {
                return p.line.seq;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    })
}

/// Every record of the entry in `dir` up to each part's last index line (a part with no index
/// line yet is skipped). Durable records only.
pub fn records(dir: &Path) -> Vec<Record> {
    let mut out = Vec::new();
    for part in goalcup_format::read::part_files(&dir.join("rec")).expect("part files") {
        let name = part.file_name().unwrap().to_str().unwrap();
        let idx = part.with_file_name(format!("{}.idx", name.trim_end_matches(".gcr.zst")));
        let Some(last) = goalcup_format::read::read_index(&idx).expect("index").pop() else {
            continue;
        };
        let f = std::fs::File::open(&part).expect("part").take(last.c_end);
        let recs = goalcup_format::read::decode(f).expect("decode");
        out.extend(recs.map(|i| i.expect("record").rec));
    }
    out
}

/// What `f` picks from each call, in call order.
pub fn pick<T>(c: &Calls, f: impl FnMut(Call) -> Option<T>) -> Vec<T> {
    c.all().into_iter().filter_map(f).collect()
}

/// The bytes of every DATA call for `sid`, joined in call order.
pub fn data_of(c: &Calls, sid: Sid) -> Vec<u8> {
    let pieces = pick(c, |call| match call {
        Call::Data(s, .., bytes) if s == sid => Some(bytes.to_vec()),
        _ => None,
    });
    pieces.concat()
}

/// Every GAP call, in order.
pub fn gaps(c: &Calls) -> Vec<Call> {
    pick(c, |call| matches!(call, Call::Gap(..)).then_some(call))
}

/// Each OPEN as (sid, kind, meta), in order.
pub fn opens(c: &Calls) -> Vec<(Sid, StreamKind, Value)> {
    pick(c, |call| match call {
        Call::Open(sid, kind, .., meta) => Some((sid, kind, meta)),
        _ => None,
    })
}

/// Each END as (sid, reason, seen, tail), in order.
pub fn ends(c: &Calls) -> Vec<(Sid, EndReason, u64, Option<Value>)> {
    pick(c, |call| match call {
        Call::End(sid, _, r, seen, tail) => Some((sid, r, seen, tail)),
        _ => None,
    })
}

/// Each EVENT, in order.
pub fn events(c: &Calls) -> Vec<Value> {
    pick(c, |call| match call {
        Call::Event(v) => Some(v),
        _ => None,
    })
}
