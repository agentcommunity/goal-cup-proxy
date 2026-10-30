//! The recorder pipeline (PRD-02): one process carries every recorded byte from the proxy and the
//! session-file reader through one memory guard and masking to the part files.
//!
//! Scaffold: seam S1 (`Recorder`, `StreamHandle`) and S4 (`Counts`, `EntryTotals`, `Durable`, `entry_totals`,
//! `status.json`) in their final shapes. The calls do nothing yet, except under `test-hooks`,
//! where `Recorder::for_tests` collects them.

/// Hands one S1 call, before masking, to the `for_tests` collector. Without `test-hooks` it
/// compiles to nothing, so the shipped recorder carries no test path.
macro_rules! collect {
    ($rec:expr, $call:expr) => {
        #[cfg(feature = "test-hooks")]
        $rec.collect(|| $call);
    };
}

#[cfg(feature = "test-hooks")]
mod calls;
pub mod clock;
pub mod counts;
pub mod cursors;
pub mod fds;
mod handle;
pub mod status;

#[cfg(feature = "test-hooks")]
pub use calls::{Call, Calls};
pub use clock::Mono;
pub use counts::{Counts, Durable, EntryTotals, TunnelHost, entry_totals};
pub use cursors::Cursors;
pub use goalcup_format::{EndReason, GapReason, StreamKind};
pub use handle::{OpenFlags, StreamHandle};
pub use status::{Live, Status};

use serde_json::Value;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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
    #[cfg(feature = "test-hooks")]
    calls: Option<Calls>,
}

impl Recorder {
    // The scaffold has no production constructor yet; `run_recorder` builds one in the core slice.
    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    fn new(#[cfg(feature = "test-hooks")] calls: Option<Calls>) -> Recorder {
        let (durable, _) = watch::channel(Durable::default());
        Recorder {
            inner: Arc::new(Inner {
                next_sid: AtomicU64::new(1),
                held: AtomicU64::new(0),
                durable,
                #[cfg(feature = "test-hooks")]
                calls,
            }),
        }
    }

    /// Opens a stream. `from` is its first offset: 0 for wire streams, the resumed cursor for
    /// native. `meta` is masked on the mask worker, never on the caller.
    // The scaffold's calls only reach the `test-hooks` collector until the core slice.
    #[cfg_attr(not(feature = "test-hooks"), allow(unused_variables))]
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
        collect!(self, Call::Open(sid, kind, conn, req, from, flags, meta));
        StreamHandle::new(self.clone(), sid, from)
    }

    /// A note or fact from another module; masked like OPEN meta.
    #[cfg_attr(not(feature = "test-hooks"), allow(unused_variables))]
    pub fn event(&self, ev: Value) {
        collect!(self, Call::Event(ev));
    }

    /// Bytes free under three quarters of the soft guard. Native reads at most this.
    pub fn room(&self) -> u64 {
        let limit = SOFT_GUARD / 4 * 3;
        #[cfg(feature = "test-hooks")]
        let limit = self.test_room().unwrap_or(limit);
        limit.saturating_sub(self.inner.held.load(Ordering::Relaxed))
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

    /// The recorder's live state (seam S4), read from its atomics: what the uploader's heartbeat
    /// `mode` and `boot`, `off_spans`, `off_ms` and `off_since` take from a live recorder, so it
    /// never parses `status.json` for them.
    pub fn live(&self) -> Live {
        Live {
            held_bytes: self.inner.held.load(Ordering::Relaxed),
            ..Live::default()
        }
    }

    /// The durable point (seam S4). Plain threads read it with `borrow()` and copy it out at once.
    pub fn durable(&self) -> watch::Receiver<Durable> {
        self.inner.durable.subscribe()
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

/// For the native reader and uploader threads: nice 10 on Linux, a no-op elsewhere in 1.0.
pub fn low_priority() {}

/// The hidden recorder subcommand (PRD-10's `main.rs` dispatches to it): fd 3 is the listen
/// socket, fd 4 the control socket, `null` the `--null` argument. The scaffold records nothing.
pub fn run_recorder(null: bool) -> ExitCode {
    let _ = null;
    ExitCode::SUCCESS
}
