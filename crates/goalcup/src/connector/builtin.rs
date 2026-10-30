//! The connectors built into the binary, one line per `connectors/<id>.toml` file. The one source
//! file that names connector files (PRD-14 `rule_code.rs` allows it); `tests/connector_builtin.rs`
//! fails when a file is missing here or its `id` differs from the listed one.
macro_rules! entry {
    ($id:literal) => { ($id, include_str!(concat!("../../../../connectors/", $id, ".toml"))) };
}
/// Every built-in connector file as `(id, text)`; the id is the file's own `id` key.
pub const CONNECTORS: &[(&str, &str)] = &[
    entry!("claude-code-cli"),
    entry!("codex-cli"),
    entry!("copilot-cli"),
    entry!("gemini-cli"),
    entry!("hermes-cli"),
    entry!("kilo-cli"),
    entry!("openclaw-cli"),
    entry!("opencode-cli"),
    entry!("opencode2-cli"),
    entry!("qwen-cli"),
];
/// The shared host list (PRD-11 section 4).
pub const HOSTS: &str = include_str!("../../../../connectors/model-hosts.toml");
