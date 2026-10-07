//! The bounded OpenAI Chat Completions decoder (`openai-chat-completions`).
//!
//! Every input is a canonical `CaptureRecord` built here with deterministic times, and every
//! byte of it is synthetic: it follows the public Chat Completions wire format (SSE `data:`
//! chunks, `[DONE]`, `choices[].delta`, `tool_calls` by `index`, `finish_reason`, `usage`, a
//! per-chunk `model`, the non-stream `chat.completion` body and the `error` body). There is no
//! sanitized real Hermes trace in git, so nothing here is "measured"; the oracles are the
//! literals this file wrote, never the decoder's own output. The decoder is driven through a
//! `DecodeSession` (`support::decode`), so routing is under test too.

mod support;

use goalcup_protocol::capture::{
    CaptureMeta, CaptureRecord, CaptureSource, CaptureTime, EndReason, GapReason, WireIdentity,
};
use goalcup_protocol::redaction::RedactionAuditSpan;
use goalcup_verifier::capture_analysis::api::{DecoderSummary, DecoderV1, Observation};
use goalcup_verifier::capture_analysis::chat::{
    ChatCompletionsDecoder, DECODER_ID, DECODER_VERSION, MAX_BUFFERED_BYTES, MAX_EVENT_BYTES,
    PROTOCOL_HINT,
};
use serde_json::{Value, json};
use support::wire::{WireSpec, facts, kind, parts};

const PRODUCER: &str = "0123456789abcdef0123456789abcdef";
const LOST_PRODUCER: &str = "fedcba9876543210fedcba9876543210";
const CONNECTION: &str = "5e1f0a2b3c4d5e6f708192a3b4c5d6e7";
const HINT: &str = "openai-chat-completions";
const MODEL: &str = "deepseek-v4-flash-0930";

// ---------------------------------------------------------------------------------------
// Constructors.

fn time(tick: u64) -> CaptureTime {
    CaptureTime {
        wall: format!("2026-10-05T00:00:00.{tick:09}Z"),
        mono_ns: tick,
        clock_id: PRODUCER.into(),
    }
}

/// One stream's recorder metadata. Defaults: the exact hint, the response of a streamed
/// completion (`text/event-stream`, HTTP 200).
#[derive(Clone)]
struct Spec {
    common: WireSpec,
}

fn spec(stream: u64, source: CaptureSource) -> Spec {
    let content_type = match source {
        CaptureSource::WireRequest => Some("application/json".to_string()),
        CaptureSource::WireResponse => Some("text/event-stream".to_string()),
        _ => None,
    };
    Spec {
        common: WireSpec::new(
            PRODUCER,
            stream,
            source,
            CaptureMeta {
                wire: Some(WireIdentity {
                    connection: CONNECTION.into(),
                    exchange: stream,
                    message_index: None,
                }),
                label: "POST /v1/chat/completions".into(),
                protocol_hint: Some(HINT.into()),
                method: (source == CaptureSource::WireRequest).then(|| "POST".to_string()),
                host: Some("api.deepseek.com".into()),
                path: Some("/v1/chat/completions".into()),
                status: (source == CaptureSource::WireResponse).then_some(200),
                content_type,
            },
            time,
        ),
    }
}

fn response() -> Spec {
    spec(1, CaptureSource::WireResponse)
}

impl Spec {
    fn content_type(mut self, content_type: Option<&str>) -> Self {
        self.common.meta.content_type = content_type.map(str::to_string);
        self
    }

    fn status(mut self, status: u16) -> Self {
        self.common.meta.status = Some(status);
        self
    }

    fn hint(mut self, hint: Option<&str>) -> Self {
        self.common.meta.protocol_hint = hint.map(str::to_string);
        self
    }

    fn open(&self) -> CaptureRecord {
        self.common.open()
    }

    fn end(&self, reason: EndReason) -> CaptureRecord {
        self.common.end(reason)
    }

    fn gap(&self) -> CaptureRecord {
        self.common.gap()
    }

    fn segment(
        &self,
        part: u64,
        offset: u64,
        bytes: &[u8],
        audit: Vec<RedactionAuditSpan>,
    ) -> CaptureRecord {
        self.common.segment(
            part,
            offset,
            bytes,
            (100 + 10 * part, 105 + 10 * part),
            audit,
        )
    }

    /// `bytes` cut at `splits` into parts 0.. An empty body has no segment, as the relay
    /// records it.
    fn segments(&self, bytes: &[u8], splits: &[usize]) -> Vec<CaptureRecord> {
        if bytes.is_empty() {
            return Vec::new();
        }
        let mut bounds = vec![0];
        bounds.extend_from_slice(splits);
        bounds.push(bytes.len());
        bounds
            .windows(2)
            .enumerate()
            .map(|(part, w)| self.segment(part as u64, w[0] as u64, &bytes[w[0]..w[1]], Vec::new()))
            .collect()
    }

    /// Open, the segments, End(Complete).
    fn records(&self, bytes: &[u8], splits: &[usize]) -> Vec<CaptureRecord> {
        let mut records = vec![self.open()];
        records.extend(self.segments(bytes, splits));
        records.push(self.end(EndReason::Complete));
        records
    }
}

fn chat() -> Box<dyn DecoderV1> {
    Box::new(ChatCompletionsDecoder::new())
}

fn decode(records: &[CaptureRecord]) -> (Vec<Observation>, DecoderSummary) {
    support::decode::decode_one(chat, records)
}

/// A streamed response of `bytes` cut at `splits`, decoded.
fn stream(bytes: &[u8], splits: &[usize]) -> (Vec<Observation>, DecoderSummary) {
    decode(&response().records(bytes, splits))
}

fn reasons(observations: &[Observation]) -> Vec<String> {
    kind(observations, "unknown")
        .iter()
        .map(|o| o.fields["reason"].as_str().unwrap().to_string())
        .collect()
}

// ---------------------------------------------------------------------------------------
// Wire builders (public Chat Completions shapes).

fn chunk(delta: Value, finish: Value) -> Value {
    json!({"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1_790_000_000,
        "model": MODEL, "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
}

fn text(content: &str) -> Value {
    chunk(json!({"content": content}), Value::Null)
}

fn finish(reason: &str) -> Value {
    chunk(json!({}), json!(reason))
}

fn usage_chunk(prompt: u64, completion: u64) -> Value {
    json!({"id": "chatcmpl-1", "object": "chat.completion.chunk", "model": MODEL,
        "choices": [], "usage": {"prompt_tokens": prompt, "completion_tokens": completion,
            "total_tokens": prompt + completion}})
}

fn tools(calls: Value) -> Value {
    chunk(json!({"tool_calls": calls}), Value::Null)
}

/// One `data:` event per value, a blank line after each.
fn sse(events: &[Value]) -> Vec<u8> {
    let mut body = Vec::new();
    for event in events {
        body.extend_from_slice(format!("data: {event}\n\n").as_bytes());
    }
    body
}

const DONE: &[u8] = b"data: [DONE]\n\n";

/// The typical stream: role, three text chunks, `stop`, a usage-only chunk, `[DONE]`.
fn typical() -> Vec<u8> {
    let mut body = sse(&[
        chunk(json!({"role": "assistant", "content": ""}), Value::Null),
        text("Hel"),
        text("lo, "),
        text("world"),
        finish("stop"),
        usage_chunk(9, 3),
    ]);
    body.extend_from_slice(DONE);
    body
}

/// The byte offsets after each event of a `data:` body (every event ends in a blank line).
fn event_ends(body: &[u8]) -> Vec<usize> {
    body.windows(2)
        .enumerate()
        .filter(|(_, w)| w == b"\n\n")
        .map(|(at, _)| at + 2)
        .collect()
}

// ---------------------------------------------------------------------------------------
// A streamed completion.

#[test]
fn a_typical_stream_decodes_content_finish_usage_and_model() {
    let body = typical();
    // One event per segment, so part numbers name events: 0 role, 1-3 text, 4 finish,
    // 5 usage, 6 [DONE].
    let splits: Vec<usize> = event_ends(&body).into_iter().rev().skip(1).rev().collect();
    let (observations, summary) = stream(&body, &splits);

    assert!(!summary.incomplete);
    assert_eq!(summary.unknown_records, 0);
    assert_eq!(summary.observations as usize, observations.len());

    let model = kind(&observations, "model");
    assert_eq!(
        model.len(),
        1,
        "named once, though every chunk carries it: {model:?}"
    );
    assert_eq!(model[0].fields["model"], MODEL);
    assert_eq!(model[0].fields["side"], "response");
    assert_eq!(model[0].supplied_id.as_deref(), Some("chatcmpl-1"));
    assert_eq!(parts(model[0]), (0, 0));

    let message = kind(&observations, "model_response");
    assert_eq!(message.len(), 1);
    let message = message[0];
    assert_eq!(message.fields["content"], "Hello, world");
    assert_eq!(message.fields["content_bytes"], 12);
    assert_eq!(message.fields["role"], "assistant");
    assert_eq!(message.fields["choice_index"], 0);
    assert_eq!(message.fields["delivery"], "stream");
    assert_eq!(message.fields["stream_complete"], true);
    assert_eq!(message.fields["chunks"], 5);
    assert!(message.fields.get("unreadable_events").is_none());
    assert_eq!(parts(message), (0, 4), "the chunks that touched the choice");
    assert_eq!(message.supplied_id.as_deref(), Some("chatcmpl-1"));

    let outcome = kind(&observations, "outcome");
    assert_eq!(outcome.len(), 1);
    assert_eq!(outcome[0].fields["finish_reason"], "stop");
    assert_eq!(outcome[0].fields["scope"], "provider_response");
    assert_eq!(outcome[0].fields["chunk_index"], 4);
    assert_eq!(parts(outcome[0]), (4, 4));

    let usage = kind(&observations, "usage");
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].fields["prompt_tokens"], 9);
    assert_eq!(usage[0].fields["completion_tokens"], 3);
    assert_eq!(usage[0].fields["total_tokens"], 12);
    assert_eq!(parts(usage[0]), (5, 5));
    assert!(usage[0].fields.get("unrecognized_fields").is_none());
}

#[test]
fn the_same_bytes_cut_at_every_offset_give_identical_facts() {
    let body = typical();
    let (whole, _) = stream(&body, &[]);
    let expected = facts(&whole);
    assert!(!expected.is_empty());
    for at in 1..body.len() {
        let (cut, summary) = stream(&body, &[at]);
        assert_eq!(facts(&cut), expected, "split at {at}");
        assert!(!summary.incomplete, "split at {at}");
    }
    // One byte per segment: every boundary at once.
    let all: Vec<usize> = (1..body.len()).collect();
    let (bytes, _) = stream(&body, &all);
    assert_eq!(facts(&bytes), expected);
}

/// A tiny deterministic generator: the property tests need no dependency and no clock.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn random_splits(rng: &mut Lcg, len: usize) -> Vec<usize> {
    let count = rng.below(12);
    let mut splits: Vec<usize> = (0..count).map(|_| 1 + rng.below(len - 1)).collect();
    splits.sort_unstable();
    splits.dedup();
    splits
}

#[test]
fn random_chunk_boundaries_never_change_what_a_stream_says() {
    // Tool calls, CRLF framing and comments make the framer work for its keep.
    let mut body = b": keep-alive\r\n\r\n".to_vec();
    for event in [
        chunk(json!({"role": "assistant", "content": null}), Value::Null),
        tools(json!([{"index": 0, "id": "call_a", "type": "function",
            "function": {"name": "read_file", "arguments": ""}}])),
        tools(json!([{"index": 0, "function": {"arguments": "{\"path\":"}}])),
        tools(json!([{"index": 0, "function": {"arguments": "\"a.txt\"}"}}])),
        finish("tool_calls"),
        usage_chunk(20, 7),
    ] {
        body.extend_from_slice(format!("data:{event}\r\n\r\n").as_bytes());
    }
    body.extend_from_slice(b"data: [DONE]\r\n\r\n");
    let (whole, _) = stream(&body, &[]);
    let expected = facts(&whole);
    assert_eq!(kind(&whole, "tool_call").len(), 1);
    let mut rng = Lcg(7);
    for _ in 0..300 {
        let splits = random_splits(&mut rng, body.len());
        let (cut, summary) = stream(&body, &splits);
        assert_eq!(facts(&cut), expected, "splits {splits:?}");
        assert!(!summary.incomplete);
    }
}

#[test]
fn event_framing_ignores_comments_crlf_and_multi_line_data() {
    // `data` lines of one event join with a newline; JSON allows it between members.
    let body = b": ping\n\ndata: {\"choices\":\ndata: [{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n\n";
    let (observations, summary) = stream(body, &[]);
    let message = kind(&observations, "model_response");
    assert_eq!(message[0].fields["content"], "hi");
    assert_eq!(summary.unknown_records, 0);
}

#[test]
fn a_model_that_changes_between_chunks_is_named_once_per_model() {
    let mut other = text("b");
    other["model"] = json!("deepseek-v4-flash-0931");
    let mut body = sse(&[text("a"), other, text("c")]);
    body.extend_from_slice(DONE);
    let (observations, _) = stream(&body, &[]);
    let models: Vec<_> = kind(&observations, "model")
        .iter()
        .map(|o| o.fields["model"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(models, [MODEL, "deepseek-v4-flash-0931"]);
    assert_eq!(
        kind(&observations, "model_response")[0].fields["content"],
        "abc"
    );
}

#[test]
fn choices_are_kept_apart_by_their_index() {
    let two = |index: u64, content: &str| {
        json!({"id": "chatcmpl-2", "model": MODEL,
            "choices": [{"index": index, "delta": {"content": content}, "finish_reason": null}]})
    };
    let mut body = sse(&[two(0, "A"), two(1, "X"), two(0, "B"), two(1, "Y")]);
    body.extend_from_slice(DONE);
    let (observations, _) = stream(&body, &[]);
    let messages = kind(&observations, "model_response");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].fields["choice_index"], 0);
    assert_eq!(messages[0].fields["content"], "AB");
    assert_eq!(messages[1].fields["choice_index"], 1);
    assert_eq!(messages[1].fields["content"], "XY");
}

// ---------------------------------------------------------------------------------------
// Tool calls.

fn tool_stream() -> Vec<u8> {
    let mut body = sse(&[
        chunk(json!({"role": "assistant", "content": null}), Value::Null),
        tools(json!([{"index": 0, "id": "call_a", "type": "function",
            "function": {"name": "read_file", "arguments": ""}}])),
        tools(json!([{"index": 1, "id": "call_b", "type": "function",
            "function": {"name": "write_file", "arguments": "{\"pa"}}])),
        tools(json!([{"index": 0, "function": {"arguments": "{\"path\":"}}])),
        tools(
            json!([{"index": 0, "function": {"arguments": "\"a.txt\"}"}},
            {"index": 1, "function": {"arguments": "th\":\"b\"}"}}]),
        ),
        finish("tool_calls"),
    ]);
    body.extend_from_slice(DONE);
    body
}

#[test]
fn tool_call_deltas_merge_by_index_and_keep_their_own_sources() {
    let body = tool_stream();
    let splits: Vec<usize> = event_ends(&body).into_iter().rev().skip(1).rev().collect();
    let (observations, summary) = stream(&body, &splits);
    assert!(!summary.incomplete);

    let calls = kind(&observations, "tool_call");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].fields["index"], 0);
    assert_eq!(calls[0].fields["call_id"], "call_a");
    assert_eq!(calls[0].supplied_id.as_deref(), Some("call_a"));
    assert_eq!(calls[0].fields["name"], "read_file");
    assert_eq!(calls[0].fields["type"], "function");
    assert_eq!(calls[0].fields["arguments"], "{\"path\":\"a.txt\"}");
    assert_eq!(calls[0].fields["arguments_bytes"], 16);
    assert_eq!(parts(calls[0]), (1, 4), "chunks 1, 3 and 4 carried index 0");
    assert_eq!(calls[1].fields["index"], 1);
    assert_eq!(calls[1].fields["call_id"], "call_b");
    assert_eq!(calls[1].fields["name"], "write_file");
    assert_eq!(calls[1].fields["arguments"], "{\"path\":\"b\"}");
    assert_eq!(parts(calls[1]), (2, 4));
    for call in &calls {
        assert_eq!(call.fields["stream_complete"], true);
    }

    // The message carries no text but its role, and the finish reason is its own outcome.
    let message = &kind(&observations, "model_response")[0];
    assert_eq!(message.fields["role"], "assistant");
    assert!(message.fields.get("content").is_none());
    assert_eq!(
        kind(&observations, "outcome")[0].fields["finish_reason"],
        "tool_calls"
    );
}

#[test]
fn a_tool_call_name_that_is_not_a_safe_token_is_withheld() {
    let mut body = sse(&[tools(json!([{"index": 0, "id": "call_a",
        "function": {"name": "rm -rf /home/user", "arguments": "{}"}}]))]);
    body.extend_from_slice(DONE);
    let (observations, _) = stream(&body, &[]);
    let call = &kind(&observations, "tool_call")[0];
    assert!(call.fields.get("name").is_none());
    assert_eq!(call.fields["withheld"], json!(["name"]));
}

// ---------------------------------------------------------------------------------------
// Usage and finish reasons.

#[test]
fn usage_is_per_snapshot_supplied_only_and_never_summed() {
    let first = json!({"id": "chatcmpl-1", "choices": [], "usage": {
        "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
        "prompt_cache_hit_tokens": 64, "prompt_cache_miss_tokens": 36,
        "completion_tokens_details": {"reasoning_tokens": 5},
        "vendor_extra": 7}});
    let second = json!({"id": "chatcmpl-1", "choices": [], "usage": {"completion_tokens": 25}});
    let mut body = sse(&[first, second]);
    body.extend_from_slice(DONE);
    let (observations, _) = stream(&body, &[]);
    let usage = kind(&observations, "usage");
    assert_eq!(usage.len(), 2, "one observation per snapshot");
    assert_eq!(usage[0].fields["prompt_tokens"], 100);
    assert_eq!(usage[0].fields["prompt_cache_hit_tokens"], 64);
    assert_eq!(usage[0].fields["prompt_cache_miss_tokens"], 36);
    assert_eq!(
        usage[0].fields["completion_tokens_details"],
        json!({"reasoning_tokens": 5})
    );
    assert_eq!(
        usage[0].fields["unrecognized_fields"],
        json!(["vendor_extra"])
    );
    // Absent stays absent: the second snapshot is not completed from the first or added to it.
    assert_eq!(usage[1].fields["completion_tokens"], 25);
    assert!(usage[1].fields.get("prompt_tokens").is_none());
    assert!(usage[1].fields.get("total_tokens").is_none());
    assert_eq!(usage[1].fields["chunk_index"], 1);
}

#[test]
fn a_null_usage_and_a_truncated_finish_are_taken_as_supplied() {
    let mut null_usage = text("x");
    null_usage["usage"] = Value::Null;
    let mut body = sse(&[null_usage, finish("length")]);
    body.extend_from_slice(DONE);
    let (observations, _) = stream(&body, &[]);
    assert!(kind(&observations, "usage").is_empty());
    let outcome = kind(&observations, "outcome");
    assert_eq!(outcome[0].fields["finish_reason"], "length");
    // A provider outcome is not a task outcome: nothing here says "success".
    assert!(
        !serde_json::to_string(&facts(&observations))
            .unwrap()
            .contains("success")
    );
}

// ---------------------------------------------------------------------------------------
// The non-stream body and error bodies.

#[test]
fn a_non_stream_completion_body_is_decoded_like_a_finished_stream() {
    let body = json!({"id": "chatcmpl-9", "object": "chat.completion", "created": 1_790_000_001,
        "model": "deepseek-v4-flash",
        "choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
            "role": "assistant", "content": null, "reasoning_content": "thinking hard",
            "tool_calls": [{"id": "call_z", "type": "function",
                "function": {"name": "run", "arguments": "{}"}}]}}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15,
            "prompt_tokens_details": {"cached_tokens": 4}}})
    .to_string();
    let spec = response().content_type(Some("application/json"));
    let (observations, summary) = decode(&spec.records(body.as_bytes(), &[17, 90]));
    assert!(!summary.incomplete);
    assert_eq!(summary.unknown_records, 0);

    let message = &kind(&observations, "model_response")[0];
    assert_eq!(message.fields["delivery"], "body");
    assert_eq!(message.fields["stream_complete"], true);
    assert_eq!(message.fields["role"], "assistant");
    assert_eq!(
        message.fields["reasoning_bytes"], 13,
        "only the size of the reasoning"
    );
    assert!(
        !serde_json::to_string(&facts(&observations))
            .unwrap()
            .contains("thinking"),
        "reasoning text is never kept"
    );
    let call = &kind(&observations, "tool_call")[0];
    assert_eq!(call.fields["call_id"], "call_z");
    assert_eq!(call.fields["name"], "run");
    assert_eq!(call.fields["arguments"], "{}");
    assert_eq!(
        kind(&observations, "outcome")[0].fields["finish_reason"],
        "tool_calls"
    );
    assert_eq!(
        kind(&observations, "outcome")[0].fields["provider_created"],
        1_790_000_001
    );
    let usage = &kind(&observations, "usage")[0];
    assert_eq!(usage.fields["total_tokens"], 15);
    assert_eq!(
        usage.fields["prompt_tokens_details"],
        json!({"cached_tokens": 4})
    );
    assert_eq!(
        kind(&observations, "model")[0].fields["model"],
        "deepseek-v4-flash"
    );
    // One record: every observation is sourced from all three parts.
    for o in &observations {
        assert_eq!(parts(o), (0, 2), "{}", o.kind);
    }
}

#[test]
fn an_error_body_keeps_safe_tokens_and_never_the_masked_key_echo() {
    let body = json!({"error": {
        "message": "Authentication Fails, Your api key: ****a1b2 is invalid",
        "type": "authentication_error", "param": null, "code": "invalid_request_error"}})
    .to_string();
    let spec = response()
        .content_type(Some("application/json"))
        .status(401);
    let (observations, summary) = decode(&spec.records(body.as_bytes(), &[]));
    assert_eq!(observations.len(), 1);
    let error = &observations[0];
    assert_eq!(error.kind, "error");
    assert_eq!(error.fields["scope"], "http_response");
    assert_eq!(error.fields["http_status"], 401);
    assert_eq!(error.fields["type"], "authentication_error");
    assert_eq!(error.fields["code"], "invalid_request_error");
    assert!(!summary.incomplete && summary.unknown_records == 0);
    let all = serde_json::to_string(&facts(&observations)).unwrap();
    for needle in [
        "api key",
        "****",
        "a1b2",
        "Authentication Fails",
        "invalid_request_error\"\"",
    ] {
        assert!(!all.contains(needle), "{needle} leaked: {all}");
    }
    assert!(all.contains("authentication_error"));
}

#[test]
fn an_error_object_in_a_stream_and_a_code_that_is_no_token() {
    let weird = json!({"error": {"message": "x", "type": "server_error", "code": "has space"}});
    let plain = json!({"error": {"message": "y", "type": "overloaded_error", "code": 529}});
    let mut body = sse(&[weird, plain]);
    body.extend_from_slice(DONE);
    let (observations, _) = stream(&body, &[]);
    let errors = kind(&observations, "error");
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].fields["scope"], "stream_event");
    assert_eq!(errors[0].fields["type"], "server_error");
    assert_eq!(errors[0].fields["withheld"], json!(["code"]));
    assert!(
        errors[1].fields.get("code").is_none(),
        "a numeric code is not a token"
    );
    assert_eq!(errors[1].fields["withheld"], json!(["code"]));
}

#[test]
fn a_json_body_that_is_not_a_completion_is_unknown() {
    let spec = response().content_type(Some("application/json"));
    let (observations, summary) = decode(&spec.records(br#"{"hello":"world"}"#, &[]));
    assert_eq!(reasons(&observations), ["unsupported_body"]);
    assert_eq!(summary.unknown_records, 1);
    let (observations, _) = decode(&spec.records(b"[1,2]", &[]));
    assert_eq!(reasons(&observations), ["unsupported_shape"]);
}

// ---------------------------------------------------------------------------------------
// `[DONE]` and what follows it.

#[test]
fn data_after_done_is_one_finding_and_never_an_observation() {
    let mut body = typical();
    body.extend_from_slice(b"data: ");
    body.extend_from_slice(text("GHOST").to_string().as_bytes());
    body.extend_from_slice(b"\n\ndata:\n\n: comment\n\ndata: [DONE]\n\ndata: not json\n\n");
    let ends = event_ends(&body);
    let (observations, summary) = stream(&body, &[]);

    // Everything up to [DONE] is as in the plain stream.
    let (plain, _) = stream(&typical(), &[]);
    let before: Vec<_> = facts(&observations)
        .into_iter()
        .filter(|(kind, _, _)| kind != "unknown")
        .collect();
    assert_eq!(before, facts(&plain));
    // After it: no observation carries the trailing text, and one finding counts the events
    // with data (the ghost chunk, a second [DONE], the non-JSON line; the empty `data:` and
    // the comment are not data).
    let all = serde_json::to_string(&facts(&observations)).unwrap();
    assert!(!all.contains("GHOST"));
    assert_eq!(reasons(&observations), ["data_after_done"]);
    let finding = &kind(&observations, "unknown")[0];
    assert_eq!(finding.fields["events"], 3);
    assert_eq!(
        finding.fields["bytes"],
        (text("GHOST").to_string().len() + "[DONE]".len() + "not json".len()) as u64
    );
    assert_eq!(finding.source.last_part, 0);
    assert_eq!(summary.unknown_records, 1);
    assert!(!summary.incomplete, "everything up to [DONE] is complete");
    assert_eq!(
        kind(&observations, "model_response")[0].fields["stream_complete"],
        true
    );
    assert!(ends.len() > 7);
}

#[test]
fn the_after_done_finding_survives_every_split_and_keeps_its_own_source() {
    let mut body = typical();
    let tail = body.len();
    body.extend_from_slice(b"data: {\"late\":true}\n\n");
    let (whole, _) = stream(&body, &[]);
    let expected = facts(&whole);
    for at in 1..body.len() {
        let (cut, _) = stream(&body, &[at]);
        assert_eq!(facts(&cut), expected, "split at {at}");
    }
    let (observations, _) = stream(&body, &[tail]);
    let finding = &kind(&observations, "unknown")[0];
    assert_eq!(
        finding.source.first_part, 1,
        "the late event is in the second part only"
    );
}

#[test]
fn a_stream_that_ends_cleanly_without_done_is_reported_and_still_decoded() {
    let body = sse(&[text("par"), text("tial")]);
    let (observations, summary) = stream(&body, &[]);
    let message = &kind(&observations, "model_response")[0];
    assert_eq!(message.fields["content"], "partial");
    assert_eq!(message.fields["stream_complete"], false);
    assert_eq!(reasons(&observations), ["missing_done"]);
    assert!(!summary.incomplete);
}

// ---------------------------------------------------------------------------------------
// Interrupted and cut streams (D45).

#[test]
fn an_interrupted_stream_is_reported_not_failed() {
    // Cut mid-event: the partial event is discarded, the chunks before it are decoded.
    let mut body = sse(&[text("one "), text("two")]);
    body.extend_from_slice(b"data: {\"choices\":[{\"delta\":{\"cont");
    let mut records = vec![response().open()];
    records.extend(response().segments(&body, &[]));
    records.push(response().end(EndReason::Interrupted));
    let (observations, summary) = decode(&records);
    let message = &kind(&observations, "model_response")[0];
    assert_eq!(message.fields["content"], "one two");
    assert_eq!(message.fields["stream_complete"], false);
    assert_eq!(
        message.fields["unreadable_events"], 1,
        "the partial event is a lost chunk"
    );
    let note = &kind(&observations, "unknown")[0];
    assert_eq!(note.fields["reason"], "interrupted");
    assert_eq!(note.fields["done_seen"], false);
    assert_eq!(note.fields["pending_event"], "partial");
    assert_eq!(reasons(&observations), ["interrupted"]);
    assert!(
        !serde_json::to_string(&facts(&observations))
            .unwrap()
            .contains("cont\"")
    );
    // D45: a faithful record of an interrupted run is not a degraded decode.
    assert!(!summary.incomplete);
    assert_eq!(summary.unknown_records, 1);
}

#[test]
fn a_stream_interrupted_after_done_is_complete_and_says_so() {
    // The Hermes reference row: finish_reason, usage and [DONE] all arrived, then the end
    // was recorded `interrupted`.
    let mut records = vec![response().open()];
    records.extend(response().segments(&typical(), &[]));
    records.push(response().end(EndReason::Interrupted));
    let (observations, summary) = decode(&records);
    assert_eq!(
        kind(&observations, "model_response")[0].fields["stream_complete"],
        true
    );
    let note = &kind(&observations, "unknown")[0];
    assert_eq!(note.fields["reason"], "interrupted");
    assert_eq!(note.fields["done_seen"], true);
    assert!(note.fields.get("pending_event").is_none());
    assert!(!summary.incomplete);
}

#[test]
fn a_decode_error_end_is_reported_the_same_way() {
    let mut records = vec![response().open()];
    records.extend(response().segments(&sse(&[text("ok")]), &[]));
    records.push(response().end(EndReason::DecodeError));
    let (observations, summary) = decode(&records);
    assert_eq!(reasons(&observations), ["decode_error"]);
    assert_eq!(
        kind(&observations, "model_response")[0].fields["content"],
        "ok"
    );
    assert!(!summary.incomplete);
}

#[test]
fn an_interrupted_json_body_is_unknown_and_never_parsed() {
    let body = br#"{"choices":[{"message":{"content":"half"#;
    let spec = response().content_type(Some("application/json"));
    let mut records = vec![spec.open()];
    records.extend(spec.segments(body, &[]));
    records.push(spec.end(EndReason::Interrupted));
    let (observations, summary) = decode(&records);
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].fields["reason"], "interrupted");
    assert!(!summary.incomplete);
}

#[test]
fn a_stream_still_open_at_finish_and_a_lost_producer_are_losses() {
    // No End at all.
    let mut records = vec![response().open()];
    records.extend(response().segments(&sse(&[text("a")]), &[]));
    let (observations, summary) = decode(&records);
    assert_eq!(reasons(&observations), ["unterminated"]);
    assert_eq!(
        kind(&observations, "model_response")[0].fields["stream_complete"],
        false
    );
    assert!(summary.incomplete);

    // ProducerClosed ends the selected stream at the record where it occurs.
    let mut records = vec![response().open()];
    records.extend(response().segments(&sse(&[text("a")]), &[]));
    records.push(CaptureRecord::ProducerClosed {
        producer: PRODUCER.into(),
        observed: time(400_000),
        final_seq: None,
    });
    let (observations, summary) = decode(&records);
    assert_eq!(reasons(&observations), ["unterminated"]);
    assert!(summary.incomplete);

    // A Recovery naming the producer ends the stream as lost.
    let mut records = vec![response().open()];
    records.extend(response().segments(&sse(&[text("a")]), &[]));
    records.push(CaptureRecord::Recovery {
        previous_producer: PRODUCER.into(),
        producer: LOST_PRODUCER.into(),
        observed: time(500_000),
        reason: GapReason::ProducerLost,
    });
    let (observations, summary) = decode(&records);
    assert_eq!(reasons(&observations), ["producer_lost"]);
    assert!(summary.incomplete);
}

#[test]
fn repeated_unrecorded_recoveries_close_chat_streams_in_recovery_order() {
    let first = response();
    let mut second = response();
    second.common.key.producer = LOST_PRODUCER.into();
    second.common.key.stream = 2;
    let mut records = vec![first.open(), first.segment(0, 0, b"data: {", Vec::new())];
    records.push(CaptureRecord::Recovery {
        previous_producer: "never-recorded-a".into(),
        producer: LOST_PRODUCER.into(),
        observed: time(200),
        reason: GapReason::ProducerLost,
    });
    records.push(second.open());
    records.push(second.segment(0, 0, b"data: {", Vec::new()));
    records.push(CaptureRecord::Recovery {
        previous_producer: "never-recorded-b".into(),
        producer: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        observed: time(300),
        reason: GapReason::ProducerLost,
    });

    let (observations, summary) = decode(&records);
    let findings = kind(&observations, "unknown");
    assert_eq!(
        findings
            .iter()
            .map(|finding| (
                finding.fields["reason"].as_str().unwrap(),
                finding.source.stream.clone(),
            ))
            .collect::<Vec<_>>(),
        [
            ("producer_lost", first.common.key),
            ("producer_lost", second.common.key),
        ]
    );
    assert!(summary.incomplete);
}

// ---------------------------------------------------------------------------------------
// Damage inside a stream.

#[test]
fn a_malformed_chunk_is_unknown_and_the_stream_says_it_lost_one() {
    let mut body = sse(&[text("a")]);
    body.extend_from_slice(b"data: {\"choices\": [oops\n\n");
    body.extend_from_slice(&sse(&[text("c")]));
    body.extend_from_slice(DONE);
    let (observations, summary) = stream(&body, &[]);
    assert_eq!(reasons(&observations), ["malformed"]);
    let message = &kind(&observations, "model_response")[0];
    assert_eq!(
        message.fields["content"], "a",
        "the text stays a prefix: nothing after the hole is appended"
    );
    assert_eq!(message.fields["content_truncated"], true);
    assert_eq!(message.fields["stream_complete"], false);
    assert_eq!(message.fields["unreadable_events"], 1);
    assert!(
        !summary.incomplete,
        "a bad chunk is an unknown, not a lost producer"
    );
    assert_eq!(summary.unknown_records, 1);
}

#[test]
fn a_chunk_of_no_known_shape_is_unknown_with_its_object_name() {
    let mut body = sse(&[json!({"object": "ping"}), text("a")]);
    body.extend_from_slice(DONE);
    let (observations, _) = stream(&body, &[]);
    let unknown = &kind(&observations, "unknown")[0];
    assert_eq!(unknown.fields["reason"], "unsupported_shape");
    assert_eq!(unknown.fields["object"], "ping");
}

#[test]
fn a_gap_inside_an_event_discards_through_the_next_blank_line() {
    let s = response();
    let first = sse(&[text("a")]);
    let middle = b"data: {\"choices\":[{\"delta\":{\"content\":\"LOST\"}}]}\n\n";
    let mut records = vec![s.open()];
    records.push(s.segment(0, 0, &first, Vec::new()));
    // Half of the middle event arrives, then a gap, then its tail and a good event.
    records.push(s.segment(1, first.len() as u64, &middle[..20], Vec::new()));
    records.push(s.gap());
    let mut rest = middle[20..].to_vec();
    rest.extend_from_slice(&sse(&[text("z")]));
    rest.extend_from_slice(DONE);
    records.push(s.segment(2, 0, &rest, Vec::new()));
    records.push(s.end(EndReason::Complete));
    let (observations, summary) = decode(&records);
    assert_eq!(reasons(&observations), ["gap"]);
    let message = &kind(&observations, "model_response")[0];
    assert_eq!(
        message.fields["content"], "a",
        "no suffix is taken for an event, and nothing is spliced after the hole"
    );
    assert_eq!(message.fields["content_truncated"], true);
    assert_eq!(message.fields["stream_complete"], false);
    assert!(
        !serde_json::to_string(&facts(&observations))
            .unwrap()
            .contains("LOST")
    );
    assert!(summary.incomplete);
}

#[test]
fn invalid_audit_discards_one_event_without_splicing_and_later_gap_is_visible() {
    let s = response();
    let before = sse(&[text("before")]);
    let secret = "raw-secret-bearing-value";
    let invalid = sse(&[text(secret)]);
    let after_invalid = sse(&[text("invalid-resync"), text("after-invalid")]);
    let after_gap = sse(&[text("gap-resync")]);
    let mut final_part = after_gap.clone();
    final_part.extend_from_slice(DONE);
    let mut offset = 0;
    let mut records = vec![s.open()];
    records.push(s.segment(0, offset, &before, Vec::new()));
    offset += before.len() as u64;
    records.push(s.segment(
        1,
        offset,
        &invalid,
        vec![RedactionAuditSpan {
            kind: "email".into(),
            start: offset,
            end: offset + 1,
        }],
    ));
    offset += invalid.len() as u64;
    records.push(s.segment(2, offset, &after_invalid, Vec::new()));
    offset += after_invalid.len() as u64;
    records.push(s.gap());
    records.push(s.segment(3, offset + 1, &final_part, Vec::new()));
    records.push(s.end(EndReason::Complete));

    let (observations, summary) = decode(&records);
    assert_eq!(reasons(&observations), ["invalid_redaction_audit", "gap"]);
    assert!(
        !serde_json::to_string(&observations)
            .unwrap()
            .contains(secret)
    );
    let message = &kind(&observations, "model_response")[0];
    assert_eq!(message.fields["content"], "before");
    assert_eq!(message.fields["content_truncated"], true);
    assert_eq!(message.fields["stream_complete"], false);
    assert_eq!(message.fields["unreadable_events"], 2);
    assert!(summary.incomplete);
}

#[test]
fn invalid_audit_and_real_gap_keep_the_real_cause_in_both_orders() {
    for (name, invalid_first) in [("invalid_then_gap", true), ("gap_then_invalid", false)] {
        let s = response();
        let invalid = sse(&[text("raw-secret-bearing-value")]);
        let invalid_record = s.segment(
            0,
            0,
            &invalid,
            vec![RedactionAuditSpan {
                kind: "email".into(),
                start: 0,
                end: 1,
            }],
        );

        let mut records = vec![s.open()];
        if invalid_first {
            records.push(invalid_record);
            records.push(s.gap());
        } else {
            records.push(s.gap());
            records.push(invalid_record);
        }
        let mut after = sse(&[finish("sacrificial_finish"), finish("surviving_finish")]);
        after.extend_from_slice(DONE);
        records.push(s.segment(2, invalid.len() as u64 + 1, &after, Vec::new()));
        records.push(s.end(EndReason::Complete));

        let (observations, summary) = decode(&records);
        assert_eq!(
            reasons(&observations),
            ["invalid_redaction_audit", "gap"],
            "{name}"
        );
        let finish_reasons: Vec<_> = kind(&observations, "outcome")
            .iter()
            .map(|observation| observation.fields["finish_reason"].as_str().unwrap())
            .collect();
        assert_eq!(finish_reasons, ["surviving_finish"], "{name}");
        assert_eq!(summary.unknown_records, 2, "{name}");
        assert!(summary.incomplete, "{name}");
    }
}

#[test]
fn pending_over_budget_survives_invalid_audit_resync_at_boundary_and_close() {
    for consume_boundary in [true, false] {
        let s = response();
        let oversized = format!("data: {}", "x".repeat(MAX_EVENT_BYTES + 1));
        let invalid = sse(&[text("invalid-audit-event")]);
        let invalid_offset = oversized.len() as u64;
        let mut records = vec![
            s.open(),
            s.segment(0, 0, oversized.as_bytes(), Vec::new()),
            s.segment(
                1,
                invalid_offset,
                &invalid,
                vec![RedactionAuditSpan {
                    kind: "email".into(),
                    start: invalid_offset,
                    end: invalid_offset + 1,
                }],
            ),
        ];
        if consume_boundary {
            let mut after = b"\n\n".to_vec();
            after.extend_from_slice(&sse(&[text("after-resync")]));
            after.extend_from_slice(DONE);
            records.push(s.segment(2, invalid_offset + invalid.len() as u64, &after, Vec::new()));
        }
        records.push(s.end(EndReason::Complete));

        let (observations, summary) = decode(&records);
        let got = reasons(&observations);
        assert_eq!(
            got.iter()
                .filter(|reason| reason.as_str() == "invalid_redaction_audit")
                .count(),
            1,
            "consume_boundary={consume_boundary}"
        );
        assert_eq!(
            got.iter()
                .filter(|reason| reason.as_str() == "over_budget")
                .count(),
            1,
            "consume_boundary={consume_boundary}"
        );
        assert!(
            got.iter().all(|reason| reason != "gap"),
            "consume_boundary={consume_boundary}: {got:?}"
        );
        assert_eq!(summary.unknown_records, got.len() as u64);
        assert!(summary.incomplete);
    }
}

#[derive(Clone, Copy, Debug)]
enum IndependentClose {
    Interrupted,
    DecodeError,
    ProducerLost,
    Unterminated,
}

impl IndependentClose {
    fn reason(self) -> &'static str {
        match self {
            Self::Interrupted => "interrupted",
            Self::DecodeError => "decode_error",
            Self::ProducerLost => "producer_lost",
            Self::Unterminated => "unterminated",
        }
    }

    fn finish(self, records: &mut Vec<CaptureRecord>, spec: &Spec) {
        match self {
            Self::Interrupted => records.push(spec.end(EndReason::Interrupted)),
            Self::DecodeError => records.push(spec.end(EndReason::DecodeError)),
            Self::ProducerLost => records.push(CaptureRecord::Recovery {
                previous_producer: PRODUCER.into(),
                producer: LOST_PRODUCER.into(),
                observed: time(500_000),
                reason: GapReason::ProducerLost,
            }),
            Self::Unterminated => {}
        }
    }
}

fn undecodable(spec: &Spec, part: u64, offset: u64) -> CaptureRecord {
    let mut record = spec.segment(part, offset, b"unreadable", Vec::new());
    let CaptureRecord::Segment { data_b64, .. } = &mut record else {
        unreachable!()
    };
    *data_b64 = "%".to_string();
    record
}

#[test]
fn pending_over_budget_precedes_later_sse_damage_and_close() {
    for damage in ["gap", "undecodable_segment"] {
        let s = response();
        let oversized = format!("data: {}", "x".repeat(MAX_EVENT_BYTES + 1));
        let damage_record = if damage == "gap" {
            s.gap()
        } else {
            undecodable(&s, 1, oversized.len() as u64)
        };
        let records = vec![
            s.open(),
            s.segment(0, 0, oversized.as_bytes(), Vec::new()),
            damage_record,
            s.end(EndReason::Interrupted),
        ];
        let (observations, summary) = decode(&records);
        assert_eq!(
            reasons(&observations),
            ["over_budget", damage, "interrupted"]
        );
        assert_eq!(summary.unknown_records, 3);
        assert!(summary.incomplete);
    }

    let s = response();
    let oversized = format!("data: {}", "x".repeat(MAX_EVENT_BYTES + 1));
    let (observations, summary) = decode(&[
        s.open(),
        s.segment(0, 0, oversized.as_bytes(), Vec::new()),
        s.gap(),
        undecodable(&s, 1, oversized.len() as u64),
        s.end(EndReason::Interrupted),
    ]);
    assert_eq!(
        reasons(&observations),
        ["over_budget", "gap", "undecodable_segment", "interrupted"]
    );
    assert_eq!(summary.unknown_records, 4);
    assert!(summary.incomplete);
}

#[test]
fn repeated_identical_pending_sse_damage_is_deduplicated() {
    let s = response();
    let (observations, summary) = decode(&[
        s.open(),
        s.segment(0, 0, b"data: {", Vec::new()),
        s.gap(),
        s.gap(),
        s.segment(1, 10, b"\n\n", Vec::new()),
        s.end(EndReason::Interrupted),
    ]);
    assert_eq!(reasons(&observations), ["gap", "interrupted"]);
    assert_eq!(summary.unknown_records, 2);
    assert!(summary.incomplete);
}

#[test]
fn pending_over_budget_and_faithful_cut_are_independent_findings() {
    for (end_reason, cut) in [
        (EndReason::Interrupted, "interrupted"),
        (EndReason::DecodeError, "decode_error"),
    ] {
        let s = response();
        let oversized = format!("data: {}", "x".repeat(MAX_EVENT_BYTES + 1));
        let (observations, summary) = decode(&[
            s.open(),
            s.segment(0, 0, oversized.as_bytes(), Vec::new()),
            s.end(end_reason),
        ]);
        assert_eq!(reasons(&observations), ["over_budget", cut]);
        assert_eq!(summary.unknown_records, 2);
        assert!(summary.incomplete);
    }
}

#[test]
fn complete_end_with_pending_event_is_loss_and_releases_for_later_stream() {
    let first = response();
    let second = spec(2, CaptureSource::WireResponse);
    let mut sentinel = sse(&[text("after-partial")]);
    sentinel.extend_from_slice(DONE);
    let (observations, summary) = decode(&[
        first.open(),
        first.segment(0, 0, b"data: {", Vec::new()),
        first.end(EndReason::Complete),
        second.open(),
        second.segment(0, 0, &sentinel, Vec::new()),
        second.end(EndReason::Complete),
    ]);
    assert_eq!(reasons(&observations), ["unterminated_event"]);
    let responses = kind(&observations, "model_response");
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].fields["content"], "after-partial");
    assert_eq!(summary.unknown_records, 1);
    assert!(summary.incomplete);
}

#[test]
fn pending_event_close_releases_shared_budget_under_cap_pressure() {
    for (end_reason, close) in [
        (EndReason::Complete, "unterminated_event"),
        (EndReason::Interrupted, "interrupted"),
    ] {
        let first = response();
        let partial = format!("data: {}", "x".repeat(900_000));
        let later: Vec<_> = (10..18)
            .map(|stream| spec(stream, CaptureSource::WireRequest))
            .collect();
        let mut records = vec![
            first.open(),
            first.segment(0, 0, partial.as_bytes(), Vec::new()),
            first.end(end_reason),
        ];

        let request_prefix = br#"{"model":"deepseek-v4-flash","messages":[]}"#;
        let mut request = request_prefix.to_vec();
        request.resize(1_000_000, b' ');
        for stream in &later {
            records.push(stream.open());
            records.push(stream.segment(0, 0, &request, Vec::new()));
        }
        for stream in &later {
            records.push(stream.end(EndReason::Complete));
        }

        let (observations, _) = decode(&records);
        assert_eq!(reasons(&observations), [close], "{end_reason:?}");
        assert_eq!(
            kind(&observations, "model_request").len(),
            later.len(),
            "released capacity serves every later request after {end_reason:?}"
        );
    }
}

#[test]
fn json_loss_precedes_every_independent_non_complete_close() {
    for close in [
        IndependentClose::Interrupted,
        IndependentClose::DecodeError,
        IndependentClose::ProducerLost,
        IndependentClose::Unterminated,
    ] {
        let s = response().content_type(Some("application/json"));
        let invalid = s.segment(
            0,
            0,
            b"{}",
            vec![RedactionAuditSpan {
                kind: "email".into(),
                start: 0,
                end: 1,
            }],
        );
        let cases: Vec<(&str, Vec<CaptureRecord>)> = vec![
            ("gap", vec![s.segment(0, 0, b"{", Vec::new()), s.gap()]),
            (
                "over_budget",
                vec![s.segment(0, 0, &vec![b'x'; MAX_EVENT_BYTES + 1], Vec::new())],
            ),
            ("undecodable_segment", vec![undecodable(&s, 0, 0)]),
            ("invalid_redaction_audit", vec![invalid]),
        ];
        for (loss, mut body) in cases {
            let mut records = vec![s.open()];
            records.append(&mut body);
            close.finish(&mut records, &s);
            let (observations, summary) = decode(&records);
            assert_eq!(
                reasons(&observations),
                [loss, close.reason()],
                "{loss}, {close:?}"
            );
            assert_eq!(summary.unknown_records, 2);
            assert!(summary.incomplete);
        }
    }
}

#[test]
fn json_preserves_distinct_causes_in_first_seen_order_before_terminal() {
    for case in [
        "over_then_gap",
        "over_then_undecodable",
        "over_then_invalid_audit",
        "over_gap_undecodable",
        "undecodable_then_gap",
        "repeated_gap",
    ] {
        let s = response().content_type(Some("application/json"));
        let oversized = vec![b'x'; MAX_EVENT_BYTES + 1];
        let mut records = vec![s.open()];
        let expected = match case {
            "over_then_gap" => {
                records.push(s.segment(0, 0, &oversized, Vec::new()));
                records.push(s.gap());
                vec!["over_budget", "gap", "interrupted"]
            }
            "over_then_undecodable" => {
                records.push(s.segment(0, 0, &oversized, Vec::new()));
                records.push(undecodable(&s, 1, oversized.len() as u64));
                vec!["over_budget", "undecodable_segment", "interrupted"]
            }
            "over_then_invalid_audit" => {
                records.push(s.segment(0, 0, &oversized, Vec::new()));
                let offset = oversized.len() as u64;
                records.push(s.segment(
                    1,
                    offset,
                    b"{}",
                    vec![RedactionAuditSpan {
                        kind: "email".into(),
                        start: offset,
                        end: offset + 1,
                    }],
                ));
                vec!["invalid_redaction_audit", "over_budget", "interrupted"]
            }
            "over_gap_undecodable" => {
                records.push(s.segment(0, 0, &oversized, Vec::new()));
                records.push(s.gap());
                records.push(undecodable(&s, 1, oversized.len() as u64 + 1));
                vec!["over_budget", "gap", "undecodable_segment", "interrupted"]
            }
            "undecodable_then_gap" => {
                records.push(undecodable(&s, 0, 0));
                records.push(s.gap());
                vec!["undecodable_segment", "gap", "interrupted"]
            }
            "repeated_gap" => {
                records.push(s.segment(0, 0, b"{", Vec::new()));
                records.push(s.gap());
                records.push(s.gap());
                vec!["gap", "interrupted"]
            }
            _ => unreachable!(),
        };
        records.push(s.end(EndReason::Interrupted));

        let (observations, summary) = decode(&records);
        let got = reasons(&observations);
        assert_eq!(got, expected, "{case}");
        assert_eq!(summary.unknown_records, expected.len() as u64, "{case}");
        assert!(summary.incomplete, "{case}");
    }
}

#[test]
fn unsupported_whole_message_preserves_distinct_damage_before_terminal() {
    let s = response().content_type(Some("text/plain"));
    let (observations, summary) = decode(&[
        s.open(),
        s.segment(0, 0, b"x", Vec::new()),
        s.gap(),
        undecodable(&s, 1, 2),
        s.end(EndReason::Interrupted),
    ]);
    assert_eq!(
        reasons(&observations),
        [
            "gap",
            "undecodable_segment",
            "unsupported_content_type",
            "interrupted"
        ]
    );
    assert_eq!(summary.unknown_records, 4);
    assert!(summary.incomplete);

    let invalid = s.segment(
        1,
        2,
        b"secret",
        vec![RedactionAuditSpan {
            kind: "email".into(),
            start: 2,
            end: 3,
        }],
    );
    let (observations, _) = decode(&[
        s.open(),
        s.segment(0, 0, b"x", Vec::new()),
        s.gap(),
        invalid,
        s.end(EndReason::Interrupted),
    ]);
    assert_eq!(
        reasons(&observations),
        [
            "invalid_redaction_audit",
            "gap",
            "unsupported_content_type",
            "interrupted"
        ]
    );
}

#[test]
fn pending_real_gap_and_faithful_cut_are_both_visible_at_close() {
    for (name, invalid_first) in [("invalid_then_gap", true), ("gap_then_invalid", false)] {
        for (end_reason, cut) in [
            (EndReason::Interrupted, "interrupted"),
            (EndReason::DecodeError, "decode_error"),
        ] {
            let s = response();
            let before = sse(&[text("before")]);
            let invalid = sse(&[text("raw-secret-bearing-value")]);
            let invalid_record = s.segment(
                1,
                before.len() as u64,
                &invalid,
                vec![RedactionAuditSpan {
                    kind: "email".into(),
                    start: before.len() as u64,
                    end: before.len() as u64 + 1,
                }],
            );
            let mut records = vec![s.open(), s.segment(0, 0, &before, Vec::new())];
            if invalid_first {
                records.push(invalid_record);
                records.push(s.gap());
            } else {
                records.push(s.gap());
                records.push(invalid_record);
            }
            records.push(s.end(end_reason));

            let (observations, summary) = decode(&records);
            assert_eq!(
                reasons(&observations),
                ["invalid_redaction_audit", "gap", cut],
                "{name}, {end_reason:?}"
            );
            let message = &kind(&observations, "model_response")[0];
            assert_eq!(message.fields["content"], "before");
            assert_eq!(message.fields["content_truncated"], true);
            assert_eq!(message.fields["stream_complete"], false);
            assert_eq!(summary.unknown_records, 3, "{name}, {end_reason:?}");
            assert!(summary.incomplete, "{name}, {end_reason:?}");
        }
    }
}

#[test]
fn complete_during_invalid_audit_resync_finalizes_prefix_and_releases_shared_budget() {
    let body_for = |prefix: &str, choices: u64| {
        let events: Vec<_> = (0..choices)
            .flat_map(|choice| {
                (0..64u64).map(move |call| {
                    json!({"id": format!("chatcmpl-{prefix}"), "model": MODEL,
                        "choices": [{"index": choice, "finish_reason": null,
                            "delta": {"tool_calls": [{"index": call,
                                "id": format!("{prefix}-{choice}-{call}"),
                                "function": {"name": "f",
                                    "arguments": "a".repeat(15 * 1024)}}]}}]})
                })
            })
            .collect();
        sse(&events)
    };
    let held_body = body_for("held", 1);
    let first = spec(1, CaptureSource::WireResponse);
    let invalid = sse(&[text("raw-secret-bearing-value")]);
    let mut records = vec![
        first.open(),
        first.segment(0, 0, &held_body, Vec::new()),
        first.segment(
            1,
            held_body.len() as u64,
            &invalid,
            vec![RedactionAuditSpan {
                kind: "email".into(),
                start: held_body.len() as u64,
                end: held_body.len() as u64 + 1,
            }],
        ),
        first.end(EndReason::Complete),
    ];

    // The later stream keeps 7.5 MiB. A leaked 960 KiB prefix would push it past the shared
    // 8 MiB bound, so every full argument below proves the first stream released its budget.
    let later = spec(2, CaptureSource::WireResponse);
    let mut later_body = body_for("later", 8);
    later_body.extend_from_slice(DONE);
    records.push(later.open());
    records.push(later.segment(0, 0, &later_body, Vec::new()));
    records.push(later.end(EndReason::Complete));

    let (observations, summary) = decode(&records);
    let first_calls: Vec<_> = kind(&observations, "tool_call")
        .into_iter()
        .filter(|observation| observation.source.stream.stream == 1)
        .collect();
    assert_eq!(first_calls.len(), 64, "the valid prefix is finalized");
    assert!(
        first_calls
            .iter()
            .all(|observation| observation.fields["arguments_truncated"] == true)
    );
    let later_calls: Vec<_> = kind(&observations, "tool_call")
        .into_iter()
        .filter(|observation| observation.source.stream.stream == 2)
        .collect();
    assert_eq!(
        later_calls.len(),
        8 * 64,
        "the released budget serves a later stream"
    );
    assert!(later_calls.iter().all(|observation| {
        observation.fields["arguments"].as_str().unwrap().len() == 15 * 1024
            && observation.fields.get("arguments_truncated").is_none()
    }));
    let invalid_count = kind(&observations, "unknown")
        .iter()
        .filter(|observation| observation.fields["reason"] == "invalid_redaction_audit")
        .count();
    assert_eq!(
        invalid_count, 1,
        "the fixed audit diagnostic is not duplicated"
    );
    assert!(summary.incomplete);
}

#[test]
fn a_gap_in_a_json_body_marks_it_unknown_and_incomplete() {
    let s = response().content_type(Some("application/json"));
    let records = vec![
        s.open(),
        s.segment(0, 0, b"{\"choices\":[", Vec::new()),
        s.gap(),
        s.segment(1, 99, b"]}", Vec::new()),
        s.end(EndReason::Complete),
    ];
    let (observations, summary) = decode(&records);
    assert_eq!(reasons(&observations), ["gap"]);
    assert!(summary.incomplete);
}

#[test]
fn a_redacted_segment_that_breaks_the_parse_is_unknown_not_malformed() {
    let broken = b"data: {\"choices\":[{\"delta\":{\"content\":\"mail <redacted:email>\n\n";
    let marker_at = broken.len() - 2 - "<redacted:email>".len();
    let audit = vec![RedactionAuditSpan {
        kind: "email".into(),
        start: marker_at as u64,
        end: marker_at as u64 + 60,
    }];
    let s = response();
    let mut audited = s.segment(0, 0, broken, audit);
    let CaptureRecord::Segment { source_len, .. } = &mut audited else {
        unreachable!()
    };
    *source_len = marker_at as u64 + 60 + 2;
    let (observations, _) = decode(&[s.open(), audited, s.end(EndReason::Complete)]);
    assert_eq!(reasons(&observations), ["redacted", "missing_done"]);
    assert_eq!(
        kind(&observations, "unknown")[0].fields["kinds"],
        json!(["email"])
    );
    // The same bytes with no audit span are malformed.
    let (observations, _) = stream(broken, &[]);
    assert_eq!(reasons(&observations), ["malformed", "missing_done"]);

    // A redaction that leaves valid JSON is decoded, and says so.
    let intact = b"data: {\"choices\":[{\"delta\":{\"content\":\"mail <redacted:email> ok\"}}]}\n\ndata: [DONE]\n\n";
    let marker_at = intact
        .windows("<redacted:email>".len())
        .position(|window| window == b"<redacted:email>")
        .unwrap();
    let audit = vec![RedactionAuditSpan {
        kind: "email".into(),
        start: marker_at as u64,
        end: (marker_at + "<redacted:email>".len()) as u64,
    }];
    let (observations, _) = decode(&[
        s.open(),
        s.segment(0, 0, intact, audit),
        s.end(EndReason::Complete),
    ]);
    let message = &kind(&observations, "model_response")[0];
    assert_eq!(message.fields["redaction_kinds"], json!(["email"]));
    assert_eq!(message.fields["content"], "mail <redacted:email> ok");
}

#[test]
fn separate_length_changing_audits_do_not_taint_the_clean_event_between_them() {
    let chunk = |index: u64, content: &str| {
        json!({"id": "chatcmpl-audit", "object": "chat.completion.chunk",
            "created": 1_790_000_000, "model": MODEL, "choices": [{
                "index": index, "delta": {"role": "assistant", "content": content},
                "finish_reason": "stop"}]})
    };
    let marker_a = "<redacted:first>";
    let marker_b = "<redacted:second>";
    let mut body = sse(&[
        chunk(0, &format!("left {marker_a}")),
        chunk(1, "clean middle"),
        chunk(2, &format!("right {marker_b}")),
    ]);
    body.extend_from_slice(DONE);

    let retained_a = body
        .windows(marker_a.len())
        .position(|window| window == marker_a.as_bytes())
        .unwrap();
    let retained_b = body
        .windows(marker_b.len())
        .position(|window| window == marker_b.as_bytes())
        .unwrap();
    let source_a_len = 3usize;
    let source_b_len = 37usize;
    let source_b_start = retained_b - (marker_a.len() - source_a_len);
    let source_len = body.len() - marker_a.len() - marker_b.len() + source_a_len + source_b_len;
    assert_ne!(source_len, body.len());

    let s = response();
    let mut segment = s.segment(
        0,
        0,
        &body,
        vec![
            RedactionAuditSpan {
                kind: "first".into(),
                start: retained_a as u64,
                end: (retained_a + source_a_len) as u64,
            },
            RedactionAuditSpan {
                kind: "second".into(),
                start: source_b_start as u64,
                end: (source_b_start + source_b_len) as u64,
            },
        ],
    );
    let CaptureRecord::Segment {
        source_len: claimed,
        ..
    } = &mut segment
    else {
        unreachable!()
    };
    *claimed = source_len as u64;

    let (observations, summary) = decode(&[s.open(), segment, s.end(EndReason::Complete)]);
    let responses = kind(&observations, "model_response");
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0].fields["redaction_kinds"], json!(["first"]));
    assert!(responses[1].fields.get("redaction_kinds").is_none());
    assert_eq!(responses[2].fields["redaction_kinds"], json!(["second"]));
    assert!(!summary.incomplete);
}

// ---------------------------------------------------------------------------------------
// Bounds.

fn padded(size: usize) -> Value {
    text(&"x".repeat(size))
}

#[test]
fn an_oversized_event_is_over_budget_and_decoding_resumes_after_it() {
    let mut body = sse(&[text("a")]);
    body.extend_from_slice(&sse(&[padded(MAX_EVENT_BYTES + 10)]));
    body.extend_from_slice(&sse(&[text("b")]));
    body.extend_from_slice(DONE);
    let splits: Vec<usize> = (1..body.len() / 65_536 + 1).map(|n| n * 65_536).collect();
    let (observations, summary) = stream(&body, &splits);
    assert_eq!(reasons(&observations), ["over_budget"]);
    let over = &kind(&observations, "unknown")[0];
    assert_eq!(over.fields["limit_bytes"], MAX_EVENT_BYTES);
    let message = &kind(&observations, "model_response")[0];
    assert_eq!(message.fields["content"], "a");
    assert_eq!(message.fields["content_truncated"], true);
    assert_eq!(message.fields["stream_complete"], false);
    assert!(summary.incomplete);
}

#[test]
fn kept_text_is_cut_with_its_full_length_and_stays_a_prefix() {
    let mut events = Vec::new();
    for n in 0..10 {
        events.push(text(&format!("{n}{}", "é".repeat(2_000))));
    }
    events.push(tools(json!([{"index": 0, "id": "call_a",
        "function": {"name": "w", "arguments": "a".repeat(20_000)}}])));
    events.push(tools(
        json!([{"index": 0, "function": {"arguments": "tail"}}]),
    ));
    let mut body = sse(&events);
    body.extend_from_slice(DONE);
    let (observations, _) = stream(&body, &[]);
    let message = &kind(&observations, "model_response")[0];
    let kept = message.fields["content"].as_str().unwrap();
    assert!(kept.len() <= 16 * 1024);
    assert_eq!(message.fields["content_truncated"], true);
    let full: usize = (0..10).map(|n| n.to_string().len() + 4_000).sum();
    assert_eq!(message.fields["content_bytes"], full);
    assert!(kept.starts_with("0é") && kept.contains("1é"));
    let call = &kind(&observations, "tool_call")[0];
    assert_eq!(call.fields["arguments"].as_str().unwrap().len(), 16 * 1024);
    assert_eq!(call.fields["arguments_bytes"], 20_004);
    assert_eq!(call.fields["arguments_truncated"], true);
}

#[test]
fn seventeenth_choice_is_named_loss_but_sixteen_choices_are_complete() {
    for (count, incomplete) in [(16usize, false), (17, true)] {
        let events: Vec<Value> = (0..count)
            .map(|index| {
                json!({"model": MODEL,
                "choices": [{"index": index, "delta": {"content": "x"}}]})
            })
            .collect();
        let mut body = sse(&events);
        body.extend_from_slice(DONE);
        let (observations, summary) = stream(&body, &[]);
        assert_eq!(kind(&observations, "model_response").len(), count.min(16));
        assert_eq!(summary.incomplete, incomplete);
        assert_eq!(
            reasons(&observations).contains(&"choices_over_limit".to_string()),
            incomplete
        );
    }
}

#[test]
fn sixty_fifth_tool_is_counted_loss_but_sixty_four_tools_are_complete() {
    for (count, incomplete) in [(64usize, false), (65, true)] {
        let calls: Vec<Value> = (0..count)
            .map(|i| json!({"index": i, "id": format!("call_{i}"), "function": {"name": "f"}}))
            .collect();
        let mut body = sse(&[tools(Value::Array(calls))]);
        body.extend_from_slice(DONE);
        let (observations, summary) = stream(&body, &[]);
        assert_eq!(kind(&observations, "tool_call").len(), count.min(64));
        let response = &kind(&observations, "model_response")[0];
        assert_eq!(
            response
                .fields
                .get("tool_calls_omitted")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            (count - count.min(64)) as u64
        );
        assert_eq!(summary.incomplete, incomplete);
    }
}

#[test]
fn seventeenth_model_is_loss_but_sixteen_models_are_complete() {
    for (count, incomplete) in [(16usize, false), (17, true)] {
        let events: Vec<Value> = (0..count)
            .map(|index| {
                json!({"model": format!("model-{index}"),
                "choices": [{"index": 0, "delta": {"content": "x"}}]})
            })
            .collect();
        let mut body = sse(&events);
        body.extend_from_slice(DONE);
        let (observations, summary) = stream(&body, &[]);
        assert_eq!(kind(&observations, "model").len(), count.min(16));
        assert_eq!(summary.incomplete, incomplete);
    }
}

#[test]
fn open_streams_share_one_buffer_budget_for_the_text_they_keep() {
    // Twelve streams each ask to keep 64 tool calls of 16 KiB: 12 MiB against an 8 MiB
    // decoder-wide budget. What is kept never exceeds the budget, and the rest is cut, flagged.
    let mut records = Vec::new();
    let mut bodies = Vec::new();
    let streams = 12u64;
    for id in 1..=streams {
        let mut events = Vec::new();
        for call in 0..64u64 {
            events.push(tools(json!([{"index": call, "id": format!("c{call}"),
                "function": {"name": "f", "arguments": "a".repeat(16 * 1024)}}])));
        }
        bodies.push(sse(&events));
        records.push(spec(id, CaptureSource::WireResponse).open());
    }
    // Interleave: every stream's first half, then every stream's second half, all open at once.
    for half in 0..2 {
        for (n, body) in bodies.iter().enumerate() {
            let s = spec(n as u64 + 1, CaptureSource::WireResponse);
            let ends = event_ends(body);
            let (from, to) = if half == 0 {
                (0, ends[31])
            } else {
                (ends[31], body.len())
            };
            records.push(s.segment(half, from as u64, &body[from..to], Vec::new()));
        }
    }
    for id in 1..=streams {
        let s = spec(id, CaptureSource::WireResponse);
        records.push(s.segment(2, 0, DONE, Vec::new()));
        records.push(s.end(EndReason::Complete));
    }
    let (observations, summary) = decode(&records);
    let held: usize = kind(&observations, "tool_call")
        .iter()
        .filter_map(|o| o.fields.get("arguments").and_then(Value::as_str))
        .map(str::len)
        .sum();
    assert!(
        held <= MAX_BUFFERED_BYTES,
        "{held} bytes kept of a {MAX_BUFFERED_BYTES} budget"
    );
    assert!(
        held > MAX_BUFFERED_BYTES / 2,
        "the budget is spent, not ignored: {held}"
    );
    let cut = kind(&observations, "tool_call")
        .iter()
        .filter(|o| o.fields.get("arguments_truncated").is_some())
        .count();
    assert!(
        cut > 0 || summary.incomplete,
        "text was cut, or events past the budget were dropped as over budget"
    );
    // A call that was kept reports its full length, cut or not.
    for o in kind(&observations, "tool_call") {
        assert_eq!(o.fields["arguments_bytes"], 16 * 1024);
    }
    assert!(summary.observations > 0);
}

// ---------------------------------------------------------------------------------------
// Requests.

fn request_body() -> Value {
    json!({"model": "deepseek-v4-flash", "stream": true,
        "stream_options": {"include_usage": true}, "tool_choice": "auto",
        "tools": [{"type": "function", "function": {"name": "read_file"}}],
        "messages": [
            {"role": "system", "content": "You are SECRET-SYSTEM-PROMPT."},
            {"role": "user", "content": [{"type": "text", "text": "write result.txt"}]},
            {"role": "assistant", "content": null, "tool_calls": [
                {"id": "call_a", "type": "function",
                    "function": {"name": "read_file", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": "call_a", "content": "file says hi"}]})
}

#[test]
fn a_request_keeps_structure_and_tool_results_not_prompt_text() {
    let body = request_body().to_string();
    let spec = spec(1, CaptureSource::WireRequest);
    let (observations, summary) = decode(&spec.records(body.as_bytes(), &[30]));
    assert_eq!(summary.unknown_records, 0);

    let request = &kind(&observations, "model_request")[0];
    assert_eq!(request.fields["stream"], true);
    assert_eq!(request.fields["include_usage"], true);
    assert_eq!(request.fields["tool_choice"], "auto");
    assert_eq!(request.fields["tools_count"], 1);
    assert_eq!(request.fields["message_count"], 4);
    assert_eq!(
        request.fields["message_roles"],
        json!({"assistant": 1, "system": 1, "tool": 1, "user": 1})
    );
    let listed = request.fields["messages"].as_array().unwrap();
    assert_eq!(
        listed[0]["text_bytes"],
        "You are SECRET-SYSTEM-PROMPT.".len()
    );
    assert_eq!(listed[1]["content_types"], json!(["text"]));
    assert_eq!(listed[2]["tool_call_ids"], json!(["call_a"]));
    assert_eq!(listed[3]["tool_call_id"], "call_a");
    assert!(
        !serde_json::to_string(&request.fields)
            .unwrap()
            .contains("SECRET")
    );

    let model = &kind(&observations, "model")[0];
    assert_eq!(model.fields["model"], "deepseek-v4-flash");
    assert_eq!(model.fields["side"], "request");

    let result = &kind(&observations, "tool_result")[0];
    assert_eq!(result.fields["call_id"], "call_a");
    assert_eq!(result.fields["output"], "file says hi");
    assert_eq!(result.fields["origin"], "request_messages");
    assert_eq!(parts(result), (0, 1));
}

#[test]
fn two_identical_requests_remain_two_observations() {
    let body = request_body().to_string();
    let mut records = spec(1, CaptureSource::WireRequest).records(body.as_bytes(), &[]);
    records.extend(spec(3, CaptureSource::WireRequest).records(body.as_bytes(), &[]));
    let (observations, _) = decode(&records);
    assert_eq!(kind(&observations, "model_request").len(), 2);
    assert_eq!(kind(&observations, "tool_result").len(), 2);
}

// ---------------------------------------------------------------------------------------
// Selection.

#[test]
fn selection_uses_hint_source_and_content_type_not_host() {
    let body = typical();
    // No hint, another hint, the Responses hint or a different spelling: nothing.
    for hint in [
        None,
        Some("openai-responses"),
        Some("anthropic-messages"),
        Some("OpenAI-Chat-Completions"),
    ] {
        let (observations, summary) = decode(&response().hint(hint).records(&body, &[]));
        assert!(observations.is_empty(), "{hint:?}");
        assert_eq!(summary.unknown_records, 0);
    }
    // Chat Completions has no WebSocket form, and native and goal streams are never its own.
    for source in [
        CaptureSource::WsClient,
        CaptureSource::WsServer,
        CaptureSource::Native,
        CaptureSource::Goal,
    ] {
        let (observations, _) = decode(&spec(1, source).records(&body, &[]));
        assert!(observations.is_empty(), "{source:?}");
    }
    // An HTTP body is decoded only as its observed content type.
    let plain = response().content_type(Some("text/plain"));
    let (observations, summary) = decode(&plain.records(&body, &[]));
    assert_eq!(reasons(&observations), ["unsupported_content_type"]);
    assert_eq!(observations[0].fields["content_type"], "text/plain");
    assert_eq!(summary.unknown_records, 1);
    // A request is JSON, never SSE.
    let sse_request = spec(1, CaptureSource::WireRequest).content_type(Some("text/event-stream"));
    let (observations, _) = decode(&sse_request.records(&body, &[]));
    assert_eq!(reasons(&observations), ["unsupported_content_type"]);
    // An empty body yields nothing.
    let (observations, summary) = decode(&response().records(b"", &[]));
    assert!(observations.is_empty());
    assert_eq!(summary.unknown_records, 0);
}

#[test]
fn the_decoder_declares_its_identity() {
    assert_eq!(DECODER_ID, "openai-chat-completions");
    assert_eq!(PROTOCOL_HINT, HINT);
    assert_eq!(chat().id(), DECODER_ID);
    assert_eq!(chat().version(), DECODER_VERSION);
}

#[test]
fn decoding_twice_is_byte_identical() {
    let records = response().records(&typical(), &[40, 100]);
    let (first, a) = decode(&records);
    let (second, b) = decode(&records);
    assert_eq!(
        format!("{first:?}"),
        format!("{second:?}"),
        "sources and times too"
    );
    assert_eq!(facts(&first), facts(&second));
    assert_eq!(
        (a.observations, a.unknown_records),
        (b.observations, b.unknown_records)
    );
}

// ---------------------------------------------------------------------------------------
// Never a panic.

const FRAGMENTS: &[&str] = &[
    "data: ",
    "data:",
    "\n\n",
    "\r\n\r\n",
    "\n",
    "\r",
    ": c",
    "[DONE]",
    "{",
    "}",
    "[",
    "]",
    "\"choices\":",
    "\"delta\":",
    "\"content\":\"x\"",
    "\"tool_calls\":[{\"index\":18446744073709551615,\"function\":{\"arguments\":\"a\"}}]",
    "\"tool_calls\":[1,null,{\"index\":-1},{\"function\":7}]",
    "\"usage\":{\"prompt_tokens\":-1,\"completion_tokens\":1.5,\"x\":{}}",
    "\"error\":{\"code\":{},\"type\":[]}",
    "\"finish_reason\":\"stop\"",
    "\"index\":99999999999999999999",
    "\"model\":1",
    "\"id\":\"\\u0000\"",
    "event: x\n",
    ",",
    "\u{ffff}é\u{1f600}",
];

#[test]
fn arbitrary_bytes_cut_anywhere_never_panic_and_summaries_stay_consistent() {
    let mut rng = Lcg(0x5eed);
    for round in 0..400 {
        let mut body = Vec::new();
        for _ in 0..rng.below(40) {
            body.extend_from_slice(FRAGMENTS[rng.below(FRAGMENTS.len())].as_bytes());
            if rng.below(7) == 0 {
                body.push(rng.below(256) as u8);
            }
        }
        if body.is_empty() {
            continue;
        }
        let splits = if body.len() > 1 {
            random_splits(&mut rng, body.len())
        } else {
            Vec::new()
        };
        let source = [
            (CaptureSource::WireResponse, "text/event-stream"),
            (CaptureSource::WireResponse, "application/json"),
            (CaptureSource::WireRequest, "application/json"),
        ][rng.below(3)];
        let s = spec(1, source.0).content_type(Some(source.1));
        let end = [
            EndReason::Complete,
            EndReason::Interrupted,
            EndReason::DecodeError,
        ][rng.below(3)];
        let mut records = vec![s.open()];
        records.extend(s.segments(&body, &splits));
        if rng.below(5) == 0 {
            records.insert(records.len().min(2), s.gap());
        }
        if rng.below(6) != 0 {
            records.push(s.end(end));
        }
        // `decode_one` fails the test if the decoder panicked or returned a failure.
        let (observations, summary) = decode(&records);
        // An unknown with no segment to point at is counted and never emitted; every other
        // observation is both.
        assert!(
            observations.len() as u64 <= summary.observations,
            "round {round}"
        );
        // Never fabricate: no observation names anything the garbage could not have said.
        for o in &observations {
            assert!(o.fields.get("capture_source").is_some());
        }
    }
}

// ---------------------------------------------------------------------------------------
// Review round: holes, tool calls without an index, repeated names, keepalives, after `[DONE]`.

/// `first` and the head of `lost`, a gap, the tail of `lost` and `rest`, as one stream.
fn with_gap(first: &[u8], lost: &[u8], at: usize, rest: &[u8]) -> Vec<CaptureRecord> {
    let s = response();
    let mut tail = lost[at..].to_vec();
    tail.extend_from_slice(rest);
    vec![
        s.open(),
        s.segment(0, 0, first, Vec::new()),
        s.segment(1, first.len() as u64, &lost[..at], Vec::new()),
        s.gap(),
        s.segment(2, 0, &tail, Vec::new()),
        s.end(EndReason::Complete),
    ]
}

#[test]
fn tool_arguments_are_never_spliced_across_a_lost_chunk() {
    let call = |args: &str| {
        tools(json!([{"index": 0, "id": "call_a", "type": "function",
            "function": {"name": "bash", "arguments": args}}]))
    };
    let first = sse(&[call("{\"cmd\":\"rm -rf ")]);
    let lost = sse(&[call("/tmp/build && rm -rf ")]);
    let mut rest = sse(&[call("/home\"}")]);
    rest.extend_from_slice(DONE);
    let (observations, summary) = decode(&with_gap(&first, &lost, 20, &rest));
    let call = &kind(&observations, "tool_call")[0];
    // The kept arguments stay a prefix: nothing after the hole is appended.
    assert_eq!(call.fields["arguments"], "{\"cmd\":\"rm -rf ");
    assert_eq!(call.fields["arguments_truncated"], true);
    assert_eq!(call.fields["stream_complete"], false);
    let all = serde_json::to_string(&facts(&observations)).unwrap();
    assert!(!all.contains("rm -rf /home"), "spliced: {all}");
    assert!(summary.incomplete);
}

#[test]
fn text_and_a_call_first_seen_after_a_hole_are_cut_not_spliced() {
    let first = sse(&[text("a")]);
    let lost = sse(&[text("LOST")]);
    let mut rest = sse(&[
        text("z"),
        tools(json!([{"index": 3, "id": "call_late", "type": "function",
            "function": {"name": "bash", "arguments": "{\"x\":1}"}}])),
    ]);
    rest.extend_from_slice(DONE);
    let (observations, _) = decode(&with_gap(&first, &lost, 10, &rest));
    let message = &kind(&observations, "model_response")[0];
    assert_eq!(message.fields["content"], "a");
    assert_eq!(message.fields["content_truncated"], true);
    assert_eq!(
        message.fields["content_bytes"], 2,
        "the length of what was read"
    );
    // A call first seen after the hole keeps its id and name, not its (possibly headless) arguments.
    let call = &kind(&observations, "tool_call")[0];
    assert_eq!(call.fields["call_id"], "call_late");
    assert_eq!(call.fields["name"], "bash");
    assert_eq!(call.fields["arguments"], "");
    assert_eq!(call.fields["arguments_truncated"], true);
}

#[test]
fn a_streamed_tool_call_without_an_index_merges_by_id_and_never_by_position() {
    let whole = |id: &str, name: &str, args: &str| {
        tools(json!([{"id": id, "type": "function",
            "function": {"name": name, "arguments": args}}]))
    };
    let mut body = sse(&[
        whole("call_A", "read", "{\"p\":\"a\"}"),
        whole("call_B", "write", "{\"p\":\"b\"}"),
        tools(json!([{"id": "call_A", "function": {"arguments": " "}}])),
    ]);
    body.extend_from_slice(DONE);
    let (observations, summary) = stream(&body, &[]);
    let calls = kind(&observations, "tool_call");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].fields["call_id"], "call_A");
    assert_eq!(calls[0].fields["name"], "read");
    assert_eq!(calls[0].fields["arguments"], "{\"p\":\"a\"} ");
    assert!(calls[0].fields.get("index").is_none());
    assert_eq!(calls[1].fields["call_id"], "call_B");
    assert_eq!(calls[1].fields["name"], "write");
    assert_eq!(calls[1].fields["arguments"], "{\"p\":\"b\"}");
    assert_eq!(summary.unknown_records, 0);
}

#[test]
fn a_streamed_tool_call_with_neither_index_nor_id_is_unknown_and_leaves_a_hole() {
    let mut body = sse(&[
        tools(
            json!([{"index": 0, "id": "call_a", "function": {"name": "bash", "arguments": "{\"a\":"}}]),
        ),
        tools(json!([{"function": {"arguments": "1}"}}])),
    ]);
    body.extend_from_slice(DONE);
    let (observations, _) = stream(&body, &[]);
    assert_eq!(reasons(&observations), ["tool_call_without_index"]);
    let call = &kind(&observations, "tool_call")[0];
    assert_eq!(call.fields["arguments"], "{\"a\":");
    assert_eq!(call.fields["arguments_truncated"], true);
    assert_eq!(call.fields["stream_complete"], false);
}

#[test]
fn a_non_stream_body_merges_tool_calls_by_position() {
    let body = json!({"id": "x", "choices": [{"index": 0, "finish_reason": "tool_calls",
        "message": {"role": "assistant", "tool_calls": [
            {"id": "c1", "function": {"name": "a", "arguments": "{}"}},
            {"id": "c2", "function": {"name": "b", "arguments": "{}"}}]}}]})
    .to_string();
    let spec = response().content_type(Some("application/json"));
    let (observations, _) = decode(&spec.records(body.as_bytes(), &[]));
    let calls = kind(&observations, "tool_call");
    assert_eq!(calls.len(), 2);
    assert_eq!(
        (
            calls[0].fields["index"].as_u64(),
            calls[1].fields["index"].as_u64()
        ),
        (Some(0), Some(1))
    );
}

#[test]
fn a_function_name_is_taken_once_like_the_id_never_concatenated() {
    let delta = |name: &str, args: &str| {
        tools(json!([{"index": 0, "id": "call_a", "type": "function",
            "function": {"name": name, "arguments": args}}]))
    };
    let mut body = sse(&[delta("bash", "{\"a\":"), delta("bash", "1}")]);
    body.extend_from_slice(DONE);
    let (observations, _) = stream(&body, &[]);
    let call = &kind(&observations, "tool_call")[0];
    assert_eq!(call.fields["name"], "bash");
    assert_eq!(call.fields["arguments"], "{\"a\":1}");
    assert!(call.fields.get("withheld").is_none());

    // A later, different name is a conflict: withheld, never merged.
    let mut body = sse(&[delta("bash", ""), delta("python", "")]);
    body.extend_from_slice(DONE);
    let (observations, _) = stream(&body, &[]);
    let call = &kind(&observations, "tool_call")[0];
    assert!(call.fields.get("name").is_none());
    assert_eq!(call.fields["withheld"], json!(["name"]));
}

#[test]
fn a_keepalive_object_is_unknown_but_not_a_lost_chunk() {
    let mut body = sse(&[text("a"), json!({"object": "ping"}), text("b")]);
    body.extend_from_slice(DONE);
    let (observations, summary) = stream(&body, &[]);
    assert_eq!(reasons(&observations), ["unsupported_shape"]);
    let message = &kind(&observations, "model_response")[0];
    assert_eq!(message.fields["content"], "ab");
    assert_eq!(message.fields["stream_complete"], true);
    assert!(message.fields.get("unreadable_events").is_none());
    assert!(message.fields.get("content_truncated").is_none());
    assert!(!summary.incomplete);
}

#[test]
fn a_partial_event_after_done_is_counted_with_the_data_after_it() {
    let mut body = typical();
    body.extend_from_slice(b"data: {\"late\"");
    let (observations, summary) = stream(&body, &[]);
    assert_eq!(reasons(&observations), ["data_after_done"]);
    let finding = &kind(&observations, "unknown")[0];
    assert_eq!(finding.fields["events"], 0);
    assert_eq!(finding.fields["partial_events"], 1);
    assert!(!summary.incomplete);

    // The same, then an interrupted end: one finding, not a second report of the partial event.
    let mut records = vec![response().open()];
    records.extend(response().segments(&body, &[]));
    records.push(response().end(EndReason::Interrupted));
    let (observations, _) = decode(&records);
    assert_eq!(reasons(&observations), ["data_after_done", "interrupted"]);
    assert!(
        kind(&observations, "unknown")[1]
            .fields
            .get("pending_event")
            .is_none()
    );
}

#[test]
fn a_gap_after_done_is_reported_once_as_a_gap_and_not_as_data() {
    let mut first = typical();
    first.extend_from_slice(b"data: {\"lo");
    let (observations, summary) = decode(&with_gap(&first, b"", 0, b"st\"}\n\n"));
    assert_eq!(reasons(&observations), ["gap"], "{observations:?}");
    assert!(summary.incomplete);
    assert_eq!(
        kind(&observations, "model_response")[0].fields["stream_complete"],
        true
    );
}

// ---------------------------------------------------------------------------------------
// Last round: shapes the decoder skips are lost chunks; an id-only continuation; comments.

/// "keep ", one chunk of `middle` bytes, then "end" and `[DONE]`.
fn around(middle: &str) -> Vec<u8> {
    let mut body = sse(&[text("keep ")]);
    body.extend_from_slice(format!("data: {middle}\n\n").as_bytes());
    body.extend_from_slice(&sse(&[text("end")]));
    body.extend_from_slice(DONE);
    body
}

#[test]
fn a_chunk_whose_choices_or_delta_are_skipped_is_a_lost_chunk_and_no_splice() {
    for (what, middle, reason) in [
        (
            "choices as an object",
            r#"{"choices":{"0":{"delta":{"content":"MIDDLE "}}}}"#,
            "unsupported_shape",
        ),
        (
            "a choices element that is no object",
            r#"{"choices":["MIDDLE "]}"#,
            "unsupported_choice",
        ),
        (
            "a delta that is no object",
            r#"{"choices":[{"index":0,"delta":"MIDDLE "}]}"#,
            "unsupported_choice",
        ),
    ] {
        let (observations, _) = stream(&around(middle), &[]);
        assert_eq!(reasons(&observations), [reason], "{what}");
        let message = &kind(&observations, "model_response")[0];
        assert_eq!(message.fields["content"], "keep ", "{what}");
        assert_eq!(message.fields["content_truncated"], true, "{what}");
        assert_eq!(message.fields["stream_complete"], false, "{what}");
        assert_eq!(message.fields["unreadable_events"], 1, "{what}");
        let all = serde_json::to_string(&facts(&observations)).unwrap();
        assert!(
            !all.contains("MIDDLE") && !all.contains("keep end"),
            "{what}: {all}"
        );
    }
    // Null is absence, not a skip: a null delta or null choices is read as nothing.
    let (observations, _) = stream(
        &around(r#"{"choices":[{"index":0,"delta":null,"finish_reason":null}]}"#),
        &[],
    );
    let message = &kind(&observations, "model_response")[0];
    assert_eq!(message.fields["content"], "keep end");
    assert_eq!(message.fields["stream_complete"], true);
}

#[test]
fn an_id_only_continuation_of_an_indexed_call_merges_into_it() {
    let mut body = sse(&[
        tools(json!([{"index": 0, "id": "call_1", "type": "function",
            "function": {"name": "bash", "arguments": "{\"cmd\":\"ls "}}])),
        tools(json!([{"id": "call_1", "function": {"arguments": "-la\"}"}}])),
    ]);
    body.extend_from_slice(DONE);
    let (observations, summary) = stream(&body, &[]);
    let calls = kind(&observations, "tool_call");
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].fields["arguments"], "{\"cmd\":\"ls -la\"}");
    assert_eq!(calls[0].fields["index"], 0);
    assert!(calls[0].fields.get("arguments_truncated").is_none());
    assert_eq!(summary.unknown_records, 0);

    // The other way: an id-keyed call gains an index, then either identifier keeps resolving
    // to that one call. Put each event in its own segment so the aggregate source must cover all
    // four contributing chunks.
    let mut body = sse(&[
        tools(json!([{"id": "call_2", "function": {"name": "bash", "arguments": "{\"a\":"}}])),
        tools(json!([{"index": 4, "id": "call_2", "function": {"arguments": "1"}}])),
        tools(json!([{"index": 4, "function": {"arguments": ",\"b\":"}}])),
        tools(json!([{"id": "call_2", "function": {"arguments": "2}"}}])),
    ]);
    body.extend_from_slice(DONE);
    let ends = event_ends(&body);
    let (observations, summary) = stream(&body, &ends[..ends.len() - 1]);
    let calls = kind(&observations, "tool_call");
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].fields["index"], 4);
    assert_eq!(calls[0].fields["call_id"], "call_2");
    assert_eq!(calls[0].fields["name"], "bash");
    assert_eq!(calls[0].fields["arguments"], "{\"a\":1,\"b\":2}");
    assert_eq!(calls[0].fields["stream_complete"], true);
    assert!(calls[0].fields.get("arguments_truncated").is_none());
    assert_eq!(parts(calls[0]), (0, 3));
    assert_eq!(summary.unknown_records, 0);
    assert!(!summary.incomplete);
}

#[test]
fn id_keyed_parallel_calls_gain_indexes_in_reverse_order_without_conflation() {
    let mut body = sse(&[
        tools(json!([
            {"id": "call_a", "function": {"name": "alpha", "arguments": "{\"a\":"}},
            {"id": "call_b", "function": {"name": "beta", "arguments": "{\"b\":"}}
        ])),
        tools(json!([
            {"index": 9, "id": "call_b", "function": {"arguments": "2"}},
            {"index": 4, "id": "call_a", "function": {"arguments": "1"}}
        ])),
        tools(json!([
            {"index": 4, "function": {"arguments": "}"}},
            {"index": 9, "function": {"arguments": "}"}}
        ])),
    ]);
    body.extend_from_slice(DONE);
    let (observations, summary) = stream(&body, &[]);
    let calls = kind(&observations, "tool_call");
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(calls[0].fields["index"], 4);
    assert_eq!(calls[0].fields["call_id"], "call_a");
    assert_eq!(calls[0].fields["name"], "alpha");
    assert_eq!(calls[0].fields["arguments"], "{\"a\":1}");
    assert_eq!(calls[0].fields["stream_complete"], true);
    assert_eq!(calls[1].fields["index"], 9);
    assert_eq!(calls[1].fields["call_id"], "call_b");
    assert_eq!(calls[1].fields["name"], "beta");
    assert_eq!(calls[1].fields["arguments"], "{\"b\":2}");
    assert_eq!(calls[1].fields["stream_complete"], true);
    assert_eq!(summary.unknown_records, 0);
    assert!(!summary.incomplete);
}

#[test]
fn conflicting_established_tool_id_and_index_are_unknown_not_conflated() {
    let mut body = sse(&[
        tools(json!([
            {"index": 4, "id": "call_a", "function": {"name": "alpha", "arguments": "a"}},
            {"id": "call_b", "function": {"name": "beta", "arguments": "b"}}
        ])),
        tools(json!([
            {"index": 4, "id": "call_b", "function": {"arguments": "c"}}
        ])),
    ]);
    body.extend_from_slice(DONE);
    let ends = event_ends(&body);
    let (observations, summary) = stream(&body, &ends[..ends.len() - 1]);

    assert_eq!(reasons(&observations), ["tool_call_identity_conflict"]);
    assert_eq!(parts(kind(&observations, "unknown")[0]), (1, 1));
    let calls = kind(&observations, "tool_call");
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(calls[0].fields["index"], 4);
    assert_eq!(calls[0].fields["call_id"], "call_a");
    assert_eq!(calls[0].fields["arguments"], "a");
    assert_eq!(calls[1].fields["call_id"], "call_b");
    assert!(calls[1].fields.get("index").is_none());
    assert_eq!(calls[1].fields["arguments"], "b");
    for call in calls {
        assert_eq!(call.fields["stream_complete"], false);
        assert_eq!(call.fields["arguments_truncated"], true);
    }
    assert_eq!(
        kind(&observations, "model_response")[0].fields["unreadable_events"],
        1
    );
    assert_eq!(summary.unknown_records, 1);
    assert!(!summary.incomplete);
}

#[test]
fn conflicting_indexes_already_bound_to_different_ids_are_not_conflated() {
    let mut body = sse(&[
        tools(json!([
            {"index": 4, "id": "call_a", "function": {"name": "alpha", "arguments": "a"}},
            {"index": 9, "id": "call_b", "function": {"name": "beta", "arguments": "b"}}
        ])),
        tools(json!([
            {"index": 4, "id": "call_b", "function": {"arguments": "c"}}
        ])),
    ]);
    body.extend_from_slice(DONE);
    let (observations, summary) = stream(&body, &[]);

    assert_eq!(reasons(&observations), ["tool_call_identity_conflict"]);
    let calls = kind(&observations, "tool_call");
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(calls[0].fields["index"], 4);
    assert_eq!(calls[0].fields["call_id"], "call_a");
    assert_eq!(calls[0].fields["arguments"], "a");
    assert_eq!(calls[1].fields["index"], 9);
    assert_eq!(calls[1].fields["call_id"], "call_b");
    assert_eq!(calls[1].fields["arguments"], "b");
    for call in calls {
        assert_eq!(call.fields["stream_complete"], false);
        assert_eq!(call.fields["arguments_truncated"], true);
    }
    assert_eq!(summary.unknown_records, 1);
    assert!(!summary.incomplete);
}

#[test]
fn an_occupied_index_rejects_a_new_conflicting_id() {
    let mut body = sse(&[
        tools(json!([
            {"index": 4, "id": "call_a", "function": {"name": "alpha", "arguments": "a"}}
        ])),
        tools(json!([
            {"index": 4, "id": "call_b", "function": {"arguments": "c"}}
        ])),
    ]);
    body.extend_from_slice(DONE);
    let (observations, summary) = stream(&body, &[]);

    assert_eq!(reasons(&observations), ["tool_call_identity_conflict"]);
    let calls = kind(&observations, "tool_call");
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].fields["index"], 4);
    assert_eq!(calls[0].fields["call_id"], "call_a");
    assert_eq!(calls[0].fields["arguments"], "a");
    assert_eq!(calls[0].fields["stream_complete"], false);
    assert_eq!(calls[0].fields["arguments_truncated"], true);
    assert_eq!(summary.unknown_records, 1);
    assert!(!summary.incomplete);
}

#[test]
fn a_comment_after_done_is_not_data() {
    let mut body = typical();
    body.extend_from_slice(b": keepalive");
    let (observations, summary) = stream(&body, &[]);
    assert!(
        reasons(&observations).is_empty(),
        "{:?}",
        reasons(&observations)
    );
    assert!(!summary.incomplete);
    // A partial `data` line is data; a bare comment line before it does not hide it.
    let mut body = typical();
    body.extend_from_slice(b": c\ndata: {\"late\"");
    let (observations, _) = stream(&body, &[]);
    assert_eq!(reasons(&observations), ["data_after_done"]);
    assert_eq!(
        kind(&observations, "unknown")[0].fields["partial_events"],
        1
    );
}
