//! The workspace checks and the workspace walk a capture entry uses: is the folder
//! available (R3), and what does it hold.
//!
//! The V1 half of this module rendered and wrote every `hook_install[]` entry and hashed
//! the `config_allowlist[]` files at launch; D48 removed its caller and OS-31 removed it.
//!
//! Nothing here interprets a rule. Every comparison below is against a constant a manifest
//! carries or a hash of bytes on disk; what a mismatch *means* is `goalcup-verifier`'s.

use crate::DaemonError;
use goalcup_protocol::seal::{SealManifest, SealedFile, is_excluded};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The 64-zero digest that stands for "this file did not exist when we looked".
///
/// What [`list_workspace`] records for a file it could not open or read through. The CLI's
/// `seal` mirror spells the same sentinel, so the two walks agree about such a file.
pub(crate) const ABSENT: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Bytes read per hashing step. A workspace file can be arbitrarily large and bounded
/// memory is a constraint on the whole daemon, so the seal walk never holds one whole.
const HASH_CHUNK: usize = 64 * 1024;

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// The workspace and this entry's artifact directory, which [`check_workspace_available`]
/// checks against each other.
///
/// `artifact_root` is **this entry's** artifact directory, not the machine-wide one.
#[derive(Debug, Clone)]
pub struct Roots {
    pub workspace: PathBuf,
    pub artifact_root: PathBuf,
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

/// Every reason a workspace is refused, in one place, and **without writing anything**.
///
/// An artifact root the agent could enumerate, a directory inside a Git worktree, and a
/// directory that already has entries — "the workspace started empty" as a checkable fact
/// rather than a claim.
///
/// It creates no directory, not even the workspace: the capture runtime asks "would this be
/// refused?" before it creates the entry's artifact directory.
///
/// A workspace that does not exist yet has zero entries; that is the normal case.
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
