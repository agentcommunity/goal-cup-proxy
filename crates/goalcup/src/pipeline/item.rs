//! The one message from a `StreamHandle` or `Recorder::event` to the mask worker (PRD-02
//! sections 2 to 4). Times stay boot time; the writer maps them to wall time at write.

use super::{Mono, OpenFlags, Sid};
use bytes::Bytes;
use goalcup_format::{EndReason, GapReason, StreamKind};
use serde_json::Value;

/// What every item but DATA counts in `held`: a fixed size for its small fields and JSON, so
/// control items fill memory only slowly and the stall exit can still see them.
pub(crate) const CONTROL_HELD: u64 = 256;

/// The WebSocket facts of one DATA piece: the message's opcode on every piece, `fin` on its last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ws {
    pub opcode: u8,
    pub fin: bool,
}

// The mask worker reads every field; it lands in the next slice.
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum Item {
    Open {
        sid: Sid,
        t: Mono,
        kind: StreamKind,
        conn: Sid,
        req: Sid,
        flags: OpenFlags,
        meta: Value,
    },
    /// At most 256 KiB; empty only for a WebSocket message end with nothing else in it.
    Data {
        sid: Sid,
        t: Mono,
        off: u64,
        ws: Option<Ws>,
        bytes: Bytes,
    },
    /// `lost` keeps the S1 meaning: `Some(n)` known bytes, `None` unknown. The record's `lost + 1`
    /// encoding happens when the record is built.
    Gap {
        sid: Sid,
        reason: GapReason,
        start: Mono,
        end: Mono,
        off: u64,
        lost: Option<u64>,
        tail: Option<Value>,
    },
    End {
        sid: Sid,
        t: Mono,
        reason: EndReason,
        seen: u64,
        tail: Option<Value>,
    },
    Event {
        t: Mono,
        ev: Value,
    },
}

/// What a DATA piece of `len` bytes counts in `held`: its length, and 1 for an empty message end,
/// so every item waiting counts in `queued`.
pub(crate) fn data_held(len: usize) -> u64 {
    (len as u64).max(1)
}

impl Item {
    /// What this item counts in `held` from the offer until the writer hands its record to the
    /// compressor: the mask worker takes and forwards (or drops) exactly this per item, and the
    /// writer releases it.
    pub(crate) fn held(&self) -> u64 {
        match self {
            Item::Data { bytes, .. } => data_held(bytes.len()),
            _ => CONTROL_HELD,
        }
    }
}
