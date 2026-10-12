//! The per-run private harness home of a Hermes capture (D39, D41).
//!
//! A profile whose `config_home` is `per_run` gets a harness home made fresh inside the run
//! directory, holding only the connector's own seed files. Hermes reads and writes there and
//! nowhere else: its configuration, its credential store, its session snapshots, its logs and
//! its tool processes' home all live under `<run>/hermes`, so nothing of the participant's
//! `~/.hermes` is read and nothing is written to it.
//!
//! ```text
//! <run>/hermes          0700   HERMES_HOME
//! <run>/hermes/home     0700   the tool processes' home (`TERMINAL_HOME_MODE=profile`)
//! <run>/hermes/config.yaml  0600   from connectors/<id>/files/config.yaml, byte for byte
//! <run>/hermes/auth.json    0600   from connectors/<id>/files/auth.json, byte for byte
//! <run>/hermes/.update_check 0600  a static, always-fresh update-check cache (no PyPI request)
//! ```
//!
//! `sessions/` is not made here: the daemon creates the one directory it reads
//! (`runtime/capture/mod.rs`), and a capture entry created over this home finds it empty.
//!
//! The seed files are the connector's own, so the connector directory's pin covers them: this
//! module reads exactly the bytes the manifest gate has already accepted, and writes them with a
//! read-back comparison.
//!
//! # A run directory beside a goal file must not be in a repository
//!
//! The run directory holds what a harness wrote while it ran, and Hermes writes request dumps
//! that can carry a masked fragment of the provider key (D41). [`refuse_inside_git_work_tree`]
//! stops a launch whose goal file sits inside a Git work tree, before anything is created, so
//! those files are never one `git add .` away from a commit.

use crate::CliError;
use goalcup_protocol::harness::{ConfigHome, HomeSpec, LoadedSeed, SeedMode};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The participant's real home, out of an environment map.
///
/// One rule for every caller. `goalcup start` is handed the environment its preflight asks
/// its questions of; the V1 `goalcup resume` (removed by D48) read this process's own. An
/// exported-but-empty `$HOME` counts as unset for the reason [`crate::config`] gives: it is
/// what a shell script that meant to compute a path and computed nothing leaves behind, and
/// joining a seed candidate onto it would read a file out of the working directory, which
/// during a run is the participant's repository.
pub(crate) fn home_from(
    env: &std::collections::BTreeMap<String, String>,
) -> Result<PathBuf, CliError> {
    match env.get("HOME").filter(|h| !h.is_empty()) {
        Some(home) => Ok(PathBuf::from(home)),
        None => Err(CliError::Refused(
            "$HOME is not set, and this connector's launch has to read the harness \
             configuration under it before it can start a session that is already signed in."
                .to_string(),
        )),
    }
}

/// Make the manifest's per-run config home from bytes captured by the protocol loader.
///
/// The directory must not exist: a home that already holds anything is not this run's, and is
/// left exactly as it was. A refusal after the directory was made removes what was made.
pub fn create_per_run_home(
    run_dir: &Path,
    spec: &HomeSpec,
    seeds: &[LoadedSeed],
) -> Result<PathBuf, CliError> {
    match spec.config_home {
        ConfigHome::PerRun => {}
        ConfigHome::Shared => {
            return Err(CliError::Internal(
                "a shared harness home was passed to the per-run home writer".into(),
            ));
        }
        _ => {
            return Err(CliError::Refused(
                "the harness uses a config-home mode this goalcup does not support".into(),
            ));
        }
    }
    let home = run_dir.join(spec.per_run_dir_name());
    // Not recursive: `mkdir` fails on anything already there, a symlink included.
    private_dir(&home)?;
    let made = (|| {
        let mut created_dirs = Vec::new();
        for dir in directory_plan(spec, seeds) {
            private_plan_dir(&home.join(dir), &mut created_dirs)?;
        }
        for seed in seeds {
            write_private_file(&home.join(&seed.file), &seed.bytes, seed.mode)?;
        }
        std::fs::canonicalize(&home).map_err(|source| CliError::Io {
            path: home.display().to_string(),
            source,
        })
    })();
    if made.is_err() {
        let _ = std::fs::remove_dir_all(&home);
    }
    made
}

/// Every directory the home needs, exactly once, parents before children.
///
/// The schema admits a child before its parent and repeated directory declarations. Seed files
/// also need parents even when the manifest does not declare them separately. Paths have already
/// passed the harness-contract relative-path checks; keeping `(depth, spelling)` in a set both
/// deduplicates common prefixes and gives one deterministic creation order.
fn directory_plan(spec: &HomeSpec, seeds: &[LoadedSeed]) -> Vec<String> {
    let mut plan = BTreeSet::new();
    for dir in &spec.dirs {
        add_directory_prefixes(&mut plan, dir);
    }
    for seed in seeds {
        if let Some((parent, _)) = seed.file.rsplit_once('/') {
            add_directory_prefixes(&mut plan, parent);
        }
    }
    plan.into_iter().map(|(_, path)| path).collect()
}

fn add_directory_prefixes(plan: &mut BTreeSet<(usize, String)>, relative: &str) {
    let mut prefix = String::new();
    for (index, component) in relative.split('/').enumerate() {
        if index > 0 {
            prefix.push('/');
        }
        prefix.push_str(component);
        plan.insert((index + 1, prefix.clone()));
    }
}

#[cfg(unix)]
type PrivateDirHandle = std::fs::File;

#[cfg(not(unix))]
struct PrivateDirHandle;

/// Create one planned directory strictly, remembering what this invocation made.
///
/// A case-insensitive filesystem can report `AlreadyExists` for two different schema-valid
/// spellings such as `Case` and `case`. That is accepted only when a no-follow directory handle
/// proves the spelling names a directory created earlier in this same plan. An arbitrary
/// pre-existing directory, symlink or directory whose permissions became non-private still fails.
#[cfg(unix)]
fn private_plan_dir(dir: &Path, created: &mut Vec<PrivateDirHandle>) -> Result<(), CliError> {
    match private_dir(dir) {
        Ok(()) => {
            created.push(open_private_dir(dir)?);
            Ok(())
        }
        Err(error)
            if matches!(
                &error,
                CliError::Io { source, .. }
                    if source.kind() == std::io::ErrorKind::AlreadyExists
            ) =>
        {
            let Ok(candidate) = open_private_dir(dir) else {
                return Err(error);
            };
            if created
                .iter()
                .any(|prior| same_private_directory(prior, &candidate))
            {
                Ok(())
            } else {
                Err(error)
            }
        }
        Err(error) => Err(error),
    }
}

#[cfg(not(unix))]
fn private_plan_dir(dir: &Path, _created: &mut Vec<PrivateDirHandle>) -> Result<(), CliError> {
    private_dir(dir)
}

#[cfg(unix)]
fn open_private_dir(dir: &Path) -> Result<PrivateDirHandle, CliError> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let io = |source| CliError::Io {
        path: dir.display().to_string(),
        source,
    };
    let mut options = std::fs::OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let handle = options.open(dir).map_err(io)?;
    let metadata = handle.metadata().map_err(io)?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(CliError::Refused(format!(
            "{} is not an owner-private directory. Nothing was launched.",
            dir.display()
        )));
    }
    Ok(handle)
}

#[cfg(unix)]
fn same_private_directory(left: &PrivateDirHandle, right: &PrivateDirHandle) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let (Ok(left), Ok(right)) = (left.metadata(), right.metadata()) else {
        return false;
    };
    left.is_dir()
        && right.is_dir()
        && left.permissions().mode() & 0o777 == 0o700
        && right.permissions().mode() & 0o777 == 0o700
        && left.dev() == right.dev()
        && left.ino() == right.ino()
}

fn private_dir(dir: &Path) -> Result<(), CliError> {
    let mut builder = std::fs::DirBuilder::new();
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

/// A new file with the declared private mode holding exactly `bytes`, read back and compared.
fn write_private_file(path: &Path, bytes: &[u8], mode: SeedMode) -> Result<(), CliError> {
    let io = |source| CliError::Io {
        path: path.display().to_string(),
        source,
    };
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    let mode = match mode {
        SeedMode::Rw0600 => 0o600,
        SeedMode::Ro0400 => 0o400,
        _ => {
            return Err(CliError::Refused(
                "the harness uses a seed mode this goalcup does not support".into(),
            ));
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path).map_err(io)?;
    file.write_all(bytes).map_err(io)?;
    file.sync_all().map_err(io)?;
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Whatever the process umask did, the file has exactly the manifest's private mode.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(io)?;
    }
    if std::fs::read(path).map_err(io)? != bytes {
        return Err(CliError::Refused(format!(
            "{} did not read back as written. Nothing was launched.",
            path.display()
        )));
    }
    Ok(())
}

/// Refuse a directory that is inside a Git work tree: any ancestor, itself included, that holds
/// a `.git` (a directory, or the file a linked work tree has).
///
/// `dir` is made absolute and has its links resolved first, so a symlinked path is judged where
/// it really is. A directory that cannot be resolved is refused too: a check that could not run
/// is not a pass.
pub fn refuse_inside_git_work_tree(dir: &Path) -> Result<(), CliError> {
    let resolved = std::fs::canonicalize(dir).map_err(|source| CliError::Io {
        path: dir.display().to_string(),
        source,
    })?;
    for ancestor in resolved.ancestors() {
        if std::fs::symlink_metadata(ancestor.join(".git")).is_ok() {
            return Err(CliError::Refused(format!(
                "the goal file is inside a Git work tree ({}), and this run's directory would \
                 be made beside it: it holds what the harness wrote, which can include a \
                 fragment of the provider key. Put the goal file in a directory outside any \
                 repository. Nothing was created.",
                ancestor.display()
            )));
        }
    }
    Ok(())
}

/// Refuse a directory a client syncs off the machine: under `Library/Mobile Documents` (iCloud
/// Drive) or `Library/CloudStorage` (the File Provider clients), or in a folder named for
/// Dropbox, OneDrive, Google Drive or iCloud Drive. A run directory there would have
/// `request_dump_*.json` uploaded, which "stays local" forbids.
///
/// By name only, on the resolved path. iCloud's Desktop & Documents folders have no such name:
/// [`refuse_synced_folder_under`] adds them when a HOME is known and the local signal is present.
#[cfg(any(test, feature = "test-support"))]
pub fn refuse_synced_folder(dir: &Path) -> Result<(), CliError> {
    refuse_synced_folder_under(dir, None)
}

/// [`refuse_synced_folder`], and with `home` known (the process's `HOME`), also a directory at or
/// below `~/Desktop` or `~/Documents` when [`icloud_folder_synced`] sees iCloud's "Desktop &
/// Documents" on that folder (H-18b). Nothing is refused for those folders when the signal is
/// absent, and none is looked for when `home` is `None` or empty.
pub fn refuse_synced_folder_under(dir: &Path, home: Option<&Path>) -> Result<(), CliError> {
    refuse_synced_folder_probing(dir, home, &icloud_folder_synced)
}

/// [`refuse_synced_folder_under`] with the iCloud signal supplied by the caller, so a test can
/// stand in for the one macOS attribute a process cannot set on a fake folder. `probe(home, name)`
/// answers whether `<home>/<name>` is iCloud-synced.
pub fn refuse_synced_folder_probing(
    dir: &Path,
    home: Option<&Path>,
    probe: &dyn Fn(&Path, &str) -> bool,
) -> Result<(), CliError> {
    let resolved = std::fs::canonicalize(dir).map_err(|source| CliError::Io {
        path: dir.display().to_string(),
        source,
    })?;
    let names: Vec<String> = resolved
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let synced = names
        .windows(2)
        .any(|w| w[0] == "Library" && (w[1] == "Mobile Documents" || w[1] == "CloudStorage"))
        || names.iter().any(|n| {
            n == "iCloud Drive"
                || n == "Google Drive"
                || n == "Dropbox"
                || n.starts_with("Dropbox (")
                || n == "OneDrive"
                || n.starts_with("OneDrive - ")
        })
        || home
            .filter(|h| !h.as_os_str().is_empty())
            .and_then(|h| std::fs::canonicalize(h).ok())
            .is_some_and(|home| {
                ["Desktop", "Documents"]
                    .iter()
                    .any(|name| resolved.starts_with(home.join(name)) && probe(&home, name))
            });
    if synced {
        return Err(CliError::Refused(format!(
            "the goal file is in a folder a sync client uploads ({}), and this run's directory \
             would be made beside it: it holds what the harness wrote, which can include a \
             fragment of the provider key. Put the goal file in a directory that is not synced. \
             Nothing was created.",
            resolved.display()
        )));
    }
    Ok(())
}

/// Whether macOS iCloud "Desktop & Documents" is syncing `<home>/<name>` (`Desktop` or
/// `Documents`), from local state only: no network, no privileged call, no `brctl`.
///
/// Either signal is enough, and no signal is "not synced":
/// * the folder carries the `com.apple.file-provider-domain-id` extended attribute ([`FILE_PROVIDER_ATTRIBUTE`]), which File
///   Provider puts on a folder it owns (macOS only); or
/// * the folder is the same directory (device and inode) as
///   `Library/Mobile Documents/com~apple~CloudDocs/<name>`, which is how iCloud mirrored these
///   two folders before File Provider.
///
/// The mere existence of `CloudDocs/<name>` is not a signal: it is left behind, as a stub, when
/// the setting is turned off, and many machines have a `Documents` there that is not
/// `~/Documents`. Unverified against a machine that has the setting on: the signals are written
/// from the documented behaviour and tested on a fake HOME (CHEAT-CHECKLIST).
pub fn icloud_folder_synced(home: &Path, name: &str) -> bool {
    icloud_folder_synced_with(home, name, FILE_PROVIDER_ATTRIBUTE)
}

/// The extended attribute File Provider puts on a folder it owns.
pub(crate) const FILE_PROVIDER_ATTRIBUTE: &str = "com.apple.file-provider-domain-id";

/// [`icloud_folder_synced`] with the attribute named, because macOS refuses to set this one on a
/// fake folder: a test exercises the mechanism with an attribute it can set.
pub fn icloud_folder_synced_with(home: &Path, name: &str, attribute: &str) -> bool {
    let folder = home.join(name);
    if !std::fs::metadata(&folder).is_ok_and(|m| m.is_dir()) {
        return false;
    }
    if has_attribute(&folder, attribute) {
        return true;
    }
    let drive = home
        .join("Library/Mobile Documents/com~apple~CloudDocs")
        .join(name);
    same_directory(&folder, &drive)
}

#[cfg(unix)]
fn same_directory(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.is_dir() && b.is_dir() && a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn same_directory(_: &Path, _: &Path) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn has_attribute(folder: &Path, attribute: &str) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let (Ok(path), Ok(name)) = (
        std::ffi::CString::new(folder.as_os_str().as_bytes()),
        std::ffi::CString::new(attribute),
    ) else {
        return false;
    };
    // Size query only: no buffer, no value read. A negative answer (no such attribute, no
    // permission, unsupported) is "no signal".
    // SAFETY: `path` and `name` are NUL-terminated and outlive the call; a null buffer with
    // size 0 asks only for the attribute's length.
    let len =
        unsafe { libc::getxattr(path.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0, 0, 0) };
    len >= 0
}

#[cfg(not(target_os = "macos"))]
fn has_attribute(_: &Path, _: &str) -> bool {
    false
}
