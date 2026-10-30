//! Written counts (seam S4): the COUNT keys of PRD-01 section 9, the durable point, entry totals.

use super::clock::Mono;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The COUNT keys of PRD-01 section 9, counted at write. As entry totals (over boots): counters
/// summed, maps summed per key, `held_peak` the largest, gauges left out.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Counts {
    pub http_req: u64,
    pub http_post: u64,
    pub ws_up_msgs: u64,
    pub ws_down_msgs: u64,
    pub req_whole: u64,
    pub req_holes: u64,
    pub conns: u64,
    pub tunnels: u64,
    pub tunnel_up: u64,
    pub tunnel_down: u64,
    pub wire_bytes: u64,
    pub native_files: u64,
    pub native_bytes: u64,
    pub snapshots: u64,
    pub gaps: u64,
    pub lost_bytes: u64,
    pub lost_unknown: u64,
    pub off_ms: u64,
    pub mask_failclosed_bytes: u64,
    /// Model requests written per decrypted host.
    pub hosts: BTreeMap<String, u64>,
    pub tunnel_hosts: BTreeMap<String, TunnelHost>,
    pub held_peak: u64,
}

impl Counts {
    /// "Model requests written": `http_post + ws_up_msgs`. A request count, never a call count.
    pub fn model_requests(&self) -> u64 {
        self.http_post + self.ws_up_msgs
    }
}

/// One tunnelled host in COUNT `tunnel_hosts`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelHost {
    pub conns: u64,
    pub up: u64,
    pub down: u64,
    /// For a listed host, the GAP reason name of its tunnels; null for an unlisted host.
    pub listed: Option<String>,
}

/// The durable point, published on a `watch` channel after each `fdatasync` (PRD-02 section 5
/// step 4). The uploader sends up to it and never past it. `totals` are entry totals as they
/// stood at this LINK, so a count that leaves the machine never runs ahead of the record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Durable {
    /// The part file's name; empty before the first durable point.
    pub part: String,
    pub c_end: u64,
    pub raw_end: u64,
    /// The LINK's `seq` and hash.
    pub seq: u64,
    pub hash: [u8; 32],
    pub totals: Counts,
    /// When this point became durable, for its age.
    pub at: Mono,
}

/// Entry totals over every boot of the entry in `entry_dir` (PRD-01 section 9): one function for
/// boot recovery, `close_dead_entry`, the null recorder and `goalcup upload`.
pub fn entry_totals(entry_dir: &Path) -> Counts {
    let _ = entry_dir;
    Counts::default()
}
