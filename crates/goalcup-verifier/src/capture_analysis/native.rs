//! Bounded native-transcript decoders (`codex-jsonl`, `claude-code-jsonl`).
//!
//! [`CodexNativeDecoder`] and [`ClaudeNativeDecoder`] implement [`super::api::DecoderV1`].
//! Each filters the ordered [`CaptureRecord`] stream to the `Open` streams
//! whose `source` is [`CaptureSource::Native`] and whose `protocol_hint` is exactly its own
//! (`codex-jsonl` or `claude-code-jsonl`); every other stream and every other
//! source is untouched. A tracked stream's `Segment` bytes are reassembled newline-delimited
//! (native transcripts are JSONL) and each complete logical record is decoded independently
//! with `serde_json::from_slice`, never by reconstructing an entire run in memory.
//!
//! **Bounds.** One logical (newline-delimited) record is held per tracked stream at a time,
//! capped at [`MAX_LOGICAL_RECORD_BYTES`]; the sum across every tracked stream's pending
//! bytes is capped at [`MAX_TOTAL_BUFFERED_BYTES`]. Crossing either bound does not grow the
//! buffer further: the record becomes `unknown` with `reason: "oversize"` once its
//! terminating newline is found, and parsing resumes cleanly on the next record. No
//! allocation here scales with total run bytes — only with the number of currently open
//! tracked streams, itself bounded by [`goalcup_protocol::capture::MAX_OPEN_STREAMS`].
//!
//! **Completeness.** `DecoderSummary.incomplete` is `true` whenever this
//! decoder's view of a tracked stream's retained bytes has any gap: a `Gap` record on that
//! stream (even between two otherwise-whole lines — a lost record leaves no trace of its own
//! to point at, but the loss itself is still real), an `End{Interrupted | DecodeError}`, a
//! `ProducerClosed` with no `End`, or a stream still open when [`NativeAssembler::finish`]
//! runs (a live-file EOF is never an analysis completion claim — a `finish()` with no `End`
//! is exactly that). This holds even for a stream that never carried a single byte: an
//! interrupted *empty* stream is `incomplete`, distinct from a genuinely `End{Complete}`
//! empty one, which is valid empty input. An `unknown` *observation* still requires at least
//! one touched byte, because there is no `SourceRef` without a contributing part.
//!
//! **Authorship.** `human_message` is emitted only behind a demonstrated positive marker on
//! the record itself. Position in the stream is never authorship, and no text prefix is
//! read. Human involvement in a headless run is evidenced by `Assistance` records, not by
//! native text. User text without a marker is counted as `unknown`.
//! - Codex: `internal_chat_message_metadata_passthrough.content_item_kinds` containing
//!   `"user.text"` on a `response_item` message of `role: "user"` is the marker (an injected
//!   `<environment_context>` user-role message carries `["environments.environment_context"]`
//!   instead). It counts only on a stream whose `session_meta.payload.source` is `"cli"` or
//!   `"vscode"`. An `exec` launch has no input channel: its first marked message is the
//!   tracker's own delivered goal, which carries the identical marker (measured on real
//!   bytes), and no later one is claimed either. A stream with no `session_meta` before the
//!   message has an unknown launch, so nothing on it is claimed.
//! - Claude Code: `origin.kind == "human"` (2.1.259) or `promptSource == "typed"` on a
//!   structurally eligible `type: "user"` record is the marker. `isMeta`, `isSidechain` or
//!   `isCompactSummary` true, `promptSource == "system"`, or any other `origin.kind` makes a
//!   record ineligible. 2.1.283 carries no `origin` field, and its sdk-delivered goal carries
//!   `promptSource: "sdk"` (measured on real bytes), so a headless run claims nothing. A
//!   tool-result continuation carries neither marker.
//!
//! **Redaction.** A logical record whose contributing segment(s) carried a non-empty
//! redaction audit and then fails to parse becomes `unknown` with `reason: "redacted"` and
//! the union of the touching audit spans' `kind`s — never a guess at the bytes a fail-closed
//! rule replaced, and never an attempt to parse a truncated prefix.
//!
//! **Source timestamps.** Every observation carries `fields.source_timestamp` — the record's
//! own top-level `timestamp`, copied verbatim, when it has one — separate from and never
//! replacing `observed` (the segment-derived interval). Absent when the record carries none
//! (for example Claude Code's `cost-state`).
//!
//! **Versions.** Codex CLI 0.153.0 exactly matches the connector's `tested_version_range`
//! and is verified against real `exec`-launch chains. Claude Code's structure was measured
//! on 2.1.259 fixtures and on 2.1.283, which carries no `origin` field at all and a
//! different `system` error shape (`subtype: "api_error"`, no `turn_duration`/text-prefixed
//! hook-block form) — both read from real bytes rather than assumed. `decoder_version` for
//! Claude names the checked range, `"2.1.259-2.1.283"`, not a false claim of a single
//! measured version.
//!
//! A disclosed, private, per-leaf segment assembler: nested in this file, not shared
//! with the other decoders.

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
use goalcup_protocol::redaction::RedactionAudit;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

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

/// Which native format one assembler decodes. Both formats share every bounded byte-assembly
/// rule; only the JSON interpretation differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Codex,
    ClaudeCode,
}

impl Format {
    /// The exact `protocol_hint` supplied for this format and the value
    /// recorded in every observation's `fields.format`.
    const fn hint(self) -> &'static str {
        match self {
            Format::Codex => "codex-jsonl",
            Format::ClaudeCode => "claude-code-jsonl",
        }
    }

    /// Whether this format reads a stream: native source and exactly its hint. The one
    /// selection rule; the decoders' `accepts` and the assembler's own guard call it.
    fn selects(self, source: CaptureSource, hint: Option<&str>) -> bool {
        source == CaptureSource::Native && hint == Some(self.hint())
    }

    const fn decoder_id(self) -> &'static str {
        match self {
            Format::Codex => "codex-native",
            Format::ClaudeCode => "claude-code-native",
        }
    }

    /// The harness version(s) this format's structure was measured/checked against — see the
    /// module doc's "Versions" section.
    const fn decoder_version(self) -> &'static str {
        match self {
            Format::Codex => "0.153.0",
            Format::ClaudeCode => "2.1.259-2.1.283",
        }
    }
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
    /// Set once, from the first contributing segment's `first` — never overwritten.
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
    /// that opened, closed and never carried a byte — valid empty input).
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

/// Per-stream Codex facts that persist for the whole life of the stream, unlike
/// [`LineState`] which resets every logical record. Bounded: two small fields per stream.
#[derive(Default)]
struct GoalState {
    /// The session's own declared `session_meta.payload.source`, once that record has been
    /// seen on this stream. `None` until then (or if it never arrives).
    codex_source: Option<String>,
    /// True once this `exec` stream's one delivered-goal record has been labelled as such.
    goal_excluded: bool,
}

/// The bounded, format-agnostic byte assembler and JSON interpreter. Each public decoder in
/// this file is a thin wrapper naming its [`Format`].
struct NativeAssembler {
    format: Format,
    streams: BTreeMap<StreamKey, (LineState, GoalState)>,
    /// Sum of `pending.len()` across every entry of `streams`, kept in step with every
    /// mutation so the total budget check never re-walks the map.
    total_buffered: usize,
    observations: u64,
    unknown_records: u64,
    incomplete: bool,
    /// The logical record currently being interpreted's own top-level `timestamp`, if it has
    /// one. Set immediately before `interpret` runs on a successfully parsed record, cleared
    /// before every other `unknown` emission (oversize/malformed/redacted/interrupted/
    /// incomplete never had a successful parse to read one from).
    current_timestamp: Option<String>,
}

impl NativeAssembler {
    fn new(format: Format) -> Self {
        Self {
            format,
            streams: BTreeMap::new(),
            total_buffered: 0,
            observations: 0,
            unknown_records: 0,
            incomplete: false,
            current_timestamp: None,
        }
    }

    fn push(&mut self, record: &CaptureRecord, emit: &mut dyn FnMut(Observation)) {
        match record {
            CaptureRecord::Open {
                stream,
                source,
                metadata,
                ..
            } => {
                let selected = self
                    .format
                    .selects(*source, metadata.protocol_hint.as_deref());
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
                if let Some((line, _)) = self.streams.get_mut(stream) {
                    self.incomplete = true;
                    let taken = std::mem::take(line);
                    self.total_buffered -= taken.pending.len();
                    if taken.touched() {
                        self.current_timestamp = None;
                        let source = taken.source_ref(stream);
                        let observed = taken.observed_range();
                        self.emit_unknown(&source, &observed, "interrupted", None, emit);
                    }
                }
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

    /// A stream closed (`End`, or a producer lost with no `End`). `reason == Complete` from a
    /// real `End` is the only clean case; anything else — including reaching here with no
    /// `End` at all — means this stream's coverage is not proven complete, whether or not it
    /// ever carried a byte (an interrupted *empty* stream is still `incomplete`, distinct
    /// from a genuinely complete empty one).
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
        self.current_timestamp = None;
        let source = line.source_ref(stream);
        let observed = line.observed_range();
        let word = match reason {
            // A clean End with a dangling, never-newline-terminated tail: an honest partial,
            // not a discarded successful record.
            EndReason::Complete => "incomplete",
            EndReason::Interrupted | EndReason::DecodeError => "interrupted",
        };
        self.emit_unknown(&source, &observed, word, None, emit);
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
            // Cannot recover any bytes from this segment. Treat like a gap on this stream:
            // whatever was mid-flight is invalidated, never silently concatenated past it.
            if let Some((line, _)) = self.streams.get_mut(stream) {
                self.incomplete = true;
                let taken = std::mem::take(line);
                self.total_buffered -= taken.pending.len();
                if taken.touched() {
                    self.current_timestamp = None;
                    let source = taken.source_ref(stream);
                    let observed = taken.observed_range();
                    self.emit_unknown(&source, &observed, "interrupted", None, emit);
                }
            }
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
        spans: &[goalcup_protocol::redaction::RedactionAuditSpan],
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
        let taken = std::mem::take(line);
        self.total_buffered -= taken.pending.len();
        self.finish_line(stream, taken, emit);
    }

    fn finish_line(
        &mut self,
        stream: &StreamKey,
        line: LineState,
        emit: &mut dyn FnMut(Observation),
    ) {
        if !line.touched() {
            return; // a blank line: not a record, not an error.
        }
        let source = line.source_ref(stream);
        let observed = line.observed_range();
        self.current_timestamp = None;
        if line.over_budget {
            self.emit_unknown(&source, &observed, "oversize", None, emit);
            return;
        }
        match serde_json::from_slice::<Value>(&line.pending) {
            Ok(value) => {
                // A source-supplied timestamp, kept separately labelled in every emitted
                // observation's fields and never replacing `observed`.
                self.current_timestamp = value
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .filter(|s| s.len() <= MAX_SOURCE_TIMESTAMP_BYTES)
                    .map(String::from);
                // This stream's persistent goal-tracking state is taken out for the duration
                // of the interpret pass (an `&mut self` borrow conflict otherwise) and put
                // back immediately after.
                let mut goal = self
                    .streams
                    .get_mut(stream)
                    .map(|(_, goal)| std::mem::take(goal))
                    .unwrap_or_default();
                self.interpret(&value, &source, &observed, &mut goal, emit);
                if let Some((_, slot)) = self.streams.get_mut(stream) {
                    *slot = goal;
                }
            }
            Err(_) => {
                if line.redacted {
                    let kinds: Vec<Value> = line
                        .redacted_kinds
                        .iter()
                        .map(|k| Value::String(k.clone()))
                        .collect();
                    self.emit_unknown(
                        &source,
                        &observed,
                        "redacted",
                        Some(("kinds", Value::Array(kinds))),
                        emit,
                    );
                } else {
                    self.emit_unknown(&source, &observed, "malformed", None, emit);
                }
            }
        }
    }

    fn interpret(
        &mut self,
        v: &Value,
        source: &SourceRef,
        observed: &ObservationTimeRange,
        goal: &mut GoalState,
        emit: &mut dyn FnMut(Observation),
    ) {
        match self.format {
            Format::Codex => self.interpret_codex(v, source, observed, goal, emit),
            Format::ClaudeCode => self.interpret_claude(v, source, observed, emit),
        }
    }

    // ----------------------------------------------------------------- Codex interpretation
    //
    // Facts measured on `spikes/codex-cli/fixtures/rollout-main-01a06614.jsonl` (Codex CLI
    // 0.153.0, TUI launch) and real `exec`-launch chains. See the module doc for the
    // human-marker/goal-exclusion rule.

    fn interpret_codex(
        &mut self,
        v: &Value,
        source: &SourceRef,
        observed: &ObservationTimeRange,
        goal: &mut GoalState,
        emit: &mut dyn FnMut(Observation),
    ) {
        let record_type = v.get("type").and_then(Value::as_str);
        // Bookkeeping only (no observation shape changes): remembers whether this stream's
        // session was launched as `exec` (its argv goal is not human input) or something
        // else. `session_meta` is still counted as `unknown{unrecognized}` below, unchanged.
        if record_type == Some("session_meta") {
            goal.codex_source = dotted(v, "payload.source")
                .and_then(Value::as_str)
                .map(String::from);
        }
        match record_type {
            Some("turn_context") => {
                if let Some(model) = dotted(v, "payload.model").and_then(Value::as_str) {
                    let turn_id = dotted(v, "payload.turn_id")
                        .and_then(Value::as_str)
                        .map(String::from);
                    self.emit_kind(
                        kinds::MODEL,
                        turn_id,
                        json!({"via": "turn_context", "model": model}),
                        source,
                        observed,
                        emit,
                    );
                } else {
                    self.emit_unrecognized(source, observed, Some("turn_context"), emit);
                }
            }
            Some("response_item") => {
                self.interpret_codex_response_item(v, source, observed, goal, emit)
            }
            Some("event_msg") => self.interpret_codex_event_msg(v, source, observed, emit),
            other => self.emit_unrecognized(source, observed, other, emit),
        }
    }

    fn interpret_codex_response_item(
        &mut self,
        v: &Value,
        source: &SourceRef,
        observed: &ObservationTimeRange,
        goal: &mut GoalState,
        emit: &mut dyn FnMut(Observation),
    ) {
        let Some(payload) = v.get("payload") else {
            self.emit_unrecognized(source, observed, Some("response_item"), emit);
            return;
        };
        let Some(item_type) = payload.get("type").and_then(Value::as_str) else {
            self.emit_unrecognized(source, observed, Some("response_item"), emit);
            return;
        };
        match item_type {
            "message" => {
                let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
                // The demonstrated positive marker (module doc): not role alone, not a text
                // prefix. `content_item_kinds` is the record's own account of what kind of
                // content this is; only "user.text" is real human-typed input.
                let has_marker = dotted(
                    payload,
                    "internal_chat_message_metadata_passthrough.content_item_kinds",
                )
                .and_then(Value::as_array)
                .is_some_and(|kinds| kinds.iter().any(|k| k.as_str() == Some("user.text")));
                if role != "user" || !has_marker {
                    self.emit_unrecognized(source, observed, Some("response_item.message"), emit);
                    return;
                }
                // The marker counts only in a session launched with an input channel. An
                // `exec` stream's first marked message is the tracker's delivered goal; no
                // marked message on an `exec` or unknown-launch stream is claimed human.
                match goal.codex_source.as_deref() {
                    Some("cli" | "vscode") => {}
                    Some("exec") if !goal.goal_excluded => {
                        goal.goal_excluded = true;
                        self.emit_unrecognized(
                            source,
                            observed,
                            Some("response_item.message.goal"),
                            emit,
                        );
                        return;
                    }
                    _ => {
                        self.emit_unrecognized(
                            source,
                            observed,
                            Some("response_item.message"),
                            emit,
                        );
                        return;
                    }
                }
                let text = payload
                    .get("content")
                    .and_then(Value::as_array)
                    .map(|blocks| {
                        blocks
                            .iter()
                            .filter_map(|b| b.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("")
                    })
                    .unwrap_or_default();
                let id = payload.get("id").and_then(Value::as_str).map(String::from);
                self.emit_kind(
                    kinds::HUMAN_MESSAGE,
                    id,
                    json!({"role": "user", "text": text}),
                    source,
                    observed,
                    emit,
                );
            }
            "custom_tool_call" | "function_call" => {
                let call_id = payload
                    .get("call_id")
                    .and_then(Value::as_str)
                    .map(String::from);
                let mut fields = json!({"item_type": item_type});
                if let Some(name) = payload.get("name").and_then(Value::as_str) {
                    fields["name"] = Value::String(name.to_string());
                }
                if let Some(status) = payload.get("status").and_then(Value::as_str) {
                    fields["status"] = Value::String(status.to_string());
                }
                self.emit_kind(kinds::TOOL_CALL, call_id, fields, source, observed, emit);
            }
            "custom_tool_call_output" | "function_call_output" => {
                let call_id = payload
                    .get("call_id")
                    .and_then(Value::as_str)
                    .map(String::from);
                self.emit_kind(
                    kinds::TOOL_RESULT,
                    call_id,
                    json!({"item_type": item_type}),
                    source,
                    observed,
                    emit,
                );
            }
            other => self.emit_unrecognized(
                source,
                observed,
                Some(&format!("response_item.{other}")),
                emit,
            ),
        }
    }

    fn interpret_codex_event_msg(
        &mut self,
        v: &Value,
        source: &SourceRef,
        observed: &ObservationTimeRange,
        emit: &mut dyn FnMut(Observation),
    ) {
        let Some(payload) = v.get("payload") else {
            self.emit_unrecognized(source, observed, Some("event_msg"), emit);
            return;
        };
        match payload.get("type").and_then(Value::as_str) {
            Some("token_count") => {
                let info = payload.get("info");
                let mut any = false;
                if let Some(last) = info.and_then(|i| i.get("last_token_usage")) {
                    let mut fields = json!({"scope": "per_call", "last_token_usage": last});
                    if let Some(window) = info.and_then(|i| i.get("model_context_window")) {
                        fields["model_context_window"] = window.clone();
                    }
                    self.emit_kind(kinds::USAGE, None, fields, source, observed, emit);
                    any = true;
                }
                // The running total is preserved as its own, separately scoped observation —
                // never summed with the per-call deltas above into a total this decoder computed.
                if let Some(total) = info.and_then(|i| i.get("total_token_usage")) {
                    self.emit_kind(
                        kinds::USAGE,
                        None,
                        json!({"scope": "cumulative", "total_token_usage": total}),
                        source,
                        observed,
                        emit,
                    );
                    any = true;
                }
                if !any {
                    self.emit_unrecognized(source, observed, Some("event_msg.token_count"), emit);
                }
            }
            Some("thread_settings_applied") => {
                if let Some(model) =
                    dotted(payload, "thread_settings.model").and_then(Value::as_str)
                {
                    self.emit_kind(
                        kinds::MODEL,
                        None,
                        json!({"via": "switch", "model": model}),
                        source,
                        observed,
                        emit,
                    );
                } else {
                    self.emit_unrecognized(
                        source,
                        observed,
                        Some("event_msg.thread_settings_applied"),
                        emit,
                    );
                }
            }
            Some("task_complete") => {
                // A turn's own observed completion, never a claim that the wider task
                // succeeded.
                let turn_id = payload
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .map(String::from);
                let mut fields = json!({"scope": "turn"});
                for key in [
                    "duration_ms",
                    "time_to_first_token_ms",
                    "started_at",
                    "completed_at",
                ] {
                    if let Some(value) = payload.get(key) {
                        fields[key] = value.clone();
                    }
                }
                self.emit_kind(kinds::OUTCOME, turn_id, fields, source, observed, emit);
            }
            other => self.emit_unrecognized(
                source,
                observed,
                other.map(|t| format!("event_msg.{t}")).as_deref(),
                emit,
            ),
        }
    }

    // ----------------------------------------------------------- Claude Code interpretation
    //
    // Facts measured on `spikes/claude-code-cli/fixtures/main-session.jsonl` (Claude Code
    // CLI 2.1.259) and real 2.1.283 chains. See the module doc for the human-marker rule.

    fn interpret_claude(
        &mut self,
        v: &Value,
        source: &SourceRef,
        observed: &ObservationTimeRange,
        emit: &mut dyn FnMut(Observation),
    ) {
        match v.get("type").and_then(Value::as_str) {
            Some("user") => self.interpret_claude_user(v, source, observed, emit),
            Some("assistant") => self.interpret_claude_assistant(v, source, observed, emit),
            Some("system") => self.interpret_claude_system(v, source, observed, emit),
            Some("attachment") => self.interpret_claude_attachment(v, source, observed, emit),
            other => self.emit_unrecognized(source, observed, other, emit),
        }
    }

    fn interpret_claude_user(
        &mut self,
        v: &Value,
        source: &SourceRef,
        observed: &ObservationTimeRange,
        emit: &mut dyn FnMut(Observation),
    ) {
        // The positive markers: a supplied `origin.kind == "human"` (2.1.259) or
        // `promptSource == "typed"`. The sdk-delivered goal carries `promptSource: "sdk"` and
        // a tool-result continuation carries neither, so neither is claimed.
        let origin_kind = dotted(v, "origin.kind").and_then(Value::as_str);
        let prompt_source = v.get("promptSource").and_then(Value::as_str);
        let has_marker = origin_kind == Some("human") || prompt_source == Some("typed");
        let prompt_id = v.get("promptId").and_then(Value::as_str).map(String::from);
        let content = dotted(v, "message.content");

        // Harness-generated records are never human, whatever marker they carry: a
        // background sub-agent task notification carries `promptSource: "system"` and
        // `origin.kind: "task-notification"` (measured), a replayed continuation
        // `isMeta: true` (measured), a sub-agent turn `isSidechain: true`, and a compaction
        // summary `isCompactSummary: true`. A present `origin.kind` must be exactly `"human"`.
        let flag = |key: &str| v.get(key).and_then(Value::as_bool) == Some(true);
        let structurally_eligible = !flag("isMeta")
            && !flag("isSidechain")
            && !flag("isCompactSummary")
            && prompt_source != Some("system")
            && origin_kind.is_none_or(|kind| kind == "human");
        let is_human = structurally_eligible && has_marker;
        let mut any = false;
        match content {
            Some(Value::String(text)) if is_human && !text.is_empty() => {
                self.emit_kind(
                    kinds::HUMAN_MESSAGE,
                    prompt_id,
                    json!({"text": text}),
                    source,
                    observed,
                    emit,
                );
                any = true;
            }
            Some(Value::Array(blocks)) => {
                if is_human {
                    let text: String = blocks
                        .iter()
                        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                        .filter_map(|b| b.get("text").and_then(Value::as_str))
                        .collect();
                    if !text.is_empty() {
                        self.emit_kind(
                            kinds::HUMAN_MESSAGE,
                            prompt_id.clone(),
                            json!({"text": text}),
                            source,
                            observed,
                            emit,
                        );
                        any = true;
                    }
                }
                for block in blocks {
                    if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                        let id = block
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .map(String::from);
                        let mut fields = json!({});
                        if let Some(is_error) = block.get("is_error") {
                            fields["is_error"] = is_error.clone();
                        }
                        self.emit_kind(kinds::TOOL_RESULT, id, fields, source, observed, emit);
                        any = true;
                    }
                }
            }
            _ => {}
        }
        if !any {
            self.emit_unrecognized(source, observed, Some("user"), emit);
        }
    }

    fn interpret_claude_assistant(
        &mut self,
        v: &Value,
        source: &SourceRef,
        observed: &ObservationTimeRange,
        emit: &mut dyn FnMut(Observation),
    ) {
        let message_id = dotted(v, "message.id")
            .and_then(Value::as_str)
            .map(String::from);
        let mut any = false;
        if let Some(model) = dotted(v, "message.model").and_then(Value::as_str) {
            self.emit_kind(
                kinds::MODEL,
                message_id.clone(),
                json!({"via": "message", "model": model}),
                source,
                observed,
                emit,
            );
            any = true;
        }
        if let Some(usage) = dotted(v, "message.usage") {
            let input = usage
                .get("input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let output = usage
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            // A `<synthetic>` resume record carries an all-zero usage block that made no API
            // call (measured): recording it would assert a spend that never
            // happened, so only a real non-zero snapshot becomes an observation.
            if input > 0 || output > 0 {
                let mut fields =
                    json!({"scope": "per_call", "input_tokens": input, "output_tokens": output});
                if let Some(c) = usage.get("cache_read_input_tokens") {
                    fields["cache_read_input_tokens"] = c.clone();
                }
                if let Some(c) = usage.get("cache_creation_input_tokens") {
                    fields["cache_creation_input_tokens"] = c.clone();
                }
                self.emit_kind(
                    kinds::USAGE,
                    message_id.clone(),
                    fields,
                    source,
                    observed,
                    emit,
                );
                any = true;
            }
        }
        if let Some(Value::Array(blocks)) = dotted(v, "message.content") {
            for block in blocks {
                if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                    let id = block.get("id").and_then(Value::as_str).map(String::from);
                    let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                    self.emit_kind(
                        kinds::TOOL_CALL,
                        id,
                        json!({"name": name}),
                        source,
                        observed,
                        emit,
                    );
                    any = true;
                }
            }
        }
        if !any {
            self.emit_unrecognized(source, observed, Some("assistant"), emit);
        }
    }

    fn interpret_claude_system(
        &mut self,
        v: &Value,
        source: &SourceRef,
        observed: &ObservationTimeRange,
        emit: &mut dyn FnMut(Observation),
    ) {
        match v.get("subtype").and_then(Value::as_str) {
            Some("informational") => {
                let content = v.get("content").and_then(Value::as_str).unwrap_or("");
                // A real, measured hook denial (2.1.259): the harness's own
                // account that a submission was blocked. Only a short, safe code is kept —
                // never the raw content, which carries the (possibly long, possibly
                // sensitive) prompt text.
                if content.starts_with("UserPromptSubmit operation blocked by hook:") {
                    self.emit_kind(
                        kinds::ERROR,
                        None,
                        json!({"subtype": "informational", "code": "prompt_blocked_by_hook"}),
                        source,
                        observed,
                        emit,
                    );
                } else {
                    self.emit_unrecognized(source, observed, Some("system.informational"), emit);
                }
            }
            Some("turn_duration") => {
                // A turn's own observed duration, never a claim that the wider task
                // succeeded.
                let mut fields = json!({"subtype": "turn_duration"});
                if let Some(d) = v.get("durationMs") {
                    fields["duration_ms"] = d.clone();
                }
                if let Some(m) = v.get("messageCount") {
                    fields["message_count"] = m.clone();
                }
                self.emit_kind(kinds::OUTCOME, None, fields, source, observed, emit);
            }
            Some("api_error") => {
                // 2.1.283's real error form — headless has no `turn_duration`/text-prefixed
                // hook-block system record, only this. Only the bounded, safe supplied facts
                // are kept: never the raw `error.message`/ `error.formatted` text, which can
                // carry arbitrary provider/network detail.
                let mut fields = json!({"subtype": "api_error"});
                if let Some(level) = v.get("level").and_then(Value::as_str) {
                    fields["level"] = Value::String(level.to_string());
                }
                if let Some(code) = dotted(v, "error.connection.code").and_then(Value::as_str) {
                    fields["code"] = Value::String(code.to_string());
                }
                if let Some(attempt) = v.get("retryAttempt") {
                    fields["retry_attempt"] = attempt.clone();
                }
                if let Some(max) = v.get("maxRetries") {
                    fields["max_retries"] = max.clone();
                }
                self.emit_kind(kinds::ERROR, None, fields, source, observed, emit);
            }
            other => self.emit_unrecognized(
                source,
                observed,
                other.map(|s| format!("system.{s}")).as_deref(),
                emit,
            ),
        }
    }

    fn interpret_claude_attachment(
        &mut self,
        v: &Value,
        source: &SourceRef,
        observed: &ObservationTimeRange,
        emit: &mut dyn FnMut(Observation),
    ) {
        if dotted(v, "attachment.type").and_then(Value::as_str) == Some("model")
            && let Some(id) = dotted(v, "attachment.identity.modelId").and_then(Value::as_str)
        {
            self.emit_kind(
                kinds::MODEL,
                None,
                json!({"via": "switch", "model": id}),
                source,
                observed,
                emit,
            );
            return;
        }
        self.emit_unrecognized(source, observed, Some("attachment"), emit);
    }

    // --------------------------------------------------------------------------- emit/count

    fn emit_kind(
        &mut self,
        kind: &str,
        supplied_id: Option<String>,
        mut fields: Value,
        source: &SourceRef,
        observed: &ObservationTimeRange,
        emit: &mut dyn FnMut(Observation),
    ) {
        if let Value::Object(map) = &mut fields {
            map.insert(
                "format".to_string(),
                Value::String(self.format.hint().to_string()),
            );
            if let Some(ts) = &self.current_timestamp {
                map.insert("source_timestamp".to_string(), Value::String(ts.clone()));
            }
        }
        emit(Observation {
            source: source.clone(),
            observed: observed.clone(),
            kind: kind.to_string(),
            supplied_id,
            fields,
        });
        self.observations += 1;
    }

    /// One bounded `unknown` observation: `fields.reason` names why, `extra` carries at most
    /// one additional bounded fact (`kinds` for a redacted record).
    fn emit_unknown(
        &mut self,
        source: &SourceRef,
        observed: &ObservationTimeRange,
        reason: &str,
        extra: Option<(&str, Value)>,
        emit: &mut dyn FnMut(Observation),
    ) {
        let mut fields = json!({"reason": reason});
        if let Some((key, value)) = extra {
            fields[key] = value;
        }
        self.emit_kind(kinds::UNKNOWN, None, fields, source, observed, emit);
        self.unknown_records += 1;
    }

    /// A structurally parseable record whose shape this decoder does not extract semantics
    /// from (outside its demonstrated scope). Counted, source-referenced — never
    /// silently dropped, never guessed at.
    fn emit_unrecognized(
        &mut self,
        source: &SourceRef,
        observed: &ObservationTimeRange,
        record_type: Option<&str>,
        emit: &mut dyn FnMut(Observation),
    ) {
        let extra = record_type.map(|t| ("record_type", Value::String(t.to_string())));
        self.emit_unknown(source, observed, "unrecognized", extra, emit);
    }

    fn finish(&mut self, emit: &mut dyn FnMut(Observation)) -> DecoderSummary {
        let streams = std::mem::take(&mut self.streams);
        for (stream, (line, _)) in streams {
            self.total_buffered -= line.pending.len();
            // A stream still open when the input ends had no `End` at all: a temporary
            // live-file EOF is never an analysis completion claim, so this is
            // incomplete regardless of whether a partial line happened to be pending.
            self.incomplete = true;
            if line.touched() {
                self.current_timestamp = None;
                let source = line.source_ref(&stream);
                let observed = line.observed_range();
                self.emit_unknown(&source, &observed, "incomplete", None, emit);
            }
        }
        DecoderSummary::new(self.observations, self.unknown_records, self.incomplete)
    }
}

/// The bounded Codex rollout-JSONL decoder (`codex-jsonl`). Reads only streams
/// opened with [`CaptureSource::Native`] and that exact protocol hint.
pub struct CodexNativeDecoder(NativeAssembler);

impl CodexNativeDecoder {
    pub fn new() -> Self {
        Self(NativeAssembler::new(Format::Codex))
    }
}

impl Default for CodexNativeDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl DecoderV1 for CodexNativeDecoder {
    fn id(&self) -> &'static str {
        Format::Codex.decoder_id()
    }

    fn version(&self) -> &'static str {
        Format::Codex.decoder_version()
    }

    fn accepts(&self, open: &StreamOpen<'_>) -> bool {
        Format::Codex.selects(open.source, open.protocol_hint)
    }

    fn push(
        &mut self,
        record: &CaptureRecord,
        emit: &mut dyn FnMut(Observation),
    ) -> Result<(), DecoderFailure> {
        self.0.push(record, emit);
        Ok(())
    }

    fn finish(
        mut self: Box<Self>,
        emit: &mut dyn FnMut(Observation),
    ) -> Result<DecoderSummary, DecoderFailure> {
        Ok(self.0.finish(emit))
    }
}

/// The bounded Claude Code project-JSONL decoder (`claude-code-jsonl`). Reads
/// only streams opened with [`CaptureSource::Native`] and that exact protocol hint.
pub struct ClaudeNativeDecoder(NativeAssembler);

impl ClaudeNativeDecoder {
    pub fn new() -> Self {
        Self(NativeAssembler::new(Format::ClaudeCode))
    }
}

impl Default for ClaudeNativeDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl DecoderV1 for ClaudeNativeDecoder {
    fn id(&self) -> &'static str {
        Format::ClaudeCode.decoder_id()
    }

    fn version(&self) -> &'static str {
        Format::ClaudeCode.decoder_version()
    }

    fn accepts(&self, open: &StreamOpen<'_>) -> bool {
        Format::ClaudeCode.selects(open.source, open.protocol_hint)
    }

    fn push(
        &mut self,
        record: &CaptureRecord,
        emit: &mut dyn FnMut(Observation),
    ) -> Result<(), DecoderFailure> {
        self.0.push(record, emit);
        Ok(())
    }

    fn finish(
        mut self: Box<Self>,
        emit: &mut dyn FnMut(Observation),
    ) -> Result<DecoderSummary, DecoderFailure> {
        Ok(self.0.finish(emit))
    }
}
