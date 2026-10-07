//! The protocol-v5 capture chain's quota, durability and acknowledgement storage.
//!
//! Every fixture is a real `TraceExport` in a temporary directory and every assertion reads
//! the bytes back off disk. The quota here is serialized-byte accounting only: nothing in this
//! file claims, or may claim, a physical reservation (that is `capture_reserved_storage.rs`'s).

use goalcup_platform::tracecap::{RefusalKind, TraceCapError};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::capture::{
    CaptureRecord, CaptureTime, GapReason, M2_PROTOCOL_VERSION, MAX_ENVELOPE_BYTES, StreamKey,
    TERMINAL_RESERVE_BYTES,
};
use goalcup_protocol::chain::Link;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::export::*;
use goalcup_protocol::provenance::{Rfc3339Utc, TerminationOutcomeV1};
use goalcup_protocol::redaction::{RedactionAudit, RedactionAuditSpan};
use goalcup_protocol::seal::{SealManifest, SealedFile};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

const ENTRY: &str = "entry-m2-0001";
const PRODUCER: &str = "0123456789abcdef0123456789abcdef";
const DAEMON_CLOCK: &str = "daemon-5b0c3a1e";
/// A small experimental quota: 16 KiB of payload in front of the protected terminal portion.
const SMALL_QUOTA: u64 = TERMINAL_RESERVE_BYTES + 16 * 1024;
const LARGE_QUOTA: u64 = 64 * 1024 * 1024;

fn at(mono_ns: u64) -> CaptureTime {
    CaptureTime {
        wall: "2026-09-27T12:00:00.250Z".to_string(),
        mono_ns,
        clock_id: DAEMON_CLOCK.to_string(),
    }
}

fn key(stream: u64) -> StreamKey {
    StreamKey {
        producer: PRODUCER.to_string(),
        stream,
    }
}

fn manifest() -> SealManifest {
    SealManifest {
        entry_id: ENTRY.to_string(),
        sealed_at: "2026-09-27T12:00:00Z".to_string(),
        workspace: "/ws".to_string(),
        files: vec![SealedFile {
            path: "src/main.rs".to_string(),
            bytes: 12,
            sha256: "ab".repeat(32),
        }],
        excluded: Vec::new(),
    }
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

/// A sanitized segment whose base64 text is exactly `data_len` bytes long.
fn segment(part: u64, data_len: usize) -> CaptureRecord {
    CaptureRecord::Segment {
        stream: key(1),
        part,
        source_offset: part * 1024,
        source_len: 1024,
        first: at(10),
        last: at(20),
        data_b64: "A".repeat(data_len),
        audit: RedactionAudit {
            spans: vec![RedactionAuditSpan {
                kind: "bearer".to_string(),
                start: 4,
                end: 12,
            }],
        },
    }
}

fn gap(mono_ns: u64) -> CaptureRecord {
    CaptureRecord::Gap {
        stream: None,
        observed: at(mono_ns),
        reason: GapReason::StorageFull,
        lost_bytes: None,
    }
}

fn stopped() -> TerminationOutcomeV1 {
    TerminationOutcomeV1::AlreadyStopped {
        checked_at: Rfc3339Utc::try_from("2026-09-27T12:00:02Z").unwrap(),
    }
}

/// A fresh protocol-v5 capture chain: quota selected before its v5 baseline, as the M2
/// create path does.
fn capture_export(root: &Path, quota: u64) -> TraceExport {
    let mut export = TraceExport::create(root).unwrap();
    export.configure_capture_quota(quota).unwrap();
    export
        .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest())
        .unwrap();
    export
}

fn chain(root: &Path) -> Vec<u8> {
    std::fs::read(root.join(CHAIN_FILE)).unwrap()
}

fn chain_len(root: &Path) -> u64 {
    std::fs::metadata(root.join(CHAIN_FILE)).unwrap().len()
}

fn snapshot(root: &Path) -> Vec<(std::ffi::OsString, Vec<u8>)> {
    let mut entries = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), std::fs::read(entry.path()).unwrap())
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

/// The exact JSONL line length the writer will produce for `env` at `seq`: every link field
/// is fixed width (64 hex characters), so the length does not depend on the hash's value.
fn line_len(seq: u64, env: &Envelope) -> usize {
    let mut stamped = env.clone();
    stamped.link = Link {
        seq,
        prev: "0".repeat(64),
        hash: "0".repeat(64),
    };
    serde_json::to_vec(&stamped).unwrap().len() + 1
}

fn append_raw(root: &Path, bytes: &[u8]) {
    use std::io::Write as _;
    std::fs::OpenOptions::new()
        .append(true)
        .open(root.join(CHAIN_FILE))
        .unwrap()
        .write_all(bytes)
        .unwrap();
}

/// A correctly linked next line for a chain whose head is `head`, as bytes with newline.
fn linked_line(head: &(u64, String), mut env: Envelope) -> Vec<u8> {
    env.link.seq = head.0 + 1;
    env.link.prev = head.1.clone();
    env.link.hash = env.expected_hash(&head.1).unwrap();
    let mut line = serde_json::to_vec(&env).unwrap();
    line.push(b'\n');
    line
}

#[test]
fn quota_does_not_claim_physical_reservation() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    export.configure_capture_quota(SMALL_QUOTA).unwrap();

    let storage = export.capture_storage();
    assert_eq!(storage.quota_bytes, SMALL_QUOTA);
    assert_eq!(storage.reserved_bytes, 0, "a quota is not a reservation");
    assert_eq!(storage.used_bytes, 0);
    assert_eq!(storage.durable, None);
    assert!(storage.accounting_available);
    let chain_meta = std::fs::metadata(tmp.path().join(CHAIN_FILE)).unwrap();
    assert_eq!(chain_meta.len(), 0, "configuring a quota writes nothing");
    assert_eq!(
        chain_meta.blocks(),
        0,
        "configuring a quota allocates no block behind the chain"
    );

    export
        .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest())
        .unwrap();
    export.append_chain(&mut v5(segment(0, 512))).unwrap();
    export.sync_capture().unwrap();

    let storage = export.capture_storage();
    let chain_meta = std::fs::metadata(tmp.path().join(CHAIN_FILE)).unwrap();
    assert!(
        chain_meta.blocks() > 0,
        "ordinary writes did allocate filesystem blocks"
    );
    assert_eq!(
        storage.reserved_bytes, 0,
        "allocated blocks from ordinary writes are never reported as reserved capacity"
    );
    assert_eq!(storage.used_bytes, chain_meta.len());
    assert!(
        storage.used_bytes < storage.quota_bytes,
        "the logical chain is not extended toward the quota"
    );
    let status = serde_json::to_value(&storage).unwrap();
    assert_eq!(status["reserved_bytes"], 0);
    assert_eq!(status["quota_bytes"], SMALL_QUOTA);
}

#[test]
fn invalid_quota_refused_before_append() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    for bad in [0, 1, TERMINAL_RESERVE_BYTES] {
        let refused = export.configure_capture_quota(bad);
        assert!(
            matches!(refused, Err(ExportError::InvalidCaptureQuota { bytes, .. }) if bytes == bad),
            "a {bad}-byte quota leaves no payload room in front of the terminal allowance: {refused:?}"
        );
    }
    assert_eq!(export.capture_storage().quota_bytes, 0);
    // Nothing was selected, so a v5 record is still an unsupported version for this export.
    assert!(matches!(
        export.write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest()),
        Err(ExportError::UnsupportedProtocolVersion {
            protocol_version: 5,
            ..
        })
    ));
    assert!(chain(tmp.path()).is_empty(), "no byte reached the chain");

    // A legacy chain that already holds a record cannot be relabelled as a capture chain.
    let legacy = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(legacy.path()).unwrap();
    export.write_baseline(&manifest()).unwrap();
    let before = snapshot(legacy.path());
    assert!(matches!(
        export.configure_capture_quota(SMALL_QUOTA),
        Err(ExportError::InvalidCaptureQuota { .. })
    ));
    assert!(matches!(
        export.append_chain(&mut v5(gap(1))),
        Err(ExportError::UnsupportedProtocolVersion {
            protocol_version: 5,
            ..
        })
    ));
    assert_eq!(snapshot(legacy.path()), before);
}

#[test]
fn oversized_envelope_leaves_head_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path(), LARGE_QUOTA);
    let head = export.head().unwrap();
    let before = chain(tmp.path());
    let seq = head.0 + 1;

    // The bound counts the complete serialized envelope including its trailing newline.
    let base = line_len(seq, &v5(segment(0, 0)));
    let exact = v5(segment(0, MAX_ENVELOPE_BYTES - base));
    assert_eq!(line_len(seq, &exact), MAX_ENVELOPE_BYTES);
    let mut over = v5(segment(0, MAX_ENVELOPE_BYTES - base + 1));

    let refused = export.append_chain(&mut over);
    assert!(
        matches!(
            refused,
            Err(ExportError::EnvelopeTooLarge { bytes, limit })
                if bytes == MAX_ENVELOPE_BYTES as u64 + 1 && limit == MAX_ENVELOPE_BYTES as u64
        ),
        "{refused:?}"
    );
    assert_eq!(export.head(), Some(head.clone()), "the head did not move");
    assert_eq!(chain(tmp.path()), before, "no byte reached the chain");
    assert_eq!(export.capture_storage().used_bytes, before.len() as u64);
    assert!(
        export.capture_storage().accounting_available,
        "a refused oversized envelope is not an accounting failure"
    );

    let link = export.append_chain(&mut exact.clone()).unwrap();
    assert_eq!(link.seq, seq, "the refusal consumed no sequence number");
    assert_eq!(
        chain_len(tmp.path()),
        before.len() as u64 + MAX_ENVELOPE_BYTES as u64,
        "an envelope of exactly the bound is admitted, newline included"
    );
}

#[test]
fn payload_cannot_consume_terminal_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path(), SMALL_QUOTA);
    let mut part = 0;
    loop {
        match export.append_chain(&mut v5(segment(part, 1024))) {
            Ok(_) => part += 1,
            Err(ExportError::CaptureStorageFull {
                needed,
                available,
                terminal_allowance: false,
            }) => {
                assert!(needed > available);
                break;
            }
            Err(other) => panic!("payload exhaustion is CaptureStorageFull: {other:?}"),
        }
    }
    assert!(part > 0, "some payload fitted before the refusal");
    let used = export.capture_storage().used_bytes;
    assert!(
        used <= SMALL_QUOTA - TERMINAL_RESERVE_BYTES,
        "payload stopped before the terminal allowance: {used}"
    );
    assert_eq!(used, chain_len(tmp.path()));
    assert!(
        export.capture_storage().accounting_available,
        "payload refusal is expected loss, recorded by a closure, not an accounting failure"
    );

    // Bounded closures still fit: the loss gap, the stream end and the producer closure.
    export.append_chain(&mut v5(gap(30))).unwrap();
    export
        .append_chain(&mut v5(CaptureRecord::End {
            stream: key(1),
            observed: at(31),
            reason: goalcup_protocol::capture::EndReason::Interrupted,
        }))
        .unwrap();
    export
        .append_chain(&mut v5(CaptureRecord::ProducerClosed {
            producer: PRODUCER.to_string(),
            observed: at(32),
            final_seq: Some(41),
        }))
        .unwrap();
    // A payload record is still refused after the closures ...
    assert!(matches!(
        export.append_chain(&mut v5(segment(part, 16))),
        Err(ExportError::CaptureStorageFull {
            terminal_allowance: false,
            ..
        })
    ));
    // ... and the local terminal still fits in the protected portion.
    let (_, terminal) = export
        .write_local_finish(&manifest(), at(40), "participant_stop", stopped())
        .unwrap();
    assert_eq!(export.head().unwrap().0, terminal.seq);
    let storage = export.capture_storage();
    assert!(storage.used_bytes <= SMALL_QUOTA);
    assert!(storage.accounting_available);
}

#[test]
fn terminal_budget_is_finite() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path(), SMALL_QUOTA);
    let mut part = 0;
    while export.append_chain(&mut v5(segment(part, 1024))).is_ok() {
        part += 1;
    }

    // One gap per discarded frame would exhaust the terminal allowance; the writer stops it
    // at the quota instead of growing without bound.
    let mut gaps = 0u64;
    loop {
        match export.append_chain(&mut v5(gap(100 + gaps))) {
            Ok(_) => gaps += 1,
            Err(ExportError::CaptureStorageFull {
                terminal_allowance: true,
                needed,
                available,
            }) => {
                assert!(needed > available);
                break;
            }
            Err(other) => panic!("closure exhaustion is CaptureStorageFull: {other:?}"),
        }
        assert!(gaps < 1_000_000, "the terminal allowance is finite");
    }
    assert!(gaps > 0);
    let storage = export.capture_storage();
    assert!(storage.used_bytes <= SMALL_QUOTA, "{}", storage.used_bytes);
    assert!(
        storage.used_bytes > SMALL_QUOTA - TERMINAL_RESERVE_BYTES,
        "the closures did spend the protected portion"
    );
    assert_eq!(storage.used_bytes, chain_len(tmp.path()));
    assert!(
        !storage.accounting_available,
        "once even a closure is refused, loss can no longer be accounted"
    );

    let before = snapshot(tmp.path());
    assert!(matches!(
        export.write_local_finish(&manifest(), at(9_000), "participant_stop", stopped()),
        Err(ExportError::CaptureStorageFull {
            terminal_allowance: true,
            ..
        })
    ));
    assert_eq!(
        snapshot(tmp.path()),
        before,
        "a refused terminal writes neither a link nor seal.json"
    );
    assert!(
        !export.capture_storage().accounting_available,
        "accounting unavailability is sticky"
    );
    assert!(
        matches!(
            export.configure_capture_quota(TERMINAL_RESERVE_BYTES + 1),
            Err(ExportError::InvalidCaptureQuota { .. })
        ),
        "a quota cannot be lowered below the chain bytes already written"
    );
}

#[test]
fn append_head_does_not_advance_durable_frontier() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    export.configure_capture_quota(LARGE_QUOTA).unwrap();
    assert_eq!(
        export.sync_capture().unwrap(),
        None,
        "an empty chain has no durable frontier"
    );

    export
        .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest())
        .unwrap();
    assert_eq!(
        export.capture_storage().durable,
        None,
        "an appended head is not a durable frontier"
    );
    let first = export.sync_capture().unwrap().unwrap();
    assert_eq!(first.seq, 0);
    assert_eq!(first.hash, export.head().unwrap().1);
    assert_eq!(first.chain_bytes, chain_len(tmp.path()));
    assert_eq!(export.capture_storage().durable, Some(first.clone()));

    export.append_chain(&mut v5(segment(0, 64))).unwrap();
    export.append_chain(&mut v5(segment(1, 64))).unwrap();
    assert_eq!(export.head().unwrap().0, 2);
    assert_eq!(
        export.capture_storage().durable,
        Some(first.clone()),
        "appends move the head, never the durable frontier"
    );

    let second = export.sync_capture().unwrap().unwrap();
    assert_eq!(second.seq, 2);
    assert_eq!(second.hash, export.head().unwrap().1);
    assert_eq!(second.chain_bytes, chain_len(tmp.path()));
    assert_eq!(export.capture_storage().durable, Some(second));
    drop(export);

    // A reopened writer has synced nothing yet, and its quota is configured again explicitly.
    let mut resumed = TraceExport::resume(tmp.path()).unwrap();
    let storage = resumed.capture_storage();
    assert_eq!(storage.durable, None);
    assert_eq!(storage.used_bytes, chain_len(tmp.path()));
    assert_eq!(storage.quota_bytes, 0);
    assert!(matches!(
        resumed.append_chain(&mut v5(segment(2, 64))),
        Err(ExportError::CaptureQuotaNotConfigured)
    ));
    resumed.configure_capture_quota(LARGE_QUOTA).unwrap();
    assert_eq!(
        resumed.append_chain(&mut v5(segment(2, 64))).unwrap().seq,
        3
    );
}

fn ack(seq: u64, hash: &str) -> AckCheckpoint {
    AckCheckpoint {
        entry_id: ENTRY.to_string(),
        seq,
        hash: hash.to_string(),
        acked_at: "2026-09-27T12:00:03Z".to_string(),
        server_signature: "0".repeat(128),
    }
}

fn ack_line(ack: &AckCheckpoint) -> Vec<u8> {
    let mut line = serde_json::to_vec(ack).unwrap();
    line.push(b'\n');
    line
}

#[test]
fn synced_acknowledgements_are_read_back_through_the_retained_file() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path(), LARGE_QUOTA);
    export.append_chain(&mut v5(segment(0, 64))).unwrap();
    let frontier = export.sync_capture().unwrap().unwrap();
    let first = ack(0, &frontier.hash);
    let second = ack(frontier.seq, &frontier.hash);
    export.append_ack_synced(&first).unwrap();
    export.append_ack_synced(&second).unwrap();

    let mut read = Vec::new();
    export
        .independent_acks_reader()
        .unwrap()
        .read_to_end(&mut read)
        .unwrap();
    let expected = [ack_line(&first), ack_line(&second)].concat();
    assert_eq!(read, expected);
    assert_eq!(std::fs::read(tmp.path().join(ACKS_FILE)).unwrap(), expected);
    assert_eq!(
        export.capture_storage().used_bytes,
        chain_len(tmp.path()),
        "acknowledgements are not chain bytes and spend no capture quota"
    );
}

#[test]
fn ack_sync_failure_is_not_delivery_commit() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path(), LARGE_QUOTA);
    let frontier = export.sync_capture().unwrap().unwrap();
    let committed = ack(0, &frontier.hash);
    export.append_ack_synced(&committed).unwrap();

    // Replace the retained acknowledgement file's name with a different file.
    let acks = tmp.path().join(ACKS_FILE);
    let detached = tmp.path().join("acks.detached");
    std::fs::rename(&acks, &detached).unwrap();
    std::fs::write(&acks, b"").unwrap();

    let refused = export.append_ack_synced(&ack(0, &frontier.hash));
    assert!(
        matches!(
            refused,
            Err(ExportError::Capability(TraceCapError::Refused {
                reason: RefusalKind::IdentityChanged,
                ..
            }))
        ),
        "a commit that is not durable under acks.jsonl is refused: {refused:?}"
    );
    assert_eq!(
        std::fs::read(&acks).unwrap(),
        b"",
        "the named acknowledgement stream never shows the refused commit"
    );
    // Bytes that reached the detached file are preserved, not rolled back or claimed.
    assert!(
        std::fs::read(&detached)
            .unwrap()
            .starts_with(&ack_line(&committed))
    );

    // The writer stays refused even when the original name is restored.
    std::fs::remove_file(&acks).unwrap();
    std::fs::rename(&detached, &acks).unwrap();
    assert!(matches!(
        export.append_ack_synced(&committed),
        Err(ExportError::Capability(TraceCapError::Poisoned { .. }))
    ));
    assert!(export.independent_acks_reader().is_err());
}

#[test]
fn replaced_acks_path_refuses_the_reader_and_the_next_commit() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path(), LARGE_QUOTA);
    let frontier = export.sync_capture().unwrap().unwrap();
    export.append_ack_synced(&ack(0, &frontier.hash)).unwrap();

    let acks = tmp.path().join(ACKS_FILE);
    std::fs::rename(&acks, tmp.path().join("acks.old")).unwrap();
    std::fs::write(&acks, b"planted\n").unwrap();

    assert!(matches!(
        export.independent_acks_reader(),
        Err(ExportError::Capability(TraceCapError::Refused {
            reason: RefusalKind::IdentityChanged,
            ..
        }))
    ));
    assert!(matches!(
        export.append_ack_synced(&ack(0, &frontier.hash)),
        Err(ExportError::Capability(TraceCapError::Poisoned { .. }))
    ));
    assert_eq!(std::fs::read(&acks).unwrap(), b"planted\n");
}

#[test]
fn torn_final_line_preserved_and_resume_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path(), LARGE_QUOTA);
    export.append_chain(&mut v5(segment(0, 64))).unwrap();
    let head = export.head().unwrap();
    export.sync_capture().unwrap();
    drop(export);

    // A crash mid-append: a correctly linked next record without its terminating newline.
    let whole = linked_line(&head, v5(gap(50)));
    append_raw(tmp.path(), &whole[..whole.len() / 2]);
    let before = snapshot(tmp.path());

    assert!(matches!(
        TraceExport::resume(tmp.path()),
        Err(ExportError::ChainCorrupt { .. })
    ));
    assert!(matches!(
        TraceExport::prepare_resume_from_root(TraceRoot::open(tmp.path()).unwrap()),
        Err(ExportError::ChainCorrupt { .. })
    ));
    assert_eq!(
        snapshot(tmp.path()),
        before,
        "the torn bytes are preserved: no truncation, repair or quarantine"
    );

    // A complete record missing only its newline is the same torn tail.
    let tmp = tempfile::tempdir().unwrap();
    let export = capture_export(tmp.path(), LARGE_QUOTA);
    let head = export.head().unwrap();
    drop(export);
    let whole = linked_line(&head, v5(gap(51)));
    append_raw(tmp.path(), &whole[..whole.len() - 1]);
    let before = snapshot(tmp.path());
    assert!(matches!(
        TraceExport::resume(tmp.path()),
        Err(ExportError::ChainCorrupt { .. })
    ));
    assert_eq!(snapshot(tmp.path()), before);
}

#[test]
fn malformed_terminated_capture_records_fail_resume() {
    // A newline-terminated malformed record before an otherwise valid suffix.
    let tmp = tempfile::tempdir().unwrap();
    let export = capture_export(tmp.path(), LARGE_QUOTA);
    let head = export.head().unwrap();
    drop(export);
    append_raw(tmp.path(), b"{\"protocol_version\":5,\"seq\":1}\n");
    append_raw(tmp.path(), &linked_line(&head, v5(gap(60))));
    let before = snapshot(tmp.path());
    assert!(matches!(
        TraceExport::resume(tmp.path()),
        Err(ExportError::ChainCorrupt { seq: 1, .. })
    ));
    assert_eq!(snapshot(tmp.path()), before);

    // A terminated record whose sanitized bytes changed after hashing.
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path(), LARGE_QUOTA);
    export.append_chain(&mut v5(segment(0, 64))).unwrap();
    export.append_chain(&mut v5(gap(61))).unwrap();
    drop(export);
    let text = String::from_utf8(chain(tmp.path())).unwrap();
    let tampered = text.replacen(&"A".repeat(64), &"B".repeat(64), 1);
    assert_ne!(tampered, text);
    std::fs::write(tmp.path().join(CHAIN_FILE), &tampered).unwrap();
    assert!(matches!(
        TraceExport::resume(tmp.path()),
        Err(ExportError::ChainCorrupt { seq: 1, .. })
    ));
    assert_eq!(chain(tmp.path()), tampered.as_bytes());
}

#[test]
fn capture_chains_admit_only_protocol_five_capture_bodies() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path(), LARGE_QUOTA);
    let before = chain(tmp.path());

    let mut legacy = v5(gap(70));
    legacy.protocol_version = goalcup_protocol::PROTOCOL_VERSION;
    legacy.body = EventBody::Leg1Record {
        store: "main".to_string(),
        payload: serde_json::json!({"legacy": true}),
    };
    assert!(matches!(
        export.append_chain(&mut legacy),
        Err(ExportError::InvalidCaptureEnvelope { .. })
    ));
    assert!(matches!(
        export.write_seal(&manifest()),
        Err(ExportError::InvalidCaptureEnvelope { .. })
    ));

    let mut leg_body_v5 = v5(gap(71));
    leg_body_v5.body = EventBody::Leg1Record {
        store: "main".to_string(),
        payload: serde_json::json!({"legacy": true}),
    };
    assert!(matches!(
        export.append_chain(&mut leg_body_v5),
        Err(ExportError::InvalidCaptureEnvelope { .. })
    ));

    let mut inline_audit = v5(gap(72));
    inline_audit.redaction = Some(RedactionAudit { spans: Vec::new() });
    assert!(matches!(
        export.append_chain(&mut inline_audit),
        Err(ExportError::InvalidCaptureEnvelope { .. })
    ));

    let mut future = v5(gap(73));
    future.protocol_version = 6;
    assert!(matches!(
        export.append_chain(&mut future),
        Err(ExportError::UnsupportedProtocolVersion {
            protocol_version: 6,
            ..
        })
    ));
    assert_eq!(chain(tmp.path()), before, "no refusal reached the chain");

    // A hand-linked legacy record inside a capture chain does not resume either.
    let head = export.head().unwrap();
    drop(export);
    let mut legacy = v5(gap(74));
    legacy.protocol_version = goalcup_protocol::PROTOCOL_VERSION;
    legacy.body = EventBody::Leg1Record {
        store: "main".to_string(),
        payload: serde_json::json!({"legacy": true}),
    };
    append_raw(tmp.path(), &linked_line(&head, legacy));
    assert!(matches!(
        TraceExport::resume(tmp.path()),
        Err(ExportError::InvalidCaptureEnvelope { seq: 1, .. })
    ));
}

#[test]
fn resume_refuses_capture_lines_over_the_envelope_bound() {
    let tmp = tempfile::tempdir().unwrap();
    let export = capture_export(tmp.path(), LARGE_QUOTA);
    let head = export.head().unwrap();
    drop(export);
    let oversized = linked_line(&head, v5(segment(0, MAX_ENVELOPE_BYTES)));
    assert!(oversized.len() > MAX_ENVELOPE_BYTES);
    append_raw(tmp.path(), &oversized);
    let before = snapshot(tmp.path());
    assert!(matches!(
        TraceExport::resume(tmp.path()),
        Err(ExportError::ChainCorrupt { seq: 1, .. })
    ));
    assert_eq!(snapshot(tmp.path()), before);
}

#[test]
fn capture_chain_begins_with_its_baseline() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    export.configure_capture_quota(LARGE_QUOTA).unwrap();
    assert!(matches!(
        export.append_chain(&mut v5(gap(1))),
        Err(ExportError::InvalidCaptureEnvelope { seq: 0, .. })
    ));
    assert!(matches!(
        export.write_local_finish(&manifest(), at(2), "participant_stop", stopped()),
        Err(ExportError::InvalidCaptureEnvelope { .. })
    ));
    assert!(chain(tmp.path()).is_empty(), "no byte reached the chain");
    assert_eq!(export.head(), None);
    drop(export);

    // A hand-written v5 chain whose genesis is a capture record does not resume.
    let mut genesis = v5(gap(3));
    genesis.link.prev = goalcup_protocol::chain::GENESIS.to_string();
    genesis.link.hash = genesis.expected_hash(&genesis.link.prev).unwrap();
    let mut line = serde_json::to_vec(&genesis).unwrap();
    line.push(b'\n');
    append_raw(tmp.path(), &line);
    let before = snapshot(tmp.path());
    assert!(matches!(
        TraceExport::resume(tmp.path()),
        Err(ExportError::InvalidCaptureEnvelope { seq: 0, .. })
    ));
    assert_eq!(snapshot(tmp.path()), before);
}
