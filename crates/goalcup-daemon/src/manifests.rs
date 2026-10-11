//! The daemon's snapshot of the installed connector manifests, loaded from exactly the bytes in
//! the accepted connector-directory hash stream.
//!
//! A local capture daemon needs this map to look up a connector's profile. It does not validate
//! `foreign_scan` declarations at load: the V1 compiler that read them was removed (D48), and no
//! shipped capture connector needs them.

use crate::DaemonError;
use goalcup_protocol::manifest::{ConnectorManifest, connector_dir_sha256_with_manifest};
use std::collections::BTreeMap;
use std::path::Path;

/// One daemon-owned snapshot of the manifest map.
#[derive(Debug, Clone)]
pub struct TrustedManifests {
    manifests: std::sync::Arc<BTreeMap<String, ConnectorManifest>>,
}

/// One installed connector as loaded.
#[derive(Debug)]
pub(crate) struct LoadedManifest {
    pub(crate) manifest: ConnectorManifest,
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
            // A name that is not UTF-8 is not a connector this daemon can name.
            entry.file_name().into_string().map_err(|_| {
                DaemonError::Config("a connector directory name is not UTF-8".into())
            })?;
            let (_directory_sha256, manifest_bytes) =
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
            let row = LoadedManifest { manifest };
            if loaded.insert(id.clone(), row).is_some() {
                return Err(DaemonError::Config(format!(
                    "more than one installed connector declares id `{id}`"
                )));
            }
        }

        let manifests = loaded
            .into_iter()
            .map(|(id, row)| (id, row.manifest))
            .collect();
        Ok(Self {
            manifests: std::sync::Arc::new(manifests),
        })
    }

    pub fn manifests(&self) -> &BTreeMap<String, ConnectorManifest> {
        &self.manifests
    }

    pub fn manifests_arc(&self) -> std::sync::Arc<BTreeMap<String, ConnectorManifest>> {
        self.manifests.clone()
    }
}
