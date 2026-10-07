//! Tracker config is watched, not locked (D6).
//!
//! The watch covers tracker files and app-wide allowlisted files.
//!
//!   * [`ConfigWatch::poll`] compares the bytes on disk against the bytes the daemon wrote
//!     and reports every difference with a timestamp and both digests, then puts the file
//!     back.
//!     No read-only flag, no immutable bit, no `chattr`: D6 chose watching over locking
//!     because a locked file makes a harness fail in a way the participant cannot debug,
//!     and a restored one leaves a record of exactly what happened.
//!   * D24's reported hook hash is not watched: no hook receiver reports one.
//!   * The `config_allowlist[]` of app-wide files, which workspace setup hashes immediately before
//!     launch and which arrive here as [`AllowlistedFile`]s with `restore = false`. These
//!     are the participant's own files, outside every tracker-created folder. A deviation
//!     on one is reported and chained and **left exactly as the participant left it** —
//!     this crate's writes are bounded to the workspace, the artifact root and the harness
//!     home, and AGENTS.md forbids touching app-wide harness config. `restore` is a field on
//!     [`WatchedFile`] rather than a rule about which
//!     argument a file arrived in, so a reader of `poll` sees the distinction without
//!     having to know where the vector was built.
//!
//! **Files outside the workspace are watched too.** Their bytes are covered here by their
//! `TrackerFile::sha256` like every other installed file, and that is why `new` takes the
//! whole `Installation::files` list rather than the workspace-scoped subset
//! `Workspace::expected_config` holds.
//!
//! **Polling, not notification.** [`poll`](ConfigWatch::poll) is a plain function the
//! daemon's timer calls and a test calls directly. No filesystem-watch crate: an inotify
//! or FSEvents stream is a second source of truth about when something happened, and a
//! missed event there is silence rather than a late report.
//!
//! **A deviation carries digests and never bytes.** [`ConfigDeviation::diff`] is left empty
//! by this module, always. A tracker file may carry the per-run bearer (`{receiver_token}`)
//! and its expected bytes are the daemon's own rendering, so a diff built from either side is
//! a direct path from a secret into the chain - and `ChainWriter::event`, the method that
//! chains a deviation, does not run the redactor (only `leg1` does).
//! `InstalledFile` omits `contents` for the same reason: the reviewer needs paths and
//! hashes, not bytes. What changed is answered by the two digests plus the traced tool
//! calls, which V9 already reconciles against.
//!
//! **Nothing here interprets a rule** (D3). Every comparison below is a digest against a
//! digest the daemon itself recorded. Whether a deviation is the agent tampering or the
//! participant tidying is `goalcup-verifier`'s V9 to decide, from the traced tool calls.

use crate::workspace::{ABSENT, AllowlistedFile, TrackerFile};
use crate::{DaemonError, MAX_FILE_BYTES, now_rfc3339};
use goalcup_protocol::events::ConfigDeviation;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[cfg(any(test, feature = "test-hooks"))]
use std::sync::{Mutex, OnceLock, mpsc};

#[cfg(any(test, feature = "test-hooks"))]
struct RestorePause {
    reached: mpsc::SyncSender<()>,
    resume: mpsc::Receiver<()>,
}

#[cfg(any(test, feature = "test-hooks"))]
static RESTORE_PAUSES: OnceLock<Mutex<BTreeMap<PathBuf, RestorePause>>> = OnceLock::new();

#[cfg(any(test, feature = "test-hooks"))]
fn pause_after_restore_validation(path: &Path) {
    let pause = {
        let mut pauses = RESTORE_PAUSES
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        pauses.remove(path)
    };
    if let Some(pause) = pause {
        let _ = pause.reached.send(());
        let _ = pause.resume.recv();
    }
}

/// Bytes read per hashing step, mirroring `workspace`'s seal walk. A config file the
/// participant replaced with a gigabyte is still hashed a chunk at a time.
const HASH_CHUNK: usize = 64 * 1024;

/// One file the watch compares on every poll: a path, the digest expected there, and — for
/// the files the tracker itself wrote — the bytes to put back.
#[derive(Debug, Clone)]
pub struct WatchedFile {
    pub path: PathBuf,
    /// Relative to the file's scope root for a tracker file; the absolute path for an
    /// app-wide allowlisted file, which belongs to no scope root the tracker owns.
    pub relative_path: String,
    pub expected_sha256: String,
    /// Empty for an allowlisted file: there is nothing to restore it with, because the
    /// tracker never wrote it and never will.
    pub expected_bytes: Vec<u8>,
    /// Restoring this file invalidates a hook-trust hash, so a restore raises
    /// `arming_lost` (D6, Codex). Meaningful only where `restore` is true.
    pub restore_invalidates_trust: bool,
    /// False for `config_allowlist[]` files: reported, never written back (D21).
    pub restore: bool,
    /// The scope root `path` was built from - `absolute_path` with `relative_path`'s
    /// components popped off, so it is the workspace, the artifact root or the harness
    /// home. `None` for a file that is never restored.
    ///
    /// A restore removes what it finds in the way (a symlink, a directory), and removing
    /// is the one operation that has to prove where it is pointed before it acts. The root
    /// is what it proves it against.
    pub restore_root: Option<PathBuf>,
}

/// The watch over one list of files.
#[derive(Debug)]
pub struct ConfigWatch {
    files: Vec<WatchedFile>,
    /// The digest last reported for a path that could not be put back, so an app-wide file
    /// the participant changed once is one event and not one per poll for the rest of the
    /// run. A restored file needs no entry here: the restore makes the next comparison
    /// match again, which is the same "once per change" by a shorter route.
    ///
    /// Held here rather than by moving `WatchedFile::expected_sha256` forward, so a second
    /// deviation is still measured against the digest taken at launch — which is what the
    /// verifier reconciles against, and what `AllowlistedFile::sha256` means.
    reported: BTreeMap<PathBuf, String>,
    /// The permission bits each tracker file had when the watch was built, so a restore
    /// after deletion puts back the file the tracker installed rather than a file with the
    /// same bytes and the default mode.
    ///
    /// An executable file recreated at 0o644 is one the harness cannot execute, which is
    /// capture stopping in silence: the exact failure D6 chose restoring over locking to avoid.
    ///
    /// Read once, at construction, and never refreshed: an agent that chmods a tracker
    /// file has changed the file, and restoring the mode it chose would be the watch
    /// adopting the deviation it is supposed to undo.
    modes: BTreeMap<PathBuf, u32>,
    /// Restores that could not be performed. A failed restore is a deviation with
    /// `restored = false` and the poll keeps going, so this is the only place the count
    /// survives.
    failed_restores: u64,
}

impl ConfigWatch {
    /// Three arguments and no `workspace`: a [`TrackerFile`] carries its own
    /// `absolute_path`, because the tracker writes into three scopes and only one of them
    /// is the workspace — Claude's `--settings` file lives in the artifact root, where the
    /// agent cannot enumerate it, and it is watched exactly like a workspace-scoped one.
    ///
    /// **Not every tracker file is watched.** A `harness_owned` one is not: see the filter.
    pub fn new(files: &[TrackerFile], allowlisted: &[AllowlistedFile]) -> ConfigWatch {
        let mut watched: Vec<WatchedFile> = files
            .iter()
            // The file is the harness's, seeded once into a home the tracker made and never
            // guarded after that: Codex writes its own tip counter into
            // `$CODEX_HOME/config.toml` on every launch, so watching it chains a deviation
            // and Fails V9 on every honest run. See
            // `goalcup_protocol::manifest::HookInstall::harness_owned` for the argument and
            // `docs/CHEAT-CHECKLIST.md` for the blind spot it accepts.
            .filter(|f| !f.harness_owned)
            .map(|f| WatchedFile {
                path: f.absolute_path.clone(),
                relative_path: f.relative_path.clone(),
                expected_sha256: f.sha256.clone(),
                expected_bytes: f.contents.as_bytes().to_vec(),
                restore_invalidates_trust: f.restore_invalidates_trust,
                restore: true,
                restore_root: scope_root(&f.absolute_path, &f.relative_path),
            })
            .collect();
        // Allowlisted files: no `expected_bytes`, because there is nothing to restore, and
        // `restore_invalidates_trust` false, because a file that is never restored cannot
        // lose trust by being restored: the two flags can never both be true.
        watched.extend(allowlisted.iter().map(|a| WatchedFile {
            path: a.path.clone(),
            // The manifest's own spelling, `{home}/.claude/settings.json`. Chaining the
            // absolute path would put the participant's login name in every event.
            relative_path: a.relative_path.clone(),
            expected_sha256: a.sha256.clone(),
            expected_bytes: Vec::new(),
            restore_invalidates_trust: false,
            restore: false,
            restore_root: None,
        }));
        let modes = watched
            .iter()
            .filter(|f| f.restore)
            .filter_map(|f| mode_of(f).map(|m| (f.path.clone(), m)))
            .collect();
        ConfigWatch {
            files: watched,
            reported: BTreeMap::new(),
            modes,
            failed_restores: 0,
        }
    }

    pub fn watched(&self) -> &[WatchedFile] {
        &self.files
    }

    /// Test-only deterministic point after a restore has pinned its ancestors.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn pause_next_restore(
        path: PathBuf,
        reached: mpsc::SyncSender<()>,
        resume: mpsc::Receiver<()>,
    ) {
        RESTORE_PAUSES
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(path, RestorePause { reached, resume });
    }

    /// Test-only one-shot failure for an ancestor open during restore traversal.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_next_restore_ancestor_open(path: PathBuf, kind: std::io::ErrorKind) {
        crate::fsutil::fail_next_restore_ancestor_open(path, kind);
    }

    /// How many restores this watch could not perform. Each one is already a chained
    /// deviation saying `restored = false`; this is the same fact as a number, for a
    /// health report that wants to say the watch is degraded without re-reading the chain.
    pub fn restore_failures(&self) -> u64 {
        self.failed_restores
    }

    /// Whether anything this watch has reported is **still** away from the digest it was
    /// pinned at — a measurement of the last poll, not a memory of the run.
    ///
    /// The memo is the whole answer, and it clears itself the moment the deviation is over:
    /// [`reported`](Self::reported) loses a path when its digest matches again (by the
    /// participant's own hand, since an app-wide file is never restored by this daemon).
    /// So a file put back reads as matching again here, which is what
    /// lets [`crate::runtime::EntryRuntime`] stop reporting `ConfigMismatch` for the rest of
    /// a run over one edit that has since been undone.
    ///
    /// It answers a fact — "the digests differ" — and not a verdict: whether that difference
    /// was tampering or a participant's own editor is the reviewer's question, and every
    /// deviation is in the chain either way.
    pub fn mismatched(&self) -> bool {
        !self.reported.is_empty()
    }

    /// Compare, report, restore — on the daemon's poll timer, and in a test by being
    /// called.
    ///
    /// A file whose digest and executable bits match is silent. A file whose digest differs
    /// produces one [`ConfigDeviation`] carrying both digests and the moment it was noticed,
    /// and is then put back **if and only if** the tracker wrote it. An absent file deviates
    /// with [`ABSENT`] as its observed digest and is written back out, parent directories
    /// and all, because a deletion took the directory with it as often as not. A file whose
    /// bytes are right and whose executable bit is gone deviates too, with the two digests
    /// equal: `chmod -x` on an installed script stops capture without changing a byte.
    ///
    /// **A failed restore is a deviation, not an error.** It comes back with
    /// `restored = false` and the poll moves on to the next file. Returning `Err` here
    /// would discard every deviation already collected in this pass and - since a caller
    /// that gets an error has nothing to chain - leave the watch reporting nothing at all
    /// for the rest of the run. One unwritable path must not blind the other files.
    pub fn poll(&mut self) -> Result<Vec<ConfigDeviation>, DaemonError> {
        // Split the borrow: the loop reads `files` and `modes` and writes `reported` and
        // `failed_restores`, and they are disjoint fields of one struct.
        let ConfigWatch {
            files,
            reported,
            modes,
            failed_restores,
            ..
        } = self;
        let mut out = Vec::new();
        for file in files.iter() {
            let seen = observe(file)?;
            if seen.sha256 == file.expected_sha256 {
                // The bytes are what the daemon wrote. The bits may not be.
                if let Some(deviation) = mode_deviation(file, &seen.sha256, modes, reported) {
                    if !deviation.restored {
                        *failed_restores += 1;
                    }
                    out.push(deviation);
                    continue;
                }
                // Back to what it should be — by our restore or by the participant's own
                // hand. Either way the next change is a new event.
                reported.remove(&file.path);
                continue;
            }
            if reported.get(&file.path) == Some(&seen.sha256) {
                // Already chained, and unchanged since: the contract is an event per change,
                // not an event per poll.
                continue;
            }

            let restored = if file.restore {
                match restore(
                    &file.path,
                    &file.expected_bytes,
                    modes.get(&file.path).copied(),
                    file.restore_root.as_deref(),
                    &file.relative_path,
                ) {
                    Ok(()) => true,
                    Err(_) => {
                        // Chained as a deviation the daemon could not undo, and memoised
                        // so it is one event rather than one per poll for the rest of the
                        // run. The next file in the list still gets polled.
                        *failed_restores += 1;
                        reported.insert(file.path.clone(), seen.sha256.clone());
                        false
                    }
                }
            } else {
                reported.insert(file.path.clone(), seen.sha256.clone());
                false
            };

            out.push(ConfigDeviation {
                path: file.relative_path.clone(),
                expected_sha256: file.expected_sha256.clone(),
                observed_sha256: seen.sha256,
                detected_at: now_rfc3339(),
                // Digests, never bytes: see the module docs.
                diff: String::new(),
                restored,
                // A restore that did not happen invalidated no trust hash.
                arming_lost: restored && file.restore_invalidates_trust,
            });
        }
        Ok(out)
    }
}

/// What one poll measured at a path.
struct Observed {
    sha256: String,
}

/// The digest of one path, streamed.
///
/// The digest covers every byte and nothing is retained: no config file is ever held whole,
/// and no config file's content is held at all, because a deviation carries digests rather
/// than bytes. A file that cannot be opened or read is [`ABSENT`], the same digest a deleted
/// one gets, because "we could not read it" and "it is not there" are the same statement
/// about the bytes the daemon expected to find.
fn observe(file: &WatchedFile) -> Result<Observed, DaemonError> {
    use std::io::Read;
    let absent = || Observed {
        sha256: ABSENT.to_string(),
    };
    let source = match (file.restore, file.restore_root.as_deref()) {
        (true, Some(root)) => crate::fsutil::open_regular_at(root, &file.relative_path),
        (true, None) => return Ok(absent()),
        (false, _) => crate::fsutil::open_regular_nofollow(&file.path),
    };
    let mut source = match source {
        Ok(f) => f,
        Err(DaemonError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(absent());
        }
        // A directory where a file should be, a permission the participant changed, a
        // broken symlink: the expected bytes are not there, which is a deviation and not
        // a reason for the daemon to stop.
        Err(_) => return Ok(absent()),
    };
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; HASH_CHUNK];
    let mut observed_bytes = 0u64;
    loop {
        match source.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                observed_bytes = observed_bytes.saturating_add(n as u64);
                if observed_bytes > MAX_FILE_BYTES {
                    return Ok(absent());
                }
                hasher.update(&buf[..n]);
            }
            Err(_) => return Ok(absent()),
        }
    }
    Ok(Observed {
        sha256: hex::encode(hasher.finalize()),
    })
}

/// The scope root a tracker file's absolute path was built from: `absolute_path` with
/// `relative_path`'s components popped off.
///
/// `workspace::render_tracker_files` builds every `absolute_path` as
/// `Roots::root_for(scope).join(relative_path)`, so popping the relative components back off
/// returns the workspace, the artifact root or the harness home — the three trees Global
/// Constraint 4 bounds this crate's writes to. Recovered here rather than carried on
/// `TrackerFile` because the type does not hold it and the arithmetic is exact.
fn scope_root(absolute: &Path, relative: &str) -> Option<PathBuf> {
    use std::path::Component;

    if relative
        .split('/')
        .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return None;
    }
    let relative = Path::new(relative);
    let components: Vec<_> = relative
        .components()
        .map(|component| match component {
            Component::Normal(part) => Some(part),
            _ => None,
        })
        .collect::<Option<_>>()?;
    if components.is_empty() {
        return None;
    }

    let mut root = absolute.to_path_buf();
    for _ in &components {
        root.pop().then_some(())?;
    }
    if root.as_os_str().is_empty() {
        return None;
    }
    let rebuilt = components
        .iter()
        .fold(root.clone(), |path, component| path.join(component));
    (rebuilt == absolute).then_some(root)
}

/// A tracker file whose bytes are right and whose executable bit is not.
///
/// `chmod -x hook-post.sh` changes no byte, so a digest comparison is silent about it and
/// capture stops without a word — the failure D6 chose restoring over locking to avoid.
/// Only the executable bits are compared: a participant tightening `0o644` to `0o600` on a
/// config file has not stopped anything, and the watch is not a permissions auditor.
///
/// The mode is put back and the deviation carries the two digests equal, which is the
/// honest report — nothing about the content changed. `arming_lost` is false for the same
/// reason: a hook-trust hash covers a file's bytes, and these are untouched.
fn mode_deviation(
    file: &WatchedFile,
    sha256: &str,
    modes: &BTreeMap<PathBuf, u32>,
    reported: &mut BTreeMap<PathBuf, String>,
) -> Option<ConfigDeviation> {
    let want = modes.get(&file.path).copied()?;
    let have = mode_of(file)?;
    if want & 0o111 == have & 0o111 {
        return None;
    }
    // Keyed by the observed bits so an un-chmod-able file is one event rather than one per
    // poll, and shares `reported` with the content path because the digest matches here and
    // the two can never be memoised at once.
    let key = format!("{sha256}+mode{have:o}");
    if reported.get(&file.path) == Some(&key) {
        return None;
    }
    let restored = match file
        .restore_root
        .as_deref()
        .ok_or_else(|| DaemonError::Guard("tracker file has no approved restore root".to_string()))
        .and_then(|root| {
            crate::fsutil::set_mode_at(
                root,
                &file.relative_path,
                &file.expected_bytes,
                want,
                || {
                    #[cfg(any(test, feature = "test-hooks"))]
                    pause_after_restore_validation(&file.path);
                },
            )
        }) {
        Ok(()) => {
            reported.remove(&file.path);
            true
        }
        Err(_) => {
            reported.insert(file.path.clone(), key);
            false
        }
    };
    Some(ConfigDeviation {
        path: file.relative_path.clone(),
        expected_sha256: file.expected_sha256.clone(),
        observed_sha256: sha256.to_string(),
        detected_at: now_rfc3339(),
        diff: String::new(),
        restored,
        arming_lost: false,
    })
}

/// Put the expected bytes back, creating the parent directory a deletion took with it.
///
/// Only ever called for a [`WatchedFile`] built from a [`TrackerFile`], whose
/// `absolute_path` is a scope root joined with the relative path its connector manifest
/// declares — so every write this module performs lands in the workspace, the artifact root
/// or the harness home (Global Constraint 4). An allowlisted file carries `restore = false`
/// and never reaches here.
///
/// `mode` is the file's permission bits as the tracker installed them, re-applied because
/// `std::fs::write` recreating a deleted file gives it the default mode and a Codex shim
/// that is not executable captures nothing.
///
/// **Never through a link.** The approved root and every ancestor are opened with
/// descriptor-relative `O_NOFOLLOW` operations and retained through repair. The final leaf
/// is also `O_NOFOLLOW | O_NONBLOCK` and verified regular before it is truncated or written.
/// The pathname is re-walked immediately after pinning, so an ancestor exchanged in the
/// setup interval produces a visible failed restore instead of redirecting shim bytes.
fn restore(
    path: &Path,
    bytes: &[u8],
    mode: Option<u32>,
    root: Option<&Path>,
    relative: &str,
) -> Result<(), DaemonError> {
    let root = root.ok_or_else(|| {
        DaemonError::Guard(format!("{} has no approved restore root", path.display()))
    })?;
    let mode = mode.ok_or_else(|| {
        DaemonError::Guard(format!("{} has no known installed mode", path.display()))
    })?;
    crate::fsutil::restore_regular_at(root, relative, bytes, mode, || {
        #[cfg(any(test, feature = "test-hooks"))]
        pause_after_restore_validation(path);
    })
}

/// The permission bits of the regular file at `path`, or `None`.
///
/// The descriptor-relative regular-file opener rejects links and special files before
/// reading mode bits. The same pinned traversal is used later for the corrective chmod.
#[cfg(unix)]
fn mode_of(file: &WatchedFile) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    let root = file.restore_root.as_deref()?;
    crate::fsutil::open_regular_at(root, &file.relative_path)
        .ok()?
        .metadata()
        .ok()
        .map(|m| m.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn mode_of(_file: &WatchedFile) -> Option<u32> {
    None
}

#[cfg(all(test, unix))]
mod tests {
    /// The final component is resolved by the kernel through the already-pinned parent.
    /// A swapped link is unlinked and replaced while its outside target keeps its bytes.
    #[test]
    fn a_link_swapped_in_after_the_check_is_replaced_without_writing_through() {
        let dir = tempfile::tempdir().expect("a tempdir");
        let outside = dir.path().join("zshrc");
        std::fs::write(&outside, b"the participant's own file").expect("the target");
        let tracker = dir.path().join("hooks.json");
        std::os::unix::fs::symlink(&outside, &tracker).expect("the swap");

        let restored = crate::fsutil::restore_regular_at(
            dir.path(),
            "hooks.json",
            b"a receiver token",
            0o644,
            || {},
        );

        assert!(restored.is_ok(), "the tracker leaf was not restored");
        assert_eq!(
            std::fs::read(&outside).expect("the target still reads"),
            b"the participant's own file",
            "the shim's bytes reached a path outside every root"
        );
        assert_eq!(
            std::fs::read(&tracker).expect("the restored tracker file reads"),
            b"a receiver token"
        );
        assert!(
            !std::fs::symlink_metadata(&tracker)
                .expect("the restored tracker file has metadata")
                .file_type()
                .is_symlink(),
            "the final-component symlink survived restoration"
        );

        // The other half: with no link in the way it is the ordinary restore.
        let plain = dir.path().join("plain.json");
        crate::fsutil::restore_regular_at(dir.path(), "plain.json", b"{}", 0o600, || {})
            .expect("a plain path is written");
        assert_eq!(std::fs::read(&plain).expect("it reads"), b"{}");
    }
}
