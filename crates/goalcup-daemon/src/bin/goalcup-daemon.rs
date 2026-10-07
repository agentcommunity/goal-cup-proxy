//! `goalcup-daemon` — the capture daemon, in the foreground.
//!
//! Argument parsing, one refusal path, [`Daemon::start`], [`Daemon::run_forever`] and the
//! two signals that stop it. Every one of the things it deliberately does not do is a
//! decision rather than an omission:
//!
//! * **No daemonisation.** No `fork`, no `setsid`, no pidfile. This runs in the foreground
//!   under whatever started it, and does not register itself as a service. `--foreground` is
//!   accepted because callers and scripts will write it, and it names the only mode this
//!   build has.
//! * **No logging.** This crate has no logger, deliberately: the control listener handles
//!   the CLI's bearer token, and a component that can write a request to a log cannot
//!   promise it never wrote one. What the daemon loses is counted on the signed status
//!   document (`StatusPayload::cycles`), which is a participant-readable surface rather
//!   than a file nobody rotates. So `stdout` here carries the two startup facts and
//!   nothing else, and
//!   `stderr` carries a refusal the process is about to exit on — and, when the configured
//!   server key is the published placeholder, the one line that says so — plus one warning
//!   for each existing configured root whose group or other permission bits are set.
//! * **A signal seals nothing, and it does not merely drop either.** SIGTERM and SIGINT
//!   end the process through [`Daemon::stop`] — the listener down, every in-flight seal
//!   awaited, every redb file lock released — but they do not end the entry. An entry has
//!   exactly two endings — the participant's early finish through the control API, and the
//!   automatic seal at `deadline_at` — and a signal is neither. A daemon that sealed on
//!   SIGTERM would turn "the laptop rebooted" into "the run is over", which is precisely the
//!   failure D9 exists to prevent: a closed laptop is the normal path, and the entry is
//!   resumed when the daemon comes back.
//!
//! **The current poll finishes.** [`Daemon::tick`] does its filesystem work inside
//! `with_entry`, which is a synchronous closure around [`goalcup_daemon::runtime::
//! EntryRuntime::poll_once_at`]: a future cannot be cancelled inside code that never
//! awaits. So a signal arriving mid-tick can only take effect at one of the loop's real
//! await points — the receipt request in `bind_start`, or an upload in `pump_all` — and
//! both are idempotent on the far side (`ReceiptRequest` is refused twice, a chunk is
//! answered with the acknowledgement it already earned). The chain and the write-ahead
//! buffer are durable per envelope, so nothing in flight is evidence that was lost.

use clap::Parser;
use goalcup_daemon::config::DaemonConfig;
use goalcup_daemon::runtime::Daemon;
use std::path::PathBuf;
use std::process::ExitCode;

// clap takes a doc comment's first paragraph as `about` (which `#[command]` below overrides)
// and the rest as `long_about`, which is what `--help` opens with. So what is written here
// has to describe the program.
/// The capture daemon.
///
/// Records one entry per tracked run: it prepares the workspace, tails what the harness
/// writes, redacts it, chains and signs it, uploads it, serves the loopback control API, and seals at the deadline or when asked. It runs in the foreground
/// under whatever started it and configures itself from one file.
///
/// README.md's "Running the daemon" carries an example of that file, the exit codes, and
/// what `--print-addresses` answers.
#[derive(Parser, Debug)]
#[command(
    name = "goalcup-daemon",
    version = goalcup_daemon::DAEMON_VERSION_TEXT,
    about = "The /goal Cup capture daemon. Runs in the foreground; service registration is M4."
)]
struct Args {
    /// The per-machine configuration file (`DaemonConfig`).
    #[arg(long, value_name = "FILE")]
    config: PathBuf,

    /// Run in the foreground. The only mode this build has; accepted so a caller that
    /// spells it out is not refused, and named so a later `--background` is a change to
    /// this flag's documentation rather than a silent reversal of the default.
    #[arg(long)]
    foreground: bool,

    /// Print the running daemon's control address and control-file path, and exit —
    /// without starting anything.
    ///
    /// The two facts `goalcup doctor` needs: where a client connects, and the file that
    /// told this process so. Printed in that order, one per line, and nothing else on
    /// `stdout` — the second line is a path and not an address on purpose, because the question a doctor is answering is "is anything there", and the
    /// file is where the answer lives.
    ///
    /// It READS `control.json` and opens no registry. `Registry::open` takes a file lock,
    /// the running daemon holds it, and this flag is worth running precisely when a daemon
    /// IS up: starting a second one to ask where the first is was refused every time it
    /// mattered. With no daemon there is no file, and that is the answer — a sentence on
    /// stderr naming the path, and a non-zero exit.
    #[arg(long)]
    print_addresses: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();
    // `enable_all` because the loop wants both the timer and the IO drivers: four
    // `tokio::time::interval`s and the loopback control listener.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => return refuse(&format!("the async runtime would not start: {e}")),
    };
    runtime.block_on(run(args))
}

async fn run(args: Args) -> ExitCode {
    // The path is named in every refusal below. A daemon that says "is not loopback" and
    // not which file said so is a daemon nobody can configure.
    let named = args.config.display().to_string();
    let config = match DaemonConfig::load(&args.config) {
        Ok(config) => config,
        Err(e) => return refuse(&format!("{named}: {e}")),
    };
    // Validated here as well as inside `Daemon::start`, so a bad configuration is one
    // message about a file rather than a failure part-way through opening a registry.
    if let Err(e) = config.validate() {
        return refuse(&format!("{named}: {e}"));
    }
    // Named through the constant `control::write_control_file` writes with, so the third
    // startup line and the file the daemon actually writes cannot spell it differently.
    let control_file = config
        .config_dir
        .join(goalcup_daemon::control::CONTROL_FILE_NAME);

    // Before anything is opened. This flag describes the daemon that is running and must
    // never become one: see the field's own documentation.
    if args.print_addresses {
        return print_addresses(&config.config_dir, &control_file);
    }

    // One line, after the validation and the `--print-addresses` early return: a doctor asking
    // where a running daemon is must not be told off about a key it did not start with.
    if let Some(notice) = config.placeholder_key_notice() {
        eprintln!("goalcup-daemon: {notice}");
    }

    let daemon = match Daemon::start(config).await {
        Ok(daemon) => daemon,
        Err(e) => return refuse(&format!("{named}: {e}")),
    };

    // **Installed before the addresses are printed**, and that ordering is the whole of it.
    // `tokio::signal` registers its handler when the stream is constructed, not when it is
    // first polled, and the two lines below are what a supervisor waits for before it
    // considers the daemon up. Constructing the streams inside the `select!` would leave a
    // window between "the daemon says it is listening" and "the daemon can be asked to
    // stop", and a SIGTERM inside it would meet the default disposition: the process would
    // die on the signal instead of exiting, which a service manager reads as a crash.
    let mut signals = match Signals::install() {
        Ok(signals) => signals,
        Err(e) => return refuse(&format!("the signal handlers would not install: {e}")),
    };

    println!("{}", daemon.control_addr());
    println!("{}", control_file.display());

    // The loop, until a signal. `run_forever` never returns `Ok`, so the only two ways out
    // are its own `Err` — which nothing inside it produces, because each cadence
    // counts its failures rather than propagating them — and the shutdown below.
    let outcome = tokio::select! {
        ended = daemon.run_forever() => ended,
        () = signals.stopped() => Ok(()),
    };
    // **`stop`, not a drop.** Dropping a `Daemon` detaches the listener task rather than
    // ending it, which would leave a control port answering with a live bearer while
    // `control.json` on disk named it. It would also leave the registry's and every
    // buffer's redb file lock held by the control router's own `Arc`, which makes a restart
    // in place fail on a database this process had already stopped using. And a seal on a
    // blocking worker is awaited here rather than abandoned mid-`terminate`.
    //
    // After the signal, on both arms: `run_forever` returning at all is this process
    // ending too, and the listener is no more use to it then than after a SIGTERM.
    daemon.stop().await;
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => refuse(&e.to_string()),
    }
}

/// The two facts, off `control.json`, in the order a running daemon prints them.
///
/// Nothing here opens the registry, binds a port or writes a file: the addresses are the
/// ones the daemon that wrote this file bound, which is the only honest answer a second
/// process can give. The bearer in the file is read and not printed — the address and the
/// path are the whole of what a doctor asked for.
fn print_addresses(config_dir: &std::path::Path, control_file: &std::path::Path) -> ExitCode {
    let file = match goalcup_daemon::control::read_control_file(config_dir) {
        Ok(file) => file,
        // Named rather than described: "no such file" about a path the reader can go and
        // look at is the whole diagnosis when nothing is listening.
        Err(e) => return refuse(&format!("{}: {e}", control_file.display())),
    };
    println!("{}", file.address);
    println!("{}", control_file.display());
    ExitCode::SUCCESS
}

/// The two signals that stop this process, registered up front.
///
/// SIGTERM is what a service manager sends and SIGINT is what a terminal sends, and this
/// build answers both the same way: stop the process, seal nothing. They are held in a
/// struct rather than created where they are awaited so that registration happens before
/// the daemon reports itself up — see the comment at the call site.
#[cfg(unix)]
struct Signals {
    term: tokio::signal::unix::Signal,
    interrupt: tokio::signal::unix::Signal,
}

#[cfg(unix)]
impl Signals {
    fn install() -> std::io::Result<Signals> {
        use tokio::signal::unix::{SignalKind, signal};
        Ok(Signals {
            term: signal(SignalKind::terminate())?,
            interrupt: signal(SignalKind::interrupt())?,
        })
    }

    /// Whichever arrives first. `recv` answers `None` only when the registration is torn
    /// down, which nothing here does, so a `None` is treated as the signal it stands in for
    /// rather than as a branch that completes forever in a `select!`.
    async fn stopped(&mut self) {
        tokio::select! {
            _ = self.term.recv() => {}
            _ = self.interrupt.recv() => {}
        }
    }
}

/// On a platform with no `SIGTERM` — none this build targets — Ctrl-C is the whole of it,
/// so the binary still stops rather than refusing to compile.
#[cfg(not(unix))]
struct Signals;

#[cfg(not(unix))]
impl Signals {
    fn install() -> std::io::Result<Signals> {
        Ok(Signals)
    }

    async fn stopped(&mut self) {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// One sentence on stderr and exit 1. Never a panic: a participant meeting a bad
/// configuration should read what is wrong with it, not a backtrace.
fn refuse(message: &str) -> ExitCode {
    eprintln!("goalcup-daemon: {message}");
    ExitCode::FAILURE
}
