# F0's post-root fixture on Linux: inside the documented residual window, and the window is wide

> **Harvested onto `v1/trunk` 2026-09-24.** Written on `v1/ci-forkbomb` while the CI hang was
> being diagnosed. Only this file was taken: that branch's `gate.yml` is the 935-line
> bisection harness, superseded by the three-job workflow `v1/ci-collapse` landed, and its
> tip commit states a conclusion the follow-up overturned. References below to "this branch"
> and to commits from `580bf91` onward point at history that remains on `v1/ci-forkbomb`,
> which is parked, not deleted.

Found while diagnosing the CI hang in `process_kill` (that
diagnosis is separate and unfinished — see `v1/ci-forkbomb`'s commit history from `580bf91`
onward). This is not that hang. It is a second, independent, MEASURED behaviour the same CI
work surfaced once per-test isolation (`probe-which-test`, run `35354783946`, job
`105631359083`) made individual test results legible again.

**Adjudicated against `docs/TERMINATION.md`'s own invariant, and quoted here rather than
left for the next reader to look up:**

```text
I1  R.observed_gone ⟹ A ∩ O ⊆ W, where W is the platform's stated residual window
    (Linux: same-poll orphans; macOS: ∅).
```

and, from the same file's "residual windows, per platform" section:

> **Linux** (pidfd-bound signals, no fork witness). A child forked and orphaned inside one
> poll interval — forker forks and exits between two snapshots — is in no snapshot under a
> live parent and no ppid walk can see it; the M0 §2.5 respawn, when the handler forks and
> exits within 50 ms, is this. The property runner allows exactly this set
> (`same_poll_orphans`).

**Verdict: case (a) — the measured behaviour is INSIDE `W` as documented. `I1` is not
violated.** `tests/terminate_sim.rs:586`'s own formal definition of `same_poll_orphans` is
"pids whose parent forked them and exited within the same poll" — exactly what the member in
this fixture does (forks the orphan, writes one pidfile line, calls `process::exit(0)`, no
wait, all inside one 50ms gap between `ps` snapshots). So `a_report_does_not_claim_the_-
harness_gone_while_a_post_root_death_orphan_lives` is asserting a property STRONGER than what
`I1` promises on Linux: the test wants `observed_gone` to never be true over a live same-poll
orphan, and the contract explicitly allows exactly that. This is closer to the
`process_kill.rs:443` precedent (a test expectation stronger than the implementation's actual
contract) than my first pass at this writeup allowed — see "What changed from the first
draft" at the bottom.

**But do not stop reading here.** The verdict does not make this a non-finding. It relocates
it: from "the implementation violates its own contract" to "the contract's Linux residual is
wide enough to swallow an ordinary wrapper shape, and the fixture is proof." That is argued
below, after the measurement and mechanism, which are unchanged from the first draft and
still the evidence base for the adjudication above.

## The measurement

`crates/goalcup-daemon/tests/process_kill.rs:311`,
`a_report_does_not_claim_the_harness_gone_while_a_post_root_death_orphan_lives`, added to
this file "yesterday" (2026-09-17) per its own history, ran alone via
`"$BIN" --exact a_report_does_not_claim_the_harness_gone_while_a_post_root_death_orphan_lives
--test-threads=1 --nocapture` in `probe-which-test`, run 35354783946, and failed in 0.68s —
not a hang, a clean red on the first try:

```
thread '...' panicked at crates/goalcup-daemon/tests/process_kill.rs:347:5:
the report claimed the harness observed gone while orphan 6000, forked after root's death,
is still running: TerminationReport { strategy: NativePid, targeted: [5961, 5960, 5958],
survived_term: [], killed: [], still_alive: [], all_gone: true, observed_gone: true,
walk_cap_exceeded: false, waited_ms: 649, guard_degraded: true, harness_unverified: false }
```

`targeted` is exactly the three processes the fixture starts with (root, member, member's
own `sleep 300` native-session stand-in — the shape-proof at `process_kill.rs:322-328`).
Orphan 6000, forked by the member ~520-650ms in per the fixture's own timing
(`wrapper-root-dies-first.sh`'s 120ms root-exit delay + the member's 400ms in-process
delay), was never targeted, never adopted, and is not in `still_alive` because nothing ever
put it there. `observed_gone` came back `true` anyway. `waited_ms: 649` is unremarkable —
almost exactly the fixture's own designed timing, no evidence of scheduling inversion.

Immediately re-run in the same job's context (`gate`, same run) as part of the full
24-test `cargo test -p goalcup-daemon --test process_kill --no-fail-fast --
--test-threads=1`, and separately on a completely clean runner (`probe-full-suite`, same
run, `CARGO RC=101`, same 23-passed/1-failed shape) — same result both times. This is not
flaky; it reproduces on the first try, every time it has been run since being added.

## Mechanism: why this is a same-poll orphan, precisely

`crates/goalcup-daemon/src/terminate/os.rs:68-71`:

```rust
/// A witness for forks, execs and the exit of the exact instance at `pid`, from this
/// moment on. macOS: an `EVFILT_PROC` kqueue. Linux: `None`.
fn fork_witness(&self, pid: u32) -> Option<Box<dyn ForkWitness>>;
```

and its `LiveOs` impl at `os.rs:195-206`: `#[cfg(target_os = "macos")]` acquires a kqueue
witness; `#[cfg(not(target_os = "macos"))]` returns `None` unconditionally — Linux included,
no fallback. `terminate/mod.rs:791-797`'s `forks_accounted` is therefore vacuously `true` on
Linux (`witnesses` is never populated), exactly as `mod.rs:465-466`'s own doc says: *"Empty
elsewhere: the seam's `fork_witness` returns `None`, so nothing is pushed and
`forks_accounted` is vacuous."* This is not a bug in that function — it is `W`'s macOS/Linux
asymmetry, stated in `TERMINATION.md`'s own residual-windows section, implemented exactly as
documented.

So on Linux, `observed_gone` depends entirely on the ppid-poll adoption in
`newcomers_from_live` (`mod.rs:712-731`), whose own doc (`mod.rs:715-717`) says: "on Linux
(no kqueue witness) a walk from the still-live forker is the only thing that can still see
it." "Still-live" is the operative word — `newcomers_from_live` only walks below a targeted
member while it is observed alive in a snapshot. This fixture's member forks the orphan,
writes one pidfile entry, and calls `std::process::exit(0)` with no wait — the whole gap
between fork and exit is one small file write, comfortably inside the 50ms
(`LIVENESS_POLL_MS`, `mod.rs:50`) gap between two `ps` snapshots. No snapshot ever shows the
member alive with the orphan as its child. That is `same_poll_orphans` by the simulation's
own definition (`terminate_sim.rs:586`, quoted above), not a different or narrower case of
it.

## The sharper finding: `W` is not a narrow race, it is the ordinary wrapper shape

This is the part worth saying loudly, per the adjudication above. "Same-poll orphan" reads
like an edge case — a fork that happens to land inside an unlucky 50ms window. It is not an
edge case here: it is what **any** wrapper of the shape "fork a child, then exit without
waiting for it" produces, every time, because the fork and the exit are two syscalls apart
with nothing enforcing a minimum gap between them. This codebase already has a name and a
fixture for that ordinary shape — `wrapper-fork.sh`
(`process_kill.rs:417`,
`the_process_named_by_the_program_is_targeted_before_the_wrapper_that_hides_it`) — and its
own doc calls it "the ordinary shape — a launcher whose session is a child."
`run_member_forks_after_root_role` (`tests/support/tree.rs:199`) is that same ordinary shape,
just timed to fire after the root's own death rather than at launch. Nothing about the
timing was engineered to be fast; `Command::spawn()` immediately followed by
`process::exit(0)` is the natural, unforced duration of "hand off to a child and leave."

So `W`'s Linux clause — "same-poll orphans" — is not a residual that requires bad luck to
hit. It is reachable by construction, on demand, by any wrapper that does not wait on its
own child, which is a common and unremarkable shape for a session launcher to have. A
residual nobody can hit in practice is a footnote in a spec. A residual every "spawn and
leave" wrapper hits is closer to a standing exception carved out of the safety property than
a genuine edge case, even though — per the verdict above — it is exactly what `I1` currently
promises for Linux and nothing more.

## Relation to the 2026-09-16 review's Part 2 finding

`docs/qualification/2026-09-16-review-evidence/targeted-and-adoption-window.md` (Part 2,
ranked CRITICAL, measured 5/5 on Linux against `4d15fbb`) found the adoption window
"unbounded" because `newcomers()` ran only under `Anchor::Rooted` — a fork could happen at
ANY point after the root closed and never be caught, not just inside one poll. The
member-anchor walk in the current `newcomers_from_live` (walking below "the live root **or**
any targeted member alive with its recorded identity") is that finding's fix, and it is a
real fix: it closes the *unbounded* window down to exactly `W`, the documented and now
formally-modelled `same_poll_orphans` set (`terminate_sim.rs:586`, exercised 5000+ cases by
the property runner per `TERMINATION.md`'s closing line). What the 2026-09-16 review called
CRITICAL and unbounded is fixed. What remains is `W` itself — bounded, documented, formally
modelled, and, per this fixture, still wide enough to swallow the ordinary case.

## What this is not

- Not the CI runner hang under investigation on this same branch. That symptom is a step
  that does not return for 40+ minutes with ample disk (3-6GB free, ladder-measured),
  memory (>4GB free) and file descriptors (<10000) immediately beforehand. This defect is a
  clean, fast (0.68s) red with a normal exit code (101). Two different problems;
  `v1/ci-forkbomb`'s other commits are the hang's own trail.
- Not a flaky test needing a longer grace period or a retry. `grace_seconds` was 3s in the
  fixture and the report closed at `waited_ms: 649` — over 2 seconds before the grace
  deadline. A longer grace would not help: nothing was ever watching for the orphan to begin
  with, so there was nothing for more time to find.
- Not a violation of `I1` as `TERMINATION.md` currently states it. See the verdict above.
  Anyone tempted to call this "the implementation is broken" should instead read it as "the
  contract, exactly as implemented, permits this."

## Recommendation

**Do not `#[ignore]` or loosen
`a_report_does_not_claim_the_harness_gone_while_a_post_root_death_orphan_lives` on the
strength of this writeup alone.** It measures something true and worth keeping visible.
But per the verdict above, the test's expectation and `I1`/`W` as currently documented
disagree, and reconciling that disagreement — tighten the contract to shrink `W`, or
reconcile the test's expectation to what `W` already promises (e.g. assert the weaker,
still-meaningful property that `harness_unverified` or an equivalent uncertainty signal is
set whenever a same-poll orphan is possible, rather than that `observed_gone` is never true
over one) — is the owner's call, not this document's. This document's job is to put the
adjudicated fact in front of that call, not to make it.

If the owner rules to tighten `W` instead, the same three candidates from the first draft of
this document still apply and are repeated here for that case:

1. A Linux fork-witness equivalent to the macOS kqueue path — candidates are the process
   events connector (`NETLINK_CONNECTOR`/`CN_IDX_PROC`, needs `CAP_NET_ADMIN`, may not be
   available in every sandboxed/CI/container environment, worth checking first) or
   `PTRACE_SEIZE` with `PTRACE_O_TRACEFORK` on every targeted member (changes signal
   delivery semantics for the tracee, needs care).
2. A tighter or adaptive Linux poll cadence specifically while a member holds
   `Anchor::Rooted`-adjacent authority post-root-death — this narrows `W`, it does not empty
   it, so it is a mitigation, not a closure, unless the interval can be made provably tighter
   than any legal member's post-fork lifetime (it cannot, in general).
3. Blocking the member's own exit is not this codebase's to control — the member is someone
   else's process (a real connector's wrapper), not code this daemon can change.

This was out of `v1/ci-forkbomb`'s scope (Task 2 owned
`crates/goalcup-daemon/tests/support/tree.rs` only, per
`docs/superpowers/plans/2026-09-18-step6-post-tag-batch.md`), so no fix, and no test change,
was attempted here. This document is the handoff.

## What changed from the first draft

The first draft of this document called the measurement a "real, security-relevant property
gap" and reached for `#[ignore]`-avoidance language without first checking the measured
report against `TERMINATION.md`'s own `I1`/`W`. It should have opened with the contract, not
the code. The mechanism section (fork witness absent on Linux, `newcomers_from_live`'s
still-live-forker requirement) was accurate then and is unchanged now; what was missing was
the comparison of the measured `TerminationReport` against `same_poll_orphans` — once made,
the behaviour is inside the documented window, not outside it. The finding survives, sharper
for the correction: not "the code is wrong," but "the documented exception is wide enough to
matter in the ordinary case, and that is worth a ruling."
