//! The control API's server side: the state the handlers share, the one authentication
//! rule, `control.json`, and the router the CLI reaches.
//!
//! The contract is `goalcup_protocol::control` and this module implements it. It defines
//! no wire type of its own — [`ControlFile`] is a local file and not a wire shape — and it
//! interprets nothing: every handler resolves an entry and calls a [`Daemon`] method.
//!
//! # One authentication rule, shared with the client
//!
//! `control::is_public_route` is called from exactly one place in this crate — the auth
//! layer below — and the CLI's `HttpDaemonClient` calls the same function to decide
//! whether to send the header. Server and client therefore cannot drift into disagreeing
//! about which routes carry a token, which is the failure that made the status page a
//! guaranteed `401` in the first place.
//!
//! It is asked about the **raw** path: `request.uri().path()`, exactly as it arrived, and
//! never a decoded, normalised or extractor-produced one. The reason is that the raw
//! path is the string axum then matches: `matchit` splits `uri().path()` on literal `/`
//! and decodes nothing. A layer that authenticated a decoded path while the router routed
//! the undecoded one would be deciding about a different request than the one that gets
//! served, and every gap between those two strings is a gap in the decision.
//!
//! `is_public_route` does its own single percent decode per segment and refuses a segment
//! that decodes to `.` or `..`, plus any path carrying a `%` that is not a well-formed
//! `%XX`. So `/v0/entries/%2e%2e/status` is refused here, and so is
//! `/v0/entries/%zz/status`. This layer adds no second rule to that: public is exactly what the
//! function says, because the CLI calls the same function to decide whether to send the
//! header and a second rule on this side is the drift the shared function exists to stop.
//!
//! `%2f` inside a segment is deliberately not refused; this layer decodes nothing itself,
//! and the handlers behind it decode only after auth has already answered.
//!
//! ## The one spelling the contract admits, written down
//!
//! `/v0/entries/%252e%252e/status` decodes once to the literal characters `%2e%2e`, which is
//! neither `.` nor `..`, so `is_public_route` admits it as the status route of an entry with a
//! strange id. That is the contract's answer and this module does not second-guess it. It is
//! not an escalation, for two reasons that `tests/it/control_api.rs` asserts rather than assumes:
//! the router matches the raw path too, so `%252e%252e` is a literal id naming no entry and
//! the answer is `404`; and the hop that would turn it into a traversal — a proxy that decodes
//! once — produces `%2e%2e`, which this layer refuses. Tightening the contract's function to
//! refuse a segment whose single decode still carries a `%` is a change to
//! `goalcup_protocol::control`, not to this crate.
//!
//! # The `Host` is checked first
//!
//! Before the public/bearer split, `authenticate` refuses any request whose `Host`
//! is not exactly one of the listener's own addresses — `127.0.0.1:<port>`, `localhost:<port>`
//! or `[::1]:<port>`, with the port the OS bound. The two public routes are readable by a
//! browser without a token, and DNS rebinding is how a web page the browser believes belongs
//! to `evil.example` reaches a loopback socket: it sends `Host: evil.example:<port>` and reads
//! the answer same-origin. The refusal is `401 unauthorized` with a fixed body (it quotes no
//! header it was sent), so no contract code is added and `CONTROL_API_VERSION` does not move;
//! every in-tree client builds its `Host` from the address in `control.json` and passes.
//! `serve_control` hands the router the bound port as a request extension, and a router
//! served any other way refuses everything.
//!
//! # The layer wraps the whole router
//!
//! Not the authenticated sub-router, and not a fallback. The bearer has to be decided
//! before a router is consulted, because otherwise the set of registered routes is
//! enumerable without a token: an unregistered protected path would answer `404` and a
//! registered one `401`, and the difference is a map of the API. Wrapping everything means
//! every protected path answers the same `401` whether or not a handler exists behind it —
//! which is also what lets a handler be added without touching this rule.
//!
//! # Two tokens, and they are never the same string
//!
//! The bearer in `control.json` is the **CLI's**, minted once per daemon and living only
//! in that file at mode 0600. D24's receiver token is per run and goes into the workspace
//! where the agent can read it. Neither is ever logged — this crate has no logger — and
//! neither appears in an error body: the refusal below is a fixed constant.
//!
//! It is also never written through anything. [`write_private`] creates a temp file and
//! renames, so the bearer only ever lands in a file this daemon made, at a mode this
//! daemon set; a `control.json` that is a symlink, a directory or anything else that is
//! not a regular file is a refusal and the daemon does not start. `config_dir` is a
//! directory a participant can reach, and an open of the target would have handed the
//! credential to whatever that target named.

mod file;
mod handlers;
mod router;

// The module's public surface: every name below is reachable as `crate::control::…`.
// The split is by area — the router and its one authentication rule, the handlers, and
// `control.json`.
pub use file::{
    CONTROL_FILE_NAME, ControlFile, mint_control_token, read_control_file,
    sweep_stale_control_temp, write_control_file,
};
pub(crate) use goalcup_protocol::constant_time::constant_time_eq;
pub use handlers::AbandonRequest;
pub use router::{
    ControlListener, ENTRY_SUFFIXES, MAX_CONTROL_BODY_BYTES, SUFFIX_ABANDON, SUFFIX_PROCESS,
    SUFFIX_SEAL, SUFFIX_STATUS, TOP_LEVEL_ROUTES, VERSION_HEADER, control_router, entry_pattern,
    serve_control,
};

use crate::DaemonError;
use crate::runtime::Daemon;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use goalcup_protocol::control::{ControlError, ErrorBody, ErrorDetail};
use std::sync::Arc;
use std::time::Instant;

// ------------------------------------------------------------------ state

/// What every control handler shares: the daemon, the CLI's bearer, and when this process
/// came up.
///
/// The token is an `Arc<String>` and not a `String` because the state is cloned into the
/// auth layer and into each sub-router, and a secret copied per clone is a secret in more
/// places than it needs to be.
#[derive(Clone)]
pub struct ControlState {
    pub daemon: Arc<Daemon>,
    pub token: Arc<String>,
    pub started_at: Instant,
}

/// Nothing but the shape. A derived `Debug` would put the CLI's bearer into every
/// interpolated error that carries the state, which is the leak `WorkspaceOpts`,
/// `WorkspaceInfo` and `RunKeys` all hand-write their `Debug` to close.
impl std::fmt::Debug for ControlState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlState")
            .field("token", &"<redacted>")
            .field("started_at", &self.started_at)
            .finish_non_exhaustive()
    }
}

// ------------------------------------------------------------------ errors

/// A `ControlError` variant → its wire code, its HTTP status, and one value of it — in
/// one place in this crate and generated from one list of rows.
///
/// The macro exists for a property a hand-written table cannot have. It expands to
/// [`http_status_for`], whose `match` names every variant with **no `_` arm**, and to
/// [`CONTROL_ERROR_CODES`] and [`every_control_error`], which the test enumerates the
/// table with. So a new `ControlError` variant does not compile until it has a row here, and
/// the row it needs gives it a status, a code and a sample at once. A new refusal cannot
/// arrive as a silent `500`, and the list the test walks cannot be left behind by the
/// match it is checking.
macro_rules! control_error_table {
    ($( $pattern:pat => ($code:literal, $status:expr, $sample:expr) ),+ $(,)?) => {
        /// Every code the contract's `ControlError` defines, in the order the table
        /// declares them. Generated, so it is exactly the set [`http_status_for`] answers.
        #[cfg(any(test, feature = "test-hooks"))]
        pub const CONTROL_ERROR_CODES: &[&str] = &[$($code),+];

        /// One value of every `ControlError` variant, in [`CONTROL_ERROR_CODES`]'s order.
        ///
        /// Public, and compiled only for tests (`test-hooks`), because the daemon's integration
        /// tests are a separate crate and this is
        /// the enumeration they assert the table over; generated from the same rows the
        /// match is, which is the whole point — a hand-held list would go stale the first
        /// time a variant was added and the test would quietly stop covering it.
        #[cfg(any(test, feature = "test-hooks"))]
        pub fn every_control_error() -> Vec<ControlError> {
            vec![$($sample),+]
        }

        /// A `ControlError` → the HTTP status, in one place in this crate.
        ///
        /// The protocol crate carries no mapping — it is shared with a client that reads
        /// codes rather than statuses — so this table is the whole of it, and every
        /// handler reaches the wire through it.
        ///
        /// | code | status | why |
        /// |---|---|---|
        /// | `unauthorized` | 401 | the bearer was absent or wrong |
        /// | `malformed` | 400 | the request or a document in it did not parse |
        /// | `not_found` | 404 | no such entry, no such route |
        /// | `arming_timeout` | 408 | the long poll waited and the signal did not come |
        /// | `conflict` | 409 | the entry itself refuses: already sealed, already started, a lock still on disk. **Permanent. A client reports it and does not retry** |
        /// | `busy` | 409 | the entry is held for the length of one pump. **Transient, and the only code a client retries** |
        /// | `preflight_failed` | 422 | the request was well formed and the machine said no |
        /// | `unsupported_connector` | 422 | likewise: this daemon does not ship that connector |
        /// | `unsupported` | 426 | the client speaks a control API version this build does not |
        /// | `internal` | 500 | the daemon broke |
        /// | `unreachable` | 503 | the daemon cannot answer right now |
        ///
        /// It takes the error and not its code, so the match is exhaustive and a variant
        /// added to the contract is a compile error here rather than a `500` on the wire.
        pub fn http_status_for(error: &ControlError) -> StatusCode {
            match error {
                $($pattern => $status),+
            }
        }
    };
}

/// The message every generated sample carries. It is never sent anywhere: the samples
/// exist so a test can walk the table.
#[cfg(any(test, feature = "test-hooks"))]
const SAMPLE_MESSAGE: &str = "a sample refusal";

control_error_table! {
    ControlError::Unreachable(_) => (
        "unreachable",
        StatusCode::SERVICE_UNAVAILABLE,
        ControlError::Unreachable(SAMPLE_MESSAGE.to_string())
    ),
    ControlError::NotFound(_) => (
        "not_found",
        StatusCode::NOT_FOUND,
        ControlError::NotFound(SAMPLE_MESSAGE.to_string())
    ),
    ControlError::Conflict(_) => (
        "conflict",
        StatusCode::CONFLICT,
        ControlError::Conflict(SAMPLE_MESSAGE.to_string())
    ),
    ControlError::Unauthorized(_) => (
        "unauthorized",
        StatusCode::UNAUTHORIZED,
        ControlError::Unauthorized(SAMPLE_MESSAGE.to_string())
    ),
    ControlError::ArmingTimeout { .. } => (
        "arming_timeout",
        StatusCode::REQUEST_TIMEOUT,
        ControlError::ArmingTimeout { detail: SAMPLE_MESSAGE.to_string() }
    ),
    ControlError::PreflightFailed(_) => (
        "preflight_failed",
        StatusCode::UNPROCESSABLE_ENTITY,
        ControlError::PreflightFailed(SAMPLE_MESSAGE.to_string())
    ),
    ControlError::UnsupportedConnector(_) => (
        "unsupported_connector",
        StatusCode::UNPROCESSABLE_ENTITY,
        ControlError::UnsupportedConnector(SAMPLE_MESSAGE.to_string())
    ),
    ControlError::Internal(_) => (
        "internal",
        StatusCode::INTERNAL_SERVER_ERROR,
        ControlError::Internal(SAMPLE_MESSAGE.to_string())
    ),
    ControlError::Malformed(_) => (
        "malformed",
        StatusCode::BAD_REQUEST,
        ControlError::Malformed(SAMPLE_MESSAGE.to_string())
    ),
    // 426, not 501. `unsupported` is the code the version gate carries, and a code that
    // named one status in this table while the gate answered another would give a client that
    // reads the code and a client that reads the status two different conclusions about
    // one refusal. `unsupported` maps to `426` and to nothing else; no code maps to
    // `501` and this build never sends one.
    ControlError::Unsupported(_) => (
        "unsupported",
        StatusCode::UPGRADE_REQUIRED,
        ControlError::Unsupported(SAMPLE_MESSAGE.to_string())
    ),
    // This code belongs only to the `/process` route after the durable generation+digest
    // boundary.  A 5xx makes old clients conservative; the explicit code keeps new clients
    // conservative even if a future status mapping regresses.
    ControlError::ProcessAdmissionUncertain(_) => (
        "process_admission_uncertain",
        StatusCode::INTERNAL_SERVER_ERROR,
        ControlError::ProcessAdmissionUncertain(SAMPLE_MESSAGE.to_string())
    ),
    // Prepared is durable, so this remains retryable without becoming a definite refusal.
    ControlError::ProcessAdmissionBusy(_) => (
        "process_admission_busy",
        StatusCode::CONFLICT,
        ControlError::ProcessAdmissionBusy(SAMPLE_MESSAGE.to_string())
    ),
    // 409, the same status `conflict` carries, and the opposite meaning. The status is
    // shared because both are conflicts as HTTP means the word; the code is what tells a
    // client which one it met, and a client that retried on the status alone would spin against
    // sealed entries — see `ControlError::Busy`.
    ControlError::Busy(_) => (
        "busy",
        StatusCode::CONFLICT,
        ControlError::Busy(SAMPLE_MESSAGE.to_string())
    ),
    // The v5 local finish ran out of its budget: the entry is open and the call is not retried,
    // which is what separates it from `busy` on the same status.
    ControlError::FinishDeadline(_) => (
        "finish_deadline",
        StatusCode::CONFLICT,
        ControlError::FinishDeadline(SAMPLE_MESSAGE.to_string())
    ),
}

/// A `DaemonError` → the contract's error, and one value of every variant — again from one
/// list of rows, for the reason [`control_error_table`] gives: the generated `match` names
/// every variant with no `_` arm, so a new `DaemonError` variant does not compile until it
/// has been given a code, and the row that gives it one hands [`every_daemon_error`] a
/// sample of it in the same breath.
macro_rules! daemon_error_table {
    ($( $pattern:pat => ($mapped:expr, $sample:expr) ),+ $(,)?) => {
        /// A [`DaemonError`] as the contract's error.
        ///
        /// The interesting pair is `Refused` → `Conflict` and `Busy` → `Busy`, both of
        /// which reach the wire as `409` through [`http_status_for`] and mean opposite
        /// things. `DaemonError::Refused` is the entry's own state saying no — already
        /// sealed, a lock still on disk — and a client that retries it spins forever.
        /// `DaemonError::Busy` is the entry held for the length of one pump, which the
        /// same call a moment later will not meet. A `500` for either would tell the CLI
        /// the daemon had broken; one `409` for both would tell it nothing it could act on,
        /// which is what this row exists to prevent.
        ///
        /// Everything a participant cannot act on is `Internal`. That includes `Io`, whose
        /// message carries an absolute path by construction — acceptable here because this
        /// error reaches the CLI over an authenticated route, and unacceptable on the
        /// signed status document, which is why `CycleErrors::last` is path-free and this
        /// is not.
        pub fn control_error(error: DaemonError) -> ControlError {
            match error {
                $($pattern => $mapped),+
            }
        }

        /// One value of every `DaemonError` variant. See [`every_control_error`] for why
        /// this is generated and public.
        #[cfg(any(test, feature = "test-hooks"))]
        pub fn every_daemon_error() -> Vec<DaemonError> {
            vec![$($sample),+]
        }
    };
}

daemon_error_table! {
    DaemonError::Missing(m) => (
        ControlError::NotFound(m),
        DaemonError::Missing(SAMPLE_MESSAGE.to_string())
    ),
    DaemonError::Refused(m) => (
        ControlError::Conflict(m),
        DaemonError::Refused(SAMPLE_MESSAGE.to_string())
    ),
    DaemonError::Guard(m) => (
        ControlError::Conflict(m),
        DaemonError::Guard(SAMPLE_MESSAGE.to_string())
    ),
    DaemonError::Busy(m) => (
        ControlError::Busy(m),
        DaemonError::Busy(SAMPLE_MESSAGE.to_string())
    ),
    DaemonError::FinishDeadline { stage } => (
        ControlError::FinishDeadline(stage.to_string()),
        DaemonError::FinishDeadline { stage: "workspace_walk" }
    ),
    DaemonError::Config(m) => (
        ControlError::PreflightFailed(m),
        DaemonError::Config(SAMPLE_MESSAGE.to_string())
    ),
    DaemonError::Protocol(e) => (
        ControlError::Malformed(e.to_string()),
        DaemonError::Protocol(goalcup_protocol::ProtocolError::Signature(
            SAMPLE_MESSAGE.to_string(),
        ))
    ),
    DaemonError::Json(e) => (
        ControlError::Malformed(e.to_string()),
        DaemonError::Json(<serde_json::Error as serde::de::Error>::custom(SAMPLE_MESSAGE))
    ),
    // The rest reach the CLI as `internal`: a participant cannot act on any of them, and
    // the message is what a bug report carries.
    e @ DaemonError::Io { .. } => (
        ControlError::Internal(e.to_string()),
        DaemonError::Io {
            path: SAMPLE_MESSAGE.to_string(),
            source: std::io::Error::other(SAMPLE_MESSAGE),
        }
    ),
    e @ DaemonError::Toml(_) => (
        ControlError::Internal(e.to_string()),
        DaemonError::Toml(SAMPLE_MESSAGE.to_string())
    ),
    e @ DaemonError::Export(_) => (
        ControlError::Internal(e.to_string()),
        DaemonError::Export(goalcup_protocol::export::ExportError::Protocol(
            goalcup_protocol::ProtocolError::Signature(SAMPLE_MESSAGE.to_string()),
        ))
    ),
    e @ DaemonError::Db(_) => (
        ControlError::Internal(e.to_string()),
        DaemonError::Db(SAMPLE_MESSAGE.to_string())
    ),
    e @ DaemonError::Sqlite(_) => (
        ControlError::Internal(e.to_string()),
        DaemonError::Sqlite(SAMPLE_MESSAGE.to_string())
    ),
    e @ DaemonError::Http(_) => (
        ControlError::Internal(e.to_string()),
        DaemonError::Http(SAMPLE_MESSAGE.to_string())
    ),
    e @ DaemonError::Crypto(_) => (
        ControlError::Internal(e.to_string()),
        DaemonError::Crypto(SAMPLE_MESSAGE.to_string())
    ),
    // Finalization normally records this typed outcome as a successful terminal report. If it
    // reaches the generic control mapper instead, it is an internal integration failure, not
    // a participant-retryable capture request; retain the original typed reason in diagnostics.
    e @ DaemonError::SubmissionCapture { .. } => (
        ControlError::Internal(e.to_string()),
        DaemonError::SubmissionCapture {
            reason: goalcup_protocol::submission::SubmissionFailureV1::StorageFailed,
        }
    ),
}

/// One failure on the wire: the contract's [`ErrorBody`], the status [`http_status_for`]
/// gives it, and all three of `ErrorDetail`'s fields filled.
pub struct ControlErrorResponse {
    error: ControlError,
    detail: serde_json::Value,
}

impl ControlErrorResponse {
    /// The default detail is the code again, as an object: a client that reads only
    /// `detail` still learns which refusal this was, and nothing is added that the code
    /// did not already say.
    pub fn new(error: ControlError) -> ControlErrorResponse {
        let detail = serde_json::json!({ "code": error.code() });
        ControlErrorResponse { error, detail }
    }

    /// A refusal that has something specific to say — the on-screen arming text, the
    /// version this daemon speaks — carries it here rather than in the message, so a
    /// client reads a field instead of parsing a sentence.
    pub fn with_detail(error: ControlError, detail: serde_json::Value) -> ControlErrorResponse {
        ControlErrorResponse { error, detail }
    }
}

impl From<DaemonError> for ControlErrorResponse {
    fn from(error: DaemonError) -> ControlErrorResponse {
        ControlErrorResponse::new(control_error(error))
    }
}

impl IntoResponse for ControlErrorResponse {
    fn into_response(self) -> Response {
        let status = http_status_for(&self.error);
        let body = ErrorBody {
            error: ErrorDetail {
                code: self.error.code().to_string(),
                message: self.error.to_string(),
                detail: self.detail,
            },
        };
        (status, axum::Json(body)).into_response()
    }
}
