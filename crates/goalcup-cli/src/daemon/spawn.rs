//! The daemon a command needs, started on demand: the run-scoped lifetime.
//!
//! There is no service. Every command that asks the daemon anything (`goalcup start`, `seal`,
//! `status` and `entries`) calls [`ensure_daemon`]: a daemon
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
//! - **No other descriptor.** Every descriptor from 3 up that would survive `exec` is closed in
//!   the child first, so nothing this process inherited is held by a daemon for a week.
//! - **`/` as its working directory**, so it holds no participant directory open.
//!
//! # One daemon
//!
//! The registry's redb file lock is what makes one daemon per machine a fact (D4), and this
//! module adds no second mechanism. Two commands that race both start a daemon; the loser
//! refuses at `Registry::open` and exits 1, and both commands then find the winner through
//! `control.json`. A daemon that is stopping holds the lock until it has stopped, so a start
//! that lost to it is tried again, with a growing pause, until the wait's deadline. A daemon
//! that exits for any other reason before it answers (a configuration it refuses) ends the wait at
//! once, naming its exit status and `daemon.log`.
//!
//! # The configuration names this command's connectors
//!
//! `goalcup start` writes `<cfg>/daemon.toml` the first time, naming the connectors directory it
//! resolved then. Binaries move, and a second checkout resolves another directory, so every
//! `start` compares: the tracker's own unedited default is rewritten to name today's directory,
//! and a file someone edited that names another one is refused, naming the file, before any
//! daemon could refuse the run with a sentence about connector bytes.

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
pub(crate) const DAEMON_BINARY_NAME: &str = "goalcup-daemon";

/// The only variables of this process a started daemon receives. `HOME` because the daemon
/// refuses a harness `config_home` under the participant's real home and must know where
/// that is; the other three are identity and scratch space, and none of them is a secret.
pub(crate) const INHERITED_ENV: [&str; 4] = ["HOME", "USER", "LOGNAME", "TMPDIR"];

/// The started daemon's `PATH`: the system directories, never this shell's. It runs `/bin/ps`
/// and nothing else by name.
pub const DAEMON_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// How long a command waits for a daemon it started to answer. A daemon opens its registry
/// and reopens what an earlier one left unfinished before it writes `control.json`; on this
/// tree's hardware that is well under a second.
pub const SPAWN_WAIT: Duration = Duration::from_secs(10);

/// The pause before starting a daemon again after the last one exited early,
/// doubling up to [`RESPAWN_PAUSE_MAX`]. A daemon that is idling out holds the lock until it
/// has stopped, and a start that met it refuses.
const RESPAWN_PAUSE: Duration = Duration::from_millis(100);
const RESPAWN_PAUSE_MAX: Duration = Duration::from_secs(1);

/// The first line of the default `daemon.toml`, which marks it as the tracker's own.
const DEFAULT_CONFIG_HEADER: &str =
    "# Written by `goalcup start` the first time it needed a daemon.";

/// The most of `daemon.toml` this module reads: a configuration is a few hundred bytes.
const CONFIG_MAX_BYTES: u64 = 64 * 1024;

/// How often the wait looks for the started daemon's answer.
const POLL: Duration = Duration::from_millis(50);

/// What [`ensure_daemon`] may do when `<cfg>/daemon.toml` is missing.
#[derive(Debug, Clone, Copy)]
pub enum MissingConfig<'a> {
    /// Write the default local-only configuration (`goalcup start`, the first use). The
    /// connectors directory is the one this command resolved, so the daemon reads the same
    /// rulebook the CLI pinned.
    WriteDefault { connectors: &'a Path },
    /// Write the default configuration only when there is none (`goalcup status` and
    /// `goalcup entries`), naming [`crate::config::connectors_dir`]. A daemon that answers is
    /// used whatever connectors it reads, and an existing file is used as it is: reading the
    /// registry needs no connector, so there is nothing to keep in step and nothing to refuse.
    WriteDefaultToRead,
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
///
/// With [`MissingConfig::Refuse`] and no `daemon.toml`, the answer is whatever
/// `from_config_dir` builds, unprobed: the refusal "no daemon" when there is no `control.json`,
/// and a client whose first call reports the daemon unreachable when the file is stale.
pub fn ensure_daemon(
    cfg: &Path,
    missing: MissingConfig<'_>,
    deadline: Instant,
) -> Result<HttpDaemonClient, CliError> {
    let config = cfg.join(DAEMON_CONFIG_NAME);
    let live = live_daemon(cfg)?;
    match missing {
        MissingConfig::WriteDefault { connectors } => {
            // A daemon someone started by hand (with its own configuration) is used as it is:
            // with one answering, a missing file is not written.
            let rewritten = prepare_config(cfg, &config, connectors, live.is_none())?;
            if let Some(client) = live {
                if rewritten {
                    return Err(CliError::Refused(format!(
                        "a daemon started from the earlier {} is still running, reading another \
                         connectors directory than this goalcup's {}. The file now names this \
                         one; the running daemon exits 30 seconds after its last run is sealed. \
                         Start again then.",
                        config.display(),
                        connectors.display()
                    )));
                }
                return Ok(client);
            }
        }
        MissingConfig::WriteDefaultToRead => {
            if let Some(client) = live {
                return Ok(client);
            }
            if read_config(&config)?.is_none() {
                let connectors = crate::config::connectors_dir()?;
                create_config(&config, &default_config(cfg, &connectors)?)?;
            }
        }
        MissingConfig::Refuse => {
            if let Some(client) = live {
                return Ok(client);
            }
            if !config.is_file() {
                // The refusal a machine with no daemon has always had: it names `control.json`.
                return HttpDaemonClient::from_config_dir(cfg);
            }
        }
    }
    let binary = daemon_binary()?;
    let log = cfg.join(DAEMON_LOG_NAME);
    let (mut child, mut said_from) = spawn(&binary, &config, &log)?;
    let mut pause = RESPAWN_PAUSE;
    let mut respawn_at: Option<Instant> = None;
    loop {
        if let Some(client) = live_daemon(cfg)? {
            // The daemon is in a session of its own and outlives this process. A thread waits
            // on it so that one which exits before this command does is reaped, not left a
            // zombie for the length of a run; it neither ends the daemon nor holds this process.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return Ok(client);
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(not_started(
                &binary,
                &config,
                &log,
                "it did not answer in time",
            ));
        }
        match respawn_at {
            Some(at) if now >= at => {
                respawn_at = None;
                (child, said_from) = spawn(&binary, &config, &log)?;
            }
            Some(_) => {}
            None => {
                if let Ok(Some(status)) = child.try_wait() {
                    // Lost the registry lock to a daemon that is starting or stopping: start
                    // again after a pause, until the deadline. Any other early exit is a
                    // daemon that will refuse every time (its configuration), so the wait
                    // ends now. A start race over a fresh artifact root also ends at the
                    // lock: the registry file is opened race-free (`goalcup-platform`).
                    if !lost_the_registry_lock(&log, said_from) {
                        return Err(not_started(
                            &binary,
                            &config,
                            &log,
                            &format!("it exited ({status}) before answering"),
                        ));
                    }
                    respawn_at = Some(now + pause);
                    pause = (pause * 2).min(RESPAWN_PAUSE_MAX);
                }
            }
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

/// What `goalcup-daemon` says when another daemon holds the registry's file lock (its
/// `Registry::from_file`, D4). The one early exit that is waited out to the deadline: the holder
/// is a daemon starting (a start race) or stopping (idling out), and the lock comes free.
/// `the_registry_lock_held_by_another_process_is_waited_out` in
/// `tests/full/run_scoped_daemon.rs` holds the two crates' wording together.
const LOST_REGISTRY_LOCK: &str = "another daemon holds the registry";

/// Whether the daemon that just exited said it lost the registry lock: in what `log` gained
/// from `from` on, which is the exited daemon's output (and any racing command's daemon's,
/// started from the same configuration).
fn lost_the_registry_lock(log: &Path, from: u64) -> bool {
    use std::io::{Read, Seek};
    let Ok(mut file) = std::fs::File::open(log) else {
        return false;
    };
    let mut said = Vec::new();
    if file.seek(std::io::SeekFrom::Start(from)).is_err()
        || file.take(CONFIG_MAX_BYTES).read_to_end(&mut said).is_err()
    {
        return false;
    }
    String::from_utf8_lossy(&said).contains(LOST_REGISTRY_LOCK)
}

/// `goalcup-daemon --config <config> --exit-when-idle`, detached as the module note says, and
/// the length of `log` before it, from which on the log is this daemon's output.
fn spawn(binary: &Path, config: &Path, log: &Path) -> Result<(Child, u64), CliError> {
    let output = open_log(log)?;
    let errors = output.try_clone().map_err(|source| CliError::Io {
        path: log.display().to_string(),
        source,
    })?;
    let said_from = output.metadata().map(|m| m.len()).unwrap_or(0);
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
        let highest = descriptor_ceiling();
        // SAFETY: the closure runs in the forked child before `exec` and calls only
        // async-signal-safe functions. `setsid` cannot fail in a child that is not already a
        // process-group leader (a fresh fork never is). Every descriptor from 3 up that would
        // survive `exec` is closed; one marked close-on-exec is left, because std's own error
        // pipe is one and it reports a failed `exec` back to `spawn`. stdio is already in place
        // on 0 to 2 when this runs.
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                for fd in 3..highest {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags >= 0 && flags & libc::FD_CLOEXEC == 0 {
                        libc::close(fd);
                    }
                }
                Ok(())
            });
        }
    }
    let child = command.spawn().map_err(|source| CliError::Io {
        path: binary.display().to_string(),
        source,
    })?;
    Ok((child, said_from))
}

/// One past the highest descriptor the child sweeps: this process's open-file limit, kept between
/// 256 and 65,536 so the sweep is bounded however the limit was raised.
#[cfg(unix)]
fn descriptor_ceiling() -> libc::c_int {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` writes the one struct it is handed.
    let current = if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } == 0 {
        limit.rlim_cur
    } else {
        0
    };
    libc::c_int::try_from(current.clamp(256, 65_536)).unwrap_or(65_536)
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

/// Make `config` name the connectors this command loaded, and say whether it was rewritten.
///
/// - Missing: the default is written when `create` says so (no daemon answers), else left
///   missing.
/// - Already naming `connectors` (or naming nothing this can read): left alone.
/// - The tracker's own default, unedited, naming another directory: rewritten to name
///   `connectors`. Binaries move and checkouts differ; the file is this tracker's, not the
///   participant's.
/// - Edited by someone and naming another directory: refused, naming the file and both
///   directories, because the edit may have been deliberate.
fn prepare_config(
    cfg: &Path,
    config: &Path,
    connectors: &Path,
    create: bool,
) -> Result<bool, CliError> {
    let wanted = default_config(cfg, connectors)?;
    let text = match read_config(config)? {
        Some(text) => text,
        None => {
            if create {
                create_config(config, &wanted)?;
            }
            return Ok(false);
        }
    };
    if text == wanted {
        return Ok(false);
    }
    let Some(named) = connectors_dir_in(&text) else {
        return Ok(false);
    };
    if same_directory(&named, connectors) {
        return Ok(false);
    }
    if text == default_config(cfg, &named)? {
        crate::config::write_secret_file(config, wanted.as_bytes())?;
        return Ok(true);
    }
    Err(CliError::Refused(format!(
        "{} names connectors_dir {}, but this goalcup loads its connectors from {}, and the \
         daemon must read the same ones. Edit connectors_dir in {} to name {}, or run the \
         goalcup whose connectors are at {} (or set $GOALCUP_CONNECTORS_DIR to it).",
        config.display(),
        named.display(),
        connectors.display(),
        config.display(),
        connectors.display(),
        named.display()
    )))
}

/// The default local-only configuration's exact text. The evidence and the runs live
/// **beside** `cfg`, never inside it (`<cfg>-artifacts`, `<cfg>-runs`): the CLI reads `cfg`, and
/// a capture buffer is not its to see. The control API takes a port the system chooses, and
/// there is no upload service. A participant who wants other roots edits the file, which then
/// is theirs: [`prepare_config`] never rewrites an edited file.
fn default_config(cfg: &Path, connectors: &Path) -> Result<String, CliError> {
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
    Ok(format!(
        "{DEFAULT_CONFIG_HEADER}\n\
         # `goalcup` starts goalcup-daemon from this file for each run, and keeps connectors_dir\n\
         # naming the connectors it loads while the file is unedited. Edit it to move where runs\n\
         # are kept; an edited file is left as it is.\n\
         artifact_root    = {}\n\
         connectors_dir   = {}\n\
         config_dir       = {}\n\
         workspace_parent = {}\n\
         control_addr     = \"127.0.0.1:0\"\n",
        toml_string(&beside("-artifacts")?)?,
        toml_string(&connectors)?,
        toml_string(&cfg)?,
        toml_string(&beside("-runs")?)?,
    ))
}

/// `config`'s text, or `None` when there is no file. Bounded at [`CONFIG_MAX_BYTES`].
fn read_config(config: &Path) -> Result<Option<String>, CliError> {
    use std::io::Read;

    let io = |source| CliError::Io {
        path: config.display().to_string(),
        source,
    };
    let file = match std::fs::File::open(config) {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(io(source)),
    };
    let mut text = String::new();
    file.take(CONFIG_MAX_BYTES)
        .read_to_string(&mut text)
        .map_err(io)?;
    Ok(Some(text))
}

/// Write the default where there is none, whole or not at all: a temp file, then a link that
/// refuses to replace. A command racing this one that linked first wrote the same bytes.
fn create_config(config: &Path, text: &str) -> Result<(), CliError> {
    let io = |path: &Path| {
        let path = path.display().to_string();
        move |source| CliError::Io {
            path: path.clone(),
            source,
        }
    };
    let mut temp = config.as_os_str().to_os_string();
    temp.push(format!(".{}.new", std::process::id()));
    let temp = PathBuf::from(temp);
    crate::config::write_secret_file(&temp, text.as_bytes())?;
    let linked = std::fs::hard_link(&temp, config);
    let _ = std::fs::remove_file(&temp);
    match linked {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(source) => Err(io(config)(source)),
    }
}

/// The `connectors_dir` a configuration names, when it is written as one TOML string on its own
/// line (a basic `"..."` string, read with JSON's escapes, or a literal `'...'` one).
fn connectors_dir_in(text: &str) -> Option<PathBuf> {
    text.lines().find_map(|line| {
        let rest = line.trim_start().strip_prefix("connectors_dir")?;
        let value = rest.trim_start().strip_prefix('=')?.trim_start();
        if value.starts_with('"') {
            let mut values = serde_json::Deserializer::from_str(value).into_iter::<String>();
            values.next()?.ok().map(PathBuf::from)
        } else {
            let (literal, _) = value.strip_prefix('\'')?.split_once('\'')?;
            Some(PathBuf::from(literal))
        }
    })
}

/// Whether two paths name one directory: by their real paths when both exist, else as written.
fn same_directory(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => absolute(left).ok() == absolute(right).ok(),
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
