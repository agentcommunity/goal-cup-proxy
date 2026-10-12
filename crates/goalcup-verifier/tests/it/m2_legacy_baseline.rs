//! The frozen historical semantic baseline.
//!
//! Every committed pre-M2 trace — the v1 fixtures, the dogfood v1 Codex capture, the M0 spike
//! traces and the 2026-09-21 v4 qualification journeys named by
//! `docs/qualification/2026-09-21-v1-evidence.md` — is run through the real verifier pipeline
//! and compared with the verdict frozen for it under `tests/fixtures/m2-legacy-baseline/`.
//!
//! **Trust is fixture-owned.** The comparison runs against `trust-fixture.json`'s pin tables
//! (the generated tables of tag `v0.1.0`), injected through `Pipeline::load_with_pin_tables`,
//! which only this crate's `trust-fixture` dev feature compiles. It never reads this build's
//! generated pins, so `scripts/pin-manifests.sh` can regenerate the shipped tables without
//! rewriting what a historical trace was judged to be, and no historical manifest has to hash
//! to a newly shipped profile. The build-relative reading is tested separately and labelled so.
//!
//! **Nothing here regenerates a golden.** The expected documents were frozen once at `19a52ea`
//! (`m2/wire`, before any M2 capture type existed). A changed historical judgement is a
//! failure to explain, never a file to refresh. The only field left out of the comparison is
//! `verifier_version`: it is the crate version of whichever build ran, not a judgement.
//!
//! Inputs are materialised into a private temporary root before verification: every file is
//! copied byte for byte, then made owner-only (files 0600, directories 0700) the way the
//! daemon exports a trace, because the V4 retained-submission reader refuses group- or
//! world-readable files and Git does not preserve private modes. A descriptor that names a
//! manifest outside its own directory (the live `connectors/<id>/manifest.toml`) is
//! materialised with the frozen copy of that manifest instead, so a later connector edit is not
//! a change to historical input. Every materialised byte is identified by SHA-256 in the
//! expected document and checked before any verdict is compared.

use crate::common as support;

use goalcup_verifier::{
    ManifestPinning, PinTable, PinTables, Pipeline, SERVER_PUBLIC_KEYS, VerifyOptions,
};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const BASELINE: &str = "tests/fixtures/m2-legacy-baseline";

/// Left out of every comparison: the build's own crate version, not a judgement.
const EXCLUDED_VERDICT_FIELDS: &[&str] = &["verifier_version"];

/// The checks whose inputs include no pin table. V13 renders both pin readings.
const TRUST_FREE_CHECKS: usize = 12;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustFixture {
    id: String,
    #[allow(dead_code)]
    description: String,
    source_commit: String,
    manifest_pins: Vec<(String, String)>,
    directory_pins: Vec<(String, String)>,
    key_policy: KeyPolicy,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyPolicy {
    #[allow(dead_code)]
    description: String,
    server_public_keys: Vec<String>,
    allow_descriptor_key: bool,
    extra_server_keys: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseDefinition {
    case: String,
    #[allow(dead_code)]
    description: String,
    source: String,
    materialize: Materialize,
    frozen_manifest: Option<String>,
    options: CaseOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Materialize {
    /// Copy the whole trace directory, including the retained V4 submission layout.
    Directory,
    /// Copy exactly the descriptor-declared evidence (`support::materialize_trace`).
    Descriptor,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseOptions {
    dev_mode: bool,
    allow_descriptor_key: bool,
    extra_server_keys: Vec<String>,
}

impl CaseOptions {
    fn verify_options(&self) -> VerifyOptions {
        VerifyOptions {
            allow_descriptor_key: self.allow_descriptor_key,
            extra_server_keys: self.extra_server_keys.clone(),
            dev_mode: self.dev_mode,
        }
    }
}

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    crate_dir().join("../..")
}

fn baseline_dir() -> PathBuf {
    crate_dir().join(BASELINE)
}

fn trust_fixture() -> &'static TrustFixture {
    static FIXTURE: OnceLock<TrustFixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let path = baseline_dir().join("trust-fixture.json");
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    })
}

/// A pin table must be `'static` because a resolved `ManifestPinning` is `Copy`; these few
/// fixture rows are leaked once per test process.
fn leak_table(rows: &[(String, String)]) -> PinTable {
    let rows: Vec<(&'static str, &'static str)> = rows
        .iter()
        .map(|(id, sha)| {
            let id: &'static str = Box::leak(id.clone().into_boxed_str());
            let sha: &'static str = Box::leak(sha.clone().into_boxed_str());
            (id, sha)
        })
        .collect();
    Box::leak(rows.into_boxed_slice())
}

/// The fixture-owned tables every historical comparison is judged against.
fn fixture_tables() -> PinTables {
    static TABLES: OnceLock<PinTables> = OnceLock::new();
    *TABLES.get_or_init(|| {
        let fixture = trust_fixture();
        PinTables::fixture(
            leak_table(&fixture.manifest_pins),
            leak_table(&fixture.directory_pins),
        )
    })
}

fn cases() -> Vec<CaseDefinition> {
    let path = baseline_dir().join("cases.json");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn case(name: &str) -> CaseDefinition {
    cases()
        .into_iter()
        .find(|c| c.case == name)
        .unwrap_or_else(|| panic!("no baseline case {name}"))
}

fn expected_path(name: &str) -> PathBuf {
    baseline_dir().join("expected").join(format!("{name}.json"))
}

fn expected(name: &str) -> Value {
    let path = expected_path(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn materialize(def: &CaseDefinition) -> tempfile::TempDir {
    let source = repo_root().join(&def.source);
    let dir = match def.materialize {
        Materialize::Directory => {
            let dst = tempfile::tempdir().expect("baseline tempdir");
            support::copy_tree(&source, dst.path());
            dst
        }
        Materialize::Descriptor => support::materialize_trace(&source),
    };
    if let Some(frozen) = &def.frozen_manifest {
        let target = dir.path().join("manifest.toml");
        assert!(
            target.is_file(),
            "{}: descriptor materialisation must place the named manifest at manifest.toml",
            def.case
        );
        let bytes = std::fs::read(baseline_dir().join(frozen)).unwrap();
        std::fs::write(&target, bytes).unwrap();
    }
    support::make_private(dir.path());
    dir
}

/// SHA-256 of every materialised file, by `/`-separated path relative to the trace root.
fn input_identity(root: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.insert(
                    relative,
                    hex::encode(Sha256::digest(std::fs::read(&path).unwrap())),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// The protocol versions the trace's own chain declares, read from the artifact rather than
/// from anything this test believes about it. Empty for an M0 trace, which has no chain.
fn chain_protocol_versions(root: &Path) -> Vec<u64> {
    let descriptor: toml::Value =
        toml::from_str(&std::fs::read_to_string(root.join("trace.toml")).unwrap()).unwrap();
    let Some(chain) = descriptor.get("chain").and_then(toml::Value::as_str) else {
        return Vec::new();
    };
    let text = std::fs::read_to_string(root.join(chain)).unwrap();
    let versions: BTreeSet<u64> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<Value>(line)
                .ok()
                .and_then(|v| v.get("protocol_version").and_then(Value::as_u64))
                .expect("every committed chain line declares its protocol version")
        })
        .collect();
    versions.into_iter().collect()
}

fn verdict_value(verdict: &goalcup_verifier::Verdict) -> Value {
    let mut value = serde_json::to_value(verdict).expect("a verdict serialises");
    let object = value.as_object_mut().expect("a verdict is an object");
    for field in EXCLUDED_VERDICT_FIELDS {
        assert!(
            object.remove(*field).is_some(),
            "excluded field {field} exists"
        );
    }
    value
}

/// The real pipeline, judged against `tables`.
fn verdict_under(tables: PinTables, root: &Path, options: &CaseOptions) -> Value {
    let pipeline = Pipeline::load_with_pin_tables(root, options.verify_options(), tables)
        .unwrap_or_else(|e| panic!("{}: {e}", root.display()));
    verdict_value(
        &pipeline
            .verdict()
            .expect("a loaded pipeline yields a verdict"),
    )
}

/// Everything the frozen document records about one case, observed now.
fn observe(def: &CaseDefinition, root: &Path) -> Value {
    serde_json::json!({
        "case": def.case,
        "trust_fixture": trust_fixture().id,
        "protocol_versions": chain_protocol_versions(root),
        "inputs": input_identity(root),
        "excluded_verdict_fields": EXCLUDED_VERDICT_FIELDS,
        "verdict": verdict_under(fixture_tables(), root, &def.options),
    })
}

/// The first JSON pointer at which `observed` and `expected` differ, if any.
fn first_difference(observed: &Value, expected: &Value, at: &str) -> Option<String> {
    match (observed, expected) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            keys.into_iter()
                .find_map(|key| match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => first_difference(x, y, &format!("{at}/{key}")),
                    (x, y) => Some(format!(
                        "{at}/{key}: observed {}, frozen {}",
                        x.map_or("absent".to_string(), |v| v.to_string()),
                        y.map_or("absent".to_string(), |v| v.to_string())
                    )),
                })
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => a
            .iter()
            .zip(b)
            .enumerate()
            .find_map(|(i, (x, y))| first_difference(x, y, &format!("{at}/{i}"))),
        (a, b) if a == b => None,
        (a, b) => Some(format!("{at}: observed {a}, frozen {b}")),
    }
}

/// The baseline's acceptance rule: identical, or a named first difference.
fn compare(observed: &Value, expected: &Value) -> Result<(), String> {
    match first_difference(observed, expected, "") {
        None => Ok(()),
        Some(difference) => Err(difference),
    }
}

#[test]
fn historical_semantics_use_fixed_trust_fixture() {
    let fixture = trust_fixture();
    // The fixture's key policy is recorded, not injected: the pipeline still uses this
    // build's compiled allowlist, so a change to it must fail here rather than re-judge old
    // anchors in silence.
    assert_eq!(
        fixture.key_policy.server_public_keys,
        SERVER_PUBLIC_KEYS.to_vec(),
        "the compiled server-key allowlist moved; historical anchors would be re-judged"
    );
    assert!(!fixture.key_policy.allow_descriptor_key);
    assert!(fixture.key_policy.extra_server_keys.is_empty());
    assert_eq!(
        fixture.source_commit, "7f0123444ae67c68f09b5e30cbfcc5960344103a",
        "the fixture names the v0.1.0 tag's generated tables"
    );
    assert_ne!(
        fixture_tables(),
        PinTables::CURRENT_BUILD,
        "the historical baseline must not be judged by this build's generated pins"
    );

    let mut failures = Vec::new();
    let definitions = cases();
    for def in &definitions {
        assert!(
            !def.options.allow_descriptor_key && def.options.extra_server_keys.is_empty(),
            "{}: a baseline case borrows no trust from its invocation",
            def.case
        );
        let root = materialize(def);
        let observed = observe(def, root.path());
        if let Err(difference) = compare(&observed, &expected(&def.case)) {
            failures.push(format!("{}: {difference}", def.case));
        }
    }
    assert!(
        failures.is_empty(),
        "historical semantics changed under the fixed trust fixture:\n{}",
        failures.join("\n")
    );

    // No orphaned golden: every frozen document belongs to a declared case.
    let declared: BTreeSet<String> = definitions.iter().map(|d| d.case.clone()).collect();
    let frozen: BTreeSet<String> = std::fs::read_dir(baseline_dir().join("expected"))
        .unwrap()
        .map(|e| {
            let name = e.unwrap().file_name().to_string_lossy().into_owned();
            name.strip_suffix(".json")
                .unwrap_or_else(|| panic!("unexpected baseline file {name}"))
                .to_string()
        })
        .collect();
    assert_eq!(frozen, declared);
}

/// Every committed trace whose chain declares only protocol v1–v4, or which has no chain at
/// all (M0), is covered by a baseline case. A v5 trace is out of this baseline's scope.
#[test]
fn every_committed_legacy_trace_is_frozen() {
    fn traces_under(dir: &Path, out: &mut BTreeSet<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.join("trace.toml").is_file() {
                    out.insert(path.clone());
                }
                traces_under(&path, out);
            }
        }
    }
    let root = repo_root().canonicalize().unwrap();
    let mut found = BTreeSet::new();
    for dir in [
        "crates/goalcup-verifier/tests/fixtures",
        "crates/goalcup-verifier/tests/traces",
        "docs/qualification",
    ] {
        traces_under(&root.join(dir), &mut found);
    }
    let covered: BTreeSet<PathBuf> = cases()
        .iter()
        .map(|c| root.join(&c.source).canonicalize().unwrap())
        .collect();
    for trace in found {
        let versions = chain_protocol_versions(&trace);
        if versions.iter().all(|v| (1..=4).contains(v)) {
            assert!(
                covered.contains(&trace.canonicalize().unwrap()),
                "{} (protocol {versions:?}) is a committed legacy trace with no frozen baseline",
                trace.display()
            );
        }
    }
}

/// The build-relative trust reading may move when the generated pins are regenerated; the
/// frozen historical reading may not. The pins reach exactly one check (V13), the verdict's
/// `manifest_pin`, and the findings V13 adds to the review queue.
#[test]
fn current_build_pin_change_does_not_rewrite_historical_baseline() {
    let def = case("journey-a-2026-09-21-v4-dev");
    let root = materialize(&def);
    let frozen = expected(&def.case);

    let historical = observe(&def, root.path());
    compare(&historical, &frozen).expect("the historical reading is the frozen one");

    // Build-relative trust output: the shipped production path, judged by this build's
    // generated tables. Its pin reading is recomputed from those tables and never compared
    // with the frozen fixture reading, because regenerating the pins may legitimately move it.
    let current =
        goalcup_verifier::verify_trace_with_options(root.path(), def.options.verify_options())
            .expect("the production pipeline reads the historical trace");
    let build_relative = ManifestPinning::resolve(
        &current.connector_id,
        &current.connector_manifest_sha256,
        def.options.dev_mode,
    );
    assert_eq!(current.manifest_pin, build_relative.state);
    let current = verdict_value(&current);

    // A simulated regeneration: every generated pin replaced, as scripts/pin-manifests.sh
    // would after a connector edit. The historical trace no longer matches this build.
    let regenerated_rows = |table: PinTable| -> Vec<(String, String)> {
        table
            .iter()
            .map(|(id, _)| (id.to_string(), "f".repeat(64)))
            .collect()
    };
    let regenerated = PinTables::fixture(
        leak_table(&regenerated_rows(PinTables::CURRENT_BUILD.manifests())),
        leak_table(&regenerated_rows(PinTables::CURRENT_BUILD.directories())),
    );
    let after_regeneration = verdict_under(regenerated, root.path(), &def.options);
    assert_eq!(
        after_regeneration["manifest_pin"],
        serde_json::json!("altered"),
        "the simulated regeneration moved the build-relative reading"
    );
    assert!(
        compare(&after_regeneration, &frozen["verdict"]).is_err(),
        "a regenerated build-relative verdict differs from the frozen one"
    );

    // ...and the frozen reading is untouched by it.
    let again = observe(&def, root.path());
    compare(&again, &frozen).expect("pin regeneration never rewrites the historical baseline");

    // Pins are the only build-relative input: V1–V12 agree across all three trust sources.
    for (label, verdict) in [
        ("build-relative", &current),
        ("regenerated", &after_regeneration),
    ] {
        for i in 0..TRUST_FREE_CHECKS {
            compare(&verdict["checks"][i], &frozen["verdict"]["checks"][i]).unwrap_or_else(
                |difference| {
                    panic!(
                        "{label} check {} depends on pins: {difference}",
                        frozen["verdict"]["checks"][i]["id"]
                    )
                },
            );
        }
        assert_eq!(verdict["checks"][TRUST_FREE_CHECKS]["id"], "V13");
    }
}

/// A single changed evidence byte is refused twice over: by the input identity, and — with
/// that check set aside — by the semantics it changes.
#[test]
fn baseline_rejects_a_changed_evidence_byte() {
    let def = case("compliant-v1");
    let root = materialize(&def);
    let chain = root.path().join("chain.jsonl");
    let original = std::fs::read(&chain).unwrap();
    let needle = b"hello.py";
    let at = original
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("the compliant goal names hello.py");
    let mut changed = original.clone();
    changed[at] = b'j';
    std::fs::write(&chain, &changed).unwrap();

    let frozen = expected(&def.case);
    let observed = observe(&def, root.path());
    let refusal = compare(&observed, &frozen).expect_err("a changed evidence byte is refused");
    assert!(refusal.starts_with("/inputs/chain.jsonl"), "{refusal}");

    let refusal = compare(&observed["verdict"], &frozen["verdict"])
        .expect_err("the changed byte also changes the historical judgement");
    assert!(!refusal.is_empty());
}

/// A frozen document whose finding severity was edited no longer matches the real pipeline.
#[test]
fn baseline_rejects_a_changed_finding_severity() {
    let def = case("journey-a-2026-09-21-v4-dev");
    let root = materialize(&def);
    let observed = observe(&def, root.path());
    let mut doctored = expected(&def.case);
    let finding = &mut doctored["verdict"]["checks"][0]["findings"][0]["severity"];
    let original = finding
        .as_str()
        .expect("a severity is a string")
        .to_string();
    *finding = Value::String(if original == "fail" { "pass" } else { "fail" }.to_string());

    let refusal = compare(&observed, &doctored).expect_err("a changed finding severity is refused");
    assert_eq!(
        refusal,
        format!(
            "/verdict/checks/0/findings/0/severity: observed \"{original}\", frozen \"{}\"",
            doctored["verdict"]["checks"][0]["findings"][0]["severity"]
                .as_str()
                .unwrap()
        )
    );
    // The untouched golden still matches, so the refusal is the edit and nothing else.
    compare(&observed, &expected(&def.case)).expect("the committed golden matches");
}

/// A v5 capture record never reaches the legacy semantic pipeline: a v5 line stops the legacy
/// chain reader as an unsupported version, and a capture record labelled v1–v4 is an
/// incompatible envelope. Both are explicit Fail findings, never silent evidence.
#[test]
fn legacy_pipeline_explicitly_refuses_capture_records() {
    use goalcup_protocol::capture::{CaptureRecord, CaptureTime, GapReason, M2_PROTOCOL_VERSION};
    use goalcup_protocol::chain::Link;
    use goalcup_protocol::envelope::{Envelope, EventBody};

    for (protocol_version, code, says) in [
        (
            M2_PROTOCOL_VERSION,
            "protocol_version_unsupported",
            "declares protocol version 5",
        ),
        (
            4,
            "envelope_version_body_incompatible",
            "cannot carry a capture record introduced in v5",
        ),
    ] {
        let def = case("compliant-v1");
        let root = materialize(&def);
        let chain_path = root.path().join("chain.jsonl");
        let text = std::fs::read_to_string(&chain_path).unwrap();
        let last: Envelope = serde_json::from_str(text.lines().last().unwrap()).unwrap();
        let mut capture = Envelope {
            protocol_version,
            entry_id: last.entry_id.clone(),
            source: "daemon".to_string(),
            recorded_at: last.recorded_at.clone(),
            link: Link {
                seq: last.link.seq + 1,
                prev: last.link.hash.clone(),
                hash: String::new(),
            },
            body: EventBody::Capture {
                record: CaptureRecord::Gap {
                    stream: None,
                    observed: CaptureTime {
                        wall: "2026-09-27T10:00:00Z".to_string(),
                        mono_ns: 1,
                        clock_id: "daemon".to_string(),
                    },
                    reason: GapReason::ProducerLost,
                    lost_bytes: None,
                },
            },
            redaction: None,
        };
        capture.link.hash = capture.expected_hash(&last.link.hash).unwrap();
        let mut appended = text.clone();
        appended.push_str(&serde_json::to_string(&capture).unwrap());
        appended.push('\n');
        std::fs::write(&chain_path, appended).unwrap();

        let verdict =
            goalcup_verifier::verify_trace_with_options(root.path(), def.options.verify_options())
                .expect("the legacy pipeline reports, rather than parses, the capture line");
        let v13 = verdict.checks.iter().find(|c| c.id == "V13").unwrap();
        let finding = v13
            .findings
            .iter()
            .find(|f| f.code == code)
            .unwrap_or_else(|| panic!("v{protocol_version}: no `{code}` in {:#?}", v13.findings));
        assert!(finding.message.contains(says), "{}", finding.message);
        assert_eq!(
            serde_json::to_value(finding.severity).unwrap(),
            serde_json::json!("fail")
        );
        assert_eq!(
            serde_json::to_value(verdict.overall).unwrap(),
            serde_json::json!("fail")
        );
    }
}
