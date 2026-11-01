//! Hermes native rows (point 4): none matches `state.db*`, whose search index holds text out of
//! context that masking cannot see; the delegation manifest and task log are the only rows.

use goalcup::connector;

#[test]
fn hermes_rows_take_no_state_database() {
    let c = connector::load("hermes-cli", None).unwrap();
    let names: Vec<&str> = c.native.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["delegation-manifest", "delegation"]);
    assert!(c.native.iter().all(|n| !n.pattern.starts_with("state.db")));
}
