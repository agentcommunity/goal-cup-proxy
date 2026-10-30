//! The clocks (PRD-02 section 1, PRD-01 section 6).
//!
//! - The record clock, [`Mono`]: boot time. Every timer of the recorder and the watchdog reads it.
//! - The awake clock, [`awake_ns`]: stops during sleep. Read only at a LINK, for the suspend event.
//! - The wall clock, [`wall_ns`]: Unix time. Paired with the record clock at every LINK.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::OnceLock;

/// Boot time in ns. It counts a suspend: `CLOCK_BOOTTIME` on Linux, `mach_continuous_time` on
/// macOS. `std::time::Instant` and tokio timers stop during a suspend, so no deadline is a
/// runtime sleep.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Mono(pub u64);

impl Mono {
    pub fn now() -> Mono {
        #[cfg(target_os = "linux")]
        let ns = read(libc::CLOCK_BOOTTIME);
        #[cfg(target_os = "macos")]
        let ns = mach::continuous_ns();
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let ns = read(libc::CLOCK_MONOTONIC);
        Mono(ns)
    }

    pub fn ns(self) -> u64 {
        self.0
    }
}

/// The awake clock in ns: `CLOCK_MONOTONIC` on Linux, `mach_absolute_time` on macOS. Both stop
/// during sleep, so the record clock's change minus this clock's change is the time asleep.
#[cfg(target_os = "macos")]
pub fn awake_ns() -> u64 {
    mach::absolute_ns()
}

/// The awake clock in ns: `CLOCK_MONOTONIC`, which stops during sleep.
#[cfg(not(target_os = "macos"))]
pub fn awake_ns() -> u64 {
    read(libc::CLOCK_MONOTONIC)
}

/// The wall clock in Unix ns (`CLOCK_REALTIME`); 0 before 1970 or if it cannot be read.
pub fn wall_ns() -> u64 {
    read(libc::CLOCK_REALTIME)
}

/// The wall clock and the record clock, read one right after the other.
pub fn now_pair() -> goalcup_format::Now {
    let mono_ns = Mono::now().0;
    let wall_ns = wall_ns();
    goalcup_format::Now { wall_ns, mono_ns }
}

/// The wall time of the record-clock time `m`, mapped through the pair `now` (both clocks of one
/// boot). Saturates at 0 and at `u64::MAX`.
pub fn wall_at(m: Mono, now: goalcup_format::Now) -> u64 {
    if m.0 <= now.mono_ns {
        now.wall_ns.saturating_sub(now.mono_ns - m.0)
    } else {
        now.wall_ns.saturating_add(m.0 - now.mono_ns)
    }
}

/// The id of the record clock, the boot it counts from, as 16 bytes. Linux reads
/// `/proc/sys/kernel/random/boot_id`. macOS reads `kern.bootsessionuuid`, which is unverified
/// until a macOS run shows it (PRD-01 section 6). All zero if it cannot be read.
pub fn boot_id() -> [u8; 16] {
    static ID: OnceLock<[u8; 16]> = OnceLock::new();
    *ID.get_or_init(|| parse_uuid(&boot_id_text().unwrap_or_default()))
}

#[cfg(target_os = "linux")]
fn boot_id_text() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()
}

#[cfg(target_os = "macos")]
fn boot_id_text() -> Option<String> {
    let mut buf = [0u8; 128];
    let mut len = buf.len();
    // SAFETY: the name is NUL-terminated, `buf` is writable for `len` bytes, and no new value is
    // set.
    let rc = unsafe {
        libc::sysctlbyname(
            c"kern.bootsessionuuid".as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    let text = &buf[..len.min(buf.len())];
    let end = text.iter().position(|&b| b == 0).unwrap_or(text.len());
    String::from_utf8(text[..end].to_vec()).ok()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn boot_id_text() -> Option<String> {
    None
}

/// UUID text (with or without dashes) to 16 bytes; all zero if it is not one.
fn parse_uuid(text: &str) -> [u8; 16] {
    let hex: String = text.trim().chars().filter(|c| *c != '-').collect();
    let mut out = [0u8; 16];
    if hex::decode_to_slice(&hex, &mut out).is_err() {
        return [0u8; 16];
    }
    out
}

/// More than this between two links is a clock jump or a suspend (PRD-01 section 6).
const DRIFT_NS: i128 = 2_000_000_000;

/// The three clocks at the last LINK. At each LINK the writer calls [`Drift::at_link`] and writes
/// the events it returns.
#[derive(Clone, Copy, Debug)]
pub struct Drift {
    wall: u64,
    mono: u64,
    awake: u64,
}

impl Drift {
    /// The clocks at a LINK: wall ns, record clock ns, awake clock ns.
    pub fn new(wall_ns: u64, mono_ns: u64, awake_ns: u64) -> Drift {
        Drift {
            wall: wall_ns,
            mono: mono_ns,
            awake: awake_ns,
        }
    }

    /// The clock and suspend events since the last LINK, then the given values become the base.
    /// `clock` when the wall clock moved more than 2 s away from the record clock, `suspend` when
    /// the record clock moved more than 2 s past the awake clock.
    pub fn at_link(&mut self, wall_ns: u64, mono_ns: u64, awake_ns: u64) -> Vec<Value> {
        let d_wall = i128::from(wall_ns) - i128::from(self.wall);
        let d_mono = i128::from(mono_ns) - i128::from(self.mono);
        let d_awake = i128::from(awake_ns) - i128::from(self.awake);
        let mut events = Vec::new();
        let jump = d_wall - d_mono;
        if jump.abs() > DRIFT_NS {
            events.push(json!({"ev": "clock", "jump_ns": clamp(jump)}));
        }
        let slept = d_mono - d_awake;
        if slept > DRIFT_NS {
            events.push(json!({"ev": "suspend", "ns": clamp(slept)}));
        }
        *self = Drift::new(wall_ns, mono_ns, awake_ns);
        events
    }
}

fn clamp(v: i128) -> i64 {
    v.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

/// One `clock_gettime` read in ns; 0 if it fails or is before 0.
fn read(clock: libc::clockid_t) -> u64 {
    // SAFETY: an all-zero timespec is valid, and clock_gettime only writes to it.
    let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
    // SAFETY: `ts` is a valid out pointer for the call.
    if unsafe { libc::clock_gettime(clock, &mut ts) } != 0 || ts.tv_sec < 0 {
        return 0;
    }
    (ts.tv_sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(ts.tv_nsec as u64)
}

#[cfg(target_os = "macos")]
mod mach {
    use std::sync::OnceLock;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }

    unsafe extern "C" {
        fn mach_continuous_time() -> u64;
        fn mach_absolute_time() -> u64;
        fn mach_timebase_info(info: *mut Timebase) -> libc::c_int;
    }

    fn scale(ticks: u64) -> u64 {
        static TB: OnceLock<Timebase> = OnceLock::new();
        let tb = *TB.get_or_init(|| {
            let mut tb = Timebase::default();
            // SAFETY: `tb` is a valid out pointer for the call.
            if unsafe { mach_timebase_info(&mut tb) } != 0 || tb.denom == 0 {
                return Timebase { numer: 1, denom: 1 };
            }
            tb
        });
        let ns = u128::from(ticks) * u128::from(tb.numer) / u128::from(tb.denom);
        u64::try_from(ns).unwrap_or(u64::MAX)
    }

    /// Time since boot, sleep included.
    pub(super) fn continuous_ns() -> u64 {
        // SAFETY: no arguments; it only reads the clock.
        scale(unsafe { mach_continuous_time() })
    }

    /// Time since boot, sleep left out.
    pub(super) fn absolute_ns() -> u64 {
        // SAFETY: no arguments; it only reads the clock.
        scale(unsafe { mach_absolute_time() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u64 = 1_000_000_000;

    #[test]
    fn a_one_second_wall_step_is_no_event() {
        let mut d = Drift::new(100 * S, 50 * S, 40 * S);
        assert!(d.at_link(102 * S, 51 * S, 41 * S).is_empty());
    }

    #[test]
    fn a_three_second_wall_jump_is_one_clock_event() {
        let mut d = Drift::new(100 * S, 50 * S, 40 * S);
        let ev = d.at_link(103 * S, 50 * S, 40 * S);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0]["ev"], "clock");
        assert_eq!(ev[0]["jump_ns"], 3 * S);
        // The new values are the base: the next link with no change is quiet.
        assert!(d.at_link(103 * S, 50 * S, 40 * S).is_empty());
    }

    #[test]
    fn three_seconds_asleep_is_one_suspend_event() {
        // Sleep moves the wall clock and the record clock together: no clock event.
        let mut d = Drift::new(100 * S, 50 * S, 40 * S);
        let ev = d.at_link(104 * S, 54 * S, 41 * S);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0]["ev"], "suspend");
        assert_eq!(ev[0]["ns"], 3 * S);
    }

    #[test]
    fn the_record_clock_never_goes_backwards_and_boot_id_is_stable() {
        let a = Mono::now();
        let b = Mono::now();
        assert!(b >= a);
        assert!(a.ns() > 0);
        assert_eq!(boot_id(), boot_id());
        #[cfg(target_os = "linux")]
        assert_ne!(boot_id(), [0u8; 16]);
    }

    #[test]
    fn wall_at_maps_through_the_pair() {
        let now = goalcup_format::Now {
            wall_ns: 1_000 * S,
            mono_ns: 10 * S,
        };
        assert_eq!(wall_at(Mono(7 * S), now), 997 * S);
        assert_eq!(wall_at(Mono(12 * S), now), 1_002 * S);
        let early = goalcup_format::Now {
            wall_ns: 1,
            mono_ns: 5 * S,
        };
        assert_eq!(wall_at(Mono(0), early), 0);
    }
}
