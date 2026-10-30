//! Every message the participant sees (PRD-10 section 13), as plain functions of facts.
//!
//! The three contract words are kept byte for byte, because the pad and scripts read them:
//! [`READY_MARK`], [`PROBLEM`] and the `shows the code <code>.` ending of [`approval_code`].
//! No harness name is written here: it comes from the connector, as `name`.

use std::path::Path;

use goalcup_format::GapReason;

use super::RunMode;
use super::status::State;
use crate::watchdog::Exit;

/// The start of the line that says the harness is being recorded.
pub const READY_MARK: &str = "Recording in chat mode";

/// The start of a status line the participant can act on or must report.
pub const PROBLEM: &str = "PROBLEM:";

/// Where a participant writes for help.
pub const SUPPORT_ADDRESS: &str = "goalcup@agentcommunity.org";

/// The approval line; it ends `shows the code <code>.`.
pub fn approval_code(code: &str) -> String {
    format!("Check that the approval page shows the code {code}.")
}

/// The footer of `goalcup --help` (PRD-10 section 11).
pub fn after_help() -> String {
    format!(
        "\
Steps:
  1. goalcup setup                                   once, until it says Ready.
  2. goalcup start --practice                         a 10-minute try, no GitHub (optional)
  3. goalcup start --repo <you>/<repo> --prompt FILE  your run, in the start window; FILE outside the folder
While it runs: goalcup status (in another terminal). If status says so: goalcup recover.
If the agent stopped early: goalcup continue.
Guide: https://github.com/agentcommunity/goal-cup-proxy/blob/main/docs/PARTICIPANT-GUIDE.md
Help: {SUPPORT_ADDRESS}
Exit codes: 0 done; 1 refused, and you can fix it; 3 the command could not do its job."
    )
}

/// PRD-10 section 1, "File connectors in competition".
pub const FILE_FLAGS_COMPETE: &str = "--connector and --hosts are for practice and test runs. \
A competition run uses the connectors built into goalcup.";

/// `GOALCUP_SERVER` holds something a run cannot use.
pub const BAD_SERVER: &str = "GOALCUP_SERVER must be off, an https:// address, or an http:// \
address on this computer (127.0.0.1, ::1 or localhost).";

/// No absolute folder to keep runs in: never a folder relative to where goalcup was started.
pub const NO_HOME: &str = "goalcup cannot tell where to keep your runs: HOME is not set to a full \
path. Set HOME, or GOALCUP_HOME, to a folder outside your project.";

/// A relative `GOALCUP_HOME`, and the current folder cannot be read (it was deleted).
pub const NO_CWD: &str = "goalcup cannot tell where to keep your runs: GOALCUP_HOME is not a full \
path and the current folder cannot be read. Set GOALCUP_HOME to a full path outside your project.";

/// A command whose body is not in this build yet.
pub fn not_built(command: &str) -> String {
    format!("{command} is not built yet")
}

// Times and sizes, as the participant reads them.

/// `Fri 7 Nov` and `10:12:05` of seconds since 1970 read as a wall clock.
fn day_and_time(secs: u64) -> (String, String) {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    // `YYYY-MM-DDTHH:MM:SSZ`, fixed width.
    let stamp = goalcup_format::dump::wall(secs.saturating_mul(1_000_000_000));
    let month: usize = stamp[5..7].parse().unwrap_or(1);
    let day: u32 = stamp[8..10].parse().unwrap_or(1);
    // 1970-01-01 was a Thursday.
    let weekday = DAYS[((secs / 86_400 + 4) % 7) as usize];
    let date = format!("{weekday} {day} {}", MONTHS[month.clamp(1, 12) - 1]);
    (date, stamp[11..19].to_owned())
}

fn local(unix: u64) -> (u64, i64, String) {
    let (off, zone) = super::local_offset(unix).unwrap_or((0, String::new()));
    let at = u64::try_from(i64::try_from(unix).unwrap_or(i64::MAX).saturating_add(off));
    (at.unwrap_or(0), off, zone)
}

/// A moment with its date: local time with the zone and UTC in brackets, as `Sat 14 Nov, 10:12
/// CET (09:12 UTC)`. A computer on UTC gets UTC once.
pub fn local_clock(unix: u64) -> String {
    let (at, off, zone) = local(unix);
    let (day, time) = day_and_time(at);
    if off == 0 {
        return format!("{day}, {} UTC", &time[..5]);
    }
    let zone = if !zone.is_empty() && zone.chars().all(|c| c.is_ascii_alphabetic()) {
        zone
    } else {
        let a = off.unsigned_abs();
        let sign = if off < 0 { '-' } else { '+' };
        format!("UTC{sign}{:02}:{:02}", a / 3600, a % 3600 / 60)
    };
    let (utc_day, utc) = day_and_time(unix);
    let utc = if utc_day == day {
        utc[..5].to_owned()
    } else {
        format!("{utc_day}, {}", &utc[..5])
    };
    format!("{day}, {} {zone} ({utc} UTC)", &time[..5])
}

/// A local time of day, `15:43:15`, or `15:43` without the seconds.
pub fn local_time(unix: u64, seconds: bool) -> String {
    let (_, time) = day_and_time(local(unix).0);
    if seconds { time } else { time[..5].to_owned() }
}

/// `95 h 10 min left`, `10 min left`, `less than a minute left`.
pub fn left_words(secs: u64) -> String {
    match secs {
        0..60 => "less than a minute left".to_owned(),
        60..3600 => format!("{} min left", secs / 60),
        _ => format!("{} h {} min left", secs / 3600, secs % 3600 / 60),
    }
}

/// A run's end: `Sat 14 Nov, 10:12 CET (09:12 UTC), 95 h 10 min left`.
pub fn end_words(deadline: u64, now: u64) -> String {
    match deadline.checked_sub(now) {
        Some(left) if left > 0 => format!("{}, {}", local_clock(deadline), left_words(left)),
        _ => local_clock(deadline),
    }
}

/// The `Ends:` line of the status block, or `Ended:` once the time is up.
pub fn ends_line(deadline: u64, now: u64) -> String {
    let word = if deadline > now { "Ends" } else { "Ended" };
    format!("{word}: {}.", end_words(deadline, now))
}

/// Bytes in decimal units: `512 bytes`, `40 kB`, `1.5 MB`, `48 MB`.
pub fn size_words(bytes: u64) -> String {
    match bytes {
        0..1000 => format!("{bytes} bytes"),
        1000..1_000_000 => format!("{} kB", bytes / 1000),
        1_000_000..10_000_000 => format!("{:.1} MB", bytes as f64 / 1e6),
        _ => format!("{} MB", bytes / 1_000_000),
    }
}

/// A span of time: `45 s`, `2 min 30 s`, `1 h 5 min`.
pub fn span_words(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min {} s", s / 60, s % 60),
        _ => format!("{} h {} min", s / 3600, s % 3600 / 60),
    }
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `exit code 0` or `signal 9`.
pub fn exit_words(exit: Exit) -> String {
    match exit {
        Exit::Code(c) => format!("exit code {c}"),
        Exit::Signal(s) => format!("signal {s}"),
    }
}

/// Gap reasons in plain words (section 5 table). 8, 9 and unknown numbers read `other (n)`.
pub fn gap_words(n: u8) -> String {
    match n {
        1 => "under heavy load",
        2 => "recording was off",
        3 | 4 => "a reply in a form goalcup cannot read",
        5 => "a session file was replaced or cut short",
        6 | 7 => "the program refused the recorder",
        10 => "still arriving when the part ended",
        11 => "a connection goalcup could not follow",
        12 => "goalcup could not check it for secrets, so it dropped it",
        13 => "the model service's certificate was not one goalcup trusts",
        14 => "a connection in a form goalcup does not read",
        15 => "sent after the agent ended",
        _ => return format!("other ({n})"),
    }
    .to_owned()
}

/// The words of a GAP reason given by name (`tunnel_hosts` `listed`).
pub fn gap_name_words(name: &str) -> String {
    (1..=u8::MAX)
        .find(|n| GapReason::from_u8(*n).name() == name)
        .map_or_else(|| name.to_owned(), gap_words)
}

// Status (section 4).

/// No run on this computer.
pub const NO_RUN_YET: &str =
    "There is no run on this computer yet. To start one, run: goalcup start";

/// `goalcup status <PART>` names nothing on this computer.
pub fn no_such_part(arg: &str) -> String {
    format!(
        "There is no run or part {arg} on this computer. Run goalcup status to see the newest run."
    )
}

/// `status.json` is missing or does not parse.
pub fn status_unreadable(path: &Path) -> String {
    format!(
        "goalcup could not read \"{}\", so it cannot say how this part is doing. Write to {SUPPORT_ADDRESS}.",
        path.display()
    )
}

/// The start of the sealed state line; never printed for a part that is not sealed.
pub const FINISHED: &str = "Recording: finished at";

/// The recording state line (section 4 table). `run` is the run id for the continue line.
pub fn state_line(state: &State, run: &str) -> String {
    let t = |at: u64| local_time(at, true);
    match *state {
        State::Sealed { at } => format!(
            "{FINISHED} {}. Nothing is recorded now.",
            local_time(at, false)
        ),
        State::Restarting { since } => format!("Recording is restarting (since {}).", t(since)),
        State::OffRestarting { since } => format!(
            "Recording is off since {}. Your agent keeps working; this time will show as a gap.",
            t(since)
        ),
        State::Recover { since } => format!(
            "{PROBLEM} Recording is off since {}: goalcup stopped, so your agent cannot reach the \
             model service. Your agent is still running. Run: goalcup recover",
            t(since)
        ),
        State::Continue { since } => format!(
            "{PROBLEM} Recording is off since {}: goalcup is not running. To go on, run: goalcup continue {run}",
            t(since)
        ),
        State::Stale { since } => format!(
            "{PROBLEM} goalcup has not updated this run since {}. The computer may have slept or \
             restarted. To go on, run: goalcup continue {run}",
            local_time(since, false)
        ),
        State::Null { since } => format!(
            "{PROBLEM} Recording is off since {}: it stopped right after each restart, so goalcup \
             lets your agent's traffic pass unrecorded for now. Your agent keeps working. goalcup \
             tries again at most 5 minutes apart until the run ends. The server is told.",
            t(since)
        ),
        State::NotAnswering { since } => format!(
            "Recording is not answering since {}. Your agent is not slowed.",
            t(since)
        ),
        State::Losing { streams, bytes } => format!(
            "Recording is losing data now: {} have a gap ({} so far). Your agent is not slowed. \
             The end screen will say how much.",
            plural(streams, "connection or file", "connections or files"),
            size_words(bytes)
        ),
        State::WriterSlow { secs } => {
            format!("Recording is behind: nothing written for {secs} s. Your agent is not slowed.")
        }
        State::Recording => "Recording: on.".to_owned(),
    }
}

/// The first line of the status block.
pub fn status_head(mode: Option<RunMode>, name: &str, state: &State) -> String {
    let kind = match mode {
        Some(RunMode::Practice) => "practice run",
        Some(RunMode::Test) => "organizer test run",
        Some(RunMode::Competition) => "competition run",
        None => "run",
    };
    let word = match state {
        State::Sealed { .. } => "finished",
        s if s.is_problem() => "recording is off",
        _ => "recording",
    };
    format!("Your {kind} with {name}: {word}.")
}

/// The harness line of the block: `running.`, `exited (exit code 0).`, `not running.`.
pub fn harness_line(name: &str, alive: bool, exit: Option<Exit>, started: bool) -> String {
    match (started, alive, exit) {
        (false, ..) => format!("{name}: not started yet."),
        (_, true, _) => format!("{name}: running."),
        (_, _, Some(e)) => format!("{name}: exited ({}).", exit_words(e)),
        _ => format!("{name}: not running."),
    }
}

/// `Written so far:`, with the hosts by count and the whole count when some have holes.
pub fn written_line(requests: u64, hosts: &[(&str, u64)], whole: u64, holes: u64) -> String {
    let mut line = format!(
        "Written so far: {}",
        plural(requests, "model request", "model requests")
    );
    if !hosts.is_empty() {
        let list: Vec<String> = hosts.iter().map(|(h, n)| format!("{h} {n}")).collect();
        line += &format!(" ({})", list.join(", "));
    }
    if holes > 0 {
        line += &format!("; {whole} whole, {holes} with missing parts");
    }
    line + "."
}

/// `Session files: 43, 48 MB.`
pub fn session_files_line(files: u64, bytes: u64) -> String {
    format!("Session files: {files}, {}.", size_words(bytes))
}

/// `Off: 1 time, 2 min 30 s in total.`
pub fn off_line(spans: u64, ms: u64) -> String {
    match (spans, ms) {
        (0, 0) => "Off: never.".to_owned(),
        _ => format!(
            "Off: {}, {} in total.",
            plural(spans, "time", "times"),
            span_words(ms)
        ),
    }
}

/// `Parts: 2. Part 1 sealed. Part 2 recording.`
pub fn parts_line(sealed: &[bool]) -> String {
    let each: Vec<String> = sealed
        .iter()
        .enumerate()
        .map(|(i, s)| {
            format!(
                " Part {} {}.",
                i + 1,
                if *s { "sealed" } else { "recording" }
            )
        })
        .collect();
    format!("Parts: {}.{}", sealed.len(), each.concat())
}

/// `Problems: none.`, or the heading over one line per cause.
pub fn problems_head(n: usize) -> String {
    if n == 0 {
        "Problems: none.".to_owned()
    } else {
        "Problems:".to_owned()
    }
}

/// Harness running, no request for 10 minutes, session files newer.
pub fn quiet_proxy(name: &str) -> String {
    format!(
        "{PROBLEM} No model request was written for 10 minutes, but {name}'s session files grew. \
         It may not be using the recording proxy. Its session files are still recorded. If it uses \
         a model service goalcup does not know, see goalcup setup --record-host."
    )
}

/// No model request at all 5 minutes after the launch.
pub const NO_REQUEST_YET: &str = "PROBLEM: No model request was written yet. If your agent is \
working, it may not use the recording proxy.";

pub fn native_missing(row: &str, name: &str) -> String {
    format!(
        "goalcup has found no {row} file for {name} yet. Its session may be stored somewhere \
         goalcup does not read."
    )
}

pub fn native_other_device(path: &str, name: &str) -> String {
    format!(
        "goalcup does not read {path} for {name}: it is on another disk. Session files there are \
         not recorded."
    )
}

/// Note `long_tunnel`: `minutes` streamed (`why: stream`), or `up` bytes sent (`why: upload`).
pub fn long_tunnel(host: &str, minutes: Option<u64>, up: u64) -> String {
    let what = match minutes {
        Some(m) => format!("streamed data for {}", plural(m, "minute", "minutes")),
        None => format!("received {} from your agent", size_words(up)),
    };
    format!(
        "{PROBLEM} {host} {what} and was not recorded. If it is a model service, run: goalcup \
         setup --record-host {host}. goalcup records it from the next part: to start one now, run \
         goalcup seal, then goalcup continue."
    )
}

/// A listed host tunnelled unread, by its GAP reason name (or note `ca_refused`).
pub fn listed_tunnel(host: &str, reason: &str) -> String {
    match reason {
        "tls_refused" | "pinned" => format!(
            "{PROBLEM} Not recorded: {host} refused the recorder (it checks its own certificates). \
             Its traffic passed through unread."
        ),
        "upstream_untrusted" => format!(
            "{PROBLEM} Not recorded: {host}'s certificate is not one goalcup trusts (a company \
             proxy?), so goalcup passed it through unread."
        ),
        _ => format!(
            "{PROBLEM} Not recorded: {host} ({}). Its traffic passed through unread.",
            gap_name_words(reason)
        ),
    }
}

/// Note `parse_stopped`; the host and time when the note has them.
pub fn parse_stopped(host: Option<&str>, at: Option<u64>) -> String {
    let to = host.map(|h| format!(" to {h}")).unwrap_or_default();
    let when = at
        .map(|t| format!(" at {}", local_time(t, false)))
        .unwrap_or_default();
    format!(
        "Recording of one connection{to} stopped{when}; its traffic still passes. Requests on it \
         after that are not recorded."
    )
}

pub fn local_model(host: &str) -> String {
    format!("Calls to a model on this computer ({host}) are not recorded.")
}

pub fn fd_limit(limit: Option<u64>) -> String {
    let limit = limit.map(|n| format!(" (limit {n})")).unwrap_or_default();
    format!("Many connections are open at once{limit}. New ones wait a moment; none is refused.")
}

pub fn fail_closed(bytes: u64) -> String {
    format!(
        "goalcup hid {} it could not check for secrets in time. Those bytes are stars in the record.",
        size_words(bytes)
    )
}

/// A note this build has no words for, shown as is.
pub fn unknown_note(what: &str) -> String {
    format!("goalcup noted: {what}.")
}

/// Section 5 rule 6: bytes after the seal's last link.
pub fn after_seal(name: &str, bytes: u64, hosts: &[String]) -> String {
    let hosts = if hosts.is_empty() {
        String::new()
    } else {
        format!(" ({})", hosts.join(", "))
    };
    format!(
        "Programs {name} started sent {} after this part ended{hosts}; that was not recorded.",
        size_words(bytes)
    )
}

/// The harness exited and the run is still open.
pub fn agent_exited(name: &str, exit: Exit, until: &str, run: &str) -> String {
    format!(
        "{name} has exited ({}). The run is still open until {until}. To go on, run: goalcup \
         continue {run}",
        exit_words(exit)
    )
}

// The end screen (section 5).

/// The heading of the finished block; only a sealed part has it.
pub const PART_RECORDED: &str = "What this part recorded (on this computer):";

/// Rule 1: only when nothing at all was lost.
pub const NOTHING_LOST: &str = "Nothing was lost.";

pub fn has_exited(name: &str, exit: Exit) -> String {
    format!("{name} has exited ({}).", exit_words(exit))
}

pub fn still_open(until: &str) -> String {
    format!("Your run is still open until {until}.")
}

pub fn continue_now(run: &str) -> String {
    format!("To go on, run: goalcup continue {run} (you can do it in another terminal now).")
}

/// Rule 2.
pub fn no_request_written(name: &str) -> String {
    format!(
        "No model request was written. {name} may not use the recording proxy, or it made no call."
    )
}

pub fn requests_written(n: u64, hosts: &[(&str, u64)]) -> String {
    let list: Vec<String> = hosts.iter().map(|(h, n)| format!("{h} {n}")).collect();
    format!(
        "{} written: {}.",
        plural(n, "model request", "model requests"),
        list.join(", ")
    )
}

/// Rule 3: whole beside the request count when some have holes, and the bytes not recorded.
pub fn whole_and_lost(whole: u64, holes: u64, lost: u64) -> String {
    let mut parts = Vec::new();
    if holes > 0 {
        parts.push(format!(
            "{whole} of them written whole, {holes} with missing parts."
        ));
    }
    if lost > 0 {
        parts.push(format!("{} was not recorded.", size_words(lost)));
    }
    parts.join(" ")
}

pub fn session_files(files: u64, bytes: u64) -> String {
    format!(
        "{}, {}.",
        plural(files, "session file", "session files"),
        size_words(bytes)
    )
}

pub fn off_total(spans: u64, ms: u64) -> String {
    format!(
        "Recording was off {}, {} in total.",
        plural(spans, "time", "times"),
        span_words(ms)
    )
}

/// Rule 4: a listed host, always on its own line.
pub fn tunnel_listed(host: &str, reason: &str, conns: u64, bytes: u64) -> String {
    format!(
        "Not recorded, passed through unread: {host} ({}; {}, {}).",
        gap_name_words(reason),
        plural(conns, "connection", "connections"),
        size_words(bytes)
    )
}

/// Rule 4: unlisted hosts, largest first (`(host, conns, bytes)`), and how many more.
pub fn tunnels_unlisted(shown: &[(&str, u64, u64)], more: usize) -> String {
    let list: Vec<String> = shown
        .iter()
        .enumerate()
        .map(|(i, (h, c, b))| match i {
            0 => format!(
                "{h} ({}, {})",
                plural(*c, "connection", "connections"),
                size_words(*b)
            ),
            _ => format!("{h} ({c}, {})", size_words(*b)),
        })
        .collect();
    let more = match more {
        0 => String::new(),
        n => format!(", and {}", plural(n as u64, "more host", "more hosts")),
    };
    format!(
        "Not recorded, passed through unread: {}{more}.",
        list.join(", ")
    )
}

/// Rule 5.
pub fn cut_files(files: u64, bytes: u64) -> String {
    format!(
        "{} still growing at the end; their last {} were not read in time.",
        plural(files, "session file was", "session files were"),
        size_words(bytes)
    )
}

/// Rule 6, outlived processes.
pub fn outlived(name: &str, count: u64, programs: &[String]) -> String {
    let list = if programs.is_empty() {
        String::new()
    } else {
        format!(" ({})", programs.join(", "))
    };
    format!(
        "{} {name} started were still running at the end{list}. goalcup did not stop them.",
        plural(count, "program", "programs")
    )
}

/// Rule 7. `status.json` holds no time the record ends at, so none is named (ledger L2).
pub const RECOVERED: &str = "This part ended without goalcup (the computer stopped, or goalcup was \
killed).";

pub fn details_at(log: &Path) -> String {
    format!("Details for support are in \"{}\"", log.display())
}
