//! Claude's derived sources have no authority until a corroborated session binds them.
#![cfg(feature = "v1-compat")]

mod support;

use goalcup_protocol::envelope::EventBody;

const CLAUDE: &str = "claude-code-cli";
const CODEX: &str = "codex-cli";
const BOUND_SESSION: &str = "claude-bound-session";
const SIBLING_SESSION: &str = "claude-sibling-session";

fn configure_distinct_subagent(h: &mut support::EntryHarness, parent_store: &str, template: &str) {
    h.manifest
        .store
        .iter_mut()
        .find(|store| store.name == parent_store)
        .unwrap_or_else(|| panic!("the shipped manifest has parent store `{parent_store}`"))
        .subagent_path_template = Some(template.to_string());
}

async fn rebuild_with_manifest(h: &support::EntryHarness) {
    let mut runtime = support::reopen_runtime(h).await;
    runtime.open_trace().expect("the rebuilt trace opens");
    h.give_back(runtime);
}

fn direct_child_path(h: &support::EntryHarness) -> std::path::PathBuf {
    h.home
        .join("derived-direct")
        .join(support::SESSION)
        .join("history-child.jsonl")
}

fn file_first_child_path(h: &support::EntryHarness, name: &str) -> std::path::PathBuf {
    h.home
        .join("derived-file-first")
        .join(support::SESSION)
        .join(name)
}

fn direct_child_record(session: &str, marker: &str) -> String {
    serde_json::json!({
        "session_id": session,
        "ts": 1_756_944_001,
        "text": marker,
        "marker": marker,
    })
    .to_string()
}

fn file_first_meta(cwd: &str, marker: &str, ordinal: i64) -> String {
    serde_json::json!({
        "type": "session_meta",
        "payload": { "cwd": cwd, "ordinal": ordinal },
        "marker": marker,
    })
    .to_string()
}

fn file_first_body(marker: &str, ordinal: i64) -> String {
    serde_json::json!({
        "type": "response_item",
        "payload": { "ordinal": ordinal },
        "marker": marker,
    })
    .to_string()
}

fn configure_file_first_child(h: &mut support::EntryHarness) {
    configure_distinct_subagent(
        h,
        "rollout",
        "{config_dir}/derived-file-first/{session_id}/child-*.jsonl",
    );
}

async fn start_file_first_child(h: &mut support::EntryHarness) -> std::path::PathBuf {
    configure_file_first_child(h);
    rebuild_with_manifest(h).await;

    let owned = file_first_child_path(h, "child-owned.jsonl");
    let foreign = file_first_child_path(h, "child-foreign.jsonl");
    let workspace = h.workspace.display().to_string();
    support::append_line(&owned, &file_first_meta(&workspace, "child-owned-meta", 0));
    support::append_line(&owned, &file_first_body("child-owned-before-restart", 1));
    support::append_line(
        &foreign,
        &file_first_meta("/foreign/child", "child-foreign-meta", 0),
    );
    support::append_line(
        &foreign,
        &file_first_body("child-foreign-before-restart", 1),
    );

    support::start_entry(h).await;
    let first = h.poll();
    assert_eq!(
        (first.captured, first.discarded),
        (2, 2),
        "the derived FileFirst child has its own owned and foreign file decisions"
    );
    assert_eq!(
        markers(h, "rollout_subagent"),
        vec!["child-owned-meta", "child-owned-before-restart"],
        "only the child file whose own first record names the workspace is admitted"
    );
    owned
}

fn claude_record(session: &str, cwd: &str, sidechain: bool, marker: &str) -> String {
    serde_json::json!({
        "type": "user",
        "sessionId": session,
        "cwd": cwd,
        "isSidechain": sidechain,
        "marker": marker,
    })
    .to_string()
}

fn claude_paths(
    h: &support::EntryHarness,
    session: &str,
) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let project = h.home.join("projects").join(h.vars.slug());
    (
        project.join(format!("{session}.jsonl")),
        project
            .join(session)
            .join("subagents")
            .join("agent-shared.jsonl"),
        project
            .join(session)
            .join("tool-results")
            .join("result-shared.json"),
    )
}

fn plant_claude_session(
    h: &support::EntryHarness,
    session: &str,
    marker: &str,
    include_primary: bool,
) {
    let workspace = h.workspace.display().to_string();
    let (main, subagent, spill) = claude_paths(h, session);
    if include_primary {
        support::append_line(
            &main,
            &claude_record(session, &workspace, false, &format!("{marker}-main")),
        );
    }
    support::append_line(
        &subagent,
        &claude_record(session, &workspace, true, &format!("{marker}-subagent")),
    );
    std::fs::create_dir_all(spill.parent().expect("the spill has a parent"))
        .expect("the spill directory exists");
    std::fs::write(
        spill,
        serde_json::json!({ "marker": format!("{marker}-spill") }).to_string(),
    )
    .expect("the spill fixture is written");
}

fn markers(h: &support::EntryHarness, store: &str) -> Vec<String> {
    support::read_chain(&h.trace_dir)
        .into_iter()
        .filter_map(|envelope| match envelope.body {
            EventBody::Leg1Record {
                store: record_store,
                payload: serde_json::Value::String(raw),
            } if record_store == store => serde_json::from_str::<serde_json::Value>(&raw)
                .ok()
                .and_then(|value| value.get("marker")?.as_str().map(ToOwned::to_owned)),
            _ => None,
        })
        .collect()
}

fn reread_count(h: &support::EntryHarness, store: &str) -> usize {
    support::read_chain(&h.trace_dir)
        .iter()
        .filter(|envelope| {
            matches!(
                &envelope.body,
                EventBody::TailNotice { kind, store: record_store, .. }
                    if *kind == goalcup_protocol::events::TailNoticeKind::Reread
                        && record_store == store
            )
        })
        .count()
}

#[tokio::test]
async fn a_fresh_unbound_claude_runtime_polls_primary_but_not_distinct_derived_sources() {
    let h = support::one_armed_entry_on(CLAUDE).await;
    plant_claude_session(&h, BOUND_SESSION, "bound", true);

    let report = h.poll();
    assert_eq!(
        support::leg1_count(&h.trace_dir, "main"),
        1,
        "the actual EntryRuntime reached the declared Claude primary source"
    );
    assert_eq!(
        report.discarded, 0,
        "the primary fixture is not a partition refusal"
    );
    assert_eq!(
        support::leg1_count(&h.trace_dir, "main_subagent"),
        0,
        "a fresh entry has no corroborated session authority for subagent paths"
    );
    assert_eq!(
        support::leg1_count(&h.trace_dir, "main_spill"),
        0,
        "a fresh entry has no corroborated session authority for spill paths"
    );
}

#[tokio::test]
async fn binding_claude_reads_retained_bound_children_once_and_refuses_same_basename_sibling() {
    let h = support::one_armed_entry_on(CLAUDE).await;
    plant_claude_session(&h, BOUND_SESSION, "bound", true);

    // The primary record makes the real Claude `sessionId` visible to the gate. The
    // declared hook post supplies the other half of the normal start transition.
    h.poll();
    assert_eq!(
        support::leg1_count(&h.trace_dir, "main"),
        1,
        "the bound primary transcript reached the current unpartitioned primary reader"
    );

    let mut runtime = h.take();
    runtime
        .bind_start(BOUND_SESSION)
        .await
        .expect("the fixture reaches the real receipt-backed bind transition");
    h.give_back(runtime);
    assert_eq!(
        h.with(|runtime| runtime.bound_session()),
        Some(BOUND_SESSION.to_string()),
        "the live gate bound the exact Claude session whose children may activate"
    );
    assert_eq!(
        h.entry().session_id.as_deref(),
        Some(BOUND_SESSION),
        "the registry start receipt persisted that exact bound session"
    );

    // The sibling needs no primary transcript to be a source-directory sibling. Adding
    // its equal-basename children only after the exact bind keeps the live gate's real
    // Claude `sessionId` unambiguous while still exposing a wildcard reader leak.
    plant_claude_session(&h, SIBLING_SESSION, "sibling", false);
    h.poll();

    assert_eq!(
        markers(&h, "main_subagent"),
        vec!["bound-subagent"],
        "the retained bound child, rather than the equal-basename sibling, is captured"
    );
    assert_eq!(
        markers(&h, "main_spill"),
        vec!["bound-spill"],
        "the retained bound spill, rather than the equal-basename sibling, is captured"
    );
    assert_eq!(
        markers(&h, "main_subagent")
            .iter()
            .filter(|marker| marker.starts_with("sibling-"))
            .count(),
        0,
        "the sibling's equal agent-shared.jsonl basename has no derived authority"
    );
    assert_eq!(
        markers(&h, "main_spill")
            .iter()
            .filter(|marker| marker.starts_with("sibling-"))
            .count(),
        0,
        "the sibling's equal result-shared.json basename has no derived authority"
    );

    // The normal duplicate corroboration path calls bind again after the receipt-backed
    // start. It must leave the live child cursors intact rather than rebuilding them at
    // zero and replaying the retained pre-bind bytes.
    let mut runtime = h.take();
    runtime
        .bind_start(BOUND_SESSION)
        .await
        .expect("a repeated corroborating bind is a no-op");
    h.give_back(runtime);
    h.poll();
    assert_eq!(
        markers(&h, "main_subagent"),
        vec!["bound-subagent"],
        "a second post-bind poll does not emit the retained child twice"
    );
    assert_eq!(
        markers(&h, "main_spill"),
        vec!["bound-spill"],
        "a second post-bind poll does not emit the retained spill twice"
    );
}

#[tokio::test]
async fn a_started_claude_rebuilds_exact_bound_children_before_restoring_their_cursors() {
    let h = support::one_armed_entry_on(CLAUDE).await;
    plant_claude_session(&h, BOUND_SESSION, "bound", true);
    h.poll();
    let mut runtime = h.take();
    runtime
        .bind_start(BOUND_SESSION)
        .await
        .expect("the fixture starts");
    h.give_back(runtime);
    h.poll();
    assert_eq!(markers(&h, "main_subagent"), vec!["bound-subagent"]);

    h.close();
    let mut restarted = support::reopen_runtime(&h).await;
    restarted.open_trace().expect("the started trace resumes");
    h.give_back(restarted);
    h.poll();
    assert_eq!(
        markers(&h, "main_subagent"),
        vec!["bound-subagent"],
        "the restored child cursor does not replay bytes already chained"
    );

    let (_, subagent, _) = claude_paths(&h, BOUND_SESSION);
    support::append_line(
        &subagent,
        &claude_record(
            BOUND_SESSION,
            &h.workspace.display().to_string(),
            true,
            "after-restart",
        ),
    );
    h.poll();
    assert_eq!(
        markers(&h, "main_subagent"),
        vec!["bound-subagent", "after-restart"],
        "the restarted runtime rebuilt the concrete child reader and restored its own cursor"
    );
}

#[tokio::test]
async fn an_unstarted_claude_rebuild_never_turns_a_launch_session_into_child_authority() {
    let mut h = support::one_armed_entry_on(CLAUDE).await;
    plant_claude_session(&h, BOUND_SESSION, "legacy", true);

    // Rendering a launch command can have a generated session value, but an unstarted
    // registry row has not corroborated it. A rebuild must discard this caller value rather
    // than use it to revive an old child cursor or read this fixture's children.
    h.vars.session_id = Some(BOUND_SESSION.to_string());
    h.close();
    let mut restarted = support::reopen_runtime(&h).await;
    restarted.open_trace().expect("the unstarted trace resumes");
    h.give_back(restarted);
    h.poll();

    assert_eq!(
        support::leg1_count(&h.trace_dir, "main_subagent"),
        0,
        "an unbound rebuild may retain the primary wildcard reader but not derived authority"
    );
    assert_eq!(
        support::leg1_count(&h.trace_dir, "main_spill"),
        0,
        "an unbound rebuild does not restore a spill reader from a launch-time session"
    );
}

/// A distinct child inherits the parent's `Direct` contract, but not its reader or its
/// first-sight state. Bytes written before corroboration stay unread; after bind, matching
/// and foreign rows are decided by the child-local tracked session.
#[tokio::test]
async fn a_derived_direct_child_reads_retained_matching_rows_and_discards_foreign_rows() {
    let mut h = support::one_armed_entry_on(CODEX).await;
    configure_distinct_subagent(
        &mut h,
        "history",
        "{config_dir}/derived-direct/{session_id}/history-child.jsonl",
    );
    rebuild_with_manifest(&h).await;

    let child = direct_child_path(&h);
    support::append_line(
        &child,
        &direct_child_record(support::SESSION, "direct-prebind-matching"),
    );
    support::append_line(
        &child,
        &direct_child_record("foreign-thread", "direct-prebind-foreign"),
    );
    let prebind = h.poll();
    assert_eq!(
        (prebind.captured, prebind.discarded),
        (0, 0),
        "an inactive child neither admits nor discards pre-bind rows"
    );
    assert_eq!(support::leg1_count(&h.trace_dir, "history_subagent"), 0);

    support::start_entry(&h).await;
    let retained = h.poll();
    assert_eq!(
        (retained.captured, retained.discarded),
        (1, 1),
        "the bound child replays retained rows through its own Direct partitioner"
    );
    assert_eq!(
        markers(&h, "history_subagent"),
        vec!["direct-prebind-matching"],
        "the matching retained child row is chained and the foreign one is not"
    );

    support::append_line(
        &child,
        &direct_child_record(support::SESSION, "direct-postbind-matching"),
    );
    support::append_line(
        &child,
        &direct_child_record("foreign-thread", "direct-postbind-foreign"),
    );
    let postbind = h.poll();
    assert_eq!(
        (postbind.captured, postbind.discarded),
        (1, 1),
        "post-bind matching and foreign child rows retain the same local decision"
    );
    assert_eq!(
        markers(&h, "history_subagent"),
        vec!["direct-prebind-matching", "direct-postbind-matching"],
        "foreign child bytes never enter the real chain"
    );
}

#[tokio::test]
async fn a_derived_file_first_child_reconstructs_its_own_decision_before_cursor_restore() {
    let mut h = support::one_armed_entry_on(CODEX).await;
    let owned = start_file_first_child(&mut h).await;

    h.close();
    let mut restarted = support::reopen_runtime(&h).await;
    restarted.open_trace().expect("the started trace resumes");
    h.give_back(restarted);
    let replay = h.poll();
    assert_eq!(
        (replay.captured, replay.discarded),
        (0, 0),
        "the child cursor restores only after its FileFirst decision is reconstructed"
    );
    assert_eq!(
        markers(&h, "rollout_subagent"),
        vec!["child-owned-meta", "child-owned-before-restart"],
        "the child does not replay its already chained owned records"
    );

    support::append_line(&owned, &file_first_body("child-owned-after-restart", 2));
    let after = h.poll();
    assert_eq!(
        (after.captured, after.discarded),
        (1, 0),
        "the reconstructed child decision admits its next owned record"
    );
    assert_eq!(
        markers(&h, "rollout_subagent"),
        vec![
            "child-owned-meta",
            "child-owned-before-restart",
            "child-owned-after-restart",
        ]
    );
}

/// Only the child source is unsafe here. Its failed no-follow reconstruction must not
/// quarantine the primary rollout, and its ordinary string tailer must not get a second
/// chance to follow the substituted directory in this runtime.
#[cfg(unix)]
#[tokio::test]
async fn an_unsafe_derived_file_first_child_is_quarantined_without_blocking_its_primary() {
    use std::os::unix::fs::{MetadataExt, symlink};

    const OUTSIDE_MARKER: &str = "derived-child-outside-source-7b89a4";
    let mut h = support::one_armed_entry_on(CODEX).await;
    let owned = start_file_first_child(&mut h).await;
    let rollout_rereads_before = reread_count(&h, "rollout");
    let child_rereads_before = reread_count(&h, "rollout_subagent");
    h.close();

    let child_root = h.home.join("derived-file-first");
    let moved_root = h.home.join("derived-file-first-original");
    std::fs::rename(&child_root, &moved_root).expect("the child root moves aside");
    let outside_root = h.artifact_dir.join("outside-derived-child");
    let outside_child = outside_root
        .join(support::SESSION)
        .join("child-owned.jsonl");
    std::fs::create_dir_all(outside_child.parent().expect("the outside parent exists"))
        .expect("the outside child directory exists");
    std::fs::write(
        &outside_child,
        format!(
            "{}\n{}\n",
            file_first_meta(&h.workspace.display().to_string(), OUTSIDE_MARKER, 0),
            file_first_body(OUTSIDE_MARKER, 1)
        ),
    )
    .expect("the outside child is written");
    let outside_facts = std::fs::metadata(&outside_child).expect("the outside child stats");
    support::set_file_cursor_facts(
        &h,
        "rollout_subagent",
        &owned,
        outside_facts.ino(),
        outside_facts.len(),
    );
    symlink(&outside_root, &child_root).expect("the child root becomes a symlink");

    let mut restarted = support::reopen_runtime(&h).await;
    restarted.open_trace().expect("the restart stays available");
    assert!(
        reread_count(&h, "rollout_subagent") == child_rereads_before + 1,
        "the unsafe child cursor is rejected and witnessed"
    );
    assert_eq!(
        reread_count(&h, "rollout"),
        rollout_rereads_before,
        "the safe primary cursor adds no new fallback witness"
    );
    let report = restarted
        .poll_once_at(h.now())
        .expect("the quarantined runtime still polls its primary");
    assert_eq!(
        (report.captured, report.discarded),
        (0, 0),
        "the substituted child produces neither admitted nor discarded outside bytes"
    );
    assert!(
        report.notices.iter().any(|notice| matches!(
            notice,
            goalcup_daemon::tailer::TailNotice::StoreUnavailable { store, .. }
                if store == "rollout_subagent"
        )),
        "the child-only quarantine is visible as its own unavailable source"
    );
    assert!(
        !std::fs::read_to_string(h.trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
            .expect("the chain reads")
            .contains(OUTSIDE_MARKER),
        "the outside child was neither reconstructed nor polled"
    );
}

/// A missing child is recoverable, unlike a source-identity safety refusal. It gets the
/// ordinary Reread witness and then accepts a replacement through the declared path.
#[tokio::test]
async fn a_missing_derived_file_first_child_uses_the_witnessed_reread_fallback() {
    let mut h = support::one_armed_entry_on(CODEX).await;
    let owned = start_file_first_child(&mut h).await;
    let rollout_rereads_before = reread_count(&h, "rollout");
    let child_rereads_before = reread_count(&h, "rollout_subagent");
    h.close();
    std::fs::remove_file(&owned).expect("the derived child is missing at reconstruction");

    let mut restarted = support::reopen_runtime(&h).await;
    restarted
        .open_trace()
        .expect("the restart accepts ordinary fallback");
    assert_eq!(
        reread_count(&h, "rollout_subagent"),
        child_rereads_before + 1,
        "the missing child gets an additional cursor-fallback witness"
    );
    assert_eq!(reread_count(&h, "rollout"), rollout_rereads_before);

    support::append_line(
        &owned,
        &file_first_meta(&h.workspace.display().to_string(), "missing-child-meta", 0),
    );
    support::append_line(&owned, &file_first_body("missing-child-body", 1));
    let report = restarted
        .poll_once_at(h.now())
        .expect("the recovered child is read through its declared source");
    assert_eq!(
        (report.captured, report.discarded),
        (2, 2),
        "the replacement owned child is captured and the retained foreign child is discarded"
    );
    assert!(
        !report.notices.iter().any(|notice| matches!(
            notice,
            goalcup_daemon::tailer::TailNotice::StoreUnavailable { store, .. }
                if store == "rollout_subagent"
        )),
        "a missing cursor source is a fallback, not a persistent quarantine"
    );
}

/// A syntactically valid but type-incompatible saved child cursor cannot be restored. The
/// primary remains restored; the child gets the same ordinary witnessed reread fallback.
#[tokio::test]
async fn a_corrupt_derived_file_first_cursor_uses_the_witnessed_reread_fallback() {
    let mut h = support::one_armed_entry_on(CODEX).await;
    let _owned = start_file_first_child(&mut h).await;
    let rollout_rereads_before = reread_count(&h, "rollout");
    let child_rereads_before = reread_count(&h, "rollout_subagent");
    h.close();
    let cursor_path = h.artifact_dir.join("cursors.json");
    let mut cursors: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&cursor_path).expect("the child cursor document reads"),
    )
    .expect("the child cursor document parses");
    cursors["rollout_subagent"]["cursor"] = serde_json::json!({
        "inode_offset": { "files": {}, "records": 0 }
    });
    std::fs::write(
        &cursor_path,
        serde_json::to_vec(&cursors).expect("the altered cursor serialises"),
    )
    .expect("the altered child cursor is written");

    let mut restarted = support::reopen_runtime(&h).await;
    restarted
        .open_trace()
        .expect("the restart accepts cursor fallback");
    assert_eq!(
        reread_count(&h, "rollout_subagent"),
        child_rereads_before + 1
    );
    assert_eq!(reread_count(&h, "rollout"), rollout_rereads_before);
    let report = restarted
        .poll_once_at(h.now())
        .expect("the corrupt child cursor rereads safely");
    assert_eq!(
        (report.captured, report.discarded),
        (2, 2),
        "the child rereads both its owned and foreign files through FileFirst partitioning"
    );
}

/// Reconstruction has one bounded poll window. A large claimed child prefix must abandon
/// reconstruction, emit the normal witness, and then read a replacement normally.
#[tokio::test]
async fn a_bounded_derived_file_first_reconstruction_uses_the_witnessed_reread_fallback() {
    let mut h = support::one_armed_entry_on(CODEX).await;
    let owned = start_file_first_child(&mut h).await;
    let rollout_rereads_before = reread_count(&h, "rollout");
    let child_rereads_before = reread_count(&h, "rollout_subagent");
    h.close();
    let file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&owned)
        .expect("the child opens in place");
    file.set_len(goalcup_daemon::MAX_POLL_BYTES * 2)
        .expect("the claimed child prefix exceeds the reconstruction budget");
    drop(file);
    support::roll_back_file_cursor(
        &h,
        "rollout_subagent",
        &owned,
        goalcup_daemon::MAX_POLL_BYTES * 2,
        0,
    );

    let mut restarted = support::reopen_runtime(&h).await;
    restarted
        .open_trace()
        .expect("the bounded restart opens promptly");
    assert_eq!(
        reread_count(&h, "rollout_subagent"),
        child_rereads_before + 1
    );
    assert_eq!(reread_count(&h, "rollout"), rollout_rereads_before);

    std::fs::write(
        &owned,
        format!(
            "{}\n{}\n",
            file_first_meta(&h.workspace.display().to_string(), "bounded-child-meta", 0),
            file_first_body("bounded-child-body", 1)
        ),
    )
    .expect("the replacement child is written");
    let report = restarted
        .poll_once_at(h.now())
        .expect("the child rereads after bounded reconstruction");
    assert_eq!(
        (report.captured, report.discarded),
        (2, 2),
        "the owned replacement and retained foreign child prove the fallback is non-vacuous"
    );
}

/// A stale child cursor is not a capability. An unstarted row can carry a launch rendering
/// value and a legacy `rollout_subagent` cursor document, but neither may activate the child.
#[tokio::test]
async fn an_unbound_rebuild_ignores_a_persisted_legacy_derived_child_cursor() {
    let mut h = support::one_armed_entry_on(CODEX).await;
    configure_file_first_child(&mut h);
    rebuild_with_manifest(&h).await;

    support::append_line(&h.store_path, &support::session_meta_line(&h));
    assert_eq!(
        h.poll().captured,
        1,
        "the primary cursor is persisted normally"
    );
    let child = file_first_child_path(&h, "child-legacy.jsonl");
    support::append_line(
        &child,
        &file_first_meta(&h.workspace.display().to_string(), "legacy-child-meta", 0),
    );
    support::append_line(&child, &file_first_body("legacy-child-body", 1));

    let cursor_path = h.artifact_dir.join("cursors.json");
    let mut cursors: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&cursor_path).expect("the primary cursor document reads"),
    )
    .expect("the primary cursor document parses");
    let child_key = child.display().to_string();
    cursors["rollout_subagent"] = serde_json::json!({
        "cursor": {
            "ordinal": {
                "last": { child_key.clone(): 1 },
                "files": { child_key: { "inode": 0, "offset": std::fs::metadata(&child).expect("the child stats").len() } },
                "records": 2
            }
        },
        "last_leg1_seq": null
    });
    std::fs::write(
        &cursor_path,
        serde_json::to_vec(&cursors).expect("the legacy child cursor serialises"),
    )
    .expect("the legacy child cursor is persisted");
    assert!(
        serde_json::from_slice::<serde_json::Value>(
            &std::fs::read(&cursor_path).expect("the legacy document rereads"),
        )
        .expect("the legacy document reparses")
        .get("rollout_subagent")
        .is_some(),
        "the regression carries an actual persisted child cursor, not only child bytes"
    );

    // A reconstruction must not take this non-authoritative launch rendering
    // as source authority. The registry row is deliberately still unstarted.
    h.vars.session_id = Some(support::SESSION.to_string());
    h.close();
    let mut restarted = support::reopen_runtime(&h).await;
    restarted.open_trace().expect("the unbound trace resumes");
    h.give_back(restarted);
    let report = h.poll();
    assert_eq!(
        (report.captured, report.discarded),
        (0, 0),
        "the unbound restart ignores the persisted child cursor instead of restoring authority"
    );
    assert_eq!(support::leg1_count(&h.trace_dir, "rollout_subagent"), 0);
    assert!(
        !std::fs::read_to_string(h.trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
            .expect("the chain reads")
            .contains("legacy-child-body"),
        "the ignored legacy cursor cannot lead to an unauthorized child read"
    );
}

/// Two declarations that differ in the manifest but render to the same exact child source
/// must be refused by the real bind transition before it can persist `Started`.
#[tokio::test]
async fn a_runtime_refuses_distinct_derived_children_that_render_to_the_same_source() {
    let mut h = support::one_armed_entry_on(CODEX).await;
    configure_distinct_subagent(
        &mut h,
        "rollout",
        "{home}/derived-collision/{session_id}/child-*.jsonl",
    );
    configure_distinct_subagent(
        &mut h,
        "history",
        "{config_dir}/derived-collision/{session_id}/child-*.jsonl",
    );
    rebuild_with_manifest(&h).await;

    support::append_line(&h.store_path, &support::session_meta_line(&h));
    assert!(
        h.poll().captured > 0,
        "the primary record corroborates the bind"
    );
    let mut runtime = h.take();
    let error = runtime
        .bind_start(support::SESSION)
        .await
        .expect_err("the dynamic derived collision must fail before receipt-backed start");
    h.give_back(runtime);
    match error {
        goalcup_daemon::DaemonError::Refused(message) => {
            assert!(message.contains("same source pattern"), "{message}");
        }
        other => panic!("expected derived-source refusal, got {other:?}"),
    }
    assert!(
        h.entry().started_at.is_none(),
        "the failed activation leaves the registry row retryable and unstarted"
    );
}
