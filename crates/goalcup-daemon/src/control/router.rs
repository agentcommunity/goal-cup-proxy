use super::handlers::{
    abandon_entry, create_capture_workspace, current_entry, entry_status, health, list_entries,
    no_such_route, record_capture_goal, record_capture_launch, register_process, seal_entry,
    wrong_method,
};
use super::{ControlErrorResponse, ControlState, http_status_for};
use crate::DaemonError;
use axum::Router;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, header::HOST};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use goalcup_protocol::constant_time::constant_time_eq;
use goalcup_protocol::control::{
    CONTROL_API_VERSION, ControlError, ErrorBody, ErrorDetail, ROUTE_CAPTURE_WORKSPACES,
    ROUTE_ENTRIES, ROUTE_ENTRIES_CURRENT, ROUTE_HEALTH, is_public_route,
};
use std::net::SocketAddr;
use std::sync::OnceLock;

// ------------------------------------------------------------------ the route table

/// The route suffixes under `/v0/entries/{id}/`, declared once — in
/// `goalcup_protocol::control`, and re-exported here.
///
/// They are strings the daemon's router and the CLI's client both have to spell the same
/// way, and a string spelled in two crates is a string that drifts. They live in the crate
/// neither side owns and are re-exported here, so `control::SUFFIX_SEAL` names the route the
/// protocol names and there is no second definition to drift from the first.
/// `ENTRY_SUFFIXES` and [`entry_pattern`] below are here only: they are this router's own
/// business, and no client reads them.
pub use goalcup_protocol::control::{
    SUFFIX_ABANDON, SUFFIX_CAPTURE_GOAL, SUFFIX_CAPTURE_LAUNCH, SUFFIX_PROCESS, SUFFIX_SEAL,
    SUFFIX_STATUS,
};

/// Every `/v0/entries/{id}/…` route the control API defines, in the order the CLI meets
/// them over a run's life. Enumerated so a test can assert the bearer rule over the whole
/// surface rather than over a handful of hand-picked paths.
pub const ENTRY_SUFFIXES: [&str; 6] = [
    SUFFIX_STATUS,
    SUFFIX_SEAL,
    SUFFIX_PROCESS,
    SUFFIX_ABANDON,
    SUFFIX_CAPTURE_GOAL,
    SUFFIX_CAPTURE_LAUNCH,
];

/// Every route that is not under `/v0/entries/{id}/`.
///
/// This daemon serves nothing under `/v0/assets/` (and the contract does not call it public).
pub const TOP_LEVEL_ROUTES: [&str; 4] = [
    ROUTE_CAPTURE_WORKSPACES,
    ROUTE_ENTRIES,
    ROUTE_ENTRIES_CURRENT,
    ROUTE_HEALTH,
];

/// `/v0/entries/{id}/{suffix}` as axum spells a route pattern, where
/// `control::entry_route` spells one concrete path. The two are different strings for the
/// same route and only this one has the capture in it.
pub fn entry_pattern(suffix: &str) -> String {
    format!("{ROUTE_ENTRIES}/{{id}}/{suffix}")
}

// ------------------------------------------------------------------ headers and limits

/// The control API version, on every response and in `control.json`. A client reads it
/// before it reads the body, so a daemon it does not understand is refused rather than
/// mis-parsed; a client that sends it and disagrees is refused by [`authenticate`].
///
/// Lowercase because `HeaderName` is, and HTTP header names are case-insensitive.
pub const VERSION_HEADER: &str = "x-goalcup-control-version";

/// The control API's one authentication scheme: it has exactly one caller shape and
/// one way in.
///
/// The scheme alone, without its space: RFC 9110 spells `auth-scheme` as a case-insensitive
/// token, so [`presented_bearer`] compares it with `eq_ignore_ascii_case` and a client that
/// writes `bearer` is the same client. What follows it is a secret and is compared byte for
/// byte — the case-insensitivity stops at the space.
const BEARER_SCHEME: &str = "Bearer";

/// The most bytes any control request body may be. Bounded memory is a constraint on the
/// whole daemon, and the largest thing the CLI posts is a
/// `WorkspaceOpts` or a `ProcessHandle` — kilobytes, not megabytes. Applied as a layer, so
/// it covers every handler.
pub const MAX_CONTROL_BODY_BYTES: usize = 1 << 20;

// ------------------------------------------------------------------ authentication

/// The fixed refusal, byte for byte identical whether the request carried no header, a
/// token nobody holds, or half of the right one.
///
/// It says which header is missing and nothing about what was wrong with what arrived:
/// the difference between "absent" and "wrong" is exactly what an attacker is looking for,
/// and a message that quoted the presented token back would put a secret in the reply.
fn unauthorized() -> Response {
    ControlErrorResponse::with_detail(
        ControlError::Unauthorized(
            "this route needs the control token from control.json".to_string(),
        ),
        serde_json::json!({ "header": "Authorization: Bearer <control token>" }),
    )
    .into_response()
}

/// The port the control listener actually bound, handed to [`authenticate`] as a request
/// extension by [`serve_control`].
///
/// The router is built before the socket is bound, and `127.0.0.1:0` is the normal test
/// configuration, so the port is the one fact the layer cannot know when it is built. An
/// extension rather than a field of `ControlState`: it is a property of the listener and not
/// of the daemon's shared state, and a router served any other way carries none, in which case
/// [`authenticate`] refuses every request rather than guess which port is its own.
#[derive(Clone, Copy, Debug)]
struct BoundPort(u16);

/// The fixed refusal for a `Host` that is not this listener's own.
///
/// Like [`unauthorized`], a constant: it quotes nothing the request carried. It is the same
/// code (`unauthorized`, `401`), because no contract code means "wrong host" and a new one
/// would move `CONTROL_API_VERSION` for a refusal no in-tree client can meet — every client
/// builds its `Host` from the address in `control.json`.
fn foreign_host(port: Option<u16>) -> Response {
    let accepted = match port {
        Some(p) => format!("127.0.0.1:{p}, localhost:{p}, [::1]:{p}"),
        None => "127.0.0.1:<port>, localhost:<port>, [::1]:<port>".to_string(),
    };
    ControlErrorResponse::with_detail(
        ControlError::Unauthorized(
            "the Host header is not one of this control listener's own addresses".to_string(),
        ),
        serde_json::json!({ "header": "Host", "accepted": accepted }),
    )
    .into_response()
}

/// Whether `authority` (a `Host` value, or the authority of an absolute-form target) names
/// this listener: `127.0.0.1`, `localhost` or `[::1]`, then `:<port>`.
///
/// Strict on purpose. The hostname is matched whole and ASCII-case-insensitively, so
/// `localhost.evil.example` and `127.0.0.1.evil.example` are foreign; the port must be the
/// decimal spelling of the bound port, so `0123`, `+123` and a second `:` are all refused; and
/// a missing port is accepted only on port 80, where RFC 9110 lets a client leave it out.
/// `userinfo@` is never part of a `Host`, so an `@` falls out as "not a hostname we know".
fn authority_is_ours(authority: &str, port: u16) -> bool {
    let (host, port_text) = if let Some(rest) = authority.strip_prefix('[') {
        match rest.split_once(']') {
            Some((inner, "")) => (format!("[{inner}]"), None),
            Some((inner, after)) => match after.strip_prefix(':') {
                Some(p) => (format!("[{inner}]"), Some(p)),
                None => return false,
            },
            None => return false,
        }
    } else {
        match authority.split_once(':') {
            Some((h, p)) => (h.to_string(), Some(p)),
            None => (authority.to_string(), None),
        }
    };
    let host_ok = ["127.0.0.1", "localhost", "[::1]"]
        .iter()
        .any(|ours| host.eq_ignore_ascii_case(ours));
    let port_ok = match port_text {
        Some(p) => p == port.to_string(),
        None => port == 80,
    };
    host_ok && port_ok
}

/// Whether the request was addressed to this listener, by every name it gives itself.
///
/// Exactly one `Host` line is required (HTTP/1.1 makes it mandatory, and two lines are two
/// answers to "who is this for"), it must be readable as text, and an absolute-form request
/// target's authority, when there is one, must pass as well. A browser reaching the daemon
/// through DNS rebinding sends the attacker's name here, which is the whole of what this
/// refuses.
fn addressed_to_this_listener(request: &Request, port: u16) -> bool {
    let mut lines = request.headers().get_all(HOST).iter();
    let host_ok = match (lines.next(), lines.next()) {
        (Some(only), None) => only.to_str().is_ok_and(|v| authority_is_ours(v, port)),
        _ => false,
    };
    host_ok
        && request
            .uri()
            .authority()
            .is_none_or(|a| authority_is_ours(a.as_str(), port))
}

/// The bearer the request presented, or `None`.
///
/// The scheme is matched case-insensitively ([`BEARER_SCHEME`]) and the credential after
/// the first space is returned exactly as it arrived: not trimmed, not case-folded, not
/// decoded. A secret this function tidied up would be a secret compared against something
/// the caller did not send.
fn presented_bearer(headers: &HeaderMap) -> Option<&str> {
    let (scheme, token) = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .split_once(' ')?;
    scheme
        .eq_ignore_ascii_case(BEARER_SCHEME)
        .then_some(token)
        .filter(|t| !t.is_empty())
}

/// The version this response, and every other one, is stamped with — built once.
///
/// `from_bytes`/`from_str` rather than the `from_static` spellings, which panic on a bad
/// constant. Both halves are constants this module owns, a
/// lowercase ASCII name and a decimal integer, so the `None` below is unreachable — and
/// where it to be reached, the header is absent rather than stamped under some other name,
/// because a version under the wrong name is worse than no version at all.
fn version_header() -> Option<&'static (HeaderName, HeaderValue)> {
    static HEADER: OnceLock<Option<(HeaderName, HeaderValue)>> = OnceLock::new();
    HEADER
        .get_or_init(|| {
            let name = HeaderName::from_bytes(VERSION_HEADER.as_bytes()).ok()?;
            let value = HeaderValue::from_str(&CONTROL_API_VERSION.to_string()).ok()?;
            Some((name, value))
        })
        .as_ref()
}

/// What the client said about the version it speaks.
enum ClientVersion {
    /// No header. `curl` sends none.
    Absent,
    /// A header this daemon cannot read as a version. Refused: a client whose version is
    /// unreadable is one this daemon cannot promise anything to.
    Unreadable,
    Is(u32),
}

fn declared_version(headers: &HeaderMap) -> ClientVersion {
    let values: Vec<_> = headers.get_all(VERSION_HEADER).iter().collect();
    if values.len() != 1 {
        return if values.is_empty() {
            ClientVersion::Absent
        } else {
            ClientVersion::Unreadable
        };
    }
    match values.into_iter().next() {
        None => ClientVersion::Absent,
        Some(raw) => match raw.to_str() {
            Ok(s) => match s.parse::<u32>() {
                Ok(v) if s == v.to_string() => ClientVersion::Is(v),
                _ => ClientVersion::Unreadable,
            },
            Err(_) => ClientVersion::Unreadable,
        },
    }
}

/// The auth layer.
///
/// Applied to the whole router (see the module note), and the one line in it that matters
/// is the `is_public_route` call: its argument is `request.uri().path()` — the raw path,
/// exactly as it arrived — and never a decoded, normalised or extracted one. This is the
/// only call to that function in this crate.
///
/// Order: the `Host`, then the bearer, then the version. A request addressed to some other
/// name is refused before anything else is asked of it (DNS rebinding: a page the browser
/// believes is same-origin with `evil.example` can otherwise read the two public routes).
/// A stranger who cannot authenticate learns one thing and one thing only, and a version
/// refusal is a conversation between two programs that have already agreed who they are.
///
/// The body is never read here. `DefaultBodyLimit` sits under this layer, so a refusal
/// costs a header lookup and a fixed reply — a stranger on the loopback chooses the size
/// of the body and nothing else in the request.
async fn authenticate(State(state): State<ControlState>, request: Request, next: Next) -> Response {
    let port = request.extensions().get::<BoundPort>().map(|p| p.0);
    let mut response = if !port.is_some_and(|p| addressed_to_this_listener(&request, p)) {
        // Before the public/bearer split: the public route is readable by a browser
        // without a token, which is exactly what a rebinding page is.
        foreign_host(port)
    } else {
        match is_public_route(request.uri().path()) {
            // Public, loopback and read-only by the contract: `GET /v0/entries/{id}/status`.
            // A browser sends no `Authorization` header and has no token to add.
            true => gated_by_version(request, next).await,
            false => match presented_bearer(request.headers()) {
                Some(token) if constant_time_eq(token.as_bytes(), state.token.as_bytes()) => {
                    gated_by_version(request, next).await
                }
                _ => unauthorized(),
            },
        }
    };
    // Every response, including both refusals above and whatever the handler produced.
    if let Some((name, value)) = version_header() {
        response.headers_mut().insert(name.clone(), value.clone());
    }
    response
}

/// A client whose major differs is refused rather than answered in
/// JSON it will mis-read.
///
/// `CONTROL_API_VERSION` is one integer and is bumped when a route or a field changes
/// meaning, so "a different major" and "a different number" are the same thing here. `426
/// Upgrade Required` is the status because the disagreement is about the protocol and not
/// about this request; the code is `unsupported`, and the detail names both versions so
/// the CLI can say which side is old.
///
/// **`426` is the second of the two statuses the code does not decide** — the version
/// header does — but unlike [`wrong_method`]'s `405` it is also what [`http_status_for`]
/// answers for the code it carries, and the status below is taken from that table rather
/// than written twice. `unsupported` maps to `426` and to nothing else, so a client that
/// reads only the body code and a client that reads only the status reach the same
/// conclusion about this one refusal.
///
/// Read-only clients may omit the header (curl does), but every
/// authenticated mutation must declare the exact generation it speaks.  Otherwise an old
/// client can reach a newly shaped `/process` handler with an empty/bare body
/// and consume authority before body validation gets a chance to refuse it.
async fn gated_by_version(request: Request, next: Next) -> Response {
    match declared_version(request.headers()) {
        ClientVersion::Absent if request.method() != axum::http::Method::POST => {
            next.run(request).await
        }
        ClientVersion::Absent => version_refusal(serde_json::Value::Null),
        ClientVersion::Is(v) if v == CONTROL_API_VERSION => next.run(request).await,
        declared => version_refusal(match declared {
            ClientVersion::Is(v) => serde_json::json!(v),
            _ => serde_json::Value::Null,
        }),
    }
}

fn version_refusal(client: serde_json::Value) -> Response {
    let body = ErrorBody {
        error: ErrorDetail {
            code: ControlError::Unsupported(String::new()).code().to_string(),
            message: format!("this daemon speaks control API version {CONTROL_API_VERSION}"),
            detail: serde_json::json!({
                "daemon": CONTROL_API_VERSION,
                "client": client,
            }),
        },
    };
    (
        http_status_for(&ControlError::Unsupported(String::new())),
        axum::Json(body),
    )
        .into_response()
}

// ------------------------------------------------------------------ the router

/// The control router: the route table.
pub fn control_router(state: ControlState) -> Router {
    let public = Router::new()
        .route(&entry_pattern(SUFFIX_STATUS), get(entry_status))
        .with_state(state.clone());
    // Everything else: the directories, the calls over a live run, and the two that end one.
    let authed = Router::new()
        .route(ROUTE_HEALTH, get(health))
        .route(ROUTE_ENTRIES, get(list_entries))
        .route(ROUTE_ENTRIES_CURRENT, get(current_entry))
        .route(&entry_pattern(SUFFIX_PROCESS), post(register_process))
        .route(&entry_pattern(SUFFIX_ABANDON), post(abandon_entry))
        .route(&entry_pattern(SUFFIX_SEAL), post(seal_entry))
        .route(ROUTE_CAPTURE_WORKSPACES, post(create_capture_workspace))
        .route(
            &entry_pattern(SUFFIX_CAPTURE_GOAL),
            post(record_capture_goal),
        )
        .route(
            &entry_pattern(SUFFIX_CAPTURE_LAUNCH),
            post(record_capture_launch),
        )
        .with_state(state.clone());

    public
        .merge(authed)
        .fallback(no_such_route)
        // A route that exists under another method answers the contract's shape as well.
        // axum's own `405` carries an empty body, and a client that parses every failure
        // as an `ErrorBody` would read "the daemon said nothing" instead of "wrong method".
        .method_not_allowed_fallback(wrong_method)
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .layer(axum::middleware::from_fn_with_state(state, authenticate))
}

// ------------------------------------------------------------------ the listener

/// A control listener that is serving: the address it took, and the one handle that can
/// take it back down again.
///
/// The handle exists for exactly one caller — [`crate::runtime::Daemon::start`], between
/// the bind and `control.json` — and for one reason, written at [`ControlListener::abort`].
/// Dropping this struct detaches the task, which is what the success path wants: the
/// listener outlives the handle and serves for the life of the process.
pub struct ControlListener {
    addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl ControlListener {
    /// Where the OS actually put it. `127.0.0.1:0` in every test and possibly in
    /// production, so this and not the configured address is what `control.json` names.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Stop serving, and do not answer until the socket is closed.
    ///
    /// The caller is `Daemon::start` when `control.json` cannot be written. A listener
    /// left up after that failure is the worst of both states: an authenticated port,
    /// bound for the life of the process, whose bearer was minted in a function that has
    /// already returned and therefore exists nowhere on the machine. Nothing can reach it
    /// and nothing can shut it down.
    ///
    /// The `await` is the point rather than a formality: `abort` only marks the task, and
    /// the `TcpListener` it owns is closed when the task is dropped. Awaiting it means the
    /// port is closed before `start` reports its failure, so a caller that retries binds
    /// the same port rather than racing its own corpse.
    pub async fn abort(self) {
        self.task.abort();
        let _ = self.task.await;
    }

    /// Hand the task to the daemon that keeps it.
    ///
    /// `Daemon::start`'s success path hands the task over rather than dropping this struct,
    /// which would detach it and leave the listener serving for the life of the process —
    /// right for a daemon that never stops, but [`crate::runtime::Daemon::stop`] would then
    /// have nothing to take down. The handle goes into the daemon's own list instead, and the
    /// detach happens there or not at all.
    pub fn into_task(self) -> tokio::task::JoinHandle<()> {
        self.task
    }
}

/// Bind `addr` and serve `router`.
///
/// TCP only: this binds no Unix domain socket
/// and `control.json` names none. `DaemonConfig::validate` has already refused a
/// non-loopback address by the time this is called, which — with the `Host` check in
/// [`authenticate`] — is what makes the two unauthenticated routes safe.
pub async fn serve_control(
    router: Router,
    addr: SocketAddr,
) -> Result<ControlListener, DaemonError> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|source| DaemonError::Io {
            path: addr.to_string(),
            source,
        })?;
    let bound = listener.local_addr().map_err(|source| DaemonError::Io {
        path: addr.to_string(),
        source,
    })?;
    // The port `authenticate` compares a `Host` against: the one the OS gave, which is not the
    // configured one when that is `0`. Added after the auth layer, so it is outside it.
    let router = router.layer(axum::Extension(BoundPort(bound.port())));
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Ok(ControlListener { addr: bound, task })
}
