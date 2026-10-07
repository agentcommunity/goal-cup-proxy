//! The fifteen health states, the signed status cache, and the bundled page.
//!
//! Three separate claims, and they fail for three different reasons:
//!
//! - The judgement (D3, D17). The daemon judges exactly five things and reports
//!   the rest as facts. `evaluate` is a priority ladder over those inputs, it is `now`-
//!   parameterised so it can be asked about any moment, and it reports what it passed over
//!   in `also` rather than dropping it, so an exited agent does not hide a
//!   stalled capture.
//! - The document (D25). One signed `control::SignedStatus`, verified with
//!   the run public key the caller pinned and never with the key the document names, and
//!   carrying no secret at all: the status route is unauthenticated, so a token on it
//!   would be a token on a route with no door.
//! - The page (D25). One file, one route, nothing loaded from the network, and
//!   the only entry it can ask about is the one its own query string names — because
//!   `/v0/entries/current` needs a bearer token and a browser has none.

mod support;

use goalcup_daemon::EntryArtifactRoot;
use goalcup_daemon::arming::{GateState, RefusalReason};
use goalcup_daemon::health::{HealthInputs, chains_as_event, evaluate};
use goalcup_daemon::registry::{
    Entry, EntryState, FinalizationPhaseV1, SealingIntent, WorkspaceSealingIntentV1,
    WorkspaceSourceBindingV1,
};
use goalcup_daemon::status::{
    build_payload, mirror, plain_words, sign_payload, write_entry_status_line_rooted,
};
#[cfg(feature = "v1-compat")]
use goalcup_daemon::ui::{STATUS_HTML, UiState, ui_router};
#[cfg(feature = "v1-compat")]
use goalcup_daemon::upload::{UploadClient, Uploader};
use goalcup_protocol::control::{ArmingState, SignedStatus};
use goalcup_protocol::events::HealthState;
use goalcup_protocol::provenance::{
    PreCutoffDrainOutcomeV1, Rfc3339Utc, Sha256Digest, TerminationFailureV1, TerminationMethodV1,
    TerminationOutcomeV1,
};
use goalcup_protocol::submission::{
    EffectiveSubmissionLimitsV1, FinishReasonV1, LimitProfileV1, SubmissionCapturedV1,
    SubmissionOutcomeV1, SubmissionProfileV1,
};
use std::collections::BTreeMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
#[cfg(unix)]
use std::os::unix::process::CommandExt as _;
#[cfg(unix)]
use std::process::Command;
#[cfg(feature = "v1-compat")]
use std::sync::{Arc, Mutex};

#[cfg(unix)]
const STATUS_UMASK_CHILD_ROOT_ENV: &str = "GOALCUP_STATUS_UMASK_CHILD_ROOT";

/// The fifteen, in the order `events.rs` declares them.
const ALL_FIFTEEN: [HealthState; 15] = [
    HealthState::Online,
    HealthState::UploadLagging,
    HealthState::ConnectionLost,
    HealthState::AgentExited,
    HealthState::TrackerInterrupted,
    HealthState::CaptureStalled,
    HealthState::HooksSilent,
    HealthState::ConfigRestored,
    HealthState::DeadlineReached,
    HealthState::ArmingPending,
    HealthState::ArmingLost,
    HealthState::VersionChanged,
    HealthState::ConfigMismatch,
    HealthState::StoreReplaced,
    HealthState::DiskPressure,
];

fn state_of(i: &HealthInputs) -> HealthState {
    evaluate(i, &support::policy()).state
}

fn sealed_entry_with_v4_termination(termination: TerminationOutcomeV1) -> Entry {
    let mut entry = support::started_entry();
    entry.state = EntryState::Sealed;
    entry.sealed_at = Some("2026-09-14T00:00:03Z".to_string());

    let intent = WorkspaceSealingIntentV1 {
        version: goalcup_daemon::registry::WorkspaceIntentVersionV1::WorkspaceSubmissionV1,
        entry_id: entry.entry_id.clone(),
        generation: 0,
        reason: FinishReasonV1::Manual,
        receipt_sha256: Sha256Digest::from_bytes([0xab; 32]),
        cutoff_at: Rfc3339Utc::try_from("2026-09-14T00:00:02Z".to_string()).unwrap(),
        trace_root_identity: "trace-root-v1:test".to_string(),
        source_date: ["2026".to_string(), "09".to_string(), "14".to_string()],
        source_root: WorkspaceSourceBindingV1::Available {
            journal: goalcup_daemon::SourceRootJournalV1 {
                identity: "source-root-v1:test".to_string(),
            },
        },
        submission_profile: SubmissionProfileV1::WorkspaceSnapshotV1,
        limits: EffectiveSubmissionLimitsV1 {
            object_count: 8,
            total_bytes: 1024,
            pre_cutoff_drain: 100,
            process_walk: 4,
        },
        limit_profile: LimitProfileV1 {
            id: "status-test".to_string(),
        },
        capture_limits: goalcup_daemon::CaptureResourceLimitsV1 {
            version: 1,
            namespace_nodes: 16,
            inventory_bytes: 1024,
            manifest_bytes: 1024,
        },
        excluded: vec![".goalcup".to_string()],
        phase: FinalizationPhaseV1::TerminalStatusPending,
        admission_watermarks: goalcup_daemon::registry::AdmissionWatermarksV1 {
            tailers: Vec::new(),
            receiver_sources: BTreeMap::new(),
            queued_receiver_posts: 0,
            admitted_chain_predecessor: None,
        },
        expected_cutoff_predecessor: None,
        drain: Some(PreCutoffDrainOutcomeV1::Complete),
        cutoff: None,
        accepted_handoff: None,
        termination: Some(termination),
        termination_report: None,
        submission: Some(SubmissionOutcomeV1::Captured(SubmissionCapturedV1 {
            manifest_sha256: Sha256Digest::from_bytes([0xcd; 32]),
        })),
        finalization_at: None,
        expected_finalization_predecessor: None,
        finalization: None,
        terminal_status_at: None,
        status_mirror_complete: false,
    };

    let mut raw = serde_json::to_value(&entry).unwrap();
    raw["sealing_intent"] =
        serde_json::to_value(SealingIntent::Workspace(Box::new(intent))).unwrap();
    serde_json::from_value(raw).unwrap()
}

fn unresolved_termination() -> TerminationOutcomeV1 {
    TerminationOutcomeV1::Unresolved {
        requested_at: Rfc3339Utc::try_from("2026-09-14T00:00:02Z".to_string()).unwrap(),
        stopped_waiting_at: Rfc3339Utc::try_from("2026-09-14T00:00:03Z".to_string()).unwrap(),
        reason: TerminationFailureV1::ProcessAuthorityUnavailable,
    }
}

fn confirmed_termination() -> TerminationOutcomeV1 {
    TerminationOutcomeV1::Confirmed {
        requested_at: Rfc3339Utc::try_from("2026-09-14T00:00:02Z".to_string()).unwrap(),
        confirmed_at: Rfc3339Utc::try_from("2026-09-14T00:00:03Z".to_string()).unwrap(),
        method: TerminationMethodV1::ProcessSignal,
    }
}

#[test]
fn the_agent_exiting_never_hides_a_stalled_capture() {
    let mut i = support::inputs();
    i.agent_exited = true;
    i.last_leg1_at = Some(i.now - 3_600.0);
    let r = evaluate(&i, &support::policy());
    assert_eq!(r.state, HealthState::CaptureStalled);
    assert!(
        r.also.contains(&HealthState::AgentExited),
        "MF-8: both facts, not one"
    );
}

#[test]
fn only_the_states_with_no_event_of_their_own_are_chained() {
    for s in [
        HealthState::ConfigMismatch,
        HealthState::ConfigRestored,
        HealthState::ArmingLost,
        HealthState::VersionChanged,
        HealthState::StoreReplaced,
        HealthState::TrackerInterrupted,
    ] {
        assert!(
            !chains_as_event(s),
            "{s:?} already has a dedicated EventBody (round 2 §7)"
        );
    }
    for s in [
        HealthState::Online,
        HealthState::UploadLagging,
        HealthState::ConnectionLost,
        HealthState::AgentExited,
        HealthState::CaptureStalled,
        HealthState::HooksSilent,
        HealthState::DeadlineReached,
        HealthState::ArmingPending,
        HealthState::DiskPressure,
    ] {
        assert!(
            chains_as_event(s),
            "{s:?} would otherwise exist only in the status cache"
        );
    }
    // Exactly six, so a state added to the enum cannot quietly join the silent side.
    assert_eq!(
        ALL_FIFTEEN.iter().filter(|s| !chains_as_event(**s)).count(),
        6
    );
}

#[test]
fn every_r14_state_but_hooks_silent_is_reachable_from_health_inputs() {
    assert_eq!(state_of(&support::inputs()), HealthState::Online);

    let mut pending = support::inputs();
    pending.arming = GateState::Armed;
    assert_eq!(state_of(&pending), HealthState::ArmingPending);

    let mut refused = support::inputs();
    refused.arming = GateState::Refused(RefusalReason::StartSignalMissing);
    assert_eq!(state_of(&refused), HealthState::ArmingPending);

    let mut lost = support::inputs();
    lost.arming_lost = true;
    assert_eq!(state_of(&lost), HealthState::ArmingLost);

    let mut mismatch = support::inputs();
    mismatch.config_mismatch = true;
    assert_eq!(state_of(&mismatch), HealthState::ConfigMismatch);

    let mut changed = support::inputs();
    changed.version_changed = true;
    assert_eq!(state_of(&changed), HealthState::VersionChanged);

    let mut stalled = support::inputs();
    stalled.last_leg1_at = Some(1_000.0);
    assert_eq!(state_of(&stalled), HealthState::CaptureStalled);

    let mut exited = support::inputs();
    exited.harness_alive = false;
    exited.agent_exited = true;
    assert_eq!(state_of(&exited), HealthState::AgentExited);

    let mut offline = support::inputs();
    offline.connected = false;
    assert_eq!(state_of(&offline), HealthState::ConnectionLost);

    let mut lagging = support::inputs();
    lagging.pending_events = 400;
    lagging.last_ack_at = Some(9_000.0);
    assert_eq!(state_of(&lagging), HealthState::UploadLagging);

    let mut done = support::inputs();
    done.deadline_at = Some(9_000.0);
    assert_eq!(state_of(&done), HealthState::DeadlineReached);

    // The on-disk signature of a rewritten transcript.
    let mut replaced = support::inputs();
    replaced.store_replaced = true;
    assert_eq!(state_of(&replaced), HealthState::StoreReplaced);

    // Reported while there is still room to act on it.
    let mut full = support::inputs();
    full.disk_pressure = true;
    assert_eq!(state_of(&full), HealthState::DiskPressure);

    // Both of these have plain words and an input that produces them.
    let mut restored = support::inputs();
    restored.config_restored = true;
    assert_eq!(state_of(&restored), HealthState::ConfigRestored);

    let mut restarted = support::inputs();
    restarted.tracker_restarted = true;
    assert_eq!(state_of(&restarted), HealthState::TrackerInterrupted);
}

#[test]
fn evaluate_reads_its_now_from_the_caller_and_never_from_the_clock() {
    // The same inputs, asked about two moments: before the deadline and after it. A
    // function that read `now_unix()` would answer the same thing twice.
    let mut i = support::inputs();
    i.deadline_at = Some(10_500.0);
    assert_eq!(state_of(&i), HealthState::Online);
    i.now = 10_501.0;
    assert_eq!(state_of(&i), HealthState::DeadlineReached);
}

#[test]
fn an_exited_agent_is_reported_alongside_a_stalled_capture_not_instead_of_it() {
    let mut both = support::inputs();
    both.harness_alive = false;
    both.last_leg1_at = Some(1_000.0);
    let report = evaluate(&both, &support::policy());
    assert_eq!(
        report.state,
        HealthState::CaptureStalled,
        "the more specific fact leads"
    );
    assert!(report.also.contains(&HealthState::AgentExited));
    assert_eq!(report.states().len(), 2);

    // With nothing else wrong, the exit is the story.
    let mut only_exited = support::inputs();
    only_exited.harness_alive = false;
    let report = evaluate(&only_exited, &support::policy());
    assert_eq!(report.state, HealthState::AgentExited);
    assert!(report.also.is_empty());
}

#[test]
fn clock_skew_backoff_and_an_unverified_acknowledgement_are_details_not_states() {
    // The clocks disagreeing is a fact about the trace, not a capture
    // failure, so it rides on whatever state applies rather than displacing it.
    let mut skewed = support::inputs();
    skewed.clock_skew_seconds = Some(21_600.0);
    skewed.backing_off = true;
    let report = evaluate(&skewed, &support::policy());
    assert_eq!(
        report.state,
        HealthState::Online,
        "capture is fine; the clocks are not"
    );
    assert!(
        report.details.iter().any(|d| d.contains("clock_skew")),
        "{:?}",
        report.details
    );
    assert!(
        report.details.iter().any(|d| d.contains("backing off")),
        "{:?}",
        report.details
    );

    // An acknowledgement the pinned key did not vouch for is no anchor at all,
    // so it reads as `ConnectionLost` and says why.
    let mut unverified = support::inputs();
    unverified.ack_unverified = true;
    let report = evaluate(&unverified, &support::policy());
    assert_eq!(report.state, HealthState::ConnectionLost);
    assert!(
        report.details.iter().any(|d| d.contains("did not verify")),
        "{:?}",
        report.details
    );
}

#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn configured_clock_skew_threshold_is_applied() {
    // The same six-hour divergence, acknowledged twice: under the default threshold it is a
    // `clock_skew` detail, and under a machine's configured `clock_skew_seconds` that
    // tolerates it, it is nothing.
    let t = tempfile::tempdir().unwrap();
    let mut h = support::harness_with_devserver(t.path()).await;
    h.devserver.set_clock(&goalcup_daemon::unix_to_rfc3339(
        goalcup_daemon::now_unix() + 21_600.0,
    ));
    let details = |skew: Option<f64>| {
        let mut inputs = support::inputs();
        inputs.clock_skew_seconds = skew;
        evaluate(&inputs, &support::policy()).details
    };

    h.buffer.append(&support::envelope(0, "main")).unwrap();
    let default = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert_eq!(default.acked_seq, Some(0), "{default:?}");
    let reported = details(default.clock_skew_seconds);
    assert!(
        reported.iter().any(|d| d.contains("clock_skew")),
        "{reported:?}"
    );

    let mut config = support::config_at(t.path(), &h.url);
    config.clock_skew_seconds = 1.0e9;
    h.uploader = Uploader::new(
        support::ENTRY_ID,
        UploadClient::new(config.upload_url.clone()),
        config.upload_chunk_max,
        h.run_keys.clone(),
        h.devserver.public_key_hex(),
        config.clock_skew_seconds,
    );
    h.buffer.append(&support::envelope(1, "main")).unwrap();
    let configured = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert_eq!(configured.acked_seq, Some(1), "{configured:?}");
    let reported = details(configured.clock_skew_seconds);
    assert!(
        !reported.iter().any(|d| d.contains("clock_skew")),
        "{reported:?}"
    );
}

#[test]
fn the_signed_status_verifies_against_the_run_key_and_carries_no_secret() {
    let e = support::started_entry();
    let key = e.keys.signing_key().unwrap();
    let signed = sign_payload(
        build_payload(&support::status_inputs(&e)),
        "2026-09-02T00:00:00Z".to_string(),
        &key,
    )
    .unwrap();
    assert!(
        signed
            .verify_with(&e.keys.public_key_hex().unwrap())
            .is_ok()
    );
    assert!(
        signed.verify_with(&"aa".repeat(32)).is_err(),
        "protocol C1: a document that names the key that checks it checks nothing"
    );

    let json = serde_json::to_string(&signed).unwrap();
    for secret in [
        e.keys.receiver_token.as_str(),
        e.keys.buffer_key_hex.as_str(),
        e.keys.signing_seed_hex.as_str(),
    ] {
        assert!(
            !json.contains(secret),
            "the status route is unauthenticated: no secrets on it"
        );
    }
    let t = tempfile::tempdir().unwrap();
    mirror(&signed, t.path()).unwrap();
    assert!(
        t.path()
            .join(format!("cache/status-{}.json", e.entry_id))
            .exists()
    );
    assert!(t.path().join("cache/status-latest.json").exists());
    let artifact = EntryArtifactRoot::create(t.path(), &e.entry_id).unwrap();
    write_entry_status_line_rooted(&signed, &artifact).unwrap();
    let line = std::fs::read_to_string(artifact.display_path().join("status-line.txt")).unwrap();
    assert_eq!(line.lines().count(), 1);
    assert!(!line.contains(&e.keys.receiver_token));
}

/// "Never started" is `started_at`, on the participant line as well as
/// in the eviction test — not `arming != Started`.
///
/// The entry this builds is the one the two definitions disagree about, and it is not a
/// contrived one: a run that started, sealed, and whose arming gate was rebuilt `pending`
/// because the daemon restarted after the seal. The gate is runtime state and a restart
/// rebuilds it from what the tick has seen since — which, after a seal, is nothing. Its
/// `started_at` is the start receipt's own stamp and no restart forgets it. Read off the
/// gate, this entry's last document would tell the participant "sealed, never started — nothing
/// to upload" about a run with a full chain on disk.
#[test]
fn a_sealed_entry_that_started_is_never_described_as_never_started() {
    let mut sealed = support::started_entry();
    sealed.sealed_at = Some("2026-09-03T09:12:44Z".to_string());

    let mut inputs = support::status_inputs(&sealed);
    // What a daemon restarted after the seal rebuilds: the gate is `Armed` and has
    // witnessed no start of its own, which is `arming != Started`.
    inputs.arming = GateState::Armed;
    let after_restart = build_payload(&inputs);
    assert_eq!(after_restart.health, HealthState::Sealed);
    assert!(
        after_restart.detail.starts_with("This entry is sealed."),
        "a run that started is not a run that never started: {}",
        after_restart.detail
    );

    // And the entry the sentence exists for still gets it: sealed with no start stamp.
    let mut never = sealed.clone();
    never.started_at = None;
    let mut inputs = support::status_inputs(&never);
    inputs.arming = GateState::Armed;
    assert_eq!(
        build_payload(&inputs).detail,
        "sealed, never started — nothing to upload"
    );
}

#[test]
fn v4_nonconfirmed_termination_forces_harness_review_without_legacy_seal_outcome() {
    for termination in [
        unresolved_termination(),
        TerminationOutcomeV1::AlreadyStopped {
            checked_at: Rfc3339Utc::try_from("2026-09-14T00:00:03Z".to_string()).unwrap(),
        },
    ] {
        let entry = sealed_entry_with_v4_termination(termination);
        assert!(
            entry.seal_outcome.is_none(),
            "the V4 intent is the only ending fact"
        );

        let payload = build_payload(&support::status_inputs(&entry));

        assert!(payload.harness_unverified);
        assert_eq!(payload.health, HealthState::SealingRecoveryNeeded);
        assert!(
            payload.detail.contains("termination") && payload.detail.contains("review"),
            "non-confirmed termination needs actionable detail: {}",
            payload.detail
        );
        assert!(matches!(
            payload.finalization,
            Some(
                goalcup_protocol::control::WorkspaceFinalizationStatusV1::Terminal {
                    termination: TerminationOutcomeV1::Unresolved { .. }
                        | TerminationOutcomeV1::AlreadyStopped { .. },
                    ..
                }
            )
        ));
    }
}

#[test]
fn v4_confirmed_termination_stays_clean_and_legacy_rows_keep_sealed_behavior() {
    let confirmed = sealed_entry_with_v4_termination(confirmed_termination());
    let payload = build_payload(&support::status_inputs(&confirmed));
    assert!(!payload.harness_unverified);
    assert_eq!(payload.health, HealthState::Sealed);
    assert!(payload.detail.starts_with("This entry is sealed."));

    let mut legacy = support::started_entry();
    legacy.state = EntryState::Sealed;
    legacy.sealed_at = Some("2026-09-14T00:00:03Z".to_string());
    let payload = build_payload(&support::status_inputs(&legacy));
    assert!(payload.finalization.is_none());
    assert!(!payload.harness_unverified);
    assert_eq!(payload.health, HealthState::Sealed);
    assert!(payload.detail.starts_with("This entry is sealed."));
}

#[test]
fn an_unreconciled_sealing_intent_is_not_reported_as_live_or_sealed() {
    let mut sealing = support::started_entry();
    sealing.state = goalcup_daemon::registry::EntryState::Sealing;
    sealing.sealed_at = None;
    let mut inputs = support::status_inputs(&sealing);
    inputs.arming = GateState::Started;
    let payload = build_payload(&inputs);
    assert_eq!(payload.health, HealthState::SealingRecoveryNeeded);
    assert_eq!(payload.sealed_at, None);
    assert_eq!(payload.seconds_remaining, Some(0));
    assert!(payload.arming_recipe.is_none());
    assert!(payload.detail.contains("Capture is closed"));
}

/// A version nothing measured is absent from the document, never an empty
/// string sitting inside the `Some`.
///
/// The registry row's field is a `String` because the version-drift event on it is a pair of
/// strings; the document's is an `Option`, and `Some("")` is a claim that a version was read
/// whose value happens to be nothing. The payload is what a reader of a sealed entry gets.
#[test]
fn an_unprobed_harness_version_is_absent_from_the_document_rather_than_empty() {
    let probed = support::started_entry();
    assert_eq!(
        build_payload(&support::status_inputs(&probed))
            .harness_version
            .as_deref(),
        Some("2.1.259"),
        "a measured version still travels"
    );

    let mut unprobed = support::started_entry();
    unprobed.harness_version = String::new();
    assert_eq!(
        build_payload(&support::status_inputs(&unprobed)).harness_version,
        None,
        "nothing measured it, and the document says so rather than saying an empty string"
    );
}

#[test]
fn the_payload_counts_down_and_hides_the_arming_recipe_once_the_gate_clears() {
    let e = support::started_entry();
    let started = build_payload(&support::status_inputs(&e));
    assert_eq!(started.arming, ArmingState::Started);
    assert_eq!(started.seconds_remaining, Some(10_000));
    assert_eq!(started.arming_recipe, None, "the gate has cleared");
    assert_eq!(started.entry_id, e.entry_id);
    assert_eq!(started.harness_version.as_deref(), Some("2.1.259"));

    let mut waiting = support::status_inputs(&e);
    waiting.arming = GateState::Armed;
    waiting.health = HealthState::ArmingPending;
    waiting.now = 30_000.0;
    let waiting = build_payload(&waiting);
    assert_eq!(waiting.arming, ArmingState::Armed);
    assert_eq!(
        waiting.seconds_remaining,
        Some(0),
        "a passed deadline saturates at zero rather than going negative"
    );
    assert_eq!(
        waiting.arming_recipe, None,
        "no connector has a human arming step since the hook leg was cut"
    );
    assert_eq!(
        waiting.detail,
        plain_words(HealthState::ArmingPending, GateState::Armed, false)
    );
}

#[test]
fn every_one_of_the_fifteen_states_has_words_a_participant_can_act_on() {
    // The health state in plain words: a state with no words reaches the status
    // page as a bare enum name, which is exactly what D25's participant surface is not for.
    for h in ALL_FIFTEEN {
        let words = plain_words(h, GateState::Started, true);
        assert!(!words.is_empty(), "{h:?} has no words");
        assert!(
            !words.contains('_'),
            "{h:?}: plain words, not an enum name: {words}"
        );
    }
    assert!(
        plain_words(HealthState::Online, GateState::Started, true)
            .to_lowercase()
            .contains("recording")
    );
    assert!(
        plain_words(HealthState::ArmingPending, GateState::Armed, false)
            .to_lowercase()
            .contains("not")
    );
}

#[test]
fn the_cache_is_mirrored_where_the_cli_the_mcp_server_and_the_page_read_it() {
    let dir = tempfile::tempdir().unwrap();
    let e = support::started_entry();
    let key = e.keys.signing_key().unwrap();
    let public = e.keys.public_key_hex().unwrap();
    let signed = sign_payload(
        build_payload(&support::status_inputs(&e)),
        "2026-09-04T09:00:00Z".to_string(),
        &key,
    )
    .unwrap();
    mirror(&signed, dir.path()).unwrap();
    // Mirroring twice replaces rather than failing: the daemon writes this every second.
    mirror(&signed, dir.path()).unwrap();

    for name in ["status-entry-0001.json", "status-latest.json"] {
        let path = dir.path().join("cache").join(name);
        let back: SignedStatus =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        back.verify_with(&public).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o644,
                "{name} is readable by the MCP server, and holds no secret"
            );
        }
    }

    // The status line is one line, carries no credential, and names no absolute path from
    // the participant's home (D25).
    let artifact = EntryArtifactRoot::create(dir.path(), &signed.payload.entry_id).unwrap();
    write_entry_status_line_rooted(&signed, &artifact).unwrap();
    let line = std::fs::read_to_string(artifact.display_path().join("status-line.txt")).unwrap();
    assert_eq!(line.lines().count(), 1, "one line, got {line:?}");
    assert!(line.contains("/goal Cup"));
    assert!(!line.contains(&signed.payload.workspace), "got {line}");
    for secret in ["bearer", "token", "grant", "key", "seed"] {
        assert!(!line.to_lowercase().contains(secret), "got {line}");
    }
}

/// The cache's public documents request 0644, even when the daemon inherits a restrictive
/// umask.  The child process keeps this test's umask change out of the parallel test runner.
#[cfg(unix)]
#[test]
fn status_cache_publication_preserves_requested_mode_under_restrictive_umask() {
    if let Some(root) = std::env::var_os(STATUS_UMASK_CHILD_ROOT_ENV) {
        let root = std::path::PathBuf::from(root);
        let e = support::started_entry();
        let key = e.keys.signing_key().unwrap();
        let signed = sign_payload(
            build_payload(&support::status_inputs(&e)),
            "2026-09-04T09:00:00Z".to_string(),
            &key,
        )
        .unwrap();
        mirror(&signed, &root).unwrap();

        for name in ["status-entry-0001.json", "status-latest.json"] {
            let path = root.join("cache").join(name);
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o644,
                "{name} must keep the requested public cache mode"
            );
        }
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "status_cache_publication_preserves_requested_mode_under_restrictive_umask",
            "--nocapture",
        ])
        .env(STATUS_UMASK_CHILD_ROOT_ENV, temp.path());
    // SAFETY: this closure runs after fork and before exec, and umask(2) is async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            libc::umask(0o077);
            Ok(())
        });
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "child status cache test failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_summary_is_the_entry_a_reader_sees_in_a_list() {
    let e = support::started_entry();
    let row = goalcup_daemon::status::summary(&e, GateState::Started, HealthState::Online);
    assert_eq!(row.entry_id, "entry-0001");
    assert_eq!(row.arming, ArmingState::Started);
    assert_eq!(row.health, HealthState::Online);
    assert_eq!(row.started_at, e.started_at);
    assert_eq!(row.sealed_at, None);
}

#[cfg(feature = "v1-compat")]
#[test]
fn the_page_is_one_file_that_loads_nothing_and_polls_only_its_own_entry() {
    let html = STATUS_HTML;
    for forbidden in [
        "http://",
        "https://cdn",
        "//fonts.",
        "<script src=",
        "<img src=\"http",
        "<link href=",
    ] {
        assert!(
            !html.contains(forbidden),
            "R18: loads nothing from the network ({forbidden})"
        );
    }
    assert!(
        html.contains("?entry=") || html.contains("entry"),
        "it reads its own query string"
    );
    assert!(html.contains("/v0/entries/"));
    assert!(
        !html.contains("/v0/entries/current"),
        "authenticated; a browser has no token"
    );
    assert!(
        !html.contains("/v0/assets/"),
        "the sponsor list is inline (round 2 §7)"
    );
    assert!(html.contains("Agent Community"));
    // The one absolute URL in the file is the Agent Community link, and it is a link
    // rather than a load.
    assert_eq!(
        html.matches("https://").count(),
        html.matches("https://agentcommunity.org").count()
    );
    // Every fetch is the one relative route the page may use.
    for line in html.lines().filter(|l| l.contains("fetch(")) {
        assert!(line.contains("/v0/entries/${"), "unexpected fetch: {line}");
    }
}

#[cfg(feature = "v1-compat")]
#[test]
fn legacy_surfaces_do_not_claim_m2_health() {
    assert!(STATUS_HTML.contains("if (e.capture)"));
    assert!(STATUS_HTML.contains("M2 capture details are unavailable in this page."));
    assert!(STATUS_HTML.contains("Run `goalcup status` for capture health."));
    let guard = STATUS_HTML.find("if (e.capture)").unwrap();
    let legacy_health = STATUS_HTML.find("row(dl, \"Health\", e.health)").unwrap();
    assert!(
        guard < legacy_health,
        "M2 rows must return before legacy health renders"
    );
}

#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn the_local_endpoint_serves_the_page_without_a_token() {
    let e = support::started_entry();
    let signed = sign_payload(
        build_payload(&support::status_inputs(&e)),
        "2026-09-04T09:00:00Z".to_string(),
        &e.keys.signing_key().unwrap(),
    )
    .unwrap();
    let state = UiState {
        cache: Arc::new(Mutex::new(BTreeMap::from([(
            e.entry_id.clone(),
            signed.clone(),
        )]))),
        current: Arc::new(Mutex::new(Some(e.entry_id.clone()))),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        let _ = axum::serve(listener, ui_router(state)).await;
    });

    // No Authorization header anywhere in this test: that is the point.
    let served = addr.clone();
    let (status, body) = tokio::task::spawn_blocking(move || {
        support::http_get(&served, goalcup_protocol::control::ROUTE_UI)
    })
    .await
    .unwrap();
    assert_eq!(status, 200, "a browser has no token to send");
    assert_eq!(body, STATUS_HTML, "one file, served whole");
}

#[cfg(feature = "v1-compat")]
#[test]
fn the_ui_state_is_per_entry() {
    // The control API serves `GET /v0/entries/{id}/status` off this same state, so a single
    // `Option<SignedStatus>` cannot answer it.
    let e = support::started_entry();
    let key = e.keys.signing_key().unwrap();
    let public = e.keys.public_key_hex().unwrap();
    let mut map: BTreeMap<String, SignedStatus> = BTreeMap::new();
    for id in ["entry-0001", "entry-0002"] {
        map.insert(
            id.to_string(),
            sign_payload(
                support::payload_for(id),
                "2026-09-04T09:00:00Z".to_string(),
                &key,
            )
            .unwrap(),
        );
    }
    let state = UiState {
        cache: Arc::new(Mutex::new(map)),
        current: Arc::new(Mutex::new(Some("entry-0002".to_string()))),
    };
    let cache = state.cache.lock().unwrap();
    assert_eq!(cache.len(), 2);
    assert_eq!(cache["entry-0001"].payload.entry_id, "entry-0001");
    cache["entry-0001"].verify_with(&public).unwrap();
    assert_eq!(state.current.lock().unwrap().as_deref(), Some("entry-0002"));
}

// ---------------------------------------------------------------------------
// The shared temp name, the page's copied threshold,
// the refusal's wire state, and the last minute of the clock.
// ---------------------------------------------------------------------------

/// Two entries writing the cache must not fight over one temp file.
///
/// `status-latest.json` is written from a temp file in the same directory, and a temp
/// name derived from the target alone is the same name for every entry — so two mirrors
/// in flight could publish one entry's document under the other's rename. The sequential
/// half of the claim: nothing is left behind, and the last writer is what `status-latest`
/// holds.
#[test]
fn mirroring_two_entries_leaves_no_temp_file_behind() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = support::started_entry();
    first.entry_id = "entry-0001".to_string();
    let mut second = support::started_entry();
    second.entry_id = "entry-0002".to_string();

    let mut last_public = String::new();
    for e in [&first, &second] {
        let signed = sign_payload(
            build_payload(&support::status_inputs(e)),
            "2026-09-04T09:00:00Z".to_string(),
            &e.keys.signing_key().unwrap(),
        )
        .unwrap();
        mirror(&signed, dir.path()).unwrap();
        last_public = e.keys.public_key_hex().unwrap();
    }

    let cache = dir.path().join("cache");
    let strays: Vec<String> = std::fs::read_dir(&cache)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(
        strays.is_empty(),
        "a temp file survived the rename: {strays:?}"
    );

    let latest: SignedStatus =
        serde_json::from_str(&std::fs::read_to_string(cache.join("status-latest.json")).unwrap())
            .unwrap();
    assert_eq!(
        latest.payload.entry_id, "entry-0002",
        "two runs that started in the same second are ordered by their ids, and the newest \
         entry is the highest id"
    );
    latest.verify_with(&last_public).unwrap();
}

/// `status-latest.json` is the one document `goalcup-mcp` reads when
/// it has not been told which entry it wants, and `Daemon::refresh_statuses` walks the entry
/// map in ID order — so writing the pointer on every mirror would make "latest" mean "the
/// highest id".
///
/// The pointer is the most recently started run. Here the lexically smaller id started
/// later and is written first, which is the order the refresh visits them in: the second
/// mirror must leave the pointer alone rather than take it.
#[test]
fn the_shared_pointer_names_the_most_recently_started_run_and_not_the_highest_id() {
    let dir = tempfile::tempdir().unwrap();
    let mut newer = support::started_entry();
    newer.entry_id = "entry-0001".to_string();
    newer.started_at = Some("2026-09-04T11:00:00Z".to_string());
    let mut older = support::started_entry();
    older.entry_id = "entry-0002".to_string();
    older.started_at = Some("2026-09-03T07:42:00Z".to_string());

    for e in [&newer, &older] {
        let signed = sign_payload(
            build_payload(&support::status_inputs(e)),
            "2026-09-04T11:00:05Z".to_string(),
            &e.keys.signing_key().unwrap(),
        )
        .unwrap();
        mirror(&signed, dir.path()).unwrap();
    }

    let cache = dir.path().join("cache");
    let latest: SignedStatus =
        serde_json::from_str(&std::fs::read_to_string(cache.join("status-latest.json")).unwrap())
            .unwrap();
    assert_eq!(
        latest.payload.entry_id, "entry-0001",
        "the pointer named the highest id rather than the run that started last"
    );
    latest
        .verify_with(&newer.keys.public_key_hex().unwrap())
        .unwrap();

    // The per-entry files are untouched by this: each entry still has its own, and
    // the older run's document is still where a caller that names it reads.
    for (name, id) in [
        ("status-entry-0001.json", "entry-0001"),
        ("status-entry-0002.json", "entry-0002"),
    ] {
        let doc: SignedStatus =
            serde_json::from_str(&std::fs::read_to_string(cache.join(name)).unwrap()).unwrap();
        assert_eq!(doc.payload.entry_id, id);
    }

    // A refresh must be able to replace its own pointer, or it would freeze at the first
    // publication of the run that holds it.
    let signed = sign_payload(
        build_payload(&support::status_inputs(&newer)),
        "2026-09-04T11:00:10Z".to_string(),
        &newer.keys.signing_key().unwrap(),
    )
    .unwrap();
    mirror(&signed, dir.path()).unwrap();
    let latest: SignedStatus =
        serde_json::from_str(&std::fs::read_to_string(cache.join("status-latest.json")).unwrap())
            .unwrap();
    assert_eq!(latest.cached_at, "2026-09-04T11:00:10Z");
}

/// The concurrent half: two entries mirroring at once leave a document that parses and
/// verifies against one of the two run keys — never a half-written one, and never one
/// entry's bytes under the other's signature.
#[test]
fn two_entries_mirroring_at_once_leave_a_whole_document() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let mut publics = Vec::new();
    let mut handles = Vec::new();
    for id in ["entry-0001", "entry-0002"] {
        let mut e = support::started_entry();
        e.entry_id = id.to_string();
        publics.push(e.keys.public_key_hex().unwrap());
        let root = root.clone();
        handles.push(std::thread::spawn(move || {
            let key = e.keys.signing_key().unwrap();
            for _ in 0..50 {
                let signed = sign_payload(
                    build_payload(&support::status_inputs(&e)),
                    "2026-09-04T09:00:00Z".to_string(),
                    &key,
                )
                .unwrap();
                mirror(&signed, &root).unwrap();
            }
        }));
    }
    // While they write, keep reading: a rename is atomic, so every read that returns
    // bytes must return a document that verifies.
    let cache = root.join("cache");
    let latest = cache.join("status-latest.json");
    while handles.iter().any(|h| !h.is_finished()) {
        if let Ok(raw) = std::fs::read_to_string(&latest)
            && let Ok(doc) = serde_json::from_str::<SignedStatus>(&raw)
        {
            assert!(
                publics.iter().any(|p| doc.verify_with(p).is_ok()),
                "a mirrored document did not verify against either run key"
            );
        }
    }
    for h in handles {
        h.join().unwrap();
    }

    let strays: Vec<String> = std::fs::read_dir(&cache)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(strays.is_empty(), "a temp file survived: {strays:?}");
    let doc: SignedStatus =
        serde_json::from_str(&std::fs::read_to_string(&latest).unwrap()).unwrap();
    assert!(
        publics.iter().any(|p| doc.verify_with(p).is_ok()),
        "the document left behind must verify"
    );
}

/// The page copies the daemon's staleness threshold into JavaScript. It may copy it, but
/// it may not drift from it: this asserts the literal in the file is
/// `control::STALE_AFTER_SECONDS`, so moving the constant fails the build's tests rather
/// than quietly telling the participant a different story.
#[cfg(feature = "v1-compat")]
#[test]
fn the_page_reads_stale_at_the_protocol_constant() {
    let expected = format!(
        "STALE_AFTER_SECONDS = {};",
        goalcup_protocol::control::STALE_AFTER_SECONDS
    );
    assert!(
        STATUS_HTML.contains(&expected),
        "assets/status.html must spell {expected:?}"
    );
}

/// The last minute is not "0h 0m left". A participant with forty seconds on the clock is
/// being told something different from a participant with forty minutes.
#[cfg(feature = "v1-compat")]
#[test]
fn the_final_minute_is_not_rendered_as_zero() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = support::started_entry();
    // `status_inputs` asks at now = 10_000.
    e.deadline_unix = Some(10_059.0);
    let signed = sign_payload(
        build_payload(&support::status_inputs(&e)),
        "2026-09-04T09:00:00Z".to_string(),
        &e.keys.signing_key().unwrap(),
    )
    .unwrap();
    assert_eq!(signed.payload.seconds_remaining, Some(59));
    let artifact = EntryArtifactRoot::create(dir.path(), &e.entry_id).unwrap();
    write_entry_status_line_rooted(&signed, &artifact).unwrap();
    let line = std::fs::read_to_string(artifact.display_path().join("status-line.txt")).unwrap();
    assert!(line.contains("<1m left"), "got {line:?}");
    assert!(!line.contains("0h 0m"), "got {line:?}");

    // And the page renders the same minute the same way.
    assert!(
        STATUS_HTML.contains("\"<1m\""),
        "the page must not print 0h 0m for the last minute either"
    );
}

/// A refused gate has its own wire state, `ArmingState::Refused`, so a machine reader
/// keying on `arming` alone can tell a run waiting to arm from one that never will.
/// `detail` carries the refusal in words for a human reader.
#[test]
fn a_refused_gate_reports_refused_and_says_why_in_words() {
    let e = support::started_entry();
    let mut inputs = support::status_inputs(&e);
    inputs.arming = GateState::Refused(RefusalReason::StartSignalMissing);
    inputs.health = HealthState::ArmingPending;
    let payload = build_payload(&inputs);
    assert_eq!(
        payload.arming,
        ArmingState::Refused,
        "a refused gate must report its own wire state, not the least-progressed live one"
    );
    assert!(
        payload.detail.contains("never saw your harness start"),
        "the refusal is still carried in words too: {:?}",
        payload.detail
    );
    let row = goalcup_daemon::status::summary(
        &e,
        GateState::Refused(RefusalReason::StartSignalMissing),
        HealthState::ArmingPending,
    );
    assert_eq!(row.arming, ArmingState::Refused);
}

/// What the daemon's own four timers lost is on the page, not only in the JSON.
///
/// `StatusPayload::cycles` is rendered: a participant whose ticks are failing must not read a
/// page that looks exactly like a healthy run. This crate has no logger, so the page is the
/// operator surface.
///
/// The assertion is against the field names the document actually serialises, so a rename in
/// `CycleErrors` fails here rather than leaving the page reading a key that is no longer
/// there — the page is a constant compiled into the binary and has no other way to be wrong.
#[cfg(feature = "v1-compat")]
#[test]
fn the_page_renders_the_cycle_error_count_and_the_last_message() {
    let e = support::started_entry();
    let mut inputs = support::status_inputs(&e);
    inputs.cycles = goalcup_protocol::control::CycleErrors {
        tick: 2,
        upload: 1,
        status: 0,
        deadline: 0,
        last: Some("io: broken pipe".to_string()),
        // When the loops last turned, beside what they lost. The page reads the four
        // failure counters and `last`; this rides on the
        // document and on `goalcup status`.
        last_pass_at: Some("2026-09-07T07:12:03Z".to_string()),
    };
    let payload = build_payload(&inputs);
    let doc = serde_json::to_value(&payload).expect("the payload serialises");
    assert_eq!(
        doc["cycles"]["last_pass_at"].as_str(),
        Some("2026-09-07T07:12:03Z"),
        "the stamp travels on the document"
    );
    for field in ["tick", "upload", "status", "deadline", "last"] {
        assert!(
            doc["cycles"].get(field).is_some(),
            "the document carries cycles.{field}"
        );
        assert!(
            STATUS_HTML.contains(&format!("c.{field}")),
            "and the page reads cycles.{field}"
        );
    }
    assert!(
        STATUS_HTML.contains("Tracker errors"),
        "the count is labelled in words a participant can act on"
    );
}

/// `tracker_restarted` is set once and never cleared, and `version_changed` is the
/// same shape. Both sit below the live signals: above them, one daemon restart would freeze the
/// health surface at `TrackerInterrupted` for the rest of the run — a server that then
/// vanished, an upload that fell behind, a harness that exited and a capture that stalled
/// would all be reported as a restart that had already been handled.
///
/// Two flags at a time: the latch, and a live
/// signal arriving after it.
#[test]
fn a_latched_restart_never_hides_a_live_signal() {
    for (name, live) in live_signals() {
        let mut restarted = live.clone();
        restarted.tracker_restarted = true;
        assert_eq!(
            state_of(&restarted),
            state_of(&live),
            "tracker_restarted masked {name}"
        );

        let mut updated = live.clone();
        updated.version_changed = true;
        assert_eq!(
            state_of(&updated),
            state_of(&live),
            "version_changed masked {name}"
        );
    }

    // And with nothing live to report, they are still the story.
    let mut only_restarted = support::inputs();
    only_restarted.tracker_restarted = true;
    assert_eq!(state_of(&only_restarted), HealthState::TrackerInterrupted);

    let mut only_updated = support::inputs();
    only_updated.version_changed = true;
    assert_eq!(state_of(&only_updated), HealthState::VersionChanged);

    // The two together: a build that changed under a daemon that restarted reports the
    // version, because the version is what the trace is about.
    let mut both = support::inputs();
    both.tracker_restarted = true;
    both.version_changed = true;
    assert_eq!(state_of(&both), HealthState::VersionChanged);
}

/// The four live signals, each on its own inputs.
fn live_signals() -> Vec<(&'static str, HealthInputs)> {
    let mut offline = support::inputs();
    offline.connected = false;

    let mut lagging = support::inputs();
    lagging.pending_events = 400;
    lagging.last_ack_at = Some(9_000.0);

    let mut exited = support::inputs();
    exited.harness_alive = false;
    exited.agent_exited = true;

    let mut stalled = support::inputs();
    stalled.last_leg1_at = Some(1_000.0);

    vec![
        ("ConnectionLost", offline),
        ("UploadLagging", lagging),
        ("AgentExited", exited),
        ("CaptureStalled", stalled),
    ]
}
