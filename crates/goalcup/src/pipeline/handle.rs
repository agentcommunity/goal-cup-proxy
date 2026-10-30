//! `StreamHandle` (seam S1): one stream's calls. The handle owns the stream's offset.
// The scaffold's calls only reach the `test-hooks` collector until the core slice.
#![cfg_attr(not(feature = "test-hooks"), allow(unused_variables))]

#[cfg(feature = "test-hooks")]
use super::Call;
use super::{Mono, Recorder, Sid};
use bytes::Bytes;
use goalcup_format::{EndReason, GapReason};
use serde_json::Value;
use std::ops::BitOr;

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
    off: u64,
    ended: bool,
}

impl StreamHandle {
    pub(super) fn new(rec: Recorder, sid: Sid, from: u64) -> StreamHandle {
        StreamHandle {
            rec,
            sid,
            off: from,
            ended: false,
        }
    }

    pub fn sid(&self) -> Sid {
        self.sid
    }

    /// One piece; moves the offset by its length, written or dropped.
    pub fn data(&mut self, t: Mono, bytes: Bytes) {
        let n = bytes.len() as u64;
        collect!(self.rec, Call::Data(self.sid, t, self.off, bytes));
        self.off += n;
    }

    /// One WebSocket piece; `fin` marks a message end.
    pub fn ws(&mut self, t: Mono, opcode: u8, fin: bool, bytes: Bytes) {
        let n = bytes.len() as u64;
        collect!(
            self.rec,
            Call::Ws(self.sid, t, self.off, opcode, fin, bytes)
        );
        self.off += n;
    }

    /// The caller's own loss, reported when it ends: `off` is the handle's offset, the end is now.
    /// `Some(n)` moves the offset by `n`; `None` means unknown and leaves it.
    pub fn gap(&mut self, reason: GapReason, start: Mono, lost: Option<u64>, tail: Option<Value>) {
        let end = self.rec.now();
        collect!(
            self.rec,
            Call::Gap(self.sid, reason, start, end, self.off, lost, tail)
        );
        self.off += lost.unwrap_or(0);
    }

    pub fn end(mut self, t: Mono, reason: EndReason, tail: Option<Value>) {
        self.finish(t, reason, tail);
    }

    fn finish(&mut self, t: Mono, reason: EndReason, tail: Option<Value>) {
        self.ended = true;
        collect!(self.rec, Call::End(self.sid, t, reason, self.off, tail));
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
