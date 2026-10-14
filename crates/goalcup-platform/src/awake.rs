//! Keep the machine awake while a run is in progress.
//!
//! The lock is a child process the platform already ships (`caffeinate` on macOS,
//! `systemd-inhibit` on Linux). Dropping the [`SleepLock`] kills and reaps it; on Linux the
//! child also receives `SIGTERM` if this process dies first.
//!
//! The child is spawned by a dedicated thread that owns it for its whole life. That matters on
//! Linux: the parent-death signal fires when the *spawning thread* exits, not the process, so a
//! short-lived caller thread must not be the one that spawns the inhibitor.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::machine::is_wsl;

const WSL_REFUSAL: &str =
    "the Windows host decides when this machine sleeps; set its power settings";
const NONE_FOUND: &str = "no sleep inhibitor on this machine";
/// How long a fresh inhibitor must stay alive before the hold counts as taken.
const SETTLE: Duration = Duration::from_millis(150);

/// An inhibitor child process; the machine may sleep again once this is dropped.
///
/// A thread owns the child. Dropping the lock stops that thread, which kills and reaps the
/// child before `drop` returns.
#[derive(Debug)]
pub struct SleepLock {
    pid: u32,
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl SleepLock {
    /// The inhibitor's process id.
    #[doc(hidden)]
    pub fn pid(&self) -> u32 {
        self.pid
    }
}

impl Drop for SleepLock {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            // A closed channel means the thread already ended; either way it releases the child.
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Hold the machine awake until the returned lock is dropped.
///
/// `why` is shown to the user by the platform's power tooling where it has such a place.
///
/// Returns an error when the inhibitor cannot start or exits within a moment of starting
/// (power policy denied, no session bus). The caller owns the returned lock; it may be held and
/// dropped on any thread, because the inhibitor is owned by a thread of its own.
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
            let parent = std::process::id() as libc::pid_t;
            // SAFETY: the closure runs between fork and exec and only calls `prctl`, `getppid`
            // and `_exit`, which are async-signal-safe.
            unsafe {
                c.pre_exec(move || {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM as libc::c_ulong) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    // The parent may have died before the signal was armed.
                    if libc::getppid() != parent {
                        libc::_exit(0);
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
    start(command)
}

/// Spawn `command` from a dedicated thread that keeps the child until told to stop.
fn start(mut command: Command) -> Result<SleepLock, String> {
    let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, String>>();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let thread = std::thread::Builder::new()
        .name("goalcup-awake".into())
        .spawn(move || {
            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("could not start the sleep inhibitor: {e}")));
                    return;
                }
            };
            let deadline = Instant::now() + SETTLE;
            loop {
                match child.try_wait() {
                    // `try_wait` has already reaped it.
                    Ok(Some(status)) => {
                        let _ = ready_tx
                            .send(Err(format!("the sleep inhibitor exited at once: {status}")));
                        return;
                    }
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Ok(None) => break,
                    Err(e) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = ready_tx.send(Err(format!("the sleep inhibitor is lost: {e}")));
                        return;
                    }
                }
            }
            let _ = ready_tx.send(Ok(child.id()));
            // A message or a dropped sender both mean the lock is gone.
            let _ = stop_rx.recv();
            let _ = child.kill();
            let _ = child.wait();
        })
        .map_err(|e| format!("could not start the sleep inhibitor: {e}"))?;
    match ready_rx.recv() {
        Ok(Ok(pid)) => Ok(SleepLock {
            pid,
            stop: Some(stop_tx),
            thread: Some(thread),
        }),
        Ok(Err(why)) => {
            let _ = thread.join();
            Err(why)
        }
        Err(_) => {
            let _ = thread.join();
            Err("the sleep inhibitor thread ended unexpectedly".into())
        }
    }
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
