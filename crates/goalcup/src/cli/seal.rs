//! `goalcup seal [PART] [--yes]` (PRD-10 section 1): ask, then tell PRD-02 to end the part, wait
//! for the seal and print the end screen. The harness is never signalled here and descendants are
//! never chased: the watchdog ends them (seam S8). Only written counts are read, from `status.json`.

use std::io::{BufRead as _, IsTerminal as _};
use std::process::ExitCode;
use std::thread::sleep;
use std::time::{Duration, Instant};

use super::{alive, fail, find, home, read_json, say, status_json, text, time_left, watchdog_pid};
use crate::pipeline::status::{Mode, Status};

const POLL: Duration = Duration::from_millis(200);
const NOTICE_EVERY: Duration = Duration::from_secs(10);

/// 0 sealed and shown, 1 refused or not finished, 3 `status.json` unreadable.
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
        return fail(&text::status_unreadable(&path));
    };
    if st.mode == Mode::Sealed {
        return fail(&text::seal_already(&found.run));
    }
    // The watchdog takes SIGUSR1; with none, the recorder takes SIGTERM and seals as `signal`.
    let (pid, start_id, signal) = match watchdog_pid(&dir).filter(|&(p, s)| alive(p, s)) {
        Some((p, s)) => (p, s, libc::SIGUSR1),
        None if alive(st.pid, st.start_id) => (st.pid, st.start_id, libc::SIGTERM),
        None => return fail(&text::seal_not_running(&found.run)),
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
    let harness_ran = st
        .harness
        .as_ref()
        .is_some_and(|h| h.exit.is_none() && alive(h.pid, h.start_id));
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
            return fail(&text::seal_could_not());
        }
        if noticed.elapsed() >= NOTICE_EVERY {
            say(text::SEAL_STILL);
            noticed = Instant::now();
        }
        sleep(POLL);
    }
    match super::finish::end_screen(&dir) {
        Ok(screen) => {
            if harness_ran {
                println!("{}", text::seal_stopped(name));
            }
            print!("{screen}");
            ExitCode::SUCCESS
        }
        Err(_) => fail(&text::seal_could_not()),
    }
}
