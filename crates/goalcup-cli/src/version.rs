//! D12's version grammar: two operators, and no more.
//!
//! Every shipped connector declares its `tested_version_range` in one of two forms —
//! `">=2.1.259"` on Claude Code, `"=0.153.0"` on Codex — so
//! two operators are the whole language. A `semver` dependency would buy a grammar this
//! tree has decided not to use: pre-release ordering, caret and tilde ranges, build
//! metadata, and the question of what `^0.153.0` means, none of which any manifest can
//! express and all of which would then have to be specified.
//!
//! A range therefore never names a pre-release, and a harness that reports one is out of
//! every range that stops at its own numbers — see [`VersionRange::matches`].
//!
//! **Comparison is numeric, per dotted component.** That is the whole of why this is not a
//! string comparison: `"2.1.9" >= "2.1.259"` is true of the strings and false of the
//! versions, and Claude's range is exactly the case where it matters — 2.1.259 is the build
//! that fixed resume-after-kill, and a participant on 2.1.9 must not be told they are
//! covered by it.
//!
//! Running the harness to find out which version it is on is preflight's
//! (`sample_harness_version`). What is here is the grammar and the comparison:
//! [`extract_version`] takes a version out of whatever the harness printed, and
//! [`VersionRange::matches`] answers the range against it.
//!
//! **D12: a range is not a pin.** Its lower bound is a floor, and
//! a version above an `=` range is newer than tested — allowed, and reported as untested.
//! [`VersionRange::standing`] is that three-way answer; preflight reads it, not `matches`.

use crate::CliError;
use std::cmp::Ordering;

/// The version on the first non-empty line of `output`, or `None` if there is not one there.
///
/// A version is a run of digits with at least one dotted numeric component after it —
/// `\d+(\.\d+)+`, the first one on the line — **with the pre-release or build tag it carries,
/// if it carries one**. The dot is required because the harnesses print their version
/// inside a sentence — `codex-cli 0.153.0 (rust)` — and a bare integer in such a line is far
/// more often a build number, a year or a count than the version.
///
/// # The first line, not the first number anywhere
///
/// A harness that prints an update notice or a deprecation warning before its version would
/// otherwise have the notice's numbers read as the version.
/// `preflight::sample_harness_version` already narrows to one line for that reason, and a
/// parser that disagreed with the probe about what it had been handed would be a second
/// answer to the same question.
///
/// # The tag is kept
///
/// `1.2.3-beta` yields `"1.2.3-beta"`, not `"1.2.3"`. Dropping it would let a pre-release answer
/// for the release it is not — `=1.2.3` would match `1.2.3-beta`, a build whose tested behaviour
/// has not shipped. [`VersionRange::matches`] is where a tag is read against a range.
pub fn extract_version(output: &str) -> Option<String> {
    let line = output.lines().map(str::trim).find(|l| !l.is_empty())?;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i;
        let mut dotted = 0;
        loop {
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            // Only a dot with a digit after it continues the version. A trailing `1.` is
            // the end of a sentence, not a component.
            if end + 1 < bytes.len() && bytes[end] == b'.' && bytes[end + 1].is_ascii_digit() {
                dotted += 1;
                end += 1;
            } else {
                break;
            }
        }
        if dotted >= 1 {
            // Both indices sit on ASCII bytes — `start` on a digit, `end` one past a digit or
            // one past a tag character — so they are character boundaries whatever else is on
            // the line.
            return line.get(start..tag_end(bytes, end)).map(str::to_string);
        }
        // `end` is past the digit run that was not a version; never re-examine it.
        i = end.max(i + 1);
    }
    None
}

/// Where the version that ends at `end` really ends, once its tag is counted.
///
/// A tag is a `-` or a `+` immediately after the last numeric component, followed by at least
/// one of the characters SemVer allows in one (ASCII alphanumerics, `.`, `-`, `+`). Anything
/// else — `2.1.259 (Claude Code)`, `1.18.27, built...` — is the sentence around the version
/// and stops it.
fn tag_end(bytes: &[u8], end: usize) -> usize {
    if end >= bytes.len() || (bytes[end] != b'-' && bytes[end] != b'+') {
        return end;
    }
    let mut i = end + 1;
    while i < bytes.len()
        && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'.' | b'-' | b'+'))
    {
        i += 1;
    }
    // A bare `-` with nothing after it is a dash in a sentence, not a tag.
    if i == end + 1 { end } else { i }
}

/// The two operators, and there is deliberately no third.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RangeOp {
    /// `>=`, the form Claude's range takes.
    AtLeast,
    /// `=`, the form Codex's takes.
    Exactly,
}

impl RangeOp {
    fn as_str(self) -> &'static str {
        match self {
            RangeOp::AtLeast => ">=",
            RangeOp::Exactly => "=",
        }
    }
}

/// A connector's `tested_version_range`, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRange {
    op: RangeOp,
    parts: Vec<u64>,
}

impl VersionRange {
    /// `">=2.1.259"` or `"=0.153.0"`, and nothing else.
    ///
    /// A range this build cannot read is [`CliError::Internal`] — rung 3 — because it is a
    /// document that could not be parsed and not something a participant does differently
    /// next time. The message quotes what was written and says what the two forms are, so
    /// whoever wrote the manifest can see which half is wrong.
    pub fn parse(s: &str) -> Result<VersionRange, CliError> {
        let trimmed = s.trim();
        // `>=` first: `=` is a prefix of nothing here, but the reverse order would read
        // `>=2.1.259` as an operator-less range starting with `>`.
        let (op, rest) = if let Some(rest) = trimmed.strip_prefix(">=") {
            (RangeOp::AtLeast, rest)
        } else if let Some(rest) = trimmed.strip_prefix('=') {
            (RangeOp::Exactly, rest)
        } else {
            return Err(unreadable(s));
        };
        let parts = numeric_parts(rest.trim()).ok_or_else(|| unreadable(s))?;
        Ok(VersionRange { op, parts })
    }

    /// Whether `version` is in this range.
    ///
    /// `version` may be a bare version (`"2.1.259"`) or the whole line a harness printed
    /// (`"codex-cli 0.153.0 (rust)"`): the bare form is read first and [`extract_version`]
    /// answers for everything else, so preflight can hand over what the probe captured
    /// without deciding in a second place what counts as a version.
    ///
    /// A string with no version in it is not in any range. It is never an error here: the
    /// caller that could not read a harness's version has a better message for it than a
    /// range does.
    ///
    /// # A tagged version does not satisfy a range that does not name the tag
    ///
    /// `1.2.3-beta` is not `1.2.3`: it is a build made before it, whose behaviour D12 did not
    /// test. So a tagged version answers `false` to `=1.2.3`, and `false` to `>=1.2.3` as
    /// well — at the bound itself it sits **below** the release, exactly as SemVer orders it.
    /// Above the bound (`2.2.0-rc1` against `>=2.1.259`) it is in range, because there the
    /// numbers already decide it.
    ///
    /// **No range can name a tag**, because D12's grammar writes a version as dotted numbers
    /// and nothing else (see [`VersionRange::parse`]). So the exception in that heading is
    /// one this build cannot currently reach, and it is written down as the thing that would
    /// have to change first if a manifest ever needed to pin a pre-release.
    pub fn matches(&self, version: &str) -> bool {
        let Some((parts, tag)) = split_version(version.trim())
            .or_else(|| extract_version(version).as_deref().and_then(split_version))
        else {
            return false;
        };
        let tagged = tag.is_some();
        match (self.op, compare(&parts, &self.parts)) {
            (RangeOp::Exactly, Ordering::Equal) => !tagged,
            (RangeOp::Exactly, _) => false,
            (RangeOp::AtLeast, Ordering::Greater) => true,
            (RangeOp::AtLeast, Ordering::Equal) => !tagged,
            (RangeOp::AtLeast, Ordering::Less) => false,
        }
    }
}

/// Where a measured version stands against a connector's range (D12).
///
/// The range answers three ways. Its lower bound is a floor: below it the harness is refused,
/// because a known-broken older build is still a reason to refuse. Inside it the harness is
/// what was tested. Above it — only an `=` range has an above — the harness is newer than
/// anything tested: allowed, and recorded as untested rather than refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Inside the range: the version, or a version, that was tested.
    Tested,
    /// Above the range: newer than what was tested. Allowed; reported as untested.
    NewerThanTested,
    /// Below the range's floor, or not a version at all. Refused.
    BelowFloor,
}

impl VersionRange {
    /// Where `version` stands against this range; see [`Standing`].
    ///
    /// Read the way [`VersionRange::matches`] reads it: a bare version or a whole banner,
    /// and a pre-release at the bound sits below it. A pre-release above an `=` bound
    /// (`0.154.0-alpha` against `=0.153.0`) is newer than tested — the numbers decide it.
    /// A string with no version in it is [`Standing::BelowFloor`]: nothing shows it is at
    /// or above the floor, and the caller that could not read a version has its own message.
    pub fn standing(&self, version: &str) -> Standing {
        if self.matches(version) {
            return Standing::Tested;
        }
        let Some((parts, _)) = split_version(version.trim())
            .or_else(|| extract_version(version).as_deref().and_then(split_version))
        else {
            return Standing::BelowFloor;
        };
        match (self.op, compare(&parts, &self.parts)) {
            (RangeOp::Exactly, Ordering::Greater) => Standing::NewerThanTested,
            _ => Standing::BelowFloor,
        }
    }

    /// The floor as a bare version, `"0.153.0"` for both `=0.153.0` and `>=0.153.0`.
    pub fn floor(&self) -> String {
        self.parts
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(".")
    }
}

impl std::fmt::Display for VersionRange {
    /// Back in the form it was written in, so a preflight check can quote the range it
    /// refused against without carrying the manifest's string alongside the parsed one.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.op.as_str())?;
        for (i, part) in self.parts.iter().enumerate() {
            if i > 0 {
                f.write_str(".")?;
            }
            write!(f, "{part}")?;
        }
        Ok(())
    }
}

/// `"2.1.259"` → `[2, 1, 259]`, or `None` if it is not a dotted run of numbers.
///
/// Every component must be there: `"2..1"`, `"2.x"`, `"v2.1"` and `""` are all `None`,
/// because a range or a version this function had to guess at is a range or a version whose
/// meaning is not written down.
fn numeric_parts(s: &str) -> Option<Vec<u64>> {
    if s.is_empty() {
        return None;
    }
    s.split('.')
        .map(|part| {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse::<u64>().ok()
        })
        .collect()
}

/// `"1.2.3-beta"` → `([1, 2, 3], Some("beta"))`, `"1.2.3"` → `([1, 2, 3], None)`.
///
/// The split is at the first `-` or `+`, which is where SemVer puts the pre-release and the
/// build tag both. They are not told apart here: what [`VersionRange::matches`] does with a
/// tag it does with either, since a range can name neither.
fn split_version(s: &str) -> Option<(Vec<u64>, Option<String>)> {
    let (numbers, tag) = match s.find(['-', '+']) {
        Some(at) => (&s[..at], Some(s[at + 1..].to_string())),
        None => (s, None),
    };
    // An empty tag is no tag: `"1.2.3-"` is a version with a dash after it, and reading the
    // dash as a pre-release would refuse a range the numbers satisfy.
    let tag = tag.filter(|t| !t.is_empty());
    numeric_parts(numbers).map(|parts| (parts, tag))
}

/// Component by component, numerically, with the shorter side padded with zeros.
///
/// Padding rather than "shorter is smaller", so `2.1` and `2.1.0` are the same version —
/// which is what a participant means by both, and what keeps `=1.18` from refusing a
/// harness that prints `1.18.0`.
fn compare(left: &[u64], right: &[u64]) -> Ordering {
    let width = left.len().max(right.len());
    for i in 0..width {
        let a = left.get(i).copied().unwrap_or(0);
        let b = right.get(i).copied().unwrap_or(0);
        match a.cmp(&b) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

fn unreadable(written: &str) -> CliError {
    CliError::Internal(format!(
        "{written:?} is not a version range this build reads. A range is one of two \
         forms, `>=<version>` or `=<version>`, with a version written as dotted \
         numbers — `>=2.1.259`, `=0.153.0`."
    ))
}

/// The grammar, checked here rather than in `tests/` because `RangeOp`, `numeric_parts` and
/// [`compare`] are private and the cases worth pinning are theirs.
///
/// **No `unwrap`, `expect` or `panic!`.** The crate's rule is about `src/`, and a test
/// module inside `src/` is inside `src/`: a `Result` is turned into a comparable value and
/// asserted on, exactly as `config.rs`'s own tests do.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_is_dotted_numbers_and_a_bare_integer_is_not_one() {
        assert_eq!(
            extract_version("2.1.259 (Claude Code)").as_deref(),
            Some("2.1.259")
        );
        assert_eq!(
            extract_version("codex-cli 0.153.0 (rust)").as_deref(),
            Some("0.153.0")
        );
        assert_eq!(
            extract_version("opencode 1.18.27").as_deref(),
            Some("1.18.27")
        );
        // The dot is required, and a run of digits without one is stepped over rather than
        // returned: `cli2` is not a version and `0.153.0` after it is.
        assert_eq!(
            extract_version("codex-cli2 build 7: 0.153.0").as_deref(),
            Some("0.153.0")
        );
        assert_eq!(
            extract_version("1.2.3-beta").as_deref(),
            Some("1.2.3-beta"),
            "the tag is kept: a pre-release is not the release"
        );
        assert_eq!(
            extract_version("opencode 1.18.27+build.7 (linux)").as_deref(),
            Some("1.18.27+build.7")
        );
        assert_eq!(
            extract_version("2.1.259 (Claude Code) - stable").as_deref(),
            Some("2.1.259"),
            "a dash in the sentence after it is not a tag"
        );
        assert_eq!(
            extract_version("ends here: 1."),
            None,
            "a trailing dot is punctuation"
        );
        // The first non-empty line and not the first number anywhere: an update notice above
        // the version must not answer for it. `preflight::sample_harness_version` narrows the
        // same way, and this is the parser agreeing with the probe.
        assert_eq!(
            extract_version("\n  update available: 9.9.9\ncodex-cli 0.153.0\n").as_deref(),
            Some("9.9.9"),
            "the first non-empty line is the one read"
        );
        assert_eq!(
            extract_version("no version on this line\n0.153.0"),
            None,
            "and a later line is not reached"
        );
        assert_eq!(extract_version("no numbers here"), None);
        assert_eq!(extract_version(""), None);
        // A multi-byte character is walked over a byte at a time and never sliced into.
        assert_eq!(
            extract_version("verzió 1.18.27 — kész").as_deref(),
            Some("1.18.27")
        );
    }

    #[test]
    fn the_grammar_is_two_operators_and_a_range_outside_it_is_rung_three() {
        for outside in [
            "^1.2", "~1.2", ">1.2", "<=1.2", "1.2", "", "=", ">=", "=v1.2", "=1.x",
        ] {
            let (code, said) = match VersionRange::parse(outside) {
                Ok(r) => (0, r.to_string()),
                Err(e) => (e.exit_code(), e.to_string()),
            };
            assert_eq!(
                code, 3,
                "{outside:?} is not D12's grammar and this build read it as {said:?}"
            );
            assert!(
                said.contains(">=2.1.259"),
                "the refusal shows the two forms: {said:?}"
            );
        }
        assert_eq!(
            VersionRange::parse(" >= 2.1.259 ")
                .map(|r| r.to_string())
                .ok(),
            Some(">=2.1.259".to_string()),
            "surrounding and internal whitespace is not part of the range"
        );
    }

    #[test]
    fn comparison_is_numeric_per_component_and_a_missing_component_is_zero() {
        assert_eq!(
            compare(&[2, 1, 9], &[2, 1, 259]),
            Ordering::Less,
            "9 is not above 259"
        );
        assert_eq!(
            compare(&[2, 1], &[2, 1, 0]),
            Ordering::Equal,
            "2.1 is 2.1.0"
        );
        assert_eq!(compare(&[2, 1, 1], &[2, 1]), Ordering::Greater);
        // A component wider than any harness would print, so the parse is not a `u32`.
        assert_eq!(
            numeric_parts("0.0.4294967296"),
            Some(vec![0, 0, 4_294_967_296])
        );
        assert_eq!(numeric_parts("2..1"), None);
        assert_eq!(numeric_parts("2.-1"), None);
        assert_eq!(
            numeric_parts("2.1 "),
            None,
            "the caller trims; this one does not guess"
        );
    }

    #[test]
    fn matches_reads_a_bare_version_and_a_whole_banner_the_same_way() {
        // Asked through the `Result` rather than unwrapped, so this module keeps the
        // crate's no-panic rule: a range that failed to parse answers `None` here and the
        // assertion fails on that instead of on a panic.
        let asked = |version: &str| VersionRange::parse(">=2.1.259").map(|r| r.matches(version));
        for (version, want) in [
            ("2.1.259", true),
            ("2.1.259 (Claude Code)", true),
            ("2.1.9 (Claude Code)", false),
            ("what version?", false),
        ] {
            assert_eq!(
                asked(version).ok(),
                Some(want),
                ">=2.1.259 against {version:?}, which a harness could print either way"
            );
        }
    }

    /// D12: newer than tested is allowed and untested, below the
    /// floor is refused, and an `>=` range has no "newer than tested".
    #[test]
    fn standing_is_tested_newer_or_below_the_floor() {
        let at =
            |range: &str, version: &str| VersionRange::parse(range).map(|r| r.standing(version));
        for (range, version, want) in [
            ("=0.153.0", "0.153.0", Standing::Tested),
            (
                "=0.153.0",
                "codex-cli 0.154.0 (rust)",
                Standing::NewerThanTested,
            ),
            ("=0.153.0", "0.200.1", Standing::NewerThanTested),
            ("=0.153.0", "0.154.0-alpha", Standing::NewerThanTested),
            ("=0.153.0", "0.153.0-beta", Standing::BelowFloor),
            ("=0.153.0", "0.143.0", Standing::BelowFloor),
            ("=0.153.0", "no version", Standing::BelowFloor),
            (">=2.1.259", "2.3.0", Standing::Tested),
            (">=2.1.259", "2.1.9", Standing::BelowFloor),
        ] {
            assert_eq!(
                at(range, version).ok(),
                Some(want),
                "{range} against {version:?}"
            );
        }
        assert_eq!(
            VersionRange::parse("=0.153.0")
                .map(|r| r.floor())
                .ok()
                .as_deref(),
            Some("0.153.0")
        );
    }

    /// D12 tested a release, and a pre-release of it is a build made before that release.
    ///
    /// `=` and the bound of `>=` both refuse a tagged version; above the bound the numbers
    /// already decide it. No range in this grammar can name a tag, so there is no version of
    /// this test where a tagged build is admitted by naming it.
    #[test]
    fn a_pre_release_does_not_satisfy_a_range_that_cannot_name_it() {
        let against =
            |range: &str, version: &str| VersionRange::parse(range).map(|r| r.matches(version));
        for (range, version, want) in [
            ("=1.2.3", "1.2.3", true),
            ("=1.2.3", "1.2.3-beta", false),
            ("=1.2.3", "1.2.3+build.7", false),
            (">=2.1.259", "2.1.259", true),
            (">=2.1.259", "2.1.259-rc1", false),
            (">=2.1.259", "2.2.0-rc1", true),
            (">=2.1.259", "2.1.258-rc1", false),
            // A dash with nothing after it is punctuation, and the numbers answer.
            ("=1.2.3", "1.2.3-", true),
        ] {
            assert_eq!(
                against(range, version).ok(),
                Some(want),
                "{range} against {version:?}"
            );
        }
        assert_eq!(
            split_version("1.2.3-beta.1"),
            Some((vec![1, 2, 3], Some("beta.1".to_string())))
        );
        assert_eq!(split_version("1.2.3"), Some((vec![1, 2, 3], None)));
        assert_eq!(
            split_version("-1.2"),
            None,
            "a version starts with a number"
        );
    }
}
