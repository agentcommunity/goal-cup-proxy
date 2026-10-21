//! No harness or product name in core code: a harness is connector data plus shared modules.
//!
//! Every source file of the protocol, daemon and verifier crates is scanned for string literals
//! that contain a harness name. The capture paths of the CLI and platform crates are held to the
//! narrower rule they had before: no literal that is exactly a family name. Comments and
//! identifiers are not scanned. A literal where a name is data is listed in `ALLOWED`, by file
//! and exact text, with its reason; any other hit in the same file still fails.
//!
//! The ban grows with `connectors/`: every connector id, without a trailing `-cli`, is banned
//! too, as a whole token (bounded by anything but `[a-z0-9]`), so a short id such as `amp`
//! does not hit `timestamp`. Harness #4 is covered the day its folder lands.
//!
//! `openai` and `anthropic` may appear in core when they name an API wire format or a provider
//! (decoder family ids, secret shapes, provider prefixes); the ban is on harness and product
//! names (codex, claude-code, hermes, …).

use goalcup_protocol::pins::shipped_connector_ids;
use std::path::{Path, PathBuf};

/// Lower-case fragments no string literal may contain, compared case-insensitively. They cover
/// the connector ids (`claude-code-cli`, `codex-cli`, `hermes-cli`), the native formats
/// (`codex-jsonl`, `claude-code-jsonl`) and the vendor CA variables (`CODEX_CA_CERTIFICATE`,
/// `HERMES_CA_BUNDLE`), which are connector data (`capture.ca.env`).
const FORBIDDEN: &[&str] = &["hermes", "codex", "claude"];

/// The literals the CLI and platform capture paths may not be.
const FAMILY_LITERALS: &[&str] = &["hermes", "codex", "claude-code"];

/// String literals whose harness name is data: (file relative to the repository root, the
/// literal's text with line continuations joined, reason). Any other hit in the same file
/// fails, and an entry no literal matches any more fails as stale.
const ALLOWED: &[(&str, &str, &str)] = &[
    (
        "crates/goalcup-protocol/src/capture_policy.rs",
        "CLAUDE_CODE_SHELL_PREFIX",
        "a refused launch variable: it makes the harness run a command",
    ),
    (
        "crates/goalcup-protocol/src/capture_policy.rs",
        "CLAUDE_ENV_FILE",
        "a refused launch variable: it makes the harness load a file",
    ),
    (
        "crates/goalcup-protocol/src/capture_policy.rs",
        "CLAUDE_CODE_USE_",
        "a refused launch prefix: it switches the harness to another model provider",
    ),
    (
        "crates/goalcup-verifier/src/capture_analysis/native/codex.rs",
        "codex-jsonl",
        "the protocol_hint a connector declares for this native decoder",
    ),
    (
        "crates/goalcup-verifier/src/capture_analysis/native/claude_code.rs",
        "claude-code-jsonl",
        "the protocol_hint a connector declares for this native decoder",
    ),
    (
        "crates/goalcup-verifier/src/capture_analysis/native/codex.rs",
        "codex-native",
        "the decoder id a report prints",
    ),
    (
        "crates/goalcup-verifier/src/capture_analysis/native/claude_code.rs",
        "claude-code-native",
        "the decoder id a report prints",
    ),
    (
        "crates/goalcup-verifier/src/capture_analysis/thread.rs",
        "amp-thread-jsonrpc",
        "the retained WebSocket format hint and independent decoder id, not harness policy",
    ),
    (
        "crates/goalcup-verifier/src/capture_analysis/registry.rs",
        "Codex CLI 0.153.0 CLI/exec rollouts and 0.156.1 native child with inherited context",
        "evidence note: the harness build this decoder was measured against",
    ),
    (
        "crates/goalcup-verifier/src/capture_analysis/registry.rs",
        "Claude Code project JSONL: 2.1.259 sessions and real 2.1.283 chains",
        "evidence note: the harness build this decoder was measured against",
    ),
    (
        "crates/goalcup-verifier/src/capture_analysis/registry.rs",
        "the WebSocket traffic of one live Codex CLI 0.153.0 run",
        "evidence note: the harness build this decoder was measured against",
    ),
    (
        "crates/goalcup-verifier/src/capture_analysis/registry.rs",
        "one real Claude Code 2.1.283 exchange",
        "evidence note: the harness build this decoder was measured against",
    ),
    (
        "crates/goalcup-verifier/src/capture_analysis/registry.rs",
        "one retrieved/replayed Hermes HTTP sample: request context, three SSE replies and one JSON reply",
        "evidence note: the harness build this decoder was measured against",
    ),
    (
        "crates/goalcup-verifier/src/capture_analysis/registry.rs",
        "one retrieved/replayed macOS Amp 0.0.1792209635-g24a9fe thread WebSocket sample",
        "evidence note: the harness build this decoder was measured against",
    ),
];

/// The name each shipped connector adds to the ban: its id without a trailing `-cli`.
fn connector_names() -> Vec<String> {
    shipped_connector_ids()
        .iter()
        .map(|id| id.strip_suffix("-cli").unwrap_or(id).to_string())
        .collect()
}

/// Whether `lower` holds `name` as a whole token: not preceded or followed by `[a-z0-9]`.
fn names_token(lower: &str, name: &str) -> bool {
    let token = |c: Option<u8>| c.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    lower.match_indices(name).any(|(at, _)| {
        !token(lower[..at].bytes().next_back()) && !token(lower[at + name.len()..].bytes().next())
    })
}

/// Whether a string literal names a harness: in a narrow capture path, by being exactly a family
/// or connector name; elsewhere in core, by holding a fixed fragment or a connector name as a token.
fn names_a_harness(literal: &str, narrow: bool, names: &[String]) -> bool {
    let lower = literal.to_ascii_lowercase();
    if narrow {
        FAMILY_LITERALS.contains(&literal) || names.contains(&lower)
    } else {
        FORBIDDEN.iter().any(|name| lower.contains(name))
            || names.iter().any(|name| names_token(&lower, name))
    }
}

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

/// A literal's text with each `\` line continuation and the indentation after it removed, as
/// the compiler reads it.
fn joined(literal: &str) -> String {
    let mut out = String::new();
    let mut rest = literal;
    while let Some(at) = rest.find("\\\n") {
        out.push_str(&rest[..at]);
        rest = rest[at + 2..].trim_start();
    }
    out.push_str(rest);
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
    assert_eq!(joined("one \\\n        two"), "one two");
}

#[test]
fn the_ban_holds_every_connector_name_as_a_whole_token() {
    // A literal naming any shipped connector fails the scan, in both kinds of path. `zz-new`
    // stands in for harness #4: its folder adds `zz-new` to the ban, and nothing else does.
    let mut names = connector_names();
    for name in &names {
        assert!(
            names_a_harness(&format!("run {name} now"), false, &names),
            "{name}"
        );
        assert!(names_a_harness(name, true, &names), "{name}");
    }
    assert!(!names_a_harness("run zz-new now", false, &names));
    names.push("zz-new".to_string());
    assert!(names_a_harness("run zz-new now", false, &names));
    assert!(names_a_harness("ZZ-NEW", true, &names));
    assert!(!names_a_harness("zz-newer", false, &names));
    for (text, hit) in [
        ("amp", true),
        ("run amp now", true),
        ("x-amp_y", true),
        ("amp/1.2", true),
        ("timestamp", false),
        ("example", false),
        ("amp2", false),
        ("2amp", false),
    ] {
        assert_eq!(names_token(text, "amp"), hit, "{text:?}");
    }
}

#[test]
fn core_code_names_no_harness_in_a_string_literal() {
    let root = repo_root();
    let names = connector_names();
    let capture_paths = [
        root.join("crates/goalcup-cli/src/launch.rs"),
        root.join("crates/goalcup-cli/src/per_run_home.rs"),
        root.join("crates/goalcup-cli/src/home.rs"),
        root.join("crates/goalcup-cli/src/commands/start/mod.rs"),
        root.join("crates/goalcup-cli/src/commands/start/finish.rs"),
        root.join("crates/goalcup-cli/src/commands/start/forked.rs"),
        root.join("crates/goalcup-cli/src/commands/start/phases.rs"),
        root.join("crates/goalcup-cli/src/commands/start/proxy.rs"),
        root.join("crates/goalcup-cli/src/commands/start/refusals.rs"),
        root.join("crates/goalcup-cli/src/commands/start/server.rs"),
        root.join("crates/goalcup-cli/src/commands/continue_run.rs"),
        root.join("crates/goalcup-cli/src/commands/prep.rs"),
        root.join("crates/goalcup-cli/src/host_log.rs"),
        root.join("crates/goalcup-cli/src/skill.rs"),
        root.join("crates/goalcup-platform/src/terminal.rs"),
        root.join("crates/goalcup-platform/src/machine.rs"),
        root.join("crates/goalcup-cli/src/folder_gate.rs"),
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
        let narrow = capture_paths.contains(path);
        let source =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for (line, literal) in string_literals(&source) {
            if !names_a_harness(&literal, narrow, &names) {
                continue;
            }
            let text = joined(&literal);
            match ALLOWED
                .iter()
                .position(|(file, allowed, _)| *file == relative && *allowed == text)
            {
                Some(n) => allowed_hit[n] = true,
                None => hits.push(format!("{relative}:{line}: {text:?}")),
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
        .map(|((file, literal, _), _)| format!("{file}: {literal:?}"))
        .collect();
    assert!(
        stale.is_empty(),
        "allowed literals no longer in their file:\n{}",
        stale.join("\n")
    );
}
