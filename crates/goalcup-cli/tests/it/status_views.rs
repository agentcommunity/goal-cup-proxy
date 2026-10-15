//! The two participant surfaces over one signed document: `goalcup status` and `goalcup
//! entries`. The third, `goalcup ui`, left with D48.
//!
//! The source sweeps use `support::walk("crates/goalcup-cli/src")` because `walk` walks
//! `<repo_root>/<rel>` and `<repo_root>/src` does not exist — a sweep over it would assert
//! nothing at all.

use crate::common::support;

use goalcup_cli::commands::{
    entries::entries_with,
    status::{render_status, status_with},
};
#[cfg(target_os = "macos")]
use goalcup_protocol::capture_status::AnchorState;
use goalcup_protocol::control::{self, DaemonClient, STALE_AFTER_SECONDS};
use goalcup_protocol::events::HealthState;
use goalcup_protocol::provenance::{
    PreCutoffDrainOutcomeV1, Rfc3339Utc, Sha256Digest, TerminationFailureV1, TerminationOutcomeV1,
};
use goalcup_protocol::submission::{SubmissionCapturedV1, SubmissionOutcomeV1};
use std::path::PathBuf;

/// A configuration directory holding one record for `info`'s run, with `entry_id` and
/// `status_url` written as given.
///
/// `support::cfg_with_entry` writes the record the daemon actually handed over, which is the
/// fixture almost every test here wants. This is the other one: a record that has been
/// EDITED — a cache a second process wrote, or a file a participant's own editor touched —
/// which is what the tests below are about. It writes through the same
/// `entries::write_entry` the real `goalcup start` uses, so what is on disk is the real
/// shape and only its contents are the test's.
fn cfg_with_edited_record(
    tmp: &tempfile::TempDir,
    info: &control::WorkspaceInfo,
    entry_id: &str,
    status_url: &str,
) -> PathBuf {
    let record = goalcup_cli::entries::EntryRecord {
        entry_id: entry_id.to_string(),
        process_generation: info.process_generation,
        connector_id: "claude-code-cli".to_string(),
        run_public_key: info.run_public_key.clone(),
        workspace: info.workspace.clone(),
        artifact_root: info.artifact_root.clone(),
        harness_home: info.harness_home.clone(),
        session_id: info.session_id.clone(),
        last_pid: None,
        status_url: status_url.to_string(),
        created_at: info.created_at.clone(),
        dev: false,
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
    assert!(text.contains("armed") || text.contains("started"));
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

/// The daemon has the field but not the number: `harness_version: Some("")`. It would render
/// as `  harness: ` with a dangling colon; the word `unknown` is an answer, a dangling colon
/// is a rendering fault a reader has to guess at.
#[test]
fn an_empty_harness_version_is_reported_as_unknown_rather_than_as_a_dangling_colon() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);

    m.set_harness_version(Some(""));
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
    assert!(
        text.contains("harness: unknown"),
        "an empty version is a version nobody measured: {text}"
    );
    assert!(
        !text.contains("harness: \n"),
        "and never a line that just stops: {text}"
    );

    // A version the daemon DOES carry is still printed as itself, so the word above is the
    // empty case and not a render that lost the number.
    m.set_harness_version(Some("2.1.259"));
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
    assert!(
        text.contains("harness: 2.1.259"),
        "the number the daemon answered: {text}"
    );

    // And a daemon that carries no field at all prints no line, which is the third answer
    // and not the same as either of the two above.
    m.set_harness_version(None);
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
    assert!(!text.contains("harness:"), "no field, no line: {text}");
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
    let cfg = cfg_with_edited_record(&tmp, &info, asked, &info.status_url);

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

/// The fourth arming state. "will never arm" must not read as "still waiting to arm", and the
/// reason is the daemon's own words.
#[test]
fn arming_refused_is_printed_as_refused_with_the_daemons_own_reason() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let why = "this harness never asked to start";
    m.set_arming_refused(why);
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
    assert!(
        text.contains(&format!("arming: refused — {why}")),
        "{text:?}"
    );
}

/// Two `#[serde(default)]` fields the page shows only in its JSON. This is the
/// only human surface for the daemon's own lost work, and for whether the round's dates were
/// configured or invented.
#[test]
fn status_prints_the_cycle_errors_and_a_fallback_event_source() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    m.set_cycle_error("upload", "connection refused");
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
    assert!(
        text.contains("connection refused"),
        "cycles.last is JSON-only on the page"
    );
    assert!(
        text.to_lowercase().contains("fallback") || text.contains("not configured"),
        "EventStatusSource::Fallback is the default: say the dates are the entry's own"
    );
}

/// An unverified ending can mean no signal was safe, or that best-effort numeric signals
/// were sent without an instance-bound proof. The participant-facing text must not turn
/// that union into the false assertion that nothing was signalled.
#[test]
fn status_describes_unverified_termination_without_inventing_a_no_signal_outcome() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    m.set_harness_unverified(true);
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

    assert!(
        text.contains("could not prove the managed process tree was fully and safely stopped"),
        "{text:?}"
    );
    assert!(
        !text.contains("nothing in the process tree was signalled"),
        "{text:?}"
    );
}

#[test]
fn cli_warns_on_v4_nonconfirmed_termination_even_without_the_legacy_harness_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let mut payload = m.status(&info.entry_id).unwrap().payload;
    payload.health = HealthState::Sealed;
    payload.sealed_at = Some("2026-09-14T00:00:03Z".to_string());
    payload.harness_unverified = false;
    payload.finalization = Some(control::WorkspaceFinalizationStatusV1::Terminal {
        pre_cutoff_drain: PreCutoffDrainOutcomeV1::Complete,
        termination: TerminationOutcomeV1::Unresolved {
            requested_at: Rfc3339Utc::try_from("2026-09-14T00:00:02Z".to_string()).unwrap(),
            stopped_waiting_at: Rfc3339Utc::try_from("2026-09-14T00:00:03Z".to_string()).unwrap(),
            reason: TerminationFailureV1::ProcessAuthorityUnavailable,
        },
        submission: SubmissionOutcomeV1::Captured(SubmissionCapturedV1 {
            manifest_sha256: Sha256Digest::from_bytes([0xcd; 32]),
        }),
    });
    let signed = goalcup_protocol::control::SignedStatus::sign(
        payload,
        support::NOW.to_string(),
        &goalcup_cli::daemon::MockDaemonClient::signing_key(),
    )
    .unwrap();

    let text = render_status(&signed, support::NOW);
    assert!(
        text.contains("the seal could not vouch for the harness"),
        "a V4 non-confirmed termination is visibly warned about: {text:?}"
    );
    assert!(
        text.contains("health: sealing_recovery_needed"),
        "it is not rendered as an ordinary sealed run: {text:?}"
    );
    assert!(!text.contains("health: sealed"), "{text:?}");
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
    entries_with(&m, false, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(
        text.lines().filter(|l| l.contains("entry-")).count(),
        m.entry_count()
    );
    let mut out = Vec::new();
    entries_with(&m, true, &mut out).unwrap();
    let v: Vec<goalcup_protocol::control::EntrySummary> = serde_json::from_slice(&out).unwrap();
    assert_eq!(v.len(), m.entry_count());

    // However the list treats a sealed entry, the command names the way to reach one.
    let before = m.entry_count();
    m.seal(&m.info().entry_id).unwrap();
    let mut out = Vec::new();
    entries_with(&m, false, &mut out).unwrap();
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
    m.set_arming_refused("\u{1b}[2K\rthe tracker says\nRecording. Leave this running.");

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
        "delivery local_only",
        "anchor local_only",
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

#[test]
fn a_legacy_status_carries_no_capture_fields() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let info = m.info();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    let mut out = Vec::new();
    status_with(&m, &cfg, Some(&info.entry_id), true, support::NOW, &mut out).unwrap();
    let raw: serde_json::Value = serde_json::from_slice(&out).unwrap();
    for field in ["capture", "producers_consumed", "admissions_refused"] {
        assert!(
            raw["payload"].get(field).is_none(),
            "a legacy document omits {field} byte-for-byte"
        );
    }
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
        // A placeholder no call reaches: a local capture asks no server anything.
        upload_url: "http://127.0.0.1:9".to_string(),
        server_public_key: support::TEST_SERVER_PUBLIC_KEY_HEX.to_string(),
        receipt_binding: goalcup_daemon::config::ReceiptBindingPolicy::Legacy,
        event: None,
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
                native_scope: None,
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

/// Codex has one required rollout source; supplemental history is excluded.
#[cfg(target_os = "macos")]
const CODEX_STORES: u64 = 1;

/// A v5 capture of a shipped profile through the real daemon and this CLI's own client: the
/// status view verifies against the key the create call returned, and the v5 reader finds the
/// goal, launch and baseline bound. The absent native scope chains one required-source gap,
/// so local finish seals a trace that reads incomplete (exit 1), never valid.
#[cfg(target_os = "macos")]
#[test]
fn a_shipped_profile_capture_reads_incomplete_while_its_stores_go_unread() {
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
        status_url: String::new(),
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
    assert!(text.contains("capture: incomplete — run running"), "{text}");
    assert!(text.contains(&format!("gaps: {CODEX_STORES};")), "{text}");
    assert!(
        text.contains("0 producer(s) consumed, 0 refused admission"),
        "{text}"
    );

    let report = read_capture(&config, &info.entry_id);
    assert!(!report.torn_tail);
    assert!(report.coverage_incomplete, "{:?}", report.findings);
    assert!(!report.sealed);
    assert_eq!(report.anchor, AnchorState::LocalOnly);
    assert_eq!(report.gap_count, CODEX_STORES);
    assert_eq!(
        report.findings,
        vec!["chain_unsealed".to_string(), "gap_recorded".to_string()],
        "every producer closed, the launch bound, the baseline bound: only the seal and the \
         unread stores are missing"
    );

    // The local finish, with no server anywhere: sealed, and still incomplete.
    let finished = client
        .seal_capture(
            &info.entry_id,
            &control::CaptureSealRequest {
                proxy: Some(control::ProxyCounters::default()),
                remaining_finish_ms: Some(10_000),
                native_scope: None,
            },
            std::time::Duration::from_secs(30),
        )
        .unwrap();
    assert_eq!(finished.seal, control::LocalSealOutcome::SealWritten);
    let report = read_capture(&config, &info.entry_id);
    runtime.block_on(daemon.stop());
    assert!(report.sealed, "{:?}", report.findings);
    assert_eq!(report.findings, vec!["gap_recorded".to_string()]);
    assert_eq!(report.exit_code(), 1);
}

/// The daemon goes away under a launched run with no producer attached, which is what a
/// crash leaves behind: nothing is chained as it goes. The reopened entry admits nothing, so
/// the chain records that loss beside the launch's required-source gap, and the sealed trace
/// never reads as complete.
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
                native_scope: None,
            },
            std::time::Duration::from_secs(30),
        )
        .unwrap();
    assert_eq!(finished.seal, control::LocalSealOutcome::SealWritten);
    let report = read_capture(&config, &info.entry_id);
    runtime.block_on(daemon.stop());
    assert!(report.sealed, "{:?}", report.findings);
    assert_eq!(report.gap_count, CODEX_STORES + 1, "{:?}", report.findings);
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
    write(
        cfg,
        &RunRecord {
            run_id: entry_id.to_string(),
            server_url: server.map(str::to_string),
            practice: false,
            connector_id: "claude-code-cli".to_string(),
            workspace: PathBuf::from("/w"),
            started_at_unix: 1,
            deadline_at: deadline_at.map(str::to_string),
            entries: vec![EntryRef {
                entry_id: entry_id.to_string(),
                trace_dir: PathBuf::from("/t"),
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
    };
    state.disk = DiskState {
        free_bytes: 3 << 30,
        reserve_bytes: 1 << 30,
        low: true,
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
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        &lines[..5],
        [
            "PROBLEM: recording has gaps",
            "PROBLEM: last upload 40 min ago: the server answered 503",
            "PROBLEM: disk: 3 GiB free beyond the reservation",
            "PROBLEM: the machine may sleep: no sleep inhibitor on this machine",
            "PROBLEM: the agent's process exited (exit code 1): run goalcup continue",
        ]
    );
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("PROBLEM:")).count(),
        5,
        "{text}"
    );
    assert!(
        text.contains(&m.status_detail()),
        "the status itself follows the problems: {text}"
    );

    // A failed recording; and an upload that has not lasted half an hour is not yet a problem,
    // unless it gave up.
    state.recording = Recording::Failed;
    state.upload.last_ok_at = Some(now - 60);
    state.sleep_lock = Ok(());
    state.disk.low = false;
    state.harness = HarnessState::Running;
    run_with_state(&cfg, &info.entry_id, state.clone());
    let text = status_text(&m, &cfg, &info.entry_id, false);
    assert!(text.starts_with("PROBLEM: recording failed\n"), "{text}");
    assert_eq!(text.matches("PROBLEM:").count(), 1, "{text}");
    state.upload.stopped = true;
    run_with_state(&cfg, &info.entry_id, state);
    let text = status_text(&m, &cfg, &info.entry_id, false);
    assert!(
        text.contains("PROBLEM: last upload 1 min ago: the server answered 503\n"),
        "{text}"
    );

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
    let exited = |why: &str| format!("PROBLEM: the agent's process exited (exit code 1){why}\n");
    let said = |cfg: &std::path::Path| status_text(&m, cfg, &info.entry_id, false);

    // A headless connector whose agent finished normally (status 0): a plain line, no PROBLEM.
    let mut finished = state.clone();
    finished.harness = HarnessState::Exited {
        status: "exit code 0".to_string(),
    };
    run_with_state_on(&cfg, &info.entry_id, finished.clone(), server, ahead);
    let text = said(&cfg);
    assert!(
        text.starts_with("the agent finished; this run cannot be continued\n"),
        "{text}"
    );
    assert!(!text.contains("PROBLEM"), "{text}");
    // The same normal end of a local run.
    run_with_state_on(&cfg, &info.entry_id, finished.clone(), None, None);
    let text = said(&cfg);
    assert!(
        text.starts_with("the agent finished; this run cannot be continued\n"),
        "{text}"
    );
    assert!(!text.contains("PROBLEM"), "{text}");
    // A chat-mode run `continue` can restart is still told so, as a problem.
    finished.continuable = true;
    run_with_state_on(&cfg, &info.entry_id, finished, server, ahead);
    let text = said(&cfg);
    assert!(
        text.starts_with(
            "PROBLEM: the agent's process exited (exit code 0): run goalcup continue\n"
        ),
        "{text}"
    );

    // A headless connector, run open on the server: not continuable.
    state.continuable = false;
    run_with_state_on(&cfg, &info.entry_id, state.clone(), server, ahead);
    let text = said(&cfg);
    assert!(
        text.starts_with(&exited("; this run cannot be continued")),
        "{text}"
    );
    assert!(!text.contains("run goalcup continue"), "{text}");

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
    assert!(text.starts_with(&exited("; the run is over")), "{text}");

    // A local run (no server) is not continuable either.
    run_with_state_on(&cfg, &info.entry_id, state.clone(), None, None);
    let text = said(&cfg);
    assert!(
        text.starts_with(&exited("; this run cannot be continued")),
        "{text}"
    );

    // And one `continue` can restart still says to run it.
    run_with_state_on(&cfg, &info.entry_id, state, server, ahead);
    let text = said(&cfg);
    assert!(
        text.starts_with(&exited(": run goalcup continue")),
        "{text}"
    );
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
    };
    run_with_state(&cfg, &info.entry_id, state.clone());
    let text = status_text(&m, &cfg, &info.entry_id, false);
    assert!(!text.contains("PROBLEM"), "{text}");

    state.heartbeat.failures = HEARTBEATS_FAILING_AFTER;
    run_with_state(&cfg, &info.entry_id, state.clone());
    let text = status_text(&m, &cfg, &info.entry_id, false);
    assert!(
        text.starts_with(
            "PROBLEM: the last 3 heartbeats to the server failed: the server answered HTTP 503\n"
        ),
        "{text}"
    );

    // A run that is over has no heartbeats to worry about.
    state.ended = true;
    run_with_state(&cfg, &info.entry_id, state);
    let text = status_text(&m, &cfg, &info.entry_id, false);
    assert!(!text.contains("heartbeats"), "{text}");
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
    assert!(text.contains("no run state yet"), "{text}");

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
