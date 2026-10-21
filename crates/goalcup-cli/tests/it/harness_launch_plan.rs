//! Pure decisions derived from harness-contract schema 1, on the fixture connectors
//! (`tests/fixtures/connectors/`): the headless per-run shape and the chat-mode shared-home shape.
//! What every shipped connector is held to is the connector contract, `it/connector_contract.rs`.

use crate::common::support;
use goalcup_cli::launch::{LaunchScope, StderrUse, StdoutUse, StreamPlan, harness_arg_refusal};
use goalcup_cli::preflight::{Level, harness_pin_gate};
use goalcup_protocol::harness::{HarnessManifest, Origin, load_dir};
use goalcup_protocol::pins::shipped_connector_ids;
use goalcup_protocol::template::TemplateVars;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn connectors() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors")
}

/// A fixture connector, loaded from its folder.
fn fixture(id: &str) -> goalcup_protocol::harness::LoadedConnector {
    load_dir(&support::fixture_connectors_dir().join(id), Origin::Pinned)
        .expect("the fixture connector loads")
}

/// The first shipped connector: the subject of a rule about pinned bytes, which only a shipped
/// connector has.
fn a_shipped_id() -> String {
    shipped_connector_ids()[0].clone()
}

/// A fixture manifest with its identity table replaced, parsed without validating: what the
/// launcher copies from a headless profile that names these rules.
fn with_identity(id: &str, identity: &str) -> HarnessManifest {
    let text = std::fs::read_to_string(
        support::fixture_connectors_dir()
            .join(id)
            .join("harness.toml"),
    )
    .unwrap();
    let from = "[capture.identity]\nsource = \"none\"";
    assert!(text.contains(from), "{id}");
    HarnessManifest::from_toml_str(&text.replace(from, &format!("[capture.identity]\n{identity}")))
        .unwrap()
}

#[test]
fn identity_rules_are_copied_from_the_manifest() {
    let json = with_identity(
        "chat-shared-home",
        "source = \"stdout_json\"\nbinding = \"at_launch\"\nmatch_field = \"type\"\n\
         match_value = \"thread.started\"\nid_field = \"thread_id\"",
    );
    let StreamPlan {
        stdout: StdoutUse::IdentityJson(rule),
        ..
    } = StreamPlan::for_spec(&json.capture).unwrap()
    else {
        panic!("a stdout_json profile must use its stdout JSON identity rule")
    };
    assert_eq!(rule.match_field, "type");
    assert_eq!(rule.match_value, "thread.started");
    assert_eq!(rule.id_field, "thread_id");
    assert_eq!(
        LaunchScope::for_spec(&json.capture).unwrap(),
        LaunchScope::FromStdout
    );

    let per_run = fixture("headless-per-run");
    let StreamPlan {
        stderr: StderrUse::IdentityLine(rule),
        ..
    } = StreamPlan::for_spec(&per_run.manifest.capture).unwrap()
    else {
        panic!("a stderr_line profile must use its stderr identity rule")
    };
    assert_eq!(rule.prefix, "session_id: ");
    assert_eq!(rule.grammar.as_str(), "timestamp-hex");
}

#[test]
fn argv_guard_comes_from_the_harness_manifest() {
    let per_run = fixture("headless-per-run");
    for arg in ["--profile", "--pro", "--tui", "--he", "-p", "-h", "-Qp"] {
        assert!(
            harness_arg_refusal(&per_run.manifest.capture, arg).is_some(),
            "the guard must refuse {arg}"
        );
    }
    for arg in [
        "--provider",
        "--model",
        "--query=hello",
        "--unknown",
        "--cli",
        "-x",
        "-mp",
        "plain",
    ] {
        assert_eq!(
            harness_arg_refusal(&per_run.manifest.capture, arg),
            None,
            "the guard must allow {arg}"
        );
    }
}

/// The chat-mode fixture connector (program `example-cli`) under `id`, in `root/dir_name`.
fn write_unpinned(root: &Path, dir_name: &str, id: &str) -> PathBuf {
    std::fs::create_dir_all(root).unwrap();
    support::copy_fixture_connector("chat-shared-home", root, dir_name, id)
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

/// A manifest with no `[participant]` table is named by its id, and has no first-screen hint.
#[test]
fn a_connector_without_a_participant_table_is_named_by_its_id() {
    let tmp = tempfile::tempdir().unwrap();
    write_unpinned(tmp.path(), "example-cli", "example-cli");
    let loaded = goalcup_cli::connector::load_harness(tmp.path(), "example-cli", true).unwrap();
    assert_eq!(loaded.display_name(), "example-cli");
    assert_eq!(loaded.first_screen(), None);
}

#[test]
fn a_mixed_pinned_and_development_root_loads_each_by_its_captured_identity() {
    let tmp = tempfile::tempdir().unwrap();
    let id = a_shipped_id();
    copy_tree(&connectors().join(&id), &tmp.path().join(&id));
    write_unpinned(tmp.path(), "z-development", "example-cli");

    let loaded = goalcup_cli::connector::load_harness(tmp.path(), "example-cli", true).unwrap();
    assert_eq!(loaded.loaded.manifest.id, "example-cli");
    assert_eq!(loaded.dir.file_name().unwrap(), "z-development");
    assert_eq!(harness_pin_gate(&loaded, true).level, Level::Warn);
}

#[test]
fn a_byte_identical_shipped_connector_under_an_alias_is_refused_at_cli_discovery() {
    let tmp = tempfile::tempdir().unwrap();
    let id = a_shipped_id();
    copy_tree(&connectors().join(&id), &tmp.path().join("alias"));

    let error = goalcup_cli::connector::load_harness(tmp.path(), &id, false)
        .expect_err("shipped bytes have one admitted folder");
    assert_eq!(error.exit_code(), 1, "{error}");
    assert!(error.to_string().contains(&id), "{error}");
    assert!(error.to_string().contains("folder"), "{error}");
}

#[test]
fn altered_pinned_id_is_never_relaxed_by_dev() {
    let tmp = tempfile::tempdir().unwrap();
    let copied = tmp.path().join("renamed");
    std::fs::create_dir_all(&copied).unwrap();
    let id = a_shipped_id();
    std::fs::copy(
        connectors().join(&id).join("harness.toml"),
        copied.join("harness.toml"),
    )
    .unwrap();
    let error = goalcup_cli::connector::load_harness(tmp.path(), &id, true)
        .expect_err("a pinned id under an alias is refused before dev preflight");
    assert_eq!(error.exit_code(), 1, "{error}");
    assert!(error.to_string().contains(&id), "{error}");
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
    let shipped = shipped_connector_ids();
    for id in &shipped {
        copy_tree(&connectors().join(id), &tmp.path().join(id));
    }

    let error = goalcup_cli::connector::load_harness(tmp.path(), "missing-dev-cli", true)
        .expect_err("the requested development connector is absent");
    assert_eq!(error.exit_code(), 1);
    assert!(
        error
            .to_string()
            .contains(&format!("It ships: {}.", shipped.join(", "))),
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
    let before = without_platform(&std::fs::read_to_string(&path).unwrap());
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
            ca_bundle: None,
            parent_env: &parent,
            time_left_file: None,
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
    let text = without_platform(&std::fs::read_to_string(&path).unwrap());
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
            ca_bundle: None,
            parent_env: &parent,
            time_left_file: None,
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

/// The CA files `goalcup start` hands, built the way it builds them (`ca_bundle::bundle_for`,
/// then the launch): every certificate in the file each `capture.ca.env` variable is pointed at.
fn handed_ca_files(
    connector: &goalcup_cli::connector::HarnessConnector,
    root: &Path,
) -> (BTreeMap<String, Vec<Vec<u8>>>, Vec<u8>, bool) {
    use goalcup_cli::ca_bundle::{bundle_for, certificates};
    use goalcup_cli::launch::{CaptureLaunch, build_capture_launch, resolve_harness_executable};

    let exe = root.join("harness");
    std::fs::write(&exe, b"any build\n").unwrap();
    let resolved = resolve_harness_executable(&exe).unwrap();
    let workspace = root.join("workspace");
    let run_home = root.join("run-home");
    let config_home = root.join("config-home");
    let ca_dir = root.join("ca");
    for dir in [&workspace, &run_home, &config_home, &ca_dir] {
        std::fs::create_dir(dir).unwrap();
    }
    let proxy = goalcup_cli::proxy::tls::ProxyCa::generate(&["api.example.org".to_string()])
        .expect("a proxy CA");
    let ca = proxy.write_certificate(&ca_dir).expect("the proxy CA file");
    let capture = &connector.loaded.manifest.capture;
    let bundle = bundle_for(capture, &ca_dir, proxy.certificate_der()).expect("the bundle");
    let mut vars = TemplateVars::new();
    vars.set("workspace", workspace.display().to_string());
    vars.set("session_id", "00000000-0000-4000-8000-000000000001");
    let parent = BTreeMap::new();
    let goal: &[u8] = b"goal";
    let plan = build_capture_launch(
        connector,
        &vars,
        &workspace,
        &CaptureLaunch {
            goal: (!capture.interactive).then_some(goal),
            executable: &resolved,
            run_home: &run_home,
            config_home: &config_home,
            proxy_url: "http://goalcup:0123456789abcdef0123456789abcdef@127.0.0.1:47123",
            ca_certificate: &ca,
            ca_bundle: bundle.as_deref(),
            parent_env: &parent,
            time_left_file: None,
        },
    )
    .unwrap();
    let files = capture
        .ca
        .env
        .iter()
        .map(|name| {
            let file = plan.env_set.get(name).expect("every CA variable is set");
            (name.clone(), certificates(&std::fs::read(file).unwrap()))
        })
        .collect();
    (files, proxy.certificate_der().to_vec(), bundle.is_some())
}

/// A connector that trusts the recording proxy through `SSL_CERT_FILE` needs no CA variable of its
/// own and no Rust row (WP12): `goalcup start` hands that shared row the proxy CA first and then
/// the system's roots, and in the same profile still hands the harness's own variable the proxy CA
/// alone. Env values that name the run's two homes render to them.
#[test]
fn a_shared_ca_row_gets_the_roots_and_an_own_variable_beside_it_does_not() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let dir = write_unpinned(&root, "example", "example-cli");
    let path = dir.join("harness.toml");
    let text = without_platform(&std::fs::read_to_string(&path).unwrap())
        .replace("program = \"example-cli\"", "program = \"harness\"")
        .replace(
            "env = [\"NODE_EXTRA_CA_CERTS\"]\nmode = \"extend\"",
            "env = [\"SSL_CERT_FILE\", \"NODE_EXTRA_CA_CERTS\"]\nmode = \"extend\"",
        )
        .replace(
            "[capture.env]\n",
            "[capture.env]\nEXAMPLE_DATA_DIR = \"{config_home}/data\"\n\
             EXAMPLE_CACHE_DIR = \"{run_home}/.cache/example\"\n",
        );
    assert!(text.contains("SSL_CERT_FILE") && text.contains("EXAMPLE_DATA_DIR"));
    std::fs::write(&path, &text).unwrap();
    let connector = goalcup_cli::connector::load_harness(&root, "example-cli", true).unwrap();

    let (files, proxy, bundle) = handed_ca_files(&connector, &root);
    assert!(bundle, "a shared row has a roots file written for it");
    let shared = &files["SSL_CERT_FILE"];
    assert_eq!(shared[0], proxy, "the proxy CA comes first");
    assert!(
        shared.len() >= 21,
        "the system's roots follow: {}",
        shared.len()
    );
    assert_eq!(files["NODE_EXTRA_CA_CERTS"], [proxy]);

    // The same launch, with the token values, and a launch with no roots file is refused.
    use goalcup_cli::launch::{CaptureLaunch, build_capture_launch, resolve_harness_executable};
    let resolved = resolve_harness_executable(&root.join("harness")).unwrap();
    let mut vars = TemplateVars::new();
    vars.set("workspace", root.join("workspace").display().to_string());
    let parent = BTreeMap::new();
    let ca = root.join("ca.pem");
    std::fs::write(&ca, b"test CA\n").unwrap();
    let (run_home, config_home) = (root.join("run-home"), root.join("config-home"));
    let launch = |ca_bundle| CaptureLaunch {
        goal: None,
        executable: &resolved,
        run_home: &run_home,
        config_home: &config_home,
        proxy_url: "http://goalcup:0123456789abcdef0123456789abcdef@127.0.0.1:47123",
        ca_certificate: &ca,
        ca_bundle,
        parent_env: &parent,
        time_left_file: None,
    };
    let workspace = root.join("workspace");
    let refused = build_capture_launch(&connector, &vars, &workspace, &launch(None))
        .expect_err("a shared row without a roots file");
    assert!(refused.to_string().contains("SSL_CERT_FILE"), "{refused}");
    let plan = build_capture_launch(&connector, &vars, &workspace, &launch(Some(&ca))).unwrap();
    assert_eq!(
        plan.env_set.get("EXAMPLE_DATA_DIR"),
        Some(&format!("{}/data", config_home.display()))
    );
    assert_eq!(
        plan.env_set.get("EXAMPLE_CACHE_DIR"),
        Some(&format!("{}/.cache/example", run_home.display()))
    );
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
        .replace("env = \"CHAT_CONFIG_DIR\"\n", "")
        .replace("config_home = \"shared\"", "config_home = \"per_run\"")
        .replace(
            "root = \"{config_home}/projects\"",
            "root = \"{run_home}/.chat/projects\"",
        );
    assert!(!text.contains("CHAT_CONFIG_DIR"));
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
            ca_bundle: None,
            parent_env: &parent,
            time_left_file: None,
        },
    )
    .unwrap();
    assert!(!plan.env_set.contains_key("CHAT_CONFIG_DIR"));
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

/// The one check `goalcup start`, `goalcup doctor` and `goalcup prep` share: a chat-mode
/// harness that is a `#!/usr/bin/env node` script (the npm harnesses) resolves only when `node`
/// is on the PATH the harness is given, which is the participant's own; the resolution says so,
/// and the version probe and the launch use that same PATH.
#[test]
fn a_node_script_harness_resolves_only_with_node_on_the_path_it_is_given() {
    use goalcup_cli::launch::{capture_executable, harness_path};
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let dir = write_unpinned(&root, "example", "example-cli");
    let path = dir.join("harness.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text).unwrap();
    let connector = goalcup_cli::connector::load_harness(&root, "example-cli", true).unwrap();
    let profile = &connector.loaded.manifest.capture;
    assert!(profile.interactive, "the fixture is chat mode");

    let bin = root.join("bin");
    std::fs::create_dir(&bin).unwrap();
    let exe = bin.join("example-cli");
    std::fs::write(&exe, "#!/usr/bin/env node\nconsole.log('hi')\n").unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    let env = BTreeMap::from([("PATH".to_string(), bin.display().to_string())]);
    assert_eq!(harness_path(profile, &env), bin.display().to_string());

    let refused = capture_executable(&connector, profile, &env).expect_err("no node on that PATH");
    assert!(refused.to_string().contains("`node` script"), "{refused}");

    let node = bin.join("node");
    std::fs::write(&node, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).unwrap();
    let resolved = capture_executable(&connector, profile, &env).expect("node is there now");
    assert_eq!(resolved.search_path(), bin.display().to_string());
}

/// A hostile goal is one `--query=<goal>` element wherever it would otherwise be read by the
/// harness's raw argv scan (its `argv_guard`) or by its argument parser, and the plan's `Debug`
/// shows its length, never the goal. The subject is the headless fixture connector
/// (`goal_flag = "--query"`).
#[test]
fn hostile_goals_stay_one_goal_flag_element_and_the_plan_never_shows_them() {
    use crate::common::m2_profile_support as s;
    let l = s::Layout::new();
    let exe = s::fake_harness(&l.bin);
    let c = support::fixture_connector("headless-per-run");
    let resolved = s::resolved(&exe);
    let proxy = s::proxy_url();
    let parent = s::parent_env();
    let profile = &c.loaded.manifest.capture;
    for goal in [
        "--profile=x",
        "-p x",
        "--tui",
        "-h",
        "a=b",
        "-leading dash",
        "line one\n--tui\nline three",
        "--query=twice",
        "--",
    ] {
        let plan = goalcup_cli::launch::build_capture_launch(
            &c,
            &s::vars(&l),
            &l.workspace,
            &s::launch(&l, goal.as_bytes(), &resolved, &proxy, &parent),
        )
        .unwrap_or_else(|e| panic!("{goal:?}: {e}"));
        assert_eq!(plan.argv.len(), profile.args.len() + 1, "{goal:?}");
        assert_eq!(
            &plan.argv[..profile.args.len()],
            profile.args.as_slice(),
            "{goal:?}: the profile's args come first and unchanged"
        );
        assert_eq!(
            plan.argv.last().unwrap(),
            &format!("--query={goal}"),
            "{goal:?}: the goal is the value of the one flag element"
        );
        assert!(
            !plan.argv.iter().any(|a| a == "--"),
            "{goal:?}: this recipe has no `--`"
        );
        let capture = plan.capture.as_ref().unwrap();
        assert_eq!(capture.goal_index, plan.argv.len() - 1);
        assert_eq!(capture.goal_prefix, "--query=".len());

        for (surface, text) in [
            ("Debug", format!("{plan:?}")),
            ("pretty Debug", format!("{plan:#?}")),
        ] {
            // A two-byte goal such as `-h` occurs inside ordinary path text (`run-home`), so the
            // whole goal is searched for only when it is long enough to mean something, and the
            // element the goal rides in is searched for always.
            assert!(
                !text.contains(&format!("--query={goal}")),
                "{goal:?}: {surface} shows the goal element"
            );
            if goal.len() >= 6 {
                assert!(!text.contains(goal), "{goal:?}: {surface} shows the goal");
            }
        }
        let debugged = format!("{plan:?}");
        assert!(
            debugged.contains(&format!("--query=<goal: {} bytes>", goal.len())),
            "{debugged}"
        );
    }
}
