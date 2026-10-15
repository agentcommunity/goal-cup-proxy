//! Per-connector static checks, in the protocol crate so CI runs them on a fork PR (harness
//! contract v1, I4; the real files closed S2 review item M-2).
//!
//! For EVERY folder under `connectors/`: it holds a `harness.toml` (this file), and the
//! checks below that this file owns. Everything `harness::load_dir` already enforces (loads as
//! `Origin::Pinned`, every `files/**` entry is a seed, nothing else sits beside `harness.toml`,
//! every seed exists and is small) is NOT restated
//! here: `load_dir` is the one loader the CLI and the daemon share, so `check_dir` calls it and
//! the sensitivity tests prove it refuses (`validate` also owns "`extra_patterns` are on the
//! protocol mirror of the recognizers"). What this file adds on top, each proven by a test that
//! no earlier gate refuses first:
//! * the declared id is the folder name (`load_dir` does not compare with the folder name);
//! * no non-comment line of `harness.toml` names the tracker, a rule broader than
//!   `validate`'s (which scans args, goal flag, env and hosts, not `continuation` or
//!   `evidence_paths`).
//!
//! The set the loop runs over is itself asserted (M-2): at least the three shipped connectors,
//! and exactly as many `harness.toml` files as connector folders, so a folder without one, or a
//! loop that silently iterates over nothing, fails here. A connector
//! folder that is a symlink is refused too (`load_dir` and `connector_dirs` both).
//!
//! The kit (offline stand-ins, macOS) stays a maintainer gate.

use goalcup_protocol::capture_policy::{EXTRA_PATTERN_SOURCES, mentions_goalcup};
use goalcup_protocol::harness::{Origin, load_dir};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The connectors the build ships: a floor, so one cannot vanish unnoticed. Not a ceiling: a
/// harness PR adds a folder and every loop and count below follows the directory, not this
/// list, so a new folder is checked with no edit here.
const SHIPPED: [&str; 3] = ["claude-code-cli", "codex-cli", "hermes-cli"];

/// Every entry under `connectors/`, sorted, judged by `symlink_metadata` so a link is not
/// followed into a directory it only points at. No filter: a folder without a `harness.toml`
/// is a failure, not a skip, and a symlinked folder is a failure too (`connector_dirs_are_real`).
fn connector_dirs() -> Vec<PathBuf> {
    let dir = root().join("connectors");
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.expect("entry").path())
        .filter(|p| {
            let t = std::fs::symlink_metadata(p).expect("metadata").file_type();
            t.is_dir() || t.is_symlink()
        })
        .collect();
    out.sort();
    out
}

/// All per-connector assertions for the folder `dir`, as `Err(why)` rather than a panic so the
/// sensitivity tests can assert a refusal. `load_dir` owns the directory-shape rules.
fn check_dir(dir: &Path) -> Result<(), String> {
    let name = dir.display();
    if !dir.join("harness.toml").is_file() {
        return Err(format!("{name}: no harness.toml"));
    }
    let loaded = load_dir(dir, Origin::Pinned).map_err(|e| format!("{name}: {e}"))?;
    let m = &loaded.manifest;

    if Some(m.id.as_str()) != dir.file_name().and_then(|n| n.to_str()) {
        return Err(format!(
            "{name}: declares id `{}`, which is not the folder name",
            m.id
        ));
    }

    // F7, restated over the raw text: no field of the file names the tracker. Comments are
    // provenance prose, not fields.
    let text = String::from_utf8(loaded.harness_toml.clone()).map_err(|e| e.to_string())?;
    for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
        if mentions_goalcup(line) {
            return Err(format!("{}: `{line}` names the tracker (F7)", m.id));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------------------
// The set under test (M-2)
// ---------------------------------------------------------------------------------------

#[test]
fn the_loop_is_not_vacuous_and_every_connector_folder_ships_a_harness_toml() {
    let dirs = connector_dirs();
    let with_harness = dirs
        .iter()
        .filter(|d| d.join("harness.toml").is_file())
        .count();
    assert!(
        with_harness >= SHIPPED.len(),
        "found {with_harness} connectors/*/harness.toml; the build ships at least {}",
        SHIPPED.len()
    );
    assert_eq!(
        with_harness,
        dirs.len(),
        "{} connector folders but {with_harness} harness.toml files: every folder must ship one",
        dirs.len()
    );
    for id in SHIPPED {
        assert!(
            root()
                .join("connectors")
                .join(id)
                .join("harness.toml")
                .is_file(),
            "{id}: the shipped connector has no harness.toml"
        );
    }
}

#[test]
fn connector_dirs_are_real_directories() {
    for d in connector_dirs() {
        let t = std::fs::symlink_metadata(&d).unwrap().file_type();
        assert!(
            t.is_dir() && !t.is_symlink(),
            "{}: not a real directory",
            d.display()
        );
    }
}

#[test]
fn every_connector_passes_every_static_check() {
    for dir in connector_dirs() {
        check_dir(&dir).unwrap_or_else(|e| panic!("{e}"));
    }
}

/// The mirror equals the patterns the shipped `harness.toml` files carry, in connector order,
/// which the daemon's own test `extra_shapes_follow_the_shipped_harnesses` equates with
/// `EXTRA_SHAPES`: so the mirror equals the recorder's recognizers by transitivity, without a
/// daemon edit.
#[test]
fn the_extra_pattern_mirror_equals_the_shipped_harness_patterns() {
    let mut shipped: Vec<String> = Vec::new();
    for id in ["codex-cli", "claude-code-cli"] {
        let dir = root().join("connectors").join(id);
        let loaded = load_dir(&dir, Origin::Pinned).unwrap_or_else(|e| panic!("{id}: {e}"));
        shipped.extend(
            loaded
                .manifest
                .redaction
                .iter()
                .flat_map(|r| &r.extra_patterns)
                .map(|p| p.regex.clone()),
        );
    }
    let mirror: Vec<String> = EXTRA_PATTERN_SOURCES
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        shipped, mirror,
        "capture_policy::EXTRA_PATTERN_SOURCES must list the recognizers in order: codex's \
         three, then claude's one"
    );
}

// ---------------------------------------------------------------------------------------
// Sensitivity: break one value in a temp copy, and `check_dir` must refuse it
// ---------------------------------------------------------------------------------------

/// Copy `connectors/<id>/` into a fresh temp directory named `folder`, and return both.
fn copy_connector_as(id: &str, folder: &str) -> (tempfile::TempDir, PathBuf) {
    copy_dir_as(&root().join("connectors").join(id), folder)
}

/// [`copy_connector`] of the frozen copy of `id` as it shipped while headless, for the tests
/// of the headless recipe's text.
fn copy_headless_connector(id: &str) -> (tempfile::TempDir, PathBuf) {
    copy_dir_as(
        &root()
            .join("crates/goalcup-protocol/tests/fixtures/headless-connectors/")
            .join(id),
        id,
    )
}

fn copy_dir_as(source: &Path, folder: &str) -> (tempfile::TempDir, PathBuf) {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            let dest = to.join(e.file_name());
            if e.path().is_dir() {
                copy(&e.path(), &dest);
            } else {
                std::fs::copy(e.path(), &dest).unwrap();
            }
        }
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join(folder);
    copy(source, &dir);
    (tmp, dir)
}

fn copy_connector(id: &str) -> (tempfile::TempDir, PathBuf) {
    copy_connector_as(id, id)
}

/// Replace `from` with `to` in a copied file, asserting `from` was there (so a mutation is
/// never a silent no-op).
fn mutate(file: &Path, from: &str, to: &str) {
    let text = std::fs::read_to_string(file).unwrap();
    assert!(text.contains(from), "{from:?} not in {}", file.display());
    std::fs::write(file, text.replacen(from, to, 1)).unwrap();
}

#[test]
fn sensitivity_an_unmutated_copy_passes() {
    for dir in connector_dirs() {
        let id = dir.file_name().unwrap().to_str().unwrap();
        let (_tmp, copy) = copy_connector(id);
        check_dir(&copy).unwrap_or_else(|e| panic!("{e}"));
    }
}

#[test]
fn sensitivity_a_missing_harness_toml_is_refused() {
    let (_tmp, dir) = copy_connector("codex-cli");
    std::fs::remove_file(dir.join("harness.toml")).unwrap();
    let err = check_dir(&dir).expect_err("a folder without harness.toml");
    assert!(err.contains("no harness.toml"), "{err}");
}

// The next two are `load_dir`'s rules, not `check_dir`'s: the tests prove the loader that
// `check_dir` calls refuses, with the loader's own sentence.

#[test]
fn sensitivity_load_dir_refuses_an_unnamed_file_under_files() {
    let (_tmp, dir) = copy_connector("hermes-cli");
    std::fs::write(dir.join("files/stray.txt"), b"x\n").unwrap();
    let err = check_dir(&dir).expect_err("a stray file");
    assert!(
        err.contains("`files/stray.txt` is not named by `capture.home.seed`"),
        "{err}"
    );
}

#[test]
fn sensitivity_load_dir_refuses_a_seed_the_directory_lacks() {
    let (_tmp, dir) = copy_connector("hermes-cli");
    std::fs::remove_file(dir.join("files/auth.json")).unwrap();
    let err = check_dir(&dir).expect_err("a missing seed");
    assert!(
        err.contains("`files/auth.json` is named by the manifest but is not in the directory"),
        "{err}"
    );
}

#[test]
fn sensitivity_load_dir_refuses_an_extra_pattern_off_the_mirror() {
    let (_tmp, dir) = copy_connector("claude-code-cli");
    mutate(
        &dir.join("harness.toml"),
        "x-anthropic-billing-[a-z0-9-]+",
        "x-anthropic-billing-[a-z0-9]+",
    );
    let err = check_dir(&dir).expect_err("a pattern the recorder does not recognize");
    assert!(
        err.contains("is not one the recorder's streaming redactor can hold"),
        "{err}"
    );
}

/// The raw-text F7 loop is reached only by a value `validate` does not scan: `continuation`
/// is documentation, never an argument, env value or host.
#[test]
fn sensitivity_f7_catches_a_value_validate_does_not_scan() {
    let (_tmp, dir) = copy_headless_connector("codex-cli");
    mutate(
        &dir.join("harness.toml"),
        "full access with network, stdin",
        "full access with network, see goalcup, stdin",
    );
    // The loader accepts it, so only the raw-text rule can refuse.
    load_dir(&dir, Origin::Pinned).expect("validate does not scan `continuation`");
    let err = check_dir(&dir).expect_err("a continuation naming the tracker");
    assert!(err.contains("(F7)"), "{err}");
}

/// The id-versus-folder rule is `check_dir`'s own. The copy has an untouched id, so no earlier
/// gate refuses it.
#[test]
fn sensitivity_an_id_that_is_not_the_folder_name_is_refused() {
    let (_tmp, dir) = copy_connector_as("hermes-cli", "other");
    load_dir(&dir, Origin::Pinned).expect("the loader accepts any folder name");
    let err = check_dir(&dir).expect_err("an id that differs from the folder");
    assert!(err.contains("not the folder name"), "{err}");
}

#[test]
fn sensitivity_a_value_the_pinned_origin_refuses_is_refused() {
    // An interpreter as the program (N3): admitted only for `Origin::Dev`.
    let (_tmp, dir) = copy_connector("hermes-cli");
    mutate(
        &dir.join("harness.toml"),
        "program = \"hermes\"",
        "program = \"python3\"",
    );
    let err = check_dir(&dir).expect_err("an interpreter program in a pinned connector");
    assert!(err.to_ascii_lowercase().contains("interpreter"), "{err}");

    // A model base URL in the capture environment (D1).
    let (_tmp, dir) = copy_connector("claude-code-cli");
    mutate(
        &dir.join("harness.toml"),
        "DISABLE_AUTOUPDATER = \"1\"",
        "ANTHROPIC_BASE_URL = \"http://127.0.0.1:1\"",
    );
    let err = check_dir(&dir).expect_err("a base URL in capture.env");
    assert!(err.contains("ANTHROPIC_BASE_URL"), "{err}");
}
