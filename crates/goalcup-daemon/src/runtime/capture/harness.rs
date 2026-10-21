//! Schema-1 harness loading for default capture.

use crate::DaemonError;
use crate::config::DaemonConfig;
use goalcup_protocol::ProtocolError;
use goalcup_protocol::harness::{
    LoadedConnector, Origin, discover_connector_dirs, load_discovered_dir,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfileDigests {
    pub(crate) profile_sha256: String,
    pub(crate) connector_dir_sha256: String,
}

/// Load every child through the schema-1 loader, select by its declared id, collect every
/// loaded real-home refusal, and pin-check before the caller creates any capture state.
pub(crate) fn capture_harness(
    config: &DaemonConfig,
    profile_id: &str,
) -> Result<(LoadedConnector, ProfileDigests, Vec<String>), DaemonError> {
    let mut loaded = BTreeMap::<String, LoadedConnector>::new();
    let children = discover_connector_dirs(&config.connectors_dir)
        .map_err(|error| DaemonError::Config(format!("connector harness discovery: {error}")))?;
    for path in children {
        let connector = load_child(&path)?;
        let declared = connector.manifest.id.clone();
        if loaded.insert(declared.clone(), connector).is_some() {
            return Err(DaemonError::Refused(format!(
                "two harness directories declare connector id {declared}"
            )));
        }
    }
    let every_loaded_real_dir = loaded
        .values()
        .flat_map(|connector| connector.manifest.capture.home.real_dirs.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let connector = loaded.remove(profile_id).ok_or_else(|| {
        DaemonError::Missing(format!("no connector harness declares id {profile_id}"))
    })?;
    let digests = ProfileDigests {
        profile_sha256: connector.profile_sha256(),
        connector_dir_sha256: connector.connector_dir_sha256.to_ascii_lowercase(),
    };
    Ok((connector, digests, every_loaded_real_dir))
}

/// Load one connector folder. Every error names the folder (a connector id, not a secret),
/// because one bad sibling refuses every start and an operator must see which one.
fn load_child(path: &Path) -> Result<LoadedConnector, DaemonError> {
    let name = path
        .file_name()
        .map_or_else(|| "?".into(), |name| name.to_string_lossy().into_owned());
    load_discovered_dir(path, Origin::Dev).map_err(|error| match error {
        ProtocolError::Field { ref field, .. } if field == "connector_folder" => {
            DaemonError::Refused(format!("connector folder {name}: {error}"))
        }
        ProtocolError::Field { ref field, .. } if field != "connector_dir" => {
            DaemonError::Config(format!("connector folder {name}: {error}"))
        }
        _ => DaemonError::Config(format!(
            "connector folder {name}: a connector harness directory is invalid; no bytes \
             outside it were read"
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::shipped_connector;
    use super::*;

    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    fn test_config(temp: &tempfile::TempDir, connectors: &Path) -> DaemonConfig {
        DaemonConfig::from_toml_str(&format!(
            "artifact_root = {artifacts:?}\n\
             connectors_dir = {connectors:?}\n\
             config_dir = {config:?}\n\
             workspace_parent = {workspaces:?}\n\
             control_addr = \"127.0.0.1:0\"\n",
            artifacts = temp.path().join("artifacts"),
            config = temp.path().join("config"),
            workspaces = temp.path().join("workspaces"),
        ))
        .unwrap()
    }

    #[test]
    fn daemon_capture_scan_rejects_byte_identical_shipped_bytes_under_an_alias() {
        let temp = tempfile::tempdir().unwrap();
        let connectors = temp.path().join("connectors");
        std::fs::create_dir(&connectors).unwrap();
        let (shipped, _, manifest) = shipped_connector();
        copy_tree(&shipped, &connectors.join("alias"));
        let config = test_config(&temp, &connectors);

        let error = capture_harness(&config, &manifest.id)
            .expect_err("the daemon consumes the shared folder/id/digest admission policy");
        assert!(error.to_string().contains(&manifest.id), "{error}");
        assert!(error.to_string().contains("folder"), "{error}");
    }
}
