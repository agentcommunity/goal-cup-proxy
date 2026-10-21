//! The loader: one connector directory read, hashed and parsed in one walk, and the `Dev`
//! guard against a pinned directory.

use super::*;
use crate::ProtocolError;
use crate::capture_policy::seed_content_problem;
use crate::connector_dir::hash_connector_dir_listing;
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------------------
// The loader
// ---------------------------------------------------------------------------------------

/// A connector directory, loaded: the harness manifest, the exact bytes of `harness.toml` that
/// went into the directory digest, the seed bytes the same walk read, and the digest.
///
/// Everything a caller writes or parses came out of the one read the digest covers, so a
/// second read of the directory can never authorize bytes the digest did not cover.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LoadedConnector {
    pub manifest: HarnessManifest,
    /// The `harness.toml` bytes that were hashed and parsed: one read, so a second read can
    /// never authorize bytes the digest did not cover.
    pub harness_toml: Vec<u8>,
    /// The manifest's seed files, in manifest order, with the bytes the directory walk hashed.
    /// The per-run home writer writes these and never re-reads `files/**` (M-6).
    pub seeds: Vec<LoadedSeed>,
    /// SHA-256 of the whole directory, lowercase hex ([`crate::connector_dir::connector_dir_sha256`]).
    /// This is what `Launch.connector_dir_sha256` records.
    pub connector_dir_sha256: String,
}

/// One seed file as the directory walk read it.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LoadedSeed {
    /// The manifest's `capture.home.seed[].file`: the path under `files/`, and the destination
    /// under the per-run config home.
    pub file: String,
    /// The mode it is written with.
    pub mode: SeedMode,
    /// Its bytes, as hashed.
    pub bytes: Vec<u8>,
}

/// Discover schema-1 connector directories as sorted immediate children of `root`.
///
/// A child directory with no `harness.toml` is not a schema-1 connector and is skipped. Once the
/// name exists it must be a regular file: a directory, symlink, device or other non-regular entry
/// is invalid rather than silently indistinguishable from absence. Root and iterator-entry errors
/// propagate; filesystem enumeration order never selects a connector.
pub fn discover_connector_dirs(root: &Path) -> Result<Vec<PathBuf>, ProtocolError> {
    let entries = std::fs::read_dir(root).map_err(|source| ProtocolError::Io {
        path: root.display().to_string(),
        source,
    })?;
    discover_connector_dirs_from(root, entries.map(|entry| entry.map(|entry| entry.path())))
}

fn discover_connector_dirs_from(
    root: &Path,
    entries: impl IntoIterator<Item = std::io::Result<PathBuf>>,
) -> Result<Vec<PathBuf>, ProtocolError> {
    let mut discovered = Vec::new();
    for child in entries {
        let child = child.map_err(|source| ProtocolError::Io {
            path: root.display().to_string(),
            source,
        })?;
        let child_kind = std::fs::symlink_metadata(&child).map_err(|source| ProtocolError::Io {
            path: child.display().to_string(),
            source,
        })?;
        if child_kind.file_type().is_symlink() {
            return Err(ProtocolError::UnhashableEntry {
                path: child.display().to_string(),
                reason: "an immediate connector child is a symlink, so its bytes are outside the \
                         directory being discovered"
                    .into(),
            });
        }
        if !child_kind.is_dir() {
            continue;
        }

        let manifest = child.join("harness.toml");
        match std::fs::symlink_metadata(&manifest) {
            Ok(metadata) if metadata.is_file() => discovered.push(child),
            Ok(_) => {
                return Err(ProtocolError::UnhashableEntry {
                    path: manifest.display().to_string(),
                    reason: "harness.toml exists but is not a regular file".into(),
                });
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(ProtocolError::Io {
                    path: manifest.display().to_string(),
                    source,
                });
            }
        }
    }
    discovered.sort();
    Ok(discovered)
}

/// Prints the name, the mode and the size of a seed, never its bytes: a seed can be a credential
/// stand-in, and a `{:?}` on a loaded connector reaches terminals and logs.
impl std::fmt::Debug for LoadedSeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LoadedSeed {{ file: {:?}, mode: {:?}, bytes: <{} bytes> }}",
            self.file,
            self.mode,
            self.bytes.len()
        )
    }
}

impl LoadedConnector {
    /// SHA-256 of the captured `harness.toml` bytes, lowercase hex: the profile digest. The
    /// bytes are the ones the directory digest covers and the manifest was parsed from, so the
    /// two digests describe one read.
    pub fn profile_sha256(&self) -> String {
        hex::encode(Sha256::digest(&self.harness_toml))
    }
}

/// Load one connector directory: the one loader the CLI, the daemon and the conformance test
/// share.
///
/// # `origin`
///
/// `origin` is the caller's word, and nothing in a manifest can claim `Dev`, so the loader
/// holds the line itself (S2 review M-1): `load_dir(_, Origin::Dev)` refuses a directory whose
/// id this build pins, or whose digest equals any pin. `Dev` is for an unpinned directory under
/// `GOALCUP_CONNECTORS_DIR` loaded with `--dev` (the CLI is the only door: the daemon cannot see
/// `--dev`). The origin is not recorded in `Launch` (the v5 shape is closed until the tag);
/// `Launch.connector_dir_sha256` against the pins says the same to a verifier.
///
/// `Pinned` does not check the pin: the caller compares [`LoadedConnector::connector_dir_sha256`]
/// with [`crate::pins`], as the daemon's preflight and the CLI's `pin_state` do.
///
/// # Reading
///
/// * It hashes the directory and parses the exact `harness.toml` bytes that went into the
///   hash, then validates for `origin`.
/// * It refuses any regular file outside `harness.toml` and the `files/**` entries
///   `capture.home.seed` names (a leftover V1 `manifest.toml` included, OS-29).
/// * Every seed must exist under `files/` and weigh at most [`MAX_SEED_BYTES`].
///
/// A symlink anywhere in the directory, or as the directory itself, is refused.
pub fn load_dir(dir: &Path, origin: Origin) -> Result<LoadedConnector, ProtocolError> {
    load_dir_resolving_origin(dir, |_, _| origin)
}

/// Load a directory reached through [`discover_connector_dirs`] and apply the protocol-owned
/// folder/id/digest admission rule before validation.
///
/// `unpinned_origin` is the caller's one remaining policy choice. The CLI supplies `Dev` only
/// when the human typed `--dev`; the daemon supplies `Dev` for its development-connector path.
/// A pinned row always validates as [`Origin::Pinned`].
pub fn load_discovered_dir(
    dir: &Path,
    unpinned_origin: Origin,
) -> Result<LoadedConnector, ProtocolError> {
    let folder_name = dir.file_name().unwrap_or_else(|| OsStr::new(""));
    load_dir_resolving_origin_fallible(dir, |id, digest| {
        match crate::pins::connector_folder_admission(folder_name, id, digest)? {
            crate::pins::ConnectorFolderAdmission::Pinned => Ok(Origin::Pinned),
            crate::pins::ConnectorFolderAdmission::Unpinned => Ok(unpinned_origin),
        }
    })
}

/// Load one connector directory and resolve its validation origin from the identity and digest
/// captured by that same load.
///
/// The resolver is called exactly once, after the directory has been walked, hashed and its
/// captured `harness.toml` bytes parsed, but before the pinned-directory guard and manifest
/// validation run. Its arguments are the captured manifest id and connector-directory digest.
/// Filesystem changes made by the resolver are not reread: the returned [`LoadedConnector`], its
/// validation and the pinned-id/digest guard all continue to use the bytes already captured.
///
/// This is the discovery form of [`load_dir`]. Callers that already know the origin should use
/// [`load_dir`]; callers discovering several directories can choose `Pinned` or `Dev` from each
/// directory's authoritative captured identity without parsing `harness.toml` a second time.
pub fn load_dir_resolving_origin(
    dir: &Path,
    resolve_origin: impl FnOnce(&str, &str) -> Origin,
) -> Result<LoadedConnector, ProtocolError> {
    load_dir_resolving_origin_fallible(dir, |id, digest| Ok(resolve_origin(id, digest)))
}

fn load_dir_resolving_origin_fallible(
    dir: &Path,
    resolve_origin: impl FnOnce(&str, &str) -> Result<Origin, ProtocolError>,
) -> Result<LoadedConnector, ProtocolError> {
    // The root itself: the walk below refuses a symlink INSIDE the directory, but `read_dir`
    // follows a link given as the root, which would load and pin bytes from somewhere else.
    match std::fs::symlink_metadata(dir) {
        Ok(m) if m.file_type().is_symlink() => {
            return Err(refuse(
                "connector_dir",
                format!(
                    "{} is a symlink: a connector directory is pinned by its own bytes, and a \
                     symlink's bytes are somewhere else",
                    dir.display()
                ),
            ));
        }
        _ => {}
    }
    // The seeds are read by the same walk that hashes them (M-6): everything under `files/` is
    // kept, and the loop below refuses any file that is not a named seed.
    let mut listing =
        hash_connector_dir_listing(dir, |p| p == "harness.toml" || p.starts_with("files/"))?;
    let harness_toml = listing.captured.remove("harness.toml").ok_or_else(|| {
        refuse(
            "connector_dir",
            format!("{} contains no regular harness.toml", dir.display()),
        )
    })?;
    let text =
        std::str::from_utf8(&harness_toml).map_err(|_| refuse("harness.toml", "is not UTF-8"))?;
    let manifest = HarnessManifest::from_toml_str(text)?;
    let origin = resolve_origin(&manifest.id, &listing.sha256)?;
    if origin == Origin::Dev {
        refuse_dev_for_a_pinned_directory(&manifest.id, &listing.sha256)?;
    }
    manifest.validate(origin)?;

    let seeds: Vec<String> = manifest
        .capture
        .home
        .seed
        .iter()
        .map(|s| format!("files/{}", s.file))
        .collect();
    for (path, _) in &listing.files {
        let allowed = path == "harness.toml" || seeds.iter().any(|s| s == path);
        if !allowed {
            return Err(refuse(
                "connector_dir",
                format!(
                    "`{path}` is not named by `capture.home.seed`: every file under `files/` \
                     must be a seed, and nothing else may sit in a connector directory"
                ),
            ));
        }
    }
    for seed in &seeds {
        match listing.files.iter().find(|(p, _)| p == seed) {
            None => {
                return Err(refuse(
                    "capture.home.seed",
                    format!("`{seed}` is named by the manifest but is not in the directory"),
                ));
            }
            Some((_, len)) if *len > MAX_SEED_BYTES => {
                return Err(refuse(
                    "capture.home.seed",
                    format!("`{seed}` is {len} bytes; a seed is at most {MAX_SEED_BYTES}"),
                ));
            }
            Some(_) => {}
        }
    }
    let seeds = manifest
        .capture
        .home
        .seed
        .iter()
        .map(|s| {
            let bytes = listing
                .captured
                .remove(&format!("files/{}", s.file))
                .expect("every seed was listed, so the walk captured it");
            if let Some(why) = seed_content_problem(&bytes) {
                return Err(refuse("capture.home.seed", format!("`{}` {why}", s.file)));
            }
            Ok(LoadedSeed {
                file: s.file.clone(),
                mode: s.mode(),
                bytes,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(LoadedConnector {
        manifest,
        harness_toml,
        seeds,
        connector_dir_sha256: listing.sha256,
    })
}

/// `Dev` loads an unpinned directory only: a pinned id or a pinned digest means the directory is
/// (or claims to be) a shipped connector, which loads as `Pinned` and never picks up `Dev`'s
/// relaxations (S2 review M-1).
fn refuse_dev_for_a_pinned_directory(id: &str, digest: &str) -> Result<(), ProtocolError> {
    let id_pinned = crate::pins::pinned_connector_dir_sha256(id).is_some();
    let digest_pinned = crate::pins::CONNECTOR_DIR_SHA256S
        .iter()
        .any(|(pinned_id, _)| crate::pins::connector_dir_is_pinned(pinned_id, digest));
    if id_pinned || digest_pinned {
        return Err(refuse(
            "connector_dir",
            format!(
                "`{id}` is pinned by this build{}: `--dev` loads only an unpinned directory, and \
                 a pinned connector loads as itself, checked against its pin (a development \
                 copy needs its own id)",
                if id_pinned {
                    ""
                } else {
                    " (this directory's digest is a pinned one)"
                }
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    #[test]
    fn discovery_propagates_an_iterator_entry_error_instead_of_flattening_it() {
        let root = Path::new("/connector-root");
        let entries = vec![Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "synthetic unreadable directory entry",
        ))];

        let error = discover_connector_dirs_from(root, entries)
            .expect_err("an unreadable child is not an absent child");
        assert!(matches!(error, ProtocolError::Io { path, source }
            if path == root.display().to_string()
                && source.kind() == std::io::ErrorKind::PermissionDenied));
    }
}
