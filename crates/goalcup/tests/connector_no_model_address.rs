//! goalcup never sets a model address (hard rule): the loader refuses a connector whose `env`
//! names one, or whose `env`, `no_prompts` or `update_off` values hold a URL (PRD-11 section 2).

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

#[test]
fn the_shipped_connectors_load() {
    for text in connector::builtin::CONNECTORS {
        let table: toml::Table = toml::from_str(text).unwrap();
        let id = table["id"].as_str().unwrap();
        connector::load(id, None).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}
