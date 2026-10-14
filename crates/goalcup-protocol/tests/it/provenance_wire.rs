//! The provenance documents a v5 chain can carry: their primitives, and the one signed-mode
//! finalization arm the frozen corpus pins.

use ed25519_dalek::SigningKey;
use goalcup_protocol::provenance::*;

fn digest(byte: char) -> Sha256Digest {
    Sha256Digest::try_from(byte.to_string().repeat(64)).unwrap()
}

fn oid(byte: char) -> GitObjectId {
    GitObjectId::try_from(byte.to_string().repeat(40)).unwrap()
}

fn root(value: &str) -> PortableRelativePathOrRoot {
    PortableRelativePathOrRoot::try_from(value).unwrap()
}

fn at(value: &str) -> Rfc3339Utc {
    Rfc3339Utc::try_from(value).unwrap()
}

fn remote() -> NormalizedGitHubRemote {
    NormalizedGitHubRemote {
        host: "github.com".to_string(),
        owner: "agentcommunity".to_string(),
        repository: "goal-cup".to_string(),
        transport: GitRemoteTransportV1::Https,
    }
}

fn selected() -> SelectedRepositoryV1 {
    SelectedRepositoryV1 {
        workspace_relative_root: root(""),
        remote: remote(),
        target_ref: ValidatedHeadsRef::try_from("refs/heads/main").unwrap(),
        object_format: GitObjectFormat::Sha1,
        pre_seal_head: PreSealHeadV1::Existing {
            commit: oid('a'),
            tree: oid('b'),
        },
        selection_basis_chain_hash: digest('c'),
    }
}

#[test]
fn validated_scalars_refuse_noncanonical_and_cross_format_values() {
    assert_eq!(digest('a').as_str(), "a".repeat(64));
    assert!(Sha256Digest::try_from("A".repeat(64)).is_err());
    assert!(Sha256Digest::try_from("a".repeat(63)).is_err());

    assert!(oid('a').validate_for(GitObjectFormat::Sha1).is_ok());
    assert!(oid('a').validate_for(GitObjectFormat::Sha256).is_err());
    assert!(GitObjectId::try_from("g".repeat(40)).is_err());

    assert!(PortableRelativePathOrRoot::try_from("a/b").is_ok());
    assert!(PortableRelativePathOrRoot::try_from("").is_ok());
    for invalid in [".", "../a", "a//b", "/a", "C:/a", "a\\b"] {
        assert!(
            PortableRelativePathOrRoot::try_from(invalid).is_err(),
            "{invalid}"
        );
    }

    assert!(ValidatedHeadsRef::try_from("refs/heads/main").is_ok());
    for invalid in [
        "main",
        "refs/heads/",
        "refs/heads/a..b",
        "refs/heads/a.lock",
        "refs/heads/a@{b",
    ] {
        assert!(ValidatedHeadsRef::try_from(invalid).is_err(), "{invalid}");
    }

    assert!(remote().validate().is_ok());
    let mut bad_remote = remote();
    bad_remote.host = "https://github.com".to_string();
    assert!(bad_remote.validate().is_err());
    let mut bad_remote = remote();
    bad_remote.owner = "AgentCommunity".to_string();
    assert!(bad_remote.validate().is_err());
    let mut bad_remote = remote();
    bad_remote.host = "git.-example.com".to_string();
    assert!(bad_remote.validate().is_err());

    assert!(Rfc3339Utc::try_from("2024-02-29T23:59:59.123Z").is_ok());
    for invalid in [
        "2025-02-29T00:00:00Z",
        "2026-09-10T24:00:00Z",
        "2026-09-10T00:00:00+00:00",
        "2026-09-10T00:00:00.Z",
    ] {
        assert!(Rfc3339Utc::try_from(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn shared_new_binding_grammars_are_bounded_and_canonical() {
    for valid in ["v1", "2026.09_v-1", "A"] {
        validate_questionnaire_version(valid).unwrap();
    }
    for invalid in ["", "-v1", "v/1", "é", &"a".repeat(129)] {
        assert!(
            validate_questionnaire_version(invalid).is_err(),
            "{invalid}"
        );
    }

    let key = SigningKey::from_bytes(&[21; 32]);
    let lowercase = hex::encode(key.verifying_key().to_bytes());
    assert_eq!(
        canonical_run_public_key(&lowercase.to_ascii_uppercase()).unwrap(),
        lowercase
    );
    assert!(canonical_run_public_key(&"a".repeat(63)).is_err());
    assert!(canonical_run_public_key(&"zz".repeat(32)).is_err());
    assert!(
        canonical_run_public_key(
            "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f"
        )
        .is_err()
    );
    let weak_identity = format!("01{}", "00".repeat(31));
    assert!(canonical_run_public_key(&weak_identity).is_err());
}

#[test]
fn finalization_validation_rejects_a_noncausal_publisher_reason() {
    let unavailable = FinalizationV1::CaptureUnavailable {
        receipt_sha256: digest('9'),
        termination: TerminationOutcomeV1::AlreadyStopped {
            checked_at: at("2026-09-10T00:00:00Z"),
        },
        capture: CaptureUnavailableV1::NotAttempted(SnapshotNotAttemptedV1 {
            selection: RepositoryNonSelectionV1::None {
                reason: NoRepositoryReasonV1::NotFound,
            },
        }),
        publisher: PublisherNotAttemptedV1 {
            reason: PublisherNotAttemptedReasonV1::SelectionFailed,
        },
    };
    unavailable.validate().unwrap();

    let mut wrong = unavailable;
    let FinalizationV1::CaptureUnavailable { publisher, .. } = &mut wrong;
    publisher.reason = PublisherNotAttemptedReasonV1::SnapshotFailed;
    assert!(wrong.validate().is_err());
}

#[test]
fn every_finalization_failure_stage_has_a_causal_closed_not_attempted_branch() {
    let selection_failures = [
        RepositoryNonSelectionV1::None {
            reason: NoRepositoryReasonV1::NotFound,
        },
        RepositoryNonSelectionV1::Ambiguous { candidate_count: 2 },
        RepositoryNonSelectionV1::Unsupported {
            reason: RepositoryUnsupportedReasonV1::EligibleOutputOutsideSelectedRepository,
        },
    ];
    for selection in selection_failures {
        FinalizationV1::CaptureUnavailable {
            receipt_sha256: digest('9'),
            termination: TerminationOutcomeV1::AlreadyStopped {
                checked_at: at("2026-09-10T00:00:00Z"),
            },
            capture: CaptureUnavailableV1::NotAttempted(SnapshotNotAttemptedV1 { selection }),
            publisher: PublisherNotAttemptedV1 {
                reason: PublisherNotAttemptedReasonV1::SelectionFailed,
            },
        }
        .validate()
        .unwrap();
    }

    for phase in [
        SnapshotPhaseV1::Eligibility,
        SnapshotPhaseV1::Enumerate,
        SnapshotPhaseV1::Copy,
        SnapshotPhaseV1::Protect,
        SnapshotPhaseV1::Verify,
    ] {
        let expected = if phase == SnapshotPhaseV1::Eligibility {
            PublisherNotAttemptedReasonV1::EligibilityFailed
        } else {
            PublisherNotAttemptedReasonV1::SnapshotFailed
        };
        FinalizationV1::CaptureUnavailable {
            receipt_sha256: digest('9'),
            termination: TerminationOutcomeV1::AlreadyStopped {
                checked_at: at("2026-09-10T00:00:00Z"),
            },
            capture: CaptureUnavailableV1::Failed {
                selected: Box::new(selected()),
                failure: SnapshotFailedV1 {
                    phase,
                    reason: SnapshotFailureReasonV1::RaceDetected,
                },
            },
            publisher: PublisherNotAttemptedV1 { reason: expected },
        }
        .validate()
        .unwrap();
    }
}

#[test]
fn a_finalization_rejects_unknown_fields() {
    let finalization = FinalizationV1::CaptureUnavailable {
        receipt_sha256: digest('9'),
        termination: TerminationOutcomeV1::AlreadyStopped {
            checked_at: at("2026-09-10T00:00:00Z"),
        },
        capture: CaptureUnavailableV1::Failed {
            selected: Box::new(selected()),
            failure: SnapshotFailedV1 {
                phase: SnapshotPhaseV1::Copy,
                reason: SnapshotFailureReasonV1::Io,
            },
        },
        publisher: PublisherNotAttemptedV1 {
            reason: PublisherNotAttemptedReasonV1::SnapshotFailed,
        },
    };
    let value = serde_json::to_value(&finalization).unwrap();
    assert_eq!(
        serde_json::from_value::<FinalizationV1>(value.clone()).unwrap(),
        finalization
    );
    let mut outer = value.clone();
    outer["capture_unavailable"]
        .as_object_mut()
        .unwrap()
        .insert("unexpected".to_string(), serde_json::json!(true));
    assert!(serde_json::from_value::<FinalizationV1>(outer).is_err());
    let mut inner = value;
    inner["capture_unavailable"]["capture"]["failed"]["selected"]
        .as_object_mut()
        .unwrap()
        .insert("unexpected".to_string(), serde_json::json!(true));
    assert!(serde_json::from_value::<FinalizationV1>(inner).is_err());
}
