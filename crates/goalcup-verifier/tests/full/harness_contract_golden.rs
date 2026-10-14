//! Characterization goldens for the harness contract (D42, slice S0): the verifier half.
//!
//! * Decoder selection (`DecoderRegistry::builtin()`): which of the five registered
//!   decoders a stream is shown to, for every combination of stream source, `protocol_hint` and
//!   payload shape in a fixed probe list. Observed through `write_research_report`: the summary's
//!   `analysis.decoders` rows. The hint a decoder keys on is data in the stream's metadata
//!   (a connector's `capture.hosts[].protocol_hint`, or the daemon's native hint); the decoder
//!   is code.
//! * The reader's Claude-only tool allowance (`reader.rs`, `profile_id == "claude-code-cli"`):
//!   which `Launch` records the capture reader accepts, by profile id and by the allowance
//!   fields they carry.
//!
//! Compared byte for byte with `decoder-selection.json` and `launch-allowance-reader.json` under
//! `goalcup-cli/tests/fixtures/harness-golden/`; `GOALCUP_REGEN_HARNESS_GOLDEN=1` rewrites them
//! (off unless set to exactly `1`). Every trace is made by the real protocol writers in a
//! temporary directory, from synthetic bytes. Nothing here depends on the V1 verifier path.

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
use goalcup_protocol::provenance::{Rfc3339Utc, TerminationOutcomeV1};
use goalcup_protocol::redaction::RedactionAudit;
use goalcup_protocol::seal::SealManifest;
use goalcup_verifier::capture_analysis::reader::scan_capture_trace;
use goalcup_verifier::{VerifyOptions, write_research_report};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use support::records::key;

const ENTRY: &str = "entry-contract-golden";
const GOAL: &str = "9d8c7b6a5f4e3d2c1b0a998877665544";
const PROBE: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90";
const CONN: &str = "5e1f0a2b3c4d5e6f708192a3b4c5d6e7";
const DAEMON: &str = "daemon-golden-clock";
const DIGEST: &str = "ab";

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../goalcup-cli/tests/fixtures/harness-golden")
}

fn check_golden(name: &str, value: &Value) {
    let text = format!("{}\n", serde_json::to_string_pretty(value).unwrap());
    let path = golden_dir().join(name);
    if std::env::var("GOALCUP_REGEN_HARNESS_GOLDEN").is_ok_and(|v| v == "1") {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, &text).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{} is missing ({e}); generate it with GOALCUP_REGEN_HARNESS_GOLDEN=1",
            path.display()
        )
    });
    if text != want {
        let (n, (got, exp)) = text
            .lines()
            .zip(want.lines())
            .enumerate()
            .find(|(_, (a, b))| a != b)
            .unwrap_or((
                text.lines().count().min(want.lines().count()),
                ("<end>", "<end>"),
            ));
        panic!(
            "{name} differs from what the code now produces; first difference at line {}:\n  \
             golden: {exp}\n  actual: {got}\n(GOALCUP_REGEN_HARNESS_GOLDEN=1 rewrites it)",
            n + 1
        );
    }
}

// ---------------------------------------------------------------------------------------
// Trace building, with the real protocol writers.
// ---------------------------------------------------------------------------------------

fn at(clock: &str, tick: u64) -> CaptureTime {
    CaptureTime {
        wall: format!("2026-09-29T12:00:00.{tick:09}Z"),
        mono_ns: tick,
        clock_id: clock.to_string(),
    }
}

fn manifest() -> SealManifest {
    SealManifest {
        entry_id: ENTRY.to_string(),
        sealed_at: "2026-09-29T12:00:00Z".to_string(),
        workspace: "/ws".to_string(),
        files: vec![],
        excluded: vec![],
        git: None,
        home: None,
    }
}

/// One stream's records: open, one segment, end.
fn stream_records(
    producer: &str,
    id: u64,
    source: CaptureSource,
    meta: CaptureMeta,
    bytes: &[u8],
) -> Vec<CaptureRecord> {
    let stream = key(producer, id);
    vec![
        CaptureRecord::Open {
            stream: stream.clone(),
            observed: at(producer, 1),
            source,
            metadata: meta,
        },
        CaptureRecord::Segment {
            stream: stream.clone(),
            part: 0,
            source_offset: 0,
            source_len: bytes.len() as u64,
            first: at(producer, 10),
            last: at(producer, 15),
            data_b64: STANDARD.encode(bytes),
            audit: RedactionAudit { spans: vec![] },
        },
        CaptureRecord::End {
            stream,
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

fn launch(profile_id: &str, rule: Option<&str>, handoff_digest: Option<&str>) -> CaptureRecord {
    CaptureRecord::Launch {
        observed: at(DAEMON, 5),
        goal_stream: key(GOAL, 1),
        profile_id: profile_id.to_string(),
        profile_digest: "cc".repeat(32),
        harness_version: "0.0.0".to_string(),
        recorder_revision: "contract-golden".to_string(),
        continuation: "none".to_string(),
        allowed_tool_rule: rule.map(str::to_string),
        process_handoff_digest: handoff_digest.map(str::to_string),
        run_public_key: None,
        connector_dir_sha256: None,
        harness_build: None,
    }
}

fn goal_meta() -> CaptureMeta {
    CaptureMeta {
        wire: None,
        label: "goal".to_string(),
        protocol_hint: None,
        method: None,
        host: None,
        path: None,
        status: None,
        content_type: None,
    }
}

/// A sealed local v5 trace: the goal stream, a Launch, then `records`.
fn trace(launch: CaptureRecord, records: Vec<CaptureRecord>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(dir.path()).unwrap();
    export.configure_capture_quota(1 << 40).unwrap();
    export
        .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest())
        .unwrap();
    let mut all = epoch(
        GOAL,
        stream_records(GOAL, 1, CaptureSource::Goal, goal_meta(), b"the goal"),
    );
    all.push(launch);
    all.extend(records);
    for record in all {
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
            body: EventBody::Capture { record },
            redaction: None,
        };
        export.append_chain(&mut env).unwrap();
    }
    let checked_at = Rfc3339Utc::try_from("2026-09-29T12:00:05Z").unwrap();
    export
        .write_local_finish(
            &manifest(),
            at(DAEMON, 990_000),
            "participant_stop",
            TerminationOutcomeV1::AlreadyStopped { checked_at },
        )
        .unwrap();
    dir
}

// ---------------------------------------------------------------------------------------
// Decoder selection.
// ---------------------------------------------------------------------------------------

/// Payloads, one per decoder's native shape.
fn payload(kind: &str) -> Vec<u8> {
    let lines: Vec<Value> = match kind {
        "codex_rollout_line" => vec![
            json!({"timestamp": "2026-09-29T11:59:59.000Z", "type": "session_meta",
                "payload": {"id": "session-1", "source": "cli"}}),
            json!({"timestamp": "2026-09-29T12:00:00.100Z", "type": "turn_context",
                "payload": {"turn_id": "turn-1", "model": "gpt-5.6-luna"}}),
        ],
        "claude_transcript_line" => vec![json!({"type": "assistant",
            "timestamp": "2026-09-29T12:00:00.500Z", "message": {
            "id": "msg-1", "model": "claude-test-model",
            "usage": {"input_tokens": 3, "output_tokens": 4},
            "content": [{"type": "text", "text": "done"}]}})],
        "openai_ws_message" => {
            return serde_json::to_vec(&json!({"type": "response.create",
                "model": "gpt-5.6-luna", "stream": true, "store": false,
                "input": [{"type": "message", "role": "user",
                    "content": [{"type": "input_text", "text": "context"}]}]}))
            .unwrap();
        }
        "anthropic_request_body" => {
            return json!({"model": "claude-test-model", "max_tokens": 16, "stream": true,
                "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}]})
            .to_string()
            .into_bytes();
        }
        other => panic!("no payload {other}"),
    };
    lines
        .iter()
        .map(|l| format!("{l}\n"))
        .collect::<String>()
        .into_bytes()
}

fn probe_records(source: CaptureSource, hint: Option<&str>, payload: &[u8]) -> Vec<CaptureRecord> {
    let ws = matches!(source, CaptureSource::WsClient | CaptureSource::WsServer);
    let wire = matches!(
        source,
        CaptureSource::WireRequest
            | CaptureSource::WireResponse
            | CaptureSource::WsClient
            | CaptureSource::WsServer
    );
    let meta = CaptureMeta {
        wire: wire.then(|| WireIdentity {
            connection: CONN.to_string(),
            exchange: 1,
            message_index: ws.then_some(1),
        }),
        label: if wire {
            "probe #1".to_string()
        } else {
            "sessions/probe#1".to_string()
        },
        protocol_hint: hint.map(str::to_string),
        method: (source == CaptureSource::WireRequest).then(|| "POST".to_string()),
        host: wire.then(|| "api.example.test".to_string()),
        path: wire.then(|| "/v1/probe".to_string()),
        status: (source == CaptureSource::WireResponse).then_some(200),
        content_type: match source {
            CaptureSource::WireRequest => Some("application/json".to_string()),
            CaptureSource::WireResponse => Some("text/event-stream".to_string()),
            _ => None,
        },
    };
    let mut records = Vec::new();
    if wire {
        records.push(CaptureRecord::Connection {
            producer: PROBE.to_string(),
            connection: CONN.to_string(),
            observed: at(PROBE, 0),
            event: ConnectionEvent::Open {
                alpn: Some("http/1.1".to_string()),
                already_open: false,
            },
        });
    }
    records.extend(stream_records(PROBE, 1, source, meta, payload));
    if wire {
        records.push(CaptureRecord::Connection {
            producer: PROBE.to_string(),
            connection: CONN.to_string(),
            observed: at(PROBE, 2),
            event: ConnectionEvent::Close {
                outcome: ConnectionOutcome::Complete,
            },
        });
    }
    epoch(PROBE, records)
}

/// Which decoders saw the stream, from the summary's decoder rows.
fn decoder_rows(dir: &Path) -> Value {
    let mut out = Vec::new();
    let result = write_research_report(dir, &VerifyOptions::default(), &mut out)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let rows = result.summary["analysis"]["decoders"].as_array().unwrap();
    json!(
        rows.iter()
            .map(|row| json!({
                "decoder": row["decoder"],
                "decoder_version": row["summary"]["decoder_version"],
                "written": row["written"],
                "observations": row["summary"]["observations"],
                "unknown_records": row["summary"]["unknown_records"],
                "failure": row["failure"],
            }))
            .collect::<Vec<_>>()
    )
}

#[test]
fn verifier_decoder_selection_per_protocol_hint_is_pinned() {
    let payloads = [
        "codex_rollout_line",
        "claude_transcript_line",
        "openai_ws_message",
        "anthropic_request_body",
    ];
    let hints: [Option<&str>; 7] = [
        Some("codex-jsonl"),
        Some("claude-code-jsonl"),
        Some("openai-responses"),
        Some("anthropic-messages"),
        Some("openai-chat-completions"),
        Some("e00-controlled"),
        None,
    ];
    let sources = [
        ("native", CaptureSource::Native),
        ("ws_client", CaptureSource::WsClient),
        ("wire_request", CaptureSource::WireRequest),
    ];
    let mut rows = Vec::new();
    for (source_name, source) in sources {
        for hint in hints {
            for kind in payloads {
                let records = probe_records(source, hint, &payload(kind));
                let dir = trace(launch("codex-cli", None, None), records);
                rows.push(json!({
                    "source": source_name,
                    "protocol_hint": hint,
                    "payload": kind,
                    "decoders": decoder_rows(dir.path()),
                }));
            }
        }
    }
    // Which decoder ids a report registers at all, with no stream to show them.
    let empty = trace(launch("codex-cli", None, None), vec![]);
    check_golden(
        "decoder-selection.json",
        &json!({
            "registered_decoders_in_order": decoder_rows(empty.path())
                .as_array().unwrap().iter().map(|r| r["decoder"].clone()).collect::<Vec<_>>(),
            "probes": rows,
        }),
    );
}

// ---------------------------------------------------------------------------------------
// The Claude-only tool allowance in the reader.
// ---------------------------------------------------------------------------------------

#[test]
fn the_reader_admits_a_tool_allowance_only_for_the_claude_profile() {
    let digest = DIGEST.repeat(32);
    let rules: [(&str, Option<&str>, Option<&str>); 7] = [
        ("none", None, None),
        (
            "rule_and_digest",
            Some("Bash(python3 demo.py *)"),
            Some(&digest),
        ),
        ("rule_without_digest", Some("Bash(python3 demo.py *)"), None),
        ("digest_without_rule", None, Some(&digest)),
        ("unbounded_rule_and_digest", Some("Bash(*)"), Some(&digest)),
        (
            "rule_and_short_digest",
            Some("Bash(python3 demo.py *)"),
            Some("abcd"),
        ),
        (
            "path_rule_and_digest",
            Some("Bash(python3 ../x.py *)"),
            Some(&digest),
        ),
    ];
    let profiles = [
        "claude-code-cli",
        "codex-cli",
        "hermes-cli",
        "m2-controlled-workload",
        "claude-code-clone",
        "",
    ];
    let mut rows = Vec::new();
    for profile in profiles {
        for (label, rule, handoff) in rules {
            let dir = trace(launch(profile, rule, handoff), vec![]);
            let report = scan_capture_trace(dir.path(), &VerifyOptions::default(), &mut |_| Ok(()));
            let verdict = match report {
                Ok(r) => json!({
                    "exit_code": r.exit_code(),
                    "integrity": r.integrity,
                    "findings": r.findings,
                }),
                Err(e) => json!({"io_error": e.kind().to_string()}),
            };
            rows.push(json!({"profile_id": profile, "fields": label, "reader": verdict}));
        }
    }
    check_golden(
        "launch-allowance-reader.json",
        &json!({
            "rule": "the reader admits Launch.allowed_tool_rule and process_handoff_digest together, once, for profile_id claude-code-cli only",
            "rows": rows,
        }),
    );
}
