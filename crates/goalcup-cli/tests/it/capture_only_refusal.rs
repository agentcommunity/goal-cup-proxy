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
            .replace(
                "protocol_hint = \"anthropic-messages\"",
                "protocol_hint = \"example-unknown\"",
            );
    std::fs::write(only.join("harness.toml"), source).unwrap();

    let cfg = tempfile::tempdir().unwrap();
    let mut out = Vec::new();
    run_with(
        None,
        tmp.path(),
        cfg.path(),
        &opts(None),
        &mut out,
        &mut Vec::new(),
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
        &opts(Some("hermes-cli")),
        &mut out,
        &mut Vec::new(),
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
