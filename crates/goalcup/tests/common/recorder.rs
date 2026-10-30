//! Helpers over the calls of `Recorder::for_tests(room)` (seam S1), the recorder that collects
//! every call before masking, with `room()` set by the test. Tests call that seam directly.
//!
//! `real(dir)`, the pipeline with masking and the writer in a temp entry folder, comes once
//! PRD-02's `Recorder::real_for_tests(dir, masker)` lands.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use goalcup::pipeline::{Call, Calls, EndReason, GapReason, Sid, StreamKind};
use serde_json::Value;

/// Appends `b` to the file at `p`, making it if it is not there.
pub fn append(p: &Path, b: &[u8]) {
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
        .unwrap();
    f.write_all(b).unwrap();
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

/// The EVENTs whose `what` is `what`, in order.
pub fn notes(c: &Calls, what: &str) -> Vec<Value> {
    events(c)
        .into_iter()
        .filter(|v| v["what"] == what)
        .collect()
}

/// How stream `sid` ended.
pub fn end_of(c: &Calls, sid: Sid) -> Option<EndReason> {
    let mut e = ends(c).into_iter();
    e.find_map(|(s, r, ..)| (s == sid).then_some(r))
}

/// Each GAP as (sid, reason, lost, tail), in order.
pub fn gap_rows(c: &Calls) -> Vec<(Sid, GapReason, Option<u64>, Option<Value>)> {
    pick(c, |call| match call {
        Call::Gap(sid, r, .., lost, t) => Some((sid, r, lost, t)),
        _ => None,
    })
}
