//! Built-in connectors, one line per `connectors/<id>.toml`: the one source file that names them
//! (PRD-14 `rule_code.rs`); `tests/connector_builtin.rs` fails when a file is missing or misnamed.
#[rustfmt::skip] // one line per connector; rustfmt would spread the arm over six
macro_rules! entry {
    ($id:literal) => { ($id, include_str!(concat!("../../../../connectors/", $id, ".toml"))) };
}
/// Every built-in connector file as `(id, text)`; the id is the file's own `id` key.
pub const CONNECTORS: &[(&str, &str)] = &[
    entry!("amp-cli"),
    entry!("claude-code-cli"),
    entry!("cline-cli"),
    entry!("codex-cli"),
    entry!("copilot-cli"),
    entry!("crush-cli"),
    entry!("droid-cli"),
    entry!("gemini-cli"),
    entry!("goose-cli"),
    entry!("hermes-cli"),
    entry!("kilo-cli"),
    entry!("kimi-code-cli"),
    entry!("openclaw-cli"),
    entry!("opencode-cli"),
    entry!("opencode2-cli"),
    entry!("qwen-cli"),
];
/// The shared host list (PRD-11 section 4).
pub const HOSTS: &str = include_str!("../../../../connectors/model-hosts.toml");
