//! The seam between the termination algorithm and the operating system.
//!
//! Everything [`super::terminate_with`] learns about processes, and everything it does to
//! them, goes through one [`ProcessOs`]. The shipped implementation is [`LiveOs`], which
//! is nothing but a thin dispatch onto the primitives — the `/bin/ps` readers in
//! [`goalcup_protocol::process`], the signal helpers here, and libc. The other
//! implementation is a simulated process table in `tests/terminate_sim.rs`, which is the
//! only way to provoke a `/bin/ps` that cannot be spawned (it is called by absolute path,
//! so no shell fixture can) and the only way to hold the ground truth of what a tree
//! forked, whether or not any snapshot ever showed it.
//!
//! **What the seam deliberately does not do.** It does not decide. `liveness` returns the
//! `Err` a failed spawn produces, and [`alive`] folds it to `false` exactly as
//! [`goalcup_protocol::process::is_alive`] does; the fold is in one place. Signal guards —
//! the refusal to name pid 0 or 1 — live in [`LiveOs`] and not in the algorithm, so a
//! simulated OS sees every request the algorithm makes, including the ones the live OS
//! would have swallowed.

use super::MIN_SIGNALLABLE_PID;
use super::authority::live_identity;
use goalcup_protocol::process::{
    ProcInfo, ProcessError, liveness, process_start_identity, snapshot,
};
use std::time::{Duration, Instant};
#[cfg(target_os = "linux")]
use std::{
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    ptr,
};

/// The operating system, as the termination algorithm sees it.
///
/// Every method is one question or one act, named for the primitive it stands in for.
/// `&self` throughout: the live implementation holds no state, and a simulation keeps its
/// table behind a lock so that one `&dyn ProcessOs` can be handed down a call tree.
pub trait ProcessOs {
    /// One `ps` over the whole table: [`goalcup_protocol::process::snapshot`].
    fn snapshot(&self) -> Result<Vec<ProcInfo>, ProcessError>;

    /// Is `pid` live and not a zombie — or could the table not be asked? `Err` means the
    /// question was never put; see [`goalcup_protocol::process::liveness`].
    fn liveness(&self, pid: u32) -> Result<bool, ProcessError>;

    /// `(elapsed seconds, args)` for a live pid, `None` when it is not in the table:
    /// [`live_identity`].
    fn identity(&self, pid: u32) -> Option<(u64, String)>;

    /// The OS process-instance identity a recycled number cannot reproduce:
    /// [`process_start_identity`].
    fn start_identity(&self, pid: u32) -> Option<String>;

    /// Does `pid` lead its own process group — `getpgid(pid) == pid`?
    fn leads_group(&self, pid: u32) -> bool;

    /// An instance-bound handle on `pid`, taken before the authority check that authorises
    /// a signal through it, so a number recycled between the two reaches the old instance
    /// (`ESRCH`) and never the new one. Linux: a pidfd. Elsewhere, or when the kernel or
    /// sandbox refuses one: `None`, and the caller takes the numeric path.
    fn bind(&self, pid: u32) -> Option<Box<dyn BoundTarget>>;

    /// `kill(2)` by number. The result is dropped: `ESRCH` is the outcome
    /// the caller wants and `EPERM` is what the sweep will report as still alive.
    fn kill(&self, pid: u32, sig: i32);

    /// `killpg(2)` by number, for a group the registered root was proven to lead.
    fn killpg(&self, pgid: u32, sig: i32);

    /// A witness for forks, execs and the exit of the exact instance at `pid`, from this
    /// moment on. macOS: an `EVFILT_PROC` kqueue. Linux: `None`.
    fn fork_witness(&self, pid: u32) -> Option<Box<dyn ForkWitness>>;

    /// The instance-bound signal authority the Darwin singleton path runs on: an audit
    /// token plus the witness above, acquired together. `None` off macOS, or when either
    /// half cannot be acquired, and the generic path runs instead.
    fn instance_authority(&self, pid: u32) -> Option<Box<dyn InstanceAuthority>>;

    /// The clock every deadline in the escalation is measured against.
    fn now(&self) -> Instant;

    /// The gap between polls.
    fn sleep(&self, d: Duration);
}

/// A signal handle bound to one process instance rather than to a number.
pub trait BoundTarget {
    /// Deliver `sig` to the bound instance. `true` when it was delivered or the instance
    /// had already exited (`ESRCH`, which a bound handle cannot misroute); `false` on any
    /// other failure, which the caller reports as unverified.
    fn signal(&mut self, sig: i32) -> bool;
}

/// What a kernel witness saw of one process instance since it was attached.
pub trait ForkWitness {
    /// `(exited, forked_or_execed)`, or `None` when the witness itself failed and nothing
    /// may be claimed from it.
    fn observed_state(&mut self) -> Option<(bool, bool)>;

    /// Did the instance fork or exec since this was last asked — or since the witness was
    /// attached? Asking clears it: the answer is one generation of forks, so an adoption
    /// can account for exactly the forks it saw and never a later one. `None` when the
    /// witness failed. The kernel hands over one bit per read (`NOTE_FORK` is a flag, not
    /// a count), so two forks between two reads are one answer.
    fn take_growth(&mut self) -> Option<bool>;
}

/// Signal authority over one exact process instance, with its own witness.
pub trait InstanceAuthority: ForkWitness {
    /// Has the exact instance not yet exited? `None` when the witness failed.
    fn is_alive(&mut self) -> Option<bool>;

    /// Signal the exact instance. `None` for a delivery failure that is neither success
    /// nor staleness.
    fn signal(&mut self, sig: i32) -> Option<SignalOutcome>;
}

/// The two answers an instance-bound signal can give short of failing outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalOutcome {
    /// The kernel matched the instance and delivered.
    Delivered,
    /// `ESRCH`: the instance is gone, or (on Darwin) an exec changed its PID version. The
    /// witness is what tells those apart; neither permits a numeric retry.
    Stale,
}

/// [`ProcessOs::liveness`] folded to the shipped decision: "could not ask" is "gone".
///
/// One function, so the fold lives in one place. This is
/// [`goalcup_protocol::process::is_alive`], and every ruling the algorithm makes on a
/// zombie or an absent row goes through here.
pub(super) fn alive(os: &dyn ProcessOs, pid: u32) -> bool {
    os.liveness(pid).unwrap_or(false)
}

/// Wall-clock milliseconds since `started`, on the seam's clock, for a report.
pub(super) fn waited_ms(os: &dyn ProcessOs, started: Instant) -> u64 {
    u64::try_from(os.now().saturating_duration_since(started).as_millis()).unwrap_or(u64::MAX)
}

/// The operating system this daemon actually runs on.
///
/// Every method is a call to a primitive. None of the `/bin/ps` argument strings are
/// spelled here; each lives in the one place that owns it.
pub struct LiveOs;

impl ProcessOs for LiveOs {
    fn snapshot(&self) -> Result<Vec<ProcInfo>, ProcessError> {
        snapshot()
    }

    fn liveness(&self, pid: u32) -> Result<bool, ProcessError> {
        liveness(pid)
    }

    fn identity(&self, pid: u32) -> Option<(u64, String)> {
        live_identity(pid)
    }

    fn start_identity(&self, pid: u32) -> Option<String> {
        process_start_identity(pid)
    }

    fn leads_group(&self, pid: u32) -> bool {
        let Ok(pid) = i32::try_from(pid) else {
            return false;
        };
        // SAFETY: `getpgid` takes one integer and writes no memory.  Equality is the exact
        // kernel statement that this pid is the leader of its own process group.
        unsafe { libc::getpgid(pid) == pid }
    }

    fn bind(&self, pid: u32) -> Option<Box<dyn BoundTarget>> {
        #[cfg(target_os = "linux")]
        {
            pidfd_for(pid).map(|fd| Box::new(PidFd(fd)) as Box<dyn BoundTarget>)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = pid;
            None
        }
    }

    fn kill(&self, pid: u32, sig: i32) {
        signal(pid, sig);
    }

    fn killpg(&self, pgid: u32, sig: i32) {
        signal_group(pgid, sig);
    }

    fn fork_witness(&self, pid: u32) -> Option<Box<dyn ForkWitness>> {
        #[cfg(target_os = "macos")]
        {
            super::darwin::DarwinProcessAuthority::acquire(pid)
                .map(|authority| Box::new(authority) as Box<dyn ForkWitness>)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = pid;
            None
        }
    }

    fn instance_authority(&self, pid: u32) -> Option<Box<dyn InstanceAuthority>> {
        #[cfg(target_os = "macos")]
        {
            super::darwin::DarwinProcessAuthority::acquire(pid)
                .map(|authority| Box::new(authority) as Box<dyn InstanceAuthority>)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = pid;
            None
        }
    }

    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// Send `sig` to one pid, or do nothing when the pid is one no harness can be.
///
/// The result is deliberately dropped: `ESRCH` means the process is already gone, which is
/// the outcome this module wants, and `EPERM` means it will still be alive in the sweep —
/// which is what `all_gone` reports. Neither is worth failing the seal over.
fn signal(pid: u32, sig: i32) {
    if pid < MIN_SIGNALLABLE_PID {
        return;
    }
    let Ok(pid) = i32::try_from(pid) else {
        return;
    };
    // SAFETY: `kill` takes two integers, dereferences nothing, and touches no memory this
    // process owns. `pid` is positive — the negative and zero forms, which address whole
    // process groups including the caller's own, are excluded above — so this addresses
    // exactly one process.
    unsafe {
        libc::kill(pid, sig);
    }
}

/// Send `sig` to every member of process group `pgid`. Called only for a group the
/// registered pid leads.
fn signal_group(pgid: u32, sig: i32) {
    if pgid < MIN_SIGNALLABLE_PID {
        return;
    }
    let Ok(pgid) = i32::try_from(pgid) else {
        return;
    };
    // SAFETY: as `signal`. `killpg` takes the group id positively and is the spelling that
    // cannot be confused with `kill(-pid)`, where a sign error addresses the caller's own
    // process group.
    unsafe {
        libc::killpg(pgid, sig);
    }
}

/// Open a Linux pidfd for one candidate target. On failure the caller uses its immediately
/// revalidated numeric fallback, and permanently marks that termination unverified because
/// the fallback cannot offer the direct-signal guarantee a pidfd would provide.
#[cfg(target_os = "linux")]
fn pidfd_for(pid: u32) -> Option<OwnedFd> {
    if pid < MIN_SIGNALLABLE_PID {
        return None;
    }
    let pid = i32::try_from(pid).ok()?;
    // SAFETY: the syscall takes integer arguments only and returns a newly owned fd on
    // success. `pid` is positive, so it cannot name a process group.
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    (raw >= 0).then(|| unsafe { OwnedFd::from_raw_fd(raw as i32) })
}

#[cfg(target_os = "linux")]
struct PidFd(OwnedFd);

#[cfg(target_os = "linux")]
impl BoundTarget for PidFd {
    /// Signal the exact kernel process object held by a pidfd. `ESRCH` means that object
    /// already exited; unlike a numeric kill it cannot be redirected to a recycled PID, so
    /// it is a safe successful termination outcome.
    fn signal(&mut self, sig: i32) -> bool {
        // SAFETY: this passes a valid owned fd and a null optional `siginfo_t` pointer as
        // the pidfd_send_signal ABI specifies; the kernel neither retains nor writes either.
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.0.as_raw_fd(),
                sig,
                ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }
}
