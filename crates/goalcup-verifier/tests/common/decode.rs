//! Drives decoders through a `DecodeSession` over records held in memory, the way a server
//! does, so a decoder test exercises routing (`accepts`) and the session's isolation too.

use goalcup_protocol::capture::CaptureRecord;
use goalcup_verifier::capture_analysis::api::{
    DecoderSummary, DecoderV1, Disposition, Observation, ObservationSink,
};
use goalcup_verifier::capture_analysis::registry::{DecoderRegistry, Registered};
use goalcup_verifier::capture_analysis::session::{DecodeOutcome, decode_records};
use std::io;

/// Keeps every observation with the decoder that emitted it.
#[derive(Default)]
pub struct Collect(pub Vec<(&'static str, Observation)>);

impl ObservationSink for Collect {
    fn observe(&mut self, id: &'static str, observation: Observation) -> io::Result<Disposition> {
        self.0.push((id, observation));
        Ok(Disposition::Kept)
    }
}

/// A registry of one decoder.
pub fn registry_of_one(make: fn() -> Box<dyn DecoderV1>) -> DecoderRegistry {
    let mut registry = DecoderRegistry::empty();
    registry
        .register(Registered::new(make, "test", &[]))
        .expect("a valid decoder id");
    registry
}

/// `records` through a session over `registry`: the outcome and every observation.
pub fn run(
    registry: &DecoderRegistry,
    records: &[CaptureRecord],
) -> (DecodeOutcome, Vec<(&'static str, Observation)>) {
    let (outcome, sink) = decode_records(
        records.iter().cloned().map(Ok),
        registry,
        Collect::default(),
    )
    .expect("a collecting sink never fails");
    (outcome, sink.0)
}

/// A fresh decoder over `records` in a session of its own: every observation in order, then
/// its summary. A decoder that fails fails the test.
pub fn decode_one(
    make: fn() -> Box<dyn DecoderV1>,
    records: &[CaptureRecord],
) -> (Vec<Observation>, DecoderSummary) {
    let (outcome, observations) = run(&registry_of_one(make), records);
    let decoder = outcome.decoders.into_iter().next().expect("one decoder");
    let summary = decoder
        .result
        .unwrap_or_else(|failed| panic!("{} failed: {failed:?}", decoder.id));
    (observations.into_iter().map(|(_, o)| o).collect(), summary)
}
