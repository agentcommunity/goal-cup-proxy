//! Per-harness parsers from raw stores to ATIF v1.8.
//!
//! A parser is the only place that knows a harness's field names. Everything it learns
//! is handed to the checks through `ParsedLegs`, so no check reads a harness field.

pub mod claude_code;
pub mod codex;

use crate::VerifierError;
use crate::atif::Trajectory;
use crate::legs::{Leg1Store, Leg2Stream};
use crate::trace::TraceDescriptor;
use goalcup_protocol::manifest::ConnectorManifest;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    Str(String),
    Bool(bool),
    Null,
}

/// One record that occupies the harness's user slot, human-authored or not.
#[derive(Debug, Clone)]
pub struct UserSlotRecord {
    pub store: String,
    pub seq: u64,
    pub ordinal: Option<u64>,
    pub timestamp: Option<String>,
    pub text: String,
    /// The identifier both legs stamp on the same submission (`human_rule.pair_key`).
    pub pair_key: Option<String>,
    pub session_id: String,
    pub directory: Option<String>,
    pub fields: BTreeMap<String, FieldValue>,
    pub raw_sha256: String,
}

impl UserSlotRecord {
    pub fn field(&self, path: &str) -> Option<&FieldValue> {
        self.fields.get(path)
    }
}

/// One leg-2 post that could be a human submission, deduped across hook sources.
/// `PartialEq` (not `Eq`: `received_at` is an `f64`) so `HumanRuleOutcome`, which carries
/// the unpaired candidates verbatim, can be compared in tests.
#[derive(Debug, Clone, PartialEq)]
pub struct Leg2Candidate {
    pub index: usize,
    pub received_at: f64,
    pub event: String,
    pub text: String,
    pub pair_key: Option<String>,
    pub session_id: Option<String>,
    pub directory: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SessionFact {
    pub id: String,
    pub store: String,
    pub directory: Option<String>,
    pub parent: Option<String>,
    pub thread_source: String,
    pub is_root: bool,
    /// The session is one of `trace.tracked_sessions`.
    pub tracked: bool,
}

#[derive(Debug, Clone)]
pub struct ModelFact {
    pub at: Option<String>,
    pub model: String,
    /// Where the identity was read: `"message"`, `"turn_context"`, `"hook"`, `"switch"`.
    pub via: String,
}

#[derive(Debug, Clone)]
pub struct TokenFact {
    pub model: String,
    pub prompt: u64,
    pub completion: u64,
    pub cached: u64,
    /// Plan / credit / billing fields dropped before publication (V11, D26).
    pub stripped_fields: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ToolFact {
    pub call_id: String,
    pub name: String,
    pub leg: &'static str,
    /// The turn identity used to join legs where ids do not match (V8).
    pub turn_key: Option<String>,
    pub order: usize,
}

#[derive(Debug, Clone)]
pub struct ConfigFact {
    pub event: String,
    pub at: Option<String>,
    pub detail: String,
    /// The id of the shim instance that posted this, where the connector's shim mints one.
    ///
    /// One launch of a harness loads one shim, and the shim mints an id at load and stamps
    /// every post it makes with it — so counting distinct ids over the arming signals
    /// counts launches, which no other fact in the trace does. `None` where the connector's
    /// shim does not mint one, and V9 then reads one signal as one launch.
    ///
    /// A participant-written string, so nothing interpolates it and nothing but equality is
    /// asked of it — and, because the participant writes it, a shim minting one constant id
    /// across two launches reads here as one launch. Distinct ids count a launch that did
    /// not hide itself; the attack of a shim built to hide one is left to V9's cap on
    /// launches the trace cannot witness.
    pub shim: Option<String>,
}

/// One `[[raw]]` body from the trace descriptor, hashed and (where the connector can)
/// linked to the step it belongs to (D14).
#[derive(Debug, Clone)]
pub struct RawBodyFact {
    pub name: String,
    /// Filename stem. Joined to a step two ways, tried in order: the stem containing a
    /// step's `requestId` (a live capture's naming convention), or — when that fails, as
    /// on this curated fixture, whose files are named after the turn rather than the
    /// request id — the body's own `id` field equaling a step's `message.id`.
    pub stem: String,
    /// Taken over the file's committed bytes, never over a re-serialisation.
    pub sha256: String,
    pub bytes: u64,
    /// The `step_id` this body was linked to, when either join (see `stem`) resolved.
    pub linked_step_id: Option<u64>,
    /// The classifier: this is a **request** body whose trailing non-system
    /// message has `role: "user"` and is not a `tool_result` block array — a new turn in
    /// the user slot, not a continuation — so a human message must pair with it. The
    /// parser records the judgement; V3 decides what to do about it.
    pub requires_paired_human: bool,
}

/// Two readings of one tool call's arguments that do not agree.
///
/// The arguments are what attribution runs on: `hello.py` is explained by the `write` call
/// whose arguments name it, so a call whose arguments are ambiguous is a file whose
/// explanation is a guess.
///
/// **No argument value is carried.** Arguments are the participant's own text (a path, a
/// shell command, a file body), and a verdict is published. The call id, the tool name and
/// the byte lengths are what a reviewer needs to go and look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolArgsConflict {
    pub call_id: String,
    pub tool: String,
    pub kind: ArgsConflictKind,
    /// Serialised byte length of the reading the parser kept, and of the one it dropped.
    pub kept_bytes: usize,
    pub dropped_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgsConflictKind {
    /// Two leg-2 hook posts carried different arguments under one callID — either the same
    /// event twice, or `tool.execute.before` announcing one thing and `.after` reporting
    /// another. First post wins, and this says a second existed.
    BetweenPosts,
    /// The leg-1 part's own `state.input` is non-empty and is not the arguments leg 2
    /// carried for the call. Leg 2 wins (the row is written before the arguments are), and
    /// this says leg 1 held a different answer rather than no answer.
    BetweenLegs,
}

#[derive(Debug, Clone)]
pub struct ParsedLegs {
    pub trajectory: Trajectory,
    pub user_slots: Vec<UserSlotRecord>,
    pub leg2_candidates: Vec<Leg2Candidate>,
    pub sessions: Vec<SessionFact>,
    pub models: Vec<ModelFact>,
    pub tokens: Vec<TokenFact>,
    pub tool_calls: Vec<ToolFact>,
    pub config_events: Vec<ConfigFact>,
    /// Deduped resume markers, e.g. `["resume"]` for the double-fired Claude SessionStart.
    pub resume_markers: Vec<String>,
    /// Text of prompts the blocking hook refused, wherever the harness kept it.
    pub blocked_prompts: Vec<String>,
    /// The `[[raw]]` bodies the descriptor names, hashed. Empty when it names none.
    pub raw_bodies: Vec<RawBodyFact>,
    /// Leg-1 record kinds the parser recognised and deliberately ignored, counted.
    ///
    /// Every parser ends its record dispatch with a fall-through arm, and this map is what
    /// that arm says: a kind nothing reads is still a kind a reviewer can learn about. On
    /// the Claude fixture it counts 48 of 148 records in 9 kinds, two of which —
    /// `last-prompt` and `queue-operation` — carry the human's text verbatim. The census is
    /// a *statement* and never a finding, because a dropped kind is not by itself evidence
    /// of anything; it is what makes those two visible to a reviewer instead of invisible
    /// to everyone.
    ///
    /// It names kinds and never contents, and it is a `BTreeMap`, so the verdict it is
    /// published in stays comparable byte for byte across two runs. A kind is the
    /// participant's own string, so each key is bounded and stripped of control
    /// characters on the way in — see `census_key`.
    ///
    /// **Scope: the tracked entry's own stores.** A store the descriptor declares
    /// `foreign = true` is counted by nobody — it belongs to another session, V10
    /// partitions it away, and folding its dropped kinds in here would tell a reviewer
    /// that the entry ignored records the entry never contained. On the Claude fixture
    /// that is `main` + `subagent`: 48 records in 9 kinds. The third declared store,
    /// `a9-other-folder`, is the `foreign = true` probe and drops 7 more
    /// (`queue-operation` 2, `atis-latch` 3, `last-prompt` 2) that this map deliberately
    /// does not carry.
    pub census: BTreeMap<String, u64>,
    /// Tool calls whose arguments two readings disagreed about.
    ///
    /// Empty for both shipped parsers, which read arguments from one source. Ordered by call
    /// id, so a verdict built from it is comparable byte for byte across two runs.
    pub tool_args_conflicts: Vec<ToolArgsConflict>,
}

/// The census accumulator a parser fills as it walks its records.
///
/// A parser reaches for `ignored` in its fall-through arm and nowhere else, so the map is
/// a function of which records the dispatch had no branch for rather than of a judgement
/// made somewhere else. The foreign-store rule lives here, in one place, rather than being
/// restated by each of the three parsers.
#[derive(Debug, Default)]
pub(crate) struct Census {
    counts: BTreeMap<String, u64>,
    /// The store names the descriptor declares `foreign = true`.
    foreign: BTreeSet<String>,
}

/// What a record with no readable kind is counted as. Unreachable on every committed
/// fixture — each one's records carry a string kind — and written rather than skipped,
/// because a record the parser cannot even name is the one a census most needs to show.
pub(crate) const UNNAMED_KIND: &str = "(no kind field)";

/// The longest a census key may be, in bytes, before it is cut.
///
/// A kind is a short identifier on every committed fixture — the longest is
/// `inter_agent_communication_metadata` at 34 bytes — so this bound never touches a real
/// one. It exists because the kind is **participant-controlled**: a legacy JSONL store is
/// a file on the participant's own disk, and its `type` field is whatever they wrote
/// there. Unbounded, that string is interned as a key of the published verdict, so a
/// paragraph of free text would ride into the verdict under a kind's name.
const MAX_KIND_BYTES: usize = 48;

/// Appended to a key that was cut, so a reader can tell a kind the harness wrote from a
/// stub of something longer. A key is never silently shortened.
const TRUNCATION_MARKER: char = '…';

/// What a control character in a kind is shown as. A newline in a key would reformat a
/// reviewer's terminal and let one record's `type` masquerade as two census lines.
const UNPRINTABLE: char = '?';

/// The verdict key a raw record kind is interned under: cut to [`MAX_KIND_BYTES`] on a
/// UTF-8 char boundary and marked where it was cut, with every control character replaced
/// by [`UNPRINTABLE`]. Bytes are the bound because the verdict is compared byte for byte.
fn census_key(kind: &str) -> String {
    let mut end = MAX_KIND_BYTES.min(kind.len());
    while !kind.is_char_boundary(end) {
        end -= 1;
    }
    let mut key: String = kind[..end]
        .chars()
        .map(|c| if c.is_control() { UNPRINTABLE } else { c })
        .collect();
    if end < kind.len() {
        key.push(TRUNCATION_MARKER);
    }
    key
}

impl Census {
    pub(crate) fn new(trace: &TraceDescriptor) -> Census {
        Census {
            counts: BTreeMap::new(),
            foreign: trace
                .leg1
                .iter()
                .filter(|s| s.foreign)
                .map(|s| s.name.clone())
                .collect(),
        }
    }

    /// True when this store's records are another session's and not the entry's.
    pub(crate) fn is_foreign(&self, store: &str) -> bool {
        self.foreign.contains(store)
    }

    /// Count one record the parser's dispatch had no branch for.
    ///
    /// The kind is normalised by [`census_key`] before it is interned, because it is the
    /// participant's own string and the map it lands in is published.
    pub(crate) fn ignored(&mut self, store: &str, kind: Option<&str>) {
        if self.is_foreign(store) {
            return;
        }
        *self
            .counts
            .entry(census_key(kind.unwrap_or(UNNAMED_KIND)))
            .or_default() += 1;
    }

    pub(crate) fn into_map(self) -> BTreeMap<String, u64> {
        self.counts
    }
}

pub trait Parser {
    fn connector_id(&self) -> &'static str;
    /// Must equal the manifest's `join_key`, so a manifest and a parser cannot drift.
    fn join_strategy(&self) -> &'static str;
    fn parse(
        &self,
        trace: &TraceDescriptor,
        manifest: &ConnectorManifest,
        leg1: &[Leg1Store],
        leg2: &Leg2Stream,
    ) -> Result<ParsedLegs, VerifierError>;
    /// Pipeline-only form that receives already bounded raw bodies, avoiding a second read.
    fn parse_with_raw(
        &self,
        trace: &TraceDescriptor,
        manifest: &ConnectorManifest,
        leg1: &[Leg1Store],
        leg2: &Leg2Stream,
        raw: &[crate::trace::RawBody],
    ) -> Result<ParsedLegs, VerifierError> {
        let _ = raw;
        self.parse(trace, manifest, leg1, leg2)
    }
}

/// Select the parser implementation for a declared connector family.
///
/// Connector IDs identify individual launch and capture surfaces; parser bodies are shared
/// by the record family those surfaces declare. Callers must validate descriptor identity
/// against the manifest before passing its family here.
pub fn parser_for_family(family: &str) -> Result<Box<dyn Parser>, VerifierError> {
    match family {
        "claude-code" => Ok(Box::new(claude_code::ClaudeCodeParser)),
        "codex" => Ok(Box::new(codex::CodexParser)),
        other => Err(VerifierError::UnknownParserFamily(other.to_string())),
    }
}

/// Leg-2 dedupe, transport-aware.
///
/// A harness that carries one submission over two hook sources must contribute one
/// candidate: Claude Code posts every `UserPromptSubmit` over both the http hook and the
/// `-cmd` shim, with the same pair key and the same bytes. Deduping on the pair key alone
/// would also swallow the case the pair key exists to catch — the *same* post arriving
/// twice on the *same* transport, which is a replay — and so make `duplicate_pairings`
/// unreachable through the real pipeline.
///
/// So a candidate is dropped only when it is the **first** arrival of that key-and-text on
/// a transport that has not carried it yet, and some other transport already has. Once a
/// transport has carried a submission, every further post on that transport passes through
/// — that is the replay, and it is the human rule's job to report it as a duplicate
/// pairing, not a parser's to hide.
///
/// What the human rule then does with what passes through, stated exactly: it pairs a
/// leg-1 record with the first unused post carrying its key, and every later post on that
/// key becomes a `duplicate_pairing` **whatever its text says**. A text mismatch is
/// reported as a divergence only when it is the *paired* post whose bytes differ from the
/// record. So a later same-key post with different text is a duplicate pairing, not a
/// divergence — the pairing is decided by the key and by arrival order, never by the text.
#[derive(Default)]
pub(crate) struct TransportDedupe {
    seen: BTreeMap<(String, String), BTreeSet<String>>,
}

impl TransportDedupe {
    /// True when this post is another transport's first copy of a submission already taken.
    ///
    /// The condition is deliberately **not** "some other transport has it": on the
    /// realistic Claude Code ordering — http, then `-cmd`, then a retrying http repost —
    /// that reading sees `cmd` in the set and drops the repost, which is the replay the
    /// human rule exists to catch. A transport that has already carried
    /// this submission keeps every later post it carries.
    pub(crate) fn is_other_transport_copy(
        &mut self,
        pair_key: Option<&str>,
        transport: &str,
        text: &str,
    ) -> bool {
        // With no pair key there is nothing to match on; keep the post and let the human
        // rule decide. Dropping it would be a silent judgement made without evidence.
        let Some(key) = pair_key else { return false };
        let transports = self
            .seen
            .entry((key.to_string(), text.to_string()))
            .or_default();
        let first_copy_on_a_new_transport =
            !transports.is_empty() && !transports.contains(transport);
        transports.insert(transport.to_string());
        first_copy_on_a_new_transport
    }
}

/// Helper shared by the parsers: insert a string field when present, a bool when present.
pub(crate) fn put_str(
    map: &mut BTreeMap<String, FieldValue>,
    key: &str,
    value: Option<&serde_json::Value>,
) {
    match value {
        Some(serde_json::Value::String(s)) => {
            map.insert(key.to_string(), FieldValue::Str(s.clone()));
        }
        Some(serde_json::Value::Null) => {
            map.insert(key.to_string(), FieldValue::Null);
        }
        _ => {}
    }
}

pub(crate) fn put_bool(
    map: &mut BTreeMap<String, FieldValue>,
    key: &str,
    value: Option<&serde_json::Value>,
) {
    if let Some(serde_json::Value::Bool(b)) = value {
        map.insert(key.to_string(), FieldValue::Bool(*b));
    }
}

/// Hash every `[[raw]]` body the descriptor names. Shared by all three parsers; a parser
/// that can join a body to a step (Claude Code, through the `stem`/`requestId` and
/// `id`/`message.id` joins documented on `RawBodyFact`) fills `linked_step_id` and
/// `requires_paired_human` afterwards.
pub(crate) fn hash_raw_bodies(raw: &[crate::trace::RawBody]) -> Vec<RawBodyFact> {
    raw_body_facts(raw)
}

/// Project already-loaded raw bodies into their public hash facts. Claude's parser needs both
/// the JSON values and the facts, so this split keeps it to one bounded read per raw file.
pub(crate) fn raw_body_facts(bodies: &[crate::trace::RawBody]) -> Vec<RawBodyFact> {
    bodies
        .iter()
        .map(|b| RawBodyFact {
            name: b.name.clone(),
            stem: b.stem.clone(),
            sha256: b.sha256.clone(),
            bytes: b.bytes,
            linked_step_id: None,
            requires_paired_human: false,
        })
        .collect()
}
