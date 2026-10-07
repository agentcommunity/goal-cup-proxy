//! The two-legged human-message rule (DECISIONS D18).
//!
//! A message counts as human only when a leg-2 post pairs with a leg-1 record that
//! passes the connector's positive test and satisfies the scoping clauses. A leg-1
//! user-slot record with no corroborating post is a flag, not a human message.

use crate::parsers::{FieldValue, Leg2Candidate, SessionFact, UserSlotRecord};
use goalcup_protocol::manifest::{HumanRuleSpec, MatchRule, Normalisation, ScopingClause};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanMessage {
    /// Position in the ordered human list; 0 is the goal. The counter runs across both
    /// `messages` and `unwitnessed` in leg-1 order, so "message 2" means the second thing
    /// a human put in the user slot, whether or not leg 2 saw it.
    pub index: usize,
    pub text: String,
    pub pair_key: Option<String>,
    pub leg1_store: String,
    pub leg1_seq: u64,
    pub leg1_sha256: String,
    /// `None` for an unwitnessed message: leg 1 recorded it and leg 2 did not.
    pub leg2_index: Option<usize>,
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExclusionReason {
    /// Failed the connector's positive test.
    FailedPositive,
    /// Failed these D18 scoping clauses — **all** of them, in manifest order, not just
    /// the first. A reviewer asking "which rule caught the subagent?"
    /// should get the whole answer, and reporting only the first made two assertions in
    /// this plan name buckets no record ever reached.
    FailedScoping(Vec<ScopingClause>),
    /// Passed every test but no leg-2 post corroborates it. Kept as a variant for the
    /// evidence trail; the record itself also becomes an `unwitnessed` message so V2 can
    /// judge its text.
    NoCorroboratingPost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcludedRecord {
    pub text: String,
    pub store: String,
    pub seq: u64,
    /// The key the record would have paired on. Carried so a downstream check can join an
    /// excluded record to the orphaned post in `leg2_without_record` — the
    /// `<task-notification>` pair on Claude Code — instead of leaving a reviewer to match
    /// them by eye on the text.
    pub pair_key: Option<String>,
    pub reason: ExclusionReason,
}

/// One submission whose two legs disagree about the bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DivergentText {
    pub leg1_store: String,
    pub leg1_seq: u64,
    pub pair_key: Option<String>,
    pub leg1_text: String,
    pub leg2_text: String,
}

// Not `Eq`: `leg2_without_record` carries `Leg2Candidate`, whose `received_at` is an `f64`.
#[derive(Debug, Clone, PartialEq)]
pub struct HumanRuleOutcome {
    pub messages: Vec<HumanMessage>,
    pub excluded: Vec<ExcludedRecord>,
    /// Leg-1 records that pass every test but carry no corroborating post.
    ///
    /// These are **not** dropped from the rule. V2 evaluates `keep going` over `messages ∪
    /// unwitnessed` and reports these separately at `Severity::Fail` when their text is not
    /// `keep going`. Leaving them out would let an instruction typed inside the hot-reload
    /// window, before the config watch restores the hook, classify itself out of the check
    /// it violates.
    pub unwitnessed: Vec<HumanMessage>,
    /// The same records, in the `ExcludedRecord` shape, for the evidence trail.
    pub leg1_without_post: Vec<ExcludedRecord>,
    /// Posts with no leg-1 record: a blocked prompt, or a capture gap.
    pub leg2_without_record: Vec<Leg2Candidate>,
    /// (leg-2 post index, leg-2 post index) — a post that would have paired with a leg-1
    /// record another post already claimed. V3 raises `duplicate_pairing`.
    pub duplicate_pairings: Vec<(usize, usize)>,
    /// Records whose legs share a key but differ after normalisation. A divergent record
    /// is **not** counted human: a byte mismatch means one of the two legs recorded
    /// something other than what was submitted, and picking a winner would be a
    /// judgement with no evidence behind it. It stays out of `leg1_without_post` too —
    /// a post did exist, it just did not say the same thing.
    pub divergent: Vec<DivergentText>,
}

pub struct HumanRule {
    pub spec: HumanRuleSpec,
    pub normalisation: Normalisation,
    pub workspace: String,
    pub tracked_sessions: BTreeSet<String>,
}

/// Evaluate one manifest predicate against a user-slot record.
pub fn matches(rule: &MatchRule, rec: &UserSlotRecord) -> bool {
    match rule {
        MatchRule::FieldEquals { field, value } => {
            matches!(rec.field(field), Some(FieldValue::Str(s)) if s == value)
        }
        // Present means the key exists and its value is not null.
        MatchRule::FieldPresent { field } => {
            matches!(
                rec.field(field),
                Some(FieldValue::Str(_)) | Some(FieldValue::Bool(_))
            )
        }
        MatchRule::FieldTrue { field } => {
            matches!(rec.field(field), Some(FieldValue::Bool(true)))
        }
        MatchRule::FieldAbsentOrNull { field } => {
            matches!(rec.field(field), None | Some(FieldValue::Null))
        }
    }
}

/// How a legacy redaction marker opens. V1 wrote `<redacted:KIND:DIGEST>`, bare or
/// wrapped in the quotes that kept a redacted JSON field a JSON field.
const MARKER_OPEN: &str = "<redacted:";

/// The length of the digest a legacy marker carried.
const MARKER_DIGEST_LEN: usize = 16;

/// Legacy `<redacted:KIND:DIGEST>` → v2 `<redacted:KIND>`, everywhere it appears
/// See [`HumanRule::normalise`] for why old per-record digests must go before a
/// cross-leg compare. A v2 marker is already in canonical form and passes through.
///
/// **Conservative by construction.** The text this runs over is the participant's own, and
/// nothing stops them typing something marker-shaped into the user slot. So a run is
/// rewritten only when it is exactly what the redactor writes: the literal opener, a
/// non-empty kind, and a tail of exactly 16 lowercase hex characters before the `>`.
/// Anything else — an unclosed opener, a kind with no digest, an upper-case or short tail —
/// is copied through byte for byte, which leaves it comparing exactly as it did before.
pub(crate) fn strip_redaction_salts(s: &str) -> String {
    if !s.contains(MARKER_OPEN) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find(MARKER_OPEN) {
        let (before, from) = rest.split_at(at);
        out.push_str(before);
        // Everything after the opener is ASCII up to the `>` the redactor closes with, so
        // these byte offsets are char boundaries.
        let body_start = MARKER_OPEN.len();
        let Some(close) = from[body_start..].find('>') else {
            // An opener nothing closes is not a marker. The remainder is text.
            out.push_str(from);
            return out;
        };
        let end = body_start + close + 1;
        match from[body_start..body_start + close].rsplit_once(':') {
            Some((kind, digest)) if !kind.is_empty() && is_marker_digest(digest) => {
                out.push_str(MARKER_OPEN);
                out.push_str(kind);
                out.push('>');
            }
            _ => out.push_str(&from[..end]),
        }
        rest = &from[end..];
    }
    out.push_str(rest);
    out
}

/// Exactly the shape the legacy v1 digest published: 16 lower-case hex characters.
fn is_marker_digest(s: &str) -> bool {
    s.len() == MARKER_DIGEST_LEN && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl HumanRule {
    pub fn new(
        spec: HumanRuleSpec,
        normalisation: Normalisation,
        workspace: &str,
        tracked: &[String],
    ) -> HumanRule {
        HumanRule {
            spec,
            normalisation,
            workspace: workspace.to_string(),
            tracked_sessions: tracked.iter().cloned().collect(),
        }
    }

    /// Strip exactly one trailing newline (V7), and the per-record digest off any legacy
    /// redaction marker. Nothing else. V2 kind-only markers pass through exactly.
    ///
    /// **Why the legacy digest has to go.** The v1 tracker redacted each record under its
    /// own fresh salt — the salt was per record because the log published each span's exact
    /// offsets and an unsalted digest of a low-entropy secret next to its exact length is a
    /// confirmation oracle. Both legs of one submission are separate records, so one prompt
    /// containing an email address reaches leg 1 as `<redacted:email:A>` and leg 2 as
    /// `<redacted:email:B>`. The compare below is byte-exact after this call; without it the
    /// two legs would disagree and the message would stop being a human message: a goal
    /// prompt with an address in it would silently cease to be the goal, reported as
    /// `DivergentText` over a difference neither harness wrote.
    ///
    /// Legacy per-record salting and byte-exact cross-record pairing cannot both hold on
    /// the raw text, so the pairing is done on the text with the legacy digest normalised
    /// out. This compatibility transform does not rewrite canonical v2 markers.
    ///
    /// **What it costs, stated.** Two different secrets of the same kind at the same
    /// position compare equal, so a leg-2 post could substitute one secret for another of
    /// the same kind and the legs would still pair. The kind stays in the marker and the
    /// surrounding bytes still have to match exactly, so what is given up is the identity of
    /// a span whose text neither leg publishes in the first place.
    pub fn normalise(&self, s: &str) -> String {
        let s = strip_redaction_salts(s);
        if self.normalisation.trailing_newline
            && let Some(stripped) = s.strip_suffix('\n')
        {
            return stripped.to_string();
        }
        s
    }

    /// Evaluate **every** scoping clause and return all the ones that failed.
    /// An empty vector means the record is in scope.
    fn scoping_failures(
        &self,
        rec: &UserSlotRecord,
        sessions: &[SessionFact],
    ) -> Vec<ScopingClause> {
        let mut failed = Vec::new();
        let session = sessions.iter().find(|s| s.id == rec.session_id);
        for clause in &self.spec.scoping {
            let ok = match clause {
                ScopingClause::ParentIdIsNull => match rec.field("session.parent_id") {
                    // Any parent id fails, including the parser's fail-closed "unknown".
                    Some(FieldValue::Str(_)) => false,
                    // Fail closed when the record's session does not resolve to a
                    // `SessionFact`: an unresolvable session has not been shown to be a
                    // root, and defaulting it to one is how a capture gap comes to read as
                    // human authorship.
                    _ => session.map(|s| s.parent.is_none()).unwrap_or(false),
                },
                ScopingClause::DelegateFromIsNull => {
                    !matches!(rec.field("_delegate_from"), Some(FieldValue::Str(_)))
                }
                ScopingClause::NotSubagentThreadSource => {
                    // Only a known root value passes. The accepted set is exactly {"user"}
                    // — the one value both parsers emit for a root thread. "subagent" is
                    // the subagent; "unknown" is the Codex parser's fail-closed default for
                    // a rollout whose `session_meta` carries no `thread_source`, and it
                    // must fail here too, or an unmeasured gap passes as human authorship.
                    // A record whose session does not resolve fails for the same reason.
                    let record_ok = match rec.field("thread_source") {
                        Some(FieldValue::Str(s)) => s == "user",
                        // The connector does not stamp the field on its records; the
                        // session is then the only evidence there is.
                        None => true,
                        Some(_) => false,
                    };
                    record_ok && session.map(|s| s.thread_source == "user").unwrap_or(false)
                }
                ScopingClause::PastSubagentHistoryStartOrdinal => {
                    // Only meaningful where a subagent replays history in the same file.
                    // On the shipped connectors the replay is a separate file, caught by
                    // NotSubagentThreadSource, so this clause is vacuously satisfied.
                    true
                }
                ScopingClause::DirectoryEqualsWorkspace => rec
                    .directory
                    .as_deref()
                    .map(|d| d == self.workspace)
                    .unwrap_or(false),
                ScopingClause::FirstSessionMetaIsTracked => {
                    self.tracked_sessions.contains(&rec.session_id)
                }
            };
            if !ok {
                failed.push(*clause);
            }
        }
        failed
    }

    pub fn apply(
        &self,
        leg1: &[UserSlotRecord],
        leg2: &[Leg2Candidate],
        sessions: &[SessionFact],
    ) -> HumanRuleOutcome {
        let mut messages = Vec::new();
        let mut excluded = Vec::new();
        let mut unwitnessed = Vec::new();
        let mut leg1_without_post = Vec::new();
        let mut duplicate_pairings = Vec::new();
        let mut divergent = Vec::new();
        let mut used_posts: BTreeSet<usize> = BTreeSet::new();
        // One counter across `messages` and `unwitnessed`, in leg-1 order.
        let mut next_index = 0usize;

        for rec in leg1 {
            // 1. positive test: every rule must hold (an empty list is vacuously true).
            if !self.spec.positive.iter().all(|r| matches(r, rec)) {
                excluded.push(ExcludedRecord {
                    text: rec.text.clone(),
                    store: rec.store.clone(),
                    seq: rec.seq,
                    pair_key: rec.pair_key.clone(),
                    reason: ExclusionReason::FailedPositive,
                });
                continue;
            }
            // 2. scoping clauses — all of them, so the reason is the whole reason.
            let failed = self.scoping_failures(rec, sessions);
            if !failed.is_empty() {
                excluded.push(ExcludedRecord {
                    text: rec.text.clone(),
                    store: rec.store.clone(),
                    seq: rec.seq,
                    pair_key: rec.pair_key.clone(),
                    reason: ExclusionReason::FailedScoping(failed),
                });
                continue;
            }
            // 3. pair with a leg-2 post on the connector's key, byte-equal after normalisation.
            let want = self.normalise(&rec.text);
            let post = leg2.iter().find(|c| {
                !used_posts.contains(&c.index) && c.pair_key.is_some() && c.pair_key == rec.pair_key
            });
            let index = next_index;
            next_index += 1;
            match post {
                Some(p) => {
                    used_posts.insert(p.index);
                    // Any *further* post carrying the key this record just paired
                    // on is a replay, not a message. It is recorded as a duplicate
                    // pairing (V3 raises `duplicate_pairing`) and consumed, so it can
                    // neither pair with a later record nor masquerade as a post with no
                    // leg-1 record.
                    let replays: Vec<usize> = leg2
                        .iter()
                        .filter(|c| {
                            c.index != p.index
                                && !used_posts.contains(&c.index)
                                && c.pair_key.is_some()
                                && c.pair_key == rec.pair_key
                        })
                        .map(|c| c.index)
                        .collect();
                    for dup in replays {
                        duplicate_pairings.push((p.index, dup));
                        used_posts.insert(dup);
                    }
                    // The legs must agree byte for byte after normalisation. Where they
                    // do not, this is not a human message: one of the two legs recorded
                    // something other than the submission, and counting either text would
                    // pick a winner on no evidence. The index is still spent, so the
                    // messages after it stay anchored to leg-1 order and the gap is itself
                    // visible.
                    let got = self.normalise(&p.text);
                    if got != want {
                        divergent.push(DivergentText {
                            leg1_store: rec.store.clone(),
                            leg1_seq: rec.seq,
                            pair_key: rec.pair_key.clone(),
                            leg1_text: want,
                            leg2_text: got,
                        });
                        continue;
                    }
                    messages.push(HumanMessage {
                        index,
                        text: want,
                        pair_key: rec.pair_key.clone(),
                        leg1_store: rec.store.clone(),
                        leg1_seq: rec.seq,
                        leg1_sha256: rec.raw_sha256.clone(),
                        leg2_index: Some(p.index),
                        timestamp: rec.timestamp.clone(),
                    });
                }
                None => {
                    // A record leg 2 never saw is still something a human typed into
                    // the user slot. It is reported separately and judged by V2, rather
                    // than classified out of the check it might violate.
                    leg1_without_post.push(ExcludedRecord {
                        text: rec.text.clone(),
                        store: rec.store.clone(),
                        seq: rec.seq,
                        pair_key: rec.pair_key.clone(),
                        reason: ExclusionReason::NoCorroboratingPost,
                    });
                    unwitnessed.push(HumanMessage {
                        index,
                        text: want,
                        pair_key: rec.pair_key.clone(),
                        leg1_store: rec.store.clone(),
                        leg1_seq: rec.seq,
                        leg1_sha256: rec.raw_sha256.clone(),
                        leg2_index: None,
                        timestamp: rec.timestamp.clone(),
                    });
                }
            }
        }

        let leg2_without_record = leg2
            .iter()
            .filter(|c| !used_posts.contains(&c.index))
            .cloned()
            .collect();

        HumanRuleOutcome {
            messages,
            excluded,
            unwitnessed,
            leg1_without_post,
            leg2_without_record,
            duplicate_pairings,
            divergent,
        }
    }
}

#[cfg(test)]
mod tests {
    //! The three M0 fixtures leave `unwitnessed` and `duplicate_pairings` empty — which is
    //! the correct measurement, and also means neither adversarial branch is exercised by
    //! them. These construct the two situations directly, so a regression that made either
    //! defence dead would fail here instead of passing three green fixtures.

    use super::*;
    use crate::parsers::{Leg2Candidate, SessionFact, UserSlotRecord};
    use goalcup_protocol::manifest::HumanRuleSpec;
    use std::collections::BTreeMap;

    fn spec() -> HumanRuleSpec {
        HumanRuleSpec {
            id: "test".to_string(),
            positive: Vec::new(),
            scoping: Vec::new(),
            byte_exact_field: "prompt".to_string(),
            leg2_event: "UserPromptSubmit".to_string(),
            pair_key: "prompt_id".to_string(),
            legacy_exclusions: None,
            legacy_reportable_exclusions: None,
        }
    }

    fn rule() -> HumanRule {
        HumanRule::new(
            spec(),
            Normalisation {
                trailing_newline: true,
            },
            "/ws",
            &["s1".to_string()],
        )
    }

    fn rec(seq: u64, text: &str, key: &str) -> UserSlotRecord {
        UserSlotRecord {
            store: "main".to_string(),
            seq,
            ordinal: Some(seq),
            timestamp: None,
            text: text.to_string(),
            pair_key: Some(key.to_string()),
            session_id: "s1".to_string(),
            directory: Some("/ws".to_string()),
            fields: BTreeMap::new(),
            raw_sha256: "0".repeat(64),
        }
    }

    fn post(index: usize, text: &str, key: &str) -> Leg2Candidate {
        Leg2Candidate {
            index,
            received_at: index as f64,
            event: "UserPromptSubmit".to_string(),
            text: text.to_string(),
            pair_key: Some(key.to_string()),
            session_id: Some("s1".to_string()),
            directory: Some("/ws".to_string()),
        }
    }

    /// A record leg 2 never saw is a human message with `leg2_index: None`, and the
    /// index counter keeps running across both lists, so "message 2" stays "message 2".
    #[test]
    fn a_record_with_no_post_becomes_an_unwitnessed_message() {
        let o = rule().apply(
            &[
                rec(1, "goal", "k1"),
                rec(2, "typed in the hot-reload window", "k2"),
                rec(3, "keep going", "k3"),
            ],
            &[post(0, "goal", "k1"), post(1, "keep going", "k3")],
            &[],
        );
        assert_eq!(o.messages.len(), 2);
        assert_eq!(o.unwitnessed.len(), 1);
        assert_eq!(
            o.unwitnessed[0].index, 1,
            "the counter runs across both lists"
        );
        assert_eq!(o.unwitnessed[0].leg2_index, None);
        assert_eq!(o.messages[1].index, 2);
        assert_eq!(o.leg1_without_post.len(), 1);
        assert!(matches!(
            o.leg1_without_post[0].reason,
            ExclusionReason::NoCorroboratingPost
        ));
    }

    /// Replaying a genuine post must not pair twice. The replay is recorded and
    /// consumed — never counted as a message, never reported as a post with no record.
    #[test]
    fn a_replayed_post_is_a_duplicate_pairing_not_a_second_message() {
        let o = rule().apply(
            &[rec(1, "keep going", "k1")],
            &[post(0, "keep going", "k1"), post(7, "keep going", "k1")],
            &[],
        );
        assert_eq!(o.messages.len(), 1);
        assert_eq!(o.messages[0].leg2_index, Some(0));
        assert_eq!(o.duplicate_pairings, vec![(0, 7)]);
        assert!(o.leg2_without_record.is_empty());
    }

    /// A post whose text drifted from the record it pairs with is
    /// reported and not counted human. The pair key is the join, the bytes are the
    /// evidence (V7), and where the bytes disagree there is no evidence for either text.
    #[test]
    fn a_divergent_pair_is_reported_and_not_counted_human() {
        let o = rule().apply(
            &[
                rec(1, "keep going", "k1"),
                rec(2, "also add a README", "k2"),
            ],
            &[
                post(0, "keep going and also rm -rf /", "k1"),
                post(1, "also add a README", "k2"),
            ],
            &[],
        );
        assert_eq!(o.messages.len(), 1, "the divergent one is not a message");
        assert_eq!(o.messages[0].text, "also add a README");
        assert_eq!(
            o.messages[0].index, 1,
            "the divergent record still spent index 0"
        );
        assert_eq!(
            o.divergent,
            vec![DivergentText {
                leg1_store: "main".to_string(),
                leg1_seq: 1,
                pair_key: Some("k1".to_string()),
                leg1_text: "keep going".to_string(),
                leg2_text: "keep going and also rm -rf /".to_string(),
            }]
        );
        assert!(
            o.leg1_without_post.is_empty(),
            "a post existed; it just did not say the same thing"
        );
        assert!(
            o.leg2_without_record.is_empty(),
            "the divergent post is accounted for, not orphaned"
        );
    }

    /// The scoping clauses fail closed. A record whose session does not resolve, and a
    /// session whose `thread_source` is the parsers' fail-closed "unknown", are both out of
    /// scope — neither has been shown to be a root.
    #[test]
    fn scoping_fails_closed_on_unresolvable_and_unknown() {
        let mut spec = spec();
        spec.scoping = vec![
            ScopingClause::ParentIdIsNull,
            ScopingClause::NotSubagentThreadSource,
        ];
        let rule = HumanRule::new(
            spec,
            Normalisation {
                trailing_newline: true,
            },
            "/ws",
            &["s1".to_string()],
        );

        // No SessionFact at all: both clauses fail rather than defaulting to root.
        let o = rule.apply(
            &[rec(1, "keep going", "k1")],
            &[post(0, "keep going", "k1")],
            &[],
        );
        assert!(o.messages.is_empty());
        assert_eq!(
            o.excluded[0].reason,
            ExclusionReason::FailedScoping(vec![
                ScopingClause::ParentIdIsNull,
                ScopingClause::NotSubagentThreadSource,
            ])
        );
        assert_eq!(
            o.excluded[0].pair_key.as_deref(),
            Some("k1"),
            "finding 5: the excluded record carries the key its orphaned post also carries"
        );
        assert_eq!(o.leg2_without_record.len(), 1);

        // A resolvable root session whose thread_source could not be measured.
        let unknown = SessionFact {
            id: "s1".to_string(),
            store: "main".to_string(),
            directory: Some("/ws".to_string()),
            parent: None,
            thread_source: "unknown".to_string(),
            is_root: false,
            tracked: true,
        };
        let o = rule.apply(
            &[rec(1, "keep going", "k1")],
            &[post(0, "keep going", "k1")],
            std::slice::from_ref(&unknown),
        );
        assert!(
            o.messages.is_empty(),
            "\"unknown\" is not a known root value"
        );
        assert_eq!(
            o.excluded[0].reason,
            ExclusionReason::FailedScoping(vec![ScopingClause::NotSubagentThreadSource])
        );

        // The same session, measured as a root, passes both clauses.
        let root = SessionFact {
            thread_source: "user".to_string(),
            is_root: true,
            ..unknown
        };
        let o = rule.apply(
            &[rec(1, "keep going", "k1")],
            &[post(0, "keep going", "k1")],
            &[root],
        );
        assert_eq!(o.messages.len(), 1);
    }

    /// The v1 daemon redacted each record under its own salt, so one prompt carrying an
    /// email address becomes `<redacted:email:A>` on leg 1 and `<redacted:email:B>` on leg
    /// 2. The cross-leg compare is byte-exact after `normalise`, so without it the goal
    /// would stop being the goal — reported as `DivergentText`, with the two legs' texts
    /// differing only in a digest neither harness wrote.
    #[test]
    fn a_marker_pairs_across_the_legs_although_each_record_carried_its_own_salt() {
        let r = rule();
        let leg1 = "mail me at <redacted:email:0123456789abcdef> please";
        let leg2 = "mail me at <redacted:email:fedcba9876543210> please";
        assert_eq!(r.normalise(leg1), r.normalise(leg2));
        assert_eq!(
            r.normalise(leg1),
            "mail me at <redacted:email> please",
            "the salt id goes, the kind and the marker stay"
        );

        // End to end, which is the failure the finding names: the goal is a human message
        // again, and nothing is divergent.
        let o = r.apply(&[rec(1, leg1, "k1")], &[post(0, leg2, "k1")], &[]);
        assert_eq!(o.divergent, Vec::new());
        assert_eq!(o.messages.len(), 1);
        assert_eq!(o.messages[0].index, 0, "still the goal");
        assert_eq!(o.messages[0].text, "mail me at <redacted:email> please");
    }

    #[test]
    fn identical_v2_kind_only_markers_pair_without_rewriting_literal_text() {
        let r = rule();
        let text = "mail me at <redacted:email> please";
        assert_eq!(r.normalise(text), text);
        let o = r.apply(&[rec(1, text, "k1")], &[post(0, text, "k1")], &[]);
        assert_eq!(o.divergent, Vec::new());
        assert_eq!(o.messages.len(), 1);
        assert_eq!(o.messages[0].text, text);
    }

    /// Two different kinds are still two different texts, and so is a marker against the
    /// text it replaced: normalisation drops the salt id and nothing else.
    #[test]
    fn normalising_a_marker_drops_the_salt_id_and_leaves_every_other_difference() {
        let r = rule();
        assert_ne!(
            r.normalise("<redacted:email:0123456789abcdef>"),
            r.normalise("<redacted:openai_key:0123456789abcdef>")
        );
        assert_ne!(
            r.normalise("<redacted:email:0123456789abcdef>"),
            r.normalise("someone@example.com")
        );
        // A quoted marker keeps its quotes: the redactor adds them to keep a JSON field a
        // JSON field, and they are part of the bytes both legs carry.
        assert_eq!(
            r.normalise("\"<redacted:email:0123456789abcdef>\""),
            "\"<redacted:email>\""
        );
        // Anything that is not a marker is left exactly as the participant typed it — the
        // tail must be a 16-character lowercase-hex digest, or the whole run is text.
        for text in [
            "<redacted:email:not-a-digest>",
            "<redacted:email:0123456789ABCDEF>",
            "<redacted:email:0123456789abcde>",
            "<redacted:email>",
            "<redacted>",
            "a < b and c > d",
            "<redacted:email:0123456789abcdef",
        ] {
            assert_eq!(r.normalise(text), text, "left alone: {text}");
        }
        // The trailing-newline rule still runs, and still exactly once.
        assert_eq!(
            r.normalise("<redacted:email:0123456789abcdef>\n\n"),
            "<redacted:email>\n"
        );
    }
}
