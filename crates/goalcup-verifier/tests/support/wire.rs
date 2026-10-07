//! Deterministic transport-record builders shared by the three built-in wire-decoder suites.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use goalcup_protocol::capture::{
    CaptureMeta, CaptureRecord, CaptureSource, CaptureTime, EndReason, GapReason, StreamKey,
    WireIdentity,
};
use goalcup_protocol::redaction::{RedactionAudit, RedactionAuditSpan};
use goalcup_verifier::capture_analysis::api::Observation;
use serde_json::Value;

#[derive(Clone)]
pub struct WireSpec {
    pub key: StreamKey,
    pub source: CaptureSource,
    pub meta: CaptureMeta,
    at: fn(u64) -> CaptureTime,
}

impl WireSpec {
    pub fn new(
        producer: &str,
        stream: u64,
        source: CaptureSource,
        meta: CaptureMeta,
        at: fn(u64) -> CaptureTime,
    ) -> Self {
        Self {
            key: StreamKey {
                producer: producer.to_string(),
                stream,
            },
            source,
            meta,
            at,
        }
    }

    pub fn wire(mut self, connection: &str, exchange: u64, message_index: Option<u64>) -> Self {
        self.meta.wire = Some(WireIdentity {
            connection: connection.into(),
            exchange,
            message_index,
        });
        self
    }

    pub fn content_type(mut self, content_type: Option<&str>) -> Self {
        self.meta.content_type = content_type.map(str::to_string);
        self
    }

    pub fn status(mut self, status: u16) -> Self {
        self.meta.status = Some(status);
        self
    }

    pub fn hint(mut self, hint: Option<&str>) -> Self {
        self.meta.protocol_hint = hint.map(str::to_string);
        self
    }

    pub fn open(&self) -> CaptureRecord {
        self.open_at(0)
    }

    pub fn open_at(&self, tick: u64) -> CaptureRecord {
        CaptureRecord::Open {
            stream: self.key.clone(),
            observed: (self.at)(tick),
            source: self.source,
            metadata: self.meta.clone(),
        }
    }

    pub fn end(&self, reason: EndReason) -> CaptureRecord {
        CaptureRecord::End {
            stream: self.key.clone(),
            observed: (self.at)(999_999),
            reason,
        }
    }

    pub fn gap(&self) -> CaptureRecord {
        CaptureRecord::Gap {
            stream: Some(self.key.clone()),
            observed: (self.at)(999_998),
            reason: GapReason::DecodeFailed,
            lost_bytes: None,
        }
    }

    pub fn segment(
        &self,
        part: u64,
        offset: u64,
        bytes: &[u8],
        times: (u64, u64),
        audit: Vec<RedactionAuditSpan>,
    ) -> CaptureRecord {
        CaptureRecord::Segment {
            stream: self.key.clone(),
            part,
            source_offset: offset,
            source_len: bytes.len() as u64,
            first: (self.at)(times.0),
            last: (self.at)(times.1),
            data_b64: STANDARD.encode(bytes),
            audit: RedactionAudit { spans: audit },
        }
    }

    pub fn segments(&self, bytes: &[u8], splits: &[usize], base: u64) -> Vec<CaptureRecord> {
        if bytes.is_empty() {
            return Vec::new();
        }
        let mut bounds = vec![0];
        bounds.extend_from_slice(splits);
        bounds.push(bytes.len());
        bounds
            .windows(2)
            .enumerate()
            .map(|(part, w)| {
                let part = part as u64;
                self.segment(
                    part,
                    w[0] as u64,
                    &bytes[w[0]..w[1]],
                    (base + 10 * part, base + 10 * part + 5),
                    Vec::new(),
                )
            })
            .collect()
    }

    pub fn records(&self, bytes: &[u8], splits: &[usize]) -> Vec<CaptureRecord> {
        let mut records = vec![self.open()];
        records.extend(self.segments(bytes, splits, 100));
        records.push(self.end(EndReason::Complete));
        records
    }
}

pub fn kind<'a>(observations: &'a [Observation], kind: &str) -> Vec<&'a Observation> {
    observations.iter().filter(|o| o.kind == kind).collect()
}

pub fn facts(observations: &[Observation]) -> Vec<(String, Option<String>, Value)> {
    observations
        .iter()
        .map(|o| (o.kind.clone(), o.supplied_id.clone(), o.fields.clone()))
        .collect()
}

pub fn parts(observation: &Observation) -> (u64, u64) {
    (observation.source.first_part, observation.source.last_part)
}
