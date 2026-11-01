//! `goalcup recover [RUN]` (PRD-10 section 3.2, ledger R3b): bring back only the recorder when the
//! watchdog and the recorder are both dead and the harness still runs. The harness is never
//! signalled, restarted or waited for; no sleep lock is taken and no `plan.after` runs.

use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::to_value;

use super::log::{Fact, line};
use super::{
    alive, fail, find, goalcup_log, home, read_json, say, status_json, text, time_left, unix_of,
    watchdog_alive,
};
use crate::connector::{self, Env, HomeLock};
use crate::mask::workspace_values;
use crate::pipeline::status::{Mode, Status};
use crate::watchdog::{CaPem, HarnessFacts, NotRecovered, StartFacts, recover};

/// 0 recording is on (again), 1 refused, 3 recording could not be restarted.
pub fn run(arg: Option<String>) -> ExitCode {
    if super::var(time_left::FILE_ENV).is_some() {
        return fail("goalcup recover runs outside your agent. Run it in another terminal.");
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
    let harness = st
        .harness
        .as_ref()
        .filter(|h| h.exit.is_none() && alive(h.pid, h.start_id));
    if st.mode == Mode::Sealed {
        return fail(&format!(
            "This part has ended, so there is nothing to recover. To go on, run: goalcup continue {run}"
        ));
    }
    if deadline.is_some_and(|d| d <= now) {
        return fail("Time is up for this run. To close the part, run: goalcup seal");
    }
    if watchdog_alive(&dir) {
        return fail(WATCHDOG_ALIVE);
    }
    if alive(st.pid, st.start_id) {
        println!("Recording is on; there is nothing to recover.");
        return ExitCode::SUCCESS;
    }
    let Some(harness) = harness else {
        return fail(&gone(name, &run));
    };
    let Some(facts) = rebuild(&found, &st, harness.pid, harness.start_id, deadline) else {
        return broke(&format!(
            "goalcup could not read what it needs to restart recording. {}",
            support(&run, &dir)
        ));
    };
    let since = if st.off.since.is_some() {
        st.off.since.as_deref()
    } else {
        Some(st.updated.as_str())
    };
    let since = since.and_then(unix_of).unwrap_or(now);
    let port = st.port;
    let outcome = recover(&dir, facts);
    let word = match &outcome {
        Ok(r) if r.late => "late",
        Ok(_) => "recovered",
        Err(NotRecovered::PortTaken) => "port_taken",
        Err(NotRecovered::KeyMissing) => "key_missing",
        Err(_) => "not_started",
    };
    let at = UNIX_EPOCH + Duration::from_secs(since);
    line(
        &dir,
        "recover",
        &[
            ("off_since", Fact::Time(at)),
            ("port", Fact::N(u64::from(port))),
            ("outcome", Fact::Word(word)),
        ],
    );
    match outcome {
        Ok(r) => {
            let part = found
                .entries
                .iter()
                .position(|e| *e == found.entry)
                .map_or(1, |i| i + 1);
            let span = text::span_words(now.saturating_sub(since) * 1000);
            println!(
                "Recording is on again for part {part} of your run with {name}, on the same port."
            );
            println!(
                "It was off from {} to {} ({span}); this time is in the record as a gap.",
                text::local_time(since, true),
                text::local_time(now, true)
            );
            println!(
                "{name} was not restarted. This part ends when {name} exits or at the deadline."
            );
            if r.late {
                println!("The recorder is still starting.");
            }
            println!("Check it any time with: goalcup status");
            ExitCode::SUCCESS
        }
        Err(NotRecovered::PortTaken | NotRecovered::KeyMissing) => {
            let why = if word == "port_taken" {
                "the port is used by another program"
            } else {
                "the run certificate key is missing"
            };
            fail(&format!(
                "goalcup cannot reopen recording on port {port}: {why}. Your agent cannot reach the model service until this part ends. To go on, run: goalcup seal, then goalcup continue {run}"
            ))
        }
        Err(NotRecovered::Sealed) => fail(&format!(
            "This part has ended, so there is nothing to recover. To go on, run: goalcup continue {run}"
        )),
        Err(NotRecovered::WatchdogAlive) => fail(WATCHDOG_ALIVE),
        Err(NotRecovered::RecordingOn) => {
            println!("Recording is on; there is nothing to recover.");
            ExitCode::SUCCESS
        }
        Err(NotRecovered::HarnessGone) => fail(&gone(name, &run)),
        Err(NotRecovered::NotStarted) => broke(&format!(
            "goalcup could not restart recording. Your agent still cannot reach the model service. Run goalcup recover once more; if it fails again, run goalcup seal, then goalcup continue {run}, and write to {}. {}",
            text::SUPPORT_ADDRESS,
            support(&run, &dir)
        )),
    }
}

const WATCHDOG_ALIVE: &str =
    "goalcup is still running for this part and restarts recording itself. See: goalcup status";

fn gone(name: &str, run: &str) -> String {
    format!(
        "{name} is not running, so there is nothing to keep recording. To go on, run: goalcup continue {run}"
    )
}

fn support(run: &str, dir: &std::path::Path) -> String {
    format!(
        "Name run {run} to {}. {}.",
        text::SUPPORT_ADDRESS,
        text::details_at(&goalcup_log(dir))
    )
}

fn broke(text: &str) -> ExitCode {
    say(text);
    ExitCode::from(3)
}

/// The start facts again, from the run's own files (as `start`'s `prepare` made them). The plan
/// is made while this process holds the home lock itself: the plan then writes nothing in the
/// running harness's home and does not run the harness, and still reads its config hosts.
fn rebuild(
    found: &super::Found,
    st: &Status,
    pid: u32,
    start_id: u64,
    deadline: Option<u64>,
) -> Option<StartFacts> {
    let server = found.server.as_ref()?;
    let home = home().ok()?;
    let source = &server.connector;
    let file = (source.source != "builtin").then_some(source.source.as_str());
    let loaded = connector::load(file.unwrap_or(&source.id), server.hosts_file.as_deref()).ok()?;
    let started = st
        .harness
        .as_ref()
        .and_then(|h| unix_of(&h.started))
        .unwrap_or(0);
    let workspace = std::fs::read_link(format!("/proc/{pid}/cwd"))
        .ok()
        .or_else(|| std::env::current_dir().ok())?;
    let env = Env {
        vars: super::env_vars(),
        workspace: workspace.clone(),
        resume: false,
        run_start: UNIX_EPOCH + Duration::from_secs(started),
        goalcup_home: home.clone(),
        server_host: super::server().ok()?.host().unwrap_or_default().to_owned(),
    };
    let isolated = super::isolated_home(&home, &loaded.id);
    let _lock = HomeLock::take(&isolated).ok()?;
    let plan = connector::plan(&loaded, &env, &isolated);
    let values = workspace_values(&workspace);
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    Some(StartFacts {
        run: found.run.clone(),
        entry: found.entry.clone(),
        segment: u32::try_from(found.entries.iter().position(|e| *e == found.entry)?).ok()?,
        entry_dir: found.entry_dir(),
        deadline: deadline.map(|d| d * 1000),
        server_time: None,
        local_wall: wall,
        workspace,
        connector: to_value(&plan.connector).ok()?,
        harness_version: server
            .entries
            .iter()
            .find(|e| e.id == found.entry)?
            .harness_version
            .clone(),
        keep: env.vars.get("GOALCUP_KEEP_PARTS").is_some_and(|v| v == "1"),
        build: None,
        hosts: to_value(&plan.hosts).ok()?,
        hosts_sha256: plan.hosts.sha256().to_owned(),
        native: to_value(&plan.native).ok()?,
        mask_rules: to_value(&plan.mask_rules).ok()?,
        values: to_value(&values).ok()?,
        ca: CaPem {
            cert: String::new(),
            key: String::new(),
        },
        prev: None,
        harness: Some(HarnessFacts {
            pid,
            program: plan.program.display().to_string(),
            started: st.harness.as_ref()?.started.clone(),
            start_id,
        }),
        null_since: None,
        notes: Vec::new(),
    })
}
