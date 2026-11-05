//! Built-in connectors: every file in `connectors/`, globbed by `goalcup-format/build.rs`, so
//! nothing names connectors by hand; `tests/connector_builtin.rs` checks the glob.
pub use goalcup_format::connectors::CONNECTORS;
/// The shared host list (PRD-11 section 4).
pub const HOSTS: &str = include_str!("../../../../connectors/model-hosts.toml");
