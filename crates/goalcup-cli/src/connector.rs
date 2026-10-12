//! The shipped connectors: finding one by the id its `harness.toml` declares.
//!
//! A connector is a directory under [`crate::config::connectors_dir`] holding a `harness.toml`
//! (schema 1) and the seed files its `capture.home.seed` names under `files/`. The protocol's
//! loader is the only parser ([`goalcup_protocol::harness::load_discovered_dir`]); nothing here
//! reads a connector's bytes a second time. The V1 `manifest.toml` and its reader were removed
//! by OS-29.
//!
//! # Discovery is by the id the manifest declares
//!
//! Not by joining `id` onto the directory path, which is a traversal waiting for a caller
//! that passes `../..`, and not by the directory's name, which need not be the id. The
//! daemon's create path reads them the same way and for the same reason, so the CLI and the
//! daemon cannot disagree about which directory a connector id names.
//!
//! # An id belongs to one directory
//!
//! Because the id is declared and not the directory's name, two directories can declare the
//! same one. [`load_harness`] does not pick between them: the first in path order would shadow
//! the other silently, and the shadowed one is exactly where an unpinned rulebook would hide.
//! It refuses instead, naming both paths.

use crate::CliError;
use goalcup_protocol::ProtocolError;
use goalcup_protocol::harness::{LoadedConnector, Origin};
use std::path::{Path, PathBuf};

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
         one that lost is where an unpinned connector would sit unnoticed.",
        first.display(),
        second.display()
    ))
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
}
