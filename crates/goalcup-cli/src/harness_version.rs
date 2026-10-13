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
//! environment is built from nothing, as the capture launch's is: `PATH`
//! ([`crate::launch::CAPTURE_PATH`]) and `HOME` (the run's private home), and nothing from the
//! participant's environment. Its stdin is `/dev/null`, its stderr is discarded, and it leads a
//! process group of its own so a timeout ends whatever it started.

use crate::launch::{CAPTURE_PATH, ResolvedExecutable};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// What is recorded when the harness did not say.
pub const UNREPORTED: &str = "unreported";

/// How long the probe may run before it is killed and [`UNREPORTED`] is recorded.
pub const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest version line kept; a longer one is [`UNREPORTED`].
pub const MAX_REPORTED_VERSION_BYTES: usize = 128;

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
    reported_version_within(
        Path::new(executable.path()),
        version_args,
        home,
        VERSION_PROBE_TIMEOUT,
    )
}

/// [`reported_version`] for any program path, with the timeout as a parameter (tests use a short
/// one).
pub fn reported_version_within(
    program: &Path,
    version_args: &[String],
    home: &Path,
    timeout: Duration,
) -> String {
    if version_args.is_empty() {
        return UNREPORTED.to_string();
    }
    let mut command = Command::new(program);
    command
        .args(version_args)
        .current_dir(home)
        .env_clear()
        .env("PATH", CAPTURE_PATH)
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
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            _ => break None,
        }
    };
    let Some(status) = status else {
        // Not yet reaped, so its pid still names its group: end the group, then the child.
        #[cfg(unix)]
        if let Ok(pgid) = libc::pid_t::try_from(child.id()) {
            // SAFETY: `killpg` takes plain integers and has no memory effects.
            unsafe { libc::killpg(pgid, libc::SIGKILL) };
        }
        let _ = child.kill();
        let _ = child.wait();
        return UNREPORTED.to_string();
    };
    if !status.success() {
        return UNREPORTED.to_string();
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    match receiver.recv_timeout(remaining) {
        Ok(bytes) => reported_line(&bytes),
        Err(_) => UNREPORTED.to_string(),
    }
}

/// The first line of `stdout`, trimmed, if it is 1 to [`MAX_REPORTED_VERSION_BYTES`] bytes of
/// printable ASCII; [`UNREPORTED`] otherwise.
pub fn reported_line(stdout: &[u8]) -> String {
    let line = stdout.split(|b| *b == b'\n').next().unwrap_or_default();
    let Ok(text) = std::str::from_utf8(line) else {
        return UNREPORTED.to_string();
    };
    let text = text.trim();
    let printable = text.bytes().all(|b| (b' '..=b'~').contains(&b));
    if text.is_empty() || text.len() > MAX_REPORTED_VERSION_BYTES || !printable {
        return UNREPORTED.to_string();
    }
    text.to_string()
}
