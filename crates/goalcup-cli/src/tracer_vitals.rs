//! What a run's heartbeats say of the disk and of the tracer's own processes (their `vitals`,
//! `docs/UPLOAD-API.md`), and where `goalcup status` finds the last of it.
//!
//! The processes are this `goalcup` (which hosts the recording proxy) and each daemon it
//! started that still runs as itself. Nothing is read about any other process, and nothing is
//! read by running a command.

use crate::upload::api::HeartbeatVitals;
use goalcup_platform::machine::ProcessUsage;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

/// `<run_dir>/vitals.json`: the vitals the last heartbeat read, for `goalcup status`.
pub const VITALS_FILE_NAME: &str = "vitals.json";

/// What the vitals are read from: this machine ([`Machine`]), or a test's own answers.
pub trait Reader: Send + Sync {
    /// Free bytes on the volume holding `near`.
    fn disk_free(&self, near: &Path) -> Option<u64>;
    /// What process `pid` uses.
    fn usage(&self, pid: u32) -> ProcessUsage;
}

/// This machine.
pub struct Machine;

impl Reader for Machine {
    fn disk_free(&self, near: &Path) -> Option<u64> {
        goalcup_platform::disk::free_bytes(near).ok()
    }

    fn usage(&self, pid: u32) -> ProcessUsage {
        goalcup_platform::machine::process_usage(pid)
    }
}

/// Every daemon this process started, with its start identity, so a pid the system gave to
/// another process later is never read.
static DAEMONS: Mutex<Vec<(u32, String)>> = Mutex::new(Vec::new());

/// Note a daemon this process just started.
pub(crate) fn note_daemon(pid: u32) {
    if let Some(identity) = goalcup_protocol::process::process_start_identity(pid) {
        DAEMONS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((pid, identity));
    }
}

/// This process, and each daemon it started that is still that same process.
fn tracer_pids() -> Vec<u32> {
    use goalcup_protocol::process::{ProcessInstanceStatus, process_instance_status};
    let daemons = DAEMONS.lock().unwrap_or_else(PoisonError::into_inner);
    std::iter::once(std::process::id())
        .chain(
            daemons
                .iter()
                .filter(|(pid, identity)| {
                    process_instance_status(*pid, Some(identity.as_str()))
                        == ProcessInstanceStatus::Same
                })
                .map(|(pid, _)| *pid),
        )
        .collect()
}

/// Whether the run's `state.json` says the tracer has paused the agent for the disk.
// TODO(D65): read `RunState.disk.disk_paused` once the disk pause adds it; until then the
// field is read from the file, and a state without it is not paused.
pub fn paused_for_disk(run_dir: &Path) -> bool {
    std::fs::read(run_dir.join(crate::run::STATE_FILE_NAME))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|state| state["disk"]["disk_paused"] == serde_json::Value::Bool(true))
}

/// The vitals as the last heartbeat read them (`<run_dir>/vitals.json`), if any.
pub fn last(run_dir: &Path) -> Option<HeartbeatVitals> {
    let bytes = std::fs::read(run_dir.join(VITALS_FILE_NAME)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// What one run's heartbeats say in `vitals`.
#[derive(Clone)]
pub struct Probe {
    run_dir: PathBuf,
    trace_dir: PathBuf,
    reader: Arc<dyn Reader>,
}

impl std::fmt::Debug for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Probe")
            .field("run_dir", &self.run_dir)
            .field("trace_dir", &self.trace_dir)
            .finish_non_exhaustive()
    }
}

impl Probe {
    /// The run in `run_dir`, recording into `trace_dir`, read from this machine.
    pub fn new(run_dir: &Path, trace_dir: &Path) -> Probe {
        Probe::reading(run_dir, trace_dir, Arc::new(Machine))
    }

    /// [`Probe::new`], read from `reader`.
    pub fn reading(run_dir: &Path, trace_dir: &Path, reader: Arc<dyn Reader>) -> Probe {
        Probe {
            run_dir: run_dir.to_path_buf(),
            trace_dir: trace_dir.to_path_buf(),
            reader,
        }
    }

    /// The vitals now, also kept in `<run_dir>/vitals.json` for `goalcup status`. A number that
    /// cannot be read is left out; a sum is left out when one of its processes cannot be read.
    pub fn read(&self) -> HeartbeatVitals {
        let free = self.reader.disk_free(&self.trace_dir);
        // What the reservation holds, as the watch tick last measured it; a state not yet
        // measured says nothing.
        let held = crate::run::read_state(&self.run_dir)
            .filter(|state| state.disk != crate::run::DiskState::default())
            .map(|state| state.disk.reserve_bytes);
        let usages: Vec<ProcessUsage> = tracer_pids()
            .into_iter()
            .map(|pid| self.reader.usage(pid))
            .collect();
        let sum = |field: fn(&ProcessUsage) -> Option<u64>| {
            usages.iter().try_fold(0u64, |total, usage| {
                Some(total.saturating_add(field(usage)?))
            })
        };
        let vitals = HeartbeatVitals {
            disk_free_bytes: free,
            disk_short: free
                .zip(held)
                .map(|(free, held)| crate::run::reserve::short_of_disk(held, free)),
            agent_paused: paused_for_disk(&self.run_dir).then(|| "disk".to_string()),
            rss_bytes: sum(|usage| usage.rss_bytes),
            cpu_ms: sum(|usage| usage.cpu_ms),
            open_files: sum(|usage| usage.open_files),
        };
        if let Ok(bytes) = serde_json::to_vec(&vitals) {
            let _ = crate::config::write_secret_file(&self.run_dir.join(VITALS_FILE_NAME), &bytes);
        }
        vitals
    }
}
