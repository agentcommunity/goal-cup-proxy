//! The one message from a `StreamHandle` or `Recorder::event` to the mask worker (PRD-02
//! sections 2 to 4). Times stay boot time; the writer maps them to wall time at write.

use super::{Mono, OpenFlags, Sid};
use bytes::Bytes;
use goalcup_format::{EndReason, GapReason, StreamKind};
use serde_json::Value;

/// What every item but DATA counts in `held` for its fixed fields; its JSON counts on top
/// ([`json_held`]), so control items fill `held` as they fill memory and the stall exit sees them.
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

/// What every DATA piece counts in `held` beyond its bytes: its queue slot and its `Bytes`
/// allocation, so many small pieces fill memory in `held` as they do in the process, and an
/// empty message end still counts in `queued`. `held` is therefore not payload bytes alone.
pub(crate) const DATA_ITEM_HELD: u64 = 128;

/// What a JSON value in a control item counts in `held`: 64 bytes for each value and each object
/// key (node and allocation) plus its string bytes, so an OPEN with many headers counts near what
/// it costs in memory and the stall mark bounds OPEN-heavy load too.
fn json_held(v: &Value) -> u64 {
    64 + match v {
        Value::String(s) => s.len() as u64,
        Value::Array(a) => a.iter().map(json_held).sum(),
        Value::Object(o) => o
            .iter()
            .map(|(k, v)| 64 + k.len() as u64 + json_held(v))
            .sum(),
        _ => 0,
    }
}

impl Item {
    /// What this item counts in `held` from the offer until the writer hands its record to the
    /// compressor: the mask worker takes and forwards (or drops) exactly this per item, and the
    /// writer releases it.
    pub(crate) fn held(&self) -> u64 {
        match self {
            Item::Data { bytes, .. } => bytes.len() as u64 + DATA_ITEM_HELD,
            Item::Open { meta: v, .. } | Item::Event { ev: v, .. } => CONTROL_HELD + json_held(v),
            Item::Gap { tail, .. } | Item::End { tail, .. } => {
                CONTROL_HELD + tail.as_ref().map_or(0, json_held)
            }
        }
    }
}
