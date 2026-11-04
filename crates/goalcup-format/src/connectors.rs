//! Every built-in connector file as `(id, text)`, sorted by id: each `connectors/*.toml` except
//! `model-hosts.toml` and `tiers.toml`, embedded by `build.rs`. The id is the file stem.

include!(concat!(env!("OUT_DIR"), "/connectors.rs"));
