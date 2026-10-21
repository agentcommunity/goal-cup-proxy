//! One decode of one trace: [`DecodeSession`] makes a fresh decoder from every factory of a
//! [`DecoderRegistry`], routes each record to the decoders that read it, and hands every
//! observation to an [`ObservationSink`]. [`decode_records`] runs a session over any stream
//! of records.
//!
//! **Routing.** On each `Open`, every decoder's [`DecoderV1::accepts`] is asked, of a selector
//! instance that no decode ever touches. An accepting decoder then receives that stream's
//! `Open`, `Segment`s, keyed `Gap` and `End`; every live decoder receives every stream-less
//! record (`Connection`, `Recovery`, `ProducerClosed`, `Gap` without a stream, `Launch`,
//! `Assistance`, `LocalFinish`). A stream no decoder accepts is capture-only and is counted in
//! [`DecodeOutcome::undecoded`]. A stream stops being routed at its `End`; a producer's
//! streams at its `ProducerClosed`; and at a `Recovery` the recovered producer's streams, and
//! also every wire stream when that producer was never seen (the reader's own rule: an
//! unrecorded predecessor means every unclosed wire producer is lost). At most `MAX_OPEN_STREAMS` streams are routed
//! to a decoder at once, the reader's bound on live streams: an `Open` past it is not
//! delivered and the decoder fails with `too_many_open_streams`, never silently truncated.
//! Input the reader has not validated can exceed it.
//!
//! **Isolation.** The factory and every `accepts`, `push` and `finish` call run under
//! `catch_unwind`. A panic or a [`DecoderFailure`] marks that decoder failed: its instance is
//! dropped inside the same isolation, it is never called again, its outcome is `Err`, and the
//! others continue. A panic's reason is `panicked`, with no payload text, and nothing is
//! printed on the panicking thread: see [`quiet_panic_hook`]. Observations a decoder emitted
//! before failing stay in the sink. A build that sets `panic = "abort"` has no isolation.
//!
//! **Identity.** The registry entry owns a decoder's id and version. An instance that reports
//! another fails at creation (`identity_changed`), and the session stamps the entry's id and
//! version on the summary it returns, so no decoder can report as another.
//!
//! **Sink errors.** A sink error ends the session for good: that `push` and every later
//! `push` and `finish` return `Err`.
//!
//! **Input is the caller's to check.** A session reads the records it is given; it neither
//! validates a chain nor trusts one. The caller composes the integrity scan
//! (`reader::scan_capture_trace`) and [`research_summary`](super::report::research_summary).

use super::api::{
    DecoderFailure, DecoderSummary, DecoderV1, Disposition, Observation, ObservationSink,
    StreamOpen, kinds,
};
use super::lineage::Lineage;
use super::operations::Operations;
use super::registry::{DecoderRegistry, Registered};
use goalcup_protocol::capture::{CaptureRecord, CaptureSource, MAX_OPEN_STREAMS, StreamKey};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::io;
use std::panic::{AssertUnwindSafe, PanicHookInfo, catch_unwind};
use std::sync::Once;

/// The most distinct (source, hint) groups [`DecodeOutcome::undecoded`] keeps; streams of
/// further groups are counted in [`DecodeOutcome::undecoded_omitted`].
pub(crate) const MAX_UNDECODED_GROUPS: usize = 64;

// ---------------------------------------------------------------------------------------
// Quiet panic isolation.
// ---------------------------------------------------------------------------------------

thread_local! {
    /// How many isolated decoder calls the current thread is inside.
    static QUIET: Cell<u32> = const { Cell::new(0) };
}

static HOOK: Once = Once::new();

/// A panic hook that stays silent for a panic inside an isolated decoder call on this thread,
/// and otherwise calls `previous`. The default hook would print the payload, which could carry
/// record text, to stderr.
///
/// The gate is per thread: concurrent sessions do not silence each other or the host. It does
/// not cover a thread a decoder spawns, whose panics print through `previous`; and it also
/// silences a panicking [`ObservationSink`], which runs inside the same gate. A panic anywhere
/// else reaches `previous`.
///
/// A [`DecodeSession`] installs this over the hook current at its first use
/// (`install_quiet_panic_hook`). A host that installs its own hook afterwards replaces it, and
/// from then on its hook sees decoder panics, payload included; to keep them quiet, wrap it:
/// `set_hook(quiet_panic_hook(take_hook()))`. The gate is shared, so wrapping again is safe.
pub fn quiet_panic_hook(
    previous: Box<dyn Fn(&PanicHookInfo<'_>) + Send + Sync + 'static>,
) -> Box<dyn Fn(&PanicHookInfo<'_>) + Send + Sync + 'static> {
    Box::new(move |info| {
        if QUIET.with(Cell::get) == 0 {
            previous(info);
        }
    })
}

/// Wraps the current panic hook with [`quiet_panic_hook`], once per process. A session calls
/// this before it runs a decoder; a host may call it first to choose when the wrapping
/// happens, relative to its own hook.
pub(crate) fn install_quiet_panic_hook() {
    HOOK.call_once(|| std::panic::set_hook(quiet_panic_hook(std::panic::take_hook())));
}

struct QuietGuard;

impl QuietGuard {
    fn enter() -> Self {
        QUIET.with(|q| q.set(q.get() + 1));
        Self
    }
}

impl Drop for QuietGuard {
    fn drop(&mut self) {
        QUIET.with(|q| q.set(q.get() - 1));
    }
}

/// A caught panic. Carries no payload.
pub(super) struct Panicked;

/// Runs `f`, catching a panic silently.
pub(super) fn quiet<T>(f: impl FnOnce() -> T) -> Result<T, Panicked> {
    install_quiet_panic_hook();
    let _gate = QuietGuard::enter();
    catch_unwind(AssertUnwindSafe(f)).map_err(|_| Panicked)
}

/// Drops a decoder instance inside the isolation. While this thread is already unwinding a
/// panic, a decoder whose destructor panics would abort the process either way, so it is
/// dropped plainly.
fn discard(decoder: Box<dyn DecoderV1>) {
    if std::thread::panicking() {
        drop(decoder);
    } else {
        let _ = quiet(move || drop(decoder));
    }
}

// ---------------------------------------------------------------------------------------
// The outcome.
// ---------------------------------------------------------------------------------------

/// Where a decoder failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecoderPhase {
    /// Its factory, or the identity of the instance it made.
    Create,
    /// Its `accepts`.
    Accepts,
    /// Its `push`, or the session refusing a stream it could not route.
    Push,
    /// Its `finish`.
    Finish,
}

impl DecoderPhase {
    /// The word a report prints.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Accepts => "accepts",
            Self::Push => "push",
            Self::Finish => "finish",
        }
    }
}

/// How a decoder failed, and in which call.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecoderFailed {
    pub phase: DecoderPhase,
    pub failure: DecoderFailure,
}

/// What the sink did with one decoder's observations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ObservationCounts {
    /// Observations the sink kept ([`Disposition::Kept`]).
    pub kept: u64,
    /// Observations the sink dropped as over its size bound ([`Disposition::OverBudget`]).
    pub over_budget: u64,
    /// Kept `model_request` observations of wire streams: model calls the recording proxy recorded.
    pub wire_model_requests: u64,
    /// Kept native-stream observations of model activity: a `model` that is not a switch, a
    /// `tool_call` or a `usage`. A session file that names model work has some; a harness may
    /// write several per call, so this is compared with the wire leg only for zero against
    /// non-zero.
    pub native_model_activity: u64,
    /// The model identities kept `model` observations supply (`fields.model`), with how many
    /// observations name each: at most [`MAX_MODEL_NAMES`] names of at most
    /// [`MAX_MODEL_NAME_BYTES`] bytes; the rest are counted in `models_omitted`.
    pub models: BTreeMap<String, u64>,
    /// `model` observations whose name was not kept: past the bound, too long or empty.
    pub models_omitted: u64,
}

/// The most distinct model names one decoder's counts keep.
pub const MAX_MODEL_NAMES: usize = 16;
/// The longest model name kept.
pub const MAX_MODEL_NAME_BYTES: usize = 128;

impl ObservationCounts {
    /// Counts one kept observation of a stream that is a wire stream (`Some(true)`), a native
    /// one (`Some(false)`), or no longer routed (`None`, counted in neither leg).
    fn tally(&mut self, kind: &str, model: Option<&str>, switch: bool, wire: Option<bool>) {
        match (kind, wire) {
            (kinds::MODEL_REQUEST, Some(true)) => self.wire_model_requests += 1,
            (kinds::MODEL, Some(false)) if !switch => self.native_model_activity += 1,
            (kinds::TOOL_CALL | kinds::USAGE, Some(false)) => self.native_model_activity += 1,
            _ => {}
        }
        if kind != kinds::MODEL {
            return;
        }
        let Some(name) = model else {
            return;
        };
        if name.is_empty() || name.len() > MAX_MODEL_NAME_BYTES {
            self.models_omitted += 1;
        } else if let Some(count) = self.models.get_mut(name) {
            *count += 1;
        } else if self.models.len() < MAX_MODEL_NAMES {
            self.models.insert(name.to_string(), 1);
        } else {
            self.models_omitted += 1;
        }
    }
}

/// One decoder's result over one trace.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DecoderOutcome {
    pub id: &'static str,
    pub version: &'static str,
    pub measured_on: &'static str,
    pub synthetic_only: &'static [&'static str],
    /// `Err` when the decoder failed. Its earlier observations are still in the sink, and
    /// `observations` counts them. The summary's identity is the registry entry's.
    pub result: Result<DecoderSummary, DecoderFailed>,
    pub observations: ObservationCounts,
}

/// Streams no decoder accepted, grouped by source and hint: recorded, not read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Undecoded {
    pub source: CaptureSource,
    pub protocol_hint: Option<String>,
    pub streams: u64,
    /// Segment source bytes of these streams.
    pub bytes: u64,
    /// `bytes` is a lower bound: more than `MAX_OPEN_STREAMS` capture-only streams were live
    /// at once, and the later ones' bytes are not counted.
    pub bytes_incomplete: bool,
}

/// Everything one decode learned besides the observations.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DecodeOutcome {
    /// One per registered decoder, in registry order.
    pub decoders: Vec<DecoderOutcome>,
    /// Capture-only streams, in first-seen order, at most `MAX_UNDECODED_GROUPS` groups.
    /// The goal stream is not capture-only and is not listed.
    pub undecoded: Vec<Undecoded>,
    /// Capture-only streams of groups past the bound.
    pub undecoded_omitted: u64,
    /// The record counts, launch and references of the records read.
    pub operations: Operations,
}

// ---------------------------------------------------------------------------------------
// The session.
// ---------------------------------------------------------------------------------------

pub(crate) fn is_wire(source: CaptureSource) -> bool {
    matches!(
        source,
        CaptureSource::WireRequest
            | CaptureSource::WireResponse
            | CaptureSource::WsClient
            | CaptureSource::WsServer
    )
}

/// One registered decoder's live state in a session.
struct Slot {
    id: &'static str,
    version: &'static str,
    measured_on: &'static str,
    synthetic_only: &'static [&'static str],
    /// The instance that reads the trace. `None` once finished or failed.
    decoder: Option<Box<dyn DecoderV1>>,
    /// A second instance that is only ever asked `accepts`, so selection never touches state a
    /// failed decode left behind. `None` after its `accepts` panicked.
    selector: Option<Box<dyn DecoderV1>>,
    failed: Option<DecoderFailed>,
    /// Streams routed to `decoder`, each with whether it is a wire stream.
    routed: BTreeMap<StreamKey, bool>,
    counts: ObservationCounts,
    summary: Option<DecoderSummary>,
}

fn panicked() -> DecoderFailure {
    DecoderFailure::new("panicked")
}

impl Slot {
    fn new(entry: &Registered) -> Self {
        let mut slot = Self {
            id: entry.id(),
            version: entry.version(),
            measured_on: entry.measured_on(),
            synthetic_only: entry.synthetic_only(),
            decoder: None,
            selector: None,
            failed: None,
            routed: BTreeMap::new(),
            counts: ObservationCounts::default(),
            summary: None,
        };
        // Two separate calls: if the second factory call panics, the first instance is
        // dropped here, outside any unwinding, inside the isolation.
        let same = |d: &dyn DecoderV1| d.id() == entry.id() && d.version() == entry.version();
        let made = quiet(|| {
            let decoder = entry.make();
            let ok = same(&*decoder);
            (decoder, ok)
        });
        let selector = quiet(|| {
            let selector = entry.make();
            let ok = same(&*selector);
            (selector, ok)
        });
        match (made, selector) {
            (Ok((decoder, true)), Ok((selector, true))) => {
                slot.decoder = Some(decoder);
                slot.selector = Some(selector);
            }
            (Ok((decoder, ok)), Ok((selector, other))) => {
                debug_assert!(!(ok && other));
                discard(decoder);
                discard(selector);
                slot.fail(
                    DecoderPhase::Create,
                    DecoderFailure::new("identity_changed"),
                );
            }
            (made, selector) => {
                if let Ok((decoder, _)) = made {
                    discard(decoder);
                }
                if let Ok((selector, _)) = selector {
                    discard(selector);
                }
                slot.fail(DecoderPhase::Create, panicked());
            }
        }
        slot
    }

    /// Records the first failure and drops the instance that failed, inside the isolation.
    fn fail(&mut self, phase: DecoderPhase, failure: DecoderFailure) {
        self.failed.get_or_insert(DecoderFailed { phase, failure });
        if let Some(decoder) = self.decoder.take() {
            discard(decoder);
        }
        self.routed.clear();
    }

    fn live(&self) -> bool {
        self.failed.is_none() && self.decoder.is_some()
    }

    /// Whether this decoder reads the stream, answered by the selector instance, so a stream
    /// a failed decoder covered is still not reported capture-only. A selector whose
    /// `accepts` panics reads nothing, and fails the decoder.
    fn reads(&mut self, open: &StreamOpen<'_>) -> bool {
        let Some(selector) = self.selector.as_ref() else {
            return false;
        };
        match quiet(|| selector.accepts(open)) {
            Ok(accepted) => accepted,
            Err(Panicked) => {
                if let Some(selector) = self.selector.take() {
                    discard(selector);
                }
                self.fail(DecoderPhase::Accepts, panicked());
                false
            }
        }
    }

    /// `Err` only for a sink failure; a decoder error or panic stops this decoder alone.
    fn push<S: ObservationSink>(&mut self, record: &CaptureRecord, sink: &mut S) -> io::Result<()> {
        if !self.live() {
            return Ok(());
        }
        let Some(decoder) = self.decoder.as_mut() else {
            return Ok(());
        };
        let mut delivery = Delivery::new(sink, self.id, &mut self.counts, &self.routed);
        let pushed = quiet(|| decoder.push(record, &mut |o| delivery.observe(o)));
        if delivery.result()? {
            self.fail(
                DecoderPhase::Push,
                DecoderFailure::new("invalid_observation_kind"),
            );
            return Ok(());
        }
        match pushed {
            Ok(Ok(())) => {}
            Ok(Err(failure)) => self.fail(DecoderPhase::Push, failure),
            Err(Panicked) => self.fail(DecoderPhase::Push, panicked()),
        }
        Ok(())
    }

    /// Delivers the session-resolved Recovery scope before routed keys are removed.
    fn push_recovery<S: ObservationSink>(
        &mut self,
        record: &CaptureRecord,
        lost_streams: &[StreamKey],
        sink: &mut S,
    ) -> io::Result<()> {
        if !self.live() {
            return Ok(());
        }
        let Some(decoder) = self.decoder.as_mut() else {
            return Ok(());
        };
        let mut delivery = Delivery::new(sink, self.id, &mut self.counts, &self.routed);
        let pushed =
            quiet(|| decoder.push_recovery(record, lost_streams, &mut |o| delivery.observe(o)));
        if delivery.result()? {
            self.fail(
                DecoderPhase::Push,
                DecoderFailure::new("invalid_observation_kind"),
            );
            return Ok(());
        }
        match pushed {
            Ok(Ok(())) => {}
            Ok(Err(failure)) => self.fail(DecoderPhase::Push, failure),
            Err(Panicked) => self.fail(DecoderPhase::Push, panicked()),
        }
        Ok(())
    }

    fn finish<S: ObservationSink>(&mut self, sink: &mut S) -> io::Result<()> {
        if !self.live() {
            return Ok(());
        }
        let Some(decoder) = self.decoder.take() else {
            return Ok(());
        };
        let mut delivery = Delivery::new(sink, self.id, &mut self.counts, &self.routed);
        let finished = quiet(|| decoder.finish(&mut |o| delivery.observe(o)));
        if delivery.result()? {
            self.fail(
                DecoderPhase::Finish,
                DecoderFailure::new("invalid_observation_kind"),
            );
            return Ok(());
        }
        match finished {
            Ok(Ok(mut summary)) => {
                summary.decoder_id = self.id.to_string();
                summary.decoder_version = self.version.to_string();
                self.summary = Some(summary);
            }
            Ok(Err(failure)) => self.fail(DecoderPhase::Finish, failure),
            Err(Panicked) => self.fail(DecoderPhase::Finish, panicked()),
        }
        Ok(())
    }

    fn outcome(mut self) -> DecoderOutcome {
        let result = match (self.failed.take(), self.summary.take()) {
            (Some(failed), _) => Err(failed),
            (None, Some(summary)) => Ok(summary),
            (None, None) => Err(DecoderFailed {
                phase: DecoderPhase::Finish,
                failure: DecoderFailure::new("not_finished"),
            }),
        };
        DecoderOutcome {
            id: self.id,
            version: self.version,
            measured_on: self.measured_on,
            synthetic_only: self.synthetic_only,
            result,
            observations: std::mem::take(&mut self.counts),
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(decoder) = self.decoder.take() {
            discard(decoder);
        }
        if let Some(selector) = self.selector.take() {
            discard(selector);
        }
    }
}

/// Hands one decoder's observations to the sink as they are emitted. The first sink error is
/// kept and every later observation of the same call is dropped, so the session can stop on
/// it. A sink that panics is a sink error, never the decoder's failure.
struct Delivery<'a, S> {
    sink: &'a mut S,
    decoder: &'static str,
    counts: &'a mut ObservationCounts,
    /// The decoder's routed streams, so a kept observation is counted for its leg.
    routed: &'a BTreeMap<StreamKey, bool>,
    error: Option<io::Error>,
    invalid_kind: bool,
}

impl<'a, S: ObservationSink> Delivery<'a, S> {
    fn new(
        sink: &'a mut S,
        decoder: &'static str,
        counts: &'a mut ObservationCounts,
        routed: &'a BTreeMap<StreamKey, bool>,
    ) -> Self {
        Self {
            sink,
            decoder,
            counts,
            routed,
            error: None,
            invalid_kind: false,
        }
    }

    fn observe(&mut self, observation: Observation) {
        if self.error.is_some() || self.invalid_kind {
            return;
        }
        if !kinds::ALL.contains(&observation.kind.as_str()) {
            self.invalid_kind = true;
            return;
        }
        let (sink, decoder) = (&mut *self.sink, self.decoder);
        // The sink takes the observation, so the leg tally keeps the two small facts it needs.
        let wire = self.routed.get(&observation.source.stream).copied();
        let kind = observation.kind.clone();
        let (model, switch) = if kind == kinds::MODEL {
            let fields = &observation.fields;
            (
                fields
                    .get("model")
                    .and_then(|m| m.as_str())
                    .map(str::to_owned),
                fields.get("via").and_then(|v| v.as_str()) == Some("switch"),
            )
        } else {
            (None, false)
        };
        match quiet(|| sink.observe(decoder, observation)) {
            Err(Panicked) => self.error = Some(io::Error::other("the observation sink panicked")),
            Ok(Err(error)) => self.error = Some(error),
            Ok(Ok(Disposition::OverBudget)) => self.counts.over_budget += 1,
            Ok(Ok(_)) => {
                self.counts.kept += 1;
                self.counts.tally(&kind, model.as_deref(), switch, wire);
            }
        }
    }

    fn result(self) -> io::Result<bool> {
        self.error.map_or(Ok(self.invalid_kind), Err)
    }
}

/// The capture-only streams seen so far.
#[derive(Default)]
struct UndecodedTally {
    groups: Vec<Undecoded>,
    /// Live capture-only streams: the group their bytes count toward (`None`: omitted) and
    /// whether the stream is a wire stream.
    live: BTreeMap<StreamKey, (Option<usize>, bool)>,
    omitted: u64,
}

impl UndecodedTally {
    fn open(&mut self, stream: &StreamKey, source: CaptureSource, hint: Option<&str>) {
        let index = self
            .groups
            .iter()
            .position(|g| g.source == source && g.protocol_hint.as_deref() == hint)
            .or_else(|| {
                (self.groups.len() < MAX_UNDECODED_GROUPS).then(|| {
                    self.groups.push(Undecoded {
                        source,
                        protocol_hint: hint.map(str::to_string),
                        streams: 0,
                        bytes: 0,
                        bytes_incomplete: false,
                    });
                    self.groups.len() - 1
                })
            });
        match index {
            Some(i) => self.groups[i].streams += 1,
            None => self.omitted += 1,
        }
        if self.live.len() < MAX_OPEN_STREAMS || self.live.contains_key(stream) {
            self.live.insert(stream.clone(), (index, is_wire(source)));
        } else if let Some(i) = index {
            self.groups[i].bytes_incomplete = true;
        }
    }

    fn segment(&mut self, stream: &StreamKey, source_len: u64) {
        if let Some((Some(i), _)) = self.live.get(stream) {
            let group = &mut self.groups[*i];
            group.bytes = group.bytes.saturating_add(source_len);
        }
    }
}

/// One decode of one trace. See the module docs for routing, isolation and failure.
pub struct DecodeSession<S: ObservationSink> {
    slots: Vec<Slot>,
    sink: S,
    operations: Operations,
    undecoded: UndecodedTally,
    lineage: Lineage,
    /// The sink error that ended the session, replayed to every later call.
    ended: Option<(io::ErrorKind, String)>,
}

impl<S: ObservationSink> DecodeSession<S> {
    /// A session with a fresh decoder from every factory of `registry`.
    pub fn new(registry: &DecoderRegistry, sink: S) -> Self {
        install_quiet_panic_hook();
        Self {
            slots: registry.entries().iter().map(Slot::new).collect(),
            sink,
            operations: Operations::default(),
            undecoded: UndecodedTally::default(),
            lineage: Lineage::default(),
            ended: None,
        }
    }

    fn check(&self) -> io::Result<()> {
        match &self.ended {
            Some((kind, message)) => Err(io::Error::new(*kind, message.clone())),
            None => Ok(()),
        }
    }

    /// Latches a sink error: the session is over.
    fn end_with(&mut self, result: io::Result<()>) -> io::Result<()> {
        if let Err(error) = &result {
            self.ended = Some((error.kind(), error.to_string()));
        }
        result
    }

    /// Reads one record, in chain order. `Err` only for a sink I/O error, which ends the
    /// session: every later `push` and `finish` return `Err` too. A decoder error or panic
    /// does not.
    pub fn push(&mut self, record: &CaptureRecord) -> io::Result<()> {
        self.check()?;
        let result = self.dispatch(record);
        self.end_with(result)
    }

    /// Notes the producer a record comes from, as the reader admits it.
    fn note_producer(&mut self, producer: &str, wire: bool) {
        self.lineage.note(producer, wire);
    }

    fn dispatch(&mut self, record: &CaptureRecord) -> io::Result<()> {
        self.operations.record(record);
        // A Recovery is judged against the producers seen before it. The session remains
        // non-validating when the bounded identity set is full; the reader rejects that trace.
        let supersession = match record {
            CaptureRecord::Recovery {
                previous_producer,
                producer,
                ..
            } => Some(self.lineage.supersedes(previous_producer, producer)),
            _ => None,
        };
        match record {
            CaptureRecord::Open { stream, source, .. } => {
                self.note_producer(&stream.producer, is_wire(*source))
            }
            CaptureRecord::Segment { stream, .. }
            | CaptureRecord::End { stream, .. }
            | CaptureRecord::Gap {
                stream: Some(stream),
                ..
            } => self.note_producer(&stream.producer, false),
            CaptureRecord::Connection { producer, .. } => self.note_producer(producer, true),
            CaptureRecord::ProducerClosed { producer, .. } => self.note_producer(producer, false),
            _ => {}
        }
        match record {
            CaptureRecord::Open {
                stream,
                source,
                metadata,
                ..
            } => {
                let open = StreamOpen::new(
                    stream,
                    *source,
                    metadata.protocol_hint.as_deref(),
                    metadata.content_type.as_deref(),
                );
                let mut accepted = false;
                for slot in &mut self.slots {
                    if !slot.reads(&open) {
                        continue;
                    }
                    accepted = true;
                    if !slot.live() {
                        continue;
                    }
                    if slot.routed.len() >= MAX_OPEN_STREAMS && !slot.routed.contains_key(stream) {
                        slot.fail(
                            DecoderPhase::Push,
                            DecoderFailure::new("too_many_open_streams"),
                        );
                        continue;
                    }
                    slot.routed.insert(stream.clone(), is_wire(*source));
                    slot.push(record, &mut self.sink)?;
                }
                if !accepted && *source != CaptureSource::Goal {
                    self.undecoded.open(stream, *source, open.protocol_hint);
                }
            }
            CaptureRecord::Segment {
                stream, source_len, ..
            } => {
                self.undecoded.segment(stream, *source_len);
                self.route(stream, record)?;
            }
            CaptureRecord::Gap {
                stream: Some(stream),
                ..
            } => self.route(stream, record)?,
            CaptureRecord::End { stream, .. } => {
                self.route(stream, record)?;
                self.undecoded.live.remove(stream);
                for slot in &mut self.slots {
                    slot.routed.remove(stream);
                }
            }
            CaptureRecord::Recovery { .. } => {
                let lost = |k: &StreamKey, wire: bool| {
                    supersession.as_ref().is_some_and(|supersession| {
                        supersession.producers.contains(&k.producer)
                            && (!supersession.unrecorded || wire)
                    })
                };
                for slot in &mut self.slots {
                    let lost_streams = slot
                        .routed
                        .iter()
                        .filter(|(key, wire)| lost(key, **wire))
                        .map(|(key, _)| key.clone())
                        .collect::<Vec<_>>();
                    slot.push_recovery(record, &lost_streams, &mut self.sink)?;
                    for key in lost_streams {
                        slot.routed.remove(&key);
                    }
                }
                self.undecoded.live.retain(|k, (_, wire)| !lost(k, *wire));
            }
            _ => {
                for slot in &mut self.slots {
                    slot.push(record, &mut self.sink)?;
                }
                if let CaptureRecord::ProducerClosed { producer, .. } = record {
                    self.undecoded.live.retain(|k, _| &k.producer != producer);
                    for slot in &mut self.slots {
                        slot.routed.retain(|k, _| &k.producer != producer);
                    }
                }
            }
        }
        Ok(())
    }

    /// Delivers a stream's record to the live decoders that were given its `Open`.
    fn route(&mut self, stream: &StreamKey, record: &CaptureRecord) -> io::Result<()> {
        for slot in &mut self.slots {
            if slot.routed.contains_key(stream) {
                slot.push(record, &mut self.sink)?;
            }
        }
        Ok(())
    }

    /// Ends every decoder, in registry order, and returns the outcome and the sink. `Err`
    /// when the sink failed, now or earlier.
    pub fn finish(mut self) -> io::Result<(DecodeOutcome, S)> {
        self.check()?;
        let mut result = Ok(());
        for slot in &mut self.slots {
            result = slot.finish(&mut self.sink);
            if result.is_err() {
                break;
            }
        }
        self.end_with(result)?;
        self.operations.finish();
        let outcome = DecodeOutcome {
            decoders: self.slots.into_iter().map(Slot::outcome).collect(),
            undecoded: self.undecoded.groups,
            undecoded_omitted: self.undecoded.omitted,
            operations: self.operations,
        };
        Ok((outcome, self.sink))
    }
}

/// Decodes a stream of records with a fresh session. The first `Err` of `records` ends the
/// decode and is returned. The records are not integrity-checked here.
pub fn decode_records<I, S>(
    records: I,
    registry: &DecoderRegistry,
    sink: S,
) -> io::Result<(DecodeOutcome, S)>
where
    I: IntoIterator<Item = io::Result<CaptureRecord>>,
    S: ObservationSink,
{
    let mut session = DecodeSession::new(registry, sink);
    for record in records {
        session.push(&record?)?;
    }
    session.finish()
}

#[cfg(test)]
mod tally_tests {
    use super::*;

    #[test]
    fn model_counts_are_per_leg_and_bounded() {
        let mut counts = ObservationCounts::default();
        counts.tally(kinds::MODEL_REQUEST, None, false, Some(true));
        counts.tally(kinds::MODEL_REQUEST, None, false, Some(false));
        counts.tally(kinds::MODEL, Some("m-0"), false, Some(false));
        counts.tally(kinds::MODEL, Some("m-0"), true, Some(false));
        counts.tally(kinds::TOOL_CALL, None, false, Some(false));
        counts.tally(kinds::USAGE, None, false, Some(true));
        counts.tally(kinds::MODEL, Some("m-x"), false, None);
        assert_eq!(
            counts.wire_model_requests, 1,
            "a request counts on the wire leg only"
        );
        // A native model (not the switch), a native tool call; not the wire usage.
        assert_eq!(counts.native_model_activity, 2);
        assert_eq!(
            counts.models.get("m-0"),
            Some(&2),
            "a switch still names a model"
        );
        assert_eq!(
            counts.models.get("m-x"),
            Some(&1),
            "names are kept whatever the leg"
        );
        for i in 1..=MAX_MODEL_NAMES {
            counts.tally(kinds::MODEL, Some(&format!("m-{i}")), false, Some(true));
        }
        assert_eq!(counts.models.len(), MAX_MODEL_NAMES);
        counts.tally(
            kinds::MODEL,
            Some(&"x".repeat(MAX_MODEL_NAME_BYTES + 1)),
            false,
            Some(true),
        );
        counts.tally(kinds::MODEL, Some(""), false, Some(true));
        // Two names past the bound (m-15, m-16), one too long, one empty.
        assert_eq!(counts.models_omitted, 4);
    }
}
