//! `StreamHandle` (seam S1): one stream's calls. The handle owns the stream's offset and its
//! guard gap (PRD-02 section 3), so the gap rule needs no shared map.

#[cfg(feature = "test-hooks")]
use super::Call;
use super::guard;
use super::item::{Item, Ws};
use super::{Mono, Recorder, Sid};
use bytes::Bytes;
use goalcup_format::{EndReason, GapReason, StreamKind};
use serde_json::{Value, json};
use std::ops::BitOr;
use std::sync::atomic::Ordering;

/// The largest DATA piece (PRD-01 section 3): bigger offers are split, with no copy.
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
    off: u64,
    /// The guard gap open on this stream, if any.
    gap: Option<GuardGap>,
    ended: bool,
}

/// Bytes the guard dropped on one stream since its last admitted piece.
struct GuardGap {
    /// The first dropped piece's time.
    start: Mono,
    /// The offset of its first byte.
    off: u64,
    lost: u64,
}

impl StreamHandle {
    pub(super) fn new(rec: Recorder, sid: Sid, kind: StreamKind, from: u64) -> StreamHandle {
        let wire = matches!(
            kind,
            StreamKind::Conn
                | StreamKind::HttpReq
                | StreamKind::HttpResp
                | StreamKind::WsUp
                | StreamKind::WsDown
        );
        StreamHandle {
            rec,
            sid,
            wire,
            off: from,
            gap: None,
            ended: false,
        }
    }

    pub fn sid(&self) -> Sid {
        self.sid
    }

    /// After the seal's stop a call on this stream only counts (PRD-02 section 11 step 2).
    pub(super) fn stopped(&self) -> bool {
        let i = &self.rec.inner;
        i.all_stopped.load(Ordering::Acquire) || (self.wire && i.wire_stopped.load(Ordering::Acquire))
    }

    /// One piece; moves the offset by its length, written or dropped.
    pub fn data(&mut self, t: Mono, bytes: Bytes) {
        collect!(self.rec, Call::Data(self.sid, t, self.off, bytes.clone()));
        self.offer(t, None, bytes);
    }

    /// One WebSocket piece; `fin` marks a message end.
    pub fn ws(&mut self, t: Mono, opcode: u8, fin: bool, bytes: Bytes) {
        collect!(
            self.rec,
            Call::Ws(self.sid, t, self.off, opcode, fin, bytes.clone())
        );
        self.offer(t, Some(Ws { opcode, fin }), bytes);
    }

    /// The caller's own loss, reported when it ends: `off` is the handle's offset, the end is now.
    /// `Some(n)` moves the offset by `n`; `None` means unknown and leaves it. An open guard gap
    /// is sent first.
    pub fn gap(&mut self, reason: GapReason, start: Mono, lost: Option<u64>, tail: Option<Value>) {
        let end = self.rec.now();
        collect!(
            self.rec,
            Call::Gap(self.sid, reason, start, end, self.off, lost, tail.clone())
        );
        if !self.stopped() {
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

    fn finish(&mut self, t: Mono, reason: EndReason, tail: Option<Value>) {
        self.ended = true;
        collect!(
            self.rec,
            Call::End(self.sid, t, reason, self.off, tail.clone())
        );
        if self.stopped() {
            // The seal writes what is still open on this stream; only the live count goes.
            if self.gap.take().is_some() {
                self.rec.inner.streams_in_gap.fetch_sub(1, Ordering::Relaxed);
            }
            return;
        }
        self.close_gap(t);
        self.rec.send(Item::End {
            sid: self.sid,
            t,
            reason,
            seen: self.off,
            tail,
        });
    }

    /// Splits an offer into pieces of at most 256 KiB (a WebSocket message keeps its opcode on
    /// every piece and `fin` on the last) and runs each through the guard.
    fn offer(&mut self, t: Mono, ws: Option<Ws>, bytes: Bytes) {
        if self.stopped() {
            let n = bytes.len() as u64;
            let i = &self.rec.inner;
            i.after_stop_bytes.fetch_add(n, Ordering::Relaxed);
            self.off += n;
            return;
        }
        if bytes.is_empty() {
            // Only a message end has anything to say with no bytes.
            if ws.is_some_and(|w| w.fin) {
                self.piece(t, ws, bytes);
            }
            return;
        }
        let len = bytes.len();
        let mut at = 0;
        while at < len {
            let n = MAX_PIECE.min(len - at);
            let last = at + n == len;
            let ws = ws.map(|w| Ws {
                opcode: w.opcode,
                fin: w.fin && last,
            });
            self.piece(t, ws, bytes.slice(at..at + n));
            at += n;
        }
    }

    /// The gap rule (PRD-02 section 3) for one piece of at most 256 KiB.
    fn piece(&mut self, t: Mono, ws: Option<Ws>, bytes: Bytes) {
        let n = bytes.len() as u64;
        let i = &self.rec.inner;
        let held = i.held.load(Ordering::Relaxed);
        if guard::admits(held, n, self.gap.is_some(), i.guard) {
            self.close_gap(t);
            self.rec.send(Item::Data {
                sid: self.sid,
                t,
                off: self.off,
                ws,
                bytes,
            });
        } else {
            match &mut self.gap {
                Some(gap) => gap.lost += n,
                None => {
                    self.gap = Some(GuardGap {
                        start: t,
                        off: self.off,
                        lost: n,
                    });
                    i.streams_in_gap.fetch_add(1, Ordering::Relaxed);
                }
            }
            i.dropped_bytes.fetch_add(n, Ordering::Relaxed);
        }
        self.off += n;
    }

    /// Sends the open guard gap, if any, ending at `end` (at least 1 ns after its start, PRD-01
    /// invariant 12).
    fn close_gap(&mut self, end: Mono) {
        let Some(gap) = self.gap.take() else {
            return;
        };
        let limit = self.rec.inner.guard;
        self.rec.send(Item::Gap {
            sid: self.sid,
            reason: GapReason::MemoryGuard,
            start: gap.start,
            end: end.max(Mono(gap.start.ns() + 1)),
            off: gap.off,
            lost: Some(gap.lost),
            tail: Some(json!({"guard": "held_bytes", "limit": limit})),
        });
        self.rec.inner.streams_in_gap.fetch_sub(1, Ordering::Relaxed);
    }
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
    use std::collections::BTreeMap;
    use std::sync::mpsc::Receiver;

    const MIB: u64 = 1 << 20;
    const PIECE: usize = 64 << 10;

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
            Item::Event { .. } => 0,
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
    fn guard_drops_never_wait_and_gaps_cover_every_hole() {
        let (rec, rx) = Recorder::pipeline(MIB);
        let payload = Bytes::from(vec![b'x'; PIECE]);
        let mut handles: Vec<StreamHandle> = (0..40)
            .map(|_| open(&rec, StreamKind::HttpReq, 0))
            .collect();
        let mut offered = vec![0u64; handles.len()];

        // Nobody drains: every offer still returns.
        std::thread::scope(|s| {
            for h in handles.iter_mut() {
                let payload = payload.clone();
                let rec = rec.clone();
                s.spawn(move || {
                    for _ in 0..64 {
                        h.data(rec.now(), payload.clone());
                    }
                });
            }
        });
        for n in offered.iter_mut() {
            *n += 64 * PIECE as u64;
        }
        let live = rec.live();
        assert!(live.losing.dropped_bytes > 0);
        assert!(live.losing.streams_in_gap > 0);

        // Drain and offer again until every stream has recovered.
        let mut items = drain(&rec, &rx);
        for _ in 0..1000 {
            if handles.iter().all(|h| h.gap.is_none()) {
                break;
            }
            for (h, n) in handles.iter_mut().zip(offered.iter_mut()) {
                if h.gap.is_some() {
                    h.data(rec.now(), payload.clone());
                    *n += PIECE as u64;
                }
            }
            items.extend(drain(&rec, &rx));
        }
        assert!(handles.iter().all(|h| h.gap.is_none()));
        assert_eq!(rec.live().losing.streams_in_gap, 0);
        for h in handles {
            let t = rec.now();
            h.end(t, EndReason::Complete, None);
        }
        items.extend(drain(&rec, &rx));

        let mut by_sid: BTreeMap<Sid, Vec<&Item>> = BTreeMap::new();
        for item in &items {
            by_sid.entry(sid_of(item)).or_default().push(item);
        }
        assert_eq!(by_sid.len(), offered.len());
        for (stream, n) in by_sid.values().zip(&offered) {
            assert_eq!(check_stream(stream, 0), *n);
        }
    }

    #[test]
    fn end_and_drop_send_the_open_gap_first() {
        let (rec, rx) = Recorder::pipeline(MIB);
        let full = rec.reserve(MIB);
        let mut h = open(&rec, StreamKind::Native, 100);
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
        assert_eq!(check_stream(&stream, 100), 3100);

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
    fn big_pieces_split_and_keep_the_message_end() {
        let (rec, rx) = Recorder::pipeline(MIB);
        let mut h = open(&rec, StreamKind::WsUp, 0);
        h.ws(rec.now(), 0x1, true, Bytes::from(vec![b'm'; 600 << 10]));
        h.ws(rec.now(), 0x2, true, Bytes::new());
        h.end(rec.now(), EndReason::Complete, None);
        let items = drain(&rec, &rx);

        let data: Vec<(u64, usize, Option<Ws>)> = items
            .iter()
            .filter_map(|i| match i {
                Item::Data { off, ws, bytes, .. } => Some((*off, bytes.len(), *ws)),
                _ => None,
            })
            .collect();
        assert_eq!(data.len(), 4);
        let mut at = 0;
        for (i, (off, len, ws)) in data[..3].iter().enumerate() {
            assert_eq!(*off, at);
            assert!(*len <= MAX_PIECE);
            assert_eq!(
                *ws,
                Some(Ws {
                    opcode: 0x1,
                    fin: i == 2
                })
            );
            at += *len as u64;
        }
        assert_eq!(at, 600 << 10);
        assert_eq!(
            data[3],
            (
                at,
                0,
                Some(Ws {
                    opcode: 0x2,
                    fin: true
                })
            )
        );
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
        assert_eq!(rec.after_stop(), (500, vec!["example.com".to_string()]));
    }
}
