//! The terminal a chat-mode harness shares with the command that launched it (D57).
//!
//! `goalcup start` runs the harness in the participant's own terminal, in its own process
//! group, so the harness is in the terminal's foreground group (a TUI outside it is stopped by
//! `SIGTTIN` at its first read). Everything the terminal sends that group therefore reaches the
//! host too: a Ctrl-C (`SIGINT`), a Ctrl-\ (`SIGQUIT`), a Ctrl-Z (`SIGTSTP`), and a closed
//! window (`SIGHUP`). The harness must get each of them; the host must outlive every one, or the
//! run is never sealed.
//!
//! [`Shield`] is how: while it is held, those four signals are caught by a handler that does
//! nothing but note a hangup. **Caught, not ignored, on purpose.** A signal set to `SIG_IGN` stays
//! ignored across `exec`, so a harness started under an ignoring host would be born deaf to
//! Ctrl-C and to the closed window; a caught signal is reset to its default in the new image.
//! A `Shield` taken after the harness was forked changes nothing for it either way.
//!
//! [`hangup_seen`] answers whether the terminal said it was going away, which is how the host
//! knows the terminal is no longer there to print on.

use std::sync::atomic::{AtomicBool, Ordering};

/// The signals a terminal sends its foreground group, which a [`Shield`] catches.
pub const SHIELDED: [libc::c_int; 4] = [libc::SIGINT, libc::SIGQUIT, libc::SIGTSTP, libc::SIGHUP];

/// Set by the handler on `SIGHUP`, and never cleared: a terminal that hung up is gone.
static HANGUP: AtomicBool = AtomicBool::new(false);

/// The handler: one atomic store, which is all a signal handler may do.
extern "C" fn absorb(signum: libc::c_int) {
    if signum == libc::SIGHUP {
        HANGUP.store(true, Ordering::SeqCst);
    }
}

/// Whether a `SIGHUP` reached this process while a [`Shield`] held it.
pub fn hangup_seen() -> bool {
    HANGUP.load(Ordering::SeqCst)
}

/// The four terminal signals caught and absorbed, each restored to what it was before when it
/// is released ([`Shield::release`]) or when the shield drops.
#[derive(Debug)]
pub struct Shield {
    saved: Vec<(libc::c_int, libc::sigaction)>,
}

impl Shield {
    /// Catch [`SHIELDED`]. `SA_RESTART`, so a host thread blocked in a read or a write is
    /// resumed rather than handed an `EINTR` it was not written to expect.
    pub fn take() -> std::io::Result<Shield> {
        let mut shield = Shield { saved: Vec::new() };
        for signal in SHIELDED {
            // SAFETY: both structs are zeroed locals, so every field this platform has is a
            // defined value; `sigaction` reads the first and writes the second. The handler
            // stores into a static atomic and nothing else.
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = absorb as *const () as libc::sighandler_t;
                libc::sigemptyset(&mut action.sa_mask);
                action.sa_flags = libc::SA_RESTART;
                let mut old: libc::sigaction = std::mem::zeroed();
                if libc::sigaction(signal, &action, &mut old) != 0 {
                    // The ones already taken are put back when `shield` drops.
                    return Err(std::io::Error::last_os_error());
                }
                shield.saved.push((signal, old));
            }
        }
        Ok(shield)
    }

    /// Put back what `signals` were before; the rest stay caught until the shield drops.
    pub fn release(&mut self, signals: &[libc::c_int]) {
        let (back, kept): (Vec<_>, Vec<_>) = self
            .saved
            .drain(..)
            .partition(|(signal, _)| signals.contains(signal));
        self.saved = kept;
        restore(&back);
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

/// Whether `fd` is a terminal this process is in the foreground group of: the condition under
/// which a harness that inherits it can read from it.
pub fn is_foreground(fd: libc::c_int) -> bool {
    // SAFETY: `isatty`, `tcgetpgrp` and `getpgrp` take integers and touch no memory of ours.
    unsafe { libc::isatty(fd) == 1 && libc::tcgetpgrp(fd) == libc::getpgrp() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shielded process outlives a hangup and notes it; once released, the signal's old
    /// disposition is back.
    #[test]
    fn a_shield_absorbs_the_terminal_signals_and_gives_them_back() {
        let shield = Shield::take().unwrap();
        // SAFETY: signals to this process, whose handlers were just installed.
        unsafe {
            libc::raise(libc::SIGHUP);
            libc::raise(libc::SIGINT);
            libc::raise(libc::SIGTSTP);
            libc::raise(libc::SIGQUIT);
        }
        assert!(hangup_seen());
        let mut shield = shield;
        shield.release(&[libc::SIGINT, libc::SIGQUIT, libc::SIGTSTP]);
        let current = |signal| {
            // SAFETY: a zeroed local the call writes.
            unsafe {
                let mut now: libc::sigaction = std::mem::zeroed();
                libc::sigaction(signal, std::ptr::null(), &mut now);
                now.sa_sigaction
            }
        };
        assert_eq!(current(libc::SIGINT), libc::SIG_DFL);
        assert_eq!(
            current(libc::SIGHUP),
            absorb as *const () as libc::sighandler_t
        );
        drop(shield);
        assert_eq!(current(libc::SIGHUP), libc::SIG_DFL);
    }
}
