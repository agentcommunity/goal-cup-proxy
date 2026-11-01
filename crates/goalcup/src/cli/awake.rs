//! The sleep lock (PRD-10 section 9): the probe, and the lock `start` holds. The tool facts are 0.7's (`goalcup-platform/src/awake.rs`): macOS
//! `/usr/bin/caffeinate`, Linux `systemd-inhibit` with a running systemd, none on WSL. The macOS
//! branch has not been run in a box.

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::connector::launch::resolve_program;

/// None when this computer has a sleep inhibitor goalcup can hold, else why not, in a few words.
/// `vars` is the environment whose `PATH` is searched, as for the harness program.
pub fn missing(vars: &BTreeMap<String, String>) -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        let found = Path::new("/usr/bin/caffeinate").exists();
        return (!found).then_some("caffeinate is missing");
    }
    if super::folder::wsl_version().is_some() {
        return Some("set Windows sleep to Never");
    }
    let tool = resolve_program("systemd-inhibit", vars).is_absolute();
    let systemd = Path::new("/run/systemd/system").is_dir();
    (!(tool && systemd)).then_some("no systemd-inhibit with a running systemd")
}

/// An inhibitor child; the machine may sleep again once this is dropped. A thread owns the child:
/// on Linux the parent-death signal fires when the spawning thread ends, not the process.
#[derive(Debug)]
pub struct SleepLock {
    pid: u32,
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl SleepLock {
    pub fn pid(&self) -> u32 {
        self.pid
    }
}

impl Drop for SleepLock {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Take the lock if this computer can. The second value is the one launch and status line when it
/// cannot: a warning, never a `PROBLEM:` line.
pub fn take(vars: &BTreeMap<String, String>) -> (Option<SleepLock>, Option<String>) {
    let held = match missing(vars) {
        Some(why) => Err(why.to_string()),
        None => hold_with(&|name| Some(resolve_program(name, vars)).filter(|p| p.is_absolute())),
    };
    match held {
        Ok(lock) => (Some(lock), None),
        Err(why) => (None, Some(super::text::no_sleep_lock(&why))),
    }
}

/// Hold the machine awake until the lock is dropped. On Linux an `Ok` means `systemd-inhibit` has
/// run its command (it takes the lock first); worst case a hanging D-Bus call bounds this at about
/// 25 seconds. On macOS `caffeinate -w <pid>` must merely stay alive for a moment.
pub fn hold_with(which: &dyn Fn(&str) -> Option<PathBuf>) -> Result<SleepLock, String> {
    let mac = cfg!(target_os = "macos");
    let tool = if mac { "caffeinate" } else { "systemd-inhibit" };
    let program = which(tool).ok_or_else(|| format!("{tool} is missing"))?;
    let mut command = Command::new(program);
    if mac {
        command.args(["-i", "-s", "-w", &std::process::id().to_string()]);
    } else {
        command.args(["--what=sleep:idle", "--mode=block", "--who=goalcup"]);
        command.args([
            "--why=goalcup is recording a run",
            "sh",
            "-c",
            "echo ready; exec sleep infinity",
        ]);
        #[cfg(target_os = "linux")]
        {
            let parent = std::process::id() as libc::pid_t;
            // SAFETY: between fork and exec the closure only calls prctl, getppid and _exit.
            unsafe {
                command.pre_exec(move || {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM as libc::c_ulong) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::getppid() != parent {
                        libc::_exit(0);
                    }
                    Ok(())
                });
            }
        }
    }
    // Its own process group: a Ctrl-C meant for the harness must not end the lock.
    command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    command.stdout(if mac { Stdio::null() } else { Stdio::piped() });
    let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, String>>();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let thread = std::thread::Builder::new()
        .name("goalcup-awake".into())
        .spawn(move || {
            let mut child = match command.spawn() {
                Ok(c) => c,
                Err(e) => {
                    return drop(
                        ready_tx.send(Err(format!("the sleep inhibitor did not start: {e}"))),
                    );
                }
            };
            match settle(&mut child, !mac) {
                Ok(()) => drop(ready_tx.send(Ok(child.id()))),
                Err(why) => return drop(ready_tx.send(Err(why))),
            }
            let _ = stop_rx.recv();
            let _ = child.kill();
            let _ = child.wait();
        })
        .map_err(|e| format!("the sleep inhibitor did not start: {e}"))?;
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
            Err("the sleep inhibitor ended unexpectedly".into())
        }
    }
}

/// Linux: wait for the child's first line, `ready`. macOS: the child must stay alive 150 ms.
/// On any failure the child is reaped.
fn settle(child: &mut std::process::Child, proof: bool) -> Result<(), String> {
    let reap = |child: &mut std::process::Child| {
        let _ = child.kill();
        let _ = child.wait();
    };
    if proof {
        let Some(out) = child.stdout.take() else {
            reap(child);
            return Err("the sleep inhibitor has no output".into());
        };
        let mut line = String::new();
        let read = BufReader::new(out).read_line(&mut line);
        if matches!(read, Ok(n) if n > 0) && line.trim_end() == "ready" {
            return Ok(());
        }
        reap(child);
        return Err("the sleep inhibitor could not take the lock".into());
    }
    let until = Instant::now() + Duration::from_millis(150);
    while Instant::now() < until {
        if matches!(child.try_wait(), Ok(Some(_)) | Err(_)) {
            return Err("the sleep inhibitor exited at once".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}
