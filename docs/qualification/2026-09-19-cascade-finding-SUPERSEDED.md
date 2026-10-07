# The gate is red at the pin, and the red is a race between the kill cascade and the walk

> **SUPERSEDED — do not act on this document.** Its frequency table stands; its mechanism and
> its recommended fix do not. See `2026-09-19-cascade-correction.md`, which disproves the
> mechanism by measurement and shows why option 2 here would have produced a false `Confirmed`.
> Kept because the pin commit `7f01234` and the `v0.1.0` tag both refer to this exchange, and
> because the reasoning that was wrong is the point.


Found 2026-09-19 running step 1 (the gate) at pin `ef7f309`, before any quota was spent.
**No journey has been run. Nothing is void.**

## The measurement

`cargo test --workspace --offline --no-fail-fast` → `rc 101`. One test, on macOS:

```
crates/goalcup-daemon/tests/process_kill.rs:261
a_native_binary_interior_to_the_root_does_not_orphan_its_own_children_out_of_the_walk

TerminationReport { strategy: NativePid, targeted: [92092, 92101, 92110],
  survived_term: [], killed: [], still_alive: [], all_gone: false,
  observed_gone: false, walk_cap_exceeded: false, waited_ms: 295,
  guard_degraded: true, harness_unverified: true }
  left:  [92092, 92101, 92110]
  right: [92025, 92035, 92092, 92101, 92110]
```

Everything else is green: `build`, `fmt`, `clippy` all `rc 0`; criterion 2 `rc 0`;
criterion 3 `rc 0`.

## Frequency — this is a flake, and the variable is NOT what it looks like

| Condition | Result |
|---|---|
| The test alone, `--test-threads=1` | **10 / 10 pass** |
| The test alone, all 12 cores saturated with busy loops | **20 / 20 pass** |
| The whole `process_kill` binary, `--test-threads=1` | **5 / 5 pass** |
| The whole `process_kill` binary, **default parallelism** | **7 pass / 3 fail of 10** |
| Same, with the only other `codex`-named sibling excluded | **5 pass / 5 fail of 10** |
| Full `cargo test --workspace` | 1 red, then 1 green (n=2) |

CPU load does not cause it — that hypothesis was tested and **disproven** at 20/20. Nor is
it the sibling fixture that also spawns a process named `codex`; excluding that sibling left
the failure rate unchanged. The trigger is **in-binary test parallelism**, and the mechanism
below says why that is the relevant variable.

`guard_degraded: true` is a red herring: it is
`handle.argv.first().and_then(file_name).is_none()` (`terminate/authority.rs:441`), and this
fixture's handle is built with `argv: Vec::new()` (`process_kill.rs:1191`). It is constant
for this test and says nothing about the failure.

## Mechanism — measured, not inferred

The process table was sampled every 50 ms from outside the test, with no code change. Two
consecutive snapshots 154 ms apart, across the termination:

```
T=1789806042.147   92025  92002  wrapper-native-interior.sh   (root, registered)
                   92035  92025  .../native_interior/codex    (interior)

T=1789806042.301   92035  92025  (codex)                      ← zombie; 92025 GONE
```

The root and the interior both left **on their own**, and the fixture says exactly why:

* `wrapper-native-interior.sh` runs the interior in the background and then blocks in `wait`.
* `run_native_interior_role` (`tests/support/tree.rs:116`) spawns three `sleep 300` children,
  each its own process-group leader, then **waits on all three and `exit(0)`s**.

So signalling the three children first — which is correct, descendants before the root
(`terminate/mod.rs:359` PHASE A, root last at `:374`) — makes the interior's `wait` return,
so the interior exits, so the root's `wait` returns, so the root exits. **Killing the leaves
collapses the trunk.**

`terminate` then walks on to signal the interior, finds it no longer under
`descendants(handle.pid, …)` because the root is gone, and takes the early
`containment_unverified` return (`terminate/authority.rs`), which is `all_gone: false,
observed_gone: false, harness_unverified: true` **by construction**. Hence `waited_ms: 295`
against a `grace_seconds: 10` — it aborted at once, using 3% of the grace.

Whether the walk beats the cascade is a race. Single-threaded it wins every time; under
parallel sibling tests it loses 30–50% of the time.

## What this is, and what it is not

**It is not a false `Confirmed`.** The failure direction is fail-closed: the tracker refuses
to claim. That is the safe direction and the contract's intent.

**It is a test expectation stronger than the contract** — the same class as the F0 Linux
residual adjudicated on 2026-09-18, where a test asserted a property `I1` never promised.
The assertion demands all five processes be *explicitly targeted*; three of them dying as a
direct consequence of signalling the other two is an equally correct — arguably better —
outcome, and the test cannot tell the two apart.

**But it also exposes something worth the owner's attention.** At the moment the report says
`observed_gone: false`, the entire tree is in fact dead. The tracker had lost the *authority*
to traverse (its root was gone), so it reported that it could not verify — without taking the
one cheap step that would have settled it: a final by-number liveness sweep of the set it had
already named. `containment_unverified` conflates **"I could not look"** with **"I looked and
could not tell."**

## Why this matters for journey A, before quota is spent

Journey A must produce `TerminationOutcomeV1::Confirmed`. `Confirmed` requires
`observed_gone`. If a real Codex tree collapses promptly once its children are signalled —
the same shape this fixture was built from, per `2026-09-15-codex-recon.md` §3, which is the
*measured* Codex shape — then the deadline sweep can land on this same path and return
`Unresolved`. That is a finding, not a retry, and it would cost a journey of quota.

This is the argument for settling it **before** step 3, not after.

## The decision, which is the owner's

1. **Test-only.** Serialise this fixture against its siblings, or accept the cascade outcome
   in the assertion. Cheapest; changes no contract; leaves the `Unresolved`-on-cascade
   behaviour in the product and therefore leaves the journey-A risk in place.
2. **Contract change.** On the `containment_unverified` path, take one final by-number
   liveness sweep of the already-named set; if every one is gone, report `observed_gone`.
   This turns the flake green *and* reduces the chance journey A returns `Unresolved`. It is
   a change to the most safety-critical code in the tree and needs its own review — the
   reason it is not being done unilaterally.
3. **Accept and disclose.** Pin anyway, record the flake as a known non-deterministic gate,
   and let criterion 1 rest on a green re-run. Honest only if stated plainly, and it leaves
   criterion 1 non-reproducible, which is the thing the gate exists to prevent.

Whatever is chosen, options 1 and 2 are code changes and **re-pin**: the gate must be re-run
and the pin moved before any journey.
