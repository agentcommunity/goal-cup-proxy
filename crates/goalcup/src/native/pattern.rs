//! Native row patterns (PRD-04 sections 2 and 7): `/`-separated components relative to a row's
//! root, each a name or a glob with `*` and `?`, or `**` (zero or more whole folders). No
//! I/O, no length or depth limit, no row cap: a walk cannot loop because no symlink is followed
//! and no mount is crossed (section 9).

use std::fmt::Write as _;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path};

/// A pattern refused at load. PRD-11's connector test calls [`parse`] as PRD-04's pattern check.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("native pattern refused: {0}")]
pub struct PatternError(pub &'static str);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    /// A name or a glob; a name with no `*` or `?` matches only itself.
    Glob(Vec<char>),
    AnyDepth,
}

/// A parsed row pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    parts: Vec<Part>,
}

/// Parses a row pattern, refusing an empty or absolute pattern, empty components, `.`, `..`,
/// braces, a NUL byte, `**` glued to other text and a trailing `**` (it names folders, not files).
pub fn parse(s: &str) -> Result<Pattern, PatternError> {
    if s.is_empty() {
        return Err(PatternError("empty pattern"));
    }
    if s.starts_with('/') {
        return Err(PatternError("absolute pattern"));
    }
    if s.contains(['{', '}', '\0']) {
        return Err(PatternError("braces or a NUL byte"));
    }
    if s == "**" || s.ends_with("/**") {
        return Err(PatternError("trailing `**`"));
    }
    let parts = s
        .split('/')
        .map(|c| match c {
            "" => Err(PatternError("empty component")),
            "." | ".." => Err(PatternError("`.` or `..` component")),
            "**" => Ok(Part::AnyDepth),
            c if c.contains("**") => Err(PatternError("`**` glued to other text")),
            c => Ok(Part::Glob(c.chars().collect())),
        })
        .collect::<Result<_, _>>()?;
    Ok(Pattern { parts })
}

impl Pattern {
    /// True when the file at `rel` (relative to the row's root) matches.
    pub fn matches(&self, rel: &Path) -> bool {
        !rel.as_os_str().is_empty() && self.states(rel).is_some_and(|s| s[self.parts.len()])
    }

    /// True when some file below the folder `rel_dir` could still match; always true once a `**`
    /// is reachable.
    pub fn may_enter(&self, rel_dir: &Path) -> bool {
        self.states(rel_dir)
            .is_some_and(|s| s[..self.parts.len()].iter().any(|&on| on))
    }

    /// The set of pattern positions reachable after consuming `rel`'s components. Position `i`
    /// means parts `..i` are matched; `parts.len()` means the whole pattern is.
    fn states(&self, rel: &Path) -> Option<Vec<bool>> {
        let n = self.parts.len();
        let mut set = vec![false; n + 1];
        set[0] = true;
        self.close(&mut set);
        for component in rel.components() {
            let Component::Normal(name) = component else {
                return None;
            };
            let name: Vec<char> = name.to_string_lossy().chars().collect();
            let mut next = vec![false; n + 1];
            for i in (0..n).filter(|&i| set[i]) {
                match &self.parts[i] {
                    Part::AnyDepth => next[i] = true,
                    Part::Glob(g) if glob_matches(&name, g) => next[i + 1] = true,
                    Part::Glob(_) => {}
                }
            }
            self.close(&mut next);
            set = next;
        }
        Some(set)
    }

    /// A `**` may match zero folders: whoever stands before it also stands after it.
    fn close(&self, set: &mut [bool]) {
        for (i, part) in self.parts.iter().enumerate() {
            if set[i] && *part == Part::AnyDepth {
                set[i + 1] = true;
            }
        }
    }
}

/// The first row, in order, whose root is a prefix of `abs` and whose pattern matches the rest;
/// returns its index and the path relative to its root. Rows may overlap; the first wins, so the
/// path ID is unique.
pub fn first_match<'a>(rows: &[(&Path, &Pattern)], abs: &'a Path) -> Option<(usize, &'a Path)> {
    rows.iter().enumerate().find_map(|(i, (root, pattern))| {
        let rel = abs.strip_prefix(root).ok()?;
        pattern.matches(rel).then_some((i, rel))
    })
}

/// The path ID `<row>:<rel>` (PRD-01 section 5), components joined with `/`. The root is never
/// part of it. Escaping as in 0.x: `\\` for a backslash, `\u{..}` for a control character,
/// `\x..` for each byte that is not UTF-8.
pub fn path_id(row: &str, rel: &Path) -> String {
    let mut out = format!("{row}:");
    for (index, name) in rel.iter().enumerate() {
        if index > 0 {
            out.push('/');
        }
        for chunk in name.as_bytes().utf8_chunks() {
            for character in chunk.valid().chars() {
                match character {
                    '\\' => out.push_str("\\\\"),
                    c if c.is_control() => {
                        let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
                    }
                    c => out.push(c),
                }
            }
            for byte in chunk.invalid() {
                let _ = write!(out, "\\x{byte:02x}");
            }
        }
    }
    out
}

/// One component: `*` any run (including none), `?` one character, anything else itself.
/// Copied from the 0.x single-component matcher.
fn glob_matches(name: &[char], pattern: &[char]) -> bool {
    let (mut i, mut j) = (0usize, 0usize);
    let (mut star, mut matched) = (None, 0usize);
    while i < name.len() {
        if j < pattern.len() && pattern[j] == '*' {
            star = Some(j);
            matched = i;
            j += 1;
        } else if j < pattern.len() && (pattern[j] == '?' || pattern[j] == name[i]) {
            i += 1;
            j += 1;
        } else if let Some(position) = star {
            j = position + 1;
            matched += 1;
            i = matched;
        } else {
            return false;
        }
    }
    while j < pattern.len() && pattern[j] == '*' {
        j += 1;
    }
    j == pattern.len()
}
