//! What the participant reads during a run: the approval, the launch, the harness's exit, the
//! deadline, `goalcup continue`, and the refusals of a start or a continue. All of it is here,
//! as plain functions of the facts, so it is worded in one place and tested in one place.
//!
//! The rules (the UX plan, section 0):
//!
//! - Plain words and short sentences. No rule codes, and no internal words (entry, segment,
//!   chain, seal.json, daemon) in a participant block.
//! - A harness is named by its manifest's `participant.display_name` only
//!   ([`crate::connector::HarnessConnector::display_name`]), never by a name in this file.
//! - A block ends with what to do next, or with "Nothing more to do". A refusal says what to do
//!   first, then why.
//! - Times are the participant's local time with its zone, and the time left comes from the
//!   server's count when there is one ([`crate::run::TimeLeft`]).
//!
//! Some words are a contract: scripts and tests read them. They are the `pub const`s below, and
//! the technical lines `goalcup start` prints after the harness exits (`deadline:`,
//! `harness exited:`, `uploaded: … and the seal`), which stay as they were under
//! [`details_heading`]. A test asserts these constants and the facts (a run id, a command, a
//! time), never a whole sentence.

use crate::account::device::RunMode;
use std::path::Path;

/// The ready mark: the last line before a chat-mode harness takes the terminal.
/// `scripts/matrix.sh`, `scripts/linux-harness-smoke.sh` and the launch tests wait for it.
pub const READY_MARK: &str = "Recording in chat mode";

/// The command that restarts a run's agent.
pub const CONTINUE_HINT: &str = "goalcup continue";

/// The only words the participant may type after the one prompt.
pub const KEEP_GOING: &str = "keep going";

/// The first words of the block that says a run goes on after its harness exited.
pub const STILL_OPEN: &str = "Your run is still open until";

/// The first words of the block at the deadline.
pub const TIME_IS_UP: &str = "Time is up.";

/// The heading above the technical lines printed after the harness exits.
pub const DETAILS_HEADING: &str = "Details for support";

/// The end of every block after which the participant need do nothing.
pub const NOTHING_MORE: &str = "Nothing more to do.";

/// The line `goalcup continue` says while the earlier part of the run is sent beside it.
pub const EARLIER_PART_UPLOADING: &str = "The earlier part of this run uploads in the background.";

/// What a refusal of `goalcup continue` on another computer says.
pub const NOT_STARTED_HERE: &str = "was not started on this computer";

/// What a refusal says when the approval code must be asked for again.
pub const RUN_AGAIN: &str = "Run the command again";

/// Where a participant writes when something unexpected happened.
pub const SUPPORT_ADDRESS: &str = "goalcup@agentcommunity.org";

// ------------------------------------------------------------------------------------ times

/// A moment as the participant reads it, in this computer's local time with its zone:
/// `Sat 14 Nov 10:12 CET`. Falls back to UTC when the local zone cannot be read.
pub fn local_clock(unix: u64) -> String {
    match local_offset(unix) {
        Some((offset, zone)) => clock_words(unix, offset, &zone),
        None => clock_words(unix, 0, "UTC"),
    }
}

/// [`local_clock`] with the zone given: `offset_secs` east of UTC, and its name. A name that is
/// empty or not plain text is replaced by the offset (`UTC+05:30`).
pub fn clock_words(unix: u64, offset_secs: i64, zone: &str) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let local = i64::try_from(unix)
        .unwrap_or(i64::MAX)
        .saturating_add(offset_secs)
        .max(0);
    let stamp = goalcup_protocol::control::format_z(local as u64);
    // `YYYY-MM-DDTHH:MM:SSZ`, fixed width.
    let month: usize = stamp[5..7].parse().unwrap_or(1);
    let day: u32 = stamp[8..10].parse().unwrap_or(1);
    // 1970-01-01 was a Thursday.
    let weekday = DAYS[((local / 86_400 + 4).rem_euclid(7)) as usize];
    let zone = if !zone.is_empty() && zone.chars().all(|c| c.is_ascii_graphic()) {
        zone.to_string()
    } else {
        let sign = if offset_secs < 0 { '-' } else { '+' };
        let off = offset_secs.unsigned_abs();
        format!("UTC{sign}{:02}:{:02}", off / 3600, off % 3600 / 60)
    };
    format!(
        "{weekday} {day} {} {} {zone}",
        MONTHS[month.clamp(1, 12) - 1],
        &stamp[11..16]
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

/// A run's end as the participant reads it: `Sat 14 Nov 10:12 CET (95 h 10 min left)`.
/// `deadline_at` is the server's `…Z` stamp; `left` is the server's count of what is left, when
/// there is one. A stamp that does not parse is shown as it is.
pub fn end_words(deadline_at: &str, left: Option<u64>) -> String {
    let at = goalcup_protocol::control::parse_z(deadline_at)
        .map_or_else(|| deadline_at.to_string(), local_clock);
    match left {
        Some(left) => format!("{at} ({})", left_words(left)),
        None => at,
    }
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

// --------------------------------------------------------------------------------- approval

/// What a browser approval is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approving<'a> {
    /// `goalcup login`: this computer, once.
    Login,
    /// One run of `harness` (its display name).
    Run { mode: RunMode, harness: &'a str },
}

/// The lines before the browser opens: which approval this is, and that two are normal.
pub fn approval_intro(what: Approving<'_>) -> Vec<String> {
    match what {
        Approving::Login => vec![
            "Connect this computer to your Goal Cup account.".to_string(),
            "You do this once per computer. Each run asks for its own approval later; that is \
             normal."
                .to_string(),
        ],
        Approving::Run { mode, harness } => vec![
            match mode {
                RunMode::Practice => {
                    format!("Approve this practice run of {harness} in your browser.")
                }
                RunMode::Competition => {
                    format!("Approve your competition run of {harness} in your browser.")
                }
                RunMode::Test => {
                    format!("Approve this organizer test run of {harness} in your browser.")
                }
            },
            "Every run needs its own approval, so this is normal: login connected this computer, \
             and this approval starts this one run."
                .to_string(),
        ],
    }
}

/// The lines that show the code and the address. `opened` is whether a browser program started.
/// The line with the code ends `shows the code XXXX-XXXX.`: `scripts/matrix.sh` reads it.
pub fn approval_code(user_code: &str, address: &str, expires_in_secs: u64) -> Vec<String> {
    vec![
        format!(
            "Your browser opens the approval page now. Check that the page shows the code {user_code}."
        ),
        format!("If no browser opens, open this address: {address}"),
        format!(
            "Waiting for you to approve… (the code expires in {} min; press Ctrl-C to cancel)",
            expires_in_secs.div_ceil(60).max(1)
        ),
    ]
}

/// Said when no browser program started.
pub fn no_browser() -> String {
    "No browser opened on its own. Open the address above yourself.".to_string()
}

/// The end of a login: approved, what it means, and what to do next. `server` is named only when
/// it is not the default one.
pub fn login_done(handle: &str, server: Option<&str>) -> Vec<String> {
    let mut lines = vec![
        format!("Approved. This computer is connected to your Goal Cup account ({handle})."),
        "You do this once per computer. Each run asks for its own approval later. That is normal."
            .to_string(),
    ];
    if let Some(server) = server {
        lines.push(format!("Runs upload to {server}."));
    }
    lines.push("Next: goalcup prep".to_string());
    lines
}

/// Said once a run's approval came back.
pub const APPROVED: &str = "Approved.";

/// Said before the participant's repository is cloned for a new run.
pub const CLONING: &str = "Cloning your repository…";

/// A denied approval.
pub fn approval_denied() -> String {
    format!(
        "{RUN_AGAIN} to start over. The approval was denied in the browser. Nothing was created."
    )
}

/// An approval code that ran out.
pub fn approval_expired() -> String {
    format!(
        "{RUN_AGAIN} for a new code. The code expired before it was approved. Nothing was created."
    )
}

/// Ctrl-C while the approval was waited for.
pub fn approval_interrupted() -> String {
    format!(
        "{RUN_AGAIN} when you are ready. It was interrupted while it waited for your approval. \
         Nothing was created, and no run was opened."
    )
}

/// The server could not be reached to ask for an approval.
pub fn server_unreachable(server: &str, why: &str) -> String {
    format!(
        "Check your connection, then run the command again. The Goal Cup server at {server} \
         could not be reached ({why}). Nothing was created."
    )
}

// ----------------------------------------------------------------------------------- launch

/// The facts the launch block is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch<'a> {
    /// The harness's display name.
    pub harness: &'a str,
    /// The run's mode, for a run on a server; `None` for a run on this machine only.
    pub mode: Option<RunMode>,
    /// A `goalcup continue`: the harness picks up its last session.
    pub continued: bool,
    /// The run's end ([`end_words`]), for a run on a server.
    pub ends: Option<String>,
    /// The manifest's `participant.first_screen`.
    pub first_screen: Option<&'a str>,
}

/// The block printed just before a chat-mode harness takes the terminal. Its last line starts
/// with [`READY_MARK`].
pub fn launch_block(l: &Launch<'_>) -> Vec<String> {
    let h = l.harness;
    let mut lines = vec![String::new()];
    match (l.continued, l.mode) {
        (true, _) => lines.push(
            "Your run goes on. No approval is needed: this computer kept the run's key."
                .to_string(),
        ),
        (false, Some(RunMode::Practice)) => lines
            .push("Your practice run is open. It does not count, and it never pushes.".to_string()),
        (false, Some(RunMode::Competition)) => {
            lines.push("Your competition run is open.".to_string())
        }
        (false, Some(RunMode::Test)) => {
            lines.push("TEST RUN: an organizer test run, not a competition entry.".to_string())
        }
        (false, None) => lines.push("This run is recorded on this computer only.".to_string()),
    }
    if l.continued {
        lines.push(format!(
            "{h} opens now in this terminal and picks up its last session."
        ));
        lines.push(format!(
            "If the agent has stopped, type \"{KEEP_GOING}\" and press Enter. If you never sent \
             your prompt, send it now: it is still your one prompt."
        ));
    } else {
        lines.push(format!("{h} opens now in this terminal."));
        lines.push(format!(
            "Type your one prompt and press Enter. After that you may only type \"{KEEP_GOING}\"."
        ));
    }
    if let Some(ends) = &l.ends {
        lines.push(format!(
            "Your run ends at {ends}. You do not need to stop it."
        ));
    }
    if let Some(first) = l.first_screen {
        lines.push(first.to_string());
    }
    lines.push(format!(
        "goalcup prints nothing here until {h} exits. To see the run, run goalcup status in \
         another terminal."
    ));
    lines.push(format!("{READY_MARK}: {h} opens now."));
    lines
}

// ------------------------------------------------------------------------------- the end

/// The heading above the technical lines printed once the harness has exited.
pub fn details_heading(host_log: Option<&Path>) -> String {
    match host_log {
        Some(log) => format!("{DETAILS_HEADING} (also in {}):", log.display()),
        None => format!("{DETAILS_HEADING}:"),
    }
}

/// Said before the wait for the rest of the upload, which can take minutes.
pub const UPLOADING_REST: &str =
    "Uploading the rest of the run. Keep this terminal open until goalcup says it is done.";

/// How a run on a server came to its end on this computer, for [`ending_block`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending<'a> {
    /// The harness exited, and `goalcup continue` can restart it.
    Open {
        run_id: &'a str,
        /// How the harness exited (`exit code 1`).
        how: &'a str,
        /// The run's end ([`end_words`]).
        until: String,
        /// The tracer stopped the harness because the disk is almost full.
        disk_stop: bool,
    },
    /// The tracer stopped the harness at the run's deadline.
    TimeUp { practice: bool },
    /// A newer run on the account replaced this one.
    Replaced,
    /// The server closed the run (it refused its heartbeat).
    Closed,
    /// The harness exited after the run's time was up.
    Over { how: &'a str },
    /// The harness exited, and it cannot be restarted in the same run (it runs headless).
    NoRestart { how: &'a str, until: String },
}

/// The block printed last, after the details: what happened, and what to do now. `all_sent` is
/// whether everything of this part of the run is on the server, with its seal.
pub fn ending_block(harness: &str, ending: &Ending<'_>, all_sent: bool) -> Vec<String> {
    let h = harness;
    let mut lines = vec![String::new()];
    let unsent =
        "Part of the run is not uploaded yet. Next: when you are online, run: goalcup upload";
    match ending {
        Ending::Open {
            run_id,
            how,
            until,
            disk_stop,
        } => {
            if *disk_stop {
                lines.push(format!(
                    "{h} was stopped: the disk is almost full and uploads are failing."
                ));
            } else {
                lines.push(format!("{h} has exited ({how})."));
            }
            lines.push(format!("{STILL_OPEN} {until}."));
            if !all_sent {
                lines.push(format!(
                    "Part of the run is not uploaded yet; {CONTINUE_HINT} sends it."
                ));
            }
            if *disk_stop {
                lines.push(format!(
                    "To go on: free some disk space, then run: {CONTINUE_HINT} {run_id}"
                ));
            } else {
                lines.push(format!("To go on: {CONTINUE_HINT} {run_id}"));
            }
            lines.push(
                "If your work is done, you need do nothing. The run ends by itself.".to_string(),
            );
        }
        Ending::TimeUp { practice } => {
            if *practice {
                lines.push(format!(
                    "{TIME_IS_UP} Your practice run is over. {h} was stopped."
                ));
            } else {
                lines.push(format!("{TIME_IS_UP} {h} was stopped."));
            }
            if all_sent {
                lines.push(format!("Your run is sealed and uploaded. {NOTHING_MORE}"));
            } else {
                lines.push("Your run is sealed on this computer.".to_string());
                lines.push(unsent.to_string());
            }
        }
        Ending::Replaced => {
            lines.push(format!(
                "This run was replaced by a newer run on your account. {h} was stopped."
            ));
            lines.push("Only the newer run counts.".to_string());
            if all_sent {
                lines.push(NOTHING_MORE.to_string());
            } else {
                lines.push(unsent.to_string());
            }
        }
        Ending::Closed => {
            lines.push(format!("The server closed this run. {h} was stopped."));
            lines.push(format!(
                "{NOTHING_MORE} If you did not expect this, write to {SUPPORT_ADDRESS}."
            ));
        }
        Ending::Over { how } => {
            lines.push(format!(
                "{h} has exited ({how}). Your run is over: its time is up."
            ));
            if all_sent {
                lines.push(NOTHING_MORE.to_string());
            } else {
                lines.push(unsent.to_string());
            }
        }
        Ending::NoRestart { how, until } => {
            lines.push(format!(
                "{h} has exited ({how}). It cannot be restarted in the same run."
            ));
            lines.push(format!("The run ends by itself at {until}."));
            if all_sent {
                lines.push(NOTHING_MORE.to_string());
            } else {
                lines.push(unsent.to_string());
            }
        }
    }
    lines
}

// --------------------------------------------------------------------------------- refusals

/// `goalcup start` while a run of the same harness records on this computer: they share one home.
pub fn home_busy(harness: &str) -> String {
    format!(
        "Exit {harness} in its other terminal first, then run goalcup start again. {harness} is \
         already recording another run on this computer, and only one {harness} run can record \
         at a time. To replace that run, exit it first. Nothing was created."
    )
}

/// `goalcup continue` while a run of the same harness records on this computer.
pub fn still_recording(harness: &str, run_id: &str) -> String {
    format!(
        "Use the terminal where {harness} runs. If it is gone, wait a minute and run \
         {CONTINUE_HINT} again: run {run_id} is still recording on this computer, and only one \
         {harness} run can record at a time. Nothing was changed."
    )
}

/// A chat-mode harness, and no terminal held in the foreground. `then` says what was left.
pub fn needs_terminal(harness: &str, then: &str) -> String {
    format!(
        "Run the command directly in a terminal window: not in the background, and not with its \
         output sent to a file. {harness} runs in this terminal, so goalcup must hold the \
         foreground of a terminal. {then}"
    )
}

/// The run's folder would be made inside the work folder.
pub fn inside_work_folder(work_root: &Path, here: &Path) -> String {
    format!(
        "Run the command from your home folder or another folder of your own, not from inside \
         the work folder {} (you are in {}). A run's own folder must never be inside a clone. \
         Nothing was created.",
        work_root.display(),
        here.display()
    )
}

/// A chat-mode connector given a `--goal-file`.
pub fn takes_no_goal_file(harness: &str, id: &str) -> String {
    format!(
        "Run goalcup start --harness {id}, without --goal-file. {harness} runs in chat mode: it \
         opens in this terminal and you type the prompt there. Nothing was created."
    )
}

/// A headless connector given no `--goal-file`.
pub fn needs_goal_file(harness: &str, id: &str) -> String {
    format!(
        "Run goalcup start --harness {id} --goal-file <file>. {harness} runs headless and needs \
         a goal file. Nothing was created."
    )
}

/// `goalcup continue` of a headless harness's run.
pub fn cannot_continue_headless(harness: &str) -> String {
    format!(
        "Start a new run instead. {harness} runs headless: it was given one goal and has no \
         session to pick up, so its run cannot be continued. Nothing was changed."
    )
}

/// `goalcup continue` with no record of the run on this computer.
pub fn not_started_here(run_id: Option<&str>) -> String {
    let which = run_id.map_or_else(|| "The run".to_string(), |id| format!("Run {id}"));
    format!(
        "Run {CONTINUE_HINT} on the computer that started the run. {which} {NOT_STARTED_HERE}, \
         and a run can be continued only there. Nothing was changed."
    )
}

/// `goalcup continue` of a run that was never opened on the server.
pub fn local_only(run_id: &str) -> String {
    format!(
        "Start a new run with goalcup start. Run {run_id} was recorded on this computer only, \
         never on the Goal Cup server, so it cannot be continued. Nothing was changed."
    )
}

/// `goalcup continue` of a run the server says is over.
pub fn run_over(run_id: &str) -> String {
    format!(
        "Nothing to continue: run {run_id} is over. Its time is up, or a newer run on your \
         account replaced it. Nothing was started."
    )
}

/// `goalcup continue` while the last part's host still holds the run's reservation.
pub fn last_part_closing(run_id: &str) -> String {
    format!(
        "Try again in a minute. The last part of run {run_id} is still being closed. Nothing was \
         changed."
    )
}

/// `goalcup continue` of a run that was continued on another computer.
pub fn continued_elsewhere(run_id: &str, latest: &str, ours: &str) -> String {
    format!(
        "Continue this run only on the computer that started it. The server's latest part of run \
         {run_id} (entry {latest}) is not the one this computer ended last (entry {ours}), so it \
         was continued from somewhere else. Nothing was started."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn last(lines: &[String]) -> &str {
        lines.last().map(String::as_str).unwrap_or("")
    }

    #[test]
    fn a_clock_is_a_weekday_a_date_a_time_and_a_zone() {
        // 2026-11-14T09:12:00Z was a Saturday.
        let unix = goalcup_protocol::control::parse_z("2026-11-14T09:12:00Z").unwrap();
        assert_eq!(clock_words(unix, 3600, "CET"), "Sat 14 Nov 10:12 CET");
        assert_eq!(clock_words(unix, 0, "UTC"), "Sat 14 Nov 09:12 UTC");
        // Over midnight, and a zone with no plain name.
        assert_eq!(
            clock_words(unix, 15 * 3600, ""),
            "Sun 15 Nov 00:12 UTC+15:00"
        );
        assert_eq!(
            clock_words(unix, -(9 * 3600 + 1800), "\u{1b}[31m"),
            "Fri 13 Nov 23:42 UTC-09:30"
        );
        // The local one names the same minute in some zone.
        assert!(local_clock(unix).contains("Nov"), "{}", local_clock(unix));
    }

    #[test]
    fn time_left_is_hours_and_minutes() {
        assert_eq!(left_words(95 * 3600 + 10 * 60 + 59), "95 h 10 min left");
        assert_eq!(left_words(168 * 3600), "168 h 0 min left");
        assert_eq!(left_words(600), "10 min left");
        assert_eq!(left_words(5), "less than a minute left");
        let end = end_words("2026-11-14T09:12:00Z", Some(600));
        assert!(end.ends_with("(10 min left)"), "{end}");
        assert_eq!(end_words("not a time", None), "not a time");
    }

    #[test]
    fn the_time_left_follows_the_servers_deadline_without_a_count() {
        let deadline = goalcup_protocol::control::format_z(1_000_000);
        assert_eq!(seconds_left(None, Some(&deadline), 999_400), Some(600));
        assert_eq!(seconds_left(None, Some(&deadline), 2_000_000), Some(0));
        assert_eq!(seconds_left(None, None, 1), None);
    }

    #[test]
    fn each_approval_says_what_it_is_for_and_that_two_are_normal() {
        let login = approval_intro(Approving::Login).join(" ");
        assert!(login.contains("Connect this computer"), "{login}");
        assert!(login.contains("once per computer"), "{login}");
        for (mode, word) in [
            (RunMode::Practice, "practice run"),
            (RunMode::Competition, "competition run"),
            (RunMode::Test, "test run"),
        ] {
            let run = approval_intro(Approving::Run {
                mode,
                harness: "Some Agent",
            })
            .join(" ");
            assert!(run.contains(word), "{run}");
            assert!(run.contains("Some Agent"), "{run}");
            assert!(run.contains("normal"), "{run}");
        }
        // The matrix reads the code from `shows the code XXXX-XXXX.`
        let code = approval_code("BCDF-GHJK", "https://x.example/a?code=BCDF-GHJK", 900);
        assert!(code[0].ends_with("shows the code BCDF-GHJK."), "{code:?}");
        assert!(code[1].contains("https://x.example/a?code=BCDF-GHJK"));
        assert!(code[2].contains("15 min"), "{code:?}");
        let done = login_done("member-1", None);
        assert!(done.join(" ").contains("member-1"));
        assert!(!done.join(" ").contains("upload to"));
        assert_eq!(last(&done), "Next: goalcup prep");
        assert!(
            login_done("m", Some("https://s.example"))
                .join(" ")
                .contains("https://s.example")
        );
    }

    #[test]
    fn the_launch_block_says_the_prompt_rule_the_end_and_ends_with_the_ready_mark() {
        let ends = end_words("2026-11-14T09:12:00Z", Some(95 * 3600 + 600));
        let block = launch_block(&Launch {
            harness: "Some Agent",
            mode: Some(RunMode::Competition),
            continued: false,
            ends: Some(ends.clone()),
            first_screen: Some("It asks whether you trust this folder. Choose Yes."),
        });
        let text = block.join("\n");
        assert!(
            text.contains("Some Agent opens now in this terminal."),
            "{text}"
        );
        assert!(text.contains("one prompt"), "{text}");
        assert!(text.contains("press Enter"), "{text}");
        assert!(text.contains(KEEP_GOING), "{text}");
        assert!(text.contains(&ends), "{text}");
        assert!(text.contains("95 h 10 min left"), "{text}");
        assert!(text.contains("trust this folder"), "{text}");
        assert!(last(&block).starts_with(READY_MARK), "{text}");
        // The trust hint comes before the ready mark, and nothing internal is said.
        assert!(text.find("trust").unwrap() < text.find(READY_MARK).unwrap());
        for word in ["entry", "segment", "daemon", "host.log", "D57"] {
            assert!(!text.contains(word), "{word}: {text}");
        }

        let continued = launch_block(&Launch {
            harness: "Some Agent",
            mode: Some(RunMode::Competition),
            continued: true,
            ends: Some(ends),
            first_screen: None,
        })
        .join("\n");
        assert!(continued.contains("No approval is needed"), "{continued}");
        assert!(continued.contains(KEEP_GOING), "{continued}");
        assert!(continued.contains("never sent your prompt"), "{continued}");

        let practice = launch_block(&Launch {
            harness: "A",
            mode: Some(RunMode::Practice),
            continued: false,
            ends: None,
            first_screen: None,
        })
        .join("\n");
        assert!(practice.contains("practice run"), "{practice}");
        assert!(!practice.contains("Your run ends at"), "{practice}");
    }

    #[test]
    fn an_exit_while_the_run_is_open_names_the_command_the_run_and_the_end() {
        let until = end_words("2026-11-14T09:12:00Z", Some(3600));
        let block = ending_block(
            "Some Agent",
            &Ending::Open {
                run_id: "r0123456789abcdef",
                how: "exit code 1",
                until: until.clone(),
                disk_stop: false,
            },
            true,
        );
        let text = block.join("\n");
        assert!(
            text.contains("Some Agent has exited (exit code 1)."),
            "{text}"
        );
        assert!(text.contains(&format!("{STILL_OPEN} {until}")), "{text}");
        assert!(
            text.contains(&format!("{CONTINUE_HINT} r0123456789abcdef")),
            "{text}"
        );
        assert!(last(&block).contains("need do nothing"), "{text}");
        assert!(!text.contains("not uploaded"), "{text}");
        let unsent = ending_block(
            "A",
            &Ending::Open {
                run_id: "r1",
                how: "exit code 0",
                until,
                disk_stop: true,
            },
            false,
        )
        .join("\n");
        assert!(unsent.contains("disk"), "{unsent}");
        assert!(unsent.contains("not uploaded"), "{unsent}");
    }

    #[test]
    fn a_run_that_is_over_never_offers_continue_and_ends_with_what_to_do() {
        for ending in [
            Ending::TimeUp { practice: false },
            Ending::TimeUp { practice: true },
            Ending::Replaced,
            Ending::Closed,
            Ending::Over { how: "exit code 0" },
            Ending::NoRestart {
                how: "exit code 0",
                until: "Sat 14 Nov 10:12 CET".into(),
            },
        ] {
            for all_sent in [true, false] {
                let block = ending_block("Some Agent", &ending, all_sent);
                let text = block.join("\n");
                assert!(!text.contains(CONTINUE_HINT), "{ending:?}: {text}");
                assert!(text.contains("Some Agent"), "{ending:?}: {text}");
                let tail = last(&block);
                assert!(
                    tail.contains(NOTHING_MORE) || tail.starts_with("Part of the run"),
                    "{ending:?}: {text}"
                );
                if !all_sent && ending != Ending::Closed {
                    assert!(tail.contains("goalcup upload"), "{ending:?}: {text}");
                }
            }
        }
        let up = ending_block("A", &Ending::TimeUp { practice: false }, true).join("\n");
        assert!(up.contains(TIME_IS_UP), "{up}");
        assert!(up.contains("sealed and uploaded"), "{up}");
    }

    #[test]
    fn every_refusal_says_what_to_do_first_and_has_no_rule_code() {
        let tmp = Path::new("/w");
        let refusals = [
            home_busy("Some Agent"),
            still_recording("Some Agent", "r1"),
            needs_terminal("Some Agent", "Nothing was created."),
            inside_work_folder(tmp, &tmp.join("x")),
            takes_no_goal_file("Some Agent", "some-cli"),
            needs_goal_file("Some Agent", "some-cli"),
            cannot_continue_headless("Some Agent"),
            not_started_here(Some("r1")),
            not_started_here(None),
            local_only("r1"),
            run_over("r1"),
            last_part_closing("r1"),
            continued_elsewhere("r1", "e2", "e1"),
            approval_denied(),
            approval_expired(),
            approval_interrupted(),
            server_unreachable("https://s.example", "refused"),
        ];
        let verbs = [
            "Exit", "Use", "Run", "Start", "Nothing", "Try", "Continue", "Check",
        ];
        for refusal in refusals {
            let first = refusal.split(' ').next().unwrap();
            assert!(verbs.contains(&first), "{refusal}");
            assert!(
                !refusal
                    .split(|c: char| !c.is_ascii_alphanumeric())
                    .any(|w| w.len() >= 2
                        && (w.starts_with('R') || w.starts_with('D'))
                        && w[1..].chars().all(|c| c.is_ascii_digit())),
                "a rule code: {refusal}"
            );
            assert!(refusal.ends_with('.'), "{refusal}");
        }
        assert!(still_recording("A", "r1").contains("run r1 is still recording"));
        assert!(home_busy("A").contains("already recording"));
        assert!(needs_terminal("A", "").contains("foreground of a terminal"));
        assert!(not_started_here(None).contains(NOT_STARTED_HERE));
    }
}
