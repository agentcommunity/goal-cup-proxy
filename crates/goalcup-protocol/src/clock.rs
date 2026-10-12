//! The monotonic clock every recorded time is read from.
//!
//! `std::time::Instant` is the wrong clock for anything written to a record: on macOS it reads
//! `CLOCK_UPTIME_RAW` (`mach_absolute_time`), which stops while the machine sleeps. A run left
//! with the lid closed overnight would record its later times hours early. [`continuous_ns`]
//! reads a clock that keeps counting through sleep instead: `CLOCK_MONOTONIC_RAW` on macOS
//! (the same count as `mach_continuous_time`) and `CLOCK_BOOTTIME` on Linux.
//!
//! `Instant` stays right for timeouts and back-off, where a sleep should not count.

/// Nanoseconds since boot on a clock that keeps counting while the machine sleeps.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn continuous_ns() -> u64 {
    #[cfg(target_os = "macos")]
    const CLOCK: libc::clockid_t = libc::CLOCK_MONOTONIC_RAW;
    #[cfg(target_os = "linux")]
    const CLOCK: libc::clockid_t = libc::CLOCK_BOOTTIME;
    read(CLOCK)
}

/// Nanoseconds on `Instant`, from the first call in this process. This fallback is only
/// for targets this build does not support, where no sleep-counting clock is wired up.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn continuous_ns() -> u64 {
    static BASE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let base = *BASE.get_or_init(std::time::Instant::now);
    u64::try_from(base.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn read(clock: libc::clockid_t) -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid, writable timespec, and `clock` is a clock this OS defines.
    // clock_gettime cannot fail for a valid clock and pointer; a failure reads as zero.
    if unsafe { libc::clock_gettime(clock, &mut ts) } != 0 {
        return 0;
    }
    let secs = u64::try_from(ts.tv_sec).unwrap_or(0);
    let nanos = u64::try_from(ts.tv_nsec).unwrap_or(0);
    secs.saturating_mul(1_000_000_000).saturating_add(nanos)
}

/// A fixed point on [`continuous_ns`]: what `Instant` is, for a recorded time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContinuousOrigin(u64);

impl ContinuousOrigin {
    pub fn now() -> Self {
        ContinuousOrigin(continuous_ns())
    }

    /// Nanoseconds since this origin, sleep included.
    pub fn elapsed_ns(&self) -> u64 {
        continuous_ns().saturating_sub(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    /// The clock keeps pace with the wall clock while the machine is awake.
    #[test]
    fn continuous_clock_keeps_pace_with_the_wall_clock() {
        let origin = ContinuousOrigin::now();
        let wall = SystemTime::now();
        std::thread::sleep(Duration::from_millis(200));
        let continuous = origin.elapsed_ns();
        let walled = u64::try_from(wall.elapsed().unwrap().as_nanos()).unwrap();
        let drift = continuous.abs_diff(walled);
        assert!(
            drift < 100_000_000,
            "continuous {continuous} ns against wall {walled} ns"
        );
    }

    /// Measures a real sleep gap against an independent witness: the wall clock minus the
    /// kernel's boot time is the real time since boot, sleep included. The continuous clock
    /// must agree with it; the clock `Instant` reads falls behind it by every second this Mac
    /// has slept since boot.
    #[cfg(target_os = "macos")]
    #[test]
    fn continuous_clock_counts_the_sleep_instant_misses() {
        let mut boot = libc::timeval {
            tv_sec: 0,
            tv_usec: 0,
        };
        let mut size = std::mem::size_of::<libc::timeval>();
        // SAFETY: `boot` is a writable timeval of `size` bytes; the name is NUL-terminated.
        let status = unsafe {
            libc::sysctlbyname(
                c"kern.boottime".as_ptr(),
                (&mut boot as *mut libc::timeval).cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        assert_eq!(status, 0, "kern.boottime");
        let boot = Duration::new(boot.tv_sec as u64, boot.tv_usec as u32 * 1_000);
        let since_boot = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .saturating_sub(boot);
        let continuous = Duration::from_nanos(continuous_ns());
        let uptime = Duration::from_nanos(read(libc::CLOCK_UPTIME_RAW));

        // A minute of slack for wall-clock adjustments since boot.
        let off = continuous.abs_diff(since_boot);
        assert!(
            off < Duration::from_secs(60),
            "the continuous clock reads {continuous:?} since boot, the wall clock {since_boot:?}"
        );
        eprintln!(
            "this Mac has slept {:?} since boot; Instant's clock missed all of it",
            since_boot.saturating_sub(uptime)
        );
    }
}
