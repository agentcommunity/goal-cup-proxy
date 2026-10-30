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
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use goalcup::cli::{self, NULL_FLAG, RECORDER_SUBCOMMAND, say, text};

/// Record one coding agent's run for the Goal Cup.
#[derive(Parser)]
#[command(
    name = "goalcup",
    version,
    after_help = text::after_help(),
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
    #[command(name = RECORDER_SUBCOMMAND, hide = true)]
    Recorder {
        /// Null mode: forward without recording.
        #[arg(long = NULL_FLAG.trim_start_matches('-'))]
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

/// Help and version on stdout with 0; no command at all prints the help on stderr with 1; every
/// other parse error is one line with 1.
fn parse_error(e: clap::Error) -> ExitCode {
    match e.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            let _ = e.print();
            ExitCode::SUCCESS
        }
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            let _ = e.print();
            ExitCode::from(1)
        }
        _ => {
            let text = e.to_string();
            let first = text.lines().next().unwrap_or_default();
            say(first.strip_prefix("error: ").unwrap_or(first));
            ExitCode::from(1)
        }
    }
}

fn run(command: Command) -> ExitCode {
    match command {
        Command::Recorder { null } => goalcup::pipeline::run_recorder(null),
        Command::Start {
            practice,
            test,
            repo,
            prompt,
            connector,
            hosts,
            ..
        } => {
            // Refused before anything is read or created.
            if (connector.is_some() || hosts.is_some()) && !practice && !test {
                say(text::FILE_FLAGS_COMPETE);
                return ExitCode::from(1);
            }
            // The built `start` keeps PRD-10 section 3's order: the connector (step 1) first,
            // then these folder checks (step 2). Nothing is created before or during them.
            let home = match cli::home() {
                Ok(home) => home,
                Err(e) => {
                    say(e);
                    return ExitCode::from(1);
                }
            };
            let Ok(project) = std::env::current_dir() else {
                say(text::NO_CWD);
                return ExitCode::from(1);
            };
            match cli::folder::check(&project, &home, repo.is_some(), prompt.as_deref()) {
                Err(problems) => {
                    say(text::FOLDER_REFUSED);
                    let mut err = std::io::stderr().lock();
                    for p in problems {
                        let _ = writeln!(err, "  {p}");
                    }
                    let _ = writeln!(err, "{}", text::NOTHING_CREATED);
                    return ExitCode::from(1);
                }
                Ok(warnings) => {
                    for w in warnings {
                        say(&w);
                    }
                }
            }
            not_built("start")
        }
        Command::Setup { .. } => not_built("setup"),
        Command::Continue { .. } => not_built("continue"),
        Command::Recover { .. } => not_built("recover"),
        Command::Status { part, json } => cli::status::run(part, json),
        Command::Seal { .. } => not_built("seal"),
        Command::Upload { .. } => not_built("upload"),
        Command::TimeLeft => not_built("time-left"),
    }
}

/// Phase 0 stub: the core slice moves each body into `src/cli/`.
fn not_built(command: &str) -> ExitCode {
    say(&text::not_built(command));
    ExitCode::from(3)
}

#[cfg(test)]
mod tests {
    use super::{Args, Command, NULL_FLAG, RECORDER_SUBCOMMAND};
    use clap::Parser;

    /// The argv the watchdog spawns (seam S8) parses as the hidden recorder in null mode.
    #[test]
    fn watchdog_argv_parses() {
        let args = Args::try_parse_from(["goalcup", RECORDER_SUBCOMMAND, NULL_FLAG]).unwrap();
        assert!(matches!(args.command, Command::Recorder { null: true }));
    }
}
