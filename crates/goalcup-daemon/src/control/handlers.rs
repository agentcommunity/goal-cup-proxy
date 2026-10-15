use super::{ControlErrorResponse, ControlState};
use crate::registry::ProcessHandoffPhase;
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
use goalcup_protocol::process::{
    ProcessRegistration, native_pid_is_bound_to_handle, process_start_identity, snapshot,
};
use serde::Deserialize;
use std::path::Path as FsPath;

/// A path this build serves nothing on, in the contract's shape.
///
/// The message names no path. The caller knows what it asked for, and a 404 that echoed
/// the request line back would put an attacker's own bytes into a response body.
pub(super) async fn no_such_route() -> Response {
    ControlErrorResponse::new(ControlError::NotFound("no such control route".to_string()))
        .into_response()
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
        cycles: state.daemon.cycle_errors(),
        reopen_failed: state.daemon.reopen_failed_ids(),
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
/// so `goalcup status` reads the bytes the daemon signed.
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

/// `POST /v0/entries/{id}/seal` — the participant's early finish of a local capture.
///
/// The entry's row decides which finish runs: a v5 capture needs a capture seal request, and
/// a capture seal request naming any other entry is refused. Any other id is an entry this
/// daemon does not hold, because the V1 seal was removed (D48).
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
    ControlErrorResponse::from(DaemonError::Missing(format!(
        "no entry {id} in this daemon"
    )))
    .into_response()
}

// ------------------------------------------------------------------ the entry handlers
//
// Each route has the same shape: resolve, call a function, map the failure through the
// one table above. Nothing here interprets a rule.

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

/// How far ahead of the daemon's clock a run's start (`native_since`) may be.
const NATIVE_SINCE_SKEW_SECS: u64 = 300;

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
                return ControlErrorResponse::new(ControlError::PreflightFailed(format!(
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
    // A start time ahead of this clock (beyond a few minutes of skew) is a launcher bug, a
    // time in milliseconds the likeliest: it would silently admit no session file (D57).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    if let Some(since) = request.native_since
        && since > now.saturating_add(NATIVE_SINCE_SKEW_SECS)
    {
        return ControlErrorResponse::new(ControlError::Malformed(format!(
            "native_since {since} is later than this machine's clock ({now}) by more than \
             {NATIVE_SINCE_SKEW_SECS} seconds: it is the run's start in Unix seconds"
        )))
        .into_response();
    }
    let entry_id = match request.entry_id {
        Some(id) if goalcup_protocol::control::is_entry_id(&id) => id,
        Some(id) => {
            return ControlErrorResponse::new(ControlError::Malformed(format!(
                "entry_id {id:?} is not `e` and 16 lowercase hex digits"
            )))
            .into_response();
        }
        None => mint_entry_id(),
    };
    match state.daemon.create_capture(
        entry_id,
        FsPath::new(&request.workspace),
        FsPath::new(&request.run_home),
        FsPath::new(&request.config_home),
        &loaded.manifest,
        &digests,
        &every_loaded_real_dir,
        request.reserve_bytes,
        request.native_since,
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
    // The launch hashes the harness's binaries (D53), seconds of file reads at worst, so it runs
    // on the blocking pool rather than on a control-API worker.
    let daemon = state.daemon.clone();
    match tokio::task::spawn_blocking(move || daemon.record_capture_launch(&id, &request)).await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(e)) => ControlErrorResponse::from(e).into_response(),
        Err(join) => ControlErrorResponse::new(ControlError::Internal(format!(
            "the launch did not complete: {join}"
        )))
        .into_response(),
    }
}

/// Whether `id`'s registry row is a v5 capture entry's.
fn is_capture_row(state: &ControlState, id: &str) -> Result<bool, DaemonError> {
    Ok(state
        .daemon
        .registry()
        .get(id)?
        .is_some_and(|row| row.capture.is_some()))
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
/// An empty `argv` is not refused. `terminate`'s guard has two halves — the process-start
/// identity (`process_start`, required below) and the program `ps` names among its
/// arguments — and a handle with no argv leaves only the first; the tree already carries
/// that case as `TerminationReport::guard_degraded`, which reports
/// the weaker guard rather than pretending the launch never happened.
///
/// **A second registration is a `409` while a handle stands**, raised inside
/// `Daemon::register_process`: the handle is what the seal terminates, so a decoy posted
/// on the loopback would redirect it. Only the V1
/// resume cleared it, and D48 removed that: a capture entry cannot be resumed, so its
/// harness registers once.
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
            // A live runtime does not need to be acquired for an idempotent registry ACK.
            // If there is no runtime, only a sealed row may acknowledge without monitoring.
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

/// `POST /v0/entries/{id}/abandon` — failure before dispatch leaves no entry.
///
/// `Daemon::abandon` drops the entry and its registry row, and refuses once `started_at`
/// is stamped. This handler adds the half that makes "leaves no entry" true of the disk as
/// well: the entry's artifact directory.
///
/// **The removal is inside the entry's own root and is checked to be**: the artifact
/// directory has to sit under the daemon's configured `artifact_root`, or nothing is
/// removed. The workspace is deliberately left alone: it is the participant's directory, that rule
/// is about the entry, and a daemon that deleted a directory a participant had been working
/// in would be the worst possible reading of "abandon".
///
/// Order matters. `Daemon::abandon` removes the artifact tree through its retained entry
/// authority before deleting the registry row; a cleanup failure is therefore an error and
/// never a successful abandon.
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
    StatusCode::NO_CONTENT.into_response()
}

// ------------------------------------------------------ the workspace, end to end

/// A fresh entry id: `e` and sixteen hex characters from the operating system's entropy.
///
/// Hex, because this string names a directory, a registry key and a URL segment. A random id rather than a counter because two daemons that had each counted
/// to three would name one entry twice on a shared server.
fn mint_entry_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("e{}", hex::encode(bytes))
}
