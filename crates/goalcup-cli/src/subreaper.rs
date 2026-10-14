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
//! named by pid on stderr, and `goalcup start` exits 1 after the seal ([`Swept::refusal`]).
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
//! The daemon is unchanged and the trace and seal gain nothing: what was swept is printed on
//! stdout, what may still be running on stderr.
//!
//! **Where nothing is swept.** If this process dies, what it adopted goes to init as before. The
//! sweep runs only after the root's own `wait` in `run_harness`, so the paths that end a released
//! harness some other way never sweep: the sealed-launch ending (`Forked::sealed_launch`) and the
//! post-release `Forked::unlaunched` paths (a tee that could not be started). What the harness
//! left behind there stays this process's child until it exits, then goes to init.
//!
//! **A seal from another terminal.** A `goalcup seal` while the harness runs has the daemon end the
//! tree; members that outlive the root reparent here and this sweep signals them too. The two
//! overlap safely: both escalate on the handle's `grace_seconds`, this sweep's clock starts no
//! earlier than the daemon's first SIGTERM, the daemon signals through pidfds, and this sweep only
//! signals its own unreaped children. The daemon's report can say `all_gone` while this sweep is
//! still ending an orphan the daemon's walk could not see.
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
    /// Still alive when the sweep's bound ran out; or, when `/proc` failed, signalled and not
    /// yet seen to end.
    pub left: Vec<u32>,
    /// `/proc` (or the root's entry in it) could not be read, so the sweep stopped there.
    pub unlisted: bool,
}

impl Swept {
    /// The stdout line: what was swept, when anything was.
    pub fn swept_line(&self) -> Option<String> {
        (self.swept > 0).then(|| {
            format!(
                "swept {} process(es) the harness left behind after it exited.",
                self.swept
            )
        })
    }

    /// The stderr lines: what may still be running, or why that cannot be said.
    pub fn warnings(&self) -> Vec<String> {
        let pids = || {
            let pids: Vec<String> = self.left.iter().map(u32::to_string).collect();
            pids.join(", ")
        };
        if self.unlisted {
            let signalled = if self.left.is_empty() {
                String::new()
            } else {
                format!(
                    "; {} process(es) it had signalled were not seen to end (pids: {})",
                    self.left.len(),
                    pids()
                )
            };
            vec![format!(
                "goalcup: this command could not read its own children from /proc, so it cannot \
                 say that nothing the harness left behind is still running{signalled}."
            )]
        } else if !self.left.is_empty() {
            // Some of these may have been first seen in the last pass, and never signalled.
            vec![format!(
                "goalcup: {} process(es) the harness left behind were still running when the \
                 sweep stopped at its bound, the grace plus {} s (pids: {}); end them by hand.",
                self.left.len(),
                SWEEP_SLACK.as_secs(),
                pids()
            )]
        } else {
            Vec::new()
        }
    }

    /// Whether the run must not exit 0: something was still running at the bound, or `/proc`
    /// failed after the sweep had signalled something.
    pub fn still_running(&self) -> bool {
        !self.left.is_empty() || (self.unlisted && self.swept > 0)
    }

    /// The refusal `goalcup start` ends with, after the seal, when [`Swept::still_running`]:
    /// exit 1, the participant's to act on, as a key scan that failed after the seal is.
    pub fn refusal(&self, entry_id: &str) -> Option<crate::CliError> {
        self.still_running().then(|| {
            crate::CliError::Refused(format!(
                "entry {entry_id} is sealed, but a process the harness left behind may still be \
                 running (the line above names it); end it by hand."
            ))
        })
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
                // What was signalled and not yet reaped is not known to be gone: named, not dropped.
                return Swept {
                    swept,
                    left: termed.union(&killed).copied().collect(),
                    unlisted: true,
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
            // Alive, or a zombie this pass could not reap: a thread-group leader that exited while
            // its other threads run is `Z` and still running, and is signalled and named like any.
            let mut live = Vec::new();
            for (pid, dead) in found {
                if dead && reap(pid) {
                    // Forgotten once reaped, so a recycled pid is a stranger again.
                    swept += usize::from(termed.remove(&pid) | killed.remove(&pid));
                } else {
                    live.push(pid);
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
            if live.is_empty() {
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

        /// Set only in the process [`the_bound_reports_what_keeps_arriving`] starts.
        const HOST: &str = "GOALCUP_SUBREAPER_SWEEP_HOST";
        /// The host's one result line.
        const RESULT: &str = "subreaper-sweep-host:";

        /// The bound and the straggler report, against a tree the sweep cannot finish. The sweep
        /// runs in a process of its own (this test binary, re-run on [`sweep_host`] alone), so
        /// this test runner never becomes a subreaper and no other test's child is ever swept.
        #[test]
        fn the_bound_reports_what_keeps_arriving() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "subreaper::linux::tests::sweep_host",
                    "--exact",
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(HOST, "1")
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&output.stdout);
            let line = text
                .lines()
                // libtest prints the test's name on the same line, first.
                .find_map(|line| Some(line.split_once(RESULT)?.1))
                .unwrap_or_else(|| {
                    panic!(
                        "no result\n{text}{}",
                        String::from_utf8_lossy(&output.stderr)
                    )
                });
            let field = |name: &str| -> u64 {
                line.split_whitespace()
                    .find_map(|kv| kv.strip_prefix(name)?.strip_prefix('='))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| panic!("no {name} in {line}"))
            };
            let bound = (HOST_GRACE + SWEEP_SLACK).as_millis() as u64;
            assert!(
                field("took_ms") >= bound,
                "stopped before its bound: {line}"
            );
            assert!(
                field("took_ms") <= bound + 2_000,
                "overran its bound: {line}"
            );
            assert!(field("left") >= 1, "no straggler reported: {line}");
            assert!(field("swept") >= 1, "nothing was swept: {line}");
            assert_eq!(field("unlisted"), 0, "{line}");
            assert_eq!(
                field("spawner_alive"),
                1,
                "a child older than the root was signalled: {line}"
            );
            assert_eq!(field("cleanup_left"), 0, "{line}");
            assert_eq!(field("exit"), 1, "a straggler at the bound exits 1: {line}");
        }

        /// The grace the host sweeps with: one second of SIGTERM that the chain ignores.
        const HOST_GRACE: Duration = Duration::from_secs(1);

        /// The chain's length. Past the grace a pass consumes at most one link and then sleeps a
        /// [`POLL`], so the slack holds at most 250 such passes; 50 more are the margin.
        const LINKS: u128 = SWEEP_SLACK.as_millis() / POLL.as_millis() + 50;

        /// A child older than the root, SIGKILLed and reaped however the host ends.
        struct Elder(std::process::Child);

        impl Drop for Elder {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        /// The chain's head, whose process group every link shares: the group is SIGKILLed
        /// whole and the head reaped however the host ends. A head the sweep already reaped
        /// makes the `wait` fail with ECHILD, which is ignored.
        struct Chain(std::process::Child);

        impl Drop for Chain {
            fn drop(&mut self) {
                // SAFETY: two integers. The group is the chain's own, made at its head's spawn;
                // with every member gone the call fails with ESRCH and does nothing.
                unsafe { libc::kill(-(self.0.id() as i32), libc::SIGKILL) };
                let _ = self.0.wait();
            }
        }

        /// The sweep's host, run only by [`the_bound_reports_what_keeps_arriving`]. A plain
        /// `sleep` is started before the root, so the start filter must keep the sweep off it:
        /// any signal ends it. After the root, the host starts a chain of [`LINKS`] processes,
        /// each the parent of the next, that ignore SIGTERM, and waits until the last is up.
        /// The sweep reaches only the chain's head, its one child. SIGKILL ends the head, and the
        /// kernel hands the head's child to this process before the head turns zombie, so every
        /// pass finds the next link: the sweep can never find this process childless, and must
        /// stop at its bound and name what it last saw.
        /// The chain replaced a loop that handed over four orphans every few milliseconds. On a
        /// loaded 4-vCPU x86_64 runner that loop fell behind the 20 ms poll, the sweep found the
        /// host childless and ended early (2 runs in 4); a chain does not depend on any timing.
        #[test]
        #[ignore = "the sweep's host process; run by the_bound_reports_what_keeps_arriving"]
        fn sweep_host() {
            use std::os::unix::process::CommandExt;

            if std::env::var_os(HOST).is_none() {
                return;
            }
            crate::subreaper::arm().expect("this process becomes a child subreaper");
            let mut elder = Elder(
                std::process::Command::new("/bin/sh")
                    .args(["-c", "exec sleep 60"])
                    .spawn()
                    .unwrap(),
            );
            // At least one clock tick (10 ms) after the elder, so it is older than the root.
            std::thread::sleep(Duration::from_millis(50));
            let mut root = std::process::Command::new("/bin/sh")
                .args(["-c", "exit 0"])
                .spawn()
                .unwrap();
            let root_start = stat(root.id()).expect("the root's stat").start;
            root.wait().unwrap();
            // Each link starts the next in the background and becomes a `sleep`; the last one
            // creates `$READY` first. The ignored SIGTERM is inherited through every fork and
            // exec, and the links all share the head's process group.
            let ready = std::env::temp_dir()
                .join(format!("goalcup-subreaper-chain-{}", std::process::id()));
            let chain = Chain(
                std::process::Command::new("/bin/sh")
                    .args(["-c", "trap '' TERM; exec sh -c \"$LINK\" link \"$LINKS\""])
                    .env(
                        "LINK",
                        "if [ \"$1\" -gt 0 ]; then sh -c \"$LINK\" link $(($1 - 1)) & \
                         else : > \"$READY\"; fi; exec sleep 60",
                    )
                    .env("LINKS", LINKS.to_string())
                    .env("READY", &ready)
                    .process_group(0)
                    .spawn()
                    .unwrap(),
            );
            let up_by = Instant::now() + Duration::from_secs(20);
            while !ready.exists() {
                assert!(
                    Instant::now() < up_by,
                    "the chain's last link never came up"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = std::fs::remove_file(&ready);
            let began = Instant::now();
            let swept = sweep(root_start, HOST_GRACE);
            let took = began.elapsed();
            let spawner_alive = elder.0.try_wait().unwrap().is_none();
            drop(elder);
            drop(chain);
            // Everything this process still holds, the chain's last links included.
            let cleanup = sweep(0, Duration::ZERO);
            println!(
                "{RESULT} took_ms={} swept={} left={} unlisted={} spawner_alive={} cleanup_left={} \
                 exit={}",
                took.as_millis(),
                swept.swept,
                swept.left.len(),
                u8::from(swept.unlisted),
                u8::from(spawner_alive),
                cleanup.left.len(),
                swept.refusal("e").map_or(0, |e| e.exit_code()),
            );
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

#[cfg(test)]
mod tests {
    use super::Swept;

    #[test]
    fn a_process_still_running_after_the_sweep_makes_start_exit_1() {
        let exit = |swept: Swept| swept.refusal("e1").map(|e| e.exit_code());
        assert_eq!(exit(Swept::default()), None, "nothing to sweep");
        let clean = Swept {
            swept: 3,
            ..Swept::default()
        };
        assert_eq!(exit(clean), None, "everything swept");
        let straggler = Swept {
            swept: 3,
            left: vec![42],
            unlisted: false,
        };
        assert!(straggler.warnings()[0].contains("pids: 42"));
        assert_eq!(exit(straggler), Some(1));
        let unread_after_signals = Swept {
            swept: 1,
            left: Vec::new(),
            unlisted: true,
        };
        assert_eq!(exit(unread_after_signals), Some(1));
        let unread_signalled = Swept {
            swept: 0,
            left: vec![7],
            unlisted: true,
        };
        assert!(unread_signalled.warnings()[0].contains("not seen to end (pids: 7)"));
        assert_eq!(exit(unread_signalled), Some(1));
        let unread_before_any_signal = Swept {
            unlisted: true,
            ..Swept::default()
        };
        assert_eq!(unread_before_any_signal.warnings().len(), 1);
        assert_eq!(exit(unread_before_any_signal), None);
    }
}
