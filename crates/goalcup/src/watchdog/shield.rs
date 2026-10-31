//! The terminal the harness shares with the watchdog (PRD-02 section 8 step 7, PRD-17 P3 to P5),
//! copied from the old `terminal.rs:27-292`.
//!
//! The harness runs in the watchdog's own process group, so every signal the terminal sends that
//! group (Ctrl-C, Ctrl-\, Ctrl-Z, a closed window) reaches the watchdog too. While a [`Shield`] is
//! held those four are caught by a handler that only notes a hangup. Caught, never ignored: a
//! `SIG_IGN` disposition survives `exec`, a caught one is reset to the default. Only `SIGTTOU` is
//! ignored, and only after the harness was forked.

use std::sync::atomic::{AtomicBool, Ordering};

/// The signals a terminal sends its foreground group, which a [`Shield`] catches.
pub const CAUGHT: [libc::c_int; 4] = [libc::SIGINT, libc::SIGQUIT, libc::SIGTSTP, libc::SIGHUP];

/// What [`Shield::harness_exited`] gives back: the keys the participant types.
pub const GIVEN_BACK: [libc::c_int; 3] = [libc::SIGINT, libc::SIGQUIT, libc::SIGTSTP];

/// Set by the handler on `SIGHUP`, and never cleared: a terminal that hung up is gone.
static HANGUP: AtomicBool = AtomicBool::new(false);

/// The handler: one atomic store, which is all a signal handler may do.
extern "C" fn absorb(signum: libc::c_int) {
    if signum == libc::SIGHUP {
        HANGUP.store(true, Ordering::SeqCst);
    }
}

/// Whether a `SIGHUP` reached this process while a [`Shield`] held it: the terminal is no longer
/// there to print on.
pub fn hangup_seen() -> bool {
    HANGUP.load(Ordering::SeqCst)
}

/// The terminal's signals caught (and `SIGTTOU` ignored), each restored to what it was before
/// when it is given back ([`Shield::harness_exited`]) or when the shield drops.
#[derive(Debug)]
pub struct Shield {
    saved: Vec<(libc::c_int, libc::sigaction)>,
}

impl Shield {
    /// Catch [`CAUGHT`] and ignore `SIGTTOU`. Take it after the harness was forked. `SA_RESTART`,
    /// so a thread blocked in a read or a write is resumed rather than handed an `EINTR` it was
    /// not written to expect.
    pub fn take() -> std::io::Result<Shield> {
        let mut shield = Shield { saved: Vec::new() };
        let handler = absorb as *const () as libc::sighandler_t;
        for (signal, action) in CAUGHT
            .iter()
            .map(|s| (*s, handler))
            .chain([(libc::SIGTTOU, libc::SIG_IGN)])
        {
            // SAFETY: both structs are zeroed locals, so every field this platform has is a
            // defined value; `sigaction` reads the first and writes the second. The handler
            // stores into a static atomic and nothing else.
            unsafe {
                let mut new: libc::sigaction = std::mem::zeroed();
                new.sa_sigaction = action;
                libc::sigemptyset(&mut new.sa_mask);
                new.sa_flags = libc::SA_RESTART;
                let mut old: libc::sigaction = std::mem::zeroed();
                if libc::sigaction(signal, &new, &mut old) != 0 {
                    // The ones already taken are put back when `shield` drops.
                    return Err(std::io::Error::last_os_error());
                }
                shield.saved.push((signal, old));
            }
        }
        Ok(shield)
    }

    /// The harness has exited: [`GIVEN_BACK`] are what they were before. `SIGHUP` and `SIGTTOU`
    /// stay held until the shield drops, so a window closed after the harness exited does not cut
    /// the seal or the upload short.
    ///
    /// A key typed while the harness ran went to the whole group, so the same signal can still be
    /// pending here, not yet taken by any thread. Given back first, it would reach the default
    /// action and end the wrap-up. So this thread blocks them, takes every pending one while the
    /// handler is still in place, and only then gives them back and unblocks. A key typed after
    /// the last drain found nothing pending, even one that arrives inside this call before the
    /// keys are given back, counts as typed after the harness exited: it gets the default action,
    /// as it would without a watchdog.
    ///
    /// Returns `false` when this thread could not block them, so they were given back without
    /// the drain; the caller logs it.
    pub fn harness_exited(&mut self) -> bool {
        let (back, kept): (Vec<_>, Vec<_>) = self
            .saved
            .drain(..)
            .partition(|(signal, _)| GIVEN_BACK.contains(signal));
        self.saved = kept;
        // SAFETY: zeroed locals filled by `sigemptyset`, `sigaddset` and `pthread_sigmask`.
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            for signal in GIVEN_BACK {
                libc::sigaddset(&mut set, signal);
            }
            let mut old: libc::sigset_t = std::mem::zeroed();
            let masked = libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut old) == 0;
            if masked {
                drain_pending(&set);
            }
            restore(&back);
            if masked {
                libc::pthread_sigmask(libc::SIG_SETMASK, &old, std::ptr::null_mut());
            }
            masked
        }
    }
}

/// Take every signal of `set` that is pending, while this thread blocks them all and the handler
/// is still installed. A thread that takes one first does so under the kernel's signal lock,
/// with the handler still in place.
#[cfg(target_os = "linux")]
fn drain_pending(set: &libc::sigset_t) {
    let zero = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    loop {
        // SAFETY: a zero timeout only takes a pending signal or returns at once.
        if unsafe { libc::sigtimedwait(set, std::ptr::null_mut(), &zero) } > 0 {
            continue;
        }
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return;
        }
    }
}

/// Elsewhere (macOS has no `sigtimedwait`): `sigwait` on one signal, and only one that
/// `sigpending` shows pending, so it never waits.
#[cfg(not(target_os = "linux"))]
fn drain_pending(set: &libc::sigset_t) {
    loop {
        // SAFETY: zeroed locals filled by `sigpending`, `sigemptyset` and `sigaddset`; `sigwait`
        // is called only for a signal that is pending and blocked here.
        unsafe {
            let mut pending: libc::sigset_t = std::mem::zeroed();
            if libc::sigpending(&mut pending) != 0 {
                return;
            }
            let Some(signal) = GIVEN_BACK
                .into_iter()
                .find(|s| libc::sigismember(set, *s) == 1 && libc::sigismember(&pending, *s) == 1)
            else {
                return;
            };
            let mut one: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut one);
            libc::sigaddset(&mut one, signal);
            let mut taken = 0;
            if libc::sigwait(&one, &mut taken) != 0 {
                // The signal stays pending; picking it again would loop for ever.
                return;
            }
        }
    }
}

impl Drop for Shield {
    fn drop(&mut self) {
        restore(&self.saved);
    }
}

fn restore(saved: &[(libc::c_int, libc::sigaction)]) {
    for (signal, old) in saved {
        // SAFETY: `old` is the action `sigaction` itself wrote for this signal.
        unsafe {
            libc::sigaction(*signal, old, std::ptr::null_mut());
        }
    }
}

/// A terminal's modes, as they were when [`Modes::save`] read them.
pub struct Modes {
    fd: libc::c_int,
    saved: libc::termios,
}

impl Modes {
    /// The modes of the terminal on `fd`, or `None` when `fd` is not one.
    pub fn save(fd: libc::c_int) -> Option<Modes> {
        // SAFETY: a zeroed local `tcgetattr` writes on success.
        unsafe {
            let mut saved: libc::termios = std::mem::zeroed();
            (libc::tcgetattr(fd, &mut saved) == 0).then_some(Modes { fd, saved })
        }
    }

    /// Put them back, once the output already written has been sent. A harness killed in raw
    /// mode would otherwise leave the participant's shell typing blind.
    pub fn restore(&self) -> std::io::Result<()> {
        // SAFETY: the struct `tcgetattr` filled, on the descriptor it was read from.
        if unsafe { libc::tcsetattr(self.fd, libc::TCSADRAIN, &self.saved) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

/// Wait for the child `pid` to exit, and continue it each time it stops (`WUNTRACED`).
///
/// A harness that suspends itself (Ctrl-Z in its own key handling, `kill(0, SIGTSTP)`) would
/// otherwise stop while the watchdog kept waiting for an exit, and the participant's shell, whose
/// foreground job is the watchdog, would never get the terminal back. `SIGCONT` goes to the
/// stopped child's process group, which is the watchdog's own: a Ctrl-Z stops the harness's
/// children too, and `SIGCONT` is harmless to the watchdog. It is the only signal ever sent to
/// that group. The watchdog never stops itself.
///
/// The child is reaped here, so the caller must not wait for it again.
pub fn wait_resuming(pid: u32) -> std::io::Result<std::process::ExitStatus> {
    use std::os::unix::process::ExitStatusExt;
    let pid = libc::pid_t::try_from(pid).map_err(|_| std::io::Error::other("pid out of range"))?;
    loop {
        let mut status = 0;
        // SAFETY: `waitpid` writes the one integer it is handed.
        let reaped = unsafe { libc::waitpid(pid, &mut status, libc::WUNTRACED) };
        if reaped == -1 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if libc::WIFSTOPPED(status) {
            // SAFETY: integers only.
            unsafe {
                let group = libc::getpgid(pid);
                if group > 0 {
                    libc::kill(-group, libc::SIGCONT);
                } else {
                    libc::kill(pid, libc::SIGCONT);
                }
            }
            continue;
        }
        return Ok(std::process::ExitStatus::from_raw(status));
    }
}
