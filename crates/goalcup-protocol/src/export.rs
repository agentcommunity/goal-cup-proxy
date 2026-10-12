//! The one writer of a trace directory.
//!
//! Seven files make up an entry's evidence — `trace.toml`, `chain.jsonl`, `acks.jsonl`,
//! `receipt.json`, `baseline.json`, `seal.json`, and the copied `manifest.toml` — and this
//! module owns their bytes. They are written here, in the crate that defines what is in
//! them, so the daemon and verifier cannot disagree about a format neither of them owns.
//!
//! **The daemon never computes a hash.** `append_chain` takes the envelope's content and
//! fills `seq`, `prev` and `hash` itself from its own state: a caller that hands over a
//! link gets it discarded (or refused, when it names a position that is not the next
//! one), so "the chain is contiguous from 0" is a property of this writer rather than a
//! rule every call site has to remember.
//!
//! **`chain::Chain` is the M0 formula and is deliberately not used here; envelopes hash
//! through `envelope.rs`.** `Chain::try_append` pre-increments, so its first link is `seq`
//! 1, which `tests/it/chain_golden.rs` pins. The entry chain starts at 0, and `seq` is inside
//! `Envelope::chain_bytes`, so the base is part of what is hashed and the two sequences
//! must not share an implementation. This module calls `link_hash` directly with its own
//! 0-based counter.
//!
//! **Some writers here have no production caller, on purpose.** The capture runtime writes
//! a trace with `write_baseline_for_protocol`, `append_chain` and `write_local_finish`.
//! The V1 writers — `write_cutoff`, `write_finalization`, `write_descriptor`,
//! `write_receipt`, `write_manifest`, `write_seal` and their `_for_protocol` forms — lost
//! their daemon caller in D48. The receipt, seal, cutoff and finalization writers stay because
//! the verifier's v5 tests build signed and sealed fixture traces with them; `write_descriptor`
//! and `write_manifest` are read by nothing since the pre-v5 verifier was removed (`TODO.md`).

use crate::ProtocolError;
use crate::ack::AckCheckpoint;
use crate::capture::{
    CaptureLimitError, CaptureRecord, CaptureStorage, CaptureTime, ConnectionEvent,
    DurableFrontier, M2_PROTOCOL_VERSION, MAX_ENVELOPE_BYTES, MAX_METADATA_STRING_BYTES,
    TERMINAL_RESERVE_BYTES,
};
use crate::chain::{GENESIS, Link, link_hash};
use crate::envelope::{Envelope, EnvelopeVersionCompatibilityError, EventBody};
use crate::provenance::{
    CutoffReachedV1, FinalizationV1, ProvenanceError, Rfc3339Utc, Sha256Digest,
    TerminationOutcomeV1,
};
use crate::receipt::StartReceipt;
use crate::seal::SealManifest;
use crate::submission::{SubmissionManifestV1, SubmissionOutcomeV1};
use goalcup_platform::tracecap::{
    FixedDocument, TraceAppendFile, TraceCapError, TraceDir, TraceReadFile,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

/// The daemon's chained envelope stream: one `Envelope` per line, `seq` from 0.
pub const CHAIN_FILE: &str = "chain.jsonl";
/// The acknowledgement stream: one `AckCheckpoint` per line.
pub const ACKS_FILE: &str = "acks.jsonl";
pub const RECEIPT_FILE: &str = "receipt.json";
pub const BASELINE_FILE: &str = "baseline.json";
pub const SEAL_FILE: &str = "seal.json";
pub const DESCRIPTOR_FILE: &str = "trace.toml";
/// The connector manifest, under the name it carries in the connector directory. The one
/// file a verifier reads whose bytes this crate does not author — it is copied in by
/// [`TraceExport::write_manifest`], and `TraceDescriptorDoc::connector_manifest` names it,
/// so the digest pin is the digest of a file the trace directory carries with it.
pub const MANIFEST_FILE: &str = "manifest.toml";
/// The fixed daemon-owned root for a captured V4 workspace submission.
pub const SUBMISSION_DIR: &str = "submission";
/// The canonical submission manifest, present only for a captured V4 workspace submission.
pub const SUBMISSION_MANIFEST_FILE: &str = "submission/manifest.json";
const SUBMISSION_MANIFEST_NAME: &str = "manifest.json";
/// Content-addressed immutable object storage beneath [`SUBMISSION_DIR`].
pub const SUBMISSION_SHA256_OBJECTS_DIR: &str = "submission/objects/sha256";

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("chain sequence space is exhausted")]
    SequenceExhausted,
    #[error("io {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("serialising {what}: {source}")]
    Serialise {
        what: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("deserialising {what}: {source}")]
    Deserialise {
        what: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("serialising trace.toml: {source}")]
    Toml {
        #[source]
        source: toml::ser::Error,
    },
    #[error("protocol: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("provenance document: {0}")]
    Provenance(#[from] ProvenanceError),
    #[error("trace capability: {0}")]
    Capability(#[from] TraceCapError),
    #[error(
        "envelope seq {found} does not continue the chain ({}): chain.jsonl is append-only and contiguous",
        last_link(.expected_after)
    )]
    OutOfOrder {
        found: u64,
        /// The seq of the last link written, or `None` when nothing is chained yet —
        /// the two states `head()` distinguishes, and for the same reason.
        expected_after: Option<u64>,
    },
    #[error("this export already chained a seal at seq {seq}")]
    SealAlreadyChained { seq: u64 },
    #[error(
        "the chain is already at seq {head_seq}: a baseline is the FIRST link or it is not \
         a baseline, because a manifest taken after the run started describes a workspace \
         the agent has been writing to"
    )]
    BaselineNotFirst { head_seq: u64 },
    #[error("this export already chained a baseline; there is one, and it is seq 0")]
    BaselineAlreadyChained,
    #[error("the chain is sealed at seq {seal_seq}: no record may follow the seal")]
    Sealed { seal_seq: u64 },
    #[error("chain.jsonl line {seq}: {reason}")]
    ChainCorrupt { seq: u64, reason: String },
    #[error("chain.jsonl seq {seq} declares unsupported protocol version {protocol_version}")]
    UnsupportedProtocolVersion { seq: u64, protocol_version: u32 },
    #[error("the envelope version and body are incompatible: {reason}")]
    IncompatibleEnvelope {
        reason: EnvelopeVersionCompatibilityError,
    },
    #[error("a parsed legacy HomeSeeded layout is historical evidence and cannot be appended")]
    LegacyHomeSeededAppend,
    #[error("chain.jsonl already exists and is not empty: resume it rather than creating over it")]
    ChainExists,
    #[error(
        "acks.jsonl already exists and is not empty: a fresh export cannot adopt orphan acknowledgements"
    )]
    AcksExist,
    #[error("{name} already exists: fixed trace documents are absent-only")]
    FixedDocumentExists { name: &'static str },
    #[error(
        "a {kind} is chained by write_{kind} and by nothing else: append_chain does not take \
         one, because the position and once-only rules that make a {kind} mean anything are \
         enforced there and a hand-assembled body would carry none of them"
    )]
    ReservedBody { kind: &'static str },
    #[error("the chain reached cutoff at seq {cutoff_seq}: only its bound finalization may follow")]
    AfterCutoff { cutoff_seq: u64 },
    #[error("this export already chained a cutoff at seq {seq}")]
    CutoffAlreadyChained { seq: u64 },
    #[error("a finalization requires a preceding cutoff")]
    FinalizationBeforeCutoff,
    #[error("this export already chained a finalization at seq {seq}")]
    FinalizationAlreadyChained { seq: u64 },
    #[error("the finalization or captured submission manifest does not match the cutoff")]
    FinalizationBindingMismatch,
    #[error("the chain is terminal at seq {terminal_seq}: no chain record may follow")]
    Terminal { terminal_seq: u64 },
    #[error(
        "chain.jsonl length changed after validation: validated {validated_bytes} bytes, found {found_bytes}"
    )]
    ChainLengthChanged {
        validated_bytes: u64,
        found_bytes: u64,
    },
    #[error("capture chain seq {seq}: {reason}")]
    InvalidCaptureEnvelope { seq: u64, reason: &'static str },
    #[error(
        "a protocol v5 envelope serializes to {bytes} bytes with its newline, over the {limit}-byte bound"
    )]
    EnvelopeTooLarge { bytes: u64, limit: u64 },
    #[error("a {bytes}-byte capture quota is refused: {reason}")]
    InvalidCaptureQuota { bytes: u64, reason: &'static str },
    #[error("this capture chain has no quota configured, so nothing may be appended to it")]
    CaptureQuotaNotConfigured,
    #[error(
        "the capture quota is spent: a {needed}-byte record does not fit the {available} bytes \
         left in the {} portion",
        quota_portion(.terminal_allowance)
    )]
    CaptureStorageFull {
        needed: u64,
        available: u64,
        /// Whether the refused record was a bounded closure spending the protected terminal
        /// allowance (loss can then no longer be accounted) or ordinary payload.
        terminal_allowance: bool,
    },
    #[error("capture record: {0}")]
    CaptureRecord(#[from] CaptureLimitError),
    #[error("this export already chained a local finish at seq {seq}")]
    LocalFinishAlreadyChained { seq: u64 },
    #[error("the fixed manifest does not match the terminal record's binding")]
    TerminalManifestMismatch,
}

/// The two fields a resume may inspect before attempting to decode the body. A new body kind
/// must arrive under a new protocol version; decoding the closed `EventBody` enum first would
/// turn that honest boundary into a parse failure and tempt a reader to retain guessed data.
#[derive(Deserialize)]
struct EnvelopeVersionSeq {
    protocol_version: u32,
    seq: u64,
}

fn version_body_export_error(seq: u64, error: EnvelopeVersionCompatibilityError) -> ExportError {
    match error {
        EnvelopeVersionCompatibilityError::UnsupportedProtocolVersion { protocol_version } => {
            ExportError::UnsupportedProtocolVersion {
                seq,
                protocol_version,
            }
        }
        reason => ExportError::IncompatibleEnvelope { reason },
    }
}

/// The parenthetical in an `OutOfOrder` message: names the last link written, or says
/// there is not one, rather than printing a `None` at a reader.
fn last_link(expected_after: &Option<u64>) -> String {
    match expected_after {
        Some(seq) => format!("the last link is {seq}"),
        None => "nothing is chained yet".to_string(),
    }
}

fn quota_portion(terminal_allowance: &bool) -> &'static str {
    if *terminal_allowance {
        "terminal"
    } else {
        "payload"
    }
}

fn resumed_phase_after(phase: &ExportPhase, env: &Envelope) -> Result<ExportPhase, String> {
    match &env.body {
        EventBody::Capture {
            record:
                CaptureRecord::LocalFinish {
                    observed,
                    reason,
                    termination,
                    seal_manifest_sha256,
                },
        } => {
            validate_new_phase_envelope(env)?;
            if env.protocol_version != M2_PROTOCOL_VERSION {
                return Err("a local finish is protocol v5 evidence only".to_string());
            }
            if env.recorded_at != observed.wall {
                return Err(
                    "a local finish envelope time differs from its own observation".to_string(),
                );
            }
            validate_local_finish(observed, reason, termination)
                .map_err(|_| "local finish fields are invalid".to_string())?;
            Sha256Digest::try_from(seal_manifest_sha256.as_str())
                .map_err(|_| "local finish manifest digest is not lowercase SHA-256".to_string())?;
            match phase {
                ExportPhase::Open => Ok(ExportPhase::Terminal {
                    seq: env.link.seq,
                    link: env.link.clone(),
                    kind: TerminalKind::LocalFinish,
                }),
                ExportPhase::Cutoff { seq, .. } => {
                    Err(format!("a local finish follows cutoff seq {seq}"))
                }
                ExportPhase::Terminal { seq, .. } => {
                    Err(format!("a local finish follows terminal seq {seq}"))
                }
            }
        }
        EventBody::CutoffReachedV1(cutoff) => {
            validate_new_phase_envelope(env)?;
            cutoff
                .validate()
                .map_err(|_| "cutoff document is invalid".to_string())?;
            match phase {
                ExportPhase::Open => Ok(ExportPhase::Cutoff {
                    seq: env.link.seq,
                    link: env.link.clone(),
                    entry_id: env.entry_id.clone(),
                    receipt_sha256: cutoff.receipt_sha256.clone(),
                }),
                ExportPhase::Cutoff { seq, .. } => {
                    Err(format!("a second cutoff; the first is at seq {seq}"))
                }
                ExportPhase::Terminal { seq, .. } => {
                    Err(format!("a cutoff follows terminal seq {seq}"))
                }
            }
        }
        EventBody::FinalizationV1(finalization) => {
            validate_new_phase_envelope(env)?;
            finalization
                .validate()
                .map_err(|_| "finalization document is invalid".to_string())?;
            match phase {
                ExportPhase::Open => Err("a finalization appears before cutoff".to_string()),
                ExportPhase::Cutoff {
                    entry_id,
                    receipt_sha256,
                    ..
                } if entry_id == &env.entry_id
                    && receipt_sha256 == finalization.receipt_sha256() =>
                {
                    Ok(ExportPhase::Terminal {
                        seq: env.link.seq,
                        link: env.link.clone(),
                        kind: TerminalKind::FinalizationV1,
                    })
                }
                ExportPhase::Cutoff { .. } => {
                    Err("finalization does not match the cutoff binding".to_string())
                }
                ExportPhase::Terminal { seq, .. } => {
                    Err(format!("a finalization follows terminal seq {seq}"))
                }
            }
        }
        EventBody::Seal { .. } => match phase {
            ExportPhase::Open => Ok(ExportPhase::Terminal {
                seq: env.link.seq,
                link: env.link.clone(),
                kind: TerminalKind::LegacySeal,
            }),
            ExportPhase::Cutoff { seq, .. } => {
                Err(format!("a legacy seal follows cutoff seq {seq}"))
            }
            ExportPhase::Terminal { seq, .. } => {
                Err(format!("a legacy seal follows terminal seq {seq}"))
            }
        },
        _ => match phase {
            ExportPhase::Open => Ok(ExportPhase::Open),
            ExportPhase::Cutoff { seq, .. } => {
                Err(format!("an ordinary record follows cutoff seq {seq}"))
            }
            ExportPhase::Terminal { seq, .. } => {
                Err(format!("a record follows terminal seq {seq}"))
            }
        },
    }
}

fn validate_new_phase_envelope(env: &Envelope) -> Result<(), String> {
    let known = crate::is_supported_protocol_version(env.protocol_version)
        || env.protocol_version == M2_PROTOCOL_VERSION;
    if !known
        || env.protocol_version < crate::INLINE_REDACTION_PROTOCOL_VERSION
        || env.source != "daemon"
        || env.redaction.is_some()
        || Rfc3339Utc::try_from(env.recorded_at.as_str()).is_err()
        || env.entry_id.is_empty()
        || env.entry_id.len() > 256
        || env.entry_id.bytes().any(|byte| byte.is_ascii_control())
    {
        Err(
            "a cutoff, finalization or local finish must be a modern daemon envelope without redaction"
                .to_string(),
        )
    } else {
        Ok(())
    }
}

/// Which wire contract a chain's records follow. The first record decides, and a chain never
/// changes format: a capture chain carries protocol v5 only, a legacy chain versions 1–4 only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainFormat {
    /// Protocol versions 1–4, admitted by the legacy version/body check.
    Legacy,
    /// Protocol v5 (M2 capture): every record, baseline and terminal included, is v5.
    Capture,
}

const LEGACY_IN_CAPTURE_CHAIN: &str = "a capture chain carries only protocol v5 envelopes";
/// Every v5 entry has a v5 baseline, and a baseline is only ever the genesis.
const CAPTURE_GENESIS_NOT_BASELINE: &str = "a capture chain begins with its v5 baseline";

/// The v5 body matrix. A capture chain holds its baseline, capture records and exactly the
/// terminal kinds this writer can close it with; legacy leg/health bodies are not v5 evidence.
/// Redaction evidence lives inside a capture `Segment`, never on the envelope.
fn validate_capture_envelope(env: &Envelope) -> Result<(), ExportError> {
    let seq = env.link.seq;
    if env.redaction.is_some() {
        return Err(ExportError::InvalidCaptureEnvelope {
            seq,
            reason: "a v5 envelope carries redaction evidence only inside its capture record",
        });
    }
    match env.body {
        EventBody::Capture { .. }
        | EventBody::Baseline { .. }
        | EventBody::Seal { .. }
        | EventBody::CutoffReachedV1(_)
        | EventBody::FinalizationV1(_) => Ok(()),
        _ => Err(ExportError::InvalidCaptureEnvelope {
            seq,
            reason: "this body is not part of protocol v5",
        }),
    }
}

/// Version/body admission of `env` into a chain of `format` (`None`: the chain is empty and
/// its first record decides). v5 is admitted only into a capture chain; the legacy check,
/// which refuses v5 as an unsupported version, still judges every legacy record.
fn admit_for_format(format: Option<ChainFormat>, env: &Envelope) -> Result<(), ExportError> {
    let seq = env.link.seq;
    check_version_for_format(format, seq, env.protocol_version)?;
    if env.protocol_version == M2_PROTOCOL_VERSION {
        validate_capture_envelope(env)
    } else {
        env.validate_version_body_compatibility()
            .map_err(|error| version_body_export_error(seq, error))
    }
}

fn check_version_for_format(
    format: Option<ChainFormat>,
    seq: u64,
    protocol_version: u32,
) -> Result<(), ExportError> {
    let v5 = protocol_version == M2_PROTOCOL_VERSION;
    let legacy = crate::is_supported_protocol_version(protocol_version);
    match format {
        Some(ChainFormat::Capture) | None if v5 => Ok(()),
        Some(ChainFormat::Capture) if legacy => Err(ExportError::InvalidCaptureEnvelope {
            seq,
            reason: LEGACY_IN_CAPTURE_CHAIN,
        }),
        Some(ChainFormat::Legacy) | None if legacy => Ok(()),
        _ => Err(ExportError::UnsupportedProtocolVersion {
            seq,
            protocol_version,
        }),
    }
}

/// The local terminal's own fields: a valid observation, a bounded non-control reason and a
/// typed termination whose time ordering passes the existing validator.
fn validate_local_finish(
    observed: &CaptureTime,
    reason: &str,
    termination: &TerminationOutcomeV1,
) -> Result<(), ExportError> {
    observed.validate()?;
    if reason.is_empty() {
        return Err(CaptureLimitError::Invalid {
            field: "reason",
            reason: "must not be empty",
        }
        .into());
    }
    if reason.len() > MAX_METADATA_STRING_BYTES {
        return Err(CaptureLimitError::TooLong {
            field: "reason",
            len: reason.len(),
            limit: MAX_METADATA_STRING_BYTES,
        }
        .into());
    }
    if reason.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(CaptureLimitError::Invalid {
            field: "reason",
            reason: "must not contain control characters",
        }
        .into());
    }
    termination.validate()?;
    Ok(())
}

/// Whether a capture-chain record is bounded accounting that may spend the protected terminal
/// allowance: loss gaps, stream/connection/producer closures, assistance and the terminal
/// records. A closure is bounded by what it closes (opens are payload) and assistance is rare;
/// gaps are coalesced by the entry owner, and the allowance itself is finite, so even gaps stop
/// at the quota.
fn spends_terminal_allowance(body: &EventBody) -> bool {
    match body {
        EventBody::Seal { .. } | EventBody::CutoffReachedV1(_) | EventBody::FinalizationV1(_) => {
            true
        }
        EventBody::Capture { record } => matches!(
            record,
            CaptureRecord::Gap { .. }
                | CaptureRecord::End { .. }
                | CaptureRecord::ProducerClosed { .. }
                | CaptureRecord::Assistance { .. }
                | CaptureRecord::LocalFinish { .. }
                | CaptureRecord::Connection {
                    event: ConnectionEvent::Close { .. },
                    ..
                }
        ),
        _ => false,
    }
}

/// The terminal record a chain was closed by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainTerminal {
    pub seq: u64,
    pub hash: String,
    pub kind: TerminalKind,
}

/// Replays the phase rules this writer enforces, one envelope at a time, over a chain whose
/// links the caller has already verified (contiguous `seq` from 0, `prev`, recomputed hash).
///
/// It is the one implementation of the terminal rules: [`TraceExport`]'s resume runs every
/// line through it, and an independent reader (the M2 integrity reader) applies the same
/// rules read-only instead of re-deriving them. It checks, in order: version admission for
/// the chain's format (decided by the first record; see [`ChainFormat`]), the version/body
/// matrix, baseline-first and once-only (a capture chain's genesis must be its v5
/// baseline), and the phase machine — a capture record never
/// follows a cutoff or a terminal; a cutoff is followed only by its bound finalization;
/// exactly one terminal (legacy seal, finalization or v5 local finish) closes the chain.
/// A local finish must be v5, carry its own observation as the envelope time, valid fields,
/// a typed termination with ordered times and a lowercase SHA-256 manifest digest.
///
/// It is pure: fixed files are not read here. Use [`check_terminal_manifest`] to bind the
/// terminal to its `seal.json`. It holds constant state, never the records it has seen.
#[derive(Debug, Clone, Default)]
pub struct ChainPhaseReplay {
    format: Option<ChainFormat>,
    phase: ExportPhase,
    baseline_seq: Option<u64>,
}

impl ChainPhaseReplay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Admit a line's declared version *before* its body is decoded, so a future version's
    /// unknown body is reported as an unsupported version rather than a parse failure.
    pub fn check_version(&self, seq: u64, protocol_version: u32) -> Result<(), ExportError> {
        check_version_for_format(self.format, seq, protocol_version)
    }

    /// The version/body admission alone, without advancing the replay.
    pub(crate) fn check_envelope(&self, env: &Envelope) -> Result<(), ExportError> {
        admit_for_format(self.format, env)
    }

    /// Admit the next already link-verified envelope. A refusal names the envelope's `seq`;
    /// phase and baseline violations are [`ExportError::ChainCorrupt`].
    pub fn push(&mut self, env: &Envelope) -> Result<(), ExportError> {
        self.check_envelope(env)?;
        let seq = env.link.seq;
        let corrupt = |reason: String| ExportError::ChainCorrupt { seq, reason };
        // The baseline is recovered exactly as the seal is, and for the same reason: the
        // once-only rule is a property of the CHAIN, not of the process that wrote it. A
        // second baseline line is a chain this writer will not extend — two answers to
        // "what did the workspace hold at t0" is one a reviewer would have to choose between.
        let baseline = matches!(env.body, EventBody::Baseline { .. });
        if self.format.is_none() && env.protocol_version == M2_PROTOCOL_VERSION && !baseline {
            return Err(ExportError::InvalidCaptureEnvelope {
                seq,
                reason: CAPTURE_GENESIS_NOT_BASELINE,
            });
        }
        if baseline {
            if let Some(first) = self.baseline_seq {
                return Err(corrupt(format!(
                    "a second baseline; the chain already carries one at seq {first}"
                )));
            }
            if seq != 0 || !matches!(self.phase, ExportPhase::Open) {
                return Err(corrupt(
                    "a baseline is permitted only as the first open-chain record".to_string(),
                ));
            }
        }
        let phase = resumed_phase_after(&self.phase, env).map_err(corrupt)?;
        if baseline {
            self.baseline_seq = Some(seq);
        }
        if self.format.is_none() {
            self.format = Some(if env.protocol_version == M2_PROTOCOL_VERSION {
                ChainFormat::Capture
            } else {
                ChainFormat::Legacy
            });
        }
        self.phase = phase;
        Ok(())
    }

    /// The chain's format, once its first record has been admitted.
    pub fn format(&self) -> Option<ChainFormat> {
        self.format
    }

    pub fn baseline_seq(&self) -> Option<u64> {
        self.baseline_seq
    }

    /// The seq of an admitted cutoff that no finalization has closed yet.
    pub fn cutoff_seq(&self) -> Option<u64> {
        match self.phase {
            ExportPhase::Cutoff { seq, .. } => Some(seq),
            _ => None,
        }
    }

    /// The terminal that closed the chain, or `None` for an unsealed chain.
    pub fn terminal(&self) -> Option<ChainTerminal> {
        match &self.phase {
            ExportPhase::Terminal { seq, link, kind } => Some(ChainTerminal {
                seq: *seq,
                hash: link.hash.clone(),
                kind: *kind,
            }),
            _ => None,
        }
    }
}

/// Whether a terminal envelope binds `canonical` (already sorted): a legacy seal binds the
/// digest, time, file count and entry; a local finish binds the digest of the canonical
/// manifest, which covers all of them, and the entry. Other bodies bind no fixed manifest.
fn terminal_binds_manifest(
    terminal: &Envelope,
    canonical: &SealManifest,
) -> Result<bool, ExportError> {
    let digest = manifest_sha256(canonical)?;
    Ok(match &terminal.body {
        EventBody::Seal {
            manifest_sha256: claimed,
            sealed_at,
            file_count,
        } => {
            terminal.entry_id == canonical.entry_id
                && terminal.recorded_at == canonical.sealed_at
                && sealed_at == &canonical.sealed_at
                && *file_count == canonical.files.len() as u64
                && claimed == &digest
        }
        EventBody::Capture {
            record:
                CaptureRecord::LocalFinish {
                    seal_manifest_sha256,
                    ..
                },
        } => terminal.entry_id == canonical.entry_id && seal_manifest_sha256 == &digest,
        _ => false,
    })
}

/// Check that a terminal record binds the fixed manifest read from `seal.json`. A valid
/// terminal link whose fixed manifest is missing or does not match is not complete integrity;
/// this is the pure comparison, the bounded file read is the caller's.
pub fn check_terminal_manifest(
    terminal: &Envelope,
    manifest: &SealManifest,
) -> Result<(), ExportError> {
    if terminal_binds_manifest(terminal, &manifest.clone().sorted())? {
        Ok(())
    } else {
        Err(ExportError::TerminalManifestMismatch)
    }
}

fn validate_phase_inputs(entry_id: &str, recorded_at: &str) -> Result<(), ProvenanceError> {
    if entry_id.is_empty()
        || entry_id.len() > 256
        || entry_id.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(ProvenanceError::InvalidField {
            field: "entry_id",
            reason: "must be bounded non-control text",
        });
    }
    Rfc3339Utc::try_from(recorded_at).map(|_| ())
}

/// The `trace.toml` document, serialisable. The verifier's `TraceDescriptor`
/// deserialises this exact shape; `crates/goalcup-verifier/tests/it/trace_export_roundtrip.rs` binds
/// the two together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceDescriptorDoc {
    pub connector: String,
    pub entry_id: String,
    pub workspace: String,
    pub tracked_sessions: Vec<String>,
    pub harness_version: String,
    /// The connector manifest, **relative to the trace root** — [`MANIFEST_FILE`] for a
    /// trace this writer produced, because [`TraceExport::write_manifest`] is what put
    /// the file there. A descriptor naming a path outside the directory is a trace that
    /// stops verifying the moment it is copied.
    pub connector_manifest: String,
    /// Always `Some("chain.jsonl")` for a trace this writer produced.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub chain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub receipt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub baseline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub acks: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub seal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub server_public_key: Option<String>,
    /// A re-read from offset 0 is expected on this trace. Written unconditionally (no
    /// `skip_serializing_if`), so the document always says which reading it asked for; the
    /// verifier honours it only with a lower-seq witness.
    #[serde(default)]
    pub expected_reread: bool,
}

/// A fixed document that a restart/rebuild consumer may inspect through a held root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixedTraceDocument {
    Receipt,
    Baseline,
    Seal,
    Manifest,
}

impl FixedTraceDocument {
    fn platform(self) -> FixedDocument {
        match self {
            Self::Receipt => FixedDocument::Receipt,
            Self::Baseline => FixedDocument::Baseline,
            Self::Seal => FixedDocument::Seal,
            Self::Manifest => FixedDocument::Manifest,
        }
    }
}

/// A protocol-owned handle to one acquired trace root.
pub struct TraceRoot {
    dir: TraceDir,
    display: PathBuf,
}

impl fmt::Debug for TraceRoot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TraceRoot")
            .field("display", &self.display)
            .finish_non_exhaustive()
    }
}

impl TraceRoot {
    /// Acquire an existing trace directory once. The path retained here is diagnostic;
    /// subsequent reads and writes are rooted at `dir`'s owned native handle.
    pub fn open(path: &Path) -> Result<Self, ExportError> {
        Ok(Self {
            dir: TraceDir::open(path)?,
            display: path.to_path_buf(),
        })
    }

    fn create(path: &Path) -> Result<Self, ExportError> {
        Ok(Self {
            dir: TraceDir::create(path)?,
            display: path.to_path_buf(),
        })
    }

    /// Construct a protocol trace root from a directory capability acquired by the daemon's
    /// artifact-root authority. `display` is diagnostics only; every later I/O remains
    /// relative to `dir`.
    pub fn from_acquired(dir: TraceDir, display: PathBuf) -> Self {
        Self { dir, display }
    }

    /// Open one fixed document as a checked read-only handle.
    pub fn read_fixed(
        &self,
        document: FixedTraceDocument,
    ) -> Result<Option<TraceReadFile>, ExportError> {
        Ok(self.dir.read_fixed(document.platform())?)
    }
}

/// One entry's on-disk export. The native directory handle, rather than `root()`'s
/// diagnostic path, owns every file operation.
#[derive(Debug)]
pub struct TraceExport {
    root: TraceRoot,
    chain: TraceAppendFile,
    acks: TraceAppendFile,
    /// Logical `chain.jsonl` bytes this writer validated or appended: the used-bytes figure
    /// quota admission is measured against.
    chain_bytes: u64,
    /// This is a protocol-v5 capture chain: selected by [`Self::configure_capture_quota`] or
    /// [`Self::reserve_capture`] on an empty chain, or recovered from a v5 first record. It
    /// admits v5 records only.
    capture_chain: bool,
    /// The serialized chain-byte quota, [`TERMINAL_RESERVE_BYTES`] included. Admission
    /// accounting only: by itself it allocates nothing and is never reported as reserved.
    capture_quota: Option<u64>,
    /// The part of `capture_quota` this writer measured as physically allocated in the chain
    /// file ([`Self::reserve_capture`]). Zero for a quota-only chain and after every reopen,
    /// until the retained allocation is remeasured.
    capture_reserved: u64,
    /// The last frontier a successful flush made durable. Never the appended head.
    durable: Option<DurableFrontier>,
    /// False once a byte count may be wrong (a failed append) or a closure could not be
    /// recorded (terminal allowance spent). Sticky.
    accounting_available: bool,
    /// The seq the next link will carry. 0 on a fresh export, so the first link is 0.
    /// Doubles as the count of links written, which is what lets `head()` tell an empty
    /// chain apart from a chain whose head is link 0.
    next_seq: u64,
    /// The head hash: `GENESIS` until something is appended.
    head: String,
    /// The execution phase reconstructed from the chain itself.
    phase: ExportPhase,
    /// The seq of the baseline link, once one exists — always 0, because `write_baseline`
    /// refuses to chain one anywhere else. Set when this instance chains it and when
    /// `resume` finds it in the file, for the same reason `seal_seq` is: the once-only
    /// rule cannot live in one process's memory, or a daemon killed after taking the
    /// baseline would take a second one over a workspace the agent has since written to.
    baseline_seq: Option<u64>,
}

/// A completely validated existing chain that has not yet performed resume mutations.
///
/// The retained append handle and root are the same capabilities finalization consumes.
/// Preparing never sweeps descriptor temporaries or creates acknowledgements; callers may
/// inspect the recovered head and checked read-only files before choosing to finalize.
#[derive(Debug)]
pub struct PreparedTraceResume {
    root: TraceRoot,
    chain: TraceAppendFile,
    validated_bytes: u64,
    next_seq: u64,
    head: String,
    replay: ChainPhaseReplay,
}

impl PreparedTraceResume {
    /// The last completely validated link and its hash, or `None` for an empty chain.
    pub fn head(&self) -> Option<(u64, String)> {
        self.next_seq
            .checked_sub(1)
            .map(|seq| (seq, self.head.clone()))
    }

    /// Open one fixed document through the retained root as a checked opaque reader.
    pub fn read_fixed(
        &self,
        document: FixedTraceDocument,
    ) -> Result<Option<TraceReadFile>, ExportError> {
        self.root.read_fixed(document)
    }

    /// Open a reader for the retained chain's exact identity. Any unsafe acquisition
    /// permanently poisons this prepared chain and makes finalization refuse.
    pub fn independent_chain_reader(&mut self) -> Result<TraceReadFile, ExportError> {
        Ok(self.root.dir.open_matching_reader(&mut self.chain)?)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum ExportPhase {
    #[default]
    Open,
    Cutoff {
        seq: u64,
        link: Link,
        entry_id: String,
        receipt_sha256: Sha256Digest,
    },
    Terminal {
        seq: u64,
        link: Link,
        kind: TerminalKind,
    },
}

/// The record that closed a chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKind {
    /// `EventBody::Seal`, written by the seal writer.
    LegacySeal,
    /// `EventBody::FinalizationV1`, bound to the preceding signed-mode cutoff.
    FinalizationV1,
    /// The v5 `CaptureRecord::LocalFinish`, written only by [`TraceExport::write_local_finish`].
    LocalFinish,
}

impl TraceExport {
    /// Create `root` (and its parents) and open `chain.jsonl` and `acks.jsonl` for append.
    ///
    /// Both streams are opened here rather than on first write, so a trace directory
    /// names two files that exist and are empty rather than two that may not be there.
    ///
    /// A directory that already holds a non-empty `chain.jsonl` is `ChainExists`, never
    /// a fresh start: the handle is opened for append and the counter starts at 0, so
    /// creating over an existing chain would write a second run's link 0 onto the end of
    /// the first run's links — one file that reads as one chain and is two. Continuing
    /// an existing chain is `resume`'s job, and saying which of the two you meant is the
    /// caller's.
    pub fn create(root: &Path) -> Result<TraceExport, ExportError> {
        Self::create_from_root(TraceRoot::create(root)?)
    }

    /// Create a fresh export inside an already-acquired, exclusively-created trace root.
    pub fn create_from_root(root: TraceRoot) -> Result<TraceExport, ExportError> {
        root.dir.sweep_descriptor_temps()?;
        let mut chain = root.dir.open_chain_for_create()?;
        if chain.len()? > 0 {
            return Err(ExportError::ChainExists);
        }
        let mut acks = root.dir.open_acks_for_create()?;
        if acks.len()? > 0 {
            return Err(ExportError::AcksExist);
        }
        for document in [
            FixedDocument::Receipt,
            FixedDocument::Baseline,
            FixedDocument::Seal,
            FixedDocument::Manifest,
        ] {
            if root.dir.fixed_exists(document)? {
                return Err(ExportError::FixedDocumentExists {
                    name: fixed_name(document),
                });
            }
        }
        Ok(TraceExport {
            root,
            chain,
            acks,
            chain_bytes: 0,
            capture_chain: false,
            capture_quota: None,
            capture_reserved: 0,
            durable: None,
            accounting_available: true,
            next_seq: 0,
            head: GENESIS.to_string(),
            phase: ExportPhase::Open,
            baseline_seq: None,
        })
    }

    /// Reopen an export after a daemon restart, continuing the chain that is on disk.
    ///
    /// The position is **read off `chain.jsonl`**, not supplied: a restarting daemon
    /// that told the writer where it had got to would be keeping chain state of its own,
    /// which is the drift this writer exists to make impossible. It shallow-decodes each
    /// line's version and sequence first; an unsupported version is refused before typed
    /// body decoding, link checks, acknowledgement creation, temp cleanup, or mutation.
    /// Supported records then have their version/body compatibility and link checked;
    /// anything else is `ChainCorrupt`, because a chain the writer cannot read from the
    /// start is not one it can honestly append to.
    ///
    /// The scan is also where the seal is found. The once-only rule cannot live in one
    /// process's memory — a daemon killed after sealing and restarted would chain a
    /// second seal, and an acknowledgement over the entry would then have two heads to
    /// anchor — so a `Seal` line seals this instance too, and two of them
    /// are `ChainCorrupt` rather than a head this writer picks between.
    ///
    /// An empty `chain.jsonl` resumes at `next_seq` 0 over `GENESIS`: a daemon killed
    /// between `create` and its first record has a chain to start, not one to continue.
    /// A `chain.jsonl` that is *missing* is an `Io` error naming the path — there is
    /// nothing to resume. `acks.jsonl` is created if absent: a run may legitimately have
    /// been acknowledged not once.
    ///
    /// The scan **streams**. `chain.jsonl` is the whole run — every leg-1 record, every
    /// leg-2 post and every raw body — and a 168-hour entry runs to tens of
    /// gigabytes, so reading it into one `String` would make the recovery path (D9:
    /// reconnect, catch up, say "back online") the one place the daemon needs the run in
    /// memory. One line is held at a time; the `Envelope` is dropped once its `seq` and
    /// `prev` have been checked.
    pub fn resume(root: &Path) -> Result<TraceExport, ExportError> {
        Self::resume_from_root(TraceRoot::open(root)?)
    }

    /// Resume using an already acquired root, so rebuild callers can inspect typed fixed
    /// documents and then consume the same directory identity here.
    pub fn resume_from_root(root: TraceRoot) -> Result<TraceExport, ExportError> {
        Self::resume_prepared(Self::prepare_resume_from_root(root)?)
    }

    /// Validate an existing chain through one retained root and append handle without
    /// changing any directory entry or file bytes.
    pub fn prepare_resume_from_root(root: TraceRoot) -> Result<PreparedTraceResume, ExportError> {
        let mut chain = root.dir.open_chain_for_resume()?;
        let chain_path = root.display.join(CHAIN_FILE);
        let mut reader = BufReader::new(&mut chain);

        let mut next_seq = 0u64;
        let mut head = GENESIS.to_string();
        let mut replay = ChainPhaseReplay::new();
        let mut validated_bytes = 0u64;
        let mut i = 0u64;
        let capture_line_limit = MAX_ENVELOPE_BYTES as u64;
        loop {
            // A capture chain's lines are bounded before they are buffered. The first line
            // decides the format; a legacy chain keeps its historical unbounded lines.
            let limit = match replay.format() {
                Some(ChainFormat::Capture) => capture_line_limit,
                _ => u64::MAX,
            };
            let mut line = Vec::new();
            let read = (&mut reader)
                .take(limit)
                .read_until(b'\n', &mut line)
                .map_err(|source| io(&chain_path, source))?;
            if read == 0 {
                break;
            }
            let corrupt = |reason: String| ExportError::ChainCorrupt { seq: i, reason };
            let read = u64::try_from(read)
                .map_err(|_| corrupt("record byte count does not fit u64".to_string()))?;
            validated_bytes = validated_bytes
                .checked_add(read)
                .ok_or_else(|| corrupt("validated byte length overflows u64".to_string()))?;
            if line.last() != Some(&b'\n') {
                if read == limit {
                    return Err(corrupt(
                        "record exceeds the protocol v5 envelope bound".to_string(),
                    ));
                }
                return Err(corrupt(
                    "record is missing its terminating newline".to_string(),
                ));
            }
            line.pop();
            // JSON allows a trailing carriage return, so the header parses either way.
            let header: EnvelopeVersionSeq = serde_json::from_slice(&line)
                .map_err(|source| corrupt(format!("not an envelope header: {source}")))?;
            // A legacy record keeps its historical tolerance of a carriage return; a v5 record
            // must be exactly its canonical serialization (checked below).
            if header.protocol_version != M2_PROTOCOL_VERSION && line.last() == Some(&b'\r') {
                line.pop();
            }
            replay.check_version(header.seq, header.protocol_version)?;
            if header.protocol_version == M2_PROTOCOL_VERSION && read > capture_line_limit {
                return Err(corrupt(
                    "record exceeds the protocol v5 envelope bound".to_string(),
                ));
            }
            let env: Envelope = serde_json::from_slice(&line)
                .map_err(|source| corrupt(format!("not an envelope: {source}")))?;
            // The link hash covers the decoded content only, so another spelling of a v5 line
            // (an unknown key, whitespace, a carriage return) is bytes nothing covers, and the
            // verifier refuses it (`record_not_canonical`). A chain it can never verify is not
            // one to append to.
            if header.protocol_version == M2_PROTOCOL_VERSION
                && !serde_json::to_vec(&env).is_ok_and(|canonical| canonical == line)
            {
                return Err(corrupt(
                    "record is not its canonical serialization (a carriage return, extra \
                     whitespace or an unknown key was added to the line)"
                        .to_string(),
                ));
            }
            replay.check_envelope(&env)?;
            if env.link.seq != next_seq {
                return Err(corrupt(format!(
                    "names seq {}, but the chain is at {next_seq}",
                    env.link.seq
                )));
            }
            if env.link.prev != head {
                return Err(corrupt(format!(
                    "prev {} does not name the link before it",
                    env.link.prev
                )));
            }
            let expected_hash = env
                .expected_hash(&head)
                .map_err(|source| corrupt(format!("cannot recompute its hash: {source}")))?;
            if env.link.hash != expected_hash {
                return Err(corrupt("hash does not cover this envelope".to_string()));
            }
            replay.push(&env)?;
            head = env.link.hash;
            next_seq = next_seq
                .checked_add(1)
                .ok_or(ExportError::SequenceExhausted)?;
            i += 1;
        }
        drop(reader);

        let found_bytes = chain.len()?;
        if found_bytes != validated_bytes {
            return Err(ExportError::ChainLengthChanged {
                validated_bytes,
                found_bytes,
            });
        }

        Ok(PreparedTraceResume {
            root,
            chain,
            validated_bytes,
            next_seq,
            head,
            replay,
        })
    }

    /// Finalize a prepared resume only while its retained chain is still the exact length
    /// that was validated. The check also refuses a chain poisoned by a failed matched
    /// reader acquisition before any sweep or acknowledgement creation can occur.
    ///
    /// This detects growth or truncation between preparation and this check. It does not
    /// lock out a same-user rewrite that preserves length or growth after the final check.
    pub fn resume_prepared(mut prepared: PreparedTraceResume) -> Result<TraceExport, ExportError> {
        let found_bytes = prepared.chain.len()?;
        if found_bytes != prepared.validated_bytes {
            return Err(ExportError::ChainLengthChanged {
                validated_bytes: prepared.validated_bytes,
                found_bytes,
            });
        }

        prepared.root.dir.sweep_descriptor_temps()?;
        let acks = prepared.root.dir.open_acks_for_resume()?;

        Ok(TraceExport {
            root: prepared.root,
            chain: prepared.chain,
            acks,
            chain_bytes: prepared.validated_bytes,
            capture_chain: prepared.replay.format() == Some(ChainFormat::Capture),
            // A reopened writer has flushed nothing and holds no quota or measured reservation:
            // the owner configures (or re-reserves) its quota again, and the first sync
            // establishes a durable frontier.
            capture_quota: None,
            capture_reserved: 0,
            durable: None,
            accounting_available: true,
            next_seq: prepared.next_seq,
            head: prepared.head,
            baseline_seq: prepared.replay.baseline_seq(),
            phase: prepared.replay.phase,
        })
    }

    /// Stamp `link` on `env` from the writer's own chain state and append one line.
    /// The caller supplies `protocol_version`, `entry_id`, `source`, `recorded_at` and
    /// `body`; `seq`, `prev` and `hash` are the writer's to fill, so a daemon can never
    /// hand-roll a link.
    ///
    /// A caller that leaves `link` at its default (`seq` 0, empty `prev` and `hash`) is
    /// saying nothing about position and is stamped silently. A caller that *did* stamp
    /// a seq is making a claim, and it is only accepted when it names the seq about to
    /// be written: anything else is `OutOfOrder`, because a daemon keeping a counter of
    /// its own is exactly the drift this writer exists to make impossible. `prev` and
    /// `hash` are overwritten either way — they are never the caller's to state.
    ///
    /// Once a seal is chained — by this instance, or by the run `resume` continued — the
    /// chain is closed and every further append is `Sealed`. The seal is the link an
    /// acknowledgement anchors the entry by, and a record written after it would leave
    /// that acknowledgement covering a chain that kept growing.
    ///
    /// **`EventBody::Seal` and `EventBody::Baseline` are not appends.** Either body here
    /// is `ReservedBody`: they are [`TraceExport::write_seal`]'s and
    /// [`TraceExport::write_baseline`]'s to build, and everything that makes them mean
    /// anything — seal-last, baseline-first, one of each, the file written beside the
    /// link — lives in those two methods. The check is here rather than in a caller so
    /// that it travels with the export to every holder of one: a chain with two seals
    /// gives an acknowledgement two heads to anchor, and a baseline chained
    /// at an arbitrary seq is a claim about a workspace the agent has been writing to.
    ///
    /// **Protocol v5 is admitted only into a capture chain** (see
    /// [`Self::configure_capture_quota`]); there every record is v5 and is budgeted against
    /// the quota before it is written, and `CaptureRecord::LocalFinish` is reserved to
    /// [`Self::write_local_finish`] exactly as the seal is to `write_seal`.
    pub fn append_chain(&mut self, env: &mut Envelope) -> Result<Link, ExportError> {
        self.admit_envelope(env)?;
        match &env.body {
            EventBody::HomeSeeded(seed) if seed.has_legacy_wire_layout() => {
                return Err(ExportError::LegacyHomeSeededAppend);
            }
            EventBody::Seal { .. } => return Err(ExportError::ReservedBody { kind: "seal" }),
            EventBody::Baseline { .. } => {
                return Err(ExportError::ReservedBody { kind: "baseline" });
            }
            EventBody::CutoffReachedV1(_) => {
                return Err(ExportError::ReservedBody { kind: "cutoff" });
            }
            EventBody::FinalizationV1(_) => {
                return Err(ExportError::ReservedBody {
                    kind: "finalization",
                });
            }
            EventBody::Capture {
                record: CaptureRecord::LocalFinish { .. },
            } => {
                return Err(ExportError::ReservedBody {
                    kind: "local_finish",
                });
            }
            _ => {}
        }
        self.ensure_open_for_ordinary_append()?;
        self.append_link_unchecked(env)
    }

    /// Version/body admission for this writer's chain. An export that has not selected
    /// capture refuses v5 as the unsupported version it always was; a capture chain refuses
    /// every legacy version.
    fn admit_envelope(&self, env: &Envelope) -> Result<(), ExportError> {
        let format = if self.capture_chain {
            ChainFormat::Capture
        } else {
            ChainFormat::Legacy
        };
        admit_for_format(Some(format), env)
    }

    /// Capture-chain admission of one complete serialized line (newline included), before
    /// any byte is written: the v5 envelope bound, then the quota. Payload stops in front of
    /// the terminal allowance; a bounded closure may spend it, and a refused closure means
    /// loss can no longer be accounted.
    fn admit_capture_line(&mut self, body: &EventBody, len: usize) -> Result<(), ExportError> {
        if !self.capture_chain {
            return Ok(());
        }
        let quota = self
            .capture_quota
            .ok_or(ExportError::CaptureQuotaNotConfigured)?;
        let len = len as u64;
        if len > MAX_ENVELOPE_BYTES as u64 {
            return Err(ExportError::EnvelopeTooLarge {
                bytes: len,
                limit: MAX_ENVELOPE_BYTES as u64,
            });
        }
        let terminal_allowance = spends_terminal_allowance(body);
        // `configure_capture_quota` refuses any quota not above the terminal allowance.
        let limit = if terminal_allowance {
            quota
        } else {
            quota - TERMINAL_RESERVE_BYTES
        };
        let available = limit.saturating_sub(self.chain_bytes);
        if len > available {
            if terminal_allowance {
                self.accounting_available = false;
            }
            return Err(ExportError::CaptureStorageFull {
                needed: len,
                available,
                terminal_allowance,
            });
        }
        Ok(())
    }

    /// The stamping itself, with no body-kind check: `write_seal` and `write_baseline`
    /// chain the two bodies `append_chain` reserves, and they reach the chain through
    /// here. Every other rule — seal-closes-the-chain, contiguity, the caller's link
    /// being the writer's to fill — applies to them exactly as it does to a record.
    fn append_link_unchecked(&mut self, env: &mut Envelope) -> Result<Link, ExportError> {
        self.admit_envelope(env)?;
        if self.capture_chain
            && self.next_seq == 0
            && !matches!(env.body, EventBody::Baseline { .. })
        {
            return Err(ExportError::InvalidCaptureEnvelope {
                seq: env.link.seq,
                reason: CAPTURE_GENESIS_NOT_BASELINE,
            });
        }
        let claimed = &env.link;
        let unstamped = claimed.seq == 0 && claimed.prev.is_empty() && claimed.hash.is_empty();
        if !unstamped && claimed.seq != self.next_seq {
            return Err(ExportError::OutOfOrder {
                found: claimed.seq,
                expected_after: self.next_seq.checked_sub(1),
            });
        }

        // A record at `u64::MAX` would have no representable successor. Refuse before
        // touching the file so a successfully appended line never leaves a writer whose
        // next ordinary call could wrap to zero.
        if self.next_seq == u64::MAX {
            return Err(ExportError::SequenceExhausted);
        }

        env.link.seq = self.next_seq;
        env.link.prev = self.head.clone();
        env.link.hash = link_hash(&self.head, &env.chain_bytes()?);

        let mut line = serde_json::to_vec(&env).map_err(|source| ExportError::Serialise {
            what: "envelope",
            source,
        })?;
        line.push(b'\n');
        self.admit_capture_line(&env.body, line.len())?;
        // Written before the state moves: a line that did not reach the file must not
        // leave the head pointing at a link nobody can read.
        if let Err(error) = self.chain.append_checked(&line) {
            // Some prefix of the line may be on disk; the byte count is no longer known.
            self.accounting_available = false;
            return Err(error.into());
        }
        match self.chain_bytes.checked_add(line.len() as u64) {
            Some(bytes) => self.chain_bytes = bytes,
            None => self.accounting_available = false,
        }

        self.next_seq = self
            .next_seq
            .checked_add(1)
            .ok_or(ExportError::SequenceExhausted)?;
        self.head = env.link.hash.clone();
        Ok(env.link.clone())
    }

    fn ensure_open_for_ordinary_append(&self) -> Result<(), ExportError> {
        match self.phase {
            ExportPhase::Open => Ok(()),
            ExportPhase::Cutoff { seq, .. } => Err(ExportError::AfterCutoff { cutoff_seq: seq }),
            ExportPhase::Terminal {
                seq,
                kind: TerminalKind::LegacySeal,
                ..
            } => Err(ExportError::Sealed { seal_seq: seq }),
            ExportPhase::Terminal { seq, .. } => Err(ExportError::Terminal { terminal_seq: seq }),
        }
    }

    /// Append the once-only local cutoff boundary at the legacy default protocol version.
    pub fn write_cutoff(
        &mut self,
        entry_id: &str,
        recorded_at: &str,
        cutoff: CutoffReachedV1,
    ) -> Result<Link, ExportError> {
        self.write_cutoff_for_protocol(crate::PROTOCOL_VERSION, entry_id, recorded_at, cutoff)
    }

    /// Append the once-only cutoff boundary under an explicit envelope protocol version.
    ///
    /// The version is admitted for this chain before anything is hashed or written: v5 only
    /// into a capture chain, a legacy version only into a legacy chain. The cutoff validation
    /// and once-only phase rules are those of [`Self::write_cutoff`], which delegates here.
    pub fn write_cutoff_for_protocol(
        &mut self,
        protocol_version: u32,
        entry_id: &str,
        recorded_at: &str,
        cutoff: CutoffReachedV1,
    ) -> Result<Link, ExportError> {
        cutoff.validate()?;
        validate_phase_inputs(entry_id, recorded_at)?;
        match self.phase {
            ExportPhase::Open => {}
            ExportPhase::Cutoff { seq, .. } => {
                return Err(ExportError::CutoffAlreadyChained { seq });
            }
            ExportPhase::Terminal { seq, .. } => {
                return Err(ExportError::Terminal { terminal_seq: seq });
            }
        }
        let receipt_sha256 = cutoff.receipt_sha256.clone();
        let mut env = Envelope {
            protocol_version,
            entry_id: entry_id.to_string(),
            source: "daemon".to_string(),
            recorded_at: recorded_at.to_string(),
            link: Link {
                seq: 0,
                prev: String::new(),
                hash: String::new(),
            },
            body: EventBody::CutoffReachedV1(cutoff),
            redaction: None,
        };
        let link = self.append_link_unchecked(&mut env)?;
        self.phase = ExportPhase::Cutoff {
            seq: link.seq,
            link: link.clone(),
            entry_id: entry_id.to_string(),
            receipt_sha256,
        };
        Ok(link)
    }

    /// Append the single terminal result bound to the preceding cutoff, at the legacy default
    /// protocol version.
    pub fn write_finalization(
        &mut self,
        entry_id: &str,
        recorded_at: &str,
        finalization: FinalizationV1,
    ) -> Result<Link, ExportError> {
        self.write_finalization_for_protocol(
            crate::PROTOCOL_VERSION,
            entry_id,
            recorded_at,
            finalization,
        )
    }

    /// Append the single terminal result bound to the preceding cutoff under an explicit
    /// envelope protocol version, admitted before hashing or writing exactly as for
    /// [`Self::write_cutoff_for_protocol`]. Binding, captured-submission and once-only rules
    /// are those of [`Self::write_finalization`], which delegates here.
    pub fn write_finalization_for_protocol(
        &mut self,
        protocol_version: u32,
        entry_id: &str,
        recorded_at: &str,
        finalization: FinalizationV1,
    ) -> Result<Link, ExportError> {
        finalization.validate()?;
        validate_phase_inputs(entry_id, recorded_at)?;
        let (cutoff_entry, cutoff_receipt, cutoff_link) = match &self.phase {
            ExportPhase::Open => return Err(ExportError::FinalizationBeforeCutoff),
            ExportPhase::Cutoff {
                entry_id: cutoff_entry,
                receipt_sha256,
                link,
                ..
            } if cutoff_entry == entry_id && receipt_sha256 == finalization.receipt_sha256() => {
                (cutoff_entry.clone(), receipt_sha256.clone(), link.clone())
            }
            ExportPhase::Cutoff { .. } => {
                return Err(ExportError::FinalizationBindingMismatch);
            }
            ExportPhase::Terminal {
                seq,
                kind: TerminalKind::FinalizationV1,
                ..
            } => return Err(ExportError::FinalizationAlreadyChained { seq: *seq }),
            ExportPhase::Terminal { seq, .. } => {
                return Err(ExportError::Terminal { terminal_seq: *seq });
            }
        };
        self.validate_captured_workspace_submission(
            &finalization,
            &cutoff_entry,
            &cutoff_receipt,
            &cutoff_link,
        )?;
        let mut env = Envelope {
            protocol_version,
            entry_id: entry_id.to_string(),
            source: "daemon".to_string(),
            recorded_at: recorded_at.to_string(),
            link: Link {
                seq: 0,
                prev: String::new(),
                hash: String::new(),
            },
            body: EventBody::FinalizationV1(finalization),
            redaction: None,
        };
        let link = self.append_link_unchecked(&mut env)?;
        self.phase = ExportPhase::Terminal {
            seq: link.seq,
            link: link.clone(),
            kind: TerminalKind::FinalizationV1,
        };
        Ok(link)
    }

    fn validate_captured_workspace_submission(
        &self,
        finalization: &FinalizationV1,
        cutoff_entry: &str,
        cutoff_receipt: &Sha256Digest,
        cutoff_link: &Link,
    ) -> Result<(), ExportError> {
        let FinalizationV1::WorkspaceSubmission {
            submission: SubmissionOutcomeV1::Captured(captured),
            ..
        } = finalization
        else {
            return Ok(());
        };

        let submission_dir = match self.root.dir.open_child_dir(SUBMISSION_DIR) {
            Ok(dir) => dir,
            Err(TraceCapError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                return Err(ExportError::FinalizationBindingMismatch);
            }
            Err(error) => return Err(error.into()),
        };
        let Some(file) = submission_dir.read_regular(SUBMISSION_MANIFEST_NAME)? else {
            return Err(ExportError::FinalizationBindingMismatch);
        };
        let manifest: SubmissionManifestV1 =
            serde_json::from_reader(file).map_err(|source| ExportError::Deserialise {
                what: "submission manifest",
                source,
            })?;
        let manifest_sha256 = manifest.digest()?;

        if manifest_sha256 != captured.manifest_sha256
            || manifest.entry_id != cutoff_entry
            || &manifest.receipt_sha256 != cutoff_receipt
            || manifest.cutoff_seq != cutoff_link.seq
            || manifest.cutoff_chain_hash.as_str() != cutoff_link.hash
        {
            return Err(ExportError::FinalizationBindingMismatch);
        }
        Ok(())
    }

    pub fn append_ack(&mut self, ack: &AckCheckpoint) -> Result<(), ExportError> {
        let mut line = serde_json::to_vec(ack).map_err(|source| ExportError::Serialise {
            what: "acknowledgement",
            source,
        })?;
        line.push(b'\n');
        self.acks.append_checked(&line)?;
        Ok(())
    }

    /// Append one acknowledgement and make it durable under `acks.jsonl` before returning.
    ///
    /// This is the delivery-commit boundary: only `Ok` means the acknowledgement is retained
    /// where a restarted reader finds it. Any failure permanently refuses further commits on
    /// this writer (see `TraceDir::sync_append`), so a caller never treats an unflushed or
    /// misplaced acknowledgement as delivered. The acknowledgement's own signature, entry,
    /// head and monotonicity are the delivery adapter's to validate, not this writer's.
    pub fn append_ack_synced(&mut self, ack: &AckCheckpoint) -> Result<(), ExportError> {
        self.append_ack(ack)?;
        self.root.dir.sync_append(&mut self.acks)?;
        Ok(())
    }

    /// Open an independent reader of `acks.jsonl` at offset zero, requiring the name to be
    /// the retained acknowledgement writer's exact file. Acquisition failure poisons that
    /// writer, as for [`Self::independent_chain_reader`].
    pub fn independent_acks_reader(&mut self) -> Result<TraceReadFile, ExportError> {
        Ok(self.root.dir.open_matching_reader(&mut self.acks)?)
    }

    /// Select protocol-v5 capture for this chain and set its serialized chain-byte quota.
    ///
    /// **This is admission accounting, not a reservation.** Nothing is allocated, the chain's
    /// logical length is untouched, and [`Self::capture_storage`] reports `reserved_bytes: 0`
    /// — even over blocks an earlier [`Self::reserve_capture`] allocated; a quota-only chain is
    /// an explicitly unreserved experiment and never qualifies a pilot launch.
    ///
    /// `bytes` includes [`TERMINAL_RESERVE_BYTES`], which payload never spends, so it must
    /// exceed that allowance. It cannot be below the bytes already written. A legacy chain
    /// that already holds a record cannot become a capture chain; on an empty chain this
    /// selects v5 before the baseline, and a reopened capture chain configures its quota
    /// again here before appending.
    pub fn configure_capture_quota(&mut self, bytes: u64) -> Result<(), ExportError> {
        self.check_capture_quota(bytes)?;
        self.capture_chain = true;
        self.capture_quota = Some(bytes);
        self.capture_reserved = 0;
        Ok(())
    }

    /// Select protocol-v5 capture, set the serialized chain-byte quota, and physically reserve
    /// that quota in `chain.jsonl` itself: the pilot's storage preflight, which subsumes
    /// [`Self::configure_capture_quota`] for a shipped run.
    ///
    /// `bytes` has exactly the quota's meaning and refusals — the whole serialized chain
    /// allowance, [`TERMINAL_RESERVE_BYTES`] included, never added. The chain file is then
    /// topped up, through the retained chain capability, until at least `bytes` of it are
    /// physically allocated, without moving its logical end ([`TraceDir::reserve_append`]:
    /// macOS APFS only, `F_ALLOCATEALL | F_ALLOCATEPERSIST` recipe, measured from the
    /// file's block count). Only after that remeasurement does [`Self::capture_storage`] report
    /// the quota as `reserved_bytes`.
    ///
    /// Any refusal — unsupported filesystem or platform, insufficient space, a kernel error, a
    /// short remeasurement, or a chain length that is not this writer's — leaves the quota and
    /// the reported reservation exactly as they were and writes no chain byte; it must refuse
    /// pilot launch, and the budget is never silently reduced. A reopened writer reports
    /// nothing reserved until this is called again, which remeasures the retained allocation
    /// and adds only what is missing (nothing, for an unchanged quota), so a restart never
    /// allocates twice.
    ///
    /// The allocation is this chain's protected floor, not admission: payload still stops in
    /// front of the terminal allowance and every record at the quota. Registry,
    /// acknowledgement, fixed-document and harness writes need free-space headroom of their
    /// own; a failed one stays a visible error.
    pub fn reserve_capture(&mut self, bytes: u64) -> Result<(), ExportError> {
        self.check_capture_quota(bytes)?;
        let measured = self.root.dir.reserve_append(&mut self.chain, bytes)?;
        if measured.logical_bytes != self.chain_bytes {
            return Err(ExportError::ChainLengthChanged {
                validated_bytes: self.chain_bytes,
                found_bytes: measured.logical_bytes,
            });
        }
        self.capture_chain = true;
        self.capture_quota = Some(bytes);
        // Measured, and never reported beyond the quota it backs.
        self.capture_reserved = measured.allocated_bytes.min(bytes);
        Ok(())
    }

    /// The quota refusals shared by [`Self::configure_capture_quota`] and
    /// [`Self::reserve_capture`], checked before either changes anything.
    fn check_capture_quota(&self, bytes: u64) -> Result<(), ExportError> {
        if bytes <= TERMINAL_RESERVE_BYTES {
            return Err(ExportError::InvalidCaptureQuota {
                bytes,
                reason: "a capture quota must exceed the terminal allowance it includes",
            });
        }
        if !self.capture_chain && self.head().is_some() {
            return Err(ExportError::InvalidCaptureQuota {
                bytes,
                reason: "an existing legacy chain cannot become a protocol v5 capture chain",
            });
        }
        if bytes < self.chain_bytes {
            return Err(ExportError::InvalidCaptureQuota {
                bytes,
                reason: "a capture quota cannot be below the chain bytes already written",
            });
        }
        Ok(())
    }

    /// Flush `chain.jsonl` through the retained capability and publish the durable frontier.
    ///
    /// Returns the frontier made durable — the head at the time of the call, with the chain
    /// bytes through it — or `None` for an empty chain. On failure the previous frontier is
    /// kept and the chain writer is poisoned: an appended head never becomes durable by a
    /// later retry after a failed flush. The directory entry is flushed too, so newly created
    /// trace files are durable with the frontier.
    pub fn sync_capture(&mut self) -> Result<Option<DurableFrontier>, ExportError> {
        let frontier = self.head().map(|(seq, hash)| DurableFrontier {
            seq,
            hash,
            chain_bytes: self.chain_bytes,
        });
        self.root.dir.sync_append(&mut self.chain)?;
        if frontier.is_some() {
            self.durable.clone_from(&frontier);
        }
        Ok(frontier)
    }

    /// The chain's storage accounting. `quota_bytes` is the configured serialized-byte
    /// allowance (zero when none is configured); `reserved_bytes` is the part of it this writer
    /// measured as physically allocated by [`Self::reserve_capture`] — zero for a quota-only
    /// chain and after a reopen until remeasured. A quota is never reported as reservation.
    pub fn capture_storage(&self) -> CaptureStorage {
        CaptureStorage {
            quota_bytes: self.capture_quota.unwrap_or(0),
            reserved_bytes: self.capture_reserved,
            used_bytes: self.chain_bytes,
            durable: self.durable.clone(),
            accounting_available: self.accounting_available,
        }
    }

    /// Close a protocol-v5 capture chain locally: the reserved terminal primitive.
    ///
    /// Binds the canonical digest of `manifest` (the exact bytes written to `seal.json`) in a
    /// `CaptureRecord::LocalFinish` carrying the local `observed` time, a bounded `reason`
    /// and the caller's actual `termination` outcome — the typed result of really reaping the
    /// process, never a claim this primitive makes; an `Unresolved` outcome stays unresolved.
    /// There is no receipt, signed cutoff or deadline prerequisite and none is fabricated.
    ///
    /// Inputs are validated before any write: the termination's time ordering through
    /// `TerminationOutcomeV1::validate`, the observation and reason bounds, and the chain
    /// state — an open capture chain with a configured quota; a cutoff chain may only
    /// finalize, and a terminal chain refuses (`LocalFinishAlreadyChained` for a committed
    /// local finish, so a retry observes it rather than writing a duplicate).
    ///
    /// It appends exactly one terminal envelope, closes the phase and flushes the chain, so the
    /// terminal link is the durable frontier before the absent-only `seal.json` is created; a
    /// final flush makes the directory entry naming `seal.json` durable. A failure after the
    /// append leaves a terminal chain whose fixed manifest is missing or unflushed: that is an
    /// incomplete artifact and is reported as an error, never as success.
    pub fn write_local_finish(
        &mut self,
        manifest: &SealManifest,
        observed: CaptureTime,
        reason: &str,
        termination: TerminationOutcomeV1,
    ) -> Result<(PathBuf, Link), ExportError> {
        match self.phase {
            ExportPhase::Open => {}
            ExportPhase::Cutoff { seq, .. } => {
                return Err(ExportError::AfterCutoff { cutoff_seq: seq });
            }
            ExportPhase::Terminal {
                seq,
                kind: TerminalKind::LocalFinish,
                ..
            } => return Err(ExportError::LocalFinishAlreadyChained { seq }),
            ExportPhase::Terminal { seq, .. } => {
                return Err(ExportError::Terminal { terminal_seq: seq });
            }
        }
        validate_local_finish(&observed, reason, &termination)?;
        let canonical = manifest.clone().sorted();
        validate_phase_inputs(&canonical.entry_id, &observed.wall)?;
        let bytes = to_json("seal", &canonical)?;
        let mut env = Envelope {
            protocol_version: M2_PROTOCOL_VERSION,
            entry_id: canonical.entry_id.clone(),
            source: "daemon".to_string(),
            recorded_at: observed.wall.clone(),
            link: Link {
                seq: 0,
                prev: String::new(),
                hash: String::new(),
            },
            body: EventBody::Capture {
                record: CaptureRecord::LocalFinish {
                    observed,
                    reason: reason.to_string(),
                    termination,
                    seal_manifest_sha256: hex::encode(Sha256::digest(&bytes)),
                },
            },
            redaction: None,
        };
        let link = self.append_link_unchecked(&mut env)?;
        self.phase = ExportPhase::Terminal {
            seq: link.seq,
            link: link.clone(),
            kind: TerminalKind::LocalFinish,
        };
        self.sync_capture()?;
        let path = self.write_fixed(FixedDocument::Seal, &bytes)?;
        self.sync_capture()?;
        Ok((path, link))
    }

    /// Written tmp-then-rename: `trace.toml` is re-written every time the
    /// descriptor changes — a rehydrated entry, a new tracked session — while a verifier
    /// may open it at any moment, and a reader that opened it mid-write must see the
    /// previous document whole rather than half of the next one. A rename replaces the
    /// directory entry in one step; an in-place truncate-and-write does not.
    pub fn write_descriptor(&self, doc: &TraceDescriptorDoc) -> Result<PathBuf, ExportError> {
        let text = toml::to_string(doc).map_err(|source| ExportError::Toml { source })?;
        self.root.dir.replace_descriptor(text.as_bytes())?;
        Ok(self.root.display.join(DESCRIPTOR_FILE))
    }

    pub fn write_receipt(&self, r: &StartReceipt) -> Result<PathBuf, ExportError> {
        self.write_fixed(FixedDocument::Receipt, &to_json("receipt", r)?)
    }

    /// Open one fixed document through this export's retained trace-root capability.
    pub fn read_fixed(
        &self,
        document: FixedTraceDocument,
    ) -> Result<Option<TraceReadFile>, ExportError> {
        self.root.read_fixed(document)
    }

    /// Put the connector's shipped `manifest.toml` in the trace directory, byte for byte,
    /// under [`MANIFEST_FILE`].
    ///
    /// The bytes are the caller's — this is the one file in a trace this crate does not
    /// author — but the *placing* is the export's, like every other file a verifier
    /// reads: `TraceDescriptorDoc::connector_manifest` names a relative file, so the
    /// directory stays copyable, and the digest pin compares the bytes that judged the
    /// run rather than whatever sits in the participant's connector directory now.
    ///
    /// Bytes and not a source path: `fs::copy` would carry the source file's mode, and a
    /// caller that has already read the manifest (to hash it, or to parse the heartbeat
    /// interval out of it) must write the bytes it hashed rather than re-reading a file
    /// that may have moved between the two.
    pub fn write_manifest(&self, bytes: &[u8]) -> Result<PathBuf, ExportError> {
        self.write_fixed(FixedDocument::Manifest, bytes)
    }

    /// Chain the baseline as the chain's FIRST link, then write it. Appends one
    /// `EventBody::Baseline` envelope carrying `manifest_sha256(m)` and `m.files.len()` —
    /// `source` is `"daemon"`, `recorded_at` is `m.sealed_at`, which for a baseline is
    /// the moment the workspace was measured — and only then writes `baseline.json`.
    ///
    /// **This is what binds the baseline to anything.** `baseline.json` on its own is a
    /// file the participant's own daemon wrote: a participant who pre-populated the
    /// workspace could list every planted file's real path and digest in it, and the
    /// verifier would exempt each one as "the workspace already held this". Chaining it
    /// at seq 0 makes the baseline the genesis link a start receipt's `chain_head`
    /// witnesses, so rewriting it after the run was declared moves the head the server
    /// signed.
    ///
    /// Called on a chain that already has links, this is `BaselineNotFirst`: a manifest
    /// taken after the run started describes a workspace the agent has been writing to,
    /// and chaining it anyway would let it read as a start. A second call is
    /// `BaselineAlreadyChained` — two baselines are two answers to the same question.
    ///
    /// The seq is recorded before `baseline.json` is written, so a failure to write the
    /// file does not license a second link over the same baseline; what is on disk then
    /// is a chained claim with no manifest beside it, which V13 reports.
    pub fn write_baseline(&mut self, m: &SealManifest) -> Result<(PathBuf, Link), ExportError> {
        self.write_baseline_for_protocol(crate::PROTOCOL_VERSION, m)
    }

    /// Write a baseline using an explicit envelope protocol version.
    ///
    /// Historical fixture generators use this to reproduce their original signed bytes.
    /// Ordinary callers use [`Self::write_baseline`], which always selects the current
    /// protocol. Protocol choice changes only this envelope; all position and phase rules
    /// remain owned by this shared implementation.
    pub fn write_baseline_for_protocol(
        &mut self,
        protocol_version: u32,
        m: &SealManifest,
    ) -> Result<(PathBuf, Link), ExportError> {
        self.ensure_open_for_ordinary_append()?;
        if self.baseline_seq.is_some() {
            return Err(ExportError::BaselineAlreadyChained);
        }
        if let Some((head_seq, _)) = self.head() {
            return Err(ExportError::BaselineNotFirst { head_seq });
        }
        let canonical = m.clone().sorted();
        let bytes = to_json("baseline", &canonical)?;
        let mut env = Envelope {
            protocol_version,
            entry_id: canonical.entry_id.clone(),
            source: "daemon".to_string(),
            recorded_at: canonical.sealed_at.clone(),
            link: Link {
                seq: 0,
                prev: String::new(),
                hash: String::new(),
            },
            body: EventBody::Baseline {
                manifest_sha256: hex::encode(Sha256::digest(&bytes)),
                file_count: canonical.files.len() as u64,
            },
            redaction: None,
        };
        let link = self.append_link_unchecked(&mut env)?;
        self.baseline_seq = Some(link.seq);
        let path = self.write_fixed(FixedDocument::Baseline, &bytes)?;
        Ok((path, link))
    }

    /// Chain the seal, THEN write it. Appends one `EventBody::Seal` envelope carrying
    /// `manifest_sha256(m)`, `m.sealed_at` and `m.files.len()` — `source` is `"daemon"`,
    /// `recorded_at` is `m.sealed_at` — and only then writes `seal.json`. `&mut self`
    /// because it appends a link, and the returned `Link` is the last link of the
    /// entry's chain, so an acknowledgement over a seq at or above it anchors the seal
    /// itself. A second call is `SealAlreadyChained`: two seal links would
    /// give an acknowledgement two heads to anchor.
    ///
    /// The seq is recorded before `seal.json` is written, so a failure to write the file
    /// does not license a second link over the same seal. What is on disk then is a
    /// chained claim with no manifest beside it, which V13 reports; what a retry would
    /// leave is two claims a reviewer has to choose between.
    pub fn write_seal(&mut self, m: &SealManifest) -> Result<(PathBuf, Link), ExportError> {
        self.write_seal_for_protocol(crate::PROTOCOL_VERSION, m)
    }

    /// Write a seal using an explicit envelope protocol version.
    ///
    /// This is the reserved-envelope counterpart to callers supplying a version on an
    /// ordinary [`Envelope`]. It exists for byte-stable historical fixtures; the phase and
    /// once-only rules are identical to [`Self::write_seal`].
    pub fn write_seal_for_protocol(
        &mut self,
        protocol_version: u32,
        m: &SealManifest,
    ) -> Result<(PathBuf, Link), ExportError> {
        match self.phase {
            ExportPhase::Open => {}
            ExportPhase::Cutoff { seq, .. } => {
                return Err(ExportError::AfterCutoff { cutoff_seq: seq });
            }
            ExportPhase::Terminal {
                seq,
                kind: TerminalKind::LegacySeal,
                ..
            } => return Err(ExportError::SealAlreadyChained { seq }),
            ExportPhase::Terminal { seq, .. } => {
                return Err(ExportError::Terminal { terminal_seq: seq });
            }
        }
        let canonical = m.clone().sorted();
        let bytes = to_json("seal", &canonical)?;
        let mut env = Envelope {
            protocol_version,
            entry_id: canonical.entry_id.clone(),
            source: "daemon".to_string(),
            recorded_at: canonical.sealed_at.clone(),
            link: Link {
                seq: 0,
                prev: String::new(),
                hash: String::new(),
            },
            body: EventBody::Seal {
                manifest_sha256: hex::encode(Sha256::digest(&bytes)),
                sealed_at: canonical.sealed_at.clone(),
                file_count: canonical.files.len() as u64,
            },
            redaction: None,
        };
        let link = self.append_link_unchecked(&mut env)?;
        self.phase = ExportPhase::Terminal {
            seq: link.seq,
            link: link.clone(),
            kind: TerminalKind::LegacySeal,
        };
        let path = self.write_fixed(FixedDocument::Seal, &bytes)?;
        Ok((path, link))
    }

    /// The seq of the last link written and the hash it left as the head, or `None` when
    /// nothing has been chained.
    ///
    /// `None` rather than `(0, GENESIS)`: those are two different states — a chain with
    /// no links, and a chain whose head is link 0 — and collapsing them is how a resume
    /// after a crash before the first record would start at seq 1 and leave the chain
    /// with no seq 0. A start receipt's `chain_head` claim is taken against `GENESIS`
    /// when this is `None`.
    pub fn head(&self) -> Option<(u64, String)> {
        self.next_seq
            .checked_sub(1)
            .map(|seq| (seq, self.head.clone()))
    }

    /// The acquisition spelling retained for diagnostics and legacy consumers. It does
    /// not identify the directory capability and is never used for this writer's I/O.
    pub fn root(&self) -> &Path {
        &self.root.display
    }

    /// Open an independent reader at offset zero and require the directory entry to be
    /// the retained chain writer's exact file identity. Acquisition failure poisons the
    /// chain writer, because a caller may otherwise restore a swapped name and append.
    pub fn independent_chain_reader(&mut self) -> Result<TraceReadFile, ExportError> {
        Ok(self.root.dir.open_matching_reader(&mut self.chain)?)
    }

    fn write_fixed(&self, document: FixedDocument, bytes: &[u8]) -> Result<PathBuf, ExportError> {
        self.root
            .dir
            .create_fixed(document)?
            .write_all_sync_checked(bytes)?;
        Ok(self.root.display.join(fixed_name(document)))
    }
}

/// The sha256, hex, of the exact bytes `write_seal` puts in `seal.json` — and of the
/// bytes `write_baseline` puts in `baseline.json`, which go through the same writer. One
/// definition, so a chained claim and V13's recomputation cannot differ; V13 calls this
/// function rather than hashing either file its own way.
///
/// It is a function of the *manifest*, not of a file: V13 recomputes it from the
/// `SealManifest` it loaded, so a `seal.json` that was re-indented after capture still
/// yields the digest the chain claims, and one whose *content* moved does not.
pub fn manifest_sha256(m: &SealManifest) -> Result<String, ExportError> {
    let canonical = m.clone().sorted();
    Ok(hex::encode(Sha256::digest(to_json("seal", &canonical)?)))
}

/// Compact `serde_json`, no trailing newline. The seal's bytes go through a digest, so
/// their layout is inside a hash rather than a matter of taste; the other documents use
/// the same writer so there is one answer to "how does this crate write a JSON file".
fn to_json<T: Serialize>(what: &'static str, value: &T) -> Result<Vec<u8>, ExportError> {
    serde_json::to_vec(value).map_err(|source| ExportError::Serialise { what, source })
}

fn io(path: &Path, source: std::io::Error) -> ExportError {
    ExportError::Io {
        path: path.display().to_string(),
        source,
    }
}

fn fixed_name(document: FixedDocument) -> &'static str {
    match document {
        FixedDocument::Receipt => RECEIPT_FILE,
        FixedDocument::Baseline => BASELINE_FILE,
        FixedDocument::Seal => SEAL_FILE,
        FixedDocument::Manifest => MANIFEST_FILE,
    }
}

/// Fault-seam tests at the real flush boundary (`TraceDir::sync_append`). The faults are real
/// filesystem states raised at that call — a second name for the chain file, or its name
/// replaced — which the flush refuses exactly as it refuses a failed `F_FULLFSYNC`: by
/// returning an error and poisoning the stream. The injected-`EIO` case of the same call is
/// the platform's own unit test.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::GapReason;
    use crate::provenance::TerminationMethodV1;
    use goalcup_platform::tracecap::{RefusalKind, TraceCapError};

    const ENTRY: &str = "entry-m2-0001";

    fn at(mono_ns: u64) -> CaptureTime {
        CaptureTime {
            wall: "2026-09-27T12:00:05.500Z".to_string(),
            mono_ns,
            clock_id: "daemon-5b0c3a1e".to_string(),
        }
    }

    fn gap(mono_ns: u64) -> Envelope {
        Envelope {
            protocol_version: M2_PROTOCOL_VERSION,
            entry_id: ENTRY.to_string(),
            source: "daemon".to_string(),
            recorded_at: "2026-09-27T12:00:01Z".to_string(),
            link: Link {
                seq: 0,
                prev: String::new(),
                hash: String::new(),
            },
            body: EventBody::Capture {
                record: CaptureRecord::Gap {
                    stream: None,
                    observed: at(mono_ns),
                    reason: GapReason::StorageIo,
                    lost_bytes: None,
                },
            },
            redaction: None,
        }
    }

    fn manifest() -> SealManifest {
        SealManifest {
            entry_id: ENTRY.to_string(),
            sealed_at: "2026-09-27T12:00:04Z".to_string(),
            workspace: "/ws".to_string(),
            files: Vec::new(),
            excluded: Vec::new(),
        }
    }

    fn confirmed() -> TerminationOutcomeV1 {
        TerminationOutcomeV1::Confirmed {
            requested_at: Rfc3339Utc::try_from("2026-09-27T12:00:02Z").unwrap(),
            confirmed_at: Rfc3339Utc::try_from("2026-09-27T12:00:03Z").unwrap(),
            method: TerminationMethodV1::ProcessSignal,
        }
    }

    fn capture_export(root: &Path) -> TraceExport {
        let mut export = TraceExport::create(root).unwrap();
        export
            .configure_capture_quota(TERMINAL_RESERVE_BYTES * 4)
            .unwrap();
        export
            .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest())
            .unwrap();
        export
    }

    #[test]
    fn failed_sync_preserves_previous_frontier() {
        let tmp = tempfile::tempdir().unwrap();
        let mut export = capture_export(tmp.path());
        export.append_chain(&mut gap(1)).unwrap();
        let first = export.sync_capture().unwrap().unwrap();
        export.append_chain(&mut gap(2)).unwrap();
        let appended = export.head().unwrap();
        assert_eq!(appended.0, first.seq + 1);

        // The flush meets a chain file that gained a second name.
        let alias = tmp.path().join("chain.alias");
        std::fs::hard_link(tmp.path().join(CHAIN_FILE), &alias).unwrap();
        assert!(matches!(
            export.sync_capture(),
            Err(ExportError::Capability(TraceCapError::Refused {
                reason: RefusalKind::MultipleLinks,
                ..
            }))
        ));
        assert_eq!(
            export.capture_storage().durable,
            Some(first.clone()),
            "the failed flush keeps the previous frontier"
        );

        // Clearing the fault does not let a retry report the unflushed head durable.
        std::fs::remove_file(&alias).unwrap();
        assert!(matches!(
            export.sync_capture(),
            Err(ExportError::Capability(TraceCapError::Poisoned { .. }))
        ));
        assert!(export.append_chain(&mut gap(3)).is_err());
        assert!(
            export
                .write_local_finish(&manifest(), at(9), "participant_stop", confirmed())
                .is_err()
        );
        assert_eq!(export.capture_storage().durable, Some(first));
        assert_eq!(export.head(), Some(appended));
    }

    #[test]
    fn local_finish_flush_failure_is_not_success() {
        let tmp = tempfile::tempdir().unwrap();
        let mut export = capture_export(tmp.path());
        let before = export.sync_capture().unwrap().unwrap();

        // The chain's name no longer identifies the retained chain file when finish flushes.
        std::fs::rename(tmp.path().join(CHAIN_FILE), tmp.path().join("chain.moved")).unwrap();
        std::fs::write(tmp.path().join(CHAIN_FILE), b"").unwrap();
        let failed = export.write_local_finish(&manifest(), at(9), "participant_stop", confirmed());
        assert!(
            matches!(
                failed,
                Err(ExportError::Capability(TraceCapError::Refused {
                    reason: RefusalKind::IdentityChanged,
                    ..
                }))
            ),
            "a terminal that is not durable under chain.jsonl is not a successful finish: {failed:?}"
        );
        assert_eq!(
            export.capture_storage().durable,
            Some(before),
            "the terminal link never became the durable frontier"
        );
        assert!(matches!(
            export.write_local_finish(&manifest(), at(10), "participant_stop", confirmed()),
            Err(ExportError::LocalFinishAlreadyChained { .. })
        ));
    }
}
