//! The daemon a command needs, started on demand: the run-scoped lifetime.
//!
//! There is no service. `goalcup start` and `goalcup seal` call [`ensure_daemon`]: a daemon
//! that answers on `control.json` is used as it is, and otherwise this module starts
//! `goalcup-daemon --config <cfg>/daemon.toml --exit-when-idle` detached and waits for it to
//! answer. The daemon ends itself once no capture is unfinished (its `IDLE_GRACE`), so a
//! finished run leaves nothing running.
//!
//! # What the started daemon inherits
//!
//! - **No environment of this process.** It is built from nothing: the variables in
//!   [`INHERITED_ENV`] and a fixed `PATH`. A shell that exported `DEEPSEEK_API_KEY`,
//!   `AMP_API_KEY` or a `*_PROXY` hands none of it to the recorder, which holds every
//!   run's chain and has no use for a provider key.
//! - **No session.** `setsid` in the child before `exec`: closing the terminal (`SIGHUP`) or
//!   typing Ctrl-C (`SIGINT` to the foreground process group) reaches the harness and the
//!   launcher, never the chain writer. A launcher killed mid-run leaves a daemon that can
//!   still end the registered harness and seal.
//! - **No terminal.** stdin is `/dev/null`; stdout and stderr append to `<cfg>/daemon.log`
//!   (mode 0600). The daemon writes its two startup lines there and, if it refuses its
//!   configuration, the one sentence that says why. It never logs a request.
//! - **`/` as its working directory**, so it holds no participant directory open.
//!
//! # One daemon
//!
//! The registry's redb file lock is what makes one daemon per machine a fact (D4), and this
//! module adds no second mechanism. Two commands that race both start a daemon; the loser
//! refuses at `Registry::open` and exits 1, and both commands then find the winner through
//! `control.json`. A daemon that is stopping holds the lock until it has stopped, so a start
//! that lost to it is tried again while the wait lasts.

use crate::CliError;
use crate::config::{ControlFile, read_control_file};
use crate::daemon::HttpDaemonClient;
use goalcup_protocol::control::{CONTROL_API_VERSION, ControlError, DaemonClient};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The daemon configuration `goalcup` starts a daemon from: `<cfg>/daemon.toml`.
pub const DAEMON_CONFIG_NAME: &str = "daemon.toml";

/// Where a started daemon's stdout and stderr go: `<cfg>/daemon.log`.
pub const DAEMON_LOG_NAME: &str = "daemon.log";

/// The daemon's binary name, looked for beside this executable and then on `PATH`.
pub const DAEMON_BINARY_NAME: &str = "goalcup-daemon";

/// The only variables of this process a started daemon receives. `HOME` because the daemon
/// refuses a harness `config_home` under the participant's real home and must know where
/// that is; the other three are identity and scratch space, and none of them is a secret.
pub const INHERITED_ENV: [&str; 4] = ["HOME", "USER", "LOGNAME", "TMPDIR"];

/// The started daemon's `PATH`: the system directories, never this shell's. It runs `/bin/ps`
/// and nothing else by name.
pub const DAEMON_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// How long a command waits for a daemon it started to answer. A daemon opens its registry
/// and reopens what an earlier one left unfinished before it writes `control.json`; on this
/// tree's hardware that is well under a second.
pub const SPAWN_WAIT: Duration = Duration::from_secs(10);

/// How many times one wait starts a daemon. More than one because a daemon that is idling out
/// holds the registry lock until it has stopped, and a start that met it refuses.
const SPAWN_ATTEMPTS: u32 = 3;

/// How often the wait looks for the started daemon's answer.
const POLL: Duration = Duration::from_millis(50);

/// What [`ensure_daemon`] may do when `<cfg>/daemon.toml` is missing.
#[derive(Debug, Clone, Copy)]
pub enum MissingConfig<'a> {
    /// Write the default local-only configuration (`goalcup start`, the first use). The
    /// connectors directory is the one this command resolved, so the daemon reads the same
    /// rulebook the CLI pinned.
    WriteDefault { connectors: &'a Path },
    /// Start nothing and refuse "no daemon" (`goalcup seal` and the finish's restart path):
    /// with no configuration no daemon was ever started from this directory, so there is
    /// no unsealed run here to recover.
    Refuse,
}

/// A client for a daemon that answers, starting one if none does, before `deadline`.
///
/// A daemon speaking another control API version that still answers is refused with the
/// sentence [`HttpDaemonClient::from_config_dir`] gives: starting a second one beside it would
/// only lose the registry lock. A `control.json` nothing answers on is a daemon that is gone.
pub fn ensure_daemon(
    cfg: &Path,
    missing: MissingConfig<'_>,
    deadline: Instant,
) -> Result<HttpDaemonClient, CliError> {
    if let Some(client) = live_daemon(cfg)? {
        return Ok(client);
    }
    let config = cfg.join(DAEMON_CONFIG_NAME);
    if !config.is_file() {
        match missing {
            MissingConfig::WriteDefault { connectors } => {
                write_default_config(cfg, connectors)?;
            }
            // The refusal a machine with no daemon has always had: it names `control.json`.
            MissingConfig::Refuse => return HttpDaemonClient::from_config_dir(cfg),
        }
    }
    let binary = daemon_binary()?;
    let log = cfg.join(DAEMON_LOG_NAME);
    let mut child = spawn(&binary, &config, &log)?;
    let mut attempts = 1;
    loop {
        if let Some(client) = live_daemon(cfg)? {
            // The daemon is in a session of its own and outlives this process; dropping the
            // handle neither waits for it nor ends it.
            drop(child);
            return Ok(client);
        }
        if let Ok(Some(_exited)) = child.try_wait() {
            if attempts >= SPAWN_ATTEMPTS {
                return Err(not_started(&binary, &config, &log, "it exited"));
            }
            attempts += 1;
            child = spawn(&binary, &config, &log)?;
        }
        if Instant::now() >= deadline {
            return Err(not_started(
                &binary,
                &config,
                &log,
                "it did not answer in time",
            ));
        }
        std::thread::sleep(POLL);
    }
}

/// A client for the daemon `control.json` names, if that daemon answers.
fn live_daemon(cfg: &Path) -> Result<Option<HttpDaemonClient>, CliError> {
    // Missing, unreadable or unparseable: there is no daemon to talk to. A daemon that is up
    // rewrites the file whole and atomically.
    let Ok(file) = read_control_file(cfg) else {
        return Ok(None);
    };
    let ControlFile {
        control_api_version,
        port,
        token,
        ..
    } = file;
    let client = HttpDaemonClient::new(format!("http://127.0.0.1:{port}"), token)?;
    match crate::retry::retry_busy(|| client.health()) {
        Ok(_) if control_api_version == CONTROL_API_VERSION => Ok(Some(client)),
        // Nobody listens there: the daemon that wrote the file is gone.
        Err(ControlError::Unreachable(_)) => Ok(None),
        // Something answered for a daemon of another version: say so rather than start a
        // second daemon that would only lose the registry lock to it.
        _ if control_api_version != CONTROL_API_VERSION => {
            HttpDaemonClient::from_config_dir(cfg).map(Some)
        }
        // The right version, and an answer that is not this daemon's: a port the system has
        // given to something else since. The file is stale.
        _ => Ok(None),
    }
}

/// `goalcup-daemon --config <config> --exit-when-idle`, detached as the module note says.
fn spawn(binary: &Path, config: &Path, log: &Path) -> Result<Child, CliError> {
    let output = open_log(log)?;
    let errors = output.try_clone().map_err(|source| CliError::Io {
        path: log.display().to_string(),
        source,
    })?;
    let mut command = Command::new(binary);
    command
        .arg("--config")
        .arg(config)
        .arg("--exit-when-idle")
        .env_clear()
        .env("PATH", DAEMON_PATH)
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(output)
        .stderr(errors);
    for name in INHERITED_ENV {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: the closure runs in the forked child before `exec` and calls one
        // async-signal-safe function, `setsid`, which cannot fail in a child that is not
        // already a process-group leader (a fresh fork never is).
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    command.spawn().map_err(|source| CliError::Io {
        path: binary.display().to_string(),
        source,
    })
}

/// `<cfg>/daemon.log` for appending, created at mode 0600 in a 0700 directory.
fn open_log(log: &Path) -> Result<std::fs::File, CliError> {
    let io = |source| CliError::Io {
        path: log.display().to_string(),
        source,
    };
    if let Some(parent) = log.parent() {
        crate::config::create_private_dir(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(log).map_err(io)
}

/// The daemon binary: beside this executable (an installed `bin/`, or `target/<profile>/`),
/// then on `PATH`.
pub fn daemon_binary() -> Result<PathBuf, CliError> {
    let exe = std::env::current_exe().map_err(|source| CliError::Io {
        path: "the running executable".to_string(),
        source,
    })?;
    let mut beside = vec![exe.with_file_name(DAEMON_BINARY_NAME)];
    if let Ok(real) = std::fs::canonicalize(&exe) {
        beside.push(real.with_file_name(DAEMON_BINARY_NAME));
    }
    let on_path = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .filter(|dir| dir.is_absolute())
                .map(|dir| dir.join(DAEMON_BINARY_NAME))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    beside
        .into_iter()
        .chain(on_path)
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            CliError::Refused(format!(
                "no daemon is running and {DAEMON_BINARY_NAME} is neither beside {} nor on \
                 PATH, so none can be started. Put the three binaries (goalcup, \
                 goalcup-daemon, goalcup-verify) in one directory.",
                exe.display()
            ))
        })
}

/// The default local-only configuration, written once and never overwritten. The evidence and
/// the runs live **beside** `cfg`, never inside it (`<cfg>-artifacts`, `<cfg>-runs`): the CLI
/// reads `cfg`, and a capture buffer is not its to see. The control API takes a port the system
/// chooses, and there is no upload service. A participant who wants other roots edits the file.
fn write_default_config(cfg: &Path, connectors: &Path) -> Result<(), CliError> {
    use std::io::Write;

    let cfg = absolute(cfg)?;
    let connectors = absolute(connectors)?;
    let beside = |suffix: &str| -> Result<PathBuf, CliError> {
        match (cfg.parent(), cfg.file_name()) {
            (Some(parent), Some(name)) => {
                let mut leaf = name.to_os_string();
                leaf.push(suffix);
                Ok(parent.join(leaf))
            }
            _ => Err(CliError::Refused(format!(
                "{} has no parent directory to keep runs beside. Set $GOALCUP_CONFIG_DIR to a \
                 directory of its own.",
                cfg.display()
            ))),
        }
    };
    let text = format!(
        "# Written by `goalcup start` the first time it needed a daemon. `goalcup` starts\n\
         # goalcup-daemon from this file for each run; edit it to move where runs are kept.\n\
         artifact_root    = {}\n\
         connectors_dir   = {}\n\
         config_dir       = {}\n\
         workspace_parent = {}\n\
         control_addr     = \"127.0.0.1:0\"\n",
        toml_string(&beside("-artifacts")?)?,
        toml_string(&connectors)?,
        toml_string(&cfg)?,
        toml_string(&beside("-runs")?)?,
    );
    crate::config::create_private_dir(&cfg)?;
    let path = cfg.join(DAEMON_CONFIG_NAME);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let io = |source| CliError::Io {
        path: path.display().to_string(),
        source,
    };
    match options.open(&path) {
        Ok(mut file) => {
            file.write_all(text.as_bytes()).map_err(io)?;
            file.sync_all().map_err(io)
        }
        // A command racing this one wrote it first: the same file.
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(source) => Err(io(source)),
    }
}

/// `path` made absolute against the working directory, as the daemon requires of every root.
fn absolute(path: &Path) -> Result<PathBuf, CliError> {
    std::path::absolute(path).map_err(|source| CliError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// `path` as a TOML basic string. A JSON string literal is one: TOML accepts every escape
/// `serde_json` writes (`\"`, `\\`, `\n`, `\u00XX`, ...). A path that is not UTF-8 cannot be
/// written into the file at all.
fn toml_string(path: &Path) -> Result<String, CliError> {
    let text = path.to_str().ok_or_else(|| {
        CliError::Refused(format!(
            "{} is not valid UTF-8, so it cannot be written into {DAEMON_CONFIG_NAME}. Set \
             $GOALCUP_CONFIG_DIR to a UTF-8 path.",
            path.display()
        ))
    })?;
    Ok(serde_json::to_string(text)?)
}

/// The refusal for a daemon that was started and never answered, naming where its own words
/// are.
fn not_started(binary: &Path, config: &Path, log: &Path, why: &str) -> CliError {
    CliError::Refused(format!(
        "no daemon is running, and the one this command started ({} --config {}) did not \
         come up: {why}. Its output is in {}; check that the configuration's config_dir is \
         the directory holding it.",
        binary.display(),
        config.display(),
        log.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_with_quotes_and_backslashes_is_one_toml_basic_string() {
        let quoted = toml_string(Path::new("/tmp/a \"b\" \\c")).map_err(|e| e.to_string());
        assert_eq!(quoted, Ok(r#""/tmp/a \"b\" \\c""#.to_string()));
    }
}
