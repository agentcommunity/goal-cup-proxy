//! Every connector file is built in and loads; no native row snapshots a key file (point 4);
//! the loader refuses duplicate row names and a skills folder in the workspace (PRD-11 section 2).

use std::path::PathBuf;

use goalcup::connector::{self, HostList, Source, builtin, hosts};

fn connectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors")
}

/// The connector ids: the file stems of `connectors/*.toml`, leaving out the two list files.
fn file_ids() -> Vec<String> {
    let mut ids: Vec<String> = std::fs::read_dir(connectors_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "toml"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .filter(|stem| stem != "model-hosts" && stem != "tiers")
        .collect();
    ids.sort();
    ids
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(bytes))
}

#[test]
fn every_connector_file_is_built_in_and_loads_by_its_id() {
    let ids = file_ids();
    assert!(!ids.is_empty());
    assert_eq!(
        builtin::CONNECTORS.len(),
        ids.len(),
        "connector/builtin.rs must list every connectors/*.toml file"
    );
    for (listed, text) in builtin::CONNECTORS {
        let table: toml::Table = toml::from_str(text).unwrap();
        assert_eq!(table["id"].as_str(), Some(*listed), "the listed id is the file's id");
    }
    for id in &ids {
        let c = connector::load(id, None).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(&c.id, id, "file stem equals id");
        let bytes = std::fs::read(connectors_dir().join(format!("{id}.toml"))).unwrap();
        assert_eq!(
            c.sha256,
            sha256_hex(&bytes),
            "{id}: built-in bytes are the file's"
        );
        assert_eq!(c.source, connector::ConnectorSource::Builtin);
    }
}

#[test]
fn the_shared_host_list_parses() {
    let bytes = std::fs::read(connectors_dir().join("model-hosts.toml")).unwrap();
    let hosts = HostList::from_file(&bytes, "model-hosts.toml").unwrap();
    assert_eq!(hosts.sha256(), sha256_hex(&bytes));
    let shared = hosts.lists()["shared"].as_array().unwrap().clone();
    assert!(!shared.is_empty());
    for entry in shared {
        let name = entry["host"].as_str().unwrap();
        assert!(hosts::check(name, Source::Shared).is_ok(), "{name}");
    }
}

/// One `/`-free component against a glob of `*` and `?`.
fn glob(pattern: &[u8], name: &[u8]) -> bool {
    match (pattern.first(), name.first()) {
        (None, None) => true,
        (Some(b'*'), _) => {
            glob(&pattern[1..], name) || (!name.is_empty() && glob(pattern, &name[1..]))
        }
        (Some(b'?'), Some(_)) => glob(&pattern[1..], &name[1..]),
        (Some(p), Some(n)) if p == n => glob(&pattern[1..], &name[1..]),
        _ => false,
    }
}

/// A `/`-separated pattern where `**` matches zero or more whole components.
fn matches(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.first() {
        None => path.is_empty(),
        Some(&"**") => (0..=path.len()).any(|skip| matches(&pattern[1..], &path[skip..])),
        Some(p) => {
            !path.is_empty()
                && glob(p.as_bytes(), path[0].as_bytes())
                && matches(&pattern[1..], &path[1..])
        }
    }
}

fn resolve(template: &str) -> String {
    template
        .replace("{home}", "/h")
        .replace("{workspace}", "/w")
}

#[test]
fn no_native_row_reads_a_key_file() {
    for id in file_ids() {
        let c = connector::load(&id, None).unwrap();
        for secret in &c.secret_files {
            let secret = resolve(secret).replace('*', "x");
            for row in &c.native {
                let root = resolve(&row.root);
                let Some(rest) = secret.strip_prefix(&format!("{root}/")) else {
                    continue;
                };
                let path: Vec<&str> = rest.split('/').collect();
                let pattern: Vec<&str> = row.pattern.split('/').collect();
                assert!(
                    !matches(&pattern, &path),
                    "{id}: native row {} would read key file {secret}",
                    row.name
                );
            }
        }
    }
}

fn load_text(text: &str) -> Result<connector::Connector, connector::ConnectorError> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test-cli.toml");
    std::fs::write(&path, text).unwrap();
    connector::load(path.to_str().unwrap(), None)
}

#[test]
fn a_connector_file_loads_from_disk() {
    let c = load_text("id = \"test-cli\"\nname = \"Test\"\nprogram = \"test\"\n").unwrap();
    assert_eq!(c.source, connector::ConnectorSource::File);
}

#[test]
fn duplicate_native_row_names_are_refused() {
    let text = r#"
id = "test-cli"
name = "Test"
program = "test"
native = [
  { name = "a", root = "{home}", pattern = "*.jsonl" },
  { name = "a", root = "{home}/other", pattern = "*.log" },
]
"#;
    assert!(load_text(text).is_err());
}

#[test]
fn a_skills_folder_in_the_workspace_is_refused() {
    let text = r#"
id = "test-cli"
name = "Test"
program = "test"
skills_dir = "{workspace}/skills"
"#;
    assert!(load_text(text).is_err());
}

/// Connector data never reaches outside the isolated home or the workspace (AGENTS.md: nothing
/// is written into the participant's real harness folders), and never takes a variable the code
/// sets (a CA or proxy variable pointed elsewhere would break every tunnel, point 3).
#[test]
fn connector_data_stays_in_its_home_and_off_the_code_variables() {
    let refused = [
        "home = { X = \"{home}/../../.claude\" }",
        "home = { X = \"{home}/a/./b\" }",
        "native = [{ name = \"a\", root = \"{home}/../../.claude/projects\", pattern = \"*.jsonl\" }]",
        "update_off = [{ file = \"{home}/../../.codex/config.toml\", key = \"a\", value = \"b\" }]",
        "secret_files = [\"{workspace}/../x.json\"]",
        "skills_dir = \"{home}/../../skills\"",
        "skills_dir = \"{home}//skills\"",
        "ca_env = [\"SSL_CERT_FILE\"]",
        "ca_env = [\"HTTPS_PROXY\"]",
        "ca_env = [\"HOME\"]",
        "home = { HTTPS_PROXY = \"{home}/x\" }",
        "home = { NODE_EXTRA_CA_CERTS = \"{home}/x\" }",
        "env = { NODE_TLS_REJECT_UNAUTHORIZED = \"0\" }",
        "env = { SSL_CERT_DIR = \"/x\" }",
    ];
    let head = "id = \"test-cli\"\nname = \"Test\"\nprogram = \"test\"\n";
    for body in refused {
        assert!(load_text(&format!("{head}{body}\n")).is_err(), "{body}");
    }
    load_text(&format!("{head}home = {{ HOME = \"{{home}}\", XDG_CONFIG_HOME = \"{{home}}/.config\" }}\n")).unwrap();
}
