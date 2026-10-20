//! Setup choices and their meaning. Presentation is in `ui`; setup still runs through prep.

use crate::{
    CliError,
    connector::{HarnessConnector, harness_dirs},
};
use goalcup_protocol::harness::{Origin, load_discovered_dir};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub const ADD_GUIDE: &str = "https://github.com/agentcommunity/goal-cup-tracer/blob/main/docs/HARNESS-ONBOARDING.md#quick-path";

pub struct HarnessChoice {
    pub id: String,
    pub label: String,
    pub installed: bool,
}

/// Discover chat connectors using the normal pin and schema checks. UI has no roster of its
/// own, and a duplicate ID must never be silently chosen from an ambiguous installation.
pub fn harness_choices(
    connectors: &Path,
    env: &BTreeMap<String, String>,
    dev: bool,
) -> Result<Vec<HarnessChoice>, CliError> {
    let mut choices = Vec::new();
    let mut ids = BTreeSet::new();
    for dir in harness_dirs(connectors)? {
        let loaded = load_discovered_dir(&dir, if dev { Origin::Dev } else { Origin::Pinned })?;
        if !ids.insert(loaded.manifest.id.clone()) {
            return Err(CliError::Refused(
                "two connectors declare the same harness id; run goalcup doctor".into(),
            ));
        }
        if !loaded.manifest.capture.interactive {
            continue;
        }
        let connector = HarnessConnector { dir, loaded };
        choices.push(HarnessChoice {
            id: connector.loaded.manifest.id.clone(),
            label: connector.display_name().to_owned(),
            installed: crate::launch::capture_executable(
                &connector,
                &connector.loaded.manifest.capture,
                env,
            )
            .is_ok(),
        });
    }
    choices.sort_by(|a, b| a.label.cmp(&b.label));
    Ok(choices)
}
