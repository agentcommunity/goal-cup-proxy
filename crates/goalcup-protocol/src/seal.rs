//! The workspace manifest recorded at `deadline_at` (D11).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedFile {
    /// Workspace-relative path, forward slashes.
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealManifest {
    pub entry_id: String,
    pub sealed_at: String,
    pub workspace: String,
    pub files: Vec<SealedFile>,
    /// Paths excluded by the connector's `workspace_exclusions[]` plus tracker config.
    pub excluded: Vec<String>,
}

impl SealManifest {
    /// Deterministic ordering: paths sort lexicographically before hashing or publishing.
    pub fn sorted(mut self) -> SealManifest {
        self.files.sort_by(|a, b| a.path.cmp(&b.path));
        self.excluded.sort();
        self
    }
}

/// **The single definition of the seal-exclusion rule.**
///
/// Every caller uses it and there must never be a second implementation:
///
/// - `goalcup-verifier/src/attribution.rs` — deciding which sealed files need an
///   explanation;
/// - `goalcup-daemon/src/workspace.rs` and `src/submission.rs` — deciding what the seal walk
///   lists in the first place;
/// - `goalcup-cli/src/seal.rs` — the CLI's mirror self-check.
///
/// Implementations with different semantics would exclude `a/b/.git/HEAD` in one place and
/// *attribute* it in another.
///
/// **Workspace-root-anchored.** A pattern matches `rel` when it equals
/// `rel`, or is a leading path prefix (`rel` starts with `p` + `/`). A pattern beginning
/// `**/` opts into any-depth matching on the remainder. Nothing else matches at depth.
///
/// A pattern never matches "any single path component of `rel`": that clause would be an
/// attack, since `mkdir -p src/.git && cp -r prewritten/* src/.git/` would hide a whole
/// subtree from the seal **and** from attribution, because every connector lists `.git`. A
/// nested `.git` is the agent's work until a pattern says otherwise, and a connector that
/// wants that reach writes `**/.git` and means it.
///
/// `rel` is always workspace-relative with forward slashes; the comparison is never a
/// substring test, so `targeted.py` is not excluded by the pattern `target`.
pub fn is_excluded(rel: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| match p.strip_prefix("**/") {
        // Any-depth: the remainder may start at any path component of `rel`.
        Some(rest) => {
            !rest.is_empty()
                && component_starts(rel).any(|from| anchored_at_root(&rel[from..], rest))
        }
        None => !p.is_empty() && anchored_at_root(rel, p),
    })
}

/// `rel` is `p` itself or lies beneath it. Never a substring test: the byte after `p`
/// must be the separator, so `target` does not match `targeted.py`.
fn anchored_at_root(rel: &str, p: &str) -> bool {
    rel == p
        || rel
            .strip_prefix(p)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Byte offsets in `rel` at which a path component begins: 0, and every index after a
/// `/`. Used only by the `**/` opt-in.
fn component_starts(rel: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(rel.match_indices('/').map(|(i, _)| i + 1))
}
