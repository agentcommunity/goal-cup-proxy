//! Restart recovery for the registry-authoritative start receipt mirror.
//!
//! These controls use the same on-disk registry, trace and runtime constructor as daemon
//! restart. The registry's checked receipt is the authority; `receipt.json` is only its
//! byte-exact mirror beneath the already acquired trace root.
#![cfg(feature = "v1-compat")]

mod support;

use goalcup_daemon::registry::{Entry, EntryState};
use goalcup_protocol::export::{MANIFEST_FILE, RECEIPT_FILE, TraceExport};

fn trace_snapshot(root: &std::path::Path) -> Vec<(std::ffi::OsString, Vec<u8>)> {
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

fn compact_receipt(entry: &Entry) -> Vec<u8> {
    serde_json::to_vec(
        entry
            .receipt()
            .expect("the checked registry row owns the receipt"),
    )
    .expect("the receipt has compact wire bytes")
}

async fn assert_rebuild_refuses_without_mutation(h: &support::EntryHarness) {
    let before_trace = trace_snapshot(&h.trace_dir);
    let before_row = h.entry();
    let error = support::try_reopen_runtime(h)
        .await
        .err()
        .expect("the contradictory receipt evidence is refused");
    assert!(
        error.to_string().contains("receipt") || error.to_string().contains("start"),
        "the refusal names the receipt/start contradiction: {error}"
    );
    assert_eq!(trace_snapshot(&h.trace_dir), before_trace);
    assert_eq!(h.entry(), before_row);
}

#[tokio::test]
async fn a_missing_authoritative_receipt_is_restored_once_across_restarts() {
    let h = support::one_armed_entry().await;
    support::start_entry(&h).await;
    let expected = compact_receipt(&h.entry());
    let receipt_path = h.trace_dir.join(RECEIPT_FILE);
    std::fs::remove_file(&receipt_path).expect("the mirror is lost in the crash window");

    let mut rebuilt = support::try_reopen_runtime(&h)
        .await
        .expect("the authoritative row rebuilds with mirror debt");
    assert!(
        rebuilt.receipt_pending(),
        "the missing mirror is explicit debt before restart mutation"
    );
    rebuilt
        .open_trace()
        .expect("restart restores the mirror before appending its witness");
    assert!(!rebuilt.receipt_pending(), "the restored mirror is ready");
    assert_eq!(std::fs::read(&receipt_path).unwrap(), expected);
    drop(rebuilt);

    let mut second = support::try_reopen_runtime(&h)
        .await
        .expect("the exact restored mirror is accepted on another restart");
    assert!(
        !second.receipt_pending(),
        "an exact existing mirror is ready without another write"
    );
    second.open_trace().expect("the second restart opens");
    assert_eq!(std::fs::read(receipt_path).unwrap(), expected);
}

#[tokio::test]
async fn a_real_daemon_restart_restores_the_registry_receipt_before_rehydration() {
    let h = support::started_entry_with_work().await;
    let expected = compact_receipt(&h.entry());
    let receipt_path = h.trace_dir.join(RECEIPT_FILE);
    std::fs::remove_file(&receipt_path).expect("the mirror is lost before daemon restart");

    let restarted = support::restart_daemon(h).await;
    assert_eq!(std::fs::read(&receipt_path).unwrap(), expected);
    assert_eq!(
        support::read_descriptor(&restarted.trace_dir)
            .receipt
            .as_deref(),
        Some(RECEIPT_FILE),
        "the rehydrated runtime publishes only the restored mirror"
    );
    restarted.daemon.stop().await;
}

#[tokio::test]
async fn exact_bytes_are_required_and_malformed_truncated_and_oversized_mirrors_are_preserved() {
    for planted in [b"{".to_vec(), b"truncated".to_vec()] {
        let h = support::one_armed_entry().await;
        support::start_entry(&h).await;
        h.close();
        std::fs::write(h.trace_dir.join(RECEIPT_FILE), &planted).unwrap();
        assert_rebuild_refuses_without_mutation(&h).await;
        assert_eq!(
            std::fs::read(h.trace_dir.join(RECEIPT_FILE)).unwrap(),
            planted
        );
    }

    let equivalent = support::one_armed_entry().await;
    support::start_entry(&equivalent).await;
    equivalent.close();
    let pretty = serde_json::to_vec_pretty(
        equivalent
            .entry()
            .receipt()
            .expect("the row owns a receipt"),
    )
    .unwrap();
    std::fs::write(equivalent.trace_dir.join(RECEIPT_FILE), &pretty).unwrap();
    assert_rebuild_refuses_without_mutation(&equivalent).await;

    let oversized = support::one_armed_entry().await;
    support::start_entry(&oversized).await;
    oversized.close();
    let too_large = vec![b'x'; goalcup_daemon::MAX_FILE_BYTES as usize + 1];
    std::fs::write(oversized.trace_dir.join(RECEIPT_FILE), &too_large).unwrap();
    assert_rebuild_refuses_without_mutation(&oversized).await;
    assert_eq!(
        std::fs::metadata(oversized.trace_dir.join(RECEIPT_FILE))
            .unwrap()
            .len(),
        goalcup_daemon::MAX_FILE_BYTES + 1
    );
}

#[tokio::test]
async fn persisted_identity_start_deadline_and_binding_mismatches_are_refused() {
    let connector = support::one_armed_entry().await;
    support::start_entry(&connector).await;
    connector
        .registry
        .update(&connector.entry_id, |entry| {
            entry.connector_id = "opencode".to_string()
        })
        .unwrap();
    assert_rebuild_refuses_without_mutation(&connector).await;

    let manifest = support::one_armed_entry().await;
    support::start_entry(&manifest).await;
    manifest
        .registry
        .update(&manifest.entry_id, |entry| {
            entry.manifest_sha256 = "11".repeat(32)
        })
        .unwrap();
    assert_rebuild_refuses_without_mutation(&manifest).await;

    let connector_dir = support::one_armed_entry().await;
    support::start_entry(&connector_dir).await;
    connector_dir
        .registry
        .update(&connector_dir.entry_id, |entry| {
            entry.connector_dir_sha256 = Some("22".repeat(32))
        })
        .unwrap();
    assert_rebuild_refuses_without_mutation(&connector_dir).await;

    let session = support::one_armed_entry().await;
    support::start_entry(&session).await;
    session
        .registry
        .update(&session.entry_id, |entry| {
            entry.session_id = Some("different-session".to_string())
        })
        .unwrap();
    assert_rebuild_refuses_without_mutation(&session).await;

    let start = support::one_armed_entry().await;
    support::start_entry(&start).await;
    start
        .registry
        .update(&start.entry_id, |entry| {
            let started_at = "2026-09-01T00:00:00Z".to_string();
            let deadline_at = goalcup_daemon::receipt::deadline_for(&started_at).unwrap();
            entry.started_at = Some(started_at);
            entry.deadline_unix =
                Some(goalcup_protocol::control::parse_z(&deadline_at).unwrap() as f64);
            entry.deadline_at = Some(deadline_at);
        })
        .unwrap();
    assert_rebuild_refuses_without_mutation(&start).await;

    let deadline = support::one_armed_entry().await;
    support::start_entry(&deadline).await;
    deadline
        .registry
        .update(&deadline.entry_id, |entry| {
            entry.deadline_at = Some("2026-09-30T00:00:00Z".to_string())
        })
        .unwrap();
    assert_rebuild_refuses_without_mutation(&deadline).await;

    let incomplete = support::one_armed_entry().await;
    support::start_entry(&incomplete).await;
    incomplete
        .registry
        .update(&incomplete.entry_id, |entry| entry.session_id = None)
        .unwrap();
    assert_rebuild_refuses_without_mutation(&incomplete).await;

    let run_key = support::one_armed_entry_with_receipt_binding("2026.09-v1", "2026.09-v1").await;
    support::start_entry(&run_key).await;
    run_key
        .registry
        .update(&run_key.entry_id, |entry| {
            entry.keys.signing_seed_hex = "33".repeat(32)
        })
        .unwrap();
    assert_rebuild_refuses_without_mutation(&run_key).await;
}

#[tokio::test]
async fn a_receipt_bound_to_the_previous_valid_genesis_is_refused() {
    let h = support::one_armed_entry().await;
    support::start_entry(&h).await;
    h.close();
    let manifest_bytes = std::fs::read(h.trace_dir.join(MANIFEST_FILE)).unwrap();
    let mut baseline: goalcup_protocol::seal::SealManifest = serde_json::from_slice(
        &std::fs::read(h.trace_dir.join(goalcup_protocol::export::BASELINE_FILE)).unwrap(),
    )
    .unwrap();
    baseline.sealed_at = "2026-09-01T00:00:00Z".to_string();
    std::fs::remove_dir_all(&h.trace_dir).unwrap();
    let mut replacement = TraceExport::create(&h.trace_dir).unwrap();
    let (_, genesis) = replacement.write_baseline(&baseline).unwrap();
    replacement.write_manifest(&manifest_bytes).unwrap();
    drop(replacement);
    h.registry
        .update(&h.entry_id, |entry| {
            entry.chain_seq = Some(genesis.seq);
            entry.chain_head = genesis.hash.clone();
        })
        .unwrap();

    assert_rebuild_refuses_without_mutation(&h).await;
}

#[tokio::test]
async fn legacy_receipt_only_rows_are_never_promoted_to_started_or_mirrored() {
    let h = support::one_armed_entry().await;
    support::start_entry(&h).await;
    h.close();
    std::fs::remove_file(h.trace_dir.join(RECEIPT_FILE)).unwrap();
    h.registry
        .update(&h.entry_id, |entry| {
            entry.state = EntryState::Armed;
            entry.session_id = None;
            entry.started_at = None;
            entry.deadline_at = None;
            entry.deadline_unix = None;
        })
        .unwrap();

    let mut rebuilt = support::try_reopen_runtime(&h)
        .await
        .expect("a signed legacy receipt-only row remains an ambiguous pre-start row");
    assert!(!rebuilt.receipt_pending());
    rebuilt.open_trace().expect("the pre-start runtime reopens");
    assert!(!h.trace_dir.join(RECEIPT_FILE).exists());
    assert_eq!(rebuilt.entry().state, EntryState::Armed);
    assert_eq!(rebuilt.entry().started_at, None);
    assert_eq!(
        support::read_descriptor(&h.trace_dir).receipt,
        None,
        "the unreferenced legacy receipt is not promoted into the trace"
    );
}

#[tokio::test]
async fn a_legacy_receipt_only_row_refuses_even_its_own_unreferenced_mirror() {
    let h = support::one_armed_entry().await;
    support::start_entry(&h).await;
    h.close();
    h.registry
        .update(&h.entry_id, |entry| {
            entry.state = EntryState::Armed;
            entry.session_id = None;
            entry.started_at = None;
            entry.deadline_at = None;
            entry.deadline_unix = None;
        })
        .unwrap();

    assert_rebuild_refuses_without_mutation(&h).await;
}

#[tokio::test]
async fn recovery_rechecks_the_receipt_against_the_current_pinned_server_key() {
    let mut h = support::one_armed_entry().await;
    support::start_entry(&h).await;
    h.close();
    h.config.server_public_key = "44".repeat(32);

    assert_rebuild_refuses_without_mutation(&h).await;
}

#[tokio::test]
async fn no_authoritative_start_refuses_an_existing_receipt_child() {
    let h = support::one_armed_entry().await;
    h.close();
    let planted = b"untrusted receipt bytes";
    std::fs::write(h.trace_dir.join(RECEIPT_FILE), planted).unwrap();
    assert_rebuild_refuses_without_mutation(&h).await;
    assert_eq!(
        std::fs::read(h.trace_dir.join(RECEIPT_FILE)).unwrap(),
        planted
    );
}

#[tokio::test]
async fn a_corroborated_pre_start_session_is_not_an_incomplete_start() {
    let h = support::one_armed_entry().await;
    h.registry
        .update(&h.entry_id, |entry| {
            entry.state = EntryState::StartSignalSeen;
            entry.start_signal_at = Some(h.now());
            entry.session_id = Some(support::SESSION.to_string());
        })
        .unwrap();

    let rebuilt = support::try_reopen_runtime(&h)
        .await
        .expect("the corroborated pre-start row has no authoritative receipt debt");
    assert!(!rebuilt.receipt_pending());
    assert_eq!(rebuilt.entry().state, EntryState::StartSignalSeen);
    assert_eq!(
        rebuilt.entry().session_id.as_deref(),
        Some(support::SESSION)
    );
    assert!(!h.trace_dir.join(RECEIPT_FILE).exists());
}

#[tokio::test]
async fn a_started_row_without_a_registry_receipt_is_refused_as_incomplete() {
    let h = support::one_armed_entry().await;
    let started_at = "2026-09-01T00:00:00Z".to_string();
    let deadline_at = goalcup_daemon::receipt::deadline_for(&started_at).unwrap();
    h.registry
        .update(&h.entry_id, |entry| {
            entry.state = EntryState::Started;
            entry.session_id = Some(support::SESSION.to_string());
            entry.started_at = Some(started_at);
            entry.deadline_unix =
                Some(goalcup_protocol::control::parse_z(&deadline_at).unwrap() as f64);
            entry.deadline_at = Some(deadline_at);
        })
        .unwrap();

    assert_rebuild_refuses_without_mutation(&h).await;
}

#[cfg(unix)]
#[tokio::test]
async fn owed_recovery_uses_the_held_root_after_the_trace_path_is_replaced() {
    let h = support::one_armed_entry().await;
    support::start_entry(&h).await;
    let expected = compact_receipt(&h.entry());
    std::fs::remove_file(h.trace_dir.join(RECEIPT_FILE)).unwrap();
    let mut rebuilt = support::try_reopen_runtime(&h)
        .await
        .expect("the absent mirror becomes debt");

    let detached = h.artifact_dir.join("detached-trace");
    std::fs::rename(&h.trace_dir, &detached).unwrap();
    TraceExport::create(&h.trace_dir).unwrap();
    std::fs::write(h.trace_dir.join("replacement-sentinel"), b"replacement").unwrap();

    rebuilt.open_trace().expect("the held root is reconciled");
    assert_eq!(
        std::fs::read(detached.join(RECEIPT_FILE)).unwrap(),
        expected
    );
    assert!(!h.trace_dir.join(RECEIPT_FILE).exists());
    assert_eq!(
        std::fs::read(h.trace_dir.join("replacement-sentinel")).unwrap(),
        b"replacement"
    );
}

#[tokio::test]
async fn a_child_planted_after_absence_is_preserved_and_a_later_retry_can_restore() {
    let h = support::one_armed_entry().await;
    support::start_entry(&h).await;
    let expected = compact_receipt(&h.entry());
    let receipt_path = h.trace_dir.join(RECEIPT_FILE);
    std::fs::remove_file(&receipt_path).unwrap();
    let mut rebuilt = support::try_reopen_runtime(&h)
        .await
        .expect("the absent mirror becomes debt");
    let planted = b"partial racing child";
    std::fs::write(&receipt_path, planted).unwrap();
    let before = trace_snapshot(&h.trace_dir);
    let before_row = h.entry();

    for _ in 0..2 {
        rebuilt
            .open_trace()
            .expect_err("the contradictory child blocks restart mutation");
        assert_eq!(trace_snapshot(&h.trace_dir), before);
        assert_eq!(h.entry(), before_row);
        assert_eq!(std::fs::read(&receipt_path).unwrap(), planted);
        assert!(rebuilt.receipt_pending());
    }

    std::fs::remove_file(&receipt_path).unwrap();
    rebuilt
        .open_trace()
        .expect("a later absence lets the same retained writer restore and resume");
    assert_eq!(std::fs::read(receipt_path).unwrap(), expected);
}
