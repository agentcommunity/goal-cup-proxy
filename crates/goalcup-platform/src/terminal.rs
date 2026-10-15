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
//! The one signal set to `SIG_IGN` is `SIGTTOU`, and only because the shield is taken after the
//! harness was forked (its dispositions are its own by then): a caught `SIGTTOU` makes a
//! background `tcsetattr` restart forever, an ignored one lets it proceed.
//!
//! [`Shield::harness_exited`] gives `SIGINT`, `SIGQUIT` and `SIGTSTP` back when the harness is
//! gone; `SIGHUP` and `SIGTTOU` stay held until the shield drops, at the end of the run's wrap-up,
//! so a window closed after the harness exited does not cut the seal or the upload short.
//!
//! [`hangup_seen`] answers whether the terminal said it was going away, which is how the host
//! knows the terminal is no longer there to print on. [`wait_resuming`] waits for the harness and
//! continues it whenever it stops, and [`Modes`] puts the terminal's modes back after a harness
//! that left it raw.

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

/// Whether a `SIGHUP` reached this process while a [`Shield`] held it.
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
    /// so a host thread blocked in a read or a write is resumed rather than handed an `EINTR` it
    /// was not written to expect.
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

    /// The harness has exited: [`GIVEN_BACK`] are what they were before. `SIGHUP` and
    /// `SIGTTOU` stay held until the shield drops.
    pub fn harness_exited(&mut self) {
        let (back, kept): (Vec<_>, Vec<_>) = self
            .saved
            .drain(..)
            .partition(|(signal, _)| GIVEN_BACK.contains(signal));
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

/// Whether `fd` is a terminal.
pub fn is_terminal(fd: libc::c_int) -> bool {
    // SAFETY: `isatty` takes an integer and touches no memory of ours.
    unsafe { libc::isatty(fd) == 1 }
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

/// The signals that end a command asking for a secret, and that [`EchoOff`] answers by putting
/// the terminal back before it lets the signal do what it would have done.
const ENDING: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

/// The modes [`EchoOff`] must restore, and where, for the signal handler to read: a pointer to
/// the saved `termios` (null when no [`EchoOff`] is held) and its descriptor.
static ECHO_SAVED: std::sync::atomic::AtomicPtr<libc::termios> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
static ECHO_FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Put the terminal back, then let the signal do what it would have done: the handler was
/// installed with `SA_RESETHAND | SA_NODEFER`, so the disposition is already the default and
/// the signal raised again is delivered at once. Only async-signal-safe calls.
extern "C" fn restore_then_reraise(signum: libc::c_int) {
    let saved = ECHO_SAVED.load(Ordering::SeqCst);
    if !saved.is_null() {
        // SAFETY: `saved` points at the `Box` an `EchoOff` holds for as long as it is
        // published here; `tcsetattr` only reads it.
        unsafe {
            libc::tcsetattr(ECHO_FD.load(Ordering::SeqCst), libc::TCSANOW, saved);
        }
    }
    // SAFETY: `raise` takes an integer.
    unsafe {
        libc::raise(signum);
    }
}

/// A terminal with its echo switched off, for reading a secret the participant pastes or
/// types: what they enter is not shown. The echo is switched back on when this drops, and
/// also when the process is ended by `SIGINT`, `SIGTERM` or `SIGHUP` meanwhile, so a parent
/// that is not a shell is not left a terminal that shows nothing.
pub struct EchoOff {
    modes: Modes,
    /// The copy of the saved modes the signal handler reads.
    _published: Box<libc::termios>,
    /// What each of [`ENDING`] was before.
    earlier: Vec<(libc::c_int, libc::sigaction)>,
}

impl EchoOff {
    /// Switch the echo of the terminal on `fd` off; `None` when `fd` is not a terminal (a pipe
    /// shows nothing, so there is nothing to hide).
    pub fn take(fd: libc::c_int) -> Option<EchoOff> {
        let modes = Modes::save(fd)?;
        let mut quiet = modes.saved;
        quiet.c_lflag &= !libc::ECHO;
        let mut published = Box::new(modes.saved);
        // SAFETY: the struct `tcgetattr` filled, with one flag cleared, on the descriptor it was
        // read from.
        if unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &quiet) } != 0 {
            return None;
        }
        ECHO_FD.store(fd, Ordering::SeqCst);
        ECHO_SAVED.store(&mut *published, Ordering::SeqCst);
        let mut earlier = Vec::new();
        for signal in ENDING {
            // SAFETY: zeroed locals `sigaction` reads and writes; the handler is async-signal-
            // safe. A signal the parent ignored (a command run under `nohup`) stays ignored.
            unsafe {
                let mut old: libc::sigaction = std::mem::zeroed();
                libc::sigaction(signal, std::ptr::null(), &mut old);
                if old.sa_sigaction == libc::SIG_IGN {
                    continue;
                }
                let mut new: libc::sigaction = std::mem::zeroed();
                new.sa_sigaction = restore_then_reraise as *const () as libc::sighandler_t;
                libc::sigemptyset(&mut new.sa_mask);
                new.sa_flags = libc::SA_RESETHAND | libc::SA_NODEFER;
                if libc::sigaction(signal, &new, &mut old) == 0 {
                    earlier.push((signal, old));
                }
            }
        }
        Some(EchoOff {
            modes,
            _published: published,
            earlier,
        })
    }
}

impl Drop for EchoOff {
    fn drop(&mut self) {
        restore(&self.earlier);
        ECHO_SAVED.store(std::ptr::null_mut(), Ordering::SeqCst);
        let _ = self.modes.restore();
    }
}

/// Wait for the child `pid` to exit, and continue it each time it stops (`WUNTRACED`).
///
/// A harness that suspends itself (Ctrl-Z in its own key handling, `kill(0, SIGTSTP)`) would
/// otherwise stop while this process kept waiting for an exit, and the participant's shell, whose
/// foreground job is this process, would never get the terminal back. Suspension is not
/// supported during a run: `SIGCONT` goes to the child's process group at once, and `on_stopped`
/// hears the signal that stopped it. This process never stops itself.
///
/// The child is reaped here, so the caller must not wait for it again.
pub fn wait_resuming(
    pid: u32,
    on_stopped: &mut dyn FnMut(libc::c_int),
) -> std::io::Result<std::process::ExitStatus> {
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
            // SAFETY: integers only; the group is the stopped child's own.
            unsafe {
                let group = libc::getpgid(pid);
                if group > 0 {
                    libc::kill(-group, libc::SIGCONT);
                } else {
                    libc::kill(pid, libc::SIGCONT);
                }
            }
            on_stopped(libc::WSTOPSIG(status));
            continue;
        }
        return Ok(std::process::ExitStatus::from_raw(status));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn current(signal: libc::c_int) -> libc::sighandler_t {
        // SAFETY: a zeroed local the call writes.
        unsafe {
            let mut now: libc::sigaction = std::mem::zeroed();
            libc::sigaction(signal, std::ptr::null(), &mut now);
            now.sa_sigaction
        }
    }

    /// A shielded process outlives the terminal's signals and notes a hangup; once the harness
    /// has exited the typed keys are given back, and the hangup and `SIGTTOU` stay held until the
    /// shield drops.
    #[test]
    fn a_shield_absorbs_the_terminal_signals_and_gives_them_back() {
        let mut shield = Shield::take().unwrap();
        // SAFETY: signals to this process, whose handlers were just installed.
        unsafe {
            libc::raise(libc::SIGHUP);
            libc::raise(libc::SIGINT);
            libc::raise(libc::SIGTSTP);
            libc::raise(libc::SIGQUIT);
            libc::raise(libc::SIGTTOU);
        }
        assert!(hangup_seen());
        shield.harness_exited();
        assert_eq!(current(libc::SIGINT), libc::SIG_DFL);
        assert_eq!(current(libc::SIGTSTP), libc::SIG_DFL);
        assert_eq!(
            current(libc::SIGHUP),
            absorb as *const () as libc::sighandler_t
        );
        assert_eq!(current(libc::SIGTTOU), libc::SIG_IGN);
        drop(shield);
        assert_eq!(current(libc::SIGHUP), libc::SIG_DFL);
        assert_eq!(current(libc::SIGTTOU), libc::SIG_DFL);
    }

    /// A descriptor that is not a terminal has no echo to switch off.
    #[test]
    fn echo_is_left_alone_on_a_descriptor_that_is_not_a_terminal() {
        let file = std::fs::File::open("/dev/null").unwrap();
        use std::os::fd::AsRawFd;
        assert!(EchoOff::take(file.as_raw_fd()).is_none());
    }

    /// A process ended by a signal while it holds the echo off leaves the terminal as it found
    /// it: the handler puts the modes back and the signal then ends the process as it would
    /// have.
    #[test]
    fn a_signal_while_the_echo_is_off_puts_the_terminal_back() {
        let (mut master, mut slave) = (-1, -1);
        // SAFETY: `openpty` writes the two descriptors; the rest is null (not wanted).
        let opened = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(opened, 0);
        let echo_on = |fd| Modes::save(fd).unwrap().saved.c_lflag & libc::ECHO != 0;
        assert!(echo_on(slave));
        let held = EchoOff::take(slave).expect("a terminal");
        assert!(!echo_on(slave), "the echo is off while it is held");
        // SAFETY: the child only raises a signal, which is async-signal-safe.
        let child = unsafe { libc::fork() };
        if child == 0 {
            // SAFETY: as above; the handler restores the modes and re-raises.
            unsafe {
                libc::raise(libc::SIGTERM);
                libc::_exit(0);
            }
        }
        let mut status = 0;
        // SAFETY: `waitpid` writes the one integer it is handed.
        unsafe { libc::waitpid(child, &mut status, 0) };
        assert!(libc::WIFSIGNALED(status), "the signal still ended it");
        assert_eq!(libc::WTERMSIG(status), libc::SIGTERM);
        assert!(echo_on(slave), "the handler put the echo back");
        drop(held);
        assert!(echo_on(slave));
    }

    /// A child that stops itself is continued, and its exit is still the status waited for.
    #[test]
    #[allow(
        clippy::zombie_processes,
        reason = "`wait_resuming` reaps it, not `Child::wait`"
    )]
    fn a_child_that_stops_itself_is_continued_and_waited_for() {
        let child = std::process::Command::new("/bin/sh")
            .args(["-c", "kill -TSTP $$; kill -TSTP $$; exit 7"])
            .spawn()
            .unwrap();
        let mut stops = Vec::new();
        let status = wait_resuming(child.id(), &mut |signal| stops.push(signal)).unwrap();
        assert_eq!(status.code(), Some(7));
        assert_eq!(stops, [libc::SIGTSTP, libc::SIGTSTP]);
    }
}
