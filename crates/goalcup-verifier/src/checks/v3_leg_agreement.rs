//! V3: leg 1 and leg 2 agree on every human message and on issued identifiers.
//!
//! Three rules:
//!
//!   * **The "blocked" annotation matches the post's own text** against the blocked-prompt
//!     notice. Claude's single blocked-prompt notice contains the word "exceeds", so a
//!     generic `contains("exceeds")` would be true for every unpaired post.
//!   * **Pairing is one-to-one.** A leg-2 post that would pair with an already-paired leg-1
//!     record is `duplicate_pairing`, so replaying a genuine post cannot manufacture a
//!     second message.
//!   * **Identifiers.** The legs agree on request or message identifiers where issued:
//!     where both legs stamp the same tool call, the ids must match, and where they do not,
//!     the count of shared ids is reported so a reviewer can see which join the connector
//!     actually relies on.
//!
//! `human_rule::HumanRuleOutcome` reports a byte mismatch between the two legs as
//! `divergent: Vec<DivergentText>`. Each entry becomes exactly one `text_divergence` fail
//! here, citing the leg-1 store and sequence as well as the two texts.

use super::{Check, CheckContext, CheckResult, Evidence, fail, flag, pass};
use crate::human_rule::strip_redaction_salts;
use std::collections::{BTreeMap, BTreeSet};

pub struct V3LegAgreement;

impl Check for V3LegAgreement {
    fn id(&self) -> &'static str {
        "V3"
    }
    fn title(&self) -> &'static str {
        "the two legs agree on every human message"
    }
    fn run(&self, cx: &CheckContext) -> CheckResult {
        let mut findings = Vec::new();

        for d in &cx.human.divergent {
            // Both sides truncated to 80 characters, as every other message in this check
            // truncates the text it quotes. This one interpolated two participant-written
            // strings whole, so a divergence over a long paste put a kilobyte of the run's
            // own text into a published verdict — and the two texts differ, so a reader
            // needs their heads, not their length.
            findings.push(fail(
                "text_divergence",
                format!(
                    "leg 1 holds `{}` where leg 2 holds `{}` for the same key",
                    d.leg1_text.chars().take(80).collect::<String>(),
                    d.leg2_text.chars().take(80).collect::<String>()
                ),
                vec![Evidence::at(
                    &d.leg1_store,
                    d.leg1_seq,
                    None,
                    "leg-1 record whose paired leg-2 post disagrees on the bytes",
                )],
            ));
        }
        for rec in &cx.human.leg1_without_post {
            findings.push(flag(
                "leg1_without_leg2_post",
                format!(
                    "leg-1 user-slot record `{}` has no corroborating post (not counted as a human message; V2 decides its severity)",
                    rec.text.chars().take(80).collect::<String>()
                ),
                vec![Evidence::at(
                    &rec.store,
                    rec.seq,
                    None,
                    "no leg-2 post with this pair key",
                )],
            ));
        }
        for post in &cx.human.leg2_without_record {
            // Match this post's text against the blocked notices. A generic
            // `contains("exceeds")` would be true for every post as soon as any block had
            // happened.
            //
            // Both sides go through `strip_redaction_salts` before the compare (same
            // function `HumanRule::normalise` uses for the pairing above) and
            // before the 40-character head is taken — normalising first, rather than
            // truncating raw text and normalising the fragment, so a marker that
            // straddles the 40-character cut is still recognised whole. The post's text
            // and the logged blocked prompt may come from a legacy trace whose separate
            // records carried different per-record salts. Comparing those raw legacy
            // bytes would miss the match and drop the "a blocking hook refused it"
            // annotation over a difference neither harness actually wrote.
            let head: String = strip_redaction_salts(&post.text).chars().take(40).collect();
            let blocked = !head.is_empty()
                && cx
                    .parsed
                    .blocked_prompts
                    .iter()
                    .any(|b| strip_redaction_salts(b).contains(&head));
            findings.push(flag(
                "leg2_without_leg1_record",
                format!(
                    "leg-2 post {} (`{head}`) has no leg-1 record{}",
                    post.index,
                    if blocked {
                        "; a blocking hook refused it, and the harness kept its text"
                    } else {
                        ""
                    }
                ),
                Vec::new(),
            ));
        }

        // One leg-1 record, one leg-2 post. Never two.
        for (first, second) in &cx.human.duplicate_pairings {
            findings.push(fail(
                "duplicate_pairing",
                format!(
                    "leg-2 post {second} carries the same pair key as post {first}, which \
                     already paired with this leg-1 record; a replayed post is not a message"
                ),
                Vec::new(),
            ));
        }

        // The legs agree on issued identifiers, where identifiers are issued.
        let leg1: BTreeSet<&str> = cx
            .parsed
            .tool_calls
            .iter()
            .filter(|t| t.leg == "leg1")
            .map(|t| t.call_id.as_str())
            .collect();
        let leg2: BTreeSet<&str> = cx
            .parsed
            .tool_calls
            .iter()
            .filter(|t| t.leg == "leg2")
            .map(|t| t.call_id.as_str())
            .collect();
        if !leg1.is_empty() && !leg2.is_empty() {
            let shared = leg1.intersection(&leg2).count();
            // What "the legs agree on identifiers" means depends on the
            // connector's declared join. An id-based join (`prompt_id`, `message_id`)
            // stamps the *same* id on both legs, so zero overlap with tool calls present
            // on both sides means the declared key joins nothing real — a Fail. A
            // turn+order join (Codex's `turn_id+order`) never puts the same id on both
            // legs by construction — leg 1 carries the model's `call_id`, leg 2 carries
            // the hook's own id — so the only thing to check is that each turn carries
            // the same *number* of tool calls on both legs; a turn whose counts differ is
            // a Flag naming that turn.
            if cx.join_strategy == "turn_id+order" {
                let mut per_turn: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
                for t in &cx.parsed.tool_calls {
                    if let Some(turn) = t.turn_key.as_deref() {
                        let entry = per_turn.entry(turn).or_insert((0, 0));
                        if t.leg == "leg1" {
                            entry.0 += 1;
                        } else {
                            entry.1 += 1;
                        }
                    }
                }
                for (turn, (l1, l2)) in &per_turn {
                    if l1 != l2 {
                        findings.push(flag(
                            "cross_leg_identifiers_mismatch",
                            format!(
                                "turn `{turn}`: {l1} leg-1 tool call(s), {l2} leg-2 tool \
                                 call(s) — the turn+order join expects equal counts per turn"
                            ),
                            Vec::new(),
                        ));
                    }
                }
            } else if shared == 0 {
                findings.push(fail(
                    "cross_leg_identifiers_disjoint",
                    format!(
                        "{} leg-1 tool id(s), {} leg-2 tool id(s), none shared; the \
                         id-based join declared by the manifest (`{}`) joins nothing",
                        leg1.len(),
                        leg2.len(),
                        cx.join_strategy
                    ),
                    Vec::new(),
                ));
            }
            // The raw counts, kept as an informational finding regardless of the join
            // strategy or whether the checks above found anything wrong (the Pass case).
            //
            // Pass-level. It counts identifiers; the two ways they can disagree have codes
            // of their own above (`cross_leg_identifiers_mismatch`,
            // `cross_leg_identifiers_disjoint`), and a Flag here would put a reviewer's
            // queue entry on the join *working*.
            findings.push(pass(
                "cross_leg_identifiers",
                format!(
                    "{} leg-1 tool id(s), {} leg-2 tool id(s), {shared} shared; the join \
                     declared by the manifest is `{}`",
                    leg1.len(),
                    leg2.len(),
                    cx.manifest
                        .join_key
                        .as_deref()
                        .unwrap_or(cx.manifest.human_rule.pair_key.as_str())
                ),
            ));
        }
        // Every paired human message must carry the same pair key on both legs — the one
        // identifier both legs are guaranteed to stamp.
        for m in &cx.human.messages {
            if m.pair_key.is_none() {
                findings.push(flag(
                    "message_without_pair_key",
                    format!(
                        "message {} paired by position rather than by an issued identifier",
                        m.index
                    ),
                    vec![Evidence::at(
                        &m.leg1_store,
                        m.leg1_seq,
                        Some(m.leg1_sha256.clone()),
                        "no pair key on the leg-1 record",
                    )],
                ));
            }
        }

        // A tool call whose arguments two readings disagreed about.
        //
        // This check is where the legs are made to agree, and the arguments are what
        // attribution runs on — a sealed file is explained by the call whose arguments name
        // it.
        //
        // No argument value is interpolated. Arguments are the participant's own text and a
        // verdict is published, so the message carries the call id, the tool name and the
        // two byte lengths — enough to go and look, and nothing to leak.
        for c in &cx.parsed.tool_args_conflicts {
            let (code, subject) = match c.kind {
                crate::parsers::ArgsConflictKind::BetweenPosts => (
                    "tool_args_conflict",
                    "two leg-2 posts carried different arguments under one call id",
                ),
                crate::parsers::ArgsConflictKind::BetweenLegs => (
                    "tool_args_cross_leg",
                    "leg 1 holds arguments for this call that are not the ones leg 2 carried",
                ),
            };
            findings.push(flag(
                code,
                format!(
                    "{subject}: call `{}` (`{}`). The reading kept is {} byte(s) and the \
                     one dropped is {} byte(s); neither is shown, because arguments are \
                     the run's own text. The arguments are what explains a sealed file, \
                     so a call with two of them is a file whose explanation is a choice",
                    c.call_id, c.tool, c.kept_bytes, c.dropped_bytes
                ),
                vec![Evidence::note("leg2", format!("tool call `{}`", c.call_id))],
            ));
        }

        if findings.is_empty() {
            findings.push(pass(
                "legs_agree",
                format!(
                    "{} human message(s) agree on both legs",
                    cx.human.messages.len()
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
    use crate::human_rule::{DivergentText, HumanRuleOutcome};
    use crate::parsers::{Leg2Candidate, ParsedLegs, ToolFact};

    fn run(
        connector: &str,
        join_strategy: &str,
        human: HumanRuleOutcome,
        parsed: ParsedLegs,
    ) -> CheckResult {
        let trace = support::trace(connector);
        let manifest = support::manifest(connector);
        let cx = CheckContext {
            trace: &trace,
            manifest: &manifest,
            parsed: &parsed,
            human: &human,
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
        };
        V3LegAgreement.run(&cx)
    }

    #[test]
    fn a_divergent_record_is_a_text_divergence_fail() {
        let mut human = support::human();
        human.divergent.push(DivergentText {
            leg1_store: "main".to_string(),
            leg1_seq: 1,
            pair_key: Some("k1".to_string()),
            leg1_text: "keep going".to_string(),
            leg2_text: "keep going and also rm -rf /".to_string(),
        });
        let result = run("claude-code-cli", "prompt_id", human, support::parsed());
        assert_eq!(result.verdict, Severity::Fail);
        assert_eq!(result.findings[0].code, "text_divergence");
        assert!(
            result.findings[0]
                .message
                .contains("keep going and also rm -rf /")
        );
    }

    /// A divergence is two participant-written strings, and this finding truncates both to
    /// 80 characters like every other message in this check: a 500-character record
    /// interpolated whole would put a kilobyte of the run's own text into a published
    /// verdict, which is the one thing a finding message must not do.
    #[test]
    fn a_divergence_is_truncated_to_eighty_characters_on_both_sides() {
        let mut human = support::human();
        human.divergent.push(DivergentText {
            leg1_store: "main".to_string(),
            leg1_seq: 1,
            pair_key: Some("k1".to_string()),
            leg1_text: "a".repeat(500),
            leg2_text: "b".repeat(500),
        });
        let result = run("claude-code-cli", "prompt_id", human, support::parsed());
        let m = &result.findings[0].message;
        assert!(m.contains(&"a".repeat(80)), "{m}");
        assert!(m.contains(&"b".repeat(80)), "{m}");
        assert!(!m.contains(&"a".repeat(81)), "leg 1 was not truncated: {m}");
        assert!(!m.contains(&"b".repeat(81)), "leg 2 was not truncated: {m}");
    }

    #[test]
    fn a_replayed_post_is_a_duplicate_pairing_fail() {
        let mut human = support::human();
        human.duplicate_pairings.push((0, 7));
        let result = run("claude-code-cli", "prompt_id", human, support::parsed());
        assert_eq!(result.verdict, Severity::Fail);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == "duplicate_pairing")
        );
    }

    #[test]
    fn nothing_wrong_is_legs_agree() {
        let mut human = support::human();
        human.messages.push(support::message(0, "goal", Some(0)));
        let result = run("claude-code-cli", "prompt_id", human, support::parsed());
        assert_eq!(result.verdict, Severity::Pass);
        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.findings[0].code, "legs_agree");
    }

    /// The blocked-prompt compare must still read legacy traces where each leg
    /// redacted the same secret under a different per-record salt. Here the post's text
    /// and the logged blocked prompt both carry a redacted email in the first 40
    /// characters, but under different digests. Without normalising
    /// both sides first, `contains` never matches and the "a blocking hook refused it"
    /// annotation is silently dropped.
    #[test]
    fn a_blocked_prompt_matches_across_differently_salted_redactions() {
        let post_marker = format!("<redacted:email:{}>", "a".repeat(16));
        let logged_marker = format!("<redacted:email:{}>", "b".repeat(16));
        assert_ne!(
            post_marker, logged_marker,
            "different salts, same secret kind"
        );

        let mut parsed = support::parsed();
        parsed.blocked_prompts.push(format!(
            "{logged_marker} please help me urgently right now, this prompt exceeds 4000 characters"
        ));

        let mut human = support::human();
        human.leg2_without_record.push(Leg2Candidate {
            index: 0,
            received_at: 0.0,
            event: "post".to_string(),
            text: format!("{post_marker} please help me urgently right now"),
            pair_key: None,
            session_id: None,
            directory: None,
        });

        let result = run("claude-code-cli", "prompt_id", human, parsed);
        assert_eq!(result.verdict, Severity::Flag);
        let finding = result
            .findings
            .iter()
            .find(|f| f.code == "leg2_without_leg1_record")
            .expect("the unpaired post is flagged");
        assert!(
            finding
                .message
                .contains("a blocking hook refused it, and the harness kept its text"),
            "differently-salted redactions of the same secret must still match: {}",
            finding.message
        );
    }

    #[test]
    fn a_v2_kind_only_marker_pairs_byte_exactly() {
        let mut human = support::human();
        human
            .messages
            .push(support::message(0, "mail <redacted:email> please", Some(0)));
        let result = run("claude-code-cli", "prompt_id", human, support::parsed());
        assert_eq!(result.verdict, Severity::Pass);
        assert_eq!(result.findings[0].code, "legs_agree");
    }

    /// An id-based join (`prompt_id`, `message_id`) with tool calls on
    /// both legs and zero overlap means the declared key joins nothing real.
    #[test]
    fn disjoint_ids_under_an_id_based_join_is_a_fail() {
        let mut parsed = support::parsed();
        parsed.tool_calls.push(ToolFact {
            call_id: "leg1-only".to_string(),
            name: "Read".to_string(),
            leg: "leg1",
            turn_key: None,
            order: 0,
        });
        parsed.tool_calls.push(ToolFact {
            call_id: "leg2-only".to_string(),
            name: "Read".to_string(),
            leg: "leg2",
            turn_key: None,
            order: 0,
        });
        let result = run("claude-code-cli", "prompt_id", support::human(), parsed);
        assert_eq!(result.verdict, Severity::Fail);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == "cross_leg_identifiers_disjoint")
        );
    }
}
