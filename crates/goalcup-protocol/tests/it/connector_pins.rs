//! Directory-pin drift, in the protocol crate so CI runs it on a fork PR (harness contract v1,
//! I4; `ci.yml` excludes only the daemon and CLI suites). It needs no shell: it hashes every
//! `connectors/*/` folder in Rust and compares with the pins this build embedded from
//! `pins/connector-dirs.txt`, in both directions.
//!
//! `goalcup-verifier/tests/it/checks_v13.rs` keeps the other half, the maintainer gate that the
//! shell script and the Rust walk compute one digest.

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
