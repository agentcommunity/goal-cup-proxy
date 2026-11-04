//! Every built-in connector file as `(id, text)`, sorted by id: each `connectors/*.toml` except
//! `model-hosts.toml` and `tiers.toml`, embedded by `build.rs`. The id is the file stem.

include!(concat!(env!("OUT_DIR"), "/connectors.rs"));

/// Built-in ids that were folded into another: `(old, new)`. A record or run that names the old id
/// reads as the new one.
pub const RENAMED: &[(&str, &str)] = &[("opencode2-cli", "opencode-cli")];

/// `id`, or the id it was renamed to.
pub fn current_id(id: &str) -> &str {
    RENAMED
        .iter()
        .find(|(old, _)| *old == id)
        .map_or(id, |(_, new)| new)
}
