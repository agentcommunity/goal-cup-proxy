//! The harness-contract kit's invented credential collides with nothing the kit ships.
//!
//! A keyed kit case hands [`support::KIT_CREDENTIAL`] to its stand-in and then requires the
//! launcher's key scan of the sealed trace and run directory to come back clean. The daemon
//! writes the case id into the chain, and the run stages the case's manifest and the shipped
//! connectors, so a key-scan form of the credential in any of them would fail the keyed case
//! for a reason that has nothing to do with the credential leaking. The kit cases themselves
//! run on macOS and Linux; this check runs everywhere, so a colliding credential or case name is
//! caught on other platforms too.

use crate::common::support;

use goalcup_cli::key_scan::FragmentMatcher;
use std::path::{Path, PathBuf};

fn matcher() -> FragmentMatcher {
    FragmentMatcher::new(support::KIT_CREDENTIAL.as_bytes()).expect("a usable kit credential")
}

/// Every file and directory under `root`, recursively, sorted.
fn entries(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)
            .unwrap_or_else(|error| panic!("cannot enumerate {}: {error}", dir.display()))
        {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path.clone());
            }
            out.push(path);
        }
    }
    out.sort();
    assert!(!out.is_empty(), "{} holds nothing", root.display());
    out
}

/// The paths under `root` whose name or contents hold a key-scan form of the kit credential.
/// Names a path and never the form it matched.
fn collisions(matcher: &FragmentMatcher, root: &Path) -> Vec<String> {
    entries(root)
        .into_iter()
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            matcher.contains(name.as_bytes())
                || (path.is_file() && matcher.contains(&std::fs::read(path).unwrap()))
        })
        .map(|path| path.strip_prefix(root).unwrap().display().to_string())
        .collect()
}

#[test]
fn the_kit_credential_is_a_usable_key_of_the_shared_grammar() {
    let key = support::KIT_CREDENTIAL;
    assert!(
        (20..=256).contains(&key.len()),
        "the kit credential is outside 20 to 256 bytes"
    );
    assert!(
        key.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "the kit credential is outside [A-Za-z0-9_-]"
    );
    matcher();
}

#[test]
fn no_kit_fixture_case_id_or_case_directory_holds_a_scan_form_of_the_kit_credential() {
    let matcher = matcher();
    let hits = collisions(&matcher, &support::fixture("harness-kit"));
    assert!(
        hits.is_empty(),
        "these kit paths hold a key-scan form of the kit credential in their name or \
         contents, so a keyed kit case would fail its own key scan: {hits:?}"
    );
}

#[test]
fn no_shipped_connector_file_holds_a_scan_form_of_the_kit_credential() {
    let matcher = matcher();
    let hits = collisions(&matcher, &support::connectors_dir());
    assert!(
        hits.is_empty(),
        "these connector paths hold a key-scan form of the kit credential: {hits:?}"
    );
}

#[test]
fn the_marker_digest_is_not_a_scan_form_of_the_kit_credential() {
    let marker = goalcup_daemon::workspace::sha256_hex(
        format!(
            "{}{}",
            support::KIT_CREDENTIAL_MARKER_PREFIX,
            support::KIT_CREDENTIAL
        )
        .as_bytes(),
    );
    assert!(
        !matcher().contains(marker.as_bytes()),
        "the kit marker would itself trip the key scan"
    );
}
