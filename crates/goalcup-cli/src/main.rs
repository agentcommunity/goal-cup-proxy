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

/// Record one coding agent's run for the /goal Cup: launch the harness on your goal, record what
/// it does, and seal the record when it finishes.
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
    // So the list under `Commands:` is exactly the five sub-commands below. clap's built-in
    // `help` sub-command would be a sixth line in it, indistinguishable to a reader — and to a
    // test — from a sub-command this tree decided to ship. `goalcup <command> --help` is
    // the same help by the route the other five already take.
    disable_help_subcommand = true
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

/// Every `goalcup --help` footer: where verification lives, and the rungs a capture run can
/// leave under.
const AFTER_HELP: &str = "\
Start a run with `goalcup start --harness <id> --goal-file <file>`. It starts the recording \
daemon (`goalcup-daemon`, beside this binary) for the run, and the daemon exits once every run \
is sealed.

To check a finished run, use the separate `goalcup-verify --capture-integrity <trace-dir>`; \
`goalcup start` prints that line for you. There is no `goalcup verify`.

Exit codes: 0 done; 1 refused, and you can fix it (no daemon to ask, the tracker still busy, \
the run already sealed, a mistyped command); 3 this command could not do its job (a file it \
could not read, an answer it could not parse). This command never exits with 2 or 4.";

/// What `goalcup doctor --help` says under its flags. There is no verify
/// mode here: a sealed capture is checked by `goalcup-verify --capture-integrity`.
const DOCTOR_AFTER_HELP: &str = "\
For each harness it checks that the connector files are the pinned ones and that the harness \
executable on PATH is the exact build `goalcup start` runs (hashed, not started).

Exit codes: 0 every check passed; 1 something failed, including a harness that is not installed \
or is not the pinned build. No daemon running is not a failure: `goalcup start` starts one.

Before a run, pass `--harness <id>` for the harness you will start: without it, a harness you \
are not using can make this command exit 1.";

/// The sub-commands: five. `verify` is deliberately absent:
/// see `AFTER_HELP`.
#[derive(Subcommand)]
enum Command {
    /// Run the harness on a goal file, record the run, and seal it when the harness exits.
    Start {
        /// Which harness to run, for example `claude-code-cli` or `codex-cli` (`goalcup doctor`
        /// lists every one this build ships).
        #[arg(long, value_name = "ID")]
        harness: String,
        /// For connector developers: accept a connector directory this build does not pin. The
        /// run says so, and it does not count as a competition entry.
        #[arg(long)]
        dev: bool,
        /// A text file holding your goal. Its text is the agent's first and only prompt. The
        /// run's folder is created next to this file, so put it in an empty folder of its own,
        /// outside any git repository.
        #[arg(long, value_name = "PATH", required = true)]
        goal_file: Option<std::path::PathBuf>,
        /// Disk space, in bytes, reserved for the recording before the harness starts.
        #[arg(
            long,
            value_name = "BYTES",
            requires = "goal_file",
            default_value_t = goalcup_protocol::capture::DEFAULT_CAPTURE_RESERVE_BYTES
        )]
        capture_reserve_bytes: u64,
        /// claude-code-cli only: let the agent run one Python script without asking, written
        /// `Bash(python3 <name>.py *)`. Applies to this run only.
        #[arg(long, value_name = "RULE", requires = "goal_file")]
        capture_claude_allowed_tool: Option<String>,
    },
    /// Show how a run is doing: recording state, health, storage and any gaps.
    Status {
        /// The entry id (`goalcup entries` lists them). Defaults to the newest run started on
        /// this machine.
        #[arg(value_name = "ENTRY")]
        entry: Option<String>,
        /// Print the signed status document as JSON, and nothing else.
        #[arg(long)]
        json: bool,
    },
    /// List the runs the daemon is holding right now.
    Entries {
        /// Print the list as one JSON array.
        #[arg(long)]
        json: bool,
    },
    /// End a run early: stop the harness, seal the record, and print where the trace is.
    ///
    /// `goalcup start` seals on its own when the harness exits, so this is only for stopping a
    /// run before then.
    Seal {
        /// The entry id. Defaults to the newest run started on this machine.
        #[arg(value_name = "ENTRY")]
        entry: Option<String>,
    },
    /// Check this machine: can a daemon run, and is each harness installed at its pinned build.
    #[command(after_help = DOCTOR_AFTER_HELP)]
    Doctor {
        /// Print the report as one JSON document instead of text.
        #[arg(long)]
        json: bool,
        /// Check this harness only, and set the exit code from it alone. Without it every
        /// shipped harness is checked and any failure makes the exit code 1.
        #[arg(long, value_name = "ID")]
        harness: Option<String>,
    },
}

fn main() -> ExitCode {
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
            // One line, on stderr, prefixed with the program's name, and never a panic.
            // Nothing here formats a document that holds a secret: `ControlFile` and the
            // control API's own types redact their bearers in `Debug`, and this prints
            // `Display`.
            eprintln!("goalcup: {e}");
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
            capture_reserve_bytes,
            capture_claude_allowed_tool,
        } => start(commands::start::StartOptions {
            harness,
            dev,
            goal_file,
            capture_reserve_bytes,
            capture_claude_allowed_tool,
        })
        .map(|()| 0),
        Command::Status { entry, json } => status(entry, json).map(|()| 0),
        Command::Entries { json } => entries(json).map(|()| 0),
        Command::Seal { entry } => seal(entry).map(|()| 0),
        Command::Doctor { json, harness } => {
            doctor(commands::doctor::DoctorOptions { json, harness })
        }
    }
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
    let client = goalcup_cli::daemon::ensure_daemon(
        &cfg,
        goalcup_cli::daemon::MissingConfig::WriteDefault {
            connectors: &connectors,
        },
        daemon_deadline(),
    )?;
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

/// A test build's one extra input to `goalcup start`: a PEM file of certificates the proxy
/// trusts upstream beside the public roots, so a synthetic local upstream can be reached. A
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
/// directory the pinned run key is in, the client the running daemon's `control.json`
/// describes, and this machine's clock.
///
/// **It starts no daemon**, and neither does `goalcup entries`. A daemon is up for as long as
/// any run is unsealed, so "no daemon is running" is the true answer to "what is being
/// recorded". Starting one here would reopen an unfinished run, and every reopen of a launched
/// run chains a gap: a look must not write. `goalcup seal` is what recovers such a run.
///
/// The clock is read here and handed over as a string, so the rendering itself is a
/// function of its arguments and a test can pin what a stale document looks like without
/// waiting two minutes for one.
fn status(entry: Option<String>, json: bool) -> Result<(), CliError> {
    let cfg = goalcup_cli::config::config_dir()?;
    let client = goalcup_cli::daemon::HttpDaemonClient::from_config_dir(&cfg)?;
    commands::status::status_with(
        &client,
        &cfg,
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
    let cfg = goalcup_cli::config::config_dir()?;
    let connectors = goalcup_cli::config::connectors_dir()?;
    let client = goalcup_cli::daemon::ensure_daemon(
        &cfg,
        goalcup_cli::daemon::MissingConfig::Refuse,
        daemon_deadline(),
    )?;
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

/// `goalcup entries`. It reads nothing off this machine but the daemon's own address.
fn entries(json: bool) -> Result<(), CliError> {
    let cfg = goalcup_cli::config::config_dir()?;
    let client = goalcup_cli::daemon::HttpDaemonClient::from_config_dir(&cfg)?;
    commands::entries::entries_with(&client, json, &mut std::io::stdout())
}

/// How long `start` and `seal` wait for a daemon they had to start.
fn daemon_deadline() -> std::time::Instant {
    std::time::Instant::now() + goalcup_cli::daemon::spawn::SPAWN_WAIT
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
/// Both streams are handed in, because under `--json` the development banner goes to stderr
/// so that stdout is one parseable document. A test that could only capture one of the two
/// could not tell those apart.
fn doctor(opts: commands::doctor::DoctorOptions) -> Result<u8, CliError> {
    let cfg = goalcup_cli::config::config_dir()?;
    let connectors = goalcup_cli::config::connectors_dir()?;
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    let client = goalcup_cli::daemon::HttpDaemonClient::from_config_dir(&cfg).ok();
    let client = client
        .as_ref()
        .map(|c| c as &dyn goalcup_protocol::control::DaemonClient);
    let env = goalcup_cli::launch::process_env();
    commands::doctor::run_with(client, &connectors, &cfg, &env, &opts, &mut out, &mut err)
}
