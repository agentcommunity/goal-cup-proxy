//! `[[versions]]` rows: the first row whose `match` holds for the read version lays its keys over
//! the base connector. A row cannot touch the fixed keys and is checked like the base.

use goalcup::connector::schema::{self, Connector};
use goalcup::connector::{self, ConnectorError, version};

/// A test connector with `rows` after its base keys.
fn c(rows: &str) -> Result<Connector, ConnectorError> {
    let text = format!(
        "id = \"t-cli\"\nname = \"T\"\nprogram = \"t\"\nargs = [\"base\"]\nenv = {{ A = \"1\" }}\nca_env = [\"T_CA\"]\nfirst_screen = \"Hi.\"\n{rows}"
    );
    schema::parse(text.as_bytes(), "t-cli.toml")
}

fn v(s: &str) -> Option<version::HarnessVersion> {
    version::parse(s)
}

#[test]
fn first_match_wins_and_merges() {
    let k = c("[[versions]]\nmatch = \"1.2.3\"\nargs = [\"exact\"]\n[[versions]]\nmatch = \"1.x\"\nargs = []\nenv = { B = \"2\" }\nfirst_screen = \"\"\n").unwrap();
    let (e, row) = k.for_version(v("t 1.2.3").as_ref());
    assert_eq!(
        (e.args.clone(), row.as_deref()),
        (vec!["exact".to_string()], Some("1.2.3"))
    );
    let (e, row) = k.for_version(v("1.9.0").as_ref());
    assert_eq!(row.as_deref(), Some("1.x"));
    assert!(e.args.is_empty() && e.first_screen.is_none());
    let env: Vec<String> = e.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    assert_eq!(env, ["A=1", "B=2"]);
    assert_eq!(e.ca_env, ["T_CA"]);
    let (e, row) = k.for_version(None);
    assert_eq!(
        (e.args.clone(), row, e.first_screen.as_deref()),
        (vec!["base".to_string()], None, Some("Hi."))
    );
    let (e, row) = k.for_version(v("2.0.0").as_ref());
    assert_eq!((e.args, row), (vec!["base".to_string()], None));
}

#[test]
fn a_row_refuses_the_fixed_keys() {
    for key in [
        "home = { X = \"{home}\" }",
        "ca_env = [\"X\"]",
        "ca_settings = []",
        "mask = []",
        "refuse_if_set = []",
        "usage = {}",
        "id = \"x\"",
        "name = \"x\"",
        "program = \"y\"",
        "checked = \"1.0\"",
        "versions = []",
        "old_ids = []",
        "hint_env = []",
        "skills_dir = \"{home}/s\"",
        "end_note = \"x\"",
    ] {
        let err = c(&format!("[[versions]]\nmatch = \"1.x\"\n{key}\n"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown field"), "{key}: {err}");
    }
}

#[test]
fn a_row_is_checked_like_the_base() {
    assert!(matches!(
        c("[[versions]]\nmatch = \"1.x\"\nenv = { OPENAI_BASE_URL = \"x\" }\n"),
        Err(ConnectorError::ModelAddress { key, .. }) if key == "versions[0].env.OPENAI_BASE_URL"
    ));
    assert!(matches!(
        c("[[versions]]\nmatch = \"1.x\"\nunset = []\nenv = { HOME = \"x\" }\n"),
        Err(ConnectorError::Invalid { key, .. }) if key == "versions[0].env.HOME"
    ));
    assert!(matches!(
        c("[[versions]]\nmatch = \"1.x\"\nsecret_files = [\"/etc/passwd\"]\n"),
        Err(ConnectorError::Invalid { key, .. }) if key == "versions[0].secret_files[0]"
    ));
    // A row may not set a variable the base hands to `ca_env`.
    assert!(matches!(
        c("[[versions]]\nmatch = \"1.x\"\nenv = { T_CA = \"x\" }\n"),
        Err(ConnectorError::Invalid { key, .. }) if key == "versions[0].env.T_CA"
    ));
}

#[test]
fn checked_must_be_a_version() {
    assert!(c("checked = \"soon\"\n").is_err());
    assert!(c("checked = \"v2.2.2\"\n").is_err());
    assert!(c("checked = \"2.2.2\"\n").is_ok());
}

#[test]
fn every_builtin_row_is_reached_by_a_fixture_line() {
    let lines = include_str!("fixtures/version-lines.tsv");
    for (id, _) in connector::builtin::CONNECTORS {
        let k = connector::load(id, None).unwrap();
        for (i, row) in k.versions.iter().enumerate() {
            let reached = lines
                .lines()
                .filter(|l| !l.starts_with('#'))
                .filter_map(|l| {
                    l.split('\t')
                        .collect::<Vec<_>>()
                        .get(..2)
                        .map(|c| (c[0], c[1]))
                })
                .filter(|(at, _)| at == id)
                .filter_map(|(_, raw)| version::parse(raw))
                .any(|v| k.for_version(Some(&v)).1.as_deref() == Some(row.spec.as_str()));
            assert!(
                reached,
                "{id}: versions[{i}] ({}) is first for no fixture line",
                row.spec
            );
        }
    }
}

#[test]
fn checked_is_set_where_a_header_named_a_build() {
    let mut got: Vec<(String, String)> = Vec::new();
    for (id, _) in connector::builtin::CONNECTORS {
        if let Some(v) = connector::load(id, None).unwrap().checked {
            got.push((id.trim_end_matches("-cli").to_string(), v));
        }
    }
    let want = "amp 0.0.1793275344-g17bf52, claude-code 2.1.312, cline 3.0.74, codex 0.165.0, \
                crush 0.99.0, droid 0.251.0, gemini 0.66.0, goose 1.57.0, hermes 0.22.5, \
                kilo 7.8.25, kimi-code 2.4.1, openclaw 2026.10.9, opencode 2.2.2, qwen 0.26.4";
    let got: Vec<String> = got.iter().map(|(id, v)| format!("{id} {v}")).collect();
    assert_eq!(got.join(", "), want);
}

/// Every key a launch reads, for comparing two connectors.
fn keys(c: &Connector) -> String {
    format!(
        "{:?}",
        (
            (&c.args, &c.resume, &c.home, &c.env, &c.ca_env, &c.unset),
            (&c.host_hints, &c.secret_files, &c.native, &c.no_prompts),
            (&c.update_off, &c.first_screen, &c.before, &c.after),
            (&c.mask, &c.usage),
        )
    )
}

/// The one OpenCode connector gives each version the keys its own 1.0 connector had.
#[test]
fn opencode_fold_keeps_each_version_s_keys() {
    let fixture = |f: &str| {
        let path = format!(
            "{}/tests/fixtures/opencode-1.0/{f}",
            env!("CARGO_MANIFEST_DIR")
        );
        schema::parse(&std::fs::read(path).unwrap(), f).unwrap()
    };
    let k = connector::load("opencode-cli", None).unwrap();
    for (line, old, row) in [
        ("1.19.5", "opencode-cli.toml", Some("1.x")),
        ("opencode v2.2.2", "opencode2-cli.toml", None),
        ("opencode v3.0.0", "opencode2-cli.toml", None),
    ] {
        let (e, got) = k.for_version(v(line).as_ref());
        assert_eq!(keys(&e), keys(&fixture(old)), "{line}");
        assert_eq!(got.as_deref(), row, "{line}");
    }
    assert_eq!(
        keys(&k.for_version(None).0),
        keys(&fixture("opencode2-cli.toml"))
    );
    assert_eq!(
        connector::load("opencode2-cli", None).unwrap().id,
        "opencode-cli"
    );
}
