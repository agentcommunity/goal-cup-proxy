//! The bounded facts a decode keeps about the records it read, whichever decoders ran:
//! record counts by kind, the first `Launch`, at most [`MAX_REFERENCES`] operational
//! references in [`MAX_REFERENCE_BYTES`], and a [`MAX_GOAL_EXCERPT_BYTES`] excerpt per open
//! goal stream. Nothing here grows with the run. A report's summary prints them; a server
//! reads them from [`DecodeOutcome`](super::session::DecodeOutcome).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use goalcup_protocol::capture::{
    CaptureRecord, CaptureSource, ConnectionEvent, EndReason, StreamKey,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// Operational references kept in a summary, and the serialized bytes they may use together.
const MAX_REFERENCES: usize = 128;
const MAX_REFERENCE_BYTES: usize = 32 * 1024;
/// The sanitized bytes a summary shows of one goal stream.
const MAX_GOAL_EXCERPT_BYTES: usize = 1024;

/// One operational fact that has no segment, kept in chain order.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum Reference {
    /// A `Gap`, `Recovery`, `Assistance` or later `Launch` record, as chained.
    Record(CaptureRecord),
    /// A goal stream's retained parts and a short excerpt of its sanitized bytes.
    Goal(GoalReference),
}

/// What a decode keeps of one goal stream: its retained parts and length, and a short excerpt
/// of its sanitized bytes. The whole goal stays in the chain.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct GoalReference {
    kind: &'static str,
    pub stream: StreamKey,
    pub first_part: Option<u64>,
    pub last_part: Option<u64>,
    pub source_bytes: u64,
    /// How the stream ended; `None` if it never did.
    pub end: Option<EndReason>,
    pub excerpt: String,
    pub excerpt_truncated: bool,
}

/// The bounded facts about the records read, whichever decoders ran. Opaque: its vocabulary
/// (the count keys, the reference kinds) is the research report's, documented in
/// `docs/research/m2-report-format.md`, and changes with that document.
#[derive(Debug, Clone, Default)]
pub struct Operations {
    counts: BTreeMap<String, u64>,
    launch: Option<CaptureRecord>,
    references: Vec<Reference>,
    references_omitted: u64,
    goals: BTreeMap<StreamKey, GoalExcerpt>,
    reference_bytes: usize,
}

impl Operations {
    /// Records by kind, qualified by source, reason or outcome (`open.native`, `gap.queue_full`).
    /// The keys come from closed enums, so few exist.
    pub fn counts(&self) -> &BTreeMap<String, u64> {
        &self.counts
    }

    /// The first `Launch` record.
    pub fn launch(&self) -> Option<&CaptureRecord> {
        self.launch.as_ref()
    }

    /// The first operational references in chain order (goal excerpts, later launches, gaps,
    /// recoveries, assistance), bounded.
    pub fn references(&self) -> &[Reference] {
        &self.references
    }

    /// References dropped once a bound was reached.
    pub fn references_omitted(&self) -> u64 {
        self.references_omitted
    }

    pub(super) fn record(&mut self, record: &CaptureRecord) {
        *self.counts.entry(count_key(record)).or_default() += 1;
        match record {
            CaptureRecord::Open {
                stream,
                source: CaptureSource::Goal,
                ..
            } => {
                self.goals.insert(stream.clone(), GoalExcerpt::default());
            }
            CaptureRecord::Segment {
                stream,
                part,
                source_len,
                data_b64,
                ..
            } => {
                if let Some(goal) = self.goals.get_mut(stream) {
                    goal.segment(*part, *source_len, data_b64);
                }
            }
            CaptureRecord::End { stream, reason, .. } => {
                if let Some(goal) = self.goals.remove(stream) {
                    self.reference(goal.reference(stream, Some(*reason)));
                }
            }
            CaptureRecord::Launch { .. } if self.launch.is_none() => {
                self.launch = Some(record.clone());
            }
            CaptureRecord::Launch { .. }
            | CaptureRecord::Gap { .. }
            | CaptureRecord::Recovery { .. }
            | CaptureRecord::Assistance { .. } => {
                self.reference(Reference::Record(record.clone()));
            }
            _ => {}
        }
    }

    /// Goal streams that never ended, after the last record.
    pub(super) fn finish(&mut self) {
        for (stream, goal) in std::mem::take(&mut self.goals) {
            self.reference(goal.reference(&stream, None));
        }
    }

    /// Keep `reference` when it and every reference before it fit the bounds, so the kept
    /// references are the first ones in chain order; count it otherwise.
    fn reference(&mut self, reference: Reference) {
        let bytes = serde_json::to_vec(&reference).map_or(usize::MAX, |bytes| bytes.len());
        if self.references_omitted == 0
            && self.references.len() < MAX_REFERENCES
            && bytes <= MAX_REFERENCE_BYTES - self.reference_bytes
        {
            self.reference_bytes += bytes;
            self.references.push(reference);
        } else {
            self.references_omitted += 1;
        }
    }
}

/// A record's count key: its kind, qualified by its source, reason or outcome.
fn count_key(record: &CaptureRecord) -> String {
    match record {
        CaptureRecord::ProducerClosed { .. } => "producer_closed".to_string(),
        CaptureRecord::Connection { event, .. } => match event {
            ConnectionEvent::Open { .. } => "connection.open".to_string(),
            ConnectionEvent::UpgradedWebSocket => "connection.upgraded_web_socket".to_string(),
            ConnectionEvent::Close { outcome } => format!("connection.close.{}", word(outcome)),
        },
        CaptureRecord::Recovery { reason, .. } => format!("recovery.{}", word(reason)),
        CaptureRecord::Open { source, .. } => format!("open.{}", word(source)),
        CaptureRecord::Segment { .. } => "segment".to_string(),
        CaptureRecord::End { reason, .. } => format!("end.{}", word(reason)),
        CaptureRecord::Gap { reason, .. } => format!("gap.{}", word(reason)),
        CaptureRecord::Launch { .. } => "launch".to_string(),
        CaptureRecord::Assistance { .. } => "assistance".to_string(),
        CaptureRecord::LocalFinish { .. } => "local_finish".to_string(),
    }
}

/// A unit enum as its serialization spells it.
fn word<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(word)) => word,
        _ => "unknown".to_string(),
    }
}

/// A goal stream's bytes as they arrive, until it ends and becomes a [`GoalReference`].
#[derive(Debug, Clone, Default)]
struct GoalExcerpt {
    first_part: Option<u64>,
    last_part: Option<u64>,
    source_bytes: u64,
    excerpt: Vec<u8>,
    truncated: bool,
}

impl GoalExcerpt {
    fn segment(&mut self, part: u64, source_len: u64, data_b64: &str) {
        self.first_part.get_or_insert(part);
        self.last_part = Some(part);
        self.source_bytes = self.source_bytes.saturating_add(source_len);
        let room = MAX_GOAL_EXCERPT_BYTES - self.excerpt.len();
        if room == 0 {
            self.truncated |= !data_b64.is_empty();
            return;
        }
        // The reader has already checked this segment's encoding and bounds.
        let bytes = STANDARD.decode(data_b64).unwrap_or_default();
        self.excerpt
            .extend_from_slice(&bytes[..bytes.len().min(room)]);
        self.truncated |= bytes.len() > room;
    }

    fn reference(self, stream: &StreamKey, end: Option<EndReason>) -> Reference {
        // Cut on a character boundary: bytes that are not UTF-8 show their valid prefix.
        let text = match std::str::from_utf8(&self.excerpt) {
            Ok(text) => text,
            Err(error) => {
                std::str::from_utf8(&self.excerpt[..error.valid_up_to()]).unwrap_or_default()
            }
        };
        Reference::Goal(GoalReference {
            kind: "goal",
            stream: stream.clone(),
            first_part: self.first_part,
            last_part: self.last_part,
            source_bytes: self.source_bytes,
            end,
            excerpt: text.to_string(),
            excerpt_truncated: self.truncated || text.len() < self.excerpt.len(),
        })
    }
}
