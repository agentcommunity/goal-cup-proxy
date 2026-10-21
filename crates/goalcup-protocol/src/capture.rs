//! M2 capture contracts: the bounded representation of recorder observations, the
//! synchronous submission seam and storage accounting, and the status projection.
//!
//! This module defines data, one trait and pure bound checks. It performs no I/O, redaction,
//! persistence or interpretation (D3): producers submit frames, the daemon sanitizes and
//! appends them as [`CaptureRecord`]s inside protocol-v5 envelopes, and the verifier reads
//! them later. Nothing here enables capture at runtime.
//!
//! Every wire type rejects unknown fields. Enums with fields are internally tagged by `kind`
//! and every enum variant is `snake_case`, so the JSON layout is the one frozen by
//! `crates/goalcup-protocol/tests/it/m2_contract.rs`.

use serde::{Deserialize, Serialize};

// Contract S1 (F6): the status projection moved out of this record module, which the v5 freeze
// covers, into `capture_status`. These re-exports keep every existing `capture::` import path
// working until S5 fixes the import sites and deletes them.
#[doc(hidden)]
pub use crate::capture_status::{
    AnalysisState, AnchorState, CaptureState, CaptureStorage, DeliveryState, DurableFrontier,
    M2Status, RunState,
};

/// The protocol version of every envelope this crate writes or reads (D54).
pub const M2_PROTOCOL_VERSION: u32 = 5;

/// Bound on one source chunk and, separately, on one sanitized segment's bytes.
pub const MAX_SEGMENT_BYTES: usize = 64 * 1024;
/// Launch resource bound on the decoded goal.
pub const MAX_GOAL_BYTES: usize = 64 * 1024;
/// One complete serialized envelope, including JSON and its trailing newline.
pub const MAX_ENVELOPE_BYTES: usize = 128 * 1024;
/// One encoded ingest frame body, checked against its length prefix before allocation.
pub const MAX_INGEST_FRAME_BYTES: usize = 512 * 1024;
/// Aggregate capture queue memory, controls included.
pub const MAX_CAPTURE_QUEUE_BYTES: usize = 8 * 1024 * 1024;
/// The logical byte quota of one entry's chain: admission accounting, never allocated. The run
/// host holds the run's one disk reservation.
pub const CHAIN_QUOTA_BYTES: u64 = 1 << 40;
/// Terminal allowance: part of, not additional to, the reservation.
pub const TERMINAL_RESERVE_BYTES: u64 = 1024 * 1024;
/// Simultaneously open payload streams per entry.
pub const MAX_OPEN_STREAMS: usize = 128;
/// Active captured connections per entry.
pub const MAX_OPEN_CONNECTIONS: usize = 128;
/// Distinct producers per entry, completed epochs included. Every recovered epoch spends one,
/// so the bound outlasts a week of stalls (D58): one every 9.2 s for 168 hours.
pub const MAX_PRODUCERS: usize = 65_536;
/// Producers of one entry live at the same time. Every bound that holds memory or queue
/// capacity per live producer uses this one, not [`MAX_PRODUCERS`] (D58).
pub const MAX_LIVE_PRODUCERS: usize = 128;
/// Each metadata string is at most 1 KiB.
pub const MAX_METADATA_STRING_BYTES: usize = 1024;
/// All metadata strings of one stream total at most 8 KiB.
pub(crate) const MAX_METADATA_BYTES: usize = 8 * 1024;
/// The separate first handshake object `{"entry_id","producer","token"}` is at most 1 KiB.
pub const MAX_HANDSHAKE_BYTES: usize = 1024;

/// One payload stream: a producer epoch and its producer-local counter. Stream zero is
/// reserved for [`FrameEvent::Connection`], [`FrameEvent::Recovery`] and
/// [`FrameEvent::Loss`] controls.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamKey {
    pub producer: String,
    pub stream: u64,
}

/// One observation instant. `mono_ns` is comparable only with another value of the same
/// `clock_id`; see [`CaptureTime::mono_elapsed_ns`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureTime {
    pub wall: String,
    pub mono_ns: u64,
    pub clock_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSource {
    WireRequest,
    WireResponse,
    WsClient,
    WsServer,
    Native,
    Goal,
}

/// Recorder transport identity of a wire stream. Provider IDs never substitute for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireIdentity {
    pub connection: String,
    pub exchange: u64,
    pub message_index: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionOutcome {
    Complete,
    Interrupted,
    TransportError,
}

/// Decoded through [`ConnectionEventWire`] so that the fieldless `UpgradedWebSocket` rejects
/// unknown fields too: serde's internally tagged *unit* variants ignore `deny_unknown_fields`.
/// The serialized layout is the derived one, unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", from = "ConnectionEventWire")]
pub enum ConnectionEvent {
    Open {
        alpn: Option<String>,
        already_open: bool,
    },
    UpgradedWebSocket,
    Close {
        outcome: ConnectionOutcome,
    },
}

/// The strict decoding mirror of [`ConnectionEvent`]: identical JSON, with the fieldless
/// variant written as an empty struct variant so `deny_unknown_fields` applies to it.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ConnectionEventWire {
    Open {
        alpn: Option<String>,
        already_open: bool,
    },
    UpgradedWebSocket {},
    Close {
        outcome: ConnectionOutcome,
    },
}

impl From<ConnectionEventWire> for ConnectionEvent {
    fn from(wire: ConnectionEventWire) -> Self {
        match wire {
            ConnectionEventWire::Open { alpn, already_open } => {
                ConnectionEvent::Open { alpn, already_open }
            }
            ConnectionEventWire::UpgradedWebSocket {} => ConnectionEvent::UpgradedWebSocket,
            ConnectionEventWire::Close { outcome } => ConnectionEvent::Close { outcome },
        }
    }
}

/// The whitelisted, sanitized metadata of one stream. `path` never carries a query or
/// fragment; there are no headers, cookies or arbitrary maps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureMeta {
    pub wire: Option<WireIdentity>,
    pub label: String,
    pub protocol_hint: Option<String>,
    pub method: Option<String>,
    pub host: Option<String>,
    pub path: Option<String>,
    pub status: Option<u16>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    Complete,
    Interrupted,
    DecodeError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapReason {
    QueueFull,
    IngestLost,
    ProducerLost,
    StorageFull,
    StorageIo,
    UnsupportedEncoding,
    DecodeFailed,
    SourceChanged,
    LimitExceeded,
    ShutdownDeadline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FrameEvent {
    Connection {
        connection: String,
        event: ConnectionEvent,
    },
    Recovery {
        previous_producer: String,
        reason: GapReason,
    },
    /// The producer relayed something it did not capture, outside any payload stream: a
    /// fail-open tunnel or opaque fallback (`DecodeFailed`), a full capture table
    /// (`LimitExceeded`), a connection cut while unsettled (`ShutdownDeadline`). Stream zero.
    /// The daemon records it as a stream-less gap on the producer's own clock.
    Loss {
        reason: GapReason,
    },
    Open {
        source: CaptureSource,
        metadata: CaptureMeta,
    },
    Bytes {
        offset: u64,
        data: Vec<u8>,
    },
    End {
        reason: EndReason,
    },
    Gap {
        reason: GapReason,
        lost_bytes: Option<u64>,
    },
}

/// One producer observation. `seq` starts at zero and increments across all of that
/// producer's frames, controls included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureFrame {
    pub stream: StreamKey,
    pub seq: u64,
    pub observed: CaptureTime,
    pub event: FrameEvent,
}

/// One retained, sanitized observation: the body of an [`crate::envelope::EventBody::Capture`]
/// in a protocol-v5 envelope. `LocalFinish` is reserved to the export's terminal writer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CaptureRecord {
    ProducerClosed {
        producer: String,
        observed: CaptureTime,
        final_seq: Option<u64>,
    },
    Connection {
        producer: String,
        connection: String,
        observed: CaptureTime,
        event: ConnectionEvent,
    },
    Recovery {
        previous_producer: String,
        producer: String,
        observed: CaptureTime,
        reason: GapReason,
    },
    Open {
        stream: StreamKey,
        observed: CaptureTime,
        source: CaptureSource,
        metadata: CaptureMeta,
    },
    Segment {
        stream: StreamKey,
        part: u64,
        source_offset: u64,
        source_len: u64,
        first: CaptureTime,
        last: CaptureTime,
        data_b64: String,
        audit: crate::redaction::RedactionAudit,
    },
    End {
        stream: StreamKey,
        observed: CaptureTime,
        reason: EndReason,
    },
    Gap {
        stream: Option<StreamKey>,
        observed: CaptureTime,
        reason: GapReason,
        lost_bytes: Option<u64>,
    },
    Launch {
        observed: CaptureTime,
        goal_stream: StreamKey,
        profile_id: String,
        profile_digest: String,
        harness_version: String,
        recorder_revision: String,
        continuation: String,
        /// A tool allowance the launch claimed, as bounded text, always paired with
        /// `process_handoff_digest` (SHA-256 hex). No daemon writes the pair since D52; the
        /// reader still admits it, by shape alone, so older traces stay valid.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        allowed_tool_rule: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        process_handoff_digest: Option<String>,
        /// The entry's per-run Ed25519 public key, 64 lowercase hex. Written by the daemon
        /// from the key it minted at create; never supplied by the CLI. Absent in traces
        /// written before contract S1, and valid forever in v5.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_public_key: Option<String>,
        /// `connector_dir_sha256` of the connector directory the launch read, 64 lowercase
        /// hex. Compared by a server against its allowlist; never interpreted by the reader.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        connector_dir_sha256: Option<String>,
        /// Which harness build ran, observed by the daemon after `exec` (D53). Added to v5
        /// after the `v0.5.0` tag: a v0.5.0 reader refuses a trace that carries it, which D53
        /// accepts. Absent in older traces, and valid forever.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        /// Boxed so the record enum stays small; the bytes are the same.
        harness_build: Option<Box<HarnessBuildV1>>,
    },
    Assistance {
        observed: CaptureTime,
        action: String,
    },
    LocalFinish {
        observed: CaptureTime,
        reason: String,
        termination: crate::provenance::TerminationOutcomeV1,
        seal_manifest_sha256: String,
    },
}

/// The harness build a launch ran, as the daemon observed it (D53). Recorded, never judged by
/// the reader: a server or judge compares the digests with a list of known builds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessBuildV1 {
    /// SHA-256 of the program file the launcher resolved and registered, 64 lowercase hex. For
    /// a script or a package-manager wrapper this is the only digest that names the harness.
    /// Absent when the registered program was not an absolute path or could not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_sha256: Option<String>,
    /// SHA-256 of the running process's executable image, 64 lowercase hex: on Linux the file
    /// `/proc/<pid>/exe` opens, on macOS the path `proc_pidpath` names. For an interpreted
    /// harness this is the interpreter, and for a wrapper that `exec`s again it may be either
    /// stage. Absent when the process had already gone, was not the registered instance, or its
    /// image could not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_sha256: Option<String>,
    /// The daemon's platform, `<os>-<arch>` as Rust spells them (`macos-aarch64`).
    pub platform: String,
}

/// Synchronous admission of one frame or close. `Queued` is volatile acceptance only: never
/// durability, never a receiver acknowledgement.
///
/// An in-process result, not a wire type: it is never serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitResult {
    Queued,
    Disabled(GapReason),
}

/// Synchronous admission of a producer epoch. `Ready` permits that ID; `Pending` keeps it
/// across polls of one attempt; `RetryFresh` retires it (a handshake may have reached the
/// receiver); `Disabled` is a locally known terminal failure.
///
/// An in-process result, not a wire type: it is never serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochAdmission {
    Ready,
    Pending,
    RetryFresh,
    Disabled(GapReason),
}

/// The one submission seam shared by the IPC sender and the direct component sink.
/// Every method is synchronous and bounded and never waits on disk or network.
pub trait CaptureSink: Send + Sync {
    fn try_submit(&self, frame: CaptureFrame) -> SubmitResult;
    fn try_begin_epoch(&self, producer: &str) -> EpochAdmission;
    fn try_close_epoch(&self, producer: &str, final_seq: Option<u64>) -> SubmitResult;
}

/// One length-prefixed item on an authenticated ingest connection after its handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
// The shared interface fixes `Frame(CaptureFrame)`; boxing it would change that interface.
#[allow(clippy::large_enum_variant)]
pub enum IngestMessage {
    Frame(CaptureFrame),
    Close {
        producer: String,
        final_seq: Option<u64>,
    },
}

/// Why a capture value breaks a bound or representation rule. Messages name fields
/// and sizes only; they never echo the rejected content.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CaptureLimitError {
    #[error("{field} is {len} bytes, over the {limit}-byte limit")]
    TooLong {
        field: &'static str,
        len: usize,
        limit: usize,
    },
    #[error("capture metadata totals {len} bytes, over the {limit}-byte aggregate limit")]
    MetadataTooLarge { len: usize, limit: usize },
    #[error("{field}: {reason}")]
    Invalid {
        field: &'static str,
        reason: &'static str,
    },
    #[error("ingest item declares {len} bytes, over the {limit}-byte limit")]
    FrameTooLarge { len: usize, limit: usize },
    #[error("ingest item is not a valid capture message")]
    Malformed,
}

fn invalid(field: &'static str, reason: &'static str) -> CaptureLimitError {
    CaptureLimitError::Invalid { field, reason }
}

/// A fresh random 128-bit identifier: exactly 32 lowercase hex digits.
pub fn is_capture_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn check_id(field: &'static str, value: &str) -> Result<(), CaptureLimitError> {
    if is_capture_id(value) {
        Ok(())
    } else {
        Err(invalid(field, "must be 32 lowercase hex digits"))
    }
}

/// A bounded display string: at most [`MAX_METADATA_STRING_BYTES`] and no control bytes.
/// Returns its length.
pub fn check_text(field: &'static str, value: &str) -> Result<usize, CaptureLimitError> {
    if value.len() > MAX_METADATA_STRING_BYTES {
        return Err(CaptureLimitError::TooLong {
            field,
            len: value.len(),
            limit: MAX_METADATA_STRING_BYTES,
        });
    }
    if value.bytes().any(|b| b.is_ascii_control()) {
        return Err(invalid(field, "must not contain control characters"));
    }
    Ok(value.len())
}

impl CaptureTime {
    /// Elapsed monotonic nanoseconds from `self` to `later`, only when both name the same
    /// clock. Two clocks are never subtracted: `None` rather than a meaningless difference,
    /// and `None` when `later` precedes `self`.
    pub fn mono_elapsed_ns(&self, later: &CaptureTime) -> Option<u64> {
        if self.clock_id != later.clock_id {
            return None;
        }
        later.mono_ns.checked_sub(self.mono_ns)
    }

    pub fn validate(&self) -> Result<(), CaptureLimitError> {
        crate::provenance::Rfc3339Utc::try_from(self.wall.as_str())
            .map_err(|_| invalid("observed.wall", "must be RFC 3339 UTC ending in Z"))?;
        if self.clock_id.is_empty() {
            return Err(invalid("observed.clock_id", "must not be empty"));
        }
        check_text("observed.clock_id", &self.clock_id).map(|_| ())
    }
}

impl CaptureMeta {
    /// Checks every metadata bound by length alone, before anything is serialized.
    pub fn validate(&self) -> Result<(), CaptureLimitError> {
        let mut total = check_text("metadata.label", &self.label)?;
        for (field, value) in [
            ("metadata.protocol_hint", &self.protocol_hint),
            ("metadata.method", &self.method),
            ("metadata.host", &self.host),
            ("metadata.path", &self.path),
            ("metadata.content_type", &self.content_type),
        ] {
            if let Some(value) = value {
                total += check_text(field, value)?;
            }
        }
        if let Some(wire) = &self.wire {
            check_id("metadata.wire.connection", &wire.connection)?;
            total += wire.connection.len();
        }
        if total > MAX_METADATA_BYTES {
            return Err(CaptureLimitError::MetadataTooLarge {
                len: total,
                limit: MAX_METADATA_BYTES,
            });
        }
        if self
            .path
            .as_deref()
            .is_some_and(|path| path.contains(['?', '#']))
        {
            return Err(invalid("metadata.path", "must exclude query and fragment"));
        }
        Ok(())
    }

    /// Wire streams carry a [`WireIdentity`]; HTTP bodies have no message index, WebSocket
    /// messages always do; native and goal streams carry none.
    fn validate_for(&self, source: CaptureSource) -> Result<(), CaptureLimitError> {
        self.validate()?;
        let index = self.wire.as_ref().map(|wire| wire.message_index.is_some());
        match (source, index) {
            (CaptureSource::WireRequest | CaptureSource::WireResponse, Some(false))
            | (CaptureSource::WsClient | CaptureSource::WsServer, Some(true))
            | (CaptureSource::Native | CaptureSource::Goal, None) => Ok(()),
            _ => Err(invalid(
                "metadata.wire",
                "does not match the stream source (HTTP: no message index, WebSocket: one, native/goal: no wire)",
            )),
        }
    }
}

impl ConnectionEvent {
    fn validate(&self) -> Result<(), CaptureLimitError> {
        match self {
            ConnectionEvent::Open {
                alpn: Some(alpn), ..
            } if alpn != "http/1.1" => Err(invalid("event.alpn", "must be absent or http/1.1")),
            _ => Ok(()),
        }
    }
}

impl CaptureFrame {
    /// Checks this frame's own bounds and stream-zero rule. Sequence and stream-state
    /// transitions depend on the producer's history and belong to the receiver.
    pub fn validate(&self) -> Result<(), CaptureLimitError> {
        check_id("stream.producer", &self.stream.producer)?;
        self.observed.validate()?;
        let control = matches!(
            self.event,
            FrameEvent::Connection { .. } | FrameEvent::Recovery { .. } | FrameEvent::Loss { .. }
        );
        if control != (self.stream.stream == 0) {
            return Err(invalid(
                "stream.stream",
                "stream zero is reserved for, and required by, connection, recovery and loss controls",
            ));
        }
        match &self.event {
            FrameEvent::Connection { connection, event } => {
                check_id("event.connection", connection)?;
                event.validate()
            }
            FrameEvent::Recovery {
                previous_producer, ..
            } => {
                check_id("event.previous_producer", previous_producer)?;
                if *previous_producer == self.stream.producer {
                    return Err(invalid(
                        "event.previous_producer",
                        "a recovery names a different, earlier producer",
                    ));
                }
                Ok(())
            }
            FrameEvent::Open { source, metadata } => metadata.validate_for(*source),
            FrameEvent::Bytes { offset, data } => {
                if data.len() > MAX_SEGMENT_BYTES {
                    return Err(CaptureLimitError::TooLong {
                        field: "event.data",
                        len: data.len(),
                        limit: MAX_SEGMENT_BYTES,
                    });
                }
                offset
                    .checked_add(data.len() as u64)
                    .map(|_| ())
                    .ok_or_else(|| invalid("event.offset", "offset plus length overflows"))
            }
            FrameEvent::Loss { .. } | FrameEvent::End { .. } | FrameEvent::Gap { .. } => Ok(()),
        }
    }
}

impl IngestMessage {
    pub(crate) fn validate(&self) -> Result<(), CaptureLimitError> {
        match self {
            IngestMessage::Frame(frame) => frame.validate(),
            IngestMessage::Close { producer, .. } => check_id("close.producer", producer),
        }
    }
}

/// The body length a big-endian `u32` prefix declares, refused above `limit` before any
/// body byte is allocated. Use [`MAX_HANDSHAKE_BYTES`] for the handshake and
/// [`MAX_INGEST_FRAME_BYTES`] for every item after it.
pub fn checked_frame_len(prefix: [u8; 4], limit: usize) -> Result<usize, CaptureLimitError> {
    let len = u32::from_be_bytes(prefix) as usize;
    if len > limit {
        return Err(CaptureLimitError::FrameTooLarge { len, limit });
    }
    Ok(len)
}

/// Longest entry ID in an ingest handshake, and in the receiver's grant for that entry.
pub const MAX_INGEST_ENTRY_ID_BYTES: usize = 256;

/// The first item of every ingest connection, before any [`IngestMessage`]. Fields serialize in
/// the order `entry_id`, `producer`, `token`; unknown fields are refused; `Debug` never shows
/// the token. The sender encodes it with [`encode_ingest_handshake`] and the receiver decodes
/// it with [`decode_ingest_handshake`], so both sides share one definition of the bytes.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngestHandshake {
    pub entry_id: String,
    pub producer: String,
    pub token: String,
}

impl std::fmt::Debug for IngestHandshake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IngestHandshake")
            .field("entry_id", &self.entry_id)
            .field("producer", &self.producer)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// Whether `value` is an ingest entry ID: 1 to [`MAX_INGEST_ENTRY_ID_BYTES`] bytes without
/// control characters.
pub fn is_ingest_entry_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_INGEST_ENTRY_ID_BYTES
        && !value.bytes().any(|b| b.is_ascii_control())
}

/// The framed handshake (`u32` big-endian length plus compact JSON), or `None` over
/// [`MAX_HANDSHAKE_BYTES`]. JSON never shortens a string, so an over-limit raw length is
/// refused before anything is serialized. Only the size is checked here; the receiver's
/// [`decode_ingest_handshake`] judges the fields.
pub fn encode_ingest_handshake(entry_id: &str, producer: &str, token: &str) -> Option<Vec<u8>> {
    if entry_id.len() + producer.len() + token.len() > MAX_HANDSHAKE_BYTES {
        return None;
    }
    let body = serde_json::to_vec(&IngestHandshake {
        entry_id: entry_id.to_string(),
        producer: producer.to_string(),
        token: token.to_string(),
    })
    .ok()?;
    if body.len() > MAX_HANDSHAKE_BYTES {
        return None;
    }
    let mut framed = Vec::with_capacity(4 + body.len());
    framed.extend_from_slice(&(body.len() as u32).to_be_bytes());
    framed.extend_from_slice(&body);
    Some(framed)
}

/// Decode one handshake body (the bytes after its prefix): unknown fields refused, then an
/// ingest entry ID, a capture ID as the producer and a token that is not empty, or `None`.
pub fn decode_ingest_handshake(body: &[u8]) -> Option<IngestHandshake> {
    if body.len() > MAX_HANDSHAKE_BYTES {
        return None;
    }
    let handshake: IngestHandshake = serde_json::from_slice(body).ok()?;
    let valid = is_ingest_entry_id(&handshake.entry_id)
        && is_capture_id(&handshake.producer)
        && !handshake.token.is_empty();
    valid.then_some(handshake)
}

/// `u32` big-endian length plus compact JSON of a validated message. The message is
/// validated first, so a value over any bound is refused before it is serialized.
pub fn encode_ingest_message(message: &IngestMessage) -> Result<Vec<u8>, CaptureLimitError> {
    message.validate()?;
    let body = serde_json::to_vec(message).map_err(|_| CaptureLimitError::Malformed)?;
    if body.len() > MAX_INGEST_FRAME_BYTES {
        return Err(CaptureLimitError::FrameTooLarge {
            len: body.len(),
            limit: MAX_INGEST_FRAME_BYTES,
        });
    }
    let mut framed = Vec::with_capacity(4 + body.len());
    framed.extend_from_slice(&(body.len() as u32).to_be_bytes());
    framed.extend_from_slice(&body);
    Ok(framed)
}

/// Decode one ingest body (the bytes after its prefix): bounded, unknown fields refused,
/// then validated. The error never carries any of the input.
pub fn decode_ingest_message(body: &[u8]) -> Result<IngestMessage, CaptureLimitError> {
    if body.len() > MAX_INGEST_FRAME_BYTES {
        return Err(CaptureLimitError::FrameTooLarge {
            len: body.len(),
            limit: MAX_INGEST_FRAME_BYTES,
        });
    }
    let message: IngestMessage =
        serde_json::from_slice(body).map_err(|_| CaptureLimitError::Malformed)?;
    message.validate()?;
    Ok(message)
}

#[cfg(test)]
mod handshake_tests {
    use super::*;

    const PRODUCER: &str = "0123456789abcdef0123456789abcdef";

    /// The handshake line of the shared golden `tests/fixtures/m2-ingest/golden-stream.hex`.
    fn golden_framed_handshake() -> Vec<u8> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/m2-ingest/golden-stream.hex"
        );
        let text = std::fs::read_to_string(path).unwrap();
        hex::decode(text.lines().next().unwrap().trim()).unwrap()
    }

    #[test]
    fn the_handshake_codec_is_byte_exact_with_the_shared_golden() {
        let framed = golden_framed_handshake();
        let len = checked_frame_len(framed[..4].try_into().unwrap(), MAX_HANDSHAKE_BYTES).unwrap();
        assert_eq!(
            framed.len(),
            4 + len,
            "the first golden line is the handshake"
        );
        let handshake = decode_ingest_handshake(&framed[4..]).expect("the golden decodes");
        assert_eq!(handshake.entry_id, "entry-m2-0001");
        assert_eq!(handshake.producer, "7a3f9c21d4e58b06a1c2d3e4f5061728");
        assert_eq!(
            encode_ingest_handshake(&handshake.entry_id, &handshake.producer, &handshake.token),
            Some(framed),
            "fields in order entry_id, producer, token; compact JSON behind a length"
        );
        assert!(!format!("{handshake:?}").contains(&handshake.token));
    }

    #[test]
    fn the_handshake_decoder_refuses_unknown_fields_and_bad_values() {
        let ok = br#"{"entry_id":"e","producer":"7a3f9c21d4e58b06a1c2d3e4f5061728","token":"t"}"#;
        assert!(decode_ingest_handshake(ok).is_some());
        let bad: [&[u8]; 6] = [
            br#"{"entry_id":"e","producer":"7a3f9c21d4e58b06a1c2d3e4f5061728","token":"t","x":1}"#,
            br#"{"entry_id":"e","producer":"7A3F9C21D4E58B06A1C2D3E4F5061728","token":"t"}"#,
            br#"{"entry_id":"","producer":"7a3f9c21d4e58b06a1c2d3e4f5061728","token":"t"}"#,
            br#"{"entry_id":"e\u0001","producer":"7a3f9c21d4e58b06a1c2d3e4f5061728","token":"t"}"#,
            br#"{"entry_id":"e","producer":"7a3f9c21d4e58b06a1c2d3e4f5061728","token":""}"#,
            br#"{"entry_id":"e","producer":"7a3f9c21d4e58b06a1c2d3e4f5061728"}"#,
        ];
        for body in bad {
            assert!(
                decode_ingest_handshake(body).is_none(),
                "{}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn the_handshake_over_the_limit_is_refused_before_and_after_escaping() {
        let fixed = r#"{"entry_id":"e","producer":"","token":""}"#.len() + PRODUCER.len();
        let fits = MAX_HANDSHAKE_BYTES - fixed;
        let exact = encode_ingest_handshake("e", PRODUCER, &"x".repeat(fits)).unwrap();
        assert_eq!(exact.len(), 4 + MAX_HANDSHAKE_BYTES);
        assert!(decode_ingest_handshake(&exact[4..]).is_some());
        assert!(encode_ingest_handshake("e", PRODUCER, &"x".repeat(fits + 1)).is_none());
        // Raw input far over the limit is refused before serialization.
        assert!(encode_ingest_handshake("e", PRODUCER, &"x".repeat(64 * 1024)).is_none());
        // Short raw input that JSON escaping pushes over the limit.
        assert!(encode_ingest_handshake("e", PRODUCER, &"\u{1}".repeat(300)).is_none());
    }
}
