//! `TraceExport` owns the bytes of a trace directory, so every assertion here reads the
//! bytes back off disk. A test that only checks the returned `Link` would pass against a
//! writer that never wrote a file.

use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::chain::{GENESIS, Link, link_hash};
use goalcup_protocol::envelope::{
    Envelope, EnvelopeVersionCompatibilityError, EventBody, V2OnlyEventBody,
};
use goalcup_protocol::events::ForeignScanOutcome;
use goalcup_protocol::export::*;
use goalcup_protocol::provenance::{
    CaptureUnavailableV1, CompleteEnumerationV1, CutoffReachedV1, FinalizationV1, GitObjectFormat,
    GitObjectId, GitRemoteTransportV1, NoRepositoryReasonV1, NormalizedGitHubRemote,
    ObservedRepositoryStateV1, PortableRelativePathOrRoot, PreCutoffDrainOutcomeV1, PreSealHeadV1,
    PublisherNotAttemptedReasonV1, PublisherNotAttemptedV1, RepositoryEligibilityPolicyV1,
    RepositoryEligibilityReleasePolicyV1, RepositoryFinalizationV1, RepositoryNonSelectionV1,
    RepositoryObservedV1, RepositoryTransitionV1, Rfc3339Utc, SelectedRepositoryV1, Sha256Digest,
    SnapshotCompletedV1, SnapshotNotAttemptedV1, TerminationOutcomeV1, ValidatedHeadsRef,
};
use goalcup_protocol::receipt::{StartReceipt, StartReceiptClaims};
use goalcup_protocol::redaction::{RedactionAudit, RedactionAuditSpan, RedactionRecord};
use goalcup_protocol::seal::{SealManifest, SealedFile};
use goalcup_protocol::submission::{
    EffectiveSubmissionLimitsV1, FinishReasonV1, LimitProfileV1, SubmissionCapturedV1,
    SubmissionFailureV1, SubmissionManifestV1, SubmissionOutcomeV1, SubmissionProfileV1,
};
use std::io::Read;
use std::path::Path;

const HISTORICAL_V1_HOME_SEEDED: &str = include_str!("fixtures/historical-v1-home-seeded.jsonl");
const HISTORICAL_V1_HOME_SEEDED_HASH: &str =
    "5658cadb5d2aa41e5978bc139d9167b7097f9e2cc737404847e2a35d46a0064a";
const HISTORICAL_V1_HOME_SEEDED_CHAIN_BYTES: &[u8] = b"goalcup-chain-v1\0{\"protocol_version\":1,\"entry_id\":\"e1\",\"source\":\"daemon\",\"recorded_at\":\"2026-09-10T00:00:00Z\",\"seq\":0,\"body\":{\"kind\":\"home_seeded\",\"content\":{\"connector\":\"fixture\",\"from\":\"{home}/.codex\",\"keys\":[{\"name\":\"theme\",\"value\":\"dark\"}],\"keys_kept\":[\"theme\"],\"files\":[{\"name\":\"auth.json\",\"bytes\":3,\"sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}],\"files_kept\":[],\"files_missing\":[]}}}";

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

fn env(source: &str, at: &str, body: EventBody) -> Envelope {
    Envelope {
        protocol_version: goalcup_protocol::PROTOCOL_VERSION,
        entry_id: "e1".to_string(),
        source: source.to_string(),
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

fn manifest() -> SealManifest {
    SealManifest {
        entry_id: "e1".to_string(),
        sealed_at: "2026-09-08T00:00:00Z".to_string(),
        workspace: "/ws".to_string(),
        files: Vec::new(),
        excluded: Vec::new(),
    }
}

fn home_seed(
    handoff_generation: Option<u64>,
    handoff_request_digest: Option<&str>,
) -> goalcup_protocol::events::HomeSeedRecord {
    goalcup_protocol::events::HomeSeedRecord {
        handoff_generation,
        handoff_request_digest: handoff_request_digest.map(str::to_string),
        connector: "fixture".to_string(),
        from: "{home}/settings.json".to_string(),
        keys: Vec::new(),
        keys_unchanged: Vec::new(),
        written: Vec::new(),
        files: Vec::new(),
        files_kept: Vec::new(),
        files_missing: Vec::new(),
        wire_layout: Default::default(),
    }
}

/// Fields whose JSON `null` must never be normalized into the same in-memory record as an
/// omitted field.  Each supplied line below carries the hash recomputed over that omitted,
/// writer-produced shape, so a permissive reader would falsely accept it.
#[derive(Clone, Copy)]
enum HomeSeedNullField {
    HandoffGeneration,
    HandoffRequestDigest,
    KeysUnchanged,
    Written,
    KeyValue,
    WrittenKeyValue,
}

impl HomeSeedNullField {
    fn name(self) -> &'static str {
        match self {
            Self::HandoffGeneration => "handoff_generation",
            Self::HandoffRequestDigest => "handoff_request_digest",
            Self::KeysUnchanged => "keys_unchanged",
            Self::Written => "written",
            Self::KeyValue => "keys[].value",
            Self::WrittenKeyValue => "written[].value",
        }
    }
}

fn null_normalized_home_seed_lines() -> Vec<(&'static str, String)> {
    let legacy: Envelope = serde_json::from_str(HISTORICAL_V1_HOME_SEEDED).unwrap();
    let mut current = env(
        "daemon",
        "2026-09-10T00:00:00Z",
        EventBody::HomeSeeded(home_seed(None, None)),
    );
    let EventBody::HomeSeeded(seed) = &mut current.body else {
        unreachable!("the test fixture is a HomeSeeded envelope");
    };
    seed.keys.push(goalcup_protocol::events::SeededKey {
        name: "theme".to_string(),
        value: None,
    });
    seed.written.push(goalcup_protocol::events::SeededKey {
        name: "workspace_trusted".to_string(),
        value: None,
    });

    [
        HomeSeedNullField::HandoffGeneration,
        HomeSeedNullField::HandoffRequestDigest,
        HomeSeedNullField::KeysUnchanged,
        HomeSeedNullField::Written,
    ]
    .into_iter()
    .map(|field| {
        (
            field.name(),
            line_with_explicit_null_and_normalized_hash(legacy.clone(), field),
        )
    })
    .chain(
        [
            HomeSeedNullField::KeyValue,
            HomeSeedNullField::WrittenKeyValue,
        ]
        .into_iter()
        .map(|field| {
            (
                field.name(),
                line_with_explicit_null_and_normalized_hash(current.clone(), field),
            )
        }),
    )
    .collect()
}

fn line_with_explicit_null_and_normalized_hash(
    mut normalized: Envelope,
    field: HomeSeedNullField,
) -> String {
    normalized.link.prev = GENESIS.to_string();
    normalized.link.hash = normalized.expected_hash(GENESIS).unwrap();
    let normalized_hash = normalized.link.hash.clone();
    let mut wire = serde_json::to_value(&normalized).unwrap();
    let content = wire["body"]["content"].as_object_mut().unwrap();

    match field {
        HomeSeedNullField::HandoffGeneration
        | HomeSeedNullField::HandoffRequestDigest
        | HomeSeedNullField::KeysUnchanged
        | HomeSeedNullField::Written => {
            assert!(content.get(field.name()).is_none());
            content.insert(field.name().to_string(), serde_json::Value::Null);
        }
        HomeSeedNullField::KeyValue => {
            let key = content
                .get_mut("keys")
                .and_then(serde_json::Value::as_array_mut)
                .unwrap()
                .first_mut()
                .unwrap()
                .as_object_mut()
                .unwrap();
            assert!(key.get("value").is_none());
            key.insert("value".to_string(), serde_json::Value::Null);
        }
        HomeSeedNullField::WrittenKeyValue => {
            let key = content
                .get_mut("written")
                .and_then(serde_json::Value::as_array_mut)
                .unwrap()
                .first_mut()
                .unwrap()
                .as_object_mut()
                .unwrap();
            assert!(key.get("value").is_none());
            key.insert("value".to_string(), serde_json::Value::Null);
        }
    }

    assert_eq!(wire["hash"], normalized_hash);
    let mut line = serde_json::to_string(&wire).unwrap();
    line.push('\n');
    line
}

fn repository_observed() -> RepositoryObservedV1 {
    RepositoryObservedV1 {
        transition: RepositoryTransitionV1::Disappeared,
        state: ObservedRepositoryStateV1::Disappeared {
            workspace_relative_root: PortableRelativePathOrRoot::try_from("").unwrap(),
        },
    }
}

fn foreign_scan_coverage() -> EventBody {
    EventBody::ForeignScanCoverage {
        declaring_connector: "opencode".to_string(),
        declaring_manifest_sha256: "a".repeat(64),
        scan_index: 0,
        outcome: ForeignScanOutcome::Complete,
        at: "2026-09-10T00:00:00Z".to_string(),
    }
}

fn legacy_redaction() -> EventBody {
    EventBody::Redaction(RedactionRecord {
        store: "main".to_string(),
        target_seq: 0,
        salt: "legacy".to_string(),
        spans: Vec::new(),
    })
}

fn inline_redaction() -> RedactionAudit {
    RedactionAudit {
        spans: vec![RedactionAuditSpan {
            kind: "email".to_string(),
            start: 0,
            end: 1,
        }],
    }
}

fn version_body_incompatibilities() -> Vec<(
    &'static str,
    u32,
    EventBody,
    Option<RedactionAudit>,
    EnvelopeVersionCompatibilityError,
)> {
    vec![
        (
            "v1 HomeSeeded handoff generation",
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            EventBody::HomeSeeded(home_seed(Some(7), None)),
            None,
            EnvelopeVersionCompatibilityError::HomeSeededRequiresV3 {
                protocol_version: goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            },
        ),
        (
            "v1 HomeSeeded handoff digest",
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            EventBody::HomeSeeded(home_seed(None, Some("ab"))),
            None,
            EnvelopeVersionCompatibilityError::HomeSeededRequiresV3 {
                protocol_version: goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            },
        ),
        (
            "v2 HomeSeeded handoff generation",
            goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
            EventBody::HomeSeeded(home_seed(Some(7), None)),
            None,
            EnvelopeVersionCompatibilityError::HomeSeededRequiresV3 {
                protocol_version: goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
            },
        ),
        (
            "v2 HomeSeeded handoff digest",
            goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
            EventBody::HomeSeeded(home_seed(None, Some("ab"))),
            None,
            EnvelopeVersionCompatibilityError::HomeSeededRequiresV3 {
                protocol_version: goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
            },
        ),
        (
            "v1 RepositoryObservedV1",
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            EventBody::RepositoryObservedV1(repository_observed()),
            None,
            EnvelopeVersionCompatibilityError::BodyRequiresV2 {
                body: V2OnlyEventBody::RepositoryObservedV1,
            },
        ),
        (
            "v1 CutoffReachedV1",
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            EventBody::CutoffReachedV1(cutoff("a")),
            None,
            EnvelopeVersionCompatibilityError::BodyRequiresV2 {
                body: V2OnlyEventBody::CutoffReachedV1,
            },
        ),
        (
            "v1 FinalizationV1",
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            EventBody::FinalizationV1(unavailable_finalization("a")),
            None,
            EnvelopeVersionCompatibilityError::BodyRequiresV2 {
                body: V2OnlyEventBody::FinalizationV1,
            },
        ),
        (
            "v1 workspace submission finalization",
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            EventBody::FinalizationV1(workspace_finalization("a")),
            None,
            EnvelopeVersionCompatibilityError::BodyRequiresV2 {
                body: V2OnlyEventBody::FinalizationV1,
            },
        ),
        (
            "v1 ForeignScanCoverage",
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            foreign_scan_coverage(),
            None,
            EnvelopeVersionCompatibilityError::BodyRequiresV2 {
                body: V2OnlyEventBody::ForeignScanCoverage,
            },
        ),
        (
            "v1 inline redaction",
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            EventBody::Leg1Record {
                store: "main".to_string(),
                payload: serde_json::json!({"redacted": true}),
            },
            Some(inline_redaction()),
            EnvelopeVersionCompatibilityError::InlineRedactionRequiresV2,
        ),
        (
            "v2 legacy redaction body",
            goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
            legacy_redaction(),
            None,
            EnvelopeVersionCompatibilityError::LegacyRedactionRequiresV1 {
                protocol_version: goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
            },
        ),
        (
            "v2 workspace submission finalization",
            2,
            EventBody::FinalizationV1(workspace_finalization("a")),
            None,
            EnvelopeVersionCompatibilityError::WorkspaceSubmissionRequiresV4 {
                protocol_version: 2,
            },
        ),
        (
            "v3 workspace submission finalization",
            3,
            EventBody::FinalizationV1(workspace_finalization("a")),
            None,
            EnvelopeVersionCompatibilityError::WorkspaceSubmissionRequiresV4 {
                protocol_version: 3,
            },
        ),
        (
            "v4 legacy redaction body",
            goalcup_protocol::PROTOCOL_VERSION,
            legacy_redaction(),
            None,
            EnvelopeVersionCompatibilityError::LegacyRedactionRequiresV1 {
                protocol_version: goalcup_protocol::PROTOCOL_VERSION,
            },
        ),
    ]
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
fn descriptor(entry_id: &str) -> TraceDescriptorDoc {
    TraceDescriptorDoc {
        connector: "claude-code-cli".to_string(),
        entry_id: entry_id.to_string(),
        workspace: "/ws".to_string(),
        tracked_sessions: vec!["s1".to_string()],
        harness_version: "2.1.259".to_string(),
        connector_manifest: MANIFEST_FILE.to_string(),
        chain: Some(CHAIN_FILE.to_string()),
        receipt: Some(RECEIPT_FILE.to_string()),
        baseline: Some(BASELINE_FILE.to_string()),
        acks: Some(ACKS_FILE.to_string()),
        seal: Some(SEAL_FILE.to_string()),
        server_public_key: Some("ab".repeat(32)),
        expected_reread: false,
    }
}

fn provenance_digest(byte: &str) -> Sha256Digest {
    Sha256Digest::try_from(byte.repeat(64)).unwrap()
}

fn cutoff(receipt_byte: &str) -> CutoffReachedV1 {
    CutoffReachedV1 {
        receipt_sha256: provenance_digest(receipt_byte),
        deadline_at: Rfc3339Utc::try_from("2026-09-10T00:00:00Z").unwrap(),
        pre_cutoff_drain: PreCutoffDrainOutcomeV1::Complete,
    }
}

fn unavailable_finalization(receipt_byte: &str) -> FinalizationV1 {
    FinalizationV1::CaptureUnavailable {
        receipt_sha256: provenance_digest(receipt_byte),
        termination: TerminationOutcomeV1::AlreadyStopped {
            checked_at: Rfc3339Utc::try_from("2026-09-10T00:00:01Z").unwrap(),
        },
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

fn workspace_finalization(receipt_byte: &str) -> FinalizationV1 {
    workspace_finalization_with_digest(receipt_byte, provenance_digest("f"))
}

fn workspace_finalization_with_digest(
    receipt_byte: &str,
    manifest_sha256: Sha256Digest,
) -> FinalizationV1 {
    FinalizationV1::WorkspaceSubmission {
        receipt_sha256: provenance_digest(receipt_byte),
        termination: TerminationOutcomeV1::AlreadyStopped {
            checked_at: Rfc3339Utc::try_from("2026-09-10T00:00:01Z").unwrap(),
        },
        submission: SubmissionOutcomeV1::Captured(SubmissionCapturedV1 { manifest_sha256 }),
    }
}

fn workspace_submission_manifest(cutoff: &Link) -> SubmissionManifestV1 {
    SubmissionManifestV1 {
        profile: SubmissionProfileV1::WorkspaceSnapshotV1,
        limit_profile: LimitProfileV1 {
            id: "workspace-v1".to_string(),
        },
        effective_limits: EffectiveSubmissionLimitsV1 {
            object_count: 1,
            total_bytes: 1,
            pre_cutoff_drain: 1,
            process_walk: 1,
        },
        entry_id: "e1".to_string(),
        receipt_sha256: provenance_digest("a"),
        cutoff_seq: cutoff.seq,
        cutoff_chain_hash: Sha256Digest::try_from(cutoff.hash.as_str()).unwrap(),
        finish_reason: FinishReasonV1::Deadline,
        entries: Vec::new(),
        excluded: Vec::new(),
        copy_started_at: Rfc3339Utc::try_from("2026-09-10T00:00:01Z").unwrap(),
        copy_completed_at: Rfc3339Utc::try_from("2026-09-10T00:00:02Z").unwrap(),
        readback_verified_at: Rfc3339Utc::try_from("2026-09-10T00:00:03Z").unwrap(),
        object_count: 0,
        total_bytes: 0,
    }
}

fn write_workspace_submission_layout(root: &Path, manifest: &SubmissionManifestV1) {
    std::fs::create_dir_all(root.join(SUBMISSION_SHA256_OBJECTS_DIR)).unwrap();
    std::fs::write(
        root.join(SUBMISSION_MANIFEST_FILE),
        manifest.canonical_bytes().unwrap(),
    )
    .unwrap();
}

fn cutoff_export() -> (tempfile::TempDir, TraceExport, Link, SubmissionManifestV1) {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    let cutoff_link = export
        .write_cutoff("e1", "2026-09-10T00:00:00Z", cutoff("a"))
        .unwrap();
    let manifest = workspace_submission_manifest(&cutoff_link);
    (tmp, export, cutoff_link, manifest)
}

fn assert_captured_finalization_refused_without_write(
    tmp: &tempfile::TempDir,
    export: &mut TraceExport,
    finalization: FinalizationV1,
) {
    let before = std::fs::read(tmp.path().join(CHAIN_FILE)).unwrap();
    let head = export.head();
    assert!(matches!(
        export.write_finalization("e1", "2026-09-10T00:00:04Z", finalization),
        Err(ExportError::FinalizationBindingMismatch)
    ));
    assert_eq!(export.head(), head);
    assert_eq!(std::fs::read(tmp.path().join(CHAIN_FILE)).unwrap(), before);
}

fn captured_finalization(receipt_byte: &str) -> FinalizationV1 {
    let selection = SelectedRepositoryV1 {
        workspace_relative_root: PortableRelativePathOrRoot::try_from("").unwrap(),
        remote: NormalizedGitHubRemote {
            host: "github.com".to_string(),
            owner: "agentcommunity".to_string(),
            repository: "goal-cup".to_string(),
            transport: GitRemoteTransportV1::Https,
        },
        target_ref: ValidatedHeadsRef::try_from("refs/heads/main").unwrap(),
        object_format: GitObjectFormat::Sha1,
        pre_seal_head: PreSealHeadV1::Existing {
            commit: GitObjectId::try_from("a".repeat(40)).unwrap(),
            tree: GitObjectId::try_from("b".repeat(40)).unwrap(),
        },
        selection_basis_chain_hash: provenance_digest("c"),
    };
    FinalizationV1::Captured {
        receipt_sha256: provenance_digest(receipt_byte),
        termination: TerminationOutcomeV1::AlreadyStopped {
            checked_at: Rfc3339Utc::try_from("2026-09-10T00:00:01Z").unwrap(),
        },
        selected: Box::new(selection),
        snapshot: SnapshotCompletedV1 {
            eligibility: RepositoryEligibilityPolicyV1::OrdinaryGitTrackedPlusUntrackedNonIgnoredV1,
            eligibility_basis_sha256: provenance_digest("d"),
            enumeration: CompleteEnumerationV1 {
                policy: RepositoryEligibilityPolicyV1::OrdinaryGitTrackedPlusUntrackedNonIgnoredV1,
                release_policy: RepositoryEligibilityReleasePolicyV1::GoalCupV1,
                eligibility_basis_sha256: provenance_digest("d"),
                included: 0,
                tracked_absent: 0,
                ignored: 0,
                release_excluded: 0,
                pruned_ignored_directories: 0,
            },
            copy_started_at: Rfc3339Utc::try_from("2026-09-10T00:00:01Z").unwrap(),
            copy_completed_at: Rfc3339Utc::try_from("2026-09-10T00:00:02Z").unwrap(),
            snapshot_manifest_sha256: provenance_digest("e"),
            integrity_verified_at: Rfc3339Utc::try_from("2026-09-10T00:00:03Z").unwrap(),
            object_count: 0,
            object_bytes: 0,
        },
        repository: Box::new(RepositoryFinalizationV1::ProjectionFailed {
            failure: goalcup_protocol::provenance::RepositoryProjectionFailureV1::ModeUnavailable,
            publisher: PublisherNotAttemptedV1 {
                reason: PublisherNotAttemptedReasonV1::ProjectionFailed,
            },
        }),
    }
}

#[test]
fn the_writer_produces_one_contiguous_chain_the_reader_can_recompute() {
    let tmp = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(tmp.path()).unwrap();
    assert_eq!(
        x.head(),
        None,
        "an empty chain has no head, it is not at seq 0"
    );

    let mut a = env(
        "main",
        "2026-09-04T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{\"type\":\"user\"}".to_string()),
        },
    );
    let mut b = env(
        "hooks",
        "2026-09-04T00:00:01Z",
        EventBody::Leg2Post {
            event: "UserPromptSubmit".to_string(),
            payload: serde_json::json!({"prompt": "go"}),
            path: None,
            receiver_meta: None,
        },
    );
    let la = x.append_chain(&mut a).unwrap();
    let lb = x.append_chain(&mut b).unwrap();

    assert_eq!((la.seq, lb.seq), (0, 1), "seq is contiguous from 0");
    assert_eq!(la.prev, GENESIS);
    assert_eq!(lb.prev, la.hash);
    assert_eq!(la.hash, link_hash(GENESIS, &a.chain_bytes().unwrap()));
    assert_eq!(lb.hash, link_hash(&la.hash, &b.chain_bytes().unwrap()));
    assert_eq!(x.head(), Some((1, lb.hash.clone())));

    // One Envelope per line, and every line recomputes.
    let text = std::fs::read_to_string(tmp.path().join(CHAIN_FILE)).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
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
    let mut y = TraceExport::resume(tmp.path()).unwrap();
    assert_eq!(
        y.head(),
        Some((1, head.clone())),
        "resume recovers the head from the file"
    );
    let mut c = env(
        "daemon",
        "2026-09-04T00:01:00Z",
        EventBody::Heartbeat {
            ts: "2026-09-04T00:01:00Z".to_string(),
            stores_seen: Vec::new(),
            receiver_ok: true,
        },
    );
    let lc = y.append_chain(&mut c).unwrap();
    assert_eq!(lc.seq, 2);
    assert_eq!(lc.prev, head);
}

#[test]
fn the_seal_is_the_last_link_before_it_is_a_file_and_there_is_only_ever_one() {
    let tmp = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(tmp.path()).unwrap();
    let mut a = env(
        "main",
        "2026-09-04T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{\"type\":\"user\"}".to_string()),
        },
    );
    x.append_chain(&mut a).unwrap();

    let m = manifest();
    let (path, link) = x.write_seal(&m).unwrap();
    assert_eq!(path, tmp.path().join(SEAL_FILE));
    assert_eq!(link.seq, 1, "the seal is a link, not only a file");
    assert_eq!(
        x.head(),
        Some((1, link.hash.clone())),
        "and it is the LAST link"
    );

    // The chained claim is the digest of the bytes on disk, so an ack over
    // seq 1 anchors the seal and V13 can recompute the claim from `seal.json`.
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
        x.write_seal(&m),
        Err(ExportError::SealAlreadyChained { seq: 1 })
    ));
    // And nothing else may follow the seal either: a record after it would leave an
    // acknowledgement over the seal anchoring a chain that kept growing.
    let mut after = env(
        "main",
        "2026-09-08T00:00:01Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    assert!(matches!(
        x.append_chain(&mut after),
        Err(ExportError::Sealed { seal_seq: 1 })
    ));
    // And the refused calls wrote nothing: the chain still ends at the seal.
    let text = std::fs::read_to_string(tmp.path().join(CHAIN_FILE)).unwrap();
    assert_eq!(text.lines().count(), 2);
}

/// `repair_seal` repairs the one terminal link this held export already validated; it must
/// not let a caller substitute a different, internally well-formed terminal and manifest.
#[test]
fn repair_seal_refuses_a_self_consistent_forged_terminal_without_mutating_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    let original = manifest();
    let (_, original_link) = export.write_seal(&original).unwrap();

    let mut forged = original.clone();
    forged.files.push(SealedFile {
        path: "forged.rs".to_string(),
        bytes: 1,
        sha256: "ff".repeat(32),
    });
    let mut forged_terminal = env(
        "daemon",
        &forged.sealed_at,
        EventBody::Seal {
            manifest_sha256: manifest_sha256(&forged).unwrap(),
            sealed_at: forged.sealed_at.clone(),
            file_count: forged.files.len() as u64,
        },
    );
    forged_terminal.link.seq = original_link.seq;
    forged_terminal.link.prev = original_link.prev.clone();
    forged_terminal.link.hash = forged_terminal
        .expected_hash(&forged_terminal.link.prev)
        .unwrap();
    assert_ne!(forged_terminal.link.hash, original_link.hash);
    assert_eq!(
        forged_terminal.link.hash,
        forged_terminal
            .expected_hash(&forged_terminal.link.prev)
            .unwrap(),
        "the forged terminal is internally self-consistent"
    );

    let stale_temp = tmp.path().join(format!("{DESCRIPTOR_FILE}.forged.tmp"));
    std::fs::write(&stale_temp, b"must remain untouched").unwrap();
    let before = regular_file_snapshot(tmp.path());

    assert!(
        export.repair_seal(&forged, &forged_terminal).is_err(),
        "a caller cannot replace seal.json with a terminal that is not the retained chain head"
    );
    assert_eq!(
        regular_file_snapshot(tmp.path()),
        before,
        "refusal leaves seal.json, chain.jsonl, acks.jsonl, and temporary files unchanged"
    );
}

#[test]
fn repair_seal_reconstructs_only_the_missing_fixed_document_from_the_exact_terminal() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    let original = manifest();
    export
        .write_seal_for_protocol(goalcup_protocol::LEGACY_PROTOCOL_VERSION, &original)
        .unwrap();
    let chain_before = std::fs::read(tmp.path().join(CHAIN_FILE)).unwrap();
    let terminal: Envelope = serde_json::from_slice(
        std::str::from_utf8(&chain_before)
            .unwrap()
            .lines()
            .last()
            .expect("the sealed chain has a terminal")
            .as_bytes(),
    )
    .unwrap();
    std::fs::remove_file(tmp.path().join(SEAL_FILE)).unwrap();
    drop(export);

    let resumed = TraceExport::resume(tmp.path()).unwrap();
    let stale_temp = tmp.path().join(format!("{DESCRIPTOR_FILE}.repair.tmp"));
    std::fs::write(&stale_temp, b"must remain untouched").unwrap();
    let acks_before = std::fs::read(tmp.path().join(ACKS_FILE)).unwrap();

    resumed.repair_seal(&original, &terminal).unwrap();

    assert_eq!(
        std::fs::read(tmp.path().join(SEAL_FILE)).unwrap(),
        serde_json::to_vec(&original).unwrap(),
        "the exact retained terminal repairs its matching fixed manifest"
    );
    assert_eq!(
        std::fs::read(tmp.path().join(CHAIN_FILE)).unwrap(),
        chain_before
    );
    assert_eq!(
        std::fs::read(tmp.path().join(ACKS_FILE)).unwrap(),
        acks_before
    );
    assert_eq!(
        std::fs::read(&stale_temp).unwrap(),
        b"must remain untouched"
    );
}

/// V13 recomputes the chained claim from the manifest it loaded, so the digest
/// has to be the digest of the bytes `write_seal` actually put on disk — not of some
/// other serialisation of the same value.
#[test]
fn the_chained_digest_is_the_digest_of_the_bytes_in_seal_json() {
    use sha2::{Digest, Sha256};
    let tmp = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(tmp.path()).unwrap();
    let mut m = manifest();
    m.files.push(SealedFile {
        path: "src/main.rs".to_string(),
        bytes: 12,
        sha256: "aa".repeat(32),
    });
    m.excluded.push(".git".to_string());
    let (path, _) = x.write_seal(&m).unwrap();

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
    let mut export = TraceExport::create(tmp.path()).unwrap();
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

    export.write_baseline(&unsorted).unwrap();
    export.write_seal(&unsorted).unwrap();

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

/// The caller hands the writer content, never a position. A stamped link that does not
/// name the seq the writer is about to write is a caller computing chain state of its
/// own, and is refused rather than silently overwritten.
#[test]
fn a_caller_cannot_hand_roll_a_link() {
    let tmp = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(tmp.path()).unwrap();
    let mut a = env(
        "main",
        "2026-09-04T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    x.append_chain(&mut a).unwrap();

    // A hand-rolled hash over a hand-rolled prev is discarded, not honoured.
    let mut b = env(
        "main",
        "2026-09-04T00:00:01Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    b.link = Link {
        seq: 1,
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
    let mut c = env(
        "main",
        "2026-09-04T00:00:02Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    c.link.seq = 9;
    assert!(matches!(
        x.append_chain(&mut c),
        Err(ExportError::OutOfOrder {
            found: 9,
            expected_after: Some(1)
        })
    ));
    assert_eq!(
        x.head().unwrap().0,
        1,
        "the refused append did not move the head"
    );
    assert_eq!(
        std::fs::read_to_string(tmp.path().join(CHAIN_FILE))
            .unwrap()
            .lines()
            .count(),
        2,
        "the refused append wrote no line"
    );
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

#[cfg(unix)]
#[test]
fn descriptor_write_stays_bound_to_the_acquired_root_after_path_exchange() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("trace");
    let detached = tmp.path().join("detached");
    let x = TraceExport::create(&root).unwrap();

    std::fs::rename(&root, &detached).unwrap();
    std::fs::create_dir(&root).unwrap();
    x.write_descriptor(&descriptor("e1")).unwrap();

    assert!(detached.join(DESCRIPTOR_FILE).is_file());
    assert!(!root.join(DESCRIPTOR_FILE).exists());
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
fn a_failed_chain_link_check_permanently_poisons_that_writer() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mut x = TraceExport::create(root).unwrap();
    let alias = root.join("chain-alias");
    std::fs::hard_link(root.join(CHAIN_FILE), &alias).unwrap();
    let mut first = env(
        "main",
        "2026-09-04T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );

    assert!(x.append_chain(&mut first).is_err());
    assert_eq!(x.head(), None);
    assert!(std::fs::read(root.join(CHAIN_FILE)).unwrap().is_empty());

    std::fs::remove_file(alias).unwrap();
    let mut second = env(
        "main",
        "2026-09-04T00:00:01Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    assert!(x.append_chain(&mut second).is_err());
    assert!(std::fs::read(root.join(CHAIN_FILE)).unwrap().is_empty());
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

    x.write_manifest(b"first manifest").unwrap();
    assert!(x.write_manifest(b"replacement manifest").is_err());
    assert_eq!(
        std::fs::read(tmp.path().join(MANIFEST_FILE)).unwrap(),
        b"first manifest"
    );
}

#[test]
fn preparing_and_dropping_a_valid_trace_changes_no_entry_or_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    export.write_receipt(&receipt()).unwrap();
    export.write_manifest(b"pinned manifest").unwrap();
    let (_, baseline) = export.write_baseline(&manifest()).unwrap();
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
fn finalizing_uses_the_prepared_root_and_chain_after_path_exchange() {
    let tmp = tempfile::tempdir().unwrap();
    let trace = tmp.path().join("trace");
    let detached = tmp.path().join("detached");
    let mut export = TraceExport::create(&trace).unwrap();
    let mut first = env(
        "main",
        "2026-09-04T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::json!({"record": 1}),
        },
    );
    let first = export.append_chain(&mut first).unwrap();
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
    let mut second = env(
        "main",
        "2026-09-04T00:00:01Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::json!({"record": 2}),
        },
    );
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

#[cfg(unix)]
#[test]
fn changed_length_refuses_finalization_before_mutation() {
    use std::io::Write as _;

    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    let mut record = env(
        "main",
        "2026-09-04T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::json!({"before": "prepare"}),
        },
    );
    export.append_chain(&mut record).unwrap();
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
    let mut x = TraceExport::create(&trace).unwrap();
    x.write_receipt(&receipt()).unwrap();
    let mut first = env(
        "main",
        "2026-09-04T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    x.append_chain(&mut first).unwrap();
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

    let mut second = env(
        "main",
        "2026-09-04T00:00:01Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    assert_eq!(resumed.append_chain(&mut second).unwrap().seq, 1);
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
    let mut x = TraceExport::create(tmp.path()).unwrap();
    let original = tmp.path().join("original-chain");
    let named = tmp.path().join(CHAIN_FILE);
    std::fs::rename(&named, &original).unwrap();
    std::fs::write(&named, b"").unwrap();
    assert!(x.independent_chain_reader().is_err());
    std::fs::remove_file(&named).unwrap();
    std::fs::rename(&original, &named).unwrap();

    let mut record = env(
        "main",
        "2026-09-04T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    assert!(x.append_chain(&mut record).is_err());
    assert!(std::fs::read(&named).unwrap().is_empty());
}

#[test]
fn the_descriptor_round_trips_and_names_the_six_files_the_writer_owns() {
    let tmp = tempfile::tempdir().unwrap();
    let x = TraceExport::create(tmp.path()).unwrap();
    let doc = TraceDescriptorDoc {
        connector: "claude-code-cli".to_string(),
        entry_id: "e1".to_string(),
        workspace: "/ws".to_string(),
        tracked_sessions: vec!["s1".to_string()],
        harness_version: "2.1.259".to_string(),
        connector_manifest: MANIFEST_FILE.to_string(),
        chain: Some(CHAIN_FILE.to_string()),
        receipt: Some(RECEIPT_FILE.to_string()),
        baseline: Some(BASELINE_FILE.to_string()),
        acks: Some(ACKS_FILE.to_string()),
        seal: Some(SEAL_FILE.to_string()),
        server_public_key: Some("ab".repeat(32)),
        expected_reread: false,
    };
    let path = x.write_descriptor(&doc).unwrap();
    assert_eq!(path, tmp.path().join(DESCRIPTOR_FILE));
    let back: TraceDescriptorDoc =
        toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(back, doc);
    // Relative, always: the directory must survive being copied elsewhere. Checked on
    // the fields parsed back out of the file, not by grepping the text — every value
    // that names a file *inside* the trace has to be a relative path a reader can join
    // onto the root. `workspace` is not one of them: it records where the run happened,
    // which is an absolute path by nature.
    let named: Vec<(&str, &String)> =
        std::iter::once(("connector_manifest", &back.connector_manifest))
            .chain(
                [
                    ("chain", &back.chain),
                    ("receipt", &back.receipt),
                    ("baseline", &back.baseline),
                    ("acks", &back.acks),
                    ("seal", &back.seal),
                ]
                .into_iter()
                .filter_map(|(name, v)| v.as_ref().map(|v| (name, v))),
            )
            .collect();
    assert_eq!(named.len(), 6, "this document names all six files");
    for (name, value) in named {
        assert!(!value.is_empty(), "{name} names nothing");
        assert!(!value.starts_with('/'), "{name} is absolute: {value}");
        assert!(
            !std::path::Path::new(value)
                .components()
                .any(|c| c == std::path::Component::ParentDir),
            "{name} climbs out of the trace root: {value}"
        );
    }
    let raw = std::fs::read_to_string(&path).unwrap();
    // The document always states which reading it asked for.
    assert!(raw.contains("expected_reread = false"));
    // An absent file is an absent key, not an empty string the reader would join onto
    // the root and then fail to open.
    let bare = TraceDescriptorDoc {
        chain: None,
        receipt: None,
        baseline: None,
        acks: None,
        seal: None,
        server_public_key: None,
        ..doc.clone()
    };
    let raw = toml::to_string(&bare).unwrap();
    assert!(!raw.contains("seal"), "{raw}");
    assert_eq!(toml::from_str::<TraceDescriptorDoc>(&raw).unwrap(), bare);
}

/// `write_descriptor` must write tmp-then-rename, the same way
/// `goalcup_daemon::status`'s own `write_atomic` does for the status cache, so a reader
/// that opens `trace.toml` mid-write sees the previous document whole rather than half of
/// the next one.
///
/// A racing reader is not something a fast unit test can force deterministically, so this
/// pins the *mechanism* instead of the race: a rename replaces a target by directory-entry
/// swap and never checks the target file's own mode bits, while an in-place write opens
/// the existing path and needs write permission ON THAT FILE. Chmod the first write's
/// `trace.toml` read-only, then write a second, different document — a tmp+rename writer
/// succeeds (and the file now holds the second document); a writer that truncates the
/// existing path in place is refused by the filesystem before a single byte changes.
#[cfg(unix)]
#[test]
fn write_descriptor_replaces_by_rename_not_by_truncating_the_existing_file() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let x = TraceExport::create(tmp.path()).unwrap();
    let first = TraceDescriptorDoc {
        connector: "claude-code-cli".to_string(),
        entry_id: "e1".to_string(),
        workspace: "/ws".to_string(),
        tracked_sessions: vec!["s1".to_string()],
        harness_version: "2.1.259".to_string(),
        connector_manifest: MANIFEST_FILE.to_string(),
        chain: Some(CHAIN_FILE.to_string()),
        receipt: None,
        baseline: None,
        acks: None,
        seal: None,
        server_public_key: None,
        expected_reread: false,
    };
    let path = x.write_descriptor(&first).unwrap();

    // Read-only: on a non-root test runner this refuses an open-for-write on the SAME
    // path, but not a rename onto it (rename is a directory-entry operation and checks
    // the directory's permissions, not the target file's mode).
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();

    let second = TraceDescriptorDoc {
        entry_id: "e2".to_string(),
        expected_reread: true,
        ..first.clone()
    };
    x.write_descriptor(&second).unwrap_or_else(|e| {
        panic!("a tmp+rename writer is not blocked by the target's own read-only mode: {e}")
    });

    // Restore write permission so the tempdir's own Drop cleanup can remove the file.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let back: TraceDescriptorDoc =
        toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        back, second,
        "the rename must have landed the second document"
    );
}

/// `write_file_atomic` carries the target's existing mode onto its
/// replacement. Otherwise the temp file gets whatever mode `File::create` gives
/// (subject to the process umask) and the rename puts THAT mode on the final path, so an
/// operator who had chmod'd `trace.toml` to 0600 would find it back at the default mode
/// the next time this writer rewrote it.
#[cfg(unix)]
#[test]
fn write_descriptor_carries_the_target_file_s_existing_mode_across_a_rewrite() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let x = TraceExport::create(tmp.path()).unwrap();
    let first = TraceDescriptorDoc {
        connector: "claude-code-cli".to_string(),
        entry_id: "e1".to_string(),
        workspace: "/ws".to_string(),
        tracked_sessions: vec!["s1".to_string()],
        harness_version: "2.1.259".to_string(),
        connector_manifest: MANIFEST_FILE.to_string(),
        chain: Some(CHAIN_FILE.to_string()),
        receipt: None,
        baseline: None,
        acks: None,
        seal: None,
        server_public_key: None,
        expected_reread: false,
    };
    let path = x.write_descriptor(&first).unwrap();

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

    let second = TraceDescriptorDoc {
        entry_id: "e2".to_string(),
        expected_reread: true,
        ..first.clone()
    };
    x.write_descriptor(&second).unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
    assert_eq!(
        mode, 0o600,
        "the rewrite must not have widened the mode an operator had tightened"
    );
    let back: TraceDescriptorDoc =
        toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(back, second, "the rewrite must still have landed");
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
fn the_writer_owns_every_path_the_descriptor_names() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("entries").join("e1");
    let mut x = TraceExport::create(&root).unwrap();
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
        "the receipt lands where the descriptor says"
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
    let (path, link) = x.write_baseline(&baseline).unwrap();
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

/// The once-only seal rule cannot live in one process's memory: a daemon that is killed
/// after sealing and restarted would otherwise chain a second seal, and an
/// acknowledgement over the entry would have two heads to anchor. `resume` reads the
/// chain it is continuing, so the seal it finds there binds the new instance too.
#[test]
fn a_resumed_export_will_not_chain_past_a_seal_it_found() {
    let tmp = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(tmp.path()).unwrap();
    let mut a = env(
        "main",
        "2026-09-04T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    x.append_chain(&mut a).unwrap();
    let (_, seal) = x.write_seal(&manifest()).unwrap();
    drop(x);

    let mut y = TraceExport::resume(tmp.path()).unwrap();
    assert_eq!(
        y.head(),
        Some((1, seal.hash.clone())),
        "the resumed writer picked the seal link up as its head"
    );

    let mut after = env(
        "main",
        "2026-09-08T00:00:01Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    assert!(matches!(
        y.append_chain(&mut after),
        Err(ExportError::Sealed { seal_seq: 1 })
    ));
    assert!(matches!(
        y.write_seal(&manifest()),
        Err(ExportError::SealAlreadyChained { seq: 1 })
    ));
    assert_eq!(
        std::fs::read_to_string(tmp.path().join(CHAIN_FILE))
            .unwrap()
            .lines()
            .count(),
        2,
        "neither refusal wrote a line"
    );
}

/// A daemon killed between `create` and its first record leaves an empty `chain.jsonl`.
/// Resuming that must start at 0 over `GENESIS` — starting at 1 would leave the chain
/// with no seq 0 and nothing to link the first record to.
#[test]
fn resume_on_an_empty_chain_starts_at_zero() {
    let tmp = tempfile::tempdir().unwrap();
    drop(TraceExport::create(tmp.path()).unwrap());

    let mut y = TraceExport::resume(tmp.path()).unwrap();
    assert_eq!(y.head(), None, "nothing is chained, so there is no head");
    let mut a = env(
        "main",
        "2026-09-04T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    let la = y.append_chain(&mut a).unwrap();
    assert_eq!(la.seq, 0, "the first link of a chain is 0, resumed or not");
    assert_eq!(la.prev, GENESIS);
    assert_eq!(y.head(), Some((0, la.hash.clone())));
}

#[test]
fn resume_continues_a_v1_head_with_a_current_inline_audit() {
    use goalcup_protocol::redaction::{RedactionAudit, RedactionAuditSpan};

    let tmp = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(tmp.path()).unwrap();
    let mut old = env(
        "main",
        "2026-09-04T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("legacy".to_string()),
        },
    );
    old.protocol_version = goalcup_protocol::LEGACY_PROTOCOL_VERSION;
    let old_link = x.append_chain(&mut old).unwrap();
    drop(x);

    let mut resumed = TraceExport::resume(tmp.path()).unwrap();
    let mut current = env(
        "main",
        "2026-09-04T00:00:01Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("<redacted:email>".to_string()),
        },
    );
    current.redaction = Some(RedactionAudit {
        spans: vec![RedactionAuditSpan {
            kind: "email".to_string(),
            start: 0,
            end: 16,
        }],
    });
    let current_link = resumed.append_chain(&mut current).unwrap();
    assert_eq!(current_link.seq, 1);
    assert_eq!(current_link.prev, old_link.hash);

    let text = std::fs::read_to_string(tmp.path().join(CHAIN_FILE)).unwrap();
    let lines: Vec<Envelope> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines[0].protocol_version, 1);
    assert_eq!(lines[0].redaction, None);
    assert_eq!(
        lines[1].protocol_version,
        goalcup_protocol::PROTOCOL_VERSION
    );
    assert!(lines[1].redaction.is_some());
}

#[test]
fn reserved_writers_keep_v2_default_and_can_reproduce_v1_bytes_explicitly() {
    let current = tempfile::tempdir().unwrap();
    let mut current_export = TraceExport::create(current.path()).unwrap();
    current_export.write_baseline(&manifest()).unwrap();
    current_export.write_seal(&manifest()).unwrap();
    drop(current_export);

    let legacy = tempfile::tempdir().unwrap();
    let mut legacy_export = TraceExport::create(legacy.path()).unwrap();
    legacy_export
        .write_baseline_for_protocol(goalcup_protocol::LEGACY_PROTOCOL_VERSION, &manifest())
        .unwrap();
    legacy_export
        .write_seal_for_protocol(goalcup_protocol::LEGACY_PROTOCOL_VERSION, &manifest())
        .unwrap();
    drop(legacy_export);

    for (root, expected_version) in [
        (current.path(), goalcup_protocol::PROTOCOL_VERSION),
        (legacy.path(), goalcup_protocol::LEGACY_PROTOCOL_VERSION),
    ] {
        let raw = std::fs::read_to_string(root.join(CHAIN_FILE)).unwrap();
        assert!(
            !raw.contains("\"redaction\""),
            "an absent audit stays omitted from {expected_version} bytes"
        );
        let envelopes: Vec<Envelope> = raw
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(envelopes.len(), 2);
        let mut previous = GENESIS.to_string();
        for envelope in envelopes {
            assert_eq!(envelope.protocol_version, expected_version);
            assert_eq!(envelope.redaction, None);
            assert_eq!(envelope.link.prev, previous);
            assert_eq!(
                envelope.link.hash,
                envelope.expected_hash(&previous).unwrap()
            );
            previous = envelope.link.hash;
        }
    }

    assert_eq!(
        std::fs::read(current.path().join(BASELINE_FILE)).unwrap(),
        std::fs::read(legacy.path().join(BASELINE_FILE)).unwrap()
    );
    assert_eq!(
        std::fs::read(current.path().join(SEAL_FILE)).unwrap(),
        std::fs::read(legacy.path().join(SEAL_FILE)).unwrap()
    );
}

#[test]
fn explicit_legacy_reserved_writers_preserve_cutoff_and_terminal_refusals() {
    let tmp = tempfile::tempdir().unwrap();
    let chain_path = tmp.path().join(CHAIN_FILE);
    let mut export = TraceExport::create(tmp.path()).unwrap();
    export
        .write_cutoff("e1", "2026-09-10T00:00:00Z", cutoff("a"))
        .unwrap();

    let cutoff_head = export.head();
    let cutoff_bytes = std::fs::read(&chain_path).unwrap();
    assert!(matches!(
        export.write_baseline_for_protocol(goalcup_protocol::LEGACY_PROTOCOL_VERSION, &manifest()),
        Err(ExportError::AfterCutoff { cutoff_seq: 0 })
    ));
    assert!(matches!(
        export.write_seal_for_protocol(goalcup_protocol::LEGACY_PROTOCOL_VERSION, &manifest()),
        Err(ExportError::AfterCutoff { cutoff_seq: 0 })
    ));
    assert_eq!(export.head(), cutoff_head);
    assert_eq!(std::fs::read(&chain_path).unwrap(), cutoff_bytes);

    export
        .write_finalization("e1", "2026-09-10T00:00:01Z", unavailable_finalization("a"))
        .unwrap();
    let terminal_head = export.head();
    let terminal_bytes = std::fs::read(&chain_path).unwrap();
    assert!(matches!(
        export.write_baseline_for_protocol(goalcup_protocol::LEGACY_PROTOCOL_VERSION, &manifest()),
        Err(ExportError::Terminal { terminal_seq: 1 })
    ));
    assert!(matches!(
        export.write_seal_for_protocol(goalcup_protocol::LEGACY_PROTOCOL_VERSION, &manifest()),
        Err(ExportError::Terminal { terminal_seq: 1 })
    ));
    assert_eq!(export.head(), terminal_head);
    assert_eq!(std::fs::read(chain_path).unwrap(), terminal_bytes);
}

#[test]
fn resume_refuses_an_unterminated_tail_without_changing_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(tmp.path()).unwrap();
    let mut first = env(
        "main",
        "2026-09-04T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    x.append_chain(&mut first).unwrap();
    drop(x);
    let path = tmp.path().join(CHAIN_FILE);
    let mut before = std::fs::read(&path).unwrap();
    before.extend_from_slice(b"{\"protocol_version\":2");
    std::fs::write(&path, &before).unwrap();

    assert!(matches!(
        TraceExport::resume(tmp.path()),
        Err(ExportError::ChainCorrupt { seq: 1, .. })
    ));
    assert_eq!(std::fs::read(path).unwrap(), before);
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

/// `create` opens `chain.jsonl` for append. Called against a directory that already
/// holds a chain it would write a second run's links onto the end of the first run's,
/// producing one file that reads as one chain and is two. Resuming is the caller's job.
#[test]
fn create_refuses_a_directory_that_already_holds_a_chain() {
    let tmp = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(tmp.path()).unwrap();
    let mut a = env(
        "main",
        "2026-09-04T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    x.append_chain(&mut a).unwrap();
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
    let mut x = TraceExport::create(tmp.path()).unwrap();
    let m = manifest();
    let (_, first) = x.write_seal(&m).unwrap();
    drop(x);

    // Hand-written, because the writer under test refuses to produce this: a *valid*
    // second seal link, contiguous and correctly hashed, so the only thing wrong with
    // the file is that it seals the entry twice.
    let mut second = env(
        "daemon",
        &m.sealed_at,
        EventBody::Seal {
            manifest_sha256: manifest_sha256(&m).unwrap(),
            sealed_at: m.sealed_at.clone(),
            file_count: 0,
        },
    );
    second.link.seq = 1;
    second.link.prev = first.hash.clone();
    second.link.hash = link_hash(&first.hash, &second.chain_bytes().unwrap());
    append_line(&tmp.path().join(CHAIN_FILE), &second);

    match TraceExport::resume(tmp.path()) {
        Err(ExportError::ChainCorrupt { seq, reason }) => {
            assert_eq!(seq, 1);
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
    let mut x = TraceExport::create(garbage.path()).unwrap();
    let mut a = env(
        "main",
        "2026-09-04T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    x.append_chain(&mut a).unwrap();
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
    let mut x = TraceExport::create(gap.path()).unwrap();
    let mut a = env(
        "main",
        "2026-09-04T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    let la = x.append_chain(&mut a).unwrap();
    drop(x);
    let mut skipped = env(
        "main",
        "2026-09-04T00:00:01Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    skipped.link.seq = 7;
    skipped.link.prev = la.hash.clone();
    skipped.link.hash = link_hash(&la.hash, &skipped.chain_bytes().unwrap());
    append_line(&gap.path().join(CHAIN_FILE), &skipped);
    assert!(matches!(
        TraceExport::resume(gap.path()),
        Err(ExportError::ChainCorrupt { seq: 1, .. })
    ));
}

/// Append one already-stamped envelope straight to the file, bypassing the writer.
fn append_line(path: &std::path::Path, env: &Envelope) {
    let mut line = serde_json::to_vec(env).unwrap();
    line.push(b'\n');
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    std::io::Write::write_all(&mut f, &line).unwrap();
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
    let mut x = TraceExport::create(tmp.path()).unwrap();

    let baseline = SealManifest {
        sealed_at: "2026-09-04T00:00:00Z".to_string(),
        files: vec![SealedFile {
            path: "README.md".to_string(),
            bytes: 12,
            sha256: "aa".repeat(32),
        }],
        ..manifest()
    };
    let (path, link) = x.write_baseline(&baseline).unwrap();
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
    let mut a = env(
        "main",
        "2026-09-04T00:00:10Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    let after = x.append_chain(&mut a).unwrap();
    assert_eq!(after.seq, 1);
    assert_eq!(after.prev, link.hash);
}

/// A baseline taken after the run started is not a baseline: it describes a workspace the
/// agent has already been writing to. The writer refuses it rather than chaining a link
/// that would read as a start.
#[test]
fn a_baseline_that_is_not_the_first_link_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(tmp.path()).unwrap();
    let mut a = env(
        "main",
        "2026-09-04T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    x.append_chain(&mut a).unwrap();

    assert!(matches!(
        x.write_baseline(&manifest()),
        Err(ExportError::BaselineNotFirst { head_seq: 0 })
    ));
    // The refusal wrote nothing: no file, and no second link.
    assert!(!tmp.path().join(BASELINE_FILE).exists());
    assert_eq!(
        std::fs::read_to_string(tmp.path().join(CHAIN_FILE))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert_eq!(x.head().unwrap().0, 0);
}

/// Two baselines would be two answers to "what did the workspace hold at t0", and a
/// reviewer would have to pick. There is one, and it is seq 0.
#[test]
fn there_is_only_ever_one_baseline() {
    let tmp = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(tmp.path()).unwrap();
    x.write_baseline(&manifest()).unwrap();
    assert!(matches!(
        x.write_baseline(&manifest()),
        Err(ExportError::BaselineAlreadyChained)
    ));
    assert_eq!(
        std::fs::read_to_string(tmp.path().join(CHAIN_FILE))
            .unwrap()
            .lines()
            .count(),
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
    let mut x = TraceExport::create(tmp.path()).unwrap();
    let (_, baseline) = x.write_baseline(&manifest()).unwrap();
    let mut a = env(
        "main",
        "2026-09-04T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    x.append_chain(&mut a).unwrap();
    drop(x);

    let mut y = TraceExport::resume(tmp.path()).unwrap();
    assert!(matches!(
        y.write_baseline(&manifest()),
        Err(ExportError::BaselineAlreadyChained)
    ));
    // And the resumed writer is otherwise the same writer: it continues at seq 2 over the
    // head it read, and the baseline it found is still the link at seq 0.
    let mut b = env(
        "main",
        "2026-09-04T00:00:01Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    assert_eq!(y.append_chain(&mut b).unwrap().seq, 2);
    let text = std::fs::read_to_string(tmp.path().join(CHAIN_FILE)).unwrap();
    let first: Envelope = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert!(matches!(first.body, EventBody::Baseline { .. }));
    assert_eq!(first.link.hash, baseline.hash);
}

/// The connector's manifest is the one file a verifier reads whose bytes this crate does
/// not author, and the verifier compares them byte for byte against the digest a receipt claims.
/// It goes into the trace directory through the export like every other file there, so
/// `write_file` stays this crate's single answer to "how does a file get into a trace"
/// and the daemon keeps no second copy of the name.
#[test]
fn the_connector_manifest_goes_into_the_trace_through_the_export() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("entries").join("e1");
    let x = TraceExport::create(&root).unwrap();

    // Bytes, not text, and not re-serialised TOML: what the digest pin covers is the shipped file,
    // whatever is in it.
    let bytes: &[u8] = b"id = \"claude-code-cli\"\n# \xf0\x9f\x94\x92 trailing junk\n\n";
    let path = x.write_manifest(bytes).unwrap();
    assert_eq!(path, root.join(MANIFEST_FILE));
    assert!(
        path.starts_with(&root),
        "the manifest lands under the trace root: {}",
        path.display()
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "byte for byte: the digest pin compares these bytes (R-D)"
    );

    // And the descriptor's manifest field names THIS file. One constant on both sides,
    // so a trace the export wrote never points at a name the export does not use.
    let doc = TraceDescriptorDoc {
        connector: "claude-code-cli".to_string(),
        entry_id: "e1".to_string(),
        workspace: "/ws".to_string(),
        tracked_sessions: vec!["s1".to_string()],
        harness_version: "2.1.259".to_string(),
        connector_manifest: MANIFEST_FILE.to_string(),
        chain: Some(CHAIN_FILE.to_string()),
        receipt: None,
        baseline: None,
        acks: Some(ACKS_FILE.to_string()),
        seal: None,
        server_public_key: None,
        expected_reread: false,
    };
    let descriptor = x.write_descriptor(&doc).unwrap();
    let back: TraceDescriptorDoc =
        toml::from_str(&std::fs::read_to_string(&descriptor).unwrap()).unwrap();
    assert_eq!(
        std::fs::read(root.join(&back.connector_manifest)).unwrap(),
        bytes,
        "the descriptor names the manifest the export wrote"
    );
}

/// `EventBody::Seal` and `EventBody::Baseline` are `write_seal`'s and `write_baseline`'s
/// to build, and `append_chain` will not take one: the position and once-only
/// rules are enforced by those two methods, and a body chained around them would carry
/// none of them. A second seal link gives an acknowledgement two heads to anchor;
/// a hand-chained baseline is a "what the workspace held at t0" claim taken at any
/// seq at all.
///
/// The guard belongs here rather than in one caller, so that it travels with the
/// `TraceExport` to every holder.
#[test]
fn append_chain_refuses_every_body_the_export_reserves() {
    let tmp = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(tmp.path()).unwrap();

    let mut seal = env(
        "daemon",
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
        "a hand-assembled seal is write_seal's to build"
    );

    let mut cutoff = env(
        "daemon",
        "2026-09-10T00:00:00Z",
        EventBody::CutoffReachedV1(cutoff("a")),
    );
    assert!(matches!(
        x.append_chain(&mut cutoff),
        Err(ExportError::ReservedBody { kind: "cutoff" })
    ));

    let mut finalization = env(
        "daemon",
        "2026-09-10T00:00:01Z",
        EventBody::FinalizationV1(unavailable_finalization("a")),
    );
    assert!(matches!(
        x.append_chain(&mut finalization),
        Err(ExportError::ReservedBody {
            kind: "finalization"
        })
    ));

    let mut baseline = env(
        "daemon",
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
        "a hand-assembled baseline is write_baseline's to build"
    );

    // Neither refusal wrote a line or moved the head, so the baseline the export builds
    // itself is still the chain's first link — and the seal still closes it.
    assert_eq!(x.head(), None);
    assert_eq!(
        std::fs::read_to_string(tmp.path().join(CHAIN_FILE)).unwrap(),
        "",
        "a refused append leaves no link behind"
    );
    assert_eq!(x.write_baseline(&manifest()).unwrap().1.seq, 0);
    assert_eq!(x.write_seal(&manifest()).unwrap().1.seq, 1);
    assert_eq!(
        std::fs::read_to_string(tmp.path().join(CHAIN_FILE))
            .unwrap()
            .lines()
            .count(),
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
    let mut x = TraceExport::create(tmp.path()).unwrap();
    let mut last = String::new();
    for i in 0..LINKS {
        let mut e = env(
            "main",
            "2026-09-04T00:00:00Z",
            EventBody::Leg1Record {
                store: "main".to_string(),
                payload: serde_json::Value::String(format!("{{\"n\":{i}}}")),
            },
        );
        last = x.append_chain(&mut e).unwrap().hash;
    }
    drop(x);

    let mut y = TraceExport::resume(tmp.path()).unwrap();
    assert_eq!(
        y.head(),
        Some((LINKS - 1, last.clone())),
        "the resumed head is the last link of the long chain"
    );
    let mut next = env(
        "main",
        "2026-09-04T00:00:01Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    let link = y.append_chain(&mut next).unwrap();
    assert_eq!(link.seq, LINKS, "the chain continues at the next seq");
    assert_eq!(link.prev, last, "over the link the scan stopped on");
}

#[test]
fn resume_refuses_a_complete_final_envelope_without_its_newline() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    let mut record = env(
        "main",
        "2026-09-04T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
    );
    export.append_chain(&mut record).unwrap();
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

#[test]
fn cutoff_allows_only_one_bound_finalization_and_then_closes_the_chain() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    let cutoff_link = export
        .write_cutoff("e1", "2026-09-10T00:00:00Z", cutoff("a"))
        .unwrap();
    assert_eq!(cutoff_link.seq, 0);

    let before_refusal = std::fs::read(tmp.path().join(CHAIN_FILE)).unwrap();
    let head_before_refusal = export.head();
    let mut ordinary = env(
        "main",
        "2026-09-10T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::json!({"after": "cutoff"}),
        },
    );
    assert!(matches!(
        export.append_chain(&mut ordinary),
        Err(ExportError::AfterCutoff { cutoff_seq: 0 })
    ));
    assert_eq!(export.head(), head_before_refusal);
    assert_eq!(
        std::fs::read(tmp.path().join(CHAIN_FILE)).unwrap(),
        before_refusal
    );
    assert!(matches!(
        export.write_cutoff("e1", "2026-09-10T00:00:01Z", cutoff("a")),
        Err(ExportError::CutoffAlreadyChained { seq: 0 })
    ));
    assert!(matches!(
        export.write_baseline(&manifest()),
        Err(ExportError::AfterCutoff { cutoff_seq: 0 })
    ));
    assert!(matches!(
        export.write_seal(&manifest()),
        Err(ExportError::AfterCutoff { cutoff_seq: 0 })
    ));

    let final_link = export
        .write_finalization("e1", "2026-09-10T00:00:01Z", unavailable_finalization("a"))
        .unwrap();
    assert_eq!(final_link.seq, 1);
    let closed = std::fs::read(tmp.path().join(CHAIN_FILE)).unwrap();
    assert!(matches!(
        export.write_finalization("e1", "2026-09-10T00:00:02Z", unavailable_finalization("a")),
        Err(ExportError::FinalizationAlreadyChained { seq: 1 })
    ));
    assert_eq!(std::fs::read(tmp.path().join(CHAIN_FILE)).unwrap(), closed);

    export
        .append_ack(&AckCheckpoint {
            entry_id: "e1".to_string(),
            seq: final_link.seq,
            hash: final_link.hash,
            acked_at: "2026-09-10T00:00:02Z".to_string(),
            server_signature: "ab".repeat(64),
        })
        .unwrap();
}

#[test]
fn captured_finalization_is_also_terminal() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    export
        .write_cutoff("e1", "2026-09-10T00:00:00Z", cutoff("a"))
        .unwrap();
    let final_link = export
        .write_finalization("e1", "2026-09-10T00:00:04Z", captured_finalization("a"))
        .unwrap();
    assert_eq!(final_link.seq, 1);
    assert!(matches!(
        export.write_cutoff("e1", "2026-09-10T00:00:05Z", cutoff("a")),
        Err(ExportError::Terminal { terminal_seq: 1 })
    ));
    drop(export);

    let mut resumed = TraceExport::resume(tmp.path()).unwrap();
    let mut ordinary = env(
        "main",
        "2026-09-10T00:00:06Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::json!({}),
        },
    );
    assert!(matches!(
        resumed.append_chain(&mut ordinary),
        Err(ExportError::Terminal { terminal_seq: 1 })
    ));
}

#[test]
fn finalization_binding_mismatch_and_pre_cutoff_call_refuse_without_writing() {
    let before = tempfile::tempdir().unwrap();
    let mut open = TraceExport::create(before.path()).unwrap();
    assert!(matches!(
        open.write_finalization("e1", "2026-09-10T00:00:01Z", unavailable_finalization("a")),
        Err(ExportError::FinalizationBeforeCutoff)
    ));
    assert_eq!(
        std::fs::read(before.path().join(CHAIN_FILE)).unwrap(),
        Vec::<u8>::new()
    );

    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    export
        .write_cutoff("e1", "2026-09-10T00:00:00Z", cutoff("a"))
        .unwrap();
    let before = std::fs::read(tmp.path().join(CHAIN_FILE)).unwrap();
    let head = export.head();

    assert!(matches!(
        export.write_finalization("e2", "2026-09-10T00:00:01Z", unavailable_finalization("a")),
        Err(ExportError::FinalizationBindingMismatch)
    ));
    assert!(matches!(
        export.write_finalization("e1", "2026-09-10T00:00:01Z", unavailable_finalization("b")),
        Err(ExportError::FinalizationBindingMismatch)
    ));
    assert_eq!(export.head(), head);
    assert_eq!(std::fs::read(tmp.path().join(CHAIN_FILE)).unwrap(), before);
}

#[test]
fn cutoff_and_terminal_phases_are_reconstructed_on_resume() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    export
        .write_cutoff("e1", "2026-09-10T00:00:00Z", cutoff("a"))
        .unwrap();
    drop(export);

    let mut resumed = TraceExport::resume(tmp.path()).unwrap();
    let final_link = resumed
        .write_finalization("e1", "2026-09-10T00:00:01Z", unavailable_finalization("a"))
        .unwrap();
    drop(resumed);

    let mut terminal = TraceExport::resume(tmp.path()).unwrap();
    assert_eq!(terminal.head().unwrap().0, final_link.seq);
    let mut ordinary = env(
        "main",
        "2026-09-10T00:00:02Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::json!({}),
        },
    );
    assert!(matches!(
        terminal.append_chain(&mut ordinary),
        Err(ExportError::Terminal { terminal_seq: 1 })
    ));
}

#[test]
fn historical_v3_cutoff_and_finalization_resume_without_relabelling() {
    let tmp = tempfile::tempdir().unwrap();
    drop(TraceExport::create(tmp.path()).unwrap());

    let mut cutoff_envelope = env(
        "daemon",
        "2026-09-10T00:00:00Z",
        EventBody::CutoffReachedV1(cutoff("a")),
    );
    cutoff_envelope.protocol_version = 3;
    cutoff_envelope.link.prev = GENESIS.to_string();
    cutoff_envelope.link.hash = cutoff_envelope.expected_hash(GENESIS).unwrap();

    let mut finalization_envelope = env(
        "daemon",
        "2026-09-10T00:00:01Z",
        EventBody::FinalizationV1(unavailable_finalization("a")),
    );
    finalization_envelope.protocol_version = 3;
    finalization_envelope.link.seq = 1;
    finalization_envelope.link.prev = cutoff_envelope.link.hash.clone();
    finalization_envelope.link.hash = finalization_envelope
        .expected_hash(&cutoff_envelope.link.hash)
        .unwrap();

    let chain = format!(
        "{}\n{}\n",
        serde_json::to_string(&cutoff_envelope).unwrap(),
        serde_json::to_string(&finalization_envelope).unwrap()
    );
    std::fs::write(tmp.path().join(CHAIN_FILE), chain.as_bytes()).unwrap();

    let resumed = TraceExport::resume(tmp.path()).unwrap();
    assert_eq!(
        resumed.head(),
        Some((1, finalization_envelope.link.hash.clone()))
    );
    drop(resumed);
    assert_eq!(
        std::fs::read(tmp.path().join(CHAIN_FILE)).unwrap(),
        chain.as_bytes(),
        "resuming V3 evidence never rewrites its bytes"
    );
}

#[test]
fn v4_workspace_finalization_round_trips_and_remains_terminal_after_resume() {
    let (tmp, export, cutoff_link, manifest) = cutoff_export();
    assert_eq!(cutoff_link.seq, 0);
    write_workspace_submission_layout(tmp.path(), &manifest);
    assert!(tmp.path().join(SUBMISSION_MANIFEST_FILE).is_file());
    assert!(tmp.path().join(SUBMISSION_SHA256_OBJECTS_DIR).is_dir());
    drop(export);

    let mut resumed = TraceExport::resume(tmp.path()).unwrap();
    let final_link = resumed
        .write_finalization(
            "e1",
            "2026-09-10T00:00:04Z",
            workspace_finalization_with_digest("a", manifest.digest().unwrap()),
        )
        .unwrap();
    assert_eq!(final_link.seq, 1);
    drop(resumed);

    let terminal = TraceExport::resume(tmp.path()).unwrap();
    assert_eq!(terminal.head(), Some((1, final_link.hash)));
    let lines = std::fs::read_to_string(tmp.path().join(CHAIN_FILE)).unwrap();
    let envelopes = lines
        .lines()
        .map(|line| serde_json::from_str::<Envelope>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(envelopes.len(), 2);
    assert!(
        envelopes
            .iter()
            .all(|envelope| envelope.protocol_version == 4)
    );
    assert!(matches!(
        envelopes[1].body,
        EventBody::FinalizationV1(FinalizationV1::WorkspaceSubmission {
            submission: SubmissionOutcomeV1::Captured(_),
            ..
        })
    ));
}

#[test]
fn captured_workspace_submission_requires_the_fixed_manifest() {
    let (tmp, mut export, _cutoff, manifest) = cutoff_export();
    assert_captured_finalization_refused_without_write(
        &tmp,
        &mut export,
        workspace_finalization_with_digest("a", manifest.digest().unwrap()),
    );
}

#[test]
fn captured_workspace_submission_requires_the_claimed_manifest_digest() {
    let (tmp, mut export, _cutoff, manifest) = cutoff_export();
    write_workspace_submission_layout(tmp.path(), &manifest);
    assert_captured_finalization_refused_without_write(
        &tmp,
        &mut export,
        workspace_finalization_with_digest("a", provenance_digest("f")),
    );
}

#[test]
fn captured_workspace_submission_manifest_entry_must_match_the_cutoff() {
    let (tmp, mut export, _cutoff, mut manifest) = cutoff_export();
    manifest.entry_id = "e2".to_string();
    write_workspace_submission_layout(tmp.path(), &manifest);
    assert_captured_finalization_refused_without_write(
        &tmp,
        &mut export,
        workspace_finalization_with_digest("a", manifest.digest().unwrap()),
    );
}

#[test]
fn captured_workspace_submission_manifest_receipt_must_match_the_cutoff() {
    let (tmp, mut export, _cutoff, mut manifest) = cutoff_export();
    manifest.receipt_sha256 = provenance_digest("b");
    write_workspace_submission_layout(tmp.path(), &manifest);
    assert_captured_finalization_refused_without_write(
        &tmp,
        &mut export,
        workspace_finalization_with_digest("a", manifest.digest().unwrap()),
    );
}

#[test]
fn captured_workspace_submission_manifest_sequence_must_match_the_cutoff() {
    let (tmp, mut export, cutoff, mut manifest) = cutoff_export();
    manifest.cutoff_seq = cutoff.seq + 1;
    write_workspace_submission_layout(tmp.path(), &manifest);
    assert_captured_finalization_refused_without_write(
        &tmp,
        &mut export,
        workspace_finalization_with_digest("a", manifest.digest().unwrap()),
    );
}

#[test]
fn captured_workspace_submission_manifest_hash_must_match_the_cutoff() {
    let (tmp, mut export, _cutoff, mut manifest) = cutoff_export();
    manifest.cutoff_chain_hash = provenance_digest("c");
    write_workspace_submission_layout(tmp.path(), &manifest);
    assert_captured_finalization_refused_without_write(
        &tmp,
        &mut export,
        workspace_finalization_with_digest("a", manifest.digest().unwrap()),
    );
}

#[test]
fn incomplete_workspace_submission_is_writable_without_an_artifact() {
    let finalization = FinalizationV1::WorkspaceSubmission {
        receipt_sha256: provenance_digest("a"),
        termination: TerminationOutcomeV1::Unresolved {
            requested_at: Rfc3339Utc::try_from("2026-09-10T00:00:01Z").unwrap(),
            stopped_waiting_at: Rfc3339Utc::try_from("2026-09-10T00:00:02Z").unwrap(),
            reason: goalcup_protocol::provenance::TerminationFailureV1::ProcessStillRunning,
        },
        submission: SubmissionOutcomeV1::Incomplete {
            reason: SubmissionFailureV1::RecoveryAmbiguous,
        },
    };
    assert_eq!(finalization.validate(), Ok(()));

    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    export
        .write_cutoff("e1", "2026-09-10T00:00:00Z", cutoff("a"))
        .unwrap();
    let final_link = export
        .write_finalization("e1", "2026-09-10T00:00:03Z", finalization)
        .unwrap();
    assert_eq!(final_link.seq, 1);
    assert!(!tmp.path().join(SUBMISSION_DIR).exists());
}

#[test]
fn resume_rejects_a_correctly_linked_finalization_with_the_wrong_receipt_binding() {
    use std::io::Write as _;

    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    let cutoff_link = export
        .write_cutoff("e1", "2026-09-10T00:00:00Z", cutoff("a"))
        .unwrap();
    drop(export);

    let mut forged = env(
        "daemon",
        "2026-09-10T00:00:01Z",
        EventBody::FinalizationV1(unavailable_finalization("b")),
    );
    forged.link.seq = 1;
    forged.link.prev = cutoff_link.hash.clone();
    forged.link.hash = forged.expected_hash(&cutoff_link.hash).unwrap();
    let mut line = serde_json::to_vec(&forged).unwrap();
    line.push(b'\n');
    std::fs::OpenOptions::new()
        .append(true)
        .open(tmp.path().join(CHAIN_FILE))
        .unwrap()
        .write_all(&line)
        .unwrap();

    let before = std::fs::read(tmp.path().join(CHAIN_FILE)).unwrap();
    assert!(matches!(
        TraceExport::resume(tmp.path()),
        Err(ExportError::ChainCorrupt { seq: 1, .. })
    ));
    assert_eq!(std::fs::read(tmp.path().join(CHAIN_FILE)).unwrap(), before);
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
fn resume_rejects_every_correctly_linked_illegal_new_phase_transition() {
    let before_cutoff = tempfile::tempdir().unwrap();
    let export = TraceExport::create(before_cutoff.path()).unwrap();
    drop(export);
    append_correctly_linked_test_envelope(
        before_cutoff.path(),
        None,
        env(
            "daemon",
            "2026-09-10T00:00:00Z",
            EventBody::FinalizationV1(unavailable_finalization("a")),
        ),
    );
    assert_resume_refuses_without_rewriting(before_cutoff.path(), 0);

    for illegal_tail in [
        env(
            "main",
            "2026-09-10T00:00:01Z",
            EventBody::Leg1Record {
                store: "main".to_string(),
                payload: serde_json::json!({"after": "cutoff"}),
            },
        ),
        env(
            "daemon",
            "2026-09-10T00:00:01Z",
            EventBody::CutoffReachedV1(cutoff("a")),
        ),
        env(
            "daemon",
            "2026-09-10T00:00:01Z",
            EventBody::Baseline {
                manifest_sha256: "00".repeat(32),
                file_count: 0,
            },
        ),
        env(
            "daemon",
            "2026-09-10T00:00:01Z",
            EventBody::Seal {
                manifest_sha256: "00".repeat(32),
                sealed_at: "2026-09-10T00:00:01Z".to_string(),
                file_count: 0,
            },
        ),
    ] {
        let after_cutoff = tempfile::tempdir().unwrap();
        let mut export = TraceExport::create(after_cutoff.path()).unwrap();
        let cutoff_link = export
            .write_cutoff("e1", "2026-09-10T00:00:00Z", cutoff("a"))
            .unwrap();
        drop(export);
        append_correctly_linked_test_envelope(
            after_cutoff.path(),
            Some(&cutoff_link),
            illegal_tail,
        );
        assert_resume_refuses_without_rewriting(after_cutoff.path(), 1);
    }

    for illegal_tail in [
        env(
            "main",
            "2026-09-10T00:00:02Z",
            EventBody::Leg2Post {
                event: "after-finalization".to_string(),
                payload: serde_json::json!({"after": "finalization"}),
                path: None,
                receiver_meta: None,
            },
        ),
        env(
            "daemon",
            "2026-09-10T00:00:02Z",
            EventBody::FinalizationV1(unavailable_finalization("a")),
        ),
    ] {
        let after_terminal = tempfile::tempdir().unwrap();
        let mut export = TraceExport::create(after_terminal.path()).unwrap();
        export
            .write_cutoff("e1", "2026-09-10T00:00:00Z", cutoff("a"))
            .unwrap();
        let finalization_link = export
            .write_finalization("e1", "2026-09-10T00:00:01Z", unavailable_finalization("a"))
            .unwrap();
        drop(export);
        append_correctly_linked_test_envelope(
            after_terminal.path(),
            Some(&finalization_link),
            illegal_tail,
        );
        assert_resume_refuses_without_rewriting(after_terminal.path(), 2);
    }
}

#[test]
fn envelope_version_body_matrix_preserves_historical_v1_and_rejects_undefined_shapes() {
    let envelope = |protocol_version, body, redaction| Envelope {
        protocol_version,
        entry_id: "e1".to_string(),
        source: "daemon".to_string(),
        recorded_at: "2026-09-10T00:00:00Z".to_string(),
        link: Link {
            seq: 0,
            prev: String::new(),
            hash: String::new(),
        },
        body,
        redaction,
    };

    // V1 `HomeSeeded` was already emitted before handoff markers existed. Its absence of
    // both markers is the historical wire shape, not an incompatibility inferred from the
    // current enum position.
    assert_eq!(
        envelope(
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            EventBody::HomeSeeded(home_seed(None, None)),
            None,
        )
        .validate_version_body_compatibility(),
        Ok(())
    );
    assert_eq!(
        envelope(
            goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
            EventBody::HomeSeeded(home_seed(None, None)),
            None,
        )
        .validate_version_body_compatibility(),
        Ok(())
    );

    for (version, seed) in [
        (
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            home_seed(Some(7), None),
        ),
        (
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            home_seed(None, Some("ab")),
        ),
        (
            goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
            home_seed(Some(7), None),
        ),
        (
            goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
            home_seed(None, Some("ab")),
        ),
    ] {
        assert_eq!(
            envelope(version, EventBody::HomeSeeded(seed), None)
                .validate_version_body_compatibility(),
            Err(EnvelopeVersionCompatibilityError::HomeSeededRequiresV3 {
                protocol_version: version,
            }),
            "v{version} cannot name v3 HomeSeeded handoff markers"
        );
    }
    for seed in [
        home_seed(Some(7), None),
        home_seed(None, Some("ab")),
        home_seed(Some(7), Some("ab")),
    ] {
        assert_eq!(
            envelope(
                goalcup_protocol::PROTOCOL_VERSION,
                EventBody::HomeSeeded(seed),
                None,
            )
            .validate_version_body_compatibility(),
            Ok(())
        );
    }

    // These four bodies were introduced by V2. V1 can deserialize their Rust shapes in
    // this build, but an evidence line that labels one V1 must stop at the typed boundary.
    for (name, body, expected) in [
        (
            "repository_observed_v1",
            EventBody::RepositoryObservedV1(repository_observed()),
            V2OnlyEventBody::RepositoryObservedV1,
        ),
        (
            "cutoff_reached_v1",
            EventBody::CutoffReachedV1(cutoff("a")),
            V2OnlyEventBody::CutoffReachedV1,
        ),
        (
            "finalization_v1",
            EventBody::FinalizationV1(unavailable_finalization("a")),
            V2OnlyEventBody::FinalizationV1,
        ),
        (
            "foreign_scan_coverage",
            foreign_scan_coverage(),
            V2OnlyEventBody::ForeignScanCoverage,
        ),
    ] {
        assert_eq!(
            envelope(goalcup_protocol::LEGACY_PROTOCOL_VERSION, body, None)
                .validate_version_body_compatibility(),
            Err(EnvelopeVersionCompatibilityError::BodyRequiresV2 { body: expected }),
            "V1 must reject V2-only {name}",
        );
    }
    for body in [
        EventBody::RepositoryObservedV1(repository_observed()),
        EventBody::CutoffReachedV1(cutoff("a")),
        EventBody::FinalizationV1(unavailable_finalization("a")),
        foreign_scan_coverage(),
    ] {
        assert_eq!(
            envelope(goalcup_protocol::PREVIOUS_PROTOCOL_VERSION, body, None)
                .validate_version_body_compatibility(),
            Ok(())
        );
    }

    assert_eq!(
        envelope(
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            EventBody::Leg1Record {
                store: "main".to_string(),
                payload: serde_json::json!({"redacted": true}),
            },
            Some(inline_redaction()),
        )
        .validate_version_body_compatibility(),
        Err(EnvelopeVersionCompatibilityError::InlineRedactionRequiresV2),
        "V1 has no inline redaction field"
    );
    for version in [
        goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
        goalcup_protocol::PROTOCOL_VERSION,
    ] {
        assert_eq!(
            envelope(version, legacy_redaction(), None).validate_version_body_compatibility(),
            Err(
                EnvelopeVersionCompatibilityError::LegacyRedactionRequiresV1 {
                    protocol_version: version,
                }
            ),
            "v{version} cannot use V1's standalone redaction record",
        );
    }
}

#[test]
fn writers_refuse_unknown_versions_before_they_hash_or_write_any_evidence() {
    let ordinary = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(ordinary.path()).unwrap();
    let before = regular_file_snapshot(ordinary.path());
    let mut unknown = env(
        "main",
        "2026-09-10T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::json!({"v": 5}),
        },
    );
    unknown.protocol_version = 5;
    assert!(matches!(
        export.append_chain(&mut unknown),
        Err(ExportError::UnsupportedProtocolVersion {
            protocol_version: 5,
            ..
        })
    ));
    assert_eq!(regular_file_snapshot(ordinary.path()), before);

    let baseline = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(baseline.path()).unwrap();
    let before = regular_file_snapshot(baseline.path());
    assert!(matches!(
        export.write_baseline_for_protocol(5, &manifest()),
        Err(ExportError::UnsupportedProtocolVersion {
            protocol_version: 5,
            ..
        })
    ));
    assert_eq!(regular_file_snapshot(baseline.path()), before);

    let seal = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(seal.path()).unwrap();
    let before = regular_file_snapshot(seal.path());
    assert!(matches!(
        export.write_seal_for_protocol(5, &manifest()),
        Err(ExportError::UnsupportedProtocolVersion {
            protocol_version: 5,
            ..
        })
    ));
    assert_eq!(regular_file_snapshot(seal.path()), before);
}

#[test]
fn writers_apply_the_complete_version_body_matrix_before_writing_or_hashing() {
    let reject = |version, body, redaction, expected, name: &str| {
        let root = tempfile::tempdir().unwrap();
        let mut export = TraceExport::create(root.path()).unwrap();
        let before = regular_file_snapshot(root.path());
        let mut incompatible = Envelope {
            protocol_version: version,
            entry_id: "e1".to_string(),
            source: "daemon".to_string(),
            recorded_at: "2026-09-10T00:00:00Z".to_string(),
            link: Link {
                seq: 0,
                prev: String::new(),
                hash: String::new(),
            },
            body,
            redaction,
        };
        match export.append_chain(&mut incompatible) {
            Err(ExportError::IncompatibleEnvelope { reason }) => assert_eq!(reason, expected),
            other => panic!("{name} is rejected as version/body incompatibility, got {other:?}"),
        }
        assert_eq!(
            regular_file_snapshot(root.path()),
            before,
            "{name} never reaches a chain write or hash"
        );
    };

    for (name, version, body, redaction, expected) in version_body_incompatibilities() {
        reject(version, body, redaction, expected, name);
    }

    let root = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(root.path()).unwrap();
    let mut historical_v1 = Envelope {
        protocol_version: goalcup_protocol::LEGACY_PROTOCOL_VERSION,
        entry_id: "e1".to_string(),
        source: "daemon".to_string(),
        recorded_at: "2026-09-10T00:00:00Z".to_string(),
        link: Link {
            seq: 0,
            prev: String::new(),
            hash: String::new(),
        },
        body: EventBody::HomeSeeded(home_seed(None, None)),
        redaction: None,
    };
    assert_eq!(export.append_chain(&mut historical_v1).unwrap().seq, 0);
}

/// The legacy layout carries `keys_kept` and an on-wire `SeededFile.sha256`; the current
/// layout renamed the key list and carries no digest. Both forms are supported historical
/// evidence, but a reader must retain the incoming form when it recomputes this link:
/// serialising it as the other shape changes the preimage.
#[test]
fn resume_preserves_the_53d3019_home_seeded_wire_preimage_and_head() {
    let historical: Envelope = serde_json::from_str(HISTORICAL_V1_HOME_SEEDED).unwrap();
    assert_eq!(
        historical.protocol_version,
        goalcup_protocol::LEGACY_PROTOCOL_VERSION
    );
    assert_eq!(historical.link.hash, HISTORICAL_V1_HOME_SEEDED_HASH);
    assert_eq!(
        historical.chain_bytes().unwrap(),
        HISTORICAL_V1_HOME_SEEDED_CHAIN_BYTES,
        "the parsed historical line retains its own 53d3019 wire preimage"
    );
    assert_eq!(
        historical.expected_hash(GENESIS).unwrap(),
        HISTORICAL_V1_HOME_SEEDED_HASH,
        "the precomputed historical hash covers that original wire layout"
    );

    let root = tempfile::tempdir().unwrap();
    drop(TraceExport::create(root.path()).unwrap());
    let chain = root.path().join(CHAIN_FILE);
    std::fs::write(&chain, HISTORICAL_V1_HOME_SEEDED).unwrap();
    let before = std::fs::read(&chain).unwrap();

    let resumed = TraceExport::resume(root.path()).unwrap();
    assert_eq!(
        resumed.head(),
        Some((0, HISTORICAL_V1_HOME_SEEDED_HASH.to_string()))
    );
    drop(resumed);
    assert_eq!(
        std::fs::read(&chain).unwrap(),
        before,
        "resuming validates without rewriting historical chain bytes"
    );
}

/// The old layout is a parser-retained preimage, not an evidence shape that any current
/// writer may create.  Its only truthful protocol label is the V1 label it arrived with.
#[test]
fn legacy_home_seeded_layout_is_v1_only_and_cannot_be_appended() {
    for protocol_version in [
        goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
        goalcup_protocol::PROTOCOL_VERSION,
    ] {
        let mut legacy: Envelope = serde_json::from_str(HISTORICAL_V1_HOME_SEEDED).unwrap();
        legacy.protocol_version = protocol_version;
        let expected =
            EnvelopeVersionCompatibilityError::LegacyHomeSeededRequiresV1 { protocol_version };
        assert_eq!(
            legacy.validate_version_body_compatibility(),
            Err(expected.clone()),
            "V{protocol_version} cannot relabel a parsed legacy HomeSeeded preimage"
        );

        let root = tempfile::tempdir().unwrap();
        let mut export = TraceExport::create(root.path()).unwrap();
        let before = regular_file_snapshot(root.path());
        match export.append_chain(&mut legacy) {
            Err(ExportError::IncompatibleEnvelope { reason }) => assert_eq!(reason, expected),
            other => {
                panic!("V{protocol_version} cannot append a legacy HomeSeeded layout: {other:?}")
            }
        }
        assert_eq!(
            regular_file_snapshot(root.path()),
            before,
            "the incompatible legacy layout never reaches an append"
        );
    }

    let mut v1_legacy: Envelope = serde_json::from_str(HISTORICAL_V1_HOME_SEEDED).unwrap();
    assert_eq!(
        v1_legacy.validate_version_body_compatibility(),
        Ok(()),
        "a V1 reader can verify the parsed historical layout"
    );
    let root = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(root.path()).unwrap();
    let before = regular_file_snapshot(root.path());
    assert!(matches!(
        export.append_chain(&mut v1_legacy),
        Err(ExportError::LegacyHomeSeededAppend)
    ));
    assert_eq!(
        regular_file_snapshot(root.path()),
        before,
        "the parser-only legacy layout never reaches an append"
    );
}

/// A compatibility reader must stop at a malformed body before normalising it and then
/// blaming a changed hash.  Each mutation targets a nested shape whose prior permissive
/// deserializer could silently discard evidence bytes.
#[test]
fn resume_refuses_unknown_or_ambiguous_home_seeded_members_at_the_parse_boundary() {
    let digest = "a".repeat(64);
    let cases = [
        (
            "unknown home-seed member",
            HISTORICAL_V1_HOME_SEEDED.replacen(
                "\"connector\":\"fixture\"",
                "\"unexpected\":true,\"connector\":\"fixture\"",
                1,
            ),
        ),
        (
            "unknown seeded key member",
            HISTORICAL_V1_HOME_SEEDED.replacen(
                "\"name\":\"theme\",\"value\":\"dark\"",
                "\"name\":\"theme\",\"value\":\"dark\",\"unexpected\":true",
                1,
            ),
        ),
        (
            "unknown seeded file member",
            HISTORICAL_V1_HOME_SEEDED.replacen(
                "\"sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"",
                "\"sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"unexpected\":true",
                1,
            ),
        ),
        (
            "null legacy key list mixed into a later layout",
            HISTORICAL_V1_HOME_SEEDED.replacen(
                &format!(
                    "\"keys_kept\":[\"theme\"],\"files\":[{{\"name\":\"auth.json\",\"bytes\":3,\"sha256\":\"{digest}\"}}]"
                ),
                "\"keys_kept\":null,\"keys_unchanged\":[\"theme\"],\"written\":[],\"files\":[{\"name\":\"auth.json\",\"bytes\":3}]",
                1,
            ),
        ),
    ];

    for (name, chain_text) in cases {
        let root = tempfile::tempdir().unwrap();
        drop(TraceExport::create(root.path()).unwrap());
        let chain = root.path().join(CHAIN_FILE);
        std::fs::write(&chain, chain_text).unwrap();
        let before = regular_file_snapshot(root.path());

        match TraceExport::resume(root.path()) {
            Err(ExportError::ChainCorrupt { seq: 0, reason }) => assert!(
                reason.contains("not an envelope"),
                "{name} is rejected while parsing, not after its fields were discarded: {reason}"
            ),
            other => panic!("{name} must stop at the parsing boundary, got {other:?}"),
        }
        assert_eq!(regular_file_snapshot(root.path()), before, "{name}");
    }
}

/// A line that spells an omitted field as `null` must not pass merely because its attacker
/// reused the hash of the canonical null-free record.  Refuse before the chain parser can
/// reserialize the body and make the different source bytes disappear.
#[test]
fn resume_refuses_null_normalization_of_every_optional_home_seed_field() {
    for (name, chain_text) in null_normalized_home_seed_lines() {
        let root = tempfile::tempdir().unwrap();
        drop(TraceExport::create(root.path()).unwrap());
        let chain = root.path().join(CHAIN_FILE);
        std::fs::write(&chain, chain_text).unwrap();
        let before = regular_file_snapshot(root.path());

        match TraceExport::resume(root.path()) {
            Err(ExportError::ChainCorrupt { seq: 0, reason }) => assert!(
                reason.contains("not an envelope"),
                "{name} is rejected while parsing, not after null was normalized away: {reason}"
            ),
            other => panic!("{name} must stop at the parsing boundary, got {other:?}"),
        }
        assert_eq!(regular_file_snapshot(root.path()), before, "{name}");
    }
}

#[test]
fn resume_rejects_unsupported_or_falsely_labelled_envelopes_without_mutation() {
    let historical = tempfile::tempdir().unwrap();
    let mut historical_export = TraceExport::create(historical.path()).unwrap();
    let mut historical_seed = Envelope {
        protocol_version: goalcup_protocol::LEGACY_PROTOCOL_VERSION,
        entry_id: "e1".to_string(),
        source: "daemon".to_string(),
        recorded_at: "2026-09-10T00:00:00Z".to_string(),
        link: Link {
            seq: 0,
            prev: String::new(),
            hash: String::new(),
        },
        body: EventBody::HomeSeeded(home_seed(None, None)),
        redaction: None,
    };
    historical_export
        .append_chain(&mut historical_seed)
        .unwrap();
    drop(historical_export);
    assert_eq!(
        TraceExport::resume(historical.path())
            .unwrap()
            .head()
            .unwrap()
            .0,
        0
    );

    let unsupported = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(unsupported.path()).unwrap();
    let mut first = env(
        "main",
        "2026-09-10T00:00:00Z",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::json!({"prefix": true}),
        },
    );
    let first_link = export.append_chain(&mut first).unwrap();
    drop(export);
    let mut future = first.clone();
    future.protocol_version = 5;
    future.link.seq = 1;
    future.link.prev = first_link.hash;
    // Deliberately stale: resume must identify V5 from its shallow header before treating
    // the old V4 hash as a chain-corruption claim.
    future.link.hash = first.link.hash;
    {
        use std::io::Write as _;

        let mut line = serde_json::to_vec(&future).unwrap();
        line.push(b'\n');
        std::fs::OpenOptions::new()
            .append(true)
            .open(unsupported.path().join(CHAIN_FILE))
            .unwrap()
            .write_all(&line)
            .unwrap();
    }
    let stale_descriptor_temp = unsupported
        .path()
        .join(format!("{DESCRIPTOR_FILE}.999999.777.tmp"));
    std::fs::write(
        &stale_descriptor_temp,
        b"must not be swept before V5 is refused",
    )
    .unwrap();
    let before = regular_file_snapshot(unsupported.path());
    assert!(matches!(
        TraceExport::resume(unsupported.path()),
        Err(ExportError::UnsupportedProtocolVersion {
            protocol_version: 5,
            ..
        })
    ));
    assert_eq!(regular_file_snapshot(unsupported.path()), before);

    for (name, version, body, redaction, expected) in version_body_incompatibilities() {
        let root = tempfile::tempdir().unwrap();
        drop(TraceExport::create(root.path()).unwrap());
        append_correctly_linked_test_envelope(
            root.path(),
            None,
            Envelope {
                protocol_version: version,
                entry_id: "e1".to_string(),
                source: "daemon".to_string(),
                recorded_at: "2026-09-10T00:00:00Z".to_string(),
                link: Link {
                    seq: 0,
                    prev: String::new(),
                    hash: String::new(),
                },
                body,
                redaction,
            },
        );
        let before = regular_file_snapshot(root.path());
        match TraceExport::resume(root.path()) {
            Err(ExportError::IncompatibleEnvelope { reason }) => assert_eq!(reason, expected),
            other => panic!("{name} does not resume through a false version/body label: {other:?}"),
        }
        assert_eq!(regular_file_snapshot(root.path()), before, "{name}");
    }
}
