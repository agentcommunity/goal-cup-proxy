//! The harness homes a run is given, for `goalcup start`, `goalcup continue` and `goalcup prep`:
//! the home every run of a connector shares (D39) or its shared run `HOME` (D59), judged and
//! locked so that one run at a time records in it, and the run's own directory ([`RunDir`]).
//! A home made fresh for each run is [`crate::per_run_home`]'s.

use crate::CliError;
use crate::connector::HarnessConnector;
use goalcup_protocol::harness::{CAPTURE_HOMES_DIR, CaptureSpec, ConfigHome, RunHome};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    kind: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SharedHomeObservation {
    leaf: FileIdentity,
    target: FileIdentity,
    canonical_target: PathBuf,
}

/// D39's exact shared-home leaf spelling plus the identities that made it safe at prepare time.
#[derive(Clone)]
pub(crate) struct SharedCaptureHome {
    alias: PathBuf,
    owner: u32,
    observed: SharedHomeObservation,
}

impl SharedCaptureHome {
    pub(crate) fn load(alias: &Path, owner: u32) -> Result<Self, CliError> {
        Ok(Self {
            alias: alias.to_path_buf(),
            owner,
            observed: observe_shared_capture_home(alias, owner)?,
        })
    }

    pub(crate) fn recheck(&self) -> Result<PathBuf, CliError> {
        // SAFETY: reading the effective uid has no preconditions or memory effects.
        let effective_uid = unsafe { libc::geteuid() };
        if effective_uid != self.owner {
            return Err(shared_home_refusal(
                &self.alias,
                "the launcher's effective user changed after preflight",
            ));
        }
        let observed = observe_shared_capture_home(&self.alias, effective_uid)?;
        if observed != self.observed {
            return Err(shared_home_refusal(
                &self.alias,
                "its leaf, target, or canonical target changed after preflight",
            ));
        }
        Ok(self.alias.clone())
    }
}

fn file_identity(metadata: &std::fs::Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;
    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        #[allow(
            clippy::unnecessary_cast,
            reason = "mode_t is u16 on macOS; the cast widens there even though it is a no-op on Linux glibc"
        )]
        kind: metadata.mode() & libc::S_IFMT as u32,
    }
}

/// D39: runs on one shared harness home are serialized. Two Codex runs on one `CODEX_HOME`
/// that both refresh the subscription token fail with `refresh_token_reused` and log the
/// participant out of that home. A second start is refused plainly, before anything is made.
///
/// An exclusive `flock` on `<capture-homes>/<connector>.goalcup.lock`, beside the home rather
/// than in it, so the harness never sees it. The descriptor is close-on-exec (std's default), so
/// the harness never inherits the lock either.
///
/// Not `<connector>.lock`: that is `<home> + ".lock"`, the very path Claude Code's token refresh
/// locks (proper-lockfile, `mkdir`, a lock younger than 60 s is busy). Goal Cup's file there,
/// rewritten at every start and continue, made a refresh due at the start fail (fix plan v6).
///
/// `name` is the harness's display name, for the refusal. `replaces`: this start is a competition
/// start that would replace an open competition run, which the refusal then says.
#[cfg(unix)]
pub(crate) fn lock_shared_home(
    config_home: &Path,
    connector: &str,
    name: &str,
    replaces: bool,
) -> Result<std::fs::File, CliError> {
    match try_lock_shared_home(config_home, connector)? {
        Ok(file) => Ok(file),
        Err(_busy) => Err(CliError::Refused(crate::participant::home_busy(
            name, replaces,
        ))),
    }
}

/// `<capture-homes>/<connector>.goalcup.lock`, beside the shared home. A name no harness uses:
/// the run home's leaf is `<connector>.run-home`, and a harness's own lock beside its home is
/// `<connector>.lock`.
fn home_lock_path(config_home: &Path, connector: &str) -> PathBuf {
    config_home
        .parent()
        .expect("a capture-home parent")
        .join(format!("{connector}.goalcup.lock"))
}

/// `<capture-homes>/<connector>.lock`, the lock a goalcup before fix plan v6 takes. Only looked
/// at, never made or written (it is Claude Code's refresh lock path). Remove with
/// [`old_home_lock_busy`] in a later release.
fn old_home_lock_path(config_home: &Path, connector: &str) -> PathBuf {
    config_home
        .parent()
        .expect("a capture-home parent")
        .join(format!("{connector}.lock"))
}

/// Upgrade safety: a run of an older goalcup holds the old-name lock on this home. The file is
/// opened read-only if it is there, flock-tested and closed at once: never created, written or
/// held. Anything that is not a regular file there (a harness's lock directory) is no goalcup's.
#[cfg(unix)]
fn old_home_lock_busy(config_home: &Path, connector: &str) -> Result<bool, CliError> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = old_home_lock_path(config_home, connector);
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
    else {
        // Absent, or nothing an older goalcup (which opens it the same no-follow way) could hold.
        return Ok(false);
    };
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return Ok(false);
    }
    match file.try_lock() {
        Ok(()) => Ok(false),
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Err(std::fs::TryLockError::Error(source)) => Err(CliError::Io {
            path: path.display().to_string(),
            source,
        }),
    }
}

/// [`lock_shared_home`]'s attempt: the lock, or the lock file's path when another process holds
/// it (so that the holder can be named, [`home_holder`]). A run of an older goalcup holding the
/// old-name lock ([`old_home_lock_busy`]) holds the home too, and its file is the one named.
#[cfg(unix)]
fn try_lock_shared_home(
    config_home: &Path,
    connector: &str,
) -> Result<Result<std::fs::File, PathBuf>, CliError> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = home_lock_path(config_home, connector);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|source| CliError::Io {
            path: path.display().to_string(),
            source,
        })?;
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(Err(path)),
        Err(std::fs::TryLockError::Error(source)) => {
            return Err(CliError::Io {
                path: path.display().to_string(),
                source,
            });
        }
    }
    // Checked while the new lock is held, so a second new start is refused by that one.
    if old_home_lock_busy(config_home, connector)? {
        return Ok(Err(old_home_lock_path(config_home, connector)));
    }
    Ok(Ok(file))
}

/// Write the id of the run that holds the shared-home lock into the lock file, so that a
/// `goalcup continue` refused by it can name that run. Best effort: the lock is the holder's
/// whether or not the note is written.
#[cfg(unix)]
pub(crate) fn note_home_holder(lock: &std::fs::File, run_id: &str) {
    use std::os::unix::fs::FileExt;
    if lock.set_len(0).is_ok() {
        let _ = lock.write_all_at(run_id.as_bytes(), 0);
    }
}

/// The run id [`note_home_holder`] wrote into the lock file at `path`, if it is a run id.
fn home_holder(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(64)
        .read_to_string(&mut text)
        .ok()?;
    let text = text.trim();
    crate::upload::is_run_id(text).then(|| text.to_string())
}

/// `goalcup continue`: take the lock on the connector's shared harness home before anything of
/// the run is closed or changed. A connector with a home of its own per run has none. Another
/// run holding it is refused, naming that run (or, when it has not yet said who it is, the run
/// that was to be continued).
#[cfg(unix)]
pub(crate) fn lock_home_for_continue(
    cfg: &Path,
    connector: &HarnessConnector,
    run_id: &str,
) -> Result<Option<std::fs::File>, CliError> {
    let id = &connector.loaded.manifest.id;
    let Some((config_home, _)) = shared_leaf(cfg, &connector.loaded.manifest.capture, id) else {
        return Ok(None);
    };
    private_dir(config_home.parent().expect("a capture-home parent"), true)?;
    match try_lock_shared_home(&config_home, id)? {
        Ok(lock) => {
            // Said at once, so a start refused by this lock before the segment's own record is
            // written names this run.
            note_home_holder(&lock, run_id);
            Ok(Some(lock))
        }
        Err(path) => {
            let holder = home_holder(&path).unwrap_or_else(|| run_id.to_string());
            Err(CliError::Refused(crate::participant::still_recording(
                connector.display_name(),
                &holder,
            )))
        }
    }
}

/// Which home of a connector is kept across runs, and so serialized under its one
/// `<capture-homes>/<id>.goalcup.lock`: its config home (D39, `config_home = "shared"`), at
/// `<cfg>/capture-homes/<id>`, or its run `HOME` (D59, `run_home = "shared"`), at
/// `<cfg>/capture-homes/<id>.run-home`. The schema never admits both. The `bool` is whether it
/// is the run `HOME`. `None`: every home is made per run.
///
/// The run home's leaf has a `.` in it, which no connector id has, so it is never another
/// connector's home or a lock, and a connector that moves its shared home from the one to the
/// other does not inherit the old tree as its `HOME`.
pub(crate) fn shared_leaf(cfg: &Path, profile: &CaptureSpec, id: &str) -> Option<(PathBuf, bool)> {
    let homes = cfg.join(CAPTURE_HOMES_DIR);
    if profile.home.config_home == ConfigHome::Shared {
        Some((homes.join(id), false))
    } else if profile.home.run_home == RunHome::Shared {
        Some((homes.join(format!("{id}.run-home")), true))
    } else {
        None
    }
}

/// The folder of a connector's kept home, which `goalcup prep` opens the harness in so that a
/// sign-in lands where the runs will look for it: its config home (D39) or its run `HOME`
/// (D59). `None` for a connector whose homes are all made per run: nothing to prepare.
pub(crate) fn prep_home_leaf(cfg: &Path, connector: &HarnessConnector) -> Option<PathBuf> {
    shared_leaf(
        cfg,
        &connector.loaded.manifest.capture,
        &connector.loaded.manifest.id,
    )
    .map(|(leaf, _)| leaf)
}

/// The kept home `goalcup prep` opens the harness in, claimed: its folder, and the lock that
/// keeps a run from it (and it from a recording run) while the participant is signed in there.
pub(crate) struct PrepHome {
    /// `<cfg>/capture-homes/<id>` or `<cfg>/capture-homes/<id>.run-home`, checked.
    pub(crate) dir: PathBuf,
    /// `dir` is the run's `HOME` (D59, `run_home = "shared"`) and not its config home.
    pub(crate) is_run_home: bool,
    _lock: std::fs::File,
}

/// Claim the home `goalcup prep` opens the harness in ([`prep_home_leaf`]): the one every run of
/// the connector shares, made if there is none, under the `<id>.goalcup.lock` a run takes. `None`
/// for a connector with no kept home. Judged as a run judges it.
pub(crate) fn claim_home_for_prep(
    cfg: &Path,
    connector: &HarnessConnector,
) -> Result<Option<PrepHome>, CliError> {
    let id = &connector.loaded.manifest.id;
    let Some((leaf, is_run_home)) = shared_leaf(cfg, &connector.loaded.manifest.capture, id) else {
        return Ok(None);
    };
    private_dir(leaf.parent().expect("a capture-home parent"), true)?;
    let lock = match try_lock_shared_home(&leaf, id)? {
        Ok(lock) => lock,
        Err(path) => {
            let holder = home_holder(&path);
            return Err(CliError::Refused(match holder {
                Some(run) => format!(
                    "run {run} is recording on this machine, and runs share one {id} home. \
                     Wait for it to end, then run `goalcup prep` again."
                ),
                None => format!(
                    "a {id} run is recording on this machine, or another `goalcup prep` has \
                     the {id} home open. Wait for it to end, then run `goalcup prep` again."
                ),
            }));
        }
    };
    let home = ensure_shared_home(&leaf)?;
    Ok(Some(PrepHome {
        dir: home.recheck()?,
        is_run_home,
        _lock: lock,
    }))
}

/// The shared harness home's folder (D39), made if there is none, and judged as every run
/// judges it. The caller holds the lock: `goalcup start` and `goalcup prep` differ only in
/// how they take it and what they say when they cannot.
pub(crate) fn ensure_shared_home(config_home: &Path) -> Result<SharedCaptureHome, CliError> {
    match std::fs::symlink_metadata(config_home) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match private_dir(config_home, false) {
                // A concurrent `goalcup start` made it between the look and the create;
                // `SharedCaptureHome::load` below judges that directory like any other.
                Err(CliError::Io { source, .. })
                    if source.kind() == std::io::ErrorKind::AlreadyExists => {}
                made => made?,
            }
        }
        Err(_) => {
            return Err(shared_home_refusal(
                config_home,
                "its exact leaf cannot be inspected",
            ));
        }
    }
    // SAFETY: reading the effective uid has no preconditions or memory effects.
    let owner = unsafe { libc::geteuid() };
    SharedCaptureHome::load(config_home, owner)
}

fn shared_home_refusal(alias: &Path, why: &str) -> CliError {
    CliError::Refused(format!(
        "the shared capture home `{}` is unsafe: {why}. Nothing was created.",
        alias.display()
    ))
}

#[cfg(unix)]
fn observe_shared_capture_home(
    alias: &Path,
    owner: u32,
) -> Result<SharedHomeObservation, CliError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let leaf = std::fs::symlink_metadata(alias).map_err(|_| {
        shared_home_refusal(
            alias,
            "its exact leaf cannot be inspected without following it",
        )
    })?;
    if !leaf.is_dir() && !leaf.file_type().is_symlink() {
        return Err(shared_home_refusal(
            alias,
            "its exact leaf is neither a directory nor a directory alias",
        ));
    }
    if leaf.uid() != owner {
        return Err(shared_home_refusal(
            alias,
            "its exact leaf is not owned by the effective user",
        ));
    }
    let leaf_identity = file_identity(&leaf);
    let canonical_target = std::fs::canonicalize(alias).map_err(|_| {
        shared_home_refusal(
            alias,
            "its exact leaf is dangling or has an unresolvable loop",
        )
    })?;
    if canonical_target.to_str().is_none() {
        return Err(shared_home_refusal(
            alias,
            "its canonical target is not a UTF-8 path",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let target = options.open(&canonical_target).map_err(|_| {
        shared_home_refusal(
            alias,
            "its canonical target cannot be opened as a no-follow directory",
        )
    })?;
    let target = target.metadata().map_err(|_| {
        shared_home_refusal(alias, "its opened canonical target cannot be inspected")
    })?;
    if !target.is_dir() {
        return Err(shared_home_refusal(
            alias,
            "its canonical target is not a directory",
        ));
    }
    if target.uid() != owner {
        return Err(shared_home_refusal(
            alias,
            "its canonical target is not owned by the effective user",
        ));
    }
    if target.mode() & 0o7777 != 0o700 {
        return Err(shared_home_refusal(
            alias,
            "its canonical target must have exact mode 0700 with no special, group, or world bits",
        ));
    }
    let target_identity = file_identity(&target);
    let leaf_after = std::fs::symlink_metadata(alias)
        .map_err(|_| shared_home_refusal(alias, "its exact leaf changed while it was inspected"))?;
    let canonical_after = std::fs::canonicalize(alias)
        .map_err(|_| shared_home_refusal(alias, "its canonical target changed while inspected"))?;
    if file_identity(&leaf_after) != leaf_identity || canonical_after != canonical_target {
        return Err(shared_home_refusal(
            alias,
            "its exact leaf or canonical spelling changed while it was inspected",
        ));
    }
    Ok(SharedHomeObservation {
        leaf: leaf_identity,
        target: target_identity,
        canonical_target,
    })
}

/// Test seam for the ownership decision without requiring a privileged `chown` fixture.
#[cfg(feature = "test-support")]
pub fn validate_shared_capture_home_for_uid(path: &Path, uid: u32) -> Result<(), CliError> {
    SharedCaptureHome::load(path, uid).map(|_| ())
}

/// This run's own directory, made in the folder `goalcup start` is run from: `workspace/` (the
/// daemon makes it), `home/` (the harness's `HOME`) and `ca/` (the recording proxy's CA
/// certificate).
///
/// A run on a server has its workspace elsewhere: a clone of the participant's repository in
/// their work folder, `<work_root>/goalcup-<16 hex digits>`, **a random name of its own**, never
/// this directory's (the folder it is run from may be the work folder, and the two must not
/// meet).
/// It is named before the server has issued the run id (the server and the daemon are told the
/// path), so the id cannot be in it.
pub(crate) struct RunDir {
    pub(crate) dir: PathBuf,
    pub(crate) workspace: PathBuf,
    /// The workspace is outside `dir`, so [`RunDir::remove`] must remove it too.
    separate_workspace: bool,
    pub(crate) home: PathBuf,
    pub(crate) ca: PathBuf,
}

/// Where a run's workspace is.
pub(crate) enum WorkspaceAt<'a> {
    /// A folder the daemon makes inside the run directory (a run with no server).
    InRunDir,
    /// A fresh clone, under a random name of its own, in this work folder.
    Clone(&'a Path),
    /// A clone that is there already: a continued run's. It is not this directory's to remove.
    Existing(&'a Path),
}

impl RunDir {
    /// `shared_home` is the connector's shared run `HOME` (D59), claimed and checked by the
    /// caller: with `RunHome::Shared` it is the run's `HOME`, used as it is and never removed.
    pub(crate) fn create(
        beside: &Path,
        home: RunHome,
        shared_home: Option<&Path>,
        env: &BTreeMap<String, String>,
        workspace_at: WorkspaceAt<'_>,
    ) -> Result<RunDir, CliError> {
        if (home == RunHome::Shared) != shared_home.is_some() {
            return Err(CliError::Internal(
                "a shared run home was not claimed for the run that keeps one".into(),
            ));
        }
        let beside = std::fs::canonicalize(beside).map_err(|source| CliError::Io {
            path: beside.display().to_string(),
            source,
        })?;
        let name = format!("goalcup-{:016x}", rand::random::<u64>());
        let dir = beside.join(&name);
        private_dir(&dir, false)?;
        let (workspace, separate_workspace) = match workspace_at {
            WorkspaceAt::Clone(root) => (
                root.join(format!("goalcup-{:016x}", rand::random::<u64>())),
                true,
            ),
            WorkspaceAt::Existing(workspace) => (workspace.to_path_buf(), false),
            WorkspaceAt::InRunDir => (dir.join("workspace"), false),
        };
        let run = RunDir {
            workspace,
            separate_workspace,
            // Outside `dir`, so `remove` never touches it.
            home: shared_home.map_or_else(|| dir.join("home"), Path::to_path_buf),
            ca: dir.join("ca"),
            dir,
        };
        let made = match shared_home {
            // Kept across runs, made by the claim: nothing is made in it here.
            Some(_) => Ok(()),
            None => private_dir(&run.home, false),
        }
        .and_then(|()| private_dir(&run.ca, false))
        .and_then(|()| match home {
            RunHome::Empty | RunHome::Shared => Ok(()),
            // The home's one entry: the user's keychain, so a subscription login is found
            // and nothing else of theirs is (D38).
            RunHome::KeychainLink => {
                let keychains = crate::per_run_home::home_from(env)?.join("Library/Keychains");
                let library = run.home.join("Library");
                private_dir(&library, false)?;
                symlink(&keychains, &library.join("Keychains"))
            }
            _ => Err(CliError::Refused(
                "the harness uses a run-home mode this goalcup does not support".into(),
            )),
        })
        .and_then(|()| write_login_profile(&run.home, &crate::launch::harness_path(env)));
        if let Err(e) = made {
            let _ = run.remove();
            return Err(e);
        }
        Ok(run)
    }

    /// Only ever before the spawn, when nothing but this command has written here. A clone
    /// outside the directory goes with it (and a failure to remove either is the error).
    ///
    /// A continued run's workspace ([`WorkspaceAt::Existing`]) is the run's clone, with the
    /// participant's agent's work in it, and is never this directory's to remove: only the
    /// segment's own folder goes.
    pub(crate) fn remove(&self) -> std::io::Result<()> {
        let clone = if self.separate_workspace {
            match std::fs::remove_dir_all(&self.workspace) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            }
        } else {
            Ok(())
        };
        std::fs::remove_dir_all(&self.dir).and(clone)
    }
}

/// The first line of the `.profile` [`write_login_profile`] writes.
pub(crate) const LOGIN_PROFILE_HEADER: &str = "# Written by Goal Cup: the PATH this run handed the \
     harness, which a login shell's /etc/profile resets.";

/// Write `<run_home>/.profile`, which puts back the `PATH` the launch hands the harness
/// ([`crate::launch::harness_path`]), unless a `.profile` is there already (a shared run home's
/// own, D59), which is kept as it is.
///
/// A harness that runs its commands in a login shell (Codex's `/bin/sh -lc`) has them run
/// `/etc/profile`, which on Debian and Ubuntu sets `PATH` outright; the participant's own
/// `~/.profile` would add `~/.local/bin` back, but the run's `HOME` is Goal Cup's empty one. This
/// repairs what that empty `HOME` took away, so the agent's commands get the participant's `PATH`
/// (D57) and `goalcup` on it. It changes no harness setting (D67). `bash -l` reads `.profile` when
/// the home has no `.bash_profile`, which an empty run home never has.
pub(crate) fn write_login_profile(run_home: &Path, path: &str) -> Result<(), CliError> {
    let file = run_home.join(".profile");
    let mut options = std::fs::OpenOptions::new();
    // `create_new` never follows or replaces anything already there, a link included.
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut out = match options.open(&file) {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(source) => {
            return Err(CliError::Io {
                path: file.display().to_string(),
                source,
            });
        }
    };
    let body = format!(
        "{LOGIN_PROFILE_HEADER}\nexport PATH={}\n",
        shell_single_quoted(path)
    );
    std::io::Write::write_all(&mut out, body.as_bytes()).map_err(|source| CliError::Io {
        path: file.display().to_string(),
        source,
    })
}

/// `value` as one POSIX shell word: single quotes, each `'` written `'\''`.
fn shell_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

pub(crate) fn private_dir(dir: &Path, recursive: bool) -> Result<(), CliError> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(recursive);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir).map_err(|source| CliError::Io {
        path: dir.display().to_string(),
        source,
    })
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> Result<(), CliError> {
    std::os::unix::fs::symlink(target, link).map_err(|source| CliError::Io {
        path: link.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A continued segment's run directory is unmade on a failed launch, and the run's clone,
    /// which the participant's agent has worked in, is not.
    #[test]
    fn unmaking_a_continued_segment_leaves_the_runs_clone_with_its_files() {
        let tmp = tempfile::tempdir().unwrap();
        let clone = tmp.path().join("clone");
        std::fs::create_dir_all(clone.join(".git")).unwrap();
        std::fs::write(clone.join("hello.txt"), b"the agent's work").unwrap();
        let beside = tmp.path().join("beside");
        std::fs::create_dir_all(&beside).unwrap();
        let env = BTreeMap::new();
        let run = RunDir::create(
            &beside,
            RunHome::Empty,
            None,
            &env,
            WorkspaceAt::Existing(&clone),
        )
        .unwrap();
        assert_eq!(run.workspace, clone);
        run.remove().unwrap();
        assert!(!run.dir.exists(), "the segment's own folder goes");
        assert_eq!(
            std::fs::read(clone.join("hello.txt")).unwrap(),
            b"the agent's work"
        );
        assert!(clone.join(".git").is_dir());
        // A fresh run's own clone is its to remove.
        let fresh = RunDir::create(
            &beside,
            RunHome::Empty,
            None,
            &env,
            WorkspaceAt::Clone(tmp.path()),
        )
        .unwrap();
        std::fs::create_dir_all(&fresh.workspace).unwrap();
        fresh.remove().unwrap();
        assert!(!fresh.workspace.exists());
    }

    /// A folder with an executable `goalcup-marker` in it, and a run made with that folder first
    /// on the participant's `PATH`.
    fn run_with_marker_on_path(tmp: &Path) -> (RunDir, String) {
        use std::os::unix::fs::PermissionsExt;
        // A quote in the folder's name proves the line is quoted for the shell.
        let bin = tmp.join("it's bin");
        std::fs::create_dir_all(&bin).unwrap();
        let marker = bin.join("goalcup-marker");
        std::fs::write(&marker, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o755)).unwrap();
        let beside = tmp.join("beside");
        std::fs::create_dir_all(&beside).unwrap();
        let path = format!("{}:/usr/bin:/bin", bin.display());
        let env = BTreeMap::from([("PATH".to_string(), path.clone())]);
        let run =
            RunDir::create(&beside, RunHome::Empty, None, &env, WorkspaceAt::InRunDir).unwrap();
        (run, path)
    }

    /// `sh -c <script>` with `HOME` the run's and the system `PATH` only.
    fn sh_in(home: &Path, flag: &str, script: &str) -> std::process::Output {
        std::process::Command::new("/bin/sh")
            .arg(flag)
            .arg(script)
            .env_clear()
            .env("HOME", home)
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap()
    }

    /// The run's `HOME` holds a `.profile` that gives a shell back exactly the `PATH` the launch
    /// hands the harness, so a login shell whose `/etc/profile` resets `PATH` (Debian's, which
    /// Codex's `/bin/sh -lc` runs) still finds the participant's tools.
    #[test]
    fn the_run_home_restores_the_harness_path_after_a_login_shell_reset() {
        let tmp = tempfile::tempdir().unwrap();
        let (run, path) = run_with_marker_on_path(tmp.path());
        let profile = run.home.join(".profile");
        let printed = sh_in(&run.home, "-c", r#". "$HOME/.profile"; printf %s "$PATH""#);
        assert_eq!(String::from_utf8_lossy(&printed.stdout), path);
        // What Debian's /etc/profile does, then what a login shell reads next.
        let found = sh_in(
            &run.home,
            "-c",
            r#"PATH=/usr/local/bin:/usr/bin:/bin; . "$HOME/.profile"; command -v goalcup-marker"#,
        );
        assert!(found.status.success(), "{found:?}");
        // A real login shell, on whatever `/etc/profile` this machine has.
        let login = sh_in(&run.home, "-lc", "command -v goalcup-marker");
        assert!(login.status.success(), "{login:?}");
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&profile).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    /// A shared run home's own `.profile` is kept as it is.
    #[test]
    fn an_existing_profile_in_the_run_home_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let shared = tmp.path().join("shared-home");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join(".profile"), b"# the participant's own\n").unwrap();
        let beside = tmp.path().join("beside");
        std::fs::create_dir_all(&beside).unwrap();
        let env = BTreeMap::from([("PATH".to_string(), "/opt/x/bin:/usr/bin".to_string())]);
        let run = RunDir::create(
            &beside,
            RunHome::Shared,
            Some(&shared),
            &env,
            WorkspaceAt::InRunDir,
        )
        .unwrap();
        assert_eq!(
            std::fs::read(run.home.join(".profile")).unwrap(),
            b"# the participant's own\n"
        );
    }

    /// #11: two runs on one shared harness home. The second is refused plainly while the first
    /// holds the home, and admitted once it lets go.
    #[test]
    fn a_second_run_on_a_shared_home_is_refused_until_the_first_ends() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("some-cli");
        let first = lock_shared_home(&home, "some-cli", "Some", false)
            .expect("the first run takes the home");

        let refused = lock_shared_home(&home, "some-cli", "Some", false)
            .expect_err("a second run is refused");
        let said = refused.to_string();
        assert!(matches!(refused, CliError::Refused(_)), "{said}");
        assert!(said.contains("already recording"), "{said}");
        // A practice or test start never replaces a competition run, and does not say so.
        assert!(!said.contains("replaces"), "{said}");
        let replacing = lock_shared_home(&home, "some-cli", "Some", true)
            .expect_err("a second run is refused")
            .to_string();
        assert!(
            replacing.contains("replaces your current competition run"),
            "{replacing}"
        );

        drop(first);
        lock_shared_home(&home, "some-cli", "Some", false)
            .expect("free again once the first run ends");
    }

    /// Fix plan v6: Claude Code locks its token refresh by making the directory `<home>.lock`
    /// (proper-lockfile). While a run holds the home and has noted itself, that still succeeds,
    /// and the run's own lock still keeps a second run out.
    #[test]
    fn the_harness_can_take_its_home_lock_while_a_run_holds_the_home() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("claude-code-cli");
        std::fs::create_dir(&home).unwrap();
        let first = lock_shared_home(&home, "claude-code-cli", "Claude Code", false)
            .expect("the first run takes the home");
        note_home_holder(&first, "run_0123456789abcdef");

        std::fs::create_dir(tmp.path().join("claude-code-cli.lock"))
            .expect("the harness's own lock beside its home is free");
        let refused = lock_shared_home(&home, "claude-code-cli", "Claude Code", false)
            .expect_err("a second run is still refused");
        assert!(matches!(refused, CliError::Refused(_)), "{refused}");
    }

    /// Upgrade safety: a run of an older goalcup holds `<connector>.lock`. A new start on the
    /// same home is refused while it does, and the old file is neither changed nor kept locked.
    #[test]
    fn a_run_of_an_older_goalcup_holding_the_old_lock_keeps_the_home() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("some-cli");
        let old_path = tmp.path().join("some-cli.lock");
        // What an older goalcup holds: its own open file, flocked.
        let old = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&old_path)
            .unwrap();
        std::fs::write(&old_path, "run_0123456789abcdef").unwrap();
        old.try_lock().unwrap();
        let before = std::fs::metadata(&old_path).unwrap();

        let refused = lock_shared_home(&home, "some-cli", "Some", false)
            .expect_err("the older run still holds the home");
        assert!(matches!(refused, CliError::Refused(_)), "{refused}");
        assert_eq!(
            try_lock_shared_home(&home, "some-cli").unwrap().err(),
            Some(old_path.clone()),
            "the older run's file is the one that names it"
        );
        let after = std::fs::metadata(&old_path).unwrap();
        assert_eq!(after.len(), before.len());
        assert_eq!(after.modified().unwrap(), before.modified().unwrap());

        drop(old);
        let taken = lock_shared_home(&home, "some-cli", "Some", false)
            .expect("free once the older run ends");
        // The new start holds no lock on the old file.
        let probe = std::fs::File::open(&old_path).unwrap();
        probe.try_lock().expect("the old file is not held");
        drop(probe);
        drop(taken);
    }
}
