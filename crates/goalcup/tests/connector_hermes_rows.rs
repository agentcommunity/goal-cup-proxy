//! Hermes native rows (point 4): none matches `state.db*`, whose search index holds text out of
//! context that masking cannot see, and none reaches a request dump; the delegation manifest and
//! task log are the only rows.

use std::fs;
use std::path::Path;

use goalcup::connector;
use goalcup::native::pattern::parse;
use goalcup::native::walk::{Row, Walker};
use goalcup::pipeline::Recorder;

#[test]
fn hermes_rows_take_no_state_database() {
    let c = connector::load("hermes-cli", None).unwrap();
    assert!(c.native.iter().all(|n| !n.pattern.starts_with("state.db")));

    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    fs::create_dir_all(home.join("cache/delegation/live/x")).unwrap();
    fs::write(home.join("cache/delegation/live/x/task-1.log"), b"log").unwrap();
    fs::write(home.join("sessions/request_dump_x.json"), b"{}").unwrap();
    fs::write(home.join("state.db"), b"SQLite format 3\0").unwrap();
    let home_s = home.to_str().unwrap();
    let rows: Vec<Row> = c
        .native
        .iter()
        .map(|n| Row {
            name: n.name.clone(),
            root: Path::new(&n.root.replace("{home}", home_s)).into(),
            pattern: parse(&n.pattern).unwrap(),
        })
        .collect();
    let (rec, _) = Recorder::for_tests(1 << 20);
    let found = Walker::default().pass(&rec, &rows).found;
    let names: Vec<String> = found
        .iter()
        .map(|f| rows[f.row].root.join(&f.rel).to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().any(|p| p.ends_with("task-1.log")), "{names:?}");
    assert!(
        !names
            .iter()
            .any(|p| p.contains("state.db") || p.contains("request_dump")),
        "{names:?}"
    );
}

#[test]
fn hermes_end_note_matches_the_verifier_text() {
    let c = connector::load("hermes-cli", None).unwrap();
    assert_eq!(
        c.end_note.as_deref(),
        Some("Subagents are shown by their task logs, not tied to individual calls.")
    );
}

/// Hermes's built-in routes, the trust fallback (plugins/model-providers/*/__init__.py, checked
/// at v0.22.5 and v0.22.7).
const ROUTES: [&str; 9] = [
    "https://api.deepseek.com/v1",
    "https://ai-gateway.vercel.sh/v1",
    "https://openrouter.ai/api/v1",
    "https://inference-api.nousresearch.com/v1",
    "https://api.x.ai/v1",
    "https://api.z.ai/api/paas/v4",
    "https://api.fireworks.ai/inference/v1",
    "https://dashscope.aliyuncs.com/compatible-mode/v1",
    "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
];

#[test]
fn hermes_trust_gives_every_address_the_run_bundle_and_records_it() {
    let c = connector::load("hermes-cli", None).unwrap();
    let t = c.ca_settings.as_ref().unwrap();
    assert_eq!(t.fallback, ROUTES);
    assert_eq!(
        (t.key.as_str(), t.value.as_str()),
        ("ssl_ca_cert", "${env:SSL_CERT_FILE}")
    );
    assert_eq!(t.file, "{home}/config.yaml");
    // Every address given the bundle is also a host hint, so its calls are decrypted and recorded.
    let hinted = |k: &String| {
        let k = k.replace("goalcup-trust", "*");
        c.host_hints.iter().any(|h| h.file == t.file && h.key == k)
    };
    for key in t.entries.iter().chain(&t.uses).chain([&t.add]) {
        assert!(hinted(key), "{key} is not a host hint");
    }
}

#[test]
fn hermes_trust_on_a_fresh_home_names_every_route_once() {
    let d = tempfile::tempdir().unwrap();
    let c = connector::load("hermes-cli", None).unwrap();
    let home = d.path().join("home");
    let rows = connector::trust_rows(c.ca_settings.as_ref().unwrap(), &home);
    let urls: Vec<&str> = rows
        .iter()
        .filter(|r| r.key.ends_with(".base_url"))
        .map(|r| r.value.as_str())
        .collect();
    assert_eq!(urls, ROUTES);
    for r in rows.iter().filter(|r| r.key.ends_with(".base_url")) {
        assert!(r.key.starts_with("providers.goalcup-trust-"), "{}", r.key);
        let twin = r.key.replace(".base_url", ".ssl_ca_cert");
        assert!(
            rows.iter()
                .any(|t| t.key == twin && t.value == "${env:SSL_CERT_FILE}"),
            "{twin}"
        );
    }
    assert_eq!(rows.len(), 18);
}
