//! Every message the participant sees (PRD-10 section 13), as plain functions of facts; a harness
//! name comes from the connector, as `name`. The contract words the pad and scripts read stay byte
//! for byte: [`READY_MARK`], [`PROBLEM`] and the `shows the code <code>.` of [`approval_code`].

use std::path::Path;

use super::RunMode;
use super::status::State;
use crate::connector::Refusal;
use crate::upload::policy::Class;
use crate::upload::state::{State as Upload, UploadJson};
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

// The folder checks of `goalcup start` (section 3 step 2). Every path is quoted.

/// The lead line of the one folder refusal; a line per problem follows, then [`NOTHING_CREATED`].
pub const FOLDER_REFUSED: &str = "this folder cannot hold a run yet. Fix each of these, then run \
the command again:";

/// The last line of a refusal that came before anything was made.
pub const NOTHING_CREATED: &str = "Nothing was created.";

/// `start` cannot read the current folder: it was deleted or moved away.
pub const PROJECT_GONE: &str = "the project folder is gone: the folder this was run in cannot be \
read any more. Change to your project folder and run the command again.";

/// Stands for a folder entry whose name could not be read.
pub const UNREADABLE_ENTRY: &str = "an entry that could not be read";

/// Stands for the entries of a folder that could not be listed.
pub const UNLISTED_FOLDER: &str = "entries that could not be listed";

/// Names in quotes; the two stand-ins above are not names and stay bare.
fn quoted(names: &[String]) -> String {
    let quote = |n: &String| match n.as_str() {
        UNREADABLE_ENTRY | UNLISTED_FOLDER => n.clone(),
        _ => format!("\"{n}\""),
    };
    names.iter().map(quote).collect::<Vec<_>>().join(", ")
}

pub fn folder_extra(names: &[String]) -> String {
    format!(
        "It holds {}. Start in a new folder that holds only .gitignore, .DS_Store and .env files.",
        quoted(names)
    )
}

pub fn env_not_ignored(names: &[String]) -> String {
    format!(
        ".gitignore does not cover {}, so a push could publish it. Add the line .env* to \
         .gitignore.",
        quoted(names)
    )
}

pub fn inside_git(work_tree: &Path) -> String {
    format!(
        "It is inside the Git work tree \"{}\". Start in a folder outside any Git repository.",
        work_tree.display()
    )
}

/// PRD-17 SC1: the agent must never see the record.
pub fn beside_goalcup(own: &Path) -> String {
    format!(
        "It and goalcup's folder \"{}\" hold each other or sit side by side. Start in a folder \
         elsewhere, or set GOALCUP_HOME to a folder away from it.",
        own.display()
    )
}

/// The line the push block reads; the value is never shown.
pub const TOKEN_LINE: &str = "\".env.local\" needs exactly one plain GH_TOKEN=<your token> line \
(no quotes, no export, no spaces, no Windows CRLF line ending). Fix it: the push block reads it.";

pub fn prompt_inside(file: &Path) -> String {
    format!(
        "The prompt file \"{}\" is inside the project folder. Move it out of the folder.",
        file.display()
    )
}

pub fn prompt_missing(file: &Path) -> String {
    format!("The prompt file \"{}\" does not exist.", file.display())
}

pub fn synced_folder(project: &Path) -> String {
    format!(
        "\"{}\" looks like a synced folder: a sync client may upload .env.local and your token. \
         A folder that is not synced is safer.",
        project.display()
    )
}

/// The entry lock is an `flock`, which NFS and SMB may not keep.
pub fn network_folder(dir: &Path, kind: &str) -> String {
    format!(
        "\"{}\" is on {kind}, where file locks may not hold. A folder on this computer's own \
         disk is safer.",
        dir.display()
    )
}

pub fn not_built(command: &str) -> String {
    format!("{command} is not built yet")
}

/// WSL1 (`drive` None), or `what` on a Windows drive of type `drive` (section 3 step 2).
pub fn wsl(what: &str, drive: Option<&str>) -> String {
    match drive {
        None => "This is WSL1, which cannot run goalcup. Convert it to WSL2 (wsl --set-version \
                 <distro> 2) and run goalcup there."
            .to_owned(),
        Some(fs) => format!(
            "{what} is on a Windows drive ({fs}), which WSL2 reads slowly and without file modes. \
             Use a folder in the Linux home (under ~)."
        ),
    }
}

// Setup (section 7). No line here names a folder or starts with `PROBLEM:` (section 9).
pub const GOALCUP_FOLDER: &str = "goalcup's folder (GOALCUP_HOME)";

/// `<harness> <version> (<path>)`; no version before the `--version` probe has run.
pub fn harness_found(name: &str, version: Option<&str>, program: &Path) -> String {
    let version = version.map(|v| format!(" {v}")).unwrap_or_default();
    format!("{name}{version} (\"{}\")", program.display())
}

pub fn not_installed(name: &str) -> String {
    format!("{name} is not installed. Install it, then run goalcup setup again.")
}

/// No `--harness` and none saved; `installed` holds the built-in ids found on `PATH`.
pub fn no_harness(installed: &[&str]) -> String {
    let found = installed.join(", ");
    let found = if found.is_empty() { "none" } else { &found };
    format!("no harness picked (installed: {found}). Run: goalcup setup --harness <id>")
}

/// A launch plan refusal (PRD-11 section 5) as the step to fix. It names files and keys only.
pub fn refusal(r: &Refusal) -> String {
    match r {
        Refusal::HomeLocked { home } => {
            format!(
                "a run uses \"{}\" now. Run this again once it has ended.",
                home.display()
            )
        }
        Refusal::SettingSet { file, key } => format!(
            "\"{}\" sets {key}, which routes around the recorder. Remove it, then run this again.",
            file.display()
        ),
        Refusal::VersionMismatch { program, want } => format!(
            "{program} --version does not show {want}, the version goalcup works with. Install \
             that one, then run this again."
        ),
    }
}

/// What `setup --yes` wrote into the isolated `home`: an `update_off` row (`update`), a
/// `no_prompts` row (`prompts`), or neither. A row not written has its own line before this.
pub fn settings_set(name: &str, home: &Path, update: bool, prompts: bool) -> String {
    let home = home.display();
    let what = match (update, prompts) {
        (true, true) => "auto-update and its approval prompts",
        (true, false) => "auto-update",
        (false, true) => "approval prompts",
        (false, false) => return format!("goalcup set nothing in {name}'s home \"{home}\"."),
    };
    format!("goalcup turned off {name}'s {what} in \"{home}\".")
}

/// A setting row not written: its file and key, never a value.
pub fn not_set(file: &Path, key: &str) -> String {
    let file = file.display();
    format!("goalcup could not set {key} in \"{file}\". Set it there yourself if you want it.")
}

pub fn not_written(path: &Path) -> String {
    let path = path.display();
    format!("goalcup could not write \"{path}\". Check that folder, then run this again.")
}

/// Section 6; the participant edits `file` to remove a host.
pub fn host_added(host: &str, file: &Path) -> String {
    format!(
        "goalcup will record {host} from the next part (during a run: goalcup seal, then goalcup \
         continue). Your harness must trust goalcup's certificate for it, which goalcup sets up. \
         Your hosts are in \"{}\".",
        file.display()
    )
}

/// `reason` is PRD-11's, which never repeats the name given.
pub fn host_refused(reason: &str) -> String {
    format!("goalcup cannot record that host: {reason}.")
}

pub const NEVER_HOST: &str = "a sign-in host, which goalcup never decrypts";

/// Section 9: a warning, never a `PROBLEM:` line.
pub fn no_sleep_lock(why: &str) -> String {
    format!(
        "This computer may sleep during the run (no sleep lock: {why}). Keep it on mains power \
         with the lid open."
    )
}

pub const SERVER_OFF: &str = "Server check skipped: GOALCUP_SERVER=off, so nothing is uploaded.";

pub fn server_unreachable(why: &str) -> String {
    format!("goalcup checked its server: {why}. A run needs it to start.")
}

pub const NO_GIT: &str = "git is not installed. Your agent needs it to push its repository.";

/// PRD-05 section 12, the default text; `db` names a harness whose database is read whole.
pub fn masking(db: Option<&str>) -> String {
    let (and, db) = db.map_or(("and ", String::new()), |n| {
        ("", format!(", and {n}'s database"))
    });
    format!(
        "Goal Cup Proxy hides known secret shapes in memory, before anything is saved: API keys with a \
         known prefix, login headers, cookies, passwords in URLs, private keys, login tokens, values \
         named like `api_key`, `secret`, `token` or `password`, and your project's GitHub token. A \
         hidden value is replaced by stars of the same length. It cannot catch every secret. These \
         can still be recorded: a key the model repeats back in its reply, a key written right \
         after a letter, a digit or `_`, a short password, {and}a value with no known shape (for \
         example a line of `cat .env` whose name is not one of those){db}. Your email address and \
         your plan and credit balance are recorded. Raw records stay private and are cleaned \
         before research use. Use keys you can revoke, and rotate every key after the run."
    )
}

/// Section 7 step 5: the last lines of a setup that exits 0.
pub const READY: &str = "Ready.
  Practice (10 minutes, no GitHub):
    d=$(mktemp -d) && cd \"$d\" && goalcup start --practice
  Your run, inside the start window, from your empty project folder, with the prompt saved outside it:
    goalcup start --repo <you>/<repo> --prompt ~/goal-prompt.txt";

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

/// A GAP reason, by its name (`GapReason::name`, `tunnel_hosts` `listed`), in plain words (section
/// 5 table). 8, 9 and unknown reasons read `other (n)`.
pub fn gap_words(name: &str) -> String {
    match name {
        "memory_guard" => "under heavy load",
        "recorder_down" => "recording was off",
        "unsupported_encoding" | "decode_failed" => "a reply in a form goalcup cannot read",
        "source_changed" => "a session file was replaced or cut short",
        "tls_refused" | "pinned" => "the program refused the recorder",
        "cut_at_seal" => "still arriving when the part ended",
        "h2_unsupported" => "a connection goalcup could not follow",
        "mask_failed" => "goalcup could not check it for secrets, so it dropped it",
        "upstream_untrusted" => "the model service's certificate was not one goalcup trusts",
        "protocol_other" => "a connection in a form goalcup does not read",
        "after_stop" => "sent after the agent ended",
        _ => {
            let n = name
                .strip_prefix("other(")
                .and_then(|n| n.strip_suffix(')'));
            return format!("other ({})", n.unwrap_or(name));
        }
    }
    .to_owned()
}

/// `api.example.com 812, chat.example.com 93`.
fn host_list(hosts: &[(&str, u64)]) -> String {
    let list: Vec<String> = hosts.iter().map(|(h, n)| format!("{h} {n}")).collect();
    list.join(", ")
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
        State::Over { since } => format!(
            "{PROBLEM} Recording is off since {}: goalcup is not running. Time is up for this run. \
             To close the part, run: goalcup seal",
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
        State::Losing { streams, bytes } => {
            let n = plural(
                streams,
                "connection or file has",
                "connections or files have",
            );
            format!(
                "Recording is losing data now: {n} a gap ({} so far). Your agent is not slowed. \
                 The end screen will say how much.",
                size_words(bytes)
            )
        }
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
    format!("Your {kind} with {name}: {}.", state.word())
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
    let n = plural(requests, "model request", "model requests");
    let mut line = format!("Written so far: {n}");
    if !hosts.is_empty() {
        line += &format!(" ({})", host_list(hosts));
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

/// `Parts: 2. Part 1 sealed. Part 2 recording.`, one word per part.
pub fn parts_line(words: &[&str]) -> String {
    let each: Vec<String> = words
        .iter()
        .enumerate()
        .map(|(i, w)| format!(" Part {} {w}.", i + 1))
        .collect();
    format!("Parts: {}.{}", words.len(), each.concat())
}

/// `Problems: none.`, or the heading over one line per cause.
pub fn problems_head(n: usize) -> String {
    format!("Problems:{}", if n == 0 { " none." } else { "" })
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
pub fn no_request_yet() -> String {
    format!(
        "{PROBLEM} No model request was written yet. If your agent is working, it may not use the \
         recording proxy."
    )
}

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
    let what = minutes.map_or_else(
        || format!("received {} from your agent", size_words(up)),
        |m| format!("streamed data for {}", plural(m, "minute", "minutes")),
    );
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
            gap_words(reason)
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
    format!(
        "Programs {name} started sent {} after this part ended{}; that was not recorded.",
        size_words(bytes),
        listed(hosts)
    )
}

/// ` (a, b)`, or nothing for an empty list.
fn listed(items: &[String]) -> String {
    match items {
        [] => String::new(),
        _ => format!(" ({})", items.join(", ")),
    }
}

/// The harness exited and the run is still open.
pub fn agent_exited(name: &str, exit: Exit, until: &str, run: &str) -> String {
    format!(
        "{} The run is still open until {until}. To go on, run: goalcup continue {run}",
        has_exited(name, exit)
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
    format!(
        "{} written: {}.",
        plural(n, "model request", "model requests"),
        host_list(hosts)
    )
}

/// Rule 3: whole beside the request count when some have holes, and the bytes not recorded.
pub fn whole_and_lost(whole: u64, holes: u64, lost: u64) -> String {
    let holes =
        (holes > 0).then(|| format!("{whole} of them written whole, {holes} with missing parts."));
    let lost = (lost > 0).then(|| format!("{} was not recorded.", size_words(lost)));
    let parts: Vec<String> = holes.into_iter().chain(lost).collect();
    parts.join(" ")
}

/// Rule 1: gaps no other line names, of unknown size or with no byte lost.
pub fn gaps_line(n: u64, unknown: bool) -> String {
    let what = if unknown {
        "of unknown size: some data was not recorded there"
    } else {
        "with no byte known lost"
    };
    format!("The record has {} {what}.", plural(n, "gap", "gaps"))
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
        gap_words(reason),
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
    format!(
        "{} {name} started were still running at the end{}. goalcup did not stop them.",
        plural(count, "program", "programs"),
        listed(programs)
    )
}

/// Rule 7. `status.json` holds no time the record ends at, so none is named (ledger L2).
pub const RECOVERED: &str = "This part ended without goalcup (the computer stopped, or goalcup was \
killed).";

pub fn details_at(log: &Path) -> String {
    format!("Details for support are in \"{}\"", log.display())
}

// Upload (PRD-06 section 10): bytes only, never a count of requests or calls.

/// `last_error.class` in plain words; its `text` is never shown.
fn class_words(class: Class) -> &'static str {
    match class {
        Class::Network => "no network",
        Class::ServerBusy => "the server is busy",
        Class::Refused => "the server refused it",
        Class::Stuck => "the copies differ",
        Class::Internal => "a fault in goalcup",
    }
}

/// The `Upload:` line of `upload.json` at `now`. Once nothing will send the part by itself (`end`:
/// the end screen, or a sealed part with no recorder), bytes still waiting name `goalcup upload`
/// and `upload_by`; after `upload_by` they cannot be sent (section 5 rule 8).
pub fn upload_line(u: &UploadJson, now: u64, end: bool) -> String {
    let t = |s: Option<&String>| s.map(String::as_str).and_then(super::unix_of);
    let by_at = t(u.upload_by.as_ref());
    let by = by_at.map(local_clock);
    let (err, waiting) = (u.last_error.as_ref(), size_words(u.waiting_bytes));
    let raw = u.confirmed.as_ref().map_or(0, |c| c.raw);
    let goes = matches!(
        u.state,
        Upload::Sending | Upload::CaughtUp | Upload::Waiting
    );
    let left = u.waiting_bytes > 0 && (goes || u.state == Upload::Refused);
    let closed = || {
        let on = by.as_ref().map(|b| format!(" on {b}")).unwrap_or_default();
        format!(
            "Upload: the server closed this run{on}. {waiting} stays on this computer and cannot \
             be sent."
        )
    };
    if left && by_at.is_some_and(|b| now >= b) {
        return closed();
    }
    if end && goes && u.waiting_bytes > 0 {
        let before = by
            .as_ref()
            .map(|b| format!(" before {b}"))
            .unwrap_or_default();
        return format!(
            "Upload: {waiting} still waits on this computer. Run goalcup upload{before}, when you \
             are online. Nothing is lost yet."
        );
    }
    match u.state {
        Upload::Local => "Upload: none. This run stays on this computer.".to_owned(),
        Upload::Sending | Upload::CaughtUp => {
            let at = t(u.confirmed.as_ref().map(|c| &c.at)).or(t(u.last_ok.as_ref()));
            let Some(at) = at else {
                return "Upload: sending; the server holds nothing of this part yet.".to_owned();
            };
            let ago = match now.saturating_sub(at) {
                s @ 0..60 => format!("{s} s"),
                s @ 60..3600 => format!("{} min", s / 60),
                s => span_words(s * 1000),
            };
            let at = local_time(at, false);
            format!("Upload: the server holds everything up to {at} ({ago} ago).")
        }
        Upload::Waiting => {
            let since = t(u.last_ok.as_ref()).or(t(err.map(|e| &e.at)));
            let since = since.map(|s| format!(" since {}", local_time(s, false)));
            let why = err.map(|e| format!(" ({})", class_words(e.class)));
            let rest = (u.waiting_bytes > 0)
                .then(|| format!(" {waiting} waits here and goes up by itself."));
            format!(
                "Upload: not reaching the server{}{}.{} Nothing is lost.",
                since.as_deref().unwrap_or(" yet"),
                why.unwrap_or_default(),
                rest.unwrap_or_default()
            )
        }
        Upload::Refused => {
            let status = err.and_then(|e| e.status).map(|s| format!(" ({s})"));
            let status = status.unwrap_or_default();
            format!(
                "Upload: the server refuses this run{status}. It keeps trying. If this lasts, \
                 write to {SUPPORT_ADDRESS}."
            )
        }
        Upload::Stuck => format!(
            "Upload: stopped. The server's copy and this computer's copy differ at byte {raw}. \
             Nothing more is sent. Write to {SUPPORT_ADDRESS}."
        ),
        Upload::Done if u.keep => {
            "Upload: done. Every record file is kept on this computer (GOALCUP_KEEP_PARTS)."
                .to_owned()
        }
        Upload::Done => {
            let n = u.pruned.parts;
            let removed = (n > 0).then(|| {
                let files = plural(n, "record file", "record files");
                format!(" Removed here: {files} the server holds.")
            });
            let whole = size_words(raw);
            format!(
                "Upload: done. The server holds your whole record ({whole}).{}",
                removed.unwrap_or_default()
            )
        }
        Upload::Closed => closed(),
    }
}

// Time left (section 10, D66): the agent may run it too.

fn advice(practice: bool) -> &'static str {
    if practice {
        "This is a practice run: it needs no repository, so there is nothing to push."
    } else {
        "Push your work often; work not pushed when the run stops is lost."
    }
}

/// The run stops at `stop` (seconds since 1970), as the server last said.
pub fn time_left(stop: u64, now: u64, practice: bool) -> String {
    match stop.checked_sub(now).filter(|left| *left >= 60) {
        Some(_) => format!(
            "The run stops {}. {}",
            end_words(stop, now),
            advice(practice)
        ),
        None => format!("Less than a minute left. {}", advice(practice)),
    }
}

/// The server said the run's time is up.
pub const TIME_IS_UP: &str = "Time is up.";

pub fn time_left_unknown(practice: bool) -> String {
    format!(
        "{} The time left is unknown right now: goalcup has no fresh word from the server.",
        advice(practice)
    )
}

pub const NO_OPEN_RUN_TIME_LEFT: &str = "Run goalcup status to see your runs. No run is open on \
this computer, so there is no time left to show.";
