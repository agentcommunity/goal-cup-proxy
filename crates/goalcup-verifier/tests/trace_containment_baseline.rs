//! Regression: participant evidence must not escape its descriptor root.
//!
//! This stays baseline-compatible so it can demonstrate the original resolver's unsafe
//! behavior without changing any committed fixture. The descriptor points its manifest at
//! a valid file outside the trace, and deletes the contained copy. A verifier that returns
//! a verdict therefore read that outside byte; the containment contract requires refusal.

mod support;

use goalcup_verifier::verify_trace;

#[test]
fn descriptor_cannot_read_a_valid_manifest_outside_its_root() {
    let trace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    support::compliant::generate(trace.path());

    let outside_manifest = outside.path().join("manifest.toml");
    std::fs::copy(trace.path().join("manifest.toml"), &outside_manifest).unwrap();
    std::fs::remove_file(trace.path().join("manifest.toml")).unwrap();

    let descriptor = trace.path().join("trace.toml");
    let text = std::fs::read_to_string(&descriptor).unwrap();
    let needle = "connector_manifest = \"manifest.toml\"";
    assert_eq!(
        text.matches(needle).count(),
        1,
        "fixture descriptor shape changed"
    );
    let replacement = format!(
        "connector_manifest = {}",
        toml::Value::String(outside_manifest.to_string_lossy().into_owned()),
    );
    let text = text.replacen(needle, &replacement, 1);
    std::fs::write(&descriptor, text).unwrap();

    assert!(
        verify_trace(trace.path()).is_err(),
        "a verdict here proves the verifier read the valid manifest outside the trace root"
    );
}
