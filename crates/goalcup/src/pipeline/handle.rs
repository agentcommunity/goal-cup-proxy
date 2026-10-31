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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc::Receiver;

    const MIB: u64 = 1 << 20;

    fn open(rec: &Recorder, kind: StreamKind, from: u64) -> StreamHandle {
        rec.open(kind, 0, 0, from, OpenFlags::NONE, json!({}))
    }

    /// Every item now in the channel, each released as the writer would.
    fn drain(rec: &Recorder, rx: &Receiver<Item>) -> Vec<Item> {
        let items: Vec<Item> = rx.try_iter().collect();
        for item in &items {
            rec.release(item.held());
        }
        items
    }

    fn sid_of(item: &Item) -> Sid {
        match item {
            Item::Open { sid, .. }
            | Item::Data { sid, .. }
            | Item::Gap { sid, .. }
            | Item::End { sid, .. } => *sid,
            Item::Event { .. } | Item::Mark(_) => 0,
        }
    }

    /// Checks one stream's items: OPEN first, DATA and GAP holes cover `[from, seen)` with no
    /// hole or overlap, every guard GAP has an extent and is followed by a DATA or the END (one
    /// GAP per drop episode), and END's `seen` is the end. Returns `seen`.
    fn check_stream(items: &[&Item], from: u64) -> u64 {
        assert!(matches!(items.first(), Some(Item::Open { .. })));
        let mut at = from;
        let mut after_gap = false;
        for item in &items[1..] {
            match item {
                Item::Data { off, bytes, .. } => {
                    assert_eq!(*off, at);
                    at += bytes.len() as u64;
                    after_gap = false;
                }
                Item::Gap {
                    reason,
                    start,
                    end,
                    off,
                    lost,
                    ..
                } => {
                    assert_eq!(*reason, GapReason::MemoryGuard);
                    assert!(!after_gap, "two guard GAPs with no piece between");
                    assert!(start < end);
                    assert_eq!(*off, at);
                    at += lost.expect("a guard GAP knows its bytes");
                    after_gap = true;
                }
                Item::End { seen, .. } => {
                    assert_eq!(*seen, at);
                    return at;
                }
                other => panic!("unexpected item {other:?}"),
            }
        }
        panic!("no END");
    }

    #[test]
    fn end_and_drop_send_the_open_gap_first() {
        let (rec, rx) = Recorder::pipeline(MIB);
        let full = rec.reserve(MIB);
        let mut h = open(&rec, StreamKind::HttpReq, 0);
        for _ in 0..3 {
            h.data(rec.now(), Bytes::from(vec![b'y'; 1000]));
        }
        h.end(rec.now(), EndReason::Complete, None);
        let mut d = open(&rec, StreamKind::HttpResp, 0);
        d.data(rec.now(), Bytes::from(vec![b'z'; 10]));
        drop(d);
        drop(full);
        let items = drain(&rec, &rx);

        let stream: Vec<&Item> = items.iter().filter(|i| sid_of(i) == 1).collect();
        assert_eq!(stream.len(), 3);
        assert!(matches!(stream[1], Item::Gap { .. }));
        assert_eq!(check_stream(&stream, 0), 3000);

        let dropped: Vec<&Item> = items.iter().filter(|i| sid_of(i) == 2).collect();
        assert!(matches!(dropped[1], Item::Gap { .. }));
        assert!(matches!(
            dropped[2],
            Item::End {
                reason: EndReason::Interrupted,
                ..
            }
        ));
        assert_eq!(check_stream(&dropped, 0), 10);
        assert_eq!(rec.live().losing.streams_in_gap, 0);
    }

    #[test]
    fn after_the_wire_stop_only_native_sends() {
        let (rec, rx) = Recorder::pipeline(MIB);
        let mut wire = open(&rec, StreamKind::HttpResp, 0);
        let mut native = open(&rec, StreamKind::Native, 0);
        drain(&rec, &rx);
        rec.stop_wire();

        wire.data(rec.now(), Bytes::from(vec![b'w'; 500]));
        native.data(rec.now(), Bytes::from(vec![b'n'; 300]));
        let late = rec.open(
            StreamKind::Conn,
            0,
            0,
            0,
            OpenFlags::NONE,
            json!({"host": "example.com"}),
        );
        let late_again = rec.open(
            StreamKind::Conn,
            0,
            0,
            0,
            OpenFlags::NONE,
            json!({"host": "example.com"}),
        );
        drop((late, late_again));
        wire.end(rec.now(), EndReason::Complete, None);
        let items = drain(&rec, &rx);

        assert!(items.iter().all(|i| sid_of(i) == native.sid()));
        assert!(matches!(items.as_slice(), [Item::Data { bytes, .. }] if bytes.len() == 300));
        // The late connections relayed nothing, so none is named.
        assert_eq!(rec.after_stop(), (500, Vec::new()));
    }

    /// Section 3: the guard weighs what a piece costs in memory, its queue slot and allocation
    /// as well as its bytes, so a stream of tiny pieces reaches the guard (and its GAP) long
    /// before its bytes alone would.
    #[test]
    fn tiny_pieces_reach_the_guard_by_their_cost() {
        let (rec, rx) = Recorder::pipeline(MIB);
        let mut h = open(&rec, StreamKind::WsDown, 0);
        let mut admitted = 0;
        while rec.live().losing.dropped_bytes == 0 {
            h.ws(rec.now(), 0x1, true, Bytes::from_static(b"x"));
            admitted += 1;
            assert!(admitted <= MIB, "the guard never dropped");
        }
        assert!(admitted < MIB / 64);
        drop(h);
        let items = drain(&rec, &rx);
        let stream: Vec<&Item> = items.iter().collect();
        assert_eq!(check_stream(&stream, 0), admitted);
        assert_eq!(rec.live().held_bytes, 0);
    }

    #[test]
    fn native_and_empty_pieces_never_drop() {
        let (rec, rx) = Recorder::pipeline(MIB);
        let full = rec.reserve(MIB);
        let mut native = open(&rec, StreamKind::Native, 0);
        let mut up = open(&rec, StreamKind::WsUp, 0);
        native.data(rec.now(), Bytes::from(vec![b'n'; 1000]));
        up.ws(rec.now(), 0x1, true, Bytes::new());
        drop(full);
        let items = drain(&rec, &rx);
        assert!(items.iter().all(|i| !matches!(i, Item::Gap { .. })));
        assert_eq!(rec.live().losing.dropped_bytes, 0);
        let data = |sid| {
            items
                .iter()
                .filter(|i| sid_of(i) == sid && matches!(i, Item::Data { .. }))
                .count()
        };
        assert_eq!((data(native.sid()), data(up.sid())), (1, 1));
    }

    /// Loss from before the seal's wire stop still gets its GAP: a guard gap closed by its
    /// handle after the stop, a caller's gap reported after it, and a guard gap whose handle is
    /// never called again, which the seal closes.
    #[test]
    fn loss_before_the_wire_stop_keeps_its_gap() {
        let (rec, rx) = Recorder::pipeline(MIB);
        let full = rec.reserve(MIB);
        let mut a = open(&rec, StreamKind::HttpResp, 0);
        let mut b = open(&rec, StreamKind::Conn, 0);
        let mut c = open(&rec, StreamKind::HttpResp, 0);
        a.data(rec.now(), Bytes::from(vec![b'a'; 1000]));
        c.data(rec.now(), Bytes::from(vec![b'c'; 400]));
        let start = rec.now();
        rec.stop_wire();

        a.data(rec.now(), Bytes::from(vec![b'a'; 50]));
        b.gap(GapReason::DecodeFailed, start, Some(500), None);
        a.end(rec.now(), EndReason::Complete, None);
        drop(b);
        rec.stop_all();
        rec.close_open_gaps(rec.now());
        drop((c, full));
        let items = drain(&rec, &rx);

        let gaps: Vec<(Sid, u64, Option<u64>)> = items
            .iter()
            .filter_map(|i| match i {
                Item::Gap {
                    sid,
                    off,
                    lost,
                    start,
                    end,
                    ..
                } => {
                    assert!(start < end);
                    Some((*sid, *off, *lost))
                }
                _ => None,
            })
            .collect();
        assert_eq!(gaps.len(), 3, "{items:?}");
        for want in [(1, 0, Some(1000)), (2, 0, Some(500)), (3, 0, Some(400))] {
            assert!(gaps.contains(&want), "{want:?} in {gaps:?}");
        }
        // The seal writes `END cut_at_seal` for streams still open; nothing else was sent.
        assert!(
            items
                .iter()
                .all(|i| matches!(i, Item::Open { .. } | Item::Gap { .. }))
        );
        assert_eq!(rec.after_stop().0, 50);
        assert_eq!(rec.live().losing.streams_in_gap, 0);
    }

    /// A stream opened after the wire stop has no OPEN, so it never gets a GAP either (PRD-01
    /// invariant 3): its known loss goes to the `after_stop` count, and its connection, with the
    /// host, to the `after_stop` connections. A connection that carried nothing after the stop
    /// is not one of them (PRD-02 section 11 step 2).
    #[test]
    fn a_stream_opened_after_the_stop_sends_nothing() {
        let (rec, rx) = Recorder::pipeline(MIB);
        let idle = rec.open(
            StreamKind::Conn,
            0,
            0,
            0,
            OpenFlags::NONE,
            json!({"host": "idle.example"}),
        );
        drain(&rec, &rx);
        rec.stop_wire();
        let mut conn = rec.open(
            StreamKind::Conn,
            0,
            0,
            0,
            OpenFlags::TUNNEL,
            json!({"host": "github.com"}),
        );
        conn.gap(GapReason::TlsRefused, rec.now(), None, None);
        let mut req = rec.open(
            StreamKind::HttpReq,
            conn.sid(),
            0,
            0,
            OpenFlags::NONE,
            json!({}),
        );
        req.gap(GapReason::DecodeFailed, rec.now(), Some(100), None);
        req.data(rec.now(), Bytes::from_static(b"more"));
        req.end(rec.now(), EndReason::Complete, None);
        let sid = conn.sid();
        drop((conn, idle));
        assert!(drain(&rec, &rx).is_empty());
        assert_eq!(
            rec.after_stop(),
            (104, vec![(sid, Some("github.com".into()))])
        );
    }

    /// Point 3 and no drop without a GAP: relay threads dropping under a full guard while the
    /// seal stops everything and closes the open gaps. Every byte lands in a GAP the seal or the
    /// handle sent, or in the `after_stop` count, and no stream is left in a gap.
    #[test]
    fn a_drop_racing_the_seal_is_in_a_gap_or_after_stop() {
        static PIECE64: [u8; 64] = [b'r'; 64];
        for _ in 0..300 {
            let (rec, rx) = Recorder::pipeline(MIB);
            let full = rec.reserve(MIB);
            let mut handles: Vec<StreamHandle> = (0..16)
                .map(|_| open(&rec, StreamKind::HttpResp, 0))
                .collect();
            let offered = AtomicU64::new(0);
            std::thread::scope(|s| {
                for h in handles.iter_mut() {
                    let (rec, offered) = (rec.clone(), &offered);
                    s.spawn(move || {
                        for _ in 0..2000 {
                            h.data(rec.now(), Bytes::from_static(&PIECE64));
                            offered.fetch_add(64, Ordering::Relaxed);
                        }
                    });
                }
                std::thread::yield_now();
                rec.stop_all();
                rec.close_open_gaps(rec.now());
            });
            drop((handles, full));
            let lost: u64 = drain(&rec, &rx)
                .iter()
                .filter_map(|i| match i {
                    Item::Gap { lost, .. } => *lost,
                    _ => None,
                })
                .sum();
            assert_eq!(lost + rec.after_stop().0, offered.into_inner());
            assert_eq!(rec.live().losing.streams_in_gap, 0);
        }
    }
}
