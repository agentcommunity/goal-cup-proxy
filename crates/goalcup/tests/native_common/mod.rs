//! Test helpers PRD-04's `native_*` tests share on top of `common::recorder` (PRD-14's). A folder
//! module does not build as its own test binary; each test includes it as `mod native_common;` after `mod common;`.

#![allow(dead_code)]

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use super::common::recorder::{ends, events, pick};
use goalcup::pipeline::{Call, Calls, EndReason, GapReason, Sid};
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
