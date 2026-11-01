//! The harness environment (PRD-11 section 3): the participant's own variables pass through,
//! their proxy and CA variables are replaced, the code never sets the variables that turn TLS
//! checks off, and no secret file is read in the default build. Also the home lock and settings.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::SystemTime;

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

/// The plan's version and refusal for a connector whose program, on `PATH`, runs `body`.
fn probe(d: &Path, body: &str) -> (Option<String>, Option<Refusal>) {
    const FAKE: &str = "id = \"fake-cli\"\nname = \"F\"\nprogram = \"fakev\"\nversion = \"2.\"";
    let (program, file) = (d.join("fakev"), d.join("fake-cli.toml"));
    std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(&file, FAKE).unwrap();
    let c = connector::load(file.to_str().unwrap(), None).unwrap();
    let plan = connector::plan(&c, &env(d, &[]), &d.join("home"));
    (plan.harness_version, plan.refusal)
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
    let (version, refusal) = probe(d, "echo; echo '  fakeharness v2.1.0  '");
    assert_eq!(version.as_deref(), Some("fakeharness v2.1.0"));
    assert_eq!(refusal, None);
    for line in ["echo fakeharness 1.9.0", "echo fakeharness 1.2.0"] {
        let refusal = probe(d, line).1;
        assert!(matches!(refusal, Some(Refusal::VersionMismatch { .. })));
    }
    assert_eq!(probe(d, "sleep 30"), (None, None));
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
    let rows = [hermes.no_prompts, hermes.update_off].concat();
    let yaml = home.join("config.yaml");
    let was = "approvals:\n  mode: off\n\n# updates\nupdates:\n  check: \"false\"\n";
    std::fs::write(&yaml, was).unwrap();
    assert!(connector::write_settings(&home, &rows).is_empty());
    let text = std::fs::read_to_string(&yaml).unwrap();
    let want = "approvals:\n  mode: \"off\"\n\n# updates\nupdates:\n  check: false\n";
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
