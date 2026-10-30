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
use serde_json::{Value, json};
use status::{Losing, Off};
use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use tokio::sync::watch;

/// A stream ID: from 1, never reused inside a boot; 0 means "no stream".
pub type Sid = u64;

/// The exit code when a pipeline thread is gone (PRD-02 section 2 rule 1).
pub(crate) const EXIT_PIPELINE_GONE: i32 = 70;

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
    /// Bytes offered and not yet written, plus reservations: what the guard checks.
    held: AtomicU64,
    held_peak: AtomicU64,
    /// Held bytes of items in the item channel or the writer's channel, waiting for the writer:
    /// records wait while this is above 0. Left out: reservations, and bytes the mask worker
    /// holds back for a match that may go on (PRD-05), since no write is due for them.
    queued: AtomicU64,
    /// Awake time (`clock::awake_ns`) when `queued` last left 0, stamped before it leaves 0.
    wait_since_ns: AtomicU64,
    /// Bytes the guard dropped this boot.
    dropped_bytes: AtomicU64,
    streams_in_gap: AtomicU64,
    /// The guard gap open on each stream that is in one (PRD-02 section 3), so the seal can close
    /// a gap whose handle is never called again. Taken at each guard drop and when a gap closes.
    open_gaps: Mutex<HashMap<Sid, GuardGap>>,
    /// Awake time of the writer's last write. The awake clock stops during sleep, so a suspend
    /// is never a stall.
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
            queued: AtomicU64::new(0),
            wait_since_ns: AtomicU64::new(0),
            dropped_bytes: AtomicU64::new(0),
            streams_in_gap: AtomicU64::new(0),
            open_gaps: Mutex::new(HashMap::new()),
            writer_last_ns: AtomicU64::new(clock::awake_ns()),
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
            ..Inner::new(guard::CEILING, None)
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
        if !handle.opened() {
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

    /// Counts `n` in `held` until the returned `Reserved` drops (a decoder window, PRD-03). A
    /// reservation is never a record waiting, so it never counts toward the stall exit.
    pub fn reserve(&self, n: u64) -> Reserved {
        self.hold(n, |_| true);
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
        self.live_at(clock::awake_ns())
    }

    /// `live()` with the awake clock at `now`.
    fn live_at(&self, now: u64) -> Live {
        let i = &self.inner;
        // Idle from the later of the last write and the moment records began to wait.
        let writer_idle_ms = if i.queued.load(Ordering::Acquire) > 0 {
            let since = i
                .writer_last_ns
                .load(Ordering::Relaxed)
                .max(i.wait_since_ns.load(Ordering::Relaxed));
            now.saturating_sub(since) / 1_000_000
        } else {
            0
        };
        Live {
            boot: i.boot.load(Ordering::Relaxed),
            held_bytes: i.held.load(Ordering::Relaxed),
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

    /// Counts `n` more in `held` when `admit` agrees with the bytes held now, in one atomic
    /// step, so callers offering at once never overshoot the guard. True if counted.
    fn hold(&self, n: u64, admit: impl Fn(u64) -> bool) -> bool {
        let i = &self.inner;
        match i
            .held
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |h| {
                admit(h).then_some(h.saturating_add(n))
            }) {
            Ok(h) => {
                i.held_peak
                    .fetch_max(h.saturating_add(n), Ordering::Relaxed);
                true
            }
            Err(_) => false,
        }
    }

    /// Counts the item in `held`, then hands it on. A recorder that sends nothing (`for_tests`,
    /// the scaffold) counts nothing.
    fn send(&self, item: Item) {
        if self.inner.tx.is_none() {
            return;
        }
        self.hold(item.held(), |_| true);
        self.push(item);
    }

    /// Hands on an item already counted in `held`, and counts it in `queued`. No lock is held
    /// here. A gone receiver means a pipeline thread died: exit 70 so the watchdog restarts the
    /// recorder.
    fn push(&self, item: Item) {
        let Some(tx) = &self.inner.tx else {
            return;
        };
        self.queue(item.held());
        if tx.send(item).is_err() {
            pipeline_gone();
        }
    }

    /// `n` more held bytes wait for the writer. The wait stamp is stored before `queued` leaves
    /// 0, so the status thread never sees records waiting with an old stamp (a false stall). Two
    /// callers may both stamp; that only moves the stall clock a little later.
    fn queue(&self, n: u64) {
        let i = &self.inner;
        if i.queued.load(Ordering::Acquire) == 0 {
            i.wait_since_ns.store(clock::awake_ns(), Ordering::Release);
        }
        i.queued.fetch_add(n, Ordering::Release);
    }

    /// The guard dropped `n` bytes of stream `sid` at offset `off`: they go into its open guard
    /// gap, or open one starting at `t`. Under the `open_gaps` lock the full stop is checked
    /// again, and `close_open_gaps` drains that map under the same lock after the stop, so the
    /// bytes land in a GAP the seal or the handle sends, or else in the `after_stop` count, never
    /// in neither. False when they went to the `after_stop` count.
    pub(crate) fn guard_drop(
        &self,
        sid: Sid,
        in_gap: &mut bool,
        t: Mono,
        off: u64,
        n: u64,
    ) -> bool {
        let i = &self.inner;
        let mut open = lock(&i.open_gaps);
        if i.all_stopped.load(Ordering::Acquire) {
            drop(open);
            self.count_after_stop(n);
            return false;
        }
        open.entry(sid)
            .and_modify(|g| g.lost += n)
            .or_insert_with(|| {
                i.streams_in_gap.fetch_add(1, Ordering::Relaxed);
                GuardGap {
                    start: t,
                    off,
                    lost: n,
                }
            });
        drop(open);
        *in_gap = true;
        i.dropped_bytes.fetch_add(n, Ordering::Relaxed);
        true
    }

    /// Sends the guard gap open on `sid`, if one is still open, ending at `end` (at least 1 ns
    /// after its start, PRD-01 invariant 12). Whoever takes it from the map sends it: the handle
    /// or the seal, never both.
    pub(crate) fn close_gap(&self, sid: Sid, end: Mono) {
        let gap = lock(&self.inner.open_gaps).remove(&sid);
        if let Some(gap) = gap {
            self.send_gap(sid, &gap, end);
        }
    }

    fn send_gap(&self, sid: Sid, gap: &GuardGap, end: Mono) {
        self.send(Item::Gap {
            sid,
            reason: GapReason::MemoryGuard,
            start: gap.start,
            end: end.max(Mono(gap.start.ns() + 1)),
            off: gap.off,
            lost: Some(gap.lost),
            tail: Some(json!({"guard": "held_bytes", "limit": self.inner.guard})),
        });
        sub(&self.inner.streams_in_gap, 1);
    }
}

/// Bytes the guard dropped on one stream since its last admitted piece. Lives only in
/// `open_gaps`, changed under its lock.
struct GuardGap {
    /// The first dropped piece's time.
    start: Mono,
    /// The offset of its first byte.
    off: u64,
    lost: u64,
}

/// `a -= n`, never below 0: the one rule for taking off a counter.
pub(crate) fn sub(a: &AtomicU64, n: u64) {
    let _ = a.try_update(Ordering::Release, Ordering::Relaxed, |v| {
        Some(v.saturating_sub(n))
    });
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

    /// The mask worker took an item worth `n` held bytes off the item channel. From here its
    /// bytes count in `held` only, until [`Recorder::forwarded`]: bytes it holds back for a match
    /// that may go on wait for more input, not for the writer, so a quiet stream never looks
    /// like a stall. What it takes is forwarded, dropped or still held: taken = forwarded +
    /// [`Recorder::dropped`] + held back.
    pub(crate) fn taken(&self, n: u64) {
        sub(&self.inner.queued, n);
    }

    /// The mask worker handed records worth `n` held bytes to the writer's channel, so `queued`
    /// returns to exactly 0 when the writer has written all it was handed.
    pub(crate) fn forwarded(&self, n: u64) {
        self.queue(n);
    }

    /// The mask worker dropped an item worth `n` held bytes that it took: one that reached it
    /// after the seal's drain or marker (an offer or gap that read the stop flag just before it
    /// was set). Its bytes leave `held` only: they are off `queued` since [`Recorder::taken`] and
    /// no write happened. `lost` is the item's DATA length or a GAP's known loss, which goes to
    /// the `after_stop` count, so no byte is lost without a count.
    pub(crate) fn dropped(&self, n: u64, lost: u64) {
        sub(&self.inner.held, n);
        self.count_after_stop(lost);
    }

    /// Every byte that reaches no record after a stop lands in the `after_stop` count, so none
    /// is lost without a count.
    pub(crate) fn count_after_stop(&self, n: u64) {
        self.inner.after_stop_bytes.fetch_add(n, Ordering::Relaxed);
    }

    /// The writer handed records worth `n` held bytes to the compressor. Only the writer calls
    /// it, for records it was handed. `release(0)` marks progress with nothing written, for work
    /// such as a long recount at boot, so it is not a stall. The write stamp is stored before
    /// `queued` goes down.
    pub(crate) fn release(&self, n: u64) {
        self.inner
            .writer_last_ns
            .store(clock::awake_ns(), Ordering::Release);
        sub(&self.inner.held, n);
        sub(&self.inner.queued, n);
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

    /// After `stop_all`, the seal's GAP ending at `end` for every guard gap still open (PRD-02
    /// section 11 step 5). The channel is alive until the seal, so these reach the writer.
    pub(crate) fn close_open_gaps(&self, end: Mono) {
        let gaps: Vec<_> = lock(&self.inner.open_gaps).drain().collect();
        for (sid, gap) in gaps {
            self.send_gap(sid, &gap, end);
        }
    }

    /// Bytes offered after a stop, and the hosts of `conn` streams opened after it. The hosts are
    /// raw OPEN meta: the seal runs each through the masker (`mask_meta`) before it reaches
    /// `status.json` or `goalcup.log`, so a host holding a secret shape never reaches disk.
    pub(crate) fn after_stop(&self) -> (u64, Vec<String>) {
        (
            self.inner.after_stop_bytes.load(Ordering::Relaxed),
            lock(&self.inner.after_stop_hosts).clone(),
        )
    }
}

/// A pipeline thread is gone: one fixed line and exit 70 (a later caller returns, since the
/// process is already ending).
fn pipeline_gone() {
    secure::exit(EXIT_PIPELINE_GONE, "pipeline_gone");
}

/// `n` bytes counted in `held` until dropped.
#[must_use = "the reservation ends when this is dropped"]
pub struct Reserved {
    rec: Recorder,
    n: u64,
}

impl Drop for Reserved {
    fn drop(&mut self) {
        sub(&self.rec.inner.held, self.n);
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

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 61_000_000_000;

    /// Point 3: a quiet minute, a reservation alone or the first record after a pause is never a
    /// stall; a record left waiting for 60 s with no write is.
    #[test]
    fn the_stall_exit_needs_a_record_waiting_with_no_write() {
        let (rec, rx) = Recorder::pipeline(guard::CEILING);
        let stalled = |at: u64| status::stalled(&rec.live_at(at), rec.inner.guard);
        // The last write was long ago.
        rec.inner.writer_last_ns.store(0, Ordering::Relaxed);

        let window = rec.reserve(16 << 20);
        let now = clock::awake_ns();
        assert_eq!(rec.live_at(now + MIN).writer_idle_ms, 0);
        assert!(!stalled(now + MIN));

        rec.event(json!({"ev": "note"}));
        let now = clock::awake_ns();
        assert!(!stalled(now));
        // The record still waits a minute later, with no write since.
        assert!(stalled(now + MIN));

        for item in rx.try_iter() {
            rec.release(item.held());
        }
        drop(window);
        assert_eq!(rec.live().held_bytes, 0);
        assert!(!stalled(clock::awake_ns() + MIN));
    }

    /// Point 3: bytes the mask worker holds back for a match that may go on wait for more input,
    /// not for the writer, so a quiet minute with a candidate held is not a stall.
    #[test]
    fn a_candidate_held_by_the_mask_worker_is_not_a_stall() {
        let (rec, rx) = Recorder::pipeline(1 << 20);
        let stalled = |at: u64| status::stalled(&rec.live_at(at), rec.inner.guard);
        rec.inner.writer_last_ns.store(0, Ordering::Relaxed);
        let mut h = rec.open(StreamKind::HttpResp, 0, 0, 0, OpenFlags::NONE, json!({}));
        h.data(
            rec.now(),
            bytes::Bytes::from_static(b"data: {\"delta\": \"partial\"}"),
        );
        // The mask worker takes both items, forwards the OPEN and the clean head of the piece,
        // and holds the 11-byte tail; the writer writes what it was handed.
        let items: Vec<Item> = rx.try_iter().collect();
        for item in &items {
            rec.taken(item.held());
        }
        let forwarded = items.iter().map(Item::held).sum::<u64>() - 11;
        rec.forwarded(forwarded);
        rec.release(forwarded);
        let now = clock::awake_ns();
        assert_eq!(rec.live_at(now).held_bytes, 11);
        assert!(!stalled(now + MIN));
        // The held tail goes on to the writer and waits there: that is a stall again.
        rec.forwarded(11);
        assert!(stalled(clock::awake_ns() + MIN));
        // An item that reached the mask worker after the seal is dropped and counted; the tail
        // still waits for the writer, so the stall stays seen.
        h.data(rec.now(), bytes::Bytes::from_static(b"sixteen bytes ok"));
        for item in rx.try_iter() {
            rec.taken(item.held());
            rec.dropped(item.held(), 16);
        }
        assert_eq!(rec.live().held_bytes, 11);
        assert_eq!(rec.after_stop().0, 16);
        assert!(stalled(clock::awake_ns() + MIN));
        rec.release(11);
        assert_eq!(rec.live().held_bytes, 0);
        assert!(!stalled(clock::awake_ns() + MIN));
    }
}
