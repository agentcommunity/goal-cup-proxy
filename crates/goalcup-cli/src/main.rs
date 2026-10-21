//! `goalcup` — the participant's command line.
//!
//! This file does two things and no others: it parses the command tree, and it is the
//! **only place an `ExitCode` is built**. Every failure's number comes from
//! [`goalcup_cli::CliError::exit_code`], so no command can invent a code without editing
//! one of these two files.
//!
//! `run` answers a `u8` rather than `()` for one command: `goalcup doctor` reports its own
//! number on success (0 or 1). Every other arm answers 0.
//!
//! # Why the parse is `try_parse`
//!
//! `Parser::parse()` exits `2` on a usage error, and `2` is a `Fail` verdict in this tree's
//! ladder. A mistyped sub-command would have been indistinguishable, to a script reading the
//! number, from an entry that did not verify. `goalcup-verify`'s `main` makes the same move
//! for the same reason and lands on `3`; this binary lands on `1`, because a mistyped
//! command is a refusal the participant can fix and rung 1 is what that means here.
//!
//! `--help` and `--version` arrive as clap errors too. They are not failures, so they print
//! on stdout and exit 0 — which is what `Error::use_stderr()` distinguishes.

use clap::{Parser, Subcommand};
use goalcup_cli::{CliError, commands};
use std::process::ExitCode;

/// Record one coding agent's run for the /goal Cup: launch the harness, record what it does, and
/// seal the record when it finishes.
#[derive(Parser)]
#[command(
    name = "goalcup",
    // The version this build carries, which is `goalcup_cli::CLI_VERSION`.
    version = goalcup_cli::CLI_VERSION,
    // Where verification lives and what number this binary leaves under, under both helps.
    // The constant carries the reasoning.
    after_help = AFTER_HELP,
    subcommand_required = true,
    arg_required_else_help = true,
    // So the list under `Commands:` is exactly the ten sub-commands below. clap's built-in
    // `help` sub-command would be an eleventh line in it, indistinguishable to a reader — and
    // to a test — from a sub-command this tree decided to ship. `goalcup <command> --help` is
    // the same help by the route the other nine already take.
    disable_help_subcommand = true
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

/// Every `goalcup --help` footer: the steps in the order a participant takes them, where the
/// guide is, and the exit codes.
const AFTER_HELP: &str = "\
Steps:
  1. goalcup login                               connect this computer (once)
  2. goalcup prep                                run it until every step is ok
  3. goalcup start --harness <id> --practice     a 10-minute practice run
  4. goalcup start --harness <id>                your run, inside the start window
While it runs: goalcup status (in another terminal). If the agent stopped early: goalcup continue.
A headless harness takes `--goal-file <file>` instead of a typed prompt.
To check a finished run: goalcup-verify --capture-integrity <trace-dir>

Guide: https://github.com/agentcommunity/goal-cup-tracer/blob/main/docs/PARTICIPANT-GUIDE.md
Help: goalcup@agentcommunity.org

Exit codes: 0 done; 1 refused, and you can fix it; 3 the command could not do its job. This \
command never exits with 2 or 4.";

/// What `goalcup doctor --help` says under its flags. There is no verify
/// mode here: a sealed capture is checked by `goalcup-verify --capture-integrity`.
const DOCTOR_AFTER_HELP: &str = "\
For each harness it checks, it verifies that the connector files are the pinned ones and \
finds the harness executable on PATH that `goalcup start` runs (hashed, not started). Any build is accepted: the \
run records which one ran.

Exit codes: 0 every check passed; 1 something failed, including a harness that is not \
installed. No daemon running is not a failure: `goalcup start` starts one.

Without `--harness` it checks the harness you chose in `goalcup prep`; pass `--harness <id>` to \
check another one, or `--harness all` to check every shipped harness (one you have not \
installed then makes this command exit 1).";

/// The sub-commands: ten, and the hidden `git-credential` that a run's clone calls. `verify`
/// is deliberately absent: see `AFTER_HELP`.
#[derive(Subcommand)]
enum Command {
    /// Step 1. Connect this computer to your Goal Cup account.
    ///
    /// Your browser opens on a page with a short code. Check that the code is the same as in
    /// your terminal, then approve. You do this once per computer. No password is typed or
    /// stored: only the server's address and your account's name are saved. Without a login,
    /// runs stay on this computer.
    Login {
        /// The server's address, for example `https://ingest.agentcommunity.org`. Without
        /// it, the server of your last login, else the Goal Cup server.
        #[arg(long, value_name = "URL")]
        server: Option<String>,
    },
    /// Step 2. Get this computer ready for your run.
    ///
    /// Six steps: you are logged in; you choose your agent (the harness) and it is installed;
    /// you open it once here, sign in, and add your settings, MCP servers and skills; your new,
    /// empty, public GitHub repository is checked, with a token for it; this computer has the
    /// power, the disk, the network and a work folder outside any Git repository that a
    /// week-long run needs; and a practice run is recorded and uploaded. Each step prints
    /// `[ok]`, `[todo]` or `[fail]`, and the last line says what to do next. Run it again as
    /// often as you like: what passed is not repeated. It never starts a run.
    Prep {
        /// Your agent, for example `codex-cli` or `claude-code-cli`. Without it, the one you
        /// chose before, or the only one this build offers.
        #[arg(long, value_name = "ID")]
        harness: Option<String>,
        /// The address of your new, empty, public GitHub repository,
        /// `https://github.com/<owner>/<repo>`. You are asked for a token for it, and what you
        /// type is not shown.
        #[arg(long, value_name = "URL")]
        repo: Option<String>,
        /// The folder each run's clone is made in, as a full path. It must be outside any Git
        /// repository. Default: `goalcup-work` in your home folder. Under WSL2 it must not be
        /// on a Windows drive.
        #[arg(long, value_name = "PATH")]
        work_root: Option<std::path::PathBuf>,
        /// Print the six steps as one JSON array (`step`, `state`, `human`, `say`), and nothing
        /// else. It asks nothing and opens nothing.
        #[arg(long)]
        json: bool,
        /// For connector developers: accept a connector directory this build does not pin.
        #[arg(long)]
        dev: bool,
    },
    /// Steps 3 and 4. Start a run: open your agent here and record what it does.
    ///
    /// Your agent opens in this terminal. Type your prompt there, and after it only
    /// "keep going". Until the agent exits, goalcup prints nothing here; open a second terminal
    /// and run `goalcup status` to see the run. A headless harness takes `--goal-file` instead.
    Start {
        /// Which harness to run, for example `claude-code-cli` or `codex-cli` (`goalcup doctor`
        /// lists every one this build ships).
        #[arg(long, value_name = "ID")]
        harness: String,
        /// For connector developers: accept a connector directory this build does not pin. The
        /// run says so, and it does not count as a competition entry.
        #[arg(long)]
        dev: bool,
        /// For a headless harness only: a text file holding your goal, the
        /// agent's first and only prompt. The run's folder is created next to this file, so put
        /// it in an empty folder of its own, outside any git repository. A chat-mode harness
        /// refuses it; its run's folder is created in the folder you run `goalcup start` from.
        #[arg(long, value_name = "PATH")]
        goal_file: Option<std::path::PathBuf>,
        /// A practice run: the server keeps it apart from the competition, and it never
        /// replaces your competition run. Needs `goalcup login` and `goalcup prep` first,
        /// like a competition run.
        #[arg(long, conflicts_with = "test")]
        practice: bool,
        /// An organizer test run: full length like a competition run, opened only by
        /// organizer accounts, and deleted from the server in one go afterwards. Not for
        /// participants.
        #[arg(long)]
        test: bool,
    },
    /// Show how your run is doing: recording state, health, storage and any gaps. Run it in a
    /// second terminal while the agent works.
    Status {
        /// The entry id (`goalcup entries` lists them). Defaults to the newest run started on
        /// this machine.
        #[arg(value_name = "ENTRY")]
        entry: Option<String>,
        /// Print the signed status document as JSON, and nothing else.
        #[arg(long)]
        json: bool,
    },
    /// Show how much time your run has left. Your agent may run it too; goalcup never tells
    /// the agent anything by itself.
    ///
    /// It reads the count the running `goalcup start` keeps from the server, and works inside
    /// the agent's session. When that count is missing or older than five minutes, it says the
    /// time is unknown right now and exits 1.
    #[command(name = "time-left")]
    TimeLeft,
    /// Start your agent again, when it stopped before the run's time was up.
    ///
    /// An agent that exits, a closed window or a restart ends one part of your run. The run
    /// stays open until its end time. This starts the next part on this computer, in the run's
    /// folder, and the agent picks up its last session (a run with no session yet starts a new
    /// one). Nothing is approved again: this computer kept the approval for this run. You can
    /// continue a run only on the computer that started it.
    Continue {
        /// The run to continue. Defaults to the newest run started on this machine.
        #[arg(value_name = "RUN_ID")]
        run: Option<String>,
    },
    /// Send the rest of a run to the Goal Cup server, when `goalcup start` could not: the
    /// network was down, the laptop slept, or the process was stopped. Safe to run again.
    Upload {
        /// The entry id (`goalcup entries` lists them). Defaults to every entry of the newest run
        /// started on this machine that is not yet sent.
        #[arg(value_name = "ENTRY")]
        entry: Option<String>,
    },
    /// List the recorded parts (entries) of your runs on this computer, sealed ones included.
    Entries {
        /// Print the list as one JSON array.
        #[arg(long)]
        json: bool,
    },
    /// End a run early: stop the agent, seal the record, and print where the trace is.
    ///
    /// `goalcup start` seals on its own when the agent exits, so this is only for stopping a
    /// run before then. goalcup records through a small background program, `goalcup-daemon`
    /// (beside this binary). It exits once every run is sealed. `status` and `entries` start
    /// it again to answer; it stays up while a run whose daemon died is unsealed, until you run
    /// `goalcup seal`.
    Seal {
        /// The entry id. Defaults to the newest run started on this machine.
        #[arg(value_name = "ENTRY")]
        entry: Option<String>,
    },
    /// Check that goalcup can record on this computer and that each agent is installed. Not one
    /// of the steps above: use it when something does not work.
    #[command(after_help = DOCTOR_AFTER_HELP)]
    Doctor {
        /// Print the report as one JSON document instead of text.
        #[arg(long)]
        json: bool,
        /// Check this harness only, and set the exit code from it alone. Without it the harness
        /// you chose in `goalcup prep` is checked. `--harness all` checks every shipped harness,
        /// and any failure makes the exit code 1.
        #[arg(long, value_name = "ID")]
        harness: Option<String>,
    },
    /// The command git runs to ask for the GitHub token of your run's repository. It is set
    /// in each run's clone by `goalcup start`; you never run it yourself.
    #[command(name = "git-credential", hide = true)]
    GitCredential {
        /// What git asks for: `get`, `store` or `erase`.
        #[arg(value_name = "OPERATION", value_parser = ["get", "store", "erase"])]
        operation: String,
    },
}

fn main() -> ExitCode {
    // A test build can shorten the local-finish budget, so that a test of a finish that is cut
    // need not wait three minutes. A shipped build has no such door.
    #[cfg(feature = "test-support")]
    if let Some(ms) = std::env::var("GOALCUP_TEST_FINISH_BUDGET_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
    {
        goalcup_cli::set_finish_budget(std::time::Duration::from_millis(ms));
    }
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => {
            let usage_error = e.use_stderr();
            // clap writes `--help` and `--version` to stdout and a usage error to stderr,
            // and `print()` honours that split. A failure to print is not worth a second
            // failure on top of the first.
            let _ = e.print();
            return ExitCode::from(if usage_error { 1 } else { 0 });
        }
    };
    match run(args.command) {
        // `goalcup doctor` answers its own 0 or 1; every other arm of `run` answers 0.
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            // One line, on stderr, prefixed with the program's name, and never a panic, even on
            // a terminal that was closed under a chat-mode run (`host_log`). Nothing here
            // formats a document that holds a secret: `ControlFile` and the control API's own
            // types redact their bearers in `Debug`, and this prints `Display`.
            goalcup_cli::host_log::say(&e.to_string());
            ExitCode::from(e.exit_code())
        }
    }
}

fn run(command: Command) -> Result<u8, CliError> {
    match command {
        Command::Start {
            harness,
            dev,
            goal_file,
            practice,
            test,
        } => start(commands::start::StartOptions {
            harness,
            dev,
            goal_file,
            run_folder: std::env::current_dir().ok(),
            disk: disk_policy()?,
            mode: if test {
                goalcup_cli::account::device::RunMode::Test
            } else if practice {
                goalcup_cli::account::device::RunMode::Practice
            } else {
                goalcup_cli::account::device::RunMode::Competition
            },
            open_url: browser_opener(),
            upload_policy: upload_policy()?,
            sleep_lock: goalcup_platform::awake::hold,
        })
        .map(|()| 0),
        Command::Continue { run } => continue_run(run).map(|()| 0),
        Command::Status { entry, json } => status(entry, json).map(|()| 0),
        Command::TimeLeft => commands::time_left::time_left_with(
            std::env::var_os(goalcup_cli::run::time_left::FILE_ENV).as_deref(),
            goalcup_cli::run::now_unix(),
            &mut std::io::stdout(),
        )
        .map(|()| 0),
        Command::Entries { json } => entries(json).map(|()| 0),
        Command::Seal { entry } => seal(entry).map(|()| 0),
        Command::Doctor { json, harness } => {
            doctor(commands::doctor::DoctorOptions { json, harness })
        }
        Command::Login { server } => login(server).map(|()| 0),
        Command::Prep {
            harness,
            repo,
            work_root,
            json,
            dev,
        } => prep(commands::prep::PrepOptions {
            harness,
            repo,
            work_root,
            json,
            dev,
        }),
        Command::GitCredential { operation } => {
            let cfg = goalcup_cli::config::config_dir()?;
            goalcup_cli::account::github::credential_helper(
                &cfg,
                &operation,
                &mut std::io::stdin().lock(),
                &mut std::io::stdout(),
            )
            .map(|()| 0)
        }
        Command::Upload { entry } => {
            let cfg = goalcup_cli::config::config_dir()?;
            let policy = upload_policy()?;
            let out = &mut std::io::stdout();
            match entry.as_deref() {
                Some(entry_id) => commands::upload::upload_with(&cfg, entry_id, policy, out),
                None => commands::upload::upload_newest_with(&cfg, policy, out),
            }
            .map(|()| 0)
        }
    }
}

/// When chunks go and how failures wait: the shipped policy, or in a test build one named by
/// [`TEST_UPLOAD_POLICY`].
fn upload_policy() -> Result<goalcup_cli::upload::uploader::Policy, CliError> {
    #[cfg(feature = "test-support")]
    if let Ok(spec) = std::env::var(TEST_UPLOAD_POLICY) {
        return goalcup_cli::upload::uploader::Policy::from_test_spec(&spec)
            .ok_or_else(|| CliError::Refused(format!("{TEST_UPLOAD_POLICY} is malformed")));
    }
    Ok(goalcup_cli::upload::uploader::Policy::default())
}

/// The sizes of the run's disk reservation: the shipped ones, or in a test build ones named by
/// [`TEST_DISK_POLICY`], so that no test holds 3 GiB.
fn disk_policy() -> Result<goalcup_cli::run::reserve::DiskPolicy, CliError> {
    #[cfg(feature = "test-support")]
    if let Ok(spec) = std::env::var(TEST_DISK_POLICY) {
        return goalcup_cli::run::reserve::DiskPolicy::from_test_spec(&spec)
            .ok_or_else(|| CliError::Refused(format!("{TEST_DISK_POLICY} is malformed")));
    }
    Ok(goalcup_cli::run::reserve::DiskPolicy::default())
}

/// A test build's disk sizes: `reserve_bytes,start_margin_bytes,tick_ms`, then optionally
/// `free_floor_bytes`, after it `chain_quota_bytes`, and after that
/// `pause_below_bytes,resume_above_bytes` (without them the agent is never paused, D65).
#[cfg(feature = "test-support")]
const TEST_DISK_POLICY: &str = "GOALCUP_TEST_DISK";

/// A test build's upload timing, so a test can see an hour's cadence in a second.
#[cfg(feature = "test-support")]
const TEST_UPLOAD_POLICY: &str = "GOALCUP_TEST_UPLOAD_POLICY";

/// What opens the approval address: the participant's browser, or nothing when `GOALCUP_NO_BROWSER`
/// is set to a non-empty value (in every build) or, in a test build, [`TEST_NO_BROWSER`] is set
/// (a test must never start a browser on its machine). The address and code are printed either
/// way, so the participant, or `scripts/matrix.sh` with its test approval key, approves them.
fn browser_opener() -> fn(&str) -> bool {
    #[cfg(feature = "test-support")]
    if std::env::var_os(TEST_NO_BROWSER).is_some() {
        return |_| false;
    }
    browser_opener_unless(no_browser_set(std::env::var_os(NO_BROWSER).as_deref()))
}

/// Whether `GOALCUP_NO_BROWSER` asks for no browser: set to something. An empty value is not a
/// request (`GOALCUP_NO_BROWSER= goalcup ...` is how a shell clears it).
fn no_browser_set(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|v| !v.is_empty())
}

fn browser_opener_unless(no_browser: bool) -> fn(&str) -> bool {
    if no_browser {
        return |_| false;
    }
    goalcup_platform::machine::open_url
}

/// The shipped switch that keeps `goalcup` from opening a browser.
const NO_BROWSER: &str = "GOALCUP_NO_BROWSER";

/// A test build's door that keeps a test's `goalcup` from opening a real browser.
#[cfg(feature = "test-support")]
const TEST_NO_BROWSER: &str = "GOALCUP_TEST_NO_BROWSER";

/// `goalcup login`: approve this tracker's account in the browser, and store the server and
/// the account's name. Nothing secret is read or kept.
fn login(server: Option<String>) -> Result<(), CliError> {
    let cfg = goalcup_cli::config::config_dir()?;
    commands::login::run(
        &commands::login::LoginArgs { server },
        &cfg,
        &browser_opener(),
        &mut std::io::stdout(),
    )
}

/// `goalcup prep`, with what it reads from this machine resolved here, as `start` does. It
/// starts no daemon: it records nothing.
fn prep(opts: commands::prep::PrepOptions) -> Result<u8, CliError> {
    let cfg = goalcup_cli::config::config_dir()?;
    let connectors = goalcup_cli::config::connectors_dir()?;
    let env = goalcup_cli::launch::process_env();
    let mut out = std::io::stdout();
    #[cfg(feature = "test-support")]
    if let Ok(spec) = std::env::var(commands::prep::Injected::VAR) {
        let door = commands::prep::Injected::parse(&spec).ok_or_else(|| {
            CliError::Refused(format!("{} is malformed", commands::prep::Injected::VAR))
        })?;
        return commands::prep::prep_with(&door, &cfg, &connectors, &env, &opts, &mut out);
    }
    commands::prep::prep_with(
        &commands::prep::Here,
        &cfg,
        &connectors,
        &env,
        &opts,
        &mut out,
    )
}

/// `goalcup start`, with the four things it reads from this machine resolved here: the
/// configuration directory, the connectors directory, the environment, and a client for the
/// daemon — the one `control.json` names if it answers, else one started for this run from
/// `<cfg>/daemon.toml`, which is written with local defaults if it is missing.
///
/// The same division of labour `doctor` below takes, and for the same reason —
/// `commands::start::start_with` takes all four as parameters, so it can be driven against
/// the in-process mock and a temporary directory without this process's environment being
/// mutated by a test.
///
/// The harness's own exit code is not this CLI's: a run whose agent exited non-zero is still
/// a run that was recorded, and `goalcup start` reports the ladder in `lib.rs` and nothing
/// else. `StartOutcome` is dropped here; what it names is on the screen already.
fn start(opts: commands::start::StartOptions) -> Result<(), CliError> {
    let cfg = goalcup_cli::config::config_dir()?;
    let connectors = goalcup_cli::config::connectors_dir()?;
    // `--goal-file` is for a headless harness only, and a headless one needs it: said before a
    // daemon is started or anything else is done.
    commands::start::goal_file_fits_connector(&connectors, &opts)?;
    let client = ensure_daemon(
        &cfg,
        goalcup_cli::daemon::MissingConfig::WriteDefault {
            connectors: &connectors,
        },
    )?;
    // Linux: before the gated spawn, so what the harness leaves behind is reparented here and
    // swept before the seal (`subreaper.rs`). Only this binary arms it, never an in-process start.
    if let Err(e) = goalcup_cli::subreaper::arm() {
        eprintln!(
            "goalcup: this command could not become a child subreaper ({e}), so a process the \
             harness leaves behind after it exits is not swept before the seal, and the daemon's \
             walk may not see it (docs/TERMINATION.md)."
        );
    }
    let env = goalcup_cli::launch::process_env();
    #[cfg(feature = "test-support")]
    if let Some(pem) = std::env::var_os(TEST_UPSTREAM_ROOTS) {
        let roots = test_upstream_roots(std::path::Path::new(&pem))?;
        commands::start::start_with_upstream_roots(
            &client,
            &connectors,
            &cfg,
            &env,
            &opts,
            roots,
            &mut std::io::stdout(),
        )?;
        return Ok(());
    }
    commands::start::start_with(
        &client,
        &connectors,
        &cfg,
        &env,
        &opts,
        &mut std::io::stdout(),
    )?;
    Ok(())
}

/// `goalcup continue`, with what it reads from this machine resolved here, as `start` does. A run
/// this machine did not start is refused before a daemon is asked for or started.
fn continue_run(run: Option<String>) -> Result<(), CliError> {
    let cfg = goalcup_cli::config::config_dir()?;
    let connectors = goalcup_cli::config::connectors_dir()?;
    commands::continue_run::resolve(&cfg, run.as_deref())?;
    let client = ensure_daemon(&cfg, goalcup_cli::daemon::MissingConfig::Refuse)?;
    if let Err(e) = goalcup_cli::subreaper::arm() {
        eprintln!(
            "goalcup: this command could not become a child subreaper ({e}), so a process the \
             harness leaves behind after it exits is not swept before the seal, and the daemon's \
             walk may not see it (docs/TERMINATION.md)."
        );
    }
    let env = goalcup_cli::launch::process_env();
    commands::continue_run::continue_with(
        &client,
        &connectors,
        &cfg,
        &env,
        &commands::continue_run::ContinueOptions {
            run,
            run_folder: std::env::current_dir().ok(),
            disk: disk_policy()?,
            upload_policy: upload_policy()?,
            sleep_lock: goalcup_platform::awake::hold,
        },
        &mut std::io::stdout(),
    )
}

/// A test build's one extra input to `goalcup start`: a PEM file of certificates the recording
/// proxy trusts upstream beside the public roots, so a synthetic local upstream can be reached. A
/// shipped build compiles none of this.
#[cfg(feature = "test-support")]
const TEST_UPSTREAM_ROOTS: &str = "GOALCUP_TEST_UPSTREAM_ROOTS";

/// The public roots and every certificate in `pem`, or a refusal naming the variable.
#[cfg(feature = "test-support")]
fn test_upstream_roots(
    pem: &std::path::Path,
) -> Result<goalcup_cli::proxy::tls::UpstreamRoots, CliError> {
    use rustls::pki_types::CertificateDer;
    use rustls::pki_types::pem::PemObject;
    let refused = |why: String| {
        CliError::Refused(format!(
            "{TEST_UPSTREAM_ROOTS} names {}, which {why}.",
            pem.display()
        ))
    };
    let certificates = CertificateDer::pem_file_iter(pem)
        .and_then(|certificates| certificates.collect::<Result<Vec<_>, _>>())
        .map_err(|e| refused(format!("is not a readable PEM file of certificates ({e})")))?;
    if certificates.is_empty() {
        return Err(refused("holds no certificate".to_string()));
    }
    goalcup_cli::proxy::tls::UpstreamRoots::webpki()
        .with_extra(certificates)
        .map_err(|e| {
            refused(format!(
                "holds a certificate that is not a usable root ({e})"
            ))
        })
}

/// `goalcup status`, with the three things it reads resolved here: the configuration
/// directory the pinned run key is in, a client for the daemon, and this machine's clock.
///
/// **The daemon's registry is the one source of truth.** The local record is looked up first:
/// a run this machine never started is refused without asking, or starting, anything. Then the
/// daemon that answers is asked, or the run-scoped one is started from `daemon.toml`, as `seal`
/// does. A sealed run is answered after its own daemon is long gone, and a daemon started for
/// it exits again on its 30-second grace.
///
/// One write path, accepted: a run whose daemon died mid-run is unfinished, so the daemon
/// started here reopens it for status and seal only and chains the lost interval as one gap.
/// That records a real loss rather than one this look caused, and that daemon then **stays up
/// until `goalcup seal`** finishes the run (an unfinished capture is work), so the seal chains
/// no second gap.
///
/// The clock is read here and handed over as a string, so the rendering itself is a
/// function of its arguments and a test can pin what a stale document looks like without
/// waiting two minutes for one.
fn status(entry: Option<String>, json: bool) -> Result<(), CliError> {
    let cfg = goalcup_cli::config::config_dir()?;
    commands::status::pinned(&cfg, entry.as_deref())?;
    let client = match daemon_to_read(&cfg)? {
        Some(client) => client,
        // A record but no configuration: say there is no daemon, naming `control.json`.
        None => goalcup_cli::daemon::HttpDaemonClient::from_config_dir(&cfg)?,
    };
    // The connectors give the harness its display name; without them the status names its id.
    let connectors = goalcup_cli::config::connectors_dir().ok();
    commands::status::status_in(
        &client,
        &cfg,
        connectors.as_deref(),
        entry.as_deref(),
        json,
        &now_z(),
        &mut std::io::stdout(),
    )
}

/// `goalcup seal`, with the three things it reads from this machine resolved here: the
/// configuration directory, the connectors directory, and the client the running daemon's
/// `control.json` describes.
///
/// The default entry is the newest run **started from this machine**, the same default
/// `goalcup status` takes and for the same reason: the connector the run used is in the
/// record this machine wrote, and no other process holds it.
///
/// The client is built before the entry is resolved: a seal's first need is a daemon, so a
/// machine with none is told about `control.json` rather than about which entry it failed to
/// name. A run that outlived its daemon (a crash, a reboot) is why this starts one from
/// `<cfg>/daemon.toml` when none answers: that daemon reopens the run for status and seal. It
/// never writes the file: with no configuration, no daemon was ever started from here.
fn seal(entry: Option<String>) -> Result<(), CliError> {
    commands::seal::refuse_inside_harness(
        std::env::var_os(goalcup_cli::launch::IN_HARNESS_ENV).as_deref(),
    )?;
    let cfg = goalcup_cli::config::config_dir()?;
    let connectors = goalcup_cli::config::connectors_dir()?;
    let client = ensure_daemon(&cfg, goalcup_cli::daemon::MissingConfig::Refuse)?;
    let entry_id = resolve_entry(&cfg, entry)?;
    commands::seal::seal_with(
        &client,
        &connectors,
        &cfg,
        &entry_id,
        &mut std::io::stdout(),
    )
}

/// The entry named on the command line, or the newest run this machine started.
///
/// Only a run started here can be sealed here: the connector it ran under lives in that record
/// and nowhere else this process can reach.
fn resolve_entry(cfg: &std::path::Path, entry: Option<String>) -> Result<String, CliError> {
    match entry {
        Some(entry_id) => Ok(entry_id),
        None => goalcup_cli::entries::latest_entry(cfg)?
            .map(|record| record.entry_id)
            .ok_or_else(|| {
                CliError::Refused(format!(
                    "no run has been started from this machine: there is no record under {}. \
                     Only a run started here can be sealed here, because the workspace to \
                     scan and the tracker's own file list are in that record.",
                    goalcup_cli::entries::entries_dir(cfg).display()
                ))
            }),
    }
}

/// `goalcup entries`: every run the registry holds, sealed ones included. Like `status`, it
/// starts the run-scoped daemon from `daemon.toml` when none answers; with no `daemon.toml`
/// no daemon was ever started here, and the answer is the empty list, with nothing started.
fn entries(json: bool) -> Result<(), CliError> {
    let cfg = goalcup_cli::config::config_dir()?;
    let mut out = std::io::stdout();
    match daemon_to_read(&cfg)? {
        Some(client) => commands::entries::entries_with(&client, json, &mut out),
        None => commands::entries::render_entries(&[], json, &mut out),
    }
}

/// Say, on stderr, that a command is still waiting for a daemon that is reopening a run.
fn say_waiting(line: &str) {
    eprintln!("goalcup: {line}");
}

/// A client for the daemon a command needs, started if none answers. A daemon that is gone is
/// waited for [`goalcup_cli::daemon::spawn::SPAWN_WAIT`]; one that is alive and reopening a run
/// for up to [`goalcup_cli::daemon::spawn::DAEMON_READY_BUDGET`], saying so every ten seconds.
fn ensure_daemon(
    cfg: &std::path::Path,
    missing: goalcup_cli::daemon::MissingConfig<'_>,
) -> Result<goalcup_cli::daemon::HttpDaemonClient, CliError> {
    goalcup_cli::daemon::spawn::ensure_daemon_waiting(
        cfg,
        missing,
        None,
        goalcup_cli::daemon::spawn::Wait::SHIPPED,
        &mut say_waiting,
    )
}

/// The daemon `status` and `entries` ask, if there is one to ask, with the same waits.
fn daemon_to_read(
    cfg: &std::path::Path,
) -> Result<Option<goalcup_cli::daemon::HttpDaemonClient>, CliError> {
    goalcup_cli::daemon::spawn::daemon_to_read(
        cfg,
        goalcup_cli::daemon::spawn::Wait::SHIPPED,
        &mut say_waiting,
    )
}

/// This machine's clock, in the one spelling this tree writes a time in.
///
/// A clock somehow before the epoch reads as the epoch rather than panicking: nothing
/// downstream would believe either number, and a status render is not worth a panic.
fn now_z() -> String {
    goalcup_protocol::control::format_z(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    )
}

/// `goalcup doctor`, with everything it reads resolved here, and the reason `run_with` takes
/// them as arguments at all: a check
/// that read the participant's environment itself could not be tested without mutating this
/// process's.
///
/// **The client is built here and a failure to build one is not a failure of the report.**
/// `from_config_dir` refuses a machine with no `control.json` and a daemon
/// speaking a control API this build does not, and both of those are things `goalcup
/// doctor` exists to tell the participant — so the error is dropped and `None` is handed
/// over, whereupon `daemon_check` reads that same file and says what it found. A `?` here
/// would have made the one command a participant runs when nothing works the one command
/// that refuses to run when nothing works.
///
/// Only stdout is handed in: nothing this command prints goes to stderr.
fn doctor(opts: commands::doctor::DoctorOptions) -> Result<u8, CliError> {
    let cfg = goalcup_cli::config::config_dir()?;
    let connectors = goalcup_cli::config::connectors_dir()?;
    let mut out = std::io::stdout();
    let client = goalcup_cli::daemon::HttpDaemonClient::from_config_dir(&cfg).ok();
    let client = client
        .as_ref()
        .map(|c| c as &dyn goalcup_protocol::control::DaemonClient);
    let env = goalcup_cli::launch::process_env();
    commands::doctor::run_with(client, &connectors, &cfg, &env, &opts, &mut out)
}

#[cfg(test)]
mod tests {
    /// `GOALCUP_NO_BROWSER` is how a scripted approval (`scripts/matrix.sh`) keeps one browser
    /// page per approval from opening on the machine; the opener it picks must not be the real one.
    #[test]
    fn the_no_browser_switch_replaces_the_real_opener() {
        let real = goalcup_platform::machine::open_url as fn(&str) -> bool;
        assert_eq!(super::browser_opener_unless(false) as usize, real as usize);
        let off = super::browser_opener_unless(true);
        assert_ne!(off as usize, real as usize);
        assert!(!off("https://example.invalid/approve"));
    }

    #[test]
    fn only_a_non_empty_value_turns_the_browser_off() {
        use std::ffi::OsStr;
        assert!(!super::no_browser_set(None));
        assert!(!super::no_browser_set(Some(OsStr::new(""))));
        assert!(super::no_browser_set(Some(OsStr::new("1"))));
    }
}
