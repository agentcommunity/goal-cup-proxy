//! One store's tailing cursor at a heartbeat (D9), and what its number counts.
//!
//! The cursor carries the unit its number counts, so the verifier never compares a byte
//! offset with a record count. A bare integer mined out of a human string would compare
//! 4,096 *bytes* with three *records* on an offset store, and a timestamp with a record
//! count on a directory tailer's `(len, mtime)` string. The human string sits beside the
//! number rather than in front of it.
//!
//! `events` re-exports `StoreCursor`, so `EventBody::Heartbeat.stores_seen` keeps its type
//! path: there is one type at two paths, never two types.

use serde::{Deserialize, Serialize};

/// What a cursor's `position` counts. The verifier compares a cursor's movement against a
/// record count ONLY under `Records`; every other unit is a number it may report and must
/// never do arithmetic against a count with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorUnit {
    /// `position` counts records the tailer has passed. The only unit comparable with a
    /// count of records in a window.
    Records,
    /// `position` is a byte offset into a file.
    Bytes,
    /// `position` is whole seconds since the epoch — a directory tailer's high-water mark.
    Time,
    /// `position` is a key with meaning only inside its own store (a rowid, an ordinal), or
    /// the cursor spans several files and has no single number. `position` may still be
    /// `Some`; it is reported and never compared.
    Opaque,
}

/// One store's cursor at a heartbeat (D9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreCursor {
    /// The store this cursor belongs to. Kept: `stores_seen` is a list, and the gap
    /// classifier looks a store up in it by name.
    pub store: String,
    pub unit: CursorUnit,
    /// The number the daemon actually holds, `None` when the cursor has none. Reported,
    /// not parsed out of prose.
    pub position: Option<u64>,
    /// The human form of the cursor — `"inode:123,offset:4096"`,
    /// `"rowid:512"`. Rendered for a reader; parsed by nobody.
    pub describe: String,
}

impl StoreCursor {
    /// `position` when, and only when, `unit == CursorUnit::Records`. This method is the
    /// whole point of the type: there is no path by which a byte offset reaches a
    /// comparison against a count of lines.
    pub fn records(&self) -> Option<u64> {
        match self.unit {
            CursorUnit::Records => self.position,
            CursorUnit::Bytes | CursorUnit::Time | CursorUnit::Opaque => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The unit is on the wire in the daemon's own spelling, so a heartbeat written by a
    /// future daemon and read by this verifier means what it said.
    #[test]
    fn every_unit_serialises_as_its_snake_case_name() {
        for (unit, text) in [
            (CursorUnit::Records, "\"records\""),
            (CursorUnit::Bytes, "\"bytes\""),
            (CursorUnit::Time, "\"time\""),
            (CursorUnit::Opaque, "\"opaque\""),
        ] {
            assert_eq!(serde_json::to_string(&unit).unwrap(), text);
            assert_eq!(serde_json::from_str::<CursorUnit>(text).unwrap(), unit);
        }
    }

    /// `records()` is a filter, not a parser: the same number is comparable under one unit
    /// and invisible under the other three.
    #[test]
    fn only_a_records_cursor_yields_a_comparable_number() {
        let at = |unit| StoreCursor {
            store: "main".to_string(),
            unit,
            position: Some(4096),
            describe: "inode:1,offset:4096".to_string(),
        };
        assert_eq!(at(CursorUnit::Records).records(), Some(4096));
        for unit in [CursorUnit::Bytes, CursorUnit::Time, CursorUnit::Opaque] {
            assert_eq!(at(unit).records(), None, "{unit:?}");
        }
    }
}
