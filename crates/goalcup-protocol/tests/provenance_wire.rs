use ed25519_dalek::SigningKey;
use goalcup_protocol::provenance::*;

fn digest(byte: char) -> Sha256Digest {
    Sha256Digest::try_from(byte.to_string().repeat(64)).unwrap()
}

fn oid(byte: char) -> GitObjectId {
    GitObjectId::try_from(byte.to_string().repeat(40)).unwrap()
}

fn path(value: &str) -> PortableRelativePath {
    PortableRelativePath::try_from(value).unwrap()
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

fn snapshot_manifest() -> PinnedEligibleRepositorySnapshotManifestV1 {
    PinnedEligibleRepositorySnapshotManifestV1 {
        format: PinnedEligibleRepositorySnapshotFormatV1::PinnedEligibleRepositorySnapshotV1,
        entries: vec![
            SnapshotEntryV1 {
                repository_path: path("README.md"),
                observed_kind: CapturedEntryKindV1::Regular {
                    posix_owner_executable: Some(false),
                },
                bytes: 7,
                content_sha256: digest('d'),
            },
            SnapshotEntryV1 {
                repository_path: path("src/main.rs"),
                observed_kind: CapturedEntryKindV1::Regular {
                    posix_owner_executable: Some(true),
                },
                bytes: 11,
                content_sha256: digest('e'),
            },
        ],
    }
}

fn snapshot_completed(
    manifest: &PinnedEligibleRepositorySnapshotManifestV1,
) -> SnapshotCompletedV1 {
    SnapshotCompletedV1 {
        eligibility: RepositoryEligibilityPolicyV1::OrdinaryGitTrackedPlusUntrackedNonIgnoredV1,
        eligibility_basis_sha256: digest('f'),
        enumeration: CompleteEnumerationV1 {
            policy: RepositoryEligibilityPolicyV1::OrdinaryGitTrackedPlusUntrackedNonIgnoredV1,
            release_policy: RepositoryEligibilityReleasePolicyV1::GoalCupV1,
            eligibility_basis_sha256: digest('f'),
            included: 2,
            tracked_absent: 0,
            ignored: 3,
            release_excluded: 1,
            pruned_ignored_directories: 1,
        },
        copy_started_at: at("2026-09-10T00:00:01Z"),
        copy_completed_at: at("2026-09-10T00:00:02.5Z"),
        snapshot_manifest_sha256: manifest.digest().unwrap(),
        integrity_verified_at: at("2026-09-10T00:00:03Z"),
        object_count: 2,
        object_bytes: 18,
    }
}

fn history_descriptor() -> RequiredGitObjectHistoryV1 {
    RequiredGitObjectHistoryV1 {
        root_commit: Some(oid('a')),
        manifest_sha256: digest('8'),
        object_count: 1,
        uncompressed_object_bytes: 5,
    }
}

fn captured_with(repository: RepositoryFinalizationV1) -> FinalizationV1 {
    let manifest = snapshot_manifest();
    FinalizationV1::Captured {
        receipt_sha256: digest('9'),
        termination: TerminationOutcomeV1::AlreadyStopped {
            checked_at: at("2026-09-10T00:00:00Z"),
        },
        selected: Box::new(selected()),
        snapshot: snapshot_completed(&manifest),
        repository: Box::new(repository),
    }
}

fn created_with(
    selected: SelectedRepositoryV1,
    disposition: CommitDispositionV1,
) -> FinalizationV1 {
    let manifest = snapshot_manifest();
    let (root_commit, object_count, uncompressed_object_bytes) = match &selected.pre_seal_head {
        PreSealHeadV1::Unborn => (None, 0, 0),
        PreSealHeadV1::Existing { commit, .. } => (Some(commit.clone()), 1, 5),
    };
    FinalizationV1::Captured {
        receipt_sha256: digest('9'),
        termination: TerminationOutcomeV1::AlreadyStopped {
            checked_at: at("2026-09-10T00:00:00Z"),
        },
        selected: Box::new(selected),
        snapshot: snapshot_completed(&manifest),
        repository: Box::new(RepositoryFinalizationV1::ProjectionComplete {
            repository_manifest_sha256: digest('7'),
            commit: Box::new(CommitConstructionOutcomeV1::Resolved {
                required_history: RequiredGitObjectHistoryV1 {
                    root_commit,
                    manifest_sha256: digest('8'),
                    object_count,
                    uncompressed_object_bytes,
                },
                disposition,
                publication: Box::new(InitialPublicationOutcomeV1::NotAttempted(
                    PublisherNotAttemptedV1 {
                        reason: PublisherNotAttemptedReasonV1::CredentialUnavailable,
                    },
                )),
            }),
        }),
    }
}

fn resolved_with(publication: InitialPublicationOutcomeV1) -> FinalizationV1 {
    captured_with(RepositoryFinalizationV1::ProjectionComplete {
        repository_manifest_sha256: digest('7'),
        commit: Box::new(CommitConstructionOutcomeV1::Resolved {
            required_history: history_descriptor(),
            disposition: CommitDispositionV1::ReusedHead {
                commit: oid('a'),
                tree: oid('b'),
            },
            publication: Box::new(publication),
        }),
    })
}

#[test]
fn validated_scalars_refuse_noncanonical_and_cross_format_values() {
    assert_eq!(digest('a').as_str(), "a".repeat(64));
    assert!(Sha256Digest::try_from("A".repeat(64)).is_err());
    assert!(Sha256Digest::try_from("a".repeat(63)).is_err());

    assert!(oid('a').validate_for(GitObjectFormat::Sha1).is_ok());
    assert!(oid('a').validate_for(GitObjectFormat::Sha256).is_err());
    assert!(GitObjectId::try_from("g".repeat(40)).is_err());

    assert!(PortableRelativePath::try_from("a/b").is_ok());
    for invalid in ["", ".", "../a", "a//b", "/a", "C:/a", "a\\b"] {
        assert!(
            PortableRelativePath::try_from(invalid).is_err(),
            "{invalid}"
        );
    }
    assert!(PortableRelativePathOrRoot::try_from("").is_ok());
    assert!(PortableRelativePathOrRoot::try_from(".").is_err());

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
fn versioned_bounds_and_domains_are_fixed_and_distinct() {
    assert_eq!(MAX_ACTIVE_GIT_CONFIG_SCOPES_V1, 4);
    assert_eq!(MAX_GIT_CONFIG_BYTES_V1, 1_048_576);
    assert_eq!(MAX_GIT_INDEX_BYTES_V1, 512 * 1024 * 1024);
    assert_eq!(MAX_GIT_INDEX_ENTRIES_V1, 2_000_000);
    assert_eq!(MAX_GITIGNORE_FILES_V1, 16_384);
    assert_eq!(MAX_GITIGNORE_BYTES_V1, 1_048_576);
    assert_eq!(MAX_IGNORE_BYTES_AGGREGATE_V1, 64 * 1024 * 1024);
    assert_eq!(MAX_IGNORE_RULES_V1, 1_000_000);
    assert_eq!(MAX_INFO_EXCLUDE_BYTES_V1, 16 * 1024 * 1024);
    assert_eq!(MAX_GLOBAL_EXCLUDES_BYTES_V1, 16 * 1024 * 1024);
    assert_eq!(MAX_REQUIRED_HISTORY_OBJECTS_V1, 1_000_000);
    assert_eq!(MAX_REQUIRED_HISTORY_BYTES_V1, 16 * 1024 * 1024 * 1024);
    assert_eq!(MAX_REQUIRED_HISTORY_OBJECT_BYTES_V1, 2 * 1024 * 1024 * 1024);
    assert_eq!(MAX_REQUIRED_HISTORY_PARENT_EDGES_V1, 1_000_000);

    let domains = [
        SNAPSHOT_MANIFEST_DOMAIN,
        REPOSITORY_MANIFEST_DOMAIN,
        REQUIRED_GIT_OBJECT_HISTORY_DOMAIN,
        RECOVERY_SIGNATURE_DOMAIN,
        RECOVERY_RECORD_DIGEST_DOMAIN,
        PUBLICATION_OBSERVATION_SIGNATURE_DOMAIN,
        goalcup_protocol::receipt::START_RECEIPT_DIGEST_DOMAIN,
    ];
    for (index, domain) in domains.iter().enumerate() {
        assert!(domain.ends_with(b"\0"));
        for other in &domains[index + 1..] {
            assert_ne!(domain, other);
            assert!(!domain.starts_with(other) && !other.starts_with(domain));
        }
    }
    assert_eq!(
        REQUIRED_GIT_OBJECT_HISTORY_DOMAIN,
        b"goalcup:required-git-object-history:v1\0"
    );
}

#[test]
fn snapshot_and_projection_manifests_are_canonical_and_exactly_equal() {
    let snapshot = snapshot_manifest();
    let bytes = snapshot.canonical_bytes().unwrap();
    let expected_snapshot = format!(
        concat!(
            r#"{{"format":"pinned_eligible_repository_snapshot_v1","entries":["#,
            r#"{{"repository_path":"README.md","observed_kind":{{"regular":{{"posix_owner_executable":false}}}},"bytes":7,"content_sha256":"{}"}},"#,
            r#"{{"repository_path":"src/main.rs","observed_kind":{{"regular":{{"posix_owner_executable":true}}}},"bytes":11,"content_sha256":"{}"}}]}}"#
        ),
        "d".repeat(64),
        "e".repeat(64)
    );
    assert_eq!(bytes, expected_snapshot.as_bytes());
    assert_eq!(
        snapshot.digest().unwrap().as_str(),
        "2ac94e2ca176c1a4efecb324b8d4457666292326052d1d79146302c789a35533"
    );
    snapshot_completed(&snapshot)
        .validate_manifest(&snapshot)
        .unwrap();

    let repository = SealedRepositoryManifestV1 {
        format: RepositoryManifestVersion::GitTreeV1,
        entries: vec![
            SealedRepositoryEntryV1 {
                repository_path: path("README.md"),
                bytes: 7,
                content_sha256: digest('d'),
                mode: GitFileModeV1::Regular,
            },
            SealedRepositoryEntryV1 {
                repository_path: path("src/main.rs"),
                bytes: 11,
                content_sha256: digest('e'),
                mode: GitFileModeV1::Executable,
            },
        ],
    };
    repository.validate_against_snapshot(&snapshot).unwrap();
    assert_eq!(
        String::from_utf8(repository.canonical_bytes().unwrap()).unwrap(),
        format!(
            concat!(
                r#"{{"format":"git_tree_v1","entries":["#,
                r#"{{"repository_path":"README.md","bytes":7,"content_sha256":"{}","mode":"100644"}},"#,
                r#"{{"repository_path":"src/main.rs","bytes":11,"content_sha256":"{}","mode":"100755"}}]}}"#
            ),
            "d".repeat(64),
            "e".repeat(64)
        )
    );
    assert_eq!(
        repository.digest().unwrap().as_str(),
        "e7aae9779c14470e26e3c904a1275a6d3af6b4a48387737fc354ac751b80ba5d"
    );

    let empty_repository = SealedRepositoryManifestV1 {
        format: RepositoryManifestVersion::GitTreeV1,
        entries: Vec::new(),
    };
    assert_eq!(
        empty_repository.canonical_bytes().unwrap(),
        br#"{"format":"git_tree_v1","entries":[]}"#
    );
    assert_eq!(
        empty_repository.digest().unwrap().as_str(),
        "5213a85a33e8a7da55d99c8a2b0c3f7020304126ac7501512b31600f32899996"
    );

    let mut mismatched = repository.clone();
    mismatched.entries[1].content_sha256 = digest('0');
    assert!(mismatched.validate_against_snapshot(&snapshot).is_err());

    let mut unsorted = snapshot.clone();
    unsorted.entries.reverse();
    assert!(unsorted.canonical_bytes().is_err());

    let empty = PinnedEligibleRepositorySnapshotManifestV1 {
        format: PinnedEligibleRepositorySnapshotFormatV1::PinnedEligibleRepositorySnapshotV1,
        entries: Vec::new(),
    };
    assert_eq!(
        empty.canonical_bytes().unwrap(),
        br#"{"format":"pinned_eligible_repository_snapshot_v1","entries":[]}"#
    );
    assert_eq!(
        empty.digest().unwrap().as_str(),
        "1cb129906db9f6244bca402245f78c2cb3f114a31627f822197042862978695d"
    );
    assert_ne!(empty.digest().unwrap(), snapshot.digest().unwrap());
}

#[test]
fn new_symlink_rows_are_typed_without_changing_the_legacy_sealed_file_shape() {
    let legacy = goalcup_protocol::seal::SealedFile {
        path: "link".to_string(),
        bytes: 6,
        sha256: "aa".repeat(32),
    };
    assert_eq!(
        serde_json::to_string(&legacy).unwrap(),
        format!(
            r#"{{"path":"link","bytes":6,"sha256":"{}"}}"#,
            "aa".repeat(32)
        )
    );

    let snapshot = PinnedEligibleRepositorySnapshotManifestV1 {
        format: PinnedEligibleRepositorySnapshotFormatV1::PinnedEligibleRepositorySnapshotV1,
        entries: vec![SnapshotEntryV1 {
            repository_path: path("link"),
            observed_kind: CapturedEntryKindV1::SymbolicLink,
            bytes: 6,
            content_sha256: digest('a'),
        }],
    };
    let repository = SealedRepositoryManifestV1 {
        format: RepositoryManifestVersion::GitTreeV1,
        entries: vec![SealedRepositoryEntryV1 {
            repository_path: path("link"),
            bytes: 6,
            content_sha256: digest('a'),
            mode: GitFileModeV1::Symlink,
        }],
    };
    repository.validate_against_snapshot(&snapshot).unwrap();
    assert!(
        serde_json::to_string(&snapshot)
            .unwrap()
            .contains("symbolic_link")
    );
    assert!(
        serde_json::to_string(&repository)
            .unwrap()
            .contains("120000")
    );
}

#[test]
fn history_body_digest_and_descriptor_bind_every_declared_fact() {
    let selection = selected();
    let body = RequiredGitObjectHistoryManifestV1 {
        format: RequiredGitObjectHistoryFormatV1::RequiredGitObjectHistoryV1,
        object_format: GitObjectFormat::Sha1,
        root_commit: Some(oid('a')),
        objects: vec![
            RequiredGitObjectV1 {
                object_id: oid('1'),
                kind: GitObjectKindV1::Blob,
                uncompressed_bytes: 3,
                content_sha256: digest('1'),
            },
            RequiredGitObjectV1 {
                object_id: oid('a'),
                kind: GitObjectKindV1::Commit,
                uncompressed_bytes: 5,
                content_sha256: digest('2'),
            },
        ],
    };
    let descriptor = RequiredGitObjectHistoryV1 {
        root_commit: Some(oid('a')),
        manifest_sha256: body.digest().unwrap(),
        object_count: 2,
        uncompressed_object_bytes: 8,
    };
    descriptor.validate_manifest(&body, &selection).unwrap();

    let canonical = format!(
        concat!(
            r#"{{"format":"required_git_object_history_v1","object_format":"sha1","root_commit":"{}","objects":["#,
            r#"{{"object_id":"{}","kind":"blob","uncompressed_bytes":3,"content_sha256":"{}"}},"#,
            r#"{{"object_id":"{}","kind":"commit","uncompressed_bytes":5,"content_sha256":"{}"}}]}}"#
        ),
        "a".repeat(40),
        "1".repeat(40),
        "1".repeat(64),
        "a".repeat(40),
        "2".repeat(64)
    );
    assert_eq!(body.canonical_bytes().unwrap(), canonical.as_bytes());
    assert_eq!(
        body.digest().unwrap().as_str(),
        "4e18c1078bdf0f382548541bbd8d65767ee7f49a8d73d4f74cdca9e5271c6407"
    );

    let mut omitted = body.clone();
    omitted.objects.pop();
    assert!(omitted.validate().is_err());
    assert!(descriptor.validate_manifest(&omitted, &selection).is_err());
    let mut duplicated = body.clone();
    duplicated.objects.push(duplicated.objects[1].clone());
    assert!(duplicated.validate().is_err());
    let mut wrong_order = body.clone();
    wrong_order.objects.reverse();
    assert!(wrong_order.validate().is_err());
    let mut wrong_descriptor = descriptor.clone();
    wrong_descriptor.uncompressed_object_bytes += 1;
    assert!(
        wrong_descriptor
            .validate_manifest(&body, &selection)
            .is_err()
    );

    let unborn_selection = SelectedRepositoryV1 {
        pre_seal_head: PreSealHeadV1::Unborn,
        ..selection
    };
    let empty = RequiredGitObjectHistoryManifestV1 {
        format: RequiredGitObjectHistoryFormatV1::RequiredGitObjectHistoryV1,
        object_format: GitObjectFormat::Sha1,
        root_commit: None,
        objects: Vec::new(),
    };
    let empty_descriptor = RequiredGitObjectHistoryV1 {
        root_commit: None,
        manifest_sha256: empty.digest().unwrap(),
        object_count: 0,
        uncompressed_object_bytes: 0,
    };
    empty_descriptor
        .validate_manifest(&empty, &unborn_selection)
        .unwrap();
    let mut nonempty_unborn = empty.clone();
    nonempty_unborn.objects.push(body.objects[0].clone());
    assert!(nonempty_unborn.validate().is_err());
    assert_eq!(
        empty.canonical_bytes().unwrap(),
        br#"{"format":"required_git_object_history_v1","object_format":"sha1","root_commit":null,"objects":[]}"#
    );
    assert_eq!(
        empty.digest().unwrap().as_str(),
        "a93cde128e25c7e317335a3ca93711810329f29335abf1d6388f3a85d6a8d861"
    );
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
    let FinalizationV1::CaptureUnavailable { publisher, .. } = &mut wrong else {
        unreachable!()
    };
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

    captured_with(RepositoryFinalizationV1::ProjectionFailed {
        failure: RepositoryProjectionFailureV1::SnapshotObjectMismatch,
        publisher: PublisherNotAttemptedV1 {
            reason: PublisherNotAttemptedReasonV1::ProjectionFailed,
        },
    })
    .validate()
    .unwrap();

    captured_with(RepositoryFinalizationV1::ProjectionComplete {
        repository_manifest_sha256: digest('7'),
        commit: Box::new(CommitConstructionOutcomeV1::Failed {
            failure: CommitConstructionFailureV1::RequiredHistory {
                reason: RequiredGitHistoryFailureV1::ContainsReleaseExcludedPath,
            },
            publisher: PublisherNotAttemptedV1 {
                reason: PublisherNotAttemptedReasonV1::RequiredHistoryFailed,
            },
        }),
    })
    .validate()
    .unwrap();

    captured_with(RepositoryFinalizationV1::ProjectionComplete {
        repository_manifest_sha256: digest('7'),
        commit: Box::new(CommitConstructionOutcomeV1::Failed {
            failure: CommitConstructionFailureV1::ObjectIdMismatch,
            publisher: PublisherNotAttemptedV1 {
                reason: PublisherNotAttemptedReasonV1::CommitConstructionFailed,
            },
        }),
    })
    .validate()
    .unwrap();

    for reason in [
        PublisherNotAttemptedReasonV1::CredentialUnavailable,
        PublisherNotAttemptedReasonV1::PublisherSetupFailed,
    ] {
        resolved_with(InitialPublicationOutcomeV1::NotAttempted(
            PublisherNotAttemptedV1 { reason },
        ))
        .validate()
        .unwrap();
    }
}

#[test]
fn every_started_publisher_outcome_is_a_valid_local_fact() {
    let outcomes = [
        PublisherAttemptOutcomeV1::PushReported {
            commit: oid('a'),
            tree: oid('b'),
            target_ref: ValidatedHeadsRef::try_from("refs/heads/main").unwrap(),
        },
        PublisherAttemptOutcomeV1::TimedOut {
            phase: PublisherPhaseV1::Transfer,
        },
        PublisherAttemptOutcomeV1::RejectedRemote,
        PublisherAttemptOutcomeV1::Failed {
            phase: PublisherPhaseV1::Confirm,
            reason: PublisherFailureReasonV1::RemoteFailed,
        },
    ];
    for (index, outcome) in outcomes.into_iter().enumerate() {
        let finalization =
            resolved_with(InitialPublicationOutcomeV1::Attempted(PublisherAttemptV1 {
                attempt_id: format!("attempt-{index}"),
                trigger: PublisherAttemptTriggerV1::InitialCutoff,
                required_history_manifest_sha256: digest('8'),
                started_at: at("2026-09-10T00:01:00Z"),
                finished_at: at("2026-09-10T00:01:01Z"),
                outcome,
            }));
        finalization.validate().unwrap();
        let wire = serde_json::to_vec(&finalization).unwrap();
        let back: FinalizationV1 = serde_json::from_slice(&wire).unwrap();
        assert_eq!(back, finalization);
    }
}

#[test]
fn created_commits_must_be_distinct_children_or_unborn_roots() {
    created_with(
        selected(),
        CommitDispositionV1::CreatedSealCommit {
            commit: oid('c'),
            tree: oid('d'),
            parent: Some(oid('a')),
        },
    )
    .validate()
    .expect("a dirty existing repository may create a distinct child commit and tree");

    let same_tree = created_with(
        selected(),
        CommitDispositionV1::CreatedSealCommit {
            commit: oid('c'),
            tree: oid('b'),
            parent: Some(oid('a')),
        },
    )
    .validate();
    let self_parent = created_with(
        selected(),
        CommitDispositionV1::CreatedSealCommit {
            commit: oid('a'),
            tree: oid('d'),
            parent: Some(oid('a')),
        },
    )
    .validate();

    let unborn = SelectedRepositoryV1 {
        pre_seal_head: PreSealHeadV1::Unborn,
        ..selected()
    };
    created_with(
        unborn,
        CommitDispositionV1::CreatedSealCommit {
            commit: oid('c'),
            tree: oid('d'),
            parent: None,
        },
    )
    .validate()
    .expect("an unborn repository creates a root commit without a parent");

    assert_eq!(
        (same_tree.is_err(), self_parent.is_err()),
        (true, true),
        "an unchanged tree must use reused_head, and a created commit cannot parent itself"
    );
}

#[test]
fn canonical_documents_reject_unknown_fields() {
    let manifest = snapshot_manifest();
    let mut value = serde_json::to_value(&manifest).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("unexpected".to_string(), serde_json::json!(true));
    assert!(serde_json::from_value::<PinnedEligibleRepositorySnapshotManifestV1>(value).is_err());

    let finalization = resolved_with(InitialPublicationOutcomeV1::NotAttempted(
        PublisherNotAttemptedV1 {
            reason: PublisherNotAttemptedReasonV1::CredentialUnavailable,
        },
    ));
    let mut value = serde_json::to_value(&finalization).unwrap();
    value["captured"]
        .as_object_mut()
        .unwrap()
        .insert("unexpected".to_string(), serde_json::json!(true));
    assert!(serde_json::from_value::<FinalizationV1>(value).is_err());
}

fn recovery_claims() -> PublicationRecoveryAttemptClaimsV1 {
    PublicationRecoveryAttemptClaimsV1 {
        entry_id: "e1".to_string(),
        session_id: "s1".to_string(),
        receipt_sha256: digest('1'),
        terminal_chain_hash: digest('2'),
        snapshot_manifest_sha256: digest('3'),
        repository_manifest_sha256: digest('4'),
        required_history_manifest_sha256: digest('5'),
        remote: remote(),
        target_ref: ValidatedHeadsRef::try_from("refs/heads/main").unwrap(),
        object_format: GitObjectFormat::Sha1,
        commit: oid('6'),
        tree: oid('7'),
        attempt_number: 1,
        previous: RecoveryPreviousV1::Terminal(digest('2')),
        trigger: RecoveryTriggerV1::AutomaticRestart,
        started_at: at("2026-09-10T00:01:00Z"),
        finished_at: at("2026-09-10T00:01:02Z"),
        outcome: RecoveryAttemptOutcomeV1::PushReported {
            commit: oid('6'),
            tree: oid('7'),
            target_ref: ValidatedHeadsRef::try_from("refs/heads/main").unwrap(),
        },
    }
}

#[test]
fn recovery_and_observation_signatures_require_an_independently_pinned_key() {
    let run_key = SigningKey::from_bytes(&[7; 32]);
    let other_key = SigningKey::from_bytes(&[8; 32]);
    let run_pin = hex::encode(run_key.verifying_key().to_bytes());
    let other_pin = hex::encode(other_key.verifying_key().to_bytes());

    let signed = SignedPublicationRecoveryAttemptV1::sign(recovery_claims(), &run_key).unwrap();
    signed.verify_with(&run_pin).unwrap();
    signed.verify_with(&run_pin.to_ascii_uppercase()).unwrap();
    assert!(signed.verify_with(&other_pin).is_err());
    let original_digest = signed.complete_digest().unwrap();
    let mut edited = signed.clone();
    edited.claims.trigger = RecoveryTriggerV1::OperatorRequested;
    assert!(edited.verify_with(&run_pin).is_err());
    assert_ne!(edited.complete_digest().unwrap(), original_digest);

    let claims = PublicationObservationClaimsV1 {
        entry_id: "e1".to_string(),
        session_id: "s1".to_string(),
        receipt_sha256: digest('1'),
        terminal_chain_hash: digest('2'),
        snapshot_manifest_sha256: digest('3'),
        repository_manifest_sha256: digest('4'),
        remote: remote(),
        target_ref: ValidatedHeadsRef::try_from("refs/heads/main").unwrap(),
        object_format: GitObjectFormat::Sha1,
        commit: oid('6'),
        tree: oid('7'),
        first_observed_at: at("2026-09-10T00:02:00Z"),
    };
    let organizer = PublicationObservationReceiptV1::sign(claims, &run_key).unwrap();
    organizer.verify_with(&run_pin).unwrap();
    organizer
        .verify_with(&run_pin.to_ascii_uppercase())
        .unwrap();
    assert!(organizer.verify_with(&other_pin).is_err());

    let mut uppercase_recovery = signed;
    uppercase_recovery.signature.make_ascii_uppercase();
    let mut uppercase_observation = organizer;
    uppercase_observation.signature.make_ascii_uppercase();
    assert_eq!(
        (
            uppercase_recovery.verify_with(&run_pin),
            uppercase_observation.verify_with(&run_pin),
        ),
        (
            Err(ProvenanceError::SignatureMalformed),
            Err(ProvenanceError::SignatureMalformed),
        ),
        "new signed documents require their serialized signature to be canonical lowercase hex"
    );
}

#[test]
fn observation_sets_distinguish_complete_from_unavailable_and_validate_widths() {
    let observed = RepositoryObservedV1 {
        transition: RepositoryTransitionV1::Appeared,
        state: ObservedRepositoryStateV1::Present {
            workspace_relative_root: root(""),
            object_format: GitObjectFormat::Sha1,
            head: ObservedHeadV1::Branch {
                target_ref: ValidatedHeadsRef::try_from("refs/heads/main").unwrap(),
                commit: oid('a'),
                tree: oid('b'),
            },
            parents: CompleteOrUnavailableV1::Complete(vec![oid('c')]),
            remotes: CompleteOrUnavailableV1::Unavailable {
                reason: RepositoryObservationUnavailableReasonV1::BoundExceeded,
            },
            commits: CompleteOrUnavailableV1::Complete(vec![oid('a')]),
        },
    };
    observed.validate().unwrap();
    let json = serde_json::to_value(&observed).unwrap();
    assert!(json.get("recorded_at").is_none());
    assert!(json.get("prev").is_none());

    let disappeared = RepositoryObservedV1 {
        transition: RepositoryTransitionV1::Disappeared,
        state: ObservedRepositoryStateV1::Disappeared {
            workspace_relative_root: root("repo"),
        },
    };
    disappeared.validate().unwrap();
    let mut invalid = disappeared;
    invalid.transition = RepositoryTransitionV1::Updated;
    assert!(invalid.validate().is_err());
}
