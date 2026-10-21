//! The checks of `goalcup prep`, `start` and `continue` over the folders a run writes to: WSL
//! ([`Host::check`]) and the filesystem type ([`Host::check_filesystems`]).
//!
//! On WSL1 Goal Cup Proxy cannot run at all. On WSL2 it can, but not on a Windows drive: reads
//! there are slow and file modes are not kept. The tests are by version and by filesystem *type*
//! (see [`goalcup_platform::machine::windows_host_filesystem`]), never by a path prefix, because a
//! Windows drive can be mounted anywhere. On a machine that is not WSL the WSL check looks at
//! nothing.
//!
//! On any machine a run keeps its records on APFS, ext4 or XFS only. `goalcup prep` fails its
//! step for a folder on any other filesystem; `start` and `continue` only warn
//! ([`Host::warn_filesystems`]) and go on, so a run is never refused there for it.

use goalcup_platform::machine;
use std::path::{Path, PathBuf};

/// What this machine says about WSL: its version, and the filesystem each folder is on.
///
/// A test build can state both instead ([`Host::VAR`], and the `wsl` and `fs` facts of the prep
/// test door); a shipped build always reads the machine.
#[derive(Clone)]
pub struct Host {
    version: Option<u8>,
    /// Stated mounts (a path prefix and the filesystem type under it); `None` reads the machine.
    mounts: Option<Vec<(PathBuf, String)>>,
}

impl Host {
    /// This machine.
    pub fn here() -> Host {
        Host {
            version: machine::wsl_version(),
            mounts: None,
        }
    }

    /// `Some(1)` or `Some(2)` on WSL, `None` elsewhere.
    pub fn version(&self) -> Option<u8> {
        self.version
    }

    /// Whether this is WSL of either version.
    pub fn is_wsl(&self) -> bool {
        self.version.is_some()
    }

    fn filesystem_of(&self, path: &Path) -> Option<String> {
        match &self.mounts {
            None => machine::windows_host_filesystem(path),
            Some(mounts) => mounts
                .iter()
                .filter(|(prefix, _)| path.starts_with(prefix))
                .max_by_key(|(prefix, _)| prefix.components().count())
                .map(|(_, name)| name.clone()),
        }
    }

    /// `Ok` unless this is WSL1 or, on WSL2, a folder is on a Windows-host filesystem. Each
    /// folder is `(what it is, path)`; the path need not exist yet. The `Err` is one or two
    /// sentences with no full stop at the end, for the caller to finish.
    pub fn check(&self, folders: &[(&str, &Path)]) -> Result<(), String> {
        machine::wsl_check_with(self.version, folders, &|p| self.filesystem_of(p))
    }

    /// `Ok` unless a folder is on a filesystem a run cannot keep its traces on (btrfs, ZFS, NFS,
    /// tmpfs, overlayfs or FUSE; see [`machine::unsupported_fstype`]), on any machine, WSL or
    /// not. The `Err` says each such folder and how to move it, with no full stop at the end.
    pub fn check_filesystems(&self, folders: &[(&str, &Path)]) -> Result<(), String> {
        let refused: Vec<String> = folders
            .iter()
            .filter_map(|(what, path)| {
                let name = match &self.mounts {
                    None => machine::unsupported_filesystem(path),
                    Some(_) => self
                        .filesystem_of(path)
                        .and_then(|name| machine::unsupported_fstype(&name)),
                }?;
                Some(crate::participant::unsupported_filesystem(
                    what,
                    &path.display().to_string(),
                    name,
                ))
            })
            .collect();
        if refused.is_empty() {
            Ok(())
        } else {
            Err(refused.join(". "))
        }
    }

    /// The refusal for `goalcup start` and `goalcup continue`: the folders a run writes to, all
    /// judged before anything is created or changed. `untouched` closes the sentence
    /// (`Nothing was created.`).
    pub fn refuse(
        &self,
        folders: &[(&str, &Path)],
        untouched: &str,
    ) -> Result<(), crate::CliError> {
        self.check(folders)
            .map_err(|why| crate::CliError::Refused(format!("{why}. {untouched}")))
    }

    /// What `goalcup start` and `goalcup continue` say of [`Host::check_filesystems`]: a
    /// warning, never a refusal (`goalcup prep` fails its step for the same). `None` when every
    /// folder is on a filesystem a run can keep its records on.
    pub fn warn_filesystems(&self, folders: &[(&str, &Path)]) -> Option<String> {
        self.check_filesystems(folders)
            .err()
            .map(|why| crate::participant::filesystem_warning(&why))
    }
}

/// **Test-only** ways to state the machine.
#[cfg(feature = "test-support")]
impl Host {
    /// The environment variable that states the machine to `start` and `continue`:
    /// comma-separated `wsl=0|1|2` and `fs=<path prefix>=<filesystem type>` (repeatable).
    pub const VAR: &'static str = "GOALCUP_TEST_WSL";

    /// This machine, with what `env` states in [`Host::VAR`] put over it.
    pub fn from_env(env: &std::collections::BTreeMap<String, String>) -> Host {
        let mut host = Host::here();
        if let Some(spec) = env.get(Host::VAR) {
            for part in spec.split(',').filter(|p| !p.is_empty()) {
                if let Some((key, value)) = part.split_once('=') {
                    host.state(key, value);
                }
            }
        }
        host
    }

    /// Put one stated fact (`wsl` or `fs`) over this host; whether `key` is one of them and
    /// `value` is well formed.
    pub fn state(&mut self, key: &str, value: &str) -> bool {
        match key {
            "wsl" => match value {
                "0" => self.version = None,
                "1" => self.version = Some(1),
                "2" => self.version = Some(2),
                _ => return false,
            },
            "fs" => match value.rsplit_once('=') {
                Some((prefix, name)) if prefix.starts_with('/') && !name.is_empty() => self
                    .mounts
                    .get_or_insert_with(Vec::new)
                    .push((PathBuf::from(prefix), name.to_string())),
                _ => return false,
            },
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stated(version: Option<u8>, mounts: &[(&str, &str)]) -> Host {
        Host {
            version,
            mounts: Some(
                mounts
                    .iter()
                    .map(|(p, n)| (PathBuf::from(p), (*n).to_string()))
                    .collect(),
            ),
        }
    }

    #[test]
    fn a_stated_mount_covers_what_is_below_it_and_the_deepest_wins() {
        let host = stated(Some(2), &[("/mnt/c", "9p"), ("/mnt/c/linux", "ext4")]);
        let c = Path::new("/mnt/c/Users/me");
        let linux = Path::new("/mnt/c/linux/work");
        let home = Path::new("/home/me");
        assert!(host.check(&[("work folder", c)]).is_err());
        assert!(host.check(&[("work folder", linux)]).is_ok());
        assert!(host.check(&[("work folder", home)]).is_ok());
    }

    #[test]
    fn wsl1_and_not_wsl() {
        let folders = [("run folder", Path::new("/home/me"))];
        let one = stated(Some(1), &[]).check(&folders).unwrap_err();
        assert!(one.contains("convert this distribution to WSL2"), "{one}");
        let none = stated(None, &[("/home", "9p")]);
        assert!(
            none.check(&folders).is_ok(),
            "not WSL: nothing is looked at"
        );
        assert!(!none.is_wsl());
    }

    #[test]
    fn an_unsupported_filesystem_is_a_warning_with_the_fix_first() {
        let folders = [("config folder", Path::new("/home/me/.config/goalcup"))];
        assert_eq!(
            stated(None, &[("/home", "ext4")]).warn_filesystems(&folders),
            None
        );
        let warned = stated(None, &[("/home", "tmpfs")])
            .warn_filesystems(&folders)
            .expect("tmpfs is warned about");
        assert!(warned.contains("GOALCUP_CONFIG_DIR"), "{warned}");
        assert!(warned.contains("tmpfs"), "{warned}");
        assert!(warned.contains("goalcup prep"), "{warned}");
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn the_test_door_states_the_version_and_the_mounts() {
        let mut host = stated(None, &[]);
        assert!(host.state("wsl", "2") && host.state("fs", "/mnt/c=9p"));
        assert_eq!(host.version(), Some(2));
        assert!(
            host.check(&[("work folder", Path::new("/mnt/c/x"))])
                .is_err()
        );
        assert!(!host.state("wsl", "3") && !host.state("fs", "mnt=9p") && !host.state("x", "1"));
    }
}
