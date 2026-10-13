//! Shared plumbing of the wire decoders: the segment assembler, the SSE framer, the
//! observation sink with its counters, and the field helpers that keep supplied values
//! bounded. Transport framing and safe-value rules only; a provider's parser lives in its
//! own decoder (`openai.rs`, `chat.rs`, `anthropic.rs`).

use super::api::{Observation, ObservationTimeRange, SourceRef, kinds};
use goalcup_protocol::capture::{CaptureSource, CaptureTime, EndReason, StreamKey, WireIdentity};
use goalcup_protocol::redaction::RedactionAuditSpan;
use serde_json::{Map, Value};
use std::collections::BTreeSet;

/// The most bytes one logical record (message, body or SSE event) may buffer.
pub const MAX_EVENT_BYTES: usize = 1024 * 1024;
/// The most bytes the decoder buffers over every open stream together.
pub const MAX_BUFFERED_BYTES: usize = 8 * 1024 * 1024;
/// The most bytes of one kept text value (tool input, tool output, message text).
pub const MAX_TEXT_BYTES: usize = 16 * 1024;

/// The most entries of a listed structure (input items, content parts).
pub(super) const MAX_LISTED: usize = 64;
/// The longest safe token (code, type, status, role, model, event and item type).
pub(super) const MAX_TOKEN_BYTES: usize = 128;
/// The longest supplied identifier kept.
pub(super) const MAX_ID_BYTES: usize = 256;
/// The most distinct names in a bounded name list (roles, audit kinds, unknown fields).
pub(super) const MAX_NAMES: usize = 16;

/// Why a selected wire stream stopped. All built-in wire decoders use this one table so a
/// faithfully recorded early end stays report detail under D45, while lost framing still
/// degrades decoder coverage.
#[derive(Clone, Copy)]
pub(super) enum Close {
    End(EndReason),
    ProducerLost,
    Unterminated,
}

#[derive(Clone, Copy)]
pub(super) struct CloseClass {
    pub(super) finding: Option<&'static str>,
    pub(super) loss: bool,
}

pub(super) fn classify_close(close: Close) -> CloseClass {
    match close {
        Close::End(EndReason::Complete) => CloseClass {
            finding: None,
            loss: false,
        },
        Close::End(EndReason::Interrupted) => CloseClass {
            finding: Some("interrupted"),
            loss: false,
        },
        Close::End(EndReason::DecodeError) => CloseClass {
            finding: Some("decode_error"),
            loss: false,
        },
        Close::ProducerLost => CloseClass {
            finding: Some("producer_lost"),
            loss: true,
        },
        Close::Unterminated => CloseClass {
            finding: Some("unterminated"),
            loss: true,
        },
    }
}

#[derive(Debug, Default)]
pub(super) struct Tally {
    pub(super) observations: u64,
    pub(super) unknown: u64,
    pub(super) incomplete: bool,
}

#[derive(Clone, Copy)]
pub(super) struct SegmentView<'a> {
    pub(super) part: u64,
    pub(super) first: &'a CaptureTime,
    pub(super) last: &'a CaptureTime,
    pub(super) audit: &'a [RedactionAuditSpan],
    pub(super) source_offset: u64,
    pub(super) source_len: u64,
}

impl<'a> SegmentView<'a> {
    pub(super) fn with_retained(self, retained: &[u8]) -> RetainedSegmentView<'a> {
        RetainedSegmentView {
            part: self.part,
            first: self.first,
            last: self.last,
            retained_len: retained.len(),
            audit: AuditProjection::new(&self, retained),
        }
    }
}

pub(super) struct RetainedSegmentView<'a> {
    part: u64,
    first: &'a CaptureTime,
    last: &'a CaptureTime,
    retained_len: usize,
    audit: AuditProjection,
}

impl RetainedSegmentView<'_> {
    pub(super) fn invalid_audit(&self) -> bool {
        self.audit.invalid
    }
}

#[derive(Debug)]
struct ProjectedAuditSpan {
    start: usize,
    end: usize,
    kind: String,
}

#[derive(Debug, Default)]
struct AuditProjection {
    spans: Vec<ProjectedAuditSpan>,
    invalid: bool,
    present: bool,
    conservative_kinds: BTreeSet<String>,
}

impl AuditProjection {
    fn new(view: &SegmentView<'_>, retained: &[u8]) -> Self {
        let mut projection = Self {
            present: !view.audit.is_empty(),
            ..Self::default()
        };
        for audit in view.audit {
            if is_token(&audit.kind) && projection.conservative_kinds.len() < MAX_NAMES {
                projection.conservative_kinds.insert(audit.kind.clone());
            }
        }
        let Some(source_end) = view.source_offset.checked_add(view.source_len) else {
            projection.invalid = true;
            return projection;
        };
        let mut source_cursor = view.source_offset;
        let mut retained_cursor = 0usize;
        for audit in view.audit {
            if audit.start < source_cursor
                || audit.start >= audit.end
                || audit.end > source_end
                || !is_token(&audit.kind)
            {
                projection.invalid = true;
                return projection;
            }
            let Ok(clean_len) = usize::try_from(audit.start - source_cursor) else {
                projection.invalid = true;
                return projection;
            };
            let Some(marker_start) = retained_cursor.checked_add(clean_len) else {
                projection.invalid = true;
                return projection;
            };
            let marker = format!("<redacted:{}>", audit.kind);
            let bare = marker.as_bytes();
            let quoted = format!("\"{marker}\"");
            let Some(bare_end) = marker_start.checked_add(bare.len()) else {
                projection.invalid = true;
                return projection;
            };
            let Some(quoted_end) = marker_start.checked_add(quoted.len()) else {
                projection.invalid = true;
                return projection;
            };
            let marker_end = if retained.get(marker_start..bare_end) == Some(bare) {
                bare_end
            } else if retained.get(marker_start..quoted_end) == Some(quoted.as_bytes()) {
                quoted_end
            } else {
                projection.invalid = true;
                return projection;
            };
            projection.spans.push(ProjectedAuditSpan {
                start: marker_start,
                end: marker_end,
                kind: audit.kind.clone(),
            });
            retained_cursor = marker_end;
            source_cursor = audit.end;
        }
        let Ok(suffix_len) = usize::try_from(source_end - source_cursor) else {
            projection.invalid = true;
            return projection;
        };
        if retained_cursor.checked_add(suffix_len) != Some(retained.len()) {
            projection.invalid = true;
            projection.spans.clear();
        }
        projection
    }
}

#[derive(Default)]
pub(super) struct AuditCursor {
    next: usize,
    #[cfg(test)]
    inspections: usize,
}

/// The contributing segments of one logical record (private assembler).
#[derive(Clone, Debug, Default)]
pub(super) struct Span {
    pub(super) first: Option<(u64, CaptureTime)>,
    pub(super) last: Option<(u64, CaptureTime)>,
    /// Some contributing segment carried a redaction audit span.
    pub(super) redacted: bool,
    pub(super) kinds: BTreeSet<String>,
}

impl Span {
    /// Extends this span over a later one: the same record, seen in more segments.
    pub(super) fn merge(&mut self, later: &Span) {
        if self.first.is_none() {
            self.first.clone_from(&later.first);
        }
        if later.last.is_some() {
            self.last.clone_from(&later.last);
        }
        self.redacted |= later.redacted;
        for kind in &later.kinds {
            if self.kinds.len() < MAX_NAMES {
                self.kinds.insert(kind.clone());
            }
        }
    }

    pub(super) fn touch_unprojected(&mut self, view: &SegmentView<'_>) {
        self.touch_time(view.part, view.first, view.last);
        if !view.audit.is_empty() {
            self.redacted = true;
            for audit in view.audit {
                if self.kinds.len() < MAX_NAMES && is_token(&audit.kind) {
                    self.kinds.insert(audit.kind.clone());
                }
            }
        }
    }

    pub(super) fn touch(&mut self, view: &RetainedSegmentView<'_>) {
        let mut cursor = AuditCursor::default();
        self.touch_range(view, &mut cursor, 0, view.retained_len);
    }

    /// Adds the retained byte range of one segment to this logical record. Redaction audit
    /// spans use source offsets, so only the event whose retained range can intersect a
    /// replacement is labelled redacted.
    pub(super) fn touch_range(
        &mut self,
        view: &RetainedSegmentView<'_>,
        cursor: &mut AuditCursor,
        start: usize,
        end: usize,
    ) {
        self.touch_time(view.part, view.first, view.last);
        if view.audit.invalid {
            self.redacted |= view.audit.present;
            for kind in &view.audit.conservative_kinds {
                if self.kinds.len() < MAX_NAMES {
                    self.kinds.insert(kind.clone());
                }
            }
            return;
        }
        if start >= end {
            return;
        }
        while let Some(audit) = view.audit.spans.get(cursor.next) {
            #[cfg(test)]
            {
                cursor.inspections += 1;
            }
            if audit.end <= start {
                cursor.next += 1;
                continue;
            }
            if audit.start >= end {
                break;
            }
            self.redacted = true;
            if self.kinds.len() < MAX_NAMES {
                self.kinds.insert(audit.kind.clone());
            }
            if audit.end <= end {
                cursor.next += 1;
                continue;
            }
            break;
        }
    }

    fn touch_time(&mut self, part: u64, first: &CaptureTime, last: &CaptureTime) {
        if self.first.is_none() {
            self.first = Some((part, first.clone()));
        }
        if self.last.as_ref().map(|(seen, _)| *seen) != Some(part) {
            self.last = Some((part, last.clone()));
        }
    }
}

/// What the SSE framer hands over at a blank line.
pub(super) enum SseItem<'a> {
    /// A complete event: its `data` field (several `data` lines joined by `\n`).
    Data {
        name: Option<&'a [u8]>,
        data: &'a [u8],
    },
    /// The event was damaged by a gap; nothing of it is kept.
    Gap,
    /// The event outgrew [`MAX_EVENT_BYTES`] or the shared budget; nothing of it is kept.
    OverBudget,
}

/// Why the shared SSE framer is discarding through its next blank-line boundary.
///
/// An invalid audit already emitted its fixed diagnostic, so its synthetic boundary is silent.
/// Any independent decoder damage remains observable and takes precedence if both happen before
/// that boundary is consumed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ResyncCause {
    causes: [Option<PendingCause>; 3],
    invalid_audit: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingCause {
    OverBudget,
    Gap,
    UndecodableSegment,
}

impl PendingCause {
    fn reason(self) -> &'static str {
        match self {
            Self::OverBudget => "over_budget",
            Self::Gap => "gap",
            Self::UndecodableSegment => "undecodable_segment",
        }
    }
}

impl ResyncCause {
    pub(super) fn remember(slot: &mut Option<Self>, reason: &'static str) {
        let cause = slot.get_or_insert_default();
        let next = match reason {
            "over_budget" => PendingCause::OverBudget,
            "gap" => PendingCause::Gap,
            "undecodable_segment" => PendingCause::UndecodableSegment,
            "invalid_redaction_audit" => {
                cause.invalid_audit = true;
                return;
            }
            _ => {
                debug_assert!(false, "unknown resync cause");
                return;
            }
        };
        if cause.causes.contains(&Some(next)) {
            return;
        }
        if let Some(slot) = cause.causes.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(next);
        }
    }

    /// Findings in causal order. Invalid-audit damage has already emitted its fixed diagnostic;
    /// it suppresses the synthetic resync marker but never an earlier budget loss.
    pub(super) fn findings(self) -> [Option<&'static str>; 3] {
        let mut findings = self.causes.map(|cause| cause.map(PendingCause::reason));
        if findings.iter().all(Option::is_none) && !self.invalid_audit {
            findings[0] = Some("gap");
        }
        findings
    }
}

/// A bounded set of independent causes that made one whole-message record undecodable.
///
/// Causes retain first-seen order, repeated causes deduplicate, and the terminal stream fact is
/// deliberately stored elsewhere. The closed slots keep adversarial input at constant space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct LossCauses {
    causes: [Option<LossCause>; 6],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LossCause {
    OverBudget,
    Gap,
    Discontinuity,
    UndecodableSegment,
    MalformedSegment,
    InvalidRedactionAudit,
}

impl LossCause {
    fn from_reason(reason: &'static str) -> Option<Self> {
        match reason {
            "over_budget" => Some(Self::OverBudget),
            "gap" => Some(Self::Gap),
            "discontinuity" => Some(Self::Discontinuity),
            "undecodable_segment" => Some(Self::UndecodableSegment),
            "malformed_segment" => Some(Self::MalformedSegment),
            "invalid_redaction_audit" => Some(Self::InvalidRedactionAudit),
            _ => None,
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::OverBudget => "over_budget",
            Self::Gap => "gap",
            Self::Discontinuity => "discontinuity",
            Self::UndecodableSegment => "undecodable_segment",
            Self::MalformedSegment => "malformed_segment",
            Self::InvalidRedactionAudit => "invalid_redaction_audit",
        }
    }
}

impl LossCauses {
    pub(super) fn remember(&mut self, reason: &'static str) {
        let Some(next) = LossCause::from_reason(reason) else {
            debug_assert!(false, "unknown whole-record loss cause");
            return;
        };
        if self.causes.contains(&Some(next)) {
            return;
        }
        if let Some(slot) = self.causes.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(next);
        }
    }

    pub(super) fn is_empty(self) -> bool {
        self.causes[0].is_none()
    }

    pub(super) fn contains(self, reason: &'static str) -> bool {
        LossCause::from_reason(reason).is_some_and(|cause| self.causes.contains(&Some(cause)))
    }

    pub(super) fn findings(self) -> [Option<&'static str>; 6] {
        self.causes.map(|cause| cause.map(LossCause::reason))
    }
}

/// The SSE framer's state for one body. One framer for every wire decoder: it only frames
/// lines into events; what an event means is the caller's.
#[derive(Debug, Default)]
pub(super) struct Sse {
    /// The current event's lines, each ended by `\n`.
    pub(super) event: Vec<u8>,
    pub(super) line_len: usize,
    pub(super) pending_cr: bool,
    pub(super) cr_dispatched: bool,
    /// The current line began after a gap, so its start is unknown.
    pub(super) fragment: bool,
    /// The current event was damaged by a gap: discard through the next blank line.
    pub(super) resync: bool,
    pub(super) over: bool,
}

impl Sse {
    /// Whether the undispatched event so far holds a `data` field with a value: a comment, a
    /// bare `event:` or an empty `data:` line is not data.
    pub(super) fn pending_has_data(&self) -> bool {
        self.event.split(|byte| *byte == b'\n').any(|line| {
            let Some(at) = line.iter().position(|byte| *byte == b':') else {
                return false;
            };
            let value = &line[at + 1..];
            &line[..at] == b"data" && !value.strip_prefix(b" ").unwrap_or(value).is_empty()
        })
    }

    /// At the end of the body: releases what the framer still buffers and says why the bytes
    /// of a last, undispatched event (if any arrived) were never decoded. `cut` is how the
    /// stream ended, when it did not end complete.
    pub(super) fn finish(
        self,
        buffered: &mut usize,
        span: &Span,
        cut: Option<&'static str>,
    ) -> Option<&'static str> {
        *buffered -= self.event.len();
        if self.resync {
            Some("gap")
        } else if span.first.is_some() {
            Some(if self.over {
                "over_budget"
            } else {
                cut.unwrap_or("unterminated_event")
            })
        } else {
            None
        }
    }

    /// Starts discard-through-boundary resynchronization. Returns whether an event had already
    /// exceeded its budget before the new damage arrived, so callers can preserve that earlier
    /// causal finding instead of replacing it with a silent invalid-audit boundary.
    pub(super) fn lose(&mut self, buffered: &mut usize) -> bool {
        let pending_over_budget = std::mem::take(&mut self.over);
        *buffered -= self.event.len();
        self.event = Vec::new();
        self.resync = true;
        self.fragment = true;
        self.pending_cr = false;
        self.line_len = 0;
        pending_over_budget
    }

    pub(super) fn feed<C: SseContext>(
        &mut self,
        span: &mut Span,
        view: &RetainedSegmentView<'_>,
        data: &[u8],
        out: &mut C,
        on_item: &mut dyn FnMut(SseItem<'_>, &Span, &mut C),
    ) {
        let mut audit_cursor = AuditCursor::default();
        for (offset, &byte) in data.iter().enumerate() {
            if self.pending_cr {
                self.pending_cr = false;
                if byte == b'\n' {
                    // The second half of a CRLF: part of the line it ends, and of no event
                    // when that line was the blank one that already dispatched.
                    if !self.cr_dispatched {
                        span.touch_range(view, &mut audit_cursor, offset, offset + 1);
                    }
                    continue;
                }
            }
            span.touch_range(view, &mut audit_cursor, offset, offset + 1);
            if byte == b'\r' || byte == b'\n' {
                self.pending_cr = byte == b'\r';
                self.cr_dispatched = false;
                if self.line_len == 0 && !self.fragment {
                    self.cr_dispatched = true;
                    self.dispatch(span, out, on_item);
                } else {
                    self.fragment = false;
                    self.line_len = 0;
                    self.push(b'\n', out);
                }
            } else {
                self.line_len += 1;
                self.push(byte, out);
            }
        }
    }

    fn push<C: SseContext>(&mut self, byte: u8, out: &mut C) {
        if self.resync || self.over {
            return;
        }
        if self.event.len() >= MAX_EVENT_BYTES || !out.take(1) {
            out.give(self.event.len());
            self.event = Vec::new();
            self.over = true;
            out.mark_incomplete();
            return;
        }
        self.event.push(byte);
    }

    fn dispatch<C: SseContext>(
        &mut self,
        span: &mut Span,
        out: &mut C,
        on_item: &mut dyn FnMut(SseItem<'_>, &Span, &mut C),
    ) {
        let span = std::mem::take(span);
        let event = std::mem::take(&mut self.event);
        let (resync, over) = (self.resync, self.over);
        self.resync = false;
        self.over = false;
        if resync {
            on_item(SseItem::Gap, &span, out);
            out.give(event.len());
            return;
        }
        if over {
            on_item(SseItem::OverBudget, &span, out);
            out.give(event.len());
            return;
        }
        let mut event_name = None;
        let mut lines = Vec::new();
        for line in event.split(|byte| *byte == b'\n') {
            if line.is_empty() || line[0] == b':' {
                continue;
            }
            let (field, value) = match line.iter().position(|byte| *byte == b':') {
                Some(at) => {
                    let value = &line[at + 1..];
                    (&line[..at], value.strip_prefix(b" ").unwrap_or(value))
                }
                None => (line, &line[line.len()..]),
            };
            if field == b"event" {
                event_name = Some(value);
            } else if field == b"data" {
                lines.push(value);
            }
        }
        // An event without data (a comment, a bare `event:`) is not dispatched (SSE).
        if lines.is_empty() {
            out.give(event.len());
            return;
        }
        if lines.len() == 1 {
            on_item(
                SseItem::Data {
                    name: event_name,
                    data: lines[0],
                },
                &span,
                out,
            );
            out.give(event.len());
            return;
        }
        let joined_len = lines.iter().map(|line| line.len()).sum::<usize>() + lines.len() - 1;
        if joined_len > MAX_EVENT_BYTES || !out.take(joined_len) {
            out.mark_incomplete();
            on_item(SseItem::OverBudget, &span, out);
            out.give(event.len());
            return;
        }
        let mut data = Vec::with_capacity(joined_len);
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                data.push(b'\n');
            }
            data.extend_from_slice(line);
        }
        on_item(
            SseItem::Data {
                name: event_name,
                data: &data,
            },
            &span,
            out,
        );
        out.give(joined_len);
        out.give(event.len());
    }
}

/// The shared SSE framer asks only for buffer accounting and loss marking. Each provider keeps
/// its own observation adapter and parser state.
pub(super) trait SseContext {
    fn take(&mut self, bytes: usize) -> bool;
    fn give(&mut self, bytes: usize);
    fn mark_incomplete(&mut self);
}

/// Appends one provider record within the shared event and decoder-wide buffer bounds.
pub(super) fn append_bounded<C: SseContext>(
    buffer: &mut Vec<u8>,
    bytes: &[u8],
    out: &mut C,
) -> bool {
    if buffer.len() + bytes.len() <= MAX_EVENT_BYTES && out.take(bytes.len()) {
        buffer.extend_from_slice(bytes);
        true
    } else {
        release_buffer(buffer, out);
        false
    }
}

pub(super) fn release_buffer<C: SseContext>(buffer: &mut Vec<u8>, out: &mut C) {
    out.give(buffer.len());
    *buffer = Vec::new();
}

/// Where a record came from: its stream and the recorder's metadata for it.
pub(super) struct Origin<'a> {
    pub(super) stream: &'a StreamKey,
    pub(super) source: CaptureSource,
    pub(super) wire: Option<&'a WireIdentity>,
    pub(super) status: Option<u16>,
}

/// The observation sink and the decoder-wide counters.
pub(super) struct Out<'a> {
    pub(super) tally: &'a mut Tally,
    pub(super) buffered: &'a mut usize,
    pub(super) emit: &'a mut dyn FnMut(Observation),
}

impl Out<'_> {
    /// Emit one observation over the record's contributing segments. A record without any
    /// segment has no source reference to give, so it is counted but never emitted.
    pub(super) fn observe(
        &mut self,
        origin: &Origin<'_>,
        span: &Span,
        kind: &str,
        supplied_id: Option<String>,
        mut fields: Map<String, Value>,
    ) {
        let (Some((first_part, first)), Some((last_part, last))) = (&span.first, &span.last) else {
            return;
        };
        fields.insert(
            "capture_source".into(),
            Value::String(source_name(origin.source).into()),
        );
        if let Some(wire) = origin.wire {
            let mut identity = Map::new();
            identity.insert("connection".into(), Value::String(wire.connection.clone()));
            identity.insert("exchange".into(), wire.exchange.into());
            if let Some(index) = wire.message_index {
                identity.insert("message_index".into(), index.into());
            }
            fields.insert("wire".into(), Value::Object(identity));
        }
        if span.redacted {
            fields.insert("redaction_kinds".into(), names(&span.kinds));
        }
        (self.emit)(Observation {
            source: SourceRef {
                stream: origin.stream.clone(),
                first_part: *first_part,
                last_part: *last_part,
            },
            observed: ObservationTimeRange {
                first: first.clone(),
                last: last.clone(),
            },
            kind: kind.into(),
            supplied_id,
            fields: Value::Object(fields),
        });
        self.tally.observations += 1;
    }

    pub(super) fn unknown(
        &mut self,
        origin: &Origin<'_>,
        span: &Span,
        reason: &str,
        mut extra: Map<String, Value>,
    ) {
        self.tally.unknown += 1;
        extra.insert("reason".into(), Value::String(reason.into()));
        if reason == "over_budget" {
            extra.insert("limit_bytes".into(), MAX_EVENT_BYTES.into());
        }
        self.observe(origin, span, kinds::UNKNOWN, None, extra);
    }
}

impl SseContext for Out<'_> {
    fn take(&mut self, bytes: usize) -> bool {
        match self.buffered.checked_add(bytes) {
            Some(total) if total <= MAX_BUFFERED_BYTES => {
                *self.buffered = total;
                true
            }
            _ => false,
        }
    }

    fn give(&mut self, bytes: usize) {
        debug_assert!(bytes <= *self.buffered, "buffer accounting underflow");
        *self.buffered = self.buffered.saturating_sub(bytes);
    }

    fn mark_incomplete(&mut self) {
        self.tally.incomplete = true;
    }
}

/// Parses one complete logical record into a JSON object. A parse failure is `redacted` when
/// a contributing segment carried an audit span, else `malformed`; a value that is not an
/// object is `unsupported_shape`. Each is one `unknown` observation, and `None`.
pub(super) fn parse_object(
    bytes: &[u8],
    span: &Span,
    origin: &Origin<'_>,
    out: &mut Out<'_>,
) -> Option<Map<String, Value>> {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(object)) => Some(object),
        Ok(_) => {
            out.unknown(origin, span, "unsupported_shape", Map::new());
            None
        }
        Err(_) if span.redacted => {
            let mut extra = Map::new();
            extra.insert("kinds".into(), names(&span.kinds));
            out.unknown(origin, span, "redacted", extra);
            None
        }
        Err(_) => {
            out.unknown(origin, span, "malformed", Map::new());
            None
        }
    }
}

pub(super) fn source_name(source: CaptureSource) -> &'static str {
    match source {
        CaptureSource::WireRequest => "wire_request",
        CaptureSource::WireResponse => "wire_response",
        CaptureSource::WsClient => "ws_client",
        CaptureSource::WsServer => "ws_server",
        CaptureSource::Native => "native",
        CaptureSource::Goal => "goal",
    }
}

/// Content parts as `{name}_parts: [{type, text}]` plus their total text bytes. The kept
/// text of all parts together is bounded by [`MAX_TEXT_BYTES`].
pub(super) fn put_parts(fields: &mut Map<String, Value>, name: &str, parts: &[Value]) {
    let mut room = MAX_TEXT_BYTES;
    let mut truncated = parts.len() > MAX_LISTED;
    let mut total = 0usize;
    let mut listed = Vec::new();
    for part in parts {
        let text = part.get("text").and_then(Value::as_str);
        total += text.map_or(0, str::len);
        if listed.len() >= MAX_LISTED {
            continue;
        }
        let mut entry = Map::new();
        put_token(&mut entry, "type", part.get("type"));
        if let Some(text) = text {
            let kept = cut(text, room);
            room -= kept.len();
            truncated |= kept.len() < text.len();
            entry.insert("text".into(), Value::String(kept.into()));
        }
        listed.push(Value::Object(entry));
    }
    fields.insert(format!("{name}_parts"), Value::Array(listed));
    fields.insert(format!("{name}_bytes"), total.into());
    if truncated {
        fields.insert(format!("{name}_truncated"), Value::Bool(true));
    }
}

pub(super) fn put_text(fields: &mut Map<String, Value>, name: &str, text: &str) {
    let kept = cut(text, MAX_TEXT_BYTES);
    fields.insert(name.into(), Value::String(kept.into()));
    fields.insert(format!("{name}_bytes"), text.len().into());
    if kept.len() < text.len() {
        fields.insert(format!("{name}_truncated"), Value::Bool(true));
    }
}

/// A supplied error `code` and `type`, each only as a safe token; a present value that is
/// not one is named in `withheld`. No message, parameter or other text is kept.
pub(super) fn put_error(
    fields: &mut Map<String, Value>,
    code: Option<&Value>,
    kind: Option<&Value>,
) {
    let mut withheld = Vec::new();
    for (name, value) in [("code", code), ("type", kind)] {
        match value {
            None | Some(Value::Null) => {}
            Some(value) => match token(Some(value)) {
                Some(value) => {
                    fields.insert(name.into(), Value::String(value));
                }
                None => withheld.push(Value::String(name.into())),
            },
        }
    }
    if !withheld.is_empty() {
        fields.insert("withheld".into(), Value::Array(withheld));
    }
}

pub(super) fn put_token(fields: &mut Map<String, Value>, name: &str, value: Option<&Value>) {
    if let Some(value) = token(value) {
        fields.insert(name.into(), Value::String(value));
    }
}

pub(super) fn put_id(fields: &mut Map<String, Value>, name: &str, value: Option<&Value>) {
    if let Some(value) = id(value) {
        fields.insert(name.into(), Value::String(value));
    }
}

/// A provider-supplied number (a timestamp), kept exactly as supplied.
pub(super) fn put_number(fields: &mut Map<String, Value>, name: &str, value: Option<&Value>) {
    if let Some(value) = value.filter(|value| value.is_number()) {
        fields.insert(name.into(), value.clone());
    }
}

/// A short enumeration-like value: code, type, status, role, model, event or item type.
pub(super) fn token(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    is_token(text).then(|| text.to_string())
}

pub(super) fn is_token(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_TOKEN_BYTES
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

/// A supplied identifier, kept verbatim when it is short and plainly an identifier.
pub(super) fn id(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    let plain = text
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte));
    (!text.is_empty() && text.len() <= MAX_ID_BYTES && plain).then(|| text.to_string())
}

pub(super) fn names(set: &BTreeSet<String>) -> Value {
    Value::Array(set.iter().cloned().map(Value::String).collect())
}

/// The longest prefix of `text` of at most `limit` bytes that ends on a character boundary.
pub(super) fn cut(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// A tool result's `output`: a string, or an array of content parts.
pub(super) fn put_output(fields: &mut Map<String, Value>, output: Option<&Value>) {
    match output {
        Some(Value::String(text)) => put_text(fields, "output", text),
        Some(Value::Array(parts)) => put_parts(fields, "output", parts),
        Some(_) => {
            fields.insert("output_kind".into(), Value::String("unsupported".into()));
        }
        None => {}
    }
}

/// The supplied integer token fields of one usage object: nothing summed, nothing defaulted.
/// `counts` names the integer members a provider documents, `details` its objects of integer
/// members; any other member is named in `unrecognized_fields`.
pub(super) fn put_usage(
    fields: &mut Map<String, Value>,
    usage: &Map<String, Value>,
    counts: &[&str],
    details: &[&str],
) {
    let mut unrecognized = BTreeSet::new();
    for (name, value) in usage {
        if counts.contains(&name.as_str()) {
            if let Some(count) = value.as_u64() {
                fields.insert(name.clone(), count.into());
            }
        } else if details.contains(&name.as_str()) {
            if let Some(details) = value.as_object() {
                let kept: Map<String, Value> = details
                    .iter()
                    .filter(|(key, value)| is_token(key) && value.is_u64())
                    .take(MAX_NAMES)
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                fields.insert(name.clone(), Value::Object(kept));
            }
        } else if unrecognized.len() < MAX_NAMES && is_token(name) {
            unrecognized.insert(name.clone());
        }
    }
    if !unrecognized.is_empty() {
        fields.insert("unrecognized_fields".into(), names(&unrecognized));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn time() -> CaptureTime {
        CaptureTime {
            wall: "2026-10-06T00:00:00Z".into(),
            mono_ns: 1,
            clock_id: "test".into(),
        }
    }

    fn projection(
        source_offset: u64,
        source_len: u64,
        audit: &[RedactionAuditSpan],
        retained: &[u8],
    ) -> AuditProjection {
        let at = time();
        AuditProjection::new(
            &SegmentView {
                part: 1,
                first: &at,
                last: &at,
                audit,
                source_offset,
                source_len,
            },
            retained,
        )
    }

    #[test]
    fn audit_projection_accepts_bare_and_quoted_markers_with_length_changes() {
        let bare = [RedactionAuditSpan {
            kind: "a".into(),
            start: 12,
            end: 17,
        }];
        let projected = projection(10, 9, &bare, b"AA<redacted:a>BB");
        assert!(!projected.invalid);
        assert_eq!((projected.spans[0].start, projected.spans[0].end), (2, 14));

        let quoted = [RedactionAuditSpan {
            kind: "b".into(),
            start: 11,
            end: 18,
        }];
        let projected = projection(10, 9, &quoted, b"A\"<redacted:b>\"B");
        assert!(!projected.invalid);
        assert_eq!((projected.spans[0].start, projected.spans[0].end), (1, 15));
    }

    #[test]
    fn audit_projection_rejects_order_overlap_range_zero_overflow_and_bad_marker() {
        let cases = [
            (
                0,
                5,
                vec![RedactionAuditSpan {
                    kind: "a".into(),
                    start: 3,
                    end: 3,
                }],
            ),
            (
                0,
                5,
                vec![RedactionAuditSpan {
                    kind: "a".into(),
                    start: 4,
                    end: 6,
                }],
            ),
            (u64::MAX, 1, Vec::new()),
            (
                0,
                5,
                vec![
                    RedactionAuditSpan {
                        kind: "a".into(),
                        start: 2,
                        end: 4,
                    },
                    RedactionAuditSpan {
                        kind: "b".into(),
                        start: 3,
                        end: 5,
                    },
                ],
            ),
        ];
        for (offset, len, spans) in cases {
            assert!(projection(offset, len, &spans, b"xx<redacted:a>x").invalid);
        }
        let bad = [RedactionAuditSpan {
            kind: "a".into(),
            start: 2,
            end: 4,
        }];
        assert!(projection(0, 5, &bad, b"xx<redacted:b>x").invalid);
    }

    #[test]
    fn audit_projection_rejects_marker_endpoint_overflow_without_panicking() {
        let near_max = [RedactionAuditSpan {
            kind: "email".into(),
            start: u64::MAX - 2,
            end: u64::MAX - 1,
        }];
        assert!(projection(0, u64::MAX, &near_max, b"retained").invalid);
    }

    #[test]
    fn whole_range_touch_attributes_every_intersecting_audit_span() {
        let audits = [
            RedactionAuditSpan {
                kind: "alpha".into(),
                start: 0,
                end: 1,
            },
            RedactionAuditSpan {
                kind: "beta".into(),
                start: 1,
                end: 2,
            },
        ];
        let retained = b"<redacted:alpha><redacted:beta>";
        let at = time();
        let raw = SegmentView {
            part: 1,
            first: &at,
            last: &at,
            audit: &audits,
            source_offset: 0,
            source_len: 2,
        };
        let view = raw.with_retained(retained);
        let mut cursor = AuditCursor::default();
        let mut span = Span::default();
        span.touch_range(&view, &mut cursor, 0, retained.len());
        assert!(span.redacted);
        assert_eq!(span.kinds, BTreeSet::from(["alpha".into(), "beta".into()]));
        assert_eq!(cursor.next, 2);
    }

    #[test]
    fn invalid_projection_is_monotone_redaction_even_without_a_safe_kind() {
        let valid = [RedactionAuditSpan {
            kind: "email".into(),
            start: 0,
            end: 1,
        }];
        let invalid = [RedactionAuditSpan {
            kind: "not a token".into(),
            start: 0,
            end: 1,
        }];
        let at = time();
        let mut span = Span::default();
        span.touch(
            &SegmentView {
                part: 1,
                first: &at,
                last: &at,
                audit: &valid,
                source_offset: 0,
                source_len: 1,
            }
            .with_retained(b"<redacted:email>"),
        );
        span.touch(
            &SegmentView {
                part: 2,
                first: &at,
                last: &at,
                audit: &invalid,
                source_offset: 0,
                source_len: 1,
            }
            .with_retained(b"raw"),
        );
        assert!(span.redacted);
        assert_eq!(span.kinds, BTreeSet::from(["email".into()]));
    }

    #[test]
    fn invalid_projection_marks_a_fresh_span_even_without_a_safe_kind() {
        let invalid = [RedactionAuditSpan {
            kind: "not a token".into(),
            start: 0,
            end: 1,
        }];
        let at = time();
        let mut span = Span::default();
        span.touch(
            &SegmentView {
                part: 1,
                first: &at,
                last: &at,
                audit: &invalid,
                source_offset: 0,
                source_len: 1,
            }
            .with_retained(b"raw"),
        );
        assert!(span.redacted);
        assert!(span.kinds.is_empty());
    }

    #[test]
    fn empty_retained_invalid_projection_marks_fresh_safe_and_unsafe_spans() {
        for (kind, expected) in [
            ("email", BTreeSet::from(["email".into()])),
            ("not a token", BTreeSet::new()),
        ] {
            let invalid = [RedactionAuditSpan {
                kind: kind.into(),
                start: 0,
                end: 1,
            }];
            let at = time();
            let mut span = Span::default();
            span.touch(
                &SegmentView {
                    part: 1,
                    first: &at,
                    last: &at,
                    audit: &invalid,
                    source_offset: 0,
                    source_len: 1,
                }
                .with_retained(b""),
            );
            assert!(span.redacted, "kind {kind}");
            assert_eq!(span.kinds, expected, "kind {kind}");
        }
    }

    #[test]
    fn monotone_audit_cursor_is_linear_and_attributes_adjacent_spans() {
        let count = 64usize;
        let mut retained = Vec::new();
        let mut audits = Vec::new();
        for index in 0..count {
            let kind = format!("k{index}");
            retained.extend_from_slice(format!("<redacted:{kind}>").as_bytes());
            audits.push(RedactionAuditSpan {
                kind,
                start: index as u64,
                end: index as u64 + 1,
            });
        }
        let at = time();
        let raw = SegmentView {
            part: 1,
            first: &at,
            last: &at,
            audit: &audits,
            source_offset: 0,
            source_len: count as u64,
        };
        let view = raw.with_retained(&retained);
        assert!(!view.audit.invalid);
        let mut cursor = AuditCursor::default();
        let mut span = Span::default();
        for offset in 0..retained.len() {
            span.touch_range(&view, &mut cursor, offset, offset + 1);
        }
        assert!(span.redacted);
        assert_eq!(cursor.next, count);
        assert!(cursor.inspections <= retained.len() + count);
    }

    #[derive(Default)]
    struct Budget {
        used: usize,
        incomplete: bool,
    }

    impl SseContext for Budget {
        fn take(&mut self, bytes: usize) -> bool {
            if self.used.saturating_add(bytes) > MAX_BUFFERED_BYTES {
                false
            } else {
                self.used += bytes;
                true
            }
        }

        fn give(&mut self, bytes: usize) {
            self.used -= bytes;
        }

        fn mark_incomplete(&mut self) {
            self.incomplete = true;
        }
    }

    #[test]
    fn multiline_join_cannot_reuse_the_source_events_live_budget() {
        let line = "x".repeat(400_000);
        let event = format!("data: {line}\ndata: {line}\n").into_bytes();
        let joined_len = 800_001;
        let mut sse = Sse {
            event,
            ..Sse::default()
        };
        let mut budget = Budget {
            // Includes the event itself and leaves one byte less than the joined copy needs.
            used: MAX_BUFFERED_BYTES - joined_len + 1,
            ..Budget::default()
        };
        let mut item = None;
        sse.dispatch(&mut Span::default(), &mut budget, &mut |seen, _, _| {
            item = Some(matches!(seen, SseItem::OverBudget));
        });
        assert_eq!(item, Some(true));
        assert!(budget.incomplete);
    }

    #[test]
    fn provider_aggregation_cannot_reuse_a_single_line_events_live_budget() {
        let event = b"data: payload\n".to_vec();
        let event_len = event.len();
        let mut sse = Sse {
            event,
            ..Sse::default()
        };
        let mut budget = Budget {
            // Includes the framed event and leaves eight bytes, one short of the provider's
            // nine-byte aggregation request.
            used: MAX_BUFFERED_BYTES - 8,
            ..Budget::default()
        };
        let mut provider = Vec::new();
        let mut appended = None;
        sse.dispatch(&mut Span::default(), &mut budget, &mut |seen, _, budget| {
            let SseItem::Data { .. } = seen else {
                panic!("expected data")
            };
            appended = Some(append_bounded(&mut provider, b"aggregate", budget));
        });
        assert_eq!(appended, Some(false));
        assert!(provider.is_empty());
        assert_eq!(budget.used, MAX_BUFFERED_BYTES - 8 - event_len);
    }
}
