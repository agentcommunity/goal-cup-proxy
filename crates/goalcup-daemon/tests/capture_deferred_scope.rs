//! A native scope the harness only knows at the end of its turn, carried on
//! the finish request, through the real daemon and its control API.
//!
//! Hermes prints its own session identity on stderr after the turn, so a Hermes entry's Launch
//! carries no scope. The launcher reads the identity after the harness exits and hands it to the
//! finish, which installs the entry's one native source before it stops the owner (CONTRACTS.md,
//! "Amendment 2026-10-02 (C6 local finish budget, H-10 4a)" ¶9). Every other profile is scoped at
//! launch and ignores a scope on the finish.
//!
//! macOS only: every entry is reserved on the temporary directory's APFS volume. Every file here
//! lives in the rig's temporary root; nothing resolves a real `HOME` or any `.hermes`.

#![cfg(target_os = "macos")]

mod capture_lifecycle_support;
mod support;
#[path = "support/table_lock.rs"]
mod table_lock;

use base64::Engine as _;
use capture_lifecycle_support::{Rig, launch_request, local_finishes, streamless};
use goalcup_protocol::capture::{CaptureRecord, CaptureSource, CaptureState, GapReason};
use goalcup_protocol::control::{CaptureSealRequest, CaptureWorkspaceInfo};
use std::time::Duration;
use support::control::Response;

const ID: &str = "20260930_141500_a1b2c3";

/// A synthetic session snapshot. No key, no credential: only the shape of the file.
fn snapshot() -> Vec<u8> {
    format!(
        r#"{{"session_id":"{ID}","model":"deepseek-v4-flash","platform":"cli","message_count":2,"messages":[{{"role":"user","content":"hi"}},{{"role":"assistant","content":"hello"}}]}}"#
    )
    .into_bytes()
}

fn write_snapshot(rig: &Rig, id: &str) -> Vec<u8> {
    let dir = rig.root.join("config-home/sessions");
    std::fs::create_dir_all(&dir).unwrap();
    let bytes = snapshot();
    std::fs::write(dir.join(format!("session_{id}.json")), &bytes).unwrap();
    bytes
}

async fn copied_connector_rig() -> (Rig, tempfile::TempDir) {
    let connectors = tempfile::tempdir().unwrap();
    for id in ["codex-cli", "claude-code-cli", "hermes-cli"] {
        support::copy_tree(
            &support::shipped_connector_dir(id),
            &connectors.path().join(id),
        );
    }
    let root = connectors.path().to_path_buf();
    let rig = Rig::start_with(move |config| config.connectors_dir = root).await;
    (rig, connectors)
}

/// A Hermes entry whose Launch carried no scope: the harness has not printed its identity.
async fn hermes_launched(rig: &Rig) -> CaptureWorkspaceInfo {
    let info = rig.created("hermes-cli").await;
    let goal = rig.goal_accepted(&info.entry_id, b"say hi").await;
    let mut request = launch_request(&goal);
    request.native_scope = None;
    let launched = rig.launch(&info.entry_id, &request).await;
    assert_eq!(launched.status(), 204, "{}", launched.body());
    info
}

async fn seal(
    rig: &Rig,
    entry_id: &str,
    scope: Option<&str>,
    remaining_ms: Option<u64>,
) -> Response {
    rig.seal_with(
        entry_id,
        &CaptureSealRequest {
            proxy: None,
            remaining_finish_ms: remaining_ms,
            native_scope: scope.map(str::to_string),
        },
    )
    .await
}

/// The native streams opened, each with its label.
fn native_opens(records: &[CaptureRecord]) -> Vec<(goalcup_protocol::capture::StreamKey, String)> {
    records
        .iter()
        .filter_map(|r| match r {
            CaptureRecord::Open {
                stream,
                source: CaptureSource::Native,
                metadata,
                ..
            } => Some((stream.clone(), metadata.label.clone())),
            _ => None,
        })
        .collect()
}

fn retained(records: &[CaptureRecord], of: &goalcup_protocol::capture::StreamKey) -> Vec<u8> {
    records
        .iter()
        .filter_map(|r| match r {
            CaptureRecord::Segment {
                stream, data_b64, ..
            } if stream == of => Some(
                base64::engine::general_purpose::STANDARD
                    .decode(data_b64)
                    .unwrap(),
            ),
            _ => None,
        })
        .flatten()
        .collect()
}

/// A Hermes launch with no scope records no gap; the run is still Recording.
#[tokio::test]
async fn a_deferred_launch_appends_no_gap() {
    let rig = Rig::start().await;
    let info = hermes_launched(&rig).await;
    let records = rig.records(&info.entry_id);
    assert_eq!(
        streamless(&records, GapReason::SourceChanged),
        0,
        "{records:#?}"
    );
    let capture = rig.status(&info.entry_id).await.payload.capture.unwrap();
    assert_eq!(capture.gap_count, 0);
    assert_eq!(capture.capture, CaptureState::Recording);

    // A Codex launch with no scope still does: its scope was due at the launch.
    let codex = rig.created("codex-cli").await;
    let goal = rig.goal_accepted(&codex.entry_id, b"say hi").await;
    let mut request = launch_request(&goal);
    request.native_scope = None;
    assert_eq!(rig.launch(&codex.entry_id, &request).await.status(), 204);
    assert_eq!(
        streamless(&rig.records(&codex.entry_id), GapReason::SourceChanged),
        1
    );
}

/// A finish with the run's own identity installs the source before the owner stops, so
/// the snapshot is read within the finish: one native stream, its bytes exact, no gap. A
/// foreign `session_*` file and a request dump beside it are never read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_finish_with_a_valid_scope_captures_the_snapshot() {
    let rig = Rig::start().await;
    let info = hermes_launched(&rig).await;
    let own = write_snapshot(&rig, ID);
    let sessions = rig.root.join("config-home/sessions");
    std::fs::write(
        sessions.join("session_20260101_000000_ffffff.json"),
        b"a foreign session must never persist\n",
    )
    .unwrap();
    std::fs::write(
        sessions.join(format!("request_dump_{ID}_20260930_141501_000000.json")),
        b"a request dump must never persist\n",
    )
    .unwrap();

    let sealed = seal(&rig, &info.entry_id, Some(ID), None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let records = rig.records(&info.entry_id);
    let opens = native_opens(&records);
    assert_eq!(opens.len(), 1, "{opens:?}");
    assert!(
        opens[0].1.contains(&format!("session_{ID}.json")),
        "{opens:?}"
    );
    assert_eq!(retained(&records, &opens[0].0), own);
    assert_eq!(
        streamless(&records, GapReason::SourceChanged),
        0,
        "{records:#?}"
    );
    assert_eq!(local_finishes(&records), 1);
    let everything = format!("{records:?}");
    assert!(!everything.contains("foreign"), "a foreign file was read");
}

/// A finish with no scope, or one that is malformed, or one that names no file, ends the
/// run with exactly one stream-less `SourceChanged` gap and no native stream.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_finish_without_a_usable_scope_records_exactly_one_gap() {
    let long = "a".repeat(129);
    let cases: [(&str, Option<&str>); 8] = [
        ("none", None),
        ("empty", Some("")),
        ("traversal", Some("../escape")),
        ("slash", Some("a/b")),
        ("space", Some("a b")),
        ("nul", Some("a\0b")),
        ("long", Some(long.as_str())),
        // Well formed, and the run's directory holds no such file.
        ("absent", Some("20260930_999999_000000")),
    ];
    for (what, scope) in cases {
        let rig = Rig::start().await;
        let info = hermes_launched(&rig).await;
        // A file exists, for a different id: it must not be picked up.
        write_snapshot(&rig, ID);
        let sealed = seal(&rig, &info.entry_id, scope, None).await;
        assert_eq!(sealed.status(), 200, "{what}: {}", sealed.body());
        let records = rig.records(&info.entry_id);
        assert_eq!(
            streamless(&records, GapReason::SourceChanged),
            1,
            "{what}: {records:#?}"
        );
        assert!(native_opens(&records).is_empty(), "{what}: {records:#?}");
        assert_eq!(local_finishes(&records), 1, "{what}");
    }
}

/// A scope on any other profile's finish is ignored. Codex's scope was due at its launch.
#[tokio::test]
async fn a_scope_on_another_profiles_finish_is_ignored() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let goal = rig.goal_accepted(&info.entry_id, b"say hi").await;
    let mut request = launch_request(&goal);
    request.native_scope = None;
    assert_eq!(rig.launch(&info.entry_id, &request).await.status(), 204);
    let sessions = rig.root.join("config-home/sessions/2026/09/30");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(sessions.join(format!("rollout-now-{ID}.jsonl")), b"{}\n").unwrap();

    let sealed = seal(&rig, &info.entry_id, Some(ID), None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let records = rig.records(&info.entry_id);
    assert!(native_opens(&records).is_empty(), "{records:#?}");
    // Exactly the one gap the launch itself recorded, and no second one from the finish.
    assert_eq!(
        streamless(&records, GapReason::SourceChanged),
        1,
        "{records:#?}"
    );
}

/// The daemon's half: a standalone seal sends no scope, so a snapshot that exists (a
/// Hermes that printed its identity after the seal was asked) is not read, and the gap says so.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_standalone_seal_leaves_a_late_snapshot_unread_and_says_so() {
    let rig = Rig::start().await;
    let info = hermes_launched(&rig).await;
    write_snapshot(&rig, ID);
    let sealed = seal(&rig, &info.entry_id, None, None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let records = rig.records(&info.entry_id);
    assert_eq!(
        streamless(&records, GapReason::SourceChanged),
        1,
        "{records:#?}"
    );
    assert!(native_opens(&records).is_empty());
    let capture = rig.status(&info.entry_id).await.payload.capture.unwrap();
    assert_eq!(capture.capture, CaptureState::Incomplete);
}

/// The binding is in memory. After a daemon restart the entry reads `IngestLost`, no
/// native capture is installed (there is no source to install it into), and the finish adds the
/// deferred gap even when it carries a scope and the file exists.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restart_before_the_finish_keeps_the_run_incomplete_with_the_deferred_gap() {
    let rig = Rig::start().await;
    let info = hermes_launched(&rig).await;
    write_snapshot(&rig, ID);
    let rig = rig.restart().await;
    let sealed = seal(&rig, &info.entry_id, Some(ID), None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let records = rig.records(&info.entry_id);
    assert!(
        streamless(&records, GapReason::IngestLost) >= 1,
        "{records:#?}"
    );
    assert_eq!(
        streamless(&records, GapReason::SourceChanged),
        1,
        "{records:#?}"
    );
    assert!(native_opens(&records).is_empty(), "{records:#?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restart_does_not_trust_deferred_binding_after_the_recorded_connector_changes() {
    let (rig, connectors) = copied_connector_rig().await;
    let info = hermes_launched(&rig).await;
    write_snapshot(&rig, ID);
    let harness = connectors.path().join("hermes-cli/harness.toml");
    let mut bytes = std::fs::read(&harness).unwrap();
    bytes.extend_from_slice(b"\n# changed after launch\n");
    std::fs::write(harness, bytes).unwrap();

    let rig = rig.restart().await;
    let sealed = seal(&rig, &info.entry_id, Some(ID), None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let records = rig.records(&info.entry_id);
    assert!(
        streamless(&records, GapReason::IngestLost) >= 1,
        "{records:#?}"
    );
    assert_eq!(
        streamless(&records, GapReason::SourceChanged),
        0,
        "changed bytes cannot reconstruct a deferred binding: {records:#?}"
    );
    assert!(native_opens(&records).is_empty(), "{records:#?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unrelated_corrupt_connector_does_not_hide_the_recorded_deferred_binding_on_restart() {
    let (rig, connectors) = copied_connector_rig().await;
    let info = hermes_launched(&rig).await;
    write_snapshot(&rig, ID);
    let corrupt = connectors.path().join("unrelated-corrupt");
    std::fs::create_dir(&corrupt).unwrap();
    std::fs::write(corrupt.join("harness.toml"), b"not valid toml = [").unwrap();

    let rig = rig.restart().await;
    let sealed = seal(&rig, &info.entry_id, Some(ID), None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let records = rig.records(&info.entry_id);
    assert_eq!(
        streamless(&records, GapReason::SourceChanged),
        1,
        "the matching recorded bytes still classify as deferred: {records:#?}"
    );
    assert!(native_opens(&records).is_empty(), "{records:#?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_folder_digest_twin_does_not_hide_the_genuine_deferred_harness_on_restart() {
    const PROFILE: &str = "restart-deferred-dev";
    let connectors = tempfile::tempdir().unwrap();
    let mut genuine = connectors.path().join("zz-genuine-deferred");
    support::copy_tree(&support::shipped_connector_dir("hermes-cli"), &genuine);
    // Change the schema-1 identity and therefore the connector-directory digest. The exact
    // changed bytes are copied into the wrong pinned folder below, while this directory remains
    // the genuine unpinned connector recorded at launch.
    let harness_path = genuine.join("harness.toml");
    let harness = std::fs::read_to_string(&harness_path).unwrap().replacen(
        "id = \"hermes-cli\"",
        &format!("id = \"{PROFILE}\""),
        1,
    );
    std::fs::write(harness_path, harness).unwrap();
    let root = connectors.path().to_path_buf();
    let rig = Rig::start_with(move |config| config.connectors_dir = root).await;
    let info = rig.created(PROFILE).await;
    let goal = rig.goal_accepted(&info.entry_id, b"say hi").await;
    let mut request = launch_request(&goal);
    request.native_scope = None;
    let launched = rig.launch(&info.entry_id, &request).await;
    assert_eq!(launched.status(), 204, "{}", launched.body());
    write_snapshot(&rig, ID);

    support::copy_tree(&genuine, &connectors.path().join("hermes-cli"));
    let order = (0..64)
        .find_map(|n| {
            let entries: Vec<_> = std::fs::read_dir(connectors.path())
                .unwrap()
                .map(|child| child.unwrap().file_name())
                .collect();
            let wrong = entries
                .iter()
                .position(|name| name == "hermes-cli")
                .unwrap();
            let right = entries
                .iter()
                .position(|name| name == genuine.file_name().unwrap())
                .unwrap();
            if wrong < right {
                return Some(entries);
            }
            let renamed = connectors
                .path()
                .join(format!("zz-genuine-deferred-{n:02}"));
            std::fs::rename(&genuine, &renamed).unwrap();
            genuine = renamed;
            None
        })
        .expect("fixture must enumerate the rejected matching digest before the genuine harness");
    let wrong = order.iter().position(|name| name == "hermes-cli").unwrap();
    let right = order
        .iter()
        .position(|name| name == genuine.file_name().unwrap())
        .unwrap();
    assert!(wrong < right, "fixture order: {order:?}");

    let rig = rig.restart().await;
    let sealed = seal(&rig, &info.entry_id, Some(ID), None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let records = rig.records(&info.entry_id);
    assert_eq!(
        streamless(&records, GapReason::SourceChanged),
        1,
        "the genuine harness still proves the recorded deferred binding: {records:#?}"
    );
    assert!(
        native_opens(&records).is_empty(),
        "restart must not reconstruct a native source: {records:#?}"
    );
}

/// A finish refused before the owner stops leaves the binding deferred, so the retry
/// installs the source; one refused after it was installed does not install it again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_finish_retried_with_the_scope_installs_the_source_once() {
    // Refused at stage 2 (the blocking permit), before anything is installed.
    let rig = Rig::start().await;
    let info = hermes_launched(&rig).await;
    let own = write_snapshot(&rig, ID);
    let hold = rig.daemon.hold_blocking_permits().await;
    let refused = seal(&rig, &info.entry_id, Some(ID), Some(1_500)).await;
    assert_eq!(refused.status(), 409, "{}", refused.body());
    drop(hold);
    let sealed = seal(&rig, &info.entry_id, Some(ID), None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let records = rig.records(&info.entry_id);
    let opens = native_opens(&records);
    assert_eq!(opens.len(), 1, "{records:#?}");
    assert_eq!(retained(&records, &opens[0].0), own);
    assert_eq!(
        streamless(&records, GapReason::SourceChanged),
        0,
        "{records:#?}"
    );

    // Refused at stage 4 (the owner), after the source was installed: the retry's scope is
    // not installed a second time, and the one source is finished by the owner that was
    // already given it.
    let rig = Rig::start().await;
    let info = hermes_launched(&rig).await;
    let own = write_snapshot(&rig, ID);
    rig.daemon.hold_capture_owner_drain(&info.entry_id, true);
    struct Release<'a>(&'a goalcup_daemon::runtime::Daemon, String);
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            self.0.hold_capture_owner_drain(&self.1, false);
        }
    }
    let _release = Release(&rig.daemon, info.entry_id.clone());
    let refused = seal(&rig, &info.entry_id, Some(ID), Some(3_000)).await;
    assert_eq!(refused.status(), 409, "{}", refused.body());
    rig.daemon.hold_capture_owner_drain(&info.entry_id, false);
    let sealed = seal(&rig, &info.entry_id, Some(ID), None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let records = rig.records(&info.entry_id);
    let opens = native_opens(&records);
    assert_eq!(opens.len(), 1, "{records:#?}");
    assert_eq!(retained(&records, &opens[0].0), own);
    assert_eq!(
        streamless(&records, GapReason::SourceChanged),
        0,
        "{records:#?}"
    );
}

/// The launcher ends its marker wait early enough that the daemon's native
/// read still has a share of the drain. A finish with 3 s left (the launcher's wait ended 1 s
/// before the drain share) reads the snapshot within the one second the daemon has left.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_finish_late_in_its_budget_still_reads_the_snapshot() {
    let rig = Rig::start().await;
    let info = hermes_launched(&rig).await;
    let own = write_snapshot(&rig, ID);
    let sealed = seal(&rig, &info.entry_id, Some(ID), Some(3_000)).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let records = rig.records(&info.entry_id);
    let opens = native_opens(&records);
    assert_eq!(opens.len(), 1, "{records:#?}");
    assert_eq!(retained(&records, &opens[0].0), own);
    assert_eq!(
        streamless(&records, GapReason::SourceChanged),
        0,
        "{records:#?}"
    );
    tokio::time::sleep(Duration::from_millis(10)).await;
}

/// Once the owner has been told to stop, a scope can no longer be read, so
/// it is not installed (installing it would set `Bound` for a source nothing will ever finish and
/// record no gap). A scope-less attempt that stopped the owner and was refused, then a retry that
/// carries the scope: the run ends with the one deferred gap and no native stream.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scope_is_not_installed_once_the_owner_was_told_to_stop() {
    let rig = Rig::start().await;
    let info = hermes_launched(&rig).await;
    write_snapshot(&rig, ID);
    rig.daemon.hold_capture_owner_drain(&info.entry_id, true);
    struct Release<'a>(&'a goalcup_daemon::runtime::Daemon, String);
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            self.0.hold_capture_owner_drain(&self.1, false);
        }
    }
    let _release = Release(&rig.daemon, info.entry_id.clone());
    let refused = seal(&rig, &info.entry_id, None, Some(3_000)).await;
    assert_eq!(refused.status(), 409, "{}", refused.body());
    rig.daemon.hold_capture_owner_drain(&info.entry_id, false);
    let sealed = seal(&rig, &info.entry_id, Some(ID), None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let records = rig.records(&info.entry_id);
    assert!(native_opens(&records).is_empty(), "{records:#?}");
    assert_eq!(
        streamless(&records, GapReason::SourceChanged),
        1,
        "{records:#?}"
    );
}

/// A finish that reached stage 5 (the deferred gap appended) and was
/// refused later, then a daemon restart: the reopened entry owes the gap again in memory, but its
/// chain already holds it, so the next finish does not append a second.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restart_after_the_deferred_gap_was_chained_does_not_record_it_twice() {
    let rig = Rig::start().await;
    let info = hermes_launched(&rig).await;
    write_snapshot(&rig, ID);
    rig.daemon.delay_capture_finish_stage(
        &info.entry_id,
        "workspace_walk",
        Duration::from_millis(5_300),
    );
    let refused = seal(&rig, &info.entry_id, None, Some(6_000)).await;
    assert_eq!(refused.status(), 409, "{}", refused.body());
    assert_eq!(
        streamless(&rig.records(&info.entry_id), GapReason::SourceChanged),
        1,
        "the refused finish chained the deferred gap at stage 5"
    );
    let rig = rig.restart().await;
    let sealed = seal(&rig, &info.entry_id, Some(ID), None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let records = rig.records(&info.entry_id);
    assert_eq!(
        streamless(&records, GapReason::SourceChanged),
        1,
        "{records:#?}"
    );
    assert!(
        streamless(&records, GapReason::IngestLost) >= 1,
        "{records:#?}"
    );
    assert!(native_opens(&records).is_empty());
}
