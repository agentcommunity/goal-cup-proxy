//! The bounded IPC [`CaptureSink`] that carries one proxy launch's capture frames to the
//! daemon's ingest endpoint, with capture-only fresh-epoch recovery and an ordered producer
//! close. [`connect_sink`] builds it.
//!
//! # Wire
//!
//! One loopback TCP connection per producer epoch. Its first item is the handshake: a
//! big-endian `u32` length and the compact JSON object `{"entry_id","producer","token"}`, in
//! that order, at most [`MAX_HANDSHAKE_BYTES`] (the shared golden is
//! `goalcup-protocol/tests/fixtures/m2-ingest/golden-stream.hex`). Every later item is one
//! length-prefixed [`IngestMessage`] from [`encode_ingest_message`], unchanged.
//!
//! Nothing is ever read back. The receiver sends no acknowledgement, so the sender never waits
//! for one: any byte, EOF or error arriving from the receiver ends the epoch as unknown loss.
//! A remote close carries no reason, so it is always `IngestLost`, never an authentication or
//! storage verdict the sender cannot know.
//!
//! # Submission
//!
//! Every `try_*` call is synchronous: it takes one short mutex, never awaits and never does
//! I/O. A frame is encoded once, outside the mutex; its **actual encoded length** is then
//! reserved against the queue bound and the bytes are queued, in call order, for the one
//! writer task. The reservation is released only when that buffer has been handed to the
//! socket or discarded. [`CaptureEmitter`](super::api::CaptureEmitter) holds its ordering lock
//! through each call, so the queue order is its sequence order; the sink checks that order and
//! never renumbers a frame. `Queued` is volatile acceptance only: not a receipt, not storage.
//!
//! The queue is bounded by [`MAX_CAPTURE_QUEUE_BYTES`] in three nested lanes, so pressure on
//! payload never starves the controls that keep an epoch readable: payload frames stop at
//! [`PAYLOAD_QUEUE_BYTES`]; stream-zero controls (Connection, Recovery, Loss) may use the
//! [`CONTROL_RESERVE_BYTES`] above it except its last [`CLOSE_RESERVE_BYTES`]; only the final
//! `Close` may use those.
//!
//! # Loss, recovery and the retry budget
//!
//! A payload frame that does not fit latches the epoch lost with `QueueFull`: the writer
//! stops at the latch, every queued buffer of the epoch is discarded, and the socket is closed
//! **without** `Close`, so the receiver records unknown tail loss. A remote close or a write
//! error latches `IngestLost` the same way. Either answer is sticky for that producer: its
//! later frames and its close are refused (with the same reason, or `ProducerLost` once a
//! newer epoch is live, which they never reach), and its ID is never reused.
//!
//! Only the caller's next proposal of a **fresh** producer ([`CaptureSink::try_begin_epoch`],
//! driven by `CaptureEmitter::try_recover` at a safe relay boundary) starts a new connection.
//! One attempt runs at a time, in the background; polls answer `Pending` with no I/O until it
//! ends. The attempt after the n-th one waits [`RECONNECT_DELAYS`] (1, 2, 5, then 10 seconds)
//! after the previous attempt or epoch ended. A failure before any handshake byte was written
//! retries the same ID; once a handshake may have reached the receiver the ID is spent, and a
//! poll for it answers `RetryFresh` unless it was already answered `Ready`.
//! [`MAX_CONNECTION_FAILURES`] failed attempts and unexpected socket losses **in a row**, or
//! [`MAX_PRODUCERS`] spent IDs per launch, disable the sink for good. An epoch that took items
//! and stayed up for [`STABLE_EPOCH`] before it was lost ends the row (that loss is the first of
//! the next), so a week of daemon stalls that each recover is not a reason to stop capturing;
//! `Ready` alone does not, nor does a receiver that takes a frame and closes at once (one that
//! refuses the credential), and a new ID cannot evade the count. A clean shutdown and a
//! `QueueFull` latch are not failures.
//! Nothing is ever replayed: a new epoch starts with an empty queue, so the emitter's
//! `Recovery` right after `Ready` never meets queue pressure. If that epoch's socket is
//! already known dead, the `Recovery` is refused (`IngestLost`), never answered `Queued` and
//! dropped, and the emitter ends `Disabled`.
//!
//! # Lifetime
//!
//! [`connect_sink`] starts the one background task and the initial attempt for the configured
//! producer. [`IngestSink::shutdown`] is the one bounded end: inside the configured deadline
//! it hands every queued frame to the socket, then the queued `Close`, and closes the socket;
//! at the deadline it aborts the task and closes the socket with the rest unwritten. Dropping
//! the last handle aborts the task and closes the socket at once; nothing resurrects it. The
//! secret lives only in this process's memory: it is written into each handshake and appears
//! in no `Debug`, error, log, argument or environment variable.

use goalcup_protocol::capture::{
    CaptureFrame, CaptureLimitError, CaptureSink, EpochAdmission, GapReason, IngestMessage,
    MAX_CAPTURE_QUEUE_BYTES, MAX_HANDSHAKE_BYTES, MAX_PRODUCERS, SubmitResult,
    encode_ingest_message, is_capture_id,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// The reconnect schedule: the pause before the second, third, fourth and every later
/// connection attempt of one launch, measured from the end of the attempt or epoch before it.
pub const RECONNECT_DELAYS: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
];

/// Failed connection attempts plus unexpected socket losses in a row that one launch
/// tolerates. The one that reaches this count disables the sink (`IngestLost`). A connection
/// that worked (it took items and lived [`STABLE_EPOCH`]) ends the row: only a receiver that
/// does not keep a connection, attempt after attempt, is given up on.
pub const MAX_CONNECTION_FAILURES: u64 = 10;

/// How long an epoch that took items must have stayed up before its loss starts a new row of
/// failures instead of adding to the last one.
pub const STABLE_EPOCH: Duration = Duration::from_secs(60);

/// Bounds one attempt's TCP connect, and separately its handshake write.
pub const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(2);

/// Capacity above the payload lane held back for stream-zero controls and the final `Close`.
/// It holds a `Recovery` plus both re-observations of each of `MAX_OPEN_CONNECTIONS`
/// connections even at the 1 KiB `clock_id` bound, with room to spare.
pub const CONTROL_RESERVE_BYTES: usize = 512 * 1024;

/// The part of [`CONTROL_RESERVE_BYTES`] only the final `Close` (one small item) may use.
pub const CLOSE_RESERVE_BYTES: usize = 1024;

/// The ceiling on queued and in-flight payload frames, in encoded bytes.
pub const PAYLOAD_QUEUE_BYTES: usize = MAX_CAPTURE_QUEUE_BYTES - CONTROL_RESERVE_BYTES;

/// The ceiling for stream-zero controls: everything but the close reserve.
const CONTROL_QUEUE_BYTES: usize = MAX_CAPTURE_QUEUE_BYTES - CLOSE_RESERVE_BYTES;

const _: () = assert!(CLOSE_RESERVE_BYTES < CONTROL_RESERVE_BYTES);
const _: () = assert!(CONTROL_RESERVE_BYTES < MAX_CAPTURE_QUEUE_BYTES / 8);

/// The launch-scoped ingest credential. It stays in this process's memory and is written only
/// into handshakes; it has no `Display`, and its `Debug` is redacted.
pub struct IngestSecret(String);

impl IngestSecret {
    pub fn new(secret: String) -> Self {
        IngestSecret(secret)
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for IngestSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("IngestSecret(<redacted>)")
    }
}

/// Everything [`connect_sink`] needs, supplied explicitly by the caller. Where the endpoint
/// and the secret come from is the caller's business.
pub struct SinkConfig {
    endpoint: SocketAddr,
    entry_id: String,
    secret: IngestSecret,
    producer: String,
    shutdown_deadline: Duration,
    reconnect_delays: [Duration; 4],
    stable_after: Duration,
}

impl SinkConfig {
    /// `endpoint` is the daemon's loopback ingest listener; `entry_id` and `secret` authorize
    /// this launch's fresh producers for that one entry; `producer` is the initial producer ID
    /// (32 lowercase hex), the same one the caller hands the proxy; `shutdown_deadline` is the
    /// one bound on [`IngestSink::shutdown`]. Reconnects follow [`RECONNECT_DELAYS`].
    pub fn new(
        endpoint: SocketAddr,
        entry_id: String,
        secret: IngestSecret,
        producer: String,
        shutdown_deadline: Duration,
    ) -> Self {
        SinkConfig {
            endpoint,
            entry_id,
            secret,
            producer,
            shutdown_deadline,
            reconnect_delays: RECONNECT_DELAYS,
            stable_after: STABLE_EPOCH,
        }
    }

    /// **Test-only.** Replaces [`STABLE_EPOCH`] so tests need not wait a minute.
    #[doc(hidden)]
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_stable_after(mut self, stable_after: Duration) -> Self {
        self.stable_after = stable_after;
        self
    }

    /// **Test-only.** Replaces the reconnect schedule (every entry nonzero) so tests can
    /// exercise the schedule's shape in seconds. Production must keep [`RECONNECT_DELAYS`]
    /// (the schedule `new` sets); no shipped caller may use this.
    #[doc(hidden)]
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_reconnect_delays(mut self, delays: [Duration; 4]) -> Self {
        self.reconnect_delays = delays;
        self
    }

    pub fn reconnect_delays(&self) -> [Duration; 4] {
        self.reconnect_delays
    }
}

impl fmt::Debug for SinkConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SinkConfig")
            .field("endpoint", &self.endpoint)
            .field("entry_id", &self.entry_id)
            .field("secret", &self.secret)
            .field("producer", &self.producer)
            .field("shutdown_deadline", &self.shutdown_deadline)
            .field("reconnect_delays", &self.reconnect_delays)
            .field("stable_after", &self.stable_after)
            .finish()
    }
}

/// Why [`connect_sink`] built no sink. Fixed text and sizes only, never an input value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SinkConfigError {
    #[error("the ingest endpoint is not a loopback address with a nonzero port")]
    NotLoopback,
    #[error("the entry ID or the ingest secret is empty")]
    EmptyCredential,
    #[error("the initial producer ID is not 32 lowercase hex characters")]
    InvalidProducer,
    #[error("the ingest handshake would exceed its {limit}-byte limit")]
    HandshakeTooLarge { limit: usize },
    #[error("the shutdown deadline or a reconnect delay is zero")]
    InvalidTiming,
    #[error("the capture sink needs a running Tokio runtime")]
    NoRuntime,
}

/// The handshake, private to the sender: field order `entry_id`, `producer`, `token`;
/// unknown fields refused; `Debug` never shows the token.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Handshake<'a> {
    entry_id: &'a str,
    producer: &'a str,
    token: &'a str,
}

impl fmt::Debug for Handshake<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handshake")
            .field("entry_id", &self.entry_id)
            .field("producer", &self.producer)
            .finish_non_exhaustive()
    }
}

/// The framed handshake, or `None` over [`MAX_HANDSHAKE_BYTES`]. JSON never shortens a
/// string, so an over-limit raw length is refused before anything is serialized.
fn encode_handshake(entry_id: &str, producer: &str, token: &str) -> Option<Vec<u8>> {
    if entry_id.len() + producer.len() + token.len() > MAX_HANDSHAKE_BYTES {
        return None;
    }
    let body = serde_json::to_vec(&Handshake {
        entry_id,
        producer,
        token,
    })
    .ok()?;
    if body.len() > MAX_HANDSHAKE_BYTES {
        return None;
    }
    let mut framed = Vec::with_capacity(4 + body.len());
    framed.extend_from_slice(&(body.len() as u32).to_be_bytes());
    framed.extend_from_slice(&body);
    Some(framed)
}

/// Counters of one sink, for its owner's report. Byte counts are encoded item bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SenderStats {
    /// TCP connects started.
    pub connection_attempts: u64,
    /// Handshakes whose write began: producer IDs the receiver may have consumed.
    pub handshakes_sent: u64,
    /// Epochs whose handshake was written, so they accepted frames. Not a receiver answer.
    pub epochs_ready: u64,
    /// Failed attempts plus unexpected socket losses, all told. The sink is disabled when
    /// [`MAX_CONNECTION_FAILURES`] of them come in a row (`State::consecutive_failures`).
    pub failures: u64,
    /// Answered-`Ready` epochs that ended in capture loss (`QueueFull`/`IngestLost`).
    /// Sticky: a later recovered epoch never lowers it.
    pub losses: u64,
    /// Of `losses`: the ones latched by queue pressure.
    pub queue_full: u64,
    pub frames_queued: u64,
    /// Items (frames and `Close`) handed to a socket in full.
    pub items_written: u64,
    pub bytes_written: u64,
    /// Queued plus in-flight bytes now.
    pub queued_bytes: u64,
    /// The most queued plus in-flight bytes ever held at once.
    pub high_water_bytes: u64,
    /// Admitted bytes never fully handed to a socket: discarded at a loss or the deadline.
    pub discarded_bytes: u64,
}

/// How the final producer epoch ended at [`IngestSink::shutdown`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SenderOutcome {
    /// Every admitted frame of the final epoch and then its `Close` were handed to the socket
    /// in order, and the socket was closed. A local drain boundary only: written is not
    /// received, validated or stored, and no acknowledgement exists to say otherwise.
    Closed {
        producer: String,
        final_seq: Option<u64>,
    },
    /// No `Close` was queued: the queued frames were written and the socket closed, which the
    /// receiver reads as unknown tail loss.
    NotClosed,
    /// The final epoch had ended in capture loss or a terminal refusal before shutdown.
    CaptureLost(GapReason),
    /// The deadline passed first: the task was aborted and the socket closed with
    /// `unwritten_bytes` of admitted items never fully handed to it. Whether any written
    /// bytes were stored is unknowable here.
    DeadlineExpired { unwritten_bytes: u64 },
    /// No epoch ever became ready.
    NeverReady,
}

/// What [`IngestSink::shutdown`] observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SenderReport {
    pub outcome: SenderOutcome,
    pub stats: SenderStats,
}

impl SenderReport {
    /// A clean producer shutdown with no loss anywhere in the launch: the final epoch closed
    /// in order and no epoch ever lost frames. Still no claim of durability.
    pub fn is_clean(&self) -> bool {
        matches!(self.outcome, SenderOutcome::Closed { .. }) && self.stats.losses == 0
    }
}

/// One established producer epoch.
#[derive(Debug)]
struct Epoch {
    producer: String,
    /// The caller was answered `Ready` for this epoch, or submitted to it.
    claimed: bool,
    next_seq: u64,
    last_seq: Option<u64>,
}

#[derive(Debug)]
enum Phase {
    /// The background attempt for `producer` is in flight: waiting out its delay,
    /// connecting, or writing the handshake.
    Attempt { producer: String },
    /// An attempt failed after its handshake may have reached the receiver, before the caller
    /// was answered `Ready`: the ID is spent and the next poll for it is `RetryFresh`.
    Spent { producer: String },
    /// No epoch and no attempt.
    Idle,
    /// The handshake was written: frames are accepted.
    Live(Epoch),
    /// `Close` is queued behind the epoch's frames; nothing more is accepted.
    Closing(Epoch),
    /// `Close` was handed to the socket and the socket closed.
    Closed,
    /// The epoch ended in capture loss; its ID is never reused and every later frame of it,
    /// its first included, is refused with `reason`.
    Lost { reason: GapReason },
    /// Terminal: an exhausted budget, or locally invalid framing or configuration.
    Disabled(GapReason),
}

/// Which bound a queued item is admitted under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lane {
    Payload,
    Control,
    Close,
}

impl Lane {
    fn limit(self) -> usize {
        match self {
            Lane::Payload => PAYLOAD_QUEUE_BYTES,
            Lane::Control => CONTROL_QUEUE_BYTES,
            Lane::Close => MAX_CAPTURE_QUEUE_BYTES,
        }
    }
}

struct Item {
    bytes: Vec<u8>,
    close: bool,
}

struct State {
    phase: Phase,
    queue: VecDeque<Item>,
    /// Bytes in `queue`.
    queued_bytes: usize,
    /// Bytes of the one item the writer is handing to the socket.
    inflight_bytes: usize,
    /// Shutdown began: no new frame, close or epoch; the writer drains and ends.
    stopping: bool,
    /// Producer IDs whose handshake write began (at most `MAX_PRODUCERS`).
    spent: BTreeSet<String>,
    last_loss: Option<GapReason>,
    /// Set by the writer when it ends the final epoch itself, or by the deadline.
    outcome: Option<SenderOutcome>,
    stats: SenderStats,
    /// Failures since a connection last worked ([`STABLE_EPOCH`]).
    consecutive_failures: u64,
    /// The live epoch's start, and whether it has taken an item.
    epoch_since: Option<Instant>,
    epoch_wrote: bool,
}

impl State {
    /// A sink whose first attempt, for `producer`, is about to run.
    fn new(producer: String) -> Self {
        State {
            phase: Phase::Attempt { producer },
            queue: VecDeque::new(),
            queued_bytes: 0,
            inflight_bytes: 0,
            stopping: false,
            spent: BTreeSet::new(),
            last_loss: None,
            outcome: None,
            stats: SenderStats::default(),
            consecutive_failures: 0,
            epoch_since: None,
            epoch_wrote: false,
        }
    }

    fn total(&self) -> usize {
        self.queued_bytes + self.inflight_bytes
    }

    fn sync_queued(&mut self) {
        self.stats.queued_bytes = self.total() as u64;
    }

    fn epoch(&self, producer: &str) -> Option<&Epoch> {
        match &self.phase {
            Phase::Live(epoch) | Phase::Closing(epoch) if epoch.producer == producer => Some(epoch),
            _ => None,
        }
    }

    /// Discards every queued buffer. The in-flight one belongs to the writer, which abandons
    /// or finishes it and releases it itself.
    fn discard_queue(&mut self) {
        self.stats.discarded_bytes += self.queued_bytes as u64;
        self.queue.clear();
        self.queued_bytes = 0;
        self.sync_queued();
    }

    fn discard_inflight(&mut self) {
        self.stats.discarded_bytes += self.inflight_bytes as u64;
        self.inflight_bytes = 0;
        self.sync_queued();
    }

    fn enqueue(&mut self, bytes: Vec<u8>, close: bool) {
        self.queued_bytes += bytes.len();
        self.queue.push_back(Item { bytes, close });
        self.sync_queued();
        self.stats.high_water_bytes = self.stats.high_water_bytes.max(self.stats.queued_bytes);
    }

    /// Ends the current epoch in capture loss: stop at the latch, discard every queued
    /// buffer, never write `Close`. The writer closes the socket when it sees the latch.
    fn latch_loss(&mut self, reason: GapReason) {
        self.discard_queue();
        self.stats.losses += 1;
        if reason == GapReason::QueueFull {
            self.stats.queue_full += 1;
        }
        self.last_loss = Some(reason);
        self.phase = Phase::Lost { reason };
    }

    /// Disables the sink for good.
    fn latch_terminal(&mut self, reason: GapReason) {
        self.discard_queue();
        self.last_loss = Some(reason);
        self.phase = Phase::Disabled(reason);
    }

    /// Counts one failed attempt or unexpected socket loss; the last one allowed in a row
    /// disables the sink.
    fn count_failure(&mut self) -> bool {
        self.stats.failures += 1;
        self.consecutive_failures += 1;
        let exhausted = self.consecutive_failures >= MAX_CONNECTION_FAILURES;
        if exhausted {
            self.latch_terminal(GapReason::IngestLost);
        }
        exhausted
    }

    /// `try_submit` under the lock.
    fn submit(
        &mut self,
        producer: &str,
        seq: u64,
        lane: Lane,
        encoded: Result<Vec<u8>, CaptureLimitError>,
    ) -> SubmitResult {
        if self.stopping {
            return SubmitResult::Disabled(GapReason::ShutdownDeadline);
        }
        let expected = match &mut self.phase {
            Phase::Live(epoch) if epoch.producer == producer => Some(epoch.next_seq),
            // A stale frame of a retired epoch: refused, and it never reaches the live one.
            Phase::Live(_) if self.spent.contains(producer) => {
                return SubmitResult::Disabled(GapReason::ProducerLost);
            }
            // A frame of an unknown producer on this epoch's connection: invalid framing.
            Phase::Live(_) => None,
            // A known-lost epoch refuses everything, a `Recovery` right after `Ready` included:
            // answering `Queued` into it would hide the loss from the emitter.
            Phase::Lost { reason } => return SubmitResult::Disabled(*reason),
            Phase::Disabled(reason) => return SubmitResult::Disabled(*reason),
            Phase::Closing(_) | Phase::Closed => {
                return SubmitResult::Disabled(GapReason::ShutdownDeadline);
            }
            // No epoch is established: the caller's epoch was lost with its socket.
            Phase::Attempt { .. } | Phase::Spent { .. } | Phase::Idle => {
                return SubmitResult::Disabled(GapReason::IngestLost);
            }
        };
        // Locally known invalid framing is terminal: a frame over a protocol limit, a foreign
        // producer, or a break in the producer's sequence (the sink never renumbers).
        let bytes = match encoded {
            Ok(bytes) if expected == Some(seq) => bytes,
            _ => {
                self.latch_terminal(GapReason::LimitExceeded);
                return SubmitResult::Disabled(GapReason::LimitExceeded);
            }
        };
        if self.total() + bytes.len() > lane.limit() {
            self.latch_loss(GapReason::QueueFull);
            return SubmitResult::Disabled(GapReason::QueueFull);
        }
        if let Phase::Live(epoch) = &mut self.phase {
            epoch.claimed = true;
            epoch.next_seq += 1;
            epoch.last_seq = Some(seq);
        }
        self.stats.frames_queued += 1;
        self.enqueue(bytes, false);
        SubmitResult::Queued
    }

    /// `try_begin_epoch` under the lock.
    fn begin(&mut self, producer: &str) -> EpochAdmission {
        if self.stopping {
            return EpochAdmission::Disabled(GapReason::ShutdownDeadline);
        }
        match &mut self.phase {
            Phase::Disabled(reason) => return EpochAdmission::Disabled(*reason),
            Phase::Closing(_) | Phase::Closed => {
                return EpochAdmission::Disabled(GapReason::ShutdownDeadline);
            }
            Phase::Attempt { producer: current } if current == producer => {
                return EpochAdmission::Pending;
            }
            Phase::Live(epoch) if epoch.producer == producer => {
                epoch.claimed = true;
                return EpochAdmission::Ready;
            }
            _ => {}
        }
        // An ID whose handshake may have reached the receiver never becomes ready again.
        if self.spent.contains(producer) {
            if matches!(&self.phase, Phase::Spent { producer: spent } if spent == producer) {
                self.phase = Phase::Idle;
            }
            return EpochAdmission::RetryFresh;
        }
        // Another ID while an attempt or epoch is in progress: the caller and this sink
        // disagree about the producer, which is invalid configuration.
        if matches!(self.phase, Phase::Attempt { .. } | Phase::Live(_)) {
            self.latch_terminal(GapReason::LimitExceeded);
            return EpochAdmission::Disabled(GapReason::LimitExceeded);
        }
        // A fresh proposal after a loss or a spent attempt.
        if self.spent.len() >= MAX_PRODUCERS {
            self.latch_terminal(GapReason::LimitExceeded);
            return EpochAdmission::Disabled(GapReason::LimitExceeded);
        }
        self.phase = Phase::Attempt {
            producer: producer.to_string(),
        };
        EpochAdmission::Pending
    }

    /// `try_close_epoch` under the lock.
    fn close(
        &mut self,
        producer: &str,
        final_seq: Option<u64>,
        encoded: Result<Vec<u8>, CaptureLimitError>,
    ) -> SubmitResult {
        if self.stopping {
            return SubmitResult::Disabled(GapReason::ShutdownDeadline);
        }
        let last_seq = match &self.phase {
            Phase::Live(epoch) if epoch.producer == producer => Some(epoch.last_seq),
            Phase::Closing(epoch) if epoch.producer == producer && epoch.last_seq == final_seq => {
                return SubmitResult::Queued;
            }
            Phase::Live(_) if self.spent.contains(producer) => {
                return SubmitResult::Disabled(GapReason::ProducerLost);
            }
            Phase::Live(_) => None,
            Phase::Lost { reason } | Phase::Disabled(reason) => {
                return SubmitResult::Disabled(*reason);
            }
            Phase::Closing(_) | Phase::Closed => {
                return SubmitResult::Disabled(GapReason::ShutdownDeadline);
            }
            Phase::Attempt { .. } | Phase::Spent { .. } | Phase::Idle => {
                return SubmitResult::Disabled(GapReason::IngestLost);
            }
        };
        // A close naming anything but this epoch's last admitted sequence would claim a drain
        // that did not happen: refused, and the epoch ends without `Close` (unknown loss).
        let bytes = match encoded {
            Ok(bytes) if last_seq == Some(final_seq) => bytes,
            _ => {
                self.latch_terminal(GapReason::LimitExceeded);
                return SubmitResult::Disabled(GapReason::LimitExceeded);
            }
        };
        if self.total() + bytes.len() > Lane::Close.limit() {
            self.latch_loss(GapReason::QueueFull);
            return SubmitResult::Disabled(GapReason::QueueFull);
        }
        if let Phase::Live(epoch) = std::mem::replace(&mut self.phase, Phase::Idle) {
            self.phase = Phase::Closing(epoch);
        }
        self.enqueue(bytes, true);
        SubmitResult::Queued
    }
}

struct Shared {
    state: Mutex<State>,
    /// Wakes the one writer task. Only it waits, so a stored permit is never lost.
    wake: Notify,
}

impl Shared {
    /// A poisoned lock is taken over: every transition leaves the state consistent, and the
    /// relay path must never panic on it.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn stopping(&self) -> bool {
        self.lock().stopping
    }
}

/// The IPC capture sink. Share it with the proxy as `Arc<dyn CaptureSink>`; keep one
/// `Arc<IngestSink>` to call [`IngestSink::shutdown`].
pub struct IngestSink {
    shared: Arc<Shared>,
    writer: Mutex<Option<JoinHandle<()>>>,
    shutdown_deadline: Duration,
}

impl fmt::Debug for IngestSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IngestSink")
            .field("stats", &self.stats())
            .field("shutdown_deadline", &self.shutdown_deadline)
            .finish_non_exhaustive()
    }
}

/// Builds the sink and starts its one background task, whose first attempt connects for the
/// configured initial producer at once. Must run inside a Tokio runtime; performs no I/O
/// itself. The caller then polls `try_begin_epoch(producer)` until `Ready` (the proxy's startup
/// does) before its first frame.
pub fn connect_sink(config: SinkConfig) -> Result<Arc<IngestSink>, SinkConfigError> {
    let SinkConfig {
        endpoint,
        entry_id,
        secret,
        producer,
        shutdown_deadline,
        reconnect_delays,
        stable_after,
    } = config;
    if !endpoint.ip().is_loopback() || endpoint.port() == 0 {
        return Err(SinkConfigError::NotLoopback);
    }
    if entry_id.is_empty() || secret.expose().is_empty() {
        return Err(SinkConfigError::EmptyCredential);
    }
    if !is_capture_id(&producer) {
        return Err(SinkConfigError::InvalidProducer);
    }
    // Every producer ID has the same length, so one encoding bounds every epoch's handshake.
    if encode_handshake(&entry_id, &producer, secret.expose()).is_none() {
        return Err(SinkConfigError::HandshakeTooLarge {
            limit: MAX_HANDSHAKE_BYTES,
        });
    }
    if shutdown_deadline.is_zero() || reconnect_delays.iter().any(Duration::is_zero) {
        return Err(SinkConfigError::InvalidTiming);
    }
    let runtime = tokio::runtime::Handle::try_current().map_err(|_| SinkConfigError::NoRuntime)?;
    let shared = Arc::new(Shared {
        state: Mutex::new(State::new(producer)),
        wake: Notify::new(),
    });
    let writer = Writer {
        shared: Arc::clone(&shared),
        endpoint,
        entry_id,
        secret,
        delays: reconnect_delays,
        stable_after,
    };
    let task = runtime.spawn(writer.run());
    Ok(Arc::new(IngestSink {
        shared,
        writer: Mutex::new(Some(task)),
        shutdown_deadline,
    }))
}

impl IngestSink {
    /// A snapshot of the counters. Never waits on the socket.
    pub fn stats(&self) -> SenderStats {
        self.shared.lock().stats.clone()
    }

    /// Ends the sink inside the one configured shutdown deadline, measured from this call.
    ///
    /// From now on every `try_*` call is refused (`ShutdownDeadline`) and no attempt starts.
    /// The writer hands every queued frame to the socket, then the `Close` if the caller
    /// closed the producer, and closes the socket. At the deadline the task is aborted and
    /// the socket closed with the rest unwritten: an incomplete epoch, never an invented clean
    /// close.
    ///
    /// Call it once. Cancelling this future (dropping it before it completes) aborts the
    /// writer at once, exactly as the deadline would, instead of leaving it to drain
    /// unobserved. A later call, even one made while the first is still draining, does not
    /// wait: it reports from the state it finds.
    pub async fn shutdown(&self) -> SenderReport {
        let deadline = Instant::now() + self.shutdown_deadline;
        self.shared.lock().stopping = true;
        self.shared.wake.notify_one();
        let task = self
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        let mut owned = AbortOnDrop(task);
        if let Some(task) = owned.0.as_mut()
            && tokio::time::timeout_at(deadline, &mut *task).await.is_err()
        {
            task.abort();
            // The task never blocks a thread, so the abort lands at once and drops the socket.
            let _ = (&mut *task).await;
            let mut state = self.shared.lock();
            if state.outcome.is_none() {
                let unwritten_bytes = state.total() as u64;
                state.discard_queue();
                state.discard_inflight();
                state.outcome = Some(SenderOutcome::DeadlineExpired { unwritten_bytes });
            }
        }
        // Joined (or aborted and joined): nothing is left to abort.
        owned.0 = None;
        let state = self.shared.lock();
        let outcome = match (&state.outcome, &state.phase) {
            (Some(outcome), _) => outcome.clone(),
            (None, Phase::Lost { reason } | Phase::Disabled(reason)) => {
                SenderOutcome::CaptureLost(*reason)
            }
            (None, _) => match state.last_loss {
                Some(reason) => SenderOutcome::CaptureLost(reason),
                None => SenderOutcome::NeverReady,
            },
        };
        SenderReport {
            outcome,
            stats: state.stats.clone(),
        }
    }
}

/// Aborts the writer if the future holding it is dropped before the writer was joined.
struct AbortOnDrop(Option<JoinHandle<()>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

impl Drop for IngestSink {
    /// Abrupt: the task is aborted and the socket closed without `Close`, which the receiver
    /// reads as unknown tail loss. Nothing restarts it.
    fn drop(&mut self) {
        let task = self
            .writer
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(task) = task {
            task.abort();
        }
    }
}

impl CaptureSink for IngestSink {
    fn try_submit(&self, frame: CaptureFrame) -> SubmitResult {
        let seq = frame.seq;
        let lane = if frame.stream.stream == 0 {
            Lane::Control
        } else {
            Lane::Payload
        };
        let producer = frame.stream.producer.clone();
        // Validated and bounded before anything is serialized (`encode_ingest_message`).
        let encoded = encode_ingest_message(&IngestMessage::Frame(frame));
        let answer = self.shared.lock().submit(&producer, seq, lane, encoded);
        self.shared.wake.notify_one();
        answer
    }

    fn try_begin_epoch(&self, producer: &str) -> EpochAdmission {
        if !is_capture_id(producer) {
            return EpochAdmission::Disabled(GapReason::LimitExceeded);
        }
        let answer = self.shared.lock().begin(producer);
        if answer != EpochAdmission::Ready {
            self.shared.wake.notify_one();
        }
        answer
    }

    fn try_close_epoch(&self, producer: &str, final_seq: Option<u64>) -> SubmitResult {
        let encoded = encode_ingest_message(&IngestMessage::Close {
            producer: producer.to_string(),
            final_seq,
        });
        let answer = self.shared.lock().close(producer, final_seq, encoded);
        self.shared.wake.notify_one();
        answer
    }
}

/// The one background task: at most one connection attempt at a time, then the one ordered
/// writer of that epoch's items.
struct Writer {
    shared: Arc<Shared>,
    endpoint: SocketAddr,
    entry_id: String,
    secret: IngestSecret,
    delays: [Duration; 4],
    stable_after: Duration,
}

/// The writer's next step inside an epoch.
enum Next {
    Write(Item),
    Wait,
    Drained,
    Ended,
}

impl Writer {
    async fn run(self) {
        let mut attempts: usize = 0;
        let mut last_end = Instant::now();
        while let Some(producer) = self.next_attempt().await {
            if attempts > 0 {
                let delay = self.delays[(attempts - 1).min(self.delays.len() - 1)];
                if !self.pause_until(last_end + delay).await {
                    return;
                }
            }
            attempts += 1;
            if let Some(stream) = self.open_epoch(&producer).await
                && self.run_epoch(stream, &producer).await
            {
                return;
            }
            last_end = Instant::now();
        }
    }

    /// Waits for an attempt to run; `None` once the sink can never attempt again.
    async fn next_attempt(&self) -> Option<String> {
        loop {
            {
                let state = self.shared.lock();
                if state.stopping {
                    return None;
                }
                match &state.phase {
                    Phase::Attempt { producer } => return Some(producer.clone()),
                    Phase::Disabled(_) | Phase::Closed => return None,
                    _ => {}
                }
            }
            self.shared.wake.notified().await;
        }
    }

    /// Sleeps until `at`; `false` if shutdown began first.
    async fn pause_until(&self, at: Instant) -> bool {
        self.or_stop(tokio::time::sleep_until(at)).await.is_some()
    }

    /// Runs `work` unless shutdown begins first (`None`).
    async fn or_stop<F: Future>(&self, work: F) -> Option<F::Output> {
        tokio::pin!(work);
        loop {
            if self.shared.stopping() {
                return None;
            }
            tokio::select! {
                out = &mut work => return Some(out),
                _ = self.shared.wake.notified() => {}
            }
        }
    }

    /// Whether `producer` is still the attempt this task is running.
    fn attempting(state: &State, producer: &str) -> bool {
        !state.stopping && matches!(&state.phase, Phase::Attempt { producer: p } if p == producer)
    }

    /// One attempt: connect, then write the handshake. On success the epoch is `Live`.
    async fn open_epoch(&self, producer: &str) -> Option<TcpStream> {
        {
            let mut state = self.shared.lock();
            if !Self::attempting(&state, producer) {
                return None;
            }
            state.stats.connection_attempts += 1;
        }
        let connect =
            tokio::time::timeout(CONNECT_ATTEMPT_TIMEOUT, TcpStream::connect(self.endpoint));
        let mut stream = match self.or_stop(connect).await? {
            Ok(Ok(stream)) => stream,
            // Nothing was written: the same ID is retried after the next delay.
            _ => {
                self.shared.lock().count_failure();
                return None;
            }
        };
        let _ = stream.set_nodelay(true);
        let Some(handshake) = encode_handshake(&self.entry_id, producer, self.secret.expose())
        else {
            // Bounded at construction for every 32-hex ID; kept terminal, never retried.
            self.shared.lock().latch_terminal(GapReason::LimitExceeded);
            return None;
        };
        {
            let mut state = self.shared.lock();
            if !Self::attempting(&state, producer) {
                return None;
            }
            // From the first handshake byte on, the receiver may have consumed this ID.
            state.spent.insert(producer.to_string());
            state.stats.handshakes_sent += 1;
        }
        let write = tokio::time::timeout(CONNECT_ATTEMPT_TIMEOUT, stream.write_all(&handshake));
        let written = matches!(self.or_stop(write).await?, Ok(Ok(())));
        let mut state = self.shared.lock();
        if !Self::attempting(&state, producer) {
            return None;
        }
        if !written {
            if !state.count_failure() {
                state.phase = Phase::Spent {
                    producer: producer.to_string(),
                };
            }
            return None;
        }
        state.phase = Phase::Live(Epoch {
            producer: producer.to_string(),
            claimed: false,
            next_seq: 0,
            last_seq: None,
        });
        state.epoch_since = Some(Instant::now());
        state.epoch_wrote = false;
        state.stats.epochs_ready += 1;
        Some(stream)
    }

    /// The one ordered writer of an epoch; `true` once the sink is done (its `Close` was
    /// written, or shutdown drained the queue). It also watches the read side: the receiver
    /// never writes, so any byte, EOF or error from it is the loss of this epoch.
    async fn run_epoch(&self, stream: TcpStream, producer: &str) -> bool {
        let (mut inbound, mut outbound) = stream.into_split();
        loop {
            match self.next_item(producer) {
                Next::Ended => return false,
                Next::Drained => {
                    // Shutdown with no `Close` queued: what was queued is written; the
                    // receiver sees EOF without `Close` and records unknown tail loss.
                    let _ = outbound.shutdown().await;
                    let mut state = self.shared.lock();
                    if state.outcome.is_none() {
                        state.outcome = Some(SenderOutcome::NotClosed);
                    }
                    return true;
                }
                Next::Wait => {
                    let mut probe = [0u8; 1];
                    tokio::select! {
                        _ = self.shared.wake.notified() => {}
                        _ = inbound.read(&mut probe) => {
                            self.lost(producer);
                            return false;
                        }
                    }
                }
                Next::Write(item) => {
                    match Self::write_item(
                        &self.shared,
                        &mut inbound,
                        &mut outbound,
                        &item,
                        producer,
                    )
                    .await
                    {
                        Some(true) => {}
                        Some(false) => {
                            self.lost(producer);
                            return false;
                        }
                        // Latched while the write was pending: stop at the latch.
                        None => {
                            self.shared.lock().discard_inflight();
                            return false;
                        }
                    }
                    if self.written(&item) {
                        let _ = outbound.shutdown().await;
                        return true;
                    }
                }
            }
        }
    }

    /// Releases a buffer the socket took in full; `true` when it was the `Close`, which ends
    /// the epoch and the sink cleanly.
    fn written(&self, item: &Item) -> bool {
        let mut state = self.shared.lock();
        state.inflight_bytes = 0;
        state.sync_queued();
        state.stats.items_written += 1;
        state.stats.bytes_written += item.bytes.len() as u64;
        state.epoch_wrote = true;
        if !item.close {
            return false;
        }
        if let Phase::Closing(epoch) = std::mem::replace(&mut state.phase, Phase::Closed) {
            state.outcome = Some(SenderOutcome::Closed {
                producer: epoch.producer,
                final_seq: epoch.last_seq,
            });
        }
        true
    }

    fn next_item(&self, producer: &str) -> Next {
        let mut state = self.shared.lock();
        if state.epoch(producer).is_none() {
            return Next::Ended;
        }
        match state.queue.pop_front() {
            Some(item) => {
                state.queued_bytes -= item.bytes.len();
                state.inflight_bytes = item.bytes.len();
                Next::Write(item)
            }
            None if state.stopping => Next::Drained,
            None => Next::Wait,
        }
    }

    /// Hands one item to the socket: `Some(true)` written, `Some(false)` the socket is lost,
    /// `None` the epoch was latched meanwhile. An abandoned partial item leaves with its
    /// socket, which the receiver reads as a torn tail, never as a complete item.
    async fn write_item(
        shared: &Shared,
        inbound: &mut OwnedReadHalf,
        outbound: &mut OwnedWriteHalf,
        item: &Item,
        producer: &str,
    ) -> Option<bool> {
        let write = outbound.write_all(&item.bytes);
        tokio::pin!(write);
        let mut probe = [0u8; 1];
        loop {
            tokio::select! {
                written = &mut write => return Some(written.is_ok()),
                _ = inbound.read(&mut probe) => return Some(false),
                _ = shared.wake.notified() => {
                    // Latched meanwhile: stop at the latch.
                    shared.lock().epoch(producer)?;
                }
            }
        }
    }

    /// The receiver closed or the socket failed under a live epoch: unknown loss, counted
    /// against the failure budget.
    fn lost(&self, producer: &str) {
        let mut state = self.shared.lock();
        // Only this writer ever sets in-flight bytes, so the failed item is released even when
        // a latch won the race and already ended the epoch (which leaves in-flight bytes alone).
        state.discard_inflight();
        let Some(epoch) = state.epoch(producer) else {
            return;
        };
        let claimed = epoch.claimed;
        // An epoch that took items and stayed up was a connection that worked: the row of
        // failures it ends starts again with this loss.
        if state.epoch_wrote
            && state
                .epoch_since
                .is_some_and(|since| since.elapsed() >= self.stable_after)
        {
            state.consecutive_failures = 0;
        }
        if claimed {
            state.latch_loss(GapReason::IngestLost);
        } else {
            // Never answered `Ready`: nothing was admitted, but the ID may be consumed.
            state.discard_queue();
            state.phase = Phase::Spent {
                producer: producer.to_string(),
            };
        }
        state.count_failure();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRODUCER: &str = "0123456789abcdef0123456789abcdef";

    /// The sink's own count of spent IDs is the one accurate producer budget (the proxy's startup
    /// proposes IDs before the emitter exists): a fresh proposal is admitted while fewer than
    /// `MAX_PRODUCERS` IDs are spent, and refused for good once that many are.
    #[test]
    fn the_producer_budget_stops_at_max_producers() {
        let id = |n: usize| format!("{n:032x}");
        let after_loss = |spent: usize| {
            let mut state = State::new(id(0));
            state.phase = Phase::Lost {
                reason: GapReason::QueueFull,
            };
            state.spent.extend((0..spent).map(id));
            state
        };

        let mut below = after_loss(MAX_PRODUCERS - 1);
        assert_eq!(below.begin(&id(MAX_PRODUCERS)), EpochAdmission::Pending);
        assert!(
            matches!(&below.phase, Phase::Attempt { producer } if *producer == id(MAX_PRODUCERS))
        );

        let mut at = after_loss(MAX_PRODUCERS);
        assert_eq!(
            at.begin(&id(MAX_PRODUCERS)),
            EpochAdmission::Disabled(GapReason::LimitExceeded)
        );
        assert!(matches!(
            at.phase,
            Phase::Disabled(GapReason::LimitExceeded)
        ));
        // Latched: no later proposal starts an attempt either.
        assert_eq!(
            at.begin(&id(MAX_PRODUCERS + 1)),
            EpochAdmission::Disabled(GapReason::LimitExceeded)
        );
    }

    #[test]
    fn handshake_is_ordered_compact_json_behind_a_length() {
        let framed = encode_handshake("entry", PRODUCER, "tok").unwrap();
        let body =
            br#"{"entry_id":"entry","producer":"0123456789abcdef0123456789abcdef","token":"tok"}"#;
        assert_eq!(&framed[..4], &(body.len() as u32).to_be_bytes());
        assert_eq!(&framed[4..], body);
    }

    #[test]
    fn handshake_refuses_unknown_fields_and_hides_the_token() {
        let extra = r#"{"entry_id":"e","producer":"p","token":"t","extra":1}"#;
        assert!(serde_json::from_str::<Handshake<'_>>(extra).is_err());
        let known = r#"{"entry_id":"e","producer":"p","token":"sentinel-token-value"}"#;
        let decoded: Handshake<'_> = serde_json::from_str(known).unwrap();
        assert_eq!(decoded.token, "sentinel-token-value");
        assert!(!format!("{decoded:?}").contains("sentinel-token-value"));
    }

    #[test]
    fn handshake_over_the_limit_is_refused_before_and_after_escaping() {
        let fixed = r#"{"entry_id":"e","producer":"","token":""}"#.len() + PRODUCER.len();
        let fits = MAX_HANDSHAKE_BYTES - fixed;
        let exact = encode_handshake("e", PRODUCER, &"x".repeat(fits)).unwrap();
        assert_eq!(exact.len(), 4 + MAX_HANDSHAKE_BYTES);
        assert!(encode_handshake("e", PRODUCER, &"x".repeat(fits + 1)).is_none());
        // Raw input far over the limit is refused before serialization.
        assert!(encode_handshake("e", PRODUCER, &"x".repeat(64 * 1024)).is_none());
        // Short raw input that JSON escaping pushes over the limit.
        assert!(encode_handshake("e", PRODUCER, &"\u{1}".repeat(300)).is_none());
    }
}
