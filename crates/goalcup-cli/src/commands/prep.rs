//! `goalcup prep` — from install to a finished practice run, in six steps, and safe to run again.
//!
//! Each step is `[ok]`, `[todo] <what to do>` or `[fail] <why>`, and the command exits 0 only
//! when all six are ok:
//!
//! 1. **login**: `server.json` names an account. Prep does not run the device flow itself.
//! 2. **harness**: one of the connectors that run in chat mode (`interactive = true`), picked
//!    with `--harness` or because it is the only one, found on `PATH`, its version shown.
//! 3. **harness home**: the harness is opened once in the home its runs will use, so the
//!    participant signs in and adds settings, MCP servers and skills there
//!    ([`crate::launch::prep_launch`]: no proxy, no capture, no daemon entry). The tracker
//!    cannot see a vendor sign-in, so this step is the person's word: it is ok once the harness
//!    they were given has exited 0, and that is recorded in `prep.json` (`home_ready`).
//! 4. **GitHub**: the repository from `--repo` is public, empty and pushable with the stored
//!    token ([`crate::account::github::check`]); with a terminal, the token is read unshown.
//! 5. **machine**: mains power, free space for a run's reservation, the server's and the
//!    provider's hosts reachable, `git` on `PATH`, under WSL the WSL2 version and a work folder,
//!    artifact root and configuration folder that are not on a Windows-host filesystem, and a
//!    sleep lock the machine can take (one it cannot take is said, and is never a `todo`).
//! 6. **practice**: a practice run of this harness whose every entry was sealed and uploaded.
//!    Prep never starts a run: it says `goalcup start --practice` and stops.
//!
//! # When prep acts, and when it only reports
//!
//! Only in a terminal, and never with `--json`, prep asks (the token) and launches (the
//! harness). `--json` prints `[{"step","state","human","say"}]` and nothing else; `human` marks
//! the steps that need the person (login, the harness home, the GitHub token, the practice run),
//! for the agent that is walking someone through the setup.
//!
//! # Re-running
//!
//! `<cfg>/prep.json` keeps what is settled: the harness, the repository, the work folder, and
//! the one fact that cannot be re-observed (`home_ready`). A second prep
//! skips the steps it has settled and keeps every value a flag does not change; changing the
//! harness starts its home again. The GitHub step is asked on every prep: the repository is
//! public, empty and pushable with the stored token, because a practice run never pushes.

use crate::CliError;
use crate::account::github::{self, GithubSetup};
use crate::connector::{HarnessConnector, harness_dirs};
use crate::prep_file::{PrepFile, read_prep_lenient, write_prep};
use crate::run::reserve::{LOW_DISK_BYTES, RUN_RESERVE_BYTES, START_MARGIN, artifact_root, human};
use crate::wsl_gate::Host;
use goalcup_protocol::harness::Origin;
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// The default work folder's name, beside the participant's other folders in their home.
pub const DEFAULT_WORK_ROOT_NAME: &str = "goalcup-work";

/// What the flags asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrepOptions {
    /// `--harness <id>`.
    pub harness: Option<String>,
    /// `--repo <url>`.
    pub repo: Option<String>,
    /// `--work-root <path>`: absolute.
    pub work_root: Option<PathBuf>,
    /// `--json`: print the steps as one JSON array, ask nothing, launch nothing.
    pub json: bool,
    /// `--dev`: accept a connector directory this build does not pin.
    pub dev: bool,
}

/// How a step stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Ok,
    Todo,
    Fail,
}

impl State {
    fn word(self) -> &'static str {
        match self {
            State::Ok => "ok",
            State::Todo => "todo",
            State::Fail => "fail",
        }
    }
}

/// One step of the report, and one element of `--json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Step {
    /// `login`, `harness`, `harness_home`, `github`, `machine` or `practice`.
    pub step: &'static str,
    pub state: State,
    /// The step needs the person (a sign-in, a token, a practice run), not only a command.
    pub human: bool,
    /// What was found, or what to do, or why it failed: one or two sentences.
    pub say: String,
}

/// The step ids, in order.
pub const STEP_LOGIN: &str = "login";
pub const STEP_HARNESS: &str = "harness";
pub const STEP_HARNESS_HOME: &str = "harness_home";
pub const STEP_GITHUB: &str = "github";
pub const STEP_MACHINE: &str = "machine";
pub const STEP_PRACTICE: &str = "practice";

fn step(step: &'static str, state: State, human: bool, say: impl Into<String>) -> Step {
    Step {
        step,
        state,
        human,
        say: say.into(),
    }
}

/// Everything prep reads from, or does to, the machine and the network, so that a test can
/// state the machine's facts instead of depending on the one it runs on.
pub trait Doors {
    /// Prep may ask and open the harness: stdin is a terminal this process holds in the
    /// foreground, and stdout is a terminal.
    fn has_terminal(&self) -> bool;
    /// Whether the machine runs on mains power; `None` when it cannot be told.
    fn on_ac_power(&self) -> Option<bool>;
    /// Free bytes on the volume holding the existing directory `dir`.
    fn free_bytes(&self, dir: &Path) -> std::io::Result<u64>;
    /// What the machine says about WSL: its version, and the filesystem of each folder.
    fn wsl(&self) -> Host;
    /// Every `(host, port)` answers a TCP connect; else the first that does not, in a phrase.
    fn hosts_reachable(&self, endpoints: &[(String, u16)]) -> Result<(), String>;
    /// Whether the machine can be kept awake (the lock is taken and let go at once).
    fn sleep_lock(&self) -> Result<(), String>;
    /// GitHub's API, as `account::github::check` takes it.
    fn github_api(&self) -> String;
    /// The token for `repo_url`: asked for on the terminal, and read without showing it.
    fn read_token(&self, repo_url: &str) -> Result<GithubSetup, CliError>;
    /// Wait for the participant to press Enter before the harness takes the terminal; `false`
    /// when the input ended without one (the participant chose not to go on).
    fn wait_for_enter(&self) -> bool;
    /// Open the harness in its home for the participant; whether it exited 0.
    fn open_harness(
        &self,
        connector: &HarnessConnector,
        cfg: &Path,
        env: &BTreeMap<String, String>,
    ) -> Result<bool, CliError>;
}

/// This machine.
pub struct Here;

impl Doors for Here {
    fn has_terminal(&self) -> bool {
        use goalcup_platform::terminal::{is_foreground, is_terminal};
        is_foreground(libc::STDIN_FILENO) && is_terminal(libc::STDOUT_FILENO)
    }

    fn on_ac_power(&self) -> Option<bool> {
        goalcup_platform::machine::on_ac_power()
    }

    fn free_bytes(&self, dir: &Path) -> std::io::Result<u64> {
        goalcup_platform::disk::free_bytes(dir)
    }

    fn wsl(&self) -> Host {
        Host::here()
    }

    fn hosts_reachable(&self, endpoints: &[(String, u16)]) -> Result<(), String> {
        crate::proxy::preflight::endpoints_reachable(
            endpoints.iter().map(|(host, port)| (host.as_str(), *port)),
        )
        .map_err(|unreachable| unreachable.reason())
    }

    fn sleep_lock(&self) -> Result<(), String> {
        goalcup_platform::awake::hold("goalcup prep is checking that the machine can stay awake")
            .map(drop)
    }

    fn github_api(&self) -> String {
        github::GITHUB_API.to_string()
    }

    fn read_token(&self, repo_url: &str) -> Result<GithubSetup, CliError> {
        // The echo is off before the question is on the screen, so nothing typed in answer to
        // it can be shown.
        let echo = goalcup_platform::terminal::EchoOff::take(libc::STDIN_FILENO);
        println!(
            "GitHub token for {repo_url} (a fine-grained token for that one repository, \
             Contents read and write; not shown):"
        );
        let _ = std::io::stdout().flush();
        let read = GithubSetup::read_token(repo_url, &mut std::io::stdin().lock());
        drop(echo);
        read
    }

    fn wait_for_enter(&self) -> bool {
        let mut line = String::new();
        matches!(std::io::stdin().lock().read_line(&mut line), Ok(n) if n > 0)
    }

    fn open_harness(
        &self,
        connector: &HarnessConnector,
        cfg: &Path,
        env: &BTreeMap<String, String>,
    ) -> Result<bool, CliError> {
        use crate::commands::start::{WorkspaceAt, claim_home_for_prep};
        let profile = &connector.loaded.manifest.capture;
        // The home every run of this connector keeps (its config home, D39, or its run
        // `HOME`, D59), held so no run is recording in it while the participant is signed in
        // there.
        let Some(home) = claim_home_for_prep(cfg, connector)? else {
            return Ok(true);
        };
        let executable = crate::launch::capture_executable(connector, profile, env)?;
        // A `HOME` that is made fresh for each run is made fresh for this, and removed after:
        // nothing the participant sets up lives there, it lives in the harness home. A kept run
        // `HOME` is the home itself, used as it is and never removed.
        let beside = cfg.join("prep-launch");
        crate::config::create_private_dir(&beside)?;
        let run = crate::commands::start::RunDir::create(
            &beside,
            profile.home.run_home,
            home.is_run_home.then_some(home.dir.as_path()),
            env,
            WorkspaceAt::InRunDir,
        )?;
        let opened = crate::config::create_private_dir(&run.workspace).and_then(|()| {
            crate::launch::prep_launch(
                connector,
                &crate::launch::PrepLaunch {
                    executable: &executable,
                    run_home: &run.home,
                    // With a kept run `HOME` the harness has no home variable: its state is
                    // under `HOME`, and the config home is the same folder.
                    config_home: if home.is_run_home {
                        &run.home
                    } else {
                        &home.dir
                    },
                    parent_env: env,
                },
                &run.workspace,
            )
        });
        let _ = run.remove();
        Ok(opened?.success())
    }
}

/// `goalcup prep`, with everything it reads handed in. Returns the exit code: 0 when all six
/// steps are ok, 1 otherwise. A mistyped flag is a refusal before anything is written.
pub fn prep_with(
    doors: &dyn Doors,
    cfg: &Path,
    connectors: &Path,
    env: &BTreeMap<String, String>,
    opts: &PrepOptions,
    out: &mut dyn Write,
) -> Result<u8, CliError> {
    // What the flags say, judged before anything is asked or written.
    let repo_flag = opts
        .repo
        .as_deref()
        .map(github::checked_repo_url)
        .transpose()?;
    if let Some(work_root) = &opts.work_root
        && !work_root.is_absolute()
    {
        return Err(CliError::Refused(format!(
            "the work folder {} is not an absolute path. Give the whole path, for example \
             `--work-root ~/goalcup-work` typed in full.",
            work_root.display()
        )));
    }
    let kept_work_root = read_prep_lenient(cfg).work_root;
    let work_root = match (&opts.work_root, kept_work_root.as_str()) {
        (Some(given), _) => given.clone(),
        (None, kept) if !kept.is_empty() => PathBuf::from(kept),
        (None, _) => crate::per_run_home::home_from(env)
            .map_err(|_| {
                CliError::Refused(
                    "$HOME is not set, so there is no default work folder. Give one with \
                     `--work-root <absolute path>`."
                        .to_string(),
                )
            })?
            .join(DEFAULT_WORK_ROOT_NAME),
    };

    crate::config::create_private_dir(cfg)?;
    let acting = !opts.json && doors.has_terminal();
    let mut file = read_prep_lenient(cfg);
    let before = file.clone();
    let mut report = Report {
        json: opts.json,
        steps: Vec::new(),
        out,
    };

    // 1. login
    report.push(login_step(cfg))?;

    // 2. harness
    let wanted = opts
        .harness
        .clone()
        .or_else(|| (!file.harness.is_empty()).then(|| file.harness.clone()));
    let (connector, harness) = harness_step(connectors, env, cfg, wanted.as_deref(), opts.dev);
    if let Some(c) = &connector {
        let id = c.loaded.manifest.id.clone();
        if file.harness != id {
            // Another harness has another home, signed in or not.
            file.harness = id;
            file.home_ready = false;
        }
    }
    report.push(harness)?;

    // 3. harness home
    let home = match &connector {
        Some(c) => home_step(doors, cfg, env, c, &mut file, acting, &mut report)?,
        None => step(
            STEP_HARNESS_HOME,
            State::Todo,
            true,
            "choose and install your agent first (the step above). Then `goalcup prep` opens it \
             so you can sign in and add your settings, MCP servers and skills",
        ),
    };
    report.push(home)?;
    save(cfg, &file, &before)?;

    // 4. GitHub
    if let Some(url) = &repo_flag {
        file.repo_url = url.clone();
    }
    let github_step = github_step(doors, cfg, &mut file, acting, &mut report)?;
    report.push(github_step)?;
    save(cfg, &file, &before)?;

    // 5. machine
    let machine = machine_step(doors, cfg, env, connector.as_ref(), &work_root);
    // A work folder is kept only once it has passed: a refused one must not replace a good one.
    if machine.work_root_ok {
        file.work_root = work_root.display().to_string();
    }
    report.push(machine.step)?;
    save(cfg, &file, &before)?;

    // 6. practice
    report.push(practice_step(
        cfg,
        connector.as_ref().map(|c| c.loaded.manifest.id.as_str()),
    ))?;

    let all_ok = report.steps.iter().all(|s| s.state == State::Ok);
    if report.json {
        let text = serde_json::to_string_pretty(&report.steps)?;
        put(report.out, &format!("{text}\n"))?;
    } else if all_ok {
        let id = connector
            .as_ref()
            .map_or("<id>", |c| c.loaded.manifest.id.as_str());
        put(
            report.out,
            &format!(
                "goalcup prep: ready. Next: goalcup start --harness {id}, inside the start \
                 window.\n"
            ),
        )?;
    } else {
        let waiting = report.steps.iter().filter(|s| s.state != State::Ok).count();
        // The last line on the screen is the next action: the first step that needs the person.
        let next = report
            .steps
            .iter()
            .find(|s| s.state != State::Ok)
            .map(|s| s.say.as_str())
            .unwrap_or_default();
        put(
            report.out,
            &format!(
                "goalcup prep: {waiting} of {} steps need you. Next: {next}\n",
                report.steps.len()
            ),
        )?;
    }
    Ok(u8::from(!all_ok))
}

/// The steps so far, printed as they come in text mode and held for the JSON array otherwise.
struct Report<'a> {
    json: bool,
    steps: Vec<Step>,
    out: &'a mut dyn Write,
}

impl Report<'_> {
    fn push(&mut self, step: Step) -> Result<(), CliError> {
        if !self.json {
            put(
                self.out,
                &format!("[{}] {}: {}\n", step.state.word(), step.step, step.say),
            )?;
        }
        self.steps.push(step);
        Ok(())
    }

    /// A line to the participant between steps, in text mode only.
    fn tell(&mut self, line: &str) -> Result<(), CliError> {
        if self.json {
            return Ok(());
        }
        put(self.out, &format!("{line}\n"))?;
        self.out.flush().map_err(|source| CliError::Io {
            path: "standard output".to_string(),
            source,
        })
    }
}

/// Write the prep file when it holds something that differs from what was read.
fn save(cfg: &Path, now: &PrepFile, before: &PrepFile) -> Result<(), CliError> {
    if now != before && *now != PrepFile::default() {
        write_prep(cfg, now)?;
    }
    Ok(())
}

// ------------------------------------------------------------------ 1. login

fn login_step(cfg: &Path) -> Step {
    match crate::server::read_server_config(cfg) {
        Ok(Some(server)) if server.account.is_some() => {
            let mut say = format!(
                "this computer is connected ({})",
                server.account.unwrap_or_default()
            );
            // The address is news only when it is not the Goal Cup server.
            if server.url.trim_end_matches('/') != crate::server::DEFAULT_SERVER_URL {
                say.push_str(&format!("; runs upload to {}", server.url));
            }
            step(STEP_LOGIN, State::Ok, false, say)
        }
        Ok(_) => step(STEP_LOGIN, State::Todo, true, "run `goalcup login`"),
        Err(error) => step(STEP_LOGIN, State::Fail, true, error.to_string()),
    }
}

// ------------------------------------------------------------------ 2. harness

/// Every connector in `connectors` that runs in chat mode and loads, in id order.
fn chat_connectors(connectors: &Path, dev: bool) -> Result<Vec<HarnessConnector>, CliError> {
    let origin = if dev { Origin::Dev } else { Origin::Pinned };
    let mut found = Vec::new();
    for dir in harness_dirs(connectors)? {
        // One that does not load is `goalcup doctor`'s to report; it cannot be picked here.
        if let Ok(loaded) = goalcup_protocol::harness::load_discovered_dir(&dir, origin)
            && loaded.manifest.capture.interactive
        {
            found.push(HarnessConnector { dir, loaded });
        }
    }
    found.sort_by(|a, b| a.loaded.manifest.id.cmp(&b.loaded.manifest.id));
    Ok(found)
}

fn harness_step(
    connectors: &Path,
    env: &BTreeMap<String, String>,
    cfg: &Path,
    wanted: Option<&str>,
    dev: bool,
) -> (Option<HarnessConnector>, Step) {
    let fail = |say: String| (None, step(STEP_HARNESS, State::Fail, false, say));
    let chat = match chat_connectors(connectors, dev) {
        Ok(chat) => chat,
        Err(error) => return fail(error.to_string()),
    };
    let ids: Vec<&str> = chat.iter().map(|c| c.loaded.manifest.id.as_str()).collect();
    let chosen = match wanted {
        Some(id) => match chat.iter().position(|c| c.loaded.manifest.id == id) {
            Some(at) => at,
            None if ids.is_empty() => {
                return fail(format!(
                    "no connector `{id}` runs in chat mode, and this build ships none that does"
                ));
            }
            None => {
                return fail(format!(
                    "no chat-mode connector is called `{id}`. Choose one of: {}.",
                    ids.join(", ")
                ));
            }
        },
        None => match chat.len() {
            0 => {
                return fail(
                    "this build ships no harness that runs in chat mode, so there is none to prepare"
                        .to_string(),
                );
            }
            1 => 0,
            _ => {
                return (
                    None,
                    step(
                        STEP_HARNESS,
                        State::Todo,
                        false,
                        format!(
                            "choose your agent: {}",
                            ids.iter()
                                .map(|id| format!("`goalcup prep --harness {id}`"))
                                .collect::<Vec<_>>()
                                .join(" or ")
                        ),
                    ),
                );
            }
        },
    };
    let connector = chat.into_iter().nth(chosen).expect("a chosen connector");
    let id = connector.loaded.manifest.id.clone();
    let check = crate::commands::doctor::executable_check(&connector, env);
    if check.level == crate::preflight::Level::Fail {
        let mut say = check.detail;
        if !check.fix.is_empty() {
            say.push(' ');
            say.push_str(&check.fix);
        }
        return (Some(connector), step(STEP_HARNESS, State::Fail, false, say));
    }
    let profile = &connector.loaded.manifest.capture;
    let (path, version) = match crate::launch::capture_executable(&connector, profile, env) {
        Ok(executable) => {
            let probe = cfg.join(format!("prep-version-{:016x}", rand::random::<u64>()));
            let version = crate::harness_version::reported_version_in_scratch_home(
                &executable,
                &profile.executable.version_args,
                &probe,
            );
            (executable.path().to_string(), version)
        }
        Err(error) => {
            return (
                Some(connector),
                step(STEP_HARNESS, State::Fail, false, error.to_string()),
            );
        }
    };
    let say = format!("{id} {version} ({path})");
    (Some(connector), step(STEP_HARNESS, State::Ok, false, say))
}

// ------------------------------------------------------------------ 3. the harness home

fn home_step(
    doors: &dyn Doors,
    cfg: &Path,
    env: &BTreeMap<String, String>,
    connector: &HarnessConnector,
    file: &mut PrepFile,
    acting: bool,
    report: &mut Report<'_>,
) -> Result<Step, CliError> {
    let id = &connector.loaded.manifest.id;
    let human = |state, say: String| step(STEP_HARNESS_HOME, state, true, say);
    // The home the runs keep: its config home (D39) or its run `HOME` (D59). A connector whose
    // homes are all made fresh for each run has none to set up.
    let Some(home) = crate::commands::start::prep_home_leaf(cfg, connector) else {
        return Ok(step(
            STEP_HARNESS_HOME,
            State::Ok,
            false,
            format!("{id} needs no set-up here: each run gets a fresh home"),
        ));
    };
    // The person's word counts only while the home they set up is still there.
    if file.home_ready && home.is_dir() {
        return Ok(step(
            STEP_HARNESS_HOME,
            State::Ok,
            false,
            format!("{id} is set up in its Goal Cup home. Your runs use it."),
        ));
    }
    if !acting {
        return Ok(human(
            State::Todo,
            format!(
                "run `goalcup prep` in a terminal. It opens {id} in its Goal Cup home ({}). \
                 Sign in there, add your settings, MCP servers and skills, then exit {id} the \
                 normal way",
                home.display()
            ),
        ));
    }
    report.tell(&format!(
        "Next, {id} opens here in its own Goal Cup home (not your usual setup). Nothing is \
         recorded.\n  1. Sign in.\n  2. Add the settings, MCP servers and skills you want your \
         agent to have.\n  3. Exit {id} the normal way.\nPress Enter to open {id}, or Ctrl-C to stop."
    ))?;
    if !doors.wait_for_enter() {
        return Ok(human(
            State::Todo,
            format!("{id} was not opened. Run `goalcup prep` again, then press Enter"),
        ));
    }
    match doors.open_harness(connector, cfg, env) {
        Ok(true) => {
            file.home_ready = true;
            Ok(step(
                STEP_HARNESS_HOME,
                State::Ok,
                false,
                format!("{id} is set up in its Goal Cup home. Your runs use it."),
            ))
        }
        Ok(false) => Ok(human(
            State::Todo,
            format!(
                "{id} did not exit normally. Run `goalcup prep` again, then exit {id} the \
                 normal way"
            ),
        )),
        Err(error) => Ok(human(State::Fail, error.to_string())),
    }
}

// ------------------------------------------------------------------ 4. GitHub

fn github_step(
    doors: &dyn Doors,
    cfg: &Path,
    file: &mut PrepFile,
    acting: bool,
    report: &mut Report<'_>,
) -> Result<Step, CliError> {
    let fail = |say: String| step(STEP_GITHUB, State::Fail, true, say);
    if file.repo_url.is_empty() {
        return Ok(step(
            STEP_GITHUB,
            State::Todo,
            true,
            "create a new, empty, public repository on GitHub (no README, license or \
             .gitignore). Then run: goalcup prep --repo https://github.com/<owner>/<repo>",
        ));
    }
    let repo = file.repo_url.clone();
    let api = doors.github_api();
    // The token stored for this repository, if there is one.
    let stored = GithubSetup::load(cfg)
        .ok()
        .filter(|setup| same_repo(&setup.repo_url, &repo));
    // Public, empty and pushable, asked on every prep: a practice run never pushes, so the
    // repository is still empty until the competition run, and a token that expired or was
    // revoked, or a repository made private, is found now and not on the day.
    let mut failed = None;
    if let Some(setup) = &stored {
        match github::check_access(&api, setup) {
            Ok(()) => {
                return Ok(match github::check_empty(&api, setup) {
                    Ok(()) => step(
                        STEP_GITHUB,
                        State::Ok,
                        false,
                        format!("{repo} is public and empty, and the stored token can push to it"),
                    ),
                    // The token is fine; a new one would not empty the repository.
                    Err(error) => fail(error.to_string()),
                });
            }
            Err(error) => failed = Some(error.to_string()),
        }
    }
    if !acting {
        return Ok(match failed {
            Some(why) => fail(why),
            None => step(
                STEP_GITHUB,
                State::Todo,
                true,
                format!(
                    "run `goalcup prep` in a terminal and paste a GitHub token for {repo} when \
                     asked (a fine-grained token for that one repository, Contents set to read \
                     and write; it is not shown)"
                ),
            ),
        });
    }
    if let Some(why) = &failed {
        report.tell(&format!("The stored token did not pass: {why}"))?;
    }
    let setup = match doors.read_token(&repo) {
        Ok(setup) => setup,
        Err(error) => return Ok(fail(error.to_string())),
    };
    // Stored only once it has passed the whole check.
    if let Err(error) = github::check(&api, &setup) {
        return Ok(fail(error.to_string()));
    }
    setup.store(cfg)?;
    Ok(step(
        STEP_GITHUB,
        State::Ok,
        false,
        format!("{repo} is public and empty, and the token you gave can push to it; it is stored"),
    ))
}

/// Two repository addresses in the one spelling they are compared in.
fn same_repo(a: &str, b: &str) -> bool {
    let spell = |url: &str| {
        let url = url.trim().trim_end_matches('/');
        url.strip_suffix(".git").unwrap_or(url).to_ascii_lowercase()
    };
    spell(a) == spell(b)
}

// ------------------------------------------------------------------ 5. the machine

struct Machine {
    step: Step,
    /// The work folder passed its own check, and may be kept in `prep.json`.
    work_root_ok: bool,
}

/// Free bytes a run needs to start at all, and to start without a warning.
const FAIL_BELOW: u64 = RUN_RESERVE_BYTES + START_MARGIN;
const OK_FROM: u64 = RUN_RESERVE_BYTES + LOW_DISK_BYTES;

fn machine_step(
    doors: &dyn Doors,
    cfg: &Path,
    env: &BTreeMap<String, String>,
    connector: Option<&HarnessConnector>,
    work_root: &Path,
) -> Machine {
    let mut worst = State::Ok;
    let mut problems: Vec<String> = Vec::new();
    let mut fine: Vec<String> = Vec::new();
    let mut note = |state: State, sentence: String| {
        if state == State::Ok {
            fine.push(sentence);
        } else {
            worst = match (worst, state) {
                (State::Fail, _) | (_, State::Fail) => State::Fail,
                _ => State::Todo,
            };
            problems.push(sentence);
        }
    };

    match doors.on_ac_power() {
        Some(false) => note(
            State::Todo,
            "the machine is on battery: plug it in, a run lasts a week".to_string(),
        ),
        Some(true) => note(State::Ok, "on mains power".to_string()),
        None => {}
    }

    let root = artifact_root(cfg);
    let near = root
        .ancestors()
        .find(|above| above.is_dir())
        .unwrap_or(&root)
        .to_path_buf();
    match doors.free_bytes(&near) {
        Ok(free) if free < FAIL_BELOW => note(
            State::Fail,
            format!(
                "only {} is free where runs are kept ({}); a run reserves {} and needs {} to \
                 start. Free up space",
                human(free),
                root.display(),
                human(RUN_RESERVE_BYTES),
                human(FAIL_BELOW)
            ),
        ),
        Ok(free) if free < OK_FROM => note(
            State::Todo,
            format!(
                "{} is free where runs are kept ({}); a run reserves {} and warns below {} \
                 beyond it, so free up space to have {}",
                human(free),
                root.display(),
                human(RUN_RESERVE_BYTES),
                human(LOW_DISK_BYTES),
                human(OK_FROM)
            ),
        ),
        Ok(free) => note(State::Ok, format!("{} free", human(free))),
        Err(error) => note(
            State::Todo,
            format!(
                "the free space where runs are kept ({}) could not be read: {error}",
                root.display()
            ),
        ),
    }

    let mut hosts: Vec<(String, u16)> = Vec::new();
    if let Ok(Some(server)) = crate::server::read_server_config(cfg) {
        hosts.push(endpoint_of(&server.url));
    }
    if let Some(c) = connector {
        let profile = &c.loaded.manifest.capture;
        hosts.extend(profile.hosts.iter().map(|h| (h.host.clone(), 443)));
        hosts.extend(profile.auth_hosts.iter().map(|h| (h.clone(), 443)));
    }
    hosts.retain(|(host, _)| !host.is_empty());
    hosts.sort();
    hosts.dedup();
    match doors.hosts_reachable(&hosts) {
        Ok(()) => note(
            State::Ok,
            "the server and the provider are reachable".to_string(),
        ),
        Err(why) => note(
            State::Fail,
            format!(
                "{why}. Check the network; a firewall such as Little Snitch may be holding \
                 goalcup's connections"
            ),
        ),
    }

    if crate::preflight::resolve_on_path("git", env).is_some() {
        note(State::Ok, "git is installed".to_string());
    } else {
        note(
            State::Fail,
            "git is not on PATH: install it (https://git-scm.com), a run is a clone you push from"
                .to_string(),
        );
    }

    // Under WSL: WSL1 cannot run this, and a Windows-host filesystem is refused for the work
    // folder, the artifact root and the configuration folder. The work folder is judged alone,
    // because only a refused one must not be kept.
    let host = doors.wsl();
    let wsl = host.is_wsl();
    let work_check = host.check(&[("work folder", work_root)]);
    if let Err(why) = &work_check {
        note(State::Fail, why.clone());
    }
    // Each run is a clone, and the daemon refuses a run folder inside a Git repository. Found
    // here, it costs a command; found at `goalcup start`, it costs an approval and a clone.
    let work_in_git = git_repository_of(work_root);
    if let Some(repo) = &work_in_git {
        let default = crate::per_run_home::home_from(env)
            .ok()
            .map(|home| home.join(DEFAULT_WORK_ROOT_NAME));
        note(
            State::Fail,
            work_in_git_message(work_root, repo, default.as_deref()),
        );
    }
    if host.version() != Some(1) {
        let artifacts = artifact_root(cfg);
        if let Err(why) = host.check(&[
            ("artifact root", artifacts.as_path()),
            ("config folder", cfg),
        ]) {
            note(State::Fail, why);
        }
    }

    // A sleep lock the machine cannot take never makes the step not-ok (WSL2 never has one, nor
    // does a Linux without `systemd-inhibit`): the person is told to keep the machine awake
    // themselves, whatever else the step says.
    let keep_awake = match doors.sleep_lock() {
        Ok(()) => {
            note(State::Ok, "the machine can be kept awake".to_string());
            None
        }
        Err(_) if wsl => Some(
            "Windows decides when this machine sleeps: in Windows Settings, System, Power, set \
             sleep to Never for the week of your run"
                .to_string(),
        ),
        Err(why) => Some(format!(
            "{why}; keep it awake yourself for the week of your run"
        )),
    };

    let mut say = if problems.is_empty() {
        fine.join(", ")
    } else {
        problems.join(". ")
    };
    if let Some(keep_awake) = keep_awake {
        if !say.is_empty() {
            say.push_str(". ");
        }
        say.push_str(&keep_awake);
    }
    Machine {
        step: step(STEP_MACHINE, worst, false, say),
        work_root_ok: work_check.is_ok() && work_in_git.is_none(),
    }
}

/// The nearest folder at or above `path` that holds a `.git` entry, file or directory (a linked
/// worktree has a file): the same rule as the daemon's `git_worktree_of`, which the CLI cannot
/// call (the CLI never links the daemon). A folder that does not exist yet is judged by its
/// parents.
fn git_repository_of(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|dir| dir.join(".git").symlink_metadata().is_ok())
        .map(Path::to_path_buf)
}

/// The refusal for a work folder inside a Git repository: what is wrong, then the command that
/// fixes it. The suggested folder is `~/goalcup-work` unless that folder is inside a repository
/// too (a home folder that is itself a repository), when only the shape of the fix is given.
fn work_in_git_message(work_root: &Path, repo: &Path, default: Option<&Path>) -> String {
    let fix = match default {
        Some(folder) if git_repository_of(folder).is_none() => {
            format!("goalcup prep --work-root ~/{DEFAULT_WORK_ROOT_NAME}")
        }
        _ => "goalcup prep --work-root <full path of a folder outside any repository>".to_string(),
    };
    format!(
        "Your work folder {} is inside a Git repository ({}). Choose a folder outside any \
         repository: {fix}",
        work_root.display(),
        repo.display()
    )
}

/// The host and port of a server URL: the port the URL names, else the scheme's (443 for
/// https, 80 for http), without scheme or user info.
fn endpoint_of(url: &str) -> (String, u16) {
    let (scheme, rest) = url.split_once("://").unwrap_or(("https", url));
    let default = if scheme.eq_ignore_ascii_case("http") {
        80
    } else {
        443
    };
    let authority = rest.split('/').next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or("");
    let (host, after) = match authority.strip_prefix('[') {
        Some(inner) => inner.split_once(']').unwrap_or((inner, "")),
        None => match authority.split_once(':') {
            Some((host, _)) => (host, &authority[host.len()..]),
            None => (authority, ""),
        },
    };
    let port = after
        .strip_prefix(':')
        .and_then(|port| port.parse().ok())
        .unwrap_or(default);
    (host.to_string(), port)
}

// ------------------------------------------------------------------ 6. practice

fn practice_step(cfg: &Path, harness: Option<&str>) -> Step {
    let id = harness.unwrap_or("<id>");
    let start = format!("goalcup start --harness {id} --practice");
    let Some(harness) = harness else {
        return step(
            STEP_PRACTICE,
            State::Todo,
            true,
            format!("choose your agent first. Then do a practice run: {start}"),
        );
    };
    let spool = cfg.join(crate::upload::UPLOADS_DIR_NAME);
    let mut practice: Vec<_> = crate::run::all(cfg)
        .into_iter()
        .filter(|run| run.practice && run.connector_id == harness && run.server_url.is_some())
        .collect();
    practice.sort_by(|a, b| (b.started_at_unix, &b.run_id).cmp(&(a.started_at_unix, &a.run_id)));
    let unsent = |run: &crate::run::RunRecord| {
        run.entries
            .iter()
            .find(|entry| !crate::upload::uploader::is_sent(&spool, &entry.entry_id))
            .map(|entry| entry.entry_id.clone())
    };
    if practice
        .iter()
        .any(|run| !run.entries.is_empty() && unsent(run).is_none())
    {
        return step(
            STEP_PRACTICE,
            State::Ok,
            false,
            "your practice run is done and uploaded".to_string(),
        );
    }
    if let Some(entry) = practice.first().and_then(unsent) {
        return step(
            STEP_PRACTICE,
            State::Todo,
            true,
            format!(
                "your last practice run is not fully uploaded. Run: goalcup upload {entry}. Or \
                 do a new practice run: {start}"
            ),
        );
    }
    step(
        STEP_PRACTICE,
        State::Todo,
        true,
        format!(
            "do a 10-minute practice run: {start}. It does not count, never pushes, and never \
             replaces your competition run. Prep does not start it"
        ),
    )
}

/// Every byte this command prints goes through here, so a closed pipe is a [`CliError::Io`]
/// naming the stream.
fn put(out: &mut dyn Write, text: &str) -> Result<(), CliError> {
    out.write_all(text.as_bytes())
        .map_err(|source| CliError::Io {
            path: "standard output".to_string(),
            source,
        })
}

// ------------------------------------------------------------------ the test door

/// The machine's facts as a test states them, in `GOALCUP_TEST_PREP`: comma-separated
/// `ac=on|battery|unknown`, `free=<bytes>`, `wsl=0|1|2`, `fs=<path prefix>=<filesystem type>`
/// (repeatable), `hosts=ok|<phrase>`, `awake=ok|<phrase>` and
/// `github=<api base>`; everything it does not name is this machine's. A shipped build has no
/// such door.
#[cfg(feature = "test-support")]
pub struct Injected {
    ac: Option<Option<bool>>,
    free: Option<u64>,
    host: Host,
    hosts: Option<Result<(), String>>,
    awake: Option<Result<(), String>>,
    github: Option<String>,
}

#[cfg(feature = "test-support")]
impl Injected {
    /// The environment variable that names the facts.
    pub const VAR: &'static str = "GOALCUP_TEST_PREP";

    pub fn parse(spec: &str) -> Option<Injected> {
        let mut door = Injected {
            ac: None,
            free: None,
            host: Host::here(),
            hosts: None,
            awake: None,
            github: None,
        };
        for part in spec.split(',').filter(|p| !p.is_empty()) {
            let (key, value) = part.split_once('=')?;
            let answer = |value: &str| match value {
                "ok" => Ok(()),
                phrase => Err(phrase.replace('_', " ")),
            };
            match key {
                "ac" => {
                    door.ac = Some(match value {
                        "on" => Some(true),
                        "battery" => Some(false),
                        _ => None,
                    })
                }
                "free" => door.free = Some(value.parse().ok()?),
                "wsl" | "fs" => {
                    if !door.host.state(key, value) {
                        return None;
                    }
                }
                "hosts" => door.hosts = Some(answer(value)),
                "awake" => door.awake = Some(answer(value)),
                "github" => door.github = Some(value.to_string()),
                _ => return None,
            }
        }
        Some(door)
    }
}

#[cfg(feature = "test-support")]
impl Doors for Injected {
    fn has_terminal(&self) -> bool {
        Here.has_terminal()
    }

    fn on_ac_power(&self) -> Option<bool> {
        self.ac.unwrap_or_else(|| Here.on_ac_power())
    }

    fn free_bytes(&self, dir: &Path) -> std::io::Result<u64> {
        match self.free {
            Some(free) => Ok(free),
            None => Here.free_bytes(dir),
        }
    }

    fn wsl(&self) -> Host {
        self.host.clone()
    }

    fn hosts_reachable(&self, endpoints: &[(String, u16)]) -> Result<(), String> {
        match &self.hosts {
            Some(answer) => answer.clone(),
            None => Here.hosts_reachable(endpoints),
        }
    }

    fn sleep_lock(&self) -> Result<(), String> {
        match &self.awake {
            Some(answer) => answer.clone(),
            None => Here.sleep_lock(),
        }
    }

    fn github_api(&self) -> String {
        self.github.clone().unwrap_or_else(|| Here.github_api())
    }

    fn read_token(&self, repo_url: &str) -> Result<GithubSetup, CliError> {
        Here.read_token(repo_url)
    }

    fn wait_for_enter(&self) -> bool {
        Here.wait_for_enter()
    }

    fn open_harness(
        &self,
        connector: &HarnessConnector,
        cfg: &Path,
        env: &BTreeMap<String, String>,
    ) -> Result<bool, CliError> {
        Here.open_harness(connector, cfg, env)
    }
}

#[cfg(test)]
mod tests {
    use super::{endpoint_of, work_in_git_message};
    use std::path::Path;

    #[test]
    fn the_work_folder_fix_is_not_the_default_folder_when_the_default_is_in_a_repository() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join(".git")).unwrap();
        let default = home.path().join("goalcup-work");
        let said = work_in_git_message(
            &home.path().join("repo/work"),
            home.path(),
            Some(default.as_path()),
        );
        assert!(
            said.contains("goalcup prep --work-root <") && !said.contains("~/goalcup-work"),
            "{said}"
        );
        // And with a home that is not a repository, the default folder is the fix.
        let clean = tempfile::tempdir().unwrap();
        let said = work_in_git_message(
            Path::new("/r/work"),
            Path::new("/r"),
            Some(clean.path().join("goalcup-work").as_path()),
        );
        assert!(
            said.contains("goalcup prep --work-root ~/goalcup-work"),
            "{said}"
        );
    }

    #[test]
    fn a_server_address_is_probed_on_its_own_port_and_on_the_schemes_when_it_names_none() {
        for (url, want) in [
            ("https://ingest.example.org", ("ingest.example.org", 443)),
            (
                "https://ingest.example.org/goal/cup",
                ("ingest.example.org", 443),
            ),
            (
                "https://ingest.example.org:8443",
                ("ingest.example.org", 8443),
            ),
            ("http://127.0.0.1:8787", ("127.0.0.1", 8787)),
            ("http://localhost", ("localhost", 80)),
            ("http://[::1]:9000/x", ("::1", 9000)),
        ] {
            assert_eq!(endpoint_of(url), (want.0.to_string(), want.1), "{url}");
        }
    }
}
