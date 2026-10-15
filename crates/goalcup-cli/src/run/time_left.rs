//! What is left of a run, as the server last said it, kept where every `goalcup` command can
//! read it.
//!
//! The server is the clock (WP11). The host counts the run down from the server's last answer
//! on a clock that keeps running while the machine sleeps
//! ([`goalcup_protocol::clock::continuous_ns`]), and writes that count into the run's
//! `state.json` ([`super::RunState::time_left`]). `goalcup status`, the reserve's release and
//! the host's own "is the run over" read it back, so none of them reads this machine's date,
//! which the participant can change. Only a `state.json` with no count (an older build wrote it)
//! falls back to the date ([`time_up`]).
//!
//! The continuous clock starts again at each boot, so a count is kept with the boot it was
//! taken on. On another boot only the time since this boot is known to have passed: the count
//! then ends late, never early.

use goalcup_protocol::clock::continuous_ns;
use serde::{Deserialize, Serialize};

/// The run's time left at one instant of the continuous clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeLeft {
    /// Whole seconds left of the run at `counted_from_ns`.
    pub seconds: u64,
    /// The continuous clock, in nanoseconds since boot, when `seconds` were left.
    pub counted_from_ns: u64,
    /// The boot `counted_from_ns` was read on; `None` when this machine does not say.
    #[serde(default)]
    pub boot: Option<String>,
}

impl TimeLeft {
    /// `seconds` left at `counted_from_ns`, on this boot.
    pub fn counted(seconds: u64, counted_from_ns: u64) -> TimeLeft {
        TimeLeft {
            seconds,
            counted_from_ns,
            boot: boot_id(),
        }
    }

    /// Whether the run's time is up now.
    pub fn up(&self) -> bool {
        self.up_at(continuous_ns(), boot_id().as_deref())
    }

    /// [`TimeLeft::up`] with the continuous clock and the boot given. On the same boot the
    /// count is exact. On another boot, or when a boot cannot be told, the least time that can
    /// have passed is counted: the time since this boot (a clock below the count's start is
    /// one that started again).
    pub fn up_at(&self, now_ns: u64, boot: Option<&str>) -> bool {
        let same_boot = self.boot.is_some() && self.boot.as_deref() == boot;
        let other_boot = self.boot.is_some() && boot.is_some() && !same_boot;
        let passed_ns = if other_boot || now_ns < self.counted_from_ns {
            now_ns
        } else {
            now_ns - self.counted_from_ns
        };
        u128::from(passed_ns) >= u128::from(self.seconds) * 1_000_000_000
    }
}

/// Whether the run's time is up: by `left`, the count the host kept, when there is one. Only
/// without one (a `state.json` an older build wrote, or no state at all) is `deadline_at` held
/// against this machine's date (`now`, unix seconds).
pub fn time_up(left: Option<&TimeLeft>, deadline_at: Option<&str>, now: u64) -> bool {
    match left {
        Some(left) => left.up(),
        None => deadline_at
            .and_then(goalcup_protocol::control::parse_z)
            .is_some_and(|deadline| now >= deadline),
    }
}

/// This boot's identity: Linux's `boot_id`, macOS's `kern.bootsessionuuid`. `None` when it
/// cannot be read.
pub fn boot_id() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
        let id = id.trim();
        (!id.is_empty()).then(|| id.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        let mut buf = [0u8; 64];
        let mut size = buf.len();
        // SAFETY: `buf` is a writable buffer of `size` bytes; the name is NUL-terminated.
        let status = unsafe {
            libc::sysctlbyname(
                c"kern.bootsessionuuid".as_ptr(),
                buf.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if status != 0 {
            return None;
        }
        let text = std::str::from_utf8(&buf[..size.min(buf.len())]).ok()?;
        let id = text.trim_end_matches('\0').trim();
        (!id.is_empty()).then(|| id.to_string())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: u64 = 1_000_000_000;

    #[test]
    fn the_count_is_exact_on_its_own_boot_and_never_early_on_another() {
        let left = TimeLeft {
            seconds: 100,
            counted_from_ns: 1_000 * SECOND,
            boot: Some("a".to_string()),
        };
        // The same boot: exactly 100 s after the count.
        assert!(!left.up_at(1_099 * SECOND, Some("a")));
        assert!(left.up_at(1_100 * SECOND, Some("a")));
        // Another boot: only the time since it is known to have passed.
        assert!(!left.up_at(99 * SECOND, Some("b")));
        assert!(left.up_at(100 * SECOND, Some("b")));
        assert!(!left.up_at(1_099 * SECOND, None), "no boot here: not early");
        // No boot on the count: a clock below the count's start started again.
        let unknown = TimeLeft {
            boot: None,
            ..left.clone()
        };
        assert!(!unknown.up_at(1_099 * SECOND, Some("a")));
        assert!(unknown.up_at(1_100 * SECOND, Some("a")));
        assert!(!unknown.up_at(99 * SECOND, None));
        assert!(unknown.up_at(100 * SECOND, None));
    }

    /// The count, not the date, says whether the run's time is up: a date set a day forward (or
    /// back) changes nothing. Only with no count is the date read.
    #[test]
    fn a_date_set_forward_does_not_end_the_count() {
        let deadline = 1_000_000;
        let deadline_at = goalcup_protocol::control::format_z(deadline);
        let a_day_on = deadline + 86_400;
        let left = TimeLeft::counted(3_600, continuous_ns());
        assert!(!time_up(Some(&left), Some(&deadline_at), a_day_on));
        let spent = TimeLeft::counted(0, continuous_ns());
        assert!(time_up(Some(&spent), Some(&deadline_at), deadline - 86_400));
        // No count: an older build's state, held against the date.
        assert!(time_up(None, Some(&deadline_at), deadline));
        assert!(!time_up(None, Some(&deadline_at), deadline - 1));
        assert!(!time_up(None, None, a_day_on));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn this_machine_names_its_boot_the_same_way_each_time() {
        let boot = boot_id().expect("a boot id");
        assert_eq!(boot_id().as_deref(), Some(boot.as_str()));
    }
}
