#![cfg(feature = "v1-compat")]
use goalcup_daemon::submission::{
    CaptureResourceLimitsV1, SubmissionRequestV1, capture_submission, prepare_source_journal,
    readback_submission,
};
use goalcup_platform::handle_budget::NativeHandleBudgetV1;
use goalcup_platform::sourcecap::SourceDir;
use goalcup_platform::tracecap::EntryArtifactRoot;
use goalcup_protocol::chain::Link;
use goalcup_protocol::provenance::Sha256Digest;
use goalcup_protocol::submission::{
    EffectiveSubmissionLimitsV1, FinishReasonV1, LimitProfileV1, SubmissionFailureV1,
    SubmissionKindV1,
};
use std::os::unix::fs::{PermissionsExt as _, symlink};

fn digest(hex: &str) -> Sha256Digest {
    Sha256Digest::try_from(hex).unwrap()
}

fn request() -> SubmissionRequestV1 {
    SubmissionRequestV1 {
        generation: 3,
        entry_id: "entry-0001".to_string(),
        receipt_sha256: digest(&"01".repeat(32)),
        cutoff: Link {
            seq: 7,
            prev: "02".repeat(32),
            hash: "03".repeat(32),
        },
        reason: FinishReasonV1::Manual,
        excluded: vec![".goalcup".to_string(), ".git".to_string()],
        limit_profile: LimitProfileV1 {
            id: "test-snapshot-v1".to_string(),
        },
        limits: EffectiveSubmissionLimitsV1 {
            object_count: 32,
            total_bytes: 1 << 20,
            pre_cutoff_drain: 1_000,
            process_walk: 64,
        },
        capture_limits: CaptureResourceLimitsV1 {
            version: 1,
            namespace_nodes: 4_096,
            inventory_bytes: 8 * 1024 * 1024,
            manifest_bytes: 8 * 1024 * 1024,
        },
    }
}

#[test]
fn capture_retains_streamed_workspace_bytes_after_source_mutation_and_deletion() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let artifact_root = temporary.path().join("artifact-root");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&artifact_root).unwrap();
    std::fs::create_dir(workspace.join("nested")).unwrap();
    std::fs::create_dir(workspace.join("nested/.git")).unwrap();
    std::fs::create_dir(workspace.join(".git")).unwrap();
    std::fs::create_dir(workspace.join(".goalcup")).unwrap();
    std::fs::write(workspace.join("empty"), b"").unwrap();
    std::fs::write(workspace.join("duplicate-a"), b"same bytes").unwrap();
    std::fs::write(workspace.join("duplicate-b"), b"same bytes").unwrap();
    std::fs::write(workspace.join("nested/naïve.txt"), b"unicode bytes").unwrap();
    std::fs::write(workspace.join(".goalcup/ignored"), b"tracker state").unwrap();
    std::fs::write(workspace.join(".git/config"), b"root repository metadata").unwrap();
    std::fs::write(
        workspace.join("nested/.git/config"),
        b"nested repository remains content",
    )
    .unwrap();
    std::fs::write(workspace.join("credential.txt"), b"synthetic-secret-only").unwrap();
    std::fs::write(workspace.join("run.sh"), b"#!/bin/sh\necho retained\n").unwrap();
    std::fs::set_permissions(
        workspace.join("run.sh"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    symlink("missing-target", workspace.join("dangling-link")).unwrap();

    let budget = NativeHandleBudgetV1::new();
    let source = SourceDir::open_selected(&workspace, &budget).unwrap();
    let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();
    let trace = artifact.create_trace().unwrap();
    let journal = prepare_source_journal(&source, &artifact).unwrap();
    let captured = capture_submission(&source, &trace, &journal, &request()).unwrap();
    let retained = readback_submission(&trace, &captured).unwrap();

    let capture_journal = artifact_root.join("artifacts/entry-0001/trace/.submission-capture-v1");
    let mut journal_records = std::fs::read_dir(capture_journal)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    journal_records.sort();
    assert_eq!(
        journal_records,
        [
            "capture.lock",
            "copy.json",
            "inventory.json",
            "manifest-planned.json",
            "objects-verified.json",
            "owner.json",
            "promoted.json",
            "resolution.json",
            "stage.json",
            "staged-verified.json",
        ]
    );

    assert_eq!(retained.digest().unwrap(), captured.manifest_sha256);
    assert_eq!(
        retained
            .entries
            .iter()
            .filter(|entry| entry.path.as_str() == "empty")
            .count(),
        1
    );
    assert_eq!(
        retained
            .entries
            .iter()
            .filter(|entry| entry.kind == SubmissionKindV1::SymbolicLink)
            .count(),
        1
    );
    assert!(
        retained
            .entries
            .iter()
            .any(|entry| entry.path.as_str() == "nested/.git/config")
    );
    assert!(
        !retained
            .entries
            .iter()
            .any(|entry| entry.path.as_str().starts_with(".git/"))
    );
    assert!(
        !retained
            .entries
            .iter()
            .any(|entry| entry.path.as_str().starts_with(".goalcup/"))
    );
    let duplicate_a = retained
        .entries
        .iter()
        .find(|entry| entry.path.as_str() == "duplicate-a")
        .unwrap();
    let duplicate_b = retained
        .entries
        .iter()
        .find(|entry| entry.path.as_str() == "duplicate-b")
        .unwrap();
    assert_eq!(duplicate_a.content_sha256, duplicate_b.content_sha256);
    assert_eq!(retained.object_count + 1, retained.entries.len() as u64);

    let manifest_digest = retained.digest().unwrap();
    std::fs::write(workspace.join("duplicate-a"), b"mutated bytes").unwrap();
    std::fs::remove_dir_all(&workspace).unwrap();
    let reopened = readback_submission(&trace, &captured).unwrap();
    assert_eq!(reopened.digest().unwrap(), manifest_digest);
    assert_eq!(reopened.entries, retained.entries);
}

#[test]
fn capture_enforces_logical_entry_bounds_before_publishing_a_submission() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let artifact_root = temporary.path().join("artifact-root");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&artifact_root).unwrap();
    std::fs::write(workspace.join("one"), b"first").unwrap();
    std::fs::write(workspace.join("two"), b"second").unwrap();

    let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
    let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();
    let trace = artifact.create_trace().unwrap();
    let journal = prepare_source_journal(&source, &artifact).unwrap();
    let mut bounded = request();
    bounded.limits.object_count = 1;
    let error = capture_submission(&source, &trace, &journal, &bounded).unwrap_err();

    assert_eq!(
        error.submission_failure(),
        Some(SubmissionFailureV1::LimitExceeded)
    );
    assert!(trace.open_child_dir("submission").is_err());
}

#[test]
fn source_namespace_is_bounded_before_a_directory_listing_is_retained() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let artifact_root = temporary.path().join("artifact-root");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&artifact_root).unwrap();
    for index in 0..8 {
        std::fs::create_dir(workspace.join(format!("directory-{index}"))).unwrap();
    }

    let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
    let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();
    let trace = artifact.create_trace().unwrap();
    let journal = prepare_source_journal(&source, &artifact).unwrap();
    let mut bounded = request();
    bounded.capture_limits.namespace_nodes = 4;

    let error = capture_submission(&source, &trace, &journal, &bounded).unwrap_err();
    assert_eq!(
        error.submission_failure(),
        Some(SubmissionFailureV1::LimitExceeded)
    );
    assert!(
        trace
            .child_names(8)
            .unwrap()
            .iter()
            .all(|name| !name.starts_with("submission-staging-v1-"))
    );
    assert!(trace.open_child_dir("submission").is_err());
}

#[test]
fn declared_inventory_and_manifest_byte_budgets_refuse_before_phase_commit() {
    for phase in ["inventory", "manifest"] {
        let temporary = tempfile::tempdir().unwrap();
        let workspace = temporary.path().join("workspace");
        let artifact_root = temporary.path().join("artifact-root");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&artifact_root).unwrap();
        for index in 0..24 {
            let name = format!("{index:02}-{}", "long-participant-name".repeat(4));
            if phase == "inventory" {
                std::fs::create_dir(workspace.join(name)).unwrap();
            } else {
                std::fs::write(workspace.join(name), b"x").unwrap();
            }
        }

        let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
        let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();
        let trace = artifact.create_trace().unwrap();
        let journal = prepare_source_journal(&source, &artifact).unwrap();
        let mut bounded = request();
        if phase == "inventory" {
            bounded.capture_limits.inventory_bytes = 1_024;
        } else {
            bounded.capture_limits.manifest_bytes = 1_200;
        }

        let error = capture_submission(&source, &trace, &journal, &bounded).unwrap_err();
        assert_eq!(
            error.submission_failure(),
            Some(SubmissionFailureV1::LimitExceeded),
            "phase {phase}"
        );
        let journal_root = artifact_root
            .join("artifacts/entry-0001/trace")
            .join(".submission-capture-v1");
        let phase_record = if phase == "inventory" {
            "inventory.json"
        } else {
            "copy.json"
        };
        assert!(!journal_root.join(phase_record).exists(), "phase {phase}");
        assert!(trace.open_child_dir("submission").is_err());
    }
}

#[test]
fn failed_capture_records_one_immutable_incomplete_resolution() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let artifact_root = temporary.path().join("artifact-root");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&artifact_root).unwrap();
    std::fs::write(workspace.join("not\\portable"), b"refused").unwrap();

    let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
    let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();
    let trace = artifact.create_trace().unwrap();
    let journal = prepare_source_journal(&source, &artifact).unwrap();
    let first = capture_submission(&source, &trace, &journal, &request()).unwrap_err();
    assert_eq!(
        first.submission_failure(),
        Some(SubmissionFailureV1::UnsupportedEntry)
    );

    let resolution =
        artifact_root.join("artifacts/entry-0001/trace/.submission-capture-v1/resolution.json");
    assert!(resolution.is_file());
    let retained = std::fs::read(&resolution).unwrap();

    std::fs::remove_file(workspace.join("not\\portable")).unwrap();
    std::fs::write(workspace.join("portable"), b"newer bytes").unwrap();
    let retry = capture_submission(&source, &trace, &journal, &request()).unwrap_err();
    assert_eq!(
        retry.submission_failure(),
        Some(SubmissionFailureV1::UnsupportedEntry)
    );
    assert_eq!(std::fs::read(resolution).unwrap(), retained);
    assert!(trace.open_child_dir("submission").is_err());
}

#[test]
fn capture_enforces_the_logical_total_byte_bound_before_promotion() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let artifact_root = temporary.path().join("artifact-root");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&artifact_root).unwrap();
    std::fs::write(workspace.join("one"), b"1234").unwrap();
    std::fs::write(workspace.join("two"), b"5678").unwrap();

    let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
    let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();
    let trace = artifact.create_trace().unwrap();
    let journal = prepare_source_journal(&source, &artifact).unwrap();
    let mut bounded = request();
    bounded.limits.total_bytes = 7;
    let error = capture_submission(&source, &trace, &journal, &bounded).unwrap_err();

    assert_eq!(
        error.submission_failure(),
        Some(SubmissionFailureV1::LimitExceeded)
    );
    assert!(trace.open_child_dir("submission").is_err());
}

#[test]
fn unreadable_regular_input_fails_closed_without_promotion() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let artifact_root = temporary.path().join("artifact-root");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&artifact_root).unwrap();
    let unreadable = workspace.join("unreadable");
    std::fs::write(&unreadable, b"not readable during capture").unwrap();
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();

    let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
    let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();
    let trace = artifact.create_trace().unwrap();
    let journal = prepare_source_journal(&source, &artifact).unwrap();
    let error = capture_submission(&source, &trace, &journal, &request()).unwrap_err();

    assert_eq!(
        error.submission_failure(),
        Some(SubmissionFailureV1::Unreadable)
    );
    assert!(trace.open_child_dir("submission").is_err());
}

#[test]
fn source_journal_recovery_only_reuses_the_original_private_identity_binding() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let artifact_root = temporary.path().join("artifact-root");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&artifact_root).unwrap();
    let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
    let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();

    let first = prepare_source_journal(&source, &artifact).unwrap();
    let recovered = prepare_source_journal(&source, &artifact).unwrap();
    assert_eq!(recovered, first);
    std::fs::set_permissions(
        artifact_root.join("artifacts/entry-0001/sealing-source.json"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert_eq!(
        prepare_source_journal(&source, &artifact)
            .unwrap_err()
            .submission_failure(),
        Some(SubmissionFailureV1::RecoveryAmbiguous)
    );
}

#[test]
fn readback_rejects_mode_relaxation_and_a_hardlinked_object() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let artifact_root = temporary.path().join("artifact-root");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&artifact_root).unwrap();
    std::fs::write(workspace.join("payload"), b"private immutable bytes").unwrap();

    let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
    let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();
    let trace = artifact.create_trace().unwrap();
    let journal = prepare_source_journal(&source, &artifact).unwrap();
    let captured = capture_submission(&source, &trace, &journal, &request()).unwrap();
    let retained = readback_submission(&trace, &captured).unwrap();
    let object = artifact_root
        .join("artifacts/entry-0001/trace/submission/objects/sha256")
        .join(retained.entries[0].content_sha256.as_str());

    std::fs::set_permissions(&object, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        readback_submission(&trace, &captured)
            .unwrap_err()
            .submission_failure(),
        Some(SubmissionFailureV1::ReadbackFailed)
    );
    std::fs::set_permissions(&object, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::hard_link(&object, temporary.path().join("outside-hardlink")).unwrap();
    assert_eq!(
        readback_submission(&trace, &captured)
            .unwrap_err()
            .submission_failure(),
        Some(SubmissionFailureV1::ReadbackFailed)
    );
    std::fs::remove_file(temporary.path().join("outside-hardlink")).unwrap();
    let submission = trace.open_child_dir("submission").unwrap();
    submission.create_regular_exclusive("unexpected").unwrap();
    assert_eq!(
        readback_submission(&trace, &captured)
            .unwrap_err()
            .submission_failure(),
        Some(SubmissionFailureV1::ReadbackFailed)
    );
}

#[test]
fn streams_a_large_object_without_a_full_file_capture_buffer() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let artifact_root = temporary.path().join("artifact-root");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&artifact_root).unwrap();
    let mut large = std::fs::File::create(workspace.join("large.bin")).unwrap();
    let chunk = [0x5au8; 64 * 1024];
    for _ in 0..512 {
        use std::io::Write as _;
        large.write_all(&chunk).unwrap();
    }
    drop(large);

    let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
    let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();
    let trace = artifact.create_trace().unwrap();
    let journal = prepare_source_journal(&source, &artifact).unwrap();
    let mut large_limits = request();
    large_limits.limits.total_bytes = 32 * 1024 * 1024;
    let captured = capture_submission(&source, &trace, &journal, &large_limits).unwrap();
    let retained = readback_submission(&trace, &captured).unwrap();

    assert_eq!(retained.entries.len(), 1);
    assert_eq!(retained.entries[0].bytes, 32 * 1024 * 1024);
    assert_eq!(retained.digest().unwrap(), captured.manifest_sha256);
}

#[test]
fn structural_directories_do_not_consume_the_manifest_entry_limit() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let artifact_root = temporary.path().join("artifact-root");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&artifact_root).unwrap();
    for index in 0..32 {
        std::fs::create_dir(workspace.join(format!("empty-{index:02}"))).unwrap();
    }
    std::fs::write(workspace.join("only-entry"), b"one retained entry").unwrap();

    let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
    let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();
    let trace = artifact.create_trace().unwrap();
    let journal = prepare_source_journal(&source, &artifact).unwrap();
    let mut bounded = request();
    bounded.limits.object_count = 1;

    let captured = capture_submission(&source, &trace, &journal, &bounded).unwrap();
    let retained = readback_submission(&trace, &captured).unwrap();
    assert_eq!(retained.entries.len(), 1);
    assert_eq!(retained.entries[0].path.as_str(), "only-entry");
    assert_eq!(retained.object_count, 1);
}

#[test]
fn retry_reuses_the_bound_artifact_without_resnapshotting_replacement_bytes() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let detached = temporary.path().join("detached-workspace");
    let artifact_root = temporary.path().join("artifact-root");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&artifact_root).unwrap();
    std::fs::write(workspace.join("answer.txt"), b"pre-cutoff bytes").unwrap();

    let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
    let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();
    let trace = artifact.create_trace().unwrap();
    let journal = prepare_source_journal(&source, &artifact).unwrap();
    let first = capture_submission(&source, &trace, &journal, &request()).unwrap();

    std::fs::rename(&workspace, &detached).unwrap();
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("answer.txt"), b"post-cutoff replacement").unwrap();
    let replacement = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();

    let recovered = capture_submission(&replacement, &trace, &journal, &request()).unwrap();
    assert_eq!(recovered, first);
    let retained = readback_submission(&trace, &recovered).unwrap();
    assert_eq!(retained.entries.len(), 1);
    assert_eq!(retained.entries[0].bytes, 16);
}

#[test]
fn a_foreign_fixed_staging_name_cannot_block_the_generation_bound_writer() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let artifact_root = temporary.path().join("artifact-root");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&artifact_root).unwrap();
    std::fs::write(workspace.join("payload"), b"retained").unwrap();

    let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
    let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();
    let trace = artifact.create_trace().unwrap();
    let foreign = trace.create_child_dir("submission-staging").unwrap();
    foreign.create_regular_exclusive("foreign").unwrap();
    let journal = prepare_source_journal(&source, &artifact).unwrap();

    let captured = capture_submission(&source, &trace, &journal, &request()).unwrap();
    assert!(foreign.read_regular("foreign").unwrap().is_some());
    readback_submission(&trace, &captured).unwrap();
}

#[test]
fn readback_rejects_semantically_equal_noncanonical_manifest_bytes() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let artifact_root = temporary.path().join("artifact-root");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&artifact_root).unwrap();
    std::fs::write(workspace.join("payload"), b"retained").unwrap();

    let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
    let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();
    let trace = artifact.create_trace().unwrap();
    let journal = prepare_source_journal(&source, &artifact).unwrap();
    let captured = capture_submission(&source, &trace, &journal, &request()).unwrap();
    let manifest_path = artifact_root.join("artifacts/entry-0001/trace/submission/manifest.json");
    let canonical = std::fs::read(&manifest_path).unwrap();
    let canonical_text = String::from_utf8(canonical.clone()).unwrap();
    let original_prefix = concat!(
        "{\"profile\":\"workspace_snapshot_v1\",",
        "\"limit_profile\":{\"id\":\"test-snapshot-v1\"},"
    );
    let reordered_prefix = concat!(
        "{\"limit_profile\":{\"id\":\"test-snapshot-v1\"},",
        "\"profile\":\"workspace_snapshot_v1\","
    );
    assert!(canonical_text.starts_with(original_prefix));
    let noncanonical = canonical_text.replacen(original_prefix, reordered_prefix, 1);
    assert_eq!(noncanonical.len(), canonical.len());
    std::fs::write(&manifest_path, noncanonical).unwrap();

    assert_eq!(
        readback_submission(&trace, &captured)
            .unwrap_err()
            .submission_failure(),
        Some(SubmissionFailureV1::ReadbackFailed)
    );
}

#[test]
fn readback_rejects_an_oversize_manifest_before_parsing_it() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let artifact_root = temporary.path().join("artifact-root");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&artifact_root).unwrap();
    std::fs::write(workspace.join("payload"), b"retained").unwrap();

    let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
    let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();
    let trace = artifact.create_trace().unwrap();
    let journal = prepare_source_journal(&source, &artifact).unwrap();
    let captured = capture_submission(&source, &trace, &journal, &request()).unwrap();
    let manifest_path = artifact_root.join("artifacts/entry-0001/trace/submission/manifest.json");
    let canonical = std::fs::read(&manifest_path).unwrap();
    let mut oversized = vec![b' '; 9 * 1024 * 1024];
    oversized.extend_from_slice(&canonical);
    std::fs::write(&manifest_path, oversized).unwrap();

    assert_eq!(
        readback_submission(&trace, &captured)
            .unwrap_err()
            .submission_failure(),
        Some(SubmissionFailureV1::ReadbackFailed)
    );
}

#[test]
fn special_and_nonportable_source_entries_fail_closed_without_promotion() {
    use std::os::unix::net::UnixListener;

    for kind in ["socket", "nonportable"] {
        let temporary = tempfile::tempdir().unwrap();
        let workspace = temporary.path().join("workspace");
        let artifact_root = temporary.path().join("artifact-root");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&artifact_root).unwrap();
        let _listener = if kind == "socket" {
            Some(UnixListener::bind(workspace.join("socket")).unwrap())
        } else {
            std::fs::write(workspace.join("not\\portable"), b"x").unwrap();
            None
        };

        let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
        let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();
        let trace = artifact.create_trace().unwrap();
        let journal = prepare_source_journal(&source, &artifact).unwrap();
        let error = capture_submission(&source, &trace, &journal, &request()).unwrap_err();

        assert_eq!(
            error.submission_failure(),
            Some(SubmissionFailureV1::UnsupportedEntry)
        );
        assert!(trace.open_child_dir("submission").is_err());
    }
}

#[test]
fn fifo_input_is_refused_as_special_without_blocking_or_promotion() {
    use std::os::unix::ffi::OsStrExt as _;

    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let artifact_root = temporary.path().join("artifact-root");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&artifact_root).unwrap();
    let fifo = std::ffi::CString::new(workspace.join("fifo").as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);

    let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
    let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();
    let trace = artifact.create_trace().unwrap();
    let journal = prepare_source_journal(&source, &artifact).unwrap();
    let error = capture_submission(&source, &trace, &journal, &request()).unwrap_err();

    assert_eq!(
        error.submission_failure(),
        Some(SubmissionFailureV1::UnsupportedEntry)
    );
    assert!(trace.open_child_dir("submission").is_err());
}

#[test]
fn promotion_collision_preserves_the_foreign_submission_and_owned_stage() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let artifact_root = temporary.path().join("artifact-root");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&artifact_root).unwrap();
    std::fs::write(workspace.join("payload"), b"retained").unwrap();

    let source = SourceDir::open_selected(&workspace, &NativeHandleBudgetV1::new()).unwrap();
    let artifact = EntryArtifactRoot::create(&artifact_root, "entry-0001").unwrap();
    let trace = artifact.create_trace().unwrap();
    let foreign = trace.create_child_dir("submission").unwrap();
    let mut sentinel = foreign.create_regular_exclusive("foreign").unwrap();
    use std::io::Write as _;
    sentinel.write_all(b"foreign remains").unwrap();
    sentinel.sync_all().unwrap();
    let journal = prepare_source_journal(&source, &artifact).unwrap();

    let error = capture_submission(&source, &trace, &journal, &request()).unwrap_err();
    assert_eq!(
        error.submission_failure(),
        Some(SubmissionFailureV1::RecoveryAmbiguous)
    );
    let mut retained = foreign.read_regular("foreign").unwrap().unwrap();
    let mut bytes = Vec::new();
    use std::io::Read as _;
    retained.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"foreign remains");
    assert!(
        trace
            .child_names(8)
            .unwrap()
            .iter()
            .any(|name| name.starts_with("submission-staging"))
    );
}
