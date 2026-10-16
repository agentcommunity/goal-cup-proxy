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

/// The lines that show the code and the address, once the browser was asked to open it:
/// `opened` is whether a browser program started. The line with the code ends
/// `shows the code XXXX-XXXX.`: `scripts/matrix.sh` reads it.
pub fn approval_code(
    user_code: &str,
    address: &str,
    opened: bool,
    expires_in_secs: u64,
) -> Vec<String> {
    let mut lines = if opened {
        vec![
            format!(
                "Your browser opened the approval page. Check that the page shows the code {user_code}."
            ),
            format!("If you do not see it, open this address: {address}"),
        ]
    } else {
        vec![format!(
            "Open this address in your browser: {address} and check that the page shows the code {user_code}."
        )]
    };
    lines.push(format!(
        "Waiting for you to approve… (the code expires in {} min; press Ctrl-C to cancel)",
        expires_in_secs.div_ceil(60).max(1)
    ));
    lines
}

/// Said before the browser opens for a competition run while another competition run of the
/// account is open on this computer: approving replaces it.
pub fn replaces_open_run(started: &str) -> String {
    format!(
        "You already have a competition run open (started {started}). Approving replaces it: \
         that run stops, and only the new run counts."
    )
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
    /// A first launch, or a `goalcup continue` and whether it picks up a session.
    pub opens: Opens,
    /// The run's end ([`end_words`]), for a run on a server.
    pub ends: Option<String>,
    /// The manifest's `participant.first_screen`.
    pub first_screen: Option<&'a str>,
}

/// How the harness opens for one part of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opens {
    /// The run's first launch.
    First,
    /// A `goalcup continue`: the harness picks up its last session.
    LastSession,
    /// A `goalcup continue` of a run where no saved session was found (the harness closed before
    /// the first message, or its home is made fresh for each part): the harness opens the normal
    /// way, with a new session.
    NewSession,
}

/// The first line of a continued run's launch block.
pub const RUN_GOES_ON: &str =
    "Your run goes on. No approval is needed: this computer kept the approval for this run.";

/// The block printed just before a chat-mode harness takes the terminal. Its last line starts
/// with [`READY_MARK`].
pub fn launch_block(l: &Launch<'_>) -> Vec<String> {
    let h = l.harness;
    let continued = l.opens != Opens::First;
    let mut lines = vec![String::new()];
    match (continued, l.mode) {
        (true, _) => lines.push(RUN_GOES_ON.to_string()),
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
    lines.push(match l.opens {
        Opens::First => format!("{h} opens now in this terminal."),
        Opens::LastSession => {
            format!("{h} opens now in this terminal and picks up its last session.")
        }
        Opens::NewSession => format!(
            "{h} opens now in this terminal with a new session: no saved session was found \
             for this run."
        ),
    });
    // What the harness shows first comes before what to type: the participant meets it first.
    if let Some(first) = l.first_screen {
        lines.push(first.to_string());
    }
    if l.opens == Opens::LastSession {
        lines.push(format!(
            "If the agent has stopped, type \"{KEEP_GOING}\" and press Enter. If you never sent \
             your prompt, send it now: it is still your one prompt."
        ));
    } else {
        lines.push(format!(
            "Type your one prompt and press Enter. After that you may only type \"{KEEP_GOING}\", \
             when the agent has stopped."
        ));
    }
    if let Some(ends) = &l.ends {
        lines.push(format!(
            "Your run ends at {ends}. You do not need to stop it."
        ));
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
    "Uploading the rest of the run… keep this terminal open until it says the upload is done.";

/// Said once the rest of the run is uploaded, with its seal.
pub const UPLOAD_DONE: &str = "Upload done.";

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

/// The block printed at the seal, before the wrap-up upload: what happened, and, for a run that
/// goes on, what to do now. `goalcup continue` works from this moment (the home and the run's
/// reserve are let go at the seal), so it is said before the wait, not after it.
pub fn ending_block(harness: &str, ending: &Ending<'_>) -> Vec<String> {
    let h = harness;
    let mut lines = vec![String::new()];
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
            if *disk_stop {
                lines.push(format!(
                    "To go on: free some disk space, then run: {CONTINUE_HINT} {run_id} (you can \
                     do it in another terminal now)."
                ));
            } else {
                lines.push(format!(
                    "To go on, run: {CONTINUE_HINT} {run_id} (you can do it in another terminal \
                     now)."
                ));
            }
            lines.push(
                "If your work is done, you need do nothing. The run ends by itself.".to_string(),
            );
        }
        Ending::TimeUp { practice: true } => lines.push(format!(
            "{TIME_IS_UP} Your practice run is over. {h} was stopped."
        )),
        Ending::TimeUp { practice: false } => lines.push(format!("{TIME_IS_UP} {h} was stopped.")),
        Ending::Replaced => {
            lines.push(format!(
                "This run was replaced by a newer run on your account. {h} was stopped."
            ));
            lines.push("Only the newer run counts.".to_string());
        }
        Ending::Closed => lines.push(format!("The server closed this run. {h} was stopped.")),
        Ending::Over { how } => lines.push(format!(
            "{h} has exited ({how}). Your run is over: its time is up."
        )),
        Ending::NoRestart { how, until } => {
            lines.push(format!(
                "{h} has exited ({how}). It cannot be restarted in the same run."
            ));
            lines.push(format!("The run ends by itself at {until}."));
        }
    }
    lines
}

/// The last lines, once the wrap-up upload is over. `all_sent`: everything of this part of the
/// run is on the server, with its seal. `server_closed`: the server said, while it took the upload,
/// that the run is sealed or incomplete, so a run that was to go on cannot be continued.
pub fn after_upload(ending: &Ending<'_>, all_sent: bool, server_closed: bool) -> Vec<String> {
    let mut lines = Vec::new();
    let unsent =
        "Part of the run is not uploaded yet. Next: when you are online, run: goalcup upload";
    match ending {
        Ending::Open { .. } if server_closed => {
            lines.push("The server has closed this run; it cannot be continued.".to_string());
            lines.push(if all_sent {
                format!("{UPLOAD_DONE} {NOTHING_MORE}")
            } else {
                unsent.to_string()
            });
        }
        Ending::Open { .. } => lines.push(if all_sent {
            format!("{UPLOAD_DONE} Nothing more to do in this terminal.")
        } else {
            format!("Part of the run is not uploaded yet; {CONTINUE_HINT} sends it.")
        }),
        Ending::TimeUp { practice } => {
            if all_sent {
                // The prep pointer is a next step, so it replaces "Nothing more to do": the two
                // would contradict each other. It is given only when the upload is complete,
                // because prep's practice step needs every part uploaded.
                if *practice {
                    lines.push("Your practice run is sealed and uploaded.".to_string());
                    lines.push(
                        "Next: goalcup prep (its practice step should now be ok).".to_string(),
                    );
                } else {
                    lines.push(format!("Your run is sealed and uploaded. {NOTHING_MORE}"));
                }
            } else {
                lines.push("Your run is sealed on this computer.".to_string());
                lines.push(unsent.to_string());
            }
        }
        Ending::Closed => lines.push(format!(
            "{NOTHING_MORE} If you did not expect this, write to {SUPPORT_ADDRESS}."
        )),
        Ending::Replaced | Ending::Over { .. } | Ending::NoRestart { .. } => {
            lines.push(if all_sent {
                format!("{UPLOAD_DONE} {NOTHING_MORE}")
            } else {
                unsent.to_string()
            })
        }
    }
    lines
}

// ---------------------------------------------------------------------------------- status

/// The heading above the technical block `goalcup status` keeps below its participant block.
pub const STATUS_DETAILS: &str = "Details:";

/// The prefix of every problem line. Scripts and the guide read it.
pub const PROBLEM: &str = "PROBLEM:";

/// How far a run has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The agent runs and the run records.
    Recording,
    /// The agent has exited; the run is still open.
    AgentExited,
    /// The run's time is up and goalcup is finishing it.
    Finishing,
    /// The run is sealed; part of it is not uploaded.
    Sealed,
    /// The run is sealed and all of it is uploaded.
    Uploaded,
}

/// What the agent's process is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Agent<'a> {
    Running,
    /// It ended by itself; `how` is `exit code 1`.
    Exited {
        how: &'a str,
    },
    /// goalcup ended it because the run is over.
    Stopped,
    /// goalcup ended it because the disk is almost full.
    StoppedForDisk,
}

/// How the upload stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadNow {
    /// The run is recorded on this computer only.
    NotUsed,
    /// Nothing has been sent yet.
    NotYet,
    /// It works; `ago` is the seconds since the last send.
    UpToDate { ago: Option<u64> },
    /// It fails now; goalcup keeps trying.
    Retrying { ago: Option<u64> },
    /// goalcup gave up on it.
    GaveUp,
    /// The run is sealed and not all of it is sent.
    Unfinished,
    /// All of it is sent.
    Done,
}

/// The facts of the first lines of `goalcup status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusHeader<'a> {
    /// The harness's display name.
    pub harness: &'a str,
    /// The run's mode; `None` for a run recorded on this computer only.
    pub mode: Option<RunMode>,
    /// `None`: this computer has no state for the run yet.
    pub phase: Option<Phase>,
    /// The run's end ([`ends_line`]), for a run on a server.
    pub ends: Option<String>,
    pub agent: Agent<'a>,
    pub upload: UploadNow,
    /// How many problem lines follow.
    pub problems: usize,
    /// Nothing has written the run's state for a while: the state below may be old.
    pub stale: bool,
    /// The run ended before its time (a newer run replaced it, or the server closed it).
    pub early: bool,
    /// With no state: the run started a moment ago, so its state is still to come.
    pub young: bool,
}

/// How a run's end is said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// The run is open; the time left is said.
    Open,
    /// Its time is up.
    Over,
    /// It was closed before its time.
    Early,
}

/// The line that says when the run ends (`Ends: Sat 14 Nov, 10:12 CET (09:12 UTC), 95 h 10 min
/// left.`), or when it ended. `left` is what is left, for a run that is open.
pub fn ends_line(deadline_at: &str, left: Option<u64>, end: End) -> String {
    match end {
        End::Open => format!("Ends: {}.", end_words(deadline_at, left)),
        End::Over => format!("Ended: {}.", end_words(deadline_at, None)),
        End::Early => "Ended early: a newer run replaced it, or the server closed it.".to_string(),
    }
}

fn run_kind(mode: Option<RunMode>) -> &'static str {
    match mode {
        Some(RunMode::Practice) => "practice run",
        Some(RunMode::Competition) => "competition run",
        Some(RunMode::Test) => "organizer test run",
        None => "run (recorded on this computer only)",
    }
}

/// The first lines of `goalcup status`: the run, its state, its end, the agent, the upload and the
/// number of problems.
pub fn status_header(h: &StatusHeader<'_>) -> Vec<String> {
    let name = h.harness;
    let state = match (h.phase, h.mode) {
        (None, _) if h.young => "no run state yet".to_string(),
        (None, _) => "no run state, so it did not start".to_string(),
        (Some(Phase::Recording), _) if h.stale => "state not updated lately".to_string(),
        (Some(Phase::Recording), _) => "recording".to_string(),
        (Some(Phase::AgentExited), _) => format!("{name} has exited, the run is still open"),
        (Some(Phase::Finishing), _) if h.stale && h.early => {
            "the run has ended, goalcup stopped before it finished the run".to_string()
        }
        (Some(Phase::Finishing), _) if h.stale => {
            "time is up, goalcup stopped before it finished the run".to_string()
        }
        (Some(Phase::Finishing), _) if h.early => {
            "the run has ended, goalcup is finishing the run".to_string()
        }
        (Some(Phase::Finishing), _) => "time is up, goalcup is finishing the run".to_string(),
        (Some(Phase::Sealed), None) => "sealed on this computer".to_string(),
        (Some(Phase::Sealed), Some(_)) => "sealed, not all of it is uploaded yet".to_string(),
        (Some(Phase::Uploaded), _) => "sealed and uploaded".to_string(),
    };
    let mut lines = vec![format!("Your {} with {name}: {state}.", run_kind(h.mode))];
    if let Some(ends) = &h.ends {
        lines.push(format!("  {ends}"));
    }
    if h.phase.is_none() {
        lines.push(if h.young {
            "  goalcup writes the run state a few seconds after the run starts.".to_string()
        } else {
            "  goalcup wrote no state for this run, so the start did not finish.".to_string()
        });
        return lines;
    }
    let state_old = h.stale && matches!(h.phase, Some(Phase::Recording | Phase::Finishing));
    lines.push(match &h.agent {
        _ if state_old => format!("  {name}: not known, the state is not being updated."),
        Agent::Running => format!("  {name}: running."),
        Agent::Exited { how } => format!("  {name}: exited ({how})."),
        Agent::Stopped => format!("  {name}: stopped."),
        Agent::StoppedForDisk => format!("  {name}: stopped (the disk is almost full)."),
    });
    lines.push(format!(
        "  Upload: {}.",
        match h.upload {
            UploadNow::NotUsed => "not used, this run stays on this computer".to_string(),
            UploadNow::NotYet => "nothing sent yet".to_string(),
            UploadNow::UpToDate { ago: Some(ago) } =>
                format!("up to date (last sent {} ago)", crate::run::age_words(ago)),
            UploadNow::UpToDate { ago: None } => "up to date".to_string(),
            UploadNow::Retrying { ago: Some(ago) } => format!(
                "not up to date, goalcup keeps trying (last sent {} ago)",
                crate::run::age_words(ago)
            ),
            UploadNow::Retrying { ago: None } => "not up to date, goalcup keeps trying".to_string(),
            UploadNow::GaveUp => "stopped, see the problem below".to_string(),
            UploadNow::Unfinished =>
                "not finished, part of the run is only on this computer".to_string(),
            UploadNow::Done => "done".to_string(),
        }
    ));
    lines.push(match h.problems {
        0 => "  Problems: none.".to_string(),
        1 => "  Problems: 1, below.".to_string(),
        n => format!("  Problems: {n}, below."),
    });
    lines
}

/// What `goalcup status` tells the participant to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next<'a> {
    /// The run records and nothing is wrong.
    NothingEnds,
    /// The run is done, or nothing can be done for it.
    Nothing,
    /// The agent can be started again.
    Continue { run_id: &'a str },
    /// Part of the run is not uploaded.
    Upload,
    /// The problem lines say what to do; the run goes on meanwhile.
    Problems,
    /// goalcup is still at work.
    Wait,
    /// goalcup stopped before it ended the run: seal it, then send it (`upload`: a run on a
    /// server).
    Seal { upload: bool },
    /// The run never started; start a new one.
    Restart {
        connector_id: &'a str,
        mode: Option<RunMode>,
    },
}

/// The last line of the participant block of `goalcup status`.
pub fn status_next(next: &Next<'_>) -> String {
    match next {
        Next::NothingEnds => "Next: nothing to do. The run ends by itself.".to_string(),
        Next::Nothing => "Next: nothing to do.".to_string(),
        // A command ends its line with no full stop: it is copied as it stands.
        Next::Continue { run_id } => format!("Next: run {CONTINUE_HINT} {run_id}"),
        Next::Upload => "Next: run goalcup upload".to_string(),
        Next::Seal { upload: true } => "Next: run goalcup seal, then goalcup upload".to_string(),
        Next::Seal { upload: false } => "Next: run goalcup seal".to_string(),
        Next::Restart { connector_id, mode } => {
            let flag = match mode {
                Some(RunMode::Practice) => " --practice",
                Some(RunMode::Test) => " --test",
                Some(RunMode::Competition) | None => "",
            };
            format!(
                "Next: this run did not start. Run goalcup start --harness {connector_id}{flag}"
            )
        }
        Next::Problems => {
            "Next: do what the problem lines above say. The run goes on meanwhile.".to_string()
        }
        Next::Wait => "Next: wait a minute, then run goalcup status again.".to_string(),
    }
}

/// `goalcup status` on a computer that has not started a run. The command comes first; the phrase
/// "no run has been started from this machine" is read by tests.
pub fn no_run_yet() -> String {
    "Run goalcup prep, then goalcup start --harness <id> --practice: no run has been started from \
     this machine yet. Nothing to show."
        .to_string()
}

/// What a problem line says about restarting the agent: the fix, or the fact that there is none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart<'a> {
    /// `goalcup continue` can restart it.
    Continue { run_id: &'a str },
    /// The run is over.
    Over,
    /// The run is open but cannot be continued.
    Cannot,
}

/// The run's facts that a problem line names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Who<'a> {
    pub harness: &'a str,
    pub run_id: &'a str,
    /// The run's end ([`end_words`]), when it has one.
    pub until: Option<&'a str>,
}

/// goalcup has not written the run's state for `silent` (words).
pub fn problem_state_stale(silent: &str, restart: Restart<'_>) -> String {
    let fix = match restart {
        Restart::Continue { run_id } => format!("To go on, run: {CONTINUE_HINT} {run_id}"),
        Restart::Over => "The run is over, so it cannot be continued. To finish it, run: goalcup seal, then goalcup upload"
            .to_string(),
        Restart::Cannot => "This run cannot be continued.".to_string(),
    };
    format!(
        "goalcup has not updated this run for {silent}. The computer may have slept or restarted. \
         {fix}"
    )
}

pub fn problem_recording_gaps() -> String {
    format!(
        "the recording has gaps: part of what happened was not recorded. Keep the run going. If \
         this keeps happening, write to {SUPPORT_ADDRESS}."
    )
}

pub fn problem_recording_failed(run_id: &str) -> String {
    format!("the recording failed. Write to {SUPPORT_ADDRESS} and name run {run_id}.")
}

/// The upload is behind. `over`: the run is sealed or the upload gave up, so the fix is the
/// command; otherwise goalcup is still trying.
pub fn problem_upload(ago: &str, why: &str, over: bool) -> String {
    if over {
        format!(
            "part of the run is not uploaded (last upload {ago} ago: {why}). Nothing is lost; it \
             waits on this computer. Check the network, then run: goalcup upload"
        )
    } else {
        format!(
            "nothing was uploaded for {ago} ({why}). Nothing is lost; it waits on this computer. \
             Check the network."
        )
    }
}

/// `failures` heartbeats in a row failed; one is sent every [`Policy::heartbeat`] (five minutes).
///
/// [`Policy::heartbeat`]: crate::upload::policy::Policy
pub fn problem_heartbeats(failures: u32, why: &str) -> String {
    let cadence_min = crate::upload::policy::Policy::default().heartbeat.as_secs() / 60;
    format!(
        "the server has not heard from this run for about {} min ({why}). Check the network.",
        u64::from(failures) * cadence_min
    )
}

pub fn problem_disk(free: &str) -> String {
    format!("only {free} of disk is free. Free some space.")
}

pub fn problem_sleep(why: &str) -> String {
    format!("this computer may go to sleep ({why}). Keep it on mains power with the lid open.")
}

/// The agent ended by itself, or the disk stopped it, and the run is not over.
pub fn problem_agent_exited(
    who: &Who<'_>,
    how: &str,
    disk_stop: bool,
    restart: Restart<'_>,
) -> String {
    let h = who.harness;
    let until = who
        .until
        .map(|until| format!(" {STILL_OPEN} {until}."))
        .unwrap_or_default();
    match restart {
        Restart::Continue { run_id } if disk_stop => format!(
            "{h} was stopped: the disk is almost full and uploads are failing.{until} Free some \
             disk space, then run: {CONTINUE_HINT} {run_id}"
        ),
        Restart::Continue { run_id } => {
            format!("{h} has exited ({how}).{until} To go on, run: {CONTINUE_HINT} {run_id}")
        }
        Restart::Over => {
            format!("{h} has exited ({how}) and the run is over. Nothing to continue.")
        }
        Restart::Cannot => format!(
            "{h} has exited ({how}) and this run cannot be continued. If you did not expect \
             this, write to {SUPPORT_ADDRESS}."
        ),
    }
}

/// The agent finished normally and the run cannot be continued: a normal end, not a problem.
pub fn agent_finished(harness: &str, over: bool) -> String {
    if over {
        format!("{harness} finished, and the run is over.")
    } else {
        format!("{harness} finished, and this run cannot be continued.")
    }
}

// ------------------------------------------------------------------------ upload and entries

/// `goalcup upload` when there is no run on this computer.
pub fn upload_no_run() -> String {
    "Run goalcup start first: no run has been started from this machine, so there is nothing to \
     upload. To send a run by name: goalcup upload <entry>."
        .to_string()
}

/// `goalcup upload` when the newest run was never opened on the server.
pub fn upload_local_run(run_id: &str) -> String {
    format!(
        "Name another run: goalcup upload <entry>. Nothing to upload: the newest run ({run_id}) \
         is local, so it was recorded on this computer only and never opened on the server."
    )
}

/// `goalcup upload` when the newest run is all on the server.
pub fn upload_nothing_left(run_id: &str) -> String {
    format!("Nothing left to send: run {run_id} is fully uploaded. {NOTHING_MORE}")
}

/// An entry id that this computer did not open on a server.
pub fn upload_not_opened(entry_id: &str) -> String {
    format!(
        "Name a run that was started on this computer: goalcup upload <entry>. Part {entry_id} \
         was never opened on the server from here, so there is nothing to upload."
    )
}

/// The run's credential is not on this computer.
pub fn upload_not_here(entry_id: &str) -> String {
    format!(
        "Run goalcup upload on the computer that started the run: this run was not started on \
         this machine (part {entry_id} has no saved credential here), and only the machine that \
         started a run can upload it."
    )
}

/// The server is away; goalcup waits `wait_secs` and asks again.
pub fn upload_waiting(why: &str, wait_secs: u64) -> String {
    format!(
        "Waiting for the server ({why}). Trying again in {wait_secs}s. Keep this terminal open, \
         or press Ctrl-C and run goalcup upload later: it carries on from where it stopped."
    )
}

/// The server refused the upload for good.
pub fn upload_refused(server: &str, entry_id: &str, why: &str) -> String {
    format!(
        "Run goalcup upload again. If it is refused again, write to {SUPPORT_ADDRESS}. The server \
         at {server} refused part {entry_id} of your run: {why}"
    )
}

/// Part of the run did not get through.
pub fn upload_incomplete(entry_id: &str, bytes: u64, why: &str) -> String {
    format!(
        "Check the network, then run goalcup upload again. Part {entry_id} of your run is not \
         fully uploaded ({bytes} bytes are): {why}. Nothing is lost; it waits on this computer."
    )
}

/// The upload finished, and the run is sealed.
pub fn upload_finished() -> String {
    format!("Uploaded the rest of your run. {NOTHING_MORE}")
}

/// The upload finished, and the run is not sealed yet.
pub fn upload_not_sealed() -> String {
    "Run goalcup upload again once the run has ended. What exists so far is uploaded; the run is \
     not sealed yet."
        .to_string()
}

/// The first lines of `goalcup entries`.
pub fn entries_intro(none: bool) -> Vec<String> {
    if none {
        vec![
            "To start a run: goalcup prep, then goalcup start --harness <id> --practice."
                .to_string(),
            "To show the newest run you started here, even a finished one: goalcup status"
                .to_string(),
            String::new(),
            "Runs on this computer:".to_string(),
        ]
    } else {
        vec![
            "To show one run in full, run: goalcup status <id>".to_string(),
            String::new(),
            "Runs on this computer (id, harness, state, folder):".to_string(),
        ]
    }
}

// --------------------------------------------------------------------------------- refusals

/// `goalcup start` while a run of the same harness records on this computer: they share one home.
///
/// Most often this is a second start by mistake, so the first words say that nothing is needed;
/// replacing the run is a choice, said last, never the first thing to do.
///
/// `replaces_competition_run`: this start is a competition start and a competition run of this
/// account is open on this computer, so a new start would replace it. A practice or test start
/// never replaces a competition run, and does not say so.
pub fn home_busy(harness: &str, replaces_competition_run: bool) -> String {
    let replaces = if replaces_competition_run {
        " (this replaces your current competition run)"
    } else {
        ""
    };
    format!(
        "Your run is already going in another terminal. Nothing to do here: {harness} is already \
         recording on this computer. To see it: goalcup status. To start a new run instead: exit \
         {harness} there first, then run goalcup start again{replaces}. Nothing was created."
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
         the work folder {} (the run's own folder would be made in {}). A run's own folder must \
         never be inside a clone. Nothing was created.",
        work_root.display(),
        here.display()
    )
}

/// The model service the harness's own config names is one Goal Cup cannot record (D62).
/// `why` is [`crate::model_host::Unrecordable::sentence`]; `id` is the connector's.
pub fn cannot_record_model_service(why: &str, id: &str) -> String {
    format!(
        "{why} Goal Cup records only an https model service that is not on this computer. Run \
         goalcup prep --harness {id}, choose such a service there, then run the command again."
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
        // Local time with its zone, and UTC in brackets.
        assert_eq!(
            clock_words(unix, 3600, "CET"),
            "Sat 14 Nov, 10:12 CET (09:12 UTC)"
        );
        // A computer on UTC says it once.
        assert_eq!(clock_words(unix, 0, "UTC"), "Sat 14 Nov, 09:12 UTC");
        // Over midnight the UTC date is named; a zone with no plain name is its offset (macOS
        // names some zones `+04`).
        assert_eq!(
            clock_words(unix, 15 * 3600, "+15"),
            "Sun 15 Nov, 00:12 UTC+15:00 (Sat 14 Nov, 09:12 UTC)"
        );
        assert_eq!(
            clock_words(unix, -(9 * 3600 + 1800), "\u{1b}[31m"),
            "Fri 13 Nov, 23:42 UTC-09:30 (Sat 14 Nov, 09:12 UTC)"
        );
        // The local one always carries the UTC minute.
        assert!(
            local_clock(unix).contains("09:12 UTC"),
            "{}",
            local_clock(unix)
        );
    }

    #[test]
    fn time_left_is_hours_and_minutes() {
        assert_eq!(left_words(95 * 3600 + 10 * 60 + 59), "95 h 10 min left");
        assert_eq!(left_words(168 * 3600), "168 h 0 min left");
        assert_eq!(left_words(600), "10 min left");
        assert_eq!(left_words(5), "less than a minute left");
        let end = end_words("2026-11-14T09:12:00Z", Some(600));
        assert!(end.ends_with(", 10 min left"), "{end}");
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
        let address = "https://x.example/a?code=BCDF-GHJK";
        let code = approval_code("BCDF-GHJK", address, true, 900);
        assert!(code[0].contains("opened"), "{code:?}");
        assert!(code[0].ends_with("shows the code BCDF-GHJK."), "{code:?}");
        assert!(code[1].contains(address));
        assert!(code[2].contains("15 min"), "{code:?}");
        // No browser: the address and the code in one line, and nothing says a browser opened.
        let code = approval_code("BCDF-GHJK", address, false, 900);
        assert!(code[0].contains(address), "{code:?}");
        assert!(code[0].ends_with("shows the code BCDF-GHJK."), "{code:?}");
        assert!(!code.join(" ").contains("opened"), "{code:?}");
        assert!(replaces_open_run("Fri 7 Nov").contains("replaces"));
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
            opens: Opens::First,
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
        // The trust hint comes before what to type (it is the first screen), and nothing
        // internal is said.
        assert!(text.find("trust").unwrap() < text.find("one prompt").unwrap());
        for word in ["entry", "segment", "daemon", "host.log", "D57"] {
            assert!(!text.contains(word), "{word}: {text}");
        }

        let continued = launch_block(&Launch {
            harness: "Some Agent",
            mode: Some(RunMode::Competition),
            opens: Opens::LastSession,
            ends: Some(ends.clone()),
            first_screen: None,
        })
        .join("\n");
        assert!(continued.contains(RUN_GOES_ON), "{continued}");
        assert!(
            continued.contains("picks up its last session"),
            "{continued}"
        );
        assert!(continued.contains(KEEP_GOING), "{continued}");
        assert!(continued.contains("never sent your prompt"), "{continued}");
        // What this computer kept is the approval, never a "key" a reader takes for an API key.
        assert!(!continued.contains("key"), "{continued}");

        // A continue of a run with no session promises none: it says the session is new, in
        // one line, and asks for the one prompt as a first launch does.
        let fresh = launch_block(&Launch {
            harness: "Some Agent",
            mode: Some(RunMode::Competition),
            opens: Opens::NewSession,
            ends: Some(ends),
            first_screen: None,
        });
        let text = fresh.join("\n");
        assert!(text.contains(RUN_GOES_ON), "{text}");
        assert!(!text.contains("picks up"), "{text}");
        assert_eq!(
            fresh.iter().filter(|l| l.contains("new session")).count(),
            1,
            "{text}"
        );
        assert!(
            text.contains(
                "Some Agent opens now in this terminal with a new session: no saved session \
                 was found for this run."
            ),
            "{text}"
        );
        assert!(
            text.contains("Type your one prompt and press Enter"),
            "{text}"
        );

        let practice = launch_block(&Launch {
            harness: "A",
            mode: Some(RunMode::Practice),
            opens: Opens::First,
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
        let open = Ending::Open {
            run_id: "r0123456789abcdef",
            how: "exit code 1",
            until: until.clone(),
            disk_stop: false,
        };
        let block = ending_block("Some Agent", &open);
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
        assert!(text.contains("another terminal now"), "{text}");
        assert!(last(&block).contains("need do nothing"), "{text}");
        // After the upload: done, or what sends the rest; or the server closed it meanwhile.
        assert!(last(&after_upload(&open, true, false)).starts_with(UPLOAD_DONE));
        assert!(
            after_upload(&open, false, false)
                .join(" ")
                .contains(CONTINUE_HINT)
        );
        let closed = after_upload(&open, true, true).join(" ");
        assert!(closed.contains("cannot be continued"), "{closed}");
        let disk = ending_block(
            "A",
            &Ending::Open {
                run_id: "r1",
                how: "exit code 0",
                until,
                disk_stop: true,
            },
        )
        .join("\n");
        assert!(disk.contains("disk"), "{disk}");
        assert!(disk.contains(&format!("{CONTINUE_HINT} r1")), "{disk}");
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
                until: "Sat 14 Nov, 10:12 CET (09:12 UTC)".into(),
            },
        ] {
            for all_sent in [true, false] {
                let head = ending_block("Some Agent", &ending);
                let tail = after_upload(&ending, all_sent, false);
                let text = head
                    .iter()
                    .chain(&tail)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(!text.contains(CONTINUE_HINT), "{ending:?}: {text}");
                assert!(head.join(" ").contains("Some Agent"), "{ending:?}: {text}");
                let last_line = last(&tail);
                assert!(
                    last_line.contains(NOTHING_MORE)
                        || last_line.starts_with("Part of the run")
                        || last_line.starts_with("Next: goalcup prep"),
                    "{ending:?}: {text}"
                );
                if !all_sent && ending != Ending::Closed {
                    assert!(text.contains("goalcup upload"), "{ending:?}: {text}");
                }
            }
        }
        let up = ending_block("A", &Ending::TimeUp { practice: false }).join("\n");
        assert!(up.contains(TIME_IS_UP), "{up}");
        let sent = after_upload(&Ending::TimeUp { practice: false }, true, false).join("\n");
        assert!(sent.contains("sealed and uploaded"), "{sent}");
        // A practice run that is all uploaded points to prep, and does not also say there is
        // nothing more to do.
        let practice = after_upload(&Ending::TimeUp { practice: true }, true, false);
        assert!(
            last(&practice).starts_with("Next: goalcup prep"),
            "{practice:?}"
        );
        assert!(!practice.join(" ").contains(NOTHING_MORE), "{practice:?}");
        // With part of it unsent the prep step is not ok yet: no pointer to it, the upload
        // command instead.
        let unsent = after_upload(&Ending::TimeUp { practice: true }, false, false);
        let text = unsent.join(" ");
        assert!(!text.contains("goalcup prep"), "{text}");
        assert!(text.contains("goalcup upload"), "{text}");
    }

    fn header(phase: Option<Phase>, mode: Option<RunMode>) -> StatusHeader<'static> {
        StatusHeader {
            harness: "Some Agent",
            mode,
            phase,
            ends: Some(ends_line(
                "2026-11-14T09:12:00Z",
                Some(95 * 3600 + 600),
                End::Open,
            )),
            agent: Agent::Running,
            upload: UploadNow::UpToDate { ago: Some(120) },
            problems: 0,
            stale: false,
            early: false,
            young: true,
        }
    }

    #[test]
    fn the_status_header_names_the_run_the_state_the_end_the_agent_and_the_upload() {
        let text =
            status_header(&header(Some(Phase::Recording), Some(RunMode::Competition))).join("\n");
        assert!(
            text.starts_with("Your competition run with Some Agent: recording."),
            "{text}"
        );
        assert!(text.contains("95 h 10 min left"), "{text}");
        assert!(text.contains("09:12 UTC"), "{text}");
        assert!(text.contains("Some Agent: running."), "{text}");
        assert!(text.contains("up to date (last sent 2 min ago)"), "{text}");
        assert!(text.contains("Problems: none."), "{text}");

        for (mode, word) in [
            (Some(RunMode::Practice), "practice run"),
            (Some(RunMode::Test), "organizer test run"),
            (None, "this computer only"),
        ] {
            let text = status_header(&header(Some(Phase::Recording), mode)).join("\n");
            assert!(text.contains(word), "{word}: {text}");
        }
        // Each state is in the first line.
        for (phase, word) in [
            (Phase::AgentExited, "has exited, the run is still open"),
            (Phase::Finishing, "time is up"),
            (Phase::Sealed, "sealed"),
            (Phase::Uploaded, "sealed and uploaded"),
        ] {
            let first = status_header(&header(Some(phase), Some(RunMode::Competition))).remove(0);
            assert!(first.contains(word), "{word}: {first}");
        }
        // A run over says when it ended and has no time left.
        let over = ends_line("2026-11-14T09:12:00Z", Some(5), End::Over);
        assert!(over.starts_with("Ended: "), "{over}");
        assert!(!over.contains("left"), "{over}");
        // A run closed before its time names no deadline as its end.
        let early = ends_line("2026-11-14T09:12:00Z", Some(5), End::Early);
        assert!(early.starts_with("Ended early"), "{early}");
        assert!(!early.contains("09:12"), "{early}");
        // No state yet: said in plain words, and nothing about the agent or the upload.
        let text = status_header(&header(None, Some(RunMode::Practice))).join("\n");
        assert!(text.contains("no run state yet"), "{text}");
        assert!(!text.contains("Upload:"), "{text}");
        // An old run with no state did not start.
        let mut old = header(None, Some(RunMode::Practice));
        old.young = false;
        let text = status_header(&old).join("\n");
        assert!(text.contains("did not start"), "{text}");
        // A state nothing updates is not called recording, and the agent is not called running.
        let mut stale = header(Some(Phase::Recording), Some(RunMode::Competition));
        stale.stale = true;
        let text = status_header(&stale).join("\n");
        assert!(
            !text.contains("recording") && !text.contains("running"),
            "{text}"
        );
        // Time up and nothing finishing the run: say so, not "finishing".
        let mut dead = header(Some(Phase::Finishing), Some(RunMode::Competition));
        dead.stale = true;
        let first = status_header(&dead).remove(0);
        assert!(first.contains("stopped before it finished"), "{first}");
        // Problems are counted.
        let mut some = header(Some(Phase::Recording), Some(RunMode::Competition));
        some.problems = 2;
        assert!(status_header(&some).join("\n").contains(" 2"));
        // The agent and the upload in trouble.
        some.agent = Agent::Exited { how: "exit code 1" };
        some.upload = UploadNow::GaveUp;
        let text = status_header(&some).join("\n");
        assert!(text.contains("exited (exit code 1)"), "{text}");
        assert!(text.contains("stopped"), "{text}");
        for word in ["entry", "segment", "chain", "daemon", "tracer"] {
            assert!(!text.contains(word), "{word}: {text}");
        }
    }

    #[test]
    fn the_status_next_line_is_the_one_thing_to_do() {
        assert!(status_next(&Next::NothingEnds).starts_with("Next: nothing to do."));
        assert!(status_next(&Next::Nothing).starts_with("Next: nothing to do."));
        // A command ends its line: nothing after it to be copied with it.
        let go_on = status_next(&Next::Continue { run_id: "r1" });
        assert!(go_on.ends_with("goalcup continue r1"), "{go_on}");
        assert!(status_next(&Next::Upload).ends_with("goalcup upload"));
        let seal = status_next(&Next::Seal { upload: true });
        assert!(
            seal.contains("goalcup seal") && seal.ends_with("goalcup upload"),
            "{seal}"
        );
        assert!(!status_next(&Next::Seal { upload: false }).contains("upload"));
        let restart = status_next(&Next::Restart {
            connector_id: "some-cli",
            mode: Some(RunMode::Practice),
        });
        assert!(
            restart.contains("goalcup start --harness some-cli --practice"),
            "{restart}"
        );
        assert!(status_next(&Next::Problems).contains("problem"));
        assert!(status_next(&Next::Wait).contains("goalcup status"));
        // The refusal with no run starts with the command and keeps the phrase tests read.
        let none = no_run_yet();
        assert!(none.starts_with("Run goalcup prep"), "{none}");
        assert!(
            none.contains("no run has been started from this machine"),
            "{none}"
        );
    }

    #[test]
    fn upload_messages_start_with_the_next_action_and_name_it() {
        // A refusal says what to do first.
        let refusals = [
            upload_no_run(),
            upload_local_run("r1"),
            upload_not_opened("e1"),
            upload_not_here("e1"),
            upload_refused("https://s.example", "e1", "401"),
            upload_incomplete("e1", 10, "the server answered 503"),
            upload_not_sealed(),
        ];
        for said in &refusals {
            let first = said.split(' ').next().unwrap();
            assert!(
                ["Run", "Name", "Check"].contains(&first),
                "next action first: {said}"
            );
        }
        assert!(upload_no_run().contains("nothing to upload"));
        assert!(upload_local_run("r1").contains("is local"));
        assert!(upload_incomplete("e1", 10, "503").contains("goalcup upload"));
        assert!(upload_refused("https://s.example", "e1", "401").contains("https://s.example"));
        assert!(upload_waiting("busy", 5).contains("5s"));
        assert!(upload_finished().contains(NOTHING_MORE));
        assert!(upload_nothing_left("r1").contains("r1"));
        // Entries: the way to reach one run comes first.
        assert!(entries_intro(false)[0].contains("goalcup status <id>"));
        assert!(entries_intro(true)[0].contains("goalcup start"));
    }

    #[test]
    fn a_problem_line_ends_with_its_fix() {
        let who = Who {
            harness: "Some Agent",
            run_id: "r1",
            until: Some("Sat 14 Nov, 10:12 UTC"),
        };
        assert!(
            problem_agent_exited(
                &who,
                "exit code 1",
                false,
                Restart::Continue { run_id: "r1" }
            )
            .ends_with("goalcup continue r1")
        );
        assert!(
            problem_state_stale("12 min", Restart::Continue { run_id: "r1" })
                .ends_with("goalcup continue r1")
        );
        assert!(problem_upload("40 min", "503", true).ends_with("goalcup upload"));
        assert!(problem_upload("40 min", "503", false).contains("Check the network"));
        assert!(problem_heartbeats(3, "503").contains("15 min"));
        assert!(problem_disk("3.2 GiB").contains("Free some space"));
        assert!(problem_sleep("no inhibitor").contains("power"));
        assert!(problem_recording_failed("r1").contains(SUPPORT_ADDRESS));
        assert!(agent_finished("Some Agent", true).contains("run is over"));
    }

    #[test]
    fn every_refusal_says_what_to_do_first_and_has_no_rule_code() {
        let tmp = Path::new("/w");
        let refusals = [
            home_busy("Some Agent", true),
            home_busy("Some Agent", false),
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
            "Your", "Use", "Run", "Start", "Nothing", "Try", "Continue", "Check",
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
        assert!(home_busy("A", false).contains("already recording"));
        // A second start by mistake is told nothing is needed first; exiting the agent to start
        // again comes last.
        let busy = home_busy("A", false);
        assert!(busy.starts_with("Your run is already going"), "{busy}");
        assert!(busy.find("Nothing to do").unwrap() < busy.find("exit A").unwrap());
        // "This replaces your run" is true only for a competition start that would replace an
        // open competition run.
        assert!(!busy.contains("replaces"), "{busy}");
        assert!(home_busy("A", true).contains("replaces your current competition run"));
        assert!(needs_terminal("A", "").contains("foreground of a terminal"));
        assert!(not_started_here(None).contains(NOT_STARTED_HERE));
    }
}
