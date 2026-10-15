//! Shared helpers for the integration tests, declared by the `it` binary as `mod common;` and
//! reached from a test module as `support` or `fixture_support`.
//!
//! `dead_code` is allowed throughout, so a helper only some modules call does not fail
//! `-D warnings`.
#![allow(dead_code)]

pub mod capture_records;
pub mod decode;
pub mod records;
pub mod wire;

/// Copy a committed directory tree byte for byte into `to`. A committed trace holds only files
/// and directories; anything else is a fixture error.
pub fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    for entry in std::fs::read_dir(from).unwrap_or_else(|e| panic!("{}: {e}", from.display())) {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            std::fs::create_dir(&target).unwrap();
            copy_tree(&entry.path(), &target);
        } else if kind.is_file() {
            std::fs::write(&target, std::fs::read(entry.path()).unwrap()).unwrap();
        } else {
            panic!(
                "{}: a committed trace holds only files and directories",
                entry.path().display()
            );
        }
    }
}

/// Owner-only modes (files 0600, directories 0700), as the daemon's export leaves them and the
/// readers require; Git does not preserve private modes. Deepest entries first, so a directory
/// never loses its search bit before its children are changed.
#[cfg(unix)]
pub fn make_private(dir: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            make_private(&path);
        } else {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
}

/// A committed trace directory copied byte for byte into a private temporary root, ready for
/// a reader.
#[cfg(unix)]
pub fn materialize_private(source: &std::path::Path) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("private trace tempdir");
    copy_tree(source, dir.path());
    make_private(dir.path());
    dir
}

/// Split a closed single-file chain into part files before the given line indices: the names
/// and bytes a rotating writer leaves, because rotation never changes a byte of the chain.
/// The first part keeps the name `chain.jsonl`; the others are named by their logical offset.
pub fn split_into_parts(dir: &std::path::Path, cuts: &[usize]) -> Vec<std::path::PathBuf> {
    use goalcup_protocol::export::{CHAIN_FILE, part_file_name};
    let bytes = std::fs::read(dir.join(CHAIN_FILE)).unwrap();
    let lines: Vec<&[u8]> = bytes.split_inclusive(|byte| *byte == b'\n').collect();
    let mut bounds = vec![0];
    bounds.extend_from_slice(cuts);
    bounds.push(lines.len());
    let mut start = 0u64;
    let mut paths = Vec::new();
    for pair in bounds.windows(2) {
        let part = lines[pair[0]..pair[1]].concat();
        let path = dir.join(part_file_name(start));
        std::fs::write(&path, &part).unwrap();
        start += part.len() as u64;
        paths.push(path);
    }
    paths
}
