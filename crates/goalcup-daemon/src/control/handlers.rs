use super::{ControlErrorResponse, ControlState};
use crate::registry::ProcessHandoffPhase;
use crate::status;
#[cfg(feature = "v1-compat")]
use crate::workspace;
use crate::{DAEMON_VERSION, DaemonError};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use goalcup_protocol::capture::{MAX_GOAL_BYTES, MAX_INGEST_FRAME_BYTES, check_text};
use goalcup_protocol::control::{
    CONTROL_API_VERSION, CaptureGoalRequest, CaptureLaunchRequest, CaptureSealRequest,
    CaptureWorkspaceRequest, ControlError, DaemonHealth, ErrorBody, ErrorDetail,
};
#[cfg(feature = "v1-compat")]
use goalcup_protocol::manifest::ConnectorManifest;
use goalcup_protocol::process::{
    ProcessRegistration, native_pid_is_bound_to_handle, process_start_identity, snapshot,
};
use serde::Deserialize;
use std::path::Path as FsPath;
#[cfg(feature = "v1-compat")]
use std::path::PathBuf;

// What only the V1 routes read (D42): compiled with `v1-compat` alone.
#[cfg(feature = "v1-compat")]
use crate::arming::GateState;
#[cfg(feature = "v1-compat")]
use crate::paths::PathVars;
#[cfg(feature = "v1-compat")]
use crate::registry::{Entry, EntryState, ExpectedProcessHandoff, RunKeys};
#[cfg(feature = "v1-compat")]
use crate::runtime::EntryRuntime;
#[cfg(feature = "v1-compat")]
use crate::workspace::Roots;
#[cfg(feature = "v1-compat")]
use axum::http::Uri;
#[cfg(feature = "v1-compat")]
use goalcup_platform::tracecap::EntryArtifactRoot;
#[cfg(feature = "v1-compat")]
use goalcup_protocol::control::{
    ProcessHandoffCancelInfo, ProcessHandoffCancelRequest, QUERY_TIMEOUT_MS, ROUTE_UI,
    ResumeRequest, StartSignal, WorkspaceInfo, WorkspaceOpts,
};
#[cfg(feature = "v1-compat")]
use goalcup_protocol::manifest::LaunchMode;
#[cfg(feature = "v1-compat")]
use goalcup_protocol::template::TemplateVars;
#[cfg(feature = "v1-compat")]
use std::time::{Duration, Instant};

/// A path this build serves nothing on, in the contract's shape.
///
/// The message names no path. The caller knows what it asked for, and a 404 that echoed
/// the request line back would put an attacker's own bytes into a response body.
pub(super) async fn no_such_route() -> Response {
    ControlErrorResponse::new(ControlError::NotFound("no such control route".to_string()))
        .into_response()
}

/// A V1 route in a default build (D42): the route table keeps its shape, and the answer says
/// what is missing instead of a bare 404.
///
/// It sits behind the bearer and version gates like every other route, so it answers only a
/// client that was allowed to ask. (`ROUTE_UI` is public, so its stub answers without a token.) `not_found` and not `unsupported`: `unsupported` already
/// means "the control API version differs" to every client.
#[cfg(not(feature = "v1-compat"))]
pub(super) async fn v1_route_not_built() -> Response {
    ControlErrorResponse::new(ControlError::NotFound(
        "this route belongs to the V1 transcript path, which this daemon build does not \
         include (it compiles only with the `v1-compat` cargo feature)"
            .to_string(),
    ))
    .into_response()
}

/// A default build's resume and resume-preview routes: a capture entry is refused exactly as
/// the `v1-compat` build refuses it ("a capture entry cannot be resumed"), and any other id
/// names the V1 path this build does not include.
#[cfg(not(feature = "v1-compat"))]
pub(super) async fn resume_refused(
    State(state): State<ControlState>,
    Path(id): Path<String>,
) -> Response {
    match refuse_capture_resume(&state, &id) {
        Err(e) => ControlErrorResponse::from(e).into_response(),
        Ok(()) => v1_route_not_built().await,
    }
}

/// A route that exists, under a method it does not answer.
///
/// **`405` is one of the two statuses this module does not take from the code**, and the
/// other is `gated_by_version`'s `426`. The contract has no code for
/// "wrong method"; the method decides this status, and inventing a code for it would be a
/// code the CLI's `from_code` collapses to `internal` anyway.
///
/// The code it carries is `malformed`, not `not_found`. `not_found` would tell a client "there
/// is no such entry" about a route that exists and an entry that may well — a wrong
/// conclusion a CLI acts on. `malformed` says what is true of every `405`: the request
/// line was not one this route answers. That code also names a `400` when a handler raises
/// it, and both readings are the same reading — the status is what separates a body that
/// did not parse from a method that does not exist.
pub(super) async fn wrong_method() -> Response {
    let error = ControlError::Malformed("that route does not answer this method".to_string());
    let body = ErrorBody {
        error: ErrorDetail {
            code: error.code().to_string(),
            message: error.to_string(),
            detail: serde_json::json!({ "code": error.code() }),
        },
    };
    (StatusCode::METHOD_NOT_ALLOWED, axum::Json(body)).into_response()
}

/// `GET /v0/health` — the one call `goalcup doctor` needs to answer "is the daemon there".
pub(super) async fn health(State(state): State<ControlState>) -> Response {
    let open = match state.daemon.entries() {
        Ok(entries) => entries
            .iter()
            .filter(|entry| {
                entry.sealed_at.is_none()
                    && entry.health != goalcup_protocol::events::HealthState::SealingRecoveryNeeded
            })
            .count(),
        Err(e) => return ControlErrorResponse::from(e).into_response(),
    };
    axum::Json(DaemonHealth {
        control_api_version: CONTROL_API_VERSION,
        daemon_version: DAEMON_VERSION.to_string(),
        uptime_seconds: state.started_at.elapsed().as_secs(),
        entries_open: u32::try_from(open).unwrap_or(u32::MAX),
        upload_ok: state.daemon.upload_ok(),
    })
    .into_response()
}

/// `GET /v0/entries/{id}/status` — public, and the one signed document.
///
/// The `Path` extractor percent-decodes, which is correct here and would have been wrong
/// in the layer: auth has already answered by the time this runs, so the decode below is
/// the handler's own and not a second one in front of the authentication decision.
///
/// It answers what the last publication left. Nothing signs a second copy on the way out,
/// so the page, `goalcup status` and the MCP server read the same bytes.
pub(super) async fn entry_status(
    State(state): State<ControlState>,
    Path(id): Path<String>,
) -> Response {
    match state.daemon.signed_status(&id) {
        Ok(Some(signed)) => axum::Json(signed).into_response(),
        // Not "no such entry, here is the list": this route is unauthenticated, and the
        // only thing it may confirm is what the caller already named.
        Ok(None) => ControlErrorResponse::new(ControlError::NotFound(
            "no status for that entry".to_string(),
        ))
        .into_response(),
        Err(e) => ControlErrorResponse::from(e).into_response(),
    }
}

// ------------------------------------------------------------------ the read handlers

/// `GET /v0/entries` — the rows `goalcup entries` prints.
///
/// Authenticated, unlike `status`: a directory of every workspace on this machine is not
/// something a browser with no token may enumerate. [`EntrySummary`] carries no secret, so
/// the refusal here is about what may be listed rather than about what is in a row.
///
/// [`EntrySummary`]: goalcup_protocol::control::EntrySummary
pub(super) async fn list_entries(State(state): State<ControlState>) -> Response {
    match state.daemon.entries() {
        Ok(entries) => axum::Json(entries).into_response(),
        Err(e) => ControlErrorResponse::from(e).into_response(),
    }
}

/// `GET /v0/entries/current` — the entry a bare `goalcup status` means.
///
/// `null` when there is none and `null` when there is more than one: a daemon holding two
/// live runs must not guess which one a caller with no id was asking about. That is
/// `Daemon::current_entry`'s rule and this handler adds nothing to it — `200` with a
/// `null` body rather than a `404`, because "no entry is current" is an answer and not a
/// missing route.
pub(super) async fn current_entry(State(state): State<ControlState>) -> Response {
    match state.daemon.current_entry() {
        Ok(id) => axum::Json(id).into_response(),
        Err(e) => ControlErrorResponse::from(e).into_response(),
    }
}

// --------------------------------------------------------------- the terminal handlers

/// `GET /v0/entries/{id}/resume-preview` — authenticated, read-only launch pins.
///
/// The same sealed, launchability, and lock refusals as resume apply before the daemon reads
/// a pin. The different answer is intentional: this path neither catches a tail up nor
/// clears the registered process slot, so rendering a relaunch line cannot alter the run it
/// describes.
#[cfg(feature = "v1-compat")]
pub(super) async fn resume_preview_entry(
    State(state): State<ControlState>,
    Path(id): Path<String>,
) -> Response {
    if let Err(e) = refuse_capture_resume(&state, &id).and_then(|()| resume_allowed(&state, &id)) {
        return ControlErrorResponse::from(e).into_response();
    }
    match state.daemon.resume_preview(&id) {
        Ok(info) => axum::Json(info).into_response(),
        Err(e) => ControlErrorResponse::from(e).into_response(),
    }
}

/// `POST /v0/entries/{id}/resume` — catch the tail up, then open one relaunch authority.
///
/// The lock check runs first and before `Daemon::catch_up`, so a refused resume polls
/// nothing and changes nothing. The order is the point: the answer this route gives is the
/// input to a relaunch, and a relaunch that could not take the harness's own lock is not a
/// relaunch — a codex resume that cannot take the thread-writer lock silently becomes a new
/// thread, whose second half is chained under a session nobody bound.
///
/// The body names the exact daemon-issued process generation to consume.  This makes the
/// resume CAS a one-winner operation instead of granting a second caller an ambiguous
/// replacement-process slot after a lost response.
///
/// A sealed entry is refused before either check runs. Its chain is closed, so the answer
/// this route would otherwise give is a `ResumeInfo` — receiver token and all — for a run
/// nothing can record another byte of, and `catch_up` over a store that grew after the seal
/// meets `ExportError::Sealed` and reads as a `500` about a state that is not an error.
#[cfg(feature = "v1-compat")]
pub(super) async fn resume_entry(
    State(state): State<ControlState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Err(e) = refuse_capture_resume(&state, &id) {
        return ControlErrorResponse::from(e).into_response();
    }
    let request: ResumeRequest = match parse_body(&body) {
        Ok(request) => request,
        Err(e) => return e.into_response(),
    };
    // This is a durable gate, deliberately ahead of `catch_up`: a pre-start response must
    // neither advance a cursor nor consume a generation.  The registry CAS repeats the
    // same test because a row can change after this read and before its write transaction.
    match resume_start_allowed(&state, &id, request.expected_generation) {
        Ok(entry) => entry,
        Err(e) => return ControlErrorResponse::from(e).into_response(),
    };
    if let Err(e) = resume_allowed(&state, &id) {
        return ControlErrorResponse::from(e).into_response();
    }
    // Catch-up happens before the durable CAS.  Its failure therefore leaves the current
    // generation and termination handle untouched.  The daemon holds the entry slot across
    // both operations and repeats Started/session/generation checks inside the CAS, so seal
    // and registration cannot interleave between the tail read and the one-winner transition.
    match state
        .daemon
        .catch_up_then_open_process_handoff(&id, request.expected_generation)
    {
        Ok(info) => {
            if info.process_generation == request.expected_generation
                || info.process_generation != request.expected_generation.saturating_add(1)
            {
                return ControlErrorResponse::new(ControlError::Conflict(
                    "the resume did not advance the process generation".to_string(),
                ))
                .into_response();
            }
            // The post-catch-up open is the CAS.  There is intentionally no retry or second
            // open after a lost response.
            axum::Json(info).into_response()
        }
        Err(e) => ControlErrorResponse::from(e).into_response(),
    }
}

/// The mutating resume route exists only for a committed run.  Preview intentionally does
/// not use this gate: it remains a read-only inspection of launch pins and never catches up
/// or opens process authority.
#[cfg(feature = "v1-compat")]
fn resume_start_allowed(
    state: &ControlState,
    id: &str,
    expected_generation: u64,
) -> Result<Entry, DaemonError> {
    let entry = state
        .daemon
        .registry()
        .get(id)?
        .ok_or_else(|| DaemonError::Missing(format!("entry {id}")))?;
    if entry.state != EntryState::Started || entry.session_id.as_deref().is_none_or(str::is_empty) {
        return Err(DaemonError::Guard(format!(
            "entry {id} has no committed started session, so resume cannot catch up or open a process handoff"
        )));
    }
    let accepts_generation = match entry.process_handoff_phase() {
        ProcessHandoffPhase::LegacyUnknown => {
            expected_generation == 0
                && (!matches!(
                    entry.expected_process_handoff(),
                    ExpectedProcessHandoff::Required
                ) || entry.last_process.is_some())
        }
        ProcessHandoffPhase::Registered => {
            entry.process_handoff_generation() == expected_generation
        }
        ProcessHandoffPhase::ResumeCancelled => {
            entry.process_handoff_generation() == expected_generation
        }
        ProcessHandoffPhase::InitialClosed => {
            entry.expected_process_handoff() == ExpectedProcessHandoff::NotRequired
                && entry.process_handoff_generation() == expected_generation
        }
        ProcessHandoffPhase::OpenInitial
        | ProcessHandoffPhase::OpenResume
        | ProcessHandoffPhase::Prepared => false,
    };
    if !accepts_generation {
        return Err(DaemonError::Guard(format!(
            "entry {id} has no registered process handoff at generation {expected_generation}"
        )));
    }
    Ok(entry)
}

/// Refuse exactly the entry states in which neither a real resume nor its preview can
/// yield a safe launch line. This performs only manifest, registry, and filesystem reads.
#[cfg(feature = "v1-compat")]
fn resume_allowed(state: &ControlState, id: &str) -> Result<(), DaemonError> {
    match state.daemon.is_sealed(id) {
        Err(e) => return Err(e),
        Ok(true) => {
            return Err(DaemonError::Refused(format!(
                "entry {id} is sealed: `EventBody::Seal` closed its chain, so there is no \
                 tail to catch up and nothing a relaunch could record"
            )));
        }
        Ok(false) => {}
    }
    // A persisted `NotRequired` can mean either `--attach` on a launchable connector or a
    // true `attach_only` connector. Only the loaded manifest distinguishes them. Refuse
    // the latter before catch-up mutates any capture state; `clear_process` repeats it
    // immediately before the durable promotion to `Required`.
    state.daemon.ensure_resume_can_launch(id)?;
    free_of_held_locks(state, id)
}

/// Whether every `lock_paths[]` entry this entry is known to hold is gone.
///
/// `Daemon::resume_lock_paths` answers where they are, and the rule it encodes is the
/// one this check would otherwise get wrong: the paths come from the entry's durable
/// `SealOutcome` when it has sealed, and from the connector's `lock_paths[]` resolved live
/// when it has not. Reading the daemon's in-memory seal map instead would make the whole check
/// vacuous across a restart — the map is empty in a fresh process, and a restart is exactly
/// when a participant resumes.
///
/// The refusal is [`DaemonError::Refused`], which the one table maps to `conflict` and so to
/// `409` — the CLI's cue to tell the participant which file to remove, which is why the
/// message names the path rather than counting them.
///
/// **What "held" means is [`goalcup_protocol::manifest::lock_path_is_held`]**, and it is
/// that function rather than a test written here because the CLI asks the same question
/// before it POSTs and the two must not answer differently.
#[cfg(feature = "v1-compat")]
fn free_of_held_locks(state: &ControlState, entry_id: &str) -> Result<(), DaemonError> {
    let paths = state.daemon.resume_lock_paths(entry_id)?;
    let held: Vec<&str> = paths
        .iter()
        .filter(|p| goalcup_protocol::manifest::lock_path_is_held(p))
        .map(String::as_str)
        .collect();
    match held.is_empty() {
        true => Ok(()),
        false => Err(DaemonError::Refused(format!(
            "entry {entry_id} cannot be resumed while the harness's lock is still on disk: \
             {}. A resume that cannot take that lock silently becomes a new session, so this \
             one is refused until the path is gone",
            held.join(", ")
        ))),
    }
}

/// `POST /v0/entries/{id}/seal` — the participant's early finish.
///
/// A workspace-submission entry finishes through `Daemon::finish_entry_blocking` with
/// `FinishReasonV1::Manual`; every other entry seals through `Daemon::seal_entry_blocking`,
/// which runs `Daemon::seal_entry`, the method the deadline calls. The seal a participant
/// asks for and the seal at the deadline are then the same bytes in the same order: the
/// workspace manifest, the second foreign scan, the chained `EventBody::Seal` and
/// `seal.json`, and only then the kill.
///
/// A second call is `409`. `seal_entry` refuses it under the slot lock and its
/// `DaemonError::Refused` reaches the wire as `conflict` through the one table — a seal is
/// the chain's last link, and there is no second last link.
///
/// **On a blocking worker** ([`receipt`] and [`upload`] use the same pattern for the same
/// reason). `seal_entry` walks the workspace, writes the chain and then calls
/// `terminate`, which sleeps out the handle's `grace_seconds` — up to ten on the shipped
/// connectors — whenever the harness survives its SIGTERM. Run inline, that is an `async`
/// handler holding a runtime worker for seconds: on a single-threaded runtime the status
/// page, the status route and every other request stop for the duration of a kill.
///
/// The refusal semantics are not this function's: the
/// "already sealed" check and the seal happen under one hold of the entry's slot lock
/// inside `seal_entry`, and that hold ends before the kill does — the grace is slept
/// outside it, deliberately, so one entry's ten-second termination does not hold a lock
/// every other call on that entry takes. A second seal arriving while the first is still in
/// its grace therefore takes the lock straight away, finds the entry already `Sealed` (the
/// state `EntryRuntime::seal` wrote under that same hold) and is refused by the
/// sealed-state check with `409`.
///
/// [`receipt`]: crate::receipt
/// [`upload`]: crate::upload
pub(super) async fn seal_entry(
    State(state): State<ControlState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    // The entry's row, not the body, decides which finish runs. A v5 entry needs a capture
    // seal request; a capture seal request naming any other entry is refused, and any other
    // body a legacy seal carries is ignored as it always was. Both refusals come first.
    let capture = match is_capture_row(&state, &id) {
        Ok(capture) => capture,
        Err(error) => return ControlErrorResponse::from(error).into_response(),
    };
    let request = serde_json::from_slice::<CaptureSealRequest>(&body).ok();
    let refused = match (capture, &request) {
        (true, None) => Some("a v5 capture entry is sealed with a capture seal request"),
        (false, Some(_)) => Some("a capture seal request names an entry that is not a v5 capture"),
        _ => None,
    };
    if let Some(refused) = refused {
        return ControlErrorResponse::new(ControlError::Malformed(refused.to_string()))
            .into_response();
    }
    if let Some(request) = request {
        return match state.daemon.finish_capture(&id, &request).await {
            Ok(report) => axum::Json(report).into_response(),
            Err(e) => ControlErrorResponse::from(e).into_response(),
        };
    }
    // `seal_entry_blocking` and not a bare `spawn_blocking`: the worker holds one of the
    // daemon's blocking permits for the length of the seal, which is what `Daemon::stop`
    // waits to take back. Spawned here on its own, `stop` would return over a seal in
    // progress, releasing nothing — the worker holds an `Arc<Daemon>`, so the registry and
    // buffer locks would outlive the stop that was meant to release them.
    seal_v1_entry(&state, &id).await
}

/// A default build seals local captures only (D42). Any other id is an entry this daemon does
/// not hold, which is what the V1 runtime map would also have said.
#[cfg(not(feature = "v1-compat"))]
async fn seal_v1_entry(_state: &ControlState, id: &str) -> Response {
    ControlErrorResponse::from(DaemonError::Missing(format!(
        "no entry {id} in this daemon"
    )))
    .into_response()
}

/// The V1 half of [`seal_entry`]: a workspace-submission finish or the legacy seal.
#[cfg(feature = "v1-compat")]
async fn seal_v1_entry(state: &ControlState, id: &str) -> Response {
    let id = id.to_string();
    let workspace = match workspace_finalization_route(state.daemon.registry().get(&id)) {
        Ok(workspace) => workspace,
        Err(error) => return ControlErrorResponse::from(error).into_response(),
    };
    let sealed = if workspace {
        state
            .daemon
            .finish_entry_blocking(&id, goalcup_protocol::submission::FinishReasonV1::Manual)
            .await
            .map(|result| result.map(goalcup_protocol::control::SealResponse::Workspace))
    } else {
        state
            .daemon
            .seal_entry_blocking(&id)
            .await
            .map(|result| result.map(goalcup_protocol::control::SealResponse::Legacy))
    };
    match sealed {
        Ok(Ok(response)) => axum::Json(response).into_response(),
        Ok(Err(e)) => ControlErrorResponse::from(e).into_response(),
        // The worker itself did not finish — a panic in it, or a runtime shutting down.
        // Neither is a state the participant can act on, so it reaches the CLI as
        // `internal` carrying what a bug report needs.
        Err(e) => ControlErrorResponse::new(ControlError::Internal(format!(
            "the seal did not run to completion: {e}"
        )))
        .into_response(),
    }
}

#[cfg(feature = "v1-compat")]
fn workspace_finalization_route(
    entry: Result<Option<Entry>, DaemonError>,
) -> Result<bool, DaemonError> {
    Ok(entry?.is_some_and(|entry| {
        matches!(
            entry.receipt_binding(),
            crate::config::ReceiptBindingPolicy::WorkspaceSubmission { .. }
        )
    }))
}

// ------------------------------------------------------------------ the entry handlers
//
// Each route has the same shape: resolve, call a function, map the failure through the
// one table above. Nothing here interprets a rule.

/// The most a caller may hold the start-signal long poll open for.
///
/// A bound rather than a refusal: a client asking for longer gets this, because the number
/// it asked for is advice about how long it is willing to wait and this is how long the
/// daemon is willing to hold a connection. Sixty seconds is well inside every connector's
/// `arming.timeout_seconds` (Claude 120, Codex 300), so a CLI that polls in a
/// loop still meets the gate's own refusal rather than this one.
///
/// **Defined in `goalcup_protocol::control` and re-exported here**: the CLI sizes its polls
/// against this number, and a bound only one side can
/// see is a bound the other side reads as the arming window closing.
#[cfg(feature = "v1-compat")]
pub use goalcup_protocol::control::MAX_START_SIGNAL_WAIT_MS;

/// How often the long poll asks the gate. The gate moves on a tick, so
/// there is nothing to await on directly; 25 ms is under a participant's perception and
/// costs one lock acquisition each time.
#[cfg(feature = "v1-compat")]
const START_SIGNAL_POLL_MS: u64 = 25;

/// `POST /v0/entries/{id}/abandon`'s body, which is the only request shape this module
/// defines. `deny_unknown_fields`, like every document this daemon parses, and `default` so
/// an empty body is a valid abandon — the CLI usually has nothing to add.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct AbandonRequest {
    /// What the CLI is abandoning for, recorded in the refusal a started entry answers with.
    pub reason: Option<String>,
}

/// A request body as one of the contract's types, or the contract's `400`.
///
/// `axum::Json` is deliberately not used as an extractor: its rejection is axum's own body
/// shape, and a client that parses every failure as an `ErrorBody` would read "the daemon
/// said nothing" — the same defect [`wrong_method`] exists to close.
fn parse_body<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ControlErrorResponse> {
    serde_json::from_slice(body).map_err(|e| {
        ControlErrorResponse::new(ControlError::Malformed(format!(
            "the request body is not the document this route takes: {e}"
        )))
    })
}

/// [`parse_body`] where an empty body means the type's default.
fn parse_optional_body<T: serde::de::DeserializeOwned + Default>(
    body: &Bytes,
) -> Result<T, ControlErrorResponse> {
    match body.iter().all(u8::is_ascii_whitespace) {
        true => Ok(T::default()),
        false => parse_body(body),
    }
}

/// `POST /v0/workspaces` — the whole call: prepare the workspace, register the entry, and
/// answer with everything the CLI needs to launch.
///
/// `harness` is dead on the wire. The connector comes from `opts.connector_id` and from
/// nothing else; the client trait still carries the parameter, and this daemon never reads it.
#[cfg(feature = "v1-compat")]
pub(super) async fn create_workspace(State(state): State<ControlState>, body: Bytes) -> Response {
    let opts: WorkspaceOpts = match parse_body(&body) {
        Ok(opts) => opts,
        Err(e) => return e.into_response(),
    };
    match prepared_workspace(&state, &opts) {
        Ok(info) => axum::Json(info).into_response(),
        Err(e) => e.into_response(),
    }
}

/// `POST /v0/capture-workspaces` — a v5 local capture entry: the profile's connector, the
/// workspace, the reserved chain and the ingest authorization. No server is asked anything.
pub(super) async fn create_capture_workspace(
    State(state): State<ControlState>,
    body: Bytes,
) -> Response {
    let request: CaptureWorkspaceRequest = match parse_body(&body) {
        Ok(request) => request,
        Err(e) => return e.into_response(),
    };
    for (field, path) in [
        ("workspace", &request.workspace),
        ("run_home", &request.run_home),
        ("config_home", &request.config_home),
    ] {
        if !FsPath::new(path).is_absolute() {
            return ControlErrorResponse::new(ControlError::Malformed(format!(
                "{field} {path:?} is not an absolute path"
            )))
            .into_response();
        }
    }
    let (loaded, digests, every_loaded_real_dir) =
        match capture_profile(&state, &request.profile_id) {
            Ok(profile) => profile,
            Err(DaemonError::Missing(_)) => {
                return ControlErrorResponse::new(ControlError::UnsupportedConnector(format!(
                    "this daemon ships no connector {:?}",
                    request.profile_id
                )))
                .into_response();
            }
            Err(e) => return ControlErrorResponse::from(e).into_response(),
        };
    if request.connector_dir_sha256 != digests.connector_dir_sha256 {
        return ControlErrorResponse::new(ControlError::PreflightFailed(
            "the launcher and daemon loaded different connector directory bytes".into(),
        ))
        .into_response();
    }
    match state.daemon.create_capture(
        mint_entry_id(),
        FsPath::new(&request.workspace),
        FsPath::new(&request.run_home),
        FsPath::new(&request.config_home),
        &loaded.manifest,
        &digests,
        &every_loaded_real_dir,
        request.reserve_bytes,
    ) {
        Ok(info) => axum::Json(info).into_response(),
        Err(e) => ControlErrorResponse::from(e).into_response(),
    }
}

/// The schema-1 harness whose declared id is `profile_id`, both digests from that authoritative
/// loaded connector, and the deduplicated real-home refusal set from the same all-harness scan.
fn capture_profile(
    state: &ControlState,
    profile_id: &str,
) -> Result<
    (
        goalcup_protocol::harness::LoadedConnector,
        crate::runtime::ProfileDigests,
        Vec<String>,
    ),
    DaemonError,
> {
    crate::runtime::capture_harness(state.daemon.config(), profile_id)
}

/// `POST /v0/entries/{id}/capture/goal` — the goal, once. The body is bounded before it is
/// decoded, and no refusal on this route repeats a byte of it.
pub(super) async fn record_capture_goal(
    State(state): State<ControlState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let refuse = |message: String| {
        ControlErrorResponse::new(ControlError::Malformed(message)).into_response()
    };
    if body.len() > MAX_INGEST_FRAME_BYTES {
        return refuse(format!(
            "a goal request is at most {MAX_INGEST_FRAME_BYTES} bytes"
        ));
    }
    let Ok(request) = serde_json::from_slice::<CaptureGoalRequest>(&body) else {
        return refuse("the request body is not a capture goal request".to_string());
    };
    if request.bytes.is_empty() || request.bytes.len() > MAX_GOAL_BYTES {
        return refuse(format!("a goal is 1 to {MAX_GOAL_BYTES} bytes"));
    }
    if std::str::from_utf8(&request.bytes).is_err() {
        return refuse("a goal is UTF-8 text".to_string());
    }
    match state.daemon.record_capture_goal(&id, &request.bytes) {
        Ok(key) => axum::Json(key).into_response(),
        Err(e) => ControlErrorResponse::from(e).into_response(),
    }
}

/// `POST /v0/entries/{id}/capture/launch` — the `Launch` record, after the harness is
/// spawned. The two strings and the spawn observation are checked against the chain's
/// record bounds before anything is appended.
pub(super) async fn record_capture_launch(
    State(state): State<ControlState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let request: CaptureLaunchRequest = match parse_body(&body) {
        Ok(request) => request,
        Err(e) => return e.into_response(),
    };
    let checked = request
        .spawned_at
        .validate()
        .and(check_text("harness_version", &request.harness_version))
        .and(check_text("continuation", &request.continuation));
    if let Err(error) = checked {
        return ControlErrorResponse::new(ControlError::Malformed(format!("launch {error}")))
            .into_response();
    }
    match state.daemon.record_capture_launch(&id, &request) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => ControlErrorResponse::from(e).into_response(),
    }
}

/// A v5 capture entry cannot resume in this release: it is sealed instead.
fn refuse_capture_resume(state: &ControlState, id: &str) -> Result<(), DaemonError> {
    if is_capture_row(state, id)? {
        return Err(DaemonError::Refused(format!(
            "entry {id} is a v5 capture entry, and a capture entry cannot be resumed: seal it \
             instead"
        )));
    }
    Ok(())
}

/// Whether `id`'s registry row is a v5 capture entry's.
fn is_capture_row(state: &ControlState, id: &str) -> Result<bool, DaemonError> {
    Ok(state
        .daemon
        .registry()
        .get(id)?
        .is_some_and(|row| row.capture.is_some()))
}

/// `POST /v0/entries/{id}/arm` — `Daemon::arm`, which runs the pre-arm scan first.
///
/// The scan's refusal is `DaemonError::Refused`, which the table maps to `conflict` and so
/// to `409`: a home that already holds a tracked session is a state the participant can act
/// on, not a broken daemon.
///
/// A started entry is the other `409`, raised inside [`EntryRuntime::arm`]: a second arm
/// re-stamps `armed_at` and replaces the gate, which a signed receipt has already fixed. A
/// re-arm before the start signal is still `204` — that launch never took.
#[cfg(feature = "v1-compat")]
pub(super) async fn arm_entry(
    State(state): State<ControlState>,
    Path(id): Path<String>,
) -> Response {
    if let Err(e) = state.daemon.require_live_capture_mutation_authority(&id) {
        return ControlErrorResponse::from(e).into_response();
    }
    match state.daemon.arm(&id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => ControlErrorResponse::from(e).into_response(),
    }
}

/// `POST /v0/entries/{id}/process` — record the handle the CLI's launch produced.
///
/// **The termination authorities validated here are `pgid` and `native_pid`.** A launch sets
/// `pgid` only when it called
/// `setpgid`, which makes the new group the leader's own pid; any other number names a
/// process group this daemon did not create and would signal at the seal. So `None` and
/// `Some(pid)` are the two honest spellings and everything else is `400`.
///
/// A native PID is only a launch-time hint.  When it is present, a snapshot taken for this
/// request must show that PID as the registered root or one of its descendants, running the
/// manifest-pinned native identity.  An unrelated same-name binary is never a valid handoff:
/// accepting it would turn a stale PID into a later seal target.
///
/// An empty `argv` is not refused. `terminate`'s guard has two halves — the pid's age and
/// the program `ps` names among its arguments — and a handle with no argv leaves only the
/// first; the tree already carries that case as `SealReport::guard_degraded`, which reports
/// the weaker guard rather than pretending the launch never happened.
///
/// **A second registration is a `409` while a handle stands**, raised inside
/// `Daemon::register_process`: the handle is what `harness_alive` measures and what the
/// seal terminates, so a decoy posted on the loopback would redirect both. A resume is what
/// clears it — see [`resume_entry`] — and the relaunched harness then registers once.
pub(super) async fn register_process(
    State(state): State<ControlState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let registration: ProcessRegistration = match parse_body(&body) {
        Ok(registration) => registration,
        Err(e) => return e.into_response(),
    };
    if let Err(e) = state.daemon.require_live_capture_mutation_authority(&id) {
        return ControlErrorResponse::from(e).into_response();
    }
    // A historical exact retry is an acknowledgement, not a fresh request to grant PID
    // authority. Its v1 canonical digest intentionally omits later optional fields, so it
    // must reach the registry's idempotency path before new-handoff checks reject those
    // absent fields. Only Prepared/Registered exact digests qualify; an Open handoff always
    // proves current identity below.
    let exact_accepted_replay = match state
        .daemon
        .registry()
        .exact_accepted_process_replay_phase(&id, &registration)
    {
        Ok(replay) => replay,
        Err(e) => return uncertain_process_admission(e).into_response(),
    };
    if let Some(phase) = exact_accepted_replay {
        if phase == ProcessHandoffPhase::Registered {
            // A live runtime does not need to be acquired for an idempotent registry ACK;
            // a concurrent pump hold must not turn the lost-204 retry ambiguous. If there
            // is no runtime, only a sealed row may acknowledge without monitoring.
            if state.daemon.has_entry_runtime(&id) {
                return StatusCode::NO_CONTENT.into_response();
            }
            return match state
                .daemon
                .acknowledge_evicted_sealed_process_replay(&id, &registration)
            {
                Ok(()) => StatusCode::NO_CONTENT.into_response(),
                Err(e) => uncertain_process_admission(e).into_response(),
            };
        }
        return match state.daemon.register_process(&id, registration.clone()) {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            // Prepared Busy is the one transient post-mutation result whose stable wire
            // identity is itself useful: the client keeps retrying the immutable request.
            Err(e @ DaemonError::Busy(_)) if phase == ProcessHandoffPhase::Prepared => {
                ControlErrorResponse::new(ControlError::ProcessAdmissionBusy(e.to_string()))
                    .into_response()
            }
            Err(e) => uncertain_process_admission(e).into_response(),
        };
    }
    // `native_binary` is the entry's manifest-pinned binary selection, not a client label.
    // For an authenticated administrative caller, this keeps a handoff from relabelling a
    // native hint as a different manifest binary that a later seal should prefer. It is not
    // root-process provenance and it does not defend against an administrative bearer that
    // deliberately nominates a different live process; that is the control-token boundary.
    let expected_native_binary = match state.daemon.native_binary_for_process(&id) {
        Ok(native_binary) => native_binary,
        // The only runtime-less case that can acknowledge a process request is an exact
        // replay for a sealed, evicted entry. Its durable canonical digest was already
        // admitted against the manifest, and the registry helper below rejects every other
        // body; do not require a gone process snapshot just to replay its lost 204.
        Err(DaemonError::Missing(_)) => {
            return match state
                .daemon
                .acknowledge_evicted_sealed_process_replay(&id, &registration)
            {
                Ok(()) => StatusCode::NO_CONTENT.into_response(),
                Err(e) => ControlErrorResponse::from(e).into_response(),
            };
        }
        Err(e) => return ControlErrorResponse::from(e).into_response(),
    };
    if registration.handle.native_binary != expected_native_binary {
        return ControlErrorResponse::new(ControlError::Malformed(format!(
            "native_binary {:?} does not match this entry's manifest-pinned identity {:?}",
            registration.handle.native_binary, expected_native_binary
        )))
        .into_response();
    }
    // A PID and argv are not a process instance.  Require the launch-time creation
    // identity to match *now* before durable authority is admitted.  A missing identity
    // from an old row remains readable for exact sealed-replay compatibility, but cannot
    // become a live signal authority through this control route.
    let observed_process_start = process_start_identity(registration.handle.pid);
    if registration.handle.process_start.is_none()
        || observed_process_start.is_none()
        || registration.handle.process_start != observed_process_start
    {
        return ControlErrorResponse::new(ControlError::Malformed(format!(
            "process_start {:?} does not identify the live registered pid {}; a recycled pid is not valid termination authority",
            registration.handle.process_start, registration.handle.pid
        )))
        .into_response();
    }
    if registration
        .handle
        .pgid
        .is_some_and(|pgid| pgid != registration.handle.pid)
    {
        return ControlErrorResponse::new(ControlError::Malformed(format!(
            "pgid {:?} is neither absent nor this process's own pid {}: a launch sets pgid \
             only when it called setpgid, and a group this daemon did not create is not a \
             group it may signal",
            registration.handle.pgid, registration.handle.pid
        )))
        .into_response();
    }
    if registration.handle.native_pid.is_some() {
        let table = match snapshot() {
            Ok(table) => table,
            Err(error) => {
                return ControlErrorResponse::new(ControlError::Internal(format!(
                    "cannot validate native_pid against the current process snapshot: {error}"
                )))
                .into_response();
            }
        };
        if !native_pid_is_bound_to_handle(&registration.handle, &table) {
            return ControlErrorResponse::new(ControlError::Malformed(format!(
                "native_pid {:?} is not the registered root or a descendant running {:?}; \
                 an unrelated process is not valid termination authority",
                registration.handle.native_pid, registration.handle.native_binary
            )))
            .into_response();
        }
        let observed_native_process_start = registration
            .handle
            .native_pid
            .and_then(process_start_identity);
        if registration.handle.native_process_start.is_none()
            || observed_native_process_start.is_none()
            || registration.handle.native_process_start != observed_native_process_start
        {
            return ControlErrorResponse::new(ControlError::Malformed(format!(
                "native_process_start {:?} does not identify native_pid {:?}; a recycled native pid is not valid termination authority",
                registration.handle.native_process_start, registration.handle.native_pid
            )))
            .into_response();
        }
    }
    match state.daemon.register_process(&id, registration.clone()) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        // A process response can be lost after the durable registration, then the run can
        // seal and be evicted before the CLI retries.  There is no runtime left to replay
        // trace/buffer work, but the sealed registry row can acknowledge only the same
        // already-accepted digest.  It never reopens authority or accepts a new handle.
        Err(DaemonError::Missing(_)) => match state
            .daemon
            .acknowledge_evicted_sealed_process_replay(&id, &registration)
        {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => ControlErrorResponse::from(e).into_response(),
        },
        Err(e) => {
            match state
                .daemon
                .registry()
                .is_exact_accepted_process_replay(&id, &registration)
            {
                Ok(true) => uncertain_process_admission(e).into_response(),
                Ok(false) => ControlErrorResponse::from(e).into_response(),
                Err(check) => uncertain_process_admission(DaemonError::Refused(format!(
                    "{e}; the daemon could not prove the exact durable handoff absent after the error: {check}"
                )))
                .into_response(),
            }
        }
    }
}

/// Preserve the route's durable admission boundary on the wire.  The original daemon
/// variant is deliberately carried only as text: after Prepared, `Refused` and `Json` no
/// longer mean what their ordinary 409/400 mappings mean to a process-owning client.
fn uncertain_process_admission(error: DaemonError) -> ControlErrorResponse {
    ControlErrorResponse::new(ControlError::ProcessAdmissionUncertain(error.to_string()))
}

/// `POST /v0/entries/{id}/process/cancel` — return one issued resume permit before a
/// replacement process has been accepted.  The entry-wide control bearer is deliberately
/// the capability: a second resume never received a successful permit, while an operator
/// holding this bearer may recover a local CLI crash without a nonce that must itself be
/// persisted and recovered.
#[cfg(feature = "v1-compat")]
pub(super) async fn cancel_process_handoff(
    State(state): State<ControlState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Err(e) = state.daemon.require_live_capture_mutation_authority(&id) {
        return ControlErrorResponse::from(e).into_response();
    }
    let request: ProcessHandoffCancelRequest = match parse_body(&body) {
        Ok(request) => request,
        Err(e) => return e.into_response(),
    };
    match state.daemon.cancel_resume_process_handoff(
        &id,
        request.expected_generation,
        request.request_digest.as_deref(),
    ) {
        Ok(entry) => axum::Json(ProcessHandoffCancelInfo {
            process_generation: entry.process_handoff_generation(),
        })
        .into_response(),
        Err(e) => ControlErrorResponse::from(e).into_response(),
    }
}

/// `POST /v0/entries/{id}/abandon` — failure before dispatch leaves no entry.
///
/// `Daemon::abandon` drops the entry and its registry row, and refuses once `started_at`
/// is stamped. This handler adds the half that makes "leaves no entry" true of the disk as
/// well: the entry's artifact directory and its signed status cache.
///
/// **Both removals are inside the entry's own roots and are checked to be.** The artifact
/// directory has to sit under the daemon's configured `artifact_root` and the cache file
/// has to be a plain file name under `config_dir/cache`, or nothing is removed. The
/// workspace is deliberately left alone: it is the participant's directory, that rule
/// is about the entry, and a daemon that deleted a directory a participant had been working
/// in would be the worst possible reading of "abandon".
///
/// Order matters. `Daemon::abandon` removes the artifact tree through its retained entry
/// authority before deleting the registry row; a cleanup failure is therefore an error and
/// never a successful abandon. This handler removes only cache views after that succeeds.
pub(super) async fn abandon_entry(
    State(state): State<ControlState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let request: AbandonRequest = match parse_optional_body(&body) {
        Ok(request) => request,
        Err(e) => return e.into_response(),
    };
    let reason = request
        .reason
        .unwrap_or_else(|| "the CLI abandoned the run before it started".to_string());
    if let Err(e) = state.daemon.abandon(&id, &reason) {
        return ControlErrorResponse::from(e).into_response();
    }
    remove_entry_files(&state, &id);
    StatusCode::NO_CONTENT.into_response()
}

/// The cache views an abandoned entry leaves behind. Artifact cleanup is not best effort:
/// it already completed through the runtime's retained capability before this is called.
/// Cache removal remains best effort because these are derived, signed views and the entry
/// is already gone from authoritative registry and runtime state.
fn remove_entry_files(state: &ControlState, entry_id: &str) {
    let config = state.daemon.config();
    // `status::mirror` writes `<config_dir>/cache/status-<entry_id>.json`, and the same
    // check it makes before writing is made here before removing: an id that is not a plain
    // file name names no cache file this daemon wrote.
    if FsPath::new(entry_id).file_name() == Some(std::ffi::OsStr::new(entry_id)) {
        let _ = std::fs::remove_file(
            config
                .config_dir
                .join("cache")
                .join(format!("status-{entry_id}.json")),
        );
    }
    // `status-latest.json` is the shared pointer — the one document `goalcup-mcp` reads
    // when it has not been told which entry it wants — and `mirror` may last have written
    // it for the entry that has just gone.
    //
    // The document is parsed and its `entry_id` compared exactly. A substring test over the
    // raw text answers "yes" for every id the pointer merely contains: `e1` is a substring
    // of `e10`, and of a workspace path, and of the detail line — so abandoning `e1` would
    // delete a pointer that named a live run, and the MCP server would then have nothing to
    // read at all.
    //
    // Removed first and refreshed after: the pointer belongs to the most recently started
    // entry, and a refresh that ran while a departed entry still held it would leave the
    // survivor's write skipped and then delete the pointer from under it.
    let _ = status::remove_latest_if_owned(entry_id, &config.config_dir);
    // Whatever is still open re-claims the pointer; when nothing is, it stays removed
    // rather than answering, signed and whole, for a run that no longer exists.
    #[cfg(feature = "v1-compat")]
    let _ = state.daemon.refresh_statuses();
}

/// `GET /v0/entries/{id}/start-signal?timeout_ms=…` — the long poll the on-screen arming text and
/// the CLI both read.
///
/// It answers the observed signal, or `408` carrying [`EntryRuntime::on_screen`] in the
/// detail — the same words the status page and the status line show, because a participant
/// looking at one and a CLI printing the other must not be told two different things about
/// one wait.
#[cfg(feature = "v1-compat")]
pub(super) async fn wait_start_signal(
    State(state): State<ControlState>,
    Path(id): Path<String>,
    uri: Uri,
) -> Response {
    let wait_ms = match requested_timeout_ms(uri.query()) {
        Ok(ms) => ms.min(MAX_START_SIGNAL_WAIT_MS),
        Err(e) => return e.into_response(),
    };
    let deadline = Instant::now() + Duration::from_millis(wait_ms);
    loop {
        // The first call is also the entry probe: `arming_of` reaches the entry map, so an
        // id this daemon does not hold is `404` before anything waits on it.
        match state.daemon.arming_of(&id) {
            Err(e) => return ControlErrorResponse::from(e).into_response(),
            Ok(Some((GateState::StartSignalSeen | GateState::Started, at))) => {
                return observed_signal(&state, &id, at);
            }
            // A gate that has already refused: `arming_of` maps `GateState::Refused` to
            // `None`, and it is the only state it does that for. Answer now. A
            // settled refusal will never become a signal, so sleeping out the rest of the
            // slice — up to `MAX_START_SIGNAL_WAIT_MS` — would make a participant wait a minute
            // for an answer the daemon already has. `ArmingState::Refused` lets a machine reader
            // tell "will never arm" from "still pending"; treating the two alike here would throw
            // that away.
            Ok(None) => break,
            // `Armed`: the signal is not in yet, which is what the wait is for.
            Ok(_) => {}
        }
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let left = deadline.saturating_duration_since(now);
        tokio::time::sleep(left.min(Duration::from_millis(START_SIGNAL_POLL_MS))).await;
    }
    arming_timeout(&state, &id)
}

/// The signal, as the contract's [`StartSignal`], read off the gate rather than rebuilt.
#[cfg(feature = "v1-compat")]
fn observed_signal(state: &ControlState, entry_id: &str, at: Option<f64>) -> Response {
    let built = (|| {
        Ok::<_, DaemonError>(StartSignal {
            entry_id: entry_id.to_string(),
            signal: state.daemon.arming_signal(entry_id)?,
            session_id: state.daemon.session_id(entry_id)?,
            observed_at: crate::unix_to_rfc3339(at.unwrap_or_else(crate::now_unix)),
            source: state.daemon.start_signal_source(entry_id)?,
        })
    })();
    match built {
        Ok(signal) => axum::Json(signal).into_response(),
        Err(e) => ControlErrorResponse::from(e).into_response(),
    }
}

/// `408`, in the gate's own words.
#[cfg(feature = "v1-compat")]
fn arming_timeout(state: &ControlState, entry_id: &str) -> Response {
    let on_screen = match state.daemon.on_screen(entry_id) {
        Ok(text) => text,
        Err(e) => return ControlErrorResponse::from(e).into_response(),
    };
    ControlErrorResponse::with_detail(
        ControlError::ArmingTimeout {
            detail: on_screen.clone(),
        },
        serde_json::json!({ "on_screen": on_screen }),
    )
    .into_response()
}

/// The long poll's one parameter, parsed by hand.
///
/// By hand because axum's `Query` extractor is behind a feature this crate does not enable
/// — `axum` is `default-features = false` here so that none of its optional extractors and,
/// above all, none of its `tracing` integration is linked. One parameter does not need a
/// deserializer, and doing it here is what lets an unknown key be a `400` in the contract's
/// shape rather than a value silently ignored: a caller that asked for something this route
/// does not do must not be answered as if it had asked for nothing.
#[cfg(feature = "v1-compat")]
fn requested_timeout_ms(query: Option<&str>) -> Result<u64, ControlErrorResponse> {
    let malformed = |message: String| ControlErrorResponse::new(ControlError::Malformed(message));
    let Some(query) = query.filter(|q| !q.is_empty()) else {
        // No parameter is one look at the gate and an immediate answer, which is what a
        // caller that wants to know now rather than wait means.
        return Ok(0);
    };
    let mut wait_ms = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key != QUERY_TIMEOUT_MS {
            return Err(malformed(format!(
                "{key:?} is not a parameter this route takes; the only one is \
                 {QUERY_TIMEOUT_MS:?}"
            )));
        }
        let parsed: u64 = value.parse().map_err(|_| {
            malformed(format!(
                "{QUERY_TIMEOUT_MS}={value:?} is not a number of milliseconds"
            ))
        })?;
        if wait_ms.replace(parsed).is_some() {
            return Err(malformed(format!(
                "{QUERY_TIMEOUT_MS} was given twice, and this route does not choose between \
                 two waits a caller asked for"
            )));
        }
    }
    Ok(wait_ms.unwrap_or(0))
}

// ------------------------------------------------------ the workspace, end to end

/// A connector this build ships, re-read from disk, with its directory and the bytes of its
/// `manifest.toml`. The map was loaded once at startup, so a connector edited underneath a
/// running daemon would otherwise be refused with a message about a directory that was there
/// the whole time; this is the one moment a manifest is chosen, and it is a few small files.
///
/// `422 unsupported_connector` and not `404`: the route exists and the request was well
/// formed; this daemon simply has no such manifest. The directory is found by the manifest's
/// declared id rather than assumed from `connector_id`, so a connector id is never joined
/// onto a path.
#[cfg(feature = "v1-compat")]
fn shipped_manifest(
    state: &ControlState,
    connector_id: &str,
) -> Result<(ConnectorManifest, PathBuf, Vec<u8>), ControlErrorResponse> {
    let shipped = state
        .daemon
        .reload_manifests()
        .map_err(ControlErrorResponse::from)?;
    let manifest = shipped
        .values()
        .find(|m| m.id == connector_id)
        .cloned()
        .ok_or_else(|| {
            ControlErrorResponse::new(ControlError::UnsupportedConnector(format!(
                "this daemon ships no connector {connector_id:?}"
            )))
        })?;
    let (connector_dir, manifest_bytes) =
        shipped_connector(&state.daemon.config().connectors_dir, &manifest.id)
            .map_err(ControlErrorResponse::from)?;
    Ok((manifest, connector_dir, manifest_bytes))
}

/// The connector directory's digest, compared against the release allowlist. Two readings,
/// and only one of them refuses:
///
///   * this build pins the connector and the bytes differ — a shipped connector whose files
///     moved. Refused before the first write, because the daemon holds the pin and can say
///     so; letting the run start would mean recording a capture under bytes this release
///     never shipped.
///   * this build pins no directory for the id — a connector being developed, or a
///     `connectors_dir` pointing outside the release. Admitted, silently, in exactly the shape
///     `DaemonConfig::server_public_key` describes for the placeholder key: refusing would make
///     developing a connector impossible. The digest still travels onto a start receipt, and
///     V13 reports it as `connector_dir_unpinned` — a Fail unless the verdict was taken with
///     `--dev`.
///
/// `why` completes the refusal for the caller's reader.
#[cfg(feature = "v1-compat")]
fn pinned_connector_dir(
    manifest_id: &str,
    connector_dir: &FsPath,
    why: &str,
) -> Result<String, ControlErrorResponse> {
    let digest =
        goalcup_protocol::connector_dir::connector_dir_sha256(connector_dir).map_err(|e| {
            ControlErrorResponse::new(ControlError::PreflightFailed(format!(
                "the {manifest_id} connector directory this daemon ships cannot be hashed: {e}"
            )))
        })?;
    if let Some(pinned) = goalcup_protocol::pins::pinned_connector_dir_sha256(manifest_id)
        && !goalcup_protocol::pins::connector_dir_is_pinned(manifest_id, &digest)
    {
        return Err(ControlErrorResponse::with_detail(
            ControlError::PreflightFailed(format!(
                "the {manifest_id} connector directory this daemon ships is not the one this \
                 build pins{why}"
            )),
            serde_json::json!({ "daemon": digest, "pinned": pinned }),
        ));
    }
    Ok(digest)
}

/// Everything `POST /v0/workspaces` does, off the request thread's `async` frame.
///
/// The order is the one `workspace::prepare` enforces again from the
/// inside: **every refusal before the first write**. The connector, the manifest digest and
/// the roots are settled here; `prepare` re-asks `check_workspace_available` before it
/// renders anything, because the first thing it would write is the receiver token into a
/// file under the artifact root, and a run refused after that point would have left a live
/// token on disk.
#[cfg(feature = "v1-compat")]
fn prepared_workspace(
    state: &ControlState,
    opts: &WorkspaceOpts,
) -> Result<WorkspaceInfo, ControlErrorResponse> {
    let daemon = &state.daemon;
    let config = daemon.config();
    // Legacy entry construction owns service clients. Refuse before manifest reads,
    // workspace preparation, tracker-file rendering or registry insertion.
    config.require_service()?;

    // 1. A connector this build ships, and 2. the one the CLI loaded is the one in the
    //    release.
    let (manifest, connector_dir, manifest_bytes) = shipped_manifest(state, &opts.connector_id)?;
    // The manifest's own bytes, which is the digest that travels: the arming receipt claims
    // it, and the verifier recomputes it from the `manifest.toml` the trace carries.
    let manifest_sha256 = workspace::sha256_hex(&manifest_bytes);
    if manifest_sha256 != opts.manifest_sha256 {
        return Err(ControlErrorResponse::with_detail(
            ControlError::PreflightFailed(format!(
                "the {} manifest this daemon ships is not the one the CLI loaded",
                manifest.id
            )),
            serde_json::json!({ "daemon": manifest_sha256, "client": opts.manifest_sha256 }),
        ));
    }
    // And the whole directory, hashed by this daemon: `render_tracker_files` reads the hook
    // bodies out of `connector_dir/files/`, records their rendered digests as the config this
    // entry will guard, and then guards them, so the process that renders them is the process
    // that has to say what they were. A digest the CLI sent would be the checked party
    // supplying its own measurement.
    let connector_dir_sha256 = pinned_connector_dir(
        &manifest.id,
        &connector_dir,
        ": the manifest matches, so what differs is a file beside it — a hook template this \
         daemon would render into the workspace and then guard",
    )?;

    let expected_handoff = if opts.attach || manifest.launch.mode == LaunchMode::AttachOnly {
        ExpectedProcessHandoff::NotRequired
    } else {
        ExpectedProcessHandoff::Required
    };
    if matches!(
        config.receipt_binding,
        crate::config::ReceiptBindingPolicy::WorkspaceSubmission { .. }
    ) && expected_handoff != ExpectedProcessHandoff::Required
    {
        return Err(ControlErrorResponse::new(ControlError::PreflightFailed(
            "workspace-submission finalization requires an accepted process handoff generation; attach mode has none"
                .to_string(),
        )));
    }

    // 3. The three roots and the run's own secrets. The entry id is minted here because it
    //    names a directory, a registry row and a URL, and nothing before this point has one.
    let entry_id = mint_entry_id();
    // A relative `workspace_parent` would be joined against whatever directory the daemon
    // happens to have been started in, which is a workspace neither the CLI nor the daemon
    // chose. The CLI knows the absolute path it means; anything else is `400`.
    let workspace_parent = match opts.workspace_parent.as_ref() {
        None => config.workspace_parent.clone(),
        Some(parent) if FsPath::new(parent).is_absolute() => PathBuf::from(parent),
        Some(parent) => {
            return Err(ControlErrorResponse::new(ControlError::Malformed(format!(
                "workspace_parent {parent:?} is not an absolute path, and a relative one \
                 would be resolved against the daemon's working directory rather than \
                 against anything the CLI named"
            ))));
        }
    };
    let roots = Roots {
        workspace: workspace_parent.join(&entry_id),
        artifact_root: config.artifact_root.join("artifacts").join(&entry_id),
        harness_home: config.artifact_root.join("homes").join(&entry_id),
    };
    let keys = RunKeys::generate();
    let session_id = mint_session_id();
    let server_port = if names_a_server_port(&manifest) {
        reserved_port()
    } else {
        None
    };

    // Every workspace refusal precedes entry-artifact creation.  The artifact root itself
    // is already configured and retained by the daemon; create `artifacts/<entry>` only
    // through that native capability chain, then hand this exact capability to the runtime.
    workspace::check_workspace_available(&roots).map_err(ControlErrorResponse::from)?;
    let cleanup_artifact = daemon
        .artifact_root()
        .create_entry(&entry_id)
        .map_err(|error| {
            ControlErrorResponse::from(DaemonError::Refused(format!(
                "fresh entry artifact authority cannot be acquired without following a swapped component: {error}"
            )))
        })?;
    let artifact = match daemon.artifact_root().open_entry(&entry_id) {
        Ok(artifact) => artifact,
        Err(error) => {
            let cleanup = cleanup_artifact.remove_recursively();
            return Err(ControlErrorResponse::from(DaemonError::Refused(format!(
                "fresh entry artifact authority cannot be duplicated through its retained parent: {error}; cleanup outcome: {cleanup:?}"
            ))));
        }
    };

    // Everything from here on can leave bytes on disk, so a failure removes them. The
    // workspace is the one root this daemon may not simply delete — `workspace_parent` may
    // be a directory the participant chose — so whether it created the directory is settled
    // before anything is written, and `rendered` collects the files `prepare` put there.
    let workspace_existed = roots.workspace.exists();
    let mut rendered: Vec<PathBuf> = Vec::new();
    let built = build_entry(
        state,
        opts,
        &manifest,
        &connector_dir,
        &manifest_sha256,
        &connector_dir_sha256,
        &roots,
        &entry_id,
        &session_id,
        server_port,
        expected_handoff,
        keys,
        artifact,
        &mut rendered,
    );
    match built {
        Ok(info) => Ok(info),
        Err(e) => {
            // A refused start leaves no half-built run. The retained artifact authority
            // removes every bearer-bearing child recursively; raw display paths are never
            // cleanup authority. Workspace files still go one by one so a pre-existing
            // workspace keeps everything the tracker did not render.
            for path in &rendered {
                if !path.starts_with(&roots.artifact_root) {
                    let _ = std::fs::remove_file(path);
                }
            }
            let _ = std::fs::remove_dir_all(&roots.harness_home);
            if !workspace_existed {
                let _ = std::fs::remove_dir_all(&roots.workspace);
            }
            if let Err(cleanup) = cleanup_artifact.remove_recursively() {
                return Err(ControlErrorResponse::from(DaemonError::Refused(format!(
                    "workspace setup failed ({e}); retained artifact cleanup also failed, so any registry row remains for inspection: {cleanup}"
                ))));
            }
            daemon
                .registry()
                .remove(&entry_id)
                .map_err(ControlErrorResponse::from)?;
            Err(ControlErrorResponse::from(e))
        }
    }
}

/// [`prepared_workspace`]'s second half: render, create, register, and say what landed.
#[cfg(feature = "v1-compat")]
#[allow(clippy::too_many_arguments)]
fn build_entry(
    state: &ControlState,
    opts: &WorkspaceOpts,
    manifest: &ConnectorManifest,
    connector_dir: &FsPath,
    manifest_sha256: &str,
    connector_dir_sha256: &str,
    roots: &Roots,
    entry_id: &str,
    session_id: &str,
    server_port: Option<u16>,
    expected_handoff: ExpectedProcessHandoff,
    keys: RunKeys,
    artifact: EntryArtifactRoot,
    rendered: &mut Vec<PathBuf>,
) -> Result<WorkspaceInfo, DaemonError> {
    let daemon = &state.daemon;
    let config = daemon.config();
    let receiver_http = crate::runtime::NO_RECEIVER.to_string();
    let status_line_path = status::canonical_status_line_path(entry_id, &config.artifact_root)?;
    let run_public_key = keys.public_key_hex()?;

    // **The directory before the `PathVars`** (D16). `PathVars::new` canonicalises the workspace
    // once and keeps the raw path if that fails, so the directory is created here: after
    // `check_workspace_available`, so every refusal still precedes a write, and before the
    // `PathVars`, `prepare` and `Entry::new`, so a symlinked parent resolves to its physical path.
    let requested_workspace = roots.workspace.clone();
    workspace::check_workspace_available(roots)?;
    std::fs::create_dir_all(&roots.workspace).map_err(|source| DaemonError::Io {
        path: roots.workspace.display().to_string(),
        source,
    })?;
    // A refusal, and not a fallback to the raw path. A directory that was just created and
    // still cannot be resolved is a workspace whose spelling this daemon cannot compare
    // against anything the harness will report, and an entry built on the raw path would
    // start, look armed, and capture nothing — which is the defect itself rather than a
    // degraded mode of it. `DaemonError::Config` is the `preflight_failed` class.
    let workspace_canonical = std::fs::canonicalize(&roots.workspace).map_err(|source| {
        DaemonError::Config(format!(
            "the workspace {} was created but cannot be resolved to a real path ({source}); \
             every store glob and every hook post is compared against the spelling the \
             harness reports for its own working directory, and there is none to compare",
            roots.workspace.display()
        ))
    })?;
    // Everything below this line — the tracker files' `{workspace}`, the `PathVars` the
    // tailers render from, the registry row, the runtime the daemon registers, and
    // `WorkspaceInfo.workspace` — is built from the one canonical spelling, so the spelling
    // the CLI caches agrees with every later answer the daemon gives (`ResumeInfo.workspace`
    // is `entry.workspace`). `requested_workspace` is carried as `workspace_requested`, which
    // is shown and never compared.
    let roots = &Roots {
        workspace: workspace_canonical,
        artifact_root: roots.artifact_root.clone(),
        harness_home: roots.harness_home.clone(),
    };

    let vars = tracker_vars(
        roots,
        &keys.receiver_token,
        entry_id,
        session_id,
        &receiver_http,
        &status_line_path,
        server_port,
    );
    let path_vars = PathVars::new(
        roots.workspace.clone(),
        // The participant's real home, which is what `{home}` means everywhere a manifest
        // uses it: `config_allowlist[]` names app-wide files the tracker did not write
        // and `foreign_scan[]` looks for an unpinned installation.
        // Pointing it at the per-run home would make both look at a
        // directory this daemon had just created, where the answer is always "nothing".
        participant_home(&roots.harness_home),
        // The harness's per-run configuration root: `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, and
        // the directory every store's `path_template` hangs off.
        roots.harness_home.clone(),
        roots.artifact_root.clone(),
        // Unbound until the run binds one, so `{session_id}` renders as `*` and the glob
        // matches every session file the harness may write.
        None,
        today(),
    );

    let prepared = workspace::prepare_with_artifact(
        manifest,
        connector_dir,
        &vars,
        &path_vars,
        roots,
        entry_id,
        &artifact,
    )?;
    // Told to the caller immediately, because every line below this one can fail and each of
    // these files carries the receiver token.
    rendered.extend(
        prepared
            .installed
            .files
            .iter()
            .map(|f| f.absolute_path.clone()),
    );

    let initial_process_generation = match expected_handoff {
        // A launch's first usable generation is opened by `arm`; an attached run has none.
        ExpectedProcessHandoff::Required => 1,
        ExpectedProcessHandoff::NotRequired | ExpectedProcessHandoff::LegacyUnknown => 0,
    };
    let mut entry = Entry::new(
        entry_id,
        &manifest.id,
        roots.workspace.clone(),
        roots.artifact_root.clone(),
        manifest_sha256,
        // The harness's own version is the CLI's preflight to measure (`detection`), and
        // this call runs no probe of its own — so it takes the number the CLI measured
        // seconds ago and sent. Nothing records the version at bind, and where
        // `version.self_updating = false` `sample_version` never runs either, so an empty value
        // would leave the signed status document at `harness: unknown` for the whole life of the
        // entry. An empty string still reaches the row where the CLI could measure nothing at
        // all, and `status` renders that as absent rather than as a version somebody read.
        opts.harness_version.as_deref().unwrap_or_default(),
        config.receipt_binding.clone(),
        expected_handoff,
        keys,
    );
    entry.bind_source_date([
        path_vars.date.0.clone(),
        path_vars.date.1.clone(),
        path_vars.date.2.clone(),
    ])?;
    entry.harness_home = Some(roots.harness_home.clone());
    // Remembered on the row because `bind_start` is where it is claimed, and that runs later
    // and on another thread: the receipt says which connector bytes captured the run, and
    // re-hashing the directory at that moment would let a template edited in between be the
    // one the receipt names.
    entry.connector_dir_sha256 = Some(connector_dir_sha256.to_string());
    entry.expected_config = prepared.expected_config.clone();
    entry.status_line_path = status_line_path.clone();
    // Remembered, not just answered: `Daemon::catch_up` reads this field to tell a relaunch
    // which port the first launch was given, and a resume that answered `None` would render
    // `--port ""` for a connector whose recipe names `{server_port}`.
    entry.server_port = server_port;
    // The arming-timeout precedence chain, stated once on `manifest::Arming::timeout_seconds`
    // and applied here: the CLI's flag overrides the daemon's config, and
    // `None` leaves the connector's own default in force inside `ArmingGate::with_timeout`.
    entry.arming_timeout_seconds = opts
        .arming_timeout_seconds
        .or(config.arming_timeout_seconds);
    let created_at = entry.created_at.clone();
    entry.event = config
        .event
        .as_ref()
        .map(|_| crate::status::event_status_for(&entry, config.event.as_ref()));

    let registry = daemon.registry();
    registry.put(&entry)?;
    let mut runtime = EntryRuntime::new_with_artifact(
        entry,
        manifest.clone(),
        config,
        path_vars,
        prepared.installed.files.clone(),
        prepared.allowlist.clone(),
        prepared.baseline.clone(),
        daemon.foreign_plan_for(manifest),
        registry,
        artifact,
    )?;
    // The round's own dates, if this machine has been told them. Unconditional: with
    // no `[event]` section `status::event_status_for` answers the entry's own timeline, which
    // is the value the runtime already holds, so there is no branch here to get wrong — and
    // one call site rather than two is what keeps the configured document and the fallback
    // document the same shape. `Daemon::rebuild` sets it the same way on the rehydration path.
    runtime.set_event_status(crate::status::event_status_for(
        runtime.entry(),
        config.event.as_ref(),
    ));
    // This creates the harness home, opens the chain so seq 0 is the baseline, and
    // publishes the first signed status — which is why `status_cache_path` below names a
    // file that is already there.
    daemon.register(runtime)?;

    Ok(WorkspaceInfo {
        entry_id: entry_id.to_string(),
        // New launched entries begin at generation zero and `arm` opens generation one.
        // Attached entries have no active handoff until their started resume CAS opens one.
        process_generation: initial_process_generation,
        // **The canonical path**, byte for byte the string on the registry row and the one
        // `resume` answers later. One spelling everywhere: the caller may cache this, hand
        // it back, and compare it against anything the daemon says about this run.
        workspace: roots.workspace.display().to_string(),
        // The spelling the participant asked for, carried for display only (D16).
        // A launcher that wants to show the participant the directory they named has it;
        // nothing routes, locks or compares on it.
        workspace_requested: requested_workspace.display().to_string(),
        artifact_root: roots.artifact_root.display().to_string(),
        harness_home: roots.harness_home.display().to_string(),
        run_public_key,
        // The page, with the id in the query — `/v0/entries/current` is authenticated and a
        // browser has no token, so the page is told which entry it is looking at.
        status_url: format!(
            "http://{}{ROUTE_UI}?entry={entry_id}",
            daemon.control_addr()
        ),
        status_cache_path: config
            .config_dir
            .join("cache")
            .join(format!("status-{entry_id}.json"))
            .display()
            .to_string(),
        status_line_path: status_line_path.display().to_string(),
        session_id: session_id.to_string(),
        server_port,
        installed_files: prepared.installed.as_protocol(),
        workspace_relative: prepared.installed.workspace_relative.clone(),
        created_at,
    })
}

/// The ten variables the tracker supplies to `hook_install[]`, and no others.
#[cfg(feature = "v1-compat")]
pub(crate) fn tracker_vars(
    roots: &Roots,
    receiver_token: &str,
    entry_id: &str,
    session_id: &str,
    receiver_http: &str,
    status_line_path: &FsPath,
    server_port: Option<u16>,
) -> TemplateVars {
    let mut v = TemplateVars::new();
    v.set("workspace", roots.workspace.display().to_string());
    v.set("artifact_root", roots.artifact_root.display().to_string());
    v.set("harness_home", roots.harness_home.display().to_string());
    v.set("receiver_http", receiver_http);
    v.set("receiver_token", receiver_token);
    v.set("entry_id", entry_id);
    v.set("run_id", entry_id);
    v.set("session_id", session_id);
    v.set(
        "server_port",
        server_port.map(|p| p.to_string()).unwrap_or_default(),
    );
    v.set("status_line_path", status_line_path.display().to_string());
    v
}

/// The directory `connectors/<dir>/manifest.toml` with this id lives in, and its bytes.
///
/// Found by reading each manifest rather than by joining `id` onto `connectors_dir`:
/// `load_manifests` keys the map by the id the manifest declares, which need not be its
/// directory's name, and a request-supplied string joined onto a path is a traversal
/// waiting for a manifest that declares a `..`.
#[cfg(feature = "v1-compat")]
pub(crate) fn shipped_connector(
    connectors_dir: &FsPath,
    id: &str,
) -> Result<(PathBuf, Vec<u8>), DaemonError> {
    let entries = std::fs::read_dir(connectors_dir).map_err(|source| DaemonError::Io {
        path: connectors_dir.display().to_string(),
        source,
    })?;
    let mut entries = entries
        .map(|entry| {
            entry.map_err(|source| DaemonError::Io {
                path: connectors_dir.display().to_string(),
                source,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::path);
    // A directory whose manifest this binary cannot parse is not a directory that is
    // missing, and the refusal must not say it is: a manifest with a section this daemon's
    // build does not know would otherwise send the reader looking for a directory that is
    // there.
    let mut unreadable: Vec<String> = Vec::new();
    for entry in entries {
        let dir = entry.path();
        let path = dir.join("manifest.toml");
        if !path.is_file() {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|source| DaemonError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let text = String::from_utf8_lossy(&bytes);
        let manifest = match ConnectorManifest::from_toml_str(&text) {
            Ok(manifest) => manifest,
            Err(e) => {
                unreadable.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        if manifest.id == id {
            return Ok((dir, bytes));
        }
    }
    if !unreadable.is_empty() {
        return Err(DaemonError::Missing(format!(
            "no connector in {} declares the id {id:?}, and {} manifest(s) there could not \
             be read by this build at all — which is what a connector edited underneath a \
             running daemon looks like: {}",
            connectors_dir.display(),
            unreadable.len(),
            unreadable.join("; ")
        )));
    }
    Err(DaemonError::Missing(format!(
        "no connector in {} declares the id {id:?}",
        connectors_dir.display()
    )))
}

/// A fresh entry id: `e` and sixteen hex characters from the operating system's entropy.
///
/// Hex, because this string names a directory, a registry key, a status cache file and a
/// URL segment. A random id rather than a counter because two daemons that had each counted
/// to three would name one entry twice on a shared server.
fn mint_entry_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("e{}", hex::encode(bytes))
}

/// A version-4 UUID, which is the shape `claude --session-id` takes.
///
/// Written out rather than pulled from a crate: this is sixteen random bytes with six bits
/// pinned, and a dependency for that is a dependency the workspace then carries.
#[cfg(feature = "v1-compat")]
fn mint_session_id() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    // Hex per field rather than one string sliced five ways: a slice index is a panic site.
    format!(
        "{}-{}-{}-{}-{}",
        hex::encode(&b[0..4]),
        hex::encode(&b[4..6]),
        hex::encode(&b[6..8]),
        hex::encode(&b[8..10]),
        hex::encode(&b[10..16])
    )
}

/// Whether this connector's recipes ask for a port, which is the only reason to reserve one.
///
/// The question is the templates, never `launch.mode`. The mode says who owns the terminal
/// — a `Tui` may not be put in a process group of its own, a `Server` may — and that is a
/// different question from whether an argv names `{server_port}`. A connector can be both
/// at once: it owns the terminal and pins the port its embedded server listens on, so a rule
/// keyed on `LaunchMode::Server` would reserve nothing for it, the port would render empty,
/// and the CLI would refuse the launch (`launch::needs_a_port_it_has`). That CLI check asks
/// the template question; this is the same question, asked on the daemon's side.
///
/// `launch.args`, `launch.env`'s values and `resume.args`: every template a launch recipe
/// has. `resume` carries no env of its own, and a `{server_port}` anywhere else in a
/// manifest is not a recipe the CLI would render a port into.
#[cfg(feature = "v1-compat")]
fn names_a_server_port(manifest: &ConnectorManifest) -> bool {
    const PORT: &str = "{server_port}";
    manifest.launch.args.iter().any(|t| t.contains(PORT))
        || manifest.launch.env.values().any(|t| t.contains(PORT))
        || manifest.resume.args.iter().any(|t| t.contains(PORT))
}

/// A loopback port nothing is listening on, for a recipe that names `{server_port}`.
///
/// Bound and released, which is as much of a reservation as an operating system offers a
/// process that is not the one going to listen. `None` when even that fails: a connector
/// whose recipe needs a port and did not get one fails at the CLI's preflight, where the
/// participant can be told which port to free, rather than here.
#[cfg(feature = "v1-compat")]
fn reserved_port() -> Option<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).ok()?;
    listener.local_addr().ok().map(|a| a.port())
}

/// The participant's home directory, or `fallback` where the environment does not say.
///
/// `HOME` and not a guess: `{home}` reaches `config_allowlist[]` and `foreign_scan[]`, and
/// both are about files this daemon did not write. A wrong answer here reports nothing
/// rather than reporting somebody else's files, which is why the fallback is the run's own
/// home — a directory this daemon just created, where every glob is empty.
#[cfg(feature = "v1-compat")]
pub(crate) fn participant_home(fallback: &FsPath) -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| fallback.to_path_buf())
}

/// `(yyyy, mm, dd)` in UTC, for the date-partitioned tree Codex writes.
///
/// Off `crate::now_rfc3339`, which is the one formatter this workspace has, so the daemon
/// and the verifier read one spelling of a date. A store template is a glob and an unbound
/// `{session_id}` already widens it, so a run that crosses midnight is picked up by the next
/// entry rather than by this one re-deriving the day mid-poll.
#[cfg(feature = "v1-compat")]
pub(crate) fn today() -> (String, String, String) {
    let now = crate::now_rfc3339();
    let date = now.split('T').next().unwrap_or("1970-01-01");
    let mut parts = date.split('-');
    (
        parts.next().unwrap_or("1970").to_string(),
        parts.next().unwrap_or("01").to_string(),
        parts.next().unwrap_or("01").to_string(),
    )
}

#[cfg(test)]
#[cfg(feature = "v1-compat")]
mod workspace_finalization_route_tests {
    use super::*;

    #[test]
    fn registry_read_failure_never_falls_back_to_the_legacy_terminal_writer() {
        let error = workspace_finalization_route(Err(DaemonError::Config(
            "registry unavailable".to_string(),
        )))
        .unwrap_err();
        assert!(matches!(error, DaemonError::Config(message) if message == "registry unavailable"));
    }
}
