//! Support shared by this crate's test binaries: the folded `it` and `full` binaries declare
//! it once with `#[path = "../common/mod.rs"] mod common;`, and the standalone `dir_modes` and
//! `health_status` with `#[path = "common/mod.rs"]`. Fixtures stay in `tests/fixtures/`.
//!
//! Every binary compiles all of this and calls a different part of it, so each declaration
//! carries `#[allow(dead_code)]`; the submodules do not. Nothing here is unused in all four
//! binaries (checked by compiling each without the allow), and a helper that becomes so should
//! be deleted.

#[cfg(target_os = "macos")]
pub mod capture_lifecycle_support;
#[path = "support/redaction.rs"]
pub mod redaction;
pub mod support;
#[path = "support/table_lock.rs"]
pub mod table_lock;
