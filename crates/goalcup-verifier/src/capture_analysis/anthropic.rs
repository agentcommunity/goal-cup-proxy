//! The bounded Anthropic Messages decoder (`anthropic-messages`).
//!
//! [`AnthropicDecoder`] reads the ordered capture records of a protocol-v5 trace and turns
//! the retained, sanitized bodies of Anthropic Messages exchanges into source-referenced
//! [`Observation`]s. It decodes structure only: supplied model names, IDs,
//! tool calls and results, usage objects, stop reasons and error types. It never joins a
//! continuation to earlier context, never classifies authorship or assistance, and never
//! turns a provider `message_stop` into a task verdict or a latency.
//!
//! **Selection.** Only a stream whose `Open` carries the protocol hint exactly
//! `anthropic-messages` is looked at; every other stream is ignored and holds no state. Of
//! those, a `WireRequest` `POST /v1/messages` `application/json` body is one logical JSON
//! record (or `<base>/messages` under a gateway base path, v4); a `WireResponse` for that
//! path is framed as server-sent events when its content type is `text/event-stream`, or
//! read as one JSON error document when it is
//! `application/json` with a non-2xx status. Content-type checks apply to HTTP streams only.
//! This protocol has no WebSocket form, so a hinted `WsClient`/`WsServer`
//! stream, like any other endpoint, method, content type or 2xx JSON body, is counted once as
//! `unknown` with its reason.
//!
//! **Supported set.** Recorded from the sanitized real Claude Code 2.1.283 exchange in
//! `tests/fixtures/m2-anthropic/`: request content blocks `text` and `tool_result`; response
//! events `message_start`, `content_block_start`, `content_block_delta`, `content_block_stop`,
//! `message_delta`, `message_stop` and `ping`; response blocks `text`, `thinking`, `tool_use`;
//! deltas `text_delta`, `thinking_delta`, `signature_delta`, `input_json_delta`. The documented
//! `error` event and JSON error document were not observed on real traffic: they are decoded
//! from synthetic tests only. Anything else is an explicit, counted `unknown` observation.
//!
//! **Observations.** Kinds `model`, `model_request`, `model_response`, `tool_call`,
//! `tool_result`, `usage`, `error`, `outcome` and `unknown`. Each carries the typed wire
//! identity of its stream (`fields.wire`, from `CaptureMeta.wire`, never the label) and the
//! retained segment parts and observation interval of the records it was decoded from. The
//! Anthropic stream supplies no event timestamps, so no source time is ever reported. Usage
//! objects are passed through as supplied, per event scope; the provider documents
//! `message_delta` usage as a cumulative snapshot, so nothing is ever summed here.
//!
//! **Bounds.** One logical record (a request or error body, one SSE event, one tool input
//! assembled from `input_json_delta` fragments) is capped at [`MAX_RECORD_BYTES`], and all
//! parser buffers together at [`MAX_BUFFERED_BYTES`]; at most `MAX_OPEN_STREAMS` streams and
//! `MAX_OPEN_BLOCKS` open content blocks per message are tracked. A record over a bound is
//! reported `over_budget` with its source reference, and decoding continues after it. A
//! request tool result keeps at most `MAX_TEXT_BYTES` of its content's text.
//!
//! **Loss and redaction.** A stream gap, producer loss or missing end marks what was in flight
//! incomplete. An interrupted or decode-error end is recorded as provider detail under D45 and
//! does not by itself degrade the decoder summary. Neither case erases earlier
//! observations, and a later `message_stop` never makes a message with a hole complete. After
//! a loss the SSE framer discards bytes up to the next blank line. An event whose `event`
//! name contradicts its data type, or a malformed or duplicate block start, is an anomaly
//! inside the open message: it too marks the message and its open blocks incomplete, so no
//! tool input is reported complete from fragments that may be missing or misattributed. A
//! logical record that overlaps a redaction audit span and fails to parse is `unknown` with
//! reason `redacted` and its kinds; a truncated prefix is never parsed. The
//! summary is `incomplete` after any loss or limit on this decoder's own streams, and after
//! any stream-less gap or producer recovery, which may have hidden an Anthropic stream; a
//! gap keyed to another protocol's stream does not count.
//!
//! **Shared framing.** Segment spans, the bounded SSE framer, buffer accounting and close
//! classification live in `wire.rs`. Anthropic keeps its provider-specific observation adapter
//! and message state machine, including its existing `fields.source` and wire serialization.

use super::api::{
    DecoderFailure, DecoderSummary, DecoderV1, Observation, ObservationTimeRange, SourceRef,
    StreamOpen, kinds,
};
use super::wire::{
    Close, LossCauses, RetainedSegmentView, SegmentView, Span, Sse as WireSse, SseContext, SseItem,
    append_bounded, classify_close, release_buffer,
};
pub use super::wire::{MAX_BUFFERED_BYTES, MAX_EVENT_BYTES as MAX_RECORD_BYTES};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use goalcup_protocol::capture::{
    CaptureMeta, CaptureRecord, CaptureSource, MAX_METADATA_STRING_BYTES, MAX_OPEN_STREAMS,
    StreamKey, WireIdentity,
};
use serde_json::{Map, Value, json};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::io;

/// The decoder identity declared in its summary.
pub const DECODER_ID: &str = "anthropic-messages";
/// The interpretation version. Any change to emitted observations changes it.
pub const DECODER_VERSION: &str = "4";
/// The exact protocol hint of a stream this decoder reads.
pub const PROTOCOL_HINT: &str = "anthropic-messages";

/// Whether this decoder reads a stream: the exact hint, whatever the source. The one
/// selection rule; [`DecoderV1::accepts`] and the decoder's own guard both call it.
fn selects(hint: Option<&str>) -> bool {
    hint == Some(PROTOCOL_HINT)
}

/// The endpoint's last path segment. The CLI hints `<base>/messages` and `<base>/v1/messages`
/// for a gateway model host (`model_host.rs` `WIRE_PATHS`), so a base prefix such as
/// `/claude-code` comes before it; `/v1/messages/count_tokens` and the like do not end in it.
const ENDPOINT_SUFFIX: &str = "/messages";
/// Open content blocks tracked per message; the provider streams them one at a time.
const MAX_OPEN_BLOCKS: usize = 64;
/// A provider code or type: short and from a conservative alphabet.
const MAX_TOKEN_BYTES: usize = 128;
/// The text kept of one replayed tool result's content.
const MAX_TEXT_BYTES: usize = 16 * 1024;
/// Distinct redaction kinds named on one record.
const MAX_REDACTION_KINDS: usize = 16;

/// The bounded Anthropic Messages decoder. See the module documentation.
pub struct AnthropicDecoder {
    streams: BTreeMap<StreamKey, Tracked>,
    totals: Totals,
}

impl AnthropicDecoder {
    pub fn new() -> Self {
        Self {
            streams: BTreeMap::new(),
            totals: Totals::default(),
        }
    }
}

impl Default for AnthropicDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl AnthropicDecoder {
    fn read(
        &mut self,
        record: &CaptureRecord,
        emit: &mut dyn FnMut(Observation),
    ) -> io::Result<()> {
        let Self { streams, totals } = self;
        let mut out = Out { emit, t: totals };
        match record {
            CaptureRecord::Open {
                stream,
                source,
                metadata,
                ..
            } => {
                // `accepts` selected this stream; this is a guard, not the selection.
                if !selects(metadata.protocol_hint.as_deref()) {
                    debug_assert!(false, "a stream `accepts` refused was routed here");
                    return Ok(());
                }
                if let Some(old) = streams.remove(stream) {
                    old.end(Close::Unterminated, &mut out);
                }
                if streams.len() >= MAX_OPEN_STREAMS {
                    out.t.unknown += 1;
                    out.t.incomplete = true;
                    return Ok(());
                }
                streams.insert(stream.clone(), Tracked::open(stream, *source, metadata));
            }
            CaptureRecord::Segment {
                stream,
                part,
                source_offset,
                source_len,
                first,
                last,
                data_b64,
                audit,
            } => {
                if let Some(tracked) = streams.get_mut(stream) {
                    let view = SegmentView {
                        part: *part,
                        first,
                        last,
                        audit: &audit.spans,
                        source_offset: *source_offset,
                        source_len: *source_len,
                    };
                    tracked.segment(&view, data_b64, &mut out);
                }
            }
            // A keyed gap concerns only its own stream: untracked keys belong to other
            // protocols (their Open is in the chain). A stream-less gap may have lost
            // an Anthropic Open this decoder never saw, so it always limits coverage.
            CaptureRecord::Gap { stream, .. } => match stream {
                None => out.t.incomplete = true,
                Some(key) => {
                    if let Some(tracked) = streams.get_mut(key) {
                        tracked.lose("gap", &mut out);
                    }
                }
            },
            CaptureRecord::End { stream, reason, .. } => {
                if let Some(tracked) = streams.remove(stream) {
                    tracked.end(Close::End(*reason), &mut out);
                }
            }
            CaptureRecord::Recovery {
                previous_producer, ..
            } => {
                out.t.incomplete = true;
                close_producer(streams, previous_producer, Close::ProducerLost, &mut out);
            }
            CaptureRecord::ProducerClosed { producer, .. } => {
                close_producer(streams, producer, Close::Unterminated, &mut out);
            }
            CaptureRecord::Connection { .. }
            | CaptureRecord::Launch { .. }
            | CaptureRecord::Assistance { .. }
            | CaptureRecord::LocalFinish { .. } => {}
        }
        Ok(())
    }

    fn summarize(&mut self, emit: &mut dyn FnMut(Observation)) -> io::Result<DecoderSummary> {
        let Self { streams, totals } = self;
        let mut out = Out { emit, t: totals };
        for (_, tracked) in std::mem::take(streams) {
            tracked.end(Close::Unterminated, &mut out);
        }
        debug_assert_eq!(self.totals.buffered, 0, "every parser buffer is released");
        Ok(DecoderSummary::new(
            self.totals.observations,
            self.totals.unknown,
            self.totals.incomplete,
        ))
    }
}

impl DecoderV1 for AnthropicDecoder {
    fn id(&self) -> &'static str {
        DECODER_ID
    }

    fn version(&self) -> &'static str {
        DECODER_VERSION
    }

    fn accepts(&self, open: &StreamOpen<'_>) -> bool {
        selects(open.protocol_hint)
    }

    fn push(
        &mut self,
        record: &CaptureRecord,
        emit: &mut dyn FnMut(Observation),
    ) -> Result<(), DecoderFailure> {
        self.read(record, emit)
            .map_err(|error| DecoderFailure::from_io(&error))
    }

    fn push_recovery(
        &mut self,
        record: &CaptureRecord,
        lost_streams: &[StreamKey],
        emit: &mut dyn FnMut(Observation),
    ) -> Result<(), DecoderFailure> {
        if !matches!(record, CaptureRecord::Recovery { .. }) {
            return Err(DecoderFailure::new("recovery_scope_unsupported"));
        }
        let Self { streams, totals } = self;
        let mut out = Out { emit, t: totals };
        out.t.incomplete = true;
        for key in lost_streams {
            if let Some(tracked) = streams.remove(key) {
                tracked.end(Close::ProducerLost, &mut out);
            }
        }
        Ok(())
    }

    fn finish(
        mut self: Box<Self>,
        emit: &mut dyn FnMut(Observation),
    ) -> Result<DecoderSummary, DecoderFailure> {
        self.summarize(emit)
            .map_err(|error| DecoderFailure::from_io(&error))
    }
}

fn close_producer(
    streams: &mut BTreeMap<StreamKey, Tracked>,
    producer: &str,
    close: Close,
    out: &mut Out<'_>,
) {
    let keys: Vec<StreamKey> = streams
        .keys()
        .filter(|key| key.producer == producer)
        .cloned()
        .collect();
    for key in keys {
        if let Some(tracked) = streams.remove(&key) {
            tracked.end(close, out);
        }
    }
}

// ---------------------------------------------------------------------------------------
// Output, counters and the shared buffer budget.
// ---------------------------------------------------------------------------------------

#[derive(Default)]
struct Totals {
    observations: u64,
    unknown: u64,
    incomplete: bool,
    buffered: usize,
}

struct Out<'a> {
    emit: &'a mut dyn FnMut(Observation),
    t: &'a mut Totals,
}

impl Out<'_> {
    fn put(
        &mut self,
        ctx: &Ctx,
        span: &Span,
        kind: &str,
        supplied_id: Option<String>,
        mut fields: Map<String, Value>,
    ) {
        fields.insert("source".into(), json!(ctx.source));
        fields.insert("wire".into(), json!(ctx.wire));
        self.t.observations += 1;
        if kind == kinds::UNKNOWN {
            self.t.unknown += 1;
        }
        (self.emit)(Observation {
            source: SourceRef {
                stream: ctx.key.clone(),
                first_part: span.first.as_ref().expect("an emitted fact has a span").0,
                last_part: span.last.as_ref().expect("an emitted fact has a span").0,
            },
            observed: ObservationTimeRange {
                first: span
                    .first
                    .as_ref()
                    .expect("an emitted fact has a span")
                    .1
                    .clone(),
                last: span
                    .last
                    .as_ref()
                    .expect("an emitted fact has a span")
                    .1
                    .clone(),
            },
            kind: kind.to_string(),
            supplied_id,
            fields: Value::Object(fields),
        });
    }

    fn unknown(&mut self, ctx: &Ctx, span: &Span, reason: &str, mut fields: Map<String, Value>) {
        fields.insert("reason".into(), json!(reason));
        self.put(ctx, span, kinds::UNKNOWN, None, fields);
    }
}

impl SseContext for Out<'_> {
    fn take(&mut self, bytes: usize) -> bool {
        match self.t.buffered.checked_add(bytes) {
            Some(total) if total <= MAX_BUFFERED_BYTES => {
                self.t.buffered = total;
                true
            }
            _ => false,
        }
    }

    fn give(&mut self, bytes: usize) {
        debug_assert!(bytes <= self.t.buffered, "buffer accounting underflow");
        self.t.buffered = self.t.buffered.saturating_sub(bytes);
    }

    fn mark_incomplete(&mut self) {
        self.t.incomplete = true;
    }
}

fn redaction_fields(kinds: &BTreeSet<String>) -> Map<String, Value> {
    let mut fields = Map::new();
    fields.insert("kinds".into(), json!(kinds));
    fields
}

// ---------------------------------------------------------------------------------------
// One tracked stream.
// ---------------------------------------------------------------------------------------

struct Ctx {
    key: StreamKey,
    source: CaptureSource,
    wire: Option<WireIdentity>,
    status: Option<u16>,
    path: Option<String>,
}

struct Tracked {
    ctx: Ctx,
    body: Body,
    /// Every retained part of the stream seen so far.
    span: Option<Span>,
    /// The source offset the next segment continues at; `None` after a loss.
    next_offset: Option<u64>,
    /// The fixed invalid-audit diagnostic was emitted before discarding its bytes.
    invalid_audit_reported: bool,
    /// Independent transport damage to an otherwise unsupported selected record.
    unsupported_losses: LossCauses,
}

enum Body {
    Json(JsonBody),
    Sse(Box<StreamEvents>),
    Unsupported(&'static str),
}

#[derive(Clone, Copy)]
enum Purpose {
    Request,
    ErrorDocument,
}

impl Tracked {
    fn open(key: &StreamKey, source: CaptureSource, meta: &CaptureMeta) -> Self {
        Self {
            ctx: Ctx {
                key: key.clone(),
                source,
                wire: meta.wire.clone(),
                status: meta.status,
                path: meta.path.clone(),
            },
            body: select(source, meta),
            span: None,
            next_offset: Some(0),
            invalid_audit_reported: false,
            unsupported_losses: LossCauses::default(),
        }
    }

    fn segment(&mut self, view: &SegmentView<'_>, data_b64: &str, out: &mut Out<'_>) {
        if let Some(expected) = self.next_offset
            && expected != view.source_offset
        {
            self.lose("discontinuity", out);
        }
        self.next_offset = view.source_offset.checked_add(view.source_len);
        let Ok(data) = STANDARD.decode(data_b64) else {
            match &mut self.span {
                Some(span) => span.touch_unprojected(view),
                None => {
                    let mut span = Span::default();
                    span.touch_unprojected(view);
                    self.span = Some(span);
                }
            }
            self.lose("malformed_segment", out);
            return;
        };
        let view = view.with_retained(&data);
        match &mut self.span {
            Some(span) => span.touch(&view),
            None => {
                let mut span = Span::default();
                span.touch(&view);
                self.span = Some(span);
            }
        }
        if view.invalid_audit() {
            let span = self.span.as_ref().expect("the invalid segment was touched");
            out.unknown(
                &self.ctx,
                span,
                "invalid_redaction_audit",
                redaction_fields(&span.kinds),
            );
            self.invalid_audit_reported = true;
            self.lose("invalid_redaction_audit", out);
            return;
        }
        match &mut self.body {
            Body::Json(body) => body.feed(&data, &view, out),
            Body::Sse(sse) => sse.feed(&self.ctx, &data, &view, out),
            Body::Unsupported(_) => {}
        }
    }

    fn lose(&mut self, reason: &'static str, out: &mut Out<'_>) {
        out.t.incomplete = true;
        self.next_offset = None;
        match &mut self.body {
            Body::Json(body) => body.lose(reason, out),
            Body::Sse(sse) => sse.lose(&self.ctx, self.span.as_ref(), reason, out),
            Body::Unsupported(_) => self.unsupported_losses.remember(reason),
        }
    }

    fn end(mut self, close: Close, out: &mut Out<'_>) {
        let close = classify_close(close);
        if close.loss {
            out.t.incomplete = true;
        }
        match &mut self.body {
            Body::Json(body) => body.finish(
                &self.ctx,
                self.span.as_ref(),
                close.finding,
                self.invalid_audit_reported,
                out,
            ),
            Body::Sse(sse) => sse.finish(
                &self.ctx,
                self.span.as_ref(),
                close.finding,
                close.loss,
                out,
            ),
            Body::Unsupported(reason) => {
                if let Some(span) = &self.span {
                    for loss in self.unsupported_losses.findings().into_iter().flatten() {
                        if loss != "invalid_redaction_audit" || !self.invalid_audit_reported {
                            out.unknown(&self.ctx, span, loss, Map::new());
                        }
                    }
                    out.unknown(&self.ctx, span, reason, Map::new());
                    if let Some(close) = close.finding {
                        out.unknown(&self.ctx, span, close, Map::new());
                    }
                }
            }
        }
    }
}

fn select(source: CaptureSource, meta: &CaptureMeta) -> Body {
    let content_type = meta.content_type.as_deref();
    let endpoint = meta
        .path
        .as_deref()
        .is_some_and(|path| path.ends_with(ENDPOINT_SUFFIX));
    match source {
        CaptureSource::WsClient | CaptureSource::WsServer => {
            Body::Unsupported("unsupported_transport")
        }
        CaptureSource::Native | CaptureSource::Goal => Body::Unsupported("unsupported_source"),
        _ if !endpoint => Body::Unsupported("unsupported_endpoint"),
        CaptureSource::WireRequest => {
            if meta.method.as_deref() != Some("POST") {
                Body::Unsupported("unsupported_method")
            } else if content_type != Some("application/json") {
                Body::Unsupported("unsupported_content_type")
            } else {
                Body::Json(JsonBody::new(Purpose::Request))
            }
        }
        CaptureSource::WireResponse => match content_type {
            Some("text/event-stream") => Body::Sse(Box::default()),
            Some("application/json") if meta.status.is_some_and(|s| !(200..300).contains(&s)) => {
                Body::Json(JsonBody::new(Purpose::ErrorDocument))
            }
            Some("application/json") => Body::Unsupported("unsupported_response_form"),
            _ => Body::Unsupported("unsupported_content_type"),
        },
    }
}

// ---------------------------------------------------------------------------------------
// JSON bodies: a request, or a non-2xx error document.
// ---------------------------------------------------------------------------------------

struct JsonBody {
    purpose: Purpose,
    buf: Vec<u8>,
    losses: LossCauses,
    redacted: BTreeSet<String>,
}

impl JsonBody {
    fn new(purpose: Purpose) -> Self {
        Self {
            purpose,
            buf: Vec::new(),
            losses: LossCauses::default(),
            redacted: BTreeSet::new(),
        }
    }

    fn feed(&mut self, data: &[u8], view: &RetainedSegmentView<'_>, out: &mut Out<'_>) {
        let mut span = Span::default();
        span.touch(view);
        for kind in span.kinds {
            if self.redacted.len() < MAX_REDACTION_KINDS {
                self.redacted.insert(kind);
            }
        }
        if !self.losses.is_empty() {
            return;
        }
        if !append_bounded(&mut self.buf, data, out) {
            self.losses.remember("over_budget");
            out.t.incomplete = true;
        }
    }

    fn lose(&mut self, reason: &'static str, out: &mut Out<'_>) {
        self.losses.remember(reason);
        release_buffer(&mut self.buf, out);
    }

    fn finish(
        &mut self,
        ctx: &Ctx,
        span: Option<&Span>,
        ending: Option<&'static str>,
        invalid_audit_reported: bool,
        out: &mut Out<'_>,
    ) {
        if let Some(span) = span {
            if !self.losses.is_empty() {
                for reason in self.losses.findings().into_iter().flatten() {
                    if reason != "invalid_redaction_audit" || !invalid_audit_reported {
                        out.unknown(ctx, span, reason, Map::new());
                    }
                }
                if let Some(ending) = ending {
                    out.unknown(ctx, span, ending, Map::new());
                }
            } else if let Some(reason) = ending {
                out.unknown(ctx, span, reason, Map::new());
            } else {
                match serde_json::from_slice::<Value>(&self.buf) {
                    Ok(value) => match self.purpose {
                        Purpose::Request => decode_request(ctx, span, &value, out),
                        Purpose::ErrorDocument => decode_error_document(ctx, span, &value, out),
                    },
                    Err(_) => {
                        out.t.incomplete = true;
                        if self.redacted.is_empty() {
                            out.unknown(ctx, span, "malformed", Map::new());
                        } else {
                            out.unknown(ctx, span, "redacted", redaction_fields(&self.redacted));
                        }
                    }
                }
            }
        }
        release_buffer(&mut self.buf, out);
    }
}

#[derive(Default)]
struct RequestCounts {
    text: u64,
    tool_result: u64,
    unsupported: u64,
}

fn decode_request(ctx: &Ctx, span: &Span, value: &Value, out: &mut Out<'_>) {
    let Some(request) = value.as_object() else {
        out.unknown(ctx, span, "unsupported_request_form", Map::new());
        return;
    };
    let mut withheld = BTreeSet::new();

    let mut model = Map::new();
    put(
        &mut model,
        &mut withheld,
        "model",
        text(request.get("model")),
    );
    if model.contains_key("model") {
        model.insert("scope".into(), json!("request"));
        out.put(ctx, span, kinds::MODEL, None, model);
    }

    let messages = request.get("messages").and_then(Value::as_array);
    let mut counts = RequestCounts::default();
    for message in messages.into_iter().flatten() {
        match message.get("content") {
            Some(Value::String(_)) => counts.text += 1,
            Some(Value::Array(blocks)) => {
                for block in blocks {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => counts.text += 1,
                        Some("tool_result") => counts.tool_result += 1,
                        _ => counts.unsupported += 1,
                    }
                }
            }
            _ => counts.unsupported += 1,
        }
    }

    let mut fields = Map::new();
    fields.insert("endpoint".into(), json!(ctx.path));
    if let Some(stream) = request.get("stream").and_then(Value::as_bool) {
        fields.insert("stream".into(), json!(stream));
    }
    if let Some(max_tokens) = request.get("max_tokens").and_then(Value::as_u64) {
        fields.insert("max_tokens".into(), json!(max_tokens));
    }
    if let Some(thread) = request.get("thread").and_then(Value::as_object) {
        let mut supplied = Map::new();
        put(
            &mut supplied,
            &mut withheld,
            "type",
            token(thread.get("type")),
        );
        put(
            &mut supplied,
            &mut withheld,
            "previous_message_id",
            text(thread.get("previous_message_id")),
        );
        fields.insert("thread".into(), Value::Object(supplied));
    }
    if let Some(messages) = messages {
        fields.insert("message_count".into(), json!(messages.len()));
    }
    fields.insert(
        "content_block_counts".into(),
        json!({
            "text": counts.text,
            "tool_result": counts.tool_result,
            "unsupported": counts.unsupported,
        }),
    );
    if let Some(tools) = request.get("tools").and_then(Value::as_array) {
        fields.insert("tool_definition_count".into(), json!(tools.len()));
    }
    put_withheld(&mut fields, &withheld);
    out.put(ctx, span, kinds::MODEL_REQUEST, None, fields);

    for (message_index, message) in messages.into_iter().flatten().enumerate() {
        match message.get("content") {
            Some(Value::String(_)) => {}
            Some(Value::Array(blocks)) => {
                for (block_index, block) in blocks.iter().enumerate() {
                    request_block(ctx, span, message_index, block_index, block, out);
                }
            }
            _ => {
                let mut fields = Map::new();
                fields.insert("message_index".into(), json!(message_index));
                out.unknown(ctx, span, "unsupported_message_form", fields);
            }
        }
    }
}

/// A request `tool_result` is a provider-source observation of what the harness sent,
/// content verbatim up to its bound (including any harness-appended text), whether or not a
/// native copy exists. It is one occurrence per request: a request that replays history
/// repeats earlier results, which are never unique results. Text blocks are counted, not
/// emitted; other blocks are explicit unknowns.
fn request_block(
    ctx: &Ctx,
    span: &Span,
    message_index: usize,
    block_index: usize,
    block: &Value,
    out: &mut Out<'_>,
) {
    let mut withheld = BTreeSet::new();
    let mut fields = Map::new();
    fields.insert("message_index".into(), json!(message_index));
    fields.insert("block_index".into(), json!(block_index));
    match block.get("type").and_then(Value::as_str) {
        Some("text") => {}
        Some("tool_result") => {
            let id = supplied(&mut withheld, "tool_use_id", block.get("tool_use_id"));
            if let Some(is_error) = block.get("is_error").and_then(Value::as_bool) {
                fields.insert("is_error".into(), json!(is_error));
            }
            if let Some(content) = block.get("content") {
                put_content(&mut fields, content);
            }
            put_withheld(&mut fields, &withheld);
            out.put(ctx, span, kinds::TOOL_RESULT, id, fields);
        }
        _ => {
            put(
                &mut fields,
                &mut withheld,
                "block_type",
                token(block.get("type")),
            );
            put_withheld(&mut fields, &withheld);
            out.unknown(ctx, span, "unsupported_content_block", fields);
        }
    }
}

/// A tool result's `content` as supplied, with `content_bytes`, the length of its text (a
/// string's own, anything else's compact JSON). Text over [`MAX_TEXT_BYTES`] is cut on a
/// character boundary and kept as a string, with `content_truncated`.
fn put_content(fields: &mut Map<String, Value>, content: &Value) {
    let text = match content {
        Value::String(text) => Cow::Borrowed(text.as_str()),
        other => Cow::Owned(other.to_string()),
    };
    fields.insert("content_bytes".into(), json!(text.len()));
    if text.len() <= MAX_TEXT_BYTES {
        fields.insert("content".into(), content.clone());
        return;
    }
    let mut end = MAX_TEXT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    fields.insert("content".into(), json!(&text[..end]));
    fields.insert("content_truncated".into(), json!(true));
}

fn decode_error_document(ctx: &Ctx, span: &Span, value: &Value, out: &mut Out<'_>) {
    if value.get("type").and_then(Value::as_str) == Some("error")
        && let Some((fields, id)) = error_fields(ctx, value, "http_response")
    {
        out.put(ctx, span, kinds::ERROR, id, fields);
    } else {
        out.unknown(ctx, span, "unsupported_error_form", Map::new());
    }
}

/// The safe fields of a provider error: its supplied type as a token, the HTTP status of
/// the stream and the supplied request ID. Message text and anything else stay behind.
fn error_fields(
    ctx: &Ctx,
    value: &Value,
    scope: &str,
) -> Option<(Map<String, Value>, Option<String>)> {
    let Str::Value(error_type) = token(value.get("error")?.get("type")) else {
        return None;
    };
    let mut withheld = BTreeSet::new();
    let mut fields = Map::new();
    fields.insert("scope".into(), json!(scope));
    fields.insert("error_type".into(), json!(error_type));
    if let Some(status) = ctx.status {
        fields.insert("http_status".into(), json!(status));
    }
    let id = supplied(&mut withheld, "request_id", value.get("request_id"));
    put_withheld(&mut fields, &withheld);
    Some((fields, id))
}

// ---------------------------------------------------------------------------------------
// Server-sent events: shared framing, provider-specific message state.
// ---------------------------------------------------------------------------------------

#[derive(Default)]
struct StreamEvents {
    framer: WireSse,
    span: Span,
    messages: Messages,
    eager_resync_causes: LossCauses,
}

impl StreamEvents {
    fn feed(&mut self, ctx: &Ctx, data: &[u8], view: &RetainedSegmentView<'_>, out: &mut Out<'_>) {
        let Self {
            framer,
            span,
            messages,
            eager_resync_causes,
        } = self;
        framer.feed(span, view, data, out, &mut |item, span, out| {
            if matches!(item, SseItem::Gap) {
                *eager_resync_causes = LossCauses::default();
            }
            messages.on_event(ctx, item, span, out)
        });
    }

    fn lose(&mut self, ctx: &Ctx, history: Option<&Span>, reason: &'static str, out: &mut Out<'_>) {
        let active_resync = !self.eager_resync_causes.is_empty();
        let partial = (self.span.first.is_some()).then(|| std::mem::take(&mut self.span));
        let pending_over_budget = self.framer.lose(&mut out.t.buffered);
        let attribution = partial.as_ref().or(history);
        if pending_over_budget && !self.eager_resync_causes.contains("over_budget") {
            if let Some(span) = attribution {
                out.unknown(ctx, span, "over_budget", Map::new());
                self.messages.lose_event("over_budget_event", span);
                self.eager_resync_causes.remember("over_budget");
            }
        } else if !active_resync && let Some(span) = &partial {
            out.unknown(ctx, span, "partial_event", Map::new());
            self.messages.lose_event("partial_event", span);
        }
        if !self.eager_resync_causes.contains(reason) {
            let represented = self.messages.lose(reason);
            if reason != "invalid_redaction_audit"
                && !represented
                && let Some(span) = attribution
            {
                out.unknown(ctx, span, reason, Map::new());
            }
            if represented || attribution.is_some() {
                self.eager_resync_causes.remember(reason);
            }
        }
    }

    fn finish(
        &mut self,
        ctx: &Ctx,
        history: Option<&Span>,
        ending: Option<&'static str>,
        decoder_loss: bool,
        out: &mut Out<'_>,
    ) {
        let Self {
            framer,
            span,
            messages,
            eager_resync_causes,
        } = self;
        let mut pending = std::mem::take(framer).finish(&mut out.t.buffered, span, ending);
        if pending == Some("gap") && !eager_resync_causes.is_empty() {
            pending = None;
        }
        let attribution = (span.first.is_some()).then_some(&*span).or(history);
        if let Some(reason) = pending {
            let reason = if reason == "unterminated_event" {
                "partial_event"
            } else {
                reason
            };
            if span.first.is_some() {
                out.unknown(ctx, span, reason, Map::new());
                if decoder_loss || ending.is_none() {
                    out.t.incomplete = true;
                    messages.lose_event(reason, span);
                } else {
                    messages.note_event(reason, span);
                }
            }
        }
        if let Some(reason) = ending {
            if decoder_loss {
                messages.lose(reason);
            } else {
                messages.note(reason);
            }
            if messages.current.is_none()
                && pending != Some(reason)
                && let Some(span) = attribution
            {
                out.unknown(ctx, span, reason, Map::new());
            }
        }
        messages.finish(ctx, ending.unwrap_or("complete"), out);
    }
}

#[derive(Default)]
struct Messages {
    current: Option<Message>,
}

struct Message {
    id: Option<String>,
    role: Option<String>,
    span: Span,
    stop_reason: Option<String>,
    stopped: bool,
    losses: BTreeSet<&'static str>,
    capture_loss: bool,
    withheld: BTreeSet<&'static str>,
    counts: BlockCounts,
    blocks: BTreeMap<u64, Block>,
}

#[derive(Default)]
struct BlockCounts {
    text: u64,
    thinking: u64,
    tool_use: u64,
    unsupported: u64,
}

struct Block {
    span: Span,
    tainted: bool,
    redacted: bool,
    kind: BlockKind,
}

enum BlockKind {
    Text,
    Thinking,
    ToolUse(ToolUse),
    Unsupported,
}

struct ToolUse {
    id: Option<String>,
    name: Option<String>,
    withheld: BTreeSet<&'static str>,
    input: Vec<u8>,
    input_bytes: u64,
    over: bool,
    unsupported_start: bool,
}

impl Messages {
    fn on_event(&mut self, ctx: &Ctx, event: SseItem<'_>, span: &Span, out: &mut Out<'_>) {
        let (name, data) = match event {
            SseItem::OverBudget => {
                out.unknown(ctx, span, "over_budget", Map::new());
                self.lose_event("over_budget_event", span);
                return;
            }
            SseItem::Gap => {
                // `Tracked::lose` already records the gap and flushes any pending event using
                // Anthropic's provider-specific `partial_event` vocabulary. The shared framer's
                // resync marker carries no additional provider record.
                return;
            }
            SseItem::Data { name, data } => (name, data),
        };
        let Ok(value) = serde_json::from_slice::<Value>(data) else {
            out.t.incomplete = true;
            if span.kinds.is_empty() {
                out.unknown(ctx, span, "malformed", Map::new());
                self.lose_event("malformed_event", span);
            } else {
                out.unknown(ctx, span, "redacted", redaction_fields(&span.kinds));
                self.lose_event("redacted_event", span);
            }
            return;
        };
        let kind = value.get("type").and_then(Value::as_str);
        if let (Some(name), Some(kind)) = (name, kind)
            && name != kind.as_bytes()
        {
            // Either reading may be the true one: whatever it carried is lost to the message.
            unsupported(ctx, span, "event_type_mismatch", kind, out);
            self.lose_event("anomalous_event", span);
            return;
        }
        let redacted = !span.kinds.is_empty();
        match kind {
            Some("ping") => {}
            Some("message_start") => self.message_start(ctx, span, &value, out),
            Some("error") => self.error(ctx, span, &value, out),
            Some(
                kind @ ("content_block_start"
                | "content_block_delta"
                | "content_block_stop"
                | "message_delta"
                | "message_stop"),
            ) => match &mut self.current {
                Some(message) if !message.stopped => {
                    message.span.merge(span);
                    match kind {
                        "content_block_start" => message.block_start(ctx, span, &value, out),
                        "content_block_delta" => {
                            message.block_delta(ctx, span, &value, redacted, out)
                        }
                        "content_block_stop" => message.block_stop(ctx, span, &value, out),
                        "message_delta" => message.delta(ctx, span, &value, out),
                        _ => message.stop(ctx, span, out),
                    }
                }
                Some(_) => unsupported(ctx, span, "event_after_message_stop", kind, out),
                None => unsupported(ctx, span, "event_outside_message", kind, out),
            },
            Some(other) => unsupported(ctx, span, "unsupported_event", other, out),
            None => {
                let named = name.and_then(|n| std::str::from_utf8(n).ok());
                unsupported(ctx, span, "unsupported_event", named.unwrap_or(""), out);
            }
        }
    }

    /// A record of this stream that could not be interpreted: the open message has a hole,
    /// and any open block may have lost a fragment.
    fn lose_event(&mut self, loss: &'static str, span: &Span) {
        if let Some(message) = &mut self.current
            && !message.stopped
        {
            message.span.merge(span);
            message.mark_loss(loss);
        }
    }

    fn note_event(&mut self, detail: &'static str, span: &Span) {
        if let Some(message) = &mut self.current
            && !message.stopped
        {
            message.span.merge(span);
            message.mark_detail(detail);
        }
    }

    fn lose(&mut self, reason: &'static str) -> bool {
        if let Some(message) = &mut self.current
            && !message.stopped
        {
            message.mark_loss(reason);
            true
        } else {
            false
        }
    }

    fn note(&mut self, detail: &'static str) {
        if let Some(message) = &mut self.current
            && !message.stopped
        {
            message.mark_detail(detail);
        }
    }

    fn message_start(&mut self, ctx: &Ctx, span: &Span, value: &Value, out: &mut Out<'_>) {
        if let Some(previous) = self.current.take() {
            previous.finish(ctx, "next_message", out);
        }
        let Some(message) = value.get("message").and_then(Value::as_object) else {
            unsupported(ctx, span, "unsupported_event_form", "message_start", out);
            return;
        };
        let mut withheld = BTreeSet::new();
        let id = supplied(&mut withheld, "id", message.get("id"));
        let role = optional(&mut withheld, "role", token(message.get("role")));
        let mut model = Map::new();
        put(
            &mut model,
            &mut withheld,
            "model",
            text(message.get("model")),
        );
        if model.contains_key("model") {
            model.insert("scope".into(), json!("message_start"));
            out.put(ctx, span, kinds::MODEL, id.clone(), model);
        }
        if let Some(usage) = message.get("usage").filter(|u| u.is_object()) {
            let mut fields = Map::new();
            fields.insert("scope".into(), json!("message_start"));
            fields.insert("reading".into(), json!("initial_snapshot"));
            fields.insert("usage".into(), usage.clone());
            out.put(ctx, span, kinds::USAGE, id.clone(), fields);
        }
        if message
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|content| !content.is_empty())
        {
            unsupported(
                ctx,
                span,
                "unsupported_message_content",
                "message_start",
                out,
            );
        }
        self.current = Some(Message {
            id,
            role,
            span: span.clone(),
            stop_reason: None,
            stopped: false,
            losses: BTreeSet::new(),
            capture_loss: false,
            withheld,
            counts: BlockCounts::default(),
            blocks: BTreeMap::new(),
        });
    }

    fn error(&mut self, ctx: &Ctx, span: &Span, value: &Value, out: &mut Out<'_>) {
        match error_fields(ctx, value, "stream_event") {
            Some((fields, id)) => out.put(ctx, span, kinds::ERROR, id, fields),
            None => unsupported(ctx, span, "unsupported_error_form", "error", out),
        }
        self.lose_event("error_event", span);
    }

    fn finish(&mut self, ctx: &Ctx, stream_end: &str, out: &mut Out<'_>) {
        if let Some(message) = self.current.take() {
            message.finish(ctx, stream_end, out);
        }
    }
}

impl Message {
    fn mark_loss(&mut self, loss: &'static str) {
        self.capture_loss = true;
        self.mark_detail(loss);
    }

    fn mark_detail(&mut self, detail: &'static str) {
        self.losses.insert(detail);
        for block in self.blocks.values_mut() {
            block.tainted = true;
        }
    }

    fn block_start(&mut self, ctx: &Ctx, span: &Span, value: &Value, out: &mut Out<'_>) {
        let (Some(index), Some(block)) = (
            value.get("index").and_then(Value::as_u64),
            value.get("content_block").and_then(Value::as_object),
        ) else {
            unsupported(
                ctx,
                span,
                "unsupported_event_form",
                "content_block_start",
                out,
            );
            // Later fragments cannot be attributed with certainty: no open block completes.
            self.mark_loss("anomalous_event");
            return;
        };
        if self.blocks.contains_key(&index) {
            unsupported(
                ctx,
                span,
                "duplicate_block_index",
                "content_block_start",
                out,
            );
            self.mark_loss("anomalous_event");
            return;
        }
        if self.blocks.len() >= MAX_OPEN_BLOCKS {
            out.t.incomplete = true;
            out.unknown(ctx, span, "over_budget", Map::new());
            self.mark_loss("over_budget_event");
            return;
        }
        let kind = match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                self.counts.text += 1;
                BlockKind::Text
            }
            Some("thinking") => {
                self.counts.thinking += 1;
                BlockKind::Thinking
            }
            Some("tool_use") => {
                self.counts.tool_use += 1;
                let mut withheld = BTreeSet::new();
                let id = supplied(&mut withheld, "id", block.get("id"));
                let name = optional(&mut withheld, "name", text(block.get("name")));
                // Streaming starts a tool input as `{}` and builds it from fragments.
                let unsupported_start = block
                    .get("input")
                    .is_some_and(|input| input.as_object().is_none_or(|o| !o.is_empty()));
                if unsupported_start {
                    let mut fields = Map::new();
                    fields.insert("index".into(), json!(index));
                    out.unknown(ctx, span, "unsupported_tool_input_form", fields);
                }
                BlockKind::ToolUse(ToolUse {
                    id,
                    name,
                    withheld,
                    input: Vec::new(),
                    input_bytes: 0,
                    over: false,
                    unsupported_start,
                })
            }
            _ => {
                self.counts.unsupported += 1;
                let mut fields = Map::new();
                let mut withheld = BTreeSet::new();
                fields.insert("index".into(), json!(index));
                put(
                    &mut fields,
                    &mut withheld,
                    "block_type",
                    token(block.get("type")),
                );
                put_withheld(&mut fields, &withheld);
                out.unknown(ctx, span, "unsupported_content_block", fields);
                BlockKind::Unsupported
            }
        };
        self.blocks.insert(
            index,
            Block {
                span: span.clone(),
                tainted: false,
                redacted: false,
                kind,
            },
        );
    }

    fn block_delta(
        &mut self,
        ctx: &Ctx,
        span: &Span,
        value: &Value,
        redacted: bool,
        out: &mut Out<'_>,
    ) {
        let index = value.get("index").and_then(Value::as_u64);
        let Some(block) = index.and_then(|index| self.blocks.get_mut(&index)) else {
            unsupported(ctx, span, "delta_without_block", "content_block_delta", out);
            // The bytes cannot be assigned to a block, so provider completion cannot make the
            // message complete. Already bound blocks remain independently attributable.
            self.capture_loss = true;
            self.losses.insert("delta_without_block");
            return;
        };
        block.span.merge(span);
        block.redacted |= redacted;
        let delta = value.get("delta");
        let delta_type = delta.and_then(|d| d.get("type"));
        let supported = match (&mut block.kind, delta_type.and_then(Value::as_str)) {
            (BlockKind::Unsupported, _) => true,
            (BlockKind::Text, Some("text_delta")) => true,
            (BlockKind::Thinking, Some("thinking_delta" | "signature_delta")) => true,
            (BlockKind::ToolUse(tool), Some("input_json_delta")) => {
                let fragment = delta
                    .and_then(|d| d.get("partial_json"))
                    .and_then(Value::as_str);
                if let Some(fragment) = fragment {
                    tool.input_bytes += fragment.len() as u64;
                    if !tool.over && !append_bounded(&mut tool.input, fragment.as_bytes(), out) {
                        tool.over = true;
                        out.t.incomplete = true;
                    }
                }
                fragment.is_some()
            }
            _ => false,
        };
        if !supported {
            if matches!(block.kind, BlockKind::ToolUse(_)) {
                block.tainted = true;
                // This provider fragment was observed but could not be incorporated into the
                // tool input. Unlike an otherwise faithful early transport end, that discarded
                // input is decoder capture loss and must remain sticky through finish.
                self.capture_loss = true;
            }
            let mut fields = Map::new();
            let mut withheld = BTreeSet::new();
            fields.insert("index".into(), json!(index));
            put(&mut fields, &mut withheld, "delta_type", token(delta_type));
            put_withheld(&mut fields, &withheld);
            out.unknown(ctx, span, "unsupported_delta", fields);
        }
    }

    fn block_stop(&mut self, ctx: &Ctx, span: &Span, value: &Value, out: &mut Out<'_>) {
        let index = value.get("index").and_then(Value::as_u64);
        let Some((index, mut block)) = index.and_then(|index| self.blocks.remove_entry(&index))
        else {
            unsupported(ctx, span, "stop_without_block", "content_block_stop", out);
            return;
        };
        block.span.merge(span);
        emit_tool_call(ctx, self.id.as_deref(), index, block, true, true, out);
    }

    fn delta(&mut self, ctx: &Ctx, span: &Span, value: &Value, out: &mut Out<'_>) {
        if let Some(delta) = value.get("delta").and_then(Value::as_object) {
            let reason = token(delta.get("stop_reason"));
            if let Some(reason) = optional(&mut self.withheld, "stop_reason", reason) {
                self.stop_reason = Some(reason);
            }
        }
        if let Some(usage) = value.get("usage").filter(|u| u.is_object()) {
            let mut fields = Map::new();
            fields.insert("scope".into(), json!("message_delta"));
            fields.insert("reading".into(), json!("cumulative_snapshot"));
            fields.insert("usage".into(), usage.clone());
            out.put(ctx, span, kinds::USAGE, self.id.clone(), fields);
        }
    }

    /// A provider's end of one message: not a task outcome, not a clean session.
    fn stop(&mut self, ctx: &Ctx, span: &Span, out: &mut Out<'_>) {
        self.stopped = true;
        let mut fields = Map::new();
        fields.insert("scope".into(), json!("provider_message"));
        fields.insert("event".into(), json!("message_stop"));
        if let Some(reason) = &self.stop_reason {
            fields.insert("stop_reason".into(), json!(reason));
        }
        out.put(ctx, span, kinds::OUTCOME, self.id.clone(), fields);
    }

    fn finish(mut self, ctx: &Ctx, stream_end: &str, out: &mut Out<'_>) {
        if !self.stopped {
            self.losses.insert("missing_message_stop");
        }
        if !self.blocks.is_empty() {
            self.losses.insert("unstopped_block");
        }
        let complete = self.losses.is_empty();
        let faithful_early_end = matches!(stream_end, "interrupted" | "decode_error");
        let incomplete_is_loss = self.capture_loss || !faithful_early_end;
        for (index, block) in std::mem::take(&mut self.blocks) {
            emit_tool_call(
                ctx,
                self.id.as_deref(),
                index,
                block,
                false,
                incomplete_is_loss,
                out,
            );
        }
        if self.capture_loss || (!complete && !faithful_early_end) {
            out.t.incomplete = true;
        }
        let mut fields = Map::new();
        if let Some(status) = ctx.status {
            fields.insert("http_status".into(), json!(status));
        }
        if let Some(role) = &self.role {
            fields.insert("role".into(), json!(role));
        }
        if let Some(reason) = &self.stop_reason {
            fields.insert("stop_reason".into(), json!(reason));
        }
        fields.insert(
            "block_counts".into(),
            json!({
                "text": self.counts.text,
                "thinking": self.counts.thinking,
                "tool_use": self.counts.tool_use,
                "unsupported": self.counts.unsupported,
            }),
        );
        fields.insert("message_complete".into(), json!(complete));
        fields.insert("losses".into(), json!(self.losses));
        fields.insert("stream_end".into(), json!(stream_end));
        put_withheld(&mut fields, &self.withheld);
        out.put(ctx, &self.span, kinds::MODEL_RESPONSE, self.id, fields);
    }
}

fn unsupported(ctx: &Ctx, span: &Span, reason: &str, event_type: &str, out: &mut Out<'_>) {
    let mut fields = Map::new();
    if let Str::Value(event_type) = token(Some(&json!(event_type))) {
        fields.insert("event_type".into(), json!(event_type));
    }
    out.unknown(ctx, span, reason, fields);
}

/// Emits a `tool_use` block as a `tool_call`; other block kinds emit nothing on their own.
fn emit_tool_call(
    ctx: &Ctx,
    message_id: Option<&str>,
    index: u64,
    block: Block,
    stopped: bool,
    incomplete_is_loss: bool,
    out: &mut Out<'_>,
) {
    let BlockKind::ToolUse(mut tool) = block.kind else {
        return;
    };
    let mut fields = Map::new();
    if let Some(message_id) = message_id {
        fields.insert("message_id".into(), json!(message_id));
    }
    fields.insert("index".into(), json!(index));
    if let Some(name) = &tool.name {
        fields.insert("name".into(), json!(name));
    }
    fields.insert("input_bytes".into(), json!(tool.input_bytes));
    let status = if tool.over {
        "over_budget"
    } else if !stopped || block.tainted {
        "incomplete"
    } else if tool.unsupported_start {
        "unsupported"
    } else if tool.input.is_empty() {
        // No fragment arrived: the documented empty start object is the whole input.
        fields.insert("input".into(), json!({}));
        "complete"
    } else {
        match serde_json::from_slice::<Value>(&tool.input) {
            Ok(input) => {
                fields.insert("input".into(), input);
                "complete"
            }
            Err(_) if block.redacted => "redacted",
            Err(_) => "invalid_json",
        }
    };
    if incomplete_is_loss && !matches!(status, "complete" | "unsupported") {
        out.t.incomplete = true;
    }
    fields.insert("input_status".into(), json!(status));
    put_withheld(&mut fields, &tool.withheld);
    release_buffer(&mut tool.input, out);
    out.put(ctx, &block.span, kinds::TOOL_CALL, tool.id, fields);
}

// ---------------------------------------------------------------------------------------
// Bounded supplied strings.
// ---------------------------------------------------------------------------------------

enum Str {
    Absent,
    Value(String),
    Withheld,
}

/// A supplied display string: at most one metadata string long, no control characters.
fn text(value: Option<&Value>) -> Str {
    match value {
        None | Some(Value::Null) => Str::Absent,
        Some(Value::String(s))
            if s.len() <= MAX_METADATA_STRING_BYTES && !s.chars().any(char::is_control) =>
        {
            Str::Value(s.clone())
        }
        Some(_) => Str::Withheld,
    }
}

/// A supplied code or type: a short token from a conservative alphabet.
fn token(value: Option<&Value>) -> Str {
    match value {
        None | Some(Value::Null) => Str::Absent,
        Some(Value::String(s)) if token_ok(s) => Str::Value(s.clone()),
        Some(_) => Str::Withheld,
    }
}

fn token_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_TOKEN_BYTES
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
}

fn put(
    fields: &mut Map<String, Value>,
    withheld: &mut BTreeSet<&'static str>,
    key: &'static str,
    value: Str,
) {
    if let Some(value) = optional(withheld, key, value) {
        fields.insert(key.into(), json!(value));
    }
}

/// A supplied value, or `None`; one present but out of bounds is named in `withheld`.
fn optional(
    withheld: &mut BTreeSet<&'static str>,
    key: &'static str,
    value: Str,
) -> Option<String> {
    match value {
        Str::Value(value) => Some(value),
        Str::Withheld => {
            withheld.insert(key);
            None
        }
        Str::Absent => None,
    }
}

/// A supplied provider ID for `Observation::supplied_id`.
fn supplied(
    withheld: &mut BTreeSet<&'static str>,
    key: &'static str,
    value: Option<&Value>,
) -> Option<String> {
    optional(withheld, key, text(value))
}

/// Supplied fields present but out of bounds are named, never silently dropped.
fn put_withheld(fields: &mut Map<String, Value>, withheld: &BTreeSet<&'static str>) {
    if !withheld.is_empty() {
        fields.insert("withheld".into(), json!(withheld));
    }
}
