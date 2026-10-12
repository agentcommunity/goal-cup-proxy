//! Linux: `goalcup start` is a child subreaper, and sweeps what the harness left behind after it
//! exited, before the seal (TERMINATION.md, "RULED: on Linux, `Confirmed` excludes fork-and-leave
//! wrappers").
//!
//! Linux has no fork witness, so a process the harness forks and leaves is in no ppid walk the
//! daemon can take, and by the seal the root is reaped and the daemon answers `AlreadyStopped`
//! without walking at all. With `PR_SET_CHILD_SUBREAPER` such an orphan is reparented to this
//! process instead of to init, and stays this process's unreaped child until it is waited for:
//! its pid cannot be recycled, so signalling it needs no pidfd and cannot hit a stranger.
//!
//! **What is swept.** After the root's `wait` returns, every child of this process (`/proc`,
//! ppid equal to ours) that started no earlier than the root and is not a daemon this process
//! started: SIGTERM, the connector's grace, SIGKILL, and each one reaped by its own pid. A child
//! that forks again hands its children to this process when it dies, so the sweep repeats until
//! a pass finds no child at all, within the grace plus [`SWEEP_SLACK`]. What outlives that is
//! reported by pid, never hidden.
//!
//! **What is never swept.**
//! - The daemon: [`note_daemon`] records each one this process starts, by pid and start tick, and
//!   a thread of `daemon::spawn` waits for it by its pid.
//! - A child this process spawned and is still waiting for. None exists at the sweep: the only
//!   `Command`s a capture run spawns are the daemon, the gated harness (whose spawn thread is
//!   joined at its release), and the `/bin/ps` runs of `launch.rs` and `released_child.rs`, which
//!   are `.output()`-waited on this same thread before the root's `wait`. No other thread (the
//!   tees, the proxy, the daemon's reaper) spawns anything. The start filter keeps anything this
//!   process inherited across its own `exec` out as well.
//! - Threads: `/proc` lists thread-group leaders only, and this process's own threads are not
//!   its children.
//!
//! Only the binary arms it (`main.rs`). A test binary that drives `start_with` in-process runs
//! other tests on other threads, each with `Command`s in flight that a sweep would reap from
//! under them, so an in-process start is never a subreaper and never sweeps.
//!
//! The daemon is unchanged and the trace and seal gain nothing: the sweep is printed on stdout.
//! If this process dies, what it adopted goes to init as before; and a `goalcup seal` from
//! another terminal while the harness runs can report `all_gone` while this sweep is still
//! ending an orphan the daemon's walk could not see.
//!
//! On every other platform this module does nothing: macOS has no child subreaper, and its
//! kqueue witness covers the same shape in the daemon.

use std::time::Duration;

/// How long the sweep may run past the grace before what is still alive is reported.
pub const SWEEP_SLACK: Duration = Duration::from_secs(5);

/// Whether [`arm`] made this process a child subreaper.
#[cfg(target_os = "linux")]
static ARMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Every daemon this process started: its pid and, where it could be read, its start tick.
#[cfg(target_os = "linux")]
static DAEMONS: std::sync::Mutex<Vec<(u32, Option<u64>)>> = std::sync::Mutex::new(Vec::new());

/// Make this process a child subreaper, on Linux; elsewhere, nothing. The error is the OS's,
/// and the caller says once that nothing will be swept.
pub fn arm() -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: `prctl(PR_SET_CHILD_SUBREAPER, 1)` takes integers only and touches no memory.
        if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        ARMED.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    Ok(())
}

/// Record a daemon this process just started, so no sweep ever signals it.
pub(crate) fn note_daemon(pid: u32) {
    #[cfg(target_os = "linux")]
    {
        let start = linux::stat(pid).map(|stat| stat.start);
        DAEMONS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((pid, start));
    }
    #[cfg(not(target_os = "linux"))]
    let _ = pid;
}

/// What one sweep did.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Swept {
    /// Live processes signalled and then reaped.
    pub swept: usize,
    /// Still alive when the sweep's bound ran out.
    pub left: Vec<u32>,
    /// `/proc` (or the root's entry in it) could not be read, so the sweep stopped there.
    pub unlisted: bool,
}

impl Swept {
    /// What the launcher prints: nothing when there was nothing to sweep.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.swept > 0 {
            lines.push(format!(
                "swept {} process(es) the harness left behind after it exited.",
                self.swept
            ));
        }
        if !self.left.is_empty() {
            let pids: Vec<String> = self.left.iter().map(u32::to_string).collect();
            lines.push(format!(
                "  ({} process(es) the harness left behind outlived the sweep's grace and SIGKILL \
                 and are still running (pids: {}); end them by hand.)",
                self.left.len(),
                pids.join(", ")
            ));
        }
        if self.unlisted {
            lines.push(
                "  (this command could not read its own children from /proc, so it cannot say \
                 that nothing the harness left behind is still running.)"
                    .to_string(),
            );
        }
        lines
    }
}

/// A sweep, armed for one root while that root is still this process's unreaped child.
pub(crate) struct Sweep {
    /// The root's start tick; a child that started earlier is not the harness's.
    #[cfg(target_os = "linux")]
    root_start: Option<u64>,
}

impl Sweep {
    /// `Some` only on Linux, and only when [`arm`] succeeded. Taken before the root's `wait`,
    /// while its `/proc` entry still exists.
    pub fn for_root(pid: u32) -> Option<Sweep> {
        #[cfg(target_os = "linux")]
        {
            ARMED
                .load(std::sync::atomic::Ordering::SeqCst)
                .then(|| Sweep {
                    root_start: linux::stat(pid).map(|stat| stat.start),
                })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = pid;
            None
        }
    }

    /// Run it, after the root's `wait` returned: see the module note.
    pub fn run(self, grace: Duration) -> Swept {
        #[cfg(target_os = "linux")]
        {
            match self.root_start {
                Some(root_start) => linux::sweep(root_start, grace),
                None => Swept {
                    unlisted: true,
                    ..Swept::default()
                },
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = grace;
            Swept::default()
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{DAEMONS, SWEEP_SLACK, Swept};
    use std::collections::BTreeSet;
    use std::time::{Duration, Instant};

    /// How often the sweep looks again.
    const POLL: Duration = Duration::from_millis(20);

    /// The three fields of `/proc/<pid>/stat` the sweep reads.
    #[derive(Debug, PartialEq, Eq)]
    pub(super) struct Stat {
        /// Exited and not yet reaped (`Z`, or `X` on its way out).
        pub dead: bool,
        pub ppid: u32,
        /// `starttime`, in clock ticks since boot.
        pub start: u64,
    }

    /// `pid`'s stat, a zombie's included, or `None` once it is gone.
    pub(super) fn stat(pid: u32) -> Option<Stat> {
        parse(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
    }

    /// Field 2 is parenthesised and may hold spaces or `)`, so the fields are split after its
    /// last `)`: `state`, then `ppid`, and `starttime` 18 fields later.
    pub(super) fn parse(stat: &str) -> Option<Stat> {
        let mut fields = stat.get(stat.rfind(')')? + 2..)?.split_whitespace();
        let dead = matches!(fields.next()?, "Z" | "X");
        let ppid = fields.next()?.parse().ok()?;
        let start = fields.nth(17)?.parse().ok()?;
        Some(Stat { dead, ppid, start })
    }

    /// A daemon this process started: the same pid, and the same start tick where one was read.
    fn is_daemon(pid: u32, start: u64) -> bool {
        DAEMONS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|&(daemon, at)| daemon == pid && at.is_none_or(|at| at == start))
    }

    /// This process's children that the sweep may touch, each with whether it has exited.
    fn children(me: u32, root_start: u64) -> std::io::Result<Vec<(u32, bool)>> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir("/proc")? {
            let Some(pid) = entry?
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            // Gone between the listing and the read: nothing to sweep.
            let Some(stat) = stat(pid) else { continue };
            if stat.ppid == me && stat.start >= root_start && !is_daemon(pid, stat.start) {
                found.push((pid, stat.dead));
            }
        }
        Ok(found)
    }

    /// Reap `pid` if it has exited. `ECHILD` is a child that is no longer ours: gone too.
    fn reap(pid: u32) -> bool {
        let mut status = 0;
        // SAFETY: `pid` is a positive child pid and `status` a valid local the call writes.
        match unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) } {
            0 => false,
            -1 => std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD),
            _ => true,
        }
    }

    pub(super) fn sweep(root_start: u64, grace: Duration) -> Swept {
        let me = std::process::id();
        let began = Instant::now();
        let (kill_at, until) = (began + grace, began + grace + SWEEP_SLACK);
        let (mut termed, mut killed) = (BTreeSet::new(), BTreeSet::new());
        let mut swept = 0;
        loop {
            let Ok(found) = children(me, root_start) else {
                return Swept {
                    swept,
                    unlisted: true,
                    ..Swept::default()
                };
            };
            // Done only when a pass finds no child at all: a child that died during the pass
            // handed its own children over before it became a zombie, and the next pass sees them.
            if found.is_empty() {
                return Swept {
                    swept,
                    ..Swept::default()
                };
            }
            let (mut live, mut unreaped) = (Vec::new(), false);
            for (pid, dead) in found {
                if !dead {
                    live.push(pid);
                } else if reap(pid) {
                    // Forgotten once reaped, so a recycled pid is a stranger again.
                    swept += usize::from(termed.remove(&pid) | killed.remove(&pid));
                } else {
                    unreaped = true;
                }
            }
            let now = Instant::now();
            if now >= until {
                return Swept {
                    swept,
                    left: live,
                    unlisted: false,
                };
            }
            // Only zombies, all reaped: look again at once for what they handed over.
            if live.is_empty() && !unreaped {
                continue;
            }
            for &pid in &live {
                let signal = if now >= kill_at {
                    killed.insert(pid).then_some(libc::SIGKILL)
                } else {
                    termed.insert(pid).then_some(libc::SIGTERM)
                };
                if let Some(signal) = signal {
                    // SAFETY: two integers. `pid` is this process's own unreaped child (its ppid
                    // is ours, and only this thread reaps it), so it cannot have been recycled.
                    unsafe { libc::kill(pid as i32, signal) };
                }
            }
            std::thread::sleep(POLL);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_stat_line_is_read_after_the_last_parenthesis() {
            let line = "4242 (a) b (c)) S 17 4242 4242 0 -1 4194304 1 0 0 0 0 0 0 0 20 0 1 0 \
                        987654 1 2 3";
            assert_eq!(
                parse(line),
                Some(Stat {
                    dead: false,
                    ppid: 17,
                    start: 987654
                })
            );
            let zombie = line.replace(") S ", ") Z ");
            assert!(parse(&zombie).unwrap().dead);
            assert_eq!(parse("4242 (truncated) S 17"), None);
        }

        #[test]
        fn this_process_reads_as_its_parents_child_and_as_live() {
            let me = stat(std::process::id()).expect("this process's own stat");
            assert!(!me.dead);
            // SAFETY: `getppid` takes nothing and cannot fail.
            assert_eq!(me.ppid, unsafe { libc::getppid() } as u32);
        }

        #[test]
        fn a_noted_daemon_is_matched_by_pid_and_start_tick() {
            DAEMONS.lock().unwrap().push((u32::MAX - 1, Some(7)));
            DAEMONS.lock().unwrap().push((u32::MAX - 2, None));
            assert!(is_daemon(u32::MAX - 1, 7));
            assert!(
                !is_daemon(u32::MAX - 1, 8),
                "a recycled pid is not the daemon"
            );
            assert!(
                is_daemon(u32::MAX - 2, 123),
                "no tick read: the pid alone excludes it"
            );
            assert!(!is_daemon(u32::MAX - 3, 7));
        }
    }
}
