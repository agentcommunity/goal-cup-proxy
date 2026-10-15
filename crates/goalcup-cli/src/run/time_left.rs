//! What is left of a run, as the server last said it, kept where every `goalcup` command can
//! read it.
//!
//! The server is the clock (WP11). The host counts the run down from the server's last answer
//! on a clock that keeps running while the machine sleeps
//! ([`goalcup_protocol::clock::continuous_ns`]), and writes that count into the run's
//! `state.json` ([`super::RunState::time_left`]). `goalcup status`, the reserve's release and
//! the host's own "is the run over" read it back, so none of them reads this machine's date,
//! which the participant can change, while the count holds ([`time_up`]).
//!
//! The continuous clock starts again at each boot, so a count is kept with the boot it was
//! taken on, and a reboot ends what it can say: the boot id changed (where the OS gives one), or
//! the clock reads below the count's start. Then, and for a `state.json` with no count (an older
//! build wrote it), the server's `deadline_at` is held against this machine's date, as before
//! WP11. A date set forward then only makes the run look over early to the participant
//! themselves; `goalcup continue` asks the server in any case.

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

    /// Whether the run's time is up now by the count; `None` when the machine has rebooted
    /// since the count was taken, and the count can say nothing.
    pub fn up(&self) -> Option<bool> {
        self.up_at(continuous_ns(), boot_id().as_deref())
    }

    /// [`TimeLeft::up`] with the continuous clock and the boot given. A reboot is a boot id
    /// that changed (both known), or a clock below the count's start.
    pub fn up_at(&self, now_ns: u64, boot: Option<&str>) -> Option<bool> {
        let other_boot =
            matches!((self.boot.as_deref(), boot), (Some(then), Some(now)) if then != now);
        if other_boot || now_ns < self.counted_from_ns {
            return None;
        }
        let passed_ns = now_ns - self.counted_from_ns;
        Some(u128::from(passed_ns) >= u128::from(self.seconds) * 1_000_000_000)
    }
}

/// Whether the run's time is up: by `left`, the count the host kept, while it holds. With no
/// count (a `state.json` an older build wrote, or no state at all), or after a reboot, the
/// server's `deadline_at` is held against this machine's date (`now`, unix seconds).
pub fn time_up(left: Option<&TimeLeft>, deadline_at: Option<&str>, now: u64) -> bool {
    match left.and_then(TimeLeft::up) {
        Some(up) => up,
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
    fn the_count_holds_on_its_own_boot_and_says_nothing_after_a_reboot() {
        let left = TimeLeft {
            seconds: 100,
            counted_from_ns: 1_000 * SECOND,
            boot: Some("a".to_string()),
        };
        // The same boot: exactly 100 s after the count.
        assert_eq!(left.up_at(1_099 * SECOND, Some("a")), Some(false));
        assert_eq!(left.up_at(1_100 * SECOND, Some("a")), Some(true));
        // Another boot id, or a clock below the count's start: a reboot.
        assert_eq!(left.up_at(5_000 * SECOND, Some("b")), None);
        assert_eq!(left.up_at(99 * SECOND, Some("a")), None);
        assert_eq!(left.up_at(99 * SECOND, None), None);
        // No boot id on either side, and the clock past the start: the count holds.
        assert_eq!(left.up_at(1_100 * SECOND, None), Some(true));
        let unknown = TimeLeft {
            boot: None,
            ..left.clone()
        };
        assert_eq!(unknown.up_at(1_099 * SECOND, Some("b")), Some(false));
        assert_eq!(unknown.up_at(99 * SECOND, Some("b")), None);
    }

    /// After a reboot the run ends at the server's `deadline_at` by this machine's date, as it
    /// did before WP11: neither held open for good nor ended by the stale count.
    #[test]
    fn after_a_reboot_the_run_ends_at_its_deadline_by_the_date() {
        let deadline = 1_000_000;
        let deadline_at = goalcup_protocol::control::format_z(deadline);
        let rebooted = |seconds| TimeLeft {
            seconds,
            counted_from_ns: continuous_ns() + 1_000 * SECOND,
            boot: boot_id(),
        };
        for left in [rebooted(3_600), rebooted(0)] {
            assert!(
                !time_up(Some(&left), Some(&deadline_at), deadline - 1),
                "{left:?}"
            );
            assert!(
                time_up(Some(&left), Some(&deadline_at), deadline),
                "{left:?}"
            );
        }
        let other_boot = TimeLeft {
            boot: Some("another boot".to_string()),
            ..TimeLeft::counted(3_600, continuous_ns())
        };
        assert!(!time_up(
            Some(&other_boot),
            Some(&deadline_at),
            deadline - 1
        ));
        assert!(time_up(Some(&other_boot), Some(&deadline_at), deadline));
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
