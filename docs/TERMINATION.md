# Termination: the contract

What `crates/goalcup-daemon/src/terminate/` promises at seal, and what it does not. Read this
instead of the module. The daemon kills, not the CLI (D22); `Confirmed` is bounded to the owned
set (D30). Code: `terminate_with` in `mod.rs`; guard, ordering and fail-closed reports in
`authority.rs`; the macOS singleton path in `darwin.rs`; the capture finish in
`runtime/termination.rs`.

## Outcomes (`TerminationOutcomeV1`)

| Outcome | It claims | It does not claim |
|---|---|---|
| `AlreadyStopped` | `liveness(root)` was false before any signal; nothing was signalled. | Anything about descendants. A wrapper's native child may be running (the D30 pre-call escape). |
| `Confirmed` | The report had `observed_gone`, a non-empty `targeted`, an empty `still_alive`, no cap hit, and a second by-number sweep 250 ms later (`RECONFIRM_DELAY_MS`) found none of `targeted` alive. | Atomic delivery (`all_gone`), or anything about the residual windows below. |
| `Unresolved` | `ProcessAuthorityUnavailable`: `ps` could not be asked, the guard refused the root, the root was lost mid-chase, `¬observed_gone`, empty `targeted`, or a group handle. `RequestFailed`: `wrapper_unsafe` or another error. `ProcessStillRunning`: `still_alive` non-empty, cap hit, or the reconfirm found one alive. `UnsupportedPlatform`: not unix. | That the harness is running: only `ProcessStillRunning` says so. `ConfirmationTimedOut` is declared and never produced. |

## The three anchors (`sweep_anchor`, sound only after a first signal)

| Anchor | When | What a poll does |
|---|---|---|
| `Rooted(table)` | The guard says the root is live with its recorded creation identity and argv, and a fresh snapshot lists it. | Adopts newcomers (below) from the root and from every live identity-verified member; authorises a signal against the root's ppid tree **or** a live member's (`Owned`). |
| `Closed(table)` | The root is not in the table (it exited), or was live at the guard and absent one read later, including an exit between the guard's own reads (`liveness` live, then no creation identity, or an argv that names no registered program, as Linux prints `[comm]` for a process that is exiting but not yet a zombie: the guard asks `liveness` again and reads "no longer live" as an exit). | Adopts from every targeted member alive with its recorded creation identity, never from the root; authorises a signal against a live member's ppid tree alone, so an orphaned member and what it forks are still SIGKILLed; a live pid ours by nothing returns `containment_unverified`. |
| `Lost` | The guard refused the root (a *different* creation identity, or no readable identity on a root that is still live, or a renamed argv on a root that is still live) or `/bin/ps` could not be spawned. | Nothing may be claimed; the call returns `containment_unverified` at once. |

Under every anchor a targeted pid is asked **by number with its recorded creation identity**, never
by lineage — the phase gate, the SIGKILL selection, the forkers adoption walks from, `still_alive` —
so an orphan alive is followed; a recycled number cannot pass; no captured identity over-reports.

## The five report claims (`TerminationReport`)

- `targeted`: every pid signalled, in signal order, root last. **Not** the owned set: a process
  never seen in a walked snapshot is not here, and the report cannot speak for it.
- `observed_gone`: the escalation ran under a root re-authenticated at every step, the walk was
  never capped, the last by-number, creation-identity-checked sweep found none of `targeted`
  alive, and on macOS every witnessed fork was paid for by an adoption from its forker (below).
  **Not** that delivery was atomic with authorisation; **not** that nothing outside `targeted`
  is alive (see the windows); **never true for a group handle**.
- `all_gone`: `observed_gone` and every delivery was instance-bound (a Linux pidfd). **Never
  true on macOS**, for the singleton included. It adds atomicity to `observed_gone` and nothing
  about the tree.
- `harness_unverified`: the daemon could not keep the root's identity (the guard refused, the
  root was lost mid-chase, `ps` failed), or the handle led a process group — always, by design,
  because `ProcInfo` carries no pgid and a group cannot be enumerated. **Not** "the root is
  gone"; **not** a capped walk. `all_gone` is never true beside it.
- `walk_cap_exceeded`: the signed `process_walk` budget refused a target, so `targeted` is not
  the whole owned set and no sweep over it may call the tree gone. **Not** a loss of authority:
  the root was kept; the seal records `ProcessStillRunning`.
- Also: `still_alive` is what the last by-number sweep found among `targeted` — empty beside
  `harness_unverified` when the fail-closed return took no sweep, and never a process that was
  not targeted; `survived_term`/`killed` name who outlived the grace and who got SIGKILL;
  `guard_degraded` says the handle carried no argv, so only the creation identity was checked.

## The ordering rule

`members_deepest_first`: the owned set below the root, **deepest level first, the native binary
hoisted only within its own depth, the root excluded and signalled last** (phase B). Each phase
is its own grace and kill sweep. Why: a parent is never ended before the children under it, so
nothing is orphaned out of the ppid walk by this daemon's own signal (D1); and while any
non-root member is owned — an orphan alive included — the root is unsignalled, so every phase-A
sweep is `Rooted` and the anchor closes only when the owned set is `{root}`.

## The adoption rule

Every poll, `newcomers` below the root (while live) **and below every targeted member alive with
its recorded creation identity** are signalled and tracked (witness attached first, on macOS);
each newcomer's ppid is the forker that adoption pays for. Under `Closed` the members alone are
walked; the root's own exit no longer ends adoption. Ceiling `MAX_TARGETED` = 4096 (`process_walk` tightens).

## The duration bound

`2 × (grace_seconds + KILL_SWEEP_TIMEOUT_MS)` — one grace and one 2 s kill sweep per phase —
plus 50 ms poll granularity and the `ps` spawns, plus the seal's 250 ms reconfirm. A tree that
dies at once ends at the first clear poll; a member ignoring SIGTERM costs the full grace.

## The residual windows, per platform (stated, not solved)

**Both.** A process that had already left the root's ppid walk when the call began is outside
the owned set and never claimed (D30). A group handle is never confirmed. Closed by the fix wave
(scenarios green in `tests/it/terminate_sim.rs`): F7 (the orphaned ignorer is SIGKILLed by identity and
named), F8 (an orphan alive holds the phase-A gate; `Closed` adopts from live members, the root's
own exit included), F9 and the root unwitnessed in phase A (both below).

**Hermes** (M6, connector `hermes-cli`). Hermes starts its tool processes with
`start_new_session=True` (`tools/environments/local.py:1388`, hermes-agent 0.19.0), so each is a
session leader of its own. A tool process the run left behind is outside the root's ppid walk once
its parent is gone, and is the D30 residual above: `Confirmed` covers the owned set only and never
claims it. The connector's `[kill]` section says the same in a comment.

**Linux** (pidfd-bound signals, no fork witness). A child forked and orphaned inside one poll
interval — forker forks and exits between two snapshots — is in no snapshot under a live
parent and no ppid walk can see it; the M0 §2.5 respawn, when the handler forks and exits
within 50 ms, is this. The property runner allows exactly this set (`same_poll_orphans`).
Linux's `W` is wider than that name: without a fork witness it covers any orphan the ppid walk
cannot see, as ruled in "RULED: on Linux, `Confirmed` excludes fork-and-leave wrappers" below.
`W` is the daemon's and the launcher's sweep described there does not narrow it: the sweep ends
such orphans in a `goalcup start` run before its seal, outside this contract.
A root that `liveness` still reads as live while its argv already reads `[comm]` (exiting, not
yet a zombie) at both of the guard's `liveness` reads is refused as renamed: the anchor is
`Lost` and the seal `Unresolved`, fail-closed. The second read narrows that window to the
time one `/bin/ps` takes; it does not close it.

**macOS** (numeric `kill`, kqueue witness per member from the root down, attached before phase
A; audit token for the singleton). Each witness is read twice a poll — before the snapshot and
after the adoption — and a fork it reports is paid only by an adoption from that member in the
poll it was read (or the next, for the second read; a forker dead by then leaves it unpaid).
So a one-poll fork-and-exit is refused whatever the member forked before, and a reaction to
this daemon's own signal always lands in a strict window. **The residual:** the kernel gives
one bit per read, so two forks by one member inside one read window — the first child adopted,
the second's line gone from the table before the next snapshot (a grandchild forked and
orphaned) — are one paid bit. A handler that runs a subprocess which exits before the next
snapshot is refused (a fork nothing adopted): fail-closed. `all_gone` is never true. The
singleton path proves one process's death and cannot see a descendant reparented before it attached.

## The launcher's ending of a child it just released (D40, outside this contract)

`goalcup start --goal-file` parks its capture child before `exec` until the daemon has durably
registered it, then releases it (CONTRACTS.md, "Amendment 2026-10-02 (C6 capture spawn gate, H-10
4b)"). A seal that lands between the registration and the release cannot signal a child that is
still parked: the daemon's guard finds a command line that is not the harness's and binds
`Unresolved(ProcessAuthorityUnavailable)`. The launcher, which sees the entry sealed right after
the release, therefore ends its own just-released child (`released_child.rs`): SIGTERM native-first,
the connector's grace, SIGKILL, inside `termination_bound(grace)`.

That ending is **not a `TerminationOutcomeV1`**, is never chained into the seal and is only
printed: the seal is already committed and the trace says what the daemon observed. It claims
nothing about descendants other than the root and a re-resolved native child, and a native binary
that was never observed (or a root that left alone while the connector names one) is reported
`NativeUnverified`, never `Ended`. The refusal line it ends with says nothing is running only for
`Ended` and `AlreadyGone`; otherwise it says the harness may still be running.

The residuals, both needing two faults and both ending the same way (a harness running on a sealed
entry that names its exact instance; a later `goalcup seal` answers the committed report and
signals nothing): a CLI crash between the release and its status look, with a seal that raced in;
and the same raced seal with the CLI alive, when the launcher's status read after the release and
the one after the `Launch` error are both unreadable (a status that cannot be read never ends the
child). The look after the `Launch` error can come after Codex's native-scope wait of up to
120 s, and each read may take the 30 s `CALL_TIMEOUT`. Separately, if a parked child's
acknowledgement arrives after the 5 s gate timeout and the child `exec`s before the gate closes,
the launcher SIGKILLs it without grace: a harness that ran for milliseconds (CONTRACTS.md, gate
amendment ¶5(f)).

## The invariant (quoted verbatim from `tests/it/terminate_sim.rs`; keep the two identical)

```text
Let B be every pid born under the root during the call (ground truth), O ⊆ B the owned
set (B minus what had left the root's ppid walk before the call, D30), A the pids alive
at return, K the kills sent, R the report.
I1  R.observed_gone ⟹ A ∩ O ⊆ W, where W is the platform's stated residual window
    (Linux: any orphan the ppid walk cannot see, see "RULED" below; macOS: ∅).
I2  R.observed_gone ⟹ A ∩ R.targeted = ∅.
I3  every k ∈ K names a pid ≥ 2 that was ours when sent, with SIGTERM or SIGKILL only;
    every killpg names a group of ours under a live leader.
I4  ps could not be asked at any point ⟹ ¬R.observed_gone ∧ R.harness_unverified
    (an Err result satisfies this vacuously).
I5  handle.pgid = root ∧ root exited ∧ R.observed_gone ⟹ every group member exited
    before the root.
I6  every process dies within k polls of its first SIGTERM and nothing forks ⟹
    R.targeted = B ∧ A ∩ B = ∅ ∧ R.still_alive = ∅, and, unless handle.pgid = root,
    R.observed_gone.
```

The runner is un-ignored: with F7, F8 and F9 closed it passes 5000 cases by default and 50 000
with `GOALCUP_SIM_CASES=50000 GOALCUP_SIM_ALL=1` reporting no seeds.

**Provenance note (2026-09-19):** the 5000-case default run is covered by the committed gate
log (`docs/qualification/2026-09-17-gate.md`, in git history at `6494030`). The 50 000-case figure is **not backed by any
committed artifact** in this repository — it is a remembered result. Either re-run it at the pin
and file the output beside the gate log, or drop the number. Do not cite it as evidence until
one of those happens.

## RULED: on Linux, `Confirmed` excludes fork-and-leave wrappers

Owner ruling (1), 2026-10-13: say plainly that on Linux `observed_gone`, `all_gone` and
`Confirmed` exclude every orphan the ppid walk cannot see. Linux has no fork witness, so such a
process is in no snapshot under a live parent. Three shapes are known: a same-poll orphan, a
fork-and-leave child (forked before the root exits and never waited for), and a member's fork
after the root has died. A report over a tree holding one can come back `observed_gone` with
`all_gone: true`, and `Confirmed` can stand, while the orphan is still running. `W` for Linux is
therefore that whole set, not only the same-poll orphan named above; `I1` holds against that
widened `W`. The ruling widens the stated limitation and changes no code.

What each run measured:

- `docs/qualification/2026-09-16-review.md` (in git history at `6494030`): 5/5 reproductions of
  an orphan invisible to the walk, and its section "seal.rs gate" recording that `Confirmed`
  *would be returned*.
- `docs/qualification/2026-09-18-f0-linux-residual.md` (also at `6494030`): the same residual,
  found again while the CI death was being diagnosed.
- 2026-10-13, in Docker: `a_report_does_not_claim_the_harness_gone_while_a_post_root_death_orphan_lives`
  runs `wrapper-root-dies-first.sh`, where a member forks an orphan about 400 ms after the root
  has died. On Linux the report said `all_gone: true` with the orphan alive. That test is
  `ignore`d on Linux, with this section as its reason, and still runs and passes on macOS.

"Same-poll orphan" understates the reach of the window. `fork()` immediately followed by
`exit()` is two syscalls apart with nothing enforcing a gap, so a wrapper of the shape "hand off
to a child and leave" has landed in it in every reproduction so far. This repository already has
a name and a fixture for that shape, "the ordinary shape — a launcher whose session is a child".

**The launcher sweeps them on Linux.** The fix the ruling asked for before the 168-hour dry run
is a child subreaper (`PR_SET_CHILD_SUBREAPER`) that is an ancestor of the harness. The daemon is
not that process: it is detached into its own session and is not an ancestor of the harness, so
it would never see a harness orphan. The launcher (`goalcup start`) is, and on Linux it now is a
child subreaper (`crates/goalcup-cli/src/subreaper.rs`):

- It sets `PR_SET_CHILD_SUBREAPER` before the gated spawn, so a process the harness forks and
  leaves is reparented to the launcher, not to init. If `prctl` fails it says so once on stderr
  and sweeps nothing; the residual is then as stated above.
- After the root's `wait` returns and before the seal, it sweeps its own children: those that
  started no earlier than the root and are not the daemon it started. It sends SIGTERM, waits the
  connector's grace, sends SIGKILL, and reaps each one by its pid. It repeats until a pass finds
  no child, because an orphan may fork again, within the grace plus 5 s. These are the
  launcher's unreaped children, so their pids cannot be recycled and a signal cannot reach a
  stranger.
- It prints `swept N process(es) the harness left behind after it exited.` on stdout when it
  swept something. Anything still alive at the bound is named by pid on stderr, as is a `/proc`
  it could not read; and `goalcup start` then exits 1 after the seal when a process may still be
  running (one alive at the bound, or `/proc` unreadable after the sweep signalled one), so a
  script that checks only the exit code learns of it. The daemon is unchanged, and the
  trace and the seal gain no field: the daemon's report still answers `AlreadyStopped` for a
  reaped root, and in a run whose sweep ended everything that is now true when it is written.
- It runs only from the `goalcup` binary, never from a `start_with` driven in-process.

Measured in Docker: `the_launcher_sweeps_an_orphan_forked_after_the_root_died_before_the_seal`
(`crates/goalcup-cli/tests/full/launcher_sweep.rs`) runs the real `goalcup start --dev` on the
controlled workload's `orphan-after-root` mode. Its member forks a detached `sleep 300` only after
the root has died. The orphan is gone when `goalcup start` returns, the sweep line comes before the
seal's report, and the trace verifies. Without the subreaper the same test fails with the orphan
alive. `a_root_reaped_by_the_scope_wait_still_has_its_orphans_swept` repeats it under a
`stdout_json` identity (Codex's shape) whose root exits before any scope line, so the launcher's
scope wait reaps the root before the launch is recorded; the sweep is armed before that wait.

The residuals, stated plainly:

- **The launcher dies.** If `goalcup start` itself dies (a crash, `kill -9`), it sweeps nothing,
  and what it adopted goes to init as before.
- **Stragglers.** A process still alive at the grace plus 5 s (one in uninterruptible sleep, or a
  tree that keeps handing over new orphans) is left running. It is named on stderr and the command
  exits 1 after the seal, but the trace and the seal do not record it.
- **Paths that end the harness some other way.** The sweep runs only after the root's own `wait`.
  The sealed-launch ending ("The launcher's ending of a child it just released" above) and the
  post-release failures that seal an unlaunched run (a tee that could not be started) never
  sweep: what the harness left behind stays the launcher's child until it exits, then goes to
  init.
- **A seal from another terminal.** A standalone `goalcup seal` while the harness runs has the
  daemon end the tree, and an orphan forked after that is reparented to the launcher, which
  sweeps it once the root's `wait` returns. The daemon's report can say `all_gone` while that
  sweep is still running. For an orphan that dies on SIGTERM the window is tens of milliseconds
  (the sweep sleeps at least one 20 ms poll); for one that ignores SIGTERM it is up to the grace
  plus 5 s. This window is not measured. The daemon and the sweep may signal the same members
  here, and that is safe: both escalate on the handle's grace, the sweep's clock starts no
  earlier than the daemon's first SIGTERM, the daemon signals through pidfds, and the sweep
  signals only the launcher's own unreaped children.

The daemon test stays `ignore`d on Linux, with its reason now pointing here: it calls the daemon's
`terminate()` in-process with no launcher above the tree, so the daemon alone still cannot see the
orphan. The CLI-level test above is what proves the sweep.

**This is not a V1 blocker.** Linux is not a V1 row; V1 claims one macOS target, where the
kqueue witness closes this.
