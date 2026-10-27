//! Pure decisions derived from harness-contract schema 1, on the fixture connectors
//! (`tests/fixtures/connectors/`): the per-run shape and the shared-home shape, both chat mode.
//! What every shipped connector is held to is the connector contract, `it/connector_contract.rs`.

use crate::common::support;
use goalcup_cli::launch::harness_arg_refusal;
use goalcup_cli::preflight::{Level, harness_pin_gate};
use goalcup_protocol::harness::{Origin, load_dir};
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

#[test]
fn argv_guard_comes_from_the_harness_manifest() {
    let per_run = fixture("chat-per-run");
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
    let parent = BTreeMap::new();
    let plan = build_capture_launch(
        &connector,
        &vars,
        &workspace,
        &CaptureLaunch {
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
    let parent = BTreeMap::new();
    let plan = build_capture_launch(
        &connector,
        &vars,
        &workspace,
        &CaptureLaunch {
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
    let parent = BTreeMap::new();
    let plan = build_capture_launch(
        connector,
        &vars,
        &workspace,
        &CaptureLaunch {
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
    let parent = BTreeMap::new();
    let plan = build_capture_launch(
        &connector,
        &vars,
        &workspace,
        &CaptureLaunch {
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

    let bin = root.join("bin");
    std::fs::create_dir(&bin).unwrap();
    let exe = bin.join("example-cli");
    std::fs::write(&exe, "#!/usr/bin/env node\nconsole.log('hi')\n").unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    let env = BTreeMap::from([("PATH".to_string(), bin.display().to_string())]);
    assert_eq!(harness_path(&env), bin.display().to_string());

    let refused = capture_executable(&connector, profile, &env).expect_err("no node on that PATH");
    assert!(refused.to_string().contains("`node` script"), "{refused}");

    let node = bin.join("node");
    std::fs::write(&node, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).unwrap();
    let resolved = capture_executable(&connector, profile, &env).expect("node is there now");
    assert_eq!(resolved.search_path(), bin.display().to_string());
}

/// D72: `[launch_prep]` writes the run's CA alone into the config home (mode 0600, replacing a
/// link there rather than writing through it) before its commands run; the commands run in
/// order in the harness home with `{model_base_url}` and `{ca_file}` filled in; and a command
/// that ends non-zero is written to the log in its template form, never with the URL or the
/// path, and the run goes on.
#[test]
fn launch_prep_writes_the_ca_then_runs_its_commands_and_only_logs_a_failure() {
    use goalcup_cli::launch::{
        LaunchPrepValues, PrepLaunch, launch_prep, resolve_harness_executable,
    };
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let dir = write_unpinned(&root, "example", "example-cli");
    let path = dir.join("harness.toml");
    let text = std::fs::read_to_string(&path).unwrap().replace(
        "resume_args = [\"--continue\"]\n",
        "resume_args = [\"--continue\"]\n\
         model_host_from = { file = \"config.yaml\", key = \"model.base_url\" }\n",
    ) + r#"
[launch_prep]
ca_file = "recorder-ca.pem"
commands = [
  ["config", "set", "providers.recorder.base_url", "{model_base_url}"],
  ["config", "set", "providers.recorder.ssl_ca_cert", "{ca_file}"],
  ["gateway", "stop", "-y"],
]
"#;
    std::fs::write(&path, text).unwrap();
    let connector = goalcup_cli::connector::load_harness(&root, "example-cli", true).unwrap();

    // The stand-in notes each call's arguments and whether the CA file was there, and fails
    // `gateway stop` as an older harness without `-y` does.
    let exe = root.join("example-cli");
    std::fs::write(
        &exe,
        "#!/bin/sh\n\
         if [ -f \"$CHAT_CONFIG_DIR/recorder-ca.pem\" ]; then ca=ca; else ca=no-ca; fi\n\
         printf '%s|' \"$@\" >> \"$CHAT_CONFIG_DIR/calls\"\n\
         echo \"$ca\" >> \"$CHAT_CONFIG_DIR/calls\"\n\
         [ \"$1\" = gateway ] && exit 2\n\
         exit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    let resolved = resolve_harness_executable(&exe).unwrap();
    let (workspace, run_home, config_home) = (
        root.join("workspace"),
        root.join("run-home"),
        root.join("config-home"),
    );
    for d in [&workspace, &run_home, &config_home] {
        std::fs::create_dir(d).unwrap();
    }
    // A link where the CA file goes must be replaced, not written through.
    let victim = root.join("victim");
    std::fs::write(&victim, "keep\n").unwrap();
    std::os::unix::fs::symlink(&victim, config_home.join("recorder-ca.pem")).unwrap();

    let der = b"not really a certificate, but bytes".to_vec();
    let model_host = goalcup_cli::model_host::ModelHost {
        host: "api.models.example.test".into(),
        port: 8443,
        base_path: "/v1".into(),
        from: "model.base_url in config.yaml".into(),
    };
    let parent: BTreeMap<String, String> = [("PATH".to_string(), "/usr/bin:/bin".to_string())]
        .into_iter()
        .collect();
    let log_path = root.join("host.log");
    let log = std::fs::File::create(&log_path).unwrap();
    launch_prep(
        &connector,
        &PrepLaunch {
            executable: &resolved,
            run_home: &run_home,
            config_home: &config_home,
            parent_env: &parent,
        },
        &workspace,
        &LaunchPrepValues {
            ca_certificate_der: &der,
            model_host: Some(&model_host),
        },
        &log,
    )
    .expect("a failed launch prep command does not stop the run");

    let ca_path = config_home.join("recorder-ca.pem");
    let meta = std::fs::symlink_metadata(&ca_path).unwrap();
    assert!(
        meta.file_type().is_file(),
        "the link was replaced by a file"
    );
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep\n");
    let pem = std::fs::read_to_string(&ca_path).unwrap();
    assert!(pem.starts_with("-----BEGIN CERTIFICATE-----\n"), "{pem}");
    assert!(pem.ends_with("-----END CERTIFICATE-----\n"), "{pem}");
    // The one block's body is the run's proxy CA, byte for byte (the stand-in DER is not a
    // parseable certificate, so the body is decoded here rather than by `ca_bundle`).
    let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
    assert_eq!(pem.matches("-----BEGIN").count(), 1, "{pem}");
    {
        use base64::Engine as _;
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(body)
                .unwrap(),
            der,
            "the file holds the run's proxy CA alone"
        );
    }

    let url = "https://api.models.example.test:8443/v1";
    let ca = ca_path.display().to_string();
    let calls = std::fs::read_to_string(config_home.join("calls")).unwrap();
    assert_eq!(
        calls.lines().collect::<Vec<_>>(),
        vec![
            format!("config|set|providers.recorder.base_url|{url}|ca"),
            format!("config|set|providers.recorder.ssl_ca_cert|{ca}|ca"),
            "gateway|stop|-y|ca".to_string(),
        ]
    );
    let logged = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        logged.contains("`example-cli gateway stop -y` ended with exit code 2"),
        "{logged}"
    );
    assert_eq!(logged.lines().count(), 1, "{logged}");
    assert!(!logged.contains(url) && !logged.contains(&ca), "{logged}");

    // The URL leaves out a 443 port.
    let default_port = goalcup_cli::model_host::ModelHost {
        port: 443,
        ..model_host
    };
    assert_eq!(
        goalcup_cli::launch::model_base_url(&default_port),
        "https://api.models.example.test/v1"
    );
}

/// D72 (0.7.1): `[launch_prep] after_exit` runs its commands in order in the harness home, with
/// the harness's own environment and no proxy; a command that ends non-zero is written to the log
/// and the next one still runs; and the launch prep `commands` are not run again.
#[test]
fn after_exit_runs_its_commands_in_order_and_only_logs_a_failure() {
    use goalcup_cli::launch::{PrepLaunch, after_exit, resolve_harness_executable};
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let dir = write_unpinned(&root, "example", "example-cli");
    let path = dir.join("harness.toml");
    let text = std::fs::read_to_string(&path).unwrap()
        + r#"
[launch_prep]
commands = [["config", "set", "before", "launch"]]
after_exit = [["gateway", "stop", "-y"], ["gateway", "status"]]
"#;
    std::fs::write(&path, text).unwrap();
    let connector = goalcup_cli::connector::load_harness(&root, "example-cli", true).unwrap();

    // The stand-in notes each call's arguments and whether a proxy is in its environment, and
    // fails `gateway stop` as Hermes 0.19.0 does.
    let exe = root.join("example-cli");
    std::fs::write(
        &exe,
        "#!/bin/sh\n\
         printf '%s|' \"$@\" >> \"$CHAT_CONFIG_DIR/calls\"\n\
         echo \"${HTTPS_PROXY:-no-proxy}\" >> \"$CHAT_CONFIG_DIR/calls\"\n\
         [ \"$2\" = stop ] && exit 2\n\
         exit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    let resolved = resolve_harness_executable(&exe).unwrap();
    let (workspace, run_home, config_home) = (
        root.join("workspace"),
        root.join("run-home"),
        root.join("config-home"),
    );
    for d in [&workspace, &run_home, &config_home] {
        std::fs::create_dir(d).unwrap();
    }
    let parent: BTreeMap<String, String> = [("PATH".to_string(), "/usr/bin:/bin".to_string())]
        .into_iter()
        .collect();
    let log_path = root.join("host.log");
    let log = std::fs::File::create(&log_path).unwrap();
    after_exit(
        &connector,
        &PrepLaunch {
            executable: &resolved,
            run_home: &run_home,
            config_home: &config_home,
            parent_env: &parent,
        },
        &workspace,
        &log,
    );

    let calls = std::fs::read_to_string(config_home.join("calls")).unwrap();
    assert_eq!(
        calls.lines().collect::<Vec<_>>(),
        vec!["gateway|stop|-y|no-proxy", "gateway|status|no-proxy"]
    );
    let logged = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        logged.contains("`example-cli gateway stop -y` ended with exit code 2; the run goes on"),
        "{logged}"
    );
    assert_eq!(logged.lines().count(), 1, "{logged}");
}
