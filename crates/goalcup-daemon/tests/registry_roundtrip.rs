//! The entry registry: what an entry holds, and that it still holds it after a restart.
//!
//! An entry carries "workspace path, harness, manifest hash, session
//! binding, signed receipts, per-run keys, health". These tests pin the receipt, alongside
//! the two lookups D4 and D24 need (by
//! workspace, by token) and the removal a failed start needs.

use goalcup_daemon::config::ReceiptBindingPolicy;
use goalcup_daemon::registry::{
    Entry, EntryState, ExpectedProcessHandoff, ProcessHandoffPhase, Registry, RunKeys,
    SealingIntent, WorkspaceSealingIntentV1,
};
use goalcup_protocol::events::{HomeSeedRecord, SeededFile, SeededKey};
use goalcup_protocol::export::{
    ACKS_FILE, BASELINE_FILE, CHAIN_FILE, MANIFEST_FILE, SEAL_FILE, TraceDescriptorDoc,
};
use goalcup_protocol::manifest::KillStrategy;
use goalcup_protocol::process::{ProcessHandle, ProcessRegistration};
use goalcup_protocol::receipt::{StartReceipt, StartReceiptClaims};
use goalcup_protocol::seal::SealManifest;

fn entry(tmp: &std::path::Path, id: &str) -> Entry {
    Entry::new(
        id,
        "claude-code-cli",
        tmp.join("ws").join(id),
        tmp.join("art").join(id),
        &"ab".repeat(32),
        "2.1.259",
        goalcup_daemon::config::ReceiptBindingPolicy::Legacy,
        ExpectedProcessHandoff::NotRequired,
        RunKeys::from_seeds([1u8; 32], [2u8; 32], [3u8; 16]),
    )
}

fn sealing_intent(entry: &Entry) -> SealingIntent {
    let manifest = SealManifest {
        entry_id: entry.entry_id.clone(),
        sealed_at: "2026-09-11T00:00:00Z".to_string(),
        workspace: entry.workspace.display().to_string(),
        files: Vec::new(),
        excluded: Vec::new(),
    };
    SealingIntent::new(
        manifest,
        TraceDescriptorDoc {
            connector: entry.connector_id.clone(),
            entry_id: entry.entry_id.clone(),
            workspace: entry.workspace.display().to_string(),
            tracked_sessions: Vec::new(),
            harness_version: entry.harness_version.clone(),
            connector_manifest: MANIFEST_FILE.to_string(),
            chain: Some(CHAIN_FILE.to_string()),
            receipt: None,
            baseline: Some(BASELINE_FILE.to_string()),
            acks: Some(ACKS_FILE.to_string()),
            seal: Some(SEAL_FILE.to_string()),
            server_public_key: None,
            expected_reread: false,
        },
        None,
        "registry-roundtrip-root".to_string(),
    )
    .unwrap()
}

/// Workspace finalization adds a new discriminated arm beside this document.  It must not
/// add a wrapper or discriminator to the historical registry bytes: old `Sealing` rows are
/// recovery authority, and changing their shape would make the daemon lose the exact
/// terminal predecessor it already promised to reconcile.
#[test]
fn legacy_sealing_intent_bytes_stay_bare_when_the_workspace_arm_is_added() {
    let tmp = tempfile::tempdir().unwrap();
    let row = entry(tmp.path(), "legacy-intent");
    let intent = sealing_intent(&row);

    let wire = serde_json::to_value(&intent).expect("the legacy intent serializes");
    let object = wire
        .as_object()
        .expect("the legacy intent stays a bare object");
    assert_eq!(
        object
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        [
            "descriptor",
            "manifest",
            "pre_seal_head",
            "pre_seal_seq",
            "terminal_hash",
            "terminal_prev",
            "terminal_seq",
            "trace_root_identity",
        ]
        .into_iter()
        .map(str::to_string)
        .collect(),
        "the untagged legacy arm adds no version key or enum wrapper",
    );
    let decoded: SealingIntent =
        serde_json::from_value(wire.clone()).expect("the historical bytes still decode");
    assert_eq!(
        serde_json::to_value(decoded).unwrap(),
        wire,
        "reading and rewriting an old registry row is byte-shape stable",
    );
}

#[test]
fn workspace_sealing_intent_is_discriminated_exact_and_roundtrips_without_inference() {
    let wire = serde_json::json!({
        "version": "workspace_submission_v1",
        "entry_id": "workspace-intent",
        "generation": 9,
        "reason": "deadline",
        "receipt_sha256": "ab".repeat(32),
        "cutoff_at": "2026-09-14T00:00:02Z",
        "trace_root_identity": "trace-root-v1:4:2",
        "source_root": {
            "availability": "available",
            "journal": { "identity": "source-root-v1:17:9" }
        },
        "source_date": ["2026", "09", "14"],
        "submission_profile": "workspace_snapshot_v1",
        "limits": {
            "object_count": 4096,
            "total_bytes": 1073741824,
            "pre_cutoff_drain": 5000,
            "process_walk": 64
        },
        "limit_profile": { "id": "goal-cup-v1-local" },
        "capture_limits": {
            "version": 1,
            "namespace_nodes": 8192,
            "inventory_bytes": 8388608,
            "manifest_bytes": 8388608
        },
        "excluded": [".goalcup"],
        "phase": "admission_closed",
        "admission_watermarks": {
            "tailers": [],
            "receiver_sources": {},
            "queued_receiver_posts": 0,
            "admitted_chain_predecessor": null
        },
        "expected_cutoff_predecessor": null,
        "drain": null,
        "cutoff": null,
        "accepted_handoff": null,
        "termination": null,
        "termination_report": null,
        "submission": null,
        "finalization_at": null,
        "expected_finalization_predecessor": null,
        "finalization": null,
        "terminal_status_at": null,
        "status_mirror_complete": false
    });

    let decoded: WorkspaceSealingIntentV1 =
        serde_json::from_value(wire.clone()).expect("the exact workspace intent decodes");
    assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
    let decoded: SealingIntent =
        serde_json::from_value(wire.clone()).expect("the discriminated arm wins");
    assert!(matches!(decoded, SealingIntent::Workspace(_)));

    let mut unknown = wire.clone();
    unknown
        .as_object_mut()
        .unwrap()
        .insert("future_phase_hint".to_string(), serde_json::json!(true));
    assert!(serde_json::from_value::<SealingIntent>(unknown).is_err());

    let mut unknown_nested = wire;
    unknown_nested["source_root"]["journal"]["reopen_path"] =
        serde_json::json!("/tmp/not-authority");
    assert!(serde_json::from_value::<SealingIntent>(unknown_nested).is_err());
}

#[test]
fn workspace_source_binding_distinguishes_lost_cutoff_authority_exactly() {
    let wire = serde_json::json!({
        "availability": "unavailable_at_cutoff",
        "reason": "recovery_ambiguous"
    });
    let binding: goalcup_daemon::registry::WorkspaceSourceBindingV1 =
        serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(binding).unwrap(), wire);

    let mut unknown = wire;
    unknown["source_path"] = serde_json::json!("/must/not/reopen");
    assert!(
        serde_json::from_value::<goalcup_daemon::registry::WorkspaceSourceBindingV1>(unknown)
            .is_err()
    );
}

fn bound_entry(tmp: &std::path::Path, id: &str, version: &str) -> Entry {
    bound_entry_with_handoff(tmp, id, version, ExpectedProcessHandoff::NotRequired)
}

fn bound_entry_with_handoff(
    tmp: &std::path::Path,
    id: &str,
    version: &str,
    expected_process_handoff: ExpectedProcessHandoff,
) -> Entry {
    let mut entry = Entry::new(
        id,
        "claude-code-cli",
        tmp.join("ws").join(id),
        tmp.join("art").join(id),
        &"ab".repeat(32),
        "2.1.259",
        ReceiptBindingPolicy::NewEvent {
            questionnaire_version: version.to_string(),
        },
        expected_process_handoff,
        RunKeys::from_seeds([1u8; 32], [2u8; 32], [3u8; 16]),
    );
    entry.connector_dir_sha256 = Some("ef".repeat(32));
    entry
}

/// Registry promotion revalidates durable signal authority, so successful transition
/// fixtures use this test process's real instance identity. These tests never terminate it.
fn recorded_launch_handle() -> ProcessHandle {
    let pid = std::process::id();
    ProcessHandle {
        pid,
        pgid: None,
        native_pid: None,
        native_binary: None,
        native_process_start: None,
        process_start: goalcup_protocol::process::process_start_identity(pid),
        strategy: KillStrategy::NativePid,
        grace_seconds: 10,
        launched_at: "2026-09-01T00:00:00Z".to_string(),
        program: "codex".to_string(),
        argv: vec!["codex".to_string()],
        env_keys: Vec::new(),
        home_seed: None,
    }
}

/// The historical v1 digest fixture, deliberately lacking additive identity fields.
fn legacy_recorded_launch_handle() -> ProcessHandle {
    ProcessHandle {
        pid: 42,
        process_start: None,
        ..recorded_launch_handle()
    }
}

fn registration(generation: u64) -> ProcessRegistration {
    ProcessRegistration {
        generation,
        handle: recorded_launch_handle(),
    }
}

fn register_initial(registry: &Registry, entry_id: &str) -> Entry {
    registry.open_initial_process_handoff(entry_id).unwrap();
    registry
        .prepare_process_handoff(entry_id, registration(1))
        .unwrap();
    registry
        .register_prepared_process_handoff(entry_id, 1)
        .unwrap()
}

/// Additive authority fields must not rewrite the v1 canonical JSON for a durable old row.
/// The golden was computed by the v1 implementation before `native_binary` and
/// `process_start` existed; an exact lost-response replay depends on preserving it.
#[test]
fn legacy_process_registration_keeps_its_v1_digest_and_exact_replay_identity() {
    let legacy_registration = ProcessRegistration {
        generation: 1,
        handle: legacy_recorded_launch_handle(),
    };
    let wire = serde_json::to_value(legacy_registration).expect("the legacy request serializes");
    assert!(wire["handle"].get("native_binary").is_none());
    assert!(wire["handle"].get("process_start").is_none());
    let legacy: ProcessRegistration =
        serde_json::from_value(wire).expect("an old request remains readable");
    assert_eq!(
        legacy.canonical_handle_digest("launch"),
        "b865bedad64eb4d05fb0271a0e3cbd767fcd2341e986df932dbb943161e5d26a"
    );

    let tmp = tempfile::tempdir().unwrap();
    let registry = Registry::open(&tmp.path().join("registry.redb")).unwrap();
    let mut row = bound_entry_with_handoff(
        tmp.path(),
        "launch",
        "2026.09-v1",
        ExpectedProcessHandoff::Required,
    );
    row.state = EntryState::Armed;
    registry.put(&row).unwrap();
    registry.open_initial_process_handoff("launch").unwrap();
    registry
        .prepare_process_handoff("launch", legacy.clone())
        .expect("the old authority prepares under its original digest");
    assert!(
        registry
            .is_exact_accepted_process_replay("launch", &legacy)
            .unwrap(),
        "a lost response can replay the same v1 authority"
    );
    registry
        .register_prepared_process_handoff("launch", 1)
        .expect_err("legacy Prepared numeric authority cannot be promoted without identity");
    assert!(
        registry
            .is_exact_accepted_process_replay("launch", &legacy)
            .unwrap(),
        "the original digest stays replayable while the unverifiable row remains Prepared"
    );
}

/// The process route is not a generic "set the last pid" operation.  Its generation is
/// daemon-issued authority for exactly one armed start or one completed resume.  This stays
/// at the registry layer so a runtime mirror, a retry, or two control connections cannot
/// turn a stale launch into the active seal target.
#[test]
fn process_handoff_requires_an_open_generation_and_exact_replays_are_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let registry = Registry::open(&tmp.path().join("registry.redb")).unwrap();
    let mut row = bound_entry_with_handoff(
        tmp.path(),
        "launch",
        "2026.09-v1",
        ExpectedProcessHandoff::Required,
    );
    registry.put(&row).unwrap();

    registry
        .prepare_process_handoff("launch", registration(1))
        .expect_err("created/pre-arm rows do not accept a process handoff");

    row.state = EntryState::Armed;
    registry.put(&row).unwrap();
    let opened = registry
        .open_initial_process_handoff("launch")
        .expect("only an armed launch opens generation one");
    assert_eq!(opened.process_handoff_generation(), 1);
    assert_eq!(
        opened.process_handoff_phase(),
        ProcessHandoffPhase::OpenInitial
    );

    let prepared = registry
        .prepare_process_handoff("launch", registration(1))
        .expect("the current generation prepares exactly once");
    assert_eq!(
        prepared.process_handoff_phase(),
        ProcessHandoffPhase::Prepared
    );
    assert!(
        prepared.last_process.is_none(),
        "prepared is not yet registered"
    );

    assert_eq!(
        registry
            .prepare_process_handoff("launch", registration(1))
            .expect("an identical retry reuses the prepared request"),
        prepared
    );

    let mut changed = registration(1);
    changed.handle.pid = 43;
    registry
        .prepare_process_handoff("launch", changed)
        .expect_err("a different handle cannot claim the same generation");
    registry
        .prepare_process_handoff("launch", registration(2))
        .expect_err("a future generation is not caller-selectable");

    let registered = registry
        .register_prepared_process_handoff("launch", 1)
        .expect("only the prepared current generation becomes registered");
    assert_eq!(
        registered.process_handoff_phase(),
        ProcessHandoffPhase::Registered
    );
    assert_eq!(
        registered.last_process.as_ref().map(|h| h.pid),
        Some(std::process::id())
    );
    assert_eq!(
        registry
            .prepare_process_handoff("launch", registration(1))
            .expect("the exact accepted retry stays idempotent"),
        registered
    );
}

/// Seal is a two-store transition: the registry records intent before `chain.jsonl` gains
/// its terminal link.  A crash before that link must restore exactly the old open state,
/// never leave an unrehydratable pseudo-sealed row.
#[test]
fn sealing_intent_restores_its_exact_open_predecessor_when_no_terminal_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let registry = Registry::open(&tmp.path().join("registry.redb")).unwrap();
    let mut row = entry(tmp.path(), "sealing");
    row.state = EntryState::Armed;
    registry.put(&row).unwrap();

    let intent = registry
        .begin_sealing("sealing", sealing_intent(&row))
        .unwrap();
    assert_eq!(intent.state, EntryState::Sealing);
    assert_eq!(intent.sealing_from_state, Some(EntryState::Armed));
    assert!(intent.sealed_at.is_none());

    let restored = registry.restore_unfinished_sealing("sealing").unwrap();
    assert_eq!(restored.state, EntryState::Armed);
    assert_eq!(restored.sealing_from_state, None);
}

#[test]
fn sealing_refuses_an_unreconciled_prepared_process_handoff() {
    let tmp = tempfile::tempdir().unwrap();
    let registry = Registry::open(&tmp.path().join("registry.redb")).unwrap();
    let mut row = bound_entry_with_handoff(
        tmp.path(),
        "prepared-seal",
        "2026.09-v1",
        ExpectedProcessHandoff::Required,
    );
    row.state = EntryState::Armed;
    registry.put(&row).unwrap();
    registry
        .open_initial_process_handoff("prepared-seal")
        .unwrap();
    registry
        .prepare_process_handoff("prepared-seal", registration(1))
        .unwrap();

    registry
        .begin_sealing("prepared-seal", sealing_intent(&row))
        .expect_err("a prepared child must become a durable termination target first");
    let preserved = registry.get("prepared-seal").unwrap().unwrap();
    assert_eq!(preserved.state, EntryState::Armed);
    assert_eq!(
        preserved.process_handoff_phase(),
        ProcessHandoffPhase::Prepared
    );
}

/// A prepared row is durable private recovery state, but it is still the source for a
/// future public `HomeSeeded` event.  A direct authenticated caller therefore cannot smuggle
/// a resolved account path into it: only the manifest's `{home}/…` spelling and relative
/// file names are safe to retain across a daemon crash.
#[test]
fn prepared_seed_recovery_never_retains_resolved_or_escaping_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let registry = Registry::open(&tmp.path().join("registry.redb")).unwrap();
    let mut row = bound_entry_with_handoff(
        tmp.path(),
        "launch",
        "2026.09-v1",
        ExpectedProcessHandoff::Required,
    );
    row.state = EntryState::Armed;
    registry.put(&row).unwrap();
    registry.open_initial_process_handoff("launch").unwrap();
    let before = registry.get("launch").unwrap().unwrap();

    let seed = |from: &str, file: &str| HomeSeedRecord {
        handoff_generation: None,
        handoff_request_digest: None,
        connector: "codex-cli".to_string(),
        from: from.to_string(),
        keys: vec![SeededKey::new("theme", &serde_json::json!("dark"))],
        keys_unchanged: Vec::new(),
        written: Vec::new(),
        files: vec![SeededFile {
            name: file.to_string(),
            bytes: 42,
        }],
        files_kept: Vec::new(),
        files_missing: Vec::new(),
        wire_layout: Default::default(),
    };

    let mut resolved = registration(1);
    resolved.handle.home_seed = Some(seed("/Users/participant/.codex", "auth.json"));
    registry
        .prepare_process_handoff("launch", resolved)
        .expect_err("a resolved account path is never durable recovery evidence");
    assert_eq!(registry.get("launch").unwrap().unwrap(), before);

    let mut escaping = registration(1);
    escaping.handle.home_seed = Some(seed("{home}/.codex", "../../auth.json"));
    registry
        .prepare_process_handoff("launch", escaping)
        .expect_err("an escaping file name is not a safe relative seed record");
    assert_eq!(registry.get("launch").unwrap().unwrap(), before);

    let mut safe = registration(1);
    safe.handle.home_seed = Some(seed("{home}/.codex", "auth.json"));
    registry
        .prepare_process_handoff("launch", safe)
        .expect("the manifest spelling and relative name are safe recovery evidence");
}

/// Resume must consume one specific last generation atomically.  A stale second resume
/// cannot clear the process that the first resume has just installed or mint another
/// concurrent authority token.
#[test]
fn resume_generation_compare_and_swap_has_one_winner_and_legacy_is_fail_closed() {
    let tmp = tempfile::tempdir().unwrap();
    let registry = std::sync::Arc::new(Registry::open(&tmp.path().join("registry.redb")).unwrap());
    let mut row = bound_entry_with_handoff(
        tmp.path(),
        "launch",
        "2026.09-v1",
        ExpectedProcessHandoff::Required,
    );
    row.state = EntryState::Armed;
    registry.put(&row).unwrap();
    registry.open_initial_process_handoff("launch").unwrap();
    registry
        .prepare_process_handoff("launch", registration(1))
        .unwrap();
    registry
        .register_prepared_process_handoff("launch", 1)
        .unwrap();
    registry
        .update("launch", |entry| {
            entry.state = EntryState::Started;
            entry.session_id = Some("s1".to_string());
        })
        .unwrap();

    let left = std::sync::Arc::clone(&registry);
    let right = std::sync::Arc::clone(&registry);
    let a = std::thread::spawn(move || left.open_resume_process_handoff("launch", 1));
    let b = std::thread::spawn(move || right.open_resume_process_handoff("launch", 1));
    let results = [a.join().unwrap(), b.join().unwrap()];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    let opened = results.into_iter().find_map(Result::ok).unwrap();
    assert_eq!(opened.process_handoff_generation(), 2);
    assert_eq!(
        opened.process_handoff_phase(),
        ProcessHandoffPhase::OpenResume
    );
    assert_eq!(
        opened.last_process.as_ref().map(|handle| handle.pid),
        Some(std::process::id()),
        "an issued permit keeps the predecessor as seal authority until a replacement registers"
    );

    let mut legacy = bound_entry_with_handoff(
        tmp.path(),
        "legacy",
        "2026.09-v1",
        ExpectedProcessHandoff::LegacyUnknown,
    );
    legacy.state = EntryState::Armed;
    let mut old = serde_json::to_value(&legacy).unwrap();
    let object = old.as_object_mut().unwrap();
    object.remove("process_handoff_phase");
    object.remove("process_handoff_generation");
    let legacy: Entry = serde_json::from_value(old).unwrap();
    registry.put(&legacy).unwrap();
    registry
        .prepare_process_handoff("legacy", registration(1))
        .expect_err("legacy required/no-phase rows cannot be opened by /process");
    registry
        .open_resume_process_handoff("legacy", 0)
        .expect_err("only a Started legacy row may obtain its first fresh resume generation");
    registry
        .update("legacy", |entry| {
            entry.state = EntryState::Started;
            entry.session_id = Some("s1".to_string());
        })
        .unwrap();
    let fresh = registry
        .open_resume_process_handoff("legacy", 0)
        .expect("a started legacy run can safely resume into a fresh generation");
    assert_eq!(fresh.process_handoff_generation(), 1);
    assert_eq!(
        fresh.process_handoff_phase(),
        ProcessHandoffPhase::OpenResume
    );
}

/// The `[5u8; 32]` seed is the placeholder server key the devserver and the compliant
/// fixture both sign with, so a receipt made here is one a default dev daemon accepts.
fn server_key() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[5u8; 32])
}

/// The hex verifying key a build pins, which is what `store_receipt` checks against.
fn pinned(key: &ed25519_dalek::SigningKey) -> String {
    hex::encode(key.verifying_key().to_bytes())
}

fn receipt_for(entry_id: &str, key: &ed25519_dalek::SigningKey) -> StartReceipt {
    StartReceipt::sign(
        StartReceiptClaims {
            entry_id: entry_id.to_string(),
            connector_id: "claude-code-cli".to_string(),
            workspace: "/ws/e1".to_string(),
            session_id: "s1".to_string(),
            started_at: "2026-09-01T00:00:00Z".to_string(),
            deadline_at: "2026-09-08T00:00:00Z".to_string(),
            manifest_sha256: "ab".repeat(32),
            chain_head: "cd".repeat(32),
            // Round-tripped like every other claim: `store_receipt` verifies the signature
            // over whatever bytes the layout produced, and this field is in those bytes.
            connector_dir_sha256: Some("ef".repeat(32)),
            run_public_key: None,
            questionnaire_version: None,
        },
        key,
    )
    .unwrap()
}

#[test]
fn an_entry_round_trips_through_the_registry() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Registry::open(&tmp.path().join("registry.redb")).unwrap();
    let e = entry(tmp.path(), "e1");
    r.put(&e).unwrap();

    let back = r.get("e1").unwrap().unwrap();
    assert_eq!(back, e);
    assert_eq!(back.state, EntryState::Created);
    assert_eq!(
        back.chain_seq, None,
        "nothing is written until the first append"
    );
    assert_eq!(back.chain_head, goalcup_protocol::chain::GENESIS);
    assert_eq!(back.trace_dir, back.artifact_dir.join("trace"));
}

#[test]
fn receipt_policy_is_legacy_for_old_rows_and_immutable_across_replacement_and_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("registry.redb");

    // JSON written before the field existed still means Legacy.
    let current = entry(tmp.path(), "old");
    let mut json = serde_json::to_value(&current).unwrap();
    json.as_object_mut().unwrap().remove("receipt_binding");
    let old: Entry = serde_json::from_value(json).unwrap();
    assert_eq!(old.receipt_binding(), &ReceiptBindingPolicy::Legacy);

    {
        let registry = Registry::open(&path).unwrap();
        let original = bound_entry(tmp.path(), "e1", "2026.09-v1");
        registry.put(&original).unwrap();

        let mut stale = entry(tmp.path(), "e1");
        stale.state = EntryState::Armed;
        registry.put(&stale).unwrap();
        assert_eq!(
            registry.get("e1").unwrap().unwrap().receipt_binding(),
            original.receipt_binding(),
            "put cannot downgrade or retarget the stored policy"
        );

        registry
            .update("e1", |value| {
                *value = entry(tmp.path(), "e1");
            })
            .unwrap();
        assert_eq!(
            registry.get("e1").unwrap().unwrap().receipt_binding(),
            original.receipt_binding(),
            "update cannot change the private policy"
        );
    }

    let reopened = Registry::open(&path).unwrap();
    assert_eq!(
        reopened.get("e1").unwrap().unwrap().receipt_binding(),
        &ReceiptBindingPolicy::NewEvent {
            questionnaire_version: "2026.09-v1".to_string()
        }
    );
}

#[test]
fn expected_process_handoff_defaults_for_old_rows_and_round_trips_for_new_ones() {
    let current = entry(tempfile::tempdir().unwrap().path(), "e1");
    let mut old_json = serde_json::to_value(&current).unwrap();
    old_json
        .as_object_mut()
        .expect("an entry serializes as an object")
        .remove("expected_process_handoff");
    old_json
        .as_object_mut()
        .expect("an entry serializes as an object")
        .remove("process_handoff_phase");
    old_json
        .as_object_mut()
        .expect("an entry serializes as an object")
        .remove("process_handoff_generation");
    let old: Entry = serde_json::from_value(old_json).unwrap();
    assert_eq!(
        old.expected_process_handoff(),
        ExpectedProcessHandoff::LegacyUnknown,
        "an old row has no trustworthy launch-mode provenance"
    );
    assert_eq!(old.process_handoff_generation(), 0);
    assert_eq!(
        old.process_handoff_phase(),
        ProcessHandoffPhase::LegacyUnknown,
        "an old row has no trustworthy issued-generation provenance either"
    );

    for expected in [
        ExpectedProcessHandoff::LegacyUnknown,
        ExpectedProcessHandoff::Required,
        ExpectedProcessHandoff::NotRequired,
    ] {
        let row = bound_entry_with_handoff(
            tempfile::tempdir().unwrap().path(),
            "e1",
            "2026.09-v1",
            expected,
        );
        let reread: Entry = serde_json::from_value(serde_json::to_value(&row).unwrap()).unwrap();
        assert_eq!(reread.expected_process_handoff(), expected);
    }
}

/// Terminal-drain quarantine is terminal evidence, not an ordinary health hint.  Broad
/// `update` and whole-row `put` are deliberately available for routine facts, so both must
/// preserve an existing failure marker; only the dedicated terminal-validation path may set
/// it and no generic caller may clear it back to an all-clear row.
#[test]
fn generic_writes_cannot_clear_terminal_drain_validation_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let registry = Registry::open(&tmp.path().join("registry.redb")).unwrap();
    let mut original = entry(tmp.path(), "e1");
    original.terminal_drain_validation_failed = true;
    registry.put(&original).unwrap();

    let after_update = registry
        .update("e1", |entry| entry.terminal_drain_validation_failed = false)
        .expect("generic update completes");
    assert!(
        after_update.terminal_drain_validation_failed,
        "generic update cannot erase durable terminal review"
    );

    let mut replacement = after_update.clone();
    replacement.terminal_drain_validation_failed = false;
    registry.put(&replacement).expect("whole-row put completes");
    assert!(
        registry
            .get("e1")
            .unwrap()
            .expect("the stored row remains")
            .terminal_drain_validation_failed,
        "generic put cannot erase durable terminal review"
    );
}

/// The `Registered` phase alone is not enough to keep launch authority coherent: seal and
/// health act on `last_process`.  Generic replacement writes must not erase or forge that
/// public termination target while preserving the private accepted-handoff fields.
#[test]
fn generic_writes_cannot_desynchronize_the_registered_termination_handle() {
    let tmp = tempfile::tempdir().unwrap();
    let registry = Registry::open(&tmp.path().join("registry.redb")).unwrap();
    let mut original = bound_entry_with_handoff(
        tmp.path(),
        "launch",
        "2026.09-v1",
        ExpectedProcessHandoff::Required,
    );
    original.state = EntryState::Armed;
    registry.put(&original).unwrap();
    let accepted = register_initial(&registry, "launch");
    let standing = accepted
        .last_process
        .clone()
        .expect("registration owns a handle");
    let mut decoy = standing.clone();
    decoy.pid += 1;

    let after_update = registry
        .update("launch", |entry| entry.last_process = Some(decoy.clone()))
        .expect("ordinary update completes");
    assert_eq!(
        after_update.last_process,
        Some(standing.clone()),
        "generic update cannot forge the seal target"
    );

    let mut replacement = after_update.clone();
    replacement.last_process = None;
    registry
        .put(&replacement)
        .expect("whole entry replacement completes");
    let after_put = registry.get("launch").unwrap().unwrap();
    assert_eq!(
        after_put.last_process,
        Some(standing),
        "whole entry replacement cannot erase the accepted termination target"
    );
    assert_eq!(
        after_put.process_handoff_phase(),
        ProcessHandoffPhase::Registered,
        "the handle and the accepted phase remain one authority"
    );
}

/// `Entry.receipt` is private and has no unchecked writer, so the receipt goes in
/// through the legacy compatibility path that verifies first.
#[test]
fn the_registry_holds_the_signed_receipt_r2_asks_for() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Registry::open(&tmp.path().join("registry.redb")).unwrap();
    r.put(&entry(tmp.path(), "e1")).unwrap();
    let server = server_key();
    let receipt = receipt_for("e1", &server);

    let stored = r.store_receipt("e1", &receipt, &pinned(&server)).unwrap();
    assert_eq!(stored.receipt(), Some(&receipt));

    let updated = r
        .update("e1", |e| {
            e.state = EntryState::Started;
            e.started_at = Some(receipt.claims.started_at.clone());
        })
        .unwrap();
    assert_eq!(updated.receipt(), Some(&receipt));
    assert_eq!(
        r.get("e1")
            .unwrap()
            .unwrap()
            .receipt()
            .unwrap()
            .claims
            .session_id,
        "s1"
    );
}

/// The receipt's invariant, stated as a property of the type rather than a convention: a caller
/// outside this crate cannot build an `Entry` at all except through `Entry::new` (the
/// `receipt` field is private, so a struct literal does not compile), and `new` leaves it
/// empty. `update` hands out `&mut Entry`, and there is no setter to reach the field
/// with — a closure that tries to keep the receipt it was given cannot change it.
#[test]
fn an_entry_can_only_be_born_without_a_receipt_and_update_cannot_add_one() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(entry(tmp.path(), "e1").receipt(), None);

    let r = Registry::open(&tmp.path().join("registry.redb")).unwrap();
    r.put(&entry(tmp.path(), "e1")).unwrap();
    let server = server_key();
    let receipt = receipt_for("e1", &server);
    r.store_receipt("e1", &receipt, &pinned(&server)).unwrap();

    // An `update` closure gets `&mut Entry` and still cannot reach the receipt: the most
    // it can do is replace the whole value with a fresh one, and the receipt survives.
    let after = r
        .update("e1", |e| {
            *e = Entry::new(
                "e1",
                &e.connector_id,
                e.workspace.clone(),
                e.artifact_dir.clone(),
                &e.manifest_sha256,
                &e.harness_version,
                goalcup_daemon::config::ReceiptBindingPolicy::Legacy,
                ExpectedProcessHandoff::NotRequired,
                e.keys.clone(),
            );
        })
        .unwrap();
    assert_eq!(
        after.receipt(),
        Some(&receipt),
        "update cannot drop the verified receipt either"
    );

    // A `put` of an entry value that never held a receipt does not erase the stored one
    // either: the receipt is the registry's, not the caller's.
    r.put(&entry(tmp.path(), "e1")).unwrap();
    assert_eq!(r.get("e1").unwrap().unwrap().receipt(), Some(&receipt));
}

#[test]
fn remove_leaves_no_entry_and_in_harness_home_finds_the_occupant() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Registry::open(&tmp.path().join("registry.redb")).unwrap();
    let home = tmp.path().join("home/codex");
    let mut e = entry(tmp.path(), "e1");
    e.harness_home = Some(home.clone());
    r.put(&e).unwrap();

    // The daemon must be able to ask "is this CODEX_HOME already mine?".
    assert_eq!(r.in_harness_home(&home).unwrap().len(), 1);
    r.remove("e1").unwrap();
    assert!(
        r.get("e1").unwrap().is_none(),
        "R5: a refused start leaves no entry"
    );
    assert!(r.in_harness_home(&home).unwrap().is_empty());
}

/// `harness_home` is `None` until workspace creation sets it. A default
/// that guessed a plausible path would make the pre-arm scan answer "nobody lives
/// here" for every entry that has not been placed yet, which is the wrong answer given
/// confidently. An unset home matches nothing at all.
#[test]
fn in_harness_home_ignores_an_entry_that_has_no_home_yet() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Registry::open(&tmp.path().join("registry.redb")).unwrap();
    let home = tmp.path().join("homes").join("entry-0001");

    let unplaced = entry(tmp.path(), "e1");
    assert_eq!(unplaced.harness_home, None, "a fresh entry has no home");
    r.put(&unplaced).unwrap();
    assert!(
        r.in_harness_home(&home).unwrap().is_empty(),
        "an entry with no home occupies no home"
    );
    // Nor does it match the path a derived default would have guessed.
    assert!(
        r.in_harness_home(&unplaced.artifact_dir.join("home"))
            .unwrap()
            .is_empty()
    );

    let mut placed = entry(tmp.path(), "e2");
    placed.harness_home = Some(home.clone());
    r.put(&placed).unwrap();
    let found = r.in_harness_home(&home).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].entry_id, "e2");
}

/// D4: entries are routed by working directory, so two entries bound to one workspace
/// would make the route a coin toss. The second `put` is refused and the first entry is
/// left exactly as it was.
#[test]
fn put_refuses_a_second_entry_bound_to_the_same_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Registry::open(&tmp.path().join("registry.redb")).unwrap();
    let first = entry(tmp.path(), "e1");
    r.put(&first).unwrap();

    let mut second = entry(tmp.path(), "e2");
    second.workspace = first.workspace.clone();
    let err = r.put(&second).unwrap_err();
    let sentence = err.to_string();
    assert!(
        sentence.contains("e1") && sentence.contains(&first.workspace.display().to_string()),
        "the refusal names the workspace and the entry already holding it: {sentence}"
    );

    assert!(r.get("e2").unwrap().is_none(), "nothing was written");
    assert_eq!(r.all().unwrap().len(), 1);
    assert_eq!(r.get("e1").unwrap().unwrap(), first);

    // Rewriting the *same* entry is not a second binding.
    r.put(&first).unwrap();
    assert_eq!(r.all().unwrap().len(), 1);
}

/// A receipt that does not verify against the pinned server key is never stored, so a
/// stored receipt is a verified one by construction. This is the one write path the arming
/// gate uses; `update` stays open for everything else.
#[test]
fn a_receipt_signed_by_the_wrong_key_is_refused_and_nothing_is_stored() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Registry::open(&tmp.path().join("registry.redb")).unwrap();
    r.put(&entry(tmp.path(), "e1")).unwrap();

    let server = server_key();
    let key = pinned(&server);
    let impostor = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);

    let forged = receipt_for("e1", &impostor);
    let err = r.store_receipt("e1", &forged, &key).unwrap_err();
    assert!(
        err.to_string().contains("receipt"),
        "the refusal names the receipt, not a database: {err}"
    );
    assert!(
        r.get("e1").unwrap().unwrap().receipt().is_none(),
        "a receipt that did not verify is never stored"
    );

    let genuine = receipt_for("e1", &server);
    let stored = r.store_receipt("e1", &genuine, &key).unwrap();
    assert_eq!(stored.receipt(), Some(&genuine));
    assert_eq!(r.get("e1").unwrap().unwrap().receipt(), Some(&genuine));
}

/// The registry is the daemon's memory across a crash. Reopening the same file must give
/// back the same entries, field for field, receipt included.
#[test]
fn the_registry_survives_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("registry.redb");
    let server = server_key();
    let receipt = receipt_for("e1", &server);
    let mut e = entry(tmp.path(), "e1");
    e.state = EntryState::Started;
    e.chain_seq = Some(0);
    e.chain_head = "ef".repeat(32);
    e.harness_home = Some(tmp.path().join("homes").join("e1"));
    e.expected_config
        .insert("settings.json".to_string(), "aa".repeat(32));

    let written;
    {
        let r = Registry::open(&path).unwrap();
        r.put(&e).unwrap();
        written = r.store_receipt("e1", &receipt, &pinned(&server)).unwrap();
    }

    let r = Registry::open(&path).unwrap();
    assert_eq!(r.get("e1").unwrap().unwrap(), written);
    assert_eq!(r.all().unwrap(), vec![written.clone()]);
    assert_eq!(written.receipt(), Some(&receipt));
}

/// A fresh registry has no entries.
#[test]
fn a_fresh_registry_is_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Registry::open(&tmp.path().join("registry.redb")).unwrap();
    assert!(r.all().unwrap().is_empty());
    assert!(r.get("nobody").unwrap().is_none());
    // Removing what was never there is not an error.
    r.remove("nobody").unwrap();
}

/// `RunKeys` holds two secrets and a bearer token. None of the three may reach a log, and
/// `Debug` is how they would: `Entry` derives it, and a `DaemonError` message interpolates
/// whatever it is handed. This is the same redaction `WorkspaceInfo` already applies.
#[test]
fn run_keys_never_print_their_secrets() {
    let keys = RunKeys::from_seeds([1u8; 32], [2u8; 32], [3u8; 16]);
    let printed = format!("{keys:?}");
    for secret in [
        &keys.buffer_key_hex,
        &keys.signing_seed_hex,
        &keys.receiver_token,
    ] {
        assert!(
            !printed.contains(secret.as_str()),
            "RunKeys Debug leaked a secret: {printed}"
        );
    }
    assert!(printed.contains("<redacted>"));

    let tmp = tempfile::tempdir().unwrap();
    let entry_printed = format!("{:?}", entry(tmp.path(), "e1"));
    assert!(
        !entry_printed.contains(keys.receiver_token.as_str()),
        "Entry Debug leaked the receiver token: {entry_printed}"
    );
}

/// The signing seed is the run key. `public_key_hex` is what `WorkspaceInfo.run_public_key`
/// carries and what `SignedStatus::verify_with` and `Chunk::verify` are pinned against, so
/// it must be the verifying key of the seed and nothing else.
#[test]
fn the_run_key_is_the_signing_seed() {
    let keys = RunKeys::from_seeds([1u8; 32], [2u8; 32], [3u8; 16]);
    let expected = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
    assert_eq!(
        keys.public_key_hex().unwrap(),
        hex::encode(expected.verifying_key().to_bytes())
    );
    assert_eq!(
        keys.signing_key().unwrap().to_bytes(),
        expected.to_bytes(),
        "signing_key rebuilds the key the seed names"
    );

    // Generated keys are distinct, per run, in all three secrets.
    let a = RunKeys::generate();
    let b = RunKeys::generate();
    assert_ne!(a.buffer_key_hex, b.buffer_key_hex);
    assert_ne!(a.signing_seed_hex, b.signing_seed_hex);
    assert_ne!(a.receiver_token, b.receiver_token);
    assert_eq!(a.buffer_key_hex.len(), 64);
    assert_eq!(a.signing_seed_hex.len(), 64);
    assert_eq!(a.receiver_token.len(), 32);
    a.public_key_hex().unwrap();
}

/// The registry file holds `RunKeys` in plaintext: the receiver token grants write access
/// to this run's capture stream, and the signing seed is the run key itself. On a shared
/// machine that makes the file a bearer-token store, so it is `0o600` inside a `0o700`
/// directory — and a file that was created loose is tightened on open, not left.
#[cfg(unix)]
#[test]
fn the_registry_file_is_readable_only_by_its_owner() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let path = Registry::default_path(tmp.path());
    let dir = path.parent().unwrap().to_path_buf();
    assert_eq!(
        dir,
        tmp.path().join("registry"),
        "the registry's own directory"
    );
    let r = Registry::open(&path).unwrap();
    r.put(&entry(tmp.path(), "e1")).unwrap();
    drop(r);

    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600, "the registry file");
    assert_eq!(mode(&dir), 0o700, "the directory it lives in");

    // Loosened behind the daemon's back, then reopened: the mode is repaired, and the
    // directory that holds nothing but the registry is still the registry's to tighten.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let r = Registry::open(&path).unwrap();
    assert_eq!(mode(&path), 0o600, "reopening tightens a loosened file");
    assert_eq!(mode(&dir), 0o700, "and its own directory");
    assert_eq!(r.get("e1").unwrap().unwrap().entry_id, "e1");
}

/// A row written by a build before `server_port` and `seal_outcome` existed (pre-21ad712)
/// has neither key at all. Both fields carry `#[serde(default)]` precisely so such a row
/// still loads; this pins that a row missing both keys deserialises with both `None`
/// rather than refusing to load, which is the failure a dropped `#[serde(default)]` would
/// cause the next time this test runs.
#[test]
fn a_pre_server_port_row_loads_with_server_port_and_seal_outcome_both_none() {
    let json = r#"{
        "entry_id": "e1",
        "connector_id": "claude-code-cli",
        "workspace": "/ws/e1",
        "artifact_dir": "/art/e1",
        "trace_dir": "/art/e1/trace",
        "manifest_sha256": "ab",
        "harness_version": "2.1.259",
        "state": "created",
        "health": "online",
        "session_id": null,
        "armed_at": null,
        "start_signal_at": null,
        "started_at": null,
        "deadline_at": null,
        "receipt": null,
        "chain_seq": null,
        "chain_head": "0000000000000000000000000000000000000000000000000000000000000000",
        "last_ack_seq": null,
        "last_ack_at": null,
        "events_uploaded": 0,
        "expected_config": {},
        "pinned_hook_hash": null,
        "deadline_unix": null,
        "sealed_at": null,
        "status_line_path": "/art/e1/status-line.txt",
        "harness_home": null,
        "arming_timeout_seconds": null,
        "last_process": null,
        "created_at": "2026-09-01T00:00:00Z",
        "keys": {
            "buffer_key_hex": "01",
            "signing_seed_hex": "02",
            "receiver_token": "03"
        }
    }"#;

    let row: Entry = serde_json::from_str(json).expect(
        "a row missing server_port and seal_outcome must still deserialise: both fields \
         are #[serde(default)] precisely so a pre-upgrade row keeps loading",
    );
    assert_eq!(row.server_port, None);
    assert_eq!(row.seal_outcome, None);
}

/// The mode the registry sets on its parent is the registry's to set only when the
/// parent is the registry's. Handed a path inside a directory that holds somebody else's
/// files — a shared `config_dir`, say, which the CLI and the MCP server both read — the
/// daemon tightens the file and leaves the directory exactly as it found it.
#[cfg(unix)]
#[test]
fn open_does_not_tighten_a_directory_it_shares_with_other_files() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let shared = tmp.path().join("config");
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::write(shared.join("daemon.toml"), b"# somebody else's\n").unwrap();
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();

    let path = shared.join("registry.redb");
    let r = Registry::open(&path).unwrap();
    r.put(&entry(tmp.path(), "e1")).unwrap();

    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode(&shared),
        0o755,
        "a directory the registry shares is not the registry's to chmod"
    );
    assert_eq!(mode(&path), 0o600, "the file is still the registry's own");
}
