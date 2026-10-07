//! V1: the original prompt is recovered and is within 4,000 characters.

use super::{Check, CheckContext, CheckResult, Evidence, MAX_PROMPT_CHARS, fail, pass};
use crate::human_rule::HumanMessage;

pub struct V1Prompt;

/// The goal is the record with `index == 0` across `messages ∪ unwitnessed`, not merely
/// `messages.first()`. A goal typed inside a hot-reload window, before the hook stream
/// comes back, still occupies index 0 as an `unwitnessed` message; reading only `messages`
/// would let its length go unchecked by V1 and let V2 skip judging it. The returned `bool`
/// says whether the goal was unwitnessed, so the finding can say so.
fn find_goal<'a>(cx: &CheckContext<'a>) -> Option<(&'a HumanMessage, bool)> {
    cx.human
        .messages
        .iter()
        .find(|m| m.index == 0)
        .map(|m| (m, false))
        .or_else(|| {
            cx.human
                .unwitnessed
                .iter()
                .find(|m| m.index == 0)
                .map(|m| (m, true))
        })
}

impl Check for V1Prompt {
    fn id(&self) -> &'static str {
        "V1"
    }
    fn title(&self) -> &'static str {
        "original prompt recovered and within 4,000 characters"
    }
    fn run(&self, cx: &CheckContext) -> CheckResult {
        let findings = match find_goal(cx) {
            None => vec![fail(
                "goal_missing",
                "no human message was recovered, so the goal cannot be established".to_string(),
                Vec::new(),
            )],
            Some((goal, unwitnessed)) => {
                let chars = goal.text.chars().count();
                let evidence = vec![Evidence::at(
                    &goal.leg1_store,
                    goal.leg1_seq,
                    Some(goal.leg1_sha256.clone()),
                    "leg-1 record for the goal",
                )];
                let unwitnessed_note = if unwitnessed {
                    " (unwitnessed: no leg-2 post corroborates it)"
                } else {
                    ""
                };
                if chars > MAX_PROMPT_CHARS {
                    vec![fail(
                        "goal_too_long",
                        format!(
                            "goal is {chars} characters, over the {MAX_PROMPT_CHARS} \
                             limit{unwitnessed_note}"
                        ),
                        evidence,
                    )]
                } else {
                    // `pass` builds the Finding with an empty evidence list; this check has
                    // evidence, so it fills it in. Using the helper keeps the `pass` import
                    // live under `-D warnings`. The text is truncated to 80 characters like
                    // V3's excerpts; the full length was already measured above.
                    let head: String = goal.text.chars().take(80).collect();
                    let mut f = pass(
                        "goal_recovered",
                        format!("goal recovered, {chars} characters{unwitnessed_note}: {head}"),
                    );
                    f.evidence = evidence;
                    vec![f]
                }
            }
        };
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
        V1Prompt.run(&cx)
    }

    /// Neither `messages` nor `unwitnessed` carries a record at `index == 0` — no leg-1
    /// record survived exclusions and scoping at all.
    #[test]
    fn goal_missing_when_no_record_holds_index_zero() {
        let result = run(support::human());
        assert_eq!(result.verdict, Severity::Fail);
        assert_eq!(result.findings[0].code, "goal_missing");
    }

    /// The witnessed case: the goal is `messages[0]` as before, and its length is judged.
    #[test]
    fn goal_too_long_when_witnessed() {
        let mut human = support::human();
        human
            .messages
            .push(support::message(0, &"x".repeat(4_001), Some(0)));
        let result = run(human);
        assert_eq!(result.verdict, Severity::Fail);
        assert_eq!(result.findings[0].code, "goal_too_long");
        assert!(
            !result.findings[0].message.contains("unwitnessed"),
            "{}",
            result.findings[0].message
        );
    }

    /// The headline case: a goal typed inside a hot-reload window has no
    /// leg-2 post, so it lands in `unwitnessed` — still at index 0 — and V1 must judge its
    /// length exactly the same way, while saying it was unwitnessed.
    #[test]
    fn goal_too_long_when_unwitnessed() {
        let mut human = support::human();
        human
            .unwitnessed
            .push(support::message(0, &"x".repeat(4_001), None));
        let result = run(human);
        assert_eq!(result.verdict, Severity::Fail);
        assert_eq!(result.findings[0].code, "goal_too_long");
        assert!(
            result.findings[0].message.contains("unwitnessed"),
            "{}",
            result.findings[0].message
        );
    }
}
