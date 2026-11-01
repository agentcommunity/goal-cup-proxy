//! `StreamHandle` (seam S1): one stream's calls. The handle owns the stream's offset and runs the
//! gap rule (PRD-02 section 3); an open guard gap lives in the recorder's `open_gaps`, so
//! the seal can close one whose handle is never called again.

#[cfg(feature = "test-hooks")]
use super::Call;
use super::item::{self, Item, Ws};
use super::{Mono, Recorder, Sid, fds, guard};
use bytes::Bytes;
use goalcup_format::{EndReason, GapReason, StreamKind};
use serde_json::Value;
use std::ops::BitOr;

/// The largest DATA piece (PRD-01 section 3): bigger offers are split, with no copy, after the
/// whole offer is admitted or dropped, so no queued piece keeps a dropped one's memory alive.
const MAX_PIECE: usize = 256 << 10;

/// What the writer counts without parsing JSON.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct OpenFlags(u8);

impl OpenFlags {
    pub const NONE: OpenFlags = OpenFlags(0);
    /// An `http_req` with method `POST`.
    pub const POST: OpenFlags = OpenFlags(1);
    /// On a decrypted connection.
    pub const DECRYPTED: OpenFlags = OpenFlags(2);
    /// A `conn` stream that is tunnelled.
    pub const TUNNEL: OpenFlags = OpenFlags(4);

    pub fn contains(self, other: OpenFlags) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn bits(self) -> u8 {
        self.0
    }
}

impl BitOr for OpenFlags {
    type Output = OpenFlags;
    fn bitor(self, rhs: OpenFlags) -> OpenFlags {
        OpenFlags(self.0 | rhs.0)
    }
}

/// One stream, owned by one task; not `Clone`. Nothing blocks and nothing returns a value the
/// caller must act on. Dropped without `end()`, the stream ends `interrupted`.
pub struct StreamHandle {
    rec: Recorder,
    sid: Sid,
    /// `conn`, `http_req`, `http_resp`, `ws_up` or `ws_down`: stopped by the seal's wire stop.
    wire: bool,
    /// A `conn` stream: counted open for `fds::may_accept` until it ends.
    conn: bool,
    /// Its OPEN was sent: false for a stream opened after a stop, which sends nothing.
    opened: bool,
    /// The `conn` stream it belongs to (its own sid for a `conn` stream; 0 for none), for the
    /// `after_stop` tail.
    conn_sid: Sid,
    /// A byte after the stop on this stream was noted on its connection.
    noted: bool,
    off: u64,
    /// A guard gap is open on this stream; it lives in the recorder's `open_gaps`.
    in_gap: bool,
    ended: bool,
}

impl StreamHandle {
    pub(super) fn new(
        rec: Recorder,
        sid: Sid,
        kind: StreamKind,
        conn_sid: Sid,
        from: u64,
    ) -> StreamHandle {
        let wire = is_wire(kind);
        let conn = kind == StreamKind::Conn;
        if conn {
            fds::conn_opened();
        }
        let conn_sid = if conn { sid } else { conn_sid };
        let mut h = StreamHandle {
            rec,
            sid,
            wire,
            conn,
            opened: true,
            conn_sid: if wire { conn_sid } else { 0 },
            noted: false,
            off: from,
            in_gap: false,
            ended: false,
        };
        h.opened = !h.stopped();
        h
    }

    pub fn sid(&self) -> Sid {
        self.sid
    }

    /// Its OPEN is sent: false when it was opened after a stop.
    pub(super) fn opened(&self) -> bool {
        self.opened
    }

    /// `n` bytes after a stop: into the `after_stop` count, and the first time also the
    /// stream's connection into the `after_stop` connections (PRD-02 section 11 steps 2 and 5).
    fn after_stop(&mut self, n: u64) {
        self.rec.count_after_stop(n);
        if n > 0 {
            self.note_conn();
        }
    }

    fn note_conn(&mut self) {
        if !self.noted {
            self.noted = true;
            self.rec.after_stop_conn(self.conn_sid);
        }
    }

    /// After the seal's stop a call on this stream only counts (PRD-02 section 11 step 2).
    pub(super) fn stopped(&self) -> bool {
        self.rec.stopped(self.wire)
    }

    /// The seal stopped everything and closes what is still open itself.
    fn all_stopped(&self) -> bool {
        self.rec.stopped(false)
    }

    /// One piece; moves the offset by its length, written or dropped.
    pub fn data(&mut self, t: Mono, bytes: Bytes) {
        collect!(self.rec, Call::Data(self.sid, t, self.off, bytes.clone()));
        let _f = self.rec.flight(self.wire);
        self.offer(t, None, bytes);
    }

    /// One WebSocket piece; `fin` marks a message end.
    pub fn ws(&mut self, t: Mono, opcode: u8, fin: bool, bytes: Bytes) {
        collect!(
            self.rec,
            Call::Ws(self.sid, t, self.off, opcode, fin, bytes.clone())
        );
        let _f = self.rec.flight(self.wire);
        self.offer(t, Some(Ws { opcode, fin }), bytes);
    }

    /// The caller's own loss, reported when it ends: `off` is the handle's offset, the end is now
    /// (at least 1 ns after `start`, PRD-01 invariant 12). `Some(n)` moves the offset by `n`;
    /// `None` means unknown and leaves it. An open guard gap is sent first. Loss that began
    /// before the seal's wire stop is still sent after it, until the seal's full stop; after
    /// that, and on a stream opened after a stop (it has no OPEN), its bytes go to the
    /// `after_stop` count. Bytes offered after a stop are in that count already, so a caller
    /// reports only loss of bytes it took before the stop.
    pub fn gap(&mut self, reason: GapReason, start: Mono, lost: Option<u64>, tail: Option<Value>) {
        let end = self.rec.now().max(Mono(start.ns() + 1));
        collect!(
            self.rec,
            Call::Gap(self.sid, reason, start, end, self.off, lost, tail.clone())
        );
        let _f = self.rec.flight(false);
        if self.all_stopped() || !self.opened {
            self.after_stop(lost.unwrap_or(0));
        } else {
            self.close_gap(end);
            self.rec.send(Item::Gap {
                sid: self.sid,
                reason,
                start,
                end,
                off: self.off,
                lost,
                tail,
            });
        }
        self.off += lost.unwrap_or(0);
    }

    pub fn end(mut self, t: Mono, reason: EndReason, tail: Option<Value>) {
        self.finish(t, reason, tail);
    }

    /// Sends the open guard gap, then END. After the wire stop the guard gap is still sent and
    /// END is left to the seal (`END cut_at_seal`); after the full stop the seal closes both.
    fn finish(&mut self, t: Mono, reason: EndReason, tail: Option<Value>) {
        self.ended = true;
        if self.conn {
            fds::conn_closed();
        }
        collect!(
            self.rec,
            Call::End(self.sid, t, reason, self.off, tail.clone())
        );
        if self.conn && !self.opened {
            self.rec.late_conn_ended(self.sid);
        }
        let _f = self.rec.flight(false);
        if self.all_stopped() {
            return;
        }
        self.close_gap(t);
        if self.stopped() {
            return;
        }
        self.rec.send(Item::End {
            sid: self.sid,
            t,
            reason,
            seen: self.off,
            tail,
        });
    }

    /// The gap rule (PRD-02 section 3), once per offer: the whole offer is admitted or dropped,
    /// then split into pieces of at most 256 KiB (a WebSocket message keeps its opcode on every
    /// piece and `fin` on the last). Native and snapshot offers are always admitted (they read at
    /// most `room()`, and the file keeps what is not read), and so is an empty message end. A
    /// wire offer larger than the guard is dropped behind a GAP, so S1 callers offer pieces far
    /// below it (PRD-03 offers at most 32 KiB). After a stop the bytes only count: they are in no
    /// record, so the offset stays where the record ends.
    fn offer(&mut self, t: Mono, ws: Option<Ws>, bytes: Bytes) {
        let n = bytes.len() as u64;
        if self.stopped() {
            self.after_stop(n);
            return;
        }
        #[cfg(feature = "test-hooks")]
        if bytes.starts_with(SLOW_MARKER) {
            // A relay thread held between its stop check and its push, as under load.
            SLOW_HELD.store(true, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        let rec = &self.rec;
        if n == 0 && !ws.is_some_and(|w| w.fin) {
            // Only a message end has anything to say with no bytes.
            return;
        }
        let (soft, in_gap, always) = (rec.inner.guard, self.in_gap, !self.wire || n == 0);
        // What the pieces below count in `held` together, so the hold and their `Item::held` match.
        let pieces = bytes.len().div_ceil(MAX_PIECE).max(1) as u64;
        let held = n + pieces * item::DATA_ITEM_HELD;
        let admitted = rec.inner.tx.is_none()
            || rec.hold(held, |h| always || guard::admits(h, held, in_gap, soft));
        if !admitted {
            // After a full stop that raced this offer the bytes only count, as above.
            if rec.guard_drop(self.sid, &mut self.in_gap, t, self.off, n) {
                self.off += n;
            } else if n > 0 {
                self.note_conn();
            }
            return;
        }
        self.close_gap(t);
        let len = bytes.len();
        let mut at = 0;
        loop {
            let k = MAX_PIECE.min(len - at);
            let last = at + k == len;
            self.rec.push(Item::Data {
                sid: self.sid,
                t,
                off: self.off,
                ws: ws.map(|w| Ws {
                    opcode: w.opcode,
                    fin: w.fin && last,
                }),
                bytes: bytes.slice(at..at + k),
            });
            self.off += k as u64;
            at += k;
            if last {
                break;
            }
        }
    }

    /// Sends the open guard gap, if the seal has not taken it, ending at `end`.
    fn close_gap(&mut self, end: Mono) {
        if std::mem::take(&mut self.in_gap) {
            self.rec.close_gap(self.sid, end);
        }
    }
}

/// Test builds only: an offer starting with these bytes waits 500 ms after its stop check
/// (`tests/pipeline_seal.rs`). No env var is read.
#[cfg(feature = "test-hooks")]
pub const SLOW_MARKER: &[u8] = b"\x01slow-offer-marker\x02";

/// Test builds only: set once an offer with [`SLOW_MARKER`] is past its stop check, so a test
/// starts the seal while the offer is held there.
#[cfg(feature = "test-hooks")]
pub static SLOW_HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `conn`, `http_req`, `http_resp`, `ws_up` or `ws_down`: a stream the seal's wire stop stops.
pub(super) fn is_wire(kind: StreamKind) -> bool {
    matches!(
        kind,
        StreamKind::Conn
            | StreamKind::HttpReq
            | StreamKind::HttpResp
            | StreamKind::WsUp
            | StreamKind::WsDown
    )
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        if !self.ended {
            let t = self.rec.now();
            self.finish(t, EndReason::Interrupted, None);
        }
    }
}
