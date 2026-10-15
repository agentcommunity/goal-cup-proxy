//! The process-table lock refuses to run outside nextest's `serial` group, so a module added to
//! a lock-taking suite but not to `.config/nextest.toml` fails at once instead of flaking.

use crate::common::table_lock::group_refusal;

#[test]
fn the_table_lock_refuses_a_test_outside_the_serial_group() {
    assert_eq!(group_refusal(None), None, "plain cargo test sets no group");
    assert_eq!(group_refusal(Some("serial")), None);
    for ungrouped in ["@global", "some-other-group"] {
        let refusal = group_refusal(Some(ungrouped)).expect("a test outside `serial` is refused");
        assert!(refusal.contains(ungrouped), "{refusal}");
        assert!(refusal.contains(".config/nextest.toml"), "{refusal}");
    }
}
