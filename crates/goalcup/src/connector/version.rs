//! One harness version parser: `parse` reads a `--version` line, `matches` checks it against a
//! connector's version spec. The grammar is pinned in
//! docs/superpowers/plans/2026-11-04-v1-fixes/F-version-core.md; scripts/pad/harness_version.py
//! is its twin.

use std::cmp::Ordering;

use super::builtin;

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
            (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                .then(|| n.parse().ok())
                .flatten()
        })
        .collect()
}

/// Whether `v` holds every `,`-separated part of `spec`: `>=1.2`, `<2`, `=1.0.105`, or a bare
/// prefix such as `1.`, `1.x` or `0.22.6`. An empty spec or an unreadable part never holds.
pub fn matches(v: &HarnessVersion, spec: &str) -> bool {
    spec.split(',').all(|part| {
        let part = part.trim();
        let (op, value) = [">=", "<=", ">", "<", "="]
            .iter()
            .find_map(|op| part.strip_prefix(op).map(|rest| (*op, rest)))
            .unwrap_or(("", part));
        let value = value
            .strip_suffix('x')
            .unwrap_or(value)
            .trim_end_matches('.');
        let Some(want) = numbers(value) else {
            return false;
        };
        let order = || {
            (0..v.nums.len().max(want.len()))
                .map(|i| v.nums.get(i).unwrap_or(&0).cmp(want.get(i).unwrap_or(&0)))
                .find(|o| o.is_ne())
                .unwrap_or(Ordering::Equal)
        };
        match op {
            ">=" => order().is_ge(),
            "<=" => order().is_le(),
            ">" => order().is_gt(),
            "<" => order().is_lt(),
            "=" => order().is_eq(),
            _ => v.nums.starts_with(&want),
        }
    })
}

/// The built-in connectors that run `program` and carry a `version` key, with that key.
pub fn same_program(program: &str) -> Vec<(&'static str, String)> {
    builtin::CONNECTORS
        .iter()
        .filter_map(|(id, _)| {
            let c = super::load(id, None).ok()?;
            (c.program == program).then_some((*id, c.version?))
        })
        .collect()
}

/// A spec as the participant reads it: `1.` and `1.x` show as `1.x`, anything else as written.
fn show(spec: &str) -> String {
    match spec.strip_suffix(".x").or(spec.strip_suffix('.')) {
        Some(nums) => format!("{nums}.x"),
        None => spec.to_string(),
    }
}

/// The note for a version read but not the one the connector was written for.
pub fn untested_say(program: &str, got: &str, want: &str) -> String {
    format!(
        "{program} {got} is not the version this connector was written for ({}). goalcup records it the same way; goalcup status says if model requests are missing.",
        show(want)
    )
}

/// The note for a version that could not be read, naming each other connector for `program`.
pub fn unchecked_say(program: &str, want: &str, others: &[(&'static str, String)]) -> String {
    let mut say = format!(
        "goalcup could not read {program} --version, so it could not check this is {program} {}.",
        show(want)
    );
    for (id, spec) in others {
        say += &format!(
            " If it is {}, run: goalcup setup --harness {id}",
            show(spec)
        );
    }
    say
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
        let v = parse("Hermes Agent v0.22.6 (2026.9.24)").unwrap();
        assert_eq!(v.nums, vec![0, 22, 6]);
        assert_eq!(parse("(8b18b55)"), None);
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
    fn opencode_ids_share_a_program() {
        assert_eq!(
            same_program("opencode"),
            vec![
                ("opencode-cli", "1.".into()),
                ("opencode2-cli", "2.".into())
            ]
        );
        assert!(same_program("claude").is_empty());
    }

    #[test]
    fn says_name_the_version_and_the_other_connector() {
        assert_eq!(
            untested_say("fakev", "1.9.0", "2."),
            "fakev 1.9.0 is not the version this connector was written for (2.x). goalcup records it the same way; goalcup status says if model requests are missing."
        );
        assert_eq!(
            unchecked_say("opencode", "1.", &[("opencode2-cli", "2.".into())]),
            "goalcup could not read opencode --version, so it could not check this is opencode 1.x. If it is 2.x, run: goalcup setup --harness opencode2-cli"
        );
        assert_eq!(
            unchecked_say("opencode", "1.", &[]),
            "goalcup could not read opencode --version, so it could not check this is opencode 1.x."
        );
    }
}
