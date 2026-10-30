//! The session-file reader (PRD-04): one low-priority thread that walks each native row's root,
//! reads the harness's session files raw and copies its databases, with no symlink followed.
//!
//! The leaf modules are here: no-follow file calls, row patterns, the full walk and the tail.
//! `start`, the loop, `NativeReader::finish` and `snapshot.rs` come in the next slice.

pub mod fsx;
pub mod pattern;
pub mod tail;
pub mod walk;

use std::collections::HashSet;

use serde_json::Value;

use crate::pipeline::Recorder;

/// The notes already written, by event text: each note is written once per boot.
#[derive(Default)]
pub(crate) struct Noted(HashSet<String>);

impl Noted {
    pub(crate) fn once(&mut self, rec: &Recorder, ev: Value) {
        if self.0.insert(ev.to_string()) {
            rec.event(ev);
        }
    }
}
