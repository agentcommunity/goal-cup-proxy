//! The fake Worker (PRD-14 section 5): the routes of `docs/UPLOAD-API.md` (seam S5) answered from
//! its own position, with a script of injected answers for failure tests. It keeps every chunk
//! body and heartbeat. It has no continuity rules of its own, so it cannot drift into a second
//! Worker: the full tier runs the real one.
//!
//! Answers: a chunk is `{offset, next_seq, head, received_at, sealed}`; a heartbeat with a bad
//! `counts` is answered `200` with the counts dropped, never `400`.
//!
//! Stub: the types are final so tests can be written against them; `start` is filled in
//! PRD-14 slice 2.

use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A route of the upload API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    /// `POST /v1/device`
    Device,
    /// `POST /v1/device/token`
    DeviceToken,
    /// `POST /v1/runs`
    Open,
    /// `GET /v1/runs/{run_id}`
    Position,
    /// `PUT /v1/runs/{run_id}/chunks/{offset}`
    Chunk,
    /// `POST /v1/runs/{run_id}/heartbeat`
    Heartbeat,
    /// `POST /v1/runs/{run_id}/continue`
    Continue,
}

/// One injected answer, used once, for the next call of its route.
#[derive(Clone, Debug, Default)]
pub struct Inject {
    /// Answer with this status (and an error body) instead of the normal answer.
    pub status: Option<u16>,
    /// Do the work (store the chunk, keep the heartbeat), then drop the connection unanswered.
    pub drop_after_commit: bool,
    /// A `Retry-After` header, in seconds.
    pub retry_after: Option<u64>,
    /// Wait this long before answering.
    pub delay: Option<Duration>,
}

/// One stored chunk.
#[derive(Clone, Debug)]
pub struct Chunk {
    pub entry_id: String,
    pub offset: u64,
    pub body: Vec<u8>,
}

#[derive(Default)]
struct State {
    script: Vec<(Route, Inject)>,
    chunks: Vec<Chunk>,
    heartbeats: Vec<Value>,
}

/// A running fake Worker on a held loopback port; it stops when dropped.
pub struct FakeWorker {
    url: String,
    state: Arc<Mutex<State>>,
}

impl FakeWorker {
    pub fn start() -> FakeWorker {
        todo!("PRD-14 slice 2: the fake Worker")
    }

    /// The base URL, `http://127.0.0.1:<port>`.
    pub fn url(&self) -> String {
        self.url.clone()
    }

    /// Queues `inject` for the next call of `route` (in the order queued).
    pub fn inject(&self, route: Route, inject: Inject) {
        self.lock().script.push((route, inject));
    }

    /// Every stored chunk, per entry in the order stored.
    pub fn chunks(&self) -> Vec<Chunk> {
        self.lock().chunks.clone()
    }

    /// The stored bodies of `entry_id`, joined in order: its compressed record stream.
    pub fn stream(&self, entry_id: &str) -> Vec<u8> {
        self.lock()
            .chunks
            .iter()
            .filter(|c| c.entry_id == entry_id)
            .flat_map(|c| c.body.iter().copied())
            .collect()
    }

    /// Every heartbeat body kept, in order.
    pub fn heartbeats(&self) -> Vec<Value> {
        self.lock().heartbeats.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}
