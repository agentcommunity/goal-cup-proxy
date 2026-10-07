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
//! test -p goalcup-verifier --test v5_corpus regenerate_the_corpus -- --ignored` rewrites the
//! directories (never `expected.toml`), and `git diff` is then the review. Regenerating is for adding a
//! case or for a writer change that is itself a v6 decision; it must not be used to make a
//! failing expectation pass.
//!
//! The one exit-3 case points outside the corpus at the committed pre-v5 `compliant` trace,
//! which has no v5 chain to read.
#![cfg(unix)]

mod support;

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
            assert!(
                output.status.code().is_some_and(|c| c <= 1),
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
    // A public synthetic key, neither the placeholder seed nor any real anchor.
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

    // A receipt, an ack and a signed finalization, with and without a run key on the receipt.
    let other_key = hex::encode(
        SigningKey::from_bytes(&[0x11; 32])
            .verifying_key()
            .to_bytes(),
    );
    for (name, launch_key, receipt_key) in [
        ("signed-receipt-ack-finalization", None, None),
        (
            "signed-receipt-key-agrees",
            Some(real_key.clone()),
            Some(real_key.clone()),
        ),
        (
            "signed-receipt-key-mismatch",
            Some(real_key.clone()),
            Some(other_key.clone()),
        ),
    ] {
        // The receipt binds the chain head after the Launch; the exchange follows it.
        let records = with_launch(launch("codex-cli", launch_key.as_deref(), None));
        let dir = corpus_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exchange = hermes_run();
        build_trace(&dir, &manifest(), &records, |export| {
            let digest = write_receipt(export, receipt_key.as_deref());
            for record in &exchange {
                append_record(export, record);
            }
            let finish = signed_finalization(export, &digest);
            export.append_ack_synced(&sign_ack(&finish)).unwrap();
        });
    }

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
