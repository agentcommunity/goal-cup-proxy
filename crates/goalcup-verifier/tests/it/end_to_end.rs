//! End-to-end: `verify_trace` over the three committed M0 trace directories, compared
//! against the `expected-verdict.json` committed beside each descriptor.

use crate::common as support;

use goalcup_verifier::checks::Severity;
use goalcup_verifier::verify_trace;
use std::collections::BTreeSet;
use std::path::PathBuf;
use support::pipeline::is_an_emittable_code;

/// The adversarial fixture, reached from `tests/traces` the way `dir` reaches the M0
/// ones. It lives beside them rather than under them because it is generated, not
/// captured.
const ADVERSARIAL: &str = "../fixtures/adversarial";

fn dir(id: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/traces")
        .join(id)
}

fn verify_fixture(id: &str) -> goalcup_verifier::Verdict {
    let trace = support::materialize_trace(&dir(id));
    verify_trace(trace.path()).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn pipeline_fixture(id: &str) -> goalcup_verifier::Pipeline {
    let trace = support::materialize_trace(&dir(id));
    support::pipeline::at(trace.path())
}

/// The `expected-verdict.json` committed beside a descriptor.
fn expectation(id: &str) -> serde_json::Value {
    let path = dir(id).join("expected-verdict.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap()
}

/// Compare a verdict against the `expected-verdict.json` committed beside its descriptor.
///
/// Inline field checks are kept where they document a specific measured fact; this one is
/// what stops a wrong expectation hiding in a scattering of `assert_eq!`.
fn assert_matches_expected(id: &str) -> goalcup_verifier::verdict::Verdict {
    let v = verify_fixture(id);
    let path = dir(id).join("expected-verdict.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let want: serde_json::Value = serde_json::from_str(&text).unwrap();

    assert_eq!(
        serde_json::to_value(v.overall).unwrap(),
        want["overall"],
        "{id}: overall verdict"
    );
    assert_eq!(
        v.exit_code() as i64,
        want["exit_code"].as_i64().unwrap(),
        "{id}: exit code"
    );
    assert_eq!(
        v.human_messages.len() as u64,
        want["human_message_count"].as_u64().unwrap(),
        "{id}: human message count"
    );
    assert_eq!(
        v.human_messages[0].text,
        want["first_human_message"].as_str().unwrap(),
        "{id}: the goal"
    );

    // `tokens`, where the expectation names it. Optional because only the
    // codex descriptor carries a measured per-model split; where it is named, both
    // the split and the totals are compared, so neither a wrong model attribution nor a
    // wrong counter can hide behind a right grand total.
    if let Some(tokens) = want.get("tokens") {
        assert_eq!(
            serde_json::to_value(&v.tokens.by_model).unwrap(),
            tokens["by_model"],
            "{id}: tokens.by_model"
        );
        let sum = |f: fn(&goalcup_verifier::verdict::ModelTokenTotals) -> u64| -> u64 {
            v.tokens.by_model.iter().map(f).sum()
        };
        assert_eq!(
            sum(|t| t.prompt),
            tokens["totals"]["prompt"].as_u64().unwrap(),
            "{id}: prompt tokens"
        );
        assert_eq!(
            sum(|t| t.completion),
            tokens["totals"]["completion"].as_u64().unwrap(),
            "{id}: completion tokens"
        );
        assert_eq!(
            sum(|t| t.cached),
            tokens["totals"]["cached"].as_u64().unwrap(),
            "{id}: cached tokens"
        );
    }

    // The parser's census of record kinds it read nothing out of. Compared as a whole
    // map, not a total: a kind moving from one bucket to another keeps the sum and is
    // exactly the change a reviewer needs to see. It is a statement and not a finding, so
    // it must never appear in `finding_codes` — asserted below, after the codes are read.
    assert_eq!(
        serde_json::to_value(&v.parser_census).unwrap(),
        want["parser_census"],
        "{id}: parser_census"
    );

    // The `checks` map as a key set, then value by value. Iterating the expectation and
    // looking each key up in the verdict would ask only that every check the file names
    // still exists: deleting `"V9": "flag"` from the file would leave the suite green, and so
    // would deleting all thirteen. The set is compared in both directions, so an expectation
    // that drops a check, or a build that grows one, fails.
    let expected_checks: BTreeSet<&str> = want["checks"]
        .as_object()
        .unwrap()
        .keys()
        .map(|s| s.as_str())
        .collect();
    let got_checks: BTreeSet<&str> = v.checks.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        expected_checks, got_checks,
        "{id}: the expectation must name every check, and only those"
    );
    for (check_id, verdict) in want["checks"].as_object().unwrap() {
        let got = v
            .checks
            .iter()
            .find(|c| &c.id == check_id)
            .unwrap_or_else(|| panic!("{id}: no check {check_id}"));
        assert_eq!(
            &serde_json::to_value(got.verdict).unwrap(),
            verdict,
            "{id}/{check_id}: {:?}",
            got.findings
        );
    }

    // `review_queue` is the union of every check's findings and the seal findings
    // `verify_trace` raises outside any check (`unexplained_seal_file`,
    // `read_only_attribution`). Reading `v.checks` here would silently
    // exclude those two codes and make a correct implementation fail the assertion.
    // One list is written, one list is read: this one.
    let codes: BTreeSet<&str> = v.review_queue.iter().map(|f| f.code.as_str()).collect();
    // Set equality, not containment. A subset check passes
    // when the verifier grows a finding nobody expected, which is the one direction a
    // committed expectation exists to catch: a new code appearing on a fixture is a claim
    // about the bytes and has to be reviewed in a diff, not absorbed silently.
    let want_codes: BTreeSet<&str> = want["finding_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        want_codes,
        "{id}: review-queue codes differ; missing {:?}, unexpected {:?}",
        want_codes.difference(&codes).collect::<Vec<_>>(),
        codes.difference(&want_codes).collect::<Vec<_>>()
    );
    for forbidden in want["must_not_contain"].as_array().unwrap() {
        let c = forbidden.as_str().unwrap();
        // The polarity is only worth checking once the code exists: a `must_not_contain`
        // entry naming a code no build can emit forbids nothing, and reads in a diff as a
        // guard that is really a comment.
        assert!(
            is_an_emittable_code(c),
            "{id}: `{c}` names no code any build can emit, so forbidding it guards nothing"
        );
        assert!(!codes.contains(c), "{id}: finding `{c}` must not appear");
    }
    // A dropped record kind is not evidence of anything, so no census key is ever a
    // finding code and the census never moves `overall`.
    for kind in v.parser_census.keys() {
        assert!(
            !codes.contains(kind.as_str()),
            "{id}: the census is a statement, and `{kind}` reached the review queue"
        );
    }
    // Every sentence the verdict says, at any severity. The review queue is a subset of what
    // is read here; what the key asserts is what its own failure message says, that the
    // verdict names the thing.
    let all_messages: String = v
        .checks
        .iter()
        .flat_map(|c| c.findings.iter())
        .chain(v.review_queue.iter())
        .map(|f| f.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    for needle in want["must_name"].as_array().unwrap() {
        let n = needle.as_str().unwrap();
        // An empty needle is contained in every string, so it would name nothing while
        // reading as a requirement.
        assert!(
            !n.is_empty(),
            "{id}: an empty `must_name` entry names nothing"
        );
        assert!(
            all_messages.contains(n),
            "{id}: the verdict must name `{n}`"
        );
    }

    // No absolute machine path anywhere in the document (see the header of `verdict.rs`).
    // Asserted over the whole serialised verdict rather than over the
    // fields a reviewer happens to think of: a home directory can reach the document
    // through a finding message, a seal path, an evidence note or a human message, and a
    // guard that names its own hiding places is not a guard.
    // `verdicts_are_byte_identical_across_two_runs` checks only this checkout's own
    // manifest directory, which a verdict published from another machine would not carry.
    // Two codex-cli findings, `goal_recovered` and `leg1_without_leg2_post`, are exempt (the
    // same two `checks_v1_v5.rs::no_finding_carries_an_absolute_machine_path` exempts):
    // Codex's synthetic `<environment_context>` header becomes the unwitnessed goal, and its
    // own text, carrying the real `<cwd>`, reaches exactly those two findings. Their
    // messages are stripped out of the serialised document before the whole-document
    // search runs, rather than skipping the search for the whole fixture. Every other
    // finding, on every fixture, still must not leak one.
    let mut json = v.to_json().unwrap();
    if id == "codex-cli" {
        for check in &v.checks {
            for f in &check.findings {
                if matches!(f.code.as_str(), "goal_recovered" | "leg1_without_leg2_post") {
                    // The message as it appears inside the serialised document (JSON
                    // escaped: a literal newline becomes `\n`), not the raw Rust string —
                    // `json.replace(&f.message, "")` would never match a byte-for-byte
                    // newline against its own escape sequence.
                    let encoded = serde_json::to_string(&f.message).unwrap();
                    let inner = &encoded[1..encoded.len() - 1];
                    json = json.replace(inner, "");
                }
            }
        }
    }
    for prefix in ["/Users/", "/home/", "/private/"] {
        assert!(
            !json.contains(prefix),
            "{id}: the verdict leaks an absolute path beginning `{prefix}`"
        );
    }
    v
}

#[test]
fn claude_code_verdict() {
    let v = assert_matches_expected("claude-code-cli");
    assert_eq!(v.connector_id, "claude-code-cli");
    assert_eq!(v.entry_id, "m0-spike4");
    assert_eq!(v.harness_version, "2.1.259");
    assert_eq!(v.schema_version, "ATIF-v1.8");
    assert_eq!(v.checks.len(), 13);
    let ids: Vec<&str> = v.checks.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "V1", "V2", "V3", "V4", "V5", "V6", "V7", "V8", "V9", "V10", "V11", "V12", "V13"
        ]
    );
    // V13 runs on an M0 capture and says why it has nothing to bind: this descriptor
    // declares [[leg1]] stores and no chain, so it carries no start receipt. `legacy_trace`
    // is a Pass and never enters `finding_codes` — a Pass finding is never a queue entry.
    //
    // One finding, and one only, with no `connector_dir_unbound` Flag beside it: every
    // statement V13 makes about the connector directory is
    // read out of a receipt whose signature verified, and a capture carrying no receipt has
    // no such document. `legacy_trace` already says no start was carried.
    let v13 = v.checks.iter().find(|c| c.id == "V13").unwrap();
    assert_eq!(v13.verdict, Severity::Pass);
    assert_eq!(
        v13.findings
            .iter()
            .map(|f| f.code.as_str())
            .collect::<Vec<_>>(),
        vec!["legacy_trace"]
    );
    assert_eq!(v.checks[0].verdict, Severity::Pass);
    assert_eq!(v.checks[1].verdict, Severity::Fail);
    assert!(v.review_queue.iter().any(|f| f.code == "not_keep_going"));
    assert!(v.review_queue.iter().any(|f| f.code == "foreign_session"));
    // V10 is a Pass here: `foreign_records_present` is Pass-level, because it states that
    // records were correctly partitioned away. The finding, and the store it cites, are
    // still in `Verdict::checks`.
    let v10 = v.checks.iter().find(|c| c.id == "V10").unwrap();
    assert_eq!(v10.verdict, Severity::Pass, "{:?}", v10.findings);
    assert!(
        v10.findings
            .iter()
            .any(|f| f.code == "foreign_records_present" && !f.evidence.is_empty()),
        "{:?}",
        v10.findings
    );
    // An M0 trace has no anchor, and V12 says so rather than staying silent.
    assert!(v.review_queue.iter().any(|f| f.code == "no_anchor"));
    assert!(
        v.gaps.is_empty(),
        "no acknowledgements and no heartbeat series in an M0 trace"
    );
    assert!(
        v.seal_attribution.is_empty(),
        "no M0 spike produced a SealManifest"
    );
    assert!(!v.tokens.by_model.is_empty());
}

#[test]
fn codex_verdict_names_the_going_typo_and_the_readme() {
    let v = assert_matches_expected("codex-cli");
    assert_eq!(v.human_messages.len(), 7);
    let v2 = v.checks.iter().find(|c| c.id == "V2").unwrap();
    assert_eq!(v2.verdict, Severity::Fail);
    let msgs: Vec<&str> = v2.findings.iter().map(|f| f.message.as_str()).collect();
    // Five, not four: nothing distinguishes Codex's synthetic `<environment_context>` header
    // from a real message, so it occupies index 0 (V1's problem, not V2's), which bumps every
    // real message up one index and leaves the actual first typed message, `create
    // hello.py...`, wrongly judged as message 1.
    assert_eq!(msgs.len(), 5, "{msgs:?}");
    assert!(
        msgs.iter()
            .any(|m| m.contains("message 1 is `create hello.py")),
        "the interim reading: the real goal is no longer recognised as the goal"
    );
    assert!(
        msgs.iter().any(|m| m.contains("message 3 is `going`")),
        "the typo, not normalised"
    );
    assert!(
        msgs.iter()
            .any(|m| m.contains("message 2 is `use a subagent to count the lines in hello.py`"))
    );
    assert!(
        msgs.iter()
            .any(|m| m.contains("message 6 is `what is 2 + 2?`"))
    );
    assert!(
        msgs.iter()
            .any(|m| m.contains("message 7 is `also add a README`"))
    );

    // No finding may claim "the id spaces never intersect": that assertion is false — two
    // `collaboration*` calls share `call_` ids across both legs.
    let v8 = v.checks.iter().find(|c| c.id == "V8").unwrap();
    assert!(
        !v8.findings
            .iter()
            .any(|f| f.code == "disjoint_tool_id_spaces")
    );
    // No V8 finding counts the overlap of tool id spaces (`tests/it/checks_v6_v11.rs` asserts
    // its absence): V8 pairs calls per turn. The measured fact — two shared `call_` ids — is
    // reported by V3 instead, so the overlap is still asserted, from where the number lives.
    let v3 = v.checks.iter().find(|c| c.id == "V3").unwrap();
    assert!(
        v3.findings
            .iter()
            .any(|f| f.code == "cross_leg_identifiers" && f.message.contains("2 shared")),
        "{:?}",
        v3.findings
    );
    assert_eq!(
        v8.findings
            .iter()
            .find(|f| f.code == "turn_tool_call_count_mismatch")
            .map(|f| f.message.contains("3 leg-1") && f.message.contains("6 leg-2")),
        Some(true),
        "the three `exec` macro calls that fan out into six leg-2 records"
    );

    // The mis-launched thread is reported as foreign, not as a second root, because
    // its excerpt carries no `session_meta` to establish a session from.
    let v4 = v.checks.iter().find(|c| c.id == "V4").unwrap();
    assert!(v4.findings.iter().any(|f| f.code == "foreign_session"));
    assert!(!v4.findings.iter().any(|f| f.code == "second_root_thread"));
}

/// The connector manifest is a participant-supplied input, and the verdict records
/// which bytes of it were read.
///
/// `trace.toml` names a connector — which is what selects the parser — and, separately, a
/// path to a manifest, which is what supplies the human rule, the exclusion catalogue and
/// the tool classes. Both are written by the participant. Nothing bound them together, and
/// nothing recorded the second one, so a trace could run Claude Code's parser over a
/// hand-written rule file and the verdict would say `connector_id: "claude-code-cli"` with
/// no way for a reviewer to tell.
mod connector_manifest {
    use super::dir;
    use goalcup_verifier::verify_trace;
    use sha2::{Digest, Sha256};
    use std::path::{Path, PathBuf};

    fn adversarial() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/adversarial")
    }

    /// `sha256sum` of a file, computed here rather than read from the verdict.
    fn digest_of(path: &Path) -> String {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut h = Sha256::new();
        h.update(&bytes);
        hex::encode(h.finalize())
    }

    /// The manifest `trace.toml` points at, resolved the way `TraceDescriptor::load`
    /// resolves it: relative to the descriptor's own directory.
    fn manifest_of(trace_dir: &Path) -> PathBuf {
        let text = std::fs::read_to_string(trace_dir.join("trace.toml")).unwrap();
        let doc: toml::Value = toml::from_str(&text).unwrap();
        trace_dir.join(doc["connector_manifest"].as_str().unwrap())
    }

    #[test]
    fn every_trace_records_the_digest_of_the_manifest_it_actually_read() {
        for source in [dir("claude-code-cli"), dir("codex-cli"), adversarial()] {
            let fixture = super::support::materialize_trace(&source);
            let trace_dir = fixture.path();
            let v =
                verify_trace(trace_dir).unwrap_or_else(|e| panic!("{}: {e}", trace_dir.display()));
            let got = &v.connector_manifest_sha256;
            assert_eq!(got.len(), 64, "{}: {got}", trace_dir.display());
            assert!(
                got.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
                "{}: lowercase hex only, got {got}",
                trace_dir.display()
            );
            assert_eq!(
                *got,
                digest_of(&manifest_of(trace_dir)),
                "{}: the recorded digest must be sha256sum of the manifest file",
                trace_dir.display()
            );
        }
    }

    /// A descriptor whose `connector` disagrees with the manifest it points at is refused,
    /// and the error names both. Proceeding would mean choosing which half of the
    /// descriptor to believe — the parser named by one field, or the rules named by the
    /// other — and a verifier that guesses is not a verifier.
    #[test]
    fn a_connector_that_disagrees_with_its_manifest_is_refused() {
        let fixture = super::support::materialize_trace(&dir("claude-code-cli"));
        let descriptor = fixture.path().join("trace.toml");
        let text = std::fs::read_to_string(&descriptor).unwrap();
        let swapped = text.replace(
            "connector = \"claude-code-cli\"",
            "connector = \"codex-cli\"",
        );
        assert!(swapped.contains("connector = \"codex-cli\""), "{swapped}");
        std::fs::write(&descriptor, &swapped).unwrap();

        let err = verify_trace(fixture.path()).expect_err("the trace contradicts itself");
        let message = err.to_string();
        assert!(
            message.contains("codex-cli") && message.contains("claude-code-cli"),
            "the refusal must name both: {message}"
        );

        // The control: the same copy, with the two left agreeing, verifies.
        let honest = swapped.replace(
            "connector = \"codex-cli\"",
            "connector = \"claude-code-cli\"",
        );
        std::fs::write(&descriptor, &honest).unwrap();
        verify_trace(fixture.path()).expect("only the mismatch was wrong");
    }
}

#[test]
fn verdicts_are_byte_identical_across_two_runs() {
    for id in ["claude-code-cli", "codex-cli"] {
        let a = verify_fixture(id).to_json().unwrap();
        let b = verify_fixture(id).to_json().unwrap();
        assert_eq!(a, b, "{id}: the verdict must be deterministic (PRD V4)");
        assert!(
            !a.contains(env!("CARGO_MANIFEST_DIR")),
            "{id}: no local path may leak"
        );
    }
}

/// `TraceDescriptor.server_public_key` is participant-writable, so a
/// key found there is only honoured when it is on the verifier's own allowlist, or when
/// the caller explicitly opted into the development escape hatch. The three M0 traces pin
/// no key at all, so both paths record `none` — and V12 has no acknowledgements to check
/// with it either way.
#[test]
fn the_m0_traces_report_no_key_source_under_either_option() {
    use goalcup_verifier::{KeySource, VerifyOptions, verify_trace_with_options};
    for id in ["claude-code-cli", "codex-cli"] {
        let strict = verify_fixture(id);
        assert_eq!(strict.key_source, KeySource::None, "{id}");
        let trace = support::materialize_trace(&dir(id));
        let dev = verify_trace_with_options(
            trace.path(),
            VerifyOptions {
                allow_descriptor_key: true,
                ..VerifyOptions::default()
            },
        )
        .unwrap();
        assert_eq!(dev.key_source, KeySource::None, "{id}");
        assert_eq!(
            strict.to_json().unwrap(),
            dev.to_json().unwrap(),
            "{id}: with no descriptor key the flag changes nothing"
        );
    }
}

/// The binary's exit codes.
///
/// `goalcup-verify` reserves 3 for "no verdict was produced", and clap's own default for a
/// usage error is 2 — which is the `Fail` verdict. A mistyped flag must not be readable as
/// a trace that failed verification. Spawned rather than unit-tested, because the exit
/// code is the thing under test and only a real process has one.
mod cli {
    use super::{dir, support};
    use std::process::Command;

    fn run(args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_goalcup-verify"))
            .args(args)
            .output()
            .expect("the binary is built by cargo before this test runs")
    }

    #[test]
    fn a_usage_error_exits_3_and_never_2() {
        for args in [vec![], vec!["--bogus"], vec!["--json"]] {
            let out = run(&args);
            assert_eq!(out.status.code(), Some(3), "args {args:?}");
            assert!(
                !String::from_utf8_lossy(&out.stderr).is_empty(),
                "args {args:?}: a usage error explains itself on stderr"
            );
            assert!(out.stdout.is_empty(), "args {args:?}: nothing on stdout");
        }
    }

    #[test]
    fn help_is_not_a_failure() {
        let out = run(&["--help"]);
        assert_eq!(out.status.code(), Some(0));
        assert!(String::from_utf8_lossy(&out.stdout).contains("--dev-descriptor-key"));
    }

    #[test]
    fn a_trace_that_cannot_be_read_exits_3_not_2() {
        let out = run(&["/goalcup-no-such-trace-dir"]);
        assert_eq!(out.status.code(), Some(3));
        assert!(out.stdout.is_empty());
    }

    /// The verdict's own code reaches the shell, and stdout is the verdict document.
    #[test]
    fn each_m0_fixture_prints_its_verdict_and_exits_2() {
        for id in ["claude-code-cli", "codex-cli"] {
            let fixture = support::materialize_trace(&dir(id));
            let out = run(&[fixture.path().to_str().unwrap()]);
            assert_eq!(out.status.code(), Some(2), "{id}");
            let v: serde_json::Value = serde_json::from_slice(&out.stdout)
                .unwrap_or_else(|e| panic!("{id}: stdout is the verdict JSON: {e}"));
            assert_eq!(v["overall"], "fail", "{id}");
        }
    }
}

/// The vacuity proof, inverted into a guard: an expectation that names a check the run
/// does not produce, or omits one it does, must fail.
///
/// `assert_matches_expected` compares the same set for the three M0 fixtures and
/// `adversarial.rs` for the fourth; this states it on its own, so the reason the
/// comparison is a set — and the count it must come to — survives a refactor of that
/// helper.
#[test]
fn the_checks_map_is_compared_as_a_full_key_set() {
    for id in ["claude-code-cli", "codex-cli", ADVERSARIAL] {
        let v = verify_fixture(id);
        let want = expectation(id);
        let expected: BTreeSet<&str> = want["checks"]
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        let got: BTreeSet<&str> = v.checks.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            expected, got,
            "{id}: the expectation must name every check, and only those"
        );
        assert_eq!(got.len(), 13);
    }
}

/// Every `must_not_contain` entry names a code some build can emit, and every `must_name`
/// entry names text the verdict actually carries.
///
/// The first half is the vacuity risk: a code that is renamed, or never existed, forbids
/// nothing at all, and the expectation goes on reading as though it did.
#[test]
fn must_name_and_must_not_contain_are_asserted_against_a_code_that_exists() {
    for id in ["claude-code-cli", "codex-cli", ADVERSARIAL] {
        let v = verify_fixture(id);
        // The review queue, not `v.checks`: `unexplained_seal_file` and
        // `read_only_attribution` are raised by `verify_trace` outside every check, and
        // both appear in the adversarial fixture's `must_not_contain` list. Reading only
        // the checks would forbid two codes it could never see.
        let all: BTreeSet<&str> = v.review_queue.iter().map(|f| f.code.as_str()).collect();
        let want = expectation(id);
        for c in want["must_not_contain"].as_array().unwrap() {
            let c = c.as_str().unwrap();
            assert!(
                is_an_emittable_code(c),
                "{id}: `{c}` names no code any build can emit, so forbidding it guards nothing"
            );
            assert!(!all.contains(c), "{id}: `{c}` must not appear");
        }
        // `must_name` reads every sentence the verdict says, at any severity — the same
        // set `assert_matches_expected` reads, and for the same reason. Pass-level findings
        // leave the queue without moving a byte of their text; `environment-context` and
        // `task-tool-subagent-prompt` are two needles inside one such message
        // (`user_slot_excluded`). The queue is a subset of what is read here.
        let messages: String = v
            .checks
            .iter()
            .flat_map(|c| c.findings.iter())
            .chain(v.review_queue.iter())
            .map(|f| f.message.clone())
            .collect::<Vec<_>>()
            .join("\n");
        for n in want["must_name"].as_array().unwrap() {
            let n = n.as_str().unwrap();
            assert!(
                !n.is_empty(),
                "{id}: an empty `must_name` entry names nothing"
            );
            assert!(messages.contains(n), "{id}: the verdict must name `{n}`");
        }
    }
}

/// The roster `is_an_emittable_code` answers from is a claim about this build, so it is
/// held to the source: every code on it is spelled somewhere in `src/`, and every code the
/// four fixtures actually emit is on it.
///
/// The second half is what stops the roster from silently going stale as checks grow; the
/// first is what stops a typo on the roster from turning a `must_not_contain` entry back
/// into a comment. `checks_v13.rs` holds the V13 half to exact equality with what V13
/// emits, which no fixture set can do for the other twelve.
#[test]
fn the_emittable_code_roster_matches_the_verifier() {
    use support::pipeline::{CHECK_CODES, V13_CODES};

    let roster: Vec<&str> = CHECK_CODES
        .iter()
        .chain(V13_CODES.iter())
        .copied()
        .collect();
    let unique: BTreeSet<&str> = roster.iter().copied().collect();
    assert_eq!(unique.len(), roster.len(), "the roster has no duplicate");

    // Every entry is a literal in the crate's own source. A code that is renamed or
    // deleted fails here rather than quietly forbidding nothing.
    let mut source = String::new();
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![src];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                source.push_str(&std::fs::read_to_string(&path).unwrap());
            }
        }
    }
    for code in &roster {
        assert!(
            source.contains(&format!("\"{code}\"")),
            "`{code}` is on the roster and appears nowhere in src/: a code nothing emits \
             forbids nothing"
        );
    }

    // And nothing the fixtures emit is missing from it.
    for id in ["claude-code-cli", "codex-cli", ADVERSARIAL] {
        let v = verify_fixture(id);
        for code in v
            .checks
            .iter()
            .flat_map(|c| c.findings.iter())
            .chain(v.review_queue.iter())
            .map(|f| f.code.as_str())
        {
            assert!(
                is_an_emittable_code(code),
                "{id}: `{code}` is emitted and is not on the roster"
            );
        }
    }
}

/// `--json` writes the document to the file and leaves stdout empty, so the run can be
/// piped; the summary goes to stderr with the word the file itself carries. And
/// `--version` is a version, not a usage error.
#[test]
fn json_mode_writes_the_file_and_prints_nothing_to_stdout() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("v.json");
    let fixture = support::materialize_trace(&dir("claude-code-cli"));
    let r = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup-verify"))
        .arg(fixture.path())
        .arg("--json")
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        r.stdout.is_empty(),
        "stdout: {}",
        String::from_utf8_lossy(&r.stdout)
    );
    let err = String::from_utf8_lossy(&r.stderr);
    assert!(
        err.contains("fail"),
        "the serialised word, not Debug: {err}"
    );
    assert!(!err.contains("Fail"));
    let written: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(
        written["overall"], "fail",
        "the word on stderr is the word in the file"
    );

    let v = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup-verify"))
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(v.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&v.stdout).contains(env!("CARGO_PKG_VERSION")));
}

/// The default invocation — no `--json` — puts the verdict on stdout and the same summary
/// line on stderr.
///
/// Stdout is exactly one document, which is what makes the pipe work.
#[test]
fn the_default_invocation_puts_the_verdict_on_stdout_and_the_summary_on_stderr() {
    let fixture = support::materialize_trace(&dir("claude-code-cli"));
    let r = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup-verify"))
        .arg(fixture.path())
        .output()
        .unwrap();
    let printed: serde_json::Value =
        serde_json::from_slice(&r.stdout).expect("stdout is one verdict document and nothing else");
    assert_eq!(printed["overall"], "fail");
    let err = String::from_utf8_lossy(&r.stderr);
    assert!(
        err.contains("goalcup-verify: claude-code-cli fail"),
        "the summary a person reads, on the stream meant for people: {err:?}"
    );
    assert!(
        !err.contains("Fail"),
        "the serialised word, the same one the document carries: {err:?}"
    );
}

/// The pipeline the integration tests borrow a context from is the one the verdict is
/// computed from — asserted, not assumed.
///
/// This is the whole point of `Pipeline` being public: a `CheckContext` built by hand
/// carries `ManifestPinning::default()`, which reads `Unpinned`, so V13 would be asked
/// about a manifest nothing had resolved.
#[test]
fn a_borrowed_context_carries_what_the_verdict_carries() {
    use goalcup_verifier::ManifestPin;

    for id in ["claude-code-cli", "codex-cli", ADVERSARIAL] {
        let pipeline = pipeline_fixture(id);
        let verdict = verify_fixture(id);
        let cx = pipeline.context();

        assert_eq!(
            cx.manifest_pin.state,
            ManifestPin::Pinned,
            "{id}: the pin is resolved by `ManifestPinning::resolve`, never defaulted"
        );
        assert_eq!(cx.manifest_pin.state, verdict.manifest_pin, "{id}");
        assert_eq!(
            cx.manifest_sha256, verdict.connector_manifest_sha256,
            "{id}"
        );
        assert_eq!(
            cx.acks.len(),
            cx.acks_verified.verified.len() + cx.acks_verified.mismatches.len(),
            "{id}: every acknowledgement was reconciled, or the reconciliation is a stub"
        );
        assert_eq!(
            pipeline.run_checks(),
            verdict.checks,
            "{id}: the checks a test runs are the checks the verdict reports"
        );
        assert_eq!(
            pipeline.verdict().unwrap().to_json().unwrap(),
            verdict.to_json().unwrap(),
            "{id}: `verify_trace` is `Pipeline::load(..)?.verdict()`"
        );
    }
}

/// The live Codex capture this repository keeps, end to end: `tests/traces/codex-live/` is
/// the daemon's own export of a real Codex CLI 0.153.0 run, neither scrubbed nor written by
/// hand. It is `#[ignore]`d because its start receipt binds manifest and connector-directory
/// pins that no longer match the shipped ones; the export is re-recorded, never edited.
#[test]
#[ignore = "awaiting re-record: this authentic export's `expected-verdict.json` still names \
            the pre-e105847 reading, and `assert_matches_expected` compares the whole \
            document, so the fixture stays dark until the run is captured again and the \
            expectation refreshed with it"]
fn the_live_codex_run_verifies() {
    let v = assert_matches_expected("codex-live");
    assert_eq!(v.connector_id, "codex-cli");
    assert_eq!(v.harness_version, "0.153.0");
    // The shipped-pin statement lives in `checks_v13::shipped_connector_pins_match`. This
    // fixture verifies against the connector snapshot it was recorded with, which is what its
    // start receipt binds.

    // The file the agent added is attributed
    // to the `exec` call whose argument text carried the `apply_patch` DSL.
    let hello = v
        .seal_attribution
        .iter()
        .find(|a| a.path == "hello.py")
        .expect("the one file the run left");
    match &hello.attribution {
        goalcup_verifier::attribution::Attribution::Traced { tool, .. } => {
            assert_eq!(tool, "exec")
        }
        other => panic!("hello.py is explained by no traced write: {other:?}"),
    }

    // The tracked session is the thread, not a turn — `session_id_field` is
    // `payload.session_id`, and V4 finds one session and the resume that re-entered it.
    let v4 = v.checks.iter().find(|c| c.id == "V4").expect("V4 ran");
    assert_eq!(v4.verdict, Severity::Pass);
    assert!(
        v4.findings.iter().any(|f| f.code == "one_tracked_session"
            && f.message.contains("01a07f44-2ecc-7ce2-91a4-1c33963d93c8")),
        "{:?}",
        v4.findings
    );

    // And the `--dev` reading, which is the line `goalcup seal` printed for this very run.
    let dev = goalcup_verifier::verify_trace_with_options(
        &dir("codex-live"),
        goalcup_verifier::VerifyOptions {
            dev_mode: true,
            ..Default::default()
        },
    )
    .expect("the dev reading");
    assert_eq!(dev.overall, Severity::Pass);
    assert!(dev.review_queue.is_empty(), "{:?}", dev.review_queue);
}
