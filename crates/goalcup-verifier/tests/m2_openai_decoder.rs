//! The bounded OpenAI Responses decoder (`openai-responses`).
//!
//! Every input is a canonical `CaptureRecord` built here with deterministic times. The real
//! case replays the sanitized Codex CLI 0.153.0 WebSocket run in
//! `tests/fixtures/m2-openai/` (provenance there); its oracles come from that run's E01
//! measurement (`docs/qualification/m2-early-live-capture.md`) and the native
//! rollout's own `turn.completed`, never from this decoder. Every other case is synthetic and
//! says so.

mod support;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use goalcup_protocol::capture::{
    CaptureMeta, CaptureRecord, CaptureSource, CaptureTime, ConnectionEvent, ConnectionOutcome,
    EndReason, GapReason, WireIdentity,
};
use goalcup_protocol::redaction::{RedactionAudit, RedactionAuditSpan};
use goalcup_verifier::capture_analysis::api::{DecoderSummary, DecoderV1, Observation};
use goalcup_verifier::capture_analysis::openai::{
    DECODER_ID, DECODER_VERSION, MAX_BUFFERED_BYTES, MAX_EVENT_BYTES, OpenAiDecoder,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use support::wire::{WireSpec, facts, kind, parts};

/// Fresh 128-bit identifiers. The producer is also the clock identity.
const PRODUCER: &str = "0123456789abcdef0123456789abcdef";
const LOST_PRODUCER: &str = "fedcba9876543210fedcba9876543210";
const CONNECTION: &str = "5e1f0a2b3c4d5e6f708192a3b4c5d6e7";
const OTHER_CONNECTION: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90";
const HINT: &str = "openai-responses";
const REAL_RUN: &str = include_str!("fixtures/m2-openai/codex-0.153.0-ws-run.jsonl");

// ---------------------------------------------------------------------------------------
// Local constructors.

fn time(tick: u64) -> CaptureTime {
    CaptureTime {
        wall: format!("2026-09-28T00:00:00.{tick:09}Z"),
        mono_ns: tick,
        clock_id: PRODUCER.into(),
    }
}

type Spec = WireSpec;

fn spec(stream: u64, source: CaptureSource) -> Spec {
    let ws = matches!(source, CaptureSource::WsClient | CaptureSource::WsServer);
    let content_type = match source {
        CaptureSource::WireRequest => Some("application/json".to_string()),
        CaptureSource::WireResponse => Some("text/event-stream".to_string()),
        _ => None,
    };
    WireSpec::new(
        PRODUCER,
        stream,
        source,
        CaptureMeta {
            wire: Some(WireIdentity {
                connection: CONNECTION.into(),
                exchange: 1,
                message_index: ws.then_some(stream),
            }),
            label: "same label".into(),
            protocol_hint: Some(HINT.into()),
            method: None,
            host: Some("chatgpt.com".into()),
            path: Some("/backend-api/codex/responses".into()),
            status: (source == CaptureSource::WireResponse).then_some(200),
            content_type,
        },
        time,
    )
}

/// One stream (number 1) of `bytes` split at `splits`.
fn wire_records(
    bytes: &[u8],
    source: CaptureSource,
    hint: Option<&str>,
    splits: &[usize],
) -> Vec<CaptureRecord> {
    let mut spec = spec(1, source);
    spec.meta.protocol_hint = hint.map(str::to_string);
    spec.records(bytes, splits)
}

fn openai() -> Box<dyn DecoderV1> {
    Box::new(OpenAiDecoder::new())
}

/// A fresh decoder over `records`, every callback observation in order, then `finish`.
fn decode(records: &[CaptureRecord]) -> (Vec<Observation>, DecoderSummary) {
    support::decode::decode_one(openai, records)
}

fn with_id<'a>(observations: &'a [Observation], kind: &str, id: &str) -> &'a Observation {
    let found: Vec<_> = observations
        .iter()
        .filter(|o| o.kind == kind && o.supplied_id.as_deref() == Some(id))
        .collect();
    assert_eq!(found.len(), 1, "exactly one {kind} for {id}: {found:?}");
    found[0]
}

fn ws(message: &Value) -> Vec<u8> {
    serde_json::to_vec(message).unwrap()
}

/// SSE framing as the Responses API sends it: `event:` then `data:`, a blank line after each.
fn sse(events: &[Value]) -> Vec<u8> {
    let mut body = Vec::new();
    for event in events {
        let name = event["type"].as_str().unwrap();
        body.extend_from_slice(format!("event: {name}\ndata: {event}\n\n").as_bytes());
    }
    body
}

fn created(id: &str) -> Value {
    json!({"type": "response.created", "sequence_number": 0, "response": {
        "id": id, "object": "response", "status": "in_progress", "created_at": 1_790_000_000,
        "model": "gpt-5.6-luna", "output": [], "usage": null, "error": null}})
}

fn completed(id: &str, usage: Value) -> Value {
    json!({"type": "response.completed", "sequence_number": 9, "response": {
        "id": id, "object": "response", "status": "completed", "created_at": 1_790_000_000,
        "completed_at": 1_790_000_004, "model": "gpt-5.6-luna", "output": [], "usage": usage,
        "error": null, "incomplete_details": null}})
}

fn tool_done(item: &str, call: &str, input: &str) -> Value {
    json!({"type": "response.output_item.done", "output_index": 1, "sequence_number": 5,
        "item": {"id": item, "type": "custom_tool_call", "status": "completed", "call_id": call,
            "name": "exec", "input": input}})
}

fn request(input: Value) -> Value {
    json!({"type": "response.create", "model": "gpt-5.6-luna", "input": input,
        "stream": true, "store": false, "reasoning": {"effort": "low"}})
}

fn ticks(observation: &Observation) -> (u64, u64) {
    (
        observation.observed.first.mono_ns,
        observation.observed.last.mono_ns,
    )
}

// ---------------------------------------------------------------------------------------
// The sanitized real Codex run.

struct RealLine {
    stream: u64,
    text: String,
    first: Option<CaptureTime>,
    last: Option<CaptureTime>,
    message_index: Option<u64>,
}

fn real_time(value: &Value) -> Option<CaptureTime> {
    Some(CaptureTime {
        wall: value.get("wall")?.as_str()?.into(),
        mono_ns: value.get("mono_ns")?.as_u64()?,
        clock_id: PRODUCER.into(),
    })
}

/// The fixture as canonical records: one stream per line in Open order, one segment carrying
/// the sanitized bytes over the measured first/last times, End(Complete). Producer and
/// connection are this file's placeholders; the empty upgrade exchange has no segment.
fn real_run() -> (Vec<CaptureRecord>, Vec<RealLine>) {
    let mut records = Vec::new();
    let mut lines = Vec::new();
    for line in REAL_RUN.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let stream = row["stream"].as_u64().unwrap();
        let source: CaptureSource = serde_json::from_value(row["source"].clone()).unwrap();
        let message_index = row.get("message_index").and_then(Value::as_u64);
        let meta = CaptureMeta {
            wire: Some(WireIdentity {
                connection: CONNECTION.into(),
                exchange: row["exchange"].as_u64().unwrap(),
                message_index,
            }),
            label: row["label"].as_str().unwrap().into(),
            protocol_hint: Some(HINT.into()),
            method: row
                .get("method")
                .and_then(Value::as_str)
                .map(str::to_string),
            host: Some("chatgpt.com".into()),
            path: Some("/backend-api/codex/responses".into()),
            status: row.get("status").and_then(Value::as_u64).map(|s| s as u16),
            content_type: row
                .get("content_type")
                .and_then(Value::as_str)
                .map(str::to_string),
        };
        let spec = WireSpec::new(PRODUCER, stream, source, meta, time);
        let text = row["text"].as_str().unwrap().to_string();
        let first = row.get("first").and_then(real_time);
        let last = row.get("last").and_then(real_time);
        records.push(spec.open());
        if !text.is_empty() {
            records.push(CaptureRecord::Segment {
                stream: spec.key.clone(),
                part: 0,
                source_offset: 0,
                source_len: text.len() as u64,
                first: first.clone().unwrap(),
                last: last.clone().unwrap(),
                data_b64: STANDARD.encode(text.as_bytes()),
                audit: RedactionAudit { spans: Vec::new() },
            });
        }
        records.push(spec.end(EndReason::Complete));
        lines.push(RealLine {
            stream,
            text,
            first,
            last,
            message_index,
        });
    }
    (records, lines)
}

#[test]
fn real_response_model_tool_usage_are_source_linked() {
    let (records, lines) = real_run();
    let by_stream: BTreeMap<u64, &RealLine> = lines.iter().map(|l| (l.stream, l)).collect();
    // The upgrade exchange, 6 client and 448 server messages.
    assert_eq!(lines.len(), 456);
    let (observations, summary) = decode(&records);

    // Every observation points at the one message it was decoded from, over that message's
    // measured observation times, and any supplied ID really occurs in that message.
    for o in &observations {
        let line = by_stream[&o.source.stream.stream];
        assert_eq!(parts(o), (0, 0));
        assert_eq!(Some(&o.observed.first), line.first.as_ref());
        assert_eq!(Some(&o.observed.last), line.last.as_ref());
        if let Some(id) = &o.supplied_id {
            assert!(line.text.contains(id.as_str()), "{id} not in its source");
        }
        assert_eq!(
            o.fields["wire"]["message_index"].as_u64(),
            line.message_index
        );
    }

    // `model` on every wire `response.create` is gpt-5.6-luna; 6 model calls.
    let requests = kind(&observations, "model_request");
    assert_eq!(requests.len(), 6);
    let models: Vec<_> = kind(&observations, "model")
        .into_iter()
        .map(|o| {
            (
                o.fields["side"].as_str().unwrap(),
                o.fields["model"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        models
            .iter()
            .filter(|m| *m == &("request", "gpt-5.6-luna"))
            .count(),
        6
    );
    assert_eq!(
        models
            .iter()
            .filter(|m| *m == &("response", "gpt-5.6-luna"))
            .count(),
        6
    );

    // Message 0 is a warm-up with generate:false and no user content; message 7
    // chains to the warm-up's response and carries developer, user, user; the 4
    // continuations each carry one custom_tool_call_output.
    let request_at = |index: u64| {
        requests
            .iter()
            .find(|o| o.fields["wire"]["message_index"] == index)
            .unwrap()
    };
    let warm_up = request_at(0);
    assert_eq!(warm_up.fields["generate"], false);
    assert!(warm_up.fields.get("previous_response_id").is_none());
    assert_eq!(warm_up.fields["input_roles"], json!({"developer": 2}));
    let goal = request_at(7);
    assert_eq!(
        goal.fields["input_roles"],
        json!({"developer": 1, "user": 2})
    );
    let warm_up_response = with_id(&observations, "outcome", "resp_ph004");
    assert_eq!(warm_up_response.fields["generate"], false);
    assert_eq!(goal.fields["previous_response_id"], "resp_ph004");
    for index in [114, 207, 324, 441] {
        let continuation = request_at(index);
        assert!(continuation.fields["previous_response_id"].is_string());
        assert_eq!(continuation.fields["input_item_count"], 1);
        assert_eq!(
            continuation.fields["input_items"][0]["type"],
            "custom_tool_call_output"
        );
    }

    // 4 exec calls, the first writes result.txt, each has an output; the
    // tool-field criterion: the call names result.txt, an output holds m2-capture-ok.
    let calls = kind(&observations, "tool_call");
    assert_eq!(calls.len(), 4);
    assert!(calls.iter().all(|c| c.fields["name"] == "exec"
        && c.fields["item_type"] == "custom_tool_call"
        && c.fields["input"].as_str().unwrap().contains("result.txt")));
    let results = kind(&observations, "tool_result");
    assert_eq!(results.len(), 4);
    let call_id = |o: &&Observation| o.fields["call_id"].as_str().unwrap().to_string();
    let call_ids: BTreeSet<_> = calls.iter().map(call_id).collect();
    let result_ids: BTreeSet<_> = results.iter().map(call_id).collect();
    assert_eq!(call_ids.len(), 4);
    assert_eq!(call_ids, result_ids);
    assert!(
        results
            .iter()
            .any(|r| r.fields["output_parts"][1]["text"] == "m2-capture-ok")
    );

    // Six provider outcomes, all completed, each with its own supplied usage. The warm-up
    // generated nothing. The other five, added up here by the test (never by the decoder),
    // equal what the native rollout's turn.completed reported: 62 545 input,
    // 51 968 cached, 800 output, 357 reasoning.
    let outcomes = kind(&observations, "outcome");
    assert_eq!(outcomes.len(), 6);
    assert!(outcomes.iter().all(|o| o.fields["status"] == "completed"
        && o.fields["scope"] == "provider_response"
        && o.fields["provider_completed_at"].is_u64()));
    let usage = kind(&observations, "usage");
    assert_eq!(usage.len(), 6);
    let warm_up_usage = with_id(&observations, "usage", "resp_ph004");
    assert_eq!(warm_up_usage.fields["input_tokens"], 10_317);
    assert_eq!(warm_up_usage.fields["output_tokens"], 0);
    let generating: Vec<_> = usage
        .iter()
        .filter(|u| u.supplied_id.as_deref() != Some("resp_ph004"))
        .collect();
    let sum = |f: &dyn Fn(&Value) -> u64| generating.iter().map(|u| f(&u.fields)).sum::<u64>();
    assert_eq!(sum(&|f| f["input_tokens"].as_u64().unwrap()), 62_545);
    assert_eq!(
        sum(&|f| f["input_tokens_details"]["cached_tokens"].as_u64().unwrap()),
        51_968
    );
    assert_eq!(sum(&|f| f["output_tokens"].as_u64().unwrap()), 800);
    assert_eq!(
        sum(&|f| f["output_tokens_details"]["reasoning_tokens"]
            .as_u64()
            .unwrap()),
        357
    );
    // The per-item attribution block is not interpreted, but it is named.
    assert!(
        generating
            .iter()
            .all(|u| u.fields["unrecognized_fields"] == json!(["attribution"]))
    );

    // One model_response per response.created and per finished message or reasoning item,
    // counted straight from the fixture's JSON without the decoder.
    let responses = lines
        .iter()
        .filter(|line| !line.text.is_empty())
        .map(|line| serde_json::from_str::<Value>(&line.text).unwrap())
        .filter(|m| {
            m["type"] == "response.created"
                || (m["type"] == "response.output_item.done"
                    && (m["item"]["type"] == "message" || m["item"]["type"] == "reasoning"))
        })
        .count();
    assert_eq!(responses, 13);
    assert_eq!(kind(&observations, "model_response").len(), responses);

    // The final answer is the model's message "done".
    let final_answer = with_id(&observations, "model_response", "msg_ph032");
    assert_eq!(final_answer.fields["phase"], "final_answer");
    assert_eq!(final_answer.fields["content_parts"][0]["text"], "done");

    // The three Codex vendor events on every call are reported, not silently dropped.
    let unknown = kind(&observations, "unknown");
    assert_eq!(unknown.len(), 18);
    let mut vendor: BTreeMap<String, usize> = BTreeMap::new();
    for u in &unknown {
        assert_eq!(u.fields["reason"], "unsupported_event");
        *vendor
            .entry(u.fields["event_type"].as_str().unwrap().into())
            .or_default() += 1;
    }
    assert_eq!(
        vendor,
        BTreeMap::from([
            ("codex.rate_limits".to_string(), 6),
            ("codex.response.metadata".to_string(), 6),
            ("responsesapi.websocket_timing".to_string(), 6),
        ])
    );
    assert_eq!(summary.decoder_id, "openai-responses");
    assert_eq!((DECODER_ID, DECODER_VERSION), ("openai-responses", "2"));
    assert_eq!(summary.unknown_records, 18);
    assert_eq!(summary.observations, observations.len() as u64);
    assert!(!summary.incomplete);
}

// ---------------------------------------------------------------------------------------
// Synthetic cases.

#[test]
fn request_context_is_not_human_assistance() {
    // Synthetic: the same request twice, a system and a user message in it.
    let body = request(json!([
        {"type": "message", "role": "system", "content": [{"type": "input_text", "text": "You are terse."}]},
        {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Please write result.txt"}]}
    ]));
    let mut records = spec(1, CaptureSource::WsClient).records(&ws(&body), &[]);
    records.extend(spec(3, CaptureSource::WsClient).records(&ws(&body), &[]));
    let (observations, summary) = decode(&records);
    let kinds: BTreeSet<_> = observations.iter().map(|o| o.kind.as_str()).collect();
    assert_eq!(kinds, BTreeSet::from(["model", "model_request"]));
    let requests = kind(&observations, "model_request");
    assert_eq!(requests.len(), 2);
    for r in &requests {
        assert_eq!(r.fields["input_roles"], json!({"system": 1, "user": 1}));
        assert_eq!(r.fields["input_items"][1]["text_bytes"], 23);
        let text = r.fields.to_string();
        assert!(
            !text.contains("Please write"),
            "request text is context, not a new input"
        );
        for claim in ["human", "assistance", "goal", "intervention"] {
            assert!(!text.contains(claim), "{claim} claimed from a role");
        }
    }
    assert_eq!(summary.unknown_records, 0);
}

#[test]
fn split_json_and_sse_fields_keep_equivalent_facts() {
    // Synthetic WebSocket message cut at every byte.
    let message = ws(&tool_done(
        "ctc_1",
        "call_1",
        "printf 'm2-capture-ok' > result.txt",
    ));
    let spec_ws = spec(1, CaptureSource::WsServer);
    let (whole, _) = decode(&spec_ws.records(&message, &[]));
    assert_eq!(kind(&whole, "tool_call").len(), 1);
    for split in 1..message.len() {
        let (cut, summary) = decode(&spec_ws.records(&message, &[split]));
        assert_eq!(facts(&cut), facts(&whole), "split at {split}");
        assert_eq!(parts(&cut[0]), (0, 1));
        assert_eq!(ticks(&cut[0]), (100, 115));
        assert_eq!(summary.unknown_records, 0);
    }

    // Synthetic SSE body: a comment-only keep-alive, then an event with CRLF line ends
    // directly followed by one with LF line ends.
    let keep_alive = ": keep-alive\n\n";
    let first = format!(
        "event: response.created\r\ndata: {}\r\n\r\n",
        created("resp_1")
    );
    let second = format!(
        "event: response.completed\ndata: {}\n\n",
        completed(
            "resp_1",
            json!({"input_tokens": 3, "output_tokens": 4, "total_tokens": 7})
        )
    );
    let body = format!("{keep_alive}{first}{second}").into_bytes();
    let first_start = keep_alive.len();
    // The first event is complete at the '\r' of its blank line. The '\n' after it belongs to
    // no event, so a cut just after it must not pull the second event into part 0.
    let first_dispatch = first_start + first.len() - 2;
    let second_start = first_start + first.len();
    let spec_sse = spec(2, CaptureSource::WireResponse);
    let (whole, _) = decode(&spec_sse.records(&body, &[]));
    assert_eq!(
        whole.iter().map(|o| o.kind.as_str()).collect::<Vec<_>>(),
        ["model_response", "model", "outcome", "usage"]
    );
    for split in 1..body.len() {
        let (cut, summary) = decode(&spec_sse.records(&body, &[split]));
        assert_eq!(facts(&cut), facts(&whole), "split at {split}");
        // The part holding byte `index` of the body.
        let part = |index: usize| if index < split { 0 } else { 1 };
        assert_eq!(
            parts(&cut[0]),
            (part(first_start), part(first_dispatch)),
            "split at {split}"
        );
        assert_eq!(parts(&cut[2]), (part(second_start), 1), "split at {split}");
        assert_eq!(summary.unknown_records, 0);
        assert!(!summary.incomplete);
    }
}

#[test]
fn openai_event_interval_spans_contributing_segments() {
    // Synthetic: event A whole in segment 0; event B from segment 0 into segment 3, whose
    // only byte is B's blank-line terminator. Each segment has its own distinct times.
    let a = sse(&[created("resp_a")]);
    let b = sse(&[completed("resp_a", json!({"input_tokens": 1}))]);
    let body = [a.as_slice(), b.as_slice()].concat();
    let s = spec(1, CaptureSource::WireResponse);
    let cuts = [a.len() + 10, a.len() + 40, body.len() - 1];
    let mut bounds = vec![0];
    bounds.extend(cuts);
    bounds.push(body.len());
    let mut records = vec![s.open()];
    for (part, w) in bounds.windows(2).enumerate() {
        let part = part as u64;
        let times = (1_000 + 100 * part, 1_050 + 100 * part);
        records.push(s.segment(part, w[0] as u64, &body[w[0]..w[1]], times, Vec::new()));
    }
    records.push(s.end(EndReason::Complete));
    let (observations, _) = decode(&records);

    let created = with_id(&observations, "model_response", "resp_a");
    assert_eq!(parts(created), (0, 0));
    assert_eq!(ticks(created), (1_000, 1_050));
    for kind in ["outcome", "usage"] {
        let o = with_id(&observations, kind, "resp_a");
        assert_eq!(parts(o), (0, 3));
        assert_eq!(ticks(o), (1_000, 1_350));
        assert_eq!(o.observed.first.clock_id, PRODUCER);
        assert_eq!(o.observed.last.clock_id, PRODUCER);
        assert_eq!(o.observed.first.wall, time(1_000).wall);
        assert_eq!(o.observed.last.wall, time(1_350).wall);
    }
    // The provider's own times stay separate fields and never replace the interval.
    let outcome = with_id(&observations, "outcome", "resp_a");
    assert_eq!(outcome.fields["provider_created_at"], 1_790_000_000);
    assert_eq!(outcome.fields["provider_completed_at"], 1_790_000_004);
}

#[test]
fn openai_error_and_outcome_keep_safe_source_fields() {
    // Synthetic (no provider error was observed live): documented Responses
    // failure shapes, each carrying text that must never be kept.
    let events = [
        json!({"type": "response.failed", "sequence_number": 3, "response": {
            "id": "resp_f", "status": "failed", "created_at": 1_790_000_100,
            "error": {"code": "server_error",
                "message": "upstream said Authorization: Bearer sk-live-SECRETSECRETSECRET"},
            "incomplete_details": null, "usage": null}}),
        json!({"type": "response.incomplete", "sequence_number": 4, "response": {
            "id": "resp_i", "status": "incomplete", "completed_at": null,
            "incomplete_details": {"reason": "max_output_tokens"},
            "usage": {"input_tokens": 5, "output_tokens": 7, "total_tokens": 12}}}),
        json!({"type": "error", "sequence_number": 5, "code": "rate_limit_exceeded",
            "message": "Rate limit reached for org-SECRETORG on tokens", "param": null}),
        json!({"type": "error", "sequence_number": 6, "code": "not a token: Bearer abc",
            "message": "dump"}),
    ];
    let mut records = spec(1, CaptureSource::WireResponse).records(&sse(&events), &[]);
    let http_error = json!({"error": {
        "message": "The model `x` does not exist or you do not have access. sk-proj-SECRET",
        "type": "invalid_request_error", "param": "model", "code": "model_not_found"}});
    let not_found = spec(2, CaptureSource::WireResponse)
        .content_type(Some("application/json"))
        .status(404);
    records.extend(not_found.records(&ws(&http_error), &[]));
    let (observations, summary) = decode(&records);

    let strip = |o: &Observation| {
        let mut fields = o.fields.as_object().unwrap().clone();
        fields.remove("capture_source");
        fields.remove("wire");
        Value::Object(fields)
    };
    assert_eq!(
        strip(with_id(&observations, "error", "resp_f")),
        json!({"event": "response.failed", "sequence_number": 3, "scope": "provider_response",
            "response_id": "resp_f", "code": "server_error"})
    );
    assert_eq!(
        strip(with_id(&observations, "outcome", "resp_f")),
        json!({"event": "response.failed", "sequence_number": 3, "scope": "provider_response",
            "response_id": "resp_f", "status": "failed", "provider_created_at": 1_790_000_100})
    );
    assert_eq!(
        strip(with_id(&observations, "outcome", "resp_i")),
        json!({"event": "response.incomplete", "sequence_number": 4,
            "scope": "provider_response", "response_id": "resp_i", "status": "incomplete",
            "incomplete_reason": "max_output_tokens"})
    );
    let stream_errors: Vec<_> = kind(&observations, "error")
        .into_iter()
        .filter(|o| o.fields["scope"] == "stream_event")
        .map(strip)
        .collect();
    assert_eq!(
        stream_errors,
        [
            json!({"event": "error", "sequence_number": 5, "scope": "stream_event",
                "code": "rate_limit_exceeded"}),
            json!({"event": "error", "sequence_number": 6, "scope": "stream_event",
                "withheld": ["code"]}),
        ]
    );
    let http: Vec<_> = kind(&observations, "error")
        .into_iter()
        .filter(|o| o.fields["scope"] == "http_response")
        .map(strip)
        .collect();
    assert_eq!(
        http,
        [json!({"scope": "http_response", "http_status": 404,
            "type": "invalid_request_error", "code": "model_not_found"})]
    );
    let everything = serde_json::to_string(&observations).unwrap();
    for leak in [
        "Bearer",
        "sk-",
        "SECRET",
        "message",
        "param",
        "does not exist",
        "dump",
    ] {
        assert!(!everything.contains(leak), "{leak} kept");
    }
    assert_eq!(summary.unknown_records, 0);
}

#[test]
fn provider_completion_is_not_task_outcome() {
    // Synthetic: a completed response without completed_at or usage, then a response whose
    // stream closes cleanly without any terminal event.
    let done = json!({"type": "response.completed", "response": {
        "id": "resp_1", "status": "completed", "usage": null}});
    let s = spec(1, CaptureSource::WsServer);
    let mut records = s.records(&ws(&done), &[]);
    let open_only = spec(2, CaptureSource::WireResponse);
    records.extend(open_only.records(&sse(&[created("resp_2")]), &[]));
    records.push(CaptureRecord::Connection {
        producer: PRODUCER.into(),
        connection: CONNECTION.into(),
        observed: time(5_000),
        event: ConnectionEvent::Close {
            outcome: ConnectionOutcome::Complete,
        },
    });
    let (observations, summary) = decode(&records);
    let outcomes = kind(&observations, "outcome");
    assert_eq!(outcomes.len(), 1, "a clean close is not a provider outcome");
    assert_eq!(
        outcomes[0].fields,
        json!({"event": "response.completed", "scope": "provider_response",
            "response_id": "resp_1", "status": "completed", "capture_source": "ws_server",
            "wire": {"connection": CONNECTION, "exchange": 1, "message_index": 1}})
    );
    assert!(
        kind(&observations, "usage").is_empty(),
        "absent usage is not zero usage"
    );
    assert!(kind(&observations, "unknown").is_empty());
    assert!(!summary.incomplete);
}

#[test]
fn wire_identity_not_labels_keeps_exchanges_separate() {
    // Synthetic: two exchanges on one connection and two WebSocket messages on another, all
    // with the same label; the SSE bodies' segments interleave mid-event.
    let request_spec = spec(1, CaptureSource::WireRequest).wire(CONNECTION, 1, None);
    let first = spec(2, CaptureSource::WireResponse).wire(CONNECTION, 1, None);
    let second = spec(3, CaptureSource::WireResponse).wire(CONNECTION, 2, None);
    let body_a = sse(&[created("resp_a")]);
    let body_b = sse(&[created("resp_b")]);
    let (a0, a1) = body_a.split_at(30);
    let (b0, b1) = body_b.split_at(50);
    let mut records =
        request_spec.records(&ws(&json!({"model": "gpt-5.6-luna", "input": "hi"})), &[]);
    records.extend([first.open(), second.open()]);
    records.push(first.segment(0, 0, a0, (10, 11), Vec::new()));
    records.push(second.segment(0, 0, b0, (12, 13), Vec::new()));
    records.push(first.segment(1, a0.len() as u64, a1, (14, 15), Vec::new()));
    records.push(second.segment(1, b0.len() as u64, b1, (16, 17), Vec::new()));
    records.extend([
        first.end(EndReason::Complete),
        second.end(EndReason::Complete),
    ]);
    let message = |id: &str| ws(&tool_done(id, "call_same", "same text"));
    let ws_a = spec(4, CaptureSource::WsServer).wire(OTHER_CONNECTION, 1, Some(3));
    let ws_b = spec(5, CaptureSource::WsServer).wire(OTHER_CONNECTION, 1, Some(4));
    records.extend(ws_a.records(&message("ctc_a"), &[]));
    records.extend(ws_b.records(&message("ctc_b"), &[]));
    let (observations, summary) = decode(&records);
    assert_eq!(summary.unknown_records, 0);

    let a = with_id(&observations, "model_response", "resp_a");
    let b = with_id(&observations, "model_response", "resp_b");
    assert_eq!((a.source.stream.stream, parts(a)), (2, (0, 1)));
    assert_eq!((b.source.stream.stream, parts(b)), (3, (0, 1)));
    assert_eq!(
        a.fields["wire"],
        json!({"connection": CONNECTION, "exchange": 1})
    );
    assert_eq!(
        b.fields["wire"],
        json!({"connection": CONNECTION, "exchange": 2})
    );
    let request = &kind(&observations, "model_request")[0];
    assert_eq!(request.fields["wire"], a.fields["wire"]);
    let call_a = with_id(&observations, "tool_call", "ctc_a");
    let call_b = with_id(&observations, "tool_call", "ctc_b");
    assert_eq!(call_a.source.stream.stream, 4);
    assert_eq!(call_a.fields["wire"]["message_index"], 3);
    assert_eq!(call_b.source.stream.stream, 5);
    assert_eq!(call_b.fields["wire"]["message_index"], 4);
    // Shared exchange or equal text never becomes an invented link.
    for o in &observations {
        let fields = o.fields.as_object().unwrap();
        for link in ["request", "lineage", "parent", "label", "response_ref"] {
            assert!(
                !fields.keys().any(|k| k.contains(link)),
                "{link} in {fields:?}"
            );
        }
    }
}

#[test]
fn two_identical_requests_remain_two_observations() {
    // Synthetic: a retried request, byte-identical, including the same tool output.
    let body = request(json!([{"type": "function_call_output", "id": "fco_1",
        "call_id": "call_1", "output": "m2-capture-ok"}]));
    let mut records = spec(1, CaptureSource::WsClient).records(&ws(&body), &[]);
    records.extend(spec(2, CaptureSource::WsClient).records(&ws(&body), &[]));
    let (observations, _) = decode(&records);
    for (kind_name, count) in [("model_request", 2), ("model", 2), ("tool_result", 2)] {
        let seen = kind(&observations, kind_name);
        assert_eq!(seen.len(), count, "{kind_name}");
        let mut first = seen[0].fields.clone();
        let mut second = seen[1].fields.clone();
        assert_eq!(first["wire"]["message_index"], 1);
        assert_eq!(second["wire"]["message_index"], 2);
        first.as_object_mut().unwrap().remove("wire");
        second.as_object_mut().unwrap().remove("wire");
        assert_eq!(first, second);
        assert_ne!(seen[0].source.stream, seen[1].source.stream);
    }
    let result = &kind(&observations, "tool_result")[0];
    assert_eq!(result.supplied_id.as_deref(), Some("fco_1"));
    assert_eq!(result.fields["output"], "m2-capture-ok");
    assert_eq!(result.fields["output_bytes"], 13);
}

#[test]
fn parallel_responses_preserve_supplied_ids() {
    // Synthetic: two concurrent SSE responses on two connections, interleaved per segment,
    // each calling the same tool with the same input under different IDs.
    let one = spec(1, CaptureSource::WireResponse).wire(CONNECTION, 1, None);
    let two = spec(2, CaptureSource::WireResponse).wire(OTHER_CONNECTION, 1, None);
    let body = |item: &str, call: &str| sse(&[tool_done(item, call, "cat result.txt")]);
    let (x, y) = (body("ctc_x", "call_x"), body("ctc_y", "call_y"));
    let mut records = vec![one.open(), two.open()];
    let cuts = [7usize, 40, 90];
    for (part, w) in [0, cuts[0], cuts[1], cuts[2], x.len()]
        .windows(2)
        .enumerate()
    {
        let p = part as u64;
        records.push(one.segment(p, w[0] as u64, &x[w[0]..w[1]], (p, p), Vec::new()));
        records.push(two.segment(p, w[0] as u64, &y[w[0]..w[1]], (p, p), Vec::new()));
    }
    records.extend([one.end(EndReason::Complete), two.end(EndReason::Complete)]);
    let (observations, _) = decode(&records);
    let calls = kind(&observations, "tool_call");
    assert_eq!(calls.len(), 2);
    let seen: BTreeSet<_> = calls
        .iter()
        .map(|c| {
            (
                c.supplied_id.clone().unwrap(),
                c.fields["call_id"].as_str().unwrap().to_string(),
                c.source.stream.stream,
                c.fields["input"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        BTreeSet::from([
            ("ctc_x".into(), "call_x".into(), 1, "cat result.txt".into()),
            ("ctc_y".into(), "call_y".into(), 2, "cat result.txt".into()),
        ])
    );
}

#[test]
fn unknown_output_block_is_counted() {
    // Synthetic: an unknown output item and an unknown usage member beside known fields.
    let messages = [
        created("resp_u"),
        json!({"type": "response.output_item.done", "output_index": 0,
            "item": {"id": "fb_1", "type": "future_block", "payload": {"x": 1}}}),
        json!({"type": "response.output_item.done", "output_index": 1,
            "item": {"id": "fc_1", "type": "function_call", "call_id": "call_f",
                "name": "shell", "arguments": "{\"cmd\":\"ls\"}", "status": "completed"}}),
        completed(
            "resp_u",
            json!({"input_tokens": 11, "output_tokens": 2, "total_tokens": 13,
            "future_usage": {"widgets": 4}, "output_tokens_details": {"reasoning_tokens": 1}}),
        ),
    ];
    let mut records = Vec::new();
    for (n, message) in messages.iter().enumerate() {
        records.extend(spec(n as u64 + 1, CaptureSource::WsServer).records(&ws(message), &[]));
    }
    let (observations, summary) = decode(&records);
    assert_eq!(kind(&observations, "model").len(), 1);
    let usage = with_id(&observations, "usage", "resp_u");
    assert_eq!(usage.fields["input_tokens"], 11);
    assert_eq!(usage.fields["output_tokens"], 2);
    assert_eq!(usage.fields["total_tokens"], 13);
    assert_eq!(
        usage.fields["output_tokens_details"],
        json!({"reasoning_tokens": 1})
    );
    assert_eq!(usage.fields["unrecognized_fields"], json!(["future_usage"]));
    let call = with_id(&observations, "tool_call", "fc_1");
    assert_eq!(call.fields["arguments"], "{\"cmd\":\"ls\"}");
    assert_eq!(call.fields["item_type"], "function_call");
    let unknown = kind(&observations, "unknown");
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].fields["reason"], "unsupported_item");
    assert_eq!(unknown[0].fields["item_type"], "future_block");
    assert_eq!(unknown[0].source.stream.stream, 2);
    assert_eq!(summary.unknown_records, 1);
    assert!(!summary.incomplete, "an unknown block is not a loss");
}

#[test]
fn gap_inside_json_marks_partial_without_guessing_suffix() {
    // Synthetic WebSocket message with a gap between its two segments: never parsed.
    let message = ws(&completed("resp_g", json!({"input_tokens": 9})));
    let s = spec(1, CaptureSource::WsServer);
    let half = message.len() / 2;
    let records = vec![
        s.open(),
        s.segment(0, 0, &message[..half], (1, 2), Vec::new()),
        s.gap(),
        s.segment(1, half as u64 + 5, &message[half..], (3, 4), Vec::new()),
        s.end(EndReason::Complete),
    ];
    let (observations, summary) = decode(&records);
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].kind, "unknown");
    assert_eq!(observations[0].fields["reason"], "gap");
    assert_eq!(parts(&observations[0]), (0, 1));
    assert!(summary.incomplete);
    assert_eq!(summary.unknown_records, 1);

    // Synthetic SSE: after the gap the bytes look like a complete terminal event, but its
    // start was lost with the gap, so everything through the next blank line is discarded.
    let before = sse(&[created("resp_1")]);
    let partial = b"event: response.output_item.done\ndata: {\"type\":\"response.outp";
    let suffix = format!(
        "data: {}\n\n",
        completed("resp_fake", json!({"input_tokens": 1}))
    );
    let after = sse(&[created("resp_3")]);
    let s = spec(2, CaptureSource::WireResponse);
    let records = vec![
        s.open(),
        s.segment(
            0,
            0,
            &[before.as_slice(), partial].concat(),
            (1, 2),
            Vec::new(),
        ),
        s.gap(),
        s.segment(
            1,
            5_000,
            &[suffix.as_bytes(), &after].concat(),
            (3, 4),
            Vec::new(),
        ),
        s.end(EndReason::Complete),
    ];
    let (observations, summary) = decode(&records);
    assert!(
        kind(&observations, "outcome").is_empty(),
        "suffix guessed as an event"
    );
    assert!(kind(&observations, "usage").is_empty());
    assert_eq!(
        kind(&observations, "model_response")
            .iter()
            .map(|o| o.supplied_id.clone().unwrap())
            .collect::<Vec<_>>(),
        ["resp_1", "resp_3"]
    );
    let unknown = kind(&observations, "unknown");
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].fields["reason"], "gap");
    assert_eq!(parts(unknown[0]), (0, 1));
    assert!(summary.incomplete);

    // A gap and then the end, with no byte after it: still one explicit loss.
    let s = spec(3, CaptureSource::WireResponse);
    let (observations, summary) = decode(&[
        s.open(),
        s.segment(0, 0, &before, (1, 2), Vec::new()),
        s.gap(),
        s.end(EndReason::Interrupted),
    ]);
    assert_eq!(kind(&observations, "model_response").len(), 1);
    assert_eq!(
        kind(&observations, "unknown")
            .iter()
            .map(|observation| observation.fields["reason"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["gap", "interrupted"]
    );
    assert_eq!(summary.unknown_records, 2);
    assert!(summary.incomplete);

    // The gap ends mid-line and the bytes after it start with that line's '\n': that is not
    // a blank line, so the complete-looking data line after it still belongs to the damaged
    // event and is never dispatched on its own.
    let head = b"event: response.completed\ndata: {\"type\"";
    let s = spec(4, CaptureSource::WireResponse);
    let (observations, summary) = decode(&[
        s.open(),
        s.segment(0, 0, head, (1, 2), Vec::new()),
        s.gap(),
        s.segment(
            1,
            9_000,
            &[b"\n".as_slice(), suffix.as_bytes(), &after].concat(),
            (3, 4),
            Vec::new(),
        ),
        s.end(EndReason::Complete),
    ]);
    assert!(
        kind(&observations, "outcome").is_empty(),
        "suffix guessed as an event"
    );
    with_id(&observations, "model_response", "resp_3");
    assert_eq!(summary.unknown_records, 1);
}

#[test]
fn late_decode_error_does_not_claim_complete_response() {
    // Synthetic SSE: two complete events, then a late decode failure inside the terminal one.
    let good = sse(&[created("resp_1"), tool_done("ctc_1", "call_1", "ls")]);
    let tail = b"event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\"";
    let s = spec(1, CaptureSource::WireResponse);
    let records = vec![
        s.open(),
        s.segment(0, 0, &good, (1, 2), Vec::new()),
        s.segment(1, good.len() as u64, tail, (3, 4), Vec::new()),
        s.gap(),
        s.end(EndReason::DecodeError),
    ];
    let (observations, summary) = decode(&records);
    assert_eq!(
        observations
            .iter()
            .map(|o| o.kind.as_str())
            .collect::<Vec<_>>(),
        ["model_response", "model", "tool_call", "unknown", "unknown"]
    );
    assert_eq!(
        kind(&observations, "unknown")
            .iter()
            .map(|observation| observation.fields["reason"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["gap", "decode_error"]
    );
    assert!(summary.incomplete, "the preceding gap remains a real loss");

    // The terminal event's data line is complete JSON, but its blank line never came: the
    // decode error ends it, so it is not dispatched.
    let whole_line = format!(
        "event: response.completed\ndata: {}\n",
        completed("resp_1", json!({"input_tokens": 1}))
    );
    let s = spec(2, CaptureSource::WireResponse);
    let mut records = vec![s.open()];
    records.extend(s.segments(&[good.as_slice(), whole_line.as_bytes()].concat(), &[], 10));
    records.push(s.end(EndReason::DecodeError));
    let (observations, summary) = decode(&records);
    assert!(kind(&observations, "outcome").is_empty());
    let unknown = kind(&observations, "unknown");
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].fields["reason"], "decode_error");
    assert!(
        !summary.incomplete,
        "D45: a faithfully recorded decode-error cut is report detail, not decoder loss"
    );

    // A WebSocket message ended by a decode error is never parsed, even when its bytes
    // happen to be valid JSON.
    let message = ws(&completed("resp_ws", json!({"input_tokens": 1})));
    let s = spec(3, CaptureSource::WsServer);
    let mut records = vec![s.open()];
    records.extend(s.segments(&message, &[], 10));
    records.push(s.end(EndReason::DecodeError));
    let (observations, summary) = decode(&records);
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].fields["reason"], "decode_error");
    assert!(
        !summary.incomplete,
        "D45: a faithfully recorded decode-error message is report detail, not decoder loss"
    );
}

#[test]
fn interrupted_end_and_producer_close_have_distinct_integrity_meaning() {
    let mut body = sse(&[created("resp_1")]);
    body.extend_from_slice(b"event: response.completed\ndata: {\"type\":\"response.completed\"");
    let s = spec(1, CaptureSource::WireResponse);
    let mut interrupted = vec![s.open()];
    interrupted.extend(s.segments(&body, &[], 10));
    interrupted.push(s.end(EndReason::Interrupted));
    let (observations, summary) = decode(&interrupted);
    assert_eq!(
        kind(&observations, "unknown")[0].fields["reason"],
        "interrupted"
    );
    assert!(!summary.incomplete, "D45 recorded cut is not capture loss");

    let s = spec(2, CaptureSource::WireResponse);
    let mut producer_closed = vec![s.open()];
    producer_closed.extend(s.segments(&body, &[], 20));
    producer_closed.push(CaptureRecord::ProducerClosed {
        producer: PRODUCER.into(),
        observed: time(30),
        final_seq: None,
    });
    let (observations, summary) = decode(&producer_closed);
    assert_eq!(
        kind(&observations, "unknown")[0].fields["reason"],
        "unterminated"
    );
    assert!(
        summary.incomplete,
        "an open stream at ProducerClosed is loss"
    );
}

#[test]
fn faithful_cut_after_dispatched_or_quiet_event_has_one_real_source_unknown() {
    for (reason, expected) in [
        (EndReason::Interrupted, "interrupted"),
        (EndReason::DecodeError, "decode_error"),
    ] {
        for events in [
            vec![created("resp_cut")],
            vec![json!({"type": "response.custom_tool_call_input.delta",
                "item_id": "ctc_quiet", "delta": "{}"})],
        ] {
            let body = sse(&events);
            let s = spec(88, CaptureSource::WireResponse);
            let concurrent = spec(89, CaptureSource::WireResponse);
            let concurrent_body = sse(&[created("resp_concurrent")]);
            let (observations, summary) = decode(&[
                s.open(),
                concurrent.open(),
                concurrent.segment(0, 0, &concurrent_body, (1, 2), Vec::new()),
                s.segment(7, 0, &body, (1, 2), Vec::new()),
                concurrent.end(EndReason::Complete),
                s.end(reason),
            ]);
            with_id(&observations, "model_response", "resp_concurrent");
            let unknown = kind(&observations, "unknown");
            assert_eq!(unknown.len(), 1);
            assert_eq!(unknown[0].fields["reason"], expected);
            assert_eq!(unknown[0].source.stream, s.key);
            assert_eq!(parts(unknown[0]), (7, 7));
            assert!(!summary.incomplete);
        }
    }
}

/// A finished tool-call message padded to exactly `size` bytes.
fn padded_tool_call(item: &str, size: usize) -> Vec<u8> {
    let empty = ws(&tool_done(item, "call_p", ""));
    let pad = "x".repeat(size - empty.len());
    let message = ws(&tool_done(item, "call_p", &pad));
    assert_eq!(message.len(), size);
    message
}

/// Split points every 64 KiB, as the recorder bounds segments.
fn chunks(len: usize) -> Vec<usize> {
    (1..len.div_ceil(64 * 1024))
        .map(|n| n * 64 * 1024)
        .collect()
}

#[test]
fn oversized_event_recovers_at_next_framing_boundary() {
    // Synthetic SSE: one event just over the logical-record bound, then a valid event.
    let huge = padded_tool_call("ctc_big", MAX_EVENT_BYTES + 1);
    let body = [
        b"data: ".as_slice(),
        &huge,
        b"\n\n",
        &sse(&[completed("resp_after", json!({"input_tokens": 2}))]),
    ]
    .concat();
    let s = spec(1, CaptureSource::WireResponse);
    let (observations, summary) = decode(&s.records(&body, &chunks(body.len())));
    let unknown = kind(&observations, "unknown");
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].fields["reason"], "over_budget");
    assert_eq!(unknown[0].fields["limit_bytes"], MAX_EVENT_BYTES);
    assert!(kind(&observations, "tool_call").is_empty());
    assert_eq!(
        with_id(&observations, "outcome", "resp_after").fields["status"],
        "completed"
    );
    assert!(summary.incomplete);

    // Synthetic WebSocket: an oversized message, then an ordinary one.
    let s1 = spec(2, CaptureSource::WsServer);
    let s2 = spec(3, CaptureSource::WsServer);
    let mut records = s1.records(&huge, &chunks(huge.len()));
    records.extend(s2.records(&ws(&created("resp_next")), &[]));
    let (observations, summary) = decode(&records);
    assert_eq!(
        kind(&observations, "unknown")[0].fields["reason"],
        "over_budget"
    );
    with_id(&observations, "model_response", "resp_next");
    assert_eq!(summary.unknown_records, 1);

    // The whole-decoder bound: eight open messages just under 1 MiB fit, a ninth does not;
    // once they end and are released, a tenth fits again.
    let size = 1_000_000;
    assert!(8 * size <= MAX_BUFFERED_BYTES && 9 * size > MAX_BUFFERED_BYTES);
    let specs: Vec<_> = (10..20).map(|n| spec(n, CaptureSource::WsServer)).collect();
    let mut records = Vec::new();
    for s in &specs[..9] {
        records.push(s.open());
        let message = padded_tool_call(&format!("ctc_{}", s.key.stream), size);
        records.extend(s.segments(&message, &chunks(size), 0));
    }
    for s in &specs[..9] {
        records.push(s.end(EndReason::Complete));
    }
    let tenth = &specs[9];
    records.extend(tenth.records(&padded_tool_call("ctc_19", size), &chunks(size)));
    let (observations, summary) = decode(&records);
    let calls = kind(&observations, "tool_call");
    assert_eq!(calls.len(), 9, "streams 10..=17 and 19 decode");
    assert!(calls.iter().all(|c| c.fields["input_truncated"] == true
        && c.fields["input"].as_str().unwrap().len() == 16 * 1024));
    let unknown = kind(&observations, "unknown");
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].source.stream.stream, 18);
    assert_eq!(unknown[0].fields["reason"], "over_budget");
    assert_eq!(summary.unknown_records, 1);
}

#[test]
fn usage_snapshots_are_not_added() {
    // Synthetic: three responses' usage, one of them partial.
    let messages = [
        completed(
            "resp_1",
            json!({"input_tokens": 100, "output_tokens": 10, "total_tokens": 110}),
        ),
        completed(
            "resp_2",
            json!({"input_tokens": 150, "output_tokens": 20, "total_tokens": 170}),
        ),
        json!({"type": "response.incomplete", "response": {"id": "resp_3",
            "status": "incomplete", "usage": {"input_tokens": 7, "output_tokens": null}}}),
    ];
    let mut records = Vec::new();
    for (n, message) in messages.iter().enumerate() {
        records.extend(spec(n as u64 + 1, CaptureSource::WsServer).records(&ws(message), &[]));
    }
    let (observations, _) = decode(&records);
    let usage = kind(&observations, "usage");
    assert_eq!(usage.len(), 3);
    let tokens = |id: &str| {
        let fields = with_id(&observations, "usage", id)
            .fields
            .as_object()
            .unwrap()
            .clone();
        ["input_tokens", "output_tokens", "total_tokens"]
            .map(|name| fields.get(name).and_then(Value::as_u64))
    };
    assert_eq!(tokens("resp_1"), [Some(100), Some(10), Some(110)]);
    assert_eq!(tokens("resp_2"), [Some(150), Some(20), Some(170)]);
    assert_eq!(
        tokens("resp_3"),
        [Some(7), None, None],
        "absence is not zero"
    );
    let everything = serde_json::to_string(&observations).unwrap();
    for aggregate in ["250", "257", "280", "\"sum", "total_input", "cumulative"] {
        assert!(!everything.contains(aggregate), "{aggregate} invented");
    }
}

#[test]
fn finish_is_deterministic_without_wall_clock_or_network() {
    let (mut records, _) = real_run();
    // Three streams never ended, opened out of key order, and one lost with its producer.
    let partial = &ws(&created("resp_open"))[..20];
    for n in [900, 700, 800] {
        let s = spec(n, CaptureSource::WsServer);
        records.push(s.open());
        records.extend(s.segments(partial, &[], n));
    }
    let base = spec(1, CaptureSource::WsServer);
    let lost = WireSpec::new(LOST_PRODUCER, 1, base.source, base.meta, time);
    records.push(lost.open());
    records.push(lost.segment(0, 0, partial, (1, 1), Vec::new()));
    records.push(CaptureRecord::Recovery {
        previous_producer: LOST_PRODUCER.into(),
        producer: PRODUCER.into(),
        observed: time(2),
        reason: GapReason::ProducerLost,
    });

    let run = || {
        let (observations, summary) = decode(&records);
        (
            serde_json::to_vec(&observations).unwrap(),
            serde_json::to_vec(&summary).unwrap(),
            observations,
        )
    };
    let (first_bytes, first_summary, observations) = run();
    let (second_bytes, second_summary, _) = run();
    assert_eq!(first_bytes, second_bytes);
    assert_eq!(first_summary, second_summary);

    let tail: Vec<_> = observations
        .iter()
        .rev()
        .take(4)
        .rev()
        .map(|o| (o.fields["reason"].as_str().unwrap(), o.source.stream.stream))
        .collect();
    assert_eq!(
        tail,
        [
            ("producer_lost", 1),
            ("unterminated", 700),
            ("unterminated", 800),
            ("unterminated", 900)
        ]
    );
    let summary: DecoderSummary = serde_json::from_slice(&first_summary).unwrap();
    assert_eq!(summary.unknown_records, 18 + 4);
    assert!(summary.incomplete);
}

#[test]
fn streamless_loss_makes_the_analysis_incomplete() {
    // Synthetic: a stream-less gap (for example connections unsettled at a shutdown
    // deadline) may have taken selected traffic with it, whichever stream it was.
    let records = spec(1, CaptureSource::WsServer).records(&ws(&created("resp_1")), &[]);
    let (_, summary) = decode(&records);
    assert!(!summary.incomplete);
    let mut lossy = records.clone();
    lossy.push(CaptureRecord::Gap {
        stream: None,
        observed: time(7),
        reason: GapReason::ShutdownDeadline,
        lost_bytes: None,
    });
    let (observations, summary) = decode(&lossy);
    with_id(&observations, "model_response", "resp_1");
    assert!(summary.incomplete);
    assert_eq!(summary.unknown_records, 0, "no record to name, only a loss");
}

#[test]
fn recovery_without_a_tracked_stream_is_still_a_loss() {
    // Synthetic: a whole message of the lost epoch never reached the chain (IngestLost, or
    // refused at Open), so the next epoch's Recovery is its only trace. Two complete
    // messages around it decode; the analysis is incomplete with no record to name.
    let mut records = spec(1, CaptureSource::WsServer).records(&ws(&created("resp_1")), &[]);
    records.push(CaptureRecord::Recovery {
        previous_producer: LOST_PRODUCER.into(),
        producer: PRODUCER.into(),
        observed: time(3),
        reason: GapReason::IngestLost,
    });
    records.extend(spec(2, CaptureSource::WsServer).records(&ws(&created("resp_2")), &[]));
    let (observations, summary) = decode(&records);
    with_id(&observations, "model_response", "resp_1");
    with_id(&observations, "model_response", "resp_2");
    assert!(summary.incomplete, "a Recovery is always a loss");
    assert_eq!(summary.unknown_records, 0, "no record to name, only a loss");
}

#[test]
fn unrecorded_recovery_closes_the_session_resolved_openai_stream_at_the_record() {
    let selected = spec(81, CaptureSource::WireResponse);
    let mut records = vec![selected.open()];
    records.push(selected.segment(0, 0, b"data: {", (10, 11), Vec::new()));
    records.push(CaptureRecord::Recovery {
        previous_producer: LOST_PRODUCER.into(),
        producer: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        observed: time(12),
        reason: GapReason::ProducerLost,
    });

    let (observations, summary) = decode(&records);
    let findings = kind(&observations, "unknown");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].fields["reason"], "producer_lost");
    assert_eq!(findings[0].source.stream, selected.key);
    assert!(summary.incomplete);
}

#[test]
fn redacted_record_is_unknown_not_malformed() {
    // Synthetic: the fail-closed redactor replaced the rest of a line, so the
    // retained message is no longer JSON. Its audit names the replaced source bytes.
    let broken = br#"{"type":"response.create","model":"gpt-5.6-luna","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"mail <redacted:email>"#;
    let marker_at = broken.len() - "<redacted:email>".len();
    // The source ran on for 80 more bytes through its newline; all of it was replaced.
    let source_len = marker_at + 80;
    let audit = vec![RedactionAuditSpan {
        kind: "email".into(),
        start: marker_at as u64,
        end: source_len as u64,
    }];
    let s = spec(1, CaptureSource::WsClient);
    let mut audited = s.segment(0, 0, broken, (1, 2), audit.clone());
    if let CaptureRecord::Segment {
        source_len: len, ..
    } = &mut audited
    {
        *len = source_len as u64;
    }
    let records = vec![s.open(), audited, s.end(EndReason::Complete)];
    let (observations, summary) = decode(&records);
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].kind, "unknown");
    assert_eq!(observations[0].fields["reason"], "redacted");
    assert_eq!(observations[0].fields["kinds"], json!(["email"]));
    assert_eq!(summary.unknown_records, 1);

    // The same bytes with no audit span are malformed, not redacted.
    let (observations, _) = decode(&s.records(broken, &[]));
    assert_eq!(observations[0].fields["reason"], "malformed");

    // A redaction that leaves valid JSON is decoded, and says so.
    let intact = ws(&request(json!([{"type": "message", "role": "user",
        "content": [{"type": "input_text", "text": "mail <redacted:email> please"}]}])));
    let intact_marker = intact
        .windows("<redacted:email>".len())
        .position(|window| window == b"<redacted:email>")
        .unwrap();
    let intact_audit = vec![RedactionAuditSpan {
        kind: "email".into(),
        start: intact_marker as u64,
        end: (intact_marker + "<redacted:email>".len()) as u64,
    }];
    let (observations, _) = decode(&[
        s.open(),
        s.segment(0, 0, &intact, (1, 2), intact_audit),
        s.end(EndReason::Complete),
    ]);
    let request = &kind(&observations, "model_request")[0];
    assert_eq!(request.fields["redaction_kinds"], json!(["email"]));
}

#[test]
fn invalid_redaction_projection_never_decodes_retained_json() {
    let secret = "raw-secret-bearing-value";
    let raw = ws(&request(json!([{"type": "message", "role": "user",
        "content": [{"type": "input_text", "text": secret}]}])));
    let s = spec(19, CaptureSource::WsClient);
    let false_audit = vec![RedactionAuditSpan {
        kind: "email".into(),
        start: 0,
        end: 1,
    }];
    let (observations, summary) = decode(&[
        s.open(),
        s.segment(0, 0, &raw, (1, 2), false_audit),
        s.end(EndReason::Complete),
    ]);
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].kind, "unknown");
    assert_eq!(
        observations[0].fields,
        json!({"reason": "invalid_redaction_audit", "redaction_kinds": ["email"],
            "wire": {"connection": CONNECTION, "exchange": 1, "message_index": 19},
            "capture_source": "ws_client"})
    );
    assert!(
        !serde_json::to_string(&observations)
            .unwrap()
            .contains(secret)
    );
    assert!(summary.incomplete);
}

#[test]
fn invalid_redaction_projection_keeps_one_fixed_diagnostic_across_sse_resync() {
    let secret = "raw-secret-bearing-value";
    let invalid = sse(&[json!({"type": "response.failed", "response": {
        "id": "resp_secret", "status": "failed", "error": {"message": secret}}})]);
    let after = sse(&[created("resp_resync"), created("resp_after_resync")]);
    let s = spec(20, CaptureSource::WireResponse);
    let (observations, summary) = decode(&[
        s.open(),
        s.segment(
            0,
            0,
            &invalid,
            (1, 2),
            vec![RedactionAuditSpan {
                kind: "email".into(),
                start: 0,
                end: 1,
            }],
        ),
        s.segment(1, invalid.len() as u64, &after, (3, 4), Vec::new()),
        s.end(EndReason::Complete),
    ]);
    let unknown = kind(&observations, "unknown");
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].fields["reason"], "invalid_redaction_audit");
    assert_eq!(parts(unknown[0]), (0, 0));
    assert!(
        !serde_json::to_string(&observations)
            .unwrap()
            .contains(secret)
    );
    assert!(
        observations
            .iter()
            .any(|observation| observation.supplied_id.as_deref() == Some("resp_after_resync"))
    );
    assert!(
        observations
            .iter()
            .all(|observation| observation.supplied_id.as_deref() != Some("resp_resync")),
        "the sacrificial resynchronization event is discarded"
    );
    assert!(summary.incomplete);
}

#[test]
fn invalid_audit_and_real_gap_keep_the_real_cause_in_both_orders() {
    for (name, invalid_first) in [("invalid_then_gap", true), ("gap_then_invalid", false)] {
        let s = spec(22, CaptureSource::WireResponse);
        let invalid = sse(&[created("resp_secret")]);
        let invalid_record = s.segment(
            0,
            0,
            &invalid,
            (1, 2),
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
        let after = sse(&[created("resp_sacrificial"), created("resp_after_resync")]);
        records.push(s.segment(1, invalid.len() as u64 + 1, &after, (3, 4), Vec::new()));
        records.push(s.end(EndReason::Complete));

        let (observations, summary) = decode(&records);
        let reasons: Vec<_> = kind(&observations, "unknown")
            .iter()
            .map(|observation| observation.fields["reason"].as_str().unwrap())
            .collect();
        assert_eq!(reasons, ["invalid_redaction_audit", "gap"], "{name}");
        assert!(
            observations
                .iter()
                .all(|observation| observation.supplied_id.as_deref() != Some("resp_sacrificial"))
        );
        with_id(&observations, "model_response", "resp_after_resync");
        assert_eq!(summary.unknown_records, 2, "{name}");
        assert!(summary.incomplete, "{name}");
    }
}

#[test]
fn pending_over_budget_survives_invalid_audit_resync_at_boundary_and_close() {
    for consume_boundary in [true, false] {
        let s = spec(24, CaptureSource::WireResponse);
        let oversized = format!("data: {}", "x".repeat(MAX_EVENT_BYTES + 1));
        let invalid = sse(&[created("resp_invalid_audit")]);
        let invalid_offset = oversized.len() as u64;
        let mut records = vec![
            s.open(),
            s.segment(0, 0, oversized.as_bytes(), (1, 2), Vec::new()),
            s.segment(
                1,
                invalid_offset,
                &invalid,
                (3, 4),
                vec![RedactionAuditSpan {
                    kind: "email".into(),
                    start: invalid_offset,
                    end: invalid_offset + 1,
                }],
            ),
        ];
        if consume_boundary {
            let after = [b"\n\n".as_slice(), &sse(&[created("resp_after_resync")])].concat();
            records.push(s.segment(
                2,
                invalid_offset + invalid.len() as u64,
                &after,
                (5, 6),
                Vec::new(),
            ));
        }
        records.push(s.end(EndReason::Complete));

        let (observations, summary) = decode(&records);
        let reasons: Vec<_> = kind(&observations, "unknown")
            .iter()
            .map(|observation| observation.fields["reason"].as_str().unwrap())
            .collect();
        assert_eq!(
            reasons
                .iter()
                .filter(|reason| **reason == "invalid_redaction_audit")
                .count(),
            1,
            "consume_boundary={consume_boundary}"
        );
        assert_eq!(
            reasons
                .iter()
                .filter(|reason| **reason == "over_budget")
                .count(),
            1,
            "consume_boundary={consume_boundary}"
        );
        assert!(
            reasons.iter().all(|reason| *reason != "gap"),
            "consume_boundary={consume_boundary}: {reasons:?}"
        );
        assert_eq!(summary.unknown_records, reasons.len() as u64);
        assert!(summary.incomplete);
    }
}

fn assert_pending_over_budget_keeps_faithful_cut(end_reason: EndReason, cut: &str) {
    let s = spec(25, CaptureSource::WireResponse);
    let oversized = format!("data: {}", "x".repeat(MAX_EVENT_BYTES + 1));
    let (observations, summary) = decode(&[
        s.open(),
        s.segment(0, 0, oversized.as_bytes(), (1, 2), Vec::new()),
        s.end(end_reason),
    ]);
    let reasons: Vec<_> = kind(&observations, "unknown")
        .iter()
        .map(|observation| observation.fields["reason"].as_str().unwrap())
        .collect();
    assert_eq!(reasons, ["over_budget", cut]);
    assert_eq!(summary.unknown_records, 2);
    assert!(
        summary.incomplete,
        "the independent budget loss remains loss"
    );
}

#[test]
fn pending_over_budget_and_interrupted_cut_are_both_visible_at_close() {
    assert_pending_over_budget_keeps_faithful_cut(EndReason::Interrupted, "interrupted");
}

#[test]
fn pending_over_budget_and_decode_error_cut_are_both_visible_at_close() {
    assert_pending_over_budget_keeps_faithful_cut(EndReason::DecodeError, "decode_error");
}

#[test]
fn complete_end_with_pending_event_is_real_decoder_loss() {
    let s = spec(25, CaptureSource::WireResponse);
    let (observations, summary) = decode(&[
        s.open(),
        s.segment(0, 0, b"data: {", (1, 2), Vec::new()),
        s.end(EndReason::Complete),
    ]);
    let reasons: Vec<_> = kind(&observations, "unknown")
        .iter()
        .map(|observation| observation.fields["reason"].as_str().unwrap())
        .collect();
    assert_eq!(reasons, ["unterminated_event"]);
    assert_eq!(summary.unknown_records, 1);
    assert!(summary.incomplete);
}

#[test]
fn pending_event_close_releases_shared_budget_under_cap_pressure() {
    for (end_reason, close) in [
        (EndReason::Complete, "unterminated_event"),
        (EndReason::Interrupted, "interrupted"),
    ] {
        let first = spec(30, CaptureSource::WireResponse);
        let partial = format!("data: {}", "x".repeat(900_000));
        let later: Vec<_> = (40..48)
            .map(|stream| spec(stream, CaptureSource::WsServer))
            .collect();
        let mut records = vec![
            first.open(),
            first.segment(0, 0, partial.as_bytes(), (1, 2), Vec::new()),
            first.end(end_reason),
        ];

        // Eight 1,000,000-byte messages fit below the 8 MiB shared bound only if closing the
        // partial SSE event released its 900 KiB. Keeping that charge drops the eighth message.
        for stream in &later {
            records.push(stream.open());
            let body = padded_tool_call(&format!("later_{}", stream.key.stream), 1_000_000);
            records.extend(stream.segments(&body, &chunks(body.len()), 0));
        }
        for stream in &later {
            records.push(stream.end(EndReason::Complete));
        }

        let (observations, _) = decode(&records);
        let reasons: Vec<_> = kind(&observations, "unknown")
            .iter()
            .map(|observation| observation.fields["reason"].as_str().unwrap())
            .collect();
        assert_eq!(reasons, [close], "{end_reason:?}");
        assert_eq!(
            kind(&observations, "tool_call").len(),
            later.len(),
            "released capacity serves every later message after {end_reason:?}"
        );
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
    let mut record = spec.segment(part, offset, b"unreadable", (3, 4), Vec::new());
    let CaptureRecord::Segment { data_b64, .. } = &mut record else {
        unreachable!()
    };
    *data_b64 = "%".to_string();
    record
}

#[test]
fn pending_over_budget_precedes_later_sse_damage_and_close() {
    for (damage, damage_record) in ["gap", "undecodable_segment"].into_iter().map(|damage| {
        let s = spec(26, CaptureSource::WireResponse);
        let oversized = format!("data: {}", "x".repeat(MAX_EVENT_BYTES + 1));
        let record = if damage == "gap" {
            s.gap()
        } else {
            undecodable(&s, 1, oversized.len() as u64)
        };
        (damage, record)
    }) {
        let s = spec(26, CaptureSource::WireResponse);
        let oversized = format!("data: {}", "x".repeat(MAX_EVENT_BYTES + 1));
        let records = vec![
            s.open(),
            s.segment(0, 0, oversized.as_bytes(), (1, 2), Vec::new()),
            damage_record,
            s.end(EndReason::Interrupted),
        ];
        let (observations, summary) = decode(&records);
        let reasons: Vec<_> = kind(&observations, "unknown")
            .iter()
            .map(|observation| observation.fields["reason"].as_str().unwrap())
            .collect();
        assert_eq!(reasons, ["over_budget", damage, "interrupted"]);
        assert_eq!(summary.unknown_records, 3);
        assert!(summary.incomplete);
    }

    let s = spec(26, CaptureSource::WireResponse);
    let oversized = format!("data: {}", "x".repeat(MAX_EVENT_BYTES + 1));
    let (observations, summary) = decode(&[
        s.open(),
        s.segment(0, 0, oversized.as_bytes(), (1, 2), Vec::new()),
        s.gap(),
        undecodable(&s, 1, oversized.len() as u64),
        s.end(EndReason::Interrupted),
    ]);
    let reasons: Vec<_> = kind(&observations, "unknown")
        .iter()
        .map(|observation| observation.fields["reason"].as_str().unwrap())
        .collect();
    assert_eq!(
        reasons,
        ["over_budget", "gap", "undecodable_segment", "interrupted"]
    );
    assert_eq!(summary.unknown_records, 4);
    assert!(summary.incomplete);

    let (observations, summary) = decode(&[
        s.open(),
        s.segment(0, 0, oversized.as_bytes(), (1, 2), Vec::new()),
        undecodable(&s, 1, oversized.len() as u64),
        s.gap(),
        s.end(EndReason::Interrupted),
    ]);
    let reasons: Vec<_> = kind(&observations, "unknown")
        .iter()
        .map(|observation| observation.fields["reason"].as_str().unwrap())
        .collect();
    assert_eq!(
        reasons,
        ["over_budget", "undecodable_segment", "gap", "interrupted"],
        "pending causes retain first-seen order"
    );
    assert_eq!(summary.unknown_records, 4);
    assert!(summary.incomplete);
}

#[test]
fn repeated_identical_pending_sse_damage_is_deduplicated() {
    let s = spec(26, CaptureSource::WireResponse);
    let (observations, summary) = decode(&[
        s.open(),
        s.segment(0, 0, b"data: {", (1, 2), Vec::new()),
        s.gap(),
        s.gap(),
        s.segment(1, 10, b"\n\n", (3, 4), Vec::new()),
        s.end(EndReason::Interrupted),
    ]);
    let reasons: Vec<_> = kind(&observations, "unknown")
        .iter()
        .map(|observation| observation.fields["reason"].as_str().unwrap())
        .collect();
    assert_eq!(reasons, ["gap", "interrupted"]);
    assert_eq!(summary.unknown_records, 2);
    assert!(summary.incomplete);
}

#[test]
fn json_loss_precedes_every_independent_non_complete_close() {
    for close in [
        IndependentClose::Interrupted,
        IndependentClose::DecodeError,
        IndependentClose::ProducerLost,
        IndependentClose::Unterminated,
    ] {
        let s = spec(27, CaptureSource::WsClient);
        let invalid = s.segment(
            0,
            0,
            b"{}",
            (1, 2),
            vec![RedactionAuditSpan {
                kind: "email".into(),
                start: 0,
                end: 1,
            }],
        );
        let cases: Vec<(&str, Vec<CaptureRecord>)> = vec![
            (
                "gap",
                vec![s.segment(0, 0, b"{", (1, 2), Vec::new()), s.gap()],
            ),
            (
                "over_budget",
                vec![s.segment(0, 0, &vec![b'x'; MAX_EVENT_BYTES + 1], (1, 2), Vec::new())],
            ),
            ("undecodable_segment", vec![undecodable(&s, 0, 0)]),
            ("invalid_redaction_audit", vec![invalid]),
        ];
        for (loss, mut body) in cases {
            let mut records = vec![s.open()];
            records.append(&mut body);
            close.finish(&mut records, &s);
            let (observations, summary) = decode(&records);
            let reasons: Vec<_> = kind(&observations, "unknown")
                .iter()
                .map(|observation| observation.fields["reason"].as_str().unwrap())
                .collect();
            assert_eq!(reasons, [loss, close.reason()], "{loss}, {close:?}");
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
        let s = spec(28, CaptureSource::WsClient);
        let oversized = vec![b'x'; MAX_EVENT_BYTES + 1];
        let mut records = vec![s.open()];
        let expected = match case {
            "over_then_gap" => {
                records.push(s.segment(0, 0, &oversized, (1, 2), Vec::new()));
                records.push(s.gap());
                vec!["over_budget", "gap", "interrupted"]
            }
            "over_then_undecodable" => {
                records.push(s.segment(0, 0, &oversized, (1, 2), Vec::new()));
                records.push(undecodable(&s, 1, oversized.len() as u64));
                vec!["over_budget", "undecodable_segment", "interrupted"]
            }
            "over_then_invalid_audit" => {
                records.push(s.segment(0, 0, &oversized, (1, 2), Vec::new()));
                let offset = oversized.len() as u64;
                records.push(s.segment(
                    1,
                    offset,
                    b"{}",
                    (3, 4),
                    vec![RedactionAuditSpan {
                        kind: "email".into(),
                        start: offset,
                        end: offset + 1,
                    }],
                ));
                // Invalid audit is diagnosed immediately; retained causes still precede close.
                vec!["invalid_redaction_audit", "over_budget", "interrupted"]
            }
            "over_gap_undecodable" => {
                records.push(s.segment(0, 0, &oversized, (1, 2), Vec::new()));
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
                records.push(s.segment(0, 0, b"{", (1, 2), Vec::new()));
                records.push(s.gap());
                records.push(s.gap());
                vec!["gap", "interrupted"]
            }
            _ => unreachable!(),
        };
        records.push(s.end(EndReason::Interrupted));

        let (observations, summary) = decode(&records);
        let reasons: Vec<_> = kind(&observations, "unknown")
            .iter()
            .map(|observation| observation.fields["reason"].as_str().unwrap())
            .collect();
        assert_eq!(reasons, expected, "{case}");
        assert_eq!(summary.unknown_records, expected.len() as u64, "{case}");
        assert!(summary.incomplete, "{case}");
    }
}

#[test]
fn unsupported_whole_message_preserves_distinct_damage_before_terminal() {
    let s = spec(29, CaptureSource::WireResponse).content_type(Some("text/plain"));
    let (observations, summary) = decode(&[
        s.open(),
        s.segment(0, 0, b"x", (1, 2), Vec::new()),
        s.gap(),
        undecodable(&s, 1, 2),
        s.end(EndReason::Interrupted),
    ]);
    let reasons: Vec<_> = kind(&observations, "unknown")
        .iter()
        .map(|observation| observation.fields["reason"].as_str().unwrap())
        .collect();
    assert_eq!(
        reasons,
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
        (3, 4),
        vec![RedactionAuditSpan {
            kind: "email".into(),
            start: 2,
            end: 3,
        }],
    );
    let (observations, _) = decode(&[
        s.open(),
        s.segment(0, 0, b"x", (1, 2), Vec::new()),
        s.gap(),
        invalid,
        s.end(EndReason::Interrupted),
    ]);
    let reasons: Vec<_> = kind(&observations, "unknown")
        .iter()
        .map(|observation| observation.fields["reason"].as_str().unwrap())
        .collect();
    assert_eq!(
        reasons,
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
            let s = spec(23, CaptureSource::WireResponse);
            let before = sse(&[created("resp_before")]);
            let invalid = sse(&[created("resp_secret")]);
            let invalid_record = s.segment(
                1,
                before.len() as u64,
                &invalid,
                (3, 4),
                vec![RedactionAuditSpan {
                    kind: "email".into(),
                    start: before.len() as u64,
                    end: before.len() as u64 + 1,
                }],
            );
            let mut records = vec![s.open(), s.segment(0, 0, &before, (1, 2), Vec::new())];
            if invalid_first {
                records.push(invalid_record);
                records.push(s.gap());
            } else {
                records.push(s.gap());
                records.push(invalid_record);
            }
            records.push(s.end(end_reason));

            let (observations, summary) = decode(&records);
            let reasons: Vec<_> = kind(&observations, "unknown")
                .iter()
                .map(|observation| observation.fields["reason"].as_str().unwrap())
                .collect();
            assert_eq!(
                reasons,
                ["invalid_redaction_audit", "gap", cut],
                "{name}, {end_reason:?}"
            );
            with_id(&observations, "model_response", "resp_before");
            assert_eq!(summary.unknown_records, 3, "{name}, {end_reason:?}");
            assert!(summary.incomplete, "{name}, {end_reason:?}");
        }
    }
}

#[test]
fn invalid_audit_resync_does_not_hide_later_gap_or_faithful_cut() {
    for (end_reason, cut_reason) in [
        (EndReason::Interrupted, "interrupted"),
        (EndReason::DecodeError, "decode_error"),
    ] {
        let secret = "raw-secret-bearing-value";
        let invalid = sse(&[json!({"type": "response.failed", "response": {
            "id": "resp_secret", "status": "failed", "error": {"message": secret}}})]);
        let after_invalid = sse(&[
            created("resp_invalid_resync"),
            created("resp_after_invalid"),
        ]);
        let after_gap = sse(&[created("resp_gap_resync"), created("resp_after_gap")]);
        let s = spec(21, CaptureSource::WireResponse);
        let offset_after_invalid = invalid.len() as u64;
        let offset_after_gap = offset_after_invalid + after_invalid.len() as u64 + 1;
        let (observations, summary) = decode(&[
            s.open(),
            s.segment(
                0,
                0,
                &invalid,
                (1, 2),
                vec![RedactionAuditSpan {
                    kind: "email".into(),
                    start: 0,
                    end: 1,
                }],
            ),
            s.segment(1, offset_after_invalid, &after_invalid, (3, 4), Vec::new()),
            s.gap(),
            s.segment(2, offset_after_gap, &after_gap, (5, 6), Vec::new()),
            s.end(end_reason),
        ]);
        let reasons: Vec<_> = kind(&observations, "unknown")
            .iter()
            .map(|observation| observation.fields["reason"].as_str().unwrap())
            .collect();
        assert_eq!(
            reasons,
            ["invalid_redaction_audit", "gap", cut_reason],
            "{end_reason:?}"
        );
        assert!(
            !serde_json::to_string(&observations)
                .unwrap()
                .contains(secret)
        );
        with_id(&observations, "model_response", "resp_after_invalid");
        with_id(&observations, "model_response", "resp_after_gap");
        assert!(summary.incomplete, "the independent real gap is loss");
    }
}

#[test]
fn redaction_audit_is_attributed_only_to_the_sse_event_it_intersects() {
    let first = sse(&[created("resp_clean")]);
    let second = sse(&[tool_done(
        "ctc_redacted",
        "call_redacted",
        "<redacted:email>",
    )]);
    let body = [first.as_slice(), second.as_slice()].concat();
    let marker_at = body
        .windows("<redacted:email>".len())
        .position(|window| window == b"<redacted:email>")
        .unwrap();
    let audit = vec![RedactionAuditSpan {
        kind: "email".into(),
        start: marker_at as u64,
        end: (marker_at + "<redacted:email>".len()) as u64,
    }];
    let s = spec(1, CaptureSource::WireResponse);
    let records = vec![
        s.open(),
        s.segment(0, 0, &body, (1, 2), audit),
        s.end(EndReason::Complete),
    ];
    let (observations, summary) = decode(&records);
    let clean = with_id(&observations, "model_response", "resp_clean");
    assert!(clean.fields.get("redaction_kinds").is_none());
    let redacted = with_id(&observations, "tool_call", "ctc_redacted");
    assert_eq!(redacted.fields["redaction_kinds"], json!(["email"]));
    assert!(!summary.incomplete);
}

#[test]
fn selection_uses_hint_source_and_content_type_not_host() {
    let message = ws(&created("resp_1"));
    // No hint (or another hint) on the provider host: nothing, however it looks.
    for hint in [None, Some("anthropic-messages"), Some("OpenAI-Responses")] {
        let (observations, summary) =
            decode(&wire_records(&message, CaptureSource::WsServer, hint, &[]));
        assert!(observations.is_empty());
        assert_eq!(summary.unknown_records, 0);
    }
    // A WebSocket message has no content type and is decoded by source and hint.
    let (observations, _) = decode(&wire_records(
        &message,
        CaptureSource::WsServer,
        Some(HINT),
        &[],
    ));
    with_id(&observations, "model_response", "resp_1");
    // An HTTP body is decoded only as its observed content type: an SSE response recorded
    // as text/plain is unknown, not parsed as SSE.
    let plain = spec(1, CaptureSource::WireResponse).content_type(Some("text/plain"));
    let (observations, summary) = decode(&plain.records(&sse(&[created("resp_2")]), &[]));
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].fields["reason"], "unsupported_content_type");
    assert_eq!(observations[0].fields["content_type"], "text/plain");
    assert_eq!(summary.unknown_records, 1);
    // Native and goal streams are never this decoder's, even with the hint.
    for source in [CaptureSource::Native, CaptureSource::Goal] {
        let (observations, _) = decode(&wire_records(&message, source, Some(HINT), &[]));
        assert!(observations.is_empty());
    }
    // The empty upgrade exchange yields nothing.
    let upgrade = spec(1, CaptureSource::WireResponse)
        .content_type(None)
        .status(101);
    let (observations, summary) = decode(&upgrade.records(b"", &[]));
    assert!(observations.is_empty());
    assert_eq!(summary.unknown_records, 0);
}
