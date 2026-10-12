use crate::common::capture_lifecycle_support;
use crate::common::support;

use base64::Engine as _;
use capture_lifecycle_support::{RESERVE, Rig, launch_request};
use goalcup_protocol::capture::CaptureRecord;
use sha2::{Digest, Sha256};
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

fn copied(ids: &[(&str, &str)]) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for (folder, id) in ids {
        support::copy_tree(
            &support::shipped_connector_dir(id),
            &root.path().join(folder),
        );
    }
    root
}

fn no_writes(rig: &Rig, workspace: &std::path::Path) {
    assert!(!workspace.exists());
    assert!(rig.daemon.entries().unwrap().is_empty(), "no registry row");
    let artifacts = rig.config.artifact_root.join("artifacts");
    assert!(std::fs::read_dir(artifacts).map_or(true, |mut rows| rows.next().is_none()));
}

#[tokio::test]
async fn shipped_connectors_record_the_pinned_directory_and_harness_profile_digests() {
    let rig = Rig::start().await;
    for id in ["codex-cli", "claude-code-cli", "hermes-cli"] {
        let info = rig.created(id).await;
        let goal = rig.goal_accepted(&info.entry_id, b"digest evidence").await;
        assert_eq!(
            rig.launch(&info.entry_id, &launch_request(&goal))
                .await
                .status(),
            204
        );
        let launch = rig
            .records(&info.entry_id)
            .into_iter()
            .find_map(|record| match record {
                CaptureRecord::Launch {
                    profile_digest,
                    connector_dir_sha256,
                    ..
                } => Some((profile_digest, connector_dir_sha256)),
                _ => None,
            })
            .unwrap();
        let harness =
            std::fs::read(support::shipped_connector_dir(id).join("harness.toml")).unwrap();
        assert_eq!(launch.0, hex::encode(Sha256::digest(harness)), "{id}");
        assert_eq!(
            launch.1.as_deref(),
            goalcup_protocol::pins::pinned_connector_dir_sha256(id),
            "{id}"
        );
    }
}

#[tokio::test]
async fn a_harness_only_unpinned_connector_is_created_and_its_redaction_reaches_the_daemon() {
    let connectors = copied(&[("dev", "codex-cli")]);
    let dir = connectors.path().join("dev");
    let harness = std::fs::read_to_string(dir.join("harness.toml"))
        .unwrap()
        .replacen("id = \"codex-cli\"", "id = \"harness-only-dev\"", 1);
    std::fs::write(dir.join("harness.toml"), harness).unwrap();
    let root = connectors.path().to_path_buf();
    let rig = Rig::start_with(move |config| config.connectors_dir = root).await;
    let response = rig
        .create("harness-only-dev", &rig.workspace("dev"), RESERVE)
        .await;
    assert_eq!(response.status(), 200, "{}", response.body());
    let info: goalcup_protocol::control::CaptureWorkspaceInfo = response.json().unwrap();
    let secret = b"S4A-HARNESS-ONLY-PLAN";
    let mut input = br#"{"plan_type":""#.to_vec();
    input.extend_from_slice(secret);
    input.extend_from_slice(br#""}"#);
    let goal = rig.goal_accepted(&info.entry_id, &input).await;
    let mut retained = Vec::new();
    let mut kinds = Vec::new();
    for record in rig.records(&info.entry_id) {
        if let CaptureRecord::Segment {
            stream,
            data_b64,
            audit,
            ..
        } = record
            && stream == goal
        {
            retained.extend(
                base64::engine::general_purpose::STANDARD
                    .decode(data_b64)
                    .unwrap(),
            );
            kinds.extend(audit.spans.into_iter().map(|span| span.kind));
        }
    }
    assert!(
        !retained.windows(secret.len()).any(|bytes| bytes == secret),
        "harness-only redaction must remove the schema-carried value: {retained:?}"
    );
    assert!(
        kinds.iter().any(|kind| kind == "codex_plan_type"),
        "the daemon must apply the harness-carried recognizer: {kinds:?}"
    );
}

#[tokio::test]
async fn two_unpinned_same_id_directories_with_different_bytes_refuse_before_any_write() {
    const ID: &str = "same-id-dev";
    let connectors = copied(&[
        ("codex-cli", "codex-cli"),
        ("claude-code-cli", "claude-code-cli"),
        ("hermes-cli", "hermes-cli"),
        ("daemon-dev", "codex-cli"),
    ]);
    let daemon_dir = connectors.path().join("daemon-dev");
    let daemon_harness = std::fs::read_to_string(daemon_dir.join("harness.toml"))
        .unwrap()
        .replacen("id = \"codex-cli\"", &format!("id = \"{ID}\""), 1);
    std::fs::write(daemon_dir.join("harness.toml"), daemon_harness).unwrap();

    let caller = copied(&[("caller-dev", "codex-cli")]);
    let caller_dir = caller.path().join("caller-dev");
    let caller_harness = std::fs::read_to_string(caller_dir.join("harness.toml"))
        .unwrap()
        .replacen("id = \"codex-cli\"", &format!("id = \"{ID}\""), 1)
        + "\n# different caller bytes\n";
    std::fs::write(caller_dir.join("harness.toml"), caller_harness).unwrap();
    let caller_digest = goalcup_protocol::connector_dir::connector_dir_sha256(&caller_dir).unwrap();
    let daemon_digest = goalcup_protocol::connector_dir::connector_dir_sha256(&daemon_dir).unwrap();
    assert_ne!(caller_digest, daemon_digest);

    let root = connectors.path().to_path_buf();
    let rig = Rig::start_with(move |config| config.connectors_dir = root).await;
    let workspace = rig.workspace("refused-cross-install-digest");
    let mut request = serde_json::to_value(rig.request(ID, &workspace, RESERVE)).unwrap();
    request["connector_dir_sha256"] = serde_json::Value::String(caller_digest);
    let response = rig
        .client
        .post_body(
            goalcup_protocol::control::ROUTE_CAPTURE_WORKSPACES,
            &request,
        )
        .await;
    assert_eq!(response.status(), 422, "{}", response.body());
    let body: goalcup_protocol::control::ErrorBody = response.json().unwrap();
    assert_eq!(body.error.code, "preflight_failed");
    assert!(
        !body
            .error
            .message
            .contains(&connectors.path().display().to_string())
            && !body
                .error
                .message
                .contains(&caller.path().display().to_string()),
        "the refusal is path-free: {}",
        body.error.message
    );
    no_writes(&rig, &workspace);
}

#[tokio::test]
async fn a_nonselected_harness_real_home_refuses_before_any_write() {
    const NONSHIPPED_HOME: &str = ".goalcup-s4a-nonshipped-union-proof";
    // The shipped connectors plus a harness-only family whose real home must constrain a
    // different selected harness.
    let connectors = copied(&[
        ("codex-cli", "codex-cli"),
        ("claude-code-cli", "claude-code-cli"),
        ("hermes-cli", "hermes-cli"),
        ("nonshipped", "codex-cli"),
    ]);
    let nonshipped = connectors.path().join("nonshipped");
    let harness = std::fs::read_to_string(nonshipped.join("harness.toml"))
        .unwrap()
        .replacen("id = \"codex-cli\"", "id = \"nonshipped-dev\"", 1)
        .replacen(
            "real_dirs = [\"{user_home}/.codex\"]",
            &format!("real_dirs = [\"{{user_home}}/{NONSHIPPED_HOME}\"]"),
            1,
        );
    std::fs::write(nonshipped.join("harness.toml"), harness).unwrap();
    let root = connectors.path().to_path_buf();
    let rig = Rig::start_with(move |config| config.connectors_dir = root).await;
    let workspace = rig.workspace("refused-nonselected-real-home");
    let mut request = rig.request("hermes-cli", &workspace, RESERVE);
    request.config_home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
        .join(NONSHIPPED_HOME)
        .join("sessions")
        .display()
        .to_string();
    let response = rig.create_request(&request).await;
    assert_eq!(response.status(), 409, "{}", response.body());
    no_writes(&rig, &workspace);
}

#[tokio::test]
async fn an_unknown_capture_profile_is_unsupported_not_missing_and_writes_nothing() {
    let rig = Rig::start().await;
    let workspace = rig.workspace("unsupported-profile");
    let response = rig.create("cursor-cli", &workspace, RESERVE).await;
    assert_eq!(response.status(), 422, "{}", response.body());
    let body: goalcup_protocol::control::ErrorBody = response.json().unwrap();
    assert_eq!(body.error.code, "unsupported_connector");
    no_writes(&rig, &workspace);
}

#[tokio::test]
async fn a_pinned_id_with_changed_directory_bytes_is_refused_before_any_write() {
    let connectors = copied(&[
        ("codex-cli", "codex-cli"),
        ("claude-code-cli", "claude-code-cli"),
        ("hermes-cli", "hermes-cli"),
    ]);
    let path = connectors.path().join("hermes-cli/harness.toml");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.extend_from_slice(b"\n# changed\n");
    std::fs::write(path, bytes).unwrap();
    let root = connectors.path().to_path_buf();
    let rig = Rig::start_with(move |config| config.connectors_dir = root).await;
    let workspace = rig.workspace("refused-pin");
    assert_eq!(
        rig.create("hermes-cli", &workspace, RESERVE).await.status(),
        409
    );
    no_writes(&rig, &workspace);
}

#[tokio::test]
async fn a_renamed_changed_shipped_connector_is_refused_before_any_write() {
    let connectors = copied(&[
        ("codex-cli", "codex-cli"),
        ("renamed-codex", "codex-cli"),
        ("claude-code-cli", "claude-code-cli"),
        ("hermes-cli", "hermes-cli"),
    ]);
    let renamed = connectors.path().join("renamed-codex");
    // The alias declares the shipped id with changed bytes, so the request reaches the loader.
    let path = renamed.join("harness.toml");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.extend_from_slice(b"\n# renamed and changed\n");
    std::fs::write(path, bytes).unwrap();
    let root = connectors.path().to_path_buf();
    let rig = Rig::start_with(move |config| config.connectors_dir = root).await;
    let workspace = rig.workspace("refused-renamed-pin");
    let response = rig.create("codex-cli", &workspace, RESERVE).await;
    assert!(
        matches!(response.status(), 409 | 422),
        "a shipped id under an alias must fail pin preflight: {}",
        response.body()
    );
    no_writes(&rig, &workspace);
}

// The runtime loader's unit test covers the shared alias policy; this endpoint test covers the
// S4 path.
#[tokio::test]
async fn a_byte_identical_shipped_connector_under_an_alias_is_refused_by_daemon_preflight() {
    let connectors = copied(&[
        ("alias", "codex-cli"),
        ("claude-code-cli", "claude-code-cli"),
        ("hermes-cli", "hermes-cli"),
    ]);
    let root = connectors.path().to_path_buf();
    let rig = Rig::start_with(move |config| config.connectors_dir = root).await;
    let workspace = rig.workspace("refused-byte-identical-alias");
    let response = rig.create("codex-cli", &workspace, RESERVE).await;
    assert_eq!(response.status(), 409, "{}", response.body());
    assert!(response.body().contains("folder"), "{}", response.body());
    no_writes(&rig, &workspace);
}

#[tokio::test]
async fn two_harness_directories_declaring_one_id_are_refused_before_any_write() {
    let connectors = copied(&[("one", "codex-cli"), ("two", "codex-cli")]);
    for folder in ["one", "two"] {
        let dir = connectors.path().join(folder);
        let harness = std::fs::read_to_string(dir.join("harness.toml"))
            .unwrap()
            .replacen("id = \"codex-cli\"", "id = \"duplicate-dev\"", 1);
        std::fs::write(dir.join("harness.toml"), harness).unwrap();
    }
    let root = connectors.path().to_path_buf();
    let rig = Rig::start_with(move |config| config.connectors_dir = root).await;
    let workspace = rig.workspace("refused-duplicate");
    assert_eq!(
        rig.create("duplicate-dev", &workspace, RESERVE)
            .await
            .status(),
        409
    );
    no_writes(&rig, &workspace);
}

/// A connector directory the daemon cannot hash is refused at capture creation, before the
/// first write. `connector_dir_sha256` refuses a symlink rather than following it, because
/// following one would pin bytes living outside the directory. The daemon no longer reads the
/// connectors directory at startup (OS-29), so this is where the refusal lands.
#[tokio::test]
async fn a_connector_directory_holding_a_symlink_is_refused_at_create_before_any_write() {
    let connectors = copied(&[
        ("codex-cli", "codex-cli"),
        ("claude-code-cli", "claude-code-cli"),
        ("hermes-cli", "hermes-cli"),
    ]);
    let files = connectors.path().join("claude-code-cli/files");
    std::fs::create_dir_all(&files).unwrap();
    std::os::unix::fs::symlink("project-settings.json", files.join("alias.json")).unwrap();
    // The cause is the symlink and nothing else: the directory walk refuses exactly that entry,
    // by the typed symlink refusal, rather than following it or failing for some other reason.
    match goalcup_protocol::connector_dir::connector_dir_sha256(
        &connectors.path().join("claude-code-cli"),
    ) {
        Err(goalcup_protocol::ProtocolError::UnhashableEntry { path, reason }) => {
            assert!(path.ends_with("alias.json"), "{path}");
            assert!(reason.contains("symlink"), "{reason}");
        }
        other => panic!("the symlinked entry is refused as unhashable, got {other:?}"),
    }
    let root = connectors.path().to_path_buf();
    let rig = Rig::start_with(move |config| config.connectors_dir = root).await;
    let workspace = rig.workspace("refused-unhashable");
    // `Rig::request` hashes the directory client-side, which the symlink refuses; name the
    // shipped pin instead, as the CLI would for an untouched install.
    let mut request = rig.request("hermes-cli", &workspace, RESERVE);
    request.profile_id = "claude-code-cli".to_string();
    request.connector_dir_sha256 =
        goalcup_protocol::pins::pinned_connector_dir_sha256("claude-code-cli")
            .unwrap()
            .to_string();
    let response = rig.create_request(&request).await;
    assert_eq!(response.status(), 422, "{}", response.body());
    let body: goalcup_protocol::control::ErrorBody = response.json().unwrap();
    assert_eq!(
        body.error.code, "preflight_failed",
        "{}",
        body.error.message
    );
    // The daemon names no path and echoes nothing of the directory it refused.
    assert!(
        !body.error.message.contains("alias.json")
            && !body.error.message.contains("project-settings"),
        "the refusal does not repeat the symlink's name or target: {}",
        body.error.message
    );
    no_writes(&rig, &workspace);
}

#[tokio::test]
async fn a_symlinked_connector_refuses_without_echoing_its_out_of_tree_target_or_writing() {
    const MARKER: &str = "OUT_OF_TREE_MARKER_MUST_NOT_ECHO";
    let connectors = copied(&[
        ("codex-cli", "codex-cli"),
        ("claude-code-cli", "claude-code-cli"),
        ("hermes-cli", "hermes-cli"),
    ]);
    let root = connectors.path().to_path_buf();
    let rig = Rig::start_with(move |config| config.connectors_dir = root).await;
    let outside = tempfile::Builder::new().prefix(MARKER).tempdir().unwrap();
    let fifo = outside.path().join("harness.toml");
    let fifo_name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: `fifo_name` is a live NUL-terminated path and the mode is an ordinary owner-only
    // FIFO mode. The temporary directory owns the resulting node.
    assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
    let cleanup_started = Arc::new(AtomicBool::new(false));
    let read_before_cleanup = Arc::new(AtomicBool::new(false));
    let cleanup_for_writer = Arc::clone(&cleanup_started);
    let read_for_writer = Arc::clone(&read_before_cleanup);
    let writer_name = fifo_name.clone();
    let probe = std::thread::spawn(move || {
        // A blocking FIFO writer returns only after some code opens the target for reading. The
        // test opens its own cleanup reader after connector preflight returns, so the ordering is
        // deterministic: an earlier return proves the loader entered the out-of-tree target.
        // SAFETY: `writer_name` is a live NUL-terminated path for this call.
        let fd = unsafe { libc::open(writer_name.as_ptr(), libc::O_WRONLY) };
        assert!(fd >= 0, "FIFO writer: {}", std::io::Error::last_os_error());
        if !cleanup_for_writer.load(Ordering::Acquire) {
            read_for_writer.store(true, Ordering::Release);
        }
        // SAFETY: `fd` is the open FIFO above and `MARKER` is live for this write.
        let written =
            unsafe { libc::write(fd, MARKER.as_ptr().cast(), MARKER.len() as libc::size_t) };
        assert_eq!(written, MARKER.len() as isize);
        // SAFETY: this thread owns the open descriptor.
        assert_eq!(unsafe { libc::close(fd) }, 0);
    });
    std::os::unix::fs::symlink(
        outside.path(),
        rig.config.connectors_dir.join("out-of-tree-link"),
    )
    .unwrap();

    let workspace = rig.workspace("refused-symlink");
    let response = tokio::time::timeout(
        Duration::from_secs(4),
        rig.create("codex-cli", &workspace, RESERVE),
    )
    .await;
    cleanup_started.store(true, Ordering::Release);
    // Unblock the probe only after preflight has returned (or its hard bound expired). Keeping
    // this read descriptor open through `join` makes cleanup independent of thread scheduling.
    // SAFETY: `fifo_name` is a live NUL-terminated FIFO path for this call.
    let cleanup_fd = unsafe { libc::open(fifo_name.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
    assert!(
        cleanup_fd >= 0,
        "FIFO cleanup reader: {}",
        std::io::Error::last_os_error()
    );
    probe.join().unwrap();
    // SAFETY: this thread owns the cleanup descriptor.
    assert_eq!(unsafe { libc::close(cleanup_fd) }, 0);
    let response = response.expect("a FIFO read cannot hang connector preflight");
    assert_eq!(response.status(), 422, "{}", response.body());
    assert!(
        !read_before_cleanup.load(Ordering::Acquire),
        "connector preflight opened the out-of-tree harness bytes"
    );
    assert!(!response.body().contains(MARKER), "{}", response.body());
    assert!(
        !response
            .body()
            .contains(&outside.path().display().to_string()),
        "{}",
        response.body()
    );
    no_writes(&rig, &workspace);
}
