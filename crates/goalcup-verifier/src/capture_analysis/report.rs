//! The private streaming research report: each decoder observation as one NDJSON line, then
//! one bounded summary line. The format is `docs/research/m2-report-format.md`.
//!
//! [`write_research_report_with`] reads a protocol-v5 trace once, through
//! [`scan_capture_trace`], and feeds each validated capture record to a
//! [`DecodeSession`](super::session::DecodeSession) over a [`DecoderRegistry`]. An observation
//! is written the moment its decoder emits it ([`JsonLinesSink`]). Nothing kept here grows
//! with the run: the session's bounded [`Operations`](super::operations::Operations) and each
//! decoder's own bounded state.
//!
//! The summary ([`research_summary`]) states the run, the capture, delivery and the analysis
//! separately. Terminal, receipt and acknowledgement facts are the reader's verified ones
//! and are never reread here. A decoder that fails is stopped and named while the others
//! continue. The library and `goalcup-verify --research-json` share this writer, so the same
//! trace and options give the same bytes. A server that holds records rather than a folder
//! composes the same parts: [`decode_records`](super::session::decode_records) for the
//! observations, `research_summary` for the summary.

use super::api::{Disposition, Observation, ObservationSink, kinds};
use super::reader::{CaptureIntegrityReport, scan_capture_trace};
use super::registry::DecoderRegistry;
use super::session::{DecodeOutcome, DecodeSession, DecoderOutcome, Undecoded, widen};
use super::wire::cut;
use crate::VerifyOptions;
use goalcup_protocol::capture::CaptureRecord;
use goalcup_protocol::capture_status::AnalysisState;
use goalcup_protocol::pins::connector_dir_is_pinned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;

/// The research-report schema version governing every observation and summary line. Version 2
/// adds `analysis.models` and `analysis.cross_leg` to the summary; observation lines are as in 1.
/// Version 3 adds `delivery.records_after_deadline`, `delivery.last_seq_at_or_before_deadline`
/// and `delivery.last_ack_at`, copied from the integrity report. Version 4 adds
/// `analysis.undecoded` (a cut hint adds `hint_truncated`) and `analysis.undecoded_omitted`, and drops the constant
/// `options.dev_mode`. Each decoder entry keeps `measured` and `synthetic_only`. Version 5 adds
/// `analysis.evidence` and `analysis.cross_leg.wire_request_range`.
const RESEARCH_SCHEMA_VERSION: u32 = 5;
/// The longest observation line written, newline included.
const MAX_OBSERVATION_LINE_BYTES: usize = 2 * 1024 * 1024;
/// The longest summary line written, newline included.
const MAX_SUMMARY_BYTES: usize = 64 * 1024;
/// The most of one capture-only stream's hint the summary shows: the hint grammar's bound.
const MAX_SHOWN_HINT_BYTES: usize = 64;

/// One report's exit code (0 complete, 1 incomplete or partial, 2 invalid evidence) and the
/// summary line it wrote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResearchResult {
    pub exit_code: u8,
    pub summary: Value,
}

/// [`write_research_report_with`] over the built-in decoders: the entry point of the library
/// and of `goalcup-verify --research-json`.
pub fn write_research_report(
    trace: &Path,
    options: &VerifyOptions,
    output: &mut dyn Write,
) -> io::Result<ResearchResult> {
    write_research_report_with(trace, options, &DecoderRegistry::builtin(), output)
}

/// Write the research report of the v5 capture trace at `trace` to `output`, decoded by
/// `registry`: each observation line as it is emitted, then the summary line, then a flush.
///
/// `Err` means no usable report: the trace could not be read or is not v5, or `output`
/// failed. No summary line is then known to have reached `output`, and its absence marks
/// the output incomplete.
pub fn write_research_report_with(
    trace: &Path,
    options: &VerifyOptions,
    registry: &DecoderRegistry,
    output: &mut dyn Write,
) -> io::Result<ResearchResult> {
    let mut session = DecodeSession::new(registry, JsonLinesSink::new(&mut *output));
    let integrity = scan_capture_trace(trace, options, &mut |record| session.push(record))?;
    // The sink's borrow of `output` ends with this statement.
    let (outcome, sink) = session.finish()?;
    let tally = sink.tally().clone();
    drop(sink);
    let (exit_code, summary) = research_summary(&integrity, &outcome, &tally, options);
    let mut line = serde_json::to_vec(&summary)?;
    line.push(b'\n');
    if line.len() > MAX_SUMMARY_BYTES {
        return Err(io::Error::other(format!(
            "the report summary is {} bytes, over its {MAX_SUMMARY_BYTES}-byte bound",
            line.len()
        )));
    }
    output.write_all(&line)?;
    output.flush()?;
    Ok(ResearchResult { exit_code, summary })
}

/// Classifies kept observation lines for the summary's decoded and human-evidence states.
/// Source-referenced `unknown` observations are deliberately excluded from the decoded count;
/// spanless findings have no observation line. Both are counted by
/// [`DecoderSummary::unknown_records`](super::api::DecoderSummary::unknown_records), not here.
/// Report vocabulary, so it lives here and not in the session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KindTally {
    by_decoder: BTreeMap<&'static str, KindCounts>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct KindCounts {
    /// Kept observations of any kind but `unknown`.
    decoded: u64,
    /// Kept observations of kind `human_message`.
    human_messages: u64,
}

impl KindTally {
    /// Classifies one kept observation of decoder `decoder_id`; `unknown` leaves both counters
    /// unchanged. [`JsonLinesSink`] does this for every observation it keeps; a host with its own
    /// [`ObservationSink`] does it for every observation it stores, so [`research_summary`] reads
    /// the same analysis state and native markers.
    pub fn observe(&mut self, decoder_id: &'static str, observation: &Observation) {
        let counts = self.by_decoder.entry(decoder_id).or_default();
        match observation.kind.as_str() {
            kinds::UNKNOWN => {}
            kinds::HUMAN_MESSAGE => {
                counts.decoded += 1;
                counts.human_messages += 1;
            }
            _ => counts.decoded += 1,
        }
    }

    fn any_decoded(&self) -> bool {
        self.by_decoder.values().any(|c| c.decoded > 0)
    }

    fn human_messages(&self) -> u64 {
        self.by_decoder.values().map(|c| c.human_messages).sum()
    }
}

/// The report's observation lines: one NDJSON line per observation, as `output` receives it.
/// An observation whose line would pass the 2 MiB bound is dropped, not truncated, and
/// reported [`Disposition::OverBudget`]. It also tallies the kept observations by kind for
/// [`research_summary`].
pub struct JsonLinesSink<W: Write> {
    writer: W,
    tally: KindTally,
}

#[derive(Serialize)]
struct ObservationLine<'a> {
    #[serde(rename = "type")]
    line: &'static str,
    decoder: &'a str,
    observation: &'a Observation,
}

impl<W: Write> JsonLinesSink<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            tally: KindTally::default(),
        }
    }

    /// The kinds of the observations kept so far.
    pub fn tally(&self) -> &KindTally {
        &self.tally
    }

    pub fn into_inner(self) -> W {
        self.writer
    }
}

impl<W: Write> ObservationSink for JsonLinesSink<W> {
    fn observe(
        &mut self,
        decoder_id: &'static str,
        mut observation: Observation,
    ) -> io::Result<Disposition> {
        observation.fields.sort_all_objects();
        let line = ObservationLine {
            line: "observation",
            decoder: decoder_id,
            observation: &observation,
        };
        let mut bytes = serde_json::to_vec(&line)?;
        bytes.push(b'\n');
        if bytes.len() > MAX_OBSERVATION_LINE_BYTES {
            return Ok(Disposition::OverBudget);
        }
        self.writer.write_all(&bytes)?;
        self.tally.observe(decoder_id, &observation);
        Ok(Disposition::Kept)
    }
}

/// The summary line of one decode and the report's exit code. `outcome` is the session's,
/// `integrity` the reader's and `tally` the report sink's; none is recomputed here.
pub fn research_summary(
    integrity: &CaptureIntegrityReport,
    outcome: &DecodeOutcome,
    tally: &KindTally,
    options: &VerifyOptions,
) -> (u8, Value) {
    let decoders = &outcome.decoders;
    let operations = &outcome.operations;
    let degraded = decoders.iter().any(degraded);
    let exit_code = integrity.exit_code().max(u8::from(degraded));
    // KindTally excludes source-referenced `unknown` observation lines from decoded state, while
    // spanless findings never reach the sink. DecoderSummary::unknown_records counts both.
    let analysis = if !tally.any_decoded() {
        AnalysisState::Unavailable
    } else if degraded {
        AnalysisState::Partial
    } else {
        AnalysisState::Available
    };
    // Only the reader's verified facts: an ACK behind the terminal is progress, not delivery.
    let delivery = match (&integrity.verified_ack, &integrity.terminal) {
        (Some(ack), Some(terminal)) if ack.seq >= terminal.seq => "through_terminal",
        (Some(_), _) => "partial",
        (None, _) if integrity.receipt_clock.is_some() => "none_acknowledged",
        (None, _) => "local_only",
    };
    let legs = Legs::of(outcome);
    let explicit = operations.counts().get("assistance").copied().unwrap_or(0);
    let markers = tally.human_messages();
    // No explicit evidence is never a finding of zero assistance.
    let assistance = if explicit > 0 || markers > 0 {
        "observed"
    } else {
        "none_observed"
    };
    let mut run = json!({"launch": operations.launch(), "terminal": integrity.terminal});
    if let Some(label) = connector_presentation(operations.launch()) {
        run["connector"] = json!(label);
    }
    let mut summary = json!({
        "type": "summary",
        "schema_version": RESEARCH_SCHEMA_VERSION,
        "exit_code": exit_code,
        "verifier_version": crate::VERIFIER_VERSION,
        "options": {
            "allow_descriptor_key": options.allow_descriptor_key,
            "extra_server_keys": options.extra_server_keys,
        },
        "key_source": integrity.key_source,
        "input": {
            "validated_head": integrity.validated_head,
            "validated_records": integrity.validated_records,
            "validated_bytes": integrity.validated_bytes,
        },
        "run": run,
        "capture": {
            "integrity": integrity.integrity,
            "sealed": integrity.sealed,
            "coverage_incomplete": integrity.coverage_incomplete,
            "torn_tail": integrity.torn_tail,
            "gap_count": integrity.gap_count,
            "findings": integrity.findings,
            "record_counts": operations.counts(),
        },
        "delivery": {
            "anchor": integrity.anchor,
            "state": delivery,
            "verified_ack": integrity.verified_ack,
            "receipt_clock": integrity.receipt_clock,
            // The reader's deadline facts, unchanged. The two counts use the participant's
            // machine clock and are a hint; `last_ack_at` is the server's signed time.
            "records_after_deadline": integrity.records_after_deadline,
            "last_seq_at_or_before_deadline": integrity.last_seq_at_or_before_deadline,
            "last_ack_at": integrity.last_ack_at,
        },
        "assistance": {
            "explicit_records": explicit,
            "positive_native_markers": markers,
            "evidence": assistance,
        },
        "analysis": {
            "state": analysis,
            "decoders": decoders.iter().map(entry).collect::<Vec<_>>(),
            "models": models(decoders),
            "models_omitted": models_omitted(decoders),
            "evidence": evidence(legs.native, legs.model_request_streams, legs.wire_requests),
            "cross_leg": cross_leg(outcome, &legs),
            "undecoded": outcome.undecoded.iter().map(undecoded).collect::<Vec<_>>(),
            "undecoded_omitted": outcome.undecoded_omitted,
        },
        "references": operations.references(),
        "references_omitted": operations.references_omitted(),
    });
    summary.sort_all_objects();
    (exit_code, summary)
}

/// The model identities each decoder's kept `model` observations named, with counts, in
/// decoder then name order. What the harness's session file and its traffic say the run used.
fn models(decoders: &[DecoderOutcome]) -> Vec<Value> {
    decoders
        .iter()
        .flat_map(|d| {
            d.observations
                .models
                .iter()
                .map(|(model, count)| json!({"decoder": d.id, "model": model, "count": count}))
        })
        .collect()
}

/// The names each decoder could not keep, for the decoders that dropped any.
fn models_omitted(decoders: &[DecoderOutcome]) -> Value {
    Value::Object(
        decoders
            .iter()
            .filter(|d| d.observations.models_omitted > 0)
            .map(|d| (d.id.to_string(), json!(d.observations.models_omitted)))
            .collect(),
    )
}

/// Whether the harness's own session file and the recorded traffic agree that model work
/// happened. Only zero against non-zero is judged: a harness may write several native entries
/// per call. `native_activity_without_wire_streams` is a note for judges: the session file names
/// model work while the recording proxy recorded no wire stream at all (a harness that bypassed the
/// recording proxy, or a fake). It never changes the exit code. `unchecked` when there is no native
/// model activity to compare, or wire streams were recorded but no model request was decoded from
/// them (a compressed or oversized body, a degraded decoder): the calls may be there unread.
/// `wire_request_range` is the wall-clock span of the decoded wire requests, `null` without one.
fn cross_leg(outcome: &DecodeOutcome, legs: &Legs) -> Value {
    let (mut first, mut last) = (None, None);
    for counts in outcome.decoders.iter().map(|d| &d.observations) {
        if let (Some(a), Some(b)) = (&counts.wire_first, &counts.wire_last) {
            widen(&mut first, a, Ordering::Less);
            widen(&mut last, b, Ordering::Greater);
        }
    }
    let range = first
        .zip(last)
        .map(|(first, last)| json!({"first": first, "last": last}));
    json!({
        "native_model_activity": legs.native,
        "wire_streams": legs.wire_streams,
        "wire_model_request_streams": legs.model_request_streams,
        "wire_model_requests": legs.wire_requests,
        "wire_request_range": range,
        "state": cross_leg_state(legs.native, legs.wire_streams, legs.wire_requests),
    })
}

/// Each leg's count, summed across its decoders.
struct Legs {
    native: u64,
    wire_streams: u64,
    model_request_streams: u64,
    wire_requests: u64,
}

impl Legs {
    fn of(outcome: &DecodeOutcome) -> Self {
        let sum = |leg: fn(&DecoderOutcome) -> u64| outcome.decoders.iter().map(leg).sum();
        Self {
            native: sum(|d| d.observations.native_model_activity),
            wire_streams: outcome.operations.wire_opens(),
            model_request_streams: outcome.operations.model_request_opens(),
            wire_requests: sum(|d| d.observations.wire_model_requests),
        }
    }
}

/// The one rule behind `analysis.cross_leg.state`.
fn cross_leg_state(native: u64, wire_streams: u64, wire_requests: u64) -> &'static str {
    if native == 0 {
        "unchecked"
    } else if wire_streams == 0 {
        "native_activity_without_wire_streams"
    } else if wire_requests == 0 {
        "unchecked"
    } else {
        "consistent"
    }
}

/// The one rule behind `analysis.evidence`: what the run holds, decoded wire requests first.
/// Model request streams with no decoded request outrank the session file, since the calls may
/// be there unread. Header, tunnel and lone GET streams are not model requests, so they do not.
fn evidence(native: u64, model_request_streams: u64, wire_requests: u64) -> &'static str {
    if wire_requests > 0 {
        if native > 0 {
            "wire_and_native"
        } else {
            "wire_only"
        }
    } else if model_request_streams > 0 {
        "wire_undecoded"
    } else if native > 0 {
        "native_only"
    } else {
        "none"
    }
}

/// User-visible connector provenance inferred from a validated Launch. A digest that is
/// present but not this profile id's compiled directory pin is the development/unpinned case;
/// an absent digest (including an absent Launch) remains the existing unbound case.
fn connector_presentation(launch: Option<&CaptureRecord>) -> Option<&'static str> {
    let Some(CaptureRecord::Launch {
        profile_id,
        connector_dir_sha256: Some(digest),
        ..
    }) = launch
    else {
        return None;
    };
    Some(if connector_dir_is_pinned(profile_id, digest) {
        "Pinned"
    } else {
        "Dev/unpinned"
    })
}

/// Failed, dropped an observation over the line bound, or reported itself incomplete.
pub(super) fn degraded(decoder: &DecoderOutcome) -> bool {
    decoder.result.as_ref().map_or(true, |s| s.incomplete) || decoder.observations.over_budget > 0
}

fn entry(decoder: &DecoderOutcome) -> Value {
    json!({
        "decoder": decoder.id,
        "summary": decoder.result.as_ref().ok(),
        "written": decoder.observations.kept,
        "over_budget": decoder.observations.over_budget,
        "failure": decoder.result.as_ref().err().map(|failed| {
            json!({"phase": failed.phase.as_str(), "error": failed.failure.reason})
        }),
        "measured": decoder.measured_on,
        "synthetic_only": decoder.synthetic_only,
    })
}

/// One group of capture-only streams: recorded, read by no decoder. A hint is shown up to the
/// hint grammar's 64 bytes, so 64 groups stay far inside the summary bound. A hint that was cut
/// carries `"hint_truncated": true`; the key is absent otherwise. The group itself is still
/// made from the full hint.
fn undecoded(group: &Undecoded) -> Value {
    let hint = group.protocol_hint.as_deref();
    let mut shown = json!({
        "source": group.source,
        "hint": hint.map(|hint| cut(hint, MAX_SHOWN_HINT_BYTES)),
        "streams": group.streams,
        "bytes": group.bytes,
        "bytes_incomplete": group.bytes_incomplete,
    });
    if hint.is_some_and(|hint| hint.len() > MAX_SHOWN_HINT_BYTES) {
        shown["hint_truncated"] = json!(true);
    }
    shown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cross_leg_note_judges_zero_against_non_zero_only() {
        assert_eq!(cross_leg_state(3, 2, 1), "consistent");
        assert_eq!(cross_leg_state(1, 9, 9), "consistent");
        // The session names model work and the recording proxy recorded nothing at all.
        assert_eq!(
            cross_leg_state(5, 0, 0),
            "native_activity_without_wire_streams"
        );
        // Wire streams with no decoded request may hold calls unread: not an accusation.
        assert_eq!(cross_leg_state(5, 3, 0), "unchecked");
        assert_eq!(cross_leg_state(0, 4, 4), "unchecked");
        assert_eq!(cross_leg_state(0, 0, 0), "unchecked");
    }

    #[test]
    fn the_evidence_label_follows_decoded_requests_first() {
        assert_eq!(evidence(1, 1, 1), "wire_and_native");
        assert_eq!(evidence(0, 1, 1), "wire_only");
        assert_eq!(evidence(0, 1, 0), "wire_undecoded");
        assert_eq!(evidence(1, 0, 0), "native_only");
        assert_eq!(evidence(0, 0, 0), "none");
        // Wire streams with no decoded request outrank the session file.
        assert_eq!(evidence(1, 1, 0), "wire_undecoded");
    }

    #[test]
    fn a_lone_get_and_header_streams_do_not_outrank_native() {
        let mut operations = crate::capture_analysis::operations::Operations::default();
        let open = |hint: Option<&str>, method: Option<&str>| CaptureRecord::Open {
            stream: StreamKey {
                producer: "producer".to_string(),
                stream: 1,
            },
            observed: time("producer", 1),
            source: CaptureSource::WireRequest,
            metadata: CaptureMeta {
                wire: None,
                label: "unit".to_string(),
                protocol_hint: hint.map(str::to_string),
                method: method.map(str::to_string),
                host: None,
                path: None,
                status: None,
                content_type: None,
            },
        };
        operations.record(&open(None, Some("GET")));
        operations.record(&open(Some("http-head-v1"), None));
        operations.record(&open(Some("http-head-v1"), None));
        // Native model work, no decoded request: the lone GET and the header streams are not
        // model requests, so the session file is the evidence.
        assert_eq!(
            evidence(1, operations.model_request_opens(), 0),
            "native_only"
        );
        // A model request that no decoder read still outranks nothing: it is undecoded.
        operations.record(&open(Some("openai-chat-completions"), Some("POST")));
        assert_eq!(
            evidence(1, operations.model_request_opens(), 0),
            "wire_undecoded"
        );
    }

    #[test]
    fn a_lone_get_beside_native_work_labels_native_only_through_the_report() {
        // The label is read on the whole report path, not only by `evidence`. A lone GET (no
        // hint, not a POST) is a wire stream that no decoder reads and not a model request, so
        // the session file's model work is the evidence. Reverting the label to `wire_streams`
        // turns this run into `wire_undecoded`.
        const WIRE: &str = "b1b2b3b4b5b6b7b8b9babbbcbdbebf00";
        let stream = StreamKey {
            producer: WIRE.to_string(),
            stream: 1,
        };
        let lone_get = vec![
            CaptureRecord::Connection {
                producer: WIRE.to_string(),
                connection: "c0ffee00c0ffee00c0ffee00c0ffee00".to_string(),
                observed: time(WIRE, 1),
                event: goalcup_protocol::capture::ConnectionEvent::Open {
                    alpn: None,
                    already_open: false,
                },
            },
            CaptureRecord::Open {
                stream: stream.clone(),
                observed: time(WIRE, 2),
                source: CaptureSource::WireRequest,
                metadata: CaptureMeta {
                    wire: Some(goalcup_protocol::capture::WireIdentity {
                        connection: "c0ffee00c0ffee00c0ffee00c0ffee00".to_string(),
                        exchange: 1,
                        message_index: None,
                    }),
                    label: "unit".to_string(),
                    protocol_hint: None,
                    method: Some("GET".to_string()),
                    host: None,
                    path: None,
                    status: None,
                    content_type: None,
                },
            },
            CaptureRecord::End {
                stream,
                observed: time(WIRE, 3),
                reason: EndReason::Complete,
            },
            CaptureRecord::ProducerClosed {
                producer: WIRE.to_string(),
                observed: time("daemon", 4),
                final_seq: Some(2),
            },
        ];
        let dir = tempfile::tempdir().unwrap();
        write_trace_with(dir.path(), lone_get);
        let mut out = Vec::new();
        let result =
            write_research_report(dir.path(), &VerifyOptions::default(), &mut out).unwrap();
        let cross_leg = &result.summary["analysis"]["cross_leg"];
        assert!(cross_leg["native_model_activity"].as_u64().unwrap() > 0);
        assert_eq!(cross_leg["wire_streams"], json!(1));
        assert_eq!(cross_leg["wire_model_request_streams"], json!(0));
        assert_eq!(result.summary["analysis"]["evidence"], json!("native_only"));
    }

    use crate::capture_analysis::api::{
        DecoderFailure, DecoderSummary, DecoderV1, ObservationTimeRange, SourceRef, StreamOpen,
    };
    use crate::capture_analysis::native;
    use crate::capture_analysis::registry::Registered;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use goalcup_protocol::capture::{CaptureMeta, CaptureTime, M2_PROTOCOL_VERSION};
    use goalcup_protocol::capture::{CaptureRecord, CaptureSource, EndReason, StreamKey};
    use goalcup_protocol::chain::Link;
    use goalcup_protocol::envelope::{Envelope, EventBody};
    use goalcup_protocol::export::TraceExport;
    use goalcup_protocol::provenance::{Rfc3339Utc, TerminationOutcomeV1};
    use goalcup_protocol::redaction::RedactionAudit;
    use goalcup_protocol::seal::SealManifest;

    const GOAL: &str = "9d8c7b6a5f4e3d2c1b0a998877665544";
    const NATIVE: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90";

    fn time(clock: &str, tick: u64) -> CaptureTime {
        CaptureTime {
            wall: format!("2026-09-29T12:00:00.{tick:09}Z"),
            mono_ns: tick,
            clock_id: clock.to_string(),
        }
    }

    /// A sealed local v5 trace: a goal stream, its Launch and one Codex rollout stream of
    /// one record, each stream in one segment.
    fn write_trace(root: &Path) {
        write_trace_with(root, Vec::new());
    }

    /// [`write_trace`] with `extra` records after the rollout stream and before the Launch.
    fn write_trace_with(root: &Path, extra: Vec<CaptureRecord>) {
        let manifest = SealManifest {
            entry_id: "entry-report-unit".to_string(),
            sealed_at: "2026-09-29T12:00:00Z".to_string(),
            workspace: "/ws".to_string(),
            files: vec![],
            excluded: vec![],
            git: None,
            home: None,
            capped: None,
        };
        let mut export = TraceExport::create(root).unwrap();
        export.configure_capture_quota(1 << 30).unwrap();
        export
            .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest)
            .unwrap();
        let rollout = b"{\"type\":\"turn_context\",\"payload\":{\"model\":\"gpt-5.6-luna\"}}\n";
        let mut records = Vec::new();
        for (producer, source, hint, bytes) in [
            (GOAL, CaptureSource::Goal, None, &b"the goal"[..]),
            (
                NATIVE,
                CaptureSource::Native,
                Some(<native::codex::Codex as native::NativeFormat>::HINT),
                &rollout[..],
            ),
        ] {
            let stream = StreamKey {
                producer: producer.to_string(),
                stream: 1,
            };
            records.push(CaptureRecord::Open {
                stream: stream.clone(),
                observed: time(producer, 1),
                source,
                metadata: CaptureMeta {
                    wire: None,
                    label: "unit".to_string(),
                    protocol_hint: hint.map(str::to_string),
                    method: None,
                    host: None,
                    path: None,
                    status: None,
                    content_type: None,
                },
            });
            records.push(CaptureRecord::Segment {
                stream: stream.clone(),
                part: 0,
                source_offset: 0,
                source_len: bytes.len() as u64,
                first: time(producer, 2),
                last: time(producer, 3),
                data_b64: STANDARD.encode(bytes),
                audit: RedactionAudit { spans: vec![] },
            });
            records.push(CaptureRecord::End {
                stream,
                observed: time(producer, 4),
                reason: EndReason::Complete,
            });
            records.push(CaptureRecord::ProducerClosed {
                producer: producer.to_string(),
                observed: time("daemon", 5),
                final_seq: Some(2),
            });
        }
        records.extend(extra);
        records.push(CaptureRecord::Launch {
            observed: time("daemon", 6),
            goal_stream: StreamKey {
                producer: GOAL.to_string(),
                stream: 1,
            },
            profile_id: "test-cli".to_string(),
            profile_digest: "cc".repeat(32),
            harness_version: "0.153.0".to_string(),
            recorder_revision: "unit".to_string(),
            continuation: "none".to_string(),
            allowed_tool_rule: None,
            process_handoff_digest: None,
            run_public_key: None,
            connector_dir_sha256: None,
            harness_build: None,
        });
        for record in records {
            let mut env = Envelope {
                protocol_version: M2_PROTOCOL_VERSION,
                entry_id: manifest.entry_id.clone(),
                source: "daemon".to_string(),
                recorded_at: "2026-09-29T12:00:01Z".to_string(),
                link: Link {
                    seq: 0,
                    prev: String::new(),
                    hash: String::new(),
                },
                body: EventBody::Capture { record },
                redaction: None,
            };
            export.append_chain(&mut env).unwrap();
        }
        let checked_at = Rfc3339Utc::try_from("2026-09-29T12:00:05Z").unwrap();
        export
            .write_local_finish(
                &manifest,
                time("daemon", 7),
                "participant_stop",
                TerminationOutcomeV1::AlreadyStopped { checked_at },
            )
            .unwrap();
    }

    /// Reads every stream and fails on the first record it is given.
    struct Failing;

    impl DecoderV1 for Failing {
        fn id(&self) -> &'static str {
            "failing"
        }

        fn version(&self) -> &'static str {
            "1"
        }

        fn accepts(&self, _: &StreamOpen<'_>) -> bool {
            true
        }

        fn push(
            &mut self,
            _: &CaptureRecord,
            _: &mut dyn FnMut(Observation),
        ) -> Result<(), DecoderFailure> {
            Err(DecoderFailure::new("invalid data"))
        }

        fn finish(
            self: Box<Self>,
            _: &mut dyn FnMut(Observation),
        ) -> Result<DecoderSummary, DecoderFailure> {
            panic!("a failed decoder is never finished");
        }
    }

    /// For each segment: one observation over the line bound, then an ordinary one.
    struct Oversized;

    impl DecoderV1 for Oversized {
        fn id(&self) -> &'static str {
            "oversized"
        }

        fn version(&self) -> &'static str {
            "1"
        }

        fn accepts(&self, _: &StreamOpen<'_>) -> bool {
            true
        }

        fn push(
            &mut self,
            record: &CaptureRecord,
            emit: &mut dyn FnMut(Observation),
        ) -> Result<(), DecoderFailure> {
            if let CaptureRecord::Segment {
                stream,
                part,
                first,
                last,
                ..
            } = record
            {
                for text in ["x".repeat(MAX_OBSERVATION_LINE_BYTES), "small".to_string()] {
                    emit(Observation {
                        source: SourceRef {
                            stream: stream.clone(),
                            first_part: *part,
                            last_part: *part,
                        },
                        observed: ObservationTimeRange {
                            first: first.clone(),
                            last: last.clone(),
                        },
                        kind: kinds::UNKNOWN.to_string(),
                        supplied_id: None,
                        fields: json!({ "text": text }),
                    });
                }
            }
            Ok(())
        }

        fn finish(
            self: Box<Self>,
            _: &mut dyn FnMut(Observation),
        ) -> Result<DecoderSummary, DecoderFailure> {
            Ok(DecoderSummary::new(4, 0, false))
        }
    }

    struct InvalidKind;

    impl DecoderV1 for InvalidKind {
        fn id(&self) -> &'static str {
            "invalid-kind"
        }
        fn version(&self) -> &'static str {
            "1"
        }
        fn accepts(&self, _: &StreamOpen<'_>) -> bool {
            true
        }
        fn push(
            &mut self,
            record: &CaptureRecord,
            emit: &mut dyn FnMut(Observation),
        ) -> Result<(), DecoderFailure> {
            if let CaptureRecord::Segment {
                stream,
                part,
                first,
                last,
                ..
            } = record
            {
                emit(Observation::new(
                    SourceRef::new(stream.clone(), *part, *part),
                    ObservationTimeRange::new(first.clone(), last.clone()),
                    "outside-v1",
                    json!({}),
                ));
            }
            Ok(())
        }
        fn finish(
            self: Box<Self>,
            _: &mut dyn FnMut(Observation),
        ) -> Result<DecoderSummary, DecoderFailure> {
            Ok(DecoderSummary::new(0, 0, false))
        }
    }

    #[test]
    fn invalid_kind_is_decoder_failure_and_report_exit_one_not_sink_failure() {
        let dir = tempfile::tempdir().unwrap();
        write_trace(dir.path());
        let mut registry = DecoderRegistry::empty();
        registry
            .register(Registered::new(|| Box::new(InvalidKind), "none", &[]))
            .unwrap();
        registry
            .register(DecoderRegistry::builtin().entries()[0].clone())
            .unwrap();
        let mut out = Vec::new();
        let result =
            write_research_report_with(dir.path(), &VerifyOptions::default(), &registry, &mut out)
                .unwrap();
        assert_eq!(result.exit_code, 1);
        assert_eq!(
            result.summary["analysis"]["decoders"][0]["failure"],
            json!({"error": "invalid_observation_kind", "phase": "push"})
        );
        assert!(!String::from_utf8(out).unwrap().contains("outside-v1"));
    }

    #[test]
    fn observation_fields_are_serialized_in_recursive_lexicographic_order() {
        let mut inner = serde_json::Map::new();
        inner.insert("z".into(), json!(1));
        inner.insert("a".into(), json!({"y": 2, "b": 3}));
        let observation = Observation::new(
            SourceRef::new(
                StreamKey {
                    producer: NATIVE.into(),
                    stream: 9,
                },
                0,
                0,
            ),
            ObservationTimeRange::new(time(NATIVE, 1), time(NATIVE, 2)),
            kinds::UNKNOWN,
            Value::Object(inner),
        );
        let mut bytes = Vec::new();
        JsonLinesSink::new(&mut bytes)
            .observe("test", observation)
            .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"fields\":{\"a\":{\"b\":3,\"y\":2},\"z\":1}"));
    }

    #[test]
    fn one_decoder_failure_leaves_other_sources_reportable() {
        let dir = tempfile::tempdir().unwrap();
        write_trace(dir.path());
        let mut registry = DecoderRegistry::empty();
        registry
            .register(Registered::new(|| Box::new(Failing), "none", &[]))
            .unwrap();
        registry
            .register(Registered::new(|| Box::new(Oversized), "none", &[]))
            .unwrap();
        registry
            .register(DecoderRegistry::builtin().entries()[0].clone())
            .unwrap();
        let native_id = DecoderRegistry::builtin().ids()[0].0;
        assert_eq!(registry.ids()[2].0, native_id);
        let mut out = Vec::new();
        let result =
            write_research_report_with(dir.path(), &VerifyOptions::default(), &registry, &mut out)
                .unwrap();
        let lines: Vec<Value> = out
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        let summary = lines.last().unwrap();
        assert_eq!(summary, &result.summary);
        assert_eq!(summary["type"], "summary");

        // The failing decoder is stopped and named; integrity is unaffected.
        let entries = summary["analysis"]["decoders"].as_array().unwrap();
        assert_eq!(
            entries[0]["failure"],
            json!({"phase": "push", "error": "invalid data"})
        );
        assert_eq!(entries[0]["summary"], Value::Null);
        assert_eq!(entries[0]["written"], 0);
        assert_eq!(summary["capture"]["integrity"], "valid");

        // An observation over the line bound is counted and not written; the next one is.
        assert_eq!(entries[1]["over_budget"], 2);
        assert_eq!(entries[1]["written"], 2);
        let oversized: Vec<_> = lines
            .iter()
            .filter(|l| l["decoder"] == "oversized")
            .collect();
        assert!(
            oversized
                .iter()
                .all(|l| l["observation"]["fields"]["text"] == "small")
        );
        assert!(out.len() < MAX_OBSERVATION_LINE_BYTES);

        // The other source still reports in full.
        let native: Vec<_> = lines.iter().filter(|l| l["decoder"] == native_id).collect();
        assert_eq!(native.len(), 1);
        assert_eq!(native[0]["observation"]["kind"], "model");
        assert_eq!(entries[2]["failure"], Value::Null);
        assert_eq!(entries[2]["summary"]["incomplete"], false);

        assert_eq!(summary["analysis"]["state"], "partial");
        assert_eq!(result.exit_code, 1);
    }
}
