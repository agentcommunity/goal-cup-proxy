//! The mask worker (PRD-02 section 4): one low-priority thread that owns every stream's
//! `StreamMasker`, masks each item off the relay and hands the records to the writer.
//!
//! Held accounting (the contract of the replan after core step 1): `taken` once an item is
//! masked (so an item the masker hangs on still waits, for the stall exit), and `forwarded`
//! before it, for what goes to the writer, so taken = forwarded + held back and an item waits
//! from before its first record is sent until the writer writes its last. Bytes the masker holds back for a match stay held until it releases them; at END and
//! GAP nothing stays held back. A masker panic drops the piece and the held bytes behind one
//! `mask_failed` GAP, and the stream goes on with a fresh masker. A panic on metadata stars every
//! string left in it and writes a `mask_failed` GAP that loses no stream byte.

// `run_recorder` starts it in the next slice; until then only `Recorder::real_for_tests` does.
#![cfg_attr(not(feature = "test-hooks"), allow(dead_code))]

use super::item::{Item, Ws};
use super::out::{Out, Source};
use super::{ExitOnPanic, Mono, Recorder, Sid, low_priority};
use crate::mask::{Masked, Masker, StreamMasker};
use goalcup_format::{EndReason, FLAG_FIN, GapReason, Record, StreamKind};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;
use std::thread::JoinHandle;

/// Records the channel to the writer holds. A full channel makes the worker wait, so `held`
/// grows until the guard drops at the front; that wait never reaches a relay.
pub(crate) const TO_WRITER: usize = 256;

/// The largest DATA record (PRD-01 section 3).
const MAX_DATA: usize = 256 << 10;

/// Test builds only: a DATA piece holding these bytes panics inside the per-item `catch_unwind`,
/// as a masker fault would (`tests/pipeline_mask_panic.rs`). No env var is read.
#[cfg(feature = "test-hooks")]
pub const PANIC_MARKER: &[u8] = b"\x01mask-worker-panic-marker\x02";

/// Test builds only: metadata whose JSON text holds this panics the same way.
#[cfg(feature = "test-hooks")]
const PANIC_TEXT: &str = "mask-worker-panic-marker";

/// Starts the mask worker. It runs until the item channel closes; a panic outside the per-item
/// `catch_unwind` ends the process with 70.
pub(crate) fn spawn(
    rec: Recorder,
    rx: mpsc::Receiver<Item>,
    masker: Masker,
    tx: mpsc::SyncSender<Out>,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("goalcup-mask".into())
        .spawn(move || {
            let _exit = ExitOnPanic("mask_worker_gone");
            low_priority();
            let mut w = Worker {
                rec,
                masker,
                tx,
                streams: HashMap::new(),
                outs: Vec::new(),
            };
            while let Ok(item) = rx.recv() {
                w.item(item);
            }
        })
        .expect("the mask worker starts")
}

struct Worker {
    rec: Recorder,
    masker: Masker,
    tx: mpsc::SyncSender<Out>,
    streams: HashMap<Sid, Stream>,
    /// The records of the item being handled.
    outs: Vec<Out>,
}

/// One open stream as the worker sees it. Offsets are source offsets: `out` is the next byte to
/// emit, `inp` the end of what was pushed, so `inp - out` bytes are held back by the masker.
struct Stream {
    kind: StreamKind,
    m: StreamMasker,
    out: u64,
    inp: u64,
    /// The end offset and time of every input piece not yet all emitted, oldest first.
    pieces: VecDeque<(u64, Mono)>,
    /// Held bytes taken and not yet forwarded.
    owed: u64,
    /// The opcode of the last WebSocket piece, for bytes the masker releases at a GAP or END.
    op: u8,
}

impl Stream {
    /// Forgets the input pieces that end at or before `off`.
    fn drop_to(&mut self, off: u64) {
        while self.pieces.front().is_some_and(|p| p.0 <= off) {
            self.pieces.pop_front();
        }
    }

    /// The time of the earliest input piece holding byte `off`.
    fn time_at(&mut self, off: u64, or: Mono) -> Mono {
        self.drop_to(off);
        self.pieces.front().map_or(or, |p| p.1)
    }
}

impl Worker {
    fn item(&mut self, item: Item) {
        let held = item.held();
        let sid = match &item {
            Item::Event { .. } => 0,
            Item::Open { sid, .. }
            | Item::Data { sid, .. }
            | Item::Gap { sid, .. }
            | Item::End { sid, .. } => *sid,
        };
        // Events and `sid 0` GAPs go through stream 0, which never holds a byte back.
        let end = matches!(item, Item::End { .. });
        let kind = match &item {
            Item::Open { kind, .. } => *kind,
            _ => StreamKind::Other(0),
        };
        stream(&mut self.streams, &self.masker, sid, kind).owed += held;
        match item {
            Item::Event { t, mut ev } => {
                let name = ev.get("ev").cloned();
                let note = name.as_ref().and_then(Value::as_str) == Some("note");
                let ok = self.mask(&mut ev);
                if let (false, Some(name), Some(m)) = (ok, name, ev.as_object_mut()) {
                    // The event's name is the recorder's own word, never a value.
                    m.insert("ev".into(), name);
                }
                let json = json_bytes(&ev);
                let mut o = Out::new(Record::Event { t: t.ns(), json }, 0);
                o.note = note;
                self.outs.push(o);
                if !ok {
                    self.meta_lost(0, t, 0, "event");
                }
            }
            Item::Open {
                t,
                kind,
                conn,
                req,
                flags,
                mut meta,
                ..
            } => {
                let mut host = (kind == StreamKind::Conn)
                    .then(|| meta.get("host").and_then(Value::as_str))
                    .flatten()
                    .map(|h| Value::String(h.to_string()));
                let source = Source::of(kind, &meta);
                let ok = host.as_mut().is_none_or(|h| self.mask(h)) & self.mask(&mut meta);
                let host = host.and_then(|h| h.as_str().map(String::from));
                stream(&mut self.streams, &self.masker, sid, kind).kind = kind;
                let r = Record::Open {
                    sid,
                    t: t.ns(),
                    skind: kind,
                    conn,
                    req,
                    meta: json_bytes(&meta),
                };
                let mut o = Out::new(r, 0);
                (o.flags, o.host, o.source) = (flags, host, source);
                self.outs.push(o);
                if !ok {
                    self.meta_lost(sid, t, 0, "open");
                }
            }
            Item::Data {
                t, off, ws, bytes, ..
            } => self.data(sid, t, off, ws, &bytes),
            Item::Gap {
                reason,
                start,
                end,
                off,
                lost,
                tail,
                ..
            } => {
                self.release(sid, start, |m| m.gap());
                let tail = self.tail(sid, end, off, tail);
                let r = Record::Gap {
                    sid,
                    reason,
                    start_wall: 0,
                    end_wall: 0,
                    start_mono: start.ns(),
                    end_mono: end.ns(),
                    off,
                    lost: lost.map_or(0, |n| n.saturating_add(1)),
                    tail,
                };
                self.outs.push(Out::new(r, 0));
                let s = self.stream(sid);
                (s.out, s.inp) = (off + lost.unwrap_or(0), off + lost.unwrap_or(0));
                s.pieces.clear();
            }
            Item::End {
                t,
                reason,
                seen,
                tail,
                ..
            } => {
                let kind = self.stream(sid).kind;
                let next = self.masker.stream(kind);
                self.release(sid, t, move |m| {
                    let mut out = match reason {
                        EndReason::Complete => Masked::default(),
                        _ => m.gap(),
                    };
                    join(&mut out, std::mem::replace(m, next).finish());
                    out
                });
                let s = self.stream(sid);
                let off = if s.kind == StreamKind::Conn { 0 } else { s.out };
                let (t, tail) = (t.ns(), self.tail(sid, t, off, tail));
                self.outs.push(Out::new(
                    Record::End {
                        sid,
                        t,
                        reason,
                        seen,
                        tail,
                    },
                    0,
                ));
            }
        }
        if self.outs.is_empty() {
            self.rec.taken(held);
        } else {
            self.forward(sid, held);
        }
        if end {
            self.streams.remove(&sid);
        }
    }

    /// One DATA piece: push, and on a WebSocket message end `boundary()`, then DATA and MASK
    /// records for what the masker released. `fin` goes on the last DATA of the message; an
    /// empty message gets one empty DATA with `fin`.
    fn data(&mut self, sid: Sid, t: Mono, off: u64, ws: Option<Ws>, bytes: &[u8]) {
        let s = stream(&mut self.streams, &self.masker, sid, StreamKind::Other(0));
        if let Some(w) = ws {
            s.op = w.opcode;
        }
        if s.inp == s.out {
            // Nothing held back: start where this piece starts (after a GAP moved the offset).
            (s.out, s.inp) = (off, off);
            s.pieces.clear();
        }
        s.pieces.push_back((s.inp + bytes.len() as u64, t));
        s.inp += bytes.len() as u64;
        let fin = ws.is_some_and(|w| w.fin);
        let r = catch_unwind(AssertUnwindSafe(|| {
            #[cfg(feature = "test-hooks")]
            if bytes.windows(PANIC_MARKER.len()).any(|w| w == PANIC_MARKER) {
                panic!("mask worker test marker");
            }
            let mut out = s.m.push(bytes);
            if fin {
                join(&mut out, s.m.boundary());
            }
            out
        }));
        match r {
            Ok(m) => {
                let opcode = ws.map_or(0, |w| w.opcode);
                let before = self.outs.len();
                emit(s, sid, m, (opcode, fin), t, &mut self.outs);
                if fin && self.outs.len() == before {
                    self.outs.push(Out::new(
                        data_rec(sid, t, s.out, (opcode << 4) | FLAG_FIN, Vec::new()),
                        0,
                    ));
                }
            }
            Err(_) => failed(s, sid, t, &self.rec, &self.masker, &mut self.outs),
        }
    }

    /// Runs `f` on the stream's masker inside `catch_unwind` and emits what it released, or the
    /// `mask_failed` GAP for what it held. Afterwards nothing is held back. Released bytes keep
    /// the opcode of the WebSocket message they belong to, without `fin`.
    fn release(&mut self, sid: Sid, t: Mono, f: impl FnOnce(&mut StreamMasker) -> Masked) {
        let s = stream(&mut self.streams, &self.masker, sid, StreamKind::Other(0));
        match catch_unwind(AssertUnwindSafe(|| f(&mut s.m))) {
            Ok(m) => emit(s, sid, m, (s.op, false), t, &mut self.outs),
            Err(_) => failed(s, sid, t, &self.rec, &self.masker, &mut self.outs),
        }
        if s.out != s.inp {
            // A masker that kept bytes past `gap()` or `finish()`: they are lost, never written.
            failed(s, sid, t, &self.rec, &self.masker, &mut self.outs);
        }
    }

    /// A GAP or END tail on `sid`, masked; empty for none. A masker panic on it writes the
    /// `mask_failed` GAP at `t` and `off` first.
    fn tail(&mut self, sid: Sid, t: Mono, off: u64, tail: Option<Value>) -> Vec<u8> {
        let Some(mut v) = tail else {
            return Vec::new();
        };
        if !self.mask(&mut v) {
            self.meta_lost(sid, t, off, "tail");
        }
        json_bytes(&v)
    }

    /// `mask_meta` inside `catch_unwind`. On a panic it stars every string left in `v`, as
    /// `fail_closed` does, so no raw value is written while keys and numbers (a tunnel's `up` and
    /// `down`) stay, and returns false: the caller then writes [`Worker::meta_lost`].
    fn mask(&self, v: &mut Value) -> bool {
        let ok = catch_unwind(AssertUnwindSafe(|| {
            #[cfg(feature = "test-hooks")]
            if v.to_string().contains(PANIC_TEXT) {
                panic!("mask worker test marker");
            }
            self.masker.mask_meta(v);
        }))
        .is_ok();
        if !ok {
            star(v);
        }
        ok
    }

    /// The `mask_failed` GAP of metadata the masker failed on (ledger R4): from `t` to now at the
    /// stream's `off`, no stream byte lost (`lost` 1), tail `{"meta": what}` naming the part.
    fn meta_lost(&mut self, sid: Sid, t: Mono, off: u64, what: &str) {
        let end = self.rec.now().max(Mono(t.ns() + 1));
        let r = Record::Gap {
            sid,
            reason: GapReason::MaskFailed,
            start_wall: 0,
            end_wall: 0,
            start_mono: t.ns(),
            end_mono: end.ns(),
            off,
            lost: 1,
            tail: json_bytes(&json!({ "meta": what })),
        };
        self.outs.push(Out::new(r, 0));
    }

    fn stream(&mut self, sid: Sid) -> &mut Stream {
        stream(&mut self.streams, &self.masker, sid, StreamKind::Other(0))
    }

    /// Moves the stream's owed held bytes, all but those still held back, onto the last record of
    /// this item and counts them as waiting for the writer, then marks the item's `held` taken
    /// (counted as waiting until here, so a masker that hangs on an item trips the stall exit)
    /// and sends the records. The carry waits from before the first record is sent until the
    /// writer releases it at the last, so a writer held while the worker waits between two
    /// records is still a stall.
    fn forward(&mut self, sid: Sid, held: u64) {
        let s = self.stream(sid);
        let carry = s.owed.saturating_sub(s.inp.saturating_sub(s.out));
        s.owed -= carry;
        self.rec.forwarded(carry);
        self.rec.taken(held);
        if let Some(o) = self.outs.last_mut() {
            o.held += carry;
        }
        self.send();
    }

    /// Hands this item's records to the writer, waiting while its channel is full. A gone writer
    /// ends the process with 70.
    fn send(&mut self) {
        for o in self.outs.drain(..) {
            if self.tx.send(o).is_err() {
                super::pipeline_gone();
            }
        }
    }
}

/// The stream's state, made at its first item (its OPEN, of `kind`); a stream with no OPEN seen
/// gets one too, so its bytes are still masked.
fn stream<'a>(
    streams: &'a mut HashMap<Sid, Stream>,
    masker: &Masker,
    sid: Sid,
    kind: StreamKind,
) -> &'a mut Stream {
    streams.entry(sid).or_insert_with(|| Stream {
        kind,
        m: masker.stream(kind),
        out: 0,
        inp: 0,
        pieces: VecDeque::new(),
        owed: 0,
        op: 0,
    })
}

/// DATA records of at most 256 KiB for `m`, each followed by one MASK per span it touches, from
/// the stream's next offset. Each DATA takes the time of the earliest input piece it holds (`t`
/// when none is known). `ws` is the WebSocket opcode and whether the message ends here.
fn emit(s: &mut Stream, sid: Sid, m: Masked, ws: (u8, bool), t: Mono, outs: &mut Vec<Out>) {
    let (opcode, fin) = ws;
    let Masked { mut bytes, spans } = m;
    let (base, n) = (s.out, bytes.len());
    let mut at = 0;
    while at < n {
        let k = MAX_DATA.min(n - at);
        let off = base + at as u64;
        let t = s.time_at(off, t);
        let flags = (opcode << 4) | (if fin && at + k == n { FLAG_FIN } else { 0 });
        // One record, the common case, takes the masker's buffer instead of a copy.
        let chunk = if k == n {
            std::mem::take(&mut bytes)
        } else {
            bytes[at..at + k].to_vec()
        };
        outs.push(Out::new(data_rec(sid, t, off, flags, chunk), 0));
        for sp in &spans {
            let (lo, hi) = (sp.start.max(at), (sp.start + sp.len).min(at + k));
            if lo < hi {
                let rec = Record::Mask {
                    sid,
                    off: base + lo as u64,
                    len: (hi - lo) as u64,
                    rule: sp.rule as u64,
                };
                outs.push(Out::new(rec, 0));
            }
        }
        at += k;
    }
    s.out = base + n as u64;
    s.drop_to(s.out);
}

/// A masker panic: the piece and the held bytes are dropped behind one `mask_failed` GAP from
/// the earliest held byte's time to now, at the first byte not emitted, and the stream gets a
/// fresh masker. DATA and GAP stay exactly contiguous.
fn failed(s: &mut Stream, sid: Sid, t: Mono, rec: &Recorder, masker: &Masker, outs: &mut Vec<Out>) {
    let start = s.time_at(s.out, t);
    let end = rec.now().max(Mono(start.ns() + 1));
    let gap = Record::Gap {
        sid,
        reason: GapReason::MaskFailed,
        start_wall: 0,
        end_wall: 0,
        start_mono: start.ns(),
        end_mono: end.ns(),
        off: s.out,
        lost: s.inp - s.out + 1,
        tail: Vec::new(),
    };
    outs.push(Out::new(gap, 0));
    s.out = s.inp;
    s.pieces.clear();
    s.m = masker.stream(s.kind);
}

fn data_rec(sid: Sid, t: Mono, off: u64, flags: u8, bytes: Vec<u8>) -> Record {
    Record::Data {
        sid,
        t: t.ns(),
        off,
        flags,
        bytes,
    }
}

/// Appends `b` to `a`, its spans moved to stay relative to `a`'s bytes.
fn join(a: &mut Masked, b: Masked) {
    let n = a.bytes.len();
    a.spans.extend(b.spans.into_iter().map(|mut sp| {
        sp.start += n;
        sp
    }));
    a.bytes.extend(b.bytes);
}

/// Stars every string leaf of `v`, same length; keys, numbers, bools and null stay.
fn star(v: &mut Value) {
    let mut stack = vec![v];
    while let Some(v) = stack.pop() {
        match v {
            Value::String(s) => *s = "*".repeat(s.len()),
            Value::Array(a) => stack.extend(a.iter_mut()),
            Value::Object(o) => stack.extend(o.values_mut()),
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
}

fn json_bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap_or_else(|_| b"{}".to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{OpenFlags, clock};
    use std::time::Duration;

    /// Point 3 (ledger R7): a writer held while the worker waits to send the second record of an
    /// item (a DATA with two masked spans, a channel of one) is still seen as a stall.
    #[test]
    fn a_worker_waiting_between_records_still_shows_a_stall() {
        let (rec, rx) = Recorder::pipeline(1 << 30);
        let (tx, out) = mpsc::sync_channel(1);
        spawn(rec.clone(), rx, Masker::new(&[], Vec::new()).unwrap(), tx);
        let mut h = rec.open(StreamKind::HttpResp, 0, 0, 0, OpenFlags::NONE, json!({}));
        let open = out.recv_timeout(Duration::from_secs(30)).unwrap();
        rec.release(open.held);
        let key = format!("Bearer sk-{}{}", "x".repeat(20), "7".repeat(20));
        let body = format!("a {key} b {key} and plain words after the two keys.");
        h.data(rec.now(), bytes::Bytes::from(body.into_bytes()));
        // The DATA fills the channel and the worker waits on its first MASK; the writer is held.
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(50));
            let idle = rec.live_at(clock::awake_ns() + 61_000_000_000).writer_idle_ms;
            assert!(idle > 0, "a held writer must show as idle");
        }
        h.end(rec.now(), EndReason::Complete, None);
        let (mut masks, mut n) = (0, 0);
        while rec.live().held_bytes > 0 && n < 100 {
            let o = out.recv_timeout(Duration::from_secs(30)).unwrap();
            masks += usize::from(matches!(o.rec, Record::Mask { .. }));
            rec.release(o.held);
            n += 1;
        }
        assert_eq!(masks, 2, "the piece had two masked spans");
    }
}
