//! Recorded V1 manifests still parse, and still bind to their receipt, although the
//! manifest schema no longer carries the hook-leg sections.
//!
//! Every manifest a V1 run recorded carries `[start_signal]`, `[blocking]`, `[receiver]`,
//! `[[hook_silence_policy]]` and an `arming.recipe`. The schema accepts and ignores exactly
//! those keys, so the verifier can still load the manifest bytes the signed start receipt
//! names, and V13 still compares that digest against the receipt's `manifest_sha256`. An
//! edit to the recorded manifest would break that binding, so the only way these traces stay
//! verifiable is for the loader to take the bytes as they are.
//!
//! Only the load and the binding are asserted here. What the rest of a verdict says about
//! these traces is `end_to_end.rs`'s question (`codex-live`'s full expectation is
//! `#[ignore]`d there).

use goalcup_protocol::manifest::ConnectorManifest;
use goalcup_verifier::VerifyOptions;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The committed live Codex capture, and the two V1 qualification journeys.
fn recorded_traces() -> Vec<PathBuf> {
    vec![
        repo().join("crates/goalcup-verifier/tests/traces/codex-live"),
        repo().join("docs/qualification/2026-09-21-v1-evidence/journeyA-trace"),
        repo().join("docs/qualification/2026-09-21-v1-evidence/journeyB-trace"),
    ]
}

#[test]
fn a_recorded_v1_manifest_parses_and_still_binds_to_its_receipt() {
    for dir in recorded_traces() {
        let bytes = std::fs::read(dir.join("manifest.toml"))
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        let text = String::from_utf8(bytes.clone()).expect("the manifest is UTF-8");

        // The premise: this manifest still carries the hook-leg sections.
        assert!(
            text.contains("[blocking]") && text.contains("[receiver]"),
            "{}: not a pre-cut manifest",
            dir.display()
        );
        let m = ConnectorManifest::from_toml_str(&text)
            .unwrap_or_else(|e| panic!("{}: a recorded manifest must parse: {e}", dir.display()));
        m.validate()
            .unwrap_or_else(|e| panic!("{}: and validate: {e}", dir.display()));

        // The bytes on disk are the bytes the receipt signed.
        let receipt: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("receipt.json")).unwrap()).unwrap();
        let signed = receipt["claims"]["manifest_sha256"]
            .as_str()
            .expect("the receipt claims a manifest digest");
        let on_disk = hex::encode(Sha256::digest(&bytes));
        assert_eq!(on_disk, signed, "{}", dir.display());

        // And the verifier loads the trace and V13 finds the same binding.
        let v = goalcup_verifier::verify_trace_with_options(
            &dir,
            VerifyOptions {
                dev_mode: true,
                ..Default::default()
            },
        )
        .unwrap_or_else(|e| panic!("{}: the trace must load: {e}", dir.display()));
        assert_eq!(v.connector_manifest_sha256, signed, "{}", dir.display());
        let v13 = v.checks.iter().find(|c| c.id == "V13").expect("V13 ran");
        assert!(
            !v13.findings.iter().any(|f| f.code == "descriptor_mismatch"),
            "{}: V13 says the receipt and the manifest disagree: {:?}",
            dir.display(),
            v13.findings
        );
    }
}

#[test]
fn an_unknown_key_in_a_recorded_manifest_is_still_refused() {
    let dir = repo().join("crates/goalcup-verifier/tests/traces/codex-live");
    let text = std::fs::read_to_string(dir.join("manifest.toml")).unwrap();
    let err = ConnectorManifest::from_toml_str(&format!("blocking_mode = \"x\"\n{text}"))
        .expect_err("only the named removed sections are ignored");
    assert!(err.to_string().contains("blocking_mode"), "{err}");
}

/// D12: versions are not pinned, and the recorded V1
/// manifests still carry `=0.153.0`. Their verdict on the version question is unchanged: the
/// harness is the tested 0.153.0, V5 reports `no_version_drift`, and
/// V5 passes — as `verifyA.out` and `verifyB.out` recorded for the two journeys.
#[test]
fn a_recorded_v1_manifests_exact_range_keeps_its_verdict() {
    for dir in recorded_traces() {
        let text = std::fs::read_to_string(dir.join("manifest.toml")).unwrap();
        let m = ConnectorManifest::from_toml_str(&text).unwrap();
        assert_eq!(m.tested_version_range, "=0.153.0", "{}", dir.display());
        let v = goalcup_verifier::verify_trace_with_options(
            &dir,
            VerifyOptions {
                dev_mode: true,
                ..Default::default()
            },
        )
        .unwrap_or_else(|e| panic!("{}: the trace must load: {e}", dir.display()));
        assert_eq!(v.harness_version, "0.153.0", "{}", dir.display());
        let v5 = v.checks.iter().find(|c| c.id == "V5").expect("V5 ran");
        assert_eq!(
            v5.verdict,
            goalcup_verifier::checks::Severity::Pass,
            "{}: {:?}",
            dir.display(),
            v5.findings
        );
        assert!(
            v5.findings.iter().any(|f| f.code == "no_version_drift"),
            "{}: {:?}",
            dir.display(),
            v5.findings
        );
    }
}
