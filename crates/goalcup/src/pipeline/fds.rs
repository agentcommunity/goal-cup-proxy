//! Open-file limits and the accept reserve (PRD-02 section 10).
//!
//! The recorder needs about 64 fixed descriptors plus 2 per open connection. No connection is ever
//! refused here: while [`may_accept`] is false the accept loop waits 10 ms and asks again (PRD-03),
//! so new connections wait in the backlog.

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

/// Whose limit `raise` sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The soft limit to the hard limit.
    Recorder,
    /// The harness and the watchdog: `max(current soft, 4096)`, at most the hard limit (and
    /// `OPEN_MAX` on macOS).
    Harness,
}

/// macOS `setrlimit` refuses `RLIM_INFINITY` for `RLIMIT_NOFILE` and asks for
/// `min(OPEN_MAX, rlim_max)` (setrlimit(2), COMPATIBILITY).
#[cfg(target_os = "macos")]
const OPEN_MAX: u64 = 10_240;

/// The harness and watchdog floor, and the recorder's fallback when the hard limit is refused.
const FLOOR: u64 = 4_096;

/// Descriptors the recorder holds besides its connections.
const FIXED: u64 = 64;

/// Raises this process's soft open-file limit for `target` and returns the soft limit in force
/// after the call (0 if it cannot be read). Never lowers it. The recorder's raise runs in its
/// `pre_exec`, so [`may_accept`] reads the limit itself.
pub fn raise(target: Target) -> u64 {
    let Some((soft, hard)) = get() else {
        return 0;
    };
    let cap = most(hard);
    match target {
        Target::Recorder => {
            if soft < cap && !set(cap, hard) {
                let floor = FLOOR.min(cap);
                if soft < floor {
                    set(floor, hard);
                }
            }
        }
        Target::Harness => {
            let want = soft.max(FLOOR).min(cap);
            if want > soft {
                set(want, hard);
            }
        }
    }
    get().map_or(soft, |(s, _)| s)
}

/// True while `2 x open connections + 64` is under the recorder's soft limit (read once). Open
/// connections are the `conn` streams not yet ended, counted by `StreamHandle`. PRD-03 asks before each `accept`; while false it waits
/// 10 ms and asks again. Turning false after being true starts one episode for [`take_note`].
pub fn may_accept() -> bool {
    RESERVE.may_accept()
}

/// A `conn` stream opened.
pub(crate) fn conn_opened() {
    RESERVE.open.fetch_add(1, Ordering::Relaxed);
}

/// A `conn` stream ended.
pub(crate) fn conn_closed() {
    RESERVE.closed();
}

/// `(limit, open connections)` once per episode in which [`may_accept`] said no, for the one
/// `note` `fd_limit` event. The episode ends when `may_accept` is true again.
pub fn take_note() -> Option<(u64, u64)> {
    RESERVE.take_note()
}

static RESERVE: Reserve = Reserve::new();

const IDLE: u8 = 0;
const PENDING: u8 = 1;
const NOTED: u8 = 2;

/// The limit, the open connections and the episode state. Atomics only, no lock.
#[derive(Debug)]
struct Reserve {
    /// 0 until read once.
    limit: AtomicU64,
    open: AtomicU64,
    episode: AtomicU8,
}

impl Reserve {
    const fn new() -> Reserve {
        Reserve {
            limit: AtomicU64::new(0),
            open: AtomicU64::new(0),
            episode: AtomicU8::new(IDLE),
        }
    }

    fn limit(&self) -> u64 {
        let limit = self.limit.load(Ordering::Relaxed);
        if limit != 0 {
            return limit;
        }
        // Unreadable: nothing to hold back against, so every accept goes ahead.
        let read = get().map_or(u64::MAX, |(soft, _)| soft);
        match self
            .limit
            .compare_exchange(0, read, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => read,
            Err(set) => set,
        }
    }

    fn may_accept(&self) -> bool {
        let open = self.open.load(Ordering::Relaxed);
        let ok = open.saturating_mul(2).saturating_add(FIXED) < self.limit();
        if ok {
            self.episode.store(IDLE, Ordering::Relaxed);
        } else {
            let _ =
                self.episode
                    .compare_exchange(IDLE, PENDING, Ordering::Relaxed, Ordering::Relaxed);
        }
        ok
    }

    fn take_note(&self) -> Option<(u64, u64)> {
        self.episode
            .compare_exchange(PENDING, NOTED, Ordering::Relaxed, Ordering::Relaxed)
            .ok()
            .map(|_| (self.limit(), self.open.load(Ordering::Relaxed)))
    }

    fn closed(&self) {
        let _ = self
            .open
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| Some(n.saturating_sub(1)));
    }
}

/// The highest soft limit to ask for: `min(OPEN_MAX, hard)` on macOS.
#[cfg(target_os = "macos")]
fn most(hard: u64) -> u64 {
    hard.min(OPEN_MAX)
}

/// The highest soft limit to ask for: the hard limit.
#[cfg(not(target_os = "macos"))]
fn most(hard: u64) -> u64 {
    hard
}

/// `(soft, hard)` for `RLIMIT_NOFILE`.
fn get() -> Option<(u64, u64)> {
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `lim` is a valid out pointer for the call.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) } != 0 {
        return None;
    }
    Some((lim.rlim_cur, lim.rlim_max))
}

/// Sets the soft limit, keeping the hard one. True if it was accepted.
fn set(soft: u64, hard: u64) -> bool {
    let lim = libc::rlimit {
        rlim_cur: soft,
        rlim_max: hard,
    };
    // SAFETY: `setrlimit` only reads the struct it is handed.
    unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &lim) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    // nextest runs each test in its own process, so lowering this process's soft limit is safe.
    #[test]
    fn raise_from_256() {
        let (_, hard) = get().unwrap();
        let cap = most(hard);
        let low = 256.min(hard);

        assert!(set(low, hard));
        assert_eq!(raise(Target::Harness), low.max(FLOOR).min(cap));

        assert!(set(low, hard));
        let got = raise(Target::Recorder);
        assert!(got == cap || got == FLOOR.min(cap), "{got} {cap}");
    }

    #[test]
    fn the_reserve_holds_back_then_lets_go_and_notes_once_per_episode() {
        static R: Reserve = Reserve::new();
        R.limit.store(100, Ordering::Relaxed);
        // 2 x 17 + 64 = 98 < 100; 2 x 18 + 64 = 100 is not.
        R.open.store(17, Ordering::Relaxed);
        assert!(R.may_accept());
        assert_eq!(R.take_note(), None);

        R.open.store(18, Ordering::Relaxed);
        assert!(!R.may_accept());
        assert_eq!(R.take_note(), Some((100, 18)));
        assert!(!R.may_accept());
        assert_eq!(R.take_note(), None);

        R.closed();
        assert!(R.may_accept());
        assert_eq!(R.take_note(), None);

        R.open.store(18, Ordering::Relaxed);
        assert!(!R.may_accept());
        assert_eq!(R.take_note(), Some((100, 18)));
        R.closed();
        assert!(R.may_accept());
    }
}
