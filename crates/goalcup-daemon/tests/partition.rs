//! Capture-time partitioning (D20, Global Constraint 3 stage 1): a foreign row is
//! discarded here, before redaction and before any hash, so it is never persisted.
#![cfg(feature = "v1-compat")]

use goalcup_daemon::partition::{DiscardReason, Outcome, Partitioner, tracked_for};
use goalcup_protocol::manifest::PartitionSpec;

fn rec(
    store: &str,
    path: &str,
    value: serde_json::Value,
) -> goalcup_daemon::tailer::CapturedRecord {
    goalcup_daemon::tailer::CapturedRecord {
        store: store.to_string(),
        source_path: path.to_string(),
        payload: serde_json::Value::String(value.to_string()),
        value,
        bytes: Vec::new(),
    }
}

#[test]
fn the_first_session_meta_decides_the_whole_rollout_file() {
    let spec = PartitionSpec::FileFirstRecord {
        select_path: "type".to_string(),
        select_value: "session_meta".to_string(),
        value_path: "payload.cwd".to_string(),
    };
    let mut p = Partitioner::new(Some(spec), vec!["/ws/e1".to_string()]);
    assert_eq!(
        p.admit(&rec(
            "main",
            "a.jsonl",
            serde_json::json!({"type":"session_meta","payload":{"cwd":"/ws/e1"}})
        )),
        Outcome::Keep
    );
    // The subagent rollout's second record is the parent's header verbatim.
    // The file's verdict was decided by its first record and must not be re-decided.
    assert_eq!(
        p.admit(&rec(
            "main",
            "a.jsonl",
            serde_json::json!({"type":"session_meta","payload":{"cwd":"/elsewhere"}})
        )),
        Outcome::Keep
    );
    assert_eq!(
        p.admit(&rec(
            "main",
            "b.jsonl",
            serde_json::json!({"type":"session_meta","payload":{"cwd":"/elsewhere"}})
        )),
        Outcome::Discard(DiscardReason::ForeignFile)
    );
    assert_eq!(p.discarded(), 1);
}

#[test]
fn a_direct_partitioner_keeps_nothing_until_the_session_binds_and_then_keeps_its_rows() {
    // The codex `history` store partitions on session_id: handed the workspace path, it
    // would discard every record, including the only witness of the blocked prompt.
    let spec = PartitionSpec::Direct {
        path: "session_id".to_string(),
    };
    assert_eq!(
        tracked_for(Some(&spec), std::path::Path::new("/ws/e1"), None),
        Vec::<String>::new()
    );
    assert_eq!(
        tracked_for(
            Some(&spec),
            std::path::Path::new("/ws/e1"),
            Some("thread-9")
        ),
        vec!["thread-9".to_string()]
    );

    let mut p = Partitioner::new(Some(spec), Vec::new());
    let r = rec(
        "history",
        "history.jsonl",
        serde_json::json!({"session_id":"thread-9"}),
    );
    assert!(matches!(p.admit(&r), Outcome::Discard(_)));
    p.set_tracked(vec!["thread-9".to_string()]);
    assert_eq!(
        p.admit(&r),
        Outcome::Keep,
        "the learned caches are cleared, not carried"
    );
}

fn shipped(connector: &str) -> goalcup_protocol::manifest::ConnectorManifest {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors")
        .join(connector)
        .join("manifest.toml");
    goalcup_protocol::manifest::ConnectorManifest::from_toml_str(
        &std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display())),
    )
    .unwrap()
}

#[test]
fn each_codex_store_is_seeded_with_what_its_own_strategy_compares() {
    // Stated against the shipped manifest: one `vec![workspace]` for every store would
    // compare a thread UUID against a filesystem path on `history` and discard
    // the only leg-1 witness of the blocked prompt.
    let m = shipped("codex-cli");
    let ws = std::path::Path::new("/ws/e1");
    let seed = |name: &str| {
        tracked_for(
            m.store_named(name).unwrap().partition.as_ref(),
            ws,
            Some("thread-9"),
        )
    };
    assert_eq!(seed("rollout"), vec!["/ws/e1".to_string()]);
    assert_eq!(seed("history"), vec!["thread-9".to_string()]);
}

#[test]
fn a_store_that_declares_no_partition_keeps_everything() {
    // Claude Code's `main` store hangs off the per-run config dir and carries an
    // `unpartitioned_reason`: there is no shared store to separate it from.
    let m = shipped("claude-code-cli");
    let store = m.store_named("main").unwrap();
    assert!(store.partition.is_none() && store.unpartitioned_reason.is_some());
    assert_eq!(
        tracked_for(
            store.partition.as_ref(),
            std::path::Path::new("/ws/e1"),
            None
        ),
        Vec::<String>::new()
    );
    let mut p = Partitioner::new(store.partition.clone(), Vec::new());
    assert_eq!(
        p.admit(&rec(
            "tool_output",
            "tool-output/x",
            serde_json::json!({"anything": 1})
        )),
        Outcome::Keep
    );
    assert_eq!(p.discarded(), 0);
}

#[test]
fn a_direct_row_with_no_partition_field_at_all_is_discarded_not_kept() {
    let mut p = Partitioner::new(
        Some(PartitionSpec::Direct {
            path: "session_id".to_string(),
        }),
        vec!["thread-9".to_string()],
    );
    // Absent, and present-but-not-a-string: neither is permission to keep the row.
    assert_eq!(
        p.admit(&rec(
            "history",
            "history.jsonl",
            serde_json::json!({"text": "hi"})
        )),
        Outcome::Discard(DiscardReason::ForeignValue)
    );
    assert_eq!(
        p.admit(&rec(
            "history",
            "history.jsonl",
            serde_json::json!({"session_id": 9})
        )),
        Outcome::Discard(DiscardReason::ForeignValue)
    );
    assert_eq!(p.discarded(), 2);
    assert_eq!(p.unknown_aggregates(), 0);
    assert_eq!(
        p.tracked().iter().cloned().collect::<Vec<_>>(),
        vec!["thread-9".to_string()]
    );
}

#[test]
fn a_file_discards_every_record_until_its_first_valid_decider_then_keeps_that_decision() {
    let spec = PartitionSpec::FileFirstRecord {
        select_path: "type".to_string(),
        select_value: "session_meta".to_string(),
        value_path: "payload.cwd".to_string(),
    };
    let mut p = Partitioner::new(Some(spec), vec!["/ws/e1".to_string()]);
    assert_eq!(
        p.admit(&rec(
            "rollout",
            "c.jsonl",
            serde_json::json!({"type": "response_item"})
        )),
        Outcome::Discard(DiscardReason::UndecidedFile),
        "ordinary bytes before the deciding header cannot be attributed safely"
    );
    // Missing and malformed values decide nothing either. They are discarded like the
    // prelude, leaving the first valid deciding record free to decide the file.
    assert_eq!(
        p.admit(&rec(
            "rollout",
            "c.jsonl",
            serde_json::json!({"type": "session_meta"})
        )),
        Outcome::Discard(DiscardReason::UndecidedFile)
    );
    assert_eq!(
        p.admit(&rec(
            "rollout",
            "c.jsonl",
            serde_json::json!({"type":"session_meta","payload":{"cwd":9}})
        )),
        Outcome::Discard(DiscardReason::UndecidedFile)
    );
    // Late owned metadata is the first valid decider, so both it and following records are
    // kept. A later valid header cannot re-decide the file.
    assert_eq!(
        p.admit(&rec(
            "rollout",
            "c.jsonl",
            serde_json::json!({"type":"session_meta","payload":{"cwd":"/ws/e1"}})
        )),
        Outcome::Keep
    );
    assert_eq!(
        p.admit(&rec(
            "rollout",
            "c.jsonl",
            serde_json::json!({"type": "response_item"})
        )),
        Outcome::Keep
    );
    assert_eq!(
        p.admit(&rec(
            "rollout",
            "c.jsonl",
            serde_json::json!({"type":"session_meta","payload":{"cwd":"/elsewhere"}})
        )),
        Outcome::Keep,
        "the first valid deciding record wins"
    );
    assert_eq!(
        p.discarded(),
        3,
        "the bounded counter is all that remains of undecided bytes"
    );
}

#[test]
fn a_re_seeded_file_partitioner_re_judges_the_file_it_had_defaulted() {
    // The rule, on the strategy that actually caches: a verdict reached against an
    // empty tracked set was defaulted, not judged, so `set_tracked` clears it.
    let spec = PartitionSpec::FileFirstRecord {
        select_path: "type".to_string(),
        select_value: "session_meta".to_string(),
        value_path: "payload.cwd".to_string(),
    };
    let header = rec(
        "rollout",
        "a.jsonl",
        serde_json::json!({"type":"session_meta","payload":{"cwd":"/ws/e1"}}),
    );
    let mut p = Partitioner::new(Some(spec), Vec::new());
    assert_eq!(
        p.admit(&header),
        Outcome::Discard(DiscardReason::ForeignFile)
    );
    p.set_tracked(vec!["/ws/e1".to_string()]);
    assert_eq!(p.admit(&header), Outcome::Keep);
}

#[test]
fn an_uncanonicalised_tracked_workspace_makes_a_private_var_cwd_foreign() {
    // The compare is byte-exact on purpose (normalising would be
    // interpretation), so the seeding site carries the obligation: on macOS a tempdir is
    // handed out as `/var/folders/…` and `std::fs::canonicalize` — and the harness's own
    // `cwd` — spell it `/private/var/folders/…`.
    let spec = PartitionSpec::FileFirstRecord {
        select_path: "type".to_string(),
        select_value: "session_meta".to_string(),
        value_path: "payload.cwd".to_string(),
    };
    let header = rec(
        "rollout",
        "r.jsonl",
        serde_json::json!({
            "type": "session_meta",
            "payload": {"cwd": "/private/var/folders/xy/T/ws-e1"},
        }),
    );

    let uncanonical = std::path::Path::new("/var/folders/xy/T/ws-e1");
    let mut p = Partitioner::new(
        Some(spec.clone()),
        tracked_for(Some(&spec), uncanonical, None),
    );
    assert_eq!(
        p.admit(&header),
        Outcome::Discard(DiscardReason::ForeignFile),
        "an uncanonicalised workspace discards the participant's own rollout"
    );

    // A trailing slash is the same class of defect.
    let mut slashed = Partitioner::new(
        Some(spec.clone()),
        vec!["/private/var/folders/xy/T/ws-e1/".to_string()],
    );
    assert_eq!(
        slashed.admit(&header),
        Outcome::Discard(DiscardReason::ForeignFile)
    );

    // Seeded the way the contract requires, the same record is kept.
    let canonical = std::path::Path::new("/private/var/folders/xy/T/ws-e1");
    let mut ok = Partitioner::new(
        Some(spec.clone()),
        tracked_for(Some(&spec), canonical, None),
    );
    assert_eq!(ok.admit(&header), Outcome::Keep);
}

#[test]
fn a_header_carrying_no_value_does_not_decide_and_a_later_one_does() {
    // The manifest says "the file's first matching record decides", and
    // the code needs the first matching record that also carries a value — a header with
    // no `cwd` decides nothing, because there is nothing to decide with.
    let spec = PartitionSpec::FileFirstRecord {
        select_path: "type".to_string(),
        select_value: "session_meta".to_string(),
        value_path: "payload.cwd".to_string(),
    };
    let mut p = Partitioner::new(Some(spec), vec!["/ws/e1".to_string()]);
    // A matching header with no value at `value_path` decides nothing, so its bytes are
    // discarded while the file remains open to the first later valid deciding value.
    assert_eq!(
        p.admit(&rec(
            "rollout",
            "a.jsonl",
            serde_json::json!({"type":"session_meta","payload":{}})
        )),
        Outcome::Discard(DiscardReason::UndecidedFile)
    );
    // The next header that does carry one decides the file.
    assert_eq!(
        p.admit(&rec(
            "rollout",
            "a.jsonl",
            serde_json::json!({"type":"session_meta","payload":{"cwd":"/elsewhere"}})
        )),
        Outcome::Discard(DiscardReason::ForeignFile)
    );
}

/// Codex's `SessionStart` fires at the first prompt, and
/// that is the same moment the prompt is written to `history.jsonl` — D2's only leg-1
/// witness for a hook-blocked goal. `Daemon::tick` drains the stores and only then binds,
/// so with an empty tracked set that record would be judged `ForeignValue` and, because `admit`
/// judges once, never offered again.
///
/// The workspace is empty at arm and the pre-arm scan refused foreign sessions, so the
/// first session id a `Direct` store sees after arming is the tracked one. It is taught,
/// reported back for `bind_start` to reconcile against, and everything after it is judged
/// against it — so a second thread is still foreign.
#[test]
fn a_direct_store_armed_to_teach_keeps_the_first_thread_and_refuses_the_second() {
    let spec = PartitionSpec::Direct {
        path: "session_id".to_string(),
    };
    let mut p = Partitioner::new(Some(spec), Vec::new());
    // Off until the runtime arms it: before the workspace is armed a `Direct` store has no
    // ground for calling the first id it sees the participant's.
    assert_eq!(
        p.admit(&rec(
            "history",
            "history.jsonl",
            serde_json::json!({"session_id":"thread-early","text":"before arming"})
        )),
        Outcome::Discard(DiscardReason::ForeignValue)
    );
    assert_eq!(p.taught(), None);

    p.teach_on_first_sight();
    assert_eq!(
        p.admit(&rec(
            "history",
            "history.jsonl",
            serde_json::json!({"session_id":"thread-9","text":"the 4100-char goal"})
        )),
        Outcome::Keep,
        "the record that arrives with the start signal is the participant's own"
    );
    assert_eq!(
        p.taught(),
        Some("thread-9"),
        "reported back, for bind_start"
    );
    assert_eq!(
        p.tracked().iter().cloned().collect::<Vec<_>>(),
        vec!["thread-9".to_string()]
    );

    // Taught once. Spike 3 P3's foreign second thread stays foreign.
    assert_eq!(
        p.admit(&rec(
            "history",
            "history.jsonl",
            serde_json::json!({"session_id":"thread-other","text":"another window"})
        )),
        Outcome::Discard(DiscardReason::ForeignValue)
    );
    assert_eq!(p.taught(), Some("thread-9"), "and never re-taught");
    // Later rows of the taught thread keep being kept.
    assert_eq!(
        p.admit(&rec(
            "history",
            "history.jsonl",
            serde_json::json!({"session_id":"thread-9","text":"keep going"})
        )),
        Outcome::Keep
    );
    assert_eq!(p.discarded(), 2);
}

/// The mode is for `Direct` stores and nothing else: a `FileFirstRecord` or
/// `AggregateLearned` store compares a workspace PATH that `tracked_for` already knows at
/// construction, so there is nothing there for a first record to teach and arming must not
/// turn one on. Its tracked set is never empty in the first place, and a row carrying a
/// foreign directory stays foreign.
#[test]
fn arming_the_teach_mode_never_teaches_a_store_that_partitions_on_the_workspace() {
    let spec = PartitionSpec::FileFirstRecord {
        select_path: "type".to_string(),
        select_value: "session_meta".to_string(),
        value_path: "payload.cwd".to_string(),
    };
    let mut p = Partitioner::new(Some(spec), vec!["/ws/e1".to_string()]);
    p.teach_on_first_sight();
    assert_eq!(
        p.admit(&rec(
            "rollout",
            "b.jsonl",
            serde_json::json!({"type":"session_meta","payload":{"cwd":"/elsewhere"}})
        )),
        Outcome::Discard(DiscardReason::ForeignFile)
    );
    assert_eq!(p.taught(), None);
}

/// `set_tracked` re-seeding a `Direct` store with an empty vec (the shape `tracked_for`
/// produces before the session binds) leaves `tracked.is_empty()` true again, but
/// `taught` is a fact reported once for `bind_start` to reconcile against and must never
/// be silently overwritten by a later record. Without also gating on `taught.is_none()`,
/// a record carrying a different id arriving after such a re-seed would re-teach.
#[test]
fn a_direct_store_re_seeded_empty_after_teaching_does_not_re_teach() {
    let spec = PartitionSpec::Direct {
        path: "session_id".to_string(),
    };
    let mut p = Partitioner::new(Some(spec), Vec::new());
    p.teach_on_first_sight();
    assert_eq!(
        p.admit(&rec(
            "history",
            "history.jsonl",
            serde_json::json!({"session_id":"thread-9","text":"the first record"})
        )),
        Outcome::Keep
    );
    assert_eq!(p.taught(), Some("thread-9"));

    // A re-seed with an empty vec — the `set_tracked(Vec::new())` shape a `Direct` store
    // sees when the bind names no tracked value — clears `tracked` but must not clear
    // `taught`.
    p.set_tracked(Vec::new());
    assert_eq!(
        p.taught(),
        Some("thread-9"),
        "set_tracked never clears what was taught"
    );

    // A record for a different id now meets an empty tracked set and an armed teach mode,
    // exactly the condition that would otherwise re-teach. It must be judged foreign, not adopted.
    assert_eq!(
        p.admit(&rec(
            "history",
            "history.jsonl",
            serde_json::json!({"session_id":"thread-other","text":"a foreign thread"})
        )),
        Outcome::Discard(DiscardReason::ForeignValue),
        "already taught; a second id must not silently re-teach"
    );
    assert_eq!(p.taught(), Some("thread-9"), "still the original teaching");
}
