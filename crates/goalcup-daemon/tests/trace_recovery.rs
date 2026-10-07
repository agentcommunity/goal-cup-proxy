#![cfg(feature = "v1-compat")]
mod support;

use goalcup_daemon::buffer::Buffer;
use goalcup_protocol::chain::{GENESIS, Link};
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::export::{
    ACKS_FILE, BASELINE_FILE, CHAIN_FILE, DESCRIPTOR_FILE, TraceExport, TraceRoot, manifest_sha256,
};
use goalcup_protocol::provenance::{
    CutoffReachedV1, FinalizationV1, Rfc3339Utc, Sha256Digest, TerminationOutcomeV1,
};
use goalcup_protocol::seal::SealManifest;
use goalcup_protocol::submission::{SubmissionFailureV1, SubmissionOutcomeV1};

fn trace_snapshot(root: &std::path::Path) -> Vec<(std::ffi::OsString, Vec<u8>)> {
    let mut snapshot = std::fs::read_dir(root)
        .expect("the trace directory reads")
        .map(|entry| {
            let entry = entry.expect("a trace entry");
            assert!(entry.file_type().expect("the entry type").is_file());
            (
                entry.file_name(),
                std::fs::read(entry.path()).expect("the trace entry reads"),
            )
        })
        .collect::<Vec<_>>();
    snapshot.sort_by(|left, right| left.0.cmp(&right.0));
    snapshot
}

#[tokio::test]
async fn a_wrong_registry_head_is_refused_before_rebuild_mutates_any_artifact() {
    let h = support::one_armed_entry().await;
    h.close();

    h.registry
        .update(&h.entry_id, |entry| {
            assert_eq!(entry.chain_seq, Some(0));
            entry.chain_head = "ff".repeat(32);
        })
        .expect("the incoherent row is stored");

    std::fs::remove_file(h.trace_dir.join(ACKS_FILE)).expect("acks starts absent");
    let buffer = Buffer::default_path(&h.artifact_dir);
    std::fs::remove_file(&buffer).expect("the buffer starts absent");
    let stale = h
        .trace_dir
        .join(format!("{DESCRIPTOR_FILE}.707070.313.tmp"));
    std::fs::write(&stale, b"owned stale descriptor bytes").expect("the stale temp exists");
    let before = trace_snapshot(&h.trace_dir);

    let rebuilt = support::try_reopen_runtime(&h).await;
    assert!(
        rebuilt.is_err(),
        "a row hash that is not the hash at its stored seq cannot rebuild"
    );
    assert_eq!(trace_snapshot(&h.trace_dir), before);
    assert_eq!(
        std::fs::read(h.trace_dir.join(CHAIN_FILE)).expect("the chain remains"),
        before
            .iter()
            .find(|(name, _)| name == CHAIN_FILE)
            .expect("the snapshot contains the chain")
            .1
    );
    assert!(!h.trace_dir.join(ACKS_FILE).exists());
    assert!(!buffer.exists(), "validation must precede Buffer::open");
}

fn arrange_refusal(
    h: &support::EntryHarness,
) -> (std::path::PathBuf, Vec<(std::ffi::OsString, Vec<u8>)>) {
    h.close();
    let acks = h.trace_dir.join(ACKS_FILE);
    if acks.exists() {
        std::fs::remove_file(acks).expect("acks starts absent");
    }
    let buffer = Buffer::default_path(&h.artifact_dir);
    if buffer.exists() {
        std::fs::remove_file(&buffer).expect("the buffer starts absent");
    }
    std::fs::write(
        h.trace_dir
            .join(format!("{DESCRIPTOR_FILE}.707070.414.tmp")),
        b"owned stale descriptor bytes",
    )
    .expect("the stale temp exists");
    let before = trace_snapshot(&h.trace_dir);
    (buffer, before)
}

async fn assert_rebuild_refuses_without_mutation(h: &support::EntryHarness) {
    let (buffer, before) = arrange_refusal(h);
    assert!(support::try_reopen_runtime(h).await.is_err());
    assert_eq!(trace_snapshot(&h.trace_dir), before);
    assert!(!h.trace_dir.join(ACKS_FILE).exists());
    assert!(!buffer.exists(), "validation must precede Buffer::open");
}

#[tokio::test]
async fn rebuilt_construction_refuses_missing_empty_partial_and_corrupt_chains_without_mutation() {
    let missing = support::one_armed_entry().await;
    missing.close();
    std::fs::remove_file(missing.trace_dir.join(CHAIN_FILE)).expect("the chain is removed");
    assert_rebuild_refuses_without_mutation(&missing).await;

    let empty = support::one_armed_entry().await;
    empty.close();
    std::fs::write(empty.trace_dir.join(CHAIN_FILE), b"").expect("the chain is emptied");
    assert_rebuild_refuses_without_mutation(&empty).await;

    let partial = support::one_armed_entry().await;
    partial.close();
    use std::io::Write as _;
    std::fs::OpenOptions::new()
        .append(true)
        .open(partial.trace_dir.join(CHAIN_FILE))
        .expect("the chain opens")
        .write_all(b"{\"protocol_version\":2")
        .expect("the partial tail is appended");
    assert_rebuild_refuses_without_mutation(&partial).await;

    let corrupt = support::one_armed_entry().await;
    corrupt.close();
    std::fs::write(corrupt.trace_dir.join(CHAIN_FILE), b"not an envelope\n")
        .expect("the chain is corrupted");
    assert_rebuild_refuses_without_mutation(&corrupt).await;
}

#[tokio::test]
async fn every_incoherent_registry_prefix_is_refused_before_mutation() {
    let ahead = support::one_armed_entry().await;
    ahead.close();
    ahead
        .registry
        .update(&ahead.entry_id, |entry| {
            entry.chain_seq = Some(1);
            entry.chain_head = "11".repeat(32);
        })
        .expect("the ahead mirror is stored");
    assert_rebuild_refuses_without_mutation(&ahead).await;

    let no_seq_wrong_hash = support::one_armed_entry().await;
    no_seq_wrong_hash.close();
    no_seq_wrong_hash
        .registry
        .update(&no_seq_wrong_hash.entry_id, |entry| {
            entry.chain_seq = None;
            entry.chain_head = "22".repeat(32);
        })
        .expect("the incoherent empty prefix is stored");
    assert_rebuild_refuses_without_mutation(&no_seq_wrong_hash).await;
}

fn baseline(h: &support::EntryHarness) -> SealManifest {
    serde_json::from_slice(
        &std::fs::read(h.trace_dir.join(BASELINE_FILE)).expect("baseline.json reads"),
    )
    .expect("baseline.json parses")
}

fn install_genesis(
    h: &support::EntryHarness,
    manifest: &SealManifest,
    envelope_entry_id: &str,
    recorded_at: &str,
    alter_claim: impl FnOnce(&mut EventBody),
) {
    h.close();
    std::fs::remove_dir_all(&h.trace_dir).expect("the old trace is removed");
    std::fs::create_dir(&h.trace_dir).expect("the trace is recreated");
    std::fs::write(
        h.trace_dir.join(BASELINE_FILE),
        serde_json::to_vec(manifest).expect("the baseline serializes"),
    )
    .expect("the baseline is installed");
    let mut body = EventBody::Baseline {
        manifest_sha256: manifest_sha256(manifest).expect("the baseline hashes"),
        file_count: u64::try_from(manifest.files.len()).expect("the file count fits"),
    };
    alter_claim(&mut body);
    let mut envelope = Envelope {
        protocol_version: goalcup_protocol::PROTOCOL_VERSION,
        entry_id: envelope_entry_id.to_string(),
        source: "daemon".to_string(),
        recorded_at: recorded_at.to_string(),
        link: Link {
            seq: 0,
            prev: GENESIS.to_string(),
            hash: String::new(),
        },
        body,
        redaction: None,
    };
    envelope.link.hash = envelope
        .expected_hash(GENESIS)
        .expect("the genesis hash computes");
    std::fs::write(
        h.trace_dir.join(CHAIN_FILE),
        serde_json::to_vec(&envelope)
            .expect("the envelope serializes")
            .into_iter()
            .chain(std::iter::once(b'\n'))
            .collect::<Vec<_>>(),
    )
    .expect("the chain is installed");
    h.registry
        .update(&h.entry_id, |entry| {
            entry.chain_seq = Some(0);
            entry.chain_head = envelope.link.hash.clone();
        })
        .expect("the mirror names the installed chain");
}

#[tokio::test]
async fn every_baseline_commitment_identity_and_time_mismatch_is_refused() {
    let wrong_genesis_entry = support::one_armed_entry().await;
    let manifest = baseline(&wrong_genesis_entry);
    install_genesis(
        &wrong_genesis_entry,
        &manifest,
        "another-entry",
        &manifest.sealed_at,
        |_| {},
    );
    assert_rebuild_refuses_without_mutation(&wrong_genesis_entry).await;

    let wrong_manifest_entry = support::one_armed_entry().await;
    let mut manifest = baseline(&wrong_manifest_entry);
    manifest.entry_id = "another-entry".to_string();
    install_genesis(
        &wrong_manifest_entry,
        &manifest,
        &wrong_manifest_entry.entry_id,
        &manifest.sealed_at,
        |_| {},
    );
    assert_rebuild_refuses_without_mutation(&wrong_manifest_entry).await;

    let wrong_workspace = support::one_armed_entry().await;
    let mut manifest = baseline(&wrong_workspace);
    manifest.workspace = "/a/different/workspace".to_string();
    install_genesis(
        &wrong_workspace,
        &manifest,
        &wrong_workspace.entry_id,
        &manifest.sealed_at,
        |_| {},
    );
    assert_rebuild_refuses_without_mutation(&wrong_workspace).await;

    let wrong_time = support::one_armed_entry().await;
    let manifest = baseline(&wrong_time);
    install_genesis(
        &wrong_time,
        &manifest,
        &wrong_time.entry_id,
        "2026-09-04T00:00:01Z",
        |_| {},
    );
    assert_rebuild_refuses_without_mutation(&wrong_time).await;

    let wrong_digest = support::one_armed_entry().await;
    let manifest = baseline(&wrong_digest);
    install_genesis(
        &wrong_digest,
        &manifest,
        &wrong_digest.entry_id,
        &manifest.sealed_at,
        |body| {
            let EventBody::Baseline {
                manifest_sha256, ..
            } = body
            else {
                unreachable!()
            };
            *manifest_sha256 = "33".repeat(32);
        },
    );
    assert_rebuild_refuses_without_mutation(&wrong_digest).await;

    let wrong_count = support::one_armed_entry().await;
    let manifest = baseline(&wrong_count);
    install_genesis(
        &wrong_count,
        &manifest,
        &wrong_count.entry_id,
        &manifest.sealed_at,
        |body| {
            let EventBody::Baseline { file_count, .. } = body else {
                unreachable!()
            };
            *file_count += 1;
        },
    );
    assert_rebuild_refuses_without_mutation(&wrong_count).await;
}

fn append_honest_tail(h: &support::EntryHarness) -> Envelope {
    h.close();
    let mut export = TraceExport::resume(&h.trace_dir).expect("the existing trace resumes");
    let (previous_head_seq, previous_head_hash) = export.head().expect("the baseline head");
    let mut event = Envelope {
        protocol_version: goalcup_protocol::PROTOCOL_VERSION,
        entry_id: h.entry_id.clone(),
        source: "daemon".to_string(),
        recorded_at: "2026-09-04T00:00:01Z".to_string(),
        link: Link {
            seq: 0,
            prev: String::new(),
            hash: String::new(),
        },
        body: EventBody::DaemonRestart {
            at: "2026-09-04T00:00:01Z".to_string(),
            previous_head_seq,
            previous_head_hash,
        },
        redaction: None,
    };
    export
        .append_chain(&mut event)
        .expect("the honest tail is appended");
    event
}

#[tokio::test]
async fn equal_and_lagging_registry_prefixes_resume_and_sync_to_the_actual_tail() {
    let equal = support::one_armed_entry().await;
    equal.close();
    let mut rebuilt = support::try_reopen_runtime(&equal)
        .await
        .expect("an equal prefix rebuilds");
    rebuilt.open_trace().expect("the equal prefix opens");
    let equal_chain = support::read_chain(&equal.trace_dir);
    assert!(equal_chain.len() >= 2);
    assert!(matches!(
        equal_chain[1].body,
        EventBody::DaemonRestart { .. }
    ));
    let equal_row = equal.entry();
    assert_eq!(
        equal_row.chain_seq,
        Some(equal_chain.last().unwrap().link.seq)
    );
    assert_eq!(equal_row.chain_head, equal_chain.last().unwrap().link.hash);

    for forget_seq in [false, true] {
        let lagging = support::one_armed_entry().await;
        let honest_tail = append_honest_tail(&lagging);
        if forget_seq {
            lagging
                .registry
                .update(&lagging.entry_id, |entry| {
                    entry.chain_seq = None;
                    entry.chain_head = GENESIS.to_string();
                })
                .expect("the empty prefix is stored");
        }
        let mut rebuilt = support::try_reopen_runtime(&lagging)
            .await
            .expect("a lagging prefix rebuilds");
        rebuilt.open_trace().expect("the lagging prefix opens");
        let chain = support::read_chain(&lagging.trace_dir);
        assert_eq!(chain[1], honest_tail, "the honest crash tail is preserved");
        assert!(matches!(chain[2].body, EventBody::DaemonRestart { .. }));
        let row = lagging.entry();
        assert_eq!(row.chain_seq, Some(chain.last().unwrap().link.seq));
        assert_eq!(row.chain_head, chain.last().unwrap().link.hash);
    }
}

#[tokio::test]
async fn reindented_equivalent_baseline_content_is_accepted() {
    let h = support::one_armed_entry().await;
    h.close();
    let manifest = baseline(&h);
    std::fs::write(
        h.trace_dir.join(BASELINE_FILE),
        serde_json::to_vec_pretty(&manifest).expect("the pretty baseline serializes"),
    )
    .expect("the baseline is reindented");
    let mut rebuilt = support::try_reopen_runtime(&h)
        .await
        .expect("content-equivalent JSON rebuilds");
    rebuilt.open_trace().expect("the rebuilt trace opens");
}

#[tokio::test]
async fn cutoff_and_terminal_chains_are_refused_before_rebuild_mutation() {
    let cutoff = support::one_armed_entry().await;
    cutoff.close();
    let mut export = TraceExport::resume(&cutoff.trace_dir).expect("the chain resumes");
    let cutoff_body: CutoffReachedV1 = serde_json::from_value(serde_json::json!({
        "receipt_sha256": "44".repeat(32),
        "deadline_at": "2026-09-11T00:00:00Z",
        "pre_cutoff_drain": "complete"
    }))
    .expect("the cutoff is valid");
    export
        .write_cutoff(&cutoff.entry_id, "2026-09-11T00:00:00Z", cutoff_body)
        .expect("the cutoff is appended");
    drop(export);
    assert_rebuild_refuses_without_mutation(&cutoff).await;

    let terminal = support::one_armed_entry().await;
    terminal.close();
    let manifest = baseline(&terminal);
    let mut export = TraceExport::resume(&terminal.trace_dir).expect("the chain resumes");
    export
        .write_seal(&manifest)
        .expect("the legacy seal is appended");
    drop(export);
    assert_rebuild_refuses_without_mutation(&terminal).await;

    let finalized = support::one_armed_entry().await;
    finalized.close();
    let receipt_sha256 = Sha256Digest::try_from("55".repeat(32)).expect("a receipt digest");
    let mut export = TraceExport::resume(&finalized.trace_dir).expect("the chain resumes");
    let cutoff_link = export
        .write_cutoff(
            &finalized.entry_id,
            "2026-09-11T00:00:00Z",
            CutoffReachedV1 {
                receipt_sha256: receipt_sha256.clone(),
                deadline_at: Rfc3339Utc::try_from("2026-09-11T00:00:00Z").expect("a cutoff time"),
                pre_cutoff_drain: goalcup_protocol::provenance::PreCutoffDrainOutcomeV1::Complete,
            },
        )
        .expect("the cutoff is appended");
    let final_link = export
        .write_finalization(
            &finalized.entry_id,
            "2026-09-11T00:00:01Z",
            FinalizationV1::WorkspaceSubmission {
                receipt_sha256,
                termination: TerminationOutcomeV1::AlreadyStopped {
                    checked_at: Rfc3339Utc::try_from("2026-09-11T00:00:01Z")
                        .expect("a termination time"),
                },
                submission: SubmissionOutcomeV1::Incomplete {
                    reason: SubmissionFailureV1::RecoveryAmbiguous,
                },
            },
        )
        .expect("the finalization is appended after its bound cutoff");
    assert_eq!(final_link.seq, cutoff_link.seq + 1);
    drop(export);

    let root = TraceRoot::open(&finalized.trace_dir).expect("the finalized root opens");
    let prepared = TraceExport::prepare_resume_from_root(root)
        .expect("the complete cutoff-plus-finalization chain validates");
    assert_eq!(prepared.head(), Some((final_link.seq, final_link.hash)));
    drop(prepared);
    // Rebuilt scanning encounters the earlier cutoff first and may refuse there. This
    // control proves that a genuine protocol-valid complete finalization chain reaches the
    // daemon's terminal refusal without mutation; it does not isolate the later match arm.
    assert_rebuild_refuses_without_mutation(&finalized).await;
}

#[cfg(unix)]
#[tokio::test]
async fn rebuilt_registration_keeps_the_selected_root_after_its_path_is_exchanged() {
    let h = support::one_armed_entry().await;
    h.close();
    let mut rebuilt = support::try_reopen_runtime(&h)
        .await
        .expect("the held trace rebuilds");
    let detached = h.artifact_dir.join("detached-trace");
    std::fs::rename(&h.trace_dir, &detached).expect("the selected root is renamed");
    std::fs::create_dir(&h.trace_dir).expect("a replacement root is planted");
    std::fs::write(h.trace_dir.join("sentinel"), b"replacement").expect("the sentinel is planted");

    rebuilt.open_trace().expect("the held root opens");
    let chain = support::read_chain(&detached);
    assert!(matches!(chain[1].body, EventBody::DaemonRestart { .. }));
    assert_eq!(
        std::fs::read(h.trace_dir.join("sentinel")).unwrap(),
        b"replacement"
    );
    assert!(!h.trace_dir.join(CHAIN_FILE).exists());
    assert!(!h.trace_dir.join(ACKS_FILE).exists());
    assert!(!h.trace_dir.join(DESCRIPTOR_FILE).exists());
}

/// Rebuild acquires the entry once, before it resumes the trace, reopens the encrypted
/// buffer, restores cursors, or publishes status.  Exchanging the whole registry-visible
/// entry afterwards must leave every one of those mutations in the detached entry.
#[cfg(unix)]
#[tokio::test]
async fn rebuilt_runtime_keeps_trace_buffer_cursors_and_status_in_its_detached_entry() {
    use std::os::unix::fs::symlink;

    let h = support::one_armed_entry().await;
    support::start_entry(&h).await;
    h.close();
    let mut rebuilt = support::try_reopen_runtime(&h)
        .await
        .expect("the complete entry rebuilds and retains its capability");

    let outside = h.config.config_dir.join("outside-rebuilt-entry");
    std::fs::create_dir_all(outside.join("trace")).expect("outside trace directory");
    std::fs::write(outside.join("trace/chain.jsonl"), b"OUTSIDE_CHAIN_SENTINEL")
        .expect("outside chain sentinel");
    std::fs::write(outside.join("buffer.redb"), b"OUTSIDE_BUFFER_SENTINEL")
        .expect("outside buffer sentinel");
    std::fs::write(outside.join("cursors.json"), b"OUTSIDE_CURSORS_SENTINEL")
        .expect("outside cursor sentinel");
    std::fs::write(outside.join("status-line.txt"), b"OUTSIDE_STATUS_SENTINEL")
        .expect("outside status sentinel");
    let detached = h.config.config_dir.join("detached-rebuilt-entry");
    std::fs::rename(&h.artifact_dir, &detached).expect("the retained entry is detached");
    symlink(&outside, &h.artifact_dir).expect("the registry-visible entry is replaced");

    rebuilt.open_trace().expect("the retained trace resumes");
    let detached_cursors_before_poll =
        std::fs::read(detached.join("cursors.json")).expect("the rebuilt cursor is present");
    let detached_buffer_before_poll =
        std::fs::read(detached.join("buffer.redb")).expect("the rebuilt buffer is present");
    std::fs::write(detached.join("status-line.txt"), b"STALE_REBUILT_STATUS\n")
        .expect("the stale detached status is planted");
    support::append_line(
        &h.store_path,
        &serde_json::json!({ "type": "response_item", "text": "after restart" }).to_string(),
    );
    let report = rebuilt
        .poll_once_at(h.now())
        .expect("the retained cursor and buffer accept the next source record");
    assert_eq!(
        report.captured, 1,
        "the known post-restart response record is accepted exactly once"
    );
    assert_ne!(
        std::fs::read(detached.join("cursors.json")).expect("the detached cursor changes"),
        detached_cursors_before_poll,
        "the accepted record advances the retained cursor"
    );
    assert_ne!(
        std::fs::read(detached.join("buffer.redb")).expect("the detached buffer changes"),
        detached_buffer_before_poll,
        "the accepted record advances the retained encrypted buffer"
    );
    rebuilt
        .publish_status(&h.config.config_dir, &h.config.artifact_root, h.now())
        .expect("the retained entry publishes its participant status line");

    assert!(
        support::read_chain(&detached.join("trace"))
            .iter()
            .any(|envelope| matches!(envelope.body, EventBody::DaemonRestart { .. })),
        "the detached retained trace receives its restart link"
    );
    assert!(detached.join("buffer.redb").is_file());
    assert!(detached.join("cursors.json").is_file());
    assert!(
        std::fs::read(detached.join("status-line.txt"))
            .expect("the detached status line reads")
            .starts_with(b"/goal Cup ")
    );
    for (relative, sentinel) in [
        ("trace/chain.jsonl", b"OUTSIDE_CHAIN_SENTINEL".as_slice()),
        ("buffer.redb", b"OUTSIDE_BUFFER_SENTINEL".as_slice()),
        ("cursors.json", b"OUTSIDE_CURSORS_SENTINEL".as_slice()),
        ("status-line.txt", b"OUTSIDE_STATUS_SENTINEL".as_slice()),
    ] {
        assert_eq!(
            std::fs::read(outside.join(relative)).expect("outside sentinel remains"),
            sentinel,
            "rebuilt mutation must not follow swapped {relative}"
        );
    }
}

#[tokio::test]
async fn a_failed_rebuilt_open_cannot_retry_or_append_a_second_restart() {
    let h = support::one_armed_entry().await;
    h.close();
    let before = support::read_chain(&h.trace_dir).len();
    let mut rebuilt = support::try_reopen_runtime(&h)
        .await
        .expect("the trace prepares");
    rebuilt
        .chain_writer_mut()
        .expect("the pending writer is available to the test hook")
        .fail_append_after(1);

    assert!(rebuilt.open_trace().is_err());
    let after_failure = support::read_chain(&h.trace_dir);
    assert_eq!(after_failure.len(), before + 1);
    assert!(matches!(
        after_failure.last().unwrap().body,
        EventBody::DaemonRestart { .. }
    ));
    assert!(rebuilt.open_trace().is_err());
    assert_eq!(support::read_chain(&h.trace_dir), after_failure);
}
