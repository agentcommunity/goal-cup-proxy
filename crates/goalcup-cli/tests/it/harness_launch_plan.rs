//! Pure decisions derived from harness-contract schema 1.

use goalcup_cli::launch::{LaunchScope, StderrUse, StdoutUse, StreamPlan, harness_arg_refusal};
use goalcup_cli::preflight::{Level, harness_pin_gate};
use goalcup_protocol::harness::{HarnessManifest, Origin, load_dir};
use goalcup_protocol::template::TemplateVars;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn connectors() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors")
}

fn load(id: &str) -> goalcup_protocol::harness::LoadedConnector {
    load_dir(&connectors().join(id), Origin::Pinned).expect("the shipped harness loads")
}

#[test]
fn shipped_harnesses_have_the_s0_stream_and_identity_plans() {
    let hermes = load("hermes-cli");
    let plan = StreamPlan::for_spec(&hermes.manifest.capture).unwrap();
    assert!(plan.scan_for_key);
    assert_eq!(plan.stdout, StdoutUse::Forward);
    assert!(matches!(plan.stderr, StderrUse::IdentityLine(_)));
    assert_eq!(
        LaunchScope::for_spec(&hermes.manifest.capture).unwrap(),
        LaunchScope::AtFinish
    );

    let codex = load("codex-cli");
    let plan = StreamPlan::for_spec(&codex.manifest.capture).unwrap();
    assert!(!plan.scan_for_key);
    assert!(matches!(plan.stdout, StdoutUse::IdentityJson(_)));
    assert_eq!(plan.stderr, StderrUse::Inherit);
    assert_eq!(
        LaunchScope::for_spec(&codex.manifest.capture).unwrap(),
        LaunchScope::FromStdout
    );

    let claude = load("claude-code-cli");
    let plan = StreamPlan::for_spec(&claude.manifest.capture).unwrap();
    assert!(!plan.scan_for_key);
    assert_eq!(plan.stdout, StdoutUse::Inherit);
    assert_eq!(plan.stderr, StderrUse::Inherit);
    assert_eq!(
        LaunchScope::for_spec(&claude.manifest.capture).unwrap(),
        LaunchScope::Preassigned
    );

    let text = std::fs::read_to_string(connectors().join("claude-code-cli/harness.toml"))
        .unwrap()
        .replace(
            "[capture.identity]\nsource = \"preassigned\"\nbinding = \"at_launch\"",
            "[capture.identity]\nsource = \"none\"",
        );
    let workload = HarnessManifest::from_toml_str(&text).unwrap();
    let plan = StreamPlan::for_spec(&workload.capture).unwrap();
    assert!(!plan.scan_for_key);
    assert_eq!(plan.stdout, StdoutUse::Inherit);
    assert_eq!(plan.stderr, StderrUse::Inherit);
    assert_eq!(
        LaunchScope::for_spec(&workload.capture).unwrap(),
        LaunchScope::None
    );
}

#[test]
fn preassigned_scope_does_not_change_when_a_credential_is_present() {
    let path = connectors().join("claude-code-cli/harness.toml");
    let text = std::fs::read_to_string(path).unwrap();
    let text = text.replace(
        "[capture.ca]",
        "[capture.credential]\nenv = \"DEEPSEEK_API_KEY\"\n\n[capture.ca]",
    );
    // Parse without validating: this deliberately probes the launcher's decision rather than
    // asking protocol validation whether such a shipped combination is useful.
    let manifest = HarnessManifest::from_toml_str(&text).unwrap();
    assert_eq!(
        LaunchScope::for_spec(&manifest.capture).unwrap(),
        LaunchScope::Preassigned
    );
    let streams = StreamPlan::for_spec(&manifest.capture).unwrap();
    assert!(streams.scan_for_key);
    assert_eq!(streams.stdout, StdoutUse::Forward);
    assert_eq!(streams.stderr, StderrUse::Forward);
}

#[test]
fn identity_rules_are_copied_from_the_manifest() {
    let codex = load("codex-cli");
    let StreamPlan {
        stdout: StdoutUse::IdentityJson(rule),
        ..
    } = StreamPlan::for_spec(&codex.manifest.capture).unwrap()
    else {
        panic!("Codex must use its stdout JSON identity rule")
    };
    assert_eq!(rule.match_field, "type");
    assert_eq!(rule.match_value, "thread.started");
    assert_eq!(rule.id_field, "thread_id");

    let hermes = load("hermes-cli");
    let StreamPlan {
        stderr: StderrUse::IdentityLine(rule),
        ..
    } = StreamPlan::for_spec(&hermes.manifest.capture).unwrap()
    else {
        panic!("Hermes must use its stderr identity rule")
    };
    assert_eq!(rule.prefix, "session_id: ");
    assert_eq!(rule.grammar.as_str(), "timestamp-hex");
}

#[test]
fn argv_guard_comes_from_the_harness_manifest() {
    let hermes = load("hermes-cli");
    for arg in ["--profile", "--pro", "--tui", "--cli", "-p", "-h"] {
        assert!(
            harness_arg_refusal(&hermes.manifest.capture, arg).is_some(),
            "Hermes must refuse {arg}"
        );
    }
    for arg in [
        "--provider",
        "--model",
        "--query=hello",
        "--unknown",
        "-x",
        "plain",
    ] {
        assert_eq!(
            harness_arg_refusal(&hermes.manifest.capture, arg),
            None,
            "Hermes must allow {arg}"
        );
    }

    for id in ["codex-cli", "claude-code-cli"] {
        let loaded = load(id);
        assert!(loaded.manifest.capture.argv_guard.is_none(), "{id}");
        assert_eq!(
            harness_arg_refusal(&loaded.manifest.capture, "--provider"),
            None,
            "{id} has no argv guard"
        );
    }
}

fn write_unpinned(root: &Path, dir_name: &str, id: &str) -> PathBuf {
    let dir = root.join(dir_name);
    std::fs::create_dir_all(&dir).unwrap();
    let source = std::fs::read_to_string(connectors().join("claude-code-cli/harness.toml"))
        .unwrap()
        .replacen("id = \"claude-code-cli\"", &format!("id = \"{id}\""), 1);
    std::fs::write(dir.join("harness.toml"), source).unwrap();
    dir
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn unpinned_harness_uses_the_one_dev_door() {
    let tmp = tempfile::tempdir().unwrap();
    write_unpinned(tmp.path(), "unexpected-folder", "example-cli");

    let strict = goalcup_cli::connector::load_harness(tmp.path(), "example-cli", false).unwrap();
    let strict_gate = harness_pin_gate(&strict, false);
    assert_eq!(strict_gate.level, Level::Fail);
    assert!(strict_gate.detail.contains("no build pins `example-cli`"));

    let dev = goalcup_cli::connector::load_harness(tmp.path(), "example-cli", true).unwrap();
    let dev_gate = harness_pin_gate(&dev, true);
    assert_eq!(dev_gate.level, Level::Warn);
    assert!(dev_gate.detail.contains("--dev"));
    assert_eq!(dev.loaded.manifest.id, "example-cli");
    assert_eq!(dev.dir.file_name().unwrap(), "unexpected-folder");
}

#[test]
fn a_mixed_pinned_and_development_root_loads_each_by_its_captured_identity() {
    let tmp = tempfile::tempdir().unwrap();
    copy_tree(
        &connectors().join("claude-code-cli"),
        &tmp.path().join("claude-code-cli"),
    );
    write_unpinned(tmp.path(), "z-development", "example-cli");

    let loaded = goalcup_cli::connector::load_harness(tmp.path(), "example-cli", true).unwrap();
    assert_eq!(loaded.loaded.manifest.id, "example-cli");
    assert_eq!(loaded.dir.file_name().unwrap(), "z-development");
    assert_eq!(harness_pin_gate(&loaded, true).level, Level::Warn);
}

#[test]
fn a_byte_identical_shipped_connector_under_an_alias_is_refused_at_cli_discovery() {
    let tmp = tempfile::tempdir().unwrap();
    copy_tree(
        &connectors().join("claude-code-cli"),
        &tmp.path().join("alias"),
    );

    let error = goalcup_cli::connector::load_harness(tmp.path(), "claude-code-cli", false)
        .expect_err("shipped bytes have one admitted folder");
    assert_eq!(error.exit_code(), 1, "{error}");
    assert!(error.to_string().contains("claude-code-cli"), "{error}");
    assert!(error.to_string().contains("folder"), "{error}");
}

#[test]
fn altered_pinned_id_is_never_relaxed_by_dev() {
    let tmp = tempfile::tempdir().unwrap();
    let copied = tmp.path().join("renamed");
    std::fs::create_dir_all(&copied).unwrap();
    std::fs::copy(
        connectors().join("claude-code-cli/harness.toml"),
        copied.join("harness.toml"),
    )
    .unwrap();
    let error = goalcup_cli::connector::load_harness(tmp.path(), "claude-code-cli", true)
        .expect_err("a pinned id under an alias is refused before dev preflight");
    assert_eq!(error.exit_code(), 1, "{error}");
    assert!(error.to_string().contains("claude-code-cli"), "{error}");
    assert!(error.to_string().contains("folder"), "{error}");
}

#[test]
fn duplicate_declared_harness_ids_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    write_unpinned(&root, "first", "example-cli");
    write_unpinned(&root, "second", "example-cli");
    let err = goalcup_cli::connector::load_harness(&root, "example-cli", true)
        .expect_err("two declarations are ambiguous");
    assert_eq!(err.exit_code(), 1);
    assert!(err.to_string().contains("two directories"), "{err}");
    assert!(err.to_string().contains("first"), "{err}");
    assert!(err.to_string().contains("second"), "{err}");
}

#[test]
fn a_malformed_duplicate_before_or_after_the_valid_target_is_fatal() {
    for malformed_name in ["a-malformed", "z-malformed"] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        write_unpinned(&root, "m-valid", "example-cli");
        let malformed = write_unpinned(&root, malformed_name, "example-cli");
        let path = malformed.join("harness.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}\nunknown_schema_field = true\n")).unwrap();

        let error = goalcup_cli::connector::load_harness(&root, "example-cli", true)
            .expect_err("no valid target may hide its malformed duplicate");
        assert_eq!(error.exit_code(), 3);
        assert!(error.to_string().contains(malformed_name), "{error}");
        assert!(
            error.to_string().contains("unknown_schema_field"),
            "{error}"
        );
    }
}

#[test]
fn a_missing_unpinned_dev_request_inventory_keeps_the_shipped_ids() {
    let tmp = tempfile::tempdir().unwrap();
    for id in ["claude-code-cli", "codex-cli", "hermes-cli"] {
        copy_tree(&connectors().join(id), &tmp.path().join(id));
    }

    let error = goalcup_cli::connector::load_harness(tmp.path(), "missing-dev-cli", true)
        .expect_err("the requested development connector is absent");
    assert_eq!(error.exit_code(), 1);
    assert!(
        error
            .to_string()
            .contains("It ships: claude-code-cli, codex-cli, hermes-cli."),
        "{error}"
    );
}

#[test]
fn launch_uses_the_harness_bytes_captured_at_load_time() {
    use goalcup_cli::launch::{CaptureLaunch, build_capture_launch, pin_harness_executable};
    use sha2::{Digest, Sha256};

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let dir = write_unpinned(&root, "example", "example-cli");
    let exe = root.join("example-cli");
    std::fs::write(&exe, b"example executable\n").unwrap();
    let digest: String = Sha256::digest(std::fs::read(&exe).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let path = dir.join("harness.toml");
    let before = without_platform(&std::fs::read_to_string(&path).unwrap())
        .replace("program = \"claude\"", "program = \"example-cli\"")
        .replace(
            "d8cb1e5c79684cc12a8bfc813e3a2073406921b6245744b3009be3ab5651d21e",
            &digest,
        );
    std::fs::write(&path, &before).unwrap();
    let connector = goalcup_cli::connector::load_harness(&root, "example-cli", true).unwrap();
    let pinned = pin_harness_executable(&connector.loaded.manifest.capture, &exe).unwrap();

    let after = before.replace("SHELL = \"/bin/zsh\"", "SHELL = \"/after-load\"");
    assert_ne!(before, after);
    std::fs::write(&path, after).unwrap();

    let workspace = root.join("workspace");
    let run_home = root.join("run-home");
    let config_home = root.join("config-home");
    for dir in [&workspace, &run_home, &config_home] {
        std::fs::create_dir(dir).unwrap();
    }
    let ca = root.join("ca.pem");
    std::fs::write(&ca, b"test CA\n").unwrap();
    let mut vars = TemplateVars::new();
    vars.set("workspace", workspace.display().to_string());
    vars.set("session_id", "00000000-0000-4000-8000-000000000001");
    let parent = BTreeMap::new();
    let plan = build_capture_launch(
        &connector,
        &vars,
        &workspace,
        &CaptureLaunch {
            goal: b"one goal",
            executable: &pinned,
            run_home: &run_home,
            config_home: &config_home,
            proxy_url: "http://goalcup:0123456789abcdef0123456789abcdef@127.0.0.1:47123",
            ca_certificate: &ca,
            parent_env: &parent,
            allowed_tool_rule: None,
        },
    )
    .unwrap();
    assert_eq!(
        plan.env_set.get("SHELL").map(String::as_str),
        Some("/bin/zsh")
    );
}

/// `text` without its `[capture.platform.*]` tables, so a test can add its own.
fn without_platform(text: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in text.lines() {
        if line.starts_with('[') {
            skipping = line.starts_with("[capture.platform.");
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[test]
fn the_pin_and_env_follow_the_running_platform() {
    use goalcup_cli::launch::{CaptureLaunch, build_capture_launch, pin_harness_executable};
    use goalcup_protocol::harness::PlatformKey;
    use sha2::{Digest, Sha256};

    let Some(key) = PlatformKey::current() else {
        return;
    };
    let hex = |bytes: &[u8]| -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    };
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let dir = write_unpinned(&root, "example", "example-cli");
    let exe = root.join("example-cli");
    std::fs::write(&exe, b"this platform's build\n").unwrap();
    let other = root.join("other-build");
    std::fs::write(&other, b"the base platform's build\n").unwrap();
    let (this_sha, base_sha) = (
        hex(b"this platform's build\n"),
        hex(b"the base platform's build\n"),
    );
    let path = dir.join("harness.toml");
    let text = without_platform(&std::fs::read_to_string(&path).unwrap())
        .replace("program = \"claude\"", "program = \"example-cli\"")
        .replace(
            "d8cb1e5c79684cc12a8bfc813e3a2073406921b6245744b3009be3ab5651d21e",
            &base_sha,
        );
    std::fs::write(
        &path,
        format!(
            "{text}\n[capture.platform.{k}]\nsha256 = \"{this_sha}\"\nversion = \"9.9.9\"\n\n\
             [capture.platform.{k}.env]\nSHELL = \"/bin/sh\"\n",
            k = key.as_str()
        ),
    )
    .unwrap();
    let connector = goalcup_cli::connector::load_harness(&root, "example-cli", true).unwrap();
    let profile = &connector.loaded.manifest.capture;

    // The base platform's bytes are not this platform's pin, and the refusal names the pin.
    let refusal = pin_harness_executable(profile, &other).expect_err("the base build");
    let refusal = refusal.to_string();
    assert!(refusal.contains(&this_sha), "{refusal}");
    assert!(refusal.contains("version 9.9.9"), "{refusal}");

    let pinned = pin_harness_executable(profile, &exe).expect("this platform's build");
    let workspace = root.join("workspace");
    let run_home = root.join("run-home");
    let config_home = root.join("config-home");
    for dir in [&workspace, &run_home, &config_home] {
        std::fs::create_dir(dir).unwrap();
    }
    let ca = root.join("ca.pem");
    std::fs::write(&ca, b"test CA\n").unwrap();
    let mut vars = TemplateVars::new();
    vars.set("workspace", workspace.display().to_string());
    vars.set("session_id", "00000000-0000-4000-8000-000000000001");
    let parent = BTreeMap::new();
    let plan = build_capture_launch(
        &connector,
        &vars,
        &workspace,
        &CaptureLaunch {
            goal: b"one goal",
            executable: &pinned,
            run_home: &run_home,
            config_home: &config_home,
            proxy_url: "http://goalcup:0123456789abcdef0123456789abcdef@127.0.0.1:47123",
            ca_certificate: &ca,
            parent_env: &parent,
            allowed_tool_rule: None,
        },
    )
    .expect("an executable pinned with the resolved sha256 builds");
    assert_eq!(
        plan.env_set.get("SHELL").map(String::as_str),
        Some("/bin/sh")
    );
    // The rest of the base table is unchanged.
    for (k, v) in &profile.env {
        if k != "SHELL" {
            assert_eq!(plan.env_set.get(k), Some(v), "{k}");
        }
    }
}

/// A connector without `home.env` gets no home variable: the harness keeps its state under the
/// private run `HOME` the launch pins.
#[test]
fn a_harness_without_a_home_variable_gets_only_the_private_home() {
    use goalcup_cli::launch::{CaptureLaunch, build_capture_launch, pin_harness_executable};
    use sha2::{Digest, Sha256};

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let dir = write_unpinned(&root, "example", "example-cli");
    let exe = root.join("example-cli");
    std::fs::write(&exe, b"example executable\n").unwrap();
    let digest: String = Sha256::digest(std::fs::read(&exe).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let path = dir.join("harness.toml");
    let text = without_platform(&std::fs::read_to_string(&path).unwrap())
        .replace("program = \"claude\"", "program = \"example-cli\"")
        .replace(
            "d8cb1e5c79684cc12a8bfc813e3a2073406921b6245744b3009be3ab5651d21e",
            &digest,
        )
        .replace("env = \"CLAUDE_CONFIG_DIR\"\n", "")
        .replace("config_home = \"shared\"", "config_home = \"per_run\"")
        .replace(
            "root = \"{config_home}/projects\"",
            "root = \"{run_home}/.claude/projects\"",
        );
    assert!(!text.contains("CLAUDE_CONFIG_DIR"));
    std::fs::write(&path, &text).unwrap();
    let connector = goalcup_cli::connector::load_harness(&root, "example-cli", true).unwrap();
    assert_eq!(connector.loaded.manifest.capture.home.env, None);
    let pinned = pin_harness_executable(&connector.loaded.manifest.capture, &exe).unwrap();

    let workspace = root.join("workspace");
    let run_home = root.join("run-home");
    let config_home = root.join("config-home");
    for dir in [&workspace, &run_home, &config_home] {
        std::fs::create_dir(dir).unwrap();
    }
    let ca = root.join("ca.pem");
    std::fs::write(&ca, b"test CA\n").unwrap();
    let mut vars = TemplateVars::new();
    vars.set("workspace", workspace.display().to_string());
    vars.set("session_id", "00000000-0000-4000-8000-000000000001");
    let parent = BTreeMap::new();
    let plan = build_capture_launch(
        &connector,
        &vars,
        &workspace,
        &CaptureLaunch {
            goal: b"one goal",
            executable: &pinned,
            run_home: &run_home,
            config_home: &config_home,
            proxy_url: "http://goalcup:0123456789abcdef0123456789abcdef@127.0.0.1:47123",
            ca_certificate: &ca,
            parent_env: &parent,
            allowed_tool_rule: None,
        },
    )
    .unwrap();
    assert!(!plan.env_set.contains_key("CLAUDE_CONFIG_DIR"));
    let config_home = config_home.display().to_string();
    assert!(
        plan.env_set.values().all(|v| *v != config_home),
        "nothing points at the config home: {:?}",
        plan.env_set
    );
    assert_eq!(
        plan.env_set.get("HOME").map(String::as_str),
        Some(run_home.display().to_string().as_str())
    );
}
