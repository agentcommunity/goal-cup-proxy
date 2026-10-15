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

    // Chat mode (D57): the terminal is the harness's, both streams inherited, and no identity
    // at launch; the sessions are every file new or written since the run started.
    for id in ["codex-cli", "claude-code-cli"] {
        let loaded = load(id);
        let capture = &loaded.manifest.capture;
        assert!(capture.interactive, "{id}");
        let plan = StreamPlan::for_spec(capture).unwrap();
        assert!(!plan.scan_for_key, "{id}");
        assert_eq!(plan.stdout, StdoutUse::Inherit, "{id}");
        assert_eq!(plan.stderr, StderrUse::Inherit, "{id}");
        assert_eq!(
            LaunchScope::for_spec(capture).unwrap(),
            LaunchScope::None,
            "{id}"
        );
        assert!(capture.native.iter().all(|n| n.since_run_start), "{id}");
        assert_eq!(capture.home.skills_dir.as_deref(), Some("skills"), "{id}");
    }
    assert_eq!(
        load("codex-cli").manifest.capture.resume_args,
        ["resume", "--last"]
    );
    assert_eq!(
        load("claude-code-cli").manifest.capture.resume_args,
        ["--continue"]
    );
}

/// A shipped manifest with its identity table replaced, parsed without validating: what the
/// launcher copies from a headless profile that names these rules.
fn with_identity(id: &str, identity: &str) -> HarnessManifest {
    let text = std::fs::read_to_string(connectors().join(id).join("harness.toml")).unwrap();
    let from = "[capture.identity]\nsource = \"none\"";
    assert!(text.contains(from), "{id}");
    HarnessManifest::from_toml_str(&text.replace(from, &format!("[capture.identity]\n{identity}")))
        .unwrap()
}

#[test]
fn preassigned_scope_does_not_change_when_a_credential_is_present() {
    let manifest = with_identity(
        "claude-code-cli",
        "source = \"preassigned\"\nbinding = \"at_launch\"",
    );
    assert_eq!(
        LaunchScope::for_spec(&manifest.capture).unwrap(),
        LaunchScope::Preassigned
    );
    let mut capture = manifest.capture.clone();
    capture.credential = load("hermes-cli").manifest.capture.credential.clone();
    assert!(capture.credential.is_some());
    assert_eq!(
        LaunchScope::for_spec(&capture).unwrap(),
        LaunchScope::Preassigned
    );
    let streams = StreamPlan::for_spec(&capture).unwrap();
    assert!(streams.scan_for_key);
    assert_eq!(streams.stdout, StdoutUse::Forward);
    assert_eq!(streams.stderr, StderrUse::Forward);
}

#[test]
fn identity_rules_are_copied_from_the_manifest() {
    let codex = with_identity(
        "codex-cli",
        "source = \"stdout_json\"\nbinding = \"at_launch\"\nmatch_field = \"type\"\n\
         match_value = \"thread.started\"\nid_field = \"thread_id\"",
    );
    let StreamPlan {
        stdout: StdoutUse::IdentityJson(rule),
        ..
    } = StreamPlan::for_spec(&codex.capture).unwrap()
    else {
        panic!("a stdout_json profile must use its stdout JSON identity rule")
    };
    assert_eq!(rule.match_field, "type");
    assert_eq!(rule.match_value, "thread.started");
    assert_eq!(rule.id_field, "thread_id");
    assert_eq!(
        LaunchScope::for_spec(&codex.capture).unwrap(),
        LaunchScope::FromStdout
    );

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
    use goalcup_cli::launch::{CaptureLaunch, build_capture_launch, resolve_harness_executable};

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let dir = write_unpinned(&root, "example", "example-cli");
    let exe = root.join("example-cli");
    std::fs::write(&exe, b"example executable\n").unwrap();
    let path = dir.join("harness.toml");
    let before = without_platform(&std::fs::read_to_string(&path).unwrap())
        .replace("program = \"claude\"", "program = \"example-cli\"");
    std::fs::write(&path, &before).unwrap();
    let connector = goalcup_cli::connector::load_harness(&root, "example-cli", true).unwrap();
    let resolved = resolve_harness_executable(&exe).unwrap();

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
            goal: None,
            executable: &resolved,
            run_home: &run_home,
            config_home: &config_home,
            proxy_url: "http://goalcup:0123456789abcdef0123456789abcdef@127.0.0.1:47123",
            ca_certificate: &ca,
            parent_env: &parent,
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
fn the_env_follows_the_running_platform() {
    use goalcup_cli::launch::{CaptureLaunch, build_capture_launch, resolve_harness_executable};
    use goalcup_protocol::harness::PlatformKey;

    let Some(key) = PlatformKey::current() else {
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let dir = write_unpinned(&root, "example", "example-cli");
    let exe = root.join("example-cli");
    std::fs::write(&exe, b"any build\n").unwrap();
    let path = dir.join("harness.toml");
    let text = without_platform(&std::fs::read_to_string(&path).unwrap())
        .replace("program = \"claude\"", "program = \"example-cli\"");
    std::fs::write(
        &path,
        format!(
            "{text}\n[capture.platform.{k}.env]\nSHELL = \"/bin/sh\"\n",
            k = key.as_str()
        ),
    )
    .unwrap();
    let connector = goalcup_cli::connector::load_harness(&root, "example-cli", true).unwrap();
    let profile = &connector.loaded.manifest.capture;

    let resolved = resolve_harness_executable(&exe).expect("any regular file");
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
            goal: None,
            executable: &resolved,
            run_home: &run_home,
            config_home: &config_home,
            proxy_url: "http://goalcup:0123456789abcdef0123456789abcdef@127.0.0.1:47123",
            ca_certificate: &ca,
            parent_env: &parent,
        },
    )
    .expect("the plan builds for any build");
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
    use goalcup_cli::launch::{CaptureLaunch, build_capture_launch, resolve_harness_executable};

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let dir = write_unpinned(&root, "example", "example-cli");
    let exe = root.join("example-cli");
    std::fs::write(&exe, b"example executable\n").unwrap();
    let path = dir.join("harness.toml");
    let text = without_platform(&std::fs::read_to_string(&path).unwrap())
        .replace("program = \"claude\"", "program = \"example-cli\"")
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
    let resolved = resolve_harness_executable(&exe).unwrap();

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
            goal: None,
            executable: &resolved,
            run_home: &run_home,
            config_home: &config_home,
            proxy_url: "http://goalcup:0123456789abcdef0123456789abcdef@127.0.0.1:47123",
            ca_certificate: &ca,
            parent_env: &parent,
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

/// A chat-mode plan (D57) places no goal and keeps the terminal: its argv is the connector's
/// rendered args alone, it is a `Tui` launch (never a group of its own, so the harness stays in
/// the terminal's foreground group), and a goal is refused, as a headless plan refuses none.
#[test]
fn an_interactive_plan_places_no_goal_and_keeps_the_terminal() {
    use goalcup_cli::launch::{CaptureLaunch, build_capture_launch, resolve_harness_executable};
    use goalcup_protocol::manifest::LaunchMode;

    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let exe = root.join("harness");
    std::fs::write(&exe, b"any build\n").unwrap();
    let resolved = resolve_harness_executable(&exe).unwrap();
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
    let launch = |goal| CaptureLaunch {
        goal,
        executable: &resolved,
        run_home: &run_home,
        config_home: &config_home,
        proxy_url: "http://goalcup:0123456789abcdef0123456789abcdef@127.0.0.1:47123",
        ca_certificate: &ca,
        parent_env: &parent,
    };
    let codex_argv = vec![
        "-c".to_string(),
        "check_for_update_on_startup=false".to_string(),
        "-c".to_string(),
        format!(
            "projects={{ \"{}\" = {{ trust_level = \"trusted\" }} }}",
            workspace.display()
        ),
        "-C".to_string(),
        workspace.display().to_string(),
    ];
    for (id, argv) in [("codex-cli", codex_argv), ("claude-code-cli", Vec::new())] {
        let connector =
            goalcup_cli::connector::load_harness(&connectors(), id, false).expect("shipped");
        let plan = build_capture_launch(&connector, &vars, &workspace, &launch(None))
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(plan.argv, argv, "{id}");
        assert_eq!(plan.capture, None, "{id}: no goal slot");
        assert_eq!(
            plan.mode,
            LaunchMode::Tui,
            "{id}: the terminal's foreground group"
        );
        let refused = build_capture_launch(&connector, &vars, &workspace, &launch(Some(b"goal")))
            .expect_err("a goal for a chat-mode connector");
        assert!(refused.to_string().contains("chat mode"), "{id}: {refused}");
    }

    // A headless connector refuses a launch with no goal.
    let hermes = goalcup_cli::connector::load_harness(&connectors(), "hermes-cli", false).unwrap();
    let refused = build_capture_launch(&hermes, &vars, &workspace, &launch(None))
        .expect_err("no goal for a headless connector");
    assert!(refused.to_string().contains("headless"), "{refused}");
}
