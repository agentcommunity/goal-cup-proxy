//! Adding a harness needs no core change: a fresh connector folder made from the contributor
//! template (`skills/adding-a-capture-harness/templates/connector-harness.toml`), with its
//! placeholders filled as a contributor fills them, loads as it is, with no Rust change. A core
//! change that the template no longer meets fails here, so the template and the schema move
//! together. The other half of "no core change", no harness name in core code, is
//! `no_family_literals.rs`.

use goalcup_protocol::harness::{Origin, load_dir};
use std::path::Path;

/// What a contributor writes in place of each placeholder on a line the template leaves active.
/// A placeholder missing here fails the test by name, so a new one is added with its value.
const FILLED: &[(&str, &str)] = &[
    ("<HARNESS_ID>", "template-check-cli"),
    ("<HARNESS NAME>", "Template Check"),
    ("<harness-name>", "template-check"),
    ("<RESUME-FLAG>", "--continue"),
    ("<config.yaml>", "config.yaml"),
    ("<model.base_url>", "model.base_url"),
    ("<PROGRAM>", "template-check"),
    ("<HARNESS>_HOME", "TEMPLATECHECK_HOME"),
    ("{user_home}/.<harness>", "{user_home}/.templatecheck"),
    ("<CA_VARIABLE>", "TEMPLATECHECK_CA_BUNDLE"),
    ("<extend|replace>", "replace"),
    ("<session>", "session"),
    ("{config_home}/<sessions>", "{config_home}/sessions"),
    ("<*/*.jsonl>", "*/*.jsonl"),
    ("<append_only|snapshot|final_snapshot>", "append_only"),
    ("<config.toml>", "config.toml"),
    ("<mcp_servers>", "mcp_servers"),
    ("<settings.json>", "settings.json"),
    ("<permissions.mode>", "permissions.mode"),
    ("<never>", "never"),
];

/// The template with its commented `[participant.*]` option blocks switched on: each block's
/// header and the `key = value` lines under it lose their `# `.
fn with_participant_options(text: &str) -> String {
    let mut out = String::new();
    let mut in_block = false;
    for line in text.lines() {
        let bare = line.strip_prefix("# ");
        let header =
            bare.is_some_and(|b| b.starts_with("[participant.") || b.starts_with("[[participant."));
        let setting = bare.is_some_and(|b| {
            b.split_once(" = ").is_some_and(|(k, _)| {
                !k.is_empty() && k.bytes().all(|c| c.is_ascii_lowercase() || c == b'_')
            })
        });
        in_block = header || (in_block && setting);
        out.push_str(if in_block { bare.unwrap() } else { line });
        out.push('\n');
    }
    out
}

/// The template with every placeholder on an active (non-comment) line filled from [`FILLED`].
/// Comment lines are kept as they are: they are pinned bytes too, and hold no field.
fn filled_template(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let mut line = line.to_string();
        let active_end = line.find('#').unwrap_or(line.len());
        if active_end > 0 {
            let (active, comment) = line.split_at(active_end);
            let mut active = active.to_string();
            for (placeholder, value) in FILLED {
                active = active.replace(placeholder, value);
            }
            assert!(
                !(active.contains('<') && active.contains('>')),
                "the template has a placeholder this test does not fill: {active:?}"
            );
            line = format!("{active}{comment}");
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// The template as it is, and with its commented participant options switched on, each filled
/// as a contributor fills it, loads as `goalcup start --dev` loads an unpinned folder, and under
/// the strict rules a pinned connector is held to once its PR lands and its pin line is added.
#[test]
fn a_connector_made_from_the_template_loads_with_no_core_change() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let template = root.join("skills/adding-a-capture-harness/templates/connector-harness.toml");
    let text = std::fs::read_to_string(&template)
        .unwrap_or_else(|e| panic!("{}: {e}", template.display()));
    let with_options = with_participant_options(&text);
    assert_ne!(
        with_options, text,
        "the template has commented participant options"
    );
    for (what, text) in [("as it is", text.as_str()), ("with options", &with_options)] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("template-check-cli");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("harness.toml"), filled_template(text)).unwrap();
        for origin in [Origin::Dev, Origin::Pinned] {
            let loaded = load_dir(&dir, origin).unwrap_or_else(|e| {
                panic!("the filled template ({what}) does not load as {origin:?}: {e}")
            });
            assert_eq!(loaded.manifest.id, "template-check-cli");
            assert_eq!(loaded.manifest.schema_version, 1);
            let participant = loaded.manifest.participant.as_ref().expect("[participant]");
            assert_eq!(
                participant.time_left_tool.is_some(),
                what == "with options",
                "{what}"
            );
            assert_eq!(
                participant.no_prompts.is_empty(),
                what == "as it is",
                "{what}"
            );
        }
    }
}
