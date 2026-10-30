//! The test collector behind `Recorder::for_tests` (PRD-14 `tests/common/recorder.rs`). Built
//! only with `test-hooks`, never in a release build (owner question 12).

use super::{Mono, OpenFlags, Recorder, Sid};
use bytes::Bytes;
use goalcup_format::{EndReason, GapReason, StreamKind};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

impl Recorder {
    /// A recorder that collects every S1 call before masking, with `room()` set by the test.
    pub fn for_tests(room: u64) -> (Recorder, Calls) {
        let calls = Calls::default();
        calls.set_room(room);
        (Recorder::new(Some(calls.clone())), calls)
    }

    pub(super) fn collect(&self, call: impl FnOnce() -> Call) {
        if let Some(calls) = &self.inner.calls {
            calls.push(call());
        }
    }

    /// The limit `room()` answers against under `for_tests`.
    pub(super) fn test_room(&self) -> Option<u64> {
        let calls = self.inner.calls.as_ref()?;
        Some(calls.room.load(Ordering::Relaxed))
    }
}

/// One S1 call as `Recorder::for_tests` saw it, before masking. Fields in the order of the call's
/// arguments, after the stream ID; `Gap` adds its end time before `off`, and `End` the handle's
/// offset as `seen`.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    /// sid, kind, conn, req, from, flags, meta.
    Open(Sid, StreamKind, Sid, Sid, u64, OpenFlags, Value),
    /// sid, t, off, bytes.
    Data(Sid, Mono, u64, Bytes),
    /// sid, t, off, opcode, fin, bytes.
    Ws(Sid, Mono, u64, u8, bool, Bytes),
    /// sid, reason, start, end, off, lost, tail.
    Gap(Sid, GapReason, Mono, Mono, u64, Option<u64>, Option<Value>),
    /// sid, t, reason, seen, tail.
    End(Sid, Mono, EndReason, u64, Option<Value>),
    Event(Value),
}

/// The calls collected by `Recorder::for_tests`, and its `room()`.
#[derive(Clone, Default)]
pub struct Calls {
    log: Arc<Mutex<Vec<Call>>>,
    room: Arc<AtomicU64>,
}

impl Calls {
    /// Every call so far, in order, leaving none.
    pub fn take(&self) -> Vec<Call> {
        std::mem::take(&mut *self.log.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Every call so far, in order.
    pub fn all(&self) -> Vec<Call> {
        self.log.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Sets the limit `room()` answers against (reservations are taken off it).
    pub fn set_room(&self, n: u64) {
        self.room.store(n, Ordering::Relaxed);
    }

    fn push(&self, call: Call) {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(call);
    }
}
