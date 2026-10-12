//! The slow tier of `goalcup-verifier` (`gate.sh full`, before a release): the harness
//! contract goldens.

#[path = "../common/mod.rs"]
mod common;

#[cfg(unix)]
mod harness_contract_golden;
