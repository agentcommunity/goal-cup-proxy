//! Ending the capture child this process has just released (the root ruling under D40).
//!
//! The capture spawn gate (`launch::spawn_gated`) releases a parked child only after the daemon
//! has durably registered it. If the entry is then found sealed (a seal raced in between the
//! registration and the release, and cannot have ended a child that was still parked), a harness
//! would be running on a sealed entry. This command ends the child it just started, here, and
//! says so.
//!
//! **This is not a second terminator.** It acts only on this process's own unreaped `Child`
//! (whose pid cannot have been recycled) and on that child's native descendant bound by its
//! creation identity. It never touches an entry, a seal or a trace; `goalcup_daemon::terminate`
//! remains the only terminator of a recorded run. Other code in this crate also signals
//! processes (the abandon path in `commands/start.rs`, the version probe, the Linux subreaper
//! sweep); `tests/it/process_handoff.rs` (`this_crate_never_terminates_anything`) exempts each
//! file by name and gives its reason.
//!
//! The CLI cannot link the daemon, so only protocol functions are used.

use goalcup_protocol::process::ProcessHandle;
#[cfg(unix)]
use goalcup_protocol::process::{
    ProcessInstanceStatus, find_named, is_alive, process_instance_status, process_start_identity,
    snapshot,
};
use std::fmt;
use std::process::Child;

/// What [`end_released_child`] observed: printed, never chained (the entry is already sealed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleasedChildEnd {
    /// The root had exited before any signal and the connector names no native binary;
    /// nothing was signalled. (With a named native binary the answer is `NativeUnverified`.)
    AlreadyGone,
    /// Every targeted instance was observed gone (root reaped; native by `process_start`).
    Ended { termed: Vec<u32>, killed: Vec<u32> },
    /// A targeted instance outlived SIGKILL and the kill sweep.
    StillRunning { pids: Vec<u32> },
    /// The root ended (or had already ended), but no native instance could be checked: the
    /// connector names a native binary and none was ever observed (it may not have been forked
    /// yet, may have escaped before the process table was read, or the root left alone), or the
    /// table could not be read.
    NativeUnverified { termed: Vec<u32>, killed: Vec<u32> },
}

impl fmt::Display for ReleasedChildEnd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |pids: &[u32]| {
            if pids.is_empty() {
                "none".to_string()
            } else {
                pids.iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        };
        match self {
            ReleasedChildEnd::AlreadyGone => {
                write!(f, "it had already exited and nothing was signalled")
            }
            ReleasedChildEnd::Ended { termed, killed } => write!(
                f,
                "ended (asked to stop: {}; killed: {})",
                list(termed),
                list(killed)
            ),
            ReleasedChildEnd::StillRunning { pids } => write!(
                f,
                "still running after the grace period and a kill (pids: {}); end it by hand",
                list(pids)
            ),
            ReleasedChildEnd::NativeUnverified { termed, killed } => write!(
                f,
                "its root ended (asked to stop: {}; killed: {}), but its native child could not be \
                 checked and may still be running",
                list(termed),
                list(killed)
            ),
        }
    }
}

impl ReleasedChildEnd {
    /// The first sentence the launcher prints: what it did, as far as the outcome says it did it.
    pub fn announcement(&self, entry_id: &str) -> String {
        let did = match self {
            ReleasedChildEnd::AlreadyGone | ReleasedChildEnd::Ended { .. } => {
                "ended the harness it had just started"
            }
            ReleasedChildEnd::StillRunning { .. } | ReleasedChildEnd::NativeUnverified { .. } => {
                "tried to end the harness it had just started"
            }
        };
        format!(
            "Entry {entry_id} was sealed while its harness was being launched, so this command \
             {did}: {self}. The seal is the record; this ending is not in it."
        )
    }

    /// The last line, which is the command's refusal. It says nothing is running only when the
    /// outcome says the root and any native child are observed gone; otherwise it says the
    /// harness may still be running and points at the line above.
    pub fn refusal(&self, entry_id: &str) -> String {
        let tail = match self {
            ReleasedChildEnd::Ended { .. } => "its harness was ended and is no longer running.",
            ReleasedChildEnd::AlreadyGone => "its harness had already exited.",
            ReleasedChildEnd::StillRunning { .. } => {
                "its harness is STILL RUNNING (see the line above): end it by hand."
            }
            ReleasedChildEnd::NativeUnverified { .. } => {
                "its harness's root was ended, but a native child could not be checked and may \
                 still be running (see the line above)."
            }
        };
        format!("entry {entry_id} was sealed before its launch could be recorded, so {tail}")
    }
}

#[cfg(unix)]
struct NativeTarget {
    pid: u32,
    start: String,
}

#[cfg(unix)]
impl NativeTarget {
    /// The native child of `root` named `binary`, with its creation identity taken at once.
    fn find(root: u32, binary: &str) -> Option<NativeTarget> {
        let table = snapshot().ok()?;
        let pid = find_named(root, binary, &table)?;
        let start = process_start_identity(pid)?;
        Some(NativeTarget { pid, start })
    }

    fn is_same_instance(&self) -> bool {
        process_instance_status(self.pid, Some(&self.start)) == ProcessInstanceStatus::Same
    }

    /// Fail closed: an instance whose status could not be read is not gone.
    fn is_gone(&self) -> bool {
        match process_instance_status(self.pid, Some(&self.start)) {
            ProcessInstanceStatus::GoneOrReused => true,
            ProcessInstanceStatus::Same => !is_alive(self.pid),
            ProcessInstanceStatus::Unknown => false,
        }
    }

    /// Signal the instance only while it is still the one that was recorded. The numeric
    /// signal carries the same check-to-signal race as `kill_retained_native`.
    fn signal(&self, signal: i32) -> bool {
        let Ok(raw) = i32::try_from(self.pid) else {
            return false;
        };
        if !self.is_same_instance() {
            return false;
        }
        // SAFETY: two integers; the pid was just checked to be the recorded instance.
        unsafe { libc::kill(raw, signal) == 0 }
    }
}

/// End this process's own just-released capture child within `termination_bound(grace)`.
///
/// The CLI cannot link the daemon, so this uses protocol functions only. The root is this
/// process's own unreaped `Child`, so its pid cannot have been recycled. A `native_pid`
/// connector's native child is re-resolved on **every** poll while the root lives, because the
/// wrapper usually has not forked it when the child is released; it is adopted, bound by
/// `process_start`, the first time it appears, and signalled first. If the connector names a
/// native binary and none was ever observed the result is [`ReleasedChildEnd::NativeUnverified`],
/// never `Ended`.
///
/// What it does not claim: descendants other than the root and that native child are never
/// signalled (the D30 residual). The native child's parentage is read in one `ps` snapshot and
/// its creation identity a moment later; a pid reused between the two would be adopted under the
/// newcomer's identity (stated in CONTRACTS.md, C6 capture spawn gate ¶5(d)).
#[cfg(unix)]
pub fn end_released_child(mut child: Child, handle: &ProcessHandle) -> ReleasedChildEnd {
    use goalcup_protocol::process::KILL_SWEEP_TIMEOUT_MS;
    const POLL: std::time::Duration = std::time::Duration::from_millis(50);

    let binary = handle.native_binary.as_deref();
    if matches!(child.try_wait(), Ok(Some(_))) {
        // The root left by itself. A connector that names a native binary may have a native
        // child that was reparented and cannot be found from here: unverified, not gone.
        return if binary.is_some() {
            ReleasedChildEnd::NativeUnverified {
                termed: Vec::new(),
                killed: Vec::new(),
            }
        } else {
            ReleasedChildEnd::AlreadyGone
        };
    }
    let root = child.id();
    let mut native: Option<NativeTarget> = None;
    let mut termed = Vec::new();
    let mut killed = Vec::new();
    let reaped = |child: &mut Child| matches!(child.try_wait(), Ok(Some(_)));
    let adopt = |native: &mut Option<NativeTarget>| -> bool {
        if native.is_none()
            && let Some(binary) = binary
        {
            *native = NativeTarget::find(root, binary);
            return native.is_some();
        }
        false
    };

    adopt(&mut native);
    if let Some(target) = &native
        && target.signal(libc::SIGTERM)
    {
        termed.push(target.pid);
    }
    // SAFETY: two integers; the root is this process's own unreaped child.
    unsafe {
        libc::kill(root as i32, libc::SIGTERM);
    }
    termed.push(root);

    let mut root_gone = false;
    let grace_until =
        std::time::Instant::now() + std::time::Duration::from_secs(handle.grace_seconds);
    loop {
        root_gone = root_gone || reaped(&mut child);
        if !root_gone && adopt(&mut native) {
            let target = native.as_ref().expect("just adopted");
            if target.signal(libc::SIGTERM) {
                termed.push(target.pid);
            }
        }
        if root_gone && native.as_ref().is_none_or(NativeTarget::is_gone) {
            break;
        }
        if std::time::Instant::now() >= grace_until {
            break;
        }
        std::thread::sleep(POLL);
    }

    let native_gone =
        |native: &Option<NativeTarget>| native.as_ref().is_none_or(NativeTarget::is_gone);
    if !(root_gone && native_gone(&native)) {
        if let Some(target) = &native
            && !target.is_gone()
            && target.signal(libc::SIGKILL)
        {
            killed.push(target.pid);
        }
        if !root_gone {
            let _ = child.kill();
            killed.push(root);
        }
        let sweep_until =
            std::time::Instant::now() + std::time::Duration::from_millis(KILL_SWEEP_TIMEOUT_MS);
        loop {
            root_gone = root_gone || reaped(&mut child);
            if !root_gone && adopt(&mut native) {
                let target = native.as_ref().expect("just adopted");
                if target.signal(libc::SIGKILL) {
                    killed.push(target.pid);
                }
            }
            if root_gone && native_gone(&native) {
                break;
            }
            if std::time::Instant::now() >= sweep_until {
                break;
            }
            std::thread::sleep(POLL);
        }
    }

    let mut pids = Vec::new();
    if !root_gone {
        pids.push(root);
    }
    if let Some(target) = &native
        && !target.is_gone()
    {
        pids.push(target.pid);
    }
    if !pids.is_empty() {
        // The child is dropped unreaped rather than waited for: nothing here blocks.
        return ReleasedChildEnd::StillRunning { pids };
    }
    if binary.is_some() && native.is_none() {
        return ReleasedChildEnd::NativeUnverified { termed, killed };
    }
    ReleasedChildEnd::Ended { termed, killed }
}
