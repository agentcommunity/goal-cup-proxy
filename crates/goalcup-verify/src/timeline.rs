//! The report's `timeline` (PRD-08 section 4), tallied in the integrity walk: native bytes and
//! GAPs by time, the off spans, and the streams by file path. Rows are sparse, keyed by bucket
//! index from the entry's first PART wall time. An off span is clamped to the PART and LINK wall
//! range, which a DATA or GAP time cannot widen, so a forged span adds no buckets past it.

use crate::integrity::{has_extent, is_meta_gap};
use crate::legs::{Streams, Timed};
use goalcup_format::dump::wall;
use goalcup_format::{GapReason, Record, StreamKind};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};

/// A timeline bucket, ns.
const BUCKET: u64 = 600_000_000_000;

/// One row: harness calls, wire calls, wire errors, native bytes, gaps, off ms.
type Row = [u64; 6];

#[derive(Default)]
pub struct Tally {
    first: Option<u64>,
    last: u64,
    boot: Option<u64>,
    /// Open native and snapshot streams: path and whether native.
    open: HashMap<u64, (String, bool)>,
    rows: BTreeMap<u64, Row>,
    off: Vec<(u64, u64)>,
    pub streams: Streams,
}

impl Tally {
    fn row(&mut self, w: u64) -> Option<&mut Row> {
        let i = w.saturating_sub(self.first?) / BUCKET;
        Some(self.rows.entry(i).or_default())
    }

    /// One record; `w` is its wall time on the PART's clock.
    pub fn item(&mut self, r: &Record, w: u64) {
        match r {
            Record::Part { boot, wall_ns, .. } => {
                if self.boot.replace(*boot) != Some(*boot) {
                    self.open.clear();
                }
                self.first.get_or_insert(*wall_ns);
                self.last = self.last.max(*wall_ns);
            }
            Record::Link { wall_ns, .. } => self.last = self.last.max(*wall_ns),
            Record::Open {
                sid, skind, meta, ..
            } if matches!(skind, StreamKind::Native | StreamKind::Snapshot) => {
                let m: Value = serde_json::from_slice(meta).unwrap_or_default();
                let path = m["path"].as_str().unwrap_or_default().to_string();
                self.open.insert(*sid, (path, *skind == StreamKind::Native));
            }
            Record::Data { sid, bytes, .. } if self.open.contains_key(sid) => {
                if let Some(x) = self.row(w) {
                    x[3] = x[3].saturating_add(bytes.len() as u64);
                }
            }
            Record::End { sid, seen, .. } => {
                if let Some((p, native)) = self.open.remove(sid) {
                    if native {
                        self.streams.native.insert(p, *seen);
                    } else {
                        self.streams.snapshots.insert(p);
                    }
                }
            }
            Record::Gap {
                sid,
                reason,
                start_wall,
                end_wall,
                start_mono,
                end_mono,
                lost,
                tail,
                ..
            } => {
                if let Some(x) = self.row(*start_wall) {
                    x[4] = x[4].saturating_add(1);
                }
                let tail: Map<String, Value> = serde_json::from_slice(tail).unwrap_or_default();
                let (wl, mo) = ((*start_wall, *end_wall), (*start_mono, *end_mono));
                if !is_meta_gap(*reason, *lost, &tail)
                    && let Some((p, _)) = self.open.get(sid)
                {
                    self.streams.gapped.insert(p.clone());
                }
                if *sid == 0
                    && *reason == GapReason::RecorderDown
                    && has_extent(&tail, wl, mo, *lost)
                {
                    self.off.push(wl);
                }
            }
            _ => {}
        }
    }

    /// Adds the calls and spreads each off span over the buckets it overlaps.
    pub fn finish(&mut self, calls: &[Timed]) {
        for c in calls {
            if let Some(x) = self.row(c.wall) {
                let k = usize::from(!c.native);
                x[k] = x[k].saturating_add(1);
                x[2] = x[2].saturating_add(u64::from(c.err && !c.native));
            }
        }
        let Some(first) = self.first else { return };
        for (s, e) in std::mem::take(&mut self.off) {
            let (s, e) = (s.max(first), e.min(self.last));
            if e <= s {
                continue;
            }
            for i in (s - first) / BUCKET..=(e - 1 - first) / BUCKET {
                let lo = first + i * BUCKET;
                let ns = e.min(lo.saturating_add(BUCKET)) - s.max(lo);
                let x = self.rows.entry(i).or_default();
                x[5] = x[5].saturating_add(ns / 1_000_000);
            }
        }
    }

    /// The report's `timeline`: the buckets with anything in them.
    pub fn timeline(&self) -> Value {
        let first = self.first.unwrap_or(0);
        let rows = self.rows.iter().filter(|(_, r)| r.iter().any(|&n| n > 0));
        rows.map(|(&i, r)| {
            let start = first.saturating_add(i.saturating_mul(BUCKET));
            json!({"bucket": i, "start": wall(start), "harness_calls": r[0], "wire_calls": r[1],
                "wire_errors": r[2], "native_bytes": r[3], "gaps": r[4], "off_ms": r[5]})
        })
        .collect()
    }
}
