//! What the CLI tests share: a `status.json` (seam S4) built through its JSON form, so a key
//! PRD-02 adds changes one place. Options left out read as null.

#![allow(dead_code)]

use goalcup::pipeline::status::{DurableStatus, Losing, Mode, Off, STATUS_V, Status, Written};
use serde_json::json;

/// 2026-11-07T10:00:00Z, the `updated` of [`status`].
pub const T0: u64 = 1_794_045_600;
pub const RUN: &str = "r0123456789abcdef";
pub const ENTRY: &str = "e0123456789abcdef";

/// A `status.json` in `mode`, written at [`T0`], with nothing counted.
pub fn status(mode: Mode) -> Status {
    serde_json::from_value(json!({
        "v": STATUS_V, "updated": "2026-11-07T10:00:00Z", "pid": 0, "start_id": 0, "boot": 0,
        "port": 41_000, "mode": mode, "written": Written::default(), "losing": Losing::default(),
        "off": Off::default(), "notes": [], "held_bytes": 0, "held_peak": 0, "guard": 0,
        "writer_idle_ms": 0, "durable": DurableStatus::default(),
    }))
    .expect("a status.json")
}
