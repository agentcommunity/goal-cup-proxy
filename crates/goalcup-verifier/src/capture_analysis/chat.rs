//! The bounded OpenAI Chat Completions decoder (`openai-chat-completions`).
//!
//! [`ChatCompletionsDecoder`] implements [`DecoderV1`] over the ordered stream of every capture
//! record of a trace. It reads only retained, sanitized transport records; it runs no model, opens
//! no network connection and reads no clock. Hermes with DeepSeek speaks this wire.
//!
//! **Selection.** Only a stream whose `Open` carries the exact protocol hint [`PROTOCOL_HINT`] on a
//! `wire_request` or `wire_response` source is decoded, never because of its host. A request body
//! is selected as `application/json`; a response body as `text/event-stream` (a streamed
//! completion) or `application/json` (a non-stream completion or an error body). Chat Completions
//! has no WebSocket form, so WebSocket sources are not accepted. Any other selected body is counted
//! as unknown, never guessed at.
//!
//! **Logical records.** An HTTP JSON body is one record. An SSE body is framed into events at blank
//! lines by the shared framer (`wire.rs`, the framer `openai.rs` uses), whatever segment boundaries
//! its lines cross; each event is one chunk. A chunk is decoded only once it is complete. A `Gap`,
//! an over-budget event, a chunk that does not parse (`malformed`, or `redacted` when a
//! contributing segment carried an audit span) or an event of an unsupported shape becomes one
//! `unknown` observation and is never partly read; after a gap bytes are discarded through the next
//! blank line. An event of an unsupported shape (a keepalive object, say) was read whole: it is an
//! `unknown` but not a lost chunk. A lost chunk is counted: the stream's aggregated observations
//! then say `stream_complete: false` and `unreadable_events`, and the kept text and arguments stop
//! at the hole. They stay a prefix of what was sent, marked `*_truncated`, never spliced across it;
//! the full length of what was read still counts. A `choices` member that is not an array, a choice
//! that is not an object, and a `delta` that is neither an object nor null are lost chunks too
//! (`unsupported_shape`, `unsupported_choice`): content may have been in what was skipped.
//!
//! **Aggregation.** A completion arrives as many chunks, so two facts are assembled across them,
//! per choice: the message text (`delta.content`, concatenated in arrival order) and each tool call
//! (`delta.tool_calls`, merged by the supplied `index`: the id, type and function name from the
//! first chunk that carries them, never concatenated; the `arguments` text concatenated). A
//! streamed call with no `index` (or a delta naming an id already held) merges by its supplied
//! `id`; with neither it is one `unknown` (`tool_call_without_index`) and a hole. Only a non-stream
//! body is read by position, since its entries are whole. If a supplied id and index already name
//! different calls, the delta is one `unknown` (`tool_call_identity_conflict`) and a hole; it is
//! appended to neither call. Nothing else is joined by text or order. They are emitted once, when
//! `[DONE]` arrives or the stream ends. Everything else is read per chunk and emitted at once.
//!
//! **`[DONE]`.** Everything up to the `[DONE]` event is complete and decoded as above. After it, an
//! event with non-empty data is never decoded into observations: the stream's trailing events are
//! reported as one `unknown` with reason `data_after_done` (`events` and `bytes` for complete
//! events with data, `partial_events` for one cut off at the end) over their segments. A gap or
//! over-budget event after `[DONE]` is reported once, under its own name. A stream that ends
//! without `[DONE]` still yields what arrived, with `stream_complete: false`, and an `unknown` with
//! the reason it ended.
//!
//! **Observations** (`kind`, then what `fields` holds; every observation also carries
//! `capture_source`, the typed `wire` identity from its `Open`, and `redaction_kinds` when a
//! contributing segment carried an audit span):
//! - `model_request`: one per request body: supplied `stream`, `include_usage`, `tools_count`,
//!   `tool_choice` (a token, or `object`) and the structure of `messages` (roles, supplied
//!   `tool_call_id`s and call ids, text byte counts, never the text). Request content is context
//!   the harness sent, not a human input; repeated requests stay separate.
//! - `model`: a supplied model identifier, `side` `request`, or `response`, once per distinct
//!   model a response's chunks name (chunks may each carry one).
//! - `model_response`: one per choice: `choice_index`, `delivery` (`stream` or `body`), `role`,
//!   the bounded `content` text, `reasoning_bytes` (only the size of any reasoning delta),
//!   `chunks` that touched the choice, and the completeness fields. Unrecognized delta members
//!   are named in `unrecognized_delta_fields`.
//! - `tool_call`: one per tool call of a choice (its `index` when supplied): `call_id`, `type`,
//!   `name` and the bounded `arguments` text. The arguments are not parsed: they may be cut
//!   mid-JSON.
//! - `tool_result`: a `role: "tool"` request message: `call_id` and the bounded output.
//! - `usage`: the supplied integer fields of one `usage` object, scope `provider_response`, one
//!   observation per chunk that carries one. Never summed, never defaulted.
//! - `outcome`: a supplied `finish_reason` of one choice, scope `provider_response`. It is the
//!   provider's statement about one response: never task completion, never a transport close.
//! - `error`: the safe `code`/`type` tokens of an in-stream `error` object or of an HTTP JSON
//!   `error` body (with the recorded HTTP status). The message is never kept: a 401 echoes a
//!   masked key in it. A present code or type that is not a safe token is named in `withheld`.
//! - `unknown`: one per record that could not be interpreted, with its `reason`.
//!
//! **Ending.** An `End` that is not `complete` leaves an `unknown` with its reason (`interrupted`,
//! `decode_error`), `done_seen` and whether a partial event was discarded. Under D45 an interrupted
//! or decode-error stream is reported, not a failure: the decoder's summary stays complete when
//! nothing was lost. A lost producer (`Recovery`), a stream still open at `finish`, a `Gap` and an
//! over-budget record do mark the summary incomplete.
//!
//! **Time.** `observed` spans the contributing retained segments in their own clock: an interval,
//! never a latency. A provider-supplied time stays a separate `provider_created`.
//!
//! **Measured forms.** None yet: the live Hermes/DeepSeek row was captured before this decoder
//! existed and has not been re-decoded. Every form follows the public Chat Completions wire shapes
//! and is tested on synthetic bytes only.
//!
//! **Bounds.** At most [`MAX_EVENT_BYTES`] per logical record and [`MAX_BUFFERED_BYTES`] over every
//! open stream, counting the text kept for aggregation; at most `MAX_OPEN_STREAMS` tracked streams.
//! Kept text is cut at [`MAX_TEXT_BYTES`] with its full length and a `*_truncated` flag; at most 16
//! choices and 64 tool calls per stream are kept.

use super::api::{DecoderFailure, DecoderSummary, DecoderV1, Observation, StreamOpen, kinds};
use super::wire::{
    Close, LossCauses, MAX_ID_BYTES, MAX_LISTED, MAX_NAMES, MAX_TEXT_BYTES, Origin, Out,
    ResyncCause, SegmentView, Span, Sse, SseItem, Tally, classify_close, cut, id, is_token, names,
    parse_object, put_error, put_id, put_output, put_token, put_usage, token,
};
pub use super::wire::{MAX_BUFFERED_BYTES, MAX_EVENT_BYTES};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use goalcup_protocol::capture::{
    CaptureMeta, CaptureRecord, CaptureSource, MAX_OPEN_STREAMS, StreamKey, WireIdentity,
};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io;

/// The decoder identity declared in its summary.
pub const DECODER_ID: &str = "openai-chat-completions";
/// Incremented whenever the interpretation of any record changes.
pub const DECODER_VERSION: &str = "2";
/// The exact profile hint this decoder selects.
pub const PROTOCOL_HINT: &str = "openai-chat-completions";

/// Whether this decoder reads a stream: the exact hint on a wire source, never by host. The
/// one selection rule; [`DecoderV1::accepts`] and the decoder's own guard both call it.
fn selects(source: CaptureSource, hint: Option<&str>) -> bool {
    hint == Some(PROTOCOL_HINT)
        && matches!(
            source,
            CaptureSource::WireRequest | CaptureSource::WireResponse
        )
}

/// The most choices of one response that are kept.
const MAX_CHOICES: usize = MAX_NAMES;

/// The integer usage members Chat Completions documents (and DeepSeek's cache split), and
/// the members that are objects of integers.
const USAGE_COUNTS: &[&str] = &[
    "prompt_tokens",
    "completion_tokens",
    "total_tokens",
    "prompt_cache_hit_tokens",
    "prompt_cache_miss_tokens",
];
const USAGE_DETAILS: &[&str] = &["prompt_tokens_details", "completion_tokens_details"];
/// Delta members that are recognized (and, but for the ones read, carry nothing kept).
const DELTA_MEMBERS: &[&str] = &[
    "role",
    "content",
    "reasoning_content",
    "reasoning",
    "tool_calls",
    "refusal",
];

/// The bounded streaming decoder of OpenAI Chat Completions traffic.
#[derive(Debug, Default)]
pub struct ChatCompletionsDecoder {
    streams: BTreeMap<StreamKey, Tracked>,
    buffered: usize,
    tally: Tally,
}

impl ChatCompletionsDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    fn open(&mut self, stream: &StreamKey, source: CaptureSource, metadata: &CaptureMeta) {
        // `accepts` selected this stream; this is a guard, not the selection.
        if !selects(source, metadata.protocol_hint.as_deref()) {
            debug_assert!(false, "a stream `accepts` refused was routed here");
            return;
        }
        let content_type = metadata.content_type.as_deref();
        let framing = match source {
            CaptureSource::WireRequest | CaptureSource::WireResponse
                if content_type == Some("application/json") =>
            {
                Framing::Json(Vec::new())
            }
            CaptureSource::WireResponse if content_type == Some("text/event-stream") => {
                Framing::Sse(Box::default())
            }
            CaptureSource::WireRequest | CaptureSource::WireResponse => {
                Framing::Unsupported(metadata.content_type.clone())
            }
            // Refused by `selects` above; the arm only keeps the match exhaustive.
            _ => return,
        };
        if self.streams.contains_key(stream) {
            // A reused stream key is invalid evidence (the reader refuses it); the first stays.
            return;
        }
        if self.streams.len() >= MAX_OPEN_STREAMS {
            // Untracked, so its records are lost to this analysis: counted, never guessed.
            self.tally.unknown += 1;
            self.tally.incomplete = true;
            return;
        }
        self.streams.insert(
            stream.clone(),
            Tracked {
                source,
                wire: metadata.wire.clone(),
                status: metadata.status,
                framing,
                span: Span::default(),
                loss: LossCauses::default(),
                invalid_audit_loss: false,
                resync_cause: None,
            },
        );
    }

    fn segment(
        &mut self,
        stream: &StreamKey,
        view: &SegmentView<'_>,
        data_b64: &str,
        emit: &mut dyn FnMut(Observation),
    ) {
        let Self {
            streams,
            buffered,
            tally,
        } = self;
        let Some(tracked) = streams.get_mut(stream) else {
            return;
        };
        let Ok(data) = STANDARD.decode(data_b64) else {
            tracked.span.touch_unprojected(view);
            tracked.damage(buffered, tally, "undecodable_segment");
            return;
        };
        let view = view.with_retained(&data);
        if view.invalid_audit() {
            tracked.span.touch(&view);
            if let Framing::Sse(body) = &mut tracked.framing {
                body.reply.discarded(&tracked.span);
            }
            let origin = Origin {
                stream,
                source: tracked.source,
                wire: tracked.wire.as_ref(),
                status: tracked.status,
            };
            Out {
                tally,
                buffered,
                emit,
            }
            .unknown(
                &origin,
                &tracked.span,
                "invalid_redaction_audit",
                Map::new(),
            );
            tracked.damage(buffered, tally, "invalid_redaction_audit");
            return;
        }
        let Tracked {
            source,
            wire,
            status,
            framing,
            span,
            loss,
            invalid_audit_loss: _,
            resync_cause,
        } = tracked;
        match framing {
            Framing::Json(buf) => {
                span.touch(&view);
                if !loss.is_empty() {
                    return;
                }
                if buf.len() + data.len() > MAX_EVENT_BYTES
                    || *buffered + data.len() > MAX_BUFFERED_BYTES
                {
                    *buffered -= buf.len();
                    *buf = Vec::new();
                    loss.remember("over_budget");
                    tally.incomplete = true;
                } else {
                    buf.extend_from_slice(&data);
                    *buffered += data.len();
                }
            }
            Framing::Unsupported(_) => span.touch(&view),
            Framing::Sse(body) => {
                let origin = Origin {
                    stream,
                    source: *source,
                    wire: wire.as_ref(),
                    status: *status,
                };
                let mut out = Out {
                    tally,
                    buffered,
                    emit,
                };
                let SseBody { sse, reply } = &mut **body;
                sse.feed(
                    span,
                    &view,
                    &data,
                    &mut out,
                    &mut |item, span, out| match item {
                        SseItem::Data { data, .. } => reply.event(data, span, &origin, out),
                        SseItem::Gap => {
                            let findings = resync_cause
                                .take()
                                .map(ResyncCause::findings)
                                .unwrap_or([None, Some("gap"), None]);
                            for reason in findings.into_iter().flatten() {
                                reply.lost(reason, span, &origin, out);
                            }
                        }
                        SseItem::OverBudget => reply.lost("over_budget", span, &origin, out),
                    },
                );
            }
        }
    }

    fn gap(&mut self, stream: &StreamKey) {
        let Self {
            streams,
            buffered,
            tally,
        } = self;
        if let Some(tracked) = streams.get_mut(stream) {
            tracked.damage(buffered, tally, "gap");
        }
    }

    fn recovery(&mut self, lost_streams: &[StreamKey], emit: &mut dyn FnMut(Observation)) {
        self.tally.incomplete = true;
        for key in lost_streams {
            if let Some(tracked) = self.streams.remove(key) {
                self.close(key, tracked, Close::ProducerLost, emit);
            }
        }
    }

    fn close(
        &mut self,
        stream: &StreamKey,
        tracked: Tracked,
        how: Close,
        emit: &mut dyn FnMut(Observation),
    ) {
        let Tracked {
            source,
            wire,
            status,
            framing,
            span,
            loss,
            invalid_audit_loss,
            resync_cause,
        } = tracked;
        let origin = Origin {
            stream,
            source,
            wire: wire.as_ref(),
            status,
        };
        let mut out = Out {
            tally: &mut self.tally,
            buffered: &mut self.buffered,
            emit,
        };
        let close = classify_close(how);
        let cut = close.finding;
        // D45: an interrupted or decode-error end is reported, and the bytes that arrived stay
        // decoded; a lost producer or a stream nobody ended is a loss.
        if close.loss {
            out.tally.incomplete = true;
        }
        match framing {
            Framing::Json(buf) => {
                *out.buffered -= buf.len();
                if !loss.is_empty() {
                    for reason in loss.findings().into_iter().flatten() {
                        if reason != "invalid_redaction_audit" || !invalid_audit_loss {
                            out.unknown(&origin, &span, reason, Map::new());
                        }
                    }
                    if let Some(cut) = cut {
                        out.unknown(&origin, &span, cut, Map::new());
                    }
                } else if let Some(reason) = cut {
                    out.unknown(&origin, &span, reason, Map::new());
                } else if span.first.is_some() {
                    let role = match source {
                        CaptureSource::WireRequest => Role::HttpRequest,
                        _ => Role::HttpJsonResponse,
                    };
                    decode_record(&buf, role, &span, &origin, &mut out);
                }
            }
            Framing::Unsupported(content_type) => {
                if span.first.is_some() || !loss.is_empty() || cut.is_some() {
                    for reason in loss.findings().into_iter().flatten() {
                        if reason != "invalid_redaction_audit" || !invalid_audit_loss {
                            let mut extra = Map::new();
                            if let Some(content_type) = &content_type {
                                extra.insert(
                                    "content_type".into(),
                                    Value::String(content_type.clone()),
                                );
                            }
                            out.unknown(&origin, &span, reason, extra);
                        }
                    }
                    let mut extra = Map::new();
                    if let Some(content_type) = content_type {
                        extra.insert("content_type".into(), Value::String(content_type));
                    }
                    out.unknown(&origin, &span, "unsupported_content_type", extra);
                    if let Some(cut) = cut {
                        out.unknown(&origin, &span, cut, Map::new());
                    }
                }
            }
            Framing::Sse(body) => {
                let SseBody { sse, mut reply } = *body;
                let pending_data = sse.pending_has_data();
                let mut leftover = sse.finish(out.buffered, &span, cut);
                if leftover == Some("gap") {
                    let loss_span = if span.first.is_some() {
                        span.clone()
                    } else {
                        reply.all.clone()
                    };
                    let findings = resync_cause.map(ResyncCause::findings).unwrap_or([
                        None,
                        Some("gap"),
                        None,
                    ]);
                    for reason in findings.into_iter().flatten() {
                        reply.lost(reason, &loss_span, &origin, &mut out);
                    }
                    leftover = None;
                }
                if let Some(reason) = loss.first() {
                    out.unknown(&origin, &span, reason, Map::new());
                } else {
                    reply.close(cut, leftover, pending_data, &span, &origin, &mut out);
                }
            }
        }
    }
}

impl ChatCompletionsDecoder {
    fn read(
        &mut self,
        record: &CaptureRecord,
        emit: &mut dyn FnMut(Observation),
    ) -> io::Result<()> {
        match record {
            CaptureRecord::Open {
                stream,
                source,
                metadata,
                ..
            } => self.open(stream, *source, metadata),
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
                let view = SegmentView {
                    part: *part,
                    first,
                    last,
                    audit: &audit.spans,
                    source_offset: *source_offset,
                    source_len: *source_len,
                };
                self.segment(stream, &view, data_b64, emit);
            }
            CaptureRecord::Gap {
                stream: Some(stream),
                ..
            } => self.gap(stream),
            // A stream-less loss may have taken selected traffic with it.
            CaptureRecord::Gap { stream: None, .. } => self.tally.incomplete = true,
            CaptureRecord::End { stream, reason, .. } => {
                if let Some(tracked) = self.streams.remove(stream) {
                    self.close(stream, tracked, Close::End(*reason), emit);
                }
            }
            CaptureRecord::Recovery {
                previous_producer, ..
            } => {
                let lost: Vec<StreamKey> = self
                    .streams
                    .keys()
                    .filter(|key| &key.producer == previous_producer)
                    .cloned()
                    .collect();
                self.recovery(&lost, emit);
            }
            CaptureRecord::ProducerClosed { producer, .. } => {
                let open: Vec<StreamKey> = self
                    .streams
                    .keys()
                    .filter(|key| &key.producer == producer)
                    .cloned()
                    .collect();
                for key in open {
                    if let Some(tracked) = self.streams.remove(&key) {
                        self.close(&key, tracked, Close::Unterminated, emit);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn summarize(&mut self, emit: &mut dyn FnMut(Observation)) -> io::Result<DecoderSummary> {
        let open = std::mem::take(&mut self.streams);
        for (key, tracked) in open {
            self.close(&key, tracked, Close::Unterminated, emit);
        }
        Ok(DecoderSummary::new(
            self.tally.observations,
            self.tally.unknown,
            self.tally.incomplete,
        ))
    }
}

impl DecoderV1 for ChatCompletionsDecoder {
    fn id(&self) -> &'static str {
        DECODER_ID
    }

    fn version(&self) -> &'static str {
        DECODER_VERSION
    }

    fn accepts(&self, open: &StreamOpen<'_>) -> bool {
        selects(open.source, open.protocol_hint)
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
        self.recovery(lost_streams, emit);
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

/// One selected stream and its current logical record.
#[derive(Debug)]
struct Tracked {
    source: CaptureSource,
    wire: Option<WireIdentity>,
    status: Option<u16>,
    framing: Framing,
    /// The segments contributing to the current record (for SSE: the event being framed).
    span: Span,
    /// Why the current whole-stream record can no longer be decoded.
    loss: LossCauses,
    /// An invalid audit discarded bytes and permanently limits this stream's coverage.
    invalid_audit_loss: bool,
    /// The cause whose discard boundary the shared SSE framer has not consumed yet.
    resync_cause: Option<ResyncCause>,
}

impl Tracked {
    /// A loss inside the current record: release what it buffered and never parse it.
    fn damage(&mut self, buffered: &mut usize, tally: &mut Tally, reason: &'static str) {
        tally.incomplete = true;
        if reason == "invalid_redaction_audit" {
            self.invalid_audit_loss = true;
        }
        match &mut self.framing {
            Framing::Json(buf) => {
                *buffered -= buf.len();
                *buf = Vec::new();
                self.loss.remember(reason);
            }
            Framing::Unsupported(_) => {
                self.loss.remember(reason);
            }
            Framing::Sse(body) => {
                let pending_over_budget = body.sse.lose(buffered);
                if pending_over_budget {
                    ResyncCause::remember(&mut self.resync_cause, "over_budget");
                }
                ResyncCause::remember(&mut self.resync_cause, reason);
            }
        }
    }
}

#[derive(Debug)]
enum Framing {
    /// A whole-stream JSON document: an HTTP request or JSON response body.
    Json(Vec<u8>),
    /// An HTTP `text/event-stream` body: the framer and what the chunks add up to.
    Sse(Box<SseBody>),
    /// A selected HTTP body of another (or no) content type, with that type.
    Unsupported(Option<String>),
}

#[derive(Debug, Default)]
struct SseBody {
    sse: Sse,
    reply: Reply,
}

#[derive(Clone, Copy)]
enum Role {
    HttpRequest,
    HttpJsonResponse,
}

/// How a completion reached the capture.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Delivery {
    Stream,
    Body,
}

impl Delivery {
    fn name(self) -> &'static str {
        match self {
            Self::Stream => "stream",
            Self::Body => "body",
        }
    }
}

/// Decode one complete logical record of a JSON body.
fn decode_record(bytes: &[u8], role: Role, span: &Span, origin: &Origin<'_>, out: &mut Out<'_>) {
    let Some(object) = parse_object(bytes, span, origin, out) else {
        return;
    };
    match role {
        Role::HttpRequest => request(&object, span, origin, out),
        Role::HttpJsonResponse => {
            let mut reply = Reply::default();
            if reply.apply(&object, Delivery::Body, span, origin, out) {
                reply.done = true;
                reply.finalize(Delivery::Body, origin, out);
            } else {
                out.unknown(origin, span, "unsupported_body", Map::new());
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// The response: chunks (or one body) added up.

/// Text kept up to a bound, with its full length and whether it was cut.
#[derive(Debug, Default)]
struct Kept {
    text: String,
    total: usize,
    truncated: bool,
}

impl Kept {
    /// Appends `add`, keeping at most `limit` bytes in all and `room` more of the shared
    /// budget. Once anything was cut nothing more is kept, so the kept text is always a prefix.
    /// `frozen` is a hole in the stream: a lost chunk cannot be attributed to a choice or a
    /// call, so from then on only the length counts and the text is marked cut.
    /// Returns how many bytes were kept.
    fn push(&mut self, add: &str, limit: usize, room: usize, frozen: bool) -> usize {
        self.total += add.len();
        if frozen {
            self.truncated |= !add.is_empty();
        }
        if self.truncated {
            return 0;
        }
        let room = limit.saturating_sub(self.text.len()).min(room);
        let kept = cut(add, room);
        self.truncated = kept.len() < add.len();
        self.text.push_str(kept);
        kept.len()
    }

    fn put(&self, fields: &mut Map<String, Value>, name: &str) {
        if self.total == 0 && !self.truncated {
            return;
        }
        fields.insert(name.into(), Value::String(self.text.clone()));
        fields.insert(format!("{name}_bytes"), self.total.into());
        if self.truncated {
            fields.insert(format!("{name}_truncated"), Value::Bool(true));
        }
    }
}

/// What merges the deltas of one tool call: its supplied `index`, or in a stream that has none
/// its supplied `id`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum ToolKey {
    Index(u64),
    Id(String),
}

/// One tool call of a choice.
#[derive(Debug, Default)]
struct Tool {
    id: Option<String>,
    kind: Option<String>,
    /// The first non-empty function name, as supplied (a safe token).
    name: Option<String>,
    /// A name was supplied that is no safe token, or a later one differed: none is kept.
    name_withheld: bool,
    arguments: Kept,
    span: Span,
}

/// One choice of a response.
#[derive(Debug, Default)]
struct Choice {
    role: Option<String>,
    content: Kept,
    reasoning_bytes: u64,
    chunks: u64,
    tools: BTreeMap<ToolKey, Tool>,
    tools_omitted: u64,
    unrecognized: BTreeSet<String>,
    span: Span,
}

/// What the chunks of one response add up to, and what was lost on the way.
#[derive(Debug, Default)]
struct Reply {
    choices: BTreeMap<u64, Choice>,
    choices_omitted: bool,
    models: Vec<String>,
    response_id: Option<String>,
    created: Option<Value>,
    /// Chunks read (the `[DONE]` marker is not one).
    chunks: u64,
    /// Events that could not be read: a gap, an over-budget event, a malformed chunk.
    unreadable: u64,
    /// A chunk was lost before `[DONE]` (a gap, an over-budget event, a malformed chunk, a
    /// partial event at the end). Nothing read after it is appended to kept text.
    hole: bool,
    /// Whether `[DONE]` arrived, and the aggregates were emitted.
    done: bool,
    finalized: bool,
    /// Every event so far, for the observations about the stream as a whole.
    all: Span,
    after: After,
    /// Bytes of kept text held against the shared budget.
    held: usize,
}

/// The events after `[DONE]`, which are reported and never decoded.
#[derive(Debug, Default)]
struct After {
    /// Complete events with non-empty data, and their data bytes.
    events: u64,
    bytes: u64,
    /// Events cut off at the end of the stream, whose bytes are not known to be complete.
    partial: u64,
    span: Span,
}

impl Reply {
    /// One complete SSE event's data.
    fn event(&mut self, data: &[u8], span: &Span, origin: &Origin<'_>, out: &mut Out<'_>) {
        if self.done {
            if !data.is_empty() {
                self.after.events += 1;
                self.after.bytes += data.len() as u64;
                self.after.span.merge(span);
            }
            return;
        }
        self.all.merge(span);
        if data.trim_ascii() == b"[DONE]" {
            self.done = true;
            self.finalize(Delivery::Stream, origin, out);
            return;
        }
        let Some(object) = parse_object(data, span, origin, out) else {
            self.unreadable += 1;
            self.hole = true;
            return;
        };
        if !self.apply(&object, Delivery::Stream, span, origin, out) {
            let mut extra = Map::new();
            put_token(&mut extra, "object", object.get("object"));
            // Read whole and said nothing a delta could have carried (a keepalive, say): an
            // unknown, but not a lost chunk. Only framer losses and parse failures are holes.
            out.unknown(origin, span, "unsupported_shape", extra);
        }
    }

    /// An event the framer could not hand over.
    fn lost(&mut self, reason: &str, span: &Span, origin: &Origin<'_>, out: &mut Out<'_>) {
        if !self.done {
            self.all.merge(span);
            self.unreadable += 1;
            self.hole = true;
        }
        out.unknown(origin, span, reason, Map::new());
    }

    /// Bytes were discarded by the decoder's own invalid-audit boundary. They are loss in the
    /// aggregate, but the fixed audit diagnostic is the only record emitted for them.
    fn discarded(&mut self, span: &Span) {
        if !self.done {
            self.all.merge(span);
            self.unreadable += 1;
            self.hole = true;
        }
    }

    /// Reads one chunk (or one non-stream body). `false` when it is none of a completion
    /// chunk, a usage-only chunk and an error object.
    fn apply(
        &mut self,
        object: &Map<String, Value>,
        delivery: Delivery,
        span: &Span,
        origin: &Origin<'_>,
        out: &mut Out<'_>,
    ) -> bool {
        let choices = object.get("choices").and_then(Value::as_array);
        let usage = object.get("usage").and_then(Value::as_object);
        let error = object.get("error").and_then(Value::as_object);
        // `choices` present but of another shape may have carried content the decoder cannot
        // read: a lost chunk. Absent or null is not (a keepalive has no `choices`).
        let bad_choices = matches!(object.get("choices"), Some(v) if !v.is_array() && !v.is_null());
        if choices.is_none() && usage.is_none() && error.is_none() {
            if bad_choices {
                self.unreadable += 1;
                self.hole = true;
            }
            return false;
        }
        if bad_choices {
            self.unreadable += 1;
            self.hole = true;
            out.unknown(origin, span, "unsupported_shape", Map::new());
        }
        let chunk = self.chunks;
        self.chunks += 1;
        let (event, member) = match delivery {
            Delivery::Stream => ("chat.completion.chunk", "delta"),
            Delivery::Body => ("chat.completion", "message"),
        };
        // A later chunk with another id is still this stream's; the first id stays.
        if self.response_id.is_none() {
            self.response_id = id(object.get("id"));
        }
        if self.created.is_none() {
            self.created = object.get("created").filter(|v| v.is_number()).cloned();
        }
        let response_id = self.response_id.clone();
        let positioned = |fields: &mut Map<String, Value>| {
            fields.insert("event".into(), Value::String(event.into()));
            if delivery == Delivery::Stream {
                fields.insert("chunk_index".into(), chunk.into());
            }
        };

        if let Some(model) = token(object.get("model"))
            && !self.models.contains(&model)
        {
            if self.models.len() < MAX_NAMES {
                self.models.push(model.clone());
                let mut fields = Map::new();
                positioned(&mut fields);
                fields.insert("model".into(), Value::String(model));
                fields.insert("side".into(), Value::String("response".into()));
                put_id(&mut fields, "response_id", object.get("id"));
                out.observe(origin, span, kinds::MODEL, response_id.clone(), fields);
            } else {
                out.tally.incomplete = true;
            }
        }
        if let Some(usage) = usage {
            let mut fields = Map::new();
            positioned(&mut fields);
            fields.insert("scope".into(), Value::String("provider_response".into()));
            put_id(&mut fields, "response_id", object.get("id"));
            put_usage(&mut fields, usage, USAGE_COUNTS, USAGE_DETAILS);
            out.observe(origin, span, kinds::USAGE, response_id.clone(), fields);
        }
        if let Some(error) = error {
            let mut fields = Map::new();
            positioned(&mut fields);
            let scope = match delivery {
                Delivery::Stream => "stream_event",
                Delivery::Body => "http_response",
            };
            fields.insert("scope".into(), Value::String(scope.into()));
            if delivery == Delivery::Body
                && let Some(status) = origin.status
            {
                fields.insert("http_status".into(), status.into());
            }
            put_error(&mut fields, error.get("code"), error.get("type"));
            out.observe(origin, span, kinds::ERROR, None, fields);
        }
        for choice in choices.into_iter().flatten() {
            if let Some(choice) = choice.as_object() {
                self.choice(choice, member, chunk, delivery, span, origin, out);
            } else {
                self.unreadable += 1;
                self.hole = true;
                out.unknown(origin, span, "unsupported_choice", Map::new());
            }
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn choice(
        &mut self,
        choice: &Map<String, Value>,
        member: &str,
        chunk: u64,
        delivery: Delivery,
        span: &Span,
        origin: &Origin<'_>,
        out: &mut Out<'_>,
    ) {
        let index = choice.get("index").and_then(Value::as_u64).unwrap_or(0);
        if !self.choices.contains_key(&index) {
            if self.choices.len() >= MAX_CHOICES {
                self.choices_omitted = true;
                out.tally.incomplete = true;
                return;
            }
            self.choices.insert(index, Choice::default());
        }
        let Self {
            choices,
            held,
            hole,
            unreadable,
            response_id,
            created,
            ..
        } = self;
        let kept = choices.get_mut(&index).expect("inserted above");
        kept.span.merge(span);
        kept.chunks += 1;
        let mut unindexed = 0u64;
        let mut identity_conflicts = 0u64;
        // A `delta` (or `message`) that is neither an object nor null may have carried content.
        let mut skipped = false;
        let delta = match choice.get(member) {
            Some(Value::Object(delta)) => Some(delta),
            Some(Value::Null) | None => None,
            Some(_) => {
                skipped = true;
                None
            }
        };
        if let Some(delta) = delta {
            if kept.role.is_none() {
                kept.role = token(delta.get("role"));
            }
            let mut frozen = *hole;
            let mut add = |kept_text: &mut Kept, text: &str, limit: usize, frozen: bool| {
                let room = MAX_BUFFERED_BYTES.saturating_sub(*out.buffered);
                let added = kept_text.push(text, limit, room, frozen);
                *out.buffered += added;
                *held += added;
            };
            match delta.get("content") {
                Some(Value::String(text)) => add(&mut kept.content, text, MAX_TEXT_BYTES, frozen),
                Some(Value::Array(parts)) => {
                    for text in parts
                        .iter()
                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                    {
                        add(&mut kept.content, text, MAX_TEXT_BYTES, frozen);
                    }
                }
                _ => {}
            }
            for name in ["reasoning_content", "reasoning"] {
                if let Some(text) = delta.get(name).and_then(Value::as_str) {
                    kept.reasoning_bytes += text.len() as u64;
                }
            }
            if let Some(Value::Array(calls)) = delta.get("tool_calls") {
                for (position, call) in calls.iter().enumerate() {
                    let Some(call) = call.as_object() else {
                        continue;
                    };
                    // The supplied `index` merges deltas. A streamed delta that names an id
                    // already held (under either key) is that call's continuation. Without an
                    // index, a streamed call is otherwise keyed by its supplied `id`, never by
                    // position: servers that send each call whole omit the index. Only a
                    // non-stream body, whose entries are whole, is read by position.
                    let supplied = id(call.get("id"));
                    let held_any_key = {
                        let wanted = supplied.as_ref();
                        wanted.and_then(|wanted| {
                            kept.tools
                                .iter()
                                .find(|(_, tool)| tool.id.as_ref() == Some(wanted))
                                .map(|(key, _)| key.clone())
                        })
                    };
                    let mut identity_conflict = false;
                    let at = match (call.get("index").and_then(Value::as_u64), delivery) {
                        (Some(index), Delivery::Stream) => {
                            let own = ToolKey::Index(index);
                            if let Some(tool) = kept.tools.get(&own) {
                                let bound_id_disagrees = supplied.as_ref().is_some_and(|wanted| {
                                    tool.id.as_ref().is_some_and(|held| held != wanted)
                                });
                                let id_points_elsewhere =
                                    held_any_key.as_ref().is_some_and(|held| held != &own);
                                if bound_id_disagrees || id_points_elsewhere {
                                    identity_conflict = true;
                                    None
                                } else {
                                    Some(own)
                                }
                            } else if let Some(previous) = held_any_key {
                                if matches!(previous, ToolKey::Id(_)) {
                                    // Once both identifiers name the call, make the supplied index
                                    // canonical. The Tool keeps its id, so either identifier still
                                    // finds this one bounded entry on later deltas.
                                    let tool =
                                        kept.tools.remove(&previous).expect("key found above");
                                    let replaced = kept.tools.insert(own.clone(), tool);
                                    debug_assert!(replaced.is_none(), "index checked absent above");
                                    Some(own)
                                } else {
                                    identity_conflict = true;
                                    None
                                }
                            } else {
                                Some(own)
                            }
                        }
                        (Some(index), Delivery::Body) => Some(ToolKey::Index(index)),
                        (None, Delivery::Body) => Some(ToolKey::Index(position as u64)),
                        (None, Delivery::Stream) => {
                            held_any_key.or_else(|| supplied.clone().map(ToolKey::Id))
                        }
                    };
                    if identity_conflict {
                        identity_conflicts += 1;
                        *hole = true;
                        frozen = true;
                        continue;
                    }
                    let Some(at) = at else {
                        // Nothing says which call this is, or whether it continues one: it is
                        // reported, and what was kept stops being trustworthy past it.
                        unindexed += 1;
                        *hole = true;
                        frozen = true;
                        continue;
                    };
                    if !kept.tools.contains_key(&at) && kept.tools.len() >= MAX_LISTED {
                        kept.tools_omitted += 1;
                        out.tally.incomplete = true;
                        continue;
                    }
                    let tool = kept.tools.entry(at).or_default();
                    tool.span.merge(span);
                    if tool.id.is_none() {
                        tool.id = id(call.get("id"));
                    }
                    if tool.kind.is_none() {
                        tool.kind = token(call.get("type"));
                    }
                    if let Some(function) = call.get("function").and_then(Value::as_object) {
                        // Like the id, the name is taken once. Servers that repeat it in every
                        // delta must not get it concatenated; a different later name is a
                        // conflict and is withheld, not merged.
                        if let Some(name) = function
                            .get("name")
                            .and_then(Value::as_str)
                            .filter(|name| !name.is_empty())
                            && !tool.name_withheld
                        {
                            match &tool.name {
                                None if is_token(name) => tool.name = Some(name.to_string()),
                                Some(first) if first == name => {}
                                _ => {
                                    tool.name = None;
                                    tool.name_withheld = true;
                                }
                            }
                        }
                        if let Some(text) = function.get("arguments").and_then(Value::as_str) {
                            add(&mut tool.arguments, text, MAX_TEXT_BYTES, frozen);
                        }
                    }
                }
            }
            for name in delta.keys() {
                if !DELTA_MEMBERS.contains(&name.as_str())
                    && is_token(name)
                    && kept.unrecognized.len() < MAX_NAMES
                {
                    kept.unrecognized.insert(name.clone());
                }
            }
        }
        if skipped {
            *unreadable += 1;
            *hole = true;
            out.unknown(origin, span, "unsupported_choice", Map::new());
        }
        *unreadable += unindexed + identity_conflicts;
        for _ in 0..unindexed {
            out.unknown(origin, span, "tool_call_without_index", Map::new());
        }
        for _ in 0..identity_conflicts {
            out.unknown(origin, span, "tool_call_identity_conflict", Map::new());
        }
        if let Some(reason) = token(choice.get("finish_reason")) {
            let mut fields = Map::new();
            let event = match delivery {
                Delivery::Stream => "chat.completion.chunk",
                Delivery::Body => "chat.completion",
            };
            fields.insert("event".into(), Value::String(event.into()));
            if delivery == Delivery::Stream {
                fields.insert("chunk_index".into(), chunk.into());
            }
            fields.insert("scope".into(), Value::String("provider_response".into()));
            fields.insert("choice_index".into(), index.into());
            fields.insert("finish_reason".into(), Value::String(reason));
            if let Some(created) = created {
                fields.insert("provider_created".into(), created.clone());
            }
            out.observe(origin, span, kinds::OUTCOME, response_id.clone(), fields);
        }
    }

    /// Emits what the chunks add up to, once: the message and tool calls of each choice.
    fn finalize(&mut self, delivery: Delivery, origin: &Origin<'_>, out: &mut Out<'_>) {
        if std::mem::replace(&mut self.finalized, true) {
            return;
        }
        *out.buffered -= self.held;
        self.held = 0;
        let unreadable = self.unreadable;
        let complete = self.done && unreadable == 0;
        let response_id = self.response_id.clone();
        let completeness = |fields: &mut Map<String, Value>| {
            fields.insert("delivery".into(), Value::String(delivery.name().into()));
            fields.insert("stream_complete".into(), Value::Bool(complete));
            if unreadable > 0 {
                fields.insert("unreadable_events".into(), unreadable.into());
            }
        };
        let hole = self.hole;
        for (index, mut choice) in std::mem::take(&mut self.choices) {
            if hole {
                // A lost chunk may have belonged to any choice or call: whatever is kept is a
                // prefix of what was sent, and says so.
                choice.content.truncated = true;
                for tool in choice.tools.values_mut() {
                    tool.arguments.truncated = true;
                }
            }
            let mut fields = Map::new();
            completeness(&mut fields);
            fields.insert("choice_index".into(), index.into());
            if let Some(role) = &choice.role {
                fields.insert("role".into(), Value::String(role.clone()));
            }
            choice.content.put(&mut fields, "content");
            if choice.reasoning_bytes > 0 {
                fields.insert("reasoning_bytes".into(), choice.reasoning_bytes.into());
            }
            fields.insert("chunks".into(), choice.chunks.into());
            if choice.tools_omitted > 0 {
                fields.insert("tool_calls_omitted".into(), choice.tools_omitted.into());
            }
            if !choice.unrecognized.is_empty() {
                fields.insert(
                    "unrecognized_delta_fields".into(),
                    names(&choice.unrecognized),
                );
            }
            out.observe(
                origin,
                &choice.span,
                kinds::MODEL_RESPONSE,
                response_id.clone(),
                fields,
            );
            for (at, tool) in choice.tools {
                let mut fields = Map::new();
                completeness(&mut fields);
                fields.insert("choice_index".into(), index.into());
                if let ToolKey::Index(at) = at {
                    fields.insert("index".into(), at.into());
                }
                if let Some(call_id) = &tool.id {
                    fields.insert("call_id".into(), Value::String(call_id.clone()));
                }
                if let Some(kind) = &tool.kind {
                    fields.insert("type".into(), Value::String(kind.clone()));
                }
                if let Some(name) = &tool.name {
                    fields.insert("name".into(), Value::String(name.clone()));
                } else if tool.name_withheld {
                    fields.insert("withheld".into(), Value::Array(vec!["name".into()]));
                }
                tool.arguments.put(&mut fields, "arguments");
                let supplied = tool.id.filter(|call_id| call_id.len() <= MAX_ID_BYTES);
                out.observe(origin, &tool.span, kinds::TOOL_CALL, supplied, fields);
            }
        }
        if self.choices_omitted {
            out.unknown(origin, &self.all, "choices_over_limit", Map::new());
        }
    }

    /// The stream ended (or `finish` found it still open): emit what arrived, then say how
    /// it ended and report anything after `[DONE]`.
    fn close(
        &mut self,
        cut: Option<&'static str>,
        leftover: Option<&'static str>,
        pending_data: bool,
        pending: &Span,
        origin: &Origin<'_>,
        out: &mut Out<'_>,
    ) {
        if leftover == Some("unterminated_event") && cut.is_none() && !self.done {
            out.tally.incomplete = true;
        }
        if leftover.is_some() && !self.done {
            // A partial event was never decoded: one more chunk lost.
            self.unreadable += 1;
            self.hole = true;
        }
        self.finalize(Delivery::Stream, origin, out);
        // An event cut off after `[DONE]` is data after it: counted there. A gap or an
        // over-budget event after it is a loss of its own, reported once under its own name.
        let mut pending_event = leftover;
        if self.done {
            match leftover {
                Some("gap" | "over_budget") => {
                    out.unknown(origin, pending, leftover.unwrap_or_default(), Map::new());
                }
                // Only data counts: a trailing comment is not data after `[DONE]`.
                Some(_) if pending_data => {
                    self.after.partial += 1;
                    self.after.span.merge(pending);
                }
                Some(_) | None => {}
            }
            pending_event = None;
        }
        if self.after.events > 0 || self.after.partial > 0 {
            let mut extra = Map::new();
            extra.insert("events".into(), self.after.events.into());
            extra.insert("bytes".into(), self.after.bytes.into());
            if self.after.partial > 0 {
                extra.insert("partial_events".into(), self.after.partial.into());
            }
            out.unknown(origin, &self.after.span, "data_after_done", extra);
        }
        let mut whole = std::mem::take(&mut self.all);
        whole.merge(pending);
        if cut.is_some() && matches!(pending_event, Some("gap" | "over_budget")) {
            out.unknown(
                origin,
                pending,
                pending_event.expect("matched pending loss"),
                Map::new(),
            );
            pending_event = None;
        }
        match (cut, pending_event) {
            (Some(reason), pending_event) => {
                let mut extra = Map::new();
                extra.insert("done_seen".into(), Value::Bool(self.done));
                if let Some(pending_event) = pending_event {
                    let kind = match pending_event {
                        "gap" | "over_budget" => pending_event,
                        _ => "partial",
                    };
                    extra.insert("pending_event".into(), Value::String(kind.into()));
                }
                out.unknown(origin, &whole, reason, extra);
            }
            (None, Some(reason)) => out.unknown(origin, pending, reason, Map::new()),
            (None, None) if !self.done && whole.first.is_some() => {
                out.unknown(origin, &whole, "missing_done", Map::new());
            }
            (None, None) => {}
        }
    }
}

// ---------------------------------------------------------------------------------------
// The request.

/// A Chat Completions request body.
fn request(object: &Map<String, Value>, span: &Span, origin: &Origin<'_>, out: &mut Out<'_>) {
    let mut fields = Map::new();
    if let Some(flag) = object.get("stream").and_then(Value::as_bool) {
        fields.insert("stream".into(), Value::Bool(flag));
    }
    if let Some(flag) = object
        .get("stream_options")
        .and_then(|options| options.get("include_usage"))
        .and_then(Value::as_bool)
    {
        fields.insert("include_usage".into(), Value::Bool(flag));
    }
    match object.get("tool_choice") {
        Some(Value::String(choice)) if is_token(choice) => {
            fields.insert("tool_choice".into(), Value::String(choice.clone()));
        }
        Some(Value::Object(_)) => {
            fields.insert("tool_choice".into(), Value::String("object".into()));
        }
        _ => {}
    }
    if let Some(Value::Array(tools)) = object.get("tools") {
        fields.insert("tools_count".into(), tools.len().into());
    }
    let messages: &[Value] = match object.get("messages") {
        Some(Value::Array(messages)) => {
            describe_messages(messages, &mut fields);
            messages
        }
        Some(_) => {
            fields.insert("messages_kind".into(), Value::String("unsupported".into()));
            &[]
        }
        None => &[],
    };
    out.observe(origin, span, kinds::MODEL_REQUEST, None, fields);
    if let Some(model) = token(object.get("model")) {
        let mut fields = Map::new();
        fields.insert("model".into(), Value::String(model));
        fields.insert("side".into(), Value::String("request".into()));
        out.observe(origin, span, kinds::MODEL, None, fields);
    }
    for message in messages {
        let Some(message) = message.as_object() else {
            continue;
        };
        if message.get("role").and_then(Value::as_str) == Some("tool") {
            let mut fields = Map::new();
            fields.insert("item_type".into(), Value::String("tool_message".into()));
            fields.insert("origin".into(), Value::String("request_messages".into()));
            put_id(&mut fields, "call_id", message.get("tool_call_id"));
            put_output(&mut fields, message.get("content"));
            out.observe(origin, span, kinds::TOOL_RESULT, None, fields);
        }
    }
}

/// The structure of a request's `messages`: roles, supplied call ids and text sizes, never
/// the text itself.
fn describe_messages(messages: &[Value], fields: &mut Map<String, Value>) {
    let mut listed = Vec::new();
    let mut roles: BTreeMap<String, u64> = BTreeMap::new();
    let mut unrecognized = 0u64;
    for message in messages {
        let Some(message) = message.as_object() else {
            unrecognized += 1;
            continue;
        };
        let role = token(message.get("role"));
        if role.is_none() {
            unrecognized += 1;
        }
        if let Some(role) = &role
            && (roles.len() < MAX_NAMES || roles.contains_key(role))
        {
            *roles.entry(role.clone()).or_default() += 1;
        }
        if listed.len() >= MAX_LISTED {
            continue;
        }
        let mut entry = Map::new();
        if let Some(role) = role {
            entry.insert("role".into(), Value::String(role));
        }
        put_token(&mut entry, "name", message.get("name"));
        put_id(&mut entry, "tool_call_id", message.get("tool_call_id"));
        match message.get("content") {
            Some(Value::String(text)) => {
                entry.insert("text_bytes".into(), text.len().into());
            }
            Some(Value::Array(parts)) => {
                let mut types = BTreeSet::new();
                let mut bytes = 0usize;
                for part in parts {
                    if let Some(kind) = token(part.get("type"))
                        && types.len() < MAX_NAMES
                    {
                        types.insert(kind);
                    }
                    if let Some(text) = part.get("text").and_then(Value::as_str) {
                        bytes += text.len();
                    }
                }
                entry.insert("content_parts".into(), parts.len().into());
                entry.insert("content_types".into(), names(&types));
                entry.insert("text_bytes".into(), bytes.into());
            }
            _ => {}
        }
        if let Some(Value::Array(calls)) = message.get("tool_calls") {
            entry.insert("tool_calls".into(), calls.len().into());
            let call_ids: Vec<Value> = calls
                .iter()
                .filter_map(|call| id(call.get("id")))
                .take(MAX_NAMES)
                .map(Value::String)
                .collect();
            entry.insert("tool_call_ids".into(), Value::Array(call_ids));
        }
        listed.push(Value::Object(entry));
    }
    fields.insert("message_count".into(), messages.len().into());
    if messages.len() > listed.len() {
        fields.insert("messages_truncated".into(), Value::Bool(true));
    }
    fields.insert("messages".into(), Value::Array(listed));
    let roles: Map<String, Value> = roles
        .into_iter()
        .map(|(role, count)| (role, count.into()))
        .collect();
    fields.insert("message_roles".into(), Value::Object(roles));
    fields.insert("unrecognized_messages".into(), unrecognized.into());
}
