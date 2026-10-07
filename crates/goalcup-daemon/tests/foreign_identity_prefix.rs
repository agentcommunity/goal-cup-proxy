//! The shipped Claude transcript begins with bookkeeping that names the session but not its
//! working directory. These tests exercise the bounded, manifest-selected reader through the
//! trusted plan: committed redacted evidence first, then the rejection and bound edges that
//! a foreign transcript can control.
#![cfg(feature = "v1-compat")]

mod support;

use goalcup_daemon::MAX_LINE_BYTES;
use goalcup_daemon::foreign::{ForeignScanResult, TrustedManifests, scan_once};
use goalcup_daemon::paths::{PathVars, claude_slug};
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::events::{ForeignScanOutcome, ForeignScanUnavailableReason};
use goalcup_protocol::manifest::ConnectorManifest;
use std::io::Write;
use std::path::{Path, PathBuf};

const SYNTHETIC_WORKSPACE: &str = "/goal-cup/foreign-prefix-tracked";

fn connectors() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors")
}

fn claude() -> ConnectorManifest {
    let text = std::fs::read_to_string(connectors().join("claude-code-cli/manifest.toml"))
        .expect("the shipped Claude manifest");
    ConnectorManifest::from_toml_str(&text).expect("the shipped Claude manifest parses")
}

fn coverage(result: &ForeignScanResult, scan_index: u64) -> ForeignScanOutcome {
    result
        .coverage
        .iter()
        .find(|coverage| {
            coverage.declaring_connector == "claude-code-cli" && coverage.scan_index == scan_index
        })
        .unwrap_or_else(|| panic!("missing Claude foreign_scan[{scan_index}] coverage"))
        .outcome
        .clone()
}

fn scan_config_dir(root: &Path, config_dir: PathBuf, workspace: PathBuf) -> ForeignScanResult {
    let vars = PathVars::new(
        workspace.clone(),
        root.join("home"),
        config_dir,
        root.join("artifacts"),
        None,
        ("2026".into(), "09".into(), "10".into()),
    );
    let manifest = claude();
    let plan = TrustedManifests::load(&connectors())
        .expect("the shipped manifests are trusted")
        .plan_for(&manifest);
    scan_once(&plan, &vars, std::slice::from_ref(&workspace))
        .expect("a bounded scan reports an outcome")
}

fn scan_synthetic(bytes: &[u8]) -> ForeignScanResult {
    let root = tempfile::tempdir().expect("a temporary isolated home");
    let config_dir = root.path().join("config");
    let transcript = config_dir.join("projects/source/session.jsonl");
    std::fs::create_dir_all(transcript.parent().expect("the transcript parent")).unwrap();
    std::fs::write(transcript, bytes).expect("the synthetic transcript writes");
    scan_config_dir(root.path(), config_dir, PathBuf::from(SYNTHETIC_WORKSPACE))
}

fn joint_identity(session: &str, directory: &str) -> String {
    serde_json::json!({ "sessionId": session, "cwd": directory }).to_string()
}

fn sized_joint_identity(total_bytes: usize) -> Vec<u8> {
    let base = serde_json::json!({
        "sessionId": "boundary-session",
        "cwd": SYNTHETIC_WORKSPACE,
        "padding": "",
    })
    .to_string();
    assert!(base.len() < total_bytes, "the test can add padding");
    let record = serde_json::json!({
        "sessionId": "boundary-session",
        "cwd": SYNTHETIC_WORKSPACE,
        "padding": "x".repeat(total_bytes - base.len()),
    })
    .to_string();
    assert_eq!(record.len(), total_bytes, "the boundary is literal bytes");
    record.into_bytes()
}

fn sized_joint_identity_for(total_bytes: usize, session: &str, directory: &str) -> Vec<u8> {
    let base = serde_json::json!({
        "sessionId": session,
        "cwd": directory,
        "padding": "",
    })
    .to_string();
    assert!(base.len() < total_bytes, "the test can add padding");
    let record = serde_json::json!({
        "sessionId": session,
        "cwd": directory,
        "padding": "x".repeat(total_bytes - base.len()),
    })
    .to_string();
    assert_eq!(record.len(), total_bytes, "the boundary is literal bytes");
    record.into_bytes()
}

fn claude_home_outcomes(chain: &[Envelope]) -> Vec<ForeignScanOutcome> {
    chain
        .iter()
        .filter_map(|envelope| match &envelope.body {
            EventBody::ForeignScanCoverage {
                declaring_connector,
                scan_index: 0,
                outcome,
                ..
            } if declaring_connector == "claude-code-cli" => Some(outcome.clone()),
            _ => None,
        })
        .collect()
}

fn foreign_transcript(h: &support::EntryHarness, directory: &str) -> PathBuf {
    h.home
        .join(".claude/projects")
        .join(directory)
        .join("session.jsonl")
}

fn write_foreign_transcript(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("the foreign transcript parent")).unwrap();
    std::fs::write(path, bytes).expect("the synthetic Claude transcript writes");
}

fn mode_only_record(session: &str) -> String {
    serde_json::json!({
        "type": "mode",
        "sessionId": session,
        "mode": "default",
    })
    .to_string()
}

#[test]
fn committed_main_and_a9_prefixes_read_only_their_first_joint_identities() {
    let root = tempfile::tempdir().expect("a temporary isolated home");
    let config_dir = root.path().join("config");
    let tracked = PathBuf::from("/Users/team/goalcup-spikes/ws-claude-code-cli-20260903T073054Z");
    let projects = config_dir.join("projects");
    let main = projects.join(claude_slug(&tracked)).join("main.jsonl");
    let a9 = projects
        .join("-Users-team-goalcup-spikes-a9-other")
        .join("a9.jsonl");
    std::fs::create_dir_all(main.parent().expect("the main transcript parent")).unwrap();
    std::fs::create_dir_all(a9.parent().expect("the A9 transcript parent")).unwrap();
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spikes/claude-code-cli/fixtures");
    std::fs::copy(fixtures.join("main-session.jsonl"), &main)
        .expect("the committed redacted main fixture copies");
    std::fs::copy(fixtures.join("a9-other-folder.jsonl"), &a9)
        .expect("the committed redacted A9 fixture copies");

    let result = scan_config_dir(root.path(), config_dir, tracked.clone());

    assert_eq!(coverage(&result, 1), ForeignScanOutcome::Complete);
    assert_eq!(
        result
            .sightings
            .iter()
            .map(|sighting| (sighting.session_id.as_str(), sighting.directory.as_str()))
            .collect::<Vec<_>>(),
        vec![(
            "95addb64-9dac-4046-8ef6-8aa53fa0c7f1",
            tracked.to_str().expect("the known tracked path is UTF-8")
        )],
        "the A9 transcript is read for its own identity but does not become a sighting in a different folder"
    );
}

#[test]
fn preceding_partial_identity_must_match_a_later_single_record_pair() {
    let agree = format!(
        "{{\"sessionId\":\"same\"}}\n{}\n",
        joint_identity("same", SYNTHETIC_WORKSPACE)
    );
    let result = scan_synthetic(agree.as_bytes());
    assert_eq!(coverage(&result, 1), ForeignScanOutcome::Complete);
    assert_eq!(result.sightings.len(), 1);
    assert_eq!(result.sightings[0].session_id, "same");

    for records in [
        format!(
            "{{\"sessionId\":\"first\"}}\n{}\n",
            joint_identity("second", SYNTHETIC_WORKSPACE)
        ),
        format!(
            "{{\"cwd\":\"/a-different-directory\"}}\n{}\n",
            joint_identity("same", SYNTHETIC_WORKSPACE)
        ),
        format!("{{\"sessionId\":\"same\"}}\n{{\"cwd\":\"{SYNTHETIC_WORKSPACE}\"}}\n"),
    ] {
        assert_eq!(
            coverage(&scan_synthetic(records.as_bytes()), 1),
            ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Identity),
            "a pair cannot be invented from separate records or from disagreeing observations"
        );
    }
}

#[test]
fn present_invalid_identity_and_bad_json_refuse_the_prefix() {
    for bytes in [
        format!("{{\"sessionId\":null,\"cwd\":\"{SYNTHETIC_WORKSPACE}\"}}\n").into_bytes(),
        format!("{{\"sessionId\":9,\"cwd\":\"{SYNTHETIC_WORKSPACE}\"}}\n").into_bytes(),
        format!("{{\"sessionId\":\"\",\"cwd\":\"{SYNTHETIC_WORKSPACE}\"}}\n").into_bytes(),
        b"{\"sessionId\":\"s\",\"cwd\":null}\n".to_vec(),
        b"{\"sessionId\":\"s\",\"cwd\":9}\n".to_vec(),
        b"{\"sessionId\":\"s\",\"cwd\":\"\"}\n".to_vec(),
        b"{\"sessionId\":\"only-one-field\"}\n".to_vec(),
        format!("{{\"cwd\":\"{SYNTHETIC_WORKSPACE}\"}}\n").into_bytes(),
    ] {
        assert_eq!(
            coverage(&scan_synthetic(&bytes), 1),
            ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Identity)
        );
    }
    assert_eq!(
        coverage(&scan_synthetic(b"{\"sessionId\":\"unfinished\""), 1),
        ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Row),
        "an incomplete final JSON line is a malformed row, not a missing identity"
    );
}

#[test]
fn valid_final_line_and_the_sixty_fourth_record_are_accepted() {
    let final_line = joint_identity("no-newline", SYNTHETIC_WORKSPACE);
    assert_eq!(
        coverage(&scan_synthetic(final_line.as_bytes()), 1),
        ForeignScanOutcome::Complete
    );

    let mut at_record_limit = "{\"type\":\"bookkeeping\"}\n".repeat(63);
    at_record_limit.push_str(&joint_identity("record-64", SYNTHETIC_WORKSPACE));
    at_record_limit.push('\n');
    assert_eq!(
        coverage(&scan_synthetic(at_record_limit.as_bytes()), 1),
        ForeignScanOutcome::Complete
    );

    let mut one_record_beyond = "{\"type\":\"bookkeeping\"}\n".repeat(64);
    one_record_beyond.push_str(&joint_identity("record-65", SYNTHETIC_WORKSPACE));
    assert_eq!(
        coverage(&scan_synthetic(one_record_beyond.as_bytes()), 1),
        ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::RowLimit)
    );
}

#[test]
fn one_mebibyte_prefix_boundary_accepts_a_complete_record_and_stops_before_suffix() {
    let cap = MAX_LINE_BYTES as usize;
    let mut exact = sized_joint_identity(cap - 1);
    exact.push(b'\n');
    assert_eq!(exact.len(), cap, "the newline is part of the input prefix");
    exact.extend_from_slice(b"{this suffix is intentionally not JSON}\n");
    assert_eq!(
        coverage(&scan_synthetic(&exact), 1),
        ForeignScanOutcome::Complete
    );

    let over_cap = sized_joint_identity(cap + 1);
    assert_eq!(
        over_cap.len(),
        cap + 1,
        "the candidate itself is one byte over the budget"
    );
    assert_eq!(
        coverage(&scan_synthetic(&over_cap), 1),
        ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::RecordLimit)
    );
}

#[test]
fn exact_one_mebibyte_final_identity_line_without_newline_is_complete() {
    let final_line = sized_joint_identity(MAX_LINE_BYTES as usize);
    assert_eq!(
        final_line.len(),
        MAX_LINE_BYTES as usize,
        "the complete final record occupies the whole prefix budget"
    );
    assert_eq!(
        coverage(&scan_synthetic(&final_line), 1),
        ForeignScanOutcome::Complete
    );
}

#[cfg(unix)]
#[test]
fn identity_reader_uses_the_existing_regular_source_guard_for_symlinks_and_fifos() {
    use std::os::unix::ffi::OsStrExt;

    let root = tempfile::tempdir().expect("a temporary isolated home");
    let config_dir = root.path().join("config");
    let source = config_dir.join("projects/source/session.jsonl");
    std::fs::create_dir_all(source.parent().expect("the transcript parent")).unwrap();
    let target = root.path().join("target.jsonl");
    std::fs::write(&target, joint_identity("linked", SYNTHETIC_WORKSPACE)).unwrap();
    std::os::unix::fs::symlink(&target, &source).expect("a symlinked transcript");
    assert_eq!(
        coverage(
            &scan_config_dir(
                root.path(),
                config_dir.clone(),
                PathBuf::from(SYNTHETIC_WORKSPACE),
            ),
            1
        ),
        ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Open)
    );

    std::fs::remove_file(&source).expect("the symlink is removed");
    let fifo = std::ffi::CString::new(source.as_os_str().as_bytes()).expect("the source path");
    assert_eq!(
        unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) },
        0,
        "a FIFO is created"
    );
    assert_eq!(
        coverage(
            &scan_config_dir(root.path(), config_dir, PathBuf::from(SYNTHETIC_WORKSPACE)),
            1
        ),
        ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Open)
    );
}

#[tokio::test]
async fn runtime_chains_a_claude_prefix_sighting_and_its_coverage_transition() {
    let h = support::one_armed_entry().await;
    support::start_entry(&h).await;
    let foreign = h.home.join(".claude/projects/foreign/session.jsonl");
    std::fs::create_dir_all(foreign.parent().expect("the foreign transcript parent")).unwrap();
    let candidate = serde_json::json!({
        "sessionId": "claude-foreign",
        "cwd": h.workspace.display().to_string(),
        "private": "foreign transcript body",
    })
    .to_string();
    std::fs::write(
        &foreign,
        format!("{{\"sessionId\":\"claude-foreign\"}}\n{candidate}\n"),
    )
    .expect("the foreign Claude prefix writes");

    support::advance_clock(&h, 61.0);
    h.poll();
    let chain = support::read_chain(&h.trace_dir);
    assert!(chain.iter().any(|envelope| {
        matches!(
            &envelope.body,
            EventBody::ForeignSession { harness, session_id, .. }
                if harness == "claude-code-cli" && session_id == "claude-foreign"
        )
    }));
    let outcomes = chain
        .iter()
        .filter_map(|envelope| match &envelope.body {
            EventBody::ForeignScanCoverage {
                declaring_connector,
                scan_index: 0,
                outcome,
                ..
            } if declaring_connector == "claude-code-cli" => Some(outcome.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes,
        vec![ForeignScanOutcome::Absent, ForeignScanOutcome::Complete],
        "the unavailable-to-complete transition is trace evidence, not an in-memory status"
    );
    assert!(
        !serde_json::to_string(&chain)
            .expect("the chain serializes")
            .contains("foreign transcript body"),
        "the chain carries the sighting identity and outcome, never transcript content"
    );
}

#[tokio::test]
async fn runtime_defers_a_mode_only_claude_prefix_until_its_joint_identity_arrives() {
    let h = support::one_armed_entry().await;
    support::start_entry(&h).await;
    let foreign = foreign_transcript(&h, "live-prefix");
    let mode_only = mode_only_record("claude-live-prefix");
    write_foreign_transcript(&foreign, format!("{mode_only}\n").as_bytes());

    // The fake Claude writer is frozen here. The heartbeat sees only a valid, complete
    // mode record that has a session id but no directory.
    support::advance_clock(&h, 61.0);
    h.poll();
    assert_eq!(
        claude_home_outcomes(&support::read_chain(&h.trace_dir)),
        vec![ForeignScanOutcome::Absent],
        "a live, mode-only prefix is not irreversible unavailable coverage"
    );

    let joint_identity = serde_json::json!({
        "sessionId": "claude-live-prefix",
        "cwd": h.workspace.display().to_string(),
        "private": "foreign transcript body",
    })
    .to_string();
    let mut writer = std::fs::OpenOptions::new()
        .append(true)
        .open(&foreign)
        .expect("the frozen source reopens for the joint record");
    writeln!(writer, "{joint_identity}").expect("the joint identity appends");

    support::advance_clock(&h, 61.0);
    h.poll();
    let chain = support::read_chain(&h.trace_dir);
    assert_eq!(
        claude_home_outcomes(&chain),
        vec![ForeignScanOutcome::Absent, ForeignScanOutcome::Complete],
        "the next live sweep settles the prefix and chains only complete coverage"
    );
    assert!(chain.iter().any(|envelope| {
        matches!(
            &envelope.body,
            EventBody::ForeignSession { harness, session_id, .. }
                if harness == "claude-code-cli" && session_id == "claude-live-prefix"
        )
    }));
    assert!(
        !serde_json::to_string(&chain)
            .expect("the chain serializes")
            .contains("foreign transcript body"),
        "settling keeps transcript content out of the chain"
    );
}

#[tokio::test]
async fn runtime_rebuild_finalizes_a_deferred_mode_only_prefix_without_a_second_grace() {
    let h = support::one_armed_entry().await;
    support::start_entry(&h).await;
    write_foreign_transcript(
        &foreign_transcript(&h, "rebuilt-prefix"),
        format!("{}\n", mode_only_record("claude-rebuilt-prefix")).as_bytes(),
    );

    support::advance_clock(&h, 61.0);
    h.poll();
    assert_eq!(
        claude_home_outcomes(&support::read_chain(&h.trace_dir)),
        vec![ForeignScanOutcome::Absent],
        "the fresh runtime defers its first eligible live prefix"
    );

    let mut rebuilt = support::reopen_runtime(&h).await;
    rebuilt.open_trace().expect("the trace resumes");
    rebuilt
        .poll_once_at(h.now())
        .expect("the rebuilt runtime sweeps its immediate heartbeat");
    h.give_back(rebuilt);

    assert_eq!(
        claude_home_outcomes(&support::read_chain(&h.trace_dir)),
        vec![
            ForeignScanOutcome::Absent,
            ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Identity),
        ],
        "a restarted runtime does not recreate the live-prefix grace"
    );
}

#[tokio::test]
async fn runtime_emits_identity_unavailable_on_the_second_unchanged_mode_only_sweep() {
    let h = support::one_armed_entry().await;
    support::start_entry(&h).await;
    let foreign = foreign_transcript(&h, "stable-prefix");
    write_foreign_transcript(
        &foreign,
        format!("{}\n", mode_only_record("claude-stable-prefix")).as_bytes(),
    );

    support::advance_clock(&h, 61.0);
    h.poll();
    assert_eq!(
        claude_home_outcomes(&support::read_chain(&h.trace_dir)),
        vec![ForeignScanOutcome::Absent],
        "the first eligible live sweep is held pending"
    );

    support::advance_clock(&h, 61.0);
    h.poll();
    assert_eq!(
        claude_home_outcomes(&support::read_chain(&h.trace_dir)),
        vec![
            ForeignScanOutcome::Absent,
            ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Identity),
        ],
        "the unchanged second eligible sweep becomes fail-closed evidence"
    );
}

#[tokio::test]
async fn runtime_does_not_restart_mode_only_grace_when_the_source_bytes_change() {
    let h = support::one_armed_entry().await;
    support::start_entry(&h).await;
    let foreign = foreign_transcript(&h, "changed-prefix");
    write_foreign_transcript(
        &foreign,
        format!("{}\n", mode_only_record("claude-changed-prefix")).as_bytes(),
    );

    support::advance_clock(&h, 61.0);
    h.poll();
    assert_eq!(
        claude_home_outcomes(&support::read_chain(&h.trace_dir)),
        vec![ForeignScanOutcome::Absent],
        "the first eligible live sweep is held pending"
    );

    let mut writer = std::fs::OpenOptions::new()
        .append(true)
        .open(&foreign)
        .expect("the source reopens for another mode record");
    writeln!(writer, "{}", mode_only_record("claude-changed-prefix"))
        .expect("the additional mode record appends");

    support::advance_clock(&h, 61.0);
    h.poll();
    assert_eq!(
        claude_home_outcomes(&support::read_chain(&h.trace_dir)),
        vec![
            ForeignScanOutcome::Absent,
            ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Identity),
        ],
        "a changed but still eligible source does not gain a second grace sweep"
    );
}

#[tokio::test]
async fn runtime_seal_emits_identity_unavailable_after_the_first_mode_only_deferral() {
    let h = support::one_armed_entry().await;
    support::start_entry(&h).await;
    let foreign = foreign_transcript(&h, "seal-prefix");
    write_foreign_transcript(
        &foreign,
        format!("{}\n", mode_only_record("claude-seal-prefix")).as_bytes(),
    );

    support::advance_clock(&h, 61.0);
    h.poll();
    assert_eq!(
        claude_home_outcomes(&support::read_chain(&h.trace_dir)),
        vec![ForeignScanOutcome::Absent],
        "the live sweep held the eligible source pending"
    );

    h.with(|runtime| runtime.seal())
        .expect("the run seals after finalizing foreign coverage");
    assert_eq!(
        claude_home_outcomes(&support::read_chain(&h.trace_dir)),
        vec![
            ForeignScanOutcome::Absent,
            ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Identity),
        ],
        "sealing never leaves a clean EOF identity absence pending"
    );
}

#[tokio::test]
async fn runtime_emits_malformed_conflicting_and_capped_prefixes_without_grace() {
    let malformed = support::one_armed_entry().await;
    support::start_entry(&malformed).await;
    write_foreign_transcript(
        &foreign_transcript(&malformed, "malformed"),
        b"{not json}\n",
    );
    support::advance_clock(&malformed, 61.0);
    malformed.poll();
    assert_eq!(
        claude_home_outcomes(&support::read_chain(&malformed.trace_dir)),
        vec![
            ForeignScanOutcome::Absent,
            ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Row),
        ],
        "a malformed complete row is not a live-prefix candidate"
    );

    let conflicting = support::one_armed_entry().await;
    support::start_entry(&conflicting).await;
    let conflict = format!(
        "{{\"sessionId\":\"first\"}}\n{}\n",
        joint_identity("second", &conflicting.workspace.display().to_string())
    );
    write_foreign_transcript(
        &foreign_transcript(&conflicting, "conflicting"),
        conflict.as_bytes(),
    );
    support::advance_clock(&conflicting, 61.0);
    conflicting.poll();
    assert_eq!(
        claude_home_outcomes(&support::read_chain(&conflicting.trace_dir)),
        vec![
            ForeignScanOutcome::Absent,
            ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Identity),
        ],
        "conflicting observed identities are not a live-prefix candidate"
    );

    let capped = support::one_armed_entry().await;
    support::start_entry(&capped).await;
    let capped_bytes = sized_joint_identity_for(
        MAX_LINE_BYTES as usize + 1,
        "claude-capped-prefix",
        &capped.workspace.display().to_string(),
    );
    write_foreign_transcript(&foreign_transcript(&capped, "capped"), &capped_bytes);
    support::advance_clock(&capped, 61.0);
    capped.poll();
    assert_eq!(
        claude_home_outcomes(&support::read_chain(&capped.trace_dir)),
        vec![
            ForeignScanOutcome::Absent,
            ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::RecordLimit),
        ],
        "a bounded source is never held pending"
    );
}

#[tokio::test]
async fn runtime_emits_a_hard_failure_when_another_matched_source_is_eligible() {
    let h = support::one_armed_entry().await;
    support::start_entry(&h).await;
    write_foreign_transcript(
        &foreign_transcript(&h, "a-eligible"),
        format!("{}\n", mode_only_record("claude-eligible-path")).as_bytes(),
    );
    write_foreign_transcript(&foreign_transcript(&h, "z-malformed"), b"{not json}\n");

    support::advance_clock(&h, 61.0);
    h.poll();
    assert_eq!(
        claude_home_outcomes(&support::read_chain(&h.trace_dir)),
        vec![
            ForeignScanOutcome::Absent,
            ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Row),
        ],
        "an eligible source cannot hide a hard failure in another matched source"
    );
}
