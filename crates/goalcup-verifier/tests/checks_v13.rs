//! V13: the trace is bound to a start the participant did not sign.
//!
//! Every scenario below is built through `goalcup_protocol::export::TraceExport` into a
//! fresh temporary directory and then mutated there. **Nothing under `spikes/` or
//! `tests/traces/` is ever written**: the committed M0
//! descriptors are read only, and `m0_trace` hands them to `verify_trace` by path.

use goalcup_protocol::pins::{CONNECTOR_DIR_SHA256S, pinned_connector_dir_sha256};
use goalcup_verifier::attribution::Attribution;
use goalcup_verifier::checks::Severity;

#[path = "support/mod.rs"]
mod fixture_support;

/// The substituted-frame attack: `trace.toml` supplies the workspace, the tracked
/// sessions and the entry id, and V13 compares them to the signed receipt.
#[test]
fn a_descriptor_that_disagrees_with_the_receipt_fails() {
    let t = support::compliant_trace();
    let mut doc = support::read_descriptor(&t);
    doc.workspace = "/some/other/ws".to_string();
    support::write_descriptor(&t, &doc);

    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let v13 = v.checks.iter().find(|c| c.id == "V13").unwrap();
    assert_eq!(v13.verdict, Severity::Fail);
    let codes: Vec<&str> = v13.findings.iter().map(|f| f.code.as_str()).collect();
    assert!(codes.contains(&"descriptor_mismatch"), "{codes:?}");
    let m = &v13
        .findings
        .iter()
        .find(|f| f.code == "descriptor_mismatch")
        .unwrap()
        .message;
    assert!(
        m.contains("workspace"),
        "the message names the field that differs: {m}"
    );
    // The field is named and its value is not: a workspace is an absolute path, and the
    // verdict is a document a reviewer publishes.
    assert!(!m.contains("/some/other/ws"), "{m}");
}

#[test]
fn a_self_signed_receipt_is_receipt_invalid_and_says_which_key_signed_it() {
    let t = support::compliant_trace_signed_by(&[7u8; 32]);
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "receipt_invalid")
        .expect("receipt_invalid");
    assert_eq!(f.severity, Severity::Fail);
    assert!(f.message.contains("public key"), "{}", f.message);
    // The two keys are distinguished by a short prefix, never carried whole: a verdict is
    // published, and a full key in a message is a value a reader can mistake for a pin.
    let signer = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
    let full = hex::encode(signer.verifying_key().to_bytes());
    assert!(!f.message.contains(&full), "{}", f.message);
    assert!(f.message.contains(&full[..8]), "{}", f.message);
}

#[test]
fn a_trace_without_a_receipt_is_receipt_missing_unless_it_is_a_legacy_m0_fixture() {
    // A chain trace with no receipt: the binding is simply absent.
    let t = support::compliant_trace_without_receipt();
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    assert!(
        v.review_queue
            .iter()
            .any(|f| f.code == "receipt_missing" && f.severity == Severity::Fail)
    );

    // The three committed M0 descriptors declare [[leg1]] and no chain: they say so and
    // are not punished for a receipt that did not exist when they were captured.
    //
    // One finding, and one only. No `connector_dir_unbound` sits beside it: every statement
    // V13 makes about the connector directory is read out of a receipt whose signature
    // verified, and a capture with no receipt has no such document to be old. `legacy_trace`
    // is already the sentence saying no start was carried, and a second Flag beside it would
    // say the same absence twice.
    for id in ["claude-code-cli", "codex-cli"] {
        let v = goalcup_verifier::verify_trace(
            fixture_support::materialize_fixture(&format!("tests/traces/{id}")).path(),
        )
        .unwrap();
        let v13 = v.checks.iter().find(|c| c.id == "V13").unwrap();
        assert_eq!(v13.verdict, Severity::Pass, "{id}: {:?}", v13.findings);
        assert_eq!(
            v13.findings
                .iter()
                .map(|f| f.code.as_str())
                .collect::<Vec<_>>(),
            vec!["legacy_trace"],
            "{id}"
        );
        assert!(!v.review_queue.iter().any(|f| f.code == "receipt_missing"));
    }
}

#[test]
fn receipt_bindings_are_required_only_for_the_new_terminal_contract() {
    let legacy = support::compliant_trace();
    let verdict = goalcup_verifier::verify_trace(legacy.path()).unwrap();
    assert!(
        !verdict.review_queue.iter().any(|finding| {
            finding.code == "receipt_binding_invalid"
                || finding.code == "receipt_binding_required_for_finalization"
        }),
        "a verified legacy receipt with the legacy seal stays compatible"
    );

    let unbound = support::finalized_trace_with_legacy_receipt();
    let verdict = goalcup_verifier::verify_trace(unbound.path()).unwrap();
    assert!(verdict.review_queue.iter().any(|finding| {
        finding.code == "receipt_binding_required_for_finalization"
            && finding.severity == Severity::Fail
    }));

    for trace in [
        support::finalized_trace_with_half_binding(),
        support::finalized_trace_with_malformed_binding(),
    ] {
        let verdict = goalcup_verifier::verify_trace(trace.path()).unwrap();
        assert!(verdict.review_queue.iter().any(|finding| {
            finding.code == "receipt_binding_invalid" && finding.severity == Severity::Fail
        }));
        assert!(
            !verdict
                .checks
                .iter()
                .flat_map(|check| &check.findings)
                .any(|finding| finding.code == "binding_checked"),
            "an invalid optional pair cannot also receive the binding summary"
        );
    }

    let bound = support::finalized_trace_with_bound_receipt();
    let verdict = goalcup_verifier::verify_trace(bound.path()).unwrap();
    assert!(
        !verdict.review_queue.iter().any(|finding| {
            finding.code == "receipt_binding_invalid"
                || finding.code == "receipt_binding_required_for_finalization"
        }),
        "a valid pair passes this receipt gate without claiming tree or publication proof"
    );
}

/// A V4 workspace submission is not passable on its terminal claim alone. The independent
/// reader validates Captured separately; the historical terminal states remain
/// non-qualifying before artifact content can help.
#[test]
fn workspace_submission_terminal_claims_fail_closed_before_artifact_verification() {
    for (trace, expected_code) in [
        (
            support::workspace_submission_with_process_authority_unavailable(),
            "workspace_submission_termination_not_confirmed",
        ),
        (
            support::workspace_submission_with_incomplete_capture(),
            "workspace_submission_incomplete",
        ),
    ] {
        let verdict = goalcup_verifier::verify_trace(trace.path()).unwrap();
        let v13 = verdict
            .checks
            .iter()
            .find(|check| check.id == "V13")
            .expect("V13 is present");
        assert_eq!(v13.verdict, Severity::Fail, "{:?}", v13.findings);
        assert!(
            v13.findings.iter().any(|finding| {
                finding.code == expected_code && finding.severity == Severity::Fail
            }),
            "{expected_code}: {:?}",
            v13.findings
        );
        assert!(
            !v13.findings
                .iter()
                .any(|finding| finding.code == "binding_checked"),
            "a refused workspace submission cannot be summarised as bound: {:?}",
            v13.findings
        );
    }
}

/// Losing authority over the harness process is an unresolved termination, even when the
/// retained artifact itself is complete and independently readable. That foreign-writer
/// condition must remain a deterministic failure and must never receive the binding summary.
#[test]
fn process_authority_unavailable_workspace_finalization_is_never_clean() {
    let trace = support::workspace_submission_with_process_authority_unavailable();
    let first = goalcup_verifier::verify_trace(trace.path()).unwrap();
    let second = goalcup_verifier::verify_trace(trace.path()).unwrap();

    assert_eq!(
        first, second,
        "the same retained trace has one stable verdict"
    );
    assert_ne!(first.overall, Severity::Pass);

    let v13 = first
        .checks
        .iter()
        .find(|check| check.id == "V13")
        .expect("V13 is present");
    assert_eq!(v13.verdict, Severity::Fail, "{:?}", v13.findings);
    let terminal_findings: Vec<(&str, Severity)> = v13
        .findings
        .iter()
        .filter(|finding| {
            finding.code == "workspace_submission_termination_not_confirmed"
                || finding.code == "binding_checked"
        })
        .map(|finding| (finding.code.as_str(), finding.severity))
        .collect();
    assert_eq!(
        terminal_findings,
        vec![(
            "workspace_submission_termination_not_confirmed",
            Severity::Fail,
        )]
    );
}

/// `AlreadyStopped` is historical terminal data, not a V1 clean termination. The
/// captured-artifact refusal must not be the only reason this trace fails.
#[test]
fn already_stopped_workspace_submission_requires_confirmed_termination() {
    let trace = support::workspace_submission_with_already_stopped_termination();
    let verdict = goalcup_verifier::verify_trace(trace.path()).unwrap();
    let v13 = verdict
        .checks
        .iter()
        .find(|check| check.id == "V13")
        .expect("V13 is present");

    assert_eq!(v13.verdict, Severity::Fail, "{:?}", v13.findings);
    assert!(
        v13.findings.iter().any(|finding| {
            finding.code == "workspace_submission_termination_not_confirmed"
                && finding.severity == Severity::Fail
        }),
        "AlreadyStopped must retain a permanent V1 refusal: {:?}",
        v13.findings
    );
    assert!(
        !v13.findings
            .iter()
            .any(|finding| finding.code == "binding_checked"),
        "a non-confirmed termination cannot be summarised as bound: {:?}",
        v13.findings
    );
}

#[test]
fn captured_workspace_submission_is_bound_to_the_actual_receipt_and_terminal_cutoff() {
    let trace = support::workspace_submission_with_captured_manifest();
    let verdict = goalcup_verifier::verify_trace(trace.path()).unwrap();
    let v13 = verdict
        .checks
        .iter()
        .find(|check| check.id == "V13")
        .unwrap();

    for code in [
        "submission_invalid",
        "submission_binding_mismatch",
        "submission_cutoff_unbound",
        "submission_deadline_mismatch",
        "submission_finish_reason_mismatch",
        "submission_exclusions_mismatch",
        "submission_finalization_not_terminal",
        "submission_post_cutoff_evidence",
    ] {
        assert!(
            !v13.findings.iter().any(|finding| finding.code == code),
            "{code}: {:?}",
            v13.findings
        );
    }
    assert!(
        v13.findings
            .iter()
            .any(|finding| finding.code == "binding_checked"),
        "a valid captured submission retains the binding summary: {:?}",
        v13.findings
    );
}

#[test]
fn captured_workspace_submission_reconciles_every_signed_frame_claim() {
    for (trace, expected_code) in [
        (
            support::workspace_submission_with_wrong_receipt_digest(),
            "submission_binding_mismatch",
        ),
        (
            support::workspace_submission_with_wrong_deadline(),
            "submission_deadline_mismatch",
        ),
        (
            support::workspace_submission_with_wrong_finish_reason(),
            "submission_finish_reason_mismatch",
        ),
        (
            support::workspace_submission_with_wrong_exclusions(),
            "submission_exclusions_mismatch",
        ),
    ] {
        let verdict = goalcup_verifier::verify_trace(trace.path()).unwrap();
        let v13 = verdict
            .checks
            .iter()
            .find(|check| check.id == "V13")
            .unwrap();
        assert!(
            v13.findings.iter().any(|finding| {
                finding.code == expected_code && finding.severity == Severity::Fail
            }),
            "{expected_code}: {:?}",
            v13.findings
        );
        assert!(
            !v13.findings
                .iter()
                .any(|finding| finding.code == "binding_checked"),
            "a failed submission binding cannot also be summarized as bound: {:?}",
            v13.findings
        );
    }
}

#[test]
fn captured_workspace_submission_rejects_every_cross_phase_chronology_edge() {
    for trace in [
        support::workspace_submission_started_after_cutoff(),
        support::workspace_submission_termination_requested_before_cutoff(),
        support::workspace_submission_copy_started_before_confirmation(),
        support::workspace_submission_readback_after_finalization(),
        support::workspace_submission_with_unparseable_start(),
    ] {
        let verdict = goalcup_verifier::verify_trace(trace.path()).unwrap();
        let v13 = verdict
            .checks
            .iter()
            .find(|check| check.id == "V13")
            .unwrap();
        assert!(
            v13.findings.iter().any(|finding| {
                finding.code == "submission_chronology_invalid"
                    && finding.severity == Severity::Fail
            }),
            "impossible chronology was accepted: {:?}",
            v13.findings
        );
        assert!(!v13.findings.iter().any(|f| f.code == "binding_checked"));
    }
}

#[test]
fn workspace_finalization_is_unique_terminal_and_admits_no_later_work() {
    let trace = support::workspace_submission_with_post_finalization_work();
    let verdict = goalcup_verifier::verify_trace(trace.path()).unwrap();
    let v13 = verdict
        .checks
        .iter()
        .find(|check| check.id == "V13")
        .unwrap();

    for code in [
        "submission_finalization_not_terminal",
        "submission_post_cutoff_evidence",
    ] {
        assert!(
            v13.findings
                .iter()
                .any(|finding| finding.code == code && finding.severity == Severity::Fail),
            "{code}: {:?}",
            v13.findings
        );
    }
    assert!(
        !v13.findings
            .iter()
            .any(|finding| finding.code == "binding_checked")
    );
}

#[test]
fn retained_v4_entries_use_the_existing_attribution_pipeline() {
    let trace = support::workspace_submission_with_unexplained_file();
    let verdict = goalcup_verifier::verify_trace(trace.path()).unwrap();

    // D35: the finding is a Flag, not a Fail. What this test pins is that a retained
    // V4 entry reaches the same attribution pipeline a legacy seal does, so the severity
    // moves with that pipeline's ruling and the code does not.
    assert!(
        verdict.review_queue.iter().any(|finding| {
            finding.code == "unexplained_seal_file" && finding.severity == Severity::Flag
        }),
        "the retained file must be attributed exactly like a legacy seal: {:?}",
        verdict.review_queue
    );
    assert!(
        verdict
            .seal_attribution
            .iter()
            .any(|entry| entry.path == "unexplained.txt")
    );
}

#[test]
fn retained_v4_entries_cannot_be_overridden_by_a_conflicting_legacy_seal() {
    let trace = support::workspace_submission_with_conflicting_legacy_seal();
    let verdict = goalcup_verifier::verify_trace(trace.path()).unwrap();

    assert!(
        verdict.seal_attribution.iter().any(|entry| {
            entry.path == "unexplained.txt" && entry.attribution == Attribution::Unexplained
        }),
        "{:?}",
        (verdict.seal_attribution, verdict.review_queue)
    );
    assert!(
        verdict
            .review_queue
            .iter()
            .any(|finding| finding.code == "unexplained_seal_file")
    );
}

/// The two edges of the window, which are not the same edge.
///
/// The fixture plants one record an hour below `started_at` and one a minute above the
/// seal. They are two statements at two severities: the late one is the Fail, and the
/// early one is a Flag, because a record below the start sits above a baseline the server
/// witnessed at seq 0 and can therefore carry no work — see the module header of `v13_binding`.
#[test]
fn work_after_the_close_fails_and_work_before_the_start_is_only_flagged() {
    let t = support::compliant_trace_with_out_of_window_records();
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();

    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "outside_window")
        .expect("outside_window");
    assert_eq!(f.severity, Severity::Fail);
    // One, not two: the record below the start is not counted here.
    assert!(f.message.contains("1 record"), "{}", f.message);
    assert_eq!(f.evidence.len(), 1, "one offence is its own first and last");
    assert!(f.evidence[0].note.contains("after"), "{:?}", f.evidence[0]);

    let e = v
        .review_queue
        .iter()
        .find(|f| f.code == "pre_start_records_early")
        .expect("pre_start_records_early");
    assert_eq!(e.severity, Severity::Flag);
    assert!(e.message.contains("1 record"), "{}", e.message);
    // The earliest clock is named, so a reviewer knows how far below the start it sits
    // without opening the chain.
    assert!(e.message.contains("2026-09-04T09:00:00Z"), "{}", e.message);
    assert_eq!(e.evidence.len(), 1);
    assert!(e.evidence[0].note.contains("before"), "{:?}", e.evidence[0]);

    // An hour below the start is not the arming window, so the Pass-level count is silent:
    // the two buckets are disjoint and a record is in exactly one of them.
    assert!(
        !v.checks
            .iter()
            .any(|c| c.findings.iter().any(|f| f.code == "pre_start_records"))
    );
}

/// The honest run.
///
/// A `claude-code-cli` run's arming signal is `launch+first_transcript_record`: the records
/// that cause the start receipt to be requested are chained before the request goes out,
/// and `started_at` is the server's clock at whole-second granularity. Failing on that would
/// decide an honest run by which side of a second boundary one HTTP round trip landed on.
/// It is a Pass finding that says the arming phase happened and counts it.
#[test]
fn records_in_the_arming_window_are_a_pass_level_count_and_not_a_failure() {
    let t = support::compliant_trace_with_arming_records();
    let v = goalcup_verifier::verify_trace_with_options(
        t.path(),
        goalcup_verifier::VerifyOptions {
            allow_descriptor_key: false,
            extra_server_keys: Vec::new(),
            dev_mode: true,
        },
    )
    .unwrap();

    let v13 = v.checks.iter().find(|c| c.id == "V13").unwrap();
    assert_eq!(
        v13.verdict,
        Severity::Pass,
        "an honest arming phase is not a finding against the run: {:?}",
        v13.findings
    );
    let f = v13
        .findings
        .iter()
        .find(|f| f.code == "pre_start_records")
        .expect("pre_start_records");
    assert_eq!(f.severity, Severity::Pass);
    assert!(f.message.contains("2 record"), "{}", f.message);
    assert_eq!(f.evidence.len(), 2, "first and last are cited");
    assert!(f.evidence[0].seq.unwrap() < f.evidence[1].seq.unwrap());
    // The summary still stands: an arming phase refuses no binding.
    assert!(v13.findings.iter().any(|f| f.code == "binding_checked"));
    // And V13 put nothing in the queue: every finding it raised is pass-level. (The
    // synthetic fixture carries unrelated V3 pairing flags; the overall-Pass claim on an
    // honest arming phase is asserted on a real capture in the devserver round trip.)
    assert!(
        v13.findings.iter().all(|f| f.severity == Severity::Pass),
        "V13 raised above Pass on an honest arming phase: {:?}",
        v13.findings
            .iter()
            .map(|f| (&f.code, f.severity))
            .collect::<Vec<_>>()
    );
    for code in ["outside_window", "pre_start_records_early"] {
        assert!(
            !v.review_queue.iter().any(|f| f.code == code),
            "{code} fired on an honest arming phase"
        );
    }
}

/// Ten minutes below the start is further than the arming sequence explains — and it is
/// still not a Fail. The baseline the receipt witnessed is seq 0, below every one of these
/// records, so what an early clock can carry is a clock nobody set and not work nobody
/// watched. The Flag says exactly that, and names the earliest timestamp.
#[test]
fn a_record_ten_minutes_below_the_start_is_flagged_and_never_failed() {
    let t = support::compliant_trace_ten_minutes_early();
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();

    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "pre_start_records_early")
        .expect("pre_start_records_early");
    assert_eq!(f.severity, Severity::Flag);
    assert!(f.message.contains("1 record"), "{}", f.message);
    assert!(
        f.message.contains("2026-09-04T09:50:00Z"),
        "the earliest timestamp is named: {}",
        f.message
    );
    // Not a Fail anywhere: the only thing wrong with this trace is that one clock, and
    // the check must not report it as work outside the run.
    assert!(!v.review_queue.iter().any(|f| f.code == "outside_window"));
    let v13 = v.checks.iter().find(|c| c.id == "V13").unwrap();
    assert!(
        !v13.findings.iter().any(|f| f.severity == Severity::Fail),
        "{:?}",
        v13.findings
    );
}

/// The upper edge: a record above the run's close is a Fail.
///
/// Asserted against both upper bounds, because the fallback is the one a participant would
/// arrange around: a run that seals is dated by the seal, and one that never seals is
/// dated by the deadline it still declared.
#[test]
fn a_record_after_the_close_fails_against_the_seal_and_against_the_deadline() {
    // Against the seal: 11:30 is above `SEALED_AT`.
    let sealed = support::compliant_trace_with_a_record_after_the_deadline();
    let v = goalcup_verifier::verify_trace(sealed.path()).unwrap();
    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "outside_window")
        .expect("outside_window");
    assert_eq!(f.severity, Severity::Fail);
    assert!(f.message.contains("the seal"), "{}", f.message);

    // And against the declared deadline, on a run that never sealed at all.
    let t = support::compliant_trace_with_a_record_after_the_deadline();
    let mut doc = support::read_descriptor(&t);
    doc.seal = None;
    support::write_descriptor(&t, &doc);
    std::fs::remove_file(t.path().join(goalcup_protocol::export::SEAL_FILE)).unwrap();

    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "outside_window")
        .expect("outside_window");
    assert_eq!(f.severity, Severity::Fail);
    assert!(f.message.contains("1 record"), "{}", f.message);
    assert!(f.message.contains("the declared deadline"), "{}", f.message);
    assert_eq!(f.evidence.len(), 1, "one offence is its own first and last");
    assert!(f.evidence[0].note.contains("after"), "{:?}", f.evidence[0]);
    // And a seal the descriptor no longer names is not held against the chain.
    assert!(!v.review_queue.iter().any(|f| f.code == "seal_not_chained"));
}

#[test]
fn a_seal_that_is_not_in_the_chain_fails_and_so_does_one_the_chain_disagrees_with() {
    // `seal.json` alone is a file at a participant-named path: until the seal
    // is a link, an acknowledgement over the head anchors whatever record happened to be
    // last, and the seal is not covered by anything.
    let t = support::compliant_trace_with_unchained_seal();
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "seal_not_chained")
        .expect("seal_not_chained");
    assert_eq!(f.severity, Severity::Fail);
    assert!(f.message.contains("no seal"), "{}", f.message);

    // And an edited seal: the file is chained, but not this file.
    let t = support::compliant_trace();
    let seal = t.path().join(goalcup_protocol::export::SEAL_FILE);
    let mut m: goalcup_protocol::seal::SealManifest =
        serde_json::from_slice(&std::fs::read(&seal).unwrap()).unwrap();
    m.excluded.push("secrets.env".to_string());
    std::fs::write(&seal, serde_json::to_vec(&m).unwrap()).unwrap();
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "seal_not_chained")
        .expect("seal_not_chained");
    assert!(
        f.message.contains("sha256"),
        "the message names which of the three: {}",
        f.message
    );

    // A legacy M0 trace has no chain and therefore no seal link, and is not punished.
    for id in ["claude-code-cli", "codex-cli"] {
        let v = goalcup_verifier::verify_trace(
            fixture_support::materialize_fixture(&format!("tests/traces/{id}")).path(),
        )
        .unwrap();
        assert!(
            !v.review_queue.iter().any(|f| f.code == "seal_not_chained"),
            "{id}"
        );
    }
}

#[test]
fn a_chain_head_the_receipt_did_not_witness_fails() {
    let t = support::compliant_trace_with_rewritten_genesis();
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    assert!(
        v.review_queue
            .iter()
            .any(|f| f.code == "chain_head_mismatch" && f.severity == Severity::Fail)
    );
    // And the rewrite also breaks the links, so both statements are made.
    assert!(v.review_queue.iter().any(|f| f.code == "chain_broken"));
}

/// The baseline is the other half of the binding.
///
/// `baseline.json` is written by the participant's own daemon. A participant who
/// pre-populated the workspace could list every planted file's real path and digest in it
/// and have `attribute_seal` exempt each one as `Attribution::Baseline` — the baseline enforced by a
/// document its own subject wrote. A trace that declares no baseline at all says nothing
/// about t0, and says it out loud.
#[test]
fn a_chain_trace_with_no_baseline_is_baseline_missing_unless_it_is_a_legacy_m0_fixture() {
    let t = support::compliant_trace_without_a_baseline();
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "baseline_missing")
        .expect("baseline_missing");
    assert_eq!(f.severity, Severity::Fail);
    // Refused bindings are never summarised as bound.
    let v13 = v.checks.iter().find(|c| c.id == "V13").unwrap();
    assert!(
        !v13.findings.iter().any(|f| f.code == "binding_checked"),
        "{:?}",
        v13.findings
    );
    assert!(
        !v13.findings
            .iter()
            .any(|f| f.code == "baseline_not_chained")
    );

    // The three committed M0 descriptors declare [[leg1]] and no chain: there is nothing
    // a baseline could have been chained into, so they are exempt exactly as they are
    // from `receipt_missing`.
    for id in ["claude-code-cli", "codex-cli"] {
        let v = goalcup_verifier::verify_trace(
            fixture_support::materialize_fixture(&format!("tests/traces/{id}")).path(),
        )
        .unwrap();
        assert!(
            !v.review_queue.iter().any(|f| f.code == "baseline_missing"),
            "{id}"
        );
    }

    // And a compliant trace, whose baseline is seq 0, says neither.
    let ok = goalcup_verifier::verify_trace(support::compliant_trace().path()).unwrap();
    assert!(
        !ok.review_queue
            .iter()
            .any(|f| { f.code == "baseline_missing" || f.code == "baseline_not_chained" })
    );
}

/// `baseline_not_chained`, its three causes said apart — the same shape `seal_not_chained`
/// takes, because it is the same threat at the other end of the run.
#[test]
fn a_baseline_the_chain_does_not_cover_fails_and_says_which_of_the_three_it_is() {
    // (a) The file is there and seq 0 is not a baseline link: nothing covers the list.
    let t = support::compliant_trace_with_an_unchained_baseline();
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "baseline_not_chained")
        .expect("baseline_not_chained");
    assert_eq!(f.severity, Severity::Fail);
    assert!(f.message.contains("first link"), "{}", f.message);
    assert!(!f.message.contains("sha256"), "{}", f.message);

    // (b) The chain carries a baseline, and it is not this one. Editing the file after
    // the fact is exactly what a participant adding a planted file to it would do.
    let t = support::compliant_trace();
    let path = t.path().join(goalcup_protocol::export::BASELINE_FILE);
    let mut m: goalcup_protocol::seal::SealManifest =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    m.excluded.push("secrets.env".to_string());
    std::fs::write(&path, serde_json::to_vec(&m).unwrap()).unwrap();
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "baseline_not_chained")
        .expect("baseline_not_chained");
    assert!(
        f.message.contains("sha256"),
        "the message names which of the three: {}",
        f.message
    );

    // (c) The chain claims a baseline the participant did not hand over.
    let t = support::compliant_trace();
    std::fs::remove_file(t.path().join(goalcup_protocol::export::BASELINE_FILE)).unwrap();
    let mut doc = support::read_descriptor(&t);
    doc.baseline = None;
    support::write_descriptor(&t, &doc);
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "baseline_not_chained")
        .expect("baseline_not_chained");
    assert!(f.message.contains("did not hand over"), "{}", f.message);
    // The sharper statement, not the vaguer one: the chain does say what t0 held.
    assert!(!v.review_queue.iter().any(|f| f.code == "baseline_missing"));
}

/// A baseline the chain does not cover exempts nothing.
///
/// This is the whole point of the link, and it is asserted through `verify_trace` rather
/// than against `attribute_seal` directly, because what is being pinned is the wiring:
/// the verifier consults the baseline only where V13 would not refuse it.
#[test]
fn a_baseline_the_chain_disagrees_with_exempts_no_sealed_file() {
    use goalcup_verifier::attribution::Attribution;

    // Bound: the file the baseline held is the file the seal holds, and it owes nothing.
    let t = support::compliant_trace_with_a_baseline_file();
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let a = v
        .seal_attribution
        .iter()
        .find(|a| a.path == support::KEPT_FILE)
        .expect("the seal lists the file");
    assert!(
        matches!(a.attribution, Attribution::Baseline { .. }),
        "{:?}",
        a.attribution
    );
    assert!(
        !v.review_queue
            .iter()
            .any(|f| f.code == "unexplained_seal_file")
    );

    // Now edit the baseline the chain covered. The file is still listed in it, and it
    // exempts nothing: the digest the chain claims is not this file's any more.
    let path = t.path().join(goalcup_protocol::export::BASELINE_FILE);
    let mut m: goalcup_protocol::seal::SealManifest =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    m.excluded.push("secrets.env".to_string());
    assert!(m.files.iter().any(|f| f.path == support::KEPT_FILE));
    std::fs::write(&path, serde_json::to_vec(&m).unwrap()).unwrap();

    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let a = v
        .seal_attribution
        .iter()
        .find(|a| a.path == support::KEPT_FILE)
        .expect("the seal lists the file");
    assert_eq!(a.attribution, Attribution::Unexplained);
    assert!(
        v.review_queue
            .iter()
            .any(|f| f.code == "unexplained_seal_file")
    );

    // And the same for a baseline no link ever covered: the list is the participant's
    // own, so the file it claims to have held is owed an explanation like any other.
    let t = support::compliant_trace_with_an_unchained_baseline_file();
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let a = v
        .seal_attribution
        .iter()
        .find(|a| a.path == support::KEPT_FILE)
        .expect("the seal lists the file");
    assert_eq!(a.attribution, Attribution::Unexplained);
    assert!(
        v.review_queue
            .iter()
            .any(|f| f.code == "baseline_not_chained")
    );
}

/// `due_entries` seals when the daemon next notices the deadline has passed, so a daemon
/// that was down at that second seals at its restart. The window's upper edge is the
/// earlier of `sealed_at` and the deadline: taken from `sealed_at` alone, everything the
/// agent wrote in between would be inside the window, and `outside_window` could not fire
/// on any of it: "168 hours, the clock never pauses" would rest on one process being alive
/// at one second.
///
/// The late seal gets a sentence of its own so the reviewer reads why rather than
/// inferring it from a list of sequences.
#[test]
fn a_seal_after_the_deadline_does_not_extend_the_window() {
    let t = support::compliant_trace_sealed_late();
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let v13 = v.checks.iter().find(|c| c.id == "V13").unwrap();

    let late = v13
        .findings
        .iter()
        .find(|f| f.code == "seal_after_deadline")
        .unwrap_or_else(|| panic!("seal_after_deadline: {:?}", v13.findings));
    assert_eq!(late.severity, Severity::Flag);
    assert!(
        late.message.contains("3600s after its own deadline"),
        "{}",
        late.message
    );

    // The record at 11:30 sits below the 12:00 seal and above the 11:00 deadline. It is
    // the deadline that binds.
    let out = v13
        .findings
        .iter()
        .find(|f| f.code == "outside_window")
        .unwrap_or_else(|| panic!("outside_window: {:?}", v13.findings));
    assert_eq!(out.severity, Severity::Fail);
    assert!(
        out.message.contains("the declared deadline") && out.message.contains(support::DEADLINE_AT),
        "the message names the bound it enforced: {}",
        out.message
    );
    assert_eq!(v13.verdict, Severity::Fail);

    // And a seal at or before the deadline still binds, so the ordinary case is untouched:
    // the same record thirty minutes past the deadline, with no seal moved, is the fixture
    // that already existed.
    let on_time = goalcup_verifier::verify_trace(support::compliant_trace().path()).unwrap();
    let v13 = on_time.checks.iter().find(|c| c.id == "V13").unwrap();
    assert!(
        !v13.findings
            .iter()
            .any(|f| f.code == "seal_after_deadline" || f.code == "outside_window"),
        "{:?}",
        v13.findings
    );
}

// The tree-wide roster of finding codes: V13's own half lives beside
// the shared fixture pipeline, where `end_to_end.rs` reads it too so an expectation cannot
// forbid a code no build can emit.
use fixture_support::pipeline::V13_CODES;

#[test]
fn every_code_v13_emits_is_on_the_roster_and_every_code_on_it_is_emitted() {
    use std::collections::BTreeSet;

    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut sweep = |dir: &std::path::Path| {
        let v = goalcup_verifier::verify_trace(dir).unwrap();
        let v13 = v.checks.iter().find(|c| c.id == "V13").unwrap();
        seen.extend(v13.findings.iter().map(|f| f.code.clone()));
    };

    // One trace per branch. Every one of these is asserted in its own test above; this
    // sweep exists to close the roster, not to re-check the behaviour.
    sweep(fixture_support::materialize_fixture("tests/traces/claude-code-cli").path());
    sweep(support::compliant_trace().path());
    sweep(support::compliant_trace_without_receipt().path());
    sweep(support::compliant_trace_signed_by(&[7u8; 32]).path());
    sweep(support::compliant_trace_with_out_of_window_records().path());
    sweep(support::compliant_trace_with_arming_records().path());
    sweep(support::compliant_trace_ten_minutes_early().path());
    sweep(support::compliant_trace_with_rewritten_genesis().path());
    sweep(support::compliant_trace_with_unchained_seal().path());
    sweep(support::compliant_trace_sealed_at(support::UNREADABLE_SEALED_AT).path());
    sweep(support::compliant_trace_sealed_late().path());
    sweep(support::compliant_trace_with_an_unreadable_clock().path());
    sweep(support::compliant_trace_without_a_baseline().path());
    sweep(support::compliant_trace_with_an_unchained_baseline().path());
    // The connector directory, in the two readings that raise a finding. The third — a
    // pinned directory — is the default every other trace above is built with, and it is
    // silent by design.
    sweep(support::compliant_trace_with_an_unpinned_connector_dir().path());
    sweep(support::compliant_trace_without_a_connector_dir_claim().path());

    let unsupported = support::compliant_trace();
    support::bump_protocol_version(&unsupported, 2, 99);
    sweep(unsupported.path());

    let partial = support::compliant_trace();
    support::append_unterminated_bytes(&partial);
    sweep(partial.path());

    let incompatible = support::compliant_trace();
    support::append_v2_legacy_redaction_and_valid_suffix(&incompatible);
    sweep(incompatible.path());

    sweep(trace_with_invalid_redaction_audit().path());
    sweep(trace_with_unpaired_legacy_marker().path());

    // A receipt nobody can check, and a frame that disagrees with one.
    let unverified = support::compliant_trace();
    let mut doc = support::read_descriptor(&unverified);
    doc.server_public_key = None;
    support::write_descriptor(&unverified, &doc);
    sweep(unverified.path());

    let mismatched = support::compliant_trace();
    let mut doc = support::read_descriptor(&mismatched);
    doc.workspace = "/some/other/ws".to_string();
    support::write_descriptor(&mismatched, &doc);
    sweep(mismatched.path());

    // A manifest edited after it was copied in: the rules the run was judged by are not
    // the rules this build ships.
    let edited = support::compliant_trace();
    support::edit_the_manifest(&edited);
    sweep(edited.path());

    // `server_key_unpinned` is the one code no default run can reach: a key this build does
    // not pin resolves to `KeySource::None` and verifies nothing, so the flag needs one of
    // the two development escape hatches to fire at all. That is the whole reason it
    // exists, and the sweep says so by taking the same options a person would type.
    let borrowed = support::compliant_trace_signed_and_named_by(&[7u8; 32]);
    let v = goalcup_verifier::verify_trace_with_options(
        borrowed.path(),
        goalcup_verifier::VerifyOptions {
            allow_descriptor_key: true,
            extra_server_keys: Vec::new(),
            dev_mode: false,
        },
    )
    .unwrap();
    let v13 = v.checks.iter().find(|c| c.id == "V13").unwrap();
    seen.extend(v13.findings.iter().map(|f| f.code.clone()));

    let roster: BTreeSet<String> = V13_CODES.iter().map(|s| (*s).to_string()).collect();
    assert_eq!(roster.len(), V13_CODES.len(), "the roster has no duplicate");
    assert_eq!(
        seen, roster,
        "left: what V13 emitted, right: V13_CODES. Add a new code to the roster, or drop \
         one nothing emits"
    );
}

#[test]
fn all_checks_returns_thirteen_and_v13_runs_last() {
    let ids: Vec<&str> = goalcup_verifier::checks::all_checks()
        .iter()
        .map(|c| c.id())
        .collect();
    assert_eq!(ids.len(), 13);
    assert_eq!(ids.last(), Some(&"V13"));
}

/// A trace whose every record and post sits inside the window raises nothing, and
/// `out_of_window` says so
/// directly rather than only through the finding it feeds.
#[test]
fn a_compliant_trace_has_nothing_outside_its_window() {
    use goalcup_protocol::control::parse_z;
    use goalcup_verifier::checks::v13_binding::out_of_window;

    let t = support::compliant_trace();
    let chain = goalcup_verifier::chain::load_and_verify_text(
        &t.path().join("chain.jsonl").display().to_string(),
        &std::fs::read_to_string(t.path().join("chain.jsonl")).unwrap(),
    )
    .unwrap();
    let started = parse_z(support::STARTED_AT).unwrap();
    let sealed = parse_z(support::SEALED_AT).unwrap();
    assert_eq!(out_of_window(&chain, sealed), Vec::new());
    // And the lower edge is empty too — which is a separate scan, so
    // "nothing outside the window" has to be said about both halves or it says half of it.
    assert_eq!(
        goalcup_verifier::checks::v13_binding::pre_start(&chain, started),
        Vec::new()
    );

    // `--dev`, because the fixtures sign with the published placeholder seed and V13 says
    // so: `placeholder_server_key` is a statement about the key, not about the window, and
    // suppressing it is what leaves this assertion measuring the thing it names.
    let v = goalcup_verifier::verify_trace_with_options(
        t.path(),
        goalcup_verifier::VerifyOptions {
            allow_descriptor_key: false,
            extra_server_keys: Vec::new(),
            dev_mode: true,
        },
    )
    .unwrap();
    let v13 = v.checks.iter().find(|c| c.id == "V13").unwrap();
    assert_eq!(v13.verdict, Severity::Pass, "{:?}", v13.findings);
    assert!(v13.findings.iter().any(|f| f.code == "binding_checked"));
}

/// The other two halves of the substituted frame. `connector` is compared against the
/// receipt's `connector_id` rather than against the manifest — `verify_trace` already
/// refuses a descriptor whose connector disagrees with the manifest it points at, so the
/// only way the two can part company is a receipt that names a third thing.
#[test]
fn the_entry_and_the_connector_are_compared_too() {
    let t = support::compliant_trace();
    let mut doc = support::read_descriptor(&t);
    doc.entry_id = "some-other-entry".to_string();
    support::write_descriptor(&t, &doc);
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let m = &v
        .review_queue
        .iter()
        .find(|f| f.code == "descriptor_mismatch")
        .expect("descriptor_mismatch")
        .message;
    assert!(m.contains("entry_id"), "{m}");
    assert!(!m.contains("some-other-entry"), "the value stays out: {m}");

    let t = support::compliant_trace();
    let path = t.path().join(goalcup_protocol::export::RECEIPT_FILE);
    let mut r: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    r["claims"]["connector_id"] = serde_json::json!("codex-cli");
    std::fs::write(&path, serde_json::to_vec(&r).unwrap()).unwrap();
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let m = &v
        .review_queue
        .iter()
        .find(|f| f.code == "descriptor_mismatch")
        .expect("descriptor_mismatch")
        .message;
    assert!(m.contains("connector"), "{m}");
}

/// The claims are compared whether or not the signature verified, and the two are said
/// apart: "somebody else signed it" and "a claim was edited after it was signed" are
/// different accusations, and a reviewer who read only the first would stop too early.
#[test]
fn an_edited_claim_is_receipt_invalid_and_the_field_it_moved_is_named() {
    let t = support::compliant_trace();
    let path = t.path().join(goalcup_protocol::export::RECEIPT_FILE);
    let mut r: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    r["claims"]["manifest_sha256"] = serde_json::json!("00".repeat(32));
    std::fs::write(&path, serde_json::to_vec(&r).unwrap()).unwrap();

    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let invalid = v
        .review_queue
        .iter()
        .find(|f| f.code == "receipt_invalid")
        .expect("receipt_invalid");
    assert!(
        invalid
            .message
            .contains("edited after the server signed it"),
        "{}",
        invalid.message
    );
    let mismatch = v
        .review_queue
        .iter()
        .find(|f| f.code == "descriptor_mismatch")
        .expect("descriptor_mismatch");
    assert!(mismatch.message.contains("sha256"), "{}", mismatch.message);
}

/// A receipt nobody in this build can check is not a binding — and it is not a forgery
/// either. This is the receipt's own analogue of V12's
/// `anchor_unverified`, a **Flag** saying the document was carried and its signature was
/// checked against nothing. `receipt_invalid` is reserved for a signature that actually
/// failed, so a reviewer reading "invalid" is reading an accusation and not an absence.
#[test]
fn a_receipt_no_recognised_key_applies_to_is_flagged_not_failed() {
    let t = support::compliant_trace();
    let mut doc = support::read_descriptor(&t);
    doc.server_public_key = None;
    support::write_descriptor(&t, &doc);

    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "receipt_unverified")
        .expect("receipt_unverified");
    assert_eq!(f.severity, Severity::Flag);
    assert!(
        f.message.contains("no recognised server key"),
        "{}",
        f.message
    );
    // The evidence does not read "signature": no signature was examined, and a note
    // saying otherwise would describe a check that did not happen.
    assert_eq!(f.evidence.len(), 1);
    assert_ne!(f.evidence[0].note, "signature");
    assert!(f.evidence[0].note.contains("key"), "{:?}", f.evidence[0]);
    // Nothing was refused, so the Fail code stays out of the verdict entirely.
    assert!(
        !v.review_queue.iter().any(|f| f.code == "receipt_invalid"),
        "an unchecked signature is not an invalid one"
    );
    // And the binding is still not summarised as checked: it was not checked.
    let v13 = v.checks.iter().find(|c| c.id == "V13").unwrap();
    assert!(
        !v13.findings.iter().any(|f| f.code == "binding_checked"),
        "{:?}",
        v13.findings
    );
}

/// An unparseable window bound is a Fail that names which edge it is, and carries at most
/// 32 characters of the participant's own string. Returning out of the binding pass at
/// that point would skip `outside_window` and the summary, and V13 would report Pass with
/// **zero findings**: a participant would only have to write a `sealed_at` nobody can read
/// and the window check would disappear without a word.
#[test]
fn an_unreadable_window_bound_fails_instead_of_taking_the_window_with_it() {
    let t = support::compliant_trace_sealed_at(support::UNREADABLE_SEALED_AT);
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let v13 = v.checks.iter().find(|c| c.id == "V13").unwrap();

    assert_eq!(v13.verdict, Severity::Fail, "{:?}", v13.findings);
    let f = v13
        .findings
        .iter()
        .find(|f| f.code == "window_unmeasurable")
        .unwrap_or_else(|| panic!("window_unmeasurable: {:?}", v13.findings));
    assert_eq!(f.severity, Severity::Fail);
    assert!(f.message.contains("sealed_at"), "{}", f.message);
    assert!(
        !f.message.contains(support::UNREADABLE_SEALED_AT),
        "the bound's text is a participant-written string: {}",
        f.message
    );
    assert!(
        f.message.contains(&support::UNREADABLE_SEALED_AT[..32]),
        "32 characters are enough to recognise it: {}",
        f.message
    );

    // The two consequences an early return would hide.
    assert!(
        !v13.findings.iter().any(|f| f.code == "binding_checked"),
        "{:?}",
        v13.findings
    );
    assert!(
        !v13.findings.is_empty(),
        "V13 always says something about a chain trace"
    );

    // The other edge, named as itself. `started_at` lives in the signed claims, so
    // editing it also invalidates the receipt — two statements, and the window's is made.
    let t = support::compliant_trace();
    let path = t.path().join(goalcup_protocol::export::RECEIPT_FILE);
    let mut r: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    r["claims"]["started_at"] = serde_json::json!("whenever we got going");
    std::fs::write(&path, serde_json::to_vec(&r).unwrap()).unwrap();

    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "window_unmeasurable")
        .expect("window_unmeasurable");
    assert!(f.message.contains("started_at"), "{}", f.message);
    assert!(!f.message.contains("sealed_at"), "{}", f.message);
    assert!(f.message.contains("lower"), "{}", f.message);
}

/// `trace.toml` supplies the tracked sessions, and V13 compares them to the receipt:
/// without that, a participant could hand over a frame naming the session the server
/// signed and chain a different one's records under it.
#[test]
fn a_session_the_descriptor_does_not_track_is_a_descriptor_mismatch() {
    let t = support::compliant_trace();
    let path = t.path().join(goalcup_protocol::export::RECEIPT_FILE);
    let mut r: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let other = "99999999-8888-7777-6666-555555555555";
    r["claims"]["session_id"] = serde_json::json!(other);
    std::fs::write(&path, serde_json::to_vec(&r).unwrap()).unwrap();

    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "descriptor_mismatch")
        .expect("descriptor_mismatch");
    assert_eq!(f.severity, Severity::Fail);
    assert!(f.message.contains("session_id"), "{}", f.message);
    assert!(
        !f.message.contains(other),
        "the value stays out: {}",
        f.message
    );
    assert!(
        f.evidence.iter().any(|e| e.note.contains("session_id")),
        "{:?}",
        f.evidence
    );
}

/// `binding_checked` is the sentence "the receipt bound this trace". A verdict never
/// carries a `descriptor_mismatch` Fail beside a Pass saying the descriptor was bound: it
/// is withheld whenever anything in this check refused the binding, including
/// `seal_not_chained`, which is raised after the binding itself was measured.
#[test]
fn a_run_whose_binding_was_refused_is_never_summarised_as_bound() {
    let has_binding_checked = |t: &tempfile::TempDir| -> bool {
        let v = goalcup_verifier::verify_trace(t.path()).unwrap();
        let v13 = v.checks.iter().find(|c| c.id == "V13").unwrap();
        v13.findings.iter().any(|f| f.code == "binding_checked")
    };

    // A compliant trace says it, once.
    let ok = support::compliant_trace();
    assert!(has_binding_checked(&ok));

    // A substituted frame does not.
    let mismatched = support::compliant_trace();
    let mut doc = support::read_descriptor(&mismatched);
    doc.workspace = "/some/other/ws".to_string();
    support::write_descriptor(&mismatched, &doc);
    assert!(!has_binding_checked(&mismatched));

    // Nor does a chain head the receipt never witnessed.
    let rewritten = support::compliant_trace_with_rewritten_genesis();
    assert!(!has_binding_checked(&rewritten));

    // And nor does a seal with records chained after it — which is decided after the
    // binding pass, so the summary cannot be pushed from inside it.
    let unsealed = support::compliant_trace();
    support::append_one_link_after_the_seal(&unsealed);
    assert!(!has_binding_checked(&unsealed));
}

/// A disputed manifest may conservatively shorten the accepted interval, but it can never
/// lengthen it. This participant-written timestamp is earlier than every chained record,
/// so the disagreement and the records outside that earliest claim are both reported.
#[test]
fn an_earlier_disputed_manifest_timestamp_conservatively_shortens_the_window() {
    let t = support::compliant_trace();
    let path = t.path().join(goalcup_protocol::export::SEAL_FILE);
    let mut m: goalcup_protocol::seal::SealManifest =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    // Half an hour before the first record. Read as the upper bound, it puts the whole
    // run after the end of its own window.
    m.sealed_at = "2026-09-04T09:30:00Z".to_string();
    std::fs::write(&path, serde_json::to_vec(&m).unwrap()).unwrap();

    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    // The disagreement itself is still stated.
    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "seal_not_chained")
        .expect("seal_not_chained");
    assert!(f.message.contains("sha256"), "{}", f.message);
    assert!(
        v.review_queue.iter().any(|f| f.code == "outside_window"),
        "the earliest disputed claim must be the conservative cutoff"
    );
    assert!(
        !v.review_queue
            .iter()
            .any(|f| f.code == "window_unmeasurable")
    );
}

#[test]
fn disagreeing_seal_timestamps_use_the_earliest_claim_as_the_cutoff() {
    let t = support::compliant_trace();
    rewrite_chain(t.path(), |envelopes| {
        let seal = envelopes
            .iter_mut()
            .find(|env| matches!(env.body, goalcup_protocol::envelope::EventBody::Seal { .. }))
            .expect("the compliant trace has a chained seal");
        // The body and manifest still claim 10:05, but the authenticated envelope claims
        // 10:00:15. The records at 10:00:20 and later must not disappear into the wider
        // body timestamp merely because the claims disagree.
        seal.recorded_at = "2026-09-04T10:00:15Z".to_string();
    });

    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let codes: Vec<&str> = v.review_queue.iter().map(|f| f.code.as_str()).collect();
    assert!(codes.contains(&"seal_not_chained"), "{codes:?}");
    assert!(codes.contains(&"outside_window"), "{codes:?}");
}

/// A record whose `recorded_at` does not parse is counted separately and flagged, not
/// silently dropped from the window scan: it is a Flag, not a Fail, because an unreadable
/// clock is a malformed document rather than proof the work happened outside the run.
#[test]
fn a_record_whose_clock_does_not_parse_is_flagged_rather_than_dropped() {
    let t = support::compliant_trace_with_an_unreadable_clock();
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "window_unplaceable")
        .expect("window_unplaceable");
    assert_eq!(f.severity, Severity::Flag);
    assert!(f.message.contains("1 record"), "{}", f.message);
    assert_eq!(f.evidence.len(), 1, "one is its own first and last");
    let seq = f.evidence[0].seq.expect("the offending seq is cited");
    assert!(f.message.contains(&seq.to_string()), "{}", f.message);
    // An unreadable clock is not an offence against the window: the record is not
    // counted as outside it, and the run is not failed for it.
    assert!(!v.review_queue.iter().any(|f| f.code == "outside_window"));
    let v13 = v.checks.iter().find(|c| c.id == "V13").unwrap();
    assert_eq!(v13.verdict, Severity::Flag, "{:?}", v13.findings);

    // A trace whose every clock reads is not flagged.
    let clean = support::compliant_trace();
    let v = goalcup_verifier::verify_trace(clean.path()).unwrap();
    assert!(
        !v.review_queue
            .iter()
            .any(|f| f.code == "window_unplaceable")
    );
}

/// The third cause of `seal_not_chained`: the seal is a link, and it is not
/// the last one. `TraceExport` refuses to append past a seal, so this can only be built
/// outside the writer — which is exactly what a participant appending after their own seal
/// would have to do.
#[test]
fn a_seal_with_links_after_it_closes_nothing() {
    let t = support::compliant_trace();
    support::append_one_link_after_the_seal(&t);

    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "seal_not_chained")
        .expect("seal_not_chained");
    assert_eq!(f.severity, Severity::Fail);
    assert!(f.message.contains("closes nothing"), "{}", f.message);
    // Not one of the other two causes: the chain is intact and the digest agrees.
    assert!(!f.message.contains("no seal"), "{}", f.message);
    assert!(!f.message.contains("sha256"), "{}", f.message);
    assert!(!v.review_queue.iter().any(|f| f.code == "chain_broken"));
}

/// The two statements V13 renders out of `chain::load_and_verify` rather than computing:
/// a version this build does not understand, and a line that did not finish being written.
#[test]
fn an_unsupported_version_fails_and_an_unfinished_write_is_only_flagged() {
    let unsupported = support::compliant_trace();
    support::bump_protocol_version(&unsupported, 2, 99);
    let v = goalcup_verifier::verify_trace(unsupported.path()).unwrap();
    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "protocol_version_unsupported")
        .expect("protocol_version_unsupported");
    assert_eq!(f.severity, Severity::Fail);
    assert!(f.message.contains("99"), "{}", f.message);
    let v13 = v.checks.iter().find(|check| check.id == "V13").unwrap();
    assert!(
        !v13.findings
            .iter()
            .any(|finding| finding.code == "binding_checked"),
        "a trace beyond an unsupported version has no physical binding summary: {:?}",
        v13.findings
    );

    // A partial final write is what the file ends with, not a break: every link below it
    // still recomputes, so it is a Flag and `chain_broken` stays silent.
    let partial = support::compliant_trace();
    support::append_unterminated_bytes(&partial);
    let v = goalcup_verifier::verify_trace(partial.path()).unwrap();
    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "truncated_tail")
        .expect("truncated_tail");
    assert_eq!(f.severity, Severity::Flag);
    assert!(!v.review_queue.iter().any(|f| f.code == "chain_broken"));
}

fn rewrite_chain(
    trace: &std::path::Path,
    mutate: impl FnOnce(&mut [goalcup_protocol::envelope::Envelope]),
) {
    let path = trace.join("chain.jsonl");
    let text = std::fs::read_to_string(&path).unwrap();
    let mut envelopes: Vec<goalcup_protocol::envelope::Envelope> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    mutate(&mut envelopes);
    let mut prev = goalcup_protocol::chain::GENESIS.to_string();
    for env in &mut envelopes {
        env.link.prev = prev.clone();
        env.link.hash = env.expected_hash(&prev).unwrap();
        prev = env.link.hash.clone();
    }
    let mut wire = envelopes
        .iter()
        .map(|env| serde_json::to_string(env).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    wire.push('\n');
    std::fs::write(path, wire).unwrap();
}

fn trace_with_invalid_redaction_audit() -> tempfile::TempDir {
    let trace = support::compliant_trace();
    rewrite_chain(trace.path(), |envelopes| {
        let env = envelopes
            .iter_mut()
            .find(|env| {
                matches!(
                    env.body,
                    goalcup_protocol::envelope::EventBody::Leg1Record { .. }
                )
            })
            .expect("the compliant trace has a leg-1 record");
        env.redaction = Some(goalcup_protocol::redaction::RedactionAudit { spans: Vec::new() });
    });
    trace
}

fn trace_with_unpaired_legacy_marker() -> tempfile::TempDir {
    let trace = support::compliant_trace();
    rewrite_chain(trace.path(), |envelopes| {
        let env = envelopes
            .iter_mut()
            .find(|env| {
                matches!(
                    env.body,
                    goalcup_protocol::envelope::EventBody::Leg1Record { .. }
                )
            })
            .expect("the compliant trace has a leg-1 record");
        env.protocol_version = goalcup_protocol::LEGACY_PROTOCOL_VERSION;
        if let goalcup_protocol::envelope::EventBody::Leg1Record { payload, .. } = &mut env.body {
            *payload = serde_json::Value::String(
                r#"{"text":"<redacted:email:0123456789abcdef>"}"#.to_string(),
            );
        }
    });
    trace
}

#[test]
fn invalid_v2_redaction_audit_fails_and_unpaired_legacy_marker_flags() {
    let invalid =
        goalcup_verifier::verify_trace(trace_with_invalid_redaction_audit().path()).unwrap();
    let finding = invalid
        .review_queue
        .iter()
        .find(|finding| finding.code == "redaction_audit_invalid")
        .expect("invalid v2 audit");
    assert_eq!(finding.severity, Severity::Fail);

    let legacy =
        goalcup_verifier::verify_trace(trace_with_unpaired_legacy_marker().path()).unwrap();
    let finding = legacy
        .review_queue
        .iter()
        .find(|finding| finding.code == "legacy_redaction_log_missing")
        .expect("legacy marker ambiguity");
    assert_eq!(finding.severity, Severity::Flag);
    assert!(finding.message.contains("missing audit evidence"));
}

/// The connector manifests this build judges a run by.
///
/// The constant cannot drift from the files it pins: every entry is recomputed here from
/// the shipped bytes, never retyped.
#[test]
fn the_pins_are_the_digests_of_the_shipped_manifests() {
    for (id, want) in goalcup_verifier::CONNECTOR_MANIFEST_SHA256S {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../connectors")
            .join(id)
            .join("manifest.toml");
        let got = goalcup_verifier::legs::sha256_hex(&std::fs::read(&p).unwrap());
        assert_eq!(&got.as_str(), want, "{id}: run scripts/pin-manifests.sh");
        assert_eq!(
            goalcup_verifier::pinned_manifest_sha256(id),
            Some(*want),
            "{id}: the lookup and the table are the same statement"
        );
    }
    assert_eq!(goalcup_verifier::CONNECTOR_MANIFEST_SHA256S.len(), 2);
    // A connector this build ships no manifest for is pinned by nothing, and the lookup
    // says so rather than returning an empty digest.
    assert_eq!(
        goalcup_verifier::pinned_manifest_sha256("no-such-cli"),
        None
    );
}

/// The repository root, from this crate's own manifest directory.
fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The manifest each shipped connector ships today still reads as the one this build pins.
///
/// Not the only place that fact is reachable, and it does not claim to be:
/// `end_to_end::a_borrowed_context_carries_what_the_verdict_carries` resolves the same
/// bytes through the same function across every shipped connector and the adversarial
/// fixture, because every one of those descriptors points `connector_manifest` at the live
/// shipped file rather than embedding a copy. What this test adds is directness — no
/// fixture, no pipeline and no trace layout between the bytes and the reading — and a
/// failure message that names the connector, both digests and the script that fixes it.
///
/// Not a second copy of `the_pins_are_the_digests_of_the_shipped_manifests` either, which
/// compares the table against the bytes. What is asserted here is the reading the verdict
/// carries — `verdict.manifest_pin == Pinned` — and `ManifestPinning::resolve` is the one
/// place that decides it, so the statement is made through that function rather than by
/// recomputing the comparison a second time beside it.
#[test]
fn shipped_connector_pins_match() {
    for (id, _) in goalcup_verifier::CONNECTOR_MANIFEST_SHA256S {
        let p = repo_root()
            .join("connectors")
            .join(id)
            .join("manifest.toml");
        let shipped = goalcup_verifier::legs::sha256_hex(
            &std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display())),
        );
        let pinning = goalcup_verifier::ManifestPinning::resolve(id, &shipped, false);
        assert_eq!(
            pinning.state,
            goalcup_verifier::ManifestPin::Pinned,
            "{id}: the shipped manifest reads as {:?} against this build's pin {:?}, so a run \
             recorded against it today would report `manifest_not_pinned`; run \
             `sh scripts/pin-manifests.sh` and commit the diff",
            pinning.state,
            pinning.pinned,
        );
    }
}

/// A directory digest is not a manifest digest, for every V1 manifest connector.
///
/// Stated as its own test because the two tables sit next to each other and are the same
/// shape: a copy-paste that pointed the directory block at `manifest.toml` would still be
/// well-formed ids and 64-character hex strings, and every other test here would pass.
fn assert_v1_manifest_pins_have_distinct_directory_pins(
    manifests: &[(&str, &str)],
    directories: &[(&str, &str)],
) {
    for (id, file_pin) in manifests {
        let dir_pin = directories
            .iter()
            .find_map(|(candidate, pin)| (*candidate == *id).then_some(*pin))
            .unwrap_or_else(|| panic!("{id} is pinned as a V1 manifest and not as a directory"));
        assert_ne!(
            *file_pin, dir_pin,
            "{id}: the directory digest covers `files/` and the manifest digest does not, so \
             the two cannot be the same string"
        );
    }
}

#[test]
fn the_two_pin_tables_are_different_digests_of_the_same_connectors() {
    assert_v1_manifest_pins_have_distinct_directory_pins(
        goalcup_verifier::CONNECTOR_MANIFEST_SHA256S,
        CONNECTOR_DIR_SHA256S,
    );
}

#[test]
fn a_fourth_directory_pin_does_not_change_v13() {
    let mut directories = CONNECTOR_DIR_SHA256S.to_vec();
    directories.push((
        "future-cli",
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    ));
    assert_v1_manifest_pins_have_distinct_directory_pins(
        goalcup_verifier::CONNECTOR_MANIFEST_SHA256S,
        &directories,
    );
    let tables = goalcup_verifier::PinTables::fixture(
        goalcup_verifier::CONNECTOR_MANIFEST_SHA256S,
        Box::leak(directories.into_boxed_slice()),
    );
    let trace = support::compliant_trace();
    let verdict = goalcup_verifier::Pipeline::load_with_pin_tables(
        trace.path(),
        goalcup_verifier::VerifyOptions::default(),
        tables,
    )
    .unwrap()
    .verdict()
    .unwrap();
    let v13 = verdict
        .checks
        .iter()
        .find(|check| check.id == "V13")
        .unwrap();
    assert_ne!(v13.verdict, Severity::Fail, "{:#?}", v13.findings);
    assert!(
        v13.findings
            .iter()
            .all(|finding| finding.code != "connector_dir_unpinned"),
        "{:#?}",
        v13.findings
    );
}

/// `scripts/pin-manifests.sh` and `connector_dir_sha256` agree, on every shipped connector.
///
/// The byte layout — `<u64 be path length> <path> <u64 be body length> <body>`, in sorted
/// path order — is written down in two places, the script's header and the Rust doc
/// comment, and implemented twice: once in `sh` with `find`, `sort`, `printf` and `cat`,
/// once in Rust. This runs the script's own `--print` mode and compares. Without it the two
/// spellings drift the first time somebody adds a nested directory under a connector, and
/// the failure surfaces as a build that refuses its own shipped connectors.
#[test]
fn the_pin_script_and_the_protocol_function_compute_the_same_digest() {
    let root = repo_root();
    let out = std::process::Command::new("sh")
        .arg("scripts/pin-manifests.sh")
        .arg("--print")
        .current_dir(&root)
        .output()
        .expect("scripts/pin-manifests.sh runs");
    assert!(
        out.status.success(),
        "pin-manifests.sh --print failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let printed = String::from_utf8(out.stdout).expect("the script prints ids and hex");
    let mut seen = 0;
    let mut manifests = 0;
    for line in printed.lines() {
        let mut fields = line.split(' ');
        let id = fields.next().expect("an id");
        let file_digest = fields.next().expect("a manifest digest");
        let dir_digest = fields.next().expect("a directory digest");
        assert_eq!(fields.next(), None, "three fields a line: {line:?}");
        if file_digest == "-" {
            assert_eq!(
                goalcup_verifier::pinned_manifest_sha256(id),
                None,
                "{id}: a schema-only connector has no legacy manifest pin"
            );
        } else {
            assert_eq!(
                goalcup_verifier::pinned_manifest_sha256(id),
                Some(file_digest),
                "{id}: the script's manifest digest is the pinned one"
            );
            manifests += 1;
        }
        assert_eq!(
            pinned_connector_dir_sha256(id),
            Some(dir_digest),
            "{id}: the script's directory digest is the pinned one"
        );
        assert_eq!(
            goalcup_protocol::connector_dir::connector_dir_sha256(
                &root.join("connectors").join(id)
            )
            .unwrap_or_else(|e| panic!("{id}: {e}")),
            dir_digest,
            "{id}: the shell and the Rust walk hash the same stream"
        );
        seen += 1;
    }
    assert_eq!(seen, 3, "one line per connector folder, got {printed:?}");
    assert_eq!(manifests, 2, "two retained V1 manifests, got {printed:?}");
}

/// Everything under `from`, copied into `to`. Regular files and directories only, which is
/// all a connector directory or a `scripts/` directory holds.
fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).expect("the destination directory");
    for entry in std::fs::read_dir(from).expect("the source directory") {
        let entry = entry.expect("a directory entry");
        let dst = to.join(entry.file_name());
        if entry.file_type().expect("the entry's kind").is_dir() {
            copy_tree(&entry.path(), &dst);
        } else {
            std::fs::copy(entry.path(), &dst).expect("a file copy");
        }
    }
}

/// **The tables are keyed by the id the manifest declares**, not by the directory's
/// basename.
///
/// Every lookup — the daemon's preflight, V13's finding, the CLI's `pin_state` — asks for a
/// pin by the declared id, because a connector id is never joined onto a path. A table keyed
/// by the basename therefore does not mis-pin a directory whose name and declared id differ:
/// it pins nothing for it, silently, and every lookup reads back `None`, which is the word
/// for an unpinned connector rather than for a mismatch. The synthetic connector below is
/// the case, and it is synthetic because every shipped connector happens to agree.
#[test]
fn the_script_keys_the_pin_table_by_the_declared_id_and_not_by_the_directory_name() {
    let tmp = tempfile::tempdir().expect("a temporary repository");
    copy_tree(&repo_root().join("scripts"), &tmp.path().join("scripts"));
    let dir = tmp
        .path()
        .join("connectors/a-directory-called-something-else");
    std::fs::create_dir_all(dir.join("files")).expect("the synthetic connector");
    std::fs::write(dir.join("manifest.toml"), b"id = \"declared-elsewhere\"\n").expect("manifest");
    std::fs::write(dir.join("files/hook.sh"), b"#!/bin/sh\nexit 0\n").expect("a template");

    let out = std::process::Command::new("sh")
        .arg("scripts/pin-manifests.sh")
        .arg("--print")
        .current_dir(tmp.path())
        .output()
        .expect("scripts/pin-manifests.sh runs");
    assert!(
        out.status.success(),
        "--print failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let printed = String::from_utf8(out.stdout).expect("ids and hex");
    let fields: Vec<&str> = printed.trim_end().split(' ').collect();
    assert_eq!(fields.len(), 3, "one connector, three fields: {printed:?}");
    assert_eq!(
        fields[0], "declared-elsewhere",
        "the key is the declared id and not `a-directory-called-something-else`"
    );
    assert_eq!(
        fields[1],
        goalcup_verifier::legs::sha256_hex(
            &std::fs::read(dir.join("manifest.toml")).expect("the manifest")
        ),
        "and the manifest digest is still that directory's manifest"
    );
    assert_eq!(
        fields[2],
        goalcup_protocol::connector_dir::connector_dir_sha256(&dir).expect("the directory hashes"),
        "and the directory digest is still that directory"
    );
}

/// **The script is idempotent, and the committed tables are what it
/// writes.**
///
/// Run over a copy of the repository rather than over the repository, so a `cargo test` can
/// never rewrite a source file. Two runs, and three claims: the first run reproduces the
/// bytes that are committed (which is what `sh scripts/pin-manifests.sh && git diff
/// --exit-code` claims in the script's own header), the second changes nothing, and the row
/// order is therefore not a function of whoever ran it — `LC_ALL=C` is exported in the
/// script so the `connectors/*/` glob and the walk's `sort` compare bytes.
///
/// Two outputs: the V1 manifest table and the directory-pin table. Both are generated data,
/// so neither is copied in first — the script has to create them.
#[test]
fn the_pin_script_writes_the_committed_tables_and_a_second_run_changes_nothing() {
    let root = repo_root();
    let tmp = tempfile::tempdir().expect("a temporary repository");
    copy_tree(&root.join("scripts"), &tmp.path().join("scripts"));
    copy_tree(&root.join("connectors"), &tmp.path().join("connectors"));
    let generated = [
        "crates/goalcup-verifier/pins/connector-manifests.txt",
        "crates/goalcup-protocol/pins/connector-dirs.txt",
    ];

    let run = || {
        let out = std::process::Command::new("sh")
            .arg("scripts/pin-manifests.sh")
            .current_dir(tmp.path())
            .output()
            .expect("scripts/pin-manifests.sh runs");
        assert!(
            out.status.success(),
            "pin-manifests.sh failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };

    run();
    for f in generated {
        assert_eq!(
            std::fs::read(tmp.path().join(f)).expect("the generated file"),
            std::fs::read(root.join(f)).expect("the committed file"),
            "{f}: the committed pins are not what the script produces — run \
             `sh scripts/pin-manifests.sh` and commit the diff"
        );
    }

    let after_one: Vec<Vec<u8>> = generated
        .iter()
        .map(|f| std::fs::read(tmp.path().join(f)).expect("the generated file"))
        .collect();
    run();
    for (f, before) in generated.iter().zip(after_one) {
        assert_eq!(
            std::fs::read(tmp.path().join(f)).expect("the generated file"),
            before,
            "{f}: a second run moved bytes, so the script is not idempotent"
        );
    }
}

/// An edited template moves the directory digest and leaves the manifest digest alone —
/// the hole this pair of tables exists to close, measured end to end.
///
/// Over a copy of a shipped connector in a tempdir, so `connectors/` is never written to.
#[test]
fn an_edited_template_moves_the_directory_digest_and_not_the_manifest_one() {
    let src = repo_root().join("connectors/codex-cli");
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let dst = tmp.path().join("codex-cli");
    std::fs::create_dir_all(dst.join("files")).expect("files/");
    std::fs::copy(src.join("manifest.toml"), dst.join("manifest.toml")).expect("the manifest");
    std::fs::copy(src.join("harness.toml"), dst.join("harness.toml")).expect("the harness file");
    for entry in std::fs::read_dir(src.join("files")).expect("the shipped files/") {
        let entry = entry.expect("a directory entry");
        std::fs::copy(entry.path(), dst.join("files").join(entry.file_name())).expect("a copy");
    }
    assert_eq!(
        goalcup_protocol::connector_dir::connector_dir_sha256(&dst).expect("the copy hashes"),
        pinned_connector_dir_sha256("codex-cli").expect("codex-cli is pinned"),
        "a byte copy of a shipped connector is the pinned connector"
    );

    let tracker = dst.join("files/config.toml");
    let mut body = std::fs::read(&tracker).expect("the hook template body");
    body.push(b'\n');
    std::fs::write(&tracker, &body).expect("plant one byte");

    assert_eq!(
        goalcup_verifier::legs::sha256_hex(
            &std::fs::read(dst.join("manifest.toml")).expect("the manifest")
        ),
        goalcup_verifier::pinned_manifest_sha256("codex-cli").expect("codex-cli is pinned"),
        "the manifest is untouched, and the manifest pin cannot see the edit"
    );
    assert_ne!(
        goalcup_protocol::connector_dir::connector_dir_sha256(&dst).expect("the copy hashes"),
        pinned_connector_dir_sha256("codex-cli").expect("codex-cli is pinned"),
        "the directory pin can, which is the whole point of it"
    );
}

/// The rules are a participant input.
#[test]
fn a_hand_written_manifest_fails_the_pin_and_a_dev_run_says_it_flagged_instead() {
    let t = support::compliant_trace();
    support::edit_the_manifest(&t);

    let strict = goalcup_verifier::verify_trace(t.path()).unwrap();
    let f = strict
        .review_queue
        .iter()
        .find(|f| f.code == "manifest_not_pinned")
        .expect("manifest_not_pinned");
    assert_eq!(f.severity, Severity::Fail);
    assert_eq!(strict.overall, Severity::Fail);
    assert_eq!(
        strict.manifest_pin,
        goalcup_verifier::ManifestPin::Altered,
        "the verdict says which reading applied, not only the queue"
    );

    let dev = goalcup_verifier::verify_trace_with_options(
        t.path(),
        goalcup_verifier::VerifyOptions {
            allow_descriptor_key: false,
            extra_server_keys: Vec::new(),
            dev_mode: true,
        },
    )
    .unwrap();
    let f = dev
        .review_queue
        .iter()
        .find(|f| f.code == "manifest_not_pinned")
        .unwrap();
    assert_eq!(f.severity, Severity::Flag);
    assert!(
        f.message.contains("--dev"),
        "a downgraded run must say so: {}",
        f.message
    );
    // `--dev` downgrades this finding and nothing else: the same trace's edited manifest
    // also parts company with the receipt's claim, and that stays a Fail.
    let m = dev
        .review_queue
        .iter()
        .find(|f| f.code == "descriptor_mismatch")
        .expect("descriptor_mismatch");
    assert_eq!(m.severity, Severity::Fail);
    assert_eq!(dev.overall, Severity::Fail);
}

/// A manifest that is the shipped one adds nothing to the verdict — no finding at any
/// severity — so the pin half of V13 on a compliant trace says nothing.
///
/// `--dev` is not a no-op over such a trace: it covers two things, and the
/// second is `placeholder_server_key`. That finding is not suppressed
/// under `--dev` — it is reported at Pass, so the sentence stays in the document — which
/// means the two runs raise exactly the same codes and differ in exactly one severity.
/// Both halves are asserted rather than assumed: a `--dev` run that quietly dropped a
/// second finding, or that moved a second severity, is what this test exists to catch.
#[test]
fn a_matching_pin_says_nothing_and_a_dev_run_over_it_moves_only_the_placeholder_flag() {
    let t = support::compliant_trace();
    let strict = goalcup_verifier::verify_trace(t.path()).unwrap();
    assert_eq!(strict.manifest_pin, goalcup_verifier::ManifestPin::Pinned);
    let v13 = strict.checks.iter().find(|c| c.id == "V13").unwrap();
    assert!(
        !v13.findings.iter().any(|f| f.code == "manifest_not_pinned"),
        "{:?}",
        v13.findings
    );

    let dev = goalcup_verifier::verify_trace_with_options(
        t.path(),
        goalcup_verifier::VerifyOptions {
            allow_descriptor_key: false,
            extra_server_keys: Vec::new(),
            dev_mode: true,
        },
    )
    .unwrap();
    let codes = |v: &goalcup_verifier::Verdict| -> Vec<(String, Severity)> {
        v.checks
            .iter()
            .flat_map(|c| c.findings.iter())
            .map(|f| (f.code.clone(), f.severity))
            .collect()
    };
    let strict_codes = codes(&strict);
    let dev_codes = codes(&dev);
    assert_eq!(
        strict_codes.iter().map(|(c, _)| c).collect::<Vec<_>>(),
        dev_codes.iter().map(|(c, _)| c).collect::<Vec<_>>(),
        "`--dev` waives a severity; it never removes a sentence from the document"
    );
    let moved: Vec<&str> = strict_codes
        .iter()
        .zip(dev_codes.iter())
        .filter(|((_, a), (_, b))| a != b)
        .map(|((c, _), _)| c.as_str())
        .collect();
    assert_eq!(
        moved,
        ["placeholder_server_key"],
        "with a pinned manifest `--dev` moves the placeholder finding and nothing else"
    );
    assert!(
        dev.dev && !strict.dev,
        "and the document itself says which run it was"
    );
    let placeholder = dev
        .checks
        .iter()
        .flat_map(|c| c.findings.iter())
        .find(|f| f.code == "placeholder_server_key")
        .expect("the waived sentence survives in `checks`");
    assert_eq!(placeholder.severity, Severity::Pass);
    assert!(placeholder.message.contains("`--dev`"), "it says why");
    assert!(
        dev.review_queue
            .iter()
            .all(|f| f.code != "placeholder_server_key"),
        "and a Pass leaves the queue"
    );

    // And the two retained committed M0 descriptors, which point at the shipped manifests, raise
    // no manifest finding — and no directory finding either: that half is read out of a
    // verified receipt, and these captures carry none.
    for id in ["claude-code-cli", "codex-cli"] {
        let v = goalcup_verifier::verify_trace(
            fixture_support::materialize_fixture(&format!("tests/traces/{id}")).path(),
        )
        .unwrap();
        assert_eq!(
            v.manifest_pin,
            goalcup_verifier::ManifestPin::Pinned,
            "{id}"
        );
        let v13 = v.checks.iter().find(|c| c.id == "V13").unwrap();
        assert!(
            !v13.findings.iter().any(|f| f.code == "manifest_not_pinned"),
            "{id}: {:?}",
            v13.findings
        );
        assert_eq!(
            v13.findings
                .iter()
                .map(|f| f.code.as_str())
                .collect::<Vec<_>>(),
            vec!["legacy_trace"],
            "{id}"
        );
    }
}

/// `--dev-key` and `--dev-descriptor-key` must not be able to produce `overall: pass`
/// with nothing in the document saying the key was not pinned: the only trace of it
/// would be the `key_source` field, which is a word in a header rather than a
/// finding a reviewer reads.
///
/// The Flag is raised whenever a receipt or an acknowledgement was actually verified
/// against a key this build does not pin, and it is never suppressed — `--dev` downgrades
/// the manifest pin and this is not that. A verdict that says a start was witnessed has to
/// say who witnessed it.
#[test]
fn a_key_the_build_does_not_pin_is_flagged_and_dev_does_not_suppress_it() {
    let seed = [7u8; 32];
    let t = support::compliant_trace_signed_and_named_by(&seed);

    let dev = goalcup_verifier::verify_trace_with_options(
        t.path(),
        goalcup_verifier::VerifyOptions {
            allow_descriptor_key: true,
            extra_server_keys: Vec::new(),
            dev_mode: false,
        },
    )
    .unwrap();
    assert_eq!(dev.key_source, goalcup_verifier::KeySource::Descriptor);
    let f = dev
        .review_queue
        .iter()
        .find(|f| f.code == "server_key_unpinned")
        .unwrap_or_else(|| panic!("server_key_unpinned: {:?}", dev.review_queue));
    assert_eq!(f.severity, Severity::Flag);
    // The evidence carries the key source word, which is the whole point: a reader learns
    // where the trust came from, not merely that it came from somewhere.
    assert!(
        f.evidence.iter().any(|e| e.note.contains("descriptor")),
        "{:?}",
        f.evidence
    );
    // The trace is otherwise compliant, so the Flag is the whole verdict.
    assert_eq!(dev.overall, Severity::Flag);
    assert_eq!(dev.exit_code(), 1);

    // Never suppressed, not even by `--dev`.
    let both = goalcup_verifier::verify_trace_with_options(
        t.path(),
        goalcup_verifier::VerifyOptions {
            allow_descriptor_key: true,
            extra_server_keys: Vec::new(),
            dev_mode: true,
        },
    )
    .unwrap();
    assert!(
        both.review_queue
            .iter()
            .any(|f| f.code == "server_key_unpinned"),
        "{:?}",
        both.review_queue
    );
    assert_eq!(both.exit_code(), 1);

    // And through the binary, which is what a person runs.
    let r = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup-verify"))
        .arg(t.path())
        .arg("--dev-descriptor-key")
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&r.stderr)
    );
    assert!(
        String::from_utf8_lossy(&r.stdout).contains("server_key_unpinned"),
        "the document itself names it"
    );

    // A supplied key is the same statement with a different provenance word.
    let key = hex::encode(
        ed25519_dalek::SigningKey::from_bytes(&seed)
            .verifying_key()
            .to_bytes(),
    );
    let supplied = goalcup_verifier::verify_trace_with_options(
        t.path(),
        goalcup_verifier::VerifyOptions {
            allow_descriptor_key: false,
            extra_server_keys: vec![key],
            dev_mode: false,
        },
    )
    .unwrap();
    let f = supplied
        .review_queue
        .iter()
        .find(|f| f.code == "server_key_unpinned")
        .expect("server_key_unpinned");
    assert!(
        f.evidence.iter().any(|e| e.note.contains("supplied")),
        "{:?}",
        f.evidence
    );
}

/// The one entry in `SERVER_PUBLIC_KEYS` is the verifying key of the
/// seed `[5u8; 32]`, which is written in this repository on purpose so the fixtures can
/// sign with it. Anybody can therefore mint a receipt or an acknowledgement that resolves
/// to `KeySource::Pinned` — the strongest word the verdict has — so the verdict says out
/// loud when the key that verified something is that placeholder.
///
/// Suppressed by `--dev`, and by nothing else: a development run is where signing with the
/// published seed is the expected thing to do.
#[test]
fn the_placeholder_server_key_is_flagged_unless_the_run_is_dev() {
    assert_eq!(
        goalcup_verifier::PLACEHOLDER_SERVER_PUBLIC_KEY,
        goalcup_verifier::SERVER_PUBLIC_KEYS[0],
        "the placeholder stays on the allowlist so the fixtures keep working"
    );

    let t = support::compliant_trace();
    let strict = goalcup_verifier::verify_trace(t.path()).unwrap();
    assert_eq!(strict.key_source, goalcup_verifier::KeySource::Pinned);
    let f = strict
        .review_queue
        .iter()
        .find(|f| f.code == "placeholder_server_key")
        .unwrap_or_else(|| panic!("placeholder_server_key: {:?}", strict.review_queue));
    assert_eq!(f.severity, Severity::Flag);
    // Never the key itself: a verdict is published and a full hex key in a message is a
    // value a reader can mistake for a pin.
    assert!(
        !f.message
            .contains(goalcup_verifier::PLACEHOLDER_SERVER_PUBLIC_KEY),
        "{}",
        f.message
    );
    // The scenario builder's trace is V13-clean and not verdict-clean — its two legs do
    // not pair — so the claim about `overall` and the exit code is made where it can be:
    // over the committed compliant fixture, in `compliant.rs`.
    let v13 = strict.checks.iter().find(|c| c.id == "V13").unwrap();
    assert_eq!(v13.verdict, Severity::Flag, "{:?}", v13.findings);
    // `server_key_unpinned` is the other half and does not fire here: the placeholder is
    // pinned, which is exactly the problem this second code exists to say.
    assert!(
        !strict
            .review_queue
            .iter()
            .any(|f| f.code == "server_key_unpinned"),
        "{:?}",
        strict.review_queue
    );

    let dev = goalcup_verifier::verify_trace_with_options(
        t.path(),
        goalcup_verifier::VerifyOptions {
            allow_descriptor_key: false,
            extra_server_keys: Vec::new(),
            dev_mode: true,
        },
    )
    .unwrap();
    assert!(
        !dev.review_queue
            .iter()
            .any(|f| f.code == "placeholder_server_key"),
        "{:?}",
        dev.review_queue
    );
    let v13 = dev.checks.iter().find(|c| c.id == "V13").unwrap();
    assert_eq!(v13.verdict, Severity::Pass, "{:?}", v13.findings);
}

/// Both codes need the key to have been used. A trace that carried no receipt and no
/// acknowledgement resolved a key against nothing, and a finding about it would be a
/// statement about the invocation rather than about the trace.
///
/// The other direction is the acknowledgement half, asserted on a trace with no receipt at
/// all: the placeholder still verified the ack, and the Flag still fires.
#[test]
fn the_key_flags_need_the_key_to_have_verified_something() {
    // The three M0 descriptors carry no receipt and no acknowledgement.
    for id in ["claude-code-cli", "codex-cli"] {
        let v = goalcup_verifier::verify_trace(
            fixture_support::materialize_fixture(&format!("tests/traces/{id}")).path(),
        )
        .unwrap();
        assert!(
            !v.review_queue
                .iter()
                .any(|f| f.code == "placeholder_server_key" || f.code == "server_key_unpinned"),
            "{id}: {:?}",
            v.review_queue
        );
    }

    // No receipt, one acknowledgement the placeholder verified.
    let t = support::compliant_trace_without_receipt();
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    assert!(
        v.review_queue.iter().any(|f| f.code == "receipt_missing"),
        "{:?}",
        v.review_queue
    );
    assert!(
        v.review_queue
            .iter()
            .any(|f| f.code == "placeholder_server_key"),
        "the acknowledgement half raises it on its own: {:?}",
        v.review_queue
    );
}

/// The connector directory the run was captured with, in its three readings.
///
/// The manifest pin answers "which rulebook judged this run" from a file the trace carries.
/// This answers "which hook bodies captured it", and the trace carries none of them: the
/// daemon hashes its own connector directory, the server signs the value into the receipt,
/// and this is the verifier reading it back.
///
///   * **pinned** — the digest is this build's for the connector, and V13 says nothing.
///     Silence is what a pinned directory looks like, for the reason a pinned manifest is
///     silent: a Pass finding here would change three committed verdicts to say something
///     that was already true.
///   * **unpinned** — a Fail, and Pass-level under `--dev`, where running a connector whose
///     bytes are not yet pinned is the expected thing to be doing.
///   * **unbound** — no claim at all, which is a Flag and never a Fail: an M0 capture
///     predates the receipt, and a receipt signed before the claim existed is an old
///     document rather than a dishonest one.
#[test]
fn the_connector_directory_the_receipt_claims_is_compared_to_the_pins() {
    // ---- pinned: the shipped directory, hashed the way the daemon hashes it.
    let ok = support::compliant_trace();
    let v = goalcup_verifier::verify_trace(ok.path()).unwrap();
    let v13 = v.checks.iter().find(|c| c.id == "V13").unwrap();
    assert!(
        !v13.findings
            .iter()
            .any(|f| f.code.starts_with("connector_dir_")),
        "a pinned connector directory is silent: {:?}",
        v13.findings
    );

    // ---- unpinned: the manifest is still this build's, and a file beside it is not.
    let bad = support::compliant_trace_with_an_unpinned_connector_dir();
    let strict = goalcup_verifier::verify_trace(bad.path()).unwrap();
    assert!(
        !strict
            .review_queue
            .iter()
            .any(|f| f.code == "manifest_not_pinned"),
        "the rulebook is untouched — this finding is about the templates: {:?}",
        strict.review_queue
    );
    let f = strict
        .review_queue
        .iter()
        .find(|f| f.code == "connector_dir_unpinned")
        .unwrap_or_else(|| panic!("connector_dir_unpinned: {:?}", strict.review_queue));
    assert_eq!(f.severity, Severity::Fail);
    assert_eq!(
        f.evidence.first().map(|e| e.sha256.as_deref()),
        Some(Some("ab".repeat(32).as_str())),
        "the evidence carries the digest the receipt claimed"
    );

    let dev = goalcup_verifier::verify_trace_with_options(
        bad.path(),
        goalcup_verifier::VerifyOptions {
            allow_descriptor_key: false,
            extra_server_keys: Vec::new(),
            dev_mode: true,
        },
    )
    .unwrap();
    assert!(
        !dev.review_queue
            .iter()
            .any(|f| f.code == "connector_dir_unpinned"),
        "`--dev` puts it at Pass level, so it leaves the queue: {:?}",
        dev.review_queue
    );
    let v13 = dev.checks.iter().find(|c| c.id == "V13").unwrap();
    assert!(
        v13.findings
            .iter()
            .any(|f| f.code == "connector_dir_unpinned" && f.severity == Severity::Pass),
        "and the sentence is still said: {:?}",
        v13.findings
    );

    // ---- unbound: a receipt that verifies and was signed before the claim existed.
    let old = support::compliant_trace_without_a_connector_dir_claim();
    let queue = goalcup_verifier::verify_trace(old.path())
        .unwrap()
        .review_queue;
    let f = queue
        .iter()
        .find(|f| f.code == "connector_dir_unbound")
        .unwrap_or_else(|| panic!("connector_dir_unbound: {queue:?}"));
    assert_eq!(f.severity, Severity::Flag);

    // ---- and a capture with no receipt at all says nothing here. There is no document to
    // be old; `receipt_missing` — or `legacy_trace` on an M0 capture — is already the whole
    // statement, and a second Flag would be the same absence reported twice.
    let none = support::compliant_trace_without_receipt();
    for (what, verdict) in [
        (
            "a chain trace with no receipt",
            goalcup_verifier::verify_trace(none.path()).unwrap(),
        ),
        (
            "an M0 capture",
            goalcup_verifier::verify_trace(
                fixture_support::materialize_fixture("tests/traces/claude-code-cli").path(),
            )
            .unwrap(),
        ),
    ] {
        let v13 = verdict.checks.iter().find(|c| c.id == "V13").unwrap();
        assert!(
            !v13.findings
                .iter()
                .any(|f| f.code.starts_with("connector_dir_")),
            "{what}: {:?}",
            v13.findings
        );
    }
}

/// **An unverified receipt cannot buy silence here.**
///
/// The whole value of the directory claim is that it is written by a document the
/// participant could not produce. Without the signature gate, `connector_dir_pin_finding`
/// would read `receipt.claims` without asking whether the signature over those claims had
/// stood — so a participant running edited hook templates could sign their own receipt
/// naming the pinned digest and the check would fall silent, exactly as if the daemon had
/// said it.
///
/// Two ways a receipt fails to verify, and neither produces a directory finding of any
/// kind — not a silence bought by the claim, not a Flag, nothing:
///
///   * no key resolves, so the signature was carried and never checked. `receipt_unverified`
///     (Flag).
///   * a key resolves and the receipt names another, because the participant signed it
///     themselves. `receipt_invalid` (Fail).
///
/// The trace is refused on the binding either way, which is a larger statement than
/// anything this check could add.
#[test]
fn a_receipt_that_did_not_verify_says_nothing_about_the_connector_directory() {
    // Both traces claim the pinned digest — the claim that would have bought silence.
    for (what, want_code, dir) in [
        (
            "a receipt nothing checked",
            "receipt_unverified",
            support::compliant_trace_signed_and_named_by(&[9u8; 32]),
        ),
        (
            "a receipt the participant signed",
            "receipt_invalid",
            support::compliant_trace_signed_by(&[9u8; 32]),
        ),
    ] {
        let v = goalcup_verifier::verify_trace(dir.path()).unwrap();
        let v13 = v.checks.iter().find(|c| c.id == "V13").unwrap();
        assert!(
            v13.findings.iter().any(|f| f.code == want_code),
            "{what}: expected {want_code}: {:?}",
            v13.findings
        );
        assert!(
            !v13.findings
                .iter()
                .any(|f| f.code.starts_with("connector_dir_")),
            "{what}: a receipt that did not verify is not a source of connector claims: {:?}",
            v13.findings
        );
    }
}

/// The claim is absent from the signed bytes when it is `None`, so every receipt written
/// before the field existed still verifies.
///
/// Asserted over the bytes rather than over a verdict: `skip_serializing_if` is the whole
/// mechanism, and a `null` in the JSON would break every committed fixture's signature while
/// changing nothing a verdict says.
#[test]
fn a_receipt_without_the_connector_claim_signs_the_bytes_it_always_did() {
    use goalcup_protocol::receipt::StartReceiptClaims;

    let base = StartReceiptClaims {
        entry_id: "e1".to_string(),
        connector_id: "claude-code-cli".to_string(),
        workspace: "/ws/e1".to_string(),
        session_id: "s1".to_string(),
        started_at: "2026-09-01T00:00:00Z".to_string(),
        deadline_at: "2026-09-08T00:00:00Z".to_string(),
        manifest_sha256: "ab".repeat(32),
        chain_head: "cd".repeat(32),
        connector_dir_sha256: None,
        run_public_key: None,
        questionnaire_version: None,
    };
    let bytes = base.signing_bytes().expect("the claims serialise");
    let text = String::from_utf8(bytes.clone()).expect("the signing bytes are UTF-8");
    assert!(
        !text.contains("connector_dir_sha256"),
        "a None leaves no key, no comma and no null: {text}"
    );
    assert!(
        text.ends_with(&format!("\"chain_head\":\"{}\"}}", "cd".repeat(32))),
        "the object still ends where it always did: {text}"
    );

    // And a Some is appended last, so the eight fields before it are byte-identical.
    let with = StartReceiptClaims {
        connector_dir_sha256: Some("ef".repeat(32)),
        ..base
    };
    let with_text =
        String::from_utf8(with.signing_bytes().expect("the claims serialise")).expect("UTF-8");
    assert_eq!(
        with_text,
        format!(
            "{},\"connector_dir_sha256\":\"{}\"}}",
            text.trim_end_matches('}'),
            "ef".repeat(32)
        )
    );
}

// The third reading — a connector this build pins no manifest for — cannot be reached
// through `verify_trace` while every connector with a parser is also pinned: a descriptor
// naming a fourth connector is refused by `parser_for` before any check runs. It is
// asserted where it is decided instead, in `checks::v13_binding`'s own unit tests over
// `manifest_pin_finding`, and by `pinned_manifest_sha256` above.

/// `goalcup-devserver` signs with a seed published in this repository. Its key must
/// never be a pinned constant, so `--dev-key` is how a trace it signed becomes verifiable:
/// the caller supplies the trust and the build does not carry it.
#[test]
fn a_supplied_key_verifies_a_receipt_the_build_does_not_pin_and_says_it_was_supplied() {
    let seed = [7u8; 32];
    let t = support::compliant_trace_signed_and_named_by(&seed);
    let key = hex::encode(
        ed25519_dalek::SigningKey::from_bytes(&seed)
            .verifying_key()
            .to_bytes(),
    );
    assert!(
        !goalcup_verifier::SERVER_PUBLIC_KEYS.contains(&key.as_str()),
        "a public seed must never be a production anchor"
    );

    // Strict: the descriptor names a key this build does not recognise, so nothing checks
    // the receipt's signature and V13 says the start was carried, not verified.
    let strict = goalcup_verifier::verify_trace(t.path()).unwrap();
    assert_eq!(strict.key_source, goalcup_verifier::KeySource::None);
    assert!(
        strict
            .review_queue
            .iter()
            .any(|f| f.code == "receipt_unverified")
    );

    let dev = goalcup_verifier::verify_trace_with_options(
        t.path(),
        goalcup_verifier::VerifyOptions {
            allow_descriptor_key: false,
            extra_server_keys: vec![key.clone()],
            dev_mode: false,
        },
    )
    .unwrap();
    assert!(
        !dev.review_queue
            .iter()
            .any(|f| f.code == "receipt_invalid" || f.code == "receipt_unverified"),
        "{:?}",
        dev.review_queue
    );
    assert_eq!(dev.key_source, goalcup_verifier::KeySource::Supplied);

    let r = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup-verify"))
        .arg(t.path())
        .arg("--dev-key")
        .arg(&key)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&r.stderr).contains("--dev-key"),
        "a run that borrowed its trust from the caller must say so"
    );
}

/// A caller who names the key is the key's provenance: `--dev-key` verifies a trace whose
/// `trace.toml` names none, and the verdict says the trust was borrowed.
#[test]
fn a_supplied_key_stands_in_for_a_descriptor_that_names_none() {
    let seed = [7u8; 32];
    let t = support::compliant_trace_signed_and_named_by(&seed);
    let mut doc = support::read_descriptor(&t);
    doc.server_public_key = None;
    support::write_descriptor(&t, &doc);
    let key = hex::encode(
        ed25519_dalek::SigningKey::from_bytes(&seed)
            .verifying_key()
            .to_bytes(),
    );

    let strict = goalcup_verifier::verify_trace(t.path()).unwrap();
    assert_eq!(strict.key_source, goalcup_verifier::KeySource::None);

    let dev = goalcup_verifier::verify_trace_with_options(
        t.path(),
        goalcup_verifier::VerifyOptions {
            allow_descriptor_key: false,
            extra_server_keys: vec![key],
            dev_mode: false,
        },
    )
    .unwrap();
    assert_eq!(dev.key_source, goalcup_verifier::KeySource::Supplied);
    assert!(
        !dev.review_queue
            .iter()
            .any(|f| f.code == "receipt_invalid" || f.code == "receipt_unverified"),
        "{:?}",
        dev.review_queue
    );
}

/// The two dev flags, as a caller meets them.
#[test]
fn the_cli_lists_both_dev_flags_and_refuses_a_key_that_is_not_64_hex_characters() {
    let bin = env!("CARGO_BIN_EXE_goalcup-verify");
    let help = std::process::Command::new(bin)
        .arg("--help")
        .output()
        .unwrap();
    assert_eq!(help.status.code(), Some(0));
    let text = String::from_utf8_lossy(&help.stdout).to_string();
    assert!(
        text.contains("--dev\n") || text.contains("--dev "),
        "{text}"
    );
    assert!(text.contains("--dev-key"), "{text}");

    let t = support::compliant_trace();
    for bad in ["cafe", &"z".repeat(64), &"a".repeat(63)] {
        let out = std::process::Command::new(bin)
            .arg(t.path())
            .arg("--dev-key")
            .arg(bad)
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(3),
            "`{bad}` is not a key, and a usage error is never the Fail verdict"
        );
        assert!(out.stdout.is_empty(), "`{bad}`: nothing on stdout");
    }
}

/// A supported protocol number can still lie about the body it carries. The whole verdict
/// must fail at that line, not continue into a correctly linked suffix and publish a
/// `binding_checked` pass over a physical trace this build rejected.
#[test]
fn an_incompatible_envelope_fails_the_trace_and_keeps_all_downstream_checks_on_its_prefix() {
    let t = support::compliant_trace();
    support::append_v2_legacy_redaction_and_valid_suffix(&t);

    let verdict = goalcup_verifier::verify_trace(t.path()).unwrap();
    assert_eq!(verdict.overall, Severity::Fail);
    let v13 = verdict
        .checks
        .iter()
        .find(|check| check.id == "V13")
        .unwrap();
    assert_eq!(v13.verdict, Severity::Fail, "{:?}", v13.findings);
    let incompatibility = v13
        .findings
        .iter()
        .find(|finding| finding.code == "envelope_version_body_incompatible")
        .expect("V13 renders the typed version/body boundary");
    assert_eq!(incompatibility.severity, Severity::Fail);
    assert!(
        incompatibility.message.contains("seq"),
        "{}",
        incompatibility.message
    );
    assert!(
        !v13.findings
            .iter()
            .any(|finding| finding.code == "binding_checked"),
        "a rejected physical trace is not also declared bound: {:?}",
        v13.findings
    );
    assert!(
        verdict
            .human_messages
            .iter()
            .all(|message| !message.text.contains("suffix-must-not-reach-checks")),
        "the line after the typed boundary is outside every downstream check"
    );
}

/// Every scenario in this file, built through the one writer the daemon uses.
mod support {
    use ed25519_dalek::{Signer, SigningKey};
    use goalcup_protocol::ack::AckCheckpoint;
    use goalcup_protocol::chain::Link;
    use goalcup_protocol::cursor::{CursorUnit, StoreCursor};
    use goalcup_protocol::envelope::{Envelope, EventBody};
    use goalcup_protocol::export::{
        ACKS_FILE, BASELINE_FILE, CHAIN_FILE, DESCRIPTOR_FILE, RECEIPT_FILE, SEAL_FILE,
        SUBMISSION_MANIFEST_FILE, SUBMISSION_SHA256_OBJECTS_DIR, TraceDescriptorDoc, TraceExport,
    };
    use goalcup_protocol::manifest::{ConnectorManifest, InstallScope};
    use goalcup_protocol::provenance::{
        CaptureUnavailableV1, CutoffReachedV1, FinalizationV1, NoRepositoryReasonV1,
        PreCutoffDrainOutcomeV1, PublisherNotAttemptedReasonV1, PublisherNotAttemptedV1,
        RepositoryNonSelectionV1, Rfc3339Utc, Sha256Digest, SnapshotNotAttemptedV1,
        TerminationFailureV1, TerminationMethodV1, TerminationOutcomeV1,
    };
    use goalcup_protocol::receipt::{StartReceipt, StartReceiptClaims};
    use goalcup_protocol::redaction::RedactionRecord;
    use goalcup_protocol::seal::{SealManifest, SealedFile};
    use goalcup_protocol::submission::{
        EffectiveSubmissionLimitsV1, FinishReasonV1, LimitProfileV1, SubmissionCapturedV1,
        SubmissionEntryV1, SubmissionFailureV1, SubmissionKindV1, SubmissionManifestV1,
        SubmissionOutcomeV1, SubmissionProfileV1,
    };
    use sha2::{Digest, Sha256};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use tempfile::TempDir;

    pub const ENTRY_ID: &str = "compliant-0001";
    pub const SESSION: &str = "11111111-2222-3333-4444-555555555555";
    pub const WORKSPACE: &str = "/ws/compliant";
    pub const CONNECTOR: &str = "claude-code-cli";
    /// The receipt's `started_at`: the lower bound of the window V13 measures against.
    pub const STARTED_AT: &str = "2026-09-04T10:00:00Z";
    /// The receipt's `deadline_at`: the upper bound when a run never sealed.
    pub const DEADLINE_AT: &str = "2026-09-04T11:00:00Z";
    /// The seal's `sealed_at`: the upper bound when it did.
    pub const SEALED_AT: &str = "2026-09-04T10:05:00Z";
    /// A `sealed_at` an hour past `DEADLINE_AT`: the late seal, which is
    /// what a daemon that was not running at the deadline produces at its next start.
    pub const LATE_SEALED_AT: &str = "2026-09-04T12:00:00Z";
    /// A `sealed_at` no build can read, written through `write_seal` like any other, so
    /// the chain carries it, the digest agrees, and the only thing wrong with the trace is
    /// that the window has no upper edge. Deliberately longer than 32 characters: a bound's
    /// text is a participant-written string and a verdict is a published document, so a
    /// finding may carry at most 32 characters of it.
    pub const UNREADABLE_SEALED_AT: &str =
        "not-a-date-and-far-too-long-to-be-repeated-in-a-verdict";
    /// The envelope `recorded_at` of the record planted by
    /// `compliant_trace_with_an_unreadable_clock`.
    pub const UNREADABLE_RECORDED_AT: &str = "sometime on Thursday";
    /// The one file `Plan::sealed_file` puts in both the baseline and the seal, with the
    /// same digest in both. No tool call in the trace names it, so the baseline is the
    /// only thing that can explain it: exempt where the chain binds the baseline, and
    /// `unexplained_seal_file` where it does not.
    pub const KEPT_FILE: &str = "kept.py";
    const KEPT_SHA: &str = "ab00112233445566778899aabbccddeeff00112233445566778899aabbccddee";
    /// The seed whose verifying key is the single entry in
    /// `goalcup_verifier::SERVER_PUBLIC_KEYS`. A receipt signed by anything else is
    /// exactly the "you signed it yourself" case.
    pub const PINNED_SEED: [u8; 32] = [5u8; 32];

    /// Which of the five shapes to build. One builder, because the five differ by a
    /// single decision each and five copies of the writer sequence would be five places
    /// for a compliant trace to stop being compliant.
    #[derive(Clone, Copy)]
    struct Plan {
        /// The seed the receipt is signed with. `None` writes no receipt at all.
        seed: Option<[u8; 32]>,
        /// The seed that signs the acknowledgement and whose verifying key the descriptor
        /// names as `server_public_key`. Separate from `seed` because the two questions
        /// are separate: `seed` asks who signed the start, this one asks which key the
        /// trace says checks it. Default `PINNED_SEED`, so every existing scenario names
        /// the key this build pins.
        server_seed: [u8; 32],
        /// Plant two leg-1 records outside the run: one an hour below `started_at` and
        /// one a minute above the seal. The two are answered
        /// differently — the early one is `pre_start_records_early`, a Flag; the late one
        /// is `outside_window`, the Fail — which is exactly why the fixture keeps both.
        out_of_window: bool,
        /// Plant two leg-1 records inside the arming window: `started_at` minus 30s and
        /// minus 2s. What every honest `claude-code-cli` run writes, because its arming
        /// signal is `launch+first_transcript_record` and the receipt is requested only
        /// once that record exists.
        arming_records: bool,
        /// Plant one leg-1 record thirty minutes above `deadline_at`, so the upper edge is
        /// measurable on a run that never sealed.
        after_deadline: bool,
        /// Plant one leg-1 record ten minutes below `started_at` — twice the arming
        /// window, and nothing else wrong with the trace.
        ten_minutes_early: bool,
        /// Rewrite seq 0's payload in the file after the fact, re-stamping its own hash.
        rewrite_genesis: bool,
        /// Write `seal.json` beside the chain instead of through `write_seal`.
        unchained_seal: bool,
        /// The seal's `sealed_at`, which is also the seal link's `recorded_at`. Anything
        /// other than `SEALED_AT` makes the window's upper bound unreadable.
        sealed_at: &'static str,
        /// Plant one leg-1 record whose envelope `recorded_at` is not a timestamp.
        unreadable_clock: bool,
        /// Where the baseline goes: into the chain as seq 0, beside it as a file the
        /// chain never covered, or nowhere at all.
        baseline: BaselinePlan,
        /// Put `KEPT_FILE` in both the baseline and the seal, so a test can ask what the
        /// attributor does with a file the baseline claims to have held.
        sealed_file: bool,
        /// What the receipt claims as `connector_dir_sha256`.
        ///
        /// Three readings, and V13 has a finding for two of them. `Pinned` is what an
        /// honest daemon shipping the release's own connectors puts there — the digest of
        /// `connectors/claude-code-cli/`, taken here the same way the daemon takes it.
        /// `Unpinned` is a well-formed digest of no directory this build ships. `Absent`
        /// leaves the claim off entirely, which is both a receipt signed before the field
        /// existed and the shape `skip_serializing_if` has to preserve byte for byte.
        connector_dir: ConnectorDirPlan,
        /// Optional claims carried by the signed receipt.
        receipt_binding: ReceiptBindingPlan,
        /// Replace the legacy seal terminal with a cutoff and FinalizationV1.
        finalization: bool,
        /// The V4 workspace-submission terminal claim, when this fixture needs one.
        workspace_submission: Option<WorkspaceSubmissionPlan>,
        /// One independently invalid V4 relationship. `None` is the genuinely bound shape.
        submission_fault: SubmissionFault,
        /// Retain one file that no traced write or baseline entry explains.
        submission_unexplained_file: bool,
    }

    /// The three things a receipt can say about the connector directory.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ConnectorDirPlan {
        Pinned,
        Unpinned,
        Absent,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ReceiptBindingPlan {
        Legacy,
        Bound,
        Half,
        Malformed,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum WorkspaceSubmissionPlan {
        AlreadyStoppedTermination,
        UnresolvedTermination,
        Incomplete,
        Captured,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum SubmissionFault {
        None,
        ReceiptDigest,
        Deadline,
        FinishReason,
        Exclusions,
        PostFinalizationWork,
        StartedAfterCutoff,
        TerminationRequestedBeforeCutoff,
        CopyStartedBeforeConfirmation,
        ReadbackAfterFinalization,
        UnparseableStart,
    }

    /// The three places a baseline can be, and the reason there are three.
    ///
    /// `baseline.json` is written by the participant's own daemon. `Chained` is the shape
    /// `TraceExport` produces and the only one that binds anything: seq 0, under the
    /// receipt's `chain_head`. `Unchained` is the same file with no link over it — the
    /// participant's own list of what they say the workspace held. `Absent` is a trace
    /// that says nothing about t0 at all.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum BaselinePlan {
        Chained,
        Unchained,
        Absent,
    }

    impl Default for Plan {
        fn default() -> Plan {
            Plan {
                seed: Some(PINNED_SEED),
                server_seed: PINNED_SEED,
                out_of_window: false,
                arming_records: false,
                after_deadline: false,
                ten_minutes_early: false,
                rewrite_genesis: false,
                unchained_seal: false,
                sealed_at: SEALED_AT,
                unreadable_clock: false,
                baseline: BaselinePlan::Chained,
                sealed_file: false,
                connector_dir: ConnectorDirPlan::Pinned,
                receipt_binding: ReceiptBindingPlan::Legacy,
                finalization: false,
                workspace_submission: None,
                submission_fault: SubmissionFault::None,
                submission_unexplained_file: false,
            }
        }
    }

    /// A minimal compliant chain trace, built in a fresh tempdir through `TraceExport`:
    /// copy `connectors/claude-code-cli/manifest.toml` in, append two leg-1 records,
    /// their two leg-2 posts and one heartbeat, then write a descriptor, a receipt signed
    /// by the pinned test key, a baseline, a seal and one ack.
    pub fn compliant_trace() -> TempDir {
        build(Plan::default())
    }

    pub fn finalized_trace_with_legacy_receipt() -> TempDir {
        build(Plan {
            finalization: true,
            ..Plan::default()
        })
    }

    pub fn finalized_trace_with_bound_receipt() -> TempDir {
        build(Plan {
            finalization: true,
            receipt_binding: ReceiptBindingPlan::Bound,
            ..Plan::default()
        })
    }

    pub fn finalized_trace_with_half_binding() -> TempDir {
        build(Plan {
            finalization: true,
            receipt_binding: ReceiptBindingPlan::Half,
            ..Plan::default()
        })
    }

    pub fn finalized_trace_with_malformed_binding() -> TempDir {
        build(Plan {
            finalization: true,
            receipt_binding: ReceiptBindingPlan::Malformed,
            ..Plan::default()
        })
    }

    pub fn workspace_submission_with_process_authority_unavailable() -> TempDir {
        build(Plan {
            finalization: true,
            workspace_submission: Some(WorkspaceSubmissionPlan::UnresolvedTermination),
            receipt_binding: ReceiptBindingPlan::Bound,
            ..Plan::default()
        })
    }

    pub fn workspace_submission_with_already_stopped_termination() -> TempDir {
        build(Plan {
            finalization: true,
            workspace_submission: Some(WorkspaceSubmissionPlan::AlreadyStoppedTermination),
            receipt_binding: ReceiptBindingPlan::Bound,
            ..Plan::default()
        })
    }

    pub fn workspace_submission_with_incomplete_capture() -> TempDir {
        build(Plan {
            finalization: true,
            workspace_submission: Some(WorkspaceSubmissionPlan::Incomplete),
            receipt_binding: ReceiptBindingPlan::Bound,
            ..Plan::default()
        })
    }

    pub fn workspace_submission_with_captured_manifest() -> TempDir {
        build(Plan {
            finalization: true,
            workspace_submission: Some(WorkspaceSubmissionPlan::Captured),
            receipt_binding: ReceiptBindingPlan::Bound,
            ..Plan::default()
        })
    }

    pub fn workspace_submission_with_wrong_receipt_digest() -> TempDir {
        build(Plan {
            finalization: true,
            workspace_submission: Some(WorkspaceSubmissionPlan::Captured),
            receipt_binding: ReceiptBindingPlan::Bound,
            submission_fault: SubmissionFault::ReceiptDigest,
            ..Plan::default()
        })
    }

    pub fn workspace_submission_with_wrong_deadline() -> TempDir {
        build(Plan {
            finalization: true,
            workspace_submission: Some(WorkspaceSubmissionPlan::Captured),
            receipt_binding: ReceiptBindingPlan::Bound,
            submission_fault: SubmissionFault::Deadline,
            ..Plan::default()
        })
    }

    pub fn workspace_submission_with_wrong_finish_reason() -> TempDir {
        build(Plan {
            finalization: true,
            workspace_submission: Some(WorkspaceSubmissionPlan::Captured),
            receipt_binding: ReceiptBindingPlan::Bound,
            submission_fault: SubmissionFault::FinishReason,
            ..Plan::default()
        })
    }

    pub fn workspace_submission_with_wrong_exclusions() -> TempDir {
        build(Plan {
            finalization: true,
            workspace_submission: Some(WorkspaceSubmissionPlan::Captured),
            receipt_binding: ReceiptBindingPlan::Bound,
            submission_fault: SubmissionFault::Exclusions,
            ..Plan::default()
        })
    }

    pub fn workspace_submission_with_post_finalization_work() -> TempDir {
        build(Plan {
            finalization: true,
            workspace_submission: Some(WorkspaceSubmissionPlan::Captured),
            receipt_binding: ReceiptBindingPlan::Bound,
            submission_fault: SubmissionFault::PostFinalizationWork,
            ..Plan::default()
        })
    }

    fn workspace_submission_with_chronology_fault(fault: SubmissionFault) -> TempDir {
        build(Plan {
            finalization: true,
            workspace_submission: Some(WorkspaceSubmissionPlan::Captured),
            receipt_binding: ReceiptBindingPlan::Bound,
            submission_fault: fault,
            ..Plan::default()
        })
    }

    pub fn workspace_submission_started_after_cutoff() -> TempDir {
        workspace_submission_with_chronology_fault(SubmissionFault::StartedAfterCutoff)
    }

    pub fn workspace_submission_termination_requested_before_cutoff() -> TempDir {
        workspace_submission_with_chronology_fault(
            SubmissionFault::TerminationRequestedBeforeCutoff,
        )
    }

    pub fn workspace_submission_copy_started_before_confirmation() -> TempDir {
        workspace_submission_with_chronology_fault(SubmissionFault::CopyStartedBeforeConfirmation)
    }

    pub fn workspace_submission_readback_after_finalization() -> TempDir {
        workspace_submission_with_chronology_fault(SubmissionFault::ReadbackAfterFinalization)
    }

    pub fn workspace_submission_with_unparseable_start() -> TempDir {
        workspace_submission_with_chronology_fault(SubmissionFault::UnparseableStart)
    }

    pub fn workspace_submission_with_unexplained_file() -> TempDir {
        build(Plan {
            finalization: true,
            workspace_submission: Some(WorkspaceSubmissionPlan::Captured),
            receipt_binding: ReceiptBindingPlan::Bound,
            submission_unexplained_file: true,
            ..Plan::default()
        })
    }

    pub fn workspace_submission_with_conflicting_legacy_seal() -> TempDir {
        let trace = workspace_submission_with_unexplained_file();
        let legacy = SealManifest {
            entry_id: ENTRY_ID.to_string(),
            sealed_at: "2026-09-04T10:05:05Z".to_string(),
            workspace: WORKSPACE.to_string(),
            files: Vec::new(),
            excluded: Vec::new(),
        };
        std::fs::write(
            trace.path().join(SEAL_FILE),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        let descriptor = trace.path().join(DESCRIPTOR_FILE);
        let mut text = std::fs::read_to_string(&descriptor).unwrap();
        text.push_str("seal = \"seal.json\"\n");
        std::fs::write(descriptor, text).unwrap();
        trace
    }

    /// `compliant_trace()` with the receipt signed by `SigningKey::from_bytes(seed)`.
    ///
    /// The descriptor still names the pinned key, so this is the self-signed receipt:
    /// a start signed by somebody other than the server this build recognises.
    pub fn compliant_trace_signed_by(seed: &[u8; 32]) -> TempDir {
        build(Plan {
            seed: Some(*seed),
            ..Plan::default()
        })
    }

    /// `compliant_trace()` where one key signs the receipt and the acknowledgement and is
    /// the key the descriptor names — a trace from a server this build does not pin.
    ///
    /// This is `goalcup-devserver` on disk: a signer whose seed is public and in the
    /// repository on purpose. Nothing about the trace is otherwise irregular, so what a
    /// default run reports about it is exactly "no recognised key applies", and
    /// `--dev-key` is what makes it verifiable without the build carrying the anchor.
    pub fn compliant_trace_signed_and_named_by(seed: &[u8; 32]) -> TempDir {
        build(Plan {
            seed: Some(*seed),
            server_seed: *seed,
            ..Plan::default()
        })
    }

    /// Rewrite one positive rule in the trace's own copy of the connector manifest, so the
    /// rules that judged the run are no longer the rules this build ships.
    ///
    /// The edit is a legal `human_rule.positive` rule that parses and validates: the attack
    /// is not a corrupt file, it is a plausible one whose positive test lets no typed
    /// message count as human.
    pub fn edit_the_manifest(t: &TempDir) {
        let path = t.path().join("manifest.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        let typed = "field = \"promptSource\", value = \"typed\"";
        assert!(
            text.contains(typed),
            "the positive rule this edit rewrites is gone"
        );
        let edited = text.replace(typed, "field = \"promptSource\", value = \"nobody\"");
        std::fs::write(&path, edited).unwrap();
    }

    /// `compliant_trace()` whose receipt claims a connector-directory digest no build pins.
    ///
    /// Everything else is unchanged, the manifest included: the point is that the rulebook
    /// can be exactly this build's while the hook bodies beside it are not.
    pub fn compliant_trace_with_an_unpinned_connector_dir() -> TempDir {
        build(Plan {
            connector_dir: ConnectorDirPlan::Unpinned,
            ..Plan::default()
        })
    }

    /// `compliant_trace()` whose receipt makes no connector-directory claim — the shape
    /// every receipt signed before that claim existed has.
    pub fn compliant_trace_without_a_connector_dir_claim() -> TempDir {
        build(Plan {
            connector_dir: ConnectorDirPlan::Absent,
            ..Plan::default()
        })
    }

    /// `compliant_trace()` with no `receipt.json` and no `receipt` key in the descriptor.
    pub fn compliant_trace_without_receipt() -> TempDir {
        build(Plan {
            seed: None,
            ..Plan::default()
        })
    }

    /// `compliant_trace()` plus two leg-1 records whose `recorded_at` is one hour before
    /// `started_at` and one minute after the seal's `sealed_at`.
    pub fn compliant_trace_with_out_of_window_records() -> TempDir {
        build(Plan {
            out_of_window: true,
            ..Plan::default()
        })
    }

    /// `compliant_trace()` plus two leg-1 records dated 30 seconds and 2 seconds below
    /// `started_at`: the arming phase every honest run has, said in the fixture.
    pub fn compliant_trace_with_arming_records() -> TempDir {
        build(Plan {
            arming_records: true,
            ..Plan::default()
        })
    }

    /// `compliant_trace()` plus one leg-1 record thirty minutes above `deadline_at`, and
    /// nothing below the start. The upper edge on its own.
    pub fn compliant_trace_with_a_record_after_the_deadline() -> TempDir {
        build(Plan {
            after_deadline: true,
            ..Plan::default()
        })
    }

    /// `compliant_trace()` plus one leg-1 record ten minutes below `started_at`: past the
    /// arming window, and the only thing wrong with the trace.
    pub fn compliant_trace_ten_minutes_early() -> TempDir {
        build(Plan {
            ten_minutes_early: true,
            ..Plan::default()
        })
    }

    /// `compliant_trace()` with the seq-0 line's payload edited in the file after the
    /// fact, so the genesis link no longer equals the receipt's `chain_head` and the
    /// chain no longer recomputes.
    pub fn compliant_trace_with_rewritten_genesis() -> TempDir {
        build(Plan {
            rewrite_genesis: true,
            ..Plan::default()
        })
    }

    /// `compliant_trace()` whose `seal.json` was written with `serde_json::to_vec`
    /// directly instead of through `TraceExport::write_seal`, so the descriptor names a
    /// seal the chain never saw. The one construction in this file that does not go
    /// through the writer, and it says why: the threat is precisely a manifest at a
    /// participant-named path that no link covers, so a fixture for it cannot be built
    /// by the writer that would have covered it.
    pub fn compliant_trace_with_unchained_seal() -> TempDir {
        build(Plan {
            unchained_seal: true,
            ..Plan::default()
        })
    }

    /// `compliant_trace()` sealed an hour after its own deadline, with one leg-1 record
    /// half an hour after the deadline and half an hour before the seal.
    /// The record is inside the seal's window and outside the deadline's, which is the
    /// whole question.
    pub fn compliant_trace_sealed_late() -> TempDir {
        build(Plan {
            sealed_at: LATE_SEALED_AT,
            after_deadline: true,
            ..Plan::default()
        })
    }

    /// `compliant_trace()` whose seal — and therefore the seal link the writer stamps
    /// from it — carries `sealed_at`. Written through `write_seal` like every other seal,
    /// so nothing else about the trace moves.
    pub fn compliant_trace_sealed_at(sealed_at: &'static str) -> TempDir {
        build(Plan {
            sealed_at,
            ..Plan::default()
        })
    }

    /// `compliant_trace()` with `baseline.json` written straight to disk instead of
    /// through `TraceExport::write_baseline`, so the descriptor names a baseline the
    /// chain never saw. The baseline's analogue of `compliant_trace_with_unchained_seal`,
    /// and built outside the writer for the same reason: the threat is a manifest at a
    /// participant-named path that no link covers, so a fixture for it cannot be built by
    /// the writer that would have covered it.
    pub fn compliant_trace_with_an_unchained_baseline() -> TempDir {
        build(Plan {
            baseline: BaselinePlan::Unchained,
            ..Plan::default()
        })
    }

    /// `compliant_trace()` with no `baseline.json` and no `baseline` key in the
    /// descriptor: a trace that says nothing at all about what the workspace held at t0.
    pub fn compliant_trace_without_a_baseline() -> TempDir {
        build(Plan {
            baseline: BaselinePlan::Absent,
            ..Plan::default()
        })
    }

    /// `compliant_trace()` with `KEPT_FILE` in both the baseline and the seal, under the
    /// same digest. Nothing in the trace's tool calls names it, so it is exempt exactly
    /// as far as the baseline is believed.
    pub fn compliant_trace_with_a_baseline_file() -> TempDir {
        build(Plan {
            sealed_file: true,
            ..Plan::default()
        })
    }

    /// The same file, in a baseline no link covers: what the participant says the
    /// workspace held, with nothing standing behind it.
    pub fn compliant_trace_with_an_unchained_baseline_file() -> TempDir {
        build(Plan {
            baseline: BaselinePlan::Unchained,
            sealed_file: true,
            ..Plan::default()
        })
    }

    /// `compliant_trace()` plus one leg-1 record whose envelope `recorded_at` is
    /// `UNREADABLE_RECORDED_AT`. Its inner harness timestamp stays valid: the malformed
    /// clock is the daemon's own field, which is the one V13 measures the window against.
    pub fn compliant_trace_with_an_unreadable_clock() -> TempDir {
        build(Plan {
            unreadable_clock: true,
            ..Plan::default()
        })
    }

    /// Read a generated trace's `trace.toml` as the exporter's own document, so a test
    /// can alter exactly one field and put it back.
    pub fn read_descriptor(t: &TempDir) -> TraceDescriptorDoc {
        let text = std::fs::read_to_string(t.path().join(DESCRIPTOR_FILE)).unwrap();
        toml::from_str(&text).unwrap()
    }

    pub fn write_descriptor(t: &TempDir, doc: &TraceDescriptorDoc) {
        let text = toml::to_string(doc).unwrap();
        std::fs::write(t.path().join(DESCRIPTOR_FILE), text).unwrap();
    }

    /// Append one envelope and return the link the writer stamped. The link is never this
    /// generator's to compute: a fixture that hand-rolled `seq`, `prev` and `hash` would
    /// assert agreement with a formula it wrote itself.
    fn push(export: &mut TraceExport, source: &str, at: &str, body: EventBody) -> Link {
        let mut env = Envelope {
            protocol_version: goalcup_protocol::PROTOCOL_VERSION,
            entry_id: ENTRY_ID.to_string(),
            source: source.to_string(),
            recorded_at: at.to_string(),
            link: Link {
                seq: 0,
                prev: String::new(),
                hash: String::new(),
            },
            body,
            redaction: None,
        };
        export.append_chain(&mut env).expect("the writer stamps it")
    }

    /// Build the writer-valid V4 fixed layout before asking `TraceExport` to chain a
    /// captured finalization. The verifier bridge remains deliberately blind to these
    /// bytes; this fixture reaches V13 only after the protocol accepts the binding claim.
    fn captured_workspace_submission(
        tmp: &TempDir,
        cutoff: &Link,
        receipt_sha256: &Sha256Digest,
        finish_reason: FinishReasonV1,
        excluded: Vec<String>,
        unexplained_file: bool,
        fault: SubmissionFault,
    ) -> SubmissionOutcomeV1 {
        let unexplained_bytes = b"unexplained retained bytes";
        let unexplained_digest = Sha256Digest::from_bytes(Sha256::digest(unexplained_bytes).into());
        let entries = unexplained_file
            .then(|| SubmissionEntryV1 {
                path: goalcup_protocol::provenance::PortableRelativePath::try_from(
                    "unexplained.txt",
                )
                .unwrap(),
                kind: SubmissionKindV1::Regular,
                posix_owner_executable: false,
                bytes: unexplained_bytes.len() as u64,
                content_sha256: unexplained_digest.clone(),
            })
            .into_iter()
            .collect::<Vec<_>>();
        let manifest = SubmissionManifestV1 {
            profile: SubmissionProfileV1::WorkspaceSnapshotV1,
            limit_profile: LimitProfileV1 {
                id: "workspace-v1".to_string(),
            },
            effective_limits: EffectiveSubmissionLimitsV1 {
                object_count: 1,
                total_bytes: 1024,
                pre_cutoff_drain: 1,
                process_walk: 1,
            },
            entry_id: ENTRY_ID.to_string(),
            receipt_sha256: receipt_sha256.clone(),
            cutoff_seq: cutoff.seq,
            cutoff_chain_hash: Sha256Digest::try_from(cutoff.hash.as_str()).unwrap(),
            finish_reason,
            entries,
            excluded,
            copy_started_at: Rfc3339Utc::try_from(
                if fault == SubmissionFault::CopyStartedBeforeConfirmation {
                    "2026-09-04T10:05:01Z"
                } else {
                    "2026-09-04T10:05:03Z"
                },
            )
            .unwrap(),
            copy_completed_at: Rfc3339Utc::try_from("2026-09-04T10:05:04Z").unwrap(),
            readback_verified_at: Rfc3339Utc::try_from(
                if fault == SubmissionFault::ReadbackAfterFinalization {
                    "2026-09-04T10:05:07Z"
                } else {
                    "2026-09-04T10:05:05Z"
                },
            )
            .unwrap(),
            object_count: u64::from(unexplained_file),
            total_bytes: if unexplained_file {
                unexplained_bytes.len() as u64
            } else {
                0
            },
        };
        std::fs::create_dir_all(tmp.path().join(SUBMISSION_SHA256_OBJECTS_DIR)).unwrap();
        let manifest_path = tmp.path().join(SUBMISSION_MANIFEST_FILE);
        std::fs::write(&manifest_path, manifest.canonical_bytes().unwrap()).unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&manifest_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        if unexplained_file {
            let object_path = tmp
                .path()
                .join(SUBMISSION_SHA256_OBJECTS_DIR)
                .join(unexplained_digest.as_str());
            std::fs::write(&object_path, unexplained_bytes).unwrap();
            #[cfg(unix)]
            std::fs::set_permissions(object_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        SubmissionOutcomeV1::Captured(SubmissionCapturedV1 {
            manifest_sha256: manifest.digest().unwrap(),
        })
    }

    /// One leg-1 record body: the harness's own line as a JSON **string**, which is what
    /// a line-oriented store's payload is. There is no `{path,text}` wrapper anywhere.
    fn record(value: serde_json::Value) -> EventBody {
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String(serde_json::to_string(&value).unwrap()),
        }
    }

    fn user_record(uuid: &str, at: &str, text: &str) -> serde_json::Value {
        serde_json::json!({
            "parentUuid": null,
            "isSidechain": false,
            "promptId": uuid,
            "type": "user",
            "message": {"role": "user", "content": text},
            "uuid": uuid,
            "timestamp": at,
            "permissionMode": "auto",
            "origin": {"kind": "human"},
            "promptSource": "typed",
            "userType": "external",
            "entrypoint": "cli",
            "cwd": WORKSPACE,
            "sessionId": SESSION,
            "version": "2.1.259",
            "gitBranch": "HEAD",
        })
    }

    fn assistant_record(uuid: &str, parent: &str, at: &str) -> serde_json::Value {
        serde_json::json!({
            "parentUuid": parent,
            "isSidechain": false,
            "type": "assistant",
            "message": {
                "role": "assistant",
                "model": "claude-opus-5",
                "content": [{"type": "text", "text": "done"}],
                "usage": {"input_tokens": 1, "output_tokens": 1},
            },
            "uuid": uuid,
            "timestamp": at,
            "cwd": WORKSPACE,
            "sessionId": SESSION,
            "version": "2.1.259",
        })
    }

    fn build(plan: Plan) -> TempDir {
        let tmp = tempfile::tempdir().expect("a temporary trace directory");
        let manifest_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../connectors")
            .join(CONNECTOR)
            .join("manifest.toml");
        let manifest_bytes = std::fs::read(&manifest_src)
            .unwrap_or_else(|e| panic!("{}: {e}", manifest_src.display()));
        let connector_manifest =
            ConnectorManifest::from_toml_str(std::str::from_utf8(&manifest_bytes).unwrap())
                .unwrap();
        let mut submission_exclusions = connector_manifest.workspace_exclusions.clone();
        submission_exclusions.extend(
            connector_manifest
                .hook_install
                .iter()
                .filter(|hook| hook.scope == InstallScope::Workspace)
                .map(|hook| hook.path.clone()),
        );
        submission_exclusions.sort();
        submission_exclusions.dedup();
        let mut export = TraceExport::create(tmp.path()).expect("open the export");
        export
            .write_manifest(&manifest_bytes)
            .expect("write the manifest");
        // The connector directory digest the receipt claims. Only `manifest.toml` is copied
        // into the trace — that is the point of the claim — so this is read from the shipped
        // directory the same way the daemon reads its own.
        let connector_dir_sha256 = match plan.connector_dir {
            ConnectorDirPlan::Pinned => Some(
                goalcup_protocol::connector_dir::connector_dir_sha256(
                    manifest_src.parent().expect("the manifest has a directory"),
                )
                .expect("the shipped connector directory hashes"),
            ),
            ConnectorDirPlan::Unpinned => Some("ab".repeat(32)),
            ConnectorDirPlan::Absent => None,
        };
        // ---- the baseline, chained first where the plan says it is chained: it is
        // then seq 0, which is the link the receipt's `chain_head` witnesses. A baseline
        // the server never saw could be rewritten afterwards, and a pre-populated
        // workspace would launder itself through a list the participant wrote.
        let files = if plan.sealed_file {
            vec![SealedFile {
                path: KEPT_FILE.to_string(),
                bytes: 7,
                sha256: KEPT_SHA.to_string(),
            }]
        } else {
            Vec::new()
        };
        let baseline = SealManifest {
            entry_id: ENTRY_ID.to_string(),
            sealed_at: STARTED_AT.to_string(),
            workspace: WORKSPACE.to_string(),
            files: files.clone(),
            excluded: Vec::new(),
        }
        .sorted();
        let chained_baseline = match plan.baseline {
            BaselinePlan::Chained => Some(
                export
                    .write_baseline(&baseline)
                    .expect("chain and write the baseline")
                    .1,
            ),
            // Not through the writer: see `compliant_trace_with_an_unchained_baseline`.
            BaselinePlan::Unchained => {
                std::fs::write(
                    tmp.path().join(BASELINE_FILE),
                    serde_json::to_vec(&baseline).unwrap(),
                )
                .unwrap();
                None
            }
            BaselinePlan::Absent => None,
        };

        // ---- leg 1 and leg 2, interleaved as the daemon would have observed them.
        let first_record = push(
            &mut export,
            "daemon",
            "2026-09-04T10:00:10Z",
            record(user_record("u1", "2026-09-04T10:00:10Z", "keep going")),
        );
        // Whatever the writer stamped as seq 0 — the baseline where there is one, the
        // first record where there is not. `chain_head` is a claim about seq 0 and about
        // nothing else, so it is read from the link rather than from the plan.
        let genesis = chained_baseline.unwrap_or(first_record);
        push(
            &mut export,
            "hooks",
            "2026-09-04T10:00:11Z",
            EventBody::Leg2Post {
                event: "/UserPromptSubmit".to_string(),
                payload: serde_json::json!({
                    "session_id": SESSION,
                    "prompt": "keep going",
                    "cwd": WORKSPACE,
                }),
                path: None,
                receiver_meta: None,
            },
        );
        push(
            &mut export,
            "daemon",
            "2026-09-04T10:00:20Z",
            record(assistant_record("u2", "u1", "2026-09-04T10:00:20Z")),
        );
        push(
            &mut export,
            "hooks",
            "2026-09-04T10:00:21Z",
            EventBody::Leg2Post {
                event: "/Stop".to_string(),
                payload: serde_json::json!({"session_id": SESSION}),
                path: None,
                receiver_meta: None,
            },
        );
        push(
            &mut export,
            "daemon",
            "2026-09-04T10:01:00Z",
            EventBody::Heartbeat {
                ts: "2026-09-04T10:01:00Z".to_string(),
                stores_seen: vec![StoreCursor {
                    store: "main".to_string(),
                    unit: CursorUnit::Records,
                    position: Some(2),
                    describe: "rowid:2".to_string(),
                }],
                receiver_ok: true,
            },
        );

        // ---- work the receipt and the seal do not cover, in both directions.
        // Appended in seq order — the early one first — so the finding's first and last
        // offending seq are a deterministic pair rather than an artefact of file order.
        if plan.out_of_window {
            push(
                &mut export,
                "daemon",
                "2026-09-04T09:00:00Z",
                record(assistant_record("u3", "u2", "2026-09-04T09:00:00Z")),
            );
            push(
                &mut export,
                "daemon",
                "2026-09-04T10:06:00Z",
                record(assistant_record("u4", "u2", "2026-09-04T10:06:00Z")),
            );
        }

        // ---- the arming phase: records the run had to write to reach `started_at`. Dated
        // inside the 300s arming window, so V13 counts them and does not fail them.
        if plan.arming_records {
            push(
                &mut export,
                "daemon",
                "2026-09-04T09:59:30Z",
                record(assistant_record("u6", "u2", "2026-09-04T09:59:30Z")),
            );
            push(
                &mut export,
                "daemon",
                "2026-09-04T09:59:58Z",
                record(assistant_record("u7", "u2", "2026-09-04T09:59:58Z")),
            );
        }

        // ---- work above the declared deadline, and above no seal: the edge the window
        // still enforces, on the run a participant who never seals would hand over.
        if plan.after_deadline {
            push(
                &mut export,
                "daemon",
                "2026-09-04T11:30:00Z",
                record(assistant_record("u8", "u2", "2026-09-04T11:30:00Z")),
            );
        }

        // ---- ten minutes below the start: twice the arming window, which is the whole
        // point — a Flag rather than the Pass-level count, and still never a Fail.
        if plan.ten_minutes_early {
            push(
                &mut export,
                "daemon",
                "2026-09-04T09:50:00Z",
                record(assistant_record("ua", "u2", "2026-09-04T09:50:00Z")),
            );
        }

        // ---- a clock nobody can read. The record itself is well-formed and inside the
        // run; only the daemon's own `recorded_at` is unreadable, so it can be placed
        // neither inside the window nor outside it.
        if plan.unreadable_clock {
            push(
                &mut export,
                "daemon",
                UNREADABLE_RECORDED_AT,
                record(assistant_record("u5", "u2", "2026-09-04T10:02:00Z")),
            );
        }

        // Build the signed receipt before V4 cutoff/finalization so every receipt digest
        // claim can name the canonical bytes that will actually be written.
        let receipt = plan.seed.map(|seed| {
            let ordinary_run_key = hex::encode(
                SigningKey::from_bytes(&[9u8; 32])
                    .verifying_key()
                    .to_bytes(),
            );
            let (run_public_key, questionnaire_version) = match plan.receipt_binding {
                ReceiptBindingPlan::Legacy => (None, None),
                ReceiptBindingPlan::Bound => {
                    (Some(ordinary_run_key), Some("2026.09-v1".to_string()))
                }
                ReceiptBindingPlan::Half => (Some(ordinary_run_key), None),
                ReceiptBindingPlan::Malformed => (
                    Some("not-a-run-key".to_string()),
                    Some("2026.09-v1".to_string()),
                ),
            };
            StartReceipt::sign(
                StartReceiptClaims {
                    entry_id: ENTRY_ID.to_string(),
                    connector_id: CONNECTOR.to_string(),
                    workspace: WORKSPACE.to_string(),
                    session_id: SESSION.to_string(),
                    started_at: match plan.submission_fault {
                        SubmissionFault::StartedAfterCutoff => "2026-09-04T10:05:30Z".to_string(),
                        SubmissionFault::UnparseableStart => "not-a-date".to_string(),
                        _ => STARTED_AT.to_string(),
                    },
                    deadline_at: DEADLINE_AT.to_string(),
                    manifest_sha256: hex::encode(Sha256::digest(&manifest_bytes)),
                    chain_head: genesis.hash.clone(),
                    connector_dir_sha256: connector_dir_sha256.clone(),
                    run_public_key,
                    questionnaire_version,
                },
                &SigningKey::from_bytes(&seed),
            )
            .expect("sign the receipt")
        });

        // ---- the seal. `files` is empty unless the plan asked for one: a sealed file
        // with no traced call behind it is `unexplained_seal_file`, and a compliant trace must
        // not carry another check's finding. `sealed_file` puts the same file in the baseline,
        // so the seal stays silent for as long as the baseline is believed.
        let seal = SealManifest {
            entry_id: ENTRY_ID.to_string(),
            sealed_at: plan.sealed_at.to_string(),
            workspace: WORKSPACE.to_string(),
            files,
            excluded: Vec::new(),
        }
        .sorted();
        let head = if plan.finalization {
            let mut receipt_sha256 = receipt
                .as_ref()
                .expect("a finalized fixture has a receipt")
                .canonical_digest()
                .unwrap();
            if plan.submission_fault == SubmissionFault::ReceiptDigest {
                receipt_sha256 = Sha256Digest::try_from("9".repeat(64)).unwrap();
            }
            let cutoff_deadline = if plan.submission_fault == SubmissionFault::Deadline {
                "2026-09-04T12:00:00Z"
            } else {
                DEADLINE_AT
            };
            let cutoff = export
                .write_cutoff(
                    ENTRY_ID,
                    SEALED_AT,
                    CutoffReachedV1 {
                        receipt_sha256: receipt_sha256.clone(),
                        deadline_at: Rfc3339Utc::try_from(cutoff_deadline).unwrap(),
                        pre_cutoff_drain: PreCutoffDrainOutcomeV1::Complete,
                    },
                )
                .expect("write the cutoff");
            let finalization_at = if plan.workspace_submission.is_some() {
                "2026-09-04T10:05:06Z"
            } else {
                SEALED_AT
            };
            Some(
                export
                    .write_finalization(
                        ENTRY_ID,
                        finalization_at,
                        match plan.workspace_submission {
                            Some(WorkspaceSubmissionPlan::AlreadyStoppedTermination) => {
                                FinalizationV1::WorkspaceSubmission {
                                    receipt_sha256: receipt_sha256.clone(),
                                    termination: TerminationOutcomeV1::AlreadyStopped {
                                        checked_at: Rfc3339Utc::try_from(SEALED_AT).unwrap(),
                                    },
                                    submission: captured_workspace_submission(
                                        &tmp,
                                        &cutoff,
                                        &receipt_sha256,
                                        FinishReasonV1::Manual,
                                        submission_exclusions.clone(),
                                        false,
                                        plan.submission_fault,
                                    ),
                                }
                            }
                            Some(WorkspaceSubmissionPlan::UnresolvedTermination) => {
                                FinalizationV1::WorkspaceSubmission {
                                    receipt_sha256: receipt_sha256.clone(),
                                    termination: TerminationOutcomeV1::Unresolved {
                                        requested_at: Rfc3339Utc::try_from("2026-09-04T10:04:00Z")
                                            .unwrap(),
                                        stopped_waiting_at: Rfc3339Utc::try_from(SEALED_AT)
                                            .unwrap(),
                                        reason: TerminationFailureV1::ProcessAuthorityUnavailable,
                                    },
                                    submission: captured_workspace_submission(
                                        &tmp,
                                        &cutoff,
                                        &receipt_sha256,
                                        FinishReasonV1::Manual,
                                        submission_exclusions.clone(),
                                        false,
                                        plan.submission_fault,
                                    ),
                                }
                            }
                            Some(WorkspaceSubmissionPlan::Incomplete) => {
                                FinalizationV1::WorkspaceSubmission {
                                    receipt_sha256: receipt_sha256.clone(),
                                    termination: TerminationOutcomeV1::Confirmed {
                                        requested_at: Rfc3339Utc::try_from("2026-09-04T10:05:01Z")
                                            .unwrap(),
                                        confirmed_at: Rfc3339Utc::try_from("2026-09-04T10:05:02Z")
                                            .unwrap(),
                                        method: TerminationMethodV1::ProcessSignal,
                                    },
                                    submission: SubmissionOutcomeV1::Incomplete {
                                        reason: SubmissionFailureV1::ReadbackFailed,
                                    },
                                }
                            }
                            Some(WorkspaceSubmissionPlan::Captured) => {
                                FinalizationV1::WorkspaceSubmission {
                                    receipt_sha256: receipt_sha256.clone(),
                                    termination: TerminationOutcomeV1::Confirmed {
                                        requested_at: Rfc3339Utc::try_from(
                                            if plan.submission_fault
                                                == SubmissionFault::TerminationRequestedBeforeCutoff
                                            {
                                                "2026-09-04T10:04:59Z"
                                            } else {
                                                "2026-09-04T10:05:01Z"
                                            },
                                        )
                                        .unwrap(),
                                        confirmed_at: Rfc3339Utc::try_from("2026-09-04T10:05:02Z")
                                            .unwrap(),
                                        method: TerminationMethodV1::ProcessSignal,
                                    },
                                    submission: captured_workspace_submission(
                                        &tmp,
                                        &cutoff,
                                        &receipt_sha256,
                                        if plan.submission_fault == SubmissionFault::FinishReason {
                                            FinishReasonV1::Deadline
                                        } else {
                                            FinishReasonV1::Manual
                                        },
                                        if plan.submission_fault == SubmissionFault::Exclusions {
                                            Vec::new()
                                        } else {
                                            submission_exclusions.clone()
                                        },
                                        plan.submission_unexplained_file,
                                        plan.submission_fault,
                                    ),
                                }
                            }
                            None => FinalizationV1::CaptureUnavailable {
                                receipt_sha256,
                                termination: TerminationOutcomeV1::AlreadyStopped {
                                    checked_at: Rfc3339Utc::try_from(SEALED_AT).unwrap(),
                                },
                                capture: CaptureUnavailableV1::NotAttempted(
                                    SnapshotNotAttemptedV1 {
                                        selection: RepositoryNonSelectionV1::None {
                                            reason: NoRepositoryReasonV1::NotFound,
                                        },
                                    },
                                ),
                                publisher: PublisherNotAttemptedV1 {
                                    reason: PublisherNotAttemptedReasonV1::SelectionFailed,
                                },
                            },
                        },
                    )
                    .expect("write the finalization"),
            )
        } else if plan.unchained_seal {
            // Not through the writer: see `compliant_trace_with_unchained_seal`.
            std::fs::write(
                tmp.path().join(SEAL_FILE),
                serde_json::to_vec(&seal).unwrap(),
            )
            .unwrap();
            None
        } else {
            Some(
                export
                    .write_seal(&seal)
                    .expect("chain and write the seal")
                    .1,
            )
        };

        // ---- one acknowledgement over whatever the chain actually ends with, signed by
        // the pinned key, so V12 has an anchor rather than a note the participant wrote.
        let (ack_seq, ack_hash) = match &head {
            Some(link) => (link.seq, link.hash.clone()),
            None => export.head().expect("the chain has links"),
        };
        let server = SigningKey::from_bytes(&plan.server_seed);
        let mut ack = AckCheckpoint {
            entry_id: ENTRY_ID.to_string(),
            seq: ack_seq,
            hash: ack_hash,
            acked_at: SEALED_AT.to_string(),
            server_signature: String::new(),
        };
        ack.server_signature = hex::encode(server.sign(&ack.signing_bytes().unwrap()).to_bytes());
        export.append_ack(&ack).expect("append the acknowledgement");

        if let Some(receipt) = &receipt {
            export.write_receipt(receipt).expect("write the receipt");
        }

        export
            .write_descriptor(&TraceDescriptorDoc {
                connector: CONNECTOR.to_string(),
                entry_id: ENTRY_ID.to_string(),
                workspace: WORKSPACE.to_string(),
                tracked_sessions: vec![SESSION.to_string()],
                harness_version: "2.1.259".to_string(),
                connector_manifest: "manifest.toml".to_string(),
                chain: Some(CHAIN_FILE.to_string()),
                receipt: plan.seed.map(|_| RECEIPT_FILE.to_string()),
                baseline: match plan.baseline {
                    BaselinePlan::Absent => None,
                    _ => Some(BASELINE_FILE.to_string()),
                },
                acks: Some(ACKS_FILE.to_string()),
                seal: (!plan.finalization).then(|| SEAL_FILE.to_string()),
                server_public_key: Some(hex::encode(
                    SigningKey::from_bytes(&plan.server_seed)
                        .verifying_key()
                        .to_bytes(),
                )),
                expected_reread: false,
            })
            .expect("write the descriptor");

        if plan.rewrite_genesis {
            rewrite_genesis(&tmp.path().join(CHAIN_FILE));
        }
        if plan.submission_fault == SubmissionFault::PostFinalizationWork {
            append_one_link_after_the_seal(&tmp);
        }
        tmp
    }

    /// Chain one more leg-1 record after the seal, correctly linked.
    ///
    /// `TraceExport` refuses this — a seal closes the chain — so it is done here, by hand,
    /// which is the only way a participant could do it either. The link is stamped from the
    /// envelope's own bytes through `Envelope::expected_hash`, so the chain still recomputes
    /// end to end and the only thing wrong with it is that a record follows the seal.
    pub fn append_one_link_after_the_seal(t: &TempDir) {
        let path = t.path().join(CHAIN_FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        let last: Envelope = serde_json::from_str(text.lines().last().unwrap()).unwrap();
        let mut env = Envelope {
            protocol_version: goalcup_protocol::PROTOCOL_VERSION,
            entry_id: ENTRY_ID.to_string(),
            source: "daemon".to_string(),
            recorded_at: "2026-09-04T10:04:00Z".to_string(),
            link: Link {
                seq: last.link.seq + 1,
                prev: last.link.hash.clone(),
                hash: String::new(),
            },
            body: record(assistant_record("u9", "u2", "2026-09-04T10:04:00Z")),
            redaction: None,
        };
        env.link.hash = env.expected_hash(&env.link.prev).unwrap();
        std::fs::write(
            &path,
            format!("{text}{}\n", serde_json::to_string(&env).unwrap()),
        )
        .unwrap();
    }

    /// Add a V2 envelope carrying V1's standalone redaction body, then one ordinary V3
    /// record correctly linked above it. Both writes bypass `TraceExport` because the
    /// fixture is a participant-created false label; its point is that the verifier retains
    /// neither line once the first one is rejected.
    pub fn append_v2_legacy_redaction_and_valid_suffix(t: &TempDir) {
        let path = t.path().join(CHAIN_FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        let previous: Envelope = serde_json::from_str(text.lines().last().unwrap()).unwrap();
        let mut incompatible = Envelope {
            protocol_version: goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
            entry_id: ENTRY_ID.to_string(),
            source: "daemon".to_string(),
            recorded_at: "2026-09-04T10:05:01Z".to_string(),
            link: Link {
                seq: previous.link.seq + 1,
                prev: previous.link.hash.clone(),
                hash: String::new(),
            },
            body: EventBody::Redaction(RedactionRecord {
                store: "main".to_string(),
                target_seq: 1,
                salt: "legacy".to_string(),
                spans: Vec::new(),
            }),
            redaction: None,
        };
        incompatible.link.hash = incompatible.expected_hash(&incompatible.link.prev).unwrap();
        let mut suffix = Envelope {
            protocol_version: goalcup_protocol::PROTOCOL_VERSION,
            entry_id: ENTRY_ID.to_string(),
            source: "daemon".to_string(),
            recorded_at: "2026-09-04T10:05:02Z".to_string(),
            link: Link {
                seq: incompatible.link.seq + 1,
                prev: incompatible.link.hash.clone(),
                hash: String::new(),
            },
            body: EventBody::Leg1Record {
                store: "main".to_string(),
                payload: serde_json::Value::String(
                    r#"{"type":"user","message":{"content":"suffix-must-not-reach-checks"}}"#
                        .to_string(),
                ),
            },
            redaction: None,
        };
        suffix.link.hash = suffix.expected_hash(&suffix.link.prev).unwrap();
        std::fs::write(
            path,
            format!(
                "{text}{}\n{}\n",
                serde_json::to_string(&incompatible).unwrap(),
                serde_json::to_string(&suffix).unwrap(),
            ),
        )
        .unwrap();
    }

    /// Rewrite one line's `protocol_version` in the file. The version is inside
    /// `chain_bytes`, so this also moves that link's hash — which is honest: a participant
    /// who edits a shipped envelope breaks its link, and V13 says both things.
    pub fn bump_protocol_version(t: &TempDir, line: usize, version: u32) {
        let path = t.path().join(CHAIN_FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        let mut env: Envelope = serde_json::from_str(&lines[line]).unwrap();
        env.protocol_version = version;
        lines[line] = serde_json::to_string(&env).unwrap();
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    }

    /// A write that did not finish: bytes on the final line with no newline after them.
    pub fn append_unterminated_bytes(t: &TempDir) {
        let path = t.path().join(CHAIN_FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}{{\"protocol_version\":1,\"entry")).unwrap();
    }

    /// Edit seq 0's payload in the file and re-stamp its own hash over the edited bytes.
    ///
    /// Re-stamped rather than left alone on purpose. Leaving the recorded hash makes the
    /// break a `HashMismatch` at seq 0 while `genesis_link` — which is that recorded hash
    /// — keeps matching the receipt, so the fixture would exercise one statement and not
    /// the other. A participant rewriting their own genesis recomputes it, which moves
    /// `genesis_link` away from the witnessed head and leaves seq 1's `prev` naming a
    /// link that no longer exists.
    fn rewrite_genesis(chain: &std::path::Path) {
        let text = std::fs::read_to_string(chain).unwrap();
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        let mut env: Envelope = serde_json::from_str(&lines[0]).unwrap();
        env.body = record(user_record(
            "u1",
            "2026-09-04T10:00:10Z",
            "[rewritten by the participant after the fact]",
        ));
        env.link.hash = env.expected_hash(&env.link.prev).unwrap();
        lines[0] = serde_json::to_string(&env).unwrap();
        std::fs::write(chain, lines.join("\n") + "\n").unwrap();
    }
}
