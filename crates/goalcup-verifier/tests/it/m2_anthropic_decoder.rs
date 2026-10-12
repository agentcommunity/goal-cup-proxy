//! The bounded Anthropic Messages decoder.
//!
//! The real case is the sanitized Claude Code 2.1.283 exchange of E01
//! (`fixtures/m2-anthropic/`, provenance there). Its expectations are read back from the
//! fixture text itself (event byte ranges, the supplied JSON of each event), never from the
//! decoder under test. Every other case is a small synthetic event stream built here, with
//! fixed observation times; the unobserved provider forms (errors, unknown blocks) exist only
//! in these synthetic cases.

use crate::common as support;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use goalcup_protocol::capture::{
    CaptureMeta, CaptureRecord, CaptureSource, CaptureTime, EndReason, GapReason, MAX_OPEN_STREAMS,
    StreamKey, WireIdentity,
};
use goalcup_protocol::redaction::{RedactionAudit, RedactionAuditSpan};
use goalcup_verifier::capture_analysis::anthropic::{
    AnthropicDecoder, DECODER_ID, DECODER_VERSION, MAX_BUFFERED_BYTES, MAX_RECORD_BYTES,
};
use goalcup_verifier::capture_analysis::api::{DecoderSummary, DecoderV1, Observation};
use serde_json::{Value, json};
use support::wire::{WireSpec, kind as of_kind};

const PRODUCER: &str = "0123456789abcdef0123456789abcdef";
const OTHER_PRODUCER: &str = "fedcba9876543210fedcba9876543210";
const CONN_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CONN_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const CLOCK: &str = "t21-test-clock";
const HINT: &str = "anthropic-messages";
const FIXTURE: &str = include_str!("../fixtures/m2-anthropic/claude-code-2.1.283-e01.jsonl");
const SEGMENT: usize = 64 * 1024;

// ---------------------------------------------------------------------------------------
// Local constructors.
// ---------------------------------------------------------------------------------------

fn at(mono_ns: u64) -> CaptureTime {
    CaptureTime {
        wall: format!("2026-09-28T00:00:00.{mono_ns:09}Z"),
        mono_ns,
        clock_id: CLOCK.to_string(),
    }
}

fn key(stream: u64) -> StreamKey {
    StreamKey {
        producer: PRODUCER.to_string(),
        stream,
    }
}

fn wire(connection: &str, exchange: u64) -> WireIdentity {
    WireIdentity {
        connection: connection.to_string(),
        exchange,
        message_index: None,
    }
}

fn meta(source: CaptureSource, identity: WireIdentity, content_type: &str) -> CaptureMeta {
    let request = source == CaptureSource::WireRequest;
    CaptureMeta {
        wire: Some(identity),
        label: if request {
            "POST /v1/messages"
        } else {
            "200 /v1/messages"
        }
        .to_string(),
        protocol_hint: Some(HINT.to_string()),
        method: request.then(|| "POST".to_string()),
        host: Some("api.anthropic.com".to_string()),
        path: Some("/v1/messages".to_string()),
        status: (!request).then_some(200),
        content_type: Some(content_type.to_string()),
    }
}

fn open_with(stream: StreamKey, source: CaptureSource, metadata: CaptureMeta) -> CaptureRecord {
    WireSpec::new(&stream.producer, stream.stream, source, metadata, at).open_at(1)
}

fn open(stream: u64, source: CaptureSource, metadata: CaptureMeta) -> CaptureRecord {
    open_with(key(stream), source, metadata)
}

fn response_open(stream: u64, identity: WireIdentity) -> CaptureRecord {
    open(
        stream,
        CaptureSource::WireResponse,
        meta(CaptureSource::WireResponse, identity, "text/event-stream"),
    )
}

fn request_open(stream: u64, identity: WireIdentity) -> CaptureRecord {
    open(
        stream,
        CaptureSource::WireRequest,
        meta(CaptureSource::WireRequest, identity, "application/json"),
    )
}

/// Part `i` of every synthetic stream is observed over `[1000(i+1), 1000(i+1)+500]`.
fn part_first(part: u64) -> CaptureTime {
    at(1000 * (part + 1))
}

fn part_last(part: u64) -> CaptureTime {
    at(1000 * (part + 1) + 500)
}

fn segment_at(key: StreamKey, part: u64, offset: u64, data: &[u8]) -> CaptureRecord {
    CaptureRecord::Segment {
        stream: key,
        part,
        source_offset: offset,
        source_len: data.len() as u64,
        first: part_first(part),
        last: part_last(part),
        data_b64: STANDARD.encode(data),
        audit: RedactionAudit { spans: Vec::new() },
    }
}

/// `bytes` cut at `splits` into consecutive parts starting at part zero, offset zero.
fn segments(stream: u64, bytes: &[u8], splits: &[usize]) -> Vec<CaptureRecord> {
    let mut cuts = vec![0];
    cuts.extend(splits.iter().copied().filter(|&s| s > 0 && s < bytes.len()));
    cuts.push(bytes.len());
    cuts.dedup();
    cuts.windows(2)
        .enumerate()
        .map(|(part, w)| segment_at(key(stream), part as u64, w[0] as u64, &bytes[w[0]..w[1]]))
        .collect()
}

fn every(len: usize, step: usize) -> Vec<usize> {
    (step..len).step_by(step).collect()
}

fn end(stream: u64, reason: EndReason) -> CaptureRecord {
    CaptureRecord::End {
        stream: key(stream),
        observed: at(9_000_000),
        reason,
    }
}

/// One complete SSE response stream (stream 2, connection A, exchange 1).
fn sse_records(bytes: &[u8], splits: &[usize]) -> Vec<CaptureRecord> {
    let mut records = vec![response_open(2, wire(CONN_A, 1))];
    records.extend(segments(2, bytes, splits));
    records.push(end(2, EndReason::Complete));
    records
}

fn request_records(stream: u64, identity: WireIdentity, body: &[u8]) -> Vec<CaptureRecord> {
    let mut records = vec![request_open(stream, identity)];
    records.extend(segments(stream, body, &every(body.len(), SEGMENT)));
    records.push(end(stream, EndReason::Complete));
    records
}

fn anthropic() -> Box<dyn DecoderV1> {
    Box::new(AnthropicDecoder::new())
}

fn decode(records: &[CaptureRecord]) -> (Vec<Observation>, DecoderSummary) {
    support::decode::decode_one(anthropic, records)
}

fn kinds(observations: &[Observation]) -> Vec<&str> {
    observations.iter().map(|o| o.kind.as_str()).collect()
}

fn summary(observations: &[Observation], unknown: u64, incomplete: bool) -> DecoderSummary {
    DecoderSummary::new(observations.len() as u64, unknown, incomplete)
        .with_identity(DECODER_ID, DECODER_VERSION)
}

/// Every object key anywhere in `value`.
fn keys_of(value: &Value, into: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                into.push(key.clone());
                keys_of(value, into);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| keys_of(v, into)),
        _ => {}
    }
}

// SSE event text. `serde_json::Value` displays as compact JSON.

fn ev(name: &str, data: Value) -> String {
    format!("event: {name}\ndata: {data}\n\n")
}

fn message_start(id: &str, usage: Value) -> String {
    ev(
        "message_start",
        json!({"type": "message_start", "message": {
            "id": id, "type": "message", "role": "assistant", "model": "claude-test-model",
            "content": [], "stop_reason": null, "usage": usage}}),
    )
}

fn block_start(index: u64, block: Value) -> String {
    ev(
        "content_block_start",
        json!({"type": "content_block_start", "index": index, "content_block": block}),
    )
}

fn tool_start(index: u64, id: &str, name: &str) -> String {
    block_start(
        index,
        json!({"type": "tool_use", "id": id, "name": name, "input": {}}),
    )
}

fn delta(index: u64, delta: Value) -> String {
    ev(
        "content_block_delta",
        json!({"type": "content_block_delta", "index": index, "delta": delta}),
    )
}

fn input_delta(index: u64, fragment: &str) -> String {
    delta(
        index,
        json!({"type": "input_json_delta", "partial_json": fragment}),
    )
}

fn text_delta(index: u64, text: &str) -> String {
    delta(index, json!({"type": "text_delta", "text": text}))
}

fn block_stop(index: u64) -> String {
    ev(
        "content_block_stop",
        json!({"type": "content_block_stop", "index": index}),
    )
}

fn message_delta(stop_reason: &str, usage: Value) -> String {
    ev(
        "message_delta",
        json!({"type": "message_delta", "delta": {"stop_reason": stop_reason, "stop_sequence": null},
               "usage": usage}),
    )
}

fn message_stop() -> String {
    ev("message_stop", json!({"type": "message_stop"}))
}

/// A short complete message: one tool call assembled from two fragments, then a text block.
fn text_message(id: &str, text: &str) -> String {
    [
        message_start(id, json!({"input_tokens": 3, "output_tokens": 1})),
        block_start(0, json!({"type": "text", "text": ""})),
        text_delta(0, text),
        block_stop(0),
        message_delta("end_turn", json!({"output_tokens": 5})),
        message_stop(),
    ]
    .concat()
}

// ---------------------------------------------------------------------------------------
// The sanitized real fixture.
// ---------------------------------------------------------------------------------------

fn fixture_records() -> Vec<CaptureRecord> {
    FIXTURE
        .lines()
        .map(|line| {
            let mut value: Value = serde_json::from_str(line).unwrap();
            let object = value.as_object_mut().unwrap();
            if let Some(text) = object.remove("text") {
                let bytes = text.as_str().unwrap().as_bytes();
                object.insert("data_b64".into(), json!(STANDARD.encode(bytes)));
            }
            serde_json::from_value(value).unwrap()
        })
        .collect()
}

struct Part {
    text: String,
    first: CaptureTime,
    last: CaptureTime,
}

/// The retained parts of one fixture stream, read back from the records.
fn fixture_parts(records: &[CaptureRecord], stream: u64) -> Vec<Part> {
    records
        .iter()
        .filter_map(|record| match record {
            CaptureRecord::Segment {
                stream: key,
                data_b64,
                first,
                last,
                ..
            } if key.stream == stream => Some(Part {
                text: String::from_utf8(STANDARD.decode(data_b64).unwrap()).unwrap(),
                first: first.clone(),
                last: last.clone(),
            }),
            _ => None,
        })
        .collect()
}

fn fixture_wire(records: &[CaptureRecord], stream: u64) -> Value {
    records
        .iter()
        .find_map(|record| match record {
            CaptureRecord::Open {
                stream: key,
                metadata,
                ..
            } if key.stream == stream => Some(json!(metadata.wire)),
            _ => None,
        })
        .unwrap()
}

fn part_at(parts: &[Part], offset: usize) -> u64 {
    let mut end = 0;
    for (index, part) in parts.iter().enumerate() {
        end += part.text.len();
        if offset < end {
            return index as u64;
        }
    }
    panic!("offset past the stream");
}

/// The byte range of the SSE event containing `needle`: its first byte and the blank line's
/// newline that dispatches it (inclusive), and the JSON of its data line.
fn event_at(body: &str, needle: &str) -> (usize, usize, Value) {
    let at = body.find(needle).unwrap_or_else(|| panic!("no {needle}"));
    let start = body[..at].rfind("event: ").unwrap();
    let end = at + body[at..].find("\n\n").unwrap() + 1;
    let data = body[start..end]
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .unwrap();
    (start, end, serde_json::from_str(data).unwrap())
}

fn expect_span(observation: &Observation, parts: &[Part], stream: u64, first: usize, last: usize) {
    let (first, last) = (part_at(parts, first), part_at(parts, last));
    assert_eq!(observation.source.stream.stream, stream, "{observation:?}");
    assert_eq!(
        (observation.source.first_part, observation.source.last_part),
        (first, last),
        "{observation:?}"
    );
    assert_eq!(observation.observed.first, parts[first as usize].first);
    assert_eq!(observation.observed.last, parts[last as usize].last);
}

#[test]
fn real_message_tool_and_usage_have_source_refs() {
    let records = fixture_records();
    let (observations, got) = decode(&records);
    assert_eq!(got, summary(&observations, 0, false));
    assert_eq!(
        kinds(&observations),
        [
            // request 1 (thread create; ten text blocks, no tool result)
            "model",
            "model_request",
            // response 1: two parallel tool_use blocks
            "model",
            "usage",
            "tool_call",
            "tool_call",
            "usage",
            "outcome",
            "model_response",
            // request 2 (thread continue): only the two tool results
            "model",
            "model_request",
            "tool_result",
            "tool_result",
            // response 2
            "model",
            "usage",
            "usage",
            "outcome",
            "model_response",
        ]
    );
    for observation in &observations {
        let stream = observation.source.stream.stream;
        assert_eq!(observation.fields["wire"], fixture_wire(&records, stream));
        if observation.kind == "model" {
            assert_eq!(observation.fields["model"], "claude-haiku-4-5-20251001");
        }
    }

    // Response 1: every expectation below comes from the fixture's own event text.
    let parts = fixture_parts(&records, 2);
    let body: String = parts.iter().map(|p| p.text.as_str()).collect();
    let (start, start_end, supplied_start) = event_at(&body, "\"type\":\"message_start\"");
    let message_id = supplied_start["message"]["id"].as_str().unwrap();
    assert_eq!(message_id, "msg_PLACEHOLDER0000000000001");
    let (_, stop_end, _) = event_at(&body, "\"type\":\"message_stop\"");

    let calls = of_kind(&observations, "tool_call");
    for (call, index, name, path_input) in [
        (
            calls[0],
            1,
            "Write",
            json!({"file_path": "/Users/Shared/goalcup-m2-e01/claude-20260928T032648Z/workspace/result.txt", "content": "m2-capture-ok"}),
        ),
        (
            calls[1],
            2,
            "Read",
            json!({"file_path": "/Users/Shared/goalcup-m2-e01/claude-20260928T032648Z/workspace/result.txt"}),
        ),
    ] {
        let (first, _, supplied) = event_at(
            &body,
            &format!("\"content_block_start\",\"index\":{index},"),
        );
        let (_, last, _) = event_at(&body, &format!("\"content_block_stop\",\"index\":{index} "));
        let fragments: String = body
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .map(|data| serde_json::from_str::<Value>(data).unwrap())
            .filter(|v| v["index"] == index && v["delta"]["type"] == "input_json_delta")
            .map(|v| v["delta"]["partial_json"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            call.supplied_id.as_deref(),
            supplied["content_block"]["id"].as_str()
        );
        assert_eq!(call.fields["name"], name);
        assert_eq!(call.fields["message_id"], message_id);
        assert_eq!(call.fields["index"], index);
        assert_eq!(call.fields["input_status"], "complete");
        assert_eq!(call.fields["input"], path_input);
        assert_eq!(
            call.fields["input"],
            serde_json::from_str::<Value>(&fragments).unwrap()
        );
        assert_eq!(call.fields["input_bytes"], fragments.len());
        expect_span(call, &parts, 2, first, last);
    }
    // The two calls start and stop in different retained parts.
    assert_eq!(
        (calls[0].source.first_part, calls[0].source.last_part),
        (2, 6)
    );
    assert_eq!(
        (calls[1].source.first_part, calls[1].source.last_part),
        (6, 7)
    );

    let usage = of_kind(&observations, "usage");
    let (delta_start, delta_end, supplied_delta) = event_at(&body, "\"type\":\"message_delta\"");
    assert_eq!(usage[0].fields["scope"], "message_start");
    assert_eq!(usage[0].fields["reading"], "initial_snapshot");
    assert_eq!(usage[0].fields["usage"], supplied_start["message"]["usage"]);
    assert_eq!(usage[0].supplied_id.as_deref(), Some(message_id));
    expect_span(usage[0], &parts, 2, start, start_end);
    assert_eq!(usage[1].fields["scope"], "message_delta");
    assert_eq!(usage[1].fields["reading"], "cumulative_snapshot");
    assert_eq!(usage[1].fields["usage"], supplied_delta["usage"]);
    expect_span(usage[1], &parts, 2, delta_start, delta_end);
    // Supplied values, not sums: the start snapshot says 6 output tokens, the delta 356.
    assert_eq!(usage[0].fields["usage"]["output_tokens"], 6);
    assert_eq!(usage[1].fields["usage"]["output_tokens"], 356);

    let outcomes = of_kind(&observations, "outcome");
    assert_eq!(outcomes[0].fields["stop_reason"], "tool_use");
    assert_eq!(outcomes[0].fields["scope"], "provider_message");
    assert_eq!(outcomes[1].fields["stop_reason"], "end_turn");
    let responses = of_kind(&observations, "model_response");
    assert_eq!(responses[0].supplied_id.as_deref(), Some(message_id));
    assert_eq!(
        responses[0].fields["block_counts"],
        json!({"text": 0, "thinking": 1, "tool_use": 2, "unsupported": 0})
    );
    assert_eq!(responses[0].fields["message_complete"], true);
    assert_eq!(responses[0].fields["losses"], json!([]));
    assert_eq!(responses[0].fields["stream_end"], "complete");
    assert_eq!(responses[0].fields["http_status"], 200);
    expect_span(responses[0], &parts, 2, start, stop_end);
    assert_eq!(
        responses[1].fields["block_counts"],
        json!({"text": 1, "thinking": 1, "tool_use": 0, "unsupported": 0})
    );

    // Request 1 parsed although redaction replaced two e-mail addresses in its first part.
    let requests = of_kind(&observations, "model_request");
    let request_parts = fixture_parts(&records, 1);
    let request_len: usize = request_parts.iter().map(|p| p.text.len()).sum();
    expect_span(requests[0], &request_parts, 1, 0, request_len - 1);
    assert_eq!(requests[0].fields["thread"], json!({"type": "create"}));
    assert_eq!(requests[0].fields["message_count"], 1);
    assert_eq!(
        requests[0].fields["content_block_counts"],
        json!({"text": 10, "tool_result": 0, "unsupported": 0})
    );
    assert_eq!(requests[0].fields["tool_definition_count"], 12);
    assert_eq!(requests[0].fields["stream"], true);
    assert_eq!(requests[0].fields["max_tokens"], 32000);
    assert_eq!(requests[0].fields["endpoint"], "/v1/messages");

    // Request 2 continues response 1's message by supplied ID; nothing is rebuilt from it.
    assert_eq!(
        requests[1].fields["thread"],
        json!({"type": "continue", "previous_message_id": message_id})
    );
    assert_eq!(requests[1].fields["message_count"], 1);
    assert_eq!(
        requests[1].fields["content_block_counts"],
        json!({"text": 0, "tool_result": 2, "unsupported": 0})
    );
    assert!(requests[1].fields.get("tool_definition_count").is_none());

    // The tool results are the provider-bound copies, verbatim: the Read result carries the
    // harness-appended system-reminder that the native transcript records separately.
    let results = of_kind(&observations, "tool_result");
    let request_2 = &fixture_parts(&records, 3)[0].text;
    let supplied: Value = serde_json::from_str(request_2).unwrap();
    for (result, block) in results
        .iter()
        .zip(supplied["messages"][0]["content"].as_array().unwrap())
    {
        assert_eq!(result.supplied_id.as_deref(), block["tool_use_id"].as_str());
        assert_eq!(result.fields["content"], block["content"]);
        assert_eq!(result.source.stream.stream, 3);
        assert_eq!((result.source.first_part, result.source.last_part), (0, 0));
    }
    assert_eq!(results[0].supplied_id, calls[0].supplied_id);
    assert_eq!(results[1].supplied_id, calls[1].supplied_id);
    assert!(results[1].fields["content"].as_str().unwrap().ends_with(
        "<system-reminder>\n<total_tokens>14978308 tokens left</total_tokens>\n</system-reminder>"
    ));
}

#[test]
fn repeat_decode_is_byte_identical() {
    let records = fixture_records();
    let first = serde_json::to_vec(&decode(&records)).unwrap();
    let second = serde_json::to_vec(&decode(&records)).unwrap();
    assert_eq!(first, second);
    // A synthetic stream with every map-bearing observation kind decodes identically too.
    let stream = sse_records(text_message("msg_R", "same").as_bytes(), &[7, 90]);
    assert_eq!(
        serde_json::to_vec(&decode(&stream)).unwrap(),
        serde_json::to_vec(&decode(&stream)).unwrap()
    );
}

// ---------------------------------------------------------------------------------------
// Framing and intervals.
// ---------------------------------------------------------------------------------------

/// A compact stream exercising the framing rules: a comment block, a CRLF event whose data is
/// split over two `data:` lines, one-space stripping, and a tool input from two fragments.
fn compact_stream() -> String {
    [
        ": keep-alive comment\n\n".to_string(),
        message_start("msg_C", json!({"input_tokens": 3, "output_tokens": 1})),
        tool_start(0, "toolu_C", "Write"),
        input_delta(0, "{\"path\": \"a"),
        input_delta(0, "b\"}"),
        block_stop(0),
        "event: message_delta\r\ndata: {\"type\":\"message_delta\",\r\ndata:\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":9}}\r\n\r\n".to_string(),
        message_stop(),
    ]
    .concat()
}

/// Fields and IDs only: what must not depend on where the capture cut the bytes.
fn semantics(observations: &[Observation]) -> Vec<(String, Option<String>, Value)> {
    observations
        .iter()
        .map(|o| (o.kind.clone(), o.supplied_id.clone(), o.fields.clone()))
        .collect()
}

#[test]
fn multiline_sse_split_at_every_boundary() {
    let text = compact_stream();
    let bytes = text.as_bytes();
    let (whole, whole_summary) = decode(&sse_records(bytes, &[]));
    assert_eq!(whole_summary, summary(&whole, 0, false));
    assert_eq!(
        kinds(&whole),
        [
            "model",
            "usage",
            "tool_call",
            "usage",
            "outcome",
            "model_response"
        ]
    );
    let call = &whole[2];
    assert_eq!(call.supplied_id.as_deref(), Some("toolu_C"));
    assert_eq!(call.fields["input"], json!({"path": "ab"}));
    // The two data lines join with LF into one JSON document.
    assert_eq!(whole[3].fields["usage"], json!({"output_tokens": 9}));
    assert_eq!(whole[4].fields["stop_reason"], "tool_use");

    // Independent byte ranges: first byte of an event and the byte that dispatches it.
    let tool_first = text.find("event: content_block_start").unwrap();
    let tool_last = text.find("event: message_delta").unwrap() - 1;
    let delta_first = text.find("event: message_delta").unwrap();
    let delta_last = text.find("event: message_stop").unwrap() - 2; // "\r" of "\r\n\r\n"
    let message_first = text.find("event: message_start").unwrap();
    let message_last = bytes.len() - 1;
    let ranges = [
        (2, tool_first, tool_last),
        (3, delta_first, delta_last),
        (5, message_first, message_last),
    ];

    for split in 1..bytes.len() {
        let (split_obs, split_summary) = decode(&sse_records(bytes, &[split]));
        assert_eq!(semantics(&split_obs), semantics(&whole), "split at {split}");
        assert_eq!(split_summary, whole_summary, "split at {split}");
        for (index, first, last) in ranges {
            let part = |offset: usize| u64::from(offset >= split);
            let source = &split_obs[index].source;
            assert_eq!(
                (source.first_part, source.last_part),
                (part(first), part(last)),
                "split at {split}, observation {index}"
            );
        }
    }

    // Every byte its own part: parts are byte offsets, intervals their part times.
    let (bytewise, _) = decode(&sse_records(bytes, &every(bytes.len(), 1)));
    assert_eq!(semantics(&bytewise), semantics(&whole));
    for (index, first, last) in ranges {
        let observation = &bytewise[index];
        assert_eq!(
            (observation.source.first_part, observation.source.last_part),
            (first as u64, last as u64)
        );
        assert_eq!(observation.observed.first, part_first(first as u64));
        assert_eq!(observation.observed.last, part_last(last as u64));
    }
}

#[test]
fn anthropic_event_interval_spans_contributing_segments() {
    let producer_clock = |mono_ns: u64| CaptureTime {
        wall: format!("2026-09-28T01:00:00.{mono_ns:09}Z"),
        mono_ns,
        clock_id: "producer-epoch-clock".to_string(),
    };
    let start = message_start("msg_T", json!({"input_tokens": 1, "output_tokens": 1}));
    let body = format!("{start}{}", ev("ping", json!({"type": "ping"})));
    let bytes = body.as_bytes();
    // message_start spans parts 0..2; part 2 also holds the ping.
    let cuts = [0, 20, 60, bytes.len()];
    let times = [(100, 110), (200, 210), (300, 310)];
    assert!(start.len() > cuts[2]);
    let mut records = vec![response_open(2, wire(CONN_A, 1))];
    for part in 0..3 {
        let data = &bytes[cuts[part]..cuts[part + 1]];
        records.push(CaptureRecord::Segment {
            stream: key(2),
            part: part as u64,
            source_offset: cuts[part] as u64,
            source_len: data.len() as u64,
            first: producer_clock(times[part].0),
            last: producer_clock(times[part].1),
            data_b64: STANDARD.encode(data),
            audit: RedactionAudit { spans: Vec::new() },
        });
    }
    let (observations, _) = decode(&records);
    let model = of_kind(&observations, "model")[0];
    assert_eq!((model.source.first_part, model.source.last_part), (0, 2));
    assert_eq!(model.observed.first, producer_clock(100));
    assert_eq!(model.observed.last, producer_clock(310));
    let usage = of_kind(&observations, "usage")[0];
    assert_eq!(usage.observed, model.observed);
    // The provider supplies no event time: nothing time-like is reported in the fields.
    for observation in &observations {
        let mut keys = Vec::new();
        keys_of(&observation.fields, &mut keys);
        assert!(
            !keys
                .iter()
                .any(|k| k.contains("time") || k.contains("latency")),
            "{keys:?}"
        );
    }
    // The unterminated stream still reports its message, over every part it reached.
    let response = of_kind(&observations, "model_response")[0];
    assert_eq!(response.fields["stream_end"], "unterminated");
    assert_eq!(response.observed.first, producer_clock(100));
    assert_eq!(response.observed.last, producer_clock(310));
}

// ---------------------------------------------------------------------------------------
// Errors, outcomes and usage.
// ---------------------------------------------------------------------------------------

#[test]
fn anthropic_error_and_outcome_keep_safe_source_fields() {
    let secret_text = "Overloaded: upstream detail 7f3a secret-ish text";
    let stream_error = [
        message_start("msg_E", json!({"input_tokens": 2, "output_tokens": 1})),
        block_start(0, json!({"type": "text", "text": ""})),
        text_delta(0, "partial"),
        ev(
            "error",
            json!({"type": "error", "error": {"type": "overloaded_error", "message": secret_text},
                   "request_id": "req_E1"}),
        ),
    ]
    .concat();
    let unsafe_error = ev(
        "error",
        json!({"type": "error", "error": {"type": "bad type\nwith lines", "message": secret_text}}),
    );
    let unsafe_stop = [
        message_start("msg_U", json!({"output_tokens": 1})),
        message_delta("end turn!", json!({"output_tokens": 2})),
        message_stop(),
    ]
    .concat();
    let document =
        json!({"type": "error", "error": {"type": "rate_limit_error", "message": secret_text},
                          "request_id": "req_J1"})
        .to_string();

    let mut records = sse_records(stream_error.as_bytes(), &[]);
    records.push(response_open(3, wire(CONN_A, 2)));
    records.extend(segments(3, unsafe_error.as_bytes(), &[]));
    records.push(end(3, EndReason::Complete));
    records.push(response_open(4, wire(CONN_A, 3)));
    records.extend(segments(4, unsafe_stop.as_bytes(), &[]));
    records.push(end(4, EndReason::Complete));
    let mut error_meta = meta(
        CaptureSource::WireResponse,
        wire(CONN_B, 1),
        "application/json",
    );
    error_meta.status = Some(429);
    records.push(open(5, CaptureSource::WireResponse, error_meta));
    records.extend(segments(5, document.as_bytes(), &[]));
    records.push(end(5, EndReason::Complete));

    let (observations, got) = decode(&records);
    let errors = of_kind(&observations, "error");
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].supplied_id.as_deref(), Some("req_E1"));
    assert_eq!(
        errors[0].fields,
        json!({"scope": "stream_event", "error_type": "overloaded_error", "http_status": 200,
               "source": "wire_response", "wire": json!(wire(CONN_A, 1))})
    );
    assert_eq!(errors[1].supplied_id.as_deref(), Some("req_J1"));
    assert_eq!(
        errors[1].fields,
        json!({"scope": "http_response", "error_type": "rate_limit_error", "http_status": 429,
               "source": "wire_response", "wire": json!(wire(CONN_B, 1))})
    );
    // No message text, header or arbitrary dump leaves the decoder.
    let all = serde_json::to_string(&observations).unwrap();
    assert!(!all.contains("upstream detail") && !all.contains("with lines"));
    // An error type outside the safe token form is an explicit unknown, not an error.
    let unknown = of_kind(&observations, "unknown");
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].fields["reason"], "unsupported_error_form");
    assert_eq!(unknown[0].source.stream.stream, 3);

    // The errored message never stopped: no outcome for it, and it stays incomplete.
    let responses = of_kind(&observations, "model_response");
    assert_eq!(responses[0].supplied_id.as_deref(), Some("msg_E"));
    assert_eq!(responses[0].fields["message_complete"], false);
    assert_eq!(
        responses[0].fields["losses"],
        json!(["error_event", "missing_message_stop", "unstopped_block"])
    );
    let outcomes = of_kind(&observations, "outcome");
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].supplied_id.as_deref(), Some("msg_U"));
    // An unsafe supplied stop reason is withheld and named, never echoed.
    assert!(outcomes[0].fields.get("stop_reason").is_none());
    assert_eq!(responses[1].fields["withheld"], json!(["stop_reason"]));
    assert_eq!(got, summary(&observations, 1, true));
}

#[test]
fn message_stop_outcome_is_not_task_success_or_exact_latency() {
    let text = text_message("msg_S", "done");
    let (observations, _) = decode(&sse_records(text.as_bytes(), &[]));
    let outcome = of_kind(&observations, "outcome")[0];
    assert_eq!(outcome.supplied_id.as_deref(), Some("msg_S"));
    assert_eq!(
        outcome.fields,
        json!({"scope": "provider_message", "event": "message_stop", "stop_reason": "end_turn",
               "source": "wire_response", "wire": json!(wire(CONN_A, 1))})
    );
    // The interval is the retained segment's, not an invented instant.
    assert_eq!(outcome.observed.first, part_first(0));
    assert_eq!(outcome.observed.last, part_last(0));
    for observation in &observations {
        let mut keys = Vec::new();
        keys_of(&observation.fields, &mut keys);
        for word in [
            "task",
            "success",
            "latency",
            "duration",
            "elapsed",
            "timestamp",
        ] {
            assert!(!keys.iter().any(|k| k.contains(word)), "{word} in {keys:?}");
        }
    }
    let response = of_kind(&observations, "model_response")[0];
    assert_eq!(response.fields["message_complete"], true);
    assert_eq!(response.fields["stream_end"], "complete");
}

#[test]
fn message_start_and_delta_usage_are_not_blindly_summed() {
    let text = [
        message_start("msg_U", json!({"input_tokens": 12, "output_tokens": 1})),
        message_delta("end_turn", json!({"output_tokens": 40})),
        message_stop(),
    ]
    .concat();
    let (observations, _) = decode(&sse_records(text.as_bytes(), &[]));
    let usage = of_kind(&observations, "usage");
    assert_eq!(usage.len(), 2);
    assert_eq!(usage[0].fields["scope"], "message_start");
    assert_eq!(usage[0].fields["reading"], "initial_snapshot");
    assert_eq!(
        usage[0].fields["usage"],
        json!({"input_tokens": 12, "output_tokens": 1})
    );
    assert_eq!(usage[1].fields["scope"], "message_delta");
    assert_eq!(usage[1].fields["reading"], "cumulative_snapshot");
    // Missing input tokens stay missing: not zero, not carried over, not added.
    assert_eq!(usage[1].fields["usage"], json!({"output_tokens": 40}));
    for observation in &observations {
        let mut keys = Vec::new();
        keys_of(&observation.fields, &mut keys);
        assert!(
            !keys
                .iter()
                .any(|k| k.contains("total") || k.contains("sum")),
            "{keys:?}"
        );
        if observation.kind != "usage" {
            assert!(!keys.iter().any(|k| k.contains("token")), "{keys:?}");
        }
    }
}

// ---------------------------------------------------------------------------------------
// Identity and assembly.
// ---------------------------------------------------------------------------------------

#[test]
fn typed_exchange_identity_survives_equal_labels() {
    // Two exchanges on two connections with identical labels and byte-identical bodies.
    let request = br#"{"model":"claude-test-model","messages":[{"role":"user","content":"hi"}],"stream":true}"#;
    let response = text_message("msg_same", "same words");
    let mut records = vec![
        request_open(1, wire(CONN_A, 1)),
        request_open(3, wire(CONN_B, 1)),
        response_open(2, wire(CONN_A, 1)),
        response_open(4, wire(CONN_B, 1)),
    ];
    let a = segments(1, request, &[10]);
    let b = segments(3, request, &[10]);
    records.extend([a[0].clone(), b[0].clone(), b[1].clone(), a[1].clone()]);
    let cuts = every(response.len(), 40);
    for (x, y) in segments(2, response.as_bytes(), &cuts)
        .into_iter()
        .zip(segments(4, response.as_bytes(), &cuts))
    {
        records.extend([y, x]);
    }
    records.extend([end(3, EndReason::Complete), end(1, EndReason::Complete)]);
    records.extend([end(4, EndReason::Complete), end(2, EndReason::Complete)]);
    let (observations, got) = decode(&records);
    assert_eq!(got, summary(&observations, 0, false));
    for (stream, identity) in [
        (1, wire(CONN_A, 1)),
        (2, wire(CONN_A, 1)),
        (3, wire(CONN_B, 1)),
        (4, wire(CONN_B, 1)),
    ] {
        let own: Vec<&Observation> = observations
            .iter()
            .filter(|o| o.source.stream.stream == stream)
            .collect();
        let expected = if stream % 2 == 1 { 2 } else { 5 };
        assert_eq!(own.len(), expected, "stream {stream}");
        for observation in own {
            assert_eq!(observation.fields["wire"], json!(identity));
        }
    }
    let responses = of_kind(&observations, "model_response");
    assert_eq!(responses.len(), 2);
    assert_ne!(responses[0].source.stream, responses[1].source.stream);
}

#[test]
fn tool_input_delta_fragments_keep_supplied_block_identity() {
    let text = [
        message_start("msg_T", json!({"output_tokens": 1})),
        tool_start(1, "toolu_one", "Write"),
        tool_start(2, "toolu_two", "Read"),
        input_delta(1, "{\"a\":"),
        input_delta(2, "{\"b\":"),
        input_delta(7, "{\"lost\":1}"),
        input_delta(1, "1}"),
        input_delta(2, "2}"),
        block_stop(2),
        block_stop(1),
        tool_start(3, "toolu_empty", "ListAgents"),
        block_stop(3),
        message_delta("tool_use", json!({"output_tokens": 30})),
        message_stop(),
    ]
    .concat();
    let (observations, got) = decode(&sse_records(text.as_bytes(), &every(text.len(), 17)));
    let calls = of_kind(&observations, "tool_call");
    let got_calls: Vec<(Option<&str>, &Value, &Value, &Value, &Value)> = calls
        .iter()
        .map(|c| {
            (
                c.supplied_id.as_deref(),
                &c.fields["index"],
                &c.fields["name"],
                &c.fields["input"],
                &c.fields["input_bytes"],
            )
        })
        .collect();
    assert_eq!(
        got_calls,
        [
            (
                Some("toolu_two"),
                &json!(2),
                &json!("Read"),
                &json!({"b": 2}),
                &json!(7)
            ),
            (
                Some("toolu_one"),
                &json!(1),
                &json!("Write"),
                &json!({"a": 1}),
                &json!(7)
            ),
            (
                Some("toolu_empty"),
                &json!(3),
                &json!("ListAgents"),
                &json!({}),
                &json!(0)
            ),
        ]
    );
    assert!(calls.iter().all(|c| c.fields["input_status"] == "complete"));
    assert!(calls.iter().all(|c| c.fields["message_id"] == "msg_T"));
    let unknown = of_kind(&observations, "unknown");
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].fields["reason"], "delta_without_block");
    assert_eq!(got, summary(&observations, 1, true));
    assert_eq!(
        of_kind(&observations, "model_response")[0].fields["message_complete"],
        false
    );
}

#[test]
fn repeated_context_is_not_a_new_human_message() {
    let first = json!({"model": "claude-test-model", "stream": true,
        "messages": [{"role": "user", "content": "Please do X"}]});
    let replay = json!({"model": "claude-test-model", "stream": true, "messages": [
        {"role": "user", "content": "Please do X"},
        {"role": "assistant", "content": [{"type": "text", "text": "ok"}]},
        {"role": "user", "content": [{"type": "text", "text": "Please do X"}]}]});
    let mut records = request_records(1, wire(CONN_A, 1), first.to_string().as_bytes());
    records.extend(request_records(
        3,
        wire(CONN_A, 2),
        replay.to_string().as_bytes(),
    ));
    let (observations, got) = decode(&records);
    assert_eq!(
        kinds(&observations),
        ["model", "model_request", "model", "model_request"]
    );
    assert_eq!(got, summary(&observations, 0, false));
    let replayed = of_kind(&observations, "model_request")[1];
    assert_eq!(replayed.fields["message_count"], 3);
    assert_eq!(
        replayed.fields["content_block_counts"],
        json!({"text": 3, "tool_result": 0, "unsupported": 0})
    );
    for observation in &observations {
        assert_eq!(observation.supplied_id, None);
        let mut keys = Vec::new();
        keys_of(&observation.fields, &mut keys);
        for word in ["human", "assist", "author", "intervention", "role"] {
            assert!(!keys.iter().any(|k| k.contains(word)), "{word} in {keys:?}");
        }
    }
}

#[test]
fn unknown_content_block_keeps_known_fields() {
    let text = [
        message_start("msg_K", json!({"input_tokens": 4, "output_tokens": 1})),
        block_start(
            0,
            json!({"type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search", "input": {}}),
        ),
        input_delta(0, "{\"query\": \"x\"}"),
        block_stop(0),
        ev("message_annotation", json!({"type": "message_annotation", "note": 1})),
        block_start(1, json!({"type": "text", "text": ""})),
        text_delta(1, "answer"),
        block_stop(1),
        message_delta("end_turn", json!({"output_tokens": 11})),
        message_stop(),
    ]
    .concat();
    let (observations, got) = decode(&sse_records(text.as_bytes(), &[]));
    assert_eq!(
        kinds(&observations),
        [
            "model",
            "usage",
            "unknown",
            "unknown",
            "usage",
            "outcome",
            "model_response"
        ]
    );
    assert_eq!(observations[0].fields["model"], "claude-test-model");
    assert_eq!(
        observations[1].fields["usage"],
        json!({"input_tokens": 4, "output_tokens": 1})
    );
    assert_eq!(
        observations[2].fields["reason"],
        "unsupported_content_block"
    );
    assert_eq!(observations[2].fields["block_type"], "server_tool_use");
    assert_eq!(observations[2].fields["index"], 0);
    assert_eq!(observations[3].fields["reason"], "unsupported_event");
    assert_eq!(observations[3].fields["event_type"], "message_annotation");
    let response = &observations[6];
    assert_eq!(
        response.fields["block_counts"],
        json!({"text": 1, "thinking": 0, "tool_use": 0, "unsupported": 1})
    );
    assert_eq!(response.fields["message_complete"], true);
    assert_eq!(got, summary(&observations, 2, false));
}

#[test]
fn interleaved_messages_do_not_merge_equal_text() {
    let one = text_message("msg_one", "identical text");
    let two = text_message("msg_two", "identical text");
    let mut records = vec![
        response_open(2, wire(CONN_A, 1)),
        response_open(4, wire(CONN_B, 1)),
    ];
    let cuts_one = every(one.len(), 25);
    let cuts_two = every(two.len(), 25);
    for (x, y) in segments(2, one.as_bytes(), &cuts_one)
        .into_iter()
        .zip(segments(4, two.as_bytes(), &cuts_two))
    {
        records.extend([x, y]);
    }
    records.extend([end(4, EndReason::Complete), end(2, EndReason::Complete)]);
    let (observations, got) = decode(&records);
    assert_eq!(got, summary(&observations, 0, false));
    let responses = of_kind(&observations, "model_response");
    let by_id: Vec<(Option<&str>, u64, &Value)> = responses
        .iter()
        .map(|r| {
            (
                r.supplied_id.as_deref(),
                r.source.stream.stream,
                &r.fields["block_counts"],
            )
        })
        .collect();
    let counts = json!({"text": 1, "thinking": 0, "tool_use": 0, "unsupported": 0});
    assert_eq!(
        by_id,
        [(Some("msg_two"), 4, &counts), (Some("msg_one"), 2, &counts)]
    );
    let outcomes: Vec<(Option<&str>, u64)> = of_kind(&observations, "outcome")
        .iter()
        .map(|o| (o.supplied_id.as_deref(), o.source.stream.stream))
        .collect();
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.contains(&(Some("msg_one"), 2)) && outcomes.contains(&(Some("msg_two"), 4)));
}

// ---------------------------------------------------------------------------------------
// Loss, malformed input and bounds.
// ---------------------------------------------------------------------------------------

#[test]
fn gap_before_message_stop_remains_incomplete() {
    let before = [
        message_start("msg_G", json!({"input_tokens": 5, "output_tokens": 1})),
        tool_start(0, "toolu_G", "Write"),
        input_delta(0, "{\"path\": \"x"),
    ]
    .concat();
    // The gap cuts one event after its first line. The line that resumes it is well formed
    // on its own, but its event lost its beginning: it is skipped up to the next blank line.
    let lost_event = input_delta(0, "yz");
    let (lost_head, lost_tail) = lost_event.split_at(lost_event.find("data: ").unwrap());
    let part0 = format!("{before}{lost_head}");
    let after = [
        lost_tail.to_string(),
        input_delta(0, "\"}"),
        block_stop(0),
        message_delta("tool_use", json!({"output_tokens": 20})),
        message_stop(),
    ]
    .concat();
    let lost_bytes = 1000;
    let records = vec![
        response_open(2, wire(CONN_A, 1)),
        segment_at(key(2), 0, 0, part0.as_bytes()),
        CaptureRecord::Gap {
            stream: Some(key(2)),
            observed: at(1700),
            reason: GapReason::QueueFull,
            lost_bytes: Some(lost_bytes),
        },
        segment_at(key(2), 1, part0.len() as u64 + lost_bytes, after.as_bytes()),
        end(2, EndReason::Complete),
    ];
    let (observations, got) = decode(&records);
    assert_eq!(
        kinds(&observations),
        [
            "model",
            "usage",
            "unknown",
            "tool_call",
            "usage",
            "outcome",
            "model_response"
        ]
    );
    // Earlier observations stand; the bytes of the event cut by the gap are named.
    assert_eq!(observations[0].fields["model"], "claude-test-model");
    assert_eq!(observations[2].fields["reason"], "partial_event");
    assert_eq!(
        (
            observations[2].source.first_part,
            observations[2].source.last_part
        ),
        (0, 0)
    );
    // The tool input lost a fragment: no input is claimed.
    let call = &observations[3];
    assert_eq!(call.supplied_id.as_deref(), Some("toolu_G"));
    assert_eq!(call.fields["input_status"], "incomplete");
    assert!(call.fields.get("input").is_none());
    assert_eq!(
        call.fields["input_bytes"],
        "{\"path\": \"x".len() + "\"}".len()
    );
    assert_eq!((call.source.first_part, call.source.last_part), (0, 1));
    // The provider's later stop is still observed, but cannot complete the message.
    assert_eq!(observations[5].fields["stop_reason"], "tool_use");
    let response = &observations[6];
    assert_eq!(response.fields["message_complete"], false);
    assert_eq!(response.fields["losses"], json!(["gap", "partial_event"]));
    assert_eq!(response.fields["stream_end"], "complete");
    assert_eq!(got, summary(&observations, 1, true));

    // An offset jump with no recorded gap is the same loss, named for what it is.
    let unrecorded: Vec<CaptureRecord> = records
        .into_iter()
        .filter(|record| !matches!(record, CaptureRecord::Gap { .. }))
        .collect();
    let (observations, got) = decode(&unrecorded);
    let response = of_kind(&observations, "model_response")[0];
    assert_eq!(
        response.fields["losses"],
        json!(["discontinuity", "partial_event"])
    );
    assert_eq!(got, summary(&observations, 1, true));
}

#[test]
fn invalid_audit_discards_raw_event_and_keeps_message_loss_through_resync() {
    let before = [
        message_start(
            "msg_invalid",
            json!({"input_tokens": 5, "output_tokens": 1}),
        ),
        block_start(0, json!({"type": "text", "text": ""})),
        text_delta(0, "before"),
    ]
    .concat();
    let secret = "raw-secret-bearing-value";
    let invalid_request_id = "req_invalid_audit_probe";
    let invalid_error_type = "invalid_audit_probe";
    let invalid = ev(
        "error",
        json!({"type": "error", "error": {
            "type": invalid_error_type, "message": secret
        }, "request_id": invalid_request_id}),
    );
    let after = [
        message_delta("sacrificial_stop", json!({"output_tokens": 999})),
        text_delta(0, "after-invalid"),
        block_stop(0),
        message_delta("end_turn", json!({"output_tokens": 5})),
        message_stop(),
    ]
    .concat();
    let offset = before.len() as u64;
    let mut invalid_record = segment_at(key(2), 1, offset, invalid.as_bytes());
    let CaptureRecord::Segment { audit, .. } = &mut invalid_record else {
        unreachable!()
    };
    audit.spans.push(RedactionAuditSpan {
        kind: "email".into(),
        start: offset,
        end: offset + 1,
    });
    let records = vec![
        response_open(2, wire(CONN_A, 1)),
        segment_at(key(2), 0, 0, before.as_bytes()),
        invalid_record,
        segment_at(key(2), 2, offset + invalid.len() as u64, after.as_bytes()),
        end(2, EndReason::Complete),
    ];

    let (observations, got) = decode(&records);
    let unknown = of_kind(&observations, "unknown");
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].fields["reason"], "invalid_redaction_audit");
    assert!(
        !serde_json::to_string(&observations)
            .unwrap()
            .contains(secret)
    );
    assert!(
        of_kind(&observations, "error").iter().all(|observation| {
            observation.supplied_id.as_deref() != Some(invalid_request_id)
                && observation.fields["error_type"] != invalid_error_type
        }),
        "the syntactically valid invalid-audit event is never provider-parsed"
    );
    assert_eq!(
        of_kind(&observations, "outcome")
            .iter()
            .map(|observation| observation.fields["stop_reason"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["end_turn"],
        "the observably emitting sacrificial outcome is discarded"
    );
    assert!(
        of_kind(&observations, "usage")
            .iter()
            .all(|observation| observation.fields["usage"]["output_tokens"] != 999),
        "the sacrificial cumulative usage snapshot is discarded"
    );
    let response = of_kind(&observations, "model_response")[0];
    assert_eq!(response.fields["message_complete"], false);
    assert_eq!(
        response.fields["losses"],
        json!(["invalid_redaction_audit"])
    );
    assert_eq!(response.fields["stream_end"], "complete");
    assert_eq!(got, summary(&observations, 1, true));
}

#[test]
fn malformed_event_and_partial_eof_are_named() {
    // Stream 2: a malformed event inside a message; the intact events around it decode.
    let malformed = [
        message_start("msg_M", json!({"output_tokens": 1})),
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\n\n"
            .to_string(),
        block_start(0, json!({"type": "text", "text": ""})),
        block_stop(0),
        message_delta("end_turn", json!({"output_tokens": 3})),
        message_stop(),
    ]
    .concat();
    // Stream 4: the final event is whole JSON but never dispatched (no blank line).
    let partial = [
        message_start("msg_P", json!({"output_tokens": 1})),
        message_delta("end_turn", json!({"output_tokens": 3})),
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n".to_string(),
    ]
    .concat();
    let mut records = sse_records(malformed.as_bytes(), &[]);
    records.push(response_open(4, wire(CONN_B, 1)));
    records.extend(segments(4, partial.as_bytes(), &[]));
    records.push(end(4, EndReason::Complete));
    let (observations, got) = decode(&records);

    let unknown = of_kind(&observations, "unknown");
    let reasons: Vec<(&Value, u64)> = unknown
        .iter()
        .map(|u| (&u.fields["reason"], u.source.stream.stream))
        .collect();
    assert_eq!(
        reasons,
        [(&json!("malformed"), 2), (&json!("partial_event"), 4)]
    );
    let responses = of_kind(&observations, "model_response");
    assert_eq!(responses[0].fields["losses"], json!(["malformed_event"]));
    assert_eq!(
        responses[0].fields["block_counts"],
        json!({"text": 1, "thinking": 0, "tool_use": 0, "unsupported": 0})
    );
    assert_eq!(
        responses[1].fields["losses"],
        json!(["missing_message_stop", "partial_event"])
    );
    // Only the dispatched stop of stream 2 is an outcome.
    let outcomes = of_kind(&observations, "outcome");
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].supplied_id.as_deref(), Some("msg_M"));
    assert_eq!(got, summary(&observations, 2, true));
}

#[test]
fn redacted_record_is_unknown_not_malformed() {
    // A fail-closed replacement ran from the key to the end of its line, keeping the newline.
    let clean = [
        message_start("msg_R", json!({"output_tokens": 1})),
        block_start(0, json!({"type": "text", "text": ""})),
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\n\n"
            .to_string(),
    ]
    .concat();
    let marker = "<redacted:secret>";
    // The replaced source bytes (a key and the rest of its line) are not retained; only
    // their length is, through the audit span.
    let replaced_source_len = 41;
    let replaced = format!(
        "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"key {marker}\n\n"
    );
    let rest = [block_stop(0), message_stop()].concat();
    let part0 = format!("{clean}{replaced}");
    let span_start = (part0.find(marker).unwrap()) as u64;
    let span_end = span_start + replaced_source_len as u64;
    let source_len0 = (part0.len() - marker.len() + replaced_source_len) as u64;
    let records = vec![
        response_open(2, wire(CONN_A, 1)),
        CaptureRecord::Segment {
            stream: key(2),
            part: 0,
            source_offset: 0,
            source_len: source_len0,
            first: part_first(0),
            last: part_last(0),
            data_b64: STANDARD.encode(part0.as_bytes()),
            audit: RedactionAudit {
                spans: vec![RedactionAuditSpan {
                    kind: "secret".to_string(),
                    start: span_start,
                    end: span_end,
                }],
            },
        },
        segment_at(key(2), 1, source_len0, rest.as_bytes()),
        end(2, EndReason::Complete),
    ];
    let (observations, got) = decode(&records);
    let unknown = of_kind(&observations, "unknown");
    assert_eq!(unknown.len(), 2);
    // The broken event before the audited range is provably untouched: malformed.
    assert_eq!(unknown[0].fields["reason"], "malformed");
    assert!(unknown[0].fields.get("kinds").is_none());
    assert_eq!(unknown[1].fields["reason"], "redacted");
    assert_eq!(unknown[1].fields["kinds"], json!(["secret"]));
    let response = of_kind(&observations, "model_response")[0];
    assert_eq!(
        response.fields["losses"],
        json!(["malformed_event", "redacted_event"])
    );
    assert_eq!(got, summary(&observations, 2, true));

    // A request body overlapping an audit span that still parses is decoded as supplied;
    // one that no longer parses is unknown `redacted`, never a parsed prefix.
    let parsed = r#"{"model":"claude-test-model","messages":[{"role":"user","content":"mail <redacted:email> now"}]}"#;
    let broken = r#"{"model":"claude-test-model","messages":[{"role":"user","content":"key <redacted:secret>"#;
    let mut records = Vec::new();
    for (stream, body) in [(1, parsed), (3, broken)] {
        let at_marker = body.find("<redacted:").unwrap() as u64;
        let kind = if stream == 1 { "email" } else { "secret" };
        let retained_marker_len = format!("<redacted:{kind}>").len() as u64;
        records.push(request_open(stream, wire(CONN_A, stream)));
        records.push(CaptureRecord::Segment {
            stream: key(stream),
            part: 0,
            source_offset: 0,
            source_len: body.len() as u64 + (24 - retained_marker_len),
            first: part_first(0),
            last: part_last(0),
            data_b64: STANDARD.encode(body.as_bytes()),
            audit: RedactionAudit {
                spans: vec![RedactionAuditSpan {
                    kind: kind.to_string(),
                    start: at_marker,
                    end: at_marker + 24,
                }],
            },
        });
        records.push(end(stream, EndReason::Complete));
    }
    let (observations, _) = decode(&records);
    assert_eq!(kinds(&observations), ["model", "model_request", "unknown"]);
    assert_eq!(observations[2].fields["reason"], "redacted");
    assert_eq!(observations[2].fields["kinds"], json!(["secret"]));
}

#[test]
fn oversized_tool_delta_is_bounded_and_reported() {
    // One tool input assembled from fragments beyond the record bound, then a later block.
    let chunk = "a".repeat(SEGMENT);
    let fragments = MAX_RECORD_BYTES / SEGMENT + 1;
    let mut text = [
        message_start("msg_O", json!({"output_tokens": 1})),
        tool_start(0, "toolu_big", "Write"),
        input_delta(0, "{\"blob\": \""),
    ]
    .concat();
    for _ in 0..fragments {
        text.push_str(&input_delta(0, &chunk));
    }
    text.push_str(&input_delta(0, "\"}"));
    text.push_str(&block_stop(0));
    text.push_str(&block_start(1, json!({"type": "text", "text": ""})));
    text.push_str(&text_delta(1, "after"));
    text.push_str(&block_stop(1));
    text.push_str(&message_delta("end_turn", json!({"output_tokens": 9})));
    text.push_str(&message_stop());
    let (observations, got) = decode(&sse_records(text.as_bytes(), &every(text.len(), SEGMENT)));
    let call = of_kind(&observations, "tool_call")[0];
    assert_eq!(call.supplied_id.as_deref(), Some("toolu_big"));
    assert_eq!(call.fields["input_status"], "over_budget");
    assert!(call.fields.get("input").is_none());
    let supplied = "{\"blob\": \"".len() + fragments * SEGMENT + 2;
    assert_eq!(call.fields["input_bytes"], supplied);
    let response = of_kind(&observations, "model_response")[0];
    assert_eq!(
        response.fields["block_counts"],
        json!({"text": 1, "thinking": 0, "tool_use": 1, "unsupported": 0})
    );
    assert_eq!(
        of_kind(&observations, "outcome")[0].fields["stop_reason"],
        "end_turn"
    );
    assert_eq!(got, summary(&observations, 0, true));

    // One SSE event beyond the record bound: unknown over budget, the next event decodes.
    let huge = format!(
        "event: content_block_delta\ndata: {}\n\n",
        json!({"type": "content_block_delta", "index": 0,
               "delta": {"type": "text_delta", "text": "b".repeat(MAX_RECORD_BYTES)}})
    );
    let text = [
        message_start("msg_H", json!({"output_tokens": 1})),
        block_start(0, json!({"type": "text", "text": ""})),
        huge,
        block_stop(0),
        message_stop(),
    ]
    .concat();
    let (observations, got) = decode(&sse_records(text.as_bytes(), &every(text.len(), SEGMENT)));
    let unknown = of_kind(&observations, "unknown");
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].fields["reason"], "over_budget");
    assert!(unknown[0].source.last_part > unknown[0].source.first_part);
    let response = of_kind(&observations, "model_response")[0];
    assert_eq!(response.fields["losses"], json!(["over_budget_event"]));
    assert_eq!(of_kind(&observations, "outcome").len(), 1);
    assert_eq!(got, summary(&observations, 1, true));
}

#[test]
fn total_parser_buffering_is_bounded() {
    // Nine request bodies held open at once, each under the record bound: together they
    // pass the shared budget, and only the one that crosses it is over budget.
    let body = |n: usize| {
        let filler = "x".repeat(MAX_RECORD_BYTES - 64 * 1024);
        format!(
            r#"{{"model":"claude-test-model","messages":[{{"role":"user","content":"{n}{filler}"}}]}}"#
        )
    };
    let streams = MAX_BUFFERED_BYTES / (MAX_RECORD_BYTES - 64 * 1024) + 1;
    assert_eq!(streams, 9);
    let bodies: Vec<String> = (0..streams).map(body).collect();
    let mut records = Vec::new();
    let mut ends = Vec::new();
    for (n, body) in bodies.iter().enumerate() {
        let stream = 2 * n as u64 + 1;
        records.push(request_open(stream, wire(CONN_A, n as u64 + 1)));
        records.extend(segments(
            stream,
            body.as_bytes(),
            &every(body.len(), SEGMENT),
        ));
        ends.push(end(stream, EndReason::Complete));
    }
    records.extend(ends);
    // Once released, the budget serves a later exchange again.
    let later = r#"{"model":"claude-test-model","messages":[{"role":"user","content":"later"}]}"#;
    records.extend(request_records(99, wire(CONN_B, 1), later.as_bytes()));
    let (observations, got) = decode(&records);
    let unknown = of_kind(&observations, "unknown");
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].fields["reason"], "over_budget");
    assert_eq!(unknown[0].source.stream.stream, 17);
    assert_eq!(of_kind(&observations, "model_request").len(), streams);
    assert_eq!(
        of_kind(&observations, "model_request")
            .last()
            .unwrap()
            .source
            .stream
            .stream,
        99
    );
    assert_eq!(got, summary(&observations, 1, true));
}

#[test]
fn unsupported_and_foreign_streams_are_counted_not_decoded() {
    let mut records = Vec::new();
    // Another protocol's stream: ignored, no state, no count.
    let mut foreign = meta(
        CaptureSource::WireRequest,
        wire(CONN_A, 1),
        "application/json",
    );
    foreign.protocol_hint = Some("openai-responses".to_string());
    records.push(open(1, CaptureSource::WireRequest, foreign));
    records.extend(segments(1, b"{\"model\":\"x\"}", &[]));
    records.push(end(1, EndReason::Complete));
    // Hinted, but not a decodable form: one explicit unknown each.
    let mut ws = meta(CaptureSource::WsServer, wire(CONN_A, 2), "application/json");
    ws.content_type = None;
    ws.wire = Some(WireIdentity {
        connection: CONN_A.to_string(),
        exchange: 2,
        message_index: Some(0),
    });
    let mut count_tokens = meta(
        CaptureSource::WireRequest,
        wire(CONN_A, 3),
        "application/json",
    );
    count_tokens.path = Some("/v1/messages/count_tokens".to_string());
    let mut html = meta(CaptureSource::WireResponse, wire(CONN_A, 4), "text/html");
    html.status = Some(502);
    let json_ok = meta(
        CaptureSource::WireResponse,
        wire(CONN_A, 5),
        "application/json",
    );
    for (stream, source, metadata) in [
        (3, CaptureSource::WsServer, ws),
        (5, CaptureSource::WireRequest, count_tokens),
        (7, CaptureSource::WireResponse, html),
        (9, CaptureSource::WireResponse, json_ok),
    ] {
        records.push(open(stream, source, metadata));
        records.extend(segments(stream, b"{\"type\":\"message\"}", &[]));
        records.push(end(stream, EndReason::Complete));
    }
    // A request with a block this decoder does not support keeps its known facts.
    let image = json!({"model": "claude-test-model", "messages": [{"role": "user", "content": [
        {"type": "image", "source": {"type": "base64", "data": "AAAA"}},
        {"type": "text", "text": "what is this"}]}]});
    records.extend(request_records(
        11,
        wire(CONN_B, 1),
        image.to_string().as_bytes(),
    ));
    let (observations, got) = decode(&records);
    let reasons: Vec<(u64, &Value)> = of_kind(&observations, "unknown")
        .iter()
        .map(|u| (u.source.stream.stream, &u.fields["reason"]))
        .collect();
    assert_eq!(
        reasons,
        [
            (3, &json!("unsupported_transport")),
            (5, &json!("unsupported_endpoint")),
            (7, &json!("unsupported_content_type")),
            (9, &json!("unsupported_response_form")),
            (11, &json!("unsupported_content_block")),
        ]
    );
    assert!(observations.iter().all(|o| o.source.stream.stream != 1));
    let request = of_kind(&observations, "model_request")[0];
    assert_eq!(
        request.fields["content_block_counts"],
        json!({"text": 1, "tool_result": 0, "unsupported": 1})
    );
    let image_unknown = of_kind(&observations, "unknown")[4];
    assert_eq!(image_unknown.fields["block_type"], "image");
    assert_eq!(
        (
            &image_unknown.fields["message_index"],
            &image_unknown.fields["block_index"]
        ),
        (&json!(0), &json!(0))
    );
    // Unsupported semantics alone are counted, not an incomplete decode.
    assert_eq!(got, summary(&observations, 5, false));
}

#[test]
fn lost_producer_and_unterminated_streams_stay_incomplete() {
    let started = [
        message_start("msg_L", json!({"output_tokens": 1})),
        tool_start(0, "toolu_L", "Bash"),
        input_delta(0, "{\"command\": \"ls"),
    ]
    .concat();
    let other = StreamKey {
        producer: OTHER_PRODUCER.to_string(),
        stream: 1,
    };
    let records = vec![
        response_open(2, wire(CONN_A, 1)),
        segment_at(key(2), 0, 0, started.as_bytes()),
        CaptureRecord::Recovery {
            previous_producer: PRODUCER.to_string(),
            producer: OTHER_PRODUCER.to_string(),
            observed: at(5000),
            reason: GapReason::ProducerLost,
        },
        open_with(
            other.clone(),
            CaptureSource::WireResponse,
            meta(
                CaptureSource::WireResponse,
                wire(CONN_A, 2),
                "text/event-stream",
            ),
        ),
        segment_at(
            other,
            0,
            0,
            message_start("msg_N", json!({"output_tokens": 1})).as_bytes(),
        ),
    ];
    let (observations, got) = decode(&records);
    let call = of_kind(&observations, "tool_call")[0];
    assert_eq!(call.fields["input_status"], "incomplete");
    let responses: Vec<(Option<&str>, &Value, &Value)> = of_kind(&observations, "model_response")
        .iter()
        .map(|r| {
            (
                r.supplied_id.as_deref(),
                &r.fields["stream_end"],
                &r.fields["message_complete"],
            )
        })
        .collect();
    assert_eq!(
        responses,
        [
            (Some("msg_L"), &json!("producer_lost"), &json!(false)),
            (Some("msg_N"), &json!("unterminated"), &json!(false)),
        ]
    );
    assert_eq!(got, summary(&observations, 0, true));
}

#[test]
fn unrecorded_recovery_releases_anthropic_slots_without_inventing_zero_segment_sources() {
    let mut records = Vec::new();
    for stream in 1..=MAX_OPEN_STREAMS as u64 {
        records.push(response_open(stream, wire(CONN_A, stream)));
    }
    records.extend(segments(
        1,
        message_start("msg_lost", json!({"output_tokens": 1})).as_bytes(),
        &[],
    ));
    records.push(CaptureRecord::Recovery {
        previous_producer: "never-recorded".into(),
        producer: OTHER_PRODUCER.into(),
        observed: at(5_000),
        reason: GapReason::ProducerLost,
    });

    let recovered = StreamKey {
        producer: OTHER_PRODUCER.into(),
        stream: 1,
    };
    records.push(open_with(
        recovered.clone(),
        CaptureSource::WireResponse,
        meta(
            CaptureSource::WireResponse,
            wire(CONN_B, 1),
            "text/event-stream",
        ),
    ));
    let body = [
        message_start("msg_after", json!({"output_tokens": 1})),
        message_stop(),
    ]
    .concat();
    records.push(segment_at(recovered.clone(), 0, 0, body.as_bytes()));
    records.push(CaptureRecord::End {
        stream: recovered,
        observed: at(6_000),
        reason: EndReason::Complete,
    });

    let (observations, summary) = decode(&records);
    let responses = of_kind(&observations, "model_response");
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0].supplied_id.as_deref(), Some("msg_lost"));
    assert_eq!(responses[0].fields["stream_end"], "producer_lost");
    assert_eq!(responses[1].supplied_id.as_deref(), Some("msg_after"));
    assert_eq!(
        of_kind(&observations, "unknown")
            .iter()
            .filter(|finding| finding.fields["reason"] == "producer_lost")
            .count(),
        0,
        "zero-segment streams have no SourceRef to fabricate"
    );
    assert!(summary.incomplete);
}

#[test]
fn tool_result_content_is_bounded() {
    // 64 KiB of replayed tool output, as a string and as a block array: each is kept as at
    // most 16 KiB of text, cut on a character boundary, with its whole length and a flag. A
    // small result is kept exactly as supplied.
    const BOUND: usize = 16 * 1024;
    let long = format!("a{}", "é".repeat(32 * 1024)); // byte BOUND falls inside an "é"
    let blocks = json!([{"type": "text", "text": "x".repeat(64 * 1024)}]);
    let small = json!([{"type": "text", "text": "ok"}]);
    let body = json!({"model": "claude-test-model", "stream": true, "messages": [
        {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_s", "content": long},
            {"type": "tool_result", "tool_use_id": "toolu_b", "content": blocks},
            {"type": "tool_result", "tool_use_id": "toolu_k", "content": small}]}]});
    let records = request_records(1, wire(CONN_A, 1), body.to_string().as_bytes());
    let (observations, got) = decode(&records);
    assert_eq!(got, summary(&observations, 0, false));
    let results = of_kind(&observations, "tool_result");
    assert_eq!(results.len(), 3);

    assert_eq!(
        results[0].fields["content"],
        format!("a{}", "é".repeat(8191))
    );
    assert_eq!(results[0].fields["content_bytes"], long.len());
    assert_eq!(results[0].fields["content_truncated"], true);

    let blocks_text = blocks.to_string();
    assert_eq!(results[1].fields["content"], &blocks_text[..BOUND]);
    assert_eq!(results[1].fields["content_bytes"], blocks_text.len());
    assert_eq!(results[1].fields["content_truncated"], true);

    assert_eq!(results[2].fields["content"], small);
    assert_eq!(results[2].fields["content_bytes"], small.to_string().len());
    assert!(results[2].fields.get("content_truncated").is_none());
}

// ---------------------------------------------------------------------------------------
// Request-body loss, in-message anomalies, the scope of `incomplete`.
// ---------------------------------------------------------------------------------------

#[test]
fn request_body_loss_is_never_parsed() {
    // The two halves happen to form valid JSON when spliced: nothing may be read from them.
    let head = br#"{"model":"claude-test-model","messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_x","content":"ab"#;
    let tail = br#"cd"}]}]}"#;
    let spliced = [head.as_slice(), tail.as_slice()].concat();
    assert!(serde_json::from_slice::<Value>(&spliced).is_ok());
    let gap = CaptureRecord::Gap {
        stream: Some(key(1)),
        observed: at(1700),
        reason: GapReason::QueueFull,
        lost_bytes: Some(5),
    };
    let tail_after_gap = segment_at(key(1), 1, head.len() as u64 + 5, tail);
    let contiguous_tail = segment_at(key(1), 1, head.len() as u64, tail);
    let recovery = CaptureRecord::Recovery {
        previous_producer: PRODUCER.to_string(),
        producer: OTHER_PRODUCER.to_string(),
        observed: at(5000),
        reason: GapReason::ProducerLost,
    };
    let cases: Vec<(&str, bool, Vec<CaptureRecord>)> = vec![
        (
            "gap",
            true,
            vec![gap, tail_after_gap.clone(), end(1, EndReason::Complete)],
        ),
        (
            "discontinuity",
            true,
            vec![tail_after_gap, end(1, EndReason::Complete)],
        ),
        (
            "interrupted",
            false,
            vec![contiguous_tail.clone(), end(1, EndReason::Interrupted)],
        ),
        (
            "decode_error",
            false,
            vec![contiguous_tail.clone(), end(1, EndReason::DecodeError)],
        ),
        ("producer_lost", true, vec![contiguous_tail, recovery]),
    ];
    for (reason, incomplete, rest) in cases {
        let mut records = vec![
            request_open(1, wire(CONN_A, 1)),
            segment_at(key(1), 0, 0, head),
        ];
        records.extend(rest);
        let (observations, got) = decode(&records);
        assert_eq!(kinds(&observations), ["unknown"], "{reason}");
        assert_eq!(observations[0].fields["reason"], reason);
        assert_eq!(
            (
                observations[0].source.first_part,
                observations[0].source.last_part
            ),
            (0, 1),
            "{reason}"
        );
        assert_eq!(got, summary(&observations, 1, incomplete), "{reason}");
    }
}

#[test]
fn faithful_early_stream_end_keeps_provider_incompleteness_without_decoder_loss() {
    let body = message_start("msg_early", json!({"output_tokens": 1}));
    for (end_reason, finding) in [
        (EndReason::Interrupted, "interrupted"),
        (EndReason::DecodeError, "decode_error"),
    ] {
        let mut records = vec![response_open(2, wire(CONN_A, 1))];
        records.extend(segments(2, body.as_bytes(), &[]));
        records.push(end(2, end_reason));
        let (observations, got) = decode(&records);
        let response = of_kind(&observations, "model_response")[0];
        assert_eq!(response.fields["message_complete"], false, "{finding}");
        assert_eq!(
            response.fields["losses"],
            json!([finding, "missing_message_stop"]),
            "{finding}"
        );
        assert_eq!(response.fields["stream_end"], finding);
        assert_eq!(got, summary(&observations, 0, false), "{finding}");
    }
}

#[test]
fn faithful_early_end_with_open_tool_keeps_incomplete_input_without_decoder_loss() {
    let body = [
        message_start("msg_tool_early", json!({"output_tokens": 1})),
        tool_start(0, "toolu_early", "lookup"),
        input_delta(0, r#"{"path":"x"#),
    ]
    .concat();
    for reason in [EndReason::Interrupted, EndReason::DecodeError] {
        let mut records = vec![response_open(2, wire(CONN_A, 1))];
        records.extend(segments(2, body.as_bytes(), &[]));
        records.push(end(2, reason));
        let (observations, got) = decode(&records);
        let call = of_kind(&observations, "tool_call")[0];
        assert_eq!(call.fields["input_status"], "incomplete");
        assert!(call.fields.get("input").is_none());
        assert_eq!(got, summary(&observations, 0, false), "{reason:?}");
    }
}

#[test]
fn unsupported_open_tool_delta_stays_capture_loss_at_faithful_early_end() {
    let body = [
        message_start("msg_tool_unsupported", json!({"output_tokens": 1})),
        tool_start(0, "toolu_unsupported", "lookup"),
        delta(0, json!({"type": "text_delta", "text": "discarded"})),
    ]
    .concat();
    for reason in [EndReason::Interrupted, EndReason::DecodeError] {
        let mut records = vec![response_open(2, wire(CONN_A, 1))];
        records.extend(segments(2, body.as_bytes(), &[]));
        records.push(end(2, reason));
        let (observations, got) = decode(&records);
        let call = of_kind(&observations, "tool_call")[0];
        assert_eq!(call.fields["input_status"], "incomplete");
        assert_eq!(
            of_kind(&observations, "unknown")[0].fields["reason"],
            "unsupported_delta"
        );
        assert_eq!(got, summary(&observations, 1, true), "{reason:?}");
    }
}

#[test]
fn complete_partial_event_is_loss_but_spanless_gap_tail_is_not_a_phantom_unknown() {
    let partial = b"data: {";
    let mut records = vec![response_open(2, wire(CONN_A, 1))];
    records.extend(segments(2, partial, &[]));
    records.push(end(2, EndReason::Complete));
    let (observations, got) = decode(&records);
    assert_eq!(of_kind(&observations, "unknown").len(), 1);
    assert_eq!(observations[0].fields["reason"], "partial_event");
    assert_eq!(got, summary(&observations, 1, true));

    let body = message_start("msg_gap_tail", json!({"output_tokens": 1}));
    let mut records = vec![response_open(2, wire(CONN_A, 1))];
    records.extend(segments(2, body.as_bytes(), &[]));
    records.push(CaptureRecord::Gap {
        stream: Some(key(2)),
        observed: at(1700),
        reason: GapReason::QueueFull,
        lost_bytes: Some(1),
    });
    records.push(end(2, EndReason::Complete));
    let (observations, got) = decode(&records);
    assert!(of_kind(&observations, "unknown").is_empty());
    let response = of_kind(&observations, "model_response")[0];
    assert_eq!(
        response.fields["losses"],
        json!(["gap", "missing_message_stop"])
    );
    assert_eq!(got, summary(&observations, 0, true));
}

#[test]
fn faithful_partial_sse_tail_names_exact_cut_without_decoder_loss() {
    let partial = b"event: content_block_delta\ndata: {\"type\":";
    for (reason, expected) in [
        (EndReason::Interrupted, "interrupted"),
        (EndReason::DecodeError, "decode_error"),
    ] {
        let mut records = vec![response_open(2, wire(CONN_A, 1))];
        records.extend(segments(2, partial, &[]));
        records.push(end(2, reason));
        let (observations, got) = decode(&records);
        assert_eq!(of_kind(&observations, "unknown").len(), 1);
        assert_eq!(observations[0].fields["reason"], expected);
        assert_eq!(got, summary(&observations, 1, false));
    }
}

fn assert_pending_over_budget_keeps_faithful_cut(end_reason: EndReason, cut: &str) {
    let oversized = format!("data: {}", "x".repeat(MAX_RECORD_BYTES + 1));
    let mut records = vec![response_open(2, wire(CONN_A, 1))];
    records.extend(segments(2, oversized.as_bytes(), &[]));
    records.push(end(2, end_reason));
    let (observations, got) = decode(&records);
    let reasons: Vec<_> = of_kind(&observations, "unknown")
        .iter()
        .map(|observation| observation.fields["reason"].as_str().unwrap())
        .collect();
    assert_eq!(reasons, ["over_budget", cut]);
    assert_eq!(got, summary(&observations, 2, true));
}

#[test]
fn pending_over_budget_without_message_keeps_interrupted_cut() {
    assert_pending_over_budget_keeps_faithful_cut(EndReason::Interrupted, "interrupted");
}

#[test]
fn pending_over_budget_without_message_keeps_decode_error_cut() {
    assert_pending_over_budget_keeps_faithful_cut(EndReason::DecodeError, "decode_error");
}

#[test]
fn pending_over_budget_with_message_keeps_cut_only_in_message_detail() {
    for (end_reason, cut) in [
        (EndReason::Interrupted, "interrupted"),
        (EndReason::DecodeError, "decode_error"),
    ] {
        let oversized = format!("data: {}", "x".repeat(MAX_RECORD_BYTES + 1));
        let body = [
            message_start("msg_pending", json!({"output_tokens": 1})),
            oversized,
        ]
        .concat();
        let mut records = vec![response_open(2, wire(CONN_A, 1))];
        records.extend(segments(2, body.as_bytes(), &[]));
        records.push(end(2, end_reason));
        let (observations, got) = decode(&records);
        let unknown = of_kind(&observations, "unknown");
        assert_eq!(unknown.len(), 1, "{cut}");
        assert_eq!(unknown[0].fields["reason"], "over_budget", "{cut}");
        let response = of_kind(&observations, "model_response")[0];
        assert_eq!(response.fields["stream_end"], cut);
        assert_eq!(
            response.fields["losses"],
            json!([cut, "missing_message_stop", "over_budget"]),
            "{cut}"
        );
        assert_eq!(got, summary(&observations, 1, true), "{cut}");
    }
}

#[test]
fn stopped_message_keeps_terminal_cut_in_stream_detail_without_duplicate_unknown() {
    let body = [
        message_start("msg_stopped", json!({"output_tokens": 1})),
        message_stop(),
    ]
    .concat();
    for (end_reason, cut) in [
        (EndReason::Interrupted, "interrupted"),
        (EndReason::DecodeError, "decode_error"),
    ] {
        let mut records = vec![response_open(2, wire(CONN_A, 1))];
        records.extend(segments(2, body.as_bytes(), &[]));
        records.push(end(2, end_reason));
        let (observations, got) = decode(&records);
        assert!(of_kind(&observations, "unknown").is_empty(), "{cut}");
        let response = of_kind(&observations, "model_response")[0];
        assert_eq!(response.fields["stream_end"], cut);
        assert_eq!(response.fields["message_complete"], true);
        assert_eq!(got, summary(&observations, 0, false), "{cut}");
    }
}

#[derive(Clone, Copy, Debug)]
enum NoMessageClose {
    Interrupted,
    DecodeError,
    ProducerLost,
    Unterminated,
}

impl NoMessageClose {
    fn reason(self) -> &'static str {
        match self {
            Self::Interrupted => "interrupted",
            Self::DecodeError => "decode_error",
            Self::ProducerLost => "producer_lost",
            Self::Unterminated => "unterminated",
        }
    }

    fn finish(self, records: &mut Vec<CaptureRecord>, stream: u64) {
        match self {
            Self::Interrupted => records.push(end(stream, EndReason::Interrupted)),
            Self::DecodeError => records.push(end(stream, EndReason::DecodeError)),
            Self::ProducerLost => records.push(CaptureRecord::Recovery {
                previous_producer: PRODUCER.to_string(),
                producer: OTHER_PRODUCER.to_string(),
                observed: at(5_000),
                reason: GapReason::ProducerLost,
            }),
            Self::Unterminated => {}
        }
    }
}

fn malformed_segment(stream: u64, part: u64, offset: u64, bytes: &[u8]) -> CaptureRecord {
    let mut record = segment_at(key(stream), part, offset, bytes);
    let CaptureRecord::Segment { data_b64, .. } = &mut record else {
        unreachable!()
    };
    *data_b64 = "%".to_string();
    record
}

fn invalid_audit_segment(stream: u64, part: u64, offset: u64, bytes: &[u8]) -> CaptureRecord {
    let mut record = segment_at(key(stream), part, offset, bytes);
    let CaptureRecord::Segment { audit, .. } = &mut record else {
        unreachable!()
    };
    audit.spans.push(RedactionAuditSpan {
        kind: "email".to_string(),
        start: 0,
        end: 1,
    });
    record
}

#[test]
fn no_message_sse_preserves_damage_then_every_non_complete_close() {
    let closes = [
        NoMessageClose::Interrupted,
        NoMessageClose::DecodeError,
        NoMessageClose::ProducerLost,
        NoMessageClose::Unterminated,
    ];
    for close in closes {
        let cases: Vec<(&str, Vec<CaptureRecord>, Vec<&str>)> = vec![
            (
                "pending_over_budget",
                segments(
                    2,
                    format!("data: {}", "x".repeat(MAX_RECORD_BYTES + 1)).as_bytes(),
                    &[],
                ),
                vec!["over_budget", close.reason()],
            ),
            (
                "partial_then_gap",
                vec![
                    segment_at(key(2), 0, 0, b"data: {"),
                    CaptureRecord::Gap {
                        stream: Some(key(2)),
                        observed: at(2_000),
                        reason: GapReason::QueueFull,
                        lost_bytes: Some(1),
                    },
                ],
                vec!["partial_event", "gap", close.reason()],
            ),
            (
                "malformed_segment",
                vec![malformed_segment(2, 0, 0, b"unreadable")],
                vec!["malformed_segment", close.reason()],
            ),
            (
                "invalid_audit",
                vec![invalid_audit_segment(2, 0, 0, b"data: {}\n\n")],
                vec!["invalid_redaction_audit", close.reason()],
            ),
            (
                "malformed_event",
                segments(2, b"data: {\n\n", &[]),
                vec!["malformed", close.reason()],
            ),
        ];
        for (damage, mut body, expected) in cases {
            let mut records = vec![response_open(2, wire(CONN_A, 1))];
            records.append(&mut body);
            close.finish(&mut records, 2);
            let (observations, got) = decode(&records);
            let reasons: Vec<_> = of_kind(&observations, "unknown")
                .iter()
                .map(|observation| observation.fields["reason"].as_str().unwrap())
                .collect();
            assert_eq!(reasons, expected, "{damage}, {close:?}");
            assert_eq!(got, summary(&observations, reasons.len() as u64, true));
        }
    }
}

#[test]
fn json_loss_precedes_every_independent_non_complete_close() {
    let closes = [
        NoMessageClose::Interrupted,
        NoMessageClose::DecodeError,
        NoMessageClose::ProducerLost,
        NoMessageClose::Unterminated,
    ];
    for close in closes {
        let cases: Vec<(&str, Vec<CaptureRecord>)> = vec![
            (
                "gap",
                vec![
                    segment_at(key(1), 0, 0, b"{"),
                    CaptureRecord::Gap {
                        stream: Some(key(1)),
                        observed: at(2_000),
                        reason: GapReason::QueueFull,
                        lost_bytes: Some(1),
                    },
                ],
            ),
            (
                "over_budget",
                segments(1, &vec![b'x'; MAX_RECORD_BYTES + 1], &[]),
            ),
            (
                "malformed_segment",
                vec![malformed_segment(1, 0, 0, b"unreadable")],
            ),
            (
                "invalid_redaction_audit",
                vec![invalid_audit_segment(1, 0, 0, b"{}")],
            ),
        ];
        for (loss, mut body) in cases {
            let mut records = vec![request_open(1, wire(CONN_A, 1))];
            records.append(&mut body);
            close.finish(&mut records, 1);
            let (observations, got) = decode(&records);
            let reasons: Vec<_> = of_kind(&observations, "unknown")
                .iter()
                .map(|observation| observation.fields["reason"].as_str().unwrap())
                .collect();
            assert_eq!(reasons, [loss, close.reason()], "{loss}, {close:?}");
            assert_eq!(got, summary(&observations, 2, true));
        }
    }
}

#[test]
fn json_preserves_distinct_causes_in_first_seen_order_before_terminal() {
    for case in [
        "over_then_gap",
        "over_then_malformed",
        "over_then_invalid_audit",
        "over_gap_malformed",
        "malformed_then_gap",
        "repeated_gap",
    ] {
        let oversized = vec![b'x'; MAX_RECORD_BYTES + 1];
        let mut records = vec![request_open(1, wire(CONN_A, 1))];
        let expected = match case {
            "over_then_gap" => {
                records.extend(segments(1, &oversized, &[]));
                records.push(CaptureRecord::Gap {
                    stream: Some(key(1)),
                    observed: at(2_000),
                    reason: GapReason::QueueFull,
                    lost_bytes: Some(1),
                });
                vec!["over_budget", "gap", "interrupted"]
            }
            "over_then_malformed" => {
                records.extend(segments(1, &oversized, &[]));
                records.push(malformed_segment(1, 1, oversized.len() as u64, b"bad"));
                vec!["over_budget", "malformed_segment", "interrupted"]
            }
            "over_then_invalid_audit" => {
                records.extend(segments(1, &oversized, &[]));
                records.push(invalid_audit_segment(1, 1, oversized.len() as u64, b"{}"));
                vec!["invalid_redaction_audit", "over_budget", "interrupted"]
            }
            "over_gap_malformed" => {
                records.extend(segments(1, &oversized, &[]));
                records.push(CaptureRecord::Gap {
                    stream: Some(key(1)),
                    observed: at(2_000),
                    reason: GapReason::QueueFull,
                    lost_bytes: Some(1),
                });
                records.push(malformed_segment(1, 1, oversized.len() as u64 + 1, b"bad"));
                vec!["over_budget", "gap", "malformed_segment", "interrupted"]
            }
            "malformed_then_gap" => {
                records.push(malformed_segment(1, 0, 0, b"bad"));
                records.push(CaptureRecord::Gap {
                    stream: Some(key(1)),
                    observed: at(2_000),
                    reason: GapReason::QueueFull,
                    lost_bytes: Some(1),
                });
                vec!["malformed_segment", "gap", "interrupted"]
            }
            "repeated_gap" => {
                records.push(segment_at(key(1), 0, 0, b"{"));
                for observed in [2_000, 3_000] {
                    records.push(CaptureRecord::Gap {
                        stream: Some(key(1)),
                        observed: at(observed),
                        reason: GapReason::QueueFull,
                        lost_bytes: Some(1),
                    });
                }
                vec!["gap", "interrupted"]
            }
            _ => unreachable!(),
        };
        records.push(end(1, EndReason::Interrupted));

        let (observations, got) = decode(&records);
        let reasons: Vec<_> = of_kind(&observations, "unknown")
            .iter()
            .map(|observation| observation.fields["reason"].as_str().unwrap())
            .collect();
        assert_eq!(reasons, expected, "{case}");
        assert_eq!(got, summary(&observations, expected.len() as u64, true));
    }
}

#[test]
fn eager_gap_is_not_repeated_when_unbounded_fragment_closes() {
    let partial = b"data: {";
    let mut records = vec![
        response_open(2, wire(CONN_A, 1)),
        segment_at(key(2), 0, 0, partial),
        CaptureRecord::Gap {
            stream: Some(key(2)),
            observed: at(2_000),
            reason: GapReason::QueueFull,
            lost_bytes: Some(1),
        },
        segment_at(key(2), 1, partial.len() as u64 + 1, b"fragment"),
    ];
    records.push(end(2, EndReason::Interrupted));

    let (observations, got) = decode(&records);
    let reasons: Vec<_> = of_kind(&observations, "unknown")
        .iter()
        .map(|observation| observation.fields["reason"].as_str().unwrap())
        .collect();
    assert_eq!(reasons, ["partial_event", "gap", "interrupted"]);
    assert_eq!(got, summary(&observations, 3, true));
}

#[test]
fn repeated_eager_gap_before_resync_boundary_is_deduplicated() {
    let partial = b"data: {";
    let records = vec![
        response_open(2, wire(CONN_A, 1)),
        segment_at(key(2), 0, 0, partial),
        CaptureRecord::Gap {
            stream: Some(key(2)),
            observed: at(2_000),
            reason: GapReason::QueueFull,
            lost_bytes: Some(1),
        },
        segment_at(key(2), 1, partial.len() as u64 + 1, b"fragment"),
        CaptureRecord::Gap {
            stream: Some(key(2)),
            observed: at(3_000),
            reason: GapReason::QueueFull,
            lost_bytes: Some(1),
        },
        end(2, EndReason::Interrupted),
    ];

    let (observations, got) = decode(&records);
    let reasons: Vec<_> = of_kind(&observations, "unknown")
        .iter()
        .map(|observation| observation.fields["reason"].as_str().unwrap())
        .collect();
    assert_eq!(reasons, ["partial_event", "gap", "interrupted"]);
    assert_eq!(got, summary(&observations, 3, true));
}

#[test]
fn unsupported_whole_message_preserves_distinct_damage_before_terminal() {
    let unsupported = || {
        open(
            3,
            CaptureSource::WireResponse,
            meta(CaptureSource::WireResponse, wire(CONN_A, 1), "text/plain"),
        )
    };
    let damaged = vec![
        unsupported(),
        segment_at(key(3), 0, 0, b"x"),
        CaptureRecord::Gap {
            stream: Some(key(3)),
            observed: at(2_000),
            reason: GapReason::QueueFull,
            lost_bytes: Some(1),
        },
        malformed_segment(3, 1, 2, b"bad"),
        end(3, EndReason::Interrupted),
    ];
    let (observations, got) = decode(&damaged);
    let reasons: Vec<_> = of_kind(&observations, "unknown")
        .iter()
        .map(|observation| observation.fields["reason"].as_str().unwrap())
        .collect();
    assert_eq!(
        reasons,
        [
            "gap",
            "malformed_segment",
            "unsupported_content_type",
            "interrupted"
        ]
    );
    assert_eq!(got, summary(&observations, 4, true));

    let audited = vec![
        unsupported(),
        segment_at(key(3), 0, 0, b"x"),
        CaptureRecord::Gap {
            stream: Some(key(3)),
            observed: at(2_000),
            reason: GapReason::QueueFull,
            lost_bytes: Some(1),
        },
        invalid_audit_segment(3, 1, 2, b"secret"),
        end(3, EndReason::Interrupted),
    ];
    let (observations, _) = decode(&audited);
    let reasons: Vec<_> = of_kind(&observations, "unknown")
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
fn pending_over_budget_survives_every_later_damage_with_and_without_message() {
    for with_message in [false, true] {
        for damage in ["gap", "malformed_segment", "invalid_redaction_audit"] {
            let oversized = format!("data: {}", "x".repeat(MAX_RECORD_BYTES + 1));
            let prefix = if with_message {
                message_start("msg_damage", json!({"output_tokens": 1}))
            } else {
                String::new()
            };
            let offset = prefix.len() + oversized.len();
            let mut records = vec![response_open(2, wire(CONN_A, 1))];
            records.extend(segments(
                2,
                [prefix.as_bytes(), oversized.as_bytes()]
                    .concat()
                    .as_slice(),
                &[],
            ));
            records.push(match damage {
                "gap" => CaptureRecord::Gap {
                    stream: Some(key(2)),
                    observed: at(3_000),
                    reason: GapReason::QueueFull,
                    lost_bytes: Some(1),
                },
                "malformed_segment" => malformed_segment(2, 1, offset as u64, b"unreadable"),
                "invalid_redaction_audit" => {
                    invalid_audit_segment(2, 1, offset as u64, b"data: {}\n\n")
                }
                _ => unreachable!(),
            });
            records.push(end(2, EndReason::Interrupted));
            let (observations, got) = decode(&records);
            let reasons: Vec<_> = of_kind(&observations, "unknown")
                .iter()
                .map(|observation| observation.fields["reason"].as_str().unwrap())
                .collect();
            let expected = if damage == "invalid_redaction_audit" {
                vec!["invalid_redaction_audit", "over_budget"]
            } else {
                vec!["over_budget"]
            };
            if with_message {
                assert_eq!(reasons, expected, "{damage}");
                let response = of_kind(&observations, "model_response")[0];
                assert_eq!(response.fields["stream_end"], "interrupted");
                assert!(
                    response.fields["losses"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|loss| loss == damage)
                );
                assert!(
                    response.fields["losses"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|loss| loss == "over_budget_event")
                );
            } else {
                let mut expected = expected;
                if damage != "invalid_redaction_audit" {
                    expected.push(damage);
                }
                expected.push("interrupted");
                assert_eq!(reasons, expected, "{damage}");
            }
            assert!(got.incomplete, "{damage}, with_message={with_message}");
        }
    }
}

#[test]
fn anomalous_events_never_complete_a_tool_input() {
    // An `event:` name contradicting the data type: the fragment it carried is not applied,
    // so the input assembled around it must not be reported complete.
    let mismatch = [
        message_start("msg_A", json!({"output_tokens": 1})),
        tool_start(0, "toolu_a", "Write"),
        input_delta(0, "{\"a\":1"),
        format!(
            "event: message_delta\ndata: {}\n\n",
            json!({"type": "content_block_delta", "index": 0,
                   "delta": {"type": "input_json_delta", "partial_json": ",\"b\":2"}})
        ),
        input_delta(0, "}"),
        block_stop(0),
        message_stop(),
    ]
    .concat();
    // A second start at an open index: its fragments must not complete the first tool.
    let duplicate = [
        message_start("msg_D", json!({"output_tokens": 1})),
        tool_start(0, "toolu_write", "Write"),
        tool_start(0, "toolu_bash", "Bash"),
        input_delta(0, "{\"command\":\"rm\"}"),
        block_stop(0),
        message_stop(),
    ]
    .concat();
    // A malformed start while a tool is open: attribution of later fragments is uncertain.
    let malformed_start = [
        message_start("msg_F", json!({"output_tokens": 1})),
        tool_start(0, "toolu_f", "Write"),
        input_delta(0, "{\"x\":"),
        ev(
            "content_block_start",
            json!({"type": "content_block_start", "index": 0}),
        ),
        input_delta(0, "2}"),
        block_stop(0),
        message_stop(),
    ]
    .concat();
    let mut records = sse_records(mismatch.as_bytes(), &[]);
    for (stream, body) in [(4, &duplicate), (6, &malformed_start)] {
        records.push(response_open(stream, wire(CONN_A, stream)));
        records.extend(segments(stream, body.as_bytes(), &[]));
        records.push(end(stream, EndReason::Complete));
    }
    let (observations, got) = decode(&records);

    let reasons: Vec<&Value> = of_kind(&observations, "unknown")
        .iter()
        .map(|u| &u.fields["reason"])
        .collect();
    assert_eq!(
        reasons,
        [
            &json!("event_type_mismatch"),
            &json!("duplicate_block_index"),
            &json!("unsupported_event_form")
        ]
    );
    let calls: Vec<(Option<&str>, &Value)> = of_kind(&observations, "tool_call")
        .iter()
        .map(|c| (c.supplied_id.as_deref(), &c.fields["input_status"]))
        .collect();
    let incomplete = json!("incomplete");
    assert_eq!(
        calls,
        [
            (Some("toolu_a"), &incomplete),
            (Some("toolu_write"), &incomplete),
            (Some("toolu_f"), &incomplete),
        ]
    );
    // No input is claimed anywhere, least of all the second tool's under the first's name.
    assert!(
        of_kind(&observations, "tool_call")
            .iter()
            .all(|c| c.fields.get("input").is_none())
    );
    for response in of_kind(&observations, "model_response") {
        assert_eq!(response.fields["message_complete"], false);
        assert_eq!(response.fields["losses"], json!(["anomalous_event"]));
    }
    assert_eq!(got, summary(&observations, 3, true));
}

#[test]
fn only_this_decoders_losses_and_streamless_gaps_limit_coverage() {
    let text = text_message("msg_K", "fine");
    let native = StreamKey {
        producer: OTHER_PRODUCER.to_string(),
        stream: 1,
    };
    let mut native_meta = meta(
        CaptureSource::WireRequest,
        wire(CONN_B, 1),
        "application/json",
    );
    native_meta.wire = None;
    native_meta.protocol_hint = Some("claude-code-jsonl".to_string());
    let native_gap = CaptureRecord::Gap {
        stream: Some(native.clone()),
        observed: at(1700),
        reason: GapReason::SourceChanged,
        lost_bytes: None,
    };
    let mut records = vec![open_with(native, CaptureSource::Native, native_meta)];
    records.extend(sse_records(text.as_bytes(), &[]));
    records.push(native_gap);
    // Another protocol's keyed gap leaves this decoder's complete evidence complete.
    let (observations, got) = decode(&records);
    assert_eq!(got, summary(&observations, 0, false));
    // A stream-less gap may have lost an Anthropic stream this decoder never saw.
    records.push(CaptureRecord::Gap {
        stream: None,
        observed: at(1800),
        reason: GapReason::ShutdownDeadline,
        lost_bytes: None,
    });
    let (observations, got) = decode(&records);
    assert_eq!(got, summary(&observations, 0, true));
}
