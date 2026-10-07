//! The verifier's checks. One trait, thirteen implementations, one ordered list.

pub mod v10_partition;
pub mod v11_tokens;
pub mod v12_anchors;
pub mod v13_binding;
pub mod v1_prompt;
pub mod v2_keep_going;
pub mod v3_leg_agreement;
pub mod v4_continuity;
pub mod v5_model;
pub mod v6_user_slot;
pub mod v7_newline;
pub mod v8_pairing;
pub mod v9_config;

use crate::chain::VerifiedChain;
use crate::gaps::{AckReconciliation, Gap};
use crate::human_rule::HumanRuleOutcome;
use crate::legs::Leg1Store;
use crate::parsers::ParsedLegs;
use crate::trace::TraceDescriptor;
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::envelope::Envelope;
use goalcup_protocol::manifest::ConnectorManifest;
use goalcup_protocol::receipt::StartReceipt;
use goalcup_protocol::seal::SealManifest;
use serde::{Deserialize, Serialize};

/// The original prompt limit (RULES.md). Re-exported from `goalcup-protocol`,
/// which is where the number lives, so the daemon's blocking hook and this
/// check read one constant. Do not redefine it here.
pub use goalcup_protocol::MAX_PROMPT_CHARS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Pass,
    Flag,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub store: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sha256: Option<String>,
    pub note: String,
}

impl Evidence {
    /// Evidence that names a store and no record in it.
    pub(crate) fn note(store: impl Into<String>, note: impl Into<String>) -> Evidence {
        Evidence {
            store: store.into(),
            seq: None,
            sha256: None,
            note: note.into(),
        }
    }

    /// Evidence that names the record at `seq` in `store`.
    pub(crate) fn at(
        store: impl Into<String>,
        seq: u64,
        sha256: Option<String>,
        note: impl Into<String>,
    ) -> Evidence {
        Evidence {
            store: store.into(),
            seq: Some(seq),
            sha256,
            note: note.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    pub id: String,
    pub title: String,
    pub verdict: Severity,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub findings: Vec<Finding>,
}

impl CheckResult {
    /// The check's verdict is the worst severity among its findings.
    pub fn from_findings(id: &str, title: &str, findings: Vec<Finding>) -> CheckResult {
        let verdict = findings
            .iter()
            .map(|f| f.severity)
            .max()
            .unwrap_or(Severity::Pass);
        CheckResult {
            id: id.to_string(),
            title: title.to_string(),
            verdict,
            findings,
        }
    }
}

pub struct CheckContext<'a> {
    pub trace: &'a TraceDescriptor,
    pub manifest: &'a ConnectorManifest,
    pub parsed: &'a ParsedLegs,
    pub human: &'a HumanRuleOutcome,
    /// The parser's own declared join strategy, compared against the manifest in V8.
    pub join_strategy: &'a str,
    /// The loaded leg-1 stores, so V12 can reconcile an acknowledgement against the chain
    /// position it names without re-reading the files.
    pub stores: &'a [Leg1Store],
    /// The daemon's chained envelope stream. Empty on an M0 fixture; this is where V9
    /// finds `TailNotice`, V4 and V10 find `ForeignSession`, and the gap classifier
    /// finds `Heartbeat` and `DaemonRestart`.
    pub events: &'a [Envelope],
    /// The acknowledgement stream V12 reconciles. Empty on an M0 fixture, which V12
    /// reports as `no_anchor` rather than as "everything checked out".
    pub acks: &'a [AckCheckpoint],
    /// That stream, already reconciled against the chain and the resolved key — computed
    /// once in `verify_trace`. `classify` anchors on its `verified` set and V12 renders its
    /// `mismatches`, so the gap classifier and the check cannot reach different conclusions
    /// from two separate calls. Use `gaps::NO_ACKS` where there are no acknowledgements.
    pub acks_verified: &'a AckReconciliation,
    /// The seal manifest, when the trace carries one (D11).
    pub seal: Option<&'a SealManifest>,
    /// The classified gaps (D9), computed once in `verify_trace` and read by V4. Computed
    /// in one place rather than per check, so two checks can never disagree about whether
    /// the daemon was down.
    pub gaps: &'a [Gap],
    /// The Ed25519 public key acknowledgement signatures are checked against, already
    /// resolved. **Not** `trace.server_public_key`: that field is
    /// written by the participant, and `verify_trace` honours it only when it is on the
    /// verifier's embedded allowlist (`crate::SERVER_PUBLIC_KEYS`) or the caller passed
    /// `--dev-descriptor-key`. A check that read the descriptor directly would hand the
    /// participant the key that verifies their own signatures, so V12 reads this and
    /// nothing else.
    pub server_key: Option<&'a str>,
    /// Where `server_key` came from, resolved once beside it in `verify_trace`.
    ///
    /// Carried here because the verdict's own `key_source` field is a word in a header and
    /// nothing else: a run made with `--dev-key` or `--dev-descriptor-key` could reach
    /// `overall: pass` with an empty review queue, and a reader of that document would have
    /// no finding to tell them the key was never pinned. `server_key_unpinned` is that
    /// finding, and this is what it reads.
    pub key_source: crate::KeySource,
    /// The caller passed `goalcup-verify --dev` (`VerifyOptions::dev_mode`).
    ///
    /// Read by V13 to suppress `placeholder_server_key`, and by nothing else. The manifest
    /// pin carries its own copy of the same flag in `manifest_pin.allowed`, where it
    /// answers a different question — the two are one option covering two things, which is
    /// why the option is called `dev_mode`.
    pub dev_mode: bool,
    /// The entry's one chained stream, link-checked and split by `chain::load_and_verify`.
    /// `None` on a legacy M0 fixture, which declares its two legs as files. `stores`,
    /// `events` and the parsed leg-2 posts are all derived from this when it is present,
    /// so a check reads it for what those views drop: `break_at`, `seal_link`,
    /// `genesis_link`, `truncated_tail` and the envelope sequence itself.
    pub chain: Option<&'a VerifiedChain>,
    /// The signed start receipt, when the trace names one.
    pub receipt: Option<&'a StartReceipt>,
    /// The `SealManifest` taken at t0, when the trace names one: what the seal is
    /// diffed against.
    pub baseline: Option<&'a SealManifest>,
    /// The sha256, hex, of the **connector manifest bytes as loaded** — the same digest
    /// the verdict carries as `connector_manifest_sha256`. It is the value a start
    /// receipt's `manifest_sha256` claim has to equal, and hashing the bytes that were
    /// actually read is the only way that comparison means anything.
    pub manifest_sha256: &'a str,
    /// Whether that digest is this build's pin for the connector, and whether the caller
    /// allowed it not to be (`goalcup-verify --dev`). Resolved once in `verify_trace` from
    /// the same digest, so V13's finding and the verdict's `manifest_pin` are one answer
    /// rather than two lookups that could part company.
    pub manifest_pin: crate::ManifestPinning,
}

pub trait Check {
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    fn run(&self, cx: &CheckContext) -> CheckResult;
}

pub fn all_checks() -> Vec<Box<dyn Check>> {
    vec![
        Box::new(v1_prompt::V1Prompt),
        Box::new(v2_keep_going::V2KeepGoing),
        Box::new(v3_leg_agreement::V3LegAgreement),
        Box::new(v4_continuity::V4Continuity),
        Box::new(v5_model::V5Model),
        Box::new(v6_user_slot::V6UserSlot),
        Box::new(v7_newline::V7Newline),
        Box::new(v8_pairing::V8Pairing),
        Box::new(v9_config::V9Config),
        Box::new(v10_partition::V10Partition),
        Box::new(v11_tokens::V11Tokens),
        Box::new(v12_anchors::V12Anchors),
        Box::new(v13_binding::V13Binding),
    ]
}

pub(crate) fn pass(code: &str, message: String) -> Finding {
    Finding {
        code: code.to_string(),
        severity: Severity::Pass,
        message,
        evidence: Vec::new(),
    }
}

/// A Pass finding that still cites the records it is about.
///
/// Informational findings a compliant run raises are Pass-level: the sentence stays in
/// `Verdict::checks`, which is where a reviewer reads *what happened*, and out of the
/// queue, which is where a reviewer reads *what needs a decision*.
///
/// Some of them cite evidence — the excluded records, the deviating config file, the
/// foreign session's link — and that citation is the half of the statement a reviewer acts
/// on. `pass` would throw it away, so this exists.
pub(crate) fn pass_with(code: &str, message: String, evidence: Vec<Evidence>) -> Finding {
    Finding {
        code: code.to_string(),
        severity: Severity::Pass,
        message,
        evidence,
    }
}

pub(crate) fn flag(code: &str, message: String, evidence: Vec<Evidence>) -> Finding {
    Finding {
        code: code.to_string(),
        severity: Severity::Flag,
        message,
        evidence,
    }
}

pub(crate) fn fail(code: &str, message: String, evidence: Vec<Evidence>) -> Finding {
    Finding {
        code: code.to_string(),
        severity: Severity::Fail,
        message,
        evidence,
    }
}

/// Fixtures shared by every check's unit tests. `CheckContext` has no
/// optional fields — a test exercising a branch V1 alone reads still has to hand every
/// other check a valid, if empty, `ParsedLegs`/`TraceDescriptor`/`HumanRuleOutcome` — so
/// this builds the cheapest valid value for each once, and a test overrides only the
/// field(s) its scenario needs.
#[cfg(test)]
pub(crate) mod support {
    use crate::atif::{Agent, Trajectory};
    use crate::human_rule::{HumanMessage, HumanRuleOutcome};
    use crate::parsers::ParsedLegs;
    use crate::trace::{Leg2Spec, TraceDescriptor};
    use goalcup_protocol::manifest::ConnectorManifest;
    use std::path::PathBuf;

    /// One of the committed connector manifests — never a manifest built by hand,
    /// so a test's `ConnectorManifest` is always one the crate actually ships. Read from
    /// `connectors/`, never from `spikes/` (read-only fixture input, untouched here).
    pub(crate) fn manifest(connector_id: &str) -> ConnectorManifest {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../connectors")
            .join(connector_id)
            .join("manifest.toml");
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        ConnectorManifest::from_toml_str(&text).unwrap()
    }

    /// A `TraceDescriptor` with no leg-1 stores, raw bodies, events, acks or seal — built
    /// in memory rather than loaded from a `trace.toml`, since a unit test needs no file
    /// on disk.
    pub(crate) fn trace(connector: &str) -> TraceDescriptor {
        TraceDescriptor {
            connector: connector.to_string(),
            entry_id: "test-entry".to_string(),
            workspace: "/ws".to_string(),
            tracked_sessions: vec!["s1".to_string()],
            harness_version: "0.0.0".to_string(),
            connector_manifest: crate::trace_fs::TracePath::parse("manifest.toml").unwrap(),
            leg1: Vec::new(),
            leg2: Some(Leg2Spec {
                path: crate::trace_fs::TracePath::parse("leg2.jsonl").unwrap(),
            }),
            raw: Vec::new(),
            chain: None,
            receipt: None,
            baseline: None,
            expected_reread: false,
            acks: None,
            seal: None,
            server_public_key: None,
            root: crate::trace_fs::TraceRoot::open(std::path::Path::new(".")).unwrap(),
            submission_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// An empty `ParsedLegs`: no user slots, sessions, models, tool calls or markers. A
    /// test that needs one populated clones this and fills in only that field.
    pub(crate) fn parsed() -> ParsedLegs {
        ParsedLegs {
            trajectory: Trajectory {
                schema_version: crate::atif::SCHEMA_VERSION.to_string(),
                session_id: None,
                trajectory_id: None,
                agent: Agent {
                    name: "test".to_string(),
                    version: "0".to_string(),
                    model_name: None,
                    tool_definitions: Vec::new(),
                    extra: None,
                },
                steps: Vec::new(),
                notes: None,
                final_metrics: None,
                continued_trajectory_ref: None,
                extra: None,
                subagent_trajectories: Vec::new(),
            },
            user_slots: Vec::new(),
            leg2_candidates: Vec::new(),
            sessions: Vec::new(),
            models: Vec::new(),
            tokens: Vec::new(),
            tool_calls: Vec::new(),
            config_events: Vec::new(),
            resume_markers: Vec::new(),
            blocked_prompts: Vec::new(),
            raw_bodies: Vec::new(),
            census: std::collections::BTreeMap::new(),
            tool_args_conflicts: Vec::new(),
        }
    }

    /// An empty `HumanRuleOutcome`: no messages, no exclusions, nothing unwitnessed or
    /// divergent.
    pub(crate) fn human() -> HumanRuleOutcome {
        HumanRuleOutcome {
            messages: Vec::new(),
            excluded: Vec::new(),
            unwitnessed: Vec::new(),
            leg1_without_post: Vec::new(),
            leg2_without_record: Vec::new(),
            duplicate_pairings: Vec::new(),
            divergent: Vec::new(),
        }
    }

    /// One `HumanMessage` at the given index and text, paired (`leg2_index: Some(0)`) or
    /// left unwitnessed by the caller depending which list it is pushed into.
    pub(crate) fn message(index: usize, text: &str, leg2_index: Option<usize>) -> HumanMessage {
        HumanMessage {
            index,
            text: text.to_string(),
            pair_key: Some(format!("k{index}")),
            leg1_store: "main".to_string(),
            leg1_seq: index as u64,
            leg1_sha256: "0".repeat(64),
            leg2_index,
            timestamp: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `skip_serializing_if` without `default` would leave a Pass finding — whose
    /// `evidence` vec is empty and so omitted from the JSON — unable to deserialise; serde
    /// needs the field present or a `default` to fill the gap it leaves. This round-trips a
    /// `CheckResult` mixing a Pass finding (empty evidence, omitted) with a Fail finding
    /// whose evidence carries one entry with `seq`/`sha256` present and one with both
    /// absent, so every field `skip_serializing_if` can omit is exercised both ways.
    #[test]
    fn a_check_result_with_omitted_optional_fields_round_trips() {
        let result = CheckResult::from_findings(
            "V1",
            "test check",
            vec![
                pass("ok", "all good".to_string()),
                fail(
                    "bad",
                    "went wrong".to_string(),
                    vec![
                        Evidence::at("main", 3, Some("abc".to_string()), "with hash"),
                        Evidence::note("hooks", "no seq or hash"),
                    ],
                ),
            ],
        );
        let json = serde_json::to_string(&result).unwrap();
        let back: CheckResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, back);
        assert!(back.findings[0].evidence.is_empty());
        assert_eq!(back.findings[1].evidence[1].seq, None);
        assert_eq!(back.findings[1].evidence[1].sha256, None);
    }
}
