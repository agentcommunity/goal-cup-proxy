//! Terminating the harness process tree at seal (D22, D30).
//!
//! **Read `docs/TERMINATION.md` first.** It is the contract this module implements — the
//! outcomes, the three [`Anchor`] states, what each [`TerminationReport`] field does and does
//! not claim, the ordering and adoption rules, the duration bound, and the windows that are
//! still open on each platform — in one page, with the property-test invariant quoted from
//! `tests/terminate_sim.rs` so the two cannot drift. This header does not repeat it.
//!
//! The shape of the code: [`terminate_with`] is the whole algorithm over one [`ProcessOs`]
//! seam (`os.rs`); [`terminate`] and [`terminate_within`] are it with [`LiveOs`]. The
//! identity guard, the ordering of the owned set and the fail-closed reports are in
//! `authority.rs`; the macOS audit-token singleton path is in `darwin.rs`. Nothing in this
//! module touches the filesystem: `lock_paths[]` are reported at seal, never deleted.
//!
//! macOS and Linux only.

use goalcup_protocol::manifest::KillStrategy;
use goalcup_protocol::process::{ProcInfo, ProcessError, ProcessHandle, descendants, liveness};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

mod authority;
#[cfg(target_os = "macos")]
mod darwin;
pub mod os;

use authority::{
    Owned, Presence, Refused, absent_root_unverified, bounded_targets, containment_unverified,
    degraded_guard, guard_registered, identity_unverified, instance_alive, members_deepest_first,
    owns_registered_group, push_capped, push_targeted, signal_owned, signal_registered_group,
};
#[cfg(target_os = "macos")]
use darwin::{direct_native_identity_matches, terminate_darwin_singleton};
use os::alive;
pub use os::{BoundTarget, ForkWitness, InstanceAuthority, LiveOs, ProcessOs, SignalOutcome};

/// How long the verification sweep waits for a SIGKILLed tree to leave the process table,
/// and the seal's reconfirm delay: both live in `goalcup_protocol::process`, where the CLI
/// derives its finish timeout from them (`termination_bound`), and are re-exported unchanged.
pub use goalcup_protocol::process::{KILL_SWEEP_TIMEOUT_MS, RECONFIRM_DELAY_MS};

/// The gap between liveness polls. Polling is used for the "is it gone" wait and for
/// nothing else: `waitpid` is not available to this process, which is not the harness's
/// parent — the CLI was.
const LIVENESS_POLL_MS: u64 = 50;

/// Ask the process table once more, after `delay`, whether any pid a termination named is
/// alive.
///
/// `false` for an empty list is deliberately not the answer this returns: an empty list is
/// vacuously gone, and the caller that turns this into a record must decide separately
/// whether anything was ever targeted. This function answers the narrow question it is
/// named for and nothing else.
pub fn none_alive_after(targeted: &[u32], delay: Duration) -> Result<bool, ProcessError> {
    std::thread::sleep(delay);
    // `liveness`, not `is_alive`: a `/bin/ps` that could not be spawned is "could not ask",
    // and the second sweep of a seal must not fold that to "gone" — the caller maps the `Err`
    // to `ProcessAuthorityUnavailable` rather than confirming a containment it never observed.
    for pid in targeted.iter().copied() {
        if liveness(pid)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// pid 0 means "every process in the caller's own group" and pid 1 is init. Neither is ever
/// a harness, and both are catastrophic to signal by accident.
const MIN_SIGNALLABLE_PID: u32 = 2;

/// The most pids one termination will ever carry in `targeted`.
///
/// The chase adopts every newcomer it finds below something already being terminated, and
/// `newcomers` is O(|targeted| × |table|) per 50 ms pass — so a harness that respawns a
/// child in a loop makes the sweep quadratic in its own fork rate and the report a list
/// with no ceiling. Past this the sweep stops adopting and the report says
/// so by naming what it did adopt: the group signal below is what still reaches a tree
/// forking faster than a sweep can name it.
///
/// 4096 is far above any real harness — the shipped connectors' trees are single digits —
/// and far below anything that costs a seal a measurable amount of time.
const MAX_TARGETED: usize = 4_096;

/// What the seal learned by killing: who was targeted, who ignored SIGTERM, who then needed
/// SIGKILL, and whether the tree is actually gone.
///
/// `all_gone` says whether the tree is actually gone. A `native_pid` still alive here is the
/// failure this module exists to report, and it has to reach the seal as data rather than as
/// a silent success.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminationReport {
    pub strategy: KillStrategy,
    /// Every pid signalled, in the order it was signalled: the native process first, then
    /// the tree it was hiding inside.
    pub targeted: Vec<u32>,
    /// Those still alive when the grace expired.
    pub survived_term: Vec<u32>,
    /// Those that then needed SIGKILL.
    pub killed: Vec<u32>,
    /// Everything the last trustworthy sweep still found running. This can be empty beside
    /// `all_gone: false` when containment itself became unverified and no remaining process
    /// can be named safely.
    pub still_alive: Vec<u32>,
    /// True only when a sweep taken *after* the kill found nothing in `targeted` alive
    /// and every signal that reached the tree was delivered through a handle bound to the
    /// exact process instance.
    ///
    /// Two independent facts, deliberately conjoined: this is the field the recovery paths
    /// (`end_harness_verified`) demand before they delete a termination authority, and
    /// neither half may be assumed there. It is not available on macOS — see
    /// [`TerminationReport::observed_gone`], which is the observation half alone.
    pub all_gone: bool,
    /// The observation half of `all_gone`, on its own: **the bounded owned set was observed
    /// gone**.
    ///
    /// True only when the escalation ran from first signal to last sweep under a root this
    /// daemon re-authenticated at every step, the whole owned set was enumerated within the
    /// caller's walk cap, and the final authenticated snapshot found none of `targeted`
    /// alive. Every fail-closed return below — a refused identity, an absent root, a lost
    /// root mid-chase — sets it `false`, because none of those took a final sweep worth the
    /// name.
    ///
    /// What it does not claim, and what separates it from `all_gone`: that each `kill(2)`
    /// was atomic with the snapshot that authorised it. macOS has no pidfd-class handle for
    /// a tree member, so the last POSIX numeric call always crosses an unguarded boundary
    /// there and `all_gone` is consequently never true on that platform. A caller that
    /// needs the stronger claim reads `all_gone`; a caller recording what was *observed*
    /// reads this.
    pub observed_gone: bool,
    /// The caller's walk cap refused a target, so the set this report names is not the
    /// whole owned set and no sweep over it can speak for the tree.
    ///
    /// Distinct from `harness_unverified`, which says the daemon lost the root's identity.
    /// A capped walk kept its root and its authority; it simply ran out of budget, and the
    /// honest record of that is "still running", not "authority unavailable".
    pub walk_cap_exceeded: bool,
    /// Wall-clock milliseconds the whole escalation took. At least `grace_seconds` when
    /// anything survived SIGTERM, because the grace is a promise to the harness and not a
    /// budget to be cut short.
    pub waited_ms: u64,
    /// The registered handle carried no `argv`, so [`guard_registered`] could ask the age
    /// question and not the identity one: everything below was signalled on the strength of
    /// a pid and its age alone.
    ///
    /// Carried rather than logged. The two guards are not
    /// interchangeable — age alone admits a number recycled early in the run, and the thing
    /// that separates such a number from the harness is the program it is running — so a
    /// seal that ran with one of them missing has to be able to say so. Every shipped
    /// connector's launch records an argv; this is the field that makes a build that
    /// stopped doing so visible instead of silently weaker.
    pub guard_degraded: bool,
    /// [`guard_registered`] refused the pid, so this daemon cannot say what remains of the
    /// harness: a recycled number, or a live pid running a program the launch never
    /// registered. A tree nobody could identify is a tree nobody may claim is gone — that is
    /// the false claim the whole module exists to refuse.
    ///
    /// **Not** set by the root merely being gone. Atomicity is `all_gone` beside
    /// `observed_gone`; a capped walk is `walk_cap_exceeded`; this is identity, and only
    /// identity.
    ///
    /// `all_gone` is never `true` beside it.
    pub harness_unverified: bool,
}

/// Terminate the tree per the connector's `kill.strategy`, then verify it is gone.
///
/// Never reaps: the daemon is not the harness's parent, and a test that owns a `Child`
/// calls `wait()` itself. A pid whose process has exited but not been reaped is gone as far
/// as this module is concerned — [`goalcup_protocol::process::is_alive`] rules on that, once, for both halves.
pub fn terminate(handle: &ProcessHandle) -> Result<TerminationReport, ProcessError> {
    terminate_with(handle, MAX_TARGETED, &LiveOs)
}

/// [`terminate`], with the owned-set walk bounded by the caller's own budget.
///
/// The V4 finalization intent carries `limits.process_walk`, a number the participant's
/// start receipt was signed over, and a seal that walked further than the profile it was
/// judged by would be reporting on a set the rules never admitted. The cap is clamped into
/// `1..=MAX_TARGETED`: a caller may tighten this module's ceiling and may never raise it.
///
/// Hitting the cap is recorded, never hidden. `walk_cap_exceeded` says the enumeration was
/// incomplete, which is why no sweep afterwards may call the tree gone.
pub fn terminate_within(
    handle: &ProcessHandle,
    walk_cap: usize,
) -> Result<TerminationReport, ProcessError> {
    terminate_with(handle, walk_cap, &LiveOs)
}

/// [`terminate_within`], against whichever operating system `os` is.
///
/// This is the whole algorithm; [`terminate`] and [`terminate_within`] are it with
/// [`LiveOs`]. The seam exists because two of the module's confirmed defects cannot be
/// provoked by any real fixture — `/bin/ps` is spawned by absolute path, so nothing a shell
/// script does can make it fail to spawn — and because the ground truth a verification is
/// judged against is what a tree forked, not what a snapshot happened to show.
pub fn terminate_with(
    handle: &ProcessHandle,
    walk_cap: usize,
    os: &dyn ProcessOs,
) -> Result<TerminationReport, ProcessError> {
    let started = os.now();
    let cap = walk_cap.clamp(1, MAX_TARGETED);

    // A low root is more dangerous than a low direct target: walking from pid 1 discovers
    // much of the machine before the per-target guard can filter anything. Refuse before
    // snapshot discovery or group ownership checks can turn init into tree authority.
    if handle.pid < MIN_SIGNALLABLE_PID {
        return Err(ProcessError::NoTarget(format!(
            "pid {} cannot be a harness termination root",
            handle.pid
        )));
    }

    // `wrapper_unsafe` is a declaration that the connector has no verified way to end its
    // session. Refusing is the whole content of the strategy: a report claiming `all_gone`
    // over a tree nobody knows how to kill is a false containment claim with extra steps.
    if handle.strategy == KillStrategy::WrapperUnsafe {
        return Err(ProcessError::Unsupported(format!(
            "kill.strategy = wrapper_unsafe: pid {} belongs to a connector that declares no \
             verified termination path, so this seal cannot claim the harness is gone",
            handle.pid
        )));
    }

    // The group is signalled whenever the registered pid leads one, whatever the declared
    // strategy. A pgid that is not the pid names a group this launch joined rather than
    // created, and its other members were not launched by the tracker, so it is never used.
    // `descendants` is ppid-only and `ProcInfo` carries no pgid, so a grandchild orphaned by our
    // own first SIGTERM wave reparents to pid 1 and leaves the walk entirely. It does not leave
    // the process group, and a `native_pid` connector that registered a leading pgid is handing
    // this module the one handle that can still reach such a process; refusing to use it
    // because of a field naming the *preferred* path would leave such an orphan alive.
    let requested_group = match handle.pgid {
        Some(pgid) if pgid == handle.pid => Some(pgid),
        // pgid: None (a TUI cannot own its group without taking SIGTTIN on its first
        // terminal read) or a borrowed group: the descendant walk, which is the same path
        // `native_pid` takes and is why `targeted` is non-empty either way.
        _ => None,
    };

    // **The guard before the snapshot**, so the table the first SIGTERM wave is chosen
    // from is the freshest thing this function read: `guard_registered` spawns `/bin/ps`,
    // and a snapshot taken on the other side of that spawn is a snapshot a fork could
    // already have outrun.
    let presence = match guard_registered(os, handle) {
        Ok(presence) => presence,
        // An argv mismatch is not permission to use a `pgid`: a recycled pid can be old
        // enough to pass the age slack and lead a newly-created stranger group. The daemon
        // records uncertainty rather than treating the number as a recovery handle.
        Err(Refused::ArgvRenamed) => return Ok(identity_unverified(os, handle, started)),
        Err(Refused::Recycled(e)) => return Err(e),
        // The guard could not spawn `/bin/ps` at all, so it never asked whether the root is
        // live. Re-raise it as the `Err(Spawn)` `os.snapshot()?` would give one line below:
        // "could not ask" is not the ordinary absent-root seal.
        Err(Refused::Unavailable(e)) => return Err(e),
    };

    // **An absent pid leads nothing.** `guard_registered` answers `Absent`
    // for a pid that is not in the process table at all, which is the ordinary seal — the
    // participant quit hours ago — and which it cannot ask either of its questions about:
    // there is no age to compare and no argv to match. The group id on the handle is then a
    // number and nothing else, and a `killpg` on it days later reaches whatever process has
    // since been given that number and made itself a group leader. It is a stranger's tree,
    // signalled on the strength of a handle that names no live process at all.
    //
    // So the group is dropped here.  More importantly, an absent root leaves no way to
    // connect a possible orphan to this launch: a native hint cannot be a descendant of a
    // root that is not in this snapshot, and no hint says even less.  It may be that the
    // harness exited cleanly, but a wrapper can have already reparented its native child;
    // `all_gone` would be an unsupported containment claim.  Record the uncertainty and
    // signal neither a recycled root number nor a group it no longer leads.
    if matches!(presence, Presence::Absent) {
        return Ok(absent_root_unverified(os, handle, started));
    }
    let table = os.snapshot()?;
    if !table.iter().any(|row| row.pid == handle.pid) {
        return Ok(absent_root_unverified(os, handle, started));
    }
    // A handle's `pgid == pid` records what the CLI created; the kernel must still say the
    // live verified root leads that group before any `killpg` can use it.
    let group = requested_group.filter(|_| owns_registered_group(os, handle));

    // The owned set below the root, ordered deepest level first. The root is not in it:
    // it is signalled last — see the two phases below.
    let (members, cap_exceeded) = bounded_targets(members_deepest_first(handle, &table), cap);

    // Darwin's audit-token singleton path: a root with no descendants and no group is ended
    // through its instance-bound audit token, which proves exact root delivery and exit (but
    // not the tree's prehistory).
    #[cfg(target_os = "macos")]
    if group.is_none()
        && members.is_empty()
        && direct_native_identity_matches(handle, &table)
        && let Some(report) = terminate_darwin_singleton(os, handle, started)
    {
        return Ok(report);
    }

    let mut esc = Escalation::new(group.is_none());
    esc.cap_exceeded = cap_exceeded;
    // The root's witness is attached now, not when phase B signals it: a root that forks and
    // exits on its own inside one poll of phase A — a wrapper quitting from its own timer
    // while its children are still dying — leaves a child no snapshot shows, and only a
    // witness already on the root can refuse the claim.
    esc.track(os, handle.pid);

    // ── PHASE A: the descendants, deepest level first, the root kept alive ──────────────────
    //
    // Root-last is the load-bearing idea. While a non-root member is still owned — an orphan
    // alive included, since the gate follows identity — the root is left unsignalled, so
    // every sweep stays [`Anchor::Rooted`] and adoption always has a live root to walk. A
    // `Closed` anchor still walks from every live identity-verified member, but the root's
    // tree is the one namespace nothing else can stand in for. Deepest first means a parent
    // is never ended before the children hanging off it, and a leaf is signalled before the
    // root even when the root would honour SIGTERM faster than the wave once reached the
    // leaves.
    for &pid in &members {
        if esc.signal_first(os, handle, pid, libc::SIGTERM).is_err() {
            return Ok(esc.into_containment(os, handle, started));
        }
    }
    if esc
        .escalate(os, handle, cap, handle.grace_seconds, Some(&members))
        .is_err()
    {
        return Ok(esc.into_containment(os, handle, started));
    }

    // ── PHASE B: the root, last ─────────────────────────────────────────────────────────────
    //
    // The root is part of the owned set and is named in the report whether it outlived its
    // descendants or exited on its own the instant its last child died (`sh: sleep 300 &
    // wait`). A live root is signalled now — individually, and, for a group, by `killpg`,
    // which necessarily ends the leader too and so could only run once the walked set was
    // handled. A group is never enumerable (`ProcInfo` carries no pgid), so its result is
    // fail-closed regardless: nothing may claim the group empty.
    if !esc.targeted.contains(&handle.pid) {
        esc.targeted.push(handle.pid);
    }
    if matches!(guard_registered(os, handle), Ok(Presence::Live)) {
        if group.is_some() {
            let _ = signal_registered_group(os, handle, libc::SIGTERM);
        }
        if let Some(atomic) = esc.signal(os, handle, handle.pid, libc::SIGTERM) {
            esc.atomically_bound &= atomic;
        }
        if esc
            .escalate(os, handle, cap, handle.grace_seconds, None)
            .is_err()
        {
            return Ok(esc.into_containment(os, handle, started));
        }
        if group.is_some() {
            let _ = signal_registered_group(os, handle, libc::SIGKILL);
        }
    }

    // ── The report ──────────────────────────────────────────────────────────────────────────
    //
    // `still_alive` is asked of every targeted pid by number (identity-guarded), never by the
    // current lineage: a member another member forked and then abandoned to pid 1 on its own
    // exit has left the walk while alive, and a lineage-only check would drop it and call the
    // tree gone over it.
    let forks_accounted = esc.forks_accounted();
    let still_alive = esc.residual_alive(os);

    // A group can never be proven empty, so the group path fails closed. Otherwise the
    // observation is: the last authenticated sweep found nothing, the walk was never capped,
    // and — on macOS — every fork a member's kqueue witness reported was accounted for by an
    // adoption from that member in the poll it was read (an escapee, which no snapshot
    // ever shows, is a fork nothing adopted).
    let harness_unverified = group.is_some();
    let observed_gone =
        !harness_unverified && !esc.cap_exceeded && still_alive.is_empty() && forks_accounted;
    Ok(TerminationReport {
        strategy: handle.strategy,
        guard_degraded: degraded_guard(handle),
        harness_unverified,
        // Atomicity — an instance-bound delivery for every signal — is what separates
        // `all_gone` from `observed_gone`, and macOS has no pidfd-class handle for a tree
        // member, so it is never true there for more than the singleton.
        all_gone: esc.atomically_bound && observed_gone,
        observed_gone,
        walk_cap_exceeded: esc.cap_exceeded,
        still_alive,
        targeted: esc.targeted,
        survived_term: esc.survived_term,
        killed: esc.killed,
        waited_ms: os::waited_ms(os, started),
    })
}

/// The mutable state threaded through both escalation phases.
///
/// # The worst-case duration bound
///
/// Each phase runs its own grace and kill sweep — the descendants together, then the root —
/// and each honours `grace_seconds`, so the total wall time is bounded by
/// `2 × (grace_seconds + KILL_SWEEP_TIMEOUT_MS)`. A deeper per-level split would multiply by
/// the tree's depth; the two-phase form is the subset that keeps the root alive while the
/// descendants die, and it is the deliberate ceiling here.
struct Escalation {
    targeted: Vec<u32>,
    survived_term: Vec<u32>,
    killed: Vec<u32>,
    /// Every signal so far was delivered through a handle bound to the exact process instance
    /// (a Linux pidfd). A group signal or a macOS numeric `kill` turns this false for good; it
    /// is what separates `all_gone` from `observed_gone`.
    atomically_bound: bool,
    /// The recorded process-start identity per targeted pid. It is what the phase gate, the
    /// SIGKILL selection, the adoption walk and the final by-number sweep all follow a member
    /// by: a number recycled since the signal cannot pass for our member, and a member
    /// orphaned out of the root's ppid walk while alive is lost to none of them.
    instances: BTreeMap<u32, Option<String>>,
    /// One kqueue fork/exec witness per targeted member, on macOS. Empty elsewhere: the seam's
    /// `fork_witness` returns `None`, so nothing is pushed and `forks_accounted` is vacuous.
    witnesses: Vec<(u32, Box<dyn ForkWitness>)>,
    /// Members whose witness reported a fork in the previous poll's second window — after
    /// that poll's snapshot — that no adoption has paid for yet. Its child is in this poll's
    /// snapshot under the forker if the forker is alive, and this poll's adoption pays it;
    /// otherwise the forker died with a child nobody named, and the bit stays unpaid for good.
    pending: BTreeSet<u32>,
    /// A witnessed fork or exec no adoption accounted for, or a witness that failed. Set once
    /// and never cleared: it blocks `observed_gone` for the whole call.
    fork_unaccounted: bool,
    cap_exceeded: bool,
}

impl Escalation {
    fn new(atomically_bound: bool) -> Escalation {
        Escalation {
            targeted: Vec::new(),
            survived_term: Vec::new(),
            killed: Vec::new(),
            atomically_bound,
            instances: BTreeMap::new(),
            witnesses: Vec::new(),
            pending: BTreeSet::new(),
            fork_unaccounted: false,
            cap_exceeded: false,
        }
    }

    /// Record a pid's instance identity and, on macOS, attach a fork witness — once per pid.
    fn track(&mut self, os: &dyn ProcessOs, pid: u32) {
        if self.instances.contains_key(&pid) {
            return;
        }
        self.instances.insert(pid, os.start_identity(pid));
        if let Some(witness) = os.fork_witness(pid) {
            self.witnesses.push((pid, witness));
        }
    }

    /// Signal `pid` as one of ours — in the live root's ppid tree, or in the tree of a
    /// targeted member alive with its recorded identity — with the proof taken immediately
    /// before the `kill`. `None` when the root was lost, or `pid` is alive and ours by nothing.
    fn signal(
        &self,
        os: &dyn ProcessOs,
        handle: &ProcessHandle,
        pid: u32,
        sig: i32,
    ) -> Option<bool> {
        signal_owned(os, handle, &self.owned(handle), pid, sig)
    }

    fn owned(&self, handle: &ProcessHandle) -> Owned<'_> {
        Owned {
            root: handle.pid,
            targeted: &self.targeted,
            instances: &self.instances,
        }
    }

    /// SIGTERM a newly discovered member, add it to `targeted`, and begin tracking it. `Err`
    /// means the re-authenticated root refused the signal — its identity changed, or it was
    /// lost — and the caller returns fail-closed.
    fn signal_first(
        &mut self,
        os: &dyn ProcessOs,
        handle: &ProcessHandle,
        pid: u32,
        sig: i32,
    ) -> Result<(), ()> {
        // Track before the signal. A macOS fork witness must be attached while the member is
        // still alive and before the SIGTERM that may make it fork-and-exit in the same
        // instant; attach it after and the member is already gone, the witness never
        // registers, and the escapee is missed.
        let is_new = !self.targeted.contains(&pid);
        if is_new {
            self.track(os, pid);
        }
        let atomic = self.signal(os, handle, pid, sig).ok_or(())?;
        self.atomically_bound &= atomic;
        if is_new {
            self.targeted.push(pid);
        }
        Ok(())
    }

    /// One grace-then-kill escalation over the members currently in `targeted`. `gate` names
    /// the pids whose death ends the grace and the sweep: `Some(members)` in phase A, so the
    /// root's own churn (a wrapper looping `sleep 300 & wait`) cannot extend it, and `None` in
    /// phase B, where the whole targeted set must go quiet. `Err` on a lost root.
    fn escalate(
        &mut self,
        os: &dyn ProcessOs,
        handle: &ProcessHandle,
        cap: usize,
        grace_seconds: u64,
        gate: Option<&[u32]>,
    ) -> Result<(), ()> {
        let grace_ends = os.now() + Duration::from_secs(grace_seconds);
        loop {
            self.poll(os, handle, cap, libc::SIGTERM)?;
            if self.gate_clear(os, gate) || os.now() >= grace_ends {
                break;
            }
            os.sleep(Duration::from_millis(LIVENESS_POLL_MS));
        }

        // SIGKILL every targeted pid still alive with its recorded identity — an orphan
        // included: when its parent died first it left the root's ppid walk, not the owned
        // set. The anchor is taken first so a table that cannot be read fails closed
        // here instead of reading every survivor as gone.
        self.anchor(os, handle)?;
        let survivors = self.alive_by_identity(os, &self.targeted);
        for pid in &survivors {
            if !self.survived_term.contains(pid) {
                self.survived_term.push(*pid);
            }
        }
        for pid in survivors {
            let atomic = self.signal(os, handle, pid, libc::SIGKILL).ok_or(())?;
            self.atomically_bound &= atomic;
            push_capped(&mut self.killed, pid, cap);
        }

        // One more chase: the walk that chose who to SIGKILL is by then a poll old.
        let sweep_ends = os.now() + Duration::from_millis(KILL_SWEEP_TIMEOUT_MS);
        loop {
            let late = self.poll(os, handle, cap, libc::SIGKILL)?;
            if (self.gate_clear(os, gate) && !late) || os.now() >= sweep_ends {
                break;
            }
            os.sleep(Duration::from_millis(LIVENESS_POLL_MS));
        }
        Ok(())
    }

    /// One poll: read every fork witness, take the anchor, adopt, read again, account.
    /// Returns whether anything new was found; `Err` on a lost root.
    ///
    /// The witnesses are read twice, and an adoption pays only for the fork it accounts for —
    /// a member trusted for one adopted child is not trusted for its next fork:
    ///
    /// - **Before the snapshot.** A fork read here happened before it, so its child is in
    ///   this snapshot under a live forker — and adopted this poll — or is nowhere, because
    ///   the forker died in between: the one-poll fork-and-exit escapee, which no
    ///   snapshot ever shows. Paid by an adoption from the forker this poll, or unpaid for
    ///   good. Every reaction to a signal this daemon sent lands here: phase A's first wave
    ///   and phase B's root are signalled before their first poll, and a newcomer signalled
    ///   in this poll's adoption is left out of the second read below.
    /// - **After the adoption.** A fork read here landed between the first read and now: its
    ///   child is in this snapshot (adopted this poll) or in the next one under a live forker
    ///   (adopted next poll). Paid by either; a forker that dies before the next snapshot
    ///   leaves it unpaid — the escapee again. What this second read is for: the shell whose
    ///   handler forks a millisecond after `kill(2)` returns, during the `ps` that follows
    ///   the first read, and exits once its children are killed. Without it that fork is read
    ///   one poll after its children were adopted, over a forker already gone, and the
    ///   qualifying journey's own respawn shape is refused.
    ///
    /// The kernel hands over one bit per read, so two forks by one member inside one window,
    /// the first child adopted and the second's line gone before the next snapshot, are one
    /// paid bit: `docs/TERMINATION.md` states that window.
    fn poll(
        &mut self,
        os: &dyn ProcessOs,
        handle: &ProcessHandle,
        cap: usize,
        sig: i32,
    ) -> Result<bool, ()> {
        let before = self.read_witnesses(&[]);
        let (table, rooted) = self.anchor(os, handle)?;
        let adopted = self.adopt(os, handle, cap, &table, rooted, sig)?;
        let after = self.read_witnesses(&adopted.signalled);
        self.account(&before, &after, adopted.from);
        Ok(adopted.late)
    }

    /// One generation of forks per witnessed member — `(pid, forked or execed since the last
    /// read)` — for every witness but those on `except`. A witness that cannot answer is a
    /// fork that cannot be accounted for.
    fn read_witnesses(&mut self, except: &[u32]) -> Vec<(u32, bool)> {
        let mut growth = Vec::with_capacity(self.witnesses.len());
        for (pid, witness) in &mut self.witnesses {
            if except.contains(pid) {
                continue;
            }
            match witness.take_growth() {
                Some(grew) => growth.push((*pid, grew)),
                None => self.fork_unaccounted = true,
            }
        }
        growth
    }

    /// Match the two reads to this poll's adoptions, forker by forker (see [`Self::poll`]).
    fn account(
        &mut self,
        before: &[(u32, bool)],
        after: &[(u32, bool)],
        adopted_from: BTreeSet<u32>,
    ) {
        let owed = std::mem::take(&mut self.pending);
        for &(pid, grew) in before {
            if (grew || owed.contains(&pid)) && !adopted_from.contains(&pid) {
                self.fork_unaccounted = true;
            }
        }
        for &(pid, grew) in after {
            if grew && !adopted_from.contains(&pid) {
                self.pending.insert(pid);
            }
        }
    }

    /// The current sweep anchor as a table and whether the root is live in it. `Err(())` on a
    /// lost or unaskable root.
    fn anchor(
        &self,
        os: &dyn ProcessOs,
        handle: &ProcessHandle,
    ) -> Result<(Vec<ProcInfo>, bool), ()> {
        match sweep_anchor(os, handle) {
            Anchor::Rooted(table) => Ok((table, true)),
            Anchor::Closed(table) => Ok((table, false)),
            Anchor::Lost => Err(()),
        }
    }

    /// Have the gating pids all gone? `Some(gate)` asks only about those (phase A); `None`
    /// asks about the whole targeted set (phase B).
    fn gate_clear(&self, os: &dyn ProcessOs, gate: Option<&[u32]>) -> bool {
        self.alive_by_identity(os, gate.unwrap_or(&self.targeted))
            .is_empty()
    }

    /// Which of `pids` are alive as the exact instance we signalled, by number. Never filtered
    /// by lineage — see [`instance_alive`] — so a member orphaned alive keeps its phase open,
    /// is chosen for SIGKILL, and is named in `still_alive`. Over-reporting is the safe way.
    fn alive_by_identity(&self, os: &dyn ProcessOs, pids: &[u32]) -> Vec<u32> {
        pids.iter()
            .copied()
            .filter(|pid| {
                instance_alive(os, *pid, self.instances.get(pid).and_then(|i| i.as_deref()))
            })
            .collect()
    }

    /// Adopt every newcomer below the live root **or below any targeted member alive with its
    /// recorded identity**, signalling each with `sig`. `Err` on a lost root.
    ///
    /// Walking from the members and not the root alone is what names a child a member forked
    /// after its own parent died — it has left the root's ppid walk, and on Linux (no kqueue
    /// witness) a walk from the still-live forker is the only thing that can still see it —
    /// and, once the root is gone, what names anything at all. The member's identity is what
    /// authorises the signal (`Owned`); a newcomer ours by nothing turns the report
    /// fail-closed, the safe direction.
    fn adopt(
        &mut self,
        os: &dyn ProcessOs,
        handle: &ProcessHandle,
        cap: usize,
        table: &[ProcInfo],
        rooted: bool,
        sig: i32,
    ) -> Result<Adopted, ()> {
        let mut adopted = Adopted::default();
        for pid in self.newcomers_from_live(os, handle, table, rooted) {
            adopted.late = true;
            if self.targeted.len() >= cap {
                self.cap_exceeded = true;
                continue;
            }
            // Attach the witness before signalling, as in `signal_first`: an adopted member
            // can fork-and-exit on its own SIGTERM too.
            self.track(os, pid);
            let atomic = self.signal(os, handle, pid, sig).ok_or(())?;
            self.atomically_bound &= atomic;
            adopted.signalled.push(pid);
            if let Some(row) = table.iter().find(|r| r.pid == pid) {
                adopted.from.insert(row.ppid);
            }
            push_targeted(&mut self.targeted, pid, cap);
            if sig == libc::SIGKILL {
                push_capped(&mut self.killed, pid, cap);
            }
        }
        Ok(adopted)
    }

    /// Newcomers below the root (only while it is live in `table`), unioned with those below
    /// every targeted member alive with its recorded identity — deduplicated, root itself
    /// never included.
    fn newcomers_from_live(
        &self,
        os: &dyn ProcessOs,
        handle: &ProcessHandle,
        table: &[ProcInfo],
        rooted: bool,
    ) -> Vec<u32> {
        let owned = self.owned(handle);
        let mut found = if rooted {
            newcomers(os, handle.pid, &self.targeted, table)
        } else {
            Vec::new()
        };
        for &member in &self.targeted {
            if !owned.verified_member(os, member) {
                continue;
            }
            for pid in newcomers(os, member, &self.targeted, table) {
                if !found.contains(&pid) {
                    found.push(pid);
                }
            }
        }
        found
    }

    /// Which targeted pids are still the exact instance we signalled, alive, by number.
    fn residual_alive(&self, os: &dyn ProcessOs) -> Vec<u32> {
        self.alive_by_identity(os, &self.targeted)
    }

    /// macOS: was every witnessed fork paid for by an adoption from its forker? A last read
    /// first: a fork after the final poll's reads, like one still pending from them, has no
    /// adoption left to pay it. Vacuous where there are no witnesses.
    fn forks_accounted(&mut self) -> bool {
        let late = self.read_witnesses(&[]);
        if late.iter().any(|&(_, grew)| grew) || !self.pending.is_empty() {
            self.fork_unaccounted = true;
        }
        !self.fork_unaccounted
    }

    /// The fail-closed report for a root lost mid-escalation, carrying what was signalled.
    fn into_containment(
        self,
        os: &dyn ProcessOs,
        handle: &ProcessHandle,
        started: Instant,
    ) -> TerminationReport {
        containment_unverified(
            os,
            handle,
            started,
            self.targeted,
            self.survived_term,
            self.killed,
        )
    }
}

/// What one poll's adoption did: whether anything new was found (a capped newcomer included),
/// the pids it signalled, and the forkers — each newcomer's ppid in the snapshot — it adopted
/// from, which is what pays for a witnessed fork.
#[derive(Default)]
struct Adopted {
    late: bool,
    signalled: Vec<u32>,
    from: BTreeSet<u32>,
}

/// Processes that appeared after the last walk and are still in the ppid subtree of `root`.
///
/// `root` is the namespace being expanded: the registered root while it is live, or a
/// targeted member the caller has just verified alive with its recorded creation identity —
/// the only two things a fresh snapshot may be walked from. `root` itself is never a
/// newcomer: for the registered root, phase B signals it last and the grace sweep must not
/// adopt the very process it is keeping alive; for a member, it is already targeted.
fn newcomers(os: &dyn ProcessOs, root: u32, targeted: &[u32], fresh: &[ProcInfo]) -> Vec<u32> {
    let mut found = Vec::new();
    for pid in descendants(root, fresh) {
        if pid != root
            && pid >= MIN_SIGNALLABLE_PID
            && !targeted.contains(&pid)
            && !found.contains(&pid)
            && alive(os, pid)
        {
            found.push(pid);
        }
    }
    found
}

/// What a sweep that has already signalled may still claim.
///
/// [`verified_snapshot`] answers one question — is the registered root live — and its `None`
/// would stand for two incompatible situations: a root the guard refused, which is loss of
/// authority, and a root that is merely absent, which after this daemon has signalled the
/// tree is the kill working. Conflating them would make [`TerminationReport::observed_gone`]
/// unreachable for every harness of more than one process: a successful kill would destroy
/// the authority needed to observe its own success.
enum Anchor {
    /// The root is live, and this is its current process table. The walk can enumerate the
    /// tree and adopt whatever has appeared below it since the last pass.
    Rooted(Vec<ProcInfo>),
    /// The root is gone, and this is the table without it. Nothing can be named from the
    /// root, but every targeted member alive with its recorded creation identity still owns
    /// its ppid subtree in this table — what it forks after the root's death is adopted from
    /// it, and it can be signalled — and every targeted pid can still be asked, by number,
    /// whether it is alive, which is the question the final sweep exists to answer.
    Closed(Vec<ProcInfo>),
    /// The guard refused the root, or the table could not be read. Nothing may be claimed
    /// about anything, and the caller's fail-closed return is the whole content of that.
    Lost,
}

/// The anchor for a sweep that has **already signalled** at least one member of the owned set.
///
/// Only sound after a signal. Before one, an absent root is [`absent_root_unverified`]: the
/// participant quit hours ago, a wrapper may have reparented a native child, and no part of
/// that is a termination this daemon may take credit for.
fn sweep_anchor(os: &dyn ProcessOs, handle: &ProcessHandle) -> Anchor {
    let live = match guard_registered(os, handle) {
        Ok(Presence::Live) => true,
        Ok(Presence::Absent) => false,
        Err(_) => return Anchor::Lost,
    };
    match os.snapshot() {
        // Live when the guard asked and absent from the table read a moment later: it exited
        // between the two reads, which is the same closed set as an absent root.
        Ok(table) if live && table.iter().any(|row| row.pid == handle.pid) => Anchor::Rooted(table),
        Ok(table) => Anchor::Closed(table),
        Err(_) => Anchor::Lost,
    }
}
