//! The slow tier of `goalcup-verifier` (`gate.sh full`, before a release): the harness
//! contract goldens and the ATIF golden.

#[path = "../common/mod.rs"]
mod common;

mod atif_golden;
#[cfg(unix)]
mod harness_contract_golden;
