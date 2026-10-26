//! The workspace checks and the workspace walk a capture entry uses: is the folder
//! available (R3), and what does it hold.
//!
//! The V1 half of this module rendered and wrote every `hook_install[]` entry and hashed
//! the `config_allowlist[]` files at launch; D48 removed its caller and OS-31 removed it.
//!
//! Nothing here interprets a rule. Every comparison below is against a constant a manifest
//! carries or a hash of bytes on disk; what a mismatch *means* is `goalcup-verifier`'s.

use crate::DaemonError;
use goalcup_protocol::seal::{GitState, SealManifest, SealedFile, WalkCap, is_excluded};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The 64-zero digest that stands for "this file did not exist when we looked".
///
/// What [`list_workspace`] records for a file it could not open or read through, and beside the
/// size of a file it does not read: one over [`HASH_LIMIT`], or a key file ([`is_key_file`]). The
/// CLI's `seal` mirror spells the same sentinel, so the two walks agree about such a file.
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
            "the project folder {} is next to goalcup's own folder {}, and the agent lists the \
             folder above its project on its first tool call; use a folder of its own",
            workspace.display(),
            artifact_root.display()
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

/// The machine-wide roots against the workspace (review R4 I5): `check_artifact_placement`
/// compares it with this entry's own artifact directory only, which a workspace inside the
/// artifact root's other subtrees, or inside the configuration directory (`control.json` and
/// its bearer), passes. Refused: the workspace equal to, inside, or an ancestor of a root
/// (symlinks resolved), or its sibling, since an agent enumerates its parent first.
pub fn check_machine_roots(workspace: &Path, roots: &[(&str, &Path)]) -> Result<(), DaemonError> {
    for (name, root) in roots {
        if crate::config::nests(workspace, root) || workspace.parent() == root.parent() {
            return Err(DaemonError::Guard(format!(
                "workspace {} is nested in, or beside, the tracker's {name} {}: the agent would \
                 enumerate the tracker's own files",
                workspace.display(),
                root.display()
            )));
        }
    }
    Ok(())
}

/// The clone rule (D56), one definition shared with `goalcup continue`: a `.git` directory, not a
/// file or a link, and no work tree above; a workspace that is itself a link is followed.
pub use goalcup_platform::clone::{CloneRoot, clone_root, git_worktree_of, is_clone_root};

/// The most files the walk of a workspace that is not its own clone lists (D71). Past it the
/// manifest says [`WalkCap::FileLimit`] and the seal goes on.
pub const WORKSPACE_FILE_LIMIT: usize = 50_000;

/// Walk `workspace`, applying [`is_excluded`] with `excluded`, and return a sorted
/// manifest. One walker for the baseline and for the seal, so the zero-entry proof and the
/// seal can never disagree about what was left out.
///
/// The exclusion rule is root-anchored: a pattern matches a path that equals it or lies
/// beneath it, and nothing else. `mkdir src/.git` therefore hides nothing — a nested `.git`
/// is the agent's work until a connector writes `**/.git` and means it.
///
/// Bounded (D71): at most [`WORKSPACE_FILE_LIMIT`] files, a file over [`HASH_LIMIT`] by its size,
/// a key file ([`is_key_file`]) by its size, and nothing read past `deadline`. A walk that stops
/// short records why in `capped`; it is never an error.
pub fn list_workspace(
    workspace: &Path,
    entry_id: &str,
    at: &str,
    excluded: &[String],
    deadline: Option<std::time::Instant>,
) -> Result<SealManifest, DaemonError> {
    let walked = walk(
        workspace,
        excluded,
        Bounds {
            limit: WORKSPACE_FILE_LIMIT,
            hash_limit: HASH_LIMIT,
            deadline,
            keys_unhashed: true,
        },
    )?;
    Ok(SealManifest {
        entry_id: entry_id.to_string(),
        sealed_at: at.to_string(),
        workspace: workspace.to_string_lossy().to_string(),
        files: walked.files,
        excluded: excluded.to_vec(),
        git: None,
        home: None,
        capped: walked.capped,
    }
    .sorted())
}

/// The workspace's baseline or seal. Whether it is a clone is asked of the workspace as it is
/// now ([`is_clone_root`]), at the create and again at the finish (D71 withdraws D56's answer
/// fixed at the create): an agent that ran `git init` is sealed by its Git state, and one whose
/// `.git` is gone is walked. A clone is never walked: its Git state, read within `budget`, or why
/// it could not be read, at most [`UNREADABLE_LIMIT`] characters. Any other workspace is walked
/// by [`list_workspace`], with `budget` as its deadline.
pub fn seal_workspace(
    workspace: &Path,
    entry_id: &str,
    at: &str,
    budget: std::time::Duration,
) -> Result<SealManifest, DaemonError> {
    if !is_clone_root(workspace) {
        let deadline = std::time::Instant::now().checked_add(budget);
        return list_workspace(workspace, entry_id, at, &[], deadline);
    }
    let git = match crate::workspace_git::state(workspace, budget) {
        Ok(git) => GitState::Read(git),
        Err(why) => GitState::Unreadable {
            unreadable: why.chars().take(UNREADABLE_LIMIT).collect(),
        },
    };
    Ok(SealManifest {
        entry_id: entry_id.to_string(),
        sealed_at: at.to_string(),
        workspace: workspace.to_string_lossy().to_string(),
        files: Vec::new(),
        excluded: Vec::new(),
        git: Some(git),
        home: None,
        capped: None,
    })
}

/// The most characters of the reason an unreadable clone's manifest records.
const UNREADABLE_LIMIT: usize = 200;

/// The most files the baseline lists of the harness home (D56).
pub const HOME_FILE_LIMIT: usize = 10_000;

/// A file larger than this is described by its size alone: in the Git digest, in the
/// baseline's harness-home listing and in a workspace walk, where it carries the absent digest
/// beside its size (D56, D71).
pub(crate) const HASH_LIMIT: u64 = 64 * 1024 * 1024;

/// The connector's config home for the baseline (D56): every file the walk meets, without
/// `excluded` (its native roots), symlinks unfollowed, stopping at [`HOME_FILE_LIMIT`] files,
/// sorted. A file over [`HASH_LIMIT`] is not read. `None` when the home cannot be read.
pub fn list_home(home: &Path, excluded: &[String]) -> Option<Vec<SealedFile>> {
    let mut files = walk(
        home,
        excluded,
        Bounds {
            limit: HOME_FILE_LIMIT,
            hash_limit: HASH_LIMIT,
            deadline: None,
            keys_unhashed: false,
        },
    )
    .ok()?
    .files;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Some(files)
}

/// What one [`walk`] may take.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Bounds {
    /// The most files listed. One more found makes the walk [`WalkCap::FileLimit`].
    pub(crate) limit: usize,
    /// A regular file over this many bytes is listed with its size and the absent digest, unread.
    pub(crate) hash_limit: u64,
    /// Checked before each entry: past it the walk stops as [`WalkCap::Deadline`].
    pub(crate) deadline: Option<std::time::Instant>,
    /// A key file ([`is_key_file`]) is listed with its size and the absent digest, unread: a
    /// digest of a low-entropy secret file would be a confirmation oracle for it (D71; the
    /// redactor's own rule, `redact.rs`). The harness home keeps D61's hashed `.env`.
    pub(crate) keys_unhashed: bool,
}

/// What a [`walk`] found, in walk order, and why it stopped short, if it did.
#[derive(Debug)]
pub(crate) struct Walked {
    pub(crate) files: Vec<SealedFile>,
    pub(crate) capped: Option<WalkCap>,
}

/// Every file below `workspace` the exclusions leave, within `bounds`, in walk order.
pub(crate) fn walk(
    workspace: &Path,
    excluded: &[String],
    bounds: Bounds,
) -> Result<Walked, DaemonError> {
    let mut walked = Walked {
        files: Vec::new(),
        capped: None,
    };
    // Lists `file`, or, with the limit reached, records that the walk is capped and says stop.
    let list = |walked: &mut Walked, file: SealedFile| {
        if walked.files.len() >= bounds.limit {
            walked.capped = Some(WalkCap::FileLimit);
            return false;
        }
        walked.files.push(file);
        true
    };
    let mut stack = vec![workspace.to_path_buf()];
    'walk: while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            // The workspace itself must be readable: without it there is nothing to seal.
            Err(source) if dir == workspace => {
                return Err(DaemonError::Io {
                    path: dir.display().to_string(),
                    source,
                });
            }
            // A directory the agent made unreadable (`chmod 000`) is listed, with the absent
            // digest, and the walk goes on: failing here would fail every retry of the seal.
            // What it holds is not listed: an absent digest on a directory means "contents
            // unseen" (CHEAT-CHECKLIST).
            Err(_) => {
                let unseen = SealedFile {
                    path: relative_to(workspace, &dir),
                    bytes: 0,
                    sha256: ABSENT.to_string(),
                };
                if !list(&mut walked, unseen) {
                    break 'walk;
                }
                continue;
            }
        };
        for entry in entries {
            let Ok(entry) = entry else {
                // The listing failed part-way: the directory is listed with the absent digest
                // beside whatever was read, and the walk leaves it.
                let unseen = SealedFile {
                    path: relative_to(workspace, &dir),
                    bytes: 0,
                    sha256: ABSENT.to_string(),
                };
                if !list(&mut walked, unseen) {
                    break 'walk;
                }
                break;
            };
            let path = entry.path();
            let rel = relative_to(workspace, &path);
            if is_excluded(&rel, excluded) {
                continue;
            }
            // `file_type()` comes from the directory read and does not follow a symlink,
            // so a link pointing at an ancestor cannot make this walk loop.
            let Ok(kind) = entry.file_type() else {
                let unseen = SealedFile {
                    path: rel,
                    bytes: 0,
                    sha256: ABSENT.to_string(),
                };
                if !list(&mut walked, unseen) {
                    break 'walk;
                }
                continue;
            };
            // Before each entry, a folder too: a tree of empty folders must not outlast it.
            if bounds
                .deadline
                .is_some_and(|end| std::time::Instant::now() >= end)
            {
                walked.capped = Some(WalkCap::Deadline);
                break 'walk;
            }
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            let (bytes, sha256) =
                if bounds.keys_unhashed && is_key_file(&entry.file_name().to_string_lossy()) {
                    (measure(&path, 0).0, ABSENT.to_string())
                } else {
                    measure(&path, bounds.hash_limit)
                };
            let file = SealedFile {
                path: rel,
                bytes,
                sha256,
            };
            if !list(&mut walked, file) {
                break 'walk;
            }
        }
    }
    Ok(walked)
}

/// A workspace-relative path with forward slashes.
fn relative_to(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Length and digest, streamed, so a large workspace file is never held whole (bounded
/// memory). Only a regular file is read. Anything else — a symlink (never followed, so a link
/// to `/dev/zero` or to a file outside the workspace is not read), a FIFO (opened without
/// blocking, never read, so an agent's `mkfifo` cannot wedge the seal), a socket, a device, a
/// file whose permissions changed under the walk — is still listed, with the absent digest:
/// dropping it would make the seal quieter than the workspace it describes.
///
/// A regular file over `hash_limit` bytes is not read: its size, with the absent digest.
pub(crate) fn measure(path: &Path, hash_limit: u64) -> (u64, String) {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let Ok(mut file) = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
    else {
        return (0, ABSENT.to_string());
    };
    let Ok(meta) = file.metadata() else {
        return (0, ABSENT.to_string());
    };
    if !meta.file_type().is_file() {
        return (0, ABSENT.to_string());
    }
    if meta.len() > hash_limit {
        return (meta.len(), ABSENT.to_string());
    }
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

/// A key file: `.env` or `.env.<anything>`. The workspace walk lists it by name and size and
/// never hashes it (D71).
pub(crate) fn is_key_file(name: &str) -> bool {
    name == ".env" || name.starts_with(".env.")
}

/// A name a new run's folder may hold before the run, as a regular file (OD6): the
/// participant's keys ([`is_key_file`]) and a `.gitignore` that keeps them out of the repository.
fn is_starting_kit(name: &str) -> bool {
    name == ".gitignore" || is_key_file(name)
}

/// Every reason a workspace is refused, in one place, and **without writing anything**.
///
/// It creates no directory, not even the workspace: the capture runtime asks "would this be
/// refused?" before it creates the entry's artifact directory.
///
/// Always: an artifact root the agent could enumerate. For a new run (`continued` false), also
/// (D71): a folder that already holds `.git` (B1: the agent makes the repository during the
/// run, so a prepared one is refused), a folder inside a Git work tree, and a folder holding
/// anything but the starting kit, regular files only ("the workspace started empty" as a
/// checkable fact). A folder that does not exist yet holds nothing; the baseline lists the kit
/// by name and size. A continued run's folder (`continued`) holds the run's work, with or
/// without `.git`, and is admitted as it is; that it is there is checked by the caller.
pub fn check_workspace_available(roots: &Roots, continued: bool) -> Result<(), DaemonError> {
    check_artifact_placement(&roots.artifact_root, &roots.workspace)?;
    if continued {
        return Ok(());
    }
    if roots.workspace.join(".git").symlink_metadata().is_ok() {
        return Err(DaemonError::Guard(format!(
            "the run's folder {} already holds .git: a new run starts in a folder with no Git \
             repository, and the agent makes one during the run",
            roots.workspace.display()
        )));
    }
    if let Some(repo) = git_worktree_of(&roots.workspace) {
        return Err(DaemonError::Guard(format!(
            "the run's folder {} is inside the Git repository {}; it must be outside any \
             repository",
            roots.workspace.display(),
            repo.display()
        )));
    }
    let existing = match std::fs::read_dir(&roots.workspace) {
        Ok(entries) => entries
            .filter(|entry| {
                // A kit name that is a folder, a link or anything but a regular file is not kit.
                !entry.as_ref().is_ok_and(|e| {
                    is_starting_kit(&e.file_name().to_string_lossy())
                        && e.file_type().is_ok_and(|kind| kind.is_file())
                })
            })
            .count(),
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
            "the run's folder {} is not empty ({existing} entries); it may hold only .gitignore \
             and .env files",
            roots.workspace.display()
        )));
    }
    Ok(())
}
