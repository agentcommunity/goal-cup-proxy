//! Open-file limits (PRD-02 section 10).

/// Whose limit `raise` sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The soft limit to the hard limit.
    Recorder,
    /// `max(current soft, 4096)`, at most the hard limit (and `OPEN_MAX` on macOS).
    Harness,
    /// As the harness.
    Watchdog,
}

/// True while `2 x open connections + 64` is under the recorder's limit. PRD-03 asks before each
/// `accept`; while false it waits 10 ms, so new connections wait in the backlog and none is refused.
pub fn may_accept() -> bool {
    true
}

/// Raises this process's soft open-file limit for `target` and returns the limit set. The
/// scaffold raises nothing and returns the current soft limit (0 if it cannot be read).
pub fn raise(target: Target) -> u64 {
    let _ = target;
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `lim` is a valid out pointer for the call.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) } != 0 {
        return 0;
    }
    lim.rlim_cur
}
