//! `goalcup start` (PRD-10 section 3). [`prepare`] is everything before the hand-over: the
//! refusals and folder checks, the connector and its plan, the approval and the open run, the mask
//! values, the sleep lock, the skill and the launch block. It returns the plan and the start facts
//! (seam S17) for `watchdog::run`. Nothing before the launch times out sooner than the approval
//! code lives (P14): the only timers are `approve`'s. No token or credential is printed.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::to_value;

use super::approve::{self, Ask, Stop};
use super::{ConnectorSource, RunMode, Server, awake, fail, folder, push, say, text};
use crate::connector::{self, Env, LaunchPlan};
use crate::mask::{Masker, workspace_values};
use crate::pipeline::Mono;
use crate::watchdog::{CaPem, StartFacts};

// Declared here because `mod.rs` belongs to another task this slice; it can move there.
#[path = "skill.rs"]
pub mod skill;

/// The tiers PRD-12 writes, read at build time.
const TIERS: &str = include_str!("../../../../connectors/tiers.toml");

/// The start arguments. `repo` is `<owner>/<name>`; a run with a repo and `trial` is a test run.
#[derive(Default)]
pub struct Args<'a> {
    pub harness: Option<&'a str>,
    pub trial: bool,
    pub repo: Option<&'a str>,
    pub prompt: Option<&'a Path>,
    pub connector: Option<&'a Path>,
    pub hosts: Option<&'a Path>,
}

/// What `start` reads from the machine, so a test can give its own.
pub struct Ctx {
    pub home: PathBuf,
    pub project: PathBuf,
    pub vars: BTreeMap<String, String>,
    pub server: Server,
}

/// Everything the hand-over needs. `_lock` keeps the machine awake while this lives.
pub struct Ready {
    pub plan: LaunchPlan,
    pub facts: StartFacts,
    pub name: String,
    pub _lock: Option<awake::SleepLock>,
}

/// The saved harness id in `setup.json`.
#[derive(serde::Deserialize)]
struct Saved {
    harness: String,
}

/// `main.rs` still passes only these four; the rest reach [`go`] when it is wired.
/// `file_flags`: `--connector` or `--hosts` was given; `trial`: a practice or test run; `repo`:
/// `--repo` was given.
pub fn run(trial: bool, file_flags: bool, repo: bool, prompt: Option<&Path>) -> ExitCode {
    if file_flags && !trial {
        return fail(text::FILE_FLAGS_COMPETE);
    }
    go(&Args {
        trial,
        repo: repo.then_some(""),
        prompt,
        ..Args::default()
    })
}

pub fn go(a: &Args) -> ExitCode {
    if (a.connector.is_some() || a.hosts.is_some()) && !a.trial {
        return fail(text::FILE_FLAGS_COMPETE);
    }
    let home = match super::home() {
        Ok(h) => h,
        Err(e) => return fail(e),
    };
    let Ok(project) = std::env::current_dir() else {
        return fail(text::PROJECT_GONE);
    };
    let server = match super::server() {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let vars = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect();
    let ctx = Ctx {
        home,
        project,
        vars,
        server,
    };
    match prepare(a, &ctx, &mut std::io::stdout()) {
        Err(stop) => {
            say(&stop.text);
            ExitCode::from(stop.code)
        }
        // `watchdog::run(plan, facts)` is not on integration yet; its call replaces this line.
        Ok(ready) => {
            let _ = (&ready.plan, &ready.facts);
            say(&text::not_built("start"));
            ExitCode::from(3)
        }
    }
}

fn stop(text: String) -> Stop {
    Stop { text, code: 1 }
}

pub fn prepare(a: &Args, c: &Ctx, out: &mut dyn std::io::Write) -> Result<Ready, Stop> {
    let mode = match (a.trial, a.repo) {
        (true, None) => RunMode::Practice,
        (true, Some(_)) => RunMode::Test,
        (false, Some(_)) => RunMode::Competition,
        (false, None) => return Err(stop(text::NEEDS_REPO.to_owned())),
    };
    let prompt = a.prompt.filter(|p| p.exists()).map(push::read_prompt);
    let mut problems = match folder::check(&c.project, &c.home, a.repo.is_some(), a.prompt) {
        Err(problems) => problems,
        Ok(warnings) => {
            warnings.iter().for_each(|w| say(w));
            Vec::new()
        }
    };
    if let Some(Err(e)) = &prompt {
        problems.push(e.clone());
    }
    let prompt = prompt.and_then(Result::ok);
    if !problems.is_empty() {
        let list: String = problems.iter().map(|p| format!("\n  {p}")).collect();
        let (head, none) = (text::FOLDER_REFUSED, text::NOTHING_CREATED);
        return Err(stop(format!("{head}{list}\n{none}")));
    }
    // `main.rs` does not pass the repository's name yet (see [`run`]).
    if a.repo == Some("") {
        return Err(Stop {
            text: text::not_built("start"),
            code: 3,
        });
    }
    // Step 1: the connector and its plan. A plan refusal names a file and a key, never a value.
    let saved = super::read_json::<Saved>(&super::setup_json(&c.home)).map(|s| s.harness);
    let id = match (a.connector, a.harness.map(str::to_owned).or(saved)) {
        // `load` takes a file when the argument holds a `/` or ends `.toml`.
        (Some(p), _) => match p.to_string_lossy() {
            s if s.contains('/') || s.ends_with(".toml") => s.into_owned(),
            s => format!("./{s}"),
        },
        (None, Some(id)) => id,
        (None, None) => return Err(stop(text::NO_HARNESS_SAVED.to_owned())),
    };
    let connector = connector::load(&id, a.hosts).map_err(|e| stop(e.to_string()))?;
    let env = Env {
        vars: c.vars.clone(),
        workspace: c.project.clone(),
        resume: false,
        run_start: SystemTime::now(),
        goalcup_home: c.home.clone(),
        server_host: c.server.host().unwrap_or_default().to_owned(),
    };
    let mut plan = connector::plan(&connector, &env, &super::isolated_home(&c.home, &connector.id));
    if let Some(r) = &plan.refusal {
        return Err(stop(text::refusal(r)));
    }
    let name = connector.name.clone();
    // Step 6, before the approval, so a mask refusal never spends one.
    let values = workspace_values(&c.project);
    if Masker::new(&plan.mask_rules, values.clone()).is_err() {
        return Err(stop(text::MASK_REFUSED.to_owned()));
    }
    // Steps 3 to 5: the repository, the approval, the open run (a null run for `off`).
    let source = ConnectorSource {
        id: connector.id.clone(),
        name: name.clone(),
        source: a.connector.map_or("builtin".to_owned(), |p| p.display().to_string()),
    };
    let ask = Ask {
        mode,
        connector: &source,
        connector_sha256: &connector.sha256,
        workspace: &c.project,
        session_id: &session_id(),
        harness_version: plan.harness_version.as_deref(),
        repo: a.repo,
        github_api: approve::GITHUB_API,
        hosts_file: a.hosts,
        machine: serde_json::json!({
            "os": std::env::consts::OS,
            "cpus": std::thread::available_parallelism().map_or(0, |n| n.get()),
        }),
    };
    let opened = approve::begin(&c.home, &c.server, &ask, out)?;
    let entry_dir = super::entry_dir(&c.home, &opened.run, &opened.entry);
    let _ = std::fs::create_dir_all(&entry_dir);
    // Steps 7 and 8: the sleep lock, then the skill.
    let (lock, sleep_warning) = awake::take(&c.vars);
    skill::install(connector.skills_dir.as_deref(), &plan.home, &c.project, &entry_dir);
    // Step 9: the launch block.
    let deadline = opened.deadline.map(|m| {
        let left_ms = m.ns().saturating_sub(Mono::now().ns()) / 1_000_000;
        wall_ms().saturating_add(left_ms)
    });
    let mut lines = vec![text::run_kind(mode)];
    lines.extend(connector.first_screen.clone());
    lines.extend(first_message(a, c, prompt.as_deref(), &opened.run)?);
    lines.push(match deadline {
        Some(ms) => text::ends_line(ms / 1000, super::now()),
        None => text::NO_END_TIME.to_owned(),
    });
    lines.push(text::upload_to(c.server.host()));
    lines.push(tier_line(&name, plan.harness_version.as_deref(), &connector.id));
    for (what, file) in [("Connector", a.connector), ("Host list", a.hosts)] {
        lines.extend(file.map(|f| text::from_file(what, f)));
    }
    lines.push(text::harness_home(&name, &plan.home));
    lines.extend(sleep_warning);
    lines.push(text::recording_now(&name));
    let _ = writeln!(out, "{}", lines.join("\n"));
    plan.env.insert(
        "GOALCUP_TIME_LEFT_FILE".to_owned(),
        entry_dir.join("upload.json").to_string_lossy().into_owned(),
    );
    let facts = StartFacts {
        run: opened.run,
        entry: opened.entry,
        segment: 0,
        entry_dir,
        deadline,
        server_time: None,
        local_wall: wall_ms(),
        workspace: c.project.clone(),
        // Each of these five becomes the typed field of the same name (S17) with one edit.
        connector: to_value(&plan.connector).unwrap_or_default(),
        harness_version: plan.harness_version.clone(),
        keep: c.vars.get("GOALCUP_KEEP_PARTS").is_some_and(|v| v == "1"),
        build: None,
        hosts: to_value(&plan.hosts).unwrap_or_default(),
        hosts_sha256: plan.hosts.sha256().to_owned(),
        native: to_value(&plan.native).unwrap_or_default(),
        mask_rules: to_value(&plan.mask_rules).unwrap_or_default(),
        values: to_value(&values).unwrap_or_default(),
        // The watchdog mints the run CA and fills this in.
        ca: CaPem {
            cert: String::new(),
            key: String::new(),
        },
        prev: None,
        harness: None,
        null_since: None,
        notes: plan.notes.clone(),
    };
    Ok(Ready {
        plan,
        facts,
        name,
        _lock: lock,
    })
}

/// Section 8: the push block and the prompt, written to `first-message.txt` and put on the
/// clipboard; the lines the launch block shows for them.
fn first_message(a: &Args, c: &Ctx, prompt: Option<&str>, run: &str) -> Result<Vec<String>, Stop> {
    let block = a.repo.map(|repo| {
        let token = c.project.join(".env.local");
        let url = format!("https://github.com/{repo}");
        push::block(&url, "Goal Cup", text::SUPPORT_ADDRESS, ".env.local", &token)
    });
    let Some(prompt) = prompt else {
        let Some(block) = block else { return Ok(vec![]) };
        push::clipboard::copy(&block.join("\n"), &c.vars);
        let mut lines = vec![text::PUSH_FRAME.to_owned()];
        lines.extend(block.iter().map(|l| format!("  {l}")));
        return Ok(lines);
    };
    let dir = super::run_dir(&c.home, run);
    let file = dir.join("first-message.txt");
    let message = push::write_first_message(&dir, block.as_deref(), prompt).map_err(|e| Stop { text: e, code: 3 })?;
    Ok(vec![if push::clipboard::copy(&message, &c.vars) {
        text::first_message_ready(&file)
    } else {
        text::first_message_file(&file)
    }])
}

/// Section 6: the tier line of the connector, from `tiers.toml`.
fn tier_line(name: &str, probed: Option<&str>, id: &str) -> String {
    #[derive(serde::Deserialize, Default)]
    struct Row {
        tier: String,
        version: String,
    }
    let rows: BTreeMap<String, Row> = toml::from_str(TIERS).unwrap_or_default();
    let row = rows.get(id).map_or_else(Row::default, |r| Row {
        tier: r.tier.clone(),
        version: r.version.clone(),
    });
    let number = |t: &&str| t.starts_with(|c: char| c.is_ascii_digit());
    let version = probed.and_then(|v| v.split_whitespace().find(number));
    text::tier_line(name, version, &row.tier, &row.version)
}

fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// The run's idempotency key for the open: not secret.
fn session_id() -> String {
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    format!("s{:x}{:x}", t.as_nanos(), std::process::id())
}
