//! `goalcup`: the participant's command line (PRD-10 section 1).
//!
//! Eight commands and one hidden one. The hidden `recorder` is the recorder process the watchdog
//! starts (PRD-02 section 8 step 4, seam S8); it is dispatched to `pipeline::run_recorder` and
//! never shows in help.
//!
//! Exit codes: 0 done, 1 refused and fixable, 3 could not do its job. The parse is
//! `Args::try_parse()`, never `parse()`: clap's own exit 2 is not one of the three, so every
//! parse error leaves with 1. `--help` and `--version` print on stdout and leave with 0. Every
//! error is one line `goalcup: <text>` on stderr.

use clap::error::ErrorKind;
use clap::{Parser, Subcommand};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

/// The footer of `goalcup --help` (PRD-10 section 11).
const AFTER_HELP: &str = "\
Steps:
  1. goalcup setup                                   once, until it says Ready.
  2. goalcup start --practice                         a 10-minute try, no GitHub (optional)
  3. goalcup start --repo <you>/<repo> --prompt FILE  your run, in the start window; FILE outside the folder
While it runs: goalcup status (in another terminal). If status says so: goalcup recover.
If the agent stopped early: goalcup continue.
Guide: https://github.com/agentcommunity/goal-cup-proxy/blob/main/docs/PARTICIPANT-GUIDE.md
Help: goalcup@agentcommunity.org
Exit codes: 0 done; 1 refused, and you can fix it; 3 the command could not do its job.";

/// PRD-10 section 1, "File connectors in competition". Moves to `cli/text.rs` in the core slice.
const FILE_FLAGS_COMPETE: &str = "--connector and --hosts are for practice and test runs. \
A competition run uses the connectors built into goalcup.";

/// Record one coding agent's run for the Goal Cup.
#[derive(Parser)]
#[command(
    name = "goalcup",
    version,
    after_help = AFTER_HELP,
    subcommand_required = true,
    disable_help_subcommand = true
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Step 1. Get this computer ready: pick your agent and sign in to it once.
    Setup {
        /// Your agent (the harness). Without it, the one you chose before, or it asks.
        #[arg(long, value_name = "ID")]
        harness: Option<String>,
        /// Ask nothing and open nothing: write the settings and print the agent's home.
        #[arg(long)]
        yes: bool,
        /// Add a model host to your list, one goalcup records.
        #[arg(long, value_name = "HOST")]
        record_host: Option<String>,
    },
    /// Step 2. Start a run: open your agent here and record what it does.
    Start {
        /// Your agent (the harness). Without it, the one setup saved.
        #[arg(long, value_name = "ID")]
        harness: Option<String>,
        /// A practice run: kept apart from the competition, no GitHub repository.
        #[arg(long)]
        practice: bool,
        /// An organizer test run. Not for participants.
        #[arg(long)]
        test: bool,
        /// This run's new, empty, public GitHub repository: <owner>/<name>.
        #[arg(long, value_name = "NAME")]
        repo: Option<String>,
        /// The first message for your agent, from a file outside the project folder.
        #[arg(long, value_name = "FILE")]
        prompt: Option<PathBuf>,
        /// A connector from a file (practice and test runs only).
        #[arg(long, value_name = "FILE")]
        connector: Option<PathBuf>,
        /// A host list from a file (practice and test runs only).
        #[arg(long, value_name = "FILE")]
        hosts: Option<PathBuf>,
    },
    /// Start your agent again when it stopped before the run's time was up.
    Continue {
        /// The run to continue. Defaults to the newest run on this computer.
        #[arg(value_name = "RUN")]
        run: Option<String>,
        /// The connector file of the run's first part, if it used one.
        #[arg(long, value_name = "FILE")]
        connector: Option<PathBuf>,
        /// The host list file of the run's first part, if it used one.
        #[arg(long, value_name = "FILE")]
        hosts: Option<PathBuf>,
    },
    /// Restart recording when status says recording stopped and your agent still runs.
    Recover {
        /// The run to recover. Defaults to the newest run on this computer.
        #[arg(value_name = "RUN")]
        run: Option<String>,
    },
    /// Show how your run is doing and list its parts.
    Status {
        /// One part of a run. Defaults to the newest run on this computer.
        #[arg(value_name = "PART")]
        part: Option<String>,
        /// Print the status as JSON, and nothing else.
        #[arg(long)]
        json: bool,
    },
    /// End a part now: stop recording, finish its record files and upload them.
    Seal {
        /// The part to seal. Defaults to the open part of the newest run.
        #[arg(value_name = "PART")]
        part: Option<String>,
        /// Do not ask first.
        #[arg(long)]
        yes: bool,
    },
    /// Send what is not uploaded yet. Safe to run again.
    Upload {
        /// One part of a run. Defaults to every part not yet sent.
        #[arg(value_name = "PART")]
        part: Option<String>,
    },
    /// Show how much time your run has left. Your agent may run it too.
    TimeLeft,
    /// The recorder process the watchdog starts (seam S8). Never run by hand.
    #[command(hide = true)]
    Recorder {
        /// Null mode: forward without recording.
        #[arg(long)]
        null: bool,
    },
}

fn main() -> ExitCode {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => return parse_error(e),
    };
    run(args.command)
}

/// Help and version on stdout with 0; every other parse error is one line with 1.
fn parse_error(e: clap::Error) -> ExitCode {
    if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
        let _ = e.print();
        return ExitCode::SUCCESS;
    }
    let text = e.to_string();
    let first = text.lines().next().unwrap_or_default();
    say(first.strip_prefix("error: ").unwrap_or(first));
    ExitCode::from(1)
}

fn run(command: Command) -> ExitCode {
    match command {
        Command::Recorder { null } => goalcup::pipeline::run_recorder(null),
        Command::Start {
            harness,
            practice,
            test,
            repo,
            prompt,
            connector,
            hosts,
        } => {
            // Refused before anything is read or created.
            if (connector.is_some() || hosts.is_some()) && !practice && !test {
                say(FILE_FLAGS_COMPETE);
                return ExitCode::from(1);
            }
            let _ = (harness, repo, prompt);
            not_built("start")
        }
        Command::Setup {
            harness,
            yes,
            record_host,
        } => {
            let _ = (harness, yes, record_host);
            not_built("setup")
        }
        Command::Continue {
            run,
            connector,
            hosts,
        } => {
            let _ = (run, connector, hosts);
            not_built("continue")
        }
        Command::Recover { run } => {
            let _ = run;
            not_built("recover")
        }
        Command::Status { part, json } => {
            let _ = (part, json);
            not_built("status")
        }
        Command::Seal { part, yes } => {
            let _ = (part, yes);
            not_built("seal")
        }
        Command::Upload { part } => {
            let _ = part;
            not_built("upload")
        }
        Command::TimeLeft => not_built("time-left"),
    }
}

/// Phase 0 stub: the core slice moves each body into `src/cli/`.
fn not_built(command: &str) -> ExitCode {
    say(&format!("{command} is not built yet"));
    ExitCode::from(3)
}

/// One line on stderr. A closed stderr is not worth a panic.
fn say(text: &str) {
    let _ = writeln!(std::io::stderr(), "goalcup: {text}");
}
