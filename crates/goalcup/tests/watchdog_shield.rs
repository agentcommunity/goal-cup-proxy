//! Point 3 (PRD-02 section 8 step 7, seam S8): a terminal key that is still pending when
//! `Shield::harness_exited` gives the keys back must not end the watchdog's wrap-up, and a key
//! typed after it still gets the default action.
//!
//! Each case runs in a forked child, which has one thread, so a signal blocked there stays pending
//! until the child unblocks it: the late Ctrl-C of a loaded run, made certain. Linux only.
#![cfg(target_os = "linux")]

use goalcup::watchdog::shield::Shield;

/// How the forked child ended: `Ok(code)` for an exit, `Err(signal)` for a signal.
fn in_child(case: impl FnOnce() -> i32) -> Result<i32, i32> {
    // SAFETY: the child runs only `case` and `_exit`; `case` uses libc calls, small allocations
    // and at most one thread, which glibc keeps usable after `fork`.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork: {}", std::io::Error::last_os_error());
    if pid == 0 {
        // SAFETY: ends the child without running the parent's destructors.
        unsafe { libc::_exit(case()) }
    }
    let mut status = 0;
    // SAFETY: `waitpid` writes the one integer it is handed.
    let reaped = unsafe { libc::waitpid(pid, &mut status, 0) };
    assert_eq!(reaped, pid, "waitpid: {}", std::io::Error::last_os_error());
    if libc::WIFEXITED(status) {
        Ok(libc::WEXITSTATUS(status))
    } else {
        Err(libc::WTERMSIG(status))
    }
}

fn mask(how: libc::c_int, signal: libc::c_int) {
    // SAFETY: a zeroed local set, filled before use.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, signal);
        libc::pthread_sigmask(how, &set, std::ptr::null_mut());
    }
}

/// The disposition a test process inherited is not ours to rely on.
fn default_action(signal: libc::c_int) {
    // SAFETY: integers only.
    unsafe { libc::signal(signal, libc::SIG_DFL) };
}

/// A late key: sent while this thread blocks it, then `harness_exited`, then a wrap-up span.
/// With `spare`, a second thread that keeps the signal unblocked runs the whole time, so the key
/// may be routed to it instead of staying pending.
fn late_key(signal: libc::c_int, spare: bool) -> i32 {
    default_action(signal);
    let Ok(mut shield) = Shield::take() else {
        return 2;
    };
    if spare {
        std::thread::spawn(|| {
            loop {
                std::thread::park();
            }
        });
    }
    mask(libc::SIG_BLOCK, signal);
    // SAFETY: integers only.
    unsafe { libc::kill(libc::getpid(), signal) };
    if !shield.harness_exited() {
        return 3;
    }
    mask(libc::SIG_UNBLOCK, signal);
    drop(shield);
    0
}

#[test]
fn a_pending_key_does_not_end_the_wrap_up() {
    for signal in [libc::SIGINT, libc::SIGQUIT] {
        for spare in [false, true] {
            let ended = in_child(|| late_key(signal, spare));
            assert_eq!(ended, Ok(0), "signal {signal}, spare thread {spare}");
        }
    }
}

#[test]
fn a_key_after_harness_exited_gets_the_default_action() {
    let ended = in_child(|| {
        default_action(libc::SIGINT);
        let Ok(mut shield) = Shield::take() else {
            return 2;
        };
        if !shield.harness_exited() {
            return 3;
        }
        // SAFETY: integers only. Unblocked on the only thread, so it is delivered before
        // `kill` returns.
        unsafe { libc::kill(libc::getpid(), libc::SIGINT) };
        drop(shield);
        0
    });
    assert_eq!(ended, Err(libc::SIGINT));
}
