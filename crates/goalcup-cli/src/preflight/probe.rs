//! The half of [`super`] that starts a process: D12's version probe, the `PATH` walk it
//! resolves the harness with, and the probe files it leaves behind.
//!
//! Everything here is bounded and everything here cleans up after itself:
//! [`sample_harness_version_within`] kills the whole process group at its deadline and never
//! waits on a pipe past it, and [`sweep_stale_probes`] removes the files a run killed between
//! two statements left in the participant's configuration directory.

use crate::CliError;
use crate::connector::Connector;
use crate::version::extract_version;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long the harness gets to print its version before the probe gives up on it.
///
/// A bound and not a guess about how slow a machine can be: a harness that has not answered
/// in this long is not going to, and `goalcup doctor` hanging on a wedged binary is a worse
/// answer than `goalcup doctor` reporting one.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// How often the probe asks whether the child has finished. Small enough that the common
/// case — a `--version` that answers in milliseconds — is not paced by this number.
const PROBE_POLL: Duration = Duration::from_millis(20);

/// The name [`writable`] gives the file it creates to ask its question, and the name
/// [`sweep_stale_probes`] looks for to clean one up.
pub(super) const PROBE_FILE_PREFIX: &str = ".goalcup-preflight-";

/// Remove every `.goalcup-preflight-<pid>` left in `dir`.
///
/// `writable` creates one and removes it in the next statement, so the only ones that
/// survive are from a process killed in between — and a participant who then finds files
/// with this tracker's name on them in their configuration directory is owed their removal
/// rather than an explanation. `goalcup doctor` sweeps before it checks anything.
///
/// Best effort throughout: a directory that cannot be read, and a file that cannot be
/// removed, are exactly the conditions [`super::required_paths_gate`] is about to report on, and
/// this is not the place that reports them. A concurrent `goalcup` losing its own probe file
/// to this sweep loses nothing: its `File::create` has already answered the question, and its
/// own removal ignores the error.
pub fn sweep_stale_probes(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(PROBE_FILE_PREFIX) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// `binary` resolved against the `PATH` in `env`, by walking it **in this process**.
///
/// Not by shelling out to `which`, which would answer out of a login shell's environment —
/// a different `PATH`, possibly a different harness — and so would disagree with the launch
/// this check is predicting.
///
/// The split is [`std::env::split_paths`] and not `split(':')`: it is the platform's own rule
/// for the variable this build is reading, and a hand-rolled separator is a second answer to
/// a question the standard library already answers. A path component that is empty is skipped
/// rather than read as the working directory, which is the participant's repository during a
/// run.
///
/// A `binary` that already contains a separator is taken as the path it is, so a manifest
/// naming an absolute program is not looked up under it.
pub fn resolve_on_path(binary: &str, env: &BTreeMap<String, String>) -> Option<PathBuf> {
    if binary.contains(std::path::MAIN_SEPARATOR) {
        let path = PathBuf::from(binary);
        return is_executable_file(&path).then_some(path);
    }
    let path = env.get("PATH")?;
    std::env::split_paths(path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(binary))
        .find(|candidate| is_executable_file(candidate))
}

/// A regular file this process could execute. On a platform without Unix modes, existing as
/// a file is the whole of the question this build can ask.
fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// D12: run `detection.version_command` and return the version it printed.
///
/// A self-updating harness is caught here, at the start, rather than discovered in the
/// verdict — which is the whole of why the version is sampled by running the harness rather
/// than read off a lockfile.
///
/// Bounded by [`PROBE_TIMEOUT`] and run in [`probe_env`], which is neither this process's
/// environment nor an empty one.
pub fn sample_harness_version(
    c: &Connector,
    env: &BTreeMap<String, String>,
) -> Result<String, CliError> {
    sample_harness_version_within(c, env, PROBE_TIMEOUT)
}

/// [`sample_harness_version`] with the bound named, so a test can assert the bound is real
/// without waiting [`PROBE_TIMEOUT`] for it.
///
/// # The bound covers the pipes, not just the child
///
/// Two things can outlast `timeout` and only one of them is the harness. The child is polled
/// and killed at the deadline; its **output** is read on threads that are never waited on
/// past that same deadline, because a harness that backgrounds a helper leaves that helper
/// holding this probe's stdout, and a read to EOF on a pipe like that returns when the helper
/// does — which is not a time this build gets to choose. Either way the answer is a refusal
/// naming `timeout`, and either way it arrives inside it.
///
/// The helper is killed too. `process_group(0)` (see the spawn below) puts the harness in a
/// group of its own — the child's own pid doubles as the pgid — so at the deadline `killpg`
/// reaches everything in it, not only the direct child `Child::kill` would reach alone. The
/// group is still the child's own and nothing outside it is ever signalled: a `goalcup
/// doctor` interrupted at the terminal signals its own group, never the probe's.
pub fn sample_harness_version_within(
    c: &Connector,
    env: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<String, CliError> {
    let m = &c.manifest;
    let binary = &m.detection.binary;
    let mut words = m.detection.version_command.split_whitespace();
    let Some(first) = words.next() else {
        return Err(CliError::Internal(format!(
            "`{}`'s `detection.version_command` is empty",
            m.id
        )));
    };
    // The program is the one `detection.binary` resolves to, never the word the command
    // happens to start with: those two disagreeing is a manifest defect, and running the
    // second would probe a binary the launch will not use.
    if first != binary {
        return Err(CliError::Internal(format!(
            "`{}`'s `detection.version_command` starts with `{first}` and its \
             `detection.binary` is `{binary}`: the probe would measure a different program \
             from the one the launch runs.",
            m.id
        )));
    }
    let program = resolve_on_path(binary, env).ok_or_else(|| {
        CliError::Refused(format!(
            "`{binary}` is not on PATH, so `{}` cannot be measured on this machine.",
            m.detection.version_command
        ))
    })?;

    let mut command = std::process::Command::new(&program);
    command
        .args(words)
        .env_clear()
        .envs(probe_env(c, env))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // The probe runs the harness in a process group of its own, so whatever the harness
    // starts is that group's and never this CLI's: a `goalcup doctor` interrupted at the
    // terminal signals its own group, and a helper the probe caused to exist has no business
    // being in it.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|source| CliError::Io {
        path: program.display().to_string(),
        source,
    })?;

    // Both pipes are read on their own thread, and neither read is ever waited on past the
    // deadline. `Child::wait_with_output` reads them to EOF instead, and EOF does not arrive
    // while a grandchild the harness backgrounded still holds the write end — so a probe
    // built on it would hang indefinitely on a harness that had already exited, which is the one
    // thing `timeout` exists to make impossible.
    let (tx, rx) = std::sync::mpsc::channel::<(Stream, Vec<u8>)>();
    let mut pipes = 0usize;
    if let Some(pipe) = child.stdout.take() {
        pipes += 1;
        drain(Stream::Stdout, pipe, tx.clone());
    }
    if let Some(pipe) = child.stderr.take() {
        pipes += 1;
        drain(Stream::Stderr, pipe, tx.clone());
    }
    drop(tx);

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(source) => {
                let _ = child.kill();
                return Err(CliError::Io {
                    path: program.display().to_string(),
                    source,
                });
            }
        }
        if Instant::now() >= deadline {
            // `process_group(0)` below put this child in a group of its own, with the
            // child's own pid doubling as the pgid, so the group is what gets signalled: a
            // helper the harness backgrounded (`sleep 30 &`) is in that group too and
            // `child.kill()` alone would never reach it, leaving it running long after this
            // probe has given up.
            #[cfg(unix)]
            if let Ok(pgid) = i32::try_from(child.id()) {
                // SAFETY: `killpg` takes an integer group id, dereferences no memory this
                // process owns, and this pgid is the group `process_group(0)` created for
                // this child alone — never this CLI's own group.
                unsafe {
                    libc::killpg(pgid, libc::SIGKILL);
                }
            }
            let _ = child.kill();
            // Reaped, so a `goalcup doctor` over several connectors does not leave a zombie
            // behind each. The kill is delivered whatever the child is blocked on, and
            // the pipes are held by the drain threads rather than by this one, so this
            // cannot be the wait that hangs.
            let _ = child.wait();
            timed_out = true;
            break;
        }
        std::thread::sleep(PROBE_POLL);
    }
    if timed_out {
        return Err(CliError::Refused(format!(
            "`{}` did not answer within {timeout:?}, so `{binary}`'s version is unknown.",
            m.detection.version_command
        )));
    }
    // The child answered; what it printed is collected under the same deadline. A pipe still
    // open after it is a probe failure and not a wait: something the harness started outlives
    // the harness and holds this probe's output, so there is no telling whether what did
    // arrive is all of it.
    let mut out_bytes = Vec::new();
    let mut err_bytes = Vec::new();
    for _ in 0..pipes {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok((Stream::Stdout, bytes)) => out_bytes = bytes,
            Ok((Stream::Stderr, bytes)) => err_bytes = bytes,
            Err(_) => {
                // The child is gone but its group is not: whatever it backgrounded still
                // holds the pipe, and the group id it inherited is the child's pid, so the
                // same signal that closes the deadline path closes this one.
                #[cfg(unix)]
                if let Ok(pgid) = i32::try_from(child.id()) {
                    // SAFETY: as above — an integer group id created by `process_group(0)`
                    // for this child alone, never this CLI's own group.
                    unsafe {
                        libc::killpg(pgid, libc::SIGKILL);
                    }
                }
                return Err(CliError::Refused(format!(
                    "`{}` exited and something it started still holds this probe's output \
                     after {timeout:?}, so `{binary}`'s version is unknown.",
                    m.detection.version_command
                )));
            }
        }
    }
    // The first non-empty line, and stderr only when stdout said nothing: a harness that
    // prints a banner, an update notice and then its version must not have the notice read
    // as the version, and `extract_version` takes the first version in whatever it is given.
    let stdout = String::from_utf8_lossy(&out_bytes).to_string();
    let stderr = String::from_utf8_lossy(&err_bytes).to_string();
    let line = first_line(&stdout).or_else(|| first_line(&stderr));
    match line.as_deref().and_then(extract_version) {
        Some(version) => Ok(version),
        None => Err(CliError::Refused(format!(
            "`{}` printed no version this build could read ({:?}).",
            m.detection.version_command,
            line.unwrap_or_default()
        ))),
    }
}

/// Which of the probe's two pipes a drained buffer came off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stream {
    Stdout,
    Stderr,
}

/// Read one of the child's pipes to the end, on a thread of its own, and post what it held.
///
/// Detached deliberately: the caller never joins it, because a join is exactly the unbounded
/// wait this exists to avoid. A thread left blocked on a pipe a grandchild still holds ends
/// when this short-lived process does.
///
/// A read that fails hands back whatever arrived before it did. The alternative is a probe
/// that reports no version because the last read of the pipe went wrong, which is a worse
/// answer than the version that was already sitting in the buffer.
fn drain(
    which: Stream,
    mut pipe: impl std::io::Read + Send + 'static,
    tx: std::sync::mpsc::Sender<(Stream, Vec<u8>)>,
) {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        // The receiver is gone on every path that gave up on this pipe, and a send to a
        // hung-up channel is the expected end of this thread rather than an error.
        let _ = tx.send((which, bytes));
    });
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

/// The environment the probe runs the harness in.
///
/// Neither this process's environment nor an empty one. `PATH` and the rest of what was
/// handed in stay, because a harness with no `PATH` cannot find its own helpers; what is
/// removed is everything that could point the harness at a different server or a different
/// recording of the answer:
///
/// * anything ending `_URL` — `ANTHROPIC_BASE_URL`, `CODEX_BASE_URL`, `AMP_URL`;
/// * anything ending `_PROXY`, in either case (`HTTPS_PROXY`, `http_proxy`, `ALL_PROXY`);
/// * every variable this connector's own `preflight.banned_env` names, because a probe run
///   under a variable the run itself refuses is a measurement of a machine nobody will run on.
///
/// D1's rule on the launch side is the same rule: the tracker never sets a model base URL.
/// This is that rule applied to the one child preflight spawns — and it is literally the same
/// rule, [`crate::launch::is_denied`], rather than a second spelling of it that would agree
/// with the launch's until one of the two was widened.
pub fn probe_env(c: &Connector, env: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    env.iter()
        .filter(|(k, _)| {
            !crate::launch::is_denied(k) && !c.manifest.preflight.banned_env.contains(k)
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}
