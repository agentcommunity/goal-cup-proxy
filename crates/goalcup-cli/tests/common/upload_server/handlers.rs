//! The routes of `UPLOAD-API.md`, one function each.
//!
//! Every handler answers in the contract's error shape, `{"error", "message"}`, including for
//! bodies that fail to parse, which is why JSON bodies are taken as raw bytes and parsed here
//! instead of through axum's `Json` extractor (its rejections are plain text).

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::{Body, Bytes, HttpBody};
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::serve::IncomingStream;
use ed25519_dalek::Signer;
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::control::format_z;
use goalcup_protocol::receipt::{StartReceipt, StartReceiptClaims};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::KillListener;
use super::state::{
    Approval, Credential, Device, DeviceRequest, EntryRecord, FinalCommit, RunRecord, RunStatus,
    Shared, StoredChunk, StoredHeartbeat, now_secs,
};

/// The cap on a chunk's compressed body (`Content-Length`).
const MAX_CHUNK_BYTES: u64 = 32 * 1024 * 1024;
/// The cap on each document a seal carries.
const MAX_SEAL_DOC_BYTES: usize = 16 * 1024 * 1024;
/// `reason` is at most 500 characters (code points) and 4000 UTF-16 code units, as the Worker counts.
const MAX_ABANDON_REASON_CHARS: usize = 500;
const MAX_ABANDON_REASON_UNITS: usize = 4000;
/// `machine`, serialized.
const MAX_MACHINE_BYTES: usize = 4096;
/// The largest whole number the Worker (a JavaScript number) can hold exactly.
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;
/// A device request lives this long from its creation, whatever its state.
const DEVICE_EXPIRES_IN: u64 = 900;
const DEVICE_INTERVAL: u64 = 5;
/// An unbound credential lasts this long from the approval.
const UNBOUND_CREDENTIAL_SECS: u64 = 3600;
/// A bound credential lasts until this long after its run's deadline.
const BOUND_CREDENTIAL_EXTRA_SECS: u64 = 24 * 3600;
const USER_CODE_ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";
/// Where a member would approve; `.invalid` never resolves, so no test can open a real page.
const VERIFICATION_URI: &str = "https://approve.invalid/goalcup/device";

/// Per-connection handle a handler uses to cut its own connection (see `KillListener`).
#[derive(Clone, Debug)]
pub struct Conn(pub Arc<AtomicBool>);

impl axum::extract::connect_info::Connected<IncomingStream<'_, KillListener>> for Conn {
    fn connect_info(stream: IncomingStream<'_, KillListener>) -> Self {
        stream.remote_addr().clone()
    }
}

type St = State<Arc<Shared>>;

pub fn router(shared: Arc<Shared>) -> Router {
    Router::new()
        .route("/v1/device", post(create_device))
        .route("/v1/device/token", post(poll_device))
        .route("/v1/runs", post(open_run))
        .route("/v1/runs/{run_id}", get(get_run))
        .route("/v1/runs/{run_id}/receipt", post(post_receipt))
        .route("/v1/runs/{run_id}/abandon", post(abandon))
        .route("/v1/runs/{run_id}/chunks/{offset}", put(put_chunk))
        .route("/v1/runs/{run_id}/seal", post(seal))
        .route("/v1/runs/{run_id}/heartbeat", post(heartbeat))
        .route("/v1/runs/{run_id}/continue", post(continue_run))
        // A seal carries two documents of up to 16 MiB each inside one JSON body.
        .layer(DefaultBodyLimit::max(2 * MAX_SEAL_DOC_BYTES + 1024 * 1024))
        .layer(middleware::from_fn_with_state(shared.clone(), gate))
        .with_state(shared)
}

// ---- errors ------------------------------------------------------------------------------

pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    retry_after: Option<u64>,
}

impl ApiError {
    fn new(status: u16, code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError {
            status: StatusCode::from_u16(status).expect("a valid status"),
            code,
            message: message.into(),
            retry_after: None,
        }
    }
    fn malformed(message: impl Into<String>) -> ApiError {
        ApiError::new(400, "malformed", message)
    }
    fn not_found() -> ApiError {
        ApiError::new(404, "not_found", "No such run for this credential.")
    }
    fn conflict(message: impl Into<String>) -> ApiError {
        ApiError::new(409, "conflict", message)
    }
    fn closed(message: impl Into<String>) -> ApiError {
        ApiError::new(410, "closed", message)
    }
    fn too_large(message: impl Into<String>) -> ApiError {
        ApiError::new(413, "too_large", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut resp = (
            self.status,
            axum::Json(json!({"error": self.code, "message": self.message})),
        )
            .into_response();
        if let Some(secs) = self.retry_after {
            resp.headers_mut()
                .insert("retry-after", HeaderValue::from(secs));
        }
        resp
    }
}

fn json_response(status: u16, value: Value) -> Response {
    (
        StatusCode::from_u16(status).expect("a valid status"),
        axum::Json(value),
    )
        .into_response()
}

// ---- shared helpers ----------------------------------------------------------------------

/// The runs, with every run whose abandon time has come already abandoned.
fn lock_runs(s: &Shared) -> std::sync::MutexGuard<'_, BTreeMap<String, RunRecord>> {
    s.lock_runs()
}

fn now_z() -> String {
    format_z(now_secs())
}

/// Request log plus the maintenance switch. Maintenance comes before authentication: the contract
/// says the server asks *every* client to wait.
async fn gate(State(s): St, req: Request, next: Next) -> Response {
    s.calls
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(format!("{} {}", req.method(), req.uri().path()));
    if s.maintenance.load(Ordering::SeqCst) {
        let mut e = ApiError::new(503, "maintenance", "The server is in maintenance.");
        e.retry_after = Some(s.retry_after.load(Ordering::SeqCst));
        return e.into_response();
    }
    next.run(req).await
}

/// Who a bearer is: a member, and for a run credential the one run it may see.
struct Who {
    member: String,
    /// sha256 of the credential; `None` for a legacy member token.
    credential: Option<String>,
    /// The run the credential opened, if it has opened one.
    run_id: Option<String>,
    /// `competition` or `practice`; a legacy token is a competition one.
    mode: String,
}

impl Who {
    /// A legacy token sees all its member's runs; a credential only the run it opened.
    fn sees(&self, run_id: &str) -> bool {
        self.credential.is_none() || self.run_id.as_deref() == Some(run_id)
    }
}

pub fn sha256_hex(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// The caller behind a bearer, or 401: a run credential that has not expired, or a legacy token.
fn auth(s: &Shared, headers: &HeaderMap) -> Result<Who, ApiError> {
    let unauthorized = || {
        ApiError::new(
            401,
            "unauthorized",
            "Missing, unknown or expired credential.",
        )
    };
    let value = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(unauthorized)?;
    let token = value.strip_prefix("Bearer ").ok_or_else(unauthorized)?;
    if let Some(member) = s.tokens.get(token) {
        return Ok(Who {
            member: member.clone(),
            credential: None,
            run_id: None,
            mode: "competition".to_string(),
        });
    }
    let hash = sha256_hex(token);
    let credentials = s.credentials.lock().unwrap_or_else(|e| e.into_inner());
    match credentials.get(&hash) {
        Some(c) if now_secs() < c.expires_at => Ok(Who {
            member: c.member.clone(),
            credential: Some(hash),
            run_id: c.run_id.clone(),
            mode: c.mode.clone(),
        }),
        _ => Err(unauthorized()),
    }
}

/// A run of this caller, or 404 (the same answer for "no such run" and "someone else's").
fn find_run<'a>(
    runs: &'a BTreeMap<String, RunRecord>,
    who: &Who,
    run_id: &str,
) -> Result<&'a RunRecord, ApiError> {
    runs.get(run_id)
        .filter(|r| r.member == who.member && who.sees(&r.run_id))
        .ok_or_else(ApiError::not_found)
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn parse_json<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|e| {
        ApiError::malformed(format!("The request body is not the expected JSON: {e}."))
    })
}

fn random_id(prefix: char, taken: impl Fn(&str) -> bool) -> String {
    loop {
        let id = format!("{prefix}{:016x}", rand::random::<u64>());
        if !taken(&id) {
            return id;
        }
    }
}

/// `https://github.com/<owner>/<repo>` and nothing else.
fn valid_repo_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://github.com/") else {
        return false;
    };
    let mut parts = rest.split('/');
    let (Some(owner), Some(repo), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    // docs/UPLOAD-API.md: the owner is letters, digits and `-`; the repository also `.` and
    // `_`, and is never `.` or `..`.
    let owner_ok = !owner.is_empty()
        && owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-');
    let repo_ok = !repo.is_empty()
        && repo != "."
        && repo != ".."
        && repo
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    owner_ok && repo_ok
}

/// The length of a text as the Worker counts it: UTF-16 code units.
fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// A text field of 1 to `max` UTF-16 code units.
fn check_text(name: &str, value: &str, max: usize) -> Result<(), ApiError> {
    if value.is_empty() {
        return Err(ApiError::malformed(format!("{name} must not be empty.")));
    }
    if utf16_len(value) > max {
        return Err(ApiError::malformed(format!(
            "{name} is longer than {max} characters."
        )));
    }
    Ok(())
}

fn is_entry_id(text: &str) -> bool {
    text.len() == 17
        && text.starts_with('e')
        && text[1..]
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn run_view(s: &Shared, run: &RunRecord, entry: &EntryRecord) -> Value {
    json!({
        "run_id": run.run_id,
        "entry_id": entry.entry_id,
        "official_start": run.official_start,
        "deadline_at": run.deadline_at,
        "seconds_left": seconds_left(s, run),
    })
}

/// The `seconds_left` an answer says: the run's, or less when a test capped it
/// (`UploadServer::cap_seconds_left`).
fn seconds_left(s: &Shared, run: &RunRecord) -> u64 {
    run.seconds_left()
        .min(s.seconds_left_cap.load(Ordering::SeqCst))
}

/// What a connector and a session are, as an open or a continue call sends them.
#[derive(Deserialize)]
struct ConnectorFields {
    connector_id: String,
    manifest_sha256: String,
    connector_dir_sha256: String,
    workspace: String,
    session_id: String,
    harness_version: String,
    proxy_version: Option<String>,
    tracer_version: Option<String>,
}

/// The client's version: `proxy_version`, or `tracer_version`, its name from before the rename
/// to Goal Cup Proxy. The Worker takes either; when both are sent they must agree.
fn proxy_version<'a>(
    new: &'a Option<String>,
    old: &'a Option<String>,
) -> Result<&'a str, ApiError> {
    match (new, old) {
        (Some(new), Some(old)) if new != old => Err(ApiError::malformed(
            "proxy_version and tracer_version differ.",
        )),
        (Some(version), _) | (None, Some(version)) => Ok(version),
        (None, None) => Err(ApiError::malformed("proxy_version is missing.")),
    }
}

impl ConnectorFields {
    fn check(&self) -> Result<(), ApiError> {
        check_text("connector_id", &self.connector_id, 128)?;
        check_text("workspace", &self.workspace, 4096)?;
        check_text("session_id", &self.session_id, 128)?;
        if !self
            .session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
        {
            return Err(ApiError::malformed(
                "session_id has characters a session id never has.",
            ));
        }
        check_text("harness_version", &self.harness_version, 64)?;
        check_text(
            "proxy_version",
            proxy_version(&self.proxy_version, &self.tracer_version)?,
            64,
        )?;
        for (name, value) in [
            ("manifest_sha256", &self.manifest_sha256),
            ("connector_dir_sha256", &self.connector_dir_sha256),
        ] {
            if !is_hex64(value) {
                return Err(ApiError::malformed(format!(
                    "{name} is not 64 lowercase hex digits."
                )));
            }
        }
        Ok(())
    }

    fn into_entry(self, entry_id: String, segment: u64) -> EntryRecord {
        EntryRecord {
            entry_id,
            segment,
            session_id: self.session_id,
            connector_id: self.connector_id,
            manifest_sha256: self.manifest_sha256,
            connector_dir_sha256: self.connector_dir_sha256,
            workspace: self.workspace,
            harness_version: self.harness_version,
            proxy_version: self
                .proxy_version
                .or(self.tracer_version)
                .unwrap_or_default(),
            receipt: None,
            chunks: Vec::new(),
            seal: None,
            heartbeats: Vec::new(),
            final_commit: None,
        }
    }
}

/// `machine`, if sent: a flat object of strings, numbers, booleans and nulls, at most 4 KiB
/// serialized. (`null` is absent.)
fn check_machine(machine: Option<Value>) -> Result<Option<Value>, ApiError> {
    let Some(machine) = machine else {
        return Ok(None);
    };
    let Value::Object(map) = &machine else {
        return Err(ApiError::malformed("machine must be a JSON object."));
    };
    if map.values().any(|v| v.is_array() || v.is_object()) {
        return Err(ApiError::malformed(
            "machine must be a flat object of strings, numbers, booleans and nulls.",
        ));
    }
    if machine.to_string().len() > MAX_MACHINE_BYTES {
        return Err(ApiError::malformed("machine is over 4 KiB."));
    }
    Ok(Some(machine))
}

/// What the connection says about itself: Cloudflare's headers, when a test sends them.
fn client_facts(headers: &HeaderMap) -> (Option<String>, Option<String>) {
    let get = |name: &str, max: usize| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty())
            .map(|v| v.chars().take(max).collect::<String>())
    };
    (get("cf-connecting-ip", 64), get("cf-ipcountry", 8))
}

// ---- POST /v1/runs -----------------------------------------------------------------------

#[derive(Deserialize)]
struct OpenRequest {
    #[serde(flatten)]
    fields: ConnectorFields,
    repo_url: String,
    #[serde(default)]
    machine: Option<Value>,
}

const BEFORE_WINDOW: &str = "The start window has not opened yet.";
const AFTER_WINDOW: &str = "after the start window you can only `goalcup continue` your run";

async fn open_run(State(s): St, headers: HeaderMap, body: Bytes) -> Result<Response, ApiError> {
    let who = auth(&s, &headers)?;
    let req: OpenRequest = parse_json(&body)?;
    req.fields.check()?;
    check_text("repo_url", &req.repo_url, 200)?;
    if !valid_repo_url(&req.repo_url) {
        return Err(ApiError::malformed(
            "repo_url is not https://github.com/<owner>/<repo>.",
        ));
    }
    let machine = check_machine(req.machine)?;

    let mut runs = lock_runs(&s);
    // A credential opens one run. The same session again is a lost reply: that run.
    if let Some(opened) = &who.run_id {
        return match runs.get(opened) {
            // The open call's session is segment 0's, whatever `continue` has added since.
            Some(run) if run.entries[0].session_id == req.fields.session_id => {
                Ok(json_response(200, run_view(&s, run, &run.entries[0])))
            }
            _ => Err(opened_already()),
        };
    }
    let member = who.member.clone();
    let practice = who.mode == "practice";
    let start_secs = now_secs();
    if !practice {
        if s.window_opens.is_some_and(|opens| start_secs < opens) {
            return Err(ApiError::new(403, "forbidden", BEFORE_WINDOW));
        }
        if s.window_closes.is_some_and(|closes| start_secs > closes) {
            return Err(ApiError::new(403, "forbidden", AFTER_WINDOW));
        }
    }
    // A legacy token (no credential to bind) asking again with the same session after a lost
    // reply is still that run.
    if who.credential.is_none()
        && let Some(open) = runs.values().find(|r| {
            r.member == member
                && r.mode == "competition"
                && r.status == RunStatus::Open
                && r.replaced_at.is_none()
        })
        && open.current().session_id == req.fields.session_id
        && open.current().receipt.is_none()
    {
        return Ok(json_response(200, run_view(&s, open, open.current())));
    }

    let run_id = random_id('r', |id| runs.contains_key(id));
    let entry_id = random_id('e', |id| runs.values().any(|r| r.entry(id).is_some()));
    let run = RunRecord {
        run_id: run_id.clone(),
        member: member.clone(),
        mode: who.mode.clone(),
        repo_url: req.repo_url,
        official_start: format_z(start_secs),
        deadline_at: format_z(
            start_secs
                + if practice {
                    s.practice_secs
                } else {
                    s.run_secs
                },
        ),
        replaced_at: None,
        status: RunStatus::Open,
        abandon_reason: None,
        machine,
        client_ip: client_facts(&headers).0,
        client_country: client_facts(&headers).1,
        entries: vec![req.fields.into_entry(entry_id, 0)],
    };
    // Bind before anything is kept: a credential that cannot bind opens no run.
    if !bind_credential(&s, who.credential.as_deref(), &run) {
        return Err(opened_already());
    }
    // A competition start replaces the member's other open competition run: it ends now. A
    // test run replaces only another test run, and a competition run never replaces a test one.
    if !practice {
        for other in runs.values_mut() {
            if other.member == member
                && other.mode == who.mode
                && other.status == RunStatus::Open
                && other.replaced_at.is_none()
            {
                // Never before it began.
                let began = other.official_start.clone();
                other.replaced_at = Some(began.max(run.official_start.clone()));
                s.persist(other);
            }
        }
    }
    s.persist(&run);
    let view = run_view(&s, &run, run.current());
    runs.insert(run_id, run);
    Ok(json_response(201, view))
}

fn opened_already() -> ApiError {
    ApiError::conflict("This credential already opened a run; run `goalcup continue`.")
}

/// The credential (by hash; `None` is a legacy token, which has nothing to bind) that opened
/// `run` is bound to it, until 24 hours after its deadline. Only a credential that is unbound,
/// or already bound to this run, binds; false means it did not.
pub fn bind_credential(s: &Shared, credential: Option<&str>, run: &RunRecord) -> bool {
    let Some(hash) = credential else { return true };
    let mut credentials = s.credentials.lock().unwrap_or_else(|e| e.into_inner());
    let Some(c) = credentials.get_mut(hash) else {
        return false;
    };
    if c.run_id.as_ref().is_some_and(|bound| *bound != run.run_id) {
        return false;
    }
    c.run_id = Some(run.run_id.clone());
    c.expires_at = run.deadline_secs() + BOUND_CREDENTIAL_EXTRA_SECS;
    s.persist_credentials(&credentials);
    true
}

// ---- POST /v1/runs/{run_id}/continue -----------------------------------------------------

#[derive(Deserialize)]
struct ContinueRequest {
    previous_entry_id: String,
    #[serde(flatten)]
    fields: ConnectorFields,
}

async fn continue_run(
    State(s): St,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let who = auth(&s, &headers)?;
    let req: ContinueRequest = parse_json(&body)?;
    if !is_entry_id(&req.previous_entry_id) {
        return Err(ApiError::malformed(
            "previous_entry_id must be e and 16 lowercase hex digits.",
        ));
    }
    req.fields.check()?;
    let mut runs = lock_runs(&s);
    find_run(&runs, &who, &run_id)?;
    let entry_id = random_id('e', |id| runs.values().any(|r| r.entry(id).is_some()));
    let run = runs.get_mut(&run_id).expect("just found");
    let view = |run: &RunRecord, entry: &EntryRecord| {
        json!({"entry_id": entry.entry_id, "segment": entry.segment,
               "official_start": run.official_start, "deadline_at": run.deadline_at,
               "seconds_left": seconds_left(&s, run)})
    };
    // The same call again: the current entry came from this session, after that entry.
    let current = run.current();
    let previous = run.entries.len().checked_sub(2).map(|i| &run.entries[i]);
    if current.segment > 0
        && current.session_id == req.fields.session_id
        && previous.is_some_and(|p| p.entry_id == req.previous_entry_id)
    {
        return Ok(json_response(200, view(run, current)));
    }
    if run.status != RunStatus::Open || run.stopped() {
        return Err(ApiError::closed(
            "This run is over; there is nothing left to continue.",
        ));
    }
    if current.entry_id != req.previous_entry_id {
        return Err(ApiError::conflict(
            "previous_entry_id is not the latest entry of this run; read the position.",
        ));
    }
    let segment = current.segment + 1;
    run.entries.push(req.fields.into_entry(entry_id, segment));
    let (ip, country) = client_facts(&headers);
    run.client_ip = ip;
    run.client_country = country;
    s.persist(run);
    Ok(json_response(201, view(run, run.current())))
}

// ---- GET /v1/runs/{run_id} ---------------------------------------------------------------

/// `name=value` out of a query string; the value as sent (entry ids need no decoding).
fn query_param(uri: &Uri, name: &str) -> Option<String> {
    uri.query()?
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_string())
}

/// The entry a request names, or the run's current one; 404 if the run has no such entry.
fn named_entry<'a>(
    run: &'a RunRecord,
    entry_id: Option<&str>,
) -> Result<&'a EntryRecord, ApiError> {
    match entry_id {
        None => Ok(run.current()),
        Some(id) => run.entry(id).ok_or_else(entry_not_found),
    }
}

fn entry_not_found() -> ApiError {
    ApiError::new(404, "not_found", "That entry does not belong to this run.")
}

async fn get_run(
    State(s): St,
    headers: HeaderMap,
    uri: Uri,
    Path(run_id): Path<String>,
) -> Result<Response, ApiError> {
    let who = auth(&s, &headers)?;
    let asked = query_param(&uri, "entry_id");
    if asked.as_deref().is_some_and(|id| !is_entry_id(id)) {
        return Err(ApiError::malformed(
            "entry_id must be e and 16 lowercase hex digits.",
        ));
    }
    let runs = lock_runs(&s);
    let run = find_run(&runs, &who, &run_id)?;
    let entry = named_entry(run, asked.as_deref())?;
    Ok(json_response(
        200,
        json!({
            "run_id": run.run_id,
            "entry_id": entry.entry_id,
            "segment": entry.segment,
            "status": run.status.as_str(),
            "official_start": run.official_start,
            "deadline_at": run.deadline_at,
            "ends_at": run.ends_at(),
            "offset": entry.offset(),
            "next_seq": entry.next_seq(),
            "head": entry.head(),
            "last_ack": entry.chunks.last().map(|c| &c.ack),
            "stop": run.stopped(),
            "final": entry.final_commit.as_ref().map(|f| json!({"seq": f.seq, "head": f.head})),
        }),
    ))
}

/// 410 for a run that takes nothing more: abandoned, or over for the close time.
fn require_accepting(s: &Shared, run: &RunRecord) -> Result<(), ApiError> {
    if run.status == RunStatus::Abandoned {
        return Err(ApiError::closed("This run is abandoned."));
    }
    if run.past_close(s.close_after_secs) {
        return Err(ApiError::closed("This run is closed."));
    }
    Ok(())
}

/// [`require_accepting`], and the entry is not sealed.
fn require_open(s: &Shared, run: &RunRecord, entry: &EntryRecord) -> Result<(), ApiError> {
    require_accepting(s, run)?;
    if entry.seal.is_some() {
        return Err(ApiError::closed("This entry is sealed."));
    }
    Ok(())
}

// ---- POST /v1/runs/{run_id}/receipt ------------------------------------------------------

#[derive(Deserialize)]
struct ReceiptRequest {
    entry_id: String,
    chain_head: String,
}

async fn post_receipt(
    State(s): St,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let who = auth(&s, &headers)?;
    let req: ReceiptRequest = parse_json(&body)?;
    if !is_entry_id(&req.entry_id) {
        return Err(ApiError::malformed(
            "entry_id must be e and 16 lowercase hex digits.",
        ));
    }
    if !is_hex64(&req.chain_head) {
        return Err(ApiError::malformed(
            "chain_head is not 64 lowercase hex digits.",
        ));
    }
    let mut runs = lock_runs(&s);
    let run = find_run(&runs, &who, &run_id)?;
    let entry = named_entry(run, Some(&req.entry_id))?;
    require_open(&s, run, entry)?;
    if let Some(existing) = &entry.receipt {
        if existing.claims.chain_head == req.chain_head {
            return Ok(json_response(200, json!({"start_receipt": existing})));
        }
        return Err(ApiError::conflict(
            "A start receipt was already issued for a different chain head.",
        ));
    }
    // The entry's own connector facts and session, the run's start and deadline.
    let claims = StartReceiptClaims {
        entry_id: entry.entry_id.clone(),
        connector_id: entry.connector_id.clone(),
        workspace: entry.workspace.clone(),
        session_id: entry.session_id.clone(),
        started_at: run.official_start.clone(),
        deadline_at: run.deadline_at.clone(),
        manifest_sha256: entry.manifest_sha256.clone(),
        chain_head: req.chain_head,
        connector_dir_sha256: Some(entry.connector_dir_sha256.clone()),
        run_public_key: None,
        questionnaire_version: None,
    };
    let receipt = StartReceipt::sign(claims, &s.key).expect("sign the start receipt");
    let run = runs.get_mut(&run_id).expect("just found");
    run.entry_mut(&req.entry_id).expect("just found").receipt = Some(receipt.clone());
    s.persist(run);
    Ok(json_response(200, json!({"start_receipt": receipt})))
}

// ---- POST /v1/runs/{run_id}/abandon ------------------------------------------------------

#[derive(Deserialize)]
struct AbandonRequest {
    reason: String,
}

async fn abandon(
    State(s): St,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let who = auth(&s, &headers)?;
    let req: AbandonRequest = parse_json(&body)?;
    if utf16_len(&req.reason) > MAX_ABANDON_REASON_UNITS {
        return Err(ApiError::malformed(
            "reason is longer than 4000 characters.",
        ));
    }
    if req.reason.chars().count() > MAX_ABANDON_REASON_CHARS {
        return Err(ApiError::malformed("reason is longer than 500 characters."));
    }
    let mut runs = lock_runs(&s);
    find_run(&runs, &who, &run_id)?;
    let run = runs.get_mut(&run_id).expect("just found");
    match run.status {
        // A repeat whose reply was lost is not an error.
        RunStatus::Abandoned => {
            return Ok(json_response(200, json!({"status": "abandoned"})));
        }
        RunStatus::Open => {}
        other => return Err(ApiError::closed(format!("The run is {}.", other.as_str()))),
    }
    if run.entries.iter().any(|e| !e.chunks.is_empty()) {
        return Err(ApiError::conflict(
            "The run has stored chunks and cannot be abandoned.",
        ));
    }
    run.status = RunStatus::Abandoned;
    run.abandon_reason = Some(req.reason);
    s.persist(run);
    Ok(json_response(200, json!({"status": "abandoned"})))
}

// ---- PUT /v1/runs/{run_id}/chunks/{offset} -----------------------------------------------

struct ChunkHeaders {
    entry_id: String,
    length: u64,
    sha256: String,
    first_seq: u64,
    last_seq: u64,
    prev: String,
    head: String,
    content_length: u64,
}

fn header<'a>(h: &'a HeaderMap, name: &str) -> Result<&'a str, ApiError> {
    h.get(name)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::malformed(format!("The {name} header is missing.")))
}

fn header_u64(h: &HeaderMap, name: &str) -> Result<u64, ApiError> {
    header(h, name)?
        .parse::<u64>()
        .map_err(|_| ApiError::malformed(format!("The {name} header is not a number.")))
}

fn header_hex64(h: &HeaderMap, name: &str) -> Result<String, ApiError> {
    let v = header(h, name)?;
    if !is_hex64(v) {
        return Err(ApiError::malformed(format!(
            "The {name} header is not 64 lowercase hex digits."
        )));
    }
    Ok(v.to_string())
}

fn chunk_headers(h: &HeaderMap) -> Result<ChunkHeaders, ApiError> {
    let parsed = ChunkHeaders {
        entry_id: header(h, "goalcup-entry-id")?.to_string(),
        length: header_u64(h, "goalcup-length")?,
        sha256: header_hex64(h, "goalcup-sha256")?,
        first_seq: header_u64(h, "goalcup-first-seq")?,
        last_seq: header_u64(h, "goalcup-last-seq")?,
        prev: header_hex64(h, "goalcup-prev")?,
        head: header_hex64(h, "goalcup-head")?,
        content_length: header_u64(h, "content-length")?,
    };
    if !is_entry_id(&parsed.entry_id) {
        return Err(ApiError::malformed(
            "The goalcup-entry-id header is not e and 16 lowercase hex digits.",
        ));
    }
    if parsed.length == 0 {
        return Err(ApiError::malformed("Goalcup-Length is zero."));
    }
    if parsed.last_seq < parsed.first_seq {
        return Err(ApiError::malformed(
            "Goalcup-Last-Seq is below Goalcup-First-Seq.",
        ));
    }
    Ok(parsed)
}

/// The end rules for a chunk of an entry: once the run is over nothing is taken until the
/// entry's final heartbeat has committed the end, and once an end is committed nothing past it
/// is taken, over or not.
fn chunk_deadline_gate(
    run: &RunRecord,
    entry: &EntryRecord,
    hdr: &ChunkHeaders,
) -> Result<(), ApiError> {
    let Some(end) = &entry.final_commit else {
        if run.stopped() {
            let mut e = ApiError::new(
                503,
                "awaiting_final",
                "The run is over and no final heartbeat has arrived yet.",
            );
            e.retry_after = Some(1);
            return Err(e);
        }
        return Ok(());
    };
    if hdr.last_seq > end.seq || (hdr.last_seq == end.seq && hdr.head != end.head) {
        return Err(ApiError::closed(
            "The chunk goes past the end the final heartbeat committed.",
        ));
    }
    Ok(())
}

/// A temp name that deletes itself on drop, so every early return leaves nothing behind. On the
/// success path the bytes were hard-linked to their final name first, so removing it is cleanup.
struct TempBody {
    path: PathBuf,
}

impl Drop for TempBody {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn chunk_reply(chunk: &StoredChunk) -> Response {
    json_response(
        200,
        json!({
            "ack": chunk.ack,
            "offset": chunk.offset + chunk.length,
            "next_seq": chunk.last_seq + 1,
            "head": chunk.head,
        }),
    )
}

async fn put_chunk(
    State(s): St,
    ConnectInfo(conn): ConnectInfo<Conn>,
    headers: HeaderMap,
    Path((run_id, offset)): Path<(String, String)>,
    body: Body,
) -> Result<Response, ApiError> {
    let who = auth(&s, &headers)?;
    if s.fail_chunks
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok()
    {
        return Err(ApiError::new(500, "internal", "Injected chunk failure."));
    }
    let offset: u64 = offset
        .parse()
        .map_err(|_| ApiError::malformed("The offset in the path is not a number."))?;

    // Everything the contract says is refused before the body is read.
    let (hdr, entry_id) = {
        let runs = lock_runs(&s);
        let run = find_run(&runs, &who, &run_id)?;
        let hdr = chunk_headers(&headers)?;
        if hdr.content_length > MAX_CHUNK_BYTES {
            return Err(ApiError::too_large("The chunk is over 32 MiB."));
        }
        let entry = named_entry(run, Some(&hdr.entry_id))?;
        // A retry is answered from the stored acknowledgement, body unread, whatever the state
        // of the run has become since.
        if offset < entry.offset()
            && let Some(held) = entry.chunks.iter().find(|c| c.offset == offset)
            && held.length == hdr.length
            && held.head == hdr.head
        {
            return Ok(chunk_reply(held));
        }
        require_open(&s, run, entry)?;
        chunk_deadline_gate(run, entry, &hdr)?;
        if offset < entry.offset() {
            return Err(ApiError::conflict(
                "A chunk at this offset is stored with a different length or head.",
            ));
        }
        if offset != entry.offset() {
            return Err(ApiError::conflict("The offset is not where the server is."));
        }
        if hdr.prev != entry.head() {
            return Err(ApiError::conflict("Goalcup-Prev is not the server's head."));
        }
        if hdr.first_seq != entry.next_seq() {
            return Err(ApiError::conflict(
                "Goalcup-First-Seq is not the server's next seq.",
            ));
        }
        let entry_id = entry.entry_id.clone();
        (hdr, entry_id)
    };

    // Stream the body to disk while hashing it; keep it only if the hash is the declared one.
    let dir = s.store_dir.join(&run_id).join(&entry_id);
    fs::create_dir_all(&dir).expect("create the chunk directory");
    let temp = TempBody {
        path: dir.join(format!(".tmp-{:016x}", rand::random::<u64>())),
    };
    let mut file = fs::File::create(&temp.path).expect("create the chunk temp file");
    let mut hasher = Sha256::new();
    let mut count: u64 = 0;
    let mut body = Box::pin(body);
    loop {
        let frame = std::future::poll_fn(|cx| Pin::as_mut(&mut body).poll_frame(cx)).await;
        match frame {
            None => break,
            Some(Err(e)) => {
                return Err(ApiError::malformed(format!(
                    "The body could not be read: {e}."
                )));
            }
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    count += data.len() as u64;
                    if count > MAX_CHUNK_BYTES {
                        return Err(ApiError::too_large("The chunk is over 32 MiB."));
                    }
                    hasher.update(&data);
                    file.write_all(&data).expect("write the chunk temp file");
                }
            }
        }
    }
    drop(file);
    if count != hdr.content_length {
        return Err(ApiError::malformed(
            "The body is not as long as Content-Length says.",
        ));
    }
    if hex::encode(hasher.finalize()) != hdr.sha256 {
        return Err(ApiError::new(
            422,
            "digest_mismatch",
            "The body's SHA-256 is not Goalcup-Sha256.",
        ));
    }

    // Commit: re-check the position (another call may have moved it while the body streamed),
    // then create-only link into place and record.
    let mut runs = lock_runs(&s);
    let run = runs.get_mut(&run_id).expect("runs are never removed");
    {
        let entry = run.entry(&entry_id).expect("entries are never removed");
        require_open(&s, run, entry)?;
        chunk_deadline_gate(run, entry, &hdr)?;
        if offset != entry.offset() || hdr.prev != entry.head() || hdr.first_seq != entry.next_seq()
        {
            return Err(ApiError::conflict(
                "The server moved while the body was being read.",
            ));
        }
    }
    let final_path = dir.join(format!("{offset:016}-{}.zst", hdr.sha256));
    match fs::hard_link(&temp.path, &final_path) {
        Ok(()) => {}
        // Same offset and same hash is the same bytes: left over from a crash before the state
        // file was written.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => panic!("link the chunk into place: {e}"),
    }
    // `temp` is dropped on return, which removes the temp name; the final name holds the bytes.

    let mut ack = AckCheckpoint {
        entry_id: entry_id.clone(),
        seq: hdr.last_seq,
        hash: hdr.head.clone(),
        acked_at: now_z(),
        server_signature: String::new(),
    };
    let signature = s.key.sign(&ack.signing_bytes().expect("ack signing bytes"));
    ack.server_signature = hex::encode(signature.to_bytes());
    let chunk = StoredChunk {
        offset,
        length: hdr.length,
        sha256: hdr.sha256,
        first_seq: hdr.first_seq,
        last_seq: hdr.last_seq,
        prev: hdr.prev,
        head: hdr.head,
        body_path: final_path,
        ack,
    };
    let reply = chunk_reply(&chunk);
    run.entry_mut(&entry_id)
        .expect("entries are never removed")
        .chunks
        .push(chunk);
    s.persist(run);

    // The chunk is committed. Lost-reply hook: cut the connection so the caller never hears.
    if s.drop_replies
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok()
    {
        conn.0.store(true, Ordering::SeqCst);
    }
    Ok(reply)
}

// ---- POST /v1/runs/{run_id}/seal ---------------------------------------------------------

#[derive(Deserialize)]
struct SealRequest {
    entry_id: String,
    seal_json: String,
    baseline_json: String,
}

async fn seal(
    State(s): St,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let who = auth(&s, &headers)?;
    let req: SealRequest = parse_json(&body)?;
    if !is_entry_id(&req.entry_id) {
        return Err(ApiError::malformed(
            "entry_id must be e and 16 lowercase hex digits.",
        ));
    }
    if req.seal_json.is_empty() || req.baseline_json.is_empty() {
        return Err(ApiError::malformed(
            "A document must be a non-empty string.",
        ));
    }
    if req.seal_json.len() > MAX_SEAL_DOC_BYTES || req.baseline_json.len() > MAX_SEAL_DOC_BYTES {
        return Err(ApiError::too_large("A document is over 16 MiB."));
    }
    let mut runs = lock_runs(&s);
    find_run(&runs, &who, &run_id)?;
    let run = runs.get_mut(&run_id).expect("just found");
    let entry = named_entry(run, Some(&req.entry_id))?;
    if let Some((seal_json, baseline_json)) = &entry.seal {
        if *seal_json == req.seal_json && *baseline_json == req.baseline_json {
            return Ok(json_response(
                200,
                json!({"status": "sealed", "run_status": run.status.as_str()}),
            ));
        }
        return Err(ApiError::conflict(
            "This entry is sealed with other documents.",
        ));
    }
    require_accepting(&s, run)?;
    // A seal needs the end the entry's final heartbeat committed, and the chain held up to it.
    let Some(end) = entry.final_commit.as_ref() else {
        if run.stopped() {
            let mut e = ApiError::new(
                503,
                "awaiting_final",
                "The run is over and no final heartbeat has arrived yet.",
            );
            e.retry_after = Some(1);
            return Err(e);
        }
        return Err(ApiError::conflict(
            "Send the final heartbeat first: a seal needs the end of the entry committed.",
        ));
    };
    if !(entry.next_seq() == end.seq + 1 && entry.head() == end.head) {
        return Err(ApiError::conflict(
            "The server does not hold the chain up to the committed end.",
        ));
    }
    let entry_id = req.entry_id.clone();
    run.entry_mut(&entry_id).expect("just found").seal = Some((req.seal_json, req.baseline_json));
    // The run is sealed when its current entry is, once it is over.
    run.status = run.status_now(s.close_after_secs);
    s.persist(run);
    Ok(json_response(
        200,
        json!({"status": "sealed", "run_status": run.status.as_str()}),
    ))
}

// ---- POST /v1/runs/{run_id}/heartbeat ----------------------------------------------------

#[derive(Deserialize)]
struct HeartbeatRequest {
    entry_id: String,
    seq: u64,
    head: String,
    offset_local: u64,
    #[serde(rename = "final")]
    final_: bool,
    repo_head: Option<String>,
    #[serde(default)]
    recording: Option<String>,
    #[serde(default)]
    model_requests: Option<u64>,
    #[serde(default)]
    vitals: Option<serde_json::Value>,
}

async fn heartbeat(
    State(s): St,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let who = auth(&s, &headers)?;
    let req: HeartbeatRequest = parse_json(&body)?;
    if !req.final_ && s.fail_heartbeats.load(Ordering::SeqCst) {
        return Err(ApiError::new(503, "unavailable", "Refused by the test."));
    }
    if !is_entry_id(&req.entry_id) {
        return Err(ApiError::malformed(
            "entry_id must be e and 16 lowercase hex digits.",
        ));
    }
    if !is_hex64(&req.head) {
        return Err(ApiError::malformed("head is not 64 lowercase hex digits."));
    }
    if let Some(repo_head) = &req.repo_head
        && !(repo_head.len() == 40
            && repo_head
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
    {
        return Err(ApiError::malformed(
            "repo_head is not 40 lowercase hex digits.",
        ));
    }
    if req.model_requests.is_some_and(|n| n > MAX_SAFE_INTEGER) {
        return Err(ApiError::malformed("model_requests is over 2^53 - 1."));
    }
    if let Some(recording) = &req.recording
        && !["ok", "gaps", "failed", "unknown"].contains(&recording.as_str())
    {
        return Err(ApiError::malformed(
            "recording is not ok, gaps, failed or unknown.",
        ));
    }
    let mut runs = lock_runs(&s);
    find_run(&runs, &who, &run_id)?;
    let run = runs.get_mut(&run_id).expect("just found");
    named_entry(run, Some(&req.entry_id))?;
    // An abandoned or closed run accepts nothing more, a final heartbeat included.
    require_accepting(&s, run)?;
    let stop = run.stopped();
    let reason = match (stop, &run.replaced_at) {
        (false, _) => Value::Null,
        (true, Some(replaced)) if *replaced < run.deadline_at => json!("replaced"),
        (true, _) => json!("deadline"),
    };
    let answer = json!({"stop": stop, "deadline_at": run.deadline_at,
                        "seconds_left": seconds_left(&s, run), "reason": reason});
    let entry = run.entry_mut(&req.entry_id).expect("just found");
    if !req.final_ {
        // A sealed entry has nothing left to record.
        if entry.seal.is_some() {
            return Ok(json_response(200, answer));
        }
    } else {
        match &entry.final_commit {
            // A repeat of the committed end, whose reply may have been lost.
            Some(end) if end.seq == req.seq && end.head == req.head => {}
            Some(_) => {
                return Err(ApiError::conflict(
                    "A final heartbeat already committed a different end of the entry.",
                ));
            }
            None => {
                // It cannot contradict the chain the server already holds.
                let stored = entry.chunks.last().map(|c| (c.last_seq, c.head.clone()));
                if let Some((last_seq, head)) = stored
                    && (last_seq > req.seq || (last_seq == req.seq && head != req.head))
                {
                    return Err(ApiError::conflict(
                        "The server already holds a different chain than this final heartbeat names.",
                    ));
                }
                entry.final_commit = Some(FinalCommit {
                    seq: req.seq,
                    head: req.head.clone(),
                    repo_head: req.repo_head.clone(),
                });
            }
        }
    }
    entry.heartbeats.push(StoredHeartbeat {
        entry_id: req.entry_id,
        seq: req.seq,
        head: req.head,
        offset_local: req.offset_local,
        final_: req.final_,
        repo_head: req.repo_head,
        recording: req.recording,
        model_requests: req.model_requests,
        vitals: req.vitals,
        received_at: now_z(),
    });
    s.persist(run);
    Ok(json_response(200, answer))
}

// ---- POST /v1/device, POST /v1/device/token ----------------------------------------------

/// A credential for `member`, stored by its hash and persisted; returns the plain text.
pub fn mint_credential(s: &Shared, member: &str, mode: &str, approved_at: u64) -> String {
    let plain = format!(
        "gcr_{:016x}{:016x}{:016x}{:016x}",
        rand::random::<u64>(),
        rand::random::<u64>(),
        rand::random::<u64>(),
        rand::random::<u64>()
    );
    let mut credentials = s.credentials.lock().unwrap_or_else(|e| e.into_inner());
    credentials.insert(
        sha256_hex(&plain),
        Credential {
            member: member.to_string(),
            mode: mode.to_string(),
            run_id: None,
            expires_at: approved_at + UNBOUND_CREDENTIAL_SECS,
        },
    );
    s.persist_credentials(&credentials);
    plain
}

#[derive(Deserialize)]
struct DeviceBody {
    purpose: String,
    mode: String,
    connector_id: String,
    proxy_version: Option<String>,
    tracer_version: Option<String>,
}

async fn create_device(State(s): St, body: Bytes) -> Result<Response, ApiError> {
    let req: DeviceBody = parse_json(&body)?;
    if !["login", "run"].contains(&req.purpose.as_str()) {
        return Err(ApiError::malformed("purpose is not login or run."));
    }
    if !["competition", "practice", "test"].contains(&req.mode.as_str()) {
        return Err(ApiError::malformed(
            "mode is not competition, practice or test.",
        ));
    }
    let version = proxy_version(&req.proxy_version, &req.tracer_version)?.to_string();
    for (name, value, max) in [
        ("connector_id", &req.connector_id, 128),
        ("proxy_version", &version, 64),
    ] {
        if value.is_empty() || utf16_len(value) > max {
            return Err(ApiError::malformed(format!(
                "{name} must be 1 to {max} characters."
            )));
        }
    }
    let device_code = format!(
        "{:016x}{:016x}{:016x}{:016x}",
        rand::random::<u64>(),
        rand::random::<u64>(),
        rand::random::<u64>(),
        rand::random::<u64>()
    );
    let letters: String = (0..8)
        .map(|_| {
            USER_CODE_ALPHABET[rand::random::<u32>() as usize % USER_CODE_ALPHABET.len()] as char
        })
        .collect();
    let user_code = format!("{}-{}", &letters[..4], &letters[4..]);
    let request = DeviceRequest {
        purpose: req.purpose,
        mode: req.mode,
        connector_id: req.connector_id,
        proxy_version: version,
    };
    let approval = *s.approval.lock().unwrap_or_else(|e| e.into_inner());
    s.device_log
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(request.clone());
    s.devices.lock().unwrap_or_else(|e| e.into_inner()).insert(
        sha256_hex(&device_code),
        Device {
            approval,
            request,
            created_at: now_secs(),
            picked_up: false,
        },
    );
    Ok(json_response(
        200,
        json!({
            "device_code": device_code,
            "user_code": user_code,
            "verification_uri": VERIFICATION_URI,
            "verification_uri_complete": format!("{VERIFICATION_URI}?code={user_code}"),
            "interval": DEVICE_INTERVAL,
            "expires_in": DEVICE_EXPIRES_IN,
        }),
    ))
}

#[derive(Deserialize)]
struct TokenBody {
    device_code: String,
}

async fn poll_device(State(s): St, body: Bytes) -> Result<Response, ApiError> {
    let req: TokenBody = parse_json(&body)?;
    if !is_hex64(&req.device_code) {
        return Err(ApiError::malformed(
            "device_code is not 64 lowercase hex digits.",
        ));
    }
    let gone = || ApiError::closed("This sign-in request has expired or was already picked up.");
    let mut devices = s.devices.lock().unwrap_or_else(|e| e.into_inner());
    let device = devices
        .get_mut(&sha256_hex(&req.device_code))
        .ok_or_else(gone)?;
    let now = now_secs();
    if now >= device.created_at + DEVICE_EXPIRES_IN || device.picked_up {
        return Err(gone());
    }
    match device.approval {
        Approval::Expire => return Err(gone()),
        Approval::Deny => {
            return Err(ApiError::new(403, "forbidden", "This sign-in was denied."));
        }
        Approval::Auto => {}
    }
    device.picked_up = true;
    if device.request.purpose == "login" {
        return Ok(json_response(
            200,
            json!({"status": "approved", "purpose": "login",
                   "account": {"handle": s.device_member}}),
        ));
    }
    let plain = mint_credential(
        &s,
        &s.device_member,
        &device.request.mode,
        device.created_at,
    );
    Ok(json_response(
        200,
        json!({"status": "approved", "purpose": "run", "credential": plain,
               "expires_at": format_z(device.created_at + UNBOUND_CREDENTIAL_SECS)}),
    ))
}
