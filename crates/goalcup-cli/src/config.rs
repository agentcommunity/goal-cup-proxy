//! The tracker's own configuration directory, and the documents in it.
//!
//! One promise is kept here and nowhere else: **the tracker's state has a directory of its
//! own and never borrows a harness's.** Not `~/.claude` and not `~/.codex`.
//! A tracker that wrote its grant into a harness's home would be handing the agent under
//! measurement the credential that starts runs, and an agent that can enumerate its own
//! home would find it there on its first tool call.
//!
//! [`config_dir`] resolves `$GOALCUP_CONFIG_DIR` first, which is why no test ever
//! needs `$HOME`: a test sets the variable to a temporary directory and everything below
//! follows it.
//!
//! # What lives there
//!
//! | Path | Written by | Held here |
//! |---|---|---|
//! | `control.json` | the daemon | [`read_control_file`] reads it; the CLI never writes it |
//! | `daemon.toml` | `goalcup start`, once, if missing | the configuration the CLI starts the daemon from (`daemon::spawn`) |
//! | `daemon.log` | a daemon the CLI started | its startup lines and any refusal |
//! | `grant.json` | the V1 `goalcup auth` (removed by D48) | nothing: a leftover one is ignored |
//! | `entries/` | the CLI | one record per entry `goalcup start` created |

use crate::CliError;
use goalcup_platform::Platform;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Where the tracker keeps its own state. Read once, by [`config_dir`].
pub const CONFIG_DIR_ENV: &str = "GOALCUP_CONFIG_DIR";

/// Where the shipped connector manifests are. Read once, by [`connectors_dir`].
pub const CONNECTORS_DIR_ENV: &str = "GOALCUP_CONNECTORS_DIR";

/// The last component of the default configuration directory, whichever per-user
/// application directory the platform puts it under.
#[cfg(any(test, feature = "test-support"))]
pub const DEFAULT_CONFIG_DIR_NAME: &str = "goalcup";

/// The file the daemon writes to say where it is. Its name, and its shape below, are the
/// daemon's: `crates/goalcup-daemon/src/control/file.rs`.
pub(crate) const CONTROL_FILE_NAME: &str = "control.json";

/// The most of `control.json` [`read_control_file`] will read.
///
/// The daemon's own is six short fields — a few hundred bytes — so this is three orders of
/// magnitude of headroom and still a bound. Unbounded is the thing it must not be: the path
/// is under a directory the participant owns, and a run that pointed `$GOALCUP_CONFIG_DIR`
/// at a directory holding a large file, by accident or otherwise, would have had this
/// process read the whole of it into memory before serde ever looked at the first byte.
pub const CONTROL_FILE_MAX_BYTES: u64 = 64 * 1024;

/// The mode every secret this CLI writes ends up at, and the mode the directory holding one
/// is created at.
#[cfg(unix)]
const SECRET_MODE: u32 = 0o600;
#[cfg(unix)]
const PRIVATE_DIR_MODE: u32 = 0o700;

// ------------------------------------------------------------------ resolution

/// The tracker's configuration directory: `$GOALCUP_CONFIG_DIR`, else the platform's
/// per-user application directory with `goalcup` as the last component.
///
/// The platform default is `~/Library/Application Support/goalcup` on macOS,
/// `$XDG_CONFIG_HOME/goalcup`, else `~/.config/goalcup` on Unix, and
/// `$LOCALAPPDATA/goalcup`, else `$USERPROFILE/AppData/Local/goalcup` on Windows. **Never a
/// harness's**:
/// not `~/.claude` and not `~/.codex`.
///
/// **It resolves and does not create.** A `goalcup doctor` run on a machine where nothing
/// has ever started must be able to say "there is no daemon, and here is where its file
/// would have been" without leaving a directory behind that makes the next run's answer
/// different. Whatever writes into this directory creates it — [`write_secret_file`] does,
/// at mode 0700.
///
/// The path is returned exactly as it was given. It is deliberately not canonicalised: on
/// macOS `/var` is a link to `/private/var`, so a caller that handed over a temporary
/// directory and got a different string back would have to know that to compare the two.
pub fn config_dir() -> Result<PathBuf, CliError> {
    config_dir_from(|name| std::env::var_os(name))
}

/// [`config_dir`] with the environment handed in, so the four branches below can be tested
/// without a test mutating the environment of the process it is running in.
///
/// `lookup` answers the same question `std::env::var_os` does. It is here rather than
/// a `#[cfg(test)]` back door because the resolution rule — which variable wins, and what an
/// exported-but-empty one means — is the part of this module worth pinning, and a rule
/// nothing can call with a made-up environment is a rule nothing tests.
pub fn config_dir_from(lookup: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, CliError> {
    config_dir_on(current_platform(), lookup)
}

/// [`config_dir_from`] with the platform handed in too, so its participant-facing refusal can
/// remain at the CLI boundary.
///
fn config_dir_on(
    platform: Platform,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Result<PathBuf, CliError> {
    goalcup_platform::config_dir_with(platform, lookup).ok_or_else(|| {
        CliError::Refused(format!(
            "no configuration directory: neither ${CONFIG_DIR_ENV} nor a platform configuration \
             root is set (Windows: $LOCALAPPDATA or $USERPROFILE; macOS: $HOME; Unix: \
             $XDG_CONFIG_HOME or $HOME). Set ${CONFIG_DIR_ENV} to the directory /goal Cup \
             should keep its state in."
        ))
    })
}

fn current_platform() -> Platform {
    #[cfg(target_os = "macos")]
    {
        Platform::Macos
    }
    #[cfg(not(target_os = "macos"))]
    {
        Platform::Unix
    }
}

/// The shipped connector manifests: `$GOALCUP_CONNECTORS_DIR`, else
/// `<exe dir>/../share/goalcup/connectors` for an installed build, else
/// `<exe dir>/../../connectors` — the repository's `connectors/` from `target/debug`.
///
/// Beside the binary and never under [`config_dir`], because the manifests are the
/// rulebook: the human rule, the exclusion catalogue and the tool classes, with their
/// digests pinned by the verifier. A copy under a directory the participant writes would be
/// a rulebook the participant edits.
///
/// Nothing here creates a directory, and the first candidate that exists wins — so a
/// developer running out of `target/debug` finds the checkout's `connectors/` without
/// setting anything, and an installed build never silently falls back to a checkout that
/// happens to be two levels above it, because that is not where an installed binary lives.
/// Both candidates are tried beside the path the executable was started by, then beside its
/// resolved path ([`connectors_beside`]).
pub fn connectors_dir() -> Result<PathBuf, CliError> {
    if let Some(dir) = non_empty_var(CONNECTORS_DIR_ENV) {
        return Ok(PathBuf::from(dir));
    }
    let exe = std::env::current_exe().map_err(|source| CliError::Io {
        path: "the running executable".to_string(),
        source,
    })?;
    if let Some(dir) = connectors_beside(&exe) {
        return Ok(dir);
    }
    Err(CliError::Refused(format!(
        "cannot find the connectors directory beside {}. Set ${CONNECTORS_DIR_ENV} to the \
         shipped `connectors/` — in a checkout, this repository's.",
        exe.display()
    )))
}

/// The first `../share/goalcup/connectors` or `../../connectors` beside `exe` as given, then
/// beside its resolved path, that is a directory holding no link.
///
/// As given first, so an installed build keeps a path that survives upgrades: Homebrew starts
/// `/opt/homebrew/bin/goalcup`, a link into a versioned keg, macOS reports the link, and
/// `daemon.toml` records `/opt/homebrew/bin/../share/goalcup/connectors` rather than a keg
/// that `brew upgrade` deletes. The resolved path serves the rest: a prefix whose `share` is
/// not linked, or one that links the connector files one by one, which the loader would refuse
/// (a pinned connector is its own bytes); the keg behind the link holds ordinary files. When
/// every candidate holds a link, the first that exists is returned, for the loader to refuse by
/// name.
fn connectors_beside(exe: &Path) -> Option<PathBuf> {
    let resolved = std::fs::canonicalize(exe).ok();
    let candidates: Vec<PathBuf> = [Some(exe), resolved.as_deref()]
        .into_iter()
        .flatten()
        .filter_map(Path::parent)
        .flat_map(|dir| {
            ["../share/goalcup/connectors", "../../connectors"].map(|candidate| dir.join(candidate))
        })
        .filter(|path| path.is_dir())
        .collect();
    let clean = candidates.iter().find(|path| !holds_link(path));
    clean.or(candidates.first()).cloned()
}

/// Whether anything below `dir` is a symbolic link, or cannot be read. `dir` itself may be
/// reached through links; only what is inside it is loaded and pinned.
fn holds_link(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return true;
    };
    entries.into_iter().any(|entry| {
        let Ok(entry) = entry else { return true };
        match entry.file_type() {
            Ok(kind) if kind.is_symlink() => true,
            Ok(kind) if kind.is_dir() => holds_link(&entry.path()),
            Ok(_) => false,
            Err(_) => true,
        }
    })
}

/// `$HOME`-style lookup that treats an empty value as unset.
///
/// An exported-but-empty variable is what a shell script that meant to compute a path and
/// computed nothing leaves behind. Honouring it would resolve the configuration directory
/// to the process's working directory, which is the participant's repository during a run.
fn non_empty_var(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}

// ------------------------------------------------------------------ the paths in it

/// `<cfg>/control.json` — where the daemon said it is.
pub fn control_path(cfg: &Path) -> PathBuf {
    cfg.join(CONTROL_FILE_NAME)
}

// ------------------------------------------------------------------ writing a secret

/// Put `bytes` at `path` at mode 0600, creating the directory above it at 0700.
///
/// The write is a fresh temp file and a rename, and **`path` itself is never opened**. That
/// is what makes the mode unconditional — a file an earlier run left behind with looser
/// bits is replaced rather than reopened — and what keeps a link planted at `path` from
/// carrying the grant into a file of the planter's choosing at whatever mode that file
/// already had. The daemon's `control.json` writer takes the same shape and says the same
/// thing about it (`crates/goalcup-daemon/src/control/file.rs`).
///
/// The temp name carries this process's id, so two `goalcup` runs cannot write each other's
/// bytes, and a leftover from a run killed between the create and the rename is removed
/// rather than colliding.
pub fn write_secret_file(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    use std::io::Write;

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        create_private_dir(parent)?;
    }
    let temp = temp_name(path)?;
    // A leftover from a run of this process id that died between the create below and the
    // rename. `create_new` would otherwise refuse it and report "another writer holds it"
    // about a writer that is long dead.
    if let Err(source) = std::fs::remove_file(&temp)
        && source.kind() != std::io::ErrorKind::NotFound
    {
        return Err(CliError::Io {
            path: temp.display().to_string(),
            source,
        });
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(SECRET_MODE);
    }
    let io = |path: &Path| {
        let path = path.display().to_string();
        move |source| CliError::Io {
            path: path.clone(),
            source,
        }
    };
    let mut file = options.open(&temp).map_err(io(&temp))?;
    file.write_all(bytes).map_err(io(&temp))?;
    file.sync_all().map_err(io(&temp))?;
    drop(file);
    std::fs::rename(&temp, path).map_err(io(path))
}

/// `<dir>` at mode 0700, created if it is not there. An existing directory is left alone:
/// a participant who tightened the bits on their own configuration directory keeps them.
pub(crate) fn create_private_dir(dir: &Path) -> Result<(), CliError> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(PRIVATE_DIR_MODE);
    }
    builder.create(dir).map_err(|source| CliError::Io {
        path: dir.display().to_string(),
        source,
    })
}

/// `<path>.<pid>.tmp`, beside `path`. A rename is only atomic within one filesystem, so the
/// temp file has to be in the same directory as its target.
fn temp_name(path: &Path) -> Result<PathBuf, CliError> {
    let name = path.file_name().ok_or_else(|| {
        CliError::Internal(format!(
            "{} does not name a file, so there is nowhere to write it",
            path.display()
        ))
    })?;
    let mut temp = name.to_os_string();
    temp.push(format!(".{}.tmp", std::process::id()));
    Ok(path.with_file_name(temp))
}

// ------------------------------------------------------------------ control.json

/// `<config_dir>/control.json`, field for field as the daemon writes it
/// (`crates/goalcup-daemon/src/control/file.rs`, plain `Serialize`, no renames).
///
/// **There is no transport field and no path field.** A key naming one is ignored here
/// rather than honoured: this CLI speaks HTTP to `address` and nothing else.
/// `contract_gate.rs` greps this file to keep that true.
///
/// A field this CLI does not know is ignored rather than refused, which is what lets a
/// newer daemon add one without breaking an older `goalcup`. What must not drift is
/// `control_api_version`: a daemon speaking a version this build does not know is refused
/// by the client rather than mis-read.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlFile {
    /// What the daemon speaks. A client with a different one refuses the daemon rather
    /// than mis-reading its JSON.
    pub control_api_version: u32,
    pub daemon_version: String,
    /// `127.0.0.1:<port>`, ready to be pasted into a URL.
    pub address: String,
    pub port: u16,
    /// The CLI's bearer. Per daemon, not per run, and it never goes into a workspace where
    /// the agent can read it.
    pub token: String,
}

/// Every field as a derive would print it, except `token`, which prints `<redacted>`.
///
/// This struct is where the daemon's bearer lives in this process, and a `{:?}` that
/// carried it — into an error message, a log line, a `goalcup doctor` dump — would hand a
/// reader the credential every authenticated route trusts. The daemon's own `ControlFile`
/// redacts the same field for the same reason.
impl std::fmt::Debug for ControlFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlFile")
            .field("control_api_version", &self.control_api_version)
            .field("daemon_version", &self.daemon_version)
            .field("address", &self.address)
            .field("port", &self.port)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// `<cfg>/control.json` as the running daemon wrote it.
///
/// **A missing file is [`CliError::Refused`] — rung 1 — naming the path.** "There is no
/// daemon" is the answer `goalcup doctor` came for, the path is where the daemon's file
/// would have been, and the fix is to start one: that is the definition of rung 1, and it
/// is the rung the ladder already gives `ControlError::Unreachable`, which is the same
/// answer arrived at one step later. Reporting it as [`CliError::Io`] would put "no daemon is
/// running" on rung 3 beside "this CLI is broken", and a script branching on the number
/// could not tell the two apart. Every other `open` failure — permission denied, a
/// directory where the file should be — stays [`CliError::Io`], because none of those is
/// "there is no daemon".
///
/// The read is bounded at [`CONTROL_FILE_MAX_BYTES`] and a file over the cap is refused
/// naming it, rather than read into memory whole and then rejected by serde.
///
/// Nothing in this crate prints what comes back.
pub fn read_control_file(cfg: &Path) -> Result<ControlFile, CliError> {
    use std::io::Read;

    let path = control_path(cfg);
    let io = |source: std::io::Error| CliError::Io {
        path: path.display().to_string(),
        source,
    };
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(CliError::Refused(format!(
                "no daemon is running: there is no {}. `goalcup start` starts one for a run, \
                 and `goalcup seal` starts one to finish a run that was not sealed.",
                path.display()
            )));
        }
        Err(source) => return Err(io(source)),
    };
    // One byte past the cap, so a file exactly at it still reads and anything larger is
    // caught having read the cap and no more.
    let mut text = String::new();
    let read = file
        .take(CONTROL_FILE_MAX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(io)?;
    if read as u64 > CONTROL_FILE_MAX_BYTES {
        return Err(CliError::Refused(format!(
            "{} is larger than {CONTROL_FILE_MAX_BYTES} bytes, which is more than any \
             control.json a daemon writes. Remove it and start the daemon again.",
            path.display()
        )));
    }
    Ok(serde_json::from_str(&text)?)
}

// ------------------------------------------------------------------ resolution, checked

/// The resolution rule, on both platforms, with the environment handed in.
///
/// These are in `src/` and not in `tests/` because [`config_dir_on`] is private: what is
/// being pinned is which variable wins and what an exported-but-empty one means, and the
/// integration tests can only see the answer after the platform has already been chosen.
///
/// **No `unwrap`, `expect` or `panic!` here either.** The crate's rule is about `src/`, and
/// a test module inside `src/` is inside `src/`. A `Result` is turned into a comparable
/// value and asserted on; nothing is unwrapped.
#[cfg(test)]
mod tests {
    use super::*;

    /// A Homebrew-shaped install: the binary and its connectors in a versioned keg, and a prefix
    /// whose `bin/goalcup` links into the keg.
    #[cfg(unix)]
    #[test]
    fn connectors_are_found_beside_the_link_unless_they_are_links() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let keg = root.path().join("Cellar/goalcup/1.0.0");
        let keg_connectors = keg.join("share/goalcup/connectors");
        std::fs::create_dir_all(keg.join("bin")).unwrap();
        std::fs::create_dir_all(keg_connectors.join("codex-cli")).unwrap();
        std::fs::write(keg_connectors.join("codex-cli/harness.toml"), "").unwrap();
        std::fs::write(keg.join("bin/goalcup"), "").unwrap();
        let prefix = root.path().join("prefix");
        let linked_bin = prefix.join("bin/goalcup");
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        symlink(keg.join("bin/goalcup"), &linked_bin).unwrap();

        // The prefix's share not linked: the keg's.
        let found = connectors_beside(&linked_bin).unwrap();
        assert_eq!(
            std::fs::canonicalize(&found).unwrap(),
            std::fs::canonicalize(&keg_connectors).unwrap()
        );

        // share/goalcup linked as one directory, as Homebrew does: the prefix path, which
        // outlives the keg.
        std::fs::create_dir_all(prefix.join("share")).unwrap();
        symlink(keg.join("share/goalcup"), prefix.join("share/goalcup")).unwrap();
        let found = connectors_beside(&linked_bin).unwrap();
        assert_eq!(found, prefix.join("bin/../share/goalcup/connectors"));

        // The connector files linked one by one: the keg's ordinary files instead.
        std::fs::remove_file(prefix.join("share/goalcup")).unwrap();
        let file_linked = prefix.join("share/goalcup/connectors/codex-cli");
        std::fs::create_dir_all(&file_linked).unwrap();
        symlink(
            keg_connectors.join("codex-cli/harness.toml"),
            file_linked.join("harness.toml"),
        )
        .unwrap();
        let found = connectors_beside(&linked_bin).unwrap();
        assert_eq!(
            std::fs::canonicalize(&found).unwrap(),
            std::fs::canonicalize(&keg_connectors).unwrap()
        );

        // Nothing beside a binary in a directory of its own.
        let bare = root.path().join("a/b/goalcup");
        std::fs::create_dir_all(bare.parent().unwrap()).unwrap();
        std::fs::write(&bare, "").unwrap();
        assert_eq!(connectors_beside(&bare), None);
    }

    /// A lookup over a fixed table, so each case below reads as the environment it means.
    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let table: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |name| {
            table
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn the_override_wins_on_either_platform_and_is_taken_exactly_as_given() {
        for platform in [Platform::Macos, Platform::Unix, Platform::Windows] {
            let resolved = config_dir_on(
                platform,
                env(&[
                    (CONFIG_DIR_ENV, "/srv/goal"),
                    ("HOME", "/home/p"),
                    ("XDG_CONFIG_HOME", "/home/p/xdg"),
                ]),
            );
            assert_eq!(
                resolved.ok(),
                Some(PathBuf::from("/srv/goal")),
                "${CONFIG_DIR_ENV} beats every default ({platform:?})"
            );
        }
    }

    #[test]
    fn macos_puts_it_under_library_application_support() {
        let resolved = config_dir_on(Platform::Macos, env(&[("HOME", "/Users/p")]));
        assert_eq!(
            resolved.ok(),
            Some(PathBuf::from(
                "/Users/p/Library/Application Support/goalcup"
            )),
            "the per-user application directory macOS actually uses"
        );
        // And $XDG_CONFIG_HOME is not macOS's rule, even when it is set.
        let with_xdg = config_dir_on(
            Platform::Macos,
            env(&[("HOME", "/Users/p"), ("XDG_CONFIG_HOME", "/Users/p/xdg")]),
        );
        assert_eq!(
            with_xdg.ok(),
            Some(PathBuf::from(
                "/Users/p/Library/Application Support/goalcup"
            ))
        );
    }

    #[test]
    fn elsewhere_it_is_xdg_config_home_when_that_is_set() {
        let resolved = config_dir_on(
            Platform::Unix,
            env(&[("HOME", "/home/p"), ("XDG_CONFIG_HOME", "/home/p/xdg")]),
        );
        assert_eq!(
            resolved.ok(),
            Some(PathBuf::from("/home/p/xdg/goalcup")),
            "$XDG_CONFIG_HOME is honoured where it is the platform's rule"
        );
        // An exported-but-empty variable is what a script that meant to compute a path and
        // computed nothing leaves behind. It is unset, not the working directory.
        let empty = config_dir_on(
            Platform::Unix,
            env(&[("HOME", "/home/p"), ("XDG_CONFIG_HOME", "")]),
        );
        assert_eq!(
            empty.ok(),
            Some(PathBuf::from("/home/p/.config/goalcup")),
            "an empty $XDG_CONFIG_HOME is unset, and the fallback is ~/.config"
        );
    }

    #[test]
    fn elsewhere_it_falls_back_to_dot_config_under_home() {
        let resolved = config_dir_on(Platform::Unix, env(&[("HOME", "/home/p")]));
        assert_eq!(
            resolved.ok(),
            Some(PathBuf::from("/home/p/.config/goalcup")),
            "no $XDG_CONFIG_HOME, so ~/.config"
        );
    }

    #[test]
    fn no_home_and_no_override_is_a_refusal_that_names_the_variable_to_set() {
        for platform in [Platform::Windows, Platform::Macos, Platform::Unix] {
            // An empty $HOME counts as unset for the same reason an empty $XDG_CONFIG_HOME
            // does: honouring it would resolve the tracker's state to the process's working
            // directory, which during a run is the participant's own repository.
            for home in [None, Some("")] {
                let pairs: Vec<(&str, &str)> = home.map(|h| vec![("HOME", h)]).unwrap_or_default();
                let (code, said) = match config_dir_on(platform, env(&pairs)) {
                    Ok(dir) => (0, dir.display().to_string()),
                    Err(e) => (e.exit_code(), e.to_string()),
                };
                assert_eq!(
                    code, 1,
                    "no directory to resolve is a refusal the participant can fix, and it \
                     resolved to {said:?} instead ({platform:?}, HOME={home:?})"
                );
                assert!(
                    said.contains(CONFIG_DIR_ENV),
                    "the refusal names the variable that fixes it: {said:?}"
                );
                assert!(
                    said.contains("LOCALAPPDATA") && said.contains("USERPROFILE"),
                    "the refusal accurately names Windows defaults: {said:?}"
                );
            }
        }
    }

    /// Nothing above ever creates a directory: `goalcup doctor` on a machine where nothing
    /// has started must not leave the next run a different answer.
    #[test]
    fn resolving_creates_nothing() {
        let before = PathBuf::from("/nonexistent-goalcup-root/p");
        let resolved = config_dir_on(
            Platform::Unix,
            env(&[("HOME", "/nonexistent-goalcup-root/p")]),
        );
        assert_eq!(resolved.ok(), Some(before.join(".config/goalcup")));
        assert!(!before.exists(), "resolution created {}", before.display());
    }
}
