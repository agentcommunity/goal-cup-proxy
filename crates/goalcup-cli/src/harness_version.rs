//! The harness's own account of its version (D53): `<program> <version_args>`, run once before
//! the launch, its first line of stdout recorded as `Launch.harness_version`.
//!
//! It is reported, not proven. The build is identified by the digests the daemon records
//! (`Launch.harness_build`); this line is what the harness says about itself, kept beside them
//! for a reader. A connector without `version_args`, a probe that fails, prints nothing
//! usable or outlives [`VERSION_PROBE_TIMEOUT`] all record [`UNREPORTED`], and none of them
//! stops a run.
//!
//! The probe is not a capture child: it carries no goal, no proxy and no credential. Its
//! environment is built from nothing, as the capture launch's is: `PATH` (the one the harness is
//! given, [`ResolvedExecutable::search_path`]: [`crate::launch::CAPTURE_PATH`], or for a
//! chat-mode connector the participant's own, so an npm harness finds `node`) and `HOME` (the
//! run's private home), and nothing else from the participant's environment. Its stdin is
//! `/dev/null`, its stderr is discarded, and it leads a process group of its own so a timeout
//! ends whatever it started.

use crate::launch::{CAPTURE_PATH, ResolvedExecutable};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// What is recorded when the harness did not say.
pub const UNREPORTED: &str = "unreported";

/// How long the probe may run before it is killed and [`UNREPORTED`] is recorded.
pub const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest version line kept; a longer one is cut to this many bytes, never refused (D53:
/// a version never stops a run). It matches the server's bound on `harness_version`, so a line
/// the CLI keeps is one the server accepts.
pub const MAX_REPORTED_VERSION_BYTES: usize = 64;

/// How much of the probe's stdout is read. The first line is all that is kept.
const MAX_PROBE_STDOUT_BYTES: u64 = 4096;

/// How often the probe is polled for its exit.
const POLL: Duration = Duration::from_millis(10);

/// The version `executable` reports for `version_args`, with `home` as its `HOME` and working
/// directory, or [`UNREPORTED`]. Never an error: a version is reported, not required.
pub fn reported_version(
    executable: &ResolvedExecutable,
    version_args: &[String],
    home: &Path,
) -> String {
    probe(
        Path::new(executable.path()),
        version_args,
        home,
        VERSION_PROBE_TIMEOUT,
        executable.search_path(),
    )
}

/// [`reported_version`] for any program path, with the timeout as a parameter (tests use a short
/// one).
/// [`reported_version`] with `home` created fresh (mode 0700) for the probe and removed after
/// it, whatever the probe left there. `unreported` when the directory cannot be made.
pub fn reported_version_in_scratch_home(
    executable: &ResolvedExecutable,
    version_args: &[String],
    home: &Path,
) -> String {
    if version_args.is_empty() {
        return UNREPORTED.to_string();
    }
    #[cfg(unix)]
    let made = {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(home)
    };
    #[cfg(not(unix))]
    let made = std::fs::create_dir(home);
    if made.is_err() {
        return UNREPORTED.to_string();
    }
    let version = reported_version(executable, version_args, home);
    let _ = std::fs::remove_dir_all(home);
    version
}

pub fn reported_version_within(
    program: &Path,
    version_args: &[String],
    home: &Path,
    timeout: Duration,
) -> String {
    probe(program, version_args, home, timeout, CAPTURE_PATH)
}

fn probe(
    program: &Path,
    version_args: &[String],
    home: &Path,
    timeout: Duration,
    path: &str,
) -> String {
    if version_args.is_empty() {
        return UNREPORTED.to_string();
    }
    let mut command = Command::new(program);
    command
        .args(version_args)
        .current_dir(home)
        .env_clear()
        .env("PATH", path)
        .env("HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let Ok(mut child) = command.spawn() else {
        return UNREPORTED.to_string();
    };
    // Read on a thread, so a probe that fills its pipe still exits and one that never closes it
    // cannot hold this launch past the deadline.
    let stdout = child.stdout.take();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(stdout) = stdout {
            let _ = stdout.take(MAX_PROBE_STDOUT_BYTES).read_to_end(&mut bytes);
        }
        let _ = sender.send(bytes);
    });
    let deadline = Instant::now() + timeout;
    let Some(status) = wait_or_end_group(&mut child, timeout) else {
        return UNREPORTED.to_string();
    };
    if !status.success() {
        return UNREPORTED.to_string();
    }
    // Output already buffered is still read when the exit came right at the deadline.
    let remaining = deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(50));
    match receiver.recv_timeout(remaining) {
        Ok(bytes) => reported_line(&bytes),
        Err(_) => UNREPORTED.to_string(),
    }
}

/// Wait up to `timeout` for `child`, which leads a process group of its own, to exit. When it
/// has not, its group is sent SIGKILL while its unreaped leader still holds the group id, and the
/// leader is reaped, so nothing can reach a recycled pid; `None` then. Shared by the version
/// probe and the connector's `capture.configure` commands (D61): both run the harness's own
/// program once before the launch, unregistered, with no goal and no key.
pub(crate) fn wait_or_end_group(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Option<std::process::ExitStatus> {
    wait_or_end_group_unless(child, timeout, &|| false)
}

/// [`wait_or_end_group`] that also ends the group as soon as `stop` says so: `goalcup start`'s
/// GitHub checks (the participant's `ssh`, `gh` and `git`, D69, never a recorded run) end at a
/// Ctrl-C ([`crate::arming::interrupted`]).
pub(crate) fn wait_or_end_group_unless(
    child: &mut std::process::Child,
    timeout: Duration,
    stop: &dyn Fn() -> bool,
) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline && !stop() => std::thread::sleep(POLL),
            _ => break,
        }
    }
    // Not yet reaped, so its pid still names its group: end the group, then the child.
    #[cfg(unix)]
    if let Ok(pgid) = libc::pid_t::try_from(child.id()) {
        // SAFETY: `killpg` takes plain integers and has no memory effects.
        unsafe { libc::killpg(pgid, libc::SIGKILL) };
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

/// The first line of `stdout`, trimmed, if it is printable ASCII and not empty; [`UNREPORTED`]
/// otherwise. A line longer than [`MAX_REPORTED_VERSION_BYTES`] is cut to its first
/// [`MAX_REPORTED_VERSION_BYTES`] bytes.
pub fn reported_line(stdout: &[u8]) -> String {
    let line = stdout.split(|b| *b == b'\n').next().unwrap_or_default();
    let Ok(text) = std::str::from_utf8(line) else {
        return UNREPORTED.to_string();
    };
    let text = text.trim();
    let printable = text.bytes().all(|b| (b' '..=b'~').contains(&b));
    if text.is_empty() || !printable {
        return UNREPORTED.to_string();
    }
    // Printable ASCII only, so any byte index is a char boundary.
    text[..text.len().min(MAX_REPORTED_VERSION_BYTES)].to_string()
}
