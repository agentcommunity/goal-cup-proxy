//! `TraceExport` owns the bytes of a trace directory, so every assertion here reads the
//! bytes back off disk. A test that only checks the returned `Link` would pass against a
//! writer that never wrote a file.

use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::capture::{CaptureRecord, CaptureTime, GapReason, M2_PROTOCOL_VERSION};
use goalcup_protocol::chain::{GENESIS, Link, link_hash};
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::export::*;
use goalcup_protocol::receipt::{StartReceipt, StartReceiptClaims};
use goalcup_protocol::redaction::RedactionAudit;
use goalcup_protocol::seal::{SealManifest, SealedFile};
use std::io::Read;
use std::path::Path;

const V5: u32 = M2_PROTOCOL_VERSION;
/// Room for the long-chain test's 100,000 records; admission accounting only, nothing is
/// allocated.
const QUOTA: u64 = 256 * 1024 * 1024;

/// One ordinary v5 record: a daemon gap, distinct per `n`.
fn env(n: u64) -> Envelope {
    reserved(
        "2026-09-04T00:00:00Z",
        EventBody::Capture {
            record: CaptureRecord::Gap {
                stream: None,
                observed: CaptureTime {
                    wall: "2026-09-04T00:00:00Z".to_string(),
                    mono_ns: n,
                    clock_id: "daemon-5b0c3a1e".to_string(),
                },
                reason: GapReason::StorageIo,
                lost_bytes: None,
            },
        },
    )
}

/// An unstamped v5 daemon envelope carrying `body`, recorded at `at`.
fn reserved(at: &str, body: EventBody) -> Envelope {
    Envelope {
        protocol_version: V5,
        entry_id: "e1".to_string(),
        source: "daemon".to_string(),
        recorded_at: at.to_string(),
        link: Link {
            seq: 0,
            prev: String::new(),
            hash: String::new(),
        },
        body,
        redaction: None,
    }
}

/// A fresh export with its quota set and nothing chained.
fn capture(root: &Path) -> TraceExport {
    let mut export = TraceExport::create(root).unwrap();
    export.configure_capture_quota(QUOTA).unwrap();
    export
}

/// A fresh export whose baseline is chained at seq 0.
fn started(root: &Path) -> (TraceExport, Link) {
    let mut export = capture(root);
    let (_, link) = export.write_baseline_for_protocol(V5, &manifest()).unwrap();
    (export, link)
}

/// Resume `root` and set the quota again, as the daemon does after a restart.
fn reopen(root: &Path) -> TraceExport {
    let mut export = TraceExport::resume(root).unwrap();
    export.configure_capture_quota(QUOTA).unwrap();
    export
}

fn chain_lines(root: &Path) -> usize {
    std::fs::read_to_string(root.join(CHAIN_FILE))
        .unwrap()
        .lines()
        .count()
}

fn regular_file_snapshot(root: &std::path::Path) -> Vec<(std::ffi::OsString, Vec<u8>)> {
    let mut snapshot = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            assert!(entry.file_type().unwrap().is_file());
            (entry.file_name(), std::fs::read(entry.path()).unwrap())
        })
        .collect::<Vec<_>>();
    snapshot.sort_by(|left, right| left.0.cmp(&right.0));
    snapshot
}

fn manifest() -> SealManifest {
    SealManifest {
        entry_id: "e1".to_string(),
        sealed_at: "2026-09-08T00:00:00Z".to_string(),
        workspace: "/ws".to_string(),
        files: Vec::new(),
        excluded: Vec::new(),
        git: None,
        home: None,
    }
}

fn receipt() -> StartReceipt {
    StartReceipt {
        claims: StartReceiptClaims {
            entry_id: "e1".to_string(),
            connector_id: "claude-code-cli".to_string(),
            workspace: "/ws".to_string(),
            session_id: "s1".to_string(),
            started_at: "2026-09-04T00:00:00Z".to_string(),
            deadline_at: "2026-09-08T00:00:00Z".to_string(),
            manifest_sha256: "cc".repeat(32),
            chain_head: GENESIS.to_string(),
            connector_dir_sha256: Some("ee".repeat(32)),
            run_public_key: None,
            questionnaire_version: None,
        },
        public_key: "dd".repeat(32),
        signature: "ee".repeat(64),
    }
}

#[cfg(unix)]
#[test]
fn fixed_document_write_stays_bound_to_the_acquired_root_after_path_exchange() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("trace");
    let detached = tmp.path().join("detached");
    let x = TraceExport::create(&root).unwrap();

    std::fs::rename(&root, &detached).unwrap();
    std::fs::create_dir(&root).unwrap();
    x.write_receipt(&receipt()).unwrap();

    assert!(detached.join(RECEIPT_FILE).is_file());
    assert!(!root.join(RECEIPT_FILE).exists());
}

#[cfg(unix)]
#[test]
fn finalized_fixed_document_read_stays_bound_to_the_acquired_root_after_path_exchange() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("trace");
    let detached = tmp.path().join("detached");
    let x = TraceExport::create(&root).unwrap();
    let expected = serde_json::to_vec(&receipt()).unwrap();
    x.write_receipt(&receipt()).unwrap();

    std::fs::rename(&root, &detached).unwrap();
    let replacement = TraceExport::create(&root).unwrap();
    let mut decoy = receipt();
    decoy.claims.entry_id = "replacement".to_string();
    replacement.write_receipt(&decoy).unwrap();

    let mut reader = x
        .read_fixed(FixedTraceDocument::Receipt)
        .unwrap()
        .expect("the detached receipt remains present");
    let mut actual = Vec::new();
    reader.read_to_end(&mut actual).unwrap();

    assert_eq!(
        actual, expected,
        "the held root supplies the original bytes"
    );
    assert_ne!(
        actual,
        std::fs::read(root.join(RECEIPT_FILE)).unwrap(),
        "the replacement path contains a distinct decoy"
    );
}

#[test]
fn receipt_writer_refuses_an_existing_hard_link_without_changing_its_alias() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("trace");
    let outside = tmp.path().join("outside.json");
    let x = TraceExport::create(&root).unwrap();
    std::fs::write(&outside, b"outside bytes").unwrap();
    std::fs::hard_link(&outside, root.join(RECEIPT_FILE)).unwrap();

    assert!(x.write_receipt(&receipt()).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside bytes");
}

#[test]
fn resume_sweeps_only_canonical_descriptor_temps() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    drop(TraceExport::create(root).unwrap());
    let owned = root.join(format!("{DESCRIPTOR_FILE}.123.4.tmp"));
    let unrelated = root.join("foo.tmp");
    std::fs::write(&owned, b"owned").unwrap();
    std::fs::write(&unrelated, b"unrelated").unwrap();

    drop(TraceExport::resume(root).unwrap());

    assert!(!owned.exists());
    assert!(unrelated.exists());
}

#[test]
fn fresh_create_refuses_orphan_acknowledgements_and_fixed_documents() {
    let ack_root = tempfile::tempdir().unwrap();
    drop(TraceExport::create(ack_root.path()).unwrap());
    std::fs::write(ack_root.path().join(ACKS_FILE), b"orphan\n").unwrap();
    assert!(matches!(
        TraceExport::create(ack_root.path()),
        Err(ExportError::AcksExist)
    ));
    assert_eq!(
        std::fs::read(ack_root.path().join(ACKS_FILE)).unwrap(),
        b"orphan\n"
    );

    let fixed_root = tempfile::tempdir().unwrap();
    drop(TraceExport::create(fixed_root.path()).unwrap());
    std::fs::write(fixed_root.path().join(RECEIPT_FILE), b"old receipt").unwrap();
    assert!(matches!(
        TraceExport::create(fixed_root.path()),
        Err(ExportError::FixedDocumentExists { name: RECEIPT_FILE })
    ));
    assert_eq!(
        std::fs::read(fixed_root.path().join(RECEIPT_FILE)).unwrap(),
        b"old receipt"
    );
}

#[test]
fn fixed_documents_are_absent_only_and_collisions_preserve_the_first_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let x = TraceExport::create(tmp.path()).unwrap();
    x.write_receipt(&receipt()).unwrap();
    let receipt_bytes = std::fs::read(tmp.path().join(RECEIPT_FILE)).unwrap();
    assert!(x.write_receipt(&receipt()).is_err());
    assert_eq!(
        std::fs::read(tmp.path().join(RECEIPT_FILE)).unwrap(),
        receipt_bytes
    );
}

#[test]
fn preparation_refuses_missing_chain_without_mutation() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join(RECEIPT_FILE), b"fixed receipt bytes").unwrap();
    let stale = tmp.path().join(format!("{DESCRIPTOR_FILE}.777777.13.tmp"));
    std::fs::write(&stale, b"stale descriptor bytes").unwrap();
    let before = regular_file_snapshot(tmp.path());

    let root = TraceRoot::open(tmp.path()).unwrap();
    assert!(TraceExport::prepare_resume_from_root(root).is_err());

    assert_eq!(regular_file_snapshot(tmp.path()), before);
    assert!(!tmp.path().join(ACKS_FILE).exists());
}

#[test]
fn preparation_refuses_corrupt_chain_without_mutation() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join(CHAIN_FILE), b"{not an envelope}\n").unwrap();
    std::fs::write(tmp.path().join(RECEIPT_FILE), b"fixed receipt bytes").unwrap();
    let stale = tmp.path().join(format!("{DESCRIPTOR_FILE}.777777.14.tmp"));
    std::fs::write(&stale, b"stale descriptor bytes").unwrap();
    let before = regular_file_snapshot(tmp.path());

    let root = TraceRoot::open(tmp.path()).unwrap();
    assert!(matches!(
        TraceExport::prepare_resume_from_root(root),
        Err(ExportError::ChainCorrupt { seq: 0, .. })
    ));

    assert_eq!(regular_file_snapshot(tmp.path()), before);
    assert!(!tmp.path().join(ACKS_FILE).exists());
}

#[cfg(unix)]
#[test]
fn poisoned_prepared_reader_refuses_finalization_before_mutation() {
    let tmp = tempfile::tempdir().unwrap();
    drop(TraceExport::create(tmp.path()).unwrap());
    std::fs::remove_file(tmp.path().join(ACKS_FILE)).unwrap();
    let stale = tmp.path().join(format!("{DESCRIPTOR_FILE}.777777.16.tmp"));
    std::fs::write(&stale, b"stale descriptor bytes").unwrap();

    let root = TraceRoot::open(tmp.path()).unwrap();
    let mut prepared = TraceExport::prepare_resume_from_root(root).unwrap();
    let named = tmp.path().join(CHAIN_FILE);
    let original = tmp.path().join("original-chain");
    std::fs::rename(&named, &original).unwrap();
    std::fs::write(&named, b"replacement chain").unwrap();
    assert!(prepared.independent_chain_reader().is_err());
    std::fs::remove_file(&named).unwrap();
    std::fs::rename(&original, &named).unwrap();
    let chain_before = std::fs::read(&named).unwrap();

    assert!(matches!(
        TraceExport::resume_prepared(prepared),
        Err(ExportError::Capability(
            goalcup_platform::tracecap::TraceCapError::Poisoned {
                stream: "chain.jsonl"
            }
        ))
    ));

    assert_eq!(std::fs::read(named).unwrap(), chain_before);
    assert_eq!(std::fs::read(stale).unwrap(), b"stale descriptor bytes");
    assert!(!tmp.path().join(ACKS_FILE).exists());
}

/// `TraceExport::create` and `TraceExport::resume` sweep any `*.tmp` file
/// left in the trace directory — the residue of a `write_file_atomic` a crash caught
/// between its `File::create` and the rename that would have cleared it. Planting one by
/// hand and resuming must remove it without touching either of the two real files
/// `create` always leaves behind.
#[test]
fn resume_sweeps_a_stale_tmp_file_and_touches_nothing_else() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    TraceExport::create(root).unwrap();

    let stale = root.join(format!("{DESCRIPTOR_FILE}.999999.3.tmp"));
    std::fs::write(&stale, b"half-written").unwrap();
    assert!(stale.exists(), "the stale file must exist before resume");

    let chain_bytes_before = std::fs::read(root.join(CHAIN_FILE)).unwrap();
    let acks_bytes_before = std::fs::read(root.join(ACKS_FILE)).unwrap();

    TraceExport::resume(root).unwrap();

    assert!(
        !stale.exists(),
        "resume must have swept the stale temp file"
    );
    assert_eq!(
        std::fs::read(root.join(CHAIN_FILE)).unwrap(),
        chain_bytes_before,
        "the real chain file must be untouched"
    );
    assert_eq!(
        std::fs::read(root.join(ACKS_FILE)).unwrap(),
        acks_bytes_before,
        "the real acks file must be untouched"
    );
}

#[test]
fn invalid_resume_preserves_canonical_temp_and_absent_acknowledgements() {
    let tmp = tempfile::tempdir().unwrap();
    drop(TraceExport::create(tmp.path()).unwrap());
    std::fs::remove_file(tmp.path().join(ACKS_FILE)).unwrap();

    let chain_path = tmp.path().join(CHAIN_FILE);
    let mut chain_before = std::fs::read(&chain_path).unwrap();
    chain_before.extend_from_slice(b"{\"protocol_version\":2");
    std::fs::write(&chain_path, &chain_before).unwrap();
    let stale_path = tmp.path().join(format!("{DESCRIPTOR_FILE}.999999.777.tmp"));
    let stale_before = b"owned stale descriptor bytes";
    std::fs::write(&stale_path, stale_before).unwrap();
    let mut entries_before = std::fs::read_dir(tmp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    entries_before.sort();

    assert!(matches!(
        TraceExport::resume(tmp.path()),
        Err(ExportError::ChainCorrupt { seq: 0, .. })
    ));

    let mut entries_after = std::fs::read_dir(tmp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    entries_after.sort();
    assert_eq!(entries_after, entries_before);
    assert_eq!(std::fs::read(chain_path).unwrap(), chain_before);
    assert_eq!(std::fs::read(stale_path).unwrap(), stale_before);
    assert!(!tmp.path().join(ACKS_FILE).exists());
}

/// Append one already-stamped envelope straight to the file, bypassing the writer.
fn append_line(path: &std::path::Path, env: &Envelope) {
    let mut line = serde_json::to_vec(env).unwrap();
    line.push(b'\n');
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    std::io::Write::write_all(&mut f, &line).unwrap();
}

fn append_correctly_linked_test_envelope(
    root: &std::path::Path,
    previous: Option<&Link>,
    mut envelope: Envelope,
) {
    use std::io::Write as _;

    envelope.link.seq = previous.map_or(0, |link| link.seq + 1);
    envelope.link.prev = previous.map_or_else(|| GENESIS.to_string(), |link| link.hash.clone());
    envelope.link.hash = envelope.expected_hash(&envelope.link.prev).unwrap();
    let mut line = serde_json::to_vec(&envelope).unwrap();
    line.push(b'\n');
    std::fs::OpenOptions::new()
        .append(true)
        .open(root.join(CHAIN_FILE))
        .unwrap()
        .write_all(&line)
        .unwrap();
}

fn assert_resume_refuses_without_rewriting(root: &std::path::Path, seq: u64) {
    let chain_path = root.join(CHAIN_FILE);
    let before = std::fs::read(&chain_path).unwrap();
    assert!(matches!(
        TraceExport::resume(root),
        Err(ExportError::ChainCorrupt {
            seq: found_seq,
            ..
        }) if found_seq == seq
    ));
    assert_eq!(std::fs::read(chain_path).unwrap(), before);
}

#[test]
fn the_writer_owns_every_path_of_the_trace() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("entries").join("e1");
    let mut x = capture(&root);
    assert_eq!(x.root(), root.as_path());

    // `create` opens both append-only streams, so both exist before anything is written.
    assert!(root.join(CHAIN_FILE).is_file());
    assert!(root.join(ACKS_FILE).is_file());

    let receipt = StartReceipt {
        claims: StartReceiptClaims {
            entry_id: "e1".to_string(),
            connector_id: "claude-code-cli".to_string(),
            workspace: "/ws".to_string(),
            session_id: "s1".to_string(),
            started_at: "2026-09-04T00:00:00Z".to_string(),
            deadline_at: "2026-09-08T00:00:00Z".to_string(),
            manifest_sha256: "cc".repeat(32),
            chain_head: GENESIS.to_string(),
            connector_dir_sha256: Some("ee".repeat(32)),
            run_public_key: None,
            questionnaire_version: None,
        },
        public_key: "dd".repeat(32),
        signature: "ee".repeat(64),
    };
    assert_eq!(
        x.write_receipt(&receipt).unwrap(),
        root.join(RECEIPT_FILE),
        "the receipt lands at its fixed name"
    );
    let back: StartReceipt =
        serde_json::from_slice(&std::fs::read(root.join(RECEIPT_FILE)).unwrap()).unwrap();
    assert_eq!(back, receipt);

    // The baseline is a seal manifest taken at t0, written to its own file AND chained
    // as the chain's first link. A baseline bound to nothing is a list the participant
    // wrote about their own workspace; the link is what a receipt's `chain_head` witnesses.
    let baseline = SealManifest {
        sealed_at: "2026-09-04T00:00:00Z".to_string(),
        ..manifest()
    };
    let (path, link) = x.write_baseline_for_protocol(V5, &baseline).unwrap();
    assert_eq!(path, root.join(BASELINE_FILE));
    assert_eq!(link.seq, 0, "the baseline is the genesis link");
    let back: SealManifest =
        serde_json::from_slice(&std::fs::read(root.join(BASELINE_FILE)).unwrap()).unwrap();
    assert_eq!(back, baseline);
    assert_eq!(
        std::fs::read_to_string(root.join(CHAIN_FILE))
            .unwrap()
            .lines()
            .count(),
        1,
        "the baseline is a link"
    );

    for (i, seq) in [0u64, 4, 9].iter().enumerate() {
        x.append_ack(&AckCheckpoint {
            entry_id: "e1".to_string(),
            seq: *seq,
            hash: "ab".repeat(32),
            acked_at: format!("2026-09-04T00:0{i}:00Z"),
            server_signature: "cd".repeat(64),
        })
        .unwrap();
    }
    let acks: Vec<AckCheckpoint> = std::fs::read_to_string(root.join(ACKS_FILE))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(acks.len(), 3, "one AckCheckpoint per line, append-only");
    assert_eq!(
        acks.iter().map(|a| a.seq).collect::<Vec<_>>(),
        vec![0, 4, 9]
    );
}

#[test]
fn preparing_and_dropping_a_valid_trace_changes_no_entry_or_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture(tmp.path());
    export.write_receipt(&receipt()).unwrap();
    let (_, baseline) = export.write_baseline_for_protocol(V5, &manifest()).unwrap();
    drop(export);
    std::fs::remove_file(tmp.path().join(ACKS_FILE)).unwrap();
    let stale = tmp.path().join(format!("{DESCRIPTOR_FILE}.777777.12.tmp"));
    std::fs::write(&stale, b"stale descriptor bytes").unwrap();
    let before = regular_file_snapshot(tmp.path());

    let root = TraceRoot::open(tmp.path()).unwrap();
    let mut prepared = TraceExport::prepare_resume_from_root(root).unwrap();
    assert_eq!(prepared.head(), Some((0, baseline.hash)));

    let mut receipt_reader = prepared
        .read_fixed(FixedTraceDocument::Receipt)
        .unwrap()
        .unwrap();
    let mut receipt_bytes = Vec::new();
    receipt_reader.read_to_end(&mut receipt_bytes).unwrap();
    assert_eq!(
        serde_json::from_slice::<StartReceipt>(&receipt_bytes).unwrap(),
        receipt()
    );

    let mut chain_reader = prepared.independent_chain_reader().unwrap();
    let mut chain_bytes = Vec::new();
    chain_reader.read_to_end(&mut chain_bytes).unwrap();
    assert_eq!(
        chain_bytes,
        std::fs::read(tmp.path().join(CHAIN_FILE)).unwrap()
    );
    drop(chain_reader);
    drop(prepared);

    assert_eq!(regular_file_snapshot(tmp.path()), before);
    assert!(stale.exists());
    assert!(!tmp.path().join(ACKS_FILE).exists());
}

#[cfg(unix)]
#[test]
fn finalizing_uses_the_prepared_root_and_chain_after_path_exchange() {
    let tmp = tempfile::tempdir().unwrap();
    let trace = tmp.path().join("trace");
    let detached = tmp.path().join("detached");
    let (export, first) = started(&trace);
    drop(export);
    std::fs::remove_file(trace.join(ACKS_FILE)).unwrap();
    let stale_name = format!("{DESCRIPTOR_FILE}.777777.15.tmp");
    std::fs::write(trace.join(&stale_name), b"stale descriptor bytes").unwrap();

    let root = TraceRoot::open(&trace).unwrap();
    let prepared = TraceExport::prepare_resume_from_root(root).unwrap();
    std::fs::rename(&trace, &detached).unwrap();
    std::fs::create_dir(&trace).unwrap();
    std::fs::write(trace.join("replacement-sentinel"), b"outside").unwrap();

    let mut resumed = TraceExport::resume_prepared(prepared).unwrap();
    assert!(!detached.join(stale_name).exists());
    assert_eq!(std::fs::read(detached.join(ACKS_FILE)).unwrap(), b"");
    resumed.configure_capture_quota(QUOTA).unwrap();
    let mut second = env(2);
    let second = resumed.append_chain(&mut second).unwrap();
    assert_eq!(second.seq, 1);
    assert_eq!(second.prev, first.hash);
    assert_eq!(
        std::fs::read_to_string(detached.join(CHAIN_FILE))
            .unwrap()
            .lines()
            .count(),
        2
    );
    assert_eq!(
        std::fs::read(trace.join("replacement-sentinel")).unwrap(),
        b"outside"
    );
    assert!(!trace.join(CHAIN_FILE).exists());
    assert!(!trace.join(ACKS_FILE).exists());
}

#[cfg(unix)]
#[test]
fn changed_length_refuses_finalization_before_mutation() {
    use std::io::Write as _;

    let tmp = tempfile::tempdir().unwrap();
    let (export, _) = started(tmp.path());
    drop(export);
    std::fs::remove_file(tmp.path().join(ACKS_FILE)).unwrap();
    let stale = tmp.path().join(format!("{DESCRIPTOR_FILE}.777777.17.tmp"));
    std::fs::write(&stale, b"stale descriptor bytes").unwrap();
    let validated_bytes = std::fs::metadata(tmp.path().join(CHAIN_FILE))
        .unwrap()
        .len();

    let root = TraceRoot::open(tmp.path()).unwrap();
    let prepared = TraceExport::prepare_resume_from_root(root).unwrap();
    std::fs::OpenOptions::new()
        .append(true)
        .open(tmp.path().join(CHAIN_FILE))
        .unwrap()
        .write_all(b"external growth")
        .unwrap();
    let changed_chain = std::fs::read(tmp.path().join(CHAIN_FILE)).unwrap();

    assert!(matches!(
        TraceExport::resume_prepared(prepared),
        Err(ExportError::ChainLengthChanged {
            validated_bytes: found_validated,
            found_bytes,
        }) if found_validated == validated_bytes && found_bytes == validated_bytes + 15
    ));

    assert_eq!(
        std::fs::read(tmp.path().join(CHAIN_FILE)).unwrap(),
        changed_chain
    );
    assert_eq!(std::fs::read(stale).unwrap(), b"stale descriptor bytes");
    assert!(!tmp.path().join(ACKS_FILE).exists());
}

#[test]
fn acquired_root_can_be_inspected_then_consumed_for_resume() {
    let tmp = tempfile::tempdir().unwrap();
    let trace = tmp.path().join("trace");
    let detached = tmp.path().join("detached");
    let (x, _) = started(&trace);
    x.write_receipt(&receipt()).unwrap();
    drop(x);

    let root = TraceRoot::open(&trace).unwrap();
    let mut receipt_file = root
        .read_fixed(FixedTraceDocument::Receipt)
        .unwrap()
        .unwrap();
    let mut receipt_bytes = Vec::new();
    receipt_file.read_to_end(&mut receipt_bytes).unwrap();
    assert_eq!(
        serde_json::from_slice::<StartReceipt>(&receipt_bytes).unwrap(),
        receipt()
    );

    std::fs::rename(&trace, &detached).unwrap();
    std::fs::create_dir(&trace).unwrap();

    let mut resumed = TraceExport::resume_from_root(root).unwrap();
    let mut chain_reader = resumed.independent_chain_reader().unwrap();
    let mut chain_bytes = Vec::new();
    chain_reader.read_to_end(&mut chain_bytes).unwrap();
    assert_eq!(
        chain_bytes,
        std::fs::read(detached.join(CHAIN_FILE)).unwrap()
    );

    resumed.configure_capture_quota(QUOTA).unwrap();
    assert_eq!(resumed.append_chain(&mut env(2)).unwrap().seq, 1);
    assert!(!trace.join(CHAIN_FILE).exists());
    assert_eq!(
        std::fs::read_to_string(detached.join(CHAIN_FILE))
            .unwrap()
            .lines()
            .count(),
        2
    );
}

#[cfg(unix)]
#[test]
fn mismatched_chain_reader_poison_survives_name_restoration() {
    let tmp = tempfile::tempdir().unwrap();
    let mut x = capture(tmp.path());
    let original = tmp.path().join("original-chain");
    let named = tmp.path().join(CHAIN_FILE);
    std::fs::rename(&named, &original).unwrap();
    std::fs::write(&named, b"").unwrap();
    assert!(x.independent_chain_reader().is_err());
    std::fs::remove_file(&named).unwrap();
    std::fs::rename(&original, &named).unwrap();

    assert!(x.write_baseline_for_protocol(V5, &manifest()).is_err());
    assert!(std::fs::read(&named).unwrap().is_empty());
}

#[test]
fn a_failed_chain_link_check_permanently_poisons_that_writer() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mut x = capture(root);
    let alias = root.join("chain-alias");
    std::fs::hard_link(root.join(CHAIN_FILE), &alias).unwrap();

    assert!(x.write_baseline_for_protocol(V5, &manifest()).is_err());
    assert_eq!(x.head(), None);
    assert!(std::fs::read(root.join(CHAIN_FILE)).unwrap().is_empty());

    std::fs::remove_file(alias).unwrap();
    assert!(x.write_baseline_for_protocol(V5, &manifest()).is_err());
    assert!(std::fs::read(root.join(CHAIN_FILE)).unwrap().is_empty());
}

/// V13 recomputes the chained claim from the manifest it loaded, so the digest
/// has to be the digest of the bytes `write_seal` actually put on disk — not of some
/// other serialisation of the same value.
#[test]
fn the_chained_digest_is_the_digest_of_the_bytes_in_seal_json() {
    use sha2::{Digest, Sha256};
    let tmp = tempfile::tempdir().unwrap();
    let (mut x, _) = started(tmp.path());
    let mut m = manifest();
    m.files.push(SealedFile {
        path: "src/main.rs".to_string(),
        bytes: 12,
        sha256: "aa".repeat(32),
    });
    m.excluded.push(".git".to_string());
    let (path, _) = x.write_seal_for_protocol(V5, &m).unwrap();

    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(&bytes)),
        manifest_sha256(&m).unwrap(),
        "manifest_sha256 must hash exactly the bytes on disk"
    );
    let reloaded: SealManifest = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        manifest_sha256(&reloaded).unwrap(),
        manifest_sha256(&m).unwrap(),
        "a round trip through the file must not move the digest"
    );

    let last: Envelope = serde_json::from_str(
        std::fs::read_to_string(tmp.path().join(CHAIN_FILE))
            .unwrap()
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap();
    let EventBody::Seal { file_count, .. } = last.body else {
        panic!("expected a seal link");
    };
    assert_eq!(file_count, 1);
}

#[test]
fn baseline_and_seal_canonicalize_manifest_order_before_hashing_and_writing() {
    use goalcup_protocol::export::BASELINE_FILE;

    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture(tmp.path());
    let mut unsorted = manifest();
    unsorted.files = vec![
        SealedFile {
            path: "z-last.rs".to_string(),
            bytes: 1,
            sha256: "bb".repeat(32),
        },
        SealedFile {
            path: "a-first.rs".to_string(),
            bytes: 2,
            sha256: "aa".repeat(32),
        },
    ];
    unsorted.excluded = vec!["target".to_string(), ".git".to_string()];
    let canonical = unsorted.clone().sorted();

    export.write_baseline_for_protocol(V5, &unsorted).unwrap();
    export.write_seal_for_protocol(V5, &unsorted).unwrap();

    let baseline: SealManifest =
        serde_json::from_slice(&std::fs::read(tmp.path().join(BASELINE_FILE)).unwrap()).unwrap();
    let seal: SealManifest =
        serde_json::from_slice(&std::fs::read(tmp.path().join(SEAL_FILE)).unwrap()).unwrap();
    assert_eq!(baseline, canonical);
    assert_eq!(seal, canonical);
    assert_eq!(
        manifest_sha256(&unsorted).unwrap(),
        manifest_sha256(&canonical).unwrap()
    );

    let envelopes: Vec<Envelope> = std::fs::read_to_string(tmp.path().join(CHAIN_FILE))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(envelopes.len(), 2);
    for env in envelopes {
        let digest = match env.body {
            EventBody::Baseline {
                manifest_sha256, ..
            }
            | EventBody::Seal {
                manifest_sha256, ..
            } => manifest_sha256,
            other => panic!("expected baseline or seal, got {other:?}"),
        };
        assert_eq!(digest, manifest_sha256(&canonical).unwrap());
    }
}

#[test]
fn the_writer_produces_one_contiguous_chain_the_reader_can_recompute() {
    let tmp = tempfile::tempdir().unwrap();
    let mut x = capture(tmp.path());
    assert_eq!(
        x.head(),
        None,
        "an empty chain has no head, it is not at seq 0"
    );

    let (_, base) = x.write_baseline_for_protocol(V5, &manifest()).unwrap();
    let mut a = env(1);
    let mut b = env(2);
    let la = x.append_chain(&mut a).unwrap();
    let lb = x.append_chain(&mut b).unwrap();

    assert_eq!(
        (base.seq, la.seq, lb.seq),
        (0, 1, 2),
        "seq is contiguous from 0"
    );
    assert_eq!(base.prev, GENESIS);
    assert_eq!(la.prev, base.hash);
    assert_eq!(lb.prev, la.hash);
    assert_eq!(la.hash, link_hash(&base.hash, &a.chain_bytes().unwrap()));
    assert_eq!(lb.hash, link_hash(&la.hash, &b.chain_bytes().unwrap()));
    assert_eq!(x.head(), Some((2, lb.hash.clone())));

    // One Envelope per line, and every line recomputes.
    let text = std::fs::read_to_string(tmp.path().join(CHAIN_FILE)).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    let mut prev = GENESIS.to_string();
    for (i, line) in lines.iter().enumerate() {
        let e: Envelope = serde_json::from_str(line).unwrap();
        assert_eq!(e.link.seq, i as u64);
        assert_eq!(e.link.prev, prev);
        assert_eq!(e.link.hash, e.expected_hash(&prev).unwrap());
        prev = e.link.hash;
    }

    // Resuming continues rather than restarting, and it reads the position off the
    // chain rather than taking the caller's word for it.
    let (_, head) = x.head().unwrap();
    drop(x);
    let mut y = reopen(tmp.path());
    assert_eq!(
        y.head(),
        Some((2, head.clone())),
        "resume recovers the head from the file"
    );
    let lc = y.append_chain(&mut env(3)).unwrap();
    assert_eq!(lc.seq, 3);
    assert_eq!(lc.prev, head);
}

#[test]
fn the_seal_is_the_last_link_before_it_is_a_file_and_there_is_only_ever_one() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut x, _) = started(tmp.path());
    x.append_chain(&mut env(1)).unwrap();

    let m = manifest();
    let (path, link) = x.write_seal_for_protocol(V5, &m).unwrap();
    assert_eq!(path, tmp.path().join(SEAL_FILE));
    assert_eq!(link.seq, 2, "the seal is a link, not only a file");
    assert_eq!(
        x.head(),
        Some((2, link.hash.clone())),
        "and it is the LAST link"
    );

    // The chained claim is the digest of the bytes on disk, so an ack over
    // seq 2 anchors the seal and a reader can recompute the claim from `seal.json`.
    let text = std::fs::read_to_string(tmp.path().join(CHAIN_FILE)).unwrap();
    let last: Envelope = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(last.source, "daemon");
    assert_eq!(last.recorded_at, "2026-09-08T00:00:00Z");
    match last.body {
        EventBody::Seal {
            manifest_sha256,
            sealed_at,
            file_count,
        } => {
            assert_eq!(
                manifest_sha256,
                goalcup_protocol::export::manifest_sha256(&m).unwrap()
            );
            assert_eq!(sealed_at, "2026-09-08T00:00:00Z");
            assert_eq!(file_count, 0);
        }
        other => panic!("the seal must be IN the chain, not only beside it: {other:?}"),
    }
    let on_disk: SealManifest =
        serde_json::from_slice(&std::fs::read(tmp.path().join(SEAL_FILE)).unwrap()).unwrap();
    assert_eq!(on_disk, m);

    assert!(matches!(
        x.write_seal_for_protocol(V5, &m),
        Err(ExportError::SealAlreadyChained { seq: 2 })
    ));
    // And nothing else may follow the seal either: a record after it would leave an
    // acknowledgement over the seal anchoring a chain that kept growing.
    assert!(matches!(
        x.append_chain(&mut env(3)),
        Err(ExportError::Sealed { seal_seq: 2 })
    ));
    // And the refused calls wrote nothing: the chain still ends at the seal.
    assert_eq!(chain_lines(tmp.path()), 3);
}

/// The caller hands the writer content, never a position. A stamped link that does not
/// name the seq the writer is about to write is a caller computing chain state of its
/// own, and is refused rather than silently overwritten.
#[test]
fn a_caller_cannot_hand_roll_a_link() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut x, _) = started(tmp.path());
    let mut a = env(1);
    x.append_chain(&mut a).unwrap();

    // A hand-rolled hash over a hand-rolled prev is discarded, not honoured.
    let mut b = env(2);
    b.link = Link {
        seq: 2,
        prev: GENESIS.to_string(),
        hash: "ff".repeat(32),
    };
    let lb = x.append_chain(&mut b).unwrap();
    assert_eq!(
        lb.prev, a.link.hash,
        "prev is the writer's, not the caller's"
    );
    assert_eq!(lb.hash, link_hash(&a.link.hash, &b.chain_bytes().unwrap()));

    // A stamped seq that skips a position is refused outright.
    let mut c = env(3);
    c.link.seq = 9;
    assert!(matches!(
        x.append_chain(&mut c),
        Err(ExportError::OutOfOrder {
            found: 9,
            expected_after: Some(2)
        })
    ));
    assert_eq!(
        x.head().unwrap().0,
        2,
        "the refused append did not move the head"
    );
    assert_eq!(
        chain_lines(tmp.path()),
        3,
        "the refused append wrote no line"
    );
}

/// The once-only seal rule cannot live in one process's memory: a daemon that is killed
/// after sealing and restarted would otherwise chain a second seal, and an
/// acknowledgement over the entry would have two heads to anchor. `resume` reads the
/// chain it is continuing, so the seal it finds there binds the new instance too.
#[test]
fn a_resumed_export_will_not_chain_past_a_seal_it_found() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut x, _) = started(tmp.path());
    x.append_chain(&mut env(1)).unwrap();
    let (_, seal) = x.write_seal_for_protocol(V5, &manifest()).unwrap();
    drop(x);

    let mut y = reopen(tmp.path());
    assert_eq!(
        y.head(),
        Some((2, seal.hash.clone())),
        "the resumed writer picked the seal link up as its head"
    );
    assert!(matches!(
        y.append_chain(&mut env(2)),
        Err(ExportError::Sealed { seal_seq: 2 })
    ));
    assert!(matches!(
        y.write_seal_for_protocol(V5, &manifest()),
        Err(ExportError::SealAlreadyChained { seq: 2 })
    ));
    assert_eq!(chain_lines(tmp.path()), 3, "neither refusal wrote a line");
}

/// A daemon killed between `create` and its first record leaves an empty `chain.jsonl`.
/// Resuming that must start at 0 over `GENESIS` — starting at 1 would leave the chain
/// with no seq 0 and nothing to link the first record to.
#[test]
fn resume_on_an_empty_chain_starts_at_zero() {
    let tmp = tempfile::tempdir().unwrap();
    drop(TraceExport::create(tmp.path()).unwrap());

    let mut y = reopen(tmp.path());
    assert_eq!(y.head(), None, "nothing is chained, so there is no head");
    let (_, la) = y.write_baseline_for_protocol(V5, &manifest()).unwrap();
    assert_eq!(la.seq, 0, "the first link of a chain is 0, resumed or not");
    assert_eq!(la.prev, GENESIS);
    assert_eq!(y.head(), Some((0, la.hash.clone())));
}

#[test]
fn resume_refuses_an_unterminated_tail_without_changing_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    let (x, _) = started(tmp.path());
    drop(x);
    let path = tmp.path().join(CHAIN_FILE);
    let mut before = std::fs::read(&path).unwrap();
    before.extend_from_slice(b"{\"protocol_version\":5");
    std::fs::write(&path, &before).unwrap();

    assert!(matches!(
        TraceExport::resume(tmp.path()),
        Err(ExportError::ChainCorrupt { seq: 1, .. })
    ));
    assert_eq!(std::fs::read(path).unwrap(), before);
}

/// `create` opens `chain.jsonl` for append. Called against a directory that already
/// holds a chain it would write a second run's links onto the end of the first run's,
/// producing one file that reads as one chain and is two. Resuming is the caller's job.
#[test]
fn create_refuses_a_directory_that_already_holds_a_chain() {
    let tmp = tempfile::tempdir().unwrap();
    let (x, _) = started(tmp.path());
    drop(x);
    let before = std::fs::read_to_string(tmp.path().join(CHAIN_FILE)).unwrap();

    assert!(matches!(
        TraceExport::create(tmp.path()),
        Err(ExportError::ChainExists)
    ));
    assert_eq!(
        std::fs::read_to_string(tmp.path().join(CHAIN_FILE)).unwrap(),
        before,
        "the refused create left the chain alone"
    );
}

/// A chain that already carries two seals is not something a writer may extend: it has
/// two candidate heads for an acknowledgement to anchor, and picking one here would be
/// the writer deciding which one counts.
#[test]
fn two_seal_lines_make_a_chain_unresumable() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut x, _) = started(tmp.path());
    let m = manifest();
    let (_, first) = x.write_seal_for_protocol(V5, &m).unwrap();
    drop(x);

    // Hand-written, because the writer under test refuses to produce this: a *valid*
    // second seal link, contiguous and correctly hashed, so the only thing wrong with
    // the file is that it seals the entry twice.
    let mut second = reserved(
        &m.sealed_at,
        EventBody::Seal {
            manifest_sha256: manifest_sha256(&m).unwrap(),
            sealed_at: m.sealed_at.clone(),
            file_count: 0,
        },
    );
    second.link.seq = 2;
    second.link.prev = first.hash.clone();
    second.link.hash = link_hash(&first.hash, &second.chain_bytes().unwrap());
    append_line(&tmp.path().join(CHAIN_FILE), &second);

    match TraceExport::resume(tmp.path()) {
        Err(ExportError::ChainCorrupt { seq, reason }) => {
            assert_eq!(seq, 2);
            assert!(reason.contains("seal"), "{reason}");
        }
        other => panic!("a twice-sealed chain must not resume: {other:?}"),
    }
}

/// Every line is read before the writer will continue the file: a chain it cannot parse,
/// or one whose seqs skip, is not one it can honestly append to.
#[test]
fn a_chain_the_writer_cannot_read_is_not_one_it_will_extend() {
    let garbage = tempfile::tempdir().unwrap();
    let (x, _) = started(garbage.path());
    drop(x);
    std::fs::OpenOptions::new()
        .append(true)
        .open(garbage.path().join(CHAIN_FILE))
        .and_then(|mut f| std::io::Write::write_all(&mut f, b"{not json\n"))
        .unwrap();
    assert!(matches!(
        TraceExport::resume(garbage.path()),
        Err(ExportError::ChainCorrupt { seq: 1, .. })
    ));

    let gap = tempfile::tempdir().unwrap();
    let (x, la) = started(gap.path());
    drop(x);
    let mut skipped = env(1);
    skipped.link.seq = 7;
    skipped.link.prev = la.hash.clone();
    skipped.link.hash = link_hash(&la.hash, &skipped.chain_bytes().unwrap());
    append_line(&gap.path().join(CHAIN_FILE), &skipped);
    assert!(matches!(
        TraceExport::resume(gap.path()),
        Err(ExportError::ChainCorrupt { seq: 1, .. })
    ));
}

/// The baseline is the chain's FIRST link, and the digest it chains is the digest of
/// the bytes in `baseline.json`.
///
/// Without the link, `baseline.json` is a file the participant's own daemon wrote and nothing
/// covers: a workspace pre-populated before the start could be listed there wholesale,
/// every planted file's real path and digest included, and the verifier would exempt each
/// one. Chaining it at seq 0 puts the baseline under the receipt's `chain_head` witness —
/// the link the server saw when the run was declared.
#[test]
fn the_baseline_is_the_first_link_and_names_the_digest_of_the_file_beside_it() {
    use sha2::{Digest, Sha256};
    let tmp = tempfile::tempdir().unwrap();
    let mut x = capture(tmp.path());

    let baseline = SealManifest {
        sealed_at: "2026-09-04T00:00:00Z".to_string(),
        files: vec![SealedFile {
            path: "README.md".to_string(),
            bytes: 12,
            sha256: "aa".repeat(32),
        }],
        ..manifest()
    };
    let (path, link) = x.write_baseline_for_protocol(V5, &baseline).unwrap();
    assert_eq!(path, tmp.path().join(BASELINE_FILE));
    assert_eq!(link.seq, 0, "the baseline is seq 0");
    assert_eq!(link.prev, GENESIS, "and nothing precedes it");
    assert_eq!(x.head(), Some((0, link.hash.clone())));

    // The line on disk, not the returned value: a writer that stamped a link and wrote
    // no file would pass an assertion over the `Link` alone.
    let text = std::fs::read_to_string(tmp.path().join(CHAIN_FILE)).unwrap();
    assert_eq!(text.lines().count(), 1);
    let chained: Envelope = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(chained.source, "daemon");
    assert_eq!(chained.recorded_at, baseline.sealed_at);
    assert_eq!(chained.entry_id, baseline.entry_id);
    assert_eq!(chained.link.hash, link.hash);
    match chained.body {
        EventBody::Baseline {
            manifest_sha256: digest,
            file_count,
        } => {
            assert_eq!(file_count, 1, "the baseline's own file count");
            // The one definition, shared with the seal: the digest of the bytes that are
            // in `baseline.json`, recomputable by anybody who has the manifest.
            assert_eq!(digest, manifest_sha256(&baseline).unwrap());
            let bytes = std::fs::read(tmp.path().join(BASELINE_FILE)).unwrap();
            assert_eq!(digest, hex::encode(Sha256::digest(&bytes)));
        }
        other => panic!("the first link is not a baseline: {other:?}"),
    }

    // A record chains on top of it, at seq 1, over the baseline's hash.
    let after = x.append_chain(&mut env(1)).unwrap();
    assert_eq!(after.seq, 1);
    assert_eq!(after.prev, link.hash);
}

/// Two baselines would be two answers to "what did the workspace hold at t0", and a
/// reviewer would have to pick. There is one, and it is seq 0.
#[test]
fn there_is_only_ever_one_baseline() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut x, _) = started(tmp.path());
    assert!(matches!(
        x.write_baseline_for_protocol(V5, &manifest()),
        Err(ExportError::BaselineAlreadyChained)
    ));
    assert_eq!(
        chain_lines(tmp.path()),
        1,
        "the refused call wrote no second link"
    );
}

/// The once-only rule cannot live in one process's memory, for the same reason the seal's
/// cannot: a daemon killed after taking the baseline and restarted would otherwise chain
/// a second one, over a workspace the agent has been writing to since.
#[test]
fn a_resumed_export_remembers_the_baseline_it_found() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut x, baseline) = started(tmp.path());
    x.append_chain(&mut env(1)).unwrap();
    drop(x);

    let mut y = reopen(tmp.path());
    assert!(matches!(
        y.write_baseline_for_protocol(V5, &manifest()),
        Err(ExportError::BaselineAlreadyChained)
    ));
    // And the resumed writer is otherwise the same writer: it continues at seq 2 over the
    // head it read, and the baseline it found is still the link at seq 0.
    assert_eq!(y.append_chain(&mut env(2)).unwrap().seq, 2);
    let text = std::fs::read_to_string(tmp.path().join(CHAIN_FILE)).unwrap();
    let first: Envelope = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert!(matches!(first.body, EventBody::Baseline { .. }));
    assert_eq!(first.link.hash, baseline.hash);
}

/// `EventBody::Seal` and `Baseline` are their writers' to build, and `append_chain` will not take one: the position and once-only
/// rules are enforced by those methods, and a body chained around them would carry
/// none of them. A second seal link gives an acknowledgement two heads to anchor;
/// a hand-chained baseline is a "what the workspace held at t0" claim taken at any
/// seq at all.
///
/// The guard belongs here rather than in one caller, so that it travels with the
/// `TraceExport` to every holder.
#[test]
fn append_chain_refuses_every_body_the_export_reserves() {
    let tmp = tempfile::tempdir().unwrap();
    let mut x = capture(tmp.path());

    let mut seal = reserved(
        "2026-09-08T00:00:00Z",
        EventBody::Seal {
            manifest_sha256: manifest_sha256(&manifest()).unwrap(),
            sealed_at: "2026-09-08T00:00:00Z".to_string(),
            file_count: 0,
        },
    );
    assert!(
        matches!(
            x.append_chain(&mut seal),
            Err(ExportError::ReservedBody { kind: "seal" })
        ),
        "a hand-assembled seal is the seal writer's to build"
    );

    let mut baseline = reserved(
        "2026-09-04T00:00:00Z",
        EventBody::Baseline {
            manifest_sha256: "00".repeat(32),
            file_count: 0,
        },
    );
    assert!(
        matches!(
            x.append_chain(&mut baseline),
            Err(ExportError::ReservedBody { kind: "baseline" })
        ),
        "a hand-assembled baseline is the baseline writer's to build"
    );

    // No refusal wrote a line or moved the head, so the baseline the export builds
    // itself is still the chain's first link — and the seal still closes it.
    assert_eq!(x.head(), None);
    assert_eq!(
        std::fs::read_to_string(tmp.path().join(CHAIN_FILE)).unwrap(),
        "",
        "a refused append leaves no link behind"
    );
    assert_eq!(
        x.write_baseline_for_protocol(V5, &manifest())
            .unwrap()
            .1
            .seq,
        0
    );
    assert_eq!(x.write_seal_for_protocol(V5, &manifest()).unwrap().1.seq, 1);
    assert_eq!(
        chain_lines(tmp.path()),
        2,
        "the two methods that own these bodies still chain them"
    );
}

/// `resume` streams: it must recover the position of a chain far larger than any buffer a
/// reader would allocate, because the recovery path runs on a 168-hour run whose
/// `chain.jsonl` runs to tens of gigabytes. 100,000 links is not that size, but a
/// reader that held the file would have to hold this one too — the test is that the answer
/// is right at a length no `read_to_string` test in this file reaches.
#[test]
fn resume_recovers_the_position_of_a_chain_far_longer_than_a_read_buffer() {
    const LINKS: u64 = 100_000;
    let tmp = tempfile::tempdir().unwrap();
    let (mut x, _) = started(tmp.path());
    let mut last = String::new();
    for i in 0..LINKS {
        last = x.append_chain(&mut env(i)).unwrap().hash;
    }
    drop(x);

    let mut y = reopen(tmp.path());
    assert_eq!(
        y.head(),
        Some((LINKS, last.clone())),
        "the resumed head is the last link of the long chain"
    );
    let link = y.append_chain(&mut env(LINKS)).unwrap();
    assert_eq!(link.seq, LINKS + 1, "the chain continues at the next seq");
    assert_eq!(link.prev, last, "over the link the scan stopped on");
}

#[test]
fn resume_refuses_a_complete_final_envelope_without_its_newline() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut export, _) = started(tmp.path());
    export.append_chain(&mut env(1)).unwrap();
    drop(export);

    let chain_path = tmp.path().join(CHAIN_FILE);
    let mut unterminated = std::fs::read(&chain_path).unwrap();
    assert_eq!(unterminated.pop(), Some(b'\n'));
    std::fs::write(&chain_path, &unterminated).unwrap();

    assert!(
        matches!(
            TraceExport::resume(tmp.path()),
            Err(ExportError::ChainCorrupt { .. })
        ),
        "a complete JSON value is still an incomplete JSONL record without its delimiter"
    );
    assert_eq!(
        std::fs::read(&chain_path).unwrap(),
        unterminated,
        "refusing an incomplete tail must not repair or rewrite evidence"
    );
}

/// A terminal closes the chain for resume as it does for the writer: a correctly linked
/// record, baseline or second seal after the seal makes the chain unresumable, and the
/// refusal rewrites nothing.
#[test]
fn resume_rejects_every_correctly_linked_record_after_the_terminal() {
    for illegal_tail in [
        env(2),
        reserved(
            "2026-09-10T00:00:01Z",
            EventBody::Baseline {
                manifest_sha256: "00".repeat(32),
                file_count: 0,
            },
        ),
        reserved(
            "2026-09-10T00:00:01Z",
            EventBody::Seal {
                manifest_sha256: "00".repeat(32),
                sealed_at: "2026-09-10T00:00:01Z".to_string(),
                file_count: 0,
            },
        ),
    ] {
        let after_terminal = tempfile::tempdir().unwrap();
        let (mut export, _) = started(after_terminal.path());
        let (_, seal_link) = export.write_seal_for_protocol(V5, &manifest()).unwrap();
        drop(export);
        append_correctly_linked_test_envelope(
            after_terminal.path(),
            Some(&seal_link),
            illegal_tail,
        );
        assert_resume_refuses_without_rewriting(after_terminal.path(), 2);
    }
}

/// Only v5 is written. A retired version 1–4 is a format that does not belong in this
/// chain; any other is unknown. Either is refused before anything is hashed or written.
#[test]
fn writers_refuse_every_version_but_five_before_they_hash_or_write_any_evidence() {
    let ordinary = tempfile::tempdir().unwrap();
    let (mut export, _) = started(ordinary.path());
    let before = regular_file_snapshot(ordinary.path());
    for (version, retired) in [(1, true), (4, true), (0, false), (6, false)] {
        let mut record = env(1);
        record.protocol_version = version;
        let refused = export.append_chain(&mut record);
        if retired {
            assert!(
                matches!(refused, Err(ExportError::InvalidCaptureEnvelope { .. })),
                "v{version}: {refused:?}"
            );
        } else {
            assert!(
                matches!(
                    refused,
                    Err(ExportError::UnsupportedProtocolVersion { protocol_version, .. })
                        if protocol_version == version
                ),
                "v{version}: {refused:?}"
            );
        }
    }
    assert!(matches!(
        export.write_seal_for_protocol(6, &manifest()),
        Err(ExportError::UnsupportedProtocolVersion {
            protocol_version: 6,
            ..
        })
    ));
    assert_eq!(regular_file_snapshot(ordinary.path()), before);

    let baseline = tempfile::tempdir().unwrap();
    let mut export = capture(baseline.path());
    let before = regular_file_snapshot(baseline.path());
    assert!(matches!(
        export.write_baseline_for_protocol(6, &manifest()),
        Err(ExportError::UnsupportedProtocolVersion {
            protocol_version: 6,
            ..
        })
    ));
    assert_eq!(regular_file_snapshot(baseline.path()), before);
}

/// Resume reads each line's version before its body: an unknown version is refused as a
/// version even when its hash is stale, a retired version as a line that does not belong,
/// and a v5 line carrying envelope redaction as one no v5 writer produces. None of the
/// refusals touches the directory, not even a stale descriptor temporary.
#[test]
fn resume_rejects_unsupported_retired_or_redacted_envelopes_without_mutation() {
    let unsupported = tempfile::tempdir().unwrap();
    let (export, first_link) = started(unsupported.path());
    drop(export);
    let mut future = env(1);
    future.protocol_version = 6;
    future.link.seq = 1;
    future.link.prev = first_link.hash.clone();
    // Deliberately stale: resume must identify v6 from its shallow header before treating
    // the hash as a chain-corruption claim.
    future.link.hash = first_link.hash;
    append_line(&unsupported.path().join(CHAIN_FILE), &future);
    let stale_descriptor_temp = unsupported
        .path()
        .join(format!("{DESCRIPTOR_FILE}.999999.777.tmp"));
    std::fs::write(
        &stale_descriptor_temp,
        b"must not be swept before v6 is refused",
    )
    .unwrap();
    let before = regular_file_snapshot(unsupported.path());
    assert!(matches!(
        TraceExport::resume(unsupported.path()),
        Err(ExportError::UnsupportedProtocolVersion {
            protocol_version: 6,
            ..
        })
    ));
    assert_eq!(regular_file_snapshot(unsupported.path()), before);

    let mut retired = env(1);
    retired.protocol_version = 4;
    let mut redacted = env(1);
    redacted.redaction = Some(RedactionAudit { spans: Vec::new() });
    for (name, tail) in [("retired", retired), ("redacted", redacted)] {
        let root = tempfile::tempdir().unwrap();
        let (export, baseline) = started(root.path());
        drop(export);
        append_correctly_linked_test_envelope(root.path(), Some(&baseline), tail);
        let before = regular_file_snapshot(root.path());
        match TraceExport::resume(root.path()) {
            Err(ExportError::InvalidCaptureEnvelope { seq: 1, .. }) => {}
            other => panic!("a {name} line does not resume: {other:?}"),
        }
        assert_eq!(regular_file_snapshot(root.path()), before, "{name}");
    }
}
