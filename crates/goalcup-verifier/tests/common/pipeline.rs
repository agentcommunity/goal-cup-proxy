//! One fixture-loading pipeline for every integration test, and the roster of finding
//! codes this build can emit.
//!
//! Both live here for the same reason: a test that builds its own version of either one
//! stops measuring the verifier. A hand-rolled loading sequence drifts away from
//! `verify_trace_with_options`: it skips `manifest.validate()` and the connector gate,
//! passes `gaps::NO_ACKS` beside a real acknowledgement stream, and builds a bare
//! `ManifestPinning`, which reads `manifest_pin: Unpinned` and is a state the shipped
//! pipeline cannot produce for a pinned connector. `goalcup_verifier::Pipeline` is the
//! shipped sequence, and this module is the two lines of glue between it and a fixture
//! directory.
//!
//! The roster is the other half of the same idea. `expected-verdict.json` carries a
//! `must_not_contain` list, and a code on it that no build can emit forbids nothing: the
//! entry reads as a guard and is a comment. Every entry is checked against
//! `is_an_emittable_code` before its polarity is asserted, so a code that is renamed or
//! deleted fails the suite instead of quietly ceasing to guard anything.
//!
//! Reached through `tests/common/mod.rs`, so all M0 copies use the one descriptor
//! materializer and preserve the historical descriptor text outside path values.

// Each test binary uses the part of this module it needs; the rest is not dead, it is
// somebody else's.
#![allow(dead_code)]

use goalcup_verifier::checks::CheckResult;
use goalcup_verifier::{Pipeline, VerifyOptions};
use std::path::{Path, PathBuf};

/// One of the three committed M0 trace directories.
pub fn trace_dir(id: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/traces")
        .join(id)
}

/// The shipped pipeline over one of the three committed M0 traces.
pub fn m0(id: &str) -> Pipeline {
    let fixture = super::materialize_trace(&trace_dir(id));
    at(fixture.path())
}

/// The shipped pipeline over any trace directory — a fixture, or one a test just wrote.
///
/// `VerifyOptions::default()` is the strict reading, so a context borrowed from this is
/// the context `verify_trace` builds. A test that needs a development escape hatch calls
/// `Pipeline::load` with its own options and says so.
pub fn at(dir: &Path) -> Pipeline {
    Pipeline::load(dir, VerifyOptions::default())
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
}

/// The thirteen checks over one committed M0 trace.
pub fn results(id: &str) -> Vec<CheckResult> {
    m0(id).run_checks()
}

/// One check's result by id.
pub fn one<'a>(rs: &'a [CheckResult], id: &str) -> &'a CheckResult {
    rs.iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("no check {id}"))
}

/// Every code V13 can emit.
///
/// Read by `checks_v13.rs`, which asserts that the set V13 actually emits across its
/// scenarios equals this list exactly — so this half of the roster is not a claim, it is
/// a measurement.
pub const V13_CODES: &[&str] = &[
    // Structural, rendered out of `chain::load_and_verify` on every chain trace.
    "chain_broken",
    "protocol_version_unsupported",
    "envelope_version_body_incompatible",
    "truncated_tail",
    "redaction_audit_invalid",
    "legacy_redaction_log_missing",
    // The binding itself.
    "receipt_missing",
    "receipt_invalid",
    "receipt_unverified",
    "descriptor_mismatch",
    "chain_head_mismatch",
    "binding_checked",
    // The baseline, which is the other half of the same binding: seq 0.
    "baseline_missing",
    "baseline_not_chained",
    // The window. `outside_window` is the upper edge and the only one that fails; the
    // lower edge is reported at two severities and enforced at neither, because the
    // arming records that sit below `started_at` are what cause the receipt that dates it.
    "window_unmeasurable",
    "window_unplaceable",
    "outside_window",
    "seal_after_deadline",
    "pre_start_records",
    "pre_start_records_early",
    // The seal.
    "seal_not_chained",
    // The rules the run was judged by, which are a participant input until they are pinned.
    "manifest_not_pinned",
    // And the hook bodies beside them, which no trace carries: the daemon hashes its own
    // connector directory and the server signs the value into the receipt.
    "connector_dir_unpinned",
    "connector_dir_unbound",
    // Who the verifier borrowed its trust from: a key this build does not pin, and the
    // published placeholder that it does.
    "server_key_unpinned",
    "placeholder_server_key",
    // The legacy escape hatch.
    "legacy_trace",
];

/// Every code V1–V12 and `verify_trace` itself can emit, in check order.
///
/// Pass codes are on it too: a `must_not_contain` entry naming one is still a statement
/// about a code that exists, and the polarity assertion is what decides whether it may
/// appear. `codes_on_the_roster_are_spelled_the_way_the_verifier_spells_them` reads every
/// entry back out of `src/`, so a rename in a check fails here rather than turning an
/// expectation into a comment.
pub const CHECK_CODES: &[&str] = &[
    // V1 — the goal.
    "goal_recovered",
    "goal_missing",
    "goal_too_long",
    // V2 — `keep going`.
    "all_keep_going",
    "not_keep_going",
    "unwitnessed_keep_going",
    "unwitnessed_not_keep_going",
    // V3 — the two legs agree.
    "legs_agree",
    "leg2_without_leg1_record",
    "leg1_without_leg2_post",
    "text_divergence",
    "duplicate_pairing",
    "message_without_pair_key",
    "cross_leg_identifiers",
    "cross_leg_identifiers_disjoint",
    "cross_leg_identifiers_mismatch",
    "tool_args_conflict",
    "tool_args_cross_leg",
    // V4 — continuity, and the gap reasons it renders.
    "resume",
    "no_resume_marker",
    "second_root_thread",
    "foreign_session",
    "one_tracked_session",
    "tracked_session_count",
    "clean_gap",
    "dirty_gap",
    "heartbeat_unparseable",
    "unanchored_records",
    "unplaceable_records",
    "cursor_mismatch",
    "cursor_ahead_of_chain",
    "restart_head_mismatch",
    "heartbeats_out_of_order",
    // V5 — the model.
    "single_model",
    "model_changed",
    "model_unknown",
    "no_version_drift",
    "version_drift",
    // V6 — the user slot.
    "no_harness_authored_user_slot",
    "user_slot_excluded",
    // V7 — normalisation.
    "normalisation_applied",
    "double_newline_survived",
    // V8 — pairing.
    "join_key_agrees",
    "join_key_mismatch",
    "no_tool_calls",
    "unkeyed_leg1_tool_calls",
    "leg1_call_without_leg2_post",
    "leg2_tool_without_leg1_call",
    "turn_tool_call_count_mismatch",
    // V9 — configuration and the event stream.
    "config_event",
    "no_config_deviation",
    "config_change_attributed",
    "unattributed_config_change",
    "store_reread",
    "store_rewritten",
    "arming_signals",
    "no_event_stream",
    // V10 — the partition.
    "partitioned",
    "partition_violation",
    "foreign_records_present",
    "unattributable_store",
    // V11 — tokens.
    "token_summary",
    "no_token_usage",
    "stripped",
    // V12 — the anchors.
    "no_anchor",
    "anchors_checked",
    "anchor_timeline",
    "anchor_unverified",
    "ack_chain_mismatch",
    "ack_entry_mismatch",
    "ack_signature_invalid",
    "late_first_anchor",
    "long_unanchored_span",
    // Raised by `verify_trace` outside any check: seal attribution and the gaps V4's
    // rendering rules would skip.
    "unexplained_seal_file",
    "read_only_attribution",
    "bulk_attribution",
    "unrendered_gap",
];

/// Whether `code` names a finding this build can emit.
///
/// The question a `must_not_contain` entry has to answer before it forbids anything.
pub fn is_an_emittable_code(code: &str) -> bool {
    CHECK_CODES.contains(&code) || V13_CODES.contains(&code)
}
