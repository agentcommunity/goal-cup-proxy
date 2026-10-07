//! V2: every human message after the goal equals `keep going` (DECISIONS D8).
//!
//! The check runs over `messages ∪ unwitnessed`. A leg-1 user-slot record that leg 2 never
//! saw is still something a human typed into the user slot; judging only the witnessed ones
//! would let an instruction typed inside the hot-reload window, before the config watch
//! restores the hook, classify itself out of the one rule the whole event rests on.
//! Unwitnessed offenders are reported under their own code, so a reviewer can tell "they
//! typed this and the tracker saw it on both legs" from "they typed this and only the
//! harness saw it".

use super::{Check, CheckContext, CheckResult, Evidence, fail, pass};

pub const KEEP_GOING: &str = "keep going";

pub struct V2KeepGoing;

impl Check for V2KeepGoing {
    fn id(&self) -> &'static str {
        "V2"
    }
    fn title(&self) -> &'static str {
        "every human message after the goal equals `keep going`"
    }
    fn run(&self, cx: &CheckContext) -> CheckResult {
        let mut findings = Vec::new();
        // The goal is index 0 in the single running counter that spans both lists, so
        // "after the goal" is `index > 0` rather than "skip the first of this vector".
        for m in cx.human.messages.iter().filter(|m| m.index > 0) {
            if m.text != KEEP_GOING {
                // Truncated to 80 characters like V3's excerpts; the
                // full text is still in the leg-1 store the evidence cites.
                let head: String = m.text.chars().take(80).collect();
                findings.push(fail(
                    "not_keep_going",
                    format!("message {} is `{head}`, not `{KEEP_GOING}`", m.index),
                    vec![Evidence::at(
                        &m.leg1_store,
                        m.leg1_seq,
                        Some(m.leg1_sha256.clone()),
                        match m.leg2_index {
                            Some(i) => format!("leg-2 post index {i}"),
                            None => "no leg-2 post".to_string(),
                        },
                    )],
                ));
            }
        }
        for m in &cx.human.unwitnessed {
            if m.index == 0 {
                // An unwitnessed *goal* is V1's problem and V3's flag, not V2's.
                continue;
            }
            if m.text != KEEP_GOING {
                // Truncated to 80 characters like V3's excerpts.
                let head: String = m.text.chars().take(80).collect();
                findings.push(fail(
                    "unwitnessed_not_keep_going",
                    format!(
                        "message {} is `{head}`, not `{KEEP_GOING}`, and no leg-2 post \
                         corroborates it — leg 1 recorded it while the hook stream did not",
                        m.index
                    ),
                    vec![Evidence::at(
                        &m.leg1_store,
                        m.leg1_seq,
                        Some(m.leg1_sha256.clone()),
                        "unwitnessed leg-1 user-slot record",
                    )],
                ));
            } else {
                findings.push(super::flag(
                    "unwitnessed_keep_going",
                    format!(
                        "message {} is `{KEEP_GOING}` but has no leg-2 post; the text is \
                         compliant, the capture is not",
                        m.index
                    ),
                    Vec::new(),
                ));
            }
        }
        if findings.is_empty() {
            // A divergent record (V3: `text_divergence`) still spends an index — it is
            // neither in `messages` nor in `unwitnessed` — so leaving it out here would
            // undercount "message(s) after the goal" by exactly the divergent count.
            let later =
                cx.human.messages.len() + cx.human.unwitnessed.len() + cx.human.divergent.len();
            findings.push(pass(
                "all_keep_going",
                format!(
                    "{} message(s) after the goal, all `{KEEP_GOING}`",
                    later.saturating_sub(1)
                ),
            ));
        }
        CheckResult::from_findings(self.id(), self.title(), findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{Severity, support};
    use crate::human_rule::HumanRuleOutcome;

    fn run(human: HumanRuleOutcome) -> CheckResult {
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
        V2KeepGoing.run(&cx)
    }

    #[test]
    fn unwitnessed_message_with_the_wrong_text_fails_under_its_own_code() {
        let mut human = support::human();
        human.unwitnessed.push(support::message(0, "goal", None));
        human
            .unwitnessed
            .push(support::message(1, "do something else", None));
        let result = run(human);
        assert_eq!(result.verdict, Severity::Fail);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == "unwitnessed_not_keep_going")
        );
    }

    #[test]
    fn unwitnessed_message_with_compliant_text_is_flagged_not_failed() {
        let mut human = support::human();
        human.unwitnessed.push(support::message(0, "goal", None));
        human
            .unwitnessed
            .push(support::message(1, KEEP_GOING, None));
        let result = run(human);
        assert_eq!(result.verdict, Severity::Flag);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == "unwitnessed_keep_going")
        );
    }

    /// A divergent record still spends an index, so the count must add `divergent.len()`
    /// back in or it undercounts "message(s) after the goal".
    #[test]
    fn all_keep_going_counts_every_message_after_the_goal() {
        let mut human = support::human();
        human.messages.push(support::message(0, "goal", Some(0)));
        human
            .messages
            .push(support::message(1, KEEP_GOING, Some(1)));
        let result = run(human);
        assert_eq!(result.verdict, Severity::Pass);
        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.findings[0].code, "all_keep_going");
        assert!(
            result.findings[0].message.starts_with("1 message(s)"),
            "{}",
            result.findings[0].message
        );
    }
}
