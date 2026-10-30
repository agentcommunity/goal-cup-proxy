//! Boot time: the one clock every timer of the recorder and the watchdog reads (PRD-02 section 1).

use serde::{Deserialize, Serialize};

/// Boot time in ns. It counts a suspend: `CLOCK_BOOTTIME` on Linux, `CLOCK_MONOTONIC` on macOS
/// (which counts sleep there). `std::time::Instant` and tokio timers stop during a suspend, so no
/// deadline is a runtime sleep.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Mono(pub u64);

impl Mono {
    pub fn now() -> Mono {
        #[cfg(target_os = "linux")]
        const CLOCK: libc::clockid_t = libc::CLOCK_BOOTTIME;
        #[cfg(not(target_os = "linux"))]
        const CLOCK: libc::clockid_t = libc::CLOCK_MONOTONIC;
        // SAFETY: an all-zero timespec is valid, and clock_gettime only writes to it.
        let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
        // SAFETY: `ts` is a valid out pointer for the call.
        if unsafe { libc::clock_gettime(CLOCK, &mut ts) } != 0 {
            return Mono(0);
        }
        Mono(ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64)
    }

    pub fn ns(self) -> u64 {
        self.0
    }
}
