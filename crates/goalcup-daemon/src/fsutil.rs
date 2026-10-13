//! The three file mechanics every writer in this crate shares: a mode, a private atomic
//! write, and the sweep that clears what a killed write left behind.
//!
//! The mode and the temp file's name are arguments; a target that is not a regular file is
//! always refused. What is shared is the sequence: create a fresh temp file in the target's own directory,
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
//! exactly that. A startup sweep covers today's one target (`control.json`); the removal here
//! keeps a target no sweep covers from being refused on every write, forever, by a file the
//! next daemon to draw that pid could not name.
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
use std::path::{Path, PathBuf};

/// Write `bytes` to `path` through `temp`, at `mode`, atomically.
///
/// `temp` must sit in the same directory as `path` — a rename across filesystems is not
/// atomic — and naming it is the caller's: a per-process name is one a startup sweep can
/// find again.
///
/// `temp` must also carry this process's pid, which is what [`process_temp`] is for and the
/// reason this function may remove a file already at that
/// name. A caller that invents a name of its own breaks the reasoning in the module's "Why
/// the temp of our own name is removed first"; there is no such caller.
///
/// Every failure after the temp file exists removes it: a refused write leaves the
/// directory as it found it, or the next write is refused by this one's leavings.
///
/// Anything at `path` that is not a regular file is refused, a symlink above all.
/// `control.json` carries the CLI's bearer into a directory the participant can reach, so a
/// link planted at that name would take the bearer and write it through to a file of the
/// planter's choosing, at whatever mode that file already had. The check is
/// `symlink_metadata`, which does not follow: the question is what is at the path, not what
/// the path leads to, and a dangling link is exactly the plant that a "does it exist?" test
/// waves through.
pub(crate) fn write_atomic(
    path: &Path,
    temp: &Path,
    bytes: &[u8],
    mode: u32,
) -> Result<(), DaemonError> {
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

/// Apply the caller's exact mode through the open descriptor before the temp is published.
///
/// The create mode is still important: a restrictive umask may make the initial inode more
/// private than requested, but it must not narrow the mode the caller asked for for good.
/// Using the descriptor avoids reopening the temp pathname between the write and rename.
#[cfg(unix)]
fn set_open_file_mode(file: &std::fs::File, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(mode))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The killed write, and the daemon that draws its pid afterwards.
    ///
    /// A process killed between the create and the rename leaves its temp file on disk. The
    /// next daemon to be handed that pid mints the same name — `process_temp` is the pid
    /// alone — and under
    /// `O_EXCL` on its own it would be refused by that file on every write it ever made.
    #[test]
    fn a_stale_temp_of_our_own_name_does_not_refuse_the_write() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("control.json");
        let temp = process_temp(&target);
        std::fs::write(&temp, b"half of what a killed write meant to say")?;

        let wrote = write_atomic(&target, &temp, b"fresh\n", 0o644);
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
        let target = dir.path().join("control.json");
        let neighbour = dir.path().join("control.json.999999.0.tmp");
        std::fs::write(&neighbour, b"not ours")?;

        let wrote = write_atomic(&target, &process_temp(&target), b"fresh\n", 0o644);
        assert!(wrote.is_ok(), "{wrote:?}");
        assert_eq!(std::fs::read(&neighbour)?, b"not ours");
        Ok(())
    }

    /// The removal must not become one thread deleting the file another is writing.
    ///
    /// `process_temp` is one name for every write of one target in this process, and
    /// `tests/it/control_api.rs` drives two threads at `control.json` at once for exactly this
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

        let second = write_atomic(&target, &temp, b"{}", 0o600);
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
        let third = write_atomic(&target, &temp, b"{}", 0o600);
        assert!(third.is_ok(), "{third:?}");
        Ok(())
    }
}
