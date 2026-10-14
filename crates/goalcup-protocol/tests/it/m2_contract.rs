//! The shared capture contracts as frozen wire bytes.
//!
//! The JSON layouts below are the goldens every M2 leaf builds against: a change to one is a
//! change to a shared interface and needs an amendment to the shared contract, never a quiet edit.

use goalcup_protocol::capture::*;
use goalcup_protocol::chain::Link;
use goalcup_protocol::control::StatusPayload;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::export::{ChainPhaseReplay, ExportError, TraceExport};
use goalcup_protocol::provenance::{Rfc3339Utc, TerminationMethodV1, TerminationOutcomeV1};
use goalcup_protocol::redaction::{RedactionAudit, RedactionAuditSpan};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fmt::Debug;
use std::path::PathBuf;

const PRODUCER: &str = "0123456789abcdef0123456789abcdef";
const NEXT_PRODUCER: &str = "fedcba9876543210fedcba9876543210";
const CONNECTION: &str = "00112233445566778899aabbccddeeff";
const DAEMON_CLOCK: &str = "daemon-5b0c3a1e";

#[test]
fn optional_claude_launch_provenance_preserves_old_bytes_and_round_trips() {
    let old = CaptureRecord::Launch {
        observed: at(DAEMON_CLOCK, 9),
        goal_stream: key(1),
        profile_id: "claude-code-cli".into(),
        profile_digest: "c".repeat(64),
        harness_version: "2.0".into(),
        recorder_revision: "test".into(),
        continuation: "none".into(),
        allowed_tool_rule: None,
        process_handoff_digest: None,
        run_public_key: None,
        connector_dir_sha256: None,
        harness_build: None,
    };
    let old_json = serde_json::to_string(&old).unwrap();
    assert!(!old_json.contains("allowed_tool_rule"));
    assert!(!old_json.contains("process_handoff_digest"));
    assert_eq!(
        serde_json::from_str::<CaptureRecord>(&old_json).unwrap(),
        old
    );
    let mut new = old.clone();
    if let CaptureRecord::Launch {
        allowed_tool_rule,
        process_handoff_digest,
        ..
    } = &mut new
    {
        *allowed_tool_rule = Some("Bash(python3 demo.py *)".into());
        *process_handoff_digest = Some("d".repeat(64));
    }
    let new_json = serde_json::to_string(&new).unwrap();
    assert!(new_json.contains("Bash(python3 demo.py *)"));
    assert_eq!(
        serde_json::from_str::<CaptureRecord>(&new_json).unwrap(),
        new
    );
    assert!(is_claude_capture_allowed_tool_rule(
        "Bash(python3 demo.py *)"
    ));
    assert!(!is_claude_capture_allowed_tool_rule("Bash(*)"));
    assert!(!is_claude_capture_allowed_tool_rule(
        "Bash(python3 -m.py *)"
    ));
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn at(clock: &str, mono_ns: u64) -> CaptureTime {
    CaptureTime {
        wall: "2026-09-27T10:00:00.250Z".to_string(),
        mono_ns,
        clock_id: clock.to_string(),
    }
}

fn key(stream: u64) -> StreamKey {
    StreamKey {
        producer: PRODUCER.to_string(),
        stream,
    }
}

fn http_meta() -> CaptureMeta {
    CaptureMeta {
        wire: Some(WireIdentity {
            connection: CONNECTION.to_string(),
            exchange: 1,
            message_index: None,
        }),
        label: "POST /v1/responses".to_string(),
        protocol_hint: Some("openai-responses".to_string()),
        method: Some("POST".to_string()),
        host: Some("api.example.test".to_string()),
        path: Some("/v1/responses".to_string()),
        status: None,
        content_type: Some("application/json".to_string()),
    }
}

fn gap_record() -> CaptureRecord {
    CaptureRecord::Gap {
        stream: None,
        observed: at(DAEMON_CLOCK, 9_000),
        reason: GapReason::ProducerLost,
        lost_bytes: None,
    }
}

fn envelope(protocol_version: u32, body: EventBody) -> Envelope {
    Envelope {
        protocol_version,
        entry_id: "entry-m2-0001".to_string(),
        source: "daemon".to_string(),
        recorded_at: "2026-09-27T10:00:01Z".to_string(),
        link: Link {
            seq: 7,
            prev: "a".repeat(64),
            hash: String::new(),
        },
        body,
        redaction: None,
    }
}

/// Serialize to exactly `golden`, and deserialize `golden` back to the same value.
fn layout<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: &T, golden: &str) {
    let json = serde_json::to_string(value).expect("serializes");
    assert_eq!(json, golden, "layout of {value:?}");
    let back: T = serde_json::from_str(golden).expect("golden deserializes");
    assert_eq!(&back, value);
}

#[test]
fn capture_roundtrips_in_v5_and_every_other_version_is_refused() {
    assert_eq!(M2_PROTOCOL_VERSION, 5);
    let replay = ChainPhaseReplay::new();
    for retired in 1..=4 {
        assert!(
            matches!(
                replay.check_version(7, retired),
                Err(ExportError::InvalidCaptureEnvelope { seq: 7, .. })
            ),
            "protocol v{retired} is a retired format, not a v5 record"
        );
    }
    for unknown in [0, 6, u32::MAX] {
        assert!(
            matches!(
                replay.check_version(7, unknown),
                Err(ExportError::UnsupportedProtocolVersion { seq: 7, protocol_version })
                    if protocol_version == unknown
            ),
            "protocol v{unknown} is an unknown version"
        );
    }
    replay.check_version(7, M2_PROTOCOL_VERSION).unwrap();

    let body = EventBody::Capture {
        record: gap_record(),
    };
    let mut v5 = envelope(M2_PROTOCOL_VERSION, body);
    v5.link.hash = v5.expected_hash(&v5.link.prev.clone()).unwrap();
    let bytes = serde_json::to_vec(&v5).unwrap();
    let back: Envelope = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(back, v5);
    assert_eq!(serde_json::to_vec(&back).unwrap(), bytes);
    assert_eq!(back.expected_hash(&back.link.prev).unwrap(), v5.link.hash);
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        format!(
            concat!(
                r#"{{"protocol_version":5,"entry_id":"entry-m2-0001","source":"daemon","#,
                r#""recorded_at":"2026-09-27T10:00:01Z","seq":7,"prev":"{}","hash":"{}","#,
                r#""body":{{"kind":"capture","content":{{"record":{{"kind":"gap","stream":null,"#,
                r#""observed":{{"wall":"2026-09-27T10:00:00.250Z","mono_ns":9000,"#,
                r#""clock_id":"daemon-5b0c3a1e"}},"reason":"producer_lost","lost_bytes":null}}}}}}}}"#
            ),
            "a".repeat(64),
            v5.link.hash
        )
    );
}

/// The v5 local-finish record is the export's reserved terminal record: an ordinary append
/// refuses it as reserved before touching the chain.
#[test]
fn ordinary_append_refuses_local_finish() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    let mut env = envelope(
        M2_PROTOCOL_VERSION,
        EventBody::Capture {
            record: CaptureRecord::LocalFinish {
                observed: at(DAEMON_CLOCK, 1),
                reason: "manual".to_string(),
                termination: TerminationOutcomeV1::AlreadyStopped {
                    checked_at: Rfc3339Utc::try_from("2026-09-27T12:00:00Z").unwrap(),
                },
                seal_manifest_sha256: "b".repeat(64),
            },
        },
    );
    env.link = Link {
        seq: 0,
        prev: String::new(),
        hash: String::new(),
    };
    let refused = export.append_chain(&mut env);
    assert!(
        matches!(
            refused,
            Err(ExportError::ReservedBody {
                kind: "local_finish"
            })
        ),
        "{refused:?}"
    );
    assert_eq!(export.head(), None, "nothing was appended");
    let chain = std::fs::read(tmp.path().join("chain.jsonl")).unwrap();
    assert!(chain.is_empty(), "no byte reached the chain");
}

#[test]
fn legacy_status_serialization_unchanged() {
    // Frozen from the unmodified protocol crate: `control_shapes.rs`'s payload,
    // serialized before `StatusPayload::capture` existed.
    let fixture = std::fs::read(fixtures().join("m2-status/legacy-capture-none.json")).unwrap();
    let frozen = fixture.strip_suffix(b"\n").expect("one trailing newline");
    let payload: StatusPayload = serde_json::from_slice(frozen).unwrap();
    assert_eq!(
        payload.capture, None,
        "a legacy document decodes without capture"
    );
    assert_eq!(
        serde_json::to_vec(&payload).unwrap(),
        frozen,
        "a legacy payload re-serializes byte for byte"
    );
    let value: serde_json::Value = serde_json::from_slice(frozen).unwrap();
    assert!(value.get("capture").is_none());
    assert_eq!(
        serde_json::to_value(&payload).unwrap(),
        value,
        "field for field"
    );
}

#[test]
fn m2_status_fixtures_are_canonical_and_name_their_states() {
    let expected: &[(&str, RunState, CaptureState, DeliveryState)] = &[
        (
            "starting",
            RunState::Starting,
            CaptureState::Starting,
            DeliveryState::LocalOnly,
        ),
        (
            "recording-pending",
            RunState::Running,
            CaptureState::Recording,
            DeliveryState::Pending,
        ),
        (
            "quota-only-experiment",
            RunState::Running,
            CaptureState::Recording,
            DeliveryState::LocalOnly,
        ),
        (
            "incomplete-synced",
            RunState::Running,
            CaptureState::Incomplete,
            DeliveryState::Synced,
        ),
        (
            "failed-local-only",
            RunState::Running,
            CaptureState::Failed,
            DeliveryState::LocalOnly,
        ),
        (
            "exited",
            RunState::Exited,
            CaptureState::Recording,
            DeliveryState::LocalOnly,
        ),
        (
            "sealed-with-gap",
            RunState::Sealed,
            CaptureState::Incomplete,
            DeliveryState::LocalOnly,
        ),
        (
            "accounting-unavailable",
            RunState::Running,
            CaptureState::Incomplete,
            DeliveryState::LocalOnly,
        ),
    ];
    let mut names: Vec<String> = std::fs::read_dir(fixtures().join("m2-status"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut want: Vec<String> = expected
        .iter()
        .map(|(n, ..)| format!("{n}.json"))
        .chain(["legacy-capture-none.json".to_string()])
        .collect();
    want.sort();
    assert_eq!(names, want, "exactly the shared status goldens");

    for (name, run, capture, delivery) in expected {
        let file = std::fs::read(fixtures().join(format!("m2-status/{name}.json"))).unwrap();
        let canonical = file.strip_suffix(b"\n").expect("one trailing newline");
        let payload: StatusPayload = serde_json::from_slice(canonical).unwrap();
        assert_eq!(
            serde_json::to_vec(&payload).unwrap(),
            canonical,
            "{name} is the canonical unsigned payload"
        );
        let m2 = payload
            .capture
            .as_ref()
            .unwrap_or_else(|| panic!("{name} carries capture"));
        assert_eq!(
            (m2.run, m2.capture, m2.delivery),
            (*run, *capture, *delivery),
            "{name}"
        );
        assert_eq!(
            m2.analysis,
            AnalysisState::NotRun,
            "{name}: status never guesses that analysis ran"
        );
        let storage = &m2.storage;
        assert!(
            storage.quota_bytes > 0,
            "{name}: quota is the admission ceiling"
        );
        assert!(storage.reserved_bytes <= storage.quota_bytes, "{name}");
        let wire: serde_json::Value = serde_json::from_slice(canonical).unwrap();
        assert_eq!(
            wire["capture"]["storage"].get("durable").is_some(),
            storage.durable.is_some(),
            "{name}: an absent durable frontier is omitted, never null"
        );
        match *name {
            "quota-only-experiment" => assert_eq!(
                storage.reserved_bytes, 0,
                "a quota-only bench never invents a reservation"
            ),
            "accounting-unavailable" => assert!(!storage.accounting_available),
            "sealed-with-gap" => {
                assert!(m2.gap_count > 0 && payload.sealed_at.is_some());
                assert!(m2.assistance_observed);
            }
            "incomplete-synced" => assert!(storage.durable.is_some()),
            _ => {
                assert!(storage.accounting_available, "{name}");
                assert!(
                    storage.reserved_bytes > 0,
                    "{name}: an installed entry reports its measured reservation"
                );
            }
        }
    }
}

#[test]
fn enum_and_record_serde_layouts_are_frozen() {
    for (source, golden) in [
        (CaptureSource::WireRequest, r#""wire_request""#),
        (CaptureSource::WireResponse, r#""wire_response""#),
        (CaptureSource::WsClient, r#""ws_client""#),
        (CaptureSource::WsServer, r#""ws_server""#),
        (CaptureSource::Native, r#""native""#),
        (CaptureSource::Goal, r#""goal""#),
    ] {
        layout(&source, golden);
    }
    for (outcome, golden) in [
        (ConnectionOutcome::Complete, r#""complete""#),
        (ConnectionOutcome::Interrupted, r#""interrupted""#),
        (ConnectionOutcome::TransportError, r#""transport_error""#),
    ] {
        layout(&outcome, golden);
    }
    for (reason, golden) in [
        (EndReason::Complete, r#""complete""#),
        (EndReason::Interrupted, r#""interrupted""#),
        (EndReason::DecodeError, r#""decode_error""#),
    ] {
        layout(&reason, golden);
    }
    for (reason, golden) in [
        (GapReason::QueueFull, r#""queue_full""#),
        (GapReason::IngestLost, r#""ingest_lost""#),
        (GapReason::ProducerLost, r#""producer_lost""#),
        (GapReason::StorageFull, r#""storage_full""#),
        (GapReason::StorageIo, r#""storage_io""#),
        (GapReason::UnsupportedEncoding, r#""unsupported_encoding""#),
        (GapReason::DecodeFailed, r#""decode_failed""#),
        (GapReason::SourceChanged, r#""source_changed""#),
        (GapReason::LimitExceeded, r#""limit_exceeded""#),
        (GapReason::ShutdownDeadline, r#""shutdown_deadline""#),
    ] {
        layout(&reason, golden);
    }
    for (event, golden) in [
        (
            ConnectionEvent::Open {
                alpn: Some("http/1.1".to_string()),
                already_open: false,
            },
            r#"{"kind":"open","alpn":"http/1.1","already_open":false}"#,
        ),
        (
            ConnectionEvent::UpgradedWebSocket,
            r#"{"kind":"upgraded_web_socket"}"#,
        ),
        (
            ConnectionEvent::Close {
                outcome: ConnectionOutcome::TransportError,
            },
            r#"{"kind":"close","outcome":"transport_error"}"#,
        ),
    ] {
        layout(&event, golden);
    }
    for (state, golden) in [
        (RunState::Starting, r#""starting""#),
        (RunState::Running, r#""running""#),
        (RunState::Exited, r#""exited""#),
        (RunState::Sealed, r#""sealed""#),
    ] {
        layout(&state, golden);
    }
    for (state, golden) in [
        (CaptureState::Starting, r#""starting""#),
        (CaptureState::Recording, r#""recording""#),
        (CaptureState::Incomplete, r#""incomplete""#),
        (CaptureState::Failed, r#""failed""#),
    ] {
        layout(&state, golden);
    }
    for (state, golden) in [
        (DeliveryState::LocalOnly, r#""local_only""#),
        (DeliveryState::Pending, r#""pending""#),
        (DeliveryState::Synced, r#""synced""#),
        (DeliveryState::Stuck, r#""stuck""#),
    ] {
        layout(&state, golden);
    }
    for (state, golden) in [
        (AnalysisState::NotRun, r#""not_run""#),
        (AnalysisState::Available, r#""available""#),
        (AnalysisState::Partial, r#""partial""#),
        (AnalysisState::Unavailable, r#""unavailable""#),
    ] {
        layout(&state, golden);
    }
    layout(&AnchorState::LocalOnly, r#""local_only""#);
    layout(&AnchorState::Signed, r#""signed""#);

    let t = r#"{"wall":"2026-09-27T10:00:00.250Z","mono_ns":5,"clock_id":"0123456789abcdef0123456789abcdef"}"#;
    let k = r#"{"producer":"0123456789abcdef0123456789abcdef","stream":1}"#;
    let frame = |stream: u64, seq: u64, event: FrameEvent| CaptureFrame {
        stream: key(stream),
        seq,
        observed: at(PRODUCER, 5),
        event,
    };
    let frames = [
        (
            frame(
                0,
                0,
                FrameEvent::Connection {
                    connection: CONNECTION.to_string(),
                    event: ConnectionEvent::Open {
                        alpn: None,
                        already_open: true,
                    },
                },
            ),
            format!(
                r#"{{"stream":{{"producer":"{PRODUCER}","stream":0}},"seq":0,"observed":{t},"event":{{"kind":"connection","connection":"{CONNECTION}","event":{{"kind":"open","alpn":null,"already_open":true}}}}}}"#
            ),
        ),
        (
            frame(
                0,
                1,
                FrameEvent::Recovery {
                    previous_producer: NEXT_PRODUCER.to_string(),
                    reason: GapReason::QueueFull,
                },
            ),
            format!(
                r#"{{"stream":{{"producer":"{PRODUCER}","stream":0}},"seq":1,"observed":{t},"event":{{"kind":"recovery","previous_producer":"{NEXT_PRODUCER}","reason":"queue_full"}}}}"#
            ),
        ),
        (
            frame(
                1,
                2,
                FrameEvent::Open {
                    source: CaptureSource::WireRequest,
                    metadata: http_meta(),
                },
            ),
            format!(
                r#"{{"stream":{k},"seq":2,"observed":{t},"event":{{"kind":"open","source":"wire_request","metadata":{{"wire":{{"connection":"{CONNECTION}","exchange":1,"message_index":null}},"label":"POST /v1/responses","protocol_hint":"openai-responses","method":"POST","host":"api.example.test","path":"/v1/responses","status":null,"content_type":"application/json"}}}}}}"#
            ),
        ),
        (
            frame(
                1,
                3,
                FrameEvent::Bytes {
                    offset: 0,
                    data: b"hi".to_vec(),
                },
            ),
            format!(
                r#"{{"stream":{k},"seq":3,"observed":{t},"event":{{"kind":"bytes","offset":0,"data":[104,105]}}}}"#
            ),
        ),
        (
            frame(
                1,
                4,
                FrameEvent::Gap {
                    reason: GapReason::DecodeFailed,
                    lost_bytes: Some(10),
                },
            ),
            format!(
                r#"{{"stream":{k},"seq":4,"observed":{t},"event":{{"kind":"gap","reason":"decode_failed","lost_bytes":10}}}}"#
            ),
        ),
        (
            frame(
                1,
                5,
                FrameEvent::End {
                    reason: EndReason::Complete,
                },
            ),
            format!(
                r#"{{"stream":{k},"seq":5,"observed":{t},"event":{{"kind":"end","reason":"complete"}}}}"#
            ),
        ),
        // H-16: the stream-zero loss control, appended so no index above moves.
        (
            frame(
                0,
                6,
                FrameEvent::Loss {
                    reason: GapReason::DecodeFailed,
                },
            ),
            format!(
                r#"{{"stream":{{"producer":"{PRODUCER}","stream":0}},"seq":6,"observed":{t},"event":{{"kind":"loss","reason":"decode_failed"}}}}"#
            ),
        ),
    ];
    for (value, golden) in &frames {
        layout(value, golden);
    }
    layout(
        &IngestMessage::Frame(frames[5].0.clone()),
        &format!(
            r#"{{"kind":"frame","stream":{k},"seq":5,"observed":{t},"event":{{"kind":"end","reason":"complete"}}}}"#
        ),
    );
    layout(
        &IngestMessage::Close {
            producer: PRODUCER.to_string(),
            final_seq: None,
        },
        &format!(r#"{{"kind":"close","producer":"{PRODUCER}","final_seq":null}}"#),
    );

    let d = r#"{"wall":"2026-09-27T10:00:00.250Z","mono_ns":9,"clock_id":"daemon-5b0c3a1e"}"#;
    let records = [
        (
            CaptureRecord::ProducerClosed {
                producer: PRODUCER.to_string(),
                observed: at(DAEMON_CLOCK, 9),
                final_seq: Some(5),
            },
            format!(
                r#"{{"kind":"producer_closed","producer":"{PRODUCER}","observed":{d},"final_seq":5}}"#
            ),
        ),
        (
            CaptureRecord::Connection {
                producer: PRODUCER.to_string(),
                connection: CONNECTION.to_string(),
                observed: at(PRODUCER, 5),
                event: ConnectionEvent::Close {
                    outcome: ConnectionOutcome::Complete,
                },
            },
            format!(
                r#"{{"kind":"connection","producer":"{PRODUCER}","connection":"{CONNECTION}","observed":{t},"event":{{"kind":"close","outcome":"complete"}}}}"#
            ),
        ),
        (
            CaptureRecord::Recovery {
                previous_producer: NEXT_PRODUCER.to_string(),
                producer: PRODUCER.to_string(),
                observed: at(PRODUCER, 5),
                reason: GapReason::IngestLost,
            },
            format!(
                r#"{{"kind":"recovery","previous_producer":"{NEXT_PRODUCER}","producer":"{PRODUCER}","observed":{t},"reason":"ingest_lost"}}"#
            ),
        ),
        (
            CaptureRecord::Open {
                stream: key(1),
                observed: at(PRODUCER, 5),
                source: CaptureSource::Goal,
                metadata: CaptureMeta {
                    wire: None,
                    label: "goal".to_string(),
                    protocol_hint: None,
                    method: None,
                    host: None,
                    path: None,
                    status: None,
                    content_type: None,
                },
            },
            format!(
                r#"{{"kind":"open","stream":{k},"observed":{t},"source":"goal","metadata":{{"wire":null,"label":"goal","protocol_hint":null,"method":null,"host":null,"path":null,"status":null,"content_type":null}}}}"#
            ),
        ),
        (
            CaptureRecord::Segment {
                stream: key(1),
                part: 0,
                source_offset: 0,
                source_len: 12,
                first: at(PRODUCER, 5),
                last: at(PRODUCER, 5),
                data_b64: "c2stW1JFREFDVEVEXQ==".to_string(),
                audit: RedactionAudit {
                    spans: vec![RedactionAuditSpan {
                        kind: "openai_key".to_string(),
                        start: 0,
                        end: 12,
                    }],
                },
            },
            format!(
                r#"{{"kind":"segment","stream":{k},"part":0,"source_offset":0,"source_len":12,"first":{t},"last":{t},"data_b64":"c2stW1JFREFDVEVEXQ==","audit":{{"spans":[{{"kind":"openai_key","start":0,"end":12}}]}}}}"#
            ),
        ),
        (
            CaptureRecord::End {
                stream: key(1),
                observed: at(PRODUCER, 5),
                reason: EndReason::Interrupted,
            },
            format!(r#"{{"kind":"end","stream":{k},"observed":{t},"reason":"interrupted"}}"#),
        ),
        (
            CaptureRecord::Gap {
                stream: Some(key(1)),
                observed: at(DAEMON_CLOCK, 9),
                reason: GapReason::StorageFull,
                lost_bytes: Some(4096),
            },
            format!(
                r#"{{"kind":"gap","stream":{k},"observed":{d},"reason":"storage_full","lost_bytes":4096}}"#
            ),
        ),
        (
            CaptureRecord::Launch {
                observed: at(DAEMON_CLOCK, 9),
                goal_stream: key(1),
                profile_id: "codex-cli".to_string(),
                profile_digest: "c".repeat(64),
                harness_version: "0.153.0".to_string(),
                recorder_revision: "19a52ea".to_string(),
                continuation: "none".to_string(),
                allowed_tool_rule: None,
                process_handoff_digest: None,
                run_public_key: None,
                connector_dir_sha256: None,
                harness_build: None,
            },
            format!(
                r#"{{"kind":"launch","observed":{d},"goal_stream":{k},"profile_id":"codex-cli","profile_digest":"{}","harness_version":"0.153.0","recorder_revision":"19a52ea","continuation":"none"}}"#,
                "c".repeat(64)
            ),
        ),
        // The fully populated shape: every optional `Launch` field present, pinning the byte
        // order. That a record gains no field after the v0.5.0 tag is not enforced here but by
        // `v5_field_and_variant_lists_are_pinned_from_serdes_own_errors` below, which does not
        // depend on what any literal in this file carries.
        (
            CaptureRecord::Launch {
                observed: at(DAEMON_CLOCK, 9),
                goal_stream: key(1),
                profile_id: "claude-code-cli".to_string(),
                profile_digest: "c".repeat(64),
                harness_version: "2.0".to_string(),
                recorder_revision: "19a52ea".to_string(),
                continuation: "none".to_string(),
                allowed_tool_rule: Some("Bash(python3 demo.py *)".to_string()),
                process_handoff_digest: Some("d".repeat(64)),
                run_public_key: Some("a".repeat(64)),
                connector_dir_sha256: Some("b".repeat(64)),
                harness_build: Some(Box::new(HarnessBuildV1 {
                    program_sha256: Some("e".repeat(64)),
                    image_sha256: Some("f".repeat(64)),
                    platform: "macos-aarch64".to_string(),
                })),
            },
            format!(
                r#"{{"kind":"launch","observed":{d},"goal_stream":{k},"profile_id":"claude-code-cli","profile_digest":"{}","harness_version":"2.0","recorder_revision":"19a52ea","continuation":"none","allowed_tool_rule":"Bash(python3 demo.py *)","process_handoff_digest":"{}","run_public_key":"{}","connector_dir_sha256":"{}","harness_build":{{"program_sha256":"{}","image_sha256":"{}","platform":"macos-aarch64"}}}}"#,
                "c".repeat(64),
                "d".repeat(64),
                "a".repeat(64),
                "b".repeat(64),
                "e".repeat(64),
                "f".repeat(64)
            ),
        ),
        (
            CaptureRecord::Assistance {
                observed: at(DAEMON_CLOCK, 9),
                action: "manual_resume".to_string(),
            },
            format!(r#"{{"kind":"assistance","observed":{d},"action":"manual_resume"}}"#),
        ),
        (
            CaptureRecord::LocalFinish {
                observed: at(DAEMON_CLOCK, 9),
                reason: "manual".to_string(),
                termination: TerminationOutcomeV1::Confirmed {
                    requested_at: Rfc3339Utc::try_from("2026-09-27T12:00:00Z").unwrap(),
                    confirmed_at: Rfc3339Utc::try_from("2026-09-27T12:00:01Z").unwrap(),
                    method: TerminationMethodV1::ProcessSignal,
                },
                seal_manifest_sha256: "d".repeat(64),
            },
            format!(
                r#"{{"kind":"local_finish","observed":{d},"reason":"manual","termination":{{"confirmed":{{"requested_at":"2026-09-27T12:00:00Z","confirmed_at":"2026-09-27T12:00:01Z","method":"process_signal"}}}},"seal_manifest_sha256":"{}"}}"#,
                "d".repeat(64)
            ),
        ),
    ];
    for (value, golden) in &records {
        layout(value, golden);
    }
    // The field set of each of the ten record kinds, as the frozen goldens spell them. A new
    // field, optional or not, changes a set here and is protocol v6.
    let mut field_sets =
        std::collections::BTreeMap::<String, std::collections::BTreeSet<String>>::new();
    for (value, _) in &records {
        let json = serde_json::to_value(value).unwrap();
        let object = json.as_object().unwrap();
        field_sets
            .entry(object["kind"].as_str().unwrap().to_string())
            .or_default()
            .extend(object.keys().cloned());
    }
    let table: Vec<(String, Vec<String>)> = field_sets
        .into_iter()
        .map(|(kind, fields)| (kind, fields.into_iter().collect()))
        .collect();
    let want: &[(&str, &[&str])] = &[
        ("assistance", &["action", "kind", "observed"]),
        (
            "connection",
            &["connection", "event", "kind", "observed", "producer"],
        ),
        ("end", &["kind", "observed", "reason", "stream"]),
        (
            "gap",
            &["kind", "lost_bytes", "observed", "reason", "stream"],
        ),
        (
            "launch",
            &[
                "allowed_tool_rule",
                "connector_dir_sha256",
                "continuation",
                "goal_stream",
                // D53: the one field admitted after the v0.5.0 tag.
                "harness_build",
                "harness_version",
                "kind",
                "observed",
                "process_handoff_digest",
                "profile_digest",
                "profile_id",
                "recorder_revision",
                "run_public_key",
            ],
        ),
        (
            "local_finish",
            &[
                "kind",
                "observed",
                "reason",
                "seal_manifest_sha256",
                "termination",
            ],
        ),
        (
            "open",
            &["kind", "metadata", "observed", "source", "stream"],
        ),
        (
            "producer_closed",
            &["final_seq", "kind", "observed", "producer"],
        ),
        (
            "recovery",
            &[
                "kind",
                "observed",
                "previous_producer",
                "producer",
                "reason",
            ],
        ),
        (
            "segment",
            &[
                "audit",
                "data_b64",
                "first",
                "kind",
                "last",
                "part",
                "source_len",
                "source_offset",
                "stream",
            ],
        ),
    ];
    let want: Vec<(String, Vec<String>)> = want
        .iter()
        .map(|(kind, fields)| {
            (
                kind.to_string(),
                fields.iter().map(|f| f.to_string()).collect(),
            )
        })
        .collect();
    assert_eq!(
        table, want,
        "a v5 record's field set changed: that is protocol v6"
    );

    layout(
        &M2Status {
            run: RunState::Running,
            capture: CaptureState::Recording,
            delivery: DeliveryState::LocalOnly,
            analysis: AnalysisState::NotRun,
            anchor: AnchorState::LocalOnly,
            storage: CaptureStorage {
                quota_bytes: 1024,
                reserved_bytes: 0,
                used_bytes: 10,
                durable: None,
                accounting_available: true,
            },
            gap_count: 0,
            assistance_observed: false,
        },
        r#"{"run":"running","capture":"recording","delivery":"local_only","analysis":"not_run","anchor":"local_only","storage":{"quota_bytes":1024,"reserved_bytes":0,"used_bytes":10,"accounting_available":true},"gap_count":0,"assistance_observed":false}"#,
    );
    layout(
        &DurableFrontier {
            seq: 3,
            hash: "e".repeat(64),
            chain_bytes: 900,
        },
        &format!(
            r#"{{"seq":3,"hash":"{}","chain_bytes":900}}"#,
            "e".repeat(64)
        ),
    );
}

#[test]
fn unknown_fields_are_rejected_at_every_frame_level() {
    let valid = serde_json::to_value(IngestMessage::Frame(CaptureFrame {
        stream: key(1),
        seq: 2,
        observed: at(PRODUCER, 5),
        event: FrameEvent::Open {
            source: CaptureSource::WireRequest,
            metadata: http_meta(),
        },
    }))
    .unwrap();
    assert!(serde_json::from_value::<IngestMessage>(valid.clone()).is_ok());
    let pointers = [
        "",
        "/stream",
        "/observed",
        "/event",
        "/event/metadata",
        "/event/metadata/wire",
    ];
    for pointer in pointers {
        let mut doctored = valid.clone();
        doctored
            .pointer_mut(pointer)
            .and_then(serde_json::Value::as_object_mut)
            .unwrap_or_else(|| panic!("{pointer} is an object"))
            .insert("headers".to_string(), serde_json::json!({"cookie": "x"}));
        assert!(
            serde_json::from_value::<IngestMessage>(doctored).is_err(),
            "an unknown field at `{pointer}` must be refused"
        );
    }
    // The connection event nested in a control frame, including its fieldless variant (a
    // serde unit variant would silently ignore the extra field).
    for event in [
        ConnectionEvent::UpgradedWebSocket,
        ConnectionEvent::Open {
            alpn: None,
            already_open: false,
        },
        ConnectionEvent::Close {
            outcome: ConnectionOutcome::Complete,
        },
    ] {
        let control = serde_json::to_value(IngestMessage::Frame(CaptureFrame {
            stream: key(0),
            seq: 0,
            observed: at(PRODUCER, 5),
            event: FrameEvent::Connection {
                connection: CONNECTION.to_string(),
                event: event.clone(),
            },
        }))
        .unwrap();
        let back: IngestMessage = serde_json::from_value(control.clone()).unwrap();
        assert!(
            matches!(back, IngestMessage::Frame(f) if matches!(&f.event, FrameEvent::Connection { event: e, .. } if *e == event))
        );
        let mut doctored = control;
        doctored
            .pointer_mut("/event/event")
            .and_then(serde_json::Value::as_object_mut)
            .unwrap()
            .insert("headers".to_string(), serde_json::json!({"cookie": "x"}));
        assert!(
            serde_json::from_value::<IngestMessage>(doctored).is_err(),
            "an unknown field on {event:?} must be refused"
        );
    }
    // The stream-zero loss control and its event body refuse an unknown field too.
    let loss = serde_json::to_value(IngestMessage::Frame(CaptureFrame {
        stream: key(0),
        seq: 0,
        observed: at(PRODUCER, 5),
        event: FrameEvent::Loss {
            reason: GapReason::DecodeFailed,
        },
    }))
    .unwrap();
    assert!(serde_json::from_value::<IngestMessage>(loss.clone()).is_ok());
    for pointer in ["", "/stream", "/observed", "/event"] {
        let mut doctored = loss.clone();
        doctored
            .pointer_mut(pointer)
            .and_then(serde_json::Value::as_object_mut)
            .unwrap_or_else(|| panic!("{pointer} is an object"))
            .insert("bytes".to_string(), serde_json::json!(1));
        assert!(
            serde_json::from_value::<IngestMessage>(doctored).is_err(),
            "an unknown field on a loss at `{pointer}` must be refused"
        );
    }
    let close = serde_json::json!({"kind":"close","producer":PRODUCER,"final_seq":1,"ack":true});
    assert!(serde_json::from_value::<IngestMessage>(close).is_err());
    let record = serde_json::json!({
        "kind":"end","stream":{"producer":PRODUCER,"stream":1},
        "observed":{"wall":"2026-09-27T10:00:00Z","mono_ns":1,"clock_id":PRODUCER},
        "reason":"complete","extra":1
    });
    assert!(serde_json::from_value::<CaptureRecord>(record).is_err());
}

#[test]
fn huge_metadata_is_refused_before_any_buffer_is_produced() {
    let mut meta = http_meta();
    meta.label = "x".repeat(10 * 1024 * 1024);
    let message = IngestMessage::Frame(CaptureFrame {
        stream: key(1),
        seq: 0,
        observed: at(PRODUCER, 1),
        event: FrameEvent::Open {
            source: CaptureSource::WireRequest,
            metadata: meta,
        },
    });
    // The refusal is the pre-serialization per-field bound, not the post-serialization
    // frame bound: the 10 MiB value was never encoded.
    assert_eq!(
        encode_ingest_message(&message),
        Err(CaptureLimitError::TooLong {
            field: "metadata.label",
            len: 10 * 1024 * 1024,
            limit: MAX_METADATA_STRING_BYTES
        })
    );

    for (field, set) in [
        (
            "metadata.path",
            (|m: &mut CaptureMeta| m.path = Some(format!("/{}", "p".repeat(1024))))
                as fn(&mut CaptureMeta),
        ),
        ("metadata.host", |m| m.host = Some("h".repeat(1025))),
        ("metadata.content_type", |m| {
            m.content_type = Some("t".repeat(1025))
        }),
        ("metadata.protocol_hint", |m| {
            m.protocol_hint = Some("q".repeat(1025))
        }),
        ("metadata.method", |m| m.method = Some("M".repeat(1025))),
    ] {
        let mut meta = http_meta();
        set(&mut meta);
        assert!(
            matches!(meta.validate(), Err(CaptureLimitError::TooLong { field: f, .. }) if f == field),
            "{field}"
        );
    }
    let mut at_limit = http_meta();
    at_limit.label = "x".repeat(MAX_METADATA_STRING_BYTES);
    assert_eq!(at_limit.validate(), Ok(()), "exactly 1 KiB is admitted");

    // A length prefix is refused before a body is allocated.
    assert_eq!(
        checked_frame_len(u32::MAX.to_be_bytes(), MAX_INGEST_FRAME_BYTES),
        Err(CaptureLimitError::FrameTooLarge {
            len: u32::MAX as usize,
            limit: MAX_INGEST_FRAME_BYTES
        })
    );
    assert_eq!(
        checked_frame_len(
            (MAX_INGEST_FRAME_BYTES as u32).to_be_bytes(),
            MAX_INGEST_FRAME_BYTES
        ),
        Ok(MAX_INGEST_FRAME_BYTES)
    );
    assert!(checked_frame_len(1025u32.to_be_bytes(), MAX_HANDSHAKE_BYTES).is_err());
    assert!(
        decode_ingest_message(&vec![b' '; MAX_INGEST_FRAME_BYTES + 1]).is_err(),
        "an over-limit body is refused before parsing"
    );

    // Payload bytes are bounded by the segment limit.
    let over = CaptureFrame {
        stream: key(1),
        seq: 1,
        observed: at(PRODUCER, 1),
        event: FrameEvent::Bytes {
            offset: 0,
            data: vec![0; MAX_SEGMENT_BYTES + 1],
        },
    };
    assert!(matches!(
        over.validate(),
        Err(CaptureLimitError::TooLong {
            field: "event.data",
            ..
        })
    ));
    let overflow = CaptureFrame {
        event: FrameEvent::Bytes {
            offset: u64::MAX,
            data: vec![1],
        },
        ..over.clone()
    };
    assert!(overflow.validate().is_err());
}

#[test]
fn frame_representation_rules_are_enforced() {
    let good = CaptureFrame {
        stream: key(1),
        seq: 0,
        observed: at(PRODUCER, 1),
        event: FrameEvent::Open {
            source: CaptureSource::WireRequest,
            metadata: http_meta(),
        },
    };
    assert_eq!(good.validate(), Ok(()));
    let with = |f: &dyn Fn(&mut CaptureFrame)| {
        let mut frame = good.clone();
        f(&mut frame);
        frame.validate()
    };
    // Stream zero is reserved for, and required by, the three controls.
    assert!(with(&|f| f.stream.stream = 0).is_err());
    // A loss is a stream-zero control: refused on a payload stream, accepted on zero. A
    // connection control on a payload stream is refused as before.
    for stream in [1, 3] {
        assert!(
            with(&|f| {
                f.stream.stream = stream;
                f.event = FrameEvent::Loss {
                    reason: GapReason::DecodeFailed,
                };
            })
            .is_err(),
            "a loss on stream {stream}"
        );
    }
    assert_eq!(
        with(&|f| {
            f.stream.stream = 0;
            f.event = FrameEvent::Loss {
                reason: GapReason::LimitExceeded,
            };
        }),
        Ok(())
    );
    assert!(
        with(&|f| f.event = FrameEvent::Connection {
            connection: CONNECTION.to_string(),
            event: ConnectionEvent::UpgradedWebSocket,
        })
        .is_err()
    );
    assert_eq!(
        with(&|f| {
            f.stream.stream = 0;
            f.event = FrameEvent::Connection {
                connection: CONNECTION.to_string(),
                event: ConnectionEvent::UpgradedWebSocket,
            };
        }),
        Ok(())
    );
    // Only http/1.1 or no ALPN.
    assert!(
        with(&|f| {
            f.stream.stream = 0;
            f.event = FrameEvent::Connection {
                connection: CONNECTION.to_string(),
                event: ConnectionEvent::Open {
                    alpn: Some("h2".to_string()),
                    already_open: false,
                },
            };
        })
        .is_err()
    );
    // Recovery names another producer.
    assert!(
        with(&|f| {
            f.stream.stream = 0;
            f.event = FrameEvent::Recovery {
                previous_producer: PRODUCER.to_string(),
                reason: GapReason::QueueFull,
            };
        })
        .is_err()
    );
    // IDs are fresh 128-bit hex.
    assert!(with(&|f| f.stream.producer = "producer-1".to_string()).is_err());
    assert!(with(&|f| f.stream.producer = PRODUCER.to_uppercase()).is_err());
    // Path never carries a query or fragment.
    for path in ["/v1/responses?key=x", "/v1/responses#frag"] {
        assert!(
            with(&|f| {
                if let FrameEvent::Open { metadata, .. } = &mut f.event {
                    metadata.path = Some(path.to_string());
                }
            })
            .is_err(),
            "{path}"
        );
    }
    // Wire identity matches the source.
    assert!(
        with(&|f| {
            if let FrameEvent::Open { source, .. } = &mut f.event {
                *source = CaptureSource::Native;
            }
        })
        .is_err()
    );
    assert!(
        with(&|f| {
            if let FrameEvent::Open { source, .. } = &mut f.event {
                *source = CaptureSource::WsClient;
            }
        })
        .is_err(),
        "a WebSocket stream carries a message index"
    );
    // Wall time is RFC 3339 UTC.
    assert!(with(&|f| f.observed.wall = "yesterday".to_string()).is_err());
    assert!(with(&|f| f.observed.clock_id = String::new()).is_err());
}

#[test]
fn decode_errors_never_echo_their_input() {
    let secret = "sk-live-planted-secret";
    let body = format!(r#"{{"kind":"close","producer":"{secret}""#);
    let error = decode_ingest_message(body.as_bytes()).unwrap_err();
    assert_eq!(error, CaptureLimitError::Malformed);
    assert!(!error.to_string().contains(secret));
    let invalid = format!(r#"{{"kind":"close","producer":"{secret}","final_seq":null}}"#);
    let error = decode_ingest_message(invalid.as_bytes()).unwrap_err();
    assert!(!error.to_string().contains(secret), "{error}");
}

/// The shared golden: a length-framed handshake followed by length-framed ingest messages.
/// The receiver and the sender both conform to these exact bytes.
#[test]
fn ingest_handshake_and_frames_match_the_shared_golden() {
    let dir = fixtures().join("m2-ingest");
    let decoded: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("golden-stream.json")).unwrap()).unwrap();
    let hex_text = std::fs::read_to_string(dir.join("golden-stream.hex")).unwrap();
    let golden: Vec<u8> = hex::decode(hex_text.split_whitespace().collect::<String>()).unwrap();

    // Encode: handshake object (compact JSON, fields in this order), then each message.
    let handshake = &decoded["handshake"];
    let handshake_bytes = serde_json::to_vec(&serde_json::json!({
        "entry_id": handshake["entry_id"],
        "producer": handshake["producer"],
        "token": handshake["token"],
    }))
    .unwrap();
    assert!(handshake_bytes.len() <= MAX_HANDSHAKE_BYTES);
    let mut encoded = (handshake_bytes.len() as u32).to_be_bytes().to_vec();
    encoded.extend_from_slice(&handshake_bytes);
    let messages: Vec<IngestMessage> = serde_json::from_value(decoded["messages"].clone()).unwrap();
    for message in &messages {
        encoded.extend_from_slice(&encode_ingest_message(message).unwrap());
    }
    assert_eq!(
        encoded, golden,
        "the golden bytes are exactly this encoding"
    );

    // Decode: bounded prefix first, then the body.
    let mut rest = golden.as_slice();
    let take = |rest: &mut &[u8], limit: usize| -> Vec<u8> {
        let len = checked_frame_len(rest[..4].try_into().unwrap(), limit).unwrap();
        let body = rest[4..4 + len].to_vec();
        *rest = &rest[4 + len..];
        body
    };
    let first = take(&mut rest, MAX_HANDSHAKE_BYTES);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&first).unwrap(),
        *handshake
    );
    let mut seen = Vec::new();
    while !rest.is_empty() {
        seen.push(decode_ingest_message(&take(&mut rest, MAX_INGEST_FRAME_BYTES)).unwrap());
    }
    assert_eq!(seen, messages);
    let IngestMessage::Close { final_seq, .. } = seen.last().unwrap() else {
        panic!("the golden ends with an ordered close");
    };
    let frames = seen.len() as u64 - 1;
    assert_eq!(
        *final_seq,
        Some(frames - 1),
        "close names the last admitted frame's sequence"
    );
}

/// Timestamps from different clocks are never subtracted, and the canonical record sequence
/// keeps each observation in its own clock domain.
#[test]
fn clock_domains_are_never_mixed() {
    let producer_start = at(PRODUCER, 1_000);
    let producer_later = at(PRODUCER, 4_000);
    let daemon = at(DAEMON_CLOCK, 900_000);
    assert_eq!(producer_start.mono_elapsed_ns(&producer_later), Some(3_000));
    assert_eq!(
        producer_later.mono_elapsed_ns(&producer_start),
        None,
        "never negative"
    );
    assert_eq!(
        producer_start.mono_elapsed_ns(&daemon),
        None,
        "no cross-clock subtraction"
    );
    assert_eq!(daemon.mono_elapsed_ns(&producer_later), None);

    let text = std::fs::read_to_string(fixtures().join("m2-capture/clock-domains.jsonl")).unwrap();
    let records: Vec<CaptureRecord> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(records.len() >= 8);
    let mut producers = std::collections::BTreeSet::new();
    let mut daemon_gap_on_stream = false;
    for record in &records {
        match record {
            // Retained producer observations keep the producer's own clock.
            CaptureRecord::Open {
                stream, observed, ..
            }
            | CaptureRecord::End {
                stream, observed, ..
            } => {
                assert_eq!(observed.clock_id, stream.producer, "{record:?}");
                producers.insert(stream.producer.clone());
            }
            CaptureRecord::Segment {
                stream,
                first,
                last,
                ..
            } => {
                assert_eq!(first.clock_id, stream.producer);
                assert_eq!(last.clock_id, stream.producer);
                assert!(first.mono_elapsed_ns(last).is_some(), "first precedes last");
            }
            CaptureRecord::Connection {
                producer, observed, ..
            }
            | CaptureRecord::Recovery {
                producer, observed, ..
            } => {
                assert_eq!(&observed.clock_id, producer, "{record:?}");
                producers.insert(producer.clone());
            }
            // A gap on a stream is either the producer's own observation or the daemon's;
            // a daemon-generated gap names the daemon clock, never the borrowed stream's.
            CaptureRecord::Gap {
                stream: Some(stream),
                observed,
                reason,
                ..
            } => {
                assert!(
                    observed.clock_id == stream.producer || observed.clock_id == DAEMON_CLOCK,
                    "{record:?}"
                );
                if *reason == GapReason::ProducerLost {
                    daemon_gap_on_stream = true;
                    assert_eq!(observed.clock_id, DAEMON_CLOCK, "{record:?}");
                }
            }
            // Daemon-generated loss and control observations name the daemon's clock.
            CaptureRecord::ProducerClosed { observed, .. }
            | CaptureRecord::Gap {
                stream: None,
                observed,
                ..
            }
            | CaptureRecord::Launch { observed, .. }
            | CaptureRecord::Assistance { observed, .. }
            | CaptureRecord::LocalFinish { observed, .. } => {
                assert_eq!(observed.clock_id, DAEMON_CLOCK, "{record:?}");
            }
        }
    }
    assert!(producers.len() >= 2, "the fixture spans a recovery");
    assert!(
        daemon_gap_on_stream,
        "the fixture holds a daemon-observed loss on a producer's stream"
    );
    assert!(!producers.contains(DAEMON_CLOCK));
    // A daemon-clock gap and a producer-clock segment have no elapsed duration between them.
    let gap = records
        .iter()
        .find_map(|r| match r {
            CaptureRecord::Gap { observed, .. } => Some(observed),
            _ => None,
        })
        .unwrap();
    let segment = records
        .iter()
        .find_map(|r| match r {
            CaptureRecord::Segment { last, .. } => Some(last),
            _ => None,
        })
        .unwrap();
    assert_eq!(segment.mono_elapsed_ns(gap), None);
    assert_eq!(gap.mono_elapsed_ns(segment), None);
}

/// Contract S1 (D43 ruling 7): `Launch` gained two optional, skip-if-none fields, so a record
/// written before S1 must re-serialize to the very bytes it was read from. The verifier relies
/// on exactly this (each v5 line must equal its own serialization). The fixture is the
/// committed pre-S1 capture record file, `Launch` included.
#[test]
fn committed_pre_s1_capture_records_reserialize_byte_identically() {
    let text = std::fs::read_to_string(fixtures().join("m2-capture/clock-domains.jsonl")).unwrap();
    let mut launches = 0;
    for line in text.lines() {
        let record: CaptureRecord = serde_json::from_str(line).unwrap();
        assert_eq!(serde_json::to_string(&record).unwrap(), line);
        if let CaptureRecord::Launch {
            run_public_key,
            connector_dir_sha256,
            ..
        } = &record
        {
            launches += 1;
            assert_eq!((run_public_key, connector_dir_sha256), (&None, &None));
        }
    }
    assert_eq!(launches, 1, "the fixture carries a pre-S1 Launch");
}

/// The two S1 fields are written only when present, spelled as the design names them, and an
/// explicit `null` is not a spelling of absence (the writer never emits it, and the reader
/// refuses a line that is not its own serialization).
#[test]
fn launch_run_key_and_connector_dir_digest_are_additive_and_skip_if_none() {
    let mut launch = CaptureRecord::Launch {
        observed: at(DAEMON_CLOCK, 9),
        goal_stream: key(1),
        profile_id: "codex-cli".into(),
        profile_digest: "c".repeat(64),
        harness_version: "0.153.0".into(),
        recorder_revision: "test".into(),
        continuation: "none".into(),
        allowed_tool_rule: None,
        process_handoff_digest: None,
        run_public_key: None,
        connector_dir_sha256: None,
        harness_build: None,
    };
    let absent = serde_json::to_string(&launch).unwrap();
    assert!(!absent.contains("run_public_key") && !absent.contains("connector_dir_sha256"));
    if let CaptureRecord::Launch {
        run_public_key,
        connector_dir_sha256,
        ..
    } = &mut launch
    {
        *run_public_key = Some("a".repeat(64));
        *connector_dir_sha256 = Some("b".repeat(64));
    }
    let present = serde_json::to_string(&launch).unwrap();
    assert!(present.ends_with(&format!(
        r#","run_public_key":"{}","connector_dir_sha256":"{}"}}"#,
        "a".repeat(64),
        "b".repeat(64)
    )));
    assert_eq!(
        serde_json::from_str::<CaptureRecord>(&present).unwrap(),
        launch
    );
    // Unknown fields stay refused: the freeze admits these two and nothing else.
    let unknown = present.replace(r#""continuation":"none""#, r#""continuation":"none","x":1"#);
    assert!(serde_json::from_str::<CaptureRecord>(&unknown).is_err());
}

/// The full field list of each v5 record kind and sub-type, and the full variant list of each
/// v5 enum, as serde's own derive reports them in its unknown-field and unknown-variant errors.
/// Unlike a list taken from a golden, this one includes optional fields that the goldens leave
/// `None` and variants that no golden uses, so adding either one fails here whatever the
/// literals carry. After the v0.5.0 tag that is protocol v6 (`docs/CONTRACTS.md`, "The v5
/// format freeze"); before it, the change updates this table in the same commit.
#[test]
fn v5_field_and_variant_lists_are_pinned_from_serdes_own_errors() {
    fn error_of<T: serde::de::DeserializeOwned>(json: &str) -> String {
        let message = serde_json::from_str::<T>(json)
            .err()
            .unwrap_or_else(|| panic!("{json} must be refused"))
            .to_string();
        // Drop serde_json's position suffix; the list before it is the pin.
        message.split(" at line ").next().unwrap().to_string()
    }
    let field = |kind: &str| format!(r#"{{"kind":"{kind}","zz_unknown":1}}"#);
    let unknown_field = |list: &str| format!("unknown field `zz_unknown`, {list}");
    let record = |kind: &str, list: &str| {
        assert_eq!(
            error_of::<CaptureRecord>(&field(kind)),
            unknown_field(list),
            "the field list of the `{kind}` record changed: that is protocol v6"
        );
    };
    record(
        "open",
        "expected one of `stream`, `observed`, `source`, `metadata`",
    );
    record(
        "segment",
        "expected one of `stream`, `part`, `source_offset`, `source_len`, `first`, `last`, `data_b64`, `audit`",
    );
    record("end", "expected one of `stream`, `observed`, `reason`");
    record(
        "gap",
        "expected one of `stream`, `observed`, `reason`, `lost_bytes`",
    );
    record(
        "connection",
        "expected one of `producer`, `connection`, `observed`, `event`",
    );
    record(
        "recovery",
        "expected one of `previous_producer`, `producer`, `observed`, `reason`",
    );
    record(
        "producer_closed",
        "expected one of `producer`, `observed`, `final_seq`",
    );
    record(
        "launch",
        // `harness_build` joined after the v0.5.0 tag by D53, which accepts that a v0.5.0
        // reader refuses a trace carrying it (CONTRACTS.md, "The v5 format freeze").
        "expected one of `observed`, `goal_stream`, `profile_id`, `profile_digest`, `harness_version`, `recorder_revision`, `continuation`, `allowed_tool_rule`, `process_handoff_digest`, `run_public_key`, `connector_dir_sha256`, `harness_build`",
    );
    record("assistance", "expected `observed` or `action`");
    record(
        "local_finish",
        "expected one of `observed`, `reason`, `termination`, `seal_manifest_sha256`",
    );

    let sub = r#"{"zz_unknown":1}"#;
    assert_eq!(
        error_of::<StreamKey>(sub),
        unknown_field("expected `producer` or `stream`")
    );
    assert_eq!(
        error_of::<CaptureTime>(sub),
        unknown_field("expected one of `wall`, `mono_ns`, `clock_id`")
    );
    assert_eq!(
        error_of::<HarnessBuildV1>(sub),
        unknown_field("expected one of `program_sha256`, `image_sha256`, `platform`")
    );
    assert_eq!(
        error_of::<WireIdentity>(sub),
        unknown_field("expected one of `connection`, `exchange`, `message_index`")
    );
    assert_eq!(
        error_of::<CaptureMeta>(sub),
        unknown_field(
            "expected one of `wire`, `label`, `protocol_hint`, `method`, `host`, `path`, `status`, `content_type`"
        )
    );
    assert_eq!(
        error_of::<ConnectionEvent>(&field("open")),
        unknown_field("expected `alpn` or `already_open`")
    );
    assert_eq!(
        error_of::<ConnectionEvent>(&field("upgraded_web_socket")),
        unknown_field("there are no fields")
    );
    assert_eq!(
        error_of::<ConnectionEvent>(&field("close")),
        unknown_field("expected `outcome`")
    );

    let variants = |list: &str| format!("unknown variant `zz`, expected one of {list}");
    assert_eq!(
        error_of::<CaptureRecord>(&field("zz")),
        variants(
            "`producer_closed`, `connection`, `recovery`, `open`, `segment`, `end`, `gap`, `launch`, `assistance`, `local_finish`"
        ),
        "the record kinds changed: that is protocol v6"
    );
    assert_eq!(
        error_of::<ConnectionEvent>(&field("zz")),
        variants("`open`, `upgraded_web_socket`, `close`")
    );
    assert_eq!(
        error_of::<CaptureSource>(r#""zz""#),
        variants("`wire_request`, `wire_response`, `ws_client`, `ws_server`, `native`, `goal`")
    );
    assert_eq!(
        error_of::<ConnectionOutcome>(r#""zz""#),
        variants("`complete`, `interrupted`, `transport_error`")
    );
    assert_eq!(
        error_of::<EndReason>(r#""zz""#),
        variants("`complete`, `interrupted`, `decode_error`")
    );
    assert_eq!(
        error_of::<GapReason>(r#""zz""#),
        variants(
            "`queue_full`, `ingest_lost`, `producer_lost`, `storage_full`, `storage_io`, `unsupported_encoding`, `decode_failed`, `source_changed`, `limit_exceeded`, `shutdown_deadline`"
        )
    );
}
