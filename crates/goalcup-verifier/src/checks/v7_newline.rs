//! V7: exactly one trailing newline is stripped before any byte comparison, and the
//! comparison uses `human_rule.byte_exact_field`. Wire-versus-typed divergence is
//! never tamper evidence on its own.

use super::{Check, CheckContext, CheckResult, fail, pass};

pub struct V7Newline;

impl Check for V7Newline {
    fn id(&self) -> &'static str {
        "V7"
    }
    fn title(&self) -> &'static str {
        "exactly one trailing newline stripped before comparison"
    }
    fn run(&self, cx: &CheckContext) -> CheckResult {
        let mut findings = vec![pass(
            "normalisation_applied",
            format!(
                "exactly one trailing newline {} stripped; comparison uses field `{}`",
                if cx.manifest.normalisation.trailing_newline {
                    "was"
                } else {
                    "was not"
                },
                cx.manifest.human_rule.byte_exact_field
            ),
        )];
        for m in &cx.human.messages {
            if m.text.ends_with('\n') {
                findings.push(fail(
                    "double_newline_survived",
                    format!(
                        "message {} still ends with a newline after normalisation",
                        m.index
                    ),
                    Vec::new(),
                ));
            }
        }
        CheckResult::from_findings(self.id(), self.title(), findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{Severity, support};

    /// None of the three fixtures leaves a message ending in `\n` — normalisation always
    /// succeeds on them — so this branch is only reachable by construction.
    #[test]
    fn a_message_still_ending_in_a_newline_is_a_fail() {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let parsed = support::parsed();
        let mut human = support::human();
        human
            .messages
            .push(support::message(0, "the normaliser missed this\n", Some(0)));
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
        let result = V7Newline.run(&cx);
        assert_eq!(result.verdict, Severity::Fail);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == "double_newline_survived" && f.message.contains("message 0"))
        );
    }
}
