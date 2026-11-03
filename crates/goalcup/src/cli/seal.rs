//! `goalcup seal [PART] [--yes]` (PRD-10 section 1): ask, then tell PRD-02 to end the part, wait
//! for the seal and print the end screen. The harness is never signalled here and descendants are
//! never chased: the watchdog ends them (seam S8), except one `goalcup recover` started, which leaves
//! the harness running. Only written counts are read, from `status.json`.
//! A part whose recorder and watchdog are both gone is closed here by `pipeline::close_dead_entry`;
//! a harness still running there is left running.

use std::io::{BufRead as _, IsTerminal as _};
use std::process::ExitCode;
use std::thread::sleep;
use std::time::{Duration, Instant};

use super::{
    alive, broke, fail, find, home, read_json, say, status_json, text, time_left, watchdog_pid,
};
use crate::pipeline::status::{Mode, Status};
use crate::pipeline::{Closed, close_dead_entry};

const POLL: Duration = Duration::from_millis(200);
const NOTICE_EVERY: Duration = Duration::from_secs(10);

/// 0 sealed and shown, 1 refused (ask, no part, already ended, not running), 3 `status.json`
/// unreadable or the part could not be finished.
pub fn run(part: Option<String>, yes: bool) -> ExitCode {
    if super::var(time_left::FILE_ENV).is_some() {
        return fail(text::SEAL_INSIDE);
    }
    let home = match home() {
        Ok(h) => h,
        Err(t) => return fail(t),
    };
    let Some(found) = find(&home, part.as_deref()) else {
        let line = part.map_or_else(|| text::NO_RUN_YET.to_owned(), |p| text::no_such_part(&p));
        return fail(&line);
    };
    let dir = found.entry_dir();
    let path = status_json(&dir);
    let Some(st) = read_json::<Status>(&path) else {
        return broke(&text::status_unreadable(&path));
    };
    if st.mode == Mode::Sealed {
        // It may have ended in the participant's absence: show what it recorded.
        if let Ok(screen) = super::finish::end_screen(&dir) {
            print!("{screen}");
        }
        return fail(&text::seal_already(&found.run));
    }
    // The watchdog takes SIGUSR1; with none, the recorder takes SIGTERM and seals as `signal`.
    let target = match watchdog_pid(&dir).filter(|&(p, s)| alive(p, s)) {
        Some((p, s)) => Some((p, s, libc::SIGUSR1)),
        None if alive(st.pid, st.start_id) => Some((st.pid, st.start_id, libc::SIGTERM)),
        None => None,
    };
    let name = found
        .server
        .as_ref()
        .map_or("your agent", |s| s.connector.name.as_str());
    if !yes {
        if !std::io::stdin().is_terminal() {
            return fail(text::SEAL_NEEDS_TERMINAL);
        }
        eprint!("{}", text::seal_question(name));
        let mut answer = String::new();
        let _ = std::io::stdin().lock().read_line(&mut answer);
        if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
            return fail(text::SEAL_NOT_CONFIRMED);
        }
    }
    let harness_alive = || {
        st.harness
            .as_ref()
            .is_some_and(|h| h.exit.is_none() && alive(h.pid, h.start_id))
    };
    let harness_ran = harness_alive();
    let Some((pid, start_id, signal)) = target else {
        return seal_dead(&found, name, harness_ran);
    };
    if let Ok(p) = libc::pid_t::try_from(pid) {
        // SAFETY: `alive` just saw this pid with this start id; it is above 0.
        unsafe { libc::kill(p, signal) };
    }
    say(text::SEAL_WAIT);
    let mut noticed = Instant::now();
    loop {
        let sealed = || read_json::<Status>(&path).is_some_and(|s| s.mode == Mode::Sealed);
        if sealed() {
            break;
        }
        if !alive(pid, start_id) {
            if sealed() {
                break;
            }
            return broke(&text::seal_could_not());
        }
        if noticed.elapsed() >= NOTICE_EVERY {
            say(text::SEAL_STILL);
            noticed = Instant::now();
        }
        sleep(POLL);
    }
    match super::finish::end_screen(&dir) {
        Ok(screen) => {
            // A watchdog seals its own harness only once it has exited. A recovered watchdog, or the
            // recorder alone on SIGTERM, seals with the harness still running.
            if harness_ran && harness_alive() {
                println!("{}", text::seal_harness_left(name));
            } else if harness_ran {
                println!("{}", text::seal_stopped(name));
            }
            print!("{screen}");
            ExitCode::SUCCESS
        }
        Err(_) => broke(&text::seal_could_not()),
    }
}

/// A dead entry: close it with seal reason `recovered`, show the end screen as after a live seal,
/// then send it (as `goalcup upload` does). The screen comes first: a server that is down makes the
/// upload wait with no cap of its own, and the participant has had their counts by then. A harness
/// still running is not signalled.
fn seal_dead(found: &super::Found, name: &str, harness_ran: bool) -> ExitCode {
    let (dir, run) = (found.entry_dir(), found.run.as_str());
    match close_dead_entry(&dir) {
        Err(_) => return fail(&text::seal_still_recording(run)),
        Ok(Closed::AlreadySealed) => return fail(&text::seal_already(run)),
        Ok(Closed::Failed(_)) => return broke(&text::seal_could_not()),
        Ok(Closed::Sealed(_)) => {}
    }
    let Ok(screen) = super::finish::end_screen(&dir) else {
        return broke(&text::seal_could_not());
    };
    if harness_ran {
        println!("{}", text::seal_dead_harness_left(name));
    }
    print!("{screen}");
    // What the server does not hold stays on disk and `goalcup upload` sends it later.
    if found.server.as_ref().is_some_and(|s| s.server.is_some()) {
        say(text::SEAL_SENDING);
    }
    let keep = super::var("GOALCUP_KEEP_PARTS").is_some_and(|v| v == "1");
    let out = crate::upload::manual::run(&dir, &found.run_dir, (run, &found.entry), keep);
    println!(
        "{}",
        super::upload::outcome_line(&dir, &out, &found.entry, run)
    );
    ExitCode::SUCCESS
}
