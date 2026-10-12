//! Directory-pin drift, in the protocol crate so CI runs it on a fork PR (harness contract v1,
//! I4; `ci.yml` excludes only the daemon and CLI suites). It needs no shell: it hashes every
//! `connectors/*/` folder in Rust and compares with the pins this build embedded from
//! `pins/connector-dirs.txt`, in both directions.
//!
//! The last three tests are the other half: `scripts/pin-manifests.sh` and the Rust walk
//! compute one digest, key the table by the declared id, and write the committed table.

use goalcup_protocol::connector_dir::connector_dir_sha256;
use goalcup_protocol::pins::{
    CONNECTOR_DIR_SHA256S, PinTable, connector_dir_is_pinned, is_pinned, parse_pins_text,
    pinned_connector_dir_sha256,
};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The id a connector folder declares: its `harness.toml`'s, else its `manifest.toml`'s. Read
/// as a generic TOML table so this test needs neither the V1 nor the schema-1 types.
fn declared_id(dir: &Path) -> String {
    let mut ids = Vec::new();
    for file in ["harness.toml", "manifest.toml"] {
        let path = dir.join(file);
        if !path.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        let table: toml::Table = text.parse().unwrap_or_else(|e| panic!("{path:?}: {e}"));
        let id = table
            .get("id")
            .and_then(toml::Value::as_str)
            .unwrap_or_else(|| panic!("{path:?} declares no top-level id"))
            .to_string();
        ids.push(id);
    }
    assert!(
        !ids.is_empty(),
        "{dir:?} holds neither harness.toml nor manifest.toml"
    );
    assert!(
        ids.windows(2).all(|w| w[0] == w[1]),
        "{dir:?}: harness.toml and manifest.toml declare different ids {ids:?}"
    );
    ids.remove(0)
}

fn connector_folders() -> Vec<(String, PathBuf)> {
    let root = repo_root().join("connectors");
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("{root:?}: {e}"))
        .map(|e| e.expect("a directory entry").path())
        .inspect(|p| {
            let kind = std::fs::symlink_metadata(p).expect("metadata").file_type();
            assert!(
                !kind.is_symlink(),
                "{p:?} is a symlink, not a connector folder"
            );
        })
        .filter(|p| p.is_dir())
        .map(|p| (declared_id(&p), p))
        .collect();
    out.sort();
    out
}

#[test]
fn every_connector_folder_hashes_to_its_pin() {
    let folders = connector_folders();
    assert!(!folders.is_empty(), "no connector folders found");
    for (id, dir) in &folders {
        let want = pinned_connector_dir_sha256(id).unwrap_or_else(|| {
            panic!("{id}: no pin in crates/goalcup-protocol/pins/connector-dirs.txt; run `sh scripts/pin-manifests.sh`")
        });
        let got = connector_dir_sha256(dir).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(
            got, want,
            "{id}: the directory changed without its pin; run `sh scripts/pin-manifests.sh` \
             on the merged tree and commit the one changed line"
        );
    }
}

#[test]
fn every_pin_names_a_connector_folder() {
    let ids: Vec<String> = connector_folders().into_iter().map(|(id, _)| id).collect();
    for (id, _) in CONNECTOR_DIR_SHA256S {
        assert!(
            ids.iter().any(|i| i == id),
            "pin `{id}` names no connectors/*/ folder; run `sh scripts/pin-manifests.sh`"
        );
    }
    assert_eq!(
        ids.len(),
        CONNECTOR_DIR_SHA256S.len(),
        "folders {ids:?} and pins must be one to one"
    );
}

#[test]
fn declared_ids_are_unique() {
    let mut ids: Vec<String> = connector_folders().into_iter().map(|(id, _)| id).collect();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "two folders declare one connector id");
}

/// The embedded table is exactly the committed file, in file order: the file the build read
/// is the file the script wrote.
#[test]
fn the_embedded_table_is_the_committed_file() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("pins/connector-dirs.txt");
    let text = std::fs::read_to_string(&path).expect("the pins file");
    let parsed = parse_pins_text(&text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(parsed.len(), CONNECTOR_DIR_SHA256S.len());
    for ((id, sha), (eid, esha)) in parsed.iter().zip(CONNECTOR_DIR_SHA256S) {
        assert_eq!((id.as_str(), sha.as_str()), (*eid, *esha));
    }
}

/// `is_pinned` is the one comparison of a pin: by the declared id, first row wins, hex case is
/// not meaningful, an id the table lacks is not pinned. `connector_dir_is_pinned` is it over the
/// embedded table.
#[test]
fn is_pinned_is_the_one_comparison_over_any_table() {
    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const TABLE: PinTable = &[("one-cli", A), ("one-cli", B), ("two-cli", B)];
    assert!(is_pinned(TABLE, "one-cli", A));
    assert!(is_pinned(TABLE, "two-cli", B));
    assert!(
        is_pinned(TABLE, "one-cli", &A.to_ascii_uppercase()),
        "hex case is not meaningful"
    );
    assert!(
        !is_pinned(TABLE, "one-cli", B),
        "the first row for an id wins"
    );
    assert!(
        !is_pinned(TABLE, "three-cli", A),
        "an id the table lacks is not pinned"
    );
    assert!(!is_pinned(&[], "one-cli", A), "an empty table pins nothing");
    assert!(!is_pinned(TABLE, "one-cli", ""));
    for (id, sha) in CONNECTOR_DIR_SHA256S {
        assert!(connector_dir_is_pinned(id, sha), "{id}");
        assert!(
            is_pinned(CONNECTOR_DIR_SHA256S, id, &sha.to_ascii_uppercase()),
            "{id}"
        );
        assert!(!connector_dir_is_pinned(id, A), "{id}");
    }
}

/// `scripts/pin-manifests.sh` and `connector_dir_sha256` agree, on every shipped connector.
///
/// The byte layout — `<u64 be path length> <path> <u64 be body length> <body>`, in sorted
/// path order — is written down in two places, the script's header and the Rust doc
/// comment, and implemented twice: once in `sh` with `find`, `sort`, `printf` and `cat`,
/// once in Rust. This runs the script's own `--print` mode and compares. Without it the two
/// spellings drift the first time somebody adds a nested directory under a connector, and
/// the failure surfaces as a build that refuses its own shipped connectors.
#[test]
fn the_pin_script_and_the_protocol_function_compute_the_same_digest() {
    let root = repo_root();
    let out = std::process::Command::new("sh")
        .arg("scripts/pin-manifests.sh")
        .arg("--print")
        .current_dir(&root)
        .output()
        .expect("scripts/pin-manifests.sh runs");
    assert!(
        out.status.success(),
        "pin-manifests.sh --print failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let printed = String::from_utf8(out.stdout).expect("the script prints ids and hex");
    let mut seen = 0;
    for line in printed.lines() {
        let mut fields = line.split(' ');
        let id = fields.next().expect("an id");
        let dir_digest = fields.next().expect("a directory digest");
        assert_eq!(fields.next(), None, "two fields a line: {line:?}");
        assert_eq!(
            pinned_connector_dir_sha256(id),
            Some(dir_digest),
            "{id}: the script's directory digest is the pinned one"
        );
        assert_eq!(
            connector_dir_sha256(&root.join("connectors").join(id))
                .unwrap_or_else(|e| panic!("{id}: {e}")),
            dir_digest,
            "{id}: the shell and the Rust walk hash the same stream"
        );
        seen += 1;
    }
    // Counted from the tree, so a new connector folder needs no edit here.
    assert!(
        seen >= 3,
        "at least the three shipped connectors, got {printed:?}"
    );
    assert_eq!(
        seen,
        connector_folders().len(),
        "one line per connector folder, got {printed:?}"
    );
}

/// Everything under `from`, copied into `to`. Regular files and directories only, which is
/// all a connector directory or a `scripts/` directory holds.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("the destination directory");
    for entry in std::fs::read_dir(from).expect("the source directory") {
        let entry = entry.expect("a directory entry");
        let dst = to.join(entry.file_name());
        if entry.file_type().expect("the entry's kind").is_dir() {
            copy_tree(&entry.path(), &dst);
        } else {
            std::fs::copy(entry.path(), &dst).expect("a file copy");
        }
    }
}

/// **The table is keyed by the id the connector declares**, not by the directory's
/// basename.
///
/// Every lookup asks for a pin by the declared id, because a connector id is never joined
/// onto a path. A table keyed by the basename therefore does not mis-pin a directory whose
/// name and declared id differ: it pins nothing for it, silently, and every lookup reads back
/// `None`, which is the word for an unpinned connector rather than for a mismatch. The
/// synthetic connector below is the case, and it is synthetic because every shipped
/// connector happens to agree.
#[test]
fn the_script_keys_the_pin_table_by_the_declared_id_and_not_by_the_directory_name() {
    let tmp = tempfile::tempdir().expect("a temporary repository");
    copy_tree(&repo_root().join("scripts"), &tmp.path().join("scripts"));
    let dir = tmp
        .path()
        .join("connectors/a-directory-called-something-else");
    std::fs::create_dir_all(dir.join("files")).expect("the synthetic connector");
    std::fs::write(dir.join("harness.toml"), b"id = \"declared-elsewhere\"\n").expect("harness");
    std::fs::write(dir.join("files/hook.sh"), b"#!/bin/sh\nexit 0\n").expect("a template");

    let out = std::process::Command::new("sh")
        .arg("scripts/pin-manifests.sh")
        .arg("--print")
        .current_dir(tmp.path())
        .output()
        .expect("scripts/pin-manifests.sh runs");
    assert!(
        out.status.success(),
        "--print failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let printed = String::from_utf8(out.stdout).expect("ids and hex");
    let fields: Vec<&str> = printed.trim_end().split(' ').collect();
    assert_eq!(fields.len(), 2, "one connector, two fields: {printed:?}");
    assert_eq!(
        fields[0], "declared-elsewhere",
        "the key is the declared id and not `a-directory-called-something-else`"
    );
    assert_eq!(
        fields[1],
        connector_dir_sha256(&dir).expect("the directory hashes"),
        "and the directory digest is still that directory"
    );
}

/// **The script is idempotent, and the committed table is what it writes.**
///
/// Run over a copy of the repository rather than over the repository, so a test can never
/// rewrite a source file. Two runs, and three claims: the first run reproduces the bytes
/// that are committed (which is what `sh scripts/pin-manifests.sh && git diff --exit-code`
/// claims in the script's own header), the second changes nothing, and the row order is
/// therefore not a function of whoever ran it — `LC_ALL=C` is exported in the script so the
/// `connectors/*/` glob and the walk's `sort` compare bytes.
///
/// The table is generated data, so it is not copied in first — the script has to create it.
#[test]
fn the_pin_script_writes_the_committed_tables_and_a_second_run_changes_nothing() {
    let root = repo_root();
    let tmp = tempfile::tempdir().expect("a temporary repository");
    copy_tree(&root.join("scripts"), &tmp.path().join("scripts"));
    copy_tree(&root.join("connectors"), &tmp.path().join("connectors"));
    let generated = "crates/goalcup-protocol/pins/connector-dirs.txt";

    let run = || {
        let out = std::process::Command::new("sh")
            .arg("scripts/pin-manifests.sh")
            .current_dir(tmp.path())
            .output()
            .expect("scripts/pin-manifests.sh runs");
        assert!(
            out.status.success(),
            "pin-manifests.sh failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };

    run();
    let after_one = std::fs::read(tmp.path().join(generated)).expect("the generated file");
    assert_eq!(
        after_one,
        std::fs::read(root.join(generated)).expect("the committed file"),
        "{generated}: the committed pins are not what the script produces — run \
         `sh scripts/pin-manifests.sh` and commit the diff"
    );
    run();
    assert_eq!(
        std::fs::read(tmp.path().join(generated)).expect("the generated file"),
        after_one,
        "{generated}: a second run moved bytes, so the script is not idempotent"
    );
    assert!(
        !tmp.path().join("crates/goalcup-verifier").exists(),
        "the script writes no table outside the protocol crate"
    );
}
