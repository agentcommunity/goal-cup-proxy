//! One entry's ordered poll, over a real workspace, a real chain and a real
//! buffer.
//!
//! Nothing here sleeps and nothing waits on a background task. `EntryRuntime` holds no
//! clock: `poll_once_at(now)` takes the moment, every timer inside the tick reads it, and
//! `support::advance_clock` moves the harness's own `f64`, which `h.now()` reads.
#![cfg(feature = "v1-compat")]

mod support;

use goalcup_protocol::envelope::EventBody;
use goalcup_protocol::events::{HealthState, TailNoticeKind};

fn obstruct_descriptor(h: &support::EntryHarness, name: &str) -> std::path::PathBuf {
    let descriptor = h.trace_dir.join(goalcup_protocol::export::DESCRIPTOR_FILE);
    let saved = h.artifact_dir.join(name);
    std::fs::rename(&descriptor, &saved).expect("the prior descriptor is saved");
    std::fs::create_dir(&descriptor).expect("a directory obstructs descriptor replacement");
    saved
}

fn restore_descriptor(h: &support::EntryHarness, saved: &std::path::Path) {
    let descriptor = h.trace_dir.join(goalcup_protocol::export::DESCRIPTOR_FILE);
    std::fs::remove_dir(&descriptor).expect("the descriptor obstruction is removed");
    std::fs::rename(saved, descriptor).expect("the prior descriptor is restored");
}

#[cfg(unix)]
fn exchange_chain(trace_dir: &std::path::Path) -> std::path::PathBuf {
    let named = trace_dir.join(goalcup_protocol::export::CHAIN_FILE);
    let original = trace_dir.join("held-chain-original");
    std::fs::rename(&named, &original).expect("the held chain is detached");
    std::fs::write(&named, b"replacement chain").expect("the replacement is planted");
    original
}

#[cfg(unix)]
fn restore_chain(trace_dir: &std::path::Path, original: &std::path::Path) {
    let named = trace_dir.join(goalcup_protocol::export::CHAIN_FILE);
    std::fs::remove_file(&named).expect("the replacement is removed");
    std::fs::rename(original, named).expect("the held chain name is restored");
}

/// Fresh construction is exclusive even when the pre-existing final `trace` directory is
/// empty.  A fresh runtime must not adopt that directory as though it had created it: doing
/// so would merge a prior failed run's artifact authority into this run before link zero.
#[tokio::test]
async fn fresh_runtime_refuses_a_preexisting_empty_trace_without_adopting_it() {
    let h = support::fresh_pending_entry().await;
    std::fs::create_dir(&h.trace_dir).expect("the attacker-owned final trace is empty");
    let error = h
        .with(|runtime| runtime.open_trace())
        .expect_err("FreshPending must exclusively create the final trace");

    assert!(
        error
            .to_string()
            .contains("fresh trace cannot be exclusively created"),
        "the shipped fresh trace path names exclusive retained-capability creation: {error}"
    );
    assert!(
        std::fs::read_dir(&h.trace_dir)
            .expect("the pre-existing trace remains readable")
            .next()
            .is_none(),
        "the refused fresh operation must not write a baseline or descriptor into the empty trace"
    );
    assert!(
        h.entry().chain_seq.is_none(),
        "a refused fresh trace creates no registry head that another runtime could adopt"
    );
}

/// Launch evidence is not authority to begin the run until the CLI has handed the
/// spawned process to the daemon.  Without this guard a fast hook/post can cause a
/// receipt-backed `Started` transition before `goalcup start` reaches `/process`.
#[tokio::test]
async fn launch_evidence_cannot_start_before_the_process_handoff() {
    let h = support::one_armed_entry_requiring_process_handoff().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    h.poll();

    assert!(
        !h.with(|runtime| runtime.may_start()),
        "launch evidence alone cannot authorize the irreversible start receipt"
    );
    let row = h.entry();
    assert!(row.started_at.is_none());
    assert!(row.receipt().is_none());
}

#[tokio::test]
async fn a_launch_starts_only_after_registering_its_process_and_attach_needs_none() {
    let launch = support::one_armed_entry_requiring_process_handoff().await;
    let mut child = support::spawn_harness();
    support::append_line(&launch.store_path, &support::session_meta_line(&launch));
    launch.poll();

    let mut runtime = launch.take();
    assert!(
        runtime.bind_start(support::SESSION).await.is_err(),
        "the direct receipt path repeats the poller's pre-handoff refusal"
    );
    runtime
        .set_process(support::process_registration(
            1,
            support::handle_for_now(child.id()),
        ))
        .expect("the CLI's process proof is persisted first");
    assert!(
        runtime.may_start(),
        "the same evidence is now eligible to start"
    );
    runtime
        .bind_start(support::SESSION)
        .await
        .expect("the registered launch starts");
    launch.give_back(runtime);
    assert!(launch.entry().receipt().is_some());
    let _ = child.kill();
    let _ = child.wait();

    let attached = support::one_armed_entry().await;
    support::append_line(&attached.store_path, &support::session_meta_line(&attached));
    attached.poll();
    let mut runtime = attached.take();
    assert!(runtime.may_start(), "attach mode owes no launched process");
    runtime
        .bind_start(support::SESSION)
        .await
        .expect("an attached run starts from the same corroborated evidence");
    attached.give_back(runtime);
    assert!(attached.entry().last_process.is_none());
}

/// `/process` is a handoff, not an eventually consistent mutation: a Busy window leaves no
/// seed evidence and preserves the previous termination authority, then the one unchanged
/// handoff succeeds once the runtime is returned to its slot.
#[tokio::test]
async fn a_busy_process_handoff_mutates_once_after_the_runtime_returns_to_its_slot() {
    use goalcup_protocol::events::HomeSeedRecord;

    let h = support::started_entry_with_work().await;
    h.daemon
        .clear_process(&h.entry_id, 1)
        .expect("the resume-shaped fixture opens the replacement handoff slot");
    let prior_handle = h.entry().last_process.clone();

    let seed = HomeSeedRecord {
        handoff_generation: None,
        handoff_request_digest: None,
        connector: "claude-code-cli".to_string(),
        from: "{home}/.claude.json".to_string(),
        keys: Vec::new(),
        keys_unchanged: Vec::new(),
        written: Vec::new(),
        files: Vec::new(),
        files_kept: Vec::new(),
        files_missing: Vec::new(),
        wire_layout: Default::default(),
    };
    let mut handed = support::handle_for_now(h.child_pid);
    handed.home_seed = Some(seed.clone());
    let mut persisted_handle = handed.clone();
    persisted_handle.home_seed = None;
    let registration = support::process_registration(2, handed.clone());

    // This is exactly the transient window a pump creates. Four fresh POST bodies model
    // the bounded CLI retry and prove a repeated Busy cannot partially chain the seed or
    // claim the replacement process slot.
    let held = h
        .daemon
        .borrow_entry(&h.entry_id)
        .expect("the runtime is held");
    for attempt in 1..=4 {
        let refusal = h
            .daemon
            .register_process(&h.entry_id, registration.clone())
            .expect_err("a held runtime answers Busy");
        assert!(
            matches!(refusal, goalcup_daemon::DaemonError::Busy(_)),
            "attempt {attempt} must remain retryable, not become a generic conflict: {refusal}"
        );
        assert_eq!(
            h.entry().last_process,
            prior_handle,
            "attempt {attempt} must not replace the standing termination authority"
        );
        assert!(
            support::read_chain(&h.trace_dir)
                .iter()
                .all(|envelope| !matches!(envelope.body, EventBody::HomeSeeded(_))),
            "attempt {attempt} must not write HomeSeeded while the runtime is held"
        );
    }
    drop(held);

    h.daemon
        .register_process(&h.entry_id, registration)
        .expect("the same handoff registers after the transient hold ends");
    assert_eq!(
        h.entry().last_process,
        Some(persisted_handle),
        "one successful handoff owns the replacement process slot"
    );
    let seeded: Vec<HomeSeedRecord> = support::read_chain(&h.trace_dir)
        .into_iter()
        .filter_map(|envelope| match envelope.body {
            EventBody::HomeSeeded(record) => Some(record),
            _ => None,
        })
        .collect();
    assert_eq!(seeded.len(), 1, "the handoff chains its seed exactly once");
    let mut seeded = seeded.into_iter().next().expect("one seed marker");
    assert_eq!(seeded.handoff_generation, Some(2));
    assert!(seeded.handoff_request_digest.is_some());
    seeded.handoff_generation = None;
    seeded.handoff_request_digest = None;
    assert_eq!(seeded, seed, "the marker does not alter seed evidence");
}

/// A legacy row did not record whether its original leg launched or attached, but a
/// launchable resume does. Opening the replacement slot therefore changes it to the
/// durable `Required` contract before the CLI can register the relaunch.
#[tokio::test]
async fn legacy_unstarted_handoff_refuses_process_registration() {
    let h = support::one_armed_legacy_handoff_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    h.poll();

    let mut runtime = h.take();
    assert!(
        runtime
            .set_process(support::process_registration(
                1,
                support::recorded_launch_handle()
            ))
            .is_err(),
        "a legacy required/no-phase row cannot accept `/process`"
    );
    h.give_back(runtime);
}

/// A genuine `attach_only` connector has no launch recipe. Its resume refusal must happen
/// before the registry can clear or promote its existing handoff contract.
#[tokio::test]
async fn attach_only_resume_refuses_without_mutating_the_handoff() {
    let h = support::one_armed_attach_only_entry().await;
    let before = h.entry();

    let error = h.with(|runtime| {
        runtime
            .clear_process(0)
            .expect_err("attach-only connectors cannot open a launch replacement slot")
    });

    assert!(error.to_string().contains("attach_only"));
    assert_eq!(
        h.entry(),
        before,
        "a refusal leaves the persisted handoff and process slot untouched"
    );
}

/// D27's bounded crash edge: a launch may have recorded the raw home-seed evidence before
/// the durable handle write. That evidence is not authority to start; a restart or another
/// poll must still see the empty handoff and leave `Started` and the receipt absent.
#[tokio::test]
async fn home_seeded_evidence_without_a_required_handoff_cannot_start() {
    let h = support::one_armed_entry_requiring_process_handoff().await;
    h.with(|runtime| {
        runtime
            .chain_home_seed(goalcup_protocol::events::HomeSeedRecord {
                handoff_generation: None,
                handoff_request_digest: None,
                connector: "codex-cli".to_string(),
                from: "{home}/.codex.json".to_string(),
                keys: Vec::new(),
                keys_unchanged: Vec::new(),
                written: Vec::new(),
                files: Vec::new(),
                files_kept: Vec::new(),
                files_missing: Vec::new(),
                wire_layout: Default::default(),
            })
            .expect("the crash window has raw seed evidence");
    });
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    h.poll();

    let mut runtime = h.take();
    assert!(!runtime.may_start());
    assert!(
        runtime.bind_start(support::SESSION).await.is_err(),
        "the receipt/commit path repeats the no-handoff refusal"
    );
    h.give_back(runtime);
    let row = h.entry();
    assert!(row.started_at.is_none());
    assert!(row.receipt().is_none());
    assert!(
        support::read_chain(&h.trace_dir)
            .iter()
            .any(|envelope| matches!(envelope.body, EventBody::HomeSeeded(_))),
        "the raw evidence survives without granting start authority"
    );
}

/// The crash boundary is after the prepared registry write and the one `HomeSeeded` append,
/// before its encrypted-buffer append and `Registered`.  A rebuilt runtime must find that
/// one non-secret marker, restore its exact envelope to the buffer, and finish without
/// appending another; two matching markers are ambiguous evidence and remain fail-closed in
/// `Prepared` rather than choosing a process authority.  This is a daemon-process
/// crash/restart test, not a sudden-power-loss/fsync-order claim.
#[tokio::test]
async fn prepared_home_seed_recovery_records_exactly_once_or_refuses_ambiguity() {
    use goalcup_daemon::registry::ProcessHandoffPhase;
    use goalcup_protocol::events::{HomeSeedRecord, SeededKey};

    let record = |theme: &str| HomeSeedRecord {
        handoff_generation: None,
        handoff_request_digest: None,
        connector: "codex-cli".to_string(),
        from: "{home}/settings.json".to_string(),
        keys: vec![SeededKey::new("theme", &serde_json::json!(theme))],
        keys_unchanged: Vec::new(),
        written: Vec::new(),
        files: Vec::new(),
        files_kept: Vec::new(),
        files_missing: Vec::new(),
        wire_layout: Default::default(),
    };

    let h = support::one_armed_entry_requiring_process_handoff().await;
    let mut child = support::spawn_harness();
    let mut handle = support::handle_for_now(child.id());
    handle.home_seed = Some(record("dark"));
    let registration = support::process_registration(1, handle);
    let digest = registration.canonical_handle_digest(&h.entry_id);
    h.registry
        .prepare_process_handoff(&h.entry_id, registration.clone())
        .expect("the durable prepared state lands before the chain write");
    let mut marker = record("dark");
    marker.handoff_generation = Some(1);
    marker.handoff_request_digest = Some(digest.clone());
    let buffered_before_marker = h.buffer_pending();
    h.with(|runtime| {
        runtime
            .chain_writer_mut()
            .expect("the writer is open")
            .event(EventBody::HomeSeeded(marker))
            .expect("the injected crash lands the chain marker before Buffer::append");
    });
    assert_eq!(
        h.buffer_pending(),
        buffered_before_marker,
        "the fault injector leaves the chain marker absent from the encrypted buffer"
    );

    let mut rebuilt = support::reopen_runtime(&h).await;
    rebuilt
        .open_trace()
        .expect("the rebuilt daemon resumes its validated trace before recovery work");
    let buffered_before_repair = rebuilt.pending().expect("the reopened buffer counts");
    rebuilt
        .set_process(registration)
        .expect("a rebuilt runtime repairs the one-marker crash boundary");
    assert_eq!(
        rebuilt.pending().expect("the repaired buffer counts"),
        buffered_before_repair + 1,
        "recovery restores the one existing marker to the encrypted buffer"
    );
    h.give_back(rebuilt);
    assert_eq!(
        h.entry().process_handoff_phase(),
        ProcessHandoffPhase::Registered,
        "recovery may finish only the exact prepared registration"
    );
    let _ = child.kill();
    let _ = child.wait();
    let markers: Vec<_> = support::read_chain(&h.trace_dir)
        .into_iter()
        .filter(|envelope| {
            matches!(
                &envelope.body,
                EventBody::HomeSeeded(seed)
                    if seed.handoff_generation == Some(1)
                        && seed.handoff_request_digest.as_deref() == Some(digest.as_str())
            )
        })
        .collect();
    assert_eq!(
        markers.len(),
        1,
        "crash replay did not append a second marker"
    );

    let ambiguous = support::one_armed_entry_requiring_process_handoff().await;
    let mut ambiguous_child = support::spawn_harness();
    let mut handle = support::handle_for_now(ambiguous_child.id());
    handle.home_seed = Some(record("light"));
    let registration = support::process_registration(1, handle);
    let digest = registration.canonical_handle_digest(&ambiguous.entry_id);
    ambiguous
        .registry
        .prepare_process_handoff(&ambiguous.entry_id, registration.clone())
        .expect("the ambiguous crash starts from Prepared");
    for _ in 0..2 {
        let mut marker = record("light");
        marker.handoff_generation = Some(1);
        marker.handoff_request_digest = Some(digest.clone());
        ambiguous.with(|runtime| {
            runtime
                .chain_writer_mut()
                .expect("the writer is open")
                .event(EventBody::HomeSeeded(marker))
                .expect("the fault injector appends a matching chain marker");
        });
    }
    let refusal = ambiguous.with(|runtime| runtime.set_process(registration));
    assert!(refusal.is_err(), "two markers are never guessed through");
    assert_eq!(
        ambiguous.entry().process_handoff_phase(),
        ProcessHandoffPhase::Prepared,
        "ambiguous recovery leaves process authority unregistered"
    );
    let _ = ambiguous_child.kill();
    let _ = ambiguous_child.wait();
}

/// Daemon reconstruction reaches the same Prepared reconciliation path as an exact wire
/// replay. A stale process instance must remain Prepared across restart and must not become
/// the rebuilt runtime's termination authority.
#[tokio::test]
async fn startup_rehydration_never_promotes_a_stale_prepared_process() {
    use goalcup_daemon::registry::ProcessHandoffPhase;

    let h = support::one_armed_entry_requiring_process_handoff().await;
    let mut child = support::spawn_harness();
    let mut handle = support::handle_for_now(child.id());
    handle.process_start = Some("the pre-crash process instance is gone".to_string());
    let registration = support::process_registration(1, handle);
    h.registry
        .prepare_process_handoff(&h.entry_id, registration)
        .expect("the original daemon persisted Prepared");

    let mut rebuilt = support::reopen_runtime(&h).await;
    rebuilt
        .open_trace()
        .expect("startup reopens the validated trace before reconciliation");
    let error = rebuilt
        .reconcile_prepared_process_handoff()
        .expect_err("startup refuses stale process authority");
    assert!(format!("{error}").contains("could not be revalidated"));
    assert_eq!(
        h.registry
            .get(&h.entry_id)
            .unwrap()
            .unwrap()
            .process_handoff_phase(),
        ProcessHandoffPhase::Prepared
    );
    assert_eq!(
        h.registry.get(&h.entry_id).unwrap().unwrap().last_process,
        None
    );
    let _ = child.kill();
    let _ = child.wait();
}

/// Once `Registered` has committed, a lost `204` must be harmless to replay: the registry
/// acknowledges the exact process identity without running the seed reconciliation a second
/// time.  This is the daemon half paired with the CLI's ambiguous-transport retry.
#[tokio::test]
async fn an_exact_registered_process_replay_chains_one_home_seeded_event() {
    use goalcup_daemon::registry::ProcessHandoffPhase;
    use goalcup_protocol::events::{HomeSeedRecord, SeededKey};

    let h = support::one_armed_entry_requiring_process_handoff().await;
    let mut child = support::spawn_harness();
    let mut handle = support::handle_for_now(child.id());
    handle.home_seed = Some(HomeSeedRecord {
        handoff_generation: None,
        handoff_request_digest: None,
        connector: "claude-code-cli".to_string(),
        from: "{home}/.claude.json".to_string(),
        keys: vec![SeededKey::new("theme", &serde_json::json!("dark"))],
        keys_unchanged: Vec::new(),
        written: Vec::new(),
        files: Vec::new(),
        files_kept: Vec::new(),
        files_missing: Vec::new(),
        wire_layout: Default::default(),
    });
    let registration = support::process_registration(1, handle);
    let pending_before = h.buffer_pending();

    h.with(|runtime| {
        runtime
            .set_process(registration.clone())
            .expect("the first process handoff registers");
        runtime
            .set_process(registration.clone())
            .expect("a lost response replays the same registered process");
    });

    let row = h.entry();
    assert_eq!(row.process_handoff_phase(), ProcessHandoffPhase::Registered);
    assert_eq!(
        h.buffer_pending(),
        pending_before + 1,
        "one HomeSeeded envelope reaches the encrypted buffer"
    );
    let seeds: Vec<HomeSeedRecord> = support::read_chain(&h.trace_dir)
        .into_iter()
        .filter_map(|envelope| match envelope.body {
            EventBody::HomeSeeded(seed) => Some(seed),
            _ => None,
        })
        .collect();
    assert_eq!(seeds.len(), 1, "the trace has one HomeSeeded event");
    assert_eq!(seeds[0].handoff_generation, Some(1));
    assert_eq!(
        seeds[0].handoff_request_digest.as_deref(),
        Some(registration.canonical_handle_digest(&h.entry_id).as_str())
    );
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
#[tokio::test]
async fn bind_start_refuses_a_chain_leaf_exchange_and_the_restored_name_stays_poisoned() {
    let h = support::one_armed_entry().await;
    let mut runtime = h.take();
    let original = exchange_chain(&h.trace_dir);

    assert!(runtime.bind_start(support::SESSION).await.is_err());
    restore_chain(&h.trace_dir, &original);
    assert!(
        runtime
            .chain_writer_mut()
            .expect("the open writer remains")
            .independent_chain_reader()
            .is_err(),
        "restoring the original directory entry cannot unpoison its writer"
    );
    h.give_back(runtime);
}

#[cfg(unix)]
#[tokio::test]
async fn seal_readback_refuses_a_chain_leaf_exchange_and_poison_survives_restore() {
    let h = support::one_armed_entry().await;
    let mut runtime = h.take();
    let original = exchange_chain(&h.trace_dir);

    assert!(runtime.seal().is_err());
    assert_eq!(
        runtime.entry().state,
        goalcup_daemon::registry::EntryState::Sealing,
        "a same-process writer identity fault leaves capture durably closed until a fresh \
         restart can validate recovery"
    );
    restore_chain(&h.trace_dir, &original);
    assert!(
        runtime
            .chain_writer_mut()
            .expect("the opened writer remains")
            .independent_chain_reader()
            .is_err(),
        "the failed seal readback permanently poisoned the actual writer"
    );
    h.give_back(runtime);
}

/// D20 in the order the constraint states it: partition, then redact, then hash, then
/// buffer. A foreign row is judged before a hash is taken over it, so it appears nowhere
/// in `chain.jsonl` — not filtered out of one afterwards.
///
/// And the tick drains. A store holding three polls' worth is caught up by one call, not
/// by three: a single-pass tick reads one `MAX_POLL_BYTES` window per store per 250 ms,
/// which is 4 MiB a second and cannot keep up with a harness writing faster than that.
#[tokio::test]
async fn one_poll_captures_partitions_redacts_chains_and_buffers_in_that_order() {
    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    support::append_line(&h.foreign_store_path, &support::foreign_meta_line());

    let r = h.poll();
    assert_eq!(
        (r.captured, r.discarded),
        (1, 1),
        "the foreign row never reached the chain"
    );
    let chain = support::read_chain(&h.trace_dir);
    assert!(
        chain.iter().all(|e| !serde_json::to_string(e)
            .unwrap()
            .contains("/somewhere/else")),
        "D20: discarded before hashing, not filtered afterwards"
    );
    // The chain's first link is the baseline, and the first record above it is the
    // tracked store's. Seq 0 is written by `TraceExport::write_baseline`, whose source is
    // `daemon`.
    assert!(matches!(chain[0].body, EventBody::Baseline { .. }));
    assert_eq!(
        chain
            .iter()
            .find(|e| matches!(e.body, EventBody::Leg1Record { .. }))
            .expect("one record was captured")
            .source,
        "rollout"
    );
    assert_eq!(
        h.buffer_pending(),
        chain.len() as u64,
        "everything chained is buffered"
    );

    let before = h.poll().captured;
    assert_eq!(before, 0, "a quiet store captures nothing");
    support::append_batches(&h.store_path, 3);
    let r = h.poll();
    assert_eq!(
        r.captured,
        support::lines_in_batches(3),
        "more_available was still set and the tick yielded anyway"
    );
    assert_eq!(
        r.polls_capped, 0,
        "three batches is nowhere near the cap, and a tick that did not hit it says so"
    );
    assert_eq!(h.poll().captured, 0, "and it left nothing behind");
    assert_eq!(
        goalcup_daemon::runtime::MAX_POLLS_PER_TICK,
        64,
        "the drain is bounded: one busy store cannot take the whole tick"
    );
}

/// A Codex rollout file has no owner until its first valid
/// `session_meta.payload.cwd`. Bytes before that decision are untrusted participant bytes,
/// so they must leave only discard counts. A later foreign header cannot retroactively
/// remove a prelude that was already chained or queued for upload.
#[tokio::test]
async fn undecided_rollout_bytes_never_reach_the_chain_export_or_upload_queue() {
    const PRELUDE_SECRET: &str = "task3-undecided-prelude-5f4ba39c";
    const MISSING_SECRET: &str = "task3-missing-cwd-20a8e2c1";
    const MALFORMED_SECRET: &str = "task3-malformed-cwd-b536c9ef";

    let h = support::one_armed_entry().await;
    support::append_line(
        &h.foreign_store_path,
        &serde_json::json!({"type":"response_item","text":PRELUDE_SECRET}).to_string(),
    );
    let prelude = h.poll();
    assert_eq!(
        (prelude.captured, prelude.discarded),
        (0, 1),
        "an undecided file discards its secret-bearing prelude"
    );

    support::append_line(
        &h.foreign_store_path,
        &serde_json::json!({
            "type":"session_meta",
            "payload":{"note":MISSING_SECRET}
        })
        .to_string(),
    );
    let missing = h.poll();
    assert_eq!(
        (missing.captured, missing.discarded),
        (0, 1),
        "metadata with no partition value leaves the file undecided"
    );

    support::append_line(
        &h.foreign_store_path,
        &serde_json::json!({
            "type":"session_meta",
            "payload":{"cwd":{"secret":MALFORMED_SECRET}}
        })
        .to_string(),
    );
    let malformed = h.poll();
    assert_eq!(
        (malformed.captured, malformed.discarded),
        (0, 1),
        "a non-string partition value is not a valid deciding value"
    );

    support::append_line(&h.foreign_store_path, &support::foreign_meta_line());
    let foreign = h.poll();
    assert_eq!(
        (foreign.captured, foreign.discarded),
        (0, 1),
        "the first valid header decides the file foreign and is itself discarded"
    );

    let chain_bytes =
        std::fs::read_to_string(h.trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
            .expect("the serialized chain export reads");
    for forbidden in [
        PRELUDE_SECRET,
        MISSING_SECRET,
        MALFORMED_SECRET,
        "/somewhere/else",
    ] {
        assert!(
            !chain_bytes.contains(forbidden),
            "discarded participant bytes reached the serialized chain export: {forbidden}"
        );
    }
    assert_eq!(
        support::leg1_count(&h.trace_dir, "rollout"),
        0,
        "redaction cannot satisfy this assertion: no discarded record has an envelope"
    );
    assert!(
        !h.artifact_dir.join("partition-decisions.json").exists(),
        "restart reconstruction creates no mutable partition-authority sidecar"
    );

    // Inspect the real decrypted redb queue, after the runtime releases its file lock.
    // This is the exact envelope stream the uploader reads, rather than a ciphertext grep.
    h.close();
    let entry = h.entry();
    let queued = {
        let buffer = goalcup_daemon::buffer::Buffer::open(
            &goalcup_daemon::buffer::Buffer::default_path(&h.artifact_dir),
            &entry.keys.buffer_key_hex,
        )
        .expect("the closed runtime's buffer opens");
        buffer
            .read_from(None, 256)
            .expect("the pending upload queue decrypts")
            .envelopes
    };
    let queued_bytes = serde_json::to_string(&queued).expect("queued envelopes serialize");
    for forbidden in [
        PRELUDE_SECRET,
        MISSING_SECRET,
        MALFORMED_SECRET,
        "/somewhere/else",
    ] {
        assert!(
            !queued_bytes.contains(forbidden),
            "discarded participant bytes reached the upload queue: {forbidden}"
        );
    }
    assert!(
        queued
            .iter()
            .all(|env| !matches!(env.body, EventBody::Leg1Record { .. })),
        "the decrypted upload queue contains no Leg1Record envelope for discarded bytes"
    );

    let mut restarted = support::reopen_runtime(&h).await;
    restarted.open_trace().expect("the trace resumes");
    h.give_back(restarted);

    // The cursor resumes at EOF, after the foreign header. The fresh in-memory partitioner
    // must reconstruct that first valid value from the source prefix before it reaches this
    // forged owned header; otherwise the same file re-decides across a daemon restart.
    support::append_line(&h.foreign_store_path, &support::session_meta_line(&h));
    let replayed = h.poll();
    assert_eq!(
        (replayed.captured, replayed.discarded),
        (0, 1),
        "the reconstructed foreign decision discards the forged owned header after EOF"
    );
    assert_eq!(
        support::leg1_count(&h.trace_dir, "rollout"),
        0,
        "the forged header did not create a Leg1Record envelope"
    );

    const OWNED_BODY: &str = "task3-owned-body-0a563ea6";
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    support::append_line(
        &h.store_path,
        &serde_json::json!({"type":"response_item","text":OWNED_BODY}).to_string(),
    );
    let owned = h.poll();
    assert_eq!(
        (owned.captured, owned.discarded),
        (2, 0),
        "an owned deciding record and the owned record after it are captured"
    );
    assert!(
        std::fs::read_to_string(h.trace_dir.join(goalcup_protocol::export::CHAIN_FILE),)
            .expect("the updated chain reads")
            .contains(OWNED_BODY),
        "the positive control is non-vacuous: owned bytes reached the chain"
    );
}

/// Restart reconstruction is an observation of the current source prefix, not durable
/// authorization for a path. If the file generation changes after reconstruction, the
/// tailer's rewrite notice clears the old decision before it offers the replacement's
/// records to the partitioner.
#[tokio::test]
async fn a_replaced_rollout_does_not_inherit_the_old_files_owned_decision() {
    const FOREIGN_REPLACEMENT: &str = "task3-replaced-foreign-body-c8545b43";

    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    assert_eq!(h.poll().captured, 1, "the original file decides owned");
    let before = support::leg1_count(&h.trace_dir, "rollout");

    h.close();
    let mut restarted = support::reopen_runtime(&h).await;
    restarted.open_trace().expect("the trace resumes");
    h.give_back(restarted);

    // Allocate the replacement while the original inode still exists, then move it into
    // the same source path. This makes file identity change deterministic.
    let replacement = h.store_path.with_extension("replacement");
    std::fs::write(
        &replacement,
        format!(
            "{}\n{}\n",
            support::foreign_meta_line(),
            serde_json::json!({"type":"response_item","text":FOREIGN_REPLACEMENT})
        ),
    )
    .expect("the replacement rollout is written");
    std::fs::remove_file(&h.store_path).expect("the owned rollout is removed");
    std::fs::rename(&replacement, &h.store_path).expect("the foreign rollout replaces it");

    let replaced = h.poll();
    assert_eq!(
        (replaced.captured, replaced.discarded),
        (0, 2),
        "the replacement header decides foreign before either replacement record persists"
    );
    assert_eq!(
        support::leg1_count(&h.trace_dir, "rollout"),
        before,
        "neither replacement record created a Leg1Record envelope"
    );
    assert!(
        !std::fs::read_to_string(h.trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
            .expect("the chain reads")
            .contains(FOREIGN_REPLACEMENT),
        "the foreign replacement body never reaches the serialized export"
    );
}

/// The cursor document is writable daemon bookkeeping. A planted path in it must not turn
/// restart reconstruction into an arbitrary-file reader outside the connector's current
/// source template; the affected store takes the ordinary witnessed reread fallback.
#[tokio::test]
async fn a_cursor_path_outside_the_declared_store_cannot_supply_a_restart_decision() {
    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    assert_eq!(h.poll().captured, 1);
    h.close();

    let outside = h.artifact_dir.join("outside-the-rollout-glob.jsonl");
    std::fs::write(&outside, support::foreign_meta_line()).expect("the planted source exists");
    support::replace_file_cursor_path(&h, "rollout", &h.store_path, &outside);

    let mut restarted = support::reopen_runtime(&h).await;
    restarted
        .open_trace()
        .expect("the old artifact still resumes");
    let chain = support::read_chain(&h.trace_dir);
    assert!(
        support::seq_of_reread(&chain, "rollout").is_some(),
        "an out-of-scope cursor path makes the store re-read instead of opening that path"
    );
    let report = restarted
        .poll_once_at(h.now())
        .expect("the declared source re-reads");
    assert_eq!(
        (report.captured, report.skipped_replayed),
        (1, 0),
        "the declared file starts at zero without a stale replay budget"
    );
}

/// A cursor path can remain syntactically inside the trusted glob while an intermediate
/// directory is swapped for a symlink. Even when the mutable cursor is made consistent
/// with the outside target's inode and length, restart must reject the source before it
/// can use the target's sentinel-bearing header as a partition decision.
#[cfg(unix)]
#[tokio::test]
async fn an_intermediate_symlink_cannot_supply_an_outside_restart_decision() {
    use std::os::unix::fs::{MetadataExt, symlink};

    const OUTSIDE_SENTINEL: &str = "task3-outside-ancestor-7ce441e9";
    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    assert_eq!(h.poll().captured, 1);
    h.close();

    let sessions = h.home.join("sessions");
    std::fs::rename(&sessions, h.home.join("sessions-original"))
        .expect("the original declared source tree moves aside");
    let outside_root = h.artifact_dir.join("outside-source-root");
    let outside_day = outside_root.join("2026/09/04");
    std::fs::create_dir_all(&outside_day).expect("the outside date tree exists");
    let outside_file = outside_day.join(h.store_path.file_name().expect("the rollout name"));
    std::fs::write(
        &outside_file,
        format!(
            "{}\n{}\n",
            serde_json::json!({
                "type":"session_meta",
                "payload":{"cwd":h.workspace, "marker":OUTSIDE_SENTINEL}
            }),
            serde_json::json!({"type":"response_item", "text":OUTSIDE_SENTINEL}),
        ),
    )
    .expect("the outside regular source is written");
    let outside_metadata = std::fs::metadata(&outside_file).expect("the outside source stats");
    support::set_file_cursor_facts(
        &h,
        "rollout",
        &h.store_path,
        outside_metadata.ino(),
        outside_metadata.len(),
    );
    symlink(&outside_root, &sessions).expect("the declared source ancestor is redirected");

    let mut restarted = support::reopen_runtime(&h).await;
    restarted.open_trace().expect("the trace resumes safely");
    let chain = support::read_chain(&h.trace_dir);
    assert!(
        support::seq_of_reread(&chain, "rollout").is_some(),
        "the symlinked ancestor must refuse cursor restore before any source poll"
    );
    let leg1_before = support::leg1_count(&h.trace_dir, "rollout");
    let refused = restarted
        .poll_once_at(h.now())
        .expect("the unsafe declared source is refused without reading it");
    assert_eq!(
        (refused.captured, refused.discarded),
        (0, 0),
        "the ordinary zero-cursor poll must not follow the rejected ancestor and learn from outside bytes"
    );
    assert!(
        refused.notices.iter().any(|notice| matches!(
            notice,
            goalcup_daemon::tailer::TailNotice::StoreUnavailable { store, .. }
                if store == "rollout"
        )),
        "the fail-closed source is visible as an unavailable store"
    );
    assert_eq!(
        support::leg1_count(&h.trace_dir, "rollout"),
        leg1_before,
        "neither the forged owned header nor its body creates a Leg1Record"
    );
    assert!(
        !std::fs::read_to_string(h.trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
            .expect("the chain reads")
            .contains(OUTSIDE_SENTINEL),
        "outside reconstruction and poll bytes never enter an envelope"
    );

    // Refusal lasts for this runtime because accepting a changed path through ordinary
    // string-based tailing would reopen the race. Repairing the source and restarting
    // restores normal owned capture.
    drop(restarted);
    std::fs::remove_file(&sessions).expect("the redirected ancestor is removed");
    std::fs::rename(h.home.join("sessions-original"), &sessions)
        .expect("the regular declared source tree is restored");
    let mut repaired = support::reopen_runtime(&h).await;
    repaired.open_trace().expect("the repaired trace resumes");
    let report = repaired
        .poll_once_at(h.now())
        .expect("the regular declared source reads again");
    assert_eq!(
        report.captured, 1,
        "a restart after restoring regular source directories captures the owned control"
    );
}

/// A declared source can be replaced by a FIFO between processes. Restart construction
/// must not block in `open`: a non-regular source cannot reconstruct a decision and the
/// store falls back to a witnessed zero cursor.
#[cfg(unix)]
#[tokio::test]
async fn a_fifo_at_a_saved_rollout_path_cannot_block_restart_reconstruction() {
    use std::os::unix::ffi::OsStrExt;

    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    assert_eq!(h.poll().captured, 1);
    h.close();
    std::fs::remove_file(&h.store_path).expect("the original rollout is removed");
    let fifo = std::ffi::CString::new(h.store_path.as_os_str().as_bytes())
        .expect("the temporary path has no NUL");
    // SAFETY: `fifo` is a NUL-terminated path inside this test's temporary directory.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);

    let started = std::time::Instant::now();
    let mut restarted = support::reopen_runtime(&h).await;
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "opening a FIFO for reconstruction waited for a writer"
    );
    restarted.open_trace().expect("the trace resumes");
    assert!(
        support::seq_of_reread(&support::read_chain(&h.trace_dir), "rollout").is_some(),
        "the non-regular source forces the safe cursor fallback"
    );
}

/// Cursor state can be edited to claim an enormous prefix. Startup reconstruction gets one
/// ordinary poll window across all files; if it cannot find the first valid decider there,
/// the store re-reads visibly through the runtime's already bounded poll loop.
#[tokio::test]
async fn an_unbounded_cursor_prefix_falls_back_before_it_can_stall_restart() {
    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    assert_eq!(h.poll().captured, 1);
    h.close();

    // Keep the inode but replace the old header and make the rest sparse. The forged
    // cursor claims all two windows were already read, with no valid header in either.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&h.store_path)
        .expect("the rollout opens in place");
    file.set_len(goalcup_daemon::MAX_POLL_BYTES * 2)
        .expect("the sparse prefix is sized");
    drop(file);
    support::roll_back_file_cursor(
        &h,
        "rollout",
        &h.store_path,
        goalcup_daemon::MAX_POLL_BYTES * 2,
        0,
    );

    let started = std::time::Instant::now();
    let mut restarted = support::reopen_runtime(&h).await;
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "startup scanned beyond its one-window reconstruction budget"
    );
    restarted.open_trace().expect("the trace resumes");
    assert!(
        support::seq_of_reread(&support::read_chain(&h.trace_dir), "rollout").is_some(),
        "budget exhaustion is visible as the ordinary cursor fallback"
    );
}

/// A restart reconstructs the per-file decision from source bytes below the byte cursor.
/// When the cursor is deliberately rolled back to after the original deciding header, the
/// ordinary record still has its owned verdict and spends exactly its one replay-budget
/// unit. The exhausted budget must not stand over genuinely new owned records.
#[tokio::test]
async fn an_undecided_replay_does_not_consume_new_owned_records_after_late_metadata() {
    const OLD_BODY: &str = "task3-old-owned-body-7cedcf74";
    const NEW_BODY: &str = "task3-new-owned-body-3ef76516";

    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    let after_header = std::fs::metadata(&h.store_path)
        .expect("the rollout exists")
        .len();
    support::append_line(
        &h.store_path,
        &serde_json::json!({"type":"response_item","text":OLD_BODY}).to_string(),
    );
    let first = h.poll();
    assert_eq!(
        (first.captured, first.discarded),
        (2, 0),
        "the original process saw the owned header and captured both records"
    );

    support::roll_back_file_cursor(&h, "rollout", &h.store_path, after_header, 1);
    let mut restarted = support::reopen_runtime(&h).await;
    restarted.open_trace().expect("the trace resumes");
    let replay = restarted
        .poll_once_at(h.now())
        .expect("the rolled-back record is replayed");
    assert_eq!(
        (replay.captured, replay.discarded),
        (0, 0),
        "the restored owned decision lets the replay budget suppress the old body"
    );
    assert_eq!(
        replay.skipped_replayed, 1,
        "the replay budget covers exactly the body above the restored cursor"
    );

    support::append_line(&h.store_path, &support::session_meta_line(&h));
    support::append_line(
        &h.store_path,
        &serde_json::json!({"type":"response_item","text":NEW_BODY}).to_string(),
    );
    let after = restarted
        .poll_once_at(h.now())
        .expect("new owned records are polled");
    assert_eq!(
        (after.captured, after.discarded),
        (2, 0),
        "late owned metadata decides the file and no stale replay budget eats it"
    );
    assert_eq!(
        after.skipped_replayed, 1,
        "the cumulative counter does not advance over genuinely new records"
    );
    h.give_back(restarted);

    let chain_bytes =
        std::fs::read_to_string(h.trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
            .expect("the chain reads");
    assert!(
        chain_bytes.contains(OLD_BODY),
        "the original record was captured"
    );
    assert!(
        chain_bytes.contains(NEW_BODY),
        "the new owned record is a non-vacuous replay-budget control"
    );
}

/// The descriptor is what a verifier opens the trace with, and it is written twice: once
/// when the chain opens, and again when the run is bound to a session.
///
/// A descriptor whose `tracked_sessions` is empty is a `no_session` Fail at the verifier,
/// and V13 reports `receipt_missing` for one that names no receipt beside a chain
/// that has one. The descriptor written before any session was bound is exactly that
/// document, so the only thing that makes a captured run judgeable rather than a
/// written-down Fail is the refresh.
#[tokio::test]
async fn the_descriptor_names_the_bound_session_the_receipt_and_the_seal() {
    let h = support::one_armed_entry().await;
    let opened = support::read_descriptor(&h.trace_dir);
    assert!(
        opened.tracked_sessions.is_empty(),
        "nothing is bound yet, and the descriptor does not invent a session"
    );

    support::start_entry(&h).await;
    let doc = support::read_descriptor(&h.trace_dir);
    assert_eq!(
        doc.tracked_sessions,
        vec![support::SESSION.to_string()],
        "an empty `tracked_sessions` is the verifier's `no_session` Fail, so the refresh is \
         what makes this run judgeable"
    );
    assert_eq!(
        doc.receipt.as_deref(),
        Some("receipt.json"),
        "the receipt is on disk, and a descriptor that does not name it is `receipt_missing`"
    );
    assert_eq!(doc.seal.as_deref(), None, "nothing is sealed yet");

    // Every field the loader needs before it will hand the trace to a check.
    assert_eq!(doc.chain.as_deref(), Some("chain.jsonl"));
    assert_eq!(doc.baseline.as_deref(), Some("baseline.json"));
    assert_eq!(doc.acks.as_deref(), Some("acks.jsonl"));
    assert_eq!(doc.connector_manifest, "manifest.toml");
    assert!(
        h.trace_dir.join(&doc.connector_manifest).exists(),
        "R-D pins the bytes that judged the run, so the file travels with the trace"
    );
    for (what, value) in [
        ("connector", &doc.connector),
        ("entry_id", &doc.entry_id),
        ("workspace", &doc.workspace),
        ("harness_version", &doc.harness_version),
    ] {
        assert!(!value.is_empty(), "the descriptor states its {what}");
    }
    let key = doc
        .server_public_key
        .clone()
        .expect("the pinned server key");
    assert!(
        key.len() == 64 && key.chars().all(|c| c.is_ascii_hexdigit()),
        "the loader refuses anything else"
    );

    h.with(|r| r.seal()).expect("the run seals");
    let sealed = support::read_descriptor(&h.trace_dir);
    assert_eq!(
        sealed.seal.as_deref(),
        Some("seal.json"),
        "the seal is the last link, and the descriptor names the manifest beside it"
    );
    assert_eq!(sealed.receipt.as_deref(), Some("receipt.json"));
    assert_eq!(sealed.tracked_sessions, vec![support::SESSION.to_string()]);
}

/// D19: a tailer that cannot restore re-reads from zero. That is expected and it is not
/// corruption — but a store that starts at offset 0 emits no `Reread` of its own, so
/// without a witness the chain shows a restart followed by a second complete copy of the
/// transcript and nothing saying why. V9 spends the `DaemonRestart` on the first
/// store; the rest read as a replayed transcript.
#[tokio::test]
async fn a_lost_cursor_file_is_witnessed_before_the_stores_are_read_again() {
    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    let r = h.poll();
    assert_eq!(r.captured, 1);
    assert_eq!(
        r.cursor_restore_failures, 0,
        "a fresh entry had no position to lose"
    );

    support::delete_cursors(&h);
    let mut restarted = support::reopen_runtime(&h).await;
    restarted.open_trace().expect("the trace resumes");

    let chain = support::read_chain(&h.trace_dir);
    let restart_at = support::seq_of_restart(&chain);
    for store in ["rollout", "history"] {
        let reread = support::seq_of_reread(&chain, store)
            .unwrap_or_else(|| panic!("a Reread witness for {store}"));
        assert!(
            reread > restart_at,
            "the restart envelope comes first, then what it cost"
        );
        assert!(
            support::first_leg1_seq_above(&chain, store, restart_at).is_none(),
            "and both are chained BEFORE the store's first poll"
        );
    }

    let report = restarted
        .poll_once_at(h.now())
        .expect("the re-reading tick runs");
    assert_eq!(
        report.cursor_restore_failures, 2,
        "one per declared store, counted rather than swallowed"
    );
    let chain = support::read_chain(&h.trace_dir);
    let reread = support::seq_of_reread(&chain, "rollout").expect("the witness");
    let record = support::first_leg1_seq_above(&chain, "rollout", restart_at)
        .expect("the store was read from zero again");
    assert!(
        reread < record,
        "a witness above the record it explains explains nothing"
    );
}

/// The on-disk signature of a rewritten transcript is in the chain, and it raises
/// `StoreReplaced`. A capacity bound the daemon itself hit is a different kind and is not
/// a rewrite.
#[tokio::test]
async fn a_truncated_store_is_chained_as_a_notice_and_a_capacity_bound_is_not_a_rewrite() {
    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    support::start_entry(&h).await;
    support::truncate_store(&h);

    let r = h.poll();
    assert!(r.notices.iter().any(|n| n.is_rewrite()));
    assert_eq!(r.health, HealthState::StoreReplaced);
    let kinds = support::chained_tail_notice_kinds(&h.trace_dir);
    assert!(
        kinds.contains(&TailNoticeKind::Truncated),
        "B-2: the on-disk signature of a rewrite is IN the chain, not only computed"
    );
    assert!(
        !kinds.contains(&TailNoticeKind::OverlongLine),
        "a capacity bound is a different kind, and this run hit none"
    );
}

/// In one run: the baseline is the genesis link, a restart names the head it
/// found before the first re-reading poll, and the heartbeat carries one typed cursor per
/// store.
#[tokio::test]
async fn a_restart_writes_its_restart_first_and_heartbeats_make_downtime_a_statement() {
    let h = support::one_armed_entry().await;
    assert!(
        matches!(
            support::read_chain(&h.trace_dir)[0].body,
            EventBody::Baseline { .. }
        ),
        "V13: seq 0 that is not a Baseline is `baseline_not_chained`, at Fail"
    );
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    h.poll();
    let (seq, head) = h.head();

    let mut restarted = support::reopen_runtime(&h).await;
    restarted.open_trace().expect("the trace resumes");
    let first_new = support::read_chain(&h.trace_dir)[seq as usize + 1].clone();
    match first_new.body {
        EventBody::DaemonRestart {
            previous_head_hash, ..
        } => assert_eq!(previous_head_hash, head),
        other => panic!("a restart must name the head it found, first: {other:?}"),
    }
    assert_eq!(
        support::read_chain(&h.trace_dir)
            .iter()
            .filter(|e| matches!(e.body, EventBody::Baseline { .. }))
            .count(),
        1,
        "two baselines are two answers to what the workspace held at t0"
    );

    support::truncate_store(&h);
    restarted
        .poll_once_at(h.now())
        .expect("the first re-reading tick runs");
    let chain = support::read_chain(&h.trace_dir);
    let restart_at = support::seq_of_restart(&chain);
    let notice_at = support::seq_of_tail_notice(&chain, TailNoticeKind::Truncated);
    assert!(
        restart_at < notice_at,
        "a witness above the notice explains nothing"
    );
    assert!(
        !support::any_leg1_between(&chain, "rollout", restart_at, notice_at),
        "an intervening record of this store spends the witness (attack 6b)"
    );

    support::advance_clock(&h, 61.0);
    restarted
        .poll_once_at(h.now())
        .expect("the heartbeat tick runs");
    let hb = support::last_heartbeat(&h.trace_dir).expect("B-5: absence of heartbeats is evidence");
    assert_eq!(
        hb.stores_seen
            .iter()
            .filter(|c| c.store == "rollout")
            .count(),
        1,
        "gaps::cursor_for takes the FIRST match by name: one cursor per store"
    );
    for store in ["rollout", "history"] {
        let cursor = hb
            .stores_seen
            .iter()
            .find(|c| c.store == store)
            .unwrap_or_else(|| panic!("every store is in the heartbeat: {store}"));
        assert_eq!(
            cursor.unit,
            goalcup_protocol::cursor::CursorUnit::Records,
            "C1 + fix-wave 17 carry-forward: without a Records unit `cursor_mismatch` is \
             unreachable on every store the three M1 connectors declare"
        );
        assert!(cursor.records().is_some());
        assert!(
            !cursor.describe.is_empty(),
            "and the byte offset beside it, parsed by nobody"
        );
    }
    // …and the write-ahead position beside them, so a heartbeat says where the run got to
    // as well as where its stores did. `Opaque`, because a chain sequence is not a count of
    // records in a window and must never reach arithmetic that compares one.
    let buffered = hb
        .stores_seen
        .iter()
        .find(|c| c.store == "buffer")
        .expect("the heartbeat carries the buffer/upload position");
    assert_eq!(buffered.unit, goalcup_protocol::cursor::CursorUnit::Opaque);
    assert_eq!(buffered.records(), None);
    assert!(buffered.describe.contains("pending:"));
    assert!(
        !hb.receiver_ok,
        "no receiver bound a port for this entry, and the heartbeat says so rather than \
         assuming one is up"
    );
}

/// In one assertion: `capture_stalled` is in the trace; a config
/// mismatch is in the trace too, but as the `ConfigDeviation` that explains it.
#[tokio::test]
async fn only_the_health_states_without_an_event_of_their_own_are_chained() {
    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    support::start_entry(&h).await;
    support::stall_capture(&h);
    h.poll();
    support::deviate_tracker_config(&h);
    h.poll();

    let states = support::chained_health_states(&h.trace_dir);
    assert!(states.contains(&HealthState::CaptureStalled));
    assert!(!states.contains(&HealthState::ConfigMismatch));
    assert_eq!(support::chained_config_deviations(&h.trace_dir).len(), 1);
    assert_eq!(
        h.with(|r| r.status()).expect("the status builds").health,
        HealthState::ConfigMismatch,
        "all fifteen still reach the signed cache"
    );
}

/// A file that matches again stops being a mismatch, and the deviation stays chained.
///
/// `config_mismatch` does not latch for the run. The daemon never
/// writes an app-wide file and cannot know a later edit was a revert — but it does not have
/// to know that: it knows the digest is the one it recorded at launch, which is the only
/// thing the flag claims. Latched, one edit would tell the participant to run `goalcup
/// doctor` for the rest of a week-long run, about a file that had been identical for hours.
///
/// What does not move is the evidence. The `ConfigDeviation` that raised the mismatch is in
/// the chain and stays there, and a second edit is a second deviation — an event
/// per change, and this is the state beside those events rather than a replacement for them.
#[tokio::test]
async fn an_app_wide_file_put_back_stops_being_a_mismatch_and_stays_in_the_chain() {
    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    support::start_entry(&h).await;

    support::deviate_tracker_config(&h);
    h.poll();
    assert_eq!(
        h.with(|r| r.status()).expect("the status builds").health,
        HealthState::ConfigMismatch,
    );
    assert_eq!(support::chained_config_deviations(&h.trace_dir).len(), 1);

    support::restore_tracker_config(&h);
    h.poll();
    assert_eq!(
        h.with(|r| r.status()).expect("the status builds").health,
        HealthState::UploadLagging,
        "the digests match again, and the run is back to the state the edit interrupted — \
         this harness never pumps, so that state is the buffer's, not merely `anything but \
         ConfigMismatch`"
    );
    assert_eq!(
        support::chained_config_deviations(&h.trace_dir).len(),
        1,
        "putting a file back is not an event: the deviation that happened is the record"
    );

    // …and a second edit is a second change, which is a second deviation and the state
    // again. A latch could not have told these two moments apart.
    support::deviate_tracker_config(&h);
    h.poll();
    assert_eq!(
        h.with(|r| r.status()).expect("the status builds").health,
        HealthState::ConfigMismatch,
    );
    assert_eq!(support::chained_config_deviations(&h.trace_dir).len(), 2);
}

/// The deadline is `started_at + 168h`, parsed once and stored, so `DeadlineReached`
/// and the automatic seal are reachable at all, and `same_process` is a real check.
#[tokio::test]
async fn the_deadline_is_parsed_once_and_a_hand_started_harness_is_not_the_tracked_one() {
    let h = support::one_armed_entry_requiring_process_handoff().await;
    let mut child = support::spawn_harness();
    h.with(|runtime| {
        runtime
            .set_process(support::process_registration(
                1,
                support::handle_for_now(child.id()),
            ))
            .expect("the launch handoff is registered before the start");
    });
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    support::start_entry(&h).await;

    let e = h.entry();
    let started = goalcup_protocol::control::parse_z(e.started_at.as_ref().expect("started_at"))
        .expect("started_at is an instant");
    assert_eq!(
        e.deadline_unix.expect("a deadline") as u64,
        started + 168 * 3_600,
        "MF-7: once, not per poll"
    );
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        e.receipt().is_some(),
        "R5: the receipt is what makes started_at a fact"
    );
    let deadline = e.deadline_unix.expect("a deadline");
    h.with(|r| {
        assert!(!r.deadline_passed(started as f64 + 1.0));
        assert!(r.deadline_passed(deadline + 1.0));
        r.clear_process(1)
            .expect("the legitimate resume opens the Required handoff slot");
        r.set_process(support::process_registration(
            2,
            support::handle_for(support::spawn_unrelated()),
        ))
        .expect("the handle is recorded");
        assert!(
            !r.same_process(),
            "attack 1c: relaunching by hand without the pins is visible"
        );
    });
}

#[tokio::test]
async fn a_new_event_entry_binds_its_generated_run_key_over_real_http_after_restart() {
    let mut h = support::one_armed_entry_with_receipt_binding("2026.09-v1", "2026.09-v1").await;
    let generated_key = h
        .entry()
        .keys
        .public_key_hex()
        .expect("the generated run key");

    // Ambient config changed before the daemon reopened. The persisted entry policy is
    // the authority, so restart cannot silently downgrade this run to Legacy.
    h.config.receipt_binding = goalcup_daemon::config::ReceiptBindingPolicy::Legacy;
    let mut reopened = support::reopen_runtime(&h).await;
    reopened.open_trace().expect("the trace resumes");
    h.give_back(reopened);

    support::start_entry(&h).await;
    let stored = h.entry();
    assert_eq!(
        stored.receipt_binding(),
        &goalcup_daemon::config::ReceiptBindingPolicy::NewEvent {
            questionnaire_version: "2026.09-v1".to_string()
        }
    );
    let receipt = stored.receipt().expect("the receipt committed");
    assert_eq!(
        receipt.claims.run_public_key.as_deref(),
        Some(generated_key.as_str())
    );
    assert_eq!(
        receipt.claims.questionnaire_version.as_deref(),
        Some("2026.09-v1")
    );
    assert_eq!(stored.state, goalcup_daemon::registry::EntryState::Started);
    assert!(
        h.trace_dir
            .join(goalcup_protocol::export::RECEIPT_FILE)
            .is_file()
    );
}

#[tokio::test]
async fn a_real_service_with_a_different_questionnaire_contract_cannot_start_the_entry() {
    let h = support::one_armed_entry_with_receipt_binding("2026.09-v1", "2026.09-v2").await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    h.poll();
    let before = h.entry();

    let mut runtime = h.take();
    let error = runtime
        .bind_start(support::SESSION)
        .await
        .expect_err("the independently configured service refuses the mismatch")
        .to_string();
    assert!(
        error.contains("400"),
        "the service returned a typed bad request: {error}"
    );
    h.give_back(runtime);

    let after = h.entry();
    assert_eq!(
        after, before,
        "the refused HTTP issue changed no registry field"
    );
    assert!(after.receipt().is_none());
    assert!(after.session_id.is_none());
    assert!(after.started_at.is_none());
    assert!(after.deadline_at.is_none());
    assert!(after.deadline_unix.is_none());
    assert_eq!(after.state, goalcup_daemon::registry::EntryState::Armed);
    assert!(
        !h.trace_dir
            .join(goalcup_protocol::export::RECEIPT_FILE)
            .exists(),
        "a refused receipt has no trace mirror"
    );
}

#[tokio::test]
async fn bind_start_refuses_a_same_id_row_with_replaced_artifact_identity() {
    let h = support::one_armed_entry_with_receipt_binding("2026.09-v1", "2026.09-v1").await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    h.poll();

    // The runtime still holds the Entry that built and opened this trace. Replace the
    // durable row with the same key, receipt policy and legal state but another coherent
    // artifact/trace pair before the real HTTP receipt request. A path snapshot taken by
    // looking the row up only after the response would adopt this replacement and accept.
    let mut replacement = h.entry();
    replacement.artifact_dir = h.artifact_dir.with_file_name("replacement-artifacts");
    replacement.trace_dir = replacement.artifact_dir.join("trace");
    h.registry
        .put(&replacement)
        .expect("the same-id row is replaced");
    let before = h.entry();
    let chain_path = h.trace_dir.join(goalcup_protocol::export::CHAIN_FILE);
    let chain_before = std::fs::read(&chain_path).expect("the original chain reads");

    let mut runtime = h.take();
    let error = runtime
        .bind_start(support::SESSION)
        .await
        .expect_err("the receipt belongs to the runtime's original artifact identity")
        .to_string();
    h.give_back(runtime);
    assert!(
        error.contains("artifact directory"),
        "the transactional path refusal is visible: {error}"
    );

    let after = h.entry();
    assert_eq!(
        after, before,
        "the refusal changed no replacement-row field"
    );
    assert!(after.receipt().is_none());
    assert!(after.session_id.is_none());
    assert!(after.started_at.is_none());
    assert!(after.deadline_at.is_none());
    assert!(after.deadline_unix.is_none());
    assert_eq!(after.state, goalcup_daemon::registry::EntryState::Armed);
    assert_eq!(
        std::fs::read(&chain_path).expect("the original chain still reads"),
        chain_before,
        "a refused start does not rewrite or append the original chain"
    );
    assert!(
        !h.trace_dir
            .join(goalcup_protocol::export::RECEIPT_FILE)
            .exists(),
        "a refused start leaves no receipt mirror in the original trace"
    );
    assert!(
        !replacement
            .trace_dir
            .join(goalcup_protocol::export::RECEIPT_FILE)
            .exists(),
        "a refused start leaves no receipt mirror in the replacement trace"
    );
}

#[tokio::test]
async fn the_control_create_arm_and_tick_path_uses_the_entrys_new_receipt_policy() {
    let h = support::new_event_control_harness("2026.09-v1", "2026.09-v1").await;
    let mut opts = support::workspace_opts("claude-code-cli");
    opts.attach = true;
    let info: goalcup_protocol::control::WorkspaceInfo = h
        .client
        .post_json(goalcup_protocol::control::ROUTE_WORKSPACES, &opts)
        .await;
    assert_eq!(
        h.daemon
            .registry()
            .get(&info.entry_id)
            .unwrap()
            .unwrap()
            .receipt_binding(),
        &goalcup_daemon::config::ReceiptBindingPolicy::NewEvent {
            questionnaire_version: "2026.09-v1".to_string()
        },
        "the control constructor snapshots daemon policy onto the new row"
    );
    let armed = h
        .client
        .post(&goalcup_protocol::control::entry_route(
            &info.entry_id,
            goalcup_protocol::control::SUFFIX_ARM,
        ))
        .await;
    assert_eq!(armed.status(), 204, "{}", armed.body());

    let workspace = std::path::Path::new(&info.workspace);
    let home = std::path::Path::new(&info.harness_home);
    support::append_line(
        &support::transcript_path(home, workspace, support::DAEMON_SESSION),
        &support::transcript_line(workspace, support::DAEMON_SESSION, "the goal"),
    );
    h.daemon
        .tick()
        .await
        .expect("the real daemon polls and binds");

    let stored = h.daemon.registry().get(&info.entry_id).unwrap().unwrap();
    let receipt = stored.receipt().expect("the start receipt committed");
    assert_eq!(stored.state, goalcup_daemon::registry::EntryState::Started);
    assert_eq!(
        receipt.claims.run_public_key.as_deref(),
        Some(info.run_public_key.as_str())
    );
    assert_eq!(
        receipt.claims.questionnaire_version.as_deref(),
        Some("2026.09-v1")
    );
}

#[tokio::test]
async fn the_control_path_leaves_a_new_entry_unstarted_when_service_policy_differs() {
    let h = support::new_event_control_harness("2026.09-v1", "2026.09-v2").await;
    let mut opts = support::workspace_opts("claude-code-cli");
    opts.attach = true;
    let info: goalcup_protocol::control::WorkspaceInfo = h
        .client
        .post_json(goalcup_protocol::control::ROUTE_WORKSPACES, &opts)
        .await;
    let armed = h
        .client
        .post(&goalcup_protocol::control::entry_route(
            &info.entry_id,
            goalcup_protocol::control::SUFFIX_ARM,
        ))
        .await;
    assert_eq!(armed.status(), 204, "{}", armed.body());
    let workspace = std::path::Path::new(&info.workspace);
    support::append_line(
        &support::transcript_path(
            std::path::Path::new(&info.harness_home),
            workspace,
            support::DAEMON_SESSION,
        ),
        &support::transcript_line(workspace, support::DAEMON_SESSION, "the goal"),
    );
    let before = h.daemon.registry().get(&info.entry_id).unwrap().unwrap();
    let before_start = (
        before.receipt().cloned(),
        before.session_id.clone(),
        before.started_at.clone(),
        before.deadline_at.clone(),
        before.deadline_unix,
        before.state,
    );
    let error = h
        .daemon
        .tick()
        .await
        .expect_err("the real service refuses the daemon's different version")
        .to_string();
    assert!(
        error.contains("400"),
        "the refusal came from receipt issuance: {error}"
    );

    let after = h.daemon.registry().get(&info.entry_id).unwrap().unwrap();
    assert_eq!(
        (
            after.receipt().cloned(),
            after.session_id.clone(),
            after.started_at.clone(),
            after.deadline_at.clone(),
            after.deadline_unix,
            after.state,
        ),
        before_start,
        "the failed start transaction wrote none of its six fields"
    );
    assert!(after.receipt().is_none());
    assert!(after.started_at.is_none());
    assert!(after.deadline_at.is_none());
    assert!(after.deadline_unix.is_none());
    assert!(after.session_id.is_none());
    assert_eq!(after.state, goalcup_daemon::registry::EntryState::Armed);
}

// -----------------------------------------------------------------------------------
// The daemon around the entry — the map, the listeners and the seal.
// -----------------------------------------------------------------------------------

/// The seal in the order the rule states it, over a real daemon: the workspace manifest is
/// taken, the seal is chained as the last link, `seal.json` is beside it — and only then
/// is the harness ended.
///
/// The order is not a preference. A kill that ran first would race the harness's own final
/// writes, and a `seal.json` at a participant-named path with nothing in the chain over it
/// is self-asserted, anchored by nothing.
#[tokio::test]
async fn the_seal_lists_the_workspace_chains_itself_and_only_then_kills() {
    let h = support::started_entry_with_work().await; // one written file, one live child
    let pid = h.child_pid;
    let seal = h.daemon.seal_entry(&h.entry_id).unwrap();

    assert!(seal.files.iter().any(|f| f.path == "main.py"));
    assert!(!seal.files.iter().any(|f| f.path.starts_with(".claude/")));
    assert!(
        seal.excluded.contains(&".claude".to_string()),
        "the union the daemon applied, not a subset (protocol N11)"
    );
    assert_eq!(seal, seal.clone().sorted(), "deterministic");

    // Chained, so an acknowledgement over this seq anchors the seal. The
    // envelope carries the digest of seal.json, not a second copy of the manifest.
    let chain = support::read_chain(&h.trace_dir);
    let last = chain.last().expect("a sealed chain is not empty");
    match &last.body {
        goalcup_protocol::envelope::EventBody::Seal {
            manifest_sha256,
            file_count,
            ..
        } => {
            assert_eq!(
                *manifest_sha256,
                goalcup_protocol::export::manifest_sha256(&seal).unwrap()
            );
            assert_eq!(*file_count as usize, seal.files.len());
        }
        other => panic!("the seal must be the chain's LAST link, not only beside it: {other:?}"),
    }
    let on_disk: goalcup_protocol::seal::SealManifest = serde_json::from_str(
        &std::fs::read_to_string(h.trace_dir.join(goalcup_protocol::export::SEAL_FILE)).unwrap(),
    )
    .unwrap();
    assert_eq!(on_disk, seal);

    // Manifest first, then terminate. The registered root is gone afterwards; whether
    // the OS can prove the complete managed tree is asserted per platform below.
    assert!(!goalcup_protocol::process::is_alive(pid));
    assert!(h.daemon.is_sealed(&h.entry_id).unwrap());
    assert_eq!(
        support::read_descriptor(&h.trace_dir).seal.as_deref(),
        Some("seal.json"),
        "the descriptor names the manifest the last link's digest is over"
    );
    let report = h
        .daemon
        .seal_report(&h.entry_id)
        .unwrap()
        .expect("the seal reported what it learned");
    assert!(
        !report.guard_degraded,
        "the registered handle carried an argv, so both halves of the kill guard ran"
    );
    assert_eq!(report.lock_paths, h.manifest.lock_paths);
    let termination = report.termination.expect("the tree was walked");
    if cfg!(target_os = "macos") {
        assert!(
            termination.harness_unverified && !termination.all_gone,
            "seal-time Darwin authority cannot prove the tree's prehistory: {termination:?}"
        );
    } else {
        assert!(
            termination.all_gone,
            "R15's whole point: the seal can name a process that outlived it, and this one did not"
        );
    }
}

/// A same-process trace capability fault is not an unfinished normal run. The failed
/// writer cannot safely regain capture authority, so the daemon keeps the durable intent
/// closed and immediately replaces the stale live status with an explicit quarantine.
#[cfg(unix)]
#[tokio::test]
async fn a_live_seal_writer_fault_stays_quarantined_and_is_visible_to_status_and_entries() {
    let h = support::started_entry_with_work().await;
    let original = exchange_chain(&h.trace_dir);

    assert!(h.daemon.seal_entry(&h.entry_id).is_err());
    let row = h.entry();
    assert_eq!(row.state, goalcup_daemon::registry::EntryState::Sealing);
    assert!(row.sealed_at.is_none());
    let signed = h
        .daemon
        .signed_status(&h.entry_id)
        .expect("the signed quarantine status reads")
        .expect("the failing seal synchronously republishes it");
    assert_eq!(signed.payload.health, HealthState::SealingRecoveryNeeded);
    assert_eq!(signed.payload.seconds_remaining, Some(0));
    let summary = h
        .daemon
        .entries()
        .expect("entries list the durable quarantine")
        .into_iter()
        .find(|entry| entry.entry_id == h.entry_id)
        .expect("the capture-closed row is not invisible");
    assert_eq!(summary.health, HealthState::SealingRecoveryNeeded);

    // Upload cadence must not use the unstarted-row shortcut to evict an unresolved
    // `Sealing` intent. The writer is poisoned, so this may report an upload error, but
    // the durable quarantine and its status owner remain present.
    let _ = h.daemon.pump_all().await;
    assert!(
        h.daemon
            .entries()
            .expect("the sealing runtime remains owned")
            .iter()
            .any(|entry| entry.entry_id == h.entry_id),
        "a capture-closed but unreconciled intent is never evicted as a drained seal"
    );

    restore_chain(&h.trace_dir, &original);
}

/// The other half: nobody has to ask. At `deadline_at` the daemon seals by itself,
/// and the entry that then tries to move into the sealed run's harness home is refused
/// — through `Daemon::arm`, which is the front door the control API uses.
#[tokio::test]
async fn the_deadline_seals_by_itself_and_arming_refuses_an_occupied_home() {
    let h = support::started_entry_with_work().await;
    support::set_deadline_in_the_past(&h);
    assert_eq!(
        h.daemon.seal_due_entries().unwrap(),
        vec![h.entry_id.clone()],
        "RULES: at 168 hours the entry seals automatically, no finish command"
    );
    assert!(h.daemon.is_sealed(&h.entry_id).unwrap());

    // Through the front door this time.
    let second = support::runtime_sharing_harness_home(&h).await;
    h.daemon.register(second).unwrap();
    let e = h.daemon.arm("e2").unwrap_err().to_string();
    assert!(e.contains("already"), "{e}");

    // The seal records a foreign session seen in the bound workspace.
    let sightings = support::chained_foreign_sessions(&h.trace_dir);
    assert!(
        sightings
            .iter()
            .any(|s| s.contains(support::FOREIGN_SESSION)),
        "the second session in this workspace is in the chain: {sightings:?}"
    );
    assert!(
        sightings.iter().all(|s| !s.contains("prompt")),
        "identifiers and paths only"
    );
    assert!(
        sightings
            .iter()
            .all(|s| !s.contains(support::DAEMON_SESSION)),
        "the run's own session is not foreign to itself, and Claude's second foreign glob          covers the very directory its tracked store lives in: {sightings:?}"
    );
}

/// The process-level half: a loopback control listener, and the four cadences driven once.
#[tokio::test]
async fn the_control_listener_is_loopback_and_the_cadences_run() {
    let h = support::started_entry_with_work().await;
    assert!(h.daemon.control_addr().ip().is_loopback());

    // The page answers a browser, which sends no `Authorization` header and has no token
    // to add (`control::is_public_route`). Everything else meets the gate.
    assert_eq!(
        support::control_get(&h, goalcup_protocol::control::ROUTE_UI).await,
        200
    );
    assert_eq!(
        support::control_get(&h, goalcup_protocol::control::ROUTE_HEALTH).await,
        401,
        "an entry list is a directory of the participant's workspaces"
    );
    // The gate asks about the raw path — `req.uri().path()`, exactly as it arrived —
    // so the one percent decode `is_public_route` does is the only one, and a segment that
    // decodes to `..` is refused rather than admitted as literal text.
    for path in [
        "/v0/assets/%2e%2e/entries/e1/seal",
        "/v0/ui/../entries",
        "/v0/assets/%zz",
    ] {
        assert!(!goalcup_protocol::control::is_public_route(path), "{path}");
        assert_eq!(support::control_get(&h, path).await, 401, "{path}");
    }

    // The four cadences, driven once each: a `tokio::time::interval` fires immediately, so
    // one short timeout runs the poll, the pump, the status write and the deadline check —
    // and a cancelled `run_forever` hands its entry back rather than stranding it mid-pump.
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(200),
            h.daemon.run_forever()
        )
        .await
        .is_err(),
        "run_forever runs until the process is stopped"
    );
    assert_eq!(h.daemon.entries().unwrap().len(), 1);
    assert!(h.daemon.signed_status(&h.entry_id).unwrap().is_some());
    assert_eq!(
        h.daemon.current_entry().unwrap().as_deref(),
        Some(h.entry_id.as_str())
    );

    // Two entries, one daemon, no shared state: registering a second entry does not put
    // anything of the first one's in it, and sealing one does not seal the other.
    let second = support::runtime_sharing_harness_home(&h).await;
    h.daemon.register(second).unwrap();
    h.daemon.seal_entry(&h.entry_id).unwrap();
    assert!(!h.daemon.is_sealed("e2").unwrap());
    assert_eq!(
        h.daemon.seal_due_entries().unwrap(),
        Vec::<String>::new(),
        "a sealed entry is not sealed twice, and an entry with no deadline is not due"
    );
    let again = h.daemon.seal_entry(&h.entry_id).unwrap_err().to_string();
    assert!(again.contains("already sealed"), "{again}");
}

/// What a crashed status write left behind is swept at startup.
///
/// A status write is a temp file, a `chmod` and a rename; a daemon killed between the two
/// leaves the temp file, and nothing else ever removes it. The cache directory is what the
/// MCP server and `goalcup status` read, and a half-written document verifies as nothing.
#[tokio::test]
async fn a_starting_daemon_sweeps_the_temp_files_a_killed_one_left_in_its_cache() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let config = support::config_at(&root, "http://127.0.0.1:1");
    let cache = config.config_dir.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    let orphan = cache.join("status-e1.json.4242.0.tmp");
    std::fs::write(&orphan, b"half a doc").unwrap();
    let kept = cache.join("status-e1.json");
    std::fs::write(&kept, b"{}").unwrap();

    let daemon = goalcup_daemon::runtime::Daemon::start(config)
        .await
        .unwrap();
    assert!(!orphan.exists(), "the orphan is swept");
    assert!(kept.exists(), "and the document beside it is not");
    assert!(daemon.control_addr().ip().is_loopback());
    assert_eq!(daemon.entries().unwrap().len(), 0);
    assert_eq!(daemon.current_entry().unwrap(), None);
    assert!(daemon.upload_ok(), "nothing has contradicted it yet");
    assert!(
        !daemon.manifests().is_empty(),
        "the shipped connectors load"
    );
}

// -----------------------------------------------------------------------------------
// The seal's order, the startup lock and reap, and
// the counters that keep the daemon's own loop from failing silently.
// -----------------------------------------------------------------------------------

/// A `lock_paths[]` template this daemon cannot render must not be able to end a run
/// half-way.
///
/// Resolving the paths after `EntryRuntime::seal` had chained the
/// seal link, written `seal.json` and moved the entry to `Sealed`, with
/// a `?`, would leave a connector whose template named a variable the tracker does not supply
/// with the entry sealed, the harness still running with nothing left to end it, and no
/// `SealReport` anywhere saying either. The paths are advice for a later relaunch; they are
/// read off the manifest and this entry's own path variables, so there is nothing to be
/// gained by asking after the seal and a whole ending to lose.
#[tokio::test]
async fn a_seal_whose_lock_paths_will_not_resolve_still_ends_the_harness() {
    let h = support::started_entry_with_work().await;
    let runtime = support::runtime_with_unresolvable_lock_paths(&h).await;
    h.daemon.register(runtime).expect("the entry registers");
    h.daemon
        .arm("e-lock")
        .expect("arming issues the launch handoff generation");
    let mut child = support::spawn_harness();
    let pid = child.id();
    h.daemon
        .register_process(
            "e-lock",
            support::process_registration(1, support::handle_for_now(pid)),
        )
        .expect("the process is registered");

    let seal = h
        .daemon
        .seal_entry("e-lock")
        .expect("a template the daemon cannot render is not a reason to abandon a seal");
    assert!(h.daemon.is_sealed("e-lock").unwrap());
    assert!(
        !goalcup_protocol::process::is_alive(pid),
        "R15: the seal terminates the harness, and a lock-path template is not a veto on it"
    );
    assert!(
        !seal.files.is_empty() || seal.files.is_empty(),
        "the manifest was taken"
    );

    let report = h
        .daemon
        .seal_report("e-lock")
        .unwrap()
        .expect("a seal that ran leaves a report, whatever the lock paths did");
    assert!(report.lock_paths.is_empty(), "nothing resolved");
    let why = report
        .lock_paths_error
        .expect("and the report says why rather than looking like a connector with none");
    assert!(
        why.contains("no_such_variable"),
        "the message names the template variable: {why}"
    );
    // The durable half, for a daemon that restarts after this process dies.
    let stored = h
        .daemon
        .registry()
        .get("e-lock")
        .unwrap()
        .expect("the row")
        .seal_outcome
        .expect("the seal is on the entry, not only in memory");
    assert_eq!(stored.lock_paths_error, Some(why));
    let _ = child.kill();
    let _ = child.wait();
}

/// One daemon per machine (D4), and the refusal happens before the second daemon touches
/// anything the first one owns.
///
/// The startup sweep runs after `Registry::open`. The other order would let
/// a second daemon delete the live temp files of a status write the first
/// one was in the middle of, and only then discover it was not allowed to start. The
/// registry's file lock is what makes a daemon the machine's, so it is taken first and
/// everything destructive waits behind it.
#[tokio::test]
async fn a_second_daemon_is_refused_by_name_and_sweeps_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let config = support::config_at(&root, "http://127.0.0.1:1");
    let first = goalcup_daemon::runtime::Daemon::start(config.clone())
        .await
        .expect("the first daemon starts");

    // A status write in flight, as the first daemon would leave it mid-rename.
    let cache = config.config_dir.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    let in_flight = cache.join("status-e1.json.9999.0.tmp");
    std::fs::write(&in_flight, b"half a doc").unwrap();

    let refused = goalcup_daemon::runtime::Daemon::start(config)
        .await
        .err()
        .expect("a second daemon on one artifact root is refused")
        .to_string();
    assert!(
        refused.contains("another daemon holds the registry"),
        "the refusal says what happened rather than surfacing a redb enum: {refused}"
    );
    assert!(
        in_flight.exists(),
        "and it swept nothing on its way out: the sweep is behind the lock"
    );
    assert!(first.control_addr().ip().is_loopback());
}

/// A daemon can die after the terminal link reached `chain.jsonl` but before its buffer and
/// registry promotion did. The next daemon must complete that exact terminal transition
/// before it reaps the live harness; treating the stale `Sealing` row as open would leave
/// the child running, and treating it as sealed without repairing the buffer would lose the
/// final upload.
#[tokio::test]
async fn a_starting_daemon_finishes_a_crashed_terminal_seal_before_reaping_the_harness() {
    use goalcup_daemon::buffer::Buffer;
    use goalcup_daemon::registry::{Entry, EntryState, Registry, RunKeys, SealingIntent};
    use goalcup_protocol::export::{
        ACKS_FILE, BASELINE_FILE, CHAIN_FILE, MANIFEST_FILE, SEAL_FILE, TraceDescriptorDoc,
        TraceExport,
    };
    use goalcup_protocol::process::ProcessRegistration;
    use goalcup_protocol::seal::SealManifest;

    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let config = support::config_at(&root, "http://127.0.0.1:1");
    let mut child = support::spawn_harness();
    let pid = child.id();

    // The registry and chain as the dead daemon left them, then closed so the successor can
    // own the redb locks. There is intentionally no buffer append or registry promotion.
    {
        let registry = Registry::open(&Registry::default_path(&config.artifact_root))
            .expect("the registry opens");
        let mut entry = Entry::new(
            "e-orphan",
            "claude-code-cli",
            root.join("workspaces/ws-orphan"),
            config.artifact_root.join("artifacts/e-orphan"),
            &"ab".repeat(32),
            "2.1.259",
            goalcup_daemon::config::ReceiptBindingPolicy::Legacy,
            goalcup_daemon::registry::ExpectedProcessHandoff::Required,
            RunKeys::generate(),
        );
        entry.state = EntryState::Armed;
        let manifest = SealManifest {
            entry_id: entry.entry_id.clone(),
            sealed_at: "2026-09-04T00:00:00Z".to_string(),
            workspace: entry.workspace.display().to_string(),
            files: Vec::new(),
            excluded: Vec::new(),
        };
        registry.put(&entry).expect("the row is stored");
        registry
            .open_initial_process_handoff(&entry.entry_id)
            .expect("the armed launch opens its first generation");
        registry
            .prepare_process_handoff(
                &entry.entry_id,
                ProcessRegistration {
                    generation: 1,
                    handle: support::handle_for_now(pid),
                },
            )
            .expect("the durable registered process is staged");
        let entry = registry
            .register_prepared_process_handoff(&entry.entry_id, 1)
            .expect("the durable registered process is finalized");
        let entry = registry
            .update(&entry.entry_id, |row| {
                row.state = EntryState::Started;
                row.started_at = Some("2026-09-04T00:00:00Z".to_string());
                row.session_id = Some("s1".to_string());
            })
            .expect("the historical started row is stored");
        let mut writer = TraceExport::create(&entry.trace_dir).expect("the trace opens");
        writer
            .write_baseline(&manifest)
            .expect("the baseline links");
        let pre_seal_head = writer.head();
        registry
            .begin_sealing(
                &entry.entry_id,
                SealingIntent::new(
                    manifest.clone(),
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
                    pre_seal_head,
                    writer.root_identity().expect("the trace root is held"),
                )
                .expect("the exact terminal identity stages"),
            )
            .expect("the durable sealing intent lands before the terminal writer");
        writer.write_seal(&manifest).expect("the terminal links");
    }
    assert!(goalcup_protocol::process::is_alive(pid), "the orphan is up");

    let daemon = goalcup_daemon::runtime::Daemon::start(config)
        .await
        .expect("the next daemon starts");
    assert!(
        !goalcup_protocol::process::is_alive(pid),
        "the successor promotes the terminal first, then reaps the harness it now durably owns"
    );
    let recovered = daemon.registry().get("e-orphan").unwrap().expect("the row");
    assert_eq!(recovered.state, EntryState::Sealed);
    assert!(recovered.sealing_from_state.is_none());
    let buffer = Buffer::open(
        &Buffer::default_path(&recovered.artifact_dir),
        &recovered.keys.buffer_key_hex,
    )
    .expect("the repaired terminal buffer opens");
    let buffered = buffer.read_from(Some(0), 8).expect("the buffer reads");
    assert_eq!(buffered.envelopes.len(), 1);
    assert!(matches!(
        buffered.envelopes[0].body,
        goalcup_protocol::envelope::EventBody::Seal { .. }
    ));
    let outcome = recovered
        .seal_outcome
        .expect("the reap cost is written down");
    let expected = if cfg!(target_os = "macos") {
        "all_gone=false"
    } else {
        "all_gone=true"
    };
    assert!(
        outcome
            .termination
            .as_deref()
            .is_some_and(|t| t.contains(expected)),
        "the summary names the platform's defensible outcome: {:?}",
        outcome.termination
    );
    assert_eq!(outcome.termination_error, None);
    let _ = child.kill();
    let _ = child.wait();
}

/// A sealed entry is refused before the daemon creates anything for it.
///
/// Every append on a sealed chain is `ExportError::Sealed`, so a sealed entry taken
/// into the map is a run that cannot record. The refusal has to come before the side
/// effects: `register` creates the entry's harness home, and a directory left on a path this
/// daemon then refuses to serve is a launch target for a harness nobody is watching.
#[tokio::test]
async fn registering_a_sealed_entry_is_refused_before_any_directory_is_created() {
    let h = support::started_entry_with_work().await;
    let (runtime, home) = support::sealed_runtime(&h).await;
    let refused = h.daemon.register(runtime).unwrap_err().to_string();
    assert!(
        refused.contains("sealed"),
        "the refusal names the reason: {refused}"
    );
    assert!(
        !home.exists(),
        "and nothing was created on the way to it: the check is above `create_dir`"
    );
    assert!(
        h.daemon
            .entries()
            .unwrap()
            .iter()
            .all(|e| e.entry_id != "e-sealed"),
        "a refused registration leaves no entry"
    );
}

/// A cadence that fails is counted, and the last failure reaches the participant in words
/// that carry no path.
///
/// `run_forever` cannot exit on a transient — a daemon that did would take the capture with
/// it — and this crate has no logger by design, because the receiver handles a bearer. So a
/// poll that failed every cycle would be indistinguishable from a quiet run on every
/// surface a participant has. The counters are that difference.
///
/// The message is path-free because the status route is unauthenticated and the
/// status line is rendered into a terminal a screen recording may catch.
#[tokio::test]
async fn a_failing_cadence_is_counted_and_the_status_says_so_without_a_path() {
    let h = support::started_entry_with_work().await;
    support::break_the_cursor_file(&h);

    let failed = h.daemon.tick().await.unwrap_err().to_string();
    assert!(failed.contains("io"), "the tick really failed: {failed}");
    assert_eq!(h.daemon.cycle_errors().tick, 1);
    assert_eq!(
        h.daemon.cycle_errors().upload,
        0,
        "per cadence, not one total"
    );

    h.daemon
        .refresh_statuses()
        .expect("the status still writes");
    let payload = h
        .daemon
        .signed_status(&h.entry_id)
        .unwrap()
        .expect("a signed status")
        .payload;
    assert_eq!(payload.cycles.tick, 1);
    let last = payload
        .cycles
        .last
        .expect("the document says what the failure was");
    assert!(
        !last.contains('/'),
        "nothing in a status document names a path in the participant's home: {last}"
    );
    assert!(
        !last.contains(&h.artifact_root.display().to_string()),
        "and least of all the artifact root: {last}"
    );
}

/// A clean run's status carries no cycle-error line.
///
/// A pass that meets the entry out of its slot for the length of one asynchronous call is
/// the daemon's own scheduling and is the transient the next pass takes. Counted, it would
/// never come down — the counters are monotonic and `last` is never cleared — so an honest
/// capture would tell its participant for its whole life that the tracker had lost
/// something.
///
/// The pass still fails and still returns its error to whoever drove it; what it stops
/// doing is writing that error onto the participant's surface.
#[tokio::test]
async fn the_daemons_own_contention_is_not_a_cycle_error() {
    let h = support::started_entry_with_work().await;
    // Exactly the state a pump is in: the runtime is out of the map for the length of one
    // asynchronous call, and any pass that arrives inside it gets `Busy`.
    let held = h.daemon.borrow_entry(&h.entry_id).expect("borrowed");
    let failed = h
        .daemon
        .tick()
        .await
        .expect_err("the pass could not take the entry");
    assert!(
        matches!(failed, goalcup_daemon::DaemonError::Busy(_)),
        "the contention this test is about, not some other failure: {failed}"
    );
    drop(held);

    let cycles = h.daemon.cycle_errors();
    assert_eq!(
        (cycles.tick, cycles.upload, cycles.status, cycles.deadline),
        (0, 0, 0, 0),
        "{cycles:?}"
    );
    assert_eq!(
        cycles.last, None,
        "and no last-error line either, which is the half a participant reads: {cycles:?}"
    );

    // The control: a failure that is not the daemon's own contention still counts, so this
    // is a narrowing of what is reported and not a silencing of it.
    support::break_the_cursor_file(&h);
    h.daemon.tick().await.expect_err("the io failure stands");
    assert_eq!(h.daemon.cycle_errors().tick, 1);
}

/// What the seal reports beyond the manifest, on a connector that actually has it.
///
/// The first half is not vacuous here: Claude ships `lock_paths = []`, so
/// `report.lock_paths == manifest.lock_paths` would compare an empty list to an empty list. Codex
/// declares `{config_dir}/thread-writer-locks/{session_id}.lock` — the per-thread file
/// — and a codex resume that cannot take that lock silently becomes a new thread,
/// a run whose second half is chained under a session nobody bound. The seal is where the run
/// last knew where its lock was, so both tokens have to arrive filled in from this entry's
/// own variables and none of them left as a template.
///
/// The second half is the kill guard. It has two questions — the pid's age, and the program
/// `ps` names among its arguments — and a handle with no argv leaves only the first. The
/// seal still signals; it says it signalled on the weaker guard.
#[tokio::test]
async fn the_seal_resolves_declared_lock_paths_and_reports_a_degraded_guard() {
    let h = support::started_entry_with_work().await;
    let (codex, codex_home) = support::codex_runtime(&h).await;
    h.daemon.register(codex).expect("a codex entry registers");

    h.daemon.seal_entry("e-codex").expect("it seals");
    let report = h.daemon.seal_report("e-codex").unwrap().expect("a report");
    assert_eq!(report.lock_paths_error, None);
    assert_eq!(
        report.lock_paths,
        vec![format!(
            "{}/thread-writer-locks/{}.lock",
            codex_home.display(),
            support::DAEMON_SESSION
        )],
        "resolved through THIS entry's own path variables, not left as a template"
    );
    assert!(
        report.lock_paths.iter().all(|p| !p.contains('{')),
        "a template that reached a report is a path nobody can take"
    );

    // …and the guard, on the entry that has a live harness to end. The Required handoff
    // slot is first opened through the same resume transition a real relaunch uses, then
    // this fixture supplies the no-argv replacement that its seal must report as degraded.
    let mut borrowed = h
        .daemon
        .borrow_entry(&h.entry_id)
        .expect("the entry borrows");
    borrowed
        .get()
        .expect("the runtime is there")
        .clear_process(1)
        .expect("the Required resume slot opens");
    borrowed
        .get()
        .expect("the runtime is there")
        .set_process(support::process_registration(
            2,
            support::handle_without_argv(h.child_pid),
        ))
        .expect("the handle is recorded");
    drop(borrowed);
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let report = h
        .daemon
        .seal_report(&h.entry_id)
        .unwrap()
        .expect("a report");
    assert!(
        report.guard_degraded,
        "only the age half of the kill guard could run, and the seal says so"
    );
    assert!(!goalcup_protocol::process::is_alive(h.child_pid));
    assert!(
        h.daemon
            .registry()
            .get(&h.entry_id)
            .unwrap()
            .expect("the row")
            .seal_outcome
            .expect("persisted")
            .guard_degraded
    );
}

/// The document on disk says sealed the moment the chain closes, and says
/// it again with the final numbers when the last chunk is acknowledged.
///
/// `goalcup status` falls back to `<config_dir>/cache/status-<entry>.json` whenever the
/// daemon is not answering, which is what a participant reads about a run they have just
/// finished. The seal's publication must not look its entry up in the live map, which a
/// concurrent pump may already have emptied while the kill was sleeping out its grace. And
/// the seal is not the last word on the numbers: its own chain link has still to be
/// uploaded, so the document is written again when the last chunk is acknowledged.
///
/// No `refresh_statuses` anywhere below. A cadence would write the same document a second
/// later and hide both.
#[tokio::test]
async fn the_document_on_disk_says_sealed_at_the_seal_and_again_when_it_drains() {
    let h = support::started_entry_with_work().await;
    let on_disk = || {
        let path = h
            .config
            .config_dir
            .join("cache")
            .join(format!("status-{}.json", h.entry_id));
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        serde_json::from_str::<goalcup_protocol::control::SignedStatus>(&text)
            .expect("the cache file is a signed status")
            .payload
    };

    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let sealed = on_disk();
    assert_eq!(sealed.health, HealthState::Sealed, "{sealed:?}");
    assert!(
        sealed.sealed_at.is_some(),
        "the moment the chain closed, on the file `goalcup status` falls back to"
    );

    // The seal's own link is still unsent, which is why a document published only at the
    // seal is not the one the run finishes on.
    assert!(
        sealed.events_pending > 0,
        "the seal link is chained and not yet acknowledged: {sealed:?}"
    );

    // The drain that acknowledges it, and the eviction that follows.
    h.daemon.pump_all().await.expect("the last chunk goes");
    assert!(
        h.daemon
            .entries()
            .expect("the rows")
            .iter()
            .all(|e| e.entry_id != h.entry_id),
        "a drained sealed entry leaves the live map"
    );
    let final_doc = on_disk();
    assert_eq!(final_doc.health, HealthState::Sealed, "{final_doc:?}");
    assert_eq!(
        final_doc.events_pending, 0,
        "the frozen document is the one the run finished on: {final_doc:?}"
    );
    assert!(
        final_doc.events_uploaded >= sealed.events_uploaded,
        "and it counts the chunk the seal was still waiting on: {final_doc:?}"
    );
}

/// `harness_unverified` reaches the status document, not only `SealReport` — an
/// in-memory value this process drops when it dies — and `SealOutcome.termination`'s
/// summary string, which nothing parses. Without it the
/// one surface a participant reads about a finished run would say nothing about the case it
/// exists for: the seal completed, the chain closed, and the agent may still be editing.
///
/// A handle whose argv `ps` cannot corroborate is what the guard refuses, so nothing in the
/// tree was signalled by number and no claim about it may be made.
#[tokio::test]
async fn a_seal_that_cannot_vouch_for_the_harness_says_so_on_the_status_document() {
    let h = support::started_entry_with_work().await;
    let payload_now = || {
        h.daemon
            .signed_status(&h.entry_id)
            .expect("the cache reads")
            .expect("a document")
            .payload
    };
    assert!(
        !payload_now().harness_unverified,
        "a run that has not sealed has no ending to be unsure about"
    );

    // This is a resumed Required launch whose replacement argv no longer corroborates the
    // harness. The explicit slot opening keeps the fixture inside the real handoff rule.
    let mut borrowed = h
        .daemon
        .borrow_entry(&h.entry_id)
        .expect("the entry borrows");
    let mut handle = support::handle_for_now(h.child_pid);
    handle.argv = vec!["nonexistent-program".to_string(), "--serve".to_string()];
    borrowed
        .get()
        .expect("the runtime is there")
        .clear_process(1)
        .expect("the Required resume slot opens");
    borrowed
        .get()
        .expect("the runtime is there")
        .set_process(support::process_registration(2, handle))
        .expect("the handle is recorded");
    drop(borrowed);

    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let report = h
        .daemon
        .seal_report(&h.entry_id)
        .unwrap()
        .expect("a report");
    assert!(
        report.harness_unverified,
        "the guard refused the pid: nothing was signalled by number"
    );
    assert!(
        goalcup_protocol::process::is_alive(h.child_pid),
        "the case is a harness the seal did NOT end, and this test is about saying so"
    );

    assert!(
        payload_now().harness_unverified,
        "the document published under the seal's own hold carries it: a participant reading \
         a finished run is told the daemon cannot say the agent stopped when the chain did"
    );
    assert!(
        h.daemon
            .registry()
            .get(&h.entry_id)
            .unwrap()
            .expect("the row")
            .seal_outcome
            .expect("persisted")
            .harness_unverified,
        "and it is durable, so the next process reads the same answer this one published"
    );
}

/// The same case as the test above, but with the runtime's own
/// `note_seal_outcome` call swallowed — standing in for a concurrent pump that evicted the
/// drained entry in the gap `Daemon::seal_entry` leaves between writing the registry row and
/// making that call. What is under test is `EntryRuntime::status_at`'s fallback to the row:
/// without it, the document `Daemon::seal_entry` publishes under its own hold reads
/// `harness_unverified: false` off a runtime whose in-memory copy was never updated, while
/// the row it wrote a moment earlier already says `true`.
#[tokio::test]
async fn a_swallowed_note_seal_outcome_still_matches_the_row_on_the_status_document() {
    let h = support::started_entry_with_work().await;
    let payload_now = || {
        h.daemon
            .signed_status(&h.entry_id)
            .expect("the cache reads")
            .expect("a document")
            .payload
    };
    assert!(
        !payload_now().harness_unverified,
        "a run that has not sealed has no ending to be unsure about"
    );

    // The same resumed-Required guard-refuses-the-pid setup as the test above, so this
    // seal's `SealOutcome` carries `harness_unverified: true` for the row to hold.
    let mut borrowed = h
        .daemon
        .borrow_entry(&h.entry_id)
        .expect("the entry borrows");
    let mut handle = support::handle_for_now(h.child_pid);
    handle.argv = vec!["nonexistent-program".to_string(), "--serve".to_string()];
    borrowed
        .get()
        .expect("the runtime is there")
        .clear_process(1)
        .expect("the Required resume slot opens");
    borrowed
        .get()
        .expect("the runtime is there")
        .set_process(support::process_registration(2, handle))
        .expect("the handle is recorded");
    drop(borrowed);

    // The one difference from the test above: `note_seal_outcome` never reaches the
    // runtime, exactly as it would not for real if a concurrent pump had already taken the
    // entry out of the map when `seal_entry` tried to mirror the row into it.
    h.daemon.skip_next_note_seal_outcome();
    h.daemon.seal_entry(&h.entry_id).expect("it seals");

    let report = h
        .daemon
        .seal_report(&h.entry_id)
        .unwrap()
        .expect("a report");
    assert!(
        report.harness_unverified,
        "the guard refused the pid: nothing was signalled by number"
    );
    let row_says = h
        .daemon
        .registry()
        .get(&h.entry_id)
        .unwrap()
        .expect("the row")
        .seal_outcome
        .expect("persisted")
        .harness_unverified;
    assert!(
        row_says,
        "the row is written before the swallowed call, so it carries the seal's answer \
         regardless of what reached the runtime"
    );

    assert_eq!(
        payload_now().harness_unverified,
        row_says,
        "the published document must match the row even though this runtime's own \
         `note_seal_outcome` call never landed"
    );
}

/// The double-seal guard is checked under the slot lock, not before taking it.
///
/// Reading `is_sealed` and then taking the lock is a check the winner of a race passes
/// twice: both callers read "not sealed", both take the lock in turn, and the loser reaches
/// `EntryRuntime::seal` and meets `ExportError::Sealed` from the export instead of the
/// friendly refusal this method exists to give. Holding the entry is how a test stands in
/// for the winner: the refusal it produces has to be "this call cannot have the runtime",
/// never a seal.
#[tokio::test]
async fn the_double_seal_guard_is_taken_under_the_lock() {
    let h = support::started_entry_with_work().await;
    let held = h.daemon.borrow_entry(&h.entry_id).expect("borrowed");
    let refused = h.daemon.seal_entry(&h.entry_id).unwrap_err().to_string();
    assert!(
        refused.contains("mid-upload"),
        "the guard and the seal are one hold of the lock: {refused}"
    );
    drop(held);
    assert!(
        !h.daemon.is_sealed(&h.entry_id).unwrap(),
        "and nothing sealed"
    );

    h.daemon.seal_entry(&h.entry_id).expect("now it seals");
    let again = h.daemon.seal_entry(&h.entry_id).unwrap_err().to_string();
    assert!(again.contains("already sealed"), "{again}");
}

// ---------------------------------------------------------------------------------------
// The foreground binary.
// ---------------------------------------------------------------------------------------

/// `--print-addresses` answers about the daemon that is running, and does not become
/// one.
///
/// It does not call `Daemon::start`: a second registry handle, a second pair of listeners and
/// a second `control.json` would make the one moment a doctor needs it, with a daemon up
/// and something wrong, the one moment it could not run at all. `Registry::open` takes a
/// file lock, and the running daemon holds it. The three facts live in
/// `control.json`, which the running daemon wrote after both listeners bound, so they are
/// read rather than re-derived.
///
/// This spawns the built binary, so it needs `cargo build --offline` to have produced one;
/// `CARGO_BIN_EXE_goalcup-daemon` makes that the test harness's job rather than the
/// reader's, and the test stays in the ordinary suite.
#[tokio::test]
async fn print_addresses_reads_the_control_file_and_never_takes_the_registry_lock() {
    let t = tempfile::tempdir().unwrap();
    let cfg = support::write_config(t.path());
    let printed = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_goalcup-daemon"))
            .arg("--config")
            .arg(&cfg)
            .arg("--print-addresses")
            .output()
            .expect("the binary runs")
    };

    // Nothing is listening. The file that would say where a daemon is does not exist, and
    // that is the answer a doctor is asking for — named, on stderr, with a non-zero exit,
    // rather than a daemon this flag started on the spot and then threw away.
    let out = printed();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("control.json"), "{err}");
    assert!(
        String::from_utf8_lossy(&out.stdout).is_empty(),
        "no addresses are invented for a daemon that is not there"
    );

    // …and with one up, holding the registry lock this flag must not want.
    let config = goalcup_daemon::config::DaemonConfig::load(&cfg).expect("the config loads");
    let config_dir = config.config_dir.clone();
    let daemon = goalcup_daemon::runtime::Daemon::start(config)
        .await
        .expect("the daemon starts");

    let out = printed();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines.len(), 2, "{s}");
    assert_eq!(
        lines[0],
        daemon.control_addr().to_string(),
        "where a client connects, as the running daemon bound it"
    );
    assert!(lines[1].ends_with("control.json"), "{s}");

    // The file it read holds the CLI's bearer. Reading it is not printing it.
    let file = goalcup_daemon::control::read_control_file(&config_dir).expect("control.json");
    assert!(
        !s.contains(&file.token),
        "the bearer is not one of the facts"
    );
}

/// A configuration the binary refuses is an exit code and a sentence on stderr, not a
/// panic and not a zero.
///
/// The refusal itself is `DaemonConfig::validate`'s (`control_addr` off loopback), and the
/// point here is the binary's half: it names the file it could not use, so a participant
/// meets the path rather than an unattributed complaint.
#[test]
fn a_config_the_daemon_refuses_is_a_nonzero_exit_and_a_sentence() {
    let t = tempfile::tempdir().unwrap();
    let cfg = support::write_config_with(t.path(), |text| {
        text.replace("127.0.0.1:0", "10.1.2.3:9999")
    });
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup-daemon"))
        .arg("--config")
        .arg(&cfg)
        .arg("--print-addresses")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("loopback"), "{err}");
    assert!(err.contains(&cfg.display().to_string()), "{err}");
}

/// SIGTERM stops the process and seals nothing.
///
/// The default disposition of SIGTERM already terminates a process, so a binary with no
/// handler at all would still "stop" — and would be reported as killed by signal 15 rather
/// than as having exited. Asserting `code() == Some(0)` is what tells the two apart, and it
/// is the whole difference between a daemon that ended and one that was killed.
///
/// Nothing here sleeps: the binary prints its three startup facts before it enters the
/// loop, so reading three lines off its stdout is a positive signal that the listeners are
/// bound and `run_forever` is running.
#[cfg(unix)]
#[test]
fn a_sigterm_ends_the_process_cleanly_and_seals_nothing() {
    use std::io::BufRead;

    let t = tempfile::tempdir().unwrap();
    let cfg = support::write_config(t.path());
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup-daemon"))
        .arg("--config")
        .arg(&cfg)
        .arg("--foreground")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = std::io::BufReader::new(child.stdout.take().expect("a piped stdout"));
    let mut started = Vec::new();
    for _ in 0..2 {
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        started.push(line.trim().to_string());
    }
    assert!(started[0].starts_with("127.0.0.1:"), "{started:?}");
    assert!(started[1].ends_with("control.json"), "{started:?}");

    // Safety: `child` is alive — nothing has waited on it — so this pid is this process's
    // own child and cannot have been recycled.
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGTERM) }, 0);
    let status = child.wait().unwrap();
    assert_eq!(
        status.code(),
        Some(0),
        "a handled SIGTERM exits; an unhandled one is reported as killed by a signal"
    );
    // The two endings are the participant's finish and the deadline. A signal is
    // neither, and this daemon had no entry to seal anyway — what matters is that it wrote
    // nothing claiming one.
    assert!(
        !t.path().join("holder/artifacts").exists(),
        "a signal writes no trace"
    );
}

// -----------------------------------------------------------------------------------------
// The capture-integrity cases, on the connector whose stores share one database.
// -----------------------------------------------------------------------------------------

/// A file's verdict survives the bind; clearing it would invert D20 rather than empty the chain.
///
/// Codex's `rollout` store decides a file by its first record. Clearing `file_values` at
/// the bind would throw that decision away, and a foreign thread's later records — which carry no
/// `session_meta` — would then meet the `None` arm, which keeps, and a second session would
/// be admitted to the chain the moment the run started.
#[tokio::test]
async fn a_second_thread_judged_foreign_stays_foreign_once_the_session_binds() {
    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    support::append_line(&h.foreign_store_path, &support::foreign_meta_line());
    let first = h.poll();
    assert_eq!(
        (first.captured, first.discarded),
        (1, 1),
        "the tracked thread is kept and the second one is judged foreign"
    );

    let mut runtime = h.take();
    runtime
        .bind_start(support::SESSION)
        .await
        .expect("the run starts");
    h.give_back(runtime);

    // The second thread carries on. Its records are ordinary ones: Codex names a session
    // once, in the header this file's verdict was already taken from.
    support::append_line(
        &h.foreign_store_path,
        r#"{"type":"response_item","text":"someone else's turn"}"#,
    );
    let after = h.poll();
    assert_eq!(
        (after.captured, after.discarded),
        (0, 1),
        "D20: the file was judged foreign and binding a session is not new evidence about it"
    );
}

/// A `Direct` store teaches itself the first session id it sees after arming, so the
/// records written before the bind are captured instead of judged against an empty set.
///
/// Codex's `history` store partitions `direct` on `session_id`; its tracked set is empty
/// until the session binds, and `admit` judges once. `SessionStart` fires at the first
/// prompt — the same moment the hook-blocked 4,100-character prompt is written to
/// `history.jsonl`, which D2 says is its only leg-1 witness — and the tick drains the stores
/// before it binds. Untaught, that record would be dropped as `ForeignValue` and never re-offered:
/// submitting a goal over the length gate would delete its leg-1 copy, for free.
#[tokio::test]
async fn a_history_record_written_before_the_bind_is_chained_and_a_second_thread_is_not() {
    let h = support::one_armed_entry().await;
    // `{config_dir}/history.jsonl`, which is the per-run CODEX_HOME and is empty at arming.
    let history = h.home.join("history.jsonl");
    support::append_line(
        &history,
        &format!(
            r#"{{"session_id":"{}","ts":1756944001,"text":"the goal, over the length gate"}}"#,
            support::SESSION
        ),
    );
    support::append_line(&h.store_path, &support::session_meta_line(&h));

    let r = h.poll();
    assert_eq!(
        (r.captured, r.discarded),
        (2, 0),
        "the blocked prompt's only leg-1 witness reached the chain before the bind"
    );
    assert_eq!(support::leg1_count(&h.trace_dir, "history"), 1);

    let mut runtime = h.take();
    runtime
        .bind_start(support::SESSION)
        .await
        .expect("the run starts");
    h.give_back(runtime);
    assert!(
        !support::read_chain(&h.trace_dir)
            .iter()
            .any(|e| matches!(e.body, EventBody::ForeignSession { .. })),
        "the store taught itself the session the receipt was issued for: nothing to report"
    );

    // A second thread on the same machine writes to the same history file.
    support::append_line(
        &history,
        r#"{"session_id":"another-thread","ts":1756944100,"text":"someone else's prompt"}"#,
    );
    let after = h.poll();
    assert_eq!(
        (after.captured, after.discarded),
        (0, 1),
        "teaching on first sight is one id, not permission to keep every id"
    );
}

/// …and the reconcile: a taught id the receipt does not agree with is chained as a fact.
///
/// Teaching on first sight is sound because the workspace starts empty and `pre_arm_scan`
/// refuses a home that already holds a tracked session — but "sound" is not "assumed". The
/// receipt is issued for the session the gate corroborated, and where the two disagree the
/// store had been reading a thread that is not this run's. The bound id takes its place and
/// the taught one is stated, never absorbed quietly.
#[tokio::test]
async fn a_taught_session_the_receipt_disagrees_with_is_chained_and_not_re_taught() {
    let h = support::one_armed_entry().await;
    let history = h.home.join("history.jsonl");
    // The first record the `history` store ever sees names a thread that is not the one the
    // rollout will corroborate.
    support::append_line(
        &history,
        r#"{"session_id":"a-thread-that-is-not-this-run","ts":1756944001,"text":"hello"}"#,
    );
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    h.poll();

    let mut runtime = h.take();
    runtime
        .bind_start(support::SESSION)
        .await
        .expect("the run starts");
    h.give_back(runtime);

    let sightings: Vec<String> = support::read_chain(&h.trace_dir)
        .into_iter()
        .filter_map(|e| match e.body {
            EventBody::ForeignSession { session_id, .. } => Some(session_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        sightings,
        vec!["a-thread-that-is-not-this-run".to_string()],
        "the disagreement is a fact in the chain"
    );

    support::append_line(
        &history,
        r#"{"session_id":"a-thread-that-is-not-this-run","ts":1756944100,"text":"more"}"#,
    );
    let after = h.poll();
    assert_eq!(
        (after.captured, after.discarded),
        (0, 1),
        "and the store now tracks the BOUND session, so that thread is foreign"
    );
}

#[tokio::test]
async fn an_own_session_seen_before_bind_is_deferred_and_reconciled_after_bind() {
    let h = support::one_armed_entry_with_distinct_config_home().await;
    let own_scan_store = claude_bound_store_path(&h, support::SESSION);
    write_claude_sighting(&own_scan_store, &h, support::SESSION);

    h.with(|runtime| runtime.sweep_foreign_live_for_test(h.now()))
        .expect("the unbound live sweep runs");
    assert!(
        support::chained_foreign_sessions(&h.trace_dir)
            .iter()
            .all(|s| !s.contains(support::SESSION)),
        "the unbound own session is not chained as foreign"
    );

    h.poll();
    assert_eq!(
        h.with(|runtime| runtime.bound_session()),
        Some(support::SESSION.to_string()),
        "the transcript binds the run before its next sweep"
    );
    h.with(|runtime| runtime.sweep_foreign_live_for_test(h.now()))
        .expect("the bound live sweep runs");
    assert!(
        support::chained_foreign_sessions(&h.trace_dir)
            .iter()
            .all(|s| !s.contains(support::SESSION)),
        "the deferred own session never becomes a foreign event"
    );
}

#[tokio::test]
async fn an_unbound_sweep_reports_same_harness_participant_home_and_other_harness() {
    let h = support::one_armed_entry().await;
    let same_harness_store = h
        .home
        .join(".codex/sessions/2026/09/04/rollout-same-harness.jsonl");
    std::fs::create_dir_all(same_harness_store.parent().expect("the codex scan parent"))
        .expect("the codex scan directory");
    support::append_line(
        &same_harness_store,
        &serde_json::json!({
            "timestamp": "2026-09-04T09:14:01.002Z",
            "type": "session_meta",
            "payload": { "id": "same-harness-foreign", "cwd": h.workspace },
        })
        .to_string(),
    );
    let claude_store = h.home.join(".claude/projects/other/foreign.jsonl");
    std::fs::create_dir_all(claude_store.parent().expect("the claude project parent"))
        .expect("the claude project directory");
    std::fs::write(
        &claude_store,
        serde_json::json!({
            "cwd": h.workspace,
            "sessionId": "other-harness-session",
        })
        .to_string()
            + "\n",
    )
    .expect("the other harness identity record");

    h.with(|runtime| runtime.sweep_foreign_live_for_test(h.now()))
        .expect("the unbound live sweep runs");
    let sightings = support::chained_foreign_sessions(&h.trace_dir);
    assert!(
        sightings
            .iter()
            .any(|s| s.contains("claude-code-cli") && s.contains("other-harness-session")),
        "the other harness is reported immediately: {sightings:?}"
    );
    assert!(
        sightings.iter().any(|s| s.contains("same-harness-foreign")),
        "participant-home same-harness evidence is immediate: {sightings:?}"
    );

    support::append_line(&h.store_path, &support::session_meta_line(&h));
    h.poll();
    let sightings = support::chained_foreign_sessions(&h.trace_dir);
    assert!(
        sightings.iter().any(|s| s.contains("same-harness-foreign")),
        "the true same-harness foreign session is preserved after binding: {sightings:?}"
    );
}

#[tokio::test]
async fn a_seal_sweep_reports_an_unbound_same_harness_sighting() {
    let h = support::one_armed_entry().await;
    let own_scan_store = h
        .home
        .join(".codex/sessions/2026/09/04/rollout-own-seal.jsonl");
    std::fs::create_dir_all(own_scan_store.parent().expect("the codex scan parent"))
        .expect("the codex scan directory");
    support::append_line(&own_scan_store, &support::session_meta_line(&h));

    h.with(|runtime| runtime.sweep_foreign_seal_for_test(h.now()))
        .expect("seal reports an unresolved same-harness sighting");
    let sightings = support::chained_foreign_sessions(&h.trace_dir);
    assert!(
        sightings.iter().any(|s| s.contains(support::SESSION)),
        "an unbound seal fails closed by reporting the unresolved session: {sightings:?}"
    );
}

fn claude_config_sighting_path(
    h: &support::EntryHarness,
    slug: &str,
    session: &str,
) -> std::path::PathBuf {
    h.vars
        .config_dir
        .join("projects")
        .join(slug)
        .join(format!("{session}.jsonl"))
}

fn claude_bound_store_path(h: &support::EntryHarness, session: &str) -> std::path::PathBuf {
    h.vars
        .config_dir
        .join("projects")
        .join(goalcup_daemon::paths::claude_slug(&h.workspace))
        .join(format!("{session}.jsonl"))
}

fn claude_home_sighting_path(
    h: &support::EntryHarness,
    slug: &str,
    session: &str,
) -> std::path::PathBuf {
    h.home
        .join(".claude/projects")
        .join(slug)
        .join(format!("{session}.jsonl"))
}

fn claude_sighting_line(h: &support::EntryHarness, session: &str) -> String {
    serde_json::json!({
        "type": "user",
        "cwd": h.workspace,
        "sessionId": session,
    })
    .to_string()
        + "\n"
}

fn write_claude_sighting(path: &std::path::Path, h: &support::EntryHarness, session: &str) {
    std::fs::create_dir_all(path.parent().expect("the claude sighting parent"))
        .expect("the claude sighting directory");
    std::fs::write(path, claude_sighting_line(h, session)).expect("the claude sighting");
}

#[tokio::test]
async fn an_own_config_sighting_is_pending_until_binding_and_exact_own_is_dropped() {
    let h = support::one_armed_entry_with_distinct_config_home().await;
    let own = claude_bound_store_path(&h, "own-config-session");
    write_claude_sighting(&own, &h, "own-config-session");

    h.with(|runtime| runtime.sweep_foreign_live_for_test(h.now()))
        .expect("the unbound own-config sweep runs");
    assert!(
        support::chained_foreign_sessions(&h.trace_dir)
            .iter()
            .all(|s| !s.contains("own-config-session")),
        "an own-config sighting is deferred before binding"
    );

    h.poll();
    assert_eq!(
        h.with(|runtime| runtime.bound_session()),
        Some("own-config-session".to_string()),
        "the config transcript binds this run"
    );
    assert!(
        support::chained_foreign_sessions(&h.trace_dir)
            .iter()
            .all(|s| !s.contains("own-config-session")),
        "binding drops only the exact own pending identity"
    );
    assert!(
        h.entry().foreign_pending.is_empty(),
        "exact own binding clears its durable pending identity without an event"
    );
}

#[tokio::test]
async fn a_pending_own_config_foreign_survives_removal_and_different_binding() {
    let h = support::one_armed_entry_with_distinct_config_home().await;
    let pending = claude_config_sighting_path(&h, "pending", "pending-config-session");
    write_claude_sighting(&pending, &h, "pending-config-session");

    h.with(|runtime| runtime.sweep_foreign_live_for_test(h.now()))
        .expect("the unbound own-config sweep runs");
    std::fs::remove_file(&pending).expect("the pending source is removed before binding");

    let bound = claude_bound_store_path(&h, "bound-config-session");
    write_claude_sighting(&bound, &h, "bound-config-session");
    h.poll();
    assert_eq!(
        h.with(|runtime| runtime.bound_session()),
        Some("bound-config-session".to_string()),
        "the surviving config transcript binds a different session"
    );
    let sightings = support::chained_foreign_sessions(&h.trace_dir);
    assert!(
        sightings
            .iter()
            .any(|s| s.contains("pending-config-session")),
        "a deferred true foreign identity is retained after its source disappears: {sightings:?}"
    );
    assert!(
        h.entry().foreign_pending.is_empty(),
        "the emitted different-session identity is cleared durably"
    );
}

#[tokio::test]
async fn deferred_own_config_sighting_survives_rebuild_before_different_binding() {
    let h = support::one_armed_entry_with_distinct_config_home().await;
    let pending = claude_config_sighting_path(&h, "crash", "crash-pending-session");
    write_claude_sighting(&pending, &h, "crash-pending-session");

    h.with(|runtime| runtime.sweep_foreign_live_for_test(h.now()))
        .expect("the unbound live sweep runs");
    std::fs::remove_file(&pending).expect("the pending source is removed before restart");

    let bound = claude_bound_store_path(&h, "crash-bound-session");
    write_claude_sighting(&bound, &h, "crash-bound-session");
    let mut restarted = support::reopen_runtime(&h).await;
    restarted
        .open_trace()
        .expect("the rebuilt trace opens before the first restarted poll");
    restarted
        .poll_once_at(h.now())
        .expect("the rebuilt runtime polls");
    h.give_back(restarted);

    let sightings = support::chained_foreign_sessions(&h.trace_dir);
    assert!(
        sightings
            .iter()
            .any(|s| s.contains("crash-pending-session")),
        "a pending identity survives restart and is emitted after a different bind: {sightings:?}"
    );
    assert!(
        h.entry().foreign_pending.is_empty(),
        "the emitted pending identity is cleared from the rebuilt registry row"
    );
}

#[tokio::test]
async fn rebuild_foreign_repair_scans_the_chain_once_for_multiple_pending_identities() {
    let h = support::one_armed_entry_with_distinct_config_home().await;
    for session in ["rebuild-pending-one", "rebuild-pending-two"] {
        let path = claude_config_sighting_path(&h, "rebuild-many", session);
        write_claude_sighting(&path, &h, session);
    }

    h.with(|runtime| runtime.sweep_foreign_live_for_test(h.now()))
        .expect("the live sweep retains both own-config identities");
    let pending_rows = h.entry().foreign_pending.clone();
    assert_eq!(pending_rows.len(), 2);
    h.with(|runtime| runtime.sweep_foreign_seal_for_test(h.now()))
        .expect("the seal sweep chains both pending identities");
    h.registry
        .update(&h.entry_id, |entry| {
            entry.foreign_pending = pending_rows.clone();
        })
        .expect("the test restores the durable pending projection for restart");

    let mut restarted = support::reopen_runtime(&h).await;
    restarted
        .open_trace()
        .expect("the rebuilt trace opens before reconciliation");
    assert_eq!(
        restarted.foreign_chain_scan_count_for_test(),
        1,
        "rebuild performs one global foreign chain scan"
    );

    // The rebuild retained the exact envelopes for both durable rows.  Exercise the
    // retry path before binding becomes known: repeated transient repair failures must
    // retain both bounded markers and must not fall back to a per-identity chain scan.
    restarted.fail_foreign_buffer_repair_for_test(2);
    for attempt in 1..=2 {
        let error = restarted
            .poll_once_at(h.now())
            .expect_err("the injected post-rebuild repair failure is returned");
        assert!(error.to_string().contains("buffer repair append failure"));
        assert_eq!(
            restarted.foreign_chain_scan_count_for_test(),
            1,
            "retry {attempt} does not rescan the chain"
        );
        assert_eq!(
            restarted.foreign_buffer_repair_needed_for_test(),
            2,
            "retry {attempt} retains both exact repair markers"
        );
    }

    let bound = claude_bound_store_path(&h, "rebuild-bound-session");
    write_claude_sighting(&bound, &h, "rebuild-bound-session");
    restarted
        .poll_once_at(h.now())
        .expect("the retained envelopes repair directly on the successful retry");
    assert_eq!(restarted.foreign_chain_scan_count_for_test(), 1);
    assert_eq!(
        restarted.foreign_buffer_repair_needed_for_test(),
        0,
        "successful reconciliation clears all exact rebuild mappings"
    );
    h.give_back(restarted);

    assert_eq!(
        support::chained_foreign_sessions(&h.trace_dir).len(),
        2,
        "reconciliation reuses both chain events without duplicates"
    );
    assert!(h.entry().foreign_pending.is_empty());
}

#[tokio::test]
async fn a_pending_foreign_family_mismatch_refuses_rehydration() {
    let h = support::one_armed_entry_with_distinct_config_home().await;
    let pending = claude_config_sighting_path(&h, "mismatch", "mismatch-session");
    write_claude_sighting(&pending, &h, "mismatch-session");
    h.with(|runtime| runtime.sweep_foreign_live_for_test(h.now()))
        .expect("the own-config identity is retained");
    h.registry
        .update(&h.entry_id, |entry| {
            entry.foreign_pending[0].family = "untrusted-family".to_string();
        })
        .expect("the generic registry shape remains valid");
    let error = match support::try_reopen_runtime(&h).await {
        Ok(_) => panic!("a pending row for another trusted family cannot rehydrate"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("trusted active harness"));
}

#[tokio::test]
async fn pending_own_config_reconciles_on_ordinary_poll_after_binding() {
    let h = support::one_armed_entry_with_distinct_config_home().await;
    h.poll();
    let pending = claude_config_sighting_path(&h, "ordinary", "ordinary-pending-session");
    write_claude_sighting(&pending, &h, "ordinary-pending-session");
    h.with(|runtime| runtime.sweep_foreign_live_for_test(h.now()))
        .expect("the live sweep defers the own-config identity");
    std::fs::remove_file(&pending).expect("the pending source is removed before ordinary poll");

    let bound = claude_bound_store_path(&h, "ordinary-bound-session");
    write_claude_sighting(&bound, &h, "ordinary-bound-session");
    h.poll();

    let sightings = support::chained_foreign_sessions(&h.trace_dir);
    assert!(
        sightings
            .iter()
            .any(|s| s.contains("ordinary-pending-session")),
        "ordinary poll reconciles pending evidence without waiting for another heartbeat: {sightings:?}"
    );
    assert!(
        h.entry().foreign_pending.is_empty(),
        "ordinary reconciliation clears the durable pending identity"
    );
}

#[tokio::test]
async fn a_chain_buffer_gap_is_repaired_without_duplicate_foreign_event() {
    let h = support::one_armed_entry_with_distinct_config_home().await;
    let pending_path = claude_config_sighting_path(&h, "restart", "restart-pending-session");
    write_claude_sighting(&pending_path, &h, "restart-pending-session");
    h.with(|runtime| runtime.sweep_foreign_live_for_test(h.now()))
        .expect("the live sweep defers the own-config identity");
    h.with(|runtime| runtime.fail_foreign_buffer_after_chain_for_test());
    let error = h
        .with(|runtime| runtime.sweep_foreign_seal_for_test(h.now()))
        .expect_err("the interposed process death leaves the buffer append unfinished");
    assert!(error.to_string().contains("chain-to-buffer"));
    assert_eq!(
        support::chained_foreign_sessions(&h.trace_dir).len(),
        1,
        "the chain append happened before the simulated process death"
    );

    h.close();

    let mut restarted = support::reopen_runtime(&h).await;
    restarted
        .open_trace()
        .expect("the rebuilt trace opens before the repair");
    let bound = claude_bound_store_path(&h, "restart-pending-session");
    write_claude_sighting(&bound, &h, "restart-pending-session");
    restarted
        .poll_once_at(h.now())
        .expect("the rebuilt runtime binds and repairs the missing buffer envelope");
    let repaired = support::read_chain(&h.trace_dir)
        .into_iter()
        .find(|envelope| {
            matches!(
                &envelope.body,
                EventBody::ForeignSession { session_id, .. }
                    if session_id == "restart-pending-session"
            )
        })
        .expect("the crash-gap foreign event remains in the chain");
    assert!(
        restarted
            .buffer_contains_envelope_for_test(&repaired)
            .expect("the repaired buffer reads"),
        "the exact chain envelope is restored to the buffer"
    );
    h.give_back(restarted);

    assert_eq!(
        support::chained_foreign_sessions(&h.trace_dir).len(),
        1,
        "repair reuses the chain event instead of appending a duplicate"
    );
    assert!(
        h.entry().foreign_pending.is_empty(),
        "the pending row clears only after the repaired envelope is durable"
    );
}

#[tokio::test]
async fn an_immediate_foreign_buffer_gap_is_repaired_on_the_next_ordinary_tick() {
    let h = support::one_armed_entry_with_distinct_config_home().await;
    let immediate_path = claude_home_sighting_path(&h, "live-gap", "live-gap-session");
    write_claude_sighting(&immediate_path, &h, "live-gap-session");

    h.with(|runtime| runtime.fail_foreign_buffer_after_chain_for_test());
    let error = h
        .with(|runtime| runtime.sweep_foreign_live_for_test(h.now()))
        .expect_err("the interposed process death leaves the immediate buffer append unfinished");
    assert!(error.to_string().contains("chain-to-buffer"));
    assert_eq!(
        support::chained_foreign_sessions(&h.trace_dir).len(),
        1,
        "the immediate sighting reached the chain before the simulated buffer failure"
    );
    std::fs::remove_file(&immediate_path).expect("the immediate source disappears");

    h.poll();

    let repaired = support::read_chain(&h.trace_dir)
        .into_iter()
        .find(|envelope| {
            matches!(
                &envelope.body,
                EventBody::ForeignSession { session_id, .. }
                    if session_id == "live-gap-session"
            )
        })
        .expect("the immediate crash-gap event remains in the chain");
    assert!(
        h.with(|runtime| runtime
            .buffer_contains_envelope_for_test(&repaired)
            .expect("the repaired buffer reads")),
        "the ordinary tick repairs the exact chain envelope after the source disappears"
    );
    assert_eq!(
        support::chained_foreign_sessions(&h.trace_dir).len(),
        1,
        "same-process repair does not append a duplicate chain event"
    );
    let buffered_after_repair = h.buffer_pending();
    h.poll();
    assert_eq!(
        h.buffer_pending(),
        buffered_after_repair,
        "a later ordinary tick does not append the repaired envelope twice"
    );
}

#[tokio::test]
async fn an_immediate_foreign_buffer_gap_survives_rearm_before_repair() {
    let h = support::one_armed_entry_with_distinct_config_home().await;
    let immediate_path = claude_home_sighting_path(&h, "rearm-gap", "rearm-gap-session");
    write_claude_sighting(&immediate_path, &h, "rearm-gap-session");

    h.with(|runtime| runtime.fail_foreign_buffer_after_chain_for_test());
    h.with(|runtime| runtime.sweep_foreign_live_for_test(h.now()))
        .expect_err("the interposed process death leaves the buffer append unfinished");
    std::fs::remove_file(&immediate_path).expect("the immediate source disappears");

    h.with(|runtime| runtime.arm(h.now()))
        .expect("the unstarted entry can open a replacement launch window");
    h.poll();

    let repaired = support::read_chain(&h.trace_dir)
        .into_iter()
        .find(|envelope| {
            matches!(
                &envelope.body,
                EventBody::ForeignSession { session_id, .. }
                    if session_id == "rearm-gap-session"
            )
        })
        .expect("the immediate crash-gap event remains in the chain");
    assert!(
        h.with(|runtime| runtime
            .buffer_contains_envelope_for_test(&repaired)
            .expect("the repaired buffer reads")),
        "re-arm clears deferred classification but preserves committed buffer repair"
    );
    assert_eq!(
        support::chained_foreign_sessions(&h.trace_dir).len(),
        1,
        "repair after re-arm does not append a duplicate chain event"
    );
}

#[tokio::test]
async fn an_immediate_foreign_buffer_gap_retries_without_scanning_the_chain() {
    let h = support::one_armed_entry_with_distinct_config_home().await;
    let immediate_path = claude_home_sighting_path(&h, "retry-gap", "retry-gap-session");
    write_claude_sighting(&immediate_path, &h, "retry-gap-session");

    h.with(|runtime| runtime.fail_foreign_buffer_after_chain_for_test());
    h.with(|runtime| runtime.sweep_foreign_live_for_test(h.now()))
        .expect_err("the interposed process death leaves the buffer append unfinished");
    std::fs::remove_file(&immediate_path).expect("the immediate source disappears");
    h.with(|runtime| runtime.fail_foreign_buffer_repair_for_test(2));

    for attempt in 1..=2 {
        let error = h
            .with(|runtime| runtime.poll_once_at(h.now()))
            .expect_err("the configured direct repair attempt fails");
        assert!(error.to_string().contains("buffer repair append failure"));
        assert_eq!(
            h.with(|runtime| runtime.foreign_buffer_repair_needed_for_test()),
            1,
            "failed direct repair attempt {attempt} retains its exact marker"
        );
        assert_eq!(
            h.with(|runtime| runtime.foreign_chain_scan_count_for_test()),
            0,
            "failed direct repair attempt {attempt} does not stream the chain"
        );
    }

    h.poll();
    let repaired = support::read_chain(&h.trace_dir)
        .into_iter()
        .find(|envelope| {
            matches!(
                &envelope.body,
                EventBody::ForeignSession { session_id, .. }
                    if session_id == "retry-gap-session"
            )
        })
        .expect("the retry-gap event remains in the chain");
    assert!(
        h.with(|runtime| runtime
            .buffer_contains_envelope_for_test(&repaired)
            .expect("the repaired buffer reads")),
        "the successful direct retry appends the retained envelope"
    );
    assert_eq!(
        h.with(|runtime| runtime.foreign_buffer_repair_needed_for_test()),
        0,
        "successful direct repair clears the marker"
    );
    assert_eq!(
        h.with(|runtime| runtime.foreign_chain_scan_count_for_test()),
        0,
        "successful direct repair does not stream the chain"
    );
    assert_eq!(
        support::chained_foreign_sessions(&h.trace_dir).len(),
        1,
        "retry repair does not append a duplicate chain event"
    );
}

#[tokio::test]
async fn participant_home_same_harness_and_other_harness_are_immediate_while_unbound() {
    let h = support::one_armed_entry_with_distinct_config_home().await;
    let participant = claude_home_sighting_path(&h, "participant", "participant-home-session");
    write_claude_sighting(&participant, &h, "participant-home-session");
    let other = h
        .home
        .join(".codex/sessions/2026/09/04/rollout-other-harness.jsonl");
    std::fs::create_dir_all(other.parent().expect("the codex sighting parent"))
        .expect("the codex sighting directory");
    std::fs::write(
        &other,
        serde_json::json!({
            "type": "session_meta",
            "payload": { "id": "other-harness-session", "cwd": h.workspace },
        })
        .to_string()
            + "\n",
    )
    .expect("the other-harness sighting");

    h.with(|runtime| runtime.sweep_foreign_live_for_test(h.now()))
        .expect("the unbound sweep runs");
    let sightings = support::chained_foreign_sessions(&h.trace_dir);
    assert!(
        sightings
            .iter()
            .any(|s| s.contains("participant-home-session")),
        "participant-home same-harness evidence is immediate: {sightings:?}"
    );
    assert!(
        sightings
            .iter()
            .any(|s| s.contains("other-harness-session")),
        "other-harness evidence is immediate: {sightings:?}"
    );
}

#[tokio::test]
async fn seal_flushes_pending_own_config_sightings_while_unbound() {
    let h = support::one_armed_entry_with_distinct_config_home().await;
    let pending = claude_config_sighting_path(&h, "seal", "seal-pending-session");
    write_claude_sighting(&pending, &h, "seal-pending-session");
    h.with(|runtime| runtime.sweep_foreign_live_for_test(h.now()))
        .expect("the live sweep defers the own-config identity");
    std::fs::remove_file(&pending).expect("the pending source is removed before seal");

    h.with(|runtime| runtime.sweep_foreign_seal_for_test(h.now()))
        .expect("seal flushes the pending identity");
    let sightings = support::chained_foreign_sessions(&h.trace_dir);
    assert!(
        sightings.iter().any(|s| s.contains("seal-pending-session")),
        "seal reports unresolved pending identity rather than dropping it: {sightings:?}"
    );
    assert!(
        h.entry().foreign_pending.is_empty(),
        "seal clears the pending identity after chaining its fail-closed event"
    );
}

#[tokio::test]
async fn pending_capacity_overflow_emits_every_identity_without_loss() {
    let h = support::one_armed_entry_with_distinct_config_home().await;
    let total = h.with(|runtime| runtime.foreign_pending_capacity_for_test() + 1);
    for index in 0..total {
        let session = format!("overflow-session-{index:04}");
        let path = claude_config_sighting_path(&h, "overflow", &session);
        write_claude_sighting(&path, &h, &session);
    }

    h.with(|runtime| runtime.sweep_foreign_live_for_test(h.now()))
        .expect("the bounded sweep handles pending capacity");
    let immediate = support::chained_foreign_sessions(&h.trace_dir);
    assert_eq!(
        immediate.len(),
        1,
        "capacity pressure emits rather than silently dropping the overflow identity"
    );
    assert_eq!(
        h.entry().foreign_pending.len(),
        total - 1,
        "the registry retains exactly the bounded non-overflow identities"
    );

    h.with(|runtime| runtime.sweep_foreign_seal_for_test(h.now()))
        .expect("seal flushes the retained pending identities");
    let all = support::chained_foreign_sessions(&h.trace_dir);
    assert_eq!(
        all.len(),
        total,
        "the bounded pending map plus its overflow preserves every identity"
    );
    assert!(h.entry().foreign_pending.is_empty());
}

/// The per-batch cursor save decides in memory whether anything moved.
///
/// The save is a correctness requirement — a cursor written only once a tick leaves a hard
/// kill mid-tick behind the chain — so it runs after every batch that carried records. It
/// answers "has anything moved?" without reading `cursors.json` back off the disk and
/// comparing the whole document on every one of those saves: Claude Code's `raw-bodies`
/// makes that document grow one entry per file for the life of the run: a bound the
/// directory tailer's seen-set does not have, paid on the hot path.
///
/// The comparison is against the digest of what this runtime last wrote. The trade is
/// stated rather than hidden: a `cursors.json` deleted from under a live daemon is not
/// re-written until a cursor actually moves, and until then a restart re-reads that store
/// from zero with a `Reread` witness over it, which is D19's own fallback.
#[tokio::test]
async fn a_tick_that_moves_no_cursor_decides_that_without_reading_the_file_back() {
    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    h.poll();
    let path = h.artifact_dir.join("cursors.json");
    assert!(path.exists(), "the first tick moved a cursor and saved it");

    support::delete_cursors(&h);
    h.poll();
    assert!(
        !path.exists(),
        "nothing moved, so nothing was written — and the question was not asked of the disk"
    );

    support::append_line(&h.store_path, r#"{"type":"response_item","n":1}"#);
    h.poll();
    assert!(
        path.exists(),
        "and the next record that does move a cursor writes the whole document again"
    );
}

// -----------------------------------------------------------------------------------------
// The skip budget has no expiry of its own.
// -----------------------------------------------------------------------------------------

/// A store whose skip budget is
/// spent correctly — every replayed record still judged `Keep` —
/// must still drop to nothing once its re-read is over, and the count
/// of what it actually skipped must equal what the replay actually replayed.
#[tokio::test]
async fn a_correctly_spent_budget_is_still_dropped_and_counted_exactly() {
    let h = support::one_armed_entry().await;
    // `history` and not `rollout`: its cursor is `inode_offset`, the shape
    // `roll_back_file_cursor` rolls, and binding seeds its `Direct` partitioner with the
    // one tracked session, so a re-read record is judged exactly as it was the first time,
    // checked here for the budget's own count.
    support::start_entry(&h).await;
    let history = h.home.join("history.jsonl");
    support::append_line(
        &history,
        &format!(
            r#"{{"session_id":"{}","ts":1,"text":"first"}}"#,
            support::SESSION
        ),
    );
    let offset_before = std::fs::metadata(&history).expect("the history file").len();
    support::append_line(
        &history,
        &format!(
            r#"{{"session_id":"{}","ts":2,"text":"second"}}"#,
            support::SESSION
        ),
    );
    let r = h.poll();
    assert_eq!(
        (r.captured, r.discarded),
        (2, 0),
        "both history lines chain: the session is already bound"
    );

    support::roll_back_file_cursor(&h, "history", &history, offset_before, 1);

    let mut restarted = support::reopen_runtime(&h).await;
    restarted.open_trace().expect("the trace resumes");
    let report = restarted
        .poll_once_at(h.now())
        .expect("the re-reading tick runs");
    assert_eq!(
        (report.captured, report.discarded),
        (0, 0),
        "the one record above the saved position is read again and correctly not re-chained"
    );
    assert_eq!(
        report.skipped_replayed, 1,
        "exactly the one record the replay actually skipped, counted rather than swallowed"
    );

    support::append_line(
        &history,
        &format!(
            r#"{{"session_id":"{}","ts":3,"text":"third"}}"#,
            support::SESSION
        ),
    );
    let after = restarted
        .poll_once_at(h.now())
        .expect("the tick after the re-read runs");
    assert_eq!(
        (after.captured, after.discarded),
        (1, 0),
        "a genuinely new record right after the re-read still chains: the budget was fully \
         spent by the replay and left nothing to eat this one"
    );
    assert_eq!(
        report.skipped_replayed, after.skipped_replayed,
        "cumulative, and nothing is spent a second time on a store with no budget left"
    );
}

/// A re-arm opens a new window, and the previous launch's start signal is not
/// a fact about it.
///
/// `arm()` re-stamps `armed_at` and replaces the gate, so the live half of the run forgets
/// the old signal, and so must the row: `ArmingGate::resume` reads `start_signal_at` and
/// `session_id` on a restart, and a run re-armed after a launch that never took must not
/// come back from a restart with a signal it has never seen this window, already
/// corroborated.
///
/// It has to be cleared in `arm`'s own `registry.update`. Clearing the row afterwards
/// leaves the refreshed `self.entry` mirror holding the old value, and `sync_head` only
/// stamps on `(None, Some(at))`, so the next signal would never reach the row at all;
/// clearing beforehand would erase the record when the pre-arm scan or the `started_at`
/// guard then refuses the re-arm.
#[tokio::test]
async fn re_arming_clears_the_rows_start_signal_and_the_session_it_named() {
    let h = support::one_armed_entry().await;
    // The signal alone, and no start: this is the launch that never took — the harness
    // wrote its first transcript record (the start signal) and
    // then died, which is the case a re-arm exists for.
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    h.poll();

    let row = h
        .registry
        .get(&h.entry_id)
        .expect("the registry reads")
        .expect("the row")
        .clone();
    assert!(
        row.start_signal_at.is_some(),
        "the case only exists once the row has a signal to lose"
    );

    // The launch never took, so the participant arms again — after the dead launch's
    // rollout is gone, or the pre-arm scan would refuse the re-arm over it.
    std::fs::remove_file(&h.store_path).expect("the dead launch's rollout is removed");
    support::advance_clock(&h, 10.0);
    let now = h.now();
    h.with(|r| r.arm(now))
        .expect("a run that never started re-arms");

    let row = h
        .registry
        .get(&h.entry_id)
        .expect("the registry reads")
        .expect("the row")
        .clone();
    assert_eq!(
        row.start_signal_at, None,
        "the previous window's signal is not evidence about this one"
    );
    assert_eq!(
        row.session_id, None,
        "nor is the session it named — `ArmingGate::resume` reads both off the row"
    );
    assert_eq!(row.armed_at, Some(now), "and the new window is stamped");

    let pending_h = support::one_armed_entry_with_distinct_config_home().await;
    let stale = claude_config_sighting_path(&pending_h, "stale", "stale-pending-session");
    write_claude_sighting(&stale, &pending_h, "stale-pending-session");
    pending_h
        .with(|r| r.sweep_foreign_live_for_test(pending_h.now()))
        .expect("the abandoned window can retain a pending foreign identity");
    assert_eq!(pending_h.entry().foreign_pending.len(), 1);

    support::advance_clock(&pending_h, 1.0);
    pending_h
        .with(|r| r.arm(pending_h.now()))
        .expect("re-arming the still-unstarted launch succeeds");
    assert!(
        pending_h.entry().foreign_pending.is_empty(),
        "re-arm clears the abandoned window's durable pending identity"
    );
    std::fs::remove_file(&stale).expect("the abandoned source is removed");
    pending_h
        .with(|r| r.sweep_foreign_seal_for_test(pending_h.now()))
        .expect("the new launch seal completes");
    assert!(
        support::chained_foreign_sessions(&pending_h.trace_dir)
            .iter()
            .all(|s| !s.contains("stale-pending-session")),
        "the abandoned window's in-memory pending identity cannot emit after re-arm"
    );

    // …and the mirror went with it, so the next signal can still be stamped.
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    h.poll();
    assert!(
        h.registry
            .get(&h.entry_id)
            .expect("the registry reads")
            .expect("the row")
            .start_signal_at
            .is_some(),
        "a cleared mirror is what lets `sync_head` stamp the next signal"
    );
}

/// A failed `write_receipt` must not end `bind_start` with the run already
/// started — `record_start` has verified the receipt against the pinned key and written it
/// to the registry, and the row is `Started` — and `receipt.json` never on disk: a
/// descriptor naming it would not load at all.
///
/// The document is held instead, retried at the top of every tick before anything else that
/// tick does, and the descriptor names `receipt.json` only once the write has gone through.
#[tokio::test]
async fn a_receipt_write_that_fails_is_retried_and_the_descriptor_waits_for_it() {
    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    h.poll();

    let mut runtime = h.take();
    runtime.fail_next_receipt_write();
    runtime
        .bind_start(support::SESSION)
        .await
        .expect("a receipt this daemon cannot write is not a failure of the START");
    assert!(
        runtime.receipt_pending(),
        "the verified document is held rather than lost"
    );
    h.give_back(runtime);

    assert!(
        h.registry
            .get(&h.entry_id)
            .expect("the registry reads")
            .expect("the row")
            .started_at
            .is_some(),
        "the run started: the receipt was verified and recorded before the write"
    );
    let receipt_file = h.trace_dir.join(goalcup_protocol::export::RECEIPT_FILE);
    assert!(
        !receipt_file.exists(),
        "and the file is genuinely not there"
    );
    assert_eq!(
        support::read_descriptor(&h.trace_dir).receipt,
        None,
        "so the descriptor does not name it: a trace naming a receipt.json the disk does \
         not hold will not load at all"
    );

    // The next tick pays the debt.
    support::advance_clock(&h, 1.0);
    h.poll();
    assert!(
        !h.with(|r| r.receipt_pending()),
        "the retry cleared the debt"
    );
    assert!(
        receipt_file.exists(),
        "and put the document beside the chain"
    );
    assert_eq!(
        support::read_descriptor(&h.trace_dir).receipt.as_deref(),
        Some(goalcup_protocol::export::RECEIPT_FILE),
        "and the descriptor names it now that there is one to name"
    );
}

#[tokio::test]
async fn descriptor_failure_keeps_exact_receipt_debt_ahead_of_every_poll() {
    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    h.poll();

    let mut runtime = h.take();
    runtime.fail_next_receipt_write();
    runtime
        .bind_start(support::SESSION)
        .await
        .expect("the registry-authoritative start survives the initial mirror failure");
    assert!(runtime.receipt_pending());
    h.give_back(runtime);

    let expected = serde_json::to_vec(h.entry().receipt().unwrap()).unwrap();
    let receipt = h.trace_dir.join(goalcup_protocol::export::RECEIPT_FILE);
    std::fs::write(&receipt, &expected).expect("the exact authoritative mirror appears");
    let saved = obstruct_descriptor(&h, "descriptor-before-receipt-retry");
    support::append_line(&h.store_path, r#"{"type":"response_item","n":901}"#);
    let chain_before =
        std::fs::read(h.trace_dir.join(goalcup_protocol::export::CHAIN_FILE)).unwrap();
    let row_before = h.entry();
    let records_before = support::leg1_count(&h.trace_dir, "rollout");

    for attempt in 1..=2 {
        let error = h
            .with(|runtime| runtime.poll_once_at(h.now()))
            .expect_err("descriptor obstruction keeps the receipt gate closed");
        assert!(
            error.to_string().contains("fixed"),
            "attempt {attempt} names the fixed-document refusal: {error}"
        );
        assert!(
            h.with(|runtime| runtime.receipt_pending()),
            "attempt {attempt} retains the receipt authority for another publication retry"
        );
        assert_eq!(
            std::fs::read(h.trace_dir.join(goalcup_protocol::export::CHAIN_FILE)).unwrap(),
            chain_before,
            "attempt {attempt} stops before queued or tailed evidence"
        );
        assert_eq!(h.entry(), row_before);
        assert_eq!(std::fs::read(&receipt).unwrap(), expected);
    }

    let planted = b"receipt changed after its last exact observation";
    std::fs::write(&receipt, planted).expect("the ready mirror is replaced while blocked");
    restore_descriptor(&h, &saved);
    let poll_error = h
        .with(|runtime| runtime.poll_once_at(h.now()))
        .expect_err("the changed child must be revalidated before descriptor publication");
    assert!(poll_error.to_string().contains("receipt"));
    let seal_error = h
        .with(|runtime| runtime.seal())
        .expect_err("the changed child must also block sealing before its workspace scan");
    assert!(seal_error.to_string().contains("receipt"));
    assert!(h.with(|runtime| runtime.receipt_pending()));
    assert_eq!(std::fs::read(&receipt).unwrap(), planted);
    assert_eq!(
        std::fs::read(h.trace_dir.join(goalcup_protocol::export::CHAIN_FILE)).unwrap(),
        chain_before
    );
    assert_eq!(h.entry(), row_before);
    assert_eq!(
        support::read_descriptor(&h.trace_dir).receipt,
        None,
        "the repaired descriptor path still carries its prior unpublished document"
    );

    std::fs::remove_file(&receipt).expect("only the contradictory child is removed");
    h.with(|runtime| runtime.poll_once_at(h.now()))
        .expect("absence permits exact restoration and publication before polling resumes");
    assert!(!h.with(|runtime| runtime.receipt_pending()));
    assert_eq!(std::fs::read(&receipt).unwrap(), expected);
    assert_eq!(
        support::read_descriptor(&h.trace_dir).receipt.as_deref(),
        Some(goalcup_protocol::export::RECEIPT_FILE)
    );
    assert_eq!(
        support::leg1_count(&h.trace_dir, "rollout"),
        records_before + 1,
        "the queued record is processed only after descriptor publication succeeds"
    );
}

#[tokio::test]
async fn bind_start_descriptor_failure_remains_repairable_by_poll() {
    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    h.poll();
    support::append_line(&h.store_path, r#"{"type":"response_item","n":902}"#);
    let records_before = support::leg1_count(&h.trace_dir, "rollout");
    let saved = obstruct_descriptor(&h, "descriptor-before-live-start");

    let mut runtime = h.take();
    let error = runtime
        .bind_start(support::SESSION)
        .await
        .expect_err("the live start reports descriptor publication failure");
    assert!(error.to_string().contains("fixed"));
    assert_eq!(
        runtime.entry().state,
        goalcup_daemon::registry::EntryState::Started
    );
    assert!(runtime.entry().receipt().is_some());
    assert!(
        runtime.receipt_pending(),
        "the first failed descriptor publication retains retry authority"
    );
    runtime
        .bind_start(support::SESSION)
        .await
        .expect("a second corroboration is an already-started no-op");
    assert!(
        runtime.receipt_pending(),
        "the no-op start cannot replace the poll path's publication retry"
    );
    h.give_back(runtime);

    restore_descriptor(&h, &saved);
    h.with(|runtime| runtime.poll_once_at(h.now()))
        .expect("poll repairs publication without repeating the start transition");
    assert!(!h.with(|runtime| runtime.receipt_pending()));
    assert_eq!(
        support::read_descriptor(&h.trace_dir).receipt.as_deref(),
        Some(goalcup_protocol::export::RECEIPT_FILE)
    );
    assert_eq!(
        support::leg1_count(&h.trace_dir, "rollout"),
        records_before + 1,
        "the queued record follows the repaired descriptor"
    );
}

/// The seen-file bound reports a `TailNotice::StoreUnavailable`,
/// which chains nothing and says "store unavailable" about a store that is perfectly
/// readable and merely full. The poll pass drops the report that notice rides on, so the
/// fact would reach no health input and no document: every file past the bound is evidence
/// that will not exist, with nothing anywhere saying so.
///
/// It is counted, cumulatively, for the same reason `polls_capped` is.
#[tokio::test]
async fn the_status_document_carries_the_files_the_seen_bound_refused() {
    let h = support::one_armed_entry_on("claude-code-cli").await;
    // `raw_bodies` is Claude Code's `mode = "dir"` store, and a directory reader is the only
    // kind that keeps a file set at all.
    let tool_output = h.artifact_dir.join("raw-bodies");
    std::fs::create_dir_all(&tool_output).expect("the raw-bodies directory");
    h.with(|r| r.set_seen_cap("raw_bodies", 2));

    for n in 0..2 {
        std::fs::write(tool_output.join(format!("call-{n}.json")), b"{\"a\":1}")
            .expect("a tool output file");
    }
    let filled = h.poll();
    assert_eq!(
        filled.files_capped, 0,
        "two files fill the bound exactly and nothing was refused"
    );

    // The third file: the set is full, every file in it is still present, so there is no
    // entry to evict and no room to remember this one.
    std::fs::write(tool_output.join("call-2.json"), b"{\"a\":1}").expect("one file too many");
    let report = h.poll();
    assert_eq!(
        report.files_capped, 1,
        "the poll that could not take the file counted it"
    );
    assert_eq!(
        h.with(|r| r.status().expect("the payload builds"))
            .files_capped,
        1,
        "and the signed document carries it: the notice beside it chains nothing, so this \
         is the only place the refused file is ever stated"
    );

    // Cumulative, not a per-tick flag: a store that has been over its bound since the run
    // started must not look like one that went over once.
    support::advance_clock(&h, 1.0);
    h.poll();
    assert_eq!(
        h.with(|r| r.status().expect("the payload builds"))
            .files_capped,
        2,
        "the second refusal is added rather than replacing the first"
    );
}

/// The retry above runs at the top of a tick, and a seal is not a
/// tick. A run sealed while the receipt was still owed would rewrite the descriptor from a state
/// that had no `receipt.json` in it, and then close the chain — after which no tick ever
/// runs again. V13 would read that trace as `receipt_missing` for good, for a run whose
/// receipt was verified and recorded and merely could not be written the first time.
///
/// So the seal pays the debt first, before the descriptor it rewrites is built.
#[tokio::test]
async fn a_seal_pays_the_receipt_debt_before_it_writes_the_last_descriptor() {
    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    h.poll();

    let mut runtime = h.take();
    runtime.fail_next_receipt_write();
    runtime
        .bind_start(support::SESSION)
        .await
        .expect("a receipt this daemon cannot write is not a failure of the START");
    assert!(runtime.receipt_pending(), "the write failed and it is owed");
    h.give_back(runtime);

    let receipt_file = h.trace_dir.join(goalcup_protocol::export::RECEIPT_FILE);
    assert!(
        !receipt_file.exists(),
        "the file is genuinely not there yet"
    );

    // The seal, on the very next thing the entry does — no tick in between, which is the
    // case: `goalcup finish` on a run whose last receipt write failed.
    h.with(|r| r.seal()).expect("the run seals");

    assert!(
        !h.with(|r| r.receipt_pending()),
        "the seal cleared the debt rather than closing the chain over it"
    );
    assert!(
        receipt_file.exists(),
        "and put the verified document beside the chain"
    );
    let doc = support::read_descriptor(&h.trace_dir);
    assert_eq!(
        doc.receipt.as_deref(),
        Some(goalcup_protocol::export::RECEIPT_FILE),
        "and the LAST descriptor names it: nothing runs after a seal to name it later, so \
         a descriptor written without it is `receipt_missing` for the life of the trace"
    );
    assert_eq!(
        doc.seal.as_deref(),
        Some(goalcup_protocol::export::SEAL_FILE),
        "the same document still names the seal it was rewritten for"
    );
}

/// Authoritative receipt debt is a mutation gate, not a best-effort retry. A
/// contradictory child is preserved, and neither polling nor sealing may append, scan or
/// rewrite around it. Once the contradictory evidence is removed, the same retained writer
/// can pay the debt and continue.
#[tokio::test]
async fn contradictory_receipt_debt_blocks_poll_and_seal_until_a_legitimate_retry() {
    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    h.poll();

    let mut runtime = h.take();
    runtime.fail_next_receipt_write();
    runtime
        .bind_start(support::SESSION)
        .await
        .expect("the checked registry start survives a mirror write failure");
    h.give_back(runtime);

    let receipt_path = h.trace_dir.join(goalcup_protocol::export::RECEIPT_FILE);
    let planted = b"partial receipt evidence";
    std::fs::write(&receipt_path, planted).unwrap();
    let chain_before =
        std::fs::read(h.trace_dir.join(goalcup_protocol::export::CHAIN_FILE)).unwrap();
    let descriptor_before =
        std::fs::read(h.trace_dir.join(goalcup_protocol::export::DESCRIPTOR_FILE)).unwrap();
    let row_before = h.entry();

    let poll_error = h
        .with(|r| r.poll_once_at(h.now()))
        .expect_err("polling cannot process evidence around receipt debt");
    assert!(poll_error.to_string().contains("receipt"));
    let seal_error = h
        .with(|r| r.seal())
        .expect_err("sealing cannot scan or append around receipt debt");
    assert!(seal_error.to_string().contains("receipt"));
    assert_eq!(std::fs::read(&receipt_path).unwrap(), planted);
    assert_eq!(
        std::fs::read(h.trace_dir.join(goalcup_protocol::export::CHAIN_FILE)).unwrap(),
        chain_before
    );
    assert_eq!(
        std::fs::read(h.trace_dir.join(goalcup_protocol::export::DESCRIPTOR_FILE)).unwrap(),
        descriptor_before
    );
    assert_eq!(h.entry(), row_before);
    assert!(h.with(|r| r.receipt_pending()));

    std::fs::remove_file(&receipt_path).unwrap();
    h.poll();
    assert!(!h.with(|r| r.receipt_pending()));
    assert_eq!(
        std::fs::read(&receipt_path).unwrap(),
        serde_json::to_vec(h.entry().receipt().unwrap()).unwrap()
    );
}

/// The per-file session memo is one entry per tailed file that named a session, and a
/// glob like `{config_dir}/projects/*/*.jsonl` matches every file the store has ever held.
/// A file the harness rotated away is forgotten once a tick, so the memo stays the size of
/// the store rather than the size of its history.
#[tokio::test]
async fn the_per_file_session_memo_forgets_a_file_that_is_gone() {
    let h = support::one_armed_entry().await;
    support::append_line(&h.store_path, &support::session_meta_line(&h));
    h.poll();
    assert_eq!(
        h.with(|r| r.sessions_by_path_len()),
        1,
        "the rollout named a session, so the memo remembers its file"
    );

    std::fs::remove_file(&h.store_path).expect("the harness rotates the transcript away");
    support::advance_clock(&h, 1.0);
    h.poll();
    assert_eq!(
        h.with(|r| r.sessions_by_path_len()),
        0,
        "a file that is gone is forgotten, so the memo stays the size of the store rather \
         than the size of its history"
    );
}

/// In one place: the counters the daemon keeps and nothing else
/// states reach the signed status document.
///
/// Each of them is invisible from every other number on that document. A capped tick looks
/// exactly like a quiet one; a store that restarted at offset 0 looks like a store with
/// nothing in it; a record the restart budget swallowed is neither captured nor discarded;
/// a hole in the write-ahead buffer is permanent and has no health rung.
/// Every counter here is driven to a value, not read at rest.
/// Asserting zeros would pass against hard-coded zeros,
/// which is precisely the failure this catches: a field wired to a literal looks
/// exactly like a field wired to a counter that has not moved.
///
/// The two hook counters, `pins_dropped` and `posts_dropped_after_seal`, are not
/// driven: nothing produces them, and they stay zero on the document.
#[tokio::test]
async fn the_status_document_carries_the_counters_nothing_else_states() {
    let h = support::one_armed_entry().await;
    support::start_entry(&h).await;

    let payload = h.with(|r| r.status().expect("the payload builds"));
    assert_eq!(payload.polls_capped, 0, "nothing has been driven yet");
    assert_eq!(payload.skipped_replayed, 0);
    assert_eq!(payload.posts_dropped_after_seal, 0);
    assert_eq!(payload.pins_dropped, 0);
    assert_eq!(payload.buffer_gap_at, None);

    // ---- the two records the restart will replay one of.
    let history = h.home.join("history.jsonl");
    support::append_line(
        &history,
        &format!(
            r#"{{"session_id":"{}","ts":1,"text":"first"}}"#,
            support::SESSION
        ),
    );
    let offset_before = std::fs::metadata(&history).expect("the history file").len();
    support::append_line(
        &history,
        &format!(
            r#"{{"session_id":"{}","ts":2,"text":"second"}}"#,
            support::SESSION
        ),
    );
    assert_eq!(h.poll().captured, 2, "both history lines chain");

    // `history` keeps its position but behind the chain — one record to replay and skip —
    // while `rollout` loses its line from the cursor file entirely and comes back at zero.
    // One restart, both counters: they are different failures and a run can meet both.
    support::roll_back_file_cursor(&h, "history", &history, offset_before, 1);
    forget_cursor_line(&h, "rollout");

    // ---- and the hole in the write-ahead buffer, punched while nothing holds the redb
    //      file lock. A seq nothing ever wrote is what a torn write leaves behind, and
    //      nothing refills one. Far above anything this run will chain, so the hole is
    //      whatever the buffer's contiguous run ends at when the pump reads it.
    h.close();
    punch_buffer_hole(&h);

    let mut restarted = support::reopen_runtime(&h).await;
    restarted.open_trace().expect("the trace resumes");
    h.give_back(restarted);

    let report = h.poll();
    assert_eq!(
        report.skipped_replayed, 1,
        "the one record above the saved position was read again and not re-chained"
    );
    assert_eq!(
        report.cursor_restore_failures, 1,
        "one store's position did not come back and it restarted at offset 0"
    );

    // ---- the poll cap: one store holding more than `MAX_POLLS_PER_TICK` windows' worth.
    //      Foreign rows, so the drain does the reading and the partitioner discards every
    //      one of them — this is about the tick yielding, not about what it captured.
    support::append_line(&h.foreign_store_path, &support::foreign_meta_line());
    let windows = goalcup_daemon::runtime::MAX_POLLS_PER_TICK + 1;
    let filler = format!("{{\"text\":\"{}\"}}", "x".repeat(1_000_000));
    for _ in 0..windows {
        support::append_line(&h.foreign_store_path, &filler);
    }
    support::advance_clock(&h, 1.0);
    let report = h.poll();
    assert_eq!(
        report.captured, 0,
        "the filler is another thread's file: `file_first_record` judged it on the \
         `session_meta` above and every line under it is discarded"
    );
    assert!(
        report.polls_capped >= 1,
        "the tick yielded with the store still holding more, and counted that it did"
    );

    // ---- and the pump that meets the hole. The chain head is where the contiguous run
    //      ends, so the missing seq is the one after it.
    let gap_at = h.head().0 + 1;
    let mut runtime = h.take();
    runtime.pump().await.expect("the pump runs");
    h.give_back(runtime);

    // ---- the document, and every one of the four with a value on it.
    let payload = h.with(|r| r.status().expect("the payload builds"));
    assert!(
        payload.polls_capped >= 1,
        "a daemon that has been behind since the run started has no other surface"
    );
    assert_eq!(
        payload.skipped_replayed, 1,
        "a record the restart budget swallowed is neither captured nor discarded"
    );
    assert_eq!(
        payload.cursor_restore_failures, 1,
        "a store that restarted at offset 0 looks like a store with nothing in it"
    );
    assert_eq!(
        payload.pins_dropped, 0,
        "no hook leg reports a pin any more"
    );
    assert_eq!(
        payload.buffer_gap_at,
        Some(gap_at),
        "a hole in the write-ahead buffer is permanent and the ladder has no rung for it"
    );

    // …and they are the runtime's own numbers rather than a second reckoning of them.
    let report = h.poll();
    let payload = h.with(|r| r.status().expect("the payload builds"));
    assert_eq!(payload.polls_capped, report.polls_capped);
    assert_eq!(payload.skipped_replayed, report.skipped_replayed);
    assert_eq!(
        payload.cursor_restore_failures,
        report.cursor_restore_failures
    );
}

/// Take one store's line out of `cursors.json`, so the restart finds no position for it and
/// begins that store at offset 0 — the failure `cursor_restore_failures` counts.
fn forget_cursor_line(h: &support::EntryHarness, store: &str) {
    let path = h.artifact_dir.join("cursors.json");
    let text = std::fs::read_to_string(&path).expect("the cursor file reads");
    let mut doc: serde_json::Value = serde_json::from_str(&text).expect("cursors.json is JSON");
    doc.as_object_mut()
        .expect("the cursor file is an object keyed by store")
        .remove(store)
        .unwrap_or_else(|| panic!("{store} had a line to remove"));
    std::fs::write(
        &path,
        serde_json::to_vec(&doc).expect("the cursor file serialises"),
    )
    .expect("the shortened cursor file is written");
}

/// Append an envelope far above the head, leaving every seq between them missing.
///
/// Far above rather than one past it, because the run goes on chaining afterwards and a
/// hole one seq wide would be filled by the very next envelope. The harness's runtime must
/// be closed: redb takes a file lock, and a second `Buffer::open` on a live one fails
/// rather than sharing it.
fn punch_buffer_hole(h: &support::EntryHarness) {
    let entry = h.entry();
    let buffer = goalcup_daemon::buffer::Buffer::open(
        &goalcup_daemon::buffer::Buffer::default_path(&entry.artifact_dir),
        &entry.keys.buffer_key_hex,
    )
    .expect("the buffer opens while the runtime is closed");
    let head = buffer
        .head()
        .expect("the head reads")
        .expect("the chain has links")
        .0;
    buffer
        .append(&support::envelope(head + 10_000, "rollout"))
        .expect("a seq above the hole is buffered");
}

/// The record the CLI hands over on `register_process` reaches the chain, once
/// per launch, with a non-secret generation/digest replay marker — and never the registry
/// row it travelled on.
///
/// The daemon interprets nothing here (D3). What is asserted is that it wrote down what it
/// was given, that a launch which seeded nothing chains nothing, and that a resume — which
/// clears the handle and registers a second one — chains its own record beside the first, so
/// a reader can see which settings each half of the run was given.
#[tokio::test]
async fn a_launch_chains_the_settings_it_seeded_and_a_resume_chains_its_own() {
    use goalcup_protocol::events::{HomeSeedRecord, SeededKey};

    let h = support::started_entry_with_work().await;
    let record = |theme: &str| HomeSeedRecord {
        handoff_generation: None,
        handoff_request_digest: None,
        connector: "claude-code-cli".to_string(),
        from: "{home}/.claude.json".to_string(),
        keys: vec![
            SeededKey::new("claudeInChromeDefaultEnabled", &serde_json::json!(true)),
            SeededKey::new("theme", &serde_json::json!(theme)),
        ],
        keys_unchanged: Vec::new(),
        written: Vec::new(),
        files: Vec::new(),
        files_kept: Vec::new(),
        files_missing: Vec::new(),
        wire_layout: Default::default(),
    };

    // This harness already registered a handle when it started, so the slot is opened the
    // one way it can be — the resume route's `clear_process` — before the launch under test.
    h.daemon
        .clear_process(&h.entry_id, 1)
        .expect("the standing handle clears");
    let mut handle = support::handle_for_now(h.child_pid);
    handle.home_seed = Some(record("dark"));
    h.daemon
        .register_process(&h.entry_id, support::process_registration(2, handle))
        .expect("the process is registered");

    let seeded: Vec<HomeSeedRecord> = support::read_chain(&h.trace_dir)
        .into_iter()
        .filter_map(|e| match e.body {
            EventBody::HomeSeeded(r) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(seeded.len(), 1, "one link per launch");
    assert_eq!(seeded[0].handoff_generation, Some(2));
    assert!(
        seeded[0].handoff_request_digest.is_some(),
        "the event carries a non-secret recovery marker, never a seed-derived digest"
    );
    let mut dark = seeded[0].clone();
    dark.handoff_generation = None;
    dark.handoff_request_digest = None;
    assert_eq!(
        dark,
        record("dark"),
        "the CLI-measured seed evidence is unchanged"
    );

    // And it is not kept anywhere else: the handle is the courier, the chain is the record.
    assert!(
        h.entry()
            .last_process
            .expect("the handle stands")
            .home_seed
            .is_none(),
        "the registry's row keeps no second copy of the participant's settings"
    );

    // The resume: the slot is opened, and the relaunch registers a handle of its own. The
    // participant changed their theme between the two halves, and the chain says so.
    h.daemon
        .clear_process(&h.entry_id, 2)
        .expect("a resume opens the slot");
    let mut relaunch = support::handle_for_now(h.child_pid);
    relaunch.home_seed = Some(record("light"));
    h.daemon
        .register_process(&h.entry_id, support::process_registration(3, relaunch))
        .expect("the relaunch registers");

    // …and a third launch that seeded nothing chains nothing, which is what `None` means.
    h.daemon.clear_process(&h.entry_id, 3).expect("and again");
    h.daemon
        .register_process(
            &h.entry_id,
            support::process_registration(4, support::handle_for_now(h.child_pid)),
        )
        .expect("a connector with no `[home_seed]` still registers");

    let seeded: Vec<HomeSeedRecord> = support::read_chain(&h.trace_dir)
        .into_iter()
        .filter_map(|e| match e.body {
            EventBody::HomeSeeded(r) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(seeded.len(), 2);
    assert_eq!(
        seeded
            .iter()
            .map(|record| record.handoff_generation)
            .collect::<Vec<_>>(),
        vec![Some(2), Some(3)],
        "each accepted launch records its own generation once"
    );
    assert!(
        seeded
            .iter()
            .all(|record| record.handoff_request_digest.is_some()),
        "each marker is a canonical process identity, not a home-seed hash"
    );
    for (actual, expected) in seeded.into_iter().zip([record("dark"), record("light")]) {
        let mut actual = actual;
        actual.handoff_generation = None;
        actual.handoff_request_digest = None;
        assert_eq!(actual, expected);
    }
}

/// An issued resume permit keeps the prior handle as termination authority until the
/// replacement becomes Registered. A later local failure must therefore never make seal
/// silently lose the original harness.
#[tokio::test]
async fn a_started_entry_with_an_open_resume_keeps_its_termination_authority() {
    let h = support::started_entry_with_work().await;

    // The resume permit is issued but the relaunch that would follow it never registers.
    h.daemon
        .clear_process(&h.entry_id, 1)
        .expect("the slot opens for a relaunch");

    {
        let mut borrowed = h
            .daemon
            .borrow_entry(&h.entry_id)
            .expect("the entry borrows");
        let runtime = borrowed.get().expect("the runtime is there");
        assert!(
            runtime.owes_a_process_handle(),
            "a started tui entry owes one"
        );
        assert!(
            runtime.same_process(),
            "the prior harness remains the seal target"
        );
    }

    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let report = h
        .daemon
        .seal_report(&h.entry_id)
        .unwrap()
        .expect("a report");
    assert!(!report.guard_degraded, "{report:?}");
    assert_eq!(
        report.harness_unverified,
        cfg!(target_os = "macos"),
        "Darwin proves exact root exit but not full-tree prehistory: {report:?}"
    );
    assert!(report.termination_error.is_none());
    assert!(
        !goalcup_protocol::process::is_alive(h.child_pid),
        "seal terminated the retained harness"
    );
    assert_eq!(
        h.daemon
            .registry()
            .get(&h.entry_id)
            .unwrap()
            .expect("the row")
            .seal_outcome
            .expect("persisted")
            .harness_unverified,
        cfg!(target_os = "macos")
    );
}
