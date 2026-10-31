//! The mask worker (PRD-02 section 4): one low-priority thread that owns every stream's
//! `StreamMasker`, masks each item off the relay and hands the records to the writer.
//!
//! Held accounting (the contract of the replan after core step 1): `taken` when an item comes
//! off the item channel, `forwarded` for what goes to the writer, so taken = forwarded + held
//! back. Bytes the masker holds back for a match stay held until it releases them; at END and
//! GAP nothing stays held back. A masker panic drops the piece and the held bytes behind one
//! `mask_failed` GAP, and the stream goes on with a fresh masker.

// `run_recorder` starts it in the next slice; until then only `Recorder::real_for_tests` does.
#![cfg_attr(not(feature = "test-hooks"), allow(dead_code))]

use super::item::{Item, Ws};
use super::out::{Out, Source};
use super::{ExitOnPanic, Mono, Recorder, Sid, low_priority};
use crate::mask::{Masked, Masker, StreamMasker};
use goalcup_format::{EndReason, FLAG_FIN, GapReason, Record, StreamKind};
use serde_json::Value;
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
}

impl Stream {
    /// The time of the earliest input piece holding byte `off`.
    fn time_at(&mut self, off: u64, or: Mono) -> Mono {
        while self.pieces.front().is_some_and(|p| p.0 <= off) {
            self.pieces.pop_front();
        }
        self.pieces.front().map_or(or, |p| p.1)
    }
}

impl Worker {
    fn item(&mut self, item: Item) {
        let held = item.held();
        self.rec.taken(held);
        let sid = match &item {
            Item::Event { .. } => 0,
            Item::Open { sid, .. }
            | Item::Data { sid, .. }
            | Item::Gap { sid, .. }
            | Item::End { sid, .. } => *sid,
        };
        // Events and `sid 0` GAPs go through stream 0, which never holds a byte back.
        let end = matches!(item, Item::End { .. });
        self.stream(sid).owed += held;
        match item {
            Item::Event { t, mut ev } => {
                let note = ev.get("ev").and_then(Value::as_str) == Some("note");
                mask_value(&self.masker, &mut ev);
                let json = json_bytes(&ev);
                let mut o = Out::new(Record::Event { t: t.ns(), json }, 0);
                o.note = note;
                self.outs.push(o);
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
                let host = (kind == StreamKind::Conn)
                    .then(|| meta.get("host").and_then(Value::as_str))
                    .flatten()
                    .and_then(|h| {
                        let mut v = Value::String(h.to_string());
                        mask_value(&self.masker, &mut v);
                        v.as_str().map(String::from)
                    });
                let source = Source::of(kind, &meta);
                mask_value(&self.masker, &mut meta);
                let m = self.masker.stream(kind);
                let s = self.stream(sid);
                (s.kind, s.m) = (kind, m);
                let (t, skind, meta) = (t.ns(), kind, json_bytes(&meta));
                let r = Record::Open { sid, t, skind, conn, req, meta };
                let mut o = Out::new(r, 0);
                (o.flags, o.host, o.source) = (flags, host, source);
                self.outs.push(o);
            }
            Item::Data { t, off, ws, bytes, .. } => self.data(sid, t, off, ws, &bytes),
            Item::Gap { reason, start, end, off, lost, tail, .. } => {
                self.release(sid, start, |m| m.gap());
                let r = Record::Gap {
                    sid,
                    reason,
                    start_wall: 0,
                    end_wall: 0,
                    start_mono: start.ns(),
                    end_mono: end.ns(),
                    off,
                    lost: lost.map_or(0, |n| n.saturating_add(1)),
                    tail: self.tail(tail),
                };
                self.outs.push(Out::new(r, 0));
                let s = self.stream(sid);
                (s.out, s.inp) = (off + lost.unwrap_or(0), off + lost.unwrap_or(0));
                s.pieces.clear();
            }
            Item::End { t, reason, seen, tail, .. } => {
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
                let (t, tail) = (t.ns(), self.tail(tail));
                self.outs.push(Out::new(Record::End { sid, t, reason, seen, tail }, 0));
            }
        }
        if !self.outs.is_empty() {
            self.forward(sid);
        }
        if end {
            self.streams.remove(&sid);
        }
    }

    /// One DATA piece: push, and on a WebSocket message end `boundary()`, then DATA and MASK
    /// records for what the masker released. `fin` goes on the last DATA of the message; an
    /// empty message gets one empty DATA with `fin`.
    fn data(&mut self, sid: Sid, t: Mono, off: u64, ws: Option<Ws>, bytes: &[u8]) {
        let s = stream(&mut self.streams, &self.masker, sid);
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
            if bytes
                .windows(PANIC_MARKER.len())
                .any(|w| w == PANIC_MARKER)
            {
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
                emit(s, sid, &m, (opcode, fin), t, &mut self.outs);
                if fin && self.outs.len() == before {
                    self.outs.push(Out::new(data_rec(sid, t, s.out, (opcode << 4) | FLAG_FIN, &[]), 0));
                }
            }
            Err(_) => failed(s, sid, t, &self.rec, &self.masker, &mut self.outs),
        }
    }

    /// Runs `f` on the stream's masker inside `catch_unwind` and emits what it released, or the
    /// `mask_failed` GAP for what it held. Afterwards nothing is held back.
    fn release(&mut self, sid: Sid, t: Mono, f: impl FnOnce(&mut StreamMasker) -> Masked) {
        let s = stream(&mut self.streams, &self.masker, sid);
        match catch_unwind(AssertUnwindSafe(|| f(&mut s.m))) {
            Ok(m) => emit(s, sid, &m, (0, false), t, &mut self.outs),
            Err(_) => failed(s, sid, t, &self.rec, &self.masker, &mut self.outs),
        }
        if s.out != s.inp {
            // A masker that kept bytes past `gap()` or `finish()`: they are lost, never written.
            failed(s, sid, t, &self.rec, &self.masker, &mut self.outs);
        }
    }

    /// A GAP or END tail, masked; empty for none.
    fn tail(&self, tail: Option<Value>) -> Vec<u8> {
        tail.map_or_else(Vec::new, |mut v| {
            mask_value(&self.masker, &mut v);
            json_bytes(&v)
        })
    }

    fn stream(&mut self, sid: Sid) -> &mut Stream {
        stream(&mut self.streams, &self.masker, sid)
    }

    /// Moves the stream's owed held bytes, all but those still held back, onto the last record of
    /// this item, then sends them all.
    fn forward(&mut self, sid: Sid) {
        let s = self.stream(sid);
        let carry = s.owed - (s.inp - s.out).min(s.owed);
        s.owed -= carry;
        if let Some(o) = self.outs.last_mut() {
            o.held += carry;
        }
        self.send();
    }

    /// Hands this item's records to the writer, waiting while its channel is full. A gone writer
    /// ends the process with 70.
    fn send(&mut self) {
        for o in self.outs.drain(..) {
            self.rec.forwarded(o.held);
            if self.tx.send(o).is_err() {
                super::pipeline_gone();
            }
        }
    }
}

/// The stream's state, made at its OPEN; a stream with no OPEN seen gets one too, so its bytes
/// are still masked.
fn stream<'a>(streams: &'a mut HashMap<Sid, Stream>, masker: &Masker, sid: Sid) -> &'a mut Stream {
    streams.entry(sid).or_insert_with(|| Stream {
        kind: StreamKind::Other(0),
        m: masker.stream(StreamKind::Other(0)),
        out: 0,
        inp: 0,
        pieces: VecDeque::new(),
        owed: 0,
    })
}

/// DATA records of at most 256 KiB for `m`, each followed by one MASK per span it touches, from
/// the stream's next offset. Each DATA takes the time of the earliest input piece it holds (`t`
/// when none is known). `ws` is the WebSocket opcode and whether the message ends here.
fn emit(s: &mut Stream, sid: Sid, m: &Masked, ws: (u8, bool), t: Mono, outs: &mut Vec<Out>) {
    let (opcode, fin) = ws;
    let base = s.out;
    let mut at = 0;
    while at < m.bytes.len() {
        let k = MAX_DATA.min(m.bytes.len() - at);
        let last = at + k == m.bytes.len();
        let off = base + at as u64;
        let t = s.time_at(off, t);
        let flags = (opcode << 4) | (if fin && last { FLAG_FIN } else { 0 });
        outs.push(Out::new(data_rec(sid, t, off, flags, &m.bytes[at..at + k]), 0));
        for sp in &m.spans {
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
    s.out = base + m.bytes.len() as u64;
    s.time_at(s.out, t);
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

fn data_rec(sid: Sid, t: Mono, off: u64, flags: u8, bytes: &[u8]) -> Record {
    Record::Data {
        sid,
        t: t.ns(),
        off,
        flags,
        bytes: bytes.to_vec(),
    }
}

/// Appends `b` to `a`, its spans moved to stay relative to `a`'s bytes.
fn join(a: &mut Masked, b: Masked) {
    let n = a.bytes.len();
    a.spans
        .extend(b.spans.into_iter().map(|mut sp| {
            sp.start += n;
            sp
        }));
    a.bytes.extend(b.bytes);
}

/// `mask_meta` inside `catch_unwind`: a panic leaves an empty object, never the raw value.
fn mask_value(masker: &Masker, v: &mut Value) {
    if catch_unwind(AssertUnwindSafe(|| masker.mask_meta(v))).is_err() {
        *v = Value::Object(serde_json::Map::new());
    }
}

fn json_bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap_or_else(|_| b"{}".to_vec())
}
