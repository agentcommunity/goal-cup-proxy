//! Workspace creation and folder-scoped config (D5, D6, D21).
//!
//! The daemon renders and writes every `hook_install[]` entry, in all three scopes, and
//! hands the CLI the resulting paths and hashes. The CLI writes none of them: one writer
//! means one list of files to watch, restore, exclude from the seal and disclose.
//!
//! Nothing here interprets a rule. Every comparison below is against a constant a manifest
//! carries or a hash of bytes on disk; what a mismatch *means* is `goalcup-verifier`'s.

use crate::DaemonError;
use crate::fsutil::set_mode;
use goalcup_platform::tracecap::EntryArtifactRoot;
use goalcup_protocol::control::InstalledFile;
use goalcup_protocol::manifest::{ConnectorManifest, InstallScope};
use goalcup_protocol::seal::{SealManifest, SealedFile, is_excluded};
use goalcup_protocol::template::{TemplateVars, render};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The 64-zero digest that stands for "this file did not exist when we looked".
///
/// Public because the config watch reports it as a deviation's `observed_sha256` when a
/// tracker file has been deleted, and two spellings of "absent" — one here, one there — is
/// how the baseline and the watch come to disagree about a file nobody can read.
pub const ABSENT: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// The default mode for a file the tracker writes. `hook_install[].mode` overrides it, and
/// [`owner_only_if_secret`] narrows whichever of the two applies when the file carries the
/// run's bearer.
pub const DEFAULT_MODE: u32 = 0o644;

/// The template variable whose presence makes a rendered file a secret.
///
/// The `{receiver_token}` a template interpolates is the per-run bearer. At `0o644` any
/// second local account on the machine could read it, and the entry id and the working
/// directory are in the same file. The registry, the write-ahead buffer and `control.json`
/// each argue at length for `0600`; this is the same argument about the same secret in a
/// different file.
const SECRET_VAR: &str = "{receiver_token}";

/// `mode`, with the group and other bits removed, when `template` interpolates the bearer.
///
/// **`mode & 0o700` rather than a flat `0o600`**: masking keeps the owner's execute bit on
/// an executable file and takes away exactly the group and other bits — every bit that lets
/// somebody who is not the participant read the file.
fn owner_only_if_secret(template: &str, mode: u32) -> u32 {
    match template.contains(SECRET_VAR) {
        true => mode & 0o700,
        false => mode,
    }
}

/// Bytes read per hashing step. A workspace file can be arbitrarily large and bounded
/// memory is a constraint on the whole daemon, so the seal walk never holds one whole.
const HASH_CHUNK: usize = 64 * 1024;

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// The three roots a connector may install into. Only the first is the workspace.
///
/// `artifact_root` is **this entry's** artifact directory, not the machine-wide one: it is
/// what `{artifact_root}` renders to in every template, where `InstallScope::ArtifactRoot`
/// files land, and what [`Workspace::artifact_dir`] then reports. One directory under one
/// name, because two would mean the `--settings` path the CLI launches with and the
/// directory the daemon writes the trace into were different places.
#[derive(Debug, Clone)]
pub struct Roots {
    pub workspace: PathBuf,
    pub artifact_root: PathBuf,
    pub harness_home: PathBuf,
}

impl Roots {
    fn root_for(&self, scope: InstallScope) -> &Path {
        match scope {
            InstallScope::Workspace => &self.workspace,
            InstallScope::ArtifactRoot => &self.artifact_root,
            InstallScope::HarnessHome => &self.harness_home,
        }
    }
}

/// Rebuild one rendered path from its declared scope without letting a public
/// [`TrackerFile::absolute_path`] choose the write destination. This is lexical on purpose:
/// fresh workspace setup must reject malformed names before it touches the filesystem, not
/// resolve a pre-existing symlink and turn that resolution into write authority.
fn scoped_tracker_path(root: &Path, relative_path: &str) -> Result<PathBuf, DaemonError> {
    use std::path::Component;

    let mut expected = root.to_path_buf();
    for component in Path::new(relative_path).components() {
        match component {
            Component::Normal(part) => expected.push(part),
            _ => {
                return Err(DaemonError::Refused(format!(
                    "tracker file has non-normal relative path {relative_path:?}"
                )));
            }
        }
    }
    if expected == root {
        return Err(DaemonError::Refused(
            "tracker file has an empty relative path".to_string(),
        ));
    }
    Ok(expected)
}

/// `Installation` remains public for the configuration/watch documents that carry it, so
/// every public writer must independently bind each supplied absolute spelling to its
/// declared scope. In particular a `Workspace` record cannot smuggle an artifact-root
/// pathname into [`create_workspace`]'s lexical workspace writer.
fn validate_installation_paths(
    roots: &Roots,
    installation: &Installation,
) -> Result<(), DaemonError> {
    for file in &installation.files {
        let expected = scoped_tracker_path(roots.root_for(file.scope), &file.relative_path)?;
        if file.absolute_path != expected {
            return Err(DaemonError::Refused(format!(
                "tracker file {:?} does not match its {:?} root: expected {}, found {}",
                file.relative_path,
                file.scope,
                expected.display(),
                file.absolute_path.display()
            )));
        }
    }
    Ok(())
}

/// One file the tracker rendered from a connector's `hook_install[]`.
///
/// `absolute_path` is carried as well as `relative_path` because the tracker writes into
/// three scopes and only one of them is the workspace: Claude's `--settings` file lands in
/// the artifact root, where the agent cannot enumerate it, and the config watch must watch
/// it by absolute path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerFile {
    /// Relative to its scope's root, forward slashes.
    pub relative_path: String,
    pub absolute_path: PathBuf,
    pub scope: InstallScope,
    pub contents: String,
    /// sha256 of `contents` as written.
    pub sha256: String,
    /// The mode this file is written at: `hook_install[].mode`, narrowed to the owner alone
    /// when the template interpolates `{receiver_token}` (see [`owner_only_if_secret`]).
    ///
    /// The mode travels with the bytes, so the file is created at its final mode and every
    /// writer of a `TrackerFile` — including a config watch putting a deviating one back — has
    /// the one answer to hand. There is no window in which a file holding the run's bearer is
    /// world-readable.
    pub mode: u32,
    /// Restoring this file invalidates hook trust, so a restore raises `arming_lost` (D6).
    pub restore_invalidates_trust: bool,
    /// `hook_install[].harness_owned`, carried here because the two things it decides
    /// happen in two crates: [`rendered`] writes this file only when it is not already
    /// there, and the V1 config watch (removed by D48) left it off the watch entirely.
    /// See `goalcup_protocol::manifest::HookInstall::harness_owned` for why a
    /// connector asks for it.
    ///
    /// It is not a third state of `restore_invalidates_trust`: an unwatched file is never
    /// restored, so it can never lose trust by being restored, and the two are independent
    /// the same way `AllowlistedFile`'s never-restored files are.
    pub harness_owned: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Installation {
    pub files: Vec<TrackerFile>,
    /// Workspace-relative paths of everything the tracker put in the workspace, sorted.
    /// The seal excludes exactly these and the public entry discloses them; deriving
    /// the list anywhere else would let the two drift.
    pub workspace_relative: Vec<String>,
}

impl Installation {
    /// The installation a caller already holding the rendered files describes.
    ///
    /// **The one derivation of `workspace_relative`.** `render_tracker_files` finishes
    /// here, as did the V1 per-entry runtime rebuilding its exclusion union from the files
    /// it was handed (removed by D48), so the list the seal excludes and the list the public
    /// entry discloses cannot come apart. A second `filter(scope == Workspace)` written
    /// somewhere else is how the union becomes a subset.
    pub fn from_files(files: Vec<TrackerFile>) -> Installation {
        let mut workspace_relative: Vec<String> = files
            .iter()
            .filter(|f| f.scope == InstallScope::Workspace)
            .map(|f| f.relative_path.clone())
            .collect();
        workspace_relative.sort();
        workspace_relative.dedup();
        Installation {
            files,
            workspace_relative,
        }
    }

    /// The view the control API hands the CLI on `WorkspaceInfo.installed_files`.
    /// Contents are deliberately absent: the CLI needs paths and hashes, not bytes.
    pub fn as_protocol(&self) -> Vec<InstalledFile> {
        self.files
            .iter()
            .map(|f| InstalledFile {
                path: f.absolute_path.to_string_lossy().to_string(),
                scope: f.scope,
                relative_path: f.relative_path.clone(),
                sha256: f.sha256.clone(),
                bytes: f.contents.len() as u64,
            })
            .collect()
    }
}

/// An app-wide file named in `config_allowlist[]` that the tracker did not write, hashed
/// just before launch so a mid-run change to it is reportable (D21).
///
/// **Never restored**, and that is the design rather than a shortcut. These are the
/// participant's own files, outside every tracker-created folder; writing to them is
/// forbidden twice over. A deviation on one is reported and chained, never put back. A
/// file that does not exist at launch is recorded with `existed = false` and the 64-zero
/// hash, so its later appearance is as visible as an existing one changing — which is the
/// case that matters, because `~/.claude/settings.local.json` not existing at launch and
/// existing an hour later is a participant adding app-wide hooks the tracker never wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct AllowlistedFile {
    pub path: PathBuf,
    /// The path as the manifest names it: the `config_allowlist[]` entry's leading token
    /// and the rest, `{home}/.claude/settings.json` rather than `/Users/someone/...`.
    ///
    /// This is what the config watch chains. A tracker file is chained relative to its
    /// scope root; an app-wide file was chained absolute, which put the participant's real
    /// home directory - their login name - into every deviation event about one. The token
    /// is the scope here, so carrying it keeps the two spellings comparable without naming
    /// anyone's disk.
    pub relative_path: String,
    /// sha256 of the bytes at launch, or 64 zeros when the file did not exist.
    pub sha256: String,
    /// Recorded because D21 asks for it: a file whose mtime moved but whose bytes did not
    /// is a different event from one that changed, and the reviewer wants both.
    pub mtime_unix: Option<f64>,
    pub existed: bool,
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub artifact_dir: PathBuf,
    /// `artifact_dir.join("trace")` — the one directory `TraceExport` owns.
    pub trace_dir: PathBuf,
    pub harness_home: PathBuf,
    /// Workspace-relative path → expected sha256, for the config watch. Files outside
    /// the workspace are watched through `installed.files`' absolute paths.
    pub expected_config: BTreeMap<String, String>,
    pub installed: Installation,
    /// The zero-entry proof, as the `SealManifest` shape the verifier compares a sealed
    /// file against. It is a manifest and not a list of names because attribution compares
    /// **path and digest**: a file present in the baseline under a different digest is not
    /// `Baseline`, it was rewritten. Empty `files` on a fresh workspace is the statement,
    /// not an omission.
    pub baseline: SealManifest,
    /// The app-wide files `config_allowlist[]` names, hashed at launch. Watched, reported,
    /// never restored.
    pub allowlist: Vec<AllowlistedFile>,
}

/// The artifact root must not be the workspace, must not be an ancestor of it, must
/// not be inside it, and must not be its sibling.
///
/// The sibling clause is the one that looks like paranoia and is not: an agent running
/// `ls -la ..` on its first tool call reads whatever the tracker keeps beside the workspace,
/// and a mis-launched Codex session listing `baseline/files/auth.json` at the artifact root
/// shows the same exposure. A sibling is exactly what `ls ..` shows.
///
/// Compared as written. The daemon's own config already refuses an entangled
/// `workspace_parent`/`artifact_root` pair through a symlink-aware check; this is the
/// per-entry restatement over the two concrete directories, and neither replaces the other.
pub fn check_artifact_placement(artifact_root: &Path, workspace: &Path) -> Result<(), DaemonError> {
    if artifact_root == workspace {
        return Err(DaemonError::Guard(format!(
            "artifact root {} is identical to the workspace: the run's own evidence would \
             be the first thing the agent lists",
            artifact_root.display()
        )));
    }
    if workspace.starts_with(artifact_root) {
        return Err(DaemonError::Guard(format!(
            "artifact root {} is an ancestor of workspace {}",
            artifact_root.display(),
            workspace.display()
        )));
    }
    if artifact_root.starts_with(workspace) {
        return Err(DaemonError::Guard(format!(
            "artifact root {} is inside workspace {}",
            artifact_root.display(),
            workspace.display()
        )));
    }
    if artifact_root.parent() == workspace.parent() {
        return Err(DaemonError::Guard(format!(
            "artifact root {} is a sibling of workspace {}; the agent enumerates the \
             parent directory on its first tool call (M0 §2.4)",
            artifact_root.display(),
            workspace.display()
        )));
    }
    // The config check, applied a second time to the two concrete directories: the clauses
    // above compare the paths as written, and `..` after a symlink does not mean what it
    // reads as. This is the pass that resolves them, and it is called rather than copied.
    if crate::config::nests(artifact_root, workspace) {
        return Err(DaemonError::Guard(format!(
            "artifact root {} and workspace {} are entangled once symlinks are resolved: \
             the directory the agent actually enumerates is not the one either path spells",
            artifact_root.display(),
            workspace.display()
        )));
    }
    Ok(())
}

/// The nearest ancestor holding a `.git` entry, file or directory, or `None`. A `.git`
/// *file* counts: that is what a linked worktree has.
pub fn git_worktree_of(path: &Path) -> Option<PathBuf> {
    let mut cur = Some(path);
    while let Some(dir) = cur {
        if dir.join(".git").symlink_metadata().is_ok() {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

/// Read one template body from `connectors/<id>/files/<name>`.
pub fn template_body(connector_dir: &Path, name: &str) -> Result<String, DaemonError> {
    let path = connector_dir.join("files").join(name);
    std::fs::read_to_string(&path).map_err(|source| DaemonError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// Write one rendered tracker file at `mode`, creating its parent.
///
/// The mode is carried on the create as well as set afterwards, and both halves are needed.
/// The create is what closes the window a token-bearing file would otherwise spend at the
/// process umask's `0644` before a `chmod` narrowed it; the `set_mode` after is what
/// applies the mode to a file that already existed, where `OpenOptions::mode` says nothing.
fn write_file(path: &Path, text: &str, mode: u32) -> Result<(), DaemonError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| DaemonError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let named = |source: std::io::Error| DaemonError::Io {
        path: path.display().to_string(),
        source,
    };
    let mut file = create_at(path, mode).map_err(named)?;
    {
        use std::io::Write;
        file.write_all(text.as_bytes()).map_err(named)?;
    }
    set_mode(path, mode)
}

#[cfg(unix)]
fn create_at(path: &Path, mode: u32) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(path)
}

#[cfg(not(unix))]
fn create_at(path: &Path, _mode: u32) -> std::io::Result<std::fs::File> {
    std::fs::File::create(path)
}

/// Render the connector's `hook_install[]` entries, writing the ones that land **outside**
/// the workspace.
///
/// The workspace-scoped ones are rendered and returned but not written, because the
/// workspace must have zero entries when it is created and a file the tracker itself
/// dropped in first would make that check pass or fail on its own writes.
/// [`create_workspace`] writes them, after it has counted zero entries.
///
/// `optional = false` renders the default set; `optional = true` renders only the fallback
/// entries, which the arming gate takes when the primary discovery path did not arm
/// (opencode's `opencode.json`).
///
/// One pass. An entry that declares `pin_hash_as` has that variable rendered empty, so no
/// hook-hash pin (D24) is rendered.
///
/// Fixture-only lexical renderer. Production fresh setup owns an
/// [`EntryArtifactRoot`] and calls [`prepare_with_artifact`] instead.
#[cfg(any(test, feature = "test-hooks"))]
pub fn render_tracker_files(
    manifest: &ConnectorManifest,
    connector_dir: &Path,
    vars: &TemplateVars,
    roots: &Roots,
    optional: bool,
) -> Result<Installation, DaemonError> {
    rendered(manifest, connector_dir, vars, roots, optional, None)
}

/// The render pass both callers share: the fixture renderer and fresh setup. A restart
/// re-render without writes was its other caller until D48 removed it.
fn rendered(
    manifest: &ConnectorManifest,
    connector_dir: &Path,
    vars: &TemplateVars,
    roots: &Roots,
    optional: bool,
    artifact: Option<&EntryArtifactRoot>,
) -> Result<Installation, DaemonError> {
    let mut done = Installation::default();
    let selected: Vec<_> = manifest
        .hook_install
        .iter()
        .filter(|h| h.optional == optional)
        .collect();

    for spec in &selected {
        let body = template_body(connector_dir, &spec.template)?;
        let mut local = vars.clone();
        if let Some(name) = &spec.pin_hash_as {
            // Declare the variable so `render` stays strict, but leave it empty.
            local.set(name, String::new());
        }
        let contents = render(&body, &local)?;
        let relative_path = render(&spec.path, &local)?;
        let absolute_path = roots.root_for(spec.scope).join(&relative_path);
        // Decided from the template and not from the rendered bytes: `{receiver_token}` is
        // what the template says, and the rendered file holds the token itself, which is a
        // string this function would then have to be handed in order to look for.
        let mode = owner_only_if_secret(&body, spec.mode);
        // Write-if-absent, for the one kind of entry that asks for it: the file belongs to
        // the harness, so a home that already holds one keeps it byte for byte.
        let found = spec.harness_owned && absolute_path.exists();
        if !found && spec.scope != InstallScope::Workspace {
            write_non_workspace_file(
                artifact,
                spec.scope,
                &relative_path,
                &absolute_path,
                &contents,
                mode,
            )?;
        }
        let sha256 = sha256_hex(contents.as_bytes());
        done.files.push(TrackerFile {
            relative_path,
            absolute_path,
            scope: spec.scope,
            contents,
            sha256,
            mode,
            restore_invalidates_trust: spec.restore_invalidates_trust,
            harness_owned: spec.harness_owned,
        });
    }

    // Through the one derivation, so every file's relative path is in the list under the
    // bytes it was written with.
    Ok(Installation::from_files(done.files))
}

/// The exclusion union the seal actually applies: the connector's `workspace_exclusions[]`
/// plus every workspace-scoped tracker file.
///
/// Returned so `SealManifest.excluded` is populated with what was used rather than with a
/// subset. Before this existed the field had no writer and no consumer, so the seal could
/// not say what it left out — and OpenCode's own tracker files sat outside both lists,
/// which is either an `unexplained_seal_file` raised on the tracker's own plugin or a
/// silent hole, depending on which list you read.
pub fn exclusion_union(manifest: &ConnectorManifest, installed: &Installation) -> Vec<String> {
    let mut out: Vec<String> = manifest.workspace_exclusions.clone();
    out.extend(installed.workspace_relative.iter().cloned());
    out.sort();
    out.dedup();
    out
}

/// Walk `workspace`, applying [`is_excluded`] with `excluded`, and return a sorted
/// manifest. One walker for the baseline and for the seal, so the zero-entry proof and the
/// seal can never disagree about what was left out.
///
/// The exclusion rule is root-anchored: a pattern matches a path that equals it or lies
/// beneath it, and nothing else. `mkdir src/.git` therefore hides nothing — a nested `.git`
/// is the agent's work until a connector writes `**/.git` and means it.
pub fn list_workspace(
    workspace: &Path,
    entry_id: &str,
    at: &str,
    excluded: &[String],
) -> Result<SealManifest, DaemonError> {
    let mut files = Vec::new();
    let mut stack = vec![workspace.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|source| DaemonError::Io {
            path: dir.display().to_string(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| DaemonError::Io {
                path: dir.display().to_string(),
                source,
            })?;
            let path = entry.path();
            let rel = relative_to(workspace, &path);
            if is_excluded(&rel, excluded) {
                continue;
            }
            // `file_type()` comes from the directory read and does not follow a symlink,
            // so a link pointing at an ancestor cannot make this walk loop.
            let kind = entry.file_type().map_err(|source| DaemonError::Io {
                path: path.display().to_string(),
                source,
            })?;
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            let (bytes, sha256) = measure(&path);
            files.push(SealedFile {
                path: rel,
                bytes,
                sha256,
            });
        }
    }
    Ok(SealManifest {
        entry_id: entry_id.to_string(),
        sealed_at: at.to_string(),
        workspace: workspace.to_string_lossy().to_string(),
        files,
        excluded: excluded.to_vec(),
    }
    .sorted())
}

/// A workspace-relative path with forward slashes.
fn relative_to(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Length and digest, streamed, so a large workspace file is never held whole (bounded
/// memory). An entry that cannot be read — a broken symlink, a socket, a file whose
/// permissions changed under the walk — is still listed, with the absent digest: dropping
/// it would make the seal quieter than the workspace it describes.
fn measure(path: &Path) -> (u64, String) {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return (0, ABSENT.to_string());
    };
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buf = vec![0u8; HASH_CHUNK];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                hasher.update(&buf[..n]);
                total += n as u64;
            }
            Err(_) => return (total, ABSENT.to_string()),
        }
    }
    (total, hex::encode(hasher.finalize()))
}

/// Hash every `config_allowlist[]` path immediately before launch (D21), recording its
/// mtime. Sorted and unique, so two globs that overlap do not produce
/// two rows for one file.
pub fn baseline_allowlist(
    manifest: &ConnectorManifest,
    vars: &crate::paths::PathVars,
) -> Result<Vec<AllowlistedFile>, DaemonError> {
    let mut out: Vec<AllowlistedFile> = Vec::new();
    for template in &manifest.config_allowlist {
        let rendered = crate::paths::resolve_template(template, vars)?;
        // Rendered once per template rather than once per matched path: a glob expands to
        // many files and they all hang off the same token.
        let root = token_root(template, vars);
        for path in crate::paths::expand_globs(&rendered) {
            record_allowlisted(&path, root.as_ref(), &mut out);
        }
        // A glob that matched nothing, or a plain path that does not exist, still gets a
        // row: its later appearance is the event this clause is actually for.
        if !rendered.contains(['*', '?']) {
            let path = PathBuf::from(&rendered);
            if !out.iter().any(|a| a.path == path) {
                record_allowlisted(&path, root.as_ref(), &mut out);
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out.dedup_by(|a, b| a.path == b.path);
    Ok(out)
}

/// The leading path token of a `config_allowlist[]` entry and what it resolves to, so a
/// matched file can be named `{home}/...` instead of by absolute path.
///
/// `None` for an entry that does not begin with a token - which no shipped manifest writes,
/// and which `path_tokens_used` would not have caught, so the absolute path stays the
/// fallback rather than becoming an error that stops a launch over a name.
fn token_root(template: &str, vars: &crate::paths::PathVars) -> Option<(String, PathBuf)> {
    let end = template.strip_prefix('{')?.find('}')? + 1;
    let token = &template[..=end];
    let root = crate::paths::resolve_template(token, vars).ok()?;
    Some((token.to_string(), PathBuf::from(root)))
}

/// `path` spelled against `root`'s token, falling back to the absolute path when the file
/// is not under the token it was rendered from - which a glob cannot produce, and which a
/// symlinked home could.
fn token_relative(root: Option<&(String, PathBuf)>, path: &Path) -> String {
    match root.and_then(|(token, base)| path.strip_prefix(base).ok().map(|rest| (token, rest))) {
        Some((token, rest)) => format!("{token}/{}", rest.to_string_lossy().replace('\\', "/")),
        None => path.display().to_string(),
    }
}

fn record_allowlisted(
    path: &Path,
    root: Option<&(String, PathBuf)>,
    out: &mut Vec<AllowlistedFile>,
) {
    if out.iter().any(|a| a.path == path) {
        return;
    }
    let existed = path.is_file();
    let (_, digest) = if existed {
        measure(path)
    } else {
        (0, ABSENT.to_string())
    };
    let mtime_unix = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64());
    out.push(AllowlistedFile {
        path: path.to_path_buf(),
        relative_path: token_relative(root, path),
        sha256: digest,
        mtime_unix,
        existed,
    });
}

/// Every reason a workspace is refused, in one place, and **without writing anything**.
///
/// An artifact root the agent could enumerate, a directory inside a Git worktree, and a
/// directory that already has entries — "the workspace started empty" as a checkable fact
/// rather than a claim.
///
/// It creates no directory, not even the workspace: a caller has to be able to ask "would
/// this be refused?" before it renders a single byte, because `render_tracker_files` writes
/// the artifact-root files, which can carry the run's bearer. A refusal after that point
/// would leave a live bearer on disk for a run that never starts. So `prepare` asks here
/// first, and [`create_workspace`] asks again for a caller that skipped it.
///
/// A workspace that does not exist yet has zero entries; that is the normal case, and it
/// is [`create_workspace`] that then creates it.
pub fn check_workspace_available(roots: &Roots) -> Result<(), DaemonError> {
    check_artifact_placement(&roots.artifact_root, &roots.workspace)?;
    if let Some(repo) = git_worktree_of(&roots.workspace) {
        return Err(DaemonError::Guard(format!(
            "workspace {} is inside the Git worktree {} (R3: a new directory outside any \
             Git worktree)",
            roots.workspace.display(),
            repo.display()
        )));
    }
    let existing = match std::fs::read_dir(&roots.workspace) {
        Ok(entries) => entries.count(),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => 0,
        Err(source) => {
            return Err(DaemonError::Io {
                path: roots.workspace.display().to_string(),
                source,
            });
        }
    };
    if existing > 0 {
        return Err(DaemonError::Guard(format!(
            "workspace {} is not empty ({existing} entries); R3 asks for zero",
            roots.workspace.display()
        )));
    }
    Ok(())
}

/// The one artifact-root file is Claude's launch settings, written at `0o600`. Production
/// setup passes the retained entry authority and must never use `absolute_path` as write
/// authority after it has created `artifacts/<entry>`. The legacy `None` branch remains for
/// isolated renderer fixtures that intentionally construct their own lexical tree.
fn write_non_workspace_file(
    artifact: Option<&EntryArtifactRoot>,
    scope: InstallScope,
    relative_path: &str,
    absolute_path: &Path,
    contents: &str,
    mode: u32,
) -> Result<(), DaemonError> {
    if scope != InstallScope::ArtifactRoot {
        return write_file(absolute_path, contents, mode);
    }
    let Some(artifact) = artifact else {
        return write_file(absolute_path, contents, mode);
    };
    if relative_path != "settings-launch.json" || mode != 0o600 {
        return Err(DaemonError::Refused(format!(
            "artifact-root hook {relative_path:?} has no retained capability writer"
        )));
    }
    artifact.replace_launch_settings(contents.as_bytes()).map_err(|error| {
        DaemonError::Refused(format!(
            "launch settings cannot be replaced through retained entry artifact authority: {error}"
        ))
    })
}

/// Create the workspace and record what landed in it.
///
/// Every refusal [`check_workspace_available`] names is checked here first, before this
/// function creates a directory or writes a file.
///
/// Call `render_tracker_files` first and pass the whole `Installation`: this function
/// writes the workspace-scoped files itself, after it has proved the directory was empty.
///
/// `excluded` is what [`exclusion_union`] returned, passed in rather than recomputed, so
/// `Workspace.baseline.excluded` holds exactly the union the seal applies. Without it the
/// zero-entry baseline and the seal could disagree about what was left out.
pub fn create_workspace(
    roots: &Roots,
    entry_id: &str,
    files: &Installation,
    excluded: &[String],
) -> Result<Workspace, DaemonError> {
    // This must precede `check_workspace_available` and directory creation: callers outside
    // this module can construct `TrackerFile`, but no forged `absolute_path` gets to choose
    // a filesystem write before its declared scope and relative spelling agree.
    validate_installation_paths(roots, files)?;
    check_workspace_available(roots)?;
    std::fs::create_dir_all(&roots.workspace).map_err(|source| DaemonError::Io {
        path: roots.workspace.display().to_string(),
        source,
    })?;

    // The entry artifact directory is created before this function by a component-by-
    // component native capability acquisition.  Do not add a lexical `create_dir_all`
    // fallback here: a workspace setup path must not become authority to recreate an entry
    // after the configured root or an ancestor has been swapped.
    let artifact_dir = roots.artifact_root.clone();

    let mut expected_config = BTreeMap::new();
    for file in &files.files {
        if file.scope == InstallScope::Workspace {
            // Write-if-absent for a `harness_owned` entry, and out of `expected_config`
            // either way: this is the workspace half of the same rule [`rendered`] applies
            // to the other two scopes, written here because this is where a
            // workspace-scoped file is written. No shipped connector declares the
            // combination — Codex's `config.toml` is `harness_home` — and the day one does,
            // it must not be silently owned by the tracker in one scope and not the others.
            if file.harness_owned {
                if !file.absolute_path.exists() {
                    write_file(&file.absolute_path, &file.contents, file.mode)?;
                }
                continue;
            }
            // The file's own mode, not `DEFAULT_MODE` with a `chmod` to follow: a
            // workspace-scoped file that carries the bearer must never exist at a wider
            // mode, not even for the microsecond between the write and the correction.
            write_file(&file.absolute_path, &file.contents, file.mode)?;
            expected_config.insert(file.relative_path.clone(), file.sha256.clone());
        }
    }

    // Taken after the tracker's own files are in place and with the union that excludes
    // them, so an empty `files` list is the zero-entry proof and not an artefact of when
    // the walk happened to run.
    let baseline = list_workspace(&roots.workspace, entry_id, &crate::now_rfc3339(), excluded)?;

    Ok(Workspace {
        root: roots.workspace.clone(),
        trace_dir: artifact_dir.join("trace"),
        artifact_dir,
        harness_home: roots.harness_home.clone(),
        expected_config,
        installed: files.clone(),
        baseline,
        allowlist: Vec::new(),
    })
}

/// The whole of workspace setup in one call: render every tracker file, create the
/// workspace, write the workspace-scoped ones with the modes the manifest declares, record
/// the expected hashes, the zero-entry baseline and the app-wide allowlist.
///
/// Fixture-only lexical setup. Production fresh setup must use
/// [`prepare_with_artifact`], which roots the token-bearing artifact write in the retained
/// entry capability.
#[cfg(any(test, feature = "test-hooks"))]
pub fn prepare(
    manifest: &ConnectorManifest,
    connector_dir: &Path,
    vars: &TemplateVars,
    path_vars: &crate::paths::PathVars,
    roots: &Roots,
    entry_id: &str,
) -> Result<Workspace, DaemonError> {
    prepare_inner(
        manifest,
        connector_dir,
        vars,
        path_vars,
        roots,
        entry_id,
        None,
    )
}

/// Production fresh-entry setup uses the artifact capability created from the configured
/// root. It is intentionally a separate entry point so fixture-only callers remain able to
/// build lexical scratch trees without making a production pathname write look legitimate.
pub fn prepare_with_artifact(
    manifest: &ConnectorManifest,
    connector_dir: &Path,
    vars: &TemplateVars,
    path_vars: &crate::paths::PathVars,
    roots: &Roots,
    entry_id: &str,
    artifact: &EntryArtifactRoot,
) -> Result<Workspace, DaemonError> {
    prepare_inner(
        manifest,
        connector_dir,
        vars,
        path_vars,
        roots,
        entry_id,
        Some(artifact),
    )
}

fn prepare_inner(
    manifest: &ConnectorManifest,
    connector_dir: &Path,
    vars: &TemplateVars,
    path_vars: &crate::paths::PathVars,
    roots: &Roots,
    entry_id: &str,
    artifact: Option<&EntryArtifactRoot>,
) -> Result<Workspace, DaemonError> {
    // Every refusal runs before the first write, not just the placement one.
    // `render_tracker_files` writes the artifact-root-scoped files immediately, and those
    // files can carry the run's bearer; a workspace refused after that point would leave a
    // live bearer on disk for a run that never starts. So every refusal is asked here, and
    // `create_workspace` asks it again below for the caller that comes in directly.
    check_workspace_available(roots)?;
    let files = rendered(manifest, connector_dir, vars, roots, false, artifact)?;

    let excluded = exclusion_union(manifest, &files);
    let mut created = create_workspace(roots, entry_id, &files, &excluded)?;

    // No second pass over the modes: `TrackerFile` carries its own, and `create_workspace`
    // wrote each file at it. The app-wide allowlist is hashed last, so the hashes are as close
    // to launch as the daemon can put them: everything above this line has already been
    // written to disk.
    created.allowlist = baseline_allowlist(manifest, path_vars)?;
    Ok(created)
}
