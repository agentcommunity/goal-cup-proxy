//! The HTTP surface: three routes, loopback, and no authentication.
//!
//! Deliberately no bearer token. This server signs with a key published in this
//! repository, so a token would protect nothing and would only make it possible to forget
//! what the key is worth. What it does carry is the banner, on start and on
//! `GET /v0/dev/banner`, so an operator who points a daemon at it is told what it is.
//!
//! `axum::serve(TcpListener, dev_router(server))` is the whole of running it, which is why
//! a test can start one in-process.

use crate::{DevError, DevServer, ReceiptRequest};
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::chunk::Chunk;
use goalcup_protocol::receipt::StartReceipt;
use std::sync::Arc;

/// The dev server's routes over one `DevServer`.
///
/// `{id}` is axum 0.8's path syntax. It is checked against the document's own `entry_id`
/// on both POSTs rather than ignored: a client that files one entry's chunks under
/// another's URL is confused about something, and a server that quietly believed the body
/// would file them under whichever of the two it happened to read.
pub fn dev_router(server: Arc<DevServer>) -> Router {
    Router::new()
        .route("/v0/dev/banner", get(banner))
        .route("/v0/entries/{id}/receipt", post(receipt))
        .route("/v0/entries/{id}/chunks", post(chunks))
        .with_state(server)
}

/// What this server is, in one sentence, to anybody who asks.
async fn banner(State(server): State<Arc<DevServer>>) -> String {
    server.banner()
}

async fn receipt(
    State(server): State<Arc<DevServer>>,
    Path(id): Path<String>,
    axum::Json(request): axum::Json<ReceiptRequest>,
) -> Result<axum::Json<StartReceipt>, DevError> {
    if request.claims.entry_id != id {
        return Err(DevError::UnknownEntry(id));
    }
    server.issue_receipt(request).map(axum::Json)
}

async fn chunks(
    State(server): State<Arc<DevServer>>,
    Path(id): Path<String>,
    axum::Json(chunk): axum::Json<Chunk>,
) -> Result<axum::Json<AckCheckpoint>, DevError> {
    if chunk.claims.entry_id != id {
        return Err(DevError::UnknownEntry(id));
    }
    server.accept_chunk(&chunk).map(axum::Json)
}

impl IntoResponse for DevError {
    /// The status a real backend would answer with, so a client written against this one
    /// is written against the right shape.
    ///
    /// 404 for an entry that is not there — which an entry with no receipt is, from the
    /// client's side of the socket: there is no such open entry to upload to. 409 for a
    /// receipt already issued. 400 for a document the client has to fix: a run key
    /// nothing can decode, a signature that does not stand, a chunk carrying nothing, and a
    /// chunk whose links do not make the chain it claims — all of them refusals a retry of
    /// the same bytes cannot cure. 503 while the server is offline, which a retrying
    /// uploader must NOT read as a rejection. And 500 for a clock this server cannot read,
    /// which is the one refusal here that is the server's own fault.
    fn into_response(self) -> Response {
        let status = match self {
            DevError::UnknownEntry(_) | DevError::NoReceipt(_) => StatusCode::NOT_FOUND,
            DevError::AlreadyIssued(_) => StatusCode::CONFLICT,
            DevError::BadRunKey(_)
            | DevError::BadReceiptAuthority(_)
            | DevError::BadReceiptBinding(_)
            | DevError::BadSignature(_)
            | DevError::EmptyChunk(_)
            | DevError::ChainMismatch { .. } => StatusCode::BAD_REQUEST,
            DevError::Offline => StatusCode::SERVICE_UNAVAILABLE,
            DevError::Clock(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}
