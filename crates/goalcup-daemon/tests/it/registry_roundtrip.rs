//! The entry registry: what an entry holds, and that it still holds it after a restart.
//!
//! An entry carries "workspace path, harness, manifest hash, session
//! binding, signed receipts, per-run keys, health". These tests pin the entry, alongside
//! the two lookups D4 and D24 need (by
//! workspace, by token) and the removal a failed start needs.

use goalcup_daemon::registry::{
    Entry, EntryState, ExpectedProcessHandoff, ProcessHandoffPhase, Registry, RunKeys,
};
use goalcup_protocol::manifest::KillStrategy;
use goalcup_protocol::process::{ProcessHandle, ProcessRegistration};

fn entry(tmp: &std::path::Path, id: &str) -> Entry {
    Entry::new(
        id,
        "claude-code-cli",
        tmp.join("ws").join(id),
        tmp.join("art").join(id),
        &"ab".repeat(32),
        "2.1.259",
        ExpectedProcessHandoff::NotRequired,
        RunKeys::from_seed([2u8; 32]),
    )
}

fn bound_entry_with_handoff(
    tmp: &std::path::Path,
    id: &str,
    expected_process_handoff: ExpectedProcessHandoff,
) -> Entry {
    let mut entry = Entry::new(
        id,
        "claude-code-cli",
        tmp.join("ws").join(id),
        tmp.join("art").join(id),
        &"ab".repeat(32),
        "2.1.259",
        expected_process_handoff,
        RunKeys::from_seed([2u8; 32]),
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
    let mut row = bound_entry_with_handoff(tmp.path(), "launch", ExpectedProcessHandoff::Required);
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
    let mut row = bound_entry_with_handoff(tmp.path(), "launch", ExpectedProcessHandoff::Required);
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
        let row = bound_entry_with_handoff(tempfile::tempdir().unwrap().path(), "e1", expected);
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
    let mut original =
        bound_entry_with_handoff(tmp.path(), "launch", ExpectedProcessHandoff::Required);
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

#[test]
fn remove_leaves_no_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Registry::open(&tmp.path().join("registry.redb")).unwrap();
    r.put(&entry(tmp.path(), "e1")).unwrap();
    r.remove("e1").unwrap();
    assert!(
        r.get("e1").unwrap().is_none(),
        "R5: a refused start leaves no entry"
    );
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

/// A continued run makes its next segment in the workspace its last one had: an entry the caller
/// says is finished with its workspace no longer holds it, and one it does not say so of still
/// does.
#[test]
fn an_entry_that_lets_go_of_its_workspace_does_not_hold_it() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Registry::open(&tmp.path().join("registry.redb")).unwrap();
    let first = entry(tmp.path(), "e1");
    r.put(&first).unwrap();
    let mut second = entry(tmp.path(), "e2");
    second.workspace = first.workspace.clone();
    r.put_letting_go(&second, &|stored| stored.entry_id != "e1")
        .unwrap_err();
    r.put_letting_go(&second, &|stored| stored.entry_id == "e1")
        .unwrap();
    assert_eq!(r.all().unwrap().len(), 2);
    // The second entry holds it now.
    let mut third = entry(tmp.path(), "e3");
    third.workspace = first.workspace.clone();
    let err = r
        .put_letting_go(&third, &|s| s.entry_id == "e1")
        .unwrap_err();
    assert!(err.to_string().contains("e2"), "{err}");
}

/// The registry is the daemon's memory across a crash. Reopening the same file must give
/// back the same entries, field for field.
#[test]
fn the_registry_survives_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("registry.redb");
    let mut e = entry(tmp.path(), "e1");
    e.state = EntryState::Started;
    e.chain_seq = Some(0);
    e.chain_head = "ef".repeat(32);
    e.connector_dir_sha256 = Some("cd".repeat(32));

    {
        let r = Registry::open(&path).unwrap();
        r.put(&e).unwrap();
    }

    let r = Registry::open(&path).unwrap();
    assert_eq!(r.get("e1").unwrap().unwrap(), e);
    assert_eq!(r.all().unwrap(), vec![e.clone()]);
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

/// `RunKeys` holds the run's signing seed, which may not reach a log, and `Debug` is how it
/// would: `Entry` derives it, and a `DaemonError` message interpolates whatever it is handed.
/// This is the same redaction `CaptureWorkspaceInfo` applies to its ingest secret.
#[test]
fn run_keys_never_print_their_secrets() {
    let keys = RunKeys::from_seed([2u8; 32]);
    let printed = format!("{keys:?}");
    assert!(
        !printed.contains(keys.signing_seed_hex.as_str()),
        "RunKeys Debug leaked the seed: {printed}"
    );
    assert!(printed.contains("<redacted>"));

    let tmp = tempfile::tempdir().unwrap();
    let entry_printed = format!("{:?}", entry(tmp.path(), "e1"));
    assert!(
        !entry_printed.contains(keys.signing_seed_hex.as_str()),
        "Entry Debug leaked the seed: {entry_printed}"
    );
}

/// The signing seed is the run key. `public_key_hex` is what
/// `CaptureWorkspaceInfo.run_public_key` carries and what `SignedStatus::verify_with` is pinned
/// against, so it must be the verifying key of the seed and nothing else.
#[test]
fn the_run_key_is_the_signing_seed() {
    let keys = RunKeys::from_seed([2u8; 32]);
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

    // Generated seeds are distinct, per run.
    let a = RunKeys::generate();
    let b = RunKeys::generate();
    assert_ne!(a.signing_seed_hex, b.signing_seed_hex);
    assert_eq!(a.signing_seed_hex.len(), 64);
    a.public_key_hex().unwrap();
}

/// The registry file holds `RunKeys` in plaintext: the signing seed is the run key itself.
/// On a shared machine that makes the file a secret store, so it is `0o600` inside a `0o700`
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

/// A row written by an older daemon still loads: it carries the V1-era keys this build no
/// longer reads (session and deadline stamps, the hook config digests, a sealing intent,
/// pending foreign sightings, a seal outcome, the receipt, the round dates, the buffer key and
/// receiver token) and a health word of the retired V1 ladder, and serde ignores the keys and
/// reads the word as `unknown` rather than refusing the row. A registry a daemon cannot open
/// is a run it cannot finish.
#[test]
fn a_row_carrying_keys_this_build_no_longer_reads_still_loads() {
    let json = r#"{
        "entry_id": "e1",
        "connector_id": "claude-code-cli",
        "workspace": "/ws/e1",
        "artifact_dir": "/art/e1",
        "trace_dir": "/art/e1/trace",
        "manifest_sha256": "ab",
        "harness_version": "2.1.259",
        "state": "sealed",
        "health": "hooks_silent",
        "session_id": "95addb64",
        "armed_at": 1.0,
        "start_signal_at": 2.0,
        "started_at": "2026-09-01T00:00:01Z",
        "deadline_at": "2026-09-08T00:00:01Z",
        "receipt": null,
        "receipt_binding": {"mode": "new_event", "questionnaire_version": "2026.09-v1"},
        "chain_seq": 3,
        "chain_head": "0000000000000000000000000000000000000000000000000000000000000000",
        "last_ack_seq": 3,
        "last_ack_at": 3.0,
        "events_uploaded": 4,
        "expected_config": {"settings.json": "aa"},
        "pinned_hook_hash": null,
        "deadline_unix": 4.0,
        "source_date": ["2026", "09", "01"],
        "deadline_observed": true,
        "finalization_observed_unix": 5,
        "sealed_at": "2026-09-02T00:00:00Z",
        "sealing_from_state": "started",
        "sealing_intent": {"anything": "the old arms held"},
        "sealed_trace_root_identity": "trace-root-v1:old",
        "status_line_path": "/art/e1/status-line.txt",
        "harness_home": "/homes/e1",
        "foreign_pending": [{"family": "f", "harness": "h", "session_id": "s",
                             "store_path": "relative", "first_seen": "not a time"}],
        "server_port": 4100,
        "arming_timeout_seconds": 60,
        "cleanup_termination_complete": true,
        "last_process": null,
        "event": {"t0": "2026-09-01T00:00:00Z", "start_window_closes_at": "2026-09-02T00:00:00Z",
                  "publication_closes_at": "2026-09-08T00:00:00Z", "rules_version": "r1"},
        "seal_outcome": {"lock_paths": [], "lock_paths_error": null, "guard_degraded": false,
                         "termination": null, "termination_error": null},
        "created_at": "2026-09-01T00:00:00Z",
        "keys": {
            "buffer_key_hex": "01",
            "signing_seed_hex": "02",
            "receiver_token": "03"
        }
    }"#;

    let row: Entry = serde_json::from_str(json)
        .expect("a row carrying keys this build no longer reads must still deserialise");
    assert_eq!(row.entry_id, "e1");
    assert_eq!(row.state, EntryState::Sealed);
    assert_eq!(row.health, goalcup_protocol::events::HealthState::Unknown);
    assert_eq!(row.sealed_at.as_deref(), Some("2026-09-02T00:00:00Z"));
    assert_eq!(row.last_ack_seq, Some(3));
    assert_eq!(row.keys.signing_seed_hex, "02");

    let tmp = tempfile::tempdir().unwrap();
    let registry = Registry::open(&tmp.path().join("registry.redb")).unwrap();
    registry
        .replace_raw_json_for_test("e1", json.as_bytes())
        .unwrap();
    assert_eq!(registry.get("e1").unwrap().unwrap(), row);
    assert_eq!(registry.all().unwrap(), vec![row]);
}

/// A row this build writes carries none of the V1 keys: no receipt, no round dates, no upload
/// counter, hook digests or status-line path, and no buffer key or receiver token.
#[test]
fn a_new_row_carries_no_v1_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let row = serde_json::to_value(entry(tmp.path(), "e1")).unwrap();
    for key in [
        "receipt",
        "event",
        "events_uploaded",
        "expected_config",
        "status_line_path",
    ] {
        assert!(row.get(key).is_none(), "{key} is still written: {row}");
    }
    for key in ["buffer_key_hex", "receiver_token"] {
        assert!(
            row["keys"].get(key).is_none(),
            "keys.{key} is still written"
        );
    }
}

/// The mode the registry sets on its parent is the registry's to set only when the
/// parent is the registry's. Handed a path inside a directory that holds somebody else's
/// files — a shared `config_dir`, say, which the CLI reads — the
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
