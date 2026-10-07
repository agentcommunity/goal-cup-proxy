//! The three file mechanics every writer in this crate shares: a mode, a private atomic
//! write, and the sweep that clears what a killed write left behind.
//!
//! Nothing here decides policy. Every knob a caller has an opinion about — the mode, the
//! temp file's name, whether a non-regular target is a refusal, whether the parent
//! directory is created — is an argument, because the call sites each have a different,
//! documented answer and sharing the mechanics must not quietly share the answers as well.
//! What is shared is the sequence: create a fresh temp file in the target's own directory,
//! write it, `fsync` it, and `rename` it over the target.
//!
//! # Why a temp file and a rename, every time
//!
//! A reader that opens a file between a truncate and its last byte reads half a document,
//! and half a signed document does not verify. `rename` within one directory is atomic, so
//! every reader sees either the old document whole or the new one whole. The bytes are
//! `fsync`ed before the rename because otherwise the rename can be durable while its
//! contents are not, and a reader after a crash finds a file that is present, named and
//! empty.
//!
//! # Why `O_EXCL` on the temp file, and why the mode is carried on the create
//!
//! `create_new` refuses a temp file that is already there rather than joining it: two
//! writers must not publish each other's bytes, and a symlink planted at the temp path is
//! refused by the same flag. The mode rides on the create (`OpenOptionsExt::mode`) so the
//! file is never briefly readable by anyone it should not be — the open file is set to the
//! caller's exact mode after the bytes are synced and before the rename, so a restrictive
//! process umask cannot silently narrow the published mode without introducing a pathname
//! reopen or a window while the file is being written.
//!
//! # Why the temp of our own name is removed first
//!
//! `O_EXCL` on its own would turn one killed write into a permanent refusal. Every temp name
//! this module mints carries this process's pid, so a file sitting at that exact name is
//! either this process's own leavings or a dead process's whose pid the operating system has
//! since handed back to us — and a daemon killed between the create and the rename leaves
//! exactly that. `status-line.txt` is the case that bites: it lives in the artifact root,
//! which [`sweep_process_temp`] does not cover, so the next daemon to draw that pid would be
//! refused on every status refresh, forever, by a file it could not name.
//!
//! So [`write_atomic`] removes a file at its own temp name before it creates one. It removes
//! that one exact name and never a pattern; the removal does not follow a link, so a symlink
//! planted at the name is unlinked rather than written through; and `create_new` still
//! stands behind it, so a name taken by a second process in the instant after the removal is
//! a refusal rather than a join.
//!
//! `O_EXCL` also refuses our own other thread, and a removal would undo that.
//! [`process_temp`] hands the same name to every write of one target in this process, and a
//! removal in front of the create would be one thread deleting the file the other is in the
//! middle of writing, after which the loser renames a half-written document over a target
//! readers poll. So a write claims its temp name for the length of the write ([`TempClaim`])
//! and the removal happens under that claim: an unclaimed name can only hold a dead
//! process's leavings, and a claimed one is refused exactly as the create would refuse it.

use crate::DaemonError;
#[cfg(unix)]
#[cfg(feature = "v1-compat")]
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};
#[cfg(unix)]
#[cfg(feature = "v1-compat")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(all(feature = "v1-compat", any(test, feature = "test-hooks")))]
use std::sync::{Mutex, OnceLock};

#[cfg(all(feature = "v1-compat", any(test, feature = "test-hooks")))]
static RESTORE_ANCESTOR_OPEN_FAILURES: OnceLock<
    Mutex<std::collections::BTreeMap<PathBuf, std::io::ErrorKind>>,
> = OnceLock::new();

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
static RESTORE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(all(feature = "v1-compat", any(test, feature = "test-hooks")))]
pub(crate) fn fail_next_restore_ancestor_open(path: PathBuf, kind: std::io::ErrorKind) {
    RESTORE_ANCESTOR_OPEN_FAILURES
        .get_or_init(|| Mutex::new(std::collections::BTreeMap::new()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(path, kind);
}

#[cfg(all(feature = "v1-compat", any(test, feature = "test-hooks")))]
fn injected_restore_ancestor_open_failure(root: &Path, relative: &str) -> Option<std::io::Error> {
    RESTORE_ANCESTOR_OPEN_FAILURES
        .get_or_init(|| Mutex::new(std::collections::BTreeMap::new()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(&root.join(relative))
        .map(std::io::Error::from)
}

/// What a caller will accept finding at the target path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetGuard {
    /// Refuse anything that is not a regular file — a symlink above all.
    ///
    /// `control.json` carries the CLI's bearer into a directory the participant can reach,
    /// so a link planted at that name would take the bearer and write it through to a file
    /// of the planter's choosing, at whatever mode that file already had. The check is
    /// `symlink_metadata`, which does not follow: the question is what is at the path, not
    /// what the path leads to, and a dangling link is exactly the plant that a "does it
    /// exist?" test waves through.
    RegularFileOnly,
    /// Replace whatever is there. The rename is still atomic and the temp file is still
    /// fresh; this only says the caller has no opinion about what it is replacing.
    Any,
}

/// Write `bytes` to `path` through `temp`, at `mode`, atomically.
///
/// `temp` must sit in the same directory as `path` — a rename across filesystems is not
/// atomic — and naming it is the caller's, because the two naming schemes in this crate
/// mean different things: a per-process name is one a startup sweep can find again, and a
/// per-write name is what lets two threads publish to one shared target at once.
///
/// `temp` must also carry this process's pid, which is what [`process_temp`] and
/// [`unique_temp`] are for and the reason this function may remove a file already at that
/// name. A caller that invents a name of its own breaks the reasoning in the module's "Why
/// the temp of our own name is removed first"; there is no such caller.
///
/// Every failure after the temp file exists removes it: a refused write leaves the
/// directory as it found it, or the next write is refused by this one's leavings.
pub(crate) fn write_atomic(
    path: &Path,
    temp: &Path,
    bytes: &[u8],
    mode: u32,
    guard: TargetGuard,
) -> Result<(), DaemonError> {
    if guard == TargetGuard::RegularFileOnly {
        match std::fs::symlink_metadata(path) {
            Ok(meta) if !meta.file_type().is_file() => {
                return Err(DaemonError::Refused(format!(
                    "{} is a {}, not a regular file: this daemon will not write through \
                     something it did not create",
                    path.display(),
                    describe(meta.file_type())
                )));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(DaemonError::Io {
                    path: path.display().to_string(),
                    source,
                });
            }
        }
    }

    let temp_named = |source: std::io::Error| DaemonError::Io {
        path: temp.display().to_string(),
        source,
    };
    // Held until this function returns, on every path: it is what makes the stale-temp
    // removal inside it safe, and what refuses our own second thread.
    let _claim = TempClaim::take(temp)?;
    let mut file = create_new(temp, mode).map_err(|source| match source.kind() {
        std::io::ErrorKind::AlreadyExists => DaemonError::Refused(format!(
            "{} already exists: another writer holds it, and this one will not join it",
            temp.display()
        )),
        _ => temp_named(source),
    })?;

    let written = {
        use std::io::Write;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .and_then(|()| set_open_file_mode(&file, mode))
            .map_err(temp_named)
            .and_then(|()| {
                std::fs::rename(temp, path).map_err(|source| DaemonError::Io {
                    path: path.display().to_string(),
                    source,
                })
            })
    };
    if written.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    written
}

/// The temp names a write in this process holds right now.
///
/// `BTreeSet` rather than a hash set only because its `new` is `const`, which is what lets
/// this be a plain `static` with no initialisation ceremony. It holds at most one entry per
/// write in flight, so it does not grow with the run.
static LIVE_TEMPS: std::sync::Mutex<std::collections::BTreeSet<PathBuf>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

/// One write's claim on its temp name, released when the write ends — including when it
/// ends in a refusal or a panic.
///
/// See the module's "Why the temp of our own name is removed first". The claim is what tells
/// a file at the temp name apart: unclaimed, it is what a killed process left behind and
/// this write removes it; claimed, another thread of ours is writing it and this write is
/// refused rather than joining.
struct TempClaim(PathBuf);

impl TempClaim {
    fn take(temp: &Path) -> Result<TempClaim, DaemonError> {
        let mut live = LIVE_TEMPS.lock().unwrap_or_else(|e| e.into_inner());
        if !live.insert(temp.to_path_buf()) {
            return Err(DaemonError::Refused(format!(
                "{} is held by another writer in this process, and this one will not join it",
                temp.display()
            )));
        }
        // Under the claim, so no thread can create the file between this removal and the
        // create that follows. Best effort and silent: a removal that fails leaves
        // `create_new` to refuse, in a message that names the file.
        let _ = std::fs::remove_file(temp);
        Ok(TempClaim(temp.to_path_buf()))
    }
}

impl Drop for TempClaim {
    fn drop(&mut self) {
        LIVE_TEMPS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

/// `O_CREAT|O_EXCL` at `mode`.
#[cfg(unix)]
fn create_new(temp: &Path, mode: u32) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(temp)
}

/// The same create where there is no Unix mode to carry. It deliberately does not pretend
/// the file is protected.
#[cfg(not(unix))]
fn create_new(temp: &Path, _mode: u32) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp)
}

/// Apply the caller's exact mode through the open descriptor before the temp is published.
///
/// The create mode is still important: a restrictive umask may make the initial inode more
/// private than requested, but it must not make a public status document permanently private.
/// Using the descriptor avoids reopening the temp pathname between the write and rename.
#[cfg(unix)]
fn set_open_file_mode(file: &std::fs::File, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_open_file_mode(_file: &std::fs::File, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

/// The temp name a startup sweep can find again: `<name>.<pid>.tmp`, beside the target.
///
/// One name per process per target, which is what makes [`sweep_process_temp`] able to name
/// the exact file a killed write left behind rather than guessing at a pattern.
pub(crate) fn process_temp(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "tmp".to_string());
    path.with_file_name(format!("{name}.{}.tmp", std::process::id()))
}

/// A temp name no other write anywhere can pick: `<name>.<pid>.<counter>.tmp`.
///
/// Needed wherever two writers share one target. `status-latest.json` is the same file for
/// every entry in the process, so a name derived from the target alone is one name two
/// mirrors both write and both rename — and the loser publishes the winner's bytes under
/// its own rename, or fails on a file the other thread already moved away.
pub(crate) fn unique_temp(path: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "tmp".to_string());
    path.with_file_name(format!(
        "{name}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Remove the one temp file a process killed mid-write leaves behind, once, at startup.
///
/// [`process_temp`] is stable per process and [`write_atomic`] refuses a temp file that
/// already exists rather than joining it. Both are deliberate, and together they have one
/// bad case: a daemon killed between the create and the rename leaves the temp file on
/// disk, nothing else removes it, and the next daemon the operating system hands that pid
/// is refused at startup by a dead process's leavings.
///
/// Best effort, and silent. A temp file that cannot be removed is the write's refusal to
/// report, in a message that names the file, rather than a second refusal here saying the
/// same thing first.
pub(crate) fn sweep_process_temp(path: &Path) {
    let _ = std::fs::remove_file(process_temp(path));
}

/// What was found where a guarded write was going, for the refusal to name.
fn describe(kind: std::fs::FileType) -> &'static str {
    match kind {
        k if k.is_symlink() => "symbolic link",
        k if k.is_dir() => "directory",
        _ => "special file",
    }
}

#[cfg(unix)]
pub(crate) fn set_mode(path: &Path, mode: u32) -> Result<(), DaemonError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(|source| {
        DaemonError::Io {
            path: path.display().to_string(),
            source,
        }
    })
}

#[cfg(not(unix))]
pub(crate) fn set_mode(_path: &Path, _mode: u32) -> Result<(), DaemonError> {
    Ok(())
}

/// Open one tracker-owned regular file by walking from an already-approved root. This is
/// deliberately Unix-only: a platform without descriptor-relative no-follow traversal must
/// refuse this security-sensitive operation rather than quietly reintroduce pathname races.
#[cfg(feature = "v1-compat")]
#[cfg(unix)]
pub(crate) fn open_regular_at(root: &Path, relative: &str) -> Result<std::fs::File, DaemonError> {
    let (parent, leaf) = pinned_parent(root, relative, false)?;
    open_regular_at_fd(&parent, &leaf)
}

#[cfg(not(unix))]
pub(crate) fn open_regular_at(_root: &Path, _relative: &str) -> Result<std::fs::File, DaemonError> {
    Err(DaemonError::Guard(
        "descriptor-relative config observation is unsupported on this platform".to_string(),
    ))
}

/// Open one untrusted absolute leaf without following a final link or blocking on a special
/// file. Report-only allowlist paths have no tracker-owned parent to pin, but they still
/// must not let a participant-controlled FIFO stall the entry tick.
#[cfg(feature = "v1-compat")]
#[cfg(unix)]
pub(crate) fn open_regular_nofollow(path: &Path) -> Result<std::fs::File, DaemonError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let name = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        DaemonError::Refused(format!("{} contains an interior NUL", path.display()))
    })?;
    let fd = unsafe {
        libc::open(
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(DaemonError::Io {
            path: path.display().to_string(),
            source: std::io::Error::last_os_error(),
        });
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    if !file
        .metadata()
        .map_err(|source| DaemonError::Io {
            path: path.display().to_string(),
            source,
        })?
        .is_file()
    {
        return Err(DaemonError::Refused(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    Ok(file)
}

#[cfg(windows)]
pub(crate) fn open_regular_nofollow(path: &Path) -> Result<std::fs::File, DaemonError> {
    use std::os::windows::fs::OpenOptionsExt;

    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(0x0020_0000) // FILE_FLAG_OPEN_REPARSE_POINT
        .open(path)
        .map_err(|source| DaemonError::Io {
            path: path.display().to_string(),
            source,
        })?;
    if !file
        .metadata()
        .map_err(|source| DaemonError::Io {
            path: path.display().to_string(),
            source,
        })?
        .is_file()
    {
        return Err(DaemonError::Refused(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn open_regular_nofollow(path: &Path) -> Result<std::fs::File, DaemonError> {
    Err(DaemonError::Guard(format!(
        "no no-follow regular-file open is available for {}",
        path.display()
    )))
}

/// Restore through pinned root/ancestor descriptors. `after_pin` is test-only plumbing at
/// the exact point where a pathname implementation would be vulnerable to an ancestor
/// exchange; production callers pass a no-op.
#[cfg(feature = "v1-compat")]
#[cfg(unix)]
pub(crate) fn restore_regular_at<F>(
    root: &Path,
    relative: &str,
    bytes: &[u8],
    mode: u32,
    after_pin: F,
) -> Result<(), DaemonError>
where
    F: FnOnce(),
{
    let (parent, leaf) = pinned_parent(root, relative, true)?;
    after_pin();
    ensure_parent_is_current(root, relative, &parent)?;
    clear_non_regular_at(&parent, &leaf)?;
    replace_regular_at(&parent, &leaf, bytes, mode, root, relative)
}

#[cfg(not(unix))]
pub(crate) fn restore_regular_at<F>(
    _root: &Path,
    _relative: &str,
    _bytes: &[u8],
    _mode: u32,
    _after_pin: F,
) -> Result<(), DaemonError>
where
    F: FnOnce(),
{
    Err(DaemonError::Guard(
        "descriptor-relative config restore is unsupported on this platform".to_string(),
    ))
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
pub(crate) fn set_mode_at<F>(
    root: &Path,
    relative: &str,
    bytes: &[u8],
    mode: u32,
    after_pin: F,
) -> Result<(), DaemonError>
where
    F: FnOnce(),
{
    let (parent, leaf) = pinned_parent(root, relative, false)?;
    after_pin();
    ensure_parent_is_current(root, relative, &parent)?;
    clear_non_regular_at(&parent, &leaf)?;
    replace_regular_at(&parent, &leaf, bytes, mode, root, relative)
}

#[cfg(not(unix))]
pub(crate) fn set_mode_at<F>(
    _root: &Path,
    _relative: &str,
    _bytes: &[u8],
    _mode: u32,
    _after_pin: F,
) -> Result<(), DaemonError>
where
    F: FnOnce(),
{
    Err(DaemonError::Guard(
        "descriptor-relative config chmod is unsupported on this platform".to_string(),
    ))
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn pinned_parent(
    root: &Path,
    relative: &str,
    create: bool,
) -> Result<(std::fs::File, std::ffi::OsString), DaemonError> {
    let parts = relative_parts(relative)?;
    let leaf = parts
        .last()
        .cloned()
        .ok_or_else(|| DaemonError::Guard("tracker relative path has no leaf".to_string()))?;
    let mut parent = open_root(root)?;
    for part in &parts[..parts.len() - 1] {
        parent = open_or_create_dir_at(&parent, part, create, root, relative)?;
    }
    Ok((parent, leaf))
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn relative_parts(relative: &str) -> Result<Vec<std::ffi::OsString>, DaemonError> {
    use std::path::Component;
    let path = Path::new(relative);
    if path.is_absolute() {
        return Err(DaemonError::Guard(format!(
            "refusing absolute tracker path {relative}"
        )));
    }
    let mut out = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => out.push(part.to_os_string()),
            _ => {
                return Err(DaemonError::Guard(format!(
                    "refusing non-normal tracker path {relative}"
                )));
            }
        }
    }
    (!out.is_empty())
        .then_some(out)
        .ok_or_else(|| DaemonError::Guard("empty tracker relative path".to_string()))
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn open_root(root: &Path) -> Result<std::fs::File, DaemonError> {
    let path = c_path(root.as_os_str())?;
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY
                | libc::O_DIRECTORY
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io_error(
            root.display().to_string(),
            std::io::Error::last_os_error(),
        ));
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    if !file
        .metadata()
        .map_err(|source| io_error(root.display().to_string(), source))?
        .is_dir()
    {
        return Err(DaemonError::Guard(format!(
            "{} is not an approved directory",
            root.display()
        )));
    }
    Ok(file)
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn open_or_create_dir_at(
    parent: &std::fs::File,
    part: &std::ffi::OsStr,
    create: bool,
    root: &Path,
    relative: &str,
) -> Result<std::fs::File, DaemonError> {
    #[cfg(any(test, feature = "test-hooks"))]
    let opened = if create {
        injected_restore_ancestor_open_failure(root, relative)
            .map_or_else(|| open_dir_at(parent, part), Err)
    } else {
        open_dir_at(parent, part)
    };
    #[cfg(not(any(test, feature = "test-hooks")))]
    let opened = open_dir_at(parent, part);

    match opened {
        Ok(file) => Ok(file),
        Err(source) if create && source.kind() == std::io::ErrorKind::NotFound => {
            mkdir_at(parent, part)?;
            open_dir_at(parent, part).map_err(|e| io_error(display_at(root, relative), e))
        }
        Err(source) if create => match entry_kind_at(parent, part)? {
            EntryKindAt::NonDirectory => {
                // A regular file, symlink or special file is a recoverable obstruction. The
                // zero flags matter: if it becomes a directory after `fstatat`, `unlinkat`
                // refuses it instead of recursively clearing participant data.
                unlink_at(parent, part)
                    .map_err(|error| io_error(display_at(root, relative), error))?;
                mkdir_at(parent, part)?;
                open_dir_at(parent, part).map_err(|e| io_error(display_at(root, relative), e))
            }
            // An otherwise valid directory that could not be opened, or an entry that
            // changed while being classified, is not evidence of an obstruction. Surface
            // the failed restore and leave the whole subtree alone.
            EntryKindAt::Directory | EntryKindAt::Missing => {
                Err(io_error(display_at(root, relative), source))
            }
        },
        Err(source) => Err(io_error(display_at(root, relative), source)),
    }
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
enum EntryKindAt {
    Missing,
    Directory,
    NonDirectory,
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn entry_kind_at(
    parent: &std::fs::File,
    name: &std::ffi::OsStr,
) -> Result<EntryKindAt, DaemonError> {
    let name = c_path(name)?;
    let mut stat = unsafe { std::mem::zeroed::<libc::stat>() };
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            &mut stat,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } < 0
    {
        let source = std::io::Error::last_os_error();
        if source.kind() == std::io::ErrorKind::NotFound {
            return Ok(EntryKindAt::Missing);
        }
        return Err(io_error("tracker ancestor".to_string(), source));
    }
    if (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR {
        Ok(EntryKindAt::Directory)
    } else {
        Ok(EntryKindAt::NonDirectory)
    }
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn ensure_parent_is_current(
    root: &Path,
    relative: &str,
    pinned: &std::fs::File,
) -> Result<(), DaemonError> {
    use std::os::unix::fs::MetadataExt;
    let parts = relative_parts(relative)?;
    let mut current = open_root(root)?;
    for part in &parts[..parts.len() - 1] {
        current = open_dir_at(&current, part)
            .map_err(|source| io_error(display_at(root, relative), source))?;
    }
    let observed = current
        .metadata()
        .map_err(|source| io_error(display_at(root, relative), source))?;
    let expected = pinned
        .metadata()
        .map_err(|source| io_error(display_at(root, relative), source))?;
    if observed.dev() != expected.dev() || observed.ino() != expected.ino() {
        return Err(DaemonError::Guard(format!(
            "refusing restore of {}: an approved ancestor changed after pinning",
            display_at(root, relative)
        )));
    }
    Ok(())
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn open_dir_at(parent: &std::fs::File, part: &std::ffi::OsStr) -> std::io::Result<std::fs::File> {
    let part = c_path(part).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in directory name")
    })?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            part.as_ptr(),
            libc::O_RDONLY
                | libc::O_DIRECTORY
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn open_regular_at_fd(
    parent: &std::fs::File,
    leaf: &std::ffi::OsStr,
) -> Result<std::fs::File, DaemonError> {
    let leaf = c_path(leaf)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io_error(
            "tracker file".to_string(),
            std::io::Error::last_os_error(),
        ));
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    if !file
        .metadata()
        .map_err(|e| io_error("tracker file".to_string(), e))?
        .is_file()
    {
        return Err(DaemonError::Guard(
            "tracker path is not a regular file".to_string(),
        ));
    }
    Ok(file)
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn replace_regular_at(
    parent: &std::fs::File,
    leaf: &std::ffi::OsStr,
    bytes: &[u8],
    mode: u32,
    root: &Path,
    relative: &str,
) -> Result<(), DaemonError> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    // Do not truncate or chmod an existing leaf: it could be a hard link to a file outside
    // the approved tree. A new `O_EXCL` leaf is private to this pinned directory until the
    // descriptor-relative rename replaces only this directory entry.
    for _ in 0..16 {
        let temp = std::ffi::OsString::from(format!(
            ".goalcup-restore-{}-{}",
            std::process::id(),
            RESTORE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = match create_private_at(parent, &temp, mode) {
            Ok(file) => file,
            Err(DaemonError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::AlreadyExists =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        let written = file
            .set_permissions(std::fs::Permissions::from_mode(mode))
            .and_then(|()| file.write_all(bytes))
            .and_then(|()| file.sync_all());
        drop(file);
        if let Err(source) = written {
            let _ = unlink_at(parent, &temp);
            return Err(io_error(display_at(root, relative), source));
        }
        let result = rename_at(parent, &temp, leaf);
        if result.is_err() {
            let _ = unlink_at(parent, &temp);
        }
        return result.map_err(|source| io_error(display_at(root, relative), source));
    }
    Err(DaemonError::Guard(format!(
        "could not allocate a private restore leaf for {}",
        display_at(root, relative)
    )))
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn create_private_at(
    parent: &std::fs::File,
    name: &std::ffi::OsStr,
    mode: u32,
) -> Result<std::fs::File, DaemonError> {
    let name = c_path(name)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY
                | libc::O_CREAT
                | libc::O_EXCL
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | libc::O_CLOEXEC,
            mode,
        )
    };
    if fd < 0 {
        return Err(io_error(
            "private tracker restore file".to_string(),
            std::io::Error::last_os_error(),
        ));
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    if !file
        .metadata()
        .map_err(|e| io_error("private tracker restore file".to_string(), e))?
        .is_file()
    {
        return Err(DaemonError::Guard(
            "private restore leaf is not a regular file".to_string(),
        ));
    }
    Ok(file)
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn rename_at(
    parent: &std::fs::File,
    from: &std::ffi::OsStr,
    to: &std::ffi::OsStr,
) -> std::io::Result<()> {
    let from = c_path(from).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in restore temp name")
    })?;
    let to = c_path(to).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in tracker leaf name")
    })?;
    if unsafe {
        libc::renameat(
            parent.as_raw_fd(),
            from.as_ptr(),
            parent.as_raw_fd(),
            to.as_ptr(),
        )
    } < 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn unlink_at(parent: &std::fs::File, name: &std::ffi::OsStr) -> std::io::Result<()> {
    let name = c_path(name).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in restore temp name")
    })?;
    if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) } < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn clear_non_regular_at(parent: &std::fs::File, leaf: &std::ffi::OsStr) -> Result<(), DaemonError> {
    let leaf_c = c_path(leaf)?;
    let mut stat = unsafe { std::mem::zeroed::<libc::stat>() };
    let rc = unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            leaf_c.as_ptr(),
            &mut stat,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if rc == 0 && (stat.st_mode & libc::S_IFMT) != libc::S_IFREG {
        remove_entry_at(parent, leaf)?;
    } else if rc != 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::NotFound {
        return Err(io_error(
            "tracker file".to_string(),
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn mkdir_at(parent: &std::fs::File, part: &std::ffi::OsStr) -> Result<(), DaemonError> {
    let part = c_path(part)?;
    let rc = unsafe { libc::mkdirat(parent.as_raw_fd(), part.as_ptr(), 0o700) };
    if rc == 0 || std::io::Error::last_os_error().kind() == std::io::ErrorKind::AlreadyExists {
        Ok(())
    } else {
        Err(io_error(
            "tracker directory".to_string(),
            std::io::Error::last_os_error(),
        ))
    }
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn remove_entry_at(parent: &std::fs::File, name: &std::ffi::OsStr) -> Result<(), DaemonError> {
    let name_c = c_path(name)?;
    let mut stat = unsafe { std::mem::zeroed::<libc::stat>() };
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name_c.as_ptr(),
            &mut stat,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } < 0
    {
        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            return Ok(());
        }
        return Err(io_error(
            "tracker path".to_string(),
            std::io::Error::last_os_error(),
        ));
    }
    if (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR {
        let dir =
            open_dir_at(parent, name).map_err(|e| io_error("tracker directory".to_string(), e))?;
        clear_dir(&dir)?;
        if unsafe { libc::unlinkat(parent.as_raw_fd(), name_c.as_ptr(), libc::AT_REMOVEDIR) } < 0 {
            return Err(io_error(
                "tracker directory".to_string(),
                std::io::Error::last_os_error(),
            ));
        }
    } else if unsafe { libc::unlinkat(parent.as_raw_fd(), name_c.as_ptr(), 0) } < 0 {
        return Err(io_error(
            "tracker path".to_string(),
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn clear_dir(dir: &std::fs::File) -> Result<(), DaemonError> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;
    let duplicate = unsafe { libc::dup(dir.as_raw_fd()) };
    if duplicate < 0 {
        return Err(io_error(
            "tracker directory".to_string(),
            std::io::Error::last_os_error(),
        ));
    }
    let stream = unsafe { libc::fdopendir(duplicate) };
    if stream.is_null() {
        unsafe {
            libc::close(duplicate);
        }
        return Err(io_error(
            "tracker directory".to_string(),
            std::io::Error::last_os_error(),
        ));
    }
    let result = (|| {
        loop {
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                break;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            if name.to_bytes() == b"." || name.to_bytes() == b".." {
                continue;
            }
            remove_entry_at(dir, std::ffi::OsStr::from_bytes(name.to_bytes()))?;
        }
        Ok(())
    })();
    unsafe {
        libc::closedir(stream);
    }
    result
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn c_path(value: &std::ffi::OsStr) -> Result<std::ffi::CString, DaemonError> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(value.as_bytes())
        .map_err(|_| DaemonError::Guard("NUL in filesystem path".to_string()))
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn io_error(path: String, source: std::io::Error) -> DaemonError {
    DaemonError::Io { path, source }
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
fn display_at(root: &Path, relative: &str) -> String {
    root.join(relative).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The killed write, and the daemon that draws its pid afterwards.
    ///
    /// A process killed between the create and the rename leaves its temp file on disk. The
    /// next daemon to be handed that pid mints the same name — `unique_temp`'s counter
    /// restarts at zero in a fresh process, and `process_temp` is the pid alone — and under
    /// `O_EXCL` on its own it would be refused by that file on every write it ever made.
    /// `status-line.txt` sits in the artifact root, which no sweep covers, so "every write"
    /// means every status refresh for the life of the run.
    #[test]
    fn a_stale_temp_of_our_own_name_does_not_refuse_the_write() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("status-line.txt");
        let temp = process_temp(&target);
        std::fs::write(&temp, b"half of what a killed write meant to say")?;

        let wrote = write_atomic(&target, &temp, b"fresh\n", 0o644, TargetGuard::Any);
        assert!(
            wrote.is_ok(),
            "a temp carrying our own pid refused the write: {wrote:?}"
        );

        assert_eq!(std::fs::read(&target)?, b"fresh\n");
        assert!(
            std::fs::symlink_metadata(&temp).is_err(),
            "{} outlived the write",
            temp.display()
        );
        Ok(())
    }

    /// The removal is the one name, never a pattern: a temp beside ours — another process's
    /// pid, or another target's — is not this write's to touch.
    #[test]
    fn a_temp_of_someone_else_s_name_is_left_alone() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("status-line.txt");
        let neighbour = dir.path().join("status-line.txt.999999.0.tmp");
        std::fs::write(&neighbour, b"not ours")?;

        let wrote = write_atomic(
            &target,
            &unique_temp(&target),
            b"fresh\n",
            0o644,
            TargetGuard::Any,
        );
        assert!(wrote.is_ok(), "{wrote:?}");
        assert_eq!(std::fs::read(&neighbour)?, b"not ours");
        Ok(())
    }

    /// The removal must not become one thread deleting the file another is writing.
    ///
    /// `process_temp` is one name for every write of one target in this process, and
    /// `tests/control_api.rs` drives two threads at `control.json` at once for exactly this
    /// reason. `O_EXCL` alone refuses the second writer; a removal in front of that create
    /// would let both through, and the loser would rename a half-written document over a
    /// target a reader polls. Held here as the claim it is.
    #[test]
    fn a_temp_another_thread_of_ours_holds_is_refused_and_not_removed() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("control.json");
        let temp = process_temp(&target);

        let held = TempClaim::take(&temp);
        assert!(held.is_ok(), "the first writer claims the name");
        std::fs::write(&temp, b"a write in progress")?;

        let second = write_atomic(&target, &temp, b"{}", 0o600, TargetGuard::Any);
        assert!(
            matches!(second, Err(DaemonError::Refused(_))),
            "a claimed temp must be refused, not joined: {second:?}"
        );
        assert_eq!(
            std::fs::read(&temp)?,
            b"a write in progress",
            "the refused write removed the file the claim holder was writing"
        );

        // And once the claim is dropped the name is writable again, so the refusal is a
        // retry and not a wedge.
        drop(held);
        let third = write_atomic(&target, &temp, b"{}", 0o600, TargetGuard::Any);
        assert!(third.is_ok(), "{third:?}");
        Ok(())
    }
}
