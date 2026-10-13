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
use axum::http::{HeaderMap, HeaderValue, StatusCode};
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
    FinalCommit, RunRecord, RunStatus, Shared, StoredChunk, StoredHeartbeat, now_secs,
};

/// The cap on a chunk's compressed body (`Content-Length`).
const MAX_CHUNK_BYTES: u64 = 32 * 1024 * 1024;
/// The cap on each document a seal carries.
const MAX_SEAL_DOC_BYTES: usize = 16 * 1024 * 1024;
const MAX_ABANDON_REASON_CHARS: usize = 500;

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
        .route("/v1/runs", post(open_run))
        .route("/v1/runs/{run_id}", get(get_run))
        .route("/v1/runs/{run_id}/receipt", post(post_receipt))
        .route("/v1/runs/{run_id}/abandon", post(abandon))
        .route("/v1/runs/{run_id}/chunks/{offset}", put(put_chunk))
        .route("/v1/runs/{run_id}/seal", post(seal))
        .route("/v1/runs/{run_id}/heartbeat", post(heartbeat))
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
        ApiError::new(404, "not_found", "No such run for this token.")
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
/// says the server asks *every* tracer to wait.
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

/// The member a bearer token belongs to, or 401.
fn auth(s: &Shared, headers: &HeaderMap) -> Result<String, ApiError> {
    let unauthorized = || ApiError::new(401, "unauthorized", "Missing or unknown token.");
    let value = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(unauthorized)?;
    let token = value.strip_prefix("Bearer ").ok_or_else(unauthorized)?;
    s.tokens.get(token).cloned().ok_or_else(unauthorized)
}

/// A run of this member, or 404 (the same answer for "no such run" and "someone else's").
fn find_run<'a>(
    runs: &'a BTreeMap<String, RunRecord>,
    member: &str,
    run_id: &str,
) -> Result<&'a RunRecord, ApiError> {
    runs.get(run_id)
        .filter(|r| r.member == member)
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

fn run_view(run: &RunRecord) -> Value {
    json!({
        "run_id": run.run_id,
        "entry_id": run.entry_id,
        "official_start": run.official_start,
        "deadline_at": run.deadline_at,
    })
}

// ---- POST /v1/runs -----------------------------------------------------------------------

#[derive(Deserialize)]
struct OpenRequest {
    connector_id: String,
    manifest_sha256: String,
    connector_dir_sha256: String,
    workspace: String,
    session_id: String,
    harness_version: String,
    tracer_version: String,
    repo_url: String,
}

async fn open_run(State(s): St, headers: HeaderMap, body: Bytes) -> Result<Response, ApiError> {
    let member = auth(&s, &headers)?;
    let req: OpenRequest = parse_json(&body)?;
    for (name, value) in [
        ("connector_id", &req.connector_id),
        ("workspace", &req.workspace),
        ("session_id", &req.session_id),
        ("harness_version", &req.harness_version),
        ("tracer_version", &req.tracer_version),
    ] {
        if value.is_empty() {
            return Err(ApiError::malformed(format!("{name} is empty.")));
        }
    }
    for (name, value) in [
        ("manifest_sha256", &req.manifest_sha256),
        ("connector_dir_sha256", &req.connector_dir_sha256),
    ] {
        if !is_hex64(value) {
            return Err(ApiError::malformed(format!(
                "{name} is not 64 lowercase hex digits."
            )));
        }
    }
    if !valid_repo_url(&req.repo_url) {
        return Err(ApiError::malformed(
            "repo_url is not https://github.com/<owner>/<repo>.",
        ));
    }

    let mut runs = lock_runs(&s);
    if let Some(open) = runs
        .values()
        .find(|r| r.member == member && r.status == RunStatus::Open)
    {
        // A lost reply to the first call is not a second run.
        if open.session_id == req.session_id && open.receipt.is_none() {
            return Ok(json_response(200, run_view(open)));
        }
        return Err(ApiError::conflict("The account already has an open run."));
    }

    let run_id = random_id('r', |id| runs.contains_key(id));
    let entry_id = random_id('e', |id| runs.values().any(|r| r.entry_id == id));
    let start_secs = now_secs();
    let run = RunRecord {
        run_id: run_id.clone(),
        entry_id,
        member,
        session_id: req.session_id,
        connector_id: req.connector_id,
        manifest_sha256: req.manifest_sha256,
        connector_dir_sha256: req.connector_dir_sha256,
        workspace: req.workspace,
        harness_version: req.harness_version,
        tracer_version: req.tracer_version,
        repo_url: req.repo_url,
        official_start: format_z(start_secs),
        deadline_at: format_z(start_secs + s.run_secs),
        status: RunStatus::Open,
        abandon_reason: None,
        receipt: None,
        chunks: Vec::new(),
        seal: None,
        heartbeats: Vec::new(),
        final_commit: None,
        auto_abandoned: false,
    };
    s.persist(&run);
    let view = run_view(&run);
    runs.insert(run_id, run);
    Ok(json_response(201, view))
}

// ---- GET /v1/runs/{run_id} ---------------------------------------------------------------

async fn get_run(
    State(s): St,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> Result<Response, ApiError> {
    let member = auth(&s, &headers)?;
    let runs = lock_runs(&s);
    let run = find_run(&runs, &member, &run_id)?;
    Ok(json_response(
        200,
        json!({
            "run_id": run.run_id,
            "entry_id": run.entry_id,
            "status": run.status.as_str(),
            "official_start": run.official_start,
            "deadline_at": run.deadline_at,
            "offset": run.offset(),
            "next_seq": run.next_seq(),
            "head": run.head(),
            "last_ack": run.chunks.last().map(|c| &c.ack),
            "stop": run.stopped(),
            "final": run.final_commit.as_ref().map(|f| json!({"seq": f.seq, "head": f.head})),
        }),
    ))
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
    let member = auth(&s, &headers)?;
    let req: ReceiptRequest = parse_json(&body)?;
    let mut runs = lock_runs(&s);
    find_run(&runs, &member, &run_id)?;
    let run = runs.get_mut(&run_id).expect("just found");
    if run.status != RunStatus::Open {
        return Err(ApiError::closed("The run is not open."));
    }
    if req.entry_id != run.entry_id {
        return Err(ApiError::not_found());
    }
    if !is_hex64(&req.chain_head) {
        return Err(ApiError::malformed(
            "chain_head is not 64 lowercase hex digits.",
        ));
    }
    if let Some(existing) = &run.receipt {
        if existing.claims.chain_head == req.chain_head {
            return Ok(json_response(200, json!({"start_receipt": existing})));
        }
        return Err(ApiError::conflict(
            "A start receipt was already issued for a different chain head.",
        ));
    }
    let claims = StartReceiptClaims {
        entry_id: run.entry_id.clone(),
        connector_id: run.connector_id.clone(),
        workspace: run.workspace.clone(),
        session_id: run.session_id.clone(),
        started_at: run.official_start.clone(),
        deadline_at: run.deadline_at.clone(),
        manifest_sha256: run.manifest_sha256.clone(),
        chain_head: req.chain_head,
        connector_dir_sha256: Some(run.connector_dir_sha256.clone()),
        run_public_key: None,
        questionnaire_version: None,
    };
    let receipt = StartReceipt::sign(claims, &s.key).expect("sign the start receipt");
    run.receipt = Some(receipt.clone());
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
    let member = auth(&s, &headers)?;
    let req: AbandonRequest = parse_json(&body)?;
    if req.reason.chars().count() > MAX_ABANDON_REASON_CHARS {
        return Err(ApiError::malformed("reason is longer than 500 characters."));
    }
    let mut runs = lock_runs(&s);
    find_run(&runs, &member, &run_id)?;
    let run = runs.get_mut(&run_id).expect("just found");
    match run.status {
        // A repeat whose reply was lost is not an error.
        RunStatus::Abandoned if !run.auto_abandoned => {
            return Ok(json_response(200, json!({"status": "abandoned"})));
        }
        RunStatus::Abandoned => return Err(ApiError::closed("The run was abandoned.")),
        RunStatus::Sealed => return Err(ApiError::closed("The run is sealed.")),
        RunStatus::Open => {}
    }
    if !run.chunks.is_empty() {
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

/// The deadline rules for a chunk: after `deadline_at` nothing is taken until a final heartbeat
/// has committed the end, and once an end is committed nothing past it is taken, before or
/// after `deadline_at`.
fn chunk_deadline_gate(run: &RunRecord, hdr: &ChunkHeaders) -> Result<(), ApiError> {
    let Some(end) = &run.final_commit else {
        if run.stopped() {
            let mut e = ApiError::new(
                503,
                "awaiting_final",
                "The run is past its deadline and no final heartbeat has arrived yet.",
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
    let member = auth(&s, &headers)?;
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
        let run = find_run(&runs, &member, &run_id)?;
        if run.status != RunStatus::Open {
            return Err(ApiError::closed("The run is not open."));
        }
        let hdr = chunk_headers(&headers)?;
        if hdr.entry_id != run.entry_id {
            return Err(ApiError::not_found());
        }
        if hdr.content_length > MAX_CHUNK_BYTES {
            return Err(ApiError::too_large("The chunk is over 32 MiB."));
        }
        chunk_deadline_gate(run, &hdr)?;
        // A retry is answered from the stored acknowledgement, body unread.
        if let Some(held) = run.chunks.iter().find(|c| c.offset == offset) {
            if held.length == hdr.length && held.head == hdr.head {
                return Ok(chunk_reply(held));
            }
            return Err(ApiError::conflict(
                "A chunk at this offset is stored with a different length or head.",
            ));
        }
        if offset != run.offset() {
            return Err(ApiError::conflict("The offset is not where the server is."));
        }
        if hdr.prev != run.head() {
            return Err(ApiError::conflict("Goalcup-Prev is not the server's head."));
        }
        if hdr.first_seq != run.next_seq() {
            return Err(ApiError::conflict(
                "Goalcup-First-Seq is not the server's next seq.",
            ));
        }
        (hdr, run.entry_id.clone())
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
    if run.status != RunStatus::Open {
        return Err(ApiError::closed("The run is not open."));
    }
    chunk_deadline_gate(run, &hdr)?;
    if offset != run.offset() || hdr.prev != run.head() || hdr.first_seq != run.next_seq() {
        return Err(ApiError::conflict(
            "The server moved while the body was being read.",
        ));
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
    run.chunks.push(chunk);
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
    let member = auth(&s, &headers)?;
    let req: SealRequest = parse_json(&body)?;
    let mut runs = lock_runs(&s);
    find_run(&runs, &member, &run_id)?;
    let run = runs.get_mut(&run_id).expect("just found");
    if run.status == RunStatus::Abandoned {
        return Err(ApiError::closed("The run was abandoned."));
    }
    if req.entry_id != run.entry_id {
        return Err(ApiError::not_found());
    }
    if req.seal_json.len() > MAX_SEAL_DOC_BYTES || req.baseline_json.len() > MAX_SEAL_DOC_BYTES {
        return Err(ApiError::too_large("A document is over 16 MiB."));
    }
    if let Some((seal_json, baseline_json)) = &run.seal {
        if *seal_json == req.seal_json && *baseline_json == req.baseline_json {
            return Ok(json_response(200, json!({"status": "sealed"})));
        }
        return Err(ApiError::conflict(
            "The run is sealed with other documents.",
        ));
    }
    // After the deadline the seal is taken only when the server holds the chain up to the end
    // the final heartbeat committed.
    if run.stopped() {
        let complete = run
            .final_commit
            .as_ref()
            .is_some_and(|end| run.next_seq() == end.seq + 1 && run.head() == end.head);
        if !complete {
            return Err(ApiError::conflict(
                "The run is past its deadline and the server does not hold the chain up to the committed end.",
            ));
        }
    }
    run.seal = Some((req.seal_json, req.baseline_json));
    run.status = RunStatus::Sealed;
    s.persist(run);
    Ok(json_response(200, json!({"status": "sealed"})))
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
}

async fn heartbeat(
    State(s): St,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let member = auth(&s, &headers)?;
    let req: HeartbeatRequest = parse_json(&body)?;
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
    let mut runs = lock_runs(&s);
    find_run(&runs, &member, &run_id)?;
    let run = runs.get_mut(&run_id).expect("just found");
    if req.entry_id != run.entry_id {
        return Err(ApiError::not_found());
    }
    match run.status {
        RunStatus::Abandoned => return Err(ApiError::closed("The run was abandoned.")),
        // The final heartbeat follows the seal in the tracer's wrap-up, so a sealed run still
        // takes it; a plain heartbeat has nothing left to say.
        RunStatus::Sealed if !req.final_ => return Err(ApiError::closed("The run is sealed.")),
        RunStatus::Sealed | RunStatus::Open => {}
    }
    if req.final_ {
        match &run.final_commit {
            // A repeat of the committed end, whose reply may have been lost.
            Some(end) if end.seq == req.seq && end.head == req.head => {}
            Some(_) => {
                return Err(ApiError::conflict(
                    "A final heartbeat already committed a different end of the run.",
                ));
            }
            None => {
                run.final_commit = Some(FinalCommit {
                    seq: req.seq,
                    head: req.head.clone(),
                    repo_head: req.repo_head.clone(),
                });
            }
        }
    }
    run.heartbeats.push(StoredHeartbeat {
        entry_id: req.entry_id,
        seq: req.seq,
        head: req.head,
        offset_local: req.offset_local,
        final_: req.final_,
        repo_head: req.repo_head,
        received_at: now_z(),
    });
    s.persist(run);
    Ok(json_response(
        200,
        json!({"stop": run.stopped(), "deadline_at": run.deadline_at}),
    ))
}
