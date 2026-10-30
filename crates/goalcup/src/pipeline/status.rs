//! `status.json` (seam S4, PRD-02 section 6). Only the recorder writes it, each second, by atomic
//! rename; `goalcup status`, the end screen (PRD-10) and the uploader (PRD-06) read it.

use super::counts::Counts;
use crate::watchdog::{Exit, SealReason};
use serde::{Deserialize, Serialize};

/// The schema version.
pub const STATUS_V: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub v: u32,
    /// Wall time of this write, RFC 3339 UTC.
    pub updated: String,
    /// The recorder's pid.
    pub pid: u32,
    /// The recorder's OS start time (as `HarnessFacts::start_id`), so `goalcup recover` never
    /// signals a process that reused its pid.
    pub start_id: u64,
    pub boot: u64,
    /// The listen port, so `goalcup continue` and `goalcup recover` can ask for it again.
    pub port: u16,
    pub mode: Mode,
    pub harness: Option<HarnessStatus>,
    /// Entry totals at the last LINK whose index line is written.
    pub written: Written,
    pub last_request_at: Option<String>,
    pub last_native_at: Option<String>,
    pub losing: Losing,
    pub off: Off,
    /// The first note of each `what`, then the last 20 `note` events.
    pub notes: Vec<serde_json::Value>,
    pub held_bytes: u64,
    pub held_peak: u64,
    /// The soft guard in bytes.
    pub guard: u64,
    pub writer_idle_ms: u64,
    pub durable: DurableStatus,
    pub deadline: Option<String>,
    /// `reading session files` while the seal reads them with no time guard; null otherwise.
    pub sealing: Option<String>,
    // Sealed only (`mode: sealed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seal: Option<SealStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut_files: Option<CutFiles>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outlived: Option<Outlived>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_seal: Option<AfterSeal>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Recording,
    /// Some stream is in a gap, or a `parse_stopped` connection's closing GAP is not yet written.
    Losing,
    /// Records wait and nothing was written for 10 s (display only).
    WriterSlow,
    /// Records nothing; `off.since` is set.
    Null,
    Sealed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessStatus {
    pub pid: u32,
    /// Wall time, RFC 3339 UTC, for display only.
    pub started: String,
    /// `HarnessFacts::start_id`: the OS start time of `pid`. The harness is alive only while
    /// `pid` answers and this still matches; `started` is never compared.
    pub start_id: u64,
    pub exit: Option<Exit>,
}

/// `written`: every COUNT key as entry totals, plus `model_requests`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Written {
    pub model_requests: u64,
    #[serde(flatten)]
    pub counts: Counts,
}

impl Written {
    pub fn new(counts: Counts) -> Written {
        Written {
            model_requests: counts.model_requests(),
            counts,
        }
    }
}

/// `Recorder::live()` (seam S4): the live atomics `status.json` is written from each second.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Live {
    pub boot: u64,
    /// Bytes offered and not yet written, plus reservations. Records wait while this is not 0.
    pub held_bytes: u64,
    /// Boot time since the writer last wrote.
    pub writer_idle_ms: u64,
    pub losing: Losing,
    /// Entry totals over boots, the open null span included.
    pub off: Off,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Losing {
    /// Streams in a gap, plus `parse_stopped` connections whose closing GAP is not yet written.
    pub streams_in_gap: u64,
    /// Bytes dropped this boot.
    pub dropped_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Off {
    /// `recorder_down` spans on `sid 0`.
    pub spans: u64,
    pub ms: u64,
    /// Wall time a null recorder started, RFC 3339 UTC.
    pub since: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableStatus {
    pub seq: u64,
    pub raw_end: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealStatus {
    pub reason: SealReason,
    /// `git rev-parse HEAD` of the workspace, null on failure.
    pub workspace_head: Option<String>,
}

/// From `NativeFinish` (PRD-04): session files cut by the seal's time guard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutFiles {
    pub files: u64,
    pub bytes: u64,
}

/// Processes the harness left running at the seal (note `outlived`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outlived {
    pub count: u64,
    pub programs: Vec<String>,
}

/// Bytes relayed after the seal's LINK, which cannot be recorded, and their hosts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfterSeal {
    pub bytes: u64,
    pub hosts: Vec<String>,
}
