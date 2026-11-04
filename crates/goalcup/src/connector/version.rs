//! One harness version parser: `parse` reads a `--version` line, `matches` checks it against a
//! connector's version spec. The grammar is pinned in
//! docs/superpowers/plans/2026-11-04-v1-fixes/F-version-core.md; scripts/pad/harness_version.py
//! is its twin.

use std::cmp::Ordering;

/// A version read from a `--version` line: the word as printed (suffix kept, `v` dropped) and its
/// dotted numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessVersion {
    pub text: String,
    pub nums: Vec<u64>,
}

/// The first word of `line` shaped like `1.2.3`, with an optional `-suffix` or `+suffix`.
pub fn parse(line: &str) -> Option<HarnessVersion> {
    line.split_whitespace().find_map(|word| {
        let word = word.trim_start_matches('(');
        let word = word.trim_end_matches(['.', ',', ';', ':', ')']);
        let word = word.strip_prefix('v').unwrap_or(word);
        let (core, suffix) = match word.find(['-', '+']) {
            Some(i) => (&word[..i], Some(&word[i + 1..])),
            None => (word, None),
        };
        let suffix_ok = suffix.is_none_or(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        });
        let nums = numbers(core)?;
        (suffix_ok && nums.len() >= 2).then(|| HarnessVersion {
            text: word.to_string(),
            nums,
        })
    })
}

/// Dotted ASCII numbers, each fitting u64; `None` for anything else.
fn numbers(s: &str) -> Option<Vec<u64>> {
    s.split('.')
        .map(|n| {
            n.bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| n.parse().ok())
                .flatten()
        })
        .collect()
}

/// One `,`-separated part of a spec: its op (`""` for a prefix) and its numbers.
fn spec_part(part: &str) -> Option<(&'static str, Vec<u64>)> {
    let (op, value) = [">=", "<=", ">", "<", "="]
        .iter()
        .find_map(|op| part.strip_prefix(op).map(|rest| (*op, rest)))
        .unwrap_or(("", part));
    let value = value
        .strip_suffix('x')
        .unwrap_or(value)
        .trim_end_matches('.');
    numbers(value).map(|want| (op, want))
}

/// Whether every part of `spec` reads, so it can hold for some version.
pub fn spec_ok(spec: &str) -> bool {
    spec.split(',').all(|part| spec_part(part).is_some())
}

/// Whether `v` holds every `,`-separated part of `spec`: `>=1.2`, `<2`, `=1.0.105`, or a bare
/// prefix such as `1.`, `1.x` or `0.22.6`. An empty spec or an unreadable part never holds.
pub fn matches(v: &HarnessVersion, spec: &str) -> bool {
    spec.split(',').all(|part| {
        let Some((op, want)) = spec_part(part) else {
            return false;
        };
        let order = (0..v.nums.len().max(want.len()))
            .map(|i| v.nums.get(i).unwrap_or(&0).cmp(want.get(i).unwrap_or(&0)))
            .find(|o| o.is_ne())
            .unwrap_or(Ordering::Equal);
        match op {
            ">=" => order.is_ge(),
            "<=" => order.is_le(),
            ">" => order.is_gt(),
            "<" => order.is_lt(),
            "=" => order.is_eq(),
            _ => v.nums.starts_with(&want),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_lines_parse() {
        let rows = include_str!("../../tests/fixtures/version-lines.tsv");
        for row in rows.lines().filter(|l| !l.starts_with('#')) {
            let cols: Vec<&str> = row.split('\t').collect();
            let (raw, want) = (cols[1], cols[2]);
            if want == "PLACEHOLDER" {
                continue;
            }
            let want = (want != "-").then(|| want.to_string());
            assert_eq!(parse(raw).map(|v| v.text), want, "{row}");
        }
        let v = parse("Fakev Agent v0.22.6 (2026.9.24)").unwrap();
        assert_eq!(v.nums, vec![0, 22, 6]);
        assert_eq!(parse("(8b18b55)"), None);
        let amp = parse("fakev 0.0.1-gabc123").unwrap();
        assert_eq!(
            (amp.text.as_str(), amp.nums),
            ("0.0.1-gabc123", vec![0, 0, 1])
        );
    }

    #[test]
    fn specs_match() {
        let v = |s| parse(s).unwrap();
        assert!(matches(&v("2.2.2"), "2."));
        assert!(!matches(&v("10.0.0"), "1."));
        assert!(!matches(&v("1.19.5"), "2."));
        assert!(matches(&v("1.19.5"), "1.x"));
        assert!(matches(&v("0.22.6"), "0.22.6"));
        assert!(!matches(&v("0.22.60"), "0.22.6"));
        assert!(matches(&v("0.22.6"), "0.22"));
        assert!(matches(&v("0.22.5"), ">=0.22.5,<0.23"));
        assert!(!matches(&v("0.23.0"), ">=0.22.5,<0.23"));
        assert!(matches(&v("1.0.105"), "=1.0.105"));
        assert!(!matches(&v("1.0.0"), ""));
        assert!(!matches(&v("1.0.0"), ">=x"));
    }

    #[test]
    fn probe_reads_stderr_and_a_named_version_on_a_failed_exit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fakev");
        let probe = |body: &str| {
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            crate::connector::launch::harness_version(&path, &std::collections::BTreeMap::new())
        };
        let some = |s: &str| Some(s.to_string());
        assert_eq!(probe("echo fakev 1.2.3"), some("fakev 1.2.3"));
        assert_eq!(probe("echo fakev 2.0.0 >&2"), some("fakev 2.0.0"));
        assert_eq!(
            probe("echo 'Fakev Agent v0.22.6 (2026.9.24)' >&2; exit 1"),
            some("Fakev Agent v0.22.6 (2026.9.24)")
        );
        assert_eq!(probe("echo 'error: needs Python 3.11.4' >&2; exit 1"), None);
        assert_eq!(probe("echo fakev 1.2.3; exit 1"), some("fakev 1.2.3"));
        // Cut at a char boundary to 64 UTF-16 units.
        let long = format!("fakev 1.2.3 {}", "é".repeat(5000));
        let cut = crate::connector::launch::cut_version(&long);
        assert_eq!(cut.encode_utf16().count(), 64);
        assert!(cut.starts_with("fakev 1.2.3 "));
        assert_eq!(probe("echo warning: no config >&2"), None);
        assert_eq!(probe("echo 'warning: node 2.0.1 deprecated' >&2"), None);
        assert_eq!(probe("echo 'fakev: requires bun 2.1.0' >&2; exit 1"), None);
    }
}
