//! V11: token accounting is derived, not enforced. Plan, credit and billing fields are
//! stripped before anything is published (DECISIONS D26).

use super::{Check, CheckContext, CheckResult, pass};
use crate::parsers::TokenFact;
use std::collections::BTreeMap;

/// Field names that never reach the published summary.
pub const STRIPPED_BEFORE_PUBLICATION: [&str; 5] =
    ["rate_limits", "plan_type", "credit", "quota", "cost"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelTotals {
    pub model: String,
    pub prompt: u64,
    pub completion: u64,
    pub cached: u64,
}

/// Sum per model, sorted by model name so the output is deterministic.
pub fn totals_by_model(tokens: &[TokenFact]) -> Vec<ModelTotals> {
    let mut acc: BTreeMap<&str, ModelTotals> = BTreeMap::new();
    for t in tokens {
        let e = acc.entry(t.model.as_str()).or_insert(ModelTotals {
            model: t.model.clone(),
            prompt: 0,
            completion: 0,
            cached: 0,
        });
        e.prompt += t.prompt;
        e.completion += t.completion;
        e.cached += t.cached;
    }
    acc.into_values().collect()
}

pub struct V11Tokens;

impl Check for V11Tokens {
    fn id(&self) -> &'static str {
        "V11"
    }
    fn title(&self) -> &'static str {
        "derived token summary with plan, credit and billing stripped"
    }
    fn run(&self, cx: &CheckContext) -> CheckResult {
        let totals = totals_by_model(&cx.parsed.tokens);
        let mut findings = Vec::new();
        if totals.is_empty() {
            // D26: no rule ever depends on a token number, so an empty summary is a fact
            // to report, never a defect to flag — V11 emits Pass-level findings only.
            findings.push(pass(
                "no_token_usage",
                "no usage fields found; the summary is empty and no rule depends on it".to_string(),
            ));
        } else {
            findings.push(pass(
                "token_summary",
                totals
                    .iter()
                    .map(|t| {
                        format!(
                            "{}: prompt={} completion={} cached={}",
                            t.model, t.prompt, t.completion, t.cached
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; "),
            ));
        }
        let mut stripped: Vec<&str> = cx
            .parsed
            .tokens
            .iter()
            .flat_map(|t| t.stripped_fields.iter().map(|s| s.as_str()))
            .collect();
        stripped.sort_unstable();
        stripped.dedup();
        findings.push(pass(
            "stripped",
            format!(
                "fields dropped before publication: {} (policy: {})",
                if stripped.is_empty() {
                    "none".to_string()
                } else {
                    stripped.join(", ")
                },
                STRIPPED_BEFORE_PUBLICATION.join(", ")
            ),
        ));
        CheckResult::from_findings(self.id(), self.title(), findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{Severity, support};

    fn run(parsed: crate::parsers::ParsedLegs) -> CheckResult {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let human = support::human();
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
        V11Tokens.run(&cx)
    }

    /// Both connectors record token usage, so the empty-summary branch is only reachable by
    /// construction. D26: V11 never emits anything but a Pass — an empty summary is a fact
    /// to report, not a defect to flag.
    #[test]
    fn no_token_facts_at_all_is_still_a_pass() {
        let result = run(support::parsed());
        assert_eq!(result.verdict, Severity::Pass);
        assert!(result.findings.iter().any(|f| f.code == "no_token_usage"));
        assert!(result.findings.iter().any(|f| {
            f.code == "stripped"
                && f.message
                    .contains("fields dropped before publication: none")
        }));
    }

    /// Two `TokenFact`s for the same model sum rather than overwrite, and the totals are
    /// sorted by model name — the whole point of routing through a `BTreeMap`.
    #[test]
    fn totals_by_model_sums_same_model_facts_and_sorts_by_name() {
        let facts = vec![
            TokenFact {
                model: "gpt-5".to_string(),
                prompt: 10,
                completion: 5,
                cached: 1,
                stripped_fields: Vec::new(),
            },
            TokenFact {
                model: "claude-opus".to_string(),
                prompt: 100,
                completion: 20,
                cached: 0,
                stripped_fields: Vec::new(),
            },
            TokenFact {
                model: "gpt-5".to_string(),
                prompt: 3,
                completion: 1,
                cached: 0,
                stripped_fields: Vec::new(),
            },
        ];
        let totals = totals_by_model(&facts);
        assert_eq!(
            totals,
            vec![
                ModelTotals {
                    model: "claude-opus".to_string(),
                    prompt: 100,
                    completion: 20,
                    cached: 0
                },
                ModelTotals {
                    model: "gpt-5".to_string(),
                    prompt: 13,
                    completion: 6,
                    cached: 1
                },
            ]
        );
    }
}
