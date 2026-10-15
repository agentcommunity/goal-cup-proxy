//! Schema 1 owns capture behavior, and doctor describes a connector from its `harness.toml`
//! without spawning it.

use crate::common::support;

use goalcup_cli::commands::doctor::{DoctorOptions, run_with};
use goalcup_cli::commands::start::start_with;

fn opts(harness: Option<&str>) -> DoctorOptions {
    DoctorOptions {
        json: false,
        harness: harness.map(str::to_string),
    }
}

/// `start` of a Hermes-shaped connector: with no family refusal left, a `--goal-file` start of
/// the shipped Hermes connector reaches the launch's own checks. With no provider key in the
/// environment it refuses, naming the variable and never a value, before the daemon is asked for
/// anything, so no capture workspace or reserved chain is left behind.
#[test]
fn a_goal_file_start_of_the_shipped_hermes_connector_refuses_without_a_key_before_the_daemon() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    let goal = tmp.path().join("goal.txt");
    std::fs::write(&goal, "write result.txt").unwrap();
    let mut o = support::start_options("hermes-cli");
    o.goal_file = Some(goal);
    let err = start_with(
        &m,
        &support::connectors_dir(),
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut Vec::new(),
    )
    .expect_err("no provider key");
    assert_eq!(err.exit_code(), 1, "{err}");
    assert!(err.to_string().contains("DEEPSEEK_API_KEY"), "{err}");
    assert!(
        !err.to_string()
            .contains("cannot be launched for capture by this build yet"),
        "the family refusal is gone: {err}"
    );
    assert!(
        m.calls().is_empty(),
        "the daemon was asked: {:?}",
        m.calls()
    );
}

/// A hint no built-in decoder accepts is reported as capture-only, from `harness.toml` alone.
#[test]
fn doctor_lists_a_harness_only_unknown_hint_as_capture_only() {
    let tmp = tempfile::tempdir().unwrap();
    let only = tmp.path().join("harness-only");
    std::fs::create_dir_all(&only).unwrap();
    let source =
        std::fs::read_to_string(support::connectors_dir().join("claude-code-cli/harness.toml"))
            .unwrap()
            .replacen("id = \"claude-code-cli\"", "id = \"example-unknown\"", 1)
            .replacen("providers = [\"anthropic\"]\n", "", 1)
            + "\n[[capture.hosts]]\nhost = \"api.anthropic.com\"\n\
               evidence_paths = [\"/v1/messages\"]\nprotocol_hint = \"example-unknown\"\n";
    assert!(!source.contains("providers = "));
    std::fs::write(only.join("harness.toml"), source).unwrap();

    let cfg = tempfile::tempdir().unwrap();
    let mut out = Vec::new();
    run_with(
        None,
        tmp.path(),
        cfg.path(),
        &std::collections::BTreeMap::new(),
        &opts(None),
        &mut out,
    )
    .expect("doctor reports even with no daemon");
    let said = String::from_utf8(out).unwrap();
    assert!(said.contains("example-unknown"), "{said}");
    assert!(said.contains("no decoder accepts"), "{said}");
    assert!(said.contains("capture-only"), "{said}");
}

#[test]
fn doctor_treats_the_hermes_wire_hint_as_decoded() {
    let cfg = tempfile::tempdir().unwrap();
    let mut out = Vec::new();
    run_with(
        None,
        &support::connectors_dir(),
        cfg.path(),
        &std::collections::BTreeMap::new(),
        &opts(Some("hermes-cli")),
        &mut out,
    )
    .expect("the harness report is available without a probe");
    let said = String::from_utf8(out).unwrap();
    assert!(said.contains("openai-chat-completions"), "{said}");
    assert!(said.contains("accepted"), "{said}");
    assert!(
        !said.contains("no decoder accepts `openai-chat-completions`"),
        "{said}"
    );
}

/// With no `--harness`, doctor checks the harness `goalcup prep` settled, so a harness the
/// participant never installed cannot fail the report; `--harness all` still checks them all.
#[test]
fn doctor_defaults_to_the_harness_in_prep_json_and_all_checks_every_one() {
    let cfg = tempfile::tempdir().unwrap();
    goalcup_cli::prep_file::write_prep(
        cfg.path(),
        &goalcup_cli::prep_file::PrepFile {
            harness: "hermes-cli".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    let report = |harness: Option<&str>| {
        let mut out = Vec::new();
        run_with(
            None,
            &support::connectors_dir(),
            cfg.path(),
            &std::collections::BTreeMap::new(),
            &opts(harness),
            &mut out,
        )
        .unwrap();
        String::from_utf8(out).unwrap()
    };

    let said = report(None);
    assert!(
        said.contains("goalcup doctor — hermes-cli (from goalcup prep)"),
        "{said}"
    );
    assert!(!said.contains("claude-code-cli"), "{said}");
    assert!(!said.contains("codex-cli"), "{said}");

    let said = report(Some("all"));
    for id in ["claude-code-cli", "codex-cli", "hermes-cli"] {
        assert!(said.contains(id), "`--harness all` reports {id}: {said}");
    }
    assert!(!said.contains("(from goalcup prep)"), "{said}");

    let said = report(Some("codex-cli"));
    assert!(
        said.contains("codex-cli only"),
        "an explicit id wins: {said}"
    );
    assert!(!said.contains("hermes-cli"), "{said}");
}

/// A `prep.json` that names a harness no connector here declares (stale after an upgrade) does
/// not make bare `goalcup doctor` refuse: it checks every connector, as before prep existed.
/// An explicit `--harness <id>` that nothing declares is still a refusal.
#[test]
fn doctor_falls_back_to_every_connector_when_prep_names_an_unknown_harness() {
    let cfg = tempfile::tempdir().unwrap();
    goalcup_cli::prep_file::write_prep(
        cfg.path(),
        &goalcup_cli::prep_file::PrepFile {
            harness: "no-such-harness".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    let run = |harness: Option<&str>, out: &mut Vec<u8>| {
        run_with(
            None,
            &support::connectors_dir(),
            cfg.path(),
            &std::collections::BTreeMap::new(),
            &opts(harness),
            out,
        )
    };
    let mut out = Vec::new();
    run(None, &mut out).expect("bare doctor still reports");
    let said = String::from_utf8(out).unwrap();
    assert!(said.starts_with("goalcup doctor\n"), "{said}");
    assert!(!said.contains("from goalcup prep"), "{said}");
    for id in ["claude-code-cli", "codex-cli", "hermes-cli"] {
        assert!(said.contains(id), "every connector is reported: {said}");
    }
    let refused = run(Some("no-such-harness"), &mut Vec::new());
    assert!(refused.is_err(), "an explicit unknown id is refused");
}
