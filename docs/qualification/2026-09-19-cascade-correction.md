# Correction: the red is `/bin/ps` under contention, not the kill cascade

> Committed 2026-09-21. The pin commit `7f01234`'s message and the `v0.1.0` tag name this file
> at its original out-of-tree path, `goalcup-evidence-2026-09-19/CORRECTION-cascade-vs-walk.md`.
> That path was where it was written; this is the same content, in the repository, so the
> reference resolves after the working directory is cleaned up. It corrects
> `2026-09-19-cascade-finding-SUPERSEDED.md`.


Measured 2026-09-21 on the owner's Mac, at the same pin `ef7f309`, before any quota was
spent and before any code change. **No journey has been run. Nothing is void.**

This corrects `FINDING-cascade-vs-walk.md`, which is wrong about the mechanism and therefore
wrong about the decision. Its frequency table stands; its explanation does not.

## What the finding got wrong

It says the walk reaches the interior, "finds it no longer under `descendants(handle.pid,…)`
because the root is gone, and takes the early `containment_unverified` return."

That path does not return early. `authority.rs:265`:

```rust
if !owned.contains(os, &table, rooted, pid) {
    return (!alive(os, pid)).then_some(false);
}
```

An unowned pid that is **not alive** returns `Some(false)` — signalling continues, the pid is
recorded in `targeted`, nothing fails closed. Since `wrapper-native-interior.sh` has exactly
one child and blocks in `wait`, the interior is always already dead when the walk reaches it.
The cascade is real, and it is **already handled correctly**.

## What actually happens — measured with probes in the two returns

Instrumented `sweep_anchor` and `signal_owned`, ran the `process_kill` binary in six
concurrent copies, three rounds:

```
PROBE signal_owned pid=95619: unowned rooted=false alive=false   <- the cascade, handled
PROBE signal_owned pid=97018: Anchor::Lost                       <- the actual red
PROBE containment: phase A escalate members=[96571, 96580]
```

`Anchor::Lost` is returned only when `guard_registered` or `os.snapshot()` returns `Err`
(`mod.rs:878-890`) — that is, when **`/bin/ps` fails to spawn**. Under contention it does.

## Frequency, re-measured

| Condition | Result |
|---|---|
| `cargo test -p goalcup-daemon --test process_kill`, default parallelism, serially | **0 fail / 30** |
| Shell fork/exec pressure (16 loops spawning `ps`) alongside | **0 fail / 10** |
| **Six concurrent copies of the `process_kill` binary** | **9 fail / 18** |

The finding's variable — in-binary parallelism — is not the variable either. What reproduces
it is concurrent *process-table* contention across whole test binaries.

## It is not one test

Under that load three tests in the binary fail, not one:

* `a_native_binary_interior_to_the_root_does_not_orphan_its_own_children_out_of_the_walk`
* `a_grandchild_in_its_own_process_group_is_still_named_by_the_descendant_walk`
* `a_report_does_not_claim_the_harness_gone_while_a_post_root_death_orphan_lives`

Several failing copies show **no `PROBE containment` line at all**, so containment is not even
the only route. The common cause is upstream of all of them: `ps` is unreliable under load and
every read that depends on it degrades.

## Why option 2 must NOT be implemented

Option 2 was "on the `containment_unverified` path, take one final by-number liveness sweep;
if every one is gone, report `observed_gone`."

That path is reached **because liveness reads are failing**. And `os.rs:132`:

```rust
/// [`ProcessOs::liveness`] folded to the shipped decision: "could not ask" is "gone".
pub(super) fn alive(os: &dyn ProcessOs, pid: u32) -> bool {
    os.liveness(pid).unwrap_or(false)
}
```

A sweep built on that would read a **live** tree as gone and report `observed_gone: true`.
That is a false `Confirmed` — the exact failure class this module exists to prevent. It also
directly violates I4, which `terminate_sim.rs`'s `f1_ps_fails_mid_sweep` already pins:

> any liveness `Err` during the call ⟹ `¬observed_gone ∧ harness_unverified`

The current behaviour is not a defect. It is I4 working.

## What this does to the journey-A risk

The finding's argument for spending a contract change before step 3 was that a real Codex
tree might collapse the same way and return `Unresolved`. It would not: the collapse path
returns `Some(false)` and proceeds. A journey runs one harness on an idle machine, not six
concurrent copies of a process-spawning test binary. **There is no journey-A risk here.**

## What the decision now is

Option 1, and narrower than the finding framed it: the tests assert a property that
`/bin/ps`-under-contention cannot deliver, and the fix belongs in how they are run, not in the
contract. The `#[ignore]`-free, in-tree options are to serialise the process-table tests
against each other, or to have the gate run `process_kill` in its own step — which is exactly
what the `process-kill:` job in `.github/workflows/gate.yml` now does for CI, since
`v1/ci-collapse` (`4dc7673`) merged, for what now looks like the same underlying reason.

Nothing here requires a change to `terminate/`.
