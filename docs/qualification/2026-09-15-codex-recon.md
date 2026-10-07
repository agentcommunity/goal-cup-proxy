# Lane Q0 — real Codex reconnaissance (2026-09-15)

**Not qualification.** No production code was changed. This document is the only file this
lane commits. It answers the five questions in the Lane Q0 card of
`docs/superpowers/plans/2026-09-15-v1-harvest-and-finish.md` with a real Codex CLI on this
Mac, and hands each answer to the lane that needs it.

## Headline, for the orchestrator

**A survivor appeared. Three runs out of three. V1 stays open under T-1.**

1. **A real process outlives the sweep, every time.** With a tool call still running, the
   tree at the kill is five processes; `terminate()` signals **two** of them, aborts after
   83-146 ms without ever using its 10-second grace, and the `exec` tool's child —
   `sleep 240`, i.e. any shell command the agent chose to run — is left alive, reparented to
   init, still running minutes later. The cause is not a lost root and not the walk cap: the
   root was **alive and still verifying** at the abort. `discover` promotes the
   `kill.native_binary` match to the front of the target list, and on Codex that process is
   the **parent** of every remaining target, so the first SIGTERM orphans the rest and they
   leave the ppid walk before they can be signalled. §7 D1 has the reproduction recipe.

   The report itself is honest about this — `still_alive: []` beside
   `harness_unverified: true` is the documented "containment unverified, no claim", not a
   false statement of safety. The defect is the abort, not the reporting.

2. **And a second blocker that fixing the sweep will not clear.** On macOS
   `signal_descendant` returns `Some(false)` unconditionally, which clears
   `atomically_bound`, and `all_gone` is `atomically_bound && …`. The only macOS path that
   can set `all_gone` requires the whole tree to be the **one** registered process, and a
   Codex launch is never one process (npm wrapper → native binary → `codex-code-mode-host`).
   Since `Confirmed` is gated on `all_gone && !harness_unverified` (`runtime/seal.rs:1274`),
   **`TerminationOutcomeV1::Confirmed` is structurally unreachable on macOS for `codex-cli`**
   as the trunk stands. The existing suite already encodes this
   (`tests/process_kill.rs:100-117`, `:262-289`). This one is a stop-and-ask, not a Lane T
   task, and it should reach the owner before Lane T spends a second pass on it.

3. **The pins are clean.** Nothing found here implies a connector change, so there is no
   stop-and-ask on pins. `codex --version` on the isolated install is exactly `0.153.0`, the
   manifest digest is the pinned one, and the connector-directory digest is the pinned one.
   **But the `codex` on this machine's default `PATH` is 0.143.0**, a Homebrew cask, and
   `goalcup doctor` correctly FAILs on it. Only `harnesses/env.sh` puts the pinned 0.153.0
   in front. Lane Q1 must source it, or its journeys die at doctor.

4. **Two live modals ate turns.** The connector already silences the update modal. A
   *second* one — "Approaching rate limits · Switch to gpt-5.6-luna?" — appeared mid-run on
   this account and **swallowed a whole typed turn**: the text and its Enter went into the
   selector, the model was switched, and the prompt was never delivered. Codex's own key for
   it is `hide_rate_limit_model_nudge` (same `[tui]` notice group as the
   `[tui.model_availability_nux]` the connector already documents). Adding it is a
   `connectors/` edit, therefore a stop-and-ask, therefore **not** something this lane did.

## 1. Environment tuple

| Field | Value |
|---|---|
| Host | macOS 26.6.2 (build 25G83), arm64 |
| Trunk | `v1/recon` at `2222ef281a02d3962de376ef6b6cb71ca0078441` (tag `wave-0-green`) |
| Toolchain | rustc 1.98.0 (88d9e12ae 2026-08-18) (Homebrew), all cargo commands `--offline` |
| Cargo target | private, `…/scratchpad/target-laneQ0` (never the repo's `target/`) |
| Codex used | `harnesses/node/bin/codex` → npm `@openai/codex`, **codex-cli 0.153.0** |
| Codex native binary | `…/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex`, sha256 `a29d9e86eef88cbbd69f97ce8c590b1d0a287c8f77424f5eef226b883d7eaa22` |
| Codex on default `PATH` | `/opt/homebrew/bin/codex` → Caskroom 0.143.0 — **outside the pin** |
| `CODEX_HOME` seed | `harnesses/home/codex` (isolated). The user's real `~/.codex` was never read or written |
| `HOME` for every command | a scratchpad fake home holding only a copy of the isolated `auth.json`, so `[home_seed]`'s `{home}/.codex` could not reach the participant's own |
| Daemon | debug build from this worktree; `config_dir`, `artifact_root`, `workspace_parent` all under the scratchpad; control `127.0.0.1:17787`, upload `127.0.0.1:17789`, receiver `127.0.0.1:50076` (ephemeral) |
| Concurrency caveat | Lane T's `process_kill` fixtures and at least one other lane's `cargo build` were running on this Mac throughout. Every latency number below is from a loaded machine and is an upper bound |

## 2. Question 1 — version and pin status

```
$ . harnesses/env.sh && codex --version
codex-cli 0.153.0

$ shasum -a 256 connectors/codex-cli/manifest.toml
25b5280e49aefe728fb726d409ec8634ca6bfcbcc5fef35997c68d9ef6d92d1f
```

| Pin | This build's constant | Measured | Verdict |
|---|---|---|---|
| `tested_version_range` | `=0.153.0` | 0.153.0 (isolated install) | in range |
| `CONNECTOR_MANIFEST_SHA256S["codex-cli"]` (`goalcup-verifier/src/lib.rs:98`) | `25b5280e…d92d1f` | `25b5280e…d92d1f` | equal |
| `CONNECTOR_DIR_SHA256S["codex-cli"]` (`goalcup-protocol/src/manifest.rs:182`) | `028548719a…b035af` | `028548719a…b035af` reported live by `goalcup doctor` | equal |

`scripts/pin-manifests.sh` was **never run** — it writes. The directory digest above is the
one `goalcup doctor --harness codex-cli` printed from the running daemon, which is a read.

`goalcup doctor --harness codex-cli` with `harnesses/env.sh` sourced: **every line PASS,
exit 0** (daemon, tier, manifest_pin, harness_version, banned_env, required_paths,
free_ports). Without it, `harness_version` FAILs with 0.143.0 and doctor exits 1 — reproduced
deliberately, and it is the single most likely way Lane Q1 loses an hour.

**No pin change is implied.** Nothing here is a stop-and-ask.

## 3. Question 2 — the real process tree

Run A: `goalcup doctor` → `goalcup start --dev --harness codex-cli`, driven through a real
pty by `spikes/wire-pty/tools/ptyrec.py` (throwaway spike tooling, not this lane's code).
Entry `ee08fd85b2c669230`, session `01a0a4d7-b24d-7270-9101-0853c45642ec`, harness reported
`0.153.0`, arming `started`, health `online`.

`ps -axo pid,ppid,pgid,sess,comm,args` was sampled every 5 s for the whole run and every
0.3 s for the second half. Classification is against the **registered root**, which is the
pid `goalcup start` handed the daemon.

| pid | ppid | pgid | comm | what it is | classification |
|---|---|---|---|---|---|
| 61050 | 61046 (pty driver) | 61050 | `goalcup` | the CLI itself; not registered, not a target | n/a |
| **61092** | 61050 | 61050 | `node` | **the registered root** — the npm wrapper `node …/harnesses/node/bin/codex …` | group member (of the CLI's group, which it does not lead) |
| 61094 | 61092 | 61050 | `codex` | the **native** binary under the wrapper; this is what `find_named` resolves and what `discover` signals first | walked descendant |
| 62390 | 61094 | **62390** | `codex-code-mode-host` | Codex's own MCP/code-mode host ("Booting MCP server: codex_apps" on screen) | **walked descendant, but in a process group of its own** |

Three things matter here.

**(a) The registered root is `node`, not `codex`.** `ProcessHandle.pid` is the npm wrapper.
This is precisely the M0 §2.5 shape the module header describes, and it is live today: the
connector's `kill.native_binary = "codex"` is doing real work, and `discover` correctly
promotes 61094 to the front of the target list.

**(b) `pgid` is `None` and the root does not lead a group.** `mode = "tui"` means
`launch::spawn` never calls `process_group(0)`; the wrapper simply inherits the pgid of
whatever started `goalcup`. So `requested_group` is `None`, `owns_registered_group` is
false, and the **descendant walk is the only containment mechanism**. There is no group
signal to fall back on.

**(c) `codex-code-mode-host` is in its own process group.** It is reachable by ppid only.
The moment its parent (61094) exits, it reparents to pid 1 and leaves the walk — and,
because it is not in the root's group, a group signal would not have reached it either.

**Short-lived members were never caught in a `ps` snapshot at all.** Sampling every 0.3 s
across the whole run captured the hook shim (`/bin/sh …/.codex/hooks/hook-post.sh <Event>`)
and its `curl` **zero times**, which is consistent with the measured shim latency below
(p50 ~16-26 ms). They exist — the receiver logged every post — but they are not a termination
hazard, because nothing that lives 20 ms is alive at a seal.

The run-B trees in §4 are the ones that show what a kill actually meets, because they were
taken while a tool call was deliberately still running.

**What this session did and did not exercise.** It did: `SessionStart`, `UserPromptSubmit`,
a real `exec` write (`apply_patch` through `exec`, creating `hello.py`), a real `exec` shell
run (`python3 hello.py`), the hook shim and its curl, and Codex's own bundled MCP/code-mode
host. It did **not** get a subagent turn: the rate-limit modal described above swallowed the
`use a subagent …` prompt, so `SubagentStart`/`SubagentStop` are **not** covered by this
recon. Nobody should read the tree table as complete for subagents.

### The tree the kill actually meets (repetitions 1-3, identical shape)

Taken by the recon test at mid-session, with a tool call deliberately still running:

| pid (rep 3) | ppid | pgid | comm | classification |
|---|---|---|---|---|
| 30173 | 30157 (the test process) | 30157 | `node` — **the registered root**, the npm wrapper | group member; does **not** lead the group |
| 30177 | 30173 | 30157 | `codex` — the native binary | walked descendant, same group |
| 30288 | 30177 | **30288** | the declared stdio MCP server | walked descendant, **own process group** |
| 31830 | 30177 | **31830** | `codex-code-mode-host` | walked descendant, **own process group** |
| 31836 | 30177 | **31836** | `sleep 240`, the `exec` tool's child | walked descendant, **own process group** |

**Every process Codex spawns gets its own process group.** Only the npm wrapper and the
native binary share the launcher's group, and the launcher's group is not one the tracker
created. A `killpg` would reach two of five even if a TUI launch could take a group — which
it cannot. The ppid walk is the entire containment mechanism, and §4 is about what happens
to it.

## 4. Question 3 — does the sweep leave a survivor? **Yes. Three times out of three.**

Method, and what was hacked to get it. A throwaway `#[ignore]` integration test at
`crates/goalcup-daemon/tests/recon_codex.rs` — **uncommitted, listed in §8** — launches the
real Codex through the same shape `goalcup_cli::launch::spawn` uses for `mode = "tui"`
(`env_clear`, the manifest's `env_base`/`env_set`/`env_remove`, the manifest's argv,
**no `process_group(0)`**), builds a `ProcessHandle` field for field as
`goalcup_cli::launch::handle` builds one, drives a real session, and calls
`goalcup_daemon::terminate::terminate(&handle)`. No production file was modified:
`ProcessHandle` is public with public fields and `pub mod terminate` is exported, exactly as
the plan's pre-dispatch note says. The test process itself is run under
`spikes/wire-pty/tools/ptyrec.py` so Codex inherits a real controlling terminal and the
registered root is `codex`'s own launcher rather than a driver.

The session in each repetition: Codex boots, its bundled `codex_apps` MCP host boots, a
declared stdio MCP server boots, and one prompt asks for a long-running shell command
(`sleep 240`) so a tool child is **provably still running at the instant of the kill**.
That is the case a deadline seal has to survive, and it is the case a short polite session
never reaches.

### The three reports, raw

| rep | `targeted` | `survived_term` | `killed` | `still_alive` | `all_gone` | `waited_ms` | `guard_degraded` | `harness_unverified` |
|---|---|---|---|---|---|---|---|---|
| 1 | `[88954, 88947]` | `[]` | `[]` | `[]` | `false` | 83 | `false` | `true` |
| 2 | `[2387, 2365]` | `[]` | `[]` | `[]` | `false` | 88 | `false` | `true` |
| 3 | `[30177, 30173]` | `[]` | `[]` | `[]` | `false` | 146 | `false` | `true` |

```json
{ "strategy": "native_pid", "targeted": [30177, 30173], "survived_term": [], "killed": [],
  "still_alive": [], "all_gone": false, "waited_ms": 146,
  "guard_degraded": false, "harness_unverified": true }
```

(Repetition 3 carried extra `ps` instrumentation around the kill, which is why its
`waited_ms` is larger. The handle, the launch and the `terminate` call are identical to 1
and 2.)

### What `still_alive: []` does and does not say

It is **not** a false claim. `terminate.rs:126-129` defines the field as "everything the
last trustworthy sweep still found running … can be empty beside `all_gone: false` when
containment itself became unverified and no remaining process can be named safely", and
`:148-151` says `harness_unverified` means "this daemon cannot say what remains of the
harness". So the trio `still_alive: [] / harness_unverified: true / all_gone: false` reads
**"containment became unverified; I make no claim about what remains"**, which is the truth
and is exactly the M0 §2.5 lie the module was written to refuse. The module is honest.

### The defect is the abort, and it is a reparenting race in practice

Five processes were alive at the kill in every repetition. Two were signalled. The walk then
aborted after 83-146 ms — far inside the 10-second grace, which never ran — through the early
return at `terminate.rs:286-294`, because `signal_descendant` returned `None` for the third
target.

Repetition 3 resolves *which* `None`. A zero-delay `ps` the instant `terminate` returned:

```
30173  ALIVE  ppid=30157  pgid=30157  comm=node          <- the REGISTERED ROOT, still live
30177  GONE                                              <- the native codex
30288  GONE                                              <- the declared stdio MCP server
31830  GONE                                              <- codex-code-mode-host
31836  ALIVE  ppid=1      pgid=31836  comm=sleep         <- the exec tool's child, ORPHANED
```

The root was **alive and still verifying** when the walk aborted (it went `<defunct>` only
~500 ms later). So `verified_snapshot` did not fail on the root. The `None` came from the
other branch, `!descendants(handle.pid, &table).contains(&pid)`: **the target had left the
ppid walk between the snapshot that chose it and the re-verification before signalling it.**

Why it leaves is the ordering. `discover` (`terminate.rs:523-544`) builds the list from
`descendants()` — a correct BFS that found all five — and then **moves the `native_binary`
match to the front**. On Codex the native binary is the *parent* of every other target:

```
node (root, registered)          <- signalled 2nd
└── codex (native)               <- signalled 1st, because discover promotes it
    ├── <stdio MCP server>       <- never signalled
    ├── codex-code-mode-host     <- never signalled
    └── sleep 240 (exec tool)    <- never signalled; SURVIVES
```

Signalling the native binary first kills the parent of the remaining three. They reparent to
init, leave the walk, and the very next `signal_descendant` refuses them and aborts the whole
escalation. The promotion that exists to fix M0 §2.5 is what makes this deterministic.

### Which of the three actually survived, and why the other two did not

The MCP server and `codex-code-mode-host` were gone by the time the report came back — but
**not because anything signalled them**. Both are stdio children: their `fd 0/1/2` are pipes
to the native codex (confirmed with `lsof`: `PIPE` on 0/1/2 plus a unix socketpair), so they
exited on their own when that pipe closed. That is the harness's implementation being
convenient, not containment.

`sleep 240` has no such pipe dependency and **survived every time**, reparented to init, in
its own process group, and was still running minutes later until this lane killed it by hand.
It is the `exec` tool's child — a shell command the participant's agent chose to run, which
Codex places in a "background terminal" (`· 1 background terminal running · /ps to view`).
Any long-lived command an agent starts — a dev server, a watcher, a build — is in this class.

### And a second, larger finding: `Confirmed` is unreachable on macOS regardless

Even with a perfect sweep, these reports could not have said `all_gone: true`.
`signal_descendant`'s macOS arm (`terminate.rs:1017-1028`) ends `signal(pid, sig); Some(false)`
— **unconditionally false** — because macOS has no pidfd-class instance-bound signal. That
clears `atomically_bound`, and

```rust
all_gone: atomically_bound && !target_limit_exceeded && still_alive.is_empty(),
harness_unverified: !atomically_bound || target_limit_exceeded,
```

The only macOS path that can set `all_gone` is `terminate_darwin_singleton`, gated on
`descendants(handle.pid, &table) == [handle.pid]` — a **one-process** tree. A Codex launch is
never one process: the tracker launches the npm wrapper, which spawns the native binary,
which spawns the MCP host. The suite already encodes this
(`tests/process_kill.rs:100-117`, `:262-289` both `cfg!(target_os = "macos")`-branch to
`harness_unverified && !all_gone`), and `end_harness_verified` gates on
`report.all_gone && !report.harness_unverified` (`runtime/seal.rs:1274`).

So `TerminationOutcomeV1::Confirmed` for `codex-cli` on macOS is blocked by **two**
independent things, and fixing the sweep fixes only the first.

## 5. Question 4 — real hook post rate and latency

Counted three ways, and the three agree: the shim's own `{artifact_root}/seq-codex-hook-post`
counter, the leg-2 records in the sealed chain, and a stub receiver's millisecond log.

**Run A, the full live entry `ee08fd85b2c669230`** — 519 s of session, two delivered turns,
five tool calls:

| Event | Posts |
|---|---|
| `SessionStart` | 1 |
| `UserPromptSubmit` | 2 |
| `PreToolUse` | 5 |
| `PostToolUse` | 5 |
| `Stop` | 2 |
| `SessionEnd` | 1 |
| **total** | **16** |

| Measure | Value |
|---|---|
| Mean over the whole session | **0.031 posts/s** (16 posts / 519 s) |
| Peak in one recorded second | **4 posts/s** (two `PreToolUse`/`PostToolUse` pairs in the same second) |
| Tightest observed spacing (stub receiver, ms resolution) | 16 ms between `SessionStart` and `UserPromptSubmit` — ~60 posts/s *instantaneous*, two posts deep |
| Shape | bursty per tool call, silent between turns. 5 of 396 sampled seconds had any post at all |
| Body sizes | 536 B (`SessionStart`), 663 B (`UserPromptSubmit`), 678-1312 B (`PreToolUse`), 1233-**4481 B** (`PostToolUse`). 16 posts totalled 20.9 KB |

**`connectors/codex-cli/files/hook-post.sh` latency**, rendered exactly as the daemon renders
it, 400-byte `PreToolUse` body, loopback receiver, measured end to end including `sh` and
`curl` process start:

| Run | n | min | p50 | p95 | p99 | max |
|---|---|---|---|---|---|---|
| During the live session (loaded machine: another lane's `cargo test` plus Codex) | 60 | 13.5 ms | **25.8 ms** | **60.7 ms** | 461 ms | 947 ms |
| After the live runs (quieter) | 80 | 13.6 ms | **15.9 ms** | **34.2 ms** | 41.7 ms | 42.9 ms |

The loaded-machine tail is contention, not the shim: the same script on the same receiver is
~16 ms p50 when nothing else is building. Lane R should size against the loaded column,
because a qualification run *is* a loaded machine. The shim is short-lived enough that a
0.3 s `ps` sampler never once caught it or its `curl` in the process table.

**The budget in force today: there is none.** At `2222ef2` `crates/goalcup-daemon/src/config.rs`
has no rate, burst or per-entry post budget of any kind. The only receiver-side bound is
`receiver::MAX_POST_BYTES = 8 MiB` per post (`receiver.rs:224`), which the largest real Codex
post misses by three orders of magnitude; the upload side has `upload_chunk_max = 256` and
`upload_interval_ms = 2000`. `posts_dropped` was 0 for the whole run. Lane R is choosing a
first budget, not tightening an existing one — and a budget anywhere above single-digit
posts/s would not have been touched by anything measured here.

## 6. Question 5 — fixture sequencing

`crates/goalcup-verifier/tests/end_to_end.rs:957-980` is already explicit and nothing found
today changes it. `the_live_codex_run_verifies` reads
`crates/goalcup-verifier/tests/traces/codex-live/`, an authentic 2026-09-08 export whose
start receipt binds the **pre-`e105847`** Codex manifest and connector-directory digests.
Against this build it correctly reports `manifest_not_pinned` and `connector_dir_unpinned`
as Fails where its committed expectation says `flag`, which is why it carries
`#[ignore = "re-record after final connector pins settle …"]`.

**The statement, for Lane Q1 and Lane V:**

- The export must be **re-recorded exactly once**, from Q1's **journey 1** trace on
  `v1-final-candidate`, **after** the connector pins are final and the binary bytes are
  frozen. Editing the export is how a trace stops being one; restoring its old pins would
  erase the evidence V13 exists to keep.
- Step 1 of this lane found **no reason for a pin change**. The manifest file digest, the
  connector directory digest and `tested_version_range` all match the shipped constants
  against the isolated 0.153.0 install. So the "after pins are final" precondition is
  satisfied by the pins *as they stand*, and there is nothing here to stop and ask about.
- The one thing that *would* force a pin change is silencing the rate-limit modal
  (`hide_rate_limit_model_nudge` in `connectors/codex-cli/files/config.toml`). That is a
  `connectors/` edit and therefore an owner decision, and it is raised in §7 rather than
  taken. If the owner approves it, `sh scripts/pin-manifests.sh` goes in the same commit and
  **both** the re-record and Q1's journeys must happen after it, not before.
- `the_live_opencode_run_verifies` stays ignored with its reason updated to "V1.1 surface".
  Nothing in this lane touched it.

## 7. Defect report for Lane T

**One defect, two independent blockers.** Neither is a flake: 3/3 repetitions, identical.

### D1 — the escalation aborts after signalling the parent of everything it had left to signal

| Field | Value |
|---|---|
| Survivor `comm` | `sleep` |
| Survivor argv | `sleep 240` — the child of Codex's `exec` tool, i.e. any shell command the agent chose to run |
| At kill time | `ppid=30177` (the native `codex`), `pgid=31836` (**its own group**), `sess` 0 |
| Immediately after `terminate` returned | `ppid=1`, `pgid=31836`, still alive; still alive minutes later |
| Did it leave the group before the first snapshot? | **No.** It was never *in* the root's group — Codex puts every child it spawns in a fresh process group. It was in the ppid walk at all three snapshots |
| Did it hit the walk cap? | **No.** `MAX_TARGETED` is 4096; the tree was 5 processes and `target_limit_exceeded` is false |
| Did it reparent mid-kill? | **Yes, and that is the defect.** It reparented when `terminate` itself killed its parent |
| Was the registered root lost? | **No.** The root was alive and still verifying when the walk aborted |

**Mechanism.** `discover` (`terminate.rs:523-544`) promotes the `kill.native_binary` match to
the front of a list that also contains that process's own descendants. For `codex-cli` the
native binary is the *interior* node of the tree. SIGTERM therefore reaches the parent first;
its children reparent to init; the next `signal_descendant` (`:1017-1028`) re-verifies, no
longer finds the target under `descendants(handle.pid, …)`, returns `None`; and
`terminate.rs:286-294` returns `containment_unverified` with two of five signalled, the
10-second grace unused, and `survived_term`/`killed` empty.

Note that plain BFS order has the same hazard one level deeper: `descendants()` returns
parents before children, so even without the promotion a grandchild is signalled after its
parent. The promotion makes it certain rather than likely.

**Direction, not a prescription — Lane T decides.** Signal leaves-first (reverse the BFS
order, keeping the native-binary promotion *within* its depth), or hold the whole target list
from one authenticated snapshot and allow a signal to a pid that has reparented **provided
that snapshot proved it a descendant and its instance identity still matches**. The second is
the one that also covers a harness that forks and detaches on its own. Neither weakens
`Confirmed`.

**Reproduction with a stand-in, no Codex required.** Three generations, where the
`native_binary`-named process is the middle one and its children escape to their own groups —
`crates/goalcup-daemon/tests/fixtures/` already has `wrapper-generations.sh` to build on:

```sh
#!/bin/sh
# root (registered).  argv must match the handle, as every fixture's does.
"$DIR/native/codex" &        # a long-lived binary whose `comm` is `codex`
wait
```

and inside that `codex` stand-in, three children each in a **new process group** (a tiny
`setsid`-alike helper, since macOS ships no `setsid(1)` — or `Command::process_group(0)` from
a Rust helper binary), one of them a `sleep 240` that ignores nothing and depends on no pipe.
Build the handle with `pgid: None`, `strategy: NativePid`, `native_binary: Some("codex")`,
`grace_seconds: 10`. Assert: `targeted` contains **all five** pids, and the `sleep` is not
alive afterwards. Today that test fails with `targeted.len() == 2`.

### D2 — `Confirmed` cannot be produced on macOS for any multi-process harness

Independent of D1 and not fixable by fixing the sweep. `signal_descendant`'s macOS arm returns
`Some(false)` unconditionally, so `atomically_bound` is false, so `all_gone` is false and
`harness_unverified` is true for every macOS tree that is not a **single** process. Codex is
always at least three (npm wrapper → native → `codex-code-mode-host`). The `Confirmed` gate
at `runtime/seal.rs:1274` is `report.all_gone && !report.harness_unverified`.

So acceptance criterion 4 — `TerminationOutcomeV1::Confirmed` on a real Codex run on macOS —
**cannot be met by wiring alone**. Lane T should say so explicitly rather than discover it at
the `api_gate` assertion. Under T-1 the bar does not move: either Darwin grows an
instance-bound path that covers a tree (not just the singleton `terminate_darwin_singleton`
covers today), or **V1 is held open and says so**. This is a stop-and-ask, and it is the one
the orchestrator should carry to the owner first.

## 8. Limitation text for Lane D

D1 is fixable and should not be written down as a limitation. What is inherent is narrower,
and this is offered as wording for `docs/CHEAT-CHECKLIST.md`:

> **A command the agent left running can outlive the seal.** The tracker ends the harness by
> walking the process tree it launched and signalling what it finds. Codex runs each shell
> command the agent asks for in its own process group, as a child of the Codex binary — so
> when that binary exits, anything it was still running is reparented to the operating system
> and is no longer connected to the run by anything the tracker can read. The seal says so
> rather than claiming the machine is clean: a termination that could not account for the
> whole tree is recorded as unresolved, never as confirmed. What this cannot do is *find*
> such a process afterwards, because on macOS nothing short of a privileged component can
> prove that a process with no surviving ancestry came from this run. A participant who
> starts a long-running server during a run should expect to stop it themselves.

And the second, which belongs beside it because it is a platform limit rather than a defect:

> **On macOS the tracker cannot prove a whole process tree is gone.** Signalling a process by
> number is not atomic with checking which process that number belongs to, and macOS offers no
> handle that binds the two for anything but a single process. Every termination of a harness
> that is more than one process is therefore recorded as unverified, whatever actually
> happened. Linux, which has `pidfd`, does not have this limit.

## 9. What was hacked, and is NOT committed

Everything below lives in this lane's worktree or scratchpad and is deliberately left
uncommitted. The next person should assume none of it is in the tree.

| Path | What it is |
|---|---|
| `crates/goalcup-daemon/tests/recon_codex.rs` | the throwaway `#[ignore]` recon test. ~430 lines. It compiles and passes under `cargo test -p goalcup-daemon --offline --test recon_codex -- --ignored` **only** with the `RECON_*` environment it expects. Delete it or leave it uncommitted; it is not a regression test and its assertions are `eprintln!`s, not asserts |
| `…/scratchpad/target-laneQ0` | the private `CARGO_TARGET_DIR`. Never `target/` in the worktree |
| `…/scratchpad/gc/` | daemon config, artifact root, workspace parent, and run A's sealed trace. Daemon and devserver were stopped at the end of the lane |
| `…/scratchpad/fakehome/.codex/auth.json` | a copy of the **isolated** `harnesses/home/codex/auth.json`. Every command in this lane ran with `HOME` pointed here so `[home_seed]`'s `{home}/.codex` could not reach the participant's real one. The real `~/.codex` was neither read nor written |
| `…/scratchpad/runB/mcp_stub.py`, `stub_receiver.py` | a minimal stdio MCP server and a logging receiver, both throwaway |
| `spikes/wire-pty/tools/ptyrec.py` | **pre-existing, untracked** spike tooling in the main checkout, used unmodified to give Codex a controlling terminal |

No production file was modified. `scripts/pin-manifests.sh` was never run. Nothing under
`connectors/` was touched. `git status` in this worktree shows exactly one added file,
`docs/qualification/2026-09-15-codex-recon.md`, plus the untracked throwaway test.

## 10. Things found in passing that other lanes should know

- **The `use a subagent …` turn never happened.** The rate-limit modal ate it, so
  `SubagentStart`/`SubagentStop` are unmeasured by this lane. Q1's journeys should treat
  subagent activity as unvalidated ground.
- **V3/V8 flagged a real join mismatch** on run A: turn `01a0a4d7-f6d5-7762-b670-5085b4ff31fe`
  has 2 leg-1 tool calls against 4 leg-2 tool calls. Codex's "background terminal" is the
  likely cause and it is worth a look before Q1 records a journey.
- **V2 failed for a reason that is this lane's fault, not the build's**: the driver's second
  delivered prompt was not the literal `keep going` the rule requires. Not a defect.
- **`codex-code-mode-host` appears in every session** — 4 sessions out of 4, booting within
  ~10 s of launch, before the first prompt, announced on screen as
  "Booting MCP server: codex_apps". It takes **no argv** beyond its own path and speaks over
  inherited stdio pipes plus a unix socketpair. It is not optional tooling that a clean
  qualification run might miss.
- **macOS `ps -o sess` prints `0` for everything** on this host, so "foreign session" could
  not be used as an escape test. The classification above uses `ppid` and `pgid`, which do
  discriminate.
