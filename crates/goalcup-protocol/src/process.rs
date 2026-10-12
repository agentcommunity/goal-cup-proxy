//! Process discovery and the launch handle (DECISIONS D22).
//!
//! The CLI spawns the harness (a TUI must inherit the participant's terminal) and hands
//! the daemon a `ProcessHandle`; the daemon terminates at the deadline, when the
//! terminal is long gone. Discovery is shared — the CLI resolves `native_pid` at launch
//! and checks lock holders at resume — so it lives here. `terminate` does not: it is
//! `goalcup_daemon::terminate::terminate`, one implementation, in the half that owns
//! the seal.
//!
//! Discovery goes through `/bin/ps`; the macOS process-instance guard additionally uses
//! `proc_pidinfo` through a target-specific `libc` dependency. macOS and Linux only.

use crate::manifest::KillStrategy;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::process::{Command, Stdio};
use std::time::Duration;

/// How long the verification sweep waits for a SIGKILLed tree to leave the process table.
/// SIGKILL is not instantaneous — a process in an uninterruptible wait leaves when the wait
/// does — and the alternative to a bounded wait is a report that says `all_gone: false`
/// about a tree that was gone a millisecond later.
///
/// Here, in the crate neither the daemon nor the CLI owns, because the CLI derives its finish
/// timeout from it ([`termination_bound`]); the daemon re-exports it unchanged.
pub const KILL_SWEEP_TIMEOUT_MS: u64 = 2_000;

/// How long the V4 seal waits before asking the process table a second, independent time.
///
/// A kill sweep ends the instant one snapshot finds nothing. That snapshot is taken while
/// the tree is still unwinding, and a process in an uninterruptible wait, an atexit handler
/// or a `SIGCHLD` handler can be between two `ps` rows rather than gone. A quarter of a
/// second is long past every exit path a signalled process has and far short of anything a
/// seal notices, and — the point — it is a second observation rather than a longer first
/// one: a re-fork, a respawn or a reparent that happened because of the kill has to survive
/// it to be missed.
pub const RECONFIRM_DELAY_MS: u64 = 250;

/// TERMINATION.md "The duration bound": one grace and one kill sweep per phase (the
/// descendants, then the root), plus the seal's reconfirm. Poll granularity and `ps` spawn
/// time are not included. `termination_bound(10)` is 24.25 s and `termination_bound(0)` is
/// 4.25 s.
pub fn termination_bound(grace_seconds: u64) -> Duration {
    Duration::from_secs(grace_seconds.saturating_mul(2)).saturating_add(Duration::from_millis(
        2 * KILL_SWEEP_TIMEOUT_MS + RECONFIRM_DELAY_MS,
    ))
}

/// What the CLI hands the daemon after a launch, so the daemon can finish the job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessHandle {
    /// The process the CLI spawned. May be a wrapper.
    pub pid: u32,
    /// Set only when the launch could own its process group, which a TUI launch cannot.
    pub pgid: Option<u32>,
    /// Resolved at launch when `kill.native_binary` is set and the process already exists.
    /// The daemon re-resolves at seal, because the native process may appear later.
    pub native_pid: Option<u32>,
    /// The manifest-pinned `kill.native_binary` that identifies the process a wrapper may
    /// hide.  Unlike `program`, which names the launcher the CLI spawned, this is the
    /// immutable native-process identity used for native-PID admission and seal discovery.
    ///
    /// Older persisted handles did not carry it.  They remain readable, but a native-PID
    /// hint without this identity is never accepted as new termination authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_binary: Option<String>,
    /// OS creation identity for `native_pid`, captured by the CLI when the hinted process
    /// exists. It prevents cleanup and Prepared replay from treating a recycled descendant
    /// PID as the native process this launch observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_process_start: Option<String>,
    /// The OS creation identity of the registered root, captured by the CLI immediately
    /// after spawn. It distinguishes a new process that reused the same pid and argv from
    /// the original launcher. Linux uses its boot ID plus `/proc/<pid>/stat` start tick;
    /// macOS uses `proc_pidinfo`'s microsecond creation timestamp. This field remains a
    /// sampled root-reuse guard; the daemon separately acquires an instance-bound signal
    /// authority where the OS supplies one.
    ///
    /// This is additive for on-disk readability. Missing identities from older rows never
    /// authorize a later signal; their seal reports `harness_unverified` instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_start: Option<String>,
    pub strategy: KillStrategy,
    pub grace_seconds: u64,
    pub launched_at: String,
    /// The resolved program path, so a relaunch of a *different* binary with the same
    /// argv is still visible.
    pub program: String,
    /// The argv the CLI actually spawned, after template rendering and the banned-args
    /// guard. The daemon compares a live process's argv against this: a harness running
    /// in the bound workspace whose argv is not this one was not launched by the tracker.
    pub argv: Vec<String>,
    /// The environment keys the CLI pinned, names only, never values (the receiver token
    /// is one of the values). A pinned key missing from the live process is a deviation.
    pub env_keys: Vec<String>,
    /// What this launch's `[home_seed]` carried into the per-run harness home, for the daemon
    /// to chain as `EventBody::HomeSeeded`.
    ///
    /// **It rides here rather than on a route of its own** because a launch calls
    /// `register_process` exactly once — the start does, and each resume does once the daemon
    /// has cleared the standing handle — which is exactly the cadence the record needs. A
    /// second route would be a second surface for a fact that already has a call.
    ///
    /// `None` on a connector that declares no `[home_seed]`, on a seeding that failed, and on
    /// every handle written before this field existed: additive, `serde(default)`, skipped
    /// when absent, so a registry row from an older build reads back exactly as it did.
    ///
    /// Nothing terminates on it. `terminate` reads the pid, the group, the program and the
    /// argv; this field is inert to all four.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_seed: Option<crate::events::HomeSeedRecord>,
}

/// The one authority-bearing request the CLI may post after it launches a harness.
///
/// A bare [`ProcessHandle`] says *what* process the CLI saw, but not which durable launch
/// opening it belongs to.  `generation` is minted by the daemon and advances only in the
/// registry's resume compare-and-swap; a caller cannot reuse a stale response to retarget a
/// later launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessRegistration {
    pub generation: u64,
    pub handle: ProcessHandle,
}

/// `ProcessHandle` is persisted in older registry rows and remains permissive for that
/// additive on-disk compatibility.  The authority-bearing control document is different:
/// it must reject an unknown nested handle field rather than silently accepting a caller's
/// misspelled termination identity.  Keep that stricter wire boundary here instead of
/// retroactively changing deserialization of persisted handles.
impl<'de> Deserialize<'de> for ProcessRegistration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireRegistration {
            generation: u64,
            handle: WireHandle,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireHandle {
            pid: u32,
            pgid: Option<u32>,
            native_pid: Option<u32>,
            #[serde(default)]
            native_binary: Option<String>,
            #[serde(default)]
            native_process_start: Option<String>,
            #[serde(default)]
            process_start: Option<String>,
            strategy: KillStrategy,
            grace_seconds: u64,
            launched_at: String,
            program: String,
            argv: Vec<String>,
            env_keys: Vec<String>,
            #[serde(default)]
            home_seed: Option<WireHomeSeedRecord>,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireSeededKey {
            name: String,
            #[serde(default)]
            value: Option<serde_json::Value>,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireSeededFile {
            name: String,
            bytes: u64,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireHomeSeedRecord {
            #[serde(default)]
            handoff_generation: Option<u64>,
            #[serde(default)]
            handoff_request_digest: Option<String>,
            connector: String,
            from: String,
            #[serde(default)]
            keys: Vec<WireSeededKey>,
            #[serde(default)]
            keys_unchanged: Vec<String>,
            #[serde(default)]
            written: Vec<WireSeededKey>,
            #[serde(default)]
            files: Vec<WireSeededFile>,
            #[serde(default)]
            files_kept: Vec<String>,
            #[serde(default)]
            files_missing: Vec<String>,
        }

        let wire = WireRegistration::deserialize(deserializer)?;
        Ok(ProcessRegistration {
            generation: wire.generation,
            handle: ProcessHandle {
                pid: wire.handle.pid,
                pgid: wire.handle.pgid,
                native_pid: wire.handle.native_pid,
                native_binary: wire.handle.native_binary,
                native_process_start: wire.handle.native_process_start,
                process_start: wire.handle.process_start,
                strategy: wire.handle.strategy,
                grace_seconds: wire.handle.grace_seconds,
                launched_at: wire.handle.launched_at,
                program: wire.handle.program,
                argv: wire.handle.argv,
                env_keys: wire.handle.env_keys,
                home_seed: wire
                    .handle
                    .home_seed
                    .map(|seed| crate::events::HomeSeedRecord {
                        handoff_generation: seed.handoff_generation,
                        handoff_request_digest: seed.handoff_request_digest,
                        connector: seed.connector,
                        from: seed.from,
                        keys: seed
                            .keys
                            .into_iter()
                            .map(|key| crate::events::SeededKey {
                                name: key.name,
                                value: key.value,
                            })
                            .collect(),
                        keys_unchanged: seed.keys_unchanged,
                        written: seed
                            .written
                            .into_iter()
                            .map(|key| crate::events::SeededKey {
                                name: key.name,
                                value: key.value,
                            })
                            .collect(),
                        files: seed
                            .files
                            .into_iter()
                            .map(|file| crate::events::SeededFile {
                                name: file.name,
                                bytes: file.bytes,
                            })
                            .collect(),
                        files_kept: seed.files_kept,
                        files_missing: seed.files_missing,
                        wire_layout: Default::default(),
                    }),
            },
        })
    }
}

impl ProcessRegistration {
    /// A domain-separated canonical identity for the termination authority in this
    /// registration.  It deliberately excludes `home_seed`: configuration evidence is not
    /// a process identity, and hashing it would turn private settings into a stable
    /// correlator.  The daemon may persist this digest and put it in a trace marker without
    /// deriving evidence from the seed contents.
    pub fn canonical_handle_digest(&self, entry_id: &str) -> String {
        #[derive(Serialize)]
        struct Claims<'a> {
            entry_id: &'a str,
            generation: u64,
            pid: u32,
            pgid: Option<u32>,
            native_pid: Option<u32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            native_binary: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            native_process_start: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            process_start: Option<String>,
            strategy: &'a KillStrategy,
            grace_seconds: u64,
            launched_at: &'a str,
            program: &'a str,
            argv: &'a [String],
            env_keys: &'a [String],
        }

        let claims = Claims {
            entry_id,
            generation: self.generation,
            pid: self.handle.pid,
            pgid: self.handle.pgid,
            native_pid: self.handle.native_pid,
            native_binary: self.handle.native_binary.clone(),
            native_process_start: self.handle.native_process_start.clone(),
            process_start: self.handle.process_start.clone(),
            strategy: &self.handle.strategy,
            grace_seconds: self.handle.grace_seconds,
            launched_at: &self.handle.launched_at,
            program: &self.handle.program,
            argv: &self.handle.argv,
            env_keys: &self.handle.env_keys,
        };
        let mut hasher = Sha256::new();
        hasher.update(b"goalcup/process-registration/v1\\0");
        // All fields above are serializable primitive/sequence shapes, so this cannot fail.
        hasher.update(serde_json::to_vec(&claims).expect("canonical process claims serialize"));
        hex::encode(hasher.finalize())
    }

    /// The durable process identity has no place for seed evidence.  Keeping the one
    /// operation here makes it difficult for a registry path to accidentally retain
    /// the seed alongside its recovery marker.
    pub fn stripped_handle(&self) -> ProcessHandle {
        let mut handle = self.handle.clone();
        handle.home_seed = None;
        handle
    }
}

/// A process-instance identity usable as a root-reuse guard.
///
/// Linux's boot ID plus `/proc/<pid>/stat` start tick identifies one kernel process
/// instance. macOS's `proc_pidinfo(PROC_PIDTBSDINFO)` supplies its creation timestamp down
/// to microseconds. Neither value is itself a retained signal handle: callers must compare
/// it while acquiring fresh authority. A zombie (a child that has exited but is not yet
/// reaped) has no identity on either platform. Numeric PID/group fallbacks still report the
/// check-to-signal race as unverified rather than claiming atomic containment.
pub fn process_start_identity(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // Field 2 is parenthesized and may itself contain spaces or `)`, so split only
        // after its final delimiter. Field 22 (`starttime`) is index 19 after `state`.
        let close = stat.rfind(')')?;
        let mut fields = stat.get(close + 2..)?.split_whitespace();
        // A zombie (`Z`) or dead (`X`) process keeps its `/proc` entry and start tick until
        // reaped, but it is no running instance: like macOS's `proc_pidinfo`, report none.
        if matches!(fields.next()?, "Z" | "X") {
            return None;
        }
        let start = fields.nth(18)?.parse::<u64>().ok()?;
        let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .ok()?
            .trim()
            .to_string();
        (!boot_id.is_empty()).then(|| format!("linux:{boot_id}:start-ticks:{start}"))
    }
    #[cfg(target_os = "macos")]
    {
        let pid = i32::try_from(pid).ok()?;
        // SAFETY: `proc_pidinfo` writes exactly `proc_bsdinfo` bytes to the valid local
        // buffer supplied here. The pid is a checked positive `i32`; the API does not
        // retain the pointer.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = i32::try_from(std::mem::size_of::<libc::proc_bsdinfo>()).ok()?;
        // SAFETY: `pid` is a checked `i32`, `info` is a valid, exclusively borrowed local of exactly `size` bytes, and `proc_pidinfo` does not retain the pointer.
        let read = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            )
        };
        (read == size).then(|| {
            format!(
                "darwin-start-us:{}:{}",
                info.pbi_start_tvsec, info.pbi_start_tvusec
            )
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let out = Command::new("/bin/ps")
            .env_clear()
            .args(["-o", "lstart=", "-p", &pid.to_string()])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        let start = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!start.is_empty()).then(|| format!("ps-lstart:{start}"))
    }
}

/// A fail-closed comparison between a cached process instance and the current process
/// table. Only a confirmed absence or a different creation identity proves that the
/// original child is gone. An identity read failure while the numeric pid is still live
/// is `Unknown`, never evidence that cleanup completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessInstanceStatus {
    Same,
    GoneOrReused,
    Unknown,
}

/// Compare one cached creation identity with the current process instance.
pub fn process_instance_status(pid: u32, expected: Option<&str>) -> ProcessInstanceStatus {
    let observed = process_start_identity(pid);
    let presence = if observed.is_some() {
        Some(true)
    } else {
        // No identity, so ask the table: a row that is absent or a zombie (it has exited and
        // only awaits its parent's reap, as on a Linux host whose reaper is slow) is gone;
        // only a failure to ask is unknown.
        liveness(pid).ok()
    };
    classify_process_instance(expected, observed.as_deref(), presence)
}

fn classify_process_instance(
    expected: Option<&str>,
    observed: Option<&str>,
    present: Option<bool>,
) -> ProcessInstanceStatus {
    match (expected, observed, present) {
        (Some(expected), Some(observed), _) if expected == observed => ProcessInstanceStatus::Same,
        (Some(_), Some(_), _) | (_, _, Some(false)) => ProcessInstanceStatus::GoneOrReused,
        _ => ProcessInstanceStatus::Unknown,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    pub comm: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("running {0}")]
    Spawn(String),
    #[error("parsing ps output: {0}")]
    Parse(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("nothing to terminate: {0}")]
    NoTarget(String),
}

/// One `ps` snapshot of every process on the machine.
pub fn snapshot() -> Result<Vec<ProcInfo>, ProcessError> {
    if !cfg!(unix) {
        return Err(ProcessError::Unsupported(
            "process discovery is macOS and Linux only in M1".into(),
        ));
    }
    let out = Command::new("/bin/ps")
        .env_clear()
        .args(["-axo", "pid=,ppid=,comm="])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| ProcessError::Spawn(format!("/bin/ps: {e}")))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut rows = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(a), Some(b)) = (it.next(), it.next()) else {
            continue;
        };
        let (Ok(pid), Ok(ppid)) = (a.parse::<u32>(), b.parse::<u32>()) else {
            continue;
        };
        // `comm` may contain spaces (a full path on Linux); keep the rest of the line.
        let rest: Vec<&str> = it.collect();
        let full = rest.join(" ");
        let comm = full
            .rsplit('/')
            .next()
            .unwrap_or(full.as_str())
            .trim()
            .to_string();
        rows.push(ProcInfo { pid, ppid, comm });
    }
    if rows.is_empty() {
        return Err(ProcessError::Parse("ps returned no usable rows".into()));
    }
    Ok(rows)
}

/// `root` plus every process reachable from it through `ppid`, deepest last.
/// Breadth-first over one snapshot, so a fork during termination cannot loop it.
pub fn descendants(root: u32, table: &[ProcInfo]) -> Vec<u32> {
    let mut out = vec![root];
    let mut queue = VecDeque::from([root]);
    while let Some(p) = queue.pop_front() {
        for row in table.iter().filter(|r| r.ppid == p) {
            if !out.contains(&row.pid) {
                out.push(row.pid);
                queue.push_back(row.pid);
            }
        }
    }
    out
}

/// The nearest descendant of `root` whose `comm` is `binary` — the native process the
/// npm wrapper hides (D22). Returns `None` when the wrapper is the session.
pub fn find_named(root: u32, binary: &str, table: &[ProcInfo]) -> Option<u32> {
    descendants(root, table)
        .into_iter()
        .skip(1)
        .find(|pid| table.iter().any(|r| r.pid == *pid && r.comm == binary))
}

/// Does a current process snapshot bind the optional native-PID hint to this launch?
///
/// A matching `comm` alone is deliberately insufficient: a machine can run many same-name
/// binaries, and a stale or malicious `native_pid` must never give one of them termination
/// authority.  The hinted PID has to be the registered root or a descendant in this exact
/// snapshot, and its `comm` has to equal the manifest-pinned immutable native identity the
/// launch registered.  Callers that receive no hint have no claim to validate and should
/// make their containment decision separately.
pub fn native_pid_is_bound_to_handle(handle: &ProcessHandle, table: &[ProcInfo]) -> bool {
    let (Some(native_pid), Some(native_binary)) =
        (handle.native_pid, handle.native_binary.as_deref())
    else {
        return false;
    };

    table
        .iter()
        .find(|row| row.pid == native_pid)
        .is_some_and(|row| {
            row.comm == native_binary && descendants(handle.pid, table).contains(&native_pid)
        })
}

/// Revalidate a durable process handle immediately before it becomes termination authority.
///
/// A Prepared row is durable signal authority, not merely a historical acknowledgement.
/// It therefore needs an instance identity for the root and for every stored native hint;
/// an older row missing those additive fields must remain Prepared rather than promote a
/// recycled numeric PID. Already-Registered replay is acknowledged before this function.
pub fn validate_live_process_handle(handle: &ProcessHandle) -> Result<(), String> {
    if process_instance_status(handle.pid, handle.process_start.as_deref())
        != ProcessInstanceStatus::Same
    {
        return Err(format!(
            "process_start {:?} no longer identifies the live registered pid {}",
            handle.process_start, handle.pid
        ));
    }
    if let Some(native_pid) = handle.native_pid {
        let table = snapshot()
            .map_err(|error| format!("cannot revalidate native_pid from a snapshot: {error}"))?;
        if !native_pid_is_bound_to_handle(handle, &table) {
            return Err(format!(
                "native_pid {:?} is no longer the registered root or a descendant running {:?}",
                handle.native_pid, handle.native_binary
            ));
        }
        if process_instance_status(native_pid, handle.native_process_start.as_deref())
            != ProcessInstanceStatus::Same
        {
            return Err(format!(
                "native_process_start {:?} no longer identifies native_pid {:?}",
                handle.native_process_start, handle.native_pid
            ));
        }
    }
    Ok(())
}

/// Is `pid` a live, non-zombie process — or could the process table not be asked?
///
/// `Err` is the one answer [`is_alive`] cannot give: `/bin/ps` failed to SPAWN, so nothing
/// was measured. It is distinct from `Ok(false)`, which is a row that was read and found
/// absent or `Z`. [`snapshot`] draws the same line with `ProcessError::Spawn`; this is the
/// per-pid half of it. Nothing in the shipped decision acts on the `Err` — every
/// caller goes through [`is_alive`], which folds it to `false` — but a termination
/// that could not ask is a different fact from one that asked and heard "gone", and the
/// seam that terminates a tree needs to be able to see the difference.
///
/// A zombie counts as gone: the caller that owns the `Child` reaps it afterwards, and
/// treating an unreaped exit as "still running" would make every report say
/// `all_gone: false`.
pub fn liveness(pid: u32) -> Result<bool, ProcessError> {
    let out = Command::new("/bin/ps")
        .env_clear()
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| ProcessError::Spawn(format!("/bin/ps: {e}")))?;
    let s = String::from_utf8_lossy(&out.stdout);
    let t = s.trim();
    Ok(!t.is_empty() && !t.starts_with('Z'))
}

/// [`liveness`] with "could not ask" folded to "gone". This is the shipped decision every
/// caller makes; see [`liveness`] for why the two are kept apart.
pub fn is_alive(pid: u32) -> bool {
    liveness(pid).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{HomeSeedRecord, SeededFile, SeededKey};

    fn registration() -> ProcessRegistration {
        ProcessRegistration {
            generation: 7,
            handle: ProcessHandle {
                pid: 4242,
                pgid: Some(4242),
                native_pid: Some(4243),
                native_binary: Some("native-harness".to_string()),
                native_process_start: Some("native-fixture-start".to_string()),
                process_start: Some("fixture-start".to_string()),
                strategy: KillStrategy::NativePid,
                grace_seconds: 5,
                launched_at: "2026-09-10T00:00:00Z".to_string(),
                // A shell launcher is intentionally not the native process whose identity
                // a manifest pins. This is the `sh -> native-harness` case that must remain
                // admissible without falling back to same-name matching.
                program: "/bin/sh".to_string(),
                argv: vec!["sh".to_string(), "./wrapper.sh".to_string()],
                env_keys: vec!["HOME".to_string()],
                home_seed: None,
            },
        }
    }

    /// A child that has exited but not been reaped is a zombie: its pid still answers
    /// `kill(pid, 0)`, but it is no running process instance and has no creation identity
    /// to compare. macOS's `proc_pidinfo` already says so; Linux must not read the dead
    /// child's `/proc/<pid>/stat` start tick as though it were alive.
    #[cfg(unix)]
    #[test]
    fn process_start_identity_is_none_for_an_unreaped_dead_child() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .stdin(Stdio::null())
            .spawn()
            .expect("spawn a child that exits at once");
        let pid = child.id();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let zombie = || {
            #[cfg(target_os = "linux")]
            {
                std::fs::read_to_string(format!("/proc/{pid}/stat"))
                    .ok()
                    .and_then(|stat| {
                        let close = stat.rfind(')')?;
                        stat.get(close + 2..)?.chars().next()
                    })
                    == Some('Z')
            }
            #[cfg(not(target_os = "linux"))]
            {
                matches!(liveness(pid), Ok(false))
            }
        };
        while !zombie() {
            assert!(
                std::time::Instant::now() < deadline,
                "the child never became a zombie"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let identity = process_start_identity(pid);
        let status = process_instance_status(pid, Some("cached"));
        child.wait().expect("reap the child");
        assert_eq!(identity, None, "a zombie has no process-start identity");
        assert_eq!(
            status,
            ProcessInstanceStatus::GoneOrReused,
            "a zombie is a gone instance, not an unknown one"
        );
    }

    #[test]
    fn canonical_registration_digest_binds_every_termination_identity_field_but_never_seed_evidence()
     {
        let bare = registration();
        let mut seeded = bare.clone();
        seeded.handle.home_seed = Some(HomeSeedRecord {
            handoff_generation: None,
            handoff_request_digest: None,
            connector: "fixture".to_string(),
            from: "{home}/private-settings.json".to_string(),
            keys: vec![SeededKey::new(
                "sensitive-setting",
                &serde_json::json!("seed-content-must-not-define-process-identity"),
            )],
            keys_unchanged: Vec::new(),
            written: vec![SeededKey::new("written-setting", &serde_json::json!(false))],
            files: vec![SeededFile {
                name: "settings.json".to_string(),
                bytes: 42,
            }],
            files_kept: Vec::new(),
            files_missing: Vec::new(),
            wire_layout: Default::default(),
        });
        let digest = bare.canonical_handle_digest("entry-a");
        assert_eq!(digest, seeded.canonical_handle_digest("entry-a"));
        let mut different_seed = seeded;
        let seed = different_seed
            .handle
            .home_seed
            .as_mut()
            .expect("the alternate seed exists");
        seed.handoff_generation = Some(99);
        seed.handoff_request_digest = Some("cd".repeat(32));
        seed.connector = "other-fixture".to_string();
        seed.from = "{home}/other-private-settings.json".to_string();
        seed.keys[0] = SeededKey::new("other-setting", &serde_json::json!(false));
        seed.keys_unchanged.push("unchanged-setting".to_string());
        seed.written[0] = SeededKey::new("other-written-setting", &serde_json::json!(true));
        seed.files[0] = SeededFile {
            name: "other-settings.json".to_string(),
            bytes: 99,
        };
        seed.files_kept.push("copied.json".to_string());
        seed.files_missing.push("missing.json".to_string());
        assert_eq!(
            digest,
            different_seed.canonical_handle_digest("entry-a"),
            "missing, present, and different home-seed evidence all leave process identity alone"
        );
        let mut cases: Vec<(&str, ProcessRegistration)> = Vec::new();
        let mut changed = bare.clone();
        changed.generation += 1;
        cases.push(("generation", changed));
        let mut changed = bare.clone();
        changed.handle.pid += 1;
        cases.push(("pid", changed));
        let mut changed = bare.clone();
        changed.handle.pgid = None;
        cases.push(("pgid", changed));
        let mut changed = bare.clone();
        changed.handle.native_pid = None;
        cases.push(("native_pid", changed));
        let mut changed = bare.clone();
        changed.handle.native_binary = Some("other-native-harness".to_string());
        cases.push(("native_binary", changed));
        let mut changed = bare.clone();
        changed.handle.native_process_start = Some("other-native-fixture-start".to_string());
        cases.push(("native_process_start", changed));
        let mut changed = bare.clone();
        changed.handle.process_start = Some("other-fixture-start".to_string());
        cases.push(("process_start", changed));
        let mut changed = bare.clone();
        changed.handle.strategy = KillStrategy::ProcessGroup;
        cases.push(("strategy", changed));
        let mut changed = bare.clone();
        changed.handle.grace_seconds += 1;
        cases.push(("grace_seconds", changed));
        let mut changed = bare.clone();
        changed.handle.launched_at.push('1');
        cases.push(("launched_at", changed));
        let mut changed = bare.clone();
        changed.handle.program.push_str(".other");
        cases.push(("program", changed));
        let mut changed = bare.clone();
        changed.handle.argv.push("--changed".to_string());
        cases.push(("argv", changed));
        let mut changed = bare.clone();
        changed.handle.env_keys.push("NEW_PIN".to_string());
        cases.push(("env_keys", changed));

        for (field, changed) in cases {
            assert_ne!(
                digest,
                changed.canonical_handle_digest("entry-a"),
                "{field} is termination authority"
            );
        }
        assert_ne!(
            digest,
            bare.canonical_handle_digest("entry-b"),
            "entry id is bound"
        );
    }

    #[test]
    fn process_registration_rejects_unknown_fields_at_every_authority_bearing_nesting_level() {
        let mut registered = registration();
        registered.handle.home_seed = Some(HomeSeedRecord {
            handoff_generation: Some(7),
            handoff_request_digest: Some("ab".repeat(32)),
            connector: "fixture".to_string(),
            from: "{home}/private-settings.json".to_string(),
            keys: vec![SeededKey::new("setting", &serde_json::json!(true))],
            keys_unchanged: Vec::new(),
            written: vec![SeededKey::new("written", &serde_json::json!(false))],
            files: vec![SeededFile {
                name: "settings.json".to_string(),
                bytes: 42,
            }],
            files_kept: Vec::new(),
            files_missing: Vec::new(),
            wire_layout: Default::default(),
        });
        let original = serde_json::to_value(registered).expect("the fixture serialises");
        let mut variants = Vec::new();

        let mut top = original.clone();
        top.as_object_mut()
            .unwrap()
            .insert("unexpected".to_string(), serde_json::json!(true));
        variants.push(("registration", top));

        let mut handle = original.clone();
        handle["handle"]
            .as_object_mut()
            .unwrap()
            .insert("unexpected".to_string(), serde_json::json!(true));
        variants.push(("handle", handle));

        let mut seed = original.clone();
        seed["handle"]["home_seed"]
            .as_object_mut()
            .unwrap()
            .insert("unexpected".to_string(), serde_json::json!(true));
        variants.push(("home_seed", seed));

        let mut key = original.clone();
        key["handle"]["home_seed"]["keys"][0]
            .as_object_mut()
            .unwrap()
            .insert("unexpected".to_string(), serde_json::json!(true));
        variants.push(("keys item", key));

        let mut written = original.clone();
        written["handle"]["home_seed"]["written"][0]
            .as_object_mut()
            .unwrap()
            .insert("unexpected".to_string(), serde_json::json!(true));
        variants.push(("written item", written));

        let mut file = original;
        file["handle"]["home_seed"]["files"][0]
            .as_object_mut()
            .unwrap()
            .insert("unexpected".to_string(), serde_json::json!(true));
        variants.push(("files item", file));

        for (level, value) in variants {
            assert!(
                serde_json::from_value::<ProcessRegistration>(value).is_err(),
                "unknown field in the {level} authority body must fail closed"
            );
        }
    }

    #[test]
    fn process_start_identity_is_a_nonempty_instance_binding_for_this_process() {
        let identity = process_start_identity(std::process::id())
            .expect("the current process has an OS creation identity");
        #[cfg(target_os = "linux")]
        assert!(
            identity.starts_with("linux:") && identity.contains(":start-ticks:"),
            "linux identity binds boot instance and start ticks: {identity}"
        );
        #[cfg(target_os = "macos")]
        assert!(
            identity.starts_with("darwin-start-us:") && identity.matches(':').count() >= 2,
            "macOS identity carries proc_pidinfo microseconds: {identity}"
        );
    }

    #[test]
    fn cleanup_identity_is_fail_closed_when_the_pid_is_live_but_identity_is_unreadable() {
        assert_eq!(
            classify_process_instance(Some("cached"), None, Some(true)),
            ProcessInstanceStatus::Unknown
        );
        assert_eq!(
            classify_process_instance(Some("cached"), None, None),
            ProcessInstanceStatus::Unknown
        );
        assert_eq!(
            classify_process_instance(Some("cached"), None, Some(false)),
            ProcessInstanceStatus::GoneOrReused
        );
        assert_eq!(
            classify_process_instance(Some("cached"), Some("other"), Some(true)),
            ProcessInstanceStatus::GoneOrReused
        );
    }

    #[test]
    fn native_hint_requires_the_registered_tree_and_manifest_native_identity() {
        let registration = registration();
        let table = vec![
            ProcInfo {
                pid: 4242,
                ppid: 1,
                comm: "wrapper".to_string(),
            },
            ProcInfo {
                pid: 4243,
                ppid: 4242,
                comm: "native-harness".to_string(),
            },
            ProcInfo {
                pid: 4244,
                ppid: 1,
                comm: "native-harness".to_string(),
            },
        ];
        assert!(native_pid_is_bound_to_handle(&registration.handle, &table));

        let mut unrelated = registration.handle.clone();
        unrelated.native_pid = Some(4244);
        assert!(
            !native_pid_is_bound_to_handle(&unrelated, &table),
            "the same binary name outside the registered tree is a stranger"
        );

        let mut wrong_identity = registration.handle.clone();
        wrong_identity.native_pid = Some(4242);
        assert!(
            !native_pid_is_bound_to_handle(&wrong_identity, &table),
            "the root is admissible only when it runs the registered native program"
        );
        let mut root_as_native = table.clone();
        root_as_native[0].comm = "native-harness".to_string();
        assert!(native_pid_is_bound_to_handle(
            &wrong_identity,
            &root_as_native,
        ));

        let mut no_hint = registration.handle.clone();
        no_hint.native_pid = None;
        assert!(!native_pid_is_bound_to_handle(&no_hint, &table));

        let mut unpinned = registration.handle.clone();
        unpinned.native_binary = None;
        assert!(
            !native_pid_is_bound_to_handle(&unpinned, &table),
            "a native PID needs an immutable identity; launcher basename is not a fallback"
        );
    }
}
