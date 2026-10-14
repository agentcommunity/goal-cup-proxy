//! The one record shape the daemon ships and the verifier reads.

use crate::ProtocolError;
use crate::chain::{CHAIN_DOMAIN, Link, link_hash};
use crate::control::parse_z;
use crate::provenance::{CutoffReachedV1, FinalizationV1};
use crate::redaction::RedactionAudit;
use serde::{Deserialize, Serialize};

/// A shipped event. The chain fields are flattened to the top level, so `seq`, `prev` and
/// `hash` sit beside `entry_id` and `body` rather than nested under a `link` object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub protocol_version: u32,
    pub entry_id: String,
    /// Who appended the record. A cutoff, finalization or local finish must say `"daemon"`;
    /// on other records it is descriptive text inside the hash.
    pub source: String,
    /// RFC 3339 with a trailing `Z`, not a float: a float could serialise as `null` (NaN)
    /// and would drop `Eq`.
    pub recorded_at: String,
    #[serde(flatten)]
    pub link: Link,
    pub body: EventBody,
    /// Pre-v5 inline redaction evidence. A v5 envelope never carries it: its redaction
    /// evidence lives inside a capture `Segment`, and a v5 chain line that sets this is
    /// refused. Absent, it is skipped, so it adds nothing to a v5 line's bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redaction: Option<RedactionAudit>,
}

/// What a link covers, in the order it is serialised.
///
/// `prev` and `hash` are absent on purpose: they are the link, not the content. Including
/// them would make the bytes unrecomputable — the hash cannot be an input to itself — and
/// would mean a chain could never be re-linked after a documented scrub.
#[derive(Serialize)]
struct ChainClaims<'a> {
    protocol_version: u32,
    entry_id: &'a str,
    source: &'a str,
    recorded_at: &'a str,
    seq: u64,
    body: &'a EventBody,
    #[serde(skip_serializing_if = "Option::is_none")]
    redaction: Option<&'a RedactionAudit>,
}

impl Envelope {
    /// `CHAIN_DOMAIN ++ serde_json::to_vec(&{protocol_version, entry_id, source,
    /// recorded_at, seq, body[, redaction]})`, struct field order, no whitespace. This is the ONE
    /// definition of what a link covers; the daemon hashes it and the verifier
    /// recomputes it.
    pub fn chain_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut bytes = CHAIN_DOMAIN.to_vec();
        bytes.extend_from_slice(&serde_json::to_vec(&ChainClaims {
            protocol_version: self.protocol_version,
            entry_id: &self.entry_id,
            source: &self.source,
            recorded_at: &self.recorded_at,
            seq: self.link.seq,
            body: &self.body,
            redaction: self.redaction.as_ref(),
        })?);
        Ok(bytes)
    }

    /// `link_hash(prev, &self.chain_bytes()?)` — what `self.link.hash` must equal.
    pub fn expected_hash(&self, prev: &str) -> Result<String, ProtocolError> {
        Ok(link_hash(prev, &self.chain_bytes()?))
    }

    /// `recorded_at` as seconds since the epoch, or `None` when it does not parse.
    /// Never `0`: a clock nobody can read is an absence, not a moment in 1970.
    pub fn recorded_at_secs(&self) -> Option<u64> {
        parse_z(&self.recorded_at)
    }
}

/// Closed by design at the five bodies a protocol-v5 chain may carry: the daemon interprets
/// nothing (D3). The pre-v5 bodies (leg-1 and leg-2 records, health, heartbeats, home seeding,
/// repository observations and the rest) left the protocol with D51: no writer has produced
/// them since D48, and a v5 chain never admitted them.
///
/// **Variants are additive.** An adjacently tagged enum serialises each variant on its own, so
/// adding or removing a variant changes no other variant's bytes; every v5 chain hashes exactly
/// as it did.
///
/// Adjacently tagged (`tag = "kind"`, `content = "content"`): the variant's fields sit in a
/// nested `content` object, so a field of a body can never collide with the outer `kind` tag.
///
/// `Capture` is far larger than the other four, and it is the body almost every line carries:
/// boxing it would allocate once per record to shrink the rare ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "content", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum EventBody {
    /// The entry's baseline, in the chain, as its FIRST link. Carries the *digest* of
    /// `baseline.json` for the same reason the seal carries the digest of `seal.json`:
    /// the manifest is on disk already, and a chain that repeated it would hand a
    /// reviewer two copies to compare instead of one claim to check.
    ///
    /// The whole point of the link is what sits above it. `baseline.json` alone is a file
    /// the participant's own daemon wrote, so a participant who pre-populated the
    /// workspace could list every planted file's real path and digest there and have the
    /// verifier exempt each one. As seq 0 the baseline IS the genesis link a start
    /// receipt's `chain_head` witnesses, so it cannot be rewritten after the run was
    /// declared without moving the head the server signed.
    ///
    /// Written by `TraceExport::write_baseline_for_protocol` and by nothing else.
    Baseline {
        manifest_sha256: String,
        file_count: u64,
    },
    /// The entry's seal, in the chain. Carries the *digest* of `seal.json` rather than
    /// the manifest itself: the manifest is already on disk, and a chain that repeated
    /// it would hand a reviewer two copies to compare instead of one claim to check.
    /// Written by `TraceExport::write_seal_for_protocol` and by nothing else — no daemon builds
    /// this variant by hand, which is what makes "the seal is the last link" a property of
    /// the writer rather than a convention.
    Seal {
        manifest_sha256: String,
        sealed_at: String,
        file_count: u64,
    },
    /// The once-only local cutoff boundary. Written only by the export's cutoff method.
    CutoffReachedV1(CutoffReachedV1),
    /// The terminal capture/publication result. Written only by the export's finalization method.
    FinalizationV1(FinalizationV1),
    /// One M2 recorder observation. `CaptureRecord::LocalFinish` is reserved to the export's
    /// terminal writer, never an ordinary append.
    Capture {
        record: crate::capture::CaptureRecord,
    },
}
