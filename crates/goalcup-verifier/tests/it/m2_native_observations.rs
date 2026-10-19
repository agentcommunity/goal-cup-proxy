//! Behavioral tests for the bounded native-transcript decoders
//! (`crates/goalcup-verifier/src/capture_analysis/native/`).
//!
//! Every fixture under `tests/fixtures/m2-native/` is a curated, already-redacted excerpt of
//! a real, measured native transcript: `codex-native-excerpt.jsonl` from
//! `spikes/codex-cli/fixtures/rollout-main-01a06614.jsonl` (Codex CLI 0.153.0, sha256
//! `04441b2e0d18be764df692d1445c5b421fbe8fee6285834b6cad653e89238c1a`, lines at 0-based
//! ordinals 2, 5, 7, 8, 9, 12, 15, 16, 27, 42, 43, plus one more at ordinal 0 — the real
//! `session_meta` record, whose `payload.source == "cli"`
//! (a TUI launch, not `exec`) — and `claude-code-native-excerpt.jsonl` from
//! `spikes/claude-code-cli/fixtures/main-session.jsonl` (Claude Code CLI 2.1.259, sha256
//! `832f5d6f97b88d18e5738eb04e56f12e9e4d9553dd66c3d185ee9e156f8d244b`, lines at 0-based
//! ordinals 0, 5, 6, 7, 24, 26, 38, 39, 142, plus two more at
//! ordinals 55 and 95 — a real background sub-agent task notification
//! (`promptSource: "system"`, `origin.kind: "task-notification"`) and a real replayed
//! continuation (`isMeta: true`) — both real `type: "user"` records a naive positional rule
//! would misclassify as human. Both source files are committed, sanitized spike fixtures.
//! A handful of tool/model text fields were shortened in place (structure and every field
//! this decoder reads left unchanged) to keep the excerpts focused, the same way
//! `history.jsonl`'s committed fixture already carries a `<truncated:N chars total>` marker
//! for an oversized prompt.
//!
//! `codex-native-e01-excerpt.jsonl` and `claude-code-native-e01-excerpt.jsonl` are
//! curated from the real, git-ignored E01 raw native chains at
//! `m2-e01-live/spikes/m2-first-release/out/early/live/*/trace/chain.jsonl`, decoded from
//! their `Capture`/`Open`/`Segment` records: `codex-native-e01-excerpt.jsonl` from
//! `codex-20260928T032110Z/trace/chain.jsonl` (Codex CLI 0.153.0, `exec` launch, chain
//! sha256 `8344bcb765127762a764259872a3c400f417921d2ada94308aaf0d427475c330`, native-stream
//! JSONL lines at 0-based ordinals 0 and 6) and `claude-code-native-e01-excerpt.jsonl` from
//! `claude-20260928T032648Z/trace/chain.jsonl` (Claude Code CLI 2.1.283, sdk launch, chain
//! sha256 `bb20b7c6ddeba4cc3a7e5215eee769ccd83719044b7df25b407ed9d7fc4bdacf`, lines at
//! 0-based ordinals 2, 4, 17, 19, 29) plus one `system.api_error` line from the separate
//! `claude-20260928T032205Z/trace/chain.jsonl` (same version, chain sha256
//! `46731498355909078d254a90cdb4798424767c71d21f6bd6ad5257d37ae6bcd0`, ordinal 17). Both
//! excerpts have a few bulky irrelevant text fields (thinking-block signature, tool
//! input/output bodies, `base_instructions`) shortened in place, every field either
//! decoder reads left unchanged.
//!
//! **Identifier placeholders.** Every real session id, message/turn/
//! window UUID, `msg_`/`req_`/`toolu_`-prefixed API id and `/Users/.../workspace` path in
//! those two E01 excerpts was replaced by a deterministic placeholder, applied in one pass
//! across both files so the same original value always maps to the same placeholder
//! (preserving cross-record structure — e.g. the goal's `promptId` and its tool-result
//! continuation's matching `promptId` are equal, as the same placeholder). The
//! rule, not the original values: a bare UUID-shaped token becomes
//! `00000000-0000-4000-8000-<hex counter>`; a `msg_`/`req_`/`toolu_`/`ctc_`/`ctco_`/`fco_`
//! id keeps its real prefix (it names the record type, not an identity) with its opaque
//! suffix replaced by `PLACEHOLDER<counter>`; an E01 workspace path becomes
//! `/Users/placeholder/e01-workspace-<counter>`. The three source chains' sha256 above are
//! the raw source's, kept for provenance; only the derived excerpt files were rewritten.

use crate::common as support;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use goalcup_protocol::capture::{
    CaptureMeta, CaptureRecord, CaptureSource, CaptureTime, EndReason, GapReason, StreamKey,
};
use goalcup_protocol::redaction::{RedactionAudit, RedactionAuditSpan};
use goalcup_verifier::capture_analysis::api::{DecoderSummary, DecoderV1, Observation};
use goalcup_verifier::capture_analysis::native::{ClaudeNativeDecoder, CodexNativeDecoder};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const CODEX_FIXTURE: &[u8] = include_bytes!("../fixtures/m2-native/codex-native-excerpt.jsonl");
const CLAUDE_FIXTURE: &[u8] =
    include_bytes!("../fixtures/m2-native/claude-code-native-excerpt.jsonl");
const CODEX_E01_FIXTURE: &[u8] =
    include_bytes!("../fixtures/m2-native/codex-native-e01-excerpt.jsonl");
const CLAUDE_E01_FIXTURE: &[u8] =
    include_bytes!("../fixtures/m2-native/claude-code-native-e01-excerpt.jsonl");

/// A fresh random-looking 128-bit hex producer id.
const PRODUCER: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90";
const STREAM: u64 = 1;

// --------------------------------------------------------------------------- fixture helpers

fn stream_key(producer: &str, stream: u64) -> StreamKey {
    StreamKey {
        producer: producer.to_string(),
        stream,
    }
}

/// A deterministic `CaptureTime` keyed on absolute byte offset (not part index): the same
/// byte offset always yields the same time regardless of how many Segments the bytes around
/// it were chopped into, which is exactly the invariant `split_every_byte_matches_*` needs.
fn byte_time(clock: &str, offset: usize) -> CaptureTime {
    CaptureTime {
        wall: "2026-09-28T00:00:00Z".to_string(),
        mono_ns: 1_000_000 + offset as u64,
        clock_id: clock.to_string(),
    }
}

fn open_native(producer: &str, stream: u64, hint: &str) -> CaptureRecord {
    CaptureRecord::Open {
        stream: stream_key(producer, stream),
        observed: byte_time(producer, 0),
        source: CaptureSource::Native,
        metadata: CaptureMeta {
            wire: None,
            label: format!("{hint}#g1"),
            protocol_hint: Some(hint.to_string()),
            method: None,
            host: None,
            path: None,
            status: None,
            content_type: None,
        },
    }
}

fn end_native(producer: &str, stream: u64, reason: EndReason) -> CaptureRecord {
    CaptureRecord::End {
        stream: stream_key(producer, stream),
        observed: byte_time(producer, 999_999_999),
        reason,
    }
}

/// Wrap `bytes` on one stream as consecutive `Segment` records split at `split_points`
/// (arbitrary byte offsets strictly between 0 and `bytes.len()`; out-of-range or duplicate
/// values are harmless). Each segment's `first`/`last` come from [`byte_time`] on its own
/// absolute start/end offset, and no segment carries a redaction audit.
fn segment_bytes(
    producer: &str,
    stream: u64,
    bytes: &[u8],
    split_points: &[usize],
) -> Vec<CaptureRecord> {
    let mut bounds = vec![0usize];
    bounds.extend(
        split_points
            .iter()
            .copied()
            .filter(|p| *p > 0 && *p < bytes.len()),
    );
    bounds.push(bytes.len());
    bounds.sort_unstable();
    bounds.dedup();
    let mut records = Vec::new();
    for (part, w) in bounds.windows(2).enumerate() {
        let (start, end) = (w[0], w[1]);
        let chunk = &bytes[start..end];
        records.push(CaptureRecord::Segment {
            stream: stream_key(producer, stream),
            part: part as u64,
            source_offset: start as u64,
            source_len: chunk.len() as u64,
            first: byte_time(producer, start),
            last: byte_time(producer, end),
            data_b64: STANDARD.encode(chunk),
            audit: RedactionAudit { spans: vec![] },
        });
    }
    records
}

/// Reads one fixture's whole bytes and wraps them as one native
/// stream's Segments (`PRODUCER`/`STREAM`) at the given split points.
fn segment_native(fixture: &[u8], split_points: &[usize]) -> Vec<CaptureRecord> {
    segment_bytes(PRODUCER, STREAM, fixture, split_points)
}

fn codex() -> Box<dyn DecoderV1> {
    Box::new(CodexNativeDecoder::new())
}

fn claude() -> Box<dyn DecoderV1> {
    Box::new(ClaudeNativeDecoder::new())
}

/// Feeds the records to a decoder through a session, returning every emitted observation in
/// order plus the final summary.
fn collect(
    make: fn() -> Box<dyn DecoderV1>,
    records: &[CaptureRecord],
) -> (Vec<Observation>, DecoderSummary) {
    support::decode::decode_one(make, records)
}

/// Byte offsets right after every newline in `bytes`, excluding one that would fall at (or
/// past) the very end — one Segment per source line, never a trailing empty one.
fn per_line_splits(bytes: &[u8]) -> Vec<usize> {
    bytes
        .iter()
        .enumerate()
        .filter_map(|(i, &b)| (b == b'\n' && i + 1 < bytes.len()).then_some(i + 1))
        .collect()
}

/// The byte range `[start, end)` of the `n`th (0-based) newline-delimited line in `bytes`,
/// end exclusive of its own trailing newline — matching exactly what the decoder hands its
/// JSON parser.
fn nth_line_range(bytes: &[u8], n: usize) -> (usize, usize) {
    let mut start = 0usize;
    let mut count = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            if count == n {
                return (start, i);
            }
            start = i + 1;
            count += 1;
        }
    }
    panic!("fixture has fewer than {} lines", n + 1);
}

fn line_bytes(fixture: &[u8], n: usize) -> &[u8] {
    let (start, end) = nth_line_range(fixture, n);
    &fixture[start..end]
}

/// One line, with its own trailing newline restored, ready to be handed to `segment_bytes`
/// on its own.
fn line_with_newline(fixture: &[u8], n: usize) -> Vec<u8> {
    let mut bytes = line_bytes(fixture, n).to_vec();
    bytes.push(b'\n');
    bytes
}

fn find(obs: &[Observation], pred: impl Fn(&Observation) -> bool) -> &Observation {
    obs.iter()
        .find(|o| pred(o))
        .unwrap_or_else(|| panic!("no matching observation among {obs:#?}"))
}

fn field_keys(obs: &Observation) -> BTreeSet<&str> {
    obs.fields
        .as_object()
        .expect("every emitted fields value is an object")
        .keys()
        .map(String::as_str)
        .collect()
}

/// Byte offsets `chunk, 2*chunk, ...` below `len`, for splitting a large synthetic payload
/// into segment-sized pieces without tripping the per-segment defensive bound.
fn chunked_splits(len: usize, chunk: usize) -> Vec<usize> {
    (chunk..len).step_by(chunk).collect()
}

// -------------------------------------------------------------------------------- Codex tests

#[test]
fn codex_tool_and_usage_keep_native_references() {
    let mut records = vec![open_native(PRODUCER, STREAM, "codex-jsonl")];
    records.extend(segment_native(
        CODEX_FIXTURE,
        &per_line_splits(CODEX_FIXTURE),
    ));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, summary) = collect(codex, &records);

    let call = find(&obs, |o| o.kind == "tool_call");
    assert_eq!(
        call.supplied_id.as_deref(),
        Some("call_9xcEaQScXwe5VvgIZIp4wfNA")
    );
    assert_eq!(call.fields["name"], "exec");
    assert_eq!(call.fields["status"], "completed");
    // One line == one segment under per-line splitting: the tool_call is wholly inside one part.
    assert_eq!(call.source.first_part, call.source.last_part);

    let result = find(&obs, |o| o.kind == "tool_result");
    assert_eq!(
        result.supplied_id.as_deref(),
        Some("call_9xcEaQScXwe5VvgIZIp4wfNA")
    );

    let per_call = find(&obs, |o| {
        o.kind == "usage"
            && o.fields["scope"] == "per_call"
            && o.fields["last_token_usage"]["input_tokens"] == 14627
    });
    assert_eq!(per_call.fields["last_token_usage"]["output_tokens"], 127);
    assert_eq!(per_call.fields["model_context_window"], 258400);

    let cumulative = find(&obs, |o| {
        o.kind == "usage"
            && o.fields["scope"] == "cumulative"
            && o.fields["total_token_usage"]["input_tokens"] == 14627
    });
    assert_eq!(cumulative.fields["total_token_usage"]["output_tokens"], 127);

    assert_eq!(summary.decoder_id, "codex-native");
    assert_eq!(summary.decoder_version, "2");
    assert!(!summary.incomplete);
}

#[test]
fn native_split_record_preserves_observation_interval() {
    // A `Segment`'s `first`/`last` describe the whole segment, not a byte within it, so the
    // only way to see the token_count line get a first/last distinct from its neighbours is
    // to isolate its own bytes onto their own segments (a segment shared with earlier lines
    // would legitimately carry an earlier `first`, exactly as a real recorder's would). Split
    // exactly in the middle of the isolated line: the resulting observation must still cover
    // the whole line's byte range, first through last, and its supplied fields are unaffected.
    let line = line_with_newline(CODEX_FIXTURE, 7); // the first token_count line, standalone
    let mid = line.len() / 2;
    let mut records = vec![open_native(PRODUCER, STREAM, "codex-jsonl")];
    records.extend(segment_native(&line, &[mid]));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, _summary) = collect(codex, &records);

    let per_call = find(&obs, |o| {
        o.kind == "usage"
            && o.fields["scope"] == "per_call"
            && o.fields["last_token_usage"]["input_tokens"] == 14627
    });
    assert_eq!(per_call.observed.first, byte_time(PRODUCER, 0));
    assert_eq!(per_call.observed.last, byte_time(PRODUCER, line.len()));
    assert!(per_call.source.first_part < per_call.source.last_part);
    // The record's own supplied timestamp is unchanged by the split: the segment-derived
    // `observed` interval above and this source-supplied fact are separate and both intact.
    assert_eq!(
        per_call.fields["source_timestamp"],
        "2026-09-03T07:04:03.455Z"
    );
}

#[test]
fn split_every_byte_matches_unsplit_observations() {
    fn run(
        splits: &[usize],
    ) -> Vec<(
        String,
        Option<String>,
        Value,
        goalcup_verifier::capture_analysis::api::ObservationTimeRange,
    )> {
        let mut records = vec![open_native(PRODUCER, STREAM, "codex-jsonl")];
        records.extend(segment_native(CODEX_FIXTURE, splits));
        records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
        let (obs, _summary) = collect(codex, &records);
        obs.into_iter()
            .map(|mut o| {
                if let Some(context) = o.fields.get_mut("native_context") {
                    // Nested identity provenance also follows actual segment boundaries.
                    let start: usize = CODEX_FIXTURE
                        .split_inclusive(|b| *b == b'\n')
                        .take(11)
                        .map(|line| line.len())
                        .sum();
                    let end = CODEX_FIXTURE.len() - 1; // the terminating newline also supplies a segment
                    let source = context["header_source"].as_object_mut().unwrap();
                    assert_eq!(
                        source["first_part"],
                        splits.iter().filter(|&&at| at <= start).count()
                    );
                    assert_eq!(
                        source["last_part"],
                        splits.iter().filter(|&&at| at <= end).count()
                    );
                    source.remove("first_part");
                    source.remove("last_part");
                }
                (o.kind, o.supplied_id, o.fields, o.observed)
            })
            .collect()
    }
    // The baseline is one segment per source line (never fewer than one line per segment, so
    // no two lines ever share a segment's declared first/last). Splitting every single byte
    // is a strict refinement of that -- every line boundary is still a split point -- so each
    // line's own first/last must come out identical; only its `source` part range grows finer.
    let baseline = run(&per_line_splits(CODEX_FIXTURE));
    let every_byte: Vec<usize> = (1..CODEX_FIXTURE.len()).collect();
    let over_split = run(&every_byte);
    assert!(!baseline.is_empty());
    assert_eq!(baseline, over_split);
}

#[test]
fn native_outcome_does_not_assert_task_success() {
    let mut records = vec![open_native(PRODUCER, STREAM, "codex-jsonl")];
    records.extend(segment_native(
        CODEX_FIXTURE,
        &per_line_splits(CODEX_FIXTURE),
    ));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, _summary) = collect(codex, &records);

    let outcome = find(&obs, |o| o.kind == "outcome");
    assert_eq!(outcome.fields["scope"], "turn");
    assert_eq!(outcome.fields["duration_ms"], 32550);
    assert_eq!(outcome.fields["time_to_first_token_ms"], 8236);
    let keys = field_keys(outcome);
    for banned in ["success", "result", "status", "passed", "task_success"] {
        assert!(
            !keys.contains(banned),
            "outcome must never assert task success via {banned}"
        );
    }
}

#[test]
fn unknown_native_type_is_counted_without_source_loss() {
    let mut records = vec![open_native(PRODUCER, STREAM, "codex-jsonl")];
    records.extend(segment_native(
        CODEX_FIXTURE,
        &per_line_splits(CODEX_FIXTURE),
    ));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, summary) = collect(codex, &records);

    let unknown = find(&obs, |o| {
        o.kind == "unknown" && o.fields["record_type"] == "event_msg.item_completed"
    });
    assert_eq!(unknown.fields["reason"], "unrecognized");
    assert_eq!(unknown.source.stream.producer, PRODUCER);
    assert_eq!(unknown.source.stream.stream, STREAM);
    assert!(summary.unknown_records >= 1);
}

#[test]
fn cumulative_usage_is_not_summed() {
    let mut records = vec![open_native(PRODUCER, STREAM, "codex-jsonl")];
    records.extend(segment_native(
        CODEX_FIXTURE,
        &per_line_splits(CODEX_FIXTURE),
    ));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, _summary) = collect(codex, &records);

    let cumulative: Vec<&Observation> = obs
        .iter()
        .filter(|o| o.kind == "usage" && o.fields["scope"] == "cumulative")
        .collect();
    assert_eq!(
        cumulative.len(),
        2,
        "two supplied snapshots, two observations"
    );
    assert_eq!(
        cumulative[0].fields["total_token_usage"]["input_tokens"],
        14627
    );
    assert_eq!(
        cumulative[1].fields["total_token_usage"]["input_tokens"],
        30702
    );

    let per_call: Vec<&Observation> = obs
        .iter()
        .filter(|o| o.kind == "usage" && o.fields["scope"] == "per_call")
        .collect();
    assert_eq!(per_call.len(), 2);
    assert_eq!(
        per_call[1].fields["last_token_usage"]["input_tokens"],
        16075
    );
    // The per-call delta and the running total genuinely differ on turn 2 -- nothing here
    // ever adds the two snapshots together into a fabricated 45,329.
    assert_ne!(
        per_call[1].fields["last_token_usage"]["input_tokens"],
        cumulative[1].fields["total_token_usage"]["input_tokens"]
    );
    assert!(
        !obs.iter()
            .any(|o| o.fields.get("input_tokens").and_then(Value::as_u64) == Some(45_329))
    );
}

#[test]
fn oversize_native_record_is_unknown_and_next_record_recovers() {
    // One line whose bytes cross the 1 MiB per-record budget, followed by a real, valid,
    // small line (Codex's own turn_context/model record).
    let padding = "x".repeat(1_100_000);
    let huge_line = format!("{{\"type\":\"padding\",\"blob\":\"{padding}\"}}\n");
    let mut bytes = huge_line.into_bytes();
    let huge_len = bytes.len();
    bytes.extend_from_slice(&line_with_newline(CODEX_FIXTURE, 2)); // turn_context (model)

    // Chop the padded line into realistic ~50 KiB segments so no single Segment alone trips
    // the (much larger) per-segment defensive bound; only the running per-line total does.
    let mut splits: Vec<usize> = (50_000..huge_len).step_by(50_000).collect();
    splits.push(huge_len);
    let mut records = vec![open_native(PRODUCER, STREAM, "codex-jsonl")];
    records.extend(segment_native(&bytes, &splits));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, summary) = collect(codex, &records);

    assert_eq!(obs.len(), 2);
    assert_eq!(obs[0].kind, "unknown");
    assert_eq!(obs[0].fields["reason"], "oversize");
    assert_eq!(obs[1].kind, "model");
    assert_eq!(obs[1].fields["model"], "gpt-5.6-sol");
    assert_eq!(summary.unknown_records, 1);
    assert_eq!(summary.observations, 2);
}

#[test]
fn partial_final_line_is_incomplete() {
    let full = line_bytes(CODEX_FIXTURE, 2); // real turn_context line, no trailing newline
    let half = &full[..full.len() / 2];
    let mut records = vec![open_native(PRODUCER, STREAM, "codex-jsonl")];
    records.extend(segment_native(half, &[]));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, summary) = collect(codex, &records);

    assert_eq!(obs.len(), 1);
    assert_eq!(obs[0].kind, "unknown");
    assert_eq!(obs[0].fields["reason"], "incomplete");
    assert!(summary.incomplete);
    assert_eq!(summary.observations, 1); // the dangling tail is counted, not silently dropped
}

#[test]
fn gap_prevents_false_complete_record() {
    // The un-interrupted bytes of this real line would parse as a valid `model` observation.
    let full = line_bytes(CODEX_FIXTURE, 2);
    let mid = full.len() / 2;
    let prefix = &full[..mid];
    let mut suffix = full[mid..].to_vec();
    suffix.push(b'\n'); // terminated on its own once it arrives, past the gap

    let mut records = vec![open_native(PRODUCER, STREAM, "codex-jsonl")];
    records.extend(segment_native(prefix, &[]));
    records.push(CaptureRecord::Gap {
        stream: Some(stream_key(PRODUCER, STREAM)),
        observed: byte_time(PRODUCER, mid),
        reason: GapReason::IngestLost,
        lost_bytes: None,
    });
    records.extend(segment_native(&suffix, &[]));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, summary) = collect(codex, &records);

    // Never the record the un-interrupted bytes would have formed.
    assert!(
        !obs.iter()
            .any(|o| o.kind == "model" && o.fields["model"] == "gpt-5.6-sol")
    );
    assert_eq!(obs.len(), 2);
    assert_eq!(obs[0].kind, "unknown");
    assert_eq!(obs[0].fields["reason"], "interrupted");
    assert_eq!(obs[1].kind, "unknown");
    assert_eq!(obs[1].fields["reason"], "malformed");
    assert!(summary.incomplete);
}

#[test]
fn interleaved_native_streams_have_separate_parse_state() {
    let meta = line_with_newline(CODEX_FIXTURE, 11); // the real session_meta, source == "cli"
    let line = line_with_newline(CODEX_FIXTURE, 3); // the real human-text message
    let mid = line.len() / 2;
    let at = meta.len();
    let stream_a = 10;
    let stream_b = 11;

    let seg = |stream: u64, chunk: &[u8], part: u64, offset: usize| CaptureRecord::Segment {
        stream: stream_key(PRODUCER, stream),
        part,
        source_offset: offset as u64,
        source_len: chunk.len() as u64,
        first: byte_time(PRODUCER, offset),
        last: byte_time(PRODUCER, offset + chunk.len()),
        data_b64: STANDARD.encode(chunk),
        audit: RedactionAudit { spans: vec![] },
    };

    let records = vec![
        open_native(PRODUCER, stream_a, "codex-jsonl"),
        open_native(PRODUCER, stream_b, "codex-jsonl"),
        seg(stream_a, &meta, 0, 0),
        seg(stream_b, &meta, 0, 0),
        seg(stream_a, &line[..mid], 1, at),
        seg(stream_b, &line[..mid], 1, at),
        seg(stream_a, &line[mid..], 2, at + mid),
        seg(stream_b, &line[mid..], 2, at + mid),
        end_native(PRODUCER, stream_a, EndReason::Complete),
        end_native(PRODUCER, stream_b, EndReason::Complete),
    ];
    let (obs, summary) = collect(codex, &records);

    let humans: Vec<&Observation> = obs.iter().filter(|o| o.kind == "human_message").collect();
    assert_eq!(humans.len(), 2);
    let streams: BTreeSet<u64> = humans.iter().map(|o| o.source.stream.stream).collect();
    assert_eq!(streams, BTreeSet::from([stream_a, stream_b]));
    assert_eq!(humans[0].fields["text"], humans[1].fields["text"]);
    assert!(!summary.incomplete);
}

#[test]
fn codex_redacted_span_becomes_unknown_not_guessed() {
    // A fail-closed redaction replaced the tail of this line with one marker, breaking its
    // JSON -- exactly the shape `StreamRedactor`'s fail-closed path produces.
    let mut bytes = br#"{"type":"turn_context","payload":{"turn_id":"t1","model":"<redacted:generic_secret_assignment:aaaaaaaa>"#.to_vec();
    bytes.push(b'\n');
    let audit = RedactionAudit {
        spans: vec![RedactionAuditSpan {
            kind: "generic_secret_assignment".to_string(),
            start: 50,
            end: 90,
        }],
    };
    let segment = CaptureRecord::Segment {
        stream: stream_key(PRODUCER, STREAM),
        part: 0,
        source_offset: 0,
        source_len: bytes.len() as u64,
        first: byte_time(PRODUCER, 0),
        last: byte_time(PRODUCER, bytes.len()),
        data_b64: STANDARD.encode(&bytes),
        audit,
    };
    let records = vec![
        open_native(PRODUCER, STREAM, "codex-jsonl"),
        segment,
        end_native(PRODUCER, STREAM, EndReason::Complete),
    ];
    let (obs, summary) = collect(codex, &records);

    assert_eq!(obs.len(), 1);
    assert_eq!(obs[0].kind, "unknown");
    assert_eq!(obs[0].fields["reason"], "redacted");
    assert_eq!(
        obs[0].fields["kinds"],
        serde_json::json!(["generic_secret_assignment"])
    );
    assert_eq!(summary.unknown_records, 1);
}

// --------------------------------------------------------------------------- Claude Code tests

#[test]
fn claude_tool_result_keeps_supplied_id() {
    let mut records = vec![open_native(PRODUCER, STREAM, "claude-code-jsonl")];
    records.extend(segment_bytes(
        PRODUCER,
        STREAM,
        CLAUDE_FIXTURE,
        &per_line_splits(CLAUDE_FIXTURE),
    ));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, summary) = collect(claude, &records);

    let call = find(&obs, |o| o.kind == "tool_call");
    assert_eq!(
        call.supplied_id.as_deref(),
        Some("toolu_018vr4qV7rUZMPwQmXbH5A6i")
    );
    assert_eq!(call.fields["name"], "Bash");

    let result = find(&obs, |o| o.kind == "tool_result");
    assert_eq!(
        result.supplied_id.as_deref(),
        Some("toolu_018vr4qV7rUZMPwQmXbH5A6i")
    );
    assert_eq!(result.fields["is_error"], false);

    let model = find(&obs, |o| o.kind == "model" && o.fields["via"] == "message");
    assert_eq!(model.fields["model"], "claude-opus-5");
    let switch = find(&obs, |o| o.kind == "model" && o.fields["via"] == "switch");
    assert_eq!(switch.fields["model"], "claude-opus-5[1m]");

    let usage = find(&obs, |o| o.kind == "usage");
    assert_eq!(usage.fields["input_tokens"], 2);
    assert_eq!(usage.fields["output_tokens"], 195);
    assert_eq!(usage.fields["cache_read_input_tokens"], 20990);

    assert_eq!(summary.decoder_id, "claude-code-native");
    // Structure measured on 2.1.259; checked against real 2.1.283 E01 bytes.
    assert_eq!(summary.decoder_version, "2.1.259-2.1.283");
    assert!(!summary.incomplete);
}

#[test]
fn native_error_and_outcome_keep_safe_source_fields() {
    let mut records = vec![open_native(PRODUCER, STREAM, "claude-code-jsonl")];
    records.extend(segment_bytes(
        PRODUCER,
        STREAM,
        CLAUDE_FIXTURE,
        &per_line_splits(CLAUDE_FIXTURE),
    ));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, _summary) = collect(claude, &records);

    let error = find(&obs, |o| o.kind == "error");
    assert_eq!(error.fields["code"], "prompt_blocked_by_hook");
    assert_eq!(error.fields["subtype"], "informational");
    // Bounded: no raw content (the long, possibly sensitive original prompt text) leaked.
    assert_eq!(
        field_keys(error),
        BTreeSet::from(["code", "subtype", "format", "source_timestamp"])
    );

    let outcome = find(&obs, |o| {
        o.kind == "outcome" && o.fields["subtype"] == "turn_duration"
    });
    assert_eq!(outcome.fields["duration_ms"], 7056);
    assert_eq!(outcome.fields["message_count"], 22);
    let keys = field_keys(outcome);
    for banned in ["success", "result", "status", "passed", "task_success"] {
        assert!(
            !keys.contains(banned),
            "outcome must never assert task success via {banned}"
        );
    }
}

#[test]
fn claude_redacted_span_becomes_unknown_not_guessed() {
    let mut bytes =
        br#"{"type":"user","message":{"role":"user","content":"<redacted:email:aaaaaaaa>"#.to_vec();
    bytes.push(b'\n');
    let audit = RedactionAudit {
        spans: vec![RedactionAuditSpan {
            kind: "email".to_string(),
            start: 45,
            end: 78,
        }],
    };
    let segment = CaptureRecord::Segment {
        stream: stream_key(PRODUCER, STREAM),
        part: 0,
        source_offset: 0,
        source_len: bytes.len() as u64,
        first: byte_time(PRODUCER, 0),
        last: byte_time(PRODUCER, bytes.len()),
        data_b64: STANDARD.encode(&bytes),
        audit,
    };
    let records = vec![
        open_native(PRODUCER, STREAM, "claude-code-jsonl"),
        segment,
        end_native(PRODUCER, STREAM, EndReason::Complete),
    ];
    let (obs, summary) = collect(claude, &records);

    assert_eq!(obs.len(), 1);
    assert_eq!(obs[0].kind, "unknown");
    assert_eq!(obs[0].fields["reason"], "redacted");
    assert_eq!(obs[0].fields["kinds"], serde_json::json!(["email"]));
    assert_eq!(summary.unknown_records, 1);
}

// --------------------------------------------------------------------- authorship (both decoders)

#[test]
fn no_positive_human_marker_means_no_human_claim() {
    // Codex: an injected <environment_context> user-role message (no "user.text" marker) and
    // a synthetic role="developer" message (not even user role).
    let mut codex_bytes = line_with_newline(CODEX_FIXTURE, 0); // role=developer
    codex_bytes.extend(line_with_newline(CODEX_FIXTURE, 1)); // role=user, environment_context
    let mut records = vec![open_native(PRODUCER, STREAM, "codex-jsonl")];
    records.extend(segment_native(&codex_bytes, &[]));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (codex_obs, _summary) = collect(codex, &records);
    assert!(!codex_obs.iter().any(|o| o.kind == "human_message"));
    assert_eq!(
        codex_obs
            .iter()
            .filter(|o| o.kind == "unknown" && o.fields["record_type"] == "response_item.message")
            .count(),
        2
    );

    // Claude Code: a tool-result continuation of type "user" carries no `origin` field at all.
    let claude_bytes = line_with_newline(CLAUDE_FIXTURE, 5);
    let mut crecords = vec![open_native(PRODUCER, 2, "claude-code-jsonl")];
    crecords.extend(segment_bytes(PRODUCER, 2, &claude_bytes, &[]));
    crecords.push(end_native(PRODUCER, 2, EndReason::Complete));
    let (claude_obs, _summary) = collect(claude, &crecords);
    assert!(!claude_obs.iter().any(|o| o.kind == "human_message"));
    assert!(claude_obs.iter().any(|o| o.kind == "tool_result")); // recognized, just not human
}

#[test]
fn measured_positive_human_marker_emits_evidence() {
    // Codex: a real "user.text"-marked message in a session whose real session_meta declares
    // `source == "cli"` (a TUI launch).
    let mut codex_bytes = line_with_newline(CODEX_FIXTURE, 11); // session_meta, source=="cli"
    codex_bytes.extend(line_with_newline(CODEX_FIXTURE, 3)); // content_item_kinds: ["user.text"]
    let mut records = vec![open_native(PRODUCER, STREAM, "codex-jsonl")];
    records.extend(segment_native(&codex_bytes, &[]));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, _summary) = collect(codex, &records);
    let human = find(&obs, |o| o.kind == "human_message");
    assert_eq!(
        human.fields["text"],
        "create hello.py that prints the current date, run it, then tell me the output"
    );

    let claude_line = line_with_newline(CLAUDE_FIXTURE, 1); // origin.kind == "human"
    let mut crecords = vec![open_native(PRODUCER, 2, "claude-code-jsonl")];
    crecords.extend(segment_bytes(PRODUCER, 2, &claude_line, &[]));
    crecords.push(end_native(PRODUCER, 2, EndReason::Complete));
    let (cobs, _summary) = collect(claude, &crecords);
    let chuman = find(&cobs, |o| o.kind == "human_message");
    assert_eq!(
        chuman.fields["text"],
        "create hello.py that prints the current date, run it, then tell me the output"
    );
    assert_eq!(
        chuman.supplied_id.as_deref(),
        Some("58740b59-3dfe-46fb-8704-7d049d21a7ed")
    );
}

// ----------------------------------------------------------------------------- bounds/scoping

#[test]
fn valid_empty_native_stream_yields_nothing() {
    let records = vec![
        open_native(PRODUCER, STREAM, "codex-jsonl"),
        end_native(PRODUCER, STREAM, EndReason::Complete),
    ];
    let (obs, summary) = collect(codex, &records);
    assert!(obs.is_empty());
    assert_eq!(summary.observations, 0);
    assert_eq!(summary.unknown_records, 0);
    assert!(!summary.incomplete);
}

#[test]
fn unrelated_hint_and_source_are_never_touched() {
    let unrelated_open = CaptureRecord::Open {
        stream: stream_key(PRODUCER, STREAM),
        observed: byte_time(PRODUCER, 0),
        source: CaptureSource::Native,
        metadata: CaptureMeta {
            wire: None,
            label: "other".to_string(),
            protocol_hint: Some("some-other-jsonl".to_string()),
            method: None,
            host: None,
            path: None,
            status: None,
            content_type: None,
        },
    };
    let segment = CaptureRecord::Segment {
        stream: stream_key(PRODUCER, STREAM),
        part: 0,
        source_offset: 0,
        source_len: CODEX_FIXTURE.len() as u64,
        first: byte_time(PRODUCER, 0),
        last: byte_time(PRODUCER, CODEX_FIXTURE.len()),
        data_b64: STANDARD.encode(CODEX_FIXTURE),
        audit: RedactionAudit { spans: vec![] },
    };
    let records = vec![
        unrelated_open,
        segment,
        end_native(PRODUCER, STREAM, EndReason::Complete),
    ];
    let (obs, summary) = collect(codex, &records);
    assert!(obs.is_empty());
    assert_eq!(summary.observations, 0);
    assert!(!summary.incomplete);
}

// ------------------------------------------------------------------- stream completeness

#[test]
fn interrupted_empty_stream_is_incomplete_and_distinct_from_valid_empty() {
    // Two streams, both opened and closed with zero Segments ever arriving: one cleanly, one
    // interrupted. Both yield zero observations either way -- there is no SourceRef without a
    // contributing part -- but only the interrupted one is `incomplete`.
    let valid = vec![
        open_native(PRODUCER, 1, "codex-jsonl"),
        end_native(PRODUCER, 1, EndReason::Complete),
    ];
    let (valid_obs, valid_summary) = collect(codex, &valid);
    assert!(valid_obs.is_empty());
    assert!(!valid_summary.incomplete);

    let interrupted = vec![
        open_native(PRODUCER, 2, "codex-jsonl"),
        end_native(PRODUCER, 2, EndReason::Interrupted),
    ];
    let (interrupted_obs, interrupted_summary) = collect(codex, &interrupted);
    assert!(interrupted_obs.is_empty());
    assert!(interrupted_summary.incomplete);
}

#[test]
fn interrupted_at_line_boundary_is_incomplete() {
    // A whole, cleanly newline-terminated line, followed by an interrupted End: the common
    // shape of an interrupted JSONL capture (writers flush whole lines). The line still
    // decodes normally; the stream's own coverage is still not proven complete.
    let mut records = vec![open_native(PRODUCER, STREAM, "codex-jsonl")];
    records.extend(segment_native(&line_with_newline(CODEX_FIXTURE, 2), &[])); // turn_context
    records.push(end_native(PRODUCER, STREAM, EndReason::Interrupted));
    let (obs, summary) = collect(codex, &records);

    assert_eq!(obs.len(), 1);
    assert_eq!(obs[0].kind, "model");
    assert!(summary.incomplete);
}

#[test]
fn no_end_at_boundary_is_incomplete() {
    // The recorder died (or this is a live-file EOF): a whole line arrived, but `finish()`
    // runs with no `End` ever having closed the stream. Not an analysis completion claim.
    let mut records = vec![open_native(PRODUCER, STREAM, "codex-jsonl")];
    records.extend(segment_native(&line_with_newline(CODEX_FIXTURE, 2), &[]));
    // No End pushed at all.
    let (obs, summary) = collect(codex, &records);

    assert_eq!(obs.len(), 1);
    assert_eq!(obs[0].kind, "model");
    assert!(summary.incomplete);
}

#[test]
fn gap_at_line_boundary_is_incomplete() {
    // A Gap between two otherwise-whole, individually valid lines: both still decode (a lost
    // record vanishing without trace is still a loss), but the gap alone must mark the
    // summary incomplete -- not only a gap that happens to catch a partial line mid-flight.
    let mut records = vec![open_native(PRODUCER, STREAM, "codex-jsonl")];
    records.extend(segment_native(&line_with_newline(CODEX_FIXTURE, 2), &[])); // turn_context
    records.push(CaptureRecord::Gap {
        stream: Some(stream_key(PRODUCER, STREAM)),
        observed: byte_time(PRODUCER, 1),
        reason: GapReason::SourceChanged,
        lost_bytes: Some(1000),
    });
    records.extend(segment_native(&line_with_newline(CODEX_FIXTURE, 9), &[])); // task_complete
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, summary) = collect(codex, &records);

    assert_eq!(obs.len(), 2);
    assert_eq!(obs.iter().filter(|o| o.kind == "unknown").count(), 0);
    assert!(summary.incomplete);
}

// --------------------------------------------------------- the delivered goal is not human

#[test]
fn codex_exec_goal_is_not_classified_human() {
    // Real E01 bytes: `session_meta.payload.source == "exec"` followed by the first
    // `"user.text"`-marked message on that stream -- the tracker's own delivered goal.
    let mut records = vec![open_native(PRODUCER, STREAM, "codex-jsonl")];
    records.extend(segment_bytes(
        PRODUCER,
        STREAM,
        CODEX_E01_FIXTURE,
        &per_line_splits(CODEX_E01_FIXTURE),
    ));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, _summary) = collect(codex, &records);

    assert!(!obs.iter().any(|o| o.kind == "human_message"));
    let goal = find(&obs, |o| {
        o.kind == "unknown" && o.fields["record_type"] == "response_item.message.goal"
    });
    assert_eq!(goal.fields["reason"], "unrecognized");
}

#[test]
fn codex_later_user_text_after_exec_goal_is_not_claimed_human() {
    // The real E01 session_meta + delivered goal (`source == "exec"`), followed by a
    // synthetic second "user.text"-marked message -- E01 has no real second turn -- shaped
    // like the real one. An `exec` session has no input channel, so no later message in it
    // is claimed human either: position is never authorship.
    let mut bytes = line_with_newline(CODEX_E01_FIXTURE, 0); // session_meta, source=="exec"
    bytes.extend(line_with_newline(CODEX_E01_FIXTURE, 1)); // the real delivered goal
    bytes.extend(
        br#"{"type":"response_item","payload":{"type":"message","id":"msg_second","role":"user","content":[{"type":"input_text","text":"actually, use a different filename"}],"internal_chat_message_metadata_passthrough":{"content_item_kinds":["user.text"]}}}"#,
    );
    bytes.push(b'\n');

    let mut records = vec![open_native(PRODUCER, STREAM, "codex-jsonl")];
    records.extend(segment_native(&bytes, &per_line_splits(&bytes)));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, _summary) = collect(codex, &records);

    assert!(!obs.iter().any(|o| o.kind == "human_message"));
    // One line per part: part 2 is the later message, counted once and never dropped.
    let later: Vec<&Observation> = obs.iter().filter(|o| o.source.first_part == 2).collect();
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].kind, "unknown");
    assert_eq!(later[0].fields["reason"], "unrecognized");
    assert_eq!(later[0].fields["record_type"], "response_item.message");
}

#[test]
fn codex_lost_session_meta_never_claims_human() {
    // The real "user.text"-marked message with no session_meta before it on its stream: the
    // launch kind is unknown, so the marker alone is not proof of a human author.
    let mut records = vec![open_native(PRODUCER, STREAM, "codex-jsonl")];
    records.extend(segment_native(&line_with_newline(CODEX_FIXTURE, 3), &[]));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, summary) = collect(codex, &records);

    assert_eq!(obs.len(), 1);
    assert_eq!(obs[0].kind, "unknown");
    assert_eq!(obs[0].fields["reason"], "unrecognized");
    assert_eq!(obs[0].fields["record_type"], "response_item.message");
    assert_eq!(summary.unknown_records, 1);
}

#[test]
fn claude_sdk_goal_is_not_classified_human() {
    // Real E01 bytes: the sdk-delivered goal carries `promptSource: "sdk"` and no `origin`
    // field at all (2.1.283 has none).
    let mut records = vec![open_native(PRODUCER, STREAM, "claude-code-jsonl")];
    records.extend(segment_native(
        &line_with_newline(CLAUDE_E01_FIXTURE, 0),
        &[],
    ));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, _summary) = collect(claude, &records);

    assert_eq!(obs.len(), 1);
    assert_eq!(obs[0].kind, "unknown");
    assert_eq!(obs[0].fields["reason"], "unrecognized");
    assert_eq!(obs[0].fields["record_type"], "user");
}

#[test]
fn claude_unmarked_later_user_text_is_not_claimed_human() {
    // The real E01 sdk-delivered goal, followed by a synthetic later `type:"user"` record
    // with real text and no `origin`/`promptSource` marker at all -- E01 has no real second
    // turn. Without a positive marker nothing is claimed human, whatever its position.
    let mut bytes = line_with_newline(CLAUDE_E01_FIXTURE, 0);
    bytes.extend(
        br#"{"type":"user","promptId":"p2","message":{"role":"user","content":"actually, use a different filename"},"sessionId":"s"}"#,
    );
    bytes.push(b'\n');

    let mut records = vec![open_native(PRODUCER, STREAM, "claude-code-jsonl")];
    records.extend(segment_native(&bytes, &per_line_splits(&bytes)));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, _summary) = collect(claude, &records);

    assert!(!obs.iter().any(|o| o.kind == "human_message"));
    // One line per part: part 1 is the later record, counted once and never dropped.
    let later: Vec<&Observation> = obs.iter().filter(|o| o.source.first_part == 1).collect();
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].kind, "unknown");
    assert_eq!(later[0].fields["reason"], "unrecognized");
    assert_eq!(later[0].fields["record_type"], "user");
}

#[test]
fn claude_compact_summary_is_not_human() {
    // Synthetic: a compaction summary is harness-written context, so `isCompactSummary`
    // disqualifies the record even when it also carries `origin.kind == "human"`.
    let mut bytes = br#"{"type":"user","promptId":"p4","isCompactSummary":true,"origin":{"kind":"human"},"message":{"role":"user","content":"This session is being continued from a previous conversation."},"sessionId":"s"}"#.to_vec();
    bytes.push(b'\n');

    let mut records = vec![open_native(PRODUCER, STREAM, "claude-code-jsonl")];
    records.extend(segment_native(&bytes, &[]));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, _summary) = collect(claude, &records);

    assert_eq!(obs.len(), 1);
    assert_eq!(obs[0].kind, "unknown");
    assert_eq!(obs[0].fields["reason"], "unrecognized");
    assert_eq!(obs[0].fields["record_type"], "user");
}

// ------------------------------------------------------- checked against real 2.1.283 E01

#[test]
fn claude_2_1_283_e01_structure_decodes_without_human_or_success_claims() {
    let mut records = vec![open_native(PRODUCER, STREAM, "claude-code-jsonl")];
    records.extend(segment_bytes(
        PRODUCER,
        STREAM,
        CLAUDE_E01_FIXTURE,
        &per_line_splits(CLAUDE_E01_FIXTURE),
    ));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, summary) = collect(claude, &records);

    // The sdk-delivered goal is never classified human.
    assert!(!obs.iter().any(|o| o.kind == "human_message"));

    let model = find(&obs, |o| o.kind == "model" && o.fields["via"] == "switch");
    assert_eq!(model.fields["model"], "claude-haiku-4-5-20251001");

    let result = find(&obs, |o| o.kind == "tool_result");
    // The real tool_use_id, replaced by the placeholder substitution:
    // still asserted, since it is the exact linkage fact this record is here to demonstrate.
    assert_eq!(result.supplied_id.as_deref(), Some("toolu_PLACEHOLDER0008"));

    // 2.1.283's real error form: no `turn_duration`/text-prefixed hook-block system record in
    // a headless run, only `subtype: "api_error"`.
    let error = find(&obs, |o| {
        o.kind == "error" && o.fields["subtype"] == "api_error"
    });
    assert_eq!(error.fields["level"], "error");
    assert_eq!(error.fields["code"], "ECONNRESET");
    assert_eq!(error.fields["retry_attempt"], 1);
    assert_eq!(error.fields["max_retries"], 10);
    // Bounded: the raw error.message/error.formatted provider text never leaked.
    let keys = field_keys(error);
    assert!(!keys.contains("message"));
    assert!(!keys.contains("formatted"));

    assert_eq!(summary.decoder_id, "claude-code-native");
    assert_eq!(summary.decoder_version, "2.1.259-2.1.283");
    assert!(!summary.incomplete);
}

// ------------------------------------------------------------------ source timestamps

#[test]
fn source_timestamp_is_preserved_when_supplied() {
    let mut records = vec![open_native(PRODUCER, STREAM, "codex-jsonl")];
    records.extend(segment_native(
        CODEX_FIXTURE,
        &per_line_splits(CODEX_FIXTURE),
    ));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, _summary) = collect(codex, &records);
    let outcome = find(&obs, |o| o.kind == "outcome");
    assert_eq!(
        outcome.fields["source_timestamp"],
        "2026-09-03T07:04:22.835Z"
    );

    let mut crecords = vec![open_native(PRODUCER, STREAM, "claude-code-jsonl")];
    crecords.extend(segment_bytes(
        PRODUCER,
        STREAM,
        CLAUDE_FIXTURE,
        &per_line_splits(CLAUDE_FIXTURE),
    ));
    crecords.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (cobs, _summary) = collect(claude, &crecords);
    let coutcome = find(&cobs, |o| o.kind == "outcome");
    assert_eq!(
        coutcome.fields["source_timestamp"],
        "2026-09-03T07:42:07.524Z"
    );
}

#[test]
fn source_timestamp_absent_stays_absent() {
    // A real E01 `cost-state` record carries no top-level "timestamp" field at all.
    let mut records = vec![open_native(PRODUCER, STREAM, "claude-code-jsonl")];
    records.extend(segment_native(
        &line_with_newline(CLAUDE_E01_FIXTURE, 4),
        &[],
    ));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, _summary) = collect(claude, &records);

    let cost_state = find(&obs, |o| {
        o.kind == "unknown" && o.fields["record_type"] == "cost-state"
    });
    assert!(cost_state.fields.get("source_timestamp").is_none());
}

// ----------------------------------------------------------- the aggregate parser budget

#[test]
fn aggregate_budget_bounds_total_pending_across_streams() {
    // Nine tracked streams. Streams 1..=8 are each fed ~950,000 unterminated bytes (well
    // under the 1 MiB per-line cap alone) and deliberately withheld their own newline, so
    // they all stay pending simultaneously. Stream 9's own content would also fit under 1 MiB
    // alone -- crossing happens only because the other eight streams' bytes are still held,
    // which is what makes this genuinely the aggregate 8 MiB check, not the per-line one
    // (removing just the aggregate check would let this pass).
    const PAD_LEN: usize = 950_000;
    fn padded_line(tag: u64) -> Vec<u8> {
        format!(
            "{{\"type\":\"pad{tag}\",\"blob\":\"{}\"}}\n",
            "a".repeat(PAD_LEN)
        )
        .into_bytes()
    }

    let mut records = Vec::new();
    for s in 1..=9u64 {
        records.push(open_native(PRODUCER, s, "codex-jsonl"));
    }
    let mut finishers: Vec<(u64, Vec<u8>)> = Vec::new();
    for s in 1..=8u64 {
        let full = padded_line(s);
        let body = full[..full.len() - 1].to_vec(); // withhold the trailing newline
        records.extend(segment_bytes(
            PRODUCER,
            s,
            &body,
            &chunked_splits(body.len(), 50_000),
        ));
        finishers.push((s, vec![b'\n']));
    }
    let stream9 = padded_line(9);
    records.extend(segment_bytes(
        PRODUCER,
        9,
        &stream9,
        &chunked_splits(stream9.len(), 50_000),
    ));
    for (s, nl) in finishers {
        records.extend(segment_bytes(PRODUCER, s, &nl, &[]));
    }
    for s in 1..=9u64 {
        records.push(end_native(PRODUCER, s, EndReason::Complete));
    }

    let (obs, _summary) = collect(codex, &records);

    let stream9_obs = obs
        .iter()
        .find(|o| o.source.stream.stream == 9)
        .expect("stream 9 produced an observation");
    assert_eq!(stream9_obs.kind, "unknown");
    assert_eq!(stream9_obs.fields["reason"], "oversize");

    for s in 1..=8u64 {
        let o = obs
            .iter()
            .find(|o| o.source.stream.stream == s)
            .unwrap_or_else(|| panic!("stream {s} still decodes"));
        assert_ne!(
            o.fields.get("reason").and_then(Value::as_str),
            Some("oversize")
        );
    }
}

// --------------------------------------------------------- harness-generated is not human

#[test]
fn harness_generated_user_records_after_sdk_goal_are_never_human() {
    // The real E01 sdk-delivered goal,
    // followed by two real Claude Code 2.1.259 `type: "user"` records that are
    // harness-generated, not human -- a background sub-agent task notification
    // (`promptSource: "system"`, `origin.kind: "task-notification"`, string content) and a
    // replayed continuation (`isMeta: true`, block-array content). Both carry real,
    // non-empty text; a positional rule (goal first => later is human) would misclassify
    // both, since neither carries `origin.kind == "human"`.
    let mut bytes = line_with_newline(CLAUDE_E01_FIXTURE, 0); // the real sdk goal
    bytes.extend(line_with_newline(CLAUDE_FIXTURE, 9)); // real task notification
    bytes.extend(line_with_newline(CLAUDE_FIXTURE, 10)); // real isMeta continuation

    let mut records = vec![open_native(PRODUCER, STREAM, "claude-code-jsonl")];
    records.extend(segment_native(&bytes, &[]));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, _summary) = collect(claude, &records);

    assert!(!obs.iter().any(|o| o.kind == "human_message"));
    // The goal and both records are still counted (structurally parseable), none claimed human.
    assert_eq!(
        obs.iter()
            .filter(|o| o.kind == "unknown" && o.fields["record_type"] == "user")
            .count(),
        3
    );
}

#[test]
fn harness_flags_override_a_positive_marker() {
    // Synthetic: each of the first four records carries a positive marker together with one
    // harness-generated sign, so only the eligibility gate stands between it and a human
    // claim. The last carries `origin.kind: "human"` alone and is the one human record.
    let bytes = [
        r#"{"type":"user","promptId":"m","isMeta":true,"promptSource":"typed","message":{"role":"user","content":"meta"}}"#,
        r#"{"type":"user","promptId":"s","isSidechain":true,"origin":{"kind":"human"},"message":{"role":"user","content":"sidechain"}}"#,
        r#"{"type":"user","promptId":"y","promptSource":"system","origin":{"kind":"human"},"message":{"role":"user","content":"system"}}"#,
        r#"{"type":"user","promptId":"t","origin":{"kind":"task-notification"},"promptSource":"typed","message":{"role":"user","content":"notification"}}"#,
        r#"{"type":"user","promptId":"h","origin":{"kind":"human"},"message":{"role":"user","content":"typed by a person"}}"#,
    ]
    .map(|line| format!("{line}\n"))
    .concat()
    .into_bytes();

    let mut records = vec![open_native(PRODUCER, STREAM, "claude-code-jsonl")];
    records.extend(segment_native(&bytes, &per_line_splits(&bytes)));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, _summary) = collect(claude, &records);

    let humans: Vec<&Observation> = obs.iter().filter(|o| o.kind == "human_message").collect();
    assert_eq!(humans.len(), 1, "{humans:#?}");
    assert_eq!(humans[0].supplied_id.as_deref(), Some("h"));
    assert_eq!(humans[0].fields["text"], "typed by a person");
}

#[test]
fn structurally_eligible_later_message_with_explicit_flags_is_still_human() {
    // The positive half, disclosed synthetic (E01 has no real second turn): a later
    // `type: "user"` record carrying the positive marker `promptSource: "typed"`, with every
    // disqualifying flag explicitly set to its safe value (rather than merely absent), is
    // human. An explicit false is treated identically to absence.
    let mut bytes = line_with_newline(CLAUDE_E01_FIXTURE, 0); // the real sdk goal
    bytes.extend(
        br#"{"type":"user","promptId":"p3","isMeta":false,"isSidechain":false,"promptSource":"typed","message":{"role":"user","content":"actually, use a different filename"},"sessionId":"s"}"#,
    );
    bytes.push(b'\n');

    let mut records = vec![open_native(PRODUCER, STREAM, "claude-code-jsonl")];
    records.extend(segment_native(&bytes, &[]));
    records.push(end_native(PRODUCER, STREAM, EndReason::Complete));
    let (obs, _summary) = collect(claude, &records);

    let human = find(&obs, |o| o.kind == "human_message");
    assert_eq!(human.fields["text"], "actually, use a different filename");
    assert_eq!(human.supplied_id.as_deref(), Some("p3"));
}

// Synthetic structures matching the privately reviewed 0.156.1 fork, without raw run data.
fn codex_rows(rows: &[Value], stream: u64) -> Vec<CaptureRecord> {
    let bytes: Vec<u8> = rows
        .iter()
        .flat_map(|row| {
            let mut line = serde_json::to_vec(row).unwrap();
            line.push(b'\n');
            line
        })
        .collect();
    let mut records = vec![open_native(PRODUCER, stream, "codex-jsonl")];
    records.extend(segment_bytes(
        PRODUCER,
        stream,
        &bytes,
        &per_line_splits(&bytes),
    ));
    records.push(end_native(PRODUCER, stream, EndReason::Complete));
    records
}

fn marked_codex_message(id: &str) -> Value {
    json!({"type":"response_item","payload":{"type":"message","id":id,"role":"user",
        "content":[{"type":"input_text","text":"same supplied text"}],
        "internal_chat_message_metadata_passthrough":{"content_item_kinds":["user.text"]}}})
}

#[test]
fn codex_copied_parent_header_cannot_promote_child_history_or_assignment_to_human() {
    let parent = json!({"type":"session_meta","payload":{"id":"parent","source":"cli"}});
    let child = json!({"type":"session_meta","payload":{"id":"child","session_id":"parent",
        "forked_from_id":"parent","parent_thread_id":"parent","thread_source":"subagent",
        "source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent"}}}}});
    let tool = json!({"type":"response_item","payload":{"type":"function_call","call_id":"same-call","name":"read"}});
    let result = json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"same-call","output":"copied"}});
    let mut records = codex_rows(
        &[
            parent.clone(),
            marked_codex_message("prompt"),
            tool.clone(),
            result.clone(),
        ],
        1,
    );
    records.extend(codex_rows(
        &[
            child,
            parent,
            marked_codex_message("prompt"),
            marked_codex_message("assignment"),
            tool.clone(),
            result.clone(),
            tool,
            result,
            json!({"type":"turn_context","payload":{"model":"child-model"}}),
        ],
        2,
    ));
    let (obs, summary) = collect(codex, &records);
    let humans: Vec<_> = obs.iter().filter(|o| o.kind == "human_message").collect();
    assert_eq!(
        humans.len(),
        1,
        "copied input and agent assignment are not new human input"
    );
    assert_eq!(humans[0].source.stream.stream, 1);
    let model = find(&obs, |o| o.kind == "model");
    assert_eq!(model.fields["native_context"]["session_id"], "child");
    assert_eq!(
        model.fields["native_context"]["parent_session_id"],
        "parent"
    );
    assert_eq!(model.fields["native_context"]["role"], "subagent");
    assert_eq!(
        model.fields["native_context"]["history_scope"],
        "may_include_inherited"
    );
    assert_eq!(
        model.fields["native_context"]["header_source"]["first_part"],
        0
    );
    assert_eq!(
        model.source.first_part, 8,
        "event and identity have distinct provenance"
    );
    assert_eq!(obs.iter().filter(|o| o.kind == "tool_call").count(), 3);
    assert_eq!(obs.iter().filter(|o| o.kind == "tool_result").count(), 3);
    assert_eq!(
        obs.iter()
            .filter(|o| o.source.stream.stream == 2
                && o.kind == "unknown"
                && o.fields["record_type"] == "response_item.message")
            .count(),
        2
    );
    assert!(!summary.incomplete);
}

#[test]
fn codex_first_header_authority_is_immutable_without_requiring_an_identity() {
    for source in ["cli", "vscode"] {
        let records = codex_rows(
            &[
                json!({"type":"session_meta","payload":{"source":source}}),
                marked_codex_message("first"),
                json!({"type":"session_meta","payload":{"id":"resume","source":source}}),
                marked_codex_message("later"),
            ],
            1,
        );
        let (obs, _) = collect(codex, &records);
        assert_eq!(
            obs.iter().filter(|o| o.kind == "human_message").count(),
            2,
            "{source}: ordinary resume keeps its markers"
        );
        assert!(
            obs.iter()
                .all(|o| o.fields["native_context"].get("session_id").is_none())
        );
    }
    for payload in [
        json!(null),
        json!({"source":null}),
        json!({"source":"exec"}),
        json!({"source":"cli","forked_from_id":"parent"}),
        json!({"source":"cli","parent_thread_id":17}),
        json!({"source":"cli","thread_source":"subagent"}),
        json!({"source":"cli","thread_source":"unrecognized"}),
    ] {
        let records = codex_rows(
            &[
                json!({"type":"session_meta","payload":payload}),
                json!({"type":"session_meta","payload":{"source":"cli"}}),
                marked_codex_message("later"),
            ],
            1,
        );
        let (obs, _) = collect(codex, &records);
        assert!(!obs.iter().any(|o| o.kind == "human_message"), "{payload}");
    }
}

#[test]
fn codex_loss_before_header_blocks_promotion_but_known_identity_survives_loss() {
    for before in [true, false] {
        let mut records = codex_rows(
            &[
                json!({"type":"session_meta","payload":{"id":"own","source":"cli"}}),
                marked_codex_message("input"),
            ],
            1,
        );
        records.insert(
            if before { 1 } else { 2 },
            CaptureRecord::Gap {
                stream: Some(stream_key(PRODUCER, 1)),
                observed: byte_time(PRODUCER, 1),
                reason: GapReason::IngestLost,
                lost_bytes: None,
            },
        );
        let (obs, summary) = collect(codex, &records);
        assert_eq!(
            obs.iter().filter(|o| o.kind == "human_message").count(),
            usize::from(!before)
        );
        assert!(
            summary.incomplete,
            "loss remains explicit regardless of identity"
        );
        if !before {
            assert_eq!(
                find(&obs, |o| o.kind == "human_message").fields["native_context"]["session_id"],
                "own"
            );
        }
    }
}

#[test]
fn codex_identity_provenance_is_bounded_and_conflicting_parent_ids_are_not_claimed() {
    let huge = "x".repeat(1024);
    let records = codex_rows(
        &[
            json!({"type":"session_meta","payload":{"id":huge,"source":"cli"}}),
            marked_codex_message("input"),
        ],
        1,
    );
    let (obs, _) = collect(codex, &records);
    let human = find(&obs, |o| o.kind == "human_message");
    assert!(human.fields["native_context"].get("session_id").is_none());
    let records = codex_rows(
        &[
            json!({"type":"session_meta","payload":{"id":"child","source":"cli","forked_from_id":"a","parent_thread_id":"b"}}),
            marked_codex_message("input"),
        ],
        1,
    );
    let (obs, _) = collect(codex, &records);
    assert!(!obs.iter().any(|o| o.kind == "human_message"));
    assert!(
        obs[0].fields["native_context"]
            .get("parent_session_id")
            .is_none()
    );
}

#[test]
fn codex_unreadable_first_header_cannot_be_replaced_by_a_later_cli_header() {
    for prefix in [b"{invalid json}\n".to_vec(), {
        let mut bytes = vec![b'x'; 2 * 1024 * 1024];
        bytes.push(b'\n');
        bytes
    }] {
        let rows = [
            json!({"type":"session_meta","payload":{"source":"cli"}}),
            marked_codex_message("copied"),
        ];
        let mut bytes = prefix;
        for row in rows {
            bytes.extend(serde_json::to_vec(&row).unwrap());
            bytes.push(b'\n');
        }
        let splits: Vec<_> = (32_000..bytes.len()).step_by(32_000).collect();
        let mut records = vec![open_native(PRODUCER, 1, "codex-jsonl")];
        records.extend(segment_bytes(PRODUCER, 1, &bytes, &splits));
        records.push(end_native(PRODUCER, 1, EndReason::Complete));
        let (obs, summary) = collect(codex, &records);
        assert!(!obs.iter().any(|o| o.kind == "human_message"));
        assert!(
            obs.iter()
                .any(|o| o.fields["reason"] == "malformed" || o.fields["reason"] == "oversize")
        );
        assert_eq!(
            summary.unknown_records, 3,
            "every record remains source-referenced"
        );
    }
}
