//! V4 finish is one durable state machine, whether the caller is manual, the signed
//! deadline, or startup recovery.
#![cfg(feature = "v1-compat")]

mod support;

use goalcup_daemon::DaemonError;
use goalcup_daemon::runtime::Daemon;
use goalcup_protocol::control::FinishReportV1;
use goalcup_protocol::submission::FinishReasonV1;

#[tokio::test]
async fn control_finish_replays_terminal_status_across_a_restart() {
    let h = support::v4_daemon(128).await;
    let (daemon, client, config) = (h.daemon, h.client, h.config);
    let mut opts = support::workspace_opts("claude-code-cli");
    opts.attach = false;
    let info: goalcup_protocol::control::WorkspaceInfo = client
        .post_json(goalcup_protocol::control::ROUTE_WORKSPACES, &opts)
        .await;
    daemon.arm(&info.entry_id).unwrap();
    let mut child = support::daemon::spawn_harness();
    daemon
        .register_process(
            &info.entry_id,
            support::process_registration(
                info.process_generation,
                support::daemon::handle_for_now(child.id()),
            ),
        )
        .unwrap();
    let workspace = std::path::Path::new(&info.workspace);
    let home = std::path::Path::new(&info.harness_home);
    support::append_line(
        &support::daemon::transcript_path(home, workspace, &info.session_id),
        &support::daemon::transcript_line(workspace, &info.session_id, "the goal"),
    );
    daemon.tick().await.unwrap();
    assert_eq!(
        daemon
            .registry()
            .get(&info.entry_id)
            .unwrap()
            .unwrap()
            .state,
        goalcup_daemon::registry::EntryState::Started,
    );
    std::fs::write(workspace.join("answer.txt"), b"durable answer\n").unwrap();

    daemon
        .fail_before_workspace_intent_cas_for_test(&info.entry_id)
        .unwrap();
    assert!(
        daemon
            .finish_entry(&info.entry_id, FinishReasonV1::Manual)
            .is_err(),
        "the exact pre-intent fault lands after source binding but before admission CAS",
    );
    assert!(
        serde_json::from_slice::<serde_json::Value>(
            &daemon
                .registry()
                .raw_json_for_test(&info.entry_id)
                .unwrap()
                .unwrap(),
        )
        .unwrap()["sealing_intent"]
            .is_null(),
    );
    let response = client
        .post(&goalcup_protocol::control::entry_route(
            &info.entry_id,
            goalcup_protocol::control::SUFFIX_SEAL,
        ))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body());
    let goalcup_protocol::control::SealResponse::Workspace(first) = response.json().unwrap() else {
        panic!("V4 control seal returned the legacy shape")
    };
    let retry = daemon
        .finish_entry(&info.entry_id, FinishReasonV1::Manual)
        .unwrap();
    assert_eq!(retry, first);
    let status = daemon
        .signed_status(&info.entry_id)
        .unwrap()
        .expect("the production control finish publishes terminal status");
    let pre_restart_status = status.clone();
    let Some(goalcup_protocol::control::WorkspaceFinalizationStatusV1::Terminal {
        termination,
        submission,
        ..
    }) = status.payload.finalization
    else {
        panic!("V4 control finish did not publish the durable terminal projection")
    };
    assert_eq!(termination, first.termination);
    assert_eq!(submission, first.submission);
    assert!(
        matches!(
            first.termination,
            goalcup_protocol::provenance::TerminationOutcomeV1::Confirmed {
                method: goalcup_protocol::provenance::TerminationMethodV1::ProcessSignal,
                ..
            }
        ),
        "R15: the daemon ends the harness it was handed authority over, and the terminal \
         projection records the signal it sent: {:?}",
        first.termination,
    );
    assert!(
        !goalcup_protocol::process::is_alive(child.id()),
        "a Confirmed record over a live process would be the M0 §2.5 lie",
    );
    assert!(matches!(
        first.submission,
        goalcup_protocol::submission::SubmissionOutcomeV1::Captured(_)
    ));
    let chain = support::read_chain(&std::path::Path::new(&info.artifact_root).join("trace"));
    let row: serde_json::Value = serde_json::from_slice(
        &daemon
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(row["sealing_intent"]["reason"], "manual");
    assert_eq!(
        row["sealing_intent"]["cutoff_at"],
        chain
            .iter()
            .find(|event| matches!(
                event.body,
                goalcup_protocol::envelope::EventBody::CutoffReachedV1(_)
            ))
            .unwrap()
            .recorded_at,
        "the cutoff envelope uses the durable manual winner's observed instant",
    );
    assert!(
        goalcup_protocol::control::parse_z(row["sealing_intent"]["cutoff_at"].as_str().unwrap())
            < goalcup_protocol::control::parse_z(row["deadline_at"].as_str().unwrap()),
        "a pre-deadline manual winner remains distinguishable from deadline finalization",
    );
    let manifest: goalcup_protocol::submission::SubmissionManifestV1 = serde_json::from_slice(
        &std::fs::read(
            std::path::Path::new(&info.artifact_root).join("trace/submission/manifest.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(manifest.finish_reason, FinishReasonV1::Manual);
    assert_eq!(
        serde_json::json!(manifest.cutoff_seq),
        row["sealing_intent"]["cutoff"]["seq"],
    );
    assert_eq!(
        serde_json::json!(manifest.cutoff_chain_hash.as_str()),
        row["sealing_intent"]["cutoff"]["hash"],
    );
    assert_eq!(
        chain
            .iter()
            .filter(|event| matches!(
                event.body,
                goalcup_protocol::envelope::EventBody::CutoffReachedV1(_)
            ))
            .count(),
        1,
    );
    let terminal = chain.last().unwrap();
    assert_eq!(
        terminal.recorded_at,
        row["sealing_intent"]["finalization_at"],
    );
    assert_eq!(
        serde_json::to_value(&terminal.link).unwrap(),
        row["sealing_intent"]["finalization"],
    );
    assert_eq!(
        chain
            .iter()
            .filter(|event| matches!(
                event.body,
                goalcup_protocol::envelope::EventBody::FinalizationV1(_)
            ))
            .count(),
        1,
    );

    let mut interrupted_status = row.clone();
    interrupted_status["sealing_intent"]["phase"] =
        serde_json::Value::String("terminal_status_pending".to_string());
    interrupted_status["sealing_intent"]["status_mirror_complete"] = serde_json::Value::Bool(false);
    daemon
        .registry()
        .replace_raw_json_for_test(
            &info.entry_id,
            &serde_json::to_vec(&interrupted_status).unwrap(),
        )
        .unwrap();
    daemon.stop().await;
    let restarted = Daemon::start(config.clone()).await.unwrap();
    let restarted_client = support::ControlClient::of(&restarted, &config.config_dir);
    let status_route = goalcup_protocol::control::entry_route(
        &info.entry_id,
        goalcup_protocol::control::SUFFIX_STATUS,
    );
    let production_status: goalcup_protocol::control::SignedStatus =
        restarted_client.get(&status_route).await.json().unwrap();
    assert_eq!(
        production_status.payload, pre_restart_status.payload,
        "the production status route replays the exact persisted terminal projection",
    );
    production_status
        .verify_with(&info.run_public_key)
        .expect("the refreshed projection remains signed by the pinned run key");
    let restarted_raw: serde_json::Value = serde_json::from_slice(
        &restarted
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        restarted_raw["sealing_intent"]["phase"], "terminal",
        "a sealed row interrupted before its status-completion CAS is recovered exactly once",
    );
    let restarted_row = restarted.registry().get(&info.entry_id).unwrap().unwrap();
    assert!(
        !restarted_row.terminal_drain_validation_failed,
        "startup verifies the terminal drain without inventing terminal corruption; cycles={:?}",
        restarted.cycle_errors(),
    );
    restarted.stop().await;
    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test]
async fn overdue_deadline_exports_the_exact_signed_boundary_and_deadline_manifest_reason() {
    let authority = goalcup_devserver::ReceiptAuthority::new_event("goal-cup-2026-v1")
        .expect("the V4 questionnaire authority is valid");
    let server =
        std::sync::Arc::new(goalcup_devserver::DevServer::with_authority(authority).unwrap());
    server.set_clock(&goalcup_protocol::control::format_z(
        (goalcup_daemon::now_unix() as u64)
            .saturating_sub(goalcup_devserver::ENTRY_WINDOW_SECONDS)
            .saturating_sub(60),
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let routed = server.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, goalcup_devserver::dev_router(routed)).await;
    });

    let h = support::empty_control_harness_on(server, &format!("http://{addr}"), |config| {
        config.receipt_binding = support::v4_binding(128);
    })
    .await;
    let (daemon, client, config) = (h.daemon, h.client, h.config);
    let mut opts = support::workspace_opts("claude-code-cli");
    opts.attach = false;
    let info: goalcup_protocol::control::WorkspaceInfo = client
        .post_json(goalcup_protocol::control::ROUTE_WORKSPACES, &opts)
        .await;
    daemon.arm(&info.entry_id).unwrap();
    let mut child = support::daemon::spawn_harness();
    daemon
        .register_process(
            &info.entry_id,
            support::process_registration(
                info.process_generation,
                support::daemon::handle_for_now(child.id()),
            ),
        )
        .unwrap();
    let workspace = std::path::Path::new(&info.workspace);
    let home = std::path::Path::new(&info.harness_home);
    support::append_line(
        &support::daemon::transcript_path(home, workspace, &info.session_id),
        &support::daemon::transcript_line(workspace, &info.session_id, "the goal"),
    );
    daemon.tick().await.unwrap();
    std::fs::write(workspace.join("answer.txt"), b"deadline answer\n").unwrap();
    let before = support::read_chain(&std::path::Path::new(&info.artifact_root).join("trace"));
    daemon.stop().await;
    support::append_line(
        &support::daemon::transcript_path(home, workspace, &info.session_id),
        &support::daemon::transcript_line(
            workspace,
            &info.session_id,
            "written while the daemon was down after the signed deadline",
        ),
    );

    let restarted = Daemon::start(config).await.unwrap();
    let row: serde_json::Value = serde_json::from_slice(
        &restarted
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(row["sealing_intent"]["reason"], "deadline");
    assert_eq!(
        row["sealing_intent"]["cutoff_at"], row["deadline_at"],
        "deadline finalization latches the exact organizer-signed instant",
    );
    let chain = support::read_chain(&std::path::Path::new(&info.artifact_root).join("trace"));
    let cutoff = chain
        .iter()
        .find(|event| {
            matches!(
                event.body,
                goalcup_protocol::envelope::EventBody::CutoffReachedV1(_)
            )
        })
        .unwrap();
    assert_eq!(cutoff.recorded_at, row["deadline_at"]);
    assert_eq!(
        cutoff.link.seq,
        before.last().unwrap().link.seq + 1,
        "startup closes the signed boundary before a restart or tailer catch-up link",
    );
    assert_eq!(cutoff.link.prev, before.last().unwrap().link.hash);
    assert!(!chain.iter().any(|event| matches!(
        event.body,
        goalcup_protocol::envelope::EventBody::DaemonRestart { .. }
    )));
    assert_eq!(
        row["sealing_intent"]["source_root"],
        serde_json::json!({
            "availability": "unavailable_at_cutoff",
            "reason": "recovery_ambiguous"
        }),
    );
    assert_eq!(
        row["sealing_intent"]["submission"],
        serde_json::json!({ "incomplete": { "reason": "recovery_ambiguous" } }),
    );
    assert_eq!(
        restarted
            .out_of_window_stats_for_test(&info.entry_id)
            .unwrap(),
        (1, 0),
        "the unread downtime tailer record is retained privately, never chained",
    );
    assert!(
        !std::path::Path::new(&info.artifact_root)
            .join("trace/submission/manifest.json")
            .exists(),
        "startup does not reopen or resnapshot the workspace after losing cutoff authority",
    );
    assert!(matches!(
        chain.last().unwrap().body,
        goalcup_protocol::envelope::EventBody::FinalizationV1(_)
    ));
    restarted.stop().await;
    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test]
async fn observed_deadline_survives_clock_rollback_and_defeats_later_manual_finish() {
    use goalcup_daemon::registry::FinalizationPhaseV1;

    let h = support::v4_daemon(128).await;
    let (daemon, client) = (h.daemon, h.client);
    let mut opts = support::workspace_opts("claude-code-cli");
    opts.attach = false;
    let info: goalcup_protocol::control::WorkspaceInfo = client
        .post_json(goalcup_protocol::control::ROUTE_WORKSPACES, &opts)
        .await;
    daemon.arm(&info.entry_id).unwrap();
    let mut child = support::daemon::spawn_harness();
    daemon
        .register_process(
            &info.entry_id,
            support::process_registration(
                info.process_generation,
                support::daemon::handle_for_now(child.id()),
            ),
        )
        .unwrap();
    let workspace = std::path::Path::new(&info.workspace);
    let home = std::path::Path::new(&info.harness_home);
    support::append_line(
        &support::daemon::transcript_path(home, workspace, &info.session_id),
        &support::daemon::transcript_line(workspace, &info.session_id, "the goal"),
    );
    daemon.tick().await.unwrap();
    let started = daemon.registry().get(&info.entry_id).unwrap().unwrap();
    let deadline =
        goalcup_protocol::control::parse_z(started.deadline_at.as_deref().unwrap()).unwrap();

    daemon.set_finalization_clock_for_test(deadline as f64);
    daemon.fail_workspace_finalization_after_for_test(FinalizationPhaseV1::AdmissionClosed);
    assert!(daemon.seal_due_entries().is_err());
    daemon.set_finalization_clock_for_test(deadline.saturating_sub(3_600) as f64);
    let _report = daemon
        .finish_entry(&info.entry_id, FinishReasonV1::Manual)
        .unwrap();
    let row: serde_json::Value = serde_json::from_slice(
        &daemon
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(row["deadline_observed"], true);
    assert_eq!(row["sealing_intent"]["reason"], "deadline");
    assert_eq!(row["sealing_intent"]["cutoff_at"], row["deadline_at"]);
    assert!(
        goalcup_protocol::control::parse_z(
            row["sealing_intent"]["finalization_at"].as_str().unwrap()
        )
        .unwrap()
            >= deadline,
        "phase timestamps use the same monotone high-water after wall rollback",
    );
    daemon.stop().await;
    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admission_samples_time_inside_the_cas_and_restart_preserves_forward_observation() {
    let h = support::v4_daemon(128).await;
    let (daemon, client, config) = (h.daemon, h.client, h.config);
    let mut entries = Vec::new();
    let mut children = Vec::new();
    for goal in ["cas-boundary", "restart-high-water", "automatic-due"] {
        let mut opts = support::workspace_opts("claude-code-cli");
        opts.attach = false;
        let info: goalcup_protocol::control::WorkspaceInfo = client
            .post_json(goalcup_protocol::control::ROUTE_WORKSPACES, &opts)
            .await;
        daemon.arm(&info.entry_id).unwrap();
        let child = support::daemon::spawn_harness();
        daemon
            .register_process(
                &info.entry_id,
                support::process_registration(
                    info.process_generation,
                    support::daemon::handle_for_now(child.id()),
                ),
            )
            .unwrap();
        let workspace = std::path::Path::new(&info.workspace);
        support::append_line(
            &support::daemon::transcript_path(
                std::path::Path::new(&info.harness_home),
                workspace,
                &info.session_id,
            ),
            &support::daemon::transcript_line(workspace, &info.session_id, goal),
        );
        daemon.tick().await.unwrap();
        entries.push(info);
        children.push(child);
    }

    let second = &entries[1];
    let second_row = daemon.registry().get(&second.entry_id).unwrap().unwrap();
    let second_deadline =
        goalcup_protocol::control::parse_z(second_row.deadline_at.as_deref().unwrap()).unwrap();
    let forward = second_deadline.saturating_sub(60);
    daemon.set_finalization_clock_for_test(forward as f64);
    assert!(daemon.seal_due_entries().unwrap().is_empty());
    let observed: serde_json::Value = serde_json::from_slice(
        &daemon
            .registry()
            .raw_json_for_test(&second.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(observed["finalization_observed_unix"], forward);
    daemon.stop().await;

    let restarted = Daemon::start(config.clone()).await.unwrap();
    restarted.set_finalization_clock_for_test(second_deadline.saturating_sub(600) as f64);
    restarted
        .finish_entry(&second.entry_id, FinishReasonV1::Manual)
        .unwrap();
    let after_restart: serde_json::Value = serde_json::from_slice(
        &restarted
            .registry()
            .raw_json_for_test(&second.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(after_restart["sealing_intent"]["reason"], "manual");
    assert_eq!(
        after_restart["sealing_intent"]["cutoff_at"],
        goalcup_protocol::control::format_z(forward),
        "a process restart and wall rollback cannot move the persisted observation backward",
    );

    let first = &entries[0];
    let first_row = restarted.registry().get(&first.entry_id).unwrap().unwrap();
    let first_deadline =
        goalcup_protocol::control::parse_z(first_row.deadline_at.as_deref().unwrap()).unwrap();
    restarted.set_finalization_clock_for_test(first_deadline.saturating_sub(1) as f64);
    std::thread::scope(|scope| {
        // Armed inside the scope, so a panic below drops (and releases) the pause before
        // the scope joins the finalizer.
        let pause = restarted
            .pause_workspace_intent_cas_for_test(&first.entry_id)
            .unwrap();
        let finish =
            scope.spawn(|| restarted.finish_entry(&first.entry_id, FinishReasonV1::Manual));
        pause
            .wait_until_paused()
            .expect("manual finalization reaches the serialized admission boundary");
        restarted.set_finalization_clock_for_test(first_deadline as f64);
        pause.release().unwrap();
        finish.join().unwrap().unwrap();
    });
    let first_raw: serde_json::Value = serde_json::from_slice(
        &restarted
            .registry()
            .raw_json_for_test(&first.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first_raw["sealing_intent"]["reason"], "deadline");
    assert_eq!(
        first_raw["sealing_intent"]["cutoff_at"], first_raw["deadline_at"],
        "time is sampled inside the serialized CAS, after source preparation",
    );

    let automatic = &entries[2];
    let automatic_row = restarted
        .registry()
        .get(&automatic.entry_id)
        .unwrap()
        .unwrap();
    let automatic_deadline =
        goalcup_protocol::control::parse_z(automatic_row.deadline_at.as_deref().unwrap()).unwrap();
    restarted.set_finalization_clock_for_test(automatic_deadline as f64);
    let sealed = restarted.seal_due_entries().unwrap();
    assert_eq!(sealed, vec![automatic.entry_id.clone()]);
    let automatic_raw: serde_json::Value = serde_json::from_slice(
        &restarted
            .registry()
            .raw_json_for_test(&automatic.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(automatic_raw["sealing_intent"]["reason"], "deadline");
    assert_eq!(
        automatic_raw["sealing_intent"]["cutoff_at"],
        automatic_raw["deadline_at"],
    );
    assert_eq!(
        automatic_raw["sealing_intent"]["generation"],
        serde_json::json!(automatic.process_generation),
    );
    restarted.stop().await;
    for mut child in children {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[tokio::test]
async fn finalization_api_serializes_manual_manual_deadline_deadline_and_mixed_races() {
    let h = support::v4_daemon(128).await;
    let (daemon, client) = (h.daemon, h.client);
    let mut entries = Vec::new();
    let mut children = Vec::new();
    for _ in ["manual-manual", "deadline-deadline", "manual-deadline"] {
        let mut opts = support::workspace_opts("claude-code-cli");
        opts.attach = false;
        let info: goalcup_protocol::control::WorkspaceInfo = client
            .post_json(goalcup_protocol::control::ROUTE_WORKSPACES, &opts)
            .await;
        daemon.arm(&info.entry_id).unwrap();
        let child = support::daemon::spawn_harness();
        daemon
            .register_process(
                &info.entry_id,
                support::process_registration(
                    info.process_generation,
                    support::daemon::handle_for_now(child.id()),
                ),
            )
            .unwrap();
        let workspace = std::path::Path::new(&info.workspace);
        let home = std::path::Path::new(&info.harness_home);
        support::append_line(
            &support::daemon::transcript_path(home, workspace, &info.session_id),
            &support::daemon::transcript_line(workspace, &info.session_id, "the goal"),
        );
        daemon.tick().await.unwrap();
        entries.push(info);
        children.push(child);
    }

    for (index, (left_reason, right_reason, expected_reason)) in [
        (
            FinishReasonV1::Manual,
            FinishReasonV1::Manual,
            FinishReasonV1::Manual,
        ),
        (
            FinishReasonV1::Deadline,
            FinishReasonV1::Deadline,
            FinishReasonV1::Deadline,
        ),
        (
            FinishReasonV1::Manual,
            FinishReasonV1::Deadline,
            FinishReasonV1::Deadline,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let info = &entries[index];
        let row = daemon.registry().get(&info.entry_id).unwrap().unwrap();
        let deadline =
            goalcup_protocol::control::parse_z(row.deadline_at.as_deref().unwrap()).unwrap();
        if expected_reason == FinishReasonV1::Deadline {
            daemon.set_finalization_clock_for_test(deadline as f64);
        }
        let barrier = std::sync::Barrier::new(3);
        let (left, right) = std::thread::scope(|scope| {
            let left = scope.spawn(|| {
                barrier.wait();
                daemon.finish_entry(&info.entry_id, left_reason)
            });
            let right = scope.spawn(|| {
                barrier.wait();
                daemon.finish_entry(&info.entry_id, right_reason)
            });
            barrier.wait();
            (left.join().unwrap(), right.join().unwrap())
        });
        assert!(
            left.is_ok() || right.is_ok(),
            "one API caller wins the slot"
        );
        let retry = daemon
            .finish_entry(&info.entry_id, expected_reason)
            .unwrap();
        if let Ok(first) = left {
            assert_eq!(first, retry);
        }
        if let Ok(first) = right {
            assert_eq!(first, retry);
        }
        let raw: serde_json::Value = serde_json::from_slice(
            &daemon
                .registry()
                .raw_json_for_test(&info.entry_id)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            raw["sealing_intent"]["reason"],
            serde_json::to_value(expected_reason).unwrap()
        );
        assert_eq!(
            raw["sealing_intent"]["generation"],
            serde_json::json!(info.process_generation),
            "every racing caller converges on the accepted handoff generation",
        );
        assert!(info.process_generation > 0);
        if expected_reason == FinishReasonV1::Deadline {
            assert_eq!(raw["sealing_intent"]["cutoff_at"], raw["deadline_at"]);
        }
        let chain = support::read_chain(&std::path::Path::new(&info.artifact_root).join("trace"));
        assert_eq!(
            chain
                .iter()
                .filter(|event| matches!(
                    event.body,
                    goalcup_protocol::envelope::EventBody::CutoffReachedV1(_)
                ))
                .count(),
            1
        );
        assert_eq!(
            chain
                .iter()
                .filter(|event| matches!(
                    event.body,
                    goalcup_protocol::envelope::EventBody::FinalizationV1(_)
                ))
                .count(),
            1
        );
    }
    daemon.stop().await;
    for mut child in children {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[tokio::test]
async fn live_and_recovery_tailers_filter_root_session_and_replay_each_coordinate_once() {
    let h = support::v4_daemon(2).await;
    let (daemon, client, config) = (h.daemon, h.client, h.config);
    let mut opts = support::workspace_opts("codex-cli");
    opts.attach = false;
    let info: goalcup_protocol::control::WorkspaceInfo = client
        .post_json(goalcup_protocol::control::ROUTE_WORKSPACES, &opts)
        .await;
    daemon.arm(&info.entry_id).unwrap();
    let mut child = std::process::Command::new("/bin/sleep")
        .arg("120")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    daemon
        .register_process(
            &info.entry_id,
            support::process_registration(
                info.process_generation,
                support::daemon::handle_for_now(child.id()),
            ),
        )
        .unwrap();
    let workspace = std::path::Path::new(&info.workspace);
    let home = std::path::Path::new(&info.harness_home);
    let created: serde_json::Value = serde_json::from_slice(
        &daemon
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let date = created["source_date"].as_array().unwrap();
    let dated = home
        .join("sessions")
        .join(date[0].as_str().unwrap())
        .join(date[1].as_str().unwrap())
        .join(date[2].as_str().unwrap());
    std::fs::create_dir_all(&dated).unwrap();
    let tracked = dated.join(format!("rollout-tracked-{}.jsonl", info.session_id));
    let foreign = dated.join("rollout-foreign-session.jsonl");
    let meta = |session: &str, cwd: &std::path::Path| {
        serde_json::json!({
            "timestamp": "2026-09-15T00:00:00Z",
            "type": "session_meta",
            "payload": { "session_id": session, "id": session, "cwd": cwd, "cli_version": "0.153.0" }
        })
        .to_string()
    };
    support::append_line(&tracked, &meta(&info.session_id, workspace));
    support::append_line(&foreign, &meta("foreign-session", workspace));
    std::fs::write(home.join("history.jsonl"), b"").unwrap();
    daemon.tick().await.unwrap();
    assert_eq!(
        daemon
            .registry()
            .get(&info.entry_id)
            .unwrap()
            .unwrap()
            .state,
        goalcup_daemon::registry::EntryState::Started,
    );

    let late = serde_json::json!({
        "timestamp": "2026-09-15T00:00:01Z",
        "type": "response_item",
        "payload": { "text": "same late bytes at distinct source coordinates" }
    })
    .to_string();
    support::append_line(&tracked, &late);
    support::append_line(&foreign, &late);
    daemon.fail_after_workspace_status_publish_for_test();
    assert!(
        daemon
            .finish_entry(&info.entry_id, FinishReasonV1::Manual)
            .is_err()
    );
    assert_eq!(
        daemon.out_of_window_stats_for_test(&info.entry_id).unwrap(),
        (1, 0),
        "live cutoff must retain only the bound root session",
    );
    daemon.stop().await;
    support::append_line(&tracked, &late);
    support::append_line(&foreign, &late);
    let decoy = home.join("sessions/2999/12/31");
    std::fs::create_dir_all(&decoy).unwrap();
    std::fs::write(
        decoy.join("rollout-midnight-decoy.jsonl"),
        format!("{}\n", meta(&info.session_id, workspace)),
    )
    .unwrap();

    let restarted = Daemon::start(config.clone()).await.unwrap();
    assert_eq!(
        restarted
            .out_of_window_stats_for_test(&info.entry_id)
            .unwrap(),
        (2, 0),
        "recovery stores the distinct tracked coordinate once and never the foreign session",
    );
    let recovered: serde_json::Value = serde_json::from_slice(
        &restarted
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        recovered["sealing_intent"]["source_date"],
        created["source_date"]
    );
    restarted.stop().await;
    let restarted_again = Daemon::start(config).await.unwrap();
    assert_eq!(
        restarted_again
            .out_of_window_stats_for_test(&info.entry_id)
            .unwrap(),
        (2, 0),
        "a second restart neither replays nor overflows already-retained late records",
    );
    restarted_again.stop().await;
    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test]
async fn restart_after_cutoff_bind_finishes_without_reopening_or_resnapshotting_workspace() {
    let h = support::v4_daemon(128).await;
    let (daemon, client, config) = (h.daemon, h.client, h.config);
    let mut opts = support::workspace_opts("claude-code-cli");
    opts.attach = false;
    let info: goalcup_protocol::control::WorkspaceInfo = client
        .post_json(goalcup_protocol::control::ROUTE_WORKSPACES, &opts)
        .await;
    daemon.arm(&info.entry_id).unwrap();
    let mut child = support::daemon::spawn_harness();
    daemon
        .register_process(
            &info.entry_id,
            support::process_registration(
                info.process_generation,
                support::daemon::handle_for_now(child.id()),
            ),
        )
        .unwrap();
    let workspace = std::path::Path::new(&info.workspace);
    let home = std::path::Path::new(&info.harness_home);
    support::append_line(
        &support::daemon::transcript_path(home, workspace, &info.session_id),
        &support::daemon::transcript_line(workspace, &info.session_id, "the goal"),
    );
    daemon.tick().await.unwrap();
    std::fs::write(workspace.join("answer.txt"), b"bytes before cutoff\n").unwrap();

    let signed_deadline = goalcup_protocol::control::parse_z(
        daemon
            .registry()
            .get(&info.entry_id)
            .unwrap()
            .unwrap()
            .deadline_at
            .as_deref()
            .unwrap(),
    )
    .unwrap();
    let drain_start = signed_deadline.saturating_sub(60) as f64;
    daemon.set_finalization_clock_for_test(drain_start);
    daemon.set_finalization_clock_steps_for_test([drain_start, drain_start, drain_start + 1.0]);

    daemon.fail_after_workspace_cutoff_append_for_test();
    assert!(
        daemon
            .finish_entry(&info.entry_id, FinishReasonV1::Manual)
            .is_err(),
        "the injected crash boundary interrupts after the cutoff link is durably bound",
    );
    let interrupted: serde_json::Value = serde_json::from_slice(
        &daemon
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        interrupted["sealing_intent"]["phase"], "cutoff_append_pending",
        "the fault lands after the chain side effect and before its registry bind",
    );
    assert_eq!(
        daemon.out_of_window_stats_for_test(&info.entry_id).unwrap(),
        (0, 0),
        "nothing was admitted after the cutoff before the crash",
    );
    daemon.stop().await;

    std::fs::write(workspace.join("answer.txt"), b"new bytes after cutoff\n").unwrap();
    support::append_line(
        &support::daemon::transcript_path(home, workspace, &info.session_id),
        &support::daemon::transcript_line(
            workspace,
            &info.session_id,
            "late recovery tailer sk-ant-api03-recovery-secret",
        ),
    );
    let restarted = Daemon::start(config.clone()).await.unwrap();
    let recovered: serde_json::Value = serde_json::from_slice(
        &restarted
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        recovered["sealing_intent"]["phase"], "terminal",
        "startup completes the existing intent rather than reopening capture",
    );
    let chain = support::read_chain(&std::path::Path::new(&info.artifact_root).join("trace"));
    assert_eq!(
        chain
            .iter()
            .filter(|event| matches!(
                event.body,
                goalcup_protocol::envelope::EventBody::CutoffReachedV1(_)
            ))
            .count(),
        1,
    );
    assert_eq!(
        chain
            .iter()
            .filter(|event| matches!(
                event.body,
                goalcup_protocol::envelope::EventBody::FinalizationV1(_)
            ))
            .count(),
        1,
    );
    assert_eq!(
        recovered["sealing_intent"]["submission"],
        serde_json::json!({ "incomplete": { "reason": "recovery_ambiguous" } }),
    );
    assert_eq!(
        restarted
            .out_of_window_stats_for_test(&info.entry_id)
            .unwrap(),
        (1, 0),
        "the recovered tailer bytes are encrypted OOW evidence",
    );
    assert!(
        !serde_json::to_string(&chain)
            .unwrap()
            .contains("recovery-secret"),
        "post-cutoff tailer bytes never enter the qualifying chain on restart",
    );
    let mut pending = recovered;
    pending["sealing_intent"]["phase"] =
        serde_json::Value::String("terminal_status_pending".to_string());
    pending["sealing_intent"]["status_mirror_complete"] = serde_json::Value::Bool(false);
    let admitted_process_walk =
        pending["receipt_binding"]["effective_limits"]["process_walk"].clone();
    pending["receipt_binding"]["effective_limits"]["process_walk"] = serde_json::json!(17);
    restarted
        .registry()
        .replace_raw_json_for_test(&info.entry_id, &serde_json::to_vec(&pending).unwrap())
        .unwrap();
    restarted.stop().await;
    let link_count = chain.len();
    let refused_policy = Daemon::start(config.clone()).await.unwrap();
    let refused_policy_row: serde_json::Value = serde_json::from_slice(
        &refused_policy
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        refused_policy_row["sealing_intent"]["phase"],
        "terminal_status_pending",
    );
    assert_eq!(refused_policy_row["terminal_drain_validation_failed"], true);
    assert_eq!(
        support::read_chain(&std::path::Path::new(&info.artifact_root).join("trace")).len(),
        link_count,
        "recovery refuses mutable row limits that do not match configured receipt authority",
    );
    pending["receipt_binding"]["effective_limits"]["process_walk"] = admitted_process_walk;
    let admitted_manifest_sha256 = pending["manifest_sha256"].clone();
    pending["manifest_sha256"] = serde_json::Value::String("00".repeat(32));
    refused_policy
        .registry()
        .replace_raw_json_for_test(&info.entry_id, &serde_json::to_vec(&pending).unwrap())
        .unwrap();
    refused_policy.stop().await;
    let refused = Daemon::start(config.clone()).await.unwrap();
    let refused_row: serde_json::Value = serde_json::from_slice(
        &refused
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        refused_row["sealing_intent"]["phase"],
        "terminal_status_pending",
    );
    assert_eq!(refused_row["terminal_drain_validation_failed"], true);
    assert_eq!(
        support::read_chain(&std::path::Path::new(&info.artifact_root).join("trace")).len(),
        link_count,
        "a changed shipped manifest is refused before recovery can poll or append",
    );
    pending["manifest_sha256"] = admitted_manifest_sha256;
    refused
        .registry()
        .replace_raw_json_for_test(&info.entry_id, &serde_json::to_vec(&pending).unwrap())
        .unwrap();
    std::fs::write(
        std::path::Path::new(&info.artifact_root).join("cursors.json"),
        b"{malformed",
    )
    .unwrap();
    refused.stop().await;
    let corrupt_cursor = Daemon::start(config.clone()).await.unwrap();
    let corrupt_row: serde_json::Value = serde_json::from_slice(
        &corrupt_cursor
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        corrupt_row["sealing_intent"]["phase"],
        "terminal_status_pending",
    );
    assert_eq!(corrupt_row["terminal_drain_validation_failed"], true);
    assert_eq!(
        support::read_chain(&std::path::Path::new(&info.artifact_root).join("trace")).len(),
        link_count,
        "a malformed cutoff cursor never falls back to a read from zero",
    );
    corrupt_cursor.stop().await;
    std::fs::remove_file(std::path::Path::new(&info.artifact_root).join("cursors.json")).unwrap();
    let missing_cursor = Daemon::start(config).await.unwrap();
    let missing_row: serde_json::Value = serde_json::from_slice(
        &missing_cursor
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        missing_row["sealing_intent"]["phase"],
        "terminal_status_pending",
    );
    assert_eq!(missing_row["terminal_drain_validation_failed"], true);
    assert_eq!(
        support::read_chain(&std::path::Path::new(&info.artifact_root).join("trace")).len(),
        link_count,
        "a missing cutoff cursor never falls back to a read from zero",
    );
    missing_cursor.stop().await;
    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test]
async fn restart_persists_a_nonclean_marker_before_refusing_an_overflowed_sealing_buffer() {
    use goalcup_daemon::registry::FinalizationPhaseV1;

    let h = support::v4_daemon(128).await;
    let (daemon, client, config) = (h.daemon, h.client, h.config);
    let mut opts = support::workspace_opts("claude-code-cli");
    opts.attach = false;
    let info: goalcup_protocol::control::WorkspaceInfo = client
        .post_json(goalcup_protocol::control::ROUTE_WORKSPACES, &opts)
        .await;
    daemon.arm(&info.entry_id).unwrap();
    let mut child = support::daemon::spawn_harness();
    daemon
        .register_process(
            &info.entry_id,
            support::process_registration(
                info.process_generation,
                support::daemon::handle_for_now(child.id()),
            ),
        )
        .unwrap();
    let workspace = std::path::Path::new(&info.workspace);
    let home = std::path::Path::new(&info.harness_home);
    let transcript = support::daemon::transcript_path(home, workspace, &info.session_id);
    support::append_line(
        &transcript,
        &support::daemon::transcript_line(workspace, &info.session_id, "the goal"),
    );
    daemon.tick().await.unwrap();

    // One transcript record past the cutoff, whose retained form is larger than the
    // out-of-window partition's per-record cap (`total_bytes`, 1 MiB here), appended while
    // blocking capture owns the runtime. The live post-cutoff retention reads it, journals the overflow, and only then
    // persists the registry marker; the injected fault after capture leaves the row Sealing.
    // The tailer discards a line above `MAX_LINE_BYTES` (1 MiB) as `OverlongLine`, so the
    // largest line it hands on is exactly at that cap. Its retained record adds the
    // canonical wrapping and so exceeds the partition's 1 MiB per-record cap.
    let wrapping = support::daemon::transcript_line(workspace, &info.session_id, "").len();
    let at_the_line_cap = support::daemon::transcript_line(
        workspace,
        &info.session_id,
        &"x".repeat(goalcup_daemon::MAX_LINE_BYTES as usize - wrapping),
    );
    assert_eq!(at_the_line_cap.len() as u64, goalcup_daemon::MAX_LINE_BYTES);
    daemon.fail_after_workspace_capture_for_test();
    std::thread::scope(|scope| {
        // Armed inside the scope, so a panic below drops (and releases) the pause before
        // the scope joins the finalizer.
        let capture = daemon
            .pause_workspace_capture_for_test(&info.entry_id)
            .unwrap();
        let daemon_ref = &daemon;
        let entry_id = &info.entry_id;
        let finish = scope.spawn(move || daemon_ref.finish_entry(entry_id, FinishReasonV1::Manual));
        capture
            .wait_until_paused()
            .expect("capture reaches the deterministic filesystem-work boundary");
        let paused: serde_json::Value = serde_json::from_slice(
            &daemon
                .registry()
                .raw_json_for_test(&info.entry_id)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(
            FinalizationPhaseV1::CutoffBound.permits(
                serde_json::from_value::<FinalizationPhaseV1>(
                    paused["sealing_intent"]["phase"].clone()
                )
                .unwrap()
            ),
            "the oversized record lands after the cutoff is bound",
        );
        support::append_line(&transcript, &at_the_line_cap);
        capture.release().unwrap();
        assert!(finish.join().unwrap().is_err());
    });
    assert_eq!(
        daemon.out_of_window_stats_for_test(&info.entry_id).unwrap(),
        (0, 1),
        "the oversized record is journaled as overflow, never as a retained record",
    );
    let mut raw: serde_json::Value = serde_json::from_slice(
        &daemon
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        raw["terminal_drain_validation_failed"], true,
        "the overflow journal is durable before its registry marker",
    );
    // Clear the marker, so the restart alone must persist it again.
    raw["terminal_drain_validation_failed"] = serde_json::Value::Bool(false);
    daemon
        .registry()
        .replace_raw_json_for_test(&info.entry_id, &serde_json::to_vec(&raw).unwrap())
        .unwrap();
    daemon.stop().await;

    let restarted = Daemon::start(config).await.unwrap();
    let recovered = restarted.registry().get(&info.entry_id).unwrap().unwrap();
    assert!(
        recovered.terminal_drain_validation_failed,
        "startup must persist the externally visible nonclean result before refusing recovery",
    );
    assert_eq!(
        recovered.state,
        goalcup_daemon::registry::EntryState::Sealing,
        "an overflowed recovery owner never advances or reopens the entry",
    );
    let recovered_raw: serde_json::Value = serde_json::from_slice(
        &restarted
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        recovered_raw["sealing_intent"]["phase"], raw["sealing_intent"]["phase"],
        "the refused recovery leaves the persisted phase where the live finalizer stopped",
    );
    restarted.stop().await;
    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn writer_fault_across_the_live_drain_and_blocking_capture_keep_post_cutoff_tailers_private()
{
    let h = support::v4_daemon(3).await;
    let (daemon, client, config) = (h.daemon, h.client, h.config);
    let mut opts = support::workspace_opts("claude-code-cli");
    opts.attach = false;
    let info: goalcup_protocol::control::WorkspaceInfo = client
        .post_json(goalcup_protocol::control::ROUTE_WORKSPACES, &opts)
        .await;
    daemon.arm(&info.entry_id).unwrap();
    let mut child = support::daemon::spawn_harness();
    daemon
        .register_process(
            &info.entry_id,
            support::process_registration(
                info.process_generation,
                support::daemon::handle_for_now(child.id()),
            ),
        )
        .unwrap();
    let workspace = std::path::Path::new(&info.workspace);
    let home = std::path::Path::new(&info.harness_home);
    let transcript = support::daemon::transcript_path(home, workspace, &info.session_id);
    support::append_line(
        &transcript,
        &support::daemon::transcript_line(workspace, &info.session_id, "the goal"),
    );
    daemon.tick().await.unwrap();
    std::fs::write(workspace.join("answer.txt"), b"captured answer\n").unwrap();

    // The writer refuses its very next append until the finalization drain returns.
    daemon
        .fail_workspace_append_after_for_test(&info.entry_id, 0)
        .unwrap();
    const ADMISSION_SECRET: &str = "sk-ant-api03-admission-window-secret";
    const CUTOFF_SECRET: &str = "sk-ant-api03-post-cutoff-secret";
    std::thread::scope(|scope| {
        // Armed inside the scope, so a panic below drops (and releases) both pauses before
        // the scope joins the finalizer.
        let cas = daemon
            .pause_workspace_intent_cas_for_test(&info.entry_id)
            .unwrap();
        let capture = daemon
            .pause_workspace_capture_for_test(&info.entry_id)
            .unwrap();
        let daemon_ref = &daemon;
        let entry_id = &info.entry_id;
        let finish = scope.spawn(move || daemon_ref.finish_entry(entry_id, FinishReasonV1::Manual));
        cas.wait_until_paused()
            .expect("finalization reaches the admission CAS with its watermarks taken");
        // Written after the tailer watermark: the live drain reads these and retains them
        // out of window. It never chains them, so the armed writer fault is never reached.
        for n in 0..2 {
            support::append_line(
                &transcript,
                &support::daemon::transcript_line(
                    workspace,
                    &info.session_id,
                    &format!("drain bytes {ADMISSION_SECRET} {n}"),
                ),
            );
        }
        cas.release().unwrap();
        capture
            .wait_until_paused()
            .expect("capture reaches the deterministic filesystem-work boundary");
        // Written while blocking capture owns the runtime: one fills the partition's third
        // and last record, the other overflows it.
        for n in 0..2 {
            support::append_line(
                &transcript,
                &support::daemon::transcript_line(
                    workspace,
                    &info.session_id,
                    &format!("tailer bytes {CUTOFF_SECRET} {n} after admission closed"),
                ),
            );
        }
        capture.release().unwrap();
        assert!(
            finish.join().unwrap().is_err(),
            "an OOW overflow must refuse terminal publication"
        );
    });
    assert_eq!(
        daemon.out_of_window_stats_for_test(&info.entry_id).unwrap(),
        (3, 1),
        "all capacity is durable and the final post-cutoff record is counted as overflow",
    );
    let row: serde_json::Value = serde_json::from_slice(
        &daemon
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        row["sealing_intent"]["drain"],
        serde_json::json!("complete"),
        "the live drain retains leg-1 bytes out of window and never appends, so an armed \
         writer fault cannot degrade it",
    );
    assert_eq!(
        row["sealing_intent"]["phase"], "terminal_status_pending",
        "late/OOW persistence must finish before terminal status publication",
    );
    assert_eq!(row["terminal_drain_validation_failed"], true);
    let chain = support::read_chain(&std::path::Path::new(&info.artifact_root).join("trace"));
    let chain_text = serde_json::to_string(&chain).unwrap();
    for secret in [ADMISSION_SECRET, CUTOFF_SECRET] {
        assert!(
            !chain_text.contains(secret),
            "tailer bytes past the admission watermark never enter the qualifying chain",
        );
        assert!(
            !std::fs::read(std::path::Path::new(&info.artifact_root).join("buffer.redb"))
                .unwrap()
                .windows(secret.len())
                .any(|window| window == secret.as_bytes()),
            "private OOW storage does not retain the tailer secret in plaintext",
        );
    }
    assert_eq!(
        chain
            .iter()
            .filter(|event| matches!(
                event.body,
                goalcup_protocol::envelope::EventBody::Leg1Record { .. }
            ))
            .count(),
        1,
        "only the transcript record chained before admission qualifies",
    );
    assert!(
        chain.iter().any(|event| matches!(
            event.body,
            goalcup_protocol::envelope::EventBody::CutoffReachedV1(_)
        )),
        "the drain cleared the writer fault, so the cutoff link lands",
    );
    let link_count = chain.len();
    daemon.stop().await;
    let restarted = Daemon::start(config).await.unwrap();
    let after_restart: serde_json::Value = serde_json::from_slice(
        &restarted
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        after_restart["sealing_intent"]["phase"],
        "terminal_status_pending"
    );
    assert_eq!(
        after_restart["sealing_intent"]["drain"],
        row["sealing_intent"]["drain"],
    );
    assert_eq!(after_restart["terminal_drain_validation_failed"], true);
    assert_eq!(
        support::read_chain(&std::path::Path::new(&info.artifact_root).join("trace")).len(),
        link_count,
        "restart neither resnapshots nor appends duplicate terminal links after OOW overflow",
    );
    restarted.stop().await;
    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test]
async fn restart_after_termination_binding_never_reopens_an_uncaptured_workspace() {
    let h = support::v4_daemon(128).await;
    let (daemon, client, config) = (h.daemon, h.client, h.config);
    let mut opts = support::workspace_opts("claude-code-cli");
    opts.attach = false;
    let info: goalcup_protocol::control::WorkspaceInfo = client
        .post_json(goalcup_protocol::control::ROUTE_WORKSPACES, &opts)
        .await;
    daemon.arm(&info.entry_id).unwrap();
    let mut child = std::process::Command::new("/bin/sleep")
        .arg("120")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    daemon
        .register_process(
            &info.entry_id,
            support::process_registration(
                info.process_generation,
                support::daemon::handle_for_now(child.id()),
            ),
        )
        .unwrap();
    let workspace = std::path::Path::new(&info.workspace);
    support::append_line(
        &support::daemon::transcript_path(
            std::path::Path::new(&info.harness_home),
            workspace,
            &info.session_id,
        ),
        &support::daemon::transcript_line(workspace, &info.session_id, "before termination"),
    );
    daemon.tick().await.unwrap();
    std::fs::write(workspace.join("answer.txt"), b"never captured\n").unwrap();

    daemon.fail_workspace_finalization_after_for_test(
        goalcup_daemon::registry::FinalizationPhaseV1::Terminated,
    );
    assert!(
        daemon
            .finish_entry(&info.entry_id, FinishReasonV1::Manual)
            .is_err()
    );
    let interrupted: serde_json::Value = serde_json::from_slice(
        &daemon
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(interrupted["sealing_intent"]["phase"], "terminated");
    let trace = std::path::Path::new(&info.artifact_root).join("trace");
    assert!(!trace.join("submission/manifest.json").exists());
    daemon.stop().await;

    std::fs::write(
        workspace.join("answer.txt"),
        b"changed while daemon was down\n",
    )
    .unwrap();
    let restarted = Daemon::start(config).await.unwrap();
    let recovered: serde_json::Value = serde_json::from_slice(
        &restarted
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(recovered["sealing_intent"]["phase"], "terminal");
    assert_eq!(
        recovered["sealing_intent"]["submission"],
        serde_json::json!({ "incomplete": { "reason": "recovery_ambiguous" } }),
        "recovery records the missing capture truth without reopening the workspace",
    );
    assert!(!trace.join("submission/manifest.json").exists());
    // The interrupted run had already passed `terminated`, so the harness was ended before
    // the restart and recovery has no second signal to send. An incomplete capture and a
    // completed termination are independent facts, and this is the pair that proves it.
    assert!(
        !goalcup_protocol::process::is_alive(child.id()),
        "the durable `terminated` phase means the kill already happened",
    );
    restarted.stop().await;
    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test]
async fn restart_from_admission_closed_and_drain_resolved_uses_persisted_watermarks_only() {
    use goalcup_daemon::registry::FinalizationPhaseV1;

    for interrupted_phase in [
        FinalizationPhaseV1::AdmissionClosed,
        FinalizationPhaseV1::DrainResolved,
    ] {
        let h = support::v4_daemon(128).await;
        let (daemon, client, config) = (h.daemon, h.client, h.config);
        let mut opts = support::workspace_opts("claude-code-cli");
        opts.attach = false;
        let info: goalcup_protocol::control::WorkspaceInfo = client
            .post_json(goalcup_protocol::control::ROUTE_WORKSPACES, &opts)
            .await;
        daemon.arm(&info.entry_id).unwrap();
        let mut child = support::daemon::spawn_harness();
        daemon
            .register_process(
                &info.entry_id,
                support::process_registration(
                    info.process_generation,
                    support::daemon::handle_for_now(child.id()),
                ),
            )
            .unwrap();
        let workspace = std::path::Path::new(&info.workspace);
        let home = std::path::Path::new(&info.harness_home);
        support::append_line(
            &support::daemon::transcript_path(home, workspace, &info.session_id),
            &support::daemon::transcript_line(workspace, &info.session_id, "the goal"),
        );
        daemon.tick().await.unwrap();

        daemon.fail_workspace_finalization_after_for_test(interrupted_phase);
        assert!(
            daemon
                .finish_entry(&info.entry_id, FinishReasonV1::Manual)
                .is_err()
        );
        let interrupted: serde_json::Value = serde_json::from_slice(
            &daemon
                .registry()
                .raw_json_for_test(&info.entry_id)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            interrupted["sealing_intent"]["phase"],
            serde_json::to_value(interrupted_phase).unwrap(),
        );
        assert_eq!(
            interrupted["sealing_intent"]["admission_watermarks"]["receiver_sources"],
            serde_json::json!({}),
            "no hook receiver runs, so the persisted receiver watermark is empty",
        );
        assert_eq!(
            interrupted["sealing_intent"]["admission_watermarks"]["queued_receiver_posts"],
            0,
        );
        let watermarks = interrupted["sealing_intent"]["admission_watermarks"].clone();
        daemon.stop().await;
        std::fs::write(
            workspace.join("after-crash.txt"),
            b"must not be snapshotted\n",
        )
        .unwrap();

        let restarted = Daemon::start(config).await.unwrap();
        let recovered: serde_json::Value = serde_json::from_slice(
            &restarted
                .registry()
                .raw_json_for_test(&info.entry_id)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(recovered["sealing_intent"]["phase"], "terminal");
        assert_eq!(
            recovered["sealing_intent"]["admission_watermarks"], watermarks,
            "recovery neither resamples nor replaces admission watermarks",
        );
        assert_eq!(
            recovered["sealing_intent"]["submission"],
            serde_json::json!({ "incomplete": { "reason": "recovery_ambiguous" } }),
        );
        let chain = support::read_chain(&std::path::Path::new(&info.artifact_root).join("trace"));
        assert_eq!(
            chain
                .iter()
                .filter(|event| matches!(
                    event.body,
                    goalcup_protocol::envelope::EventBody::CutoffReachedV1(_)
                ))
                .count(),
            1,
        );
        assert_eq!(
            chain
                .iter()
                .filter(|event| matches!(
                    event.body,
                    goalcup_protocol::envelope::EventBody::FinalizationV1(_)
                ))
                .count(),
            1,
        );
        // Recovery resumes the same intent and completes it, kill included: a daemon that
        // died mid-finalization must not cost the participant the ending the run had
        // already earned.
        assert!(
            !goalcup_protocol::process::is_alive(child.id()),
            "startup recovery finishes the intent it found, including its termination",
        );
        restarted.stop().await;
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn assert_refused_recovery(
    daemon: &Daemon,
    entry_id: &str,
    attack: &str,
    trace: &std::path::Path,
    chain_len: usize,
    buffer_path: &std::path::Path,
    buffer_before: &[u8],
) {
    assert_eq!(
        daemon.workspace_recovery_writer_prepare_calls_for_test(),
        0,
        "{attack} must be refused before prepare_resume opens writer authority",
    );
    let refused: serde_json::Value = serde_json::from_slice(
        &daemon
            .registry()
            .raw_json_for_test(entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        refused["sealing_intent"]["phase"], "admission_closed",
        "{attack} must not advance the durable recovery phase",
    );
    assert_eq!(
        refused["terminal_drain_validation_failed"], true,
        "{attack} remains durably visible as a nonclean recovery",
    );
    assert_eq!(
        support::read_chain(trace).len(),
        chain_len,
        "{attack} must be refused before a recovery append",
    );
    assert_eq!(
        std::fs::read(buffer_path).unwrap(),
        buffer_before,
        "{attack} must be refused before tailer polling or an OOW append",
    );
}

#[tokio::test]
async fn sealing_recovery_refuses_unverified_or_rebound_receipts_before_tailers_or_appends() {
    use goalcup_daemon::registry::FinalizationPhaseV1;
    use goalcup_protocol::receipt::StartReceipt;

    let h = support::v4_daemon(128).await;
    let (daemon, client, config) = (h.daemon, h.client, h.config);
    let mut opts = support::workspace_opts("claude-code-cli");
    opts.attach = false;
    let info: goalcup_protocol::control::WorkspaceInfo = client
        .post_json(goalcup_protocol::control::ROUTE_WORKSPACES, &opts)
        .await;
    daemon.arm(&info.entry_id).unwrap();
    let mut child = support::daemon::spawn_harness();
    daemon
        .register_process(
            &info.entry_id,
            support::process_registration(
                info.process_generation,
                support::daemon::handle_for_now(child.id()),
            ),
        )
        .unwrap();
    let workspace = std::path::Path::new(&info.workspace);
    let home = std::path::Path::new(&info.harness_home);
    support::append_line(
        &support::daemon::transcript_path(home, workspace, &info.session_id),
        &support::daemon::transcript_line(workspace, &info.session_id, "the goal"),
    );
    daemon.tick().await.unwrap();

    daemon.fail_workspace_finalization_after_for_test(FinalizationPhaseV1::AdmissionClosed);
    assert!(
        daemon
            .finish_entry(&info.entry_id, FinishReasonV1::Manual)
            .is_err()
    );
    let clean_raw: serde_json::Value = serde_json::from_slice(
        &daemon
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(clean_raw["sealing_intent"]["phase"], "admission_closed");
    let trace = std::path::Path::new(&info.artifact_root).join("trace");
    let chain_len = support::read_chain(&trace).len();
    let buffer_path = std::path::Path::new(&info.artifact_root).join("buffer.redb");

    let mut forged_raw = clean_raw.clone();
    let mut forged_receipt: StartReceipt =
        serde_json::from_value(forged_raw["receipt"].clone()).unwrap();
    let replacement = if forged_receipt.signature.starts_with('0') {
        "1"
    } else {
        "0"
    };
    forged_receipt.signature.replace_range(0..1, replacement);
    forged_raw["receipt"] = serde_json::to_value(&forged_receipt).unwrap();
    forged_raw["sealing_intent"]["receipt_sha256"] =
        serde_json::to_value(forged_receipt.canonical_digest().unwrap()).unwrap();
    daemon
        .registry()
        .replace_raw_json_for_test(&info.entry_id, &serde_json::to_vec(&forged_raw).unwrap())
        .unwrap();
    daemon.stop().await;
    let buffer_before = std::fs::read(&buffer_path).unwrap();
    support::append_line(
        &support::daemon::transcript_path(home, workspace, &info.session_id),
        &support::daemon::transcript_line(workspace, &info.session_id, "late recovery record"),
    );

    let restarted = Daemon::start(config.clone()).await.unwrap();
    assert_refused_recovery(
        &restarted,
        &info.entry_id,
        "forged signature",
        &trace,
        chain_len,
        &buffer_path,
        &buffer_before,
    );

    let mut rebound_raw = clean_raw;
    let mut rebound_receipt: StartReceipt =
        serde_json::from_value(rebound_raw["receipt"].clone()).unwrap();
    rebound_receipt.claims.workspace = "/tmp/another-entry-workspace".to_string();
    rebound_receipt = StartReceipt::sign(
        rebound_receipt.claims,
        &goalcup_devserver::dev_server_signing_key(),
    )
    .unwrap();
    rebound_raw["receipt"] = serde_json::to_value(&rebound_receipt).unwrap();
    rebound_raw["sealing_intent"]["receipt_sha256"] =
        serde_json::to_value(rebound_receipt.canonical_digest().unwrap()).unwrap();
    restarted
        .registry()
        .replace_raw_json_for_test(&info.entry_id, &serde_json::to_vec(&rebound_raw).unwrap())
        .unwrap();
    restarted.stop().await;

    let restarted = Daemon::start(config).await.unwrap();
    assert_refused_recovery(
        &restarted,
        &info.entry_id,
        "rebound workspace",
        &trace,
        chain_len,
        &buffer_path,
        &buffer_before,
    );
    restarted.stop().await;
    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test]
async fn restart_joins_the_durable_capture_owner_without_resnapshotting_changed_source() {
    let h = support::v4_daemon(128).await;
    let (daemon, client, config) = (h.daemon, h.client, h.config);
    let mut opts = support::workspace_opts("claude-code-cli");
    opts.attach = false;
    let info: goalcup_protocol::control::WorkspaceInfo = client
        .post_json(goalcup_protocol::control::ROUTE_WORKSPACES, &opts)
        .await;
    daemon.arm(&info.entry_id).unwrap();
    let mut child = std::process::Command::new("/bin/sleep")
        .arg("120")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a long-lived harness stand-in starts");
    daemon
        .register_process(
            &info.entry_id,
            support::process_registration(
                info.process_generation,
                support::daemon::handle_for_now(child.id()),
            ),
        )
        .unwrap();
    let workspace = std::path::Path::new(&info.workspace);
    let home = std::path::Path::new(&info.harness_home);
    support::append_line(
        &support::daemon::transcript_path(home, workspace, &info.session_id),
        &support::daemon::transcript_line(workspace, &info.session_id, "the goal"),
    );
    daemon.tick().await.unwrap();
    std::fs::write(workspace.join("answer.txt"), b"captured before crash\n").unwrap();

    daemon.fail_after_workspace_capture_for_test();
    assert!(
        daemon
            .finish_entry(&info.entry_id, FinishReasonV1::Manual)
            .is_err(),
        "the fault lands after writer resolution but before the registry outcome bind",
    );
    let interrupted: serde_json::Value = serde_json::from_slice(
        &daemon
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(interrupted["sealing_intent"]["phase"], "terminated");
    std::fs::write(workspace.join("answer.txt"), b"changed after capture\n").unwrap();

    daemon.fail_after_workspace_finalization_append_for_test();
    assert!(
        daemon
            .finish_entry(&info.entry_id, FinishReasonV1::Manual)
            .is_err()
    );
    let append_pending: serde_json::Value = serde_json::from_slice(
        &daemon
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        append_pending["sealing_intent"]["phase"],
        "finalization_append_pending"
    );
    let trace = std::path::Path::new(&info.artifact_root).join("trace");
    assert_eq!(
        support::read_chain(&trace)
            .iter()
            .filter(|event| matches!(
                event.body,
                goalcup_protocol::envelope::EventBody::FinalizationV1(_)
            ))
            .count(),
        1,
    );

    daemon.fail_after_workspace_status_publish_for_test();
    assert!(
        daemon
            .finish_entry(&info.entry_id, FinishReasonV1::Manual)
            .is_err()
    );
    let status_pending: serde_json::Value = serde_json::from_slice(
        &daemon
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        status_pending["sealing_intent"]["phase"],
        "terminal_status_pending"
    );
    assert!(daemon.signed_status(&info.entry_id).unwrap().is_some());
    assert!(
        !goalcup_protocol::process::is_alive(child.id()),
        "the V4 arm signals before the terminal status publish, so the injected \
         status-publish fault lands after the harness is already gone",
    );
    daemon.stop().await;

    let restarted = Daemon::start(config).await.unwrap();
    assert!(
        !goalcup_protocol::process::is_alive(child.id()),
        "the harness stays gone across the restart: recovery replays the durable outcome \
         rather than manufacturing a second termination",
    );
    let recovered: serde_json::Value = serde_json::from_slice(
        &restarted
            .registry()
            .raw_json_for_test(&info.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(recovered["sealing_intent"]["phase"], "terminal");
    assert!(
        recovered["sealing_intent"]["submission"]["captured"].is_object(),
        "recovery outcome: {}",
        recovered["sealing_intent"]["submission"],
    );
    let manifest: goalcup_protocol::submission::SubmissionManifestV1 =
        serde_json::from_slice(&std::fs::read(trace.join("submission/manifest.json")).unwrap())
            .unwrap();
    let answer = manifest
        .entries
        .iter()
        .find(|entry| entry.path.as_str() == "answer.txt")
        .expect("the pre-cutoff answer is retained");
    assert_eq!(
        std::fs::read(
            trace
                .join("submission/objects/sha256")
                .join(answer.content_sha256.as_str()),
        )
        .unwrap(),
        b"captured before crash\n",
        "recovery consumes immutable retained bytes and never snapshots the changed workspace",
    );
    restarted.stop().await;
    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test]
async fn v4_attach_is_refused_before_writing_a_zero_generation_workspace() {
    let h = support::v4_daemon(128).await;
    let (daemon, client) = (h.daemon, h.client);
    let mut opts = support::workspace_opts("claude-code-cli");
    opts.attach = true;
    let response = client
        .post_body(goalcup_protocol::control::ROUTE_WORKSPACES, &opts)
        .await;
    assert_eq!(response.status(), 422, "{}", response.body());
    assert!(
        response
            .body()
            .contains("accepted process handoff generation")
    );
    assert!(daemon.registry().all().unwrap().is_empty());

    opts.attach = false;
    let unstarted: goalcup_protocol::control::WorkspaceInfo = client
        .post_json(goalcup_protocol::control::ROUTE_WORKSPACES, &opts)
        .await;
    let error = daemon
        .finish_entry(&unstarted.entry_id, FinishReasonV1::Manual)
        .unwrap_err();
    assert!(
        error.to_string().contains("no signed deadline"),
        "an unstarted V4 row has no admission authority: {error}",
    );
    let unstarted_row: serde_json::Value = serde_json::from_slice(
        &daemon
            .registry()
            .raw_json_for_test(&unstarted.entry_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(unstarted_row["sealing_intent"].is_null());
    daemon.stop().await;
}

/// The public entry point has one reason-bearing shape. Deadline cadence and the control
/// handler must not grow separate implementations whose ordering can drift.
fn finish_signature(
    daemon: &Daemon,
    entry_id: &str,
    reason: FinishReasonV1,
) -> Result<FinishReportV1, DaemonError> {
    daemon.finish_entry(entry_id, reason)
}

#[test]
fn manual_and_deadline_share_one_finish_entry_signature() {
    let method = finish_signature;
    let _: fn(&Daemon, &str, FinishReasonV1) -> Result<FinishReportV1, DaemonError> = method;
}

/// Every inclusive phase name says what is already durable. In particular, an incomplete
/// submission is still a resolved submission outcome, and `artifact_verified` would
/// overstate what happened.
#[test]
fn finalization_phase_names_are_monotone_and_truthful() {
    use goalcup_daemon::registry::FinalizationPhaseV1;

    let phases = [
        FinalizationPhaseV1::AdmissionClosed,
        FinalizationPhaseV1::DrainResolved,
        FinalizationPhaseV1::CutoffAppendPending,
        FinalizationPhaseV1::CutoffBound,
        FinalizationPhaseV1::HandoffReconciled,
        FinalizationPhaseV1::Terminated,
        FinalizationPhaseV1::SubmissionResolved,
        FinalizationPhaseV1::FinalizationAppendPending,
        FinalizationPhaseV1::FinalizationBound,
        FinalizationPhaseV1::TerminalStatusPending,
        FinalizationPhaseV1::Terminal,
    ];
    let spellings = phases
        .into_iter()
        .map(|phase| serde_json::to_value(phase).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        spellings,
        [
            "admission_closed",
            "drain_resolved",
            "cutoff_append_pending",
            "cutoff_bound",
            "handoff_reconciled",
            "terminated",
            "submission_resolved",
            "finalization_append_pending",
            "finalization_bound",
            "terminal_status_pending",
            "terminal",
        ]
        .into_iter()
        .map(serde_json::Value::from)
        .collect::<Vec<_>>(),
    );
    assert!(!spellings.iter().any(|value| value == "artifact_verified"));

    for (index, from) in phases.into_iter().enumerate() {
        for (next_index, next) in phases.into_iter().enumerate() {
            assert_eq!(
                from.permits(next),
                index <= next_index,
                "{from:?} must permit exactly its own or a later durable phase",
            );
        }
    }
}
