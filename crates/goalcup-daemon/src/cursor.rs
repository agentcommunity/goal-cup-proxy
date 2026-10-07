//! Where a tailer got to in a store, and what its number counts (D19).
//!
//! Five cursor types, one per `manifest::CursorType`, plus the directory tailer's
//! realisation of `inode_offset` as a seen-set. Each carries a `records` counter beside
//! whatever key it actually reads with, and [`CursorState::position`] reports **that
//! counter**, under [`CursorUnit::Records`].
//!
//! That is the whole point of the module, and it is worth stating why the obvious design
//! is wrong. The obvious design reports what the cursor *is*: `Bytes` for an offset,
//! `Time` for a directory tailer's mtime high-water mark, `Opaque` for a rowid. Under it
//! `StoreCursor::records()` is `None` on every store the shipped connectors declare — the
//! verifier compares cursor movement with a count of records only under `Records` — and
//! `gaps.rs`'s `cursor_mismatch`, the Fail that catches records in the chain the daemon
//! never accounted for, can fire on nothing. So the number a cursor reports is the count of
//! records the tailer has emitted, and the byte offset, the rowid, the ordinal and the
//! mtime survive in [`CursorState::describe`], which is rendered for a reader and parsed by
//! nobody.
//!
//! Nothing here interprets a rule. A cursor is a position and a count; what a moved cursor
//! *means* is `goalcup-verifier`'s.

use crate::DaemonError;
use goalcup_protocol::cursor::{CursorUnit, StoreCursor};
use goalcup_protocol::manifest::{ConnectorManifest, CursorType, Store};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One store's line in `cursors.json`: where its tailer got to, and **where the chain was
/// when that position was taken**.
///
/// The two travel together because neither answers the restart question on its own.
/// `cursors.json` is written after a batch is chained, so a kill part-way through one
/// leaves the saved position at the batch's start while `k` of that batch's records are
/// already in the chain. The restored tailer re-reads all of them and emits nothing — it
/// went back from no offset it knew about — so without a second fact the daemon either
/// chains `k` duplicates or guesses.
///
/// `last_leg1_seq` is that fact, and it is exact: the number of `EventBody::Leg1Record`s
/// for this store in the chain above it is `k`, counted rather than inferred.
///
/// `None` means the store had chained nothing when its position was saved, which is not the
/// same as zero: every `Leg1Record` for the store then sits above it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedCursor {
    pub cursor: CursorState,
    #[serde(default)]
    pub last_leg1_seq: Option<u64>,
}

/// One file's byte position, keyed by the inode it was measured against. A new inode at the
/// same path is a different file, not a longer one, which is why the two travel together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileCursor {
    pub inode: u64,
    /// The first byte not yet emitted. Always on a line boundary for a line store: a
    /// partial trailing line is held back rather than half-consumed, so this offset is
    /// exactly where a restarted tailer resumes.
    pub offset: u64,
}

/// A tailer's position in one store.
///
/// `records` is a monotonic count of the records the tailer has emitted for the store,
/// carried beside the key rather than derived from it. See the module note: without it,
/// `cursor_mismatch` is unreachable on every line-oriented store the shipped connectors declare.
///
/// Emitted, and not read: a partition filter and an already-passed ordinal both drop
/// records, and a count that included them would read as a chain gap. Each tailer owns the
/// counter it advances — it holds the count beside its own reader state and builds a
/// `CursorState` from both in `Tailer::cursor` — so the number arrives here already
/// correct, and there is no mutator on this enum to disagree with them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorState {
    /// Byte position per file, for an append-only text store.
    InodeOffset {
        files: BTreeMap<String, FileCursor>,
        records: u64,
    },
    /// The offset cursor with a second key: the highest ordinal already emitted per file.
    /// Bytes are still read by inode and offset — the ordinal is what makes a re-read of a
    /// file the tailer had already passed idempotent, per file (spike 2).
    Ordinal {
        last: BTreeMap<String, i64>,
        files: BTreeMap<String, FileCursor>,
        records: u64,
    },
    /// SQLite `rowid`. `-1` before the first row, so a row at rowid 0 is still read.
    Rowid { last: i64, records: u64 },
    /// A SQLite primary key the connector names (`id`).
    SqlitePk { last: i64, records: u64 },
    /// Per-session sequence numbers. No tailer is built for this: `tailer_for` refuses it by
    /// name. The variant stays because `cursor_field_name` and `guard_unsafe` need it — OpenCode
    /// declares `unsafe_cursors = ["event.seq"]`.
    SessionSeq { last: BTreeMap<String, i64> },
    /// The directory tailer's realisation of `inode_offset`: a per-file `(len, mtime_nanos)`
    /// seen-set rather than a byte offset, because it captures whole files.
    SeenFiles {
        seen: BTreeMap<String, (u64, i128)>,
        records: u64,
    },
}

impl CursorState {
    /// The empty cursor for a declared type.
    ///
    /// `InodeOffset` yields the byte-offset state and never the directory tailer's
    /// seen-set: both are declared `inode_offset` in a manifest, and it is the tailer, not
    /// the manifest, that knows which of the two it keeps. `DirTailer` builds its own.
    pub fn initial(kind: CursorType) -> CursorState {
        match kind {
            CursorType::InodeOffset => CursorState::InodeOffset {
                files: BTreeMap::new(),
                records: 0,
            },
            CursorType::Ordinal => CursorState::Ordinal {
                last: BTreeMap::new(),
                files: BTreeMap::new(),
                records: 0,
            },
            // `-1` and not `0`: a row at rowid 0 exists, and `> last` must include it.
            CursorType::Rowid => CursorState::Rowid {
                last: -1,
                records: 0,
            },
            CursorType::SqlitePk => CursorState::SqlitePk {
                last: -1,
                records: 0,
            },
            CursorType::SessionSeq => CursorState::SessionSeq {
                last: BTreeMap::new(),
            },
        }
    }

    /// The manifest type this state realises. `SeenFiles` reports `InodeOffset`, which is
    /// what the two `mode = "dir"` stores declare.
    pub fn kind(&self) -> CursorType {
        match self {
            CursorState::InodeOffset { .. } | CursorState::SeenFiles { .. } => {
                CursorType::InodeOffset
            }
            CursorState::Ordinal { .. } => CursorType::Ordinal,
            CursorState::Rowid { .. } => CursorType::Rowid,
            CursorState::SqlitePk { .. } => CursorType::SqlitePk,
            CursorState::SessionSeq { .. } => CursorType::SessionSeq,
        }
    }

    /// The human form — `"inode:123,offset:4096"`, `"rowid:512"`. Rendered for a reader;
    /// parsed by nobody, ever. Bounded: a store with many files shows the first few and
    /// counts the rest, because this string goes into a heartbeat once a minute.
    pub fn describe(&self) -> String {
        match self {
            CursorState::InodeOffset { files, records } => {
                format!("{},records:{records}", describe_files(files, |_| None))
            }
            CursorState::Ordinal {
                last,
                files,
                records,
            } => format!(
                "{},records:{records}",
                describe_files(files, |name| last.get(name).map(|o| format!("ordinal:{o}")))
            ),
            CursorState::Rowid { last, records } => format!("rowid:{last},records:{records}"),
            CursorState::SqlitePk { last, records } => format!("id:{last},records:{records}"),
            CursorState::SessionSeq { last } => {
                let mut parts: Vec<String> =
                    last.iter().map(|(s, n)| format!("{s}=seq:{n}")).collect();
                truncate_parts(&mut parts);
                if parts.is_empty() {
                    "seq:none".to_string()
                } else {
                    parts.join(";")
                }
            }
            CursorState::SeenFiles { seen, records } => {
                let bytes: u64 = seen.values().map(|(len, _)| *len).sum();
                let mtime = seen.values().map(|(_, m)| *m).max().unwrap_or(0);
                format!(
                    "files:{},bytes:{bytes},mtime:{mtime},records:{records}",
                    seen.len()
                )
            }
        }
    }

    /// The same fact, typed: the count of records emitted, under [`CursorUnit::Records`].
    ///
    /// `Records` for every state that counts them, which is all but `SessionSeq`, whose
    /// per-session map has no single number.
    pub fn position(&self) -> (Option<u64>, CursorUnit) {
        match self {
            CursorState::InodeOffset { records, .. }
            | CursorState::Ordinal { records, .. }
            | CursorState::Rowid { records, .. }
            | CursorState::SqlitePk { records, .. }
            | CursorState::SeenFiles { records, .. } => (Some(*records), CursorUnit::Records),
            CursorState::SessionSeq { .. } => (None, CursorUnit::Opaque),
        }
    }

    /// Exactly one cursor per store per heartbeat: `gaps::cursor_for` takes the first
    /// `StoreCursor` whose `store` matches, so a second one for the same store would be
    /// invisible — and if it were the first, it would displace the one that counts records.
    pub fn to_store_cursor(&self, store: &str) -> StoreCursor {
        let (position, unit) = self.position();
        StoreCursor {
            store: store.to_string(),
            unit,
            position,
            describe: self.describe(),
        }
    }
}

/// At most this many per-file entries are rendered into a `describe` string; the rest are
/// counted. Codex writes one rollout file per session under a dated tree, and a heartbeat
/// carrying every one of them is a log line nobody reads.
const DESCRIBE_MAX_PARTS: usize = 4;

fn describe_files(
    files: &BTreeMap<String, FileCursor>,
    extra: impl Fn(&str) -> Option<String>,
) -> String {
    if files.is_empty() {
        return "inode:none,offset:0".to_string();
    }
    let one = files.len() == 1;
    let mut parts: Vec<String> = files
        .iter()
        .map(|(name, c)| {
            let mut s = format!("inode:{},offset:{}", c.inode, c.offset);
            if let Some(e) = extra(name) {
                s.push(',');
                s.push_str(&e);
            }
            // A single-file store — every Claude and Codex line store in practice — reads as
            // the plain form.
            if one { s } else { format!("{name}={s}") }
        })
        .collect();
    truncate_parts(&mut parts);
    parts.join(";")
}

fn truncate_parts(parts: &mut Vec<String>) {
    if parts.len() > DESCRIBE_MAX_PARTS {
        let more = parts.len() - DESCRIBE_MAX_PARTS;
        parts.truncate(DESCRIBE_MAX_PARTS);
        parts.push(format!("(+{more} more)"));
    }
}

/// The record field a cursor type reads, for the one purpose D19 gives it: naming the
/// cursor a manifest's `unsafe_cursors[]` refers to. OpenCode writes `"event.seq"`, and
/// `seq` is this function's answer for `SessionSeq`.
///
/// `Ordinal` and `InodeOffset` have no canonical column — Codex names its own
/// (`payload.ordinal`) and an offset is not a field at all — so they answer with their type
/// name's natural field, which is what `<store>.<field>` would have to spell to refer to
/// them. The manifest's own `cursor.column`, where it has one, is checked too.
pub fn cursor_field_name(kind: CursorType) -> &'static str {
    match kind {
        CursorType::Ordinal => "ordinal",
        CursorType::Rowid => "rowid",
        CursorType::SqlitePk => "id",
        CursorType::SessionSeq => "seq",
        CursorType::InodeOffset => "offset",
    }
}

/// D19's run-time twin of the manifest check: never tail through a cursor the connector
/// itself declared unsafe.
///
/// The decision is [`ConnectorManifest::cursor_is_unsafe`] and not a second reading of the
/// rule here — one rule, one implementation, in the crate that owns the manifest. This
/// function's own work is the *message*: it names the `unsafe_cursors[]` entry that
/// matched, so a refusal says `event.seq` rather than "this store is unsafe".
pub fn guard_unsafe(manifest: &ConnectorManifest, store: &Store) -> Result<(), DaemonError> {
    if !manifest.cursor_is_unsafe(store) {
        return Ok(());
    }
    let field = cursor_field_name(store.cursor.kind);
    let mut candidates = vec![
        field.to_string(),
        format!("{}.{field}", store.name),
        cursor_type_name(store.cursor.kind).to_string(),
        format!("{}.{}", store.name, cursor_type_name(store.cursor.kind)),
    ];
    if let Some(col) = &store.cursor.column {
        candidates.push(col.clone());
        candidates.push(format!("{}.{col}", store.name));
    }
    let named = manifest
        .unsafe_cursors
        .iter()
        .find(|u| candidates.iter().any(|c| &c == u));
    Err(DaemonError::Guard(match named {
        Some(u) => format!(
            "store `{}` reads cursor `{u}`, which this connector declares in \
             unsafe_cursors[] (D19): a cursor that is unique per aggregate only cannot \
             order a store-wide read",
            store.name
        ),
        // Unreachable while `cursor_is_unsafe` matches on the same four spellings; stated
        // rather than unwrapped, because `unwrap` is not allowed in this crate and a
        // protocol that grew a fifth spelling should still refuse the store.
        None => format!(
            "store `{}` reads a cursor this connector declares in unsafe_cursors[] (D19)",
            store.name
        ),
    }))
}

/// The manifest's own spelling of a cursor type, which `unsafe_cursors[]` may also name.
/// The protocol crate keeps this private, so it is restated here — as a `match` that fails
/// to compile if a sixth type is added, not as a lookup that would silently miss it.
///
/// `pub(crate)` for the SQLite tailer's refusals, which name the type a store declared
/// (`inode_offset`) the way the manifest spells it. One spelling, in one place: a third
/// copy is how a refusal starts naming a cursor type no manifest has ever written.
pub(crate) fn cursor_type_name(k: CursorType) -> &'static str {
    match k {
        CursorType::Ordinal => "ordinal",
        CursorType::Rowid => "rowid",
        CursorType::SqlitePk => "sqlite_pk",
        CursorType::SessionSeq => "session_seq",
        CursorType::InodeOffset => "inode_offset",
    }
}
