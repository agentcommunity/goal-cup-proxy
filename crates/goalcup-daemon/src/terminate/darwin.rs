use super::authority::{Presence, degraded_guard, guard_registered};
use super::os::{ForkWitness, InstanceAuthority, ProcessOs, SignalOutcome, waited_ms};
use super::{
    KILL_SWEEP_TIMEOUT_MS, LIVENESS_POLL_MS, MIN_SIGNALLABLE_PID, TerminationReport, grace,
};
use goalcup_protocol::process::{ProcInfo, ProcessHandle, descendants};
use std::time::{Duration, Instant};
use std::{
    os::fd::{FromRawFd, OwnedFd},
    ptr,
};

/// A manifest that pins a native binary may call the registered process itself native.
/// Otherwise a singleton wrapper is not proof that the native session has appeared, and
/// the narrow Darwin exact-root path must not target it merely because no child was visible.
#[cfg(target_os = "macos")]
pub(super) fn direct_native_identity_matches(handle: &ProcessHandle, table: &[ProcInfo]) -> bool {
    let Some(native_binary) = handle.native_binary.as_deref() else {
        return true;
    };
    table
        .iter()
        .find(|row| row.pid == handle.pid)
        .is_some_and(|row| row.comm == native_binary)
        && handle.native_pid.is_none_or(|pid| pid == handle.pid)
}

/// Terminate one authenticated, group-less Darwin process through its audit token.
///
/// This is not a numeric `kill(pid)`: the kernel checks the audit token's PID version while
/// delivering the signal. `EVFILT_PROC` is installed before the final root/tree validation,
/// so it witnesses exact root exit and later forks/execs. It cannot witness a descendant
/// that forked and reparented before acquisition, so every Darwin result remains tree-wide
/// unverified even when the root is exactly gone. Failure to acquire both capabilities falls
/// back to the existing best-effort path. After acquisition, any validation/witness/signal
/// error returns unverified without ever retrying through a numeric PID.
#[cfg(target_os = "macos")]
pub(super) fn terminate_darwin_singleton(
    os: &dyn ProcessOs,
    handle: &ProcessHandle,
    started: Instant,
) -> Option<TerminationReport> {
    let target = os.instance_authority(handle.pid)?;
    Some(terminate_darwin_singleton_with_authority(
        os, handle, started, target,
    ))
}

#[cfg(target_os = "macos")]
fn terminate_darwin_singleton_with_authority(
    os: &dyn ProcessOs,
    handle: &ProcessHandle,
    started: Instant,
    target: Box<dyn InstanceAuthority>,
) -> TerminationReport {
    let mut attempt = DarwinTerminationAttempt::new(os, handle, started, target);

    // Close both acquisition races: the root must still be the registered process after the
    // audit token and kqueue attachment exist, and the fresh tree must still be a singleton.
    // Once acquisition succeeded, no failure may fall through to the numeric path: an exec
    // can stale the token while leaving the same PID live, and a numeric retry could then hit
    // a later replacement. The report stays unverified and no unbound signal is attempted.
    if !matches!(guard_registered(os, handle), Ok(Presence::Live)) {
        return attempt.finish();
    }
    match attempt.target.is_alive() {
        Some(true) => {}
        // The process exited before the post-acquisition tree snapshot, so there is no
        // authenticated singleton observation inside the witnessed interval.
        Some(false) => return attempt.finish(),
        None => return attempt.finish(),
    }
    let table = match os.snapshot() {
        Ok(table) => table,
        Err(_) => return attempt.finish(),
    };
    if !table.iter().any(|row| row.pid == handle.pid)
        || descendants(handle.pid, &table) != [handle.pid]
        || !direct_native_identity_matches(handle, &table)
    {
        return attempt.finish();
    }

    match attempt.target.signal(libc::SIGTERM) {
        Some(SignalOutcome::Delivered) => attempt.targeted = true,
        Some(SignalOutcome::Stale) => {
            // ESRCH can mean exact exit or an exec-induced PID-version change. Only the
            // kqueue witness distinguishes them; neither permits a numeric retry.
            return attempt.finish();
        }
        None => return attempt.finish(),
    }

    let grace_ends = os.now() + grace(handle.grace_seconds);
    let gone_after_term = match wait_until_exit(os, attempt.target.as_mut(), grace_ends) {
        Some(gone) => gone,
        None => return attempt.finish(),
    };
    if !gone_after_term {
        attempt.survived_term.push(handle.pid);
        match attempt.target.signal(libc::SIGKILL) {
            Some(SignalOutcome::Delivered) => attempt.killed.push(handle.pid),
            Some(SignalOutcome::Stale) => {}
            None => return attempt.finish(),
        }
    }

    let sweep_ends = os.now() + Duration::from_millis(KILL_SWEEP_TIMEOUT_MS);
    if wait_until_exit(os, attempt.target.as_mut(), sweep_ends).is_none() {
        return attempt.finish();
    }

    attempt.finish()
}

/// Wait until exact exit or the caller's deadline. `false` means the process is still
/// live at the deadline; `None` means the witness failed and no proof may be claimed.
#[cfg(target_os = "macos")]
fn wait_until_exit(
    os: &dyn ProcessOs,
    target: &mut dyn InstanceAuthority,
    deadline: Instant,
) -> Option<bool> {
    loop {
        match target.is_alive()? {
            false => return Some(true),
            true if os.now() >= deadline => return Some(false),
            true => os.sleep(Duration::from_millis(LIVENESS_POLL_MS)),
        }
    }
}

/// Report state accumulated after exact Darwin authority was acquired.
#[cfg(target_os = "macos")]
struct DarwinTerminationAttempt<'a> {
    os: &'a dyn ProcessOs,
    handle: &'a ProcessHandle,
    started: Instant,
    target: Box<dyn InstanceAuthority>,
    targeted: bool,
    survived_term: Vec<u32>,
    killed: Vec<u32>,
}

#[cfg(target_os = "macos")]
impl<'a> DarwinTerminationAttempt<'a> {
    fn new(
        os: &'a dyn ProcessOs,
        handle: &'a ProcessHandle,
        started: Instant,
        target: Box<dyn InstanceAuthority>,
    ) -> DarwinTerminationAttempt<'a> {
        DarwinTerminationAttempt {
            os,
            handle,
            started,
            target,
            targeted: false,
            survived_term: Vec::new(),
            killed: Vec::new(),
        }
    }

    fn finish(mut self) -> TerminationReport {
        let observed = self.target.observed_state();
        let gone = observed.map(|(gone, _)| gone);
        // A fork or exec witnessed after acquisition means the set this path was authorised
        // over — the one-process tree the post-acquisition snapshot proved — is not the set
        // that died. The witness is the only thing that can say so, and it fails closed.
        let grew = observed.map(|(_, grew)| grew) != Some(false);
        TerminationReport {
            strategy: self.handle.strategy,
            targeted: self
                .targeted
                .then_some(self.handle.pid)
                .into_iter()
                .collect(),
            survived_term: self.survived_term,
            killed: self.killed,
            still_alive: (gone != Some(true))
                .then_some(self.handle.pid)
                .into_iter()
                .collect(),
            // This watcher begins at seal. Exact root exit cannot establish that no child
            // forked and reparented before then, so it is not a full-tree proof.
            all_gone: false,
            // What this path can say, and says without the numeric-signal caveat the generic
            // path carries: the signal was delivered against a kernel-validated PID version,
            // the post-acquisition snapshot proved the owned set was this one process, the
            // kqueue witnessed that exact instance exit, and it witnessed no fork or exec
            // from it in between. The residual gap is the same one the whole module
            // declares — a descendant that left the lineage before the watcher existed.
            observed_gone: self.targeted && gone == Some(true) && !grew,
            walk_cap_exceeded: false,
            waited_ms: waited_ms(self.os, self.started),
            guard_degraded: degraded_guard(self.handle),
            harness_unverified: true,
        }
    }
}

/// Kernel-bound authority for one Darwin process instance plus a fork/exit witness.
///
/// This is what [`super::os::LiveOs`] hands out as both an [`InstanceAuthority`] and a bare
/// [`ForkWitness`]: the kqueue is the witness half, the audit token the signal half, and
/// they are acquired together because the token without the witness cannot tell an exit
/// from an exec.
#[cfg(target_os = "macos")]
pub(super) struct DarwinProcessAuthority {
    token: DarwinAuditToken,
    events: OwnedFd,
    signal_with_token: DarwinAuditSignalFn,
    saw_tree_change: bool,
    saw_exit: bool,
    /// A fork or exec seen since the last [`ForkWitness::take_growth`]; `saw_tree_change`
    /// is the same fact for the whole life of the witness.
    growth_pending: bool,
}

#[cfg(target_os = "macos")]
type DarwinAuditSignalFn = unsafe extern "C" fn(*mut DarwinAuditToken, i32) -> i32;

#[cfg(target_os = "macos")]
impl DarwinProcessAuthority {
    pub(super) fn acquire(pid: u32) -> Option<Self> {
        Self::acquire_with_signal(pid, darwin_audit_signal())
    }

    fn acquire_with_signal(
        pid: u32,
        signal_with_token: Option<DarwinAuditSignalFn>,
    ) -> Option<Self> {
        if pid < MIN_SIGNALLABLE_PID {
            return None;
        }
        let signal_with_token = signal_with_token?;
        let pid = i32::try_from(pid).ok()?;
        let mut task = libc::MACH_PORT_NULL as libc::mach_port_t;
        let self_task = mach_task_self();
        // SAFETY: `task_name_for_pid` writes one Mach port name to `task`; it retains no
        // userspace pointer. The checked positive pid names the process being acquired.
        if unsafe { task_name_for_pid(self_task, pid, &mut task) } != libc::KERN_SUCCESS {
            return None;
        }

        let mut token = DarwinAuditToken { val: [0; 8] };
        let mut count = 8;
        // SAFETY: `DarwinAuditToken` is the C `audit_token_t` layout: eight natural words.
        // `task_info` writes at most the supplied count into its valid local buffer.
        let info = unsafe {
            libc::task_info(
                task,
                15, // TASK_AUDIT_TOKEN
                token.val.as_mut_ptr().cast(),
                &mut count,
            )
        };
        // SAFETY: `task` is the send right returned above and belongs to this process.
        unsafe {
            mach_port_deallocate(self_task, task);
        }
        if info != libc::KERN_SUCCESS || count != 8 {
            return None;
        }

        // SAFETY: `kqueue` returns a new owned descriptor or -1.
        let raw = unsafe { libc::kqueue() };
        if raw < 0 {
            return None;
        }
        // SAFETY: ownership of the fresh nonnegative descriptor transfers exactly once.
        let events = unsafe { OwnedFd::from_raw_fd(raw) };
        let change = libc::kevent {
            ident: usize::try_from(pid).ok()?,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_CLEAR,
            fflags: libc::NOTE_FORK | libc::NOTE_EXEC | libc::NOTE_EXIT,
            data: 0,
            udata: ptr::null_mut(),
        };
        let timeout = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: all pointers name initialized local values for the exact declared lengths;
        // the zero timeout makes registration nonblocking.
        if unsafe { libc::kevent(raw, &change, 1, ptr::null_mut(), 0, &timeout) } < 0 {
            return None;
        }

        Some(Self {
            token,
            events,
            signal_with_token,
            saw_tree_change: false,
            saw_exit: false,
            growth_pending: false,
        })
    }

    fn raw_signal(&mut self, sig: i32) -> i32 {
        // SAFETY: the kernel produced this audit token for the acquired task. The API reads
        // the fixed-size token and atomically validates its PID version before signalling.
        unsafe { (self.signal_with_token)(&mut self.token, sig) }
    }

    fn poll_events(&mut self) -> Option<()> {
        let timeout = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        loop {
            let mut event = libc::kevent {
                ident: 0,
                filter: 0,
                flags: 0,
                fflags: 0,
                data: 0,
                udata: ptr::null_mut(),
            };
            // SAFETY: `event` is one writable event slot and the zero timeout is nonblocking.
            let count = unsafe {
                libc::kevent(
                    std::os::fd::AsRawFd::as_raw_fd(&self.events),
                    ptr::null(),
                    0,
                    &mut event,
                    1,
                    &timeout,
                )
            };
            if count < 0 {
                return None;
            }
            if count == 0 {
                return Some(());
            }
            let grew = event.fflags & (libc::NOTE_FORK | libc::NOTE_EXEC) != 0;
            self.saw_tree_change |= grew;
            self.growth_pending |= grew;
            self.saw_exit |= event.fflags & libc::NOTE_EXIT != 0;
        }
    }
}

#[cfg(target_os = "macos")]
impl ForkWitness for DarwinProcessAuthority {
    fn observed_state(&mut self) -> Option<(bool, bool)> {
        self.poll_events()?;
        Some((self.saw_exit, self.saw_tree_change))
    }

    fn take_growth(&mut self) -> Option<bool> {
        self.poll_events()?;
        Some(std::mem::take(&mut self.growth_pending))
    }
}

#[cfg(target_os = "macos")]
impl InstanceAuthority for DarwinProcessAuthority {
    /// The kqueue filter is attached to the exact process instance, not its reusable PID.
    /// Once NOTE_EXIT arrives this authority stays dead even after that number is recycled.
    fn is_alive(&mut self) -> Option<bool> {
        self.poll_events()?;
        Some(!self.saw_exit)
    }

    /// Signal the exact process instance. libproc returns zero or errno directly.
    fn signal(&mut self, sig: i32) -> Option<SignalOutcome> {
        match self.raw_signal(sig) {
            0 => Some(SignalOutcome::Delivered),
            libc::ESRCH => Some(SignalOutcome::Stale),
            _ => None,
        }
    }
}

/// `proc_signal_with_audittoken` is declared in the installed SDK's `libproc.h`, within the
/// libproc API family Apple documents as private and subject to change, and it has no SDK
/// availability annotation. Resolve it at runtime so a system without the symbol degrades
/// to the unverified path instead of making the daemon fail to load.
#[cfg(target_os = "macos")]
fn darwin_audit_signal() -> Option<DarwinAuditSignalFn> {
    // SAFETY: `RTLD_DEFAULT` searches already-loaded libSystem/libproc images. The symbol's
    // declaration in the SDK is exactly `int (audit_token_t *, int)`; a missing symbol is a
    // null pointer and causes a safe capability fallback.
    let symbol =
        unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"proc_signal_with_audittoken".as_ptr()) };
    if symbol.is_null() {
        None
    } else {
        // SAFETY: the non-null address was resolved by its exact C symbol name and signature.
        Some(unsafe { std::mem::transmute::<*mut libc::c_void, DarwinAuditSignalFn>(symbol) })
    }
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct DarwinAuditToken {
    val: [u32; 8],
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn mach_task_self() -> libc::mach_port_t {
    // SAFETY: this is the process-global send right exported by libSystem.
    unsafe { libc::mach_task_self_ }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn task_name_for_pid(
        target_tport: libc::mach_port_t,
        pid: libc::pid_t,
        task: *mut libc::mach_port_t,
    ) -> libc::kern_return_t;
    fn mach_port_deallocate(
        task: libc::mach_port_t,
        name: libc::mach_port_t,
    ) -> libc::kern_return_t;
}

#[cfg(test)]
mod tests {
    use super::super::os::{ForkWitness, InstanceAuthority};

    fn handle(argv: &[&str]) -> goalcup_protocol::process::ProcessHandle {
        goalcup_protocol::process::ProcessHandle {
            pid: 4_242,
            pgid: None,
            native_pid: None,
            native_binary: None,
            native_process_start: None,
            process_start: None,
            strategy: goalcup_protocol::manifest::KillStrategy::NativePid,
            grace_seconds: 1,
            launched_at: "2026-09-04T00:00:00Z".to_string(),
            program: "agent".to_string(),
            argv: argv.iter().map(|a| a.to_string()).collect(),
            env_keys: Vec::new(),
        }
    }

    fn wait_for_darwin_event(
        target: &mut super::DarwinProcessAuthority,
        predicate: impl Fn((bool, bool)) -> bool,
    ) -> (bool, bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let state = target.observed_state().expect("read the kqueue witness");
            if predicate(state) {
                return state;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for the Darwin process event; last state {state:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// The private libproc symbol is runtime-optional: absence must stop acquisition before
    /// any process authority is claimed, leaving the caller to report the fallback.
    #[test]
    fn an_unavailable_darwin_audit_signal_capability_is_not_acquired() {
        assert!(
            super::DarwinProcessAuthority::acquire_with_signal(std::process::id(), None).is_none()
        );
    }

    /// EVFILT_PROC is the exact non-signalling liveness witness. A fast exit is recorded on
    /// the watched process object, independent of whether its numeric PID can later recycle.
    #[test]
    fn a_darwin_process_exit_is_witnessed_without_signal_zero() {
        use super::DarwinProcessAuthority;
        use std::process::{Command, Stdio};

        let mut child = Command::new("/bin/cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn a gated process");
        let mut target = DarwinProcessAuthority::acquire(child.id())
            .expect("same-user child exposes an audit token");
        drop(child.stdin.take().expect("release the child's stdin gate"));

        assert_eq!(
            wait_for_darwin_event(&mut target, |(exited, _)| exited),
            (true, false)
        );
        child.wait().expect("reap the exited child");
    }

    /// `exec` changes the PID version and stales the audit token without changing the PID.
    /// The watcher must record that transition so ESRCH is never promoted to "gone".
    #[test]
    fn a_darwin_exec_is_unverified_and_never_retried_by_number() {
        use super::super::os::SignalOutcome;
        use super::DarwinProcessAuthority;
        use std::io::Write;
        use std::process::{Command, Stdio};

        let mut child = Command::new("/bin/sh")
            .args(["-c", "read first; exec /bin/sleep 300"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn an exec-gated process");
        let mut target = DarwinProcessAuthority::acquire(child.id())
            .expect("same-user child exposes an audit token");
        child
            .stdin
            .as_mut()
            .expect("child stdin")
            .write_all(b"go\n")
            .expect("release the exec gate");

        assert_eq!(
            wait_for_darwin_event(&mut target, |(_, changed)| changed),
            (false, true)
        );
        assert_eq!(target.signal(libc::SIGTERM), Some(SignalOutcome::Stale));
        assert!(
            child.try_wait().expect("query exec'd child").is_none(),
            "a stale token must not reach the live PID"
        );

        child.kill().expect("clean up the exec'd child");
        child.wait().expect("reap the exec'd child");
    }

    /// A fork after the final singleton snapshot is still recorded even though a seal-time
    /// watcher is never sufficient to prove the tree's earlier history.
    #[test]
    fn a_darwin_fork_is_recorded_by_the_exact_process_witness() {
        use super::DarwinProcessAuthority;
        use std::io::Write;
        use std::process::{Command, Stdio};

        let mut child = Command::new("/bin/sh")
            .args(["-c", "read first; (:) & wait; read second"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn a fork-gated process");
        let mut target = DarwinProcessAuthority::acquire(child.id())
            .expect("same-user child exposes an audit token");
        child
            .stdin
            .as_mut()
            .expect("child stdin")
            .write_all(b"go\n")
            .expect("release the fork gate");

        assert_eq!(
            wait_for_darwin_event(&mut target, |(_, changed)| changed),
            (false, true)
        );

        child.kill().expect("clean up the watched shell");
        child.wait().expect("reap the watched shell");
    }

    /// libproc returns errno directly. A token with the wrong PID version must be reported
    /// as stale without delivering a numeric signal to the still-live process at that PID.
    #[test]
    fn a_stale_darwin_audit_token_does_not_signal_the_live_numeric_pid() {
        use super::super::os::LiveOs;
        use super::{DarwinProcessAuthority, terminate_darwin_singleton_with_authority};
        use std::process::{Command, Stdio};

        let mut child = Command::new("/bin/sleep")
            .arg("300")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn a live process for the audit-token check");
        let mut target = DarwinProcessAuthority::acquire(child.id())
            .expect("same-user child exposes an audit token");
        target.token.val[7] = target.token.val[7].wrapping_add(1);
        let mut registered = handle(&["/bin/sleep", "300"]);
        registered.pid = child.id();
        registered.program = "/bin/sleep".to_string();
        registered.process_start = goalcup_protocol::process::process_start_identity(child.id());
        registered.grace_seconds = 0;

        let report = terminate_darwin_singleton_with_authority(
            &LiveOs,
            &registered,
            std::time::Instant::now(),
            Box::new(target),
        );
        assert!(
            report.targeted.is_empty(),
            "a stale token delivered no signal"
        );
        assert!(report.harness_unverified && !report.all_gone, "{report:?}");
        assert_eq!(report.still_alive, vec![child.id()]);
        assert!(
            child.try_wait().expect("query child").is_none(),
            "the stale token fell through to a numeric signal against the live PID"
        );

        child.kill().expect("clean up the untouched child");
        child.wait().expect("reap the untouched child");
    }
}
