//! Record and trace builders for the v5 reader corpus (`tests/v5_corpus.rs`).
//!
//! Deliberately self-contained: the corpus traces are frozen reader inputs, so their generator
//! does not borrow the clocks, ids or shapes of any other suite, which are free to change. The
//! suites with their own builders (`m2_capture_integrity.rs`, `m2_hermes_capture_only.rs`) use
//! other constants and are not folded in here.

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
use goalcup_protocol::export::TraceExport;
use goalcup_protocol::provenance::{
    CaptureUnavailableV1, CutoffReachedV1, FinalizationV1, NoRepositoryReasonV1,
    PreCutoffDrainOutcomeV1, PublisherNotAttemptedReasonV1, PublisherNotAttemptedV1,
    RepositoryNonSelectionV1, Rfc3339Utc, Sha256Digest, SnapshotNotAttemptedV1,
    TerminationMethodV1, TerminationOutcomeV1,
};
use goalcup_protocol::receipt::{StartReceipt, StartReceiptClaims};
use goalcup_protocol::redaction::RedactionAudit;
use goalcup_protocol::seal::{SealManifest, SealedFile};
use std::path::Path;

pub const ENTRY: &str = "entry-v5-corpus";
pub const GOAL: &str = "9d8c7b6a5f4e3d2c1b0a998877665544";
pub const WIRE: &str = "0123456789abcdef0123456789abcdef";
pub const NATIVE: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90";
pub const CONN: &str = "5e1f0a2b3c4d5e6f708192a3b4c5d6e7";
pub const DAEMON: &str = "daemon-corpus-clock";
pub const HERMES_SESSION: &str = "20260930_141500_a1b2c3";

pub fn at(clock: &str, tick: u64) -> CaptureTime {
    CaptureTime {
        wall: "2026-10-02T12:00:01Z".to_string(),
        mono_ns: tick,
        clock_id: clock.to_string(),
    }
}

pub fn key(producer: &str, stream: u64) -> StreamKey {
    StreamKey {
        producer: producer.to_string(),
        stream,
    }
}

pub fn utc(value: &str) -> Rfc3339Utc {
    Rfc3339Utc::try_from(value).unwrap()
}

pub fn meta(label: &str) -> CaptureMeta {
    CaptureMeta {
        wire: None,
        label: label.to_string(),
        protocol_hint: None,
        method: None,
        host: None,
        path: None,
        status: None,
        content_type: None,
    }
}

pub fn wire_meta(source: CaptureSource, exchange: u64) -> CaptureMeta {
    let request = source == CaptureSource::WireRequest;
    CaptureMeta {
        wire: Some(WireIdentity {
            connection: CONN.to_string(),
            exchange,
            message_index: None,
        }),
        label: "POST /v1/chat/completions".to_string(),
        protocol_hint: Some("openai-chat-completions".to_string()),
        method: request.then(|| "POST".to_string()),
        host: Some("api.deepseek.com".to_string()),
        path: Some("/v1/chat/completions".to_string()),
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

/// One stream: Open, one Segment, End(Complete).
pub fn stream(
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
            observed: at(producer, 900),
            reason: EndReason::Complete,
        },
    ]
}

/// A producer's records followed by the daemon's attestation that it is closed.
pub fn epoch(producer: &str, mut records: Vec<CaptureRecord>) -> Vec<CaptureRecord> {
    let final_seq = records.len() as u64 - 1;
    records.push(CaptureRecord::ProducerClosed {
        producer: producer.to_string(),
        observed: at(DAEMON, 950),
        final_seq: Some(final_seq),
    });
    records
}

pub fn goal() -> Vec<CaptureRecord> {
    epoch(
        GOAL,
        stream(
            GOAL,
            1,
            CaptureSource::Goal,
            meta("goal"),
            b"write result.txt",
        ),
    )
}

/// The `Launch` of a trace, with the two S1 fields as the case wants them.
pub fn launch(
    profile: &str,
    run_public_key: Option<&str>,
    connector_dir_sha256: Option<&str>,
) -> CaptureRecord {
    CaptureRecord::Launch {
        observed: at(DAEMON, 5),
        goal_stream: key(GOAL, 1),
        profile_id: profile.to_string(),
        profile_digest: "cc".repeat(32),
        harness_version: "0.153.0".to_string(),
        recorder_revision: "v5-corpus".to_string(),
        continuation: "none".to_string(),
        allowed_tool_rule: None,
        process_handoff_digest: None,
        run_public_key: run_public_key.map(str::to_string),
        connector_dir_sha256: connector_dir_sha256.map(str::to_string),
    }
}

/// One complete chat-completions exchange on one connection, and an unhinted native snapshot:
/// the shape of the Hermes reference row.
pub fn hermes_run() -> Vec<CaptureRecord> {
    let mut wire = vec![CaptureRecord::Connection {
        producer: WIRE.to_string(),
        connection: CONN.to_string(),
        observed: at(WIRE, 2),
        event: ConnectionEvent::Open {
            alpn: Some("http/1.1".to_string()),
            already_open: false,
        },
    }];
    wire.extend(stream(
        WIRE,
        1,
        CaptureSource::WireRequest,
        wire_meta(CaptureSource::WireRequest, 1),
        br#"{"model":"deepseek-v4-flash","stream":true,"messages":[{"role":"user","content":"write result.txt"}]}"#,
    ));
    wire.extend(stream(
        WIRE,
        2,
        CaptureSource::WireResponse,
        wire_meta(CaptureSource::WireResponse, 1),
        b"data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n",
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
        stream(
            NATIVE,
            1,
            CaptureSource::Native,
            meta(&format!("session:session_{HERMES_SESSION}.json#g1")),
            format!(r#"{{"session_id":"{HERMES_SESSION}","messages":[]}}"#).as_bytes(),
        ),
    ));
    records
}

pub fn manifest() -> SealManifest {
    SealManifest {
        entry_id: ENTRY.to_string(),
        sealed_at: "2026-10-02T12:00:00Z".to_string(),
        workspace: "/ws".to_string(),
        files: vec![],
        excluded: vec![],
    }
}

/// Append one record through the ordinary writer.
pub fn append_record(export: &mut TraceExport, record: &CaptureRecord) -> Link {
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

pub fn confirmed() -> TerminationOutcomeV1 {
    TerminationOutcomeV1::Confirmed {
        requested_at: utc("2026-10-02T12:00:02Z"),
        confirmed_at: utc("2026-10-02T12:00:03.125Z"),
        method: TerminationMethodV1::ProcessSignal,
    }
}

/// The local finish: the seal, bound by a `LocalFinish` record.
pub fn local_finish(
    export: &mut TraceExport,
    manifest: &SealManifest,
    termination: TerminationOutcomeV1,
) -> Link {
    export
        .write_local_finish(
            manifest,
            CaptureTime {
                wall: "2026-10-02T12:00:05.500Z".to_string(),
                mono_ns: 990,
                clock_id: DAEMON.to_string(),
            },
            "participant_stop",
            termination,
        )
        .unwrap()
        .1
}

/// A trace directory: baseline for `manifest`, then `records`, then whatever `finish` does
/// (nothing, a local finish, or a receipt, acks and a signed finalization).
pub fn build_trace(
    dir: &Path,
    manifest: &SealManifest,
    records: &[CaptureRecord],
    finish: impl FnOnce(&mut TraceExport),
) {
    let mut export = TraceExport::create(dir).unwrap();
    export.configure_capture_quota(1 << 40).unwrap();
    export
        .write_baseline_for_protocol(M2_PROTOCOL_VERSION, manifest)
        .unwrap();
    for record in records {
        append_record(&mut export, record);
    }
    finish(&mut export);
}

pub fn write_trace(dir: &Path, records: &[CaptureRecord], sealed: bool) {
    build_trace(dir, &manifest(), records, |export| {
        if sealed {
            local_finish(export, &manifest(), confirmed());
        }
    });
}

/// The seed of the synthetic signer of receipts and acknowledgements: a public test key,
/// neither the placeholder seed nor any real anchor.
pub const SERVER_SEED: [u8; 32] = [0x42; 32];

pub fn server_key() -> SigningKey {
    SigningKey::from_bytes(&SERVER_SEED)
}

pub fn server_public_key_hex() -> String {
    hex::encode(server_key().verifying_key().to_bytes())
}

/// A signed acknowledgement of `link`.
pub fn sign_ack(link: &Link) -> AckCheckpoint {
    let mut ack = AckCheckpoint {
        entry_id: ENTRY.to_string(),
        seq: link.seq,
        hash: link.hash.clone(),
        acked_at: "2026-10-02T12:01:00Z".to_string(),
        server_signature: String::new(),
    };
    ack.server_signature = hex::encode(server_key().sign(&ack.signing_bytes().unwrap()).to_bytes());
    ack
}

pub const STARTED_AT: &str = "2026-10-02T11:59:30Z";
pub const DEADLINE_AT: &str = "2026-10-02T12:30:00Z";

/// A start receipt over the chain's current head, written beside it. Returns its digest.
pub fn write_receipt(export: &mut TraceExport, run_public_key: Option<&str>) -> Sha256Digest {
    let (_, head) = export.head().unwrap();
    let receipt = StartReceipt::sign(
        StartReceiptClaims {
            entry_id: ENTRY.to_string(),
            connector_id: "codex-cli".to_string(),
            workspace: "/ws".to_string(),
            session_id: "session-corpus".to_string(),
            started_at: STARTED_AT.to_string(),
            deadline_at: DEADLINE_AT.to_string(),
            manifest_sha256: "11".repeat(32),
            chain_head: head,
            connector_dir_sha256: None,
            run_public_key: run_public_key.map(str::to_string),
            questionnaire_version: None,
        },
        &server_key(),
    )
    .unwrap();
    export.write_receipt(&receipt).unwrap();
    receipt.canonical_digest().unwrap()
}

/// The signed terminal: a cutoff and a finalization that cite the receipt.
pub fn signed_finalization(export: &mut TraceExport, receipt: &Sha256Digest) -> Link {
    export
        .write_cutoff_for_protocol(
            M2_PROTOCOL_VERSION,
            ENTRY,
            "2026-10-02T12:00:08Z",
            CutoffReachedV1 {
                receipt_sha256: receipt.clone(),
                deadline_at: utc(DEADLINE_AT),
                pre_cutoff_drain: PreCutoffDrainOutcomeV1::Complete,
            },
        )
        .unwrap();
    export
        .write_finalization_for_protocol(
            M2_PROTOCOL_VERSION,
            ENTRY,
            "2026-10-02T12:00:09Z",
            FinalizationV1::CaptureUnavailable {
                receipt_sha256: receipt.clone(),
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
        .unwrap()
}

/// A second wire producer, for the recovery case.
pub const WIRE_B: &str = "fedcba9876543210fedcba9876543210";
pub const CONN_B: &str = "c3d4e5f60718293a4b5c6d7e8f90a1b2";
/// The producer of a WebSocket session.
pub const WS: &str = "13579bdf02468ace13579bdf02468ace";
pub const CONN_WS: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f0";

fn connection(
    producer: &str,
    connection: &str,
    tick: u64,
    event: ConnectionEvent,
) -> CaptureRecord {
    CaptureRecord::Connection {
        producer: producer.to_string(),
        connection: connection.to_string(),
        observed: at(producer, tick),
        event,
    }
}

/// A WebSocket session: the upgrade, one client and one server message as one stream each,
/// then the close, all on the WebSocket producer.
pub fn websocket_run() -> Vec<CaptureRecord> {
    let message = |id: u64, source: CaptureSource, bytes: &[u8]| {
        let mut metadata = wire_meta(source, 1);
        metadata.wire = Some(WireIdentity {
            connection: CONN_WS.to_string(),
            exchange: 1,
            message_index: Some(id),
        });
        metadata.method = None;
        metadata.status = None;
        metadata.label = format!("ws message {id}");
        stream(WS, id, source, metadata, bytes)
    };
    let mut records = vec![
        connection(
            WS,
            CONN_WS,
            2,
            ConnectionEvent::Open {
                alpn: Some("http/1.1".to_string()),
                already_open: false,
            },
        ),
        connection(WS, CONN_WS, 3, ConnectionEvent::UpgradedWebSocket),
    ];
    records.extend(message(
        1,
        CaptureSource::WsClient,
        br#"{"type":"response.create"}"#,
    ));
    records.extend(message(
        2,
        CaptureSource::WsServer,
        br#"{"type":"response.completed"}"#,
    ));
    records.push(connection(
        WS,
        CONN_WS,
        4,
        ConnectionEvent::Close {
            outcome: ConnectionOutcome::Complete,
        },
    ));
    epoch(WS, records)
}

/// A wire producer lost mid-exchange (its request stream never ends and it is never closed),
/// replaced by a fresh producer that carries one whole exchange: a `Recovery`.
pub fn recovered_wire_run() -> Vec<CaptureRecord> {
    let mut lost = vec![connection(
        WIRE,
        CONN,
        2,
        ConnectionEvent::Open {
            alpn: Some("http/1.1".to_string()),
            already_open: false,
        },
    )];
    lost.extend(stream(
        WIRE,
        1,
        CaptureSource::WireRequest,
        wire_meta(CaptureSource::WireRequest, 1),
        b"{\"model\":\"m\"}",
    ));
    lost.truncate(lost.len() - 1); // no End: the producer was lost before it
    let mut records = lost;
    records.push(CaptureRecord::Recovery {
        previous_producer: WIRE.to_string(),
        producer: WIRE_B.to_string(),
        observed: at(WIRE_B, 1),
        reason: GapReason::IngestLost,
    });
    let mut fresh = vec![connection(
        WIRE_B,
        CONN_B,
        2,
        ConnectionEvent::Open {
            alpn: Some("http/1.1".to_string()),
            already_open: false,
        },
    )];
    fresh.extend(stream(
        WIRE_B,
        1,
        CaptureSource::WireRequest,
        wire_meta(CaptureSource::WireRequest, 1),
        b"{\"model\":\"m\"}",
    ));
    fresh.push(connection(
        WIRE_B,
        CONN_B,
        3,
        ConnectionEvent::Close {
            outcome: ConnectionOutcome::Complete,
        },
    ));
    records.extend(epoch(WIRE_B, fresh));
    records
}

/// A stream-less gap and an `Assistance` record.
pub fn gap_and_assistance() -> Vec<CaptureRecord> {
    vec![
        CaptureRecord::Gap {
            stream: None,
            observed: at(DAEMON, 6),
            reason: GapReason::QueueFull,
            lost_bytes: None,
        },
        CaptureRecord::Assistance {
            observed: at(DAEMON, 7),
            action: "manual_resume".to_string(),
        },
    ]
}

/// One wire request stream that ends as `reason`, on its own producer.
pub fn wire_stream_ending(reason: EndReason) -> Vec<CaptureRecord> {
    let mut records = vec![connection(
        WIRE,
        CONN,
        2,
        ConnectionEvent::Open {
            alpn: Some("http/1.1".to_string()),
            already_open: false,
        },
    )];
    let mut one = stream(
        WIRE,
        1,
        CaptureSource::WireRequest,
        wire_meta(CaptureSource::WireRequest, 1),
        b"{\"model\":\"m\"}",
    );
    if let Some(CaptureRecord::End { reason: r, .. }) = one.last_mut() {
        *r = reason;
    }
    records.extend(one);
    records.push(connection(
        WIRE,
        CONN,
        3,
        ConnectionEvent::Close {
            outcome: ConnectionOutcome::Complete,
        },
    ));
    epoch(WIRE, records)
}

/// A Claude `Launch` carrying the paired allowance claim.
pub fn claude_launch_with_allowance() -> CaptureRecord {
    let mut launch = launch("claude-code-cli", None, None);
    if let CaptureRecord::Launch {
        allowed_tool_rule,
        process_handoff_digest,
        ..
    } = &mut launch
    {
        *allowed_tool_rule = Some("Bash(python3 demo.py *)".to_string());
        *process_handoff_digest = Some("dd".repeat(32));
    }
    launch
}

/// A manifest that lists sealed files and an exclusion.
pub fn manifest_with_files() -> SealManifest {
    SealManifest {
        files: vec![
            SealedFile {
                path: "src/a.rs".to_string(),
                bytes: 12,
                sha256: "ab".repeat(32),
            },
            SealedFile {
                path: "src/z.rs".to_string(),
                bytes: 3,
                sha256: "cd".repeat(32),
            },
        ],
        excluded: vec!["target".to_string()],
        ..manifest()
    }
}
