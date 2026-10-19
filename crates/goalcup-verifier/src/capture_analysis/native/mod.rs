//! Bounded native-transcript decoders: one [`NativeDecoder`] generic over a [`NativeFormat`].
//!
//! This file is the format-agnostic part: it reassembles a tracked stream's `Segment` bytes
//! into newline-delimited logical records (native transcripts are JSONL) and parses each one
//! independently with `serde_json::from_slice`, never by reconstructing a whole run in
//! memory. What a parsed record means is the format's: one file per format
//! ([`codex`], [`claude_code`]). A new native format is one more file that implements
//! [`NativeFormat`], plus one line in [`super::registry::DecoderRegistry::builtin`].
//!
//! A decoder reads only the `Open` streams whose `source` is [`CaptureSource::Native`] and
//! whose `protocol_hint` is exactly its format's [`NativeFormat::HINT`]; every other stream
//! and every other source is untouched.
//!
//! **Bounds.** One logical record is held per tracked stream at a time, capped at
//! [`MAX_LOGICAL_RECORD_BYTES`]; the sum across every tracked stream's pending bytes is capped
//! at [`MAX_TOTAL_BUFFERED_BYTES`]. Crossing either bound does not grow the buffer further:
//! the record becomes `unknown` with `reason: "oversize"` once its terminating newline is
//! found, and parsing resumes cleanly on the next record. No allocation here scales with total
//! run bytes, only with the number of open tracked streams, itself bounded by
//! [`goalcup_protocol::capture::MAX_OPEN_STREAMS`].
//!
//! **Completeness.** `DecoderSummary.incomplete` is `true` whenever this decoder's view of a
//! tracked stream's retained bytes has any gap: a `Gap` record on that stream (even between two
//! whole lines: a lost record leaves no trace of its own to point at, but the loss is real), an
//! `End{Interrupted | DecodeError}`, a `ProducerClosed` with no `End`, or a stream still open at
//! `finish` (a live-file EOF is never an analysis completion claim). This holds even for a
//! stream that never carried a byte: an interrupted *empty* stream is `incomplete`, distinct
//! from an `End{Complete}` empty one, which is valid empty input. An `unknown` *observation*
//! still needs at least one touched byte, because there is no `SourceRef` without a
//! contributing part.
//!
//! **Authorship.** `human_message` is emitted only behind a demonstrated positive marker on
//! the record itself, never from position in the stream or a text prefix. Each format file
//! names its marker. User text without a marker is counted as `unknown`.
//!
//! **Redaction.** A logical record whose contributing segment(s) carried a non-empty
//! redaction audit and then fails to parse becomes `unknown` with `reason: "redacted"` and
//! the union of the touching audit spans' `kind`s: never a guess at the bytes a fail-closed
//! rule replaced, and never an attempt to parse a truncated prefix.
//!
//! **Source timestamps.** Every observation of a parsed record carries
//! `fields.source_timestamp`, the record's own top-level `timestamp` copied verbatim when it
//! has one, separate from and never replacing `observed` (the segment-derived interval).

pub mod claude_code;
pub mod codex;

use super::api::{
    DecoderFailure, DecoderSummary, DecoderV1, Observation, ObservationTimeRange, SourceRef,
    StreamOpen, kinds,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use goalcup_protocol::capture::{
    CaptureRecord, CaptureSource, CaptureTime, EndReason, MAX_OPEN_STREAMS, MAX_SEGMENT_BYTES,
    StreamKey,
};
use goalcup_protocol::redaction::{RedactionAudit, RedactionAuditSpan};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// The Codex rollout-JSONL decoder (`codex-jsonl`).
pub type CodexNativeDecoder = NativeDecoder<codex::Codex>;
/// The Claude Code project-JSONL decoder (`claude-code-jsonl`).
pub type ClaudeNativeDecoder = NativeDecoder<claude_code::ClaudeCode>;

/// One native session format: its identity and what its parsed JSON records mean. Every
/// bounded byte-assembly rule is [`NativeDecoder`]'s; a format only interprets.
pub trait NativeFormat: 'static {
    /// The exact `protocol_hint` this format reads, also recorded in every observation's
    /// `fields.format`.
    const HINT: &'static str;
    /// The decoder id (see [`DecoderV1::id`]).
    const ID: &'static str;
    /// The decoder version (see [`DecoderV1::version`]).
    const VERSION: &'static str;
    /// What the format remembers about one stream across its records. Bounded and small.
    type Stream: Default + Send;

    /// One successfully parsed logical record of one stream. Emits at least one observation
    /// through `out`: a known kind, or `unknown` through [`NativeOut::unrecognized`].
    fn interpret(record: &Value, stream: &mut Self::Stream, out: &mut NativeOut<'_>);

    /// Lost or unreadable context. Formats may prevent a later copied header from becoming
    /// initial authority; existing formats need not change their interpretation.
    fn lost_context(_stream: &mut Self::Stream) {}
}

/// Walk a dotted path into a JSON value. `dotted(v, "payload.thread_source")`.
fn dotted<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = value;
    for segment in path.split('.') {
        cur = cur.get(segment)?;
    }
    Some(cur)
}

/// The prospective per-logical-record budget: 1 MiB per active native parser.
const MAX_LOGICAL_RECORD_BYTES: usize = 1024 * 1024;
/// The aggregate budget across every stream this decoder currently holds pending bytes for:
/// 8 MiB in total.
const MAX_TOTAL_BUFFERED_BYTES: usize = 8 * 1024 * 1024;
/// Distinct redaction-span kinds retained per redacted logical record. Bounded so a record
/// crossed by many spans never grows `unknown.fields.kinds` without limit.
const MAX_TRACKED_REDACTION_KINDS: usize = 16;
/// The padded standard base64 length of one full segment: the encoded bound checked
/// before decoding, mirroring `capture_analysis::reader`'s own defensive bound.
const MAX_SEGMENT_B64_BYTES: usize = MAX_SEGMENT_BYTES.div_ceil(3) * 4;
/// A bounded cap on the copied `source_timestamp` string: it is display text, not a parsed
/// clock, so this only guards against a pathological field, matching the protocol's metadata-string bound.
const MAX_SOURCE_TIMESTAMP_BYTES: usize = 1024;

/// One Segment's part number and observation interval, bundled so the byte-assembly helpers
/// below stay under clippy's argument-count bound.
struct SegmentStamp<'a> {
    part: u64,
    first: &'a CaptureTime,
    last: &'a CaptureTime,
}

/// The in-progress state of one not-yet-newline-terminated logical record on one tracked
/// stream. Reset every time a newline completes a record (or a Gap/End/producer loss
/// invalidates it), so nothing here ever grows with total run bytes.
#[derive(Default)]
struct LineState {
    pending: Vec<u8>,
    /// The first Segment `part` that contributed a byte to this line, so [`SourceRef`]
    /// names only the parts that actually contributed (never a fabricated part).
    first_part: Option<u64>,
    last_part: u64,
    /// Set once, from the first contributing segment's `first`, never overwritten.
    observed_first: Option<CaptureTime>,
    /// Overwritten by every contributing segment's `last`.
    observed_last: Option<CaptureTime>,
    /// True once any contributing segment carried a non-empty redaction audit.
    redacted: bool,
    redacted_kinds: BTreeSet<String>,
    /// True once this line's accumulated bytes crossed a budget; further bytes for it are
    /// not retained, only counted toward the eventual `unknown` observation.
    over_budget: bool,
}

impl LineState {
    /// Whether anything at all was ever contributed to this line (as opposed to a stream
    /// that opened, closed and never carried a byte: valid empty input).
    fn touched(&self) -> bool {
        self.observed_first.is_some()
    }

    fn source_ref(&self, stream: &StreamKey) -> SourceRef {
        SourceRef {
            stream: stream.clone(),
            first_part: self.first_part.unwrap_or(self.last_part),
            last_part: self.last_part,
        }
    }

    fn observed_range(&self) -> ObservationTimeRange {
        ObservationTimeRange {
            first: self
                .observed_first
                .clone()
                .expect("touched() checked by every caller"),
            last: self
                .observed_last
                .clone()
                .expect("set alongside observed_first"),
        }
    }
}

/// The decoder's observation counters.
#[derive(Default)]
struct Counts {
    observations: u64,
    unknown: u64,
}

/// Where a format sends the observations of one logical record. It stamps each with the
/// record's source reference, interval, `fields.format` and `fields.source_timestamp`, and
/// counts it.
pub struct NativeOut<'a> {
    hint: &'static str,
    source: &'a SourceRef,
    observed: &'a ObservationTimeRange,
    timestamp: Option<&'a str>,
    native_context: Option<Value>,
    counts: &'a mut Counts,
    emit: &'a mut dyn FnMut(Observation),
}

impl NativeOut<'_> {
    /// The current record's retained source, also usable as provenance for supplied identity.
    fn source_ref(&self) -> &SourceRef {
        self.source
    }

    /// Optional decoder-versioned source-session context. It labels the container, never
    /// the authorship or freshness of every record in it. Formats opt in explicitly.
    fn set_native_context(&mut self, context: Option<&Value>) {
        self.native_context = context.cloned();
    }

    /// One observation of `kind`. `fields` is a JSON object.
    fn kind(&mut self, kind: &str, supplied_id: Option<String>, mut fields: Value) {
        if let Value::Object(map) = &mut fields {
            map.insert("format".to_string(), Value::String(self.hint.to_string()));
            if let Some(context) = &self.native_context {
                map.insert("native_context".to_string(), context.clone());
            }
            if let Some(ts) = self.timestamp {
                map.insert(
                    "source_timestamp".to_string(),
                    Value::String(ts.to_string()),
                );
            }
        }
        (self.emit)(Observation {
            source: self.source.clone(),
            observed: self.observed.clone(),
            kind: kind.to_string(),
            supplied_id,
            fields,
        });
        self.counts.observations += 1;
    }

    /// One bounded `unknown` observation: `fields.reason` names why, `extra` carries at most
    /// one additional bounded fact (`kinds` for a redacted record).
    fn unknown(&mut self, reason: &str, extra: Option<(&str, Value)>) {
        let mut fields = json!({"reason": reason});
        if let Some((key, value)) = extra {
            fields[key] = value;
        }
        self.kind(kinds::UNKNOWN, None, fields);
        self.counts.unknown += 1;
    }

    /// A parseable record whose shape the format does not extract meaning from. Counted and
    /// source-referenced, never silently dropped, never guessed at.
    fn unrecognized(&mut self, record_type: Option<&str>) {
        let extra = record_type.map(|t| ("record_type", Value::String(t.to_string())));
        self.unknown("unrecognized", extra);
    }
}

/// The bounded decoder of native format `F`. Reads only streams opened with
/// [`CaptureSource::Native`] and exactly `F::HINT`.
pub struct NativeDecoder<F: NativeFormat> {
    streams: BTreeMap<StreamKey, (LineState, F::Stream)>,
    /// Sum of `pending.len()` across every entry of `streams`, kept in step with every
    /// mutation so the total budget check never re-walks the map.
    total_buffered: usize,
    counts: Counts,
    incomplete: bool,
}

impl<F: NativeFormat> NativeDecoder<F> {
    pub fn new() -> Self {
        Self {
            streams: BTreeMap::new(),
            total_buffered: 0,
            counts: Counts::default(),
            incomplete: false,
        }
    }

    /// The one selection rule; `accepts` and the `Open` guard both call it.
    fn selects(source: CaptureSource, hint: Option<&str>) -> bool {
        source == CaptureSource::Native && hint == Some(F::HINT)
    }

    fn read(&mut self, record: &CaptureRecord, emit: &mut dyn FnMut(Observation)) {
        match record {
            CaptureRecord::Open {
                stream,
                source,
                metadata,
                ..
            } => {
                let selected = Self::selects(*source, metadata.protocol_hint.as_deref());
                // `accepts` selected this stream; this is a guard, not the selection.
                debug_assert!(selected, "a stream `accepts` refused was routed here");
                if selected && self.streams.len() < MAX_OPEN_STREAMS {
                    self.streams.entry(stream.clone()).or_default();
                }
            }
            CaptureRecord::Segment {
                stream,
                part,
                first,
                last,
                data_b64,
                audit,
                ..
            } => {
                if self.streams.contains_key(stream) {
                    let stamp = SegmentStamp {
                        part: *part,
                        first,
                        last,
                    };
                    self.ingest_segment(stream, &stamp, data_b64, audit, emit);
                }
            }
            CaptureRecord::Gap {
                stream: Some(stream),
                ..
            } => {
                // A gap on a tracked stream always means lost coverage, whether or not a
                // partial line happened to be mid-flight: two whole records either side of a
                // gap can still lose a whole record in between, with nothing left to point a
                // SourceRef at. Never parse across it either way.
                self.invalidate_line(stream, emit);
            }
            CaptureRecord::End { stream, reason, .. } => {
                if let Some((line, _)) = self.streams.remove(stream) {
                    self.finalize_removed(stream, line, *reason, emit);
                }
            }
            CaptureRecord::ProducerClosed { producer, .. } => {
                // No End arrived for this producer's tracked streams (if any): the same
                // "invalidate partial assembly" rule as a Gap, since nothing more will come.
                let lost: Vec<StreamKey> = self
                    .streams
                    .keys()
                    .filter(|key| &key.producer == producer)
                    .cloned()
                    .collect();
                for stream in lost {
                    if let Some((line, _)) = self.streams.remove(&stream) {
                        self.finalize_removed(&stream, line, EndReason::Interrupted, emit);
                    }
                }
            }
            _ => {}
        }
    }

    /// Lost coverage on a still-open tracked stream (a `Gap`, or a segment whose bytes cannot
    /// be recovered): whatever was mid-flight is reported `interrupted`, never parsed across.
    fn invalidate_line(&mut self, stream: &StreamKey, emit: &mut dyn FnMut(Observation)) {
        if let Some((line, state)) = self.streams.get_mut(stream) {
            F::lost_context(state);
            self.incomplete = true;
            let taken = std::mem::take(line);
            self.total_buffered -= taken.pending.len();
            if taken.touched() {
                self.unknown(stream, &taken, "interrupted", None, emit);
            }
        }
    }

    /// A stream closed (`End`, or a producer lost with no `End`). `reason == Complete` from a
    /// real `End` is the only clean case; anything else, including reaching here with no
    /// `End` at all, means this stream's coverage is not proven complete, whether or not it
    /// ever carried a byte.
    fn finalize_removed(
        &mut self,
        stream: &StreamKey,
        line: LineState,
        reason: EndReason,
        emit: &mut dyn FnMut(Observation),
    ) {
        self.total_buffered -= line.pending.len();
        let clean = reason == EndReason::Complete;
        if !line.touched() {
            if !clean {
                self.incomplete = true;
            }
            return; // no SourceRef without a contributing part: nothing to emit either way.
        }
        self.incomplete = true;
        let word = match reason {
            // A clean End with a dangling, never-newline-terminated tail: an honest partial,
            // not a discarded successful record.
            EndReason::Complete => "incomplete",
            EndReason::Interrupted | EndReason::DecodeError => "interrupted",
        };
        self.unknown(stream, &line, word, None, emit);
    }

    fn ingest_segment(
        &mut self,
        stream: &StreamKey,
        stamp: &SegmentStamp<'_>,
        data_b64: &str,
        audit: &RedactionAudit,
        emit: &mut dyn FnMut(Observation),
    ) {
        if data_b64.len() > MAX_SEGMENT_B64_BYTES {
            // Defensive only (an upstream-validated chain never exceeds this), but this
            // decoder stays independently bounded rather than trusting the caller.
            if let Some((line, _)) = self.streams.get_mut(stream) {
                line.over_budget = true;
                self.total_buffered -= line.pending.len();
                line.pending.clear();
            }
            return;
        }
        let mut raw = Vec::new();
        if STANDARD.decode_vec(data_b64.as_bytes(), &mut raw).is_err() {
            // Cannot recover any bytes from this segment: treat it like a gap on this stream.
            self.invalidate_line(stream, emit);
            return;
        }
        let has_audit = !audit.spans.is_empty();
        let mut idx = 0;
        while idx < raw.len() {
            match raw[idx..].iter().position(|&b| b == b'\n') {
                Some(offset) => {
                    let end = idx + offset;
                    self.append_chunk(stream, stamp, &raw[idx..end], has_audit, &audit.spans);
                    self.complete_line(stream, emit);
                    idx = end + 1;
                }
                None => {
                    self.append_chunk(stream, stamp, &raw[idx..], has_audit, &audit.spans);
                    idx = raw.len();
                }
            }
        }
    }

    /// Append one newline-free chunk to `stream`'s in-progress line, tracking part/interval/
    /// audit metadata regardless of budget, then enforcing both budgets before the bytes
    /// themselves are retained.
    fn append_chunk(
        &mut self,
        stream: &StreamKey,
        stamp: &SegmentStamp<'_>,
        chunk: &[u8],
        has_audit: bool,
        spans: &[RedactionAuditSpan],
    ) {
        let Some((line, _)) = self.streams.get_mut(stream) else {
            return;
        };
        if line.first_part.is_none() {
            line.first_part = Some(stamp.part);
        }
        line.last_part = stamp.part;
        if line.observed_first.is_none() {
            line.observed_first = Some(stamp.first.clone());
        }
        line.observed_last = Some(stamp.last.clone());
        if has_audit {
            line.redacted = true;
            for span in spans {
                if line.redacted_kinds.len() >= MAX_TRACKED_REDACTION_KINDS {
                    break;
                }
                line.redacted_kinds.insert(span.kind.clone());
            }
        }
        if line.over_budget || chunk.is_empty() {
            return;
        }
        let projected_total = self.total_buffered + chunk.len();
        if line.pending.len() + chunk.len() > MAX_LOGICAL_RECORD_BYTES
            || projected_total > MAX_TOTAL_BUFFERED_BYTES
        {
            self.total_buffered -= line.pending.len();
            line.pending.clear();
            line.over_budget = true;
            return;
        }
        line.pending.extend_from_slice(chunk);
        self.total_buffered += chunk.len();
    }

    /// A newline was found: finalize whatever this stream's line currently holds and reset
    /// it, ready for the next logical record on the same still-open stream.
    fn complete_line(&mut self, stream: &StreamKey, emit: &mut dyn FnMut(Observation)) {
        let Some((line, _)) = self.streams.get_mut(stream) else {
            return;
        };
        let line = std::mem::take(line);
        self.total_buffered -= line.pending.len();
        if !line.touched() {
            return; // a blank line: not a record, not an error.
        }
        if line.over_budget {
            self.lose_context(stream);
            self.unknown(stream, &line, "oversize", None, emit);
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&line.pending) else {
            self.lose_context(stream);
            if line.redacted {
                let kinds: Vec<Value> = line
                    .redacted_kinds
                    .iter()
                    .map(|k| Value::String(k.clone()))
                    .collect();
                let extra = Some(("kinds", Value::Array(kinds)));
                self.unknown(stream, &line, "redacted", extra, emit);
            } else {
                self.unknown(stream, &line, "malformed", None, emit);
            }
            return;
        };
        // A source-supplied timestamp, kept separately labelled in every emitted
        // observation's fields and never replacing `observed`.
        let timestamp = value
            .get("timestamp")
            .and_then(Value::as_str)
            .filter(|s| s.len() <= MAX_SOURCE_TIMESTAMP_BYTES);
        let source = line.source_ref(stream);
        let observed = line.observed_range();
        let Some((_, state)) = self.streams.get_mut(stream) else {
            return; // `complete_line` found the entry above, and nothing removed it since.
        };
        let mut out = NativeOut {
            hint: F::HINT,
            source: &source,
            observed: &observed,
            timestamp,
            native_context: None,
            counts: &mut self.counts,
            emit,
        };
        F::interpret(&value, state, &mut out);
    }

    fn lose_context(&mut self, stream: &StreamKey) {
        if let Some((_, state)) = self.streams.get_mut(stream) {
            F::lost_context(state);
        }
    }

    /// An `unknown` observation over `line` that comes from assembly, not interpretation: it
    /// carries no `source_timestamp`, because no record was parsed.
    fn unknown(
        &mut self,
        stream: &StreamKey,
        line: &LineState,
        reason: &str,
        extra: Option<(&str, Value)>,
        emit: &mut dyn FnMut(Observation),
    ) {
        let source = line.source_ref(stream);
        let observed = line.observed_range();
        NativeOut {
            hint: F::HINT,
            source: &source,
            observed: &observed,
            timestamp: None,
            native_context: None,
            counts: &mut self.counts,
            emit,
        }
        .unknown(reason, extra);
    }

    fn summarize(&mut self, emit: &mut dyn FnMut(Observation)) -> DecoderSummary {
        let streams = std::mem::take(&mut self.streams);
        for (stream, (line, _)) in streams {
            self.total_buffered -= line.pending.len();
            // A stream still open when the input ends had no `End` at all: a temporary
            // live-file EOF is never an analysis completion claim, so this is
            // incomplete regardless of whether a partial line happened to be pending.
            self.incomplete = true;
            if line.touched() {
                self.unknown(&stream, &line, "incomplete", None, emit);
            }
        }
        DecoderSummary::new(
            self.counts.observations,
            self.counts.unknown,
            self.incomplete,
        )
    }
}

impl<F: NativeFormat> Default for NativeDecoder<F> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F: NativeFormat> DecoderV1 for NativeDecoder<F> {
    fn id(&self) -> &'static str {
        F::ID
    }

    fn version(&self) -> &'static str {
        F::VERSION
    }

    fn accepts(&self, open: &StreamOpen<'_>) -> bool {
        Self::selects(open.source, open.protocol_hint)
    }

    fn push(
        &mut self,
        record: &CaptureRecord,
        emit: &mut dyn FnMut(Observation),
    ) -> Result<(), DecoderFailure> {
        self.read(record, emit);
        Ok(())
    }

    fn finish(
        mut self: Box<Self>,
        emit: &mut dyn FnMut(Observation),
    ) -> Result<DecoderSummary, DecoderFailure> {
        Ok(self.summarize(emit))
    }
}
