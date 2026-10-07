//! The public conformance suite (DECISIONS D13). Every connector must pass it,
//! first-party ones included.

use goalcup_protocol::manifest::ConnectorManifest;
use goalcup_verifier::checks::all_checks;
use goalcup_verifier::{VerifierError, verify_trace};
use std::collections::BTreeMap;
use std::path::PathBuf;

mod support;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// One shipping surface, found by its manifest's declared identity rather than a directory
/// name. The parser belongs to `family`; the directory remains only where the rulebook is.
#[derive(Debug)]
struct DeclaredConnector {
    id: String,
    family: String,
    dir: PathBuf,
}

/// Discover every declared shipping surface, sorted by declared id.
fn connectors_in(dir: &std::path::Path) -> Result<Vec<DeclaredConnector>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut by_id: BTreeMap<String, DeclaredConnector> = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let manifest_path = entry.path().join("manifest.toml");
        if !manifest_path.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
        let manifest = ConnectorManifest::from_toml_str(&text)
            .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
        manifest
            .validate()
            .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
        let found = DeclaredConnector {
            id: manifest.id.clone(),
            family: manifest.family,
            dir: entry.path(),
        };
        if let Some(first) = by_id.insert(manifest.id.clone(), found) {
            return Err(format!(
                "duplicate connector id {:?}: {} and {}",
                manifest.id,
                first.dir.display(),
                manifest_path.parent().unwrap_or(dir).display()
            ));
        }
    }
    if by_id.is_empty() {
        return Err(format!("no connector manifests in {}", dir.display()));
    }
    Ok(by_id.into_values().collect())
}

fn connectors() -> Vec<DeclaredConnector> {
    let dir = repo_root().join("connectors");
    connectors_in(&dir).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn every_connector_has_a_trace_directory() {
    for connector in connectors() {
        let trace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/traces")
            .join(&connector.id)
            .join("trace.toml");
        assert!(
            trace.is_file(),
            "{} has no conformance trace at {}",
            connector.id,
            trace.display()
        );
    }
}

#[test]
fn every_manifest_validates_and_names_a_parser() {
    for connector in connectors() {
        goalcup_verifier::parsers::parser_for_family(&connector.family)
            .unwrap_or_else(|e| panic!("{}: {e}", connector.id));
    }
}

#[test]
fn every_check_runs_on_every_connector() {
    let expected: Vec<&str> = all_checks().iter().map(|c| c.id()).collect();
    assert_eq!(
        expected,
        vec![
            "V1", "V2", "V3", "V4", "V5", "V6", "V7", "V8", "V9", "V10", "V11", "V12", "V13"
        ],
        "thirteen checks, in order; V12 (anchor reconciliation) and V13 (the trace is bound \
         to a server-signed start) both joined on 2026-09-04"
    );
    for connector in connectors() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/traces")
            .join(&connector.id);
        let trace = support::materialize_trace(&dir);
        let v = verify_trace(trace.path()).unwrap_or_else(|e| panic!("{}: {e}", connector.id));
        let got: Vec<&str> = v.checks.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            got, expected,
            "{}: every check must run, in order",
            connector.id
        );
        assert!(
            v.checks.iter().all(|c| !c.findings.is_empty()),
            "{}: a check with no findings says nothing",
            connector.id
        );
    }
}

#[test]
fn every_connector_recovers_a_goal_and_a_stable_verdict() {
    for connector in connectors() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/traces")
            .join(&connector.id);
        let first = support::materialize_trace(&dir);
        let second = support::materialize_trace(&dir);
        let a = verify_trace(first.path()).unwrap();
        let b = verify_trace(second.path()).unwrap();
        assert_eq!(
            a.to_json().unwrap(),
            b.to_json().unwrap(),
            "{}: verdict is not stable",
            connector.id
        );
        assert!(
            !a.human_messages.is_empty(),
            "{}: no human message recovered",
            connector.id
        );
        assert!(
            a.human_messages[0].text.chars().count() <= goalcup_verifier::checks::MAX_PROMPT_CHARS,
            "{}: the goal exceeds the limit",
            connector.id
        );
        assert_eq!(a.schema_version, "ATIF-v1.8");
        assert!(!a.verifier_version.is_empty());
    }
}

/// A surface has its own connector id but shares the Claude Code parser family. This fresh
/// trace is generated into a temporary directory, then changes only its descriptor and
/// manifest identity; no recorded fixture or signed historical trace is doctored.
#[test]
fn a_fourth_known_family_surface_reaches_the_full_pipeline() {
    let trace = tempfile::tempdir().unwrap();
    support::compliant::generate(trace.path());
    rewrite_identity(trace.path(), "claude-code-synthetic");

    let verdict = verify_trace(trace.path()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(verdict.checks.len(), all_checks().len());
    assert!(
        !verdict.human_messages.is_empty(),
        "the selected family parser reached normal human-rule checks"
    );
    assert_eq!(
        verdict.human_messages[0].text, "write hello.py that prints the current date",
        "the synthetic known-family surface recovers the generated trace's goal bytes"
    );
    assert_eq!(
        verdict.manifest_pin,
        goalcup_verifier::ManifestPin::Unpinned,
        "a new surface id is truthfully unpinned"
    );
    assert!(
        verdict
            .checks
            .iter()
            .find(|check| check.id == "V13")
            .unwrap()
            .findings
            .iter()
            .any(|finding| finding.code == "descriptor_mismatch"),
        "the unchanged receipt remains an explicit binding mismatch"
    );
}

#[test]
fn discovery_rejects_an_empty_or_duplicate_declared_shipping_set() {
    let empty = tempfile::tempdir().unwrap();
    let error = connectors_in(empty.path()).unwrap_err();
    assert!(error.contains("no connector manifests"), "{error}");

    let duplicate = tempfile::tempdir().unwrap();
    for name in ["first", "second"] {
        let dir = duplicate.path().join(name);
        std::fs::create_dir(&dir).unwrap();
        std::fs::copy(
            repo_root().join("connectors/claude-code-cli/manifest.toml"),
            dir.join("manifest.toml"),
        )
        .unwrap();
    }
    let error = connectors_in(duplicate.path()).unwrap_err();
    assert!(error.contains("duplicate connector id"), "{error}");
    assert!(error.contains("claude-code-cli"), "{error}");
}

#[test]
fn unknown_family_and_descriptor_identity_mismatch_are_distinct_refusals() {
    let unknown = tempfile::tempdir().unwrap();
    support::compliant::generate(unknown.path());
    rewrite_manifest_line(
        unknown.path(),
        "family = \"claude-code\"",
        "family = \"unknown-family\"",
    );
    match verify_trace(unknown.path()) {
        Err(VerifierError::UnknownParserFamily(family)) => assert_eq!(family, "unknown-family"),
        Ok(_) => panic!("unknown family must refuse before a verdict"),
        Err(other) => panic!("unknown family was not named distinctly: {other}"),
    }

    let mismatched = tempfile::tempdir().unwrap();
    support::compliant::generate(mismatched.path());
    rewrite_descriptor_connector(mismatched.path(), "different-surface");
    assert!(
        matches!(
            verify_trace(mismatched.path()),
            Err(VerifierError::ConnectorMismatch { .. })
        ),
        "descriptor identity must refuse before family dispatch"
    );
}

fn rewrite_identity(trace: &std::path::Path, id: &str) {
    rewrite_manifest_line(trace, "id = \"claude-code-cli\"", &format!("id = \"{id}\""));
    rewrite_manifest_line(
        trace,
        "harness = \"claude-code-cli\"\npath_glob = \"{config_dir}/projects/*/*.jsonl\"",
        &format!("harness = \"{id}\"\npath_glob = \"{{config_dir}}/projects/*/*.jsonl\""),
    );

    rewrite_descriptor_connector(trace, id);
}

fn rewrite_manifest_line(trace: &std::path::Path, from: &str, to: &str) {
    let manifest = trace.join("manifest.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    assert!(text.contains(from), "manifest did not contain {from:?}");
    std::fs::write(manifest, text.replacen(from, to, 1)).unwrap();
}

fn rewrite_descriptor_connector(trace: &std::path::Path, id: &str) {
    let descriptor = trace.join("trace.toml");
    let descriptor_text = std::fs::read_to_string(&descriptor).unwrap();
    std::fs::write(
        descriptor,
        descriptor_text.replacen(
            "connector = \"claude-code-cli\"",
            &format!("connector = \"{id}\""),
            1,
        ),
    )
    .unwrap();
}

#[test]
fn partitioning_is_asserted_on_fixtures_that_contain_foreign_rows() {
    // DECISIONS D20: "the conformance suite asserts partition behaviour on fixtures
    // containing foreign rows."
    //
    // codex-cli's expectation: V10 sweeps every record of a `Direct`-partition store, not
    // only its session facts. Codex's `history.jsonl` is one shared, append-only log across
    // every thread on the machine, and the M0 fixture's own bytes measure three records (seq
    // 6-8) stamped with the other thread's session id landing inside the tracked thread's
    // `history` store. D20 requires the daemon to catch such a capture-time partition gap
    // before hashing — so the correct conformance assertion is that partitioning did catch
    // it, not that it stayed silent. claude-code-cli carries no such in-store mixing, so it
    // asserts no `partition_violation`.
    {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/traces")
            .join("claude-code-cli");
        let trace = support::materialize_trace(&dir);
        let v = verify_trace(trace.path()).unwrap();
        let v10 = v.checks.iter().find(|c| c.id == "V10").unwrap();
        assert!(
            v10.findings
                .iter()
                .any(|f| f.code == "foreign_records_present"),
            "claude-code-cli: the foreign store must be reported"
        );
        assert!(
            !v10.findings.iter().any(|f| f.code == "partition_violation"),
            "claude-code-cli: no tracked record may fail its partition key"
        );
    }
    {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/traces")
            .join("codex-cli");
        let trace = support::materialize_trace(&dir);
        let v = verify_trace(trace.path()).unwrap();
        let v10 = v.checks.iter().find(|c| c.id == "V10").unwrap();
        assert!(
            v10.findings
                .iter()
                .any(|f| f.code == "foreign_records_present"),
            "codex-cli: the foreign store must be reported"
        );
        let violations: Vec<_> = v10
            .findings
            .iter()
            .filter(|f| f.code == "partition_violation")
            .collect();
        assert_eq!(
            violations.len(),
            3,
            "codex-cli: history seq 6-8 carry the foreign session id and must each fail \
             partitioning (task-11 review, ruling 6): {:?}",
            v10.findings
        );
        for f in &violations {
            assert!(f.message.contains("history"), "{}", f.message);
            assert!(f.message.contains("01a06616"), "{}", f.message);
        }
    }
}

/// Every retained V1 manifest participates; a schema-only harness does not invent a V1 surface.
#[test]
fn every_retained_v1_manifest_is_in_the_v1_conformance_suite() {
    let ids: Vec<String> = connectors().into_iter().map(|c| c.id).collect();
    assert_eq!(ids, ["claude-code-cli", "codex-cli"]);
    assert!(
        !repo_root()
            .join("connectors/hermes-cli/manifest.toml")
            .exists(),
        "the schema-only connector must not regain a V1 rulebook"
    );
}
