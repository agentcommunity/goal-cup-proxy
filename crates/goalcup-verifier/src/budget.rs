//! Aggregate bounds for JSONL evidence retained by one verifier pipeline.

/// Physical JSONL bytes one verifier run may retain across chain, legacy legs, and ACKs.
///
/// The daemon documents one to three envelopes per second for 168 hours and that unusually
/// large raw-body runs can reach tens of GiB. Sixty-four GiB admits that documented week
/// without pretending the retained in-memory representation is streaming, while still
/// putting a hard ceiling on participant-controlled growth.
pub const MAX_RETAINED_EVIDENCE_BYTES: u64 = 64 * 1024 * 1024 * 1024;

/// Parsed JSONL records one verifier run may retain across chain, legacy legs, and ACKs.
///
/// A week at the documented peak of three envelopes per second is 1,814,400 envelopes.
/// This power-of-two ceiling leaves more than nine times that envelope count for ACK and
/// legacy-leg records while remaining an explicit finite allocation boundary.
pub const MAX_RETAINED_EVIDENCE_RECORDS: u64 = 16_777_216;

#[derive(Debug)]
pub(crate) struct RetentionLimit {
    pub path: String,
    pub bytes: u64,
    pub records: u64,
    pub byte_limit: u64,
    pub record_limit: u64,
}

/// One cumulative budget shared by every retaining JSONL loader in a pipeline.
pub(crate) struct RetentionBudget {
    bytes: u64,
    records: u64,
    byte_limit: u64,
    record_limit: u64,
}

impl RetentionBudget {
    pub(crate) fn standard() -> Self {
        Self::with_limits(MAX_RETAINED_EVIDENCE_BYTES, MAX_RETAINED_EVIDENCE_RECORDS)
    }

    pub(crate) fn with_limits(byte_limit: u64, record_limit: u64) -> Self {
        Self {
            bytes: 0,
            records: 0,
            byte_limit,
            record_limit,
        }
    }

    /// Reserve one parsed line before any decoded value or derivative is retained.
    pub(crate) fn retain(&mut self, path: &str, physical_bytes: u64) -> Result<(), RetentionLimit> {
        let bytes = self.bytes.saturating_add(physical_bytes);
        let records = self.records.saturating_add(1);
        if bytes > self.byte_limit || records > self.record_limit {
            return Err(RetentionLimit {
                path: path.to_string(),
                bytes,
                records,
                byte_limit: self.byte_limit,
                record_limit: self.record_limit,
            });
        }
        self.bytes = bytes;
        self.records = records;
        Ok(())
    }
}
