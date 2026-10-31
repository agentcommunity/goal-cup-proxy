//! The one message from the mask worker to the writer (PRD-02 sections 4 and 5).

use super::OpenFlags;
use super::cursors::Fid;
use super::item::Mark;
use goalcup_format::{Record, StreamKind};
use serde_json::Value;

/// What the writer's channel carries: a masked record, or a seal marker, which counts
/// `CONTROL_HELD` and is released like a control item.
pub(crate) enum ToWriter {
    Rec(Out),
    Mark(Mark),
}

/// One masked record. Times are on the record clock (Mono ns); a GAP's `start_wall` and
/// `end_wall` are left 0 and the writer fills them at write. `held` is what the writer releases
/// once the record is written: per item, the `held` of its records add up to exactly what the
/// mask worker forwarded. The other fields carry, for OPEN only, what the writer may not read
/// back from masked meta (PRD-05 amendment).
pub(crate) struct Out {
    pub rec: Record,
    pub held: u64,
    pub flags: OpenFlags,
    /// A `conn` stream's host, taken before `mask_meta` and masked on its own.
    pub host: Option<String>,
    pub source: Option<Source>,
    /// A `note` event, known before its body was masked.
    pub note: bool,
}

impl Out {
    pub fn new(rec: Record, held: u64) -> Out {
        Out {
            rec,
            held,
            flags: OpenFlags::NONE,
            host: None,
            source: None,
            note: false,
        }
    }
}

/// A session file's cursor facts, from the raw OPEN meta `native/tail.rs` and
/// `native/snapshot.rs` write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Native {
        path: String,
        generation: u64,
        fid: Fid,
        from: u64,
    },
    Snapshot {
        path: String,
        size: u64,
        mtime_ns: i64,
    },
}

impl Source {
    /// The facts of a native or snapshot OPEN; None for other kinds or a meta without them.
    pub fn of(kind: StreamKind, meta: &Value) -> Option<Source> {
        let path = meta.get("path")?.as_str()?.to_string();
        match kind {
            StreamKind::Native => Some(Source::Native {
                path,
                generation: meta.get("gen")?.as_u64()?,
                fid: fid(meta.get("fid")?.as_str()?)?,
                from: meta.get("from").and_then(Value::as_u64).unwrap_or(0),
            }),
            StreamKind::Snapshot => Some(Source::Snapshot {
                path,
                size: meta.get("size")?.as_u64()?,
                mtime_ns: meta.get("mtime_ns")?.as_i64()?,
            }),
            _ => None,
        }
    }
}

/// OPEN meta `fid` back to a `Fid`: `<dev>.<ino>[.<birth>]` in hex, as `native::tail::fid_hex`
/// writes it (a negative birth time is its two's complement).
fn fid(text: &str) -> Option<Fid> {
    let mut it = text.split('.').map(|p| u64::from_str_radix(p, 16));
    let (dev, ino) = (it.next()?.ok()?, it.next()?.ok()?);
    let birth_ns = match it.next() {
        Some(b) => Some(b.ok()? as i64),
        None => None,
    };
    it.next().is_none().then_some(Fid { dev, ino, birth_ns })
}
