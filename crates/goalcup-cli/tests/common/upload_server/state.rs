//! The stand-in's state: what the server holds per run, and how it reaches disk.
//!
//! One JSON file per run under the store directory, written temp-then-rename so a stop at any
//! instant leaves either the old file or the new one. A restarted server loads every file, which
//! is the whole "server down mid-run, then back" story: nothing about a run lives only in memory.
//! Chunk bodies are separate files under `<run>/<entry>/`, written create-only.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64};

use ed25519_dalek::SigningKey;
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::chain::GENESIS;
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
}

impl RunRecord {
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
    pub tokens: HashMap<String, String>,
    pub run_hours: u64,
    pub key: SigningKey,
    /// Ordered, so `run_ids()` is stable.
    pub runs: Mutex<BTreeMap<String, RunRecord>>,
    pub calls: Mutex<Vec<String>>,
    pub maintenance: AtomicBool,
    pub retry_after: AtomicU64,
    pub fail_chunks: AtomicU64,
    pub drop_replies: AtomicU64,
}

impl Shared {
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
