//! Characterization goldens for the harness contract (D42): the daemon half that is reachable
//! through the public surface.
//!
//! * `<id>.daemon.json` per connector, through a real daemon (macOS only, like every test that
//!   reserves a capture chain): what a capture entry created for the connector records and
//!   answers. The `Launch` record's profile id, its digest relation (the SHA-256 of the
//!   connector's `harness.toml`) and whether its folder digest matches the pin; the native
//!   process identity the daemon admits into the process handoff
//!   (`capture.termination.native_binary`); the gaps a launch with and without a native scope
//!   leaves in the chain; and the label and protocol hint of each native stream the entry opens
//!   (`<source>:<path relative to the source root>#g<n>`).
//! * `redaction-extra-patterns.json`, shared: one rule of each form the daemon streams (a JSON
//!   field with an object, string or scalar value, and a header prefix), each with the regex
//!   its form derives and what it does alone and all together over a fixed corpus. The names
//!   are the shipped ones, so the file also pins what those rules strip. It is keyed by form,
//!   not by connector. Every shipped connector's own list is checked to stream, and the list
//!   itself is in its `<id>.json`.
//!
//! The connector list is `shipped_connector_ids()` and the per-connector fields come from each
//! connector's files, so a new connector folder needs no edit here, only its regenerated
//! `<id>.daemon.json`.
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
use goalcup_protocol::harness::{ExtraPattern, FieldValue, HarnessManifest, RedactionRule};
use goalcup_protocol::pins::shipped_connector_ids;
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

fn shipped_harness(id: &str) -> HarnessManifest {
    let path = connectors_root().join(id).join("harness.toml");
    HarnessManifest::from_toml_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// One rule of each form, under a kind of the test's own: the three JSON value forms on the
/// fields Codex writes, then the header form on Claude's billing prefix.
fn one_of_each_form() -> Vec<ExtraPattern> {
    let field = |i: usize, field: &str, value| {
        ExtraPattern::new(
            format!("extra_{i}"),
            RedactionRule::JsonField {
                field: field.into(),
                value,
            },
        )
        .unwrap()
    };
    vec![
        field(0, "rate_limits", FieldValue::Object),
        field(1, "plan_type", FieldValue::String),
        field(2, "credits_remaining", FieldValue::Scalar),
        ExtraPattern::new(
            "extra_3",
            RedactionRule::HeaderPrefix("x-anthropic-billing-".into()),
        )
        .unwrap(),
    ]
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

fn verdict(patterns: &[ExtraPattern]) -> &'static str {
    match StreamRedactor::with_extra_patterns(patterns) {
        Ok(_) => "ok",
        Err(_) => "refused",
    }
}

#[test]
fn the_extra_redaction_recognizers_are_pinned() {
    let corpus: Vec<&[u8]> = vec![
        TOKEN_COUNT,
        SSE_EVENT,
        CREDITS_QUOTED,
        HTTP_HEADERS,
        CLAUDE_BODY,
        HERMES_LINE,
        NEAR_MISS,
    ];
    let all = one_of_each_form();
    let each: Vec<Value> = all
        .iter()
        .map(|p| {
            json!({
                "kind": p.kind(),
                "regex": p.regex(),
                "alone": verdict(std::slice::from_ref(p)),
                "over_the_corpus": StreamRedactor::with_extra_patterns(std::slice::from_ref(p))
                    .map(|r| through(r, &corpus))
                    .unwrap_or(Value::Null),
            })
        })
        .collect();
    // The empty list is the shared policy alone. A rule that cannot be built never reaches the
    // redactor; `ExtraPattern::new` refuses it (protocol `harness_schema.rs`).
    check_golden(
        "redaction-extra-patterns.json",
        &json!({
            "recognizers": each,
            "all_together": verdict(&all),
            "all_together_over_the_corpus": through(StreamRedactor::with_extra_patterns(&all).unwrap(), &corpus),
            "empty_list": verdict(&[]),
            "shared_policy_alone_over_the_corpus": through(StreamRedactor::new(), &corpus),
        }),
    );
}

/// Every shipped connector's `[redaction].extra_patterns` streams as a whole list.
#[test]
fn every_shipped_connector_redaction_list_streams() {
    for id in shipped_connector_ids() {
        let patterns = goalcup_daemon::redact::extra_patterns_of(&shipped_harness(&id));
        assert_eq!(verdict(&patterns), "ok", "{id}: its list does not stream");
    }
}

// ---------------------------------------------------------------------------------------
// A real daemon.
// ---------------------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod live {
    use super::*;
    use crate::common::capture_lifecycle_support::{Rig, launch_request};
    use crate::common::support;

    /// The shipped connectors and the controlled workload, as a connectors directory.
    fn connectors() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for id in shipped_connector_ids() {
            support::copy_tree(&support::shipped_connector_dir(&id), &dir.path().join(&id));
        }
        let template = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../goalcup-cli/tests/fixtures/m2-controlled-workload/harness.toml"),
        )
        .unwrap();
        let rendered = template
            .replace("@WORKLOAD@", "/synthetic/workload.py")
            .replace("@CONFIG@", "/synthetic/config.json");
        let controlled = dir.path().join(CONTROLLED);
        std::fs::create_dir_all(&controlled).unwrap();
        std::fs::write(controlled.join("harness.toml"), rendered).unwrap();
        dir
    }

    /// What the Launch record's directory digest says, in three states: `absent` (no digest
    /// was recorded), `matches_pin` (the digest this build pins for `id`) and `unpinned` (a
    /// digest no pin of `id` matches). A digest literal would change with every connector edit;
    /// one bool would let a daemon that stopped recording the digest pass as an unpinned one.
    fn pin_state(id: &str, digest: Option<&str>) -> &'static str {
        match digest {
            None => "absent",
            Some(d) if goalcup_protocol::pins::connector_dir_is_pinned(id, d) => "matches_pin",
            Some(_) => "unpinned",
        }
    }

    /// The three states are told apart: no digest, the pin, and a digest that is not the pin.
    #[test]
    fn the_launch_digest_state_tells_absent_from_unpinned() {
        let id = &goalcup_protocol::pins::shipped_connector_ids()[0];
        let pin = goalcup_protocol::pins::pinned_connector_dir_sha256(id).unwrap();
        assert_eq!(pin_state(id, None), "absent");
        assert_eq!(pin_state(id, Some(pin)), "matches_pin");
        assert_eq!(pin_state(id, Some(&"0".repeat(64))), "unpinned");
    }

    /// What one created, goal-recorded and launched entry shows.
    async fn entry(rig: &Rig, id: &str) -> Value {
        // The controlled workload's native store lives under the run's own HOME.
        std::fs::create_dir_all(rig.run_home().join("native")).unwrap();
        let info = rig.created_at_its_home(id).await;
        let native_binary = rig
            .daemon
            .native_binary_for_process(&info.entry_id)
            .unwrap();
        let goal = rig.goal_accepted(&info.entry_id, b"write hello.txt").await;
        let launched = rig.launch(&info.entry_id, &launch_request(&goal)).await;
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
            "native_binary_admitted_into_the_process_handoff": native_binary,
            "launch_profile_id": launch.0,
            "launch_profile_digest_is_sha256_of_harness_toml":
                launch.1 == hex::encode(Sha256::digest(&harness)),
            "launch_allowed_tool_rule": launch.2,
            "launch_process_handoff_digest": launch.3,
            "launch_connector_dir_sha256": pin_state(id, launch.4.as_deref()),
            "gaps_after_launch": gaps,
        })
    }

    /// The native streams a connector's entry opens, labelled and hinted, one per
    /// `[[capture.native]]` row, sorted by label: the label is
    /// `<source name>:<path relative to the source root>#g<generation>`, so it names the root
    /// and pattern each connector's native source resolves to.
    async fn native_opens(id: &str) -> Value {
        let dir = connectors();
        let harness = HarnessManifest::from_toml_str(
            &std::fs::read_to_string(dir.path().join(id).join("harness.toml")).unwrap(),
        )
        .unwrap();
        let rows = &harness.capture.native;
        if rows.is_empty() {
            return json!([]);
        }
        let path = dir.path().to_path_buf();
        let rig = Rig::start_with(move |config| config.connectors_dir = path).await;
        let config_home = rig.root.join("config-home");
        // The native file each `[[capture.native]]` row names, each `*` read as `unknown`.
        let files: Vec<PathBuf> = rows
            .iter()
            .map(|native| {
                let file = PathBuf::from(
                    native
                        .root
                        .replace("{config_home}", config_home.to_str().unwrap())
                        .replace("{run_home}", rig.run_home_for(id).to_str().unwrap()),
                )
                .join(native.pattern.replace('*', "unknown"));
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                file
            })
            .collect();
        let info = rig.created_at_its_home(id).await;
        // Written after the create, so a `since_run_start` source (which reads from the create
        // time, in whole seconds) never misses it at a second boundary.
        for file in &files {
            std::fs::write(file, b"{\"type\":\"native\"}\n").unwrap();
        }
        let goal = rig.goal_accepted(&info.entry_id, b"say hi").await;
        assert_eq!(
            rig.launch(&info.entry_id, &launch_request(&goal))
                .await
                .status(),
            204
        );
        // Every row's file must open before the golden is read, so a row that stops opening
        // fails here rather than leaving the golden short.
        let expected = rows.len();
        let distinct_native_labels = |records: &[CaptureRecord]| {
            records
                .iter()
                .filter_map(|r| match r {
                    CaptureRecord::Open {
                        source: CaptureSource::Native,
                        metadata,
                        ..
                    } => Some(metadata.label.clone()),
                    _ => None,
                })
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        };
        let records = rig
            .records_when(&info.entry_id, |records| {
                distinct_native_labels(records) == expected
            })
            .await;
        let mut opens: Vec<Value> = records
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
        opens.sort_by(|a, b| a["label"].as_str().cmp(&b["label"].as_str()));
        json!(opens)
    }

    #[tokio::test]
    async fn the_daemon_side_of_each_connector_is_pinned() {
        let dir = connectors();
        let path = dir.path().to_path_buf();
        let rig = Rig::start_with(move |config| config.connectors_dir = path).await;
        // Every differing connector is reported together, not just the first.
        let mut checked = 0;
        let mut mismatches = Vec::new();
        for id in shipped_connector_ids()
            .iter()
            .map(String::as_str)
            .chain([CONTROLLED])
        {
            let launched = entry(&rig, id).await;
            let native_streams = native_opens(id).await;
            let name = format!("{id}.daemon.json");
            checked += 1;
            mismatches.extend(golden_mismatch(
                &name,
                &json!({
                    "connector": id,
                    "launched": launched,
                    "native_streams_opened": native_streams,
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
