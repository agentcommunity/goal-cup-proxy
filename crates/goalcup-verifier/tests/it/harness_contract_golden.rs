//! Characterization goldens for the harness contract (D42): the verifier half. Neither file is
//! keyed by a connector, so adding one changes neither.
//!
//! * Decoder selection (`DecoderRegistry::builtin()`): which of the registered
//!   decoders a stream is shown to, for every combination of stream source, `protocol_hint` and
//!   payload shape in a fixed probe list. Observed through `write_research_report`: the summary's
//!   `analysis.decoders` rows. The golden lists only the rows that are not zero; the test
//!   asserts that every other pair is zero. The hint a decoder keys on is data in the stream's metadata
//!   (a connector's `capture.hosts[].protocol_hint`, or the daemon's native hint); the decoder
//!   is code.
//! * The reader's tool allowance (`reader.rs`, shape only since the D52 amendment): which
//!   `Launch` records the capture reader accepts, by the allowance fields they carry. The
//!   golden uses synthetic profile ids; every shipped connector's id is checked to get the same
//!   verdicts.
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
/// A profile id no connector has: the reader keys on no profile id.
const ANY_PROFILE: &str = "any-profile";

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
        capped: None,
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

/// Every registered decoder's row for the trace, from the summary's decoder rows.
fn decoder_rows(dir: &Path) -> Vec<Value> {
    let mut out = Vec::new();
    let result = write_research_report(dir, &VerifyOptions::default(), &mut out)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let rows = result.summary["analysis"]["decoders"].as_array().unwrap();
    rows.iter()
        .map(|row| {
            json!({
                "decoder": row["decoder"],
                "decoder_version": row["summary"]["decoder_version"],
                "written": row["written"],
                "observations": row["summary"]["observations"],
                "unknown_records": row["summary"]["unknown_records"],
                "failure": row["failure"],
            })
        })
        .collect()
}

/// A decoder that wrote nothing, observed nothing and did not fail.
fn is_zero(row: &Value) -> bool {
    row["written"] == 0
        && row["observations"] == 0
        && row["unknown_records"] == 0
        && row["failure"].is_null()
}

fn decoder_id(row: &Value) -> Value {
    json!({"decoder": row["decoder"], "decoder_version": row["decoder_version"]})
}

#[test]
fn verifier_decoder_selection_per_protocol_hint_is_pinned() {
    let payloads = [
        "codex_rollout_line",
        "claude_transcript_line",
        "openai_ws_message",
        "anthropic_request_body",
    ];
    let hints: [Option<&str>; 8] = [
        Some("codex-jsonl"),
        Some("claude-code-jsonl"),
        Some("openai-responses"),
        Some("anthropic-messages"),
        Some("openai-chat-completions"),
        Some("amp-thread-jsonrpc"),
        Some("e00-controlled"),
        None,
    ];
    let sources = [
        ("native", CaptureSource::Native),
        ("ws_client", CaptureSource::WsClient),
        ("wire_request", CaptureSource::WireRequest),
    ];
    // Which decoders a report registers at all, with no stream to show them.
    let empty = decoder_rows(trace(launch(ANY_PROFILE, None, None), vec![]).path());
    assert!(empty.iter().all(is_zero), "an empty trace: {empty:?}");
    let registered: Vec<Value> = empty.iter().map(decoder_id).collect();
    // Only the probe and decoder pairs that did something are pinned. Every other pair is
    // checked to be all zero here, so a decoder that starts seeing a new stream still changes
    // the golden.
    let mut active = Vec::new();
    for (source_name, source) in sources {
        for hint in hints {
            for kind in payloads {
                let records = probe_records(source, hint, &payload(kind));
                let rows = decoder_rows(trace(launch(ANY_PROFILE, None, None), records).path());
                assert_eq!(
                    rows.iter().map(decoder_id).collect::<Vec<_>>(),
                    registered,
                    "{source_name} {hint:?} {kind}: every registered decoder reports a row"
                );
                for row in rows.iter().filter(|r| !is_zero(r)) {
                    active.push(json!({
                        "source": source_name,
                        "protocol_hint": hint,
                        "payload": kind,
                        "decoder": row["decoder"],
                        "written": row["written"],
                        "observations": row["observations"],
                        "unknown_records": row["unknown_records"],
                        "failure": row["failure"],
                    }));
                }
            }
        }
    }
    check_golden(
        "decoder-selection.json",
        &json!({
            "registered_decoders_in_order": registered,
            "rule": "every combination of source, protocol_hint, payload and decoder not listed in active is zero: written, observations and unknown_records are 0 and failure is null",
            "active": active,
        }),
    );
}

// ---------------------------------------------------------------------------------------
// The tool allowance in the reader: shape only, for every profile.
// ---------------------------------------------------------------------------------------

#[test]
fn the_reader_admits_a_paired_tool_allowance_by_shape_for_every_profile() {
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
    let reader = |profile: &str, rule: Option<&str>, handoff: Option<&str>| -> Value {
        let dir = trace(launch(profile, rule, handoff), vec![]);
        match scan_capture_trace(dir.path(), &VerifyOptions::default(), &mut |_| Ok(())) {
            Ok(r) => json!({
                "exit_code": r.exit_code(),
                "integrity": r.integrity,
                "findings": r.findings,
            }),
            Err(e) => json!({"io_error": e.kind().to_string()}),
        }
    };
    let mut rows = Vec::new();
    for profile in [ANY_PROFILE, ""] {
        for (label, rule, handoff) in rules {
            rows.push(json!({
                "profile_id": profile,
                "fields": label,
                "reader": reader(profile, rule, handoff),
            }));
        }
    }
    for id in goalcup_protocol::pins::shipped_connector_ids() {
        for (label, rule, handoff) in rules {
            assert_eq!(
                reader(&id, rule, handoff),
                reader(ANY_PROFILE, rule, handoff),
                "{id} ({label}): the reader keys on no profile id"
            );
        }
    }
    check_golden(
        "launch-allowance-reader.json",
        &json!({
            "rule": "the reader admits Launch.allowed_tool_rule and process_handoff_digest together, by shape only (bounded text and SHA-256 hex), for every profile_id",
            "rows": rows,
        }),
    );
}
