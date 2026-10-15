//! Keep the machine awake while a run is in progress.
//!
//! The lock is a child process the platform already ships (`caffeinate` on macOS,
//! `systemd-inhibit` on Linux). Dropping the [`SleepLock`] kills and reaps it; on Linux the
//! child also receives `SIGTERM` if this process dies first.
//!
//! On Linux a hold counts as taken only once the inhibitor has run its command: `systemd-inhibit`
//! takes the lock first and then executes the command it was given, so the command is run as
//! `sh -c 'echo ready; exec sleep infinity'` and the hold waits for that `ready`. On macOS
//! `caffeinate` has no such moment, so a fresh child must merely stay alive for a short while.
//!
//! The child is spawned by a dedicated thread that owns it for its whole life. That matters on
//! Linux: the parent-death signal fires when the *spawning thread* exits, not the process, so a
//! short-lived caller thread must not be the one that spawns the inhibitor.

use std::io::{BufRead as _, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::machine::{find_on_path, is_executable, is_wsl};

const WSL_REFUSAL: &str =
    "the Windows host decides when this machine sleeps; set its power settings";
const NONE_FOUND: &str = "no sleep inhibitor on this machine";
/// How long a fresh `caffeinate` must stay alive before the hold counts as taken (macOS only; the
/// Linux path waits for proof instead).
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
/// Returns an error when the inhibitor cannot start or cannot take the lock (power policy
/// denied, no session bus). On Linux an `Ok` means the inhibitor has taken the lock and is
/// running its command, however long that took; there is no fixed wait to outlast. On macOS an
/// `Ok` means `caffeinate` stayed alive for a short settling time. The caller owns the returned
/// lock; it may be held and dropped on any thread, because the inhibitor is owned by a thread of
/// its own.
///
/// Worst case on Linux: `hold` blocks until `systemd-inhibit` has the lock or gives up. If its
/// D-Bus call hangs, that is bounded by the bus method-call timeout, on the order of 25 seconds.
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
    // Linux: the inhibitor's command reports back once it runs, which is proof of the lock.
    let proof = !cfg!(target_os = "macos");
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
            "sh",
            "-c",
            "echo ready; exec sleep infinity",
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
        .stdout(if proof { Stdio::piped() } else { Stdio::null() })
        .stderr(Stdio::null());
    // A process group of its own: what a terminal sends its foreground group (a Ctrl-C, a closed
    // window) reaches the host, which rides it out to seal and upload, and must not end the
    // lock that keeps the machine awake meanwhile. Its life stays tied to the host's:
    // `caffeinate -w <pid>` on macOS, `PR_SET_PDEATHSIG` on Linux, and the owning thread.
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    start(command, proof)
}

/// Spawn `command` from a dedicated thread that keeps the child until told to stop.
///
/// With `proof` the child's stdout is piped and the hold waits for its first line, `ready`;
/// without, the child must merely stay alive for [`SETTLE`].
fn start(mut command: Command, proof: bool) -> Result<SleepLock, String> {
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
            let taken = if proof {
                await_ready(&mut child)
            } else {
                await_settled(&mut child)
            };
            match taken {
                Ok(()) => {
                    let _ = ready_tx.send(Ok(child.id()));
                }
                Err(why) => {
                    let _ = ready_tx.send(Err(why));
                    return;
                }
            }
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

/// Wait for the child's first stdout line to be `ready`. On any other outcome the child has been
/// reaped (or killed and reaped) when this returns.
///
/// The read has no timeout of its own. It ends when the command prints, or when the inhibitor
/// fails and exits (the pipe reaches end of file); a stuck bus call is bounded by systemd's own
/// D-Bus timeout, after which `systemd-inhibit` fails.
fn await_ready(child: &mut Child) -> Result<(), String> {
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("the sleep inhibitor is lost: no output pipe".into());
    };
    let mut line = String::new();
    let read = BufReader::new(stdout).read_line(&mut line);
    // The read end is dropped here: the command prints nothing after `ready`.
    match read {
        Ok(_) if line.trim_end() == "ready" => Ok(()),
        Ok(0) => {
            // End of file: the inhibitor failed before it ran the command, and is exiting.
            let status = child
                .wait()
                .map_err(|e| format!("the sleep inhibitor is lost: {e}"))?;
            Err(format!("the sleep inhibitor exited at once: {status}"))
        }
        Ok(_) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(format!(
                "the sleep inhibitor said {:?}, not ready",
                line.trim_end()
            ))
        }
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(format!("the sleep inhibitor is lost: {e}"))
        }
    }
}

/// Wait [`SETTLE`] and require the child to still be running.
fn await_settled(child: &mut Child) -> Result<(), String> {
    let deadline = Instant::now() + SETTLE;
    loop {
        match child.try_wait() {
            // `try_wait` has already reaped it.
            Ok(Some(status)) => {
                return Err(format!("the sleep inhibitor exited at once: {status}"));
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => return Ok(()),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("the sleep inhibitor is lost: {e}"));
            }
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
