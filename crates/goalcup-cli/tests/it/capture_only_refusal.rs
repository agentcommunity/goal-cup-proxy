//! Recorded V1 capture keys are accepted and ignored; schema 1 owns capture behavior, and
//! doctor describes a connector from its `harness.toml` without spawning it.

use crate::common::support;

use goalcup_cli::commands::doctor::{DoctorOptions, run_with};
use goalcup_cli::commands::start::start_with;

const MANIFEST: &str = r#"
id = "hermes-test"
family = "hermes"
tier = "elevated_review"
tested_version_range = "=0.19.0"
capture_only = true

[detection]
binary = "hermes"
version_command = "hermes --version"

[arming]
signal = "none"
timeout_seconds = 1

[kill]
strategy = "process_group"
grace_seconds = 10

[launch]
mode = "server"
program = "hermes"
home_env = "HERMES_HOME"

[resume]
session_arg_style = "flag"

[version]
self_updating = false
sample_interval_seconds = 300
record_field = "version"

[normalisation]
trailing_newline = true

[capture]
automatic_goal = true
goal_flag = "--query"
home = "empty"
config_home = "shared"
ca_env = ["HERMES_CA_BUNDLE"]
continuation = "none: one chat turn"
args = ["chat", "-Q", "--provider", "deepseek", "--model", "deepseek-v4-flash"]

[capture.executable]
version = "0.19.0"
sha256 = "0444572f0bce8a805473fbcaa431beed399818528758b454fbf24825a51be325"

[[capture.hosts]]
host = "api.deepseek.com"
evidence_paths = ["/v1/chat/completions"]
protocol_hint = "openai-chat-completions"
"#;

fn opts(harness: Option<&str>) -> DoctorOptions {
    DoctorOptions {
        json: false,
        harness: harness.map(str::to_string),
    }
}

/// A recorded legacy value is consumed rather than treated as an executable switch.
#[test]
fn recorded_capture_keys_are_ignored_by_the_v1_parser() {
    let manifest = goalcup_protocol::manifest::ConnectorManifest::from_toml_str(MANIFEST)
        .expect("recorded capture values parse");
    let json = serde_json::to_string(&manifest).unwrap();
    assert!(!json.contains("capture_only"), "{json}");
    assert!(!json.contains("automatic_goal"), "{json}");
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
        &std::collections::BTreeMap::new(),
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
        &std::collections::BTreeMap::new(),
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
