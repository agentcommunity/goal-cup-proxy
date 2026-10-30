//! Host hints and refusals (PRD-11 sections 2 and 3, PRD-17 N7 and N8).

use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

use goalcup::connector::{self, Decision, Env, LaunchPlan, Refusal, Source};
use tempfile::TempDir;

const CLAUDE: &str = "claude-code-cli";
const JSON: &str = "{\"env\":{\"ANTHROPIC_BASE_URL\":\"https://gw.example.com/v1\"}}";

/// The plan for built-in connector `id` under `dir`.
fn plan_in(dir: &Path, id: &str, vars: &[(&str, &str)]) -> LaunchPlan {
    let mut all = BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]);
    all.extend(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    let env = Env {
        vars: all,
        workspace: dir.join("workspace"),
        resume: false,
        run_start: SystemTime::now(),
        goalcup_home: dir.join("goalcup"),
        server_host: "ingest.example.org".into(),
    };
    let home = dir.join("goalcup/homes").join(id);
    connector::plan(&connector::load(id, None).unwrap(), &env, &home)
}

/// [`plan_in`] a new folder, with `text` written to `file` in the isolated home first.
fn plan(id: &str, file: &str, text: &str, vars: &[(&str, &str)]) -> (LaunchPlan, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("goalcup/homes").join(id).join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    (plan_in(dir.path(), id, vars), dir)
}

fn config(plan: &LaunchPlan, host: &str) -> bool {
    let decision = plan.hosts.decide(host);
    matches!(decision, Decision::Listed { source, .. } if source == Source::Config)
}

fn no_config(plan: &LaunchPlan) -> bool {
    plan.hosts.lists()["config"].as_array().unwrap().is_empty()
}

#[test]
fn one_hint_per_format_reaches_the_config_list() {
    let c = "{ // a \"comment\"\n \"provider\": { \"gw\": { \"options\": {\n \
                 \"baseURL\": \"https://c.example.com/v1\", /* a, b */ }, }, }, }\n";
    let j = c.replace("c.example", "j.example");
    let t = "[model_providers.gw]\nbase_url = \"https://t.example.com/v1\"\n";
    let y = "model:\n  default: x\nproviders:\n  gw:\n    base_url: https://y.example.com\n";
    // `*` over list indexes, items at the key's indent or deeper; block scalar text is no key.
    let l = "custom_providers:\n- name: a\n  base_url: https://l0.example.com\n- name: b\n  \
             notes: |\n    base_url: https://no.example.com\n  base_url: https://l1.example.com\n";
    let deep = "custom_providers:\n  - name: a\n    base_url: https://l2.example.com\n";
    let d = "# keys\nexport GW_BASE_URL=\"https://d.example.com/v1\"\n";
    let cases = [
        (CLAUDE, "settings.json", JSON, "gw.example.com"),
        ("kilo-cli", ".config/kilo/kilo.jsonc", c, "c.example.com"),
        (
            "kilo-cli",
            ".config/kilo/kilo.json",
            j.as_str(),
            "j.example.com",
        ),
        ("codex-cli", "config.toml", t, "t.example.com"),
        ("hermes-cli", "config.yaml", y, "y.example.com"),
        ("hermes-cli", "config.yaml", l, "l0.example.com"),
        ("hermes-cli", "config.yaml", l, "l1.example.com"),
        ("hermes-cli", "config.yaml", deep, "l2.example.com"),
        ("hermes-cli", ".env", d, "d.example.com"),
        (CLAUDE, "unrelated.txt", "", "ev.example.com"),
    ];
    let vars = [("ANTHROPIC_BASE_URL", "https://ev.example.com")];
    for (id, file, text, host) in cases {
        let plan = plan(id, file, text, &vars).0;
        assert!(config(&plan, host), "{id} {file} {host}");
        assert!(!config(&plan, "no.example.com"), "{id} {file}");
    }
}

#[test]
fn a_link_and_a_file_over_one_mib_are_skipped_and_a_bom_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("goalcup/homes").join(CLAUDE);
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(dir.path().join("real.json"), JSON).unwrap();
    std::os::unix::fs::symlink(dir.path().join("real.json"), home.join("settings.json")).unwrap();
    let linked = plan_in(dir.path(), CLAUDE, &[]);
    assert!(!config(&linked, "gw.example.com"), "link");
    assert_eq!(
        linked.unread,
        [home.join("settings.json")],
        "one start line"
    );

    let big = format!("{{\"pad\":\"{}\",{}", "x".repeat(1 << 20), &JSON[1..]);
    let (big, _d) = plan(CLAUDE, "settings.json", &big, &[]);
    assert!(!config(&big, "gw.example.com"), "over 1 MiB");
    assert_eq!(big.unread.len(), 1, "over 1 MiB");
    let (bom, _d) = plan(CLAUDE, "settings.json", &format!("\u{feff}{JSON}"), &[]);
    assert!(config(&bom, "gw.example.com"), "BOM");
}

#[test]
fn a_loopback_value_gives_one_local_model_note_and_no_host() {
    let json = "{\"env\":{\"ANTHROPIC_BASE_URL\":\"http://127.0.0.1:11434/v1\"}}";
    let vars = [("ANTHROPIC_BASE_URL", "https://user:pw@127.0.0.1:11434")];
    let (plan, _dir) = plan(CLAUDE, "settings.json", json, &vars);
    assert_eq!(plan.notes.len(), 1, "{:?}", plan.notes);
    assert_eq!(plan.notes[0]["what"], "local_model");
    assert_eq!(plan.notes[0]["host"], "127.0.0.1:11434");
    assert!(no_config(&plan));
}

#[test]
fn plain_http_an_ip_and_an_unnamed_variable_add_nothing_and_skips_are_logged() {
    let json = "{\"env\":{\"ANTHROPIC_BASE_URL\":\"http://plain.example.com/v1\"}}";
    let ip = ("ANTHROPIC_VERTEX_BASE_URL", "https://203.0.113.9/v1");
    let (plan, _dir) = plan(
        CLAUDE,
        "settings.json",
        json,
        &[ip, ("DB_HOST", "https://db.x.io")],
    );
    assert!(no_config(&plan) && plan.notes.is_empty());
    let named = |s: &str| plan.log.iter().filter(|l| l.contains(s)).count();
    let lines = [
        named("env.ANTHROPIC_*BASE_URL"),
        named("ANTHROPIC_VERTEX"),
        named("DB_HOST"),
    ];
    assert_eq!(
        lines,
        [1, 1, 0],
        "one line per skipped value: {:?}",
        plan.log
    );
    let shown = format!("{:?}", plan.log);
    assert!(
        !shown.contains("plain.example") && !shown.contains("203.0"),
        "never the value"
    );
}

#[test]
fn a_refuse_if_set_hit_names_file_and_key_never_the_value() {
    let json = "{\"env\":{\"HTTPS_PROXY\":\"http://fixture-proxy-value.example:8080\"}}";
    let (plan, dir) = plan(CLAUDE, "settings.json", json, &[]);
    let home = dir.path().join("goalcup/homes");
    let file = home.join(CLAUDE).join("settings.json");
    let key = "env.HTTPS_PROXY".to_string();
    assert_eq!(plan.refusal, Some(Refusal::SettingSet { file, key }));
    assert!(!format!("{:?}", plan.refusal).contains("fixture-proxy-value"));
}
