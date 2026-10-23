//! Whether a folder is the root of its own Git clone: one rule, read by the recorder when it
//! creates a run's workspace and by `goalcup continue` before it closes anything (D4, D56).
//!
//! The rule: the folder holds a `.git` *directory* (not a file, not a link) and no folder above
//! it is a Git work tree. The folder itself may be a symbolic link to such a clone: it is
//! followed, as it always has been, so a clone moved to a bigger disk and linked back at its
//! old path is still the run's clone. Only the folder's own `.git` must be real.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// What is at a run's workspace path, as the clone rule sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CloneRoot {
    /// The root of its own clone.
    Clone,
    /// Nothing is there, or a link that points at nothing.
    Missing,
    /// Something is there, but it is not a folder.
    NotAFolder,
    /// A folder with no `.git` directory: none at all, or a `.git` file or link.
    NoGitDirectory,
    /// A clone root that lies inside another Git work tree, named.
    InsideRepository(PathBuf),
    /// The folder, or its `.git`, could not be looked at (permission denied, most often).
    Unreadable(ErrorKind),
}

/// Look at `workspace` and say what it is under the clone rule. Changes nothing.
pub fn clone_root(workspace: &Path) -> CloneRoot {
    // Follows a link at `workspace` itself: a linked clone is the clone.
    match std::fs::metadata(workspace) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return CloneRoot::NotAFolder,
        Err(error) if error.kind() == ErrorKind::NotFound => return CloneRoot::Missing,
        Err(error) => return CloneRoot::Unreadable(error.kind()),
    }
    // Does not follow a link at `.git`: a `.git` link or file is not a clone's own.
    match workspace.join(".git").symlink_metadata() {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return CloneRoot::NoGitDirectory,
        Err(error) if error.kind() == ErrorKind::NotFound => return CloneRoot::NoGitDirectory,
        Err(error) => return CloneRoot::Unreadable(error.kind()),
    }
    match workspace.parent().and_then(git_worktree_of) {
        Some(outer) => CloneRoot::InsideRepository(outer),
        None => CloneRoot::Clone,
    }
}

/// `workspace` is the root of its own clone ([`clone_root`] is [`CloneRoot::Clone`]).
pub fn is_clone_root(workspace: &Path) -> bool {
    clone_root(workspace) == CloneRoot::Clone
}

/// The nearest ancestor of `path` (itself included) holding a `.git` entry, file or directory,
/// or `None`. A `.git` *file* counts: that is what a linked worktree has.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn clone_in(dir: &Path) -> PathBuf {
        let ws = dir.join("ws");
        std::fs::create_dir_all(ws.join(".git")).unwrap();
        ws
    }

    #[test]
    fn a_folder_with_its_own_git_directory_is_a_clone() {
        let t = tempfile::tempdir().unwrap();
        let ws = clone_in(t.path());
        assert_eq!(clone_root(&ws), CloneRoot::Clone);
        assert!(is_clone_root(&ws));
    }

    #[test]
    fn nothing_there_and_a_dangling_link_are_missing() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(clone_root(&t.path().join("gone")), CloneRoot::Missing);
        let link = t.path().join("link");
        symlink(t.path().join("nowhere"), &link).unwrap();
        assert_eq!(clone_root(&link), CloneRoot::Missing);
    }

    #[test]
    fn a_file_is_not_a_folder() {
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join("ws");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(clone_root(&file), CloneRoot::NotAFolder);
    }

    /// The agent ran `rm -rf .git`, or `.git` is a file or a link: there, but not a clone.
    #[test]
    fn a_folder_whose_git_directory_is_gone_or_not_real_is_no_clone() {
        let t = tempfile::tempdir().unwrap();
        let ws = t.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        assert_eq!(clone_root(&ws), CloneRoot::NoGitDirectory);
        std::fs::write(ws.join(".git"), b"gitdir: /elsewhere\n").unwrap();
        assert_eq!(clone_root(&ws), CloneRoot::NoGitDirectory);
        std::fs::remove_file(ws.join(".git")).unwrap();
        let real = t.path().join("real-git");
        std::fs::create_dir_all(&real).unwrap();
        symlink(&real, ws.join(".git")).unwrap();
        assert_eq!(clone_root(&ws), CloneRoot::NoGitDirectory);
    }

    /// A clone moved to another disk and linked back at its old path is still the clone.
    #[test]
    fn a_link_to_a_clone_is_the_clone() {
        let t = tempfile::tempdir().unwrap();
        let moved = clone_in(&t.path().join("bigger-disk"));
        let link = t.path().join("ws");
        symlink(&moved, &link).unwrap();
        assert_eq!(clone_root(&link), CloneRoot::Clone);
    }

    #[test]
    fn a_clone_inside_another_work_tree_names_it() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join(".git")).unwrap();
        let ws = clone_in(&t.path().join("work"));
        assert_eq!(
            clone_root(&ws),
            CloneRoot::InsideRepository(t.path().to_path_buf())
        );
        assert!(!is_clone_root(&ws));
    }

    /// A folder this user cannot enter cannot be judged; it is not called missing.
    #[test]
    fn a_folder_that_cannot_be_entered_is_unreadable() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let ws = clone_in(t.path());
        std::fs::set_permissions(&ws, std::fs::Permissions::from_mode(0o000)).unwrap();
        let said = clone_root(&ws);
        std::fs::set_permissions(&ws, std::fs::Permissions::from_mode(0o755)).unwrap();
        // root reads through any mode; everyone else is refused.
        if is_root() {
            assert_eq!(said, CloneRoot::Clone);
        } else {
            assert_eq!(said, CloneRoot::Unreadable(ErrorKind::PermissionDenied));
        }
    }

    fn is_root() -> bool {
        // SAFETY: geteuid has no preconditions and cannot fail.
        unsafe { libc::geteuid() == 0 }
    }
}
