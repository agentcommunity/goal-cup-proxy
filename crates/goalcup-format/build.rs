//! Builds `$OUT_DIR/connectors.rs`: every `connectors/*.toml` except the two list files and
//! hidden files (`._x.toml` from macOS), by stem.

use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../connectors");
    println!("cargo:rerun-if-changed=../../connectors");
    let mut files: Vec<(String, PathBuf)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "toml"))
        .map(|p| (p.file_stem().unwrap().to_string_lossy().into_owned(), p))
        .filter(|(stem, _)| !stem.starts_with('.') && stem != "model-hosts" && stem != "tiers")
        .collect();
    files.sort();
    let mut out = String::from("pub const CONNECTORS: &[(&str, &str)] = &[\n");
    for (stem, path) in files {
        let path = path.canonicalize().unwrap();
        out += &format!("    ({stem:?}, include_str!({path:?})),\n");
    }
    out += "];\n";
    let dest = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("connectors.rs");
    std::fs::write(dest, out).unwrap();
}
