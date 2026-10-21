//! The v5 reader corpus (D43 ruling 7; harness contract S1, `docs/CONTRACTS.md` "The v5 format
//! freeze").
//!
//! `tests/fixtures/v5-corpus/` holds small, synthetic, committed v5 trace directories and an
//! `expected.toml` naming, for each, the exact exit of `goalcup-verify --capture-integrity`
//! and the exact finding codes. This test runs the real binary over every one. It is the
//! reader's half of the freeze: the writer's goldens live in `goalcup-protocol/tests/
//! m2_contract.rs`, and this corpus is what a verifier change has to leave alone.
//!
//! **The rule.** A verifier change may not turn a previously valid v5 trace invalid without a
//! `docs/DECISIONS.md` row. An expectation here changes only in the pull request that carries
//! that row. A new case may always be added.
//!
//! The traces were written once by the real protocol writers (`TraceExport`), and every byte of
//! them is synthetic. They are data, not regenerated per run: `GOALCUP_REGEN_V5_CORPUS=1 cargo
//! test -p goalcup-verifier --test it v5_corpus::regenerate_the_corpus -- --ignored` rewrites the
//! directories (never `expected.toml`), and `git diff` is then the review. Regenerating is for adding a
//! case or for a writer change that is itself a v6 decision; it must not be used to make a
//! failing expectation pass.
//!
//! The one exit-3 case, `legacy-pre-v5-trace`, is not written by the writer: it holds the first
//! two records of a protocol-v1 chain, which has no v5 chain to read.

use crate::common as support;

use ed25519_dalek::SigningKey;
use goalcup_protocol::capture::CaptureRecord;
use goalcup_protocol::export::CHAIN_FILE;
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use support::capture_records::*;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v5-corpus")
}

// ---------------------------------------------------------------------------------------
// The verdicts.
// ---------------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Expected {
    case: Vec<Case>,
}

/// Exactly this many cases. Adding one raises it in the same change; removing one is the
/// verdict change this corpus exists to catch, so it needs a DECISIONS row.
const CASES: usize = 27;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    name: String,
    /// Relative to the corpus directory.
    path: String,
    /// The exit of `goalcup-verify --capture-integrity`: 0, 1, 2 or 3.
    exit: i32,
    /// Supply the test signer's public key, as `--dev-key`.
    #[serde(default)]
    dev_key: bool,
    // Everything below is absent for exit 3, which prints no report.
    integrity: Option<String>,
    /// The exact finding codes, in order.
    #[serde(default)]
    findings: Vec<String>,
    sealed: Option<bool>,
    anchor: Option<String>,
    key_source: Option<String>,
    validated_records: Option<u64>,
    validated_head: Option<String>,
    gap_count: Option<u64>,
    coverage_incomplete: Option<bool>,
    torn_tail: Option<bool>,
    /// The terminal's kind, or `none`.
    terminal_kind: Option<String>,
    /// Whether a verified acknowledgement position is reported.
    verified_ack: Option<bool>,
    /// The `--research-json` count of every validated record kind, when pinned.
    record_counts: Option<std::collections::BTreeMap<String, u64>>,
}

fn verify(case: &Case, args: &[&std::ffi::OsStr]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_goalcup-verify"));
    command.args(args);
    if case.dev_key {
        command
            .arg("--dev-key")
            .arg(support::capture_records::server_public_key_hex());
    }
    command.output().unwrap()
}

#[test]
fn every_corpus_trace_keeps_its_exact_verdict() {
    let text = std::fs::read_to_string(corpus_dir().join("expected.toml")).unwrap();
    let expected: Expected = toml::from_str(&text).unwrap();
    assert_eq!(expected.case.len(), CASES, "the corpus case count is exact");
    let mut names = std::collections::BTreeSet::new();
    for case in &expected.case {
        assert!(names.insert(case.name.clone()), "duplicate {}", case.name);
        let root = support::materialize_private(&corpus_dir().join(&case.path));
        let output = verify(
            case,
            &["--capture-integrity".as_ref(), root.path().as_os_str()],
        );
        assert_eq!(
            output.status.code(),
            Some(case.exit),
            "{}: exit\nstderr: {}",
            case.name,
            String::from_utf8_lossy(&output.stderr)
        );
        if case.exit == 3 {
            assert!(output.stdout.is_empty(), "{}: no usable report", case.name);
            continue;
        }
        let report: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|e| panic!("{}: stdout is not a report: {e}", case.name));
        let name = &case.name;
        assert_eq!(
            report["integrity"].as_str(),
            case.integrity.as_deref(),
            "{name}: integrity"
        );
        let findings: Vec<&str> = report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|code| code.as_str().unwrap())
            .collect();
        assert_eq!(findings, case.findings, "{name}: findings");
        assert_eq!(report["sealed"].as_bool(), case.sealed, "{name}: sealed");
        assert_eq!(
            report["anchor"].as_str(),
            case.anchor.as_deref(),
            "{name}: anchor"
        );
        assert_eq!(
            report["key_source"].as_str(),
            case.key_source.as_deref(),
            "{name}: key_source"
        );
        assert_eq!(
            report["validated_records"].as_u64(),
            case.validated_records,
            "{name}: validated_records"
        );
        assert_eq!(
            report["validated_head"].as_str(),
            case.validated_head.as_deref(),
            "{name}: validated_head"
        );
        assert_eq!(
            report["gap_count"].as_u64(),
            case.gap_count,
            "{name}: gap_count"
        );
        assert_eq!(
            report["coverage_incomplete"].as_bool(),
            case.coverage_incomplete,
            "{name}: coverage_incomplete"
        );
        assert_eq!(
            report["torn_tail"].as_bool(),
            case.torn_tail,
            "{name}: torn_tail"
        );
        assert_eq!(
            report["terminal"]["kind"].as_str().unwrap_or("none"),
            case.terminal_kind.as_deref().unwrap(),
            "{name}: terminal"
        );
        assert_eq!(
            !report["verified_ack"].is_null(),
            case.verified_ack.unwrap(),
            "{name}: verified_ack"
        );
        if let Some(counts) = &case.record_counts {
            // The research report reads the same trace through the same reader; its counts
            // say which record kinds the reader validated.
            let out_dir = tempfile::tempdir().unwrap();
            let ndjson = out_dir.path().join("report.ndjson");
            let output = verify(
                case,
                &[
                    "--research-json".as_ref(),
                    root.path().as_os_str(),
                    "--output".as_ref(),
                    ndjson.as_os_str(),
                ],
            );
            // D63 preserves contradictory signed fixtures as invalid, including research.
            let research_exit = output.status.code();
            assert!(
                if case.exit == 2 {
                    research_exit == Some(2)
                } else {
                    research_exit.is_some_and(|c| c <= 1)
                },
                "{name}: research exit"
            );
            let text = std::fs::read_to_string(&ndjson).unwrap();
            let summary: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
            let got: std::collections::BTreeMap<String, u64> = summary["capture"]["record_counts"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.as_u64().unwrap()))
                .collect();
            assert_eq!(&got, counts, "{name}: record_counts");
        }
    }
}

/// The corpus holds exactly the directories `expected.toml` names, so a case cannot be
/// dropped from the table and left on disk (or the reverse) without the test noticing.
#[test]
fn the_corpus_directory_and_the_table_agree() {
    let text = std::fs::read_to_string(corpus_dir().join("expected.toml")).unwrap();
    let expected: Expected = toml::from_str(&text).unwrap();
    let named: std::collections::BTreeSet<String> = expected
        .case
        .iter()
        .filter(|case| !case.path.starts_with(".."))
        .map(|case| case.path.clone())
        .collect();
    let on_disk: std::collections::BTreeSet<String> = std::fs::read_dir(corpus_dir())
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_type().unwrap().is_dir())
        .map(|entry| entry.file_name().into_string().unwrap())
        .collect();
    assert_eq!(named, on_disk);
}

// ---------------------------------------------------------------------------------------
// The chain as rotating parts (D55).
// ---------------------------------------------------------------------------------------

/// A first record that fails a check after its entry id was read does not name the entry, and a
/// chain that starts above byte 0 has no trusted position for its first record. The mismatching
/// Launch of `signed-receipt-key-mismatch` is its sixth record; keeping only the parts from it on
/// makes it the first record of a locally pruned chain.
#[test]
fn a_pruned_chains_failing_first_record_names_neither_entry_nor_seq() {
    let root = support::materialize_private(&corpus_dir().join("signed-receipt-key-mismatch"));
    support::split_into_parts(root.path(), &[5]);
    std::fs::remove_file(root.path().join(CHAIN_FILE)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_goalcup-verify"))
        .args(["--capture-integrity".as_ref(), root.path().as_os_str()])
        .args([
            "--dev-key",
            &support::capture_records::server_public_key_hex(),
        ])
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(2), "{report:#}");
    assert_eq!(report["validated_records"], 0, "{report:#}");
    assert_eq!(report["entry_id"], Value::Null, "{report:#}");
    assert_eq!(
        report["first_failure"],
        serde_json::json!({"seq": null, "code": "record_field_invalid"}),
        "{report:#}"
    );
}

/// Part naming and reading are on-disk format, and the uploader depends on them. Every corpus
/// trace, cut into parts the way a rotating writer leaves them, must read exactly as the one
/// file does and as `expected.toml` says. Nothing here regenerates a byte of the corpus: the
/// copy is split, the committed directory is not.
#[test]
fn every_corpus_trace_reads_the_same_when_rotated() {
    let text = std::fs::read_to_string(corpus_dir().join("expected.toml")).unwrap();
    let expected: Expected = toml::from_str(&text).unwrap();
    let mut rotated_cases = 0;
    for case in &expected.case {
        if case.exit == 3 || case.path.starts_with("..") {
            continue; // no v5 chain to split
        }
        let name = &case.name;
        let whole = support::materialize_private(&corpus_dir().join(&case.path));
        let parts = support::materialize_private(&corpus_dir().join(&case.path));
        let lines = std::fs::read(parts.path().join(CHAIN_FILE))
            .unwrap()
            .iter()
            .filter(|byte| **byte == b'\n')
            .count();
        if lines < 3 {
            continue;
        }
        let mut cuts = vec![1, lines / 2, lines - 1];
        cuts.dedup();
        let made = support::split_into_parts(parts.path(), &cuts);
        assert_eq!(made.len(), cuts.len() + 1, "{name}: parts written");

        let run = |root: &Path| {
            let output = verify(case, &["--capture-integrity".as_ref(), root.as_os_str()]);
            let report: Value = serde_json::from_slice(&output.stdout)
                .unwrap_or_else(|e| panic!("{name}: stdout is not a report: {e}"));
            (output.status.code(), report)
        };
        let (whole_exit, whole_report) = run(whole.path());
        let (exit, report) = run(parts.path());
        assert_eq!(exit, Some(case.exit), "{name}: rotated exit\n{report}");
        assert_eq!(exit, whole_exit, "{name}: exit differs from one file");
        assert_eq!(
            report, whole_report,
            "{name}: the report differs from one file"
        );
        let findings: Vec<&str> = report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|code| code.as_str().unwrap())
            .collect();
        assert_eq!(findings, case.findings, "{name}: rotated findings");
        assert_eq!(
            report["validated_head"].as_str(),
            case.validated_head.as_deref(),
            "{name}: rotated validated_head"
        );
        rotated_cases += 1;
    }
    assert!(rotated_cases >= 20, "only {rotated_cases} cases were split");
}

// ---------------------------------------------------------------------------------------
// The seal of a clone (D56).
// ---------------------------------------------------------------------------------------

/// A seal that records the Git state instead of the files, beside a baseline that lists the
/// harness home: the reader binds both through the manifest digest and the trace is valid. A
/// `seal.json` whose uncommitted digest was edited afterwards no longer is.
#[test]
fn a_seal_with_git_verifies() {
    use goalcup_protocol::seal::{GitState, SealedFile, WorkspaceGit};
    let mut baseline = manifest();
    baseline.git = Some(GitState::Read(WorkspaceGit {
        commit: Some("ab".repeat(20)),
        dirty_sha256: "cd".repeat(32),
    }));
    baseline.home = Some(vec![SealedFile {
        path: "config.toml".to_string(),
        bytes: 3,
        sha256: "ef".repeat(32),
    }]);
    let mut seal = manifest();
    seal.git = Some(GitState::Read(WorkspaceGit {
        commit: None,
        dirty_sha256: "12".repeat(32),
    }));
    let dir = tempfile::tempdir().unwrap();
    let mut records = goal();
    records.push(launch("codex-cli", None, None));
    build_trace(dir.path(), &baseline, &records, |export| {
        local_finish(export, &seal, confirmed());
    });
    let case = |root: &Path| {
        let output = Command::new(env!("CARGO_BIN_EXE_goalcup-verify"))
            .arg("--capture-integrity")
            .arg(root)
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        (output.status.code(), report)
    };
    let (exit, report) = case(dir.path());
    assert_eq!(exit, Some(0), "{report}");
    assert_eq!(report["integrity"].as_str(), Some("valid"), "{report}");
    assert_eq!(report["sealed"].as_bool(), Some(true), "{report}");

    let path = dir.path().join("seal.json");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace(&"12".repeat(32), &"34".repeat(32))).unwrap();
    let (exit, report) = case(dir.path());
    assert_ne!(
        exit,
        Some(0),
        "an edited Git digest must not verify: {report}"
    );
}

/// D56 amended: a clone whose Git state could not be read still seals, with the reason in place
/// of the state. The trace is incomplete (exit 1), never invalid, and the report names why, once
/// for the baseline and once for the seal.
#[test]
fn an_unreadable_clone_is_incomplete_and_says_why() {
    use goalcup_protocol::seal::GitState;
    let unreadable = |why: &str| {
        let mut manifest = manifest();
        manifest.git = Some(GitState::Unreadable {
            unreadable: why.to_string(),
        });
        manifest
    };
    let run = |baseline: &goalcup_protocol::seal::SealManifest,
               seal: &goalcup_protocol::seal::SealManifest| {
        let dir = tempfile::tempdir().unwrap();
        let mut records = goal();
        records.push(launch("codex-cli", None, None));
        build_trace(dir.path(), baseline, &records, |export| {
            local_finish(export, seal, confirmed());
        });
        let output = Command::new(env!("CARGO_BIN_EXE_goalcup-verify"))
            .arg("--capture-integrity")
            .arg(dir.path())
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        (output.status.code(), report, stderr)
    };

    let (exit, report, stderr) = run(&manifest(), &unreadable("`git ls-files` failed"));
    assert_eq!(exit, Some(1), "{report}");
    assert_eq!(report["integrity"].as_str(), Some("incomplete"), "{report}");
    assert_eq!(report["sealed"].as_bool(), Some(true), "{report}");
    let findings = report["findings"].to_string();
    assert!(findings.contains("seal_git_unreadable"), "{report}");
    assert!(!findings.contains("baseline_git_unreadable"), "{report}");
    assert_eq!(
        report["git_unreadable"],
        serde_json::json!([
            "the workspace's Git state could not be read at the seal: `git ls-files` failed"
        ]),
        "{report}"
    );
    assert!(
        stderr.contains("the workspace's Git state could not be read at the seal"),
        "{stderr}"
    );

    // At the baseline too; a reason with a control character is printed without it.
    let (exit, report, _) = run(
        &unreadable("the time to read it ran out"),
        &unreadable("bad\u{1b}[2Jname"),
    );
    assert_eq!(exit, Some(1), "{report}");
    let findings = report["findings"].to_string();
    assert!(findings.contains("baseline_git_unreadable"), "{report}");
    assert!(findings.contains("seal_git_unreadable"), "{report}");
    assert_eq!(
        report["git_unreadable"],
        serde_json::json!([
            "the workspace's Git state could not be read at the baseline: the time to read it ran out",
            "the workspace's Git state could not be read at the seal: bad?[2Jname"
        ]),
        "{report}"
    );

    // A readable clone has no such field.
    let (exit, report, _) = run(&manifest(), &manifest());
    assert_eq!(exit, Some(0), "{report}");
    assert!(report.get("git_unreadable").is_none(), "{report}");

    // A baseline that read a clone beside a seal that walked a workspace: an older daemon, or a
    // path that missed the clone. Incomplete, never valid.
    let mut clone = manifest();
    clone.git = Some(GitState::Read(goalcup_protocol::seal::WorkspaceGit {
        commit: None,
        dirty_sha256: "cd".repeat(32),
    }));
    let (exit, report, _) = run(&clone, &manifest());
    assert_eq!(exit, Some(1), "{report}");
    assert_eq!(report["integrity"].as_str(), Some("incomplete"), "{report}");
    assert!(
        report["findings"].to_string().contains("seal_git_missing"),
        "{report}"
    );
    let (exit, report, _) = run(&clone, &unreadable("it has no .git directory"));
    assert_eq!(exit, Some(1), "{report}");
    assert!(
        !report["findings"].to_string().contains("seal_git_missing"),
        "{report}"
    );
}

/// The two fields are skipped when absent, so a manifest written before them serializes to
/// the same bytes and keeps the digest its terminal binds.
#[test]
fn a_manifest_without_the_new_fields_keeps_its_digest() {
    for name in ["sealed-with-files", "sealed-old-launch"] {
        for file in ["baseline.json", "seal.json"] {
            let bytes = std::fs::read(corpus_dir().join(name).join(file)).unwrap();
            let manifest: goalcup_protocol::seal::SealManifest =
                serde_json::from_slice(&bytes).unwrap();
            assert_eq!(manifest.git, None);
            assert_eq!(manifest.home, None);
            assert_eq!(
                goalcup_protocol::export::manifest_sha256(&manifest).unwrap(),
                hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&bytes)),
                "{name}/{file}"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------
// The writer of the corpus (runs only when asked to).
// ---------------------------------------------------------------------------------------

/// Rewrite the `Launch` line of an unsealed trace's chain by text, for a spelling the writer
/// never produces. The link hash does not cover such bytes, so the reader must catch them.
fn edit_launch_line(dir: &Path, from: &str, to: &str) {
    let path = dir.join(CHAIN_FILE);
    let text = std::fs::read_to_string(&path).unwrap();
    let mut done = false;
    let lines: Vec<String> = text
        .split_inclusive('\n')
        .map(|line| {
            if line.contains(r#""kind":"launch""#) {
                assert!(line.contains(from), "{from} not in the launch line");
                done = true;
                line.replacen(from, to, 1)
            } else {
                line.to_string()
            }
        })
        .collect();
    assert!(done, "no launch line");
    std::fs::write(&path, lines.concat()).unwrap();
}

#[test]
#[ignore = "rewrites the committed corpus; run with GOALCUP_REGEN_V5_CORPUS=1 and --ignored"]
fn regenerate_the_corpus() {
    assert!(
        std::env::var_os("GOALCUP_REGEN_V5_CORPUS").is_some(),
        "set GOALCUP_REGEN_V5_CORPUS=1 to rewrite the corpus"
    );
    // A public, test-only key the verifier never trusts.
    let real_key = hex::encode(
        SigningKey::from_bytes(&[0x42; 32])
            .verifying_key()
            .to_bytes(),
    );
    let dir_digest = "ab".repeat(32);
    let with_launch = |launch: CaptureRecord| {
        let mut records = goal();
        records.push(launch);
        records
    };
    let case = |name: &str, records: Vec<CaptureRecord>, sealed: bool| {
        let dir = corpus_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_trace(&dir, &records, sealed);
        dir
    };

    let mut vertical = with_launch(launch("codex-cli", None, None));
    vertical.extend(hermes_run());
    case("unsealed-vertical", vertical, false);
    case(
        "sealed-old-launch",
        with_launch(launch("codex-cli", None, None)),
        true,
    );
    case(
        "sealed-new-launch",
        with_launch(launch("codex-cli", Some(&real_key), Some(&dir_digest))),
        true,
    );
    case(
        "sealed-key-only",
        with_launch(launch("codex-cli", Some(&real_key), None)),
        true,
    );
    case(
        "sealed-digest-only",
        with_launch(launch("codex-cli", None, Some(&dir_digest))),
        true,
    );
    let mut hermes = with_launch(launch("hermes-cli", Some(&real_key), Some(&dir_digest)));
    hermes.extend(hermes_run());
    case("sealed-hermes-capture-only", hermes, true);
    case(
        "launch-key-not-hex",
        with_launch(launch("codex-cli", Some(&"zz".repeat(32)), None)),
        true,
    );
    case(
        "launch-key-uppercase",
        with_launch(launch("codex-cli", Some(&real_key.to_uppercase()), None)),
        true,
    );
    case(
        "launch-key-weak-point",
        with_launch(launch(
            "codex-cli",
            Some(&format!("01{}", "00".repeat(31))),
            None,
        )),
        true,
    );
    case(
        "launch-digest-uppercase",
        with_launch(launch("codex-cli", None, Some(&dir_digest.to_uppercase()))),
        true,
    );
    case(
        "launch-digest-short",
        with_launch(launch("codex-cli", None, Some(&"ab".repeat(31)))),
        true,
    );
    let dir = case(
        "launch-key-explicit-null",
        with_launch(launch("codex-cli", None, None)),
        false,
    );
    edit_launch_line(
        &dir,
        r#""continuation":"none""#,
        r#""continuation":"none","run_public_key":null"#,
    );

    // ----- The rest of what the reader accepts. -----
    let case_with =
        |name: &str,
         manifest: &goalcup_protocol::seal::SealManifest,
         records: Vec<CaptureRecord>,
         finish: &dyn Fn(&mut goalcup_protocol::export::TraceExport)| {
            let dir = corpus_dir().join(name);
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            build_trace(&dir, manifest, &records, |export| finish(export));
            dir
        };
    let plain = || with_launch(launch("codex-cli", None, None));
    let sealed =
        |manifest: &goalcup_protocol::seal::SealManifest,
         termination: goalcup_protocol::provenance::TerminationOutcomeV1| {
            let manifest = manifest.clone();
            move |export: &mut goalcup_protocol::export::TraceExport| {
                local_finish(export, &manifest, termination.clone());
            }
        };

    let mut records = plain();
    records.extend(gap_and_assistance());
    case("sealed-gap-and-assistance", records, true);
    let mut records = plain();
    records.extend(recovered_wire_run());
    case("sealed-recovery", records, true);
    let mut records = plain();
    records.extend(websocket_run());
    case("sealed-websocket", records, true);
    for (name, reason) in [
        (
            "sealed-end-interrupted",
            goalcup_protocol::capture::EndReason::Interrupted,
        ),
        (
            "sealed-end-decode-error",
            goalcup_protocol::capture::EndReason::DecodeError,
        ),
    ] {
        let mut records = plain();
        records.extend(wire_stream_ending(reason));
        case(name, records, true);
    }
    case(
        "sealed-claude-allowance",
        with_launch(claude_launch_with_allowance()),
        true,
    );
    case_with(
        "sealed-with-files",
        &manifest_with_files(),
        plain(),
        &sealed(&manifest_with_files(), confirmed()),
    );
    let utc = |v: &str| goalcup_protocol::provenance::Rfc3339Utc::try_from(v).unwrap();
    case_with(
        "sealed-termination-already-stopped",
        &manifest(),
        plain(),
        &sealed(
            &manifest(),
            goalcup_protocol::provenance::TerminationOutcomeV1::AlreadyStopped {
                checked_at: utc("2026-10-02T12:00:02Z"),
            },
        ),
    );
    case_with(
        "sealed-termination-unresolved",
        &manifest(),
        plain(),
        &sealed(
            &manifest(),
            goalcup_protocol::provenance::TerminationOutcomeV1::Unresolved {
                requested_at: utc("2026-10-02T12:00:02Z"),
                stopped_waiting_at: utc("2026-10-02T12:00:04Z"),
                reason: goalcup_protocol::provenance::TerminationFailureV1::ConfirmationTimedOut,
            },
        ),
    );

    // `signed-receipt-ack-finalization`, `signed-receipt-key-agrees` and
    // `signed-receipt-key-mismatch` end in a signed-mode cutoff and finalization. Their writer
    // left with the D54 amendment of 21 Oct 2026, so this generator no longer writes them: their
    // committed bytes are frozen reader inputs, and regenerating leaves the three directories
    // exactly as they are.

    // A torn tail: a real trace plus the front of a line that never got its newline.
    let dir = case("torn-tail", plain(), false);
    {
        use std::io::Write as _;
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join(CHAIN_FILE))
            .unwrap()
            .write_all(br#"{"protocol_version":5,"entry_id":"entry-v5-corpus","sour"#)
            .unwrap();
    }
}
