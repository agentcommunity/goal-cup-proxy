//! Full tier: the whole devserver crate. It is a development-only signer, run before a
//! release by `scripts/gate.sh full`, not on every change.

#[path = "../common/mod.rs"]
mod common;

mod dev_cli;
mod dev_http;
mod placeholder_seed;
