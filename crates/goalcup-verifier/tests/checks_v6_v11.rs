//! V6–V11 over the three committed M0 traces.
//!
//! Every fixture is loaded through `goalcup_verifier::Pipeline` — the sequence
//! `verify_trace_with_options` itself runs — so a check is asserted against the context
//! the shipped verifier builds.

// The shared fixture pipeline, pulled in by path so every check test reaches it the same
// way — several of them already carry a file-local `mod support` of scenario builders.
#[path = "support/mod.rs"]
mod fixture_support;

use fixture_support::pipeline::{one, results};
use goalcup_protocol::chain::Link;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::events::{ConfigDeviation, ForeignScanOutcome, ForeignScanUnavailableReason};
use goalcup_verifier::checks::v9_config::V9Config;
use goalcup_verifier::checks::v10_partition::V10Partition;
use goalcup_verifier::checks::{Check, Severity};

#[test]
fn v6_names_every_excluded_user_slot_category() {
    // `ExclusionReason::Excluded(id)` is never produced by a real fixture. Every
    // harness-authored record (`task-notification`, `continue-from-where-you-left-off`,
    // `sidechain`, `environment-context`, `task-tool-subagent-prompt`) is excluded from the
    // human list by the ordinary positive test or a scoping clause, and V6 groups by that
    // reason.
    let rs = results("claude-code-cli");
    let v6 = one(&rs, "V6");
    // `user_slot_excluded` is the exclusion mechanism working, so it is
    // Pass-level — still named, with its evidence, in `Verdict::checks`. The Fail signal a
    // reviewer must decide about is V2's `unwitnessed_not_keep_going`, not a V6 code, so V6
    // is a Pass.
    assert_eq!(v6.verdict, Severity::Pass, "{:?}", v6.findings);
    assert!(
        v6.findings
            .iter()
            .all(|f| f.code == "user_slot_excluded" && !f.evidence.is_empty()),
        "the demotion must keep the records each exclusion cites: {:?}",
        v6.findings
    );
    // All four of claude-code-cli's harness-authored records fail the same test
    // (`origin.kind` is not `human`), so they land in one group instead of four.
    assert_eq!(v6.findings.len(), 1, "{:?}", v6.findings);
    assert!(
        v6.findings[0]
            .message
            .contains("4 record(s) excluded as `failed_positive_test`"),
        "{}",
        v6.findings[0].message
    );

    // Every clause a record failed, not the first. Codex's subagent records fail
    // `first_session_meta_is_tracked` (declared first) and `not_subagent_thread_source`.
    // Both buckets must exist. This includes the subagent rollout's own
    // `<environment_context>` header, which falls through to scoping like its two
    // replayed-parent-turn neighbours — three records in each bucket, not two.
    let rs = results("codex-cli");
    let v6 = one(&rs, "V6");
    assert!(
        v6.findings.iter().any(|f| f
            .message
            .contains("3 record(s) excluded as `first_session_meta_is_tracked`")),
        "{:?}",
        v6.findings
    );
    assert!(
        v6.findings.iter().any(|f| f
            .message
            .contains("3 record(s) excluded as `not_subagent_thread_source`")),
        "{:?}",
        v6.findings
    );
    assert!(
        !v6.findings
            .iter()
            .any(|f| f.message.contains("environment-context")),
        "the catalogue id is gone; nothing names it any more"
    );
}

#[test]
fn v7_reports_the_normalisation_that_was_applied() {
    for id in ["claude-code-cli", "codex-cli"] {
        let rs = results(id);
        let v7 = one(&rs, "V7");
        assert_eq!(v7.verdict, Severity::Pass, "{id}: {:?}", v7.findings);
        assert!(
            v7.findings[0]
                .message
                .contains("exactly one trailing newline")
        );
        assert!(v7.findings[0].message.contains("field `prompt`"));
    }
}

#[test]
fn v8_pairs_tool_calls_and_reports_mismatches_per_join_strategy() {
    // V8 pairs calls rather than counting how much the two id spaces overlap: an id-based
    // join names the specific unmatched ids on each side, and a turn+order join compares
    // per-turn tool-call counts.
    let rs = results("claude-code-cli");
    let v8 = one(&rs, "V8");
    assert!(v8.findings.iter().any(|f| f.code == "join_key_agrees"));
    // The two id sets match exactly on this fixture: nothing to pair-report.
    assert!(!v8.findings.iter().any(
        |f| f.code == "leg1_call_without_leg2_post" || f.code == "leg2_tool_without_leg1_call"
    ));

    let rs = results("codex-cli");
    let v8 = one(&rs, "V8");
    assert!(
        v8.findings
            .iter()
            .any(|f| f.code == "join_key_agrees" && f.message.contains("turn_id+order"))
    );
    // The first turn's three leg-1 `exec` macro calls fan out into six leg-2 records —
    // each `exec` custom call can run several `tools.exec_command()` sub-calls
    // internally. A real, reportable mismatch, not a defect in the check.
    let mismatch = v8
        .findings
        .iter()
        .find(|f| f.code == "turn_tool_call_count_mismatch")
        .expect("V8 compares per-turn tool-call counts under a turn+order join");
    assert!(mismatch.message.contains("3 leg-1"), "{}", mismatch.message);
    assert!(mismatch.message.contains("6 leg-2"), "{}", mismatch.message);
    assert!(
        !v8.findings
            .iter()
            .any(|f| f.code == "tool_id_space_overlap"),
        "replaced by real pairing"
    );
    assert!(
        !v8.findings
            .iter()
            .any(|f| f.code == "disjoint_tool_id_spaces"),
        "that finding is gone: the assertion behind it is false against the bytes"
    );
}

#[test]
fn v9_reports_arming_and_config_events() {
    let rs = results("claude-code-cli");
    let v9 = one(&rs, "V9");
    // An M0 trace carries no daemon event stream
    // at all, so config-deviation/store-rewrite reconciliation cannot run — that is
    // `no_event_stream` at Flag, a distinct fact from "reconciled and found nothing"
    // (`no_config_deviation` at Pass, which does not apply to an M0 fixture).
    assert_eq!(
        v9.verdict,
        Severity::Flag,
        "no event stream in the M0 capture"
    );
    assert!(v9.findings.iter().any(|f| f.code == "no_event_stream"));
    // Claude's `arming.signal` is the name of a procedure, not of a message, so the
    // arming branch counts zero here and there is nothing to report. Asserting the absence
    // keeps the fact visible.
    assert!(
        !v9.findings.iter().any(|f| f.code == "arming_signals"),
        "`launch+first_transcript_record` is never emitted as a ConfigFact"
    );

    // An M0 trace has no event stream, so no store was observed truncated or
    // replaced. The adversarial fixture is where `store_rewritten` fires.
    for id in ["claude-code-cli", "codex-cli"] {
        let rs = results(id);
        let v9 = one(&rs, "V9");
        assert!(
            !v9.findings.iter().any(|f| f.code == "store_rewritten"),
            "{id}"
        );
        assert!(
            !v9.findings
                .iter()
                .any(|f| f.code == "unattributed_config_change"),
            "{id}"
        );
        assert!(
            v9.findings.iter().any(|f| f.code == "no_event_stream"),
            "{id}: an M0 trace carries no event stream"
        );
    }
}

/// The half of the codex `harness_owned` rule that must not change: `.codex/hooks.json`
/// is still watched, so removing it still chains a `ConfigDeviation`, and a deviation no
/// traced write explains is still `unattributed_config_change` at **Fail**.
///
/// The daemon half, the V1 config watch that chained the event, left with D48. This is what
/// the verifier still does with that event in a recorded trace: the deviation is planted into
/// the context the shipped pipeline builds for the codex fixture, changing only the event
/// stream, so the verdict is V9's own and not a constructed one.
#[test]
fn a_removed_codex_hook_file_is_still_an_unattributed_config_change() {
    let fixture = fixture_support::pipeline::m0("codex-cli");
    let entry_id = fixture.context().trace.entry_id.clone();
    let unavailable = Envelope {
        protocol_version: 1,
        entry_id,
        source: "daemon".to_string(),
        recorded_at: "2026-09-07T00:00:00Z".to_string(),
        link: Link {
            seq: 1,
            prev: "0".repeat(64),
            hash: "a".repeat(64),
        },
        body: EventBody::ConfigDeviation(ConfigDeviation {
            path: ".codex/hooks.json".to_string(),
            expected_sha256: "a".repeat(64),
            // The 64 zeros the watch reports for a file that is not there.
            observed_sha256: "0".repeat(64),
            detected_at: "2026-09-07T00:00:00Z".to_string(),
            diff: String::new(),
            restored: true,
            arming_lost: true,
        }),
        redaction: None,
    };
    let mut complete = unavailable.clone();
    complete.link.seq = 100;
    complete.body = EventBody::ForeignScanCoverage {
        declaring_connector: "opencode".to_string(),
        declaring_manifest_sha256: "c".repeat(64),
        scan_index: 0,
        outcome: ForeignScanOutcome::Complete,
        at: "2026-09-04T00:01:00Z".to_string(),
    };
    let mut absent = complete.clone();
    absent.link.seq = 101;
    absent.body = EventBody::ForeignScanCoverage {
        declaring_connector: "opencode".to_string(),
        declaring_manifest_sha256: "c".repeat(64),
        scan_index: 0,
        outcome: ForeignScanOutcome::Absent,
        at: "2026-09-04T00:02:00Z".to_string(),
    };
    let events = vec![unavailable, complete, absent];
    let mut cx = fixture.context();
    cx.events = &events;
    let result = V9Config.run(&cx);
    assert_eq!(
        result.verdict,
        Severity::Fail,
        "a deviation on a watched hook file is not attributable to any traced write: {:?}",
        result.findings
    );
    let finding = result
        .findings
        .iter()
        .find(|f| f.code == "unattributed_config_change")
        .unwrap_or_else(|| panic!("{:?}", result.findings));
    assert_eq!(finding.severity, Severity::Fail);
    assert!(
        finding.message.contains("hooks.json"),
        "the finding names the file: {}",
        finding.message
    );
}

#[test]
fn v10_partitions_every_record_and_names_the_foreign_stores() {
    let rs = results("claude-code-cli");
    let v10 = one(&rs, "V10");
    assert!(
        !v10.findings.iter().any(|f| f.code == "partition_violation"),
        "claude-code-cli: the fixtures are already partitioned; nothing tracked may fail its key"
    );
    // It says `foreign_records_present`: the `a9-other-folder` probe file is declared foreign in the
    // descriptor. The finding is Pass-level: it states that records were correctly
    // partitioned away — the probe's rows are not in this entry's evidence — which is the
    // check succeeding. A record that actually reached the chain in breach of its key is
    // `partition_violation`, asserted absent above and a Fail.
    let claude = results("claude-code-cli");
    let v10 = one(&claude, "V10");
    assert_eq!(v10.verdict, Severity::Pass, "{:?}", v10.findings);
    let foreign = v10
        .findings
        .iter()
        .find(|f| f.code == "foreign_records_present")
        .unwrap_or_else(|| panic!("{:?}", v10.findings));
    assert!(
        !foreign.evidence.is_empty(),
        "the demotion must keep the store the finding cites"
    );

    let rs = results("codex-cli");
    let v10 = one(&rs, "V10");
    // The chain sweep reads every
    // record of a `Direct`-partition store. Codex's `history.jsonl` is one shared,
    // append-only log across every thread on the machine, and the M0 fixture's own bytes
    // measure three records (seq 6-8) stamped with the other thread's session id — the
    // same thread `wrong-cwd-01a06616` names — landing inside the tracked thread's
    // `history` store. This is a real, per-record Fail, not the fixture-declaration Flag
    // path, so codex-cli's V10 verdict is Fail, unlike the other two connectors above.
    assert_eq!(v10.verdict, Severity::Fail, "{:?}", v10.findings);
    let violations: Vec<_> = v10
        .findings
        .iter()
        .filter(|f| f.code == "partition_violation")
        .collect();
    assert_eq!(violations.len(), 3, "{violations:?}");
    for f in &violations {
        assert_eq!(f.severity, Severity::Fail);
        assert!(f.message.contains("history"), "{}", f.message);
    }
    let seqs: std::collections::BTreeSet<Option<u64>> =
        violations.iter().map(|f| f.evidence[0].seq).collect();
    assert_eq!(
        seqs,
        [Some(6), Some(7), Some(8)].into_iter().collect(),
        "{seqs:?}"
    );

    assert!(
        v10.findings.iter().any(
            |f| f.code == "foreign_records_present" && f.message.contains("wrong-cwd-01a06616")
        )
    );
    // The same store has no teaching record, so it is also reported as
    // unattributable — its records belong to no session establishable from its bytes.
    assert!(
        v10.findings
            .iter()
            .any(|f| f.code == "unattributable_store" && f.message.contains("wrong-cwd-01a06616")),
        "a file_first_record store with no session_meta must be named, not dropped"
    );
}

#[test]
fn v10_flags_only_explicit_unavailable_foreign_scan_coverage() {
    let fixture = fixture_support::pipeline::m0("claude-code-cli");
    let events = vec![Envelope {
        protocol_version: 2,
        entry_id: fixture.context().trace.entry_id.clone(),
        source: "daemon".to_string(),
        recorded_at: "2026-09-04T00:00:00Z".to_string(),
        link: Link {
            seq: 99,
            prev: "a".repeat(64),
            hash: "b".repeat(64),
        },
        body: EventBody::ForeignScanCoverage {
            declaring_connector: "opencode".to_string(),
            declaring_manifest_sha256: "c".repeat(64),
            scan_index: 0,
            outcome: ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Open),
            at: "2026-09-04T00:00:00Z".to_string(),
        },
        redaction: None,
    }];
    let mut cx = fixture.context();
    cx.events = &events;
    let result = V10Partition.run(&cx);
    let incomplete: Vec<_> = result
        .findings
        .iter()
        .filter(|finding| finding.code == "foreign_scan_incomplete")
        .collect();
    assert_eq!(incomplete.len(), 1, "complete and absent are not findings");
    assert_eq!(incomplete[0].severity, Severity::Flag);
    assert_eq!(incomplete[0].evidence[0].seq, Some(99));
    let evidence_hash = "b".repeat(64);
    assert_eq!(
        incomplete[0].evidence[0].sha256.as_deref(),
        Some(evidence_hash.as_str())
    );

    let empty = Vec::new();
    cx.events = &empty;
    assert!(
        V10Partition
            .run(&cx)
            .findings
            .iter()
            .all(|finding| finding.code != "foreign_scan_incomplete")
    );
}

#[test]
fn v11_sums_tokens_per_model_and_strips_billing_fields() {
    for id in ["claude-code-cli", "codex-cli"] {
        let rs = results(id);
        let v11 = one(&rs, "V11");
        assert_eq!(
            v11.verdict,
            Severity::Pass,
            "{id}: no rule depends on a token number (D26)"
        );
        let msg = &v11.findings[0].message;
        assert!(msg.contains("prompt="), "{id}: {msg}");
        assert!(
            v11.findings.iter().any(|f| f.code == "stripped"),
            "{id} must name the fields dropped before publication"
        );
    }
    let rs = results("codex-cli");
    let v11 = one(&rs, "V11");
    assert!(
        v11.findings
            .iter()
            .any(|f| f.code == "stripped" && f.message.contains("rate_limits"))
    );
}

/// The parser emits Codex's *raw* tool names (`exec`, `spawn_agent`, `wait_agent`) — never
/// the classic `shell`/`apply_patch` function-call names, which would make
/// `tool_classes.write` match nothing on any real capture. This loads the real fixture (not a
/// constructed one) and checks that at least one traced call falls in the write set V9's
/// attribution reads from.
#[test]
fn codex_fixture_yields_a_non_empty_write_set_for_v9() {
    let fixture = fixture_support::pipeline::m0("codex-cli");
    let cx = fixture.context();

    let write_calls: Vec<&str> = cx
        .parsed
        .trajectory
        .steps
        .iter()
        .flat_map(|s| s.tool_calls.iter())
        .filter(|c| cx.manifest.tool_classes.write.contains(&c.function_name))
        .map(|c| c.function_name.as_str())
        .collect();
    assert!(
        !write_calls.is_empty(),
        "the real fixture's traced calls must be recognised as writes now that the \
         manifest names the parser's raw tool names, not `shell`/`apply_patch`"
    );
    assert!(write_calls.iter().all(|n| *n == "exec"), "{write_calls:?}");
}
