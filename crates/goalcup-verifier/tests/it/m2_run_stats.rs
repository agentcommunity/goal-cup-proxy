//! The numbers-only run summary: `run_stats` and `goalcup-verify --stats TRACE… --output
//! STATS.json` (`docs/research/m2-report-format.md`, "Run stats").
//!
//! Every trace is written by the real protocol writers (`support::capture_records`). Decoder
//! inputs are synthetic; the expected numbers are worked out by hand from what each test wrote.

use crate::common as support;

use goalcup_protocol::capture::{
    CaptureMeta, CaptureRecord, CaptureSource, ConnectionEvent, ConnectionOutcome, GapReason,
    WireIdentity,
};
use goalcup_protocol::export::CHAIN_FILE;
use goalcup_verifier::capture_analysis::{anthropic, openai, thread};
use goalcup_verifier::{RunStatsResult, VerifyOptions, run_stats};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use support::capture_records::{at, epoch, goal, key, launch, stream, wire_meta, write_trace};

const CLAUDE: &str = "b2c3d4e5f60718293a4b5c6d7e8f90a1";
const CODEX: &str = "c0dec0dec0dec0dec0dec0dec0dec0de";
const WIRE: &str = "0123456789abcdef0123456789abcdef";
const CONN: &str = "5e1f0a2b3c4d5e6f708192a3b4c5d6e7";
/// Typed by the participant: must never reach the stats.
const MARKER: &str = "SECRET-PROMPT-MARKER-7f3a";

// ---------------------------------------------------------------------------------------
// Inputs.
// ---------------------------------------------------------------------------------------

fn jsonl(lines: &[Value]) -> Vec<u8> {
    lines
        .iter()
        .map(|l| format!("{l}\n"))
        .collect::<String>()
        .into_bytes()
}

fn native_meta(hint: &str) -> CaptureMeta {
    CaptureMeta {
        wire: None,
        label: format!("sessions/{hint}"),
        protocol_hint: Some(hint.to_string()),
        method: None,
        host: None,
        path: None,
        status: None,
        content_type: None,
    }
}

fn anthropic_meta(source: CaptureSource, exchange: u64) -> CaptureMeta {
    let request = source == CaptureSource::WireRequest;
    CaptureMeta {
        wire: Some(WireIdentity {
            connection: CONN.to_string(),
            exchange,
            message_index: None,
        }),
        label: "POST /v1/messages".to_string(),
        protocol_hint: Some(anthropic::PROTOCOL_HINT.to_string()),
        method: request.then(|| "POST".to_string()),
        host: Some("api.example.test".to_string()),
        path: Some("/v1/messages".to_string()),
        status: (!request).then_some(200),
        content_type: Some(
            if request {
                "application/json"
            } else {
                "text/event-stream"
            }
            .to_string(),
        ),
    }
}

fn claude_human(prompt_id: &str, text: &str) -> Value {
    json!({"type": "user", "promptId": prompt_id, "promptSource": "typed",
        "message": {"role": "user", "content": text}})
}

/// One assistant line of message `id`. Claude Code writes one line per content block, each
/// with the message's usage repeated.
fn claude_assistant(id: &str, usage: Value, content: Value) -> Value {
    json!({"type": "assistant", "message": {"id": id, "model": "claude-test-model",
        "usage": usage, "content": content}})
}

fn claude_tool_result(tool_use_id: &str, is_error: bool) -> Value {
    json!({"type": "user", "message": {"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": tool_use_id, "is_error": is_error,
         "content": format!("output of {tool_use_id}")}]}})
}

/// The Claude Code session file of the Claude run: the prompt, two messages (the first
/// written as two lines, the duplicate view), two tool results and two later messages.
fn claude_session() -> Vec<u8> {
    let usage_1 = json!({"input_tokens": 50, "output_tokens": 6,
        "cache_read_input_tokens": 900, "cache_creation_input_tokens": 80});
    let usage_2 = json!({"input_tokens": 40, "output_tokens": 9,
        "cache_read_input_tokens": 1000, "cache_creation_input_tokens": 0});
    jsonl(&[
        claude_human("prompt-1", &format!("{MARKER} build the parser")),
        claude_assistant(
            "msg_1",
            usage_1.clone(),
            json!([{"type": "text", "text": "on it"}]),
        ),
        claude_assistant(
            "msg_1",
            usage_1,
            json!([
                {"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {}},
                {"type": "tool_use", "id": "toolu_2", "name": "Task", "input": {}}
            ]),
        ),
        claude_tool_result("toolu_1", true),
        claude_tool_result("toolu_2", false),
        claude_assistant("msg_2", usage_2, json!([{"type": "text", "text": "done"}])),
        claude_human("prompt-2", "Keep going."),
        claude_human("prompt-3", "please also add tests"),
    ])
}

fn ev(name: &str, data: Value) -> String {
    format!("event: {name}\ndata: {data}\n\n")
}

/// One streamed reply: `message_start` with the input counts and an initial output of 1, the
/// given tool calls, and a final `message_delta` whose output count is cumulative.
fn anthropic_reply(id: &str, input: u64, cache_read: u64, output: u64, tools: &[&str]) -> String {
    let mut events = vec![ev(
        "message_start",
        json!({"type": "message_start", "message": {"id": id, "type": "message",
            "role": "assistant", "model": "claude-test-model", "content": [],
            "stop_reason": null, "usage": {"input_tokens": input, "output_tokens": 1,
                "cache_read_input_tokens": cache_read, "cache_creation_input_tokens": 0}}}),
    )];
    for (index, tool) in tools.iter().enumerate() {
        events.push(ev(
            "content_block_start",
            json!({"type": "content_block_start", "index": index,
                "content_block": {"type": "tool_use", "id": tool, "name": "Bash", "input": {}}}),
        ));
        events.push(ev(
            "content_block_stop",
            json!({"type": "content_block_stop", "index": index}),
        ));
    }
    events.push(ev(
        "message_delta",
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"},
            "usage": {"output_tokens": output}}),
    ));
    events.push(ev("message_stop", json!({"type": "message_stop"})));
    events.concat()
}

/// A request replaying the history so far: every earlier tool result, every time.
fn anthropic_request(results: &[(&str, bool)]) -> Vec<u8> {
    let mut content = vec![json!({"type": "text", "text": format!("{MARKER} context")})];
    for (id, is_error) in results {
        content.push(
            json!({"type": "tool_result", "tool_use_id": id, "is_error": is_error,
            "content": "replayed output"}),
        );
    }
    json!({"model": "claude-test-model", "max_tokens": 1024, "stream": true,
        "messages": [{"role": "user", "content": content}]})
    .to_string()
    .into_bytes()
}

/// The recorded traffic of the Claude run: three exchanges, each request replaying every
/// earlier tool result. Output counts come only from the final cumulative delta.
fn anthropic_wire() -> Vec<CaptureRecord> {
    epoch(WIRE, anthropic_wire_records())
}

/// [`anthropic_wire`] before its producer's close.
fn anthropic_wire_records() -> Vec<CaptureRecord> {
    let exchanges = [
        (
            anthropic_request(&[]),
            anthropic_reply("msg_w1", 10, 100, 7, &["toolu_1"]),
        ),
        (
            anthropic_request(&[("toolu_1", true)]),
            anthropic_reply("msg_w2", 20, 200, 11, &["toolu_3"]),
        ),
        (
            anthropic_request(&[("toolu_1", true), ("toolu_3", false)]),
            anthropic_reply("msg_w3", 30, 300, 13, &[]),
        ),
    ];
    let mut records = vec![connection_open()];
    for (exchange, (request, reply)) in (1u64..).zip(exchanges) {
        records.extend(stream(
            WIRE,
            2 * exchange - 1,
            CaptureSource::WireRequest,
            anthropic_meta(CaptureSource::WireRequest, exchange),
            &request,
        ));
        records.extend(stream(
            WIRE,
            2 * exchange,
            CaptureSource::WireResponse,
            anthropic_meta(CaptureSource::WireResponse, exchange),
            reply.as_bytes(),
        ));
    }
    records.push(connection_close());
    records
}

/// One WebSocket connection of `hint`: each message is one stream, in order.
fn ws_run(hint: &str, messages: &[(CaptureSource, Value)]) -> Vec<CaptureRecord> {
    let mut records = vec![connection_open()];
    for (index, (source, message)) in (1u64..).zip(messages) {
        let meta = CaptureMeta {
            wire: Some(WireIdentity {
                connection: CONN.to_string(),
                exchange: 1,
                message_index: Some(index),
            }),
            label: format!("{hint} #{index}"),
            protocol_hint: Some(hint.to_string()),
            method: None,
            host: Some("api.example.test".to_string()),
            path: Some("/backend-api/codex/responses".to_string()),
            status: None,
            content_type: None,
        };
        records.extend(stream(
            WIRE,
            index,
            *source,
            meta,
            message.to_string().as_bytes(),
        ));
    }
    records.push(connection_close());
    epoch(WIRE, records)
}

fn connection_close() -> CaptureRecord {
    CaptureRecord::Connection {
        producer: WIRE.to_string(),
        connection: CONN.to_string(),
        observed: at(WIRE, 3),
        event: ConnectionEvent::Close {
            outcome: ConnectionOutcome::Complete,
        },
    }
}

fn openai_completed(id: &str) -> Value {
    json!({"type": "response.completed", "sequence_number": 9, "response": {
        "id": id, "object": "response", "status": "completed", "model": "gpt-test",
        "output": [], "error": null, "incomplete_details": null,
        "usage": {"input_tokens": 100, "input_tokens_details": {"cached_tokens": 60},
            "output_tokens": 20, "output_tokens_details": {"reasoning_tokens": 5},
            "total_tokens": 120}}})
}

fn openai_created(id: &str) -> Value {
    json!({"type": "response.created", "sequence_number": 0, "response": {
        "id": id, "object": "response", "status": "in_progress", "model": "gpt-test",
        "output": [], "usage": null, "error": null}})
}

fn amp_snapshot(message_id: Option<&str>, output: u64) -> Value {
    let mut message = json!({"threadId": "thread-1", "role": "assistant",
        "state": {"type": "complete"},
        "content": [{"type": "text", "text": "done", "blockState": "complete"}],
        "usage": {"model": "amp-test-model", "inputTokens": 4, "outputTokens": output,
            "cacheCreationInputTokens": 300, "cacheReadInputTokens": 9000}});
    if let Some(id) = message_id {
        message["messageId"] = json!(id);
    }
    json!({"jsonrpc": "2.0", "method": "message_added", "params": {"message": message, "seq": 1}})
}

fn connection_open() -> CaptureRecord {
    CaptureRecord::Connection {
        producer: WIRE.to_string(),
        connection: CONN.to_string(),
        observed: at(WIRE, 2),
        event: ConnectionEvent::Open {
            alpn: Some("http/1.1".to_string()),
            already_open: false,
        },
    }
}

fn claude_native() -> Vec<CaptureRecord> {
    epoch(
        CLAUDE,
        stream(
            CLAUDE,
            1,
            CaptureSource::Native,
            native_meta("claude-code-jsonl"),
            &claude_session(),
        ),
    )
}

/// A Codex rollout: one call and its result, and two cumulative token totals of the session.
fn codex_native() -> Vec<CaptureRecord> {
    let total = |input: u64, cached: u64, output: u64| {
        json!({"type": "event_msg", "payload": {"type": "token_count", "info": {
            "last_token_usage": {"input_tokens": 999, "output_tokens": 999},
            "total_token_usage": {"input_tokens": input, "cached_input_tokens": cached,
                "output_tokens": output, "reasoning_output_tokens": 4}}}})
    };
    let lines = jsonl(&[
        json!({"type": "session_meta", "payload": {"id": "session-1", "source": "cli"}}),
        json!({"type": "turn_context", "payload": {"turn_id": "turn-1", "model": "gpt-test"}}),
        json!({"type": "response_item", "payload": {"type": "function_call",
            "call_id": "call_1", "name": "shell", "arguments": "{}"}}),
        json!({"type": "response_item", "payload": {"type": "function_call_output",
            "call_id": "call_1", "output": "exit 1"}}),
        total(100, 60, 10),
        total(300, 200, 25),
    ]);
    epoch(
        CODEX,
        stream(
            CODEX,
            1,
            CaptureSource::Native,
            native_meta("codex-jsonl"),
            &lines,
        ),
    )
}

// ---------------------------------------------------------------------------------------
// Traces.
// ---------------------------------------------------------------------------------------

/// Every shipped connector's `continuation`: the participant types the prompt.
const CHAT: &str = "chat: the participant types the prompt and may type keep going";

fn records_of(continuation: &str, parts: Vec<Vec<CaptureRecord>>) -> Vec<CaptureRecord> {
    let mut records = goal();
    let mut launch = launch("claude-code-cli", None, None);
    if let CaptureRecord::Launch {
        continuation: c, ..
    } = &mut launch
    {
        *c = continuation.to_string();
    }
    records.push(launch);
    records.extend(parts.into_iter().flatten());
    records
}

fn trace_with(continuation: &str, parts: Vec<Vec<CaptureRecord>>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write_trace(dir.path(), &records_of(continuation, parts), true);
    dir
}

fn trace(parts: Vec<Vec<CaptureRecord>>, sealed: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write_trace(dir.path(), &records_of(CHAT, parts), sealed);
    dir
}

fn stats_of(traces: &[&Path]) -> RunStatsResult {
    let paths: Vec<PathBuf> = traces.iter().map(|p| p.to_path_buf()).collect();
    run_stats(&paths, &VerifyOptions::default()).unwrap()
}

fn claude_run() -> tempfile::TempDir {
    trace(vec![anthropic_wire(), claude_native()], true)
}

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_goalcup-verify"))
}

// ---------------------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------------------

#[test]
fn tokens_come_from_one_named_source_and_count_each_reply_once() {
    let run = claude_run();
    let result = stats_of(&[run.path()]);
    assert_eq!(result.exit_code, 0, "{}", result.stats);
    let tokens = &result.stats["tokens"];
    // The wire: input and cache from each message_start; output from the last cumulative
    // message_delta only (7 + 11 + 13), never added to the start's 1.
    assert_eq!(tokens["source"], "wire");
    assert_eq!(tokens["responses"], 3);
    assert_eq!(tokens["responses_without_usage"], 0);
    assert_eq!(tokens["input"], 60);
    assert_eq!(tokens["cache_read"], 600);
    assert_eq!(tokens["cache_write"], 0);
    assert_eq!(tokens["output"], 31);
    assert_eq!(
        tokens["reasoning"],
        Value::Null,
        "Anthropic never states it"
    );
    assert_eq!(tokens["lower_bound"], false);
    // The session file's numbers stay beside, never added: msg_1 is written twice and counts
    // once.
    let native = &tokens["other_source"];
    assert_eq!(native["source"], "native");
    assert_eq!(native["responses"], 2);
    assert_eq!(native["input"], 90);
    assert_eq!(native["cache_read"], 1900);
    assert_eq!(native["cache_write"], 80);
    assert_eq!(native["output"], 15);
    // Models of the token source, one per reply.
    assert_eq!(
        result.stats["models"],
        json!([{"model": "claude-test-model", "responses": 3}])
    );
    assert_eq!(result.stats["models_omitted"], 0);
}

#[test]
fn a_degraded_wire_hands_the_tokens_to_the_session_files() {
    let mut wire = anthropic_wire_records();
    // A lost piece of the first reply: the wire decoder can no longer say it read everything.
    let at_reply = wire
        .iter()
        .position(|r| matches!(r, CaptureRecord::End { stream, .. } if *stream == key(WIRE, 2)))
        .unwrap();
    wire.insert(
        at_reply,
        CaptureRecord::Gap {
            stream: Some(key(WIRE, 2)),
            observed: at(WIRE, 500),
            reason: GapReason::DecodeFailed,
            lost_bytes: Some(10),
        },
    );
    let run = trace(vec![epoch(WIRE, wire.clone()), claude_native()], true);
    let result = stats_of(&[run.path()]);
    assert_eq!(result.stats["tokens"]["source"], "native");
    assert_eq!(result.stats["tokens"]["input"], 90);
    assert_eq!(result.stats["tokens"]["lower_bound"], false);
    assert_eq!(result.stats["tokens"]["other_source"]["source"], "wire");
    assert_eq!(result.stats["analysis"]["state"], "partial");
    assert_eq!(
        result.stats["analysis"]["degraded"],
        json!([anthropic::DECODER_ID])
    );
    assert_eq!(result.exit_code, 1);

    // With no session file the degraded wire is all there is: at least its numbers.
    let wire_only = trace(vec![epoch(WIRE, wire)], true);
    let tokens = &stats_of(&[wire_only.path()]).stats["tokens"];
    assert_eq!(tokens["source"], "wire");
    assert_eq!(tokens["lower_bound"], true);
}

#[test]
fn replayed_tool_results_count_once_and_errors_only_where_the_harness_marks_them() {
    let claude = claude_run();
    let tools = &stats_of(&[claude.path()]).stats["tools"];
    // toolu_1 (native and wire), toolu_2 (native), toolu_3 (wire): three calls. toolu_1's
    // error is replayed in two requests and written once natively: one error.
    assert_eq!(tools["calls"], 3);
    assert_eq!(tools["calls_without_id"], 0);
    assert_eq!(tools["results"], 3);
    assert_eq!(tools["errors"], 1);
    assert_eq!(tools["error_marking"], "is_error");

    // Codex marks no tool error the decoders read: unknown, not zero.
    let codex = trace(vec![codex_native()], true);
    let stats = stats_of(&[codex.path()]).stats;
    assert_eq!(stats["tools"]["calls"], 1);
    assert_eq!(stats["tools"]["results"], 1);
    assert_eq!(stats["tools"]["errors"], Value::Null);
    assert_eq!(stats["tools"]["error_marking"], "none");
    // The last cumulative total of the session, never the per-call counts. Codex counts per
    // session, so there is no response count, and it states no cache write.
    let tokens = &stats["tokens"];
    assert_eq!(tokens["source"], "native");
    assert_eq!(tokens["input"], 100);
    assert_eq!(tokens["cache_read"], 200);
    assert_eq!(tokens["cache_write"], Value::Null);
    assert_eq!(tokens["output"], 25);
    assert_eq!(tokens["reasoning"], 4);
    assert_eq!(tokens["responses"], Value::Null);
    assert_eq!(stats["models_counted_per"], "turn");
}

#[test]
fn keep_going_is_counted_after_the_prompt_and_subagents_once() {
    let run = claude_run();
    let stats = stats_of(&[run.path()]).stats;
    assert_eq!(
        stats["human_messages"],
        json!({"source": "native", "total": 3, "first_is_prompt": true,
            "keep_going": 1, "other": 1})
    );
    assert_eq!(stats["subagents"]["spawned"], 1);

    // A launch that is not chat mode delivered the goal itself: every human message is a
    // later one, the first included.
    let delivered = trace_with("exec", vec![claude_native()]);
    assert_eq!(
        stats_of(&[delivered.path()]).stats["human_messages"],
        json!({"source": "native", "total": 3, "first_is_prompt": false,
            "keep_going": 1, "other": 2})
    );

    // No session file: whether a human typed cannot be read, so it is unknown.
    let wire_only = trace(vec![anthropic_wire()], true);
    let stats = stats_of(&[wire_only.path()]).stats;
    assert_eq!(stats["human_messages"]["source"], "none");
    assert_eq!(stats["human_messages"]["keep_going"], Value::Null);
}

#[test]
fn openai_responses_subtract_cached_input_and_count_a_retry_as_its_own_reply() {
    // resp_1, then a retry of the same request that the provider answered as resp_2.
    let wire = ws_run(
        openai::PROTOCOL_HINT,
        &[
            (
                CaptureSource::WsClient,
                json!({"type": "response.create", "model": "gpt-test",
                "input": []}),
            ),
            (CaptureSource::WsServer, openai_created("resp_1")),
            (CaptureSource::WsServer, openai_completed("resp_1")),
            (
                CaptureSource::WsClient,
                json!({"type": "response.create", "model": "gpt-test",
                "input": []}),
            ),
            (CaptureSource::WsServer, openai_created("resp_2")),
            (CaptureSource::WsServer, openai_completed("resp_2")),
        ],
    );
    let run = trace(vec![wire], true);
    let stats = stats_of(&[run.path()]).stats;
    let tokens = &stats["tokens"];
    assert_eq!(tokens["source"], "wire");
    assert_eq!(tokens["responses"], 2);
    assert_eq!(tokens["responses_without_usage"], 0);
    assert_eq!(tokens["input"], 80, "100 - 60 cached, twice");
    assert_eq!(tokens["cache_read"], 120);
    assert_eq!(tokens["cache_write"], Value::Null, "OpenAI never states it");
    assert_eq!(tokens["output"], 40);
    assert_eq!(tokens["reasoning"], 10, "a part of output, not added to it");
    assert_eq!(
        stats["models"],
        json!([{"model": "gpt-test", "responses": 2}])
    );
}

#[test]
fn chat_completions_count_the_usage_chunk_with_deepseek_cache_hits() {
    let request = json!({"model": "deepseek-test", "stream": true,
        "stream_options": {"include_usage": true},
        "messages": [{"role": "user", "content": "x"}]});
    let reply = [
        json!({"id": "chatcmpl-1", "model": "deepseek-test",
            "choices": [{"index": 0, "delta": {"role": "assistant", "content": "ok"}}]}),
        json!({"id": "chatcmpl-1", "model": "deepseek-test", "choices": [],
            "usage": {"prompt_tokens": 50, "completion_tokens": 7, "total_tokens": 57,
                "prompt_cache_hit_tokens": 30, "prompt_cache_miss_tokens": 20}}),
    ]
    .iter()
    .map(|chunk| format!("data: {chunk}\n\n"))
    .collect::<String>()
        + "data: [DONE]\n\n";
    let mut wire = vec![connection_open()];
    wire.extend(stream(
        WIRE,
        1,
        CaptureSource::WireRequest,
        wire_meta(CaptureSource::WireRequest, 1),
        request.to_string().as_bytes(),
    ));
    wire.extend(stream(
        WIRE,
        2,
        CaptureSource::WireResponse,
        wire_meta(CaptureSource::WireResponse, 1),
        reply.as_bytes(),
    ));
    wire.push(connection_close());
    let run = trace(vec![epoch(WIRE, wire)], true);
    let tokens = &stats_of(&[run.path()]).stats["tokens"];
    assert_eq!(tokens["source"], "wire");
    assert_eq!(tokens["responses"], 1);
    assert_eq!(tokens["responses_without_usage"], 0);
    assert_eq!(tokens["input"], 20);
    assert_eq!(tokens["cache_read"], 30);
    assert_eq!(tokens["output"], 7);
    assert_eq!(tokens["cache_write"], Value::Null);
    assert_eq!(tokens["reasoning"], Value::Null);
}

#[test]
fn amp_snapshots_count_once_per_message_and_never_without_an_id() {
    // The same message three times as it grows, then a snapshot with no message id, which
    // cannot be told from a repeat.
    let wire = ws_run(
        thread::PROTOCOL_HINT,
        &[
            (CaptureSource::WsServer, amp_snapshot(Some("msg-1"), 10)),
            (CaptureSource::WsServer, amp_snapshot(Some("msg-1"), 40)),
            (CaptureSource::WsServer, amp_snapshot(Some("msg-1"), 40)),
            (CaptureSource::WsServer, amp_snapshot(None, 999)),
        ],
    );
    let run = trace(vec![wire], true);
    let stats = stats_of(&[run.path()]).stats;
    let tokens = &stats["tokens"];
    assert_eq!(tokens["source"], "wire");
    assert_eq!(tokens["responses"], 1);
    assert_eq!(tokens["input"], 4);
    assert_eq!(tokens["cache_write"], 300);
    assert_eq!(tokens["cache_read"], 9000);
    assert_eq!(tokens["output"], 40);
}

#[test]
fn model_paths_and_urls_are_hidden_and_a_resume_placeholder_is_no_reply() {
    let assistant = |id: &str, model: &str| {
        json!({"type": "assistant", "message": {"id": id, "model": model,
            "usage": {"input_tokens": 1, "output_tokens": 1},
            "content": [{"type": "text", "text": "x"}]}})
    };
    let session = jsonl(&[
        assistant("msg_a", "/Users/alice/models/q.gguf"),
        assistant("msg_b", "http://gpu-box.local:8080/v1"),
        assistant("msg_c", "good-model"),
        // Claude Code's resume placeholder: no call, all-zero usage.
        json!({"type": "assistant", "message": {"id": "msg_s", "model": "<synthetic>",
            "usage": {"input_tokens": 0, "output_tokens": 0},
            "content": [{"type": "text", "text": "x"}]}}),
    ]);
    let run = trace(
        vec![epoch(
            CLAUDE,
            stream(
                CLAUDE,
                1,
                CaptureSource::Native,
                native_meta("claude-code-jsonl"),
                &session,
            ),
        )],
        true,
    );
    let stats = stats_of(&[run.path()]).stats;
    assert_eq!(
        stats["models"],
        json!([{"model": "good-model", "responses": 1}])
    );
    assert_eq!(stats["models_omitted"], 2);
    assert_eq!(stats["tokens"]["responses"], 3);
    assert_eq!(stats["tokens"]["responses_without_usage"], 0);
    let text = stats.to_string();
    assert!(
        !text.contains("alice") && !text.contains("gpu-box"),
        "{text}"
    );
}

#[test]
fn a_codex_spawn_counts_once_and_a_wait_is_no_spawn() {
    let collab = |id: &str, tool: &str| {
        json!({"type": "event_msg", "payload": {"type": "item_completed", "item": {
            "type": "CollabAgentToolCall", "id": id, "tool": tool, "status": "completed",
            "receiver_thread_ids": []}}})
    };
    let call = |id: &str, name: &str| {
        json!({"type": "response_item", "payload": {"type": "function_call",
            "call_id": id, "name": name, "arguments": "{}"}})
    };
    let lines = jsonl(&[
        json!({"type": "session_meta", "payload": {"id": "session-1", "source": "cli"}}),
        call("call_spawn", "spawn_agent"),
        collab("call_spawn", "spawn_agent"),
        call("call_wait", "wait_agent"),
        collab("call_wait", "wait"),
    ]);
    let run = trace(
        vec![epoch(
            CODEX,
            stream(
                CODEX,
                1,
                CaptureSource::Native,
                native_meta("codex-jsonl"),
                &lines,
            ),
        )],
        true,
    );
    let stats = stats_of(&[run.path()]).stats;
    assert_eq!(stats["subagents"]["spawned"], 1);
    assert_eq!(stats["tools"]["calls"], 2);
}

#[test]
fn no_text_from_the_run_reaches_the_stats() {
    let run = claude_run();
    let text = serde_json::to_string(&stats_of(&[run.path()]).stats).unwrap();
    for secret in [
        MARKER,
        "build the parser",
        "please also add tests",
        "replayed output",
        "output of toolu_1",
        "toolu_",
        "msg_",
        "api.example.test",
        "/v1/messages",
        "Bash",
        "prompt-1",
    ] {
        assert!(!text.contains(secret), "{secret:?} is in {text}");
    }
}

#[test]
fn a_run_of_two_segments_sums_them_and_counts_repeated_ids_once() {
    let first = claude_run();
    // The second segment: the agent restarted, its session file repeats the first one's
    // messages (same ids), then one new reply.
    let mut session = claude_session();
    session.extend(jsonl(&[claude_assistant(
        "msg_3",
        json!({"input_tokens": 5, "output_tokens": 2}),
        json!([{"type": "tool_use", "id": "toolu_9", "name": "Bash", "input": {}}]),
    )]));
    let second = trace(
        vec![epoch(
            CLAUDE,
            stream(
                CLAUDE,
                1,
                CaptureSource::Native,
                native_meta("claude-code-jsonl"),
                &session,
            ),
        )],
        true,
    );
    let one = stats_of(&[first.path()]).stats;
    let both = stats_of(&[first.path(), second.path()]);
    assert_eq!(both.exit_code, 0);
    let stats = &both.stats;
    assert_eq!(stats["traces"].as_array().unwrap().len(), 2);
    let native = &stats["tokens"]["other_source"];
    assert_eq!(native["responses"], 3);
    assert_eq!(
        native["input"],
        one["tokens"]["other_source"]["input"].as_u64().unwrap() + 5
    );
    assert_eq!(
        native["output"],
        one["tokens"]["other_source"]["output"].as_u64().unwrap() + 2
    );
    assert_eq!(stats["tools"]["calls"], 4);
    assert_eq!(stats["human_messages"]["total"], 3);
    // The wire is in the first segment only: its numbers are unchanged.
    assert_eq!(stats["tokens"]["input"], one["tokens"]["input"]);
}

#[test]
fn an_unsealed_or_broken_trace_says_so_and_sets_the_exit_code() {
    let unsealed = trace(vec![claude_native()], false);
    let result = stats_of(&[unsealed.path()]);
    assert_eq!(result.exit_code, 1);
    let entry = &result.stats["traces"][0];
    assert_eq!(entry["sealed"], false);
    assert_eq!(entry["integrity"], "incomplete");
    assert_eq!(entry["exit_code"], 1);
    assert_eq!(entry["entry_id"], support::capture_records::ENTRY);

    // A changed byte in the chain: invalid evidence, counted up to where it holds.
    let broken = claude_run();
    let chain = broken.path().join(CHAIN_FILE);
    let text = std::fs::read_to_string(&chain).unwrap();
    std::fs::write(
        &chain,
        text.replacen("claude-code-cli", "claude-code-clj", 1),
    )
    .unwrap();
    let result = stats_of(&[broken.path()]);
    assert_eq!(result.exit_code, 2);
    assert_eq!(result.stats["traces"][0]["integrity"], "invalid");
}

#[cfg(unix)]
#[test]
fn the_cli_writes_one_private_file_and_leaves_none_on_failure() {
    use std::os::unix::fs::PermissionsExt as _;
    let run = claude_run();
    let unsealed = trace(vec![claude_native()], false);
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("run.json");
    let cli = bin()
        .arg("--stats")
        .arg(run.path())
        .arg(unsealed.path())
        .arg("--output")
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(
        cli.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );
    assert!(cli.stdout.is_empty());
    let written: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(
        written,
        stats_of(&[run.path(), unsealed.path()]).stats,
        "the CLI writes the library's document"
    );
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );

    // No usable document (a trace that does not exist, the same trace twice, an output inside
    // a trace, an existing output): exit 3, and no new file.
    let missing = out.path().join("missing");
    let refused = |args: &[&std::ffi::OsStr], output: &Path| {
        let cli = bin()
            .args(args)
            .arg("--output")
            .arg(output)
            .output()
            .unwrap();
        assert_eq!(cli.status.code(), Some(3), "{args:?}");
        assert!(cli.stdout.is_empty());
    };
    let other = out.path().join("other.json");
    refused(&["--stats".as_ref(), missing.as_os_str()], &other);
    refused(
        &[
            "--stats".as_ref(),
            run.path().as_os_str(),
            run.path().as_os_str(),
        ],
        &other,
    );
    assert!(!other.exists(), "a failed run leaves no file");
    refused(
        &["--stats".as_ref(), run.path().as_os_str()],
        &run.path().join("run.json"),
    );
    assert!(!run.path().join("run.json").exists());
    let before = std::fs::read(&file).unwrap();
    refused(&["--stats".as_ref(), run.path().as_os_str()], &file);
    assert_eq!(
        std::fs::read(&file).unwrap(),
        before,
        "an existing file is kept"
    );
}
