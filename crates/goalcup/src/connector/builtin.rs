//! The connectors built into the binary. This is the one source file that names connector files
//! (PRD-14 `rule_code.rs` allows it). A new connector is one TOML file in `connectors/` plus one
//! line here; `tests/connector_builtin.rs` fails when a file is missing from this list.

/// Every built-in connector file. Each one's id is its own `id` key.
pub const CONNECTORS: &[&str] = &[
    include_str!("../../../../connectors/claude-code-cli.toml"),
    include_str!("../../../../connectors/codex-cli.toml"),
    include_str!("../../../../connectors/hermes-cli.toml"),
    include_str!("../../../../connectors/kilo-cli.toml"),
];

/// The shared host list (PRD-11 section 4).
pub const HOSTS: &str = include_str!("../../../../connectors/model-hosts.toml");
