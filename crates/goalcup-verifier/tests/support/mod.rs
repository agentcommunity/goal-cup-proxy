//! Shared helpers for the integration tests. Rust makes each file under `tests/` its own
//! crate, so a helper has to live in a `mod` directory that the tests declare.
//!
//! `dead_code` is allowed throughout, and the reason is that same rule: every test crate
//! that says `mod support;` compiles the whole module, so `tests/adversarial.rs` compiles
//! the compliant generator it never calls and `tests/compliant.rs` compiles the adversarial
//! one. Without the allow, `-D warnings` turns the `never used` warnings for that code into a
//! failure, though the code is used by the other test.
#![allow(dead_code)]

pub mod adversarial;
pub mod capture_records;
pub mod compliant;
pub mod decode;
pub mod pipeline;
pub mod records;
pub mod wire;

/// The compliant trace, regenerated into a tempdir with a truncation planted and
/// `expected_reread` declared in the descriptor.
///
/// `corroborated` decides whether the chain carries a `DaemonRestart` immediately below the
/// `TailNotice`. With it false the chain carries no witness at all — which is the whole
/// test: `expected_reread` is a switch the participant writes, and writing it must not turn
/// the on-disk signature of a rewritten transcript into a Flag.
///
/// The `TempDir` is returned rather than a path, so the directory outlives the caller's
/// `verify_trace` and is removed when the test's binding drops.
pub fn compliant_with_truncation_and_expected_reread(corroborated: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temporary directory for the variant trace");
    compliant::with_truncation_and_expected_reread(dir.path(), corroborated);
    dir
}

/// The compliant trace, regenerated into a tempdir with the launch's `[home_seed]` record
/// chained. The `TempDir` is returned for the reason the helper above
/// returns one: it must outlive the caller's `verify_trace`.
pub fn compliant_with_home_seed() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temporary directory for the variant trace");
    compliant::with_home_seed(dir.path());
    dir
}

/// A trusted repository fixture copied into an owning, contained temporary trace root.
///
/// This is deliberately test-only: production descriptors never receive an outside-root
/// exception.  The source descriptor is historical test input and each declared evidence
/// path is copied byte-for-byte before only its temporary descriptor path value changes.
pub fn materialize_trace(source: &std::path::Path) -> tempfile::TempDir {
    use std::collections::BTreeMap;

    let dst = tempfile::tempdir().expect("contained fixture tempdir");
    let descriptor = source.join("trace.toml");
    let text = std::fs::read_to_string(&descriptor)
        .unwrap_or_else(|e| panic!("{}: {e}", descriptor.display()));
    let mut collisions = BTreeMap::<String, usize>::new();
    let mut rewritten = String::new();
    for line in text.split_inclusive('\n') {
        let (body, newline) = line
            .strip_suffix('\n')
            .map_or((line, ""), |body| (body, "\n"));
        let rewritten_line = descriptor_path_value(body).map_or_else(
            || body.to_owned(),
            |(value_start, value_end, relative)| {
                let source_file = source.join(relative);
                let basename = source_file
                    .file_name()
                    .unwrap_or_else(|| panic!("{} has no basename", source_file.display()));
                let base = basename.to_string_lossy().into_owned();
                let n = collisions.entry(base.clone()).or_insert(0);
                let target_relative = if *n == 0 {
                    base
                } else {
                    format!("evidence/{n}/{}", basename.to_string_lossy())
                };
                *n += 1;
                let target = dst.path().join(&target_relative);
                std::fs::create_dir_all(target.parent().expect("evidence has parent")).unwrap();
                let bytes = std::fs::read(&source_file)
                    .unwrap_or_else(|e| panic!("{}: {e}", source_file.display()));
                std::fs::write(&target, &bytes).unwrap();
                assert_eq!(
                    std::fs::read(&target).unwrap(),
                    bytes,
                    "{} copied exactly",
                    source_file.display()
                );
                format!(
                    "{}{}{}",
                    &body[..value_start],
                    target_relative,
                    &body[value_end..]
                )
            },
        );
        rewritten.push_str(&rewritten_line);
        rewritten.push_str(newline);
    }
    std::fs::write(dst.path().join("trace.toml"), rewritten).unwrap();
    dst
}

/// Return the byte range inside a quoted descriptor path while retaining every byte around
/// it. This is intentionally lexical: test copies must preserve comments, ordering,
/// spacing, and non-path descriptor values exactly.
fn descriptor_path_value(line: &str) -> Option<(usize, usize, &str)> {
    let (key, after_key) = line.split_once('=')?;
    if !matches!(
        key.trim(),
        "connector_manifest" | "path" | "chain" | "receipt" | "baseline" | "acks" | "seal"
    ) {
        return None;
    }
    let quote_start = line.len() - after_key.len() + after_key.find('"')?;
    let rest = &line[quote_start + 1..];
    let quote_end = quote_start + 1 + rest.find('"')?;
    Some((
        quote_start + 1,
        quote_end,
        &line[quote_start + 1..quote_end],
    ))
}

/// Materialize one committed M0 or generated fixture directory by id.
pub fn materialize_fixture(relative: &str) -> tempfile::TempDir {
    materialize_trace(&std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative))
}

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
