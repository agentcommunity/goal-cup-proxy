//! Helpers over the calls of `Recorder::for_tests(room)` (seam S1), the recorder that collects
//! every call before masking, with `room()` set by the test. Tests call that seam directly.
//!
//! `real(dir)`, the pipeline with masking and the writer in a temp entry folder, comes once
//! PRD-02's `Recorder::real_for_tests(dir, masker)` lands.

use goalcup::pipeline::{Call, Calls, EndReason, Sid, StreamKind};
use serde_json::Value;

/// The bytes of every DATA call for `sid`, joined in call order.
pub fn data_of(calls: &Calls, sid: Sid) -> Vec<u8> {
    let mut out = Vec::new();
    for call in calls.all() {
        if let Call::Data(s, _, _, bytes) = call
            && s == sid
        {
            out.extend_from_slice(&bytes);
        }
    }
    out
}

/// Every GAP call, in order.
pub fn gaps(calls: &Calls) -> Vec<Call> {
    calls
        .all()
        .into_iter()
        .filter(|c| matches!(c, Call::Gap(..)))
        .collect()
}

/// Each OPEN as (sid, kind, meta), in order.
pub fn opens(c: &Calls) -> Vec<(Sid, StreamKind, Value)> {
    let f = |c: Call| match c {
        Call::Open(sid, kind, .., meta) => Some((sid, kind, meta)),
        _ => None,
    };
    c.all().into_iter().filter_map(f).collect()
}

/// Each END as (sid, reason, seen), in order.
pub fn ends(c: &Calls) -> Vec<(Sid, EndReason, u64)> {
    let f = |c: Call| match c {
        Call::End(sid, _, r, seen, _) => Some((sid, r, seen)),
        _ => None,
    };
    c.all().into_iter().filter_map(f).collect()
}

/// Each EVENT, in order.
pub fn events(c: &Calls) -> Vec<Value> {
    let f = |c: Call| match c {
        Call::Event(v) => Some(v),
        _ => None,
    };
    c.all().into_iter().filter_map(f).collect()
}
