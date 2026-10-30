//! The session-file reader (PRD-04): one low-priority thread that walks each native row's root,
//! reads the harness's session files raw and copies its databases, with no symlink followed.
//!
//! `start` and `NativeReader::finish` come once PRD-11's seam S3 (`connector::NativeSources`)
//! lands. For now this holds the leaf modules that need no seam.

pub mod fsx;
pub mod pattern;
