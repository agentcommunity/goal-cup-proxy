//! The bounded protocol-v5 capture-integrity reader and `goalcup-verify
//! --capture-integrity TRACE`.
//!
//! Every fixture is a real `TraceExport`: a v5 baseline, ordinary capture appends, and
//! either no terminal, `write_local_finish`, or the versioned signed cutoff/finalization
//! writers. Nothing here hand-appends a reserved body. Negative cases mutate a temporary
//! fixture only after its writer is closed (`append_bytes`, or a byte edit of one file), and
//! the expectations come from what was written — the chain read back independently, the
//! records the test itself appended — never from the reader under test.
//!
//! Canonical exits: 0 complete valid local evidence
//! (anchor may stay `local_only`), 1 incomplete or unsealed coverage, 2 invalid evidence
//! including a torn tail (which overrides unsealed), 3 no usable report.

use crate::common as support;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signer, SigningKey};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::capture::{
    CaptureMeta, CaptureRecord, CaptureSource, CaptureTime, ConnectionEvent, ConnectionOutcome,
    EndReason, GapReason, M2_PROTOCOL_VERSION, MAX_ENVELOPE_BYTES, MAX_OPEN_CONNECTIONS,
    MAX_OPEN_STREAMS, MAX_PRODUCERS, StreamKey, WireIdentity,
};
use goalcup_protocol::capture_status::AnchorState;
use goalcup_protocol::chain::Link;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::export::{
    ACKS_FILE, BASELINE_FILE, CHAIN_FILE, RECEIPT_FILE, SEAL_FILE, TraceExport, manifest_sha256,
    part_file_name,
};
use goalcup_protocol::provenance::{
    CaptureUnavailableV1, CutoffReachedV1, FinalizationV1, NoRepositoryReasonV1,
    PreCutoffDrainOutcomeV1, PublisherNotAttemptedReasonV1, PublisherNotAttemptedV1,
    RepositoryNonSelectionV1, Rfc3339Utc, Sha256Digest, SnapshotNotAttemptedV1,
    TerminationMethodV1, TerminationOutcomeV1,
};
use goalcup_protocol::receipt::{StartReceipt, StartReceiptClaims};
use goalcup_protocol::redaction::{RedactionAudit, RedactionAuditSpan};
use goalcup_protocol::seal::{SealManifest, SealedFile};
use goalcup_verifier::VerifyOptions;
use goalcup_verifier::capture_analysis::api::{
    DecoderFailure, DecoderSummary, DecoderV1, Disposition, Observation, StreamOpen,
};
use goalcup_verifier::capture_analysis::reader::{
    CaptureIntegrity, CaptureIntegrityReport, CaptureTerminalFacts, CaptureTerminalKind, KeySource,
    VerifiedAckPosition, VerifiedReceiptClock, capture_integrity_document, scan_capture_trace,
};
use goalcup_verifier::capture_analysis::registry::{DecoderRegistry, Registered};
use goalcup_verifier::capture_analysis::session::decode_records;
use sha2::{Digest, Sha256};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use support::records::key;

const ENTRY: &str = "entry-m2-t18";
/// Producer epochs: fresh 128-bit identifiers.
const WIRE: &str = "0123456789abcdef0123456789abcdef";
const RECOVERED: &str = "fedcba9876543210fedcba9876543210";
const NATIVE: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90";
const GOAL: &str = "9d8c7b6a5f4e3d2c1b0a998877665544";
/// A producer that was admitted and whose Recovery was answered `Queued`, but of which
/// nothing was ever persisted (an epoch lost in flight).
const IN_FLIGHT: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f0";
/// A second wire producer, for the negative control.
const OTHER: &str = "13579bdf02468ace13579bdf02468ace";
const CONN: &str = "5e1f0a2b3c4d5e6f708192a3b4c5d6e7";
const OTHER_CONN: &str = "c3d4e5f60718293a4b5c6d7e8f90a1b2";
const DAEMON_CLOCK: &str = "daemon-t18-clock";
/// A quota far above anything these fixtures write: admission is not under test.
const QUOTA: u64 = 1 << 40;
const STARTED_AT: &str = "2026-09-27T11:59:30Z";
const DEADLINE_AT: &str = "2026-09-27T12:30:00Z";
const REQUEST_BODY: &[u8] = b"{\"input\":\"hi\"}";
const REQUEST_MORE: &[u8] = b"{\"more\":true}";

// ---------------------------------------------------------------------------------------
// Fixture helpers. The record and trace builders live here; trace-directory copying
// (`materialize_private`) is shared through tests/common.
// ---------------------------------------------------------------------------------------

fn at(clock: &str, mono_ns: u64) -> CaptureTime {
    CaptureTime {
        wall: "2026-09-27T12:00:01Z".to_string(),
        mono_ns,
        clock_id: clock.to_string(),
    }
}

fn finish_time() -> CaptureTime {
    CaptureTime {
        wall: "2026-09-27T12:00:05.500Z".to_string(),
        mono_ns: 900,
        clock_id: DAEMON_CLOCK.to_string(),
    }
}

fn utc(value: &str) -> Rfc3339Utc {
    Rfc3339Utc::try_from(value).unwrap()
}

fn manifest() -> SealManifest {
    SealManifest {
        entry_id: ENTRY.to_string(),
        sealed_at: "2026-09-27T12:00:00Z".to_string(),
        workspace: "/ws".to_string(),
        files: vec![
            SealedFile {
                path: "src/z.rs".to_string(),
                bytes: 3,
                sha256: "cd".repeat(32),
            },
            SealedFile {
                path: "src/a.rs".to_string(),
                bytes: 12,
                sha256: "ab".repeat(32),
            },
        ],
        excluded: vec!["target".to_string()],
        git: None,
        home: None,
    }
}

fn confirmed() -> TerminationOutcomeV1 {
    TerminationOutcomeV1::Confirmed {
        requested_at: utc("2026-09-27T12:00:02Z"),
        confirmed_at: utc("2026-09-27T12:00:03.125Z"),
        method: TerminationMethodV1::ProcessSignal,
    }
}

fn conn(producer: &str, event: ConnectionEvent) -> CaptureRecord {
    CaptureRecord::Connection {
        producer: producer.to_string(),
        connection: CONN.to_string(),
        observed: at(producer, 5),
        event,
    }
}

fn conn_open(producer: &str, already_open: bool) -> CaptureRecord {
    conn(
        producer,
        ConnectionEvent::Open {
            alpn: Some("http/1.1".to_string()),
            already_open,
        },
    )
}

fn conn_close(producer: &str) -> CaptureRecord {
    conn(
        producer,
        ConnectionEvent::Close {
            outcome: ConnectionOutcome::Complete,
        },
    )
}

fn open_wire(producer: &str, stream: u64, exchange: u64, hint: Option<&str>) -> CaptureRecord {
    CaptureRecord::Open {
        stream: key(producer, stream),
        observed: at(producer, 10),
        source: CaptureSource::WireRequest,
        metadata: CaptureMeta {
            wire: Some(WireIdentity {
                connection: CONN.to_string(),
                exchange,
                message_index: None,
            }),
            label: "POST /v1/responses".to_string(),
            protocol_hint: hint.map(str::to_string),
            method: Some("POST".to_string()),
            host: Some("api.example.test".to_string()),
            path: Some("/v1/responses".to_string()),
            status: None,
            content_type: Some("application/json".to_string()),
        },
    }
}

fn open_native(producer: &str, stream: u64) -> CaptureRecord {
    CaptureRecord::Open {
        stream: key(producer, stream),
        observed: at(producer, 10),
        source: CaptureSource::Native,
        metadata: CaptureMeta {
            wire: None,
            label: format!("sessions/rollout.jsonl#{stream}"),
            protocol_hint: Some("codex-jsonl".to_string()),
            method: None,
            host: None,
            path: None,
            status: None,
            content_type: None,
        },
    }
}

fn segment(producer: &str, stream: u64, part: u64, offset: u64, bytes: &[u8]) -> CaptureRecord {
    CaptureRecord::Segment {
        stream: key(producer, stream),
        part,
        source_offset: offset,
        source_len: bytes.len() as u64,
        first: at(producer, 20 + part),
        last: at(producer, 21 + part),
        data_b64: STANDARD.encode(bytes),
        audit: RedactionAudit { spans: vec![] },
    }
}

fn end(producer: &str, stream: u64) -> CaptureRecord {
    CaptureRecord::End {
        stream: key(producer, stream),
        observed: at(producer, 90),
        reason: EndReason::Complete,
    }
}

fn gap(stream: Option<StreamKey>) -> CaptureRecord {
    CaptureRecord::Gap {
        stream,
        observed: at(DAEMON_CLOCK, 50),
        reason: GapReason::QueueFull,
        lost_bytes: None,
    }
}

fn recovery_of(previous: &str, producer: &str) -> CaptureRecord {
    CaptureRecord::Recovery {
        previous_producer: previous.to_string(),
        producer: producer.to_string(),
        observed: at(producer, 1),
        reason: GapReason::IngestLost,
    }
}

fn closed(producer: &str, final_seq: Option<u64>) -> CaptureRecord {
    CaptureRecord::ProducerClosed {
        producer: producer.to_string(),
        observed: at(DAEMON_CLOCK, 99),
        final_seq,
    }
}

fn open_goal(stream: u64) -> CaptureRecord {
    CaptureRecord::Open {
        stream: key(GOAL, stream),
        observed: at(GOAL, 10),
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
    }
}

/// The goal producer's one complete stream: Open 0, Bytes 1, End 2, so `final_seq` is 2.
fn goal_stream() -> Vec<CaptureRecord> {
    vec![
        open_goal(1),
        segment(GOAL, 1, 0, 0, b"reach the goal"),
        end(GOAL, 1),
        closed(GOAL, Some(2)),
    ]
}

/// `records` behind the goal stream and the Launch naming it: a complete trace needs a
/// Launch bound to a complete goal stream.
fn launched(records: Vec<CaptureRecord>) -> Vec<CaptureRecord> {
    let mut all = goal_stream();
    all.push(launch());
    all.extend(records);
    all
}

fn launch() -> CaptureRecord {
    launch_naming(key(GOAL, 1))
}

fn launch_naming(goal_stream: StreamKey) -> CaptureRecord {
    CaptureRecord::Launch {
        observed: CaptureTime {
            wall: "2026-09-27T12:00:00.750Z".to_string(),
            mono_ns: 7,
            clock_id: DAEMON_CLOCK.to_string(),
        },
        goal_stream,
        profile_id: "codex-cli".to_string(),
        profile_digest: "cc".repeat(32),
        harness_version: "0.153.0".to_string(),
        recorder_revision: "t18-fixture".to_string(),
        continuation: "none".to_string(),
        allowed_tool_rule: None,
        process_handoff_digest: None,
        run_public_key: None,
        connector_dir_sha256: None,
        harness_build: None,
    }
}

/// One complete HTTP exchange on one connection, closed by its producer. The producer's own
/// frame sequence is: connection open 0, stream open 1, two Bytes frames 2–3, end 4,
/// connection close 5 — so the receiver-attested `final_seq` is 5.
fn complete_wire(hint: Option<&str>) -> Vec<CaptureRecord> {
    vec![
        conn_open(WIRE, false),
        open_wire(WIRE, 1, 1, hint),
        segment(WIRE, 1, 0, 0, REQUEST_BODY),
        segment(WIRE, 1, 1, REQUEST_BODY.len() as u64, REQUEST_MORE),
        end(WIRE, 1),
        conn_close(WIRE),
        closed(WIRE, Some(5)),
    ]
}

fn v5(record: CaptureRecord) -> Envelope {
    Envelope {
        protocol_version: M2_PROTOCOL_VERSION,
        entry_id: ENTRY.to_string(),
        source: "daemon".to_string(),
        recorded_at: "2026-09-27T12:00:01Z".to_string(),
        link: Link {
            seq: 0,
            prev: String::new(),
            hash: String::new(),
        },
        body: EventBody::Capture { record },
        redaction: None,
    }
}

fn capture_export(root: &Path) -> TraceExport {
    let mut export = TraceExport::create(root).unwrap();
    export.configure_capture_quota(QUOTA).unwrap();
    export
        .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest())
        .unwrap();
    export
}

fn append(export: &mut TraceExport, record: &CaptureRecord) -> Link {
    export.append_chain(&mut v5(record.clone())).unwrap()
}

/// A real v5 trace: baseline, `records` through the ordinary append, and — when
/// `sealed` — the local finish, with no server, receipt or ACK.
fn make_trace(records: &[CaptureRecord], sealed: bool) -> tempfile::TempDir {
    make_trace_finishing_at(records, sealed.then(finish_time))
}

fn make_trace_finishing_at(
    records: &[CaptureRecord],
    finish: Option<CaptureTime>,
) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let mut export = capture_export(dir.path());
    for record in records {
        append(&mut export, record);
    }
    if let Some(observed) = finish {
        export
            .write_local_finish(&manifest(), observed, "participant_stop", confirmed())
            .unwrap();
    }
    dir
}

/// Append raw bytes to one file of a fixture whose writer is already closed.
fn append_bytes(path: &Path, bytes: &[u8]) {
    std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(bytes)
        .unwrap();
}

/// The chain read back independently of the reader under test.
fn chain_lines(dir: &Path) -> Vec<Envelope> {
    std::fs::read_to_string(dir.join(CHAIN_FILE))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn chain_len(dir: &Path) -> u64 {
    std::fs::metadata(dir.join(CHAIN_FILE)).unwrap().len()
}

fn capture_records(chain: &[Envelope]) -> Vec<CaptureRecord> {
    chain
        .iter()
        .filter_map(|env| match &env.body {
            EventBody::Capture { record } => Some(record.clone()),
            _ => None,
        })
        .collect()
}

/// The real public reader; the callback keeps only this small test's records.
fn scan_with(dir: &Path, options: &VerifyOptions) -> (CaptureIntegrityReport, Vec<CaptureRecord>) {
    let mut seen = Vec::new();
    let report = scan_capture_trace(dir, options, &mut |record| {
        seen.push(record.clone());
        Ok(())
    })
    .unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    (report, seen)
}

fn scan(dir: &Path) -> (CaptureIntegrityReport, Vec<CaptureRecord>) {
    scan_with(dir, &VerifyOptions::default())
}

fn has(report: &CaptureIntegrityReport, code: &str) -> bool {
    report.findings.iter().any(|f| f == code)
}

const LINEAGE_HINT: &str = "lineage-probe";

#[derive(Default)]
struct LineageProbe {
    segments: u64,
}

impl DecoderV1 for LineageProbe {
    fn id(&self) -> &'static str {
        LINEAGE_HINT
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn accepts(&self, open: &StreamOpen<'_>) -> bool {
        open.protocol_hint == Some(LINEAGE_HINT)
    }

    fn push(
        &mut self,
        record: &CaptureRecord,
        _: &mut dyn FnMut(Observation),
    ) -> Result<(), DecoderFailure> {
        self.segments += u64::from(matches!(record, CaptureRecord::Segment { .. }));
        Ok(())
    }

    fn push_recovery(
        &mut self,
        record: &CaptureRecord,
        _: &[StreamKey],
        emit: &mut dyn FnMut(Observation),
    ) -> Result<(), DecoderFailure> {
        self.push(record, emit)
    }

    fn finish(
        self: Box<Self>,
        _: &mut dyn FnMut(Observation),
    ) -> Result<DecoderSummary, DecoderFailure> {
        Ok(DecoderSummary::new(self.segments, 0, false))
    }
}

fn probe_open(mut record: CaptureRecord) -> CaptureRecord {
    let CaptureRecord::Open { metadata, .. } = &mut record else {
        panic!("probe_open requires an Open")
    };
    metadata.protocol_hint = Some(LINEAGE_HINT.to_string());
    record
}

fn decoded_segments(records: &[CaptureRecord]) -> u64 {
    let mut registry = DecoderRegistry::empty();
    registry
        .register(Registered::new(
            || Box::new(LineageProbe::default()),
            "lineage parity",
            &[],
        ))
        .unwrap();
    let (outcome, _) = decode_records(records.iter().cloned().map(Ok), &registry, |_, _| {
        Ok(Disposition::Kept)
    })
    .unwrap();
    outcome.decoders[0].result.as_ref().unwrap().observations
}

fn server_key() -> SigningKey {
    // A public, test-only key the verifier never trusts.
    SigningKey::from_bytes(&[0x42; 32])
}

fn supplied(key: &SigningKey) -> VerifyOptions {
    VerifyOptions {
        extra_server_keys: vec![hex::encode(key.verifying_key().to_bytes())],
        ..VerifyOptions::default()
    }
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

fn receipt_digest_finalization(digest: &Sha256Digest) -> FinalizationV1 {
    FinalizationV1::CaptureUnavailable {
        receipt_sha256: digest.clone(),
        termination: confirmed(),
        capture: CaptureUnavailableV1::NotAttempted(SnapshotNotAttemptedV1 {
            selection: RepositoryNonSelectionV1::None {
                reason: NoRepositoryReasonV1::NotFound,
            },
        }),
        publisher: PublisherNotAttemptedV1 {
            reason: PublisherNotAttemptedReasonV1::SelectionFailed,
        },
    }
}

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_goalcup-verify"))
}

// ---------------------------------------------------------------------------------------
// Named cases.
// ---------------------------------------------------------------------------------------

#[test]
fn valid_unknown_protocol_keeps_integrity() {
    let records = launched(complete_wire(Some("x-unknown-protocol/9")));
    let dir = make_trace(&records, true);
    let (report, seen) = scan(dir.path());

    assert_eq!(report.exit_code(), 0, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Valid);
    assert_eq!(report.anchor, AnchorState::LocalOnly);
    assert!(report.sealed);
    assert!(!report.coverage_incomplete);
    assert!(!report.torn_tail);
    assert_eq!(report.gap_count, 0);
    assert_eq!(report.schema_version, 1);

    // Valid hashes: the validated prefix is the whole chain, as read back independently.
    let chain = chain_lines(dir.path());
    assert_eq!(report.validated_records, chain.len() as u64);
    assert_eq!(report.validated_bytes, chain_len(dir.path()));
    assert_eq!(
        report.validated_head.as_deref(),
        Some(chain.last().unwrap().link.hash.as_str())
    );

    // The callback saw exactly the retained capture records, in source order, both parts of
    // the unknown-protocol stream included, and the local finish last.
    assert_eq!(seen, capture_records(&chain));
    assert_eq!(&seen[..records.len()], &records[..]);
    let parts: Vec<u64> = seen
        .iter()
        .filter_map(|r| match r {
            CaptureRecord::Segment { stream, part, .. } if stream.producer == WIRE => Some(*part),
            _ => None,
        })
        .collect();
    assert_eq!(parts, vec![0, 1]);
    assert!(matches!(
        seen.last(),
        Some(CaptureRecord::LocalFinish { .. })
    ));
}

#[test]
fn oversized_line_refused_before_unbounded_allocation() {
    let records = complete_wire(None);

    // A newline-terminated line one byte over the bound.
    let dir = make_trace(&records, false);
    let valid_records = chain_lines(dir.path()).len() as u64;
    let valid_bytes = chain_len(dir.path());
    let mut line = vec![b'x'; MAX_ENVELOPE_BYTES];
    line.push(b'\n');
    append_bytes(&dir.path().join(CHAIN_FILE), &line);
    let (report, seen) = scan(dir.path());
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Invalid);
    assert!(
        !report.torn_tail,
        "an oversized line is refused, not read as a torn tail"
    );
    assert!(has(&report, "envelope_too_large"), "{:?}", report.findings);
    assert_eq!(report.validated_records, valid_records);
    assert_eq!(report.validated_bytes, valid_bytes);
    assert_eq!(seen, records);

    // A sparse 16 GiB line with no newline at all: a reader that buffered the line before
    // bounding it would try to hold 16 GiB. The bounded reader stops at the bound.
    let dir = make_trace(&records, false);
    let chain = dir.path().join(CHAIN_FILE);
    let file = std::fs::OpenOptions::new()
        .append(true)
        .open(&chain)
        .unwrap();
    (&file)
        .write_all(b"{\"protocol_version\":5,\"entry_id\":\"")
        .unwrap();
    file.set_len(valid_bytes + (16u64 << 30)).unwrap();
    drop(file);
    let started = std::time::Instant::now();
    let (report, _) = scan(dir.path());
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "the reader must stop at the envelope bound, not read the line"
    );
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert!(has(&report, "envelope_too_large"), "{:?}", report.findings);
    assert_eq!(report.validated_records, valid_records);
}

#[test]
fn torn_tail_preserves_validated_prefix_and_fails() {
    let records = vec![conn_open(WIRE, false), open_wire(WIRE, 1, 1, None)];

    let dir = make_trace(&records, false);
    let chain = chain_lines(dir.path());
    assert_eq!(chain.len(), 3, "baseline and two valid records");
    let prefix_bytes = chain_len(dir.path());
    append_bytes(
        &dir.path().join(CHAIN_FILE),
        b"{\"protocol_version\":5,\"entry_id\":\"entry-m2-t18\",\"sou",
    );
    let (report, seen) = scan(dir.path());
    // Torn is invalid evidence and exits 2, overriding the unsealed exit 1.
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert!(report.torn_tail);
    assert_eq!(report.integrity, CaptureIntegrity::Invalid);
    assert!(!report.sealed);
    assert_eq!(report.terminal, None);
    assert!(has(&report, "torn_tail"), "{:?}", report.findings);
    assert_eq!(report.validated_records, 3);
    assert_eq!(report.validated_bytes, prefix_bytes);
    assert_eq!(
        report.validated_head.as_deref(),
        Some(chain[2].link.hash.as_str())
    );
    assert_eq!(seen, records, "the validated prefix is still delivered");

    // A complete, correctly linked next record that lacks only its newline is torn too.
    let dir = make_trace(&records, false);
    let last = chain_lines(dir.path()).pop().unwrap();
    let mut next = v5(gap(None));
    next.link = Link {
        seq: last.link.seq + 1,
        prev: last.link.hash.clone(),
        hash: String::new(),
    };
    next.link.hash = next.expected_hash(&last.link.hash).unwrap();
    append_bytes(
        &dir.path().join(CHAIN_FILE),
        &serde_json::to_vec(&next).unwrap(),
    );
    let (report, _) = scan(dir.path());
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert!(report.torn_tail);
    assert_eq!(report.validated_records, 3);
    assert_eq!(report.gap_count, 0, "the torn record is not counted");
}

#[test]
fn terminated_malformed_interior_record_fails() {
    let records = complete_wire(None);
    let dir = make_trace(&records, true);
    let path = dir.path().join(CHAIN_FILE);
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut rebuilt = String::new();
    rebuilt.push_str(lines[0]);
    rebuilt.push_str(lines[1]);
    rebuilt.push_str("{\"protocol_version\":5,\"seq\":2,\"body\":{\n");
    for line in &lines[2..] {
        rebuilt.push_str(line);
    }
    std::fs::write(&path, rebuilt).unwrap();

    let (report, seen) = scan(dir.path());
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Invalid);
    assert!(
        !report.torn_tail,
        "a terminated malformed record is not a torn tail"
    );
    assert!(has(&report, "record_malformed"), "{:?}", report.findings);
    assert_eq!(report.validated_records, 2);
    assert_eq!(seen, records[..1].to_vec());
    assert!(
        !report.sealed,
        "a valid suffix after the break proves nothing"
    );
    assert_eq!(report.terminal, None);
}

#[test]
fn noncanonical_v5_line_is_invalid() {
    let build = || make_trace(&launched(complete_wire(None)), true);
    assert_eq!(scan(build().path()).0.exit_code(), 0, "control");

    // Bytes the link hash does not cover: an unknown key, or a CRLF line end.
    for (name, from, to) in [("unknown key", "{", "{\"x\":1,"), ("crlf", "\n", "\r\n")] {
        let dir = build();
        let path = dir.path().join(CHAIN_FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
        lines[3] = lines[3].replacen(from, to, 1);
        std::fs::write(&path, lines.concat()).unwrap();
        let (report, _) = scan(dir.path());
        assert_eq!(report.exit_code(), 2, "{name}: {report:#?}");
        assert!(
            has(&report, "record_not_canonical"),
            "{name}: {:?}",
            report.findings
        );
        assert_eq!(report.validated_records, 3, "{name}");
    }
}

#[test]
fn hash_or_signature_tamper_fails() {
    let key = server_key();
    let options = supplied(&key);

    // (a) The sanitized bytes of one segment, changed to other valid base64 of equal length.
    let dir = make_trace(&launched(complete_wire(None)), true);
    assert_eq!(scan(dir.path()).0.exit_code(), 0, "control");
    let path = dir.path().join(CHAIN_FILE);
    let text = std::fs::read_to_string(&path).unwrap();
    let original = STANDARD.encode(REQUEST_BODY);
    let forged = STANDARD.encode(b"{\"input\":\"HI\"}");
    assert_eq!(original.len(), forged.len());
    assert_eq!(text.matches(&original).count(), 1);
    std::fs::write(&path, text.replacen(&original, &forged, 1)).unwrap();
    let (report, seen) = scan(dir.path());
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert!(has(&report, "link_hash_mismatch"), "{:?}", report.findings);
    // baseline, goal and launch, connection open, stream open; the segment is the first
    // unverifiable line.
    assert_eq!(report.validated_records, 1 + 5 + 2);
    assert_eq!(seen.len(), 5 + 2);

    // (b) The server signature of an acknowledgement, independently of the chain: two ACKs,
    // one behind the terminal and one at it, each signed by the supplied key.
    let dir = tempfile::tempdir().unwrap();
    let mut export = capture_export(dir.path());
    let links: Vec<Link> = launched(complete_wire(None))
        .iter()
        .map(|record| append(&mut export, record))
        .collect();
    let (_, finish) = export
        .write_local_finish(&manifest(), finish_time(), "participant_stop", confirmed())
        .unwrap();
    for link in [&links[1], &finish] {
        export
            .append_ack_synced(&sign_ack(&key, link, "2026-09-27T12:01:00Z"))
            .unwrap();
    }
    drop(export);
    let (control, _) = scan_with(dir.path(), &options);
    assert_eq!(control.exit_code(), 0, "control: {control:#?}");
    assert_eq!(
        control.verified_ack.as_ref().map(|a| a.seq),
        Some(finish.seq)
    );

    let acks_path = dir.path().join(ACKS_FILE);
    let original_acks = std::fs::read_to_string(&acks_path).unwrap();
    let tamper = |which: usize| {
        let mut acks: Vec<AckCheckpoint> = original_acks
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let signature = &mut acks[which].server_signature;
        let flipped = if signature.starts_with('0') { "1" } else { "0" };
        signature.replace_range(0..1, flipped);
        let mut bytes = Vec::new();
        for ack in &acks {
            bytes.extend(serde_json::to_vec(ack).unwrap());
            bytes.push(b'\n');
        }
        std::fs::write(&acks_path, bytes).unwrap();
    };

    // The later ACK, after the earlier one fixed the anchoring key.
    tamper(1);
    let (report, _) = scan_with(dir.path(), &options);
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert!(
        has(&report, "ack_signature_invalid"),
        "{:?}",
        report.findings
    );
    assert_eq!(
        report.verified_ack.as_ref().map(|a| a.seq),
        Some(links[1].seq),
        "only the verified frontier is returned"
    );
    assert_eq!(
        report.validated_records, control.validated_records,
        "the chain itself still validates"
    );

    // The first ACK, which no accepted key then verifies.
    tamper(0);
    let (report, _) = scan_with(dir.path(), &options);
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert!(
        has(&report, "ack_signature_unverified"),
        "{:?}",
        report.findings
    );
    assert_eq!(report.verified_ack, None);
}

#[test]
fn local_terminal_without_receipt_or_ack_has_complete_local_integrity() {
    let dir = make_trace(&launched(complete_wire(None)), true);
    assert!(!dir.path().join(RECEIPT_FILE).exists());
    assert_eq!(std::fs::read(dir.path().join(ACKS_FILE)).unwrap(), b"");

    let (report, _) = scan(dir.path());
    assert_eq!(report.exit_code(), 0, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Valid);
    assert_eq!(report.anchor, AnchorState::LocalOnly);
    assert_eq!(report.key_source, KeySource::None);
    assert!(report.sealed);
    assert_eq!(report.receipt_clock, None, "no receipt clock is fabricated");
    assert_eq!(
        report.verified_ack, None,
        "no delivery position is fabricated"
    );

    let last = chain_lines(dir.path()).pop().unwrap();
    let seal_bytes = std::fs::read(dir.path().join(SEAL_FILE)).unwrap();
    let seal_digest = hex::encode(Sha256::digest(&seal_bytes));
    assert_eq!(seal_digest, manifest_sha256(&manifest()).unwrap());
    assert_eq!(
        report.terminal,
        Some(CaptureTerminalFacts {
            seq: last.link.seq,
            hash: last.link.hash.clone(),
            observed: Some(finish_time()),
            observed_at: finish_time().wall,
            kind: CaptureTerminalKind::LocalFinish,
            reason: Some("participant_stop".to_string()),
            termination: Some(confirmed()),
            seal_manifest_sha256: Some(seal_digest),
        })
    );
}

#[test]
fn local_terminal_binds_fixed_manifest() {
    let dir = make_trace(&complete_wire(None), true);
    let records = chain_lines(dir.path()).len() as u64;
    let mut changed = manifest().sorted();
    changed.files[0].sha256 = "ef".repeat(32);
    std::fs::write(
        dir.path().join(SEAL_FILE),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    let (report, _) = scan(dir.path());
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Invalid);
    assert!(
        has(&report, "seal_manifest_mismatch"),
        "{:?}",
        report.findings
    );
    assert!(!report.sealed);
    assert_eq!(report.terminal, None);
    assert_eq!(
        report.validated_records, records,
        "the chain itself is intact"
    );

    // A missing fixed manifest is an incomplete artifact: neither a pass nor a forgery.
    let dir = make_trace(&launched(complete_wire(None)), true);
    std::fs::remove_file(dir.path().join(SEAL_FILE)).unwrap();
    let (report, _) = scan(dir.path());
    assert_eq!(report.exit_code(), 1, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Incomplete);
    assert!(
        has(&report, "seal_manifest_missing"),
        "{:?}",
        report.findings
    );
    assert!(!report.sealed);
    assert_eq!(report.terminal, None);
}

#[test]
fn edited_or_missing_baseline_is_not_valid() {
    let build = || make_trace(&launched(complete_wire(None)), true);
    let rewrite = |dir: &Path, bytes: &[u8]| {
        std::fs::write(dir.join(BASELINE_FILE), bytes).unwrap();
        scan(dir).0
    };

    // The chain binds the manifest, not its layout: a re-indented copy still agrees.
    let dir = build();
    let report = rewrite(dir.path(), &serde_json::to_vec_pretty(&manifest()).unwrap());
    assert_eq!(report.exit_code(), 0, "{report:#?}");

    // One planted file more than the chained baseline counts.
    let mut edited = manifest();
    edited.files.push(SealedFile {
        path: "src/planted.rs".to_string(),
        bytes: 1,
        sha256: "ef".repeat(32),
    });
    let report = rewrite(dir.path(), &serde_json::to_vec(&edited).unwrap());
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Invalid);
    assert!(
        has(&report, "baseline_manifest_mismatch"),
        "{:?}",
        report.findings
    );
    let report = rewrite(dir.path(), b"{");
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert!(
        has(&report, "baseline_manifest_malformed"),
        "{:?}",
        report.findings
    );

    // Deleted or empty: a missing artifact, incomplete rather than forged.
    let report = rewrite(dir.path(), b"");
    assert_eq!(report.exit_code(), 1, "{report:#?}");
    assert!(
        has(&report, "baseline_manifest_missing"),
        "{:?}",
        report.findings
    );
    std::fs::remove_file(dir.path().join(BASELINE_FILE)).unwrap();
    let (report, _) = scan(dir.path());
    assert_eq!(report.exit_code(), 1, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Incomplete);
    assert!(
        has(&report, "baseline_manifest_missing"),
        "{:?}",
        report.findings
    );
    assert!(
        report.sealed,
        "the terminal is still bound to its own manifest"
    );
}

#[test]
fn verified_ack_progress_is_returned_without_claiming_terminal_delivery() {
    let key = server_key();
    let options = supplied(&key);
    let acked_at = "2026-09-27T12:01:00Z";

    // Two byte-identical chains; one ACK frontier behind the terminal, one at it.
    let build = |behind: bool| {
        let dir = tempfile::tempdir().unwrap();
        let mut export = capture_export(dir.path());
        let links: Vec<Link> = launched(complete_wire(None))
            .iter()
            .map(|record| append(&mut export, record))
            .collect();
        let (_, finish) = export
            .write_local_finish(&manifest(), finish_time(), "participant_stop", confirmed())
            .unwrap();
        let acked = if behind {
            links[1].clone()
        } else {
            finish.clone()
        };
        export
            .append_ack_synced(&sign_ack(&key, &acked, acked_at))
            .unwrap();
        (dir, acked, finish)
    };
    let (behind_dir, behind_link, finish) = build(true);
    let (caught_dir, caught_link, _) = build(false);
    assert_eq!(
        std::fs::read(behind_dir.path().join(CHAIN_FILE)).unwrap(),
        std::fs::read(caught_dir.path().join(CHAIN_FILE)).unwrap()
    );

    let (behind, _) = scan_with(behind_dir.path(), &options);
    let (caught, _) = scan_with(caught_dir.path(), &options);
    assert_eq!(behind.exit_code(), 0, "{behind:#?}");
    assert_eq!(caught.exit_code(), 0, "{caught:#?}");
    assert_eq!(
        behind.verified_ack,
        Some(VerifiedAckPosition {
            seq: behind_link.seq,
            hash: behind_link.hash.clone(),
            acked_at: acked_at.to_string(),
        })
    );
    assert_eq!(
        caught.verified_ack,
        Some(VerifiedAckPosition {
            seq: caught_link.seq,
            hash: caught_link.hash.clone(),
            acked_at: acked_at.to_string(),
        })
    );
    assert!(behind_link.seq < finish.seq, "a behind-terminal frontier");
    assert_eq!(caught_link, finish);

    // Only the verified delivery fact differs: the reader reports evidence, not a guessed
    // delivery state, and progress behind the terminal changes nothing else.
    let strip = |report: &CaptureIntegrityReport| {
        let mut value = serde_json::to_value(report).unwrap();
        value.as_object_mut().unwrap().remove("verified_ack");
        value
    };
    assert_eq!(strip(&behind), strip(&caught));
    assert_ne!(behind.verified_ack, caught.verified_ack);
}

#[test]
fn signed_receipt_clock_is_returned_without_replacing_launch_clock() {
    let key = server_key();
    let options = supplied(&key);
    let dir = tempfile::tempdir().unwrap();
    let mut export = capture_export(dir.path());
    let launch = launch();
    append(&mut export, &launch);
    // Actual receipt issuance over the chain head at that moment.
    let (_, head) = export.head().unwrap();
    let receipt = StartReceipt::sign(
        StartReceiptClaims {
            entry_id: ENTRY.to_string(),
            connector_id: "codex-cli".to_string(),
            workspace: "/ws".to_string(),
            session_id: "session-t18".to_string(),
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
    for record in goal_stream().into_iter().chain(complete_wire(None)) {
        append(&mut export, &record);
    }
    let digest = receipt.canonical_digest().unwrap();
    export
        .write_cutoff_for_protocol(
            M2_PROTOCOL_VERSION,
            ENTRY,
            "2026-09-27T12:00:08Z",
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
            "2026-09-27T12:00:09Z",
            receipt_digest_finalization(&digest),
        )
        .unwrap();
    drop(export);

    let (report, seen) = scan_with(dir.path(), &options);
    assert_eq!(report.exit_code(), 0, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Valid);
    assert_eq!(report.anchor, AnchorState::Signed);
    assert_eq!(report.key_source, KeySource::Supplied);
    assert!(report.sealed);
    assert_eq!(
        report.receipt_clock,
        Some(VerifiedReceiptClock {
            started_at: STARTED_AT.to_string(),
            deadline_at: DEADLINE_AT.to_string(),
        })
    );
    assert_eq!(
        report.terminal,
        Some(CaptureTerminalFacts {
            seq: final_link.seq,
            hash: final_link.hash.clone(),
            observed: None,
            observed_at: "2026-09-27T12:00:09Z".to_string(),
            kind: CaptureTerminalKind::SignedFinalization,
            reason: None,
            termination: Some(confirmed()),
            seal_manifest_sha256: None,
        })
    );
    // The launch record keeps its own local clock; the signed start does not replace it.
    assert_eq!(seen[0], launch);
    let CaptureRecord::Launch { observed, .. } = &seen[0] else {
        unreachable!()
    };
    assert_ne!(observed.wall, STARTED_AT);

    // Without the signer's key no accepted key checks the receipt: it is neither a signed
    // anchor nor an artifact the reader may ignore.
    let (unkeyed, _) = scan(dir.path());
    assert_eq!(unkeyed.exit_code(), 2, "{unkeyed:#?}");
    assert!(
        has(&unkeyed, "receipt_key_not_accepted"),
        "{:?}",
        unkeyed.findings
    );
    assert_eq!(unkeyed.receipt_clock, None);
    assert_eq!(unkeyed.anchor, AnchorState::LocalOnly);
}

/// A signed-receipt trace whose `Launch` carries `launch_key` and whose receipt claims
/// `receipt_key`, built exactly like the receipt trace above.
fn receipt_trace_with_launch_key(receipt_key: Option<&str>, launch_key: &str) -> tempfile::TempDir {
    let server = server_key();
    let dir = tempfile::tempdir().unwrap();
    let mut export = capture_export(dir.path());
    let mut launch = launch();
    if let CaptureRecord::Launch { run_public_key, .. } = &mut launch {
        *run_public_key = Some(launch_key.to_string());
    }
    append(&mut export, &launch);
    let (_, head) = export.head().unwrap();
    let receipt = StartReceipt::sign(
        StartReceiptClaims {
            entry_id: ENTRY.to_string(),
            connector_id: "codex-cli".to_string(),
            workspace: "/ws".to_string(),
            session_id: "session-t18".to_string(),
            started_at: STARTED_AT.to_string(),
            deadline_at: DEADLINE_AT.to_string(),
            manifest_sha256: "11".repeat(32),
            chain_head: head,
            connector_dir_sha256: None,
            run_public_key: receipt_key.map(str::to_string),
            questionnaire_version: None,
        },
        &server,
    )
    .unwrap();
    export.write_receipt(&receipt).unwrap();
    for record in goal_stream().into_iter().chain(complete_wire(None)) {
        append(&mut export, &record);
    }
    let digest = receipt.canonical_digest().unwrap();
    export
        .write_cutoff_for_protocol(
            M2_PROTOCOL_VERSION,
            ENTRY,
            "2026-09-27T12:00:08Z",
            CutoffReachedV1 {
                receipt_sha256: digest.clone(),
                deadline_at: utc(DEADLINE_AT),
                pre_cutoff_drain: PreCutoffDrainOutcomeV1::Complete,
            },
        )
        .unwrap();
    export
        .write_finalization_for_protocol(
            M2_PROTOCOL_VERSION,
            ENTRY,
            "2026-09-27T12:00:09Z",
            receipt_digest_finalization(&digest),
        )
        .unwrap();
    dir
}

/// Contract S1: a `Launch.run_public_key` must equal the key of a verified receipt that
/// carries one. No v5 producer writes a receipt key yet (capture entries are Legacy-bound and
/// request no receipt), so this guards the S-phase registration, not today's traces.
#[test]
fn a_launch_key_must_agree_with_a_verified_receipts_key() {
    let hex_key = |seed: u8| {
        hex::encode(
            SigningKey::from_bytes(&[seed; 32])
                .verifying_key()
                .to_bytes(),
        )
    };
    let (run_a, run_b) = (hex_key(0x11), hex_key(0x22));
    let options = supplied(&server_key());

    // Match: valid, signed.
    let dir = receipt_trace_with_launch_key(Some(&run_a), &run_a);
    let (report, _) = scan_with(dir.path(), &options);
    assert_eq!(report.exit_code(), 0, "{report:#?}");
    assert_eq!(report.anchor, AnchorState::Signed);

    // Mismatch: the Launch is refused as the first capture record, and nothing after it is read.
    let dir = receipt_trace_with_launch_key(Some(&run_a), &run_b);
    let (report, seen) = scan_with(dir.path(), &options);
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert_eq!(
        report.findings.first().map(String::as_str),
        Some("record_field_invalid")
    );
    assert_eq!(
        report.validated_records, 1,
        "only the baseline precedes the Launch"
    );
    assert!(seen.is_empty());

    // The receipt's spelling is compared canonically: an uppercase receipt key for the same
    // point agrees. (The Launch's own spelling must be lowercase; the corpus pins that.)
    let dir = receipt_trace_with_launch_key(Some(&run_a.to_uppercase()), &run_a);
    let (report, _) = scan_with(dir.path(), &options);
    assert_eq!(report.exit_code(), 0, "{report:#?}");

    // A receipt key that is not a usable key is a receipt fault, named as one.
    let dir = receipt_trace_with_launch_key(Some(&"zz".repeat(32)), &run_a);
    let (report, _) = scan_with(dir.path(), &options);
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert!(has(&report, "receipt_malformed"), "{:?}", report.findings);
    assert!(
        !has(&report, "record_field_invalid"),
        "{:?}",
        report.findings
    );

    // A receipt with no key claims nothing to compare. Skipping is right: the design requires
    // equality only with "any receipt or registration key supplied", and every receipt that
    // can sit beside a v5 chain today is keyless.
    let dir = receipt_trace_with_launch_key(None, &run_a);
    let (report, _) = scan_with(dir.path(), &options);
    assert_eq!(report.exit_code(), 0, "{report:#?}");
}

#[test]
fn a_malformed_launch_connector_digest_is_still_refused() {
    let mut launch = launch();
    let CaptureRecord::Launch {
        connector_dir_sha256,
        ..
    } = &mut launch
    else {
        unreachable!()
    };
    *connector_dir_sha256 = Some("not-a-sha256".to_string());
    let mut records = goal_stream();
    records.push(launch);
    records.extend(complete_wire(None));
    let dir = make_trace(&records, true);
    let (report, _) = scan(dir.path());
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert!(has(&report, "record_field_invalid"), "{report:#?}");
}

#[test]
fn local_terminal_preserves_monotonic_clock_identity() {
    let observed = CaptureTime {
        wall: "2026-09-27T12:00:07.250Z".to_string(),
        mono_ns: 987_654_321_000,
        clock_id: "daemon-clock-epoch-7".to_string(),
    };
    let dir = make_trace_finishing_at(&launched(complete_wire(None)), Some(observed.clone()));
    let (report, seen) = scan(dir.path());
    assert_eq!(report.exit_code(), 0, "{report:#?}");
    match seen.last() {
        Some(CaptureRecord::LocalFinish {
            observed: emitted, ..
        }) => assert_eq!(emitted, &observed),
        other => panic!("the local finish is emitted last, got {other:?}"),
    }
    let terminal = report.terminal.expect("a verified local terminal");
    assert_eq!(terminal.observed, Some(observed.clone()));
    assert_eq!(terminal.observed_at, observed.wall);
}

#[test]
fn verifier_accepts_paired_claude_launch_provenance_and_refuses_unpaired_claims() {
    let mut records = launched(complete_wire(None));
    let launch = records
        .iter_mut()
        .find(|record| matches!(record, CaptureRecord::Launch { .. }))
        .unwrap();
    if let CaptureRecord::Launch {
        profile_id,
        allowed_tool_rule,
        process_handoff_digest,
        ..
    } = launch
    {
        *profile_id = "claude-code-cli".into();
        *allowed_tool_rule = Some("Bash(python3 demo.py *)".into());
        *process_handoff_digest = Some("d".repeat(64));
    }
    let (accepted, _) = scan(make_trace(&records, true).path());
    assert_eq!(accepted.exit_code(), 0, "{accepted:#?}");
    let launch = records
        .iter_mut()
        .find(|record| matches!(record, CaptureRecord::Launch { .. }))
        .unwrap();
    if let CaptureRecord::Launch {
        process_handoff_digest,
        ..
    } = launch
    {
        *process_handoff_digest = None;
    }
    let (refused, _) = scan(make_trace(&records, true).path());
    assert_eq!(refused.exit_code(), 2, "{refused:#?}");
    assert!(has(&refused, "record_field_invalid"), "{refused:#?}");
}

#[test]
fn missing_terminal_is_incomplete_not_pass() {
    let dir = make_trace(&launched(complete_wire(None)), false);
    let (report, _) = scan(dir.path());
    assert_eq!(report.exit_code(), 1, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Incomplete);
    assert!(!report.sealed);
    assert!(!report.torn_tail);
    assert!(!report.coverage_incomplete, "every producer closed");
    assert_eq!(report.terminal, None);
    assert!(has(&report, "chain_unsealed"), "{:?}", report.findings);
    assert_eq!(
        report.validated_records,
        chain_lines(dir.path()).len() as u64
    );
}

#[test]
fn a_complete_trace_needs_a_launch_bound_to_a_complete_goal_stream() {
    let incomplete = |records: Vec<CaptureRecord>, code: &str| {
        let (report, _) = scan(make_trace(&records, true).path());
        assert_eq!(report.exit_code(), 1, "{code}: {report:#?}");
        assert_eq!(report.integrity, CaptureIntegrity::Incomplete, "{code}");
        assert!(has(&report, code), "{code}: {:?}", report.findings);
    };
    let then_wire = |mut records: Vec<CaptureRecord>| {
        records.extend(complete_wire(None));
        records
    };

    // No Launch at all.
    incomplete(complete_wire(None), "launch_missing");
    // A Launch whose goal stream never opens.
    incomplete(then_wire(vec![launch()]), "launch_goal_unbound");
    // A goal stream that did not end Complete, and a stream the Launch names that is not a
    // goal stream.
    let mut interrupted = goal_stream();
    interrupted[2] = CaptureRecord::End {
        stream: key(GOAL, 1),
        observed: at(GOAL, 90),
        reason: EndReason::Interrupted,
    };
    interrupted.push(launch());
    incomplete(then_wire(interrupted), "launch_goal_unbound");
    let mut native = goal_stream();
    native[0] = open_native(GOAL, 1);
    native.push(launch());
    incomplete(then_wire(native), "launch_goal_unbound");

    // Bound in either order: goal first, or Launch first as the example capture runner
    // writes it.
    let mut launch_first = vec![launch()];
    launch_first.extend(goal_stream());
    for records in [launched(vec![]), launch_first] {
        let (report, _) = scan(make_trace(&then_wire(records), true).path());
        assert_eq!(report.exit_code(), 0, "{report:#?}");
        assert!(report.findings.is_empty(), "{:?}", report.findings);
    }
}

#[test]
fn gap_cannot_be_cleared_by_end_or_upload() {
    let key = server_key();
    let options = supplied(&key);
    let dir = tempfile::tempdir().unwrap();
    let mut export = capture_export(dir.path());
    for record in [
        conn_open(WIRE, false),
        open_wire(WIRE, 1, 1, None),
        segment(WIRE, 1, 0, 0, REQUEST_BODY),
        gap(Some(key_of(WIRE, 1))),
        end(WIRE, 1),
        conn_close(WIRE),
        closed(WIRE, Some(4)),
    ] {
        append(&mut export, &record);
    }
    let (_, finish) = export
        .write_local_finish(&manifest(), finish_time(), "participant_stop", confirmed())
        .unwrap();
    export
        .append_ack_synced(&sign_ack(&key, &finish, "2026-09-27T12:01:00Z"))
        .unwrap();
    drop(export);

    let (report, _) = scan_with(dir.path(), &options);
    assert_eq!(report.exit_code(), 1, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Incomplete);
    assert_eq!(report.gap_count, 1);
    assert!(report.coverage_incomplete);
    assert!(
        report.sealed,
        "the terminal is valid; the coverage is not complete"
    );
    assert_eq!(
        report.verified_ack.as_ref().map(|a| a.seq),
        Some(finish.seq),
        "a final ACK is delivery, not coverage"
    );
    assert!(has(&report, "gap_recorded"), "{:?}", report.findings);
}

fn key_of(producer: &str, stream: u64) -> StreamKey {
    key(producer, stream)
}

#[test]
fn producer_closed_attestation_does_not_claim_raw_sequence_reconstruction() {
    let with_close = |close: CaptureRecord| {
        let mut records = launched(complete_wire(None));
        *records.last_mut().unwrap() = close;
        make_trace(&records, true)
    };

    // A final_seq above the persisted records is an attestation the reader cannot
    // reconstruct from redacted records and does not contradict.
    let (report, _) = scan(with_close(closed(WIRE, Some(40))).path());
    assert_eq!(report.exit_code(), 0, "{report:#?}");
    let keys: Vec<String> = serde_json::to_value(&report)
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    let mut expected_keys = vec![
        "schema_version",
        "integrity",
        "anchor",
        "sealed",
        "validated_records",
        "validated_bytes",
        "validated_head",
        "gap_count",
        "coverage_incomplete",
        "torn_tail",
        "key_source",
        "findings",
        "verified_ack",
        "receipt_clock",
        "terminal",
    ];
    expected_keys.sort();
    let mut keys_sorted = keys.clone();
    keys_sorted.sort();
    assert_eq!(
        keys_sorted, expected_keys,
        "no field claims raw ingress sequences"
    );
    assert!(
        report.findings.iter().all(|f| !f.contains("sequence")),
        "{:?}",
        report.findings
    );

    // Below the frame-consuming records it closes (connection open/close, stream open/end:
    // at least four frames, so final_seq >= 3), the attestation contradicts the chain.
    let (report, _) = scan(with_close(closed(WIRE, Some(2))).path());
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert!(
        has(&report, "producer_closed_final_seq_inconsistent"),
        "{:?}",
        report.findings
    );
    let (report, _) = scan(with_close(closed(WIRE, None)).path());
    assert_eq!(report.exit_code(), 2, "None means zero frames: {report:#?}");

    // Once-only, and nothing of that epoch after it.
    let mut twice = complete_wire(None);
    twice.push(closed(WIRE, Some(5)));
    let (report, _) = scan(make_trace(&twice, true).path());
    assert_eq!(report.exit_code(), 2);
    assert!(
        has(&report, "producer_closed_twice"),
        "{:?}",
        report.findings
    );

    let mut after = complete_wire(None);
    after.push(open_wire(WIRE, 2, 2, None));
    let (report, _) = scan(make_trace(&after, true).path());
    assert_eq!(report.exit_code(), 2);
    assert!(
        has(&report, "record_after_producer_closed"),
        "{:?}",
        report.findings
    );

    // A close with a payload stream still open is not a clean drain.
    let open_at_close = vec![
        open_native(NATIVE, 1),
        segment(NATIVE, 1, 0, 0, b"line\n"),
        closed(NATIVE, Some(1)),
    ];
    let (report, _) = scan(make_trace(&open_at_close, true).path());
    assert_eq!(report.exit_code(), 2);
    assert!(
        has(&report, "producer_closed_with_open_streams"),
        "{:?}",
        report.findings
    );

    // Absent closure retains an unknown tail: incomplete, never a pass.
    let mut unclosed = complete_wire(None);
    unclosed.pop();
    let (report, _) = scan(make_trace(&unclosed, true).path());
    assert_eq!(report.exit_code(), 1, "{report:#?}");
    assert!(report.coverage_incomplete);
    assert!(has(&report, "producer_unclosed"), "{:?}", report.findings);
}

#[test]
fn large_chain_scans_without_whole_trace_collection() {
    // Evidence hook: M2_T18_LARGE_TRACE_DIR keeps the generated trace for an external
    // peak-memory measurement of the binary, and M2_T18_LARGE_SEGMENTS sizes it.
    let segments: u64 = std::env::var("M2_T18_LARGE_SEGMENTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_500);
    let kept = std::env::var_os("M2_T18_LARGE_TRACE_DIR").map(PathBuf::from);
    let temp = tempfile::tempdir().unwrap();
    let root = kept.clone().unwrap_or_else(|| temp.path().to_path_buf());

    let chunk_len = 8 * 1024;
    let mut export = capture_export(&root);
    for record in launched(vec![]) {
        append(&mut export, &record);
    }
    append(&mut export, &conn_open(WIRE, false));
    append(
        &mut export,
        &open_wire(WIRE, 1, 1, Some("openai-responses")),
    );
    let mut offset = 0u64;
    for part in 0..segments {
        // Generated incrementally; nothing retains the records.
        let chunk = vec![b'a' + (part % 26) as u8; chunk_len];
        append(&mut export, &segment(WIRE, 1, part, offset, &chunk));
        offset += chunk_len as u64;
    }
    append(&mut export, &end(WIRE, 1));
    append(&mut export, &conn_close(WIRE));
    append(&mut export, &closed(WIRE, Some(segments + 3)));
    export
        .write_local_finish(&manifest(), finish_time(), "participant_stop", confirmed())
        .unwrap();
    drop(export);

    let mut count = 0u64;
    let mut segment_parts = 0u64;
    let report = scan_capture_trace(&root, &VerifyOptions::default(), &mut |record| {
        count += 1;
        if matches!(record, CaptureRecord::Segment { stream, .. } if stream.producer == WIRE) {
            segment_parts += 1;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(report.exit_code(), 0, "{report:#?}");
    assert_eq!(segment_parts, segments);
    // goal and launch, connection open, stream open, segments, end, connection close,
    // producer close, finish
    assert_eq!(count, 5 + segments + 6);
    assert_eq!(report.validated_records, 5 + segments + 7);
    assert_eq!(
        report.validated_bytes,
        std::fs::metadata(root.join(CHAIN_FILE)).unwrap().len()
    );
}

// ---------------------------------------------------------------------------------------
// Bounded state, ranges, recovery and the command line.
// ---------------------------------------------------------------------------------------

#[test]
fn stream_ids_and_bounded_state_limits_are_enforced() {
    // A reused stream ID is refused against the producer's high-water mark alone.
    let reused = vec![
        open_native(NATIVE, 1),
        end(NATIVE, 1),
        open_native(NATIVE, 1),
    ];
    let (report, _) = scan(make_trace(&reused, true).path());
    assert_eq!(report.exit_code(), 2);
    assert!(
        has(&report, "stream_id_not_increasing"),
        "{:?}",
        report.findings
    );

    // A long run of streams opened and ended one at a time does not grow any state.
    let streams = 3 * MAX_OPEN_STREAMS as u64;
    let mut sequential = Vec::new();
    for stream in 1..=streams {
        sequential.push(open_native(NATIVE, stream));
        sequential.push(end(NATIVE, stream));
    }
    sequential.push(closed(NATIVE, Some(2 * streams - 1)));
    let (report, _) = scan(make_trace(&launched(sequential), true).path());
    assert_eq!(report.exit_code(), 0, "{report:#?}");

    // One more simultaneously open stream than the bound admits.
    let simultaneous: Vec<CaptureRecord> = (1..=MAX_OPEN_STREAMS as u64 + 1)
        .map(|stream| open_native(NATIVE, stream))
        .collect();
    let (report, _) = scan(make_trace(&simultaneous, false).path());
    assert_eq!(report.exit_code(), 2);
    assert!(
        has(&report, "open_stream_limit_exceeded"),
        "{:?}",
        report.findings
    );
    assert_eq!(report.validated_records, 1 + MAX_OPEN_STREAMS as u64);

    // The distinct-producer bound (MAX_PRODUCERS, completed epochs included) is checked at its
    // edges by the reader's unit tests: a trace that reaches it takes seconds to build (D58).

    // Launch goals and completed goal streams are each held to 128, the producer bound before
    // D58 raised it: these verdicts did not change.
    const GOAL_STREAMS: u64 = 128;
    let launches: Vec<CaptureRecord> = (1..=GOAL_STREAMS + 1)
        .map(|stream| launch_naming(key(GOAL, stream)))
        .collect();
    let (report, _) = scan(make_trace(&launches, false).path());
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert!(
        has(&report, "launch_limit_exceeded"),
        "{:?}",
        report.findings
    );
    assert_eq!(report.validated_records, 1 + GOAL_STREAMS);
    let goals: Vec<CaptureRecord> = (1..=GOAL_STREAMS + 1)
        .flat_map(|stream| [open_goal(stream), end(GOAL, stream)])
        .collect();
    let (report, _) = scan(make_trace(&goals, false).path());
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert!(
        has(&report, "goal_stream_limit_exceeded"),
        "{:?}",
        report.findings
    );
    assert_eq!(report.validated_records, 2 + 2 * GOAL_STREAMS);
}

#[test]
fn segment_ranges_and_audit_bounds_are_checked() {
    let native = |segments: Vec<CaptureRecord>| {
        let mut records = vec![open_native(NATIVE, 1)];
        let frames = segments.len() as u64 + 1;
        records.extend(segments);
        records.push(end(NATIVE, 1));
        records.push(closed(NATIVE, Some(frames)));
        records
    };

    // A hole in the source range with no recorded gap is dishonest accounting.
    let hole = native(vec![
        segment(NATIVE, 1, 0, 0, b"abc"),
        segment(NATIVE, 1, 1, 10, b"def"),
    ]);
    let (report, _) = scan(make_trace(&hole, true).path());
    assert_eq!(report.exit_code(), 2);
    assert!(
        has(&report, "segment_source_unexplained_hole"),
        "{:?}",
        report.findings
    );

    // The same hole explained by a stream gap is incomplete coverage, not invalid.
    let explained = native(vec![
        segment(NATIVE, 1, 0, 0, b"abc"),
        gap(Some(key(NATIVE, 1))),
        segment(NATIVE, 1, 1, 10, b"def"),
    ]);
    let (report, _) = scan(make_trace(&explained, true).path());
    assert_eq!(report.exit_code(), 1, "{report:#?}");

    // Overlap and out-of-order parts.
    let overlap = native(vec![
        segment(NATIVE, 1, 0, 0, b"abcdef"),
        segment(NATIVE, 1, 1, 3, b"def"),
    ]);
    let (report, _) = scan(make_trace(&overlap, true).path());
    assert!(
        has(&report, "segment_source_overlap"),
        "{:?}",
        report.findings
    );
    let parts = native(vec![segment(NATIVE, 1, 1, 0, b"abc")]);
    let (report, _) = scan(make_trace(&parts, true).path());
    assert!(
        has(&report, "segment_part_out_of_order"),
        "{:?}",
        report.findings
    );

    // Unredacted bytes must be exactly the source range they claim.
    let CaptureRecord::Segment {
        stream,
        first,
        last,
        data_b64,
        audit,
        ..
    } = segment(NATIVE, 1, 0, 0, b"abc")
    else {
        unreachable!()
    };
    let long_claim = CaptureRecord::Segment {
        stream: stream.clone(),
        part: 0,
        source_offset: 0,
        source_len: 7,
        first: first.clone(),
        last: last.clone(),
        data_b64: data_b64.clone(),
        audit,
    };
    let (report, _) = scan(make_trace(&native(vec![long_claim]), true).path());
    assert!(
        has(&report, "segment_length_mismatch"),
        "{:?}",
        report.findings
    );

    // A redaction audit legitimately changes the length, inside its own source range...
    let redacted = |span_end: u64| CaptureRecord::Segment {
        stream: stream.clone(),
        part: 0,
        source_offset: 0,
        source_len: 40,
        first: first.clone(),
        last: last.clone(),
        data_b64: STANDARD.encode(b"key=[REDACTED:api_key]"),
        audit: RedactionAudit {
            spans: vec![RedactionAuditSpan {
                kind: "api_key".to_string(),
                start: 4,
                end: span_end,
            }],
        },
    };
    let (report, _) = scan(make_trace(&launched(native(vec![redacted(40)])), true).path());
    assert_eq!(report.exit_code(), 0, "{report:#?}");
    // ...and never outside it.
    let (report, _) = scan(make_trace(&native(vec![redacted(41)]), true).path());
    assert!(
        has(&report, "segment_audit_out_of_range"),
        "{:?}",
        report.findings
    );
}

#[test]
fn recovery_releases_the_old_epoch_and_reobserves_connections() {
    // Recorder recovery: the old epoch is lost (its stream keys stay invalid), the fresh
    // producer's first frame is the Recovery, and a surviving socket is re-observed.
    let recovered = vec![
        conn_open(WIRE, false),
        open_wire(WIRE, 1, 1, None),
        segment(WIRE, 1, 0, 0, REQUEST_BODY),
        gap(Some(key(WIRE, 1))),
        CaptureRecord::Recovery {
            previous_producer: WIRE.to_string(),
            producer: RECOVERED.to_string(),
            observed: at(RECOVERED, 1),
            reason: GapReason::IngestLost,
        },
        conn_open(RECOVERED, true),
        open_wire(RECOVERED, 1, 2, None),
        end(RECOVERED, 1),
        conn_close(RECOVERED),
        closed(RECOVERED, Some(4)),
    ];
    let (report, _) = scan(make_trace(&launched(recovered.clone()), true).path());
    assert_eq!(report.exit_code(), 1, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Incomplete);
    assert!(report.coverage_incomplete);
    // Sequence B: the Recovery names a recorded producer, so the orphan rule is not taken
    // and no `recovery_names_unrecorded_producer` finding is raised. Explicit loss, not
    // unknown tail.
    assert_eq!(
        report.findings,
        vec!["gap_recorded".to_string(), "producer_recovered".to_string()]
    );

    // The lost epoch's stream key cannot carry more payload after recovery.
    let mut stale = recovered.clone();
    stale.insert(6, segment(WIRE, 1, 1, REQUEST_BODY.len() as u64, b"late"));
    let (report, _) = scan(make_trace(&stale, true).path());
    assert_eq!(report.exit_code(), 2);
    assert!(has(&report, "stream_not_open"), "{:?}", report.findings);

    // A re-observation outside a recovered epoch is not a fresh recovery observation.
    let (report, _) = scan(make_trace(&[conn_open(WIRE, true)], false).path());
    assert_eq!(report.exit_code(), 2);
    assert!(
        has(&report, "connection_reobserved_outside_recovery"),
        "{:?}",
        report.findings
    );
}

#[test]
fn a_trace_with_five_hundred_producers_verifies() {
    // Every daemon stall ends an epoch and its recovery is a fresh producer. Five hundred in
    // one entry, each re-observing the surviving socket, raise no producer limit (D58).
    const PRODUCERS: u64 = 500;
    let id = |n: u64| format!("{n:032x}");
    let mut records = vec![conn_open(&id(1), false)];
    for n in 2..=PRODUCERS {
        records.push(gap(None));
        records.push(recovery_of(&id(n - 1), &id(n)));
        records.push(conn_open(&id(n), true));
    }
    records.push(conn_close(&id(PRODUCERS)));
    records.push(closed(&id(PRODUCERS), Some(2)));
    let (report, _) = scan(make_trace(&launched(records), true).path());
    assert_eq!(report.exit_code(), 1, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Incomplete);
    assert_eq!(report.gap_count, PRODUCERS - 1);
    assert_eq!(
        report.findings,
        findings(&["gap_recorded", "producer_recovered"]),
        "explicit loss at every stall, and no producer limit"
    );
}

#[test]
fn recoveries_naming_unrecorded_predecessors_cost_linear_time() {
    // The worst case for an untrusted upload: every Recovery names a predecessor the chain never
    // recorded, so each supersedes the whole wire lineage. A superseded producer is never named
    // again, so each one is walked once. 32 768 such recoveries admit exactly MAX_PRODUCERS IDs
    // (D58). Linear, the scan takes about a second in the test profile; walking every wire
    // producer ever seen took minutes.
    let recoveries = MAX_PRODUCERS as u64 / 2;
    let records: Vec<CaptureRecord> = (0..recoveries)
        .map(|n| recovery_of(&format!("{:032x}", 2 * n), &format!("{:032x}", 2 * n + 1)))
        .collect();
    let trace = make_trace(&records, false);
    let started = std::time::Instant::now();
    let (report, _) = scan(trace.path());
    let scanned = started.elapsed();
    assert!(!has(&report, "producer_limit_exceeded"), "{report:#?}");
    assert_eq!(report.validated_records, 1 + recoveries);
    assert!(has(&report, "recovery_names_unrecorded_producer"));
    let started = std::time::Instant::now();
    assert_eq!(decoded_segments(&records), 0);
    let decoded = started.elapsed();
    eprintln!("{recoveries} orphan recoveries: scan {scanned:?}, decode {decoded:?}");
    assert!(
        scanned + decoded < std::time::Duration::from_secs(15),
        "scan {scanned:?}, decode {decoded:?}"
    );
}

fn findings(codes: &[&str]) -> Vec<String> {
    codes.iter().map(|code| code.to_string()).collect()
}

#[test]
fn recovery_naming_unrecorded_producer_supersedes_the_wire_lineage() {
    // Sequence A. P (WIRE) is the initial wire producer: it opens CONN and P/1, and
    // the owner records P's loss as a stream gap. Q (IN_FLIGHT) was admitted and its
    // Recovery answered Queued, but nothing of Q was persisted. R's Recovery names Q and
    // re-observes CONN.
    let lineage = vec![
        conn_open(WIRE, false),
        open_wire(WIRE, 1, 1, None),
        segment(WIRE, 1, 0, 0, REQUEST_BODY),
        gap(Some(key(WIRE, 1))),
        recovery_of(IN_FLIGHT, RECOVERED),
        conn_open(RECOVERED, true),
        open_wire(RECOVERED, 1, 2, None),
        segment(RECOVERED, 1, 0, 0, REQUEST_MORE),
        end(RECOVERED, 1),
        conn_close(RECOVERED),
        // Recovery 0, Connection 1, Open 2, Bytes 3, End 4, Connection 5.
        closed(RECOVERED, Some(5)),
    ];
    let entry = launched(lineage.clone());
    let (report, seen) = scan(make_trace(&entry, true).path());
    // Q is lost with zero records, P is superseded with it, and every Recovery keeps
    // coverage incomplete.
    assert_eq!(report.exit_code(), 1, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Incomplete);
    assert!(report.sealed);
    assert!(report.coverage_incomplete);
    assert_eq!(report.gap_count, 1);
    assert_eq!(
        report.findings,
        findings(&[
            "gap_recorded",
            "producer_recovered",
            "recovery_names_unrecorded_producer",
        ]),
        "P and Q are explicit loss, not unknown tail"
    );
    assert_eq!(&seen[..entry.len()], &entry[..]);

    // P was superseded, not skipped over: after the Recovery, its connection context and
    // its stream are gone.
    for (late, code) in [
        (conn_close(WIRE), "record_from_lost_producer"),
        (
            segment(WIRE, 1, 1, REQUEST_BODY.len() as u64, b"late"),
            "stream_not_open",
        ),
    ] {
        let mut stale = lineage.clone();
        stale.insert(5, late);
        let (report, _) = scan(make_trace(&stale, true).path());
        assert_eq!(report.exit_code(), 2, "{code}: {report:#?}");
        assert!(has(&report, code), "{code}: {:?}", report.findings);
        assert_eq!(report.validated_records, 1 + 5, "{code}");
    }

    // The unrecorded Q counts toward MAX_PRODUCERS like any recorded one. The reader's unit
    // test `producer_bound_edges_keep_the_sequential_rule` holds the cap-edge cases (D58).
}

/// A stream-less gap on `producer`'s own clock: what the daemon chains for a `Loss` frame
/// (H-16). A daemon-origin gap names the daemon's clock instead.
fn producer_loss(producer: &str, n: u64) -> CaptureRecord {
    CaptureRecord::Gap {
        stream: None,
        observed: at(producer, n),
        reason: GapReason::DecodeFailed,
        lost_bytes: None,
    }
}

/// H-16 pin (green before and after the daemon records `Loss` frames: the reader changed not
/// at all). A clean exchange, a producer-clocked stream-less gap, and the producer's clean
/// `ProducerClosed` read incomplete: the close attests the frame sequence, and the gap in it
/// is the loss.
#[test]
fn a_producer_clocked_streamless_gap_before_a_clean_close_is_incomplete() {
    // Producer frames: connection open 0, stream open 1, bytes 2-3, end 4, loss 5,
    // connection close 6.
    let records = launched(vec![
        conn_open(WIRE, false),
        open_wire(WIRE, 1, 1, None),
        segment(WIRE, 1, 0, 0, REQUEST_BODY),
        segment(WIRE, 1, 1, REQUEST_BODY.len() as u64, REQUEST_MORE),
        end(WIRE, 1),
        producer_loss(WIRE, 5),
        conn_close(WIRE),
        closed(WIRE, Some(6)),
    ]);
    let (report, seen) = scan(make_trace(&records, true).path());
    assert_eq!(report.exit_code(), 1, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Incomplete);
    assert!(report.sealed);
    assert!(report.coverage_incomplete);
    assert_eq!(report.gap_count, 1);
    assert!(has(&report, "gap_recorded"), "{:?}", report.findings);
    assert_eq!(&seen[..records.len()], &records[..]);

    // The same trace without the loss is complete: the verdict is the gap and nothing else.
    let clean = launched(vec![
        conn_open(WIRE, false),
        open_wire(WIRE, 1, 1, None),
        segment(WIRE, 1, 0, 0, REQUEST_BODY),
        segment(WIRE, 1, 1, REQUEST_BODY.len() as u64, REQUEST_MORE),
        end(WIRE, 1),
        conn_close(WIRE),
        closed(WIRE, Some(5)),
    ]);
    let (report, _) = scan(make_trace(&clean, true).path());
    assert_eq!(report.exit_code(), 0, "{report:#?}");
}

/// H-16 (review I-3c): a loss-only epoch is a new chain shape. One producer-clocked stream-less
/// gap and a `ProducerClosed { final_seq: Some(0) }` with no producer-attributed record reads
/// valid and incomplete, never invalid; and a `Recovery` that names such an epoch reads only
/// the informational `recovery_names_unrecorded_producer` besides the loss.
#[test]
fn a_loss_only_epoch_reads_valid_and_incomplete() {
    let records = launched(vec![producer_loss(WIRE, 0), closed(WIRE, Some(0))]);
    let (report, seen) = scan(make_trace(&records, true).path());
    assert_eq!(report.exit_code(), 1, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Incomplete);
    assert!(report.sealed);
    assert!(report.coverage_incomplete);
    assert_eq!(report.gap_count, 1);
    assert!(has(&report, "gap_recorded"), "{:?}", report.findings);
    assert_eq!(&seen[..records.len()], &records[..]);

    // A close that attests more frames than the chain holds for the producer is still a
    // lower-bound match, as before: a loss frame is a frame the chain does not attribute.
    let records = launched(vec![producer_loss(WIRE, 0), closed(WIRE, Some(3))]);
    let (report, _) = scan(make_trace(&records, true).path());
    assert_eq!(report.exit_code(), 1, "{report:#?}");

    // A fresh epoch recovers from the loss-only epoch.
    let records = launched(vec![
        producer_loss(WIRE, 0),
        recovery_of(WIRE, RECOVERED),
        conn_open(RECOVERED, true),
        open_wire(RECOVERED, 1, 1, None),
        segment(RECOVERED, 1, 0, 0, REQUEST_BODY),
        end(RECOVERED, 1),
        conn_close(RECOVERED),
        // Recovery 0, Connection 1, Open 2, Bytes 3, End 4, Connection 5.
        closed(RECOVERED, Some(5)),
    ]);
    let (report, _) = scan(make_trace(&records, true).path());
    assert_eq!(report.exit_code(), 1, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Incomplete);
    assert_eq!(
        report.findings,
        findings(&[
            "gap_recorded",
            "producer_recovered",
            "recovery_names_unrecorded_producer",
        ]),
        "{:?}",
        report.findings
    );
}

#[test]
fn native_and_goal_producers_survive_an_unrecorded_recovery() {
    // Sequence C. A goal producer and a native producer are both live, each with an
    // open stream, when the wire lineage P, Q (lost in flight), R recovers. Neither ever
    // recorded a Connection, so neither is wire and neither is superseded.
    let entry = vec![
        launch(),
        open_goal(1),
        segment(GOAL, 1, 0, 0, b"reach the goal"),
        open_native(NATIVE, 1),
        segment(NATIVE, 1, 0, 0, b"line one\n"),
        conn_open(WIRE, false),
        open_wire(WIRE, 1, 1, None),
        segment(WIRE, 1, 0, 0, REQUEST_BODY),
        gap(Some(key(WIRE, 1))),
        recovery_of(IN_FLIGHT, RECOVERED),
        conn_open(RECOVERED, true),
        open_wire(RECOVERED, 1, 2, None),
        end(RECOVERED, 1),
        conn_close(RECOVERED),
        closed(RECOVERED, Some(4)),
        // The native and goal streams are still open after the Recovery, and both
        // producers close validly.
        segment(NATIVE, 1, 1, 9, b"line two\n"),
        end(NATIVE, 1),
        closed(NATIVE, Some(3)),
        end(GOAL, 1),
        closed(GOAL, Some(2)),
    ];
    let (report, _) = scan(make_trace(&entry, true).path());
    assert_eq!(report.exit_code(), 1, "the recovery alone: {report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Incomplete);
    assert!(report.sealed);
    assert_eq!(
        report.findings,
        findings(&[
            "gap_recorded",
            "producer_recovered",
            "recovery_names_unrecorded_producer",
        ]),
        "no producer_unclosed for N or G, and no refusal of their later records"
    );
}

#[test]
fn reader_and_session_apply_the_same_recovery_lineage_sequences() {
    // Recorded predecessor: only the named epoch is superseded. The independent wire stream
    // survives in both the validating reader and the non-validating decode session.
    let recorded = vec![
        conn_open(WIRE, false),
        probe_open(open_wire(WIRE, 1, 1, None)),
        probe_open(open_wire(OTHER, 1, 2, None)),
        recovery_of(WIRE, RECOVERED),
        segment(OTHER, 1, 0, 0, b"survives"),
        end(OTHER, 1),
    ];
    let (report, seen) = scan(make_trace(&recorded, false).path());
    assert_ne!(report.integrity, CaptureIntegrity::Invalid, "{report:#?}");
    assert_eq!(seen, recorded);
    assert_eq!(decoded_segments(&recorded), 1);

    // Unrecorded predecessor: only wire streams cascade. The native stream survives and is
    // delivered after the same sequence passed reader validation.
    let unrecorded = vec![
        probe_open(open_native(NATIVE, 1)),
        conn_open(WIRE, false),
        probe_open(open_wire(WIRE, 1, 1, None)),
        recovery_of(IN_FLIGHT, RECOVERED),
        segment(NATIVE, 1, 0, 0, b"native survives"),
        end(NATIVE, 1),
    ];
    let (report, seen) = scan(make_trace(&unrecorded, false).path());
    assert_ne!(report.integrity, CaptureIntegrity::Invalid, "{report:#?}");
    assert!(has(&report, "recovery_names_unrecorded_producer"));
    assert_eq!(seen, unrecorded);
    assert_eq!(decoded_segments(&unrecorded), 1);
}

#[test]
fn repeated_and_cap_edge_recoveries_are_bounded_for_the_nonvalidating_session() {
    // The reader rejects a repeated recovery of the same recorded predecessor. DecodeSession
    // does not validate, but it classifies the predecessor as recorded on the second recovery,
    // so an unrelated wire stream stays routed and the sequence cannot panic.
    let repeated = vec![
        conn_open(WIRE, false),
        probe_open(open_wire(OTHER, 1, 2, None)),
        recovery_of(WIRE, RECOVERED),
        recovery_of(WIRE, NATIVE),
        segment(OTHER, 1, 0, 0, b"still routed"),
    ];
    let (report, _) = scan(make_trace(&repeated, false).path());
    assert_eq!(report.integrity, CaptureIntegrity::Invalid, "{report:#?}");
    assert!(has(&report, "producer_recovered_twice"));
    assert_eq!(decoded_segments(&repeated), 1);

    // Admit OTHER's probe before filling to MAX_PRODUCERS - 1 identities. At that edge an unseen
    // predecessor and a fresh producer cannot both fit: the non-validating session admits the
    // Recovery's fresh owner in the final slot and cascades the live probe. Reopen OTHER, then
    // compare a recovery naming that recorded owner with an unseen predecessor: the former
    // preserves the probe and delivers one segment; the latter cascades it before a second segment.
    // If the critical fresh producer was not admitted, both recoveries cascade and the count is
    // zero.
    let mut capped = vec![probe_open(open_wire(OTHER, 1, 2, None))];
    capped.extend((1..(MAX_PRODUCERS as u64 - 1)).map(|n| closed(&format!("f{n:031x}"), None)));
    capped.push(recovery_of(IN_FLIGHT, RECOVERED));
    capped.push(probe_open(open_wire(OTHER, 2, 3, None)));
    capped.push(recovery_of(RECOVERED, NATIVE));
    capped.push(segment(OTHER, 2, 0, 0, b"recorded predecessor preserves"));
    capped.push(probe_open(open_wire(OTHER, 3, 4, None)));
    capped.push(recovery_of(
        "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        "dddddddddddddddddddddddddddddddd",
    ));
    capped.push(segment(OTHER, 3, 0, 0, b"unseen predecessor cascades"));
    // The reader's verdicts on this edge are its unit test's; only the session runs here.
    assert_eq!(decoded_segments(&capped), 1);
}

#[test]
fn recorded_recovery_does_not_release_another_live_wire_producer() {
    // Negative control. X (OTHER) is a live wire producer holding CONN; P (WIRE) holds
    // its own connection and fails. R's Recovery names P, which is recorded: the orphan rule
    // does not apply, X keeps CONN, and a re-observation never transfers ownership.
    let p_conn = CaptureRecord::Connection {
        producer: WIRE.to_string(),
        connection: OTHER_CONN.to_string(),
        observed: at(WIRE, 5),
        event: ConnectionEvent::Open {
            alpn: None,
            already_open: false,
        },
    };
    let chain = |previous: &str| {
        vec![
            conn_open(OTHER, false),
            p_conn.clone(),
            recovery_of(previous, RECOVERED),
            conn_open(RECOVERED, true),
        ]
    };
    let (report, _) = scan(make_trace(&chain(WIRE), true).path());
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert!(
        has(&report, "connection_already_open"),
        "{:?}",
        report.findings
    );
    assert!(!has(&report, "recovery_names_unrecorded_producer"));
    assert_eq!(report.validated_records, 1 + 3);

    // The control: the same chain naming a producer with no record supersedes every live
    // wire producer, X included, so CONN is free for R.
    let mut orphaned = chain(IN_FLIGHT);
    orphaned.extend([conn_close(RECOVERED), closed(RECOVERED, Some(2))]);
    let (report, _) = scan(make_trace(&launched(orphaned), true).path());
    assert_eq!(report.exit_code(), 1, "{report:#?}");
    assert_eq!(
        report.findings,
        findings(&["producer_recovered", "recovery_names_unrecorded_producer"])
    );
}

#[test]
fn recovery_cannot_supersede_a_native_or_goal_producer() {
    for (producer, first) in [(NATIVE, open_native(NATIVE, 1)), (GOAL, open_goal(1))] {
        let records = vec![first, recovery_of(producer, RECOVERED)];
        let (report, _) = scan(make_trace(&records, true).path());
        assert_eq!(report.exit_code(), 2, "{producer}: {report:#?}");
        assert!(
            has(&report, "recovery_names_non_wire_producer"),
            "{producer}: {:?}",
            report.findings
        );
        assert_eq!(report.validated_records, 2, "{producer}");
    }
}

#[test]
fn native_resume_opens_with_a_gap_and_continues_past_the_durable_end() {
    // A resumed native stream opens with an unknown-loss Gap, and
    // its first bytes start at the previously durable end, past zero.
    let resumed = vec![
        open_native(NATIVE, 1),
        gap(Some(key(NATIVE, 1))),
        segment(NATIVE, 1, 0, 4096, b"next line\n"),
        segment(NATIVE, 1, 1, 4106, b"more\n"),
        end(NATIVE, 1),
        gap(None),
        closed(NATIVE, Some(3)),
    ];
    let (report, _) = scan(make_trace(&resumed, true).path());
    assert_eq!(report.exit_code(), 1, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Incomplete);
    assert_eq!(
        report.gap_count, 2,
        "the stream gap and the stream-less loss"
    );

    // Without the gap the same forward jump is unexplained loss.
    let mut unexplained = resumed.clone();
    unexplained.remove(1);
    let (report, _) = scan(make_trace(&unexplained, true).path());
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert!(
        has(&report, "segment_source_unexplained_hole"),
        "{:?}",
        report.findings
    );

    // A gap licenses a forward jump only; a backward offset is still an overlap.
    let backward = vec![
        open_native(NATIVE, 1),
        segment(NATIVE, 1, 0, 0, b"abcdef"),
        gap(Some(key(NATIVE, 1))),
        segment(NATIVE, 1, 1, 3, b"def"),
        end(NATIVE, 1),
        closed(NATIVE, Some(3)),
    ];
    let (report, _) = scan(make_trace(&backward, true).path());
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert!(
        has(&report, "segment_source_overlap"),
        "{:?}",
        report.findings
    );
}

#[test]
fn v5_legacy_seal_terminal_is_reported_as_legacy_seal() {
    let dir = tempfile::tempdir().unwrap();
    let mut export = capture_export(dir.path());
    for record in launched(complete_wire(None)) {
        append(&mut export, &record);
    }
    let (_, link) = export
        .write_seal_for_protocol(M2_PROTOCOL_VERSION, &manifest())
        .unwrap();
    drop(export);
    let (report, _) = scan(dir.path());
    assert_eq!(report.exit_code(), 0, "{report:#?}");
    assert_eq!(
        report.terminal,
        Some(CaptureTerminalFacts {
            seq: link.seq,
            hash: link.hash.clone(),
            observed: None,
            observed_at: manifest().sealed_at,
            kind: CaptureTerminalKind::LegacySeal,
            reason: None,
            termination: None,
            seal_manifest_sha256: Some(manifest_sha256(&manifest()).unwrap()),
        })
    );
}

/// No key whose seed is written in this repository is trusted. A receipt and an ACK signed
/// under the old published placeholder seed `[5u8; 32]`, or under this suite's own test seed,
/// are never a signed anchor; only a key the run supplies (`--dev-key`) verifies them, and the
/// report says the trust was borrowed.
#[test]
fn a_receipt_signed_by_an_untrusted_key_is_never_a_signed_anchor() {
    for seed in [[5u8; 32], [0x42; 32]] {
        let key = SigningKey::from_bytes(&seed);
        let dir = tempfile::tempdir().unwrap();
        let mut export = capture_export(dir.path());
        append(&mut export, &launch());
        let (_, head) = export.head().unwrap();
        let receipt = StartReceipt::sign(
            StartReceiptClaims {
                entry_id: ENTRY.to_string(),
                connector_id: "codex-cli".to_string(),
                workspace: "/ws".to_string(),
                session_id: "session-t18".to_string(),
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
        for record in goal_stream().into_iter().chain(complete_wire(None)) {
            append(&mut export, &record);
        }
        let (_, finish) = export
            .write_local_finish(&manifest(), finish_time(), "participant_stop", confirmed())
            .unwrap();
        export
            .append_ack_synced(&sign_ack(&key, &finish, "2026-09-27T12:01:00Z"))
            .unwrap();
        drop(export);

        let (report, _) = scan(dir.path());
        assert_eq!(report.anchor, AnchorState::LocalOnly, "{report:#?}");
        assert_eq!(report.key_source, KeySource::None);
        assert_eq!(report.receipt_clock, None);
        assert_eq!(report.verified_ack, None);
        assert!(
            has(&report, "receipt_key_not_accepted"),
            "{:?}",
            report.findings
        );
        assert_eq!(report.exit_code(), 2);
        let output = bin()
            .arg("--capture-integrity")
            .arg(dir.path())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let printed: CaptureIntegrityReport = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(printed.anchor, AnchorState::LocalOnly);
        assert_eq!(printed.key_source, KeySource::None);

        // Supplied for this run, the same trace verifies as before, and says so.
        let (report, _) = scan_with(dir.path(), &supplied(&key));
        assert_eq!(report.exit_code(), 0, "{report:#?}");
        assert_eq!(report.anchor, AnchorState::Signed);
        assert_eq!(report.key_source, KeySource::Supplied);
        assert_eq!(
            report.verified_ack.as_ref().map(|a| a.seq),
            Some(finish.seq)
        );
        assert!(has(&report, "server_key_unpinned"), "{:?}", report.findings);
    }
}

#[test]
fn receipt_and_ack_bindings_fail_closed() {
    let key = server_key();
    let options = supplied(&key);
    let claims = |entry: &str, head: String| StartReceiptClaims {
        entry_id: entry.to_string(),
        connector_id: "codex-cli".to_string(),
        workspace: "/ws".to_string(),
        session_id: "session-t18".to_string(),
        started_at: STARTED_AT.to_string(),
        deadline_at: DEADLINE_AT.to_string(),
        manifest_sha256: "11".repeat(32),
        chain_head: head,
        connector_dir_sha256: None,
        run_public_key: None,
        questionnaire_version: None,
    };
    // A local-finish trace carrying a receipt issued after the launch record, then ACKs.
    type Acks<'a> = &'a dyn Fn(&[Link], &Link) -> Vec<AckCheckpoint>;
    let build = |receipt_for: &dyn Fn(String) -> StartReceipt, acks: Acks| {
        let dir = tempfile::tempdir().unwrap();
        let mut export = capture_export(dir.path());
        let mut links = vec![append(&mut export, &launch())];
        let (_, head) = export.head().unwrap();
        export.write_receipt(&receipt_for(head)).unwrap();
        for record in goal_stream().into_iter().chain(complete_wire(None)) {
            links.push(append(&mut export, &record));
        }
        let (_, finish) = export
            .write_local_finish(&manifest(), finish_time(), "participant_stop", confirmed())
            .unwrap();
        for ack in acks(&links, &finish) {
            export.append_ack_synced(&ack).unwrap();
        }
        dir
    };
    let honest = |head: String| StartReceipt::sign(claims(ENTRY, head), &key).unwrap();
    let at_terminal =
        |_: &[Link], finish: &Link| vec![sign_ack(&key, finish, "2026-09-27T12:01:00Z")];
    let expect_invalid = |dir: tempfile::TempDir, code: &str| {
        let (report, _) = scan_with(dir.path(), &options);
        assert_eq!(report.exit_code(), 2, "{code}: {report:#?}");
        assert!(has(&report, code), "{code}: {:?}", report.findings);
        report
    };

    let (control, _) = scan_with(build(&honest, &at_terminal).path(), &options);
    assert_eq!(control.exit_code(), 0, "control: {control:#?}");
    assert_eq!(control.anchor, AnchorState::Signed);
    assert!(control.receipt_clock.is_some());

    // A claim edited after the server signed it.
    let edited = |head: String| {
        let mut receipt = honest(head);
        receipt.claims.deadline_at = "2026-09-27T13:30:00Z".to_string();
        receipt
    };
    let report = expect_invalid(build(&edited, &at_terminal), "receipt_signature_invalid");
    assert_eq!(report.receipt_clock, None);
    assert_eq!(report.anchor, AnchorState::LocalOnly);

    // A genuine signature over a head this chain never held, or over another entry.
    let elsewhere = |_: String| StartReceipt::sign(claims(ENTRY, "ab".repeat(32)), &key).unwrap();
    let report = expect_invalid(
        build(&elsewhere, &at_terminal),
        "receipt_chain_head_unbound",
    );
    assert_eq!(report.receipt_clock, None);
    let other_entry = |head: String| StartReceipt::sign(claims("entry-other", head), &key).unwrap();
    expect_invalid(build(&other_entry, &at_terminal), "receipt_entry_mismatch");

    // A genuinely signed ACK that names the wrong head for its sequence.
    let wrong_head = |links: &[Link], _: &Link| {
        let forged = Link {
            seq: links[1].seq,
            prev: String::new(),
            hash: links[2].hash.clone(),
        };
        vec![sign_ack(&key, &forged, "2026-09-27T12:01:00Z")]
    };
    expect_invalid(build(&honest, &wrong_head), "ack_hash_mismatch");

    // An ACK past the chain's end, and ACKs that go backwards.
    let beyond = |_: &[Link], finish: &Link| {
        let past = Link {
            seq: finish.seq + 1,
            prev: String::new(),
            hash: finish.hash.clone(),
        };
        vec![sign_ack(&key, &past, "2026-09-27T12:01:00Z")]
    };
    expect_invalid(build(&honest, &beyond), "ack_beyond_chain");
    let backwards = |links: &[Link], finish: &Link| {
        vec![
            sign_ack(&key, finish, "2026-09-27T12:01:00Z"),
            sign_ack(&key, &links[1], "2026-09-27T12:02:00Z"),
        ]
    };
    expect_invalid(build(&honest, &backwards), "ack_not_monotonic");
}

/// A connection event on `WIRE` for the numbered connection identity `n`.
fn numbered_conn(n: u64, event: ConnectionEvent) -> CaptureRecord {
    CaptureRecord::Connection {
        producer: WIRE.to_string(),
        connection: format!("{n:032x}"),
        observed: at(WIRE, 5),
        event,
    }
}

#[test]
fn connection_identity_state_is_bounded_and_released_on_close() {
    let open = || ConnectionEvent::Open {
        alpn: None,
        already_open: false,
    };
    let close = || ConnectionEvent::Close {
        outcome: ConnectionOutcome::Complete,
    };

    // A long run of connections opened and closed one at a time: Close releases each
    // identity, so the run length alone never reaches the active-connection bound.
    let pairs = 3 * MAX_OPEN_CONNECTIONS as u64;
    let mut sequential = Vec::new();
    for n in 1..=pairs {
        sequential.push(numbered_conn(n, open()));
        sequential.push(numbered_conn(n, close()));
    }
    sequential.push(closed(WIRE, Some(2 * pairs - 1)));
    let (report, _) = scan(make_trace(&launched(sequential), true).path());
    assert_eq!(report.exit_code(), 0, "{report:#?}");

    // One more simultaneously active connection than the bound admits.
    let simultaneous: Vec<CaptureRecord> = (1..=MAX_OPEN_CONNECTIONS as u64 + 1)
        .map(|n| numbered_conn(n, open()))
        .collect();
    let (report, _) = scan(make_trace(&simultaneous, false).path());
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert!(
        has(&report, "connection_limit_exceeded"),
        "{:?}",
        report.findings
    );
    assert_eq!(report.validated_records, 1 + MAX_OPEN_CONNECTIONS as u64);
}

#[test]
fn unsigned_fail_closed_rules_refuse_inconsistent_chains() {
    let expect = |records: &[CaptureRecord], code: &str, validated: u64| {
        let (report, _) = scan(make_trace(records, true).path());
        assert_eq!(report.exit_code(), 2, "{code}: {report:#?}");
        assert!(has(&report, code), "{code}: {:?}", report.findings);
        assert_eq!(report.validated_records, validated, "{code}");
    };
    let recovery = |previous: &str, producer: &str| CaptureRecord::Recovery {
        previous_producer: previous.to_string(),
        producer: producer.to_string(),
        observed: at(producer, 1),
        reason: GapReason::IngestLost,
    };

    // A record of an epoch that a Recovery superseded.
    expect(
        &[
            conn_open(WIRE, false),
            recovery(WIRE, RECOVERED),
            conn_close(WIRE),
        ],
        "record_from_lost_producer",
        3,
    );
    // A Recovery's producer must be fresh: its Recovery is its first frame.
    expect(
        &[
            open_native(NATIVE, 1),
            end(NATIVE, 1),
            recovery(WIRE, NATIVE),
        ],
        "recovery_producer_not_fresh",
        3,
    );
    // A stream Gap names a stream that producer actually opened.
    expect(
        &[open_wire(WIRE, 1, 1, None), gap(Some(key(WIRE, 2)))],
        "gap_on_unknown_stream",
        2,
    );
    // Within one clock, a segment's first observation cannot follow its last.
    let CaptureRecord::Segment {
        stream,
        data_b64,
        audit,
        ..
    } = segment(NATIVE, 1, 0, 0, b"abc")
    else {
        unreachable!()
    };
    let reversed = CaptureRecord::Segment {
        stream,
        part: 0,
        source_offset: 0,
        source_len: 3,
        first: at(NATIVE, 30),
        last: at(NATIVE, 20),
        data_b64,
        audit,
    };
    expect(
        &[open_native(NATIVE, 1), reversed],
        "segment_time_reversed",
        2,
    );
    // Only the producer that opened a connection may report its upgrade.
    let foreign_upgrade = CaptureRecord::Connection {
        producer: NATIVE.to_string(),
        connection: CONN.to_string(),
        observed: at(NATIVE, 6),
        event: ConnectionEvent::UpgradedWebSocket,
    };
    expect(
        &[conn_open(WIRE, false), foreign_upgrade],
        "connection_not_open",
        2,
    );

    // One chain belongs to one entry.
    let dir = tempfile::tempdir().unwrap();
    let mut export = capture_export(dir.path());
    append(&mut export, &conn_open(WIRE, false));
    let mut foreign = v5(conn_close(WIRE));
    foreign.entry_id = "entry-other".to_string();
    export.append_chain(&mut foreign).unwrap();
    drop(export);
    let (report, _) = scan(dir.path());
    assert_eq!(report.exit_code(), 2, "{report:#?}");
    assert!(has(&report, "entry_id_mismatch"), "{:?}", report.findings);
    assert_eq!(report.validated_records, 2);
}

#[test]
fn empty_optional_artifacts_read_as_absent() {
    // A zero-length seal.json is the local-finish crash window (terminal chained, fixed
    // file created, bytes not yet written): incomplete, exactly like a missing one.
    let dir = make_trace(&launched(complete_wire(None)), true);
    std::fs::write(dir.path().join(SEAL_FILE), b"").unwrap();
    let (report, _) = scan(dir.path());
    assert_eq!(report.exit_code(), 1, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Incomplete);
    assert!(
        has(&report, "seal_manifest_missing"),
        "{:?}",
        report.findings
    );
    assert!(!report.sealed);
    assert_eq!(report.terminal, None);

    // A zero-length receipt.json is an absent optional service artifact: normal local
    // capture, neither a signed anchor nor a malformed receipt.
    let dir = make_trace(&launched(complete_wire(None)), true);
    std::fs::write(dir.path().join(RECEIPT_FILE), b"").unwrap();
    let (report, _) = scan(dir.path());
    assert_eq!(report.exit_code(), 0, "{report:#?}");
    assert_eq!(report.anchor, AnchorState::LocalOnly);
    assert_eq!(report.key_source, KeySource::None);
    assert_eq!(report.receipt_clock, None);
    assert!(report.findings.is_empty(), "{:?}", report.findings);
}

#[test]
fn t00_clock_domain_golden_reads_as_recovered_coverage() {
    // The committed clock-domain record sequence, wrapped as a real v5 chain with a
    // local finish. Its recovered producer attests final_seq 3 (Recovery 0, Connection 1,
    // Open 2, End 3): gap and recovery make it incomplete, never invalid.
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../goalcup-protocol/tests/fixtures/m2-capture/clock-domains.jsonl");
    // Its Launch names the goal stream, which the golden itself does not carry.
    let mut records = goal_stream();
    records.extend(
        std::fs::read_to_string(&golden)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<CaptureRecord>(line).unwrap()),
    );
    let (report, seen) = scan(make_trace(&records, true).path());
    assert_eq!(report.exit_code(), 1, "{report:#?}");
    assert_eq!(report.integrity, CaptureIntegrity::Incomplete);
    assert_eq!(report.gap_count, 2);
    assert_eq!(
        report.findings,
        vec!["gap_recorded".to_string(), "producer_recovered".to_string()]
    );
    assert_eq!(&seen[..records.len()], &records[..]);
}

#[test]
fn callback_failure_is_an_io_error_not_a_report() {
    let dir = make_trace(&complete_wire(None), true);
    let error = scan_capture_trace(dir.path(), &VerifyOptions::default(), &mut |_| {
        Err(std::io::Error::other("consumer refused the record"))
    })
    .expect_err("a failed consumer leaves no usable report");
    assert_eq!(error.to_string(), "consumer refused the record");
}

#[test]
fn replaced_evidence_is_refused_not_followed() {
    // A chain replaced by a symbolic link to a valid chain elsewhere is not read.
    let elsewhere = make_trace(&launched(complete_wire(None)), true);
    assert_eq!(scan(elsewhere.path()).0.exit_code(), 0, "control");
    let dir = make_trace(&launched(complete_wire(None)), true);
    assert_eq!(scan(dir.path()).0.exit_code(), 0, "control");
    std::fs::remove_file(dir.path().join(CHAIN_FILE)).unwrap();
    std::os::unix::fs::symlink(
        elsewhere.path().join(CHAIN_FILE),
        dir.path().join(CHAIN_FILE),
    )
    .unwrap();
    assert!(scan_capture_trace(dir.path(), &VerifyOptions::default(), &mut |_| Ok(())).is_err());

    // Likewise the fixed manifest a terminal binds.
    let dir = make_trace(&complete_wire(None), true);
    std::fs::remove_file(dir.path().join(SEAL_FILE)).unwrap();
    std::os::unix::fs::symlink(elsewhere.path().join(SEAL_FILE), dir.path().join(SEAL_FILE))
        .unwrap();
    assert!(scan_capture_trace(dir.path(), &VerifyOptions::default(), &mut |_| Ok(())).is_err());

    // A hard link to a valid chain or seal elsewhere is a second name for bytes outside the
    // trace: refused, though following it would read a valid trace.
    let refusal = |dir: &Path| {
        scan_capture_trace(dir, &VerifyOptions::default(), &mut |_| Ok(()))
            .expect_err("replaced evidence leaves no usable report")
            .to_string()
    };
    for file in [CHAIN_FILE, SEAL_FILE] {
        let dir = make_trace(&launched(complete_wire(None)), true);
        std::fs::remove_file(dir.path().join(file)).unwrap();
        std::fs::hard_link(elsewhere.path().join(file), dir.path().join(file)).unwrap();
        let said = refusal(dir.path());
        assert!(said.contains("hard linked"), "{file}: {said}");
    }

    // A FIFO in the chain's place is not evidence, and opening it never waits for a writer.
    let dir = make_trace(&launched(complete_wire(None)), true);
    std::fs::remove_file(dir.path().join(CHAIN_FILE)).unwrap();
    let fifo = std::ffi::CString::new(std::os::unix::ffi::OsStringExt::into_vec(
        dir.path().join(CHAIN_FILE).into_os_string(),
    ))
    .unwrap();
    // SAFETY: `fifo` is a live NUL-terminated path for the duration of the call.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let said = refusal(dir.path());
    assert!(said.contains("not a regular file"), "{said}");
}

#[test]
fn exit_code_follows_canonical_precedence() {
    let base = CaptureIntegrityReport {
        schema_version: 1,
        integrity: CaptureIntegrity::Valid,
        anchor: AnchorState::LocalOnly,
        sealed: true,
        validated_records: 1,
        validated_bytes: 1,
        validated_head: None,
        gap_count: 0,
        coverage_incomplete: false,
        torn_tail: false,
        key_source: KeySource::None,
        findings: vec![],
        verified_ack: None,
        receipt_clock: None,
        terminal: None,
    };
    let with = |edit: &dyn Fn(&mut CaptureIntegrityReport)| {
        let mut report = base.clone();
        edit(&mut report);
        report.exit_code()
    };
    assert_eq!(
        base.exit_code(),
        0,
        "local-only anchoring alone never fails"
    );
    assert_eq!(with(&|r| r.integrity = CaptureIntegrity::Incomplete), 1);
    assert_eq!(with(&|r| r.sealed = false), 1);
    assert_eq!(with(&|r| r.coverage_incomplete = true), 1);
    assert_eq!(with(&|r| r.integrity = CaptureIntegrity::Invalid), 2);
    // A torn tail is invalid evidence and overrides the unsealed exit.
    assert_eq!(
        with(&|r| {
            r.torn_tail = true;
            r.sealed = false;
            r.integrity = CaptureIntegrity::Incomplete;
        }),
        2
    );
    assert!(serde_json::from_str::<CaptureIntegrity>("\"unknown\"").is_err());
}

#[test]
fn typed_reader_statuses_keep_the_schema_one_words_and_report_bytes() {
    assert_eq!(
        serde_json::to_string(&CaptureIntegrity::Valid).unwrap(),
        "\"valid\""
    );
    assert_eq!(
        serde_json::to_string(&CaptureIntegrity::Incomplete).unwrap(),
        "\"incomplete\""
    );
    assert_eq!(
        serde_json::to_string(&CaptureIntegrity::Invalid).unwrap(),
        "\"invalid\""
    );
    assert_eq!(
        serde_json::to_string(&KeySource::Pinned).unwrap(),
        "\"pinned\""
    );
    assert_eq!(
        serde_json::to_string(&KeySource::Supplied).unwrap(),
        "\"supplied\""
    );
    assert_eq!(serde_json::to_string(&KeySource::None).unwrap(), "\"none\"");

    let report = CaptureIntegrityReport {
        schema_version: 1,
        integrity: CaptureIntegrity::Valid,
        anchor: AnchorState::LocalOnly,
        sealed: true,
        validated_records: 1,
        validated_bytes: 2,
        validated_head: Some("head".into()),
        gap_count: 0,
        coverage_incomplete: false,
        torn_tail: false,
        key_source: KeySource::None,
        findings: vec![],
        verified_ack: None,
        receipt_clock: None,
        terminal: None,
    };
    assert_eq!(
        serde_json::to_string(&report).unwrap(),
        r#"{"schema_version":1,"integrity":"valid","anchor":"local_only","sealed":true,"validated_records":1,"validated_bytes":2,"validated_head":"head","gap_count":0,"coverage_incomplete":false,"torn_tail":false,"key_source":"none","findings":[],"verified_ack":null,"receipt_clock":null,"terminal":null}"#
    );
}

#[test]
fn cli_capture_integrity_mode() {
    // Complete local evidence: stdout is exactly the library document, stderr the summary.
    let dir = make_trace(&launched(complete_wire(None)), true);
    let (report, document) =
        capture_integrity_document(dir.path(), &VerifyOptions::default()).unwrap();
    let output = bin()
        .arg("--capture-integrity")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(report.exit_code(), 0);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{document}\n")
    );
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "goalcup-verify: capture integrity valid (anchor local_only, exit 0)\n"
    );

    // A torn tail: the validated-prefix report still reaches stdout, with exit 2.
    let dir = make_trace(&complete_wire(None), false);
    append_bytes(&dir.path().join(CHAIN_FILE), b"{\"protocol_version\":5");
    let output = bin()
        .arg("--capture-integrity")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let torn: CaptureIntegrityReport = serde_json::from_slice(&output.stdout).unwrap();
    assert!(torn.torn_tail);
    assert!(torn.validated_records > 0);

    // The development key option keeps its meaning and its disclosure.
    let key = server_key();
    let signed = tempfile::tempdir().unwrap();
    let mut export = capture_export(signed.path());
    for record in launched(complete_wire(None)) {
        append(&mut export, &record);
    }
    let (_, finish) = export
        .write_local_finish(&manifest(), finish_time(), "participant_stop", confirmed())
        .unwrap();
    export
        .append_ack_synced(&sign_ack(&key, &finish, "2026-09-27T12:01:00Z"))
        .unwrap();
    drop(export);
    let output = bin()
        .arg("--capture-integrity")
        .arg(signed.path())
        .arg("--dev-key")
        .arg(hex::encode(key.verifying_key().to_bytes()))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let keyed: CaptureIntegrityReport = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(keyed.key_source, KeySource::Supplied);
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("--dev-key")
    );

    // Conflicting modes are usage errors (3), refused before any output file is opened.
    let out = tempfile::tempdir().unwrap();
    let json = out.path().join("report.json");
    let output = bin()
        .arg("--capture-integrity")
        .arg(dir.path())
        .arg("--json")
        .arg(&json)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    assert!(!json.exists());
    let output = bin()
        .arg(dir.path())
        .arg("--capture-integrity")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());

    // No trace, no usable report.
    let output = bin()
        .arg("--capture-integrity")
        .arg(out.path().join("absent"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
}

/// D53: a recorded harness build is read for its shape only. Canonical digests and a bounded
/// platform pass; a malformed digest is a malformed record, never a verdict on the build.
#[test]
fn verifier_reads_a_recorded_harness_build_for_its_shape_only() {
    let with = |build: goalcup_protocol::capture::HarnessBuildV1| {
        let mut records = launched(complete_wire(None));
        if let Some(CaptureRecord::Launch { harness_build, .. }) = records
            .iter_mut()
            .find(|record| matches!(record, CaptureRecord::Launch { .. }))
        {
            *harness_build = Some(Box::new(build));
        }
        scan(make_trace(&records, true).path()).0
    };
    let good = with(goalcup_protocol::capture::HarnessBuildV1 {
        program_sha256: Some("e".repeat(64)),
        image_sha256: None,
        platform: "linux-x86_64".into(),
    });
    assert_eq!(good.exit_code(), 0, "{good:#?}");
    for (program, platform) in [(Some("E".repeat(64)), "macos-aarch64"), (None, "")] {
        let bad = with(goalcup_protocol::capture::HarnessBuildV1 {
            program_sha256: program,
            image_sha256: None,
            platform: platform.into(),
        });
        assert_eq!(bad.exit_code(), 2, "{bad:#?}");
        assert!(has(&bad, "record_field_invalid"), "{bad:#?}");
    }
}

// ---------------------------------------------------------------------------------------
// The chain as rotating parts (D55).
// ---------------------------------------------------------------------------------------

/// Split a closed single-file chain into part files before the given line indices: the names
/// and bytes a rotating writer leaves, because rotation never changes a byte of the chain.
fn split_into_parts(dir: &Path, cuts: &[usize]) -> Vec<PathBuf> {
    let bytes = std::fs::read(dir.join(CHAIN_FILE)).unwrap();
    let lines: Vec<&[u8]> = bytes.split_inclusive(|byte| *byte == b'\n').collect();
    let mut bounds = vec![0];
    bounds.extend_from_slice(cuts);
    bounds.push(lines.len());
    let mut start = 0u64;
    let mut paths = Vec::new();
    for pair in bounds.windows(2) {
        let part = lines[pair[0]..pair[1]].concat();
        let path = dir.join(part_file_name(start));
        std::fs::write(&path, &part).unwrap();
        start += part.len() as u64;
        paths.push(path);
    }
    paths
}

#[test]
fn a_rotated_chain_verifies_like_one_file() {
    let one = make_trace(&launched(complete_wire(None)), true);
    let rotated = make_trace(&launched(complete_wire(None)), true);
    let parts = split_into_parts(rotated.path(), &[1, 4, 9]);
    assert_eq!(parts.len(), 4);

    let (single, single_records) = scan(one.path());
    let (split, split_records) = scan(rotated.path());
    assert_eq!(single.exit_code(), 0, "{single:#?}");
    assert_eq!(
        serde_json::to_value(&split).unwrap(),
        serde_json::to_value(&single).unwrap()
    );
    assert_eq!(split_records, single_records);

    // A part missing between two others is a chain with a hole in it.
    std::fs::remove_file(&parts[2]).unwrap();
    let (holed, _) = scan(rotated.path());
    assert_eq!(holed.integrity, CaptureIntegrity::Invalid, "{holed:#?}");
    assert!(has(&holed, "chain_part_missing"), "{holed:#?}");
}

#[test]
fn a_chain_missing_its_first_part_says_verify_the_server_copy() {
    let key = server_key();
    let options = supplied(&key);
    let dir = tempfile::tempdir().unwrap();
    let mut export = capture_export(dir.path());
    let links: Vec<Link> = launched(complete_wire(None))
        .iter()
        .map(|record| append(&mut export, record))
        .collect();
    let (_, finish) = export
        .write_local_finish(&manifest(), finish_time(), "participant_stop", confirmed())
        .unwrap();
    // One acknowledgement inside the part removed below, one after it.
    export
        .append_ack_synced(&sign_ack(&key, &links[1], "2026-09-27T12:01:00Z"))
        .unwrap();
    export
        .append_ack_synced(&sign_ack(&key, &links[9], "2026-09-27T12:01:30Z"))
        .unwrap();
    drop(export);
    let chain = chain_lines(dir.path());
    let total = chain_len(dir.path());
    let parts = split_into_parts(dir.path(), &[4, 8]);
    std::fs::remove_file(&parts[0]).unwrap();

    let (report, _) = scan_with(dir.path(), &options);
    assert_eq!(
        report.integrity,
        CaptureIntegrity::Incomplete,
        "{report:#?}"
    );
    assert_eq!(report.exit_code(), 1);
    assert_eq!(
        report.findings,
        findings(&["chain_pruned_locally", "server_key_unpinned"])
    );
    // What survived is still checked link by link, through the bound terminal.
    assert!(report.sealed, "{report:#?}");
    assert_eq!(report.validated_records, chain.len() as u64 - 4);
    assert_eq!(report.validated_head, Some(finish.hash.clone()));
    assert_eq!(
        report.terminal.map(|terminal| terminal.hash),
        Some(finish.hash)
    );
    assert_eq!(report.validated_bytes, total, "offsets stay logical");
    // The acknowledgement of a pruned record is skipped, not refused.
    assert_eq!(report.verified_ack.map(|ack| ack.seq), Some(links[9].seq));

    let output = bin()
        .arg("--capture-integrity")
        .arg(dir.path())
        .arg("--dev-key")
        .arg(hex::encode(key.verifying_key().to_bytes()))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("pruned on this machine: verify the server copy"),
        "{stderr}"
    );
}
