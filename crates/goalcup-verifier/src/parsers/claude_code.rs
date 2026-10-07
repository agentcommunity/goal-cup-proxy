//! Claude Code CLI 2.1.259, launch mode. Facts from spikes/claude-code-cli/REPORT.md.

use super::{
    Census, ConfigFact, FieldValue, Leg2Candidate, ModelFact, ParsedLegs, Parser, SessionFact,
    TokenFact, ToolFact, UserSlotRecord, put_bool, put_str,
};
use crate::VerifierError;
use crate::atif::{
    Agent, Extra, GoalcupExtra, Message, Provenance, RawLink, Step, StepSource, ToolCall,
    Trajectory,
};
use crate::legs::{Leg1Store, Leg2Stream, dotted};
use crate::trace::TraceDescriptor;
use goalcup_protocol::manifest::ConnectorManifest;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

pub struct ClaudeCodeParser;

/// Which trajectory a step a raw body joined to actually lives in. `Trajectory::push`
/// numbers steps from 1 **per trajectory**, so a bare `step_id` is ambiguous across
/// trajectories — the main trajectory's step 2 and a subagent trajectory's step 2 are
/// unrelated steps that merely share a number. `usize` is the index into
/// `trajectory.subagent_trajectories`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkTarget {
    Main(u64),
    Subagent(usize, u64),
}

impl LinkTarget {
    fn step_id(self) -> u64 {
        match self {
            LinkTarget::Main(id) => id,
            LinkTarget::Subagent(_, id) => id,
        }
    }
}

/// The record's own answer to "does this line belong to a subagent?".
///
/// The store's name is never consulted: the manifest's store glob is
/// `{config_dir}/projects/{slug}/{session_id}/subagents/agent-*.jsonl`, so the name a daemon export
/// lands on is `agent-<id>`. Both signals are read because both are present on every sidechain line
/// in the measured transcript (17 records, all `isSidechain: true`, `agentId` set) and a
/// cut that carries only one of them is still a subagent's.
fn is_sidechain(v: &serde_json::Value) -> bool {
    dotted(v, "isSidechain").and_then(|b| b.as_bool()) == Some(true) || agent_id(v).is_some()
}

/// The subagent's own identifier, empty strings treated as absent.
fn agent_id(v: &serde_json::Value) -> Option<String> {
    dotted(v, "agentId")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// One record's claim on a session's facts, and the order those claims are settled in.
///
/// A session's facts come from its own first record — but "first" has to be a property of
/// the records themselves, never of the order `[[leg1]]` happens to declare the stores. A
/// Claude Code sidechain transcript carries the same `sessionId` as the main one, so both
/// files make a claim on the same session, and keeping whichever arrival came first would
/// record `thread_source: "subagent"` for the user's own session whenever the subagent
/// store was declared above the main one.
///
/// The order is:
///   1. `recorded_at`, when the record carries a time. An untimed record sorts before every
///      timed one: Claude Code opens a transcript with `mode` / `permission-mode` /
///      `atis-latch` bookkeeping lines that carry the `sessionId` and no `timestamp` at
///      all, and those lines are the session's first.
///   2. the store's own sequence, which is the only thing that orders records inside one
///      file (`SeqSpace::LocalPerStore`).
///   3. a root record before a sidechain one. Reachable only when records in two different
///      files tie on both keys above, and it settles that tie toward the one thing that is
///      certainly true: a sidechain cannot open the session that spawned it.
#[derive(Debug, Clone)]
struct SessionClaim {
    at: Option<f64>,
    seq: u64,
    sidechain: bool,
}

impl SessionClaim {
    fn precedes(&self, other: &SessionClaim) -> bool {
        match (self.at, other.at) {
            (None, Some(_)) => return true,
            (Some(_), None) => return false,
            (Some(a), Some(b)) if a != b => return a < b,
            _ => {}
        }
        if self.seq != other.seq {
            return self.seq < other.seq;
        }
        !self.sidechain && other.sidechain
    }
}

impl Parser for ClaudeCodeParser {
    fn connector_id(&self) -> &'static str {
        "claude-code-cli"
    }

    fn join_strategy(&self) -> &'static str {
        "prompt_id"
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
        let mut sessions: Vec<SessionFact> = Vec::new();
        let mut models = Vec::new();
        let mut tokens = Vec::new();
        let mut tool_calls = Vec::new();
        let mut config_events = Vec::new();
        let mut blocked_prompts = Vec::new();
        // Join keys back to a `[[raw]]` body, each tagged with the `LinkTarget` (main, or
        // a specific subagent trajectory) the matched step actually lives in — see
        // `LinkTarget`'s doc comment for why an untagged `step_id` alone is not enough.
        //
        // (target, requestId): the join key in the general case, where a body's filename
        // stem is the request id.
        let mut request_ids: Vec<(LinkTarget, String)> = Vec::new();
        // (target, message.id): a raw **response** body's own `id` field is the API
        // message id (`response.id` in the Claude facts), never the transcript's
        // `requestId` (trace.rs: "the join is by filename, not by matching `id` fields"
        // describes the general case; on this committed fixture the curator renamed the
        // files to descriptive stems, so the only field-based join left is `message.id`
        // == the response body's `id`).
        let mut message_ids: Vec<(LinkTarget, String)> = Vec::new();

        let mut trajectory = Trajectory::new(Agent {
            name: "claude-code-cli".to_string(),
            version: trace.harness_version.clone(),
            model_name: None,
            tool_definitions: Vec::new(),
            extra: None,
        });
        trajectory.session_id = trace.tracked_sessions.first().cloned();

        // ------------------------------------------------------------ pass 1: sessions
        //
        // Every session's facts are settled over all stores before any of them is walked
        // for content, so no fact depends on which `[[leg1]]` line came first. See
        // `SessionClaim` for the order a session's own first record is chosen in, and why
        // this cannot be the walk order: the sidechain transcript shares the main one's
        // `sessionId`.
        //
        // The directory is settled separately, from the first record that has a `cwd`
        // rather than from the first record that has a `sessionId`. Record 0 of
        // `main-session.jsonl` is `{"type":"mode",…}`: it carries the `sessionId` and no
        // `cwd` at all. Taking the directory from record 0 would leave `directory: None` on
        // the tracked session, and V10 would then emit a false `partition_violation` Fail
        // on a fixture whose data is perfectly partitioned.
        //
        // `order` is the order the sessions are reported in — first appearance in the walk.
        // It is deliberately not the claim order: what a session's facts are must not depend
        // on the declaration order, and that is what `identity` settles below; where a
        // session sits in the list is bookkeeping, and no check reads a fact out of it.
        let mut order: Vec<String> = Vec::new();
        let mut identity: BTreeMap<String, (SessionClaim, String, bool)> = BTreeMap::new();
        let mut directories: BTreeMap<String, (SessionClaim, String)> = BTreeMap::new();
        for store in leg1 {
            for rec in &store.records {
                let v = &rec.value;
                let Some(session_id) = dotted(v, "sessionId")
                    .and_then(|s| s.as_str())
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                let sidechain = is_sidechain(v);
                let claim = SessionClaim {
                    at: rec.recorded_at,
                    seq: rec.seq,
                    sidechain,
                };
                match identity.get_mut(session_id) {
                    Some(held) if !claim.precedes(&held.0) => {}
                    Some(held) => *held = (claim.clone(), store.name.clone(), sidechain),
                    None => {
                        order.push(session_id.to_string());
                        identity.insert(
                            session_id.to_string(),
                            (claim.clone(), store.name.clone(), sidechain),
                        );
                    }
                }
                if let Some(cwd) = dotted(v, "cwd").and_then(|s| s.as_str()) {
                    match directories.get_mut(session_id) {
                        Some(held) if !claim.precedes(&held.0) => {}
                        Some(held) => *held = (claim, cwd.to_string()),
                        None => {
                            directories.insert(session_id.to_string(), (claim, cwd.to_string()));
                        }
                    }
                }
            }
        }
        for id in &order {
            let (_, store, sidechain) = &identity[id];
            sessions.push(SessionFact {
                id: id.clone(),
                store: store.clone(),
                directory: directories.get(id).map(|(_, d)| d.clone()),
                parent: None,
                thread_source: if *sidechain { "subagent" } else { "user" }.to_string(),
                is_root: !sidechain,
                tracked: tracked.contains(id.as_str()),
            });
        }

        // The stores in an order the descriptor cannot influence: earliest record first,
        // ties broken by name. It orders one thing — the model facts — because V5 publishes
        // them as "in order of first appearance", and an appearance order that depends on
        // which `[[leg1]]` line came first is a statement about the descriptor rather than
        // about the run. Inside a store the append order is kept and the timestamps are not
        // consulted at all — a Claude transcript's own timestamps are out of order,
        // and append order is the only ordering the file guarantees.
        let mut store_rank: BTreeMap<&str, usize> = BTreeMap::new();
        {
            let mut ranked: Vec<(&str, Option<f64>)> = leg1
                .iter()
                .map(|s| {
                    let first = s
                        .records
                        .iter()
                        .filter_map(|r| r.recorded_at)
                        .reduce(f64::min);
                    (s.name.as_str(), first)
                })
                .collect();
            ranked.sort_by(|a, b| match (a.1, b.1) {
                (Some(x), Some(y)) => x
                    .partial_cmp(&y)
                    .unwrap_or(Ordering::Equal)
                    .then_with(|| a.0.cmp(b.0)),
                // A store whose every record is untimed cannot be placed against a timed
                // one, so it goes last rather than pretending to a position it has not shown.
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => a.0.cmp(b.0),
            });
            for (i, (name, _)) in ranked.iter().enumerate() {
                store_rank.insert(name, i);
            }
        }
        // Model facts with the (store rank, in-store sequence) they are ordered by, sorted
        // into `models` once every store has been walked.
        let mut leg1_models: Vec<(usize, u64, ModelFact)> = Vec::new();

        // --------------------------------------------------- pass 2: content and steps
        //
        // Which subagent trajectory a sidechain record belongs to, keyed by
        // `(session_id, agent_id)` rather than by the store it arrived in. The manifest's
        // subagent glob is `…/{session_id}/subagents/agent-*.jsonl`, so one session can
        // land two subagent files; keying on the store name collided them on `RawLink.store`
        // and keying on the session id alone inlined their steps into the main trajectory.
        // The value is the index into `trajectory.subagent_trajectories`.
        let mut subagent_index: BTreeMap<(String, Option<String>), usize> = BTreeMap::new();
        // Which record kinds this dispatch has no branch for, counted as it goes.
        let mut census = Census::new(trace);

        for store in leg1 {
            // A store the descriptor declared foreign contributes no step to any
            // trajectory — neither the main one nor a subagent's. It still contributes
            // user slots and session facts, which is what the checks judge it by.
            let foreign = trace.leg1.iter().any(|s| s.name == store.name && s.foreign);
            let rank = store_rank[store.name.as_str()];

            for rec in &store.records {
                let v = &rec.value;
                let session_id = dotted(v, "sessionId")
                    .and_then(|s| s.as_str())
                    .unwrap_or_default()
                    .to_string();
                let sidechain = is_sidechain(v);
                // The trajectory this record's step belongs to, created on first use so a
                // subagent with no step never becomes an empty trajectory.
                let mut subagent_slot = |trajectory: &mut Trajectory| -> usize {
                    let key = (session_id.clone(), agent_id(v));
                    *subagent_index.entry(key.clone()).or_insert_with(|| {
                        let mut sub = Trajectory::new(Agent {
                            name: "claude-code-cli".to_string(),
                            version: trace.harness_version.clone(),
                            model_name: None,
                            tool_definitions: Vec::new(),
                            extra: None,
                        });
                        sub.trajectory_id = key.1.clone();
                        trajectory.subagent_trajectories.push(sub);
                        trajectory.subagent_trajectories.len() - 1
                    })
                };

                match dotted(v, "type").and_then(|t| t.as_str()) {
                    Some("user") => {
                        let content = dotted(v, "message.content");
                        // A typed prompt is a plain string; tool results are block arrays.
                        let text = match content {
                            Some(serde_json::Value::String(s)) => s.clone(),
                            Some(serde_json::Value::Array(blocks)) => blocks
                                .iter()
                                .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                                .collect::<Vec<_>>()
                                .join(""),
                            _ => String::new(),
                        };
                        if text.is_empty() {
                            continue; // tool_result records carry no user text
                        }
                        let mut fields = BTreeMap::new();
                        put_str(&mut fields, "origin.kind", dotted(v, "origin.kind"));
                        put_str(&mut fields, "promptSource", dotted(v, "promptSource"));
                        put_str(&mut fields, "userType", dotted(v, "userType"));
                        put_str(&mut fields, "entrypoint", dotted(v, "entrypoint"));
                        put_bool(&mut fields, "isMeta", dotted(v, "isMeta"));
                        put_bool(&mut fields, "isSidechain", dotted(v, "isSidechain"));
                        put_bool(
                            &mut fields,
                            "isCompactSummary",
                            dotted(v, "isCompactSummary"),
                        );
                        // The record's own answer, carried on the record, so the human
                        // rule's `not_subagent_thread_source` clause can read it directly.
                        // Without it the clause could only consult the session — and a
                        // sidechain record shares its parent's session id, so every
                        // sidechain prompt would pass the clause and be kept out of the
                        // human list by the manifest's `sidechain` exclusion alone.
                        fields.insert(
                            "thread_source".to_string(),
                            FieldValue::Str(
                                if sidechain { "subagent" } else { "user" }.to_string(),
                            ),
                        );

                        let record = UserSlotRecord {
                            store: store.name.clone(),
                            seq: rec.seq,
                            ordinal: None,
                            timestamp: dotted(v, "timestamp")
                                .and_then(|s| s.as_str())
                                .map(String::from),
                            text: text.clone(),
                            pair_key: dotted(v, "promptId")
                                .and_then(|s| s.as_str())
                                .map(String::from),
                            session_id: session_id.clone(),
                            directory: dotted(v, "cwd").and_then(|s| s.as_str()).map(String::from),
                            fields,
                            raw_sha256: rec.hash.clone(),
                        };
                        let step = Step {
                            step_id: 0,
                            timestamp: record.timestamp.clone(),
                            source: StepSource::User,
                            model_name: None,
                            message: Message::Text(text),
                            reasoning_content: None,
                            tool_calls: Vec::new(),
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
                                    provenance: Some(Provenance {
                                        origin: as_str(record.field("origin.kind")),
                                        source: as_str(record.field("promptSource")),
                                        pair_key: record.pair_key.clone(),
                                    }),
                                },
                            }),
                        };
                        if !foreign {
                            if sidechain {
                                let idx = subagent_slot(&mut trajectory);
                                trajectory.subagent_trajectories[idx].push(step);
                            } else {
                                trajectory.push(step);
                            }
                        }
                        user_slots.push(record);
                    }
                    Some("assistant") => {
                        let model = dotted(v, "message.model")
                            .and_then(|s| s.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        leg1_models.push((
                            rank,
                            rec.seq,
                            ModelFact {
                                at: dotted(v, "timestamp")
                                    .and_then(|s| s.as_str())
                                    .map(String::from),
                                model: model.clone(),
                                via: "message".to_string(),
                            },
                        ));
                        if let Some(usage) = dotted(v, "message.usage") {
                            let prompt = num(usage.get("input_tokens"));
                            let completion = num(usage.get("output_tokens"));
                            // The `<synthetic>` resume record carries a `usage` block, but
                            // it made no API call: every field in it is zero. Recording a
                            // token fact for zero-and-zero would assert a spend that never
                            // happened, so only a record with a real non-zero usage becomes
                            // a `TokenFact`.
                            if prompt > 0 || completion > 0 {
                                tokens.push(TokenFact {
                                    model: model.clone(),
                                    prompt,
                                    completion,
                                    cached: num(usage.get("cache_read_input_tokens")),
                                    stripped_fields: vec!["service_tier".to_string()],
                                });
                            }
                        }
                        let mut calls = Vec::new();
                        if let Some(serde_json::Value::Array(blocks)) = dotted(v, "message.content")
                        {
                            for b in blocks {
                                if b.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                                    let id = b
                                        .get("id")
                                        .and_then(|s| s.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let name = b
                                        .get("name")
                                        .and_then(|s| s.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    tool_calls.push(ToolFact {
                                        call_id: id.clone(),
                                        name: name.clone(),
                                        leg: "leg1",
                                        turn_key: dotted(v, "requestId")
                                            .and_then(|s| s.as_str())
                                            .map(String::from),
                                        order: tool_calls.len(),
                                    });
                                    calls.push(ToolCall {
                                        tool_call_id: id,
                                        function_name: name,
                                        arguments: b
                                            .get("input")
                                            .cloned()
                                            .unwrap_or(serde_json::json!({})),
                                        extra: None,
                                    });
                                }
                            }
                        }
                        let text = text_of(dotted(v, "message.content"));
                        let step = Step {
                            step_id: 0,
                            timestamp: dotted(v, "timestamp")
                                .and_then(|s| s.as_str())
                                .map(String::from),
                            source: StepSource::Agent,
                            model_name: Some(model),
                            message: Message::Text(text),
                            reasoning_content: None,
                            tool_calls: calls,
                            observation: None,
                            metrics: None,
                            llm_call_count: Some(1),
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
                        // Remember which step carries which `requestId` and `message.id`:
                        // together they are the join keys to the `[[raw]]` bodies.
                        let request_id = dotted(v, "requestId")
                            .and_then(|s| s.as_str())
                            .map(String::from);
                        let message_id = dotted(v, "message.id")
                            .and_then(|s| s.as_str())
                            .map(String::from);
                        if !foreign {
                            let target = if sidechain {
                                let idx = subagent_slot(&mut trajectory);
                                let id = trajectory.subagent_trajectories[idx].push(step);
                                LinkTarget::Subagent(idx, id)
                            } else {
                                LinkTarget::Main(trajectory.push(step))
                            };
                            if let Some(r) = request_id {
                                request_ids.push((target, r));
                            }
                            if let Some(m) = message_id {
                                message_ids.push((target, m));
                            }
                        }
                    }
                    Some("system") => {
                        if let Some(content) = dotted(v, "content").and_then(|s| s.as_str())
                            && content.starts_with("UserPromptSubmit operation blocked by hook:")
                        {
                            blocked_prompts.push(content.to_string());
                        }
                    }
                    Some("attachment") => {
                        if dotted(v, "attachment.type").and_then(|s| s.as_str()) == Some("model")
                            && let Some(id) =
                                dotted(v, "attachment.identity.modelId").and_then(|s| s.as_str())
                        {
                            leg1_models.push((
                                rank,
                                rec.seq,
                                ModelFact {
                                    at: dotted(v, "timestamp")
                                        .and_then(|s| s.as_str())
                                        .map(String::from),
                                    model: id.to_string(),
                                    via: "switch".to_string(),
                                },
                            ));
                        }
                    }
                    // The dispatch has no branch for this kind, so nothing downstream
                    // will ever see the record. Counted rather than dropped in silence:
                    // `last-prompt` and `queue-operation` land here, and both carry the
                    // human's text verbatim.
                    kind => census.ignored(&store.name, kind),
                }
            }
        }

        // Every store has been walked, so the leg-1 model facts can be put in the order
        // they were observed in rather than the order the descriptor declared. Leg-2 model
        // facts follow: they arrive on the hook stream, after leg 1.
        leg1_models.sort_by_key(|(rank, seq, _)| (*rank, *seq));
        models.extend(leg1_models.into_iter().map(|(_, _, fact)| fact));

        // Leg 2: collapse the two hook sources into one candidate per submission, and
        // record the double-fired resume once. The dedupe is transport-aware: the http
        // hook's copy and the `-cmd` shim's copy of the same prompt are one submission, but
        // a second post over the same transport is a replay and must reach the human rule,
        // which reports it as a duplicate pairing.
        let event = manifest.human_rule.leg2_event.clone();
        let field = manifest.human_rule.byte_exact_field.clone();
        let pair = manifest.human_rule.pair_key.clone();
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

        let mut resume_markers: Vec<String> = Vec::new();
        let mut seen_starts: BTreeSet<(String, String)> = BTreeSet::new();
        for post in leg2.by_event("SessionStart") {
            let sid = dotted(&post.body, "session_id")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let source = dotted(&post.body, "source")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            // One resume fires SessionStart twice: dedupe by (session_id, source).
            if seen_starts.insert((sid, source.clone())) && source == "resume" {
                resume_markers.push(source);
            }
            if let Some(m) = dotted(&post.body, "model").and_then(|s| s.as_str()) {
                models.push(ModelFact {
                    at: None,
                    model: m.to_string(),
                    via: "hook".to_string(),
                });
            }
        }
        for post in leg2.by_event("PostModelSwitch") {
            if let Some(to) = dotted(&post.body, "to_model").and_then(|s| s.as_str()) {
                models.push(ModelFact {
                    at: None,
                    model: to.to_string(),
                    via: "switch".to_string(),
                });
            }
        }
        for post in leg2.by_event("ConfigChange") {
            config_events.push(ConfigFact {
                event: "ConfigChange".to_string(),
                at: None,
                detail: post.body.to_string(),
                // This connector's hooks mint no instance id, so a launch is counted as
                // one arming signal — what V9 read before the field existed.
                shim: None,
            });
        }
        for post in leg2.by_event("PreToolUse") {
            let id = dotted(&post.body, "tool_use_id")
                .and_then(|s| s.as_str())
                .unwrap_or("");
            let name = dotted(&post.body, "tool_name")
                .and_then(|s| s.as_str())
                .unwrap_or("");
            tool_calls.push(ToolFact {
                call_id: id.to_string(),
                name: name.to_string(),
                leg: "leg2",
                turn_key: None,
                order: post.index,
            });
        }

        // ---------------------------------------------------------- raw bodies
        //
        // Two jobs. First, hash-link: every `[[raw]]` body is joined to the transcript step
        // it belongs to, so the bodies are hash-linked from `extra` as well as the
        // transcript lines. Two join strategies, tried in order:
        //   1. filename-stem contains a `requestId` an assistant step carries — the
        //      general-case naming convention (a live capture names each file after its
        //      request id).
        //   2. the body's own `id` field equals an assistant step's `message.id` — the
        //      response's own API identifier (trace.rs: "the body's own `id` is the API's
        //      own identifier … rather than the transcript's `requestId`"). This is the
        //      join that actually resolves on this committed fixture, whose files are
        //      curated under descriptive stems (`rawbody-goal-turn.response.json`) rather
        //      than request ids.
        // Both `request_ids` and `message_ids` carry a `LinkTarget`, not a bare `step_id`:
        // the main trajectory and each subagent trajectory number their steps from 1
        // independently, so a match must be attached to the step in the trajectory it
        // actually came from, never to a same-numbered step in a different one (see
        // `LinkTarget`'s doc comment, and the regression test
        // `a_subagent_joinable_raw_body_never_lands_on_the_main_trajectory`). The link
        // points at a hash and a store name, never at a path, so a verdict survives the
        // trace directory moving.
        //
        // Second, classify. A *request* body's `messages` array carries a
        // trailing synthetic `role: "system"` entry on every captured turn (the
        // token-budget reminder Claude Code appends to every outgoing request) — skip past
        // any such trailing entries to reach the message that actually carries the turn.
        // That message is a new turn in the user slot — and therefore must have a human
        // message paired with it — when it has `role: "user"` and is not a `tool_result`
        // block array; a `tool_result` message is a continuation of a turn already
        // counted, even when Claude Code also appends a trailing `<system-reminder>` text
        // block to it (a block array is classified by the *absence* of
        // a `tool_result` block, not by the presence of a `text` one). Recording that
        // judgement here is what lets V3 ask "was every new user-slot turn witnessed on
        // both legs?" of the raw bodies as well as of the transcript.
        let mut raw_bodies = super::raw_body_facts(raw);
        // Resolved per raw body, in the same order as `raw_bodies`: which trajectory (and
        // which step within it) a match belongs to, so the attach loop below routes the
        // `RawLink` to the right trajectory rather than just the right `step_id` number.
        let mut link_targets: Vec<Option<LinkTarget>> = Vec::with_capacity(raw_bodies.len());
        for (fact, body) in raw_bodies.iter_mut().zip(raw.iter()) {
            let target = request_ids
                .iter()
                .find(|(_, req)| !req.is_empty() && fact.stem.contains(req.as_str()))
                .map(|(target, _)| *target)
                .or_else(|| {
                    let id = body.value.get("id").and_then(|v| v.as_str())?;
                    message_ids
                        .iter()
                        .find(|(_, mid)| mid == id)
                        .map(|(target, _)| *target)
                });
            fact.linked_step_id = target.map(LinkTarget::step_id);
            fact.requires_paired_human = trailing_user_text_turn(&body.value);
            link_targets.push(target);
        }
        for (fact, target) in raw_bodies.iter().zip(link_targets.iter()) {
            let Some(target) = target else {
                continue;
            };
            let step_id = target.step_id();
            let steps = match target {
                LinkTarget::Main(_) => &mut trajectory.steps,
                LinkTarget::Subagent(idx, _) => &mut trajectory.subagent_trajectories[*idx].steps,
            };
            if let Some(step) = steps.iter_mut().find(|s| s.step_id == step_id) {
                let extra = step.extra.get_or_insert(Extra {
                    goalcup: GoalcupExtra {
                        raw: Vec::new(),
                        provenance: None,
                    },
                });
                extra.goalcup.raw.push(RawLink {
                    store: "raw_bodies".to_string(),
                    seq: step_id,
                    sha256: fact.sha256.clone(),
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
            raw_bodies,
            census: census.into_map(),
            // One source for a call's arguments, so two readings cannot disagree.
            tool_args_conflicts: Vec::new(),
        })
    }
}

/// True when a raw **request** body's last real message is a `role: "user"` turn that is
/// not a `tool_result` block array — i.e. a new turn in the user slot, which a human
/// message must pair with.
///
/// Claude Code appends a synthetic `role: "system"` entry (the token-budget reminder) to
/// the tail of every captured request body, so the literal last array element is never the
/// message that carries the turn; trailing system entries are skipped first.
///
/// A block-array message is classified by the **absence** of a `tool_result` block, not by
/// the presence of a `text` one: Claude Code appends a `<system-reminder>` text block after
/// a `tool_result` block in the same message, and that message is still a continuation of
/// a turn already counted, not a new human-or-harness turn (an `any(type == "text")` check
/// would misclassify it).
///
/// A body with no `messages` array is a response, not a request, and is never a new turn.
fn trailing_user_text_turn(body: &serde_json::Value) -> bool {
    let Some(messages) = body.get("messages").and_then(|m| m.as_array()) else {
        return false;
    };
    let Some(last) = messages
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(|r| r.as_str()) != Some("system"))
    else {
        return false;
    };
    if last.get("role").and_then(|r| r.as_str()) != Some("user") {
        return false;
    }
    match last.get("content") {
        Some(serde_json::Value::String(s)) => !s.is_empty(),
        Some(serde_json::Value::Array(blocks)) => !blocks
            .iter()
            .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result")),
        _ => false,
    }
}

fn as_str(v: Option<&FieldValue>) -> Option<String> {
    match v {
        Some(FieldValue::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

fn num(v: Option<&serde_json::Value>) -> u64 {
    v.and_then(|n| n.as_u64()).unwrap_or(0)
}

fn text_of(content: Option<&serde_json::Value>) -> String {
    match content {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(blocks)) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::trailing_user_text_turn;
    use serde_json::json;

    #[test]
    fn a_mixed_tool_result_and_text_trailing_message_is_not_a_new_turn() {
        // Claude Code appends a `<system-reminder>` text block after a
        // `tool_result` block in the same message (e.g. a reminder trailing a tool's
        // output). The presence of that text block must not by itself flip the
        // classification to "new human-or-harness turn" — the message is still a
        // continuation of a turn already counted, because it also carries a `tool_result`.
        let body = json!({
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "tool_result", "tool_use_id": "toolu_1", "content": "ok"},
                        {"type": "text", "text": "<system-reminder>...</system-reminder>"}
                    ]
                }
            ]
        });
        assert!(!trailing_user_text_turn(&body));
    }

    #[test]
    fn a_pure_text_trailing_message_is_a_new_turn() {
        // Sanity check alongside the mixed-array case above: a block array with a `text`
        // block and no `tool_result` is still a new turn.
        let body = json!({
            "messages": [
                {
                    "role": "user",
                    "content": [{"type": "text", "text": "keep going"}]
                }
            ]
        });
        assert!(trailing_user_text_turn(&body));
    }
}
