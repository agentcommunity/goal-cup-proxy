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

/// One S1 call as `Recorder::for_tests` saw it, before masking.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    Open {
        sid: Sid,
        kind: StreamKind,
        conn: Sid,
        req: Sid,
        from: u64,
        flags: OpenFlags,
        meta: Value,
    },
    Data {
        sid: Sid,
        t: Mono,
        off: u64,
        bytes: Bytes,
    },
    Ws {
        sid: Sid,
        t: Mono,
        off: u64,
        opcode: u8,
        fin: bool,
        bytes: Bytes,
    },
    Gap {
        sid: Sid,
        reason: GapReason,
        start: Mono,
        end: Mono,
        off: u64,
        lost: Option<u64>,
        tail: Option<Value>,
    },
    End {
        sid: Sid,
        t: Mono,
        reason: EndReason,
        seen: u64,
        tail: Option<Value>,
    },
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
