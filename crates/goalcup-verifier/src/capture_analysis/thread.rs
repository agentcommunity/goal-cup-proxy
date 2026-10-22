//! Thread application WebSocket snapshots (`amp-thread-jsonrpc`), independently versioned.
//!
//! This is application/thread evidence, not the server's hidden provider request. Only complete
//! server `message_added.params.message` snapshots are interpreted. Assistant text/thinking
//! blocks yield `model_response`, model-facing `tool_use` blocks yield `tool_call`, and user
//! `tool_result` blocks yield context-visible `tool_result`: `run.result.output`, or a `run.result`
//! that is itself the text (a `Task` subagent's final answer, `create_file`), with the number of
//! the subagent's own tool uses a `Task` run lists under `run.progress[].tool_uses` as
//! `subagent_tool_uses`. A file edit's `run.result` is `{diff, lineRange}`: the diff the model
//! sees as bounded text `diff`, and a two-integer `lineRange` as `line_range` (version 3). Subagents share the parent's thread WebSocket: a snapshot whose
//! `params.parentToolUseId` (or `parentToolCallId`) names the parent's `Task` call carries it as
//! `parent_tool_use_id` on every fact, so a subagent's messages are told from the parent's
//! (version 2). Supplied `usage.model` and integer
//! usage counters are snapshots, never summed. All facts carry `scope: application_thread`,
//! message/block identity where safely supplied, and their own retained message's source span.
//!
//! No state joins separate messages: deltas, leases, executor notifications, user send/echo,
//! settings, RPC results and future forms are referenced unknowns. Repeated complete snapshots
//! stay separate occurrences. This decoder emits no human marker, provider request or outcome.
//! Environment/account/tool-definition metadata is never projected. Text and serialized tool
//! input use the shared bounded private-report helpers; raw retained bytes remain authoritative.
//!
//! The shared Tracker reassembles one WS message under the existing 1 MiB message, 8 MiB total
//! buffer and open-stream bounds. Loss, invalid audits and early ends cannot become snapshots.
//! Any valid redaction audit also prevents snapshot interpretation. The v5 WS payload contract
//! has no opcode/encoding fields: unsupported frames/encoding arrive as Gap/DecodeError from
//! capture, which the tracker conservatively retains as unknown rather than guessing content.
//!
//! Measured: one macOS Amp 0.0.1792209635-g24a9fe thread capture, with text/tool/usage snapshots,
//! a user context-visible tool result, JSON objects/array and literal ping/pong; and a second (22
//! Oct) with `Task` subagents: their snapshots carry `params.parentToolUseId`, and the `Task`
//! results a string `run.result` with `run.progress`. Thinking blocks,
//! batch arrays containing complete messages, malformed/lost data and limit paths are synthetic.
//! So is the `edit_file` result: its `{diff, lineRange}` shape comes from public readers of Amp's
//! thread files (the same `messages` block format), not from a measured capture.

use super::api::{DecoderFailure, DecoderSummary, DecoderV1, Observation, StreamOpen, kinds};
use super::tracker::{Protocol, SseEnd, Tracker};
use super::wire::{MAX_LISTED, Origin, Out, Span, id, names, put_id, put_text, put_token};
use goalcup_protocol::capture::{CaptureRecord, CaptureSource, StreamKey};
use serde_json::{Map, Value};

pub const DECODER_ID: &str = "amp-thread-jsonrpc";
pub const DECODER_VERSION: &str = "3";
pub const PROTOCOL_HINT: &str = "amp-thread-jsonrpc";

fn selects(source: CaptureSource, hint: Option<&str>) -> bool {
    hint == Some(PROTOCOL_HINT)
        && matches!(source, CaptureSource::WsClient | CaptureSource::WsServer)
}

#[derive(Debug, Default)]
pub struct ThreadDecoder {
    tracker: Tracker<Thread>,
}
impl ThreadDecoder {
    pub fn new() -> Self {
        Self::default()
    }
}
impl DecoderV1 for ThreadDecoder {
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

#[derive(Debug)]
struct Thread;
impl Protocol for Thread {
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
        if span.redacted {
            let mut fields = Map::new();
            fields.insert("kinds".into(), names(&span.kinds));
            out.unknown(origin, span, "redacted", fields);
            return;
        }
        if matches!(bytes, b"ping" | b"pong") {
            out.unknown(origin, span, "heartbeat", Map::new());
            return;
        }
        let value = match serde_json::from_slice::<Value>(bytes) {
            Ok(value) => value,
            Err(_) => {
                out.unknown(origin, span, "malformed", Map::new());
                return;
            }
        };
        if source == CaptureSource::WsClient {
            out.unknown(origin, span, "unsupported_client_message", Map::new());
            return;
        }
        match value {
            Value::Object(object) => Record {
                origin,
                span,
                out,
                array_index: None,
            }
            .object(&object),
            Value::Array(items) if !items.is_empty() => {
                for (index, item) in items.iter().take(MAX_LISTED).enumerate() {
                    let mut record = Record {
                        origin,
                        span,
                        out,
                        array_index: Some(index),
                    };
                    if let Some(object) = item.as_object() {
                        record.object(object)
                    } else {
                        record.unknown("unsupported_array_item", Map::new())
                    }
                }
                if items.len() > MAX_LISTED {
                    out.tally.incomplete = true;
                    let mut fields = Map::new();
                    fields.insert("items_omitted".into(), (items.len() - MAX_LISTED).into());
                    out.unknown(origin, span, "over_budget", fields);
                }
            }
            _ => out.unknown(origin, span, "unsupported_shape", Map::new()),
        }
    }
    // WS-only selection never supplies an SSE body. Trait hooks remain conservative.
    fn event(_: &mut (), _: &[u8], span: &Span, origin: &Origin<'_>, out: &mut Out<'_>) {
        out.unknown(origin, span, "unsupported_framing", Map::new());
    }
    fn lost(_: &mut (), reason: &str, span: &Span, origin: &Origin<'_>, out: &mut Out<'_>) {
        out.unknown(origin, span, reason, Map::new());
    }
    fn discarded(_: &mut (), _: &Span) {}
    fn stream_span(_: &(), history: &Span) -> Span {
        history.clone()
    }
    fn close(_: (), _: &SseEnd<'_>, _: &Origin<'_>, _: &mut Out<'_>) {}
}

struct Record<'a, 'b> {
    origin: &'a Origin<'a>,
    span: &'a Span,
    out: &'a mut Out<'b>,
    array_index: Option<usize>,
}
impl Record<'_, '_> {
    fn base(&self) -> Map<String, Value> {
        let mut fields = Map::new();
        fields.insert("scope".into(), "application_thread".into());
        if let Some(index) = self.array_index {
            fields.insert("array_index".into(), index.into());
        }
        fields
    }
    fn unknown(&mut self, reason: &str, extra: Map<String, Value>) {
        let mut fields = self.base();
        fields.extend(extra);
        self.out.unknown(self.origin, self.span, reason, fields);
    }
    fn observe(&mut self, kind: &str, supplied_id: Option<String>, fields: Map<String, Value>) {
        self.out
            .observe(self.origin, self.span, kind, supplied_id, fields);
    }
    fn object(&mut self, object: &Map<String, Value>) {
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
            || object.get("method").and_then(Value::as_str) != Some("message_added")
        {
            let mut extra = Map::new();
            put_token(&mut extra, "method", object.get("method"));
            put_token(&mut extra, "type", object.get("type"));
            self.unknown("unsupported_event", extra);
            return;
        }
        let params = object.get("params");
        let Some(message) = params
            .and_then(|v| v.get("message"))
            .and_then(Value::as_object)
        else {
            self.unknown("unsupported_message", Map::new());
            return;
        };
        // A subagent's snapshot names the parent's tool call it runs under.
        let parent = params.and_then(|p| {
            p.get("parentToolUseId")
                .or_else(|| p.get("parentToolCallId"))
        });
        let message_fields = |record: &Self| record.message_fields(message, parent);
        let role = message.get("role").and_then(Value::as_str);
        let assistant = role == Some("assistant");
        if !(assistant || role == Some("user")) {
            self.unknown("unsupported_role", Map::new());
            return;
        }
        if assistant
            && message
                .get("state")
                .and_then(|v| v.get("type"))
                .and_then(Value::as_str)
                != Some("complete")
        {
            self.unknown("incomplete_message", Map::new());
            return;
        }
        let Some(blocks) = message
            .get("content")
            .and_then(Value::as_array)
            .filter(|v| !v.is_empty())
        else {
            self.unknown("unsupported_content", Map::new());
            return;
        };
        for (index, block) in blocks.iter().take(MAX_LISTED).enumerate() {
            let mut fields = message_fields(self);
            fields.insert("block_index".into(), index.into());
            let Some(block) = block.as_object() else {
                self.unknown("unsupported_block", fields);
                continue;
            };
            put_id(&mut fields, "block_id", block.get("id"));
            put_token(&mut fields, "type", block.get("type"));
            match (assistant, block.get("type").and_then(Value::as_str)) {
                (true, Some("text" | "thinking")) => {
                    let name = if block.get("type").and_then(Value::as_str) == Some("thinking") {
                        "thinking"
                    } else {
                        "text"
                    };
                    if block.get("blockState").and_then(Value::as_str) != Some("complete") {
                        self.unknown("incomplete_block", fields);
                        continue;
                    }
                    let Some(text) = block.get(name).and_then(Value::as_str) else {
                        self.unknown("unsupported_block", fields);
                        continue;
                    };
                    put_text(&mut fields, name, text);
                    self.observe(kinds::MODEL_RESPONSE, id(message.get("messageId")), fields);
                }
                (true, Some("tool_use")) => {
                    if block.get("blockState").and_then(Value::as_str) != Some("complete")
                        || block.get("complete").and_then(Value::as_bool) != Some(true)
                    {
                        self.unknown("incomplete_block", fields);
                        continue;
                    }
                    let Some(input) = block.get("input").filter(|v| v.is_object()) else {
                        self.unknown("unsupported_tool_input", fields);
                        continue;
                    };
                    put_token(&mut fields, "name", block.get("name"));
                    put_id(&mut fields, "call_id", block.get("id"));
                    put_text(&mut fields, "input", &input.to_string());
                    self.observe(kinds::TOOL_CALL, id(block.get("id")), fields);
                }
                (false, Some("tool_result")) => {
                    let run = block.get("run");
                    let result = run.and_then(|v| v.get("result"));
                    let done =
                        run.and_then(|v| v.get("status")).and_then(Value::as_str) == Some("done");
                    // `{output, exitCode}` for a shell; the text itself for a `Task` or a file
                    // write; `{diff, lineRange}` for a file edit (version 3).
                    let output = result
                        .and_then(|v| v.get("output"))
                        .and_then(Value::as_str)
                        .or_else(|| result.and_then(Value::as_str));
                    let diff = result.and_then(|v| v.get("diff")).and_then(Value::as_str);
                    if !done || (output.is_none() && diff.is_none()) {
                        self.unknown("unsupported_tool_result", fields);
                        continue;
                    }
                    put_id(&mut fields, "call_id", block.get("toolUseID"));
                    fields.insert("status".into(), "done".into());
                    if let Some(output) = output {
                        put_text(&mut fields, "output", output);
                    } else if let Some(diff) = diff {
                        put_text(&mut fields, "diff", diff);
                        if let Some(range) = result
                            .and_then(|v| v.get("lineRange"))
                            .and_then(Value::as_array)
                            .filter(|r| r.len() == 2 && r.iter().all(Value::is_u64))
                        {
                            fields.insert("line_range".into(), Value::Array(range.clone()));
                        }
                    }
                    if let Some(progress) = run
                        .and_then(|v| v.get("progress"))
                        .and_then(Value::as_array)
                    {
                        let uses: usize = progress
                            .iter()
                            .filter_map(|p| p.get("tool_uses").and_then(Value::as_array))
                            .map(Vec::len)
                            .sum();
                        fields.insert("subagent_tool_uses".into(), uses.into());
                    }
                    if let Some(code) = result
                        .and_then(|v| v.get("exitCode"))
                        .filter(|v| v.is_i64() || v.is_u64())
                    {
                        fields.insert("exit_code".into(), code.clone());
                    }
                    self.observe(kinds::TOOL_RESULT, id(block.get("toolUseID")), fields);
                }
                _ => self.unknown("unsupported_block", fields),
            }
        }
        if blocks.len() > MAX_LISTED {
            self.out.tally.incomplete = true;
            let mut fields = message_fields(self);
            fields.insert("blocks_omitted".into(), (blocks.len() - MAX_LISTED).into());
            self.unknown("over_budget", fields);
        }
        if assistant && let Some(usage) = message.get("usage").and_then(Value::as_object) {
            let mut fields = message_fields(self);
            put_token(&mut fields, "model", usage.get("model"));
            if fields.contains_key("model") {
                self.observe(kinds::MODEL, id(message.get("messageId")), fields);
            }
            let mut fields = message_fields(self);
            for name in [
                "inputTokens",
                "outputTokens",
                "cacheCreationInputTokens",
                "cacheReadInputTokens",
                "totalInputTokens",
                "maxInputTokens",
            ] {
                if let Some(value) = usage.get(name).filter(|v| v.is_u64()) {
                    fields.insert(name.into(), value.clone());
                }
            }
            self.observe(kinds::USAGE, id(message.get("messageId")), fields);
        }
    }
    fn message_fields(
        &self,
        message: &Map<String, Value>,
        parent: Option<&Value>,
    ) -> Map<String, Value> {
        let mut fields = self.base();
        put_id(&mut fields, "message_id", message.get("messageId"));
        put_id(&mut fields, "thread_id", message.get("threadId"));
        put_token(&mut fields, "role", message.get("role"));
        put_id(&mut fields, "parent_tool_use_id", parent);
        fields
    }
}
