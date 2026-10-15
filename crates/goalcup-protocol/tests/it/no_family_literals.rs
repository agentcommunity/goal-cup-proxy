//! Schema-1 capture paths route from connector data, never a compiled harness family.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rust_files_below(path: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())) {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            rust_files_below(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn capture_paths_contain_no_compiled_family_routing_literals() {
    let root = repo_root();
    let mut files = vec![
        root.join("crates/goalcup-cli/src/launch.rs"),
        root.join("crates/goalcup-cli/src/capture_tee.rs"),
        root.join("crates/goalcup-cli/src/per_run_home.rs"),
        root.join("crates/goalcup-cli/src/commands/start.rs"),
        root.join("crates/goalcup-cli/src/commands/continue_run.rs"),
        root.join("crates/goalcup-cli/src/commands/prep.rs"),
        root.join("crates/goalcup-cli/src/host_log.rs"),
        root.join("crates/goalcup-cli/src/skill.rs"),
        root.join("crates/goalcup-platform/src/terminal.rs"),
        root.join("crates/goalcup-cli/src/prep_file.rs"),
        root.join("crates/goalcup-cli/src/run/mod.rs"),
        root.join("crates/goalcup-cli/src/run/reserve.rs"),
        root.join("crates/goalcup-cli/src/run/watch.rs"),
        root.join("crates/goalcup-cli/src/upload/policy.rs"),
        root.join("crates/goalcup-cli/src/upload/prune.rs"),
        root.join("crates/goalcup-daemon/src/native_capture.rs"),
        root.join("crates/goalcup-daemon/src/workspace_git.rs"),
        root.join("crates/goalcup-protocol/src/harness.rs"),
        root.join("crates/goalcup-protocol/src/capture_policy.rs"),
    ];
    rust_files_below(
        &root.join("crates/goalcup-daemon/src/runtime/capture"),
        &mut files,
    );
    files.sort();

    let forbidden = [r#""hermes""#, r#""codex""#, r#""claude-code""#];
    let mut hits = Vec::new();
    for path in files {
        let source =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for literal in forbidden {
            for (line, _) in source.match_indices(literal) {
                let line = source[..line].bytes().filter(|b| *b == b'\n').count() + 1;
                hits.push(format!("{}:{line}: {literal}", path.display()));
            }
        }
    }

    assert!(
        hits.is_empty(),
        "capture routing belongs to harness.toml, not family literals:\n{}",
        hits.join("\n")
    );
}
