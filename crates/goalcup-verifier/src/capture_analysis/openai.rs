//! The bounded OpenAI Responses decoder (`openai-responses`).
//!
//! [`OpenAiDecoder`] implements [`DecoderV1`] over the ordered stream of every capture
//! record of a trace. It reads only retained, sanitized transport records; it runs no model,
//! opens no network connection and reads no clock.
//!
//! **Selection.** Only a stream whose `Open` carries the exact protocol hint
//! [`PROTOCOL_HINT`] is decoded, never because of its host. WebSocket messages (`ws_client`,
//! `ws_server`) are selected by source and hint alone: their `Open` carries no content type.
//! HTTP bodies are selected by source, hint and the observed
//! content type: a `wire_request` body as `application/json`, a `wire_response` body as
//! `text/event-stream` (SSE) or `application/json`. Any other selected body is counted as
//! unknown, never guessed at; an empty body (the WebSocket upgrade exchange) yields nothing.
//!
//! **Logical records.** A WebSocket message is one stream, so its record boundary is the
//! stream's `Open`/`End`. An HTTP JSON body is one record. An SSE body is framed into events at
//! blank lines, whatever segment boundaries its lines cross. A record is decoded only once it
//! is complete: a `Gap`, an `End` other than `complete`, a lost producer (a `Recovery` naming
//! it), a stream still open at `finish`, or an over-budget record becomes one `unknown`
//! observation with its reason, and no prefix of it is ever parsed. After a gap inside an SSE
//! body, bytes are discarded through the next blank line (the next framing boundary), so a
//! suffix is never taken for a new event. A stream-less `Gap` and every `Recovery` make the
//! summary incomplete even when no tracked record is affected: whole records of a lost epoch
//! may never have reached the chain. A record that fails to parse while a contributing
//! segment carries a redaction audit span is `unknown` with reason `redacted` and its audit
//! kinds; for an SSE event that is any audit span in a segment it touches.
//!
//! **Observations** (`kind`, then what `fields` holds; every observation also carries
//! `capture_source`, the typed `wire` identity from its `Open`, and `redaction_kinds` when a
//! contributing segment carried an audit span):
//! - `model_request`: one per client request (a `response.create` message or an HTTP JSON
//!   body): supplied `previous_response_id`, `generate`, `stream`, `store`,
//!   `reasoning_effort`, and the structure of `input` (item types, roles, supplied item IDs,
//!   text byte counts, never the text). Request content is context the harness sent; it is
//!   not a human input, a goal or an assistance event, and repeated requests stay separate.
//! - `model`: a supplied model identifier, `side` `request`, or `response` from
//!   `response.created`.
//! - `model_response`: `response.created` (response ID, status, supplied creation time), and
//!   each finished `message` or `reasoning` output item (role, phase, bounded text parts; for
//!   reasoning only the summary count and whether encrypted content was present).
//! - `tool_call`: a finished `custom_tool_call` or `function_call` output item: name,
//!   `call_id`, status and the bounded `input` or `arguments` text.
//! - `tool_result`: a `custom_tool_call_output` or `function_call_output` request input item:
//!   `call_id` and the bounded output text or parts.
//! - `usage`: the supplied integer token fields of one terminal response event, scope
//!   `provider_response`. Never summed, never defaulted: absent fields stay absent, and
//!   unrecognized usage members are named in `unrecognized_fields`.
//! - `outcome`: a terminal response event (`response.completed`, `.incomplete`, `.failed`),
//!   scope `provider_response`: supplied status, incomplete reason and provider timestamps.
//!   It is the provider's statement about one response: never task completion, never a
//!   transport close.
//! - `error`: the safe `code`/`type` tokens of `response.failed`'s error, of an `error` stream
//!   event, or of an HTTP JSON `error` body (with the recorded HTTP status). The message and
//!   any other text are never kept; a present code or type that is not a safe token is named
//!   in `withheld`.
//! - `unknown`: one per record that could not be interpreted, with its `reason`.
//!
//! Deltas, `*.added` events, part and text `*.done` events and `response.in_progress` are
//! recognized and produce nothing: their facts arrive complete in the finished item. Supplied
//! IDs stay in `supplied_id` or named fields; nothing joins observations by label, text or
//! message order, so an output item never inherits a response ID from another record.
//!
//! **Time.** `observed` spans the contributing retained segments, from the first segment's
//! `first` to the last segment's `last`, in their own clock: an interval, never a latency. A
//! provider-supplied time stays a separate `provider_*` field, exactly as supplied.
//!
//! **Measured forms.** The Codex CLI 0.153.0 WebSocket traffic of a live run (a warm-up
//! `generate: false` request, `previous_response_id` continuations carrying only new items,
//! `custom_tool_call` items, `response.completed` with usage) is measured; see
//! `tests/fixtures/m2-openai/`. `function_call` items, the HTTP SSE fallback,
//! `response.failed`/`.incomplete`, `error` events and HTTP JSON error bodies follow the
//! documented Responses shapes and are tested on synthetic bytes only.
//!
//! **Bounds.** At most [`MAX_EVENT_BYTES`] per logical record and [`MAX_BUFFERED_BYTES`] over
//! every open stream; at most `MAX_OPEN_STREAMS` tracked streams. Exceeding a bound makes that
//! record `unknown` (`over_budget`) and the summary incomplete; decoding resumes at the next
//! record boundary. Kept text is cut at [`MAX_TEXT_BYTES`], with its full length and a
//! `*_truncated` flag; the retained source stays available for reinterpretation.
//!
//! **Shared plumbing.** The segment assembler, the SSE framer and the bounded field helpers
//! live in `wire.rs`, shared with the Chat Completions decoder. They are transport framing,
//! not a provider parser. The stream tracking (open streams, framing choice, losses, ends) is
//! `tracker.rs`, also shared with the Chat Completions decoder; this file supplies only what a
//! Responses record means.

use super::api::{DecoderFailure, DecoderSummary, DecoderV1, Observation, StreamOpen, kinds};
use super::tracker::{Protocol, SseEnd, Tracker};
pub use super::wire::{MAX_BUFFERED_BYTES, MAX_EVENT_BYTES, MAX_TEXT_BYTES};
use super::wire::{
    MAX_LISTED, MAX_NAMES, Origin, Out, Span, id, names, parse_object, put_error, put_id,
    put_number, put_output, put_parts, put_text, put_token, put_usage, token,
};
use goalcup_protocol::capture::{CaptureRecord, CaptureSource, StreamKey};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// The decoder identity declared in its summary.
pub const DECODER_ID: &str = "openai-responses";
/// Incremented whenever the interpretation of any record changes.
pub const DECODER_VERSION: &str = "2";
/// The exact profile hint this decoder selects.
pub const PROTOCOL_HINT: &str = "openai-responses";

/// Whether this decoder reads a stream: the exact hint on a wire or WebSocket source, never
/// by host. The one selection rule; [`DecoderV1::accepts`] and the decoder's own guard both call it.
fn selects(source: CaptureSource, hint: Option<&str>) -> bool {
    hint == Some(PROTOCOL_HINT)
        && matches!(
            source,
            CaptureSource::WireRequest
                | CaptureSource::WireResponse
                | CaptureSource::WsClient
                | CaptureSource::WsServer
        )
}

/// The integer usage members and the integer-object members a Responses usage documents.
const USAGE_COUNTS: &[&str] = &["input_tokens", "output_tokens", "total_tokens"];
const USAGE_DETAILS: &[&str] = &["input_tokens_details", "output_tokens_details"];

/// Request input item types whose structure is recognized.
const INPUT_ITEM_TYPES: &[&str] = &[
    "message",
    "function_call",
    "function_call_output",
    "custom_tool_call",
    "custom_tool_call_output",
    "reasoning",
    "additional_tools",
];
/// Server events that are recognized and carry no fact the finished item does not.
const QUIET_EVENTS: &[&str] = &[
    "response.in_progress",
    "response.output_item.added",
    "response.content_part.added",
    "response.content_part.done",
    "response.output_text.delta",
    "response.output_text.done",
    "response.custom_tool_call_input.delta",
    "response.custom_tool_call_input.done",
    "response.function_call_arguments.delta",
    "response.function_call_arguments.done",
];

/// The bounded streaming decoder of OpenAI Responses traffic.
#[derive(Debug, Default)]
pub struct OpenAiDecoder {
    tracker: Tracker<Responses>,
}

impl OpenAiDecoder {
    pub fn new() -> Self {
        Self::default()
    }
}

impl DecoderV1 for OpenAiDecoder {
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
        self.tracker.push(record, emit)
    }

    fn push_recovery(
        &mut self,
        record: &CaptureRecord,
        lost_streams: &[StreamKey],
        emit: &mut dyn FnMut(Observation),
    ) -> Result<(), DecoderFailure> {
        self.tracker.push_recovery(record, lost_streams, emit)
    }

    fn finish(
        mut self: Box<Self>,
        emit: &mut dyn FnMut(Observation),
    ) -> Result<DecoderSummary, DecoderFailure> {
        self.tracker.finish(emit)
    }
}

/// What the Responses wire means, at the points the shared stream tracker asks.
#[derive(Debug)]
struct Responses;

impl Protocol for Responses {
    /// Each Responses SSE event stands alone: nothing is added up across events.
    type SseState = ();

    fn selects(source: CaptureSource, hint: Option<&str>) -> bool {
        selects(source, hint)
    }

    fn json(
        bytes: &[u8],
        source: CaptureSource,
        span: &Span,
        origin: &Origin<'_>,
        out: &mut Out<'_>,
    ) {
        let role = match source {
            CaptureSource::WsClient => Role::ClientMessage,
            CaptureSource::WireRequest => Role::HttpRequest,
            CaptureSource::WireResponse => Role::HttpJsonResponse,
            _ => Role::ServerEvent,
        };
        decode_record(bytes, role, span, origin, out);
    }

    fn event(_: &mut (), data: &[u8], span: &Span, origin: &Origin<'_>, out: &mut Out<'_>) {
        decode_record(data, Role::ServerEvent, span, origin, out);
    }

    fn lost(_: &mut (), reason: &str, span: &Span, origin: &Origin<'_>, out: &mut Out<'_>) {
        out.unknown(origin, span, reason, Map::new());
    }

    fn discarded(_: &mut (), _: &Span) {}

    fn stream_span(_: &(), history: &Span) -> Span {
        history.clone()
    }

    fn close(_: (), end: &SseEnd<'_>, origin: &Origin<'_>, out: &mut Out<'_>) {
        if end.leftover == Some("unterminated_event") && end.cut.is_none() {
            out.tally.incomplete = true;
        }
        if let Some(reason) = end.leftover {
            out.unknown(origin, end.pending, reason, Map::new());
        }
        // A pending record loss and the faithful stream cut are independent facts. A partial
        // event already uses the cut as its pending reason, so report that once.
        if let Some(reason) = end.cut
            && end.leftover != Some(reason)
            && end.history.first.is_some()
        {
            out.unknown(origin, end.history, reason, Map::new());
        }
    }
}

#[derive(Clone, Copy)]
enum Role {
    ClientMessage,
    HttpRequest,
    ServerEvent,
    HttpJsonResponse,
}

/// Decode one complete logical record. A parse failure is `redacted` when a contributing
/// segment carried an audit span, else `malformed`.
fn decode_record(bytes: &[u8], role: Role, span: &Span, origin: &Origin<'_>, out: &mut Out<'_>) {
    let Some(object) = parse_object(bytes, span, origin, out) else {
        return;
    };
    let mut record = Record {
        origin,
        span,
        out,
        object: &object,
    };
    match role {
        Role::ClientMessage => record.client_message(),
        Role::HttpRequest => record.request(None),
        Role::ServerEvent => record.server_event(),
        Role::HttpJsonResponse => record.http_json_response(),
    }
}

/// One parsed JSON object and where to report what it says.
struct Record<'r, 'o> {
    origin: &'r Origin<'r>,
    span: &'r Span,
    out: &'r mut Out<'o>,
    object: &'r Map<String, Value>,
}

impl<'r> Record<'r, '_> {
    fn observe(&mut self, kind: &str, supplied_id: Option<String>, fields: Map<String, Value>) {
        self.out
            .observe(self.origin, self.span, kind, supplied_id, fields);
    }

    fn unknown(&mut self, reason: &str, extra: Map<String, Value>) {
        self.out.unknown(self.origin, self.span, reason, extra);
    }

    /// An unrecognized or malformed event: its type is named only when it is a safe token.
    fn unknown_event(&mut self, reason: &str) {
        let mut extra = Map::new();
        put_token(&mut extra, "event_type", self.object.get("type"));
        self.unknown(reason, extra);
    }

    fn client_message(&mut self) {
        match self.object.get("type").and_then(Value::as_str) {
            Some(kind @ "response.create") => self.request(Some(kind)),
            Some(_) => self.unknown_event("unsupported_event"),
            None => self.unknown("missing_type", Map::new()),
        }
    }

    /// A Responses request: a `response.create` message or an HTTP JSON body.
    fn request(&mut self, request_type: Option<&str>) {
        let object = self.object;
        let mut fields = Map::new();
        if let Some(kind) = request_type {
            fields.insert("request_type".into(), Value::String(kind.into()));
        }
        put_id(
            &mut fields,
            "previous_response_id",
            object.get("previous_response_id"),
        );
        for name in ["generate", "stream", "store"] {
            if let Some(flag) = object.get(name).and_then(Value::as_bool) {
                fields.insert(name.into(), Value::Bool(flag));
            }
        }
        put_token(
            &mut fields,
            "reasoning_effort",
            object.get("reasoning").and_then(|r| r.get("effort")),
        );
        let items = match object.get("input") {
            Some(Value::Array(items)) => {
                describe_input(items, &mut fields);
                items.as_slice()
            }
            Some(Value::String(text)) => {
                fields.insert("input_kind".into(), Value::String("text".into()));
                fields.insert("input_text_bytes".into(), text.len().into());
                &[]
            }
            Some(_) => {
                fields.insert("input_kind".into(), Value::String("unsupported".into()));
                &[]
            }
            None => &[],
        };
        if let Some(Value::Array(tools)) = object.get("tools") {
            fields.insert("tools_count".into(), tools.len().into());
        }
        self.observe(kinds::MODEL_REQUEST, None, fields);
        if let Some(model) = token(object.get("model")) {
            let mut fields = Map::new();
            fields.insert("model".into(), Value::String(model));
            fields.insert("side".into(), Value::String("request".into()));
            self.observe(kinds::MODEL, None, fields);
        }
        for item in items {
            let Some(item) = item.as_object() else {
                continue;
            };
            let kind = item.get("type").and_then(Value::as_str);
            if let Some(kind @ ("custom_tool_call_output" | "function_call_output")) = kind {
                let mut fields = Map::new();
                fields.insert("item_type".into(), Value::String(kind.into()));
                fields.insert("origin".into(), Value::String("request_input".into()));
                put_id(&mut fields, "call_id", item.get("call_id"));
                put_output(&mut fields, item.get("output"));
                self.observe(kinds::TOOL_RESULT, id(item.get("id")), fields);
            }
        }
    }

    fn server_event(&mut self) {
        let Some(kind) = self.object.get("type").and_then(Value::as_str) else {
            self.unknown("missing_type", Map::new());
            return;
        };
        match kind {
            "response.created" => self.response_created(),
            "response.completed" | "response.incomplete" | "response.failed" => {
                self.response_terminal(kind)
            }
            "response.output_item.done" => self.item_done(),
            "error" => self.stream_error(),
            quiet if QUIET_EVENTS.contains(&quiet) => {}
            _ => self.unknown_event("unsupported_event"),
        }
    }

    fn response_object(&mut self) -> Option<&'r Map<String, Value>> {
        let object = self.object;
        let response = object.get("response").and_then(Value::as_object);
        if response.is_none() {
            self.unknown_event("unsupported_shape");
        }
        response
    }

    fn response_created(&mut self) {
        let Some(response) = self.response_object() else {
            return;
        };
        let response_id = id(response.get("id"));
        let mut fields = event_fields("response.created", self.object);
        put_id(&mut fields, "response_id", response.get("id"));
        put_token(&mut fields, "status", response.get("status"));
        put_id(
            &mut fields,
            "previous_response_id",
            response.get("previous_response_id"),
        );
        if let Some(flag) = response.get("generate").and_then(Value::as_bool) {
            fields.insert("generate".into(), Value::Bool(flag));
        }
        put_number(
            &mut fields,
            "provider_created_at",
            response.get("created_at"),
        );
        let model = token(response.get("model"));
        self.observe(kinds::MODEL_RESPONSE, response_id.clone(), fields);
        if let Some(model) = model {
            let mut fields = Map::new();
            fields.insert("model".into(), Value::String(model));
            fields.insert("side".into(), Value::String("response".into()));
            put_id(&mut fields, "response_id", response.get("id"));
            self.observe(kinds::MODEL, response_id, fields);
        }
    }

    fn response_terminal(&mut self, kind: &str) {
        let Some(response) = self.response_object() else {
            return;
        };
        let response_id = id(response.get("id"));
        let scoped = |object: &Map<String, Value>| {
            let mut fields = event_fields(kind, object);
            fields.insert("scope".into(), Value::String("provider_response".into()));
            put_id(&mut fields, "response_id", response.get("id"));
            fields
        };
        let mut fields = scoped(self.object);
        put_token(&mut fields, "status", response.get("status"));
        put_token(
            &mut fields,
            "incomplete_reason",
            response
                .get("incomplete_details")
                .and_then(|details| details.get("reason")),
        );
        if let Some(flag) = response.get("generate").and_then(Value::as_bool) {
            fields.insert("generate".into(), Value::Bool(flag));
        }
        put_number(
            &mut fields,
            "provider_created_at",
            response.get("created_at"),
        );
        put_number(
            &mut fields,
            "provider_completed_at",
            response.get("completed_at"),
        );
        if let Some(Value::Array(output)) = response.get("output") {
            fields.insert("output_item_count".into(), output.len().into());
        }
        let usage = response.get("usage").and_then(Value::as_object);
        let error = response.get("error").and_then(Value::as_object);
        let (usage_fields, error_fields) = (scoped(self.object), scoped(self.object));
        self.observe(kinds::OUTCOME, response_id.clone(), fields);
        if let Some(usage) = usage {
            let mut fields = usage_fields;
            put_usage(&mut fields, usage, USAGE_COUNTS, USAGE_DETAILS);
            self.observe(kinds::USAGE, response_id.clone(), fields);
        }
        if let Some(error) = error {
            let mut fields = error_fields;
            put_error(&mut fields, error.get("code"), error.get("type"));
            self.observe(kinds::ERROR, response_id, fields);
        }
    }

    fn item_done(&mut self) {
        let Some(item) = self.object.get("item").and_then(Value::as_object) else {
            self.unknown_event("unsupported_shape");
            return;
        };
        let mut fields = event_fields("response.output_item.done", self.object);
        let item_id = id(item.get("id"));
        match item.get("type").and_then(Value::as_str) {
            Some(kind @ ("custom_tool_call" | "function_call")) => {
                fields.insert("item_type".into(), Value::String(kind.into()));
                put_token(&mut fields, "name", item.get("name"));
                put_id(&mut fields, "call_id", item.get("call_id"));
                put_token(&mut fields, "status", item.get("status"));
                let field = if kind == "custom_tool_call" {
                    "input"
                } else {
                    "arguments"
                };
                if let Some(text) = item.get(field).and_then(Value::as_str) {
                    put_text(&mut fields, field, text);
                }
                self.observe(kinds::TOOL_CALL, item_id, fields);
            }
            Some("message") => {
                fields.insert("item_type".into(), Value::String("message".into()));
                put_token(&mut fields, "role", item.get("role"));
                put_token(&mut fields, "status", item.get("status"));
                put_token(&mut fields, "phase", item.get("phase"));
                if let Some(Value::Array(parts)) = item.get("content") {
                    put_parts(&mut fields, "content", parts);
                }
                self.observe(kinds::MODEL_RESPONSE, item_id, fields);
            }
            Some("reasoning") => {
                fields.insert("item_type".into(), Value::String("reasoning".into()));
                if let Some(Value::Array(summary)) = item.get("summary") {
                    fields.insert("summary_parts".into(), summary.len().into());
                }
                let encrypted = item
                    .get("encrypted_content")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.is_empty());
                fields.insert("encrypted_content".into(), Value::Bool(encrypted));
                self.observe(kinds::MODEL_RESPONSE, item_id, fields);
            }
            _ => {
                let mut extra = Map::new();
                put_token(&mut extra, "item_type", item.get("type"));
                self.unknown("unsupported_item", extra);
            }
        }
    }

    /// An `error` stream event: its code at the top level or inside a nested `error`.
    fn stream_error(&mut self) {
        let nested = self.object.get("error").and_then(Value::as_object);
        let code = self
            .object
            .get("code")
            .filter(|code| !code.is_null())
            .or_else(|| nested.and_then(|error| error.get("code")));
        let mut fields = event_fields("error", self.object);
        fields.insert("scope".into(), Value::String("stream_event".into()));
        put_error(
            &mut fields,
            code,
            nested.and_then(|error| error.get("type")),
        );
        self.observe(kinds::ERROR, None, fields);
    }

    /// An HTTP JSON response body: only the documented `error` body is interpreted.
    fn http_json_response(&mut self) {
        let Some(error) = self.object.get("error").and_then(Value::as_object) else {
            self.unknown("unsupported_body", Map::new());
            return;
        };
        let mut fields = Map::new();
        fields.insert("scope".into(), Value::String("http_response".into()));
        if let Some(status) = self.origin.status {
            fields.insert("http_status".into(), status.into());
        }
        put_error(&mut fields, error.get("code"), error.get("type"));
        self.observe(kinds::ERROR, None, fields);
    }
}

/// The event type plus its supplied `output_index` and `sequence_number`, when present.
fn event_fields(kind: &str, object: &Map<String, Value>) -> Map<String, Value> {
    let mut fields = Map::new();
    fields.insert("event".into(), Value::String(kind.into()));
    for name in ["output_index", "sequence_number"] {
        if let Some(number) = object.get(name).and_then(Value::as_u64) {
            fields.insert(name.into(), number.into());
        }
    }
    fields
}

/// The structure of a request's `input` array: item types, roles, supplied IDs and text
/// sizes, never the text itself.
fn describe_input(items: &[Value], fields: &mut Map<String, Value>) {
    let mut listed = Vec::new();
    let mut roles: BTreeMap<String, u64> = BTreeMap::new();
    let mut unrecognized = 0u64;
    for item in items {
        let Some(item) = item.as_object() else {
            unrecognized += 1;
            continue;
        };
        let role = token(item.get("role"));
        let kind = match item.get("type") {
            Some(kind) => token(Some(kind)),
            // The documented short form of a message item has a role and no type.
            None if role.is_some() => Some("message".to_string()),
            None => None,
        };
        if !kind
            .as_deref()
            .is_some_and(|kind| INPUT_ITEM_TYPES.contains(&kind))
        {
            unrecognized += 1;
        }
        if let Some(role) = &role
            && (roles.len() < MAX_NAMES || roles.contains_key(role))
        {
            *roles.entry(role.clone()).or_default() += 1;
        }
        if listed.len() < MAX_LISTED {
            let mut entry = Map::new();
            if let Some(kind) = kind {
                entry.insert("type".into(), Value::String(kind));
            }
            if let Some(role) = role {
                entry.insert("role".into(), Value::String(role));
            }
            put_id(&mut entry, "id", item.get("id"));
            match item.get("content") {
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
                Some(Value::String(text)) => {
                    entry.insert("text_bytes".into(), text.len().into());
                }
                _ => {}
            }
            listed.push(Value::Object(entry));
        }
    }
    fields.insert("input_item_count".into(), items.len().into());
    if items.len() > listed.len() {
        fields.insert("input_items_truncated".into(), Value::Bool(true));
    }
    fields.insert("input_items".into(), Value::Array(listed));
    let roles: Map<String, Value> = roles
        .into_iter()
        .map(|(role, count)| (role, count.into()))
        .collect();
    fields.insert("input_roles".into(), Value::Object(roles));
    fields.insert("unrecognized_input_items".into(), unrecognized.into());
}
