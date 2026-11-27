//! Every connector file is built in and loads; no native row snapshots a key file (point 4);
//! the loader refuses what PRD-11 section 2 refuses.

use std::path::{Path, PathBuf};

use goalcup::connector::{self, ConnectorError, HostList, Source, builtin, hosts};
use goalcup::native::pattern;

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
        "the connectors/*.toml glob and the file list disagree"
    );
    assert_eq!(
        builtin::CONNECTORS.iter().map(|c| c.0).collect::<Vec<_>>(),
        ids
    );
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
fn a_program_name_picks_its_connector() {
    let id = |arg: &str| connector::load(arg, None).unwrap().id;
    assert_eq!(id("claude"), "claude-code-cli");
    assert_eq!(id("codex"), "codex-cli");
    assert_eq!(id("claude-code-cli"), "claude-code-cli");
    // OpenCode 1 and 2 are one connector; the old 2.x id still loads it.
    assert_eq!(id("opencode"), "opencode-cli");
    assert_eq!(id("opencode2-cli"), "opencode-cli");
    // An old id without its `-cli`, as `--harness opencode2`.
    assert_eq!(id("opencode2"), "opencode-cli");
    assert_eq!(id("homemade"), "homemade-cli");
    assert_eq!(id("homemade-agent"), "homemade-cli");
}

#[test]
fn an_unknown_id_lists_every_valid_id() {
    let e = connector::load("nonesuch", None).unwrap_err().to_string();
    for (id, _) in builtin::CONNECTORS {
        assert!(e.contains(id), "{e}");
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

fn resolve(template: &str) -> String {
    template
        .replace("{home}", "/h")
        .replace("{workspace}", "/w")
}

/// True when some native row of `c` would read `file`: its resolved root plus `/` prefixes the
/// resolved file and its pattern matches the rest.
fn takes(c: &connector::Connector, file: &str) -> bool {
    let file = resolve(file);
    c.native.iter().any(|row| {
        file.strip_prefix(&format!("{}/", resolve(&row.root)))
            .is_some_and(|rest| {
                pattern::parse(&row.pattern)
                    .unwrap()
                    .matches(Path::new(rest))
            })
    })
}

#[test]
fn no_native_row_reads_a_key_file() {
    for id in file_ids() {
        let c = connector::load(&id, None).unwrap();
        for secret in &c.secret_files {
            assert!(
                !takes(&c, &secret.replace('*', "x")),
                "{id}: a native row would read key file {secret}"
            );
        }
    }
}

/// Databases that no reader uses and masking cannot see are not copied: Copilot's search index
/// and Codex's logs database. Codex keeps its state database.
#[test]
fn no_native_row_takes_an_index_or_a_log_database() {
    let load = |id| connector::load(id, None).unwrap();
    assert!(!takes(&load("copilot-cli"), "{home}/session-store.db"));
    assert!(takes(
        &load("copilot-cli"),
        "{home}/session-state/s/events.jsonl"
    ));
    assert!(!takes(&load("codex-cli"), "{home}/logs_2.sqlite-wal"));
    assert!(takes(&load("codex-cli"), "{home}/state_5.sqlite"));
}

/// A long paste into Codex's `/goal` goes to a file in its home, and the goal is a pointer to it:
/// both goal files are recorded, the images Codex copies beside them are not.
#[test]
fn codex_goal_files_are_recorded() {
    let codex = connector::load("codex-cli", None).unwrap();
    assert!(takes(&codex, "{home}/attachments/u1/pasted-text-1.txt"));
    assert!(takes(&codex, "{home}/attachments/u1/goal-objective.md"));
    assert!(!takes(&codex, "{home}/attachments/u1/image-1.png"));
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
fn a_usage_table_loads() {
    load_text("id = \"test-cli\"\nname = \"Test\"\nprogram = \"test\"\n[usage]\nthread = [\"x-session-id\"]\njoin = \"thread\"\n").unwrap();
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

/// Connector data never reaches outside the isolated home or the workspace (AGENTS.md: nothing
/// is written into the participant's real harness folders), and never takes a variable the code
/// sets (a CA or proxy variable pointed elsewhere would break every tunnel, point 3). Settings
/// are written into the isolated home only; a native pattern passes PRD-04's check.
#[test]
fn connector_data_stays_in_its_home_and_off_the_code_variables() {
    let refused = [
        "home = { X = \"{home}/../../.claude\" }",
        "home = { X = \"{home}/a/./b\" }",
        "native = [{ name = \"a\", root = \"{home}/../../.claude/projects\", pattern = \"*.jsonl\" }]",
        "update_off = [{ file = \"{home}/../../.codex/config.toml\", key = \"a\", value = \"b\" }]",
        "secret_files = [\"{workspace}/../x.json\"]",
        "no_prompts = [{ file = \"{workspace}/x.json\", key = \"a\", value = \"b\" }]",
        "update_off = [{ file = \"{workspace}/x.json\", key = \"a\", value = \"b\" }]",
        "native = [{ name = \"a\", root = \"{home}\", pattern = \"logs/**\" }]",
        "skills_dir = \"{workspace}/skills\"",
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
        let got = load_text(&format!("{head}{body}\n"));
        assert!(matches!(got, Err(ConnectorError::Invalid { .. })), "{body}");
    }
    load_text(&format!(
        "{head}home = {{ HOME = \"{{home}}\", XDG_CONFIG_HOME = \"{{home}}/.config\" }}\n"
    ))
    .unwrap();
}
