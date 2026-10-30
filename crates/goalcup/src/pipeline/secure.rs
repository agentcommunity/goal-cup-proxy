//! What a recorder boot does before it reads anything (PRD-02 section 9.2 step 1): no core dump
//! can hold a recorded byte or the CA key, the kernel kills the recorder before the harness under
//! memory pressure, and a panic never prints its message.

use std::io::Write;

/// The unused `prctl` arguments, as the `unsigned long` the kernel reads.
#[cfg(target_os = "linux")]
const Z: libc::c_ulong = 0;

/// Core dumps off (`RLIMIT_CORE` 0; on Linux also `PR_SET_DUMPABLE` 0) and, on Linux,
/// `oom_score_adj` 500. Every failure is ignored: recording goes on without it.
pub fn harden() {
    let zero = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `zero` is a valid rlimit for the call.
    let _ = unsafe { libc::setrlimit(libc::RLIMIT_CORE, &zero) };
    #[cfg(target_os = "linux")]
    {
        // Before PR_SET_DUMPABLE: a process that is not dumpable no longer owns its /proc files.
        let _ = std::fs::write("/proc/self/oom_score_adj", "500");
        // SAFETY: PR_SET_DUMPABLE reads its integer arguments and changes no memory.
        let _ = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, Z, Z, Z, Z) };
    }
}

/// A panic hook that writes a fixed text and the panic's file and line to stderr, never the
/// payload: an `expect` or a `panic!` could format a recorded value or a key.
///
/// `goalcup.log` gets the same line once PRD-10's `cli::log` (seam S11) lands; this file does not
/// write the log itself.
pub fn panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let line = match info.location() {
            Some(at) => format!("goalcup: internal error at {}:{}\n", at.file(), at.line()),
            None => "goalcup: internal error\n".to_string(),
        };
        let _ = std::io::stderr().lock().write_all(line.as_bytes());
    }));
}
