//! The connector-directory digest: the one definition of what "this directory" means to a pin.
//!
//! It is shared by both manifest schemas, by the pins ([`crate::pins`]) and by the server
//! allowlist (D43 ruling 5), so it lives here and not in the V1 module `manifest.rs`. The walk
//! also returns the bytes it hashed ([`DirListing::captured`]): a caller that parses or writes
//! a file reads it from there, never a second time (a second read could authorize bytes the
//! digest did not cover).

use crate::ProtocolError;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// SHA-256 over a whole connector directory: every byte the daemon reads out of
/// `connectors/<id>/`, and not only its `manifest.toml`.
///
/// # Why the directory and not the file
///
/// A connector is a manifest **and** the hook bodies beside it under `files/`. The daemon
/// renders those bodies into the workspace, records the rendered digests as the config it
/// will guard, and then guards them. A pin taken over `manifest.toml` alone therefore reads
/// as "this is the shipped connector" for a directory whose genuine manifest sits beside a
/// tracker script somebody edited — and the daemon goes on to guard the edited body as the
/// legitimate one. The bytes that decide what is captured have to be inside the digest.
///
/// # The exact byte layout
///
/// The hashed stream is, for every regular file under `dir` (recursively), in ascending
/// order of the **bytes** of its path relative to `dir` with `/` separators:
///
/// ```text
/// <u64 be: path byte count> <relative path bytes> <u64 be: file byte count> <file bytes>
/// ```
///
/// and the output is that digest in lowercase hex. `scripts/pin-manifests.sh` builds the
/// same stream in portable shell, and a test compares the two over both shipped
/// connectors, so the layout is written down in both places and measured in one.
///
/// **Both halves are length-prefixed, and nothing is separated by a sentinel.** A NUL after
/// the path and a NUL after the contents is injective only while no file body contains a
/// NUL — and a connector may ship a binary
/// asset. With NUL separators, a file `a` holding `b\0c` and a pair of files `a` holding `b`
/// and `c` holding nothing can be made to hash one stream. A byte count cannot be forged
/// from inside the bytes it counts, so the encoding is injective over arbitrary bodies and
/// arbitrary names. Big-endian and fixed-width, because the shell writes the same eight
/// bytes with `printf` and neither side may depend on a host's word order.
///
/// # What it refuses
///
/// * A **symlink**, at any depth — [`ProtocolError::UnhashableEntry`]. Following one would
///   pin bytes from outside the directory, so two directories whose own contents differ
///   could produce one digest; skipping one would leave a name inside the directory that
///   the digest says nothing about.
/// * Anything else that is neither a regular file nor a directory — a fifo, a socket, a
///   device node. Checked on the OPEN HANDLE and not on the path: see
///   [`read_regular_file`].
/// * A name that is not UTF-8, because the digest is compared against one a portable shell
///   script also produces.
///
/// An empty directory contributes nothing, deliberately: it holds no byte the daemon reads.
pub fn connector_dir_sha256(dir: &Path) -> Result<String, ProtocolError> {
    Ok(hash_connector_dir_listing(dir, |_| false)?.sha256)
}

/// The directory digest, the exact bytes of selected files that went into it, and the
/// directory's file list: one walk, so what was listed is what was hashed.
pub(crate) struct DirListing {
    pub sha256: String,
    /// The bytes of each file the `capture` predicate selected.
    pub captured: BTreeMap<String, Vec<u8>>,
    /// Every regular file as `(relative path with / separators, byte length)`, in hash order.
    pub files: Vec<(String, u64)>,
}

/// [`connector_dir_sha256`] over `dir`, also returning the bytes of every file `capture` selects
/// (by its relative path) and the file list. The schema-1 loader (`harness::load_dir`) parses
/// the captured `harness.toml` bytes, checks the list and carries the captured seed bytes, the
/// same discipline as `manifest::connector_dir_sha256_with_manifest`: no second read can
/// authorize bytes the digest did not cover.
pub(crate) fn hash_connector_dir_listing(
    dir: &Path,
    capture: impl Fn(&str) -> bool,
) -> Result<DirListing, ProtocolError> {
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    collect_regular_files(dir, "", &mut files)?;
    // By the bytes of the relative path, which is the order `LC_ALL=C sort` gives the
    // shell. Sorting the `String`s would agree — Rust compares `str` by bytes — but the
    // comparison is spelled over `as_bytes()` because the byte order is the contract.
    files.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut hasher = Sha256::new();
    let mut captured = BTreeMap::new();
    let mut listed = Vec::with_capacity(files.len());
    for (relative, path) in &files {
        let bytes = read_regular_file(path)?;
        hasher.update(u64_be(relative.len()));
        hasher.update(relative.as_bytes());
        hasher.update(u64_be(bytes.len()));
        hasher.update(&bytes);
        listed.push((relative.clone(), bytes.len() as u64));
        if capture(relative) {
            captured.insert(relative.clone(), bytes);
        }
    }
    Ok(DirListing {
        sha256: hex::encode(hasher.finalize()),
        captured,
        files: listed,
    })
}

/// The eight bytes that prefix each half of a file's record: the count, big-endian.
///
/// `as u64` and not a fallible conversion: `usize` is at most 64 bits on every target this
/// workspace builds for, so the cast is exact, and a verifier must not panic over the width
/// of a number it just measured.
fn u64_be(len: usize) -> [u8; 8] {
    (len as u64).to_be_bytes()
}

/// One file's bytes, read from a handle whose kind was checked **on that handle**.
///
/// The walk that produced `path` read the directory a moment ago, and between that read and
/// this one the name can be replaced. `std::fs::symlink_metadata` followed by
/// `std::fs::read` decides on one inode and hashes another: the classic time-of-check to
/// time-of-use gap, and here it would let a connector directory be pinned as regular files
/// while a fifo — which blocks — or a device node took a name's place. So the file is
/// opened first, the open handle is `fstat`ed, and the bytes are read from that same
/// handle. Anything the handle says is not a regular file is refused rather than read.
fn read_regular_file(path: &Path) -> Result<Vec<u8>, ProtocolError> {
    let mut file = std::fs::File::open(path).map_err(|source| ProtocolError::Io {
        path: path.display().to_string(),
        source,
    })?;
    // `File::metadata` is `fstat` on the descriptor, not a second lookup of the name.
    let meta = file.metadata().map_err(|source| ProtocolError::Io {
        path: path.display().to_string(),
        source,
    })?;
    if !meta.is_file() {
        return Err(ProtocolError::UnhashableEntry {
            path: path.display().to_string(),
            reason: "the open handle is not a regular file, so whatever the directory listing \
                     named has been replaced since it was read"
                .to_string(),
        });
    }
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut bytes).map_err(|source| ProtocolError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(bytes)
}

/// Every regular file under `dir`, as `(path relative to the root, path on disk)`.
///
/// `prefix` is the caller's relative path, with a trailing `/` once it is non-empty, so the
/// relative paths this produces are joined with forward slashes on every platform rather
/// than with whatever separator `Path::join` would render.
fn collect_regular_files(
    dir: &Path,
    prefix: &str,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), ProtocolError> {
    let entries = std::fs::read_dir(dir).map_err(|source| ProtocolError::Io {
        path: dir.display().to_string(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| ProtocolError::Io {
            path: dir.display().to_string(),
            source,
        })?;
        let path = entry.path();
        let Ok(name) = entry.file_name().into_string() else {
            return Err(ProtocolError::UnhashableEntry {
                path: path.display().to_string(),
                reason: "the name is not UTF-8, and the digest it feeds is compared against \
                         one a shell script also produces"
                    .to_string(),
            });
        };
        // `DirEntry::file_type` and never `Path::metadata`: the latter follows the link and
        // reports the TARGET's kind, which is how a symlink comes to be hashed as the file
        // it points at. This is the listing's own answer, and it is only ever used to
        // decide where to descend — the kind that decides what gets HASHED is taken from
        // the open handle in `read_regular_file`, because this one can go stale.
        let kind = entry.file_type().map_err(|source| ProtocolError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let relative = format!("{prefix}{name}");
        if kind.is_symlink() {
            return Err(ProtocolError::UnhashableEntry {
                path: path.display().to_string(),
                reason: "a connector directory is pinned by its own bytes, and a symlink's \
                         bytes are somewhere else"
                    .to_string(),
            });
        }
        if kind.is_dir() {
            collect_regular_files(&path, &format!("{relative}/"), out)?;
        } else if kind.is_file() {
            out.push((relative, path));
        } else {
            return Err(ProtocolError::UnhashableEntry {
                path: path.display().to_string(),
                reason: "neither a regular file nor a directory, so it carries no bytes \
                         to pin"
                    .to_string(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod connector_dir_digest {
    use super::connector_dir_sha256;
    use sha2::{Digest, Sha256};
    use std::path::Path;

    /// A connector directory in miniature: a manifest, and one template under `files/`.
    fn fixture(root: &Path) {
        std::fs::create_dir_all(root.join("files")).expect("the fixture's files/ directory");
        std::fs::write(root.join("manifest.toml"), b"id = \"demo\"\n").expect("the manifest");
        std::fs::write(root.join("files/hook.sh"), b"#!/bin/sh\nexit 0\n").expect("the hook");
    }

    /// The documented stream, spelled out by hand rather than by walking anything: this is
    /// the assertion that the layout in the doc comment is the layout on the wire.
    ///
    /// Written with `to_be_bytes` on a literal `u64` rather than by calling `super::u64_be`,
    /// so a change to that helper is a test failure rather than a test that follows it.
    fn stream(entries: &[(&str, &[u8])]) -> String {
        let mut bytes = Vec::new();
        for (relative, contents) in entries {
            bytes.extend_from_slice(&(relative.len() as u64).to_be_bytes());
            bytes.extend_from_slice(relative.as_bytes());
            bytes.extend_from_slice(&(contents.len() as u64).to_be_bytes());
            bytes.extend_from_slice(contents);
        }
        hex::encode(Sha256::digest(&bytes))
    }

    #[test]
    fn the_digest_is_the_documented_stream_in_sorted_relative_path_order() {
        let tmp = tempfile::tempdir().expect("a temporary directory");
        fixture(tmp.path());
        // `files/hook.sh` sorts before `manifest.toml` because `f` < `m` — the nested file
        // is not appended after the top-level ones, it takes its place among them.
        let want = stream(&[
            ("files/hook.sh", b"#!/bin/sh\nexit 0\n"),
            ("manifest.toml", b"id = \"demo\"\n"),
        ]);
        assert_eq!(
            connector_dir_sha256(tmp.path()).expect("the fixture hashes"),
            want
        );
    }

    #[test]
    fn the_order_is_the_paths_and_not_the_order_the_files_were_created() {
        let first = tempfile::tempdir().expect("a temporary directory");
        std::fs::create_dir_all(first.path().join("files")).expect("files/");
        std::fs::write(first.path().join("files/hook.sh"), b"#!/bin/sh\nexit 0\n").expect("hook");
        std::fs::write(first.path().join("manifest.toml"), b"id = \"demo\"\n").expect("manifest");

        let second = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(second.path().join("manifest.toml"), b"id = \"demo\"\n").expect("manifest");
        std::fs::create_dir_all(second.path().join("files")).expect("files/");
        std::fs::write(second.path().join("files/hook.sh"), b"#!/bin/sh\nexit 0\n").expect("hook");

        assert_eq!(
            connector_dir_sha256(first.path()).expect("hashes"),
            connector_dir_sha256(second.path()).expect("hashes"),
            "the digest is a function of the paths and the bytes, and of nothing else"
        );
    }

    #[test]
    fn one_extra_byte_in_a_template_changes_the_digest() {
        let tmp = tempfile::tempdir().expect("a temporary directory");
        fixture(tmp.path());
        let before = connector_dir_sha256(tmp.path()).expect("hashes");
        // The manifest is untouched, and this is the byte a
        // manifest-only pin could not see.
        std::fs::write(tmp.path().join("files/hook.sh"), b"#!/bin/sh\nexit 0\n\n").expect("edit");
        let after = connector_dir_sha256(tmp.path()).expect("hashes");
        assert_ne!(before, after, "an edited template must move the digest");
        assert_eq!(
            std::fs::read(tmp.path().join("manifest.toml")).expect("the manifest is still there"),
            b"id = \"demo\"\n",
            "the manifest was not what changed"
        );
    }

    #[test]
    fn a_change_to_the_manifest_alone_changes_the_digest_too() {
        let tmp = tempfile::tempdir().expect("a temporary directory");
        fixture(tmp.path());
        let before = connector_dir_sha256(tmp.path()).expect("hashes");
        std::fs::write(tmp.path().join("manifest.toml"), b"id = \"demo2\"\n").expect("edit");
        assert_ne!(
            before,
            connector_dir_sha256(tmp.path()).expect("hashes"),
            "the directory digest covers the manifest, so it is not weaker than the file pin \
             it replaces"
        );
    }

    #[test]
    fn a_new_file_nobody_declared_changes_the_digest() {
        let tmp = tempfile::tempdir().expect("a temporary directory");
        fixture(tmp.path());
        let before = connector_dir_sha256(tmp.path()).expect("hashes");
        std::fs::write(tmp.path().join("files/extra.sh"), b"whatever\n").expect("plant");
        assert_ne!(
            before,
            connector_dir_sha256(tmp.path()).expect("hashes"),
            "every regular file under the directory is in the stream, declared or not"
        );
    }

    /// The reason the separators are byte counts and not NULs.
    ///
    /// With `<path> 0x00 <body> 0x00` these two directories hash the same stream: the first
    /// holds one file `a` whose body is `b\0c`, and the second holds `a` holding `b` and `c`
    /// holding nothing — the NUL inside the body plays the part of a separator and the name
    /// `c` walks straight out of it. A length prefix cannot be forged from inside the bytes
    /// it counts, so the two are different digests.
    #[test]
    fn a_nul_inside_a_file_body_cannot_impersonate_a_second_file() {
        let one = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(one.path().join("a"), b"b\0c").expect("the body carrying a NUL");

        let two = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(two.path().join("a"), b"b").expect("a");
        std::fs::write(two.path().join("c"), b"").expect("c");

        assert_ne!(
            connector_dir_sha256(one.path()).expect("hashes"),
            connector_dir_sha256(two.path()).expect("hashes"),
            "a body holding a NUL must not be able to spell a neighbouring file"
        );
    }

    /// An empty file is a byte of information, and its absence is another one.
    ///
    /// A connector shipping a zero-length `files/override.json` and one shipping no such
    /// file are two different installs, and the digest has to say so — an empty file
    /// contributes only its name and two counts, which is exactly what makes the assertion
    /// worth writing down.
    #[test]
    fn an_empty_file_and_a_missing_file_are_different_digests() {
        let with = tempfile::tempdir().expect("a temporary directory");
        fixture(with.path());
        let without = connector_dir_sha256(with.path()).expect("hashes");

        std::fs::write(with.path().join("files/override.json"), b"").expect("the empty file");
        let present = connector_dir_sha256(with.path()).expect("hashes");
        assert_ne!(
            without, present,
            "an empty file is in the stream: it contributes its name and two counts"
        );

        std::fs::remove_file(with.path().join("files/override.json")).expect("remove it again");
        assert_eq!(
            connector_dir_sha256(with.path()).expect("hashes"),
            without,
            "and removing it puts the digest back, so the difference was the file itself"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_refused_rather_than_followed_or_skipped() {
        let tmp = tempfile::tempdir().expect("a temporary directory");
        fixture(tmp.path());
        std::os::unix::fs::symlink("manifest.toml", tmp.path().join("files/link.toml"))
            .expect("the symlink");
        let e = connector_dir_sha256(tmp.path()).expect_err("a symlink is not hashable");
        assert!(
            matches!(e, crate::ProtocolError::UnhashableEntry { .. }),
            "expected UnhashableEntry, got {e}"
        );
        assert!(
            e.to_string().contains("link.toml"),
            "the refusal names the entry: {e}"
        );
    }

    #[test]
    fn a_directory_that_is_not_there_is_an_io_error_naming_it() {
        let tmp = tempfile::tempdir().expect("a temporary directory");
        let e = connector_dir_sha256(&tmp.path().join("no-such-connector"))
            .expect_err("there is no such directory");
        assert!(
            matches!(e, crate::ProtocolError::Io { .. }),
            "expected Io, got {e}"
        );
        assert!(e.to_string().contains("no-such-connector"), "{e}");
    }
}
