//! `terminate_with` against a simulated process table.
//!
//! `tests/full/process_kill.rs` runs the escalation against real `/bin/sh` trees, and it is the
//! right test for the walk, the signals and the verification. It cannot test two things:
//!
//! 1. A `/bin/ps` that cannot be spawned. Every reader spawns it by absolute path, so no
//!    fixture, `PATH` or shell trick can make the spawn fail. The only way to see what the
//!    module does when it *cannot ask* is a [`ProcessOs`] whose `ps` fails on command.
//! 2. Ground truth. A real test can only assert over what `ps` showed it, and a process
//!    that forks and reparents between two snapshots is invisible to every snapshot. The
//!    world here records `born` — every pid it ever created under the root, transitively,
//!    whether or not any snapshot ever listed it — and the assertions quantify over that.
//!    Phrased over authenticated snapshots they would miss a process that forks and
//!    reparents between two snapshots entirely, because it is never in one.
//!
//! The property runner quantifies the invariants of `docs/TERMINATION.md` over generated
//! trees. `GOALCUP_SIM_CASES` and `GOALCUP_SIM_SEED` override the defaults; a failure prints
//! the seed that replays it alone.
use goalcup_daemon::terminate::{
    BoundTarget, ForkWitness, InstanceAuthority, ProcessOs, SignalOutcome, TerminationReport,
    terminate_with,
};
use goalcup_protocol::manifest::KillStrategy;
use goalcup_protocol::process::{ProcInfo, ProcessError, ProcessHandle};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// The registered root in every scenario.
const ROOT: u32 = 100;
/// The module's own ceiling; the sim never approaches it.
const WALK_CAP: usize = 4_096;
/// The poll interval the module sleeps between sweeps, so a scenario can count in polls.
const POLL: Duration = Duration::from_millis(50);

// ─── The world ──────────────────────────────────────────────────────────────────────────

/// Which OS the seam pretends to be. The difference is what the seam hands out: macOS has
/// a fork witness and an instance authority and no pidfd; Linux has a pidfd and neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    Macos,
    Linux,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Alive,
    /// Exited, not yet reaped by a live parent: `ps` lists it with `Z`.
    Zombie,
    Gone,
}

/// What a process does when SIGTERM reaches it. SIGKILL is never negotiable.
#[derive(Clone, Debug)]
enum OnTerm {
    /// Exits `n` polls later; `0` is "before the next `ps`".
    DiesAfterPolls(u32),
    Ignores,
    /// Forks `children` after `fork_after` polls and exits after `exit_after` polls.
    ForksThenExits {
        children: Vec<Spawn>,
        fork_after: u32,
        exit_after: u32,
    },
    /// Forks `children` after `after_polls` polls and keeps running.
    SpawnsAndLives {
        children: Vec<Spawn>,
        after_polls: u32,
    },
}

/// A process to create: by a `Fork` event, by a parent's SIGTERM reaction, or at build.
#[derive(Clone, Debug)]
struct Spawn {
    pid: u32,
    comm: &'static str,
    /// `setpgid(0, 0)` at birth; otherwise the parent's group.
    own_group: bool,
    on_term: OnTerm,
}

#[derive(Clone, Debug)]
enum Reaction {
    Exit,
    Fork(Vec<Spawn>),
}

#[derive(Clone, Debug)]
struct SimProc {
    ppid: u32,
    pgid: u32,
    comm: String,
    /// The kernel process object: what a Linux pidfd is bound to and what
    /// `process_start_identity` (a creation timestamp on both platforms) names. Fresh per
    /// fork, unchanged by exec, never reused.
    creation: u64,
    /// The instance version: fresh per fork and per exec. It is what a Darwin audit token
    /// validates and what `NOTE_EXEC` on the witness reports, so an exec goes `Stale` on
    /// the token while the pidfd and the creation identity still name the same process.
    start_id: u64,
    state: State,
    on_term: OnTerm,
    /// Scheduled SIGTERM reactions: `(fire at poll, what)`.
    pending: Vec<(u32, Reaction)>,
    /// `on_term` describes the reaction to the first SIGTERM. A later one — the `killpg`
    /// that follows an individual kill, or a SIGKILL sweep's re-adoption — changes nothing,
    /// and in particular never re-runs a fork whose pid the script allocated exactly once.
    term_seen: bool,
    born_at_poll: u32,
}

/// Per-instance history, kept after the number is recycled so a witness attached to the
/// old instance still answers for the old instance.
#[derive(Clone, Debug, Default)]
struct InstanceLog {
    pid: u32,
    exited: bool,
    exit_seq: Option<u64>,
    exited_at_poll: Option<u32>,
    forks: u32,
    execs: u32,
}

#[derive(Clone, Debug)]
enum Event {
    Fork {
        parent: u32,
        child: Spawn,
    },
    Exit {
        pid: u32,
    },
    Exec {
        pid: u32,
    },
    /// `/bin/ps` cannot be spawned from now until poll `until` (exclusive).
    PsFails {
        until: u32,
    },
}

#[derive(Clone, Debug)]
struct KillRecord {
    pid: u32,
    sig: i32,
    /// Through a pidfd or an audit token, not a bare number.
    bound: bool,
    /// The instance that held the number when the signal was sent, if any.
    instance: Option<u64>,
    at_poll: u32,
}

#[derive(Clone, Debug)]
struct KillpgRecord {
    pgid: u32,
    sig: i32,
    leader_alive: bool,
}

struct SimWorld {
    platform: Platform,
    procs: BTreeMap<u32, SimProc>,
    instances: BTreeMap<u64, InstanceLog>,
    poll: u32,
    elapsed: Duration,
    base: Instant,
    script: Vec<(u32, Event)>,
    ps_broken_until: Option<u32>,
    next_start_id: u64,
    exit_seq: u64,
    /// Ground truth: every pid ever created under the root, transitively, whether or not a
    /// snapshot ever showed it.
    born: Vec<u32>,
    /// The instances behind `born`, so a recycled number is not mistaken for one of ours.
    born_instances: BTreeSet<u64>,
    /// The instances that had already left the root's ppid lineage when `terminate_with` was
    /// called — orphaned by a scripted exit at poll 0 — and everything they fork or exec
    /// afterwards. D30 states the owned set as what the first snapshot could reach; these
    /// are outside it by the contract's own definition, and I1 does not quantify over them.
    /// Nothing that leaves the lineage during the call is ever put here.
    escaped_before_call: BTreeSet<u64>,
    kills: Vec<KillRecord>,
    killpgs: Vec<KillpgRecord>,
    liveness_errs: u32,
    snapshot_errs: u32,
}

/// A tree to build the world from.
#[derive(Clone, Debug)]
struct Tree {
    spawn: Spawn,
    children: Vec<Tree>,
}

fn leaf(pid: u32, comm: &'static str, own_group: bool, on_term: OnTerm) -> Tree {
    Tree {
        spawn: Spawn {
            pid,
            comm,
            own_group,
            on_term,
        },
        children: Vec::new(),
    }
}

fn node(
    pid: u32,
    comm: &'static str,
    own_group: bool,
    on_term: OnTerm,
    children: Vec<Tree>,
) -> Tree {
    Tree {
        spawn: Spawn {
            pid,
            comm,
            own_group,
            on_term,
        },
        children,
    }
}

/// The group the terminal gave the root when it does not own one.
const TERMINAL_PGID: u32 = 50;

impl SimWorld {
    fn new(platform: Platform, root: Tree, script: Vec<(u32, Event)>) -> SimWorld {
        let mut world = SimWorld {
            platform,
            procs: BTreeMap::new(),
            instances: BTreeMap::new(),
            poll: 0,
            elapsed: Duration::ZERO,
            base: Instant::now(),
            script,
            ps_broken_until: None,
            next_start_id: 1,
            exit_seq: 0,
            born: Vec::new(),
            born_instances: BTreeSet::new(),
            escaped_before_call: BTreeSet::new(),
            kills: Vec::new(),
            killpgs: Vec::new(),
            liveness_errs: 0,
            snapshot_errs: 0,
        };
        // Strangers: init and the participant's shell, which leads the terminal group.
        world.insert(1, 0, 1, "launchd", false);
        world.insert(TERMINAL_PGID, 1, TERMINAL_PGID, "zsh", false);
        assert_eq!(root.spawn.pid, ROOT, "every scenario registers pid {ROOT}");
        let pgid = if root.spawn.own_group {
            ROOT
        } else {
            TERMINAL_PGID
        };
        let id = world.insert(ROOT, TERMINAL_PGID, pgid, root.spawn.comm, true);
        world.procs.get_mut(&ROOT).expect("root").on_term = root.spawn.on_term.clone();
        world.procs.get_mut(&ROOT).expect("root").ppid = 1;
        let _ = id;
        for child in &root.children {
            world.build(ROOT, child);
        }
        world.fire_script();
        world.mark_escaped_before_call();
        world
    }

    /// What the first snapshot cannot reach: every lineage instance that is not, at poll 0,
    /// a ppid-descendant of the root over the table `ps` would print (zombies included, as
    /// `ps` includes them). Called exactly once, before the call under test begins.
    fn mark_escaped_before_call(&mut self) {
        let mut reachable = BTreeSet::from([ROOT]);
        let mut frontier = vec![ROOT];
        while let Some(parent) = frontier.pop() {
            for (pid, p) in &self.procs {
                if p.state != State::Gone && p.ppid == parent && reachable.insert(*pid) {
                    frontier.push(*pid);
                }
            }
        }
        for (pid, p) in &self.procs {
            if p.state != State::Gone
                && self.born_instances.contains(&p.start_id)
                && !reachable.contains(pid)
            {
                self.escaped_before_call.insert(p.start_id);
            }
        }
    }

    fn build(&mut self, parent: u32, tree: &Tree) {
        self.fork(parent, &tree.spawn);
        for child in &tree.children {
            self.build(tree.spawn.pid, child);
        }
    }

    fn insert(&mut self, pid: u32, ppid: u32, pgid: u32, comm: &str, lineage: bool) -> u64 {
        let start_id = self.next_start_id;
        self.next_start_id += 1;
        if let Some(existing) = self.procs.get(&pid) {
            assert_eq!(
                existing.state,
                State::Gone,
                "sim script bug: pid {pid} is still in the table"
            );
        }
        self.procs.insert(
            pid,
            SimProc {
                ppid,
                pgid,
                comm: comm.to_string(),
                creation: start_id,
                start_id,
                state: State::Alive,
                on_term: OnTerm::Ignores,
                pending: Vec::new(),
                term_seen: false,
                born_at_poll: self.poll,
            },
        );
        self.instances.insert(
            start_id,
            InstanceLog {
                pid,
                ..InstanceLog::default()
            },
        );
        if lineage {
            self.born.push(pid);
            self.born_instances.insert(start_id);
        }
        start_id
    }

    fn in_lineage(&self, pid: u32) -> bool {
        self.procs
            .get(&pid)
            .is_some_and(|p| p.state != State::Gone && self.born_instances.contains(&p.start_id))
    }

    /// `fork(2)` by `parent`. A dead parent forks nothing.
    fn fork(&mut self, parent: u32, child: &Spawn) {
        let Some(p) = self.procs.get(&parent) else {
            return;
        };
        if p.state != State::Alive {
            return;
        }
        let parent_start = p.start_id;
        let pgid = if child.own_group { child.pid } else { p.pgid };
        let lineage = self.born_instances.contains(&parent_start);
        let child_start = self.insert(child.pid, parent, pgid, child.comm, lineage);
        if self.escaped_before_call.contains(&parent_start) {
            self.escaped_before_call.insert(child_start);
        }
        self.procs
            .get_mut(&child.pid)
            .expect("just inserted")
            .on_term = child.on_term.clone();
        self.instances
            .get_mut(&parent_start)
            .expect("parent instance")
            .forks += 1;
    }

    /// `exit(2)`: children reparent to pid 1; a live parent is left a zombie to reap, and
    /// an orphan is reaped by init at once.
    fn exit(&mut self, pid: u32) {
        let Some(p) = self.procs.get(&pid) else {
            return;
        };
        if p.state != State::Alive {
            return;
        }
        let start_id = p.start_id;
        let parent_alive = self
            .procs
            .get(&p.ppid)
            .is_some_and(|pp| pp.state == State::Alive && p.ppid != 1);
        let p = self.procs.get_mut(&pid).expect("checked");
        p.state = if parent_alive {
            State::Zombie
        } else {
            State::Gone
        };
        p.pending.clear();
        for other in self.procs.values_mut() {
            if other.ppid == pid {
                other.ppid = 1;
                // init reaps at once what the dead parent never did.
                if other.state == State::Zombie {
                    other.state = State::Gone;
                }
            }
        }
        self.exit_seq += 1;
        let log = self.instances.get_mut(&start_id).expect("instance");
        log.exited = true;
        log.exit_seq = Some(self.exit_seq);
        log.exited_at_poll = Some(self.poll);
    }

    /// `execve(2)`: the same number and the same kernel process (`creation` is untouched),
    /// a new instance version.
    fn exec(&mut self, pid: u32) {
        let Some(p) = self.procs.get_mut(&pid) else {
            return;
        };
        if p.state != State::Alive {
            return;
        }
        let old = p.start_id;
        let new = self.next_start_id;
        self.next_start_id += 1;
        p.start_id = new;
        self.instances.get_mut(&old).expect("instance").execs += 1;
        self.instances.insert(
            new,
            InstanceLog {
                pid,
                ..InstanceLog::default()
            },
        );
        if self.born_instances.contains(&old) {
            self.born_instances.insert(new);
        }
        if self.escaped_before_call.contains(&old) {
            self.escaped_before_call.insert(new);
        }
    }

    /// The live lineage pids D30 puts outside the owned set: gone from the root's ppid walk
    /// before the call began, or forked below one that was.
    fn escaped_before_call_pids(&self) -> BTreeSet<u32> {
        self.procs
            .iter()
            .filter(|(_, p)| {
                p.state == State::Alive && self.escaped_before_call.contains(&p.start_id)
            })
            .map(|(pid, _)| *pid)
            .collect()
    }

    /// Deliver `sig` to the live process at `pid`, as the kernel would.
    fn deliver(&mut self, pid: u32, sig: i32) {
        let Some(p) = self.procs.get(&pid) else {
            return;
        };
        if p.state != State::Alive {
            return;
        }
        if sig == libc::SIGKILL {
            self.exit(pid);
            return;
        }
        if sig != libc::SIGTERM {
            return;
        }
        if p.term_seen {
            return;
        }
        let at = self.poll;
        let reactions: Vec<(u32, Reaction)> = match &p.on_term {
            OnTerm::DiesAfterPolls(n) => vec![(at + n, Reaction::Exit)],
            OnTerm::Ignores => Vec::new(),
            OnTerm::ForksThenExits {
                children,
                fork_after,
                exit_after,
            } => vec![
                (at + fork_after, Reaction::Fork(children.clone())),
                (at + exit_after, Reaction::Exit),
            ],
            OnTerm::SpawnsAndLives {
                children,
                after_polls,
            } => vec![(at + after_polls, Reaction::Fork(children.clone()))],
        };
        let p = self.procs.get_mut(&pid).expect("checked");
        p.pending = reactions;
        p.term_seen = true;
        self.run_pending();
    }

    fn run_pending(&mut self) {
        loop {
            let mut due: Option<(u32, Reaction)> = None;
            for (pid, p) in self.procs.iter_mut() {
                if p.state != State::Alive {
                    continue;
                }
                if let Some(i) = p.pending.iter().position(|(at, _)| *at <= self.poll) {
                    let (_, reaction) = p.pending.remove(i);
                    due = Some((*pid, reaction));
                    break;
                }
            }
            let Some((pid, reaction)) = due else {
                return;
            };
            match reaction {
                Reaction::Exit => self.exit(pid),
                Reaction::Fork(children) => {
                    for child in &children {
                        self.fork(pid, child);
                    }
                }
            }
        }
    }

    fn fire_script(&mut self) {
        let due: Vec<Event> = {
            let mut due = Vec::new();
            let mut i = 0;
            while i < self.script.len() {
                if self.script[i].0 <= self.poll {
                    due.push(self.script.remove(i).1);
                } else {
                    i += 1;
                }
            }
            due
        };
        for event in due {
            match event {
                Event::Fork { parent, child } => self.fork(parent, &child),
                Event::Exit { pid } => self.exit(pid),
                Event::Exec { pid } => self.exec(pid),
                Event::PsFails { until } => self.ps_broken_until = Some(until),
            }
        }
    }

    /// One poll interval passes: zombies are reaped, the script fires, reactions land.
    fn tick(&mut self, d: Duration) {
        self.poll += 1;
        self.elapsed += d;
        let zombies: Vec<u32> = self
            .procs
            .iter()
            .filter(|(_, p)| p.state == State::Zombie)
            .map(|(pid, _)| *pid)
            .collect();
        for pid in zombies {
            self.procs.get_mut(&pid).expect("zombie").state = State::Gone;
        }
        self.fire_script();
        self.run_pending();
    }

    fn ps_ok(&self) -> bool {
        !matches!(self.ps_broken_until, Some(until) if self.poll < until)
    }

    fn alive(&self) -> BTreeSet<u32> {
        self.procs
            .iter()
            .filter(|(_, p)| p.state == State::Alive)
            .map(|(pid, _)| *pid)
            .collect()
    }

    fn alive_born(&self) -> BTreeSet<u32> {
        self.alive()
            .into_iter()
            .filter(|pid| self.in_lineage(*pid))
            .collect()
    }

    /// I1′'s allowance: pids whose parent forked them and exited within the same poll.
    fn same_poll_orphans(&self) -> BTreeSet<u32> {
        let mut out = BTreeSet::new();
        for (pid, p) in &self.procs {
            if !self.born_instances.contains(&p.start_id) {
                continue;
            }
            let forked_at = p.born_at_poll;
            let parent_exited_same_poll =
                self.instances
                    .values()
                    .filter(|log| log.forks > 0)
                    .any(|log| {
                        // The parent instance that forked this pid is the one whose pid was
                        // this proc's ppid at birth; ppid is rewritten on reparent, so match
                        // by exit poll on any lineage instance that forked.
                        log.exited_at_poll == Some(forked_at)
                            && self.born_instances.contains(&self.instance_id_of(log))
                    });
            if parent_exited_same_poll {
                out.insert(*pid);
            }
        }
        out
    }

    fn instance_id_of(&self, log: &InstanceLog) -> u64 {
        self.instances
            .iter()
            .find(|(_, l)| std::ptr::eq(*l, log))
            .map(|(id, _)| *id)
            .unwrap_or(0)
    }

    fn describe(&self) -> String {
        let mut s = format!(
            "platform={:?} poll={} born={:?} alive={:?}\n",
            self.platform,
            self.poll,
            self.born,
            self.alive()
        );
        for (pid, p) in &self.procs {
            if *pid == 1 || *pid == TERMINAL_PGID {
                continue;
            }
            s.push_str(&format!(
                "  pid {pid} ppid {} pgid {} {} {:?} start {} born@{} on_term {:?}\n",
                p.ppid, p.pgid, p.comm, p.state, p.start_id, p.born_at_poll, p.on_term
            ));
        }
        s.push_str(&format!("  kills: {:?}\n", self.kills));
        s.push_str(&format!("  killpgs: {:?}\n", self.killpgs));
        s.push_str(&format!(
            "  liveness_errs={} snapshot_errs={} escaped_before_call={:?}\n",
            self.liveness_errs,
            self.snapshot_errs,
            self.escaped_before_call_pids()
        ));
        s
    }
}

// ─── The seam over the world ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Sim(Arc<Mutex<SimWorld>>);

impl Sim {
    fn new(platform: Platform, root: Tree, script: Vec<(u32, Event)>) -> Sim {
        Sim(Arc::new(Mutex::new(SimWorld::new(platform, root, script))))
    }

    fn world(&self) -> MutexGuard<'_, SimWorld> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The handle the CLI would have registered for this world's root.
    fn handle(&self, pgid: Option<u32>, native_binary: Option<&str>) -> ProcessHandle {
        let w = self.world();
        let root = w.procs.get(&ROOT).expect("root");
        ProcessHandle {
            pid: ROOT,
            pgid,
            native_pid: None,
            native_binary: native_binary.map(str::to_string),
            native_process_start: None,
            process_start: Some(format!("sim:{}", root.creation)),
            strategy: if pgid.is_some() {
                KillStrategy::ProcessGroup
            } else {
                KillStrategy::NativePid
            },
            grace_seconds: 1,
            launched_at: "2026-09-17T00:00:00Z".to_string(),
            program: format!("/usr/bin/{}", root.comm),
            argv: vec![format!("/usr/bin/{}", root.comm)],
            env_keys: Vec::new(),
        }
    }

    fn record_kill(w: &mut SimWorld, pid: u32, sig: i32, bound: bool) {
        let instance = w
            .procs
            .get(&pid)
            .filter(|p| p.state != State::Gone)
            .map(|p| p.start_id);
        let at_poll = w.poll;
        w.kills.push(KillRecord {
            pid,
            sig,
            bound,
            instance,
            at_poll,
        });
    }
}

impl ProcessOs for Sim {
    fn snapshot(&self) -> Result<Vec<ProcInfo>, ProcessError> {
        let mut w = self.world();
        if !w.ps_ok() {
            w.snapshot_errs += 1;
            return Err(ProcessError::Spawn(
                "/bin/ps: simulated spawn failure".into(),
            ));
        }
        Ok(w.procs
            .iter()
            .filter(|(_, p)| p.state != State::Gone)
            .map(|(pid, p)| ProcInfo {
                pid: *pid,
                ppid: p.ppid,
                comm: p.comm.clone(),
            })
            .collect())
    }

    fn liveness(&self, pid: u32) -> Result<bool, ProcessError> {
        let mut w = self.world();
        if !w.ps_ok() {
            w.liveness_errs += 1;
            return Err(ProcessError::Spawn(
                "/bin/ps: simulated spawn failure".into(),
            ));
        }
        Ok(w.procs.get(&pid).is_some_and(|p| p.state == State::Alive))
    }

    fn identity(&self, pid: u32) -> Option<(u64, String)> {
        let w = self.world();
        if !w.ps_ok() {
            return None;
        }
        let p = w.procs.get(&pid).filter(|p| p.state != State::Gone)?;
        let etime = u64::from(w.poll.saturating_sub(p.born_at_poll)) * POLL.as_secs();
        Some((etime, format!("/usr/bin/{}", p.comm)))
    }

    fn start_identity(&self, pid: u32) -> Option<String> {
        let w = self.world();
        let p = w.procs.get(&pid).filter(|p| p.state != State::Gone)?;
        Some(format!("sim:{}", p.creation))
    }

    fn leads_group(&self, pid: u32) -> bool {
        let w = self.world();
        w.procs
            .get(&pid)
            .is_some_and(|p| p.state != State::Gone && p.pgid == pid)
    }

    fn bind(&self, pid: u32) -> Option<Box<dyn BoundTarget>> {
        let w = self.world();
        if w.platform != Platform::Linux || pid < 2 {
            return None;
        }
        let p = w.procs.get(&pid).filter(|p| p.state != State::Gone)?;
        Some(Box::new(SimBound {
            sim: self.clone(),
            pid,
            creation: p.creation,
        }))
    }

    fn kill(&self, pid: u32, sig: i32) {
        let mut w = self.world();
        Sim::record_kill(&mut w, pid, sig, false);
        if pid < 2 {
            return;
        }
        w.deliver(pid, sig);
    }

    fn killpg(&self, pgid: u32, sig: i32) {
        let mut w = self.world();
        let leader_alive = w
            .procs
            .get(&pgid)
            .is_some_and(|p| p.state == State::Alive && p.pgid == pgid);
        w.killpgs.push(KillpgRecord {
            pgid,
            sig,
            leader_alive,
        });
        if pgid < 2 {
            return;
        }
        let members: Vec<u32> = w
            .procs
            .iter()
            .filter(|(_, p)| p.state == State::Alive && p.pgid == pgid)
            .map(|(pid, _)| *pid)
            .collect();
        for pid in members {
            w.deliver(pid, sig);
        }
    }

    fn fork_witness(&self, pid: u32) -> Option<Box<dyn ForkWitness>> {
        self.attach(pid)
            .map(|witness| Box::new(witness) as Box<dyn ForkWitness>)
    }

    fn instance_authority(&self, pid: u32) -> Option<Box<dyn InstanceAuthority>> {
        self.attach(pid)
            .map(|witness| Box::new(witness) as Box<dyn InstanceAuthority>)
    }

    fn now(&self) -> Instant {
        let w = self.world();
        w.base + w.elapsed
    }

    fn sleep(&self, d: Duration) {
        self.world().tick(d);
    }
}

impl Sim {
    fn attach(&self, pid: u32) -> Option<SimWitness> {
        let w = self.world();
        if w.platform != Platform::Macos || pid < 2 {
            return None;
        }
        let p = w.procs.get(&pid).filter(|p| p.state == State::Alive)?;
        let log = w.instances.get(&p.start_id).expect("instance");
        Some(SimWitness {
            sim: self.clone(),
            pid,
            instance: p.start_id,
            forks0: log.forks,
            execs0: log.execs,
            forks_taken: log.forks,
            execs_taken: log.execs,
        })
    }
}

/// A pidfd: bound to the kernel process object, which an exec does not replace.
struct SimBound {
    sim: Sim,
    pid: u32,
    creation: u64,
}

impl BoundTarget for SimBound {
    fn signal(&mut self, sig: i32) -> bool {
        let mut w = self.sim.world();
        let same = w
            .procs
            .get(&self.pid)
            .is_some_and(|p| p.state != State::Gone && p.creation == self.creation);
        if same {
            Sim::record_kill(&mut w, self.pid, sig, true);
            w.deliver(self.pid, sig);
        } else {
            // ESRCH on the bound instance: nothing is delivered anywhere.
            let at_poll = w.poll;
            w.kills.push(KillRecord {
                pid: self.pid,
                sig,
                bound: true,
                instance: None,
                at_poll,
            });
        }
        true
    }
}

/// An `EVFILT_PROC`-shaped witness on one instance: exit, fork and exec since attachment.
struct SimWitness {
    sim: Sim,
    pid: u32,
    instance: u64,
    forks0: u32,
    execs0: u32,
    /// The counts at the last `take_growth`: one bit per read, as the kernel gives it.
    forks_taken: u32,
    execs_taken: u32,
}

impl ForkWitness for SimWitness {
    fn observed_state(&mut self) -> Option<(bool, bool)> {
        let w = self.sim.world();
        let log = w.instances.get(&self.instance).expect("instance");
        Some((
            log.exited,
            log.forks > self.forks0 || log.execs > self.execs0,
        ))
    }

    fn take_growth(&mut self) -> Option<bool> {
        let w = self.sim.world();
        let log = w.instances.get(&self.instance).expect("instance");
        let grew = log.forks > self.forks_taken || log.execs > self.execs_taken;
        self.forks_taken = log.forks;
        self.execs_taken = log.execs;
        Some(grew)
    }
}

impl InstanceAuthority for SimWitness {
    fn is_alive(&mut self) -> Option<bool> {
        self.observed_state().map(|(exited, _)| !exited)
    }

    fn signal(&mut self, sig: i32) -> Option<SignalOutcome> {
        let mut w = self.sim.world();
        let current = w
            .procs
            .get(&self.pid)
            .is_some_and(|p| p.state == State::Alive && p.start_id == self.instance);
        if !current {
            return Some(SignalOutcome::Stale);
        }
        Sim::record_kill(&mut w, self.pid, sig, true);
        w.deliver(self.pid, sig);
        Some(SignalOutcome::Delivered)
    }
}

// ─── Assertions over ground truth ───────────────────────────────────────────────────────

fn sorted(v: &[u32]) -> Vec<u32> {
    let mut v = v.to_vec();
    v.sort_unstable();
    v.dedup();
    v
}

fn run(sim: &Sim, handle: &ProcessHandle) -> Result<TerminationReport, ProcessError> {
    terminate_with(handle, WALK_CAP, sim)
}

// ─── Green: the sim and the seam agree with process_kill on the ordinary cases ──────────

/// A lone root that honours SIGTERM. On macOS the singleton audit-token path runs and
/// reports exactly what darwin.rs documents; on Linux the generic walk does.
#[test]
fn sanity_singleton_root_dies_on_term() {
    for platform in [Platform::Macos, Platform::Linux] {
        let sim = Sim::new(
            platform,
            leaf(ROOT, "sleep", false, OnTerm::DiesAfterPolls(0)),
            vec![],
        );
        let handle = sim.handle(None, None);
        let report = run(&sim, &handle).expect("a live root terminates");
        let w = sim.world();
        assert!(
            report.observed_gone,
            "{platform:?}: {report:?}\n{}",
            w.describe()
        );
        assert_eq!(report.targeted, vec![ROOT], "{platform:?}");
        assert!(report.still_alive.is_empty(), "{platform:?}: {report:?}");
        assert!(w.alive_born().is_empty(), "{platform:?}:\n{}", w.describe());
    }
}

/// A three-process tree that dies at its own pace under a root that outlives its children:
/// every member is targeted, the grace runs, and the final sweep finds nothing.
#[test]
fn sanity_tree_dies_under_live_root() {
    for platform in [Platform::Macos, Platform::Linux] {
        let tree = node(
            ROOT,
            "node",
            false,
            OnTerm::DiesAfterPolls(3),
            vec![node(
                101,
                "codex",
                false,
                OnTerm::DiesAfterPolls(1),
                vec![leaf(102, "sh", false, OnTerm::DiesAfterPolls(2))],
            )],
        );
        let sim = Sim::new(platform, tree, vec![]);
        let handle = sim.handle(None, Some("codex"));
        let report = run(&sim, &handle).expect("a live tree terminates");
        let w = sim.world();
        assert!(
            report.observed_gone,
            "{platform:?}: {report:?}\n{}",
            w.describe()
        );
        assert_eq!(sorted(&report.targeted), sorted(&w.born), "{platform:?}");
        assert!(w.alive_born().is_empty(), "{platform:?}:\n{}", w.describe());
        assert!(!report.harness_unverified, "{platform:?}: {report:?}");
    }
}

// ─── Red: each of these sees one confirmed defect ───────────────────────────────────────

/// The root dies at poll 1. A member that was already targeted forks a child at
/// poll 2 and exits at poll 3. The child is a descendant of the registered root in the
/// only sense that matters — it was forked by a process this daemon signalled — and it
/// is alive when the call returns. No snapshot walked after the root is gone lists it, so
/// the report must not say the owned set was observed gone over a live process.
///
/// I1: `observed_gone ⟹ A ∩ B = ∅`. The escapee is in `born` and in no snapshot.
#[test]
fn f0_member_forks_after_root_death() {
    let tree = node(
        ROOT,
        "node",
        false,
        OnTerm::DiesAfterPolls(1),
        vec![leaf(
            101,
            "codex",
            false,
            OnTerm::ForksThenExits {
                children: vec![Spawn {
                    pid: 102,
                    comm: "codex",
                    own_group: false,
                    on_term: OnTerm::Ignores,
                }],
                fork_after: 2,
                exit_after: 3,
            },
        )],
    );
    let sim = Sim::new(Platform::Macos, tree, vec![]);
    let handle = sim.handle(None, Some("codex"));
    let report = run(&sim, &handle).expect("a live tree terminates");
    let w = sim.world();
    let escaped = w.alive_born();
    assert!(
        !report.observed_gone || escaped.is_empty(),
        "F0: observed_gone over a live process forked after the root died.\n\
         escaped (in `born`, alive at return, never in a walked snapshot): {escaped:?}\n\
         report: {report:?}\n{}",
        w.describe()
    );
}

/// `/bin/ps` stops spawning at poll 2, with the whole tree still alive and
/// ignoring SIGTERM. Folding the spawn failure to `false` would read the
/// root as absent and every target as gone, and report
/// `observed_gone: true, harness_unverified: false` over two live processes.
///
/// I4: any liveness `Err` during the call ⟹ `¬observed_gone ∧ harness_unverified`.
#[test]
fn f1_ps_fails_mid_sweep() {
    let tree = node(
        ROOT,
        "node",
        false,
        OnTerm::Ignores,
        vec![leaf(101, "codex", false, OnTerm::Ignores)],
    );
    let sim = Sim::new(
        Platform::Macos,
        tree,
        vec![(2, Event::PsFails { until: 8 })],
    );
    let handle = sim.handle(None, Some("codex"));
    let report = run(&sim, &handle).expect("the call returns a report");
    let w = sim.world();
    assert!(
        w.liveness_errs > 0,
        "the scenario did not reach a failing ps; fix the script\n{}",
        w.describe()
    );
    assert!(
        !report.observed_gone && report.harness_unverified,
        "F1: ps could not be spawned {} times mid-sweep and the report still claims \
         observed_gone={} harness_unverified={} over live pids {:?}\n{report:?}\n{}",
        w.liveness_errs,
        report.observed_gone,
        report.harness_unverified,
        w.alive_born(),
        w.describe()
    );
}

/// `/bin/ps` cannot be spawned from before the first guard. The root is alive,
/// nothing has been signalled, and a guard that read the spawn failure as
/// "absent" would return the ordinary seal — the participant quit hours ago — over a
/// harness that is running. `snapshot()?` returns `Err(Spawn)` for the
/// same failure; the guard has to give the same answer, because the seal maps an `Err`
/// to `RequestFailed` and an absent root to a completed seal.
#[test]
fn f1b_ps_fails_before_first_signal() {
    let tree = node(
        ROOT,
        "node",
        false,
        OnTerm::Ignores,
        vec![leaf(101, "codex", false, OnTerm::Ignores)],
    );
    let sim = Sim::new(
        Platform::Macos,
        tree,
        vec![(0, Event::PsFails { until: u32::MAX })],
    );
    let handle = sim.handle(None, Some("codex"));
    let result = run(&sim, &handle);
    let w = sim.world();
    assert!(
        w.liveness_errs > 0,
        "the scenario did not reach a failing ps; fix the script\n{}",
        w.describe()
    );
    assert!(
        matches!(result, Err(ProcessError::Spawn(_))),
        "F1b: ps could not be spawned before the first signal; the honest answer is the \
         Err(Spawn) that snapshot() gives one line later, not the ordinary absent-root seal \
         over a live root {:?}.\ngot: {result:?}\n{}",
        w.alive_born(),
        w.describe()
    );
}

/// The shipped Codex shape: `node` wrapper → `codex` native → a shell and its
/// child, each in its own process group so no group signal can reach them. Every process
/// honours SIGTERM at once. `discover` hoists the native pid to the front, so the interior
/// `codex` is signalled first, exits, and its children reparent to pid 1 before the walk
/// reaches them; the call must still name and end them rather than bail
/// out unverified with two live processes it never named.
///
/// I6: where every process dies within `k` polls of its first signal and nothing forks,
/// `observed_gone ∧ targeted == born`.
#[test]
fn d1_codex_tree_native_interior_own_groups() {
    let tree = node(
        ROOT,
        "node",
        false,
        OnTerm::DiesAfterPolls(0),
        vec![node(
            101,
            "codex",
            false,
            OnTerm::DiesAfterPolls(0),
            vec![node(
                102,
                "sh",
                true,
                OnTerm::DiesAfterPolls(0),
                vec![leaf(103, "sleep", false, OnTerm::DiesAfterPolls(0))],
            )],
        )],
    );
    let sim = Sim::new(Platform::Macos, tree, vec![]);
    let handle = sim.handle(None, Some("codex"));
    let report = run(&sim, &handle).expect("a live tree terminates");
    let w = sim.world();
    assert!(
        report.observed_gone && sorted(&report.targeted) == sorted(&w.born),
        "D1: every process dies on its first SIGTERM and nothing forks, so the owned set \
         must be observed gone with every member targeted.\n\
         targeted={:?} born={:?} alive={:?}\n{report:?}\n{}",
        report.targeted,
        w.born,
        w.alive_born(),
        w.describe()
    );
    assert!(w.alive_born().is_empty(), "{}", w.describe());
}

/// The root leads its own process group and the handle says so. One member
/// ignores SIGTERM; the leader exits cleanly on it. The individual SIGTERM to the root
/// lands first, the root dies, and the member's own SIGTERM is refused because the root
/// is gone — so the group signal (taken whenever a live registered pid leads one) is what
/// has to reach the member, and the report has to kill it or name it.
///
/// I5 holds trivially here (`¬observed_gone`), which is why it is not the assertion. What
/// the scenario asserts is that a process discovered under the live, authenticated root
/// is either dead at return or in `still_alive`, and that it was signalled at least once.
#[test]
fn f6_group_member_ignores_term_leader_exits_clean() {
    let tree = node(
        ROOT,
        "harness",
        true,
        OnTerm::DiesAfterPolls(0),
        vec![leaf(101, "worker", false, OnTerm::Ignores)],
    );
    let sim = Sim::new(Platform::Macos, tree, vec![]);
    let handle = sim.handle(Some(ROOT), None);
    let report = run(&sim, &handle).expect("a live group terminates");
    let w = sim.world();
    assert!(!report.observed_gone, "I5: {report:?}\n{}", w.describe());
    let unnamed: Vec<u32> = w
        .alive_born()
        .into_iter()
        .filter(|pid| !report.still_alive.contains(pid))
        .collect();
    let member_signalled = w
        .kills
        .iter()
        .any(|k| k.pid == 101 && (k.sig == libc::SIGTERM || k.sig == libc::SIGKILL))
        || w.killpgs.iter().any(|g| g.pgid == ROOT && g.leader_alive);
    assert!(
        unnamed.is_empty() && member_signalled,
        "F6: the member was discovered under the live root and is alive at return; it must \
         be dead or in still_alive, and it must have been signalled.\n\
         alive-and-unnamed={unnamed:?} member_signalled={member_signalled}\n{report:?}\n{}",
        w.describe()
    );
}

// ─── The Claude Code shape ──────────────────────────────────────────────────────────────
//
// The next harness to be qualified. Its manifest declares `[kill] strategy = "process_group"`
// with `mode = "tui"`, and a TUI launch cannot own its process group (SIGTTIN), so the CLI
// registers `pgid: None` and no `native_binary`: every child sits in the inherited terminal
// group, `discover` promotes nothing, and the strategy degrades to the plain descendant walk.
// When `claude` is alone, the macOS singleton audit-token path applies.

/// A lone `claude` with nothing under it: the singleton path on macOS (no `native_binary`,
/// so `direct_native_identity_matches` is vacuously true), the generic walk on Linux.
#[test]
fn claude_shape_singleton_inherited_group() {
    for platform in [Platform::Macos, Platform::Linux] {
        let sim = Sim::new(
            platform,
            leaf(ROOT, "claude", false, OnTerm::DiesAfterPolls(1)),
            vec![],
        );
        let handle = sim.handle(None, None);
        let report = run(&sim, &handle).expect("a live claude terminates");
        let w = sim.world();
        assert!(
            report.observed_gone && report.targeted == vec![ROOT],
            "{platform:?}: {report:?}\n{}",
            w.describe()
        );
        assert!(w.alive_born().is_empty(), "{platform:?}:\n{}", w.describe());
        assert!(report.still_alive.is_empty(), "{platform:?}: {report:?}");
    }
}

/// `claude` running a Bash tool: `claude → sh → sleep`, every child in the inherited
/// group, none of them a native binary. Deepest first signals `sleep` before `sh`; `sh`
/// dies at once and orphans the still-dying `sleep` to pid 1, out of the ppid lineage.
/// The orphan was already named, so the by-number sweep after the root's exit waits for it
/// instead of calling the tree gone over it.
#[test]
fn claude_shape_bash_tool_orphaned_child_dies_late() {
    for platform in [Platform::Macos, Platform::Linux] {
        let tree = node(
            ROOT,
            "claude",
            false,
            OnTerm::DiesAfterPolls(1),
            vec![node(
                101,
                "sh",
                false,
                OnTerm::DiesAfterPolls(0),
                vec![leaf(102, "sleep", false, OnTerm::DiesAfterPolls(3))],
            )],
        );
        let sim = Sim::new(platform, tree, vec![]);
        let handle = sim.handle(None, None);
        let report = run(&sim, &handle).expect("a live claude tree terminates");
        let w = sim.world();
        assert!(
            report.observed_gone && sorted(&report.targeted) == sorted(&w.born),
            "{platform:?}: {report:?}\n{}",
            w.describe()
        );
        assert!(w.alive_born().is_empty(), "{platform:?}:\n{}", w.describe());
        assert!(!report.harness_unverified, "{platform:?}: {report:?}");
    }
}

/// The same Bash-tool shape, but the grandchild ignores SIGTERM — a server
/// the tool started that traps it. `sh` dies on its SIGTERM and orphans the ignorer alive.
///
/// Ignored: its first line, `!observed_gone`, assumes the orphan survives the call; it
/// does not — it is SIGKILLed by its recorded identity, nothing of ours is alive, and
/// `observed_gone` is the correct answer.
/// `claude_shape_bash_tool_orphaned_ignorer_is_killed` below asserts that behaviour.
#[test]
#[ignore = "F7 closed: this scenario's `!observed_gone` line asserts the orphan survives; it is SIGKILLed now — see claude_shape_bash_tool_orphaned_ignorer_is_killed"]
fn claude_shape_bash_tool_orphaned_child_ignores_term() {
    for platform in [Platform::Macos, Platform::Linux] {
        let tree = node(
            ROOT,
            "claude",
            false,
            OnTerm::DiesAfterPolls(1),
            vec![node(
                101,
                "sh",
                false,
                OnTerm::DiesAfterPolls(0),
                vec![leaf(102, "sleep", false, OnTerm::Ignores)],
            )],
        );
        let sim = Sim::new(platform, tree, vec![]);
        let handle = sim.handle(None, None);
        let report = run(&sim, &handle).expect("a live claude tree terminates");
        let w = sim.world();
        assert!(!report.observed_gone, "I1: {platform:?}: {report:?}");
        let unnamed: Vec<u32> = w
            .alive_born()
            .into_iter()
            .filter(|pid| !report.still_alive.contains(pid))
            .collect();
        assert!(
            unnamed.is_empty() && report.killed == vec![102],
            "F7 {platform:?}: the orphan must be SIGKILLed or named.\n\
             alive-and-unnamed={unnamed:?}\n{report:?}\n{}",
            w.describe()
        );
    }
}

/// The orphaned SIGTERM-ignorer is chosen for SIGKILL by its recorded
/// identity — the phase-A gate follows it out of the lineage — and the signal is authorised
/// by that identity too, so it dies, is named in `killed`, and the tree is observed gone.
#[test]
fn claude_shape_bash_tool_orphaned_ignorer_is_killed() {
    for platform in [Platform::Macos, Platform::Linux] {
        let tree = node(
            ROOT,
            "claude",
            false,
            OnTerm::DiesAfterPolls(1),
            vec![node(
                101,
                "sh",
                false,
                OnTerm::DiesAfterPolls(0),
                vec![leaf(102, "sleep", false, OnTerm::Ignores)],
            )],
        );
        let sim = Sim::new(platform, tree, vec![]);
        let handle = sim.handle(None, None);
        let report = run(&sim, &handle).expect("a live claude tree terminates");
        let w = sim.world();
        assert!(
            report.killed == vec![102] && report.still_alive.is_empty() && report.observed_gone,
            "F7 {platform:?}: the orphan must be SIGKILLed by identity and the tree confirmed\n\
             {report:?}\n{}",
            w.describe()
        );
        assert!(w.alive_born().is_empty(), "{platform:?}:\n{}", w.describe());
    }
}

/// `claude` whose Bash tool respawns on shutdown: the shell forks a replacement on its own
/// SIGTERM and then exits, while `claude` is still alive. The adoption rule — the walk runs
/// from every live, identity-verified member, not only the root — is what names the child
/// before its forker is gone.
#[test]
fn claude_shape_bash_tool_forks_on_term() {
    for platform in [Platform::Macos, Platform::Linux] {
        let tree = node(
            ROOT,
            "claude",
            false,
            OnTerm::DiesAfterPolls(3),
            vec![leaf(
                101,
                "sh",
                false,
                OnTerm::ForksThenExits {
                    children: vec![Spawn {
                        pid: 102,
                        comm: "sleep",
                        own_group: false,
                        on_term: OnTerm::DiesAfterPolls(1),
                    }],
                    fork_after: 1,
                    exit_after: 2,
                },
            )],
        );
        let sim = Sim::new(platform, tree, vec![]);
        let handle = sim.handle(None, None);
        let report = run(&sim, &handle).expect("a live claude tree terminates");
        let w = sim.world();
        assert!(
            report.observed_gone && sorted(&report.targeted) == sorted(&w.born),
            "{platform:?}: the forked child must be adopted and named\n{report:?}\n{}",
            w.describe()
        );
        assert!(w.alive_born().is_empty(), "{platform:?}:\n{}", w.describe());
    }
}

// ─── A member orphaned alive ─────────────────────────────────────────────────────────────
//
// Every question about a targeted pid is asked by its recorded identity, and a signal is
// authorised against a live member's ppid tree as well as the root's, under a `Closed`
// anchor too: a member whose parent died first is orphaned to pid 1 while still alive, and
// a walk filtered by the root's lineage alone would stop seeing it.

/// The ordering half. `node → sh → sleep`: `sh` dies one poll after its SIGTERM;
/// `sleep` — signalled first, deepest first — takes longer and is orphaned alive. The root
/// must not be signalled while a member it owns is alive, and the orphan's later fork must
/// be adopted from it or refused over, on both platforms.
#[test]
fn f8_orphaned_member_clears_phase_a_gate() {
    for platform in [Platform::Macos, Platform::Linux] {
        let tree = node(
            ROOT,
            "node",
            false,
            OnTerm::DiesAfterPolls(0),
            vec![node(
                101,
                "sh",
                false,
                OnTerm::DiesAfterPolls(1),
                vec![leaf(
                    102,
                    "sleep",
                    false,
                    OnTerm::ForksThenExits {
                        children: vec![Spawn {
                            pid: 200,
                            comm: "sleep",
                            own_group: false,
                            on_term: OnTerm::Ignores,
                        }],
                        fork_after: 3,
                        exit_after: 4,
                    },
                )],
            )],
        );
        let sim = Sim::new(platform, tree, vec![]);
        let handle = sim.handle(None, None);
        let report = run(&sim, &handle).expect("a live tree terminates");
        let w = sim.world();
        let root_first_signal = w
            .kills
            .iter()
            .find(|k| k.pid == ROOT)
            .map(|k| k.at_poll)
            .expect("the root is signalled");
        let orphan_exit = w
            .instances
            .values()
            .find(|l| l.pid == 102)
            .and_then(|l| l.exited_at_poll)
            .expect("the orphan exits");
        assert!(
            root_first_signal >= orphan_exit,
            "F8 {platform:?}: the root was signalled at poll {root_first_signal} while member \
             102 lived until poll {orphan_exit}\n{report:?}\n{}",
            w.describe()
        );
        assert!(
            !report.observed_gone || w.alive_born().is_empty(),
            "F8 {platform:?}: observed_gone over {:?}\n{report:?}\n{}",
            w.alive_born(),
            w.describe()
        );
    }
}

/// The root exits at poll 1 of its own accord. `codex`, already signalled, forks a
/// replacement at poll 2 and exits at poll 3.
///
/// Ignored: the escapee is adopted from the live `codex` under a `Closed` anchor,
/// SIGKILLed by its own identity, and `observed_gone` is the correct answer with nothing of
/// ours alive, so this scenario's `!observed_gone` line does not hold.
/// `fork_after_root_self_exit_is_adopted_from_the_live_member`
/// asserts that behaviour on both platforms.
#[test]
#[ignore = "F8 closed: this scenario asserted the witness's fail-closed refusal over an escapee that is now adopted and killed — see fork_after_root_self_exit_is_adopted_from_the_live_member"]
fn macos_witness_refuses_fork_after_root_self_exit() {
    let sim = Sim::new(
        Platform::Macos,
        root_self_exit_then_fork_tree(),
        vec![(1, Event::Exit { pid: ROOT })],
    );
    let handle = sim.handle(None, Some("codex"));
    let report = run(&sim, &handle).expect("a live tree terminates");
    let w = sim.world();
    assert!(
        !report.observed_gone,
        "I1: observed_gone over the escapee {:?}\n{report:?}\n{}",
        w.alive_born(),
        w.describe()
    );
}

/// On Linux, with no fork witness. The fork at poll 2 happens while `codex` is
/// alive and identity-verified, so a walk from `codex` names pid 102, even with the anchor
/// `Closed` by the root's own exit; the walk runs and the escapee dies.
#[test]
fn linux_fork_after_root_self_exit_is_confirmed_over() {
    let sim = Sim::new(
        Platform::Linux,
        root_self_exit_then_fork_tree(),
        vec![(1, Event::Exit { pid: ROOT })],
    );
    let handle = sim.handle(None, Some("codex"));
    let report = run(&sim, &handle).expect("a live tree terminates");
    let w = sim.world();
    assert!(
        !report.observed_gone || w.alive_born().is_empty(),
        "F8: observed_gone over the escapee {:?}\n{report:?}\n{}",
        w.alive_born(),
        w.describe()
    );
}

/// The root's own exit at poll 1 closes the anchor; the fork at poll 2 is
/// adopted from the live, identity-verified `codex` all the same, killed by its own identity
/// once `codex` is gone, and the tree is observed gone on both platforms.
#[test]
fn fork_after_root_self_exit_is_adopted_from_the_live_member() {
    for platform in [Platform::Macos, Platform::Linux] {
        let sim = Sim::new(
            platform,
            root_self_exit_then_fork_tree(),
            vec![(1, Event::Exit { pid: ROOT })],
        );
        let handle = sim.handle(None, Some("codex"));
        let report = run(&sim, &handle).expect("a live tree terminates");
        let w = sim.world();
        assert!(
            report.targeted.contains(&102) && report.observed_gone,
            "{platform:?}: the fork after the root's own exit must be adopted from the live \
             member and the tree confirmed\n{report:?}\n{}",
            w.describe()
        );
        assert!(w.alive_born().is_empty(), "{platform:?}:\n{}", w.describe());
    }
}

// ─── The macOS witness, read every poll ──────────────────────────────────────────────────
//
// The witness is read every poll, before the snapshot and after the adoption, and a fork it
// reports is paid only by an adoption from that member in the poll it was read (or the next,
// for the second read). Read once per member as "did it fork at all", a member that forked
// once and had that child adopted would be trusted for every fork after it. The root is
// witnessed from before phase A.

/// The green half: a root whose SIGTERM handler forks a replacement and exits in the same
/// instant, with no earlier adoption. The witness on the root reports the fork, nothing
/// adopted it, and the claim is refused.
#[test]
fn macos_witness_refuses_root_fork_and_exit_on_term() {
    let sim = Sim::new(Platform::Macos, root_respawns_on_term_tree(), vec![]);
    let handle = sim.handle(None, None);
    let report = run(&sim, &handle).expect("a live tree terminates");
    let w = sim.world();
    assert!(
        !report.observed_gone,
        "I1: observed_gone over the respawn {:?}\n{report:?}\n{}",
        w.alive_born(),
        w.describe()
    );
}

/// The same root, but it also forks an ordinary child at poll 1 — which is adopted —
/// before phase B reaches it. The respawn on its SIGTERM is a fork of its own generation
/// that nothing adopts, and the claim is refused.
#[test]
fn f9_macos_witness_trusts_forker_after_one_adoption() {
    let sim = Sim::new(
        Platform::Macos,
        root_respawns_on_term_tree(),
        vec![(
            1,
            Event::Fork {
                parent: ROOT,
                child: Spawn {
                    pid: 201,
                    comm: "sh",
                    own_group: false,
                    on_term: OnTerm::DiesAfterPolls(0),
                },
            },
        )],
    );
    let handle = sim.handle(None, None);
    let report = run(&sim, &handle).expect("a live tree terminates");
    let w = sim.world();
    assert!(
        report.targeted.contains(&201),
        "the scenario needs the poll-1 child adopted; fix the script\n{report:?}\n{}",
        w.describe()
    );
    assert!(
        !report.observed_gone || w.alive_born().is_empty(),
        "F9: observed_gone over the respawn {:?}\n{report:?}\n{}",
        w.alive_born(),
        w.describe()
    );
}

/// The two composed, on macOS: `node → sh → sleep`, where `sleep` forks an ordinary child
/// at poll 1 (adopted from it), is orphaned alive at poll 2 when `sh` dies, and forks its
/// respawn at poll 3 with the anchor closed. A witness that excused that second fork on
/// the strength of the first would report `observed_gone` over the respawn.
#[test]
fn f8_f9_compose_forker_trusted_then_orphaned_alive_on_macos() {
    let tree = node(
        ROOT,
        "node",
        false,
        OnTerm::DiesAfterPolls(0),
        vec![node(
            101,
            "sh",
            false,
            OnTerm::DiesAfterPolls(2),
            vec![leaf(
                102,
                "sleep",
                false,
                OnTerm::ForksThenExits {
                    children: vec![Spawn {
                        pid: 200,
                        comm: "sleep",
                        own_group: false,
                        on_term: OnTerm::Ignores,
                    }],
                    fork_after: 3,
                    exit_after: 4,
                },
            )],
        )],
    );
    let sim = Sim::new(
        Platform::Macos,
        tree,
        vec![(
            1,
            Event::Fork {
                parent: 102,
                child: Spawn {
                    pid: 201,
                    comm: "sleep",
                    own_group: false,
                    on_term: OnTerm::DiesAfterPolls(0),
                },
            },
        )],
    );
    let handle = sim.handle(None, None);
    let report = run(&sim, &handle).expect("a live tree terminates");
    let w = sim.world();
    assert!(
        report.targeted.contains(&201),
        "the scenario needs the poll-1 child adopted from 102; fix the script\n{report:?}\n{}",
        w.describe()
    );
    assert!(
        !report.observed_gone || w.alive_born().is_empty(),
        "F8∘F9: macOS observed_gone over {:?}, forked by a member orphaned alive whose \
         earlier fork was adopted\n{report:?}\n{}",
        w.alive_born(),
        w.describe()
    );
}

/// The root is witnessed during phase A. It forks a child
/// and exits on its own inside one poll of phase A, while a member still ignores SIGTERM: the
/// child is orphaned before any snapshot shows it, and only a witness already on the root can
/// refuse the claim, so the root's witness is attached before phase A.
#[test]
fn macos_witness_refuses_root_fork_and_exit_during_phase_a() {
    let tree = node(
        ROOT,
        "node",
        false,
        OnTerm::DiesAfterPolls(0),
        vec![leaf(101, "node", false, OnTerm::Ignores)],
    );
    let sim = Sim::new(
        Platform::Macos,
        tree,
        vec![
            (
                9,
                Event::Fork {
                    parent: ROOT,
                    child: Spawn {
                        pid: 200,
                        comm: "sleep",
                        own_group: true,
                        on_term: OnTerm::Ignores,
                    },
                },
            ),
            (9, Event::Exit { pid: ROOT }),
        ],
    );
    let handle = sim.handle(None, None);
    let report = run(&sim, &handle).expect("a live tree terminates");
    let w = sim.world();
    assert!(
        report.killed == vec![101],
        "the ignorer must still be SIGKILLed under the closed anchor\n{report:?}\n{}",
        w.describe()
    );
    assert!(
        !report.observed_gone || w.alive_born().is_empty(),
        "I1: observed_gone over the root's unwitnessed same-poll fork {:?}\n{report:?}\n{}",
        w.alive_born(),
        w.describe()
    );
}

/// A root that respawns on SIGTERM — fork and exit in one instant — over one child that
/// keeps phase A open for two polls.
fn root_respawns_on_term_tree() -> Tree {
    node(
        ROOT,
        "codex",
        false,
        OnTerm::ForksThenExits {
            children: vec![Spawn {
                pid: 200,
                comm: "codex",
                own_group: false,
                on_term: OnTerm::Ignores,
            }],
            fork_after: 0,
            exit_after: 0,
        },
        vec![leaf(101, "sleep", false, OnTerm::DiesAfterPolls(2))],
    )
}

fn root_self_exit_then_fork_tree() -> Tree {
    node(
        ROOT,
        "node",
        false,
        OnTerm::Ignores,
        vec![leaf(
            101,
            "codex",
            false,
            OnTerm::ForksThenExits {
                children: vec![Spawn {
                    pid: 102,
                    comm: "codex",
                    own_group: false,
                    on_term: OnTerm::Ignores,
                }],
                fork_after: 2,
                exit_after: 3,
            },
        )],
    )
}

// ─── The root's guard reads race the root's own exit (H-03 cause D) ─────────────────────

/// What the registered root's creation identity answers once the race has been set up.
#[derive(Clone, Copy)]
enum RootRead {
    /// The root exits between the guard's `liveness` read and its `start_identity` read, on
    /// the first such read made once `after_kills` signals have been delivered. The read
    /// then finds nothing, exactly as a read of a process that just exited does.
    ExitsAfterKills { after_kills: usize },
    /// The number is held by a stranger: the read answers a creation identity the handle
    /// never registered, and the process stays live.
    Stranger,
    /// The identity cannot be read although the process is still live (not a recycle, but
    /// not provably an exit either).
    UnreadableWhileLive,
}

/// [`Sim`] with one deterministic interleaving injected into `guard_registered`'s two reads
/// of the registered root. A real `/bin/sh -c '... & wait'` root exits because its child was
/// just signalled, so the scheduler decides which side of the second read it lands on; here
/// the order is fixed.
struct RacingRoot {
    sim: Sim,
    read: RootRead,
    fired: std::sync::atomic::AtomicBool,
}

impl RacingRoot {
    fn new(sim: &Sim, read: RootRead) -> RacingRoot {
        RacingRoot {
            sim: sim.clone(),
            read,
            fired: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl ProcessOs for RacingRoot {
    fn snapshot(&self) -> Result<Vec<ProcInfo>, ProcessError> {
        self.sim.snapshot()
    }
    fn liveness(&self, pid: u32) -> Result<bool, ProcessError> {
        self.sim.liveness(pid)
    }
    fn identity(&self, pid: u32) -> Option<(u64, String)> {
        self.sim.identity(pid)
    }
    fn start_identity(&self, pid: u32) -> Option<String> {
        use std::sync::atomic::Ordering::SeqCst;
        if pid != ROOT {
            return self.sim.start_identity(pid);
        }
        match self.read {
            RootRead::ExitsAfterKills { after_kills } => {
                if !self.fired.load(SeqCst) && self.sim.world().kills.len() >= after_kills {
                    self.fired.store(true, SeqCst);
                    self.sim.world().exit(ROOT);
                }
                self.sim.start_identity(pid)
            }
            RootRead::Stranger => Some("sim:stranger".to_string()),
            RootRead::UnreadableWhileLive => None,
        }
    }
    fn leads_group(&self, pid: u32) -> bool {
        self.sim.leads_group(pid)
    }
    fn bind(&self, pid: u32) -> Option<Box<dyn BoundTarget>> {
        self.sim.bind(pid)
    }
    fn kill(&self, pid: u32, sig: i32) {
        self.sim.kill(pid, sig);
    }
    fn killpg(&self, pgid: u32, sig: i32) {
        self.sim.killpg(pgid, sig);
    }
    fn fork_witness(&self, pid: u32) -> Option<Box<dyn ForkWitness>> {
        self.sim.fork_witness(pid)
    }
    fn instance_authority(&self, pid: u32) -> Option<Box<dyn InstanceAuthority>> {
        self.sim.instance_authority(pid)
    }
    fn now(&self) -> Instant {
        self.sim.now()
    }
    fn sleep(&self, d: Duration) {
        self.sim.sleep(d);
    }
}

/// `sh -c 'worker & wait'`: a root that only exits once its child has been signalled.
fn root_waiting_on_one_worker() -> Tree {
    node(
        ROOT,
        "sh",
        false,
        OnTerm::DiesAfterPolls(0),
        vec![leaf(101, "sleep", false, OnTerm::DiesAfterPolls(1))],
    )
}

/// The root exits after the first signal, between the sweep guard's two reads. That is the
/// kill working, not a recycled number: the sweep must stay anchored on the targeted members
/// and report what it observed, as it does for a root that exits between two sweeps.
#[test]
fn a_root_that_exits_between_the_guards_reads_after_a_signal_is_a_closed_anchor_not_a_loss() {
    for platform in [Platform::Macos, Platform::Linux] {
        let sim = Sim::new(platform, root_waiting_on_one_worker(), vec![]);
        let handle = sim.handle(None, None);
        let os = RacingRoot::new(&sim, RootRead::ExitsAfterKills { after_kills: 1 });
        let report = terminate_with(&handle, WALK_CAP, &os).expect("the chase completes");
        let w = sim.world();
        assert!(
            os.fired.load(std::sync::atomic::Ordering::SeqCst),
            "{platform:?}: the interleaving was never injected\n{}",
            w.describe()
        );
        assert!(
            report.observed_gone && !report.harness_unverified,
            "{platform:?}: a root that exited between the guard's reads, after its child was \
             signalled, lost the chase: {report:?}\n{}",
            w.describe()
        );
        assert!(w.alive_born().is_empty(), "{platform:?}:\n{}", w.describe());
        assert!(
            report.targeted.contains(&ROOT) && report.targeted.contains(&101),
            "{platform:?}: {report:?}"
        );
    }
}

/// The same exit before anything has been signalled is an absent root, never a recycled one:
/// the call returns the ordinary unverified report instead of an error that says the number
/// "may be recycled", and signals nothing.
#[test]
fn a_root_that_exits_between_the_guards_reads_before_any_signal_is_absent_not_recycled() {
    for platform in [Platform::Macos, Platform::Linux] {
        let sim = Sim::new(platform, root_waiting_on_one_worker(), vec![]);
        let handle = sim.handle(None, None);
        let os = RacingRoot::new(&sim, RootRead::ExitsAfterKills { after_kills: 0 });
        let report = terminate_with(&handle, WALK_CAP, &os)
            .expect("an exited root is an absent root, not a refusal");
        let w = sim.world();
        assert!(
            report.harness_unverified && !report.all_gone && !report.observed_gone,
            "{platform:?}: nothing was signalled, so nothing may be claimed: {report:?}"
        );
        assert!(
            w.kills.is_empty(),
            "{platform:?}: an absent root authorises no signal: {:?}",
            w.kills
        );
    }
}

/// The fix must not weaken the recycled-number refusal: a *different* creation identity at
/// the registered pid is a stranger, and nothing is signalled.
#[test]
fn a_different_creation_identity_at_the_root_is_still_refused_as_recycled() {
    for platform in [Platform::Macos, Platform::Linux] {
        let sim = Sim::new(platform, root_waiting_on_one_worker(), vec![]);
        let handle = sim.handle(None, None);
        let os = RacingRoot::new(&sim, RootRead::Stranger);
        let err = terminate_with(&handle, WALK_CAP, &os)
            .expect_err("a recycled pid has no safe termination");
        assert!(err.to_string().contains("recycled"), "{platform:?}: {err}");
        assert!(sim.world().kills.is_empty(), "{platform:?}");
    }
}

/// An identity that cannot be read from a root that is still live is not evidence of an
/// exit: the re-check finds it alive, so the refusal stands.
#[test]
fn an_unreadable_creation_identity_on_a_live_root_is_still_refused() {
    for platform in [Platform::Macos, Platform::Linux] {
        let sim = Sim::new(platform, root_waiting_on_one_worker(), vec![]);
        let handle = sim.handle(None, None);
        let os = RacingRoot::new(&sim, RootRead::UnreadableWhileLive);
        let err = terminate_with(&handle, WALK_CAP, &os)
            .expect_err("a live root with no readable identity is not signalled");
        assert!(err.to_string().contains("recycled"), "{platform:?}: {err}");
        assert!(sim.world().kills.is_empty(), "{platform:?}");
    }
}

// ─── The property runner ────────────────────────────────────────────────────────────────

/// xorshift64*: enough randomness for a process tree, no dependency, and a seed that can
/// be printed and replayed.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.max(1))
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u32) -> u32 {
        (self.next() % u64::from(n.max(1))) as u32
    }

    fn chance(&mut self, num: u32, den: u32) -> bool {
        self.below(den) < num
    }
}

const COMMS: [&str; 4] = ["node", "codex", "sh", "sleep"];

struct Generated {
    platform: Platform,
    tree: Tree,
    script: Vec<(u32, Event)>,
    pgid: Option<u32>,
    native_binary: Option<&'static str>,
    /// Every process dies within `Some(k)` polls of SIGTERM and nothing forks: I6 applies.
    prompt_within: Option<u32>,
}

fn gen_on_term(rng: &mut Rng, next_pid: &mut u32, forks_allowed: bool) -> OnTerm {
    let child = |next_pid: &mut u32, rng: &mut Rng| {
        let pid = *next_pid;
        *next_pid += 1;
        Spawn {
            pid,
            comm: COMMS[rng.below(4) as usize],
            own_group: rng.chance(1, 3),
            on_term: if rng.chance(1, 2) {
                OnTerm::DiesAfterPolls(rng.below(3))
            } else {
                OnTerm::Ignores
            },
        }
    };
    match rng.below(if forks_allowed { 6 } else { 4 }) {
        0..=2 => OnTerm::DiesAfterPolls(rng.below(4)),
        3 => OnTerm::Ignores,
        4 => {
            let fork_after = rng.below(3);
            OnTerm::ForksThenExits {
                children: vec![child(next_pid, rng)],
                fork_after,
                exit_after: fork_after + rng.below(3),
            }
        }
        _ => OnTerm::SpawnsAndLives {
            children: vec![child(next_pid, rng)],
            after_polls: rng.below(3),
        },
    }
}

fn generate(seed: u64) -> Generated {
    let mut rng = Rng::new(seed);
    let platform = if rng.chance(1, 2) {
        Platform::Macos
    } else {
        Platform::Linux
    };
    let well_behaved = rng.chance(1, 4);
    let mut next_pid = 200;
    let mut pids = vec![ROOT];
    let mut nodes: BTreeMap<u32, Tree> = BTreeMap::new();
    let mut parents: BTreeMap<u32, u32> = BTreeMap::new();
    let root_own_group = rng.chance(1, 2);
    let root_on_term = if well_behaved {
        OnTerm::DiesAfterPolls(rng.below(3))
    } else {
        gen_on_term(&mut rng, &mut next_pid, true)
    };
    nodes.insert(
        ROOT,
        leaf(
            ROOT,
            COMMS[rng.below(2) as usize],
            root_own_group,
            root_on_term,
        ),
    );
    let extra = rng.below(5);
    for i in 0..extra {
        let pid = 101 + i;
        let parent = pids[rng.below(pids.len() as u32) as usize];
        let on_term = if well_behaved {
            OnTerm::DiesAfterPolls(rng.below(3))
        } else {
            gen_on_term(&mut rng, &mut next_pid, true)
        };
        nodes.insert(
            pid,
            leaf(pid, COMMS[rng.below(4) as usize], rng.chance(1, 3), on_term),
        );
        parents.insert(pid, parent);
        pids.push(pid);
    }
    // Assemble bottom-up: children have larger pids than parents.
    for pid in pids.iter().rev() {
        if let Some(parent) = parents.get(pid).copied() {
            let child = nodes.remove(pid).expect("node");
            nodes.get_mut(&parent).expect("parent").children.push(child);
        }
    }
    let tree = nodes.remove(&ROOT).expect("root");

    let mut script = Vec::new();
    if !well_behaved {
        for _ in 0..rng.below(4) {
            let at = rng.below(10);
            let target = pids[rng.below(pids.len() as u32) as usize];
            let event = match rng.below(8) {
                0..=2 => {
                    let pid = next_pid;
                    next_pid += 1;
                    Event::Fork {
                        parent: target,
                        child: Spawn {
                            pid,
                            comm: COMMS[rng.below(4) as usize],
                            own_group: rng.chance(1, 3),
                            on_term: gen_on_term(&mut rng, &mut next_pid, false),
                        },
                    }
                }
                3..=4 => Event::Exit { pid: target },
                5 => Event::Exec { pid: target },
                _ => Event::PsFails {
                    until: at + 1 + rng.below(5),
                },
            };
            script.push((at, event));
        }
    }
    let pgid = (root_own_group && rng.chance(1, 2)).then_some(ROOT);
    let native_binary = rng.chance(1, 2).then_some("codex");
    Generated {
        platform,
        tree,
        script,
        pgid,
        native_binary,
        prompt_within: well_behaved.then_some(3),
    }
}

/// Cases per default run, at ~75 µs a case: 2000 → 0.15 s,
/// 5000 → 0.38 s, 20 000 → 1.5 s, 50 000 → 3.75 s. Five thousand is sub-second in CI; the
/// failures it finds surface at 0.01–0.2 % of seeds, so a fix is judged at 20 000 and 50 000.
const DEFAULT_CASES: u64 = 5_000;

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Every invariant, over ground truth. This block is what `docs/TERMINATION.md` quotes;
/// keep the two identical.
///
/// ```text
/// Let B be every pid born under the root during the call (ground truth), O ⊆ B the owned
/// set (B minus what had left the root's ppid walk before the call, D30), A the pids alive
/// at return, K the kills sent, R the report.
/// I1  R.observed_gone ⟹ A ∩ O ⊆ W, where W is the platform's stated residual window
///     (Linux: same-poll orphans; macOS: ∅).
/// I2  R.observed_gone ⟹ A ∩ R.targeted = ∅.
/// I3  every k ∈ K names a pid ≥ 2 that was ours when sent, with SIGTERM or SIGKILL only;
///     every killpg names a group of ours under a live leader.
/// I4  ps could not be asked at any point ⟹ ¬R.observed_gone ∧ R.harness_unverified
///     (an Err result satisfies this vacuously).
/// I5  handle.pgid = root ∧ root exited ∧ R.observed_gone ⟹ every group member exited
///     before the root.
/// I6  every process dies within k polls of its first SIGTERM and nothing forks ⟹
///     R.targeted = B ∧ A ∩ B = ∅ ∧ R.still_alive = ∅, and, unless handle.pgid = root,
///     R.observed_gone.
/// ```
fn check_invariants(
    seed: u64,
    g: &Generated,
    sim: &Sim,
    handle: &ProcessHandle,
    result: &Result<TerminationReport, ProcessError>,
) {
    let w = sim.world();
    let alive_born = w.alive_born();
    let ctx = || {
        format!(
            "seed={seed}\n{}  handle: pgid={:?} native_binary={:?}\n  script: {:?}\nresult={result:?}",
            w.describe(),
            g.pgid,
            g.native_binary,
            g.script
        )
    };

    // I3 needs no report: every kill names one of ours, never pid 0/1, never a stranger,
    // and only the two signals the escalation is made of.
    for k in &w.kills {
        assert!(k.pid >= 2, "I3: signalled pid {} — {}", k.pid, ctx());
        assert!(
            k.sig == libc::SIGTERM || k.sig == libc::SIGKILL,
            "I3: sent signal {} — {}",
            k.sig,
            ctx()
        );
        if let Some(instance) = k.instance {
            assert!(
                w.born_instances.contains(&instance),
                "I3: signalled a stranger at pid {} (instance {instance}) — {}",
                k.pid,
                ctx()
            );
        } else {
            assert!(
                w.born.contains(&k.pid),
                "I3: signalled pid {} which was never ours — {}",
                k.pid,
                ctx()
            );
        }
    }
    // A group signal is sent only under a live leader of that group (the module's own
    // rule), and the group is one of ours.
    for g_ in &w.killpgs {
        assert!(
            g_.pgid >= 2
                && w.born.contains(&g_.pgid)
                && g_.leader_alive
                && (g_.sig == libc::SIGTERM || g_.sig == libc::SIGKILL),
            "I3: killpg {:?} — {}",
            g_,
            ctx()
        );
    }

    // I4: could not ask ⟹ not confirmed. An Err result satisfies it vacuously.
    if (w.liveness_errs > 0 || w.snapshot_errs > 0)
        && let Ok(r) = result
    {
        assert!(
            !r.observed_gone && r.harness_unverified,
            "I4: ps failed and the report is confident — {}",
            ctx()
        );
    }

    let Ok(r) = result else {
        return;
    };

    // `all_gone` is `observed_gone` and every delivery instance-bound: no numeric kill and
    // no group signal may sit behind it.
    if r.all_gone {
        assert!(
            r.observed_gone && w.kills.iter().all(|k| k.bound) && w.killpgs.is_empty(),
            "all_gone beside a numeric delivery — {}",
            ctx()
        );
    }

    if r.observed_gone {
        // I1 / I1′, over the owned set as D30 defines it. Two allowances, both preconditions
        // of the contract and neither a hole in the walk:
        //  - a lineage process that had already left the root's ppid walk before the call
        //    began (orphaned by a poll-0 exit) is outside the owned set on every platform —
        //    D30: "a process that left that set before the first snapshot is not claimed";
        //  - on Linux, with no fork witness, a child forked and orphaned inside one poll
        //    interval is unobservable by any ppid walk (`docs/TERMINATION.md`, residual
        //    windows). macOS gets no such allowance: the kqueue witness closes it.
        // Anything that leaves the lineage during the call, other than the Linux same-poll
        // case, is asserted over — a fork after the root's own exit included.
        let mut allowance = w.escaped_before_call_pids();
        if g.platform == Platform::Linux {
            allowance.extend(w.same_poll_orphans());
        }
        let escaped: Vec<u32> = alive_born
            .iter()
            .copied()
            .filter(|p| !allowance.contains(p))
            .collect();
        assert!(
            escaped.is_empty(),
            "I1: observed_gone over live {escaped:?} — {}",
            ctx()
        );
        // I2
        let targeted_alive: Vec<u32> = r
            .targeted
            .iter()
            .copied()
            .filter(|p| w.alive().contains(p))
            .collect();
        assert!(
            targeted_alive.is_empty(),
            "I2: observed_gone over targeted-and-alive {targeted_alive:?} — {}",
            ctx()
        );
    }

    // I5
    if handle.pgid == Some(ROOT) && !w.alive().contains(&ROOT) && r.observed_gone {
        let root_exit = w
            .instances
            .values()
            .filter(|l| l.pid == ROOT)
            .filter_map(|l| l.exit_seq)
            .max();
        let members_gone_first = w
            .procs
            .iter()
            .filter(|(pid, p)| **pid != ROOT && p.pgid == ROOT)
            .all(|(_, p)| {
                w.instances
                    .get(&p.start_id)
                    .and_then(|l| l.exit_seq)
                    .is_some_and(|seq| Some(seq) < root_exit)
            });
        assert!(
            members_gone_first,
            "I5: the root led the group and died, and the report is confident — {}",
            ctx()
        );
    }

    // I6: liveness — a fix may not pass by refusing everything.
    //
    // Scoped, not weakened. The claim half (`observed_gone`) is asserted only where the
    // implementation is capable of proving it: a handle whose root leads its own process
    // group (`pgid == Some(root)`) is excluded, because `ProcInfo` carries no pgid, a group
    // can never be enumerated, and the module fails closed there by design —
    // `harness_unverified` is `true` for every group termination (`docs/TERMINATION.md`).
    // Asserting `observed_gone` on that shape would be asserting an unsatisfiable property.
    //
    // What is not dropped for the group shape is the action half: a well-behaved group tree
    // must still have every member named in `targeted` and nothing of ours alive at return.
    // That is what stops a fix from passing by refusing everything — a group is refused a
    // claim, never a kill. (I5 above is the group's ordering guard; under the current
    // contract its `observed_gone` antecedent is never true for a group, so it is vacuous
    // here and this clause is what actually exercises the group shape.)
    if g.prompt_within.is_some() {
        assert!(
            sorted(&r.targeted) == sorted(&w.born),
            "I6: a well-behaved tree was not terminated with every member targeted — {}",
            ctx()
        );
        assert!(
            alive_born.is_empty() && r.still_alive.is_empty(),
            "I6: a well-behaved tree left a member alive at return — {}",
            ctx()
        );
        if handle.pgid != Some(ROOT) {
            assert!(
                r.observed_gone,
                "I6: a well-behaved non-group tree was not confirmed — {}",
                ctx()
            );
        }
    }
}

/// `GOALCUP_SIM_SEED` picks the base seed, `GOALCUP_SIM_CASES` the count. A failure prints
/// the case's own seed, which replays it alone with `GOALCUP_SIM_CASES=1`.
/// `GOALCUP_SIM_ALL=1` runs every case before failing and prints one line per failing seed,
/// for characterising a failure set rather than chasing it one seed at a time.
#[test]
fn property_terminate_with_respects_ground_truth() {
    let base = env_u64("GOALCUP_SIM_SEED", 0x5EED_2026);
    let cases = env_u64("GOALCUP_SIM_CASES", DEFAULT_CASES);
    let collect_all = env_u64("GOALCUP_SIM_ALL", 0) == 1;
    let mut failures: Vec<(u64, String)> = Vec::new();
    for i in 0..cases {
        let seed = base.wrapping_add(i);
        let outcome = std::panic::catch_unwind(|| {
            let g = generate(seed);
            let sim = Sim::new(g.platform, g.tree.clone(), g.script.clone());
            let handle = sim.handle(g.pgid, g.native_binary);
            let result = run(&sim, &handle);
            check_invariants(seed, &g, &sim, &handle, &result);
        });
        if let Err(payload) = outcome {
            eprintln!(
                "terminate_sim property: FAILING SEED {seed} (GOALCUP_SIM_SEED={seed} GOALCUP_SIM_CASES=1)"
            );
            if !collect_all {
                std::panic::resume_unwind(payload);
            }
            let first_line = payload
                .downcast_ref::<String>()
                .map(|s| s.lines().next().unwrap_or("").to_string())
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            failures.push((seed, first_line));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {cases} cases failed:\n{}",
        failures.len(),
        failures
            .iter()
            .map(|(seed, what)| format!("  seed {seed}: {what}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
