//! The one record shape the daemon ships and the verifier reads.

use crate::ProtocolError;
use crate::chain::{CHAIN_DOMAIN, Link, link_hash};
use crate::control::parse_z;
use crate::events::{
    ConfigDeviation, ForeignScanOutcome, HealthEvent, StoreCursor, TailNoticeKind, VersionChange,
};
use crate::provenance::{CutoffReachedV1, FinalizationV1, RepositoryObservedV1};
use crate::redaction::{RedactionAudit, RedactionRecord};
use serde::{Deserialize, Serialize};

/// Why an envelope's declared protocol version cannot truthfully describe its body.
///
/// This is owned by the protocol rather than by a writer or reader: evidence is valid only
/// when the version that names its wire contract also admits the body it carries.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvelopeVersionCompatibilityError {
    #[error("protocol version {protocol_version} is unsupported")]
    UnsupportedProtocolVersion { protocol_version: u32 },
    #[error("protocol v{protocol_version} HomeSeeded evidence cannot carry v3 handoff markers")]
    HomeSeededRequiresV3 { protocol_version: u32 },
    #[error("protocol v{protocol_version} cannot relabel a parsed V1 HomeSeeded wire layout")]
    LegacyHomeSeededRequiresV1 { protocol_version: u32 },
    #[error("protocol v1 cannot carry {body}: that event body was introduced in v2")]
    BodyRequiresV2 { body: V2OnlyEventBody },
    #[error("protocol v1 has no inline redaction-audit field")]
    InlineRedactionRequiresV2,
    #[error("protocol v{protocol_version} cannot carry V1's standalone Redaction body")]
    LegacyRedactionRequiresV1 { protocol_version: u32 },
    #[error(
        "protocol v{protocol_version} cannot carry a workspace submission finalization introduced in v4"
    )]
    WorkspaceSubmissionRequiresV4 { protocol_version: u32 },
    #[error("protocol v{protocol_version} cannot carry a capture record introduced in v5")]
    CaptureRequiresV5 { protocol_version: u32 },
}

/// The event bodies that first appeared in V2 and must not be relabelled as V1 evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum V2OnlyEventBody {
    #[error("RepositoryObservedV1")]
    RepositoryObservedV1,
    #[error("CutoffReachedV1")]
    CutoffReachedV1,
    #[error("FinalizationV1")]
    FinalizationV1,
    #[error("ForeignScanCoverage")]
    ForeignScanCoverage,
}

/// A shipped event. The chain fields are flattened to the top level, so `seq`, `prev` and
/// `hash` sit beside `entry_id` and `body` rather than nested under a `link` object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub protocol_version: u32,
    pub entry_id: String,
    /// Store name for a leg-1 record, `"hooks"` for a leg-2 post, `"daemon"` for
    /// everything the daemon observed about itself or the filesystem.
    pub source: String,
    /// RFC 3339 with a trailing `Z`, not a float: a float could serialise as `null` (NaN)
    /// and would drop `Eq`.
    pub recorded_at: String,
    #[serde(flatten)]
    pub link: Link,
    pub body: EventBody,
    /// V2 evidence for bytes altered in this leg record. Absent on clean records and on
    /// every v1 envelope, so old chain bytes retain their exact representation.
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
    /// Check that this envelope's body is part of the protocol version it declares.
    ///
    /// Historical V1 and V2 later-layout `HomeSeeded` lines remain valid when they omit both
    /// V3 handoff markers. The legacy wire layout is V1-only and cannot be newly appended.
    /// V1 cannot label any V2-only body or an inline redaction audit, and V2/V3 cannot label
    /// V1's standalone `Redaction` record. Versions 1–4 cannot label a v5 `Capture` body.
    /// Protocol v5 itself stays `UnsupportedProtocolVersion` here: this is the LEGACY check the
    /// legacy writer and readers apply, and v5 admission belongs to the v5 writer and reader.
    /// Every mismatch is rejected before the line can be hashed, written, resumed, or retained
    /// by a verifier.
    pub fn validate_version_body_compatibility(
        &self,
    ) -> Result<(), EnvelopeVersionCompatibilityError> {
        if !crate::is_supported_protocol_version(self.protocol_version) {
            return Err(
                EnvelopeVersionCompatibilityError::UnsupportedProtocolVersion {
                    protocol_version: self.protocol_version,
                },
            );
        }

        if self.protocol_version == crate::LEGACY_PROTOCOL_VERSION && self.redaction.is_some() {
            return Err(EnvelopeVersionCompatibilityError::InlineRedactionRequiresV2);
        }

        match &self.body {
            EventBody::HomeSeeded(seed)
                if self.protocol_version != crate::LEGACY_PROTOCOL_VERSION
                    && seed.has_legacy_wire_layout() =>
            {
                Err(
                    EnvelopeVersionCompatibilityError::LegacyHomeSeededRequiresV1 {
                        protocol_version: self.protocol_version,
                    },
                )
            }
            EventBody::HomeSeeded(seed)
                if self.protocol_version < crate::HOME_SEED_HANDOFF_PROTOCOL_VERSION
                    && (seed.handoff_generation.is_some()
                        || seed.handoff_request_digest.is_some()) =>
            {
                Err(EnvelopeVersionCompatibilityError::HomeSeededRequiresV3 {
                    protocol_version: self.protocol_version,
                })
            }
            EventBody::RepositoryObservedV1(_)
                if self.protocol_version == crate::LEGACY_PROTOCOL_VERSION =>
            {
                Err(EnvelopeVersionCompatibilityError::BodyRequiresV2 {
                    body: V2OnlyEventBody::RepositoryObservedV1,
                })
            }
            EventBody::CutoffReachedV1(_)
                if self.protocol_version == crate::LEGACY_PROTOCOL_VERSION =>
            {
                Err(EnvelopeVersionCompatibilityError::BodyRequiresV2 {
                    body: V2OnlyEventBody::CutoffReachedV1,
                })
            }
            EventBody::FinalizationV1(_)
                if self.protocol_version == crate::LEGACY_PROTOCOL_VERSION =>
            {
                Err(EnvelopeVersionCompatibilityError::BodyRequiresV2 {
                    body: V2OnlyEventBody::FinalizationV1,
                })
            }
            EventBody::FinalizationV1(FinalizationV1::WorkspaceSubmission { .. })
                if self.protocol_version < crate::WORKSPACE_SUBMISSION_PROTOCOL_VERSION =>
            {
                Err(
                    EnvelopeVersionCompatibilityError::WorkspaceSubmissionRequiresV4 {
                        protocol_version: self.protocol_version,
                    },
                )
            }
            EventBody::ForeignScanCoverage { .. }
                if self.protocol_version == crate::LEGACY_PROTOCOL_VERSION =>
            {
                Err(EnvelopeVersionCompatibilityError::BodyRequiresV2 {
                    body: V2OnlyEventBody::ForeignScanCoverage,
                })
            }
            // Every version this legacy check admits is below v5.
            EventBody::Capture { .. } => {
                Err(EnvelopeVersionCompatibilityError::CaptureRequiresV5 {
                    protocol_version: self.protocol_version,
                })
            }
            EventBody::Redaction(_)
                if self.protocol_version >= crate::INLINE_REDACTION_PROTOCOL_VERSION =>
            {
                Err(
                    EnvelopeVersionCompatibilityError::LegacyRedactionRequiresV1 {
                        protocol_version: self.protocol_version,
                    },
                )
            }
            _ => Ok(()),
        }
    }

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

/// Closed by design at eighteen variants: the daemon interprets nothing (D3). Every variant is
/// either raw bytes, a comparison against a manifest constant, or a measurement the daemon
/// took of the filesystem or of itself. The eighteenth, `Capture`, is protocol v5 only.
///
/// **`HomeSeeded` is additive.** Variants appended
/// to an adjacently tagged enum change no other variant's bytes, so every chain written
/// before it hashes exactly as it did, and a trace that carries none is an older chain rather
/// than a run that seeded nothing — the same reading `Leg2Post::path`'s absence has.
///
/// Adjacently tagged (`tag = "kind"`, `content = "content"`) rather than internally
/// tagged: `TailNotice`'s own field is also named `kind` (`TailNoticeKind`), and serde
/// refuses to compile an internally tagged enum whose variant field name collides with
/// the tag. Adjacent tagging keeps the variant's fields in a nested `content` object, so
/// the outer `"kind":"tail_notice"` and the nested `"kind":"file_replaced"` never share a
/// JSON object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "content", rename_all = "snake_case")]
pub enum EventBody {
    /// One record the harness persisted. `payload` is a JSON **string** holding the
    /// harness's raw line for a line-oriented store, and a JSON **object** holding the
    /// row for a SQLite store. There is no `{path,text}` wrapper anywhere.
    Leg1Record {
        store: String,
        payload: serde_json::Value,
    },
    Leg2Post {
        event: String,
        payload: serde_json::Value,
        /// The receiver PATH this post arrived on, verbatim, always recorded. Additive,
        /// `serde(default)`, and skipped when absent, so every chain written before this
        /// field hashes exactly as it did — which is the only thing `None` means.
        ///
        /// **It is a harness-chosen string.** Whatever posts to the receiver picks it, and
        /// the receiver takes the raw URI path with no decoding step in between; its length
        /// is bounded by the HTTP stack's request head and by nothing this protocol
        /// chooses. Nothing reads it as anything but a discriminator —
        /// `goalcup_verifier::legs::Leg2Post::transport` asks whether it ends in `-cmd` —
        /// and no surface interpolates it. A v2 writer includes it in the complete compact
        /// `EventBody::Leg2Post` byte vector passed through the redactor once.
        ///
        /// It exists because a harness may carry ONE submission over two hook sources, and
        /// the only thing that tells that apart from the same post arriving twice on one
        /// source is which source carried it. Claude Code does exactly that: the launch
        /// settings register an `http` hook at `/UserPromptSubmit` and the project settings
        /// a `curl` shim at `/UserPromptSubmit-cmd`, both carrying `X-GoalCup-Event:
        /// UserPromptSubmit`. The daemon chains the header's name — which is right, because
        /// that is the event — so without the path `goalcup_verifier::legs::Leg2Post::transport`
        /// would read both copies as `http` and the second would become a `duplicate_pairing`
        /// Fail on an honest run.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// What the RECEIVER measured about this post. Additive,
        /// `serde(default)`, skipped when absent, so every chain written before it hashes
        /// exactly as it did — `the_new_variant_did_not_move_an_older_body_s_chain_bytes`
        /// pins those bytes.
        ///
        /// D24 says the bearer defends against accident and cross-talk and not against a
        /// determined participant, and names three narrow measures that sit behind it. Two
        /// of them are measurements the receiver takes per post — a monotonic per-source
        /// counter, so a suppressed post leaves a gap and a replayed one leaves a
        /// duplicate, and a freshness window — and this field carries them into the chain
        /// instead of dropping them when the post arrives. A participant who suppressed a
        /// leg-2 post to hide a human message would otherwise leave a counter gap nobody
        /// could ever see.
        ///
        /// **An untyped `Value`, on purpose.** This is the daemon's `receiver::PostMeta`
        /// serialised as it stands: the counter it read, the source name the post claimed,
        /// and the anomalies the receiver observed. The daemon makes decisions from that
        /// raw typed value, then persists its value-preserving redacted representation.
        /// Freezing a shape for it would commit this crate to a schema for a check that does
        /// not exist. **Nothing reads it** — the two-leg reconciliation (V2/V3) is what
        /// catches a suppressed post, as an unwitnessed leg-1 record. It is evidence kept in
        /// the trace rather than absent from it.
        ///
        /// The header-chosen strings pass through the same complete-body redaction as the
        /// event, path and payload. The daemon reconstructs `receiver::PostMeta` before
        /// appending, while this protocol field stays deliberately untyped. The bearer is
        /// never in it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        receiver_meta: Option<serde_json::Value>,
    },
    ConfigDeviation(ConfigDeviation),
    VersionChange(VersionChange),
    Health(HealthEvent),
    /// Legacy v1 evidence for the record at `target_seq`. New v2 writers carry
    /// `Envelope::redaction` inline, and the verifier rejects this body under v2.
    Redaction(RedactionRecord),
    /// The two on-disk signatures of a rewritten transcript, chained rather than
    /// discarded. `offset_before`/`offset_after` and the two inode numbers are what let
    /// V9 distinguish an ordinary resume re-read from a truncation nothing explains.
    TailNotice {
        store: String,
        path: String,
        kind: TailNoticeKind,
        offset_before: u64,
        offset_after: u64,
        inode_before: Option<u64>,
        inode_after: Option<u64>,
        at: String,
    },
    /// A session belonging to another installed harness whose directory is a bound
    /// workspace. Identifiers and paths only — never content (D20).
    ForeignSession {
        harness: String,
        session_id: String,
        store_path: String,
        first_seen: String,
    },
    /// Whether one declaration in the installed, pinned connector union was inspected
    /// completely. Its identity is the declaring manifest bytes plus their stable row
    /// number; the outcome carries no participant content or path.
    ForeignScanCoverage {
        declaring_connector: String,
        declaring_manifest_sha256: String,
        scan_index: u64,
        outcome: ForeignScanOutcome,
        at: String,
    },
    /// Proof of life. Absence of heartbeats over a window is what makes downtime a
    /// positive statement in the verdict rather than an absence.
    Heartbeat {
        ts: String,
        stores_seen: Vec<StoreCursor>,
        receiver_ok: bool,
    },
    /// Written on start when a chain already exists, naming the head it found.
    DaemonRestart {
        at: String,
        previous_head_seq: u64,
        previous_head_hash: String,
    },
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
    /// Written by `TraceExport::write_baseline` and by nothing else.
    Baseline {
        manifest_sha256: String,
        file_count: u64,
    },
    /// The entry's seal, in the chain. Carries the *digest* of `seal.json` rather than
    /// the manifest itself: the manifest is already on disk, and a chain that repeated
    /// it would hand a reviewer two copies to compare instead of one claim to check.
    /// Written by `TraceExport::write_seal` and by nothing else — no daemon builds this
    /// variant by hand, which is what makes "the seal is the last link" a property of
    /// the writer rather than a convention.
    Seal {
        manifest_sha256: String,
        sealed_at: String,
        file_count: u64,
    },
    /// Which of the participant's own harness settings this launch's `[home_seed]` carried
    /// into the per-run home. Raw: no check reads it and no finding rests on
    /// it. Written once per launch — the start and each resume both seed — and built by the
    /// CLI, which is the half that does the seeding; see
    /// [`crate::events::HomeSeedRecord`] for what it may and may not carry.
    ///
    /// A writer that adds a future body also increments `protocol_version`. A verifier
    /// shallow-decodes that version before it asks serde to interpret this closed enum, then
    /// stops at the unsupported boundary; a body unknown under a *supported* version remains
    /// malformed evidence rather than being guessed at.
    HomeSeeded(crate::events::HomeSeedRecord),
    /// A bounded repository state observation. Its time and prior hash are the envelope's.
    RepositoryObservedV1(RepositoryObservedV1),
    /// The once-only local cutoff boundary. Written only by the export's cutoff method.
    CutoffReachedV1(CutoffReachedV1),
    /// The terminal capture/publication result. Written only by the export's finalization method.
    FinalizationV1(FinalizationV1),
    /// One M2 recorder observation. Protocol v5 only: a version 1–4 envelope
    /// carrying it is `CaptureRequiresV5`, and the legacy writer, export reader and semantic
    /// verifier refuse every v5 envelope as an unsupported version. `CaptureRecord::LocalFinish`
    /// is reserved to the export's terminal writer, never an ordinary append.
    Capture {
        record: crate::capture::CaptureRecord,
    },
}
