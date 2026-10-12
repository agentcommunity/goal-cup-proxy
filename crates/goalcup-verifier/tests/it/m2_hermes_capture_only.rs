//! Hermes lane A, S3: the Hermes reference row, end to end through the research report. A
//! DeepSeek Chat Completions stream (`openai-chat-completions`) and an unhinted native session
//! snapshot are retained and integrity-valid. The Chat Completions decoder now reads the
//! stream; the unhinted snapshot is still read by no decoder. Selection stays by exact hint and
//! never by host, so the `openai-responses` decoder reads none of it, and the chat decoder
//! reads nothing when the same bytes carry another hint. (Before the decoder existed this file
//! pinned the whole row as capture-only, analysis `unavailable`; the file name is kept.)
//!
//! The trace is written by the real protocol writers (as in `m2_research_report.rs`), and
//! every byte of it is synthetic.

use crate::common as support;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use goalcup_protocol::capture::{
    CaptureMeta, CaptureRecord, CaptureSource, CaptureTime, ConnectionEvent, ConnectionOutcome,
    EndReason, M2_PROTOCOL_VERSION, WireIdentity,
};
use goalcup_protocol::chain::Link;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::export::TraceExport;
use goalcup_protocol::provenance::{Rfc3339Utc, TerminationMethodV1, TerminationOutcomeV1};
use goalcup_protocol::redaction::RedactionAudit;
use goalcup_protocol::seal::SealManifest;
use goalcup_verifier::capture_analysis::{chat, openai};
use goalcup_verifier::{VerifyOptions, write_research_report};
use serde_json::{Value, json};
use std::path::Path;
use support::records::key;

const ENTRY: &str = "entry-m2-hermes-s3";
const GOAL: &str = "9d8c7b6a5f4e3d2c1b0a998877665544";
const NATIVE: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90";
const WIRE: &str = "0123456789abcdef0123456789abcdef";
const CONN: &str = "5e1f0a2b3c4d5e6f708192a3b4c5d6e7";
const DAEMON: &str = "daemon-s3-clock";
const HINT: &str = "openai-chat-completions";
const SESSION: &str = "20260930_141500_a1b2c3";
const GOAL_TEXT: &[u8] = b"write result.txt";

fn at(clock: &str, tick: u64) -> CaptureTime {
    CaptureTime {
        wall: format!("2026-10-02T12:00:00.{tick:09}Z"),
        mono_ns: tick,
        clock_id: clock.to_string(),
    }
}

fn utc(value: &str) -> Rfc3339Utc {
    Rfc3339Utc::try_from(value).unwrap()
}

fn meta(label: &str, hint: Option<&str>) -> CaptureMeta {
    CaptureMeta {
        wire: None,
        label: label.to_string(),
        protocol_hint: hint.map(str::to_string),
        method: None,
        host: None,
        path: None,
        status: None,
        content_type: None,
    }
}

/// One stream: Open, one Segment holding `bytes`, End(Complete).
fn stream_records(
    producer: &str,
    id: u64,
    source: CaptureSource,
    metadata: CaptureMeta,
    bytes: &[u8],
) -> Vec<CaptureRecord> {
    vec![
        CaptureRecord::Open {
            stream: key(producer, id),
            observed: at(producer, 1),
            source,
            metadata,
        },
        CaptureRecord::Segment {
            stream: key(producer, id),
            part: 0,
            source_offset: 0,
            source_len: bytes.len() as u64,
            first: at(producer, 10),
            last: at(producer, 15),
            data_b64: STANDARD.encode(bytes),
            audit: RedactionAudit { spans: vec![] },
        },
        CaptureRecord::End {
            stream: key(producer, id),
            observed: at(producer, 900_000),
            reason: EndReason::Complete,
        },
    ]
}

fn epoch(producer: &str, mut records: Vec<CaptureRecord>) -> Vec<CaptureRecord> {
    let final_seq = records.len() as u64 - 1;
    records.push(CaptureRecord::ProducerClosed {
        producer: producer.to_string(),
        observed: at(DAEMON, 950_000),
        final_seq: Some(final_seq),
    });
    records
}

fn wire_meta(source: CaptureSource, hint: &str) -> CaptureMeta {
    CaptureMeta {
        wire: Some(WireIdentity {
            connection: CONN.to_string(),
            exchange: 1,
            message_index: None,
        }),
        label: format!("{hint} {source:?}"),
        protocol_hint: Some(hint.to_string()),
        method: (source == CaptureSource::WireRequest).then(|| "POST".to_string()),
        host: Some("api.deepseek.com".to_string()),
        path: Some("/v1/chat/completions".to_string()),
        status: (source == CaptureSource::WireResponse).then_some(200),
        content_type: Some(
            if source == CaptureSource::WireRequest {
                "application/json"
            } else {
                "text/event-stream"
            }
            .to_string(),
        ),
    }
}

fn manifest() -> SealManifest {
    SealManifest {
        entry_id: ENTRY.to_string(),
        sealed_at: "2026-10-02T12:00:00Z".to_string(),
        workspace: "/ws".to_string(),
        files: vec![],
        excluded: vec![],
    }
}

fn append(export: &mut TraceExport, record: &CaptureRecord) -> Link {
    let mut env = Envelope {
        protocol_version: M2_PROTOCOL_VERSION,
        entry_id: ENTRY.to_string(),
        source: "daemon".to_string(),
        recorded_at: "2026-10-02T12:00:01Z".to_string(),
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

fn trace(records: &[CaptureRecord]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(dir.path()).unwrap();
    export.configure_capture_quota(1 << 40).unwrap();
    export
        .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest())
        .unwrap();
    let goal = epoch(
        GOAL,
        stream_records(GOAL, 1, CaptureSource::Goal, meta("goal", None), GOAL_TEXT),
    );
    for record in goal {
        append(&mut export, &record);
    }
    append(
        &mut export,
        &CaptureRecord::Launch {
            observed: at(DAEMON, 5),
            goal_stream: key(GOAL, 1),
            profile_id: "hermes-cli".to_string(),
            profile_digest: "cc".repeat(32),
            harness_version: "0.19.0".to_string(),
            recorder_revision: "s3-fixture".to_string(),
            continuation: "none".to_string(),
            allowed_tool_rule: None,
            process_handoff_digest: None,
            run_public_key: None,
            connector_dir_sha256: None,
        },
    );
    for record in records {
        append(&mut export, record);
    }
    export
        .write_local_finish(
            &manifest(),
            CaptureTime {
                wall: "2026-10-02T12:00:05.500Z".to_string(),
                mono_ns: 990_000,
                clock_id: DAEMON.to_string(),
            },
            "participant_stop",
            TerminationOutcomeV1::Confirmed {
                requested_at: utc("2026-10-02T12:00:02Z"),
                confirmed_at: utc("2026-10-02T12:00:03.125Z"),
                method: TerminationMethodV1::ProcessSignal,
            },
        )
        .unwrap();
    dir
}

fn report(dir: &Path) -> (u8, Vec<Value>) {
    let mut out = Vec::new();
    let result = write_research_report(dir, &VerifyOptions::default(), &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    let lines = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    (result.exit_code, lines)
}

/// The synthetic DeepSeek exchange and the Hermes session snapshot, with the wire streams
/// carrying `hint`.
fn deepseek_run(hint: &str) -> Vec<CaptureRecord> {
    let request = json!({
        "model": "deepseek-v4-flash",
        "stream": true,
        "messages": [{"role": "user", "content": "write result.txt"}],
    })
    .to_string();
    let response = format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"model": "deepseek-v4-flash-0930", "choices": [{"delta": {"content": "ok"}}]}),
        json!({"choices": [], "usage": {"prompt_tokens": 9, "completion_tokens": 2}}),
    );
    let snapshot = json!({
        "session_id": SESSION,
        "model": "deepseek-v4-flash",
        "messages": [{"role": "user", "content": "write result.txt"}],
    })
    .to_string();

    let mut wire = vec![CaptureRecord::Connection {
        producer: WIRE.to_string(),
        connection: CONN.to_string(),
        observed: at(WIRE, 2),
        event: ConnectionEvent::Open {
            alpn: Some("http/1.1".to_string()),
            already_open: false,
        },
    }];
    wire.extend(stream_records(
        WIRE,
        1,
        CaptureSource::WireRequest,
        wire_meta(CaptureSource::WireRequest, hint),
        request.as_bytes(),
    ));
    wire.extend(stream_records(
        WIRE,
        2,
        CaptureSource::WireResponse,
        wire_meta(CaptureSource::WireResponse, hint),
        response.as_bytes(),
    ));
    wire.push(CaptureRecord::Connection {
        producer: WIRE.to_string(),
        connection: CONN.to_string(),
        observed: at(WIRE, 3),
        event: ConnectionEvent::Close {
            outcome: ConnectionOutcome::Complete,
        },
    });
    let mut records = epoch(WIRE, wire);
    records.extend(epoch(
        NATIVE,
        stream_records(
            NATIVE,
            1,
            CaptureSource::Native,
            meta(&format!("session:session_{SESSION}.json#g1"), None),
            snapshot.as_bytes(),
        ),
    ));
    records
}

fn decoder_entry<'a>(summary: &'a Value, id: &str) -> &'a Value {
    summary["analysis"]["decoders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["decoder"] == id)
        .unwrap_or_else(|| panic!("{id} is registered and ran"))
}

/// T-S3-1.
#[test]
fn a_deepseek_chat_completions_stream_is_decoded_and_an_unhinted_snapshot_stays_unread() {
    let dir = trace(&deepseek_run(HINT));
    let (exit_code, lines) = report(dir.path());
    let summary = lines.last().unwrap();

    assert_eq!(summary["type"], "summary");
    assert_eq!(exit_code, 0, "{summary:#}");
    assert_eq!(summary["capture"]["integrity"], "valid");
    assert_eq!(summary["analysis"]["state"], "available", "{summary:#}");

    // Every observation line is the chat decoder's, in the stream's order.
    let body = &lines[..lines.len() - 1];
    assert!(
        body.iter().all(|l| l["decoder"] == chat::DECODER_ID),
        "{body:#?}"
    );
    let kinds: Vec<&str> = body
        .iter()
        .map(|l| l["observation"]["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        ["model_request", "model", "model", "usage", "model_response"],
        "{body:#?}"
    );
    let fields = |kind: &str| -> Vec<&Value> {
        body.iter()
            .map(|l| &l["observation"])
            .filter(|o| o["kind"] == kind)
            .map(|o| &o["fields"])
            .collect()
    };
    // The oracle is the bytes this file wrote above, not the decoder.
    assert_eq!(fields("usage")[0]["prompt_tokens"], 9);
    assert_eq!(fields("usage")[0]["completion_tokens"], 2);
    assert_eq!(fields("model_response")[0]["content"], "ok");
    assert_eq!(fields("model_response")[0]["stream_complete"], true);
    assert_eq!(fields("model")[1]["model"], "deepseek-v4-flash-0930");
    assert_eq!(fields("model")[1]["side"], "response");

    let chat = decoder_entry(summary, chat::DECODER_ID);
    assert_eq!(chat["written"], 5, "{chat:#}");
    assert_eq!(chat["summary"]["unknown_records"], 0, "{chat:#}");
    assert_eq!(chat["summary"]["incomplete"], false, "{chat:#}");
    // Not one observation of the Responses decoder: selection is the hint.
    assert_ne!(HINT, openai::PROTOCOL_HINT);
    let responses = decoder_entry(summary, openai::DECODER_ID);
    assert_eq!(responses["written"], 0, "{responses:#}");
    assert_eq!(responses["summary"]["unknown_records"], 0, "{responses:#}");

    // The unhinted native snapshot is retained and read by no decoder.
    let counts = &summary["capture"]["record_counts"];
    assert_eq!(counts["open.wire_request"], 1, "{counts:#}");
    assert_eq!(counts["open.wire_response"], 1, "{counts:#}");
    assert_eq!(counts["open.native"], 1, "{counts:#}");
}

/// The control: the same bytes under the exact `openai-responses` hint ARE read by that
/// decoder (as unreadable-for-its-format records), and the chat decoder reads none of them. So
/// the split is the hint, not the trace, the host or the path.
#[test]
fn the_same_bytes_under_the_responses_hint_are_selected_by_that_decoder() {
    let dir = trace(&deepseek_run(openai::PROTOCOL_HINT));
    let (_, lines) = report(dir.path());
    let summary = lines.last().unwrap();
    assert!(
        lines.len() > 1,
        "the OpenAI decoder saw the streams: {summary:#}"
    );
    assert!(
        lines[..lines.len() - 1]
            .iter()
            .all(|l| l["decoder"] == openai::DECODER_ID),
        "{lines:#?}"
    );
    let chat = decoder_entry(summary, chat::DECODER_ID);
    assert_eq!(chat["written"], 0, "{chat:#}");
    assert_eq!(chat["summary"]["unknown_records"], 0, "{chat:#}");
}

/// The control the other way: a trace with an unknown wire hint is read by no decoder.
#[test]
fn the_same_bytes_under_an_unknown_hint_are_read_by_no_decoder() {
    let dir = trace(&deepseek_run("some-new-protocol"));
    let (exit_code, lines) = report(dir.path());
    assert_eq!(exit_code, 0);
    assert_eq!(lines.len(), 1, "only the summary line: {lines:#?}");
    assert_eq!(lines[0]["analysis"]["state"], "unavailable");
}
