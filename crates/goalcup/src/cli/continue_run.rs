//! `goalcup continue` (PRD-10 section 3.1): close the last part if it died, tell the server, and
//! relaunch the harness as `start` does, with no new approval (the run's saved credential) and no
//! folder or repository checks. Whether to resume the harness's session comes from the sealed
//! `status.json` files of the run's earlier parts only; no harness session store is read.
//! Known gap: `server.json` keeps the connector id and the two file paths, not their contents, so
//! a file edited in place under the same path is not seen as changed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use serde::Deserialize;
use serde_json::json;

use super::approve::{self, Stop};
use super::start::{self, Part, Ready};
use super::{
    EntryFacts, Server, ServerJson, alive, awake, credential, entry_dir, fail, find, home, log,
    push, read_json, say, server_json, skill, status_json, text, watchdog_alive,
};
use crate::connector::{self, Env};
use crate::mask::{Masker, launch_values};
use crate::pipeline::status::{Mode, Status};
use crate::pipeline::{Closed, Mono, close_dead_entry};
use crate::upload::client::{ServerClient, ServerError, Token};
use crate::watchdog::Exit;

/// A harness that exits this fast with a non-zero code after a resume did not take the session.
const QUICK_EXIT_SECS: u64 = 30;

pub struct Args<'a> {
    pub run: Option<&'a str>,
    pub connector: Option<&'a Path>,
    pub hosts: Option<&'a Path>,
}

/// `goalcup continue`: the checks and the server call ([`prepare`]), then the hand-over to the
/// watchdog, as `start` does.
pub fn run(a: &Args) -> ExitCode {
    let home = match home() {
        Ok(h) => h,
        Err(e) => return fail(e),
    };
    let Ok(project) = std::env::current_dir() else {
        return fail(text::PROJECT_GONE);
    };
    match prepare(
        a,
        &home,
        &project,
        &super::env_vars(),
        &mut std::io::stdout(),
    ) {
        Err(stop) => {
            say(&stop.text);
            ExitCode::from(stop.code)
        }
        Ok(ready) => start::hand_over(ready),
    }
}

/// Whether to resume the harness's session: the earlier parts wrote at least one model request
/// or session file in their sealed `status.json`, unless the last part was itself a resume whose
/// harness exited non-zero within 30 s (D73). A part with no sealed `status.json` counts nothing.
pub fn resume_from_files(run_dir: &Path, entries: &[EntryFacts]) -> bool {
    let (mut written, mut failed) = (0, false);
    for e in entries {
        let st = read_json::<Status>(&status_json(&run_dir.join(&e.id)))
            .filter(|s| s.mode == Mode::Sealed);
        let seconds = |t: &str| super::unix_of(t);
        let quick = st.as_ref().and_then(|s| {
            let h = s.harness.as_ref()?;
            let ran = seconds(&s.updated)?.saturating_sub(seconds(&h.started)?);
            Some(matches!(h.exit, Some(Exit::Code(n)) if n != 0) && ran <= QUICK_EXIT_SECS)
        });
        written += st.as_ref().map_or(0, |s| {
            s.written.model_requests + s.written.counts.native_files
        });
        failed = e.resumed && quick.unwrap_or(false);
    }
    !failed && written > 0
}

fn refuse(text: impl Into<String>) -> Stop {
    Stop {
        text: text.into(),
        code: 1,
    }
}

/// A file flag must name what the run's first part used, if anything (`first` is `builtin` or the
/// absolute path the first part recorded).
fn same_file(what: &str, given: Option<&Path>, first: Option<&str>) -> Result<(), Stop> {
    let given = given.map(|p| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()));
    match (given, first) {
        (None, None) => Ok(()),
        (Some(g), Some(f)) if g.to_string_lossy() == f => Ok(()),
        (_, first) => Err(refuse(text::file_changed(
            what,
            &first.map_or("none".to_owned(), |f| format!("\"{f}\"")),
        ))),
    }
}

/// Everything before the hand-over: the files, the dead part, the resume rule, the plan, the
/// server call and `server.json`. `home` holds the runs; the run's own `server.json` says which
/// server to ask.
pub fn prepare(
    a: &Args,
    home: &Path,
    project: &Path,
    vars: &BTreeMap<String, String>,
    out: &mut dyn std::io::Write,
) -> Result<Ready, Stop> {
    let found = find(home, a.run).ok_or_else(|| {
        refuse(
            a.run
                .map_or_else(|| text::NO_RUN_YET.to_owned(), text::no_such_part),
        )
    })?;
    let (Some(sj), Some(last)) = (found.server.clone(), found.entries.last().cloned()) else {
        return Err(refuse(text::NOT_STARTED_HERE));
    };
    // An old file has no recorded folder: then there is nothing to compare. A recorded folder
    // that no longer exists (moved or renamed) cannot be gone back to, so it does not refuse.
    if let Some(ws) = sj.entries.last().and_then(|e| e.workspace.as_ref())
        && let Ok(recorded) = std::fs::canonicalize(ws)
        && std::fs::canonicalize(project).ok() != Some(recorded)
    {
        return Err(refuse(text::wrong_folder(
            &found.run,
            &ws.to_string_lossy(),
        )));
    }
    let base = sj.server.clone();
    let secret = match &base {
        Some(_) => match std::fs::read_to_string(credential(&found.run_dir)) {
            Ok(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
            _ => return Err(refuse(text::NOT_STARTED_HERE)),
        },
        None => None,
    };
    // The files first: a refusal here has touched nothing.
    if sj.mode == super::RunMode::Competition && (a.connector.is_some() || a.hosts.is_some()) {
        return Err(refuse(text::FILE_FLAGS_COMPETE));
    }
    let first = (sj.connector.source != "builtin").then_some(sj.connector.source.as_str());
    same_file("connector file", a.connector, first)?;
    let hosts = sj.hosts_file.as_ref().map(|p| p.to_string_lossy());
    same_file("host list", a.hosts, hosts.as_deref())?;
    let id = a
        .connector
        .map_or_else(|| sj.connector.id.clone(), |_| sj.connector.source.clone());
    let connector = connector::load(&id, a.hosts).map_err(|e| refuse(e.to_string()))?;
    if connector.id != sj.connector.id {
        return Err(refuse(text::file_changed(
            "connector file",
            &sj.connector.source,
        )));
    }
    let name = sj.connector.name.clone();

    // Step 1: the last part. Nothing is closed while anything of it still lives.
    let dir = entry_dir(home, &found.run, &last);
    let st = read_json::<Status>(&status_json(&dir));
    let mut lines = vec![text::run_kind(sj.mode).to_owned()];
    let sealed = st.as_ref().is_some_and(|s| s.mode == Mode::Sealed);
    if !sealed {
        let recording =
            watchdog_alive(&dir) || st.as_ref().is_some_and(|s| alive(s.pid, s.start_id));
        if recording {
            return Err(refuse(text::STILL_RECORDING));
        }
    }
    // A harness left running by `goalcup seal` (no recorder, no home lock) still owns its home.
    let running = st
        .as_ref()
        .and_then(|s| s.harness.as_ref())
        .is_some_and(|h| h.exit.is_none() && alive(h.pid, h.start_id));
    if running {
        return Err(refuse(text::harness_running(&name, sealed)));
    }
    if !sealed && dir.is_dir() {
        match close_dead_entry(&dir) {
            Err(_) => return Err(refuse(text::STILL_RECORDING)),
            Ok(Closed::Failed(kind)) => {
                return Err(refuse(text::not_closed(&found.run, &last, kind)));
            }
            Ok(Closed::Sealed(_)) => lines.push(text::RECOVERED.to_owned()),
            Ok(Closed::AlreadySealed) => {}
        }
    }

    // Step 3: resume or a new session; then the plan, which takes the harness home's lock (HF4).
    let resumed = resume_from_files(&found.run_dir, &sj.entries);
    let server = base.clone().map_or(Server::Off, Server::Url);
    let env = Env {
        vars: vars.clone(),
        workspace: project.to_path_buf(),
        resume: resumed,
        run_start: SystemTime::now(),
        goalcup_home: home.to_path_buf(),
        server_host: server.host().unwrap_or_default().to_owned(),
    };
    let mut plan = connector::plan(&connector, &env, &super::isolated_home(home, &connector.id));
    if let Some(r) = &plan.refusal {
        return Err(refuse(text::refusal(r)));
    }
    let values = launch_values(project, &plan.env);
    // Only the shapes and rules can refuse; the values are built once, by the recorder.
    if Masker::new(&plan.mask_rules, Vec::new()).is_err() {
        return Err(refuse(text::mask_refused()));
    }

    // Step 2: the server's next part of the same run, or a local one.
    let (entry, segment, left) = match (&base, secret) {
        (Some(base), Some(secret)) => {
            let body = json!({
                "previous_entry_id": last, "connector_id": connector.id,
                "connector_sha256": connector.sha256, "workspace": project.to_string_lossy(),
                "session_id": start::session_id(), "harness_version": approve::wire_version(plan.harness_version.as_deref()),
                "proxy_version": env!("CARGO_PKG_VERSION"), "os": std::env::consts::OS,
            });
            let client = ServerClient::new(base, Some(Token::new(secret)));
            let path = format!("/v1/runs/{}/continue", found.run);
            let got: Next = approve::send(&client, &path, &body, true).map_err(|e| match e {
                ServerError::Status { status: 410, .. } => refuse(text::TIME_UP),
                ServerError::Status { status: 409, .. } => refuse(text::ELSEWHERE),
                ServerError::Status {
                    status,
                    code,
                    message,
                    ..
                } if status < 500 => refuse(text::refused(if message.is_empty() {
                    &code
                } else {
                    &message
                })),
                e => Stop {
                    text: format!("{} Nothing was started.", text::server_unreachable(&e)),
                    code: 3,
                },
            })?;
            if !approve::is_id(&got.entry_id, 'e') {
                return Err(Stop {
                    text: text::bad_answer(),
                    code: 3,
                });
            }
            got.notes.print(out);
            let ns = got.seconds_left.saturating_mul(1_000_000_000);
            let left = Mono(Mono::now().ns().saturating_add(ns));
            (got.entry_id, got.segment, Some(left))
        }
        _ => (
            approve::random_id('e'),
            u32::try_from(sj.entries.len()).unwrap_or(u32::MAX),
            None,
        ),
    };
    let mut next: ServerJson = sj.clone();
    next.entries.push(EntryFacts {
        id: entry.clone(),
        harness_version: plan.harness_version.clone(),
        resumed,
        workspace: Some(project.to_path_buf()),
        hosts: Some(plan.hosts.clone()),
        segment: Some(segment),
    });
    let bytes = serde_json::to_vec(&next).unwrap_or_default();
    let path = server_json(&found.run_dir);
    approve::put(&path, &bytes).map_err(|_| Stop {
        text: text::not_written(&path),
        code: 3,
    })?;

    // Steps 7 to 9 of `start`: the sleep lock, the skill and the launch block.
    let new_dir = entry_dir(home, &found.run, &entry);
    let _ = std::fs::create_dir_all(&new_dir);
    let (lock, sleep_warning) = awake::take(vars);
    skill::install(
        connector.skills_dir.as_deref(),
        &plan.home,
        project,
        &new_dir,
    );
    let (local_wall, deadline) = start::deadline_ms(left);
    lines.push(if resumed {
        format!("Continuing {name}'s last session.")
    } else {
        format!("Starting a new {name} session: the last one could not be resumed.")
    });
    let (before, now) = (
        sj.entries.last().and_then(|e| e.harness_version.as_deref()),
        plan.harness_version.as_deref(),
    );
    if let (Some(a), Some(b)) = (before, now) {
        lines.extend((a != b).then(|| text::updated(&name, a, b)));
    }
    lines.extend(plan.effective.first_screen.clone());
    lines.extend(first_message(
        &found.run_dir,
        resumed,
        sj.repo.as_deref(),
        project,
        vars,
    ));
    lines.extend(deadline.map(|ms| text::ends_line(ms / 1000, super::now())));
    lines.push(text::upload_to(server.host()));
    lines.extend(plan.version_notes.clone());
    if plan.harness_version.is_none() {
        log::line(&new_dir, "version_unread", &[]);
        lines.push(text::version_unread(&name, server.host().is_some()));
    }
    for (what, file) in [
        ("Connector", first.map(PathBuf::from)),
        ("Host list", sj.hosts_file.clone()),
    ] {
        lines.extend(file.map(|f| text::from_file(what, &f)));
    }
    lines.push(text::harness_home(&name, &plan.home));
    lines.extend(sleep_warning);
    let _ = writeln!(out, "{}", lines.join("\n"));
    let part = Part {
        run: found.run.clone(),
        entry,
        segment,
        entry_dir: new_dir,
        deadline,
        local_wall,
    };
    let facts = start::start_facts(project, vars, &mut plan, &values, part);
    Ok(Ready {
        plan,
        facts,
        _lock: lock,
    })
}

/// A new session has not seen the first message: it is on the clipboard again. A start stopped
/// before it wrote one (Ctrl-C after the open, or no prompt) still owes the push lines.
fn first_message(
    run_dir: &Path,
    resumed: bool,
    repo: Option<&str>,
    project: &Path,
    vars: &BTreeMap<String, String>,
) -> Vec<String> {
    if resumed {
        return vec![];
    }
    let file = run_dir.join("first-message.txt");
    match std::fs::read_to_string(&file) {
        Ok(message) => vec![text::first_message(
            &file,
            push::clipboard::copy(&message, vars),
        )],
        Err(_) => repo
            .map(|r| {
                let mut lines = start::push_only(r, project, vars);
                lines.push(text::PROMPT_AFTER_PUSH.to_owned());
                lines
            })
            .unwrap_or_default(),
    }
}

#[derive(Deserialize)]
struct Next {
    entry_id: String,
    segment: u32,
    seconds_left: u64,
    #[serde(default)]
    notes: super::notes::Notes,
}
