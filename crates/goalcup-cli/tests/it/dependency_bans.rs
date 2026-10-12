//! D3 at the dependency level.

use crate::common::support;

/// D3 runs both ways, and this is the command that checks it.
///
/// That `crates/goalcup-cli/Cargo.toml` names no `goalcup-daemon` is not asserted here: the
/// crate is a **dev**-dependency so `tests/seal_scan.rs` can run both walkers over one tree,
/// and `the_shipped_binary_links_no_daemon` in that file asserts both halves of what is left
/// of the rule (no `goalcup_daemon` in `src/` code, and the declaration below
/// `[dev-dependencies]`). Repeating it here weaker would be the drift, not the guard.
#[test]
fn the_structural_bans_hold_at_the_dependency_level() {
    assert!(support::cargo_tree("goalcup-cli").contains("goalcup-verifier"));
    assert!(!support::cargo_tree("goalcup-daemon").contains("goalcup-verifier"));
}
