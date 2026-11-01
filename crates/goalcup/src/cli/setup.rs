//! `goalcup setup` without questions (PRD-10 section 7, seam S15). Normal lines go to stdout, an
//! error is one `say` line on stderr, and every path is quoted.
//!
//! - `--record-host HOST` adds one host to `record-hosts.txt` and does nothing else.
//! - Without `--yes` and without a terminal setup asks nothing, opens nothing and writes nothing:
//!   steps 1 and 4, then `Ready.` or the step to fix. With a terminal it picks the harness, opens
//!   it in the isolated home to sign in, writes `update_off`, asks the one `no_prompts` question.
//! - With `--yes` it also writes the connector's `update_off` and `no_prompts` rows into the
//!   isolated home and prints that home's path, the last folder path on stdout (S15: the pad takes
//!   the last quoted folder path as the home).

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead as _, IsTerminal as _, Write as _};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::{Server, awake, fail, folder, isolated_home, record_hosts};
use super::{setup_json, text};
use crate::connector::launch::{fill, make_dir, resolve_program};
use crate::connector::{self, Connector, Decision, Env, Form, HostList, LaunchPlan};
use crate::connector::{Refusal, SettingRow};
use crate::connector::{Source, builtin};
use crate::upload::client::ServerClient;

/// `$GOALCUP_HOME/setup.json`: the saved harness and the rows written: file, key and value.
#[derive(Serialize, Deserialize)]
struct Saved {
    v: u32,
    harness: String,
    #[serde(default)]
    update_off: Vec<Row>,
    #[serde(default)]
    no_prompts: Vec<Row>,
}

#[derive(Serialize, Deserialize)]
struct Row {
    file: PathBuf,
    key: String,
    value: String,
}

pub fn run(harness: Option<&str>, yes: bool, record_host: Option<&str>) -> ExitCode {
    let home = match super::home() {
        Ok(home) => home,
        Err(e) => return fail(e),
    };
    if let Some(host) = record_host {
        return add_host(&home, host);
    }
    let server = match super::server() {
        Ok(server) => server,
        Err(e) => return fail(e),
    };
    let vars = super::env_vars();
    let saved = super::saved_harness(&home);
    let ask = !yes && std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let id = match harness {
        Some(id) => Some(id.to_owned()),
        None if ask => pick(saved.as_deref(), &installed(&vars)),
        None => saved,
    };
    let Some(id) = id else {
        return fail(&text::no_harness(&installed(&vars)));
    };
    let c = match connector::load(&id, None) {
        Ok(c) => c,
        Err(e) => return fail(&e.to_string()),
    };
    let program = resolve_program(&c.program, &vars);
    if !program.is_absolute() {
        return fail(&text::not_installed(&c.name));
    }
    let fix = if yes || ask {
        match write_home(&c, &home, &server, vars.clone(), ask) {
            Ok(fix) => fix,
            Err(e) => return fail(&e),
        }
    } else {
        put(&text::harness_found(&c.name, None, &program));
        None
    };
    machine(&home, &server, &vars);
    let db = c.native.iter().any(|row| row.form == Form::Final);
    put(&text::masking(db.then_some(c.name.as_str())));
    match fix {
        None => {
            put(text::READY);
            ExitCode::SUCCESS
        }
        Some(step) => fail(&step),
    }
}

/// `--yes` (or `ask`, with a terminal): `plan` takes the home's lock, makes the home, probes the version and writes
/// `update_off`; then every `update_off` and `no_prompts` row is written (a row already holding
/// its value is left as it is), and `setup.json` saves those written. `Ok(Some(step))` is a
/// refusal to show after the machine lines; `Err` a home another run holds, where nothing was
/// written. `ask` opens the harness once to sign in and asks before the `no_prompts` rows.
fn write_home(
    c: &Connector,
    home: &Path,
    server: &Server,
    vars: BTreeMap<String, String>,
    ask: bool,
) -> Result<Option<String>, String> {
    let iso = isolated_home(home, &c.id);
    let env = Env {
        vars,
        // Setup has no project folder: a `{workspace}` row then reads in the isolated home, which
        // holds none of the participant's files, never in the folder setup runs from.
        workspace: iso.clone(),
        resume: false,
        run_start: SystemTime::now(),
        goalcup_home: home.to_owned(),
        server_host: server.host().unwrap_or_default().to_owned(),
    };
    let plan = connector::plan(c, &env, &iso);
    if let Some(r @ Refusal::HomeLocked { .. }) = &plan.refusal {
        return Err(text::refusal(r));
    }
    let version = plan.harness_version.as_deref().map(|v| {
        let number = |t: &&str| t.starts_with(|c: char| c.is_ascii_digit());
        v.split_whitespace().find(number).unwrap_or(v)
    });
    put(&text::harness_found(&c.name, version, &plan.program));
    if ask {
        open(&plan, &c.name);
    }
    let update_off = write_rows(&plan.home, &c.update_off);
    let no_prompts = if !ask || confirm(&format!("{} [Y/n] ", text::no_prompts_question(&c.name))) {
        write_rows(&plan.home, &c.no_prompts)
    } else {
        put(&text::no_prompts_left(&c.name));
        Vec::new()
    };
    let (update, prompts) = (!update_off.is_empty(), !no_prompts.is_empty());
    let saved = Saved {
        v: 1,
        harness: c.id.clone(),
        update_off,
        no_prompts,
    };
    let file = setup_json(home);
    if save(&file, &saved).is_err() {
        return Ok(Some(text::not_written(&file)));
    }
    put(&text::settings_set(&c.name, &plan.home, update, prompts));
    Ok(plan.refusal.as_ref().map(text::refusal))
}

/// One line typed at the terminal, trimmed; empty at end of input.
fn read_line(prompt: &str) -> String {
    let _ = write!(std::io::stdout(), "{prompt}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    line.trim().to_owned()
}

/// Step 1: Enter takes the saved harness; otherwise the id typed.
fn pick(saved: Option<&str>, installed: &[&str]) -> Option<String> {
    let found = installed.join(", ");
    let line = read_line(&format!(
        "Harness ({found}) [{}]: ",
        saved.unwrap_or("none")
    ));
    (!line.is_empty())
        .then_some(line)
        .or(saved.map(str::to_owned))
}

/// Step 2: the harness in the isolated home with its own launch data; a failure is a line.
fn open(plan: &LaunchPlan, name: &str) {
    read_line(&format!(
        "Press Enter to open {name} in its Goal Cup home. Sign in there, then quit it."
    ));
    let mut cmd = Command::new(&plan.program);
    cmd.args(&plan.argv).env_clear().envs(&plan.env);
    if cmd.current_dir(&plan.home).status().is_err() {
        put(&format!(
            "goalcup could not open {name}. Open it yourself and sign in there."
        ));
    }
}

/// Step 3: anything but `n` is yes.
fn confirm(prompt: &str) -> bool {
    !read_line(prompt).eq_ignore_ascii_case("n")
}

/// Writes each row into `home` and gives back those written; a row not written gets its line.
fn write_rows(home: &Path, rows: &[SettingRow]) -> Vec<Row> {
    let mut written = Vec::new();
    for row in rows {
        // Filled as connector::launch::write_settings fills it (keep the two in step), until
        // PRD-11 returns the file.
        let file = PathBuf::from(fill(&row.file, home, Path::new("")));
        if connector::write_settings(home, std::slice::from_ref(row)).is_empty() {
            let (key, value) = (row.key.clone(), row.value.clone());
            written.push(Row { file, key, value });
        } else {
            put(&text::not_set(&file, &row.key));
        }
    }
    written
}

/// `setup.json` written beside itself (0600), then renamed over.
fn save(file: &Path, saved: &Saved) -> std::io::Result<()> {
    let tmp = file.with_extension("json.tmp");
    let _ = fs::remove_file(&tmp);
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(&serde_json::to_vec_pretty(saved)?)?;
    fs::rename(&tmp, file)
}

/// Step 4: one warning line each, none of which holds back `Ready.` and none of which names a
/// folder, so the isolated home stays the last folder path printed.
fn machine(home: &Path, server: &Server, vars: &BTreeMap<String, String>) {
    let health = match server {
        Server::Off => Some(text::SERVER_OFF.to_owned()),
        Server::Url(base) => ServerClient::new(base, None)
            .json::<serde_json::Value, ()>("GET", "/v1/health", None)
            .err()
            .map(|e| text::server_unreachable(&e)),
    };
    let lines = [
        awake::missing(vars).map(text::no_sleep_lock),
        health,
        (!resolve_program("git", vars).is_absolute()).then(|| text::NO_GIT.to_owned()),
        folder::wsl(home, text::GOALCUP_FOLDER),
    ];
    for line in lines.into_iter().flatten() {
        put(&line);
    }
}

/// The built-in connector ids whose program is on `PATH`.
fn installed(vars: &BTreeMap<String, String>) -> Vec<&'static str> {
    let on_path = |c: Connector| resolve_program(&c.program, vars).is_absolute();
    let ids = builtin::CONNECTORS.iter().map(|(id, _)| *id);
    ids.filter(|id| connector::load(id, None).is_ok_and(on_path))
        .collect()
}

/// `--record-host`: PRD-11's one host check plus the built-in `never` list (as
/// `HostList::add_participant` reads the file), then one line in `record-hosts.txt` (0600), never
/// twice. A refusal shows the check's reason, never the name given, and writes nothing.
fn add_host(home: &Path, name: &str) -> ExitCode {
    let hosts = HostList::from_file(builtin::HOSTS.as_bytes(), "connectors/model-hosts.toml");
    // Fails closed: a built-in list that does not parse lets no host in.
    let hosts = hosts.as_ref().ok();
    let never = |n: &str| hosts.is_none_or(|l| l.decide(n) == Decision::Never);
    let host = match connector::hosts::check(name, Source::Participant) {
        Ok(host) if never(&host.name) => return fail(&text::host_refused(text::NEVER_HOST)),
        Ok(host) => host.name,
        Err(skipped) => return fail(&text::host_refused(&skipped.reason)),
    };
    let file = record_hosts(home);
    let old = fs::read_to_string(&file).unwrap_or_default();
    if !old.lines().any(|l| l.trim() == host) {
        make_dir(home);
        let gap = if old.is_empty() || old.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        let mut options = fs::OpenOptions::new();
        let opened = options.append(true).create(true).mode(0o600).open(&file);
        if opened.and_then(|mut f| writeln!(f, "{gap}{host}")).is_err() {
            return fail(&text::not_written(&file));
        }
    }
    put(&text::host_added(&host, &file));
    ExitCode::SUCCESS
}

/// One line on stdout; a closed stdout is not worth a panic.
fn put(line: &str) {
    let _ = writeln!(std::io::stdout(), "{line}");
}
