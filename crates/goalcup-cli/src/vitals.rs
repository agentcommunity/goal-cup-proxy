//! What a run's heartbeats say of the disk and of goalcup's own processes (their `vitals`,
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

/// This process (with no identity), and each daemon it started that is still that same process
/// (with its start identity).
fn own_processes() -> Vec<(u32, Option<String>)> {
    use goalcup_protocol::process::{ProcessInstanceStatus, process_instance_status};
    let daemons = DAEMONS.lock().unwrap_or_else(PoisonError::into_inner);
    std::iter::once((std::process::id(), None))
        .chain(
            daemons
                .iter()
                .filter(|(pid, identity)| {
                    process_instance_status(*pid, Some(identity.as_str()))
                        == ProcessInstanceStatus::Same
                })
                .map(|(pid, identity)| (*pid, Some(identity.clone()))),
        )
        .collect()
}

/// Whether the daemon `pid` of start `identity` has exited (or its pid now names another
/// process). This process (`None`) never has.
fn gone(pid: u32, identity: Option<&str>) -> bool {
    use goalcup_protocol::process::{ProcessInstanceStatus, process_instance_status};
    identity.is_some_and(|identity| {
        process_instance_status(pid, Some(identity)) == ProcessInstanceStatus::GoneOrReused
    })
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
    /// cannot be read is left out; a sum is left out when one of its processes that still runs
    /// cannot be read. A daemon that exited while it was read is skipped, and the sum is of the
    /// others.
    pub fn read(&self) -> HeartbeatVitals {
        let free = self.reader.disk_free(&self.trace_dir);
        // Read once: a heartbeat never waits on the state lock. The watch tick writes both what
        // the reservation holds (a state not yet measured says nothing) and the pause (D65).
        let state = crate::run::read_state(&self.run_dir);
        let held = state
            .as_ref()
            .filter(|state| state.disk != crate::run::DiskState::default())
            .map(|state| state.disk.reserve_bytes);
        let paused = state.as_ref().is_some_and(|state| state.disk.disk_paused);
        let usages: Vec<ProcessUsage> = own_processes()
            .into_iter()
            .filter_map(|(pid, identity)| {
                let usage = self.reader.usage(pid);
                (!gone(pid, identity.as_deref())).then_some(usage)
            })
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
            agent_paused: paused.then(|| "disk".to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// This process uses 1 MiB; the daemon exits while it is read, and says nothing.
    struct ExitsWhileRead(Mutex<std::process::Child>);

    impl Reader for ExitsWhileRead {
        fn disk_free(&self, _near: &Path) -> Option<u64> {
            None
        }

        fn usage(&self, pid: u32) -> ProcessUsage {
            if pid == std::process::id() {
                return ProcessUsage {
                    rss_bytes: Some(1 << 20),
                    cpu_ms: Some(10),
                    open_files: Some(3),
                };
            }
            let mut child = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = child.kill();
            let _ = child.wait();
            ProcessUsage {
                rss_bytes: None,
                cpu_ms: None,
                open_files: None,
            }
        }
    }

    #[test]
    fn a_daemon_that_exits_while_it_is_read_does_not_blank_the_sums() {
        let child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        note_daemon(child.id());
        let run_dir = tempfile::tempdir().unwrap();
        let probe = Probe::reading(
            run_dir.path(),
            run_dir.path(),
            Arc::new(ExitsWhileRead(Mutex::new(child))),
        );
        let vitals = probe.read();
        assert_eq!(vitals.rss_bytes, Some(1 << 20));
        assert_eq!(vitals.cpu_ms, Some(10));
        assert_eq!(vitals.open_files, Some(3));
        assert_eq!(vitals.agent_paused, None);
    }
}
