//! Point 7 and the AGENTS.md hard rule: no harness names in core code. A harness and a model host
//! are data (`connectors/`), so no string literal in `crates/goalcup/src` or
//! `crates/goalcup-format/src` holds a connector id (the stem of each flat `connectors/*.toml`) or a
//! host from `connectors/model-hosts.toml`, both matched case-insensitively as whole tokens. The
//! lists are read at test time, so a new connector or host is covered with no edit here. Comments
//! and identifiers are not scanned.

use std::path::{Path, PathBuf};

/// Exceptions: (file from the repository root, literal text, reason). An empty text exempts the
/// whole file. An entry that no longer excuses a hit fails as stale.
const ALLOWED: &[(&str, &str, &str)] = &[(
    "crates/goalcup/src/connector/builtin.rs",
    "",
    "the one include_str! list that embeds the connector files by id (PRD-11); skipped while absent",
)];

/// Connector ids, then the fixed dotted parts of each host pattern (`*.maas.aliyuncs.com` gives
/// `maas.aliyuncs.com`, `bedrock-runtime.*.amazonaws.com` gives `amazonaws.com`).
fn banned(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root.join("connectors")).expect("connectors/") {
        let path = entry.expect("entry").path();
        let stem = path.file_stem().unwrap().to_string_lossy().to_ascii_lowercase();
        if path.extension().is_some_and(|e| e == "toml") && !["model-hosts", "tiers"].contains(&&*stem) {
            out.push(stem);
        }
    }
    let hosts = std::fs::read_to_string(root.join("connectors/model-hosts.toml")).expect("hosts");
    for line in hosts.lines().map(|l| l.split('#').next().unwrap()) {
        for pattern in line.split('"').skip(1).step_by(2) {
            let labels: Vec<_> = pattern.split('.').collect();
            for fixed in labels.split(|label| label.contains('*')).filter(|run| run.len() > 1) {
                out.push(fixed.join(".").to_ascii_lowercase());
            }
        }
    }
    assert!(out.len() > 8, "too few names read from connectors/: {out:?}");
    out
}

/// Whether `lower` holds `name` with no `[a-z0-9]` right before or after it.
fn names_token(lower: &str, name: &str) -> bool {
    let token = |c: Option<u8>| c.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    lower.match_indices(name).any(|(at, _)| {
        !token(lower[..at].bytes().next_back()) && !token(lower[at + name.len()..].bytes().next())
    })
}

/// Every string literal (plain, byte, raw) with its first line; comments and char literals skipped.
fn string_literals(source: &str) -> Vec<(usize, String)> {
    let b = source.as_bytes();
    let (mut out, mut line, mut i) = (Vec::new(), 1, 0);
    while i < b.len() {
        if b[i..].starts_with(b"//") {
            i += b[i..].iter().position(|c| *c == b'\n').unwrap_or(b.len() - i);
            continue;
        }
        if b[i..].starts_with(b"/*") {
            let mut depth = 0;
            while i < b.len() {
                if b[i..].starts_with(b"/*") || b[i..].starts_with(b"*/") {
                    depth += if b[i] == b'/' { 1 } else { -1 };
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
        match b[i] {
            b'\n' => line += 1,
            // A char literal ('"', '\'', '\u{22}'); a lifetime has no closing quote.
            b'\'' => {
                let end = if b.get(i + 1) == Some(&b'\\') {
                    b[i + 2..].iter().position(|c| *c == b'\'').map(|p| i + 2 + p)
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
                let at = i - hashes;
                let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
                let raw = at > 0 && b[at - 1] == b'r' && (at < 2 || !ident(b[at - 2]) || b[at - 2] == b'b');
                let mut j = i + 1;
                while j < b.len() {
                    if raw && b[j] == b'"' && b[j + 1..].iter().take_while(|c| **c == b'#').count() >= hashes {
                        break;
                    } else if !raw && b[j] == b'\\' {
                        j += 1;
                    } else if !raw && b[j] == b'"' {
                        break;
                    }
                    j += 1;
                }
                let text = &source[i + 1..j.min(b.len())];
                out.push((line, text.to_string()));
                line += text.matches('\n').count();
                i = j + 1 + if raw { hashes } else { 0 };
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

fn rust_files_below(path: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())) {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            rust_files_below(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn one_system_no_harness_or_host_names_in_core() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let names = banned(&root);
    let mut files = Vec::new();
    for dir in ["crates/goalcup/src", "crates/goalcup-format/src"] {
        rust_files_below(&root.join(dir), &mut files);
    }
    files.sort();
    let (mut hits, mut used) = (Vec::new(), vec![false; ALLOWED.len()]);
    for path in &files {
        let file = path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        let source = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{file}: {e}"));
        for (line, literal) in string_literals(&source) {
            let lower = literal.to_ascii_lowercase();
            if !names.iter().any(|name| names_token(&lower, name)) {
                continue;
            }
            match ALLOWED.iter().position(|(f, text, _)| *f == file && literal.contains(text)) {
                Some(n) => used[n] = true,
                None => hits.push(format!("{file}:{line}: {literal:?}")),
            }
        }
    }
    assert!(hits.is_empty(), "harness and host names are connector data, not core code:\n{}", hits.join("\n"));
    let stale: Vec<_> = ALLOWED
        .iter()
        .zip(&used)
        // A whole-file entry whose file is absent waits for its file (PRD-11 lands builtin.rs).
        .filter(|((file, text, _), used)| !**used && (!text.is_empty() || root.join(file).exists()))
        .map(|((file, text, _), _)| format!("{file}: {text:?}"))
        .collect();
    assert!(stale.is_empty(), "ALLOWED entries that excuse nothing any more:\n{}", stale.join("\n"));
}
