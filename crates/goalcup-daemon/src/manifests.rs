//! The daemon's snapshot of the installed connector manifests, loaded from exactly the bytes in
//! the accepted connector-directory hash stream.
//!
//! A local capture daemon needs this map to look up a connector's profile; the V1 path also
//! compiles each manifest's `foreign_scan` declarations from the same bytes
//! (`foreign::compile_declarations`, `v1-compat` only). A default daemon therefore no longer
//! validates `foreign_scan` declarations at load (D42); no shipped capture connector needs them.

use crate::DaemonError;
use goalcup_protocol::manifest::{ConnectorManifest, connector_dir_sha256_with_manifest};
use std::collections::BTreeMap;
use std::path::Path;

/// One daemon-owned snapshot of the manifest map, and — with `v1-compat` — the scan
/// declarations compiled from the same pinned byte streams.
#[derive(Debug, Clone)]
pub struct TrustedManifests {
    manifests: std::sync::Arc<BTreeMap<String, ConnectorManifest>>,
    #[cfg(feature = "v1-compat")]
    pub(crate) declarations: std::sync::Arc<Vec<crate::foreign::TrustedForeignScan>>,
}

/// One installed connector as loaded. The directory name and the two digests are read by the V1
/// foreign-scan compiler alone, so only a `v1-compat` build keeps them.
#[derive(Debug)]
pub(crate) struct LoadedManifest {
    #[cfg(feature = "v1-compat")]
    pub(crate) directory_name: String,
    pub(crate) manifest: ConnectorManifest,
    #[cfg(feature = "v1-compat")]
    pub(crate) directory_sha256: String,
    #[cfg(feature = "v1-compat")]
    pub(crate) manifest_sha256: String,
}

impl TrustedManifests {
    /// Load every installed declaration from exactly the bytes in the accepted
    /// connector-directory hash stream.
    pub fn load(dir: &Path) -> Result<Self, DaemonError> {
        let entries = std::fs::read_dir(dir).map_err(|source| DaemonError::Io {
            path: dir.display().to_string(),
            source,
        })?;
        let mut loaded = BTreeMap::new();
        for entry in entries {
            let entry = entry.map_err(|source| DaemonError::Io {
                path: dir.display().to_string(),
                source,
            })?;
            let kind = entry.file_type().map_err(|source| DaemonError::Io {
                path: entry.path().display().to_string(),
                source,
            })?;
            if !kind.is_dir() || !entry.path().join("manifest.toml").is_file() {
                continue;
            }
            // Checked in both builds: a name that is not UTF-8 is not a connector this daemon
            // can name.
            #[cfg_attr(not(feature = "v1-compat"), allow(unused_variables))]
            let directory_name = entry.file_name().into_string().map_err(|_| {
                DaemonError::Config("a connector directory name is not UTF-8".into())
            })?;
            #[cfg_attr(not(feature = "v1-compat"), allow(unused_variables))]
            let (directory_sha256, manifest_bytes) =
                connector_dir_sha256_with_manifest(&entry.path())?;
            let manifest_text = std::str::from_utf8(&manifest_bytes).map_err(|_| {
                DaemonError::Config(format!(
                    "{}/manifest.toml is not UTF-8",
                    entry.path().display()
                ))
            })?;
            let manifest = ConnectorManifest::from_toml_str(manifest_text).map_err(|error| {
                DaemonError::Config(format!("{}/manifest.toml: {error}", entry.path().display()))
            })?;
            let id = manifest.id.clone();
            let row = LoadedManifest {
                #[cfg(feature = "v1-compat")]
                directory_name,
                manifest,
                #[cfg(feature = "v1-compat")]
                directory_sha256,
                #[cfg(feature = "v1-compat")]
                manifest_sha256: crate::workspace::sha256_hex(&manifest_bytes),
            };
            if loaded.insert(id.clone(), row).is_some() {
                return Err(DaemonError::Config(format!(
                    "more than one installed connector declares id `{id}`"
                )));
            }
        }

        #[cfg(feature = "v1-compat")]
        let declarations = crate::foreign::compile_declarations(&loaded)?;
        let manifests = loaded
            .into_iter()
            .map(|(id, row)| (id, row.manifest))
            .collect();
        Ok(Self {
            manifests: std::sync::Arc::new(manifests),
            #[cfg(feature = "v1-compat")]
            declarations: std::sync::Arc::new(declarations),
        })
    }

    pub fn manifests(&self) -> &BTreeMap<String, ConnectorManifest> {
        &self.manifests
    }

    pub fn manifests_arc(&self) -> std::sync::Arc<BTreeMap<String, ConnectorManifest>> {
        self.manifests.clone()
    }
}
