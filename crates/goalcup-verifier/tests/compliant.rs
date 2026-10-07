//! The pass path. A run that broke no rule reads as a run that broke no rule.
//!
//! Every other fixture in this crate is of something going wrong, so this is the one that
//! asserts the other direction over real bytes. `tests/support/compliant.rs` generates both
//! current test traces and the
//! frozen-input protocol-v1 bytes under `tests/fixtures/compliant/`.
//!
//! The pass claim is a `--dev` claim, and the reason is the
//! anchor rather than the rules: the generated trace signs with a seed published here,
//! and V13 will not let a signature under a published seed read as an anchor. See `dev()`
//! below and the committed `expected-verdict.json`, which records the equivalent current
//! trace's non-dev verdict shape. The historical fixture's current-pin result is separate.

mod support;

use goalcup_verifier::checks::Severity;

fn dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/compliant")
}

fn current_trace() -> tempfile::TempDir {
    let trace = tempfile::tempdir().unwrap();
    support::compliant::generate(trace.path());
    trace
}

/// A `--dev` run, which is what the current generated trace's pass claim is about.
///
/// The fixture signs its receipt and its acknowledgements with `[5u8; 32]`, the seed
/// written in this repository so that the fixtures can sign at all; its verifying key is
/// `PLACEHOLDER_SERVER_PUBLIC_KEY`, the single entry in `SERVER_PUBLIC_KEYS`. V13 says so —
/// `placeholder_server_key` fires whenever that key
/// verified something, because anybody can mint a receipt under a published seed and
/// `key_source: "pinned"` would otherwise read as an anchor. `--dev` suppresses it, and a
/// `--dev` run is exactly what this fixture is: a check that the rules pass, made against a
/// build whose anchor is still a placeholder.
fn dev() -> goalcup_verifier::VerifyOptions {
    goalcup_verifier::VerifyOptions {
        allow_descriptor_key: false,
        extra_server_keys: Vec::new(),
        dev_mode: true,
    }
}

/// The pass claim, made under `--dev`. See `dev()` above and the current generated non-dev
/// verdict control below for what a run without the flag says instead.
#[test]
fn a_compliant_trace_passes_every_check_under_dev_and_the_queue_is_empty() {
    let trace = current_trace();
    let v = goalcup_verifier::verify_trace_with_options(trace.path(), dev()).unwrap();
    assert_eq!(
        v.overall,
        Severity::Pass,
        "review queue: {:?}",
        v.review_queue
            .iter()
            .map(|f| (&f.code, f.severity))
            .collect::<Vec<_>>()
    );
    assert!(v.review_queue.is_empty());
    assert_eq!(v.exit_code(), 0);
    assert_eq!(v.checks.len(), 13);
    for c in &v.checks {
        assert_eq!(c.verdict, Severity::Pass, "{}: {:?}", c.id, c.findings);
    }

    // Pass-level findings are still said, in the checks. Three of them are
    // reachable on an honest trace at all — `cross_leg_identifiers` needs tool calls on
    // both legs, `clean_gap` needs accounted-for downtime, `user_slot_excluded` needs a
    // harness-authored record in the user slot — and this fixture arranges all three on
    // purpose. `anchor_timeline` and `binding_checked` are the two summaries a reader looks
    // for to know that V12 and V13 actually ran rather than finding nothing to say.
    //
    // V13's summary is `binding_checked`; there is no `binding_verified` or `chain_verified`
    // code. The chain's integrity is reported by the absence of findings (`chain_broken` is
    // what a chain that stopped holding raises), which the per-check Pass assertion above
    // already covers.
    let said: std::collections::BTreeSet<&str> = v
        .checks
        .iter()
        .flat_map(|c| c.findings.iter())
        .map(|f| f.code.as_str())
        .collect();
    for code in [
        "cross_leg_identifiers",
        "clean_gap",
        "user_slot_excluded",
        "anchor_timeline",
        "binding_checked",
    ] {
        assert!(
            said.contains(code),
            "{code} left the document instead of leaving the queue; said: {said:?}"
        );
    }

    // The frame the whole trace hangs on, asserted rather than assumed: a Pass reached with
    // an unpinned manifest or an unrecognised key would be a Pass about nothing.
    assert_eq!(v.manifest_pin, goalcup_verifier::ManifestPin::Pinned);
    assert_eq!(v.key_source, goalcup_verifier::KeySource::Pinned);
    assert_eq!(v.entry_id, support::compliant::ENTRY_ID);
    assert!(
        v.seal_attribution.iter().all(|a| !matches!(
            a.attribution,
            goalcup_verifier::attribution::Attribution::Unexplained
        )),
        "{:?}",
        v.seal_attribution
    );
    assert!(
        v.gaps
            .iter()
            .all(|g| g.kind == goalcup_verifier::gaps::GapKind::Clean),
        "every hole in an accounted-for week is clean"
    );

    // No absolute machine path anywhere in the document, asserted over the whole serialised
    // verdict exactly as `end_to_end.rs` does it. A passing verdict is the one most likely
    // to be published, and `--manifest-path`-shaped strings reach it through a finding
    // message, a seal path or an evidence note.
    let json = v.to_json().unwrap();
    for prefix in ["/Users/", "/home/", "/private/"] {
        assert!(
            !json.contains(prefix),
            "the verdict leaks an absolute path beginning `{prefix}`"
        );
    }
}

#[test]
fn the_binary_exits_zero_on_it_with_dev() {
    let trace = current_trace();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup-verify"))
        .arg(trace.path())
        .arg("--dev")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The expected verdict is the stable shape of a current generated compliant trace
/// without `--dev`: one Flag, `placeholder_server_key`, and exit 1.
///
/// The committed historical bytes carry an older connector pin, so their current
/// verifier result is asserted separately below rather than being conflated with this
/// current-pin expectation.
#[test]
fn the_expected_verdict_matches_a_current_generated_trace_without_dev() {
    let want: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir().join("expected-verdict.json")).unwrap(),
    )
    .unwrap();

    let trace = current_trace();
    let v = goalcup_verifier::verify_trace(trace.path()).unwrap();
    assert_eq!(
        serde_json::to_value(v.overall).unwrap(),
        want["overall"],
        "{:?}",
        v.review_queue
    );
    assert_eq!(v.exit_code() as i64, want["exit_code"].as_i64().unwrap());
    assert_eq!(
        serde_json::to_value(v.key_source).unwrap(),
        want["key_source"]
    );
    assert_eq!(
        serde_json::to_value(v.manifest_pin).unwrap(),
        want["manifest_pin"]
    );
    let codes: std::collections::BTreeSet<&str> =
        v.review_queue.iter().map(|f| f.code.as_str()).collect();
    let want_codes: std::collections::BTreeSet<&str> = want["finding_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(codes, want_codes);

    // The binary a person runs, on equivalent current generated bytes.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup-verify"))
        .arg(trace.path())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn the_historical_fixture_reports_its_old_manifest_and_connector_directory_pins() {
    let v = goalcup_verifier::verify_trace_with_options(&dir(), dev()).unwrap();
    assert_eq!(v.manifest_pin, goalcup_verifier::ManifestPin::Altered);
    assert_eq!(v.overall, Severity::Flag);
    let v13 = v.checks.iter().find(|check| check.id == "V13").unwrap();
    let manifest = v13
        .findings
        .iter()
        .find(|finding| finding.code == "manifest_not_pinned")
        .expect("the historical rulebook is no longer a current build pin");
    assert_eq!(manifest.severity, Severity::Flag);
    let directory = v13
        .findings
        .iter()
        .find(|finding| finding.code == "connector_dir_unpinned")
        .expect("the historical tracker templates are no longer a current build pin");
    assert_eq!(directory.severity, Severity::Pass);
}

/// The fixture is a build artefact of a generator, not a file anybody edits: regenerate it
/// and the bytes must be the ones that are committed.
#[test]
fn the_committed_bytes_are_what_the_generator_produces() {
    let tmp = tempfile::tempdir().unwrap();
    support::compliant::generate_historical(tmp.path());
    for f in support::compliant::FILES {
        assert_eq!(
            std::fs::read(tmp.path().join(f)).unwrap(),
            std::fs::read(dir().join(f)).unwrap(),
            "{f} drifted from its generator"
        );
    }
}

#[test]
fn the_generator_replaces_only_its_outputs_on_a_second_pass() {
    let tmp = tempfile::tempdir().unwrap();
    support::compliant::generate_historical(tmp.path());
    let first: std::collections::BTreeMap<&str, Vec<u8>> = support::compliant::FILES
        .into_iter()
        .map(|name| (name, std::fs::read(tmp.path().join(name)).unwrap()))
        .collect();
    let sentinel = tmp.path().join("unrelated-sentinel.txt");
    std::fs::write(&sentinel, b"preserve me").unwrap();

    support::compliant::generate_historical(tmp.path());

    assert_eq!(std::fs::read(sentinel).unwrap(), b"preserve me");
    for name in support::compliant::FILES {
        let second = std::fs::read(tmp.path().join(name)).unwrap();
        assert_eq!(second, first[name], "{name} changed between passes");
        assert_eq!(
            second,
            std::fs::read(dir().join(name)).unwrap(),
            "{name} differs from the committed historical fixture"
        );
    }
}

/// Rewrites the committed fixture.
///
/// Two locks, not one — the same pair `adversarial.rs` uses. `#[ignore]` keeps it out of an
/// ordinary `cargo test`, but `cargo test -- --include-ignored` would run it, overwriting
/// the checked-in bytes and
/// then asserting they matched what had just been written. So the regeneration itself is
/// gated on an environment variable a person types on purpose: under `--include-ignored`
/// alone this returns early, having written nothing, and says why.
#[test]
#[ignore = "writes the committed fixture; set GOALCUP_REGEN=1; see scripts/regen-compliant-fixture.md"]
fn regenerate() {
    if std::env::var("GOALCUP_REGEN").as_deref() != Ok("1") {
        println!(
            "regenerate: GOALCUP_REGEN=1 is not set, so the committed fixture at {} was \
             left alone. Run `GOALCUP_REGEN=1 cargo test -p goalcup-verifier --test \
             compliant -- --ignored regenerate`, read the diff, then commit.",
            dir().display()
        );
        return;
    }
    support::compliant::generate_historical(&dir());
}

/// The switch is participant-written. Declaring it must not be enough.
#[test]
fn expected_reread_without_a_restart_does_not_downgrade_a_rewrite() {
    let bare = support::compliant_with_truncation_and_expected_reread(false);
    let v = goalcup_verifier::verify_trace(bare.path()).unwrap();
    assert!(
        v.review_queue
            .iter()
            .any(|f| f.code == "store_rewritten" && f.severity == Severity::Fail),
        "{:?}",
        v.review_queue
            .iter()
            .map(|f| (&f.code, f.severity))
            .collect::<Vec<_>>()
    );

    let corroborated = support::compliant_with_truncation_and_expected_reread(true);
    let v = goalcup_verifier::verify_trace(corroborated.path()).unwrap();
    assert!(v.review_queue.iter().all(|f| f.code != "store_rewritten"));
    assert!(
        v.checks
            .iter()
            .flat_map(|c| &c.findings)
            .any(|f| f.code == "store_reread")
    );
}

/// The chained `HomeSeeded` record reaches the verdict as a plain fact, and
/// changes nothing else about it.
///
/// Three claims, and they are the whole of what this record is for:
///
///   1. `carried_settings` carries the launch's record verbatim, values and all — the
///      browser-tools switch included.
///   2. It is not a judgement. The verdict over this trace is the verdict over the compliant
///      one: same severity, same exit code, same review queue.
///   3. A trace that carries none — current or historical — reads as an empty list and
///      serialises without the field at all, so an older verdict's bytes are unchanged.
#[test]
fn the_settings_a_launch_carried_reach_the_verdict_as_facts_and_judge_nothing() {
    let seeded = support::compliant_with_home_seed();
    let v = goalcup_verifier::verify_trace(seeded.path()).expect("the variant verifies");

    assert_eq!(
        v.carried_settings,
        vec![support::compliant::home_seed_record()],
        "one record per launch, exactly as the chain carries it"
    );
    let switch = v.carried_settings[0]
        .keys
        .iter()
        .find(|k| k.name == "claudeInChromeDefaultEnabled")
        .expect("the browser-tools switch was carried");
    assert_eq!(switch.value, Some(serde_json::Value::Bool(true)));
    assert!(
        v.carried_settings[0]
            .keys
            .iter()
            .any(|k| k.name == "oauthAccount" && k.value.is_none()),
        "an object holding an account is named and never quoted"
    );
    // Copied and written are two lists, so a reader can tell the participant's decisions
    // from the tracker's one.
    assert_eq!(
        v.carried_settings[0]
            .written
            .iter()
            .map(|k| (k.name.as_str(), k.value.clone()))
            .collect::<Vec<_>>(),
        vec![(
            "hasTrustDialogAccepted",
            Some(serde_json::Value::Bool(true))
        )],
        "the workspace trust entry is the tracker's own write, and it is reported as one"
    );
    let json = v.to_json().unwrap();
    assert!(
        !json.contains("someone@example.test"),
        "the verdict quotes an address out of the participant's own file"
    );

    // Nothing else moved. The record is a statement, not a check.
    let plain_trace = current_trace();
    let plain = goalcup_verifier::verify_trace(plain_trace.path())
        .expect("the equivalent current trace verifies");
    assert_eq!(v.overall, plain.overall);
    assert_eq!(v.exit_code(), plain.exit_code());
    assert_eq!(v.review_queue.len(), plain.review_queue.len());

    // And equivalent current bytes with no event say nothing at all — absent, never an
    // empty list.
    assert!(plain.carried_settings.is_empty());
    let plain_json: serde_json::Value = serde_json::from_str(&plain.to_json().unwrap()).unwrap();
    assert!(
        plain_json.get("carried_settings").is_none(),
        "an older chain's verdict serialises exactly as it did"
    );

    let historical = goalcup_verifier::verify_trace(&dir()).expect("the old trace verifies");
    assert!(historical.carried_settings.is_empty());
    let historical_json: serde_json::Value =
        serde_json::from_str(&historical.to_json().unwrap()).unwrap();
    assert!(historical_json.get("carried_settings").is_none());
}
