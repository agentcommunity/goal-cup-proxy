# PRD-16: Build orchestration and acceptance

Id: PRD-16. Version: goalcup 1.0. Status: plan for the one-day build, revision 2 (after the three reviews of
30 October). Date: 30 October 2026. Base: main at 4b226de2. Index: [PRD-00](PRD-00-index.md). Ledger:
[OWNER-DECISIONS](OWNER-DECISIONS.md). Format: [PRD-01](PRD-01-record-format.md).

## Goal

Run one build day in which many agents build goalcup 1.0 from PRD-01 to PRD-17, prove it on real harnesses
in Vercel Sandboxes, and leave the owner one page at the end that says "ready to merge" or "not, because".
Nothing reaches `main`, a tag or production without the owner.

## Points served

All seven, as the gate that decides whether the build serves them. 1 and 2: the pad and the acceptance
runs compare the wire and the session files with the harness's own log. 3: the kill and fault runs and
the load runs. 4: secret scans of every record, server copy, log and pushed repository. 5: server copy
checks on staging. 6: counts the participant saw against the verifier's recount. 7: 15 harnesses run the
way people run them, gateways included.

## Owner decisions it carries out

- One big build, many parallel agents, shipped as **goalcup 1.0** (ledger L1). The build **starts in the
  morning** after the owner wakes and the keys are in place (ledger B7). "Night" in file names
  (`NIGHT-LOG.md`) is kept only so other PRDs' references hold.
- The relayed answers of 30 Oct: "1 core OK" and "version 1 sure" confirm the core set (Claude Code,
  Codex, Hermes) and the version. "2 swp" is read as "swap the harnesses in the list", which the 29 Oct
  night list carries out. "3 i can fund one" is read as the one funded Google key. Both readings are
  unverified; PRD-00 owner question 17 holds them.
- **One system for every harness** (ledger H1 to H5). The owner's question, "when it is a gateway, isn't
  it still an HTTPS proxy with a different API endpoint, like AI Gateway under Kilo?": yes. A gateway is one
  more HTTPS host on the one shared host list; the same proxy decrypts and records it, with no gateway
  code (PRD-00 "One system for every harness"). Proxy dodgers use the same system: a pinned or refused
  host becomes a recorded tunnel with a named gap, plus session files, plus an honest label. This PRD proves
  that answer with real runs: the `gateway` rows of the core set, A3 (Hermes through the gateway at length)
  and A4 (Kilo through the gateway, the owner's own example), and the `dodger` rows.
- Option B, one process; fresh core; old crates deleted in the same build after PRD-17's deletion gate.
- **Builds, compiles, tests and real runs happen in Vercel Sandboxes**; this cloud session writes code,
  reviews and orchestrates only (ledger B1).
- **Lanes are parallel workflows in this one session**, a planner per PRD, builders in worktrees, at least
  two independent reviewers per merge request, a third on core parts; separate cloud sessions are a
  fallback only (ledger B2). A replan after each PRD merges (ledger B2a).
- Short real tests during the build, mass testing at the end, watchers that fix and rerun, three failed
  fixes on one cause end that loop with a note (ledger B3).
- **Staging only**: every run uploads to the staging Worker, approved with the staging test approval key;
  agents may deploy staging; production, the merge to `main` and tags are the owner's; no website
  database schema migration (ledger B4 to B6, S3).
- Spend: 10 USD per run (ledger T6). Pad runs are about 20 minutes of real work with many subagents
  (ledger T5). Keys in 0600 files, separate keys per harness where possible, read-only database and R2
  credentials on the conductor only (PRD-00 defaults).
- The long run uses run 1's Codex prompt, so the two runs compare; without it, a load comparison
  (PRD-00 owner question 16).

## Today

| What | Where | Fact |
|---|---|---|
| Workspace members and version | `Cargo.toml:3-9`, `:18` | Five old crates; version `0.7.0`. The comment at `:12-17` says it is the one place a version is written and "the gate fails on any other hard-coded copy" |
| Toolchain | `rust-toolchain.toml` | `1.99.0`, minimal profile, clippy and rustfmt |
| A dev profile for a crate that goes | `Cargo.toml:61-62` | `[profile.dev.package.goalcup-daemon]`, opt-level 1 |
| The gate refuses root | `scripts/gate.sh:32-39` | Some permission tests always fail as root. This container runs as `root` with 4 CPUs (`whoami`, `nproc` = 4) |
| Release script | `scripts/release.sh:7-14`, `:54-61`, `:72-75` | `prepare` opens a version PR (main is protected); `tag` tags `origin/main` only when its version matches; the next-step text names `linux-check.sh` |
| Version floor | `server/worker/wrangler.toml:51-55` (production), `:150` (staging) | `MIN_COMPETITION_VERSION = ""` in both. A dashboard edit is wiped by the next deploy |
| Staging Worker | `server/worker/wrangler.toml:107-164` | `goalcup-ingest-staging`, own bucket `goalcup-traces-staging`, own events, the only deployment that may hold `TEST_APPROVAL_KEY` |
| Event switch | `server/worker/wrangler.toml:36-42` | `EVENT_ID` becomes `goalcup-cup-2026` between 6 Nov 00:00 and 7 Nov 00:00 UTC, by the owner |
| Maintenance switch | `server/worker/wrangler.toml:68-70` | `MAINTENANCE` on holds clients, who keep their data and back off |
| Plan schedule | `docs/plans/2026-10-30-cut-first-plan.md:364-475` | Day-by-day PRs, a second 2-hour run on Monday 2 Nov, a release-candidate run on Wednesday 4 Nov. Superseded by one build |
| Run 1 | run report (scratchpad copy `run-report-74daaa03.md`), on `origin/smart/tender-clarke-qupne3` | 2 hours, Codex 0.165.0, about 20 subagents at once, 42 in all; wire stopped for good at 15:43:15Z; 473 calls (37.5%) and 15 of 42 subagents with no wire record; finish line said 1,736; server stored 1,766; 405 MB uploaded at `-3`, 100.8 MB at `--long=27`; about $1.50 by Codex's own record |
| Run 1 prompt | `docs/research/results/2026-10-30-two-hour-run/` on that branch | **Not in the repository.** `start-pane.txt` shows only the Codex banner; the report says only "as written by the owner" |
| Where the PRDs live | branch `smart/intelligent-newton-0avrpi`, docs-only commits ahead of `main` 4b226de2 | The integration branch starts from `main`, so it has none of the PRDs until they are merged in |
| Agent concurrency | the workflow runner's own limit | At most min(16, CPUs minus 2) agents at once per workflow. On this 4-CPU container that is **2** per workflow; several workflows run at once (ledger B2) |
| Vercel Sandbox | `docs/research/VERCEL-COMPAT.md:41`, `:50`, `:52` | Sessions end at 45 minutes on Hobby, 24 hours on Pro; Pro boxes up to 8 vCPU and 16 GB; $0.128 per active CPU-hour, $0.0212 per GB-hour |
| DeepSeek limits | https://api-docs.deepseek.com/quick_start/rate_limit | Concurrency limits per account, at least 500 requests at once, a 429 past it. So one DeepSeek key can feed six pad boxes at once |

## Design

### 1. Hosts, lanes and roles

| Host | What runs there | Holds |
|---|---|---|
| **Conductor** | This cloud session, 4 CPUs, root. The lane workflows, their agents, git worktrees, `scripts/pad/pad` (conductor half), `fetch-run`, `goalcup-verify` for checks. It never compiles or tests | The approval key, read-only database and R2 credentials, the GitHub token, the Cloudflare token for staging, all key files under `~/.pad-keys/` (0600). Never a production write credential |
| **Build boxes** | Vercel Sandboxes, 8 vCPU and 16 GB, user `ubuntu`, from the build snapshot | The Rust toolchain from `rust-toolchain.toml`, nextest, zstd, Node 22, Python 3, PostgreSQL, a warm cargo cache. No key, no token |
| **Pad boxes** | Vercel Sandboxes from PRD-12's pad image, 4 vCPU, `--non-persistent` | One harness, its key file for the life of the run, `goalcup` and `goalcup-verify` from the build under test |
| **Staging Worker** | `goalcup-ingest-staging` on Cloudflare, 1.0 code from the integration branch | Its own bucket; the shared database through the Worker's own calls only |

Why build boxes: the gate refuses root (`scripts/gate.sh:32-39`), the conductor runs as root, and many
builders compiling a TLS, HTTP/2 and zstd stack on 4 shared CPUs would queue for hours. A build box is
non-root, has 8 CPUs, and is thrown away. Code goes in as a `git bundle` with `sandbox copy`, so no token
ever enters a box. JUnit and the gate's last line come back out.

**Lanes.** Each lane is one workflow in this session (BUILD-PROMPT section 4), at most 2 agents at once.

| Lane | PRDs | Starts |
|---|---|---|
| Integrator | This PRD: phase 0, merges, `gate` runs in build boxes, staging deploys, build snapshot, the scribe | H+0, first |
| Lessons | PRD-17 | H+0, read only |
| Format and core | PRD-01, PRD-02 | H+0 |
| Masking | PRD-05 (a copy; needs only the S2 stub) | H+0 |
| Wire | PRD-03, PRD-04 | after the S1 and S3 stubs |
| Upload and server | PRD-07 at H+0; PRD-06 after S1 and S4 | H+0 |
| Judge | PRD-08, PRD-09 (from fixtures) | H+0 |
| Harnesses and pad | PRD-11, PRD-12 | H+0 |
| Participant | PRD-10 (`main.rs` stubs in phase 0) | H+0 |
| Test support and gate | PRD-14 (gate change first, `tests/common/`) | H+0 |
| Cut and docs | PRD-13 (read only, its copy ledger, until phase 3), PRD-15 | H+0 |
| Real runs | Watchers, from phase 1a | when the core slice records |

**Roles inside the lanes:**

| Role | Count | Does | Never |
|---|---|---|---|
| Conductor (this session) | 1 | Starts and watches the lane workflows; phases, timers, queues, caps, the call list. Deterministic control flow in each workflow script | Writes product code |
| Integrator | 1 | Phase 0, merges by section 4, `gate fast` on every merge and `gate full` at phase exits in a build box, staging deploys, the build snapshot | Merges a branch that fails section 4 |
| Scribe | 1, in the integrator lane | Merges the lane logs into `NIGHT-LOG.md`; writes `MORNING.md` and `ACCEPTANCE.md` | Copies a key, token or planted value into any file |
| Planner | one per PRD | Reads the PRD, the ledger, its seams, the code and the PRD's Amendments note; writes a short task list, each task inside the PRD's owned paths | Plans a task outside ownership |
| Builder | one or more per PRD | Builds tasks on `v1.0/prd-NN-<slug>` in its own worktree, runs its tests in a build box | Edits a path its PRD does not own |
| Reviewers A and B | two per merge request | A: correctness, the seven points, the ledger, masking before disk, no drop without a GAP. B: code quality and less code. Given the diff and the PRD, not the builder's summary | Rewrite the code; they return findings |
| Breaker | one per core merge request (PRD-01, 02, 03, 05, 06) | Tries to break it against run 1's shape (about 40 subagents, 1.5 MB bodies, rate-limit bursts, WebSockets) with PRD-02's `pipeline_guard.rs` and the latest pad smoke row, once per merge request | Runs a real 2-hour replay per diff |
| Replan | one, after each PRD merges | Reads review findings, test and real-run results; appends an "Amendments" note to the remaining PRDs it affects (PRD-00 ownership table) and a night-log `decision` line | Rewrites a section a builder is working from, or changes a locked ledger item (that goes to the call list, section 11) |
| Watcher | one per failing signature, at most 6 at once | PRD-12 section 13: read, name the cause, reproduce, fix on the owning branch, rerun | Changes a check to make it pass without record evidence |

Fallback: if the session cannot hold the lanes (workflows refused, or the conductor's own tokens or
memory run out), builders move to separate cloud sessions, one per lane, started with the session tool and
told their PRD, branch and the build-box rule. Whether such a session can push `v1.0/*` is unverified;
the integrator checks it with one branch push before using the fallback. The night log records which mode
ran.

### 2. Phase 0: foundations (H+0 to H+1.5)

PRD-00 "Build order, Phase 0" is the list; this section is how the integrator runs its own steps. Every
step is a commit on `v1.0/integration`, merge commits only, never a force push. The PRD set as revised
after the three reviews of 30 Oct is the input: no reconciliation step is left for H+0.

1. At H+0: the Vercel plan and session limit (Pro, or A1 cannot run), that a `v1.0/*` push works, and which
   keys are present (names only, never values). The start line of the night log.
2. Branch `v1.0/integration` from `main` at 4b226de2 and push it. Pushes go only to `v1.0/*` branches.
3. Merge `smart/intelligent-newton-0avrpi` (docs only). Check with `git diff --stat` that the merge touches
   nothing outside `docs/plans/`. Builders read the PRDs from the integration branch.
4. **PRD-14's gate change lands next**: `scripts/check-version.py` is red today on `docs/plans/`, so no
   merge could pass before it.
5. **Workspace edit (this PRD's files)** in one commit:
   - `[workspace.package] version = "1.0.0"` (`Cargo.toml:18`), and the comment at `:12-17` rewritten: it is
     still the one place a version is written; the sentence about the gate failing on other copies goes with
     `check-version.py` (PRD-14). Every binary built in the build says `1.0.0`, so the staging floor lets
     them in.
   - `members` gains `crates/goalcup-format`, `crates/goalcup`, `crates/goalcup-verify`; the five old
     crates move from `members` to `exclude`. They stay on disk for PRD-17 and the copy-ins and are never
     built, linted or tested again. No binary rename, no `--exclude` line in `gate.sh`.
   - Dev profile: opt-level 3 for `zstd-sys`, `sha2`, `brotli-decompressor` and the deflate backend
     (PRD-14 section 4); delete `[profile.dev.package.goalcup-daemon]` and its comment (`Cargo.toml:55-62`).
   - `rust-toolchain.toml` unchanged.
6. When PRD-01's format types and PRD-02's scaffold (with every dependency the PRDs named, PRD-00 phase 0
   step 5) are merged, regenerate `Cargo.lock` once with `cargo generate-lockfile` in a build box and commit
   it. After that a new dependency goes through PRD-02's `crates/goalcup/Cargo.toml` (or PRD-08's
   `crates/goalcup-verify/Cargo.toml`) with a night-log line, and the integrator refreshes the lock in the
   merge. No other lane touches the lock.
7. Build the **build snapshot** (toolchain, nextest, zstd, Node 22, Python 3, PostgreSQL, `npm ci` for
   `server/worker`, `cargo fetch` of the lock) before H+1, and rebuild it at each phase exit. PRD-12 builds
   the **pad image** with the three core harnesses in phase 0 (PRD-12 section 10).
8. Start the lane logs and `NIGHT-LOG.md` (section 12).

Exit gate (PRD-00): `cargo build` and `cargo test -p goalcup-format` green in a build box on integration.

### 3. Phase 1: three slices (H+1.5 to H+8)

PRD-00 "Phase 1" defines the core, second and third slices by file. This PRD adds how they run.

- Each planner starts with its PRD, PRD-00, PRD-01, the seams it uses and the PRD's builder notes.
- **Build-box rule.** No agent runs cargo on the conductor. A builder sends a bundle to its build box and
  runs `cargo nextest run -p <its crate> -E <its tests>`, then `gate fast`, before asking to merge.
- **Merge requests early.** A lane asks to merge whenever a slice of its PRD is green, not only at the end.
  Seams land first as compiling stubs by the "stable by" hours of PRD-00's seam table.
- **Seam change.** Only the seam's owner changes a seam, with a log line `seam S<n>: <change>, users
  <PRDs>`. Users rebase within the hour.
- **Vendor docs first.** Every builder of a harness-specific part reads the vendor's current docs and cites
  the page in the commit message (ledger T7).
- **PRD-17** publishes `LESSONS.md` by H+1.5 (PRD-17 builder notes); `tests/lessons_*.rs` follow as
  modules land.

**Phase 1a, the early real test.** As soon as the core slice records one request (PRD-00 core slice), the
real-runs lane runs PRD-12's P2: Claude Code and Codex, `direct`, `--prompt smoke`, `--server off`, in pad
boxes. Then one short real run after each later core feature lands (native capture, uploader, watchdog
restart, HTTP/2). If the concept fails here, no lane adds a new feature until it is fixed (BUILD-PROMPT
section 5).

**First staging deploy.** As soon as PRD-07's builder step 2 (heartbeat and floor) and PRD-06 are merged
(PRD-07 section 12 step 2), the integrator deploys staging: `wrangler deploy --env staging` from the
integration head, with `MIN_COMPETITION_VERSION = "1.0.0"` in both blocks of `wrangler.toml` (PRD-07
section 6). From then on, smoke runs use `--server staging` and check the server copy. Every staging deploy
writes a log line with the commit and the Worker version id. A 0.7 client on staging is refused from then
on; nobody runs one in this build, and it tests the refusal path. Before that deploy, runs use
`--server off` or `--server local`.

### 4. The merge rule

PRD-00 "Branches and the merge rule" is binding; made concrete here. The integrator merges a PRD branch
into `v1.0/integration` when all hold:

1. `git diff --name-only integration...branch` lists only paths that PRD owns (PRD-00 ownership map), or
   a seam change the seam owner approved in the log.
2. The PRD's own tests pass in a build box.
3. `gate fast` passes on the merged result in a build box as `ubuntu`, and the last line
   `gate fast: PASS in <N>s` goes into the log.
4. Reviewers A and B read the merge request's diff and found no blocker; for core parts the breaker too.
   Blockers are only: a path outside ownership, a hard-rule break (AGENTS.md as PRD-00 restates it), a byte
   written to disk before masking, a drop with no GAP, a cap of our own, a harness name in core code, a
   secret in a fixture, or a check made to pass without record evidence. Everything else is fixed if it
   makes the code smaller or clearer, and never holds the merge.

Merge commits only. A conflict is fixed by the owner of the conflicting file. The push of each merge runs
CI `check` (PRD-14 section 9), an independent record; a red CI on integration stops further merges until
it is green or the log names it as a runner fault. A watcher's fix is a merge request like any other.

### 5. Phase 2 and 3: integration and the cut (H+8 to H+10)

- **Phase 2 exit:** everything merged; `gate fast` green; PRD-17's deletion gate green on the new core;
  the pad end to end on staging for Claude Code `smoke`; PRD-12's `selftest` makes the pad trusted
  (PRD-12 P1); `gate full` timed (PRD-14 G1).
- **Phase 3, the cut:** PRD-14's gate and CI edits and PRD-15's docs have landed earlier, as soon as each
  was green. The phase 3 merge holds only PRD-13's four delete commits, PRD-14's delete of `check-links.py`
  after PRD-15's last run, and this PRD's workspace edit: drop the `exclude` entries and every workspace
  dependency only the old crates used (checked with `cargo tree`; expected `ed25519-dalek`, `redb`,
  `axum`), then refresh `Cargo.lock` (PRD-13 section 5). Exit: `gate full` green, PRD-13's reference guard
  clean, CI green, G1 and G4 in the log. G3 is read from the build's own gate and CI runs. G2 and G5 are
  after the merge to `main` (PRD-00 "Not in this build").
- **Cut-off H+10.** If PRD-17's deletion gate is not green by H+10, the cut does not happen. Old crates stay
  on disk, out of the workspace, and the end-of-build report says so. Phases 4 to 6 still run, so the owner
  sees real results.

### 6. Phase 4, 5 and 6: real runs (H+10 to about H+15)

All runs use PRD-12's pad on staging, with binaries built from the integration head in a build box. The
commit under test is in every row. A fix merged during a phase starts a rerun of the rows it can touch; the
release check (PRD-12 section 9, PRD-14) later demands that the tagged commit equals the pad's commit under
`crates/`, `server/worker/src/`, `connectors/` and `Cargo.lock`.

| Phase | Runs | Parallel | Est. time |
|---|---|---|---|
| 4. Core set | Claude Code, Codex, Hermes: `direct` `full`, `gateway` and `dodger` `smoke` (PRD-00 done item 1): 9 rows. The kill rows `--kill-recorder 8,10` on all three `direct` `full` (PRD-12 P10). The fault rows once each (PRD-12 P11) | 6 boxes; per key, section 9 | about 2 h with fix loops |
| 5. The rest | The other 12 harnesses, each case that applies; no key means "not verified: no key" with no run (Amp, Factory Droid, OpenRouter cases). Kilo's `gateway` row runs `full`: that is A4 | 6 boxes, from when phase 4's first wave is done and keys are free | about 2.5 h |
| 6. Acceptance | A1, A2, A3 (section 7) | Starts when the core set is green; shares the box limit with phase 5 | about 2.5 h, A1 is the longest |

Keys and rate limits: run 1 hit 2M tokens a minute on one OpenAI key with about 20 subagents (run report,
measure 9). So A1 holds the OpenAI key alone while it runs; a phase 5 run that would use the cheap OpenAI
model on the same key waits, or uses DeepSeek. The Claude key and the gateway key: at most 2 runs at once
each. DeepSeek: up to all 6 boxes at once, because DeepSeek limits concurrency per account at 500 or more
requests (link in Today), far above six runs with 20 subagents each; a 429 share above 5% in a row's
`provider_errors` drops it back to 2 for the rest of the build, with a log line.

**Watchers** follow PRD-12 section 13 exactly. A FAIL row gets one watcher, keyed by the signature
`<connector>|<case>|<variant>|<check id>|<first finding code>`. INCONCLUSIVE rows are rerun once. A fix
goes on the owning PRD's branch in a worktree; the integrator merges it under section 4; the watcher
reruns.

### 7. Final acceptance runs

Real runs on staging after the core set is green, each longer than a pad row, each with the pad's checks
(C1 to C6, PRD-12 section 8) plus the X lines below. The pad runs them with `--prompt-file`, `--minutes`,
`--keep-going` and `--kill-recorder` (PRD-12 section 1). Typing "keep going" when the agent sits idle is
what a participant may do (`AGENTS.md:83-84`); goalcup itself never does it.

| Id | Harness, case | Prompt | Length | Why |
|---|---|---|---|---|
| A2 | Claude Code, `direct`, Claude key | The pad's `full` prompt, with "use up to 20 subagents in parallel" in plain words | 60 minutes; one `kill -9` of the recorder at minute 30 | Hours-scale many-subagent load and a restart under load, before the 2-hour run (review A). Starts first |
| A1 | Codex, `direct`, OpenAI key alone, the same cheapest model and low reasoning setting as run 1, Codex at the pad's pinned version | **Run 1's prompt, word for word**, typed as one first message, if the owner put it in a file (PRD-00 question 16); else the pad's `full` prompt and the comparison is by load | 2 hours from the paste; "keep going" after 3 idle minutes | The same run as run 1, so every number compares. Starts when A2's first 30 minutes show no `fail` in `check --live` and no `memory_guard` gap, or when A2 ends |
| A3 | Hermes, `gateway` (Vercel AI Gateway, its built-in `ai-gateway` provider, PRD-12 section 2), chat TUI | The pad's `full` prompt asking for many subagents | 30 minutes | The owner's gateway question at length: one host list, same proxy, no gateway code. Runs beside A1 (a different key) |
| A4 | Kilo CLI, `gateway` (Vercel AI Gateway as an OpenAI-compatible provider) | The pad's `full` prompt | 20 minutes: Kilo's `gateway` `full` row of phase 5 | The owner's own example, "AI Gateway under Kilo". Not a release blocker; checked with C1 to C6 and N3 only |

Pass lines beyond C1 to C6 (A1 to A3):

| # | Check | Pass |
|---|---|---|
| X1 | Wire against the harness, by time | A1, A2: every 10-minute bucket of PRD-08's `timeline` has wire requests within PRD-08's tolerance (max(3 calls, 2%), 2-minute lag) of the harness's own calls; no bucket at 0 while the harness made calls. A3 (Hermes keeps per-session totals): the totals form after the seal (PRD-08 section 3.1, PRD-00 done item 1). A1 also sits next to run 1's table (run report, section 4) |
| X2 | Subagents | Every subagent file on disk has a native stream, and every thread has wire calls. A1: at least 20 subagents spawned (run 1: 42); A2: at least 15, with 10 or more running at once. Fewer spawned is INCONCLUSIVE for load, never PASS |
| X3 | No loss under normal load | Zero `memory_guard` gaps in A1, A2 and A3. One is a FAIL even though it is named: run 1's load is the normal load. Its guard name, limit and the peak `held_peak` (COUNT and `status.json`, PRD-01, PRD-02) go to PRD-02 |
| X4 | Restart under load | A2: exactly one `recorder_down` gap, start before end, under 5 s (PRD-02 defaults: 2 s is the Codex pad bound, 5 s is this run under A2's load); the harness's own log shows no failed call it did not retry in that window; `goalcup status` showed the off time; links verify across both boots |
| X5 | Upload keeps up | PRD-06 U2: at every minute the server is at most 6 minutes or 64 MiB raw behind the durable point; no wait over 120 s; after the seal, done within 2 minutes |
| X6 | Server copy | `fetch-run` from staging (read only, S13); the joined `rec.gcr.zst` equals the concatenated local parts (same SHA-256); `goalcup-verify check --server --local` has no `fail`; `fetch.json`'s `model_requests` equals the written count; the heartbeat rows say `failed` only while recording was off (`mode` null, or the entry ended while off, PRD-07 section 4), read in a `BEGIN READ ONLY` transaction |
| X7 | Secrets | `goalcup-verify secrets` with the known-values file: 0 hits in local parts, server copy, logs, the night log and the pushed repository |
| X8 | Usage | `goalcup-verify usage` on the server copy: tokens and model for every call, one row per subagent, `unattributed` 0 with PRD-09's meaning. A1: totals against run 1's (Codex's own record: 117.4M input, 410.6k output) as a ratio per hour |
| X9 | Size and memory | A1: raw and compressed bytes per hour, framing share under 2% (PRD-01 acceptance 4), against run 1's 405 MB uploaded in 49 recorded minutes; recorder RSS minus `held_bytes` under 260 MiB (PRD-06 U7) |
| X10 | Counts on screen | The end screen's number equals the verifier's recount and `fetch.json`. No "recording works" line with any gap |

A1, A2 and A3 inform the owner's merge decision (section 13); they are not PRD-00 done items.

### 8. Autonomy rules

Agents decide anything inside the seven points, AGENTS.md, the ledger and the PRDs (ledger B8). In
particular they may, without asking: merge into `v1.0/integration` by section 4; push `v1.0/*` branches;
deploy the Worker to staging; create sandboxes; create public, empty GitHub repositories for pad runs
(prefix `gc-pad-`); create `goalcup-usage-staging` (PRD-07, PRD-09); change a default marked "PRD-16" in
another PRD with a `decision` log line; append an Amendments note to a PRD (replan); mark a harness "not
verified" with a reason.

They stop that item and leave a log note (the build goes on) for:

| Stop | Examples |
|---|---|
| A locked ledger item or a hard rule | Setting a base URL from goalcup, writing into `~/.claude`, patching a harness, disabling pinning, any AGENTS.md rule beyond PRD-00's two changes |
| The website database | Any schema change, any write outside the staging Worker's own calls |
| Production | Any deploy, secret change or request to `ingest.agentcommunity.org` or bucket `goalcup-traces`; an approval bypass on production |
| `main` and tags | A merge to `main`, a tag, a release, a version PR |
| Spend past a cap | Section 9 |
| `VISION.md` and `OWNER-DECISIONS.md` | Never edited by an agent |

Commits and pushes are allowed only on `v1.0/*` branches, as part of this build (the owner's decision is
the request). Commit messages end with the session's attribution lines.

### 9. Spend caps

Live spend pricing in the pad is cut (PRD-00 "Not in this build"). Pad rows are bounded by their length
and checked by PRD-12's ledger after the run. The two runs longer than 20 minutes get a watcher check.

| Cap | Value | On reaching it |
|---|---|---|
| Per pad run | 10 USD `full`, 2 USD `smoke` (ledger T6, PRD-12 section 7) | Bounded by `--minutes`; the ledger after the run marks `over_cap`, and no further `full` run of that harness and case starts until the owner answers. Not a check failure |
| Per acceptance run (A1, A2, A3) | 10 USD each (ledger T6) | Every 10 minutes the watcher runs `goalcup-verify usage <entry> --json` on the open local entry with `sandbox exec` (an open entry reads to its last whole frame, PRD-09) and prices the token totals with the pad's flat rate (`scripts/pad/pad.toml`, which errs high). At 10 USD it runs `goalcup seal --yes` in the box. Checks run on what was recorded; if X2 is INCONCLUSIVE for that reason, the report asks the owner for a higher cap and a rerun |
| Whole build, model spend | 400 USD by the pad's ledger (PRD-00 question 18) | No new run starts. Running runs finish. A log line; the report shows it |
| Whole build, sandboxes | 150 USD estimated at Pro rates (`VERCEL-COMPAT.md:52`) | No new build box; builds queue on the boxes that exist |
| Boxes at once | 6 pad boxes; per key as section 6; build boxes one per active lane plus one for the integrator | A queue in the conductor |

Why 400: phase 1a and selftest about 35 USD; phase 4 with fault rows and one rerun per row up to 180;
phase 5 about 25 runs on DeepSeek, expected 1 to 2 USD each, at most 250 if every one hit its cap;
acceptance up to 30. The expected total is well under half of that. The ledger is an estimate; the
provider dashboards are the truth (PRD-12 section 7). Agent tokens of the build itself are not in these
caps; the workflow's token budget applies if the owner set one, and the report states tokens used.

### 10. Stop rules

| # | Rule | What happens |
|---|---|---|
| S1 | Three failed fixes on one signature (PRD-12 section 13) | That loop ends. The row stays FAIL, the tier is "not verified", the log holds the three attempts. For the core set this is a release blocker in the report |
| S2 | A phase exit gate missed by 2 hours | The scribe notes it. The conductor narrows: phase 5 harnesses first become "not verified: no time", then A4 is dropped. The core set and A1 to A3 are never dropped for time |
| S3 | Report time (`REPORT_AT`, default H+16) | No new run starts. `MORNING.md` is written from what is done; a run still going is listed as running, and the scribe updates the file when it ends |
| S4 | A build cap | No new run or box starts (section 9) |
| S5 | A stop of section 8 | That item waits for the owner; everything else goes on |
| S6 | A call of W1 or W2 (section 11) | Every pad box is sealed and removed; no new run starts until the owner answers |
| S7 | PRD-17's deletion gate not green by H+10 | No cut (section 5) |
| S8 | Build boxes fail to start three times in 10 minutes (a Vercel fault) | Builds wait and retry every 5 minutes. They never fall back to the conductor (ledger B1). After 30 minutes the owner is called (W6) |
| S9 | A red CI `check` on integration | No further merges until it is green or named as a runner fault |

### 11. What calls the owner

The owner is awake but away during the build. A push notification to the owner's phone, one per cause,
with a top line in `NIGHT-LOG.md`. Only these (BUILD-PROMPT section 7):

| # | Event | Why it cannot wait |
|---|---|---|
| W1 | A real secret (a provider key, the GitHub token, the approval key, a Cloudflare or database credential) found in any record, server copy, log, night log, pushed repository or commit | Only the owner can rotate it. All runs stop (S6) |
| W2 | Anything touched production: a request from a box or agent to the production Worker or bucket, a production deploy, a production secret change | Participants could be affected. All runs stop |
| W3 | The shared website database looks hurt: staging Worker database errors (5xx from the database, not 4xx from the Worker's own rules) more than 20 times in 10 minutes | The website shares it |
| W4 | Spend out of control: the ledger or a provider dashboard past 1.5 times a build cap, or a sandbox that will not stop | Money keeps leaving |
| W5 | A key the core set needs is missing or refused by its provider (401 or 403 on the first run) | The owner can fix it in minutes; otherwise the core set cannot pass |
| W6 | A locked ledger item cannot hold, a hard rule seems wrong (from a replan or a watcher), or Vercel Sandboxes are down for 30 minutes (S8) | Only the owner can change it |
| W7 | The core set still FAIL after the fix loops (S1 on a core row) | The release blocker is known; the owner may want to look before the report |

Nothing else calls the owner: failing checks inside a loop, late phases and missing keys for other
harnesses all wait for the report.

### 12. The night log

`docs/plans/2026-10-30-v1.0/NIGHT-LOG.md`, append only. One writer per file: each lane appends to its own
file `<night-dir>/lanes/<lane>.log` on the conductor's disk (all workflows share this session's
filesystem; `<night-dir>` is outside the repository and named in the start line). Every 30 minutes the
scribe merges new lines, sorted by time, into `NIGHT-LOG.md` and commits it on integration. A lane never
commits log lines on its PRD branch, so the log never conflicts with a merge. PRD-13's copy ledger lines
and PRD-17's gate lines go the same way.

One line per entry:

```
<HH:MM>Z | <lane> <role> <PRD or harness> | <kind> | <plain text, links to row.json, gate line, CI run, commit>
```

Kinds: `start`, `decision`, `seam`, `merge`, `gate`, `deploy`, `run`, `fix`, `stop`, `call`, `spend`,
`check changed`, `amendment`, `owner question`. No key, token, planted value, model name or model id. A line
that quotes an error copies only the error code and the plain words, never a request body.

### 13. The end-of-build report

`docs/plans/2026-10-30-v1.0/MORNING.md` (the name is kept), one screen first, details after.

1. **One line:** "Ready to merge `v1.0/integration` at `<commit>`" when PRD-00's definition of done items 1
   to 7 hold, else "Not ready: <the one main reason>".
2. **Acceptance line:** "A1 PASS, A2 PASS, A3 FAIL (<why>)". The owner weighs it; a FAIL here is printed
   next to line 1, never hidden under it.
3. **Core set:** the 9 rows (harness x case) and the 3 kill rows, each PASS or FAIL with its `why` line.
4. **Acceptance:** A1 to A4 with X1 to X10, and A1 next to run 1: wire calls against harness calls,
   subagents recorded, gaps and off time, raw and compressed bytes per hour, upload lag, recorder RSS,
   spend.
5. **The 15 harnesses:** the tier summary from `PAD-RESULTS.md` and `connectors/tiers.toml` (PRD-12
   section 9), with versions.
6. **The gateway answer:** the gateway rows (core set, A3, A4): same host list, same proxy, no gateway
   code, in two sentences with links. And the dodger answer: tunnel, named gap, session files, honest label.
7. **Blockers:** each with the owning PRD, the signature, the three attempts if S1 ended it.
8. **Decisions agents took** that the owner may reverse (`decision`, `amendment`, `check changed` lines).
9. **Owner questions** (PRD-00 list and `owner question` lines), each with the default the build used.
10. **Release checklist** (section 14) with the commit filled in.
11. **Spend and resources:** model spend by provider (estimate), sandbox cost, tokens used; every GitHub
    repository created (to delete), staging runs made, the staging Worker version; the codebase size
    before and after (`wc -l`, src and tests); a line that every sandbox was removed and every key file
    wiped.

`ACCEPTANCE.md` (same folder) holds the A1 to A4 prompts as run (run 1's prompt only if the owner agreed it
may be committed), each run's row, its X lines, and the commands to repeat it.

### 14. Release steps (the owner's, after the report)

1. Read `MORNING.md`. Answer the owner questions or accept the defaults.
2. Check that no real run is open on production (PRD-07 section 12 step 3, the read-only query).
3. Merge `v1.0/integration` into `main` (merge commit). If `main` moved since 4b226de2, the merge result
   differs from the verified commit: the core set `direct` reruns on it before the tag, or the release
   check refuses (PRD-14 G6).
4. `sh scripts/gate.sh full` on `main` in a build box as `ubuntu`.
5. `sh scripts/release.sh tag 1.0.0`. The version is already `1.0.0` on `main` from phase 0, so no
   `prepare` PR is needed (`scripts/release.sh:72-75`). The release check runs first (PRD-14 section 9,
   PRD-12 `--release-check`): the core rows and the Codex kill row PASS on the same commit with a trusted
   pad.
6. `release.yml` publishes the four targets (PRD-14 G5). Check `install.sh` from `releases/latest` on Linux
   and on the owner's Mac (PRD-14 G7), and run the pad once on the Mac (PRD-12 P9).
7. **Production Worker, from the owner's own device, right after step 6:** `wrangler deploy` from the
   tagged commit. `MIN_COMPETITION_VERSION = "1.0.0"` is already in `wrangler.toml` (PRD-07 section 6), so
   from this deploy every mode refuses a 0.7 client. The floor goes last, so nobody is refused while 1.0
   cannot be downloaded. Between steps 5 and 7 a 1.0 client cannot use the 0.7 Worker; keep that window to
   minutes. Then delete the `RECEIPT_SIGNING_KEY` secret (PRD-07 section 12 step 5).
8. Production smoke: one practice run with 1.0, sealed, fetched, checked (PRD-07 section 12 step 6).
   Optional: repeat A1 on production for a production proof.
9. After the merge, the deferred trust checks: PRD-14 G2, PRD-12 P8 (PRD-00 "Not in this build").
10. `EVENT_ID` switch between 6 Nov 00:00 and 7 Nov 00:00 UTC (`wrangler.toml:36-42`), the owner's deploy.
11. On 5 Nov, rerun the core set `direct` on the harness versions current that day (PRD-12 risk "a verified
    version is not the one people run"). A new failure there is a fix-forward release `1.0.1`.

No rollback to the 0.7 Worker once 1.0 clients run (PRD-07 section 12). A bad 1.0 Worker is held with
`MAINTENANCE` on and fixed forward.

## Files it owns

Change:

- `Cargo.toml` (workspace): version and its comment, members and `exclude`, dev profiles, workspace
  dependencies at the cut.
- `Cargo.lock`: generated after the scaffold and at the cut; refreshed by the integrator at merges that
  add a dependency.
- `rust-toolchain.toml`: unchanged unless a builder shows a need, with a log line.

Create:

- `docs/plans/2026-10-30-v1.0/NIGHT-LOG.md`
- `docs/plans/2026-10-30-v1.0/MORNING.md`
- `docs/plans/2026-10-30-v1.0/ACCEPTANCE.md`

No script in the repository. The workflow scripts live where the workflow tool saves them; the lane logs
live in `<night-dir>` on the conductor; the start line names both. Build-box and staging commands are run
by the integrator from this PRD.

Must not touch: every path another PRD owns, in particular `scripts/pad/**`, `PAD-RESULTS.md` and
`connectors/tiers.toml` (PRD-12), `server/worker/**` including `wrangler.toml` (PRD-07), `scripts/gate.sh`,
`scripts/release.sh` and `.github/**` (PRD-14), `connectors/**` (PRD-11), `crates/**` (PRD-01 to PRD-10,
PRD-14, PRD-17), `VISION.md` (the owner), `OWNER-DECISIONS.md` (the owner's ledger). A PRD file is its
author's; this PRD's replan role only appends Amendments notes (PRD-00).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-00 | Build order, slices, merge rule, done items, owner questions 16 to 19, defaults; "Where the build runs" | Binding here |
| PRD-01 | Golden test and worked example green is the phase 0 exit; acceptance 3 (kill) and 4 (size) in phases 4 and 6; `start` keys `build` and `keep`; `held_bytes`, `held_peak` | Used here |
| PRD-02 | Kill test R1 (2 s, Codex) and the 5 s bound under A2's load (its defaults table); status `mode`; `GOALCUP_KEEP_PARTS`; `held_peak` for X3 | Used here |
| PRD-06 | U2 and U7 lines in X5 and X9; `upload.json` `keep`; first staging deploy after its merge | Used here |
| PRD-07 | Staging deploy after its builder step 2 and PRD-06; `MIN_COMPETITION_VERSION = "1.0.0"` in both blocks from the first staging deploy; `fetch-run` and S13 `fetch.json`; `failed` only while recording is off; the read-only queries; deploy order (section 12) | Used here |
| PRD-08 | `check`, `check --server --local`, `check --live`, `secrets`, `timeline`; tolerances owned by "PRD-16" in its defaults: kept as PRD-08 states them; `conn_bytes` as PRD-08 rules | Used here |
| PRD-09 | `usage --json` on open and fetched entries, for X8 and the acceptance spend check; the per-thread table in the report | Used here |
| PRD-11 | Connector data and the one host list the gateway and dodger rows rely on | Used here |
| PRD-12 | The pad, `selftest`, rows, `PAD-RESULTS.md`, `connectors/tiers.toml`, `--release-check`, the watchers' loop, the ledger and flat rate; `--prompt-file`, `--minutes`, `--keep-going`, `--kill-recorder`; P10 and P11 scheduled in phase 4 | Used here |
| PRD-13 | The phase 3 merge: four delete commits plus this PRD's workspace edit; the H+10 cut-off | Done together |
| PRD-14 | `gate fast` and `gate full` in build boxes as `ubuntu`; build lock; CI on integration pushes; G1, G3, G4 in the build; G2, G5, G6, G7 at release; the release check in `release.sh tag`. Old crates are out of `members` from phase 0, so no exclude line | Used here |
| PRD-15 | Docs land as soon as green; README "The gate" numbers from G1 | Used here |
| PRD-17 | `LESSONS.md` by H+1.5; the deletion gate is the phase 2 exit and the precondition of the cut | Used here |

## What is cut

| Path | What | Lines |
|---|---|---:|
| `Cargo.toml:3-9` | The five old members (to `exclude` at phase 0, gone at the cut) | 5 |
| `Cargo.toml:55-62` | The daemon's dev profile and its comment | 8 |
| `Cargo.toml:64-132` | Workspace dependencies used only by the old crates, found with `cargo tree` at the cut (expected `ed25519-dalek`, `redb`, `axum`, PRD-13 section 5) | unverified until the cut |
| `docs/plans/2026-10-30-cut-first-plan.md:364-475` | The day-by-day schedule, the Monday 2-hour run and the Wednesday release-candidate run | Superseded by this PRD; the file stays as history (PRD-00 "Not in this build") |

No tests go here; the tests of the old crates go with PRD-13.

## Acceptance

### Real-run proofs (first)

| # | Proof | Pass |
|---|---|---|
| N1 | **The core set is green on one commit.** Phase 4 | 9 PASS rows (3 harnesses x 3 cases: `direct` `full`, `gateway` and `dodger` at least `smoke`) from a trusted pad on the same integration commit, every check (PRD-00 done item 1). Hermes passes C1 by the totals form. Dodger rows: verifier exit 1 from `listed_hosts_decrypted: named`, `wire_keeps_up` `named` (or `skip` "no per-call times" for Hermes), the end screen says "not recorded" (PRD-12 section 2). The Codex kill row PASS (done item 2) |
| N2 | **The long runs pass.** Phase 6 | A1, A2 and A3 pass C1 to C6 and X1 to X10; A1's table sits next to run 1's in `ACCEPTANCE.md` |
| N3 | **One system for gateways.** The gateway rows of N1, A3 and A4 | `ai-gateway.vercel.sh` is `decrypt`, `listed: shared`, `kind: gateway` in every `conn`; no connector or core line names a gateway; wire matches the harness within tolerance |
| N4 | **Every merge is accounted for.** At report time | Each merge commit on integration has a `merge` line with its `gate fast: PASS` line, two reviewer lines (three for core) and a green CI run |
| N5 | **The release check accepts the verified commit and refuses another.** PRD-14 G6 by a watcher on a scratch commit | Accepts the commit of N1; refuses a commit that differs under `crates/` |
| N6 | **Nothing leaked.** After phase 6 | `goalcup-verify secrets` with the build's known values: 0 hits over every kept record, server copy, `pad-out/`, `NIGHT-LOG.md`, the lane logs, `MORNING.md`, `ACCEPTANCE.md`, every `gc-pad-*` repository and the integration branch history |
| N7 | **Nothing touched production.** After phase 6 | The staging Worker was the only deploy (Cloudflare deployments list); no row with a production event id was created by the build (read-only query); no request to the production host in any box's `goalcup.log` |
| N8 | **The report is complete.** At `REPORT_AT` | Every item of section 13 is filled, every claim links to a row, a gate line, a CI run or a commit; a reader who was away can decide in 10 minutes |

### Automated tests that guard them

None new. This PRD writes no code. Its guards are other PRDs' tests: PRD-12 `selftest` and `test_pad.py`
(the pad cannot pass a known fault), PRD-14 `test_gate.py` and `test_release.py` (the gate and the release
check), PRD-08's check tests, and CI on every integration push.

## Risks and blind spots

| Risk | What we do |
|---|---|
| 2 agents per workflow make the build crawl | Twelve lanes as parallel workflows; the three slices let the core slice prove the concept first; S2 narrows phase 5 before the core set is touched. If the session cannot hold the lanes, the cloud-session fallback (section 1) |
| Many lanes on one repository step on each other | One owner per path, worktrees, seams as stubs first, every dependency in the scaffold, merges only through the integrator, lane logs off git |
| A seam changes late and three PRDs break | Seam changes only by the owner with a log line; users rebase within the hour |
| A replan edits a PRD a builder is reading | Replan only appends an Amendments note; planners read it before each task |
| Vercel Sandbox limits: concurrent boxes, session length, egress | Concurrency for 6 pad boxes plus up to 13 build boxes is unverified; the conductor queues. A1 needs a box of about 2.5 hours: Pro allows 24 hours, Hobby 45 minutes (`VERCEL-COMPAT.md:41`), checked at H+0. Never turn on credential brokering for a model host (`VERCEL-COMPAT.md:46`) |
| Staging is shared by other people's tests | The 1.0 floor refuses 0.7 clients on staging from the first deploy; said in the report. Runs are told apart by run and entry id |
| The staging Worker writes into the shared website database | Only through its own calls, with today's tables (PRD-07). W3 calls the owner if the database looks hurt |
| A provider outage looks like 15 failures | PRD-12's `provider_down` and `provider_refused` come first; W5 only for a core key |
| A watcher "fixes" a check instead of the code | PRD-12 section 13 rule; `check changed` lines; `selftest` again before any row is trusted |
| A green pad today and a different harness version on 7 Nov | Rows name versions; connectors turn auto-update off; the 5 Nov rerun (section 14 step 11) |
| Run 1's prompt is not available | A1 runs the pad's `full` prompt for 2 hours and the report says the comparison is by load, not by prompt (PRD-00 question 16) |
| A2's 10 USD cap ends it before 15 subagents run | X2 INCONCLUSIVE, not PASS; the report asks for a higher cap and a rerun |
| The acceptance spend check reads the open entry and misses the last frame | It errs low by at most one frame; the flat rate errs high; the after-run ledger is the record |
| DeepSeek at 6 boxes at once gets 429s anyway | The 5% rule in section 6 drops it to 2; `provider_errors` is in every row |
| The kill in A2 coincides with a burst and costs more than the gap shows | X4 checks the harness's own log for failed calls in that window; PRD-02 owns the watchdog |
| Sleep, network change and hours of many subagents are untested | PRD-12 P11 runs `freeze` and `route-drop` on Codex in phase 4; A2 runs 60 minutes of up to 20 subagents before A1 |
| Linux only | The owner's Mac run (section 14 step 6); macOS-only paths run in PRD-14's release workflow on each target |
| The night log grows a secret by quoting an error | Section 12 rule; N6 scans it and the lane logs |
| The report is too long to read | Section 13 puts the decision on the first line and acceptance second |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Lanes | Twelve workflows in this session; cloud sessions only as fallback | Owner |
| `REPORT_AT` | H+16 | Owner |
| Cut-off for the cut | H+10 | Owner |
| Build model spend cap | 400 USD estimate | Owner |
| Build sandbox cap | 150 USD estimate | Owner |
| Per acceptance run cap | 10 USD (ledger T6) | Owner |
| A1, A2, A3, A4 length | 2 h, 60 min, 30 min, 20 min | Owner |
| Kill in A2 | one, at minute 30 | PRD-02 |
| Idle before "keep going" | 3 minutes | Owner |
| Runs at once per key | 2 for Claude, OpenAI and gateway; A1 holds OpenAI alone; up to 6 for DeepSeek until 429s pass 5% | PRD-16 |
| Phase-miss narrowing | after 2 hours late | Owner |
| Call list | W1 to W7 | Owner |
| Night log merge and commit | every 30 minutes | Integrator |
| Test bucket | the existing `goalcup-traces-staging` | PRD-07 |

## Builder notes

Order of work for the integrator and the scribe:

1. H+0: the checks of phase 0 step 1. Write the start line.
2. Phase 0 steps 2 to 8. Publish the build snapshot before H+1.
3. Start the lanes of section 1 at once; lanes whose PRDs wait on seams start with planning against the
   stubs.
4. Merge on request by section 4, seams first. First staging deploy as soon as PRD-07 step 2 and PRD-06
   are in.
5. Phase 1a smoke runs; then one short run per new core feature.
6. Phase 2 exit, phase 3 merge, phases 4 to 6, the report.

Traps:

- The gate refuses root (`scripts/gate.sh:32-39`). Never set `GOALCUP_GATE_ALLOW_ROOT=1` to pass a merge:
  root hides permission bugs. Run it in a build box as `ubuntu`. Never fall back to the conductor.
- `scripts/check-version.py` is red on today's `docs/plans/`: land PRD-14's change before any other merge.
- Two packages building a `goalcup` binary would make `target/*/goalcup` ambiguous: phase 0 moves the old
  crates out of `members`, so this never happens.
- `scripts/release.sh tag` tags `origin/main` only and checks its version (`:54-61`); it never tags a
  branch. Nothing is tagged during the build.
- The production floor is a file value (`wrangler.toml:51-55`); a dashboard edit is wiped by the next
  deploy.
- Never `--persistent` sandboxes: a stop keeps the disk with keys for 30 days (PRD-12 section 7).
- Codex: Enter after a delay; `$CODEX_HOME/.env` overrides proxy variables, so the gateway key file holds
  only the key line (PRD-12 builder notes).
- Hermes: stop any gateway for the same home first; run the chat TUI, because one-shot mode caps delegation
  at 2 children (PRD-12 builder notes).
- One OpenAI key with 20 subagents hit 2M tokens a minute in run 1: A1 holds its key alone.
- Never check a pruned folder: `GOALCUP_KEEP_PARTS=1` on every run (PRD-12).
- A lane never commits to `NIGHT-LOG.md` itself; it appends to its own lane file.

How a watcher proves this PRD in a real run: at report time, open `MORNING.md`, follow the first line's
commit to its 9 core rows and the Codex kill row in `PAD-RESULTS.md` and the A1 to A3 rows in
`ACCEPTANCE.md`; run `goalcup-verify check --server <fetched A1> --local <A1 entry>` and compare the two
SHA-256 sums; then run `scripts/pad/table --release-check <commit>` and see exit 0.

## Asks (after PRD-00 revision 2)

PRD-00 revision 2 ruled on all eleven asks of revision 1 (8 is owner question 16, 9 is owner question 17),
and PRD-01 accepted both (`build` and `keep` in the `start` event). Left:

1. **PRD-00:** the phase hours of "Phases 2 to 6" follow this PRD: phase 2 and 3 H+8 to H+10, real runs
   H+10 to about H+15, the report at H+16, the cut-off at H+10. Defaults "Keys per harness" gains the
   DeepSeek exception (up to 6 at once, section 6).
2. **PRD-13:** the cut-off is H+10, not H+9 (section 5 "the time PRD-16 sets").
3. **PRD-14:** drop "the fallback of PRD-16 S8 (builds one at a time on the conductor as a non-root user)"
   from the build-lock paragraph; S8 now waits for Vercel and never builds on the conductor (ledger B1).
4. **PRD-12:** defaults row "Parallelism, 2 per provider key" gains "up to 6 for DeepSeek" (section 6).
   No new flag: the acceptance spend check uses `sandbox exec` and the existing flat rate.

## Review answers

Answers to the three reviews of 30 Oct (CC = consistency check, A = review A, B = review B), for items
that concern this PRD or where it is the natural owner.

| Item | Answer |
|---|---|
| CC 1 Asks not absorbed | Accepted where it touches this PRD: PRD-00 and PRD-01 ruled on this PRD's asks; the remaining four are in "Asks" |
| CC 2 Night cap | Accepted. 400 USD stays this PRD's number, now PRD-00 question 18 |
| CC 13, B C17 Old crates during phases 0 to 3 | Accepted. This PRD's `exclude` plan is the one; PRD-13 and PRD-14 dropped theirs |
| CC 14, B O1 Phase 0 order | Accepted. Docs merge, then PRD-14's gate change, then the workspace edit (section 2) |
| CC 15, CC 104, B C19 18-CPU conductor | Accepted. Removed. Lanes are parallel workflows in this session at 2 agents each; cloud sessions only as fallback (section 1) |
| CC 16 Phase 6 and the 2-hour run | Accepted. A1 runs on staging before the hand-off; it informs the merge, it is not a done item; the owner may repeat it on production |
| CC 17, B A11 Dodger exit code | Accepted as PRD-12 rules: exit 1 from `listed_hosts_decrypted: named`; N1 lists the observable pass lines |
| CC 19 Server access | Accepted. Read-only credentials on the conductor only (section 1) |
| CC 22, B N14 Staging floor from the first deploy | CC 22 accepted. N14 rejected, as PRD-00 ruled: no 0.7 client runs on staging in this build, and it tests the refusal path |
| CC 23 New owner questions | Accepted. Run 1's prompt and "2 swp" are PRD-00 questions 16 and 17 |
| CC 37 `build` and `keep` in `start` | Accepted by PRD-01; ask removed |
| CC 38 `queue_bytes` | Accepted. X3 reads `held_peak` |
| CC 50, B C24 Gap length 2 s against 5 s | Partly. Two runs, two bounds, both in PRD-02's defaults table: 2 s on the Codex pad kill row, 5 s in A2 under load. X4 cites it |
| CC 53, B W3 Runs with no key | Accepted. Phase 5 says "not verified: no key", no run |
| CC 91 Rename asks | Accepted. Closed by the `exclude` plan |
| CC 93, B C16 Root and the build host | Accepted. Gate only in build boxes as `ubuntu`; root refusal kept; S8 no longer builds on the conductor (ask 3 of PRD-14) |
| CC 96 Flags | Accepted. PRD-12 section 1 has all four; this PRD uses them in section 7 |
| CC 98 Tier file | Accepted. `connectors/tiers.toml`, PRD-12's; interfaces and report read it |
| CC 99 Test bucket | Accepted. `goalcup-traces-staging`; agents may create `goalcup-usage-staging` only |
| CC 102 `VERCEL-COMPAT.md` | Accepted. History docs are kept in this build, so the citations hold |
| CC 105 X6 `failed` rule | Accepted. "`failed` only while recording was off (`mode` null, or the entry ended while off)", PRD-07 section 4 |
| CC 106, B O8 "Night" framing | Accepted. The build starts in the morning; `MORNING_AT` is now `REPORT_AT`; "wakes the owner" is now "calls the owner". File names `NIGHT-LOG.md` and `MORNING.md` are kept so other PRDs' references hold |
| CC 107 PRD-13 at H+0 | Accepted. Lanes table: PRD-13 starts read only at H+0 |
| A verdict 5, A coverage gaps (sleep, network change, hours of many subagents) | Accepted. PRD-12 P11's `freeze` and `route-drop` are scheduled in phase 4; A2 (60 minutes, up to 20 subagents) now starts before A1, and A1 waits for A2's first 30 clean minutes. Partly: A2 stays 60 minutes, not 2 hours, for the 10 USD cap |
| A 4d Kill case per core harness | Accepted. P10's three kill rows in phase 4; the Codex row is in N1 and the release check |
| A other findings | Not this PRD's; PRD-00 routed them to PRD-02 to PRD-11 |
| B verdict 2 Too big for one day | Partly. The slices, the deferred list (PRD-00) and narrower real-run phases take it in. Phase hours moved to H+8, H+10 and H+16, honest about the size; S2 still narrows phase 5 first |
| B C19, B7 Reviewers | Accepted. Two reviewers per merge request, the breaker once per core merge request, not per commit (sections 1 and 4) |
| B A3 Owner questions in three places | Accepted for this PRD. Its open questions moved to PRD-00's list; the report reads that list |
| B A4 Phase 1a slice | Accepted. Phase 1a starts on PRD-00's core slice, `--server off` |
| B A10, M10 Replan edits PRDs mid-build | Partly, as PRD-00 ruled: Amendments notes appended to a PRD, never a rewrite of a section in use; no separate `AMENDMENTS.md` |
| B R1 Hermes cannot pass C1 | Accepted. N1 and X1 use the totals form for Hermes (PRD-00 question 19) |
| B B6 Real-run size | Accepted. Gateway and dodger `smoke` in phase 4; A3 30 minutes; A4 is Kilo's phase 5 `gateway` `full` row. Partly: A2 stays 60 minutes (review A asks for longer load before A1) |
| B M1 Lock and dependencies | Accepted. The lock is generated once after the scaffold and refreshed only by the integrator |
| B M5 One big phase 3 merge | Accepted. Phase 3 holds only the deletes and this PRD's workspace edit |
| B M6 One writer for the night log | Accepted. Lane log files on the conductor, merged by the scribe every 30 minutes (section 12) |
| B O2 Step 0 reconciliation | Accepted. Done by this review round; phase 0 says the revised set is the input |
| B O6 Pad image in phase 0 | Accepted. Three core harnesses (section 2 step 7) |
| B O7 First staging deploy | Accepted. After PRD-07 step 2 and PRD-06; phase 1a uses `--server off` |
| B T4 G2, G3, G5 | Accepted. G2 and G5 after the merge (section 14); G3 read from the build's own runs; G1 and G4 at phase 3 |
| B T5 P5 and P8 | Accepted. P5 is phase 4's normal waves; P8 after the merge |
| B N4 Live pricing | Partly. Cut from the pad. Kept as a watcher check for the three acceptance runs only, which are longer than the time cap that bounds a pad row and must hold the locked 10 USD cap (section 9) |
| B W9 Run 1's prompt | Accepted. The load comparison is the default when no prompt file exists |
| B section 4 rate limits (implied by B6 timing) | New: DeepSeek limits concurrency per account at 500 or more requests (https://api-docs.deepseek.com/quick_start/rate_limit), so phase 5 may use up to 6 boxes on that key; a 429 share above 5% drops it to 2 |
