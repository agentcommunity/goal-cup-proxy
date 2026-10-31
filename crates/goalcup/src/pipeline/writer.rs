//! The writer thread (PRD-02 section 5): it owns the `PartWriter`, writes every record the mask
//! worker hands it, counts at write, links, syncs, publishes the durable point and rotates.
//!
//! LINK 1 s after the last one when records were written, or once 16 MiB raw wait; COUNT every
//! 60 s, then a LINK. Before each LINK the clock and suspend events, then this boot's COUNT JSON
//! at that `seq` in `counts.jsonl`. `fdatasync` at most every 5 s, after a LINK, then the
//! durable point, `counts.jsonl` compacted and `cursors.json`, then rotation when due. A write
//! error logs its kind and exits 74; a panic exits 70. At the seal's final marker it writes the
//! seal's records and its LINK (PRD-02 section 11 steps 5 and 6) and ends.

use super::clock::{self, Drift, Mono, now_pair, wall_at};
use super::counts::{self, Counter, CountsLog, EntryTotals};
use super::cursors::{Cursors, DurablePos, NativeCursor, SnapshotCursor};
use super::item::{CONTROL_HELD, SealEnd, Sealed};
use super::out::{Out, Source, ToWriter};
use super::status::{Board, Off, SealStatus};
use super::{ExitOnPanic, OpenFlags, Recorder, Sid, low_priority, secure};
use crate::cli::log::{self, Fact};
use goalcup_format::{
    EndReason, FLAG_FIN, GapReason, PartOptions, PartWriter, Record, StreamKind,
};
use serde_json::Value;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::Duration;

const S: u64 = 1_000_000_000;
const LINK_NS: u64 = S;
const LINK_RAW: u64 = 16 << 20;
const COUNT_NS: u64 = 60 * S;
const SYNC_NS: u64 = 5 * S;
/// The write error exit (PRD-02 section 5).
const EXIT_WRITE_ERROR: i32 = 74;

/// What the writer starts from.
pub(crate) struct WriterStart {
    /// Open, its PART written.
    pub part: PartWriter,
    pub entry_dir: PathBuf,
    /// The `seq` of the next LINK (`PartWriter` keeps its own and has no accessor).
    pub next_seq: u64,
    /// Synced at boot ([`CountsLog::sync`]), which gave `base`.
    pub log: CountsLog,
    /// Entry totals of earlier boots.
    pub base: EntryTotals,
    /// The start event's `mask_rules`, for [`Counter::new`].
    pub mask_rules: Vec<String>,
    /// The start event, written first.
    pub start: Value,
    pub board: Board,
}

/// The `PartOptions` of a recorder's part files with zstd window 2^`window_log`. Under
/// `test-hooks`, `GOALCUP_TEST_ROTATE_MB` (MiB) sets `rotate_at`.
pub(crate) fn part_options(window_log: u32) -> PartOptions {
    let opts = PartOptions {
        window_log,
        ..PartOptions::default()
    };
    #[cfg(feature = "test-hooks")]
    if let Some(mb) = std::env::var("GOALCUP_TEST_ROTATE_MB")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
    {
        return PartOptions {
            rotate_at: mb << 20,
            ..opts
        };
    }
    opts
}

/// Starts the writer. It runs until the seal's final marker or until the channel from the mask
/// worker closes.
pub(crate) fn spawn(
    rec: Recorder,
    rx: mpsc::Receiver<ToWriter>,
    start: WriterStart,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("goalcup-writer".into())
        .spawn(move || {
            let _exit = ExitOnPanic("writer_gone");
            low_priority();
            let t0 = rec.now();
            #[cfg(feature = "test-hooks")]
            let stall = stall_window(t0);
            let mut w = Writer::new(rec, start, t0);
            loop {
                #[cfg(feature = "test-hooks")]
                if let Some((from, until)) = stall {
                    let now = w.rec.now();
                    if now >= from && now < until {
                        std::thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                }
                match rx.recv_timeout(Duration::from_secs(1)) {
                    Ok(ToWriter::Rec(out)) => w.write(out),
                    Ok(ToWriter::Mark(m)) => {
                        w.rec.release(CONTROL_HELD);
                        let Some(end) = m.last else {
                            let _ = m.reply.send(None);
                            continue;
                        };
                        let _ = m.reply.send(Some(w.seal(*end)));
                        return;
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
                w.tick();
            }
        })
        .expect("the writer starts")
}

/// Test builds only: `GOALCUP_TEST_WRITER_STALL=<from_ms>:<for_ms>` holds the writer, receiving
/// nothing, from `from_ms` after its start for `for_ms`.
#[cfg(feature = "test-hooks")]
fn stall_window(t0: Mono) -> Option<(Mono, Mono)> {
    let v = std::env::var("GOALCUP_TEST_WRITER_STALL").ok()?;
    let (from, len) = v.trim().split_once(':')?;
    let ms = |s: &str| s.parse::<u64>().ok().map(|n| n * 1_000_000);
    let from = t0.ns() + ms(from)?;
    Some((Mono(from), Mono(from + ms(len)?)))
}

/// What the writer keeps per open stream it acts on beyond counting.
struct Track {
    kind: StreamKind,
    /// An `http_req` with `POST` on a decrypted connection.
    post: bool,
    source: Option<Source>,
}

struct Writer {
    rec: Recorder,
    /// Taken by the seal.
    w: Option<PartWriter>,
    dir: PathBuf,
    seq: u64,
    log: CountsLog,
    base: EntryTotals,
    counter: Counter,
    board: Board,
    drift: Drift,
    boot: u64,
    t0: Mono,
    tracks: HashMap<Sid, Track>,
    cursors: Cursors,
    /// Entry totals at the last LINK.
    at_link: EntryTotals,
    /// Records written since the last LINK; a LINK not yet synced.
    dirty: bool,
    unsynced: bool,
    last_link: Mono,
    last_count: Mono,
    last_sync: Mono,
}

impl Writer {
    fn new(rec: Recorder, st: WriterStart, t0: Mono) -> Writer {
        let np = now_pair();
        let boot = st.part.boot();
        rec.set_boot(boot);
        let cursors = rec.native_cursors();
        let mut w = Writer {
            w: Some(st.part),
            dir: st.entry_dir,
            seq: st.next_seq,
            log: st.log,
            at_link: st.base.clone(),
            base: st.base,
            counter: Counter::new(&st.mask_rules),
            board: st.board,
            drift: Drift::new(np.wall_ns, np.mono_ns, clock::awake_ns()),
            boot,
            t0,
            tracks: HashMap::new(),
            cursors,
            dirty: false,
            unsynced: false,
            last_link: t0,
            last_count: t0,
            last_sync: t0,
            rec,
        };
        w.event(t0, &st.start);
        w
    }

    /// Writes one record, counts it, releases its held bytes and tracks what status and the
    /// cursors need.
    fn write(&mut self, out: Out) {
        let mut r = out.rec;
        if let Record::Gap {
            start_wall,
            end_wall,
            start_mono,
            end_mono,
            ..
        } = &mut r
        {
            let now = now_pair();
            *start_wall = wall_at(Mono(*start_mono), now);
            *end_wall = wall_at(Mono(*end_mono), now);
        }
        if let Err(e) = self.part().write(&r) {
            self.fail(&e);
        }
        self.counter.written(&r, out.flags, out.host.as_deref());
        self.rec.release(out.held);
        self.dirty = true;
        self.track(&r, out.flags, out.source, out.note);
    }

    fn track(&mut self, r: &Record, flags: OpenFlags, source: Option<Source>, note: bool) {
        let wall = |t: u64| {
            let w = wall_at(Mono(t), now_pair());
            Some(goalcup_format::dump::wall(w / S * S))
        };
        match r {
            Record::Open { sid, skind, .. } => {
                if let Some(Source::Native {
                    path,
                    generation,
                    fid,
                    from,
                }) = &source
                {
                    let c = NativeCursor {
                        generation: *generation,
                        fid: *fid,
                        off: *from,
                    };
                    self.cursors.native.insert(path.clone(), c);
                }
                let post = flags.contains(OpenFlags::POST | OpenFlags::DECRYPTED);
                let kind = *skind;
                if post || source.is_some() || kind == StreamKind::WsUp {
                    let t = Track { kind, post, source };
                    self.tracks.insert(*sid, t);
                }
            }
            Record::Data {
                sid,
                t,
                off,
                flags,
                bytes,
            } => match self.tracks.get(sid) {
                Some(Track {
                    source: Some(Source::Native { path, .. }),
                    ..
                }) => {
                    if let Some(c) = self.cursors.native.get_mut(path) {
                        c.off = off + bytes.len() as u64;
                    }
                    let at = wall(*t);
                    self.board.set(|s| s.last_native_at = at);
                }
                Some(Track {
                    kind: StreamKind::WsUp,
                    ..
                }) if flags & FLAG_FIN != 0 && matches!(flags >> 4, 0x1 | 0x2) => {
                    let at = wall(*t);
                    self.board.set(|s| s.last_request_at = at);
                }
                _ => {}
            },
            Record::Gap { sid, off, lost, .. } if *lost > 1 => {
                if let Some(Track {
                    source: Some(Source::Native { path, .. }),
                    ..
                }) = self.tracks.get(sid)
                    && let Some(c) = self.cursors.native.get_mut(path)
                {
                    c.off = off + lost - 1;
                }
            }
            Record::End { sid, t, reason, .. } => match self.tracks.remove(sid) {
                Some(Track { post: true, .. }) => {
                    let at = wall(*t);
                    self.board.set(|s| s.last_request_at = at);
                }
                Some(Track {
                    source:
                        Some(Source::Snapshot {
                            path,
                            size,
                            mtime_ns,
                        }),
                    ..
                }) if *reason == EndReason::Complete => {
                    let c = SnapshotCursor { size, mtime_ns };
                    self.cursors.snapshots.insert(path, c);
                }
                _ => {}
            },
            Record::Event { json, .. } if note => {
                if let Ok(v) = serde_json::from_slice::<Value>(json) {
                    self.board.note(v);
                }
            }
            _ => {}
        }
    }

    /// One of the writer's own events (start, count, clock, suspend), at `t`.
    fn event(&mut self, t: Mono, v: &Value) {
        let r = Record::Event {
            t: t.ns(),
            json: serde_json::to_vec(v).unwrap_or_default(),
        };
        if let Err(e) = self.part().write(&r) {
            self.fail(&e);
        }
        self.dirty = true;
    }

    /// After each record or a second with none: COUNT and LINK, LINK, sync and rotation as due.
    fn tick(&mut self) {
        let now = self.rec.now();
        let since = |t: Mono| now.ns().saturating_sub(t.ns());
        if since(self.last_count) >= COUNT_NS {
            let count = self.count();
            self.event(now, &count);
            self.last_count = now;
            self.link();
        } else if (self.dirty && since(self.last_link) >= LINK_NS)
            || self.part().raw_since_link() >= LINK_RAW
        {
            self.link();
        }
        if self.unsynced && since(self.last_sync) >= SYNC_NS {
            // Records written since the LINK get one first: the cursors and totals a sync
            // publishes must be those of its LINK, never ahead of it.
            if self.dirty {
                self.link();
            }
            self.sync();
        }
    }

    /// This boot's COUNT event as it stands.
    fn count(&self) -> Value {
        let up_ms = self.rec.now().ns().saturating_sub(self.t0.ns()) / 1_000_000;
        let held = self.rec.live().held_bytes;
        let peak = self.rec.inner.held_peak.load(Ordering::Relaxed);
        self.counter.count_event(self.boot, up_ms, held, peak)
    }

    /// The clock events, this boot's COUNT JSON at the LINK's `seq` in `counts.jsonl`, then the
    /// LINK and its index line.
    fn link(&mut self) {
        let np = now_pair();
        for ev in self
            .drift
            .at_link(np.wall_ns, np.mono_ns, clock::awake_ns())
        {
            self.event(Mono(np.mono_ns), &ev);
        }
        let count = self.count();
        self.append(self.seq, &count);
        match self.part().link(np) {
            Ok(line) => self.seq = line.seq + 1,
            Err(e) => self.fail(&e),
        }
        let t = self.totals(&count);
        self.linked(t);
        (self.dirty, self.unsynced, self.last_link) = (false, true, Mono(np.mono_ns));
    }

    /// The entry totals at a LINK whose COUNT is `count`.
    fn totals(&self, count: &Value) -> EntryTotals {
        let mut c = self.counter.counts();
        c.held_peak = count["held_peak"].as_u64().unwrap_or_default();
        let mut t = self.base.clone();
        t.counts.add(&c);
        t.off_spans = t.off_spans.saturating_add(self.counter.off_spans());
        t
    }

    /// `t` is the totals at the LINK just written: for `counts()`, `live()` and the durable point.
    fn linked(&mut self, t: EntryTotals) {
        self.rec.set_off(Off {
            spans: t.off_spans,
            ms: t.counts.off_ms,
            since: None,
        });
        self.rec.set_counts(t.counts.clone());
        self.at_link = t;
    }

    /// The seal's records (PRD-02 section 11 steps 5 and 6; the mask worker wrote `END
    /// cut_at_seal` for what was open just before): the `after_stop` GAP when bytes came after the
    /// stop, COUNT, the seal event, then `PartWriter::seal` (LINK, frame end, `E` line, both
    /// files synced). The durable point is published from that line, then `counts.jsonl`
    /// compacted and `cursors.json` written, so neither runs ahead of the LINK.
    fn seal(mut self, end: SealEnd) -> Sealed {
        let now = self.rec.now();
        if end.after_bytes > 0 {
            let r = Record::Gap {
                sid: 0,
                reason: GapReason::AfterStop,
                start_wall: 0,
                end_wall: 0,
                start_mono: end.stop.ns(),
                end_mono: now.max(Mono(end.stop.ns() + 1)).ns(),
                off: 0,
                lost: end.after_bytes.saturating_add(1),
                tail: serde_json::to_vec(&end.tail).unwrap_or_default(),
            };
            self.write(Out::new(r, 0));
        }
        let count = self.count();
        self.event(now, &count);
        let totals = self.totals(&count);
        let mut ev = end.event;
        ev["boots"] = (self.boot + 1).into();
        ev["totals"] = serde_json::to_value(&totals.counts).unwrap_or_default();
        self.event(now, &ev);
        self.append(self.seq, &count);
        let part = self.w.take().map(|w| w.seal(now_pair()));
        let d = match part {
            Some(Ok(d)) => d,
            Some(Err(e)) => self.fail(&e),
            None => self.fail(&io::Error::other("sealed twice")),
        };
        self.seq = d.line.seq + 1;
        self.linked(totals.clone());
        self.publish(Some(d));
        let seal = SealStatus {
            reason: end.reason,
            workspace_head: end.workspace_head,
        };
        self.board.sealed(seal, &totals);
        Sealed {
            totals,
            masker: end.masker,
        }
    }

    fn part(&mut self) -> &mut PartWriter {
        match &mut self.w {
            Some(w) => w,
            None => fail(&self.dir, &io::Error::other("write after the seal")),
        }
    }

    fn append(&mut self, seq: u64, count: &Value) {
        if let Err(e) = self.log.append(seq, count, self.counter.off_spans()) {
            self.fail(&e);
        }
    }

    /// `fdatasync`, the durable point and its files, then rotation when due. Called with no
    /// record written since the last LINK, so the cursors and totals are those of that LINK. The
    /// rotation's LINK has its `counts.jsonl` line before it, whether or not it comes: whether a
    /// rotation is due is `PartWriter`'s own (part size and age, links in the part, all private
    /// to PRD-01) and its LINK is written inside `rotate_if_due`, so the line cannot wait for the
    /// answer. A line with no LINK after it is harmless: `count_at` takes the last line of a
    /// `seq` and compaction keeps the latest per boot. A `PartWriter::rotation_due` seam would
    /// let it go.
    fn sync(&mut self) {
        let d = self.part().sync().unwrap_or_else(|e| self.fail(&e));
        self.publish(d);
        let count = self.count();
        self.append(self.seq, &count);
        let np = now_pair();
        match self.part().rotate_if_due(np) {
            Ok(Some(d)) => {
                self.seq = d.line.seq + 1;
                self.publish(Some(d));
            }
            Ok(None) => {}
            Err(e) => self.fail(&e),
        }
        (self.unsynced, self.last_sync) = (false, Mono(np.mono_ns));
    }

    fn publish(&mut self, point: Option<goalcup_format::Durable>) {
        let totals = self.at_link.clone();
        self.rec.publish_durable(counts::Durable {
            point: point.clone(),
            totals,
            at: self.rec.now(),
        });
        if let Err(e) = self.log.compact() {
            self.fail(&e);
        }
        self.cursors.durable = point.map(|d| DurablePos {
            part: d.part,
            index_len: d.index_len,
        });
        self.cursors.boot_id = Some(hex::encode(clock::boot_id()));
        if let Err(e) = self.cursors.write(&self.dir) {
            self.fail(&e);
        }
        self.rec.set_cursors(self.cursors.clone());
    }

    fn fail(&self, e: &io::Error) -> ! {
        fail(&self.dir, e)
    }
}

/// A write error: one `goalcup.log` line with its kind, then exit 74. The next boot's
/// `recorder_down` gap names the span.
fn fail(dir: &Path, e: &io::Error) -> ! {
    log::line(dir, "write_error", &[("io", Fact::Io(e.kind()))]);
    secure::exit(EXIT_WRITE_ERROR, "write_error");
    // Another thread is already ending the process.
    loop {
        std::thread::park();
    }
}
