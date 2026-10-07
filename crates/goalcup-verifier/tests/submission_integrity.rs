//! Public-pipeline acceptance for the independent V4 artifact reader.

#![cfg(unix)]

use ed25519_dalek::SigningKey;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::export::{
    BASELINE_FILE, CHAIN_FILE, RECEIPT_FILE, TraceDescriptorDoc, TraceExport,
};
use goalcup_protocol::manifest::{ConnectorManifest, InstallScope};
use goalcup_protocol::provenance::{
    CutoffReachedV1, FinalizationV1, PreCutoffDrainOutcomeV1, Rfc3339Utc, TerminationMethodV1,
    TerminationOutcomeV1,
};
use goalcup_protocol::receipt::{StartReceipt, StartReceiptClaims};
use goalcup_protocol::seal::SealManifest;
use goalcup_protocol::submission::{
    EffectiveSubmissionLimitsV1, FinishReasonV1, LimitProfileV1, SubmissionCapturedV1,
    SubmissionEntryV1, SubmissionKindV1, SubmissionManifestV1, SubmissionOutcomeV1,
    SubmissionProfileV1,
};
use goalcup_verifier::checks::Severity;
use sha2::{Digest, Sha256};
use std::os::unix::fs::{PermissionsExt, symlink};

const OBJECT_SHA256: &str = "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881";

fn private_file(path: &std::path::Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

fn public_v4_trace() -> tempfile::TempDir {
    let trace = tempfile::tempdir().unwrap();
    let manifest_source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors/claude-code-cli/manifest.toml");
    let connector_bytes = std::fs::read(&manifest_source).unwrap();
    let connector =
        ConnectorManifest::from_toml_str(std::str::from_utf8(&connector_bytes).unwrap()).unwrap();
    let mut excluded = connector.workspace_exclusions.clone();
    excluded.extend(
        connector
            .hook_install
            .iter()
            .filter(|hook| hook.scope == InstallScope::Workspace)
            .map(|hook| hook.path.clone()),
    );
    excluded.sort();
    excluded.dedup();

    let mut export = TraceExport::create(trace.path()).unwrap();
    export.write_manifest(&connector_bytes).unwrap();
    let baseline = SealManifest {
        entry_id: "entry-1".to_string(),
        sealed_at: "2026-09-14T00:00:00Z".to_string(),
        workspace: "/private/workspace".to_string(),
        files: Vec::new(),
        excluded: excluded.clone(),
    };
    let genesis = export.write_baseline(&baseline).unwrap().1;
    let signer = SigningKey::from_bytes(&[5u8; 32]);
    let receipt = StartReceipt::sign(
        StartReceiptClaims {
            entry_id: "entry-1".to_string(),
            connector_id: connector.id.clone(),
            workspace: "/private/workspace".to_string(),
            session_id: "session-1".to_string(),
            started_at: "2026-09-14T00:00:00Z".to_string(),
            deadline_at: "2026-09-14T01:00:00Z".to_string(),
            manifest_sha256: hex::encode(Sha256::digest(&connector_bytes)),
            chain_head: genesis.hash,
            connector_dir_sha256: Some(
                goalcup_protocol::connector_dir::connector_dir_sha256(
                    manifest_source.parent().unwrap(),
                )
                .unwrap(),
            ),
            run_public_key: Some(hex::encode(
                SigningKey::from_bytes(&[9u8; 32])
                    .verifying_key()
                    .to_bytes(),
            )),
            questionnaire_version: Some("2026.09-v1".to_string()),
        },
        &signer,
    )
    .unwrap();
    export.write_receipt(&receipt).unwrap();
    let receipt_sha256 = receipt.canonical_digest().unwrap();
    let cutoff = export
        .write_cutoff(
            "entry-1",
            "2026-09-14T00:30:00Z",
            CutoffReachedV1 {
                receipt_sha256: receipt_sha256.clone(),
                deadline_at: Rfc3339Utc::try_from("2026-09-14T01:00:00Z").unwrap(),
                pre_cutoff_drain: PreCutoffDrainOutcomeV1::Complete,
            },
        )
        .unwrap();

    let object_digest =
        goalcup_protocol::provenance::Sha256Digest::try_from(OBJECT_SHA256).unwrap();
    let retained = SubmissionManifestV1 {
        profile: SubmissionProfileV1::WorkspaceSnapshotV1,
        limit_profile: LimitProfileV1 {
            id: "public-test-v1".to_string(),
        },
        effective_limits: EffectiveSubmissionLimitsV1 {
            object_count: 2,
            total_bytes: 16,
            pre_cutoff_drain: 1,
            process_walk: 1,
        },
        entry_id: "entry-1".to_string(),
        receipt_sha256: receipt_sha256.clone(),
        cutoff_seq: cutoff.seq,
        cutoff_chain_hash: goalcup_protocol::provenance::Sha256Digest::try_from(
            cutoff.hash.as_str(),
        )
        .unwrap(),
        finish_reason: FinishReasonV1::Manual,
        entries: vec![SubmissionEntryV1 {
            path: goalcup_protocol::provenance::PortableRelativePath::try_from("empty").unwrap(),
            kind: SubmissionKindV1::Regular,
            posix_owner_executable: false,
            bytes: 1,
            content_sha256: object_digest.clone(),
        }],
        excluded,
        copy_started_at: Rfc3339Utc::try_from("2026-09-14T00:30:03Z").unwrap(),
        copy_completed_at: Rfc3339Utc::try_from("2026-09-14T00:30:04Z").unwrap(),
        readback_verified_at: Rfc3339Utc::try_from("2026-09-14T00:30:05Z").unwrap(),
        object_count: 1,
        total_bytes: 1,
    };
    let object_dir = trace.path().join("submission/objects/sha256");
    std::fs::create_dir_all(&object_dir).unwrap();
    let retained_bytes = serde_json::to_vec(&retained).unwrap();
    private_file(
        &trace.path().join("submission/manifest.json"),
        &retained_bytes,
    );
    private_file(&object_dir.join(object_digest.as_str()), b"x");
    let captured = SubmissionCapturedV1 {
        manifest_sha256: goalcup_protocol::provenance::Sha256Digest::from_bytes(
            Sha256::digest(&retained_bytes).into(),
        ),
    };
    export
        .write_finalization(
            "entry-1",
            "2026-09-14T00:30:06Z",
            FinalizationV1::WorkspaceSubmission {
                receipt_sha256,
                termination: TerminationOutcomeV1::Confirmed {
                    requested_at: Rfc3339Utc::try_from("2026-09-14T00:30:01Z").unwrap(),
                    confirmed_at: Rfc3339Utc::try_from("2026-09-14T00:30:02Z").unwrap(),
                    method: TerminationMethodV1::ProcessSignal,
                },
                submission: SubmissionOutcomeV1::Captured(captured),
            },
        )
        .unwrap();
    export
        .write_descriptor(&TraceDescriptorDoc {
            connector: connector.id,
            entry_id: "entry-1".to_string(),
            workspace: "/private/workspace".to_string(),
            tracked_sessions: vec!["session-1".to_string()],
            harness_version: "public-test".to_string(),
            connector_manifest: "manifest.toml".to_string(),
            chain: Some(CHAIN_FILE.to_string()),
            receipt: Some(RECEIPT_FILE.to_string()),
            baseline: Some(BASELINE_FILE.to_string()),
            acks: None,
            seal: None,
            server_public_key: Some(hex::encode(signer.verifying_key().to_bytes())),
            expected_reread: false,
        })
        .unwrap();
    trace
}

fn replace_manifest_bytes_and_rebind(trace: &tempfile::TempDir, bytes: Vec<u8>) {
    private_file(&trace.path().join("submission/manifest.json"), &bytes);
    let digest =
        goalcup_protocol::provenance::Sha256Digest::from_bytes(Sha256::digest(&bytes).into());
    let chain_path = trace.path().join(CHAIN_FILE);
    let chain = std::fs::read_to_string(&chain_path).unwrap();
    let mut envelopes = chain
        .lines()
        .map(|line| serde_json::from_str::<Envelope>(line).unwrap())
        .collect::<Vec<_>>();
    let terminal = envelopes.last_mut().unwrap();
    let EventBody::FinalizationV1(FinalizationV1::WorkspaceSubmission {
        submission: SubmissionOutcomeV1::Captured(captured),
        ..
    }) = &mut terminal.body
    else {
        panic!("public fixture ends in a captured workspace submission")
    };
    captured.manifest_sha256 = digest;
    terminal.link.hash = terminal.expected_hash(&terminal.link.prev).unwrap();
    let mut encoded = envelopes
        .iter()
        .map(|envelope| serde_json::to_string(envelope).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    encoded.push('\n');
    std::fs::write(chain_path, encoded).unwrap();
}

fn assert_public_reader_rejects(trace: &tempfile::TempDir) {
    let verdict = goalcup_verifier::verify_trace(trace.path()).unwrap();
    let v13 = verdict
        .checks
        .iter()
        .find(|check| check.id == "V13")
        .unwrap();
    assert!(v13.findings.iter().any(|finding| {
        finding.code == "submission_invalid" && finding.severity == Severity::Fail
    }));
    assert!(!v13.findings.iter().any(|f| f.code == "binding_checked"));
}

#[test]
fn public_pipeline_accepts_the_exact_canonical_layout() {
    let trace = public_v4_trace();
    let verdict = goalcup_verifier::verify_trace(trace.path()).unwrap();
    let v13 = verdict
        .checks
        .iter()
        .find(|check| check.id == "V13")
        .unwrap();
    assert!(!v13.findings.iter().any(|finding| {
        finding.code.starts_with("submission_") && finding.severity == Severity::Fail
    }));
    assert!(v13.findings.iter().any(|f| f.code == "binding_checked"));
}

#[test]
fn public_pipeline_rejects_extra_entries_at_every_fixed_layout_level() {
    for relative in [
        "submission/hidden",
        "submission/objects/other-algorithm",
        "submission/objects/sha256/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        let trace = public_v4_trace();
        let path = trace.path().join(relative);
        if relative.contains("other-algorithm") {
            std::fs::create_dir(path).unwrap();
        } else {
            private_file(&path, b"extra");
        }
        assert_public_reader_rejects(&trace);
    }
}

#[test]
fn public_pipeline_rejects_missing_corrupt_mode_and_hardlink_objects() {
    let object = |trace: &tempfile::TempDir| {
        trace
            .path()
            .join("submission/objects/sha256")
            .join(OBJECT_SHA256)
    };
    let missing = public_v4_trace();
    std::fs::remove_file(object(&missing)).unwrap();
    assert_public_reader_rejects(&missing);
    let corrupt = public_v4_trace();
    private_file(&object(&corrupt), b"y");
    assert_public_reader_rejects(&corrupt);
    let mode = public_v4_trace();
    std::fs::set_permissions(object(&mode), std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_public_reader_rejects(&mode);
    let linked = public_v4_trace();
    std::fs::hard_link(object(&linked), linked.path().join("outside-link")).unwrap();
    assert_public_reader_rejects(&linked);
}

#[test]
fn public_pipeline_rejects_noncanonical_manifest_and_replaced_ancestor() {
    let noncanonical = public_v4_trace();
    let manifest = noncanonical.path().join("submission/manifest.json");
    let mut bytes = std::fs::read(&manifest).unwrap();
    bytes.push(b'\n');
    replace_manifest_bytes_and_rebind(&noncanonical, bytes);
    assert_public_reader_rejects(&noncanonical);
    let replaced = public_v4_trace();
    let submission = replaced.path().join("submission");
    let moved = replaced.path().join("moved-submission");
    std::fs::rename(&submission, &moved).unwrap();
    symlink(&moved, &submission).unwrap();
    assert_public_reader_rejects(&replaced);
}

#[test]
fn public_pipeline_rejects_a_manifest_rebound_to_the_wrong_cutoff() {
    let trace = public_v4_trace();
    let mut manifest: SubmissionManifestV1 = serde_json::from_slice(
        &std::fs::read(trace.path().join("submission/manifest.json")).unwrap(),
    )
    .unwrap();
    manifest.cutoff_chain_hash =
        goalcup_protocol::provenance::Sha256Digest::try_from("9".repeat(64)).unwrap();
    replace_manifest_bytes_and_rebind(&trace, manifest.canonical_bytes().unwrap());

    let verdict = goalcup_verifier::verify_trace(trace.path()).unwrap();
    let v13 = verdict
        .checks
        .iter()
        .find(|check| check.id == "V13")
        .unwrap();
    assert!(
        v13.findings.iter().any(|finding| {
            finding.code == "submission_binding_mismatch" && finding.severity == Severity::Fail
        }),
        "{:?}",
        v13.findings
    );
    assert!(!v13.findings.iter().any(|f| f.code == "binding_checked"));
}

#[test]
fn public_pipeline_rejects_declared_count_and_size_bound_violations() {
    for (from, to) in [
        ("\"object_count\":2", "\"object_count\":0"),
        ("\"total_bytes\":16", "\"total_bytes\":0"),
    ] {
        let trace = public_v4_trace();
        let manifest = std::fs::read_to_string(trace.path().join("submission/manifest.json"))
            .unwrap()
            .replacen(from, to, 1);
        replace_manifest_bytes_and_rebind(&trace, manifest.into_bytes());
        assert_public_reader_rejects(&trace);
    }
}

#[test]
fn public_pipeline_rejects_the_manifest_entry_and_file_policy_matrix() {
    #[derive(Clone, Copy, Debug)]
    enum Case {
        EntryLength,
        EntryPath,
        EntryKind,
        EntryExecutable,
        DuplicatePath,
        UnknownProfile,
        CorruptJson,
        ManifestMode,
        ManifestHardlink,
    }

    for case in [
        Case::EntryLength,
        Case::EntryPath,
        Case::EntryKind,
        Case::EntryExecutable,
        Case::DuplicatePath,
        Case::UnknownProfile,
        Case::CorruptJson,
        Case::ManifestMode,
        Case::ManifestHardlink,
    ] {
        let trace = public_v4_trace();
        let manifest_path = trace.path().join("submission/manifest.json");
        match case {
            Case::EntryLength => {
                let text = std::fs::read_to_string(&manifest_path)
                    .unwrap()
                    .replacen("\"bytes\":1", "\"bytes\":2", 1)
                    .replacen("\"total_bytes\":1}", "\"total_bytes\":2}", 1);
                replace_manifest_bytes_and_rebind(&trace, text.into_bytes());
            }
            Case::EntryPath => {
                let text = std::fs::read_to_string(&manifest_path).unwrap().replacen(
                    "\"path\":\"empty\"",
                    "\"path\":\"../escape\"",
                    1,
                );
                replace_manifest_bytes_and_rebind(&trace, text.into_bytes());
            }
            Case::EntryKind => {
                let text = std::fs::read_to_string(&manifest_path).unwrap().replacen(
                    "\"kind\":\"regular\"",
                    "\"kind\":\"directory\"",
                    1,
                );
                replace_manifest_bytes_and_rebind(&trace, text.into_bytes());
            }
            Case::EntryExecutable => {
                let text = std::fs::read_to_string(&manifest_path).unwrap().replacen(
                    "\"posix_owner_executable\":false",
                    "\"posix_owner_executable\":null",
                    1,
                );
                replace_manifest_bytes_and_rebind(&trace, text.into_bytes());
            }
            Case::DuplicatePath => {
                let mut manifest: SubmissionManifestV1 =
                    serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
                manifest.entries.push(manifest.entries[0].clone());
                manifest.total_bytes = 2;
                replace_manifest_bytes_and_rebind(&trace, serde_json::to_vec(&manifest).unwrap());
            }
            Case::UnknownProfile => {
                let text = std::fs::read_to_string(&manifest_path).unwrap().replacen(
                    "\"profile\":\"workspace_snapshot_v1\"",
                    "\"profile\":\"workspace_snapshot_v2\"",
                    1,
                );
                replace_manifest_bytes_and_rebind(&trace, text.into_bytes());
            }
            Case::CorruptJson => {
                let mut bytes = std::fs::read(&manifest_path).unwrap();
                bytes.pop();
                replace_manifest_bytes_and_rebind(&trace, bytes);
            }
            Case::ManifestMode => {
                std::fs::set_permissions(&manifest_path, std::fs::Permissions::from_mode(0o644))
                    .unwrap();
            }
            Case::ManifestHardlink => {
                std::fs::hard_link(&manifest_path, trace.path().join("manifest-alias")).unwrap();
            }
        }

        let verdict = goalcup_verifier::verify_trace(trace.path()).unwrap();
        let v13 = verdict
            .checks
            .iter()
            .find(|check| check.id == "V13")
            .unwrap();
        assert!(
            v13.findings.iter().any(|finding| {
                finding.code == "submission_invalid" && finding.severity == Severity::Fail
            }),
            "{case:?}: {:?}",
            v13.findings
        );
        assert!(
            !v13.findings.iter().any(|f| f.code == "binding_checked"),
            "{case:?}: {:?}",
            v13.findings
        );
    }
}
