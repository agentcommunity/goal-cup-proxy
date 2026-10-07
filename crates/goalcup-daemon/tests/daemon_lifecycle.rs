//! The daemon's own lifecycle: what `abandon` may
//! delete, what a sealed entry keeps holding, what one entry's stall costs another, and
//! what `stop` is allowed to return over.
//!
//! Every case here drives the process-level object — the entry map, the four cadences, the
//! registry the daemon opened — rather than a single `EntryRuntime`, because each of them
//! is about two things happening at once and one runtime cannot be two things.
#![cfg(feature = "v1-compat")]

mod support;

use goalcup_protocol::manifest::KillStrategy;
use std::io::Write as _;

/// A service-free installation may serve capture entries, but legacy creation must refuse
/// before preparing a workspace, rendering tracker files or inserting a registry row.
#[tokio::test]
async fn no_service_legacy_create_leaves_no_workspace_or_row() {
    let dir = tempfile::tempdir().expect("a disposable tracker prefix");
    let path = support::write_config_with(dir.path(), |text| {
        text.lines()
            .filter(|line| {
                !line.starts_with("upload_url") && !line.starts_with("server_public_key")
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    let config = goalcup_daemon::config::DaemonConfig::load(&path).expect("local config parses");
    let daemon = goalcup_daemon::runtime::Daemon::start(config.clone())
        .await
        .expect("an empty service-free installation starts");
    let client = support::ControlClient::of(&daemon, &config.config_dir);
    let response = client
        .post_body(
            goalcup_protocol::control::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    assert_eq!(response.status(), 409, "{}", response.body());
    assert!(
        response.body().contains("upload_url") && response.body().contains("server_public_key")
    );
    assert!(daemon.registry().all().expect("registry reads").is_empty());
    assert!(
        !config.workspace_parent.exists(),
        "no workspace parent is prepared"
    );
    assert!(
        !config.artifact_root.join("artifacts").exists(),
        "no entry artifacts are prepared"
    );
    assert!(
        !config.artifact_root.join("homes").exists(),
        "no harness home is prepared"
    );
    daemon.stop().await;
}

/// Losing a legacy service pair is an actionable startup refusal, not permission to
/// sweep status files, complete pending abandon cleanup, or rehydrate part of a registry.
#[tokio::test]
async fn no_service_startup_preserves_legacy_and_mixed_registries() {
    for mixed in [false, true] {
        let h = support::empty_control_harness().await;
        let info: goalcup_protocol::control::WorkspaceInfo = h
            .post_json(
                goalcup_protocol::control::ROUTE_WORKSPACES,
                &support::workspace_opts("claude-code-cli"),
            )
            .await;
        let entry = h.daemon.registry().get(&info.entry_id).unwrap().unwrap();
        let chain = entry.trace_dir.join("chain.jsonl");
        let baseline =
            std::fs::read(&chain).expect("the configured legacy path wrote its baseline");
        assert!(!baseline.is_empty());
        h.daemon
            .registry()
            .update(&info.entry_id, |entry| {
                entry.state = goalcup_daemon::registry::EntryState::CleanupPending;
            })
            .expect("stage recoverable legacy cleanup");
        if mixed {
            // A persisted v5-shaped row is enough to exercise registry admission: startup
            // must refuse before trying to open either row's capture or legacy artifacts.
            let mut capture = support::entry_in(
                &h.daemon.registry(),
                &h.config.artifact_root,
                "capture-row",
                "codex-cli",
                h.root.join("capture-workspace").to_str().unwrap(),
            );
            capture.capture = Some(goalcup_daemon::registry::CaptureRowV1 {
                reserve_bytes: 64 << 20,
                finished: None,
                sealed_status: None,
            });
            h.daemon
                .registry()
                .put(&capture)
                .expect("stage mixed registry");
        }
        let before = serde_json::to_vec(&h.daemon.registry().all().unwrap()).unwrap();
        let mut config = h.config.clone();
        h.daemon.stop().await;
        let stale = config.config_dir.join("cache").join("preserve.tmp");
        std::fs::write(&stale, b"STALE_CACHE_SENTINEL").expect("stage stale cache temp");
        config.upload_url.clear();
        config.server_public_key.clear();
        let error = match goalcup_daemon::runtime::Daemon::start(config.clone()).await {
            Err(error) => error.to_string(),
            Ok(daemon) => {
                daemon.stop().await;
                panic!("service-free startup must refuse a legacy or mixed registry");
            }
        };
        assert!(
            error.contains("upload_url") && error.contains("server_public_key"),
            "{error}"
        );
        assert_eq!(std::fs::read(&stale).unwrap(), b"STALE_CACHE_SENTINEL");
        assert_eq!(
            std::fs::read(&chain).unwrap(),
            baseline,
            "legacy artifacts remain intact"
        );
        let registry = goalcup_daemon::registry::Registry::open(
            &goalcup_daemon::registry::Registry::default_path(&config.artifact_root),
        )
        .expect("refused startup releases the registry");
        assert_eq!(
            serde_json::to_vec(&registry.all().unwrap()).unwrap(),
            before,
            "startup refusal leaves every row unchanged (mixed={mixed})"
        );
    }
}

/// A terminal validation refusal is capture-closed evidence.  It must be durable and visible
/// through the signed status even where the stale acknowledgement watermark would otherwise
/// make the ordinary pending calculation say zero.
fn assert_terminal_drain_review(h: &support::DaemonHarness) {
    assert!(
        h.entry().terminal_drain_validation_failed,
        "the terminal validation failure is durable rather than cadence-local"
    );
    let status = h
        .daemon
        .signed_status(&h.entry_id)
        .expect("the signed status reads")
        .expect("the terminal validation refusal republishes capture-closed status");
    assert_eq!(
        status.payload.health,
        goalcup_protocol::events::HealthState::SealingRecoveryNeeded,
        "a terminal validation refusal cannot remain a sealed all-clear"
    );
    assert!(
        status.payload.events_pending > 0,
        "an irreconcilable terminal acknowledgement cannot be published as zero pending"
    );
    assert!(
        status
            .payload
            .detail
            .contains("terminal upload validation failed"),
        "the signed status names the elevated-review condition"
    );
}

/// A persisted terminal-validation refusal has priority over a stale in-memory or on-disk
/// signed all-clear.  A publication attempt can itself fail (a full filesystem is the real
/// example), so the immediate daemon/API answer must be the newly composed recovery document;
/// it must never be the pre-refusal `Sealed` zero-pending document. The raw file reader may
/// still be unavailable while the mirror view retries, which is why this probes the daemon's
/// guarded signed-status surface rather than treating a failed filesystem write as a lie.
fn assert_terminal_drain_review_after_publication_failure(h: &support::DaemonHarness) {
    assert!(
        h.entry().terminal_drain_validation_failed,
        "the terminal validation refusal is durable before publication is attempted"
    );
    let status = h
        .daemon
        .signed_status(&h.entry_id)
        .expect("the signed status lookup itself reads")
        .expect("the daemon cache installs review before the filesystem views run");
    assert_eq!(
        status.payload.health,
        goalcup_protocol::events::HealthState::SealingRecoveryNeeded,
        "a durable terminal refusal may not serve a stale sealed all-clear"
    );
    assert!(
        status.payload.events_pending > 0,
        "a durable terminal refusal may not serve zero pending"
    );
}

fn offline_signed_status(
    path: &std::path::Path,
) -> Option<goalcup_protocol::control::SignedStatus> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}

fn write_mcp_key(config_dir: &std::path::Path, entry: &goalcup_daemon::registry::Entry) {
    let entries = config_dir.join("entries");
    std::fs::create_dir_all(&entries).expect("the MCP key directory is created");
    std::fs::write(
        entries.join(format!("{}.json", entry.entry_id)),
        serde_json::json!({
            "entry_id": entry.entry_id,
            "run_public_key": entry.keys.public_key_hex().expect("the run key renders"),
        })
        .to_string(),
    )
    .expect("the exact MCP pinned-key record is written");
}

fn mcp_source(h: &support::DaemonHarness) -> goalcup_mcp::StatusSource {
    write_mcp_key(&h.config.config_dir, &h.entry());
    goalcup_mcp::StatusSource::open(&h.config.config_dir).expect("the actual MCP reader opens")
}

fn canonical_status_line(h: &support::DaemonHarness) -> std::path::PathBuf {
    goalcup_daemon::status::canonical_status_line_path(&h.entry_id, &h.config.artifact_root)
        .expect("the fixture entry id has one safe canonical status line")
}

/// A live runtime owns the entry artifact capability it acquired at construction. Swapping
/// the registry-visible path afterwards must not redirect the ordinary status cadence.
#[cfg(unix)]
#[tokio::test]
async fn a_live_status_refresh_stays_in_the_retained_entry_after_an_entry_swap() {
    use std::os::unix::fs::symlink;

    let h = support::started_entry_with_work().await;
    let outside = h.config.config_dir.join("outside-artifact-sentinel");
    std::fs::create_dir(&outside).expect("outside artifact directory");
    std::fs::write(outside.join("status-line.txt"), b"OUTSIDE_SENTINEL").expect("outside sentinel");
    let detached = h.config.config_dir.join("detached-entry-artifact");
    std::fs::rename(&h.artifact_dir, &detached).expect("detach acquired entry");
    symlink(&outside, &h.artifact_dir).expect("replace entry path with outside link");

    h.daemon
        .refresh_statuses()
        .expect("live cadence retains its acquired entry authority");

    assert!(
        std::fs::read(detached.join("status-line.txt"))
            .expect("the retained entry receives its status line")
            .starts_with(b"/goal Cup "),
    );
    assert_eq!(
        std::fs::read(outside.join("status-line.txt")).expect("outside sentinel remains"),
        b"OUTSIDE_SENTINEL"
    );
}

/// A terminal review is still a publication by the live runtime that observed the
/// refusal.  It must not reacquire the entry spelling after an attacker has redirected it:
/// the detached, retained entry receives the review while the replacement stays untouched.
#[cfg(unix)]
#[tokio::test]
async fn a_live_terminal_review_stays_in_the_retained_entry_after_an_entry_swap() {
    use std::os::unix::fs::symlink;

    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let terminal = h.entry().chain_seq.expect("the terminal sequence");
    let outside = h
        .config
        .config_dir
        .join("outside-terminal-artifact-sentinel");
    std::fs::create_dir(&outside).expect("outside artifact directory");
    std::fs::write(
        outside.join("status-line.txt"),
        b"OUTSIDE_TERMINAL_SENTINEL",
    )
    .expect("outside sentinel");
    let detached = h.config.config_dir.join("detached-terminal-entry-artifact");
    std::fs::rename(&h.artifact_dir, &detached).expect("detach acquired entry");
    symlink(&outside, &h.artifact_dir).expect("replace entry path with outside link");
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| entry.last_ack_seq = Some(terminal + 1))
        .expect("the impossible durable acknowledgement is staged");

    h.daemon
        .pump_all()
        .await
        .expect_err("the terminal validation refusal reaches the upload cadence");

    let review_line = std::fs::read(detached.join("status-line.txt"))
        .expect("the retained entry receives terminal review truth");
    assert!(
        std::str::from_utf8(&review_line)
            .expect("line is UTF-8")
            .starts_with("/goal Cup — Capture is closed:"),
        "the terminal review replaces the retained status line"
    );
    assert_eq!(
        std::fs::read(outside.join("status-line.txt")).expect("outside sentinel remains"),
        b"OUTSIDE_TERMINAL_SENTINEL"
    );
}

/// A terminal persistence refusal revokes its old status line, so this removal has the same
/// retained-authority requirement as terminal review replacement.  After the entry spelling
/// is redirected, the detached line must disappear and the replacement must remain untouched.
#[cfg(unix)]
#[tokio::test]
async fn a_live_terminal_persistence_failure_invalidates_the_retained_entry_after_a_swap() {
    use std::os::unix::fs::symlink;

    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let terminal = h.entry().chain_seq.expect("the terminal sequence");
    h.daemon
        .fail_next_terminal_drain_quarantine_persistence_for_test();
    let outside = h
        .config
        .config_dir
        .join("outside-terminal-persist-failure-sentinel");
    std::fs::create_dir(&outside).expect("outside artifact directory");
    std::fs::write(
        outside.join("status-line.txt"),
        b"OUTSIDE_PERSIST_FAILURE_SENTINEL",
    )
    .expect("outside sentinel");
    let detached = h
        .config
        .config_dir
        .join("detached-terminal-persist-failure-entry");
    std::fs::rename(&h.artifact_dir, &detached).expect("detach acquired entry");
    symlink(&outside, &h.artifact_dir).expect("replace entry path with outside link");
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| entry.last_ack_seq = Some(terminal + 1))
        .expect("the impossible durable acknowledgement is staged");

    h.daemon
        .pump_all()
        .await
        .expect_err("the unpersistable terminal refusal reaches the upload cadence");

    assert!(
        !detached.join("status-line.txt").exists(),
        "the retained terminal artifact loses its stale all-clear"
    );
    assert_eq!(
        std::fs::read(outside.join("status-line.txt")).expect("outside sentinel remains"),
        b"OUTSIDE_PERSIST_FAILURE_SENTINEL"
    );
}

/// The MCP and the CLI's offline fallback read these exact cache files, rather than the
/// daemon's guarded in-memory status API. A terminal quarantine must make each view either
/// carry the elevated-review document or be unavailable while a filesystem retry is pending.
fn assert_terminal_offline_views_fail_closed(h: &support::DaemonHarness) {
    let cache = h.config.config_dir.join("cache");
    let entry = offline_signed_status(&cache.join(format!("status-{}.json", h.entry_id)));
    match entry {
        Some(entry) => {
            assert_eq!(
                entry.payload.health,
                goalcup_protocol::events::HealthState::SealingRecoveryNeeded,
                "the per-entry offline cache may not retain a stale sealed all-clear"
            );
            assert!(entry.payload.events_pending > 0);
        }
        None => assert!(
            mcp_source(h).for_entry(&h.entry_id).is_err(),
            "a missing target cache must be unavailable through the actual MCP reader"
        ),
    }
    let latest = offline_signed_status(&cache.join("status-latest.json"));
    match latest {
        Some(latest) if latest.payload.entry_id == h.entry_id => {
            assert_eq!(
                latest.payload.health,
                goalcup_protocol::events::HealthState::SealingRecoveryNeeded,
                "the MCP latest pointer may not retain a stale sealed all-clear for this entry"
            );
            assert!(latest.payload.events_pending > 0);
        }
        Some(_) => panic!("the one-entry fixture's latest pointer may not be repointed"),
        None => assert!(
            mcp_source(h).latest().is_err(),
            "a missing target-owned latest pointer must be unavailable through MCP"
        ),
    }
}

fn assert_terminal_offline_views_are_review(h: &support::DaemonHarness) {
    let cache = h.config.config_dir.join("cache");
    let entry = offline_signed_status(&cache.join(format!("status-{}.json", h.entry_id)))
        .expect("the recovered per-entry offline cache exists");
    let latest = offline_signed_status(&cache.join("status-latest.json"))
        .expect("the recovered MCP latest pointer exists");
    for signed in [entry, latest] {
        assert_eq!(signed.payload.entry_id, h.entry_id);
        assert_eq!(
            signed.payload.health,
            goalcup_protocol::events::HealthState::SealingRecoveryNeeded
        );
        assert!(signed.payload.events_pending > 0);
    }
    let source = mcp_source(h);
    assert_eq!(
        source
            .for_entry(&h.entry_id)
            .expect("MCP reads entry")
            .payload
            .health,
        goalcup_protocol::events::HealthState::SealingRecoveryNeeded
    );
    assert_eq!(
        source.latest().expect("MCP reads latest").payload.health,
        goalcup_protocol::events::HealthState::SealingRecoveryNeeded
    );
}

/// A start receipt is durable before `Daemon::bind_start` returns, so the status cache must
/// already name the bound session too.  Waiting for the cadence hid the durable transition
/// behind a stale `session_id: null` document that an immediate CLI/status reader could use.
#[tokio::test]
async fn successful_start_republishes_the_signed_status_before_the_next_cadence() {
    let h = support::started_entry_with_work().await;
    let cached = h
        .daemon
        .signed_status(&h.entry_id)
        .expect("the cache reads")
        .expect("the successful start published a status");
    assert_eq!(
        cached.payload.session_id.as_deref(),
        Some(support::DAEMON_SESSION)
    );
    assert!(cached.payload.started_at.is_some());

    let disk: goalcup_protocol::control::SignedStatus = serde_json::from_slice(
        &std::fs::read(
            h.config
                .config_dir
                .join("cache")
                .join(format!("status-{}.json", h.entry_id)),
        )
        .expect("the entry cache exists without a later status cadence"),
    )
    .expect("the signed cache parses");
    assert_eq!(
        disk.payload.session_id.as_deref(),
        Some(support::DAEMON_SESSION)
    );
}

fn copy_connector(root: &std::path::Path, id: &str, directory_name: &str) {
    fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    let shipped = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors")
        .join(id);
    copy_tree(&shipped, &root.join(directory_name));
}

#[test]
fn trusted_scan_union_refuses_missing_mismatched_duplicate_and_modified_authority() {
    let missing = tempfile::tempdir().unwrap();
    copy_connector(missing.path(), "codex-cli", "codex-cli");
    let error = goalcup_daemon::foreign::TrustedManifests::load(missing.path())
        .expect_err("the installed owner names an absent target")
        .to_string();
    assert!(
        error.contains("uninstalled target `claude-code-cli`"),
        "{error}"
    );

    let mismatched = tempfile::tempdir().unwrap();
    copy_connector(mismatched.path(), "claude-code-cli", "claude-code-cli");
    copy_connector(mismatched.path(), "codex-cli", "wrong-directory");
    let error = goalcup_daemon::foreign::TrustedManifests::load(mismatched.path())
        .expect_err("an applicable target must declare its directory identity")
        .to_string();
    assert!(
        error.contains("wrong-directory") && error.contains("codex-cli"),
        "{error}"
    );

    let duplicate = tempfile::tempdir().unwrap();
    for id in ["claude-code-cli", "codex-cli"] {
        copy_connector(duplicate.path(), id, id);
    }
    copy_connector(duplicate.path(), "codex-cli", "codex-copy");
    let error = goalcup_daemon::foreign::TrustedManifests::load(duplicate.path())
        .expect_err("duplicate declared ids are refused")
        .to_string();
    assert!(
        error.contains("more than one") && error.contains("codex-cli"),
        "{error}"
    );

    let modified = tempfile::tempdir().unwrap();
    for id in ["claude-code-cli", "codex-cli"] {
        copy_connector(modified.path(), id, id);
    }
    let manifest = modified.path().join("codex-cli/manifest.toml");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&manifest)
        .unwrap()
        .write_all(b"\n# modified after release\n")
        .unwrap();
    let error = goalcup_daemon::foreign::TrustedManifests::load(modified.path())
        .expect_err("a scan owner must match its release pin")
        .to_string();
    assert!(
        error.contains("codex-cli") && error.contains("not pinned"),
        "{error}"
    );

    let invalid = tempfile::tempdir().unwrap();
    for id in ["claude-code-cli", "codex-cli"] {
        copy_connector(invalid.path(), id, id);
    }
    let manifest = invalid.path().join("codex-cli/manifest.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replacen(
            "session_id_field = \"payload.session_id\"",
            "session_id_field = \"payload.session_id\"\nreader = { kind = \"sqlite_session\", table = \"session.id\", session_id_column = \"id\", directory_column = \"directory\" }",
            1,
        ),
    )
    .unwrap();
    let error = goalcup_daemon::foreign::TrustedManifests::load(invalid.path())
        .expect_err("an installed scan owner must pass semantic validation")
        .to_string();
    assert!(
        error.contains("valid scan authority") && error.contains("foreign_scan.reader"),
        "{error}"
    );

    let legacy_root = tempfile::tempdir().unwrap();
    for id in ["claude-code-cli", "codex-cli"] {
        copy_connector(legacy_root.path(), id, id);
    }
    let manifest = legacy_root.path().join("claude-code-cli/manifest.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, text.replace("root = \"own_config_dir\"\n", "")).unwrap();
    let error = goalcup_daemon::foreign::TrustedManifests::load(legacy_root.path())
        .expect_err("the runtime must not render a compatibility-default root")
        .to_string();
    assert!(
        error.contains("foreign_scan[1]") && error.contains("root"),
        "{error}"
    );
}
use goalcup_protocol::process::ProcessHandle;
use std::time::{Duration, Instant};

/// A committed run is never abandonable. While a binding call owns its runtime slot the
/// immediate answer is Busy; after it returns, the durable started-state refusal says to
/// seal rather than delete the run's evidence.
#[tokio::test]
async fn abandoning_a_started_entry_is_refused_while_its_runtime_is_mid_binding() {
    let h = support::started_entry_with_work().await;
    let chain = h.trace_dir.join("chain.jsonl");
    assert!(chain.is_file(), "the run chained before any of this");

    // The runtime can be temporarily absent during a binding round trip. Its Busy response
    // prevents a caller from deleting evidence while a receipt transition owns the slot.
    let held = h.daemon.borrow_entry(&h.entry_id).expect("borrowed");
    let refused = h
        .daemon
        .abandon(&h.entry_id, "the participant changed their mind")
        .expect_err("an entry whose runtime is mid-binding is not abandonable");
    assert!(format!("{refused}").contains("mid-upload"), "got {refused}");
    drop(held);

    // With the runtime back, the durable started state takes over.
    let still = h
        .daemon
        .abandon(&h.entry_id, "the participant changed their mind")
        .expect_err("a started run ends by sealing");
    let text = format!("{still}");
    assert!(
        text.contains("has started and cannot be abandoned"),
        "the durable refusal names the state: {text}"
    );

    // Nothing was taken: the row, the map and the evidence all stand.
    assert!(chain.is_file(), "a refused abandon removed the chain");
    assert!(h.trace_dir.join("baseline.json").is_file());
    assert!(
        h.daemon
            .registry()
            .get(&h.entry_id)
            .expect("the registry reads")
            .is_some(),
        "the row is the tombstone, and it was never written"
    );
    assert!(
        h.daemon.entries().expect("the entries list").len() == 1,
        "the entry is still open in this daemon"
    );
}

/// The other half: an entry that never started is abandonable, and the row goes. A refusal
/// that refused everything would pass the case above and break "failure before
/// dispatch leaves no entry".
#[tokio::test]
async fn an_entry_that_never_started_is_still_abandoned_row_and_all() {
    let h = support::started_entry_with_work().await;
    let runtime = support::second_entry(&h, "e2").await;
    h.daemon.register(runtime).expect("the second entry opens");

    h.daemon
        .abandon("e2", "the launch never took")
        .expect("it is abandoned");

    assert!(
        h.daemon
            .registry()
            .get("e2")
            .expect("the registry reads")
            .is_none(),
        "an unstarted entry leaves no row"
    );
    assert!(h.daemon.is_sealed("e2").is_err(), "and no entry at all");
}

/// A sealed run leaves the entry map: kept there, its redb
/// `Buffer`, its registry handle, its chain writer and every tailer descriptor live for the life
/// of the process, and many short runs under one long-lived daemon exhaust file descriptors.
///
/// The condition is the last upload and not the seal: a sealed entry's remaining work is
/// draining its buffer, which writes to `acks.jsonl` rather than to the chain, and an entry
/// evicted before that would strand its own last chunk. The registry row stays, which is
/// what keeps `is_sealed` answering.
#[tokio::test]
async fn a_sealed_entry_leaves_the_map_once_its_last_chunk_is_acknowledged() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    assert!(
        h.daemon
            .entries()
            .expect("the list")
            .iter()
            .any(|e| e.entry_id == h.entry_id),
        "a seal alone evicts nothing: the buffer still has to drain"
    );

    // The uploader is the one cadence a sealed entry still runs. Pumped until the verified
    // watermark reaches the chain head, which is the seal's own link.
    let drained = Instant::now() + Duration::from_secs(10);
    while Instant::now() < drained {
        h.daemon.pump_all().await.expect("the pump runs");
        if h.daemon.entries().expect("the list").is_empty() {
            break;
        }
        // A beat between attempts: the pump is a network round trip on this thread and a
        // loop with no yield in it spends ten seconds re-asking a server that is answering
        // as fast as it can.
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert!(
        h.daemon.entries().expect("the list").is_empty(),
        "the drained entry is still holding its buffer, its tailers and its registry handle"
    );
    // The row stays, and it is what every later question is answered from.
    let row = h
        .daemon
        .registry()
        .get(&h.entry_id)
        .expect("the registry reads")
        .expect("the row outlives the map");
    assert!(row.sealed_at.is_some());
    assert!(
        h.daemon.is_sealed(&h.entry_id).expect("asked of the row"),
        "an evicted entry is still a sealed one"
    );
    // And so does the `SealReport`, which is per-run state with a durable
    // copy — `seal.json`, the chain, and `SealOutcome` on the row — and is therefore the
    // same unbounded growth the map would be.
    //
    // The signed status document is the exception, and it is kept.
    // One frozen `SignedStatus` per sealed entry is bounded by the entry count rather than
    // by the run, and dropping it would make `goalcup status` answer `404` for a run that had
    // just finished cleanly.
    let last = h
        .daemon
        .signed_status(&h.entry_id)
        .expect("the cache")
        .expect("a finished run still answers with the document it ended on");
    assert_eq!(last.payload.entry_id, h.entry_id);
    assert!(
        last.payload.started_at.is_some(),
        "and it is the STARTED run's own last document, not a stub"
    );
    assert!(
        h.daemon
            .seal_report(&h.entry_id)
            .expect("the seal map")
            .is_none(),
        "the in-memory seal report outlived the entry it is about"
    );
    assert!(
        row.seal_outcome.is_some(),
        "and what the seal learned is on the ROW, which is the copy that survives"
    );
    assert_eq!(
        row.last_ack_seq, row.chain_seq,
        "and it drained to the head"
    );
}

/// A restart does not rebuild a terminal runtime — its tailers must stay closed — but it
/// must not strand the exact encrypted terminal envelope that the interrupted daemon had
/// not yet uploaded. The upload-only recovery owner supplies the same retry path without
/// reopening the sealed workspace.
#[tokio::test]
async fn a_restarted_daemon_drains_an_unacknowledged_sealed_terminal() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let before = h.entry();
    assert_ne!(
        before.last_ack_seq, before.chain_seq,
        "the terminal envelope is deliberately still pending before the restart"
    );

    let h = support::restart_daemon(h).await;
    assert!(
        h.daemon.entries().expect("the live capture map").is_empty(),
        "a terminal row is not rehydrated as a tailing runtime"
    );
    let restarted_status = h
        .daemon
        .signed_status(&h.entry_id)
        .expect("the startup status cache reads")
        .expect("startup immediately republishes the capture-closed row");
    assert_eq!(
        restarted_status.payload.health,
        goalcup_protocol::events::HealthState::Sealed
    );
    assert_eq!(
        restarted_status.payload.arming,
        goalcup_protocol::control::ArmingState::Started,
        "terminal state must not erase the durable start fact"
    );
    assert_eq!(restarted_status.payload.seconds_remaining, Some(0));
    assert_eq!(
        restarted_status.payload.session_id.as_deref(),
        Some(support::DAEMON_SESSION)
    );
    // The human-facing terminal text does not render its acknowledgement counters, so a
    // clean ACK can legitimately produce identical normal bytes.  Replace it with an
    // unmistakably stale view: the assertion below proves that the recovered owner still
    // performs its rooted atomic replacement, rather than merely updating the signed cache.
    std::fs::write(canonical_status_line(&h), b"STALE_TERMINAL_VIEW\n")
        .expect("the stale fixture line is written");
    h.daemon
        .pump_all()
        .await
        .expect("the recovered terminal buffer drains");

    let after = h.entry();
    assert_eq!(after.last_ack_seq, after.chain_seq);
    assert!(
        after.events_uploaded > before.events_uploaded,
        "the recovered drain counted the terminal link exactly once"
    );
    let signed = h
        .daemon
        .signed_status(&h.entry_id)
        .expect("the status cache reads")
        .expect("the restarted drain republishes acknowledgement progress");
    assert_eq!(signed.payload.events_uploaded, after.events_uploaded);
    assert!(
        std::fs::read(canonical_status_line(&h))
            .expect("the recovered acknowledgement keeps a terminal status line")
            .starts_with(b"/goal Cup "),
        "recovered acknowledgement progress replaces a stale participant line through its retained drain"
    );
}

/// A recovered drain retains the entry it opened during its first offline cadence.  Its
/// ordinary (not-yet-terminal) acknowledgement must replace the participant line in that
/// detached entry, while its buffer and trace append through the same retained authority.
#[cfg(unix)]
#[tokio::test]
async fn a_retained_drain_publishes_partial_ack_in_its_detached_entry_after_a_swap() {
    use std::os::unix::fs::symlink;

    let mut h = support::started_entry_with_work().await;
    // The restart owns the recovered uploader. One envelope per chunk leaves a real
    // ordinary-ACK cadence before the terminal completion, rather than collapsing both
    // status paths into the default all-at-once drain.
    h.config.upload_chunk_max = 1;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let h = support::restart_daemon(h).await;
    h.set_upload_offline(true);
    h.daemon
        .pump_all()
        .await
        .expect("the offline cadence constructs the retained drain");
    h.set_upload_offline(false);
    h.daemon
        .resume_retained_upload_for_test(&h.entry_id)
        .expect("the real retained uploader leaves its offline backoff");

    let outside = h.config.config_dir.join("outside-partial-retained-drain");
    std::fs::create_dir_all(outside.join("trace")).expect("outside trace directory");
    std::fs::write(outside.join("status-line.txt"), b"OUTSIDE_STATUS_SENTINEL")
        .expect("outside status sentinel");
    std::fs::write(outside.join("buffer.redb"), b"OUTSIDE_BUFFER_SENTINEL")
        .expect("outside buffer sentinel");
    std::fs::write(outside.join("trace/acks.jsonl"), b"OUTSIDE_ACKS_SENTINEL")
        .expect("outside trace sentinel");
    let detached = h.config.config_dir.join("detached-partial-retained-drain");
    std::fs::rename(&h.artifact_dir, &detached).expect("detach the retained entry");
    symlink(&outside, &h.artifact_dir).expect("replace the mutable entry spelling");
    std::fs::write(detached.join("status-line.txt"), b"STALE_PARTIAL_STATUS\n")
        .expect("the stale detached status is planted");
    let detached_buffer_before_partial =
        std::fs::read(detached.join("buffer.redb")).expect("the retained buffer is present");
    let detached_acks_before_partial =
        std::fs::read(detached.join("trace/acks.jsonl")).unwrap_or_default();

    let before = h.entry();
    let terminal = before.chain_seq.expect("the terminal sequence");
    h.daemon
        .pump_all()
        .await
        .expect("the first online retained chunk succeeds");
    let partial = h.entry();
    assert!(
        partial.last_ack_seq > before.last_ack_seq && partial.last_ack_seq < Some(terminal),
        "the first one-envelope chunk records real nonterminal ACK progress"
    );
    assert!(
        std::fs::read(detached.join("status-line.txt"))
            .expect("the retained entry line reads")
            .starts_with(b"/goal Cup "),
        "ordinary recovered status replaces a stale detached line"
    );
    assert_ne!(
        std::fs::read(detached.join("buffer.redb")).expect("the retained buffer changes"),
        detached_buffer_before_partial,
        "the acknowledged partial chunk advances the detached retained buffer"
    );
    let detached_acks_after_partial =
        std::fs::read(detached.join("trace/acks.jsonl")).expect("the retained ACK trace exists");
    assert!(
        detached_acks_after_partial.len() > detached_acks_before_partial.len(),
        "the acknowledged partial chunk appends an ACK in the detached retained trace"
    );
    assert_eq!(
        std::fs::read(outside.join("status-line.txt")).expect("outside status survives"),
        b"OUTSIDE_STATUS_SENTINEL"
    );
    assert_eq!(
        std::fs::read(outside.join("buffer.redb")).expect("outside buffer survives"),
        b"OUTSIDE_BUFFER_SENTINEL"
    );
    assert_eq!(
        std::fs::read(outside.join("trace/acks.jsonl")).expect("outside trace survives"),
        b"OUTSIDE_ACKS_SENTINEL"
    );

    std::fs::write(detached.join("status-line.txt"), b"STALE_CLEAN_STATUS\n")
        .expect("the stale clean-completion line is planted after the partial acknowledgement");
    while h.entry().last_ack_seq < Some(terminal) {
        h.daemon
            .pump_all()
            .await
            .expect("later retained chunks complete through their held entry");
    }
    assert!(
        std::fs::read(detached.join("status-line.txt"))
            .expect("the clean retained completion keeps the detached line")
            .starts_with(b"/goal Cup "),
        "clean completion replaces the post-partial stale line through the retained entry"
    );
    for (relative, sentinel) in [
        ("status-line.txt", b"OUTSIDE_STATUS_SENTINEL".as_slice()),
        ("buffer.redb", b"OUTSIDE_BUFFER_SENTINEL".as_slice()),
        ("trace/acks.jsonl", b"OUTSIDE_ACKS_SENTINEL".as_slice()),
    ] {
        assert_eq!(
            std::fs::read(outside.join(relative)).expect("outside sentinel survives"),
            sentinel,
            "clean completion must not follow the swapped entry to outside {relative}"
        );
    }
}

/// Older sealed rows did not persist the opaque trace-root identity.  A restart must not
/// strand their final acknowledgement, but may fill that field only after it has validated
/// the old trace's terminal link against the registry head.
#[tokio::test]
async fn a_legacy_sealed_row_migrates_its_validated_root_before_restart_drain() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    h.daemon
        .registry()
        .clear_sealed_trace_root_identity_for_test(&h.entry_id)
        .expect("the historical fixture clears only the root identity");
    let h = support::restart_daemon(h).await;
    h.daemon
        .pump_all()
        .await
        .expect("the guarded legacy terminal drain succeeds");
    assert!(
        h.daemon
            .registry()
            .sealed_trace_root_identity_for_test(&h.entry_id)
            .expect("the migrated row reads")
            .is_some(),
        "the validated legacy row is migrated before its buffer is acknowledged"
    );
    let row = h.entry();
    assert_eq!(row.last_ack_seq, row.chain_seq);
}

#[tokio::test]
async fn restart_marks_a_registered_sealed_row_without_a_handle_as_reap_pending() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    h.daemon
        .registry()
        .mark_registered_without_termination_handle_for_test(&h.entry_id)
        .expect("the corrupt registered fixture is durable");
    let h = support::restart_daemon(h).await;
    let outcome = h
        .entry()
        .seal_outcome
        .expect("the reaper records an unverified durable outcome");
    assert!(outcome.harness_unverified);
    assert!(outcome.termination_error.as_deref().is_some_and(|reason| {
        reason.contains("registered without a persisted termination handle")
    }));
}

#[tokio::test]
async fn a_restarted_sealed_drain_refuses_any_buffer_tail_after_its_terminal_head() {
    use goalcup_daemon::buffer::Buffer;

    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let before = h.entry();
    let mut after_terminal = support::read_chain(&h.trace_dir)
        .pop()
        .expect("the trace has its terminal envelope");
    after_terminal.link.seq = before.chain_seq.expect("the terminal seq") + 1;
    after_terminal.link.prev = before.chain_head.clone();
    after_terminal.link.hash = "ff".repeat(32);
    let h = support::restart_daemon_after_stop(h, |row| {
        let buffer = Buffer::open(
            &Buffer::default_path(&row.artifact_dir),
            &row.keys.buffer_key_hex,
        )
        .expect("the stopped sealed buffer opens");
        buffer
            .append(&after_terminal)
            .expect("the corrupt tail is staged");
    })
    .await;
    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("a sealed drain must not upload a record after its terminal head");
    assert!(error.to_string().contains("after terminal"));
    let after = h.entry();
    assert_eq!(after.last_ack_seq, before.last_ack_seq);
    let status = h
        .daemon
        .signed_status(&h.entry_id)
        .expect("the signed status reads")
        .expect("the corrupt capture-closed row is republished");
    assert_eq!(
        status.payload.health,
        goalcup_protocol::events::HealthState::SealingRecoveryNeeded,
        "a post-terminal tail is terminal upload validation failure, not a sealed all-clear"
    );
}

#[tokio::test]
async fn a_restarted_sealed_drain_refuses_a_buffer_watermark_above_its_terminal() {
    use goalcup_daemon::buffer::Buffer;

    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let before = h.entry();
    let terminal = before.chain_seq.expect("the terminal seq");
    let h = support::restart_daemon_after_stop(h, |row| {
        let buffer = Buffer::open(
            &Buffer::default_path(&row.artifact_dir),
            &row.keys.buffer_key_hex,
        )
        .expect("the stopped sealed buffer opens");
        buffer
            .ack(terminal + 1)
            .expect("the corrupt high watermark is durable");
    })
    .await;

    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("a terminal drain must reject a watermark beyond its head");
    assert!(error.to_string().contains("acknowledgement"), "{error}");
    let after = h.entry();
    assert!(
        after.last_ack_seq.is_none_or(|acked| acked <= terminal),
        "the corrupt buffer floor may not be copied into registry acknowledgement state"
    );
    assert_eq!(after.last_ack_seq, before.last_ack_seq);
}

/// A terminal ceiling constrains the durable registry watermark as well as the encrypted
/// buffer.  Otherwise a corrupt row can make a live runtime report an impossible
/// acknowledgement without asking the buffer or the uploader any question.
#[tokio::test]
async fn a_live_sealed_runtime_refuses_a_registry_acknowledgement_beyond_its_terminal() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let terminal = h.entry().chain_seq.expect("the terminal sequence");
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| {
            entry.last_ack_seq = Some(terminal + 1);
        })
        .expect("the corrupt durable watermark is staged");

    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("a live terminal pump rejects an acknowledgement beyond its ceiling");
    assert!(
        error.to_string().contains("acknowledges beyond terminal"),
        "{error}"
    );
    assert_eq!(h.entry().last_ack_seq, Some(terminal + 1));
    assert_terminal_drain_review(&h);
}

/// A row that says its terminal was already acknowledged is not exempt from the sealed
/// buffer invariant.  A crash can leave that registry update durable while a later corrupt
/// buffer watermark remains; startup must find it before serving the restarted daemon, and
/// every cadence must continue to refuse it.
#[tokio::test]
async fn a_fully_acked_sealed_row_still_rejects_a_buffer_watermark_beyond_its_terminal() {
    use goalcup_daemon::buffer::Buffer;

    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let terminal = h.entry().chain_seq.expect("the terminal sequence");
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| {
            entry.last_ack_seq = Some(terminal);
        })
        .expect("the fully acknowledged fixture is durable");
    let h = support::restart_daemon_after_stop(h, |row| {
        let buffer = Buffer::open(
            &Buffer::default_path(&row.artifact_dir),
            &row.keys.buffer_key_hex,
        )
        .expect("the stopped sealed buffer opens");
        buffer
            .ack(terminal + 1)
            .expect("the corrupt high watermark is durable");
    })
    .await;

    assert!(
        h.daemon.cycle_errors().upload > 0,
        "startup validates a fully acknowledged sealed drain before listeners serve it"
    );
    let startup_status = h
        .daemon
        .signed_status(&h.entry_id)
        .expect("the signed status reads")
        .expect("startup republishes the capture-closed row");
    assert_eq!(
        startup_status.payload.health,
        goalcup_protocol::events::HealthState::SealingRecoveryNeeded,
        "a corrupt terminal buffer cannot be published as an all-clear sealed status"
    );
    assert!(
        startup_status
            .payload
            .detail
            .contains("terminal upload validation failed"),
        "the signed participant/API surface names the elevated-review condition: {:?}",
        startup_status.payload
    );
    assert_eq!(
        mcp_source(&h)
            .for_entry(&h.entry_id)
            .expect("startup writes the signed review to the actual MCP cache reader")
            .payload
            .health,
        goalcup_protocol::events::HealthState::SealingRecoveryNeeded,
        "startup validation publishes review before subsequent startup work can serve a stale cache"
    );
    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("upload cadence must not skip a fully acknowledged corrupt drain");
    assert!(error.to_string().contains("acknowledgement"), "{error}");
    assert_eq!(h.entry().last_ack_seq, Some(terminal));
    let later_status = h
        .daemon
        .signed_status(&h.entry_id)
        .expect("the signed status reads after the later cadence")
        .expect("a failed cadence cannot evict the quarantine status");
    assert_eq!(
        later_status.payload.health,
        goalcup_protocol::events::HealthState::SealingRecoveryNeeded,
        "a later cadence may not erase the durable upload-validation quarantine"
    );
}

/// The retained object is deliberately constructed before the durable row is corrupted.
/// That is the stale-cache race: a pump which trusts its reconstructed `Entry` can send and
/// overwrite the newer impossible acknowledgement instead of refusing it before it touches
/// the buffer, registry, or signed status cache.
#[tokio::test]
async fn a_retained_sealed_drain_rereads_an_impossible_registry_ack_before_every_pump() {
    use goalcup_daemon::buffer::Buffer;

    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let h = support::restart_daemon(h).await;
    h.set_upload_offline(true);
    h.daemon
        .pump_all()
        .await
        .expect("the first offline cadence retains its real recovered drain");
    h.set_upload_offline(false);

    let terminal = h.entry().chain_seq.expect("the terminal seq");
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| entry.last_ack_seq = Some(terminal + 1))
        .expect("the later impossible durable acknowledgement is staged");
    let buffer_path = Buffer::default_path(&h.artifact_dir);
    let buffer_bytes_before = std::fs::read(&buffer_path)
        .expect("the retained encrypted buffer is readable without taking its redb lock");
    let acks_before = std::fs::read(h.trace_dir.join("acks.jsonl")).expect("acks read");
    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("the retained drain re-reads its durable acknowledgement ceiling");
    assert!(
        error.to_string().contains("acknowledges beyond terminal"),
        "{error}"
    );
    assert_eq!(
        h.entry().last_ack_seq,
        Some(terminal + 1),
        "the stale retained copy may not overwrite the newer impossible durable row"
    );
    assert_eq!(
        std::fs::read(&buffer_path).expect("the retained buffer rereads"),
        buffer_bytes_before,
        "the refusal did not write or acknowledge the encrypted buffer"
    );
    assert_eq!(
        std::fs::read(h.trace_dir.join("acks.jsonl")).expect("acks reread"),
        acks_before,
        "the refusal appended no acknowledgement"
    );
    assert_terminal_drain_review(&h);
}

#[tokio::test]
async fn an_unproven_live_sealing_terminal_is_never_uploaded_or_acknowledged() {
    let h = support::started_entry_with_work().await;
    let planned = h
        .daemon
        .begin_live_sealing_without_terminal_for_test(&h.entry_id)
        .expect("the fixture stages a seal without letting its terminal writer run");
    let before = h.entry();
    let predecessor = before.chain_seq.expect("the pre-seal head is durable");
    assert!(planned.link.seq > predecessor);
    h.daemon
        .append_live_buffer_for_test(&h.entry_id, &planned)
        .expect("the unproven planned terminal is staged only in the buffer fixture");

    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("a live Sealing runtime may not send the planned terminal");
    assert!(error.to_string().contains("after terminal"), "{error}");
    let after = h.entry();
    assert!(
        after.last_ack_seq.is_none_or(|acked| acked <= predecessor),
        "only trace-proven pre-seal history can be acknowledged while intent is unresolved"
    );
}

/// Startup owns no predecessor runtime, so an interrupted seal must acquire the configured
/// entry afresh before it decides whether the incomplete intent can be restored.  A swapped
/// entry is a refusal, not permission to inspect or repair an outside trace/buffer.
#[cfg(unix)]
#[tokio::test]
async fn unresolved_sealing_recovery_refuses_a_swapped_entry_without_touching_outside() {
    use std::os::unix::fs::symlink;

    let h = support::started_entry_with_work().await;
    h.daemon
        .begin_live_sealing_without_terminal_for_test(&h.entry_id)
        .expect("the fixture stages the unresolved sealing intent");
    std::fs::write(
        h.artifact_dir.join("status-line.txt"),
        b"DETACHED_STATUS_BEFORE_RECOVERY",
    )
    .expect("the retained pre-restart status fixture is written");
    let outside = h
        .config
        .config_dir
        .join("outside-unresolved-sealing-recovery");
    std::fs::create_dir_all(outside.join("trace")).expect("outside trace directory");
    std::fs::write(outside.join("trace/chain.jsonl"), b"OUTSIDE_CHAIN_SENTINEL")
        .expect("outside chain sentinel");
    std::fs::write(outside.join("buffer.redb"), b"OUTSIDE_BUFFER_SENTINEL")
        .expect("outside buffer sentinel");
    std::fs::write(outside.join("status-line.txt"), b"OUTSIDE_STATUS_SENTINEL")
        .expect("outside status sentinel");
    let detached = h
        .config
        .config_dir
        .join("detached-unresolved-sealing-entry");
    let artifact_dir = h.artifact_dir.clone();
    let outside_for_restart = outside.clone();
    let detached_for_restart = detached.clone();
    let mut detached_evidence_before_restart = None;

    let h = support::restart_daemon_after_stop(h, |_| {
        detached_evidence_before_restart = Some((
            std::fs::read(artifact_dir.join("trace/chain.jsonl"))
                .expect("the stopped trace is present before successor startup"),
            std::fs::read(artifact_dir.join("buffer.redb"))
                .expect("the stopped buffer is present before successor startup"),
            std::fs::read(artifact_dir.join("status-line.txt"))
                .expect("the stopped status is present before successor startup"),
        ));
        std::fs::rename(&artifact_dir, &detached_for_restart)
            .expect("the entry is detached after the old owner stops");
        symlink(&outside_for_restart, &artifact_dir)
            .expect("the mutable entry spelling is replaced before startup recovery");
    })
    .await;
    let (detached_chain_before, detached_buffer_before, detached_status_before) =
        detached_evidence_before_restart
            .expect("the detached evidence is snapshotted before refused successor startup");

    assert_eq!(
        h.entry().state,
        goalcup_daemon::registry::EntryState::Sealing,
        "startup refuses the replaced entry instead of restoring or promoting it"
    );
    let no_runtime = h
        .daemon
        .seal_entry(&h.entry_id)
        .expect_err("the swapped capture-closed row is not rehydrated as a live runtime");
    assert!(
        no_runtime.to_string().contains("no entry"),
        "a registry-only Sealing summary is not a mutable runtime: {no_runtime}"
    );
    for (relative, sentinel) in [
        ("trace/chain.jsonl", b"OUTSIDE_CHAIN_SENTINEL".as_slice()),
        ("buffer.redb", b"OUTSIDE_BUFFER_SENTINEL".as_slice()),
        ("status-line.txt", b"OUTSIDE_STATUS_SENTINEL".as_slice()),
    ] {
        assert_eq!(
            std::fs::read(outside.join(relative)).expect("outside sentinel remains"),
            sentinel,
            "unresolved sealing recovery must not follow swapped {relative}"
        );
    }
    for (relative, before) in [
        ("trace/chain.jsonl", detached_chain_before.as_slice()),
        ("buffer.redb", detached_buffer_before.as_slice()),
        ("status-line.txt", detached_status_before.as_slice()),
    ] {
        assert!(
            std::fs::read(detached.join(relative)).expect("the detached evidence remains")
                == before,
            "refused recovery must not rewrite detached {relative}"
        );
    }
}

/// A pause is test-only, but it must be owned by the individual runtime it exercises.
/// Independent daemons legitimately start their first entry as `e1`; a process-global
/// pause keyed only by that display id lets one test capture another daemon's seal.
#[cfg(unix)]
#[tokio::test]
async fn live_seal_write_pauses_are_scoped_to_their_own_daemon() {
    let first = support::started_entry_with_work().await;
    let second = support::started_entry_with_work().await;

    first
        .daemon
        .pause_after_live_seal_write_for_test(&first.entry_id)
        .expect("the first daemon arms its pause");
    second
        .daemon
        .pause_after_live_seal_write_for_test(&second.entry_id)
        .expect("a separate daemon with the same entry id owns an independent pause");
}

/// The pause is only for a terminal that the retained writer actually appended. A failed
/// `write_seal` must return normally rather than reach a wait that could strand the worker.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_live_seal_never_enters_its_postwrite_pause() {
    let h = support::started_entry_with_work().await;
    let pause = h
        .daemon
        .pause_after_live_seal_write_for_test(&h.entry_id)
        .expect("the post-write pause arms on the retained runtime");
    let mut held = h
        .daemon
        .borrow_entry(&h.entry_id)
        .expect("the live runtime is borrowed for writer failure injection");
    held.get()
        .expect("the live runtime remains present")
        .chain_writer_mut()
        .expect("the trace writer is retained")
        .fail_append_after(0);
    drop(held);

    let sealing = h.daemon.clone_handle();
    let entry_id = h.entry_id.clone();
    let task = tokio::task::spawn_blocking(move || sealing.seal_entry(&entry_id));
    let wait_error = pause
        .wait_until_paused()
        .expect_err("a failed terminal writer never enters the post-write pause");
    assert!(
        wait_error
            .to_string()
            .contains("timed out waiting for live seal write pause"),
        "the bounded wait explains that no successful terminal write reached it: {wait_error}"
    );
    drop(pause);
    let seal_error = task
        .await
        .expect("the failed seal worker joins after the guard releases it")
        .expect_err("the injected terminal writer failure reaches the caller");
    assert!(
        seal_error.to_string().contains("injected append failure"),
        "the writer failure, rather than a blocked test pause, remains the seal outcome: {seal_error}"
    );
}

/// A live seal writes its terminal link before it promotes the registry row.  That tiny
/// interval still belongs to the runtime that acquired the entry: replacing the display
/// entry while the writer is paused cannot redirect the promotion's reopen, buffer append,
/// or final participant status.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_seal_promotion_keeps_the_entry_acquired_before_its_terminal_write() {
    use std::os::unix::fs::symlink;

    let h = support::started_entry_with_work().await;
    let pause = h
        .daemon
        .pause_after_live_seal_write_for_test(&h.entry_id)
        .expect("the deterministic pre-promotion pause arms");
    let sealing = h.daemon.clone_handle();
    let entry_id = h.entry_id.clone();
    let task = tokio::task::spawn_blocking(move || sealing.seal_entry(&entry_id));
    pause
        .wait_until_paused()
        .expect("the deterministic pre-promotion pause is reached after a successful write");

    let outside = h.config.config_dir.join("outside-live-seal-promotion");
    std::fs::create_dir_all(outside.join("trace")).expect("outside trace directory");
    std::fs::write(outside.join("trace/chain.jsonl"), b"OUTSIDE_CHAIN_SENTINEL")
        .expect("outside chain sentinel");
    std::fs::write(
        outside.join("trace/trace.toml"),
        b"OUTSIDE_DESCRIPTOR_SENTINEL",
    )
    .expect("outside descriptor sentinel");
    std::fs::write(outside.join("buffer.redb"), b"OUTSIDE_BUFFER_SENTINEL")
        .expect("outside buffer sentinel");
    std::fs::write(outside.join("status-line.txt"), b"OUTSIDE_STATUS_SENTINEL")
        .expect("outside status sentinel");
    let detached = h.config.config_dir.join("detached-live-seal-promotion");
    std::fs::rename(&h.artifact_dir, &detached).expect("the acquired entry is detached");
    symlink(&outside, &h.artifact_dir).expect("the mutable entry spelling is replaced");
    std::fs::write(
        detached.join("trace/trace.toml"),
        b"STALE_LIVE_DESCRIPTOR_AFTER_TERMINAL",
    )
    .expect("the stale descriptor is planted while the terminal is paused");
    std::fs::write(
        detached.join("status-line.txt"),
        b"STALE_LIVE_STATUS_AFTER_TERMINAL",
    )
    .expect("the stale status is planted while the terminal is paused");
    let detached_buffer_before_promotion =
        std::fs::read(detached.join("buffer.redb")).expect("the pre-promotion buffer reads");

    pause
        .release()
        .expect("the terminal promotion may continue");
    task.await
        .expect("the blocking seal task joins")
        .expect("the retained live seal promotes through its held entry");

    assert_eq!(
        h.entry().state,
        goalcup_daemon::registry::EntryState::Sealed,
        "the detached retained entry completes its durable terminal promotion"
    );
    assert!(detached.join("trace/seal.json").is_file());
    assert_ne!(
        std::fs::read(detached.join("buffer.redb")).expect("the detached buffer reads"),
        detached_buffer_before_promotion,
        "post-pause terminal promotion appends the terminal to its detached buffer"
    );
    assert_ne!(
        std::fs::read(detached.join("trace/trace.toml")).expect("the detached descriptor reads"),
        b"STALE_LIVE_DESCRIPTOR_AFTER_TERMINAL",
        "post-pause descriptor repair uses the held trace authority"
    );
    assert!(
        std::fs::read(detached.join("status-line.txt"))
            .expect("the detached status line reads")
            .starts_with(b"/goal Cup ")
    );
    for (relative, sentinel) in [
        ("trace/chain.jsonl", b"OUTSIDE_CHAIN_SENTINEL".as_slice()),
        (
            "trace/trace.toml",
            b"OUTSIDE_DESCRIPTOR_SENTINEL".as_slice(),
        ),
        ("buffer.redb", b"OUTSIDE_BUFFER_SENTINEL".as_slice()),
        ("status-line.txt", b"OUTSIDE_STATUS_SENTINEL".as_slice()),
    ] {
        assert_eq!(
            std::fs::read(outside.join(relative)).expect("outside sentinel remains"),
            sentinel,
            "live terminal promotion must not follow swapped {relative}"
        );
    }
}

#[tokio::test]
async fn a_recovered_buffer_ack_repairs_registry_and_republishes_status_without_uploading() {
    use goalcup_daemon::buffer::Buffer;

    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let before = h.entry();
    let terminal = before.chain_seq.expect("the terminal seq");
    assert_ne!(before.last_ack_seq, Some(terminal));
    let h = support::restart_daemon_after_stop(h, |row| {
        let buffer = Buffer::open(
            &Buffer::default_path(&row.artifact_dir),
            &row.keys.buffer_key_hex,
        )
        .expect("the stopped sealed buffer opens");
        buffer
            .ack(terminal)
            .expect("the crash-window buffer acknowledgement is durable");
    })
    .await;

    h.daemon
        .pump_all()
        .await
        .expect("the recovered drain repairs registry-only acknowledgement state");
    let after = h.entry();
    assert_eq!(after.last_ack_seq, Some(terminal));
    let expected_ack_delta = terminal + 1 - before.last_ack_seq.map_or(0, |seq| seq + 1);
    assert_eq!(
        after.events_uploaded,
        before.events_uploaded + expected_ack_delta,
        "a durable buffer acknowledgement repaired after a crash still counts every newly \
         acknowledged event even though this recovery uploaded no bytes"
    );
    let signed = h
        .daemon
        .signed_status(&h.entry_id)
        .expect("the corrected cache reads")
        .expect("the recovered drain republishes even with zero uploaded envelopes");
    assert_eq!(signed.payload.events_pending, 0);
    assert_eq!(signed.payload.events_uploaded, after.events_uploaded);
}

/// `Buffer::ack` commits before the registry mirror.  The ordinary live runtime therefore
/// has the same zero-byte repair shape as a restart drain: its counters must advance by the
/// durable watermark delta, and its signed status must be replaced in the same pump.
#[tokio::test]
async fn a_live_runtime_repairs_zero_byte_ack_progress_and_republishes_its_signed_status() {
    let h = support::started_entry_with_work().await;
    let before = h.entry();
    let head = before.chain_seq.expect("the started run has a chain head");
    h.daemon
        .ack_live_buffer_for_test(&h.entry_id, head)
        .expect("the crash-window buffer acknowledgement is staged without its registry mirror");

    h.daemon
        .pump_all()
        .await
        .expect("the zero-byte live retry repairs durable acknowledgement state");
    let after = h.entry();
    assert_eq!(after.last_ack_seq, Some(head));
    let expected_delta = head + 1 - before.last_ack_seq.map_or(0, |seq| seq + 1);
    assert_eq!(
        after.events_uploaded,
        before.events_uploaded + expected_delta,
        "the counter follows the durable acknowledgement watermark, not this pump's bytes"
    );
    let signed = h
        .daemon
        .signed_status(&h.entry_id)
        .expect("the signed status reads")
        .expect("the repair synchronously republishes its status");
    assert_eq!(
        signed.payload.last_sync_at,
        after.last_ack_at.map(goalcup_daemon::unix_to_rfc3339)
    );
    assert_eq!(signed.payload.events_uploaded, after.events_uploaded);
    assert_eq!(signed.payload.events_pending, 0);
}

/// The terminal ceiling belongs to the live runtime too: a seal that has not yet been evicted
/// must not upload a corrupt tail on the very next cadence.
#[tokio::test]
async fn a_live_sealed_runtime_never_uploads_a_buffer_record_after_its_terminal_head() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let before = h.entry();
    let mut after_terminal = support::read_chain(&h.trace_dir)
        .pop()
        .expect("the trace has its terminal envelope");
    after_terminal.link.seq = before.chain_seq.expect("the terminal seq") + 1;
    after_terminal.link.prev = before.chain_head.clone();
    after_terminal.link.hash = "ee".repeat(32);
    h.daemon
        .append_live_buffer_for_test(&h.entry_id, &after_terminal)
        .expect("the live fixture stages the corrupt tail");

    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("the live uploader must reject a post-terminal buffer tail");
    assert!(error.to_string().contains("after terminal"), "{error}");
    assert_eq!(h.entry().last_ack_seq, before.last_ack_seq);
    assert_terminal_drain_review(&h);
}

/// A recovered drain must keep checking terminal storage after it was retained for uploader
/// backoff.  A post-start corrupt floor has no live entry owner to repair it, so the retained
/// owner must durably quarantine and republish the same capture-closed review status.
#[tokio::test]
async fn a_retained_sealed_drain_quarantines_a_post_start_buffer_floor() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let h = support::restart_daemon(h).await;
    h.set_upload_offline(true);
    h.daemon
        .pump_all()
        .await
        .expect("the offline cadence constructs its retained drain");
    h.set_upload_offline(false);

    let terminal = h.entry().chain_seq.expect("the terminal sequence");
    h.daemon
        .ack_retained_buffer_for_test(&h.entry_id, terminal + 1)
        .expect("the retained floor corruption is staged after daemon start");
    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("a retained floor above terminal is refused");
    assert!(
        error
            .to_string()
            .contains("buffer acknowledgement exceeds terminal")
    );
    assert_terminal_drain_review(&h);
}

/// The same retained owner must quarantine a newly staged post-terminal tail.  This is
/// distinct from the floor: it proves the sealed buffer cannot hide a later record behind a
/// perfectly ordinary acknowledgement watermark.
#[tokio::test]
async fn a_retained_sealed_drain_quarantines_a_post_start_buffer_tail() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let h = support::restart_daemon(h).await;
    h.set_upload_offline(true);
    h.daemon
        .pump_all()
        .await
        .expect("the offline cadence constructs its retained drain");
    h.set_upload_offline(false);
    h.daemon
        .resume_retained_upload_for_test(&h.entry_id)
        .expect("the post-pump ordering fixture clears retained backoff");

    let before = h.entry();
    let mut after_terminal = support::read_chain(&h.trace_dir)
        .pop()
        .expect("the trace has its terminal envelope");
    after_terminal.link.seq = before.chain_seq.expect("the terminal sequence") + 1;
    after_terminal.link.prev = before.chain_head;
    after_terminal.link.hash = "ab".repeat(32);
    h.daemon
        .append_retained_buffer_for_test(&h.entry_id, &after_terminal)
        .expect("the retained tail corruption is staged after daemon start");
    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("a retained post-terminal tail is refused");
    assert!(
        error
            .to_string()
            .contains("cannot drain beyond its terminal")
    );
    assert_terminal_drain_review(&h);
}

/// The successful upload result is not a licence to publish an all-clear before the retained
/// owner has made its final drain check.  A tail that appears in that narrow interval must
/// turn the same cadence into review status, rather than relying on the next cadence to
/// repair a stale zero-pending cache.
#[tokio::test]
async fn a_recovered_pump_republishes_terminal_review_when_its_final_drain_check_fails() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let h = support::restart_daemon(h).await;
    h.set_upload_offline(true);
    h.daemon
        .pump_all()
        .await
        .expect("the offline cadence constructs its retained drain");
    h.set_upload_offline(false);
    h.daemon
        .resume_retained_upload_for_test(&h.entry_id)
        .expect("the post-pump ordering fixture clears retained backoff");

    let before = h.entry();
    let mut after_terminal = support::read_chain(&h.trace_dir)
        .pop()
        .expect("the trace has its terminal envelope");
    after_terminal.link.seq = before.chain_seq.expect("the terminal sequence") + 1;
    after_terminal.link.prev = before.chain_head;
    after_terminal.link.hash = "ac".repeat(32);
    h.daemon
        .stage_retained_post_pump_tail_for_test(&h.entry_id, after_terminal)
        .expect("the retained drain stages a tail after its uploader returns");

    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("the final recovered drain validation refuses the staged tail");
    assert!(error.to_string().contains("after terminal"), "{error}");
    assert_terminal_drain_review(&h);
}

/// The final validation can fail after a successful upload while its own registry quarantine
/// transaction cannot persist. That secondary failure must block clean status publication and
/// retained-drain eviction just as decisively as a persisted review marker does.
#[tokio::test]
async fn a_recovered_final_validation_persistence_failure_never_publishes_clean_completion() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let h = support::restart_daemon(h).await;
    h.set_upload_offline(true);
    h.daemon
        .pump_all()
        .await
        .expect("the offline cadence constructs its retained drain");
    h.set_upload_offline(false);
    h.daemon
        .resume_retained_upload_for_test(&h.entry_id)
        .expect("the persistence-failure fixture clears retained backoff");

    let before = h.entry();
    let terminal = before.chain_seq.expect("the terminal sequence");
    let mut after_terminal = support::read_chain(&h.trace_dir)
        .pop()
        .expect("the trace has its terminal envelope");
    after_terminal.link.seq = terminal + 1;
    after_terminal.link.prev = before.chain_head;
    after_terminal.link.hash = "ae".repeat(32);
    h.daemon
        .stage_retained_post_pump_tail_for_test(&h.entry_id, after_terminal)
        .expect("the tail is staged only after a real uploader returns");
    h.daemon
        .fail_next_retained_terminal_quarantine_for_test(&h.entry_id)
        .expect("the next post-pump quarantine persistence is faulted");
    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("the final validation persistence error reaches the cadence");
    assert!(
        error.to_string().contains("quarantine persistence"),
        "{error}"
    );
    let after = h.entry();
    assert_eq!(after.last_ack_seq, Some(terminal));
    assert!(after.events_uploaded > before.events_uploaded);
    assert!(
        !after.terminal_drain_validation_failed,
        "the injected persistence failure proves this guard cannot depend on the marker"
    );
    assert!(
        h.daemon
            .signed_status(&h.entry_id)
            .expect("the daemon status read succeeds")
            .is_none(),
        "a PersistFailed final validation withholds daemon status rather than serving old cache"
    );
    assert_terminal_offline_views_fail_closed(&h);
    let later = h
        .daemon
        .pump_all()
        .await
        .expect_err("the later cadence retries the withheld terminal-tail quarantine");
    assert!(later.to_string().contains("after terminal"), "{later}");
    assert_terminal_drain_review(&h);
    assert_terminal_offline_views_are_review(&h);
    h.daemon
        .ack_retained_buffer_for_test(&h.entry_id, terminal)
        .expect("the drain remains retained rather than evicted after final validation failure");
}

/// A durable-row refresh can fail after the uploader has returned too. This distinct path has
/// no quarantine marker at all, so it proves an `is_drained` error itself suppresses clean
/// publication and eviction rather than relying on a successfully refreshed registry row.
#[tokio::test]
async fn a_recovered_final_validation_refresh_failure_never_publishes_clean_completion() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let h = support::restart_daemon(h).await;
    h.set_upload_offline(true);
    h.daemon
        .pump_all()
        .await
        .expect("the offline cadence constructs its retained drain");
    h.set_upload_offline(false);
    h.daemon
        .resume_retained_upload_for_test(&h.entry_id)
        .expect("the refresh-failure fixture clears retained backoff");

    let before = h.entry();
    let terminal = before.chain_seq.expect("the terminal sequence");
    h.daemon
        .fail_next_retained_post_pump_refresh_for_test(&h.entry_id)
        .expect("the next final durable refresh is faulted after upload");
    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("the post-pump durable refresh failure reaches the cadence");
    assert!(error.to_string().contains("durable refresh"), "{error}");
    let after = h.entry();
    assert_eq!(after.last_ack_seq, Some(terminal));
    assert!(after.events_uploaded > before.events_uploaded);
    assert!(
        h.daemon
            .signed_status(&h.entry_id)
            .expect("the daemon status read succeeds")
            .is_none(),
        "a PersistFailed durable refresh withholds daemon status rather than serving old cache"
    );
    assert_terminal_offline_views_fail_closed(&h);
    h.daemon
        .ack_retained_buffer_for_test(&h.entry_id, terminal)
        .expect("the drain remains retained rather than evicted after final refresh failure");
}

/// Withholding is a transient safety state, not a second terminal outcome. Once the next
/// fully acknowledged recovered validation is clean, it must publish a fresh sealed document
/// before releasing the latch and evicting the retained owner; an old missing status is not a
/// healthy completed run.
#[tokio::test]
async fn a_clean_recovered_equality_retry_republishes_before_releasing_withheld_status() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let h = support::restart_daemon(h).await;
    h.set_upload_offline(true);
    h.daemon
        .pump_all()
        .await
        .expect("the offline cadence constructs its retained drain");
    h.set_upload_offline(false);
    h.daemon
        .resume_retained_upload_for_test(&h.entry_id)
        .expect("the fixture clears retained backoff");
    h.daemon
        .fail_next_retained_post_pump_refresh_for_test(&h.entry_id)
        .expect("the first final refresh is faulted");

    h.daemon
        .pump_all()
        .await
        .expect_err("the first clean terminal validation is withheld by its refresh fault");
    assert!(
        h.daemon
            .signed_status(&h.entry_id)
            .expect("the daemon status lookup succeeds")
            .is_none(),
        "the first fault establishes the withheld latch"
    );

    goalcup_daemon::status::fail_next_mirror_for_test(&h.config.config_dir);
    h.daemon
        .pump_all()
        .await
        .expect_err("a clean completion whose signed cache cannot publish stays withheld");
    assert!(
        h.daemon
            .signed_status(&h.entry_id)
            .expect("the daemon status lookup succeeds")
            .is_none(),
        "the latch survives until every clean completion view has published"
    );
    h.daemon
        .ack_retained_buffer_for_test(&h.entry_id, h.entry().chain_seq.expect("terminal"))
        .expect("the failed clean publication keeps the retained drain owned");

    h.daemon
        .pump_all()
        .await
        .expect("the later clean equality validation can complete after the one-shot fault");
    let status = h
        .daemon
        .signed_status(&h.entry_id)
        .expect("the daemon status lookup succeeds")
        .expect("a fresh clean terminal document releases the latch");
    assert_eq!(
        status.payload.health,
        goalcup_protocol::events::HealthState::Sealed
    );
    assert_eq!(status.payload.events_pending, 0);
    assert!(
        std::fs::read(canonical_status_line(&h))
            .expect("the clean recovered completion restores its rooted status line")
            .starts_with(b"/goal Cup "),
        "cache completion alone is insufficient: the retained drain publishes the line first"
    );
    assert!(
        !h.daemon
            .entries()
            .expect("the live entries read")
            .iter()
            .any(|entry| entry.entry_id == h.entry_id),
        "only after a fresh completion publish may the recovered owner be evicted"
    );
    assert_eq!(
        mcp_source(&h)
            .for_entry(&h.entry_id)
            .expect("MCP reads the fresh sealed completion")
            .payload
            .events_pending,
        0
    );
}

/// The equality branch on startup is still a terminal drain: a fully acknowledged registry
/// row paired with a matching buffer floor may conceal a record after the immutable terminal.
/// It must quarantine before startup can serve an all-clear, and stay quarantined on the
/// next upload cadence too.
#[tokio::test]
async fn a_fully_acked_startup_row_quarantines_a_post_terminal_buffer_tail() {
    use goalcup_daemon::buffer::Buffer;

    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let before = h.entry();
    let terminal = before.chain_seq.expect("the terminal sequence");
    let mut after_terminal = support::read_chain(&h.trace_dir)
        .pop()
        .expect("the trace has its terminal envelope");
    after_terminal.link.seq = terminal + 1;
    after_terminal.link.prev = before.chain_head;
    after_terminal.link.hash = "ad".repeat(32);
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| entry.last_ack_seq = Some(terminal))
        .expect("the fully acknowledged registry fixture is durable");

    let h = support::restart_daemon_after_stop(h, |row| {
        let buffer = Buffer::open(
            &Buffer::default_path(&row.artifact_dir),
            &row.keys.buffer_key_hex,
        )
        .expect("the stopped sealed buffer opens");
        buffer
            .ack(terminal)
            .expect("the matching durable floor is staged");
        buffer
            .append(&after_terminal)
            .expect("the post-terminal tail is durable");
    })
    .await;

    assert_terminal_drain_review(&h);
    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("a later cadence cannot clear the startup terminal-tail quarantine");
    assert!(error.to_string().contains("after terminal"), "{error}");
    assert_terminal_drain_review(&h);
}

/// Bootstrap may still abort after terminal classification (for example, its retained
/// harness reaper can fail).  That later failure must not leave the previous raw MCP cache
/// as the only offline truth: startup publishes the durable review first.  The participant
/// line is deliberately unchanged here because terminal line mutation needs the queued
/// rooted artifact capability, which must also cover this startup path.
#[tokio::test]
async fn startup_publishes_terminal_review_before_a_later_reaper_abort() {
    use goalcup_daemon::buffer::Buffer;

    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let config = h.config.clone();
    let entry_id = h.entry_id.clone();
    let entry = h.entry();
    let terminal = entry.chain_seq.expect("the terminal sequence");
    let line_before = std::fs::read(&entry.status_line_path).expect("the existing line reads");
    h.daemon.fail_next_startup_reap_for_test();
    let failed = support::restart_daemon_after_stop_expect_error(h, |row| {
        let buffer = Buffer::open(
            &Buffer::default_path(&row.artifact_dir),
            &row.keys.buffer_key_hex,
        )
        .expect("the stopped terminal buffer opens");
        buffer
            .ack(terminal + 1)
            .expect("the corrupt post-terminal floor is staged");
    })
    .await;
    assert!(
        failed.error.to_string().contains("reaper"),
        "{}",
        failed.error
    );

    write_mcp_key(&config.config_dir, &entry);
    let source = goalcup_mcp::StatusSource::open(&config.config_dir)
        .expect("the offline MCP source opens after failed bootstrap");
    assert_eq!(
        source
            .for_entry(&entry_id)
            .expect("startup wrote signed terminal review before reaper abort")
            .payload
            .health,
        goalcup_protocol::events::HealthState::SealingRecoveryNeeded
    );
    let review_line = std::fs::read(&entry.status_line_path).expect("the rooted line reads");
    assert_ne!(
        review_line, line_before,
        "terminal bootstrap replaces its rooted line before the later reaper failure"
    );
    assert!(
        std::str::from_utf8(&review_line)
            .expect("line is UTF-8")
            .starts_with("/goal Cup — Capture is closed:")
    );
}

/// A mirror failure happens after the terminal refusal has reached the registry but before
/// the cache can be replaced.  That leaves stale signed bytes on disk and in memory; the
/// immediate read must be unavailable rather than leak the old sealed all-clear, and the
/// next upload cadence must retry the persisted review publication without an ACK delta.
#[tokio::test]
async fn a_live_terminal_quarantine_never_serves_stale_status_after_mirror_failure() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let terminal = h.entry().chain_seq.expect("the terminal sequence");
    let faults_before = goalcup_daemon::status::status_fault_counts_for_test(
        &h.config.config_dir,
        &canonical_status_line(&h),
    );
    goalcup_daemon::status::fail_next_mirror_for_test(&h.config.config_dir);
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| entry.last_ack_seq = Some(terminal + 1))
        .expect("the impossible durable acknowledgement is staged");

    h.daemon.pump_all().await.expect_err(
        "the live validation refusal still reaches the caller when its publication fails",
    );
    let faults_after_first = goalcup_daemon::status::status_fault_counts_for_test(
        &h.config.config_dir,
        &canonical_status_line(&h),
    );
    assert_eq!(
        faults_after_first.mirror_attempts,
        faults_before.mirror_attempts + 1
    );
    assert_eq!(
        faults_after_first.mirror_failures,
        faults_before.mirror_failures + 1
    );
    assert_terminal_drain_review_after_publication_failure(&h);
    assert_terminal_offline_views_fail_closed(&h);

    h.daemon
        .pump_all()
        .await
        .expect_err("the durable terminal refusal remains an upload-cycle error");
    let faults_after_second = goalcup_daemon::status::status_fault_counts_for_test(
        &h.config.config_dir,
        &canonical_status_line(&h),
    );
    assert_eq!(
        faults_after_second.mirror_attempts,
        faults_after_first.mirror_attempts + 1,
        "a persistent quarantine retries its mirror even without an ACK delta"
    );
    assert_eq!(
        faults_after_second.mirror_failures,
        faults_after_first.mirror_failures
    );
    assert_terminal_drain_review(&h);
    assert_terminal_offline_views_are_review(&h);
}

/// Terminal recovery republishes the review line through its rooted artifact capability.
/// The line changes alongside the MCP/API view without trusting any registry path.
#[tokio::test]
async fn a_recovered_terminal_quarantine_republishes_the_rooted_status_line() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let h = support::restart_daemon(h).await;
    h.set_upload_offline(true);
    h.daemon
        .pump_all()
        .await
        .expect("the offline cadence constructs its retained drain");
    h.set_upload_offline(false);

    let terminal = h.entry().chain_seq.expect("the terminal sequence");
    let line_before = std::fs::read(canonical_status_line(&h)).expect("the existing line reads");
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| entry.last_ack_seq = Some(terminal + 1))
        .expect("the impossible retained acknowledgement is staged");

    h.daemon
        .pump_all()
        .await
        .expect_err("the retained validation refusal reaches the upload cadence");
    let review_line = std::fs::read(canonical_status_line(&h)).expect("the rooted line reads");
    assert_ne!(
        review_line, line_before,
        "terminal recovery replaces the rooted status line with elevated-review truth"
    );
    assert!(
        std::str::from_utf8(&review_line)
            .expect("line is UTF-8")
            .starts_with("/goal Cup — Capture is closed:")
    );
    assert_terminal_drain_review_after_publication_failure(&h);
    assert_terminal_offline_views_fail_closed(&h);

    h.daemon
        .pump_all()
        .await
        .expect_err("the persisted retained refusal is still refused on its next cadence");
    assert_terminal_drain_review(&h);
    assert_terminal_offline_views_are_review(&h);
}

/// The recovery drain owns the same kind of capability as a live runtime. Once it has been
/// rebuilt, terminal review must stay in that detached entry even if the registry spelling
/// is replaced before its final validation cadence.
#[cfg(unix)]
#[tokio::test]
async fn a_recovered_terminal_review_stays_in_its_retained_entry_after_an_entry_swap() {
    use std::os::unix::fs::symlink;

    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let h = support::restart_daemon(h).await;
    h.set_upload_offline(true);
    h.daemon
        .pump_all()
        .await
        .expect("the offline cadence constructs the retained drain");
    h.set_upload_offline(false);
    let terminal = h.entry().chain_seq.expect("the terminal sequence");
    let outside = h
        .config
        .config_dir
        .join("outside-recovered-artifact-sentinel");
    std::fs::create_dir(&outside).expect("outside artifact directory");
    std::fs::write(
        outside.join("status-line.txt"),
        b"OUTSIDE_RECOVERED_SENTINEL",
    )
    .expect("outside sentinel");
    let detached = h
        .config
        .config_dir
        .join("detached-recovered-entry-artifact");
    std::fs::rename(&h.artifact_dir, &detached).expect("detach recovered entry");
    symlink(&outside, &h.artifact_dir).expect("replace entry path with outside link");
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| entry.last_ack_seq = Some(terminal + 1))
        .expect("the impossible retained acknowledgement is staged");

    h.daemon
        .pump_all()
        .await
        .expect_err("the retained validation refusal reaches the upload cadence");

    assert!(
        std::str::from_utf8(
            &std::fs::read(detached.join("status-line.txt"))
                .expect("the retained drain writes its detached line"),
        )
        .expect("line is UTF-8")
        .starts_with("/goal Cup — Capture is closed:"),
        "the recovered terminal review replaces detached status truth"
    );
    assert_eq!(
        std::fs::read(outside.join("status-line.txt")).expect("outside sentinel remains"),
        b"OUTSIDE_RECOVERED_SENTINEL"
    );
}

/// The shared MCP latest pointer is a separate sink after the per-entry document. A failure
/// there must leave the new entry document as review, remove only this entry's stale latest
/// pointer, and retry that exact pointer on the next upload cadence.
#[tokio::test]
async fn a_live_terminal_quarantine_invalidates_and_retries_its_latest_pointer() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let terminal = h.entry().chain_seq.expect("the terminal sequence");
    let faults_before = goalcup_daemon::status::status_fault_counts_for_test(
        &h.config.config_dir,
        &canonical_status_line(&h),
    );
    goalcup_daemon::status::fail_next_latest_for_test(&h.config.config_dir);
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| entry.last_ack_seq = Some(terminal + 1))
        .expect("the impossible durable acknowledgement is staged");

    h.daemon
        .pump_all()
        .await
        .expect_err("the latest-pointer fault does not clear the terminal validation refusal");
    let faults_after_first = goalcup_daemon::status::status_fault_counts_for_test(
        &h.config.config_dir,
        &canonical_status_line(&h),
    );
    assert_eq!(
        faults_after_first.latest_attempts,
        faults_before.latest_attempts + 1
    );
    assert_eq!(
        faults_after_first.latest_failures,
        faults_before.latest_failures + 1
    );
    assert_terminal_offline_views_fail_closed(&h);

    h.daemon
        .pump_all()
        .await
        .expect_err("the durable validation refusal remains the upload-cycle error");
    let faults_after_second = goalcup_daemon::status::status_fault_counts_for_test(
        &h.config.config_dir,
        &canonical_status_line(&h),
    );
    assert_eq!(
        faults_after_second.latest_attempts,
        faults_after_first.latest_attempts + 1
    );
    assert_eq!(
        faults_after_second.latest_failures,
        faults_after_first.latest_failures
    );
    assert_terminal_offline_views_are_review(&h);
}

/// `status-latest.json` belongs to its signed payload, not to every terminal quarantine
/// that happens to publish later. A newer unrelated run must remain both readable and
/// byte-identical while this entry loses its own stale all-clear and then retries review.
#[tokio::test]
async fn a_terminal_quarantine_preserves_an_unrelated_newer_latest_pointer() {
    let h = support::started_entry_with_work().await;
    let second_runtime = support::second_entry(&h, "e2").await;
    h.daemon
        .register(second_runtime)
        .expect("the unrelated second entry opens");
    let second = h
        .daemon
        .registry()
        .get("e2")
        .expect("the second registry row reads")
        .expect("the second registry row exists");
    let mut second_payload = h
        .daemon
        .signed_status(&h.entry_id)
        .expect("the first signed status reads")
        .expect("the first signed status exists")
        .payload;
    second_payload.entry_id = second.entry_id.clone();
    second_payload.started_at = Some("9999-12-31T23:59:59Z".to_string());
    let unrelated_latest = goalcup_daemon::status::sign_payload(
        second_payload,
        "9999-12-31T23:59:59Z".to_string(),
        &second
            .keys
            .signing_key()
            .expect("the second signing key opens"),
    )
    .expect("the unrelated run signs its own status");
    goalcup_daemon::status::mirror(&unrelated_latest, &h.config.config_dir)
        .expect("the newer unrelated status owns latest");
    write_mcp_key(&h.config.config_dir, &second);
    let latest_path = h.config.config_dir.join("cache/status-latest.json");
    let unrelated_bytes = std::fs::read(&latest_path).expect("the unrelated latest exists");
    assert_eq!(
        mcp_source(&h)
            .latest()
            .expect("MCP reads the unrelated latest")
            .payload
            .entry_id,
        second.entry_id
    );

    h.daemon
        .seal_entry(&h.entry_id)
        .expect("the first entry seals");
    let terminal = h.entry().chain_seq.expect("the terminal sequence");
    goalcup_daemon::status::fail_next_mirror_for_test(&h.config.config_dir);
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| entry.last_ack_seq = Some(terminal + 1))
        .expect("the first entry's impossible acknowledgement is staged");
    h.daemon
        .pump_all()
        .await
        .expect_err("the first terminal quarantine reaches its failed per-entry publication");

    assert!(
        mcp_source(&h).for_entry(&h.entry_id).is_err(),
        "the failed first-entry replacement leaves no stale all-clear for MCP"
    );
    assert_eq!(
        std::fs::read(&latest_path).expect("the unrelated latest remains"),
        unrelated_bytes,
        "terminal invalidation never deletes or rewrites another entry's latest pointer"
    );
    assert_eq!(
        mcp_source(&h)
            .latest()
            .expect("MCP still reads the unrelated latest")
            .payload
            .entry_id,
        second.entry_id
    );

    h.daemon
        .pump_all()
        .await
        .expect_err("the persisted terminal quarantine retries on the next cadence");
    let first = mcp_source(&h)
        .for_entry(&h.entry_id)
        .expect("the retried first-entry review is readable through MCP");
    assert_eq!(
        first.payload.health,
        goalcup_protocol::events::HealthState::SealingRecoveryNeeded
    );
    assert!(first.payload.events_pending > 0);
    assert_eq!(
        std::fs::read(&latest_path).expect("the unrelated latest remains after retry"),
        unrelated_bytes,
        "a later review retry still cannot take an unrelated newer latest pointer"
    );
    assert_eq!(
        mcp_source(&h)
            .latest()
            .expect("MCP keeps reading the unrelated latest after retry")
            .payload
            .entry_id,
        second.entry_id
    );
}

/// The read/compare/delete side of terminal invalidation owns the same latest-pointer lock
/// as normal ranked publication. This pauses E1 after it has recognized an E1 pointer: E2
/// must block behind that lock, then become the exact final latest bytes after E1 deletes
/// only its own old pointer. Without the shared lock, E2 could write between E1's read and
/// delete and be erased as though it were E1.
#[tokio::test]
async fn terminal_latest_invalidation_cannot_delete_a_newer_ranked_writer() {
    let h = support::started_entry_with_work().await;
    let first = h.entry();
    let first_status = h
        .daemon
        .signed_status(&h.entry_id)
        .expect("the first signed status reads")
        .expect("the first signed status exists");
    goalcup_daemon::status::mirror(&first_status, &h.config.config_dir)
        .expect("E1 owns the initial latest pointer");

    let second_runtime = support::second_entry(&h, "e2").await;
    h.daemon
        .register(second_runtime)
        .expect("the unrelated second entry opens");
    let second = h
        .daemon
        .registry()
        .get("e2")
        .expect("the second registry lookup succeeds")
        .expect("the second row exists");
    let mut second_payload = first_status.payload.clone();
    second_payload.entry_id = second.entry_id.clone();
    second_payload.started_at = Some("9999-12-31T23:59:59Z".to_string());
    let second_status = goalcup_daemon::status::sign_payload(
        second_payload,
        "9999-12-31T23:59:59Z".to_string(),
        &second
            .keys
            .signing_key()
            .expect("the second signing key opens"),
    )
    .expect("E2 signs its newer ranked status");
    let expected_bytes =
        serde_json::to_vec_pretty(&second_status).expect("the exact E2 cache bytes encode");
    let observer = goalcup_daemon::status::pause_next_latest_invalidation_after_read_for_test(
        &h.config.config_dir,
    );
    let config_dir = h.config.config_dir.clone();
    let artifact_root = h.config.artifact_root.clone();
    let invalidator = std::thread::spawn(move || {
        goalcup_daemon::status::invalidate_terminal_views(&first, &config_dir, &artifact_root)
    });
    observer.wait_until_paused();

    let (attempt_tx, attempt_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let writer_config = h.config.config_dir.clone();
    let writer = std::thread::spawn(move || {
        attempt_tx.send(()).expect("the competing writer starts");
        done_tx
            .send(goalcup_daemon::status::mirror_latest(
                &second_status,
                &writer_config,
            ))
            .expect("the competing writer reports its result");
    });
    attempt_rx
        .recv()
        .expect("E2 reached the shared-latest write boundary");
    assert!(
        done_rx.try_recv().is_err(),
        "E2 is blocked while E1 still owns read/compare/delete under the latest lock"
    );

    observer.release();
    invalidator
        .join()
        .expect("the E1 invalidator thread joins")
        .expect("E1 invalidation succeeds");
    done_rx
        .recv()
        .expect("E2 completes only after E1 releases the latest lock")
        .expect("E2 latest publication succeeds");
    writer.join().expect("the E2 writer thread joins");

    let latest_path = h.config.config_dir.join("cache/status-latest.json");
    assert_eq!(
        std::fs::read(&latest_path).expect("the final latest pointer exists"),
        expected_bytes,
        "E1 invalidation cannot delete E2 bytes written after the lock handoff"
    );
    write_mcp_key(&h.config.config_dir, &second);
    assert_eq!(
        mcp_source(&h)
            .latest()
            .expect("the actual MCP reader opens the final pointer")
            .payload
            .entry_id,
        second.entry_id,
        "the actual offline reader sees E2, never an absent or stale E1 latest"
    );
}

/// A live terminal refusal whose durable quarantine transaction fails has no review marker
/// to protect the cache. It must therefore withhold every stale all-clear before the cadence
/// considers normal publication or live-owner eviction.
#[tokio::test]
async fn a_live_terminal_quarantine_persistence_failure_withholds_stale_status() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let terminal = h.entry().chain_seq.expect("the terminal sequence");
    h.daemon
        .fail_next_terminal_drain_quarantine_persistence_for_test();
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| entry.last_ack_seq = Some(terminal + 1))
        .expect("the impossible acknowledgement is staged");

    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("the live terminal quarantine persistence fault reaches the cadence");
    assert!(
        error.to_string().contains("quarantine persistence"),
        "{error}"
    );
    assert!(
        h.daemon
            .signed_status(&h.entry_id)
            .expect("the daemon status query succeeds")
            .is_none(),
        "a live persistence failure must withhold its pre-failure sealed all-clear"
    );
    assert!(
        mcp_source(&h).for_entry(&h.entry_id).is_err(),
        "the exact offline entry reader cannot retain stale clean status either"
    );
}

/// The live owner must make its post-uploader terminal decision before it publishes a clean
/// acknowledgement or evicts itself. This injects a record only after the real uploader has
/// returned; a pre-pump tail would not exercise the publication ordering boundary.
#[tokio::test]
async fn a_live_post_uploader_terminal_tail_quarantines_before_clean_publication() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let before = h.entry();
    let terminal = before.chain_seq.expect("the terminal sequence");
    let mut after_terminal = support::read_chain(&h.trace_dir)
        .pop()
        .expect("the trace has its terminal envelope");
    after_terminal.link.seq = terminal + 1;
    after_terminal.link.prev = before.chain_head;
    after_terminal.link.hash = "b1".repeat(32);
    h.daemon
        .stage_live_post_pump_tail_for_test(&h.entry_id, after_terminal)
        .expect("the tail is held until the real live uploader returns");

    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("the live post-uploader terminal tail is refused before clean publication");
    assert!(error.to_string().contains("after terminal"), "{error}");
    assert_terminal_drain_review(&h);
    assert!(
        h.daemon
            .entries()
            .expect("the authenticated summary reads")
            .iter()
            .any(|entry| {
                entry.entry_id == h.entry_id
                    && entry.health == goalcup_protocol::events::HealthState::SealingRecoveryNeeded
            }),
        "the live summary cannot call a post-uploader quarantine sealed"
    );
}

/// A failed terminal marker write must latch while the live owner is still borrowed. The
/// deterministic pause makes the race observable: while the owner is paused before drop,
/// a concurrent signed-status read and status cadence must not observe or republish stale
/// sealed truth.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_persist_failure_latches_before_owner_release_and_blocks_status_races() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let terminal = h.entry().chain_seq.expect("the terminal sequence");
    h.daemon
        .fail_next_terminal_drain_quarantine_persistence_for_test();
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| entry.last_ack_seq = Some(terminal + 1))
        .expect("the impossible acknowledgement is staged");
    h.daemon.pause_after_terminal_latch_for_test(&h.entry_id);

    let pumping = h.daemon.clone_handle();
    let entry_id = h.entry_id.clone();
    let task = tokio::spawn(async move { pumping.pump_all().await });
    h.daemon.wait_for_terminal_latch_pause_for_test(&h.entry_id);
    // This is deliberately the cadence, not a second `signed_status` read.  The owner is
    // still borrowed at this pause, so a refresh must finish by skipping it; waiting for
    // the producer before release proves it cannot deadlock trying to borrow the slot or
    // write an ordinary status over the newly installed latch.
    let faults_before_refresh = goalcup_daemon::status::status_fault_counts_for_test(
        &h.config.config_dir,
        &canonical_status_line(&h),
    );
    let refreshing = h.daemon.clone_handle();
    let (refreshed_tx, refreshed_rx) = tokio::sync::oneshot::channel();
    tokio::task::spawn_blocking(move || {
        let _ = refreshed_tx.send(refreshing.refresh_statuses());
    });
    let refreshed = tokio::time::timeout(std::time::Duration::from_secs(2), refreshed_rx).await;
    let faults_after_refresh = goalcup_daemon::status::status_fault_counts_for_test(
        &h.config.config_dir,
        &canonical_status_line(&h),
    );
    // Release before checking the result: a future assertion must never leave the paused
    // producer holding its runtime, which would turn a test failure into a Tokio shutdown
    // deadlock and conceal the actual cadence result.
    h.daemon.release_terminal_latch_pause_for_test();
    assert!(
        matches!(refreshed, Ok(Ok(_))),
        "the concurrent status cadence returns while terminal owner is borrowed: {refreshed:?}"
    );
    assert_eq!(
        faults_after_refresh.status_line_attempts, faults_before_refresh.status_line_attempts,
        "a concurrent refresh must not write any ordinary status line after the latch"
    );
    assert!(
        h.daemon
            .signed_status(&h.entry_id)
            .expect("the guarded status read succeeds")
            .is_none(),
        "the latch is visible before the owner returns to its slot"
    );
    assert!(
        h.daemon
            .signed_status(&entry_id)
            .expect("the second guarded status read succeeds")
            .is_none(),
        "a second reader cannot resurrect a sealed all-clear while the terminal owner is paused"
    );
    let error = task
        .await
        .expect("the pump task joins")
        .expect_err("the persistence failure reaches the pump caller");
    assert!(
        error.to_string().contains("quarantine persistence"),
        "{error}"
    );
    assert!(
        mcp_source(&h).for_entry(&h.entry_id).is_err(),
        "after the owner returns, the exact MCP view is invalidated rather than stale"
    );
}

/// Fully acknowledged recovered rows take a separate equality branch. A tail discovered
/// there whose marker persistence fails needs the same withheld state as the partial drain;
/// otherwise its old frozen zero-pending cache survives the failure.
#[tokio::test]
async fn a_fully_acked_retained_quarantine_persistence_failure_withholds_stale_status() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let h = support::restart_daemon(h).await;
    h.set_upload_offline(true);
    h.daemon
        .pump_all()
        .await
        .expect("the offline cadence creates the retained drain");
    h.set_upload_offline(false);

    let before = h.entry();
    let terminal = before.chain_seq.expect("the terminal sequence");
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| entry.last_ack_seq = Some(terminal))
        .expect("the equality-branch registry watermark is staged");
    h.daemon
        .ack_retained_buffer_for_test(&h.entry_id, terminal)
        .expect("the equality-branch buffer floor is staged");
    let mut tail = support::read_chain(&h.trace_dir)
        .pop()
        .expect("the terminal envelope exists");
    tail.link.seq = terminal + 1;
    tail.link.prev = before.chain_head;
    tail.link.hash = "af".repeat(32);
    h.daemon
        .append_retained_buffer_for_test(&h.entry_id, &tail)
        .expect("the equality-branch tail is staged");
    h.daemon
        .fail_next_terminal_drain_quarantine_persistence_for_test();

    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("the equality-branch persistence fault reaches the cadence");
    assert!(
        error.to_string().contains("quarantine persistence"),
        "{error}"
    );
    assert!(
        h.daemon
            .signed_status(&h.entry_id)
            .expect("the daemon status query succeeds")
            .is_none(),
        "a fully acknowledged retained persistence failure may not keep a stale all-clear"
    );
    assert!(
        mcp_source(&h).for_entry(&h.entry_id).is_err(),
        "the fully acknowledged row's offline target is unavailable until classification"
    );
}

/// A registry row is durable input, not authority to choose an arbitrary status-line file.
/// A terminal quarantine may invalidate its cache views but must never delete or replace an
/// outside sentinel named by a corrupted row.
#[tokio::test]
async fn a_terminal_quarantine_refuses_an_outside_status_line_path_without_touching_it() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let terminal = h.entry().chain_seq.expect("the terminal sequence");
    let canonical = canonical_status_line(&h);
    let canonical_before = std::fs::read(&canonical).expect("the pre-quarantine line exists");
    let sentinel = h.config.config_dir.join("outside-status-line.txt");
    std::fs::write(&sentinel, "do not alter\n").expect("the outside sentinel is written");
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| {
            entry.status_line_path = sentinel.clone();
            entry.last_ack_seq = Some(terminal + 1);
        })
        .expect("the corrupt durable path is staged");
    h.daemon
        .pump_all()
        .await
        .expect_err("the terminal refusal reaches its validation cadence");
    assert_eq!(
        std::fs::read_to_string(&sentinel).expect("the outside sentinel remains readable"),
        "do not alter\n",
        "terminal status publication must not follow a registry-selected arbitrary path"
    );
    let review_line = std::fs::read(&canonical).expect("the rooted canonical line reads");
    assert_ne!(
        review_line, canonical_before,
        "terminal recovery replaces only the rooted canonical line"
    );
    assert!(
        std::str::from_utf8(&review_line)
            .expect("line is UTF-8")
            .starts_with("/goal Cup — Capture is closed:")
    );
    assert_eq!(
        mcp_source(&h)
            .for_entry(&h.entry_id)
            .expect("the safe target cache is review rather than stale all-clear")
            .payload
            .health,
        goalcup_protocol::events::HealthState::SealingRecoveryNeeded
    );
    h.daemon
        .pump_all()
        .await
        .expect_err("the durable review remains a refused upload cadence while it retries");
    assert_eq!(
        std::fs::read(&canonical).expect("the rooted retry line reads"),
        review_line
    );
    assert_eq!(
        std::fs::read_to_string(&sentinel).expect("the outside sentinel still remains"),
        "do not alter\n",
        "a later retry still cannot follow the corrupted row path"
    );
}

/// The registry is only a mirror of the durable buffer ACK.  A registry acknowledgement at
/// the terminal while the buffer floor is still behind is not completion; it must not publish
/// zero pending or evict the live owner merely because the registry row has the terminal seq.
#[tokio::test]
async fn a_live_sealed_registry_ack_ahead_of_its_buffer_cannot_complete_or_evict() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let terminal = h.entry().chain_seq.expect("the terminal sequence");
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| entry.last_ack_seq = Some(terminal))
        .expect("the impossible registry completion is staged after start");

    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("a registry acknowledgement ahead of its buffer floor is refused");
    assert!(error.to_string().contains("ahead of durable buffer floor"));
    assert_terminal_drain_review(&h);
    assert!(
        h.daemon
            .entries()
            .expect("the live entries read")
            .iter()
            .any(|entry| entry.entry_id == h.entry_id),
        "a quarantined terminal entry cannot be evicted as though it had drained"
    );
    h.daemon
        .refresh_statuses()
        .expect("the later status cadence republishes the live terminal row");
    assert_terminal_drain_review(&h);
    assert!(
        h.daemon
            .entries()
            .expect("the live entries reread")
            .iter()
            .any(|entry| entry.entry_id == h.entry_id),
        "refreshing status cannot erase terminal review and evict the live owner"
    );
}

/// A live terminal owner must reject a buffer floor beyond its immutable terminal before the
/// uploader sees it. This is independent of a post-terminal record: an impossible durable
/// acknowledgement alone is enough to require capture-closed review status.
#[tokio::test]
async fn a_live_sealed_buffer_floor_beyond_terminal_is_quarantined_and_republished() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let terminal = h.entry().chain_seq.expect("the terminal sequence");
    h.daemon
        .ack_live_buffer_for_test(&h.entry_id, terminal + 1)
        .expect("the live corrupt buffer floor is staged after daemon start");

    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("a live floor beyond terminal is refused");
    assert!(
        error
            .to_string()
            .contains("buffer acknowledgement exceeds terminal")
    );
    assert_terminal_drain_review(&h);
}

/// A recovered registry row at the terminal is still false completion when the buffer floor
/// is behind. It must be quarantined, remain retained for review rather than be removed as
/// drained, and republish non-zero pending capture-closed status.
#[tokio::test]
async fn a_retained_registry_ack_ahead_of_its_buffer_cannot_complete_or_evict() {
    let h = support::started_entry_with_work().await;
    h.daemon.seal_entry(&h.entry_id).expect("it seals");
    let h = support::restart_daemon(h).await;
    h.set_upload_offline(true);
    h.daemon
        .pump_all()
        .await
        .expect("the offline cadence constructs its retained drain");
    h.set_upload_offline(false);

    let terminal = h.entry().chain_seq.expect("the terminal sequence");
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| entry.last_ack_seq = Some(terminal))
        .expect("the impossible recovered registry completion is staged after start");
    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("a recovered registry acknowledgement ahead of its buffer is refused");
    assert!(error.to_string().contains("ahead of durable buffer floor"));
    assert_terminal_drain_review(&h);
    h.daemon
        .ack_retained_buffer_for_test(&h.entry_id, terminal)
        .expect("the false-complete retained drain was not removed as drained");
}

/// An entry whose seal does not go through must not stop `due_entries` from considering
/// any later entry in the sorted map — once a second, silently, for the rest of the run.
/// A deadline is the one ending nobody is
/// present for, and an entry that missed it because a neighbour was busy has no second
/// chance.
#[tokio::test]
async fn one_entrys_failed_deadline_seal_does_not_stop_the_next_entrys() {
    let h = support::started_entry_with_work().await;
    let runtime = support::second_entry(&h, "e2").await;
    h.daemon.register(runtime).expect("the second entry opens");

    for id in [h.entry_id.as_str(), "e2"] {
        h.daemon
            .registry()
            .update(id, |e| {
                e.deadline_unix = Some(1.0);
                e.deadline_at = Some(goalcup_protocol::control::format_z(1));
            })
            .expect("the deadline moves");
    }

    // `due_entries` walks the entry map, which is a `BTreeMap`: this is the case where the
    // first id fails and a later one must still be considered, so the ordering is asserted
    // rather than assumed.
    assert!(
        h.entry_id.as_str() < "e2",
        "the harness entry {} does not sort before e2, so this case is upside down",
        h.entry_id
    );
    // The first entry in id order cannot be sealed: its runtime is out of the map, exactly
    // as it is for the length of a pump or a receipt round trip.
    let held = h.daemon.borrow_entry(&h.entry_id).expect("borrowed");
    let refused = h
        .daemon
        .seal_due_entries()
        .expect_err("the failure is still reported");
    assert!(format!("{refused}").contains("mid-upload"), "got {refused}");
    drop(held);

    assert!(
        h.daemon.is_sealed("e2").expect("e2 is asked"),
        "e2 was due and was never even considered"
    );
    assert!(
        !h.daemon.is_sealed(&h.entry_id).expect("e1 is asked"),
        "and the one that could not be sealed was not pretended to be"
    );
}

/// `run_forever` must not run tick, upload, status and the deadline check on one task while
/// the tick awaits the start receipt — a bounded HTTP call carrying a ten-second timeout. With
/// the receipt server down, `may_start()` stays true, so every beat would pay the whole timeout
/// and for the length of the arming window (120-300 s, per connector) nothing would be polled,
/// nothing uploaded, no status published and no deadline checked.
///
/// The stall is a real round trip: e2's receipt goes to a server that accepts the socket
/// and answers nothing for three seconds, so `ReceiptClient` is on the wire while the loop
/// is asked to do its other work. A borrow held by the test would be the same shape and
/// would exercise none of the client.
#[tokio::test]
async fn a_stalled_receipt_round_trip_does_not_stop_the_other_cadences() {
    let h = support::started_entry_with_work().await;
    // Five seconds: long enough that the deadline cadence — which ticks once a second — has
    // room to run to completion twice over while the receipt is still on the wire.
    let stalling = support::stalling_receipt_server(Duration::from_secs(5));
    let mut harness = support::entry_owed_a_receipt(&h, "e2", &stalling).await;

    // The other run's deadline is in the past, so the deadline cadence has real work to do
    // while e2's receipt is outstanding.
    h.daemon
        .registry()
        .update(&h.entry_id, |e| {
            e.deadline_unix = Some(1.0);
            e.deadline_at = Some(goalcup_protocol::control::format_z(1));
        })
        .expect("the deadline moves");

    let daemon = h.daemon.clone_handle();
    let looping = tokio::spawn(async move { daemon.run_forever().await });

    // The premise first: the receipt is really on the wire. Everything after this is a
    // claim about what the other cadences did while it was.
    let onwire = Instant::now() + Duration::from_secs(5);
    while Instant::now() < onwire && !h.daemon.is_binding("e2") {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let started_stalled = h.daemon.is_binding("e2");
    // Sealed while the receipt is outstanding, which is the whole claim.
    let mut sealed_while_stalled = false;
    let mut stall_ended_first = false;
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if h.daemon.is_sealed(&h.entry_id).unwrap_or(false) {
            sealed_while_stalled = h.daemon.is_binding("e2");
            break;
        }
        if !h.daemon.is_binding("e2") {
            stall_ended_first = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    looping.abort();
    let _ = harness.kill();
    let _ = harness.wait();

    assert!(
        started_stalled,
        "e2 never had a receipt in flight, so this proves nothing"
    );
    assert!(
        !stall_ended_first,
        "the five-second stall ran out before the once-a-second deadline cadence fired: \
         this machine is too loaded for the case to be measured"
    );
    assert!(
        sealed_while_stalled,
        "one entry's receipt round trip stopped the deadline check for every other run"
    );
}

/// The `binding` set is cleared by a drop guard, not on the last line of the spawned task,
/// which is a line a panic skips — and `poll_pass` steps over an entry whose id is in that
/// set, so one unwind would stop that run being polled for the life of the daemon while
/// nothing at all was binding it. A drop guard runs on every exit there is.
#[tokio::test]
async fn a_binding_that_panics_gives_its_entry_back_and_polling_resumes() {
    let h = support::started_entry_with_work().await;
    let stalling = support::stalling_receipt_server(Duration::from_millis(1));
    let mut harness = support::entry_owed_a_receipt(&h, "e2", &stalling).await;

    // Armed before the pass that spawns the binding, so the task that panics is the one
    // the guard has to survive — and polled until the binding really is spawned, because
    // an entry whose session has not bound yet is owed nothing and a pass over one would
    // prove nothing at all.
    h.daemon.panic_next_binding();
    let spawned = Instant::now() + Duration::from_secs(5);
    while Instant::now() < spawned {
        h.daemon.poll_all().expect("the poll pass runs");
        if h.daemon.is_binding("e2") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        h.daemon.is_binding("e2"),
        "e2 was never owed a receipt, so no binding panicked"
    );

    let clear = Instant::now() + Duration::from_secs(5);
    while Instant::now() < clear && h.daemon.is_binding("e2") {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        !h.daemon.is_binding("e2"),
        "the panicking binding pinned e2, and nothing will poll it again"
    );
    assert_eq!(
        h.daemon.free_permits(),
        64,
        "the permit went with the unwind too, or `stop` would never return"
    );

    // And the entry is polled again: the next pass asks for the binding the panic lost.
    // `poll_all` is synchronous and the id goes into the set before the spawn, so this is
    // read without an await in between and is not a race.
    h.daemon.poll_all().expect("the second poll pass runs");
    assert!(
        h.daemon.is_binding("e2"),
        "e2 was stepped over: the poll pass skips an entry the set still holds"
    );
    let _ = harness.kill();
    let _ = harness.wait();
}

/// The seal runs on `tokio::task::spawn_blocking`, which is not abortable and is not
/// on `listeners`. `stop` must not return while a seal is mid-`terminate` — still to write
/// `seal.json` and the registry row, still killing a tree — and the worker holds an
/// `Arc<Daemon>`, so the redb registry and buffer locks would outlive the stop that is supposed
/// to release them.
///
/// The harness ignores SIGTERM, so the seal spends its whole grace inside `terminate`: a
/// `stop` that returned before the seal finished would return in milliseconds.
#[tokio::test]
async fn stop_returns_after_an_in_flight_seal_and_not_before() {
    let h = support::started_entry_with_work().await;

    // A harness that will not go on SIGTERM, so the seal's grace is really spent.
    let mut stubborn = std::process::Command::new("/bin/sh")
        .args(["-c", "trap '' TERM; sleep 30 & wait"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a stubborn harness starts");
    h.daemon
        .clear_process(&h.entry_id, 1)
        .expect("the slot opens");
    h.daemon
        .register_process(
            &h.entry_id,
            support::process_registration(2, stubborn_handle(stubborn.id())),
        )
        .expect("the handle is registered");

    let sealing = h.daemon.clone_handle();
    let entry_id = h.entry_id.clone();
    let seal = tokio::spawn(async move { sealing.seal_entry_blocking(&entry_id).await });
    // Waited for, not slept over: what `stop` has to be looking at is a permit that is
    // already taken, and a fixed 200 ms is either longer than it needs to be or — on a
    // loaded machine — shorter than it has to be, which is a test that passes for the
    // wrong reason.
    let taken = Instant::now() + Duration::from_secs(5);
    while Instant::now() < taken && h.daemon.free_permits() == 64 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        h.daemon.free_permits(),
        63,
        "the seal never took its permit, so `stop` has nothing to wait for"
    );

    let stopping = h.daemon.clone_handle();
    let started = Instant::now();
    stopping.stop().await;
    let waited = started.elapsed();

    assert!(
        waited >= Duration::from_secs(1),
        "the seal's grace was not spent inside `stop`, so nothing was awaited: {waited:?}"
    );
    // The work itself was already over when `stop` returned: the join below is bookkeeping
    // and not a wait. A `stop` that returned over a live seal would spend the rest of the
    // grace here instead.
    let joined = tokio::time::timeout(Duration::from_millis(250), seal)
        .await
        .expect("`stop` returned over a seal that is still writing the chain");
    joined
        .expect("the seal task ran")
        .expect("the blocking worker finished")
        .expect("and the seal itself went through");
    assert!(
        h.daemon.is_sealed(&h.entry_id).expect("the row"),
        "the seal that `stop` waited for finished its work"
    );
    let _ = stubborn.kill();
    let _ = stubborn.wait();
}

/// The binding's permit is taken synchronously, before the spawn. Acquired inside the
/// spawned task, it would leave the semaphore untouched between `spawn_bind_start` returning and
/// the runtime first polling that task — and a `stop` arriving in that window would take all 64
/// permits, read an idle daemon and return over a start that had not run.
#[tokio::test]
async fn stop_waits_for_a_binding_that_has_not_run_yet() {
    let h = support::started_entry_with_work().await;
    let stalling = support::stalling_receipt_server(Duration::from_secs(3));
    let mut harness = support::entry_owed_a_receipt(&h, "e2", &stalling).await;

    h.daemon.poll_all().expect("the poll pass runs");
    // No await between the spawn and this, so the task has not been polled at all: the
    // permit below is held by the code that spawned it and by nothing else.
    assert!(h.daemon.is_binding("e2"), "the binding was spawned");
    assert_eq!(
        h.daemon.free_permits(),
        63,
        "the permit is taken before the spawn, or `stop` has a window to slip through"
    );

    let started = Instant::now();
    h.daemon.clone_handle().stop().await;
    let waited = started.elapsed();

    let _ = harness.kill();
    let _ = harness.wait();
    assert!(
        waited >= Duration::from_secs(1),
        "`stop` returned over a start binding that was still on the wire: {waited:?}"
    );
    assert_eq!(
        h.daemon.free_permits(),
        64,
        "and it returned with every permit back"
    );
}

/// The stubborn harness's handle: `program` and `argv` name the shell the test spawned, so
/// the kill guard's identity half passes, and the grace is long enough to be measured.
fn stubborn_handle(pid: u32) -> ProcessHandle {
    ProcessHandle {
        pid,
        pgid: None,
        native_pid: None,
        native_binary: None,
        native_process_start: None,
        process_start: goalcup_protocol::process::process_start_identity(pid),
        strategy: KillStrategy::NativePid,
        grace_seconds: 2,
        launched_at: goalcup_protocol::control::format_z(goalcup_daemon::now_unix() as u64),
        program: "sh".to_string(),
        argv: vec!["/bin/sh".to_string(), "-c".to_string()],
        env_keys: Vec::new(),
        home_seed: None,
    }
}
