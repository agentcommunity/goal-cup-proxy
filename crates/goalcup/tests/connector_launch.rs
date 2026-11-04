//! The harness environment (PRD-11 section 3): the participant's own variables pass through,
//! their proxy and CA variables are replaced, the code never sets the variables that turn TLS
//! checks off, and no secret file is read in the default build. Also the home lock and settings.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::SystemTime;

use goalcup::cli::text;
use goalcup::connector::{self, Env, LaunchPlan, Refusal};

const PROXY_AND_CA: &[&str] = &[
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
    "NODE_EXTRA_CA_CERTS",
    "SSL_CERT_FILE",
    "REQUESTS_CA_BUNDLE",
];

const NEVER_SET: &[&str] = &[
    "SSL_CERT_DIR",
    "CURL_CA_BUNDLE",
    "NODE_TLS_REJECT_UNAUTHORIZED",
];

fn env(dir: &Path, extra: &[(&str, &str)]) -> Env {
    let mut vars: BTreeMap<String, String> = BTreeMap::new();
    vars.insert("PATH".into(), format!("{}:/usr/bin:/bin", dir.display()));
    vars.insert("OPENAI_API_KEY".into(), "participant-key-value".into());
    for name in PROXY_AND_CA {
        vars.insert((*name).into(), "participant-value".into());
    }
    for (name, value) in extra {
        vars.insert((*name).into(), (*value).into());
    }
    Env {
        vars,
        workspace: dir.join("workspace"),
        resume: false,
        run_start: SystemTime::now(),
        goalcup_home: dir.join("goalcup"),
        server_host: "ingest.example.org".into(),
    }
}

fn finished(id: &str, env: &Env, dir: &Path) -> LaunchPlan {
    let c = connector::load(id, None).unwrap();
    let home = dir.join("goalcup/homes").join(id);
    let mut plan = connector::plan(&c, env, &home);
    plan.finish_env(40123, &dir.join("run-ca.pem"), &dir.join("bundle.pem"));
    plan
}

#[test]
fn participant_keys_pass_and_proxy_and_ca_variables_are_ours() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let e = env(d, &[("CLAUDE_CODE_CERT_STORE", "participant-value")]);
    let plan = finished("claude-code-cli", &e, d);
    let v = &plan.env;

    assert_eq!(v["OPENAI_API_KEY"], "participant-key-value");
    assert!(!v.contains_key("CLAUDE_CODE_CERT_STORE"), "unset removes");
    for name in ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
        assert!(!v.contains_key(name), "{name} is removed");
    }
    let proxy = "http://127.0.0.1:40123";
    assert_eq!(v["HTTPS_PROXY"], proxy);
    assert_eq!(v["https_proxy"], proxy);
    assert_eq!(v["NO_PROXY"], "127.0.0.1,localhost,::1");
    assert_eq!(v["no_proxy"], "127.0.0.1,localhost,::1");
    assert_eq!(v["NODE_USE_ENV_PROXY"], "1");
    let run_ca = d.join("run-ca.pem").to_string_lossy().into_owned();
    let bundle = d.join("bundle.pem").to_string_lossy().into_owned();
    assert_eq!(v["NODE_EXTRA_CA_CERTS"], run_ca);
    assert_eq!(v["SSL_CERT_FILE"], bundle);
    assert_eq!(v["REQUESTS_CA_BUNDLE"], bundle);
    for name in NEVER_SET {
        assert!(!v.contains_key(*name), "{name} is never set by goalcup");
    }
    let home = d.join("goalcup/homes/claude-code-cli");
    assert_eq!(v["CLAUDE_CONFIG_DIR"], home.to_string_lossy());
    assert!(home.is_dir());
}

#[test]
fn tls_variables_the_participant_exported_pass_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let theirs = [
        ("SSL_CERT_DIR", "/their/certs"),
        ("CURL_CA_BUNDLE", "/their/bundle.pem"),
        ("NODE_TLS_REJECT_UNAUTHORIZED", "1"),
    ];
    let plan = finished("codex-cli", &env(d, &theirs), d);
    for (name, value) in theirs {
        assert_eq!(plan.env[name], value);
    }
}

#[test]
fn a_harness_ca_variable_gets_the_run_ca() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let plan = finished("codex-cli", &env(d, &[]), d);
    assert_eq!(
        plan.env["CODEX_CA_CERTIFICATE"],
        d.join("run-ca.pem").to_string_lossy()
    );
}

#[test]
fn codex_gets_no_dash_c_override() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    for resume in [false, true] {
        let mut e = env(d, &[]);
        e.resume = resume;
        let plan = finished("codex-cli", &e, d);
        assert!(!plan.argv.iter().any(|a| a == "-c"), "{:?}", plan.argv);
        assert!(
            plan.argv
                .contains(&e.workspace.to_string_lossy().into_owned())
        );
    }
}

#[test]
fn no_secret_value_is_read_in_the_default_build() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let e = env(d, &[]);
    for (id, _) in connector::builtin::CONNECTORS {
        let home = d.join("goalcup/homes").join(id);
        std::fs::create_dir_all(&home).unwrap();
        for file in ["auth.json", ".env", ".credentials.json"] {
            std::fs::write(home.join(file), "{\"token\":\"fixture-token\"}").unwrap();
        }
        let plan = finished(id, &e, d);
        assert!(plan.secret_values.is_empty(), "{id}");
    }
}

/// The plan for a connector whose program, on `PATH`, runs `body`.
fn probe(d: &Path, body: &str) -> LaunchPlan {
    const FAKE: &str = "id = \"fake-cli\"\nname = \"F\"\nprogram = \"fakev\"\nversion = \"2.\"";
    let (program, file) = (d.join("fakev"), d.join("fake-cli.toml"));
    std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(&file, FAKE).unwrap();
    let c = connector::load(file.to_str().unwrap(), None).unwrap();
    connector::plan(&c, &env(d, &[]), &d.join("home"))
}

#[test]
fn a_held_home_refuses_and_the_version_probe_never_holds_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let (d, e) = (dir.path(), env(dir.path(), &[]));
    let yaml = d.join("goalcup/homes/hermes-cli/config.yaml");
    let first = finished("hermes-cli", &e, d);
    assert!(d.join("goalcup/homes/hermes-cli.goalcup.lock").is_file());
    let text = std::fs::read_to_string(&yaml).unwrap();
    assert!(text.contains("updates:\n  check: false\n"), "{text}");
    std::fs::remove_file(&yaml).unwrap();
    let second = finished("hermes-cli", &e, d);
    let home = first.home.clone();
    assert_eq!(second.refusal, Some(Refusal::HomeLocked { home }));
    assert!(!second.lock.is_held() && !yaml.exists());
    drop((first, second));
    assert!(finished("hermes-cli", &e, d).lock.is_held());
    let plan = probe(d, "echo; echo '  fakeharness v2.1.0  '");
    let version = plan.harness_version.as_deref();
    assert_eq!(
        (version, &plan.refusal),
        (Some("fakeharness v2.1.0"), &None)
    );
    drop(plan);
    let plan = probe(d, "echo fakeharness 1.9.0");
    assert_eq!(plan.refusal, None);
    let say = "fakev 1.9.0 is not the version this connector was written for (2.x). goalcup records it the same way; goalcup status says if model requests are missing.";
    assert_eq!(plan.version_notes, vec![say]);
    drop(plan);
    let plan = probe(d, "sleep 30");
    assert_eq!((plan.harness_version, plan.refusal), (None, None));
}

#[test]
fn opencode_ids_share_a_program() {
    let pair = vec![
        ("opencode-cli", "1.".into()),
        ("opencode2-cli", "2.".into()),
    ];
    assert_eq!(connector::version::same_program("opencode"), pair);
    assert!(connector::version::same_program("claude").is_empty());
}

#[test]
fn a_wrong_opencode_pick_names_the_other_connector() {
    let fake_on = |d: &Path, body: &str, resume: bool| {
        let program = d.join("opencode");
        std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        finished(
            "opencode-cli",
            &Env {
                resume,
                ..env(d, &[])
            },
            d,
        )
    };
    let fake = |d: &Path, body: &str| fake_on(d, body, false);
    let dir = tempfile::tempdir().unwrap();
    let refusal = fake(dir.path(), "echo opencode v2.2.2").refusal.unwrap();
    let want = Refusal::VersionMismatch {
        program: "opencode".into(),
        want: "1.".into(),
        got: "2.2.2".into(),
        sibling: "opencode2-cli".into(),
    };
    assert_eq!(refusal, want);
    assert_eq!(
        text::refusal(&refusal),
        "opencode --version shows 2.2.2; this connector is for opencode 1.x. Run: goalcup setup --harness opencode2-cli"
    );
    let dir = tempfile::tempdir().unwrap();
    let plan = fake(dir.path(), "echo boom; exit 1");
    assert_eq!(plan.refusal, None);
    let say = "goalcup could not read opencode --version, so it could not check this is opencode 1.x. If it is 2.x, run: goalcup setup --harness opencode2-cli";
    assert_eq!(plan.version_notes, vec![say]);
    // A run being continued keeps its connector: the 2.x pick is a note, never a refusal.
    let dir = tempfile::tempdir().unwrap();
    let plan = fake_on(dir.path(), "echo opencode v2.2.2", true);
    assert_eq!(plan.refusal, None);
    let say = "opencode 2.2.2 is not the version this connector was written for (1.x). goalcup records it the same way; goalcup status says if model requests are missing.";
    assert_eq!(plan.version_notes, vec![say]);
}

#[test]
fn settings_are_typed_keep_other_keys_and_never_follow_a_link() {
    let dir = tempfile::tempdir().unwrap();
    let (d, home) = (dir.path(), dir.path().join("goalcup/homes/codex-cli"));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), "model = \"m\"\n").unwrap();
    drop(finished("codex-cli", &env(d, &[]), d));
    let toml = std::fs::read_to_string(home.join("config.toml"));
    let toml: toml::Table = toml.unwrap().parse().unwrap();
    let check = &toml["check_for_update_on_startup"];
    assert_eq!((check, toml["model"].as_str()), (&false.into(), Some("m")));
    let rows = connector::load("claude-code-cli", None).unwrap();
    let (rows, json) = (rows.no_prompts, home.join("settings.json"));
    // Comments and a trailing comma, as a JSONC-reading harness accepts them.
    std::fs::write(&json, "{ // c\n\"model\":\"m\",\"permissions\":{},}").unwrap();
    assert!(connector::write_settings(&home, &rows).is_empty());
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
    assert_eq!(v["permissions"]["defaultMode"], "bypassPermissions");
    assert_eq!(v["skipDangerousModePermissionPrompt"], true);
    assert_eq!(v["model"], "m");
    std::fs::remove_file(&json).unwrap();
    std::os::unix::fs::symlink(d.join("target.json"), &json).unwrap();
    std::fs::write(d.join("target.json"), "{}").unwrap();
    assert_eq!(connector::write_settings(&home, &rows).len(), rows.len());
    assert_eq!(std::fs::read(d.join("target.json")).unwrap(), b"{}");
    // YAML: a 1.1 loader reads a bare `off` as a bool and a quoted "false" as a string.
    let hermes = connector::load("hermes-cli", None).unwrap();
    let mut rows = [hermes.no_prompts, hermes.update_off].concat();
    let mut row = rows[0].clone();
    (row.key, row.value) = ("telemetry.shared_metrics.enabled".into(), "true".into());
    rows.push(row.clone());
    (row.key, row.value) = ("telemetry.shared_metrics.send".into(), "false".into());
    rows.push(row);
    let yaml = home.join("config.yaml");
    let was = "approvals:\n  mode: off\n\n# updates\nupdates:\n  check: \"false\"\n\ntelemetry:\n  shared_metrics:\n    enabled: false\n    send: \"true\"\n";
    std::fs::write(&yaml, was).unwrap();
    assert!(connector::write_settings(&home, &rows).is_empty());
    let text = std::fs::read_to_string(&yaml).unwrap();
    let want = "approvals:\n  mode: \"off\"\n\n# updates\nupdates:\n  check: false\n\ntelemetry:\n  shared_metrics:\n    enabled: true\n    send: false\n";
    assert_eq!(text, want);
    assert!(connector::write_settings(&home, &rows).is_empty());
    assert_eq!(std::fs::read_to_string(&yaml).unwrap(), text);
}

#[test]
fn with_home_swapped_a_login_shell_finds_goalcup() {
    let dir = tempfile::tempdir().unwrap();
    let (d, bin) = (dir.path(), dir.path().join("bin"));
    std::fs::create_dir_all(&bin).unwrap();
    let fake = bin.join("goalcup");
    std::fs::write(&fake, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let file = d.join("home-swap.toml");
    let toml = "id = \"home-swap\"\nname = \"H\"\nprogram = \"sh\"\nhome = { HOME = \"{home}\" }\n";
    std::fs::write(&file, toml).unwrap();
    let c = connector::load(file.to_str().unwrap(), None).unwrap();
    let (e, home, profile) = (env(&bin, &[]), d.join("home"), d.join("home/.profile"));
    let found = |plan: LaunchPlan| {
        let mut sh = std::process::Command::new("sh");
        // PATH reset as Debian's /etc/profile does: only `.profile` can put the handed one back.
        sh.env_clear().envs(&plan.env).env("PATH", "/usr/bin:/bin");
        let out = sh
            .args(["-lc", "command -v goalcup"])
            .output()
            .unwrap()
            .stdout;
        String::from_utf8(out).unwrap()
    };
    let want = format!("{}\n", fake.display());
    assert!(found(connector::plan(&c, &e, &home)).ends_with(&want));
    // goalcup's own `.profile` is replaced; anything else is kept.
    let header = std::fs::read_to_string(&profile).unwrap();
    let header = header.lines().next().unwrap().to_owned();
    std::fs::write(&profile, format!("{header}\nexport PATH=/nowhere\n")).unwrap();
    assert!(found(connector::plan(&c, &e, &home)).ends_with(&want));
    std::fs::write(&profile, "# mine\n").unwrap();
    drop(connector::plan(&c, &e, &home));
    assert_eq!(std::fs::read_to_string(&profile).unwrap(), "# mine\n");
}

/// A connector whose `ca_settings` reads `{home}/config.yaml` as Hermes's does, and one plan of it
/// on `home` holding `config` (none when `None`); the plan is dropped, so the home is free again.
fn trusted(d: &Path, config: Option<&str>) -> (LaunchPlan, String) {
    let file = d.join("trust-cli.toml");
    let toml = "id = \"trust-cli\"\nname = \"T\"\nprogram = \"true\"\n\
        host_hints = [{ file = \"{home}/config.yaml\", key = \"model.base_url\" }, \
        { file = \"{home}/config.yaml\", key = \"providers.*.base_url\" }]\n[ca_settings]\n\
        file = \"{home}/config.yaml\"\n\
        entries = [\"custom_providers.*.base_url\", \"providers.*.base_url\"]\n\
        uses = [\"model.base_url\"]\nadd = \"providers.*.base_url\"\nkey = \"ssl_ca_cert\"\n\
        value = \"${env:SSL_CERT_FILE}\"\n\
        fallback = [\"https://api.deepseek.com/v1\", \"https://openrouter.ai/api/v1\"]\n";
    std::fs::write(&file, toml).unwrap();
    let c = connector::load(file.to_str().unwrap(), None).unwrap();
    let home = d.join("home");
    std::fs::create_dir_all(&home).unwrap();
    if let Some(text) = config {
        std::fs::write(home.join("config.yaml"), text).unwrap();
    }
    let plan = connector::plan(&c, &env(d, &[]), &home);
    let text = std::fs::read_to_string(home.join("config.yaml")).unwrap_or_default();
    (plan, text)
}

const CA: &str = "    ssl_ca_cert: \"${env:SSL_CERT_FILE}\"\n";

#[test]
fn trust_rows_fall_back_to_the_routes_once() {
    let dir = tempfile::tempdir().unwrap();
    let (plan, text) = trusted(dir.path(), None);
    for (name, url) in [
        ("api-deepseek-com-v1", "https://api.deepseek.com/v1"),
        ("openrouter-ai-api-v1", "https://openrouter.ai/api/v1"),
    ] {
        let entry = format!("  goalcup-trust-{name}:\n    base_url: \"{url}\"\n{CA}");
        assert!(text.contains(&entry), "{text}");
    }
    assert!(
        plan.log.iter().all(|l| !l.contains("ca_settings")),
        "{:?}",
        plan.log
    );
    drop(plan);
    // The second start writes nothing: the bytes are equal.
    let (_, again) = trusted(dir.path(), Some(&text));
    assert_eq!(again, text);
}

#[test]
fn a_custom_endpoint_entry_gets_the_bundle_and_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    // As Hermes's setup saves a custom endpoint: a list item at its key's indent.
    let config = "model:\n  provider: custom\n  base_url: https://llm.example.net/v1\n\
        custom_providers:\n- name: mine\n  base_url: https://llm.example.net/v1\n  model: m\n";
    let (plan, text) = trusted(dir.path(), Some(config));
    let want = "custom_providers:\n- name: mine\n  base_url: https://llm.example.net/v1\n  model: m\n  \
        ssl_ca_cert: \"${env:SSL_CERT_FILE}\"\n";
    assert!(text.ends_with(want), "{text}");
    assert!(
        !text.contains("goalcup-trust"),
        "no second entry, no fallback: {text}"
    );
    // Not on the shared list, still decrypted and recorded as a config host.
    let decided = plan.hosts.decide("llm.example.net");
    assert!(
        matches!(decided, connector::Decision::Listed { .. }),
        "{decided:?}"
    );
}

#[test]
fn a_custom_base_url_without_an_entry_gets_one() {
    let dir = tempfile::tempdir().unwrap();
    let config = "model:\n  provider: custom\n  base_url: https://llm.example.net/v1\n\
        providers:\n  mine:\n    base_url: https://other.example.org/v1\n    ssl_ca_cert: /mine.pem\n  \
        blank:\n    base_url: https://other.example.org/v2\n    ssl_ca_cert: \"\"\n";
    let (_, text) = trusted(dir.path(), Some(config));
    let entry = format!(
        "  goalcup-trust-llm-example-net-v1:\n    base_url: \"https://llm.example.net/v1\"\n{CA}"
    );
    assert!(text.contains(&entry), "{text}");
    // A participant entry with its own CA keeps it.
    assert!(
        text.contains(
            "  mine:\n    base_url: https://other.example.org/v1\n    ssl_ca_cert: /mine.pem\n"
        ),
        "{text}"
    );
    // An empty CA is not the participant's own: it gets the bundle.
    assert!(
        text.contains(&format!(
            "  blank:\n    base_url: https://other.example.org/v2\n{CA}"
        )),
        "{text}"
    );
    assert!(!text.contains("deepseek"), "no fallback: {text}");
}

#[test]
fn trust_rows_skip_a_flow_map_and_log() {
    let dir = tempfile::tempdir().unwrap();
    let (plan, text) = trusted(dir.path(), Some("providers: {}\n"));
    let skipped = |l: &String| l.starts_with("ca_settings not written: ");
    assert!(plan.log.iter().any(skipped), "{:?}", plan.log);
    assert_eq!(text, "providers: {}\n");
    assert!(plan.refusal.is_none());
}

#[test]
fn a_host_left_undecrypted_keeps_the_os_trust() {
    let dir = tempfile::tempdir().unwrap();
    // A custom_providers entry no host hint names: goalcup does not decrypt it, so writes nothing.
    let config = "custom_providers:\n- name: lan\n  base_url: https://lan.example.net/v1\n";
    let (_, text) = trusted(dir.path(), Some(config));
    assert!(
        !text.contains("lan.example.net/v1\n  ssl_ca_cert"),
        "{text}"
    );
}
