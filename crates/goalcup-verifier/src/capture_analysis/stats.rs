//! The numbers-only run summary: `goalcup-verify --stats TRACE [TRACE…] --output STATS.json`.
//!
//! [`run_stats`] reads each trace of one run, in segment order, through [`scan_capture_trace`]
//! and a fresh [`DecodeSession`] per trace, as the research report does. Its sink
//! ([`StatsSink`]) writes no observation: it keeps counters and bounded sets of hashed ids,
//! shared across the run's traces, so an id repeated across a `continue` counts once. The
//! result is one JSON object of `type: run_stats` (`docs/research/m2-report-format.md`,
//! section "Run stats").
//!
//! The de-dup rules are CONTRACTS C4's: never sum cumulative usage, retries or replayed history
//! into a total, and never add the native and the wire count of the same call. Tokens come from
//! one source per run (the wire, else the native session files), named in `tokens.source`; the
//! other source's numbers stay beside them. A number the run's decoders cannot state is `null`,
//! never `0`.
//!
//! Nothing here is text from the run. Prompts, messages, tool names, paths and hosts are read
//! only to count; the output holds counts, model names (bounded, printable ASCII), safe error
//! code tokens, decoder ids and each trace's `entry_id`.

use super::api::{Disposition, Observation, ObservationSink, kinds};
use super::native::NativeFormat;
use super::native::{claude_code::ClaudeCode, codex::Codex};
use super::reader::{CaptureIntegrityReport, scan_capture_trace};
use super::registry::DecoderRegistry;
use super::report::degraded;
use super::session::{DecodeOutcome, DecodeSession};
use super::{anthropic, chat, openai, thread};
use crate::VerifyOptions;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

/// The run-stats schema version.
pub const RUN_STATS_SCHEMA_VERSION: u32 = 1;
/// The most traces (segments) one run may name.
pub const MAX_TRACES: usize = 128;
/// The most ids one run keeps for de-dup, over every set. Past it, counting goes on without
/// de-dup and `bounds.ids_over_budget` says so.
pub const MAX_IDS: u64 = 4_000_000;
/// Models listed; the rest are counted in `models_omitted`.
const MAX_LISTED_MODELS: usize = 16;
/// Distinct model names counted per source before new ones go to `models_omitted`.
const MAX_TRACKED_MODELS: usize = 256;
/// The longest model name kept.
const MAX_MODEL_NAME_BYTES: usize = 64;
/// Error codes listed; the rest are counted under `"other"`.
const MAX_LISTED_CODES: usize = 16;
/// Distinct error codes counted per source before new ones go to `"other"`.
const MAX_TRACKED_CODES: usize = 256;
/// The longest error code token kept.
const MAX_CODE_BYTES: usize = 64;
/// The longest `entry_id` kept.
const MAX_ENTRY_ID_BYTES: usize = 128;
/// The longest stats document.
const MAX_STATS_BYTES: usize = 64 * 1024;

/// One run's exit code (0 complete and valid, 1 incomplete or degraded, 2 invalid) and its
/// stats document.
#[derive(Debug, Clone, PartialEq)]
pub struct RunStatsResult {
    pub exit_code: u8,
    pub stats: Value,
}

/// [`run_stats_with`] over the built-in decoders: the entry point of the library and of
/// `goalcup-verify --stats`.
pub fn run_stats(traces: &[PathBuf], options: &VerifyOptions) -> io::Result<RunStatsResult> {
    run_stats_with(traces, options, &DecoderRegistry::builtin())
}

/// The stats of one run whose traces are `traces`, in segment order, decoded by `registry`.
///
/// `Err` means no usable document: no trace, too many, the same trace twice, a trace that
/// could not be read or is not v5. An invalid or unsealed trace is not an error: its
/// `traces[]` entry says so and the exit code is 2 or 1.
pub fn run_stats_with(
    traces: &[PathBuf],
    options: &VerifyOptions,
    registry: &DecoderRegistry,
) -> io::Result<RunStatsResult> {
    if traces.is_empty() || traces.len() > MAX_TRACES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("name 1 to {MAX_TRACES} traces of one run"),
        ));
    }
    let mut canonical = BTreeSet::new();
    for trace in traces {
        if !canonical.insert(std::fs::canonicalize(trace)?) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is named twice", trace.display()),
            ));
        }
    }
    let mut stats = RunStats::default();
    let mut read = Vec::with_capacity(traces.len());
    for trace in traces {
        read.push(stats.read(trace, options, registry)?);
    }
    let result = stats.document(&read);
    if serde_json::to_vec(&result.stats)?.len() > MAX_STATS_BYTES {
        return Err(io::Error::other(format!(
            "the stats document is over its {MAX_STATS_BYTES}-byte bound"
        )));
    }
    Ok(result)
}

/// What one trace contributed besides its observations.
struct TraceRead {
    integrity: CaptureIntegrityReport,
    outcome: DecodeOutcome,
}

/// The sink of a stats decode: counts each observation into the run's [`RunStats`] and keeps
/// none of it.
pub struct StatsSink<'a> {
    stats: &'a mut RunStats,
}

impl ObservationSink for StatsSink<'_> {
    fn observe(
        &mut self,
        decoder_id: &'static str,
        observation: Observation,
    ) -> io::Result<Disposition> {
        self.stats.observe(decoder_id, &observation);
        Ok(Disposition::Kept)
    }
}

// ---------------------------------------------------------------------------------------
// Sources and ids.
// ---------------------------------------------------------------------------------------

/// The two sources of one model call: the harness's own session files and the recorded
/// traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leg {
    Native,
    Wire,
}

impl Leg {
    fn name(self) -> &'static str {
        match self {
            Leg::Native => "native",
            Leg::Wire => "wire",
        }
    }

    fn other(self) -> Leg {
        match self {
            Leg::Native => Leg::Wire,
            Leg::Wire => Leg::Native,
        }
    }
}

fn leg(decoder: &str) -> Option<Leg> {
    match decoder {
        Codex::ID | ClaudeCode::ID => Some(Leg::Native),
        anthropic::DECODER_ID | openai::DECODER_ID | chat::DECODER_ID | thread::DECODER_ID => {
            Some(Leg::Wire)
        }
        _ => None,
    }
}

/// A fixed-size key for one id in one namespace: the id itself is never kept.
type Key = [u8; 16];

fn key(parts: &[&str]) -> Key {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    let digest = hash.finalize();
    let mut key = [0u8; 16];
    key.copy_from_slice(&digest[..16]);
    key
}

/// The run's id budget, shared by every set.
#[derive(Default)]
struct Budget {
    used: u64,
    over: bool,
}

impl Budget {
    /// Whether one more id may be kept; past the budget it records that de-dup stopped.
    fn take(&mut self) -> bool {
        if self.used >= MAX_IDS {
            self.over = true;
            return false;
        }
        self.used += 1;
        true
    }
}

#[derive(Default)]
struct Ids(HashSet<Key>);

impl Ids {
    /// Whether `key` is new. Past the budget every key is new: counting goes on, de-dup stops.
    fn insert(&mut self, key: Key, budget: &mut Budget) -> bool {
        if self.0.contains(&key) {
            return false;
        }
        if budget.take() {
            self.0.insert(key);
        }
        true
    }

    fn len(&self) -> u64 {
        self.0.len() as u64
    }
}

fn supplied_id(observation: &Observation) -> Option<&str> {
    observation
        .supplied_id
        .as_deref()
        .filter(|id| !id.is_empty())
}

fn field<'a>(observation: &'a Observation, name: &str) -> Option<&'a Value> {
    observation.fields.get(name)
}

fn field_str<'a>(observation: &'a Observation, name: &str) -> Option<&'a str> {
    field(observation, name).and_then(Value::as_str)
}

fn count(value: Option<&Value>) -> Option<u64> {
    value.and_then(Value::as_u64)
}

/// The supplied tool call id of a `tool_call` or `tool_result`: the `call_id` field where a
/// decoder supplies one (the OpenAI wire's own id is the item's), else the supplied id. Native
/// and wire share these ids (`toolu_…`, `call_…`), so one set serves both.
fn tool_id(observation: &Observation) -> Option<&str> {
    field_str(observation, "call_id")
        .filter(|id| !id.is_empty())
        .or_else(|| supplied_id(observation))
}

// ---------------------------------------------------------------------------------------
// Tokens.
// ---------------------------------------------------------------------------------------

/// The five token counts, in one order everywhere.
const INPUT: usize = 0;
const CACHE_READ: usize = 1;
const CACHE_WRITE: usize = 2;
const OUTPUT: usize = 3;
const REASONING: usize = 4;

/// One usage snapshot, normalized: a count the source does not state is `None`.
type Snapshot = [Option<u64>; 5];

/// One response's counted usage: the field-wise maximum of its snapshots, so a cumulative
/// snapshot and a repeated one both count once.
#[derive(Default)]
struct Counted {
    counts: [u64; 5],
    usage: bool,
}

#[derive(Default)]
struct Tokens {
    responses: HashMap<Key, Counted>,
    totals: [u64; 5],
    reasoning_stated: bool,
    /// Responses with usage.
    with_usage: u64,
}

impl Tokens {
    /// A response was seen, with or without usage.
    fn response(&mut self, key: Key, budget: &mut Budget) {
        if !self.responses.contains_key(&key) && budget.take() {
            self.responses.insert(key, Counted::default());
        }
    }

    /// One snapshot of the response `key` (`None`: a response of its own).
    fn usage(&mut self, key: Option<Key>, snapshot: Snapshot, budget: &mut Budget) {
        if snapshot[REASONING].is_some() {
            self.reasoning_stated = true;
        }
        let kept = match key {
            Some(key) if self.responses.contains_key(&key) => self.responses.get_mut(&key),
            Some(key) if budget.take() => Some(self.responses.entry(key).or_default()),
            _ => None,
        };
        let mut fresh = Counted::default();
        let counted = kept.unwrap_or(&mut fresh);
        if !counted.usage {
            counted.usage = true;
            self.with_usage = self.with_usage.saturating_add(1);
        }
        for (i, value) in snapshot.iter().enumerate() {
            if let Some(value) = *value
                && value > counted.counts[i]
            {
                self.totals[i] = self.totals[i].saturating_add(value - counted.counts[i]);
                counted.counts[i] = value;
            }
        }
    }

    fn without_usage(&self) -> u64 {
        self.responses.values().filter(|c| !c.usage).count() as u64
    }

    fn has_usage(&self) -> bool {
        self.with_usage > 0
    }

    fn document(&self, source: &str) -> Value {
        json!({
            "source": source,
            "responses": self.with_usage,
            "responses_without_usage": self.without_usage(),
            "input": self.totals[INPUT],
            "cache_read": self.totals[CACHE_READ],
            "cache_write": self.totals[CACHE_WRITE],
            "output": self.totals[OUTPUT],
            "reasoning": self.reasoning_stated.then_some(self.totals[REASONING]),
        })
    }
}

/// A usage object's normalized counts. `input` is input not served from cache.
fn snapshot(input: Option<u64>, cached: Option<u64>, rest: [Option<u64>; 3]) -> Snapshot {
    let [cache_write, output, reasoning] = rest;
    [
        input.map(|input| input.saturating_sub(cached.unwrap_or(0))),
        cached,
        cache_write,
        output,
        reasoning,
    ]
}

/// The normalized snapshot of a `usage` observation, and the key of the response it belongs to
/// (`None` with `Some` snapshot: an id-less response of its own). `None` for a usage the
/// rules do not count.
fn usage_of(decoder: &str, o: &Observation) -> Option<(Option<String>, Snapshot)> {
    let id = supplied_id(o).map(str::to_string);
    match decoder {
        anthropic::DECODER_ID => {
            let usage = field(o, "usage")?;
            let get = |name: &str| count(usage.get(name));
            match field_str(o, "scope") {
                // Input and cache from the start; output from the last cumulative delta.
                Some("message_start") => Some((
                    id,
                    [
                        get("input_tokens"),
                        get("cache_read_input_tokens"),
                        get("cache_creation_input_tokens"),
                        get("output_tokens"),
                        None,
                    ],
                )),
                // A delta without an id cannot be joined to its start: never a new response.
                Some("message_delta") if id.is_some() => {
                    Some((id, [None, None, None, get("output_tokens"), None]))
                }
                _ => None,
            }
        }
        openai::DECODER_ID => {
            let details = |name: &str, member: &str| count(field(o, name)?.get(member));
            Some((
                id,
                snapshot(
                    count(field(o, "input_tokens")),
                    details("input_tokens_details", "cached_tokens"),
                    [
                        None,
                        count(field(o, "output_tokens")),
                        details("output_tokens_details", "reasoning_tokens"),
                    ],
                ),
            ))
        }
        chat::DECODER_ID => {
            let details = |name: &str, member: &str| count(field(o, name)?.get(member));
            let cached = details("prompt_tokens_details", "cached_tokens")
                .or_else(|| count(field(o, "prompt_cache_hit_tokens")));
            Some((
                id,
                snapshot(
                    count(field(o, "prompt_tokens")),
                    cached,
                    [
                        None,
                        count(field(o, "completion_tokens")),
                        details("completion_tokens_details", "reasoning_tokens"),
                    ],
                ),
            ))
        }
        thread::DECODER_ID => Some((
            id,
            [
                count(field(o, "inputTokens")),
                count(field(o, "cacheReadInputTokens")),
                count(field(o, "cacheCreationInputTokens")),
                count(field(o, "outputTokens")),
                None,
            ],
        )),
        ClaudeCode::ID => Some((
            id,
            [
                count(field(o, "input_tokens")),
                count(field(o, "cache_read_input_tokens")),
                count(field(o, "cache_creation_input_tokens")),
                count(field(o, "output_tokens")),
                None,
            ],
        )),
        // Only the session's cumulative total; per-call deltas repeat and are never summed.
        Codex::ID if field_str(o, "scope") == Some("cumulative") => {
            let total = field(o, "total_token_usage")?;
            let get = |name: &str| count(total.get(name));
            Some((
                Some(codex_session(o)),
                snapshot(
                    get("input_tokens"),
                    get("cached_input_tokens"),
                    [None, get("output_tokens"), get("reasoning_output_tokens")],
                ),
            ))
        }
        _ => None,
    }
}

/// A Codex observation's session: its own supplied session id, else its stream.
fn codex_session(o: &Observation) -> String {
    match o
        .fields
        .get("native_context")
        .and_then(|c| c.get("session_id"))
        .and_then(Value::as_str)
    {
        Some(id) => format!("session:{id}"),
        None => format!(
            "stream:{}:{}",
            o.source.stream.producer, o.source.stream.stream
        ),
    }
}

/// A Codex child or fork stream: its totals may include its parent's context (unmeasured).
fn codex_child(o: &Observation) -> bool {
    o.fields
        .get("native_context")
        .and_then(|c| c.get("history_scope"))
        .is_some()
}

/// The id of the response an observation shows was produced, for counting responses that
/// carried no usage.
fn response_of<'a>(decoder: &str, o: &'a Observation) -> Option<&'a str> {
    let response = match (decoder, o.kind.as_str()) {
        (anthropic::DECODER_ID | chat::DECODER_ID, kinds::MODEL_RESPONSE) => true,
        (openai::DECODER_ID, kinds::MODEL_RESPONSE) => {
            field_str(o, "event") == Some("response.created")
        }
        (thread::DECODER_ID, kinds::MODEL_RESPONSE) => field_str(o, "role") == Some("assistant"),
        (ClaudeCode::ID, kinds::MODEL) => field_str(o, "via") == Some("message"),
        _ => false,
    };
    response.then(|| supplied_id(o)).flatten()
}

/// The model a response used: only response-scoped `model` observations, never a request's
/// or a switch.
fn response_model<'a>(decoder: &str, o: &'a Observation) -> Option<&'a str> {
    let scoped = match decoder {
        anthropic::DECODER_ID => field_str(o, "scope") == Some("message_start"),
        openai::DECODER_ID | chat::DECODER_ID => field_str(o, "side") == Some("response"),
        thread::DECODER_ID => true,
        ClaudeCode::ID => field_str(o, "via") == Some("message"),
        Codex::ID => field_str(o, "via") == Some("turn_context"),
        _ => false,
    };
    if scoped { field_str(o, "model") } else { None }
}

// ---------------------------------------------------------------------------------------
// The per-source counts.
// ---------------------------------------------------------------------------------------

#[derive(Default)]
struct Bounded {
    counts: BTreeMap<String, u64>,
    omitted: u64,
}

impl Bounded {
    fn add(&mut self, name: Option<&str>, cap: usize) {
        match name {
            Some(name) if self.counts.contains_key(name) => {
                *self.counts.get_mut(name).expect("present") += 1;
            }
            Some(name) if self.counts.len() < cap => {
                self.counts.insert(name.to_string(), 1);
            }
            _ => self.omitted = self.omitted.saturating_add(1),
        }
    }

    /// The `limit` largest, by count then name, and the count of everything else.
    fn top(&self, limit: usize) -> (Vec<(&str, u64)>, u64) {
        let mut sorted: Vec<(&str, u64)> =
            self.counts.iter().map(|(n, c)| (n.as_str(), *c)).collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let rest = sorted
            .iter()
            .skip(limit)
            .fold(self.omitted, |sum, (_, c)| sum.saturating_add(*c));
        sorted.truncate(limit);
        (sorted, rest)
    }
}

/// What one source (native or wire) said, counted once per supplied id.
#[derive(Default)]
struct Side {
    /// Any decoded (not `unknown`) observation came from this source.
    active: bool,
    tokens: Tokens,
    model_ids: Ids,
    models: Bounded,
    error_ids: Ids,
    errors: Bounded,
    error_total: u64,
    calls_without_id: u64,
    subagents_without_id: u64,
}

/// A model name kept on the page: 1 to 64 bytes of printable ASCII.
fn shown_model(name: &str) -> Option<&str> {
    (!name.is_empty()
        && name.len() <= MAX_MODEL_NAME_BYTES
        && name.bytes().all(|b| b.is_ascii_graphic()))
    .then_some(name)
}

/// An error code kept on the page: a short token of letters, digits and `._-`.
fn shown_code(code: &str) -> Option<&str> {
    (!code.is_empty()
        && code.len() <= MAX_CODE_BYTES
        && code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)))
    .then_some(code)
}

/// "keep going", exactly, after trimming, lower-casing and dropping end punctuation.
fn is_keep_going(text: &str) -> bool {
    let trimmed = text
        .trim()
        .trim_end_matches(|c: char| c.is_ascii_punctuation() || c == '…')
        .trim_end();
    trimmed.eq_ignore_ascii_case("keep going")
}

// ---------------------------------------------------------------------------------------
// The run.
// ---------------------------------------------------------------------------------------

/// The counters of one run, over all of its traces.
#[derive(Default)]
pub struct RunStats {
    budget: Budget,
    native: Side,
    wire: Side,
    decoded: bool,
    tool_calls: Ids,
    tool_results: Ids,
    tool_errors: Ids,
    /// Decoders that mark tool errors and emitted tool facts: `is_error`, `exit_code`.
    marks_is_error: bool,
    marks_exit_code: bool,
    subagents: Ids,
    children: Ids,
    human_ids: Ids,
    human_total: u64,
    keep_going: u64,
    human_other: u64,
}

impl RunStats {
    fn side(&mut self, leg: Leg) -> &mut Side {
        match leg {
            Leg::Native => &mut self.native,
            Leg::Wire => &mut self.wire,
        }
    }

    fn read(
        &mut self,
        trace: &Path,
        options: &VerifyOptions,
        registry: &DecoderRegistry,
    ) -> io::Result<TraceRead> {
        let mut session = DecodeSession::new(registry, StatsSink { stats: self });
        let integrity = scan_capture_trace(trace, options, &mut |record| session.push(record))?;
        let (outcome, _) = session.finish()?;
        Ok(TraceRead { integrity, outcome })
    }

    fn observe(&mut self, decoder: &'static str, o: &Observation) {
        let Some(leg) = leg(decoder) else {
            return;
        };
        if o.kind == kinds::UNKNOWN {
            return;
        }
        self.decoded = true;
        let Self {
            budget,
            native,
            wire,
            children,
            ..
        } = self;
        let side = match leg {
            Leg::Native => native,
            Leg::Wire => wire,
        };
        side.active = true;
        if let Some(id) = response_of(decoder, o) {
            side.tokens
                .response(key(&["response", decoder, id]), budget);
        }
        match o.kind.as_str() {
            kinds::USAGE if decoder == Codex::ID && codex_child(o) => {
                children.insert(key(&["child", &codex_session(o)]), budget);
            }
            kinds::USAGE => {
                if let Some((id, snapshot)) = usage_of(decoder, o) {
                    let key = id.map(|id| key(&["response", decoder, &id]));
                    side.tokens.usage(key, snapshot, budget);
                }
            }
            kinds::MODEL => {
                if let Some(model) = response_model(decoder, o) {
                    let new = supplied_id(o).is_none_or(|id| {
                        side.model_ids.insert(key(&["model", decoder, id]), budget)
                    });
                    if new {
                        side.models.add(shown_model(model), MAX_TRACKED_MODELS);
                    }
                }
            }
            kinds::ERROR => self.provider_error(leg, decoder, o),
            kinds::TOOL_CALL => self.tool_call(leg, decoder, o),
            kinds::TOOL_RESULT => self.tool_result(decoder, o),
            kinds::HUMAN_MESSAGE => self.human_message(decoder, o),
            _ => {}
        }
    }

    fn provider_error(&mut self, leg: Leg, decoder: &str, o: &Observation) {
        // The harness's own account of a blocked prompt is not a provider answer.
        if field_str(o, "code") == Some("prompt_blocked_by_hook") {
            return;
        }
        let Self {
            budget,
            native,
            wire,
            ..
        } = self;
        let side = match leg {
            Leg::Native => native,
            Leg::Wire => wire,
        };
        let new = supplied_id(o)
            .is_none_or(|id| side.error_ids.insert(key(&["error", decoder, id]), budget));
        if !new {
            return;
        }
        side.error_total = side.error_total.saturating_add(1);
        let code = ["code", "error_type", "type", "subtype"]
            .iter()
            .find_map(|name| field_str(o, name));
        side.errors.add(
            code.and_then(shown_code).or(Some("other")),
            MAX_TRACKED_CODES,
        );
    }

    fn tool_call(&mut self, leg: Leg, decoder: &str, o: &Observation) {
        self.note_marking(decoder);
        let subagent = match decoder {
            Codex::ID => field_str(o, "item_type") == Some("CollabAgentToolCall"),
            ClaudeCode::ID | anthropic::DECODER_ID | thread::DECODER_ID => {
                matches!(field_str(o, "name"), Some("Task" | "Agent"))
            }
            _ => false,
        };
        match tool_id(o) {
            Some(id) => {
                let id = key(&["tool", id]);
                self.tool_calls.insert(id, &mut self.budget);
                if subagent {
                    self.subagents.insert(id, &mut self.budget);
                }
            }
            None => {
                let side = self.side(leg);
                side.calls_without_id = side.calls_without_id.saturating_add(1);
                if subagent {
                    side.subagents_without_id = side.subagents_without_id.saturating_add(1);
                }
            }
        }
    }

    /// A result is counted once per tool call id, however often a request replays it. One
    /// with no id cannot be told from a replay and is not counted.
    fn tool_result(&mut self, decoder: &str, o: &Observation) {
        self.note_marking(decoder);
        let Some(id) = tool_id(o) else {
            return;
        };
        let id = key(&["tool", id]);
        self.tool_results.insert(id, &mut self.budget);
        let error = match decoder {
            ClaudeCode::ID | anthropic::DECODER_ID => {
                field(o, "is_error").and_then(Value::as_bool) == Some(true)
            }
            thread::DECODER_ID => field(o, "exit_code")
                .and_then(Value::as_i64)
                .is_some_and(|code| code != 0),
            _ => false,
        };
        if error {
            self.tool_errors.insert(id, &mut self.budget);
        }
    }

    fn note_marking(&mut self, decoder: &str) {
        match decoder {
            ClaudeCode::ID | anthropic::DECODER_ID => self.marks_is_error = true,
            thread::DECODER_ID => self.marks_exit_code = true,
            _ => {}
        }
    }

    /// One per supplied id; the first is the prompt, the rest are "keep going" or other.
    fn human_message(&mut self, decoder: &str, o: &Observation) {
        let new = supplied_id(o).is_none_or(|id| {
            self.human_ids
                .insert(key(&["human", decoder, id]), &mut self.budget)
        });
        if !new {
            return;
        }
        self.human_total = self.human_total.saturating_add(1);
        if self.human_total == 1 {
            return;
        }
        if field_str(o, "text").is_some_and(is_keep_going) {
            self.keep_going = self.keep_going.saturating_add(1);
        } else {
            self.human_other = self.human_other.saturating_add(1);
        }
    }

    // -----------------------------------------------------------------------------------
    // The document.
    // -----------------------------------------------------------------------------------

    fn document(&self, reads: &[TraceRead]) -> RunStatsResult {
        let mut degraded_ids = BTreeSet::new();
        let mut degraded_legs = BTreeSet::new();
        let mut traces = Vec::new();
        let mut exit_code = 0u8;
        for read in reads {
            let mut trace_code = read.integrity.exit_code();
            for decoder in read.outcome.decoders.iter().filter(|d| degraded(d)) {
                trace_code = trace_code.max(1);
                degraded_ids.insert(decoder.id);
                if let Some(leg) = leg(decoder.id) {
                    degraded_legs.insert(leg.name());
                }
            }
            exit_code = exit_code.max(trace_code);
            let entry_id = read.integrity.entry_id.as_deref().filter(|id| {
                !id.is_empty()
                    && id.len() <= MAX_ENTRY_ID_BYTES
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
            });
            traces.push(json!({
                "entry_id": entry_id,
                "exit_code": trace_code,
                "integrity": read.integrity.integrity,
                "sealed": read.integrity.sealed,
                "validated_records": read.integrity.validated_records,
                "validated_bytes": read.integrity.validated_bytes,
            }));
        }
        let state = if !self.decoded {
            "unavailable"
        } else if degraded_ids.is_empty() {
            "available"
        } else {
            "partial"
        };

        // Tokens: the wire when it stated usage and no wire decoder degraded, else the native
        // session files, else whatever stated usage, else none.
        let wire_clean = self.wire.tokens.has_usage() && !degraded_legs.contains("wire");
        let source = if wire_clean {
            Some(Leg::Wire)
        } else if self.native.tokens.has_usage() {
            Some(Leg::Native)
        } else if self.wire.tokens.has_usage() {
            Some(Leg::Wire)
        } else {
            None
        };
        let side = |leg: Leg| match leg {
            Leg::Native => &self.native,
            Leg::Wire => &self.wire,
        };
        let mut tokens = match source {
            Some(leg) => {
                let mut tokens = side(leg).tokens.document(leg.name());
                let other = side(leg.other());
                tokens["other_source"] = if other.tokens.has_usage() {
                    other.tokens.document(leg.other().name())
                } else {
                    Value::Null
                };
                tokens
            }
            None => json!({
                "source": "none", "responses": null, "responses_without_usage": null,
                "input": null, "cache_read": null, "cache_write": null, "output": null,
                "reasoning": null, "other_source": null,
            }),
        };
        tokens["children_flagged"] = json!(self.children.len());

        // Models, provider errors and id-less calls come from the same source as the tokens;
        // with no usage at all, from the wire if it decoded anything, else the native files.
        let primary = source.unwrap_or(if self.wire.active {
            Leg::Wire
        } else {
            Leg::Native
        });
        let primary = side(primary);
        let (models, models_omitted) = primary.models.top(MAX_LISTED_MODELS);
        let models: Vec<Value> = models
            .into_iter()
            .map(|(model, responses)| json!({"model": model, "responses": responses}))
            .collect();
        let (codes, other_codes) = primary.errors.top(MAX_LISTED_CODES);
        let mut by_code = Map::new();
        for (code, count) in codes {
            by_code.insert(code.to_string(), json!(count));
        }
        if other_codes > 0 {
            let other = by_code.entry("other").or_insert(json!(0));
            *other = json!(other.as_u64().unwrap_or(0).saturating_add(other_codes));
        }

        let error_marking = match (self.marks_is_error, self.marks_exit_code) {
            (true, true) => "both",
            (true, false) => "is_error",
            (false, true) => "exit_code",
            (false, false) => "none",
        };
        let errors = (error_marking != "none").then_some(self.tool_errors.len());
        let human = if self.native.active {
            json!({"source": "native", "total": self.human_total,
                "keep_going": self.keep_going, "other": self.human_other})
        } else {
            json!({"source": "none", "total": null, "keep_going": null, "other": null})
        };

        let mut stats = json!({
            "type": "run_stats",
            "schema_version": RUN_STATS_SCHEMA_VERSION,
            "verifier_version": crate::VERIFIER_VERSION,
            "traces": traces,
            "analysis": {"state": state, "degraded": degraded_ids},
            "tokens": tokens,
            "models": models,
            "models_omitted": models_omitted,
            "tools": {
                "calls": self.tool_calls.len().saturating_add(primary.calls_without_id),
                "calls_without_id": primary.calls_without_id,
                "results": self.tool_results.len(),
                "errors": errors,
                "error_marking": error_marking,
            },
            "subagents": {
                "spawned": self.subagents.len().saturating_add(primary.subagents_without_id),
            },
            "provider_errors": {"total": primary.error_total, "by_code": by_code},
            "human_messages": human,
            "bounds": {"ids_over_budget": self.budget.over},
        });
        stats.sort_all_objects();
        RunStatsResult { exit_code, stats }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_going_is_the_exact_phrase_only() {
        for yes in [
            "keep going",
            "  Keep going.\n",
            "KEEP GOING!!",
            "keep going…",
        ] {
            assert!(is_keep_going(yes), "{yes:?}");
        }
        for no in [
            "keep going and add tests",
            "please keep going",
            "keepgoing",
            "",
        ] {
            assert!(!is_keep_going(no), "{no:?}");
        }
    }
}
