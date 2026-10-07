//! The adversarial acceptance test.
//!
//! Four assertions, in this order:
//!   1. regenerating the fixture from the committed Claude bytes reproduces the checked-in
//!      copy exactly — so the fixture is reviewable in a diff and cannot drift silently;
//!   2. `verify_trace` over it produces exactly the four tamper findings and no others;
//!   3. the committed `expected-verdict.json` is what `verify_trace` produces, so the
//!      expectation lives in data rather than only in this file's literals;
//!   4. the verdict is byte-identical across two runs.
//!
//! Every set of finding codes below is built from `Verdict::review_queue`, never from
//! `Verdict::checks`: the two seal findings are raised outside any check, so `checks` is
//! not the complete list.

mod support;

use goalcup_verifier::checks::Severity;
use goalcup_verifier::verify_trace;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn committed() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/adversarial")
}

fn contained() -> tempfile::TempDir {
    support::materialize_trace(&committed())
}

/// The byte offset the planted `TailNotice` says `main` fell from, read out of the chain.
///
/// There is no `main.jsonl` beside the descriptor to measure: this is a pure chain trace,
/// and the store lives inside `chain.jsonl` as leg-1 envelopes. The number is still
/// derived rather than retyped — it comes from the one envelope that
/// states it, which is also the envelope V9 reads.
fn truncation_offset() -> u64 {
    use goalcup_protocol::envelope::{Envelope, EventBody};
    let text = std::fs::read_to_string(committed().join("chain.jsonl")).unwrap();
    let mut found: Option<u64> = None;
    for line in text.lines() {
        let env: Envelope = serde_json::from_str(line).unwrap();
        if let EventBody::TailNotice { offset_before, .. } = env.body {
            assert!(
                found.is_none(),
                "the fixture plants exactly one tail notice"
            );
            found = Some(offset_before);
        }
    }
    found.expect("the chain carries the planted tail notice")
}

/// The tamper-class findings: the ones a compliant capture cannot produce. Everything else
/// in this verdict is inherited from the Claude script the fixture derives from.
const TAMPER_CODES: [&str; 6] = [
    "ack_chain_mismatch",
    "ack_signature_invalid",
    "store_rewritten",
    "unexplained_seal_file",
    "read_only_attribution",
    "dirty_gap",
];

#[test]
fn the_checked_in_fixture_is_what_the_generator_produces() {
    let tmp = tempfile::tempdir().unwrap();
    support::adversarial::generate(tmp.path());
    for name in support::adversarial::FILES {
        let want = std::fs::read(committed().join(name))
            .unwrap_or_else(|e| panic!("{name}: {e} — run the `regenerate` test"));
        let got = std::fs::read(tmp.path().join(name)).unwrap();
        assert_eq!(
            want.len(),
            got.len(),
            "{name}: the generator and the committed fixture disagree in length"
        );
        assert!(
            want == got,
            "{name}: the generator no longer produces the committed bytes"
        );
    }
}

#[test]
fn the_generator_replaces_only_its_outputs_on_a_second_pass() {
    let tmp = tempfile::tempdir().unwrap();
    support::adversarial::generate(tmp.path());
    let first: std::collections::BTreeMap<&str, Vec<u8>> = support::adversarial::FILES
        .into_iter()
        .map(|name| (name, std::fs::read(tmp.path().join(name)).unwrap()))
        .collect();
    let sentinel = tmp.path().join("unrelated-sentinel.txt");
    std::fs::write(&sentinel, b"preserve me").unwrap();

    support::adversarial::generate(tmp.path());

    assert_eq!(std::fs::read(sentinel).unwrap(), b"preserve me");
    for name in support::adversarial::FILES {
        let second = std::fs::read(tmp.path().join(name)).unwrap();
        assert_eq!(second, first[name], "{name} changed between passes");
        assert_eq!(
            second,
            std::fs::read(committed().join(name)).unwrap(),
            "{name} differs from the committed historical fixture"
        );
    }
}

#[test]
fn verify_produces_exactly_the_four_tamper_findings() {
    let trace = contained();
    let v = verify_trace(trace.path()).unwrap();
    assert_eq!(v.overall, Severity::Fail);
    assert_eq!(v.exit_code(), 2);
    // The acknowledgements are checked against the verifier's own allowlist, not against a
    // key the participant chose. Without this the `ack_chain_mismatch`
    // below would be one self-signed note disagreeing with another.
    assert_eq!(v.key_source, goalcup_verifier::KeySource::Pinned);

    // Read the list the emitters write to. `unexplained_seal_file` and
    // `read_only_attribution` are raised by `verify_trace` from `seal_attribution`, not by
    // any check, so they exist in `review_queue` and in no `checks[].findings`.
    // `review_queue` is the union of both, so it is the only set that can contain all six
    // `TAMPER_CODES`; building this from `v.checks` makes a correct implementation fail.
    let codes: BTreeSet<&str> = v.review_queue.iter().map(|f| f.code.as_str()).collect();
    let tamper: BTreeSet<&str> = codes
        .iter()
        .copied()
        .filter(|c| TAMPER_CODES.contains(c))
        .collect();

    let want: BTreeSet<&str> = [
        "ack_chain_mismatch",
        "store_rewritten",
        "unexplained_seal_file",
        "dirty_gap",
    ]
    .into_iter()
    .collect();
    assert_eq!(
        tamper, want,
        "the four planted signatures, and nothing else"
    );

    // Each one, with the reason it is there.
    let v12 = v.checks.iter().find(|c| c.id == "V12").unwrap();
    assert_eq!(v12.verdict, Severity::Fail);
    let mismatch = v12
        .findings
        .iter()
        .find(|f| f.code == "ack_chain_mismatch")
        .unwrap();
    // The participant rewrote one assistant record deep inside the acknowledged range.
    // The chain recomputes cleanly over the doctored file, so nothing in the trace itself
    // objects. What catches it is the server's word about seq ANCHOR_SEQ, which the
    // rewrite moved out from under.
    assert!(
        mismatch
            .message
            .contains(&support::adversarial::ANCHOR_SEQ.to_string()),
        "the mismatch is reported at the acknowledged sequence: {}",
        mismatch.message
    );
    assert_eq!(mismatch.severity, Severity::Fail);
    // Exactly one acknowledgement fails: the one above the rewrite. The one below it still
    // reconciles, which is what places the participant's edit between the two.
    assert_eq!(
        v12.findings
            .iter()
            .filter(|f| f.code == "ack_chain_mismatch")
            .count(),
        1,
        "only the acknowledgement above the rewrite may fail"
    );
    // The signature verifies — the server really did sign it — which is what makes the
    // hash mismatch damning rather than ambiguous.
    assert!(
        !v12.findings
            .iter()
            .any(|f| f.code == "ack_signature_invalid")
    );
    assert!(!v12.findings.iter().any(|f| f.code == "no_anchor"));

    let v9 = v.checks.iter().find(|c| c.id == "V9").unwrap();
    let notice = v9
        .findings
        .iter()
        .find(|f| f.code == "store_rewritten")
        .unwrap();
    assert_eq!(notice.severity, Severity::Fail);
    assert!(
        notice.message.contains("truncated below the saved offset"),
        "{}",
        notice.message
    );
    // The offset it fell from is the one the chain's `TailNotice` states — read back from
    // the envelope, not retyped, so no literal can drift away from the fixture.
    let store_len = truncation_offset();
    assert!(
        notice.message.contains(&store_len.to_string()),
        "the offset it fell from ({store_len}): {}",
        notice.message
    );

    let v4 = v.checks.iter().find(|c| c.id == "V4").unwrap();
    let gap = v4.findings.iter().find(|f| f.code == "dirty_gap").unwrap();
    assert_eq!(gap.severity, Severity::Fail);
    assert!(
        !gap.evidence.is_empty(),
        "a dirty gap must name the lines appended in it"
    );
    assert!(
        !v4.findings.iter().any(|f| f.code == "clean_gap"),
        "this hole is not clean: the store grew while the daemon was not watching"
    );

    // The seal: one file traced, one explained by nothing. Both halves are asserted. A
    // regression that stopped attributing anything would leave `unexplained_seal_file` in
    // the queue and pass every code-set assertion in this file, so the fixture has to say
    // that the attributor still *distinguishes* the two files rather than failing them
    // both. Located by path, never by index: `SealManifest::sorted` fixes the order today,
    // and an index would silently follow it if it ever changed.
    assert_eq!(v.seal_attribution.len(), 2);
    let wrote_it = v
        .seal_attribution
        .iter()
        .find(|a| a.path == "hello.py")
        .expect("the seal lists hello.py");
    assert!(
        matches!(
            wrote_it.attribution,
            goalcup_verifier::attribution::Attribution::Traced { .. }
        ),
        "hello.py is explained by a traced shell call and must stay Traced: {:?}",
        wrote_it.attribution
    );
    let planted = v
        .seal_attribution
        .iter()
        .find(|a| a.path == "planted.txt")
        .expect("the seal lists planted.txt");
    assert_eq!(
        planted.attribution,
        goalcup_verifier::attribution::Attribution::Unexplained
    );
    assert!(
        v.review_queue
            .iter()
            .any(|f| f.code == "unexplained_seal_file" && f.message.contains("planted.txt"))
    );

    // And the gap section of the verdict agrees with the check that read it. Pinned to
    // `is_hole`: without it this passes on any dirty window at all, including the
    // unanchored stretch above the last acknowledgement, and stops being a statement about
    // the planted heartbeat hole.
    assert!(v.gaps.iter().any(|g| {
        g.kind == goalcup_verifier::gaps::GapKind::Dirty && !g.appended.is_empty() && g.is_hole
    }));
}

/// The committed `expected-verdict.json` is data, not decoration: read it, the same way
/// `end_to_end.rs`'s `assert_matches_expected` reads the three M0 ones. Without this the
/// committed expected verdict is checked by nothing and can drift away from the four codes
/// hardcoded above.
///
/// Every code set here comes from `review_queue`: the seal findings live there and in no
/// check.
#[test]
fn the_committed_expected_verdict_is_what_verify_produces() {
    let trace = contained();
    let v = verify_trace(trace.path()).unwrap();
    let text = std::fs::read_to_string(committed().join("expected-verdict.json")).unwrap();
    let want: serde_json::Value = serde_json::from_str(&text).unwrap();

    assert_eq!(serde_json::to_value(v.overall).unwrap(), want["overall"]);
    assert_eq!(v.exit_code() as i64, want["exit_code"].as_i64().unwrap());
    assert_eq!(
        v.human_messages.len() as u64,
        want["human_message_count"].as_u64().unwrap()
    );
    assert_eq!(
        v.human_messages[0].text,
        want["first_human_message"].as_str().unwrap()
    );

    // The parser census, which this fixture inherits unchanged from the M0 Claude capture
    // it chains. Read here so a change to what the parser drops is seen on the adversarial
    // trace too, and not only on the three M0 ones.
    assert_eq!(
        serde_json::to_value(&v.parser_census).unwrap(),
        want["parser_census"]
    );
    // The chain `TraceExport` writes ends in a newline, so nothing is half-written and the
    // tail is silent. Asserted rather than assumed: `truncated_tail` is a Flag, and a
    // Flag that fired here would be a fifth code in a file whose whole point is that
    // exactly four attacks are planted.
    assert!(!v.review_queue.iter().any(|f| f.code == "truncated_tail"));

    // The `checks` map as a key set first, then value by value: iterating the file and
    // looking each key up in the verdict asks only that the checks the file names still
    // exist, so deleting an expectation from the file would keep this test green.
    let expected_checks: BTreeSet<&str> = want["checks"]
        .as_object()
        .unwrap()
        .keys()
        .map(|s| s.as_str())
        .collect();
    let got_checks: BTreeSet<&str> = v.checks.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        expected_checks, got_checks,
        "the expectation must name every check, and only those"
    );
    for (check_id, verdict) in want["checks"].as_object().unwrap() {
        let got = v
            .checks
            .iter()
            .find(|c| &c.id == check_id)
            .unwrap_or_else(|| panic!("no check {check_id}"));
        assert_eq!(
            &serde_json::to_value(got.verdict).unwrap(),
            verdict,
            "{check_id}: {:?}",
            got.findings
        );
    }

    let codes: BTreeSet<&str> = v.review_queue.iter().map(|f| f.code.as_str()).collect();
    // Set equality, not containment — the same rule `end_to_end.rs` uses.
    // A containment check passes when the verifier grows a finding
    // nobody expected, and on the one trace in this repo that is *supposed* to be
    // suspicious that is exactly the direction that matters: a new code here is a new
    // claim about the planted bytes and has to be read in a diff.
    let want_codes: BTreeSet<&str> = want["finding_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        want_codes,
        "review-queue codes differ; missing {:?}, unexpected {:?}",
        want_codes.difference(&codes).collect::<Vec<_>>(),
        codes.difference(&want_codes).collect::<Vec<_>>()
    );
    for forbidden in want["must_not_contain"].as_array().unwrap() {
        let c = forbidden.as_str().unwrap();
        // A forbidden code that no build can emit forbids nothing. This fixture's list is
        // the longest in the tree — ten codes the planted attacks must not produce — and
        // every one of them has to name a real code for the list to mean anything.
        assert!(
            support::pipeline::is_an_emittable_code(c),
            "`{c}` names no code any build can emit, so forbidding it guards nothing"
        );
        assert!(!codes.contains(c), "finding `{c}` must not appear");
    }
    // `tamper_findings` is the subset the four planted attacks produce, and it must equal
    // exactly what the assertion above hardcodes — one list, checked against the other.
    let want_tamper: BTreeSet<&str> = want["tamper_findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    let got_tamper: BTreeSet<&str> = codes
        .iter()
        .copied()
        .filter(|c| TAMPER_CODES.contains(c))
        .collect();
    assert_eq!(got_tamper, want_tamper);

    let all_messages: String = v
        .review_queue
        .iter()
        .map(|f| f.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    for needle in want["must_name"].as_array().unwrap() {
        let n = needle.as_str().unwrap();
        assert!(all_messages.contains(n), "the verdict must name `{n}`");
    }
}

#[test]
fn the_adversarial_verdict_is_deterministic() {
    let first = contained();
    let second = contained();
    let a = verify_trace(first.path()).unwrap().to_json().unwrap();
    let b = verify_trace(second.path()).unwrap().to_json().unwrap();
    assert_eq!(a, b);
    assert!(
        !a.contains(env!("CARGO_MANIFEST_DIR")),
        "no local path may leak"
    );
}

/// Rewrites the committed fixture.
///
/// Two locks, not one. `#[ignore]` keeps it out of an ordinary `cargo test`, but
/// `cargo test -- --include-ignored` would run it, overwriting the checked-in adversarial
/// bytes and
/// then asserting they matched what had just been written. So the regeneration itself is
/// gated on an environment variable a person types on purpose: under
/// `--include-ignored` alone this returns early, having written nothing, and says why.
#[test]
#[ignore = "writes the committed fixture; set GOALCUP_REGEN=1; see scripts/regen-adversarial-fixture.md"]
fn regenerate() {
    if std::env::var("GOALCUP_REGEN").as_deref() != Ok("1") {
        println!(
            "regenerate: GOALCUP_REGEN=1 is not set, so the committed fixture at {} was \
             left alone. Run `GOALCUP_REGEN=1 cargo test -p goalcup-verifier --test \
             adversarial -- --ignored regenerate`, read the diff, then commit.",
            committed().display()
        );
        return;
    }
    support::adversarial::generate(&committed());
}
