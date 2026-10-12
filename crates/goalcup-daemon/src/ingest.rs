//! The authenticated, bounded capture ingest receiver and ordered producer-close
//! validation.
//!
//! [`IngestListener`] accepts loopback TCP connections from capture producers (the sender
//! inside the proxy), authenticates each against a per-entry, launch-scoped [`IngestSecret`],
//! checks every frame's framing integrity, and hands each admitted [`CaptureFrame`] to the
//! entry's supplied [`CaptureSink`] (the entry owner). It never hashes, redacts or persists a
//! byte and never interprets one (D3): what the sink does with a frame is the sink's.
//!
//! # Wire
//!
//! The shared golden `crates/goalcup-protocol/tests/fixtures/m2-ingest/golden-stream.{json,hex}`
//! fixes the bytes: every item is a big-endian `u32` body length followed by compact JSON.
//! The first item is the handshake `{"entry_id","producer","token"}` of at most
//! [`MAX_HANDSHAKE_BYTES`], decoded here by a private struct that rejects unknown fields and
//! never prints its token. Every later item is an [`IngestMessage`], decoded by the
//! protocol's [`decode_ingest_message`] after [`checked_frame_len`] has refused an over-limit
//! prefix, so no body byte of an over-limit item is ever read or allocated. The receiver
//! never writes a byte: there is no ACK, no rejection frame and no replay. A refusal is a
//! closed socket, and the sender cannot tell which receiver policy caused it.
//!
//! # One producer epoch per connection
//!
//! An authenticated handshake consumes its producer ID for the entry, frames or not: the
//! entry ledger holds at most [`MAX_PRODUCERS`] IDs, including any the caller declares were
//! consumed before this authorization ([`EntryGrant::prior_producers`]). The same ID never
//! connects twice. After the sink admits the epoch (`try_begin_epoch` returns `Ready`), each
//! frame is checked in arrival order before it is offered to the sink:
//!
//! - it names the handshake's producer, and `seq` is exactly the next sequence of that
//!   producer, shared by every stream and control, starting at zero;
//! - `Recovery` only at sequence zero, naming a producer of this entry that is neither closed
//!   nor already superseded. A fresh producer is not required to begin with one. The
//!   Recovery supersedes the producer it names. If that producer had no admitted frame (its
//!   epoch was lost in flight), it also supersedes every other *wire* producer of the entry
//!   that is neither closed nor superseded: one that admitted a Connection frame or
//!   began with a Recovery. Native and goal producers are never wire and never superseded;
//! - `Open` takes a stream ID above the producer's high-water mark while the entry has fewer
//!   than [`MAX_OPEN_STREAMS`] open streams; `Bytes`/`End`/`Gap` need an open stream;
//! - source offsets: a stream's first `Bytes` starts at zero, a later one exactly
//!   where the last ended, and only the first `Bytes` right after a `Gap` may start further
//!   on; every sum is checked.
//!
//! Only open-stream state, one high-water stream ID and a wire flag per producer are kept: no
//! set of completed streams. A lost producer's open streams stay counted against the entry
//! until an admitted `Recovery` supersedes it, by name or as an orphaned wire producer
//! as the integrity reader counts them.
//!
//! # Outcomes: the out-of-band latch
//!
//! Every producer the sink admitted ends in exactly one [`ProducerOutcome`], latched once in
//! the entry's own bounded list (at most one per admitted producer), independent of the sink's
//! queue, so a loss is reported even while that queue is full. [`IngestEntry::take_outcomes`]
//! delivers each outcome once.
//!
//! - [`ProducerOutcome::Closed`]: a `Close` whose `final_seq` is the last admitted sequence
//!   (`None` for zero frames), with no open payload stream, followed by end of stream. Only
//!   then is the sink's `try_close_epoch` called. The entry owner turns this outcome, with its
//!   receiver-observed [`CaptureTime`], into `CaptureRecord::ProducerClosed`; a client cannot
//!   submit that record. It asserts a clean drain at ingress, not durability or an ACK.
//! - [`ProducerOutcome::Lost`]: unknown tail loss. End of stream without a validated `Close`,
//!   a partial item, a framing or sequence violation, a sink refusal, a `Close` with the wrong
//!   `final_seq` or open streams, bytes after `Close`, supersession by a `Recovery`, or a
//!   cancelled epoch. The socket is closed at once and nothing more from it is offered.
//!
//! **Ordering.** Every sink call for an entry is made under that entry's lock, and an outcome
//! is latched under the same lock after the producer's last sink call. `final_seq` and
//! [`ProducerLoss::admitted_through`] name the last frame the sink queued for that producer:
//! the owner applies the outcome after it has drained that producer's frames through that
//! sequence. A `Recovery` frame is queued before the producer it names is latched as
//! superseded, inside the same critical section.
//!
//! **Lock order.** The sink is called while the entry lock is held, so a
//! [`CaptureSink`] implementation must not call back into [`IngestEntry`], and the owner must
//! not call [`IngestEntry`] methods while holding a lock its sink methods take. The same
//! holds for the [`IngestClock`] in [`IngestConfig`]: it is called under the entry lock to
//! stamp every latched outcome, so it must not call into [`IngestEntry`] or take a lock the
//! owner may hold while calling [`IngestEntry`] methods. The sink contract (synchronous,
//! bounded, never awaiting disk or network), which the clock must also meet, is what makes
//! holding the entry lock across these calls safe.
//!
//! # Resources
//!
//! - One accept task; per connection, one task in a [`JoinSet`] owned by the accept task.
//!   Finished tasks are reaped as it runs; shutdown joins or aborts every one.
//! - At most [`IngestConfig::max_pending_handshakes`] unauthenticated sockets. Another is
//!   closed on accept. Each has [`IngestConfig::setup_timeout`] from accept to an admitted
//!   epoch, handshake read and sink admission polling (`Pending`) included.
//! - Per connection, one reusable body buffer, grown only by bytes actually received, at
//!   most [`MAX_INGEST_FRAME_BYTES`] (the handshake's at most [`MAX_HANDSHAKE_BYTES`]). The
//!   receiver keeps no frame queue of its own: a frame is decoded, checked and offered to the
//!   sink before the next is read, so the sink's bounded queue and the socket buffer are the
//!   only backlog. The worst case per entry, reachable only by a token holder, is
//!   `MAX_PRODUCERS × MAX_INGEST_FRAME_BYTES` of partly received bodies.
//! - Per entry: at most [`MAX_PRODUCERS`] ledger slots, [`MAX_OPEN_STREAMS`] open-stream
//!   records and one latched outcome per admitted producer.
//!
//! # Authentication installation and removal
//!
//! [`IngestListener::authorize`] installs one entry with its secret and sink and returns the
//! [`IngestEntry`] handle; dropping that handle removes the authorization and cancels the
//! entry's unsettled producers. [`IngestEntry::close_admission`] refuses further handshakes
//! while live producers finish; [`IngestEntry::drain`] also waits for them up to a deadline
//! and then cancels the rest. [`IngestListener::shutdown`] cancels everything at once and
//! joins every task within its deadline.
//!
//! # Credential bootstrap
//!
//! The secret is per launch and per entry, never the daemon-wide control token. The create
//! response returns it with [`IngestListener::local_addr`] and the entry id, and the proxy
//! holds it in memory only: never argv, environment, disk, logs or retained metadata.
//!
//! # Conformance note for senders
//!
//! Send the handshake first, then only `IngestMessage` items, each length-prefixed. Expect
//! no response bytes, ever. A closed socket is the only refusal signal and carries no reason.
//! A clean drain is `Close { final_seq }` naming the last frame sent, with every stream ended,
//! followed by closing the write side within [`IngestConfig::close_grace`]. A producer ID used
//! in any handshake that may have reached the receiver is consumed; a fresh epoch uses a
//! fresh ID and may start with `Recovery` at sequence zero naming the epoch it replaces.

use crate::control::constant_time_eq;
use goalcup_protocol::capture::{
    CaptureFrame, CaptureLimitError, CaptureSink, CaptureTime, EpochAdmission, FrameEvent,
    GapReason, IngestMessage, MAX_HANDSHAKE_BYTES, MAX_INGEST_FRAME_BYTES, MAX_OPEN_STREAMS,
    MAX_PRODUCERS, SubmitResult, checked_frame_len, decode_ingest_message, is_capture_id,
};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::Instant;

/// Default bound on unauthenticated sockets held at once.
pub(crate) const DEFAULT_MAX_PENDING_HANDSHAKES: usize = 16;
/// Default time from accept to an admitted epoch.
pub const DEFAULT_SETUP_TIMEOUT: Duration = Duration::from_secs(5);
/// Default wait, after a valid `Close`, for the end of stream that makes it clean.
pub(crate) const DEFAULT_CLOSE_GRACE: Duration = Duration::from_secs(2);
/// How long a listener dropped without [`IngestListener::shutdown`] lets its tasks settle
/// before aborting them.
pub(crate) const DROP_SHUTDOWN_GRACE: Duration = Duration::from_secs(1);
/// Longest entry ID accepted by [`IngestListener::authorize`] and in a handshake.
pub(crate) const MAX_ENTRY_ID_BYTES: usize = 256;

const ADMISSION_POLL_FIRST: Duration = Duration::from_millis(5);
const ADMISSION_POLL_MAX: Duration = Duration::from_millis(100);
const ACCEPT_ERROR_PAUSE: Duration = Duration::from_millis(50);
const SECRET_MIN_BYTES: usize = 32;
const SECRET_MAX_BYTES: usize = 256;

/// The daemon clock the receiver stamps its own observations with (daemon loss/control
/// observations name the daemon clock, never a producer's).
pub(crate) type IngestClock = Arc<dyn Fn() -> CaptureTime + Send + Sync>;

/// A clock named `clock_id`: RFC 3339 UTC wall time with nanoseconds, and monotonic
/// nanoseconds from the moment this clock was made, sleep included
/// ([`goalcup_protocol::clock`]). The entry owner may supply its own daemon clock instead, so
/// that every daemon observation shares one `clock_id`.
pub fn monotonic_clock(clock_id: impl Into<String>) -> IngestClock {
    let clock_id = clock_id.into();
    let base = goalcup_protocol::clock::ContinuousOrigin::now();
    Arc::new(move || {
        let mono_ns = base.elapsed_ns();
        // A clock before the epoch reads as the epoch, never a panic.
        let since = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let whole = goalcup_protocol::control::format_z(since.as_secs());
        let wall = format!("{}.{:09}Z", &whole[..19], since.subsec_nanos());
        CaptureTime {
            wall,
            mono_ns,
            clock_id: clock_id.clone(),
        }
    })
}

/// Listener configuration. Build with [`IngestConfig::new`], then adjust the public fields.
#[derive(Clone)]
pub struct IngestConfig {
    pub clock: IngestClock,
    /// From accept to an admitted epoch: the handshake read plus sink admission polling.
    pub setup_timeout: Duration,
    /// After a valid `Close`, how long to wait for the end of stream that makes it clean.
    pub close_grace: Duration,
    /// Unauthenticated sockets held at once (at least one).
    pub max_pending_handshakes: usize,
}

impl IngestConfig {
    pub fn new(clock: IngestClock) -> Self {
        Self {
            clock,
            setup_timeout: DEFAULT_SETUP_TIMEOUT,
            close_grace: DEFAULT_CLOSE_GRACE,
            max_pending_handshakes: DEFAULT_MAX_PENDING_HANDSHAKES,
        }
    }
}

impl fmt::Debug for IngestConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IngestConfig")
            .field("setup_timeout", &self.setup_timeout)
            .field("close_grace", &self.close_grace)
            .field("max_pending_handshakes", &self.max_pending_handshakes)
            .finish_non_exhaustive()
    }
}

/// A launch-scoped ingest credential. Held in memory only; `Debug` never prints it, and it
/// has no `Display`, serialization or equality.
#[derive(Clone)]
pub struct IngestSecret(String);

impl IngestSecret {
    /// 32 bytes from the operating system's entropy source, as 64 lowercase hex digits.
    pub fn generate() -> Self {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self(hex::encode(bytes))
    }

    /// A caller-chosen secret: 32 to 256 printable ASCII bytes without `"` or `\`, so its
    /// JSON encoding is itself and the handshake stays within [`MAX_HANDSHAKE_BYTES`].
    pub fn new(value: String) -> Result<Self, IngestError> {
        let valid = (SECRET_MIN_BYTES..=SECRET_MAX_BYTES).contains(&value.len())
            && value
                .bytes()
                .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\');
        if valid {
            Ok(Self(value))
        } else {
            Err(IngestError::InvalidSecret)
        }
    }

    /// The secret itself, for the handshake comparison and for handing it to the producer in
    /// memory. Never into argv, environment, disk, logs or retained metadata.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for IngestSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("IngestSecret(<redacted>)")
    }
}

/// Construction and authorization failures. Messages never carry a secret or any input.
#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("the ingest endpoint must be a loopback address")]
    NotLoopback,
    #[error("ingest listener I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("an entry ID is 1 to {MAX_ENTRY_ID_BYTES} bytes without control characters")]
    InvalidEntryId,
    #[error("an ingest secret is 32 to 256 printable ASCII bytes without quote or backslash")]
    InvalidSecret,
    #[error("this entry is already authorized on the listener")]
    AlreadyAuthorized,
    #[error("{prior} prior producers exceed the {limit}-producer limit")]
    ProducerBudget { prior: usize, limit: usize },
    #[error("the ingest listener is shut down")]
    ShutDown,
}

/// What [`IngestListener::authorize`] installs for one entry.
pub struct EntryGrant {
    pub entry_id: String,
    pub secret: IngestSecret,
    /// The entry owner's sink. It receives every admitted frame, epoch and validated close.
    pub sink: Arc<dyn CaptureSink>,
    /// Producer epochs this entry consumed before this authorization (for example in a chain
    /// being explicitly resumed); they count against [`MAX_PRODUCERS`].
    pub prior_producers: usize,
}

impl fmt::Debug for EntryGrant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EntryGrant")
            .field("entry_id", &self.entry_id)
            .field("prior_producers", &self.prior_producers)
            .finish_non_exhaustive()
    }
}

/// One admitted producer's end state, latched once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProducerOutcome {
    Closed(ValidatedClose),
    Lost(ProducerLoss),
}

/// A validated clean drain: every admitted frame, then `Close { final_seq }`, then end of
/// stream. The entry owner persists `CaptureRecord::ProducerClosed` from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedClose {
    pub producer: String,
    /// When the receiver validated the drain, on the daemon clock.
    pub observed: CaptureTime,
    /// The last admitted frame sequence; `None` for zero frames.
    pub final_seq: Option<u64>,
}

/// Unknown tail loss of one admitted producer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducerLoss {
    pub producer: String,
    /// When the receiver observed the loss, on the daemon clock.
    pub observed: CaptureTime,
    /// The gap reason to record.
    pub reason: GapReason,
    pub cause: LossCause,
    /// The last frame sequence the sink queued for this producer; `None` if none was.
    pub admitted_through: Option<u64>,
}

/// Why an admitted producer's epoch ended without a validated close.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LossCause {
    /// End of stream or a transport error at an item boundary, with no validated `Close`.
    Disconnected,
    /// The stream ended inside a length prefix or a body.
    PartialItem,
    /// A length prefix declared more than [`MAX_INGEST_FRAME_BYTES`]; no body was read.
    OversizedItem,
    /// Not valid JSON for an [`IngestMessage`], or an unknown field or variant.
    Malformed,
    /// A decoded message broke a bound or representation rule (including offset overflow).
    Invalid,
    /// A frame or `Close` named another producer than the handshake's.
    ForeignProducer,
    /// `seq` was not the producer's next sequence.
    Sequence,
    /// `Open` reused or lowered a stream ID, or `Bytes`/`End`/`Gap` named a stream not open.
    StreamState,
    /// Source offsets overlapped, left an unexplained hole, or overflowed.
    Offset,
    /// Opening a stream would exceed [`MAX_OPEN_STREAMS`] for the entry.
    StreamLimit,
    /// `Recovery` after sequence zero, or naming a producer that is not an unsuperseded,
    /// unclosed epoch of this entry.
    Recovery,
    /// The sink refused the frame or the close; [`ProducerLoss::reason`] is its reason.
    SinkRefused,
    /// `Close.final_seq` was not the last admitted sequence.
    CloseMismatch,
    /// `Close` arrived while payload streams were open.
    CloseWithOpenStreams,
    /// Bytes followed a valid `Close`.
    AfterClose,
    /// No end of stream followed a valid `Close` within the close grace.
    CloseWithoutEof,
    /// A fresh epoch's `Recovery` superseded this producer while it was still admitted:
    /// named directly, or as an orphaned wire producer.
    Superseded,
    /// Admission was withdrawn: entry drain or revocation, or listener shutdown.
    Cancelled,
    /// The connection task ended without settling (a panic or a forced abort).
    Aborted,
}

impl LossCause {
    fn reason(self) -> GapReason {
        match self {
            LossCause::OversizedItem | LossCause::StreamLimit => GapReason::LimitExceeded,
            LossCause::Cancelled => GapReason::ShutdownDeadline,
            LossCause::Disconnected
            | LossCause::PartialItem
            | LossCause::CloseWithoutEof
            | LossCause::Aborted => GapReason::IngestLost,
            _ => GapReason::ProducerLost,
        }
    }
}

/// Listener counters. Unauthenticated refusals are counted, never described.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ListenerStats {
    pub connections_accepted: u64,
    /// Closed on accept because the unauthenticated bound was full.
    pub rejected_at_capacity: u64,
    pub handshakes_timed_out: u64,
    /// Oversized, malformed, unknown entry, wrong token, invalid or consumed producer ID,
    /// exhausted producer budget, or closed admission.
    pub handshakes_refused: u64,
    /// Unauthenticated sockets held right now.
    pub pending_handshakes: u64,
}

/// One entry's receiver state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EntrySnapshot {
    pub admission_open: bool,
    /// Prior producers plus every authenticated ID of this authorization.
    pub producers_consumed: usize,
    /// Producers still admitting, live, or awaiting end of stream after `Close`.
    pub producers_unsettled: usize,
    /// Open payload streams, a lost producer's included until a `Recovery` names it.
    pub open_streams: usize,
    pub frames_admitted: u64,
    /// Authenticated IDs the sink never admitted (`RetryFresh`, `Disabled`, setup deadline,
    /// cancellation while admitting).
    pub admissions_refused: u64,
    /// Latched outcomes not yet taken.
    pub outcomes_pending: usize,
}

/// What [`IngestListener::shutdown`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShutdownReport {
    /// Unsettled producers cancelled across all entries (each latched `Cancelled` once if the
    /// sink had admitted it).
    pub producers_cancelled: usize,
    /// Connection tasks still running at the deadline, then aborted and joined.
    pub tasks_aborted: usize,
}

/// What [`IngestEntry::drain`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrainReport {
    /// Producers still unsettled at the deadline, cancelled then.
    pub producers_cancelled: usize,
}

// -------------------------------------------------------------------------------------------
// Handshake: a private codec, byte-exact against the shared golden.
// -------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(deny_unknown_fields)]
struct Handshake {
    entry_id: String,
    producer: String,
    token: String,
}

impl fmt::Debug for Handshake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handshake")
            .field("entry_id", &self.entry_id)
            .field("producer", &self.producer)
            .field("token", &"<redacted>")
            .finish()
    }
}

fn decode_handshake(body: &[u8]) -> Option<Handshake> {
    let handshake: Handshake = serde_json::from_slice(body).ok()?;
    let valid = valid_entry_id(&handshake.entry_id)
        && is_capture_id(&handshake.producer)
        && !handshake.token.is_empty();
    valid.then_some(handshake)
}

fn valid_entry_id(entry_id: &str) -> bool {
    !entry_id.is_empty()
        && entry_id.len() <= MAX_ENTRY_ID_BYTES
        && !entry_id.bytes().any(|b| b.is_ascii_control())
}

// -------------------------------------------------------------------------------------------
// Listener.
// -------------------------------------------------------------------------------------------

/// The ingest listener. Construct inside a Tokio runtime; see the module documentation.
pub struct IngestListener {
    shared: Arc<ListenerShared>,
    local_addr: SocketAddr,
    accept: Option<JoinHandle<ShutdownReport>>,
}

impl fmt::Debug for IngestListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IngestListener")
            .field("local_addr", &self.local_addr)
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

struct ListenerShared {
    config: IngestConfig,
    entries: Mutex<HashMap<String, Arc<EntryShared>>>,
    shutdown: watch::Sender<bool>,
    deadline: Mutex<Option<Instant>>,
    handshakes: Arc<Semaphore>,
    max_pending: usize,
    accepted: AtomicU64,
    rejected_at_capacity: AtomicU64,
    timed_out: AtomicU64,
    refused: AtomicU64,
}

impl IngestListener {
    /// Listen on a loopback TCP address (port zero picks one): the endpoint the sender
    /// connects to. Refuses a non-loopback address. Must be called within a Tokio runtime.
    pub fn bind_loopback(addr: SocketAddr, config: IngestConfig) -> Result<Self, IngestError> {
        if !addr.ip().is_loopback() {
            return Err(IngestError::NotLoopback);
        }
        let listener = std::net::TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        let listener = TcpListener::from_std(listener)?;
        let local_addr = listener.local_addr()?;
        let max_pending = config.max_pending_handshakes.max(1);
        let shared = Arc::new(ListenerShared {
            config,
            entries: Mutex::new(HashMap::new()),
            shutdown: watch::channel(false).0,
            deadline: Mutex::new(None),
            handshakes: Arc::new(Semaphore::new(max_pending)),
            max_pending,
            accepted: AtomicU64::new(0),
            rejected_at_capacity: AtomicU64::new(0),
            timed_out: AtomicU64::new(0),
            refused: AtomicU64::new(0),
        });
        let accept = tokio::spawn(accept_loop(listener, shared.clone()));
        Ok(Self {
            shared,
            local_addr,
            accept: Some(accept),
        })
    }

    /// The bound loopback address, to hand to the producer with its secret.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Install one entry's launch-scoped authorization. The returned handle is the only way
    /// to read its outcomes; dropping it removes the authorization.
    pub fn authorize(&self, grant: EntryGrant) -> Result<IngestEntry, IngestError> {
        if *self.shared.shutdown.borrow() {
            return Err(IngestError::ShutDown);
        }
        if !valid_entry_id(&grant.entry_id) {
            return Err(IngestError::InvalidEntryId);
        }
        if grant.prior_producers > MAX_PRODUCERS {
            return Err(IngestError::ProducerBudget {
                prior: grant.prior_producers,
                limit: MAX_PRODUCERS,
            });
        }
        let entry = Arc::new(EntryShared {
            entry_id: grant.entry_id,
            secret: grant.secret,
            sink: grant.sink,
            clock: self.shared.config.clock.clone(),
            state: Mutex::new(EntryState {
                admission_open: true,
                prior_producers: grant.prior_producers,
                producers: BTreeMap::new(),
                open_streams: 0,
                outcomes: VecDeque::new(),
                frames_admitted: 0,
                admissions_refused: 0,
            }),
            events: watch::channel(0).0,
        });
        let mut entries = lock(&self.shared.entries);
        if entries.contains_key(&entry.entry_id) {
            return Err(IngestError::AlreadyAuthorized);
        }
        entries.insert(entry.entry_id.clone(), entry.clone());
        Ok(IngestEntry {
            shared: entry,
            listener: Arc::downgrade(&self.shared),
        })
    }

    pub fn stats(&self) -> ListenerStats {
        let s = &self.shared;
        ListenerStats {
            connections_accepted: s.accepted.load(Ordering::Relaxed),
            rejected_at_capacity: s.rejected_at_capacity.load(Ordering::Relaxed),
            handshakes_timed_out: s.timed_out.load(Ordering::Relaxed),
            handshakes_refused: s.refused.load(Ordering::Relaxed),
            pending_handshakes: s
                .max_pending
                .saturating_sub(s.handshakes.available_permits())
                as u64,
        }
    }

    /// Stop accepting and cancel every pending handshake and unsettled producer at once
    /// (each admitted one latched `Cancelled` exactly once), then join every connection task,
    /// aborting any still running at `deadline`. No task outlives the returned future.
    pub async fn shutdown(mut self, deadline: Instant) -> ShutdownReport {
        *lock(&self.shared.deadline) = Some(deadline);
        self.shared.shutdown.send_replace(true);
        match self.accept.take() {
            Some(task) => task.await.unwrap_or_default(),
            None => ShutdownReport::default(),
        }
    }
}

impl Drop for IngestListener {
    fn drop(&mut self) {
        if self.accept.take().is_some() {
            let mut deadline = lock(&self.shared.deadline);
            if deadline.is_none() {
                *deadline = Some(Instant::now() + DROP_SHUTDOWN_GRACE);
            }
            drop(deadline);
            self.shared.shutdown.send_replace(true);
        }
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // The guarded state is plain counters and maps; keep serving after a panic elsewhere.
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

async fn accept_loop(listener: TcpListener, shared: Arc<ListenerShared>) -> ShutdownReport {
    let mut shutdown = shared.shutdown.subscribe();
    let mut tasks: JoinSet<()> = JoinSet::new();
    loop {
        let accepted = tokio::select! {
            biased;
            _ = shutdown.wait_for(|stop| *stop) => break,
            Some(_) = tasks.join_next(), if !tasks.is_empty() => continue,
            accepted = listener.accept() => accepted,
        };
        match accepted {
            Ok((stream, _)) => shared.admit_connection(stream, &mut tasks),
            // EMFILE and friends: pause rather than spin.
            Err(_) => tokio::select! {
                biased;
                _ = shutdown.wait_for(|stop| *stop) => break,
                _ = tokio::time::sleep(ACCEPT_ERROR_PAUSE) => {}
            },
        }
    }
    drop(listener);
    let entries: Vec<Arc<EntryShared>> = lock(&shared.entries).values().cloned().collect();
    let producers_cancelled = entries.iter().map(|entry| entry.cancel_unsettled()).sum();
    let deadline = *lock(&shared.deadline);
    let deadline = deadline.unwrap_or_else(|| Instant::now() + DROP_SHUTDOWN_GRACE);
    let joined = tokio::time::timeout_at(deadline, async {
        while tasks.join_next().await.is_some() {}
    })
    .await
    .is_ok();
    let mut tasks_aborted = 0;
    if !joined {
        tasks_aborted = tasks.len();
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
    ShutdownReport {
        producers_cancelled,
        tasks_aborted,
    }
}

impl ListenerShared {
    fn admit_connection(self: &Arc<Self>, stream: TcpStream, tasks: &mut JoinSet<()>) {
        self.accepted.fetch_add(1, Ordering::Relaxed);
        let Ok(permit) = self.handshakes.clone().try_acquire_owned() else {
            self.rejected_at_capacity.fetch_add(1, Ordering::Relaxed);
            return; // `stream` drops here: closed on accept.
        };
        tasks.spawn(serve(stream, self.clone(), permit));
    }

    /// Entry, token, then a fresh producer ID within budget, which is consumed here.
    fn authenticate(
        &self,
        handshake: Handshake,
    ) -> Option<(Arc<EntryShared>, String, watch::Receiver<bool>)> {
        let entry = lock(&self.entries).get(&handshake.entry_id).cloned()?;
        if !constant_time_eq(handshake.token.as_bytes(), entry.secret.expose().as_bytes()) {
            return None;
        }
        let stop = entry.reserve(&handshake.producer)?;
        Some((entry, handshake.producer, stop))
    }
}

/// One connection: the handshake under the unauthenticated bound, then one producer epoch.
async fn serve(mut stream: TcpStream, shared: Arc<ListenerShared>, permit: OwnedSemaphorePermit) {
    let setup_deadline = Instant::now() + shared.config.setup_timeout;
    let mut shutdown = shared.shutdown.subscribe();
    let mut buf = Vec::new();
    let read = tokio::select! {
        biased;
        _ = shutdown.wait_for(|stop| *stop) => return,
        read = tokio::time::timeout_at(
            setup_deadline,
            read_item(&mut stream, &mut buf, MAX_HANDSHAKE_BYTES),
        ) => read,
    };
    let authenticated = match read {
        Err(_elapsed) => {
            shared.timed_out.fetch_add(1, Ordering::Relaxed);
            return;
        }
        Ok(Err(_)) => None,
        Ok(Ok(())) => decode_handshake(&buf).and_then(|h| shared.authenticate(h)),
    };
    let Some((entry, producer, stop)) = authenticated else {
        shared.refused.fetch_add(1, Ordering::Relaxed);
        return;
    };
    drop(permit);
    let close_grace = shared.config.close_grace;
    drop(shared);
    run_producer(
        stream,
        &entry,
        &producer,
        stop,
        setup_deadline,
        close_grace,
        buf,
    )
    .await;
}

/// Read one length-prefixed item into `buf`. The prefix is checked against `limit` before any
/// body byte is read, and the body grows only with bytes actually received.
async fn read_item(
    stream: &mut TcpStream,
    buf: &mut Vec<u8>,
    limit: usize,
) -> Result<(), LossCause> {
    let mut prefix = [0u8; 4];
    let mut filled = 0;
    while filled < prefix.len() {
        match stream.read(&mut prefix[filled..]).await {
            Ok(0) | Err(_) if filled == 0 => return Err(LossCause::Disconnected),
            Ok(0) | Err(_) => return Err(LossCause::PartialItem),
            Ok(n) => filled += n,
        }
    }
    let len = checked_frame_len(prefix, limit).map_err(|_| LossCause::OversizedItem)?;
    buf.clear();
    match (&mut *stream).take(len as u64).read_to_end(buf).await {
        Ok(read) if read == len => Ok(()),
        _ => Err(LossCause::PartialItem),
    }
}

async fn run_producer(
    mut stream: TcpStream,
    entry: &EntryShared,
    producer: &str,
    mut stop: watch::Receiver<bool>,
    setup_deadline: Instant,
    close_grace: Duration,
    mut buf: Vec<u8>,
) {
    let _settle = SettleOnDrop { entry, producer };

    // Sink admission, polled within the setup deadline while it answers Pending.
    let mut pause = ADMISSION_POLL_FIRST;
    loop {
        match entry.begin_epoch(producer) {
            Admission::Ready => break,
            Admission::Gone => return,
            Admission::Pending => {
                let now = Instant::now();
                if now >= setup_deadline {
                    entry.fail(producer, LossCause::Cancelled);
                    return;
                }
                tokio::select! {
                    biased;
                    _ = stop.wait_for(|stop| *stop) => return,
                    _ = tokio::time::sleep_until((now + pause).min(setup_deadline)) => {}
                }
                pause = (pause * 2).min(ADMISSION_POLL_MAX);
            }
        }
    }

    // Frames, one at a time: read, decode, check, offer to the sink.
    loop {
        let read = tokio::select! {
            biased;
            _ = stop.wait_for(|stop| *stop) => return,
            read = read_item(&mut stream, &mut buf, MAX_INGEST_FRAME_BYTES) => read,
        };
        if let Err(cause) = read {
            entry.fail(producer, cause);
            return;
        }
        let message = match decode_ingest_message(&buf) {
            Ok(message) => message,
            Err(CaptureLimitError::Malformed) => {
                entry.fail(producer, LossCause::Malformed);
                return;
            }
            Err(_) => {
                entry.fail(producer, LossCause::Invalid);
                return;
            }
        };
        match message {
            IngestMessage::Frame(frame) => {
                if !entry.admit_frame(producer, frame) {
                    return;
                }
            }
            IngestMessage::Close {
                producer: named,
                final_seq,
            } => {
                if !entry.begin_close(producer, &named, final_seq) {
                    return;
                }
                let mut probe = [0u8; 1];
                let tail = tokio::select! {
                    biased;
                    _ = stop.wait_for(|stop| *stop) => return,
                    tail = tokio::time::timeout(close_grace, stream.read(&mut probe)) => tail,
                };
                match tail {
                    Ok(Ok(0)) => entry.finish_close(producer),
                    Ok(Ok(_)) => entry.fail(producer, LossCause::AfterClose),
                    Ok(Err(_)) => entry.fail(producer, LossCause::Disconnected),
                    Err(_elapsed) => entry.fail(producer, LossCause::CloseWithoutEof),
                }
                return;
            }
        }
    }
}

/// Settles a producer whose task ends without having done so (a panic or an abort).
struct SettleOnDrop<'a> {
    entry: &'a EntryShared,
    producer: &'a str,
}

impl Drop for SettleOnDrop<'_> {
    fn drop(&mut self) {
        self.entry.fail(self.producer, LossCause::Aborted);
    }
}

// -------------------------------------------------------------------------------------------
// Entries.
// -------------------------------------------------------------------------------------------

/// One authorized entry: its outcome latch, drain and revocation. Dropping it removes the
/// authorization and cancels the entry's unsettled producers.
pub struct IngestEntry {
    shared: Arc<EntryShared>,
    listener: Weak<ListenerShared>,
}

impl fmt::Debug for IngestEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IngestEntry")
            .field("entry_id", &self.shared.entry_id)
            .field("snapshot", &self.snapshot())
            .finish_non_exhaustive()
    }
}

impl IngestEntry {
    /// Every latched outcome not yet taken, in latch order. Each is delivered once.
    pub fn take_outcomes(&self) -> Vec<ProducerOutcome> {
        self.shared.lock().outcomes.drain(..).collect()
    }

    /// Resolves once at least one outcome is waiting to be taken.
    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn wait_outcome(&self) {
        let mut events = self.shared.events.subscribe();
        while self.shared.lock().outcomes.is_empty() {
            if events.changed().await.is_err() {
                return;
            }
        }
    }

    pub fn snapshot(&self) -> EntrySnapshot {
        let state = self.shared.lock();
        EntrySnapshot {
            admission_open: state.admission_open,
            producers_consumed: state.prior_producers + state.producers.len(),
            producers_unsettled: state
                .producers
                .values()
                .filter(|p| p.status.unsettled())
                .count(),
            open_streams: state.open_streams,
            frames_admitted: state.frames_admitted,
            admissions_refused: state.admissions_refused,
            outcomes_pending: state.outcomes.len(),
        }
    }

    /// Refuse every further handshake for this entry. Live producers continue.
    pub(crate) fn close_admission(&self) {
        self.shared.lock().admission_open = false;
    }

    /// Close admission, wait until every producer has settled or `deadline` passes, then
    /// cancel the rest (latched `Cancelled`; their own tasks close their sockets).
    pub async fn drain(&self, deadline: Instant) -> DrainReport {
        self.close_admission();
        let mut events = self.shared.events.subscribe();
        loop {
            let unsettled = {
                let state = self.shared.lock();
                state.producers.values().any(|p| p.status.unsettled())
            };
            if !unsettled {
                return DrainReport::default();
            }
            match tokio::time::timeout_at(deadline, events.changed()).await {
                Ok(Ok(())) => {}
                _ => break,
            }
        }
        DrainReport {
            producers_cancelled: self.shared.cancel_unsettled(),
        }
    }
}

impl Drop for IngestEntry {
    fn drop(&mut self) {
        if let Some(listener) = self.listener.upgrade() {
            let mut entries = lock(&listener.entries);
            if entries
                .get(&self.shared.entry_id)
                .is_some_and(|installed| Arc::ptr_eq(installed, &self.shared))
            {
                entries.remove(&self.shared.entry_id);
            }
        }
        self.shared.cancel_unsettled();
    }
}

struct EntryShared {
    entry_id: String,
    secret: IngestSecret,
    sink: Arc<dyn CaptureSink>,
    clock: IngestClock,
    state: Mutex<EntryState>,
    /// Bumped on every settlement, so waiters re-check under the lock.
    events: watch::Sender<u64>,
}

struct EntryState {
    admission_open: bool,
    prior_producers: usize,
    producers: BTreeMap<String, Producer>,
    open_streams: usize,
    outcomes: VecDeque<ProducerOutcome>,
    frames_admitted: u64,
    admissions_refused: u64,
}

struct Producer {
    status: Status,
    next_seq: u64,
    high_stream: u64,
    /// Open payload streams. A lost producer keeps its map, still counted in the entry's
    /// `open_streams`, until a `Recovery` supersedes it.
    open: BTreeMap<u64, Offsets>,
    /// Signals the connection task to stop; taken when the producer settles.
    stop: Option<watch::Sender<bool>>,
    /// Admitted a Connection or Loss frame or began with a Recovery. Native and goal
    /// producers never are, so an orphaned-lineage Recovery never supersedes them.
    wire: bool,
    /// Admitted a frame the chain records under this producer's name: anything but a
    /// stream-zero `Loss`, which chains only as a stream-less gap naming no producer. A
    /// producer without one is "unrecorded" to the integrity reader (CONTRACTS, H-16 ¶4).
    named: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    /// Authenticated; the sink has not answered `Ready` yet.
    Admitting,
    Live,
    /// A valid `Close` naming `final_seq`, awaiting end of stream.
    Closing(Option<u64>),
    Closed,
    Lost,
    /// Authenticated but never admitted by the sink.
    Refused,
    /// A `Recovery` named it.
    Superseded,
}

impl Status {
    fn unsettled(self) -> bool {
        matches!(self, Status::Admitting | Status::Live | Status::Closing(_))
    }

    /// Neither closed nor already superseded.
    fn recoverable(self) -> bool {
        !matches!(self, Status::Closed | Status::Superseded)
    }
}

/// A stream's source-offset position, shared by the receiver and the owner's sink. Offsets are
/// contiguous: the first `Bytes` starts at 0, and a forward jump is allowed only right after a
/// `Gap` on that stream.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Offsets {
    pub(crate) next: Option<u64>,
    pub(crate) after_gap: bool,
}

impl Offsets {
    pub(crate) fn admits(&self, offset: u64) -> bool {
        match (self.next, self.after_gap) {
            (next, true) => next.is_none_or(|next| offset >= next),
            (None, false) => offset == 0,
            (Some(next), false) => offset == next,
        }
    }
}

enum Admission {
    Ready,
    Pending,
    Gone,
}

/// A checked frame's effect, applied only after the sink queued it.
enum Step {
    /// A `Connection`: wire traffic, recorded under the producer's name.
    Control,
    /// A `Loss`: wire traffic, recorded only as a stream-less gap.
    Loss,
    Recovery(String),
    Open(u64),
    Bytes(u64, u64),
    Gap(u64),
    End(u64),
}

impl EntryShared {
    fn lock(&self) -> MutexGuard<'_, EntryState> {
        lock(&self.state)
    }

    fn bump(&self) {
        self.events.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// Consume `producer` for this entry: fresh, within budget, while admission is open.
    fn reserve(&self, producer: &str) -> Option<watch::Receiver<bool>> {
        let mut state = self.lock();
        if !state.admission_open
            || state.producers.contains_key(producer)
            || state.prior_producers + state.producers.len() >= MAX_PRODUCERS
        {
            return None;
        }
        let (stop, stopped) = watch::channel(false);
        state.producers.insert(
            producer.to_string(),
            Producer {
                status: Status::Admitting,
                next_seq: 0,
                high_stream: 0,
                open: BTreeMap::new(),
                stop: Some(stop),
                wire: false,
                named: false,
            },
        );
        Some(stopped)
    }

    fn begin_epoch(&self, id: &str) -> Admission {
        let mut guard = self.lock();
        let state = &mut *guard;
        if state.producers.get(id).map(|p| p.status) != Some(Status::Admitting) {
            return Admission::Gone;
        }
        if !state.admission_open {
            self.lose(state, id, LossCause::Cancelled, GapReason::ShutdownDeadline);
            return Admission::Gone;
        }
        match self.sink.try_begin_epoch(id) {
            EpochAdmission::Ready => {
                if let Some(p) = state.producers.get_mut(id) {
                    p.status = Status::Live;
                }
                Admission::Ready
            }
            EpochAdmission::Pending => Admission::Pending,
            EpochAdmission::RetryFresh | EpochAdmission::Disabled(_) => {
                self.lose(state, id, LossCause::SinkRefused, GapReason::ProducerLost);
                Admission::Gone
            }
        }
    }

    /// Check one frame, offer it to the sink, and apply it once queued. `false` ends the
    /// connection: the producer is settled (by this call or earlier).
    fn admit_frame(&self, id: &str, frame: CaptureFrame) -> bool {
        let mut guard = self.lock();
        let state = &mut *guard;
        if state.producers.get(id).map(|p| p.status) != Some(Status::Live) {
            return false;
        }
        let step = match check(state, id, &frame) {
            Ok(step) => step,
            Err(cause) => {
                self.lose(state, id, cause, cause.reason());
                return false;
            }
        };
        match self.sink.try_submit(frame) {
            SubmitResult::Queued => {
                self.apply(state, id, step);
                true
            }
            SubmitResult::Disabled(reason) => {
                self.lose(state, id, LossCause::SinkRefused, reason);
                false
            }
        }
    }

    fn apply(&self, state: &mut EntryState, id: &str, step: Step) {
        state.frames_admitted += 1;
        let Some(p) = state.producers.get_mut(id) else {
            return;
        };
        p.next_seq += 1; // `check` refused u64::MAX.
        p.named |= !matches!(step, Step::Loss);
        match step {
            // A Connection or a Loss: this producer observes wire traffic.
            Step::Control | Step::Loss => p.wire = true,
            Step::Open(stream) => {
                p.high_stream = stream;
                p.open.insert(stream, Offsets::default());
                state.open_streams += 1;
            }
            Step::Bytes(stream, end) => {
                if let Some(position) = p.open.get_mut(&stream) {
                    position.next = Some(end);
                    position.after_gap = false;
                }
            }
            Step::Gap(stream) => {
                if let Some(position) = p.open.get_mut(&stream) {
                    position.after_gap = true;
                }
            }
            Step::End(stream) => {
                if p.open.remove(&stream).is_some() {
                    state.open_streams -= 1;
                }
            }
            Step::Recovery(previous) => {
                p.wire = true;
                // A named epoch the chain records nothing under its name for (no admitted
                // frame, or only `Loss` frames) was lost in flight or loss-only; the
                // Recovery then also retires the rest of the wire lineage (one proxy launch
                // per entry), releasing what those epochs held. The reader's definition.
                let unrecorded = state.producers.get(&previous).is_some_and(|q| !q.named);
                self.supersede(state, &previous);
                if unrecorded {
                    let orphans: Vec<String> = state
                        .producers
                        .iter()
                        .filter(|(other, q)| {
                            other.as_str() != id && q.wire && q.status.recoverable()
                        })
                        .map(|(other, _)| other.clone())
                        .collect();
                    for orphan in orphans {
                        self.supersede(state, &orphan);
                    }
                }
            }
        }
    }

    /// Retire `id` for a `Recovery`: latch it lost if it was still admitted, release its open
    /// streams from the entry count, and mark it superseded so no later Recovery names it.
    fn supersede(&self, state: &mut EntryState, id: &str) {
        self.lose(state, id, LossCause::Superseded, GapReason::ProducerLost);
        if let Some(old) = state.producers.get_mut(id) {
            state.open_streams -= old.open.len();
            old.open.clear();
            old.status = Status::Superseded;
        }
    }

    /// A `Close` is valid if it names this producer, the last admitted sequence, and no
    /// open stream. It becomes clean only at end of stream ([`Self::finish_close`]).
    fn begin_close(&self, id: &str, named: &str, final_seq: Option<u64>) -> bool {
        let mut guard = self.lock();
        let state = &mut *guard;
        let Some(p) = state.producers.get_mut(id) else {
            return false;
        };
        if p.status != Status::Live {
            return false;
        }
        let refusal = if named != id {
            Some(LossCause::ForeignProducer)
        } else if final_seq != p.next_seq.checked_sub(1) {
            Some(LossCause::CloseMismatch)
        } else if !p.open.is_empty() {
            Some(LossCause::CloseWithOpenStreams)
        } else {
            None
        };
        match refusal {
            Some(cause) => {
                self.lose(state, id, cause, cause.reason());
                false
            }
            None => {
                p.status = Status::Closing(final_seq);
                true
            }
        }
    }

    fn finish_close(&self, id: &str) {
        let mut guard = self.lock();
        let state = &mut *guard;
        let Some(Status::Closing(final_seq)) = state.producers.get(id).map(|p| p.status) else {
            return;
        };
        match self.sink.try_close_epoch(id, final_seq) {
            SubmitResult::Queued => {
                if let Some(p) = state.producers.get_mut(id) {
                    p.status = Status::Closed;
                    p.stop = None;
                }
                state
                    .outcomes
                    .push_back(ProducerOutcome::Closed(ValidatedClose {
                        producer: id.to_string(),
                        observed: (self.clock)(),
                        final_seq,
                    }));
                self.bump();
            }
            SubmitResult::Disabled(reason) => {
                self.lose(state, id, LossCause::SinkRefused, reason);
            }
        }
    }

    fn fail(&self, id: &str, cause: LossCause) {
        let mut guard = self.lock();
        self.lose(&mut guard, id, cause, cause.reason());
    }

    /// Settle an unsettled producer as lost, once. An admitted one latches its outcome; one
    /// the sink never admitted is only counted. Returns whether anything changed.
    fn lose(&self, state: &mut EntryState, id: &str, cause: LossCause, reason: GapReason) -> bool {
        let Some(p) = state.producers.get_mut(id) else {
            return false;
        };
        let admitted = match p.status {
            Status::Live | Status::Closing(_) => true,
            Status::Admitting => false,
            _ => return false,
        };
        p.status = if admitted {
            Status::Lost
        } else {
            Status::Refused
        };
        if let Some(stop) = p.stop.take() {
            stop.send_replace(true);
        }
        let admitted_through = p.next_seq.checked_sub(1);
        if admitted {
            state
                .outcomes
                .push_back(ProducerOutcome::Lost(ProducerLoss {
                    producer: id.to_string(),
                    observed: (self.clock)(),
                    reason,
                    cause,
                    admitted_through,
                }));
        } else {
            state.admissions_refused += 1;
        }
        self.bump();
        true
    }

    /// Close admission and cancel every unsettled producer. Returns how many.
    fn cancel_unsettled(&self) -> usize {
        let mut guard = self.lock();
        let state = &mut *guard;
        state.admission_open = false;
        let unsettled: Vec<String> = state
            .producers
            .iter()
            .filter(|(_, p)| p.status.unsettled())
            .map(|(id, _)| id.clone())
            .collect();
        unsettled
            .iter()
            .filter(|id| self.lose(state, id, LossCause::Cancelled, GapReason::ShutdownDeadline))
            .count()
    }
}

/// The receiver's sequence, stream-state and offset checks, before anything changes.
fn check(state: &EntryState, id: &str, frame: &CaptureFrame) -> Result<Step, LossCause> {
    if frame.stream.producer != id {
        return Err(LossCause::ForeignProducer);
    }
    let p = &state.producers[id];
    if frame.seq != p.next_seq || p.next_seq == u64::MAX {
        return Err(LossCause::Sequence);
    }
    let stream = frame.stream.stream;
    match &frame.event {
        FrameEvent::Connection { .. } => Ok(Step::Control),
        FrameEvent::Loss { .. } => Ok(Step::Loss),
        FrameEvent::Recovery {
            previous_producer, ..
        } => {
            let named = state.producers.get(previous_producer).map(|q| q.status);
            if frame.seq == 0 && named.is_some_and(Status::recoverable) {
                Ok(Step::Recovery(previous_producer.clone()))
            } else {
                Err(LossCause::Recovery)
            }
        }
        FrameEvent::Open { .. } => {
            if stream <= p.high_stream {
                Err(LossCause::StreamState)
            } else if state.open_streams >= MAX_OPEN_STREAMS {
                Err(LossCause::StreamLimit)
            } else {
                Ok(Step::Open(stream))
            }
        }
        FrameEvent::Bytes { offset, data } => {
            let position = p.open.get(&stream).ok_or(LossCause::StreamState)?;
            let end = offset
                .checked_add(data.len() as u64)
                .ok_or(LossCause::Offset)?;
            if position.admits(*offset) {
                Ok(Step::Bytes(stream, end))
            } else {
                Err(LossCause::Offset)
            }
        }
        FrameEvent::Gap { .. } if p.open.contains_key(&stream) => Ok(Step::Gap(stream)),
        FrameEvent::End { .. } if p.open.contains_key(&stream) => Ok(Step::End(stream)),
        FrameEvent::Gap { .. } | FrameEvent::End { .. } => Err(LossCause::StreamState),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn golden_handshake_body() -> Vec<u8> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../goalcup-protocol/tests/fixtures/m2-ingest/golden-stream.hex"
        );
        let text = std::fs::read_to_string(path).unwrap();
        let first_line = text.lines().next().unwrap();
        let framed = hex::decode(first_line.trim()).unwrap();
        let len = checked_frame_len(framed[..4].try_into().unwrap(), MAX_HANDSHAKE_BYTES).unwrap();
        assert_eq!(
            framed.len(),
            4 + len,
            "the first golden line is exactly the handshake"
        );
        framed[4..].to_vec()
    }

    #[test]
    fn private_handshake_codec_is_byte_exact_with_the_shared_golden() {
        let body = golden_handshake_body();
        let handshake = decode_handshake(&body).expect("the golden handshake decodes");
        assert_eq!(handshake.entry_id, "entry-m2-0001");
        assert_eq!(handshake.producer, "7a3f9c21d4e58b06a1c2d3e4f5061728");
        assert_eq!(
            serde_json::to_vec(&handshake).unwrap(),
            body,
            "fields in order entry_id, producer, token; compact JSON"
        );
        assert!(!format!("{handshake:?}").contains(&handshake.token));
    }

    #[test]
    fn handshake_refuses_unknown_fields_and_bad_ids() {
        let ok = br#"{"entry_id":"e","producer":"7a3f9c21d4e58b06a1c2d3e4f5061728","token":"t"}"#;
        assert!(decode_handshake(ok).is_some());
        let bad: [&[u8]; 5] = [
            br#"{"entry_id":"e","producer":"7a3f9c21d4e58b06a1c2d3e4f5061728","token":"t","x":1}"#,
            br#"{"entry_id":"e","producer":"7A3F9C21D4E58B06A1C2D3E4F5061728","token":"t"}"#,
            br#"{"entry_id":"","producer":"7a3f9c21d4e58b06a1c2d3e4f5061728","token":"t"}"#,
            br#"{"entry_id":"e","producer":"7a3f9c21d4e58b06a1c2d3e4f5061728","token":""}"#,
            br#"{"entry_id":"e","producer":"7a3f9c21d4e58b06a1c2d3e4f5061728"}"#,
        ];
        for body in bad {
            assert!(decode_handshake(body).is_none());
        }
    }
}
