//! The descriptor limit. A test that opens more than 500 descriptors calls [`raise`] first
//! (PRD-14 section 4): the default soft limit is 1,024 on Linux and 256 on macOS.

/// Lifts the soft `RLIMIT_NOFILE` to at least `n`. Fails the test with one clear line when the
/// hard limit is lower than `n`.
pub fn raise(n: u64) {
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit writes into the struct we own.
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) };
    assert_eq!(rc, 0, "getrlimit(RLIMIT_NOFILE) failed");
    let want = n as libc::rlim_t;
    if lim.rlim_cur >= want {
        return;
    }
    assert!(
        lim.rlim_max >= want,
        "this test needs {n} file descriptors but the hard limit is {}; raise it (ulimit -Hn)",
        lim.rlim_max
    );
    lim.rlim_cur = want;
    // SAFETY: setrlimit reads the struct we own.
    let rc = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &lim) };
    assert_eq!(rc, 0, "setrlimit(RLIMIT_NOFILE, {n}) failed");
}
