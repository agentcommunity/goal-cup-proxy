//! The harness environment (PRD-11 section 3): the participant's own variables pass through,
//! their proxy and CA variables are replaced, the code never sets the variables that turn TLS
//! checks off, and no secret file is read in the default build.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

use goalcup::connector::{self, Env, LaunchPlan};

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
    vars.insert("PATH".into(), "/usr/bin:/bin".into());
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
