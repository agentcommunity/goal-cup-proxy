//! The workspace manifest recorded at `deadline_at` (D11).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedFile {
    /// Workspace-relative path, forward slashes.
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// The Git state of a workspace that is the root of its own clone (D56).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceGit {
    /// The commit `HEAD` names, lowercase hex; `None` for an unborn `HEAD`.
    pub commit: Option<String>,
    /// SHA-256, hex, of what differs from that commit (D56 gives the framing).
    pub dirty_sha256: String,
}

/// What the baseline or the seal of a clone root records of its Git state (D56): the state, or
/// why Git could not read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GitState {
    /// Written as the [`WorkspaceGit`] object itself, so a manifest written before `Unreadable`
    /// existed keeps its bytes and its digest.
    Read(WorkspaceGit),
    /// `{"unreadable": "<why>"}`: a short sentence with no path and no file content. The clone
    /// is not walked instead, and the verifier reports the trace incomplete.
    Unreadable { unreadable: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealManifest {
    pub entry_id: String,
    pub sealed_at: String,
    pub workspace: String,
    /// Every file of a walked workspace. Empty when `git` is set: a clone is not walked.
    pub files: Vec<SealedFile>,
    /// Paths excluded by the connector's `workspace_exclusions[]` plus tracker config.
    pub excluded: Vec<String>,
    /// The commit and uncommitted digest of a workspace that is its own clone, or why they could
    /// not be read (D56).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<GitState>,
    /// The baseline only: the connector's config home before the run, without its native
    /// roots, at most 10 000 files; one over 64 MiB has its size and the absent digest (D56).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<Vec<SealedFile>>,
}

impl SealManifest {
    /// Deterministic ordering: paths sort lexicographically before hashing or publishing.
    pub fn sorted(mut self) -> SealManifest {
        self.files.sort_by(|a, b| a.path.cmp(&b.path));
        self.excluded.sort();
        if let Some(home) = &mut self.home {
            home.sort_by(|a, b| a.path.cmp(&b.path));
        }
        self
    }
}

/// **The single definition of the seal-exclusion rule.**
///
/// Its one caller is `goalcup-daemon/src/workspace.rs`, deciding what the seal walk lists and
/// what the baseline's harness-home listing leaves out. A later reader of sealed paths uses
/// this function and never a second implementation: two with different semantics would
/// exclude `a/b/.git/HEAD` in one place and *attribute* it in another.
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
