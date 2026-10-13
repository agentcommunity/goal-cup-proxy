//! The CLI's half of the control API: one client, one transport, one auth rule, no retry.
//!
//! # One transport
//!
//! `http://127.0.0.1:<port>`, and nothing else. The daemon's `control.json` carries no field
//! naming another transport, so there is no enum here to choose between two, and no second
//! module to keep in step with this one. A client with one way in cannot pick the wrong one.
//!
//! The base URL is built from `ControlFile.port` and the literal `127.0.0.1`, never from
//! `ControlFile.address`. The two say the same thing on every daemon this tree ships, and
//! the difference only shows up when the file has been edited: an `address` naming another
//! host would send the control bearer there, and a port cannot. That is why the field this
//! client reads is the one that cannot name a destination.
//!
//! # One auth rule, and it is the server's
//!
//! [`goalcup_protocol::control::is_public_route`] decides whether the bearer is attached, and it is
//! asked about the **raw** path — the exact string about to go on the wire, not a normalised copy,
//! not one this client rewrote first. The daemon's middleware asks the same function about
//! `request.uri().path()`, so the two cannot drift into disagreeing about which routes need a
//! token. A client that normalised before asking would be a client that sends a token on a route
//! the server treats as public.
//!
//! In this API it matters in exactly one direction: `/v0/entries/{id}/status` **is** public,
//! so `goalcup status` sends no `Authorization` header at all. Everything else this client
//! calls is authed.
//!
//! # No retry here, and that is the point
//!
//! [`ControlError::Busy`] and [`ControlError::Conflict`] share `409` and mean the opposite
//! things. `Busy` is the daemon holding the entry for the length of one asynchronous call —
//! raised on every mutating route while an entry is mid-pump — and the same request a
//! moment later is expected to succeed. `Conflict` is the entry answering "already sealed,
//! already started, a lock still on disk", and retrying that is a client spinning against a
//! state that will not change.
//!
//! Which of the two is worth asking again is a real question, and it is not this file's.
//! **This transport sends one request and reports what came back**, `busy` included.
//!
//! Retry policy lives one layer up, in [`crate::retry`]. Most commands use
//! [`crate::retry::retry_busy`]; the `/abandon` and `/process` routes and the capture seal
//! select their route-specific policies there. It is above the transport because a retry buried in the
//! HTTP client is a retry that only that client has: a command driven by the in-process
//! `MockDaemonClient` would meet the mock's one `busy` and stop, the fake would be modelling
//! the daemon faithfully, and the rule the command is supposed to honour — an abandon
//! retries a busy — would be untestable everywhere the commands are actually tested. One
//! policy seam, and both clients behave the same on the far side of it.
//!
//! # The token
//!
//! It is the one secret this struct holds. It is never interpolated into an error, never
//! printed, and [`HttpDaemonClient`]'s `Debug` redacts it — the same treatment
//! `config::ControlFile` gives the
//! credential it carries. `tests/it/daemon_client.rs` greps every refusal this client raises
//! for it.

#[cfg(feature = "test-support")]
mod mock;
pub mod spawn;

#[cfg(feature = "test-support")]
pub use mock::{MockCapture, MockDaemonClient, MockState};
pub use spawn::{MissingConfig, daemon_to_read, ensure_daemon};

use crate::CliError;
use crate::config::read_control_file;
use goalcup_protocol::capture::StreamKey;
use goalcup_protocol::control::{
    CONTROL_API_VERSION, CaptureFinishReport, CaptureGoalRequest, CaptureLaunchRequest,
    CaptureSealError, CaptureSealRequest, CaptureWorkspaceInfo, CaptureWorkspaceRequest,
    ControlError, DaemonClient, DaemonHealth, EntrySummary, ErrorBody, FINISH_COMMIT_RESERVE,
    ROUTE_CAPTURE_WORKSPACES, ROUTE_ENTRIES, ROUTE_ENTRIES_CURRENT, ROUTE_HEALTH, SUFFIX_ABANDON,
    SUFFIX_CAPTURE_GOAL, SUFFIX_CAPTURE_LAUNCH, SUFFIX_PROCESS, SUFFIX_SEAL, SUFFIX_STATUS,
    SealResponse, SignedStatus, entry_route, is_public_route,
};
use goalcup_protocol::process::ProcessRegistration;
use std::path::Path;
use std::time::Duration;

/// The most bytes this client reads from any control response.
///
/// The daemon's own request limit is a megabyte and its answers are smaller than that by an
/// order of magnitude — an entry summary, a workspace document, a seal manifest. Sixty-four
/// kibibytes is well past every one of them and small enough that a daemon answering
/// something this client did not ask for cannot make it allocate. A body over the cap is
/// refused having read the cap and one byte, never read whole and then rejected.
pub const MAX_RESPONSE_BYTES: u64 = 64 * 1024;

/// How long this client waits for the loopback to accept a connection.
///
/// Short, because the far end is this machine: a connection to 127.0.0.1 that has not been
/// accepted in two seconds is not a slow link, it is a daemon that is not there.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// How long the capture seal waits for its answer: what remained of the one local-finish
/// budget when the request was built, the termination allowance, and [`CONNECT_TIMEOUT`] as
/// slack for request latency and `ps` spawn overhead beyond `termination_bound` (poll
/// granularity is excluded from that bound, as TERMINATION.md states). All three are existing
/// constants; no new budget is added (CONTRACTS.md, H-10 4a ¶6).
pub fn capture_seal_read_timeout(remaining: Duration, termination_allowance: Duration) -> Duration {
    remaining
        .saturating_add(termination_allowance)
        .saturating_add(CONNECT_TIMEOUT)
}

/// The deadline on an ordinary call — a directory read, a health check, an abandon, a
/// capture workspace.
///
/// Public because it is also the unit [`crate::retry::BUSY_BUDGET`] is written in: the
/// whole of a retried call is allowed twice this and no more, and the two numbers moving
/// apart would break that bound.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// The deadline on the short `/process` handoff route.
///
/// The daemon only validates and records one immutable process handle on this route; it does
/// not wait for the newly launched child to do work. A quick `busy`, success, or transport
/// refusal is therefore the useful answer. This duration is deliberately part of
/// [`crate::retry::PROCESS_BUSY_BUDGET`]: nine attempts plus that helper's 13-second retry
/// envelope bound a real handoff at 31 seconds rather than letting ordinary 30-second calls
/// hold the terminal. That envelope spans one daemon upload network timeout while preserving
/// this route's short request deadline.
pub(crate) const PROCESS_HANDOFF_CALL_TIMEOUT: Duration = Duration::from_secs(2);

/// The deadline on the **one** call that does unbounded work before it answers: the seal,
/// which finishes the entry and terminates a process tree.
///
/// It is five minutes, and an admitted call that may take five minutes must never be replayed
/// after an ambiguous timeout, unreachable transport, or malformed response: the daemon may
/// have completed that seal. The bare seal route reads only the entry's registry row and never
/// answers `busy`. No other call carries this timeout: giving an ordinary call the seal's
/// deadline would give the retry helper a budget it could blow through.
///
/// So this constant appears at exactly one call site. No test pins that count any more.
pub const SLOW_CALL_TIMEOUT: Duration = Duration::from_secs(300);

/// The control API client. One base URL, one bearer, one agent.
pub struct HttpDaemonClient {
    base: String,
    token: String,
    agent: ureq::Agent,
}

/// A successful control response, retaining whether its body had zero bytes before JSON
/// decoding normalizes an empty body and the JSON literal `null` to the same value.
struct CallResponse {
    status: u16,
    value: serde_json::Value,
    body_is_empty: bool,
}

/// A control call that did not produce its ordinary successful response.
///
/// A framed daemon refusal is distinct from a reply this client could not verify or parse.
/// The ordinary control trait exposes both as [`ControlError`]; only the capture seal reads
/// the difference, to tell a refusal made before the finish began from an ambiguous one.
enum CallError {
    Uncertain(ControlError),
    /// The connection was never accepted, so nothing was written (`ureq`'s `ConnectionFailed`,
    /// raised only on connect). Still [`ControlError::Unreachable`] to every caller that does
    /// not ask; only the capture seal reads the difference.
    NotConnected(ControlError),
    Refusal {
        error: ControlError,
    },
}

impl CallError {
    fn into_error(self) -> ControlError {
        match self {
            Self::Uncertain(error) | Self::NotConnected(error) | Self::Refusal { error, .. } => {
                error
            }
        }
    }
}

/// Every field as a derive would print it, except `token`, which prints `<redacted>`.
///
/// This struct holds the credential every authenticated control route trusts, and a `{:?}`
/// of a client is exactly the thing that ends up in a `goalcup doctor` dump or an
/// `Internal` message somebody pastes into a bug report.
impl std::fmt::Debug for HttpDaemonClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpDaemonClient")
            .field("base", &self.base)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl HttpDaemonClient {
    /// The client the running daemon's `control.json` describes.
    ///
    /// The document has five fields and this reads three of them: the version, so a daemon
    /// this build cannot speak to is refused here rather than mis-parsed one call later;
    /// the port, which is what the base URL is built from; and the token. `address` is read
    /// by other callers — `goalcup doctor` — and deliberately not by this one.
    pub fn from_config_dir(cfg: &Path) -> Result<HttpDaemonClient, CliError> {
        let file = read_control_file(cfg)?;
        if file.control_api_version != CONTROL_API_VERSION {
            return Err(CliError::Refused(format!(
                "the daemon at {}/control.json speaks control API version {} and this \
                 goalcup speaks {CONTROL_API_VERSION}. Upgrade whichever of the two is \
                 older and start the daemon again.",
                cfg.display(),
                file.control_api_version
            )));
        }
        HttpDaemonClient::new(format!("http://127.0.0.1:{}", file.port), file.token)
    }

    /// A client for `base` — `http://127.0.0.1:<port>`, no trailing slash — holding `token`.
    ///
    /// **A base that is not loopback is a refusal**, the same refusal `from_config_dir`
    /// raises, and for the same reason: this client attaches the control bearer to every
    /// route the daemon does not publish, and a base naming another host would send that
    /// credential there. `from_config_dir` reaches this rule by building the base from
    /// `ControlFile.port` and never from `ControlFile.address`; a caller that hands one over
    /// directly meets it here. Neither path can produce a client that would carry the token
    /// off this machine.
    ///
    /// A *name* is refused as well, `localhost` included: a name is resolved by something
    /// this process does not control, and the fix is to write the literal.
    ///
    /// No connection pooling. The calls a run makes are seconds and minutes apart, and a
    /// pooled connection the daemon closed between two of them is a transport error this
    /// client would have reported as "the daemon is not reachable" — which is the one
    /// sentence a participant acts on by restarting a daemon that was running fine.
    pub fn new(base: String, token: String) -> Result<HttpDaemonClient, CliError> {
        if !is_loopback_base(&base) {
            return Err(CliError::Refused(format!(
                "{base} is not a loopback control API, and this client will not send the \
                 daemon's control token to an address that is not this machine. The base \
                 must read http://<loopback literal>:<port> — 127.0.0.1 or [::1], written \
                 out, because a name is resolved by something this process does not control."
            )));
        }
        let agent = ureq::AgentBuilder::new()
            .max_idle_connections(0)
            .max_idle_connections_per_host(0)
            // A control route that answered a redirect is a control route this client does
            // not understand, and following one would carry the bearer off the loopback.
            .redirects(0)
            .timeout_connect(CONNECT_TIMEOUT)
            .build();
        Ok(HttpDaemonClient { base, token, agent })
    }

    /// **The one place a request is made**, and so the one place the auth rule lives.
    ///
    /// One request, sent once. Nothing here retries anything and nothing here sleeps: a
    /// `busy` is handed back to the caller exactly as it arrived, and the command's
    /// [`crate::retry`] policy decides whether to ask again.
    ///
    /// `path` is the raw request target, query and all, exactly as it will go on the wire.
    fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
        read_timeout: Duration,
    ) -> Result<CallResponse, CallError> {
        let url = format!("{}{path}", self.base);
        let mut request = match method {
            "GET" => self.agent.get(&url),
            "POST" => self.agent.post(&url),
            // Not reachable from this file — every call site below passes one of the two —
            // and an `Internal` rather than a panic because nothing in `src/` panics.
            other => {
                return Err(CallError::Uncertain(ControlError::Internal(format!(
                    "{other} is not a method this client sends"
                ))));
            }
        };
        request = request
            .timeout(read_timeout)
            .set("Accept", "application/json")
            .set(
                "x-goalcup-control-version",
                &CONTROL_API_VERSION.to_string(),
            );
        // The RAW path — the string above, before anything touched it — and the same
        // function the daemon's middleware asks.
        if !is_public_route(path) {
            request = request.set("Authorization", &format!("Bearer {}", self.token));
        }
        let sent = match body {
            Some(document) => {
                let text = serde_json::to_string(document).map_err(|e| {
                    CallError::Uncertain(ControlError::Malformed(format!("{url}: {e}")))
                })?;
                request
                    .set("Content-Type", "application/json")
                    .send_string(&text)
            }
            None => request.call(),
        };
        let response = match sent {
            Ok(response) => response,
            // A refusal is an answer: its body carries the wire code, and reading it is the
            // whole point of the mapping below.
            Err(ureq::Error::Status(_, response)) => response,
            Err(ureq::Error::Transport(t)) => {
                let unreachable = ControlError::Unreachable(format!("{url}: {t}"));
                return Err(if t.kind() == ureq::ErrorKind::ConnectionFailed {
                    CallError::NotConnected(unreachable)
                } else {
                    CallError::Uncertain(unreachable)
                });
            }
        };
        // The response version is part of the paired control contract, including error bodies.
        // A request header protects a new daemon from an old client; this exact-one check is
        // the reverse direction, preventing a new CLI from interpreting an old daemon's
        // pre-mutation refusal or success JSON as the current version.
        let physical_versions = response
            .headers_names()
            .into_iter()
            .filter(|name| name.eq_ignore_ascii_case("x-goalcup-control-version"))
            .count();
        let versions = response.all("x-goalcup-control-version");
        if physical_versions != 1
            || versions.len() != 1
            || versions[0] != CONTROL_API_VERSION.to_string()
        {
            return Err(CallError::Uncertain(ControlError::Malformed(format!(
                "{url}: expected exactly one x-goalcup-control-version response header equal to {CONTROL_API_VERSION}"
            ))));
        }
        let status = response.status();
        // `ureq` correctly exposes every `204` as bodyless, even if a broken peer writes
        // bytes after its headers. Preserve its Content-Length framing too: an advertised
        // nonzero body is not a bare acknowledgement, even though the HTTP reader cannot
        // hand those bytes to `read_bounded` for a `204`.
        let declared_zero_bytes = response
            .all("content-length")
            .iter()
            .all(|length| length.parse::<u64>() == Ok(0));
        // A bare acknowledgement has neither a length-delimited nor transfer-delimited body.
        // `ureq` correctly hides a 204's chunk bytes, so header framing is the remaining
        // evidence that a peer wrote something this route must not accept as an ack.
        let has_transfer_encoding = !response.all("transfer-encoding").is_empty();
        let text = read_bounded(&url, response).map_err(CallError::Uncertain)?;
        if (200..300).contains(&status) {
            // The ordinary JSON callers only need a null sentinel for a zero-byte body. The
            // `/process` acknowledgement additionally needs to distinguish that body from
            // whitespace and the JSON literal `null`, both of which are non-empty on the
            // wire.
            if text.is_empty() {
                return Ok(CallResponse {
                    status,
                    value: serde_json::Value::Null,
                    body_is_empty: declared_zero_bytes && !has_transfer_encoding,
                });
            }
            return serde_json::from_str(&text)
                .map(|value| CallResponse {
                    status,
                    value,
                    body_is_empty: false,
                })
                .map_err(|e| CallError::Uncertain(ControlError::Malformed(format!("{url}: {e}"))));
        }
        // The wire codes survive the wire: the body decides, not the status. An
        // unrecognised code becomes `Internal` inside `from_code`, never a silent success.
        match serde_json::from_str::<ErrorBody>(&text) {
            Ok(body) => Err(CallError::Refusal {
                error: ControlError::from_code(&body.error.code, &body.error.message),
            }),
            Err(e) => Err(CallError::Uncertain(ControlError::Malformed(format!(
                "{url} answered {status} with a body this client could not read as the \
                 control API's error shape: {e}"
            )))),
        }
    }

    /// `GET`, with the ordinary deadline.
    fn get(&self, path: &str) -> Result<serde_json::Value, ControlError> {
        self.call("GET", path, None, CALL_TIMEOUT)
            .map(|response| response.value)
            .map_err(CallError::into_error)
    }

    /// `POST`, with the deadline named.
    fn post(
        &self,
        path: &str,
        body: Option<&serde_json::Value>,
        read_timeout: Duration,
    ) -> Result<serde_json::Value, ControlError> {
        self.call("POST", path, body, read_timeout)
            .map(|response| response.value)
            .map_err(CallError::into_error)
    }

    /// `POST` to an entry's seal route, legacy or v5: the one call that carries
    /// [`SLOW_CALL_TIMEOUT`].
    fn post_seal(
        &self,
        entry_id: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, ControlError> {
        self.post(&entry_route(entry_id, SUFFIX_SEAL), body, SLOW_CALL_TIMEOUT)
    }
}

/// One capture seal that survives the daemon being restarted under the caller (H-14), or
/// dying under it.
///
/// `goalcup start` builds its control client once, before the run, from `control.json`; the
/// daemon re-mints that file's port and token at every start. A daemon restarted between the
/// launch and the finish is therefore at another port or behind another token, and the first
/// attempt is refused with `unauthorized` or a connection nobody accepted. Both are
/// [`CaptureSealError::NotAdmitted`] (nothing of the finish began), so asking again is safe.
///
/// So this reads `control.json` under `cfg` once, builds a client from it and sends the same
/// request there. When nobody answers there either, the daemon is gone, and there is no service
/// to bring it back: [`ensure_daemon`] starts one from `<cfg>/daemon.toml`, which reopens the
/// unfinished entry for status and seal, and the request goes to it. The time that start takes
/// comes out of the one local-finish budget: the request's `remaining_finish_ms` and the read
/// timeout both shrink by it, and a start that would leave less than [`FINISH_COMMIT_RESERVE`] is
/// not attempted or not followed by a request. The client that answered is kept in `fresh` and
/// every later attempt goes to it. Every other failure, a `busy` included, is
/// returned as it came, and a daemon that cannot be found or started leaves the first failure
/// standing.
///
/// `cfg` is `None` where the caller has no configuration directory to re-read (a standalone
/// `goalcup seal` makes sure of its daemon moments before its one call).
///
/// Called only from inside the capture seal's own retry (`retry_capture_seal`, in
/// `commands::seal::finish_locally`): the one place a capture seal is sent.
pub fn seal_capture_following_restart(
    first: &dyn DaemonClient,
    fresh: &mut Option<HttpDaemonClient>,
    cfg: Option<&Path>,
    entry_id: &str,
    request: &CaptureSealRequest,
    read_timeout: Duration,
) -> Result<CaptureFinishReport, CaptureSealError> {
    if let Some(rebuilt) = fresh.as_ref() {
        return rebuilt.seal_capture(entry_id, request, read_timeout);
    }
    let failure = match first.seal_capture(entry_id, request, read_timeout) {
        Err(
            failure @ CaptureSealError::NotAdmitted(
                ControlError::Unreachable(_) | ControlError::Unauthorized(_),
            ),
        ) => failure,
        answered => return answered,
    };
    let Some(cfg) = cfg else {
        return Err(failure);
    };
    // A daemon restarted under the run: `control.json` names it.
    if let Ok(rebuilt) = HttpDaemonClient::from_config_dir(cfg) {
        match rebuilt.seal_capture(entry_id, request, read_timeout) {
            // Nobody there either: the daemon is gone, and one is started below.
            Err(CaptureSealError::NotAdmitted(ControlError::Unreachable(_))) => {}
            answer => {
                *fresh = Some(rebuilt);
                return answer;
            }
        }
    }
    // What of the budget a daemon start may spend: everything but the commit share.
    let remaining = request.remaining_finish_ms.map_or(
        goalcup_protocol::control::LOCAL_CAPTURE_FINISH_BUDGET,
        Duration::from_millis,
    );
    let spare = remaining.saturating_sub(FINISH_COMMIT_RESERVE);
    if spare.is_zero() {
        return Err(failure);
    }
    let began = std::time::Instant::now();
    let deadline = began + spare.min(spawn::SPAWN_WAIT);
    let Ok(started) = ensure_daemon(cfg, MissingConfig::Refuse, deadline) else {
        return Err(failure);
    };
    let spent = began.elapsed();
    if remaining.saturating_sub(spent) < FINISH_COMMIT_RESERVE {
        return Err(failure);
    }
    let mut request = request.clone();
    request.remaining_finish_ms = request
        .remaining_finish_ms
        .map(|ms| ms.saturating_sub(u64::try_from(spent.as_millis()).unwrap_or(u64::MAX)));
    let answer = started.seal_capture(entry_id, &request, read_timeout.saturating_sub(spent));
    *fresh = Some(started);
    answer
}

/// The response body, up to [`MAX_RESPONSE_BYTES`], or a refusal naming the cap.
///
/// One byte past the cap, so a body exactly at it still reads and anything larger is caught
/// having read the cap and no more: the shape `config::read_control_file` also uses.
fn read_bounded(url: &str, response: ureq::Response) -> Result<String, ControlError> {
    use std::io::Read;

    let mut text = String::new();
    let read = response
        .into_reader()
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_string(&mut text)
        // A body that stopped early, or one that is not UTF-8, is a daemon that answered
        // badly — not a daemon that is not there. `Unreachable` is the sentence a
        // participant fixes by restarting a daemon, and restarting a daemon that answered
        // is the wrong thing to be told to do, so the two cases are separated here rather
        // than folded into the transport's error.
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::InvalidData => {
                ControlError::Malformed(format!("{url}: {e}"))
            }
            _ => ControlError::Unreachable(format!("{url}: {e}")),
        })?;
    if read as u64 > MAX_RESPONSE_BYTES {
        return Err(ControlError::Malformed(format!(
            "{url} answered with more than {MAX_RESPONSE_BYTES} bytes, which is more than \
             any control API answer"
        )));
    }
    Ok(text)
}

/// Whether `base` names a loopback address on this machine, written as a literal.
///
/// The scheme must be `http://` (this API is loopback cleartext and this client speaks
/// nothing else), the authority is taken up to the first `/`, userinfo is dropped from the
/// front and the port from the back, and an IPv6 literal keeps its brackets so the colons
/// inside it are not read as a port separator. What is left must parse as an
/// `IpAddr` that `is_loopback` — the rule, rather than a string comparison this file would
/// have had to get right twice.
fn is_loopback_base(base: &str) -> bool {
    let Some(authority) = base.strip_prefix("http://") else {
        return false;
    };
    let authority = match authority.split_once('/') {
        Some((authority, _)) => authority,
        None => authority,
    };
    let after_userinfo = match authority.rsplit_once('@') {
        Some((_, host)) => host,
        None => authority,
    };
    let host = if let Some(rest) = after_userinfo.strip_prefix('[') {
        match rest.split_once(']') {
            Some((inside, _)) => inside,
            None => return false,
        }
    } else {
        match after_userinfo.split_once(':') {
            Some((host, _)) => host,
            None => after_userinfo,
        }
    };
    match host.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        // A name, including `localhost`. Refused by spelling: this process does not resolve
        // it and cannot promise where it points.
        Err(_) => false,
    }
}

/// One answer, as the type its route promises.
fn parsed<T: serde::de::DeserializeOwned>(
    path: &str,
    value: serde_json::Value,
) -> Result<T, ControlError> {
    serde_json::from_value(value).map_err(|e| ControlError::Malformed(format!("{path}: {e}")))
}

/// One request body, as JSON.
fn document<T: serde::Serialize>(value: &T) -> Result<serde_json::Value, ControlError> {
    serde_json::to_value(value).map_err(|e| ControlError::Malformed(e.to_string()))
}

impl DaemonClient for HttpDaemonClient {
    /// The one public route this client calls: no `Authorization` header goes with it, and
    /// [`SignedStatus::verify_with`] against the run key the CLI pinned at creation is what
    /// makes that safe.
    fn status(&self, entry_id: &str) -> Result<SignedStatus, ControlError> {
        let path = entry_route(entry_id, SUFFIX_STATUS);
        let answer = self.get(&path)?;
        parsed(&path, answer)
    }

    fn entries(&self) -> Result<Vec<EntrySummary>, ControlError> {
        let answer = self.get(ROUTE_ENTRIES)?;
        parsed(ROUTE_ENTRIES, answer)
    }

    /// The bare seal, which carries [`SLOW_CALL_TIMEOUT`] and no other route does.
    ///
    /// `goalcup seal` sends it only when the entry has no capture status: the daemon answers
    /// it `malformed` for a capture row it could not reopen (a torn entry) and `missing` for an
    /// id it does not hold. The `conflict` that shares `busy`'s `409` is never retried.
    fn seal(&self, entry_id: &str) -> Result<SealResponse, ControlError> {
        let answer = self.post_seal(entry_id, None)?;
        parsed(&entry_route(entry_id, SUFFIX_SEAL), answer)
    }

    /// The handoff's acknowledgement has a narrower contract than an ordinary JSON control
    /// response: only a bare `204` (no length- or transfer-delimited body) acknowledges.
    fn register_process(
        &self,
        entry_id: &str,
        request: &ProcessRegistration,
    ) -> Result<(), ControlError> {
        let body = document(request)?;
        let path = entry_route(entry_id, SUFFIX_PROCESS);
        match self.call("POST", &path, Some(&body), PROCESS_HANDOFF_CALL_TIMEOUT) {
            Ok(response) if response.status == 204 && response.body_is_empty => Ok(()),
            Ok(_) => Err(ControlError::Malformed(format!(
                "{path}: expected HTTP 204 No Content for a process registration acknowledgement"
            ))),
            Err(error) => Err(error.into_error()),
        }
    }

    fn health(&self) -> Result<DaemonHealth, ControlError> {
        let answer = self.get(ROUTE_HEALTH)?;
        parsed(ROUTE_HEALTH, answer)
    }

    fn current_entry(&self) -> Result<Option<String>, ControlError> {
        let answer = self.get(ROUTE_ENTRIES_CURRENT)?;
        parsed(ROUTE_ENTRIES_CURRENT, answer)
    }

    /// The reason is the whole body: the daemon reads `{"reason": …}` and records it in the
    /// refusal a started entry answers with.
    fn abandon(&self, entry_id: &str, reason: &str) -> Result<(), ControlError> {
        let body = serde_json::json!({ "reason": reason });
        self.post(
            &entry_route(entry_id, SUFFIX_ABANDON),
            Some(&body),
            CALL_TIMEOUT,
        )?;
        Ok(())
    }

    fn create_capture_workspace(
        &self,
        request: &CaptureWorkspaceRequest,
    ) -> Result<CaptureWorkspaceInfo, ControlError> {
        let body = document(request)?;
        let answer = self.post(ROUTE_CAPTURE_WORKSPACES, Some(&body), CALL_TIMEOUT)?;
        parsed(ROUTE_CAPTURE_WORKSPACES, answer)
    }

    /// Sent once. The caller treats every failure, a lost answer included, as a goal that
    /// may be recorded and does not launch.
    fn record_capture_goal(
        &self,
        entry_id: &str,
        request: &CaptureGoalRequest,
    ) -> Result<StreamKey, ControlError> {
        let path = entry_route(entry_id, SUFFIX_CAPTURE_GOAL);
        let body = document(request)?;
        let answer = self.post(&path, Some(&body), CALL_TIMEOUT)?;
        parsed(&path, answer)
    }

    fn record_capture_launch(
        &self,
        entry_id: &str,
        request: &CaptureLaunchRequest,
    ) -> Result<(), ControlError> {
        let body = document(request)?;
        self.post(
            &entry_route(entry_id, SUFFIX_CAPTURE_LAUNCH),
            Some(&body),
            CALL_TIMEOUT,
        )?;
        Ok(())
    }

    /// Sent once, with the caller's own read timeout. Only a failure that is provably before
    /// the finish began is `NotAdmitted`: `busy`, `unauthorized` (the auth middleware answers
    /// before the handler) and a connection the daemon never accepted. Every other failure,
    /// a framed 400 and every 404 included, is `MaybeAdmitted`.
    fn seal_capture(
        &self,
        entry_id: &str,
        request: &CaptureSealRequest,
        read_timeout: Duration,
    ) -> Result<CaptureFinishReport, CaptureSealError> {
        let body = document(request).map_err(CaptureSealError::MaybeAdmitted)?;
        let path = entry_route(entry_id, SUFFIX_SEAL);
        match self.call("POST", &path, Some(&body), read_timeout) {
            Ok(answer) => parsed(&path, answer.value).map_err(CaptureSealError::MaybeAdmitted),
            Err(CallError::NotConnected(error)) => Err(CaptureSealError::NotAdmitted(error)),
            Err(CallError::Refusal {
                error: error @ (ControlError::Busy(_) | ControlError::Unauthorized(_)),
                ..
            }) => Err(CaptureSealError::NotAdmitted(error)),
            Err(other) => Err(CaptureSealError::MaybeAdmitted(other.into_error())),
        }
    }
}
