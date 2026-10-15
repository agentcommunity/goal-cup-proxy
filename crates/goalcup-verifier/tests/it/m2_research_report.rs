//! The private streaming research report: `write_research_report` and
//! `goalcup-verify --research-json TRACE --output REPORT.ndjson`
//! (`docs/research/m2-report-format.md`).
//!
//! Every trace is written by the real protocol writers: a v5 baseline and its
//! `baseline.json`, a goal stream ended `Complete`, a Launch naming it, ordinary capture
//! appends and, when sealed, `write_local_finish` (or the signed cutoff and finalization).
//! Expectations come from what each test wrote, or from the reader's verified facts where the
//! claim is that the report carries exactly those, never from the report under test. Decoder
//! inputs are synthetic unless a comment names a committed fixture.

use crate::common as support;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signer, SigningKey};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::capture::{
    CaptureMeta, CaptureRecord, CaptureSource, CaptureTime, ConnectionEvent, ConnectionOutcome,
    EndReason, GapReason, M2_PROTOCOL_VERSION, StreamKey, WireIdentity,
};
use goalcup_protocol::chain::Link;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::export::{ACKS_FILE, CHAIN_FILE, RECEIPT_FILE, TraceExport};
use goalcup_protocol::pins::CONNECTOR_DIR_SHA256S;
use goalcup_protocol::provenance::{
    CaptureUnavailableV1, CutoffReachedV1, FinalizationV1, NoRepositoryReasonV1,
    PreCutoffDrainOutcomeV1, PublisherNotAttemptedReasonV1, PublisherNotAttemptedV1,
    RepositoryNonSelectionV1, Rfc3339Utc, SnapshotNotAttemptedV1, TerminationFailureV1,
    TerminationMethodV1, TerminationOutcomeV1,
};
use goalcup_protocol::receipt::{StartReceipt, StartReceiptClaims};
use goalcup_protocol::redaction::{RedactionAudit, RedactionAuditSpan};
use goalcup_protocol::seal::{SealManifest, SealedFile};
use goalcup_verifier::capture_analysis::reader::scan_capture_trace;
use goalcup_verifier::capture_analysis::{anthropic, api::kinds, chat, openai};
use goalcup_verifier::{ResearchResult, VerifyOptions, write_research_report};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Write as _;
use std::path::Path;
use std::process::Command;
use support::records::key;

const ENTRY: &str = "entry-m2-t22";
/// Producer epochs: fresh 128-bit identifiers.
const GOAL: &str = "9d8c7b6a5f4e3d2c1b0a998877665544";
const CODEX: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90";
const CLAUDE: &str = "b2c3d4e5f60718293a4b5c6d7e8f90a1";
const WIRE: &str = "0123456789abcdef0123456789abcdef";
const RECOVERED: &str = "fedcba9876543210fedcba9876543210";
const ANTHROPIC: &str = "c3d4e5f60718293a4b5c6d7e8f90a1b2";
const CONN: &str = "5e1f0a2b3c4d5e6f708192a3b4c5d6e7";
const ANTHROPIC_CONN: &str = "d4e5f60718293a4b5c6d7e8f90a1b2c3";
const DAEMON: &str = "daemon-t22-clock";
const CODEX_HINT: &str = "codex-jsonl";
const CLAUDE_HINT: &str = "claude-code-jsonl";
const GOAL_TEXT: &[u8] = b"Build the parser and make its tests pass.";
/// A goal is retained in parts of at most this many bytes.
const GOAL_PART: usize = 32 * 1024;
const STARTED_AT: &str = "2026-09-29T11:59:30Z";
const DEADLINE_AT: &str = "2026-09-29T12:30:00Z";
const DECODERS: [&str; 5] = [
    "codex-native",
    "claude-code-native",
    "openai-responses",
    "anthropic-messages",
    chat::DECODER_ID,
];

// ---------------------------------------------------------------------------------------
// Records.
// ---------------------------------------------------------------------------------------

/// Tick `n` of `clock`: wall time and monotonic time move together.
fn at(clock: &str, tick: u64) -> CaptureTime {
    CaptureTime {
        wall: format!("2026-09-29T12:00:00.{tick:09}Z"),
        mono_ns: tick,
        clock_id: clock.to_string(),
    }
}

fn finish_time() -> CaptureTime {
    CaptureTime {
        wall: "2026-09-29T12:00:05.500Z".to_string(),
        mono_ns: 990_000,
        clock_id: DAEMON.to_string(),
    }
}

fn utc(value: &str) -> Rfc3339Utc {
    Rfc3339Utc::try_from(value).unwrap()
}

fn confirmed() -> TerminationOutcomeV1 {
    TerminationOutcomeV1::Confirmed {
        requested_at: utc("2026-09-29T12:00:02Z"),
        confirmed_at: utc("2026-09-29T12:00:03.125Z"),
        method: TerminationMethodV1::ProcessSignal,
    }
}

/// One capture stream: its key, source and recorder metadata.
struct Stream {
    key: StreamKey,
    source: CaptureSource,
    meta: CaptureMeta,
}

fn stream(producer: &str, id: u64, source: CaptureSource, meta: CaptureMeta) -> Stream {
    Stream {
        key: key(producer, id),
        source,
        meta,
    }
}

fn bare_meta(label: String, hint: Option<&str>) -> CaptureMeta {
    CaptureMeta {
        wire: None,
        label,
        protocol_hint: hint.map(str::to_string),
        method: None,
        host: None,
        path: None,
        status: None,
        content_type: None,
    }
}

fn native(producer: &str, id: u64, hint: &str) -> Stream {
    let label = format!("sessions/{hint}#{id}");
    stream(
        producer,
        id,
        CaptureSource::Native,
        bare_meta(label, Some(hint)),
    )
}

/// A wire stream of `hint` on `connection`. A WebSocket message carries its message index
/// and no content type; an HTTP request is a JSON `POST`, an HTTP response a `200` event
/// stream.
fn wire(
    producer: &str,
    id: u64,
    source: CaptureSource,
    hint: &str,
    connection: &str,
    exchange: u64,
) -> Stream {
    let ws = matches!(source, CaptureSource::WsClient | CaptureSource::WsServer);
    let path = match hint {
        openai::PROTOCOL_HINT => "/backend-api/codex/responses",
        anthropic::PROTOCOL_HINT => "/v1/messages",
        _ => "/v1/unknown",
    };
    let content_type = match source {
        CaptureSource::WireRequest => Some("application/json".to_string()),
        CaptureSource::WireResponse => Some("text/event-stream".to_string()),
        _ => None,
    };
    let meta = CaptureMeta {
        wire: Some(WireIdentity {
            connection: connection.to_string(),
            exchange,
            message_index: ws.then_some(id),
        }),
        label: format!("{hint} #{id}"),
        protocol_hint: Some(hint.to_string()),
        method: (source == CaptureSource::WireRequest).then(|| "POST".to_string()),
        host: Some("api.example.test".to_string()),
        path: Some(path.to_string()),
        status: (source == CaptureSource::WireResponse).then_some(200),
        content_type,
    };
    stream(producer, id, source, meta)
}

impl Stream {
    fn open(&self) -> CaptureRecord {
        CaptureRecord::Open {
            stream: self.key.clone(),
            observed: at(&self.key.producer, 1),
            source: self.source,
            metadata: self.meta.clone(),
        }
    }

    /// `bytes` cut at `splits` into parts 0.., part `p` observed over ticks
    /// `base + 10p ..= base + 10p + 5` of the producer's own clock.
    fn segments(&self, bytes: &[u8], splits: &[usize], base: u64) -> Vec<CaptureRecord> {
        let mut cuts = vec![0];
        cuts.extend_from_slice(splits);
        cuts.push(bytes.len());
        cuts.windows(2)
            .enumerate()
            .map(|(part, w)| {
                let part = part as u64;
                CaptureRecord::Segment {
                    stream: self.key.clone(),
                    part,
                    source_offset: w[0] as u64,
                    source_len: (w[1] - w[0]) as u64,
                    first: at(&self.key.producer, base + 10 * part),
                    last: at(&self.key.producer, base + 10 * part + 5),
                    data_b64: STANDARD.encode(&bytes[w[0]..w[1]]),
                    audit: RedactionAudit { spans: vec![] },
                }
            })
            .collect()
    }

    fn end(&self) -> CaptureRecord {
        CaptureRecord::End {
            stream: self.key.clone(),
            observed: at(&self.key.producer, 900_000),
            reason: EndReason::Complete,
        }
    }

    /// Open, the segments, End(Complete).
    fn records(&self, bytes: &[u8], splits: &[usize], base: u64) -> Vec<CaptureRecord> {
        let mut records = vec![self.open()];
        records.extend(self.segments(bytes, splits, base));
        records.push(self.end());
        records
    }
}

/// One producer's `records` followed by its receiver-attested close. Every record consumed at
/// least one frame, so the count bounds the final sequence.
fn epoch(producer: &str, mut records: Vec<CaptureRecord>) -> Vec<CaptureRecord> {
    let final_seq = records.len() as u64 - 1;
    records.push(CaptureRecord::ProducerClosed {
        producer: producer.to_string(),
        observed: at(DAEMON, 950_000),
        final_seq: Some(final_seq),
    });
    records
}

fn connection(producer: &str, connection: &str, event: ConnectionEvent) -> CaptureRecord {
    CaptureRecord::Connection {
        producer: producer.to_string(),
        connection: connection.to_string(),
        observed: at(producer, 2),
        event,
    }
}

fn connection_open(already_open: bool) -> ConnectionEvent {
    ConnectionEvent::Open {
        alpn: Some("http/1.1".to_string()),
        already_open,
    }
}

fn connection_close() -> ConnectionEvent {
    ConnectionEvent::Close {
        outcome: ConnectionOutcome::Complete,
    }
}

fn launch() -> CaptureRecord {
    CaptureRecord::Launch {
        observed: at(DAEMON, 5),
        goal_stream: key(GOAL, 1),
        profile_id: "codex-cli".to_string(),
        profile_digest: "cc".repeat(32),
        harness_version: "0.153.0".to_string(),
        recorder_revision: "t22-fixture".to_string(),
        continuation: "none".to_string(),
        allowed_tool_rule: None,
        process_handoff_digest: None,
        run_public_key: None,
        connector_dir_sha256: None,
        harness_build: None,
    }
}

fn assistance(tick: u64, action: &str) -> CaptureRecord {
    CaptureRecord::Assistance {
        observed: at(DAEMON, tick),
        action: action.to_string(),
    }
}

/// The goal stream (its own producer, closed) and the Launch naming it.
fn prefix(goal: &[u8]) -> Vec<CaptureRecord> {
    prefix_with(goal, launch())
}

fn prefix_with(goal: &[u8], launch: CaptureRecord) -> Vec<CaptureRecord> {
    let goal_stream = stream(
        GOAL,
        1,
        CaptureSource::Goal,
        bare_meta("goal".to_string(), None),
    );
    let splits: Vec<usize> = (GOAL_PART..goal.len()).step_by(GOAL_PART).collect();
    let mut records = epoch(GOAL, goal_stream.records(goal, &splits, 10));
    records.push(launch);
    records
}

fn jsonl(lines: &[Value]) -> Vec<u8> {
    lines
        .iter()
        .map(|line| format!("{line}\n"))
        .collect::<String>()
        .into_bytes()
}

// Decoder inputs.

fn session_meta(source: &str) -> Value {
    json!({"timestamp": "2026-09-29T11:59:59.000Z", "type": "session_meta",
        "payload": {"id": "session-1", "source": source}})
}

fn turn_context(model: &str) -> Value {
    json!({"timestamp": "2026-09-29T12:00:00.100Z", "type": "turn_context",
        "payload": {"turn_id": "turn-1", "model": model}})
}

fn token_count(last: u64, total: u64) -> Value {
    json!({"timestamp": "2026-09-29T12:00:00.200Z", "type": "event_msg", "payload": {
        "type": "token_count", "info": {
            "last_token_usage": {"input_tokens": last, "output_tokens": 10},
            "total_token_usage": {"input_tokens": total, "output_tokens": 40}}}})
}

/// A Codex user message; `marked` carries the `user.text` content kind.
fn codex_user(text: &str, marked: bool) -> Value {
    let kinds = if marked {
        json!(["user.text"])
    } else {
        json!([])
    };
    json!({"timestamp": "2026-09-29T12:00:00.300Z", "type": "response_item", "payload": {
        "type": "message", "role": "user",
        "content": [{"type": "input_text", "text": text}],
        "internal_chat_message_metadata_passthrough": {"content_item_kinds": kinds}}})
}

fn claude_user(text: &str, prompt_source: Option<&str>) -> Value {
    let mut record = json!({"type": "user", "timestamp": "2026-09-29T12:00:00.400Z",
        "promptId": "prompt-1", "message": {"role": "user", "content": text}});
    if let Some(source) = prompt_source {
        record["promptSource"] = json!(source);
    }
    record
}

fn claude_assistant(id: &str, input: u64, output: u64) -> Value {
    json!({"type": "assistant", "timestamp": "2026-09-29T12:00:00.500Z", "message": {
        "id": id, "model": "claude-test-model",
        "usage": {"input_tokens": input, "output_tokens": output},
        "content": [{"type": "text", "text": "done"}]}})
}

fn codex_records() -> Vec<CaptureRecord> {
    let lines = [
        session_meta("cli"),
        turn_context("gpt-5.6-luna"),
        token_count(100, 100),
    ];
    epoch(
        CODEX,
        native(CODEX, 1, CODEX_HINT).records(&jsonl(&lines), &[40], 100),
    )
}

fn ws(message: &Value) -> Vec<u8> {
    serde_json::to_vec(message).unwrap()
}

fn response_create(model: &str) -> Value {
    json!({"type": "response.create", "model": model, "stream": true, "store": false,
        "input": [{"type": "message", "role": "user",
            "content": [{"type": "input_text", "text": "context"}]}]})
}

fn response_created(id: &str) -> Value {
    json!({"type": "response.created", "sequence_number": 0, "response": {
        "id": id, "object": "response", "status": "in_progress", "created_at": 1_790_000_000,
        "model": "gpt-5.6-luna", "output": [], "usage": null, "error": null}})
}

fn response_completed(id: &str, input_tokens: u64) -> Value {
    json!({"type": "response.completed", "sequence_number": 9, "response": {
        "id": id, "object": "response", "status": "completed", "created_at": 1_790_000_000,
        "completed_at": 1_790_000_004, "model": "gpt-5.6-luna", "output": [],
        "usage": {"input_tokens": input_tokens, "output_tokens": 5,
            "total_tokens": input_tokens + 5},
        "error": null, "incomplete_details": null}})
}

/// WebSocket messages of one `openai-responses` connection: each message is one stream.
fn openai_messages(
    producer: &str,
    first_stream: u64,
    messages: &[(CaptureSource, Value)],
) -> Vec<CaptureRecord> {
    messages
        .iter()
        .zip(first_stream..)
        .flat_map(|((source, message), id)| {
            wire(producer, id, *source, openai::PROTOCOL_HINT, CONN, 1).records(
                &ws(message),
                &[],
                100 * id,
            )
        })
        .collect()
}

/// One WebSocket exchange: a request, the created response and its completion.
fn openai_records() -> Vec<CaptureRecord> {
    let mut records = vec![connection(WIRE, CONN, connection_open(false))];
    records.extend(openai_messages(
        WIRE,
        1,
        &[
            (CaptureSource::WsClient, response_create("gpt-5.6-luna")),
            (CaptureSource::WsServer, response_created("resp_1")),
            (CaptureSource::WsServer, response_completed("resp_1", 11)),
        ],
    ));
    records.push(connection(WIRE, CONN, connection_close()));
    epoch(WIRE, records)
}

fn ev(name: &str, data: Value) -> String {
    format!("event: {name}\ndata: {data}\n\n")
}

fn message_start(id: &str) -> String {
    ev(
        "message_start",
        json!({"type": "message_start", "message": {"id": id, "type": "message",
            "role": "assistant", "model": "claude-test-model", "content": [],
            "stop_reason": null, "usage": {"input_tokens": 3, "output_tokens": 1}}}),
    )
}

fn anthropic_message(id: &str) -> String {
    [
        message_start(id),
        ev(
            "content_block_start",
            json!({"type": "content_block_start", "index": 0,
                "content_block": {"type": "text", "text": ""}}),
        ),
        ev(
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "text_delta", "text": "hello"}}),
        ),
        ev(
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 0}),
        ),
        ev(
            "message_delta",
            json!({"type": "message_delta",
                "delta": {"stop_reason": "end_turn", "stop_sequence": null},
                "usage": {"output_tokens": 5}}),
        ),
        ev("message_stop", json!({"type": "message_stop"})),
    ]
    .concat()
}

/// One Anthropic Messages exchange whose response body is `sse`, cut at `splits`.
fn anthropic_records(sse: &str, splits: &[usize]) -> Vec<CaptureRecord> {
    let request = json!({"model": "claude-test-model", "max_tokens": 1024, "stream": true,
        "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}]});
    let hint = anthropic::PROTOCOL_HINT;
    let mut records = vec![connection(
        ANTHROPIC,
        ANTHROPIC_CONN,
        connection_open(false),
    )];
    records.extend(
        wire(
            ANTHROPIC,
            1,
            CaptureSource::WireRequest,
            hint,
            ANTHROPIC_CONN,
            1,
        )
        .records(request.to_string().as_bytes(), &[], 100),
    );
    records.extend(
        wire(
            ANTHROPIC,
            2,
            CaptureSource::WireResponse,
            hint,
            ANTHROPIC_CONN,
            1,
        )
        .records(sse.as_bytes(), splits, 300),
    );
    records.push(connection(ANTHROPIC, ANTHROPIC_CONN, connection_close()));
    epoch(ANTHROPIC, records)
}

fn replace_end_reason(
    records: &mut [CaptureRecord],
    producer: &str,
    stream_id: u64,
    reason: EndReason,
) {
    let record = records
        .iter_mut()
        .find(|record| {
            matches!(record, CaptureRecord::End { stream, .. }
                if stream.producer == producer && stream.stream == stream_id)
        })
        .expect("selected stream has an End");
    let CaptureRecord::End {
        reason: end_reason, ..
    } = record
    else {
        unreachable!()
    };
    *end_reason = reason;
}

/// Native and wire evidence from four of the five decoders (not Chat Completions), in this
/// order: Codex rollout, OpenAI
/// WebSocket, Anthropic HTTP, Claude Code transcript.
fn mixed_records() -> Vec<CaptureRecord> {
    let mut records = codex_records();
    records.extend(openai_records());
    records.extend(anthropic_records(&anthropic_message("msg_1"), &[50]));
    let claude = jsonl(&[claude_assistant("msg_n1", 5, 2)]);
    records.extend(epoch(
        CLAUDE,
        native(CLAUDE, 1, CLAUDE_HINT).records(&claude, &[], 100),
    ));
    records
}

// ---------------------------------------------------------------------------------------
// Traces.
// ---------------------------------------------------------------------------------------

fn manifest() -> SealManifest {
    SealManifest {
        entry_id: ENTRY.to_string(),
        sealed_at: "2026-09-29T12:00:00Z".to_string(),
        workspace: "/ws".to_string(),
        files: vec![SealedFile {
            path: "src/lib.rs".to_string(),
            bytes: 12,
            sha256: "ab".repeat(32),
        }],
        excluded: vec!["target".to_string()],
        git: None,
        home: None,
    }
}

/// A v5 export with its baseline (and `baseline.json`) written.
fn new_export(root: &Path) -> TraceExport {
    let mut export = TraceExport::create(root).unwrap();
    export.configure_capture_quota(1 << 40).unwrap();
    export
        .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest())
        .unwrap();
    export
}

fn append(export: &mut TraceExport, record: &CaptureRecord) -> Link {
    let mut env = Envelope {
        protocol_version: M2_PROTOCOL_VERSION,
        entry_id: ENTRY.to_string(),
        source: "daemon".to_string(),
        recorded_at: "2026-09-29T12:00:01Z".to_string(),
        link: Link {
            seq: 0,
            prev: String::new(),
            hash: String::new(),
        },
        body: EventBody::Capture {
            record: record.clone(),
        },
        redaction: None,
    };
    export.append_chain(&mut env).unwrap()
}

struct Trace {
    dir: tempfile::TempDir,
    export: TraceExport,
    /// The links of the caller's records, after the goal and Launch prefix.
    links: Vec<Link>,
    finish: Option<Link>,
}

impl Trace {
    fn path(&self) -> &Path {
        self.dir.path()
    }
}

/// A real v5 trace: baseline, the goal stream and its Launch, `records` through the ordinary
/// append and, when `finish` is given, the local finish. No server, receipt or ACK.
fn make_report_trace(
    goal: &[u8],
    records: &[CaptureRecord],
    finish: Option<TerminationOutcomeV1>,
) -> Trace {
    make_report_trace_with_launch(goal, launch(), records, finish)
}

fn make_report_trace_with_launch(
    goal: &[u8],
    launch: CaptureRecord,
    records: &[CaptureRecord],
    finish: Option<TerminationOutcomeV1>,
) -> Trace {
    let dir = tempfile::tempdir().unwrap();
    let mut export = new_export(dir.path());
    for record in prefix_with(goal, launch) {
        append(&mut export, &record);
    }
    let links = records
        .iter()
        .map(|record| append(&mut export, record))
        .collect();
    let finish = finish.map(|termination| {
        export
            .write_local_finish(&manifest(), finish_time(), "participant_stop", termination)
            .unwrap()
            .1
    });
    Trace {
        dir,
        export,
        links,
        finish,
    }
}

fn make_report_trace_without_launch(
    goal: &[u8],
    records: &[CaptureRecord],
    finish: Option<TerminationOutcomeV1>,
) -> Trace {
    let dir = tempfile::tempdir().unwrap();
    let mut export = new_export(dir.path());
    for record in prefix_with(goal, launch()) {
        if !matches!(record, CaptureRecord::Launch { .. }) {
            append(&mut export, &record);
        }
    }
    let links = records
        .iter()
        .map(|record| append(&mut export, record))
        .collect();
    let finish = finish.map(|termination| {
        export
            .write_local_finish(&manifest(), finish_time(), "participant_stop", termination)
            .unwrap()
            .1
    });
    Trace {
        dir,
        export,
        links,
        finish,
    }
}

/// The real report writer into a small test buffer: its result, bytes and parsed lines.
fn report_with(dir: &Path, options: &VerifyOptions) -> (ResearchResult, Vec<u8>, Vec<Value>) {
    let mut out = Vec::new();
    let result = write_research_report(dir, options, &mut out)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let lines = parse_lines(&out);
    (result, out, lines)
}

fn report_bytes(dir: &Path) -> (ResearchResult, Vec<u8>, Vec<Value>) {
    report_with(dir, &VerifyOptions::default())
}

/// Every complete (newline-terminated) line of `bytes`, parsed.
fn parse_lines(bytes: &[u8]) -> Vec<Value> {
    let text = std::str::from_utf8(bytes).unwrap();
    let complete = &text[..text.rfind('\n').map_or(0, |end| end + 1)];
    complete
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// The one summary line: the last line, and the only one of its type.
fn summary_of(lines: &[Value]) -> &Value {
    let summaries = lines.iter().filter(|l| l["type"] == "summary").count();
    assert_eq!(summaries, 1, "exactly one summary line");
    let last = lines.last().unwrap();
    assert_eq!(last["type"], "summary", "the summary is the last line");
    last
}

fn observations<'a>(lines: &'a [Value], decoder: &str, kind: &str) -> Vec<&'a Value> {
    lines
        .iter()
        .filter(|l| {
            l["type"] == "observation"
                && l["decoder"] == decoder
                && l["observation"]["kind"] == kind
        })
        .collect()
}

fn decoder_entry<'a>(summary: &'a Value, decoder: &str) -> &'a Value {
    summary["analysis"]["decoders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["decoder"] == decoder)
        .unwrap()
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

fn keys(value: &Value) -> Vec<String> {
    let mut into = Vec::new();
    keys_of(value, &mut into);
    into
}

/// The chain read back independently of the report.
fn chain_lines(dir: &Path) -> Vec<Envelope> {
    std::fs::read_to_string(dir.join(CHAIN_FILE))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn sign_ack(key: &SigningKey, link: &Link, acked_at: &str) -> AckCheckpoint {
    let mut ack = AckCheckpoint {
        entry_id: ENTRY.to_string(),
        seq: link.seq,
        hash: link.hash.clone(),
        acked_at: acked_at.to_string(),
        server_signature: String::new(),
    };
    ack.server_signature = hex::encode(key.sign(&ack.signing_bytes().unwrap()).to_bytes());
    ack
}

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_goalcup-verify"))
}

/// Every file under `dir`, with its length and digest.
fn snapshot(dir: &Path) -> Vec<(String, u64, String)> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                files.push((
                    path.display().to_string(),
                    bytes.len() as u64,
                    hex::encode(Sha256::digest(&bytes)),
                ));
            }
        }
    }
    files.sort();
    files
}

// ---------------------------------------------------------------------------------------
// Named cases.
// ---------------------------------------------------------------------------------------

#[test]
fn mixed_observations_end_in_one_bound_summary() {
    let trace = make_report_trace(GOAL_TEXT, &mixed_records(), Some(confirmed()));
    let (result, _, lines) = report_bytes(trace.path());
    let summary = summary_of(&lines);
    assert_eq!(&result.summary, summary);
    assert_eq!(result.exit_code, 0, "{summary:#}");
    assert_eq!(summary["exit_code"], 0);
    assert_eq!(summary["schema_version"], 3);
    assert_eq!(
        summary["verifier_version"],
        goalcup_verifier::VERIFIER_VERSION
    );

    // Every other line is an observation, attributed to the decoder of its own source, and
    // the decoders' first lines follow the chain order of their streams.
    let body = &lines[..lines.len() - 1];
    let producer_of = |decoder: &str| match decoder {
        "codex-native" => CODEX,
        "openai-responses" => WIRE,
        "anthropic-messages" => ANTHROPIC,
        "claude-code-native" => CLAUDE,
        other => panic!("unexpected decoder {other}"),
    };
    for line in body {
        assert_eq!(line["type"], "observation", "{line}");
        assert!(
            kinds::ALL.contains(&line["observation"]["kind"].as_str().unwrap()),
            "built-in output used an undeclared kind: {line}"
        );
        let decoder = line["decoder"].as_str().unwrap();
        assert_eq!(
            line["observation"]["source"]["stream"]["producer"],
            producer_of(decoder),
            "{line}"
        );
    }
    let first_line = |decoder: &str| body.iter().position(|l| l["decoder"] == decoder).unwrap();
    assert!(first_line("codex-native") < first_line("openai-responses"));
    assert!(first_line("openai-responses") < first_line("anthropic-messages"));
    assert!(first_line("anthropic-messages") < first_line("claude-code-native"));

    // Provenance: the validated head is the chain's last link, read back independently.
    let chain = chain_lines(trace.path());
    let chain_bytes = std::fs::metadata(trace.path().join(CHAIN_FILE))
        .unwrap()
        .len();
    assert_eq!(
        summary["input"],
        json!({"validated_head": chain.last().unwrap().link.hash,
            "validated_records": chain.len(), "validated_bytes": chain_bytes})
    );
    assert_eq!(summary["run"]["launch"], json!(launch()));
    assert_eq!(
        summary["options"],
        json!({"dev_mode": false, "allow_descriptor_key": false, "extra_server_keys": []})
    );

    // Each decoder: its declared identity and version, and counts that are its own lines.
    let versions = [
        "0.153.0",
        "2.1.259-2.1.283",
        openai::DECODER_VERSION,
        anthropic::DECODER_VERSION,
        chat::DECODER_VERSION,
    ];
    let entries = summary["analysis"]["decoders"].as_array().unwrap();
    assert_eq!(entries.len(), DECODERS.len());
    for ((entry, id), version) in entries.iter().zip(DECODERS).zip(versions) {
        assert_eq!(entry["decoder"], id);
        assert_eq!(entry["summary"]["decoder_id"], id);
        assert_eq!(entry["summary"]["decoder_version"], version);
        let written = body.iter().filter(|l| l["decoder"] == id).count();
        // The mixed trace carries no Chat Completions stream (`m2_chat_decoder.rs` and
        // `m2_hermes_capture_only.rs` hold that decoder's end-to-end cases), so the fifth
        // decoder is registered, ran, and found nothing: the pinned fixtures stay one row longer.
        if id == chat::DECODER_ID {
            assert_eq!(written, 0, "{id} saw no stream");
        } else {
            assert!(written > 0, "{id} wrote nothing");
        }
        assert_eq!(entry["written"], written, "{id}");
        assert_eq!(entry["summary"]["observations"], written, "{id}");
        assert_eq!(entry["summary"]["incomplete"], false, "{id}");
        assert_eq!(entry["failure"], Value::Null);
        assert_eq!(entry["over_budget"], 0);
    }
    assert_eq!(summary["analysis"]["state"], "available");
}

/// Contract S1: `--research-json` echoes the two new `Launch` fields, additively. Absent, they
/// are absent from `run.launch` (the pinned report bytes above); present, they appear.
#[test]
fn the_report_echoes_launch_keys_and_presents_connector_pin_provenance() {
    let run_key = hex::encode(
        ed25519_dalek::SigningKey::from_bytes(&[0x42; 32])
            .verifying_key()
            .to_bytes(),
    );
    let dir_digest = "ab".repeat(32);
    let mut populated = launch();
    if let CaptureRecord::Launch {
        run_public_key,
        connector_dir_sha256,
        ..
    } = &mut populated
    {
        *run_public_key = Some(run_key.clone());
        *connector_dir_sha256 = Some(dir_digest.clone());
    }
    let trace =
        make_report_trace_with_launch(GOAL_TEXT, populated, &codex_records(), Some(confirmed()));
    let (result, bytes, lines) = report_bytes(trace.path());
    let summary = summary_of(&lines);
    assert_eq!(result.exit_code, 0, "{summary:#}");
    assert_eq!(summary["run"]["launch"]["run_public_key"], json!(run_key));
    assert_eq!(
        summary["run"]["launch"]["connector_dir_sha256"],
        json!(dir_digest)
    );
    assert_eq!(summary["run"]["connector"], "Dev/unpinned");
    assert_eq!(
        hex::encode(Sha256::digest(unversioned(&bytes))),
        "d6a3efededdca07b46a30b0c1a088e29e46f83c39dfec3fce0d0059c49ac0dfa"
    );

    let (pinned_id, pinned_digest) = CONNECTOR_DIR_SHA256S[0];
    let cases = [
        (pinned_id, Some(pinned_digest), Some("Pinned")),
        (pinned_id, Some("ab"), Some("Dev/unpinned")),
        ("unknown-id", Some(pinned_digest), Some("Dev/unpinned")),
        ("unknown-id", Some("cd"), Some("Dev/unpinned")),
        (pinned_id, None, None),
    ];
    for (profile_id, digest_prefix, expected) in cases {
        let mut launch = launch();
        let CaptureRecord::Launch {
            profile_id: id,
            connector_dir_sha256,
            ..
        } = &mut launch
        else {
            unreachable!()
        };
        *id = profile_id.to_string();
        *connector_dir_sha256 = digest_prefix.map(|prefix| {
            if prefix.len() == 64 {
                prefix.to_string()
            } else {
                prefix.repeat(32)
            }
        });
        let trace =
            make_report_trace_with_launch(GOAL_TEXT, launch, &codex_records(), Some(confirmed()));
        let (result, _, lines) = report_bytes(trace.path());
        let summary = summary_of(&lines);
        assert_eq!(result.exit_code, 0, "{profile_id}: {summary:#}");
        assert_eq!(
            summary["run"]["connector"].as_str(),
            expected,
            "{profile_id}: {summary:#}"
        );
    }

    let trace = make_report_trace_without_launch(GOAL_TEXT, &codex_records(), Some(confirmed()));
    let (result, _, lines) = report_bytes(trace.path());
    let summary = summary_of(&lines);
    assert_eq!(result.exit_code, 1, "{summary:#}");
    assert!(summary["run"]["connector"].is_null(), "{summary:#}");
}

#[test]
fn complete_local_report_without_server_exits_zero() {
    let trace = make_report_trace(GOAL_TEXT, &codex_records(), Some(confirmed()));
    assert!(!trace.path().join(RECEIPT_FILE).exists());
    let acks = std::fs::metadata(trace.path().join(ACKS_FILE)).map_or(0, |m| m.len());
    assert_eq!(acks, 0, "no acknowledgement");
    let (result, _, lines) = report_bytes(trace.path());
    let summary = summary_of(&lines);

    assert_eq!(result.exit_code, 0, "{summary:#}");
    assert_eq!(summary["capture"]["integrity"], "valid");
    assert_eq!(summary["capture"]["sealed"], true);
    assert_eq!(summary["capture"]["coverage_incomplete"], false);
    assert_eq!(summary["key_source"], "none");
    assert_eq!(
        summary["delivery"],
        json!({"anchor": "local_only", "state": "local_only", "verified_ack": null,
            "receipt_clock": null, "records_after_deadline": null,
            "last_seq_at_or_before_deadline": null, "last_ack_at": null})
    );
    // The typed terminal, exactly as the local finish wrote it.
    let finish = trace.finish.as_ref().unwrap();
    let chain = chain_lines(trace.path());
    let EventBody::Capture {
        record:
            CaptureRecord::LocalFinish {
                seal_manifest_sha256,
                ..
            },
    } = &chain.last().unwrap().body
    else {
        panic!("the chain ends in its local finish");
    };
    assert_eq!(
        summary["run"]["terminal"],
        json!({"seq": finish.seq, "hash": finish.hash, "observed": finish_time(),
            "observed_at": finish_time().wall, "kind": "local_finish",
            "reason": "participant_stop", "termination": confirmed(),
            "seal_manifest_sha256": seal_manifest_sha256})
    );
    // No signed duration, and no duration of any kind, is claimed. The deadline counts are the
    // one exception to the word "deadline": they are in the delivery block, and the equality
    // above pins them to null for a trace with no receipt.
    for key in keys(summary) {
        for word in ["duration", "deadline", "elapsed", "latency"] {
            let deadline_fact = key.ends_with("records_after_deadline")
                || key.ends_with("last_seq_at_or_before_deadline");
            assert!(deadline_fact || !key.contains(word), "{key}");
        }
    }
    assert_eq!(summary["analysis"]["state"], "available");
}

#[test]
fn faithful_wire_interruptions_are_report_detail_not_analysis_loss() {
    for reason in [EndReason::Interrupted, EndReason::DecodeError] {
        let mut openai_records = vec![connection(WIRE, CONN, connection_open(false))];
        let openai_sse = format!("data: {}\n\n", response_created("resp_early"));
        let mut message = wire(
            WIRE,
            1,
            CaptureSource::WireResponse,
            openai::PROTOCOL_HINT,
            CONN,
            1,
        )
        .records(openai_sse.as_bytes(), &[], 100);
        replace_end_reason(&mut message, WIRE, 1, reason);
        openai_records.extend(message);
        openai_records.push(connection(WIRE, CONN, connection_close()));
        let openai_records = epoch(WIRE, openai_records);

        let mut anthropic_records = anthropic_records(&message_start("msg_early"), &[]);
        replace_end_reason(&mut anthropic_records, ANTHROPIC, 2, reason);

        for (decoder, records) in [
            (openai::DECODER_ID, openai_records),
            (anthropic::DECODER_ID, anthropic_records),
        ] {
            let trace = make_report_trace(GOAL_TEXT, &records, Some(confirmed()));
            let (result, _, lines) = report_bytes(trace.path());
            let summary = summary_of(&lines);
            assert_eq!(result.exit_code, 0, "{decoder} {reason:?}: {summary:#}");
            assert_eq!(summary["capture"]["integrity"], "valid");
            assert_eq!(summary["capture"]["coverage_incomplete"], false);
            assert_eq!(summary["analysis"]["state"], "available");
            if decoder == openai::DECODER_ID {
                let cut: Vec<_> = lines
                    .iter()
                    .filter(|line| {
                        line["decoder"] == decoder && line["observation"]["kind"] == kinds::UNKNOWN
                    })
                    .collect();
                assert_eq!(cut.len(), 1, "{decoder} {reason:?}");
                assert_eq!(
                    cut[0]["observation"]["fields"]["reason"],
                    match reason {
                        EndReason::Interrupted => "interrupted",
                        EndReason::DecodeError => "decode_error",
                        EndReason::Complete => unreachable!(),
                    }
                );
            }
            let entry = summary["analysis"]["decoders"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["decoder"] == decoder)
                .unwrap();
            assert_eq!(
                entry["summary"]["incomplete"], false,
                "{decoder} {reason:?}"
            );
        }
    }
}

#[test]
fn chat_choice_cap_changes_whole_report_from_available_to_partial() {
    for (count, expected_exit, expected_state) in
        [(16usize, 0, "available"), (17usize, 1, "partial")]
    {
        let mut body = Vec::new();
        for index in 0..count {
            let event = json!({"model": "bounded-model", "choices": [
                {"index": index, "delta": {"content": "x"}}
            ]});
            body.extend_from_slice(format!("data: {event}\n\n").as_bytes());
        }
        body.extend_from_slice(b"data: [DONE]\n\n");
        let mut records = vec![connection(WIRE, CONN, connection_open(false))];
        records.extend(
            wire(
                WIRE,
                1,
                CaptureSource::WireResponse,
                chat::PROTOCOL_HINT,
                CONN,
                1,
            )
            .records(&body, &[], 100),
        );
        records.push(connection(WIRE, CONN, connection_close()));
        let trace = make_report_trace(GOAL_TEXT, &epoch(WIRE, records), Some(confirmed()));
        let (result, _, lines) = report_bytes(trace.path());
        let summary = summary_of(&lines);
        assert_eq!(result.exit_code, expected_exit, "{count}: {summary:#}");
        assert_eq!(summary["analysis"]["state"], expected_state);
        let entry = summary["analysis"]["decoders"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["decoder"] == chat::DECODER_ID)
            .unwrap();
        assert_eq!(entry["summary"]["incomplete"], count > 16);
    }
}

#[test]
fn verified_reader_facts_are_the_only_terminal_and_delivery_authority() {
    // A public, test-only key the verifier never trusts, supplied
    // as `--dev-key` would supply it.
    let key = SigningKey::from_bytes(&[0x42; 32]);
    let options = VerifyOptions {
        extra_server_keys: vec![hex::encode(key.verifying_key().to_bytes())],
        ..VerifyOptions::default()
    };
    let reader_facts = |dir: &Path| {
        let report = scan_capture_trace(dir, &options, &mut |_| Ok(())).unwrap();
        (
            json!(report.verified_ack),
            json!(report.receipt_clock),
            json!(report.terminal),
            // The deadline facts travel with the delivery, exactly as the reader found them.
            json!([
                report.records_after_deadline,
                report.last_seq_at_or_before_deadline,
                report.last_ack_at
            ]),
        )
    };
    let delivered = |summary: &Value| {
        (
            summary["delivery"]["verified_ack"].clone(),
            summary["delivery"]["receipt_clock"].clone(),
            summary["run"]["terminal"].clone(),
            json!([
                summary["delivery"]["records_after_deadline"],
                summary["delivery"]["last_seq_at_or_before_deadline"],
                summary["delivery"]["last_ack_at"]
            ]),
        )
    };

    // (a) A verified ACK behind the local terminal is partial delivery; (b) one at the
    // terminal is delivery through it.
    for at_terminal in [false, true] {
        let mut trace = make_report_trace(GOAL_TEXT, &codex_records(), Some(confirmed()));
        let finish = trace.finish.clone().unwrap();
        let acked = if at_terminal {
            &finish
        } else {
            &trace.links[0]
        };
        let ack = sign_ack(&key, acked, "2026-09-29T12:01:00Z");
        trace.export.append_ack_synced(&ack).unwrap();
        let (result, _, lines) = report_with(trace.path(), &options);
        let summary = summary_of(&lines);
        assert_eq!(result.exit_code, 0, "{summary:#}");
        assert_eq!(delivered(summary), reader_facts(trace.path()));
        assert_eq!(
            summary["delivery"]["verified_ack"],
            json!({"seq": acked.seq, "hash": acked.hash, "acked_at": "2026-09-29T12:01:00Z"})
        );
        let state = if at_terminal {
            "through_terminal"
        } else {
            "partial"
        };
        assert_eq!(summary["delivery"]["state"], state);
        assert_eq!(summary["delivery"]["anchor"], "local_only");
        assert_eq!(summary["key_source"], "supplied");
        // The server's signed time is copied; with no receipt there is no deadline to count against.
        assert_eq!(summary["delivery"]["last_ack_at"], "2026-09-29T12:01:00Z");
        assert_eq!(summary["delivery"]["records_after_deadline"], Value::Null);

        if !at_terminal {
            // An ACK at the terminal under another key is not verified evidence: the reader
            // stops at the last verified ACK and the report says exactly that, never
            // delivery through the terminal.
            let forged = sign_ack(
                &SigningKey::from_bytes(&[0x17; 32]),
                &finish,
                "2026-09-29T12:02:00Z",
            );
            trace.export.append_ack_synced(&forged).unwrap();
            let (result, _, lines) = report_with(trace.path(), &options);
            let summary = summary_of(&lines);
            assert_eq!(result.exit_code, 2, "{summary:#}");
            assert_eq!(summary["capture"]["integrity"], "invalid");
            assert_eq!(delivered(summary), reader_facts(trace.path()));
            assert_eq!(summary["delivery"]["verified_ack"]["seq"], acked.seq);
            assert_eq!(summary["delivery"]["state"], "partial");
        }
    }

    // (c) A signed trace: the verified receipt clock is reported beside the terminal, and
    // the signed terminal keeps its own wall time with no capture time substituted.
    let dir = tempfile::tempdir().unwrap();
    let mut export = new_export(dir.path());
    let (_, head) = export.head().unwrap();
    let receipt = StartReceipt::sign(
        StartReceiptClaims {
            entry_id: ENTRY.to_string(),
            connector_id: "codex-cli".to_string(),
            workspace: "/ws".to_string(),
            session_id: "session-t22".to_string(),
            started_at: STARTED_AT.to_string(),
            deadline_at: DEADLINE_AT.to_string(),
            manifest_sha256: "11".repeat(32),
            chain_head: head,
            connector_dir_sha256: None,
            run_public_key: None,
            questionnaire_version: None,
        },
        &key,
    )
    .unwrap();
    export.write_receipt(&receipt).unwrap();
    for record in prefix(GOAL_TEXT).iter().chain(&codex_records()) {
        append(&mut export, record);
    }
    let digest = receipt.canonical_digest().unwrap();
    export
        .write_cutoff_for_protocol(
            M2_PROTOCOL_VERSION,
            ENTRY,
            "2026-09-29T12:00:08Z",
            CutoffReachedV1 {
                receipt_sha256: digest.clone(),
                deadline_at: utc(DEADLINE_AT),
                pre_cutoff_drain: PreCutoffDrainOutcomeV1::Complete,
            },
        )
        .unwrap();
    let final_link = export
        .write_finalization_for_protocol(
            M2_PROTOCOL_VERSION,
            ENTRY,
            "2026-09-29T12:00:09Z",
            FinalizationV1::CaptureUnavailable {
                receipt_sha256: digest,
                termination: confirmed(),
                capture: CaptureUnavailableV1::NotAttempted(SnapshotNotAttemptedV1 {
                    selection: RepositoryNonSelectionV1::None {
                        reason: NoRepositoryReasonV1::NotFound,
                    },
                }),
                publisher: PublisherNotAttemptedV1 {
                    reason: PublisherNotAttemptedReasonV1::SelectionFailed,
                },
            },
        )
        .unwrap();
    drop(export);
    let (result, _, lines) = report_with(dir.path(), &options);
    let summary = summary_of(&lines);
    assert_eq!(result.exit_code, 0, "{summary:#}");
    assert_eq!(delivered(summary), reader_facts(dir.path()));
    let delivery = &summary["delivery"];
    assert_eq!(delivery["anchor"], "signed");
    assert_eq!(delivery["state"], "none_acknowledged");
    assert_eq!(delivery["verified_ack"], Value::Null);
    assert_eq!(delivery["last_ack_at"], Value::Null);
    assert_eq!(
        delivery["receipt_clock"],
        json!({"started_at": STARTED_AT, "deadline_at": DEADLINE_AT})
    );
    // A verified, bound receipt gives the deadline counts a clock to be set against.
    assert!(delivery["records_after_deadline"].is_u64(), "{delivery:#}");
    assert_eq!(
        summary["run"]["terminal"],
        json!({"seq": final_link.seq, "hash": final_link.hash, "observed": null,
            "observed_at": "2026-09-29T12:00:09Z", "kind": "signed_finalization",
            "reason": null, "termination": confirmed(), "seal_manifest_sha256": null})
    );
}

#[test]
fn local_terminal_keeps_typed_termination_outcome() {
    let outcomes = [
        TerminationOutcomeV1::AlreadyStopped {
            checked_at: utc("2026-09-29T12:00:04Z"),
        },
        confirmed(),
        TerminationOutcomeV1::Unresolved {
            requested_at: utc("2026-09-29T12:00:02Z"),
            stopped_waiting_at: utc("2026-09-29T12:00:04Z"),
            reason: TerminationFailureV1::ConfirmationTimedOut,
        },
    ];
    for outcome in outcomes {
        let trace = make_report_trace(GOAL_TEXT, &codex_records(), Some(outcome.clone()));
        let (_, _, lines) = report_bytes(trace.path());
        let summary = summary_of(&lines);
        assert_eq!(summary["run"]["terminal"]["kind"], "local_finish");
        assert_eq!(summary["run"]["terminal"]["termination"], json!(outcome));
        assert_eq!(summary["capture"]["sealed"], true);
        if let TerminationOutcomeV1::Unresolved { .. } = outcome {
            // A successful seal does not turn an unresolved stop into a confirmed one.
            assert!(
                !keys(summary).iter().any(|k| k.contains("confirm")),
                "{summary:#}"
            );
        }
    }
}

#[test]
fn report_preserves_error_outcome_and_observation_intervals() {
    // Claude Code's real 2.1.283 `api_error` shape and the synthetic provider error forms
    // (none was observed on real traffic), each spanning two retained segments and carrying
    // text that must never reach the report.
    let secret = "upstream detail SECRET-7f3a";
    let claude_lines = jsonl(&[
        json!({"type": "system", "subtype": "api_error", "level": "error",
            "timestamp": "2026-09-29T12:00:02.250Z",
            "error": {"connection": {"code": "ECONNRESET"}, "message": secret},
            "retryAttempt": 1, "maxRetries": 10}),
    ]);
    let mut records = epoch(
        CLAUDE,
        native(CLAUDE, 1, CLAUDE_HINT).records(&claude_lines, &[30], 100),
    );
    let failed = json!({"type": "response.failed", "sequence_number": 3, "response": {
        "id": "resp_f", "status": "failed", "created_at": 1_790_000_100,
        "error": {"code": "server_error", "message": secret},
        "incomplete_details": null, "usage": null}});
    let mut openai = vec![connection(WIRE, CONN, connection_open(false))];
    openai.extend(
        wire(
            WIRE,
            1,
            CaptureSource::WsServer,
            openai::PROTOCOL_HINT,
            CONN,
            1,
        )
        .records(&ws(&failed), &[20], 200),
    );
    openai.push(connection(WIRE, CONN, connection_close()));
    records.extend(epoch(WIRE, openai));
    let start = message_start("msg_e");
    let sse = start.clone()
        + &ev(
            "error",
            json!({"type": "error", "error": {"type": "overloaded_error", "message": secret},
                "request_id": "req_e"}),
        );
    records.extend(anthropic_records(&sse, &[start.len() + 10]));
    let trace = make_report_trace(GOAL_TEXT, &records, Some(confirmed()));
    let (_, bytes, lines) = report_bytes(trace.path());
    let summary = summary_of(&lines);

    // Each error: its two source parts and the exact interval of those parts, in the
    // producer's own clock; the response body's parts start at tick 300.
    for (decoder, producer, base, field, value) in [
        ("claude-code-native", CLAUDE, 100, "code", "ECONNRESET"),
        ("openai-responses", WIRE, 200, "code", "server_error"),
        (
            "anthropic-messages",
            ANTHROPIC,
            300,
            "error_type",
            "overloaded_error",
        ),
    ] {
        let errors = observations(&lines, decoder, "error");
        assert_eq!(errors.len(), 1, "{decoder}");
        let error = &errors[0]["observation"];
        assert_eq!(error["source"]["first_part"], 0, "{decoder}");
        assert_eq!(error["source"]["last_part"], 1, "{decoder}");
        assert_eq!(
            error["observed"],
            json!({"first": at(producer, base), "last": at(producer, base + 15)}),
            "{decoder}"
        );
        // A bounded supplied code or type.
        assert_eq!(error["fields"][field], value, "{decoder}");
    }
    // Supplied times stay their own fields, as supplied.
    let native = &observations(&lines, "claude-code-native", "error")[0]["observation"];
    assert_eq!(
        native["fields"]["source_timestamp"],
        "2026-09-29T12:00:02.250Z"
    );
    let outcome = &observations(&lines, "openai-responses", "outcome")[0]["observation"];
    assert_eq!(outcome["fields"]["provider_created_at"], 1_790_000_100);

    // No message text, header dump or invented latency anywhere in the report.
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains("SECRET"));
    for line in &lines {
        for key in keys(line) {
            for word in ["latency", "header", "authorization"] {
                assert!(!key.to_ascii_lowercase().contains(word), "{key}");
            }
        }
    }
    // The error forms are labelled synthetic-only, and nothing claims live validation.
    for decoder in ["openai-responses", "anthropic-messages"] {
        let entry = decoder_entry(summary, decoder);
        assert!(!entry["measured"].as_str().unwrap().is_empty());
        let synthetic = entry["synthetic_only"].as_array().unwrap();
        assert!(
            synthetic
                .iter()
                .any(|form| form.as_str().unwrap().contains("error")),
            "{decoder}: {synthetic:?}"
        );
    }
    assert!(!text.contains("live-validated") && !text.contains("live_validated"));
}

#[test]
fn automatic_sink_recovery_keeps_gap_without_assistance() {
    // A wire epoch loses its sink mid-message; the sender records the loss and recovers on
    // a fresh epoch that re-observes the surviving connection. No human acted.
    let lost = wire(
        WIRE,
        1,
        CaptureSource::WsClient,
        openai::PROTOCOL_HINT,
        CONN,
        1,
    );
    let request = ws(&response_create("gpt-5.6-luna"));
    let mut records = vec![connection(WIRE, CONN, connection_open(false)), lost.open()];
    records.extend(lost.segments(&request[..10], &[], 100));
    let gap = CaptureRecord::Gap {
        stream: None,
        observed: at(DAEMON, 400),
        reason: GapReason::QueueFull,
        lost_bytes: None,
    };
    let recovery = CaptureRecord::Recovery {
        previous_producer: WIRE.to_string(),
        producer: RECOVERED.to_string(),
        observed: at(RECOVERED, 1),
        reason: GapReason::IngestLost,
    };
    records.push(gap.clone());
    let mut resumed = vec![
        recovery.clone(),
        connection(RECOVERED, CONN, connection_open(true)),
    ];
    resumed.extend(openai_messages(
        RECOVERED,
        1,
        &[
            (CaptureSource::WsClient, response_create("gpt-5.6-luna")),
            (CaptureSource::WsServer, response_created("resp_2")),
        ],
    ));
    resumed.push(connection(RECOVERED, CONN, connection_close()));
    records.extend(epoch(RECOVERED, resumed));
    let trace = make_report_trace(GOAL_TEXT, &records, Some(confirmed()));
    let (result, _, lines) = report_bytes(trace.path());
    let summary = summary_of(&lines);

    // The loss stays visible after the clean reconnect and the seal.
    assert_eq!(result.exit_code, 1, "{summary:#}");
    let capture = &summary["capture"];
    assert_eq!(capture["integrity"], "incomplete");
    assert_eq!(capture["sealed"], true);
    assert_eq!(capture["coverage_incomplete"], true);
    assert_eq!(capture["gap_count"], 1);
    let findings = capture["findings"].as_array().unwrap();
    for code in ["gap_recorded", "producer_recovered"] {
        assert!(findings.iter().any(|f| f == code), "{code}: {findings:?}");
    }
    assert_eq!(capture["record_counts"]["gap.queue_full"], 1);
    assert_eq!(capture["record_counts"]["recovery.ingest_lost"], 1);
    let references = summary["references"].as_array().unwrap();
    assert!(references.contains(&json!(gap)));
    assert!(references.contains(&json!(recovery)));
    // The recovery is a capture event, not assistance.
    assert_eq!(
        summary["assistance"],
        json!({"explicit_records": 0, "positive_native_markers": 0,
            "evidence": "none_observed"})
    );
    // After the reconnect the recovered epoch's traffic is decoded, and the lost message is
    // an explicit unknown, not a request.
    let created = observations(&lines, "openai-responses", "model_response");
    assert_eq!(created.len(), 1);
    assert_eq!(
        created[0]["observation"]["source"]["stream"]["producer"],
        RECOVERED
    );
    let requests = observations(&lines, "openai-responses", "model_request");
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]["observation"]["source"]["stream"]["producer"],
        RECOVERED
    );
    assert_eq!(summary["analysis"]["state"], "partial");
    assert_eq!(
        decoder_entry(summary, "openai-responses")["summary"]["incomplete"],
        true
    );
}

#[test]
fn unknown_protocol_is_integrity_valid_analysis_unavailable() {
    let hint = "x-unknown-protocol/9";
    let mut records = vec![connection(WIRE, CONN, connection_open(false))];
    records.extend(
        wire(WIRE, 1, CaptureSource::WireRequest, hint, CONN, 1).records(
            b"{\"ask\":\"something\"}",
            &[6],
            100,
        ),
    );
    records.extend(
        wire(WIRE, 2, CaptureSource::WireResponse, hint, CONN, 1).records(
            b"data: answer\n\n",
            &[],
            200,
        ),
    );
    // A known hint on a form its decoder does not read: Anthropic Messages has no WebSocket.
    let known = anthropic::PROTOCOL_HINT;
    records.extend(wire(WIRE, 3, CaptureSource::WsClient, known, CONN, 1).records(b"{}", &[], 300));
    records.push(connection(WIRE, CONN, connection_close()));
    let trace = make_report_trace(GOAL_TEXT, &epoch(WIRE, records), Some(confirmed()));
    let (result, _, lines) = report_bytes(trace.path());
    let summary = summary_of(&lines);

    assert_eq!(result.exit_code, 0, "{summary:#}");
    assert_eq!(summary["capture"]["integrity"], "valid");
    assert_eq!(summary["analysis"]["state"], "unavailable");
    // Nothing of the unknown protocol, and one counted unknown for the unread form: neither
    // is analysis.
    let observed: Vec<_> = lines[..lines.len() - 1]
        .iter()
        .map(|l| (l["decoder"].clone(), l["observation"]["kind"].clone()))
        .collect();
    assert_eq!(observed, [(json!("anthropic-messages"), json!("unknown"))]);
    // The operational facts are kept as counted records: one connection, three streams and
    // four segments beside the goal's one. Nothing counts calls.
    let counts = &summary["capture"]["record_counts"];
    assert_eq!(counts["connection.open"], 1);
    assert_eq!(counts["connection.close.complete"], 1);
    assert_eq!(counts["open.wire_request"], 1);
    assert_eq!(counts["open.wire_response"], 1);
    assert_eq!(counts["open.ws_client"], 1);
    assert_eq!(counts["open.goal"], 1);
    assert_eq!(counts["segment"], 5);
    for key in keys(summary) {
        assert!(!key.contains("call"), "{key}");
    }
}

/// D53: builds are recorded, not pinned, so a session file that names model work while the proxy
/// recorded no wire stream at all is noted for judges: a harness that bypassed the proxy, or a
/// fake. The note never changes the exit code.
#[test]
fn a_session_with_model_work_and_no_wire_stream_is_noted_not_failed() {
    let trace = make_report_trace(GOAL_TEXT, &codex_records(), Some(confirmed()));
    let (result, _, lines) = report_bytes(trace.path());
    let summary = summary_of(&lines);
    assert_eq!(result.exit_code, 0, "{summary:#}");
    let cross_leg = &summary["analysis"]["cross_leg"];
    assert_eq!(
        cross_leg["state"], "native_activity_without_wire_streams",
        "{cross_leg:#}"
    );
    assert_eq!(cross_leg["wire_streams"], 0);
    assert!(cross_leg["native_model_activity"].as_u64().unwrap() > 0);
    assert_eq!(
        summary["analysis"]["models"],
        json!([{"decoder": "codex-native", "model": "gpt-5.6-luna", "count": 1}])
    );
}

#[test]
fn native_and_wire_counts_remain_distinct() {
    // The same model is named by the native transcript twice and by the wire traffic of two
    // identical requests: each source keeps its own repeated observations.
    let lines = [
        session_meta("cli"),
        turn_context("gpt-5.6-luna"),
        turn_context("gpt-5.6-luna"),
    ];
    let mut records = epoch(
        CODEX,
        native(CODEX, 1, CODEX_HINT).records(&jsonl(&lines), &[], 100),
    );
    let mut wire_records = vec![connection(WIRE, CONN, connection_open(false))];
    wire_records.extend(openai_messages(
        WIRE,
        1,
        &[
            (CaptureSource::WsClient, response_create("gpt-5.6-luna")),
            (CaptureSource::WsClient, response_create("gpt-5.6-luna")),
            (CaptureSource::WsServer, response_created("resp_1")),
        ],
    ));
    wire_records.push(connection(WIRE, CONN, connection_close()));
    records.extend(epoch(WIRE, wire_records));
    let trace = make_report_trace(GOAL_TEXT, &records, Some(confirmed()));
    let (_, _, report) = report_bytes(trace.path());
    let summary = summary_of(&report);

    assert_eq!(observations(&report, "codex-native", "model").len(), 2);
    let wire_models = observations(&report, "openai-responses", "model");
    let side = |s: &str| {
        wire_models
            .iter()
            .filter(|l| l["observation"]["fields"]["side"] == s)
            .count()
    };
    assert_eq!((side("request"), side("response")), (2, 1));
    assert_eq!(
        observations(&report, "openai-responses", "model_request").len(),
        2
    );
    // Each decoder reports its own count; no total is formed across them.
    for decoder in ["codex-native", "openai-responses"] {
        let written = report
            .iter()
            .filter(|l| l["type"] == "observation" && l["decoder"] == decoder)
            .count();
        assert_eq!(decoder_entry(summary, decoder)["written"], written);
    }
    // Per decoder, no key names a total or a model, outside the two schema-2 analysis keys
    // that compare the legs (`analysis.cross_leg`, which sums each leg across its decoders) and
    // list the models each decoder saw (`analysis.models`, `analysis.models_omitted`).
    let mut analysis = summary["analysis"].as_object().unwrap().clone();
    let cross_leg = analysis.remove("cross_leg").unwrap();
    let models = analysis.remove("models").unwrap();
    analysis.remove("models_omitted").unwrap();
    let mut rest = summary.clone();
    rest["analysis"] = Value::Object(analysis);
    for key in keys(&rest) {
        assert!(!key.contains("total") && !key.contains("model"), "{key}");
    }
    // The native session names model work and the proxy recorded model calls.
    assert_eq!(cross_leg["state"], "consistent", "{cross_leg:#}");
    assert_eq!(cross_leg["wire_model_requests"], 2);
    assert!(cross_leg["wire_streams"].as_u64().unwrap() > 0);
    assert!(cross_leg["native_model_activity"].as_u64().unwrap() > 0);
    let models = models.as_array().unwrap();
    assert!(
        models.iter().any(|m| m["decoder"] == "codex-native"),
        "{models:#?}"
    );
}

#[test]
fn cumulative_usage_is_observed_not_totalled() {
    let codex = [
        session_meta("cli"),
        token_count(100, 1000),
        token_count(150, 1150),
    ];
    let mut records = epoch(
        CODEX,
        native(CODEX, 1, CODEX_HINT).records(&jsonl(&codex), &[], 100),
    );
    let claude = [
        claude_assistant("msg_a", 5, 2),
        claude_assistant("msg_b", 7, 3),
    ];
    records.extend(epoch(
        CLAUDE,
        native(CLAUDE, 1, CLAUDE_HINT).records(&jsonl(&claude), &[], 100),
    ));
    let mut wire_records = vec![connection(WIRE, CONN, connection_open(false))];
    wire_records.extend(openai_messages(
        WIRE,
        1,
        &[
            (CaptureSource::WsServer, response_completed("resp_1", 11)),
            (CaptureSource::WsServer, response_completed("resp_2", 13)),
        ],
    ));
    wire_records.push(connection(WIRE, CONN, connection_close()));
    records.extend(epoch(WIRE, wire_records));
    let trace = make_report_trace(GOAL_TEXT, &records, Some(confirmed()));
    let (_, _, lines) = report_bytes(trace.path());
    let summary = summary_of(&lines);

    // Each snapshot stays the observation it was, with the value its source supplied.
    let codex_usage = observations(&lines, "codex-native", "usage");
    let scoped = |scope: &str, field: &str| -> Vec<Value> {
        codex_usage
            .iter()
            .filter(|l| l["observation"]["fields"]["scope"] == scope)
            .map(|l| l["observation"]["fields"][field]["input_tokens"].clone())
            .collect()
    };
    assert_eq!(
        scoped("per_call", "last_token_usage"),
        [json!(100), json!(150)]
    );
    assert_eq!(
        scoped("cumulative", "total_token_usage"),
        [json!(1000), json!(1150)]
    );
    assert_eq!(observations(&lines, "claude-code-native", "usage").len(), 2);
    let wire_usage: Vec<_> = observations(&lines, "openai-responses", "usage")
        .iter()
        .map(|l| l["observation"]["fields"]["input_tokens"].clone())
        .collect();
    assert_eq!(wire_usage, [json!(11), json!(13)]);
    // The summary carries no usage figure at all: nothing to total or sum.
    for key in keys(summary) {
        assert!(!key.contains("token") && !key.contains("usage"), "{key}");
    }
}

#[test]
fn explicit_assistance_and_unknown_native_authorship_stay_distinct() {
    let claude = [
        claude_user("Build the parser.", Some("sdk")),
        claude_user("Please also add a test.", Some("typed")),
        claude_user("Unmarked follow-up.", None),
    ];
    let mut records = epoch(
        CLAUDE,
        native(CLAUDE, 1, CLAUDE_HINT).records(&jsonl(&claude), &[], 100),
    );
    let help = assistance(700, "operator_message");
    records.push(help.clone());
    let trace = make_report_trace(GOAL_TEXT, &records, Some(confirmed()));
    let (_, _, lines) = report_bytes(trace.path());
    let summary = summary_of(&lines);

    // Two evidence sources, counted apart.
    assert_eq!(
        summary["assistance"],
        json!({"explicit_records": 1, "positive_native_markers": 1, "evidence": "observed"})
    );
    let human = observations(&lines, "claude-code-native", "human_message");
    assert_eq!(human.len(), 1);
    assert_eq!(
        human[0]["observation"]["fields"]["text"],
        "Please also add a test."
    );
    // The delivered goal and the unmarked record stay indeterminate, never human.
    let unknown: Vec<_> = observations(&lines, "claude-code-native", "unknown")
        .iter()
        .map(|l| l["observation"]["fields"]["record_type"].clone())
        .collect();
    assert_eq!(unknown, [json!("user"), json!("user")]);
    assert!(
        summary["references"]
            .as_array()
            .unwrap()
            .contains(&json!(help))
    );
    assert_eq!(summary["capture"]["record_counts"]["assistance"], 1);
}

#[test]
fn torn_tail_emits_invalid_summary_and_exit_two() {
    let trace = make_report_trace(GOAL_TEXT, &codex_records(), None);
    let chain = trace.path().join(CHAIN_FILE);
    let validated = chain_lines(trace.path());
    std::fs::OpenOptions::new()
        .append(true)
        .open(&chain)
        .unwrap()
        .write_all(b"{\"protocol_version\":5,\"seq\"")
        .unwrap();
    let (result, _, lines) = report_bytes(trace.path());
    let summary = summary_of(&lines);

    assert_eq!(result.exit_code, 2, "{summary:#}");
    assert_eq!(summary["exit_code"], 2);
    let capture = &summary["capture"];
    assert_eq!(capture["integrity"], "invalid");
    assert_eq!(capture["torn_tail"], true);
    assert_eq!(capture["sealed"], false);
    assert_eq!(summary["run"]["terminal"], Value::Null);
    assert_eq!(
        summary["input"]["validated_head"],
        validated.last().unwrap().link.hash.as_str()
    );
    // The validated prefix is still useful: its observations are in the report.
    assert_eq!(observations(&lines, "codex-native", "model").len(), 1);
    assert_eq!(observations(&lines, "codex-native", "usage").len(), 2);
}

/// Accepts `limit` bytes, then fails every write; `flush_fails` fails every flush.
struct FailingWriter {
    limit: usize,
    flush_fails: bool,
    accepted: Vec<u8>,
}

impl std::io::Write for FailingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let room = self.limit - self.accepted.len();
        if room == 0 {
            return Err(std::io::Error::other("no space left"));
        }
        let n = buf.len().min(room);
        self.accepted.extend_from_slice(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.flush_fails {
            return Err(std::io::Error::other("flush failed"));
        }
        Ok(())
    }
}

#[test]
fn writer_failure_has_no_success_summary() {
    let trace = make_report_trace(GOAL_TEXT, &mixed_records(), Some(confirmed()));
    let (_, full, _) = report_bytes(trace.path());
    let summary_start = full[..full.len() - 1]
        .iter()
        .rposition(|b| *b == b'\n')
        .unwrap()
        + 1;
    for (limit, flush_fails) in [(0, false), (summary_start + 5, false), (full.len(), true)] {
        let mut writer = FailingWriter {
            limit,
            flush_fails,
            accepted: Vec::new(),
        };
        let result = write_research_report(trace.path(), &VerifyOptions::default(), &mut writer);
        assert!(result.is_err(), "limit {limit}: {result:?}");
        if !flush_fails {
            let lines = parse_lines(&writer.accepted);
            assert!(
                lines.iter().all(|l| l["type"] == "observation"),
                "limit {limit}: a summary was written"
            );
        }
    }
}

#[test]
fn summary_reference_budget_is_enforced() {
    // (a) More short operational facts than references: the first 128 in chain order are
    // kept (the goal reference first), and the rest are counted, never dropped silently.
    let records: Vec<_> = (0..200).map(|n| assistance(1_000 + n, "resume")).collect();
    let trace = make_report_trace(GOAL_TEXT, &records, Some(confirmed()));
    let (_, bytes, lines) = report_bytes(trace.path());
    let summary = summary_of(&lines);
    let references = summary["references"].as_array().unwrap();
    assert_eq!(references.len(), 128);
    assert_eq!(summary["references_omitted"], 201 - 128);
    assert_eq!(references[0]["kind"], "goal");
    assert_eq!(
        &references[1..],
        &records[..127].iter().map(|r| json!(r)).collect::<Vec<_>>()[..]
    );
    assert_eq!(summary["capture"]["record_counts"]["assistance"], 200);
    assert_eq!(summary["assistance"]["explicit_records"], 200);
    let summary_line = bytes.split(|b| *b == b'\n').rev().nth(1).unwrap();
    assert!(summary_line.len() < 64 * 1024, "{}", summary_line.len());

    // (b) Fewer, larger facts: the byte bound, not the count, stops them. A small fact after
    // the first omitted one is omitted too, so the kept ones stay a chain-order prefix.
    let action = "a".repeat(1000);
    let mut records: Vec<_> = (0..60).map(|n| assistance(1_000 + n, &action)).collect();
    records.push(assistance(2_000, "x"));
    let trace = make_report_trace(GOAL_TEXT, &records, Some(confirmed()));
    let (_, bytes, lines) = report_bytes(trace.path());
    let summary = summary_of(&lines);
    let references = summary["references"].as_array().unwrap();
    let kept = references.len();
    assert!(kept < 61 && kept > 1, "{kept}");
    assert_eq!(summary["references_omitted"], 62 - kept as u64);
    assert_eq!(references.last().unwrap(), &json!(records[kept - 2]));
    // The bound is on the references themselves; the array adds its brackets and commas.
    assert!(serde_json::to_vec(references).unwrap().len() <= 32 * 1024 + kept + 1);
    let summary_line = bytes.split(|b| *b == b'\n').rev().nth(1).unwrap();
    assert!(summary_line.len() < 64 * 1024, "{}", summary_line.len());
}

#[test]
fn large_goal_does_not_expand_summary() {
    // 1023 ASCII bytes, a two-byte character across the 1 KiB mark, then about 59 KiB more
    // ending in a marker.
    let mut goal = vec![b'a'; 1023];
    goal.extend_from_slice("é".as_bytes());
    goal.extend(std::iter::repeat_n(b'b', 60 * 1024 - goal.len()));
    goal.extend_from_slice(b"TAIL-MARKER");
    let trace = make_report_trace(&goal, &codex_records(), Some(confirmed()));
    let (result, bytes, lines) = report_bytes(trace.path());
    let summary = summary_of(&lines);

    assert_eq!(result.exit_code, 0, "{summary:#}");
    let reference = &summary["references"][0];
    assert_eq!(
        reference,
        &json!({"kind": "goal", "stream": key(GOAL, 1), "first_part": 0, "last_part": 1,
            "source_bytes": goal.len(), "end": "complete", "excerpt": "a".repeat(1023),
            "excerpt_truncated": true})
    );
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains("TAIL-MARKER"));
    assert!(!text.contains(&"b".repeat(64)));
    let summary_line = text.lines().last().unwrap();
    assert!(summary_line.len() < 8 * 1024, "{}", summary_line.len());
}

#[test]
fn library_and_cli_outputs_match() {
    let trace = make_report_trace(GOAL_TEXT, &mixed_records(), Some(confirmed()));
    let before = snapshot(trace.path());
    let (result, library, _) = report_bytes(trace.path());
    let out = tempfile::tempdir().unwrap();
    let report = out.path().join("report.ndjson");

    let run = bin()
        .arg("--research-json")
        .arg(trace.path())
        .arg("--output")
        .arg(&report)
        .output()
        .unwrap();
    assert_eq!(
        run.status.code(),
        Some(i32::from(result.exit_code)),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(run.stdout.is_empty(), "stdout stays empty");
    assert!(!run.stderr.is_empty(), "a summary line for the person");
    assert_eq!(std::fs::read(&report).unwrap(), library);
    use std::os::unix::fs::PermissionsExt as _;
    assert_eq!(
        std::fs::metadata(&report).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // Neither the library nor the CLI touched the trace.
    assert_eq!(snapshot(trace.path()), before);

    // Refused invocations exit 3 and write nothing: an existing output, an output inside
    // the trace, an output not named .ndjson, conflicting modes, a missing output.
    let refused = |args: &[&std::ffi::OsStr]| {
        let run = bin().args(args).output().unwrap();
        assert_eq!(
            run.status.code(),
            Some(3),
            "{args:?}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert!(run.stdout.is_empty());
    };
    let trace_arg = trace.path().as_os_str();
    refused(&[
        "--research-json".as_ref(),
        trace_arg,
        "--output".as_ref(),
        report.as_os_str(),
    ]);
    assert_eq!(
        std::fs::read(&report).unwrap(),
        library,
        "an existing output is kept"
    );
    let inside = trace.path().join("report.ndjson");
    refused(&[
        "--research-json".as_ref(),
        trace_arg,
        "--output".as_ref(),
        inside.as_os_str(),
    ]);
    assert!(!inside.exists());
    let named = out.path().join("report.json");
    refused(&[
        "--research-json".as_ref(),
        trace_arg,
        "--output".as_ref(),
        named.as_os_str(),
    ]);
    assert!(!named.exists());
    let other = out.path().join("other.ndjson");
    refused(&[
        "--capture-integrity".as_ref(),
        trace_arg,
        "--research-json".as_ref(),
        trace_arg,
        "--output".as_ref(),
        other.as_os_str(),
    ]);
    refused(&["--research-json".as_ref(), trace_arg]);
    refused(&[trace_arg, "--output".as_ref(), other.as_os_str()]);
    assert!(!other.exists());
    assert_eq!(snapshot(trace.path()), before);
}

#[test]
fn report_has_no_by_position_human_claim() {
    // Real Codex 0.153.0 exec-launch bytes (session_meta `exec`, then the delivered goal
    // carrying the `user.text` marker), followed by one synthetic later user record without
    // any marker.
    let real = include_str!("../fixtures/m2-native/codex-native-e01-excerpt.jsonl");
    let mut codex = real.as_bytes().to_vec();
    codex.extend(jsonl(&[codex_user("And now the docs.", false)]));
    let mut records = epoch(
        CODEX,
        native(CODEX, 1, CODEX_HINT).records(&codex, &[], 100),
    );
    // Claude Code: the sdk-delivered goal, then a later user record with no marker.
    let claude = [
        claude_user("Build the parser.", Some("sdk")),
        claude_user("And now the docs.", None),
    ];
    records.extend(epoch(
        CLAUDE,
        native(CLAUDE, 1, CLAUDE_HINT).records(&jsonl(&claude), &[], 100),
    ));
    let trace = make_report_trace(GOAL_TEXT, &records, Some(confirmed()));
    let (_, _, lines) = report_bytes(trace.path());
    let summary = summary_of(&lines);

    assert!(
        !lines
            .iter()
            .any(|l| l["observation"]["kind"] == "human_message"),
        "a later user record is not human by position"
    );
    let record_types = |decoder: &str| -> Vec<Value> {
        observations(&lines, decoder, "unknown")
            .iter()
            .map(|l| l["observation"]["fields"]["record_type"].clone())
            .collect()
    };
    let codex_types = record_types("codex-native");
    assert!(codex_types.contains(&json!("response_item.message.goal")));
    assert!(codex_types.contains(&json!("response_item.message")));
    assert_eq!(
        record_types("claude-code-native"),
        [json!("user"), json!("user")]
    );
    assert_eq!(
        summary["assistance"],
        json!({"explicit_records": 0, "positive_native_markers": 0,
            "evidence": "none_observed"})
    );
}

#[test]
fn invalid_redaction_projection_cannot_leak_retained_json_into_research_ndjson() {
    let secret = "raw-secret-bearing-value";
    let bytes = ws(&json!({"type": "response.create", "model": "secret-model",
        "stream": true, "store": false, "input": [{"type": "message", "role": "user",
            "content": [{"type": "input_text", "text": secret}]}]}));
    let selected = wire(
        WIRE,
        91,
        CaptureSource::WsClient,
        openai::PROTOCOL_HINT,
        CONN,
        1,
    );
    let mut segment = selected.segments(&bytes, &[], 100).remove(0);
    let CaptureRecord::Segment { audit, .. } = &mut segment else {
        unreachable!()
    };
    audit.spans.push(RedactionAuditSpan {
        kind: "email".into(),
        start: 0,
        end: 1,
    });
    let trace = make_report_trace(
        GOAL_TEXT,
        &epoch(WIRE, vec![selected.open(), segment, selected.end()]),
        Some(confirmed()),
    );
    let (result, bytes, rows) = report_bytes(trace.path());
    let report = String::from_utf8(bytes).unwrap();

    assert_eq!(result.exit_code, 1);
    assert!(!report.contains(secret));
    assert!(!report.contains("secret-model"));
    assert!(report.contains("invalid_redaction_audit"));
    assert_eq!(
        rows.iter()
            .filter(|row| row["type"] == "observation")
            .map(|row| row["observation"]["kind"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["unknown"]
    );
}

// ---------------------------------------------------------------------------------------
// Pinned report bytes (contract S7).
// ---------------------------------------------------------------------------------------

fn regenerate_report_bytes(value: Option<&std::ffi::OsStr>) -> bool {
    value == Some(std::ffi::OsStr::new("1"))
}

#[test]
fn report_regeneration_requires_the_exact_value_one() {
    use std::ffi::OsStr;

    assert!(!regenerate_report_bytes(None));
    assert!(!regenerate_report_bytes(Some(OsStr::new(""))));
    assert!(!regenerate_report_bytes(Some(OsStr::new("0"))));
    assert!(!regenerate_report_bytes(Some(OsStr::new("yes"))));
    assert!(regenerate_report_bytes(Some(OsStr::new("1"))));
}

/// A report names the verifier build that wrote it, and that version changes at every release.
/// Pinned bytes and digests are taken with it replaced by a marker, so a release does not
/// invalidate them; `summary["verifier_version"]` is asserted against `VERIFIER_VERSION` alone.
fn unversioned(bytes: &[u8]) -> Vec<u8> {
    let own = format!(
        "\"verifier_version\":\"{}\"",
        goalcup_verifier::VERIFIER_VERSION
    );
    let text = std::str::from_utf8(bytes).expect("a report is UTF-8");
    assert_eq!(
        text.matches(&own).count(),
        1,
        "the report names its verifier once"
    );
    text.replace(&own, "\"verifier_version\":\"@VERSION@\"")
        .into_bytes()
}

/// Whole-report bytes of five traces, pinned before the Decoder v1 port. A change of
/// decoder plumbing must leave `--research-json` byte-identical; only a deliberate change of
/// a decoder or of the report format may regenerate these (`REGENERATE_REPORT_BYTES=1`).
#[test]
fn report_bytes_are_pinned_for_representative_traces() {
    let unknown_hint = "x-unknown-protocol/9";
    let mut unknown = vec![connection(WIRE, CONN, connection_open(false))];
    unknown.extend(
        wire(WIRE, 1, CaptureSource::WireRequest, unknown_hint, CONN, 1).records(
            b"{\"ask\":\"something\"}",
            &[6],
            100,
        ),
    );
    unknown.extend(
        wire(
            WIRE,
            3,
            CaptureSource::WsClient,
            anthropic::PROTOCOL_HINT,
            CONN,
            1,
        )
        .records(b"{}", &[], 300),
    );
    unknown.push(connection(WIRE, CONN, connection_close()));

    let real = include_str!("../fixtures/m2-native/codex-native-e01-excerpt.jsonl");
    let mut codex = real.as_bytes().to_vec();
    codex.extend(jsonl(&[codex_user("And now the docs.", false)]));
    let mut native_real = epoch(
        CODEX,
        native(CODEX, 1, CODEX_HINT).records(&codex, &[], 100),
    );
    native_real.extend(epoch(
        CLAUDE,
        native(CLAUDE, 1, CLAUDE_HINT).records(
            &jsonl(&[
                claude_user("Build the parser.", Some("sdk")),
                claude_user("And now the docs.", None),
            ]),
            &[],
            100,
        ),
    ));

    let anthropic_error = {
        let start = message_start("msg_e");
        let sse = start.clone()
            + &ev(
                "error",
                json!({"type": "error", "error": {"type": "overloaded_error",
                    "message": "overloaded"}, "request_id": "req_e"}),
            );
        anthropic_records(&sse, &[start.len() + 10])
    };

    // A gap and a recovery: a wire message cut off when its producer is lost (a stream-less
    // gap, then a Recovery that supersedes the open stream), a recovered epoch that decodes
    // again, a keyed gap in a tracked Claude Code stream, an interrupted end, and an
    // explicit Assistance record. No `human_message` is produced: that would need a new
    // capture on the old code.
    let gap_recovery = {
        let lost = wire(
            WIRE,
            1,
            CaptureSource::WsClient,
            openai::PROTOCOL_HINT,
            CONN,
            1,
        );
        let request = ws(&response_create("gpt-5.6-luna"));
        let mut records = vec![connection(WIRE, CONN, connection_open(false)), lost.open()];
        records.extend(lost.segments(&request[..10], &[], 100));
        records.push(CaptureRecord::Gap {
            stream: None,
            observed: at(DAEMON, 400),
            reason: GapReason::QueueFull,
            lost_bytes: None,
        });
        let mut resumed = vec![
            CaptureRecord::Recovery {
                previous_producer: WIRE.to_string(),
                producer: RECOVERED.to_string(),
                observed: at(RECOVERED, 1),
                reason: GapReason::IngestLost,
            },
            connection(RECOVERED, CONN, connection_open(true)),
        ];
        resumed.extend(openai_messages(
            RECOVERED,
            1,
            &[
                (CaptureSource::WsClient, response_create("gpt-5.6-luna")),
                (CaptureSource::WsServer, response_created("resp_2")),
            ],
        ));
        resumed.push(connection(RECOVERED, CONN, connection_close()));
        records.extend(epoch(RECOVERED, resumed));

        let claude = native(CLAUDE, 1, CLAUDE_HINT);
        let lines = jsonl(&[
            claude_user("Build the parser.", Some("sdk")),
            claude_assistant("msg_g1", 5, 2),
        ]);
        let mut stream = vec![claude.open()];
        stream.extend(claude.segments(&lines, &[40], 500));
        stream.push(CaptureRecord::Gap {
            stream: Some(claude.key.clone()),
            observed: at(CLAUDE, 600),
            reason: GapReason::QueueFull,
            lost_bytes: None,
        });
        stream.extend(
            claude
                .segments(&jsonl(&[claude_assistant("msg_g2", 1, 1)]), &[], 700)
                .into_iter()
                .map(|record| match record {
                    CaptureRecord::Segment {
                        stream,
                        source_offset,
                        part,
                        source_len,
                        first,
                        last,
                        data_b64,
                        audit,
                    } => CaptureRecord::Segment {
                        stream,
                        part: part + 2,
                        source_offset: source_offset + lines.len() as u64,
                        source_len,
                        first,
                        last,
                        data_b64,
                        audit,
                    },
                    other => other,
                }),
        );
        stream.push(CaptureRecord::End {
            stream: claude.key.clone(),
            observed: at(CLAUDE, 900),
            reason: EndReason::Interrupted,
        });
        records.extend(epoch(CLAUDE, stream));
        records.extend(epoch(
            CODEX,
            native(CODEX, 1, CODEX_HINT).records(
                &jsonl(&[session_meta("exec"), codex_user("Build the parser.", true)]),
                &[],
                800,
            ),
        ));
        records.push(assistance(700, "operator_message"));
        records
    };

    let scenarios: Vec<(&str, Vec<CaptureRecord>, Option<TerminationOutcomeV1>)> = vec![
        ("gap-recovery", gap_recovery, Some(confirmed())),
        ("mixed", mixed_records(), Some(confirmed())),
        ("torn", codex_records(), None),
        ("unknown-protocol", epoch(WIRE, unknown), Some(confirmed())),
        ("native-real", native_real, Some(confirmed())),
        ("anthropic-error", anthropic_error, Some(confirmed())),
    ];
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2-report-bytes");
    for (name, records, finish) in scenarios {
        let trace = make_report_trace(GOAL_TEXT, &records, finish);
        let (_, bytes, _) = report_bytes(trace.path());
        let bytes = unversioned(&bytes);
        let path = root.join(format!("{name}.ndjson"));
        if regenerate_report_bytes(std::env::var_os("REGENERATE_REPORT_BYTES").as_deref()) {
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(&path, &bytes).unwrap();
        }
        let pinned = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(pinned == bytes, "{name}: the report bytes changed");
    }
}

/// A server holds records, not a folder: the record-stream entry composed from its parts
/// (`decode_records`, `JsonLinesSink`, `research_summary`) gives the folder entry's bytes.
#[test]
fn record_stream_entry_equals_folder_entry() {
    use goalcup_verifier::capture_analysis::registry::DecoderRegistry;
    use goalcup_verifier::capture_analysis::report::{JsonLinesSink, research_summary};
    use goalcup_verifier::capture_analysis::session::decode_records;

    let trace = make_report_trace(GOAL_TEXT, &mixed_records(), Some(confirmed()));
    let options = VerifyOptions::default();
    let (folder, folder_bytes, _) = report_with(trace.path(), &options);

    let mut records = Vec::new();
    let integrity = scan_capture_trace(trace.path(), &options, &mut |record| {
        records.push(record.clone());
        Ok(())
    })
    .unwrap();
    let registry = DecoderRegistry::builtin();
    let (outcome, sink) = decode_records(
        records.into_iter().map(Ok),
        &registry,
        JsonLinesSink::new(Vec::new()),
    )
    .unwrap();
    let (exit_code, summary) = research_summary(&integrity, &outcome, sink.tally(), &options);
    let mut bytes = sink.into_inner();
    bytes.extend(serde_json::to_vec(&summary).unwrap());
    bytes.push(b'\n');

    assert_eq!(exit_code, folder.exit_code);
    assert_eq!(summary, folder.summary);
    assert!(
        bytes == folder_bytes,
        "the two entries wrote different bytes"
    );
    // And the folder wrapper is the registry entry over the built-in decoders.
    let mut with = Vec::new();
    goalcup_verifier::write_research_report_with(trace.path(), &options, &registry, &mut with)
        .unwrap();
    assert!(with == folder_bytes);
}

/// A host's own sink, using only the public API, feeds `research_summary` through
/// `KindTally::observe` and gets the folder entry's summary.
#[test]
fn a_host_sink_feeds_the_summary_through_the_public_kind_tally() {
    use goalcup_verifier::capture_analysis::api::{Disposition, Observation, ObservationSink};
    use goalcup_verifier::capture_analysis::registry::DecoderRegistry;
    use goalcup_verifier::capture_analysis::report::{KindTally, research_summary};
    use goalcup_verifier::capture_analysis::session::decode_records;

    /// Stores observations, as a server would, and tallies the kept ones.
    #[derive(Default)]
    struct Store {
        kept: Vec<(&'static str, Observation)>,
        tally: KindTally,
    }
    impl ObservationSink for Store {
        fn observe(
            &mut self,
            id: &'static str,
            observation: Observation,
        ) -> std::io::Result<Disposition> {
            self.tally.observe(id, &observation);
            self.kept.push((id, observation));
            Ok(Disposition::Kept)
        }
    }

    let trace = make_report_trace(GOAL_TEXT, &mixed_records(), Some(confirmed()));
    let options = VerifyOptions::default();
    let (folder, _, _) = report_with(trace.path(), &options);
    let mut records = Vec::new();
    let integrity = scan_capture_trace(trace.path(), &options, &mut |record| {
        records.push(record.clone());
        Ok(())
    })
    .unwrap();
    let (outcome, store) = decode_records(
        records.into_iter().map(Ok),
        &DecoderRegistry::builtin(),
        Store::default(),
    )
    .unwrap();
    let (exit_code, summary) = research_summary(&integrity, &outcome, &store.tally, &options);
    assert!(!store.kept.is_empty());
    assert_eq!(exit_code, folder.exit_code);
    assert_eq!(summary, folder.summary);
    assert_eq!(summary["analysis"]["state"], "available");
}
