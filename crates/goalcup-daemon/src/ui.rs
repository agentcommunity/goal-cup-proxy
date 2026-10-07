//! The bundled status page (D25). Read-only, loopback, no secrets, one file.
//!
//! `GET /v0/ui` is **unauthenticated** by the contract's own ruling
//! (`control::is_public_route`): `goalcup start` opens this page in the participant's
//! default browser, a browser sends no `Authorization` header, and the page has no token
//! to add. Leaving it open is safe because `DaemonConfig::validate` refuses a
//! non-loopback `control_addr` and because the page carries nothing private — it is a
//! constant compiled into the binary.
//!
//! # One file, and one route
//!
//! The sponsor list is markup inside `assets/status.html`, so a page whose whole promise is
//! "loads nothing from the network" does not depend on a second route being up.
//!
//! The page polls `GET /v0/entries/{id}/status`, which the control router serves
//! against this same [`UiState::cache`], so there is one status route in the process and
//! the page cannot drift from `goalcup status`. The id comes from the page's own
//! `?entry=` parameter, because `/v0/entries/current` is authenticated.

use axum::Router;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use goalcup_protocol::control::SignedStatus;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// The page, compiled in. There is no `assets/` directory to ship beside the binary and
/// nothing to read at run time, which is what makes "it loads nothing" a property of the
/// build rather than of a deployment.
pub const STATUS_HTML: &str = include_str!("../assets/status.html");

/// The signed status **per entry**, plus which entry a caller with no id should get.
///
/// Two fields, because the control router answers both `GET /v0/entries/{id}/status` and
/// `GET /v0/entries/current` off this one state and neither can be served from a single
/// `Option<SignedStatus>`.
#[derive(Clone)]
pub struct UiState {
    /// entry id → its most recent signed cache. Empty before the first evaluation.
    pub cache: Arc<Mutex<BTreeMap<String, SignedStatus>>>,
    /// The single non-sealed entry, or `None` when there is none or more than one.
    pub current: Arc<Mutex<Option<String>>>,
}

/// `GET /v0/ui`, and nothing else. It is merged into the control router outside the auth layer.
pub fn ui_router(state: UiState) -> Router {
    Router::new()
        .route(goalcup_protocol::control::ROUTE_UI, get(page))
        .with_state(state)
}

async fn page() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        STATUS_HTML,
    )
}
