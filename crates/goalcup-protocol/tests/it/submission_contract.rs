use goalcup_protocol::provenance::{
    PortableRelativePath, ProvenanceError, Rfc3339Utc, Sha256Digest,
};
use goalcup_protocol::submission::{
    EffectiveSubmissionLimitsV1, FinishReasonV1, LimitProfileV1, SubmissionEntryV1,
    SubmissionKindV1, SubmissionManifestV1, SubmissionProfileV1,
};
use sha2::{Digest, Sha256};

fn digest(byte: char) -> Sha256Digest {
    Sha256Digest::try_from(byte.to_string().repeat(64)).unwrap()
}

fn at(value: &str) -> Rfc3339Utc {
    Rfc3339Utc::try_from(value).unwrap()
}

fn entry(
    path: &str,
    kind: SubmissionKindV1,
    executable: bool,
    bytes: u64,
    content: char,
) -> SubmissionEntryV1 {
    SubmissionEntryV1 {
        path: PortableRelativePath::try_from(path).unwrap(),
        kind,
        posix_owner_executable: executable,
        bytes,
        content_sha256: digest(content),
    }
}

fn manifest() -> SubmissionManifestV1 {
    SubmissionManifestV1 {
        profile: SubmissionProfileV1::WorkspaceSnapshotV1,
        limit_profile: LimitProfileV1 {
            id: "workspace-v1".to_string(),
        },
        effective_limits: EffectiveSubmissionLimitsV1 {
            object_count: 10,
            total_bytes: 100,
            pre_cutoff_drain: 2_000,
            process_walk: 3_000,
        },
        entry_id: "entry-1".to_string(),
        receipt_sha256: digest('a'),
        cutoff_seq: 9,
        cutoff_chain_hash: digest('b'),
        finish_reason: FinishReasonV1::Deadline,
        entries: vec![
            entry("bin/empty", SubmissionKindV1::Regular, false, 0, 'c'),
            entry("bin/run.sh", SubmissionKindV1::Regular, true, 7, 'd'),
            entry("docs/café.txt", SubmissionKindV1::Regular, false, 7, 'd'),
            entry(
                "links/current",
                SubmissionKindV1::SymbolicLink,
                false,
                4,
                'e',
            ),
        ],
        excluded: vec![".git".to_string(), "target".to_string()],
        copy_started_at: at("2026-09-10T00:00:00Z"),
        copy_completed_at: at("2026-09-10T00:00:01.5Z"),
        readback_verified_at: at("2026-09-10T00:00:02Z"),
        object_count: 3,
        total_bytes: 18,
    }
}

#[test]
fn canonical_manifest_covers_modes_symlinks_unicode_duplicates_and_limit_units() {
    let manifest = manifest();
    let bytes = manifest.canonical_bytes().unwrap();
    let expected = format!(
        concat!(
            "{{\"profile\":\"workspace_snapshot_v1\",",
            "\"limit_profile\":{{\"id\":\"workspace-v1\"}},",
            "\"effective_limits\":{{\"object_count\":10,\"total_bytes\":100,",
            "\"pre_cutoff_drain\":2000,\"process_walk\":3000}},",
            "\"entry_id\":\"entry-1\",\"receipt_sha256\":\"{}\",",
            "\"cutoff_seq\":9,\"cutoff_chain_hash\":\"{}\",",
            "\"finish_reason\":\"deadline\",\"entries\":[",
            "{{\"path\":\"bin/empty\",\"kind\":\"regular\",",
            "\"posix_owner_executable\":false,\"bytes\":0,\"content_sha256\":\"{}\"}},",
            "{{\"path\":\"bin/run.sh\",\"kind\":\"regular\",",
            "\"posix_owner_executable\":true,\"bytes\":7,\"content_sha256\":\"{}\"}},",
            "{{\"path\":\"docs/café.txt\",\"kind\":\"regular\",",
            "\"posix_owner_executable\":false,\"bytes\":7,\"content_sha256\":\"{}\"}},",
            "{{\"path\":\"links/current\",\"kind\":\"symbolic_link\",",
            "\"posix_owner_executable\":false,\"bytes\":4,\"content_sha256\":\"{}\"}}],",
            "\"excluded\":[\".git\",\"target\"],",
            "\"copy_started_at\":\"2026-09-10T00:00:00Z\",",
            "\"copy_completed_at\":\"2026-09-10T00:00:01.5Z\",",
            "\"readback_verified_at\":\"2026-09-10T00:00:02Z\",",
            "\"object_count\":3,\"total_bytes\":18}}"
        ),
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
        "d".repeat(64),
        "d".repeat(64),
        "e".repeat(64),
    );
    assert_eq!(bytes, expected.as_bytes());

    let expected_digest: [u8; 32] = Sha256::digest(&bytes).into();
    assert_eq!(
        manifest.digest().unwrap(),
        Sha256Digest::from_bytes(expected_digest)
    );
}

#[test]
fn manifest_rejects_non_positive_limits_and_unversioned_identifiers() {
    let mut candidate = manifest();
    candidate.effective_limits.object_count = 0;
    assert!(candidate.validate().is_err());

    let mut candidate = manifest();
    candidate.effective_limits.total_bytes = 0;
    assert!(candidate.validate().is_err());

    let mut candidate = manifest();
    candidate.effective_limits.pre_cutoff_drain = 0;
    assert!(candidate.validate().is_err());

    let mut candidate = manifest();
    candidate.effective_limits.process_walk = 0;
    assert!(candidate.validate().is_err());

    let mut candidate = manifest();
    candidate.limit_profile.id.clear();
    assert!(candidate.validate().is_err());

    let mut candidate = manifest();
    candidate.entry_id.clear();
    assert!(candidate.validate().is_err());
}

#[test]
fn manifest_rejects_unsorted_or_duplicate_paths_and_exclusions() {
    let mut candidate = manifest();
    candidate.entries.swap(0, 1);
    assert!(candidate.validate().is_err());

    let mut candidate = manifest();
    candidate.entries[1].path = candidate.entries[0].path.clone();
    assert!(candidate.validate().is_err());

    let mut candidate = manifest();
    candidate.excluded.swap(0, 1);
    assert!(candidate.validate().is_err());

    let mut candidate = manifest();
    candidate.excluded[1] = candidate.excluded[0].clone();
    assert!(candidate.validate().is_err());
}

#[test]
fn manifest_rejects_reused_digest_with_different_length_and_executable_symlink() {
    let mut candidate = manifest();
    candidate.entries[2].bytes = 8;
    candidate.total_bytes = 19;
    assert!(candidate.validate().is_err());

    let mut candidate = manifest();
    candidate.entries[3].posix_owner_executable = true;
    assert!(candidate.validate().is_err());
}

#[test]
fn manifest_checks_logical_and_physical_totals_counts_and_overflow() {
    let mut candidate = manifest();
    candidate.total_bytes = 17;
    assert!(candidate.validate().is_err());

    let mut candidate = manifest();
    candidate.object_count = 4;
    assert!(candidate.validate().is_err());

    let mut candidate = manifest();
    candidate.effective_limits.object_count = 3;
    assert!(
        candidate.validate().is_err(),
        "entry count has its own object-count bound"
    );

    let mut candidate = manifest();
    candidate.effective_limits.total_bytes = 17;
    assert!(
        matches!(
            candidate.validate(),
            Err(ProvenanceError::InvalidField {
                field: "total_bytes",
                reason: "logical bytes exceed the effective limit",
            })
        ),
        "the logical limit also bounds physical distinct-object bytes because they are a subset"
    );

    let mut candidate = manifest();
    candidate.entries[0].bytes = u64::MAX;
    candidate.entries[1].bytes = 1;
    candidate.entries[2].content_sha256 = digest('f');
    candidate.object_count = 4;
    candidate.total_bytes = u64::MAX;
    candidate.effective_limits.total_bytes = u64::MAX;
    assert!(matches!(
        candidate.validate(),
        Err(ProvenanceError::IntegerOverflow {
            field: "total_bytes"
        })
    ));
}

#[test]
fn manifest_rejects_reversed_copy_and_readback_times() {
    let mut candidate = manifest();
    candidate.copy_completed_at = at("2026-09-09T23:59:59Z");
    assert!(candidate.validate().is_err());

    let mut candidate = manifest();
    candidate.readback_verified_at = at("2026-09-10T00:00:01Z");
    assert!(candidate.validate().is_err());
}

#[test]
fn deserialization_fails_closed_for_invalid_primitives_and_unknown_fields() {
    let json = serde_json::to_value(manifest()).unwrap();
    for (field, value) in [
        ("receipt_sha256", serde_json::json!("ABC")),
        (
            "copy_started_at",
            serde_json::json!("2026-09-10T00:00:00+00:00"),
        ),
    ] {
        let mut broken = json.clone();
        broken[field] = value;
        assert!(serde_json::from_value::<SubmissionManifestV1>(broken).is_err());
    }

    let mut broken = json.clone();
    broken["entries"][0]["path"] = serde_json::json!("../escape");
    assert!(serde_json::from_value::<SubmissionManifestV1>(broken).is_err());

    let mut broken = json.clone();
    broken["future"] = serde_json::json!(true);
    assert!(serde_json::from_value::<SubmissionManifestV1>(broken).is_err());

    let mut broken = json;
    broken["entries"][0]["future"] = serde_json::json!(true);
    assert!(serde_json::from_value::<SubmissionManifestV1>(broken).is_err());
}

#[test]
fn unknown_submission_profile_is_not_legacy() {
    use goalcup_protocol::submission::SubmissionProfileV1;
    assert!(serde_json::from_str::<SubmissionProfileV1>("\"future_profile\"").is_err());
}
