#[cfg(test)]
use super::MAX_TARGETED;
use super::os::{ProcessOs, alive, waited_ms};
use super::{Anchor, MIN_SIGNALLABLE_PID, TerminationReport, sweep_anchor};
use goalcup_protocol::process::{
    ProcInfo, ProcessError, ProcessHandle, descendants, find_named, native_pid_is_bound_to_handle,
};
use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::time::Instant;

/// The owned set below the registered root, ordered **deepest level first, native hoisted
/// within its own depth**, with the root itself deliberately excluded.
///
/// Deepest first means a parent is never ended before the children that hang off it, so
/// nothing is orphaned out of the walk by our own signal. Promoting the native process to the
/// front of the entire list would signal an *interior* native node (the Codex shape:
/// `node → codex → shell → child`) before its own children; they would reparent to pid 1 and
/// leave the ppid walk before the next signal could reach them, and the escalation would
/// abort `Unresolved` with live processes it never named. The native promotion is therefore
/// only a tie-break *within* a depth — it decides who dies first among siblings, never a
/// parent before a child.
///
/// The root is excluded because it is signalled last, by [`super::terminate_with`], only once
/// this set has been observed gone or its sweep exhausted; keeping the root alive is what
/// keeps every sweep [`Anchor::Rooted`] while the descendants die.
pub(super) fn members_deepest_first(handle: &ProcessHandle, table: &[ProcInfo]) -> Vec<u32> {
    let root = handle.pid;
    let order = descendants(root, table); // BFS, root first, parents before children.

    // Depth from the root, computed off the same BFS order: a parent is always seen before
    // its child, so its depth is known by the time the child is reached.
    let mut depth: BTreeMap<u32, usize> = BTreeMap::new();
    depth.insert(root, 0);
    for &pid in &order {
        if pid == root {
            continue;
        }
        if let Some(row) = table.iter().find(|r| r.pid == pid) {
            let d = depth.get(&row.ppid).copied().map_or(1, |dp| dp + 1);
            depth.insert(pid, d);
        }
    }

    let mut members: Vec<u32> = order
        .into_iter()
        .filter(|p| *p != root && *p >= MIN_SIGNALLABLE_PID)
        .collect();
    // Deepest first. `sort_by` is stable, so the BFS relative order is preserved within a
    // depth (and the native tie-break below is applied on top of it).
    members.sort_by(|a, b| depth.get(b).cmp(&depth.get(a)));

    // The native process, re-resolved at seal (it may have appeared after launch), hoisted to
    // the front of its own depth — never ahead of anything deeper.
    let native = native_binary(handle)
        .and_then(|name| find_named(root, name, table))
        .or_else(|| {
            native_pid_is_bound_to_handle(handle, table)
                .then_some(handle.native_pid)
                .flatten()
        })
        .filter(|p| *p >= MIN_SIGNALLABLE_PID);
    if let Some(native) = native
        && let Some(nd) = depth.get(&native).copied()
    {
        members.retain(|p| *p != native);
        // `members` is depth-descending, so its depth-`nd` band begins at the first index no
        // deeper than `nd` — the front of that band, and the right slot for the native even
        // when it is the only process at its depth (where matching `== nd` after removal finds
        // nothing and would wrongly append it last, behind deeper members it must precede).
        let idx = members
            .iter()
            .position(|p| depth.get(p).copied().unwrap_or(0) <= nd)
            .unwrap_or(members.len());
        members.insert(idx, native);
    }
    members
}

/// Adopt one newcomer, up to [`MAX_TARGETED`]. `false` when the ceiling refused it.
///
/// A silent ceiling rather than a refusal: past it the escalation stops growing and the
/// pids already targeted are still chased, which is a bounded sweep over a tree that is
/// forking faster than a 50 ms poll can name it. The alternative — an unbounded push — is
/// a quadratic sweep and a report a status document has to carry.
///
/// The boolean is what keeps a capped pid from being signalled on every pass: it is not in
/// `targeted`, so `newcomers` keeps finding it, and a caller that signalled before checking
/// would send it a fresh signal twenty times a second for the length of the sweep.
pub(super) fn push_targeted(targeted: &mut Vec<u32>, pid: u32, cap: usize) -> bool {
    push_capped(targeted, pid, cap)
}

/// One pid onto a bounded report list. [`TerminationReport::killed`] and
/// [`TerminationReport::survived_term`] are read by the seal and serialised into it, so
/// each of them is held to the same ceiling as `targeted` — a list without one is a seal
/// manifest whose size is chosen by whatever the harness was doing while it died.
pub(super) fn push_capped(list: &mut Vec<u32>, pid: u32, cap: usize) -> bool {
    if list.len() >= cap {
        return false;
    }
    list.push(pid);
    true
}

/// Bound the initial discovery before any signal or report allocation uses it.
pub(super) fn bounded_targets(mut targets: Vec<u32>, cap: usize) -> (Vec<u32>, bool) {
    let exceeded = targets.len() > cap;
    targets.truncate(cap);
    (targets, exceeded)
}

/// Is `pid` still the exact instance we signalled — alive, and not a recycled stranger?
///
/// Every question the escalation asks of a targeted pid goes through here — the phase gate,
/// the SIGKILL selection, the adoption walk's choice of forkers, the final `still_alive` —
/// and none of them filters by the current lineage. A member whose parent died first has
/// been reparented to pid 1 and has left the root's ppid walk while still alive; a
/// lineage-filtered check would drop it, end the root over it, and call the tree gone over
/// it. Asking by number instead can only ever over-report — the safe direction — and the
/// recorded process-start identity is what keeps a number recycled since the signal from
/// reading as our member still running. `expected == None` (no identity captured) falls back
/// to bare liveness, which still over-reports rather than under-reports.
pub(super) fn instance_alive(os: &dyn ProcessOs, pid: u32, expected: Option<&str>) -> bool {
    if !alive(os, pid) {
        return false;
    }
    match expected {
        Some(expected) => os.start_identity(pid).as_deref() == Some(expected),
        None => true,
    }
}

/// The fail-closed report for an argv-renamed root.
///
/// **Nothing is walked and nothing is signalled.** An argv mismatch means the daemon cannot
/// authenticate the root that the handle's group number came from. `getpgid(pid) == pid`
/// would only authenticate the *current* leader at that recycled number, so it cannot turn
/// a stale group into authority either.
///
/// **`all_gone` is `false`, always.** `ProcInfo` carries no pgid, so there is no snapshot
/// this module can take that says whether the group is empty; a report that claimed the
/// tree was gone off a signal it could not verify would be a false containment claim in a
/// new place. `harness_unverified` is how the seal and the status document say so instead.
pub(super) fn identity_unverified(
    os: &dyn ProcessOs,
    handle: &ProcessHandle,
    started: Instant,
) -> TerminationReport {
    TerminationReport {
        strategy: handle.strategy,
        targeted: Vec::new(),
        survived_term: Vec::new(),
        killed: Vec::new(),
        still_alive: Vec::new(),
        all_gone: false,
        observed_gone: false,
        walk_cap_exceeded: false,
        waited_ms: waited_ms(os, started),
        guard_degraded: true,
        harness_unverified: true,
    }
}

/// A process table that no longer contains the registered root cannot prove whether a
/// wrapper left a native process behind.  The ordinary seal remains durable, but its
/// termination report is intentionally non-successful: no PID/group is signalled and no
/// consumer may read this as verified containment.
pub(super) fn absent_root_unverified(
    os: &dyn ProcessOs,
    handle: &ProcessHandle,
    started: Instant,
) -> TerminationReport {
    TerminationReport {
        strategy: handle.strategy,
        targeted: Vec::new(),
        survived_term: Vec::new(),
        killed: Vec::new(),
        still_alive: Vec::new(),
        all_gone: false,
        observed_gone: false,
        walk_cap_exceeded: false,
        waited_ms: waited_ms(os, started),
        guard_degraded: degraded_guard(handle),
        harness_unverified: true,
    }
}

/// The owned set, as the proof taken immediately before a signal sees it.
///
/// Two ways for a pid to be ours in a fresh table. Under a live root, anything in the root's
/// ppid tree — the walk. Under any anchor, anything in the ppid tree of a targeted member
/// (the member itself included) that is alive with the creation identity recorded when it
/// was targeted: the member the walk lost when its parent died first, whatever it has forked
/// since, and, once the root is gone, every live member at all. This second way is what lets
/// an orphan be SIGKILLed, keeps the root unsignalled while an orphan lives, and makes the
/// adoption walk from live members signal what it finds. A member whose identity was never
/// captured is ours only through the root.
pub(super) struct Owned<'a> {
    pub(super) root: u32,
    pub(super) targeted: &'a [u32],
    pub(super) instances: &'a BTreeMap<u32, Option<String>>,
}

impl Owned<'_> {
    /// Is `member` a targeted non-root pid alive with its recorded creation identity — a pid
    /// whose ppid subtree in a fresh table is ours to walk and to signal?
    pub(super) fn verified_member(&self, os: &dyn ProcessOs, member: u32) -> bool {
        member != self.root
            && self
                .instances
                .get(&member)
                .and_then(|i| i.as_deref())
                .is_some_and(|expected| instance_alive(os, member, Some(expected)))
    }

    /// Is `pid` ours in `table`? `rooted` says the registered root is live and in it.
    pub(super) fn contains(
        &self,
        os: &dyn ProcessOs,
        table: &[ProcInfo],
        rooted: bool,
        pid: u32,
    ) -> bool {
        if rooted && descendants(self.root, table).contains(&pid) {
            return true;
        }
        self.targeted.iter().any(|&member| {
            self.verified_member(os, member) && descendants(member, table).contains(&pid)
        })
    }
}

/// Re-authenticate the root, take a fresh table, and prove `pid` is ours in it immediately
/// before sending a direct signal. A targeted number is never a durable authority — it may
/// have exited and been recycled since the walk that named it — so the proof is taken every
/// time, as the last thing before the `kill`.
///
/// `Some(atomic)` when the signal went, or when `pid` had already gone: that needs no signal
/// and leaves the owned set accounted for, and it is `Some(false)` because nothing
/// instance-bound was delivered. `None` when nothing may be claimed — the root was lost, or
/// `pid` is alive and ours by nothing in the fresh table, and a bare number is not authority
/// to signal anything.
pub(super) fn signal_owned(
    os: &dyn ProcessOs,
    handle: &ProcessHandle,
    owned: &Owned<'_>,
    pid: u32,
    sig: i32,
) -> Option<bool> {
    // Acquire the kernel object first; a PID number is never retained as the authority to
    // signal. A number recycled between here and the proof below reaches the old instance
    // (`ESRCH`) and never the new one. Linux binds a pidfd; macOS has no pidfd-class signal
    // capability and binds nothing, so it takes the numeric path, where the proof is
    // deliberately the last operation before `kill` and is not atomic with it — which is
    // what keeps `all_gone` false there.
    let bound = os.bind(pid);
    let (table, rooted) = match sweep_anchor(os, handle) {
        Anchor::Rooted(table) => (table, true),
        Anchor::Closed(table) => (table, false),
        Anchor::Lost => return None,
    };
    if !owned.contains(os, &table, rooted, pid) {
        return (!alive(os, pid)).then_some(false);
    }
    match bound {
        Some(mut target) => target.signal(sig).then_some(true),
        None => {
            os.kill(pid, sig);
            Some(false)
        }
    }
}

/// Re-authenticate the root and its private process group immediately before a group
/// signal. A durable `pgid` field alone is a bare number after the root has gone.
///
/// # Why this one stays fail-closed when the sweep does not
///
/// The sweep above outlives the root, because every pid it asks about was named while the
/// root was alive. A group cannot be enumerated that way: `ProcInfo` carries no pgid, so a
/// member that never appeared in the walk — an orphan that left the lineage without leaving
/// the group — is reachable by `killpg` and by nothing else.
/// Once the leader is gone that signal cannot be sent (the pgid may already name a stranger's
/// group) and that member cannot be named, so there is nothing left to observe and the
/// honest report is the fail-closed one.
///
/// The two paths cannot both survive the root: whichever signal goes first ends the leader,
/// and the other then has no authority. That is a real limit of an unprivileged tracker on a
/// tree it did not birth, and it reaches **no shipped connector** — every one is
/// `mode = "tui"`, `owns_its_group` is false for a TUI, and every registered handle therefore
/// carries `pgid: None`, so `group` is `None` and this function is never called on a real
/// run. A connector that does register a leading pgid gets the conservative answer.
pub(super) fn signal_registered_group(
    os: &dyn ProcessOs,
    handle: &ProcessHandle,
    sig: i32,
) -> bool {
    if !owns_registered_group(os, handle) {
        return false;
    }
    os.killpg(handle.pid, sig);
    true
}

/// A durable report for a termination whose original targets were safe when signalled, but
/// whose root is no longer safe to use for another traversal, PID signal, or group signal.
pub(super) fn containment_unverified(
    os: &dyn ProcessOs,
    handle: &ProcessHandle,
    started: Instant,
    targeted: Vec<u32>,
    survived_term: Vec<u32>,
    killed: Vec<u32>,
) -> TerminationReport {
    TerminationReport {
        strategy: handle.strategy,
        targeted,
        survived_term,
        killed,
        still_alive: Vec::new(),
        all_gone: false,
        observed_gone: false,
        walk_cap_exceeded: false,
        waited_ms: waited_ms(os, started),
        guard_degraded: degraded_guard(handle),
        harness_unverified: true,
    }
}

/// Does the live, verified root still lead the private group it created at launch?
pub(super) fn owns_registered_group(os: &dyn ProcessOs, handle: &ProcessHandle) -> bool {
    if handle.pgid != Some(handle.pid) {
        return false;
    }
    if !matches!(guard_registered(os, handle), Ok(Presence::Live)) {
        return false;
    }
    // The exact kernel statement that this still-live root is the leader of its own
    // process group.
    os.leads_group(handle.pid)
}

/// Refuse a pid that cannot be the process the CLI registered.
///
/// Two questions, both asked of the live process and neither sufficient alone.
///
/// **Creation identity.** The CLI records an OS process-instance value immediately after
/// spawn. A PID that now has another creation identity is a stranger even if it reused the
/// same argv. A missing identity is an old readable row, not permission to make a new live
/// signal.
///
/// **What it is running.** The argv check is a second independent binding. `ps -o args=` is
/// where the live process states that, and `handle.argv` is what the CLI spawned.
pub(super) fn guard_registered(
    os: &dyn ProcessOs,
    handle: &ProcessHandle,
) -> Result<Presence, Refused> {
    // `ps -o etime,args` can still print an unreaped zombie. It is not a live root of
    // signal authority: `liveness` is the shared definition used by seal reporting and
    // excludes `Z`, so take the ordinary absent-root path before accepting its stale row.
    //
    // **A failed spawn is "could not ask", never "gone".** `liveness` returns `Err` when
    // `/bin/ps` could not be spawned at all. Folding that to "absent" is a false
    // containment claim in a new place: the participant's harness reads as already-quit on any
    // machine that momentarily cannot run `ps`. It propagates as [`Refused::Unavailable`],
    // which the initial guard surfaces as the `Err(Spawn)` `os.snapshot()?` would give one
    // line later, and which [`super::sweep_anchor`] folds to [`super::Anchor::Lost`].
    match os.liveness(handle.pid) {
        Ok(true) => {}
        Ok(false) => return Ok(Presence::Absent),
        Err(e) => return Err(Refused::Unavailable(e)),
    }
    let Some((_, args)) = os.identity(handle.pid) else {
        // Not in the process table at all — the ordinary seal, where the participant
        // already quit. There is nothing to mistake for the harness and nothing to kill.
        //
        // Said as `Absent` rather than as a bare `Ok`: neither question below was asked,
        // so the caller has been told nothing about this number's identity, and the
        // process group it names is not a target it may take on trust.
        return Ok(Presence::Absent);
    };
    let expected_start = handle.process_start.as_deref();
    let observed_start = os.start_identity(handle.pid);
    // **A root that exited between the reads is gone, not recycled.** `liveness` above said
    // live; an exit after it (a `sh -c 'worker & wait'` root quits the instant its worker is
    // signalled) leaves this read with nothing to return, and `None` would then fail the
    // comparison below and be reported as a stranger's number. The question that tells the
    // two apart is the one already asked: ask it again. Not in the table now is the
    // ordinary absent root, exactly as if the first read had come a moment later. A number
    // that is still live with no readable identity stays refused, and so does any
    // *different* identity: only "no longer live" is read as an exit, and a pid recycled
    // between the reads is live (or again unreadable) and fails the same check.
    if observed_start.is_none() {
        match os.liveness(handle.pid) {
            Ok(false) => return Ok(Presence::Absent),
            Ok(true) => {}
            Err(e) => return Err(Refused::Unavailable(e)),
        }
    }
    if expected_start.is_none() || expected_start != observed_start.as_deref() {
        return Err(Refused::Recycled(ProcessError::NoTarget(format!(
            "pid {} no longer has the registered process-start identity {:?} (observed {:?}); \
             it may be recycled and will not be signalled",
            handle.pid, expected_start, observed_start
        ))));
    }
    if !argv_matches(handle, &args) {
        return Err(Refused::ArgvRenamed);
    }
    Ok(Presence::Live)
}

/// What [`guard_registered`] found at the registered number: the launch's own process,
/// verified twice over, or nothing at all.
///
/// The two are not the same permission. `Live` is a pid this module has just checked the
/// age and the argv of, and the process group such a pid leads is the group the launch
/// created. `Absent` is a number the process table does not hold — so it is not leading
/// anything, and the group id beside it on the handle is a number that may since have been
/// handed to a stranger who made themselves a leader with it.
pub(super) enum Presence {
    /// In the table, with the registered creation identity and argv.
    Live,
    /// Not in the table at all: the harness exited, which is the ordinary seal.
    Absent,
}

/// Which of [`guard_registered`]'s two questions refused. An argv mismatch has its own
/// variant so callers can preserve the fail-closed report rather than treating it as an I/O
/// failure; neither kind of refusal grants a bare PID or PGID new authority.
pub(super) enum Refused {
    /// The number is another process's. Nothing about this handle names a target.
    Recycled(ProcessError),
    /// The program the live pid runs is not the one registered.
    ArgvRenamed,
    /// The process table could not be asked at all — `/bin/ps` failed to spawn. This is not
    /// a ruling on the pid; it is the absence of one, and "could not ask" must never read as
    /// "the participant quit". The carried error is the `Err(Spawn)` the caller
    /// re-raises.
    Unavailable(ProcessError),
}

/// Was the identity half of [`guard_registered`] unable to run at all?
///
/// True when the handle's `argv` states no program to compare against — an empty list, or a
/// first element that is not a path with a file name. [`argv_matches`] answers `true` in
/// that case, which is right (there is nothing to be inconsistent with) and is also
/// indistinguishable, from the outside, from a live process that matched. This says which
/// of the two it was.
///
/// Deliberately not set when `ps` returned an empty argument line: that is a process which
/// left between the snapshot and the check, which is the ordinary seal, and the guard was
/// present and had nothing to rule on.
pub(super) fn degraded_guard(handle: &ProcessHandle) -> bool {
    handle.argv.first().and_then(|a| file_name(a)).is_none()
}

/// Does the live process's `ps -o args=` name what the launch registered?
///
/// The comparison is the *program*: the file name of `argv[0]`, appearing anywhere among
/// the live arguments. Not whole-line equality — `ps` re-renders an argv, collapsing the
/// separators and truncating a long line, and a launch that had appended a resumed thread
/// id would then read as a stranger — and not `handle.program`, which for a wrapped
/// connector names the native binary rather than the process the CLI actually spawned.
///
/// Two things are deliberately not refusals. An empty `argv` states nothing to compare, so
/// there is nothing to be inconsistent with; and an empty `ps` line is a process that left
/// between the snapshot and this call, which is the ordinary seal and not a stranger.
pub(super) fn argv_matches(handle: &ProcessHandle, args: &str) -> bool {
    let Some(wanted) = handle.argv.first().and_then(|a| file_name(a)) else {
        return true;
    };
    if args.is_empty() {
        return true;
    }
    args.split_whitespace()
        .filter_map(file_name)
        .any(|token| token == wanted)
}

fn file_name(path: &str) -> Option<&str> {
    std::path::Path::new(path).file_name()?.to_str()
}

/// The manifest-pinned native identity carried in the launch handle.
///
/// `program` is the resolved launcher path and may be a shell/npm wrapper.  It must not be
/// repurposed as the native identity: doing so turns a valid `sh -> sleep` or wrapper ->
/// native handoff into a rejection while still leaving same-name strangers plausible.
pub(super) fn native_binary(handle: &ProcessHandle) -> Option<&str> {
    handle.native_binary.as_deref()
}

/// How long `pid` has been running and what it is running, or `None` when it is not in the
/// process table.
///
/// One `ps` for both, because the guard asks the same question twice and a second spawn
/// would let the answers come from two different instants.
///
/// `etime` and not `lstart`: elapsed time carries no timezone, and the local-time string
/// `lstart` prints cannot be compared with a UTC `launched_at` without an offset this crate
/// has no library to compute. Formats are `mm:ss`, `hh:mm:ss` and `dd-hh:mm:ss` on both
/// macOS and Linux.
pub(super) fn live_identity(pid: u32) -> Option<(u64, String)> {
    let out = Command::new("/bin/ps")
        .args(["-o", "etime=,args=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    split_identity(&String::from_utf8_lossy(&out.stdout))
}

/// `"    01:30 /bin/sh ./wrapper.sh"` → `(90, "/bin/sh ./wrapper.sh")`.
///
/// A kernel thread prints an elapsed time and no arguments at all; that is an empty argv,
/// not an unreadable line, and the guard treats it as nothing to compare rather than as a
/// mismatch.
fn split_identity(line: &str) -> Option<(u64, String)> {
    let line = line.lines().next()?.trim();
    let (etime, args) = match line.split_once(char::is_whitespace) {
        Some((etime, rest)) => (etime, rest.trim()),
        None => (line, ""),
    };
    Some((parse_etime(etime)?, args.to_string()))
}

fn parse_etime(s: &str) -> Option<u64> {
    let text = s.trim();
    if text.is_empty() {
        return None;
    }
    let (days, clock) = match text.split_once('-') {
        Some((d, rest)) => (d.trim().parse::<u64>().ok()?, rest),
        None => (0, text),
    };
    let fields: Vec<&str> = clock.split(':').collect();
    if fields.is_empty() || fields.len() > 3 {
        return None;
    }
    let mut seconds = days.checked_mul(86_400)?;
    // Least significant first, so the same parser reads all three widths.
    for (i, field) in fields.iter().rev().enumerate() {
        let value: u64 = field.trim().parse().ok()?;
        seconds = seconds.checked_add(value.checked_mul(60u64.pow(i as u32))?)?;
    }
    Some(seconds)
}

#[cfg(test)]
mod tests {
    use super::{MAX_TARGETED, argv_matches, bounded_targets, parse_etime, split_identity};
    use goalcup_protocol::manifest::KillStrategy;
    use goalcup_protocol::process::ProcessHandle;

    fn handle(argv: &[&str]) -> ProcessHandle {
        ProcessHandle {
            pid: 4_242,
            pgid: None,
            native_pid: None,
            native_binary: None,
            native_process_start: None,
            process_start: None,
            strategy: KillStrategy::NativePid,
            grace_seconds: 1,
            launched_at: "2026-09-04T00:00:00Z".to_string(),
            program: "codex".to_string(),
            argv: argv.iter().map(|a| a.to_string()).collect(),
            env_keys: Vec::new(),
        }
    }

    /// The two columns the guard reads out of one `ps` line, and the line a kernel thread
    /// prints — an elapsed time and no arguments at all.
    #[test]
    fn one_ps_line_carries_both_halves_of_the_guard() {
        assert_eq!(
            split_identity("    01:30 /bin/sh ./wrapper.sh\n"),
            Some((90, "/bin/sh ./wrapper.sh".to_string()))
        );
        assert_eq!(
            split_identity("07-19:55:32 sleep 300"),
            Some((7 * 86_400 + 71_732, "sleep 300".to_string()))
        );
        assert_eq!(split_identity("  00:04\n"), Some((4, String::new())));
        assert_eq!(split_identity(""), None);
        assert_eq!(split_identity("  nope  args\n"), None);
    }

    /// What the argv half accepts, and the reused pid it does not. The comparison is the
    /// program `argv[0]` names, wherever `ps` renders it in the line.
    #[test]
    fn a_live_process_is_matched_by_the_program_its_argv_names() {
        let h = handle(&["/opt/homebrew/bin/codex", "--model", "o3"]);
        assert!(argv_matches(&h, "/opt/homebrew/bin/codex --model o3"));
        // `ps` re-renders argv: the path resolved, the flags rewritten, the line truncated.
        assert!(argv_matches(&h, "codex --model o3 --resume 0f2c"));
        // The reused pid: same number, a program the launch never registered.
        assert!(!argv_matches(&h, "/usr/bin/vim notes.md"));
        // A prefix is not the program: `codex-helper` is a different binary.
        assert!(!argv_matches(&h, "codex-helper --serve"));
        // Nothing registered and nothing readable are both "no answer", not "mismatch".
        assert!(argv_matches(&handle(&[]), "/usr/bin/vim notes.md"));
        assert!(argv_matches(&h, ""));
    }

    /// The three widths `ps` prints, and the shapes it never does.
    #[test]
    fn elapsed_time_is_read_in_all_three_widths() {
        assert_eq!(parse_etime("00:00\n"), Some(0));
        assert_eq!(parse_etime("   01:30 "), Some(90));
        assert_eq!(parse_etime("01:00:00"), Some(3_600));
        assert_eq!(parse_etime("07-19:55:32"), Some(7 * 86_400 + 71_732));
        assert_eq!(parse_etime(""), None);
        assert_eq!(parse_etime("  "), None);
        assert_eq!(parse_etime("1:2:3:4"), None);
        assert_eq!(parse_etime("nope"), None);
    }

    #[test]
    fn initial_process_discovery_is_bounded_before_signalling() {
        let (targets, exceeded) = bounded_targets(
            (2..u32::try_from(MAX_TARGETED + 4).unwrap()).collect(),
            MAX_TARGETED,
        );
        assert!(exceeded);
        assert_eq!(targets.len(), MAX_TARGETED);

        let (targets, exceeded) = bounded_targets(vec![2, 3], MAX_TARGETED);
        assert!(!exceeded);
        assert_eq!(targets, vec![2, 3]);

        // A caller's own budget tightens the ceiling and is reported when it bites.
        let (targets, exceeded) = bounded_targets(vec![2, 3, 4], 2);
        assert!(exceeded);
        assert_eq!(targets, vec![2, 3]);
    }
}
