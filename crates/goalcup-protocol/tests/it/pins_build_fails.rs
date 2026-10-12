//! A malformed `pins/connector-dirs.txt` fails the BUILD (harness contract v1, S2).
//!
//! The const parser in `src/pins.rs` panics during const evaluation, which no runtime test can
//! observe. So this test builds the module for real: it copies `pins.rs` next to a candidate
//! pins file in a temporary crate root and runs `rustc` on it. The module uses only `core` and
//! `std`, so this needs no dependencies and works offline. A well-formed file must compile (the
//! control), and each malformed one must not, with the parser's own message on stderr.

use std::path::Path;
use std::process::Command;

const GOOD: &str = "1111111111111111111111111111111111111111111111111111111111111111";

/// Compile `pins.rs` against `file`; `Ok` is a clean build, `Err` is rustc's stderr.
fn build_with(file: &str) -> Result<(), String> {
    let tmp = tempfile::tempdir().expect("a temporary crate root");
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).expect("src/");
    std::fs::create_dir_all(tmp.path().join("pins")).expect("pins/");
    std::fs::create_dir_all(tmp.path().join("out")).expect("out/");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/pins.rs"),
        src.join("pins.rs"),
    )
    .expect("copy pins.rs");
    std::fs::write(
        src.join("lib.rs"),
        "pub enum ProtocolError {\n    Field { field: String, reason: String },\n}\npub mod pins;\n",
    )
    .expect("lib.rs");
    std::fs::write(tmp.path().join("pins/connector-dirs.txt"), file).expect("pins file");
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let out = Command::new(rustc)
        .args([
            "--edition",
            "2024",
            "--crate-type",
            "lib",
            "--crate-name",
            "pins_probe",
        ])
        .args(["--emit", "metadata", "--out-dir"])
        .arg(tmp.path().join("out"))
        .arg(src.join("lib.rs"))
        .output()
        .expect("rustc runs");
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

#[test]
fn a_well_formed_file_builds() {
    build_with(&format!("# c\nalpha-cli {GOOD}\nbeta-cli {GOOD}\n"))
        .unwrap_or_else(|e| panic!("the control build failed:\n{e}"));
}

#[test]
fn each_malformed_file_fails_the_build_with_the_parsers_message() {
    let upper = GOOD.replace('1', "A");
    let cases: Vec<(String, &str)> = vec![
        (
            format!("beta-cli {GOOD}\nalpha-cli {GOOD}\n"),
            "is not sorted by id",
        ),
        (
            format!("alpha-cli {GOOD}\nalpha-cli {GOOD}\n"),
            "repeats an id",
        ),
        (format!("alpha-cli {upper}\n"), "not 64 lowercase hex"),
        (format!("Alpha-cli {GOOD}\n"), "holds an id that is not"),
        (format!("alpha-cli {GOOD}"), "does not end with a newline"),
        (format!("alpha-cli  {GOOD}\n"), "exactly one space"),
        (format!("alpha-cli {GOOD}\n\n"), "blank line"),
        ("# nothing\n".to_string(), "would pin nothing"),
    ];
    for (file, message) in cases {
        let stderr = build_with(&file).expect_err(&format!("{file:?} must not build"));
        assert!(
            stderr.contains(message),
            "{file:?}: rustc failed, but not with `{message}`:\n{stderr}"
        );
    }
}
