//! Helpers over `Recorder::for_tests(room)` (seam S1): a recorder that collects every call
//! before masking, with `room()` set by the test.
//!
//! `real(dir)`, the pipeline with masking and the writer in a temp entry folder, comes once
//! PRD-02's `Recorder::real_for_tests(dir, masker)` exists (PRD-14 slice 2).

use goalcup::pipeline::{Call, Calls, Recorder, Sid};

/// A recorder whose `room()` answers against `room`, and its collected calls.
pub fn start(room: u64) -> (Recorder, Calls) {
    Recorder::for_tests(room)
}

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
