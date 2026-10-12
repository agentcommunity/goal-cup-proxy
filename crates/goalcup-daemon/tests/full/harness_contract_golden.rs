//! Characterization goldens for the harness contract (D42, slice S0): the daemon half that is
//! reachable through the public surface.
//!
//! * `<id>.daemon.json` per connector, through a real daemon (macOS only, like every test that
//!   reserves a capture chain): what a capture entry created for the connector records and
//!   answers. The `Launch` record's profile id and digest relation (the digest is the SHA-256 of
//!   the connector's `harness.toml`), the native process identity the daemon admits into the
//!   process handoff (`capture.termination.native_binary`, taken from the harness at create), and
//!   the gaps a launch with and without a native scope leaves in the chain (the connector's native
//!   binding: at launch, deferred, or not required).
//! * `native-stream-labels.json`: the label and protocol hint of the native stream each
//!   connector's entry opens (`<source>:<path relative to the source root>#g<n>`), which is
//!   where the controlled workload's `native/` prefix is visible.
//! * `redaction-extra-patterns.json`: the extra redaction patterns each connector carries, which
//!   of them the streaming redactor accepts, a refusal for a pattern it has no recognizer for,
//!   and the bytes and audit spans the redactor produces for each shipped connector over a fixed
//!   corpus. `EXTRA_SHAPES` is private, so its recognizers are pinned by what they do.
//!
//! The connector list is the `connectors/` directory (folders holding a `harness.toml`, sorted)
//! and the per-connector fields come from each connector's files, so a new connector folder
//! needs no edit here, only regenerated goldens.
//!
//! Schema-driven native-plan and config-home refusals are tested in
//! `src/runtime/capture/tests.rs`, because those items are `pub(super)`. Goldens are compared
//! byte for byte; `GOALCUP_REGEN_HARNESS_GOLDEN=1` rewrites
//! them (off unless set to exactly `1`). Everything is synthetic and lives in the test's
//! temporary root.

use crate::common::redaction;

use base64::Engine as _;
use goalcup_daemon::redact::stream::StreamRedactor;
use goalcup_protocol::capture::{CaptureRecord, CaptureSource, EndReason};
use goalcup_protocol::harness::HarnessManifest;
use goalcup_protocol::manifest::RedactionPattern;
use redaction::{P1, Producer, daemon_time};
use serde_json::{Value, json};
#[cfg(target_os = "macos")]
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
const CONTROLLED: &str = "m2-controlled-workload";

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../goalcup-cli/tests/fixtures/harness-golden")
}

/// How `value` differs from the committed file `name`, or `None` when they match exactly;
/// with the regenerate switch on it writes the file and reports no difference.
fn golden_mismatch(name: &str, value: &Value) -> Option<String> {
    let text = format!("{}\n", serde_json::to_string_pretty(value).unwrap());
    let path = golden_dir().join(name);
    if std::env::var("GOALCUP_REGEN_HARNESS_GOLDEN").is_ok_and(|v| v == "1") {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, &text).unwrap();
        return None;
    }
    let want = match std::fs::read_to_string(&path) {
        Ok(want) => want,
        Err(e) => {
            return Some(format!(
                "{} is missing ({e}); generate it with GOALCUP_REGEN_HARNESS_GOLDEN=1",
                path.display()
            ));
        }
    };
    if text == want {
        return None;
    }
    let (n, (got, exp)) = text
        .lines()
        .zip(want.lines())
        .enumerate()
        .find(|(_, (a, b))| a != b)
        .unwrap_or((
            text.lines().count().min(want.lines().count()),
            ("<end>", "<end>"),
        ));
    Some(format!(
        "{name} differs from what the code now produces; first difference at line {}:\n  \
         golden: {exp}\n  actual: {got}\n(GOALCUP_REGEN_HARNESS_GOLDEN=1 rewrites it)",
        n + 1
    ))
}

fn check_golden(name: &str, value: &Value) {
    if let Some(mismatch) = golden_mismatch(name, value) {
        panic!("{mismatch}");
    }
}

fn connectors_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../connectors")
}

/// The shipped connectors, by the rule the loaders discover them with: the folders under
/// `connectors/` that hold a `harness.toml`, sorted. A new folder needs no edit here.
fn shipped_ids() -> Vec<String> {
    goalcup_protocol::harness::discover_connector_dirs(&connectors_root())
        .expect("the connectors directory")
        .iter()
        .map(|d| d.file_name().unwrap().to_str().unwrap().to_string())
        .collect()
}

fn shipped_harness(id: &str) -> HarnessManifest {
    let path = connectors_root().join(id).join("harness.toml");
    HarnessManifest::from_toml_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The family the golden shape carries: the connector's capture program. Never keyed by id.
fn family(id: &str) -> String {
    shipped_harness(id).capture.executable.program
}

fn redaction_patterns(id: &str) -> Vec<RedactionPattern> {
    goalcup_daemon::redact::extra_patterns_of(&shipped_harness(id))
}

// ---------------------------------------------------------------------------------------
// Redaction.
// ---------------------------------------------------------------------------------------

const TOKEN_COUNT: &[u8] = b"{\"timestamp\":\"2026-09-28T10:00:00.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"rate_limits\":{\"primary\":{\"used_percent\":12.5,\"resets_in_seconds\":4821},\"credits\":{\"balance\":\"412\"},\"plan_type\":\"plus\"}}}\n";
const SSE_EVENT: &[u8] = b"event: codex.rate_limits\ndata: {\"type\":\"codex.rate_limits\",\"plan_type\":\"pro\",\"rate_limits\":{\"primary\":{\"used_percent\":40.0}},\"credits_remaining\":17.5}\n\n";
const CREDITS_QUOTED: &[u8] = b"{\"credits_remaining\": \"412.00\", \"plan_type\": \"team\"}\n";
const HTTP_HEADERS: &[u8] = b"POST /v1/messages HTTP/1.1\r\nhost: api.example.test\r\nx-anthropic-billing-header: cc_version=2.1.283.4f2; cc_entrypoint=cli\r\ncontent-type: application/json\r\n\r\n";
const CLAUDE_BODY: &[u8] = b"{\"model\":\"claude-haiku\",\"system\":[{\"type\":\"text\",\"text\":\"x-anthropic-billing-header: cc_version=2.1.283.4f2; cc_entrypoint=cli; cch=00000;\"}],\"messages\":[]}\n";
const HERMES_LINE: &[u8] = b"{\"model\":\"deepseek-v4-flash\",\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}],\"plan_type\":\"not-a-hermes-rule\"}\n";
const NEAR_MISS: &[u8] =
    b"{\"rate_limits\": null, \"plan_type\": \"\", \"credits_remaining\": ,}\n";

/// The retained bytes and the audit spans of one stream's records.
fn retained(records: &[CaptureRecord]) -> (Vec<u8>, Vec<Value>) {
    let mut bytes = Vec::new();
    let mut spans = Vec::new();
    for r in records {
        if let CaptureRecord::Segment {
            data_b64, audit, ..
        } = r
        {
            bytes.extend(
                base64::engine::general_purpose::STANDARD
                    .decode(data_b64)
                    .unwrap(),
            );
            for s in &audit.spans {
                spans.push(json!({"kind": s.kind, "start": s.start, "end": s.end}));
            }
        }
    }
    (bytes, spans)
}

fn through(redactor: StreamRedactor, corpus: &[&[u8]]) -> Value {
    let mut p = Producer::with_redactor(P1, redactor);
    let mut records = p.open(1, CaptureSource::WireRequest);
    for part in corpus {
        records.extend(p.bytes(1, part));
    }
    records.extend(p.redactor.flush(daemon_time(1)).unwrap());
    records.extend(p.end(1, EndReason::Complete));
    let (bytes, spans) = retained(&records);
    json!({
        "retained": String::from_utf8_lossy(&bytes),
        "spans": spans,
    })
}

#[test]
fn the_extra_redaction_patterns_and_their_recognizers_are_pinned() {
    let corpus: Vec<&[u8]> = vec![
        TOKEN_COUNT,
        SSE_EVENT,
        CREDITS_QUOTED,
        HTTP_HEADERS,
        CLAUDE_BODY,
        HERMES_LINE,
        NEAR_MISS,
    ];
    let mut per_connector = serde_json::Map::new();
    for id in shipped_ids() {
        let patterns = redaction_patterns(&id);
        let family = family(&id);
        let accepted = StreamRedactor::with_extra_patterns(&patterns).map(|_| "ok");
        let each: Vec<Value> = patterns
            .iter()
            .map(|p| {
                json!({
                    "kind": p.kind,
                    "regex": p.regex,
                    "alone": StreamRedactor::with_extra_patterns(std::slice::from_ref(p))
                        .map(|_| "ok").map_err(|_| "refused").unwrap_or_else(|e| e),
                })
            })
            .collect();
        let redactor = StreamRedactor::with_extra_patterns(&patterns).unwrap();
        per_connector.insert(
            id.clone(),
            json!({
                "family": family,
                "extra_patterns": each,
                "streaming_redactor_accepts_the_list": accepted.map_err(|_| "refused").unwrap_or_else(|e| e),
                "over_the_corpus": through(redactor, &corpus),
            }),
        );
    }
    // A pattern with no recognizer is refused whatever its kind or regex; the empty list is the
    // shared policy alone.
    let stranger = RedactionPattern {
        kind: "stranger".to_string(),
        regex: "foo[0-9]+".to_string(),
    };
    let bad_kind = RedactionPattern {
        kind: "has space".to_string(),
        regex: redaction_patterns("codex-cli")[1].regex.clone(),
    };
    let known_under_another_kind = RedactionPattern {
        kind: "renamed_plan_type".to_string(),
        regex: redaction_patterns("codex-cli")[1].regex.clone(),
    };
    let verdict = |ps: &[RedactionPattern]| match StreamRedactor::with_extra_patterns(ps) {
        Ok(_) => "ok",
        Err(_) => "refused",
    };
    check_golden(
        "redaction-extra-patterns.json",
        &json!({
            "connectors": per_connector,
            "unknown_regex": verdict(std::slice::from_ref(&stranger)),
            "shipped_regex_under_a_bad_kind": verdict(std::slice::from_ref(&bad_kind)),
            "shipped_regex_under_another_kind": verdict(std::slice::from_ref(&known_under_another_kind)),
            "empty_list": verdict(&[]),
            "shared_policy_alone_over_the_corpus": through(StreamRedactor::new(), &corpus),
        }),
    );
}

// ---------------------------------------------------------------------------------------
// A real daemon.
// ---------------------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod live {
    use super::*;
    use crate::common::capture_lifecycle_support::{Rig, launch_request};
    use crate::common::support;
    use goalcup_protocol::harness::{Binding, IdGrammar, IdentitySource};

    /// The shipped connectors and the controlled workload, as a connectors directory.
    fn connectors() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for id in shipped_ids() {
            support::copy_tree(&support::shipped_connector_dir(&id), &dir.path().join(&id));
        }
        let template = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../goalcup-cli/tests/fixtures/m2-controlled-workload/harness.toml"),
        )
        .unwrap();
        let rendered = template
            .replace("@PYTHON3_SHA256@", &"ab".repeat(32))
            .replace("@PYTHON3_VERSION@", "3.12.0")
            .replace("@WORKLOAD@", "/synthetic/workload.py")
            .replace("@CONFIG@", "/synthetic/config.json");
        let controlled = dir.path().join(CONTROLLED);
        std::fs::create_dir_all(&controlled).unwrap();
        std::fs::write(controlled.join("harness.toml"), rendered).unwrap();
        dir
    }

    /// What one created, goal-recorded and launched entry shows.
    async fn entry(rig: &Rig, id: &str, scope: Option<&str>) -> Value {
        // The controlled workload's native store lives under the run's own HOME.
        std::fs::create_dir_all(rig.run_home().join("native")).unwrap();
        let info = rig.created(id).await;
        let native_binary = rig
            .daemon
            .native_binary_for_process(&info.entry_id)
            .unwrap();
        let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
        let mut request = launch_request(&goal);
        request.native_scope = scope.map(str::to_string);
        let launched = rig.launch(&info.entry_id, &request).await;
        assert_eq!(launched.status(), 204, "{}", launched.body());
        let records = rig.records(&info.entry_id);
        let launch = records
            .iter()
            .find_map(|r| match r {
                CaptureRecord::Launch {
                    profile_id,
                    profile_digest,
                    allowed_tool_rule,
                    process_handoff_digest,
                    connector_dir_sha256,
                    ..
                } => Some((
                    profile_id.clone(),
                    profile_digest.clone(),
                    allowed_tool_rule.clone(),
                    process_handoff_digest.clone(),
                    connector_dir_sha256.clone(),
                )),
                _ => None,
            })
            .expect("the launch is chained");
        let harness =
            std::fs::read(rig.config.connectors_dir.join(id).join("harness.toml")).unwrap();
        let gaps: Vec<String> = records
            .iter()
            .filter_map(|r| match r {
                CaptureRecord::Gap { stream, reason, .. } => Some(format!(
                    "{}:{reason:?}",
                    if stream.is_some() {
                        "stream"
                    } else {
                        "streamless"
                    }
                )),
                _ => None,
            })
            .collect();
        json!({
            "native_scope_in_launch_request": scope,
            "native_binary_admitted_into_the_process_handoff": native_binary,
            "launch_profile_id": launch.0,
            "launch_profile_digest_is_sha256_of_harness_toml":
                launch.1 == hex::encode(Sha256::digest(&harness)),
            "launch_allowed_tool_rule": launch.2,
            "launch_process_handoff_digest": launch.3,
            "launch_connector_dir_sha256": launch.4,
            "gaps_after_launch": gaps,
        })
    }

    /// File names under the config home that the first recording of these goldens used where
    /// a native pattern has `*` segments. Optional: any other connector's native file is
    /// derived from its first `[[capture.native]]` entry, `*` read as `unknown`.
    const RECORDED_NATIVE_FILES: [(&str, &str); 2] = [
        (
            "codex-cli",
            "sessions/2026/09/30/rollout-fixture-test-session.jsonl",
        ),
        ("claude-code-cli", "projects/a-slug/test-session.jsonl"),
    ];

    /// The native streams a connector's entry opens, labelled and hinted: the label is
    /// `<source name>:<path relative to the source root>#g<generation>`, so it names the root
    /// and pattern each connector's native source resolves to.
    async fn native_opens(id: &str) -> Value {
        const HERMES_ID: &str = "20260930_141500_a1b2c3";
        let dir = connectors();
        let harness = HarnessManifest::from_toml_str(
            &std::fs::read_to_string(dir.path().join(id).join("harness.toml")).unwrap(),
        )
        .unwrap();
        let Some(native) = harness.capture.native.first() else {
            return json!([]);
        };
        // The identity the launch or the finish names, from the profile's identity rule.
        let identity = &harness.capture.identity;
        let (session, launch_scope, finish_scope): (&str, Option<&str>, Option<&str>) =
            match (identity.source, identity.binding) {
                (IdentitySource::None, _) => ("unused", Some("unused"), None),
                (_, Some(Binding::Deferred)) => {
                    let session = match identity.grammar {
                        Some(IdGrammar::TimestampHex) => HERMES_ID,
                        _ => "test-session",
                    };
                    (session, None, Some(session))
                }
                _ => ("test-session", Some("test-session"), Some("test-session")),
            };
        let path = dir.path().to_path_buf();
        let rig = Rig::start_with(move |config| config.connectors_dir = path).await;
        let config_home = rig.root.join("config-home");
        let file = match RECORDED_NATIVE_FILES.iter().find(|(c, _)| *c == id) {
            Some((_, rel)) => config_home.join(rel),
            None => PathBuf::from(
                native
                    .root
                    .replace("{config_home}", config_home.to_str().unwrap())
                    .replace("{run_home}", rig.run_home().to_str().unwrap()),
            )
            .join(
                native
                    .pattern
                    .replace("{id}", session)
                    .replace('*', "unknown"),
            ),
        };
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"{\"type\":\"native\"}\n").unwrap();
        let info = rig.created(id).await;
        let goal = rig.goal_accepted(&info.entry_id, b"say hi").await;
        let mut request = launch_request(&goal);
        request.native_scope = launch_scope.map(str::to_string);
        assert_eq!(rig.launch(&info.entry_id, &request).await.status(), 204);
        if launch_scope.is_none() {
            // A deferred identity (Hermes) is learnt at the end of the turn: the finish
            // carries it.
            let sealed = rig
                .seal_with(
                    &info.entry_id,
                    &goalcup_protocol::control::CaptureSealRequest {
                        proxy: None,
                        remaining_finish_ms: None,
                        native_scope: finish_scope.map(str::to_string),
                    },
                )
                .await;
            assert_eq!(sealed.status(), 200, "{}", sealed.body());
        }
        let opened = |records: &[CaptureRecord]| {
            records.iter().any(|r| {
                matches!(
                    r,
                    CaptureRecord::Open {
                        source: CaptureSource::Native,
                        ..
                    }
                )
            })
        };
        let records = rig.records_when(&info.entry_id, opened).await;
        let opens: Vec<Value> = records
            .iter()
            .filter_map(|r| match r {
                CaptureRecord::Open {
                    source: CaptureSource::Native,
                    metadata,
                    ..
                } => Some(json!({
                    "label": metadata.label,
                    "protocol_hint": metadata.protocol_hint,
                })),
                _ => None,
            })
            .collect();
        json!(opens)
    }

    #[tokio::test]
    async fn the_native_stream_labels_and_hints_of_each_connector_are_pinned() {
        let mut out = serde_json::Map::new();
        for id in shipped_ids().iter().map(String::as_str).chain([CONTROLLED]) {
            out.insert(id.to_string(), native_opens(id).await);
        }
        check_golden("native-stream-labels.json", &Value::Object(out));
    }

    #[tokio::test]
    async fn the_daemon_side_of_each_connector_is_pinned() {
        let dir = connectors();
        let path = dir.path().to_path_buf();
        let rig = Rig::start_with(move |config| config.connectors_dir = path).await;
        // Every differing connector is reported together, not just the first.
        let mut checked = 0;
        let mut mismatches = Vec::new();
        for id in shipped_ids().iter().map(String::as_str).chain([CONTROLLED]) {
            let with_scope = entry(&rig, id, Some("test-session")).await;
            let without_scope = entry(&rig, id, None).await;
            let name = format!("{id}.daemon.json");
            checked += 1;
            mismatches.extend(golden_mismatch(
                &name,
                &json!({
                    "connector": id,
                    "launched_with_a_native_scope": with_scope,
                    "launched_without_a_native_scope": without_scope,
                }),
            ));
        }
        assert!(
            mismatches.is_empty(),
            "{} of {checked} golden(s) differ:\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }
}
