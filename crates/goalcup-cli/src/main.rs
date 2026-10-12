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

/// Capture a /goal Cup entry: create the workspace, arm the folder, launch the harness, and
/// seal what it produced.
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
Verification is a separate binary: run `goalcup-verify --capture-integrity <trace-dir>` on a \
sealed capture. It is not a sub-command here.

Exit codes: 0 the command did what it was asked; 1 refused, and you can fix it (no daemon, \
a tracker still busy, an entry already sealed, a mistyped sub-command); 3 this CLI could not \
do its job (a file it could not read, an answer it could not parse). 2 is never this \
binary's, and neither is 4: both belonged to the V1 path, which this tree no longer carries \
(D48; the v0.5.0 tag has it).";

/// What `goalcup doctor --help` says under its flags. There is no verify
/// mode here: a sealed capture is checked by `goalcup-verify --capture-integrity`.
const DOCTOR_AFTER_HELP: &str = "\
Exit codes: 0 = every check passed and 1 = something failed (a machine with no daemon \
running is a 1).

--harness <id> narrows the report AND the exit code to one connector, which is the \
question a participant about to start that harness is asking: without it, a connector you \
are not running can fail this command.";

/// The sub-commands: five. `verify` is deliberately absent:
/// see `AFTER_HELP`.
#[derive(Subcommand)]
enum Command {
    /// Record a local capture run: launch the harness headless with a goal file's text, through
    /// the recording proxy, and seal what it produced.
    Start {
        /// Which harness: a connector this build ships (`goalcup doctor` lists them).
        #[arg(long, value_name = "ID")]
        harness: String,
        /// Development only: accept a rulebook this build does not pin. The run says so on
        /// stdout, and its result is not a judged entry's.
        #[arg(long)]
        dev: bool,
        /// Record a local capture run: launch the harness headless with this file's text as
        /// its one goal, through the recording proxy. The run's directory is made beside
        /// the file, so keep it in a directory of its own, outside any repository.
        #[arg(long, value_name = "PATH", required = true)]
        goal_file: Option<std::path::PathBuf>,
        /// With --goal-file: the chain bytes the daemon reserves on disk before the launch.
        #[arg(
            long,
            value_name = "BYTES",
            requires = "goal_file",
            default_value_t = goalcup_protocol::capture::DEFAULT_CAPTURE_RESERVE_BYTES
        )]
        capture_reserve_bytes: u64,
        /// With --goal-file: one bounded Claude Bash command rule for this launch only.
        #[arg(long, value_name = "RULE", requires = "goal_file")]
        capture_claude_allowed_tool: Option<String>,
    },
    /// What one entry's signed status says.
    Status {
        /// The entry. Defaults to the newest run started from this machine.
        #[arg(value_name = "ENTRY")]
        entry: Option<String>,
        /// The signed document itself on stdout, and nothing else.
        #[arg(long)]
        json: bool,
    },
    /// The runs this daemon is carrying right now.
    Entries {
        /// The summaries as one JSON array.
        #[arg(long)]
        json: bool,
    },
    /// Not in this build: the removed V1 sign-in command (D48).
    #[command(hide = true, disable_help_flag = true)]
    Auth {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        _args: Vec<std::ffi::OsString>,
    },
    /// Not in this build: the removed V1 status page (D48).
    #[command(hide = true, disable_help_flag = true)]
    Ui {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        _args: Vec<std::ffi::OsString>,
    },
    /// Not in this build: the removed V1 relaunch (D48).
    #[command(hide = true, disable_help_flag = true)]
    Resume {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        _args: Vec<std::ffi::OsString>,
    },
    /// Record the workspace manifest and end the run.
    Seal {
        /// The entry. Defaults to the newest run started from this machine.
        #[arg(value_name = "ENTRY")]
        entry: Option<String>,
        /// Scan the workspace and print the self-check, and seal nothing. The tracker is
        /// not called and the run goes on.
        #[arg(long)]
        dry_run: bool,
    },
    /// Is the daemon there, and what can it see.
    #[command(after_help = DOCTOR_AFTER_HELP)]
    Doctor {
        /// The report as one JSON document instead of a page of text.
        #[arg(long)]
        json: bool,
        /// Report on this connector alone, and exit on its result alone. Without it every
        /// shipped connector is reported and any failure among them is a 1.
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
        Command::Auth { .. } => Err(v1_only("auth")),
        Command::Ui { .. } => Err(v1_only("ui")),
        Command::Resume { .. } => Err(v1_only("resume")),
        Command::Seal { entry, dry_run } => seal(entry, dry_run).map(|()| 0),
        Command::Doctor { json, harness } => {
            doctor(commands::doctor::DoctorOptions { json, harness })
        }
    }
}

/// The refusal a removed V1 command gets (D48): rung 1, naming the way back.
fn v1_only(command: &str) -> CliError {
    CliError::Refused(format!(
        "`goalcup {command}` belonged to the V1 transcript path, which this tree no longer \
         carries (D48; the v0.5.0 tag has it). A local capture run \
         is `goalcup start --harness <id> --goal-file <file>`; then `goalcup status`, \
         `goalcup entries` and `goalcup seal`."
    ))
}

/// `goalcup start`, with the four things it reads from this machine resolved here: the
/// configuration directory, the connectors directory, the environment, and the client the
/// running daemon's `control.json` describes.
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
    let client = goalcup_cli::daemon::HttpDaemonClient::from_config_dir(&cfg)?;
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
/// `goalcup status` takes and for the same reason: the workspace to scan and the tracker's
/// own file list to scan it against are in the record this machine wrote, and no other
/// process holds them.
///
/// **The client is built even for `--dry-run`.** The dry run makes no call — `seal_with`
/// never reaches the daemon on that path, and `tests/it/seal_scan.rs` asserts it — but this
/// function resolves the same three things whichever way the run goes. A `--dry-run` on a
/// machine with no daemon therefore refuses at rung 1 naming `control.json`, which is the
/// answer every other command gives from that file.
fn seal(entry: Option<String>, dry_run: bool) -> Result<(), CliError> {
    let cfg = goalcup_cli::config::config_dir()?;
    let connectors = goalcup_cli::config::connectors_dir()?;
    // The dry run resolves its entry and leaves before a client is built: building the client
    // first would make `goalcup seal --dry-run` on a machine with no daemon running answer
    // "the tracker is unreachable" — to a question that never reaches the tracker.
    // `dry_run_with` takes no client at all, so the ordering cannot quietly come back.
    //
    // A real seal keeps the old order, and deliberately: its first need is a daemon, so a
    // machine with none is told about `control.json` rather than about which entry it failed
    // to name, the same first answer `goalcup start` and `doctor` give.
    if dry_run {
        let entry_id = resolve_entry(&cfg, entry)?;
        return commands::seal::dry_run_with(&connectors, &cfg, &entry_id, &mut std::io::stdout());
    }
    let client = goalcup_cli::daemon::HttpDaemonClient::from_config_dir(&cfg)?;
    let entry_id = resolve_entry(&cfg, entry)?;
    commands::seal::seal_with(
        &client,
        &connectors,
        &cfg,
        &entry_id,
        dry_run,
        &mut std::io::stdout(),
    )
}

/// The entry named on the command line, or the newest run this machine started.
///
/// Only a run started here can be sealed here: the workspace to scan and the tracker's own
/// file list live in that record and nowhere else this process can reach.
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
    let env = goalcup_cli::launch::process_env();
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    let client = goalcup_cli::daemon::HttpDaemonClient::from_config_dir(&cfg).ok();
    let client = client
        .as_ref()
        .map(|c| c as &dyn goalcup_protocol::control::DaemonClient);
    commands::doctor::run_with(client, &connectors, &cfg, &env, &opts, &mut out, &mut err)
}
