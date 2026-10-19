//! Synthetic shapes drawn from the privately inspected thread WS sample. No private bytes.
//! Each assertion catches a semantic selection, projection, source or conservative-loss failure.
use crate::common::{
    decode,
    wire::{WireSpec, facts, kind, parts},
};
use goalcup_protocol::capture::{
    CaptureMeta, CaptureRecord, CaptureSource, CaptureTime, EndReason, GapReason, MAX_OPEN_STREAMS,
};
use goalcup_protocol::redaction::RedactionAuditSpan;
use goalcup_verifier::capture_analysis::{api::Observation, registry::DecoderRegistry};
use serde_json::{Value, json};
const ID: &str = "amp-thread-jsonrpc";
const PRODUCER: &str = "0123456789abcdef0123456789abcdef";
fn time(tick: u64) -> CaptureTime {
    CaptureTime {
        wall: "2026-10-19T00:00:00Z".into(),
        mono_ns: tick,
        clock_id: PRODUCER.into(),
    }
}
fn spec(n: u64, source: CaptureSource) -> WireSpec {
    WireSpec::new(
        PRODUCER,
        n,
        source,
        CaptureMeta {
            wire: None,
            label: "thread".into(),
            protocol_hint: Some(ID.into()),
            method: None,
            host: None,
            path: None,
            status: None,
            content_type: None,
        },
        time,
    )
    .wire("123456789abcdef0123456789abcdef0", 1, Some(n))
}
fn server(n: u64) -> WireSpec {
    spec(n, CaptureSource::WsServer)
}
fn run(records: &[CaptureRecord]) -> (Vec<Observation>, bool, u64) {
    let (out, obs) = decode::run(&DecoderRegistry::builtin(), records);
    let entry = out.decoders.iter().find(|d| d.id == ID);
    let summary = entry.and_then(|d| d.result.as_ref().ok());
    (
        obs.into_iter()
            .filter(|(id, _)| *id == ID)
            .map(|(_, o)| o)
            .collect(),
        summary.is_some_and(|s| s.incomplete),
        summary.map_or(0, |s| s.unknown_records),
    )
}
fn snapshot(role: &str, content: Value) -> Value {
    let mut m = json!({"threadId":"thread-1","messageId":"msg-1","role":role,"content":content,"createdAt":"not-projected","readAt":null});
    if role == "assistant" {
        m["state"] = json!({"type":"complete"});
    }
    json!({"jsonrpc":"2.0","method":"message_added","params":{"message":m,"seq":8}})
}
fn text() -> Value {
    snapshot(
        "assistant",
        json!([{"type":"text","text":"Hello world","blockState":"complete"}]),
    )
}
fn decode_value(v: Value) -> Vec<Observation> {
    run(&server(1).records(v.to_string().as_bytes(), &[])).0
}
fn unknown_reasons(obs: &[Observation]) -> Vec<&str> {
    kind(obs, "unknown")
        .iter()
        .map(|o| o.fields["reason"].as_str().unwrap())
        .collect()
}
#[test]
fn complete_assistant_snapshots_are_source_qualified_and_never_provider_outcomes() {
    let obs = decode_value(text());
    let replies = kind(&obs, "model_response");
    assert_eq!(replies.len(), 1);
    let o = replies[0];
    assert_eq!(o.fields["text"], "Hello world");
    assert_eq!(o.fields["scope"], "application_thread");
    assert_eq!(o.fields["message_id"], "msg-1");
    assert_eq!(o.fields["block_index"], 0);
    assert_eq!(o.fields["type"], "text");
    assert_eq!(o.supplied_id.as_deref(), Some("msg-1"));
    assert_eq!(o.fields["wire"]["message_index"], 1);
    assert_eq!(parts(o), (0, 0));
    assert!(
        obs.iter()
            .all(|o| !["human_message", "model_request", "outcome"].contains(&o.kind.as_str()))
    );
}
#[test]
fn tool_call_preserves_model_name_and_bounded_json_input() {
    let obs = decode_value(snapshot(
        "assistant",
        json!([{"type":"tool_use","id":"call-1","name":"shell","blockState":"complete","complete":true,"input":{"command":"echo safe","workdir":"/synthetic"},"description":"not-projected"}]),
    ));
    let calls = kind(&obs, "tool_call");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].fields["name"], "shell");
    assert_eq!(calls[0].fields["call_id"], "call-1");
    assert_eq!(calls[0].supplied_id.as_deref(), Some("call-1"));
    assert_eq!(
        serde_json::from_str::<Value>(calls[0].fields["input"].as_str().unwrap()).unwrap(),
        json!({"command":"echo safe","workdir":"/synthetic"})
    );
    assert!(calls[0].fields.get("description").is_none());
}
#[test]
fn user_tool_result_is_the_context_snapshot_only() {
    let obs = decode_value(snapshot(
        "user",
        json!([{"type":"tool_result","toolUseID":"call-1","run":{"status":"done","result":{"output":"synthetic output","exitCode":0,"environment":"not-projected"}}}]),
    ));
    let results = kind(&obs, "tool_result");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].fields["output"], "synthetic output");
    assert_eq!(results[0].fields["exit_code"], 0);
    assert_eq!(results[0].fields["status"], "done");
    assert_eq!(results[0].fields["call_id"], "call-1");
    assert_eq!(results[0].supplied_id.as_deref(), Some("call-1"));
    assert!(results[0].fields.get("environment").is_none());
    assert!(kind(&obs, "human_message").is_empty());
}
#[test]
fn supplied_model_and_usage_are_per_snapshot_with_only_integer_counts() {
    let mut v = text();
    v["params"]["message"]["usage"] = json!({"model":"model-one","inputTokens":4,"outputTokens":136,"cacheCreationInputTokens":42350,"cacheReadInputTokens":0,"totalInputTokens":42354,"maxInputTokens":872000,"features":["private-feature"],"timestamp":"not-projected","unsafe":"private-value"});
    let mut records = server(1).records(v.to_string().as_bytes(), &[]);
    records.extend(server(2).records(v.to_string().as_bytes(), &[]));
    let (obs, _, _) = run(&records);
    assert_eq!(kind(&obs, "model").len(), 2);
    let usage = kind(&obs, "usage");
    assert_eq!(usage.len(), 2);
    assert_eq!(usage[0].fields["inputTokens"], 4);
    assert_eq!(usage[0].fields["outputTokens"], 136);
    assert_eq!(usage[0].fields["totalInputTokens"], 42354);
    assert_eq!(usage[0].fields["scope"], "application_thread");
    assert_eq!(kind(&obs, "model")[0].fields["model"], "model-one");
    assert!(
        !serde_json::to_string(&obs)
            .unwrap()
            .contains("private-value")
    );
    assert!(
        !serde_json::to_string(&obs)
            .unwrap()
            .contains("private-feature")
    );
    v["params"]["message"]["usage"] =
        json!({"model":"not a safe token","inputTokens":-1,"outputTokens":1.5});
    let obs = decode_value(v);
    assert!(kind(&obs, "model").is_empty());
    let u = kind(&obs, "usage");
    assert_eq!(u.len(), 1);
    assert!(u[0].fields.get("inputTokens").is_none());
    assert!(u[0].fields.get("outputTokens").is_none());
}
#[test]
fn split_boundaries_and_interleaving_preserve_occurrences_and_source_parts() {
    let body = text().to_string();
    let whole = run(&server(1).records(body.as_bytes(), &[])).0;
    assert_eq!(kind(&whole, "model_response").len(), 1);
    for at in 1..body.len() {
        let (obs, _, _) = run(&server(1).records(body.as_bytes(), &[at]));
        assert_eq!(facts(&obs), facts(&whole));
        assert!(obs.iter().all(|o| parts(o) == (0, 1)));
    }
    let a = server(1);
    let b = server(2);
    let seg = a.segments(body.as_bytes(), &[13], 100);
    let mut records = vec![a.open(), seg[0].clone(), b.open()];
    records.extend(b.segments(body.as_bytes(), &[], 200));
    records.push(b.end(EndReason::Complete));
    records.push(seg[1].clone());
    records.push(a.end(EndReason::Complete));
    let (obs, _, _) = run(&records);
    let r = kind(&obs, "model_response");
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].source.stream, b.key);
    assert_eq!(r[1].source.stream, a.key);
    assert_eq!(parts(r[1]), (0, 1));
}
#[test]
fn batch_array_decodes_items_individually_and_keeps_unsupported_items() {
    let obs = decode_value(json!([text(),{"type":"agent_state","secret":"not-projected"},7]));
    assert_eq!(kind(&obs, "model_response").len(), 1);
    assert_eq!(kind(&obs, "unknown").len(), 2);
    assert!(
        !serde_json::to_string(&obs)
            .unwrap()
            .contains("not-projected")
    );
    assert!(obs.iter().all(|o| parts(o) == (0, 0)));
}
#[test]
fn deltas_leases_executor_user_echo_and_heartbeats_remain_unknown() {
    for v in [
        json!({"jsonrpc":"2.0","method":"delta","params":{"text":"not-projected"}}),
        json!({"method":"tool_lease","params":{"name":"executor-name"}}),
        json!({"method":"executor_tool_result"}),
        json!({"method":"tool_progress"}),
        json!({"method":"client_append_user_msg"}),
        json!({"type":"thread_settings"}),
        snapshot("user", json!([{"type":"text","text":"not-human"}])),
    ] {
        let obs = decode_value(v);
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].kind, "unknown");
    }
    for literal in [b"ping".as_slice(), b"pong"] {
        let (obs, incomplete, n) = run(&server(1).records(literal, &[]));
        assert_eq!(unknown_reasons(&obs), ["heartbeat"]);
        assert!(!incomplete);
        assert_eq!(n, 1);
    }
}
#[test]
fn client_snapshots_never_decode_and_selection_requires_exact_hint_and_ws() {
    let body = text().to_string();
    let (obs, _, _) = run(&spec(1, CaptureSource::WsClient).records(body.as_bytes(), &[]));
    assert_eq!(unknown_reasons(&obs), ["unsupported_client_message"]);
    for source in [
        CaptureSource::WireRequest,
        CaptureSource::WireResponse,
        CaptureSource::Native,
        CaptureSource::Goal,
    ] {
        assert!(
            run(&spec(1, source).records(body.as_bytes(), &[]))
                .0
                .is_empty()
        );
    }
    for hint in [
        None,
        Some("amp-thread-jsonrpc-v2"),
        Some("openai-responses"),
    ] {
        let mut s = server(1);
        s.meta.protocol_hint = hint.map(str::to_string);
        assert!(run(&s.records(body.as_bytes(), &[])).0.is_empty());
    }
}
#[test]
fn incomplete_or_unknown_blocks_are_referenced_instead_of_guessed() {
    for content in [
        json!([{"type":"text","text":"partial","blockState":"streaming"}]),
        json!([{"type":"tool_use","input":{},"blockState":"complete","complete":false}]),
        json!([{"type":"future","secret":"not-projected"}]),
        json!([7]),
        json!({}),
    ] {
        let obs = decode_value(snapshot("assistant", content));
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].kind, "unknown");
    }
    let mut v = text();
    v["params"]["message"]["state"] = json!({"type":"streaming"});
    assert_eq!(decode_value(v)[0].kind, "unknown");
}
#[test]
fn text_thinking_ids_and_text_input_output_bounds_are_projected_safely() {
    let v = snapshot(
        "assistant",
        json!([{"type":"thinking","thinking":"synthetic thought","id":"block-1","blockState":"complete"},{"type":"text","text":"x".repeat(17000),"blockState":"complete"}]),
    );
    let obs = decode_value(v);
    let r = kind(&obs, "model_response");
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].fields["type"], "thinking");
    assert_eq!(r[0].fields["thinking"], "synthetic thought");
    assert_eq!(r[0].fields["block_id"], "block-1");
    assert_eq!(r[1].fields["text"].as_str().unwrap().len(), 16384);
    assert_eq!(r[1].fields["text_bytes"], 17000);
    assert_eq!(r[1].fields["text_truncated"], true);
    let obs = decode_value(snapshot(
        "assistant",
        json!([{"type":"tool_use","id":"bad id","name":"unsafe name","input":{"v":"x".repeat(17000)},"blockState":"complete","complete":true}]),
    ));
    let c = kind(&obs, "tool_call");
    assert_eq!(c.len(), 1);
    assert!(c[0].fields.get("name").is_none());
    assert!(c[0].fields.get("call_id").is_none());
    assert_eq!(c[0].fields["input_truncated"], true);
    let obs = decode_value(snapshot(
        "user",
        json!([{"type":"tool_result","toolUseID":"call-1","run":{"status":"done","result":{"output":"y".repeat(17000),"exitCode":-1}}}]),
    ));
    let r = kind(&obs, "tool_result");
    assert_eq!(r[0].fields["output_truncated"], true);
    assert_eq!(r[0].fields["exit_code"], -1);
}
#[test]
fn malformed_nonobject_utf8_and_redaction_never_become_facts() {
    for bytes in [b"{".as_slice(), &[255, 254], b"null", b"[]"] {
        let (obs, _, n) = run(&server(1).records(bytes, &[]));
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].kind, "unknown");
        assert_eq!(n, 1);
    }
    let s = server(1);
    let bytes = b"<redacted:secret>";
    let mut seg = s.segment(
        0,
        0,
        bytes,
        (1, 2),
        vec![RedactionAuditSpan {
            kind: "secret".into(),
            start: 0,
            end: 1,
        }],
    );
    if let CaptureRecord::Segment { source_len, .. } = &mut seg {
        *source_len = 1;
    }
    let (obs, _, _) = run(&[s.open(), seg, s.end(EndReason::Complete)]);
    assert_eq!(unknown_reasons(&obs), ["redacted"]);
    // Valid JSON redaction still cannot establish a complete unmodified snapshot.
    let body = text()
        .to_string()
        .replace("Hello world", "<redacted:secret>");
    let start = body.find("<redacted").unwrap() as u64;
    let mut seg = s.segment(
        0,
        0,
        body.as_bytes(),
        (1, 2),
        vec![RedactionAuditSpan {
            kind: "secret".into(),
            start,
            end: start + 1,
        }],
    );
    if let CaptureRecord::Segment { source_len, .. } = &mut seg {
        *source_len = body.len() as u64 - 16;
    }
    let (obs, _, _) = run(&[s.open(), seg, s.end(EndReason::Complete)]);
    assert!(kind(&obs, "model_response").is_empty());
    assert_eq!(obs[0].kind, "unknown");
}
#[test]
fn gaps_unsupported_encoding_and_bad_base64_discard_partial_frames() {
    let body = text().to_string();
    let s = server(1);
    for reason in [
        GapReason::DecodeFailed,
        GapReason::UnsupportedEncoding,
        GapReason::LimitExceeded,
    ] {
        let mut records = s.records(body.as_bytes(), &[10]);
        let mut gap = s.gap();
        if let CaptureRecord::Gap { reason: r, .. } = &mut gap {
            *r = reason;
        }
        records.insert(2, gap);
        let (obs, incomplete, _) = run(&records);
        assert!(incomplete);
        assert!(kind(&obs, "model_response").is_empty());
        assert_eq!(unknown_reasons(&obs), ["gap"]);
        assert_eq!(parts(&obs[0]), (0, 1));
    }
    let mut records = s.records(body.as_bytes(), &[]);
    if let CaptureRecord::Segment { data_b64, .. } = &mut records[1] {
        *data_b64 = "invalid%".into();
    }
    let (obs, incomplete, _) = run(&records);
    assert!(incomplete);
    assert_eq!(unknown_reasons(&obs), ["undecodable_segment"]);
    let mut records = s.records(body.as_bytes(), &[]);
    if let CaptureRecord::Segment { audit, .. } = &mut records[1] {
        audit.spans.push(RedactionAuditSpan {
            kind: "bad".into(),
            start: 2,
            end: 1,
        });
    }
    let (obs, incomplete, _) = run(&records);
    assert!(incomplete);
    assert_eq!(unknown_reasons(&obs), ["invalid_redaction_audit"]);
}
#[test]
fn interrupted_decodeerror_unterminated_and_lost_producer_do_not_decode_frames() {
    let body = text().to_string();
    let s = server(1);
    for (reason, want) in [
        (EndReason::Interrupted, "interrupted"),
        (EndReason::DecodeError, "decode_error"),
    ] {
        let mut records = s.records(body.as_bytes(), &[]);
        *records.last_mut().unwrap() = s.end(reason);
        let (obs, incomplete, _) = run(&records);
        assert_eq!(unknown_reasons(&obs), [want]);
        assert!(!incomplete);
    }
    let mut records = s.records(body.as_bytes(), &[]);
    records.pop();
    let (obs, incomplete, _) = run(&records);
    assert!(incomplete);
    assert_eq!(unknown_reasons(&obs), ["unterminated"]);
    records.push(CaptureRecord::ProducerClosed {
        producer: PRODUCER.into(),
        observed: time(999),
        final_seq: Some(99),
    });
    let (obs, incomplete, _) = run(&records);
    assert!(incomplete);
    assert_eq!(unknown_reasons(&obs), ["unterminated"]);
    records.pop();
    records.push(CaptureRecord::Recovery {
        previous_producer: PRODUCER.into(),
        producer: "fedcba9876543210fedcba9876543210".into(),
        observed: time(999),
        reason: GapReason::ProducerLost,
    });
    let (obs, incomplete, _) = run(&records);
    assert!(incomplete);
    assert_eq!(unknown_reasons(&obs), ["producer_lost"]);
}
#[test]
fn frame_list_content_and_total_buffer_bounds_degrade_with_source_unknowns() {
    let s = server(1);
    let body = vec![b'x'; 1024 * 1024 + 1];
    let (obs, incomplete, _) = run(&s.records(&body, &[]));
    assert!(incomplete);
    assert_eq!(unknown_reasons(&obs), ["over_budget"]);
    assert_eq!(parts(&obs[0]), (0, 0));
    let obs = decode_value(Value::Array(vec![text(); 65]));
    assert_eq!(kind(&obs, "model_response").len(), 64);
    assert_eq!(unknown_reasons(&obs), ["over_budget"]);
    let v = snapshot(
        "assistant",
        Value::Array(vec![
            json!({"type":"text","text":"x","blockState":"complete"});
            65
        ]),
    );
    let (obs, incomplete, _) = run(&s.records(v.to_string().as_bytes(), &[]));
    assert!(incomplete);
    assert_eq!(kind(&obs, "model_response").len(), 64);
    assert_eq!(unknown_reasons(&obs), ["over_budget"]);
    let mut records = Vec::new();
    for n in 1..=9 {
        let s = server(n);
        records.push(s.open());
        records.push(s.segment(0, 0, &vec![b'x'; 1024 * 1024], (1, 2), vec![]));
    }
    for n in 1..=9 {
        records.push(server(n).end(EndReason::Complete));
    }
    let (obs, incomplete, _) = run(&records);
    assert!(incomplete);
    assert_eq!(kind(&obs, "unknown").len(), 9);
    assert!(unknown_reasons(&obs).contains(&"over_budget"));
}
#[test]
fn open_stream_bound_fails_decoder_without_inventing_segment_refs() {
    let mut records = Vec::new();
    for n in 1..=MAX_OPEN_STREAMS as u64 + 1 {
        records.push(server(n).open());
    }
    let (out, obs) = decode::run(&DecoderRegistry::builtin(), &records);
    assert!(obs.is_empty());
    let entry = out.decoders.iter().find(|d| d.id == ID).unwrap();
    assert_eq!(
        entry.result.as_ref().unwrap_err().failure.reason,
        "too_many_open_streams"
    );
}

#[test]
fn unsupported_snapshot_fields_and_empty_messages_cannot_supply_facts() {
    for value in [
        json!({"jsonrpc":"2.0","method":"message_added","params":{}}),
        snapshot("system", json!([{"type":"text","text":"not-projected"}])),
        snapshot("assistant", json!([])),
        snapshot(
            "assistant",
            json!([{"type":"text","blockState":"complete"}]),
        ),
        snapshot(
            "assistant",
            json!([{"type":"tool_use","id":"call-1","name":"safe","input":"unsupported","blockState":"complete","complete":true}]),
        ),
        snapshot(
            "user",
            json!([{"type":"tool_result","toolUseID":"call-1","run":{"status":"running","result":{"output":"not-projected"}}}]),
        ),
        snapshot(
            "user",
            json!([{"type":"tool_result","toolUseID":"call-1","run":{"status":"done","result":{"output":{"future":"not-projected"}}}}]),
        ),
        json!([[text()]]),
    ] {
        let obs = decode_value(value);
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].kind, "unknown");
        assert_eq!(parts(&obs[0]), (0, 0));
        assert!(
            !serde_json::to_string(&obs)
                .unwrap()
                .contains("not-projected")
        );
    }
    let (obs, incomplete, unknown) = run(&server(1).records(b"", &[]));
    assert!(
        obs.is_empty(),
        "no retained part means no invented source reference"
    );
    assert!(!incomplete);
    assert_eq!(unknown, 1);
}
