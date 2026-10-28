//! `goalcup setup` (once `goalcup prep`, still its hidden alias): from install to ready, in
//! four steps, and safe to run again. No GitHub here: a real run asks for its repository at
//! `goalcup start` (`commands::run_repo`), and a practice run needs none (D69).
//!
//! Each step is `[ok]`, `[warn] <what it means and what to do>`, `[todo] <what to do>` or
//! `[fail] <why>`, and the command exits 0 when no step is todo or failed (a warning does not
//! hold a step back):
//!
//! 1. **login**: `server.json` names an account. In a terminal, a computer that is not connected
//!    is logged in here, with `goalcup login`'s own device flow.
//! 2. **harness**: one of the connectors that run in chat mode (`interactive = true`), picked
//!    with `--harness`, before, or because it is the only one; in a terminal, the installed ones
//!    and the other supported ones are listed and the participant picks. Found on `PATH`, its
//!    version shown, and a pointer to its section of the setup guide, `docs/HARNESS-SETUP.md`
//!    (the connector's `participant.setup_section`), on every setup. When the connector names
//!    where its MCP servers are listed (`participant.time_left_tool`, fix plan v9), the step also
//!    prints the lines that add `goalcup mcp` there, with this binary's absolute path; it never
//!    writes them.
//! 3. **harness home**: the harness is opened once in the home its runs will use, so the
//!    participant signs in (or pastes an API key into the harness's own sign-in, which Goal Cup
//!    never reads or stores) and adds settings, MCP servers and skills there
//!    ([`crate::launch::prep_launch`]: no proxy, no capture, no daemon entry). The tracker
//!    cannot see a vendor sign-in, so this step is the person's word: it is ok once the harness
//!    they were given has exited 0, and that is recorded in `prep.json` (`home_ready`). When the
//!    harness's config does not set a connector's `participant.no_prompts` setting, the step's
//!    line also recommends it, with the setup guide's link (D67); that never changes the setting
//!    or the step's state.
//! 4. **machine**: mains power, free space for a run's reservation, the server's and the
//!    provider's hosts reachable, `git` on `PATH`, under WSL the WSL2 version and an artifact
//!    root and configuration folder that are not on a Windows-host filesystem, the same two
//!    folders on APFS, ext4 or XFS (not btrfs, ZFS, NFS, SMB, tmpfs, overlayfs or FUSE), and a
//!    sleep lock the machine can take (one it cannot take is said, and is never a `todo`).
//!
//! Setup makes no work folder and no keys file: a run works in the project folder `goalcup
//! start` is run from, where the participant keeps their own `.gitignore` and `.env.local`.
//!
//! # When setup acts, and when it only reports
//!
//! Only in a terminal, and never with `--json`, setup asks (the harness), logs in and launches
//! (the harness). `--json` prints `[{"step","state","human","say"}]`
//! and nothing else; `human` marks the steps that need the person (login, the harness home), for
//! the agent that is walking someone through the setup.
//!
//! # Re-running
//!
//! `<cfg>/prep.json` keeps what is settled: the harness, and the one fact that cannot be
//! re-observed (`home_ready`). A second setup skips the steps it has settled and keeps
//! every value a flag does not change; changing the harness starts its home again.

use crate::CliError;
use crate::connector::{HarnessConnector, harness_dirs};
use crate::folder_gate::Host;
use crate::prep_file::{PrepFile, read_prep_lenient, write_prep};
use crate::run::reserve::{LOW_DISK_BYTES, RUN_RESERVE_BYTES, START_MARGIN, artifact_root, human};
use goalcup_protocol::harness::Origin;
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::Path;

/// What the flags asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrepOptions {
    /// `--harness <id>`.
    pub harness: Option<String>,
    /// `--json`: print the steps as one JSON array, ask nothing, launch nothing.
    pub json: bool,
    /// `--dev`: accept a connector directory this build does not pin.
    pub dev: bool,
}

/// How a step stands. A warning says what happened and what to do, and the run can go on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Ok,
    Warn,
    Todo,
    Fail,
}

impl State {
    fn word(self) -> &'static str {
        match self {
            State::Ok => "ok",
            State::Warn => "warn",
            State::Todo => "todo",
            State::Fail => "fail",
        }
    }

    /// Whether the step needs nothing more from the person: ok, or a warning.
    fn settled(self) -> bool {
        matches!(self, State::Ok | State::Warn)
    }
}

/// One step of the report, and one element of `--json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Step {
    /// `login`, `harness`, `harness_home` or `machine`.
    pub step: &'static str,
    pub state: State,
    /// The step needs the person (a login, a sign-in), not only a command.
    pub human: bool,
    /// What was found, or what to do, or why it failed: one or two sentences.
    pub say: String,
}

/// The step ids, in order.
pub const STEP_LOGIN: &str = "login";
pub const STEP_HARNESS: &str = "harness";
pub const STEP_HARNESS_HOME: &str = "harness_home";
pub const STEP_MACHINE: &str = "machine";

/// What setup opens for the participant: the login's approval address. It does nothing when
/// `GOALCUP_NO_BROWSER` is set.
pub struct Openers {
    pub url: fn(&str) -> bool,
}

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
    /// The harness's own default hosts, when its settings name a model service the run records
    /// instead: the first that does not answer, in a phrase. A failure here is a warning. The
    /// default is [`Doors::hosts_reachable`].
    fn default_hosts_reachable(&self, endpoints: &[(String, u16)]) -> Result<(), String> {
        self.hosts_reachable(endpoints)
    }
    /// Whether the machine can be kept awake (the lock is taken and let go at once).
    fn sleep_lock(&self) -> Result<(), String>;
    /// One line typed by the participant, or `None` when the input ended.
    fn read_line(&self) -> Option<String>;
    /// Wait for the participant to press Enter before the harness takes the terminal; `false`
    /// when the input ended without one (the participant chose not to go on).
    fn wait_for_enter(&self) -> bool;
    /// Open the harness in its home for the participant. The home is held first, so a home in
    /// use is said before the participant is asked anything; then `go` is asked, and `false`
    /// opens nothing. `None` when nothing was opened, else whether the harness exited 0.
    fn open_harness(
        &self,
        connector: &HarnessConnector,
        cfg: &Path,
        env: &BTreeMap<String, String>,
        go: &mut dyn FnMut() -> Result<bool, CliError>,
    ) -> Result<Option<bool>, CliError>;
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
        goalcup_platform::awake::hold("goalcup setup is checking that the machine can stay awake")
            .map(drop)
    }

    fn read_line(&self) -> Option<String> {
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(n) if n > 0 => Some(line),
            _ => None,
        }
    }

    fn wait_for_enter(&self) -> bool {
        self.read_line().is_some()
    }

    fn open_harness(
        &self,
        connector: &HarnessConnector,
        cfg: &Path,
        env: &BTreeMap<String, String>,
        go: &mut dyn FnMut() -> Result<bool, CliError>,
    ) -> Result<Option<bool>, CliError> {
        use crate::home::{WorkspaceAt, claim_home_for_prep};
        let profile = &connector.loaded.manifest.capture;
        // The home every run of this connector keeps (its config home, D39, or its run
        // `HOME`, D59), held so no run is recording in it while the participant is signed in
        // there. It is held before the pause: a run recording there is said at once, not after
        // the participant pressed Enter.
        let Some(home) = claim_home_for_prep(cfg, connector)? else {
            return Ok(Some(true));
        };
        let executable = crate::launch::capture_executable(connector, profile, env)?;
        if !go()? {
            return Ok(None);
        }
        // A `HOME` that is made fresh for each run is made fresh for this, and removed after:
        // nothing the participant sets up lives there, it lives in the harness home. A kept run
        // `HOME` is the home itself, used as it is and never removed.
        let beside = cfg.join("prep-launch");
        crate::config::create_private_dir(&beside)?;
        let run = crate::home::RunDir::create(
            &beside,
            profile.home.run_home,
            home.is_run_home.then_some(home.dir.as_path()),
            env,
            WorkspaceAt::InRunDir,
        )?;
        let launch = crate::launch::PrepLaunch {
            executable: &executable,
            run_home: &run.home,
            // With a kept run `HOME` the harness has no home variable: its state is under
            // `HOME`, and the config home is the same folder.
            config_home: if home.is_run_home {
                &run.home
            } else {
                &home.dir
            },
            parent_env: env,
        };
        // The CA file a past run's launch prep left (D72) names that run's closed recording
        // proxy, and the harness may refuse every connection while its config points at it.
        forget_stale_ca_file(connector, launch.config_home);
        let opened = crate::config::create_private_dir(&run.workspace)
            .and_then(|()| crate::launch::prep_launch(connector, &launch, &run.workspace));
        // After the participant's own setup, the settings the capture needs (D61). A home they
        // did not reach is not ready, so it is not configured either.
        let configured = match &opened {
            Ok(status) if status.success() => Some(configure_after_setup(
                connector,
                cfg,
                &launch,
                &run.workspace,
            )),
            _ => None,
        };
        let _ = run.remove();
        let status = opened?;
        if let Some(configured) = configured {
            configured?;
        }
        Ok(Some(status.success()))
    }
}

/// Remove the connector's `[launch_prep] ca_file` (D72) from `config_home` before setup opens
/// the harness there. It only ever held a past run's recording-proxy CA, and goalcup writes it
/// again at every start and continue. With it gone, a provider entry whose `ssl_ca_cert` still
/// names it falls back to the system's trust (Hermes 0.22, `agent/ssl_verify.py`), so the
/// harness reaches its model outside a run. A link is removed, never followed; a folder of that
/// name is left as it is. Nothing is said: it is goalcup's own file.
pub(crate) fn forget_stale_ca_file(connector: &HarnessConnector, config_home: &Path) {
    let Some(name) = connector
        .loaded
        .manifest
        .launch_prep
        .as_ref()
        .and_then(|prep| prep.ca_file.as_ref())
    else {
        return;
    };
    let path = config_home.join(name);
    if std::fs::symlink_metadata(&path).is_ok_and(|meta| !meta.is_dir()) {
        let _ = std::fs::remove_file(&path);
    }
}

/// Where `goalcup setup` appends the output of the connector's `capture.configure` commands, in
/// the configuration folder: nothing they print reaches the terminal.
pub const PREP_LOG_NAME: &str = "prep.log";

/// The connector's `capture.configure` commands (D61), run in the home the participant has just
/// set up, with their output appended to `<cfg>/prep.log`.
fn configure_after_setup(
    connector: &HarnessConnector,
    cfg: &Path,
    launch: &crate::launch::PrepLaunch<'_>,
    workspace: &Path,
) -> Result<(), CliError> {
    if connector.loaded.manifest.capture.configure.is_empty() {
        return Ok(());
    }
    let log_path = cfg.join(PREP_LOG_NAME);
    let log = crate::host_log::open_append(&log_path).map_err(|source| CliError::Io {
        path: log_path.display().to_string(),
        source,
    })?;
    crate::launch::configure_harness(
        connector,
        launch,
        workspace,
        &log,
        &format!(
            "What it printed is in {}. Run `goalcup setup` again.",
            log_path.display()
        ),
    )
}

/// `goalcup setup`, with everything it reads handed in. Returns the exit code: 0 when all four
/// steps are ok, 1 otherwise. A mistyped flag is a refusal before anything is written.
pub fn prep_with(
    doors: &dyn Doors,
    cfg: &Path,
    connectors: &Path,
    env: &BTreeMap<String, String>,
    opts: &PrepOptions,
    open: &Openers,
    out: &mut dyn Write,
) -> Result<u8, CliError> {
    crate::config::create_private_dir(cfg)?;
    let acting = !opts.json && doors.has_terminal();
    let mut file = read_prep_lenient(cfg);
    let before = file.clone();
    let mut report = Report {
        json: opts.json,
        steps: Vec::new(),
        out,
    };
    report.tell("Goal Cup setup. 4 steps. Ctrl-C stops at any time; run it again to continue.")?;

    // 1. login, here and now when this computer is not connected and there is a terminal.
    let mut login = login_step(cfg);
    if login.state == State::Todo && acting {
        report.tell("This computer is not connected to Goal Cup yet. Connecting it:")?;
        let args = crate::commands::login::LoginArgs::default();
        login = match crate::commands::login::run(&args, cfg, &open.url, report.out) {
            Ok(()) => login_step(cfg),
            Err(error) => step(STEP_LOGIN, State::Fail, true, error.to_string()),
        };
    }
    report.push(login)?;

    // 2. harness
    let mut wanted = opts
        .harness
        .clone()
        .or_else(|| (!file.harness.is_empty()).then(|| file.harness.clone()));
    if wanted.is_none() && acting {
        wanted = ask_harness(doors, connectors, env, opts.dev, &mut report)?;
    }
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
    let mut home = match &connector {
        Some(c) => home_step(doors, cfg, env, c, &mut file, acting, &mut report)?,
        None => step(
            STEP_HARNESS_HOME,
            State::Todo,
            true,
            "choose and install your agent first (the step above). Then `goalcup setup` opens it \
             so you can sign in and add your settings, MCP servers and skills",
        ),
    };
    // D67: a recommendation only. It never changes the step's state or the harness's settings.
    if let Some(advice) = connector.as_ref().and_then(|c| no_prompts_advice(cfg, c))
        && home.state != State::Fail
    {
        if !home.say.ends_with('.') {
            home.say.push('.');
        }
        home.say.push(' ');
        home.say.push_str(&advice);
    }
    report.push(home)?;
    save(cfg, &file, &before)?;

    // 4. machine
    report.push(machine_step(doors, cfg, env, connector.as_ref()))?;
    // A warning beside the step, never a change of its state (D73): what the agent could push
    // with the participant's own SSH keys.
    if !report.json
        && let Some(found) = crate::ssh_keys::look(env)
    {
        if let Some(warning) = found.key_warning() {
            report.tell(&format!("[warn] {warning}"))?;
        }
        if let Some(note) = found.agent_note() {
            report.tell(&format!("[note] {note}"))?;
        }
    }

    let all_ok = report.steps.iter().all(|s| s.state.settled());
    if report.json {
        let text = serde_json::to_string_pretty(&report.steps)?;
        put(report.out, &format!("{text}\n"))?;
        return Ok(u8::from(!all_ok));
    }
    // D69: nothing reads or writes `github.json` now but the old clones' helper. Its token stays
    // valid on GitHub until the participant revokes it, so say where it is, never what it is.
    let old_token = crate::account::github::github_path(cfg);
    if old_token.exists() {
        report.tell(&format!(
            "An old GitHub token from an earlier goalcup is stored at {}. Goal Cup no longer uses \
             it: delete the file and revoke the token on GitHub once your older dry run is over.",
            old_token.display()
        ))?;
    }
    if all_ok {
        let id = connector
            .as_ref()
            .map_or("<id>", |c| c.loaded.manifest.id.as_str());
        put(
            report.out,
            &format!(
                "Ready.\n  Try it (10 min, no GitHub, never counts; a new \
                 folder each time): d=$(mktemp -d ~/goalcup-practice-XXXX) && cd \"$d\" && \
                 goalcup start --harness {id} --practice\n  The real run, inside the start window: make your \
                 repo, token and folder first (the guide's folder step), then in that folder: \
                 goalcup start --harness {id} --repo <you>/<name>\n"
            ),
        )?;
    } else {
        let waiting = report.steps.iter().filter(|s| !s.state.settled()).count();
        // The last line on the screen points at the next action: the first step that needs the
        // person. That step's own line says what to do, so it is said once, not again here.
        let next = report
            .steps
            .iter()
            .find(|s| !s.state.settled())
            .map_or("", |s| s.step);
        put(
            report.out,
            &format!(
                "goalcup setup: {waiting} of {} steps need you. Next: the {next} step above.\n",
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

/// Every connector in `connectors` that loads (each runs in chat mode, D57), in id order.
fn loadable_connectors(connectors: &Path, dev: bool) -> Result<Vec<HarnessConnector>, CliError> {
    let origin = if dev { Origin::Dev } else { Origin::Pinned };
    let mut found = Vec::new();
    for dir in harness_dirs(connectors)? {
        // One that does not load is `goalcup doctor`'s to report; it cannot be picked here.
        if let Ok(loaded) = goalcup_protocol::harness::load_discovered_dir(&dir, origin) {
            found.push(HarnessConnector { dir, loaded });
        }
    }
    found.sort_by(|a, b| a.loaded.manifest.id.cmp(&b.loaded.manifest.id));
    Ok(found)
}

/// The bring-your-own-harness guide: how a harness Goal Cup does not ship becomes a connector.
pub const OWN_HARNESS_GUIDE: &str =
    "https://github.com/agentcommunity/goal-cup-proxy/blob/main/docs/HARNESS-ONBOARDING.md";

/// The chat-mode harnesses, as setup lists them: the installed ones (found on `PATH`), the other
/// supported ones with where to read how to install them, and the guide for one of the
/// participant's own.
fn harness_menu(chat: &[HarnessConnector], env: &BTreeMap<String, String>) -> String {
    let (installed, other): (Vec<&HarnessConnector>, Vec<&HarnessConnector>) =
        chat.iter().partition(|c| {
            crate::commands::doctor::executable_check(c, env).level != crate::preflight::Level::Fail
        });
    let ids = |list: &[&HarnessConnector]| {
        list.iter()
            .map(|c| c.loaded.manifest.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut menu = format!(
        "Installed: {}",
        if installed.is_empty() {
            "none yet".to_string()
        } else {
            ids(&installed)
        }
    );
    if !other.is_empty() {
        menu.push_str(&format!(
            "\n  Also supported: {} (how to install: {SETUP_GUIDE})",
            ids(&other)
        ));
    }
    menu.push_str(&format!("\n  Your own harness? {OWN_HARNESS_GUIDE}"));
    menu
}

/// In a terminal, with more than one chat-mode harness and none chosen yet: list them
/// ([`harness_menu`]) and ask which one, the first installed one by default. `None` leaves the
/// harness step to say what to do.
fn ask_harness(
    doors: &dyn Doors,
    connectors: &Path,
    env: &BTreeMap<String, String>,
    dev: bool,
    report: &mut Report<'_>,
) -> Result<Option<String>, CliError> {
    let Ok(chat) = loadable_connectors(connectors, dev) else {
        return Ok(None);
    };
    if chat.len() < 2 {
        return Ok(None);
    }
    let default = chat
        .iter()
        .find(|c| {
            crate::commands::doctor::executable_check(c, env).level != crate::preflight::Level::Fail
        })
        .map(|c| c.loaded.manifest.id.clone());
    report.tell(&harness_menu(&chat, env))?;
    put(
        report.out,
        &format!(
            "Which one?{} ",
            default
                .as_deref()
                .map(|d| format!(" [{d}]"))
                .unwrap_or_default()
        ),
    )?;
    report.out.flush().map_err(|source| CliError::Io {
        path: "standard output".to_string(),
        source,
    })?;
    Ok(match doors.read_line().map(|l| l.trim().to_string()) {
        Some(typed) if !typed.is_empty() => Some(typed),
        Some(_) => default,
        None => None,
    })
}

fn harness_step(
    connectors: &Path,
    env: &BTreeMap<String, String>,
    cfg: &Path,
    wanted: Option<&str>,
    dev: bool,
) -> (Option<HarnessConnector>, Step) {
    let fail = |say: String| (None, step(STEP_HARNESS, State::Fail, false, say));
    let chat = match loadable_connectors(connectors, dev) {
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
                let menu = harness_menu(&chat, env);
                return (
                    None,
                    step(
                        STEP_HARNESS,
                        State::Todo,
                        false,
                        format!("choose your agent: goalcup setup --harness <id>. {menu}",),
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
    // Fix plan v8: the guide to the harness's settings, on every prep, missing settings or not.
    // Only a pointer: it never changes the step's state or a setting.
    let mut say = format!(
        "{id} {version} ({path}). Before a long run, read {}",
        setup_guide_link(&connector)
    );
    // Fix plan v9: the lines that add the time-left tool, on every prep. Only printed.
    if let Some(lines) = time_left_tool_advice(cfg, &connector) {
        say.push('\n');
        say.push_str(&lines);
    }
    (Some(connector), step(STEP_HARNESS, State::Ok, false, say))
}

/// The marker lines around the time-left tool's lines in prep's report. Copy what is between.
const COPY_START: &str = "  ----- start of the lines to copy -----";
const COPY_END: &str = "  ----- end of the lines to copy -----";

/// Fix plan v9: when the connector names where its MCP servers are listed
/// (`participant.time_left_tool`), the lines that add `goalcup mcp` there, with this binary's
/// absolute path, so the agent can ask the time left whatever its `PATH`. The lines are printed
/// as they go in the file, with no indent, between [`COPY_START`] and [`COPY_END`] (review P4
/// L3: YAML lines indented for display nest wrongly when pasted). Never written: Goal Cup does
/// not change the harness's settings (D67).
fn time_left_tool_advice(cfg: &Path, connector: &HarnessConnector) -> Option<String> {
    let tool = connector
        .loaded
        .manifest
        .participant
        .as_ref()?
        .time_left_tool
        .as_ref()?;
    let file = crate::home::prep_home_leaf(cfg, connector)?.join(&tool.file);
    let goalcup = std::env::current_exe().ok()?;
    let lines = crate::commands::mcp::setup_lines(tool, goalcup.to_str()?);
    Some(format!(
        "  So your agent can ask how much time is left whatever its PATH, add Goal Cup's \
         time-left tool to `{}` in {} (Goal Cup does not change that file). Copy the lines \
         between the two marker lines exactly as shown, with no spaces added in front, {}.\n\
         {COPY_START}\n{lines}\n{COPY_END}",
        tool.key,
        file.display(),
        crate::commands::mcp::setup_place(tool),
    ))
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
    let Some(home) = crate::home::prep_home_leaf(cfg, connector) else {
        return Ok(step(
            STEP_HARNESS_HOME,
            State::Ok,
            false,
            format!("{id} needs no set-up here: each run gets a fresh home"),
        ));
    };
    // D62: a home whose harness names no model service Goal Cup can record is not ready, so the
    // harness opens again for the participant to choose one.
    let unrecordable = model_host_problem(cfg, connector);
    let stale = unrecordable.as_ref().filter(|_| home.is_dir());
    if stale.is_some() {
        file.home_ready = false;
    }
    // The person's word counts only while the home they set up is still there.
    if file.home_ready && home.is_dir() {
        return Ok(step(
            STEP_HARNESS_HOME,
            State::Ok,
            false,
            format!("{id} is set up in its Goal Cup home. Your runs use it."),
        ));
    }
    if let (Some(why), false) = (stale, acting) {
        return Ok(human(
            State::Todo,
            format!("{why} Run `goalcup setup` in a terminal"),
        ));
    }
    if let Some(why) = stale {
        report.tell(why)?;
    }
    if !acting {
        return Ok(human(
            State::Todo,
            format!(
                "run `goalcup setup` in a terminal. It opens {id} in its Goal Cup home ({}). \
                 Sign in there (or paste your API key; Goal Cup never reads or stores it), add \
                 your settings, MCP servers and skills, then quit {id} (most agents: /quit, /exit or Ctrl-C twice)",
                home.display()
            ),
        ));
    }
    // Asked once the home is held, so a home in use is said before the pause.
    let mut go = || -> Result<bool, CliError> {
        report.tell(&format!(
            "Next, {id} opens here in its own Goal Cup home (not your usual setup). Nothing is \
             recorded.\n  1. Sign in, or paste your API key when it asks. Goal Cup never \
             reads or stores it.\n  2. Add the settings, MCP servers and skills you want your \
             agent to have, and set it not to ask for permission (see the guide above).\n  3. Quit {id} \
             when you are done (for most agents: type /quit or /exit, or press Ctrl-C twice). \
             Setup goes on once it has quit.\nPress Enter to open {id}, or Ctrl-C to stop."
        ))?;
        let entered = doors.wait_for_enter();
        if !entered {
            // The input ended (Ctrl-D) with no Enter, so the terminal's cursor is still on the
            // question's line: the next report line starts on a line of its own.
            report.tell("")?;
        }
        Ok(entered)
    };
    match doors.open_harness(connector, cfg, env, &mut go) {
        Ok(None) => Ok(human(
            State::Todo,
            format!("{id} was not opened. Run `goalcup setup` again, then press Enter"),
        )),
        Ok(Some(true)) => match model_host_problem(cfg, connector) {
            Some(why) => Ok(human(
                State::Todo,
                format!("{why} Run `goalcup setup` again"),
            )),
            None => {
                file.home_ready = true;
                Ok(step(
                    STEP_HARNESS_HOME,
                    State::Ok,
                    false,
                    format!("{id} is set up in its Goal Cup home. Your runs use it."),
                ))
            }
        },
        Ok(Some(false)) => Ok(human(
            State::Todo,
            format!(
                "{id} did not exit normally. Run `goalcup setup` again, then exit {id} the \
                 normal way"
            ),
        )),
        Err(error) => Ok(human(State::Fail, error.to_string())),
    }
}

/// Why the model service the harness's config names cannot be recorded (D62), with what to do in
/// the harness, or `None` when it can be or the connector reads none. The name is also looked up:
/// one that resolves only to this computer is refused as `goalcup start` refuses it.
fn model_host_problem(cfg: &Path, connector: &HarnessConnector) -> Option<String> {
    let why = match crate::model_host::check(cfg, connector) {
        Ok(Some(model)) => crate::model_host::refuse_if_on_this_computer(&model).err()?,
        Ok(None) => return None,
        Err(why) => why,
    };
    let name = connector.display_name();
    // A file Goal Cup cannot read is fixed in the file, not by choosing another service.
    if why.file.is_some() {
        return Some(why.sentence(name));
    }
    Some(format!(
        "{} In {name}, choose an https model service that is not on this computer, then exit \
         {name}.",
        why.sentence(name)
    ))
}

/// The participant's harness setup guide (fix plan v8): every setting a 168-hour run needs.
pub const SETUP_GUIDE: &str =
    "https://github.com/agentcommunity/goal-cup-proxy/blob/main/docs/HARNESS-SETUP.md";

/// The setup guide's link for `connector`: its `participant.setup_section`, else the guide.
/// The section comes from the connector's data, so this names no harness.
pub(crate) fn setup_guide_link(connector: &HarnessConnector) -> String {
    match connector.loaded.manifest.setup_section() {
        Some(section) => format!("{SETUP_GUIDE}#{section}"),
        None => SETUP_GUIDE.to_string(),
    }
}

/// D67: when the harness's own config (in its Goal Cup home) does not set each of the
/// connector's `participant.no_prompts` settings to an accepted value, one sentence recommending
/// them, with the setup guide's link. `None` when all are set, or the connector lists none. Only
/// reads; never writes the harness's settings and never fails prep.
fn no_prompts_advice(cfg: &Path, connector: &HarnessConnector) -> Option<String> {
    let manifest = &connector.loaded.manifest;
    let settings = &manifest.participant.as_ref()?.no_prompts;
    let home = cfg
        .join(goalcup_protocol::harness::CAPTURE_HOMES_DIR)
        .join(&manifest.id);
    let unset: Vec<String> = settings
        .iter()
        .filter(|s| {
            let value = crate::model_host::read_setting(&home.join(&s.file), &s.file, &s.key);
            !value.is_some_and(|v| s.values.contains(&v))
        })
        .map(|s| {
            format!(
                "`{}` to `{}` in {}",
                s.key,
                s.values[0],
                home.join(&s.file).display()
            )
        })
        .collect();
    if unset.is_empty() {
        return None;
    }
    let name = connector.display_name();
    Some(format!(
        "{name} may stop to ask your permission, and during a run you may type only \"keep \
         going\", so it would wait. Goal Cup recommends setting {}. Goal Cup does not change \
         {name}'s settings; see {}",
        unset.join(", "),
        setup_guide_link(connector)
    ))
}

// ------------------------------------------------------------------ 4. the machine

/// Free bytes a run needs to start at all, and to start without a warning.
const FAIL_BELOW: u64 = RUN_RESERVE_BYTES + START_MARGIN;
const OK_FROM: u64 = RUN_RESERVE_BYTES + LOW_DISK_BYTES;

fn machine_step(
    doors: &dyn Doors,
    cfg: &Path,
    env: &BTreeMap<String, String>,
    connector: Option<&HarnessConnector>,
) -> Step {
    let mut worst = State::Ok;
    let mut problems: Vec<String> = Vec::new();
    let mut fine: Vec<String> = Vec::new();
    let mut note = |state: State, sentence: String| {
        if state == State::Ok {
            fine.push(sentence);
        } else {
            worst = match (worst, state) {
                (State::Fail, _) | (_, State::Fail) => State::Fail,
                (State::Todo, _) | (_, State::Todo) => State::Todo,
                _ => State::Warn,
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

    // The hosts that must answer: the server, and the model service the settings name. Without a
    // model service, the harness's own hosts (its profile's and sign-in hosts) must answer too.
    let mut hosts: Vec<(String, u16)> = Vec::new();
    if let Ok(Some(server)) = crate::server::read_server_config(cfg) {
        hosts.push(endpoint_of(&server.url));
    }
    // The harness's own hosts, when a model service is named: a run records that service instead,
    // so their failure is a warning.
    let mut defaults: Vec<(String, u16)> = Vec::new();
    let mut service = None;
    // Whether a model host is probed too, so the line says only what was probed.
    let mut provider = false;
    if let Some(c) = connector {
        let profile = &c.loaded.manifest.capture;
        let own: Vec<(String, u16)> = profile
            .hosts
            .iter()
            .map(|h| (h.host.clone(), 443))
            .chain(profile.auth_hosts.iter().map(|h| (h.clone(), 443)))
            .collect();
        provider = !profile.hosts.is_empty();
        // The model service the harness's config names (D62), once it names one.
        match crate::model_host::check(cfg, c) {
            Ok(Some(m)) => {
                hosts.push((m.host.clone(), m.port));
                defaults = own;
                service = Some(m);
                provider = true;
            }
            _ => hosts.extend(own),
        }
    }
    for list in [&mut hosts, &mut defaults] {
        list.retain(|(host, _)| !host.is_empty());
        list.sort();
        list.dedup();
    }
    match doors.hosts_reachable(&hosts) {
        Ok(()) => note(
            State::Ok,
            if provider {
                "the server and the model service are reachable"
            } else {
                "the server is reachable"
            }
            .to_string(),
        ),
        Err(why) => note(
            State::Fail,
            format!(
                "{why}. Check the network; a firewall such as Little Snitch may be holding \
                 goalcup's connections"
            ),
        ),
    }
    if let Some(m) = &service
        && !defaults.is_empty()
        && let Err(why) = doors.default_hosts_reachable(&defaults)
    {
        note(
            State::Warn,
            format!(
                "{why}; your settings use {}, so the run goes on. The run does not \
                 need that host. To use it, check the network or a firewall such as Little \
                 Snitch, then run setup again",
                m.authority()
            ),
        );
    }

    if crate::preflight::resolve_on_path("git", env).is_some() {
        note(State::Ok, "git is installed".to_string());
    } else {
        note(
            State::Fail,
            "git is not on PATH: install it (https://git-scm.com), your agent pushes your run \
             with it"
                .to_string(),
        );
    }

    // Under WSL: WSL1 cannot run this, and a Windows-host filesystem is refused for the artifact
    // root and the configuration folder.
    let host = doors.wsl();
    let wsl = host.is_wsl();
    let artifacts = artifact_root(cfg);
    if let Err(why) = host.check(&[
        ("artifact root", artifacts.as_path()),
        ("config folder", cfg),
    ]) {
        note(State::Fail, why);
    }
    // On any machine: a run keeps its records on APFS, ext4 or XFS only. Found here, the folder
    // is moved before any approval.
    if let Err(why) = host.check_filesystems(&[
        ("artifact root", artifacts.as_path()),
        ("config folder", cfg),
    ]) {
        note(State::Fail, why);
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
    step(STEP_MACHINE, worst, false, say)
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
/// (repeatable), `hosts=ok|<phrase>`, `defaults=ok|<phrase>` (the harness's own hosts, when its
/// settings name a service) and `awake=ok|<phrase>`; everything it does not name is this
/// machine's. A shipped build has no such door.
#[cfg(feature = "test-support")]
pub struct Injected {
    ac: Option<Option<bool>>,
    free: Option<u64>,
    host: Host,
    hosts: Option<Result<(), String>>,
    defaults: Option<Result<(), String>>,
    awake: Option<Result<(), String>>,
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
            defaults: None,
            awake: None,
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
                "defaults" => door.defaults = Some(answer(value)),
                "awake" => door.awake = Some(answer(value)),
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

    fn default_hosts_reachable(&self, endpoints: &[(String, u16)]) -> Result<(), String> {
        match &self.defaults {
            Some(answer) => answer.clone(),
            None => self.hosts_reachable(endpoints),
        }
    }

    fn sleep_lock(&self) -> Result<(), String> {
        match &self.awake {
            Some(answer) => answer.clone(),
            None => Here.sleep_lock(),
        }
    }

    fn read_line(&self) -> Option<String> {
        Here.read_line()
    }

    fn wait_for_enter(&self) -> bool {
        Here.wait_for_enter()
    }

    fn open_harness(
        &self,
        connector: &HarnessConnector,
        cfg: &Path,
        env: &BTreeMap<String, String>,
        go: &mut dyn FnMut() -> Result<bool, CliError>,
    ) -> Result<Option<bool>, CliError> {
        Here.open_harness(connector, cfg, env, go)
    }
}

#[cfg(test)]
mod tests {
    use super::{State, endpoint_of, login_step};

    /// Setup removes the CA file a past run left in the Hermes home before it opens Hermes
    /// there, and touches nothing for a connector with no launch prep (D72, D73).
    #[test]
    fn setup_removes_a_past_runs_ca_file_before_it_opens_the_harness() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors");
        let hermes = crate::connector::load_harness(&dir, "hermes-cli", false).unwrap();
        let codex = crate::connector::load_harness(&dir, "codex-cli", false).unwrap();
        let home = tempfile::tempdir().unwrap();
        let ca = home.path().join("recorder-ca.pem");
        std::fs::write(&ca, "-----BEGIN CERTIFICATE-----\n").unwrap();
        super::forget_stale_ca_file(&codex, home.path());
        assert!(ca.exists());
        super::forget_stale_ca_file(&hermes, home.path());
        assert!(!ca.exists());
        // Gone already: nothing to do, and nothing fails.
        super::forget_stale_ca_file(&hermes, home.path());
    }
    use crate::server::{DEFAULT_SERVER_URL, ServerConfig, write_server_config};

    #[test]
    fn the_login_step_names_the_server_only_when_it_is_not_the_default() {
        for (url, named) in [
            (DEFAULT_SERVER_URL.to_string(), false),
            (format!("{DEFAULT_SERVER_URL}/"), false),
            ("https://ingest.example.org".to_string(), true),
        ] {
            let cfg = tempfile::tempdir().unwrap();
            let config = ServerConfig {
                url: url.clone(),
                account: Some("member-1".to_string()),
            };
            write_server_config(cfg.path(), &config).unwrap();
            let step = login_step(cfg.path());
            assert_eq!(step.state, State::Ok, "{url}");
            assert!(step.say.contains("member-1"), "{url}: {}", step.say);
            assert_eq!(step.say.contains(&url), named, "{url}: {}", step.say);
        }
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
