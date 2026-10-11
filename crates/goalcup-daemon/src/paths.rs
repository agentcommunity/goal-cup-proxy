//! Store path templates to real paths. Templates come from the manifest
//! (`store[].path_template`, `subagent_path_template`, `spill_path_template`,
//! `lock_paths[]`, `config_allowlist[]`, `foreign_scan[].path_glob`); the values come from
//! the entry.
//!
//! **One substitution syntax in the whole system.** Everything goes through
//! [`goalcup_protocol::template::render`], which is strict, and the nine legal tokens are
//! [`goalcup_protocol::manifest::PATH_TOKENS`]. A second renderer would be enumerated nowhere and
//! tested against no manifest, so a typo in a `path_template` would produce a silently non-matching
//! glob instead of an error. No manifest names an environment variable: the daemon resolves the
//! environment into `home` and `config_dir` before rendering, so a participant cannot move a tailed
//! store by exporting one.

use crate::DaemonError;
use goalcup_protocol::events::ForeignScanUnavailableReason;
use goalcup_protocol::manifest::PATH_TOKENS;
use goalcup_protocol::template::{TemplateVars, render};
use std::path::{Path, PathBuf};

/// `PathVars` fills exactly the nine names [`PATH_TOKENS`] declares. This fails the build if a
/// tenth is ever added without a field here to fill it — which is the failure that matters, because
/// a token `to_template_vars` does not set makes `render` reject every manifest that uses it. It is
/// a compile-time check because a run-time assertion would say nothing in a release build and would
/// panic, which nothing in `src/` may do.
const _: () = assert!(PATH_TOKENS.len() == 9);

/// The nine values a path template may refer to, and no others.
#[derive(Debug, Clone)]
pub struct PathVars {
    pub workspace: PathBuf,
    /// The participant's real home directory, resolved by the daemon. A manifest never
    /// names `$HOME`; it writes `{home}`. Used only where a connector must baseline an
    /// app-wide file it did not write.
    pub home: PathBuf,
    /// The harness's per-run configuration root — the directory `launch.home_env` points
    /// at (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`) or, for a harness with no single home
    /// variable, the per-run harness home the daemon created.
    pub config_dir: PathBuf,
    /// This entry's artifact directory.
    pub artifact_root: PathBuf,
    /// Known only once the session is bound; until then `{session_id}` renders as `*`,
    /// which the glob expander then matches against every session file. That is what a
    /// store reader wants before the session binds, and it is deliberately not an empty string:
    /// an empty segment would name a file that cannot exist and match nothing, silently.
    pub session_id: Option<String>,
    /// `(yyyy, mm, dd)` for the date-partitioned tree Codex writes. Passed in rather than
    /// read from the clock, so a test is deterministic and a run that crosses midnight can
    /// be given both days.
    pub date: (String, String, String),

    /// [`PathVars::workspace`] as `std::fs::canonicalize` answered for it once, at construction.
    ///
    /// Not `pub`, and not recomputed: every field beside it is data the caller supplies, and this
    /// is the one derived value the type owns. Deriving it per render would make the answer depend
    /// on when it was asked — a workspace sealed, cleaned up or simply removed mid-run turns a
    /// later `canonicalize` into a failure whose fallback is the raw path, so one `PathVars` would
    /// render two spellings of one workspace across its own life. Held here, a clone agrees with
    /// its original by construction and an abandoned workspace path reused for a new entry gets a
    /// fresh answer, because it gets a fresh `PathVars`.
    pub(crate) workspace_canonical: PathBuf,
}

impl PathVars {
    /// The nine values, with the canonical workspace resolved once, here.
    ///
    /// The one constructor, so [`PathVars::workspace_canonical`] can be a field rather than
    /// a filesystem call every render answers differently. A caller that wants a variant of
    /// an existing `PathVars` (the V1 foreign scan's undated one, before D48 removed it)
    /// clones and overwrites the fields it means to change, which keeps the canonical
    /// spelling the original resolved.
    pub fn new(
        workspace: PathBuf,
        home: PathBuf,
        config_dir: PathBuf,
        artifact_root: PathBuf,
        session_id: Option<String>,
        date: (String, String, String),
    ) -> PathVars {
        // The fallback is the raw path, which is what a workspace that does not exist yet
        // has to render as.
        let workspace_canonical =
            std::fs::canonicalize(&workspace).unwrap_or_else(|_| workspace.clone());
        PathVars {
            workspace,
            home,
            config_dir,
            artifact_root,
            session_id,
            date,
            workspace_canonical,
        }
    }

    /// The workspace as [`std::fs::canonicalize`] returns it, which is the spelling the harness
    /// writes.
    ///
    /// The two tokens a store template hangs off — `{workspace}` and `{slug}` — are compared
    /// against paths the harness produced from its own `getcwd`, and on macOS a `/var/folders/…`
    /// tempdir is `/private/var/folders/…` there. A `PathVars` built from the uncanonicalised path
    /// would render Claude Code's project directory as `-var-folders-…` while the harness writes
    /// `-private-var-…`, so the `main` glob would match nothing and leg-1 capture would be silently
    /// zero.
    ///
    /// Resolved here, in the one place both tokens are read, rather than left to each construction
    /// site: the V1 per-entry runtime canonicalised its own copy for its partitioner (both
    /// removed by D48), and two answers to how the workspace is spelled is the defect itself.
    /// A site that canonicalises before building its `PathVars` makes this a no-op; one that
    /// does not is still correct.
    ///
    /// **Resolved once, at construction, and never re-derived.** `std::fs::canonicalize` is
    /// fallible, and calling it fresh on every render would make the answer depend on when it is
    /// asked: a workspace sealed, cleaned up or simply removed mid-run turns the next render's call
    /// into a failure whose fallback is the raw path, a second spelling for the one workspace this
    /// `PathVars` had already rendered once.
    ///
    /// **A field and not a process-wide cache.** A cache keyed by the raw path answers for the
    /// process, so a workspace abandoned and then re-created at the same raw path — with a
    /// symlinked parent retargeted in between, which is exactly the shape `{slug}` is sensitive to
    /// — would be rendered with the first entry's canonical spelling, and the new entry's `main`
    /// glob would match nothing. A field is per `PathVars`, and a new entry builds a new one, so it
    /// cannot inherit a spelling that was true for a directory that is gone.
    pub fn workspace_canonical(&self) -> PathBuf {
        self.workspace_canonical.clone()
    }

    pub fn slug(&self) -> String {
        claude_slug(&self.workspace_canonical())
    }

    /// Exactly the nine names in [`PATH_TOKENS`], filled, and no others. A tenth name here
    /// and a manifest could use a token `path_tokens_used()` never validated. The count is
    /// held by the `const` assertion above this `impl`.
    pub fn to_template_vars(&self) -> TemplateVars {
        let mut v = TemplateVars::new();
        v.set(
            "workspace",
            self.workspace_canonical().to_string_lossy().to_string(),
        );
        v.set("home", self.home.to_string_lossy().to_string());
        v.set("config_dir", self.config_dir.to_string_lossy().to_string());
        v.set(
            "artifact_root",
            self.artifact_root.to_string_lossy().to_string(),
        );
        v.set("slug", self.slug());
        v.set(
            "session_id",
            self.session_id.clone().unwrap_or_else(|| "*".to_string()),
        );
        v.set("yyyy", self.date.0.clone());
        v.set("mm", self.date.1.clone());
        v.set("dd", self.date.2.clone());
        v
    }
}

/// Claude Code's project-directory name: every character that is not ASCII alphanumeric
/// becomes `-`. Measured against spike 4's
/// `projects/-Users-team-goalcup-spikes-ws-claude-code-cli-20260903T073054Z/`.
pub fn claude_slug(workspace: &Path) -> String {
    workspace
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Render a store `path_template` through the one renderer. An unknown token is an error,
/// not a silently non-matching glob; an unbound `{session_id}` renders as `*`.
pub fn resolve_template(template: &str, vars: &PathVars) -> Result<String, DaemonError> {
    Ok(render(template, &vars.to_template_vars())?)
}

/// Expand a resolved template into the paths that exist under it, sorted, so a poll order
/// never depends on the filesystem's iteration order.
///
/// `*` and `?` match within one path component. **There is no recursive wildcard, and
/// `**` is not rejected either: it is matched as an ordinary `*`, so `a/**/b` finds
/// `a/<one component>/b` and nothing deeper.** That is a real limit and not a guard — a
/// manifest that wrote `**` meaning "at any depth" would quietly match a smaller set — and
/// it is deliberate on both halves: a recursive wildcard over a participant's home is an
/// unbounded walk this daemon has no business doing, and no shipped manifest writes one.
///
/// A pattern with no wildcard is returned when it exists and dropped when it does not, and
/// an unreadable directory yields nothing rather than an error: a store whose directory
/// does not exist yet is normal before the harness starts, and a caller that needs to know
/// a path is absent asks the filesystem itself.
pub fn expand_globs(pattern: &str) -> Vec<PathBuf> {
    let path = Path::new(pattern);
    let mut frontier: Vec<PathBuf> = Vec::new();
    let mut rest = path.components().peekable();

    // The literal prefix: every component up to the first that holds a wildcard.
    let mut base = PathBuf::new();
    while let Some(component) = rest.peek() {
        let text = component.as_os_str().to_string_lossy();
        if is_wild(&text) {
            break;
        }
        base.push(component.as_os_str());
        rest.next();
    }
    frontier.push(base);

    for component in rest {
        let text = component.as_os_str().to_string_lossy().to_string();
        let mut next = Vec::new();
        for dir in &frontier {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if matches_component(&name, &text) {
                    next.push(dir.join(&name));
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            return Vec::new();
        }
    }

    let mut out: Vec<PathBuf> = frontier
        .into_iter()
        .filter(|p| p.symlink_metadata().is_ok())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Result of the foreign reader's bounded, fallible glob discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForeignGlobResult {
    Absent,
    Paths(Vec<PathBuf>),
    Unavailable(ForeignScanUnavailableReason),
}

/// The foreign-scan variant of [`expand_globs`]. It keeps ordinary glob behavior intact
/// while making every discovery failure and the cap visible to the coverage event. Its V1
/// foreign-scan caller was removed by D48; only this module's tests call it now.
pub fn expand_foreign_glob(pattern: &str, max_paths: usize) -> ForeignGlobResult {
    let path = Path::new(pattern);
    let mut frontier = Vec::new();
    let mut rest = path.components().peekable();
    let mut base = PathBuf::new();
    while let Some(component) = rest.peek() {
        let text = component.as_os_str().to_string_lossy();
        if is_wild(&text) {
            break;
        }
        base.push(component.as_os_str());
        rest.next();
    }

    if rest.peek().is_none() {
        return match base.symlink_metadata() {
            Ok(metadata) if metadata.file_type().is_file() => ForeignGlobResult::Paths(vec![base]),
            Ok(_) => ForeignGlobResult::Unavailable(ForeignScanUnavailableReason::Open),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => ForeignGlobResult::Absent,
            Err(_) => ForeignGlobResult::Unavailable(ForeignScanUnavailableReason::Discovery),
        };
    }
    match base.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => {
            return ForeignGlobResult::Unavailable(ForeignScanUnavailableReason::Discovery);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ForeignGlobResult::Absent;
        }
        Err(_) => {
            return ForeignGlobResult::Unavailable(ForeignScanUnavailableReason::Discovery);
        }
    }
    frontier.push(base);
    let mut entries_examined = 0usize;
    for component in rest {
        let text = component.as_os_str().to_string_lossy().to_string();
        let mut next = Vec::new();
        for dir in &frontier {
            match dir.symlink_metadata() {
                Ok(metadata) if metadata.file_type().is_dir() => {}
                Ok(_) => {
                    return ForeignGlobResult::Unavailable(ForeignScanUnavailableReason::Discovery);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return ForeignGlobResult::Unavailable(ForeignScanUnavailableReason::Race);
                }
                Err(_) => {
                    return ForeignGlobResult::Unavailable(ForeignScanUnavailableReason::Discovery);
                }
            }
            let entries = match read_matched_frontier(dir) {
                Ok(entries) => entries,
                Err(reason) => return ForeignGlobResult::Unavailable(reason),
            };
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(_) => {
                        return ForeignGlobResult::Unavailable(
                            ForeignScanUnavailableReason::Discovery,
                        );
                    }
                };
                entries_examined += 1;
                if entries_examined > max_paths {
                    return ForeignGlobResult::Unavailable(ForeignScanUnavailableReason::PathLimit);
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if matches_component(&name, &text) {
                    next.push(dir.join(&name));
                    if next.len() > max_paths {
                        return ForeignGlobResult::Unavailable(
                            ForeignScanUnavailableReason::PathLimit,
                        );
                    }
                }
            }
        }
        next.sort();
        next.dedup();
        frontier = next;
        if frontier.is_empty() {
            return ForeignGlobResult::Absent;
        }
    }

    let mut out = Vec::new();
    for path in frontier {
        match path.symlink_metadata() {
            Ok(metadata) if metadata.file_type().is_file() => out.push(path),
            Ok(_) => {
                return ForeignGlobResult::Unavailable(ForeignScanUnavailableReason::Open);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return ForeignGlobResult::Unavailable(ForeignScanUnavailableReason::Race);
            }
            Err(_) => {
                return ForeignGlobResult::Unavailable(ForeignScanUnavailableReason::Discovery);
            }
        }
    }
    out.sort();
    out.dedup();
    if out.is_empty() {
        ForeignGlobResult::Absent
    } else {
        ForeignGlobResult::Paths(out)
    }
}

/// Called only after `dir` was retained as a matched directory frontier. Its later
/// disappearance is therefore a race, unlike an initially absent fixed search base.
fn read_matched_frontier(dir: &Path) -> Result<std::fs::ReadDir, ForeignScanUnavailableReason> {
    std::fs::read_dir(dir).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ForeignScanUnavailableReason::Race
        } else {
            ForeignScanUnavailableReason::Discovery
        }
    })
}

fn is_wild(component: &str) -> bool {
    component.contains('*') || component.contains('?')
}

/// One path component against one glob component. `*` matches any run of characters
/// (including none) and `?` exactly one; a leading `.` is matched like any other
/// character, because the trees these globs walk are all dot-directories.
fn matches_component(name: &str, pattern: &str) -> bool {
    let n: Vec<char> = name.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    // Iterative backtracking, so a pathological pattern cannot recurse the stack away.
    let (mut i, mut j) = (0usize, 0usize);
    let (mut star, mut matched) = (None, 0usize);
    while i < n.len() {
        if j < p.len() && (p[j] == '?' || p[j] == n[i]) {
            i += 1;
            j += 1;
        } else if j < p.len() && p[j] == '*' {
            star = Some(j);
            matched = i;
            j += 1;
        } else if let Some(s) = star {
            j = s + 1;
            matched += 1;
            i = matched;
        } else {
            return false;
        }
    }
    while j < p.len() && p[j] == '*' {
        j += 1;
    }
    j == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_glob_keeps_absence_discovery_and_path_exhaustion_distinct() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            expand_foreign_glob(&dir.path().join("missing").display().to_string(), 2),
            ForeignGlobResult::Absent
        );

        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, b"ordinary file").unwrap();
        assert_eq!(
            expand_foreign_glob(&format!("{}/*", blocked.display()), 2),
            ForeignGlobResult::Unavailable(ForeignScanUnavailableReason::Discovery)
        );

        let many = dir.path().join("many");
        std::fs::create_dir(&many).unwrap();
        for name in ["a", "b", "c"] {
            std::fs::write(many.join(name), b"x").unwrap();
        }
        assert_eq!(
            expand_foreign_glob(&format!("{}/*", many.display()), 2),
            ForeignGlobResult::Unavailable(ForeignScanUnavailableReason::PathLimit)
        );

        let nonmatching = dir.path().join("nonmatching");
        std::fs::create_dir(&nonmatching).unwrap();
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(nonmatching.join(name), b"x").unwrap();
        }
        assert_eq!(
            expand_foreign_glob(&format!("{}/*.jsonl", nonmatching.display()), 2),
            ForeignGlobResult::Unavailable(ForeignScanUnavailableReason::PathLimit),
            "nonmatching directory entries consume the same finite discovery budget"
        );

        assert!(
            matches!(
                read_matched_frontier(&dir.path().join("vanished")),
                Err(ForeignScanUnavailableReason::Race)
            ),
            "once a frontier was matched, its disappearance is not clean absence"
        );
    }

    #[cfg(unix)]
    #[test]
    fn foreign_glob_refuses_symbolic_sources_and_frontiers() {
        let dir = tempfile::tempdir().unwrap();
        let regular = dir.path().join("record.jsonl");
        std::fs::write(&regular, b"{}\n").unwrap();
        let source_link = dir.path().join("linked.jsonl");
        std::os::unix::fs::symlink(&regular, &source_link).unwrap();
        assert_eq!(
            expand_foreign_glob(&source_link.display().to_string(), 2),
            ForeignGlobResult::Unavailable(ForeignScanUnavailableReason::Open)
        );

        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let frontier_link = dir.path().join("linked-dir");
        std::os::unix::fs::symlink(&real, &frontier_link).unwrap();
        assert_eq!(
            expand_foreign_glob(&format!("{}/*.jsonl", frontier_link.display()), 2),
            ForeignGlobResult::Unavailable(ForeignScanUnavailableReason::Discovery)
        );
    }

    /// The participant's harness records its own `getcwd`, which on macOS resolves
    /// `/var/folders/…` to `/private/var/folders/…`. Claude Code's project directory is
    /// named from that, so a `PathVars` holding the uncanonicalised spelling renders a
    /// `{slug}` — and a `{workspace}` — that matches nothing, and leg-1 capture is silently
    /// zero with no counter to show for it.
    #[test]
    fn a_symlinked_parent_still_renders_the_slug_the_harness_writes() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let real = dir.path().join("real/ws");
        std::fs::create_dir_all(&real).expect("the real workspace");
        // The parent is a symlink, which is exactly what `/var` is on macOS.
        let linked_parent = dir.path().join("link");
        std::os::unix::fs::symlink(dir.path().join("real"), &linked_parent)
            .expect("the symlinked parent");
        let through_link = linked_parent.join("ws");

        let vars = PathVars::new(
            through_link.clone(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            None,
            ("2026".into(), "09".into(), "05".into()),
        );
        let canonical = std::fs::canonicalize(&through_link).expect("it canonicalises");
        assert_ne!(
            through_link, canonical,
            "the case only exists while the two spellings differ"
        );
        assert_eq!(
            vars.slug(),
            claude_slug(&canonical),
            "`{{slug}}` is the harness's own getcwd, not the path the daemon was handed"
        );
        let rendered = resolve_template("{workspace}/x", &vars).expect("it renders");
        assert_eq!(rendered, format!("{}/x", canonical.display()));
    }

    /// `workspace_canonical()` is resolved once: a workspace that vanishes mid-run — sealed,
    /// cleaned up, or simply removed — must not make the next render fall back to the raw,
    /// uncanonicalised path after an earlier render of the same `PathVars` answered with the
    /// canonical one. One `{workspace}` rendered two different ways over the life of one entry is
    /// the defect the canonical spelling exists to prevent, across time instead of across call
    /// sites.
    #[test]
    fn workspace_canonical_is_stable_after_the_workspace_directory_is_removed() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        // A symlinked parent, as above, so the raw and canonical spellings actually differ
        // — otherwise a stale fallback and a fresh success would look identical by luck.
        let real = dir.path().join("real/ws");
        std::fs::create_dir_all(&real).expect("the real workspace");
        let linked_parent = dir.path().join("link");
        std::os::unix::fs::symlink(dir.path().join("real"), &linked_parent)
            .expect("the symlinked parent");
        let through_link = linked_parent.join("ws");

        let vars = PathVars::new(
            through_link.clone(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            None,
            ("2026".into(), "09".into(), "05".into()),
        );
        let canonical = std::fs::canonicalize(&through_link).expect("it canonicalises");
        assert_eq!(
            vars.workspace_canonical(),
            canonical,
            "the first render resolves and remembers it"
        );

        std::fs::remove_dir_all(&real).expect("the workspace directory is removed");
        assert!(
            std::fs::canonicalize(&through_link).is_err(),
            "the case only exists while a fresh canonicalise would now fail"
        );
        assert_eq!(
            vars.workspace_canonical(),
            canonical,
            "a render after the workspace is gone must still answer with the spelling \
             already committed to, not fall back to the raw path a fresh canonicalise \
             cannot avoid once the directory is gone"
        );

        // A clone agrees, which is the property a caller that clones its `vars` freely
        // needs: two clones disagreeing about the one workspace they both name
        // would be the same defect one instance disagreeing with itself over time is.
        assert_eq!(
            vars.clone().workspace_canonical(),
            canonical,
            "a clone carries the spelling its original resolved"
        );
    }

    /// The stability above is per `PathVars`, not a process-wide cache keyed by the raw workspace
    /// path: a cache keyed that way answers for the process rather than for the entry.
    ///
    /// A workspace abandoned and then re-created at the same raw path — with the symlinked parent
    /// retargeted in between, which is exactly the shape `{slug}` is sensitive to — is a different
    /// directory that happens to be spelled the same way. A process-wide cache would hand the new
    /// entry the old entry's canonical spelling, so its `{workspace}` and `{slug}` would name a
    /// directory that no longer existed and its `main` glob would match nothing: leg-1 capture
    /// silently zero, with no counter to show for it. Resolved once per `PathVars`, a new entry
    /// builds a new one and cannot inherit anything.
    #[test]
    fn a_workspace_recreated_at_the_same_raw_path_renders_its_own_canonical_spelling() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let first_real = dir.path().join("first/ws");
        let second_real = dir.path().join("second/ws");
        std::fs::create_dir_all(&first_real).expect("the first workspace");
        std::fs::create_dir_all(&second_real).expect("the second workspace");
        let linked_parent = dir.path().join("link");
        std::os::unix::fs::symlink(dir.path().join("first"), &linked_parent)
            .expect("the symlinked parent");
        let raw = linked_parent.join("ws");

        let first = PathVars::new(
            raw.clone(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            None,
            ("2026".into(), "09".into(), "05".into()),
        );
        assert_eq!(
            first.workspace_canonical(),
            std::fs::canonicalize(&first_real).expect("it canonicalises")
        );

        // Abandoned, and the same raw path re-pointed at a different directory.
        std::fs::remove_file(&linked_parent).expect("the old link goes");
        std::os::unix::fs::symlink(dir.path().join("second"), &linked_parent)
            .expect("the retargeted parent");

        let second = PathVars::new(
            raw,
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            None,
            ("2026".into(), "09".into(), "05".into()),
        );
        let want = std::fs::canonicalize(&second_real).expect("it canonicalises");
        assert_ne!(
            first.workspace_canonical(),
            want,
            "the case only exists while the two entries' canonical spellings differ"
        );
        assert_eq!(
            second.workspace_canonical(),
            want,
            "the new entry renders its OWN workspace, not the one a process-wide cache \
             remembered for the raw path"
        );
        assert_eq!(
            second.slug(),
            claude_slug(&want),
            "and so does `{{slug}}`, which is the token the `main` glob hangs off"
        );
    }
}
