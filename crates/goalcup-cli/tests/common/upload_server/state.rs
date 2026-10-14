//! The stand-in's state: what the server holds per run, and how it reaches disk.
//!
//! One JSON file per run under the store directory, written temp-then-rename so a stop at any
//! instant leaves either the old file or the new one. A restarted server loads every file, which
//! is the whole "server down mid-run, then back" story: nothing about a run lives only in memory.
//! Chunk bodies are separate files under `<run>/<entry>/`, written create-only.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::chain::GENESIS;
use goalcup_protocol::control::parse_z;
use goalcup_protocol::receipt::StartReceipt;
use serde::{Deserialize, Serialize};

/// One stored slice of an entry's `chain.jsonl`, as the server recorded it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredChunk {
    pub offset: u64,
    /// The uncompressed length the tracer declared (`Goalcup-Length`).
    pub length: u64,
    /// SHA-256 of the stored (compressed) body.
    pub sha256: String,
    pub first_seq: u64,
    pub last_seq: u64,
    pub prev: String,
    pub head: String,
    pub body_path: PathBuf,
    /// The acknowledgement answered for this chunk, kept so a retry gets the same bytes back.
    pub ack: AckCheckpoint,
}

/// One heartbeat the server accepted, with the server's own receive time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredHeartbeat {
    pub entry_id: String,
    pub seq: u64,
    pub head: String,
    pub offset_local: u64,
    pub final_: bool,
    pub repo_head: Option<String>,
    /// How the tracer reported its recording (`ok`, `gaps`, `failed`, `unknown`), if it said.
    #[serde(default)]
    pub recording: Option<String>,
    /// Model calls seen so far, cumulative, if the tracer said.
    #[serde(default)]
    pub model_requests: Option<u64>,
    /// The server's clock when it received the heartbeat (RFC 3339, to the second).
    pub received_at: String,
}

/// The end of the run a final heartbeat committed: the last record the server will accept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalCommit {
    pub seq: u64,
    pub head: String,
    pub repo_head: Option<String>,
}

/// What the stand-in does with a device request the moment it is created. The Worker also has
/// a pending state, between creation and the member's decision; the stand-in has none, so a
/// test sees the decision on its first poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    /// The test member approves it: the first poll returns the credential.
    Auto,
    /// The member denies it: every poll is `403`.
    Deny,
    /// It ran out of time: every poll is `410`.
    Expire,
}

/// What `POST /v1/device` asked for, as a test can see it afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRequest {
    pub purpose: String,
    pub mode: String,
    pub connector_id: String,
    pub tracer_version: String,
}

/// A device request the stand-in holds (in memory only: a restart forgets pending ones).
pub struct Device {
    pub approval: Approval,
    pub request: DeviceRequest,
    pub created_at: u64,
    pub picked_up: bool,
}

/// A run credential, stored by the sha256 of its text. `run_id` is unset until it opens a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub member: String,
    pub mode: String,
    pub run_id: Option<String>,
    /// Epoch seconds: approved + 1 hour while unbound, the run's deadline + 24 hours once bound.
    pub expires_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Open,
    Sealed,
    Abandoned,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Open => "open",
            RunStatus::Sealed => "sealed",
            RunStatus::Abandoned => "abandoned",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: String,
    pub entry_id: String,
    pub member: String,
    pub session_id: String,
    pub connector_id: String,
    pub manifest_sha256: String,
    pub connector_dir_sha256: String,
    pub workspace: String,
    pub harness_version: String,
    pub tracer_version: String,
    pub repo_url: String,
    pub official_start: String,
    pub deadline_at: String,
    pub status: RunStatus,
    pub abandon_reason: Option<String>,
    pub receipt: Option<StartReceipt>,
    pub chunks: Vec<StoredChunk>,
    /// `(seal_json, baseline_json)`, verbatim.
    pub seal: Option<(String, String)>,
    /// Every heartbeat accepted, in order.
    #[serde(default)]
    pub heartbeats: Vec<StoredHeartbeat>,
    /// Set by the first final heartbeat; never changes after.
    #[serde(default)]
    pub final_commit: Option<FinalCommit>,
    /// True when the server abandoned the run itself, `abandon_after_seconds` after the deadline
    /// with no seal, as opposed to the member giving it back through `/abandon`.
    #[serde(default)]
    pub auto_abandoned: bool,
}

impl RunRecord {
    /// The deadline in epoch seconds.
    pub fn deadline_secs(&self) -> u64 {
        parse_z(&self.deadline_at).expect("deadline_at is a stored RFC 3339 time")
    }

    /// True once the server's clock is at or past `deadline_at`.
    pub fn stopped(&self) -> bool {
        now_secs() >= self.deadline_secs()
    }

    /// Bytes of `chain.jsonl` held: the end of the last chunk.
    pub fn offset(&self) -> u64 {
        self.chunks.last().map_or(0, |c| c.offset + c.length)
    }

    /// The `seq` the next chunk must start at. A chain's first record has `seq` 0.
    pub fn next_seq(&self) -> u64 {
        self.chunks.last().map_or(0, |c| c.last_seq + 1)
    }

    /// The chain hash of the last stored record, or the genesis before any.
    pub fn head(&self) -> String {
        self.chunks
            .last()
            .map_or_else(|| GENESIS.to_string(), |c| c.head.clone())
    }
}

/// Where the stand-in's tests can see the server's current position for a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    pub offset: u64,
    pub next_seq: u64,
    pub head: String,
}

/// Everything the handlers and the test hooks share.
pub struct Shared {
    pub store_dir: PathBuf,
    /// Legacy pasted member tokens: removed with the pasted token (T5).
    pub tokens: HashMap<String, String>,
    /// The member a device request is approved by (and a login request's handle).
    pub device_member: String,
    pub approval: Mutex<Approval>,
    /// Device requests by the sha256 of the device code.
    pub devices: Mutex<HashMap<String, Device>>,
    pub device_log: Mutex<Vec<DeviceRequest>>,
    /// Run credentials by the sha256 of the credential. Persisted to `credentials.json`.
    pub credentials: Mutex<HashMap<String, Credential>>,
    /// Run length in seconds: `deadline_at` is `official_start` plus this.
    pub run_secs: u64,
    /// How long after `deadline_at` an unsealed run is abandoned.
    pub abandon_after_secs: u64,
    pub key: SigningKey,
    /// Ordered, so `run_ids()` is stable.
    pub runs: Mutex<BTreeMap<String, RunRecord>>,
    pub calls: Mutex<Vec<String>>,
    pub maintenance: AtomicBool,
    pub retry_after: AtomicU64,
    pub fail_chunks: AtomicU64,
    pub drop_replies: AtomicU64,
}

/// The server's clock, in whole seconds since the epoch.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after 1970")
        .as_secs()
}

impl Shared {
    /// Lock the runs after abandoning every open run whose time has come, so no caller sees a
    /// stale `open`. The abandonment is persisted like any other change.
    pub fn lock_runs(&self) -> MutexGuard<'_, BTreeMap<String, RunRecord>> {
        let mut runs = self.runs.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_secs();
        for run in runs.values_mut() {
            if run.status == RunStatus::Open
                && now >= run.deadline_secs().saturating_add(self.abandon_after_secs)
            {
                run.status = RunStatus::Abandoned;
                run.auto_abandoned = true;
                self.persist(run);
            }
        }
        runs
    }

    /// Load the credentials a previous server of this store handed out.
    pub fn load_credentials(store_dir: &Path) -> HashMap<String, Credential> {
        match fs::read_to_string(store_dir.join("credentials.json")) {
            Ok(text) => serde_json::from_str(&text).expect("credentials.json is valid"),
            Err(_) => HashMap::new(),
        }
    }

    /// Write the credentials atomically, like a run file.
    pub fn persist_credentials(&self, credentials: &HashMap<String, Credential>) {
        let path = self.store_dir.join("credentials.json");
        let tmp = self.store_dir.join("credentials.json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(credentials).expect("serialise credentials"),
        )
        .expect("write the credentials temp file");
        fs::rename(&tmp, &path).expect("rename the credentials file into place");
    }

    /// Load every run file already in `store_dir`.
    pub fn load_runs(store_dir: &Path) -> BTreeMap<String, RunRecord> {
        fs::create_dir_all(store_dir).expect("create the upload store directory");
        let mut runs = BTreeMap::new();
        for entry in fs::read_dir(store_dir).expect("read the upload store directory") {
            let path = entry.expect("read a store entry").path();
            let is_run_file = path.extension().is_some_and(|e| e == "json")
                && path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.starts_with('r'));
            if !is_run_file {
                continue;
            }
            let text = fs::read_to_string(&path).expect("read a run state file");
            let run: RunRecord = serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("run state file {} is not valid: {e}", path.display()));
            runs.insert(run.run_id.clone(), run);
        }
        runs
    }

    /// Write one run's state file atomically: a temp file in the same directory, then rename.
    pub fn persist(&self, run: &RunRecord) {
        let path = self.store_dir.join(format!("{}.json", run.run_id));
        let tmp = self.store_dir.join(format!("{}.json.tmp", run.run_id));
        let text = serde_json::to_vec_pretty(run).expect("serialise a run");
        fs::write(&tmp, text).expect("write the run state temp file");
        fs::rename(&tmp, &path).expect("rename the run state file into place");
    }
}
