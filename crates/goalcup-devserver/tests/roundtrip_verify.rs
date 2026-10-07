//! A trace THIS DAEMON wrote, read by the real verifier.
//!
//! Everything else is machinery. The claim is that the daemon writes what the verifier
//! reads, and until this passes that claim is prose.
//!
//! **Why it lives in the devserver crate.** D3 forbids `goalcup-verifier` as a dependency
//! of `goalcup-daemon` — the daemon must not link the judge — and this crate is the one
//! place allowed to hold the verifier, in its dev list, where nothing it links reaches the
//! daemon binary. `goalcup-daemon` joins that list for the same reason and with the same
//! confinement: a dev-dependency of a crate that ships nothing into the daemon.
//!
//! **Why it passes only under `--dev`.** The devserver signs with the placeholder seed,
//! which `goalcup_verifier::SERVER_PUBLIC_KEYS` pins, so the run resolves
//! `KeySource::Pinned` and the only thing V13 has left to say about provenance is
//! `placeholder_server_key` — the Flag that exists to say "this signature stood against a
//! key written in this repository". `dev_mode` retains that finding at Pass so the verdict
//! remains self-describing without putting it in the review queue; the second assertion
//! below pins the strict Flag reading, because that Flag is the release gate rather than a
//! defect. Replacing the constant with the organiser's key makes this test fail loudly
//! instead of passing quietly on a key nobody holds any more.
#![cfg(feature = "v1-compat")]

mod support;

use goalcup_verifier::checks::Severity;
use goalcup_verifier::{
    VerifyOptions, verify_trace_document_with_options, verify_trace_with_options,
};

/// What an honest run with an empty leg 2 is reviewed for, in queue order: V2's
/// unwitnessed `keep going`, V3's two leg-1 messages with no leg-2 post, and V8's leg-1
/// call with no leg-2 post.
const TRANSCRIPT_ONLY: &[(&str, Severity)] = &[
    ("unwitnessed_keep_going", Severity::Flag),
    ("leg1_without_leg2_post", Severity::Flag),
    ("leg1_without_leg2_post", Severity::Flag),
    ("leg1_call_without_leg2_post", Severity::Fail),
];

/// The checks that pair leg 1 against leg 2, and so cannot pass on a transcript-only run.
const LEG2_CHECKS: &[&str] = &["V2", "V3", "V8"];

fn review(v: &goalcup_verifier::Verdict) -> Vec<(&str, Severity)> {
    v.review_queue
        .iter()
        .map(|f| (f.code.as_str(), f.severity))
        .collect()
}

#[test]
fn writer_produced_submission_round_trips_through_private_readback() {
    use ed25519_dalek::SigningKey;
    use goalcup_daemon::submission::{
        SubmissionRequestV1, capture_submission, prepare_source_journal, readback_submission,
    };
    use goalcup_daemon::{EntryArtifactRoot, NativeHandleBudgetV1, SourceDir};
    use goalcup_protocol::control::format_z;
    use goalcup_protocol::export::{
        CHAIN_FILE, DESCRIPTOR_FILE, RECEIPT_FILE, TraceDescriptorDoc, TraceExport,
    };
    use goalcup_protocol::manifest::{ConnectorManifest, InstallScope};
    use goalcup_protocol::provenance::{
        CutoffReachedV1, FinalizationV1, PreCutoffDrainOutcomeV1, Rfc3339Utc, TerminationMethodV1,
        TerminationOutcomeV1,
    };
    use goalcup_protocol::receipt::{StartReceipt, StartReceiptClaims};
    use goalcup_protocol::seal::SealManifest;
    use goalcup_protocol::submission::{
        EffectiveSubmissionLimitsV1, FinishReasonV1, LimitProfileV1, SubmissionOutcomeV1,
    };

    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let artifacts = temporary.path().join("artifact-root");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&artifacts).unwrap();
    std::fs::write(workspace.join("empty"), b"").unwrap();
    std::fs::write(workspace.join("product.txt"), b"retained bytes").unwrap();

    let budget = NativeHandleBudgetV1::new();
    let source = SourceDir::open_selected(&workspace, &budget).unwrap();
    let artifact = EntryArtifactRoot::create(&artifacts, "entry-1").unwrap();
    let trace = artifact.create_trace().unwrap();
    let journal = prepare_source_journal(&source, &artifact).unwrap();
    let trace_path = artifacts.join("artifacts/entry-1/trace");
    let manifest_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors/claude-code-cli/manifest.toml");
    let manifest_bytes = std::fs::read(&manifest_path).unwrap();
    let connector =
        ConnectorManifest::from_toml_str(std::str::from_utf8(&manifest_bytes).unwrap()).unwrap();
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

    let mut export = TraceExport::create(&trace_path).unwrap();
    export.write_manifest(&manifest_bytes).unwrap();
    let workspace_text = workspace.to_string_lossy().to_string();
    let cutoff_second = goalcup_daemon::now_unix().floor() as u64;
    let started_at = format_z(cutoff_second.saturating_sub(1));
    let cutoff_at = format_z(cutoff_second);
    let deadline_at = format_z(cutoff_second.saturating_add(3_600));
    let baseline = SealManifest {
        entry_id: "entry-1".to_string(),
        sealed_at: started_at.clone(),
        workspace: workspace_text.clone(),
        files: Vec::new(),
        excluded: excluded.clone(),
    };
    let genesis = export.write_baseline(&baseline).unwrap().1;
    let signing_key = SigningKey::from_bytes(&[5u8; 32]);
    let receipt = StartReceipt::sign(
        StartReceiptClaims {
            entry_id: "entry-1".to_string(),
            connector_id: connector.id.clone(),
            workspace: workspace_text.clone(),
            session_id: "roundtrip-session".to_string(),
            started_at,
            deadline_at: deadline_at.clone(),
            manifest_sha256: goalcup_daemon::workspace::sha256_hex(&manifest_bytes),
            chain_head: genesis.hash,
            connector_dir_sha256: Some(
                goalcup_protocol::connector_dir::connector_dir_sha256(
                    manifest_path.parent().unwrap(),
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
        &signing_key,
    )
    .unwrap();
    export.write_receipt(&receipt).unwrap();
    let receipt_sha256 = receipt.canonical_digest().unwrap();
    let cutoff = export
        .write_cutoff(
            "entry-1",
            &cutoff_at,
            CutoffReachedV1 {
                receipt_sha256: receipt_sha256.clone(),
                deadline_at: Rfc3339Utc::try_from(deadline_at.as_str()).unwrap(),
                pre_cutoff_drain: PreCutoffDrainOutcomeV1::Complete,
            },
        )
        .unwrap();
    let request = SubmissionRequestV1 {
        generation: 1,
        entry_id: "entry-1".to_string(),
        receipt_sha256: receipt_sha256.clone(),
        cutoff,
        reason: FinishReasonV1::Manual,
        excluded: excluded.clone(),
        limit_profile: LimitProfileV1 {
            id: "roundtrip-v1".to_string(),
        },
        limits: EffectiveSubmissionLimitsV1 {
            object_count: 8,
            total_bytes: 1024,
            pre_cutoff_drain: 1,
            process_walk: 1,
        },
        capture_limits: goalcup_daemon::CaptureResourceLimitsV1 {
            version: 1,
            namespace_nodes: 64,
            inventory_bytes: 16 * 1024,
            manifest_bytes: 16 * 1024,
        },
    };
    let captured = capture_submission(&source, &trace, &journal, &request).unwrap();
    let retained = readback_submission(&trace, &captured).unwrap();
    let finalization_at = goalcup_daemon::now_rfc3339();
    export
        .write_finalization(
            "entry-1",
            &finalization_at,
            FinalizationV1::WorkspaceSubmission {
                receipt_sha256,
                termination: TerminationOutcomeV1::Confirmed {
                    requested_at: Rfc3339Utc::try_from(cutoff_at.as_str()).unwrap(),
                    confirmed_at: Rfc3339Utc::try_from(cutoff_at.as_str()).unwrap(),
                    method: TerminationMethodV1::ProcessSignal,
                },
                submission: SubmissionOutcomeV1::Captured(captured.clone()),
            },
        )
        .unwrap();
    export
        .write_descriptor(&TraceDescriptorDoc {
            connector: connector.id,
            entry_id: "entry-1".to_string(),
            workspace: workspace_text,
            tracked_sessions: vec!["roundtrip-session".to_string()],
            harness_version: "2.1.259".to_string(),
            connector_manifest: "manifest.toml".to_string(),
            chain: Some(CHAIN_FILE.to_string()),
            receipt: Some(RECEIPT_FILE.to_string()),
            baseline: Some(goalcup_protocol::export::BASELINE_FILE.to_string()),
            acks: None,
            seal: None,
            server_public_key: Some(hex::encode(signing_key.verifying_key().to_bytes())),
            expected_reread: false,
        })
        .unwrap();

    assert_eq!(retained.digest().unwrap(), captured.manifest_sha256);
    assert_eq!(
        retained
            .entries
            .iter()
            .filter(|e| e.path.as_str() == "empty")
            .count(),
        1
    );

    assert!(trace_path.join(DESCRIPTOR_FILE).is_file());
    let verdict = verify_trace_with_options(&trace_path, dev(true)).unwrap();
    let v13 = verdict
        .checks
        .iter()
        .find(|check| check.id == "V13")
        .unwrap();
    assert!(
        v13.findings
            .iter()
            .any(|finding| finding.code == "binding_checked"),
        "the independent verifier accepted the actual writer's V4 binding: {:?}",
        v13.findings
    );
    assert!(
        !v13.findings
            .iter()
            .any(|finding| finding.code.starts_with("submission_")
                && finding.severity == Severity::Fail),
        "the independent verifier rejected the writer's V4 artifact: {:?}",
        v13.findings
    );
    assert!(
        verdict
            .seal_attribution
            .iter()
            .any(|entry| entry.path == "product.txt")
    );
    let development_json = verdict.to_json().unwrap();
    let (_, offline_json) = verify_trace_document_with_options(&trace_path, dev(true)).unwrap();
    assert_eq!(
        offline_json.as_bytes(),
        development_json.as_bytes(),
        "the shipped offline document must be byte-identical to the development roundtrip"
    );
}

/// The devserver's default key is the pinned placeholder, so nothing is supplied and
/// nothing is read out of the descriptor. `dev_mode` is the only dial this trace needs.
fn dev(dev_mode: bool) -> VerifyOptions {
    VerifyOptions {
        allow_descriptor_key: false,
        extra_server_keys: Vec::new(),
        dev_mode,
    }
}

#[tokio::test]
async fn a_trace_this_daemon_produced_fails_only_on_its_missing_leg_2_under_dev() {
    // Workspace, config, arm, goal prompt + hook post, `keep going` + post, one Write call,
    // receipt, heartbeats, hourly acknowledgements, seal. All through the daemon's own code.
    let run = support::compliant_run().await;
    let v = verify_trace_with_options(&run.trace_dir, dev(true)).unwrap();

    // The devserver signs with the placeholder seed, so the key resolves `Pinned`; V13
    // retains `placeholder_server_key` at Pass under `dev_mode` (see the module header).
    assert_eq!(v.key_source, goalcup_verifier::KeySource::Pinned);
    assert_eq!(
        review(&v),
        TRANSCRIPT_ONLY.to_vec(),
        "a compliant transcript-only run enters review for its missing leg 2 and nothing else"
    );
    assert_eq!(v.overall, Severity::Fail);
    assert_eq!(v.exit_code(), 2);
    assert_eq!(v.checks.len(), 13);
    for c in &v.checks {
        if !LEG2_CHECKS.contains(&c.id.as_str()) {
            assert_eq!(c.verdict, Severity::Pass, "{}: {:?}", c.id, c.findings);
        }
    }
    let placeholder = v
        .checks
        .iter()
        .flat_map(|c| c.findings.iter())
        .find(|f| f.code == "placeholder_server_key")
        .expect("development verdict retains placeholder_server_key");
    assert_eq!(placeholder.severity, Severity::Pass);

    // Strictly, the identical bytes carry one more finding, and it is about the signer
    // rather than about the run. Asserted rather than assumed: this Flag is what the release
    // gate closes.
    let strictly = verify_trace_with_options(&run.trace_dir, dev(false)).unwrap();
    let mut expected = TRANSCRIPT_ONLY.to_vec();
    expected.push(("placeholder_server_key", Severity::Flag));
    expected.sort();
    let mut got = review(&strictly);
    got.sort();
    assert_eq!(got, expected, "{:?}", strictly.review_queue);
    assert_eq!(strictly.overall, Severity::Fail);
    assert_eq!(strictly.exit_code(), 2);

    // And the unpinned path, once, end to end: a devserver started under some other seed is
    // `Supplied` however carefully the key is handed in, and `dev_mode` does not touch
    // `server_key_unpinned`. That is the shape a real anchor replaces, not this one.
    let other = support::compliant_run_signed_by(&[7u8; 32]).await;
    let u = verify_trace_with_options(
        &other.trace_dir,
        VerifyOptions {
            allow_descriptor_key: false,
            extra_server_keys: vec![other.server_public_key.clone()],
            dev_mode: true,
        },
    )
    .unwrap();
    assert_eq!(u.key_source, goalcup_verifier::KeySource::Supplied);
    assert!(
        u.review_queue
            .iter()
            .any(|f| f.code == "server_key_unpinned")
    );

    // The statements this whole test exists to make true, each read back off the disk by the
    // verifier rather than asserted by the daemon about itself. The codes are the ones that
    // EXIST: V13's summary is `binding_checked`, V12's is `anchor_timeline`, and the chain's
    // integrity is reported by the ABSENCE of `chain_broken` — there is no `chain_verified`
    // and no `binding_verified` anywhere in the verifier.
    let said: std::collections::BTreeSet<&str> = v
        .checks
        .iter()
        .flat_map(|c| c.findings.iter())
        .map(|f| f.code.as_str())
        .collect();
    for code in ["binding_checked", "anchor_timeline"] {
        assert!(said.contains(code), "{code} missing: {said:?}");
    }

    // The arming phase, on a run this daemon really produced. The connector's arming signal
    // is the harness's own first output, so the records that CAUSE the receipt to be
    // requested are chained before it is issued and are dated below `started_at`. This
    // fixture does not hide that by opening the server's clock under the run, so the
    // pre-start records are here, they are said out loud, and they are said at Pass,
    // measured on a real capture rather than on a hand-built one.
    assert!(
        said.contains("pre_start_records"),
        "the margin is gone, so an honest run's arming records must be reported: {said:?}"
    );
    let arming = v
        .checks
        .iter()
        .flat_map(|c| c.findings.iter())
        .find(|f| f.code == "pre_start_records")
        .expect("pre_start_records");
    assert_eq!(
        arming.severity,
        Severity::Pass,
        "a run cannot reach its own start receipt without writing these"
    );
    for code in [
        "chain_broken",
        "protocol_version_unsupported",
        "receipt_missing",
        "receipt_invalid",
        "receipt_unverified",
        "descriptor_mismatch",
        "chain_head_mismatch",
        "outside_window",
        "window_unmeasurable",
        "window_unplaceable",
        "baseline_missing",
        "baseline_not_chained",
        "manifest_not_pinned",
        "seal_not_chained",
        "truncated_tail",
        "ack_signature_invalid",
        "anchor_unverified",
        "no_anchor",
        "late_first_anchor",
        "long_unanchored_span",
        "cursor_mismatch",
        "unexplained_seal_file",
    ] {
        assert!(!said.contains(code), "{code} fired on a compliant run");
    }
    // The goal and one `keep going` have no leg-2 witness, so V1 holds both as unwitnessed.
    assert_eq!(
        v.human_messages.len(),
        0,
        "the goal and one `keep going` are unwitnessed"
    );
}

#[tokio::test]
async fn the_same_trace_fails_the_way_it_should_when_one_line_is_edited() {
    // The other half of the claim: a clean run is only meaningful if a tampered trace fails.
    let run = support::compliant_run().await;
    support::rewrite_one_chain_line(&run.trace_dir, 1);
    let v = verify_trace_with_options(&run.trace_dir, dev(true)).unwrap();
    assert_eq!(v.overall, Severity::Fail);
    assert_eq!(v.exit_code(), 2);
    assert!(v.review_queue.iter().any(|f| f.code == "chain_broken"));
}

#[tokio::test]
async fn the_baseline_is_the_link_the_receipt_covers_and_moving_it_is_visible() {
    // The point of chaining the baseline, end to end: `baseline.json` alone is a file the
    // participant's own daemon wrote, so a participant who pre-populated the workspace could
    // list every planted file in it and have attribution exempt each one. Chained at seq 0,
    // rewriting it moves the head the server signed.
    let run = support::compliant_run().await;
    let chain = support::read_chain(&run.trace_dir);
    assert!(matches!(
        chain[0].body,
        goalcup_protocol::envelope::EventBody::Baseline { .. }
    ));
    assert!(
        matches!(
            chain.last().unwrap().body,
            goalcup_protocol::envelope::EventBody::Seal { .. }
        ),
        "and the seal is the last link, so an acknowledgement over the head anchors it"
    );

    support::rewrite_baseline_json(&run.trace_dir); // add a planted file
    let v = verify_trace_with_options(&run.trace_dir, dev(true)).unwrap();
    assert_eq!(v.overall, Severity::Fail);
    assert!(
        v.review_queue
            .iter()
            .any(|f| f.code == "baseline_not_chained"),
        "the chained digest no longer names these bytes: {:?}",
        v.review_queue
    );
}
