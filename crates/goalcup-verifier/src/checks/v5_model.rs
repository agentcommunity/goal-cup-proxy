//! V5: model identity and changes reported, or `unknown` — **and version drift reported**.
//! Version drift is read here from the `VersionChange` event the
//! daemon emits. It folds naturally here, because a mid-run change of *what was answering*
//! is one question whether the thing that changed was the model or the harness binary. D12
//! makes it reportable, never disqualifying.

use super::{Check, CheckContext, CheckResult, Evidence, flag, pass};
use goalcup_protocol::envelope::EventBody;
use std::collections::BTreeSet;

pub struct V5Model;

impl Check for V5Model {
    fn id(&self) -> &'static str {
        "V5"
    }
    fn title(&self) -> &'static str {
        "model identity and changes reported"
    }
    fn run(&self, cx: &CheckContext) -> CheckResult {
        // Deduplicate but keep first-seen order: the sequence is the evidence. The
        // `"unknown"` sentinel is a parser's fail-closed default for a field it could not
        // read, never a measured identity — counting it would let a run with one real model
        // and one unreadable field pass as `single_model` instead of reporting that the
        // identity was never established.
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut order: Vec<&str> = Vec::new();
        for m in cx.parsed.models.iter().filter(|m| m.model != "unknown") {
            if seen.insert(m.model.as_str()) {
                order.push(m.model.as_str());
            }
        }
        let mut findings = if order.is_empty() {
            vec![flag(
                "model_unknown",
                "no model identity recorded anywhere in either leg".to_string(),
                Vec::new(),
            )]
        } else if order.len() == 1 {
            vec![pass(
                "single_model",
                format!("one model throughout: {}", order[0]),
            )]
        } else {
            // Pass-level. No rule anywhere says an entry runs on one model, so
            // naming the models used is the same kind of statement `single_model` is: the
            // list, at the severity the list deserves. `version_drift` below — the harness
            // updating itself out from under a declared range — is still a Flag.
            vec![pass(
                "model_changed",
                format!(
                    "models observed, in order of first appearance: {}",
                    order.join(" -> ")
                ),
            )]
        };

        // Version drift. D12 says the tested range is declared and a mid-run change is
        // recorded; the ChatGPT app was measured self-updating mid-run, so this is measured
        // behaviour, not a hypothetical.
        let mut drifted = 0usize;
        for env in cx.events {
            if let EventBody::VersionChange(v) = &env.body {
                drifted += 1;
                findings.push(flag(
                    "version_drift",
                    format!(
                        "`{}` changed from {} to {} at {}; the entry was started against \
                         the tested range `{}`",
                        v.component, v.from, v.to, v.detected_at, cx.manifest.tested_version_range
                    ),
                    vec![Evidence::at(
                        "events",
                        env.link.seq,
                        Some(env.link.hash.clone()),
                        &v.component,
                    )],
                ));
            }
        }
        if drifted == 0 {
            findings.push(pass(
                "no_version_drift",
                format!(
                    "no version change recorded; harness reported as {} against tested range `{}`",
                    cx.trace.harness_version, cx.manifest.tested_version_range
                ),
            ));
        }
        CheckResult::from_findings(self.id(), self.title(), findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::support;
    use crate::parsers::ModelFact;

    /// `"unknown"` is a parser's fail-closed default, not a measured
    /// model. A run where every `ModelFact` reads `"unknown"` must report `model_unknown`,
    /// not `single_model: unknown`.
    #[test]
    fn only_unknown_model_facts_is_model_unknown_not_single_model() {
        let mut parsed = support::parsed();
        parsed.models.push(ModelFact {
            at: None,
            model: "unknown".to_string(),
            via: "hook".to_string(),
        });
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
        let result = V5Model.run(&cx);
        assert!(result.findings.iter().any(|f| f.code == "model_unknown"));
        assert!(!result.findings.iter().any(|f| f.code == "single_model"));
    }
}
