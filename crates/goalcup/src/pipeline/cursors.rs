//! Durable native positions (`cursors.json`, PRD-02 section 9.3), written only at a durable point.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Per session file path (the ID, `<source>:<relative path>`) and per snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cursors {
    pub native: BTreeMap<String, NativeCursor>,
    pub snapshots: BTreeMap<String, SnapshotCursor>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotCursor {
    pub size: u64,
    pub mtime_ns: i64,
}
