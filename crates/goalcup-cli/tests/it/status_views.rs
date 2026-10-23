//! The two participant surfaces over one signed document: `goalcup status` and `goalcup
//! entries`. The third, `goalcup ui`, left with D48.
//!
//! The source sweeps use `support::walk("crates/goalcup-cli/src")` because `walk` walks
//! `<repo_root>/<rel>` and `<repo_root>/src` does not exist — a sweep over it would assert
//! nothing at all.

use crate::common::support;
use goalcup_cli::participant;

use goalcup_cli::commands::{
    entries::entries_with,
    status::{status_in, status_with},
};
#[cfg(target_os = "macos")]
use goalcup_protocol::capture_status::AnchorState;
use goalcup_protocol::control::{self, DaemonClient, STALE_AFTER_SECONDS};
use std::path::PathBuf;

/// A configuration directory holding one record for `info`'s run, with `entry_id` written as
/// given.
///
/// `support::cfg_with_entry` writes the record the daemon actually handed over, which is the
/// fixture almost every test here wants. This is the other one: a record that has been
/// EDITED — a cache a second process wrote, or a file a participant's own editor touched —
/// which is what the tests below are about. It writes through the same
/// `entries::write_entry` the real `goalcup start` uses, so what is on disk is the real
/// shape and only its contents are the test's.
fn cfg_with_edited_record(
    tmp: &tempfile::TempDir,
    info: &goalcup_cli::entries::EntryRecord,
    entry_id: &str,
) -> PathBuf {
    let record = goalcup_cli::entries::EntryRecord {
        entry_id: entry_id.to_string(),
        ..info.clone()
    };
    goalcup_cli::entries::write_entry(tmp.path(), &record).unwrap();
    tmp.path().to_path_buf()
}

#[test]
fn status_prints_what_the_signed_payload_says_and_json_still_verifies() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    let mut out = Vec::new();
    status_with(
        &m,
        &cfg,
        Some(&info.entry_id),
        false,
        support::NOW,
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("capture: recording"), "{text}");
    assert!(
        text.contains(&m.status_detail()),
        "the health state in plain words (D25)"
    );

    let mut out = Vec::new();
    status_with(&m, &cfg, Some(&info.entry_id), true, support::NOW, &mut out).unwrap();
    let doc: goalcup_protocol::control::SignedStatus =
        serde_json::from_slice(&out).expect("--json puts one document on stdout and nothing else");
    assert!(
        doc.verify_with(&info.run_public_key).is_ok(),
        "a participant can hand this to an organizer and it still verifies"
    );

    // With no cached entry there is no pinned key, and this command does not improvise one.
    let err = status_with(
        &m,
        &support::empty_cfg(),
        Some(&info.entry_id),
        false,
        support::NOW,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), 1);
    assert!(
        format!("{err}").contains(&info.entry_id),
        "name the entry it cannot check"
    );
}

/// An unverifiable document is REFUSED OUTRIGHT. Printing it with a warning on it would put
/// a deadline and a health state on the screen in the daemon's voice while a sentence above
/// said not to believe any of it. The status route is unauthenticated, so those bytes may be
/// a rogue process's invention and are not evidence of anything.
#[test]
fn a_status_that_does_not_verify_renders_nothing_of_itself_in_either_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let detail = m.status_detail();
    m.tamper_status_payload();
    let cfg = support::cfg_with_entry(tmp.path(), &info);

    let mut out = Vec::new();
    let err = status_with(
        &m,
        &cfg,
        Some(&info.entry_id),
        false,
        support::NOW,
        &mut out,
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), 1);
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains(&info.entry_id),
        "the headline names the entry"
    );
    assert!(
        text.contains("signature did not verify"),
        "and says what failed: {text:?}"
    );
    assert!(
        !text.contains(&detail) && !text.contains("Recording"),
        "nothing out of the payload is rendered: {text:?}"
    );
    for field in ["workspace:", "health:", "arming:", "time remaining"] {
        assert!(
            !text.contains(field),
            "{field} came out of the payload: {text:?}"
        );
    }

    // `--json` prints a small error object and never the document, so a caller parsing
    // stdout is not handed one it would have to know not to trust.
    let mut out = Vec::new();
    let err =
        status_with(&m, &cfg, Some(&info.entry_id), true, support::NOW, &mut out).unwrap_err();
    assert_eq!(err.exit_code(), 1);
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(
        v.get("payload").is_none() && v.get("signature").is_none(),
        "not the document: {v}"
    );
    assert_eq!(v["entry_id"], serde_json::json!(info.entry_id));
    assert!(
        v["error"].as_str().is_some(),
        "a code a caller can branch on"
    );
    assert!(
        v["message"]
            .as_str()
            .unwrap_or_default()
            .contains("signature did not verify"),
        "{v}"
    );

    // The key is the caller's, never the document's.
    let src = std::fs::read_to_string("src/commands/status.rs").unwrap();
    assert!(src.contains("verify_with"));
    assert!(
        !src.contains(".public_key"),
        "that field is a label, not a credential"
    );
}

/// One run key signs every document of the run, so a payload about another entry verifies
/// perfectly. The id is checked separately or a routing bug hands a participant the wrong
/// run's deadline with a good signature on it.
#[test]
fn a_document_about_another_entry_is_refused_naming_both_even_though_it_verifies() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    // The mock answers `status` with its one document whatever id is asked for, which is the
    // shape of the bug: the record is this machine's, the payload is the other entry's, and
    // the run key checks out on both.
    let asked = "entry-mock-0002";
    let cfg = cfg_with_edited_record(&tmp, &info, asked);

    let mut out = Vec::new();
    let err = status_with(&m, &cfg, Some(asked), false, support::NOW, &mut out).unwrap_err();
    assert_eq!(err.exit_code(), 1);
    let said = format!("{err}");
    assert!(
        said.contains(asked) && said.contains(&info.entry_id),
        "both are named: {said:?}"
    );
    let text = String::from_utf8(out).unwrap();
    assert!(
        !text.contains("health:") && !text.contains("workspace:"),
        "and nothing out of the payload is rendered: {text:?}"
    );
}

/// A `cached_at` ahead of this machine's clock must not come back from `stale_seconds` as an age of
/// zero, which reads as "published just now" — the one thing it is not.
#[test]
fn a_document_stamped_ahead_of_this_machine_is_reported_as_clock_skew_and_not_as_fresh() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    let earlier = control::format_z(control::parse_z(support::NOW).unwrap() - 30);

    let mut out = Vec::new();
    status_with(&m, &cfg, Some(&info.entry_id), false, &earlier, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("published in the future by 30s"), "{text:?}");
    assert!(text.contains("clock skew"), "{text:?}");
    assert!(!text.contains("published 0s ago"), "never that: {text:?}");
}

/// A status document older than the protocol's staleness window renders as offline.
#[test]
fn a_status_past_the_staleness_window_renders_offline() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let later = goalcup_protocol::control::format_z(
        goalcup_protocol::control::parse_z(support::NOW).unwrap() + STALE_AFTER_SECONDS + 1,
    );
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    let mut out = Vec::new();
    let _ = status_with(&m, &cfg, Some(&info.entry_id), false, &later, &mut out);
    assert!(String::from_utf8(out).unwrap().contains("offline"));
}

#[test]
fn entries_lists_one_line_per_entry_json_is_an_array_and_a_sealed_entry_leaves_the_list() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    let mut out = Vec::new();
    entries_with(&m, tmp.path(), false, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(
        text.lines().filter(|l| l.contains("entry-")).count(),
        m.entry_count()
    );
    // The way to a run comes first, and the states are in words, not wire names.
    assert!(
        text.lines()
            .next()
            .unwrap()
            .contains("goalcup status <part-id>"),
        "{text}"
    );
    assert!(!text.contains("online"), "{text}");
    // With a run record for the part, its line names the run `goalcup continue` takes.
    let entry_id = m.info().entry_id;
    goalcup_cli::run::write(
        tmp.path(),
        &goalcup_cli::run::RunRecord {
            run_id: "run-with-a-part".to_string(),
            server_url: Some("https://server.example".to_string()),
            practice: false,
            test: false,
            connector_id: "codex-cli".to_string(),
            workspace: std::path::PathBuf::from("/w"),
            started_at_unix: 1,
            deadline_at: None,
            entries: vec![goalcup_cli::run::EntryRef {
                entry_id: entry_id.clone(),
                trace_dir: std::path::PathBuf::from("/t"),
            }],
        },
    )
    .unwrap();
    let mut out = Vec::new();
    entries_with(&m, tmp.path(), false, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("goalcup continue <run-id>"), "{text}");
    let line = text.lines().find(|l| l.contains(&entry_id)).unwrap();
    assert!(line.contains("run-with-a-part"), "{text}");
    let mut out = Vec::new();
    entries_with(&m, tmp.path(), true, &mut out).unwrap();
    let v: Vec<goalcup_protocol::control::EntrySummary> = serde_json::from_slice(&out).unwrap();
    assert_eq!(v.len(), m.entry_count());

    // However the list treats a sealed entry, the command names the way to reach one.
    let before = m.entry_count();
    m.seal(&m.info().entry_id).unwrap();
    let mut out = Vec::new();
    entries_with(&m, tmp.path(), false, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        m.entry_count() < before,
        "a sealed entry is evicted from the live map"
    );
    assert!(
        text.to_lowercase().contains("goalcup status"),
        "name the way to reach a run that has finished"
    );
}

/// The daemon's own strings reach a terminal through one function, and a control character
/// in one of them is not obeyed.
///
/// `render_status` is the one place in this crate where bytes it did
/// not author reach a screen, and a terminal reads control characters as instructions: an
/// escape sequence inside `detail` can move the cursor over the freshness line above it and
/// repaint it. The document is signed against the pinned run key by the time this renders,
/// so this is the last inch of a chain that is otherwise sound — which is why it is one
/// function and not a judgement call at nine call sites.
#[test]
fn a_control_character_in_a_daemon_string_is_not_handed_to_the_terminal() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    // An escape that clears the line, a bare carriage return that puts the cursor back at
    // its start, and a newline that would draw a line this command did not write.
    m.set_capture_status(
        local_capture(),
        "\u{1b}[2K\rthe tracker says\nRecording. Leave this running.",
        1,
        0,
    );

    let mut out = Vec::new();
    status_with(
        &m,
        &cfg,
        Some(&info.entry_id),
        false,
        support::NOW,
        &mut out,
    )
    .unwrap();
    let said = String::from_utf8(out).unwrap();
    assert!(
        !said.contains('\u{1b}') && !said.contains('\r'),
        "no escape and no carriage return reaches the screen: {said:?}"
    );
    assert!(
        said.contains("the tracker says") && said.contains("Recording"),
        "the words themselves are still shown — this replaces, it does not censor: {said}"
    );
    assert!(
        said.matches('\n').count() == said.lines().count(),
        "and the daemon's newline did not become a line of its own: {said:?}"
    );
    // The one document `--json` prints is the daemon's bytes unchanged: it is a signed
    // document a participant hands to an organiser, and a rendering that edited it would
    // break the signature it is being handed for.
    let mut json = Vec::new();
    status_with(
        &m,
        &cfg,
        Some(&info.entry_id),
        true,
        support::NOW,
        &mut json,
    )
    .unwrap();
    let doc: goalcup_protocol::control::SignedStatus =
        serde_json::from_slice(&json).expect("--json is one signed document");
    assert!(
        doc.payload.detail.contains('\u{1b}'),
        "unchanged on the wire"
    );
    assert!(doc.verify_with(&info.run_public_key).is_ok());
}

/// There is no offline copy: a daemon that does not answer is the answer, even with a
/// status-shaped file where the old cache lived. `goalcup status` starts the daemon that holds
/// the registry instead (`main`), so a file on disk is never a second source of truth.
#[test]
fn an_unreachable_daemon_is_reported_and_no_file_on_disk_stands_in_for_it() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    let document = m.status(&info.entry_id).expect("the mock answers once");
    let cache = cfg.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(
        cache.join(format!("status-{}.json", info.entry_id)),
        serde_json::to_string(&document).unwrap(),
    )
    .unwrap();

    m.fail_next(
        "status",
        control::ControlError::Unreachable("connection refused".to_string()),
    );
    let mut out = Vec::new();
    let error = status_with(
        &m,
        &cfg,
        Some(&info.entry_id),
        false,
        support::NOW,
        &mut out,
    )
    .expect_err("an unreachable daemon is not answered from disk");
    assert!(format!("{error}").contains("connection refused"), "{error}");
    assert!(out.is_empty(), "nothing is rendered: {out:?}");
}

/// A v5 local capture's status as the daemon composes it: recording, nothing uploaded, no
/// signed start or deadline.
fn local_capture() -> goalcup_protocol::capture::M2Status {
    use goalcup_protocol::capture::*;
    M2Status {
        run: RunState::Running,
        capture: CaptureState::Recording,
        delivery: DeliveryState::LocalOnly,
        analysis: AnalysisState::NotRun,
        anchor: AnchorState::LocalOnly,
        storage: CaptureStorage {
            quota_bytes: 4_194_304,
            reserved_bytes: 4_194_304,
            used_bytes: 2_048,
            durable: Some(DurableFrontier {
                seq: 4,
                hash: "ab".repeat(32),
                chain_bytes: 1_536,
            }),
            accounting_available: true,
        },
        gap_count: 2,
        assistance_observed: false,
    }
}

#[test]
fn a_capture_status_shows_its_capture_block_and_no_legacy_run_lines() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    m.set_capture_status(
        local_capture(),
        "Local capture is recording to this machine only.",
        1,
        0,
    );

    let mut out = Vec::new();
    status_with(
        &m,
        &cfg,
        Some(&info.entry_id),
        false,
        support::NOW,
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    for said in [
        "capture: recording",
        "run running",
        "analysis not_run",
        "Local capture is recording to this machine only.",
        "2048 of 4194304 bytes used, 4194304 reserved, durable through seq 4 (1536 bytes)",
        "gaps: 2; assistance recorded: no",
        "1 producer(s) consumed, 0 refused admission",
    ] {
        assert!(text.contains(said), "missing {said:?} in:\n{text}");
    }
    for legacy in [
        "arming:",
        "health:",
        "events:",
        "time remaining",
        "round dates",
        // The recorder never knows the upload: its constant local_only words are not shown
        // beside the Upload line that knows (they stay in --json, as signed).
        "delivery local_only",
        "anchor local_only",
    ] {
        assert!(
            !text.contains(legacy),
            "a local capture has no {legacy:?} line:\n{text}"
        );
    }
}

#[test]
fn a_capture_status_in_json_is_the_signed_document_with_its_capture_block() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    m.set_capture_status(local_capture(), "recording", 1, 0);

    let mut out = Vec::new();
    status_with(&m, &cfg, Some(&info.entry_id), true, support::NOW, &mut out).unwrap();
    let doc: control::SignedStatus = serde_json::from_slice(&out).unwrap();
    doc.verify_with(&info.run_public_key).unwrap();
    assert_eq!(doc.payload.capture, Some(local_capture()));
    assert_eq!(
        (
            doc.payload.producers_consumed,
            doc.payload.admissions_refused
        ),
        (Some(1), Some(0))
    );
    let raw: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(raw["payload"]["capture"]["delivery"], "local_only");
    assert_eq!(raw["payload"]["producers_consumed"], 1);
}

/// A real daemon over `root` with no server anywhere, on a runtime of its own.
#[cfg(target_os = "macos")]
fn local_daemon(
    root: &std::path::Path,
) -> (
    tokio::runtime::Runtime,
    goalcup_daemon::runtime::Daemon,
    goalcup_daemon::config::DaemonConfig,
) {
    let config = goalcup_daemon::config::DaemonConfig {
        artifact_root: root.join("artifacts"),
        connectors_dir: support::connectors_dir(),
        config_dir: root.join("daemon-config"),
        workspace_parent: root.join("workspaces"),
        control_addr: "127.0.0.1:0".parse().unwrap(),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let daemon = runtime
        .block_on(goalcup_daemon::runtime::Daemon::start(config.clone()))
        .unwrap();
    (runtime, daemon, config)
}

/// A v5 capture in `root/workspace`, its goal accepted and its launch recorded.
#[cfg(target_os = "macos")]
fn launched_capture(
    client: &goalcup_cli::daemon::HttpDaemonClient,
    root: &std::path::Path,
) -> control::CaptureWorkspaceInfo {
    let info = client
        .create_capture_workspace(&control::CaptureWorkspaceRequest {
            workspace: root.join("runs/workspace").display().to_string(),
            profile_id: "codex-cli".to_string(),
            connector_dir_sha256: goalcup_protocol::pins::pinned_connector_dir_sha256("codex-cli")
                .unwrap()
                .to_string(),
            reserve_bytes: 4 * 1024 * 1024,
            run_home: root.join("run-home").display().to_string(),
            config_home: root.join("config-home").display().to_string(),
            entry_id: None,
            native_since: None,
            run_secrets: Vec::new(),
            existing_clone: false,
        })
        .unwrap();
    let goal = client
        .record_capture_goal(
            &info.entry_id,
            &control::CaptureGoalRequest {
                bytes: b"write hello.txt".to_vec(),
            },
        )
        .unwrap();
    client
        .record_capture_launch(
            &info.entry_id,
            &control::CaptureLaunchRequest {
                goal_stream: goal,
                spawned_at: goalcup_cli::proxy::metadata::capture_time(),
                harness_version: "0.153.0".to_string(),
                continuation: "exec: no input after the goal".to_string(),
            },
        )
        .unwrap();
    info
}

/// The v5 reader's report on `entry_id`'s trace.
#[cfg(target_os = "macos")]
fn read_capture(
    config: &goalcup_daemon::config::DaemonConfig,
    entry_id: &str,
) -> goalcup_verifier::capture_analysis::reader::CaptureIntegrityReport {
    let trace = config
        .artifact_root
        .join("artifacts")
        .join(entry_id)
        .join("trace");
    goalcup_verifier::capture_analysis::reader::scan_capture_trace(
        &trace,
        &goalcup_verifier::VerifyOptions::default(),
        &mut |_| Ok(()),
    )
    .unwrap()
}

/// A v5 capture of a shipped profile through the real daemon and this CLI's own client: the
/// status view verifies against the key the create call returned, and the v5 reader finds the
/// goal, launch and baseline bound. The shipped Codex profile runs in chat mode (D57): it
/// names no session at launch and reads every rollout since the run started, so no
/// required-source gap is chained and the local finish seals a trace that reads valid.
#[cfg(target_os = "macos")]
#[test]
fn a_shipped_chat_profile_capture_records_and_seals_valid() {
    use goalcup_cli::daemon::HttpDaemonClient;

    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let (runtime, daemon, config) = local_daemon(&root);
    let client = HttpDaemonClient::from_config_dir(&config.config_dir).unwrap();
    let info = launched_capture(&client, &root);

    let cfg = root.join("cli-config");
    let record = goalcup_cli::entries::EntryRecord {
        entry_id: info.entry_id.clone(),
        process_generation: info.process_generation,
        connector_id: "codex-cli".to_string(),
        run_public_key: info.run_public_key.clone(),
        workspace: root.join("runs/workspace").display().to_string(),
        artifact_root: config.artifact_root.display().to_string(),
        harness_home: root.join("config-home").display().to_string(),
        session_id: String::new(),
        last_pid: None,
        created_at: support::NOW.to_string(),
        dev: false,
    };
    goalcup_cli::entries::write_entry(&cfg, &record).unwrap();
    let mut out = Vec::new();
    status_with(
        &client,
        &cfg,
        Some(&info.entry_id),
        false,
        support::NOW,
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("capture: recording — run running"), "{text}");
    assert!(
        text.contains("0 producer(s) consumed, 0 refused admission"),
        "{text}"
    );

    let report = read_capture(&config, &info.entry_id);
    assert!(!report.torn_tail);
    assert!(!report.coverage_incomplete, "{:?}", report.findings);
    assert!(!report.sealed);
    assert_eq!(report.anchor, AnchorState::LocalOnly);
    assert_eq!(report.gap_count, 0);
    assert_eq!(
        report.findings,
        vec!["chain_unsealed".to_string()],
        "every producer closed, the launch bound, the baseline bound: only the seal is missing"
    );

    // The local finish, with no server anywhere: sealed and valid.
    let finished = client
        .seal_capture(
            &info.entry_id,
            &control::CaptureSealRequest {
                proxy: Some(control::ProxyCounters::default()),
                remaining_finish_ms: Some(10_000),
            },
            std::time::Duration::from_secs(30),
        )
        .unwrap();
    assert_eq!(finished.seal, control::LocalSealOutcome::SealWritten);
    let report = read_capture(&config, &info.entry_id);
    runtime.block_on(daemon.stop());
    assert!(report.sealed, "{:?}", report.findings);
    assert_eq!(report.findings, Vec::<String>::new());
    assert_eq!(report.exit_code(), 0);
}

/// The daemon goes away under a launched run with no producer attached, which is what a
/// crash leaves behind: nothing is chained as it goes. The reopened entry admits nothing, so
/// the chain records that loss, and the sealed trace never reads as complete.
#[cfg(target_os = "macos")]
#[test]
fn a_capture_reopened_under_its_live_run_seals_with_the_loss_chained() {
    use goalcup_cli::daemon::HttpDaemonClient;

    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let (runtime, daemon, config) = local_daemon(&root);
    let client = HttpDaemonClient::from_config_dir(&config.config_dir).unwrap();
    let info = launched_capture(&client, &root);
    runtime.block_on(daemon.stop());

    let daemon = runtime
        .block_on(goalcup_daemon::runtime::Daemon::start(config.clone()))
        .unwrap();
    let client = HttpDaemonClient::from_config_dir(&config.config_dir).unwrap();
    let finished = client
        .seal_capture(
            &info.entry_id,
            &control::CaptureSealRequest {
                proxy: Some(control::ProxyCounters::default()),
                remaining_finish_ms: Some(10_000),
            },
            std::time::Duration::from_secs(30),
        )
        .unwrap();
    assert_eq!(finished.seal, control::LocalSealOutcome::SealWritten);
    let report = read_capture(&config, &info.entry_id);
    runtime.block_on(daemon.stop());
    assert!(report.sealed, "{:?}", report.findings);
    assert_eq!(report.gap_count, 1, "{:?}", report.findings);
    assert!(report.coverage_incomplete);
    assert_eq!(report.findings, vec!["gap_recorded".to_string()]);
    assert_eq!(report.exit_code(), 1);
}

// ------------------------------------------------------------------ the run's own problems

/// The run record and state file `goalcup start` leaves under `cfg` for `entry_id`: a run with
/// no server.
fn run_with_state(cfg: &std::path::Path, entry_id: &str, state: goalcup_cli::run::RunState) {
    run_with_state_on(cfg, entry_id, state, None, None);
}

/// [`run_with_state`] for a run that may be on a server (`server`), with a deadline
/// (`deadline_at`); a run on a server has the credential `goalcup continue` needs.
fn run_with_state_on(
    cfg: &std::path::Path,
    entry_id: &str,
    state: goalcup_cli::run::RunState,
    server: Option<&str>,
    deadline_at: Option<&str>,
) {
    use goalcup_cli::run::{EntryRef, RunRecord, StateKeeper, run_dir, write};
    // The entry's trace holds a valid seal.json: a run whose host finished is sealed.
    let trace_dir = trace_of(cfg, entry_id);
    std::fs::create_dir_all(&trace_dir).unwrap();
    let seal = serde_json::json!({
        "entry_id": entry_id, "sealed_at": "2026-10-21T00:00:00Z", "workspace": "/w",
        "files": [], "excluded": [],
    });
    std::fs::write(trace_dir.join("seal.json"), seal.to_string()).unwrap();
    write(
        cfg,
        &RunRecord {
            run_id: entry_id.to_string(),
            server_url: server.map(str::to_string),
            practice: false,
            test: false,
            connector_id: "claude-code-cli".to_string(),
            workspace: PathBuf::from("/w"),
            started_at_unix: 1,
            deadline_at: deadline_at.map(str::to_string),
            entries: vec![EntryRef {
                entry_id: entry_id.to_string(),
                trace_dir,
            }],
        },
    )
    .unwrap();
    if server.is_some() {
        let credential = goalcup_cli::account::credential::RunCredential::new(
            format!("gcr_{}", "ab".repeat(32)),
            "2099-01-01T00:00:00Z".to_string(),
        )
        .unwrap();
        goalcup_cli::account::credential::store(&run_dir(cfg, entry_id), &credential).unwrap();
    }
    StateKeeper::begin(&run_dir(cfg, entry_id), state);
}

/// The trace directory [`run_with_state_on`] gives the entry.
fn trace_of(cfg: &std::path::Path, entry_id: &str) -> PathBuf {
    cfg.join("traces").join(entry_id)
}

/// Write the run's state as a goalcup process that stopped at `updated_at` left it: the keeper
/// stamps the time of every write, so the file is edited after.
fn leave_state_at(cfg: &std::path::Path, entry_id: &str, updated_at: u64) {
    let file = goalcup_cli::run::run_dir(cfg, entry_id).join(goalcup_cli::run::STATE_FILE_NAME);
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    json["updated_at"] = updated_at.into();
    std::fs::write(&file, serde_json::to_vec(&json).unwrap()).unwrap();
}

fn status_text(
    m: &goalcup_cli::daemon::MockDaemonClient,
    cfg: &std::path::Path,
    entry: &str,
    json: bool,
) -> String {
    let mut out = Vec::new();
    status_with(m, cfg, Some(entry), json, support::NOW, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

/// The `PROBLEM:` lines of a status text, in order.
fn problem_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|l| l.starts_with(participant::PROBLEM))
        .collect()
}

/// The participant block of a status text: everything before the technical block.
fn participant_block(text: &str) -> &str {
    text.split(&format!("\n{}\n", participant::STATUS_DETAILS))
        .next()
        .unwrap()
}

/// `goalcup status` shows the free disk, what goalcup's own processes use and a pause for the
/// disk on one line, and under `--json` beside the signed document, which still verifies.
#[test]
fn status_shows_the_disk_and_what_goalcup_uses() {
    use goalcup_cli::run::{DiskState, RunState, run_dir};
    use goalcup_cli::upload::api::HeartbeatVitals;
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    let mut state = RunState::launched(&info.entry_id, Ok(()));
    state.disk = DiskState {
        free_bytes: 20 << 30,
        reserve_bytes: 3 << 30,
        low: false,
        disk_paused: false,
    };
    run_with_state(&cfg, &info.entry_id, state);
    let dir = run_dir(&cfg, &info.entry_id);
    // What the last heartbeat read of goalcup's processes.
    let heartbeat = HeartbeatVitals {
        rss_bytes: Some(80 << 20),
        cpu_ms: Some(125_000),
        open_files: Some(40),
        ..HeartbeatVitals::default()
    };
    std::fs::write(
        dir.join(goalcup_cli::vitals::VITALS_FILE_NAME),
        serde_json::to_vec(&heartbeat).unwrap(),
    )
    .unwrap();
    // The disk pause says so in the state (D65).
    let file = dir.join(goalcup_cli::run::STATE_FILE_NAME);
    let mut raw: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    raw["disk"]["disk_paused"] = true.into();
    std::fs::write(&file, serde_json::to_vec(&raw).unwrap()).unwrap();

    let text = status_text(&m, &cfg, &info.entry_id, false);
    let line = participant_block(&text)
        .lines()
        .find(|line| line.contains("This computer"))
        .unwrap_or_else(|| panic!("{text}"));
    for said in ["20 GiB", "80 MiB", "2 min", "paused for disk"] {
        assert!(line.contains(said), "{said}: {line}");
    }

    let json = status_text(&m, &cfg, &info.entry_id, true);
    let doc: control::SignedStatus = serde_json::from_str(&json).unwrap();
    doc.verify_with(&info.run_public_key).unwrap();
    let raw: serde_json::Value = serde_json::from_str(&json).unwrap();
    let local = &raw[goalcup_cli::commands::status::LOCAL_KEY];
    assert_eq!(local["disk_free_bytes"], 20u64 << 30, "{json}");
    assert_eq!(local["agent_paused"], "disk", "{json}");
    assert_eq!(local["rss_bytes"], 80u64 << 20, "{json}");
    assert_eq!(local["cpu_ms"], 125_000, "{json}");
}

#[test]
fn status_puts_every_problem_first() {
    use goalcup_cli::run::{DiskState, HarnessState, Recording, RunState, UploadState, now_unix};
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    let now = control::parse_z(support::NOW).unwrap();
    let mut state = RunState::launched(
        &info.entry_id,
        Err("no sleep inhibitor on this machine".into()),
    );
    state.updated_at = now_unix();
    state.recording = Recording::Gaps;
    state.upload = UploadState {
        last_ok_at: Some(now - 40 * 60),
        problem: Some("the server answered 503".to_string()),
        stopped: false,
        refused: None,
    };
    state.disk = DiskState {
        free_bytes: 3 << 30,
        reserve_bytes: 1 << 30,
        low: true,
        disk_paused: false,
    };
    state.harness = HarnessState::Exited {
        status: "exit code 1".to_string(),
    };
    // A run on a server with a chat-mode connector, its deadline ahead: `continue` can restart it.
    state.continuable = true;
    let run_with_state = |cfg: &std::path::Path, entry: &str, state: RunState| {
        run_with_state_on(
            cfg,
            entry,
            state,
            Some("https://server.example"),
            Some("2099-01-01T00:00:00Z"),
        );
    };
    run_with_state(&cfg, &info.entry_id, state.clone());

    let text = status_text(&m, &cfg, &info.entry_id, false);
    // The participant block comes first: the run, then the problems, each with its fix, then the
    // one thing to do next, then the technical block under its heading.
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("Your "), "{text}");
    assert!(lines[0].contains("claude-code-cli"), "{text}");
    assert!(
        text.contains("Problems: 5") || text.contains(" 5, "),
        "{text}"
    );
    let problems = problem_lines(&text);
    assert_eq!(problems.len(), 5, "{text}");
    for (line, words) in problems.iter().zip([
        &["gaps"][..],
        &["40 min", "the server answered 503", "Check the network"],
        &["3 GiB", "Free some space"],
        &["sleep", "no sleep inhibitor on this machine"],
        &["has exited (exit code 1)", "goalcup continue"],
    ]) {
        for word in words {
            assert!(line.contains(word), "{word}: {line}");
        }
    }
    let first_problem = text.find("PROBLEM:").unwrap();
    let next = text.find("Next: ").unwrap();
    let details = text
        .find(&format!("\n{}\n", participant::STATUS_DETAILS))
        .unwrap();
    assert!(text.find("Problems:").unwrap() < first_problem, "{text}");
    assert!(first_problem < next && next < details, "{text}");
    assert!(
        text.contains(&m.status_detail()),
        "the status itself follows the problems, under Details: {text}"
    );
    assert!(text[details..].contains(&m.status_detail()), "{text}");

    // A failed recording; and an upload that has not lasted half an hour is not yet a problem,
    // unless it gave up.
    state.recording = Recording::Failed;
    state.upload.last_ok_at = Some(now - 60);
    state.sleep_lock = Ok(());
    state.disk.low = false;
    state.harness = HarnessState::Running;
    run_with_state(&cfg, &info.entry_id, state.clone());
    let text = status_text(&m, &cfg, &info.entry_id, false);
    let problems = problem_lines(&text);
    assert_eq!(problems.len(), 1, "{text}");
    assert!(problems[0].contains("recording failed"), "{text}");
    assert!(
        problems[0].contains(&info.entry_id),
        "the run it is about: {text}"
    );
    state.upload.stopped = true;
    run_with_state(&cfg, &info.entry_id, state);
    let text = status_text(&m, &cfg, &info.entry_id, false);
    let upload = problem_lines(&text)
        .into_iter()
        .find(|l| l.contains("not uploaded"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(upload.contains("1 min"), "{upload}");
    assert!(upload.contains("the server answered 503"), "{upload}");
    assert!(upload.contains("goalcup upload"), "{upload}");
    assert!(text.contains("Upload: stopped"), "{text}");

    // `--json` stays one document.
    let json = status_text(&m, &cfg, &info.entry_id, true);
    serde_json::from_str::<control::SignedStatus>(&json).expect("one document and nothing else");
}

/// `goalcup continue` is advised only for a run it can restart: otherwise status says what is
/// true.
#[test]
fn status_advises_continue_only_when_continue_can_restart_the_run() {
    use goalcup_cli::run::{HarnessState, RunState};
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    let mut state = RunState::launched(&info.entry_id, Ok(()));
    state.harness = HarnessState::Exited {
        status: "exit code 1".to_string(),
    };
    let server = Some("https://server.example");
    let ahead = Some("2099-01-01T00:00:00Z");
    let said = |cfg: &std::path::Path| status_text(&m, cfg, &info.entry_id, false);
    // The one problem line of a status text (the agent that left), with the participant block.
    let left = |text: &str| {
        let problems = problem_lines(text);
        assert_eq!(problems.len(), 1, "{text}");
        assert!(problems[0].contains("has exited (exit code 1)"), "{text}");
        problems[0].to_string()
    };

    // A headless connector whose agent finished normally (status 0): a plain line, no PROBLEM,
    // and nothing to do.
    let mut finished = state.clone();
    finished.harness = HarnessState::Exited {
        status: "exit code 0".to_string(),
    };
    run_with_state_on(&cfg, &info.entry_id, finished.clone(), server, ahead);
    let text = said(&cfg);
    assert!(
        text.contains("finished, and this run cannot be continued"),
        "{text}"
    );
    assert!(!text.contains("PROBLEM"), "{text}");
    assert!(
        participant_block(&text).contains("Next: nothing to do."),
        "{text}"
    );
    assert!(!text.contains("goalcup continue"), "{text}");
    // The same normal end of a local run.
    run_with_state_on(&cfg, &info.entry_id, finished.clone(), None, None);
    let text = said(&cfg);
    assert!(
        text.contains("finished, and this run cannot be continued"),
        "{text}"
    );
    assert!(!text.contains("PROBLEM"), "{text}");
    // A chat-mode run `continue` can restart is still told so, as a problem, and the next action
    // is the command with the run's id.
    finished.continuable = true;
    run_with_state_on(&cfg, &info.entry_id, finished, server, ahead);
    let text = said(&cfg);
    let problem = problem_lines(&text);
    assert_eq!(problem.len(), 1, "{text}");
    assert!(problem[0].contains("exit code 0"), "{text}");
    assert!(
        participant_block(&text).contains(&format!("Next: run goalcup continue {}", info.entry_id)),
        "{text}"
    );

    // A headless connector, run open on the server: not continuable.
    state.continuable = false;
    run_with_state_on(&cfg, &info.entry_id, state.clone(), server, ahead);
    let text = said(&cfg);
    assert!(left(&text).contains("cannot be continued"), "{text}");
    assert!(!text.contains("goalcup continue"), "{text}");

    // A chat-mode run on the server whose deadline has passed: the run is over.
    state.continuable = true;
    run_with_state_on(
        &cfg,
        &info.entry_id,
        state.clone(),
        server,
        Some("2020-01-01T00:00:00Z"),
    );
    let text = said(&cfg);
    assert!(left(&text).contains("the run is over"), "{text}");
    assert!(!text.contains("goalcup continue"), "{text}");
    assert!(text.contains("Ended: "), "{text}");

    // A local run (no server) is not continuable either.
    run_with_state_on(&cfg, &info.entry_id, state.clone(), None, None);
    let text = said(&cfg);
    assert!(left(&text).contains("cannot be continued"), "{text}");
    assert!(!text.contains("goalcup continue"), "{text}");

    // And one `continue` can restart still says to run it.
    run_with_state_on(&cfg, &info.entry_id, state, server, ahead);
    let text = said(&cfg);
    assert!(
        left(&text).contains(&format!("goalcup continue {}", info.entry_id)),
        "{text}"
    );
    assert!(text.contains("Ends: "), "{text}");
}

#[test]
fn status_names_heartbeats_that_keep_failing() {
    use goalcup_cli::run::{HEARTBEATS_FAILING_AFTER, HarnessState, HeartbeatState, RunState};
    use goalcup_cli::run::{Recording, now_unix};
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    let mut state = RunState::launched(&info.entry_id, Ok(()));
    state.updated_at = now_unix();
    state.recording = Recording::Ok;
    state.harness = HarnessState::Running;

    // One short of the threshold is a blip, not a problem.
    state.heartbeat = HeartbeatState {
        failures: HEARTBEATS_FAILING_AFTER - 1,
        last_failure: Some("the server answered HTTP 503".to_string()),
        ..HeartbeatState::default()
    };
    run_with_state(&cfg, &info.entry_id, state.clone());
    let text = status_text(&m, &cfg, &info.entry_id, false);
    assert!(!text.contains("PROBLEM"), "{text}");

    state.heartbeat.failures = HEARTBEATS_FAILING_AFTER;
    run_with_state(&cfg, &info.entry_id, state.clone());
    let text = status_text(&m, &cfg, &info.entry_id, false);
    let problems = problem_lines(&text);
    assert_eq!(problems.len(), 1, "{text}");
    assert!(problems[0].contains("15 min"), "{text}");
    assert!(
        problems[0].contains("the server answered HTTP 503"),
        "{text}"
    );
    assert!(problems[0].contains("Check the network"), "{text}");

    // A run that is over has no heartbeats to worry about.
    state.ended = true;
    run_with_state(&cfg, &info.entry_id, state);
    let text = status_text(&m, &cfg, &info.entry_id, false);
    assert!(problem_lines(&text).is_empty(), "{text}");
}

/// D64: a server that keeps refusing the run's calls is one problem line, naming the status,
/// while the run records; none once the server answers again or the run has ended.
#[test]
fn status_names_a_server_that_keeps_refusing_the_run() {
    use goalcup_cli::run::{HarnessState, Recording, RunState, now_unix};
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    let mut state = RunState::launched(&info.entry_id, Ok(()));
    state.updated_at = now_unix();
    state.recording = Recording::Ok;
    state.harness = HarnessState::Running;
    state.upload.refused = Some(401);
    run_with_state(&cfg, &info.entry_id, state.clone());
    let text = status_text(&m, &cfg, &info.entry_id, false);
    let problems = problem_lines(&text);
    assert_eq!(problems.len(), 1, "{text}");
    assert!(problems[0].contains("401"), "{text}");

    for cleared in [false, true] {
        let mut state = state.clone();
        if cleared {
            state.upload.refused = None;
        } else {
            state.ended = true;
        }
        run_with_state(&cfg, &info.entry_id, state);
        let text = status_text(&m, &cfg, &info.entry_id, false);
        assert!(problem_lines(&text).is_empty(), "{text}");
    }
}

/// Start a practice run P, then a competition run C, then continue P: P's new segment is the
/// newest entry while C is still the newest run. `goalcup status` with no id shows the pinned
/// entry, and the run section must be that entry's run (P), never the newest run's (C).
#[test]
fn status_with_no_id_never_mixes_the_newest_entry_with_the_newest_run() {
    use goalcup_cli::entries::{EntryRecord, write_entry};
    use goalcup_cli::run::{
        EntryRef, HarnessState, Recording, RunRecord, RunState, StateKeeper, now_unix, run_dir,
        write,
    };
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = tmp.path().to_path_buf();
    let entry = |entry_id: &str, created_at: &str| EntryRecord {
        entry_id: entry_id.to_string(),
        process_generation: info.process_generation,
        connector_id: "claude-code-cli".to_string(),
        run_public_key: info.run_public_key.clone(),
        workspace: info.workspace.clone(),
        artifact_root: info.artifact_root.clone(),
        harness_home: info.harness_home.clone(),
        session_id: info.session_id.clone(),
        last_pid: None,
        created_at: created_at.to_string(),
        dev: false,
    };
    let run = |run_id: &str, practice: bool, started: u64, entries: &[&str]| RunRecord {
        run_id: run_id.to_string(),
        server_url: None,
        practice,
        test: false,
        connector_id: "claude-code-cli".to_string(),
        workspace: PathBuf::from("/w"),
        started_at_unix: started,
        deadline_at: None,
        entries: entries
            .iter()
            .map(|id| EntryRef {
                entry_id: (*id).to_string(),
                trace_dir: PathBuf::from("/t"),
            })
            .collect(),
    };
    let state = |recording: Recording| {
        let mut state = RunState::launched("x", Ok(()));
        state.updated_at = now_unix();
        state.recording = recording;
        state.harness = HarnessState::Running;
        state
    };
    // The practice run P: first segment, then (after C) a continued segment, the mock's entry.
    write_entry(&cfg, &entry("p-1", "2026-09-01T00:00:00Z")).unwrap();
    write_entry(&cfg, &entry("c-1", "2026-09-02T00:00:00Z")).unwrap();
    write_entry(&cfg, &entry(&info.entry_id, "2026-09-03T00:00:00Z")).unwrap();
    write(&cfg, &run("run-p", true, 100, &["p-1", &info.entry_id])).unwrap();
    write(&cfg, &run("run-c", false, 200, &["c-1"])).unwrap();

    let text_of = |cfg: &std::path::Path| {
        let mut out = Vec::new();
        status_with(&m, cfg, None, false, support::NOW, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    };

    // C is failing, P is healthy: the status is P's, so no problem of C's may be printed.
    StateKeeper::begin(&run_dir(&cfg, "run-p"), state(Recording::Ok));
    StateKeeper::begin(&run_dir(&cfg, "run-c"), state(Recording::Failed));
    let text = text_of(&cfg);
    assert!(text.contains(&m.status_detail()), "P's own status: {text}");
    assert!(
        !text.contains("PROBLEM"),
        "C's problem leaked into P: {text}"
    );

    // P is failing, C is healthy: P's problem is shown.
    StateKeeper::begin(&run_dir(&cfg, "run-p"), state(Recording::Failed));
    StateKeeper::begin(&run_dir(&cfg, "run-c"), state(Recording::Ok));
    let text = text_of(&cfg);
    let problems = problem_lines(&text);
    assert_eq!(problems.len(), 1, "{text}");
    assert!(problems[0].contains("recording failed"), "{text}");
}

#[test]
fn status_with_no_problem_prints_no_problem_line() {
    use goalcup_cli::run::{HarnessState, Recording, RunState};
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);

    // A run that has no state file yet says so in plain words, and that is not a problem.
    {
        use goalcup_cli::run::{EntryRef, RunRecord, write};
        write(
            &cfg,
            &RunRecord {
                run_id: info.entry_id.clone(),
                server_url: None,
                practice: false,
                test: false,
                connector_id: "claude-code-cli".to_string(),
                workspace: PathBuf::from("/w"),
                started_at_unix: 1,
                deadline_at: None,
                entries: vec![EntryRef {
                    entry_id: info.entry_id.clone(),
                    trace_dir: PathBuf::from("/t"),
                }],
            },
        )
        .unwrap();
    }
    let text = status_text(&m, &cfg, &info.entry_id, false);
    assert!(!text.contains("PROBLEM"), "{text}");
    // The record says it started long ago and has no state: it did not start, so waiting is
    // no help and the next step is a new start.
    assert!(text.contains("did not start"), "{text}");
    assert!(
        participant_block(&text).contains("goalcup start --harness claude-code-cli"),
        "{text}"
    );
    // A run a moment old is still starting: wait.
    let mut record = goalcup_cli::run::read(&cfg, &info.entry_id).unwrap();
    record.started_at_unix = control::parse_z(support::NOW).unwrap() - 30;
    goalcup_cli::run::write(&cfg, &record).unwrap();
    let text = status_text(&m, &cfg, &info.entry_id, false);
    assert!(text.contains("no run state yet"), "{text}");
    assert!(text.contains("Next: wait a minute"), "{text}");
    assert!(!text.contains("did not start"), "{text}");

    // A healthy run, an unknown recording (the daemon was slow to answer) and an upload that
    // is failing for less than half an hour, an agent the host ended on purpose: no problem.
    let mut state = RunState::launched(&info.entry_id, Ok(()));
    state.recording = Recording::Unknown;
    state.upload.last_ok_at = Some(control::parse_z(support::NOW).unwrap() - 120);
    state.upload.problem = Some("the server answered 503".to_string());
    state.harness = HarnessState::Exited {
        status: "signal 15".to_string(),
    };
    state.stopped = Some("the server says the run is over".to_string());
    run_with_state(&cfg, &info.entry_id, state);
    let text = status_text(&m, &cfg, &info.entry_id, false);
    assert!(!text.contains("PROBLEM"), "{text}");
    assert!(!text.contains("no run state yet"), "{text}");
    assert!(text.contains(&m.status_detail()), "{text}");
}

/// The header names the harness by its manifest's display name, the run's mode, and when it
/// ends; a connector that cannot be loaded falls back to its id.
#[test]
fn status_header_names_the_harness_the_mode_and_the_end() {
    use goalcup_cli::run::{HarnessState, RunState};
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    let mut state = RunState::launched(&info.entry_id, Ok(()));
    state.harness = HarnessState::Running;
    state.updated_at = goalcup_cli::run::now_unix();
    run_with_state_on(
        &cfg,
        &info.entry_id,
        state,
        Some("https://server.example"),
        Some("2099-01-01T00:00:00Z"),
    );
    let mut record = goalcup_cli::run::read(&cfg, &info.entry_id).unwrap();
    record.practice = true;
    goalcup_cli::run::write(&cfg, &record).unwrap();

    let mut out = Vec::new();
    status_in(
        &m,
        &cfg,
        Some(&support::connectors_dir()),
        Some(&info.entry_id),
        false,
        support::NOW,
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    let block = participant_block(&text);
    assert!(
        block.starts_with("Your practice run with Claude Code: recording."),
        "{text}"
    );
    assert!(block.contains("Ends: "), "{text}");
    assert!(block.contains("UTC"), "{text}");
    assert!(block.contains("left"), "{text}");
    assert!(block.contains("Claude Code: running."), "{text}");
    assert!(block.contains("Problems: none."), "{text}");
    assert!(block.contains("Next: nothing to do."), "{text}");
    // The ids are in the details, not in the participant block.
    assert!(!block.contains(&info.entry_id), "{text}");
    assert!(
        text.contains(&format!("\nrun {}\n", info.entry_id)),
        "{text}"
    );
    // The contents of the connector directory decide the name; no directory, the id.
    let text = status_text(&m, &cfg, &info.entry_id, false);
    assert!(
        text.starts_with("Your practice run with claude-code-cli: recording."),
        "{text}"
    );
}

/// The Upload line says "up to date" only when every record written is acknowledged; between
/// the timed sends it says the upload works, how often it sends, and how many records wait.
#[test]
fn status_says_up_to_date_only_when_every_record_is_acknowledged() {
    use goalcup_cli::run::{HarnessState, RunState};
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    let mut state = RunState::launched(&info.entry_id, Ok(()));
    state.harness = HarnessState::Running;
    state.updated_at = goalcup_cli::run::now_unix();
    state.upload.last_ok_at = Some(control::parse_z(support::NOW).unwrap() - 16 * 60);
    run_with_state_on(
        &cfg,
        &info.entry_id,
        state,
        Some("https://server.example"),
        Some("2099-01-01T00:00:00Z"),
    );
    let trace = trace_of(&cfg, &info.entry_id);
    let chain: String = (0..=40u64)
        .map(|seq| format!("{{\"seq\":{seq},\"prev\":\"p\",\"hash\":\"h{seq}\"}}\n"))
        .collect();
    std::fs::write(trace.join("chain.jsonl"), chain).unwrap();
    let ack = |seq: u64| {
        let line = serde_json::json!({
            "entry_id": info.entry_id, "seq": seq, "hash": format!("h{seq}"),
            "acked_at": "2026-10-22T12:00:00Z", "server_signature": "00",
        });
        std::fs::write(trace.join("acks.jsonl"), format!("{line}\n")).unwrap();
    };
    let upload_line = |text: &str| {
        participant_block(text)
            .lines()
            .find(|l| l.contains("Upload:"))
            .unwrap_or_else(|| panic!("{text}"))
            .to_string()
    };

    ack(9);
    let line = upload_line(&status_text(&m, &cfg, &info.entry_id, false));
    assert!(!line.contains("up to date"), "{line}");
    assert!(line.contains("sent every 30 minutes"), "{line}");
    assert!(line.contains("last sent 16 min ago"), "{line}");
    assert!(line.contains("31 records wait for the next send"), "{line}");

    ack(40);
    let line = upload_line(&status_text(&m, &cfg, &info.entry_id, false));
    assert!(line.contains("up to date (last sent 16 min ago)"), "{line}");

    // No acknowledgement on this machine: every record waits, never "up to date".
    std::fs::remove_file(trace.join("acks.jsonl")).unwrap();
    let line = upload_line(&status_text(&m, &cfg, &info.entry_id, false));
    assert!(line.contains("41 records wait"), "{line}");
}

/// What `goalcup status` says once a run is over: sealed, uploaded, being finished, or left
/// unfinished by a goalcup process that stopped. The Next line is the action each state needs.
#[test]
fn status_names_the_next_action_for_every_state_of_a_run_that_is_over() {
    use goalcup_cli::run::{HarnessState, RunState};
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    let server = Some("https://server.example");
    let over = Some("2020-01-01T00:00:00Z");
    // The run's time is up and its goalcup process ended the agent for it.
    let state = |ended: bool, updated_at: u64| {
        let mut state = RunState::launched(&info.entry_id, Ok(()));
        state.harness = HarnessState::Exited {
            status: "signal 15".to_string(),
        };
        state.stopped = Some(
            "the server says the run is over (its deadline was 2020-01-01T00:00:00Z)".to_string(),
        );
        state.ended = ended;
        state.updated_at = updated_at;
        state
    };
    let fresh = goalcup_cli::run::now_unix();
    let said = |cfg: &std::path::Path| status_text(&m, cfg, &info.entry_id, false);
    let next_of = |text: &str| {
        participant_block(text)
            .lines()
            .find(|l| l.starts_with("Next:"))
            .unwrap_or_else(|| panic!("{text}"))
            .to_string()
    };

    // Sealed, part of it not on the server: send it.
    run_with_state_on(&cfg, &info.entry_id, state(true, fresh), server, over);
    let text = said(&cfg);
    assert!(participant_block(&text).contains("sealed"), "{text}");
    assert!(text.contains("Ended: "), "{text}");
    assert!(text.contains("Upload: not finished"), "{text}");
    assert!(next_of(&text).ends_with("goalcup upload"), "{text}");
    assert!(!text.contains("goalcup continue"), "{text}");

    // The same, and the upload failed: the problem line and the next line both say to run it.
    let mut failed = state(true, fresh);
    failed.upload.problem = Some("the server answered 503".to_string());
    failed.upload.last_ok_at = Some(fresh - 60);
    run_with_state_on(&cfg, &info.entry_id, failed, server, over);
    let text = said(&cfg);
    let problems = problem_lines(&text);
    assert_eq!(problems.len(), 1, "{text}");
    assert!(problems[0].contains("503"), "{text}");
    assert!(problems[0].contains("goalcup upload"), "{text}");
    assert!(next_of(&text).ends_with("goalcup upload"), "{text}");

    // All of it sent: done.
    let spool = cfg.join(goalcup_cli::upload::UPLOADS_DIR_NAME);
    std::fs::create_dir_all(&spool).unwrap();
    std::fs::write(spool.join(format!("{}.sent", info.entry_id)), b"sent\n").unwrap();
    run_with_state_on(&cfg, &info.entry_id, state(true, fresh), server, over);
    let text = said(&cfg);
    assert!(
        participant_block(&text).contains("sealed and uploaded"),
        "{text}"
    );
    assert!(text.contains("Upload: done"), "{text}");
    assert!(next_of(&text).contains("nothing to do"), "{text}");
    std::fs::remove_file(spool.join(format!("{}.sent", info.entry_id))).unwrap();

    // Time is up and goalcup is still finishing the run: wait.
    run_with_state_on(&cfg, &info.entry_id, state(false, fresh), server, over);
    let text = said(&cfg);
    assert!(participant_block(&text).contains("finishing"), "{text}");
    assert!(next_of(&text).contains("wait"), "{text}");
    assert!(problem_lines(&text).is_empty(), "{text}");

    // Time is up and nothing writes the state any more (the computer restarted) after its part
    // was sealed: no seal is asked for again, only the upload.
    run_with_state_on(&cfg, &info.entry_id, state(false, 1), server, over);
    leave_state_at(&cfg, &info.entry_id, 1);
    let text = said(&cfg);
    assert_eq!(next_of(&text), "Next: run goalcup upload", "{text}");
    assert!(!text.contains("goalcup seal"), "{text}");

    // The same before its part was sealed: waiting never helps. It is sealed by hand, then sent.
    std::fs::remove_file(trace_of(&cfg, &info.entry_id).join("seal.json")).unwrap();
    let text = said(&cfg);
    assert!(
        participant_block(&text).contains("stopped before it finished"),
        "{text}"
    );
    assert!(!text.contains("is finishing the run"), "{text}");
    let next = next_of(&text);
    assert!(
        next.contains("goalcup seal") && next.ends_with("goalcup upload"),
        "{text}"
    );
    assert!(!next.contains("wait"), "{text}");
    assert_eq!(problem_lines(&text).len(), 1, "{text}");

    // A run on this computer only: sealed here, nothing to send.
    run_with_state_on(&cfg, &info.entry_id, state(true, fresh), None, None);
    let text = said(&cfg);
    assert!(
        participant_block(&text).contains("sealed on this computer"),
        "{text}"
    );
    assert!(next_of(&text).contains("nothing to do"), "{text}");
}

/// T25: a run whose host finished without writing seal.json (a full disk) is not called sealed,
/// and the next action is to seal it, then upload.
#[test]
fn status_calls_a_run_sealed_only_when_its_seal_json_is_there() {
    use goalcup_cli::run::{HarnessState, RunState};
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    let mut state = RunState::launched(&info.entry_id, Ok(()));
    state.harness = HarnessState::Exited {
        status: "signal 15".to_string(),
    };
    state.stopped =
        Some("the server says the run is over (its deadline was 2020-01-01T00:00:00Z)".into());
    state.ended = true;
    let server = Some("https://server.example");
    let over = Some("2020-01-01T00:00:00Z");
    run_with_state_on(&cfg, &info.entry_id, state, server, over);
    let text = status_text(&m, &cfg, &info.entry_id, false);
    assert!(
        participant_block(&text).contains("sealed, not all"),
        "{text}"
    );

    std::fs::remove_file(trace_of(&cfg, &info.entry_id).join("seal.json")).unwrap();
    let text = status_text(&m, &cfg, &info.entry_id, false);
    let block = participant_block(&text);
    assert!(block.contains("not sealed"), "{text}");
    assert!(!block.contains("sealed, not all"), "{text}");
    let next = block.lines().find(|l| l.starts_with("Next:")).unwrap();
    assert!(
        next.contains("goalcup seal") && next.ends_with("goalcup upload"),
        "{text}"
    );
}

/// A run a newer run replaced did not end at its deadline, and the header does not say it did.
#[test]
fn status_of_a_run_closed_early_names_no_deadline_as_its_end() {
    use goalcup_cli::run::{HarnessState, RunState};
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    let mut state = RunState::launched(&info.entry_id, Ok(()));
    state.harness = HarnessState::Exited {
        status: "signal 15".to_string(),
    };
    state.stopped = Some("a newer run on your account replaced this one".to_string());
    run_with_state_on(
        &cfg,
        &info.entry_id,
        state,
        Some("https://server.example"),
        Some("2099-01-01T00:00:00Z"),
    );
    let text = status_text(&m, &cfg, &info.entry_id, false);
    let block = participant_block(&text);
    assert!(block.contains("Ended early"), "{text}");
    assert!(
        !block.contains("2099") && !block.contains("time is up"),
        "{text}"
    );
}

/// A state nobody updates is not called recording, and `goalcup continue` is the next action;
/// an upload nobody retries is not "kept trying".
#[test]
fn status_does_not_call_an_old_state_recording_or_an_ended_upload_retrying() {
    use goalcup_cli::run::{HarnessState, RunState};
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    let server = Some("https://server.example");
    let ahead = Some("2099-01-01T00:00:00Z");
    let mut state = RunState::launched(&info.entry_id, Ok(()));
    state.continuable = true;
    run_with_state_on(&cfg, &info.entry_id, state.clone(), server, ahead);
    leave_state_at(&cfg, &info.entry_id, 1);
    let text = status_text(&m, &cfg, &info.entry_id, false);
    let block = participant_block(&text);
    assert!(
        !block.contains("recording") && !block.contains("running"),
        "{text}"
    );
    assert!(
        block.contains(&format!("Next: run goalcup continue {}", info.entry_id)),
        "{text}"
    );

    // The agent left, the host ended its part, and the upload failed: nothing retries it.
    state.updated_at = goalcup_cli::run::now_unix();
    state.ended = true;
    state.harness = HarnessState::Exited {
        status: "exit code 1".to_string(),
    };
    state.upload.problem = Some("the server answered 503".to_string());
    run_with_state_on(&cfg, &info.entry_id, state, server, ahead);
    let text = status_text(&m, &cfg, &info.entry_id, false);
    assert!(text.contains("Upload: not finished"), "{text}");
    assert!(!text.contains("keeps trying"), "{text}");
}

/// An organizer test run says so first, before the daemon is asked and before any problem line.
#[test]
fn status_of_a_test_run_says_test_run_first() {
    use goalcup_cli::run::{RunState, read, write};
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    run_with_state(
        &cfg,
        &info.entry_id,
        RunState::launched(&info.entry_id, Ok(())),
    );
    let text = status_text(&m, &cfg, &info.entry_id, false);
    assert!(!text.contains("TEST RUN"), "{text}");

    let mut record = read(&cfg, &info.entry_id).unwrap();
    record.test = true;
    write(&cfg, &record).unwrap();
    let text = status_text(&m, &cfg, &info.entry_id, false);
    assert!(text.starts_with("TEST RUN"), "{text}");
    // Still the same status below it.
    assert!(text.contains(&m.status_detail()), "{text}");
    // `--json` stays the daemon's document alone.
    let json = status_text(&m, &cfg, &info.entry_id, true);
    assert!(!json.contains("TEST RUN"), "{json}");
}

/// A `run.json` written before `--test` existed has no `test` field and still reads, as a run
/// that is not a test run.
#[test]
fn a_run_json_without_the_test_field_still_reads() {
    use goalcup_cli::run::{read, run_dir, write};
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    run_with_state(
        &cfg,
        &info.entry_id,
        goalcup_cli::run::RunState::launched(&info.entry_id, Ok(())),
    );
    let mut record = read(&cfg, &info.entry_id).unwrap();
    record.test = true;
    write(&cfg, &record).unwrap();
    let file = run_dir(&cfg, &info.entry_id).join("run.json");
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(json["test"], true, "{json}");
    json.as_object_mut().unwrap().remove("test").unwrap();
    std::fs::write(&file, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
    let old = read(&cfg, &info.entry_id).unwrap();
    assert!(!old.test && !old.practice, "{old:?}");
    assert_eq!(
        old.mode(),
        goalcup_cli::account::device::RunMode::Competition
    );
}

/// `continue` starts its next segment in the mode the run was started in, read from `mode()`:
/// a test run stays a test run, a practice run a practice run.
#[test]
fn a_run_records_mode_is_what_continue_restarts_it_in() {
    use goalcup_cli::account::device::RunMode;
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    run_with_state(
        &cfg,
        &info.entry_id,
        goalcup_cli::run::RunState::launched(&info.entry_id, Ok(())),
    );
    let mut record = goalcup_cli::run::read(&cfg, &info.entry_id).unwrap();
    for (practice, test, mode) in [
        (false, false, RunMode::Competition),
        (true, false, RunMode::Practice),
        (false, true, RunMode::Test),
    ] {
        record.practice = practice;
        record.test = test;
        goalcup_cli::run::write(&cfg, &record).unwrap();
        let kept = goalcup_cli::run::read(&cfg, &info.entry_id).unwrap();
        assert_eq!(kept.mode(), mode);
        assert_eq!(kept.mode().is_practice(), practice);
    }
}

/// The binary's text for a machine that has started nothing: `status` says to run `prep` then
/// `start --harness <id>` (a chat-mode harness refuses `--goal-file`), and the help leads with
/// that same command before it mentions `--goal-file`.
#[test]
fn the_binary_leads_with_chat_mode_start() {
    let goalcup = env!("CARGO_BIN_EXE_goalcup");
    let cfg = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new(goalcup)
            .args(args)
            .env("GOALCUP_CONFIG_DIR", cfg.path())
            .output()
            .unwrap()
    };

    let out = run(&["status"]);
    let said = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{said}");
    assert!(
        said.contains("goalcup setup, then goalcup start --harness <id>"),
        "{said}"
    );
    assert!(!said.contains("--goal-file"), "{said}");

    let out = run(&["--help"]);
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("goalcup start --harness <id>"), "{help}");
    // Headless mode was removed: no goal file is offered anywhere.
    assert!(!help.contains("--goal-file"), "{help}");
}

/// `goalcup upload` with no id works on the newest run only, skips entries already sent, says
/// so when nothing is left, and says plainly that a local run has nothing to send.
#[test]
fn upload_with_no_id_picks_the_newest_runs_unsent_entries() {
    use goalcup_cli::commands::upload::{unsent_of_newest_run, upload_newest_with};
    use goalcup_cli::run::{EntryRef, RunRecord, write};
    let cfg = tempfile::tempdir().unwrap();
    let cfg = cfg.path();

    // Nothing started yet.
    let error = unsent_of_newest_run(cfg).unwrap_err();
    assert!(error.to_string().contains("nothing to upload"), "{error}");
    assert!(
        error.to_string().starts_with("Run goalcup start"),
        "{error}"
    );

    let run = |run_id: &str, server: Option<&str>, started: u64, entries: &[&str]| RunRecord {
        run_id: run_id.to_string(),
        server_url: server.map(str::to_string),
        practice: false,
        test: false,
        connector_id: "claude-code-cli".to_string(),
        workspace: PathBuf::from("/w"),
        started_at_unix: started,
        deadline_at: None,
        entries: entries
            .iter()
            .map(|id| EntryRef {
                entry_id: (*id).to_string(),
                trace_dir: PathBuf::from("/t"),
            })
            .collect(),
    };
    let mark_sent = |entry_id: &str| {
        let spool = cfg.join(goalcup_cli::upload::UPLOADS_DIR_NAME);
        std::fs::create_dir_all(&spool).unwrap();
        std::fs::write(spool.join(format!("{entry_id}.sent")), b"sent\n").unwrap();
    };
    // An older run with an unsent entry, and the newest run with three segments, the middle one
    // already sent.
    write(cfg, &run("old", Some("https://s.example"), 100, &["o-1"])).unwrap();
    write(
        cfg,
        &run(
            "new",
            Some("https://s.example"),
            200,
            &["n-1", "n-2", "n-3"],
        ),
    )
    .unwrap();
    mark_sent("n-2");
    let (run_id, unsent) = unsent_of_newest_run(cfg).unwrap();
    assert_eq!(run_id, "new");
    assert_eq!(unsent, ["n-1", "n-3"], "oldest first, the sent one skipped");

    // Everything of the newest run sent: said so, and the older run is left alone.
    mark_sent("n-1");
    mark_sent("n-3");
    let mut out = Vec::new();
    upload_newest_with(cfg, Default::default(), &mut out).unwrap();
    let said = String::from_utf8(out).unwrap();
    assert!(
        said.contains("Nothing left to send") && said.contains("run new"),
        "{said}"
    );

    // A newest run that was never opened on a server has nothing to send.
    write(cfg, &run("local", None, 300, &["l-1"])).unwrap();
    let error = unsent_of_newest_run(cfg).unwrap_err();
    assert!(error.to_string().contains("is local"), "{error}");
    assert!(
        error.to_string().contains("goalcup upload <part-id>"),
        "{error}"
    );
}
