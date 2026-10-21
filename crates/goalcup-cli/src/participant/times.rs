//! Times as the participant reads them, and what is left of a run.

/// A moment as the participant reads it: this computer's local time with its zone, and UTC in
/// brackets, because a short zone name alone is not unique and the event speaks UTC:
/// `Fri 7 Nov, 10:12 WITA (02:12 UTC)`. The UTC date is named when it is not the local one:
/// `Sat 14 Nov, 00:12 CET (Fri 13 Nov, 23:12 UTC)`. A computer on UTC, or one whose zone cannot
/// be read, gets UTC once.
pub fn local_clock(unix: u64) -> String {
    match local_offset(unix) {
        Some((offset, zone)) => clock_words(unix, offset, &zone),
        None => clock_words(unix, 0, "UTC"),
    }
}

/// [`local_clock`] with the zone given: `offset_secs` east of UTC, and its name. A name that is
/// empty or not plain text is replaced by the offset (`UTC+05:30`).
pub fn clock_words(unix: u64, offset_secs: i64, zone: &str) -> String {
    let local = i64::try_from(unix)
        .unwrap_or(i64::MAX)
        .saturating_add(offset_secs)
        .max(0) as u64;
    let (local_day, local_time) = day_and_time(local);
    if offset_secs == 0 {
        return format!("{local_day}, {local_time} UTC");
    }
    let zone = if !zone.is_empty() && zone.chars().all(|c| c.is_ascii_alphabetic()) {
        zone.to_string()
    } else {
        let sign = if offset_secs < 0 { '-' } else { '+' };
        let off = offset_secs.unsigned_abs();
        format!("UTC{sign}{:02}:{:02}", off / 3600, off % 3600 / 60)
    };
    let (utc_day, utc_time) = day_and_time(unix);
    if utc_day == local_day {
        format!("{local_day}, {local_time} {zone} ({utc_time} UTC)")
    } else {
        format!("{local_day}, {local_time} {zone} ({utc_day}, {utc_time} UTC)")
    }
}

/// `Fri 7 Nov` and `10:12` of a count of seconds since 1970 read as a wall clock.
fn day_and_time(secs: u64) -> (String, String) {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let stamp = goalcup_protocol::control::format_z(secs);
    // `YYYY-MM-DDTHH:MM:SSZ`, fixed width.
    let month: usize = stamp[5..7].parse().unwrap_or(1);
    let day: u32 = stamp[8..10].parse().unwrap_or(1);
    // 1970-01-01 was a Thursday.
    let weekday = DAYS[((secs / 86_400 + 4) % 7) as usize];
    (
        format!("{weekday} {day} {}", MONTHS[month.clamp(1, 12) - 1]),
        stamp[11..16].to_string(),
    )
}

/// This computer's offset from UTC at `unix`, and the zone's short name.
#[cfg(unix)]
fn local_offset(unix: u64) -> Option<(i64, String)> {
    unsafe extern "C" {
        fn tzset();
    }
    let t: libc::time_t = unix.try_into().ok()?;
    // SAFETY: `tm` is plain data, valid when all zero.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `tzset` reads `TZ` and the zone files and sets libc's own zone state; it takes no
    // arguments. `localtime_r` writes only into `tm`, which outlives the call.
    let done = unsafe {
        tzset();
        libc::localtime_r(&t, &mut tm)
    };
    if done.is_null() {
        return None;
    }
    let zone = if tm.tm_zone.is_null() {
        String::new()
    } else {
        // SAFETY: a non-null `tm_zone` from `localtime_r` points at a NUL-terminated name in
        // libc's static zone table, which lives as long as the process.
        unsafe { std::ffi::CStr::from_ptr(tm.tm_zone) }
            .to_string_lossy()
            .into_owned()
    };
    // `c_long` is 32 bits on some targets.
    #[allow(clippy::useless_conversion)]
    let offset = i64::from(tm.tm_gmtoff);
    Some((offset, zone))
}

#[cfg(not(unix))]
fn local_offset(_unix: u64) -> Option<(i64, String)> {
    None
}

/// A time left in words: `95 h 10 min left`, `10 min left`, `less than a minute left`.
pub fn left_words(secs: u64) -> String {
    match secs {
        0..60 => "less than a minute left".to_string(),
        60..3600 => format!("{} min left", secs / 60),
        _ => format!("{} h {} min left", secs / 3600, secs % 3600 / 60),
    }
}

/// A run's end as the participant reads it:
/// `Sat 14 Nov, 10:12 CET (09:12 UTC), 95 h 10 min left`.
/// `deadline_at` is the server's `…Z` stamp; `left` is the server's count of what is left, when
/// there is one. A stamp that does not parse is shown as it is.
pub fn end_words(deadline_at: &str, left: Option<u64>) -> String {
    let at = goalcup_protocol::control::parse_z(deadline_at)
        .map_or_else(|| deadline_at.to_string(), local_clock);
    match left {
        Some(left) => format!("{at}, {}", left_words(left)),
        None => at,
    }
}

/// [`seconds_left`] of `run` by the count its `state` keeps: what `goalcup status`, `goalcup
/// seal` and `goalcup time-left` all say is left of a run.
pub fn run_seconds_left(
    run: &crate::run::RunRecord,
    state: Option<&crate::run::RunState>,
    now: u64,
) -> Option<u64> {
    seconds_left(
        state.and_then(|state| state.time_left.as_ref()),
        run.deadline_at.as_deref(),
        now,
    )
}

/// What is left of a run now: by the server's count when it holds (the same boot), else by the
/// server's `deadline_at` against this machine's date (`now`). Only for words: whether the run is
/// over is [`crate::run::is_over`]'s to say.
pub fn seconds_left(
    count: Option<&crate::run::TimeLeft>,
    deadline_at: Option<&str>,
    now: u64,
) -> Option<u64> {
    let counted = count.and_then(|count| {
        let then = count.boot.as_deref()?;
        if crate::run::time_left::boot_id().is_some_and(|boot| boot != then) {
            return None;
        }
        let passed = goalcup_protocol::clock::continuous_ns().checked_sub(count.counted_from_ns)?;
        Some(count.seconds.saturating_sub(passed / 1_000_000_000))
    });
    counted.or_else(|| {
        deadline_at
            .and_then(goalcup_protocol::control::parse_z)
            .map(|deadline| deadline.saturating_sub(now))
    })
}

/// `goalcup time-left` inside the agent's session, with no fresh time-left file: the advice
/// first, then why there is no number.
pub fn time_left_unknown() -> String {
    format!(
        "{} The time left is unknown right now: there is no fresh time-left file.",
        crate::run::time_left::PUSH_OFTEN
    )
}

/// `goalcup time-left` in a person's own terminal, with no open run on this computer.
pub fn no_open_run_time_left() -> &'static str {
    "Run goalcup status to see your runs. No run is open on this computer, so there is no time \
     left to show."
}
