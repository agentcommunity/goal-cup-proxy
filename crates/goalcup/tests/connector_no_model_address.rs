//! goalcup never sets a model address (hard rule): the loader refuses a connector whose `env`
//! names one, or whose `env`, `no_prompts` or `update_off` values hold a URL (PRD-11 section 2).
//! The one exception is `ca_settings` (AGENTS.md): its fallback trust addresses name shared-list
//! hosts over https, and its CA value is never an address.
//! That all shipped connectors load is `connector_builtin.rs`.

use goalcup::connector::{self, ConnectorError};

const HEAD: &str = "id = \"test-cli\"\nname = \"Test\"\nprogram = \"test\"\n";

fn load_text(body: &str) -> Result<connector::Connector, ConnectorError> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test-cli.toml");
    std::fs::write(&path, format!("{HEAD}{body}")).unwrap();
    connector::load(path.to_str().unwrap(), None)
}

#[test]
fn every_refused_shape_fails_load() {
    let refused = [
        "env = { OPENAI_BASE_URL = \"x\" }",
        "env = { my_base_url = \"x\" }",
        "env = { OPENAI_API_BASE = \"x\" }",
        "env = { AZURE_ENDPOINT = \"x\" }",
        "env = { OLLAMA_HOST = \"x\" }",
        "env = { MY_PROXY = \"x\" }",
        "env = { HARNESS_SWITCH = \"https://example.com\" }",
        "no_prompts = [{ file = \"{home}/c.json\", key = \"a\", value = \"http://127.0.0.1:9\" }]",
        "update_off = [{ file = \"{home}/c.json\", key = \"a\", value = \"wss://example.com\" }]",
        &trust("value = \"https://api.deepseek.com/v1\"\nfallback = []"),
        &trust("value = \"x\"\nfallback = [\"http://127.0.0.1:9\"]"),
        &trust("value = \"x\"\nfallback = [\"http://api.deepseek.com/v1\"]"),
        &trust("value = \"x\"\nfallback = [\"https://evil.example.com/v1\"]"),
    ];
    for body in refused {
        let err = load_text(body).err();
        assert!(
            matches!(err, Some(ConnectorError::ModelAddress { .. })),
            "{body}: {err:?}"
        );
    }
}

#[test]
fn a_plain_switch_loads() {
    let body = "env = { HARNESS_DISABLE_AUTOUPDATE = \"1\" }\nupdate_off = [{ file = \"{home}/c.json\", key = \"a\", value = \"false\" }]";
    load_text(body).unwrap();
}

/// A `ca_settings` table with `rest` (its `value` and `fallback` lines).
fn trust(rest: &str) -> String {
    format!(
        "[ca_settings]\nfile = \"{{home}}/c.yaml\"\nentries = [\"p.*.base_url\"]\nuses = []\n\
         add = \"p.*.base_url\"\nkey = \"ssl_ca_cert\"\n{rest}"
    )
}

#[test]
fn a_trust_table_for_listed_hosts_loads() {
    let c = load_text(&trust(
        "value = \"${env:SSL_CERT_FILE}\"\nfallback = [\"https://api.deepseek.com/v1\"]",
    ))
    .unwrap();
    assert_eq!(
        c.ca_settings.unwrap().fallback,
        ["https://api.deepseek.com/v1"]
    );
}
