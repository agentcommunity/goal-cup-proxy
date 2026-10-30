//! The recorder pipeline (PRD-02): one process carries every recorded byte from the proxy and the
//! session-file reader through one memory guard and masking to the part files.
//!
//! Seam S1 (`Recorder`, `StreamHandle`) and S4 (`Counts`, `EntryTotals`, `Durable`,
//! `entry_totals`, `status.json`). The front half is here: every S1 call goes through the one
//! held-bytes guard and the gap rule into an unbounded item channel. The mask worker and the
//! writer that drain it come next. Under `test-hooks`, `Recorder::for_tests` collects every call
//! before the guard and sends nothing.

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
mod guard;
mod handle;
mod item;
pub mod secure;
pub mod status;

#[cfg(feature = "test-hooks")]
pub use calls::{Call, Calls};
pub use clock::Mono;
pub use counts::{Counts, Durable, EntryTotals, TunnelHost, entry_totals};
pub use cursors::Cursors;
pub use goalcup_format::{EndReason, GapReason, StreamKind};
pub use handle::{OpenFlags, StreamHandle};
pub use status::{Live, Status};

use item::Item;
use serde_json::Value;
use status::{Losing, Off};
use std::io::Write;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use tokio::sync::watch;

/// A stream ID: from 1, never reused inside a boot; 0 means "no stream".
pub type Sid = u64;

/// The soft guard's default: 512 MiB (or one eighth of RAM if smaller, floor 128 MiB).
pub const SOFT_GUARD: u64 = 512 << 20;

/// The exit code when a pipeline thread is gone (PRD-02 section 2 rule 1).
const EXIT_PIPELINE_GONE: i32 = 70;

/// The recorder handle (seam S1). Cheap to clone.
#[derive(Clone)]
pub struct Recorder {
    inner: Arc<Inner>,
}

struct Inner {
    next_sid: AtomicU64,
    /// To the mask worker. None under `for_tests` and in the scaffold, which send nothing.
    tx: Option<mpsc::Sender<Item>>,
    /// The soft guard in bytes.
    guard: u64,
    /// Bytes offered and not yet written, plus reservations.
    held: AtomicU64,
    held_peak: AtomicU64,
    /// Bytes the guard dropped this boot.
    dropped_bytes: AtomicU64,
    streams_in_gap: AtomicU64,
    /// Boot time of the writer's last write.
    writer_last_ns: AtomicU64,
    boot: AtomicU64,
    /// Entry totals at the last LINK, set by the writer.
    counts: Mutex<Counts>,
    off: Mutex<Off>,
    cursors: Mutex<Cursors>,
    /// The seal stopped the wire streams (PRD-02 section 11 step 2).
    wire_stopped: AtomicBool,
    /// The seal stopped every stream and event.
    all_stopped: AtomicBool,
    /// Bytes offered after a stop, and the hosts of `conn` streams opened after it.
    after_stop_bytes: AtomicU64,
    after_stop_hosts: Mutex<Vec<String>>,
    durable: watch::Sender<Durable>,
    /// Set only by `for_tests`.
    #[cfg(feature = "test-hooks")]
    calls: Option<Calls>,
}

impl Inner {
    fn new(guard: u64, tx: Option<mpsc::Sender<Item>>) -> Inner {
        let (durable, _) = watch::channel(Durable::default());
        Inner {
            next_sid: AtomicU64::new(1),
            tx,
            guard,
            held: AtomicU64::new(0),
            held_peak: AtomicU64::new(0),
            dropped_bytes: AtomicU64::new(0),
            streams_in_gap: AtomicU64::new(0),
            writer_last_ns: AtomicU64::new(Mono::now().ns()),
            boot: AtomicU64::new(0),
            counts: Mutex::new(Counts::default()),
            off: Mutex::new(Off::default()),
            cursors: Mutex::new(Cursors::default()),
            wire_stopped: AtomicBool::new(false),
            all_stopped: AtomicBool::new(false),
            after_stop_bytes: AtomicU64::new(0),
            after_stop_hosts: Mutex::new(Vec::new()),
            durable,
            #[cfg(feature = "test-hooks")]
            calls: None,
        }
    }
}

/// A lock that a panic elsewhere never poisons for good: the data are plain values.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Recorder {
    // `for_tests` builds the collector with it; `pipeline` is the real front.
    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    fn new(#[cfg(feature = "test-hooks")] calls: Option<Calls>) -> Recorder {
        let inner = Inner {
            #[cfg(feature = "test-hooks")]
            calls,
            ..Inner::new(SOFT_GUARD, None)
        };
        Recorder {
            inner: Arc::new(inner),
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
        collect!(
            self,
            Call::Open(sid, kind, conn, req, from, flags, meta.clone())
        );
        let handle = StreamHandle::new(self.clone(), sid, kind, from);
        if handle.stopped() {
            if kind == StreamKind::Conn
                && let Some(host) = meta.get("host").and_then(Value::as_str)
            {
                let mut hosts = lock(&self.inner.after_stop_hosts);
                if !hosts.iter().any(|h| h == host) {
                    hosts.push(host.to_string());
                }
            }
            return handle;
        }
        self.send(Item::Open {
            sid,
            t: self.now(),
            kind,
            conn,
            req,
            flags,
            meta,
        });
        handle
    }

    /// A note or fact from another module; masked like OPEN meta. Always admitted; after the
    /// seal's full stop, a no-op.
    pub fn event(&self, ev: Value) {
        collect!(self, Call::Event(ev.clone()));
        if self.inner.all_stopped.load(Ordering::Acquire) {
            return;
        }
        self.send(Item::Event { t: self.now(), ev });
    }

    /// Bytes free under three quarters of the soft guard. Native reads at most this.
    pub fn room(&self) -> u64 {
        let limit = guard::low_mark(self.inner.guard);
        #[cfg(feature = "test-hooks")]
        let limit = self.test_room().unwrap_or(limit);
        limit.saturating_sub(self.inner.held.load(Ordering::Relaxed))
    }

    /// Counts `n` in `held` until the returned `Reserved` drops (a decoder window, PRD-03).
    pub fn reserve(&self, n: u64) -> Reserved {
        self.add_held(n);
        Reserved {
            rec: self.clone(),
            n,
        }
    }

    /// Durable file positions from earlier boots (PRD-02 section 9.3).
    pub fn native_cursors(&self) -> Cursors {
        lock(&self.inner.cursors).clone()
    }

    /// Boot time; counts a suspend.
    pub fn now(&self) -> Mono {
        Mono::now()
    }

    /// Entry totals at the last LINK (seam S4).
    pub fn counts(&self) -> Counts {
        lock(&self.inner.counts).clone()
    }

    /// The recorder's live state (seam S4), read from its atomics: what the uploader's heartbeat
    /// `mode` and `boot`, `off_spans`, `off_ms` and `off_since` take from a live recorder, so it
    /// never parses `status.json` for them.
    pub fn live(&self) -> Live {
        let i = &self.inner;
        let held = i.held.load(Ordering::Relaxed);
        let writer_idle_ms = if held > 0 {
            self.now()
                .ns()
                .saturating_sub(i.writer_last_ns.load(Ordering::Relaxed))
                / 1_000_000
        } else {
            0
        };
        Live {
            boot: i.boot.load(Ordering::Relaxed),
            held_bytes: held,
            writer_idle_ms,
            losing: Losing {
                streams_in_gap: i.streams_in_gap.load(Ordering::Relaxed),
                dropped_bytes: i.dropped_bytes.load(Ordering::Relaxed),
            },
            off: lock(&i.off).clone(),
        }
    }

    /// The durable point (seam S4). Plain threads read it with `borrow()` and copy it out at once.
    pub fn durable(&self) -> watch::Receiver<Durable> {
        self.inner.durable.subscribe()
    }

    fn add_held(&self, n: u64) {
        let now = self.inner.held.fetch_add(n, Ordering::Relaxed) + n;
        self.inner.held_peak.fetch_max(now, Ordering::Relaxed);
    }

    /// The one send: counts the item in `held`, then hands it on. No lock is held here. A gone
    /// receiver means a pipeline thread died: exit 70 so the watchdog restarts the recorder.
    fn send(&self, item: Item) {
        let Some(tx) = &self.inner.tx else {
            return;
        };
        self.add_held(item.held());
        if tx.send(item).is_err() {
            pipeline_gone();
        }
    }
}

/// The front's API for the mask worker, the writer, the status thread and the seal. Their
/// callers land in the next slice.
#[allow(dead_code)]
impl Recorder {
    /// A recorder whose items go to the returned receiver, under soft guard `guard`.
    pub(crate) fn pipeline(guard: u64) -> (Recorder, mpsc::Receiver<Item>) {
        let (tx, rx) = mpsc::channel();
        let rec = Recorder {
            inner: Arc::new(Inner::new(guard, Some(tx))),
        };
        (rec, rx)
    }

    /// The writer handed records worth `n` held bytes to the compressor.
    pub(crate) fn release(&self, n: u64) {
        let _ = self
            .inner
            .held
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |h| {
                Some(h.saturating_sub(n))
            });
        self.inner
            .writer_last_ns
            .store(self.now().ns(), Ordering::Relaxed);
    }

    /// The soft guard in bytes (`status.json` `guard`).
    pub(crate) fn guard(&self) -> u64 {
        self.inner.guard
    }

    /// The most bytes ever held at once this boot (`status.json` and COUNT `held_peak`).
    pub(crate) fn held_peak(&self) -> u64 {
        self.inner.held_peak.load(Ordering::Relaxed)
    }

    pub(crate) fn set_counts(&self, counts: Counts) {
        *lock(&self.inner.counts) = counts;
    }

    pub(crate) fn set_off(&self, off: Off) {
        *lock(&self.inner.off) = off;
    }

    pub(crate) fn set_boot(&self, boot: u64) {
        self.inner.boot.store(boot, Ordering::Relaxed);
    }

    pub(crate) fn set_cursors(&self, cursors: Cursors) {
        *lock(&self.inner.cursors) = cursors;
    }

    pub(crate) fn publish_durable(&self, durable: Durable) {
        self.inner.durable.send_replace(durable);
    }

    /// Every later call on a wire stream only counts (PRD-02 section 11 step 2).
    pub(crate) fn stop_wire(&self) {
        self.inner.wire_stopped.store(true, Ordering::Release);
    }

    /// Every later call on any stream only counts, and events are dropped.
    pub(crate) fn stop_all(&self) {
        self.inner.wire_stopped.store(true, Ordering::Release);
        self.inner.all_stopped.store(true, Ordering::Release);
    }

    /// Bytes offered after a stop, and the hosts of `conn` streams opened after it.
    pub(crate) fn after_stop(&self) -> (u64, Vec<String>) {
        (
            self.inner.after_stop_bytes.load(Ordering::Relaxed),
            lock(&self.inner.after_stop_hosts).clone(),
        )
    }
}

/// A pipeline thread is gone. The first caller writes one fixed line and exits 70; a caller that
/// comes later returns, since the process is already ending.
fn pipeline_gone() {
    static EXITING: AtomicBool = AtomicBool::new(false);
    if EXITING.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::io::stderr()
        .lock()
        .write_all(b"goalcup: recorder pipeline stopped; restarting\n");
    std::process::exit(EXIT_PIPELINE_GONE);
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

/// For the mask worker, the writer, the native reader and the uploader threads: nice 10 for the
/// calling thread on Linux (where the priority is per thread), a no-op on macOS in 1.0. Errors
/// are ignored.
pub fn low_priority() {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: gettid takes no argument and cannot fail.
        let tid = unsafe { libc::syscall(libc::SYS_gettid) };
        // SAFETY: setpriority only changes the scheduling of the given thread.
        let _ = unsafe { libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, 10) };
    }
}

/// The hidden recorder subcommand (PRD-10's `main.rs` dispatches to it): fd 3 is the listen
/// socket, fd 4 the control socket, `null` the `--null` argument. The scaffold records nothing.
pub fn run_recorder(null: bool) -> ExitCode {
    let _ = null;
    ExitCode::SUCCESS
}
