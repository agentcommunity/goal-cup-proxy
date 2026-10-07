//! Codex CLI 0.153.0, launch mode. Facts from spikes/codex-cli/REPORT.md.
//!
//! Two structural rules dominate this parser:
//!   1. a rollout file is identified by its first `session_meta` only (ordinal 0);
//!      the subagent file carries the parent's `session_meta` verbatim at ordinal 1;
//!   2. the legs join by `turn_id` plus order — the nearest preceding `turn_context`
//!      of a `role=user` record carries the turn id the hook post also carries.

use super::{
    Census, ConfigFact, FieldValue, Leg2Candidate, ModelFact, ParsedLegs, Parser, SessionFact,
    TokenFact, ToolFact, UserSlotRecord, put_str,
};
use crate::VerifierError;
use crate::atif::{
    Agent, Extra, GoalcupExtra, Message, Provenance, RawLink, Step, StepSource, ToolCall,
    Trajectory,
};
use crate::legs::{Leg1Store, Leg2Stream, dotted};
use crate::trace::TraceDescriptor;
use goalcup_protocol::manifest::ConnectorManifest;
use std::collections::{BTreeMap, BTreeSet};

pub struct CodexParser;

/// The fixture-scrub marker `make_fixtures.py` leaves in place of a long prompt's
/// body: `...<truncated:<N> chars total>`. `history.jsonl`'s committed copy of the
/// 4,100-character paste carries this marker rather than its own bytes (fixtures
/// README).
///
/// **Descriptive only — never the block decision.** `text` is human input on a live
/// capture, so a length- or marker-based decision is attacker-controllable: a
/// 5,000-character prompt padded with a fabricated `<truncated:1 chars total>` suffix
/// would defeat a length comparison and hide a real block. The decision in `parse` is an
/// absence check instead — a tracked `history.jsonl` record whose text matches no
/// `role=user` text anywhere in the rollout is the one the hook refused. This helper only
/// makes the resulting diagnostic message readable when the record happens to carry the
/// marker; the caller falls back to the record's own `chars().count()` for an unredacted
/// capture, used the same way — as detail, never as the trigger.
fn truncated_marker_chars(text: &str) -> Option<usize> {
    let marker = "<truncated:";
    let start = text.rfind(marker)? + marker.len();
    let rest = &text[start..];
    let end = rest.find(' ')?;
    rest[..end].parse::<usize>().ok()
}

impl Parser for CodexParser {
    fn connector_id(&self) -> &'static str {
        "codex-cli"
    }

    fn join_strategy(&self) -> &'static str {
        "turn_id+order"
    }

    fn parse(
        &self,
        trace: &TraceDescriptor,
        manifest: &ConnectorManifest,
        leg1: &[Leg1Store],
        leg2: &Leg2Stream,
    ) -> Result<ParsedLegs, VerifierError> {
        let raw = trace.load_raw()?;
        self.parse_with_raw(trace, manifest, leg1, leg2, &raw)
    }

    fn parse_with_raw(
        &self,
        trace: &TraceDescriptor,
        manifest: &ConnectorManifest,
        leg1: &[Leg1Store],
        leg2: &Leg2Stream,
        raw: &[crate::trace::RawBody],
    ) -> Result<ParsedLegs, VerifierError> {
        let tracked: BTreeSet<&str> = trace.tracked_sessions.iter().map(|s| s.as_str()).collect();
        let mut user_slots = Vec::new();
        let mut sessions = Vec::new();
        let mut models = Vec::new();
        let mut tokens = Vec::new();
        let mut tool_calls = Vec::new();
        let config_events: Vec<ConfigFact> = Vec::new();
        let mut blocked_prompts: Vec<String> = Vec::new();
        // `history.jsonl` texts for the tracked session, buffered so the block decision
        // (below, after the loop) can compare against the full rollout, not just
        // whatever of it happened to be parsed before `history` was reached in
        // `trace.toml`'s `[[leg1]]` order.
        let mut tracked_history_texts: Vec<String> = Vec::new();
        // Which record kinds the dispatch below has no branch for, counted as it goes.
        let mut census = Census::new(trace);

        let mut trajectory = Trajectory::new(Agent {
            name: "codex-cli".to_string(),
            version: trace.harness_version.clone(),
            model_name: None,
            tool_definitions: Vec::new(),
            extra: None,
        });
        trajectory.session_id = trace.tracked_sessions.first().cloned();

        for store in leg1 {
            // `history.jsonl` is a flat side store, not a rollout: `{session_id, ts, text}`
            // per line. It is **read, not skipped**. The capture measured it as the only
            // place the hook-blocked 4,100-character prompt survives for the tracked
            // thread, and D2 names it in those words; skipping it would leave
            // `blocked_prompts` empty and V3 unable to say a block ever happened on Codex.
            // Its records are not turns: they never enter `user_slots` or the trajectory.
            // Which of the tracked session's history texts are blocked is decided after
            // this loop, once the whole rollout has been read (see the absence check below
            // `for store in leg1`).
            if store.name == "history" {
                for rec in &store.records {
                    let v = &rec.value;
                    let sid = dotted(v, "session_id")
                        .and_then(|s| s.as_str())
                        .unwrap_or("");
                    if !tracked.contains(sid) {
                        continue;
                    }
                    let text = dotted(v, "text").and_then(|s| s.as_str()).unwrap_or("");
                    tracked_history_texts.push(text.to_string());
                }
                continue;
            }
            let first = match store.records.first() {
                Some(r) => r,
                None => continue,
            };
            // Rule 1's guard. A rollout file with no `session_meta` has no partition key,
            // so it is discarded and reported as an unattributable store — it must never be
            // silently attributed to the tracked thread.
            if dotted(&first.value, "type").and_then(|t| t.as_str()) != Some("session_meta") {
                continue;
            }
            // Rule 1: the first session_meta identifies the file.
            let meta = dotted(&first.value, "payload")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            let file_session = meta
                .get("id")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            // A missing `thread_source` must not default to "user": that would silently
            // grant root status below and would pass the manifest's
            // `not_subagent_thread_source` scoping, fail-open on the exact field that
            // decides human authorship. Default to "unknown" instead — neither root nor
            // subagent — so a check downstream can report the gap rather than treat it as
            // human authorship by default. Every committed fixture carries the field, so
            // this branch is unexercised by the fixture tests.
            let thread_source = meta
                .get("thread_source")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown")
                .to_string();
            let parent = meta
                .get("parent_thread_id")
                .and_then(|s| s.as_str())
                .map(String::from);
            let directory = meta.get("cwd").and_then(|s| s.as_str()).map(String::from);
            sessions.push(SessionFact {
                id: file_session.clone(),
                store: store.name.clone(),
                directory: directory.clone(),
                parent: parent.clone(),
                thread_source: thread_source.clone(),
                is_root: thread_source == "user",
                tracked: tracked.contains(file_session.as_str()),
            });

            let is_tracked_file = tracked.contains(file_session.as_str());
            let mut current_turn: Option<String> = None;
            let mut sub = Trajectory::new(Agent {
                name: "codex-cli".to_string(),
                version: trace.harness_version.clone(),
                model_name: None,
                tool_definitions: Vec::new(),
                extra: None,
            });
            sub.trajectory_id = Some(file_session.clone());

            for (i, rec) in store.records.iter().enumerate() {
                let v = &rec.value;
                let ordinal = dotted(v, "ordinal").and_then(|o| o.as_u64());
                let ts = dotted(v, "timestamp")
                    .and_then(|s| s.as_str())
                    .map(String::from);
                match dotted(v, "type").and_then(|t| t.as_str()) {
                    // Rule 2: remember the turn id for the records that follow.
                    Some("turn_context") => {
                        current_turn = dotted(v, "payload.turn_id")
                            .and_then(|s| s.as_str())
                            .map(String::from);
                        if let Some(m) = dotted(v, "payload.model").and_then(|s| s.as_str()) {
                            models.push(ModelFact {
                                at: ts.clone(),
                                model: m.to_string(),
                                via: "turn_context".to_string(),
                            });
                        }
                    }
                    Some("response_item") => {
                        let p = dotted(v, "payload").unwrap_or(&serde_json::Value::Null);
                        match p.get("type").and_then(|t| t.as_str()) {
                            Some("message") => {
                                let role = p.get("role").and_then(|s| s.as_str()).unwrap_or("");
                                let text = p
                                    .get("content")
                                    .and_then(|c| c.as_array())
                                    .map(|blocks| {
                                        blocks
                                            .iter()
                                            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                                            .collect::<Vec<_>>()
                                            .join("")
                                    })
                                    .unwrap_or_default();
                                if role == "user" {
                                    let mut fields = BTreeMap::new();
                                    fields.insert(
                                        "thread_source".to_string(),
                                        FieldValue::Str(thread_source.clone()),
                                    );
                                    put_str(&mut fields, "role", p.get("role"));
                                    if let Some(pp) = &parent {
                                        fields.insert(
                                            "parent_thread_id".to_string(),
                                            FieldValue::Str(pp.clone()),
                                        );
                                    }
                                    user_slots.push(UserSlotRecord {
                                        store: store.name.clone(),
                                        seq: rec.seq,
                                        ordinal,
                                        timestamp: ts.clone(),
                                        text: text.clone(),
                                        pair_key: current_turn.clone(),
                                        session_id: file_session.clone(),
                                        directory: directory.clone(),
                                        fields,
                                        raw_sha256: rec.hash.clone(),
                                    });
                                }
                                let step = Step {
                                    step_id: 0,
                                    timestamp: ts.clone(),
                                    source: match role {
                                        "user" => StepSource::User,
                                        "assistant" => StepSource::Agent,
                                        _ => StepSource::System,
                                    },
                                    model_name: None,
                                    message: Message::Text(text),
                                    reasoning_content: None,
                                    tool_calls: Vec::new(),
                                    observation: None,
                                    metrics: None,
                                    llm_call_count: None,
                                    // The subagent file replays the parent verbatim (Surprise 2).
                                    is_copied_context: if thread_source == "subagent" {
                                        Some(true)
                                    } else {
                                        None
                                    },
                                    extra: Some(Extra {
                                        goalcup: GoalcupExtra {
                                            raw: vec![RawLink {
                                                store: store.name.clone(),
                                                seq: rec.seq,
                                                sha256: rec.hash.clone(),
                                            }],
                                            provenance: Some(Provenance {
                                                origin: Some(thread_source.clone()),
                                                source: Some(role.to_string()),
                                                pair_key: current_turn.clone(),
                                            }),
                                        },
                                    }),
                                };
                                if is_tracked_file {
                                    trajectory.push(step);
                                } else {
                                    sub.push(step);
                                }
                            }
                            Some("custom_tool_call") | Some("function_call") => {
                                let call_id = p
                                    .get("call_id")
                                    .and_then(|s| s.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let name = p
                                    .get("name")
                                    .and_then(|s| s.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                tool_calls.push(ToolFact {
                                    call_id: call_id.clone(),
                                    name: name.clone(),
                                    leg: "leg1",
                                    turn_key: current_turn.clone(),
                                    order: tool_calls.len(),
                                });
                                let step = Step {
                                    step_id: 0,
                                    timestamp: ts.clone(),
                                    source: StepSource::Agent,
                                    model_name: None,
                                    message: Message::Text(String::new()),
                                    reasoning_content: None,
                                    tool_calls: vec![ToolCall {
                                        tool_call_id: call_id,
                                        function_name: name,
                                        arguments: serde_json::json!({
                                            "input": p.get("input").cloned().unwrap_or(serde_json::Value::Null),
                                            "arguments": p.get("arguments").cloned().unwrap_or(serde_json::Value::Null),
                                        }),
                                        extra: None,
                                    }],
                                    observation: None,
                                    metrics: None,
                                    llm_call_count: None,
                                    is_copied_context: None,
                                    extra: Some(Extra {
                                        goalcup: GoalcupExtra {
                                            raw: vec![RawLink {
                                                store: store.name.clone(),
                                                seq: rec.seq,
                                                sha256: rec.hash.clone(),
                                            }],
                                            provenance: None,
                                        },
                                    }),
                                };
                                if is_tracked_file {
                                    trajectory.push(step);
                                } else {
                                    sub.push(step);
                                }
                            }
                            _ => {}
                        }
                    }
                    Some("event_msg") => {
                        if dotted(v, "payload.type").and_then(|t| t.as_str()) == Some("token_count")
                        {
                            // The per-turn delta, not `total_token_usage`.
                            // `total_token_usage` is a running total that also resets when
                            // the thread rolls over — 14,627 → 30,702 → … → 226,245, then
                            // back down — so pushing a `TokenFact` per `token_count` event
                            // would count the same tokens once more for every turn that
                            // followed them. `last_token_usage` is the delta for the turn
                            // the event reports, and summing deltas is what a total is.
                            let info = dotted(v, "payload.info.last_token_usage");
                            tokens.push(TokenFact {
                                model: models
                                    .last()
                                    .map(|m| m.model.clone())
                                    .unwrap_or_else(|| "unknown".to_string()),
                                prompt: info
                                    .and_then(|i| i.get("input_tokens"))
                                    .and_then(|n| n.as_u64())
                                    .unwrap_or(0),
                                completion: info
                                    .and_then(|i| i.get("output_tokens"))
                                    .and_then(|n| n.as_u64())
                                    .unwrap_or(0),
                                cached: info
                                    .and_then(|i| i.get("cached_input_tokens"))
                                    .and_then(|n| n.as_u64())
                                    .unwrap_or(0),
                                // plan_type, credit balance and quota ride here.
                                stripped_fields: vec!["rate_limits".to_string()],
                            });
                        }
                        if dotted(v, "payload.type").and_then(|t| t.as_str())
                            == Some("thread_settings_applied")
                            && let Some(m) =
                                dotted(v, "payload.thread_settings.model").and_then(|s| s.as_str())
                        {
                            models.push(ModelFact {
                                at: ts.clone(),
                                model: m.to_string(),
                                via: "switch".to_string(),
                            });
                        }
                    }
                    // No branch for this kind, so nothing downstream sees the record.
                    //
                    // Record 0 is the identifying `session_meta` — read above this loop,
                    // by Rule 1, which is what keys the whole file — so it is not ignored
                    // and is not counted. Every later `session_meta` really is dropped:
                    // the subagent rollout carries the parent's own meta verbatim at
                    // ordinal 1, and that one is counted.
                    kind => {
                        if i > 0 {
                            census.ignored(&store.name, kind);
                        }
                    }
                }
            }
            if !is_tracked_file && !sub.steps.is_empty() && thread_source == "subagent" {
                trajectory.subagent_trajectories.push(sub);
            }
        }

        // The block decision itself: a prompt that reached `history.jsonl` for the tracked
        // session but appears as no `role=user` text anywhere in the rollout is the one the
        // hook refused — everything else in `history.jsonl` is an ordinary prompt that also
        // landed as a `response_item`. This is an absence check over `user_slots`, not a
        // length check over `text`, deliberately: `text` is attacker-controlled human input
        // on a live capture, so deciding "blocked" from its length (or from a scrub marker
        // inside it) could be defeated by the input itself. `truncated_marker_chars` still
        // runs here, but only to make the message below readable; it plays no part in which
        // records land in `blocked_prompts`.
        let rollout_texts: BTreeSet<&str> = user_slots.iter().map(|u| u.text.as_str()).collect();
        for text in &tracked_history_texts {
            if rollout_texts.contains(text.as_str()) {
                continue;
            }
            let measured_chars =
                truncated_marker_chars(text).unwrap_or_else(|| text.chars().count());
            blocked_prompts.push(format!(
                "history.jsonl retains a {measured_chars}-character prompt absent from the \
                 rollout, which the hook refused: {}",
                text.chars().take(40).collect::<String>()
            ));
        }

        let event = manifest.human_rule.leg2_event.clone();
        let field = manifest.human_rule.byte_exact_field.clone();
        let pair = manifest.human_rule.pair_key.clone();
        // Transport-aware dedupe, same as the Claude Code parser. Codex has one hook shim,
        // so nothing here ever arrives over a second transport and this is a no-op on the
        // fixture — which is the point: a retrying shim's second post over the same
        // transport is a replay, and it reaches the human rule as a duplicate pairing.
        let mut dedupe = super::TransportDedupe::default();
        let mut leg2_candidates = Vec::new();
        for post in leg2.by_event(&event) {
            let text = dotted(&post.body, &field)
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let key = dotted(&post.body, &pair)
                .and_then(|s| s.as_str())
                .map(String::from);
            if dedupe.is_other_transport_copy(key.as_deref(), post.transport(), &text) {
                continue;
            }
            leg2_candidates.push(Leg2Candidate {
                index: post.index,
                received_at: post.received_at,
                event: post.event.clone(),
                text,
                pair_key: key,
                session_id: dotted(&post.body, "session_id")
                    .and_then(|s| s.as_str())
                    .map(String::from),
                directory: dotted(&post.body, "cwd")
                    .and_then(|s| s.as_str())
                    .map(String::from),
            });
        }
        for post in leg2.by_event("PreToolUse") {
            tool_calls.push(ToolFact {
                call_id: dotted(&post.body, "tool_use_id")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                name: dotted(&post.body, "tool_name")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                leg: "leg2",
                turn_key: dotted(&post.body, "turn_id")
                    .and_then(|s| s.as_str())
                    .map(String::from),
                order: post.index,
            });
        }
        let mut resume_markers = Vec::new();
        for post in leg2.by_event("SessionStart") {
            if dotted(&post.body, "source").and_then(|s| s.as_str()) == Some("resume") {
                resume_markers.push("resume".to_string());
            }
            if let Some(m) = dotted(&post.body, "model").and_then(|s| s.as_str()) {
                models.push(ModelFact {
                    at: None,
                    model: m.to_string(),
                    via: "hook".to_string(),
                });
            }
        }

        Ok(ParsedLegs {
            trajectory,
            user_slots,
            leg2_candidates,
            sessions,
            models,
            tokens,
            tool_calls,
            config_events,
            resume_markers,
            blocked_prompts,
            // The codex descriptor names no `[[raw]]` bodies — the capture holds no API
            // bodies — so this is empty. It is still computed rather than hard-coded, so a
            // descriptor that gains one is read without touching this parser.
            raw_bodies: super::hash_raw_bodies(raw),
            // `history` is absent from this map on purpose: the parser reads that store in
            // full before this loop, so its records are not ignored ones.
            census: census.into_map(),
            // One source for a call's arguments, so two readings cannot disagree.
            tool_args_conflicts: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::truncated_marker_chars;

    #[test]
    fn no_marker_returns_none() {
        assert_eq!(truncated_marker_chars("also add a README"), None);
        assert_eq!(truncated_marker_chars(""), None);
    }

    #[test]
    fn unterminated_marker_returns_none() {
        // No trailing space after the digits: `rest.find(' ')` finds nothing.
        assert_eq!(truncated_marker_chars("xxx<truncated:4100"), None);
    }

    #[test]
    fn non_numeric_count_returns_none() {
        assert_eq!(
            truncated_marker_chars("xxx<truncated:many chars total>"),
            None
        );
    }

    #[test]
    fn valid_marker_returns_the_count() {
        assert_eq!(
            truncated_marker_chars("xxx...<truncated:4100 chars total>"),
            Some(4100)
        );
        // `rfind` picks the last marker, matching the fixture's own single-marker case
        // and staying well-defined if a body ever carried more than one.
        assert_eq!(
            truncated_marker_chars("<truncated:1 chars total><truncated:4100 chars total>"),
            Some(4100)
        );
    }
}
