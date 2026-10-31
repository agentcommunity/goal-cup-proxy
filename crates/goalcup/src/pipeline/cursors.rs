//! Durable native positions (`cursors.json`, PRD-02 section 9.3), written only at a durable point.

use super::status::write_private;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// Per session file path (the ID, `<source>:<relative path>`) and per snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cursors {
    pub native: BTreeMap<String, NativeCursor>,
    pub snapshots: BTreeMap<String, SnapshotCursor>,
    /// The durable index length at the last sync: what recovery after a reboot keeps (PRD-01
    /// section 12 step 1, `goalcup_format::Recover::durable`).
    pub durable: Option<DurablePos>,
    /// The machine's `boot_id` (32 hex) when this file was written. Recovery passes `durable` only
    /// when the current `boot_id` differs.
    pub boot_id: Option<String>,
}

/// A part file name and its index length in bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurablePos {
    pub part: String,
    pub index_len: u64,
}

impl Cursors {
    const NAME: &'static str = "cursors.json";

    /// The cursors in `entry_dir`. A missing or unreadable file gives the default, never an error.
    pub fn read(entry_dir: &Path) -> Cursors {
        std::fs::read(entry_dir.join(Self::NAME))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Writes `cursors.json` by atomic rename, synced, mode 0600.
    pub fn write(&self, entry_dir: &Path) -> io::Result<()> {
        let bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        write_private(&entry_dir.join(Self::NAME), &bytes, true)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeCursor {
    #[serde(rename = "gen")]
    pub generation: u64,
    pub fid: Fid,
    /// The offset durably written.
    pub off: u64,
}

/// A file's identity: device and inode, and on macOS the birth time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fid {
    pub dev: u64,
    pub ino: u64,
    pub birth_ns: Option<i64>,
}

/// A snapshot file as last read whole. It carries no `fid` yet: a file replaced with the same size
/// and `mtime_ns` is not read again on resume (known gap, PRD-02 amendment after 01afcf22). The
/// `fid` lands with PRD-04's matching change to `Tail::new` and `tests/native_resume.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotCursor {
    pub size: u64,
    pub mtime_ns: i64,
}
