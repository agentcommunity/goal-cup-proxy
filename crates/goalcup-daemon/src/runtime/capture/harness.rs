//! Schema-1 harness loading for default capture.

use crate::DaemonError;
use crate::config::DaemonConfig;
use goalcup_protocol::ProtocolError;
use goalcup_protocol::connector_dir::connector_dir_sha256;
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

/// Locate the exact directory bytes a row recorded without making an unrelated sibling's
/// current validity a prerequisite for recovery. The digest walk is safe and refuses links;
/// only the matching directory is parsed, and the loaded digest and declared id are rechecked.
pub(super) fn recorded_capture_harness(
    config: &DaemonConfig,
    connector_id: &str,
    recorded_digest: &str,
) -> Option<LoadedConnector> {
    let children = std::fs::read_dir(&config.connectors_dir).ok()?;
    recorded_capture_harness_from_paths(
        connector_id,
        recorded_digest,
        children.map(|child| child.map(|child| child.path())),
    )
}

/// Restart recovery is deliberately best-effort over the current directory listing: a row's
/// recorded connector may still be recoverable when an unrelated sibling cannot be enumerated.
/// The relaxation stops at enumeration. A candidate is accepted only after its safe directory
/// digest equals the recorded digest, the protocol loader admits its folder/id/pin identity, and
/// the loaded digest and declared id both repeat the row's provenance.
fn recorded_capture_harness_from_paths(
    connector_id: &str,
    recorded_digest: &str,
    paths: impl IntoIterator<Item = std::io::Result<std::path::PathBuf>>,
) -> Option<LoadedConnector> {
    for path in paths.into_iter().flatten() {
        if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            continue;
        }
        let Ok(digest) = connector_dir_sha256(&path) else {
            continue;
        };
        if !digest.eq_ignore_ascii_case(recorded_digest) {
            continue;
        }
        let Ok(connector) = load_child(&path) else {
            continue;
        };
        if connector.manifest.id == connector_id
            && connector
                .connector_dir_sha256
                .eq_ignore_ascii_case(recorded_digest)
        {
            return Some(connector);
        }
    }
    None
}
fn load_child(path: &Path) -> Result<LoadedConnector, DaemonError> {
    load_discovered_dir(path, Origin::Dev).map_err(|error| match error {
        ProtocolError::Field { ref field, .. } if field == "connector_folder" => {
            DaemonError::Refused(format!("connector harness: {error}"))
        }
        ProtocolError::Field { ref field, .. } if field != "connector_dir" => {
            DaemonError::Config(format!("connector harness: {error}"))
        }
        _ => DaemonError::Config(
            "a connector harness directory is invalid; no bytes outside it were read".into(),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{replaced_once, shipped_connector};
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

    #[test]
    fn restart_lookup_is_best_effort_over_entries_but_still_binds_digest_and_id() {
        let temp = tempfile::tempdir().unwrap();
        let connectors = temp.path().join("connectors");
        std::fs::create_dir(&connectors).unwrap();
        let (_, shipped, manifest) = shipped_connector();
        let source = replaced_once(
            &shipped,
            &format!("id = {:?}", manifest.id),
            "id = \"restart-dev\"",
        );
        let matching = connectors.join("matching");
        std::fs::create_dir(&matching).unwrap();
        std::fs::write(matching.join("harness.toml"), &source).unwrap();
        let digest = connector_dir_sha256(&matching).unwrap();
        let wrong = connectors.join("wrong");
        std::fs::create_dir(&wrong).unwrap();
        std::fs::write(
            wrong.join("harness.toml"),
            format!("{source}\n# different bytes\n"),
        )
        .unwrap();
        let entries = vec![
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "synthetic unreadable directory entry",
            )),
            Ok(wrong),
            Ok(matching),
        ];

        let loaded = recorded_capture_harness_from_paths("restart-dev", &digest, entries)
            .expect("restart lookup skips an unreadable entry and finds exact recorded provenance");
        assert_eq!(loaded.manifest.id, "restart-dev");
        assert_eq!(loaded.connector_dir_sha256, digest);
    }
}
