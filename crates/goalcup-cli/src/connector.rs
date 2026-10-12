//! The shipped connectors: finding one, and asking whether it is the one this build pins.
//!
//! A connector is a directory under [`crate::config::connectors_dir`] holding a
//! `manifest.toml` and the hook-install bodies beside it. The manifest is the rulebook —
//! the human rule, the exclusion catalogue, the tool classes, the launch recipe — and
//! nothing here interprets any of it. Three questions are asked and no more: which
//! connectors are there, load this one, and is this one the rulebook the verifier will
//! judge the run by.
//!
//! # Loading is the protocol's parser and no other
//!
//! [`load`] is [`ConnectorManifest::from_toml_str`] followed by
//! [`ConnectorManifest::validate`]. A second parser in this crate — even a small one, even
//! for one field — would be a second answer to "what does this manifest say", and the first
//! one to disagree with the daemon's would be the one nobody noticed.
//!
//! # Discovery is by the id the manifest declares
//!
//! Not by joining `id` onto the directory path, which is a traversal waiting for a caller
//! that passes `../..`, and not by the directory's name, which need not be the id. The
//! daemon's own `shipped_connector` reads them the same way and for the same reason
//! (`crates/goalcup-daemon/src/control/handlers.rs`), so the CLI and the daemon cannot
//! disagree about which file a connector id names.
//!
//! # An id belongs to one directory
//!
//! Because the id is declared and not the directory's name, two directories can declare the
//! same one. Neither [`load`] nor [`list`] picks between them: the first in path order would
//! shadow the other silently, and the shadowed one is exactly where an unpinned rulebook
//! would hide. Both refuse instead, naming both paths.

use crate::CliError;
use goalcup_protocol::ProtocolError;
use goalcup_protocol::export::MANIFEST_FILE;
use goalcup_protocol::harness::{LoadedConnector, Origin};
use goalcup_protocol::manifest::ConnectorManifest;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One shipped connector, as it was found on disk.
#[derive(Debug, Clone, PartialEq)]
pub struct Connector {
    pub manifest: ConnectorManifest,
    /// The directory the manifest was read from, which is where its `files/` are.
    pub dir: PathBuf,
    /// SHA-256 of the manifest's **bytes**, lowercase hex.
    ///
    /// Of the bytes and never of a re-serialisation of [`Self::manifest`]: a digest taken
    /// over this build's own TOML writer would compare this build against itself, and the
    /// number the verifier pins is a digest of the file. `ManifestPinning::resolve` says
    /// the same thing on the other side of the run.
    ///
    /// This is the digest that **travels**: it goes to the daemon on
    /// `WorkspaceOpts.manifest_sha256`, onto the arming receipt's claims, and the verifier
    /// recomputes it from the `manifest.toml` the trace carries.
    pub manifest_sha256: String,
    /// SHA-256 of the whole connector **directory**, lowercase hex —
    /// `goalcup_protocol::connector_dir::connector_dir_sha256` over [`Self::dir`].
    ///
    /// The manifest is one file in a directory that also holds the hook bodies the daemon
    /// renders into the workspace, records the rendered digests of, and then guards. A
    /// connector carrying the genuine manifest beside an edited template passes
    /// [`Self::manifest_sha256`] and changes what the run captures, so [`pin_state`] is
    /// resolved from this one.
    ///
    /// **It does not travel.** `WorkspaceOpts` carries no directory digest and must not: the
    /// daemon hashes its own connectors directory and puts that on the start receipt, and a
    /// digest supplied by the CLI would be the checked party's own measurement. This one
    /// answers a question about the CLI's own installation — the question `goalcup doctor`
    /// asks — and [`pin_state`] is what reads it.
    pub dir_sha256: String,
}

/// One schema-1 capture connector, held exactly as the protocol loader read and hashed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessConnector {
    pub dir: PathBuf,
    pub loaded: LoadedConnector,
}

/// Load the schema-1 connector declaring `id` under `dir`.
///
/// Discovery enumerates child directories, then the protocol loader reads each directory once.
/// Selection, duplicate detection, origin and pin policy use only the manifest and bytes held in
/// that [`LoadedConnector`]. A directory name is never derived from caller input. Every candidate
/// must load and validate: an invalid sibling fails the scan rather than hiding a duplicate id.
pub fn load_harness(dir: &Path, id: &str, dev: bool) -> Result<HarnessConnector, CliError> {
    load_harness_with_hook(dir, id, dev, |_| {})
}

fn load_harness_with_hook(
    dir: &Path,
    id: &str,
    dev: bool,
    mut before_load: impl FnMut(&Path),
) -> Result<HarnessConnector, CliError> {
    let mut shipped = Vec::new();
    let mut found: Option<HarnessConnector> = None;
    for connector_dir in harness_dirs(dir)? {
        let candidate = connector_dir.join("harness.toml");
        before_load(&connector_dir);
        let loaded = goalcup_protocol::harness::load_discovered_dir(
            &connector_dir,
            if dev { Origin::Dev } else { Origin::Pinned },
        )
        .map_err(|error| {
            let message = format!("{}: {error}", candidate.display());
            match &error {
                ProtocolError::Field { field, .. } if field == "connector_folder" => {
                    CliError::Refused(message)
                }
                _ => CliError::Internal(message),
            }
        })?;
        if loaded.manifest.id != id {
            shipped.push(loaded.manifest.id);
            continue;
        }
        if let Some(first) = &found {
            return Err(declared_twice(
                id,
                &first.dir.join("harness.toml"),
                &candidate,
            ));
        }
        found = Some(HarnessConnector {
            dir: connector_dir,
            loaded,
        });
    }
    let Some(connector) = found else {
        shipped.sort();
        return Err(CliError::Refused(format!(
            "no harness connector {id:?} in {}. It ships: {}.",
            dir.display(),
            if shipped.is_empty() {
                "nothing".to_string()
            } else {
                shipped.join(", ")
            }
        )));
    };
    Ok(connector)
}

/// Every child `harness.toml`, sorted, including ones that do not parse.
pub fn harness_dirs(dir: &Path) -> Result<Vec<PathBuf>, CliError> {
    goalcup_protocol::harness::discover_connector_dirs(dir).map_err(CliError::from)
}

/// What the release-hash allowlist says about one connector.
///
/// The pin covers the connector **directory** — the manifest and the hook-install bodies
/// beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinState {
    /// Every byte under the connector directory matches the digest this build's verifier
    /// pins.
    Pinned,
    /// This build pins the connector, and this is not it. `pinned` is the digest it should
    /// have had, so a participant can see that the difference is in the connector directory
    /// and not in which connector was named.
    Altered { pinned: String },
    /// No build pins this connector — a hand-written one, or one from a newer release.
    Unpinned,
}

/// The connector declaring `id`, under `dir`.
///
/// Refusals, and the rungs they leave under:
///
/// * `dir` cannot be read — [`CliError::Io`], rung 3, naming it.
/// * nothing under `dir` declares `id` — [`CliError::Refused`], rung 1, naming what ships.
///   That is the rung `ControlError::UnsupportedConnector` already gets, and the fix is the
///   same one: name a connector this build ships.
/// * two directories declare `id` — [`CliError::Refused`], rung 1, naming both. See
///   [`declared_twice`]; the scan runs to the end rather than returning on the first match,
///   which is what makes the second one visible at all.
/// * the manifest is there and does not parse, or does not `validate()` —
///   [`CliError::Internal`], rung 3, naming the file. A rulebook this build cannot read is
///   not something a participant does differently next time.
pub fn load(dir: &Path, id: &str) -> Result<Connector, CliError> {
    let mut shipped = Vec::new();
    let mut unreadable = Vec::new();
    let mut found: Option<PathBuf> = None;
    for candidate in manifests_in(dir)? {
        let manifest = match read_manifest(&candidate) {
            Ok((manifest, _)) => manifest,
            Err(e) => {
                // A neighbour that does not parse must not answer for the one that was
                // asked for. It is carried into the refusal below instead, so a corrupt
                // `codex-cli` does not read as "no such connector" when `codex-cli` is
                // exactly what was asked for.
                unreadable.push(format!("{}: {e}", candidate.display()));
                continue;
            }
        };
        if manifest.id != id {
            shipped.push(manifest.id);
            continue;
        }
        if let Some(first) = &found {
            return Err(declared_twice(id, first, &candidate));
        }
        found = Some(candidate);
    }
    if let Some(candidate) = found {
        // `load_dir` validates and hashes after the scan and not during it. A manifest
        // that failed to validate is still a directory that declared this id, and refusing
        // it as a defect before the twin was even looked at would hide the ambiguity behind
        // whichever one sorts first. The neighbours are walked to find which manifest
        // declares `id`, and hashing each one's `files/` on the way past would read every
        // shipped connector's bytes to answer a question about one.
        return load_dir(candidate.parent().unwrap_or(dir));
    }
    shipped.sort();
    Err(CliError::Refused(format!(
        "no connector {id:?} in {}. It ships: {}.{}",
        dir.display(),
        if shipped.is_empty() {
            "nothing".to_string()
        } else {
            shipped.join(", ")
        },
        if unreadable.is_empty() {
            String::new()
        } else {
            format!(" ({})", unreadable.join("; "))
        }
    )))
}

/// Every connector id under `dir`, sorted.
///
/// A manifest that does not parse is left out rather than fatal: `goalcup doctor` on a
/// machine with one broken connector must still be able to say what the others are.
/// [`load`] is where asking for that one by name reports what is wrong with it.
///
/// `validate()` is deliberately not run here. A manifest that parses declares an id, and an
/// id that exists but is unusable is a better answer than an id that silently is not there
/// — the participant asks for it by name and reads what is wrong with it.
///
/// An id two directories declare is the one thing this does refuse, for the reason in the
/// module header: a roster that listed it once would name a connector without saying which
/// of the two the next `goalcup start` would run.
pub fn list(dir: &Path) -> Result<Vec<String>, CliError> {
    let mut seen: BTreeMap<String, PathBuf> = BTreeMap::new();
    for path in manifests_in(dir)? {
        let Ok((manifest, _)) = read_manifest(&path) else {
            continue;
        };
        if let Some(first) = seen.get(&manifest.id) {
            return Err(declared_twice(&manifest.id, first, &path));
        }
        seen.insert(manifest.id, path);
    }
    // A `BTreeMap`'s keys come out sorted, which is the order this promises.
    Ok(seen.into_keys().collect())
}

/// Every connector directory under `dir`, sorted: one entry per `<dir>/*/manifest.toml`,
/// whether or not that manifest parses.
///
/// [`list`] answers with the ids manifests declare, and a manifest that does not parse
/// declares nothing — so a broken connector is absent from that answer by construction.
/// `goalcup doctor` enumerates these instead, because a connector that vanishes from the
/// report is the one case the report exists to catch.
pub fn dirs(dir: &Path) -> Result<Vec<PathBuf>, CliError> {
    Ok(manifests_in(dir)?
        .into_iter()
        .map(|manifest| {
            manifest
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| dir.to_path_buf())
        })
        .collect())
}

/// The connector in one directory: `<dir>/manifest.toml`, parsed, validated, and digested.
///
/// [`load`] is the by-id door and answers "no such connector"; this is the by-directory one
/// and answers what is wrong with the directory it was handed. Neither parses a manifest
/// itself — [`read_manifest`] is the only reader, so the digest a report shows is the digest
/// a start would compare.
pub fn load_dir(dir: &Path) -> Result<Connector, CliError> {
    let path = dir.join(MANIFEST_FILE);
    let (manifest, sha256) = read_manifest(&path)?;
    manifest
        .validate()
        .map_err(|e| CliError::Internal(format!("{}: {e}", path.display())))?;
    // A symlink or an unreadable file under the directory is `CliError::Protocol` (exit 3,
    // rung 3) — a connector directory this CLI cannot hash is not something a participant
    // does differently next time.
    let dir_sha256 = goalcup_protocol::connector_dir::connector_dir_sha256(dir)?;
    Ok(Connector {
        manifest,
        dir: dir.to_path_buf(),
        manifest_sha256: sha256,
        dir_sha256,
    })
}

/// One id, two directories: rung 1, naming both paths.
///
/// Rung 1 and not rung 3 because there is something to do about it — remove or rename one of
/// the two directories — and it is a machine's arrangement rather than a document this build
/// could not read. Both paths are named because "duplicate connector id" without them sends
/// a participant hunting through a directory for the twin.
fn declared_twice(id: &str, first: &Path, second: &Path) -> CliError {
    CliError::Refused(format!(
        "connector {id:?} is declared by two directories: {} and {}. Discovery keys on the id \
         a manifest DECLARES, so one of these would silently answer for the other, and the \
         one that lost is where an unpinned rulebook would sit unnoticed.",
        first.display(),
        second.display()
    ))
}

/// The release-hash allowlist.
///
/// Compares this connector's loaded bytes against the pinned digest, so `goalcup doctor` can
/// tell a participant that their own installation is not the release's — rather than letting
/// them discover it in a verdict a week later.
///
/// **About the CLI's own copy, and about nothing else.** The daemon does not take this
/// value: it hashes the connector directory it will render the workspace from, compares that
/// against the same table, and claims it on the start receipt. Two machines, two
/// installations, two measurements — and the one that decides a run is the one taken by the
/// process that renders the hook bodies and then guards them.
///
/// # Over the directory, which is where the allowlist has to be widest
///
/// The allowlist covers "manifests **and shims**", and this is the one place both can be checked.
/// A connector is a manifest and the hook bodies beside it under `files/`; the daemon
/// renders those bodies into the workspace, records the rendered digests as the config it
/// will guard, and then guards them. A directory carrying the genuine `manifest.toml`
/// beside an edited tracker body therefore passes any check over the manifest file and still
/// changes what the run captures. So this resolves from `CONNECTOR_DIR_SHA256S`, which covers
/// both.
///
/// The comparison is the one `goalcup_verifier::ManifestPinning::resolve` makes, over the
/// sibling constant, case-insensitively for the same reason: the pins are written lowercase
/// by `scripts/pin-manifests.sh`, and a comparison that depended on that would be a silent
/// trap for anyone who pasted a digest from elsewhere.
pub fn pin_state(c: &Connector) -> PinState {
    match goalcup_protocol::pins::pinned_connector_dir_sha256(&c.manifest.id) {
        None => PinState::Unpinned,
        Some(_)
            if goalcup_protocol::pins::connector_dir_is_pinned(&c.manifest.id, &c.dir_sha256) =>
        {
            PinState::Pinned
        }
        Some(pinned) => PinState::Altered {
            pinned: pinned.to_string(),
        },
    }
}

/// Every `<dir>/*/manifest.toml`, sorted, so discovery does not depend on the order the
/// filesystem hands directories back.
///
/// A directory entry that cannot be read is fatal, just like a `dir` that cannot be opened at
/// all: retained V1 enumeration must not turn an unreadable connector into an absent one.
fn manifests_in(dir: &Path) -> Result<Vec<PathBuf>, CliError> {
    let entries = std::fs::read_dir(dir).map_err(|source| CliError::Io {
        path: dir.display().to_string(),
        source,
    })?;
    manifest_paths_from_entries(dir, entries.map(|entry| entry.map(|entry| entry.path())))
}

fn manifest_paths_from_entries(
    dir: &Path,
    entries: impl IntoIterator<Item = std::io::Result<PathBuf>>,
) -> Result<Vec<PathBuf>, CliError> {
    let mut out = Vec::new();
    for child in entries {
        let child = child.map_err(|source| CliError::Io {
            path: dir.display().to_string(),
            source,
        })?;
        let manifest = child.join(MANIFEST_FILE);
        if manifest.is_file() {
            out.push(manifest);
        }
    }
    out.sort();
    Ok(out)
}

/// One `manifest.toml`, parsed, with the digest of the bytes it was parsed from.
///
/// The read is not bounded, and that is a decision rather than an omission: this directory
/// is the rulebook shipped beside the binary, the daemon reads the same files the same way
/// (`load_manifests`), and a cap here that the daemon did not have would be a machine where
/// `goalcup start` refuses a connector the daemon is perfectly happy to run.
///
/// Invalid UTF-8 is an error and never lossy. A lossy read would hash one document and parse
/// another, which is precisely the gap the allowlist exists to close.
fn read_manifest(path: &Path) -> Result<(ConnectorManifest, String), CliError> {
    let bytes = std::fs::read(path).map_err(|source| CliError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|e| CliError::Internal(format!("{}: not UTF-8: {e}", path.display())))?;
    let manifest = ConnectorManifest::from_toml_str(text)
        .map_err(|e| CliError::Internal(format!("{}: {e}", path.display())))?;
    Ok((manifest, format!("{:x}", Sha256::digest(&bytes))))
}

#[cfg(test)]
mod harness_load_tests {
    use super::*;

    #[test]
    fn a_requested_pinned_id_cannot_turn_into_an_unpinned_dev_identity_at_load() {
        let tmp = tempfile::tempdir().unwrap();
        let candidate = tmp.path().join("candidate");
        std::fs::create_dir(&candidate).unwrap();
        let shipped = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../connectors/claude-code-cli/harness.toml");
        let original = std::fs::read_to_string(shipped).unwrap();
        std::fs::write(candidate.join("harness.toml"), &original).unwrap();

        let error = load_harness_with_hook(tmp.path(), "claude-code-cli", true, |dir| {
            let path = dir.join("harness.toml");
            let changed = std::fs::read_to_string(&path).unwrap().replacen(
                "id = \"claude-code-cli\"",
                "id = \"untrusted-dev-cli\"",
                1,
            );
            std::fs::write(path, changed).unwrap();
        })
        .expect_err("the identity selected for a pinned request changed before its one load");

        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("claude-code-cli"), "{error}");
        assert!(error.to_string().contains("untrusted-dev-cli"), "{error}");
    }

    #[test]
    fn retained_v1_discovery_propagates_an_iterator_entry_error() {
        let root = Path::new("/connector-root");
        let entries = vec![Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "synthetic unreadable directory entry",
        ))];

        let error = manifest_paths_from_entries(root, entries)
            .expect_err("a V1 directory entry error is not an absent manifest");
        assert!(matches!(error, CliError::Io { path, source }
            if path == root.display().to_string()
                && source.kind() == std::io::ErrorKind::PermissionDenied));
    }
}
