//! The Decoder v1 contract.
//!
//! A decoder implements [`DecoderV1`]: the one trait a server, an external crate or an in-tree
//! decoder codes against. It is registered in a
//! [`DecoderRegistry`](super::registry::DecoderRegistry) and driven by a
//! [`DecodeSession`](super::session::DecodeSession). Nothing here reads or validates a trace.

use goalcup_protocol::capture::{CaptureRecord, CaptureSource, CaptureTime, StreamKey};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;

/// The version of the decoder contract. Bumped only by a breaking change to the stable set:
/// [`DecoderV1`], [`StreamOpen`], [`DecoderFailure`], [`Observation`] (with [`SourceRef`] and
/// [`ObservationTimeRange`]), [`DecoderSummary`], [`ObservationSink`] and [`Disposition`],
/// `Registered`, `DecoderFactory`, `RegistryError`, `DecoderRegistry`, the aggregate
/// `DecodeOutcome`, its per-decoder `DecoderOutcome` element type, and reachable `DecoderFailed`,
/// `DecoderPhase`, `ObservationCounts`, `Undecoded`,
/// `Operations`, `Reference`, and `GoalReference` support types, and [`kinds`]. Public orchestration,
/// report and reader helpers such as `DecodeSession`, `decode_records`, `JsonLinesSink`,
/// `KindTally`, reader facts, `CaptureIntegrityReport`, `ResearchResult`, and `research_summary`
/// are outside this stable decoder set. A new Rust field may arrive through `#[non_exhaustive]`,
/// but any field addition that would enter research-report JSON requires a research-report schema
/// bump or an explicit projection layer; a kind's field vocabulary is versioned by the decoder's
/// version. Protocol types reachable through this boundary retain their own protocol-version
/// contract.
pub const DECODER_API_VERSION: u32 = 1;

/// The closed observation-kind vocabulary of research-report schema 1 and Decoder v1.
pub mod kinds {
    pub const MODEL_REQUEST: &str = "model_request";
    pub const MODEL: &str = "model";
    pub const MODEL_RESPONSE: &str = "model_response";
    pub const TOOL_CALL: &str = "tool_call";
    pub const TOOL_RESULT: &str = "tool_result";
    pub const USAGE: &str = "usage";
    pub const OUTCOME: &str = "outcome";
    pub const ERROR: &str = "error";
    /// A source-referenced decoder finding, or retained state, record or form not safely
    /// interpreted; never a decoded fact.
    pub const UNKNOWN: &str = "unknown";
    pub const HUMAN_MESSAGE: &str = "human_message";

    pub const ALL: &[&str] = &[
        MODEL_REQUEST,
        MODEL,
        MODEL_RESPONSE,
        TOOL_CALL,
        TOOL_RESULT,
        USAGE,
        OUTCOME,
        ERROR,
        UNKNOWN,
        HUMAN_MESSAGE,
    ];
}

/// The retained `Segment` parts of one stream an observation was decoded from. Only actual
/// segment ranges: operational records without parts never fabricate a segment zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SourceRef {
    pub stream: StreamKey,
    pub first_part: u64,
    pub last_part: u64,
}

impl SourceRef {
    pub fn new(stream: StreamKey, first_part: u64, last_part: u64) -> Self {
        Self {
            stream,
            first_part,
            last_part,
        }
    }
}

/// The observation interval of the contributing retained segments: an interval, never an
/// invented event instant or a precise latency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ObservationTimeRange {
    pub first: CaptureTime,
    pub last: CaptureTime,
}

impl ObservationTimeRange {
    pub fn new(first: CaptureTime, last: CaptureTime) -> Self {
        Self { first, last }
    }
}

/// One decoded, source-referenced fact. `fields` holds supplied facts, not guessed intent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Observation {
    pub source: SourceRef,
    pub observed: ObservationTimeRange,
    pub kind: String,
    pub supplied_id: Option<String>,
    pub fields: serde_json::Value,
}

impl Observation {
    /// An observation with no supplied id; add one with [`with_supplied_id`](Self::with_supplied_id).
    pub fn new(
        source: SourceRef,
        observed: ObservationTimeRange,
        kind: impl Into<String>,
        fields: serde_json::Value,
    ) -> Self {
        Self {
            source,
            observed,
            kind: kind.into(),
            supplied_id: None,
            fields,
        }
    }

    /// The id the source supplied for this fact (a response id, a tool call id).
    pub fn with_supplied_id(mut self, id: impl Into<String>) -> Self {
        self.supplied_id = Some(id.into());
        self
    }
}

/// What one decoder did over one trace. Unknown and over-budget records are counted, never
/// reported as zeros.
///
/// A decoder builds it with [`new`](Self::new); the session stamps `decoder_id` and
/// `decoder_version` from the registry entry, so no decoder names itself in a report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DecoderSummary {
    pub decoder_id: String,
    pub decoder_version: String,
    pub observations: u64,
    pub unknown_records: u64,
    pub incomplete: bool,
}

impl DecoderSummary {
    /// A summary whose identity the session fills in.
    pub fn new(observations: u64, unknown_records: u64, incomplete: bool) -> Self {
        Self {
            decoder_id: String::new(),
            decoder_version: String::new(),
            observations,
            unknown_records,
            incomplete,
        }
    }

    /// The summary with `decoder_id` and `decoder_version` set. The session does this for a
    /// decoder's summary; a test comparing against one does it too.
    pub fn with_identity(mut self, id: &str, version: &str) -> Self {
        self.decoder_id = id.to_string();
        self.decoder_version = version.to_string();
        self
    }
}

/// What a decoder is told about a stream when it opens. Built from the `Open` record.
///
/// Non-exhaustive: new `Open` metadata is added without breaking external decoders. A new
/// field arrives as `None` here, with a `with_*` setter, so `new` keeps its four arguments.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct StreamOpen<'a> {
    pub stream: &'a StreamKey,
    pub source: CaptureSource,
    pub protocol_hint: Option<&'a str>,
    pub content_type: Option<&'a str>,
}

impl<'a> StreamOpen<'a> {
    pub fn new(
        stream: &'a StreamKey,
        source: CaptureSource,
        protocol_hint: Option<&'a str>,
        content_type: Option<&'a str>,
    ) -> Self {
        Self {
            stream,
            source,
            protocol_hint,
            content_type,
        }
    }
}

/// A decoder's reason for giving up on a trace. Never carries record bytes: the reason is a
/// short token (the in-tree decoders use the `io::ErrorKind` word) that a report prints.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecoderFailure {
    pub reason: String,
}

impl DecoderFailure {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    /// The failure of an I/O error: its `io::ErrorKind` word, never the error's message.
    pub fn from_io(error: &io::Error) -> Self {
        Self::new(error.kind().to_string())
    }
}

impl fmt::Display for DecoderFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "decoder failed: {}", self.reason)
    }
}

impl std::error::Error for DecoderFailure {}

/// One decoder instance reads one trace. `Send` so a server can decode on worker threads.
///
/// A decoder runs no model, reads no credential and must not panic on any [`CaptureRecord`],
/// including one that arrived from outside. If it does, the session catches the panic and
/// ends that decoder alone (see [`DecodeSession`](super::session::DecodeSession)); that
/// isolation does not exist in a build that sets `panic = "abort"`. A panic message must not
/// carry record bytes: the session silences decoder panics on its own thread only (see
/// [`quiet_panic_hook`](super::session::quiet_panic_hook)).
///
/// A failed decoder, and one whose `finish` ran, is never called again and is dropped inside
/// the same isolation.
///
/// Driving a decoder outside a session is unsupported for the built-ins: each repeats its
/// selection as a `debug_assert!` guard, so in a debug build `push` panics on an `Open` that
/// `accepts` would have refused.
pub trait DecoderV1: Send {
    /// Stable id, `^[a-z][a-z0-9-]{0,63}$`: the report key, never reused for another format.
    /// The registry entry owns it; an instance that reports another one fails.
    fn id(&self) -> &'static str;
    /// Bumped on any change to emitted observations. Reports record it; an instance whose
    /// version differs from its registry entry fails.
    fn version(&self) -> &'static str;
    /// Whether this decoder reads the stream. A pure function of the `Open` metadata, and the
    /// only place a stream is selected for a decoder: `push` is never given the records of a
    /// stream `accepts` refused. A decoder may repeat the check as a guard, never as the
    /// selection.
    fn accepts(&self, open: &StreamOpen<'_>) -> bool;
    /// Records of accepted streams plus every stream-less record, in chain order.
    fn push(
        &mut self,
        record: &CaptureRecord,
        emit: &mut dyn FnMut(Observation),
    ) -> Result<(), DecoderFailure>;
    /// Session-resolved Recovery control. The provided implementation preserves Decoder v1
    /// source compatibility and delegates the ordinary recorded-predecessor case to [`Self::push`].
    /// A decoder that has not opted into expanded recovery scope fails closed before stale
    /// decoder-owned state can survive an unrecorded-predecessor cascade.
    #[doc(hidden)]
    fn push_recovery(
        &mut self,
        record: &CaptureRecord,
        lost_streams: &[StreamKey],
        emit: &mut dyn FnMut(Observation),
    ) -> Result<(), DecoderFailure> {
        let CaptureRecord::Recovery {
            previous_producer, ..
        } = record
        else {
            return Err(DecoderFailure::new("recovery_scope_unsupported"));
        };
        if lost_streams
            .iter()
            .all(|stream| &stream.producer == previous_producer)
        {
            self.push(record, emit)
        } else {
            Err(DecoderFailure::new("recovery_scope_unsupported"))
        }
    }
    /// Consumes the instance: a decoder is never reused across traces. The session stamps the
    /// returned summary's identity.
    fn finish(
        self: Box<Self>,
        emit: &mut dyn FnMut(Observation),
    ) -> Result<DecoderSummary, DecoderFailure>;
}

/// Whether the sink kept an observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Disposition {
    /// Stored or written.
    Kept,
    /// Dropped, not truncated, because it exceeded the sink's own size bound. The session
    /// counts it as over budget.
    OverBudget,
}

/// Where observations go: the server stores them, the CLI writes JSON lines.
pub trait ObservationSink {
    /// Takes one observation of decoder `decoder_id`. `Err` is a sink failure: it ends the
    /// session for good and is never attributed to the decoder.
    fn observe(
        &mut self,
        decoder_id: &'static str,
        observation: Observation,
    ) -> io::Result<Disposition>;
}

impl<F: FnMut(&'static str, Observation) -> io::Result<Disposition>> ObservationSink for F {
    fn observe(
        &mut self,
        decoder_id: &'static str,
        observation: Observation,
    ) -> io::Result<Disposition> {
        self(decoder_id, observation)
    }
}
