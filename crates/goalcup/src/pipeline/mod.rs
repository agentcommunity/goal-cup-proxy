//! The recorder pipeline (PRD-02): one process carries every recorded byte from the proxy and the
//! session-file reader through one memory guard and masking to the part files.
//!
//! Scaffold: seam S1 (`Recorder`, `StreamHandle`) and S4 (`Counts`, `Durable`, `entry_totals`,
//! `status.json`) in their final shapes. The calls do nothing yet, except under `test-hooks`,
//! where `Recorder::for_tests` collects them.

pub mod clock;
pub mod counts;
pub mod cursors;
pub mod fds;
mod handle;
pub mod status;

pub use clock::Mono;
pub use counts::{Counts, Durable, TunnelHost, entry_totals};
pub use cursors::Cursors;
pub use goalcup_format::{EndReason, GapReason, StreamKind};
pub use handle::{OpenFlags, StreamHandle};
pub use status::Status;

use bytes::Bytes;
use serde_json::Value;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

/// A stream ID: from 1, never reused inside a boot; 0 means "no stream".
pub type Sid = u64;

/// The soft guard's default: 512 MiB (or one eighth of RAM if smaller, floor 128 MiB).
pub const SOFT_GUARD: u64 = 512 << 20;

/// The recorder handle (seam S1). Cheap to clone.
#[derive(Clone)]
pub struct Recorder {
    inner: Arc<Inner>,
}

struct Inner {
    next_sid: AtomicU64,
    /// Bytes offered and not yet written, plus reservations.
    held: AtomicU64,
    durable: watch::Sender<Durable>,
    /// Set only by `for_tests`.
    calls: Option<Calls>,
}

impl Recorder {
    // The scaffold has no production constructor yet; `run_recorder` builds one in the core slice.
    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    fn new(calls: Option<Calls>) -> Recorder {
        let (durable, _) = watch::channel(Durable::default());
        Recorder {
            inner: Arc::new(Inner {
                next_sid: AtomicU64::new(1),
                held: AtomicU64::new(0),
                durable,
                calls,
            }),
        }
    }

    /// Opens a stream. `from` is its first offset: 0 for wire streams, the resumed cursor for
    /// native. `meta` is masked on the mask worker, never on the caller.
    pub fn open(
        &self,
        kind: StreamKind,
        conn: Sid,
        req: Sid,
        from: u64,
        flags: OpenFlags,
        meta: Value,
    ) -> StreamHandle {
        let sid = self.inner.next_sid.fetch_add(1, Ordering::Relaxed);
        self.record(|| Call::Open {
            sid,
            kind,
            conn,
            req,
            from,
            flags,
            meta,
        });
        StreamHandle::new(self.clone(), sid, from)
    }

    /// A note or fact from another module; masked like OPEN meta.
    pub fn event(&self, ev: Value) {
        self.record(|| Call::Event(ev));
    }

    /// Bytes free under three quarters of the soft guard. Native reads at most this.
    pub fn room(&self) -> u64 {
        let held = self.inner.held.load(Ordering::Relaxed);
        let limit = match &self.inner.calls {
            Some(calls) => calls.room.load(Ordering::Relaxed),
            None => SOFT_GUARD / 4 * 3,
        };
        limit.saturating_sub(held)
    }

    /// Counts `n` in `held` until the returned `Reserved` drops (a decoder window, PRD-03).
    pub fn reserve(&self, n: u64) -> Reserved {
        self.inner.held.fetch_add(n, Ordering::Relaxed);
        Reserved {
            rec: self.clone(),
            n,
        }
    }

    /// Durable file positions from earlier boots (PRD-02 section 9.3).
    pub fn native_cursors(&self) -> Cursors {
        Cursors::default()
    }

    /// Boot time; counts a suspend.
    pub fn now(&self) -> Mono {
        Mono::now()
    }

    /// Live written counts of this entry (seam S4).
    pub fn counts(&self) -> Counts {
        Counts::default()
    }

    /// The durable point (seam S4). Plain threads read it with `borrow()` and copy it out at once.
    pub fn durable(&self) -> watch::Receiver<Durable> {
        self.inner.durable.subscribe()
    }

    /// A recorder that collects every S1 call before masking, with `room()` set by the test
    /// (PRD-14 `tests/common/recorder.rs`).
    #[cfg(feature = "test-hooks")]
    pub fn for_tests(room: u64) -> (Recorder, Calls) {
        let calls = Calls::default();
        calls.set_room(room);
        (Recorder::new(Some(calls.clone())), calls)
    }

    fn record(&self, call: impl FnOnce() -> Call) {
        if let Some(calls) = &self.inner.calls {
            calls.push(call());
        }
    }
}

/// `n` bytes counted in `held` until dropped.
#[must_use = "the reservation ends when this is dropped"]
pub struct Reserved {
    rec: Recorder,
    n: u64,
}

impl Drop for Reserved {
    fn drop(&mut self) {
        self.rec.inner.held.fetch_sub(self.n, Ordering::Relaxed);
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

/// For the native reader and uploader threads: nice 10 on Linux, a no-op elsewhere in 1.0.
pub fn low_priority() {}

/// The hidden recorder subcommand (PRD-10's `main.rs` dispatches to it): fd 3 is the listen
/// socket, fd 4 the control socket, `null` the `--null` argument. The scaffold records nothing.
pub fn run_recorder(null: bool) -> ExitCode {
    let _ = null;
    ExitCode::SUCCESS
}
