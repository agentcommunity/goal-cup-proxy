//! The upload API's documents, field for field as docs/UPLOAD-API.md spells them.

use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::receipt::StartReceipt;
use serde::{Deserialize, Serialize};

/// An error answer's body. Missing fields read as empty: a proxy's HTML error page is still an
/// error, with the status code alone.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}

/// `POST /v1/runs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRunRequest {
    pub connector_id: String,
    pub manifest_sha256: String,
    pub connector_dir_sha256: String,
    pub workspace: String,
    pub session_id: String,
    pub harness_version: String,
    pub tracer_version: String,
    pub repo_url: String,
    /// What the machine is, without who it is ([`goalcup_platform::machine::facts`]). Left out
    /// when a caller has none to send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<goalcup_platform::machine::MachineFacts>,
}

/// `POST /v1/runs/{run_id}/continue`: a new segment of a run that is still open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinueRequest {
    /// The run's latest entry as this machine knows it; the server answers `409` to any other.
    pub previous_entry_id: String,
    pub connector_id: String,
    pub manifest_sha256: String,
    pub connector_dir_sha256: String,
    pub workspace: String,
    pub session_id: String,
    pub harness_version: String,
    pub tracer_version: String,
}

/// The answer to [`ContinueRequest`]: the new entry, and the run's own start and deadline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinueRun {
    pub entry_id: String,
    /// The segment's number: 1 for the first continue.
    pub segment: u64,
    pub official_start: String,
    pub deadline_at: String,
    /// Whole seconds from the server's clock to `deadline_at` when it answered (0 once it has
    /// come). The tracer counts the run down from this, never from this machine's date. An
    /// older server leaves it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds_left: Option<u64>,
}

/// The answer to [`OpenRunRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRun {
    pub run_id: String,
    pub entry_id: String,
    pub official_start: String,
    pub deadline_at: String,
    /// Whole seconds from the server's clock to `deadline_at` when it answered (0 once it has
    /// come). The tracer counts the run down from this, never from this machine's date. An
    /// older server leaves it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds_left: Option<u64>,
}

/// `POST /v1/runs/{run_id}/receipt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptRequest {
    pub entry_id: String,
    pub chain_head: String,
}

/// The answer to [`ReceiptRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptAnswer {
    pub start_receipt: StartReceipt,
}

/// `POST /v1/runs/{run_id}/abandon`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbandonRequest {
    pub reason: String,
}

/// `GET /v1/runs/{run_id}`: where the server is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunPosition {
    pub run_id: String,
    pub entry_id: String,
    pub status: String,
    pub official_start: String,
    pub deadline_at: String,
    /// Bytes of the entry's `chain.jsonl` the server holds.
    pub offset: u64,
    /// The `seq` the next chunk starts at: 0 before the first chunk.
    pub next_seq: u64,
    /// The chain hash of the last stored record; the genesis hash before any.
    pub head: String,
    pub last_ack: Option<AckCheckpoint>,
    /// The server's clock is at or past `deadline_at`: the run is over.
    #[serde(default)]
    pub stop: bool,
    /// The end a final heartbeat committed, if one has.
    #[serde(default, rename = "final")]
    pub committed: Option<ChainEnd>,
}

/// A chain position: the `seq` and `hash` of one record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainEnd {
    pub seq: u64,
    pub head: String,
}

/// `POST /v1/runs/{run_id}/heartbeat`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatRequest {
    pub entry_id: String,
    /// The last whole record of the local chain.
    pub seq: u64,
    pub head: String,
    /// The bytes of the local chain up to the end of that record.
    pub offset_local: u64,
    /// The run has ended: this commits `(seq, head)` as the last record the server accepts.
    #[serde(rename = "final")]
    pub is_final: bool,
    /// The commit the workspace repository had checked out when the run ended.
    pub repo_head: Option<String>,
    /// How the tracker sees its own recording: `ok`, `gaps`, `failed` or `unknown`. Left out
    /// by a heartbeat that has nothing to say (`goalcup upload`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording: Option<String>,
    /// Model requests seen so far, cumulative. Left out with `recording`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_requests: Option<u64>,
}

/// The answer to a heartbeat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatAnswer {
    /// The run is over: stop now.
    pub stop: bool,
    pub deadline_at: String,
    /// Whole seconds from the server's clock to `deadline_at` when it answered (0 once it has
    /// come). The tracer counts the run down from this, never from this machine's date. An
    /// older server leaves it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds_left: Option<u64>,
    /// Why `stop` is true when the deadline is not all of it: `deadline` or `replaced`. An
    /// older server leaves it out.
    #[serde(default)]
    pub reason: Option<String>,
}

/// The answer to a stored chunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkAnswer {
    pub ack: AckCheckpoint,
    pub offset: u64,
    pub next_seq: u64,
    pub head: String,
}

/// `POST /v1/runs/{run_id}/seal`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealRequest {
    pub entry_id: String,
    pub seal_json: String,
    pub baseline_json: String,
}

/// A bare `{"status": …}` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusAnswer {
    pub status: String,
    /// The run's own status, when the answer names it (a seal's does): `open`, `sealed`,
    /// `incomplete`.
    #[serde(default)]
    pub run_status: Option<String>,
}

impl StatusAnswer {
    /// The server says the run itself is over: sealed or incomplete.
    pub fn run_ended(&self) -> bool {
        matches!(self.run_status.as_deref(), Some("sealed" | "incomplete"))
    }
}
