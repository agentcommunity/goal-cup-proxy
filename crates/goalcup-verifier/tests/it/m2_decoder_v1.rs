//! Decoder v1 (contract S7): the registry, the decode session and its isolation, over records
//! held in memory. The built-in decoders' bytes are pinned elsewhere
//! (`m2_research_report.rs`); this file pins the contract itself: selection by `accepts`,
//! capture-only for a hint nothing reads, one decoder's panic or error stopping that decoder
//! alone, the bound on routed streams, the session owning every decoder's identity, a sink
//! error ending the session, and that a registry and a session can be used from worker
//! threads.

use crate::common as support;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use goalcup_protocol::capture::{
    CaptureMeta, CaptureRecord, CaptureSource, CaptureTime, EndReason, GapReason, MAX_OPEN_STREAMS,
    StreamKey,
};
use goalcup_protocol::hints::valid_hint;
use goalcup_protocol::redaction::RedactionAudit;
use goalcup_verifier::capture_analysis::api::{
    DECODER_API_VERSION, DecoderFailure, DecoderSummary, DecoderV1, Disposition, Observation,
    ObservationSink, ObservationTimeRange, SourceRef, StreamOpen, kinds,
};
use goalcup_verifier::capture_analysis::registry::{DecoderRegistry, Registered, RegistryError};
use goalcup_verifier::capture_analysis::session::{
    DecodeOutcome, DecodeSession, DecoderOutcome, DecoderPhase, decode_records,
};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use support::decode::{Collect, run};

const PRODUCER: &str = "0123456789abcdef0123456789abcdef";

fn producer_stream(producer: &str, stream: u64) -> StreamKey {
    StreamKey {
        producer: producer.to_string(),
        stream,
    }
}

fn key(stream: u64) -> StreamKey {
    producer_stream(PRODUCER, stream)
}

fn at(tick: u64) -> CaptureTime {
    CaptureTime {
        wall: format!("2026-10-05T12:00:00.{tick:09}Z"),
        mono_ns: tick,
        clock_id: "clock".to_string(),
    }
}

fn open_in(key: StreamKey, source: CaptureSource, hint: Option<&str>) -> CaptureRecord {
    CaptureRecord::Open {
        stream: key,
        observed: at(1),
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
    }
}

fn open(stream: u64, source: CaptureSource, hint: Option<&str>) -> CaptureRecord {
    open_in(key(stream), source, hint)
}

fn segment_in(key: StreamKey, bytes: &[u8]) -> CaptureRecord {
    CaptureRecord::Segment {
        stream: key,
        part: 0,
        source_offset: 0,
        source_len: bytes.len() as u64,
        first: at(2),
        last: at(3),
        data_b64: STANDARD.encode(bytes),
        audit: RedactionAudit { spans: vec![] },
    }
}

fn segment(stream: u64, bytes: &[u8]) -> CaptureRecord {
    segment_in(key(stream), bytes)
}

fn end(stream: u64) -> CaptureRecord {
    CaptureRecord::End {
        stream: key(stream),
        observed: at(4),
        reason: EndReason::Complete,
    }
}

/// One stream of one segment under `hint`.
fn stream(stream: u64, source: CaptureSource, hint: Option<&str>) -> Vec<CaptureRecord> {
    vec![
        open(stream, source, hint),
        segment(stream, b"payload"),
        end(stream),
    ]
}

// ---------------------------------------------------------------------------------------
// Test decoders. They read streams hinted `probe`, emit one `note` per segment, and fail
// where their `At` says.
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum At {
    Never,
    Segment,
    Finish,
    Accepts,
    Error,
    /// Fails in `push`, then panics when dropped.
    ErrorThenDropPanics,
}

/// Records what the instance was asked after its first failure.
static PUSHES_AFTER_FAILURE: AtomicU64 = AtomicU64::new(0);
static DROPS: AtomicU64 = AtomicU64::new(0);

struct Probe {
    id: &'static str,
    version: &'static str,
    at: At,
    notes: u64,
    failed: bool,
}

impl Probe {
    fn new(id: &'static str, at: At) -> Self {
        Self {
            id,
            version: "1",
            at,
            notes: 0,
            failed: false,
        }
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        if self.at == At::ErrorThenDropPanics && self.failed {
            DROPS.fetch_add(1, Ordering::SeqCst);
            panic!("SECRET-RECORD-TEXT from a destructor");
        }
    }
}

impl DecoderV1 for Probe {
    fn id(&self) -> &'static str {
        self.id
    }

    fn version(&self) -> &'static str {
        self.version
    }

    fn accepts(&self, open: &StreamOpen<'_>) -> bool {
        assert!(self.at != At::Accepts, "accepts exploded");
        open.protocol_hint == Some("probe")
    }

    fn push(
        &mut self,
        record: &CaptureRecord,
        emit: &mut dyn FnMut(Observation),
    ) -> Result<(), DecoderFailure> {
        if self.failed {
            PUSHES_AFTER_FAILURE.fetch_add(1, Ordering::SeqCst);
        }
        if let CaptureRecord::Segment {
            stream,
            part,
            first,
            last,
            ..
        } = record
        {
            self.notes += 1;
            emit(Observation::new(
                SourceRef::new(stream.clone(), *part, *part),
                ObservationTimeRange::new(first.clone(), last.clone()),
                "unknown",
                serde_json::json!({"text": "SECRET-RECORD-TEXT"}),
            ));
            assert!(self.at != At::Segment, "SECRET-RECORD-TEXT in a panic");
            if matches!(self.at, At::Error | At::ErrorThenDropPanics) {
                self.failed = true;
                return Err(DecoderFailure::new("gave_up"));
            }
        }
        Ok(())
    }

    fn push_recovery(
        &mut self,
        record: &CaptureRecord,
        _: &[StreamKey],
        emit: &mut dyn FnMut(Observation),
    ) -> Result<(), DecoderFailure> {
        self.push(record, emit)
    }

    fn finish(
        self: Box<Self>,
        _: &mut dyn FnMut(Observation),
    ) -> Result<DecoderSummary, DecoderFailure> {
        assert!(self.at != At::Finish, "finish exploded");
        // A decoder naming itself in its summary changes nothing: the session stamps it.
        Ok(DecoderSummary::new(self.notes, 0, false).with_identity("liar", "99"))
    }
}

fn healthy() -> Box<dyn DecoderV1> {
    Box::new(Probe::new("healthy", At::Never))
}

struct LegacyProbe(Probe);

impl DecoderV1 for LegacyProbe {
    fn id(&self) -> &'static str {
        self.0.id()
    }

    fn version(&self) -> &'static str {
        self.0.version()
    }

    fn accepts(&self, open: &StreamOpen<'_>) -> bool {
        self.0.accepts(open)
    }

    fn push(
        &mut self,
        record: &CaptureRecord,
        emit: &mut dyn FnMut(Observation),
    ) -> Result<(), DecoderFailure> {
        if matches!(record, CaptureRecord::Recovery { .. }) {
            self.0.notes += 1;
        }
        self.0.push(record, emit)
    }

    fn finish(
        self: Box<Self>,
        emit: &mut dyn FnMut(Observation),
    ) -> Result<DecoderSummary, DecoderFailure> {
        Box::new(self.0).finish(emit)
    }
}

fn legacy_healthy() -> Box<dyn DecoderV1> {
    Box::new(LegacyProbe(Probe::new("legacy-healthy", At::Never)))
}
fn panics_in_push() -> Box<dyn DecoderV1> {
    Box::new(Probe::new("panics-in-push", At::Segment))
}
fn panics_in_finish() -> Box<dyn DecoderV1> {
    Box::new(Probe::new("panics-in-finish", At::Finish))
}
fn panics_in_accepts() -> Box<dyn DecoderV1> {
    Box::new(Probe::new("panics-in-accepts", At::Accepts))
}
fn errors() -> Box<dyn DecoderV1> {
    Box::new(Probe::new("errors", At::Error))
}

struct InvalidKind {
    id: &'static str,
    at_finish: bool,
    pending: Option<Observation>,
    pending_valid: Option<Observation>,
}

impl DecoderV1 for InvalidKind {
    fn id(&self) -> &'static str {
        self.id
    }
    fn version(&self) -> &'static str {
        "1"
    }
    fn accepts(&self, open: &StreamOpen<'_>) -> bool {
        open.protocol_hint == Some("probe")
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
            let invalid = Observation::new(
                SourceRef::new(stream.clone(), *part, *part),
                ObservationTimeRange::new(first.clone(), last.clone()),
                "not-in-decoder-v1",
                serde_json::json!({}),
            );
            let valid = Observation::new(
                SourceRef::new(stream.clone(), *part, *part),
                ObservationTimeRange::new(first.clone(), last.clone()),
                kinds::UNKNOWN,
                serde_json::json!({"after": "invalid"}),
            );
            if self.at_finish {
                self.pending = Some(invalid);
                self.pending_valid = Some(valid);
            } else {
                emit(invalid);
                emit(valid);
            }
        }
        Ok(())
    }
    fn finish(
        mut self: Box<Self>,
        emit: &mut dyn FnMut(Observation),
    ) -> Result<DecoderSummary, DecoderFailure> {
        if let Some(observation) = self.pending.take() {
            emit(observation);
        }
        if let Some(observation) = self.pending_valid.take() {
            emit(observation);
        }
        Ok(DecoderSummary::new(0, 0, false))
    }
}

fn invalid_in_push() -> Box<dyn DecoderV1> {
    Box::new(InvalidKind {
        id: "invalid-push",
        at_finish: false,
        pending: None,
        pending_valid: None,
    })
}

fn invalid_in_finish() -> Box<dyn DecoderV1> {
    Box::new(InvalidKind {
        id: "invalid-finish",
        at_finish: true,
        pending: None,
        pending_valid: None,
    })
}

fn registry_of(factories: &[fn() -> Box<dyn DecoderV1>]) -> DecoderRegistry {
    let mut registry = DecoderRegistry::empty();
    for make in factories {
        registry
            .register(Registered::new(*make, "unit", &[]))
            .unwrap();
    }
    registry
}

fn outcome_of<'a>(outcome: &'a DecodeOutcome, id: &str) -> &'a DecoderOutcome {
    outcome.decoders.iter().find(|d| d.id == id).unwrap()
}

fn failed(outcome: &DecoderOutcome) -> (DecoderPhase, &str) {
    let failed = outcome.result.as_ref().unwrap_err();
    (failed.phase, failed.failure.reason.as_str())
}

// ---------------------------------------------------------------------------------------
// Isolation.
// ---------------------------------------------------------------------------------------

#[test]
fn a_panicking_decoder_is_isolated_and_the_others_finish() {
    let registry = registry_of(&[
        panics_in_push,
        panics_in_finish,
        panics_in_accepts,
        errors,
        healthy,
    ]);
    let mut records = stream(1, CaptureSource::Native, Some("probe"));
    records.extend(stream(2, CaptureSource::Native, Some("probe")));
    let (outcome, observations) = run(&registry, &records);

    // The healthy decoder read both streams, undisturbed, and was finished.
    let healthy = outcome_of(&outcome, "healthy");
    assert_eq!(healthy.result.as_ref().unwrap().observations, 2);
    assert_eq!(healthy.observations.kept, 2);

    // A panic in push: failed at the first segment, told nothing further, no payload text.
    let push = outcome_of(&outcome, "panics-in-push");
    assert_eq!(failed(push), (DecoderPhase::Push, "panicked"));
    // What it emitted before panicking stays in the sink and is counted.
    assert_eq!(push.observations.kept, 1);
    assert!(observations.iter().any(|(id, _)| *id == "panics-in-push"));

    // A panic in finish; a panic in accepts (it reads nothing); a decoder's own error.
    let finish = outcome_of(&outcome, "panics-in-finish");
    assert_eq!(failed(finish), (DecoderPhase::Finish, "panicked"));
    assert_eq!(finish.observations.kept, 2);
    let accepts = outcome_of(&outcome, "panics-in-accepts");
    assert_eq!(failed(accepts), (DecoderPhase::Accepts, "panicked"));
    assert_eq!(accepts.observations.kept, 0);
    let error = outcome_of(&outcome, "errors");
    assert_eq!(failed(error), (DecoderPhase::Push, "gave_up"));
    assert_eq!(error.observations.kept, 1, "told nothing after the error");

    // Registry order is outcome order.
    let ids: Vec<_> = outcome.decoders.iter().map(|d| d.id).collect();
    assert_eq!(
        ids,
        [
            "panics-in-push",
            "panics-in-finish",
            "panics-in-accepts",
            "errors",
            "healthy"
        ]
    );
}

#[test]
fn invalid_observation_kinds_fail_only_the_emitting_decoder_in_push_and_finish() {
    let registry = registry_of(&[invalid_in_push, invalid_in_finish, healthy]);
    let records = stream(1, CaptureSource::Native, Some("probe"));
    let (outcome, observations) = run(&registry, &records);

    assert_eq!(
        failed(outcome_of(&outcome, "invalid-push")),
        (DecoderPhase::Push, "invalid_observation_kind")
    );
    assert_eq!(
        failed(outcome_of(&outcome, "invalid-finish")),
        (DecoderPhase::Finish, "invalid_observation_kind")
    );
    assert!(outcome_of(&outcome, "healthy").result.is_ok());
    assert!(
        observations
            .iter()
            .all(|(_, observation)| kinds::ALL.contains(&observation.kind.as_str()))
    );
    assert_eq!(
        observations.len(),
        1,
        "only the healthy peer reaches the sink"
    );
}

#[test]
fn a_failed_decoder_is_never_called_again_and_is_dropped_inside_the_isolation() {
    fn drop_panics() -> Box<dyn DecoderV1> {
        Box::new(Probe::new("drop-panics", At::ErrorThenDropPanics))
    }
    let registry = registry_of(&[drop_panics, healthy]);
    let mut records = stream(1, CaptureSource::Native, Some("probe"));
    records.extend(stream(2, CaptureSource::Native, Some("probe")));
    let (outcome, _) = run(&registry, &records);

    assert_eq!(
        failed(outcome_of(&outcome, "drop-panics")),
        (DecoderPhase::Push, "gave_up")
    );
    assert_eq!(PUSHES_AFTER_FAILURE.load(Ordering::SeqCst), 0);
    // The failed instance's destructor ran, and its panic did not escape the session.
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(outcome_of(&outcome, "healthy").observations.kept, 2);
}

#[test]
fn a_decoder_cannot_report_as_another_and_an_instance_must_match_its_entry() {
    // `healthy` finishes with a summary naming "liar" version 99.
    let (outcome, _) = run(
        &registry_of(&[healthy]),
        &stream(1, CaptureSource::Native, Some("probe")),
    );
    let summary = outcome_of(&outcome, "healthy").result.as_ref().unwrap();
    assert_eq!(
        (
            summary.decoder_id.as_str(),
            summary.decoder_version.as_str()
        ),
        ("healthy", "1")
    );

    // The factory changes its version, or panics, after registration.
    static MADE: AtomicU64 = AtomicU64::new(0);
    let mut registry = DecoderRegistry::empty();
    registry
        .register(Registered::new(
            || {
                let mut probe = Probe::new("shifty", At::Never);
                if MADE.fetch_add(1, Ordering::SeqCst) > 0 {
                    probe.version = "2";
                }
                Box::new(probe)
            },
            "unit",
            &[],
        ))
        .unwrap();
    static BOOMS: AtomicU64 = AtomicU64::new(0);
    registry
        .register(Registered::new(
            || {
                assert!(
                    BOOMS.fetch_add(1, Ordering::SeqCst) == 0,
                    "factory exploded"
                );
                Box::new(Probe::new("boom", At::Never))
            },
            "unit",
            &[],
        ))
        .unwrap();
    let (outcome, _) = run(&registry, &stream(1, CaptureSource::Native, Some("probe")));
    assert_eq!(
        failed(&outcome.decoders[0]),
        (DecoderPhase::Create, "identity_changed")
    );
    assert_eq!(
        failed(&outcome.decoders[1]),
        (DecoderPhase::Create, "panicked")
    );
}

#[test]
fn a_factory_can_capture_configuration() {
    let limit = Arc::new(AtomicU64::new(7));
    let seen = Arc::clone(&limit);
    let registered = Registered::new(
        move || {
            seen.fetch_add(1, Ordering::SeqCst);
            healthy()
        },
        "unit",
        &["a synthetic form"],
    );
    // Every instance of a decode comes from the captured closure.
    let mut registry = DecoderRegistry::empty();
    registry.register(registered.clone()).unwrap();
    let before = limit.load(Ordering::SeqCst);
    run(&registry, &[]);
    assert!(limit.load(Ordering::SeqCst) > before);
    assert_eq!(
        (
            registered.id(),
            registered.version(),
            registered.measured_on()
        ),
        ("healthy", "1", "unit")
    );
    assert_eq!(registered.synthetic_only(), ["a synthetic form"]);
}

// ---------------------------------------------------------------------------------------
// The sink.
// ---------------------------------------------------------------------------------------

#[test]
fn a_sink_error_ends_the_session_for_good() {
    let registry = registry_of(&[healthy]);
    let mut session = DecodeSession::new(&registry, |_: &'static str, _: Observation| {
        Err::<Disposition, _>(io::Error::new(io::ErrorKind::StorageFull, "disk full"))
    });
    let records = stream(1, CaptureSource::Native, Some("probe"));
    session.push(&records[0]).unwrap();
    assert_eq!(
        session.push(&records[1]).unwrap_err().to_string(),
        "disk full"
    );
    // Later calls replay the error, whatever they are.
    let again = session.push(&records[2]).unwrap_err();
    assert_eq!(again.kind(), io::ErrorKind::StorageFull);
    let finished = session.finish().err().expect("finish after a sink error");
    assert_eq!(finished.kind(), io::ErrorKind::StorageFull);
}

#[test]
fn a_sink_that_drops_observations_is_counted_over_budget() {
    let registry = registry_of(&[healthy]);
    let (outcome, _) = decode_records(
        stream(1, CaptureSource::Native, Some("probe"))
            .into_iter()
            .map(Ok),
        &registry,
        |_: &'static str, _: Observation| Ok(Disposition::OverBudget),
    )
    .unwrap();
    let healthy = outcome_of(&outcome, "healthy");
    assert_eq!(healthy.observations.over_budget, 1);
    assert_eq!(healthy.observations.kept, 0);
}

// ---------------------------------------------------------------------------------------
// Selection, capture-only and routing.
// ---------------------------------------------------------------------------------------

#[test]
fn a_decoder_is_given_only_the_streams_it_accepts() {
    let registry = registry_of(&[healthy]);
    let mut records = stream(1, CaptureSource::Native, Some("something-else"));
    records.extend(stream(2, CaptureSource::Native, Some("probe")));
    let (outcome, observations) = run(&registry, &records);
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].1.source.stream, key(2));
    assert_eq!(
        outcome_of(&outcome, "healthy")
            .result
            .as_ref()
            .unwrap()
            .observations,
        1
    );
}

#[test]
fn an_unknown_hint_is_capture_only_and_named() {
    let mut records = stream(1, CaptureSource::WireRequest, Some("x-some-new-protocol"));
    records.extend(stream(
        2,
        CaptureSource::WireResponse,
        Some("x-some-new-protocol"),
    ));
    records.extend(stream(3, CaptureSource::Native, None));
    // The goal is not capture-only.
    records.extend(stream(4, CaptureSource::Goal, None));
    let (outcome, observations) = run(&DecoderRegistry::builtin(), &records);

    assert!(observations.is_empty(), "no decoder read an unknown hint");
    for decoder in &outcome.decoders {
        let summary = decoder.result.as_ref().unwrap();
        assert_eq!((summary.observations, summary.unknown_records), (0, 0));
        assert!(!summary.incomplete);
    }
    let names: Vec<_> = outcome
        .undecoded
        .iter()
        .map(|u| (u.source, u.protocol_hint.as_deref(), u.streams, u.bytes))
        .collect();
    assert_eq!(
        names,
        [
            (
                CaptureSource::WireRequest,
                Some("x-some-new-protocol"),
                1,
                7
            ),
            (
                CaptureSource::WireResponse,
                Some("x-some-new-protocol"),
                1,
                7
            ),
            (CaptureSource::Native, None, 1, 7),
        ]
    );
    assert!(outcome.undecoded.iter().all(|u| !u.bytes_incomplete));
    assert_eq!(outcome.undecoded_omitted, 0);
    // Record counts are the session's, whoever decodes.
    assert_eq!(outcome.operations.counts()["segment"], 4);
}

#[test]
fn a_known_hint_on_a_source_its_decoder_does_not_read_is_capture_only() {
    // The Responses decoder reads wire and WebSocket sources, not a native store.
    let records = stream(1, CaptureSource::Native, Some("openai-responses"));
    let (outcome, observations) = run(&DecoderRegistry::builtin(), &records);
    assert!(observations.is_empty());
    assert_eq!(outcome.undecoded.len(), 1);
}

#[test]
fn stream_less_records_reach_every_live_decoder() {
    /// Counts the records it was pushed, by kind of record that has no stream.
    struct Counter {
        id: &'static str,
        hint: &'static str,
        stream_less: u64,
        streamed: u64,
    }
    impl DecoderV1 for Counter {
        fn id(&self) -> &'static str {
            self.id
        }
        fn version(&self) -> &'static str {
            "1"
        }
        fn accepts(&self, open: &StreamOpen<'_>) -> bool {
            open.protocol_hint == Some(self.hint)
        }
        fn push(
            &mut self,
            record: &CaptureRecord,
            _: &mut dyn FnMut(Observation),
        ) -> Result<(), DecoderFailure> {
            match record {
                CaptureRecord::Open { .. }
                | CaptureRecord::Segment { .. }
                | CaptureRecord::End { .. }
                | CaptureRecord::Gap {
                    stream: Some(_), ..
                } => self.streamed += 1,
                _ => self.stream_less += 1,
            }
            Ok(())
        }
        fn finish(
            self: Box<Self>,
            _: &mut dyn FnMut(Observation),
        ) -> Result<DecoderSummary, DecoderFailure> {
            Ok(DecoderSummary::new(self.stream_less, self.streamed, false))
        }
    }
    let mut registry = DecoderRegistry::empty();
    for (id, hint) in [("counts-a", "a"), ("counts-b", "b")] {
        registry
            .register(Registered::new(
                move || {
                    Box::new(Counter {
                        id,
                        hint,
                        stream_less: 0,
                        streamed: 0,
                    })
                },
                "unit",
                &[],
            ))
            .unwrap();
    }
    let mut records = stream(1, CaptureSource::Native, Some("a")); // open, segment, end
    records.push(CaptureRecord::Gap {
        stream: Some(key(1)),
        observed: at(5),
        reason: GapReason::QueueFull,
        lost_bytes: None,
    });
    records.push(CaptureRecord::Gap {
        stream: None,
        observed: at(6),
        reason: GapReason::QueueFull,
        lost_bytes: None,
    });
    records.push(CaptureRecord::ProducerClosed {
        producer: PRODUCER.to_string(),
        observed: at(7),
        final_seq: Some(2),
    });
    let (outcome, _) = run(&registry, &records);
    let a = outcome_of(&outcome, "counts-a").result.as_ref().unwrap();
    let b = outcome_of(&outcome, "counts-b").result.as_ref().unwrap();
    // Both saw the two stream-less records; only `a` was given the stream's four records
    // (the keyed gap follows the stream's End, so it is no longer routed: three).
    assert_eq!((a.observations, a.unknown_records), (2, 3));
    assert_eq!((b.observations, b.unknown_records), (2, 0));
}

// ---------------------------------------------------------------------------------------
// The routing bound and pruning.
// ---------------------------------------------------------------------------------------

fn opens(producer: &str, from: u64, count: usize, source: CaptureSource) -> Vec<CaptureRecord> {
    (from..from + count as u64)
        .map(|n| open_in(producer_stream(producer, n), source, Some("probe")))
        .collect()
}

#[test]
fn an_open_past_the_routing_bound_fails_the_decoder_and_is_never_dropped_silently() {
    let registry = registry_of(&[healthy]);
    let mut records = opens(PRODUCER, 1, MAX_OPEN_STREAMS + 2, CaptureSource::Native);
    for n in 1..=(MAX_OPEN_STREAMS as u64 + 2) {
        records.push(segment(n, b"x"));
    }
    let (outcome, observations) = run(&registry, &records);
    let healthy = outcome_of(&outcome, "healthy");
    assert_eq!(
        failed(healthy),
        (DecoderPhase::Push, "too_many_open_streams")
    );
    // Nothing was read past the bound: the segments of the streams routed before it were.
    assert!(observations.len() <= MAX_OPEN_STREAMS);
}

#[test]
fn a_recovery_of_one_wire_producer_leaves_another_live_one_routed() {
    let registry = registry_of(&[healthy]);
    // Two live wire producers; the reader accepts a recorded Recovery of one of them and
    // leaves the other's streams valid.
    let a = |n| producer_stream("aaaa", n);
    let c = |n| producer_stream("cccc", n);
    let records = vec![
        open_in(a(1), CaptureSource::WsClient, Some("probe")),
        open_in(c(1), CaptureSource::WsClient, Some("probe")),
        segment_in(c(1), b"before"),
        CaptureRecord::Recovery {
            previous_producer: "cccc".to_string(),
            producer: "rrrr".to_string(),
            observed: at(9),
            reason: GapReason::IngestLost,
        },
        // The recovered producer's stream is gone; the other's is not.
        segment_in(c(1), b"after the recovery, from a lost producer"),
        segment_in(a(1), b"still routed"),
        CaptureRecord::End {
            stream: a(1),
            observed: at(10),
            reason: EndReason::Complete,
        },
    ];
    let (outcome, observations) = run(&registry, &records);
    assert!(outcome_of(&outcome, "healthy").result.is_ok());
    let streams: Vec<_> = observations
        .iter()
        .map(|(_, o)| (o.source.stream.producer.as_str(), o.source.stream.stream))
        .collect();
    assert_eq!(streams, [("cccc", 1), ("aaaa", 1)]);
}

#[test]
fn a_recovery_prunes_every_wire_stream_and_a_producer_close_prunes_its_own() {
    let registry = registry_of(&[healthy]);
    // A full house of open wire streams under one producer, never ended.
    let mut records = opens("aaaa", 1, MAX_OPEN_STREAMS, CaptureSource::WsClient);
    // An unrecorded Recovery: the named predecessor was never seen, and the reader still
    // treats every unclosed wire producer as lost.
    records.push(CaptureRecord::Recovery {
        previous_producer: "never-seen".to_string(),
        producer: "bbbb".to_string(),
        observed: at(9),
        reason: GapReason::IngestLost,
    });
    records.extend(opens("bbbb", 1, MAX_OPEN_STREAMS, CaptureSource::WsClient));
    // A closed producer's streams stop counting at once.
    records.push(CaptureRecord::ProducerClosed {
        producer: "bbbb".to_string(),
        observed: at(10),
        final_seq: Some(1),
    });
    records.extend(opens("cccc", 1, MAX_OPEN_STREAMS, CaptureSource::WsClient));
    let (outcome, _) = run(&registry, &records);
    assert!(
        outcome_of(&outcome, "healthy").result.is_ok(),
        "stale keys of a lost or closed producer filled the bound: {:?}",
        outcome_of(&outcome, "healthy").result
    );

    // A native stream is not a wire stream: a Recovery does not end it.
    let mut records = opens("native", 1, 1, CaptureSource::Native);
    records.push(CaptureRecord::Recovery {
        previous_producer: "never-seen".to_string(),
        producer: "dddd".to_string(),
        observed: at(9),
        reason: GapReason::IngestLost,
    });
    records.push(segment_in(producer_stream("native", 1), b"still routed"));
    let (outcome, observations) = run(&registry, &records);
    assert!(outcome_of(&outcome, "healthy").result.is_ok());
    assert_eq!(observations.len(), 1);

    // Source classification is per stream in the non-validating session. A producer that
    // happens to own both native and wire streams loses only the wire stream in an unrecorded
    // recovery cascade.
    let mixed = |n| producer_stream("mixed", n);
    let records = vec![
        open_in(mixed(1), CaptureSource::Native, Some("probe")),
        open_in(mixed(2), CaptureSource::WsClient, Some("probe")),
        CaptureRecord::Recovery {
            previous_producer: "never-seen".to_string(),
            producer: "eeee".to_string(),
            observed: at(9),
            reason: GapReason::IngestLost,
        },
        segment_in(mixed(1), b"native survives"),
        segment_in(mixed(2), b"wire is lost"),
    ];
    let (outcome, observations) = run(&registry, &records);
    assert!(outcome_of(&outcome, "healthy").result.is_ok());
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].1.source.stream, mixed(1));
}

#[test]
fn an_external_decoder_fails_closed_when_recovery_expands_past_the_named_predecessor() {
    let registry = registry_of(&[legacy_healthy]);
    let live = producer_stream("recorded-wire", 1);
    let records = vec![
        open_in(live, CaptureSource::WsClient, Some("probe")),
        CaptureRecord::Recovery {
            previous_producer: "never-recorded".to_string(),
            producer: "fresh-wire".to_string(),
            observed: at(9),
            reason: GapReason::IngestLost,
        },
    ];
    let (outcome, _) = run(&registry, &records);
    assert_eq!(
        failed(outcome_of(&outcome, "legacy-healthy")),
        (DecoderPhase::Push, "recovery_scope_unsupported")
    );
}

#[test]
fn a_legacy_decoder_delegates_recorded_predecessor_recovery_to_push() {
    let registry = registry_of(&[legacy_healthy]);
    let recorded = producer_stream("recorded-wire", 1);
    let records = vec![
        open_in(recorded, CaptureSource::WsClient, Some("probe")),
        CaptureRecord::Recovery {
            previous_producer: "recorded-wire".to_string(),
            producer: "fresh-wire".to_string(),
            observed: at(9),
            reason: GapReason::IngestLost,
        },
    ];
    let (outcome, observations) = run(&registry, &records);
    let legacy = outcome_of(&outcome, "legacy-healthy");
    assert!(legacy.result.is_ok());
    assert_eq!(legacy.result.as_ref().unwrap().observations, 1);
    assert!(observations.is_empty());
}

// ---------------------------------------------------------------------------------------
// The registry.
// ---------------------------------------------------------------------------------------

#[test]
fn decoder_acceptance_is_the_authoritative_hint_registry() {
    let registry = DecoderRegistry::builtin();
    let key = key(1);
    let sources = [
        CaptureSource::Goal,
        CaptureSource::Native,
        CaptureSource::WireRequest,
        CaptureSource::WireResponse,
        CaptureSource::WsClient,
        CaptureSource::WsServer,
    ];
    let rows: &[(&str, &str, &[CaptureSource])] = &[
        ("codex-native", "codex-jsonl", &[CaptureSource::Native]),
        (
            "claude-code-native",
            "claude-code-jsonl",
            &[CaptureSource::Native],
        ),
        (
            "openai-responses",
            "openai-responses",
            &[
                CaptureSource::WireRequest,
                CaptureSource::WireResponse,
                CaptureSource::WsClient,
                CaptureSource::WsServer,
            ],
        ),
        // Anthropic deliberately selects by hint regardless of source. Unsupported source,
        // transport, and endpoint forms are observations, not selection failures.
        ("anthropic-messages", "anthropic-messages", &sources),
        (
            "openai-chat-completions",
            "openai-chat-completions",
            &[CaptureSource::WireRequest, CaptureSource::WireResponse],
        ),
        (
            "amp-thread-jsonrpc",
            "amp-thread-jsonrpc",
            &[CaptureSource::WsClient, CaptureSource::WsServer],
        ),
    ];
    let mut decoders_with_a_hint = std::collections::BTreeSet::new();
    for (decoder, hint, accepted_sources) in rows {
        assert!(valid_hint(hint));
        for source in sources {
            let accepting = registry.accepting(&StreamOpen::new(&key, source, Some(hint), None));
            let expected = if accepted_sources.contains(&source) {
                vec![*decoder]
            } else {
                vec![]
            };
            assert_eq!(accepting, expected, "{hint} on {source:?}");
            assert!(accepting.len() <= 1, "two decoders read {hint}");
            decoders_with_a_hint.extend(accepting);
        }
    }
    let all: std::collections::BTreeSet<_> = registry.ids().into_iter().map(|(id, _)| id).collect();
    assert_eq!(decoders_with_a_hint, all);
    for source in sources {
        assert!(
            registry
                .accepting(&StreamOpen::new(
                    &key,
                    source,
                    Some("x-not-in-the-registry"),
                    None
                ))
                .is_empty()
        );
    }
}

#[test]
fn observation_kind_vocabulary_is_closed_and_unique() {
    let kinds: std::collections::BTreeSet<_> = kinds::ALL.iter().copied().collect();
    assert_eq!(kinds.len(), kinds::ALL.len());
    assert_eq!(
        kinds,
        [
            "error",
            "human_message",
            "model",
            "model_request",
            "model_response",
            "outcome",
            "tool_call",
            "tool_result",
            "unknown",
            "usage",
        ]
        .into_iter()
        .collect()
    );
}

#[test]
fn the_registry_refuses_a_duplicate_or_malformed_id() {
    let mut registry = registry_of(&[healthy]);
    assert_eq!(
        registry.register(Registered::new(healthy, "unit", &[])),
        Err(RegistryError::DuplicateId("healthy"))
    );
    fn upper() -> Box<dyn DecoderV1> {
        Box::new(Probe::new("Upper", At::Never))
    }
    assert_eq!(
        registry.register(Registered::new(upper, "unit", &[])),
        Err(RegistryError::InvalidId("Upper"))
    );
    assert_eq!(registry.ids(), [("healthy", "1")]);
}

#[test]
fn the_builtin_registry_lists_the_six_decoders_in_report_order() {
    assert_eq!(DECODER_API_VERSION, 1);
    assert_eq!(
        DecoderRegistry::builtin().ids(),
        [
            ("codex-native", "3"),
            ("claude-code-native", "2.1.259-2.1.283"),
            ("openai-responses", "2"),
            ("anthropic-messages", "3"),
            ("openai-chat-completions", "2"),
            ("amp-thread-jsonrpc", "2"),
        ]
    );
    assert!(DecoderRegistry::empty().ids().is_empty());
}

// ---------------------------------------------------------------------------------------
// Threads.
// ---------------------------------------------------------------------------------------

fn assert_send<T: Send>() {}
fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn registry_and_session_are_usable_from_worker_threads() {
    assert_send_sync::<DecoderRegistry>();
    assert_send_sync::<Registered>();
    assert_send::<Box<dyn DecoderV1>>();
    assert_send::<DecodeSession<Collect>>();
    assert_send::<DecodeOutcome>();

    // One registry shared by reference; each thread decodes its own trace.
    let registry = DecoderRegistry::builtin();
    let probe = registry_of(&[healthy]);
    let records = stream(1, CaptureSource::Native, Some("probe"));
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    let (a, _) = run(&registry, &records);
                    let (b, observations) = run(&probe, &records);
                    (a.undecoded.len(), b.decoders.len(), observations.len())
                })
            })
            .collect();
        for handle in handles {
            assert_eq!(handle.join().unwrap(), (1, 1, 1));
        }
    });
}

#[test]
fn an_observation_sink_is_any_closure_returning_a_disposition() {
    let mut kept = 0;
    let registry = registry_of(&[healthy]);
    let (_, sink) = decode_records(
        stream(1, CaptureSource::Native, Some("probe"))
            .into_iter()
            .map(Ok),
        &registry,
        |_: &'static str, _: Observation| {
            kept += 1;
            Ok(Disposition::Kept)
        },
    )
    .unwrap();
    let _ = sink;
    assert_eq!(kept, 1);
    // And a named sink is just a trait impl.
    fn takes_sink(_: &mut impl ObservationSink) {}
    takes_sink(&mut Collect::default());
}
