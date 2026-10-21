//! The agent's pause on a nearly full disk (D65), the one exception to "the tracer records and
//! never manages the agent".
//!
//! Below [`DiskPolicy::pause_below`] free beyond the reservation the watch tick freezes the
//! harness (`SIGSTOP`), so that nothing it does goes unrecorded once the disk fills; above
//! [`DiskPolicy::resume_above`] it continues it (`SIGCONT`) by itself. It never prompts and never
//! kills. Before anything ends the harness (the run's deadline, a newer run replacing it, the
//! harness's exit, this command's end) the pause is ended: what is frozen is continued, because a
//! stopped process does not act on the daemon's termination signal, and nothing is paused again
//! ([`AgentPause::end`]). This file only ever sends `SIGSTOP` and `SIGCONT`; it ends nothing,
//! and `goalcup_daemon::terminate` stays the only terminator of a recorded run.
//!
//! **Which processes.** The ones the termination code reaches for the registered handle: the root
//! this command spawned and every descendant of it in one `ps` walk
//! ([`goalcup_protocol::process::descendants`]), walked twice so that a child forked during the
//! first pass is stopped too, and the handle's process group when the launch owns one. Best
//! effort: a process that left the tree (detached, daemonized, or reparented to this command by
//! the Linux subreaper) is not paused.

use super::reserve::DiskPolicy;
use goalcup_protocol::process::ProcessHandle;
use std::sync::{Arc, Mutex, PoisonError};

/// How the pause reaches the harness's processes: [`Tree`] in a run, a fake in a test (a test
/// must never stop its own runner).
pub trait Freeze: Send {
    /// Send `sig` (`SIGSTOP` or `SIGCONT`) to the harness's processes, best effort.
    fn send(&mut self, sig: i32);
}

/// What one reading changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Paused,
    Resumed,
}

struct State {
    freeze: Box<dyn Freeze>,
    paused: bool,
    ended: bool,
}

/// The run's pause, shared by the watch tick (which pauses and resumes) and the host (which ends
/// it before the harness is ended).
#[derive(Clone)]
pub struct AgentPause(Arc<Mutex<State>>);

impl AgentPause {
    pub fn new(freeze: impl Freeze + 'static) -> AgentPause {
        AgentPause(Arc::new(Mutex::new(State {
            freeze: Box::new(freeze),
            paused: false,
            ended: false,
        })))
    }

    /// The pause of the harness `handle` names: see the module note.
    pub fn of_harness(handle: &ProcessHandle) -> AgentPause {
        AgentPause::new(Tree {
            root: handle.pid,
            pgid: handle.pgid,
            stopped: Vec::new(),
        })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether the agent is paused now. A stop of the harness seen while this is true is the
    /// pause's own, not the harness suspending itself.
    pub fn is_paused(&self) -> bool {
        self.state().paused
    }

    /// One reading of the free bytes beyond the reservation: pause below `pause_below`, resume
    /// above `resume_above`. Nothing once the pause has ended. The flag is set before the
    /// `SIGSTOP` goes, so a wait that sees the stop already reads it as the pause's.
    pub fn follow(&self, free: u64, policy: &DiskPolicy) -> Option<Change> {
        let mut state = self.state();
        if state.ended {
            return None;
        }
        if !state.paused && free < policy.pause_below {
            state.paused = true;
            state.freeze.send(libc::SIGSTOP);
            Some(Change::Paused)
        } else if state.paused && free > policy.resume_above {
            state.freeze.send(libc::SIGCONT);
            state.paused = false;
            Some(Change::Resumed)
        } else {
            None
        }
    }

    /// Continue whatever is paused, and never pause again: called before the harness is ended
    /// and when the host is done with it. Answers whether the agent was paused.
    pub fn end(&self) -> bool {
        let mut state = self.state();
        state.ended = true;
        let was = std::mem::replace(&mut state.paused, false);
        if was {
            state.freeze.send(libc::SIGCONT);
        }
        was
    }

    /// A guard that ends the pause when it is dropped, on every way out of the host's wait (an
    /// unwinding panic included).
    pub fn end_on_drop(&self) -> EndOnDrop {
        EndOnDrop(self.clone())
    }
}

/// See [`AgentPause::end_on_drop`].
pub struct EndOnDrop(AgentPause);

impl Drop for EndOnDrop {
    fn drop(&mut self) {
        self.0.end();
    }
}

/// The live harness tree: see the module note.
pub struct Tree {
    root: u32,
    pgid: Option<u32>,
    /// Every pid the last `SIGSTOP` reached, continued with the tree walked again.
    stopped: Vec<u32>,
}

impl Tree {
    /// The root and its descendants now; the root alone when `ps` cannot be read.
    fn walk(&self) -> Vec<u32> {
        goalcup_protocol::process::snapshot()
            .map(|table| goalcup_protocol::process::descendants(self.root, &table))
            .unwrap_or_else(|_| vec![self.root])
    }
}

impl Freeze for Tree {
    fn send(&mut self, sig: i32) {
        let me = std::process::id();
        let mut reached = std::mem::take(&mut self.stopped);
        // Twice for a stop: a child forked during the first pass is in the second walk.
        let passes = if sig == libc::SIGSTOP { 2 } else { 1 };
        for _ in 0..passes {
            for pid in self.walk() {
                if !reached.contains(&pid) {
                    reached.push(pid);
                }
            }
            for &pid in &reached {
                if pid > 1 && pid != me {
                    // SAFETY: two integers. The root is this process's own unreaped child; the
                    // rest are its descendants in the walk just taken.
                    unsafe { libc::kill(pid as i32, sig) };
                }
            }
            // A group of the harness's own (never this process's), led by the root.
            // SAFETY: integers only.
            if let Some(pgid) = self.pgid
                && pgid as i32 != unsafe { libc::getpgrp() }
            {
                unsafe { libc::killpg(pgid as i32, sig) };
            }
        }
        if sig == libc::SIGSTOP {
            self.stopped = reached;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    /// A fake harness: the signals it was sent.
    #[derive(Clone, Default)]
    struct Sent(Arc<Mutex<Vec<i32>>>);

    impl Freeze for Sent {
        fn send(&mut self, sig: i32) {
            self.0.lock().unwrap().push(sig);
        }
    }

    impl Sent {
        fn take(&self) -> Vec<i32> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }

    fn paused_at(free: u64) -> (AgentPause, Sent) {
        let sent = Sent::default();
        let pause = AgentPause::new(sent.clone());
        assert_eq!(
            pause.follow(free, &DiskPolicy::default()),
            Some(Change::Paused)
        );
        assert_eq!(sent.take(), [libc::SIGSTOP]);
        (pause, sent)
    }

    #[test]
    fn the_agent_is_paused_below_one_gib_beyond_the_reservation_once() {
        let policy = DiskPolicy::default();
        let sent = Sent::default();
        let pause = AgentPause::new(sent.clone());
        for free in [10 * GIB, 2 * GIB, GIB] {
            assert_eq!(pause.follow(free, &policy), None, "{free}");
        }
        assert!(sent.take().is_empty() && !pause.is_paused());
        assert_eq!(pause.follow(GIB - 1, &policy), Some(Change::Paused));
        assert!(pause.is_paused());
        // Still full: not stopped again.
        assert_eq!(pause.follow(0, &policy), None);
        assert_eq!(sent.take(), [libc::SIGSTOP]);
    }

    #[test]
    fn a_paused_agent_resumes_by_itself_only_above_three_gib() {
        let policy = DiskPolicy::default();
        let (pause, sent) = paused_at(0);
        for free in [GIB, 2 * GIB, 3 * GIB] {
            assert_eq!(pause.follow(free, &policy), None, "{free}");
        }
        assert!(pause.is_paused() && sent.take().is_empty());
        assert_eq!(pause.follow(3 * GIB + 1, &policy), Some(Change::Resumed));
        assert!(!pause.is_paused());
        assert_eq!(sent.take(), [libc::SIGCONT]);
        // And it is paused again if the disk fills again.
        assert_eq!(pause.follow(0, &policy), Some(Change::Paused));
    }

    #[test]
    fn a_paused_agent_is_continued_before_it_is_ended_and_never_paused_again() {
        let (pause, sent) = paused_at(0);
        // The run's deadline, a newer run, the harness's exit or this command's end.
        drop(pause.end_on_drop());
        assert_eq!(sent.take(), [libc::SIGCONT]);
        assert!(!pause.is_paused());
        assert_eq!(pause.follow(0, &DiskPolicy::default()), None);
        assert!(!pause.end(), "a second end has nothing to continue");
        assert!(sent.take().is_empty());
        // An agent that was never paused is sent nothing.
        let sent = Sent::default();
        let pause = AgentPause::new(sent.clone());
        assert!(!pause.end());
        assert!(sent.take().is_empty());
    }

    /// The live tree: the root and the child it forked are both frozen and both continued. The
    /// root is this test's own child, never the test runner.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_harness_tree_is_frozen_and_continued_whole() {
        let mut root = std::process::Command::new("/bin/sh")
            .args(["-c", "sleep 5 & wait"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let state = |pid: u32| {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            stat.rsplit_once(") ")
                .and_then(|(_, rest)| rest.chars().next())
        };
        let tree = |tree: &Tree| tree.walk();
        let mut freeze = Tree {
            root: root.id(),
            pgid: None,
            stopped: Vec::new(),
        };
        let began = std::time::Instant::now();
        while tree(&freeze).len() < 2 && began.elapsed() < std::time::Duration::from_secs(5) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let pids = tree(&freeze);
        assert_eq!(pids.len(), 2, "{pids:?}");
        freeze.send(libc::SIGSTOP);
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(pids.iter().all(|&pid| state(pid) == Some('T')), "{pids:?}");
        freeze.send(libc::SIGCONT);
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(pids.iter().all(|&pid| state(pid) != Some('T')), "{pids:?}");
        let _ = root.kill();
        let _ = root.wait();
    }

    #[test]
    fn a_policy_without_a_pause_line_never_pauses() {
        let policy = DiskPolicy {
            pause_below: 0,
            resume_above: 0,
            ..DiskPolicy::default()
        };
        let sent = Sent::default();
        let pause = AgentPause::new(sent.clone());
        assert_eq!(pause.follow(0, &policy), None);
        assert!(sent.take().is_empty());
    }
}
