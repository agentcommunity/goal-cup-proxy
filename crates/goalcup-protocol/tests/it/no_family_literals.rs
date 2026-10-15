//! No harness or vendor name in core code: a harness is connector data plus shared modules.
//!
//! Every source file of the protocol, daemon and verifier crates is scanned for string literals
//! that contain a harness name. The capture paths of the CLI and platform crates are held to the
//! narrower rule they had before: no literal that is exactly a family name. Comments and
//! identifiers are not scanned. A file where a name is data stays in `ALLOWED`, with its reason.

use std::path::{Path, PathBuf};

/// Lower-case fragments no string literal may contain, compared case-insensitively. They cover
/// the connector ids (`claude-code-cli`, `codex-cli`, `hermes-cli`), the native formats
/// (`codex-jsonl`, `claude-code-jsonl`) and the vendor CA variables (`CODEX_CA_CERTIFICATE`,
/// `HERMES_CA_BUNDLE`).
const FORBIDDEN: &[&str] = &["hermes", "codex", "claude"];

/// The literals the CLI and platform capture paths may not be.
const FAMILY_LITERALS: &[&str] = &["hermes", "codex", "claude-code"];

/// Files whose harness names are data, relative to the repository root. Each must still hold a
/// hit, so a cleaned file leaves this list.
const ALLOWED: &[(&str, &str)] = &[
    (
        "crates/goalcup-protocol/src/capture_policy.rs",
        "closed launch-environment tables (CA registry, refused names, provider prefixes); a policy cannot come from the connector it constrains (D41, D43)",
    ),
    (
        "crates/goalcup-verifier/src/capture_analysis/native.rs",
        "the native decoders keyed by the `protocol_hint` a connector declares, and the decoder ids a report prints",
    ),
    (
        "crates/goalcup-verifier/src/capture_analysis/registry.rs",
        "each built-in decoder's evidence note names the harness build it was measured against",
    ),
];

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

/// The contents of every string literal in `source` (plain, byte and raw), with the line each
/// starts on. Comments, char literals and lifetimes are skipped.
fn string_literals(source: &str) -> Vec<(usize, String)> {
    let b = source.as_bytes();
    let mut out = Vec::new();
    let mut line = 1;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\n' => line += 1,
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let mut depth = 0;
                while i < b.len() {
                    if b[i..].starts_with(b"/*") {
                        depth += 1;
                        i += 2;
                    } else if b[i..].starts_with(b"*/") {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        line += usize::from(b[i] == b'\n');
                        i += 1;
                    }
                }
                continue;
            }
            // A char literal ('"', '\'', '\u{22}'); a lifetime has no closing quote.
            b'\'' => {
                let end = if b.get(i + 1) == Some(&b'\\') {
                    b[i + 2..]
                        .iter()
                        .position(|c| *c == b'\'')
                        .map(|p| i + 2 + p)
                } else {
                    let len = source[i + 1..].chars().next().map_or(1, char::len_utf8);
                    (b.get(i + 1 + len) == Some(&b'\'')).then_some(i + 1 + len)
                };
                if let Some(end) = end {
                    i = end + 1;
                    continue;
                }
            }
            b'"' => {
                let hashes = b[..i].iter().rev().take_while(|c| **c == b'#').count();
                let before = i - hashes;
                let raw = before > 0
                    && b[before - 1] == b'r'
                    && (before < 2 || !is_ident(b[before - 2]) || b[before - 2] == b'b');
                let start = i + 1;
                let mut j = start;
                let end = loop {
                    if j >= b.len() {
                        break b.len();
                    }
                    if raw {
                        if b[j] == b'"'
                            && b[j + 1..].iter().take_while(|c| **c == b'#').count() >= hashes
                        {
                            break j;
                        }
                    } else if b[j] == b'\\' {
                        j += 1;
                    } else if b[j] == b'"' {
                        break j;
                    }
                    j += 1;
                };
                out.push((line, source[start..end].to_string()));
                line += source[start..end].matches('\n').count();
                i = end + 1 + if raw { hashes } else { 0 };
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

#[test]
fn the_scanner_reads_literals_only() {
    let source = "// \"codex\" in a comment\n/* \"claude\" /* nested */ */ let codex = 'x';\n\
                  let q = '\"'; let s = \"a \\\" hermes\";\nlet r = r#\"raw \"quoted\" claude\"#;\n\
                  fn f<'a>(x: &'a str) -> &'a [u8] { b\"Codex\" }";
    let literals = string_literals(source);
    assert_eq!(
        literals,
        [
            (3, "a \\\" hermes".to_string()),
            (4, "raw \"quoted\" claude".to_string()),
            (5, "Codex".to_string()),
        ]
    );
}

#[test]
fn core_code_names_no_harness_in_a_string_literal() {
    let root = repo_root();
    let capture_paths = [
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
    ];
    let mut files = capture_paths.to_vec();
    for core in ["goalcup-protocol", "goalcup-daemon", "goalcup-verifier"] {
        rust_files_below(&root.join("crates").join(core).join("src"), &mut files);
    }
    files.sort();

    let mut hits = Vec::new();
    let mut allowed_hit = vec![false; ALLOWED.len()];
    for path in &files {
        let relative = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let allowed = ALLOWED.iter().position(|(file, _)| *file == relative);
        let narrow = capture_paths.contains(path);
        let source =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for (line, literal) in string_literals(&source) {
            let lower = literal.to_ascii_lowercase();
            let named = if narrow {
                FAMILY_LITERALS.contains(&literal.as_str())
            } else {
                FORBIDDEN.iter().any(|name| lower.contains(name))
            };
            if named {
                match allowed {
                    Some(n) => allowed_hit[n] = true,
                    None => hits.push(format!("{relative}:{line}: {literal:?}")),
                }
            }
        }
    }

    assert!(
        hits.is_empty(),
        "a harness name belongs to connector data or a test fixture, not core code:\n{}",
        hits.join("\n")
    );
    let stale: Vec<_> = ALLOWED
        .iter()
        .zip(&allowed_hit)
        .filter(|(_, hit)| !**hit)
        .map(|((file, _), _)| *file)
        .collect();
    assert!(
        stale.is_empty(),
        "allowed files that no longer name a harness: {stale:?}"
    );
}
