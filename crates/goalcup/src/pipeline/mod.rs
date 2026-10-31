//! The recorder pipeline (PRD-02): one process carries every recorded byte from the proxy and the
//! session-file reader through one memory guard and masking to the part files.
//!
//! Seam S1 (`Recorder`, `StreamHandle`) and S4 (`Counts`, `EntryTotals`, `Durable`,
//! `entry_totals`, `status.json`). Every S1 call goes through the one held-bytes guard and the
//! gap rule into an unbounded item channel; the mask worker masks off the relay and the writer
//! writes the part files. Under `test-hooks`, `Recorder::for_tests` collects every call before
//! the guard and sends nothing, and `Recorder::real_for_tests` runs the real pipeline.

/// Hands one S1 call, before masking, to the `for_tests` collector. Without `test-hooks` it
/// compiles to nothing, so the shipped recorder carries no test path.
macro_rules! collect {
    ($rec:expr, $call:expr) => {
        #[cfg(feature = "test-hooks")]
        $rec.collect(|| $call);
    };
}

mod boot;
#[cfg(feature = "test-hooks")]
mod calls;
pub mod clock;
pub mod counts;
pub mod cursors;
pub mod fds;
mod guard;
mod handle;
mod item;
mod mask_worker;
mod out;
mod seal;
pub mod secure;
pub mod status;
mod writer;

#[cfg(feature = "test-hooks")]
pub use calls::{Call, Calls};
pub use clock::Mono;
pub use counts::{Counts, Durable, EntryTotals, TunnelHost, entry_totals};
pub use cursors::Cursors;
pub use goalcup_format::{EndReason, GapReason, StreamKind};
#[cfg(feature = "test-hooks")]
pub use handle::SLOW_MARKER;
pub use handle::{OpenFlags, StreamHandle};
#[cfg(feature = "test-hooks")]
pub use mask_worker::PANIC_MARKER;
pub use status::{Live, Status};

use crate::cli;
use crate::mask::Masker;
use crate::native::{self, NativeReader};
use crate::proxy::{self, ProxyCtx, ProxyHandle, ca::RunCa};
use crate::upload::{self, UploadCtx, UploadHandle};
use crate::watchdog::{
    Exit, HarnessFacts, SealReason, ToRecorder, ToWatchdog, read_line, send_line,
};
use item::Item;
use serde_json::{Value, json};
use status::{Losing, Off, Outlived};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::os::fd::FromRawFd;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
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
    /// To the mask worker. None under `for_tests` and in null mode, which send nothing.
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
    /// a gap whose handle is never called again. Added to at each guard drop, removed when the
    /// gap closes or the seal drains it.
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
    /// Bytes offered after a stop, and each connection that carried one, by its `conn` sid.
    after_stop_bytes: AtomicU64,
    after_stop_conns: Mutex<HashSet<Sid>>,
    /// The raw host of each `conn` stream opened after a stop (no OPEN names it), kept once it
    /// ends only if it carried a byte.
    late_hosts: Mutex<HashMap<Sid, String>>,
    /// S1 calls that read the stop flags unset and have not yet pushed their item (`Flight`).
    in_flight: AtomicU64,
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
            after_stop_conns: Mutex::new(HashSet::new()),
            late_hosts: Mutex::new(HashMap::new()),
            in_flight: AtomicU64::new(0),
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
        let _f = self.flight(handle::is_wire(kind));
        collect!(
            self,
            Call::Open(sid, kind, conn, req, from, flags, meta.clone())
        );
        let handle = StreamHandle::new(self.clone(), sid, kind, conn, from);
        if !handle.opened() {
            if kind == StreamKind::Conn
                && let Some(host) = meta.get("host").and_then(Value::as_str)
            {
                lock(&self.inner.late_hosts).insert(sid, host.to_string());
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
        let _f = self.flight(false);
        if self.inner.all_stopped.load(Ordering::SeqCst) {
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
    /// null mode) counts nothing.
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
        if i.all_stopped.load(Ordering::SeqCst) {
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

/// One S1 call in flight (`Recorder::flight`); a panic in it unwinds through the drop too.
pub(super) struct Flight(Option<Arc<Inner>>);

impl Drop for Flight {
    fn drop(&mut self) {
        if let Some(i) = &self.0 {
            i.in_flight.fetch_sub(1, Ordering::SeqCst);
        }
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

/// The front's API for the mask worker, the writer, the boot and the status thread.
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

    /// Every byte that reaches no record after a stop lands in the `after_stop` count, so none
    /// is lost without a count.
    pub(crate) fn count_after_stop(&self, n: u64) {
        self.inner.after_stop_bytes.fetch_add(n, Ordering::Relaxed);
    }

    /// Connection `conn` carried a byte after a stop. Once per stream handle; connection 0
    /// (none) is not counted.
    pub(crate) fn after_stop_conn(&self, conn: Sid) {
        if conn != 0 {
            lock(&self.inner.after_stop_conns).insert(conn);
        }
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
}

/// The seal's API (PRD-02 section 11).
impl Recorder {
    /// The mask worker dropped an item worth `n` held bytes that it took: one that reached it
    /// after the seal's drain or marker (an offer or gap that read the stop flag just before it
    /// was set). Its bytes leave `held` only: they are off `queued` since [`Recorder::taken`] and
    /// no write happened. `lost` is the item's DATA length or a GAP's known loss, which goes to
    /// the `after_stop` count, so no byte is lost without a count.
    pub(crate) fn dropped(&self, n: u64, lost: u64) {
        sub(&self.inner.held, n);
        self.count_after_stop(lost);
    }

    /// Every later call on a wire stream only counts (PRD-02 section 11 step 2). Returns once
    /// every call that read the flag unset has pushed its item, so none lands behind the marker
    /// the seal sends next.
    pub(crate) fn stop_wire(&self) {
        self.inner.wire_stopped.store(true, Ordering::SeqCst);
        self.settle();
    }

    /// Every later call on any stream only counts, and events are dropped. Returns as
    /// `stop_wire` does.
    pub(crate) fn stop_all(&self) {
        self.inner.wire_stopped.store(true, Ordering::SeqCst);
        self.inner.all_stopped.store(true, Ordering::SeqCst);
        self.settle();
    }

    /// True once the seal stopped calls on this kind of stream.
    pub(super) fn stopped(&self, wire: bool) -> bool {
        let i = &self.inner;
        i.all_stopped.load(Ordering::SeqCst) || (wire && i.wire_stopped.load(Ordering::SeqCst))
    }

    /// Enters an S1 call on a wire stream or not: in flight until the returned value drops,
    /// unless the stop already holds (such a call only counts and pushes nothing). The count goes
    /// up before the caller reads the flags again, and the seal sets a flag before it reads the
    /// count (all `SeqCst`), so the seal sees every call that read a flag unset. Only calls that
    /// began before the stop are waited for, so the wait ends; it never touches a relay.
    pub(super) fn flight(&self, wire: bool) -> Flight {
        if self.stopped(wire) {
            return Flight(None);
        }
        self.inner.in_flight.fetch_add(1, Ordering::SeqCst);
        Flight(Some(self.inner.clone()))
    }

    /// Waits until no call that read a stop flag unset is still on its way to the item channel.
    fn settle(&self) {
        while self.inner.in_flight.load(Ordering::SeqCst) > 0 {
            std::thread::yield_now();
        }
    }

    /// A `conn` stream opened after a stop ended: its host is kept only if it carried a byte.
    pub(super) fn late_conn_ended(&self, sid: Sid) {
        if !lock(&self.inner.after_stop_conns).contains(&sid) {
            lock(&self.inner.late_hosts).remove(&sid);
        }
    }

    /// After `stop_all`, the seal's GAP ending at `end` for every guard gap still open (PRD-02
    /// section 11 step 5). The channel is alive until the seal, so these reach the writer.
    pub(crate) fn close_open_gaps(&self, end: Mono) {
        let gaps: Vec<_> = lock(&self.inner.open_gaps).drain().collect();
        for (sid, gap) in gaps {
            self.send_gap(sid, &gap, end);
        }
    }

    /// Bytes offered after a stop, and the connections that carried them, by sid, each with
    /// its host when it was opened after the stop. The hosts are raw OPEN meta: the seal runs
    /// each through the masker (`mask_meta`) before it reaches a record or `status.json`, so a
    /// host holding a secret shape never reaches disk.
    pub(crate) fn after_stop(&self) -> (u64, Vec<(Sid, Option<String>)>) {
        let late = lock(&self.inner.late_hosts);
        let mut conns: Vec<_> = (lock(&self.inner.after_stop_conns).iter())
            .map(|sid| (*sid, late.get(sid).cloned()))
            .collect();
        conns.sort_unstable();
        (self.inner.after_stop_bytes.load(Ordering::Relaxed), conns)
    }
}

/// A pipeline thread is gone: one fixed line and exit 70 (a later caller returns, since the
/// process is already ending).
fn pipeline_gone() {
    secure::exit(EXIT_PIPELINE_GONE, "pipeline_gone");
}

/// Ends the process with 70, naming the cause, when the thread holding it unwinds from a panic.
pub(crate) struct ExitOnPanic(pub &'static str);

impl Drop for ExitOnPanic {
    fn drop(&mut self) {
        if std::thread::panicking() {
            secure::exit(EXIT_PIPELINE_GONE, self.0);
        }
    }
}

#[cfg(feature = "test-hooks")]
impl Recorder {
    /// The real pipeline for tests (seam S1): the mask worker with `masker`, the writer and the
    /// status thread over a new entry in `dir` (part files in `dir/rec`, mode 0700), boot 0 with a
    /// made-up entry id, a 1 MiB zstd window (PRD-14 section 4) and the soft guard of
    /// `GOALCUP_TEST_GUARD_MB` when set. Its threads run for the life of the process.
    pub fn real_for_tests(dir: &std::path::Path, masker: crate::mask::Masker) -> Recorder {
        const ENTRY: &str = "e00000000000000a1";
        let rec_dir = dir.join("rec");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&rec_dir)
            .expect("the test entry folder");
        let opts = writer::part_options(20);
        let part = goalcup_format::PartWriter::open(
            &rec_dir,
            opts,
            ENTRY,
            clock::boot_id(),
            clock::now_pair(),
        )
        .expect("the test part file");
        let mut log = counts::CountsLog::open(dir);
        let base = log.sync(dir).expect("counts.jsonl");
        let mask_rules = masker.mask_rules();
        let facts = crate::watchdog::StartFacts {
            run: "r00000000000000a1".into(),
            entry: ENTRY.into(),
            segment: 0,
            entry_dir: dir.to_path_buf(),
            deadline: None,
            server_time: None,
            local_wall: 0,
            workspace: dir.to_path_buf(),
            connector: Value::Null,
            harness_version: None,
            keep: true,
            build: None,
            hosts: Value::Null,
            hosts_sha256: String::new(),
            native: Value::Null,
            mask_rules: Value::Null,
            values: Value::Null,
            ca: crate::watchdog::CaPem {
                cert: String::new(),
                key: String::new(),
            },
            prev: None,
            harness: None,
            null_since: None,
            notes: Vec::new(),
        };
        let typed = crate::watchdog::Typed {
            connector: crate::connector::ConnectorRef {
                id: "test".into(),
                name: "test".into(),
                sha256: String::new(),
                source: crate::connector::ConnectorSource::Builtin,
            },
            hosts: crate::connector::HostList::default(),
            native: crate::connector::NativeSources::default(),
            mask_rules: Vec::new(),
            values: Vec::new(),
        };
        let start = boot::start_event(&facts, &typed, &masker, &mask_rules, None, false);
        let (rec, rx) = Recorder::pipeline(guard::soft_guard());
        let (tx, out_rx) = mpsc::sync_channel(mask_worker::TO_WRITER);
        let board = status::Board::default();
        mask_worker::spawn(rec.clone(), rx, masker, tx);
        let start = writer::WriterStart {
            part,
            entry_dir: dir.to_path_buf(),
            next_seq: 0,
            log,
            base,
            mask_rules,
            start,
            board: board.clone(),
        };
        writer::spawn(rec.clone(), out_rx, start);
        status::spawn(rec.clone(), dir.to_path_buf(), board);
        rec
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

/// The first control line is missing, too long, not a `start` line or holds a field that does
/// not parse.
const EXIT_NO_START: i32 = 65;
/// The run CA from the first line cannot be imported.
const EXIT_CA: i32 = 76;
/// The masker cannot be built from its rules (PRD-02 section 8 step 5).
const EXIT_MASKER: i32 = 78;

/// The hidden recorder subcommand (PRD-10's `main.rs` dispatches to it; PRD-02 section 9.2): fd 3
/// is the listen socket, fd 4 the control socket, `null` the `--null` argument. In this order:
/// no core dump, the first line, the entry folder and the panic hook before any thread, the entry
/// lock, the masker, the run CA, the proxy at once, the boot, the native reader, the uploader,
/// the status thread, `ready`; then the control lines until the seal. A null recorder relays and
/// records nothing, and so does a boot on an entry already sealed.
pub fn run_recorder(null: bool) -> ExitCode {
    secure::harden();
    // SAFETY: the watchdog's spawn hands fd 4 to this process as a connected Unix socket, owned
    // by nothing else here.
    let ctl = unsafe { UnixStream::from_raw_fd(4) };
    let Ok(read) = ctl.try_clone() else {
        secure::die(EXIT_NO_START, "no_start_line")
    };
    let mut lines = BufReader::new(read);
    // The first line holds the CA key and the exact values: never logged, never echoed.
    let Some(Some(ToRecorder::Start(facts))) = read_line(&mut lines) else {
        secure::die(EXIT_NO_START, "no_start_line")
    };
    let mut facts = *facts;
    let Some(mut typed) = facts.typed() else {
        secure::die(EXIT_NO_START, "no_start_line")
    };
    let dir = facts.entry_dir.clone();
    let _ = std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir.join("rec"));
    secure::panic_hook(&dir);
    let _lock = boot::lock(&dir);
    let masker = match null {
        true => None,
        false => match Masker::new(&typed.mask_rules, std::mem::take(&mut typed.values)) {
            Ok(m) => Some(m),
            Err(_) => secure::die(EXIT_MASKER, "masker"),
        },
    };
    let Ok(ca) = RunCa::import(&facts.ca) else {
        secure::die(EXIT_CA, "ca_import")
    };
    // SAFETY: the watchdog's spawn hands fd 3 to this process as a listening TCP socket, owned by
    // nothing else here.
    let listener = unsafe { std::net::TcpListener::from_raw_fd(3) };
    let port = listener.local_addr().map_or(0, |a| a.port());
    let (rec, rx) = match null {
        true => (
            Recorder {
                inner: Arc::new(Inner::new(guard::CEILING, None)),
            },
            None,
        ),
        false => {
            let (rec, rx) = Recorder::pipeline(guard::soft_guard());
            (rec, Some(rx))
        }
    };
    let proxy = proxy::serve(
        listener,
        ProxyCtx {
            recorder: rec.clone(),
            hosts: typed.hosts.clone(),
            ca,
        },
    );
    let board = status::Board::default();
    board.set(|s| {
        s.pid = std::process::id();
        s.start_id = cli::proc_start_id(s.pid).unwrap_or(0);
        s.port = port;
        s.deadline = facts
            .deadline
            .map(|ms| goalcup_format::dump::wall(ms * 1_000_000));
        s.harness = facts.harness.as_ref().map(harness_status);
    });
    let mut run = Run {
        rec: rec.clone(),
        board: board.clone(),
        ctl,
        proxy: Some(proxy),
        native: None,
        uploader: None,
        status: None,
        harness: facts.harness.clone(),
        exit: None,
        exit_at: None,
        null,
        sealed: false,
    };
    match (rx, &masker) {
        (Some(rx), Some(masker)) => {
            run.sealed = !boot::start(&rec, rx, &facts, &typed, masker, board.clone());
            if !run.sealed {
                let native = std::mem::take(&mut typed.native);
                run.native = Some(native::start(rec.clone(), native));
            }
        }
        _ => {
            let t = counts::entry_totals(&dir);
            let since = facts.null_since.unwrap_or(clock::wall_ns() / 1_000_000);
            let since = Some(goalcup_format::dump::wall(since * 1_000_000));
            let (spans, ms) = (t.off_spans, t.counts.off_ms);
            rec.set_off(Off { spans, ms, since });
            rec.set_counts(t.counts);
        }
    }
    // On an entry sealed before this boot the uploader still runs its wrap-up (it reads the files,
    // as in null mode), and `status.json` keeps the sealed one.
    run.uploader = Some(upload::spawn(UploadCtx {
        home: facts.home(),
        run_dir: dir.parent().map(Path::to_path_buf).unwrap_or_default(),
        entry_dir: dir.clone(),
        run: facts.run.clone(),
        entry: facts.entry.clone(),
        recorder: (!null && !run.sealed).then(|| rec.clone()),
        keep: facts.keep,
    }));
    if !run.sealed {
        run.status = Some(status::spawn(rec, dir.clone(), board));
    } else {
        // The sealed `status.json` names this recorder, which serves the wrap-up.
        let mut me = (0, 0, 0);
        board.set(|b| me = (b.pid, b.start_id, b.port));
        seal::sealed_status(&dir, |s| (s.pid, s.start_id, s.port) = me);
    }
    run.send(&ToWatchdog::Ready);
    run.control(&mut lines, &dir, &facts.workspace)
}

/// What the control loop holds for the seal.
struct Run {
    rec: Recorder,
    board: status::Board,
    ctl: UnixStream,
    proxy: Option<ProxyHandle>,
    native: Option<NativeReader>,
    uploader: Option<UploadHandle>,
    /// The status thread, which the seal stops before its own writes.
    status: Option<std::thread::JoinHandle<()>>,
    harness: Option<HarnessFacts>,
    exit: Option<Exit>,
    exit_at: Option<Mono>,
    null: bool,
    /// The entry was sealed before this boot: the recorder records nothing, and a seal removes
    /// the key, answers `sealed` and runs the wrap-up.
    sealed: bool,
}

impl Run {
    /// Reads control lines from the watchdog (PRD-02 section 8) until the seal. On EOF (the
    /// watchdog is gone, section 9.1) it watches the harness by pid and start id each second and
    /// seals with `harness_exit` once it is gone.
    fn control(mut self, lines: &mut impl BufRead, dir: &Path, workspace: &Path) -> ExitCode {
        while let Some(line) = read_line(lines) {
            match line {
                Some(ToRecorder::Launch(h)) => {
                    self.board.set(|s| s.harness = Some(harness_status(&h)));
                    self.harness = Some(h);
                }
                Some(ToRecorder::Exit { exit }) => {
                    self.board.set(|s| {
                        if let Some(h) = &mut s.harness {
                            h.exit = exit;
                        }
                    });
                    (self.exit, self.exit_at) = (exit, Some(self.rec.now()));
                }
                Some(ToRecorder::Seal { reason, outlived }) => {
                    return self.seal(reason, outlived, dir, workspace);
                }
                // A null recorder exits without sealing; the watchdog starts a normal one.
                Some(ToRecorder::Restart) if self.null => return ExitCode::SUCCESS,
                _ => {}
            }
        }
        // Slice B adds the recorder's own deadline timer to this watch.
        loop {
            let gone = (self.harness.as_ref()).is_none_or(|h| !cli::alive(h.pid, h.start_id));
            if gone {
                return self.seal(SealReason::HarnessExit, None, dir, workspace);
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    /// The seal (PRD-02 section 11), once; `sealed` goes to the watchdog at its durable point. A
    /// null recorder exits instead: the next normal boot recovers the entry and the watchdog sends
    /// the seal again. On an entry sealed before this boot, the key a seal that died before
    /// removing it left goes now, then `sealed` and the wrap-up.
    fn seal(
        mut self,
        reason: SealReason,
        outlived: Option<Outlived>,
        dir: &Path,
        workspace: &Path,
    ) -> ExitCode {
        if self.sealed {
            seal::remove_key(dir);
            self.send(&ToWatchdog::Sealed);
            seal::wrap_up_sealed(&self.rec, dir, reason, self.uploader.take());
            return ExitCode::SUCCESS;
        }
        if self.null {
            return ExitCode::SUCCESS;
        }
        let proxy = self.proxy.take();
        let quiet_since =
            proxy.map(|p| Box::new(move || p.quiet_since()) as Box<dyn Fn() -> Mono + Send>);
        let sealer = seal::Sealer {
            rec: self.rec.clone(),
            board: Some(self.board.clone()),
            status: self.status.take(),
            entry_dir: dir.to_path_buf(),
            workspace: Some(workspace.to_path_buf()),
            quiet_since,
            native: self.native.take(),
            uploader: self.uploader.take(),
        };
        let ask = seal::SealAsk {
            reason,
            server_reason: None,
            harness: self.harness.clone(),
            exit: self.exit,
            exit_at: self.exit_at,
            outlived,
        };
        sealer.run(ask, move || self.send(&ToWatchdog::Sealed));
        ExitCode::SUCCESS
    }

    /// One line to the watchdog. A watchdog that is gone is not an error: recording goes on.
    fn send(&self, line: &ToWatchdog) {
        send_line(&self.ctl, line);
    }
}

fn harness_status(h: &HarnessFacts) -> status::HarnessStatus {
    status::HarnessStatus {
        pid: h.pid,
        started: h.started.clone(),
        start_id: h.start_id,
        exit: None,
    }
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
