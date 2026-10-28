//! The M2 recording proxy: a process-scoped, loopback HTTP CONNECT proxy. It relays harness
//! traffic unchanged and, for the capture paths of a matched host profile, submits decoded
//! body observations through a [`api::CaptureEmitter`]. It never changes a model base URL and
//! never installs global trust.
//!
//! [`api`] is the shared seam. This module root composes the leaves into one lifecycle:
//! [`start`] takes a [`ProxyConfig`] and returns a [`ProxyHandle`] once the listener, the
//! per-launch CA and the supplied sink are all ready; [`ProxyHandle::stop`] drains and closes
//! it inside one absolute deadline.
//!
//! # What one accepted connection goes through
//!
//! 1. [`connect::admit`] authenticates the CONNECT with the proxy credential, then
//!    either tunnels it opaquely (authentication hosts, hosts that are not approved, and an
//!    approved host whose ClientHello does not name it) or intercepts it: both TLS legs up,
//!    HTTP/1.1 or no ALPN.
//! 2. An interception takes a place in the capture table (the emitter's connection table,
//!    `MAX_OPEN_CONNECTIONS` places), a fresh physical connection ID (128 random bits, 32
//!    lowercase hex) and a `Connection Open { alpn, already_open: false }` control, then
//!    [`http::relay_http`] relays it with a [`api::RelayContext`] over the one shared
//!    emitter and the host's capture policy. When the table is full it is relayed as bytes
//!    instead, uncaptured and counted (`capture_table_full`).
//! 3. [`api::HttpOutcome::WebSocket`] records `UpgradedWebSocket` and moves the complete
//!    [`api::Upgrade`] (both prefixes and its `capture`, including the relay-only `None`) into
//!    [`websocket::relay_websocket`] unchanged. An HTTP relay that stops parsing goes on as
//!    bytes, uncaptured, and is counted when it does (`opaque_fallbacks`), however the
//!    connection then ends; a clean end is [`api::HttpOutcome::Opaque`].
//! 4. The connection ends with `Close { outcome }`: `Ok` is `Complete`; an `UnexpectedEof`,
//!    `ConnectionReset`, `ConnectionAborted` or `BrokenPipe` error, or cancellation, is
//!    `Interrupted`; any other error (and a panicking relay) is `TransportError`. The
//!    close is recorded by a drop guard after the relay future has been dropped and has
//!    ended its own open streams, so a cancelled connection still reports its lifecycle.
//!
//! Every observation this root makes is [`metadata::capture_time`] (one process clock).
//! Recovery re-observation belongs to the emitter: this root never re-emits anything.
//! Refused admissions and failures emit nothing, and an authentication host or a host that is
//! not approved stays opaque: a tunnel's bytes are never inspected. They are counted in
//! [`ProxyHealth`], where failed upstream connects are also coalesced into outage windows
//! ([`outage`]) that `goalcup start` reports at finish; the frozen v5 chain has no record for them. What the recording proxy relays but does not capture is a different matter: each such
//! loss is also submitted as a stream-zero `Loss` frame ([`api::CaptureEmitter::loss`]) at the
//! moment it happens, inside the connection task, so the producer's own stream carries it
//! before any `Close` can attest the stream complete: a fail-open tunnel when it is decided
//! (before its first byte), a full capture table before the uncaptured relay starts, an opaque
//! fallback when the relay stops parsing, and a connection cut while unsettled.
//!
//! # Bounds
//!
//! At most [`ProxyConfig::max_connections`] connection tasks (admission, tunnel or relay) are
//! alive at once, and that limit is at most [`MAX_CONNECTION_TASKS`]; an accept beyond it is
//! closed at once after a best-effort `503`, and counted. Capture never refuses a connection:
//! the capture table bounds only which intercepted connections are captured, so this proxy
//! never overflows the emitter's connection table.
//!
//! # Ownership and drop
//!
//! [`start`] consumes the pre-bound listener, the credential and the sink. The CA signing key
//! lives only inside `start`: every approved host's leaf is issued there and the key is
//! dropped before `start` returns. Only the CA **certificate** survives, as a new 0600 file in
//! [`ProxyConfig::ca_dir`]. The recording proxy never removes that file after a successful start;
//! its owner removes it with the scoped home (a relaunch into the same directory must, because
//! the file is created new). A failed start removes the file it created.
//!
//! Dropping a [`ProxyHandle`] without [`ProxyHandle::stop`] aborts the accept loop and every
//! connection task (their connections still record `Close { Interrupted }`) but never closes
//! the producer: that is abrupt task loss, which a receiver must read as unknown tail loss.
//! Only `stop` asks the sink to close the producer, and only once every task has ended.

pub mod api;
pub mod connect;
pub mod http;
pub mod ingest;
pub mod metadata;
pub(crate) mod outage;
pub mod preflight;
pub mod tls;
pub mod tunnels;
pub mod websocket;

use api::{CaptureEmitter, HttpOutcome, ProxyLoss, RelayContext, RelayPair};
use connect::{
    Admission, AdmissionConfig, ApprovedHost, ConnectFailureKind, FailureClass, ProxyCredential,
};
use goalcup_protocol::capture::{
    CaptureSink, ConnectionEvent, ConnectionOutcome, EpochAdmission, GapReason,
    MAX_OPEN_CONNECTIONS, MAX_PRODUCERS, SubmitResult, is_capture_id,
};
use rustls::pki_types::CertificateDer;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::{JoinError, JoinHandle, JoinSet};
use tokio::time::Instant;

/// The first pause between two polls of a `Pending` initial admission; it doubles up to
/// [`ADMISSION_POLL_MAX`]. Startup never busy-loops on the sink.
pub(crate) const ADMISSION_POLL_FIRST: Duration = Duration::from_millis(5);

/// The longest pause between two polls of a `Pending` initial admission.
pub(crate) const ADMISSION_POLL_MAX: Duration = Duration::from_millis(100);

/// The pause after a listener accept error before accepting again.
pub(crate) const ACCEPT_RETRY_PAUSE: Duration = Duration::from_millis(10);

/// The most connection tasks [`ProxyConfig::max_connections`] may allow: a guard on the file
/// descriptors this process holds, since a relayed connection holds two.
pub const MAX_CONNECTION_TASKS: u64 = 1024;

/// The most of the shutdown deadline kept back for cancelling what did not drain and asking
/// the sink to close the producer. The cut happens at `deadline - min(deadline / 4, this)`,
/// so both finish inside the one deadline rather than after a second timeout.
pub(crate) const SHUTDOWN_CANCEL_RESERVE: Duration = Duration::from_millis(250);

/// What a best-effort refusal of an accept beyond the connection limit writes before closing.
const OVER_CAPACITY_RESPONSE: &[u8] =
    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// Everything one proxy run needs, supplied by its owner.
///
/// No endpoint, ingest credential, daemon client or sink constructor enters here: the sink is
/// any [`CaptureSink`], already constructed by the caller for `producer`.
pub struct ProxyConfig {
    /// A listener the caller already bound to a loopback address (for example
    /// `127.0.0.1:0`). Connections wait in its backlog until the recording
    /// proxy is ready; none is accepted before the initial producer is admitted.
    pub listener: std::net::TcpListener,
    /// A private directory (the harness's scoped home) the per-launch CA certificate is
    /// written into as [`tls::CA_CERT_FILE_NAME`], created new with mode 0600.
    pub ca_dir: PathBuf,
    /// The intercepted hosts and their capture policies (the host rules). At least one.
    pub approved: Vec<ApprovedHost>,
    /// Hosts that always stay opaque tunnels, whatever else matches them.
    pub auth_hosts: Vec<String>,
    /// The secret a client must present on CONNECT. Derive the harness's proxy URL
    /// ([`ProxyCredential::proxy_url`]) before moving it in: the recording proxy never hands it
    /// back.
    pub credential: ProxyCredential,
    /// Trust anchors for upstream TLS: [`tls::UpstreamRoots::webpki`] in production.
    pub upstream_roots: tls::UpstreamRoots,
    /// The initial producer ID the sink was configured with: 32 lowercase hex.
    pub producer: String,
    /// Where capture frames go. Only synchronous, non-blocking `try_*` calls reach it.
    pub sink: Arc<dyn CaptureSink>,
    /// Bounds all of [`start`], including polling the sink until it admits a producer.
    pub startup_deadline: Duration,
    /// The one bound on [`ProxyHandle::stop`], from its call to its return.
    pub shutdown_deadline: Duration,
    /// Bounds everything before one connection's tunnel/intercept decision (see
    /// [`connect::DEFAULT_ADMISSION_TIMEOUT`]).
    pub admission_timeout: Duration,
    /// Bounds each upstream connect attempt (see
    /// [`connect::DEFAULT_CONNECT_ATTEMPT_TIMEOUT`]).
    pub connect_attempt_timeout: Duration,
    /// The most connection tasks alive at once, `1..=`[`MAX_CONNECTION_TASKS`].
    pub max_connections: usize,
}

impl std::fmt::Debug for ProxyConfig {
    /// The credential prints redacted (its own `Debug`); the sink is not formatted.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyConfig")
            .field("listener", &self.listener.local_addr().ok())
            .field("ca_dir", &self.ca_dir)
            .field("approved", &self.approved)
            .field("auth_hosts", &self.auth_hosts)
            .field("credential", &self.credential)
            .field("upstream_roots", &self.upstream_roots)
            .field("producer", &self.producer)
            .field("startup_deadline", &self.startup_deadline)
            .field("shutdown_deadline", &self.shutdown_deadline)
            .field("admission_timeout", &self.admission_timeout)
            .field("connect_attempt_timeout", &self.connect_attempt_timeout)
            .field("max_connections", &self.max_connections)
            .finish_non_exhaustive()
    }
}

/// Why [`start`] returned no proxy. Fixed text: no host, path, credential or sink detail.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StartError {
    #[error("the recording proxy listener is not bound to a loopback address")]
    NotLoopback,
    #[error("the recording proxy listener cannot be served ({0:?})")]
    Listener(io::ErrorKind),
    #[error("the initial producer ID is not 32 lowercase hex characters")]
    InvalidProducer,
    #[error("a deadline is zero or the connection limit is outside 1..=MAX_CONNECTION_TASKS")]
    InvalidLimits,
    #[error("the per-launch CA could not be prepared: {0}")]
    Ca(#[from] tls::CaError),
    #[error("the admission configuration was refused: {0}")]
    Admission(#[from] connect::AdmissionConfigError),
    #[error("the capture sink refused the initial producer ({0:?})")]
    CaptureDisabled(GapReason),
    #[error("the capture sink asked for more fresh producer IDs than one entry admits")]
    ProducerBudget,
    #[error("the startup deadline passed before the capture sink admitted a producer")]
    StartupDeadline,
}

/// Starts the recording proxy. Returns only once the listener is being served, the CA certificate
/// is written, and the sink admitted a producer `Ready`; no frame is submitted before that.
///
/// Initial admission polls [`CaptureSink::try_begin_epoch`] without blocking: `Pending`
/// keeps the same ID for the next poll, `RetryFresh` retires it and proposes a fresh one
/// (the sink keeps its own retry budget; at most `MAX_PRODUCERS` IDs are proposed here),
/// `Disabled` fails startup, and the startup deadline bounds it all. Only then is the one
/// [`CaptureEmitter`] built over the admitted ID; its constructor performs no I/O.
pub async fn start(config: ProxyConfig) -> Result<ProxyHandle, StartError> {
    let ProxyConfig {
        listener,
        ca_dir,
        approved,
        auth_hosts,
        credential,
        upstream_roots,
        producer,
        sink,
        startup_deadline,
        shutdown_deadline,
        admission_timeout,
        connect_attempt_timeout,
        max_connections,
    } = config;
    let deadline = Instant::now() + startup_deadline;
    if startup_deadline.is_zero()
        || shutdown_deadline.is_zero()
        || !(1..=MAX_CONNECTION_TASKS).contains(&(max_connections as u64))
    {
        return Err(StartError::InvalidLimits);
    }
    if !is_capture_id(&producer) {
        return Err(StartError::InvalidProducer);
    }
    let local_addr = listener
        .local_addr()
        .map_err(|e| StartError::Listener(e.kind()))?;
    if !local_addr.ip().is_loopback() {
        return Err(StartError::NotLoopback);
    }
    listener
        .set_nonblocking(true)
        .map_err(|e| StartError::Listener(e.kind()))?;
    let listener = TcpListener::from_std(listener).map_err(|e| StartError::Listener(e.kind()))?;

    // The CA key lives only here: every leaf is issued inside `AdmissionConfig::new`.
    let hosts: Vec<String> = approved.iter().map(|rule| rule.host.clone()).collect();
    let ca = tls::ProxyCa::generate(&hosts)?;
    let admission = AdmissionConfig::new(approved, auth_hosts, credential, &ca, &upstream_roots)?
        .with_timeouts(admission_timeout, connect_attempt_timeout);
    let ca_certificate_der = ca.certificate_der().clone();
    let ca_certificate = ca.write_certificate(&ca_dir)?;
    drop(ca);

    let producer = match admit_initial_producer(sink.as_ref(), producer, deadline).await {
        Ok(producer) => producer,
        Err(refused) => {
            // Nothing will ever serve this certificate: leave the directory as it was.
            let _ = std::fs::remove_file(&ca_certificate);
            return Err(refused);
        }
    };
    let counters = Arc::new(Counters::default());
    bump(&counters.producers_admitted);
    let observed: Arc<dyn CaptureSink> = Arc::new(ObservedSink {
        inner: sink,
        counters: Arc::clone(&counters),
    });
    let emitter = Arc::new(CaptureEmitter::new(producer.clone(), observed));
    let shared = Arc::new(Shared {
        admission,
        emitter,
        counters,
        outages: outage::OutageTracker::default(),
        tunnels: tunnels::TunnelNotes::default(),
    });
    let (stop_tx, stop_rx) = oneshot::channel();
    let accept = tokio::spawn(accept_loop(
        listener,
        Arc::clone(&shared),
        max_connections as u64,
        stop_rx,
    ));
    Ok(ProxyHandle {
        local_addr,
        ca_certificate,
        ca_certificate_der,
        producer,
        shutdown_deadline,
        shared,
        stop: Some(stop_tx),
        accept: Some(accept),
    })
}

/// Drives the initial `EpochAdmission` to `Ready` within `deadline`; returns the admitted ID.
async fn admit_initial_producer(
    sink: &dyn CaptureSink,
    configured: String,
    deadline: Instant,
) -> Result<String, StartError> {
    let mut candidate = configured;
    let mut proposed = 1usize;
    let mut pause = ADMISSION_POLL_FIRST;
    loop {
        match sink.try_begin_epoch(&candidate) {
            EpochAdmission::Ready => return Ok(candidate),
            EpochAdmission::Disabled(reason) => return Err(StartError::CaptureDisabled(reason)),
            EpochAdmission::Pending => {}
            EpochAdmission::RetryFresh => {
                // The ID may have reached the receiver: it is never proposed again.
                if proposed >= MAX_PRODUCERS {
                    return Err(StartError::ProducerBudget);
                }
                proposed += 1;
                candidate = fresh_id();
            }
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(StartError::StartupDeadline);
        }
        tokio::time::sleep_until((now + pause).min(deadline)).await;
        pause = (pause * 2).min(ADMISSION_POLL_MAX);
    }
}

/// A fresh 128-bit identifier as 32 lowercase hex (the producer and connection ID form).
fn fresh_id() -> String {
    format!("{:032x}", rand::random::<u128>())
}

/// A running proxy. See the module documentation for drop behavior.
pub struct ProxyHandle {
    local_addr: SocketAddr,
    ca_certificate: PathBuf,
    ca_certificate_der: CertificateDer<'static>,
    producer: String,
    shutdown_deadline: Duration,
    shared: Arc<Shared>,
    stop: Option<oneshot::Sender<()>>,
    accept: Option<JoinHandle<JoinSet<()>>>,
}

impl std::fmt::Debug for ProxyHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyHandle")
            .field("local_addr", &self.local_addr)
            .field("ca_certificate", &self.ca_certificate)
            .field("producer", &self.producer)
            .field("health", &self.health())
            .finish_non_exhaustive()
    }
}

impl ProxyHandle {
    /// The address the recording proxy serves: the pre-bound listener's.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The per-launch CA certificate file the harness is pointed at (`CODEX_CA_CERTIFICATE`,
    /// `NODE_EXTRA_CA_CERTS`). Certificate only; the key never left memory and is gone.
    pub fn ca_certificate(&self) -> &Path {
        &self.ca_certificate
    }

    /// The same CA certificate, DER-encoded.
    pub fn ca_certificate_der(&self) -> &CertificateDer<'static> {
        &self.ca_certificate_der
    }

    /// The producer ID the sink admitted at startup: the configured one, or the fresh ID a
    /// `RetryFresh` led to. A later capture recovery rotates to IDs the sink itself admits;
    /// this stays the initial one.
    pub fn producer(&self) -> &str {
        &self.producer
    }

    /// The recording proxy's count of model requests ([`ProxyHealth::model_requests`]), shared: it
    /// keeps its last value when the recording proxy stops.
    pub fn model_requests(&self) -> Arc<std::sync::atomic::AtomicU64> {
        self.shared.emitter.model_requests_counter()
    }

    /// The recording proxy's count of recorded model requests
    /// ([`api::CaptureEmitter::recorded_requests`]), shared as [`ProxyHandle::model_requests`] is.
    pub fn recorded_requests(&self) -> Arc<std::sync::atomic::AtomicU64> {
        self.shared.emitter.recorded_requests_counter()
    }

    /// The recording proxy's count of requests on a path with a protocol hint
    /// ([`api::CaptureEmitter::generation_requests`]), shared as [`ProxyHandle::model_requests`]
    /// is: the model calls, not a model list or a probe under the same base path.
    pub fn generation_requests(&self) -> Arc<std::sync::atomic::AtomicU64> {
        self.shared.emitter.generation_requests_counter()
    }

    /// The recording proxy's count of harness connections whose TLS handshake failed
    /// ([`AdmissionFailureCounts::client_tls`]), shared as [`ProxyHandle::model_requests`] is. A
    /// harness that does not trust the recording certificate shows here.
    pub fn client_tls_failures(&self) -> Arc<std::sync::atomic::AtomicU64> {
        Arc::clone(&self.shared.counters.admission_client_tls)
    }

    /// A snapshot of the counters. It never waits on a connection or the sink.
    pub fn health(&self) -> ProxyHealth {
        self.shared.health()
    }

    /// Stops the recording proxy inside the one [`ProxyConfig::shutdown_deadline`], measured from
    /// now.
    ///
    /// 1. Stop accepting; the listener is closed.
    /// 2. Let every connection end on its own until the cut, `deadline - min(deadline / 4,
    ///    SHUTDOWN_CANCEL_RESERVE)`.
    /// 3. At the cut, cancel whatever is left: intercepted connections end their open streams
    ///    `Interrupted` and record `Close { Interrupted }`; connections still in admission are
    ///    counted (`cut_unsettled`), since no capture record exists for them. An open tunnel
    ///    is cut without one: it was decided, and a fail-open one was chained then.
    /// 4. Only when every task has ended by the deadline, ask the emitter to close the
    ///    producer ([`CaptureEmitter::try_close`]: the sink's `try_close_epoch` with the exact
    ///    last admitted sequence). `Queued` is volatile acceptance, never a claim that the
    ///    receiver validated or persisted it. A task still alive at the deadline means open
    ///    streams cannot be ruled out, so the close is not attempted: no drain is invented.
    ///
    /// The bound assumes the sink keeps the `CaptureSink` promise that `try_*` never blocks. A sink
    /// that blocks inside a call holds a runtime worker (and the emitter's lock) and can starve
    /// that runtime's own timers; `stop` still returns by the deadline, reporting the stuck task as
    /// `unterminated` with the close `NotAttempted`, as long as the runtime polling `stop` can
    /// still run timers.
    pub async fn stop(self) -> ProxyStopReport {
        let bound = self.shutdown_deadline;
        self.stop_within(bound).await
    }

    /// [`ProxyHandle::stop`] inside `bound` instead, never longer than the configured
    /// [`ProxyConfig::shutdown_deadline`]. For a stop while the harness still runs (the run's
    /// deadline): its open connections do not end on their own then, so waiting the whole
    /// shutdown deadline for them only delays the stop.
    pub async fn stop_within(mut self, bound: Duration) -> ProxyStopReport {
        let bound = bound.min(self.shutdown_deadline);
        let deadline = Instant::now() + bound;
        let cut = deadline - (bound / 4).min(SHUTDOWN_CANCEL_RESERVE);
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let counters = &self.shared.counters;
        let mut tasks = match self.accept.take() {
            Some(mut accept) => match tokio::time::timeout_at(cut, &mut accept).await {
                Ok(Ok(tasks)) => Some(tasks),
                // The loop did not hand its tasks back; aborting it drops (and so aborts)
                // them, but when they end is no longer observable here.
                Ok(Err(_)) | Err(_) => {
                    accept.abort();
                    None
                }
            },
            None => None,
        };
        let mut drained = false;
        let mut unterminated = 0;
        if let Some(tasks) = tasks.as_mut() {
            drained = tokio::time::timeout_at(cut, reap_all(tasks, counters))
                .await
                .is_ok();
            if !drained {
                tasks.abort_all();
                if tokio::time::timeout_at(deadline, reap_all(tasks, counters))
                    .await
                    .is_err()
                {
                    unterminated = tasks.len();
                }
            }
        }
        let producer_close = if tasks.is_some() && unterminated == 0 {
            match self.shared.emitter.try_close(metadata::capture_time()) {
                SubmitResult::Queued => ProducerClose::Queued,
                SubmitResult::Disabled(reason) => ProducerClose::Refused(reason),
            }
        } else {
            ProducerClose::NotAttempted
        };
        ProxyStopReport {
            drained,
            accept_loop_lost: tasks.is_none(),
            unterminated,
            producer_close,
            health: self.shared.health(),
        }
    }
}

impl Drop for ProxyHandle {
    fn drop(&mut self) {
        // Abrupt: every task is aborted with the accept loop's JoinSet, and the producer
        // stays open, which a receiver reads as unknown tail loss.
        if let Some(accept) = self.accept.take() {
            accept.abort();
        }
    }
}

/// How the producer close went at [`ProxyHandle::stop`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProducerClose {
    /// The sink queued `Close` with the exact final sequence and no open payload stream.
    /// Volatile acceptance only: whether it reached the retained chain is the receiver's
    /// (or direct sink's) validated `ProducerClosed`, not this answer.
    Queued,
    /// The close was refused: the current epoch had failed or was disabled (capture loss,
    /// reason given), or a payload stream was still open (`ShutdownDeadline`).
    Refused(GapReason),
    /// Not attempted: a task was still alive at the deadline, or the accept loop was lost,
    /// so open streams could not be ruled out.
    NotAttempted,
}

/// What [`ProxyHandle::stop`] observed. Three independent questions, answered separately:
/// did the shutdown drain normally ([`Self::is_clean_drain`]), did this proxy see capture
/// loss ([`Self::capture_loss`]), and did connections or the recording proxy itself fail
/// ([`Self::transport_failures`]). A run is fully clean only when all three say so. The
/// sink/receiver remains the authority on sticky gaps: the retained chain holds each loss.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyStopReport {
    /// Every connection task ended on its own before the cut.
    pub drained: bool,
    /// The accept loop did not hand its tasks back, so their end was not observed.
    pub accept_loop_lost: bool,
    /// Connection tasks still alive at the deadline even after cancellation.
    pub unterminated: usize,
    pub producer_close: ProducerClose,
    /// The final counters, including what shutdown cut (`cut_intercepted`, `cut_unsettled`).
    pub health: ProxyHealth,
}

impl ProxyStopReport {
    /// The drain itself: every connection ended on its own before the cut (nothing was cut,
    /// so no tail is incomplete because of shutdown), no task outlived the deadline, and the
    /// sink queued the producer close. It says nothing about capture loss or failed
    /// connections before shutdown.
    #[cfg(any(test, feature = "test-support"))]
    pub fn is_clean_drain(&self) -> bool {
        self.drained
            && !self.accept_loop_lost
            && self.unterminated == 0
            && self.producer_close == ProducerClose::Queued
    }

    /// Capture this proxy knows it lost, or cannot rule out having lost. Known: a refused
    /// producer close, a frame or control the sink or emitter refused (recovered or not), an
    /// approved host relayed as an opaque tunnel (fail-open, not captured), an intercepted
    /// connection relayed uncaptured because the capture table was full, or an HTTP relay
    /// that fell back to opaque bytes. Possible: connections cut at shutdown while still in
    /// admission (`cut_unsettled`: one may have become a fail-open tunnel), tasks that outlived the deadline
    /// (`unterminated`), and a lost accept loop. Since H-16 every loss but the last two is also
    /// a `Loss` frame in the producer's stream (see the module documentation); the last two leave
    /// the producer unclosed. The launcher's seal request still carries these counters as a
    /// confirmatory copy, which the entry owner records as stream-less gaps.
    #[cfg(any(test, feature = "test-support"))]
    pub fn capture_loss(&self) -> bool {
        matches!(self.producer_close, ProducerClose::Refused(_))
            || self.health.control_refusals > 0
            || self.health.sink_refusals > 0
            || self.health.tunneled_fail_open > 0
            || self.health.capture_table_full > 0
            || self.health.opaque_fallbacks > 0
            || self.health.cut_unsettled > 0
            || self.unterminated > 0
            || self.accept_loop_lost
    }

    /// Connections or proxy work that failed: relays closed `TransportError`, panicked tasks,
    /// failed or refused admissions (a `407` included), accept errors, accepts refused over
    /// capacity, and tasks that never ended. `Interrupted` closes are not counted: a peer
    /// that closes without a TLS `close_notify` or a WebSocket closing handshake ends that
    /// way routinely; they stay visible in [`ProxyHealth`].
    #[cfg(any(test, feature = "test-support"))]
    pub fn transport_failures(&self) -> u64 {
        let h = &self.health;
        h.closed_transport_error
            + h.task_panics
            + h.admission_failed
            + h.admission_refused_auth
            + h.accept_errors
            + h.refused_over_capacity
            + self.unterminated as u64
    }
}

/// Fixed, non-secret admission failure classes. Authentication refusals are counted separately.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdmissionFailureCounts {
    /// Invalid CONNECT request head.
    pub malformed: u64,
    /// CONNECT request head exceeded its byte bound.
    pub head_too_large: u64,
    /// Authenticated request used another method.
    pub not_connect: u64,
    /// Overall admission deadline elapsed; the inner phase is not retained.
    pub timeout: u64,
    /// Harness-side I/O failed before interception.
    pub client_io: u64,
    /// Upstream TCP connection failed.
    pub upstream_connect: u64,
    /// Upstream DNS lookup failed.
    pub upstream_connect_dns: u64,
    /// Upstream DNS lookup returned no addresses.
    pub upstream_connect_no_addresses: u64,
    /// Last attempted upstream TCP address refused the connection.
    pub upstream_connect_tcp_last_refused: u64,
    /// Last attempted upstream TCP address timed out.
    pub upstream_connect_tcp_last_timeout: u64,
    /// Last attempted upstream TCP address denied the connection.
    pub upstream_connect_tcp_last_permission_denied: u64,
    /// Last attempted upstream TCP address failed for another I/O reason.
    pub upstream_connect_tcp_last_other: u64,
    /// Upstream TLS handshake or certificate validation failed.
    pub upstream_tls: u64,
    /// Harness-side TLS handshake failed.
    pub client_tls: u64,
    /// Either leg rejected or negotiated incompatible ALPN.
    pub alpn_incompatible: u64,
}

/// Counters of one proxy run. The gauges (`active`, `unsettled`, `intercepted_open`) are
/// current; everything else counts since start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProxyHealth {
    /// Connections accepted from the listener, refused ones included.
    pub accepted: u64,
    /// Accepts closed at once because `max_connections` tasks were alive.
    pub refused_over_capacity: u64,
    /// Listener accept errors, each followed by [`ACCEPT_RETRY_PAUSE`].
    pub accept_errors: u64,
    /// Connection tasks alive now.
    pub active: u64,
    /// Of `active`: connections in admission or in an opaque tunnel.
    pub unsettled: u64,
    /// Of `active`: intercepted connections holding a place in the capture table (at most
    /// `MAX_OPEN_CONNECTIONS`).
    pub intercepted_open: u64,
    /// Connections intercepted since start.
    pub intercepted: u64,
    /// Completed tunnels to authentication hosts.
    pub tunneled_auth_host: u64,
    /// Completed tunnels to hosts that are not approved.
    pub tunneled_not_approved: u64,
    /// Tunnels to an **approved** host whose ClientHello did not name it (absent or different
    /// SNI, not TLS, unparsable): relayed unchanged and not captured. Counted when decided, so a
    /// tunnel still open at stop is counted too. A client that closes before completing its
    /// ClientHello is no tunnel: it is an admission failure (`client_io`), and no loss.
    pub tunneled_fail_open: u64,
    /// Of `tunneled_fail_open`, by reason: the ClientHello was malformed or over its bound.
    pub fail_open_unparsed: u64,
    /// Of `tunneled_fail_open`: the ClientHello carried no server name.
    pub fail_open_no_sni: u64,
    /// Of `tunneled_fail_open`: the ClientHello named another server, or a tunnel to another
    /// authority named an approved host.
    pub fail_open_sni_mismatch: u64,
    /// Of `tunneled_fail_open`: the first bytes were not a TLS handshake record.
    pub fail_open_not_tls: u64,
    /// Intercepted connections relayed as bytes, uncaptured, because the capture table was
    /// full.
    pub capture_table_full: u64,
    /// Intercepted connections whose HTTP relay stopped parsing (a head over its bound or
    /// unparsable, framing it cannot follow, a `101` it does not hand to the WebSocket relay)
    /// and relayed the rest as bytes: nothing after that point was captured. Counted when the
    /// relay falls back, however the connection then ends.
    pub opaque_fallbacks: u64,
    /// Admissions refused `407` for a missing, repeated or wrong proxy credential.
    pub admission_refused_auth: u64,
    /// Every other admission failure (malformed, timeout, upstream or client TLS, ALPN ...).
    pub admission_failed: u64,
    /// Fixed non-secret classes of `admission_failed`. After stop, the top-level classes sum to
    /// that aggregate; the six connect subtypes partition `upstream_connect` separately. TCP
    /// subtypes describe only the last address attempted.
    pub admission_failures: AdmissionFailureCounts,
    /// Intercepted connections closed `Complete`.
    pub closed_complete: u64,
    /// Intercepted connections closed `Interrupted` (shutdown cuts included).
    pub closed_interrupted: u64,
    /// Intercepted connections closed `TransportError` (panics included).
    pub closed_transport_error: u64,
    /// Intercepted connections cancelled by shutdown or by dropping the handle.
    pub cut_intercepted: u64,
    /// Connections cancelled while still in admission, before a tunnel or interception was
    /// decided. A tunnel cut while open is not counted: a fail-open one was chained as a loss
    /// when it was decided, and any other is opaque by design.
    pub cut_unsettled: u64,
    /// Connection controls (`Open`, `UpgradedWebSocket`, `Close`) the emitter refused.
    pub control_refusals: u64,
    /// Frames and producer closes the sink itself refused (`Disabled`), and the refusals the
    /// emitter made on its own, which the sink never saw (an exhausted protocol limit, metadata
    /// outside the protocol limits): capture loss, even when a later epoch recovered.
    pub sink_refusals: u64,
    /// Producer epochs the sink admitted `Ready`: the initial one plus every recovery.
    pub producers_admitted: u64,
    /// Connection tasks that panicked.
    pub task_panics: u64,
    /// Requests the harness made of a captured host: each request relayed on an intercepted
    /// connection, and each client-to-server WebSocket message on one. The run's heartbeat
    /// sends it, cumulative, so the server can tell a harness that has gone quiet.
    pub model_requests: u64,
    /// Upstream outage windows since start: stretches of consecutive admissions whose upstream
    /// connect failed after every retry, each ended by the next one that reached an upstream
    /// ([`outage`]). Zero on a run whose network never failed it.
    pub upstream_outages: u64,
    /// The most recent of those windows (at most [`outage::MAX_KEPT_OUTAGE_WINDOWS`]), oldest
    /// first; one still open has no `recovered` time.
    pub outage_windows: Vec<outage::OutageWindow>,
    /// Every tunnel that ended, or was cut open at the stop, in the order they did (at most
    /// [`tunnels::MAX_TUNNEL_NOTES`]): the sealed-connection list `goalcup start` writes at
    /// finish (D74).
    pub tunnels: Vec<tunnels::TunnelNote>,
    /// Tunnels past that bound, counted only.
    pub tunnels_dropped: u64,
}

#[derive(Default)]
struct Counters {
    accepted: AtomicU64,
    refused_over_capacity: AtomicU64,
    accept_errors: AtomicU64,
    active: AtomicU64,
    unsettled: AtomicU64,
    intercepted_open: AtomicU64,
    intercepted: AtomicU64,
    tunneled_auth_host: AtomicU64,
    tunneled_not_approved: AtomicU64,
    tunneled_fail_open: AtomicU64,
    fail_open_unparsed: AtomicU64,
    fail_open_no_sni: AtomicU64,
    fail_open_sni_mismatch: AtomicU64,
    fail_open_not_tls: AtomicU64,
    capture_table_full: AtomicU64,
    admission_refused_auth: AtomicU64,
    admission_failed: AtomicU64,
    admission_malformed: AtomicU64,
    admission_head_too_large: AtomicU64,
    admission_not_connect: AtomicU64,
    admission_timeout: AtomicU64,
    admission_client_io: AtomicU64,
    admission_upstream_connect: AtomicU64,
    admission_upstream_connect_dns: AtomicU64,
    admission_upstream_connect_no_addresses: AtomicU64,
    admission_upstream_connect_tcp_last_refused: AtomicU64,
    admission_upstream_connect_tcp_last_timeout: AtomicU64,
    admission_upstream_connect_tcp_last_permission_denied: AtomicU64,
    admission_upstream_connect_tcp_last_other: AtomicU64,
    admission_upstream_tls: AtomicU64,
    admission_client_tls: Arc<AtomicU64>,
    admission_alpn_incompatible: AtomicU64,
    closed_complete: AtomicU64,
    closed_interrupted: AtomicU64,
    closed_transport_error: AtomicU64,
    cut_intercepted: AtomicU64,
    cut_unsettled: AtomicU64,
    control_refusals: AtomicU64,
    sink_refusals: AtomicU64,
    producers_admitted: AtomicU64,
    task_panics: AtomicU64,
}

fn bump(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::SeqCst);
}

fn unbump(counter: &AtomicU64) {
    counter.fetch_sub(1, Ordering::SeqCst);
}

impl Counters {
    fn snapshot(&self) -> ProxyHealth {
        let read = |counter: &AtomicU64| counter.load(Ordering::SeqCst);
        ProxyHealth {
            accepted: read(&self.accepted),
            refused_over_capacity: read(&self.refused_over_capacity),
            accept_errors: read(&self.accept_errors),
            active: read(&self.active),
            unsettled: read(&self.unsettled),
            intercepted_open: read(&self.intercepted_open),
            intercepted: read(&self.intercepted),
            tunneled_auth_host: read(&self.tunneled_auth_host),
            tunneled_not_approved: read(&self.tunneled_not_approved),
            tunneled_fail_open: read(&self.tunneled_fail_open),
            fail_open_unparsed: read(&self.fail_open_unparsed),
            fail_open_no_sni: read(&self.fail_open_no_sni),
            fail_open_sni_mismatch: read(&self.fail_open_sni_mismatch),
            fail_open_not_tls: read(&self.fail_open_not_tls),
            capture_table_full: read(&self.capture_table_full),
            // Counted on the emitter by the relays; `Shared::health` fills it in.
            opaque_fallbacks: 0,
            admission_refused_auth: read(&self.admission_refused_auth),
            admission_failed: read(&self.admission_failed),
            admission_failures: AdmissionFailureCounts {
                malformed: read(&self.admission_malformed),
                head_too_large: read(&self.admission_head_too_large),
                not_connect: read(&self.admission_not_connect),
                timeout: read(&self.admission_timeout),
                client_io: read(&self.admission_client_io),
                upstream_connect: read(&self.admission_upstream_connect),
                upstream_connect_dns: read(&self.admission_upstream_connect_dns),
                upstream_connect_no_addresses: read(&self.admission_upstream_connect_no_addresses),
                upstream_connect_tcp_last_refused: read(
                    &self.admission_upstream_connect_tcp_last_refused,
                ),
                upstream_connect_tcp_last_timeout: read(
                    &self.admission_upstream_connect_tcp_last_timeout,
                ),
                upstream_connect_tcp_last_permission_denied: read(
                    &self.admission_upstream_connect_tcp_last_permission_denied,
                ),
                upstream_connect_tcp_last_other: read(
                    &self.admission_upstream_connect_tcp_last_other,
                ),
                upstream_tls: read(&self.admission_upstream_tls),
                client_tls: read(&self.admission_client_tls),
                alpn_incompatible: read(&self.admission_alpn_incompatible),
            },
            closed_complete: read(&self.closed_complete),
            closed_interrupted: read(&self.closed_interrupted),
            closed_transport_error: read(&self.closed_transport_error),
            cut_intercepted: read(&self.cut_intercepted),
            cut_unsettled: read(&self.cut_unsettled),
            control_refusals: read(&self.control_refusals),
            sink_refusals: read(&self.sink_refusals),
            producers_admitted: read(&self.producers_admitted),
            task_panics: read(&self.task_panics),
            // Counted on the emitter by the relays; `Shared::health` fills it in.
            model_requests: 0,
            // Kept by `Shared::outages` and `Shared::tunnels`; `Shared::health` fills them in.
            upstream_outages: 0,
            outage_windows: Vec::new(),
            tunnels: Vec::new(),
            tunnels_dropped: 0,
        }
    }

    fn sink_answer(&self, answer: SubmitResult) {
        if matches!(answer, SubmitResult::Disabled(_)) {
            bump(&self.sink_refusals);
        }
    }

    fn control(&self, answer: SubmitResult) {
        if matches!(answer, SubmitResult::Disabled(_)) {
            bump(&self.control_refusals);
        }
    }

    fn reaped(&self, joined: Result<(), JoinError>) {
        if joined.is_err_and(|e| e.is_panic()) {
            bump(&self.task_panics);
        }
    }
}

/// What every connection task shares: the immutable admission rules, the one emitter, the
/// counters, the upstream outage windows and the tunnel notes.
struct Shared {
    admission: AdmissionConfig,
    emitter: Arc<CaptureEmitter>,
    counters: Arc<Counters>,
    outages: outage::OutageTracker,
    tunnels: tunnels::TunnelNotes,
}

impl Shared {
    /// The counters, with the emitter's own refusals among the sink refusals and the
    /// fallbacks its relays counted on it.
    fn health(&self) -> ProxyHealth {
        let mut health = self.counters.snapshot();
        health.sink_refusals += self.emitter.local_refusals();
        health.opaque_fallbacks = self.emitter.opaque_fallbacks();
        health.model_requests = self.emitter.model_requests();
        (health.upstream_outages, health.outage_windows) = self.outages.snapshot();
        (health.tunnels, health.tunnels_dropped) = self.tunnels.snapshot();
        health
    }
}

/// The supplied sink as the emitter sees it: every call passes straight through, and its
/// answers are counted, so a capture loss the emitter later recovered from is still visible
/// in [`ProxyHealth`] (the retained chain holds its sticky gap).
struct ObservedSink {
    inner: Arc<dyn CaptureSink>,
    counters: Arc<Counters>,
}

impl CaptureSink for ObservedSink {
    fn try_submit(&self, frame: goalcup_protocol::capture::CaptureFrame) -> SubmitResult {
        let answer = self.inner.try_submit(frame);
        self.counters.sink_answer(answer);
        answer
    }

    fn try_begin_epoch(&self, producer: &str) -> EpochAdmission {
        let answer = self.inner.try_begin_epoch(producer);
        if answer == EpochAdmission::Ready {
            bump(&self.counters.producers_admitted);
        }
        answer
    }

    fn try_close_epoch(&self, producer: &str, final_seq: Option<u64>) -> SubmitResult {
        let answer = self.inner.try_close_epoch(producer, final_seq);
        self.counters.sink_answer(answer);
        answer
    }
}

async fn accept_loop(
    listener: TcpListener,
    shared: Arc<Shared>,
    max_connections: u64,
    mut stop: oneshot::Receiver<()>,
) -> JoinSet<()> {
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = &mut stop => break,
            Some(joined) = tasks.join_next(), if !tasks.is_empty() => {
                shared.counters.reaped(joined);
            }
            accepted = listener.accept() => match accepted {
                Ok((tcp, _peer)) => {
                    bump(&shared.counters.accepted);
                    if shared.counters.active.load(Ordering::SeqCst) >= max_connections {
                        bump(&shared.counters.refused_over_capacity);
                        let _ = tcp.try_write(OVER_CAPACITY_RESPONSE);
                        continue;
                    }
                    tasks.spawn(serve_connection(tcp, Active::enter(Arc::clone(&shared))));
                }
                Err(_) => {
                    bump(&shared.counters.accept_errors);
                    tokio::time::sleep(ACCEPT_RETRY_PAUSE).await;
                }
            },
        }
    }
    // The listener is dropped here: nothing more is accepted.
    tasks
}

async fn reap_all(tasks: &mut JoinSet<()>, counters: &Counters) {
    while let Some(joined) = tasks.join_next().await {
        counters.reaped(joined);
    }
}

/// One live connection task's share of the recording proxy; counted in `active` for as long as the
/// task's future exists, however it ends (completion, cancellation or panic).
struct Active(Arc<Shared>);

impl Active {
    fn enter(shared: Arc<Shared>) -> Self {
        bump(&shared.counters.active);
        Self(shared)
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        unbump(&self.0.counters.active);
    }
}

async fn serve_connection(tcp: TcpStream, active: Active) {
    let shared = &*active.0;
    let counters = &shared.counters;
    let _ = tcp.set_nodelay(true);
    let unsettled = Unsettled::enter(shared);
    let open_tunnel = OpenTunnel::new(&shared.tunnels);
    // A fail-open tunnel is chained as a loss when it is decided, before its first byte is
    // relayed: the tunnel runs inside `admit`, so a task that is cut or panics mid-relay
    // never reaches the counters below. Every decided tunnel settles there too: a cut one is
    // then either that loss, already chained, or opaque by design, which is no loss.
    let admission = connect::admit_observed(tcp, &shared.admission, |start| {
        let reason = start.reason;
        unsettled.settle();
        open_tunnel.decided(start);
        // A tunnel is decided only once its upstream is connected; every reason but a host
        // that is not approved is a provider host's, or (a `SniMismatch` to another authority)
        // names one in its ClientHello.
        if reason != connect::TunnelReason::NotApproved {
            shared.outages.success();
        }
        if let Some(by_reason) = fail_open_counter(counters, reason) {
            bump(&counters.tunneled_fail_open);
            bump(by_reason);
            let _ = shared
                .emitter
                .proxy_loss(metadata::capture_time(), ProxyLoss::FailOpenTunnel);
        }
    })
    .await;
    unsettled.settle();
    drop(unsettled);
    record_upstream_reach(shared, &admission);
    match admission {
        Admission::Failed(failure) => {
            let class = match failure.class {
                FailureClass::ProxyAuthRequired => {
                    bump(&counters.admission_refused_auth);
                    None
                }
                FailureClass::Malformed => Some(&counters.admission_malformed),
                FailureClass::HeadTooLarge => Some(&counters.admission_head_too_large),
                FailureClass::NotConnect => Some(&counters.admission_not_connect),
                FailureClass::Timeout => Some(&counters.admission_timeout),
                FailureClass::ClientIo => Some(&counters.admission_client_io),
                FailureClass::UpstreamConnect(kind) => {
                    let subtype = match kind {
                        ConnectFailureKind::Dns => &counters.admission_upstream_connect_dns,
                        ConnectFailureKind::NoAddresses => {
                            &counters.admission_upstream_connect_no_addresses
                        }
                        ConnectFailureKind::TcpRefused => {
                            &counters.admission_upstream_connect_tcp_last_refused
                        }
                        ConnectFailureKind::TcpTimeout => {
                            &counters.admission_upstream_connect_tcp_last_timeout
                        }
                        ConnectFailureKind::TcpPermissionDenied => {
                            &counters.admission_upstream_connect_tcp_last_permission_denied
                        }
                        ConnectFailureKind::TcpOther => {
                            &counters.admission_upstream_connect_tcp_last_other
                        }
                    };
                    bump(subtype);
                    Some(&counters.admission_upstream_connect)
                }
                FailureClass::UpstreamTls => Some(&counters.admission_upstream_tls),
                FailureClass::ClientTls => Some(&*counters.admission_client_tls),
                FailureClass::AlpnIncompatible => Some(&counters.admission_alpn_incompatible),
            };
            if let Some(class) = class {
                bump(&counters.admission_failed);
                bump(class);
            }
        }
        Admission::Tunneled(tunnel) => match open_tunnel.ended(&tunnel) {
            connect::TunnelReason::AuthHost => bump(&counters.tunneled_auth_host),
            connect::TunnelReason::NotApproved => bump(&counters.tunneled_not_approved),
            // Counted at the decision, with its loss.
            connect::TunnelReason::SniAbsent
            | connect::TunnelReason::SniMismatch
            | connect::TunnelReason::NotTls
            | connect::TunnelReason::ClientHelloUnparsed => {}
        },
        Admission::Intercepted(intercepted) => relay_intercepted(intercepted, shared).await,
    }
}

/// What a settled admission says about reaching a provider host (approved or authentication),
/// for the outage windows. A tunnel is recorded when it is decided (admission returns only when
/// it ends); an interception, and a TLS or ALPN failure that came after the TCP connect, reached the
/// upstream; a failed connect did not. A host that is not approved is the agent's own traffic and
/// says nothing about the provider; nor does a refusal, a timeout or a client that went away.
fn record_upstream_reach(shared: &Shared, admission: &Admission) {
    let reached = match admission {
        Admission::Intercepted(_) => true,
        Admission::Failed(failure)
            if !failure
                .host
                .as_deref()
                .is_some_and(|host| shared.admission.is_provider_host(host)) =>
        {
            return;
        }
        Admission::Failed(failure) => match failure.class {
            FailureClass::UpstreamConnect(_) => false,
            FailureClass::UpstreamTls
            | FailureClass::ClientTls
            | FailureClass::AlpnIncompatible => true,
            _ => return,
        },
        Admission::Tunneled(_) => return,
    };
    if reached {
        shared.outages.success();
    } else {
        shared.outages.failure();
    }
}

/// An approved host's traffic relayed uncaptured: a tunnel to an approved host that was not
/// intercepted, or a tunnel to any other authority whose ClientHello names an approved host
/// (`SniMismatch`). An authentication host and a host that is not approved are otherwise
/// opaque by design, not a loss. Returns the per-reason counter of a fail-open tunnel, `None`
/// for one that is opaque by design.
fn fail_open_counter(counters: &Counters, reason: connect::TunnelReason) -> Option<&AtomicU64> {
    match reason {
        connect::TunnelReason::AuthHost | connect::TunnelReason::NotApproved => None,
        connect::TunnelReason::ClientHelloUnparsed => Some(&counters.fail_open_unparsed),
        connect::TunnelReason::SniAbsent => Some(&counters.fail_open_no_sni),
        connect::TunnelReason::SniMismatch => Some(&counters.fail_open_sni_mismatch),
        connect::TunnelReason::NotTls => Some(&counters.fail_open_not_tls),
    }
}

/// Counts a connection as unsettled (in admission or an opaque tunnel) while it lives; one
/// dropped without [`Unsettled::settle`] was cancelled before admission decided it, which is a
/// loss: it is counted (`cut_unsettled`) and chained as `Loss { ShutdownDeadline }`. That
/// includes a connection cut before it relayed anything (still reading the CONNECT head):
/// conservative, and the same `ShutdownDeadline` mapping the seal request's counter has always
/// had. A tunnel settles when it is decided, so one cut while open is not counted here.
struct Unsettled<'a> {
    shared: &'a Shared,
    settled: AtomicBool,
}

impl<'a> Unsettled<'a> {
    fn enter(shared: &'a Shared) -> Self {
        bump(&shared.counters.unsettled);
        Self {
            shared,
            settled: AtomicBool::new(false),
        }
    }

    fn settle(&self) {
        self.settled.store(true, Ordering::SeqCst);
    }
}

impl Drop for Unsettled<'_> {
    fn drop(&mut self) {
        unbump(&self.shared.counters.unsettled);
        if !*self.settled.get_mut() && !std::thread::panicking() {
            bump(&self.shared.counters.cut_unsettled);
            let _ = self
                .shared
                .emitter
                .proxy_loss(metadata::capture_time(), ProxyLoss::CutUnsettled);
        }
    }
}

/// A decided tunnel's note until it ends. One dropped before [`OpenTunnel::ended`] was cut open
/// (the stop, or a panic): it is noted then, with the drop as its end and its bytes unknown.
struct OpenTunnel<'a> {
    notes: &'a tunnels::TunnelNotes,
    open: std::sync::Mutex<Option<tunnels::TunnelNote>>,
}

impl<'a> OpenTunnel<'a> {
    fn new(notes: &'a tunnels::TunnelNotes) -> Self {
        Self {
            notes,
            open: std::sync::Mutex::new(None),
        }
    }

    fn decided(&self, start: connect::TunnelStart<'_>) {
        *self.open.lock().unwrap_or_else(|e| e.into_inner()) = Some(tunnels::TunnelNote {
            host: start.host.to_string(),
            port: start.port,
            reason: tunnels::reason_word(start.reason),
            opened: start.opened,
            closed: start.opened,
            up_bytes: None,
            down_bytes: None,
        });
    }

    /// Notes the tunnel as it ended; answers its reason.
    fn ended(&self, tunnel: &connect::Tunneled) -> connect::TunnelReason {
        self.open.lock().unwrap_or_else(|e| e.into_inner()).take();
        self.notes.push(tunnels::TunnelNote::of(tunnel));
        tunnel.reason
    }
}

impl Drop for OpenTunnel<'_> {
    fn drop(&mut self) {
        let open = self.open.get_mut().unwrap_or_else(|e| e.into_inner());
        if let Some(mut cut) = open.take() {
            cut.closed = std::time::SystemTime::now();
            self.notes.push(cut);
        }
    }
}

async fn relay_intercepted(intercepted: connect::Intercepted, shared: &Shared) {
    let connect::Intercepted {
        pair,
        host,
        policy,
        alpn,
    } = intercepted;
    let counters = &shared.counters;
    bump(&counters.intercepted);
    let connection = fresh_id();
    let emitter = Arc::clone(&shared.emitter);
    // Declared before the relay future: a cancelled relay is dropped first (ending its open
    // streams `Interrupted`), and only then does this guard record the connection's close.
    let Some(mut close) = CloseGuard::enter(counters, Arc::clone(&emitter), connection.clone())
    else {
        // The capture table is full: relay this one uncaptured rather than refuse it. The loss
        // is chained before the relay starts, at the moment it begins.
        bump(&counters.capture_table_full);
        let _ = shared
            .emitter
            .proxy_loss(metadata::capture_time(), ProxyLoss::CaptureTableFull);
        let RelayPair {
            mut client,
            mut upstream,
        } = pair;
        let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
        return;
    };
    counters.control(emitter.connection_event(
        &connection,
        metadata::capture_time(),
        ConnectionEvent::Open {
            alpn,
            already_open: false,
        },
    ));
    let context = RelayContext {
        connection,
        host,
        emitter,
        policy,
    };
    let result = relay(pair, context, counters).await;
    close.outcome = Some(outcome_of(&result));
}

/// The HTTP relay, then, for an accepted upgrade, the WebSocket relay on the same connection.
async fn relay(pair: RelayPair, context: RelayContext, counters: &Counters) -> io::Result<()> {
    match http::relay_http(pair, context).await? {
        HttpOutcome::Closed | HttpOutcome::Opaque => Ok(()),
        HttpOutcome::WebSocket(upgrade) => {
            counters.control(upgrade.context.emitter.connection_event(
                &upgrade.context.connection,
                metadata::capture_time(),
                ConnectionEvent::UpgradedWebSocket,
            ));
            websocket::relay_websocket(upgrade).await
        }
    }
}

/// How a relay result closes the connection.
fn outcome_of(result: &io::Result<()>) -> ConnectionOutcome {
    match result {
        Ok(()) => ConnectionOutcome::Complete,
        Err(error) => match error.kind() {
            io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe => ConnectionOutcome::Interrupted,
            _ => ConnectionOutcome::TransportError,
        },
    }
}

/// An intercepted connection's place in the capture table. Dropped, it records `Close`: with
/// the relay's mapped outcome, `Interrupted` when the relay was cancelled, `TransportError`
/// when it panicked. Only then, once the emitter has let the connection go, is the place given
/// back.
struct CloseGuard<'a> {
    counters: &'a Counters,
    emitter: Arc<CaptureEmitter>,
    connection: String,
    outcome: Option<ConnectionOutcome>,
}

impl<'a> CloseGuard<'a> {
    /// `None` when the table is full: one connection more would overflow the emitter's
    /// connection table, which ends capture for every connection.
    fn enter(
        counters: &'a Counters,
        emitter: Arc<CaptureEmitter>,
        connection: String,
    ) -> Option<Self> {
        counters
            .intercepted_open
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |open| {
                (open < MAX_OPEN_CONNECTIONS as u64).then_some(open + 1)
            })
            .ok()?;
        Some(Self {
            counters,
            emitter,
            connection,
            outcome: None,
        })
    }
}

impl Drop for CloseGuard<'_> {
    fn drop(&mut self) {
        let outcome = match self.outcome {
            Some(outcome) => outcome,
            None if std::thread::panicking() => ConnectionOutcome::TransportError,
            None => {
                bump(&self.counters.cut_intercepted);
                ConnectionOutcome::Interrupted
            }
        };
        bump(match outcome {
            ConnectionOutcome::Complete => &self.counters.closed_complete,
            ConnectionOutcome::Interrupted => &self.counters.closed_interrupted,
            ConnectionOutcome::TransportError => &self.counters.closed_transport_error,
        });
        self.counters.control(self.emitter.connection_event(
            &self.connection,
            metadata::capture_time(),
            ConnectionEvent::Close { outcome },
        ));
        unbump(&self.counters.intercepted_open);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kind by kind: the four ordinary ways a peer goes away are `Interrupted`, anything
    /// else a leaf returns is `TransportError`, and only `Ok` is `Complete`.
    #[test]
    fn leaf_results_map_to_connection_outcomes() {
        assert_eq!(outcome_of(&Ok(())), ConnectionOutcome::Complete);
        for kind in [
            io::ErrorKind::UnexpectedEof,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::BrokenPipe,
        ] {
            assert_eq!(
                outcome_of(&Err(io::Error::from(kind))),
                ConnectionOutcome::Interrupted,
                "{kind:?}"
            );
        }
        for kind in [
            io::ErrorKind::InvalidData,
            io::ErrorKind::TimedOut,
            io::ErrorKind::Other,
        ] {
            assert_eq!(
                outcome_of(&Err(io::Error::from(kind))),
                ConnectionOutcome::TransportError,
                "{kind:?}"
            );
        }
    }
}
