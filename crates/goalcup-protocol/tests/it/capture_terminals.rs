//! Explicit terminal writers for protocol-v5 capture chains.
//!
//! Covers the versioned cutoff/finalization writers, the local-only `LocalFinish` terminal
//! primitive, and the public phase replay a reader applies to the same rules. Every
//! fixture is a real `TraceExport`; every assertion reads the bytes back off disk.

use goalcup_protocol::capture::{
    CaptureRecord, CaptureTime, GapReason, M2_PROTOCOL_VERSION, StreamKey,
};
use goalcup_protocol::chain::Link;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::export::*;
use goalcup_protocol::provenance::{
    CaptureUnavailableV1, CutoffReachedV1, FinalizationV1, NoRepositoryReasonV1,
    PreCutoffDrainOutcomeV1, ProvenanceError, PublisherNotAttemptedReasonV1,
    PublisherNotAttemptedV1, RepositoryNonSelectionV1, Rfc3339Utc, Sha256Digest,
    SnapshotNotAttemptedV1, TerminationFailureV1, TerminationMethodV1, TerminationOutcomeV1,
};
use goalcup_protocol::seal::{SealManifest, SealedFile};
use sha2::{Digest, Sha256};
use std::path::Path;

const ENTRY: &str = "entry-m2-0001";
const PRODUCER: &str = "0123456789abcdef0123456789abcdef";
const DAEMON_CLOCK: &str = "daemon-5b0c3a1e";
const QUOTA: u64 = 64 * 1024 * 1024;

fn at(mono_ns: u64) -> CaptureTime {
    CaptureTime {
        wall: "2026-09-27T12:00:05.500Z".to_string(),
        mono_ns,
        clock_id: DAEMON_CLOCK.to_string(),
    }
}

fn utc(value: &str) -> Rfc3339Utc {
    Rfc3339Utc::try_from(value).unwrap()
}

fn manifest() -> SealManifest {
    // Deliberately unsorted: the writer binds the canonical (sorted) manifest.
    SealManifest {
        entry_id: ENTRY.to_string(),
        sealed_at: "2026-09-27T12:00:04Z".to_string(),
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
        excluded: vec!["target".to_string(), ".git".to_string()],
    }
}

fn confirmed() -> TerminationOutcomeV1 {
    TerminationOutcomeV1::Confirmed {
        requested_at: utc("2026-09-27T12:00:02Z"),
        confirmed_at: utc("2026-09-27T12:00:03.125Z"),
        method: TerminationMethodV1::ProcessSignal,
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

fn gap(mono_ns: u64) -> CaptureRecord {
    CaptureRecord::Gap {
        stream: Some(StreamKey {
            producer: PRODUCER.to_string(),
            stream: 1,
        }),
        observed: at(mono_ns),
        reason: GapReason::ProducerLost,
        lost_bytes: None,
    }
}

fn local_finish_record(digest: &str) -> CaptureRecord {
    CaptureRecord::LocalFinish {
        observed: at(900),
        reason: "participant_stop".to_string(),
        termination: confirmed(),
        seal_manifest_sha256: digest.to_string(),
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

fn envelopes(root: &Path) -> Vec<Envelope> {
    std::fs::read_to_string(root.join(CHAIN_FILE))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Recompute every link from GENESIS: contiguous seq, prev naming the link before, and a
/// hash covering the envelope. Returns the chain for further inspection.
fn assert_recomputed_chain(root: &Path) -> Vec<Envelope> {
    let chain = envelopes(root);
    let mut prev = goalcup_protocol::chain::GENESIS.to_string();
    for (index, env) in chain.iter().enumerate() {
        assert_eq!(env.link.seq, index as u64);
        assert_eq!(env.link.prev, prev);
        assert_eq!(env.link.hash, env.expected_hash(&prev).unwrap());
        prev = env.link.hash.clone();
    }
    chain
}

/// Replay the public phase rules over the retained chain, as an independent reader does.
fn replay(root: &Path) -> Result<ChainPhaseReplay, ExportError> {
    let mut replay = ChainPhaseReplay::new();
    for env in envelopes(root) {
        replay.check_version(env.link.seq, env.protocol_version)?;
        replay.push(&env)?;
    }
    Ok(replay)
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

fn append_raw(root: &Path, bytes: &[u8]) {
    use std::io::Write as _;
    std::fs::OpenOptions::new()
        .append(true)
        .open(root.join(CHAIN_FILE))
        .unwrap()
        .write_all(bytes)
        .unwrap();
}

fn linked_line(head: &(u64, String), mut env: Envelope) -> Vec<u8> {
    env.link.seq = head.0 + 1;
    env.link.prev = head.1.clone();
    env.link.hash = env.expected_hash(&head.1).unwrap();
    let mut line = serde_json::to_vec(&env).unwrap();
    line.push(b'\n');
    line
}

fn receipt_digest() -> Sha256Digest {
    // A receipt digest computed from actual bytes, shared by the cutoff and finalization.
    Sha256Digest::from_bytes(Sha256::digest(b"synthetic start receipt for entry-m2-0001").into())
}

fn cutoff() -> CutoffReachedV1 {
    CutoffReachedV1 {
        receipt_sha256: receipt_digest(),
        deadline_at: utc("2026-09-27T12:00:03Z"),
        pre_cutoff_drain: PreCutoffDrainOutcomeV1::Complete,
    }
}

fn finalization() -> FinalizationV1 {
    FinalizationV1::CaptureUnavailable {
        receipt_sha256: receipt_digest(),
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

#[test]
fn v5_cutoff_and_finalization_keep_version_five() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path());
    export.append_chain(&mut v5(gap(10))).unwrap();
    let cutoff_link = export
        .write_cutoff_for_protocol(M2_PROTOCOL_VERSION, ENTRY, "2026-09-27T12:00:03Z", cutoff())
        .unwrap();
    // Only the bound finalization may follow a cutoff.
    assert!(matches!(
        export.append_chain(&mut v5(gap(11))),
        Err(ExportError::AfterCutoff { cutoff_seq }) if cutoff_seq == cutoff_link.seq
    ));
    assert!(matches!(
        export.write_local_finish(&manifest(), at(12), "participant_stop", confirmed()),
        Err(ExportError::AfterCutoff { .. })
    ));
    let final_link = export
        .write_finalization_for_protocol(
            M2_PROTOCOL_VERSION,
            ENTRY,
            "2026-09-27T12:00:04Z",
            finalization(),
        )
        .unwrap();
    assert_eq!(final_link.seq, cutoff_link.seq + 1);

    let chain = assert_recomputed_chain(tmp.path());
    assert!(
        chain
            .iter()
            .all(|env| env.protocol_version == M2_PROTOCOL_VERSION),
        "every record of a capture chain is protocol v5"
    );
    assert_eq!(chain.len(), 4);
    assert!(matches!(chain[0].body, EventBody::Baseline { .. }));
    assert!(matches!(chain[2].body, EventBody::CutoffReachedV1(ref c) if *c == cutoff()));
    assert_eq!(chain[2].link, cutoff_link);
    assert!(
        matches!(chain[3].body, EventBody::FinalizationV1(ref f) if *f == finalization()),
        "the finalization is the final position"
    );
    assert_eq!(chain[3].link, final_link);
    assert_eq!(
        export.head(),
        Some((final_link.seq, final_link.hash.clone()))
    );

    // Terminal: nothing further, neither a record nor a second terminal.
    let before = snapshot(tmp.path());
    assert!(matches!(
        export.append_chain(&mut v5(gap(13))),
        Err(ExportError::Terminal { terminal_seq }) if terminal_seq == final_link.seq
    ));
    assert!(matches!(
        export.write_local_finish(&manifest(), at(14), "participant_stop", confirmed()),
        Err(ExportError::Terminal { .. })
    ));
    assert_eq!(snapshot(tmp.path()), before);

    // The phase replay and a reopened writer both reconstruct the same terminal.
    let replayed = replay(tmp.path()).unwrap();
    assert_eq!(
        replayed.terminal(),
        Some(ChainTerminal {
            seq: final_link.seq,
            hash: final_link.hash.clone(),
            kind: TerminalKind::FinalizationV1,
        })
    );
    drop(export);
    let mut resumed = TraceExport::resume(tmp.path()).unwrap();
    resumed.configure_capture_quota(QUOTA).unwrap();
    assert!(matches!(
        resumed.append_chain(&mut v5(gap(15))),
        Err(ExportError::Terminal { .. })
    ));
}

/// The reserved writers take a version only so the fixtures can name it; nothing but v5 is
/// admitted, and a refused version writes nothing.
#[test]
fn reserved_writers_refuse_every_version_but_five_before_writing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path());
    let before = snapshot(tmp.path());
    for version in [1, 4] {
        assert!(matches!(
            export.write_cutoff_for_protocol(version, ENTRY, "2026-09-27T12:00:03Z", cutoff()),
            Err(ExportError::InvalidCaptureEnvelope { .. })
        ));
        assert!(matches!(
            export.write_seal_for_protocol(version, &manifest()),
            Err(ExportError::InvalidCaptureEnvelope { .. })
        ));
    }
    assert!(matches!(
        export.write_cutoff_for_protocol(6, ENTRY, "2026-09-27T12:00:03Z", cutoff()),
        Err(ExportError::UnsupportedProtocolVersion {
            protocol_version: 6,
            ..
        })
    ));
    assert_eq!(snapshot(tmp.path()), before);

    let fresh = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(fresh.path()).unwrap();
    export.configure_capture_quota(QUOTA).unwrap();
    let before = snapshot(fresh.path());
    assert!(matches!(
        export.write_baseline_for_protocol(4, &manifest()),
        Err(ExportError::InvalidCaptureEnvelope { .. })
    ));
    assert_eq!(export.head(), None);
    assert_eq!(snapshot(fresh.path()), before);
}

#[test]
fn local_finish_without_receipt_closes_chain() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path());
    export.append_chain(&mut v5(gap(10))).unwrap();
    export
        .append_chain(&mut v5(CaptureRecord::ProducerClosed {
            producer: PRODUCER.to_string(),
            observed: at(11),
            final_seq: Some(3),
        }))
        .unwrap();

    let (path, link) = export
        .write_local_finish(&manifest(), at(900), "participant_stop", confirmed())
        .unwrap();
    assert_eq!(path, tmp.path().join(SEAL_FILE));

    let seal_bytes = std::fs::read(tmp.path().join(SEAL_FILE)).unwrap();
    let canonical = manifest().sorted();
    assert_eq!(seal_bytes, serde_json::to_vec(&canonical).unwrap());
    let digest = hex::encode(Sha256::digest(&seal_bytes));
    assert_eq!(digest, manifest_sha256(&manifest()).unwrap());

    let chain = assert_recomputed_chain(tmp.path());
    let terminal = chain.last().unwrap();
    assert_eq!(terminal.link, link, "the local finish is the final link");
    assert_eq!(terminal.protocol_version, M2_PROTOCOL_VERSION);
    assert_eq!(terminal.entry_id, ENTRY);
    assert_eq!(terminal.recorded_at, at(900).wall);
    assert_eq!(terminal.redaction, None);
    assert_eq!(
        terminal.body,
        EventBody::Capture {
            record: CaptureRecord::LocalFinish {
                observed: at(900),
                reason: "participant_stop".to_string(),
                termination: confirmed(),
                seal_manifest_sha256: digest,
            }
        }
    );
    assert_eq!(
        chain
            .iter()
            .filter(|env| matches!(
                env.body,
                EventBody::Capture {
                    record: CaptureRecord::LocalFinish { .. }
                } | EventBody::Seal { .. }
                    | EventBody::CutoffReachedV1(_)
                    | EventBody::FinalizationV1(_)
            ))
            .count(),
        1,
        "exactly one terminal record and no seal, cutoff or finalization"
    );
    assert!(check_terminal_manifest(terminal, &manifest()).is_ok());
    for absent in [RECEIPT_FILE, "submission"] {
        assert!(
            !tmp.path().join(absent).exists(),
            "{absent}: no receipt or submission bundle is fabricated"
        );
    }
    assert_eq!(
        std::fs::read(tmp.path().join(ACKS_FILE)).unwrap(),
        b"",
        "no acknowledgement is fabricated"
    );
    let durable = export.capture_storage().durable.unwrap();
    assert_eq!(
        (durable.seq, durable.hash.as_str()),
        (link.seq, link.hash.as_str()),
        "the terminal link is synced before the finish reports success"
    );

    // Closed: later capture, seal, cutoff and terminal writes are refused; artifact preserved.
    let before = snapshot(tmp.path());
    assert!(matches!(
        export.append_chain(&mut v5(gap(12))),
        Err(ExportError::Terminal { terminal_seq }) if terminal_seq == link.seq
    ));
    assert!(matches!(
        export.write_seal_for_protocol(M2_PROTOCOL_VERSION, &manifest()),
        Err(ExportError::Terminal { .. })
    ));
    assert!(matches!(
        export.write_cutoff_for_protocol(
            M2_PROTOCOL_VERSION,
            ENTRY,
            "2026-09-27T12:00:06Z",
            cutoff()
        ),
        Err(ExportError::Terminal { .. })
    ));
    assert_eq!(snapshot(tmp.path()), before);
    assert_eq!(
        replay(tmp.path()).unwrap().terminal(),
        Some(ChainTerminal {
            seq: link.seq,
            hash: link.hash,
            kind: TerminalKind::LocalFinish,
        })
    );
}

#[test]
fn ordinary_append_cannot_forge_local_finish() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path());
    let digest = manifest_sha256(&manifest()).unwrap();
    let before = snapshot(tmp.path());
    assert!(matches!(
        export.append_chain(&mut v5(local_finish_record(&digest))),
        Err(ExportError::ReservedBody {
            kind: "local_finish"
        })
    ));
    assert_eq!(snapshot(tmp.path()), before);
    assert_eq!(export.head().unwrap().0, 0);

    // A hand-written local finish under a retired version is not terminal evidence.
    let head = export.head().unwrap();
    drop(export);
    let mut legacy_labelled = v5(local_finish_record(&digest));
    legacy_labelled.protocol_version = 4;
    legacy_labelled.recorded_at = at(900).wall;
    append_raw(tmp.path(), &linked_line(&head, legacy_labelled));
    assert!(TraceExport::resume(tmp.path()).is_err());
    assert!(replay(tmp.path()).is_err());
}

#[test]
fn duplicate_local_finish_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path());
    let (_, link) = export
        .write_local_finish(&manifest(), at(900), "participant_stop", confirmed())
        .unwrap();
    let before = snapshot(tmp.path());
    assert!(matches!(
        export.write_local_finish(&manifest(), at(901), "participant_stop", confirmed()),
        Err(ExportError::LocalFinishAlreadyChained { seq }) if seq == link.seq
    ));
    assert!(matches!(
        export.write_seal_for_protocol(M2_PROTOCOL_VERSION, &manifest()),
        Err(ExportError::Terminal { .. })
    ));
    assert_eq!(snapshot(tmp.path()), before, "no second terminal record");
}

#[test]
fn reopened_local_finish_stays_terminal() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path());
    export.append_chain(&mut v5(gap(10))).unwrap();
    let (_, link) = export
        .write_local_finish(&manifest(), at(900), "participant_stop", confirmed())
        .unwrap();
    drop(export);

    let before = snapshot(tmp.path());
    let mut resumed = TraceExport::resume(tmp.path()).unwrap();
    assert_eq!(resumed.head(), Some((link.seq, link.hash.clone())));
    resumed.configure_capture_quota(QUOTA).unwrap();
    assert!(matches!(
        resumed.append_chain(&mut v5(gap(11))),
        Err(ExportError::Terminal { terminal_seq }) if terminal_seq == link.seq
    ));
    assert!(matches!(
        resumed.write_local_finish(&manifest(), at(902), "participant_stop", confirmed()),
        Err(ExportError::LocalFinishAlreadyChained { seq }) if seq == link.seq
    ));
    assert!(matches!(
        resumed.write_seal_for_protocol(M2_PROTOCOL_VERSION, &manifest()),
        Err(ExportError::Terminal { .. })
    ));
    assert!(matches!(
        resumed.write_cutoff_for_protocol(
            M2_PROTOCOL_VERSION,
            ENTRY,
            "2026-09-27T12:00:06Z",
            cutoff()
        ),
        Err(ExportError::Terminal { .. })
    ));
    assert_eq!(snapshot(tmp.path()), before);

    // A hand-linked record after the local finish makes the chain unresumable.
    drop(resumed);
    append_raw(
        tmp.path(),
        &linked_line(&(link.seq, link.hash), v5(gap(12))),
    );
    assert!(matches!(
        TraceExport::resume(tmp.path()),
        Err(ExportError::ChainCorrupt { .. })
    ));
    assert!(matches!(
        replay(tmp.path()),
        Err(ExportError::ChainCorrupt { .. })
    ));
}

#[test]
fn local_finish_manifest_mismatch_is_invalid() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path());
    export
        .write_local_finish(&manifest(), at(900), "participant_stop", confirmed())
        .unwrap();
    let terminal = envelopes(tmp.path()).pop().unwrap();
    let stored: SealManifest =
        serde_json::from_slice(&std::fs::read(tmp.path().join(SEAL_FILE)).unwrap()).unwrap();
    assert!(check_terminal_manifest(&terminal, &stored).is_ok());

    // seal.json changed after close: one added file, one changed digest, another entry.
    let mut added = stored.clone();
    added.files.push(SealedFile {
        path: "planted.rs".to_string(),
        bytes: 1,
        sha256: "ee".repeat(32),
    });
    let mut changed = stored.clone();
    changed.files[0].sha256 = "ff".repeat(32);
    let mut other_entry = stored.clone();
    other_entry.entry_id = "entry-m2-0002".to_string();
    for mismatched in [added, changed, other_entry] {
        std::fs::write(
            tmp.path().join(SEAL_FILE),
            serde_json::to_vec(&mismatched).unwrap(),
        )
        .unwrap();
        let reread: SealManifest =
            serde_json::from_slice(&std::fs::read(tmp.path().join(SEAL_FILE)).unwrap()).unwrap();
        assert!(matches!(
            check_terminal_manifest(&terminal, &reread),
            Err(ExportError::TerminalManifestMismatch)
        ));
    }
    // A non-terminal record binds no manifest at all.
    let baseline = envelopes(tmp.path()).remove(0);
    assert!(matches!(
        check_terminal_manifest(&baseline, &stored),
        Err(ExportError::TerminalManifestMismatch)
    ));
}

#[test]
fn local_finish_artifact_failure_is_not_success() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path());
    // Something else put a seal.json in the trace after creation.
    std::fs::write(tmp.path().join(SEAL_FILE), b"not ours").unwrap();

    let failed = export.write_local_finish(&manifest(), at(900), "participant_stop", confirmed());
    assert!(
        failed.is_err(),
        "a missing fixed manifest is not a successful finish"
    );
    assert_eq!(
        std::fs::read(tmp.path().join(SEAL_FILE)).unwrap(),
        b"not ours",
        "existing artifact bytes are preserved, never overwritten"
    );
    let chain = envelopes(tmp.path());
    assert!(
        matches!(
            chain.last().unwrap().body,
            EventBody::Capture {
                record: CaptureRecord::LocalFinish { .. }
            }
        ),
        "the one terminal link stays chained"
    );
    let terminal_seq = chain.last().unwrap().link.seq;
    let terminal = chain.last().unwrap().clone();
    assert!(matches!(
        check_terminal_manifest(&terminal, &manifest()),
        Ok(())
    ));
    assert!(
        std::fs::read(tmp.path().join(SEAL_FILE))
            .map(|bytes| serde_json::from_slice::<SealManifest>(&bytes).is_err())
            .unwrap(),
        "the fixed file on disk does not bind the terminal, so integrity is incomplete"
    );

    // A retry observes the committed terminal rather than writing a second one.
    let before = snapshot(tmp.path());
    assert!(matches!(
        export.write_local_finish(&manifest(), at(901), "participant_stop", confirmed()),
        Err(ExportError::LocalFinishAlreadyChained { seq }) if seq == terminal_seq
    ));
    assert_eq!(snapshot(tmp.path()), before);
}

#[test]
fn a_failed_seal_write_still_leaves_the_terminal_durable() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path());
    export.append_chain(&mut v5(gap(10))).unwrap();
    let pre_finish = export.sync_capture().unwrap().unwrap();
    // `seal.json` is created absent-only, so an empty one already there fails the seal write.
    std::fs::write(tmp.path().join(SEAL_FILE), b"").unwrap();

    let failed = export.write_local_finish(&manifest(), at(900), "participant_stop", confirmed());
    assert!(failed.is_err(), "the seal write fails: {failed:?}");
    assert_eq!(std::fs::read(tmp.path().join(SEAL_FILE)).unwrap(), b"");

    let terminal = envelopes(tmp.path()).pop().unwrap();
    assert!(matches!(
        terminal.body,
        EventBody::Capture {
            record: CaptureRecord::LocalFinish { .. }
        }
    ));
    assert_eq!(terminal.link.seq, pre_finish.seq + 1);
    assert_eq!(
        export
            .capture_storage()
            .durable
            .map(|frontier| (frontier.seq, frontier.hash)),
        Some((terminal.link.seq, terminal.link.hash)),
        "the terminal is flushed before the seal write, so its failure cannot strand it"
    );
}

#[test]
fn local_finish_refuses_invalid_inputs_before_writing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path());
    let before = snapshot(tmp.path());

    let reversed = TerminationOutcomeV1::Confirmed {
        requested_at: utc("2026-09-27T12:00:03Z"),
        confirmed_at: utc("2026-09-27T12:00:02Z"),
        method: TerminationMethodV1::ProcessSignal,
    };
    assert!(matches!(
        export.write_local_finish(&manifest(), at(900), "participant_stop", reversed),
        Err(ExportError::Provenance(
            ProvenanceError::InvalidField { .. }
        ))
    ));
    let reversed_wait = TerminationOutcomeV1::Unresolved {
        requested_at: utc("2026-09-27T12:00:03Z"),
        stopped_waiting_at: utc("2026-09-27T12:00:02.999Z"),
        reason: TerminationFailureV1::ConfirmationTimedOut,
    };
    assert!(matches!(
        export.write_local_finish(&manifest(), at(900), "participant_stop", reversed_wait),
        Err(ExportError::Provenance(
            ProvenanceError::InvalidField { .. }
        ))
    ));
    for reason in ["", "line\nbreak"] {
        assert!(matches!(
            export.write_local_finish(&manifest(), at(900), reason, confirmed()),
            Err(ExportError::CaptureRecord(_))
        ));
    }
    let long = "r".repeat(goalcup_protocol::capture::MAX_METADATA_STRING_BYTES + 1);
    assert!(matches!(
        export.write_local_finish(&manifest(), at(900), &long, confirmed()),
        Err(ExportError::CaptureRecord(_))
    ));
    let mut bad_time = at(900);
    bad_time.wall = "yesterday".to_string();
    assert!(matches!(
        export.write_local_finish(&manifest(), bad_time, "participant_stop", confirmed()),
        Err(ExportError::CaptureRecord(_))
    ));
    assert_eq!(snapshot(tmp.path()), before, "nothing was written");

    // An unresolved termination is recorded as exactly that, never as a confirmed stop.
    let unresolved = TerminationOutcomeV1::Unresolved {
        requested_at: utc("2026-09-27T12:00:02Z"),
        stopped_waiting_at: utc("2026-09-27T12:00:12Z"),
        reason: TerminationFailureV1::ProcessStillRunning,
    };
    export
        .write_local_finish(&manifest(), at(900), "participant_stop", unresolved.clone())
        .unwrap();
    assert!(matches!(
        envelopes(tmp.path()).pop().unwrap().body,
        EventBody::Capture {
            record: CaptureRecord::LocalFinish { termination, .. }
        } if termination == unresolved
    ));
}

#[test]
fn phase_replay_applies_the_writer_terminal_rules() {
    // A v5 legacy-style seal is a LegacySeal terminal of a capture chain.
    let sealed = tempfile::tempdir().unwrap();
    let mut export = capture_export(sealed.path());
    let (_, seal_link) = export
        .write_seal_for_protocol(M2_PROTOCOL_VERSION, &manifest())
        .unwrap();
    let replayed = replay(sealed.path()).unwrap();
    assert_eq!(replayed.baseline_seq(), Some(0));
    assert_eq!(
        replayed.terminal(),
        Some(ChainTerminal {
            seq: seal_link.seq,
            hash: seal_link.hash.clone(),
            kind: TerminalKind::LegacySeal,
        })
    );
    let terminal = envelopes(sealed.path()).pop().unwrap();
    assert!(check_terminal_manifest(&terminal, &manifest()).is_ok());
    let mut other = manifest();
    other.sealed_at = "2026-09-27T12:00:09Z".to_string();
    assert!(check_terminal_manifest(&terminal, &other).is_err());

    // Unsealed chains replay with no terminal.
    let open = tempfile::tempdir().unwrap();
    let mut export = capture_export(open.path());
    export.append_chain(&mut v5(gap(1))).unwrap();
    let head = export.head().unwrap();
    assert_eq!(replay(open.path()).unwrap().terminal(), None);
    drop(export);

    // A hand-linked local finish with an invalid field is refused by the replay and resume.
    let mut bad_digest = v5(local_finish_record("not-a-digest"));
    bad_digest.recorded_at = at(900).wall;
    let forged = tempfile::tempdir().unwrap();
    std::fs::copy(open.path().join(CHAIN_FILE), forged.path().join(CHAIN_FILE)).unwrap();
    append_raw(forged.path(), &linked_line(&head, bad_digest));
    assert!(matches!(
        replay(forged.path()),
        Err(ExportError::ChainCorrupt { .. })
    ));
    assert!(matches!(
        TraceExport::resume(forged.path()),
        Err(ExportError::ChainCorrupt { .. })
    ));

    // The envelope time must be the local finish observation time.
    let mut shifted = v5(local_finish_record(&manifest_sha256(&manifest()).unwrap()));
    shifted.recorded_at = "2026-09-27T12:00:07Z".to_string();
    let forged = tempfile::tempdir().unwrap();
    std::fs::copy(open.path().join(CHAIN_FILE), forged.path().join(CHAIN_FILE)).unwrap();
    append_raw(forged.path(), &linked_line(&head, shifted));
    assert!(matches!(
        replay(forged.path()),
        Err(ExportError::ChainCorrupt { .. })
    ));

    // A correctly formed hand-linked local finish is the terminal the reader reports.
    let mut formed = v5(local_finish_record(&manifest_sha256(&manifest()).unwrap()));
    formed.recorded_at = at(900).wall;
    let forged = tempfile::tempdir().unwrap();
    std::fs::copy(open.path().join(CHAIN_FILE), forged.path().join(CHAIN_FILE)).unwrap();
    append_raw(forged.path(), &linked_line(&head, formed));
    assert_eq!(
        replay(forged.path()).unwrap().terminal().map(|t| t.kind),
        Some(TerminalKind::LocalFinish)
    );

    // Version admission before decoding: a retired version after v5, an unknown version, and a
    // retired-version genesis.
    let mut capture = ChainPhaseReplay::new();
    capture.check_version(0, M2_PROTOCOL_VERSION).unwrap();
    capture.push(&envelopes(open.path())[0]).unwrap();
    assert!(matches!(
        capture.check_version(1, 4),
        Err(ExportError::InvalidCaptureEnvelope { seq: 1, .. })
    ));
    assert!(matches!(
        capture.check_version(1, 6),
        Err(ExportError::UnsupportedProtocolVersion {
            protocol_version: 6,
            ..
        })
    ));
    let mut retired = ChainPhaseReplay::new();
    assert!(matches!(
        retired.check_version(0, 4),
        Err(ExportError::InvalidCaptureEnvelope { seq: 0, .. })
    ));
    let mut retired_baseline = envelopes(open.path())[0].clone();
    retired_baseline.protocol_version = 4;
    assert!(matches!(
        retired.push(&retired_baseline),
        Err(ExportError::InvalidCaptureEnvelope { seq: 0, .. })
    ));
}

/// A hand-written LocalFinish that is otherwise well formed: v5, envelope time equal to its
/// observation, bound to the canonical manifest digest.
fn hand_local_finish(reason: &str, termination: TerminationOutcomeV1) -> Envelope {
    let mut env = v5(CaptureRecord::LocalFinish {
        observed: at(900),
        reason: reason.to_string(),
        termination,
        seal_manifest_sha256: manifest_sha256(&manifest()).unwrap(),
    });
    env.recorded_at = at(900).wall;
    env
}

/// Hand-link `forged` after the chain in `root` (head `head`). Both the public replay and
/// resume must refuse it as `ChainCorrupt` at its own seq, and neither may mutate the trace.
fn assert_hand_linked_local_finish_refused(
    root: &Path,
    head: (u64, String),
    forged: Envelope,
    case: &str,
) {
    append_raw(root, &linked_line(&head, forged));
    let before = snapshot(root);
    let seq = head.0 + 1;
    match replay(root) {
        Err(ExportError::ChainCorrupt { seq: refused, .. }) if refused == seq => {}
        other => panic!("{case}: the replay must refuse seq {seq}: {other:?}"),
    }
    match TraceExport::resume(root) {
        Err(ExportError::ChainCorrupt { seq: refused, .. }) if refused == seq => {}
        other => panic!("{case}: resume must refuse seq {seq}: {other:?}"),
    }
    assert_eq!(snapshot(root), before, "{case}: nothing was mutated");
}

#[test]
fn replay_refuses_a_local_finish_after_a_cutoff() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path());
    export
        .write_cutoff_for_protocol(M2_PROTOCOL_VERSION, ENTRY, "2026-09-27T12:00:03Z", cutoff())
        .unwrap();
    let head = export.head().unwrap();
    drop(export);
    assert_eq!(replay(tmp.path()).unwrap().cutoff_seq(), Some(head.0));
    assert_hand_linked_local_finish_refused(
        tmp.path(),
        head,
        hand_local_finish("participant_stop", confirmed()),
        "local finish after a signed-mode cutoff",
    );
}

#[test]
fn replay_refuses_a_local_finish_after_another_terminal() {
    // A second local finish.
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path());
    export
        .write_local_finish(&manifest(), at(900), "participant_stop", confirmed())
        .unwrap();
    let head = export.head().unwrap();
    drop(export);
    assert_hand_linked_local_finish_refused(
        tmp.path(),
        head,
        hand_local_finish("participant_stop", confirmed()),
        "second local finish",
    );

    // A local finish after a v5 legacy-style seal.
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path());
    export
        .write_seal_for_protocol(M2_PROTOCOL_VERSION, &manifest())
        .unwrap();
    let head = export.head().unwrap();
    drop(export);
    assert_hand_linked_local_finish_refused(
        tmp.path(),
        head,
        hand_local_finish("participant_stop", confirmed()),
        "local finish after a seal",
    );

    // A local finish after a signed finalization.
    let tmp = tempfile::tempdir().unwrap();
    let mut export = capture_export(tmp.path());
    export
        .write_cutoff_for_protocol(M2_PROTOCOL_VERSION, ENTRY, "2026-09-27T12:00:03Z", cutoff())
        .unwrap();
    export
        .write_finalization_for_protocol(
            M2_PROTOCOL_VERSION,
            ENTRY,
            "2026-09-27T12:00:04Z",
            finalization(),
        )
        .unwrap();
    let head = export.head().unwrap();
    drop(export);
    assert_hand_linked_local_finish_refused(
        tmp.path(),
        head,
        hand_local_finish("participant_stop", confirmed()),
        "local finish after a finalization",
    );
}

#[test]
fn replay_refuses_a_local_finish_with_invalid_termination_or_reason() {
    let open_chain = || {
        let tmp = tempfile::tempdir().unwrap();
        let mut export = capture_export(tmp.path());
        export.append_chain(&mut v5(gap(1))).unwrap();
        let head = export.head().unwrap();
        drop(export);
        (tmp, head)
    };

    // Control: the same hand-written record with valid fields is the reported terminal, so
    // each refusal below is the field rule under test and nothing incidental.
    let (tmp, head) = open_chain();
    append_raw(
        tmp.path(),
        &linked_line(&head, hand_local_finish("participant_stop", confirmed())),
    );
    assert_eq!(
        replay(tmp.path()).unwrap().terminal().map(|t| t.kind),
        Some(TerminalKind::LocalFinish)
    );
    assert!(TraceExport::resume(tmp.path()).is_ok());

    let reversed_confirmed = TerminationOutcomeV1::Confirmed {
        requested_at: utc("2026-09-27T12:00:03Z"),
        confirmed_at: utc("2026-09-27T12:00:02Z"),
        method: TerminationMethodV1::ProcessSignal,
    };
    let reversed_unresolved = TerminationOutcomeV1::Unresolved {
        requested_at: utc("2026-09-27T12:00:03Z"),
        stopped_waiting_at: utc("2026-09-27T12:00:02.999Z"),
        reason: TerminationFailureV1::ConfirmationTimedOut,
    };
    for (case, forged) in [
        (
            "reversed confirmed termination",
            hand_local_finish("participant_stop", reversed_confirmed),
        ),
        (
            "reversed unresolved termination",
            hand_local_finish("participant_stop", reversed_unresolved),
        ),
        ("empty reason", hand_local_finish("", confirmed())),
        (
            "control character in reason",
            hand_local_finish("line\nbreak", confirmed()),
        ),
    ] {
        let (tmp, head) = open_chain();
        assert_hand_linked_local_finish_refused(tmp.path(), head, forged, case);
    }
}
