//! `goalcup recover [RUN]` (PRD-10 section 3.2, ledger R3b): bring back only the recorder when the
//! watchdog and the recorder are both dead and the harness still runs. The harness is never
//! signalled, restarted or waited for; no sleep lock is taken and no `plan.after` runs.

use std::process::ExitCode;
use std::time::{Duration, UNIX_EPOCH};

use super::log::{Fact, line};
use super::start::{self, Part};
use super::{
    alive, broke, fail, find, goalcup_log, home, read_json, status_json, text, time_left, unix_of,
    watchdog_alive,
};
use crate::connector::{self, Env, HomeLock};
use crate::mask::{Masker, launch_values, process_env};
use crate::pipeline::status::{Mode, Status};
use crate::watchdog::{HarnessFacts, NotRecovered, StartFacts, recover};

/// 0 recording is on (again), 1 refused, 3 recording could not be restarted.
pub fn run(arg: Option<String>) -> ExitCode {
    if super::var(time_left::FILE_ENV).is_some() {
        return fail(text::RECOVER_INSIDE);
    }
    let home = match home() {
        Ok(h) => h,
        Err(t) => return fail(t),
    };
    let Some(found) = find(&home, arg.as_deref()) else {
        let line = arg.map_or_else(|| text::NO_RUN_YET.to_owned(), |p| text::no_such_part(&p));
        return fail(&line);
    };
    let (dir, run, now) = (found.entry_dir(), found.run.clone(), super::now());
    let path = status_json(&dir);
    let Some(st) = read_json::<Status>(&path) else {
        return broke(&text::status_unreadable(&path));
    };
    let Some(server) = found.server.as_ref() else {
        return broke(&text::status_unreadable(&super::server_json(
            &found.run_dir,
        )));
    };
    let deadline = server.deadline.as_deref().or(st.deadline.as_deref());
    let deadline = deadline.and_then(unix_of);
    let name = server.connector.name.as_str();
    let said = |why: &NotRecovered| not_recovered(why, name, &run, st.port, &dir);
    let harness = st
        .harness
        .as_ref()
        .filter(|h| h.exit.is_none() && alive(h.pid, h.start_id));
    if st.mode == Mode::Sealed {
        return said(&NotRecovered::Sealed);
    }
    if deadline.is_some_and(|d| d <= now) {
        return fail(text::RECOVER_TIME_UP);
    }
    if watchdog_alive(&dir) {
        return said(&NotRecovered::WatchdogAlive);
    }
    if alive(st.pid, st.start_id) {
        return said(&NotRecovered::RecordingOn);
    }
    let Some(harness) = harness else {
        return said(&NotRecovered::HarnessGone);
    };
    let Some(facts) = rebuild(&found, &st, harness.pid, harness.start_id, deadline) else {
        return broke(&text::recover_unreadable(&support(&run, &dir)));
    };
    let since = if st.off.since.is_some() {
        st.off.since.as_deref()
    } else {
        Some(st.updated.as_str())
    };
    let since = since.and_then(unix_of).unwrap_or(now);
    let outcome = recover(&dir, facts);
    let word = match &outcome {
        Ok(r) if r.late => "late",
        Ok(_) => "recovered",
        Err(why) => outcome_word(why),
    };
    line(
        &dir,
        "recover",
        &[
            (
                "off_since",
                Fact::Time(UNIX_EPOCH + Duration::from_secs(since)),
            ),
            ("port", Fact::N(u64::from(st.port))),
            ("outcome", Fact::Word(word)),
        ],
    );
    let r = match outcome {
        Ok(r) => r,
        Err(why) => return said(&why),
    };
    let part = found
        .entries
        .iter()
        .position(|e| *e == found.entry)
        .map_or(1, |i| i + 1);
    if r.late {
        println!("{}", text::recover_late(name, part));
    } else {
        let span = text::span_words(now.saturating_sub(since) * 1000);
        let (from, to) = (text::local_time(since, true), text::local_time(now, true));
        println!("{}", text::recover_on_again(name, part, &span, &from, &to));
    }
    ExitCode::SUCCESS
}

/// The word `goalcup.log` carries for each way a recover does not happen.
fn outcome_word(why: &NotRecovered) -> &'static str {
    match why {
        NotRecovered::Sealed => "sealed",
        NotRecovered::WatchdogAlive => "watchdog_alive",
        NotRecovered::RecordingOn => "recording_on",
        NotRecovered::HarnessGone => "harness_gone",
        NotRecovered::PortTaken => "port_taken",
        NotRecovered::KeyMissing => "key_missing",
        NotRecovered::NotStarted => "not_started",
    }
}

/// What the participant is told for each way a recover does not happen, and the exit code.
fn not_recovered(
    why: &NotRecovered,
    name: &str,
    run: &str,
    port: u16,
    dir: &std::path::Path,
) -> ExitCode {
    match why {
        NotRecovered::Sealed => fail(&text::recover_sealed(run)),
        NotRecovered::WatchdogAlive => fail(text::RECOVER_WATCHDOG_ALIVE),
        NotRecovered::RecordingOn => {
            println!("{}", text::RECOVER_ON);
            ExitCode::SUCCESS
        }
        NotRecovered::HarnessGone => fail(&text::recover_gone(name, run)),
        NotRecovered::PortTaken | NotRecovered::KeyMissing => fail(&text::recover_port(
            port,
            *why == NotRecovered::KeyMissing,
            run,
        )),
        NotRecovered::NotStarted => broke(&text::recover_not_started(run, &support(run, dir))),
    }
}

fn support(run: &str, dir: &std::path::Path) -> String {
    text::recover_support(run, &goalcup_log(dir))
}

/// The start facts again, from the run's own files: the part's folder and host lists as `start` or
/// `continue` saved them in `server.json` (never this terminal's), the run's own server, and the
/// connector the first part used. None when one is missing or changed, and nothing is guessed.
/// The exact values come from the live harness's own environment where the system shows it.
/// The plan is made while this process holds the home lock itself: the plan then writes nothing in
/// the running harness's home and does not run the harness.
fn rebuild(
    found: &super::Found,
    st: &Status,
    pid: u32,
    start_id: u64,
    deadline: Option<u64>,
) -> Option<StartFacts> {
    let server = found.server.as_ref()?;
    let me = server.entries.iter().find(|e| e.id == found.entry)?;
    let (workspace, hosts) = (me.workspace.clone()?, me.hosts.clone()?);
    if !workspace.is_dir() {
        return None;
    }
    let home = home().ok()?;
    let source = &server.connector;
    let file = (source.source != "builtin").then_some(source.source.as_str());
    let loaded = connector::load(file.unwrap_or(&source.id), server.hosts_file.as_deref()).ok()?;
    if loaded.id != source.id {
        return None;
    }
    // Without a start time the native sources' `since` would be a guess: refuse instead.
    let started = unix_of(&st.harness.as_ref()?.started)?;
    let vars = super::env_vars();
    let own = server
        .server
        .clone()
        .map_or(super::Server::Off, super::Server::Url);
    let env = Env {
        vars: vars.clone(),
        workspace: workspace.clone(),
        resume: false,
        run_start: UNIX_EPOCH + Duration::from_secs(started),
        goalcup_home: home.clone(),
        server_host: own.host().unwrap_or_default().to_owned(),
    };
    let isolated = super::isolated_home(&home, &loaded.id);
    let _lock = HomeLock::take(&isolated).ok()?;
    let mut plan = connector::plan(&loaded, &env, &isolated);
    plan.hosts = hosts;
    // The values the live harness holds: its own start environment where the system shows it,
    // plus this terminal's plan, which may differ (extra values only add masking; PRD-05 section 10).
    // A name held with two values keeps both, the plan's under a suffixed name that passes the same gate.
    let mut harness_env = process_env(pid).unwrap_or_default();
    for (name, value) in &plan.env {
        match harness_env.get(name) {
            None => harness_env.insert(name.clone(), value.clone()),
            Some(live) if live != value => {
                harness_env.insert(format!("{name}_PLAN"), value.clone())
            }
            Some(_) => None,
        };
    }
    let values = launch_values(&workspace, &harness_env);
    // Only the shapes and rules can refuse; the values are built once, by the recorder.
    Masker::new(&plan.mask_rules, Vec::new()).ok()?;
    let part = Part {
        run: found.run.clone(),
        entry: found.entry.clone(),
        segment: me.segment?,
        entry_dir: found.entry_dir(),
        deadline: deadline.map(|d| d * 1000),
        local_wall: start::wall_ms(),
    };
    let mut facts = start::start_facts(&workspace, &vars, &mut plan, &values, part);
    facts.harness_version = me.harness_version.clone();
    // The first boot wrote these events; a recovered recorder does not write them again.
    facts.notes = Vec::new();
    facts.harness = Some(HarnessFacts {
        pid,
        program: plan.program.display().to_string(),
        started: st.harness.as_ref()?.started.clone(),
        start_id,
    });
    Some(facts)
}
