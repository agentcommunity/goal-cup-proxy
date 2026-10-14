//! Keep the machine awake while a run is in progress.
//!
//! The lock is a child process the platform already ships (`caffeinate` on macOS,
//! `systemd-inhibit` on Linux). Dropping the [`SleepLock`] kills and reaps it; on Linux the
//! child also receives `SIGTERM` if this process dies first.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::machine::is_wsl;

const WSL_REFUSAL: &str =
    "the Windows host decides when this machine sleeps; set its power settings";
const NONE_FOUND: &str = "no sleep inhibitor on this machine";

/// An inhibitor child process; the machine may sleep again once this is dropped.
#[derive(Debug)]
pub struct SleepLock {
    child: Child,
}

impl SleepLock {
    /// The inhibitor's process id.
    #[doc(hidden)]
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for SleepLock {
    fn drop(&mut self) {
        // Errors mean the child is already gone; there is nothing left to release.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Hold the machine awake until the returned lock is dropped.
///
/// `why` is shown to the user by the platform's power tooling where it has such a place.
pub fn hold(why: &str) -> Result<SleepLock, String> {
    hold_with(why, &find_tool, is_wsl())
}

/// [`hold`] with the tool lookup and the WSL answer supplied, so tests need not change the host.
#[doc(hidden)]
pub fn hold_with(
    why: &str,
    which: &dyn Fn(&str) -> Option<PathBuf>,
    wsl: bool,
) -> Result<SleepLock, String> {
    if wsl {
        return Err(WSL_REFUSAL.into());
    }
    let mut command = if cfg!(target_os = "macos") {
        let tool = which("caffeinate").ok_or_else(|| String::from(NONE_FOUND))?;
        let mut c = Command::new(tool);
        c.args(["-i", "-s", "-w", &std::process::id().to_string()]);
        c
    } else {
        let tool = which("systemd-inhibit").ok_or_else(|| String::from(NONE_FOUND))?;
        let mut c = Command::new(tool);
        c.args([
            "--what=sleep:idle",
            "--mode=block",
            "--who=goalcup",
            &format!("--why={why}"),
            "sleep",
            "infinity",
        ]);
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt as _;
            // SAFETY: the closure runs between fork and exec and only calls `prctl`, which is
            // async-signal-safe.
            unsafe {
                c.pre_exec(|| {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM as libc::c_ulong) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        c
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
        .spawn()
        .map(|child| SleepLock { child })
        .map_err(|e| format!("could not start the sleep inhibitor: {e}"))
}

/// The tool lookup `hold` uses: `caffeinate` only from its fixed system path, anything else
/// from `PATH`.
fn find_tool(name: &str) -> Option<PathBuf> {
    if name == "caffeinate" {
        let fixed = PathBuf::from("/usr/bin/caffeinate");
        return is_executable(&fixed).then_some(fixed);
    }
    find_on_path(name)
}

/// The first executable called `name` on `PATH`.
pub(crate) fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
