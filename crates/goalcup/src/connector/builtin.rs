//! The connectors built into the binary. This is the one source file that names connector files
//! (PRD-14 `rule_code.rs` allows it). A new connector is one TOML file in `connectors/` plus one
//! line here; `tests/connector_builtin.rs` fails when a file is missing from this list or its `id`
//! differs from the listed one.

/// Every built-in connector file as `(id, text)`; the id is the file's own `id` key.
pub const CONNECTORS: &[(&str, &str)] = &[
    (
        "claude-code-cli",
        include_str!("../../../../connectors/claude-code-cli.toml"),
    ),
    (
        "codex-cli",
        include_str!("../../../../connectors/codex-cli.toml"),
    ),
    (
        "hermes-cli",
        include_str!("../../../../connectors/hermes-cli.toml"),
    ),
    (
        "kilo-cli",
        include_str!("../../../../connectors/kilo-cli.toml"),
    ),
];

/// The shared host list (PRD-11 section 4).
pub const HOSTS: &str = include_str!("../../../../connectors/model-hosts.toml");
