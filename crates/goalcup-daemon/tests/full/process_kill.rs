//! Terminating the harness process tree at seal (D22).
//!
//! Every case here runs against a real process tree built out of `/bin/sh` and `sleep`.
//! No harness is launched and no manifest is read: what is under test is the walk, the
//! escalation and the verification, and a fixture that only pretended to fork would prove
//! none of them.
//!
//! The fixtures are `#!/bin/sh` scripts and the assertions read a `ps` snapshot, so the
//! whole file is unix-only. An ungated test here would fail to compile rather than be skipped.
use goalcup_daemon::terminate::terminate;
use goalcup_protocol::manifest::KillStrategy;

// The process-group escape helper (`tests/common/support/tree.rs`). The `mod support` below
// is this file's own, not the shared one.
use crate::common::support::tree;

/// Darwin has no pidfd, but it does have an instance-bound process audit token. A direct
/// TUI launch with no process group uses that authority to terminate the exact root.
///
/// The seal-time authority cannot establish that the root never left an orphan before it
/// was acquired, so exact root exit evidence must not be promoted to full-tree verification.
#[cfg(target_os = "macos")]
#[test]
fn a_darwin_native_process_is_signalled_exactly_but_the_tree_stays_unverified() {
    let child = support::spawn_fixture("wrapper-exec.sh");
    let mut handle = support::handle(child.pid, KillStrategy::NativePid, "sleep");
    handle.pgid = None;
    handle.argv = vec!["/bin/sleep".to_string(), "300".to_string()];

    let report = terminate(&handle).expect("the instance-bound Darwin target terminates");

    assert_eq!(report.targeted, vec![child.pid]);
    assert!(
        !report.all_gone,
        "seal-time root evidence cannot prove the tree's prehistory: {report:?}"
    );
    assert!(
        report.harness_unverified,
        "the process root is gone, but a preexisting orphan remains possible: {report:?}"
    );
    assert!(!goalcup_protocol::process::is_alive(child.pid));
}

/// The pid-file and reparent checks are barriers: before termination starts, the registered
/// root has already forked a child through an intermediate shell, that child has reparented
/// outside the ppid walk, and the root has exec'd into a current singleton. A watcher
/// installed at seal sees no fork and proves exact root exit, but cannot prove this orphan
/// absent. This is the complete-prehistory hole behind the macOS lifecycle contract.
#[cfg(target_os = "macos")]
#[test]
fn a_preexisting_orphan_is_not_hidden_by_a_clean_darwin_singleton_exit() {
    let dir = tempfile::tempdir().expect("a tempdir for the orphan barrier");
    let pidfile = dir.path().join("preexisting-orphan.pid");
    let child = support::spawn_fixture_args("wrapper-preexisting-orphan.sh", &[&pidfile]);
    let orphan = support::read_pid(&pidfile);
    support::wait_until_reparented(child.pid, orphan);
    assert_eq!(
        goalcup_protocol::process::descendants(child.pid, &support::ps()),
        vec![child.pid],
        "the authorizing seal snapshot must look exactly like a singleton"
    );

    let mut handle = support::handle(child.pid, KillStrategy::NativePid, "sleep");
    handle.pgid = None;
    handle.argv = vec!["/bin/sleep".to_string(), "300".to_string()];
    let report = terminate(&handle).expect("the exact root remains safe to terminate");

    assert!(!goalcup_protocol::process::is_alive(child.pid));
    assert!(
        goalcup_protocol::process::is_alive(orphan),
        "the barrier did not create the preexisting orphan the seal-time watcher misses"
    );
    assert!(report.harness_unverified && !report.all_gone, "{report:?}");
}

#[test]
fn a_forking_wrapper_does_not_leave_its_grandchild_holding_the_lock() {
    let child = support::spawn_fixture("wrapper-fork.sh"); // forks a grandchild, waits
    let tree = goalcup_protocol::process::descendants(child.pid, &support::ps());
    assert!(
        tree.len() >= 2,
        "the grandchild is in the tree we discovered, not assumed: {tree:?}"
    );

    let r = terminate(&support::handle(
        child.pid,
        KillStrategy::NativePid,
        "sleep",
    ))
    .unwrap();
    assert!(r.harness_unverified, "lost root must end the chase: {r:?}");
    assert!(!r.all_gone);
    // A wrapper can exit as soon as its native child receives SIGTERM, so the fail-closed
    // chase may stop before recording the wrapper itself. The fixture's pre-signal tree is
    // the non-vacuous proof that no original member holding the lock survived.
    for pid in tree {
        assert!(!goalcup_protocol::process::is_alive(pid));
    }
}

#[test]
fn a_tui_launch_with_no_process_group_uses_the_descendant_walk() {
    let child = support::spawn_fixture("wrapper-exec.sh");
    let mut h = support::handle(child.pid, KillStrategy::ProcessGroup, "sleep");
    h.pgid = None; // a TUI cannot own its group without SIGTTIN on the first tty read
    let r = terminate(&h).unwrap();
    assert_eq!(r.strategy, KillStrategy::ProcessGroup);
    if cfg!(target_os = "macos") {
        assert!(r.harness_unverified && !r.all_gone, "{r:?}");
    } else {
        assert!(r.all_gone, "survivors: {:?}", r.still_alive);
        assert!(!r.harness_unverified, "{r:?}");
    }
    assert!(
        !r.targeted.is_empty(),
        "a silent no-op here is exactly the M0 §2.5 failure"
    );
}

/// The shape measured in a live Codex session: almost everything the
/// agent actually starts leaves the launch's process group.
///
/// | process | pgid | in the launch's group |
/// |---|---|---|
/// | `goalcup start` (registered root) | 88942 | leader |
/// | `node .../bin/codex` (wrapper) | 88942 | yes |
/// | native `codex` | 88942 | yes |
/// | MCP server | 89372 | no |
/// | `codex-code-mode-host` | 90855 | no |
/// | the `exec` tool's own children | 90880 | no |
///
/// So `killpg` on the registered group reaches the wrapper and the native binary and
/// nothing else, and the descendant walk carries the whole termination guarantee. It is
/// not a belt-and-braces supplement to the group signal; on the reference harness it is the
/// only thing that reaches the working set.
///
/// What this pins is the pair of properties that makes that walk sound: the helper is still
/// a descendant (so the ppid walk names it) and it is no longer in the launch's group (so a
/// termination that had quietly become group-only would leave it running and pass every
/// other fixture in this file). The group id is read from the kernel rather than assumed,
/// because a fixture whose helper stayed in the group would prove nothing at all.
///
/// Bound by lineage, never by name. The session also had an unrelated `codex-code-mode-host`
/// belonging to the user's own ChatGPT desktop app running beside the tracked session; any
/// matching on `comm` would have swept up a stranger's process.
#[test]
fn a_grandchild_in_its_own_process_group_is_still_named_by_the_descendant_walk() {
    let helper = tree::hardlink_codex_helper("setpgid_grandchild");
    let dir = tempfile::tempdir().expect("a tempdir for the escaped grandchild's pid");
    let pidfile = dir.path().join("setpgid-grandchild.pid");
    let child = support::spawn_fixture_args("wrapper-setpgid-grandchild.sh", &[&helper, &pidfile]);
    let escaped = support::read_pid(&pidfile);
    let _cleanup = support::KillOnDrop(vec![escaped]);

    let table = support::ps();
    assert!(
        goalcup_protocol::process::descendants(child.pid, &table).contains(&escaped),
        "the helper must remain in the lineage or this fixture tests nothing"
    );
    assert_ne!(
        support::pgid_of(escaped),
        support::pgid_of(child.pid),
        "the helper must have left the launch's process group or this fixture is vacuous"
    );

    let r = terminate(&support::handle(
        child.pid,
        KillStrategy::ProcessGroup,
        "sleep",
    ))
    .expect("the tree terminates");

    assert!(
        r.targeted.contains(&escaped),
        "the group-escaped helper must be in the set the report speaks about: {r:?}"
    );
    support::wait_until_gone(escaped);
    // macOS cannot bind a numeric signal to a process instance, so no tree here is ever
    // reported `all_gone`; what the walk did reach is the claim this test makes.
    if cfg!(target_os = "macos") {
        assert!(!r.all_gone, "{r:?}");
    }
}

/// Row 1 — the measured shape itself:
///
///   `wrapper-native-interior.sh` (root, registered)
///     `codex` (native, interior — comm == "codex")
///       three children, each in a process group of its own
///
/// `discover` promotes the
/// `native_binary` match to the front of the target list, and here that match is the
/// *parent* of every remaining target. Signalling it first orphans the rest — they leave
/// the ppid walk before `signal_descendant` gets to them, and the very next call returns
/// `None` (the target no longer under `descendants(handle.pid, …)`), which aborts the
/// whole escalation through the early `containment_unverified` return.
#[test]
fn a_native_binary_interior_to_the_root_does_not_orphan_its_own_children_out_of_the_walk() {
    let codex_bin = tree::hardlink_codex_helper("native_interior");
    let dir = tempfile::tempdir().expect("a tempdir for the native-interior fixture");
    let children_pidfile = dir.path().join("children.pid");
    let child = support::spawn_fixture_args(
        "wrapper-native-interior.sh",
        &[&codex_bin, &children_pidfile],
    );

    // Shape proof, before any signal: exactly five processes, and each of the three
    // children is its own process-group leader, read from the kernel rather than assumed.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut tree_now = goalcup_protocol::process::descendants(child.pid, &support::ps());
    while tree_now.len() < 5 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
        tree_now = goalcup_protocol::process::descendants(child.pid, &support::ps());
    }
    assert_eq!(
        tree_now.len(),
        5,
        "shape-proof: root + interior codex + three own-group children: {tree_now:?}"
    );
    let children = support::read_pids(&children_pidfile);
    let _cleanup = support::KillOnDrop(children.clone());
    assert_eq!(
        children.len(),
        3,
        "the native-interior role must report exactly three children: {children:?}"
    );
    for pid in &children {
        assert_eq!(
            support::pgid_of(*pid),
            *pid,
            "child {pid} must lead its own group or this fixture is vacuous"
        );
    }
    let codex_pid = *tree_now
        .iter()
        .find(|p| **p != child.pid && !children.contains(p))
        .expect("the interior codex process must be in the discovered tree");

    let mut h = support::handle(child.pid, KillStrategy::NativePid, "codex");
    h.pgid = None; // the shipped TUI shape: no process group leadership
    h.grace_seconds = 10;

    let r = terminate(&h).expect("the tree terminates");

    let mut expected = vec![child.pid, codex_pid];
    expected.extend(children.iter().copied());
    expected.sort_unstable();
    let mut targeted_sorted = r.targeted.clone();
    targeted_sorted.sort_unstable();
    assert_eq!(
        targeted_sorted, expected,
        "every one of the five measured processes must be targeted, not just the root and \
         the interior native binary that orphaned the rest: {r:?}"
    );
    assert!(
        r.observed_gone,
        "the whole owned set must be confirmed gone by the final sweep: {r:?}"
    );
    assert!(
        !r.harness_unverified,
        "this daemon held the root's identity throughout; the escalation must not have \
         aborted early: {r:?}"
    );
    for pid in &expected {
        assert!(
            !goalcup_protocol::process::is_alive(*pid),
            "pid {pid} outlived a termination over the tree it was discovered in"
        );
    }
}

/// Row 2 — the adoption window:
/// root dies at ~120ms, well inside the initial SIGTERM sweep; the member it left behind
/// does not die until ~400-500ms later, and only then forks a detached orphan. By the time
/// that orphan exists, the root has been gone long enough that the daemon's own authority
/// over it (`Anchor::Rooted`) has already closed, so newcomer-adoption — which needs a
/// fresh `ps` snapshot rooted at a still-live registered root — can never see it appear.
///
/// The safety property this pins is narrower than "the orphan is found": it is that the
/// report may never claim the harness is `observed_gone` while that never-targeted orphan
/// is still alive.
///
/// The member is `tests/common/support/tree.rs`'s `member-forks-after-root` role, not a shell
/// trap: a shell `SIGTERM` delay forks a helper, and that fork is itself a fresh descendant
/// this daemon's own newcomer-adoption can discover and signal mid-delay. See
/// `run_member_forks_after_root_role`'s doc for the mechanism.
#[test]
#[cfg_attr(
    target_os = "linux",
    ignore = "TERMINATION.md: no fork witness on Linux; a fork-and-leave orphan can be missed until the subreaper lands"
)]
fn a_report_does_not_claim_the_harness_gone_while_a_post_root_death_orphan_lives() {
    let member_bin = std::env::current_exe().expect("goalcup fixture: current_exe");
    let dir = tempfile::tempdir().expect("a tempdir for the orphan pidfile");
    let pidfile = dir.path().join("orphan.pid");
    let child = support::spawn_fixture_args("wrapper-root-dies-first.sh", &[&member_bin, &pidfile]);

    // Shape proof, before any signal: the member is root's child in the first snapshot,
    // already running its own `sleep 300` stand-in for the long-lived native session
    // process the measured shape hides (`run_member_forks_after_root_role`'s own first
    // step) — three processes, not two. `spawn_fixture_args` already settled on this, but
    // the count is asserted explicitly rather than assumed from that internal heuristic.
    let tree_now = goalcup_protocol::process::descendants(child.pid, &support::ps());
    assert_eq!(
        tree_now.len(),
        3,
        "shape-proof: root, member, and member's own native-session stand-in, before any \
         of them has been signalled: {tree_now:?}"
    );

    // `native_binary` is deliberately left unset (unlike every other fixture in this file,
    // which passes `"sleep"` through `support::handle`). The member's own native-session
    // stand-in is also a `sleep 300`; naming it would have `discover` hoist that stand-in to
    // the front of the target list, which does not change this fixture's timing (the
    // member role's `SIGTERM` handling is independent of that child's own lifecycle — see
    // `run_member_forks_after_root_role`'s doc) but is not the identity this handle
    // is meant to carry.
    let mut h = support::handle(child.pid, KillStrategy::NativePid, "sleep");
    h.native_binary = None;
    h.pgid = None; // the shipped TUI shape: no process group leadership
    h.grace_seconds = 3;

    let r = terminate(&h).expect("the tree terminates");

    let orphan = support::read_pid(&pidfile);
    let _cleanup = support::KillOnDrop(vec![orphan]);
    assert!(
        !(r.observed_gone && goalcup_protocol::process::is_alive(orphan)),
        "the report claimed the harness observed gone while orphan {orphan}, forked after \
         root's death, is still running: {r:?}"
    );
}

/// Row 3 — a leader that leads its own registered group, and a member that
/// reparents to pid 1 (via an intermediate subshell that exits without waiting) while
/// staying in that same group, because reparenting alone never moves a process out of the
/// group it was forked into. `killpg` on the leader's group still reaches the member — it
/// simply ignores the SIGTERM that delivers, by design (`trap '' TERM` before the member
/// was ever forked, a disposition that survives both `fork` and, unlike a shell function
/// trap, `exec`).
///
/// What this pins is the same safety property as the row above, for a different way to
/// reach the same live-and-never-targeted shape: the ppid walk cannot find the member (it
/// reparented before discovery), and the group signal that can reach it does not confirm
/// delivery. Timing-sensitive only in how long the member takes to reparent and how long
/// the leader takes to exit after its own trap fires; both are driven by polling
/// (`descendants`/`ps`) rather than a fixed sleep.
#[test]
fn a_report_does_not_claim_the_harness_gone_while_a_group_member_outside_the_walk_lives() {
    let dir = tempfile::tempdir().expect("a tempdir for the escaped member's pid");
    let pidfile = dir.path().join("group-member.pid");
    let child = support::spawn_fixture_args("wrapper-group-member-ignores-term.sh", &[&pidfile]);
    let member = support::read_pid(&pidfile);
    let _cleanup = support::KillOnDrop(vec![member]);

    // Shape proof, before any signal, read from the kernel: the member shares the leader's
    // process group, and has already reparented to pid 1 — the escape this fixture exists
    // to build, not an assumption about it.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut ppid = support::ppid_of(member);
    while ppid != Some(1) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
        ppid = support::ppid_of(member);
    }
    assert_eq!(
        ppid,
        Some(1),
        "shape-proof: the member must have reparented to pid 1 or this fixture is vacuous"
    );
    assert_eq!(
        support::pgid_of(member),
        child.pid,
        "shape-proof: the member must still share the leader's process group or this \
         fixture tests nothing"
    );

    let h = support::handle(child.pid, KillStrategy::ProcessGroup, "sleep");
    assert_eq!(
        h.pgid,
        Some(child.pid),
        "the fixture is spawned into its own process group"
    );

    let r = terminate(&h).expect("the tree terminates");

    assert!(
        !(r.observed_gone && goalcup_protocol::process::is_alive(member)),
        "the report claimed the harness observed gone while group member {member}, outside \
         the ppid walk and only ever reachable by the (ignored) group signal, is still \
         running: {r:?}"
    );
}

/// The native process is what holds the writer lock, so it is what the report must name
/// first — the wrapper dying first would orphan it.
#[test]
fn the_process_named_by_the_program_is_targeted_before_the_wrapper_that_hides_it() {
    let child = support::spawn_fixture("wrapper-fork.sh");
    let table = support::ps();
    let native = goalcup_protocol::process::find_named(child.pid, "sleep", &table)
        .expect("the wrapper's child is the `sleep` the handle names");
    assert_ne!(native, child.pid, "the wrapper is not the session");

    let r = terminate(&support::handle(
        child.pid,
        KillStrategy::NativePid,
        "sleep",
    ))
    .unwrap();
    assert_eq!(
        r.targeted.first(),
        Some(&native),
        "D22: the lock holder is signalled first, got {:?}",
        r.targeted
    );
    assert!(
        r.harness_unverified && !r.all_gone,
        "the native child is correctly first, but later root loss must be fail-closed: {r:?}"
    );
}

/// H-10 4b (M3, a pin). The capture spawn gate registers a Codex-family child before it has
/// exec'd, so its handle carries no native hint (`native_pid: None`, `native_process_start:
/// None`) and the daemon must find the native child itself, from a fresh snapshot at seal.
///
/// A handle with no hint, a wrapper that forked its native child after registration, and the
/// walk: the native child is named first, signalled, and observed gone.
#[test]
fn native_pid_strategy_without_hint_targets_live_native_descendant() {
    let child = support::spawn_fixture("wrapper-fork.sh");
    let table = support::ps();
    let native = goalcup_protocol::process::find_named(child.pid, "sleep", &table)
        .expect("the wrapper forked the `sleep` its handle names, after the handle was made");
    let handle = support::handle(child.pid, KillStrategy::NativePid, "sleep");
    assert_eq!(handle.native_pid, None, "a gated handle has no native hint");
    assert_eq!(handle.native_process_start, None);

    let report = terminate(&handle).expect("a hintless NativePid handle still terminates");

    assert_eq!(
        report.targeted.first(),
        Some(&native),
        "the descendant walk found the native child without a hint: {report:?}"
    );
    assert!(
        !goalcup_protocol::process::is_alive(native),
        "the native child the walk named is gone: {report:?}"
    );
    assert!(
        !goalcup_protocol::process::is_alive(child.pid),
        "and so is its wrapper: {report:?}"
    );
}

/// A successful kill must be able to reach `TerminationOutcomeV1::Confirmed` and
/// `observed_gone`.
///
/// A successful kill destroys the authority the sweep is walking from. `wrapper-fork.sh` is
/// the ordinary shape — a launcher whose session is a child — and both processes die on the
/// first SIGTERM. Every poll after that asks `verified_snapshot`, and if that required the
/// registered root to answer `Presence::Live`, the escalation that had just ended the
/// whole tree would return `containment_unverified`: `still_alive` empty, `observed_gone` false,
/// and a seal obliged to say it did not know what it had in fact measured.
///
/// A root that is absent after this daemon signalled it is evidence of the kill, not loss of
/// authority. That distinction is narrow: a root the guard
/// refuses — a recycled number, an argv the launch never registered — is still fail-closed,
/// which `a_same_argv_pid_with_a_different_process_identity_is_never_signalled` and
/// `an_argv_renamed_pid_that_leads_a_group_is_not_signalled_and_is_reported_unverified`
/// continue to pin.
#[test]
fn a_tree_whose_root_dies_with_it_is_observed_gone() {
    let child = support::spawn_fixture("wrapper-fork.sh");
    let tree = goalcup_protocol::process::descendants(child.pid, &support::ps());
    assert!(
        tree.len() > 1,
        "a one-process tree takes the singleton path and would prove nothing here: {tree:?}"
    );

    // `pgid: None` is not a convenience here, it is the shape every shipped V1 connector
    // registers: each is `mode = "tui"`, a TUI cannot own its process group without
    // taking SIGTTIN on its first terminal read, so `owns_its_group` is false and the walk
    // carries the whole termination. A group-leading launch keeps the conservative answer
    // for the reason `signal_registered_group`'s doc gives.
    let mut h = support::handle(child.pid, KillStrategy::NativePid, "sleep");
    h.pgid = None;

    let r = terminate(&h).expect("the tree terminates");

    assert!(
        r.observed_gone,
        "the whole owned set died and the final sweep must be allowed to say so: {r:?}"
    );
    assert!(
        r.still_alive.is_empty() && !r.walk_cap_exceeded,
        "nothing survived and the walk was never capped: {r:?}"
    );
    for pid in tree {
        assert!(
            !goalcup_protocol::process::is_alive(pid),
            "pid {pid} outlived a termination the report calls observed gone"
        );
    }
}

/// `harness_unverified` is the field a signed status document turns into "this run needs
/// review" (`commands/status.rs`), and its own doc says what it means: the daemon lost the
/// root's identity, so it cannot say what remains of the harness.
///
/// It is not `!atomically_bound || target_limit_exceeded`: on macOS `atomically_bound` is
/// false for every tree of more than one process — a POSIX
/// numeric `kill` is not instance-bound and `signal_descendant` says so by returning
/// `Some(false)`. So a termination that enumerated the owned set, ended it, and watched it go
/// would describe itself as a harness this daemon could not verify, beside `Confirmed` on
/// the same status document.
///
/// Neither fact is lost. Atomicity is exactly what separates
/// `all_gone` from `observed_gone`, a capped walk is `walk_cap_exceeded`, and
/// `termination_summary` puts all of them in the durable record beside the verdict.
#[test]
fn a_kill_that_kept_its_root_is_not_reported_as_one_that_lost_it() {
    let child = support::spawn_fixture("wrapper-fork.sh");
    let tree = goalcup_protocol::process::descendants(child.pid, &support::ps());
    assert!(
        tree.len() > 1,
        "a singleton would prove nothing here: {tree:?}"
    );

    let mut h = support::handle(child.pid, KillStrategy::NativePid, "sleep");
    h.pgid = None; // the shipped TUI shape, as above
    let r = terminate(&h).expect("the tree terminates");

    assert!(
        r.observed_gone,
        "the precondition for this test is a completed sweep: {r:?}"
    );
    assert!(
        !r.harness_unverified,
        "this daemon held the root's identity from the first signal to the last sweep, and \
         saying otherwise puts every clean run under review: {r:?}"
    );
}

/// The group path, on a launch that really does lead its own group.
#[test]
fn a_launch_that_leads_its_own_group_is_killed_as_a_group() {
    let child = support::spawn_fixture("wrapper-fork.sh");
    let tree = goalcup_protocol::process::descendants(child.pid, &support::ps());
    let h = support::handle(child.pid, KillStrategy::ProcessGroup, "sleep");
    assert_eq!(
        h.pgid,
        Some(child.pid),
        "the fixture is spawned into its own process group"
    );
    let r = terminate(&h).unwrap();
    // A launch that leads a group keeps the conservative answer, and the doc on
    // `signal_registered_group` says why: a group cannot be enumerated, so once the leader
    // is gone a member that never entered the walk can be neither signalled nor named.
    assert!(
        r.harness_unverified,
        "lost root must end the group chase: {r:?}"
    );
    assert!(!r.all_gone);
    for pid in tree {
        assert!(
            !goalcup_protocol::process::is_alive(pid),
            "the registered private group did not end member {pid}"
        );
    }
}

/// The escalation, against a tree that ignores SIGTERM. The grace is the promise the
/// participant's harness gets to flush; a SIGKILL before it expires would break that.
#[test]
fn a_tree_that_ignores_sigterm_is_given_the_whole_grace_and_then_killed() {
    let child = support::spawn_stubborn();
    let mut h = support::handle(child.pid, KillStrategy::NativePid, "sleep");
    h.grace_seconds = 1;

    let r = terminate(&h).unwrap();
    assert!(
        !r.survived_term.is_empty(),
        "a tree that traps SIGTERM must be named as a survivor"
    );
    assert!(
        r.waited_ms >= 1_000,
        "the grace was cut short: {} ms",
        r.waited_ms
    );
    assert!(
        r.harness_unverified,
        "once SIGKILL removes the root, a later group retry cannot be trusted: {r:?}"
    );
    assert!(
        !r.all_gone,
        "a lost root may not turn a partially observed kill into verified containment"
    );
}

/// The stale-pid guard. A pid that cannot be the process the CLI registered is never
/// signalled — the alternative is the daemon killing whatever the OS handed the number to.
#[test]
fn a_same_argv_pid_with_a_different_process_identity_is_never_signalled() {
    let child = support::spawn_fixture("wrapper-fork.sh");
    let mut h = support::handle(child.pid, KillStrategy::NativePid, "sleep");
    // This emulates a PID that recycled into an identical wrapper invocation. The live argv
    // still matches, so only the persisted creation identity can keep this number from
    // becoming a termination target.
    h.process_start = Some("recycled-process-instance".to_string());

    let err = terminate(&h).expect_err("a recycled pid has no safe termination");
    let text = format!("{err}");
    assert!(text.contains(&child.pid.to_string()), "got {text}");
    assert!(
        goalcup_protocol::process::is_alive(child.pid),
        "the stranger must still be running"
    );
}

/// `wrapper_unsafe` is a declaration that this connector has no verified kill. Refusing is
/// the whole content of the strategy; a report claiming `all_gone` would be a lie.
#[test]
fn wrapper_unsafe_refuses_rather_than_pretending() {
    let child = support::spawn_fixture("wrapper-exec.sh");
    let err = terminate(&support::handle(
        child.pid,
        KillStrategy::WrapperUnsafe,
        "sleep",
    ))
    .expect_err("wrapper_unsafe has no safe termination");
    assert!(format!("{err}").contains("wrapper_unsafe"), "got {err}");
    assert!(
        goalcup_protocol::process::is_alive(child.pid),
        "a refusal signals nothing"
    );
}

/// Pid 0 is a process-group wildcard and pid 1 roots most of the machine's process tree.
/// Neither may reach discovery even if a persisted or malicious handle carries one.
#[test]
fn pid_zero_and_one_are_never_harness_termination_roots() {
    let child = support::spawn_fixture("wrapper-exec.sh");
    for pid in [0, 1] {
        let mut handle = support::handle(child.pid, KillStrategy::NativePid, "sleep");
        handle.pid = pid;
        handle.pgid = Some(pid);
        handle.process_start = goalcup_protocol::process::process_start_identity(pid);
        handle.argv.clear();

        let error = terminate(&handle).expect_err("a low PID is never termination authority");
        assert!(format!("{error}").contains("termination root"), "{error}");
    }
    assert!(
        goalcup_protocol::process::is_alive(child.pid),
        "rejecting a low root must not signal another process"
    );
}

/// The tree is re-walked while it is dying, not enumerated once and trusted. The wrapper
/// here starts a fresh session from its own SIGTERM handler and only then leaves: the child
/// is a member of the tree when it is forked and an orphan of pid 1 a second later, so a
/// termination that looks only at the list it started with reports `all_gone` over a
/// process that is still holding the writer lock.
#[test]
fn a_child_started_before_the_next_authenticated_snapshot_is_chased() {
    let dir = tempfile::tempdir().expect("a tempdir for the fixture's pid file");
    let pidfile = dir.path().join("newcomer.pid");
    let child = support::spawn_fixture_args("wrapper-late-fork.sh", &[&pidfile]);
    let mut h = support::handle(child.pid, KillStrategy::NativePid, "sleep");
    // The shipped TUI shape, as `a_tui_launch_with_no_process_group_uses_the_descendant_walk`
    // already sets it: a TUI cannot own its process group without taking
    // SIGTTIN on its first terminal read. Left at `support::handle`'s default `pgid:
    // Some(pid)`, `terminate.rs`'s `group.is_none()` is false, `atomically_bound` starts
    // false and can only ever fall, and the non-macOS branch's `all_gone` below becomes
    // unsatisfiable by construction — not a fact about this sweep, a fact about a handle
    // this fixture never actually leads a group with.
    h.pgid = None;
    // Long enough that the wrapper's own exit falls well inside the grace, which is what
    // leaves a single enumeration at the end of it walking from an already-dead root.
    h.grace_seconds = 3;

    let r = terminate(&h).unwrap();

    let forked = support::read_pid(&pidfile);
    assert!(
        r.observed_gone,
        "the wrapper's exit falls inside the grace, so this is the case where the sweep
         has to outlive its own root and still finish: {r:?}"
    );
    if cfg!(target_os = "macos") {
        assert!(!r.all_gone, "macOS cannot atomically bind kill: {r:?}");
    } else {
        assert!(r.all_gone, "survivors: {:?}", r.still_alive);
    }
    assert!(
        !goalcup_protocol::process::is_alive(forked),
        "a child born while the verified root remained live was not terminated"
    );
}

/// An unreaped root is not signal authority.
///
/// The test process remains the fixture's parent, so SIGKILL leaves the registered root as
/// a zombie until `Fixture::Drop` reaps it. `is_alive` deliberately calls that gone, but
/// `ps` still has a row: this proves the guard rejects a stale process-table identity before
/// taking either a direct PID or a process-group signal path.
#[test]
fn an_unreaped_zombie_root_has_no_termination_authority() {
    let child = support::spawn_fixture("wrapper-exec.sh");
    let h = support::handle(child.pid, KillStrategy::NativePid, "sleep");
    let pid = child.pid;

    let pid = i32::try_from(pid).expect("fixture pid fits libc");
    // SAFETY: `child` is the live test fixture the test just started, and the fixture Drop
    // implementation will reap it after the assertions.
    unsafe {
        libc::kill(pid, libc::SIGKILL);
    }
    support::wait_until_gone(child.pid);
    let r = terminate(&h).unwrap();

    assert!(
        r.harness_unverified,
        "a root that is only an unreaped zombie is not verified authority: {r:?}"
    );
    assert!(!r.all_gone, "an unauthenticated tree cannot be called gone");
    assert!(r.targeted.is_empty(), "a zombie PID was signalled: {r:?}");
    assert!(r.survived_term.is_empty(), "a zombie was escalated: {r:?}");
    assert!(r.killed.is_empty(), "a zombie was escalated: {r:?}");
    assert!(
        r.still_alive.is_empty(),
        "a zombie was reported as live: {r:?}"
    );
}

/// A tree that keeps forking while it is being terminated is chased generation by
/// generation — and the chase is bounded, because a seal that ran until a forking harness
/// stopped would never seal at all.
#[test]
fn a_tree_that_keeps_forking_is_chased_generation_by_generation_and_still_returns() {
    let child = support::spawn_fixture("wrapper-generations.sh");
    let mut h = support::handle(child.pid, KillStrategy::NativePid, "sleep");
    h.grace_seconds = 1;

    let r = terminate(&h).unwrap();

    assert!(
        r.harness_unverified,
        "the root can exit during the multi-generation chase: {r:?}"
    );
    assert!(!r.all_gone);
    assert!(
        r.waited_ms < 1_000 + goalcup_daemon::terminate::KILL_SWEEP_TIMEOUT_MS + 3_000,
        "the chase is unbounded: {} ms",
        r.waited_ms
    );
}

/// The stale-pid guard is identity, not only age. A recycled number can be younger than
/// the slack the age check allows for, and what settles it is that the process behind the
/// number is running something the launch never registered.
#[test]
fn a_pid_running_a_program_the_launch_never_registered_is_stale_not_a_target() {
    let child = support::spawn_fixture("wrapper-exec.sh"); // execs into `sleep 300`
    let mut h = support::handle(child.pid, KillStrategy::NativePid, "sleep");
    h.argv = vec!["nonexistent-program".to_string(), "--serve".to_string()];
    // No group: neither a direct PID nor a group number can restore authority after this
    // argv mismatch.
    h.pgid = None;

    let r = terminate(&h).expect("an identity refusal is recorded without a signal");
    assert!(r.harness_unverified);
    assert!(!r.all_gone);
    assert!(r.targeted.is_empty());
    assert!(
        goalcup_protocol::process::is_alive(child.pid),
        "a refusal signals nothing"
    );
}

/// The other half of that guard: a handle whose argv names the process it was registered
/// for is signalled normally. A check that refused every launch would pass the test above
/// and break every seal.
#[test]
fn a_pid_running_the_argv_it_was_registered_with_is_signalled_normally() {
    let child = support::spawn_fixture("wrapper-fork.sh");
    let mut h = support::handle(child.pid, KillStrategy::NativePid, "sleep");
    h.argv = vec![
        support::fixture_path("wrapper-fork.sh")
            .display()
            .to_string(),
    ];

    let r = terminate(&h).unwrap();
    assert!(
        !r.targeted.is_empty(),
        "the current root was authenticated before its initial signals"
    );
    assert!(
        r.harness_unverified && !r.all_gone,
        "the later root loss is not a license to claim verified containment: {r:?}"
    );
}

/// The seal's ordinary case: the participant quit before the deadline, so there is nothing
/// left to kill. A seal that failed here would fail for every well-behaved run.
#[test]
fn a_harness_that_already_exited_seals_without_an_error() {
    let child = support::spawn_fixture("wrapper-exec.sh");
    let pid = child.pid;
    let h = support::handle(pid, KillStrategy::NativePid, "sleep");
    drop(child); // the fixture's Drop kills the tree and reaps it
    support::wait_until_gone(pid);

    let r = terminate(&h).unwrap();
    assert!(r.harness_unverified);
    assert!(!r.all_gone);
    assert!(r.targeted.is_empty());
    assert!(r.killed.is_empty(), "nothing was signalled: {:?}", r.killed);
}

/// A `native_pid` whose name matches the connector is still not the harness when the
/// current snapshot cannot connect it to the registered root: matching `comm` alone would
/// send SIGTERM/SIGKILL to this unrelated sleeper.
#[test]
fn an_unrelated_same_name_native_hint_is_never_signalled() {
    let root = support::spawn_fixture("wrapper-exec.sh");
    let unrelated = support::spawn_fixture("wrapper-exec.sh");
    let mut h = support::handle(root.pid, KillStrategy::NativePid, "sleep");
    h.native_pid = Some(unrelated.pid);
    h.native_binary = Some("sleep".to_string());
    assert!(
        !goalcup_protocol::process::descendants(root.pid, &support::ps()).contains(&unrelated.pid),
        "the adversarial native PID must be outside the registered tree"
    );

    let r = terminate(&h).expect("the registered root still has a safe termination path");

    assert!(
        !r.targeted.contains(&unrelated.pid),
        "a same-name stranger became a target: {:?}",
        r.targeted
    );
    assert!(
        goalcup_protocol::process::is_alive(unrelated.pid),
        "the unrelated same-name process was signalled"
    );
}

/// A grandchild orphaned before the sweep re-snapshots leaves the ppid walk — it
/// reparents to pid 1 — but it does not leave the process group the launch created. The
/// registered pid leads that group, so `killpg` reaches it and the descendant walk cannot,
/// and the strategy field must not be what decides whether the one reachable target is
/// used: `codex-cli` is `native_pid`.
#[test]
fn an_orphan_that_left_the_walk_is_still_reached_through_the_group_the_launch_leads() {
    let dir = tempfile::tempdir().expect("a tempdir for the fixture's pid file");
    let pidfile = dir.path().join("orphan.pid");
    let child = support::spawn_fixture_args("wrapper-orphan.sh", &[&pidfile]);
    let orphan = support::read_pid(&pidfile);
    support::wait_until_reparented(child.pid, orphan);

    // `native_pid`, and a pgid the launch leads: the two together must not
    // signal the walk and stop there.
    let h = support::handle(child.pid, KillStrategy::NativePid, "sleep");
    assert_eq!(h.pgid, Some(child.pid));
    assert!(
        !goalcup_protocol::process::descendants(child.pid, &support::ps()).contains(&orphan),
        "pid {orphan} is still in the walk, so this test would pass without the group signal"
    );

    let r = terminate(&h).unwrap();

    assert!(
        r.harness_unverified && !r.all_gone,
        "a root that departed after its first safe group signal cannot prove containment: {r:?}"
    );
}

/// A harness that renames its argv defeats the identity guard. A leading group does
/// not restore authority: the current PID/PGID could belong to a recycled stranger.
///
/// `all_gone` stays false and `harness_unverified` is true: `ProcInfo` carries no pgid, so
/// nothing here can enumerate the group and say it is empty, and a report claiming the
/// tree was gone off a signal it could not verify would be a false claim.
#[test]
fn an_argv_renamed_pid_that_leads_a_group_is_not_signalled_and_is_reported_unverified() {
    let child = support::spawn_fixture("wrapper-exec.sh"); // execs into `sleep 300`
    let mut h = support::handle(child.pid, KillStrategy::NativePid, "sleep");
    h.argv = vec!["nonexistent-program".to_string(), "--serve".to_string()];
    assert_eq!(h.pgid, Some(child.pid));

    let r = terminate(&h).expect("an identity mismatch is reported without a signal");

    assert!(
        goalcup_protocol::process::is_alive(child.pid),
        "a group number cannot restore authority after the root failed argv validation"
    );
    assert!(
        r.harness_unverified,
        "the guard refused and nothing says so"
    );
    assert!(r.guard_degraded);
    assert!(
        !r.all_gone,
        "an unenumerable group may never be claimed empty"
    );
    assert!(
        r.targeted.is_empty(),
        "a refused pid's descendants are a stranger's children: {:?}",
        r.targeted
    );
}

/// The ordinary seal is one where the pid is not in the process table at all — the
/// participant quit hours ago — and the guard must not answer that with a bare `Ok`, having
/// asked neither of its questions. Every connector whose launch leads a process group then
/// would take the `killpg` path off a number that is no longer a leader and no longer anybody's:
/// days into a 168-hour run the OS has handed it out again, and a recycled pid that leads
/// its own group is a stranger's whole tree, SIGKILLed by a seal.
///
/// The group here is real and alive — a member the leader left behind — which is precisely
/// what makes the signal reach something. It must not be signalled — and, because the
/// wrapper departed without a durable native hint, the report may not claim `all_gone` over
/// an orphan whose containment is no longer provable.
#[test]
fn an_absent_wrapper_without_a_native_hint_is_reported_unverified() {
    let dir = tempfile::tempdir().expect("a tempdir for the fixture's pid file");
    let pidfile = dir.path().join("member.pid");
    let mut leader = support::spawn_unsettled("wrapper-departed-leader.sh", &[&pidfile]);
    let member = support::read_pid(&pidfile);
    // Reaped, so the number is out of the table rather than a zombie in it.
    leader.reap();
    support::wait_until_gone(leader.pid);

    let h = support::handle(leader.pid, KillStrategy::NativePid, "sleep");
    assert_eq!(h.pgid, Some(leader.pid), "the group the launch created");
    assert!(
        goalcup_protocol::process::is_alive(member),
        "the group is alive through its surviving member, which is what makes a killpg \
         on it reach anything at all"
    );

    let r = terminate(&h).expect("an uncertain departed wrapper is reported without an error");

    assert!(
        goalcup_protocol::process::is_alive(member),
        "pid {member} was signalled through a group the registered pid no longer leads"
    );
    assert!(
        r.harness_unverified,
        "the orphan was not provably contained"
    );
    assert!(
        !r.all_gone,
        "an absent wrapper without a native hint cannot prove its session is gone"
    );
    assert_eq!(
        r.targeted,
        Vec::<u32>::new(),
        "an absent root is not a number the seal may signal"
    );
    assert!(
        r.killed.is_empty(),
        "nothing was alive to escalate against: {:?}",
        r.killed
    );
}

// ---------------------------------------------------------------------------------------
// The fixtures. Inline rather than `tests/common/support/mod.rs`, which pulls axum, tokio and the
// dev server into every binary that names it: this file spawns `sleep` and reads `ps`, and
// a termination test that had to link a web server to do it would be the wrong test.
// ---------------------------------------------------------------------------------------
mod support {
    use crate::common::table_lock::TableLock;
    use goalcup_protocol::manifest::KillStrategy;
    use goalcup_protocol::process::{ProcInfo, ProcessHandle, descendants, is_alive, snapshot};
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    /// One `ps` snapshot, the same one `terminate` walks.
    pub fn ps() -> Vec<ProcInfo> {
        snapshot().expect("a ps snapshot")
    }

    /// A live fixture tree. Dropping it kills the whole group and reaps the leader, so a
    /// failing assertion leaves no `sleep 300` behind for the next five minutes.
    pub struct Fixture {
        pub pid: u32,
        child: Child,
        /// Declared last so it is released only after `Drop for Fixture` has killed the
        /// tree: the next test must not start spawning while these processes still exist.
        _lock: TableLock,
    }

    impl Fixture {
        /// Wait for the leader and reap it, so its pid leaves the process table rather than
        /// lingering as a zombie. `is_alive` calls a zombie gone but `ps` still prints it;
        /// callers that need an absent root must reap it before exercising that case.
        pub fn reap(&mut self) {
            let _ = self.child.wait();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            // The group, not the pid: the leader's children outlive it otherwise, which is
            // the very failure these tests exist for.
            let _ = Command::new("/bin/kill")
                .args(["-9", &format!("-{}", self.pid)])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// Where a fixture script lives, so a test can register the argv the CLI would have.
    pub fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    /// Spawn `tests/fixtures/<name>` into its own process group and wait for its tree to
    /// settle, so a test never races the shell's own fork.
    pub fn spawn_fixture(name: &str) -> Fixture {
        spawn_fixture_args(name, &[])
    }

    /// The same, for a fixture that is handed a path to report through.
    pub fn spawn_fixture_args(name: &str, args: &[&Path]) -> Fixture {
        let fixture = spawn_unsettled(name, args);
        settle(fixture.pid);
        fixture
    }

    /// Spawned into its own group and not waited for. `settle` waits for a tree below the
    /// leader, and a fixture whose whole point is that it leaves none — it backgrounds one
    /// process and returns — would spend the settle's five seconds and then fail.
    pub fn spawn_unsettled(name: &str, args: &[&Path]) -> Fixture {
        // Taken before the spawn, not after: the point is that no other test's tree is in
        // the table while this one builds its own.
        let _lock = TableLock::acquire();
        let path = fixture_path(name);
        let mut command = Command::new(&path);
        command.args(args);
        let child = own_group(command)
            .spawn()
            .unwrap_or_else(|e| panic!("spawning {}: {e}", path.display()));
        Fixture {
            pid: child.id(),
            child,
            _lock,
        }
    }

    /// The pid a fixture wrote for us, once it has written it. Polled rather than read
    /// once: the fixture writes from a signal handler, and the test must not race it.
    pub fn read_pid(path: &Path) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(path)
                && let Ok(pid) = text.trim().parse::<u32>()
            {
                return pid;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("{} never named the process it forked", path.display());
    }

    /// The pids a `tree::hardlink_codex_helper` role wrote for us in one shot (see
    /// `GOALCUP_FIXTURE_PIDFILE`'s doc), once the file exists. The role writes every pid in
    /// a single `fs::write`, so a non-empty read is a complete one — there is no partial-line
    /// race the way there would be with N separate appends.
    pub fn read_pids(path: &Path) -> Vec<u32> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(path) {
                let pids: Vec<u32> = text.lines().filter_map(|l| l.trim().parse().ok()).collect();
                if !pids.is_empty() {
                    return pids;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("{} never named the processes it forked", path.display());
    }

    /// The ppid the kernel currently has on record for `pid`, or `None` once it is gone.
    /// `ProcInfo` already carries this; this is for a test that wants one pid's ppid without
    /// building the descendant walk around it — e.g. proving a reparent target directly
    /// (`ppid == 1`) rather than inferring it from absence in `descendants`.
    pub fn ppid_of(pid: u32) -> Option<u32> {
        ps().into_iter().find(|r| r.pid == pid).map(|r| r.ppid)
    }

    /// Force-kills every pid in the list when dropped, panic or not. `Fixture::Drop` only
    /// reaches its own registered root's process group; several fixtures
    /// deliberately spawn processes in groups of their own (that is the shape under test),
    /// so a failing assertion between spawn and `terminate` — or a `terminate` that, as
    /// documented, does not reach all of them — must not leave `sleep 300` running for
    /// five minutes on a shared machine.
    pub struct KillOnDrop(pub Vec<u32>);

    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            for pid in &self.0 {
                let _ = Command::new("/bin/kill")
                    .args(["-9", &pid.to_string()])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
    }

    /// A tree that ignores SIGTERM, for the escalation case. Inline rather than a third
    /// fixture file: the plan ships two executables and this needs no bytes on disk.
    ///
    /// An ignored disposition is inherited across `fork` and `exec`, so the `sleep` ignores
    /// SIGTERM too and the whole tree has to be escalated.
    pub fn spawn_stubborn() -> Fixture {
        let _lock = TableLock::acquire();
        let mut c = Command::new("/bin/sh");
        c.args(["-c", "trap '' TERM; sleep 300 & wait"]);
        let child = own_group(c).spawn().expect("spawning the stubborn tree");
        // Built before settling, as `spawn_fixture_args` does: if `settle` panics, this
        // `Fixture`'s drop kills the group before the lock is released. A bare `Child` here
        // would leak `sh` + `sleep 300` past the release, which is the contamination this
        // lock exists to prevent.
        let fixture = Fixture {
            pid: child.id(),
            child,
            _lock,
        };
        settle(fixture.pid);
        fixture
    }

    /// Its own process group and no inherited stdio, which is what makes `pgid: Some(pid)`
    /// on the handle true rather than decorative.
    fn own_group(mut c: Command) -> Command {
        c.process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        c
    }

    /// Wait until the wrapper has either forked its child or exec'd into it.
    fn settle(pid: u32) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let table = ps();
            let root = table.iter().find(|r| r.pid == pid).map(|r| r.comm.clone());
            match root {
                // Gone already: nothing left to wait for, and the test will say so.
                None => return,
                // Forked, or exec'd into the sleep the handle names.
                Some(c) if c == "sleep" => return,
                _ if descendants(pid, &table).len() >= 2 => return,
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the fixture tree never settled");
    }

    /// Wait until `pid` has left `root`'s ppid walk, which is what happens the moment the
    /// middle shell that forked it exits. Polled rather than slept: the reparenting is the
    /// kernel's and a fixed sleep would be either flaky or slow.
    pub fn wait_until_reparented(root: u32, pid: u32) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if !descendants(root, &ps()).contains(&pid) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("pid {pid} never left pid {root}'s walk");
    }

    /// The process group the kernel says a pid is in, read rather than assumed.
    ///
    /// `ProcInfo` carries no pgid — the walk is ppid-only — so a fixture that claims to have
    /// left the launch's group has to be checked against `ps` or it proves nothing.
    pub fn pgid_of(pid: u32) -> u32 {
        let out = Command::new("/bin/ps")
            .args(["-o", "pgid=", "-p", &pid.to_string()])
            .stdin(Stdio::null())
            .output()
            .expect("ps reads a process group");
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .unwrap_or_else(|e| panic!("ps gave no process group for pid {pid}: {e}"))
    }

    pub fn wait_until_gone(pid: u32) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if !is_alive(pid) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("pid {pid} outlived its own kill");
    }

    /// The handle the CLI would have registered: launched now, leading its own group, with
    /// `program` naming the launcher and `native_binary` naming the process that holds the
    /// session. Fixtures use a shell launcher, so keeping these distinct pins the same
    /// wrapper/native relationship a real manifest records.
    pub fn handle(pid: u32, strategy: KillStrategy, native_binary: &str) -> ProcessHandle {
        ProcessHandle {
            pid,
            pgid: Some(pid),
            native_pid: None,
            native_binary: Some(native_binary.to_string()),
            native_process_start: None,
            process_start: goalcup_protocol::process::process_start_identity(pid),
            strategy,
            grace_seconds: 1,
            launched_at: goalcup_daemon::now_rfc3339(),
            program: "/bin/sh".to_string(),
            argv: Vec::new(),
            env_keys: Vec::new(),
        }
    }
}
