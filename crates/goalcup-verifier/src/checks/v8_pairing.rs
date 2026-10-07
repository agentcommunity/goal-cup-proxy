//! V8: request/response pairing where the harness issues identifiers, and through the
//! declared `join_key` where it does not (DECISIONS D2).
//!
//! Counting how much a connector's two id spaces overlap would tell a reviewer nothing
//! about which specific calls or turns failed to pair, so this check pairs:
//!
//!   * For an **id-based join** (`prompt_id`, `message_id`, …) — every leg-2 tool
//!     response whose id no leg-1 call carries is `leg2_tool_without_leg1_call`, and every
//!     leg-1 call whose id no leg-2 response carries is `leg1_call_without_leg2_post`,
//!     each naming the ids. Ordinarily a `Flag`; a `Fail` when the *entire* complementary
//!     leg records zero tool calls while the other leg has some — that is not a partial
//!     mismatch, it is a capture gap.
//!   * For a **turn+order join** (`turn_id+order`, Codex) — leg-1 and leg-2 tool-call
//!     counts are compared per turn, and any turn whose counts differ is
//!     `turn_tool_call_count_mismatch`. On the Codex fixture this genuinely fires once:
//!     the first turn's three leg-1 `exec` macro calls fan out into six leg-2 records,
//!     because a single `exec` call can run several `tools.exec_command()` sub-calls
//!     internally. That is real, useful information, not a defect in the check.
//!   * When **both** legs record zero tool calls at all, there is nothing to pair —
//!     `no_tool_calls` at `Pass`. An entry whose agent used no tool broke no
//!     rule; the two legs agree about that as completely as they can agree about anything.
//!     A leg that records calls the other leg does not is `leg1_call_without_leg2_post`
//!     / `leg2_tool_without_leg1_call`, and both are still raised.

use super::{Check, CheckContext, CheckResult, fail, flag, pass};
use std::collections::{BTreeMap, BTreeSet};

pub struct V8Pairing;

impl Check for V8Pairing {
    fn id(&self) -> &'static str {
        "V8"
    }
    fn title(&self) -> &'static str {
        "request and response pairing, and the declared join key"
    }
    fn run(&self, cx: &CheckContext) -> CheckResult {
        let mut findings = Vec::new();

        let declared = cx
            .manifest
            .join_key
            .as_deref()
            .unwrap_or(cx.manifest.human_rule.pair_key.as_str());
        if declared == cx.join_strategy {
            findings.push(pass(
                "join_key_agrees",
                format!("manifest and parser agree on the join key: {declared}"),
            ));
        } else {
            findings.push(fail(
                "join_key_mismatch",
                format!(
                    "manifest declares `{declared}` but the parser joins by `{}`",
                    cx.join_strategy
                ),
                Vec::new(),
            ));
        }

        let leg1_ids: BTreeSet<&str> = cx
            .parsed
            .tool_calls
            .iter()
            .filter(|t| t.leg == "leg1")
            .map(|t| t.call_id.as_str())
            .collect();
        let leg2_ids: BTreeSet<&str> = cx
            .parsed
            .tool_calls
            .iter()
            .filter(|t| t.leg == "leg2")
            .map(|t| t.call_id.as_str())
            .collect();

        if leg1_ids.is_empty() && leg2_ids.is_empty() {
            findings.push(pass(
                "no_tool_calls",
                "neither leg records any tool call; there is nothing to pair".to_string(),
            ));
        } else if cx.join_strategy == "turn_id+order" {
            // Turn+order join: compare per-turn call counts rather than individual ids,
            // since the id spaces are not the join (V8's own doc comment).
            let mut leg1_counts: BTreeMap<&str, usize> = BTreeMap::new();
            for t in cx.parsed.tool_calls.iter().filter(|t| t.leg == "leg1") {
                if let Some(k) = &t.turn_key {
                    *leg1_counts.entry(k.as_str()).or_default() += 1;
                }
            }
            let mut leg2_counts: BTreeMap<&str, usize> = BTreeMap::new();
            for t in cx.parsed.tool_calls.iter().filter(|t| t.leg == "leg2") {
                if let Some(k) = &t.turn_key {
                    *leg2_counts.entry(k.as_str()).or_default() += 1;
                }
            }
            let turns: BTreeSet<&str> = leg1_counts
                .keys()
                .chain(leg2_counts.keys())
                .copied()
                .collect();
            for turn in turns {
                let c1 = leg1_counts.get(turn).copied().unwrap_or(0);
                let c2 = leg2_counts.get(turn).copied().unwrap_or(0);
                if c1 != c2 {
                    findings.push(flag(
                        "turn_tool_call_count_mismatch",
                        format!("turn `{turn}`: {c1} leg-1 tool call(s), {c2} leg-2 tool call(s)"),
                        Vec::new(),
                    ));
                }
            }

            let unkeyed = cx
                .parsed
                .tool_calls
                .iter()
                .filter(|t| t.leg == "leg1" && t.turn_key.is_none())
                .count();
            if unkeyed > 0 {
                findings.push(flag(
                    "unkeyed_leg1_tool_calls",
                    format!("{unkeyed} leg-1 tool call(s) carry no turn key and cannot be joined"),
                    Vec::new(),
                ));
            }
        } else {
            // Id-based join: pair, don't just count.
            let unmatched_leg2: Vec<&str> = leg2_ids.difference(&leg1_ids).copied().collect();
            let unmatched_leg1: Vec<&str> = leg1_ids.difference(&leg2_ids).copied().collect();

            if !unmatched_leg2.is_empty() {
                let msg = format!(
                    "{} leg-2 tool response(s) with no leg-1 call: {}",
                    unmatched_leg2.len(),
                    unmatched_leg2
                        .iter()
                        .take(10)
                        .copied()
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                // Fail only when leg 1 records zero tool calls at all while leg 2 has
                // some: that is a capture gap, not a partial mismatch.
                findings.push(if leg1_ids.is_empty() {
                    fail("leg2_tool_without_leg1_call", msg, Vec::new())
                } else {
                    flag("leg2_tool_without_leg1_call", msg, Vec::new())
                });
            }
            if !unmatched_leg1.is_empty() {
                let msg = format!(
                    "{} leg-1 tool call(s) with no leg-2 post: {}",
                    unmatched_leg1.len(),
                    unmatched_leg1
                        .iter()
                        .take(10)
                        .copied()
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                findings.push(if leg2_ids.is_empty() {
                    fail("leg1_call_without_leg2_post", msg, Vec::new())
                } else {
                    flag("leg1_call_without_leg2_post", msg, Vec::new())
                });
            }
        }

        CheckResult::from_findings(self.id(), self.title(), findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{Severity, support};
    use crate::parsers::ToolFact;

    fn base_cx<'a>(
        trace: &'a crate::trace::TraceDescriptor,
        manifest: &'a goalcup_protocol::manifest::ConnectorManifest,
        parsed: &'a crate::parsers::ParsedLegs,
        human: &'a crate::human_rule::HumanRuleOutcome,
        join_strategy: &'a str,
    ) -> CheckContext<'a> {
        CheckContext {
            trace,
            manifest,
            parsed,
            human,
            join_strategy,
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
        }
    }

    /// None of the three fixtures ships a parser whose `join_strategy()` disagrees with
    /// its own manifest — that would be an actual bug in a shipped connector. This
    /// constructs the disagreement directly.
    #[test]
    fn a_join_strategy_that_disagrees_with_the_manifest_is_a_fail() {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let parsed = support::parsed();
        let human = support::human();
        let cx = base_cx(&trace, &manifest, &parsed, &human, "not_the_declared_key");
        let result = V8Pairing.run(&cx);
        assert_eq!(result.verdict, Severity::Fail);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == "join_key_mismatch")
        );
        assert!(
            !result.findings.iter().any(|f| f.code == "join_key_agrees"),
            "the two are mutually exclusive"
        );
    }

    /// With no tool calls recorded on either leg, there is nothing to pair — and an entry
    /// whose agent used no tool broke no rule, so the statement is Pass-level.
    #[test]
    fn no_tool_calls_on_either_leg_is_said_at_pass() {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let parsed = support::parsed();
        let human = support::human();
        let cx = base_cx(&trace, &manifest, &parsed, &human, "prompt_id");
        let result = V8Pairing.run(&cx);
        assert!(result.findings.iter().any(|f| f.code == "no_tool_calls"));
        assert!(
            !result
                .findings
                .iter()
                .any(|f| f.code == "leg1_call_without_leg2_post"
                    || f.code == "leg2_tool_without_leg1_call")
        );
    }

    /// Id-based join, partial mismatch on both sides: neither leg is entirely empty, so
    /// both findings are a `Flag`, and the ids themselves are named.
    #[test]
    fn id_based_join_pairs_by_call_id_and_names_the_unmatched_ones() {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let mut parsed = support::parsed();
        parsed.tool_calls.push(ToolFact {
            call_id: "matched".to_string(),
            name: "Read".to_string(),
            leg: "leg1",
            turn_key: Some("t1".to_string()),
            order: 0,
        });
        parsed.tool_calls.push(ToolFact {
            call_id: "matched".to_string(),
            name: "Read".to_string(),
            leg: "leg2",
            turn_key: None,
            order: 0,
        });
        parsed.tool_calls.push(ToolFact {
            call_id: "leg1_only".to_string(),
            name: "Write".to_string(),
            leg: "leg1",
            turn_key: Some("t2".to_string()),
            order: 1,
        });
        parsed.tool_calls.push(ToolFact {
            call_id: "leg2_only".to_string(),
            name: "Write".to_string(),
            leg: "leg2",
            turn_key: None,
            order: 1,
        });
        let human = support::human();
        let cx = base_cx(&trace, &manifest, &parsed, &human, "prompt_id");
        let result = V8Pairing.run(&cx);
        let leg1_missing = result
            .findings
            .iter()
            .find(|f| f.code == "leg1_call_without_leg2_post")
            .expect("leg1_only has no leg-2 counterpart");
        assert_eq!(leg1_missing.severity, Severity::Flag);
        assert!(leg1_missing.message.contains("leg1_only"));
        assert!(!leg1_missing.message.contains("matched"));
        let leg2_missing = result
            .findings
            .iter()
            .find(|f| f.code == "leg2_tool_without_leg1_call")
            .expect("leg2_only has no leg-1 counterpart");
        assert_eq!(leg2_missing.severity, Severity::Flag);
        assert!(leg2_missing.message.contains("leg2_only"));
    }

    /// Id-based join, one leg entirely empty while the other has calls: this is a capture
    /// gap, not a partial mismatch, so the finding for the empty side is a `Fail`.
    #[test]
    fn id_based_join_with_one_leg_entirely_empty_is_a_fail() {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let mut parsed = support::parsed();
        parsed.tool_calls.push(ToolFact {
            call_id: "c1".to_string(),
            name: "Read".to_string(),
            leg: "leg1",
            turn_key: Some("t1".to_string()),
            order: 0,
        });
        parsed.tool_calls.push(ToolFact {
            call_id: "c2".to_string(),
            name: "Read".to_string(),
            leg: "leg1",
            turn_key: Some("t2".to_string()),
            order: 1,
        });
        let human = support::human();
        let cx = base_cx(&trace, &manifest, &parsed, &human, "prompt_id");
        let result = V8Pairing.run(&cx);
        assert_eq!(result.verdict, Severity::Fail);
        let f = result
            .findings
            .iter()
            .find(|f| f.code == "leg1_call_without_leg2_post")
            .expect("leg 2 recorded nothing at all");
        assert_eq!(f.severity, Severity::Fail);
        assert!(
            !result
                .findings
                .iter()
                .any(|f| f.code == "leg2_tool_without_leg1_call"),
            "leg 2 has no ids to report as unmatched"
        );
    }

    /// Turn+order join: per-turn counts are compared, not ids. A turn where the counts
    /// differ is reported by name; a turn where they agree is silent.
    #[test]
    fn turn_order_join_compares_per_turn_counts() {
        let trace = support::trace("codex-cli");
        let manifest = support::manifest("codex-cli");
        let mut parsed = support::parsed();
        // Turn "a": 1 leg-1 call fans out into 2 leg-2 records (mismatch).
        parsed.tool_calls.push(ToolFact {
            call_id: "call_a1".to_string(),
            name: "exec".to_string(),
            leg: "leg1",
            turn_key: Some("a".to_string()),
            order: 0,
        });
        parsed.tool_calls.push(ToolFact {
            call_id: "exec-a1".to_string(),
            name: "Bash".to_string(),
            leg: "leg2",
            turn_key: Some("a".to_string()),
            order: 0,
        });
        parsed.tool_calls.push(ToolFact {
            call_id: "exec-a2".to_string(),
            name: "Bash".to_string(),
            leg: "leg2",
            turn_key: Some("a".to_string()),
            order: 1,
        });
        // Turn "b": 1 leg-1 call, 1 leg-2 record (agrees, silent).
        parsed.tool_calls.push(ToolFact {
            call_id: "call_b1".to_string(),
            name: "exec".to_string(),
            leg: "leg1",
            turn_key: Some("b".to_string()),
            order: 2,
        });
        parsed.tool_calls.push(ToolFact {
            call_id: "exec-b1".to_string(),
            name: "Bash".to_string(),
            leg: "leg2",
            turn_key: Some("b".to_string()),
            order: 2,
        });
        let human = support::human();
        let cx = base_cx(&trace, &manifest, &parsed, &human, "turn_id+order");
        let result = V8Pairing.run(&cx);
        let mismatches: Vec<_> = result
            .findings
            .iter()
            .filter(|f| f.code == "turn_tool_call_count_mismatch")
            .collect();
        assert_eq!(mismatches.len(), 1, "{:?}", result.findings);
        assert!(mismatches[0].message.contains("turn `a`"));
        assert!(mismatches[0].message.contains("1 leg-1"));
        assert!(mismatches[0].message.contains("2 leg-2"));
    }

    /// A leg-1 tool call with no turn key cannot be joined under turn+order, and must be
    /// reported — but this is meaningful only under that join strategy, since an
    /// id-based join does not use `turn_key` at all.
    #[test]
    fn a_leg1_call_with_no_turn_key_is_reported_under_turn_order_join() {
        let trace = support::trace("codex-cli");
        let manifest = support::manifest("codex-cli");
        let mut parsed = support::parsed();
        parsed.tool_calls.push(ToolFact {
            call_id: "call_1".to_string(),
            name: "exec".to_string(),
            leg: "leg1",
            turn_key: None,
            order: 0,
        });
        parsed.tool_calls.push(ToolFact {
            call_id: "exec-1".to_string(),
            name: "Bash".to_string(),
            leg: "leg2",
            turn_key: Some("t1".to_string()),
            order: 0,
        });
        let human = support::human();
        let cx = base_cx(&trace, &manifest, &parsed, &human, "turn_id+order");
        let result = V8Pairing.run(&cx);
        assert!(result.findings.iter().any(
            |f| f.code == "unkeyed_leg1_tool_calls" && f.message.contains("1 leg-1 tool call")
        ));
    }
}
