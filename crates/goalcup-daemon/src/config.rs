//! The daemon's per-machine configuration file.
//!
//! This is the only thing the daemon invents that is not already in `goalcup-protocol`.
//! Everything below is a path root or the control address; nothing here is a rule, and
//! nothing here is read by the verifier. The round's dates are the upload server's: it owns
//! the clock.

use crate::DaemonError;
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// Read through [`DaemonConfigFile`], whose `deny_unknown_fields` is the whole mechanism behind
/// the cut-field assertions in `tests/it/config_load.rs` (`the_cut_fields_are_gone`): without
/// it, a config still setting `unix_socket` or `foreign_scan_interval_ms` would load and be
/// silently ignored — the participant would believe a transport was configured and a
/// scan scheduled, and neither would exist.
#[derive(Debug, Clone, Deserialize)]
#[serde(from = "DaemonConfigFile")]
pub struct DaemonConfig {
    /// Where the daemon writes. **The only tree it writes to outside a tracker-created
    /// workspace**: never `$HOME`, never a harness config directory, never `spikes/`.
    pub artifact_root: PathBuf,
    /// Where the shipped connector directories (`<id>/harness.toml`) live.
    pub connectors_dir: PathBuf,
    /// Where `control.json` lives. Separate from
    /// `artifact_root` because the CLI reads it, and it may not see a capture buffer.
    pub config_dir: PathBuf,
    /// The default workspace parent when the caller does not choose one.
    pub workspace_parent: PathBuf,
    /// The control API's listen address. `validate()` refuses anything but loopback.
    pub control_addr: SocketAddr,
}

/// The file's own keys, each one named so that no other key loads.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DaemonConfigFile {
    artifact_root: PathBuf,
    connectors_dir: PathBuf,
    config_dir: PathBuf,
    workspace_parent: PathBuf,
    control_addr: SocketAddr,
}

impl From<DaemonConfigFile> for DaemonConfig {
    fn from(f: DaemonConfigFile) -> Self {
        DaemonConfig {
            artifact_root: f.artifact_root,
            connectors_dir: f.connectors_dir,
            config_dir: f.config_dir,
            workspace_parent: f.workspace_parent,
            control_addr: f.control_addr,
        }
    }
}

impl DaemonConfig {
    /// Parse without touching the filesystem. `validate()` is a separate call, so a
    /// caller can load a config in order to report what is wrong with it.
    pub fn from_toml_str(s: &str) -> Result<DaemonConfig, DaemonError> {
        toml::from_str(s).map_err(|e| DaemonError::Toml(e.to_string()))
    }

    /// Read and parse `path`. The read is `map_err`'d with the path, not `?`'d: a daemon
    /// that says "No such file or directory" and not which one is a daemon nobody can
    /// configure.
    pub fn load(path: &Path) -> Result<DaemonConfig, DaemonError> {
        let text = std::fs::read_to_string(path).map_err(|source| DaemonError::Io {
            path: path.display().to_string(),
            source,
        })?;
        DaemonConfig::from_toml_str(&text)
    }

    /// What `validate` refuses.
    ///
    /// - **Loopback only.** One control route, the per-entry status, is unauthenticated by
    ///   design (`is_public_route`), and what makes that safe is that the socket is not
    ///   reachable off the machine. Stated here as a refusal so it is an enforced
    ///   property rather than a hope.
    /// - **A workspace parent that is not entangled with the artifact root**. Agents
    ///   enumerate the workspace's parent directory on their first tool call, so
    ///   a `workspace_parent` that contains — or sits inside — the artifact root puts the
    ///   run's own evidence in the agent's field of view.
    pub fn validate(&self) -> Result<(), DaemonError> {
        if !self.control_addr.ip().is_loopback() {
            return Err(DaemonError::Config(format!(
                "control_addr {} is not loopback: the control API's unauthenticated \
                 status route is safe only because the socket is not reachable off this machine",
                self.control_addr
            )));
        }

        for (name, p) in [
            ("artifact_root", &self.artifact_root),
            ("connectors_dir", &self.connectors_dir),
            ("config_dir", &self.config_dir),
            ("workspace_parent", &self.workspace_parent),
        ] {
            if !p.is_absolute() {
                return Err(DaemonError::Config(format!(
                    "{name} must be an absolute path, and it is {}: a daemon that resolves a \
                     root against its own working directory writes somewhere different \
                     depending on who started it",
                    p.display()
                )));
            }
        }

        if nests(&self.workspace_parent, &self.artifact_root) {
            return Err(DaemonError::Config(format!(
                "workspace_parent {} and artifact_root {} are nested: an agent enumerates its \
                 workspace's parent on its first tool call, so the run's own evidence would be \
                 in its field of view",
                self.workspace_parent.display(),
                self.artifact_root.display()
            )));
        }

        Ok(())
    }
}

/// Whether either path is the other, or contains the other. Component-wise, never a
/// substring test: `/tmp/gc/artifacts-old` does not sit inside `/tmp/gc/artifacts`.
///
/// Compared twice, because D5 is a statement about the directory the agent actually
/// enumerates and not about how the two roots were spelled. First lexically, so
/// `/tmp/other/../gc/artifacts/ws` is recognised as sitting inside `/tmp/gc/artifacts`
/// even when nothing on either path exists yet; then, as far as the filesystem will
/// resolve them, so a `workspace_parent` symlinked into the artifact root is recognised
/// too. The two passes disagree, and that is the point: a `..` that crosses a symlink
/// does not mean what it reads as, so the lexical answer for `link/../ws` is not the
/// directory the agent gets. Neither pass is a refinement of the other, which is why
/// both run and why either one finding an entanglement is enough. This refuses, and a
/// refusal that is occasionally too eager costs a participant one edited path, where a
/// miss puts the run's own evidence in the agent's field of view.
///
/// `pub(crate)` because [`crate::workspace::check_artifact_placement`] applies it a
/// second time, to the two *concrete* directories of one entry rather than to the
/// machine-wide roots. That check has clauses this one does not — it names the
/// direction and it refuses a sibling — but the symlink pass belongs here and is
/// reused, not copied.
pub(crate) fn nests(a: &Path, b: &Path) -> bool {
    if overlaps(&lexically_normalized(a), &lexically_normalized(b)) {
        return true;
    }
    overlaps(
        &resolved_as_far_as_possible(a),
        &resolved_as_far_as_possible(b),
    )
}

/// One direction or the other, on paths already normalised by the caller.
fn overlaps(a: &Path, b: &Path) -> bool {
    a == b || a.starts_with(b) || b.starts_with(a)
}

/// Resolve `.` and `..` textually, touching nothing on disk. A purely lexical pass is
/// wrong in general — `..` after a symlink does not mean what it reads as — which is
/// exactly why it is only half of `nests()`; on its own it is the half that works
/// against a root that does not exist yet, which is the normal case on first run.
fn lexically_normalized(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in p.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                // `/..` is `/`: there is nothing above a root to walk up to.
                Some(Component::RootDir) | Some(Component::Prefix(_)) => {}
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                // A leading `..` on a relative path is meaningful and is kept. Absolute
                // paths are all `validate()` ever passes here, so this is defensive.
                _ => out.push(component.as_os_str()),
            },
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// `canonicalize` the longest prefix of `p` that exists — which is what follows
/// symlinks — and re-attach the components below it. Canonicalising the whole path
/// would fail on a root the daemon has not created yet and leave the symlink
/// unresolved; resolving the prefix still catches the case that matters, where the
/// link is a directory somewhere above.
///
/// The prefix is taken from `p` **as written**, never from its lexical normalisation,
/// and that ordering is the whole correctness of this function. `..` after a symlink
/// walks up from the link's *target*, not from where the link sits: with
/// `/tmp/gc/link -> /tmp/gc/artifacts/inner`, the kernel resolves `/tmp/gc/link/../ws`
/// to `/tmp/gc/artifacts/ws`, inside the artifact root. Normalising first collapses that
/// `..` against `link` and yields `/tmp/gc/ws`, a directory that has nothing to do with
/// the one the agent would enumerate — so the entanglement is erased before the
/// filesystem is ever asked. Handing the raw path to `canonicalize` lets the kernel
/// resolve the `..` itself, with the symlink in view.
///
/// Only the components past the longest existing prefix are resolved lexically, and by
/// construction they name nothing on disk, so no symlink can be hiding under them. The
/// canonical base has none left in it either, which is what makes popping for `..`
/// sound here where it was not sound on the raw path.
fn resolved_as_far_as_possible(p: &Path) -> PathBuf {
    use std::path::Component;
    let components: Vec<Component> = p.components().collect();
    // Longest prefix first: the whole path when every component of it exists.
    for cut in (1..=components.len()).rev() {
        let mut prefix = PathBuf::new();
        for component in &components[..cut] {
            prefix.push(component.as_os_str());
        }
        let Ok(base) = std::fs::canonicalize(&prefix) else {
            continue;
        };
        let mut out = base;
        for component in &components[cut..] {
            match component {
                Component::CurDir => {}
                // `pop()` on a root is a no-op, which is the wanted `/..` is `/`.
                Component::ParentDir => {
                    out.pop();
                }
                other => out.push(other.as_os_str()),
            }
        }
        return out;
    }
    // Nothing on this path exists. The lexical comparison is all there is.
    lexically_normalized(p)
}
