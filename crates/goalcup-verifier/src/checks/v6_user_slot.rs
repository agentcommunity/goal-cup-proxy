//! V6: harness-authored user-slot content is classified, not counted.
//!
//!   * **Every failed clause.** A record that failed scoping is reported under **every**
//!     clause it failed, not only the first. Codex's subagent records fail
//!     `first_session_meta_is_tracked` *and* `not_subagent_thread_source`. A reviewer
//!     asking "which rule caught the subagent?" should get the whole answer from the
//!     verdict.
//!   * **No quoted text.** An excluded record's own text is never quoted in a finding — a
//!     sidechain instruction can legitimately tell a subagent to read a file at an absolute
//!     path (the Claude Code fixture), and Codex's `<environment_context>` stamps one with no
//!     preceding whitespace at all. Redaction is the daemon's job, not the verifier's; this
//!     check cites `store`/`seq` only, exactly as V4 does for the session-directory /
//!     foreign-store-path class of the same leak.

use super::{Check, CheckContext, CheckResult, Evidence, pass, pass_with};
use crate::human_rule::ExclusionReason;
use std::collections::BTreeMap;

pub struct V6UserSlot;

impl Check for V6UserSlot {
    fn id(&self) -> &'static str {
        "V6"
    }
    fn title(&self) -> &'static str {
        "harness-authored user-slot content is classified, not counted"
    }
    fn run(&self, cx: &CheckContext) -> CheckResult {
        // A record can appear under more than one key when it failed more than one
        // scoping clause: that is the point.
        let mut by_reason: BTreeMap<String, Vec<&crate::human_rule::ExcludedRecord>> =
            BTreeMap::new();
        for rec in &cx.human.excluded {
            let keys: Vec<String> = match &rec.reason {
                ExclusionReason::FailedPositive => vec!["failed_positive_test".to_string()],
                ExclusionReason::FailedScoping(clauses) => clauses
                    .iter()
                    .map(|c| {
                        serde_json::to_value(c)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_string))
                            .unwrap_or_else(|| "scoping".to_string())
                    })
                    .collect(),
                ExclusionReason::NoCorroboratingPost => {
                    vec!["no_corroborating_post".to_string()]
                }
            };
            for key in keys {
                by_reason.entry(key).or_default().push(rec);
            }
        }
        let mut findings = Vec::new();
        for (reason, recs) in &by_reason {
            // Sorted by (store, seq) rather than left in leg-1 processing order: records
            // from different stores share a bucket like `failed_positive_test`, so which
            // three the evidence cites must not depend on the order leg-1 stores were
            // declared in.
            let mut sorted: Vec<&crate::human_rule::ExcludedRecord> = recs.clone();
            sorted.sort_by(|a, b| (a.store.as_str(), a.seq).cmp(&(b.store.as_str(), b.seq)));
            // Pass-level, evidence and all: this is a harness-authored record correctly
            // kept out of the user slot. `reason` is the ordinary positive test or a
            // scoping clause. The Fail signal a reviewer must still decide about is V2's
            // `unwitnessed_not_keep_going`.
            findings.push(pass_with(
                "user_slot_excluded",
                format!("{} record(s) excluded as `{reason}`", recs.len()),
                sorted
                    .iter()
                    .take(3)
                    .map(|r| Evidence::at(&r.store, r.seq, None, reason.clone()))
                    .collect(),
            ));
        }

        if findings.is_empty() {
            findings.push(pass(
                "no_harness_authored_user_slot",
                "no harness-authored user-slot content in this trace".to_string(),
            ));
        }
        CheckResult::from_findings(self.id(), self.title(), findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{Severity, support};

    fn run(human: crate::human_rule::HumanRuleOutcome) -> CheckResult {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let parsed = support::parsed();
        let cx = CheckContext {
            trace: &trace,
            manifest: &manifest,
            parsed: &parsed,
            human: &human,
            join_strategy: "prompt_id",
            stores: &[],
            events: &[],
            acks: &[],
            acks_verified: &crate::gaps::NO_ACKS,
            seal: None,
            gaps: &[],
            server_key: None,
            key_source: crate::KeySource::None,
            dev_mode: false,
            chain: None,
            receipt: None,
            baseline: None,
            manifest_sha256: "",
            manifest_pin: crate::ManifestPinning::default(),
        };
        V6UserSlot.run(&cx)
    }

    /// None of the three fixtures leaves `human.excluded` empty, so the pass branch is
    /// never reached by a fixture run.
    #[test]
    fn no_excluded_records_is_a_pass() {
        let result = run(support::human());
        assert_eq!(result.verdict, Severity::Pass);
        assert_eq!(result.findings[0].code, "no_harness_authored_user_slot");
    }

    /// No finding may ever quote the excluded record's own text — only
    /// its store and sequence number, whatever the text says.
    #[test]
    fn a_finding_never_quotes_the_excluded_records_own_text() {
        let mut human = support::human();
        human.excluded.push(crate::human_rule::ExcludedRecord {
            text: "rm -rf /Users/real/name/secret-project".to_string(),
            store: "main".to_string(),
            seq: 1,
            pair_key: None,
            reason: ExclusionReason::FailedPositive,
        });
        let result = run(human);
        for f in &result.findings {
            assert!(
                !f.message.contains("/Users/") && !f.message.contains("secret-project"),
                "message quoted the record's text: {}",
                f.message
            );
            for e in &f.evidence {
                assert!(
                    !e.note.contains("/Users/") && !e.note.contains("secret-project"),
                    "evidence quoted the record's text: {}",
                    e.note
                );
            }
        }
    }
}
