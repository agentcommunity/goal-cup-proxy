//! Protocol-v5 local capture through the real daemon and its control API: creation with a
//! quota-only chain and no server, the one-shot goal, the launch record, and the
//! signed status projection.

use crate::common::capture_lifecycle_support;
use crate::common::support;

use capture_lifecycle_support::{
    Child, RESERVE, Rig, Wire, chat_rig, controlled_native_rig, descriptors_on, dev_connectors,
    fresh_id, launch_request, local_finishes, native_rig,
};
use goalcup_protocol::capture::{
    AnalysisState, AnchorState, CaptureMeta, CaptureRecord, CaptureSource, CaptureState,
    ConnectionEvent, ConnectionOutcome, DeliveryState, EndReason, FrameEvent, GapReason, RunState,
    StreamKey, WireIdentity, is_capture_id,
};
use goalcup_protocol::control::{
    CaptureFinishReport, CaptureWorkspaceInfo, LocalSealOutcome, ProxyCounters,
    SUFFIX_CAPTURE_GOAL, SUFFIX_SEAL, SUFFIX_STATUS, SignedStatus, entry_route,
};
use goalcup_protocol::envelope::EventBody;
use goalcup_protocol::provenance::TerminationOutcomeV1;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// A key the shared redaction policy recognises (`aws_access_key`).
const PLANTED_SECRET: &str = "AKIAIOSFODNN7EXAMPLE";

/// Every directory and file below `root`, with file bytes, so a refusal cannot hide an empty
/// directory creation or a rewrite that happened to preserve the set of path names.
fn tree_inventory(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, current: &Path, inventory: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for child in std::fs::read_dir(current).expect("the snapshot root reads") {
            let child = child.expect("a snapshot directory entry reads");
            let path = child.path();
            let relative = path
                .strip_prefix(root)
                .expect("the walk stays below its root")
                .to_path_buf();
            let metadata = std::fs::symlink_metadata(&path).expect("snapshot metadata reads");
            if metadata.is_dir() {
                inventory.insert(relative, b"directory".to_vec());
                visit(root, &path, inventory);
            } else if metadata.is_file() {
                let mut value = b"file\0".to_vec();
                value.extend(std::fs::read(&path).expect("snapshot file reads"));
                inventory.insert(relative, value);
            } else {
                panic!("the capture fixture contains an unexpected object: {path:?}");
            }
        }
    }

    let mut inventory = BTreeMap::new();
    visit(root, root, &mut inventory);
    inventory
}

fn assert_error_identity(response: &support::Response, expected_message: &str) {
    assert_eq!(response.status(), 409, "{}", response.body());
    let body: goalcup_protocol::control::ErrorBody = response
        .json()
        .expect("the refusal has the complete control-error shape");
    assert_eq!(body.error.code, "conflict");
    assert_eq!(body.error.message, expected_message);
    assert_eq!(body.error.detail, serde_json::json!({ "code": "conflict" }));
}

#[derive(Debug, PartialEq)]
struct ReopenedCaptureSnapshot {
    row: goalcup_daemon::registry::Entry,
    chain: Vec<u8>,
    artifacts: BTreeMap<PathBuf, Vec<u8>>,
    workspace: BTreeMap<PathBuf, Vec<u8>>,
}

fn reopened_capture_snapshot(rig: &Rig, entry_id: &str) -> ReopenedCaptureSnapshot {
    let row = rig
        .daemon
        .registry()
        .get(entry_id)
        .expect("the registry reads")
        .expect("the capture row exists");
    ReopenedCaptureSnapshot {
        chain: std::fs::read(row.trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
            .expect("the chain reads"),
        artifacts: tree_inventory(&row.artifact_dir),
        workspace: tree_inventory(&row.workspace),
        row,
    }
}

#[tokio::test]
async fn local_capture_create_needs_no_server_config() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;

    assert_eq!(info.reserved_bytes, 0, "a chain is quota-only");
    assert!(info.ingest_addr.ip().is_loopback());
    assert!(is_capture_id(&info.producer_id));
    assert!(info.ingest_secret.len() >= 32);
    assert!(
        !format!("{info:?}").contains(&info.ingest_secret),
        "the ingest secret never reaches Debug output"
    );

    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    let launched = rig.launch(&info.entry_id, &launch_request(&goal)).await;
    assert_eq!(launched.status(), 204, "{}", launched.body());
    let status = rig.status(&info.entry_id).await;
    status
        .verify_with(&info.run_public_key)
        .expect("the status verifies against the run key the create call returned");

    let trace = rig.trace_dir(&info.entry_id);
    assert!(!trace.join("receipt.json").exists(), "no receipt exists");
    assert_eq!(
        rig.upload_dials(),
        0,
        "nothing ever connects to the configured upload URL"
    );
}

/// The blocks `chain.jsonl` holds beyond its logical end. A chain is a quota, not a
/// reservation: the run host holds the one reserve file, so nothing here may allocate ahead.
fn blocks_beyond_the_end(trace: &Path) -> (u64, u64) {
    let file = std::fs::metadata(trace.join("chain.jsonl")).unwrap();
    let allocated = file.blocks() * 512;
    (allocated.saturating_sub(file.len()), file.blksize())
}

#[tokio::test]
async fn a_sealed_entry_holds_no_blocks_beyond_its_end() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let sealed = rig.seal(&info.entry_id, None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());

    let (beyond, block) = blocks_beyond_the_end(&rig.trace_dir(&info.entry_id));
    assert!(
        beyond <= block,
        "{beyond} bytes are allocated past the sealed chain's end; one block is {block}"
    );
}

#[tokio::test]
async fn a_restart_reserves_nothing() {
    let rig = Rig::start().await;
    let mut ids = Vec::new();
    for _ in 0..5 {
        ids.push(rig.created("codex-cli").await.entry_id);
    }

    let rig = rig.restart().await;
    for id in &ids {
        let reopened = rig.status(id).await.payload.capture.unwrap();
        assert_eq!(reopened.storage.reserved_bytes, 0, "{id}");
        let (beyond, block) = blocks_beyond_the_end(&rig.trace_dir(id));
        assert!(
            beyond <= block,
            "{id}: {beyond} bytes are allocated past the reopened chain's end; one block is {block}"
        );
    }
}

#[tokio::test]
async fn v5_capture_uses_real_inline_chain_without_redb() {
    let rig = Rig::start().await;
    let info = rig.created("claude-code-cli").await;

    let chain = rig.chain(&info.entry_id);
    assert_eq!(chain.len(), 1, "the baseline is the only record");
    assert_eq!(chain[0].protocol_version, 5);
    let EventBody::Baseline {
        manifest_sha256,
        file_count,
    } = &chain[0].body
    else {
        panic!("seq 0 is the baseline, found {:?}", chain[0].body);
    };
    let trace = rig.trace_dir(&info.entry_id);
    let baseline = std::fs::read(trace.join("baseline.json")).expect("baseline.json");
    assert_eq!(*manifest_sha256, hex::encode(Sha256::digest(&baseline)));
    assert_eq!(*file_count, 0, "a fresh workspace is empty");
    assert!(
        !rig.artifact_dir(&info.entry_id)
            .join("buffer.redb")
            .exists(),
        "a v5 entry keeps no redb payload buffer"
    );

    let file = std::fs::metadata(trace.join("chain.jsonl")).unwrap();
    assert!(
        file.blocks() * 512 <= file.len() + file.blksize(),
        "the chain file holds no blocks beyond its end"
    );
    let storage = rig
        .status(&info.entry_id)
        .await
        .payload
        .capture
        .unwrap()
        .storage;
    assert_eq!(storage.quota_bytes, RESERVE);
    assert_eq!(storage.reserved_bytes, 0);
    assert_eq!(storage.used_bytes, file.len());
    let durable = storage.durable.expect("the baseline is synced");
    assert_eq!((durable.seq, durable.chain_bytes), (0, file.len()));
}

#[tokio::test]
async fn goal_sync_failure_prevents_preflight_success() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    // The retained writer keeps appending to the moved file, and its flush refuses because
    // the chain's name no longer names that file: a real sync failure.
    let trace = rig.trace_dir(&info.entry_id);
    std::fs::rename(trace.join("chain.jsonl"), trace.join("moved.jsonl")).unwrap();

    let failed = rig.goal(&info.entry_id, b"write hello.txt").await;
    assert_eq!(failed.status(), 500, "{}", failed.body());
    assert!(
        failed.json::<StreamKey>().is_err(),
        "no goal stream is handed out"
    );
    let capture = rig.status(&info.entry_id).await.payload.capture.unwrap();
    assert_eq!(capture.capture, CaptureState::Failed);
    assert_eq!(capture.storage.durable.map(|d| d.seq), Some(0));

    let again = rig.goal(&info.entry_id, b"write hello.txt").await;
    assert_eq!(
        again.status(),
        409,
        "a failed goal is not retried: {}",
        again.body()
    );
}

#[tokio::test]
async fn accepted_goal_cannot_be_replaced() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let goal = format!("Fix the build. The key {PLANTED_SECRET} is for the deploy step.");
    let key = rig.goal_accepted(&info.entry_id, goal.as_bytes()).await;
    assert!(is_capture_id(&key.producer));
    assert_ne!(
        key.producer, info.producer_id,
        "the goal has its own producer"
    );

    let second = rig.goal(&info.entry_id, b"a different goal").await;
    assert_eq!(second.status(), 409, "{}", second.body());

    let records = rig.records(&info.entry_id);
    let opens: Vec<_> = records
        .iter()
        .filter_map(|record| match record {
            CaptureRecord::Open { stream, source, .. } => Some((stream.clone(), *source)),
            _ => None,
        })
        .collect();
    assert_eq!(opens, vec![(key.clone(), CaptureSource::Goal)]);
    let mut retained = Vec::new();
    for record in &records {
        if let CaptureRecord::Segment {
            stream, data_b64, ..
        } = record
        {
            assert_eq!(stream, &key);
            use base64::Engine as _;
            retained.extend(
                base64::engine::general_purpose::STANDARD
                    .decode(data_b64)
                    .unwrap(),
            );
        }
    }
    let retained = String::from_utf8(retained).unwrap();
    assert!(retained.starts_with("Fix the build. The key "));
    assert!(!retained.contains(PLANTED_SECRET), "the goal is redacted");
    assert!(
        matches!(
            records.iter().rev().nth(1),
            Some(CaptureRecord::End { stream, reason: EndReason::Complete, .. }) if *stream == key
        ),
        "the goal stream ends complete, before its producer's close"
    );
    let raw = std::fs::read_to_string(rig.trace_dir(&info.entry_id).join("chain.jsonl")).unwrap();
    assert!(!raw.contains(PLANTED_SECRET));
    assert!(
        !raw.contains("a different goal"),
        "the refused goal left nothing"
    );
}

/// D53: the daemon records the build of the registered process that ran. The test child is
/// `/bin/sleep`, registered as the relative program `sleep`, so only its image is hashed.
#[tokio::test]
async fn the_launch_records_the_running_harness_build() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    let harness = Child::sleeping();
    let registration = harness.registration(info.process_generation);
    assert_eq!(
        rig.register(&info.entry_id, &registration).await.status(),
        204
    );
    let launched = rig.launch(&info.entry_id, &launch_request(&goal)).await;
    assert_eq!(launched.status(), 204, "{}", launched.body());
    let records = rig.records(&info.entry_id);
    let Some(CaptureRecord::Launch { harness_build, .. }) = records
        .iter()
        .find(|record| matches!(record, CaptureRecord::Launch { .. }))
    else {
        panic!("the launch is chained");
    };
    let build = harness_build.as_ref().expect("the build is recorded");
    assert_eq!(build.program_sha256, None);
    assert_eq!(
        build.image_sha256.as_deref(),
        Some(hex::encode(Sha256::digest(std::fs::read("/bin/sleep").unwrap())).as_str())
    );
    assert_eq!(
        build.platform,
        format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
    );
}

#[tokio::test]
async fn the_launch_record_names_the_goal_and_the_harness_profile() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    let after_goal = rig.status(&info.entry_id).await.payload.capture.unwrap();
    let goal_frontier = after_goal
        .storage
        .durable
        .clone()
        .expect("the goal is synced");
    assert_eq!(goal_frontier.chain_bytes, after_goal.storage.used_bytes);

    let launched = rig.launch(&info.entry_id, &launch_request(&goal)).await;
    assert_eq!(launched.status(), 204, "{}", launched.body());
    let records = rig.records(&info.entry_id);
    let Some(CaptureRecord::Launch {
        goal_stream,
        profile_id,
        profile_digest,
        recorder_revision,
        allowed_tool_rule,
        process_handoff_digest,
        run_public_key,
        connector_dir_sha256,
        harness_build,
        ..
    }) = records
        .iter()
        .find(|record| matches!(record, CaptureRecord::Launch { .. }))
    else {
        panic!("the launch is chained");
    };
    // D53: no process was registered here, so no build was observed, and none is guessed.
    assert_eq!(harness_build, &None);
    assert_eq!(goal_stream, &goal);
    assert_eq!(profile_id, "codex-cli");
    let harness =
        std::fs::read(support::shipped_connector_dir("codex-cli").join("harness.toml")).unwrap();
    assert_eq!(*profile_digest, hex::encode(Sha256::digest(&harness)));
    assert!(!recorder_revision.is_empty());
    assert_eq!(allowed_tool_rule, &None);
    assert_eq!(process_handoff_digest, &None);
    // Contract S1: the daemon writes the key it minted at create, canonical lowercase hex.
    // `connector_dir_sha256` is forwarded from the entry row and names the exact loaded
    // connector directory bytes.
    assert_eq!(
        run_public_key.as_deref(),
        Some(info.run_public_key.as_str())
    );
    assert_eq!(
        connector_dir_sha256.as_deref(),
        goalcup_protocol::pins::pinned_connector_dir_sha256("codex-cli")
    );

    let after_launch = rig.status(&info.entry_id).await.payload.capture.unwrap();
    assert!(after_launch.storage.used_bytes > goal_frontier.chain_bytes);
}

#[tokio::test]
async fn a_default_launch_does_not_revalidate_an_unrelated_stored_digest() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let goal = rig.goal_accepted(&info.entry_id, b"private goal").await;
    let harness = Child::sleeping();
    let registration = harness.registration(info.process_generation);
    assert_eq!(
        rig.register(&info.entry_id, &registration).await.status(),
        204
    );
    let registry = rig.daemon.registry();
    let mut row: serde_json::Value =
        serde_json::from_slice(&registry.raw_json_for_test(&info.entry_id).unwrap().unwrap())
            .unwrap();
    row["process_handoff_digest"] = serde_json::json!("0".repeat(64));
    registry
        .replace_raw_json_for_test(&info.entry_id, &serde_json::to_vec(&row).unwrap())
        .unwrap();
    assert_eq!(
        rig.launch(&info.entry_id, &launch_request(&goal))
            .await
            .status(),
        204
    );
    let records = rig.records(&info.entry_id);
    let launch = records
        .iter()
        .find(|record| matches!(record, CaptureRecord::Launch { .. }))
        .unwrap();
    assert!(matches!(
        launch,
        CaptureRecord::Launch {
            allowed_tool_rule: None,
            process_handoff_digest: None,
            ..
        }
    ));
}

/// The stream-less gaps chained after the entry's Launch, by reason.
fn gaps_after_launch(records: &[CaptureRecord]) -> Vec<GapReason> {
    let launch = position(records, |r| matches!(r, CaptureRecord::Launch { .. }));
    records[launch + 1..]
        .iter()
        .filter_map(|record| match record {
            CaptureRecord::Gap {
                stream: None,
                reason,
                lost_bytes: None,
                ..
            } => Some(*reason),
            _ => None,
        })
        .collect()
}

/// A shipped family declares one required scoped source; no scope records one gap.
/// V1 supplemental stores add no gaps. A controlled family retains run-HOME capture.
#[tokio::test]
async fn a_launch_marks_each_store_it_does_not_read() {
    let rig = Rig::start_headless().await;
    let info = rig.created("headless-codex-cli").await;
    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    let mut request = launch_request(&goal);
    request.native_scope = None;
    let launched = rig.launch(&info.entry_id, &request).await;
    assert_eq!(launched.status(), 204, "{}", launched.body());
    let stores = 1;
    assert_eq!(
        gaps_after_launch(&rig.records(&info.entry_id)),
        vec![GapReason::SourceChanged; stores]
    );
    let capture = rig.status(&info.entry_id).await.payload.capture.unwrap();
    assert_eq!(capture.gap_count, stores as u64);
    assert_eq!(capture.capture, CaptureState::Incomplete);

    let (rig, _native, _connectors) = native_rig().await;
    let info = rig.created("capture-dev").await;
    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    let launched = rig.launch(&info.entry_id, &launch_request(&goal)).await;
    assert_eq!(launched.status(), 204, "{}", launched.body());
    assert_eq!(gaps_after_launch(&rig.records(&info.entry_id)), vec![]);
    let capture = rig.status(&info.entry_id).await.payload.capture.unwrap();
    assert_eq!(capture.gap_count, 0);
    assert_eq!(capture.capture, CaptureState::Recording);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_scope_excludes_other_runs_files() {
    use base64::Engine as _;
    for (profile, directory, own_name, other_name) in [
        (
            "headless-codex-cli",
            "sessions/2026/09/30",
            "rollout-now-test-session.jsonl",
            "rollout-now-foreign.jsonl",
        ),
        (
            "headless-claude-code-cli",
            "projects/workspace",
            "test-session.jsonl",
            "foreign.jsonl",
        ),
    ] {
        let rig = Rig::start_headless().await;
        let native = rig.root.join("config-home").join(directory);
        std::fs::create_dir_all(&native).unwrap();
        std::fs::write(native.join(own_name), b"own session\n").unwrap();
        std::fs::write(
            native.join(other_name),
            b"foreign session must never persist\n",
        )
        .unwrap();
        // V1 supplemental store content must also remain outside this capture.
        std::fs::write(
            rig.root.join("config-home/history.jsonl"),
            b"foreign history\n",
        )
        .unwrap();
        let info = rig.created(profile).await;
        let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
        let request = launch_request(&goal);
        assert_eq!(rig.launch(&info.entry_id, &request).await.status(), 204);
        rig.records_when(&info.entry_id, |records| native_producer(records).is_some())
            .await;
        // A retried Launch neither appends another Launch nor rebinds to a foreign scope.
        let mut retry = request.clone();
        retry.native_scope = Some("foreign".into());
        assert_eq!(rig.launch(&info.entry_id, &retry).await.status(), 204);
        assert_eq!(rig.seal(&info.entry_id, None).await.status(), 200);
        let records = rig.records(&info.entry_id);
        let native_opens: Vec<_> = records
            .iter()
            .filter_map(|record| match record {
                CaptureRecord::Open {
                    stream,
                    source: CaptureSource::Native,
                    metadata,
                    ..
                } => Some((stream, &metadata.label)),
                _ => None,
            })
            .collect();
        assert_eq!(native_opens.len(), 1, "{records:?}");
        assert!(
            native_opens[0].1.contains(own_name),
            "{:?}",
            native_opens[0]
        );
        let retained: Vec<u8> = records
            .iter()
            .filter_map(|record| match record {
                CaptureRecord::Segment {
                    stream, data_b64, ..
                } if stream == native_opens[0].0 => Some(
                    base64::engine::general_purpose::STANDARD
                        .decode(data_b64)
                        .unwrap(),
                ),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(retained, b"own session\n");
        assert_eq!(gaps_after_launch(&records), vec![]);
        assert_eq!(
            records
                .iter()
                .filter(|r| matches!(r, CaptureRecord::Launch { .. }))
                .count(),
            1
        );
    }
}

/// Incident H-17: a refusal test that points `config_home` into the live `HOME` must never
/// run against it. On source that has regressed (or not yet gained) a refusal, the daemon
/// ACCEPTS `$HOME/.hermes` and its native capture creates `$HOME/.hermes/sessions`: the first
/// RED run of the Hermes `.hermes` refusal did exactly that, in the participant's real home.
///
/// So the refusals run in a CHILD of this test binary whose `HOME` is a private temporary
/// directory (the pattern `shipped_connectors::the_child_half_of_the_connectors_dir_fallback`
/// uses). Whatever a regressed daemon creates, it creates there. This parent only starts the
/// child, and brackets it with a guard on the real home: the same three app-wide harness
/// directories must exist, or not, exactly as before.
#[test]
fn refused_native_roots_run_under_a_private_home_and_never_touch_the_real_one() {
    let real = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let app_wide = |home: &Option<std::path::PathBuf>| {
        [".hermes", ".codex", ".claude"].map(|name| {
            home.as_ref()
                .map(|h| std::fs::symlink_metadata(h.join(name)).is_ok())
        })
    };
    let before = app_wide(&real);
    let private = tempfile::tempdir().expect("a private home");
    let private_home = private.path().canonicalize().expect("a resolvable home");
    assert_ne!(Some(private_home.clone()), real.clone());

    let out = std::process::Command::new(std::env::current_exe().expect("this test binary"))
        .args([
            "--exact",
            CHILD_TEST,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("HOME", &private_home)
        .env(PRIVATE_HOME_ENV, &private_home)
        .output()
        .expect("the child runs");
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // The guard comes first: even a failing child must not have reached the real home.
    assert_eq!(
        app_wide(&real),
        before,
        "the real HOME's app-wide harness directories changed while the child ran"
    );
    assert!(out.status.success(), "the child failed:\n{printed}");
    assert!(
        printed.contains("1 passed"),
        "the child did not run its one test:\n{printed}"
    );
    // And nothing was created in the private home either: a refusal creates nothing.
    for name in [".hermes", ".codex", ".claude"] {
        assert!(
            !private_home.join(name).exists(),
            "the refusals created {name} in the private home"
        );
    }
}

// The libtest name inside the folded `it` binary, which prefixes the module path.
const CHILD_TEST: &str = "capture_lifecycle::refused_native_roots_child_half";
const PRIVATE_HOME_ENV: &str = "GOALCUP_TEST_PRIVATE_HOME";

/// Run only by [`refused_native_roots_run_under_a_private_home_and_never_touch_the_real_one`].
/// It refuses to run against any other `HOME`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "run by its parent, with a private HOME"]
async fn refused_native_roots_child_half() {
    let Some(private) = std::env::var_os(PRIVATE_HOME_ENV) else {
        println!("${PRIVATE_HOME_ENV} is unset: this case is run by its parent, not alone.");
        return;
    };
    let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME"));
    assert_eq!(
        home,
        std::path::PathBuf::from(private),
        "the child's HOME is the parent's private directory, and nothing else"
    );
    assert!(
        home.starts_with(std::env::temp_dir().canonicalize().unwrap())
            || home.starts_with("/private/var")
            || home.starts_with("/var"),
        "{} is not a temporary directory",
        home.display()
    );
    let rig = Rig::start().await;
    let link = rig.root.join("home-link");
    std::os::unix::fs::symlink(&home, &link).unwrap();
    for config_home in [
        home.clone(),
        home.join(".codex"),
        home.join(".claude/nested"),
        link.join(".codex/nested"),
        link.join(".claude"),
        home.join(".hermes"),
        home.join(".hermes/profiles/nested"),
        link.join(".hermes"),
        std::path::PathBuf::from("relative"),
    ] {
        let workspace = rig.workspace("refused-native-root");
        let mut request = rig.request("codex-cli", &workspace, RESERVE);
        request.config_home = config_home.display().to_string();
        let response = rig.create_request(&request).await;
        assert!(
            matches!(response.status(), 400 | 409),
            "{}: {}",
            config_home.display(),
            response.body()
        );
        assert!(!workspace.exists());
        assert!(
            std::fs::read_dir(rig.config.artifact_root.join("artifacts"))
                .map_or(true, |mut dir| dir.next().is_none())
        );
    }
}

#[tokio::test]
async fn native_scope_unavailable_records_named_gap() {
    for scope in [None, Some("../foreign"), Some(""), Some("bad.scope")] {
        for profile in ["headless-codex-cli", "headless-claude-code-cli"] {
            let rig = Rig::start_headless().await;
            let info = rig.created(profile).await;
            let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
            let mut request = launch_request(&goal);
            request.native_scope = scope.map(str::to_owned);
            assert_eq!(rig.launch(&info.entry_id, &request).await.status(), 204);
            assert_eq!(rig.seal(&info.entry_id, None).await.status(), 200);
            let records = rig.records(&info.entry_id);
            assert_eq!(gaps_after_launch(&records), vec![GapReason::SourceChanged]);
            assert!(native_producer(&records).is_none());
            assert_eq!(local_finishes(&records), 1);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_native_source_never_opened_is_a_gap() {
    for vanished in [false, true] {
        let rig = Rig::start_headless().await;
        let source = rig
            .root
            .join("config-home/sessions/2026/09/30/rollout-now-test-session.jsonl");
        let info = rig.created("headless-codex-cli").await;
        let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
        assert_eq!(
            rig.launch(&info.entry_id, &launch_request(&goal))
                .await
                .status(),
            204
        );
        if vanished {
            std::fs::create_dir_all(source.parent().unwrap()).unwrap();
            std::fs::write(&source, b"own native bytes\n").unwrap();
            rig.records_when(&info.entry_id, |records| native_producer(records).is_some())
                .await;
            std::fs::remove_file(&source).unwrap();
        }
        assert_eq!(rig.seal(&info.entry_id, None).await.status(), 200);
        let records = rig.records(&info.entry_id);
        assert_eq!(gaps_after_launch(&records), vec![GapReason::SourceChanged]);
        assert_eq!(local_finishes(&records), 1);
    }
    // A fixed source root that disappears after create cannot be constructed at Launch.
    let rig = Rig::start_headless().await;
    let info = rig.created("headless-codex-cli").await;
    std::fs::remove_dir(rig.root.join("config-home/sessions")).unwrap();
    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    rig.launch(&info.entry_id, &launch_request(&goal)).await;
    assert_eq!(rig.seal(&info.entry_id, None).await.status(), 200);
    assert_eq!(
        gaps_after_launch(&rig.records(&info.entry_id)),
        vec![GapReason::SourceChanged]
    );
}

#[tokio::test]
async fn local_capture_status_has_no_signed_duration() {
    let (rig, _native, _connectors) = native_rig().await;
    let info = rig.created("capture-dev").await;
    let created = rig.status(&info.entry_id).await;
    created.verify_with(&info.run_public_key).unwrap();
    let payload = &created.payload;
    assert_eq!(payload.entry_id, info.entry_id);
    assert_eq!(payload.deadline_at, None);
    assert_eq!(payload.seconds_remaining, None);
    assert_eq!(payload.started_at, None);
    assert_eq!(payload.sealed_at, None);
    assert!(!payload.detail.contains("synced"), "{}", payload.detail);
    let capture = payload
        .capture
        .clone()
        .expect("a v5 status carries its capture");
    assert_eq!(capture.run, RunState::Starting);
    assert_eq!(capture.capture, CaptureState::Starting);
    assert_eq!(capture.delivery, DeliveryState::LocalOnly);
    assert_eq!(capture.anchor, AnchorState::LocalOnly);
    assert_eq!(capture.analysis, AnalysisState::NotRun);
    assert_eq!(capture.gap_count, 0);
    assert!(!capture.assistance_observed);

    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    rig.launch(&info.entry_id, &launch_request(&goal)).await;
    let launched = rig.status(&info.entry_id).await;
    launched.verify_with(&info.run_public_key).unwrap();
    let capture = launched.payload.capture.unwrap();
    assert_eq!(capture.run, RunState::Running);
    assert_eq!(capture.capture, CaptureState::Recording);
    assert_eq!(launched.payload.deadline_at, None);
    assert_eq!(launched.payload.seconds_remaining, None);
}

#[tokio::test]
async fn status_carries_the_entrys_own_admission_counts() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let fresh = rig.status(&info.entry_id).await.payload;
    assert_eq!(
        (fresh.producers_consumed, fresh.admissions_refused),
        (Some(0), Some(0))
    );

    // Refused before authentication: a listener-wide count, not this entry's.
    let mut wrong = info.clone();
    wrong.ingest_secret = "not-the-secret-not-the-secret-not-the-secret".to_string();
    tokio::task::spawn_blocking(move || Wire::connect(&wrong, &fresh_id()).hang_up())
        .await
        .unwrap();
    let after_wrong = rig.status(&info.entry_id).await.payload;
    assert_eq!(
        (
            after_wrong.producers_consumed,
            after_wrong.admissions_refused
        ),
        (Some(0), Some(0))
    );

    // Authenticated: the entry consumed the producer ID, and its owner admitted the epoch.
    let wired = info.clone();
    tokio::task::spawn_blocking(move || Wire::connect(&wired, &wired.producer_id).hang_up())
        .await
        .unwrap();
    let after_right = rig.status(&info.entry_id).await.payload;
    assert_eq!(
        (
            after_right.producers_consumed,
            after_right.admissions_refused
        ),
        (Some(1), Some(0))
    );
}

#[tokio::test]
async fn both_shipped_manifests_compile_at_preflight() {
    let rig = Rig::start().await;
    for profile in ["codex-cli", "claude-code-cli"] {
        let info = rig.created(profile).await;
        assert_eq!(info.reserved_bytes, 0, "{profile}");
    }

    // A connector whose carried pattern has no streaming recognizer: the daemon refuses the
    // capture before it reserves, creates or authorizes anything.
    let connectors = dev_connectors(|harness| {
        let start = harness.find("[redaction]").unwrap();
        let end = harness[start..].find("[[capture.hosts]]").unwrap() + start;
        harness.replace_range(
            start..end,
            "[redaction]\nextra_patterns = [{ kind = \"custom\", regex = \"custom-[0-9]{8}\" }]\n\n",
        );
    });
    let dir = connectors.path().to_path_buf();
    let dev_rig = Rig::start_with(move |config| config.connectors_dir = dir).await;

    let workspace = dev_rig.workspace("refused");
    let refused = dev_rig.create("capture-dev", &workspace, RESERVE).await;
    assert_eq!(refused.status(), 422, "{}", refused.body());
    assert!(
        refused.body().contains("preflight_failed"),
        "{}",
        refused.body()
    );
    assert!(
        refused.body().contains("custom"),
        "the refusal names the pattern's kind"
    );
    assert!(
        !workspace.exists(),
        "a refused capture creates no workspace"
    );
    let artifacts = dev_rig.config.artifact_root.join("artifacts");
    assert!(
        std::fs::read_dir(&artifacts).map_or(true, |mut dir| dir.next().is_none()),
        "a refused capture leaves no entry artifacts"
    );
}

#[tokio::test]
async fn a_restart_never_rebuilds_a_capture_row_as_a_legacy_runtime() {
    let rig = Rig::start_headless().await;
    let info = rig.created("headless-codex-cli").await;
    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    let mut launch = launch_request(&goal);
    // This fixture isolates reopen behavior: no native producer is bound.
    launch.native_scope = None;
    rig.launch(&info.entry_id, &launch).await;
    let chain = rig.trace_dir(&info.entry_id).join("chain.jsonl");
    let before = std::fs::read(&chain).unwrap();
    let records_before = rig.records(&info.entry_id).len();

    let rig = rig.restart().await;
    // Reopened for status and seal: the Launch appended before the stop is durable.
    let reopened = rig.status(&info.entry_id).await;
    reopened.verify_with(&info.run_public_key).unwrap();
    let capture = reopened.payload.capture.unwrap();
    assert_eq!(capture.run, RunState::Running);
    // No producer was live at the stop, as after a crash, so the stop chained nothing. The
    // reopened entry admits nothing, and its one stream-less gap says so, beside the launch's
    // one required-source gap for the unavailable native scope.
    let unread = 1;
    assert_eq!(capture.gap_count, unread + 1);
    assert_eq!(capture.capture, CaptureState::Incomplete);
    let after = std::fs::read(&chain).unwrap();
    assert_eq!(
        capture.storage.durable.map(|d| d.chain_bytes),
        Some(after.len() as u64)
    );
    let listed = rig.daemon.entries().unwrap();
    assert!(listed.iter().any(|e| e.entry_id == info.entry_id));
    let cycles = rig.daemon.cycle_errors();
    assert_eq!(
        (cycles.tick, cycles.last),
        (0, None),
        "nothing failed to rebuild"
    );
    // The reopen gap is the one record the restart adds: no legacy restart record.
    assert!(after.starts_with(&before));
    let records = rig.records(&info.entry_id);
    assert_eq!(records.len(), records_before + 1);
    assert!(matches!(
        records.last(),
        Some(CaptureRecord::Gap {
            stream: None,
            reason: GapReason::IngestLost,
            ..
        })
    ));
}

/// A harness registered before its launch is recorded is already running: when the daemon
/// goes away in between, as a crash would, the reopened entry chains that loss too.
#[tokio::test]
async fn a_registered_run_reopened_before_its_launch_chains_the_loss() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    let harness = Child::sleeping();
    let registered = rig
        .register(
            &info.entry_id,
            &harness.registration(info.process_generation),
        )
        .await;
    assert_eq!(registered.status(), 204, "{}", registered.body());

    let rig = rig.restart().await;
    let records = rig.records(&info.entry_id);
    assert!(
        matches!(
            records.last(),
            Some(CaptureRecord::Gap {
                stream: None,
                reason: GapReason::IngestLost,
                ..
            })
        ),
        "{records:#?}"
    );
    let capture = rig.status(&info.entry_id).await.payload.capture.unwrap();
    assert_eq!(capture.gap_count, 1);
    assert_eq!(capture.capture, CaptureState::Incomplete);
}

/// Reopening retains exactly enough authority to report and seal. Neither an unfinished entry
/// that has no goal nor one whose goal was already recorded may regain any creation-time
/// mutation authority after the daemon restarts. (Abandoning one that never launched is not a
/// mutation of a run: `a_recovered_capture_that_never_launched_can_be_abandoned`.)
#[tokio::test]
async fn a_reopened_capture_accepts_only_status_and_seal() {
    let rig = Rig::start().await;
    let pre_goal = rig.created("codex-cli").await;
    let post_goal = rig.created("codex-cli").await;
    let goal = rig
        .goal_accepted(&post_goal.entry_id, b"write hello.txt")
        .await;
    let child = Child::sleeping();
    let registration = child.registration(pre_goal.process_generation);

    let rig = rig.restart().await;
    let pre_goal_snapshot = reopened_capture_snapshot(&rig, &pre_goal.entry_id);
    let post_goal_snapshot = reopened_capture_snapshot(&rig, &post_goal.entry_id);

    let assert_unchanged = || {
        assert_eq!(
            reopened_capture_snapshot(&rig, &pre_goal.entry_id),
            pre_goal_snapshot,
            "the pre-goal capture changed"
        );
        assert_eq!(
            reopened_capture_snapshot(&rig, &post_goal.entry_id),
            post_goal_snapshot,
            "the post-goal capture changed"
        );
    };
    let recovered_conflict = |entry_id: &str| {
        format!("conflict: entry {entry_id} is a recovered capture restricted to status and seal")
    };

    let response = rig.goal(&pre_goal.entry_id, b"a late goal").await;
    assert_error_identity(&response, &recovered_conflict(&pre_goal.entry_id));
    assert_unchanged();

    let response = rig.register(&pre_goal.entry_id, &registration).await;
    assert_error_identity(&response, &recovered_conflict(&pre_goal.entry_id));
    assert_unchanged();

    let response = rig
        .launch(&post_goal.entry_id, &launch_request(&goal))
        .await;
    assert_error_identity(&response, &recovered_conflict(&post_goal.entry_id));
    assert_unchanged();

    rig.status(&pre_goal.entry_id)
        .await
        .verify_with(&pre_goal.run_public_key)
        .expect("the pre-goal recovered status verifies");
    rig.status(&post_goal.entry_id)
        .await
        .verify_with(&post_goal.run_public_key)
        .expect("the post-goal recovered status verifies");
    assert_unchanged();

    let post_goal_chain_before = rig.chain(&post_goal.entry_id);
    let sealed = rig.seal(&post_goal.entry_id, None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let report: CaptureFinishReport = sealed.json().expect("the local finish report");
    let post_goal_chain_after = rig.chain(&post_goal.entry_id);
    let post_goal_chain_bytes_after = std::fs::read(
        rig.trace_dir(&post_goal.entry_id)
            .join(goalcup_protocol::export::CHAIN_FILE),
    )
    .expect("the sealed chain bytes read");
    assert!(
        post_goal_chain_bytes_after.starts_with(&post_goal_snapshot.chain),
        "seal appends after the exact recovered chain bytes"
    );
    assert_eq!(
        post_goal_chain_after.len(),
        post_goal_chain_before.len() + 1,
        "a v5 local finish appends exactly one terminal envelope"
    );
    assert_eq!(
        &post_goal_chain_after[..post_goal_chain_before.len()],
        post_goal_chain_before.as_slice(),
        "seal leaves every pre-terminal envelope byte-for-byte represented by the same value"
    );
    let terminal = post_goal_chain_after
        .last()
        .expect("the LocalFinish is the terminal envelope");
    let previous = post_goal_chain_before
        .last()
        .expect("the recovered chain has its baseline and goal");
    let EventBody::Capture {
        record:
            CaptureRecord::LocalFinish {
                seal_manifest_sha256,
                ..
            },
    } = &terminal.body
    else {
        panic!("the only suffix envelope is LocalFinish: {terminal:#?}");
    };
    assert!(
        !post_goal_chain_after[post_goal_chain_before.len()..]
            .iter()
            .any(|envelope| matches!(envelope.body, EventBody::Seal { .. })),
        "EventBody::Seal belongs to the legacy V1 seal path, not v5 LocalFinish"
    );
    assert_eq!(terminal.link.seq, previous.link.seq + 1);
    assert_eq!(terminal.link.prev, previous.link.hash);
    assert_eq!(
        terminal.link.hash,
        terminal
            .expected_hash(&previous.link.hash)
            .expect("the terminal envelope hashes")
    );
    assert_eq!(report.terminal_seq, Some(terminal.link.seq));
    let seal_path = rig.trace_dir(&post_goal.entry_id).join("seal.json");
    let seal_bytes = std::fs::read(&seal_path).expect("seal publishes its terminal manifest");
    assert_eq!(
        seal_manifest_sha256,
        &hex::encode(Sha256::digest(&seal_bytes)),
        "LocalFinish binds the exact seal.json bytes"
    );
    let sealed_status = rig.status(&post_goal.entry_id).await;
    sealed_status
        .verify_with(&post_goal.run_public_key)
        .expect("the sealed recovered status verifies");
    let post_seal_row = rig
        .daemon
        .registry()
        .get(&post_goal.entry_id)
        .expect("the registry reads after seal")
        .expect("the sealed capture row remains");
    assert_eq!(post_seal_row.sealed_at, sealed_status.payload.sealed_at);
    let mut expected_post_seal_row = post_goal_snapshot.row.clone();
    expected_post_seal_row.sealed_at = post_seal_row.sealed_at.clone();
    let expected_capture = expected_post_seal_row
        .capture
        .as_mut()
        .expect("the expected row is a capture");
    expected_capture.finished = Some(report.clone());
    expected_capture.sealed_status = Some(
        sealed_status
            .payload
            .capture
            .clone()
            .expect("the sealed status carries capture state"),
    );
    assert_eq!(
        post_seal_row, expected_post_seal_row,
        "seal changes only sealed_at, the finish report, and the retained sealed status"
    );
    assert_eq!(
        tree_inventory(&post_goal_snapshot.row.workspace),
        post_goal_snapshot.workspace,
        "seal only inventories the workspace"
    );
    let mut before_artifacts = post_goal_snapshot.artifacts.clone();
    let mut after_artifacts = tree_inventory(&post_goal_snapshot.row.artifact_dir);
    assert!(
        after_artifacts
            .remove(&PathBuf::from("trace/seal.json"))
            .is_some(),
        "seal.json is the one new artifact"
    );
    before_artifacts.remove(&PathBuf::from("trace/chain.jsonl"));
    after_artifacts.remove(&PathBuf::from("trace/chain.jsonl"));
    assert_eq!(
        after_artifacts, before_artifacts,
        "seal changes only chain.jsonl and adds seal.json"
    );
    assert_eq!(
        reopened_capture_snapshot(&rig, &pre_goal.entry_id),
        pre_goal_snapshot,
        "sealing the other recovered capture changes nothing here"
    );
}

#[tokio::test]
async fn the_goal_producer_is_closed_inside_the_goal_sync() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let key = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    let records = rig.records(&info.entry_id);
    let closed: Vec<_> = records
        .iter()
        .filter_map(|record| match record {
            CaptureRecord::ProducerClosed {
                producer,
                final_seq,
                ..
            } => Some((producer.clone(), *final_seq)),
            _ => None,
        })
        .collect();
    assert_eq!(
        closed,
        vec![(key.producer.clone(), Some(2))],
        "Open, Bytes and End are the goal producer's sequences 0, 1 and 2"
    );
    assert!(matches!(
        records.last(),
        Some(CaptureRecord::ProducerClosed { .. })
    ));
    let chain = rig.chain(&info.entry_id);
    let durable = rig
        .status(&info.entry_id)
        .await
        .payload
        .capture
        .unwrap()
        .storage
        .durable;
    assert_eq!(
        durable.map(|d| d.seq),
        Some(chain.len() as u64 - 1),
        "the close is inside the sync that precedes the key"
    );
}

#[tokio::test]
async fn create_refuses_a_relative_run_home() {
    let rig = Rig::start().await;
    let workspace = rig.workspace("relative-home");
    let mut request = rig.request("codex-cli", &workspace, RESERVE);
    request.run_home = "relative/home".to_string();
    let refused = rig.create_request(&request).await;
    assert_eq!(refused.status(), 400, "{}", refused.body());
    assert!(!workspace.exists());
}

#[tokio::test]
async fn a_capture_process_registers_once_before_its_launch() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    assert_eq!(info.process_generation, 1);
    let harness = Child::sleeping();
    let registration = harness.registration(info.process_generation);

    let registered = rig.register(&info.entry_id, &registration).await;
    assert_eq!(registered.status(), 204, "{}", registered.body());
    let replayed = rig.register(&info.entry_id, &registration).await;
    assert_eq!(replayed.status(), 204, "an exact replay is acknowledged");
    let decoy = Child::sleeping();
    let second = rig
        .register(&info.entry_id, &decoy.registration(info.process_generation))
        .await;
    assert_eq!(second.status(), 409, "{}", second.body());
    let row = rig.daemon.registry().get(&info.entry_id).unwrap().unwrap();
    assert_eq!(
        row.last_process.map(|handle| handle.pid),
        Some(harness.0.id())
    );

    // After the launch is recorded no process is admitted, even one never offered before.
    let late = rig.created("codex-cli").await;
    let goal = rig.goal_accepted(&late.entry_id, b"write hello.txt").await;
    let launched = rig.launch(&late.entry_id, &launch_request(&goal)).await;
    assert_eq!(launched.status(), 204, "{}", launched.body());
    let after = rig
        .register(
            &late.entry_id,
            &harness.registration(late.process_generation),
        )
        .await;
    assert_eq!(after.status(), 409, "{}", after.body());
    let row = rig.daemon.registry().get(&late.entry_id).unwrap().unwrap();
    assert_eq!(row.last_process, None);
}

#[tokio::test]
async fn abandon_before_launch_leaves_nothing_and_later_is_refused() {
    let rig = Rig::start().await;
    let workspace = rig.workspace("abandoned");
    let created = rig.create("codex-cli", &workspace, RESERVE).await;
    assert_eq!(created.status(), 200, "{}", created.body());
    let info: CaptureWorkspaceInfo = created.json().unwrap();
    rig.goal_accepted(&info.entry_id, b"write hello.txt").await;

    let abandoned = rig.abandon(&info.entry_id).await;
    assert_eq!(abandoned.status(), 204, "{}", abandoned.body());
    assert!(
        !rig.artifact_dir(&info.entry_id).exists(),
        "the chain is gone"
    );
    assert!(rig.daemon.registry().get(&info.entry_id).unwrap().is_none());
    assert!(!workspace.exists(), "the still-empty workspace is gone");
    let status = rig
        .client
        .get_no_token(&entry_route(&info.entry_id, SUFFIX_STATUS))
        .await;
    assert_eq!(status.status(), 404, "{}", status.body());
    assert_eq!(rig.goal(&info.entry_id, b"again").await.status(), 404);
    let again = rig.create("codex-cli", &workspace, RESERVE).await;
    assert_eq!(
        again.status(),
        200,
        "the workspace is free: {}",
        again.body()
    );

    let launched = rig.created("codex-cli").await;
    let goal = rig
        .goal_accepted(&launched.entry_id, b"write hello.txt")
        .await;
    rig.launch(&launched.entry_id, &launch_request(&goal)).await;
    let refused = rig.abandon(&launched.entry_id).await;
    assert_eq!(refused.status(), 409, "{}", refused.body());
    assert!(refused.body().contains("seal"), "{}", refused.body());
    assert!(
        rig.trace_dir(&launched.entry_id)
            .join("chain.jsonl")
            .exists()
    );

    let owned = rig.created("codex-cli").await;
    let harness = Child::sleeping();
    let registered = rig
        .register(
            &owned.entry_id,
            &harness.registration(owned.process_generation),
        )
        .await;
    assert_eq!(registered.status(), 204, "{}", registered.body());
    let refused = rig.abandon(&owned.entry_id).await;
    assert_eq!(
        refused.status(),
        409,
        "a registered harness is sealed: {}",
        refused.body()
    );
}

#[tokio::test]
async fn goal_limits_are_refused_before_the_one_shot_and_never_echoed() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let route = entry_route(&info.entry_id, SUFFIX_CAPTURE_GOAL);
    let marker = "NEVER-ECHOED";
    let mut not_utf8 = vec![0xff, 0xfe];
    not_utf8.extend_from_slice(marker.as_bytes());
    for (case, bytes) in [
        ("empty", Vec::new()),
        (
            "over the goal bound",
            vec![b'a'; goalcup_protocol::capture::MAX_GOAL_BYTES + 1],
        ),
        ("not UTF-8", not_utf8),
    ] {
        let refused = rig.goal(&info.entry_id, &bytes).await;
        assert_eq!(refused.status(), 400, "{case}: {}", refused.body());
        assert!(
            !refused.body().contains(marker),
            "{case}: {}",
            refused.body()
        );
    }
    let malformed = rig
        .client
        .post_body(
            &route,
            &serde_json::json!({ "bytes": [1], "extra": marker }),
        )
        .await;
    assert_eq!(malformed.status(), 400, "{}", malformed.body());
    assert!(!malformed.body().contains(marker), "{}", malformed.body());
    let oversized = rig
        .client
        .post_body(
            &route,
            &serde_json::json!({ "bytes": vec![1u8; goalcup_protocol::capture::MAX_INGEST_FRAME_BYTES / 2] }),
        )
        .await;
    assert_eq!(oversized.status(), 400, "{}", oversized.body());

    rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
}

#[tokio::test]
async fn launch_refusals() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let unknown = StreamKey {
        producer: fresh_id(),
        stream: 1,
    };
    let early = rig.launch(&info.entry_id, &launch_request(&unknown)).await;
    assert_eq!(early.status(), 409, "no goal yet: {}", early.body());

    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    let wrong = rig.launch(&info.entry_id, &launch_request(&unknown)).await;
    assert_eq!(wrong.status(), 409, "another stream: {}", wrong.body());
    let mut control = launch_request(&goal);
    control.continuation = "exec\nsecond line".to_string();
    let malformed = rig.launch(&info.entry_id, &control).await;
    assert_eq!(malformed.status(), 400, "{}", malformed.body());
    let mut long = launch_request(&goal);
    long.harness_version = "9".repeat(goalcup_protocol::capture::MAX_METADATA_STRING_BYTES + 1);
    assert_eq!(rig.launch(&info.entry_id, &long).await.status(), 400);

    let launched = rig.launch(&info.entry_id, &launch_request(&goal)).await;
    assert_eq!(launched.status(), 204, "{}", launched.body());
    let again = rig.launch(&info.entry_id, &launch_request(&goal)).await;
    assert_eq!(again.status(), 204, "idempotent launch: {}", again.body());
    let launches = rig
        .records(&info.entry_id)
        .iter()
        .filter(|record| matches!(record, CaptureRecord::Launch { .. }))
        .count();
    assert_eq!(launches, 1);
}

fn connection(id: &str, event: ConnectionEvent) -> FrameEvent {
    FrameEvent::Connection {
        connection: id.to_string(),
        event,
    }
}

fn opened(already_open: bool) -> ConnectionEvent {
    ConnectionEvent::Open {
        alpn: None,
        already_open,
    }
}

fn request_open(connection: &str) -> FrameEvent {
    FrameEvent::Open {
        source: CaptureSource::WireRequest,
        metadata: CaptureMeta {
            wire: Some(WireIdentity {
                connection: connection.to_string(),
                exchange: 1,
                message_index: None,
            }),
            label: "request".to_string(),
            protocol_hint: None,
            method: Some("POST".to_string()),
            host: Some("api.example.test".to_string()),
            path: Some("/v1/responses".to_string()),
            status: None,
            content_type: Some("application/json".to_string()),
        },
    }
}

fn bytes(data: &[u8]) -> FrameEvent {
    FrameEvent::Bytes {
        offset: 0,
        data: data.to_vec(),
    }
}

/// A wire exchange that ends cleanly: Connection, Open, Bytes, End, Connection close.
fn clean_exchange(wire: &mut Wire, id: &str) {
    wire.send(0, connection(id, opened(false)));
    wire.send(1, request_open(id));
    wire.send(1, bytes(br#"{"input":"hello"}"#));
    wire.send(
        1,
        FrameEvent::End {
            reason: EndReason::Complete,
        },
    );
    let close = ConnectionEvent::Close {
        outcome: ConnectionOutcome::Complete,
    };
    wire.send(0, connection(id, close));
}

/// One epoch of `info`'s own producer: a clean exchange, then its close.
async fn clean_epoch(info: &CaptureWorkspaceInfo) {
    let wired = info.clone();
    tokio::task::spawn_blocking(move || {
        let mut wire = Wire::connect(&wired, &wired.producer_id);
        clean_exchange(&mut wire, &fresh_id());
        wire.close();
    })
    .await
    .unwrap();
}

fn closed_by(records: &[CaptureRecord], producer: &str) -> Option<Option<u64>> {
    records.iter().find_map(|record| match record {
        CaptureRecord::ProducerClosed {
            producer: p,
            final_seq,
            ..
        } if p == producer => Some(*final_seq),
        _ => None,
    })
}

fn position(records: &[CaptureRecord], matches: impl Fn(&CaptureRecord) -> bool) -> usize {
    records
        .iter()
        .position(matches)
        .expect("the record is in the chain")
}

fn names_producer(record: &CaptureRecord, producer: &str) -> bool {
    match record {
        CaptureRecord::Open { stream, .. }
        | CaptureRecord::Segment { stream, .. }
        | CaptureRecord::End { stream, .. }
        | CaptureRecord::Gap {
            stream: Some(stream),
            ..
        } => stream.producer == producer,
        CaptureRecord::Connection { producer: p, .. }
        | CaptureRecord::ProducerClosed { producer: p, .. } => p == producer,
        _ => false,
    }
}

fn wire_gap(records: &[CaptureRecord], producer: &str, reason: GapReason) -> bool {
    records.iter().any(|record| {
        matches!(record, CaptureRecord::Gap { stream: Some(s), reason: r, .. }
            if s.producer == producer && *r == reason)
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wire_native_and_goal_share_one_chain() {
    let (rig, native_dir, _connectors) = native_rig().await;
    let info = rig.created("capture-dev").await;
    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    rig.launch(&info.entry_id, &launch_request(&goal)).await;
    std::fs::write(
        native_dir.join("rollout-fixture-test-session.jsonl"),
        b"{\"type\":\"native\"}\n",
    )
    .unwrap();
    clean_epoch(&info).await;

    let producer = info.producer_id.clone();
    let native = |record: &CaptureRecord| {
        matches!(
            record,
            CaptureRecord::Open {
                source: CaptureSource::Native,
                ..
            }
        )
    };
    let records = rig
        .records_when(&info.entry_id, |records| {
            closed_by(records, &producer).is_some() && records.iter().any(native)
        })
        .await;
    let sources: Vec<_> = records
        .iter()
        .filter_map(|record| match record {
            CaptureRecord::Open {
                source, metadata, ..
            } => Some((*source, metadata.protocol_hint.clone())),
            _ => None,
        })
        .collect();
    for expected in [
        (CaptureSource::Goal, None),
        (CaptureSource::WireRequest, None),
        (CaptureSource::Native, Some("codex-jsonl".to_string())),
    ] {
        assert!(sources.contains(&expected), "{expected:?} in {sources:?}");
    }
    let status = rig.status(&info.entry_id).await.payload;
    assert_eq!(
        (status.producers_consumed, status.admissions_refused),
        (Some(1), Some(0))
    );
}

/// D53 follow-up: the launch reads the harness build (two files of up to 512 MiB) before it
/// chains `Launch`, and the harness is already running. Its traffic is drained into the chain
/// under its own producer while the read is held: more than the sink's whole queue
/// (`MAX_CAPTURE_QUEUE_BYTES`) goes through, which traffic held until `Launch` could not do
/// without a `QueueFull` loss. Only the status lags: it reads `Starting` until the launch is
/// chained.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slow_harness_hash_loses_and_delays_no_traffic() {
    use goalcup_protocol::capture::{MAX_CAPTURE_QUEUE_BYTES, MAX_SEGMENT_BYTES};
    const FRAME: usize = MAX_SEGMENT_BYTES;
    const BATCH: usize = 32;
    let (rig, _native, _connectors) = native_rig().await;
    let workspace = rig.workspace(&format!("slow-hash-{}", &fresh_id()[..12]));
    let created = rig.create("capture-dev", &workspace, 64 << 20).await;
    assert_eq!(created.status(), 200, "create: {}", created.body());
    let info: CaptureWorkspaceInfo = created.json().unwrap();
    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    let harness = Child::sleeping();
    let registration = harness.registration(info.process_generation);
    assert_eq!(
        rig.register(&info.entry_id, &registration).await.status(),
        204
    );
    rig.daemon.hold_capture_harness_build(&info.entry_id, true);
    // A failed assertion below must not leave the launch parked.
    struct Release<'a>(&'a goalcup_daemon::runtime::Daemon, String);
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            self.0.hold_capture_harness_build(&self.1, false);
        }
    }
    let release = Release(&rig.daemon, info.entry_id.clone());

    let launch_request = launch_request(&goal);
    let launch = rig.launch(&info.entry_id, &launch_request);
    tokio::pin!(launch);
    let producer = info.producer_id.clone();
    let chained = |records: &[CaptureRecord]| -> u64 {
        records
            .iter()
            .filter_map(|record| match record {
                CaptureRecord::Segment {
                    stream, source_len, ..
                } if stream.producer == producer => Some(*source_len),
                _ => None,
            })
            .sum()
    };
    let line = b"the quick brown fox jumps over the lazy dog\n";
    let payload: Vec<u8> = line.iter().copied().cycle().take(FRAME).collect();
    let connection_id = fresh_id();
    let traffic = async {
        let wired = info.clone();
        let id = connection_id.clone();
        let mut wire = tokio::task::spawn_blocking(move || {
            let mut wire = Wire::connect(&wired, &wired.producer_id);
            wire.send(0, connection(&id, opened(false)));
            wire.send(1, request_open(&id));
            wire
        })
        .await
        .unwrap();
        let mut sent = 0u64;
        // Each batch is chained before the next is sent, all while the launch is held.
        while sent <= MAX_CAPTURE_QUEUE_BYTES as u64 + (FRAME * BATCH) as u64 {
            let data = payload.clone();
            let from = sent;
            wire = tokio::task::spawn_blocking(move || {
                for n in 0..BATCH as u64 {
                    let offset = from + n * FRAME as u64;
                    wire.send(
                        1,
                        FrameEvent::Bytes {
                            offset,
                            data: data.clone(),
                        },
                    );
                }
                wire
            })
            .await
            .unwrap();
            sent += (FRAME * BATCH) as u64;
            rig.records_when(&info.entry_id, |records| chained(records) >= sent)
                .await;
        }
        (wire, sent)
    };
    let (wire, sent) = tokio::select! {
        answered = &mut launch => panic!("the launch answered while held: {}", answered.status()),
        done = traffic => done,
    };
    assert!(sent > MAX_CAPTURE_QUEUE_BYTES as u64);

    // Still held: the launch is not chained, and only the status says so.
    let held = rig.records(&info.entry_id);
    assert!(
        !held
            .iter()
            .any(|r| matches!(r, CaptureRecord::Launch { .. }))
    );
    assert_eq!(chained(&held), sent);
    let status = rig.status(&info.entry_id).await.payload;
    let capture = status.capture.unwrap();
    assert_eq!(
        (capture.run, capture.capture, capture.gap_count),
        (RunState::Starting, CaptureState::Starting, 0)
    );
    assert_eq!(status.admissions_refused, Some(0));
    tokio::select! {
        answered = &mut launch => panic!("the launch answered while held: {}", answered.status()),
        _ = tokio::time::sleep(std::time::Duration::from_millis(300)) => {}
    }

    drop(release);
    let launched = launch.await;
    assert_eq!(launched.status(), 204, "{}", launched.body());
    let capture = rig.status(&info.entry_id).await.payload.capture.unwrap();
    assert_eq!(
        (capture.run, capture.capture, capture.gap_count),
        (RunState::Running, CaptureState::Recording, 0)
    );

    tokio::task::spawn_blocking(move || {
        let mut wire = wire;
        wire.send(
            1,
            FrameEvent::End {
                reason: EndReason::Complete,
            },
        );
        let close = ConnectionEvent::Close {
            outcome: ConnectionOutcome::Complete,
        };
        wire.send(0, connection(&connection_id, close));
        wire.close();
    })
    .await
    .unwrap();
    rig.records_when(&info.entry_id, |records| {
        closed_by(records, &producer).is_some()
    })
    .await;
    let sealed = rig.seal(&info.entry_id, None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());

    let records = rig.records(&info.entry_id);
    assert!(
        !records
            .iter()
            .any(|r| matches!(r, CaptureRecord::Gap { .. })),
        "nothing was lost: {:#?}",
        records
            .iter()
            .filter(|r| matches!(r, CaptureRecord::Gap { .. }))
            .collect::<Vec<_>>()
    );
    assert_eq!(chained(&records), sent);
    let launch_at = position(&records, |r| matches!(r, CaptureRecord::Launch { .. }));
    let last_held = records
        .iter()
        .rposition(|r| {
            matches!(r, CaptureRecord::Segment { stream, source_offset, .. }
            if stream.producer == producer && *source_offset < sent)
        })
        .unwrap();
    assert!(
        last_held < launch_at,
        "the held traffic is chained before the launch"
    );
    // Every wire record names the one producer that sent it.
    let wire_records = records.iter().filter(|r| {
        matches!(
            r,
            CaptureRecord::Connection { .. }
                | CaptureRecord::Open {
                    source: CaptureSource::WireRequest,
                    ..
                }
        )
    });
    assert!(
        wire_records
            .into_iter()
            .all(|r| names_producer(r, &producer))
    );
    let Some(CaptureRecord::Launch { harness_build, .. }) = records.get(launch_at) else {
        unreachable!()
    };
    assert!(harness_build.is_some(), "the build is still recorded");

    // End to end with the built `goalcup-verify` beside the test binaries, or the one
    // `GOALCUP_VERIFY_BIN` names: the trace verifies.
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("goalcup-verify")))
        .filter(|path| path.exists());
    if let Some(bin) = std::env::var_os("GOALCUP_VERIFY_BIN").or(sibling.map(Into::into)) {
        let out = std::process::Command::new(bin)
            .arg("--capture-integrity")
            .arg(rig.trace_dir(&info.entry_id))
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn producer_close_after_all_frames_is_clean() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    clean_epoch(&info).await;
    let producer = info.producer_id.clone();
    let records = rig
        .records_when(&info.entry_id, |records| {
            closed_by(records, &producer).is_some()
        })
        .await;
    // Connection 0, Open 1, Bytes 2, End 3, Connection close 4.
    assert_eq!(closed_by(&records, &producer), Some(Some(4)));
    let closes = records
        .iter()
        .filter(|record| names_producer(record, &producer))
        .filter(|record| matches!(record, CaptureRecord::ProducerClosed { .. }))
        .count();
    assert_eq!(closes, 1);
    assert!(
        !records
            .iter()
            .any(|r| matches!(r, CaptureRecord::Gap { .. }))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn eof_without_producer_close_marks_unknown_tail() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let wired = info.clone();
    tokio::task::spawn_blocking(move || {
        let mut wire = Wire::connect(&wired, &wired.producer_id);
        let id = fresh_id();
        wire.send(0, connection(&id, opened(false)));
        wire.send(1, request_open(&id));
        wire.send(1, bytes(b"partial"));
        wire.hang_up();
    })
    .await
    .unwrap();
    let producer = info.producer_id.clone();
    let records = rig
        .records_when(&info.entry_id, |records| {
            wire_gap(records, &producer, GapReason::IngestLost)
        })
        .await;
    assert_eq!(
        closed_by(&records, &producer),
        None,
        "EOF is never a clean close"
    );
    let capture = rig.status(&info.entry_id).await.payload.capture.unwrap();
    assert_eq!(capture.capture, CaptureState::Incomplete);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_cannot_hide_missing_sequence() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let wired = info.clone();
    tokio::task::spawn_blocking(move || {
        let mut wire = Wire::connect(&wired, &wired.producer_id);
        clean_exchange(&mut wire, &fresh_id());
        // Frames 0..=4 were sent; a Close claiming 6 would hide two that never arrived.
        wire.close_claiming(Some(6));
    })
    .await
    .unwrap();
    let producer = info.producer_id.clone();
    let records = rig
        .records_when(&info.entry_id, |records| {
            records
                .iter()
                .any(|record| matches!(record, CaptureRecord::Gap { stream: None, .. }))
        })
        .await;
    assert_eq!(closed_by(&records, &producer), None);
    let capture = rig.status(&info.entry_id).await.payload.capture.unwrap();
    assert_eq!(capture.capture, CaptureState::Incomplete);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recorder_recovery_preserves_gap_without_assistance() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let fresh = fresh_id();
    let (wired, recovered) = (info.clone(), fresh.clone());
    tokio::task::spawn_blocking(move || {
        let id = fresh_id();
        let mut lost = Wire::connect(&wired, &wired.producer_id);
        lost.send(0, connection(&id, opened(false)));
        lost.send(1, request_open(&id));
        lost.send(1, bytes(b"half a request"));
        lost.hang_up();
        let mut wire = Wire::connect(&wired, &recovered);
        let recovery = FrameEvent::Recovery {
            previous_producer: wired.producer_id.clone(),
            reason: GapReason::IngestLost,
        };
        wire.send(0, recovery);
        wire.send(0, connection(&id, opened(true)));
        wire.close();
    })
    .await
    .unwrap();
    let records = rig
        .records_when(&info.entry_id, |records| {
            closed_by(records, &fresh).is_some()
        })
        .await;
    let lost_gap = position(
        &records,
        |record| matches!(record, CaptureRecord::Gap { stream: Some(s), .. } if s.producer == info.producer_id),
    );
    let recovery = position(
        &records,
        |record| matches!(record, CaptureRecord::Recovery { producer, .. } if *producer == fresh),
    );
    assert!(
        lost_gap < recovery,
        "the old epoch's loss precedes its Recovery"
    );
    assert!(
        !records
            .iter()
            .any(|r| matches!(r, CaptureRecord::Assistance { .. }))
    );
    let capture = rig.status(&info.entry_id).await.payload.capture.unwrap();
    assert!(!capture.assistance_observed);
    assert_eq!(capture.capture, CaptureState::Incomplete, "the gap stays");
}

/// The native producer's stream key, once its Open is chained.
fn native_producer(records: &[CaptureRecord]) -> Option<String> {
    records.iter().find_map(|record| match record {
        CaptureRecord::Open {
            stream,
            source: CaptureSource::Native,
            ..
        } => Some(stream.producer.clone()),
        _ => None,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orphan_lineage_recovery_interrupts_superseded_redactors() {
    let (rig, native_dir, _connectors) = native_rig().await;
    let info = rig.created("capture-dev").await;
    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    rig.launch(&info.entry_id, &launch_request(&goal)).await;
    let session = native_dir.join("rollout-fixture-test-session.jsonl");
    std::fs::write(&session, b"{\"type\":\"native\"}\n").unwrap();
    let id = fresh_id();
    // P holds an undecided secret prefix in its redactor and stays connected.
    let (wired, lineage) = (info.clone(), id.clone());
    let p = tokio::task::spawn_blocking(move || {
        let mut wire = Wire::connect(&wired, &wired.producer_id);
        wire.send(0, connection(&lineage, opened(false)));
        wire.send(1, request_open(&lineage));
        wire.send(1, bytes(b"authorization sk-ant-"));
        wire
    })
    .await
    .unwrap();
    let producer = info.producer_id.clone();
    let records = rig
        .records_when(&info.entry_id, |records| {
            native_producer(records).is_some()
                && records.iter().any(|record| {
                    matches!(record, CaptureRecord::Open { stream, .. } if stream.producer == producer)
                })
        })
        .await;
    let native = native_producer(&records).unwrap();
    // Q is admitted and lost before its first frame; R's Recovery names Q.
    let (q, r) = (fresh_id(), fresh_id());
    let (wired, orphan, fresh) = (info.clone(), q.clone(), r.clone());
    tokio::task::spawn_blocking(move || {
        Wire::connect(&wired, &orphan).hang_up();
        let mut wire = Wire::connect(&wired, &fresh);
        let recovery = FrameEvent::Recovery {
            previous_producer: orphan,
            reason: GapReason::IngestLost,
        };
        wire.send(0, recovery);
        wire.send(0, connection(&id, opened(true)));
        wire.close();
        p.hang_up();
    })
    .await
    .unwrap();
    let is_recovery = |record: &CaptureRecord| matches!(record, CaptureRecord::Recovery { producer, .. } if *producer == r);
    rig.records_when(&info.entry_id, |records| closed_by(records, &r).is_some())
        .await;
    // The native producer is not wire: the Recovery does not supersede it, and it goes on.
    let mut more = std::fs::OpenOptions::new()
        .append(true)
        .open(&session)
        .unwrap();
    std::io::Write::write_all(&mut more, b"{\"type\":\"native\",\"n\":2}\n").unwrap();
    let native_after = |records: &[CaptureRecord]| {
        records.iter().position(is_recovery).is_some_and(|at| {
            records[at..].iter().any(|record| {
                matches!(record, CaptureRecord::Segment { stream, .. } if stream.producer == native)
            })
        })
    };
    rig.records_when(&info.entry_id, native_after).await;
    let sealed = rig.seal(&info.entry_id, None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());

    let records = rig.records(&info.entry_id);
    let recovery = position(&records, is_recovery);
    let superseded = position(
        &records,
        |record| matches!(record, CaptureRecord::Gap { stream: Some(s), .. } if s.producer == producer),
    );
    assert!(
        superseded < recovery,
        "P's redactor is interrupted before the Recovery that supersedes it"
    );
    assert!(
        !records[recovery..]
            .iter()
            .any(|record| names_producer(record, &producer)),
        "nothing of a superseded producer follows its Recovery"
    );
    let raw = std::fs::read_to_string(rig.trace_dir(&info.entry_id).join("chain.jsonl")).unwrap();
    assert!(
        !raw.contains("sk-ant-"),
        "the held prefix is discarded, never released"
    );
    assert!(
        !records.iter().any(|record| {
            matches!(record, CaptureRecord::Gap { stream: Some(s), .. } if s.producer == native)
        }),
        "the native producer lost nothing"
    );
    let closed = position(
        &records,
        |record| matches!(record, CaptureRecord::ProducerClosed { producer, .. } if *producer == native),
    );
    let finish = position(&records, |r| matches!(r, CaptureRecord::LocalFinish { .. }));
    assert!(
        closed < finish,
        "the native producer closes cleanly at the finish"
    );
}

#[tokio::test]
async fn proxy_counters_become_streamless_gaps() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let counters = ProxyCounters {
        tunneled_fail_open: 1,
        opaque_fallbacks: 2,
        capture_table_full: 3,
        cut_unsettled: 0,
        unterminated: 4,
        sink_refusals: 5,
    };
    let finished = rig.seal(&info.entry_id, Some(counters)).await;
    assert_eq!(finished.status(), 200, "{}", finished.body());
    let records = rig.records(&info.entry_id);
    assert!(matches!(
        records.last(),
        Some(CaptureRecord::LocalFinish { .. })
    ));
    let gaps: Vec<_> = records
        .into_iter()
        .filter_map(|record| match record {
            CaptureRecord::Gap {
                stream: None,
                reason,
                lost_bytes: None,
                ..
            } => Some(reason),
            _ => None,
        })
        .collect();
    assert_eq!(
        gaps,
        vec![
            GapReason::DecodeFailed,
            GapReason::DecodeFailed,
            GapReason::LimitExceeded,
            GapReason::ShutdownDeadline,
        ],
        "one per non-zero counter; sink refusals are chained already"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_finish_without_service_is_complete() {
    let (rig, _native, _connectors) = native_rig().await;
    let info = rig.created("capture-dev").await;
    let harness = Child::sleeping();
    let registered = rig
        .register(
            &info.entry_id,
            &harness.registration(info.process_generation),
        )
        .await;
    assert_eq!(registered.status(), 204, "{}", registered.body());
    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    rig.launch(&info.entry_id, &launch_request(&goal)).await;
    clean_epoch(&info).await;

    let sealed = rig
        .seal(&info.entry_id, Some(ProxyCounters::default()))
        .await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let report: CaptureFinishReport = sealed.json().unwrap();
    assert_eq!(report.seal, LocalSealOutcome::SealWritten);
    assert!(
        matches!(report.termination, TerminationOutcomeV1::Confirmed { .. }),
        "{:?}",
        report.termination
    );
    let records = rig.records(&info.entry_id);
    assert!(closed_by(&records, &info.producer_id).is_some());
    assert_eq!(local_finishes(&records), 1);
    assert!(matches!(
        records.last(),
        Some(CaptureRecord::LocalFinish { .. })
    ));
    assert_eq!(
        report.terminal_seq,
        Some(rig.chain(&info.entry_id).len() as u64 - 1)
    );
    let trace = rig.trace_dir(&info.entry_id);
    assert!(trace.join("seal.json").exists());
    assert!(!trace.join("receipt.json").exists());
    assert_eq!(rig.upload_dials(), 0, "no server is ever asked");
    let status = rig.status(&info.entry_id).await;
    status.verify_with(&info.run_public_key).unwrap();
    let capture = status.payload.capture.unwrap();
    assert_eq!(capture.run, RunState::Sealed);
    assert_eq!(
        capture.capture,
        CaptureState::Recording,
        "no gap was recorded"
    );
    assert_eq!(status.payload.seconds_remaining, None);
    assert_eq!(status.payload.deadline_at, None);
}

#[tokio::test]
async fn local_finish_binds_actual_seal_manifest() {
    let rig = Rig::start().await;
    let workspace = rig.workspace("work");
    let info: CaptureWorkspaceInfo = rig
        .create("codex-cli", &workspace, RESERVE)
        .await
        .json()
        .unwrap();
    std::fs::write(workspace.join("hello.txt"), b"hello\n").unwrap();
    let sealed = rig.seal(&info.entry_id, None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let seal = std::fs::read(rig.trace_dir(&info.entry_id).join("seal.json")).unwrap();
    let manifest: goalcup_protocol::seal::SealManifest = serde_json::from_slice(&seal).unwrap();
    let file = manifest
        .files
        .iter()
        .find(|f| f.path == "hello.txt")
        .expect("the file the run wrote is sealed");
    assert_eq!(file.sha256, hex::encode(Sha256::digest(b"hello\n")));
    let Some(CaptureRecord::LocalFinish {
        seal_manifest_sha256,
        ..
    }) = rig.records(&info.entry_id).pop()
    else {
        panic!("the chain ends with the local finish");
    };
    assert_eq!(seal_manifest_sha256, hex::encode(Sha256::digest(&seal)));

    // The finished entry keeps its seal: a late goal and an abandon are both refused.
    let late = rig.goal(&info.entry_id, b"write hello.txt").await;
    assert_eq!(late.status(), 409, "{}", late.body());
    let abandoned = rig.abandon(&info.entry_id).await;
    assert_eq!(abandoned.status(), 409, "{}", abandoned.body());
    let kept = std::fs::read(rig.trace_dir(&info.entry_id).join("seal.json")).unwrap();
    assert_eq!(kept, seal);
    let capture = rig.status(&info.entry_id).await.payload.capture.unwrap();
    assert_ne!(
        capture.capture,
        CaptureState::Failed,
        "the chain is as the finish left it"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finish_closes_capture_before_terminal() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let wired = info.clone();
    tokio::task::spawn_blocking(move || {
        let mut wire = Wire::connect(&wired, &wired.producer_id);
        let id = fresh_id();
        wire.send(0, connection(&id, opened(false)));
        wire.send(1, request_open(&id));
        wire.send(1, bytes(b"cut off"));
        wire.hang_up();
    })
    .await
    .unwrap();
    // The legacy seal, with no capture request, is refused before anything happens.
    let bare = rig
        .client
        .post(&entry_route(&info.entry_id, SUFFIX_SEAL))
        .await;
    assert_eq!(bare.status(), 400, "{}", bare.body());
    let sealed = rig.seal(&info.entry_id, None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let records = rig.records(&info.entry_id);
    let finish = position(&records, |r| matches!(r, CaptureRecord::LocalFinish { .. }));
    assert_eq!(finish, records.len() - 1);
    assert!(
        wire_gap(&records[..finish], &info.producer_id, GapReason::IngestLost),
        "the lost producer's gap is chained before the terminal"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_frame_cannot_append_after_local_finish() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let sealed = rig.seal(&info.entry_id, None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let chain = rig.trace_dir(&info.entry_id).join("chain.jsonl");
    let before = std::fs::read(&chain).unwrap();
    // The receiver refuses the late handshake and closes the socket before reading a frame;
    // `hang_up` returns once it has.
    let wired = info.clone();
    tokio::task::spawn_blocking(move || Wire::connect(&wired, &fresh_id()).hang_up())
        .await
        .unwrap();
    let status = rig.status(&info.entry_id).await.payload;
    assert_eq!(
        status.producers_consumed, None,
        "the finished entry holds no ingest grant"
    );
    assert_eq!(
        std::fs::read(&chain).unwrap(),
        before,
        "the chain is closed"
    );
}

#[tokio::test]
async fn closed_restart_keeps_capture_status_signed() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    rig.seal(&info.entry_id, None).await;
    let rig = rig.restart().await;
    let status = rig.status(&info.entry_id).await;
    status.verify_with(&info.run_public_key).unwrap();
    assert!(status.payload.sealed_at.is_some());
    assert_eq!(status.payload.seconds_remaining, None);
    assert_eq!(status.payload.capture.unwrap().run, RunState::Sealed);
    assert!(
        rig.daemon
            .entries()
            .unwrap()
            .iter()
            .any(|e| e.entry_id == info.entry_id)
    );
}

#[tokio::test]
async fn restart_preserves_local_terminal_without_receipt() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let first: CaptureFinishReport = rig.seal(&info.entry_id, None).await.json().unwrap();
    let rig = rig.restart().await;
    let again = rig.seal(&info.entry_id, None).await;
    assert_eq!(again.status(), 200, "{}", again.body());
    let again: CaptureFinishReport = again.json().unwrap();
    assert_eq!(again, first, "a retry observes the committed finish");
    assert_eq!(local_finishes(&rig.records(&info.entry_id)), 1);
    assert!(!rig.trace_dir(&info.entry_id).join("receipt.json").exists());
}

/// Once its seal is written the entry lets go of its chain: the daemon holds no descriptor on
/// it, and the entry still answers status, a retried seal and its refusals.
#[tokio::test]
async fn a_sealed_entry_lets_go_of_its_chain() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let chain = rig.trace_dir(&info.entry_id).join("chain.jsonl");
    assert!(descriptors_on(&chain) > 0, "a live entry writes its chain");
    let sealed = rig.seal(&info.entry_id, None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let report: CaptureFinishReport = sealed.json().unwrap();
    assert_eq!(report.seal, LocalSealOutcome::SealWritten);

    assert_eq!(descriptors_on(&chain), 0, "the finished chain is closed");
    let status = rig.status(&info.entry_id).await;
    status.verify_with(&info.run_public_key).unwrap();
    assert_eq!(status.payload.capture.unwrap().run, RunState::Sealed);
    let again: CaptureFinishReport = rig.seal(&info.entry_id, None).await.json().unwrap();
    assert_eq!(again, report);
    let late = rig.goal(&info.entry_id, b"write hello.txt").await;
    assert_eq!(late.status(), 409, "{}", late.body());
    assert!(
        rig.daemon
            .entries()
            .unwrap()
            .iter()
            .any(|e| e.entry_id == info.entry_id && e.sealed_at.is_some())
    );
}

/// A restart does not reopen sealed chains: each sealed entry's status is the one its finish
/// recorded, even over a chain damaged since, which a reopen would refuse, and the daemon
/// holds no descriptor on any of them.
#[tokio::test]
async fn a_restart_leaves_sealed_chains_unopened() {
    let rig = Rig::start().await;
    let mut sealed = Vec::new();
    for _ in 0..3 {
        let info = rig.created("codex-cli").await;
        let counters = ProxyCounters {
            capture_table_full: 1,
            ..ProxyCounters::default()
        };
        let finished = rig.seal(&info.entry_id, Some(counters)).await;
        assert_eq!(finished.status(), 200, "{}", finished.body());
        let capture = rig.status(&info.entry_id).await.payload.capture.unwrap();
        assert_eq!(capture.gap_count, 1);
        let chain = rig.trace_dir(&info.entry_id).join("chain.jsonl");
        let mut torn = std::fs::OpenOptions::new()
            .append(true)
            .open(&chain)
            .unwrap();
        std::io::Write::write_all(&mut torn, b"{\"protocol_version\":5,").unwrap();
        sealed.push((info, capture, chain));
    }

    let rig = rig.restart().await;
    for (info, before, chain) in &sealed {
        assert_eq!(descriptors_on(chain), 0, "{}", info.entry_id);
        let status = rig.status(&info.entry_id).await;
        status.verify_with(&info.run_public_key).unwrap();
        assert_eq!(status.payload.capture.as_ref(), Some(before));
    }
    let cycles = rig.daemon.cycle_errors();
    assert_eq!(
        (cycles.tick, cycles.last),
        (0, None),
        "nothing was reopened"
    );
    let listed = rig.daemon.entries().unwrap();
    assert_eq!(listed.len(), sealed.len());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restart_keeps_the_gap_history_through_the_seal() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    let launched = rig.launch(&info.entry_id, &launch_request(&goal)).await;
    assert_eq!(launched.status(), 204, "{}", launched.body());
    // A producer is live, with a stream open, when the daemon stops: the stop cuts it.
    let wired = info.clone();
    let live = tokio::task::spawn_blocking(move || {
        let mut wire = Wire::connect(&wired, &wired.producer_id);
        let id = fresh_id();
        wire.send(0, connection(&id, opened(false)));
        wire.send(1, request_open(&id));
        wire
    })
    .await
    .unwrap();
    let producer = info.producer_id.clone();
    rig.records_when(&info.entry_id, |records| {
        records
            .iter()
            .any(|record| names_producer(record, &producer))
    })
    .await;
    let rig = rig.restart().await;
    tokio::task::spawn_blocking(move || live.hang_up())
        .await
        .unwrap();
    let chained = rig
        .records(&info.entry_id)
        .iter()
        .filter(|record| matches!(record, CaptureRecord::Gap { .. }))
        .count() as u64;
    assert!(chained >= 1, "the stop chained the cut producer's loss");

    let reopened = rig.status(&info.entry_id).await.payload.capture.unwrap();
    assert_eq!(reopened.gap_count, chained);
    assert_eq!(reopened.capture, CaptureState::Incomplete);
    let sealed = rig.seal(&info.entry_id, None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let report: CaptureFinishReport = sealed.json().unwrap();
    assert_eq!(report.seal, LocalSealOutcome::SealWritten);
    let finished = rig.status(&info.entry_id).await.payload.capture.unwrap();
    assert_eq!(finished.run, RunState::Sealed);
    assert_eq!(finished.gap_count, chained);
    assert_eq!(finished.capture, CaptureState::Incomplete);
}

#[tokio::test]
async fn a_failed_local_finish_is_reported_and_never_reads_finished() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    // `seal.json` is created absent-only: a file already at its name fails the local finish
    // after its terminal record is chained.
    std::fs::write(rig.trace_dir(&info.entry_id).join("seal.json"), b"{}").unwrap();
    let sealed = rig.seal(&info.entry_id, None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let report: CaptureFinishReport = sealed.json().unwrap();
    assert_eq!(report.seal, LocalSealOutcome::SealWriteFailed);
    let again: CaptureFinishReport = rig.seal(&info.entry_id, None).await.json().unwrap();
    assert_eq!(
        again, report,
        "the failure is answered again, never retried"
    );

    let failed = |status: SignedStatus| {
        let capture = status.payload.capture.unwrap();
        assert_eq!(capture.capture, CaptureState::Failed);
        assert_ne!(capture.run, RunState::Sealed);
        assert!(
            !status.payload.detail.contains("finished"),
            "{}",
            status.payload.detail
        );
    };
    failed(rig.status(&info.entry_id).await);
    let rig = rig.restart().await;
    failed(rig.status(&info.entry_id).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_session_still_captures_and_finishes_locally() {
    let (rig, native_dir, _connectors) = controlled_native_rig().await;
    let info = rig.created("capture-dev").await;
    // No session or thread identity is ever given to the daemon.
    std::fs::write(native_dir.join("unknown.jsonl"), b"{\"type\":\"native\"}\n").unwrap();
    rig.records_when(&info.entry_id, |records| native_producer(records).is_some())
        .await;
    let sealed = rig.seal(&info.entry_id, None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let records = rig.records(&info.entry_id);
    let native = native_producer(&records).unwrap();
    let closed = position(
        &records,
        |r| matches!(r, CaptureRecord::ProducerClosed { producer, .. } if *producer == native),
    );
    let finish = position(&records, |r| matches!(r, CaptureRecord::LocalFinish { .. }));
    assert!(closed < finish, "native capture closes before the terminal");
}

// ---------------------------------------------------------------------------------------------
// D48: what the daemon does with V1 rows already in its registry. The V1 runtime that
// recovered them was removed, so a row that still owes work is refused.

/// A V1 (non-capture) row with the given state, chain head and acknowledgement watermark.
fn plant_v1_row(
    registry: &goalcup_daemon::registry::Registry,
    root: &std::path::Path,
    id: &str,
    state: goalcup_daemon::registry::EntryState,
    chain_seq: Option<u64>,
    last_ack_seq: Option<u64>,
) {
    let mut entry = support::entry_in(
        registry,
        root,
        id,
        "claude-code-cli",
        &root.join("workspaces").join(id).display().to_string(),
    );
    entry.state = state;
    entry.chain_seq = chain_seq;
    entry.last_ack_seq = last_ack_seq;
    registry.put(&entry).expect("the V1 row is stored");
}

#[tokio::test]
async fn a_sealed_v1_row_does_not_block_a_default_daemon_and_a_capture_entry_beside_it_finishes() {
    let (rig, _native, _connectors) = native_rig().await;
    let info = rig.created("capture-dev").await;
    let harness = Child::sleeping();
    let registered = rig
        .register(
            &info.entry_id,
            &harness.registration(info.process_generation),
        )
        .await;
    assert_eq!(registered.status(), 204, "{}", registered.body());
    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    rig.launch(&info.entry_id, &launch_request(&goal)).await;
    let root = rig.root.clone();
    let rig = rig
        .restart_planting(|registry| {
            use goalcup_daemon::registry::EntryState;
            plant_v1_row(
                registry,
                &root,
                "v1-sealed",
                EntryState::Sealed,
                Some(7),
                Some(7),
            );
            plant_v1_row(
                registry,
                &root,
                "v1-refused",
                EntryState::Refused,
                None,
                None,
            );
        })
        .await
        .expect("inert V1 rows leave a default daemon free to start");

    // The capture entry reopens and seals exactly as it would beside no V1 row at all.
    let sealed = rig
        .seal(&info.entry_id, Some(ProxyCounters::default()))
        .await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let report: CaptureFinishReport = sealed.json().unwrap();
    assert_eq!(report.seal, LocalSealOutcome::SealWritten);
    assert!(rig.trace_dir(&info.entry_id).join("seal.json").exists());
    let status = rig.status(&info.entry_id).await;
    status.verify_with(&info.run_public_key).unwrap();
    assert_eq!(status.payload.capture.unwrap().run, RunState::Sealed);
    // The V1 rows are inert evidence: this build neither lists nor changes them.
    assert!(
        rig.daemon
            .entries()
            .unwrap()
            .iter()
            .all(|e| e.entry_id != "v1-sealed")
    );
}

#[tokio::test]
async fn a_v1_row_that_owes_work_is_refused_by_a_default_daemon_with_a_remedy_that_clears_it() {
    use goalcup_daemon::registry::EntryState;
    // Each of these is work only the V1 runtime can do.
    let cases: [(&str, EntryState, Option<u64>, Option<u64>); 3] = [
        ("unsealed", EntryState::Started, None, None),
        (
            "drain short of the head",
            EntryState::Sealed,
            Some(9),
            Some(4),
        ),
        ("sealing", EntryState::Sealing, Some(2), Some(2)),
    ];
    for (what, state, chain_seq, last_ack_seq) in cases {
        let rig = Rig::start().await;
        let root = rig.root.clone();
        let refused = rig
            .restart_planting(|registry| {
                plant_v1_row(registry, &root, "v1-owing", state, chain_seq, last_ack_seq);
            })
            .await;
        let message = match refused {
            Ok(_) => panic!("{what}: a V1 row owing work was accepted"),
            Err(error) => error.to_string(),
        };
        assert!(
            message.contains("v1-owing"),
            "{what}: names the row: {message}"
        );
        assert!(
            message.contains("sealed and fully acknowledged")
                && message.contains("v0.5.0")
                && message.contains("artifact_root"),
            "{what}: says what clears it: {message}"
        );
    }
}

/// A default daemon has no V1 cadence to run, and the foreground binary relies on `run_forever`
/// never returning: the only way out is a signal.
#[tokio::test]
async fn a_default_daemons_loop_stays_pending() {
    let rig = Rig::start().await;
    let ended = tokio::time::timeout(
        std::time::Duration::from_millis(300),
        rig.daemon.run_forever(),
    )
    .await;
    assert!(ended.is_err(), "run_forever returned: {ended:?}");
}

/// Review R4 I1: `goalcup start` died between create and launch, and the daemon restarted. The
/// entry never handed anything off, so it may still be abandoned: otherwise it holds its
/// reservation and keeps a run-scoped daemon up forever.
#[tokio::test]
async fn a_recovered_capture_that_never_launched_can_be_abandoned() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let rig = rig.restart().await;
    let abandoned = rig.abandon(&info.entry_id).await;
    assert_eq!(abandoned.status(), 204, "{}", abandoned.body());
    assert!(
        !rig.artifact_dir(&info.entry_id).exists(),
        "the chain is gone"
    );
    assert!(rig.daemon.registry().get(&info.entry_id).unwrap().is_none());
}

/// Review R4 I1: the CLI died between create and launch, so nothing will ever seal or abandon
/// the entry. The idle loop abandons a never-launched entry past its bound, and leaves a
/// launched one alone.
#[tokio::test]
async fn a_stale_capture_that_never_launched_is_abandoned_by_the_daemon() {
    let rig = Rig::start().await;
    let stale = rig.created("codex-cli").await;
    let launched = rig.created("codex-cli").await;
    let goal = rig
        .goal_accepted(&launched.entry_id, b"write hello.txt")
        .await;
    rig.launch(&launched.entry_id, &launch_request(&goal)).await;
    let registered = rig.created("codex-cli").await;
    let harness = Child::sleeping();
    let response = rig
        .register(
            &registered.entry_id,
            &harness.registration(registered.process_generation),
        )
        .await;
    assert_eq!(response.status(), 204, "{}", response.body());

    rig.daemon
        .abandon_stale_unlaunched(std::time::Duration::from_secs(3600));
    assert!(
        rig.artifact_dir(&stale.entry_id).exists(),
        "a young entry is kept"
    );

    let errors = format!("{:?}", rig.daemon.cycle_errors());
    rig.daemon
        .abandon_stale_unlaunched(std::time::Duration::ZERO);
    assert!(
        !rig.artifact_dir(&stale.entry_id).exists(),
        "the chain is gone"
    );
    assert!(
        rig.artifact_dir(&registered.entry_id).exists(),
        "a registered harness is left for `goalcup seal`"
    );
    assert_eq!(
        format!("{:?}", rig.daemon.cycle_errors()),
        errors,
        "and it is not tried, so no refusal is noted"
    );
    assert!(
        rig.daemon
            .registry()
            .get(&stale.entry_id)
            .unwrap()
            .is_none()
    );
    assert!(
        rig.trace_dir(&launched.entry_id)
            .join("chain.jsonl")
            .exists(),
        "a launched entry is never abandoned"
    );
}

/// Review R4 I3: the terminal is chained and synced, and then the registry row does not say so
/// (its write failed, or the daemon died between the two). The next daemon reopens the entry
/// and the next seal must commit that terminal as it stands: nothing appended after it, and
/// never `SealWriteFailed`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_seal_whose_registry_write_was_lost_is_committed_without_a_second_terminal() {
    let (rig, _native, _connectors) = native_rig().await;
    let info = rig.created("capture-dev").await;
    let harness = Child::sleeping();
    let registered = rig
        .register(
            &info.entry_id,
            &harness.registration(info.process_generation),
        )
        .await;
    assert_eq!(registered.status(), 204, "{}", registered.body());
    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    rig.launch(&info.entry_id, &launch_request(&goal)).await;
    clean_epoch(&info).await;
    let sealed = rig
        .seal(&info.entry_id, Some(ProxyCounters::default()))
        .await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let first: CaptureFinishReport = sealed.json().unwrap();
    assert_eq!(first.seal, LocalSealOutcome::SealWritten);
    let chain = rig.trace_dir(&info.entry_id).join("chain.jsonl");
    let closed = std::fs::read(&chain).unwrap();

    // The row as it stood before the registry write.
    rig.daemon
        .registry()
        .update(&info.entry_id, |entry| {
            entry.sealed_at = None;
            if let Some(capture) = entry.capture.as_mut() {
                capture.finished = None;
                capture.sealed_status = None;
            }
        })
        .unwrap();
    let rig = rig.restart().await;
    let again = rig
        .seal(&info.entry_id, Some(ProxyCounters::default()))
        .await;
    assert_eq!(again.status(), 200, "{}", again.body());
    let again: CaptureFinishReport = again.json().unwrap();
    assert_eq!(again.seal, LocalSealOutcome::SealWritten);
    assert_eq!(again.terminal_seq, first.terminal_seq);
    assert_eq!(
        std::fs::read(&chain).unwrap(),
        closed,
        "nothing after the terminal"
    );
    assert_eq!(local_finishes(&rig.records(&info.entry_id)), 1);
    let status = rig.status(&info.entry_id).await;
    assert_eq!(status.payload.capture.unwrap().run, RunState::Sealed);
    let seal: goalcup_protocol::seal::SealManifest = serde_json::from_slice(
        &std::fs::read(rig.trace_dir(&info.entry_id).join("seal.json")).unwrap(),
    )
    .unwrap();
    let row = rig.daemon.registry().get(&info.entry_id).unwrap().unwrap();
    assert_eq!(row.sealed_at.as_deref(), Some(seal.sealed_at.as_str()));
}

/// The same, with the terminal chained and `seal.json` never written (a crash between the
/// two, or a full disk): the seal is a failed one, and stays one, and nothing is appended.
#[tokio::test]
async fn a_terminal_chained_without_its_seal_is_committed_as_a_failed_seal() {
    let rig = Rig::start().await;
    let info = rig.created("codex-cli").await;
    let first: CaptureFinishReport = rig.seal(&info.entry_id, None).await.json().unwrap();
    assert_eq!(first.seal, LocalSealOutcome::SealWritten);
    let chain = rig.trace_dir(&info.entry_id).join("chain.jsonl");
    let closed = std::fs::read(&chain).unwrap();
    std::fs::remove_file(rig.trace_dir(&info.entry_id).join("seal.json")).unwrap();
    rig.daemon
        .registry()
        .update(&info.entry_id, |entry| {
            entry.sealed_at = None;
            if let Some(capture) = entry.capture.as_mut() {
                capture.finished = None;
                capture.sealed_status = None;
            }
        })
        .unwrap();
    let rig = rig.restart().await;
    let again = rig.seal(&info.entry_id, None).await;
    assert_eq!(again.status(), 200, "{}", again.body());
    let again: CaptureFinishReport = again.json().unwrap();
    assert_eq!(again.seal, LocalSealOutcome::SealWriteFailed);
    assert_eq!(again.terminal_seq, first.terminal_seq);
    assert_eq!(
        std::fs::read(&chain).unwrap(),
        closed,
        "nothing after the terminal"
    );
}

/// Review R4 I5 / R5 I-5: the placement guard compared the workspace with this entry's own
/// artifact directory only. A workspace inside the machine-wide artifact root (beside the
/// registry and every other run's chain) or inside the configuration directory (`control.json`
/// and its bearer) was accepted.
#[tokio::test]
async fn a_workspace_inside_the_tracker_roots_is_refused() {
    let rig = Rig::start().await;
    let beside = rig
        .config
        .artifact_root
        .parent()
        .unwrap()
        .join("beside-the-roots");
    for workspace in [
        rig.config.artifact_root.join("goalcup-x/workspace"),
        rig.config.config_dir.join("goalcup-x/workspace"),
        beside,
    ] {
        let refused = rig.create("codex-cli", &workspace, RESERVE).await;
        assert_eq!(
            refused.status(),
            409,
            "{}: {}",
            workspace.display(),
            refused.body()
        );
        assert!(refused.body().contains("tracker's"), "{}", refused.body());
        assert!(!workspace.exists(), "nothing is created for it");
    }
    let fine = rig.create("codex-cli", &rig.workspace("ok"), RESERVE).await;
    assert_eq!(fine.status(), 200, "{}", fine.body());
}

/// The chain's parts, once there are at least `count` of them (10 s at most).
async fn parts_when(trace: &Path, count: usize) -> Vec<goalcup_protocol::export::ChainPart> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let parts = goalcup_protocol::export::chain_parts(trace).unwrap();
        if parts.len() >= count {
            return parts;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the chain never rotated into {count} parts: {parts:#?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// D55: the chain rotates into part files as it is flushed, and acknowledged parts are removed
/// while the daemon is down. The next daemon reopens the entry from the first surviving part
/// and appends from the true head: its restart gap and its seal continue the same chain.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restart_after_parts_were_deleted_keeps_appending() {
    let rig = Rig::start().await;
    // Every flush that finds the current part non-empty closes it.
    rig.daemon
        .set_chain_part_limits_for_test(1, std::time::Duration::from_secs(3600));
    let info = rig.created("codex-cli").await;
    let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
    let mut launch = launch_request(&goal);
    // This fixture isolates reopen behavior: no native producer is bound.
    launch.native_scope = None;
    let launched = rig.launch(&info.entry_id, &launch).await;
    assert_eq!(launched.status(), 204, "{}", launched.body());
    // Exchanges further apart than the owner's flush cadence, so each lands in its own part.
    let wired = info.clone();
    tokio::task::spawn_blocking(move || {
        let mut wire = Wire::connect(&wired, &wired.producer_id);
        for stream in 1..=3 {
            let id = fresh_id();
            wire.send(0, connection(&id, opened(false)));
            wire.send(stream, request_open(&id));
            wire.send(stream, bytes(br#"{"input":"hello"}"#));
            let end = FrameEvent::End {
                reason: EndReason::Complete,
            };
            wire.send(stream, end);
            let close = ConnectionEvent::Close {
                outcome: ConnectionOutcome::Complete,
            };
            wire.send(0, connection(&id, close));
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        wire.close();
    })
    .await
    .unwrap();
    let trace = rig.trace_dir(&info.entry_id);
    // Four parts: the oldest two are deleted, and the third, closed by a rotation, holds
    // records, so the head survives.
    parts_when(&trace, 4).await;

    let mut before = None;
    let rig = rig
        .restart_planting(|_| {
            let parts = goalcup_protocol::export::chain_parts(&trace).unwrap();
            let chain = support::read_chain(&trace);
            std::fs::remove_file(&parts[0].path).unwrap();
            std::fs::remove_file(&parts[1].path).unwrap();
            before = Some((parts, chain));
        })
        .await
        .expect("the daemon starts over the pruned chain");
    let (parts, chain_before) = before.unwrap();
    let head = chain_before.last().unwrap().link.clone();
    let cycles = rig.daemon.cycle_errors();
    assert_eq!((cycles.tick, cycles.last), (0, None), "the entry reopened");
    let reopened = rig.status(&info.entry_id).await.payload.capture.unwrap();
    assert_eq!(reopened.run, RunState::Running);
    let logical_end = parts.last().map(|part| part.start + part.len).unwrap();
    assert!(
        reopened
            .storage
            .durable
            .as_ref()
            .is_some_and(|durable| durable.chain_bytes > logical_end),
        "offsets stay logical: {reopened:?}"
    );

    let sealed = rig.seal(&info.entry_id, None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let report: CaptureFinishReport = sealed.json().unwrap();
    assert_eq!(report.seal, LocalSealOutcome::SealWritten);

    // The surviving parts are one contiguous stream from the first part not deleted.
    let surviving = goalcup_protocol::export::chain_parts(&trace).unwrap();
    assert_eq!(surviving[0].start, parts[2].start);
    let chain = support::read_chain(&trace);
    let first = &chain[0];
    assert_eq!(
        first.link,
        chain_before
            .iter()
            .find(|envelope| envelope.link.seq == first.link.seq)
            .unwrap()
            .link
    );
    for pair in chain.windows(2) {
        let (prev, next) = (&pair[0], &pair[1]);
        assert_eq!(next.link.seq, prev.link.seq + 1);
        assert_eq!(next.link.prev, prev.link.hash);
        assert_eq!(next.expected_hash(&prev.link.hash).unwrap(), next.link.hash);
    }
    // What the second daemon chained follows the first daemon's head.
    let after: Vec<_> = chain
        .iter()
        .filter(|envelope| envelope.link.seq > head.seq)
        .collect();
    assert_eq!(after[0].link.prev, head.hash);
    assert!(matches!(
        after[0].body,
        EventBody::Capture {
            record: CaptureRecord::Gap {
                stream: None,
                reason: GapReason::IngestLost,
                ..
            }
        }
    ));
    assert!(matches!(
        after.last().unwrap().body,
        EventBody::Capture {
            record: CaptureRecord::LocalFinish { .. }
        }
    ));
}

// ---------------------------------------------------------------- a clone as the workspace (D56)

fn read_manifest(path: &Path) -> goalcup_protocol::seal::SealManifest {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// The participant's own clone is the workspace: admitted with its files, and neither the
/// baseline nor the seal walks it. Both record the commit and the digest of what is
/// uncommitted instead.
#[tokio::test]
async fn a_clone_is_admitted_non_empty_and_not_walked() {
    let rig = Rig::start().await;
    let ws = rig.workspace("clone");
    let home = rig.root.join("git-home");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let git = |args: &[&str]| {
        let out = goalcup_daemon::workspace_git::command(&ws, &home)
            .expect("git on PATH")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}");
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    git(&["init", "-q"]);
    std::fs::write(ws.join("README.md"), b"# entry\n").unwrap();
    git(&["add", "README.md"]);
    git(&["commit", "-q", "-m", "first"]);
    let commit = git(&["rev-parse", "HEAD"]);

    let created = rig.create("codex-cli", &ws, RESERVE).await;
    assert_eq!(created.status(), 200, "{}", created.body());
    let info: CaptureWorkspaceInfo = created.json().unwrap();
    let trace = rig.trace_dir(&info.entry_id);
    let baseline = read_manifest(&trace.join("baseline.json"));
    assert_eq!(baseline.files, [], "the clone is not walked");
    let empty = hex::encode(Sha256::digest(b"diff\0untracked\0"));
    assert_eq!(
        baseline.git,
        Some(goalcup_protocol::seal::GitState::Read(
            goalcup_protocol::seal::WorkspaceGit {
                commit: Some(commit.clone()),
                dirty_sha256: empty.clone(),
            }
        ))
    );

    std::fs::write(ws.join("hello.txt"), b"hello\n").unwrap();
    let sealed = rig.seal(&info.entry_id, None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let seal = read_manifest(&trace.join("seal.json"));
    assert_eq!(seal.files, [], "the seal does not walk the clone either");
    assert_eq!(
        seal.home, None,
        "the harness home is listed on the baseline only"
    );
    let Some(goalcup_protocol::seal::GitState::Read(git_state)) = seal.git else {
        panic!("the seal records the Git state: {:?}", seal.git);
    };
    assert_eq!(git_state.commit, Some(commit));
    assert_ne!(
        git_state.dirty_sha256, empty,
        "the untracked file moved the digest"
    );
}

/// A clone whose Git state cannot be read is never walked instead, and is never refused either
/// (D56 amended): the create is admitted and the finish seals at once, each manifest recording
/// why in place of the state, so an agent that breaks its own `.git` makes only its own trace
/// incomplete.
#[tokio::test]
async fn a_clone_git_cannot_read_still_seals_never_walked() {
    use goalcup_protocol::seal::GitState;
    let rig = Rig::start().await;
    let home = rig.root.join("git-home");
    std::fs::create_dir_all(&home).unwrap();
    let clone = |name: &str| {
        let ws = rig.workspace(name);
        std::fs::create_dir_all(&ws).unwrap();
        let out = goalcup_daemon::workspace_git::command(&ws, &home)
            .expect("git on PATH")
            .args(["init", "-q"])
            .output()
            .unwrap();
        assert!(out.status.success());
        std::fs::write(ws.join("README.md"), b"# entry\n").unwrap();
        ws
    };
    let unreadable = |manifest: &goalcup_protocol::seal::SealManifest| {
        assert_eq!(manifest.files, [], "an unreadable clone is not walked");
        assert!(
            matches!(&manifest.git, Some(GitState::Unreadable { unreadable }) if !unreadable.is_empty()),
            "{:?}",
            manifest.git
        );
    };

    let broken = clone("broken");
    std::fs::write(broken.join(".git/HEAD"), b"not a ref\n").unwrap();
    let created = rig.create("codex-cli", &broken, RESERVE).await;
    assert_eq!(created.status(), 200, "{}", created.body());
    let info: CaptureWorkspaceInfo = created.json().unwrap();
    unreadable(&read_manifest(
        &rig.trace_dir(&info.entry_id).join("baseline.json"),
    ));

    let ws = clone("later-broken");
    let created = rig.create("codex-cli", &ws, RESERVE).await;
    assert_eq!(created.status(), 200, "{}", created.body());
    let info: CaptureWorkspaceInfo = created.json().unwrap();
    std::fs::write(ws.join(".git/HEAD"), b"not a ref\n").unwrap();
    let sealed = rig.seal(&info.entry_id, None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    unreadable(&read_manifest(
        &rig.trace_dir(&info.entry_id).join("seal.json"),
    ));
}

/// The seal follows the baseline (D56 amended): an entry created over a clone is never walked at
/// its finish, whether the agent removed `.git` or swapped it for a link. The seal records why
/// the state could not be read instead, which the verifier reports incomplete.
#[tokio::test]
async fn a_clone_whose_git_was_removed_or_swapped_still_seals_unwalked() {
    use goalcup_protocol::seal::GitState;
    let rig = Rig::start().await;
    let home = rig.root.join("git-home");
    std::fs::create_dir_all(&home).unwrap();
    for (name, break_git) in [
        (
            "removed",
            (|ws: &Path| std::fs::remove_dir_all(ws.join(".git")).unwrap()) as fn(&Path),
        ),
        ("swapped", |ws: &Path| {
            let moved = ws.with_file_name("moved-git");
            std::fs::rename(ws.join(".git"), &moved).unwrap();
            std::os::unix::fs::symlink(&moved, ws.join(".git")).unwrap();
        }),
    ] {
        let ws = rig.workspace(name);
        std::fs::create_dir_all(&ws).unwrap();
        let out = goalcup_daemon::workspace_git::command(&ws, &home)
            .expect("git on PATH")
            .args(["init", "-q"])
            .output()
            .unwrap();
        assert!(out.status.success());
        let created = rig.create("codex-cli", &ws, RESERVE).await;
        assert_eq!(created.status(), 200, "{}", created.body());
        let info: CaptureWorkspaceInfo = created.json().unwrap();

        break_git(&ws);
        std::fs::write(ws.join("work.rs"), b"work").unwrap();
        let sealed = rig.seal(&info.entry_id, None).await;
        assert_eq!(sealed.status(), 200, "{name}: {}", sealed.body());
        let seal = read_manifest(&rig.trace_dir(&info.entry_id).join("seal.json"));
        assert_eq!(seal.files, [], "{name}: the clone is not walked");
        assert_eq!(
            seal.git,
            Some(GitState::Unreadable {
                unreadable: "it has no .git directory".to_string()
            }),
            "{name}"
        );
    }
}

/// The baseline lists the connector's config home as it was before the run, without the
/// native root the harness writes its sessions into.
#[tokio::test]
async fn the_baseline_lists_the_harness_home_without_its_session_folder() {
    let rig = Rig::start().await;
    let config_home = rig.root.join("config-home");
    std::fs::create_dir_all(config_home.join("sessions/2026/10")).unwrap();
    std::fs::write(config_home.join("config.toml"), b"model = \"m\"\n").unwrap();
    std::fs::write(config_home.join("sessions/2026/10/old.jsonl"), b"{}\n").unwrap();
    // Over 64 MiB: its size alone, never read (a sparse file).
    std::fs::File::create(config_home.join("big.bin"))
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();

    let info = rig.created("codex-cli").await;
    let baseline = read_manifest(&rig.trace_dir(&info.entry_id).join("baseline.json"));
    let home = baseline.home.expect("the baseline lists the harness home");
    assert_eq!(
        home,
        [
            goalcup_protocol::seal::SealedFile {
                path: "big.bin".to_string(),
                bytes: 64 * 1024 * 1024 + 1,
                sha256: "0".repeat(64),
            },
            goalcup_protocol::seal::SealedFile {
                path: "config.toml".to_string(),
                bytes: 12,
                sha256: hex::encode(Sha256::digest(b"model = \"m\"\n")),
            },
        ]
    );
    assert_eq!(baseline.git, None, "a fresh workspace is no clone");
}

// ---------------------------------------------------------------------------------------------
// D57 chat mode: the session files since the run started, and the roots that hold them.

/// The labels of every native stream the chain opened.
fn native_labels(records: &[CaptureRecord]) -> Vec<String> {
    records
        .iter()
        .filter_map(|record| match record {
            CaptureRecord::Open {
                source: CaptureSource::Native,
                metadata,
                ..
            } => Some(metadata.label.clone()),
            _ => None,
        })
        .collect()
}

/// Wait until the clock is past the next whole second, so that every file touched before this
/// call is from an earlier second than anything after it (the floor compares whole seconds).
fn into_the_next_second() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let next = std::time::Duration::from_secs(now.as_secs() + 1);
    std::thread::sleep(next - now + std::time::Duration::from_millis(50));
}

/// The entry's row as the registry holds it, read while no daemon runs, then the daemon again.
async fn reopened_row(rig: Rig, entry_id: &str) -> (Rig, goalcup_daemon::registry::Entry) {
    let mut row = None;
    let rig = rig
        .restart_planting(|registry| row = registry.get(entry_id).unwrap())
        .await
        .expect("the daemon restarts");
    (rig, row.expect("the row survives"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_run_start_uses_the_entry_create_time() {
    let (rig, config_home, _connectors) = chat_rig().await;
    let day = config_home.join("sessions/2026/10/14");
    std::fs::create_dir_all(&day).unwrap();
    let old = day.join("rollout-old.jsonl");
    std::fs::write(&old, b"{\"old\":true}\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(86_400))
        .unwrap();
    into_the_next_second();
    // No `native_since`: the floor is the entry's own create time.
    let info = rig.created("capture-dev").await;
    std::fs::write(day.join("rollout-new.jsonl"), b"{\"new\":true}\n").unwrap();
    rig.records_when(&info.entry_id, |records| {
        native_labels(records)
            .iter()
            .any(|label| label.contains("rollout-new"))
    })
    .await;
    let sealed = rig.seal(&info.entry_id, None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    assert_eq!(
        native_labels(&rig.records(&info.entry_id)),
        ["rollout:2026/10/14/rollout-new.jsonl#g1"],
        "the old session is skipped and the new one recorded"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reopened_entry_keeps_its_create_time_floor() {
    let (rig, _config_home, _connectors) = chat_rig().await;
    let info = rig.created("capture-dev").await;
    let (rig, created) = reopened_row(rig, &info.entry_id).await;
    // Reopen again a second later: a floor taken at reopen would move, the create time does not.
    into_the_next_second();
    let (_rig, reopened) = reopened_row(rig, &info.entry_id).await;
    assert_eq!(reopened.created_at, created.created_at);
    let floor = goalcup_protocol::control::parse_z(&reopened.created_at).expect("RFC 3339");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(
        floor < now,
        "the floor is the create time, not the reopen time"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_native_root_under_the_config_home_is_created() {
    use std::os::unix::fs::PermissionsExt;
    let (rig, config_home, _connectors) = chat_rig().await;
    let sessions = config_home.join("sessions");
    assert!(!sessions.exists(), "a fresh home has no session store yet");
    let info = rig.created("capture-dev").await;
    let made = std::fs::symlink_metadata(&sessions).expect("the root was created");
    assert!(made.is_dir());
    assert_eq!(made.permissions().mode() & 0o777, 0o700);
    let day = sessions.join("2026/10/15");
    std::fs::create_dir_all(&day).unwrap();
    std::fs::write(day.join("rollout-first.jsonl"), b"{\"first\":true}\n").unwrap();
    rig.records_when(&info.entry_id, |records| !native_labels(records).is_empty())
        .await;
    let sealed = rig.seal(&info.entry_id, None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    assert_eq!(
        native_labels(&rig.records(&info.entry_id)),
        ["rollout:2026/10/15/rollout-first.jsonl#g1"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_symlinked_native_root_component_is_refused() {
    let (rig, config_home, _connectors) = chat_rig().await;
    let outside = rig.root.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, config_home.join("sessions")).unwrap();
    let workspace = rig.workspace("symlinked-root");
    let response = rig.create("capture-dev", &workspace, RESERVE).await;
    assert_ne!(response.status(), 200, "{}", response.body());
    assert!(
        response.body().contains("native root"),
        "{}",
        response.body()
    );
    assert_eq!(
        std::fs::read_dir(&outside).unwrap().count(),
        0,
        "nothing was created through the link"
    );
    assert!(!workspace.exists(), "a refused create leaves no workspace");
}

/// A continued run makes its next segment in the workspace its last entry had (D4): the last
/// entry is sealed, so it no longer holds the workspace. An entry still open does.
#[tokio::test]
async fn a_continued_segment_takes_the_workspace_of_its_sealed_entry() {
    let rig = Rig::start().await;
    // A clone root, so the next create is admitted whatever the last entry left in it (R3).
    let ws = rig.workspace("continued");
    let home = rig.root.join("git-home");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let init = goalcup_daemon::workspace_git::command(&ws, &home)
        .expect("git on PATH")
        .args(["init", "-q"])
        .output()
        .unwrap();
    assert!(init.status.success(), "git init");

    let first: CaptureWorkspaceInfo = {
        let created = rig.create("codex-cli", &ws, RESERVE).await;
        assert_eq!(created.status(), 200, "{}", created.body());
        created.json().unwrap()
    };
    // Open and unsealed, it holds the workspace.
    let held = rig.create("codex-cli", &ws, RESERVE).await;
    assert_eq!(held.status(), 409, "{}", held.body());
    assert!(
        held.body().contains("one workspace holds one entry"),
        "{}",
        held.body()
    );

    let sealed = rig.seal(&first.entry_id, None).await;
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let second = rig.create("codex-cli", &ws, RESERVE).await;
    assert_eq!(second.status(), 200, "{}", second.body());
    let second: CaptureWorkspaceInfo = second.json().unwrap();
    assert_ne!(second.entry_id, first.entry_id);

    // The new segment holds it in turn.
    let held = rig.create("codex-cli", &ws, RESERVE).await;
    assert_eq!(held.status(), 409, "{}", held.body());
    assert!(
        held.body().contains("one workspace holds one entry"),
        "{}",
        held.body()
    );
}

/// An entry this daemon could not reopen routes nothing, so it lets go of its workspace too: a
/// run whose chain was lost can still be continued where it ran.
#[tokio::test]
async fn a_continued_segment_takes_the_workspace_of_an_entry_that_failed_to_reopen() {
    let rig = Rig::start().await;
    let ws = rig.workspace("lost");
    let home = rig.root.join("git-home");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let init = goalcup_daemon::workspace_git::command(&ws, &home)
        .expect("git on PATH")
        .args(["init", "-q"])
        .output()
        .unwrap();
    assert!(init.status.success(), "git init");

    let created = rig.create("codex-cli", &ws, RESERVE).await;
    assert_eq!(created.status(), 200, "{}", created.body());
    let first: CaptureWorkspaceInfo = created.json().unwrap();
    std::fs::remove_file(rig.trace_dir(&first.entry_id).join("chain.jsonl")).unwrap();

    let rig = rig.restart().await;
    // Relies on the chain having no rotated parts: removing `chain.jsonl` leaves nothing to resume.
    let last = rig.daemon.cycle_errors().last;
    assert!(
        last.as_deref()
            .is_some_and(|e| e.contains("was not reopened")),
        "the lost chain was not reopened: {last:?}"
    );
    let second = rig.create("codex-cli", &ws, RESERVE).await;
    assert_eq!(second.status(), 200, "{}", second.body());
}
