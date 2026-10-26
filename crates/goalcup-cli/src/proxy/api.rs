//! The shared seam of the recording proxy: the relay I/O types, the
//! capture policy handed to a relay, the WebSocket hand-off, and [`CaptureEmitter`], the one
//! place a relay turns observations into ordered [`CaptureFrame`]s.
//!
//! [`CaptureEmitter`] is synchronous and bounded. Every method takes one short mutex, holds
//! it through the sink's non-blocking `try_*` call so frames reach the sink in exactly the
//! order their sequence numbers were allocated, and never performs I/O or waits. Relays keep
//! forwarding bytes whatever it answers: a `Disabled` result stops capture, never the relay.
//!
//! **Clock domain.** Every frame the emitter builds is stamped with `clock_id` equal to
//! the producer ID current when it is built: the producer is the clock epoch. Callers supply
//! `wall` and `mono_ns` only; whatever `clock_id` they pass is overwritten. After a recovery
//! the new producer's frames therefore name the new producer's clock, and monotonic values
//! of two epochs are never compared.

use goalcup_protocol::capture::{
    CaptureFrame, CaptureMeta, CaptureSink, CaptureSource, CaptureTime, ConnectionEvent,
    EpochAdmission, FrameEvent, GapReason, MAX_OPEN_CONNECTIONS, MAX_OPEN_STREAMS, MAX_PRODUCERS,
    MAX_SEGMENT_BYTES, StreamKey, SubmitResult, is_capture_id,
};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// Every way the recording proxy relays something it does not capture, and the one [`GapReason`]
/// each is written as in the stream (H-16; CONTRACTS.md, "Proxy loss in the stream (H-16)"). The
/// reasons are the protocol's closed set; several losses share one, and the emitter sends each
/// reason once per emitter, so losses that map to the same reason leave one `Loss` between them.
/// That collapse is written here and nowhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyLoss {
    /// A tunnel to an approved host that its ClientHello did not name, relayed unchanged. Not a
    /// decode failure, but the protocol has no "uncaptured by policy" reason, and what the
    /// verifier needs is that the stream is incomplete.
    FailOpenTunnel,
    /// An intercepted connection relayed as bytes because the capture table was full.
    CaptureTableFull,
    /// An intercepted connection whose relay stopped parsing and went on as bytes. Shares
    /// [`GapReason::DecodeFailed`] with [`ProxyLoss::FailOpenTunnel`], so the two leave one `Loss`.
    OpaqueFallback,
    /// A connection cut by the shutdown before it settled.
    CutUnsettled,
}

impl ProxyLoss {
    /// The reason this loss is chained as.
    pub fn gap_reason(self) -> GapReason {
        match self {
            ProxyLoss::FailOpenTunnel | ProxyLoss::OpaqueFallback => GapReason::DecodeFailed,
            ProxyLoss::CaptureTableFull => GapReason::LimitExceeded,
            ProxyLoss::CutUnsettled => GapReason::ShutdownDeadline,
        }
    }
}

/// A byte stream a relay reads and writes: a TCP socket, a TLS stream, or a test duplex.
pub trait AsyncIo: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> AsyncIo for T {}

pub type BoxIo = Box<dyn AsyncIo>;

/// The harness-facing and provider-facing halves of one relayed connection.
pub struct RelayPair {
    pub client: BoxIo,
    pub upstream: BoxIo,
}

/// One captured path of a host profile: matched exactly, or as an explicit prefix, against
/// the request path without its query or fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturePath {
    pub path: String,
    pub prefix: bool,
    pub protocol_hint: Option<String>,
}

/// The matched host profile's capture paths. An empty policy captures no bodies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapturePolicy {
    pub paths: Vec<CapturePath>,
}

/// What a relay needs about the physical connection it serves.
pub struct RelayContext {
    pub connection: String,
    pub host: String,
    pub emitter: Arc<CaptureEmitter>,
    pub policy: CapturePolicy,
}

/// An accepted `101` upgrade handed from the HTTP relay to the WebSocket relay, with every
/// byte either side had already read. `capture` is the matched request's sanitized
/// metadata; `None` means relay-only.
pub struct Upgrade {
    pub exchange: u64,
    pub pair: RelayPair,
    pub context: RelayContext,
    pub client_prefix: Vec<u8>,
    pub upstream_prefix: Vec<u8>,
    pub extensions: Option<String>,
    pub capture: Option<CaptureMeta>,
}

/// How an HTTP relay ended.
#[derive(Debug)]
// The variant carries `Upgrade` unboxed on purpose: boxing it would change the shared interface.
#[allow(clippy::large_enum_variant)]
pub enum HttpOutcome {
    Closed,
    /// Both directions ended, but at least one stopped parsing and relayed the rest of the
    /// connection as bytes: nothing after that point was captured.
    Opaque,
    WebSocket(Upgrade),
}

impl std::fmt::Debug for RelayPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayPair").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for RelayContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayContext")
            .field("connection", &self.connection)
            .field("host", &self.host)
            .field("emitter", &self.emitter)
            .field("policy", &self.policy)
            .finish()
    }
}

impl std::fmt::Debug for Upgrade {
    /// Lengths only: the prefixes are relayed bytes, never diagnostic text.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Upgrade")
            .field("exchange", &self.exchange)
            .field("context", &self.context)
            .field("client_prefix_len", &self.client_prefix.len())
            .field("upstream_prefix_len", &self.upstream_prefix.len())
            .field("extensions", &self.extensions)
            .field("capture", &self.capture.is_some())
            .finish_non_exhaustive()
    }
}

/// The state of the emitter's current producer epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Epoch {
    /// Admitted and accepting frames.
    Live,
    /// Ended by `QueueFull`/`IngestLost`: recoverable with a fresh producer at the next
    /// safe boundary through [`CaptureEmitter::try_recover`].
    Failed(GapReason),
    /// Ended for good: a terminal sink answer, an exhausted protocol limit, or a failed close.
    Disabled(GapReason),
    /// Closed cleanly by [`CaptureEmitter::try_close`].
    Closed,
}

/// A physical connection a relay reported open, kept so a recovered epoch can re-observe it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TrackedConnection {
    connection: String,
    alpn: Option<String>,
    upgraded: bool,
}

#[derive(Debug)]
struct EmitterState {
    producer: String,
    epoch: Epoch,
    /// The sequence number the next admitted frame takes. Starts at zero per producer.
    next_seq: u64,
    /// The last sequence number the sink admitted; `None` before any frame.
    last_seq: Option<u64>,
    /// The next payload stream ID. Starts at one per producer; zero is for controls.
    next_stream: u64,
    /// Open payload streams of the current producer (at most `MAX_OPEN_STREAMS`).
    open: BTreeSet<u64>,
    /// Surviving physical connections in the order they opened (at most
    /// `MAX_OPEN_CONNECTIONS`). Kept across epochs: connection identity outlives a producer.
    connections: Vec<TrackedConnection>,
    /// A prospective producer ID kept across `Pending` polls of one admission attempt.
    pending: Option<String>,
    /// Producer IDs that may have reached the receiver: the initial one, every `Ready`,
    /// and every `RetryFresh`. Bounded by `MAX_PRODUCERS`.
    consumed: usize,
    /// Reasons whose [`FrameEvent::Loss`] a live epoch queued. Each reason is sent once per
    /// emitter, across epochs: a later loss of the same reason is already explained by it. At
    /// most one entry per reason, so at most the number of `GapReason` values the recording proxy
    /// uses.
    lost_reasons: Vec<GapReason>,
}

impl EmitterState {
    fn refusal(&self) -> Option<GapReason> {
        match self.epoch {
            Epoch::Live => None,
            Epoch::Failed(reason) | Epoch::Disabled(reason) => Some(reason),
            // A closed epoch has drained; anything after it is past the shutdown boundary.
            Epoch::Closed => Some(GapReason::ShutdownDeadline),
        }
    }

    /// Record a refusal from the sink: pressure and ingest loss end this epoch
    /// recoverably; every other reason is terminal.
    fn latch(&mut self, reason: GapReason) {
        self.epoch = match reason {
            GapReason::QueueFull | GapReason::IngestLost => Epoch::Failed(reason),
            _ => Epoch::Disabled(reason),
        };
    }

    /// The next frame of the current producer. Its `clock_id` is the producer ID, whatever
    /// clock the caller named.
    fn frame(&self, stream: u64, observed: CaptureTime, event: FrameEvent) -> CaptureFrame {
        CaptureFrame {
            stream: StreamKey {
                producer: self.producer.clone(),
                stream,
            },
            seq: self.next_seq,
            observed: CaptureTime {
                clock_id: self.producer.clone(),
                ..observed
            },
            event,
        }
    }

    fn admitted(&mut self) {
        self.last_seq = Some(self.next_seq);
        self.next_seq += 1;
    }

    fn fresh_producer() -> String {
        format!("{:032x}", rand::random::<u128>())
    }
}

/// Allocates producer-wide stream and sequence IDs and submits frames in allocation order.
///
/// Constructed only after the initial producer ID was admitted `Ready` by the sink;
/// construction performs no I/O. `open` is the only source of `FrameEvent::Open`, `emit` of
/// payload `Bytes`/`End`/`Gap`, `connection_event` of `Connection` controls and
/// `try_recover` of `Recovery` and `loss` of `Loss`. Refusals never block or fail the relay that
/// asked.
pub struct CaptureEmitter {
    sink: Arc<dyn CaptureSink>,
    state: Mutex<EmitterState>,
    /// See [`CaptureEmitter::local_refusals`]; outside the lock so reading it never waits.
    local_refusals: AtomicU64,
    /// See [`CaptureEmitter::opaque_fallbacks`]; outside the lock for the same reason.
    opaque_fallbacks: AtomicU64,
    /// See [`CaptureEmitter::record_model_request`]; shared, so the run's heartbeat can read it
    /// without asking the recording proxy.
    model_requests: Arc<AtomicU64>,
    /// See [`CaptureEmitter::recorded_requests`]; shared like `model_requests`.
    recorded_requests: Arc<AtomicU64>,
    /// See [`CaptureEmitter::generation_requests`]; shared like `recorded_requests`.
    generation_requests: Arc<AtomicU64>,
}

impl std::fmt::Debug for CaptureEmitter {
    /// Counters only, and never a blocking lock: formatting must not wait on a relay.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut out = f.debug_struct("CaptureEmitter");
        match self.state.try_lock() {
            Ok(state) => out
                .field("producer", &state.producer)
                .field("epoch", &state.epoch)
                .field("next_seq", &state.next_seq)
                .field("open_streams", &state.open.len())
                .field("connections", &state.connections.len()),
            Err(_) => out.field("state", &"<busy>"),
        };
        out.finish_non_exhaustive()
    }
}

impl CaptureEmitter {
    /// `producer` must be the fresh 128-bit hex ID the sink already admitted `Ready`. An ID
    /// that is not one leaves the emitter disabled (`LimitExceeded`) rather than submitting
    /// frames the receiver would refuse.
    pub fn new(producer: String, sink: Arc<dyn CaptureSink>) -> Self {
        let epoch = if is_capture_id(&producer) {
            Epoch::Live
        } else {
            Epoch::Disabled(GapReason::LimitExceeded)
        };
        CaptureEmitter {
            sink,
            state: Mutex::new(EmitterState {
                producer,
                epoch,
                next_seq: 0,
                last_seq: None,
                next_stream: 1,
                open: BTreeSet::new(),
                connections: Vec::new(),
                pending: None,
                consumed: 1,
                lost_reasons: Vec::new(),
            }),
            local_refusals: AtomicU64::new(0),
            opaque_fallbacks: AtomicU64::new(0),
            model_requests: Arc::new(AtomicU64::new(0)),
            recorded_requests: Arc::new(AtomicU64::new(0)),
            generation_requests: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Refusals this emitter made on its own, which its sink never saw: an exhausted protocol limit
    /// (open streams, tracked connections, producers), which ends capture for good, and a
    /// stream open or recovery frame outside those limits. An answer that repeats an ended epoch's
    /// refusal is not counted again. Reading it never waits on a relay.
    pub(crate) fn local_refusals(&self) -> u64 {
        self.local_refusals.load(Ordering::SeqCst)
    }

    /// Connections whose relay stopped parsing and went on relaying bytes it does not
    /// capture, each counted once by its relay when it falls back, however it then ends.
    pub(crate) fn opaque_fallbacks(&self) -> u64 {
        self.opaque_fallbacks.load(Ordering::SeqCst)
    }

    /// Requests the harness made of a captured host: each request relayed on an intercepted
    /// connection, and each client-to-server WebSocket message on one. Counted when the relay
    /// sees it, whether or not it is captured. Reading it never waits on a relay.
    pub fn model_requests(&self) -> u64 {
        self.model_requests.load(Ordering::SeqCst)
    }

    /// The counter itself, for a reader that outlives the recording proxy's handle.
    pub fn model_requests_counter(&self) -> Arc<AtomicU64> {
        self.model_requests.clone()
    }

    /// Count one request of a captured host.
    pub(crate) fn record_model_request(&self) {
        self.model_requests.fetch_add(1, Ordering::SeqCst);
    }

    /// Requests the recording proxy selected for capture: each request to one of the connector's
    /// evidence paths (its model endpoints), and each client-to-server message on a WebSocket
    /// opened on one. Telemetry, login and other paths of a captured host are not counted.
    pub fn recorded_requests(&self) -> u64 {
        self.recorded_requests.load(Ordering::SeqCst)
    }

    /// The counter itself, for a reader that outlives the recording proxy's handle.
    pub fn recorded_requests_counter(&self) -> Arc<AtomicU64> {
        self.recorded_requests.clone()
    }

    /// Requests the recording proxy selected on a path that carries a protocol hint: the
    /// connector's model-turn endpoints (`/chat/completions`, `/messages`, `/responses`, a
    /// provider's reviewed paths). A request under a model host's base path that carries no hint
    /// (a model list, a health probe) is recorded ([`CaptureEmitter::recorded_requests`]) but
    /// is not counted here, so this is what "the agent made a model call" means.
    pub fn generation_requests(&self) -> u64 {
        self.generation_requests.load(Ordering::SeqCst)
    }

    /// The counter itself, for a reader that outlives the recording proxy's handle.
    pub fn generation_requests_counter(&self) -> Arc<AtomicU64> {
        self.generation_requests.clone()
    }

    /// Count one request selected on a path with a protocol hint.
    pub(crate) fn record_generation_request(&self) {
        self.generation_requests.fetch_add(1, Ordering::SeqCst);
    }

    /// Count one request selected for capture.
    pub(crate) fn record_recorded_request(&self) {
        self.recorded_requests.fetch_add(1, Ordering::SeqCst);
    }

    /// Counts the fallback and records it in the stream ([`ProxyLoss::OpaqueFallback`]).
    pub(crate) fn record_opaque_fallback(&self, observed: CaptureTime) {
        self.opaque_fallbacks.fetch_add(1, Ordering::SeqCst);
        let _ = self.proxy_loss(observed, ProxyLoss::OpaqueFallback);
    }

    /// Record one of the recording proxy's own losses: [`CaptureEmitter::loss`] with the reason
    /// that loss is written as.
    pub(crate) fn proxy_loss(&self, observed: CaptureTime, loss: ProxyLoss) -> SubmitResult {
        self.loss(observed, loss.gap_reason())
    }

    fn refused_locally(&self, reason: GapReason) -> GapReason {
        self.local_refusals.fetch_add(1, Ordering::SeqCst);
        reason
    }

    /// The state lock. Counters move only after the sink has answered, so even a sink that
    /// panicked mid-call leaves counters describing exactly the admitted frames; a poisoned
    /// lock is therefore taken over rather than turned into a relay-path panic.
    fn lock(&self) -> MutexGuard<'_, EmitterState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Open a payload stream: allocate the next stream ID and submit its `Open`.
    ///
    /// Metadata over a protocol limit is refused with `LimitExceeded` before anything is
    /// serialized or submitted, and leaves the epoch live. Exhausting `MAX_OPEN_STREAMS`
    /// disables this producer's capture for good (`LimitExceeded`); the relay continues.
    pub fn open(
        &self,
        source: CaptureSource,
        metadata: CaptureMeta,
        observed: CaptureTime,
    ) -> Result<StreamKey, GapReason> {
        let mut state = self.lock();
        if let Some(reason) = state.refusal() {
            return Err(reason);
        }
        if state.open.len() >= MAX_OPEN_STREAMS {
            state.epoch = Epoch::Disabled(GapReason::LimitExceeded);
            return Err(self.refused_locally(GapReason::LimitExceeded));
        }
        let stream = state.next_stream;
        let frame = state.frame(
            stream,
            observed.clone(),
            FrameEvent::Open { source, metadata },
        );
        if frame.validate().is_err() {
            // The epoch stays live and the relay goes on uncaptured, so the loss is chained
            // here, under the same lock, or the epoch could still close cleanly without it.
            let _ = self.loss_locked(&mut state, observed, GapReason::LimitExceeded);
            return Err(self.refused_locally(GapReason::LimitExceeded));
        }
        let key = frame.stream.clone();
        match self.sink.try_submit(frame) {
            SubmitResult::Queued => {
                state.admitted();
                state.next_stream += 1;
                state.open.insert(stream);
                Ok(key)
            }
            SubmitResult::Disabled(reason) => {
                state.latch(reason);
                Err(reason)
            }
        }
    }

    /// Submit one payload event (`Bytes`, `End` or `Gap`) on an open stream of the current
    /// producer. `End` closes the stream.
    ///
    /// `Bytes` longer than `MAX_SEGMENT_BYTES` are split, under the same lock, into
    /// consecutive frames of at most `MAX_SEGMENT_BYTES` with contiguous offsets, so a relay
    /// never pre-splits a decoded read. If the sink refuses part-way, the frames before the
    /// refusal stay queued and the epoch latches as for any refusal.
    ///
    /// A key from a rotated or closed epoch, or one that is not open, is refused with
    /// `ProducerLost` (its capture context is gone) and submits nothing. Any other local refusal —
    /// an event kind that is not payload, an `observed` or offset outside the protocol limits — is
    /// `LimitExceeded`, submits nothing, and leaves both the epoch live and **the stream open**:
    /// the caller must still end that stream with `End` (for example `End { reason: Interrupted
    /// }`), or record the loss with `Gap` and continue, or the producer can never close cleanly and
    /// the stream keeps its `MAX_OPEN_STREAMS` slot.
    pub fn emit(
        &self,
        stream: &StreamKey,
        observed: CaptureTime,
        event: FrameEvent,
    ) -> SubmitResult {
        let mut state = self.lock();
        if let Some(reason) = state.refusal() {
            return SubmitResult::Disabled(reason);
        }
        if stream.producer != state.producer || !state.open.contains(&stream.stream) {
            return SubmitResult::Disabled(GapReason::ProducerLost);
        }
        let ends = match &event {
            FrameEvent::End { .. } => true,
            FrameEvent::Bytes { .. } | FrameEvent::Gap { .. } => false,
            FrameEvent::Open { .. }
            | FrameEvent::Connection { .. }
            | FrameEvent::Recovery { .. }
            | FrameEvent::Loss { .. } => {
                return SubmitResult::Disabled(GapReason::LimitExceeded);
            }
        };
        let events = match event {
            FrameEvent::Bytes { offset, data } if data.len() > MAX_SEGMENT_BYTES => {
                if offset.checked_add(data.len() as u64).is_none() {
                    return SubmitResult::Disabled(GapReason::LimitExceeded);
                }
                let mut next = offset;
                data.chunks(MAX_SEGMENT_BYTES)
                    .map(|chunk| {
                        let event = FrameEvent::Bytes {
                            offset: next,
                            data: chunk.to_vec(),
                        };
                        next += chunk.len() as u64;
                        event
                    })
                    .collect()
            }
            event => vec![event],
        };
        // Every frame is checked before the first is submitted, so a local refusal submits
        // nothing at all.
        let mut frames = Vec::with_capacity(events.len());
        for (i, event) in events.into_iter().enumerate() {
            let mut frame = state.frame(stream.stream, observed.clone(), event);
            frame.seq += i as u64;
            if frame.validate().is_err() {
                return SubmitResult::Disabled(GapReason::LimitExceeded);
            }
            frames.push(frame);
        }
        for frame in frames {
            if let SubmitResult::Disabled(reason) = self.sink.try_submit(frame) {
                state.latch(reason);
                return SubmitResult::Disabled(reason);
            }
            state.admitted();
        }
        if ends {
            state.open.remove(&stream.stream);
        }
        SubmitResult::Queued
    }

    /// Record a physical connection event as a stream-zero control.
    ///
    /// The emitter tracks every surviving connection whatever the epoch's state, so a
    /// recovered producer can re-observe it; tracking more than `MAX_OPEN_CONNECTIONS`
    /// disables capture for good (`LimitExceeded`) instead of growing. Upgrading an untracked
    /// connection is refused with `ProducerLost`.
    pub fn connection_event(
        &self,
        connection: &str,
        observed: CaptureTime,
        event: ConnectionEvent,
    ) -> SubmitResult {
        let mut state = self.lock();
        // Checked before anything is tracked: an ID or ALPN outside the protocol limits is
        // never remembered.
        let frame = state.frame(
            0,
            observed,
            FrameEvent::Connection {
                connection: connection.to_string(),
                event: event.clone(),
            },
        );
        if frame.validate().is_err() {
            return SubmitResult::Disabled(GapReason::LimitExceeded);
        }
        let position = state
            .connections
            .iter()
            .position(|tracked| tracked.connection == connection);
        match (&event, position) {
            (ConnectionEvent::Open { alpn, .. }, Some(at)) => {
                state.connections[at].alpn = alpn.clone();
                state.connections[at].upgraded = false;
            }
            (ConnectionEvent::Open { alpn, .. }, None) => {
                if state.connections.len() >= MAX_OPEN_CONNECTIONS {
                    state.epoch = Epoch::Disabled(GapReason::LimitExceeded);
                    return SubmitResult::Disabled(self.refused_locally(GapReason::LimitExceeded));
                }
                state.connections.push(TrackedConnection {
                    connection: connection.to_string(),
                    alpn: alpn.clone(),
                    upgraded: false,
                });
            }
            (ConnectionEvent::UpgradedWebSocket, Some(at)) => state.connections[at].upgraded = true,
            (ConnectionEvent::UpgradedWebSocket, None) => {
                return SubmitResult::Disabled(GapReason::ProducerLost);
            }
            (ConnectionEvent::Close { .. }, Some(at)) => {
                state.connections.remove(at);
            }
            (ConnectionEvent::Close { .. }, None) => {}
        }
        if let Some(reason) = state.refusal() {
            return SubmitResult::Disabled(reason);
        }
        match self.sink.try_submit(frame) {
            SubmitResult::Queued => {
                state.admitted();
                SubmitResult::Queued
            }
            SubmitResult::Disabled(reason) => {
                state.latch(reason);
                SubmitResult::Disabled(reason)
            }
        }
    }

    /// Record that this proxy relayed something it does not capture, as a stream-zero
    /// [`FrameEvent::Loss`] of the current epoch, under the ordering lock. It never blocks and
    /// never fails the relay that reports it.
    ///
    /// - A live epoch whose `reason` was not yet queued submits one frame. If the frame fails
    ///   the protocol's own validation the epoch is disabled for good (`LimitExceeded`, a local
    ///   refusal), so it can never close cleanly. Otherwise `Queued` takes its sequence and
    ///   remembers the reason, and a sink refusal latches the epoch as every refusal does.
    /// - A live epoch that already queued `reason` answers `Queued` and submits nothing.
    /// - A failed or disabled epoch answers `Disabled` and submits nothing: it is followed by
    ///   a `Recovery`, or it never closes.
    /// - A closed epoch answers `Disabled(ShutdownDeadline)` and counts a local refusal. The
    ///   proxy closes only after every connection task has ended, so this is unreachable there.
    ///
    /// Each reason is sent once per emitter, and the memory of it is the sink's volatile
    /// `Queued` answer. That is verdict-safe, not complete, for a later epoch: a queued `Loss`
    /// that a latch later discards is not re-sent, but every later epoch exists only behind a
    /// `Recovery`, which the verifier already reads as incomplete, and the discard also drops the
    /// queued `Close`. Per-emitter dedup is also what bounds the closure records this adds.
    pub fn loss(&self, observed: CaptureTime, reason: GapReason) -> SubmitResult {
        let mut state = self.lock();
        self.loss_locked(&mut state, observed, reason)
    }

    fn loss_locked(
        &self,
        state: &mut EmitterState,
        observed: CaptureTime,
        reason: GapReason,
    ) -> SubmitResult {
        match state.epoch {
            Epoch::Live => {}
            Epoch::Failed(refused) | Epoch::Disabled(refused) => {
                return SubmitResult::Disabled(refused);
            }
            Epoch::Closed => {
                return SubmitResult::Disabled(self.refused_locally(GapReason::ShutdownDeadline));
            }
        }
        if state.lost_reasons.contains(&reason) {
            return SubmitResult::Queued;
        }
        let frame = state.frame(0, observed, FrameEvent::Loss { reason });
        if frame.validate().is_err() {
            state.epoch = Epoch::Disabled(GapReason::LimitExceeded);
            return SubmitResult::Disabled(self.refused_locally(GapReason::LimitExceeded));
        }
        match self.sink.try_submit(frame) {
            SubmitResult::Queued => {
                state.admitted();
                state.lost_reasons.push(reason);
                SubmitResult::Queued
            }
            SubmitResult::Disabled(refused) => {
                state.latch(refused);
                SubmitResult::Disabled(refused)
            }
        }
    }

    /// At a new HTTP request or WebSocket message boundary, restore capture after a
    /// recoverable epoch failure.
    ///
    /// A live epoch answers `Queued` and changes nothing, so concurrent relays may all call
    /// it. A failed epoch drives one non-blocking admission poll for a fresh producer: a
    /// `Pending` ID is kept for the next poll, a `RetryFresh` ID is retired and never
    /// proposed again, and at most `MAX_PRODUCERS` IDs are ever consumed. On `Ready` the
    /// emitter rotates to that producer, restarts its sequence and stream counters, submits
    /// `Recovery` first and then re-observes every surviving connection (`Open` with
    /// `already_open`, plus `UpgradedWebSocket` where it applies) before any new payload.
    /// Old stream keys stay invalid; nothing is replayed.
    pub fn try_recover(&self, observed: CaptureTime) -> SubmitResult {
        let mut state = self.lock();
        let reason = match state.epoch {
            Epoch::Live => return SubmitResult::Queued,
            Epoch::Failed(reason) => reason,
            Epoch::Disabled(terminal) => return SubmitResult::Disabled(terminal),
            Epoch::Closed => return SubmitResult::Disabled(GapReason::ShutdownDeadline),
        };
        let candidate = match state.pending.clone() {
            Some(pending) => pending,
            None => {
                if state.consumed >= MAX_PRODUCERS {
                    state.epoch = Epoch::Disabled(GapReason::LimitExceeded);
                    return SubmitResult::Disabled(self.refused_locally(GapReason::LimitExceeded));
                }
                let fresh = EmitterState::fresh_producer();
                state.pending = Some(fresh.clone());
                fresh
            }
        };
        match self.sink.try_begin_epoch(&candidate) {
            EpochAdmission::Pending => return SubmitResult::Disabled(reason),
            EpochAdmission::RetryFresh => {
                state.pending = None;
                state.consumed += 1;
                return SubmitResult::Disabled(reason);
            }
            EpochAdmission::Disabled(terminal) => {
                state.pending = None;
                state.epoch = Epoch::Disabled(terminal);
                return SubmitResult::Disabled(terminal);
            }
            EpochAdmission::Ready => {}
        }
        state.pending = None;
        state.consumed += 1;
        let previous_producer = std::mem::replace(&mut state.producer, candidate);
        state.epoch = Epoch::Live;
        state.next_seq = 0;
        state.last_seq = None;
        state.next_stream = 1;
        state.open.clear();

        let mut controls = vec![FrameEvent::Recovery {
            previous_producer,
            reason,
        }];
        for tracked in &state.connections {
            controls.push(FrameEvent::Connection {
                connection: tracked.connection.clone(),
                event: ConnectionEvent::Open {
                    alpn: tracked.alpn.clone(),
                    already_open: true,
                },
            });
            if tracked.upgraded {
                controls.push(FrameEvent::Connection {
                    connection: tracked.connection.clone(),
                    event: ConnectionEvent::UpgradedWebSocket,
                });
            }
        }
        for (index, event) in controls.into_iter().enumerate() {
            let frame = state.frame(0, observed.clone(), event);
            if frame.validate().is_err() {
                state.epoch = Epoch::Disabled(GapReason::LimitExceeded);
                return SubmitResult::Disabled(self.refused_locally(GapReason::LimitExceeded));
            }
            match self.sink.try_submit(frame) {
                SubmitResult::Queued => state.admitted(),
                SubmitResult::Disabled(refused) => {
                    if index == 0 {
                        // A refused Recovery frame itself is always
                        // terminal, whatever the reason. The candidate producer it named
                        // was never persisted, so it must never be proposed or named again;
                        // `latch` (recoverable for QueueFull/IngestLost) applies only to a
                        // refusal of a later re-observation or payload frame.
                        state.epoch = Epoch::Disabled(refused);
                    } else {
                        state.latch(refused);
                    }
                    return SubmitResult::Disabled(refused);
                }
            }
        }
        SubmitResult::Queued
    }

    /// Close the current producer epoch: with no open payload stream, ask the sink to queue
    /// `Close` carrying the last admitted sequence (`None` for zero frames).
    ///
    /// Open streams refuse the close with `ShutdownDeadline` and leave the epoch live, so a
    /// caller can end them and retry within its own deadline; nothing is sent, and a
    /// producer that is never closed leaves unknown tail loss at the receiver rather than an
    /// invented clean drain. A failed or disabled epoch cannot close cleanly. A retry after a
    /// queued close answers `Queued` again without a second close. The `Close` message
    /// carries no time of its own: the receiver stamps `ProducerClosed` with its own clock.
    pub fn try_close(&self, _observed: CaptureTime) -> SubmitResult {
        let mut state = self.lock();
        match state.epoch {
            Epoch::Closed => return SubmitResult::Queued,
            Epoch::Failed(reason) | Epoch::Disabled(reason) => {
                return SubmitResult::Disabled(reason);
            }
            Epoch::Live => {}
        }
        if !state.open.is_empty() {
            return SubmitResult::Disabled(GapReason::ShutdownDeadline);
        }
        match self.sink.try_close_epoch(&state.producer, state.last_seq) {
            SubmitResult::Queued => {
                state.epoch = Epoch::Closed;
                SubmitResult::Queued
            }
            SubmitResult::Disabled(reason) => {
                state.epoch = Epoch::Disabled(reason);
                SubmitResult::Disabled(reason)
            }
        }
    }
}
