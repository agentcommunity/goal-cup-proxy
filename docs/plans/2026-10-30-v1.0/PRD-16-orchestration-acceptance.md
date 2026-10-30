# PRD-16: Build orchestration and acceptance

Id: PRD-16. Version: goalcup 1.0. Status: plan for the one-day build, revision 3 (PRD-00 rulings R3-8 to R3-11 and the
alignment pass applied). Date: 30 October 2026. Base: main at 4b226de2. Index: [PRD-00](PRD-00-index.md). Ledger:
[OWNER-DECISIONS](OWNER-DECISIONS.md). Format: [PRD-01](PRD-01-record-format.md).

## Goal

Run one build day in which many agents build goalcup 1.0 from PRD-01 to PRD-17, prove it on real harnesses
in Vercel Sandboxes, and leave the owner one page at the end that says "ready to merge" or "not, because".
Nothing reaches `main`, a tag or production without the owner.

## Points served

All seven, as the gate that decides whether the build serves them. 1 and 2: the proxy record is the
proof (ledger T4): no unexplained gap, the history check, every subagent recorded; the harness's own
local log is an optional extra check where it keeps one. 3: the kill and fault runs, `goalcup recover`
after a watchdog kill, and the load runs. 4: secret scans of every record, server copy, log and pushed repository. 5: server copy
checks on staging. 6: counts the participant saw against the verifier's recount. 7: 15 harnesses run the
way people run them, gateways included.

## Owner decisions it carries out

- One big build, many parallel agents, shipped as **goalcup 1.0** (ledger L1). The build **starts in the
  morning** after the owner wakes and the keys are in place (ledger B7). "Night" in file names
  (`NIGHT-LOG.md`) is kept only so other PRDs' references hold.
- The relayed answers of 30 Oct: "1 core OK" and "version 1 sure" confirm the core set (Claude Code,
  Codex, Hermes) and the version. "2 swp" is read as "swap the harnesses in the list", which the 29 Oct
  night list carries out. "3 i can fund one" is read as the one funded Google key. Both readings are
  unverified; PRD-00 owner question 8 (old 17) holds them.
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
- **Staging only**: every run uploads to the staging Worker, approved with the staging test approval key and the owner's member id;
  agents may deploy staging; production, the merge to `main` and tags are the owner's; no website
  database schema migration (ledger B4 to B6, S3).
- Spend: 10 USD per run (ledger T6), checked live by the pad every 5 minutes; no whole-build cap, since
  cost is otherwise not a constraint (section 9). Pad runs are about 20 minutes of real work with many subagents
  (ledger T5). Keys in 0600 files, separate keys per harness where possible, read-only database and R2
  credentials on the conductor only (PRD-00 defaults).
- The long run uses run 1's Codex prompt, so the two runs compare; without it, a load comparison
  (PRD-00 owner question 6, old 16).
- **The proxy record is the proof** (ledger T4). Every check here reads the record, `status.json` and
  the staging server copy. A comparison with the harness's own local log is an optional extra, as
  PRD-12 section 8 runs it: `skip` where the harness keeps none, never a block to a PASS then. **No
  outside API is called to verify**: no provider usage or billing lookup in any check.
- A crashed recorder restarts at once; only a crash loop goes to pass-through, retried never more than
  5 minutes apart (ledger R3a). When the watchdog itself dies, `goalcup recover` restarts only the
  recorder on the same port with the same run certificate, and the harness never restarts (ledger R3b).
  A2 proves both under load (section 7, X4).

## Today

| What | Where | Fact |
|---|---|---|
| Workspace members and version | `Cargo.toml:3-9`, `:18` | Five old crates; version `0.7.0`. The comment at `:13-17` says it is the one place a version is written and "the gate fails on any other hard-coded copy" |
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
| **Conductor** | This cloud session, 4 CPUs, root. The lane workflows, their agents, git worktrees, `scripts/pad/pad` (conductor half: sandboxes, approval, test repos, fetch, copy-in, read-only queries), `fetch-run`. `fetch-run` runs here because only the conductor holds the read-only credentials. It never compiles, tests or runs `goalcup` or `goalcup-verify`: it copies the fetched folder into the run's pad box (or a build box) with `sandbox copy`, and every check runs there (PRD-12 section 8). Nothing from the build under test executes on the conductor (ledger B1) | The approval key and the owner's member id, read-only database and R2 credentials, the `goalcup-usage-staging` write token (PRD-09), `GH_TOKEN` (creates test repos; never enters a box), the Cloudflare token for staging, the known-values files, all key files and push token files under `~/.pad-keys/` (0600) until copied into a box. Never a production write credential |
| **Build boxes** | Vercel Sandboxes, 8 vCPU and 16 GB, user `ubuntu`, from the build snapshot | The Rust toolchain from `rust-toolchain.toml`, nextest, zstd, Node 22, Python 3, a warm cargo cache. No key, no token |
| **Pad boxes** | Vercel Sandboxes from PRD-12's pad image, 4 vCPU, `--non-persistent` | One harness; its key file, for `gateway` the gateway key file, and the run's push token (fine-grained, throwaway pad owner, Contents read and write; never `GH_TOKEN`), each 0600, for the life of the run; `goalcup` and `goalcup-verify` from the build under test; after the seal, the fetched copy and the per-run known-values file (only values that entered this box, deleted after the scan) for the checks |
| **Staging Worker** | `goalcup-ingest-staging` on Cloudflare, 1.0 code from the integration branch | Its own bucket; the shared database through the Worker's own calls only |

Why build boxes: the gate refuses root (`scripts/gate.sh:32-39`), the conductor runs as root, and many
builders compiling a TLS, HTTP/2 and zstd stack on 4 shared CPUs would queue for hours. A build box is
non-root, has 8 CPUs, and is thrown away. Code goes in as a `git bundle` with `sandbox copy`, so no token
ever enters a box. JUnit and the gate's last line come back out.

**Lanes.** Each lane is one workflow in this session, at most 2 agents at once, and all lanes run in
parallel (ledger B2). The real-runs lane may run as up to three workflows of 2 agents each, so six
watchers can work at once. This table replaces BUILD-PROMPT section 4's older table (PRD-00 R3-10);
ledger B1, B2 and B2a win over both. A phase 0 seam diff (PRD-01's types, the PRD-02 scaffold, the S1 to
S4 stubs) gets reviewers A and B and no breaker, so the Format and core lane can ship PRD-01's types by
H+1 and the PRD-02 scaffold by H+1.5.

| Lane | PRDs | Starts |
|---|---|---|
| Integrator | This PRD: phase 0, merges, `gate` runs in build boxes, staging deploys, build snapshot, the scribe | H+0, first |
| Lessons | PRD-17 (`LESSONS.md` by H+3, PRD-00 R3-8) | H+0, read only |
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
| Builder | one or more per PRD | Builds tasks on `v1.0/prd-NN-<slug>-<task>` from the PRD branch, in its own worktree, merged into `v1.0/prd-NN-<slug>` by the lane before its merge request;, runs its tests in a build box | Edits a path its PRD does not own |
| Reviewers A and B | at least two per merge request | A: correctness, the seven points, the ledger (a break of one of the seven points or of a locked ledger item is a blocker), masking before disk, no drop without a GAP. B: code quality and less code, and the PRD's row in PRD-00 "Size budget". Given the diff and the PRD, not the builder's summary. Each is a fresh agent, not the builder, and writes its findings before it sees the other reviewer's | Rewrite the code; they return findings |
| Breaker | one per core merge request (PRD-01, 02, 03, 05, 06) | Tries to break it against run 1's shape (about 40 and then about 200 synthetic subagents, 1.5 MB bodies in bursts, a held writer, WebSockets) with PRD-02's `pipeline_guard.rs`, `pipeline_stall.rs` and `by_hand_load`, and the latest pad smoke row, once per merge request. There is no run 1 replay tool | Runs a real 2-hour replay per diff |
| Replan | one per merge of a PRD branch (each slice), the last step of that PRD's lane pipeline, run in the integrator lane | Reads review findings, test and real-run results; appends an "Amendments" note to each remaining PRD it affects and to this PRD, which with PRD-00 "Build order" is the build plan of ledger B2a (lanes, order, phases, runs); posts `wc -l` of the merged PRD against its PRD-00 "Size budget" row; one docs-only commit on integration with an `amendment` night-log line. Reviewer A of that lane reads the note against the ledger. The conductor re-reads this PRD's Amendments before each phase and each new lane task; planners read their PRD's before each task | Rewrites a section a builder is working from, edits BUILD-PROMPT.md, OWNER-DECISIONS.md or VISION.md, or changes a locked ledger item (that calls the owner, W6) |
| Watcher | one per named cause, at most 6 at once, run as up to three real-runs workflows of 2 agents each, started by the conductor | PRD-12 section 13: read, name the cause, reproduce, fix on the owning branch, rerun | Changes a check to make it pass without record evidence |

Fallback: if the session cannot hold the lanes (workflows refused, or the conductor's own tokens or
memory run out), builders move to separate cloud sessions, one per lane, started with the session tool and
told their PRD, branch and the build-box rule. Whether such a session can push `v1.0/*` is unverified;
the integrator checks it with one branch push before using the fallback. The night log records which mode
ran.

### 2. Phase 0: foundations (H+0 to H+1.5)

PRD-00 "Build order, Phase 0" is the list; this section is how the integrator runs its own steps. Every
step is a commit on `v1.0/integration`, merge commits only, never a force push. The PRD set as revised
after the three reviews of 30 Oct and the alignment pass is the input: no reconciliation step is left for
H+0.

Every step that changes a file outside `docs/plans/` (PRD-14's gate change, the workspace edit, the
`members` lines, the phase 3 workspace edit) goes through section 4 like any merge request: on its own
`v1.0/prd-14-gate` or `v1.0/prd-16-<slug>` branch, reviewers A and B, `gate fast` in a build box (ledger
B2). A `Cargo.lock` refresh whose only cause is a reviewed `Cargo.toml` change needs no reviewer. Only the
docs merge of step 3, the scribe's `NIGHT-LOG.md`, `MORNING.md` and `ACCEPTANCE.md` commits and the
replan's Amendments commits are direct docs-only commits; N6 scans them.

1. At H+0: the Vercel plan and session limit (Pro, or A1 cannot run), that a `v1.0/*` push works, and which
   keys are present (names only, never values; the staging approval key and member id included). The
   start line of the night log.
2. Branch `v1.0/integration` from `main` at 4b226de2 and push it. Pushes go only to `v1.0/*` branches.
3. Merge `smart/intelligent-newton-0avrpi` (docs only). Check with `git diff --stat` that the merge touches
   nothing outside `docs/plans/`. Builders read the PRDs from the integration branch. This merge is docs
   only and is the one merge made before `gate fast` can pass.
4. **PRD-14's gate change lands next**: `scripts/check-version.py` is red today on `docs/plans/`, so no
   merge could pass before it. It is PRD-14's lane's merge request, reviewed by A and B; its `gate fast`
   runs on the result of the first workspace merge (step 5), and the two merge back to back.
5. **Workspace edit (this PRD's files)**, on `v1.0/prd-16-workspace`, merged together with PRD-01's first
   merge:
   - `[workspace.package] version = "1.0.0"` (`Cargo.toml:18`), and the comment at `:13-17` rewritten: it is
     still the one place a version is written; the sentence about the gate failing on other copies goes with
     `check-version.py` (PRD-14). Every binary built in the build says `1.0.0`, so the staging floor lets
     them in.
   - `members` gains each new crate in the merge that creates it (`crates/goalcup-format` with PRD-01's
     first merge, `crates/goalcup` with PRD-02's scaffold, `crates/goalcup-verify` with PRD-08's first
     stub); the integrator applies that one `Cargo.toml` line inside the merge. The five old crates move
     to `exclude` in the same merge as the first new member, so the workspace is never empty and never
     names a missing crate. They stay on disk for PRD-17 and the copy-ins and are never built, linted or
     tested again. No binary rename, no `--exclude` line in `gate.sh`.
   - Dev profile: opt-level 3 for `zstd-sys`, `sha2`, `brotli-decompressor` and the deflate backend
     (PRD-14 section 4); delete `[profile.dev.package.goalcup-daemon]` and its comment (`Cargo.toml:55-62`).
   - `rust-toolchain.toml` unchanged.
6. In the merge of PRD-02's scaffold (with every dependency the PRDs named, PRD-00 phase 0 step 5), the
   integrator runs `cargo generate-lockfile` and `cargo fetch --locked` in the build box before `gate fast`
   and commits the lock in that merge. After that a new dependency goes through PRD-02's
   `crates/goalcup/Cargo.toml` (or PRD-01's `crates/goalcup-format/Cargo.toml`, or PRD-08's
   `crates/goalcup-verify/Cargo.toml`) with a night-log line, and the integrator refreshes the lock in the
   merge. No other lane touches the lock; builders never commit `Cargo.lock`.
7. Build the **build snapshot** (toolchain, nextest, zstd, Node 22, Python 3, `npm ci` for
   `server/worker`, `cargo fetch` of the lock) before H+1. Rebuild it at once after step 6's lock
   regeneration and at each phase exit. After any other merge that changes `Cargo.lock`, build boxes run
   `cargo fetch --locked` before the gate. PRD-12 builds the **pad image** with the three core harnesses
   in phase 0 (PRD-12 section 10).
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
  <PRDs>`. Users merge integration into their branch within the hour (merge commits, no rebase of a
  pushed branch).
- **Vendor docs first.** Every builder of a harness-specific part reads the vendor's current docs and cites
  the page in the commit message (ledger T7).
- **PRD-17** publishes `LESSONS.md` by H+3 (PRD-00 R3-8, PRD-17 builder notes); `tests/lessons_*.rs` follow as
  modules land.

**Phase 1a, the early real test.** As soon as the core slice records one request (PRD-00 core slice), the
real-runs lane runs PRD-12's P2: Claude Code and Codex, `direct`, `--prompt smoke`, `--server off`, in pad
boxes. Precondition: PRD-05's core-slice masker (option b, the left guard, `boundary()`) is merged on the
commit under test, checked in the `start` event, whose `mask_rules` names the shared kinds and not only
`fail_closed` (the S2 stub). No real key enters a pad box before that (ledger M1, point 4). Then one short
real run after each later core feature lands (each core feature of PRD-00's second and third slices:
native capture, uploader, watchdog restart and null mode, `goalcup recover`, refusal learning, the gateway and dodger cases,
Hermes `state.db`, HTTP/2, and exact-value masking only if the owner said yes, PRD-00 owner question 3).
If the concept fails here, no lane adds a new feature until it is fixed (BUILD-PROMPT section 5).

**First staging deploy.** As soon as PRD-07's builder step 2 (heartbeat and floor) and PRD-06 are merged
(PRD-07 section 12 step 2), the integrator deploys staging: `wrangler deploy --env staging` from the
integration head, with `MIN_COMPETITION_VERSION = "1.0.0"` in both blocks of `wrangler.toml` (PRD-07
section 6). From then on, smoke runs use `--server staging` and check the server copy. Every staging deploy
writes a log line with the commit and the Worker version id. A 0.7 client on staging is refused from then
on; nobody runs one in this build, and it tests the refusal path. Before that deploy, runs use
`--server off` or `--server local`: they upload nowhere, so B4 holds. The integrator deploys staging the
hour PRD-07 step 2 and PRD-06 merge; no run ever targets production.

### 4. The merge rule

PRD-00 "Branches and the merge rule" is binding; made concrete here. The integrator merges a PRD branch
into `v1.0/integration` when all hold:

1. `git diff --name-only integration...branch` lists only paths that PRD owns (PRD-00 ownership map), or
   a seam change the seam owner approved in the log.
2. The PRD's own tests pass in a build box.
3. `gate fast` passes on the merged result in a build box as `ubuntu`, and the last line
   `gate fast: PASS in <N>s` goes into the log.
4. At least two independent reviewers (A and B) read the merge request's diff, and for core parts the
   breaker too. Every finding is fixed in the merge request before the merge (ledger B2). A finding may be
   declined only with a one-line reason in the merge request that the reviewer who raised it accepts;
   reviewer B's less-code findings are declined when the change would not make the code smaller or
   clearer. Never declined (blockers): a break of one of the seven points or of a locked ledger item (for
   example backpressure on the wire, a recorder that stops for good, a count taken before its bytes are
   written, `failed` not sent while recording is off); a hard-rule break (AGENTS.md as ledger section 7
   and PRD-00 restate it); a path outside ownership; a byte written to disk before masking; a drop with no
   GAP; a cap of our own; a harness name in core code; a secret in a fixture; a wrong count; a failing
   acceptance case of the PRD; a check made to pass without record evidence.

Merge commits only. A conflict is fixed by the owner of the conflicting file. The push of each merge runs
CI `check` (PRD-14 section 9), an independent record on GitHub's runners; it is not a build host for
agents, and no agent builds there (ledger B1); a red CI on integration stops further merges until
it is green or the log names it as a runner fault. A watcher's fix is a merge request like any other.

### 5. Phase 2 and 3: integration and the cut (H+8 to H+10)

- **Phase 2 exit:** everything merged; `gate fast` green; PRD-17's deletion gate green on the new core;
  the pad end to end on staging for Claude Code `smoke`; the Codex kill row (PRD-02 R1, PRD-00 phase 2)
  once, `direct` with `--kill-recorder 8,10` on `full`, because both kill minutes fall after a 3-minute
  `smoke` run ends; PRD-12's `selftest` makes the pad trusted (PRD-12 P1); `gate full` timed (PRD-14 G1).
- **Phase 3, the cut:** PRD-14's gate and CI edits and PRD-15's docs have landed earlier, as soon as each
  was green. The phase 3 merge holds only PRD-13's four delete commits, PRD-14's delete of `check-links.py`
  after PRD-15's last run, and this PRD's workspace edit: drop the `exclude` entries and every workspace
  dependency only the old crates used (checked with `cargo tree`; expected `ed25519-dalek`, `redb`,
  `axum`), then refresh `Cargo.lock` (PRD-13 section 5). Exit: `gate full` green, PRD-13's reference guard
  clean, CI green, G1 and G4 in the log. G3 is read from the build's own gate and CI runs. G2 and G5 are
  after the merge to `main` (PRD-00 "Not in this build").
- **Cut target H+10.** If PRD-17's deletion gate is not green by H+10, phases 4 to 6 start without the cut
  and the owner is called under W6: locked L3 cannot hold on time. Old crates stay on disk, out of the
  workspace, and nothing builds them. The lessons lane keeps working and reruns the gate after each fix
  merges. The cut lands in the same build as soon as the gate is green, before `REPORT_AT`; the nine core
  rows and the Codex kill row then rerun on the cut commit, because the release check compares `crates/`
  and `Cargo.lock` (PRD-12 section 9). If the gate is still not green at `REPORT_AT`, line 1 of
  `MORNING.md` reads "Not ready: old crates not deleted (ledger L3), deletion gate <state>".

### 6. Phase 4, 5 and 6: real runs (H+10 to about H+15)

All runs use PRD-12's pad on staging, with binaries built from the integration head in a build box. The
commit under test is in every row. A fix merged during a phase starts a rerun of the rows it can touch; the
release check (PRD-12 section 9, PRD-14) later demands that the tagged commit equals the pad's commit under
`crates/`, `server/worker/src/`, `connectors/` (except `connectors/tiers.toml`), `Cargo.lock`, `Cargo.toml`
and `rust-toolchain.toml`.

| Phase | Runs | Parallel | Est. time |
|---|---|---|---|
| 4. Core set | Claude Code, Codex, Hermes: `direct`, `gateway` and `dodger`, all `full`, the 20-minute payments prompt (ledger T5, PRD-00 done item 1): 9 rows. The kill rows `--kill-recorder 8,10` on all three `direct` `full` (PRD-12 P10). The fault rows once each (PRD-12 P11) | 6 boxes; per key, section 9 | about 2 h with fix loops |
| 5. The rest | The other 12 harnesses, each case that applies; no key means "not verified: no key" with no run (Amp, Factory Droid, OpenRouter cases). All rows run `full`. Kilo's `gateway` row is A4. As each harness's first `full` sample lands, a PRD-09 builder may write its log reader from that sample (PRD-09) for the optional local log check. A harness with no reader is judged from the record (ledger T4); a missing reader is never a "not verified" reason | 6 boxes, from when phase 4's first wave is done and keys are free | about 2.5 h |
| 6. Acceptance | A1, A2, A3 (section 7) | Starts when the core set is green; shares the box limit with phase 5 | about 2.5 h, A1 is the longest |

Keys and rate limits: run 1 hit 2M tokens a minute on one OpenAI key with about 20 subagents (run report,
measure 9). So A1 holds the OpenAI key alone while it runs; a phase 5 run that would use the cheap OpenAI
model on the same key waits, or uses DeepSeek. The Claude key and the gateway key: at most 2 runs at once
each. DeepSeek: up to all 6 boxes at once, because DeepSeek limits concurrency per account at 500 or more
requests (link in Today), far above six runs with 20 subagents each; a 429 share above 5% in a row's
`provider_errors` drops it back to 2 for the rest of the build, with a log line.

After the last phase 5 merge that touches any path the release check compares, the nine core rows and the
Codex kill row rerun on that head before `REPORT_AT`.

**Watchers** follow PRD-12 section 13 exactly. A FAIL row gets one watcher per named cause (PRD-12
section 13 step 2); the signature `<connector>|<case>|<variant>|<check id>|<first finding code>` only
groups rows. INCONCLUSIVE rows are rerun once. A fix
goes on the owning PRD's branch in a worktree; the integrator merges it under section 4; the watcher
reruns.

**Newcomer run** (PRD-15 acceptance 1): one practice run in a pad box on staging, after the third slice
lands the guide, by a watcher given no PRD, only `README.md`, `docs/PARTICIPANT-GUIDE.md` and
`GOALCUP_SERVER`; the conductor approves it; about 15 minutes, a short prompt. Its result sits in
`MORNING.md` beside done item 6.

### 7. Final acceptance runs

Real runs on staging after the core set is green, each longer than a pad row, each with the pad's checks
(C1 to C6, PRD-12 section 8) plus the X lines below. The pad runs them with `--prompt-file`, `--minutes`,
`--keep-going` and `--kill-recorder` (PRD-12 section 1). Typing "keep going" when the agent sits idle is
what a participant may do (`AGENTS.md:83-84`); goalcup itself never does it.

| Id | Harness, case | Prompt | Length | Why |
|---|---|---|---|---|
| A2 | Claude Code, `direct`, Claude key | The pad's `full` prompt, with "use up to 20 subagents in parallel" in plain words | 60 minutes; one `kill -9` of the recorder at minute 30 (`--kill-recorder 30`); the fault `kill-watchdog-recover@45` (PRD-12 section 3.1): the watchdog killed at minute 45, the recorder at minute 46, `goalcup recover` at minute 47 | Hours-scale many-subagent load, an eager restart under load (ledger R3a) and a recover after the watchdog died (ledger R3b), before the 2-hour run (review A). Starts first |
| A1 | Codex, `direct`, OpenAI key alone, the same cheapest model and low reasoning setting as run 1, Codex at the pad's pinned version | **Run 1's prompt, word for word**, typed as one first message, if the owner put it in a file (PRD-00 owner question 6, old 16); else the pad's `full` prompt and the comparison is by load | 2 hours from the paste; "keep going" after 3 idle minutes | The same run as run 1, so every number compares. Starts when A2's first 30 minutes show no `fail` in `check --live` and no `memory_guard` gap, or when A2 ends |
| A3 | Hermes, `gateway` (Vercel AI Gateway, its built-in `ai-gateway` provider, PRD-12 section 2), chat TUI | The pad's `full` prompt asking for many subagents | 30 minutes | The owner's gateway question at length: one host list, same proxy, no gateway code. Runs beside A1 (a different key) |
| A4 | Kilo CLI, `gateway` (Vercel AI Gateway as an OpenAI-compatible provider) | The pad's `full` prompt | 20 minutes: Kilo's `gateway` `full` row of phase 5 | The owner's own example, "AI Gateway under Kilo". Not a release blocker; checked with C1 to C6 and N3 only |

Pass lines beyond C1 to C6 (A1 to A3):

| # | Check | Pass |
|---|---|---|
| X1 | No missed call, by time | From the record (ledger T4): no unexplained gap, and the history check `pass` or `skip` (PRD-12 section 8), with no orphan outside a named GAP in any 10-minute bucket of PRD-08's `timeline`. Optional extra, where the harness keeps a local log (PRD-12's local log check): PRD-08 `wire_keeps_up` and `threads_match` pass or are `named`; A1, A2: each bucket within PRD-08's tolerance (max(3 calls, 2%), 2-minute lag) of the harness's own calls; A3 (Hermes): per call when its `state.db` rows carry usable times, else the totals form (PRD-08 section 3.1, PRD-00 owner question 7). With no local log it is `skip` and X1 rests on the record. A1 also sits next to run 1's table (run report, section 4) |
| X2 | Subagents | From the record (PRD-12 C2): every thread in usage `threads` (PRD-09's thread hints, the history check's chains, native streams where read) has wire calls or a native stream. Optional extra, with a local log: every subagent file on disk has a native stream; A3 (Hermes): every subagent session in `state.db` has a native stream, matched by PRD-08's form for Hermes. A1: at least 20 subagent threads in the record (run 1: 42); A2: at least 15, with 10 or more running at once. Fewer spawned is INCONCLUSIVE for load, never PASS |
| X3 | No loss under normal load | Zero `memory_guard` gaps in A1, A2 and A3. One is a FAIL even though it is named: run 1's load is the normal load. Its guard name, limit and the peak `held_peak` (COUNT and `status.json`, PRD-01, PRD-02) go to PRD-02. The 200-stream `by_hand_load` result (PRD-02 R8) is reported next to A1 and A2; its `memory_guard` gaps are a night-log line, not a FAIL |
| X4 | Restart and recover under load | A2, from the record and `status.jsonl`: exactly two `recorder_down` gaps, each start before end. The first, at minute 30, under 5 s (PRD-02 defaults: 2 s is the Codex pad bound, 5 s is this run under A2's load), and no `null_mode` gap in the run (ledger R3a: two kills 15 minutes apart are no crash loop). The second runs from the recorder kill at minute 46 to the recovered boot's first record, with cause `watchdog_lost`, and passes PRD-12's `kill-watchdog-recover` lines: the same harness pid and the same `port` before and after (the harness never restarts), every later `conn` to a listed host `decrypt` (same run certificate), written model requests after the recover above 0 (ledger R3b). The history check finds no orphan outside the two gaps; `goalcup status` showed both off times; links verify across all three boots. If Claude Code lost its terminal with the watchdog, X4 is INCONCLUSIVE as PRD-12 rules. Optional extra: the harness's own log shows no failed call it did not retry outside the gaps |
| X5 | Upload keeps up | PRD-06 U2: at every minute t of the watcher's log, `entries.upload_offset` is at or past the `durable_raw` the log shows at t minus 6 minutes, except while `upload.json` shows a network or server error; no wait over 120 s; after the seal, done within 2 minutes |
| X6 | Server copy | `fetch-run` from staging on the conductor (read only, S13); then in the run's pad box, on the fetched copy: the joined `rec.gcr.zst` equals the concatenated local parts (same SHA-256); `goalcup-verify check --server --local` has no `fail`; `fetch.json`'s `model_requests` equals the written count; the heartbeat rows of `fetch.json` `heartbeats` (PRD-07 section 7; no hand-written database query) say `failed` only while recording was off (PRD-06 section 6: `mode` null, or the writer idle over 60 s while records wait) and `gaps` from the next heartbeat after a good boot |
| X7 | Secrets | `goalcup-verify secrets` in the run's pad box, on the fetched copy, with the per-run known-values file, which holds the run certificate key (`known-box`, ledger R3b): 0 hits in local parts, server copy, logs and the pushed repository. The night log is scanned by N6 |
| X8 | Usage | `goalcup-verify usage` in the run's box, on the fetched server copy: every `ok` call has a model; tokens are present or the call is counted in `tokens.calls_without_usage` with its status (refused calls have no usage by nature); one row per subagent; `unattributed` 0 with PRD-09's meaning. Usage comes from the record only; no provider usage lookup (ledger T4). A1, informative only: totals against run 1's report (Codex's own record: 117.4M input, 410.6k output) as a ratio per hour |
| X9 | Size and memory | A1: raw and compressed bytes per hour, framing share under 2% (PRD-01 acceptance 4), against run 1's 405 MB uploaded in 49 recorded minutes; recorder RSS minus `held_bytes` under 260 MiB (PRD-06 U7a's line) and no step of about 140 MiB at the 60-minute rotation (PRD-06 U7b) |
| X10 | Counts on screen | The end screen's number equals the verifier's recount and `fetch.json`. No "recording works" line with any gap |

A1, A2 and A3 inform the owner's merge decision (section 13); they are not PRD-00 done items.

### 8. Autonomy rules

Agents decide anything inside the seven points, AGENTS.md, the ledger and the PRDs (ledger B8). In
particular they may, without asking: merge into `v1.0/integration` by section 4; push `v1.0/*` branches;
deploy the Worker to staging; create sandboxes; create empty public GitHub repositories for pad runs
(prefix `gc-pad-`) with `GH_TOKEN` on the conductor, where only the run's push token (fine-grained,
throwaway pad owner, Contents read and write) enters a pad box and `GH_TOKEN` never does; create `goalcup-usage-staging` (PRD-07, PRD-09); change a default marked "PRD-16" in
another PRD with a `decision` log line; append an Amendments note to a PRD (replan); mark a harness "not
verified" with a reason.

They stop that item and leave a log note (the build goes on) for:

| Stop | Examples |
|---|---|
| A locked ledger item or a hard rule | Setting a base URL from goalcup, writing into `~/.claude`, patching a harness, disabling pinning, any AGENTS.md rule beyond the changes ledger section 7 lists |
| The website database | Any schema change, any write outside the staging Worker's own calls |
| Production | Any deploy, secret change or request to `ingest.agentcommunity.org` or bucket `goalcup-traces`; an approval bypass on production |
| `main` and tags | A merge to `main`, a tag, a release, a version PR |
| Spend past a cap | Section 9 |
| `VISION.md` and `OWNER-DECISIONS.md` | Never edited by an agent |

Commits and pushes are allowed only on `v1.0/*` branches, as part of this build (the owner's decision is
the request). Commit messages end with the session's attribution lines.

### 9. Spend caps

Live pricing inside goalcup is cut (PRD-00 "Not in this build"); the pad holds the locked 10 USD per-run
cap (ledger T6). Cost is otherwise not a constraint, so there is no whole-build cap; the build reports its
spend instead.

| Cap or line | Value | On reaching it |
|---|---|---|
| Per run: every pad row, A1, A2, A3 | 10 USD (ledger T6). A `smoke` run is expected near 2 USD: an estimate, not a cap | Every 5 minutes during the run, `pad` runs `goalcup-verify usage <open entry> --json` in the box with `sandbox exec` (an open entry reads to its last whole frame, PRD-09), prices the totals with PRD-12's per-model rate lines (which err high), and at 10 USD runs `goalcup seal --yes` in the box (PRD-12 section 7). The row gets `over_cap: true` and a night-log line; the checks run on what was recorded. Not a check failure. If X2 is INCONCLUSIVE for that reason in A1 to A3, the report asks the owner for a higher cap and a rerun |
| Build model spend | No cap (ledger T6) | The scribe writes a `spend` line at the top of `NIGHT-LOG.md` at 200 and 400 USD by the pad's ledger estimate; the report shows it; runs go on |
| Build sandboxes | No cap | A `spend` line at 75 and 150 USD, estimated at Pro rates (`VERCEL-COMPAT.md:52`); builds go on |
| Boxes at once | 6 pad boxes; per key as section 6; build boxes one per active lane plus one for the integrator | A queue in the conductor |

Expected model spend is about 200 USD, at most about 300: phase 1a and selftest about 35; phase 4 up to
180 with fault rows and reruns; phase 5 about 25 runs on DeepSeek at 1 to 2 USD each; acceptance up to 30.
The ledger is an estimate from the record; no check reads a provider usage or billing page (ledger T4). The owner's own dashboards stay the owner's check of real spend (PRD-12 section 7). Agent tokens of the
build itself are not in these lines; the workflow's token budget applies if the owner set one, and the
report states tokens used.

### 10. Stop rules

| # | Rule | What happens |
|---|---|---|
| S1 | Three failed fixes on one cause (the cause the watcher names in PRD-12 section 13 step 2; the signature only groups rows) | That loop ends. The row stays FAIL, the tier is "not verified", the log holds the three attempts. For the core set this is a release blocker in the report |
| S2 | A phase exit gate missed by 2 hours | The scribe notes it. The conductor reorders, never drops: phase 5 runs on, harnesses with a key first; every harness and case gets its run before the report calls it "not verified". A4 runs as Kilo's phase 5 row. The core set and A1 to A3 keep their place |
| S3 | Report time (`REPORT_AT`, default H+16) | The scribe writes `MORNING.md` from what is done; a run still going is listed as running. Runs go on until every harness and case has had its run and A1 to A3 have run (ledger B3); the scribe updates `MORNING.md` as each ends and sends one notice when the last ends |
| S4 | (removed) A build cap | Nothing: there is no build cap (ledger T6); section 9's `spend` lines only report. The id is kept so references hold |
| S5 | A stop of section 8 | That item waits for the owner; everything else goes on |
| S6 | A call of W1 or W2 (section 11) | Every pad box is sealed and removed; no new run starts until the owner answers |
| S7 | PRD-17's deletion gate not green by H+10 | No cut yet (section 5). Calls the owner (W6): locked L3 cannot hold on time. The lessons lane keeps working; the cut lands in the same build as soon as the gate is green, before `REPORT_AT` |
| S8 | Build boxes fail to start three times in 10 minutes (a Vercel fault) | Builds wait and retry every 5 minutes. They never fall back to the conductor (ledger B1). After 30 minutes the owner is called (W6) |
| S9 | A red CI `check` on integration | No further merges until it is green or named as a runner fault |

### 11. What calls the owner

The owner is awake but away during the build. A push notification to the owner's phone, one per cause,
with a top line in `NIGHT-LOG.md`. Only these; BUILD-PROMPT section 7 lists the same seven:

| # | Event | Why it cannot wait |
|---|---|---|
| W1 | A real secret (a provider key, the GitHub token or a push token, the approval key, a Cloudflare or database credential) found in any record, server copy, log, night log, pushed repository or commit | Only the owner can rotate it. All runs stop (S6) |
| W2 | Anything touched production: a request from a box or agent to the production Worker or bucket, a production deploy, a production secret change | Participants could be affected. All runs stop |
| W3 | The shared website database looks hurt: staging Worker database errors (5xx from the database, not 4xx from the Worker's own rules) more than 20 times in 10 minutes | The website shares it |
| W4 | Spend out of control: a run past 1.5 times its 10 USD cap by the pad's ledger, or a sandbox that will not stop. Agents read no provider dashboard (ledger T4) | Money keeps leaving |
| W5 | A key the core set needs is missing or refused by its provider (401 or 403 on the first run) | The owner can fix it in minutes; otherwise the core set cannot pass |
| W6 | A locked ledger item cannot hold (including PRD-17's deletion gate not green by H+10, S7), a hard rule seems wrong (from a replan or a watcher), or Vercel Sandboxes are down for 30 minutes (S8) | Only the owner can change it |
| W7 | The core set still FAIL after the fix loops (S1 on a core row) | The release blocker is known; the owner may want to look before the report |

Nothing else calls the owner: failing checks inside a loop, late phases and missing keys for other
harnesses all wait for the report. The one exception is S3's notice that the report is final.

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

`docs/plans/2026-10-30-v1.0/MORNING.md` (the name is kept). The scribe fills PRD-00 "Morning report
template"; that template is the one list of what the report holds and in which order. This section adds
only these rules:

- **Line 1** says "Ready to merge `v1.0/integration` at `<commit>`" only when PRD-00's definition of done
  items 1 to 9 all hold, else "Not ready: <the one main reason>".
- **The acceptance line** (A1 to A4) is printed next to line 1, never hidden under it. The owner weighs it.
- Every claim links to a row, a gate line, a CI run or a commit.
- **Owner notes**, under "Owner questions" with the default the build used, if not answered before H+0:
  the reading of ledger M1 (the Codex rate-limit rules are the four plan, quota and credit rows,
  `connectors/codex-cli/harness.toml:110-113`; the three id rows stay; PRD-05 section 8); and that with
  1.0 the unchanged cron sends the `recording` email when a heartbeat says `failed`, and the idle notices
  count written requests. No email code changed; the owner confirms this is wanted before the production
  deploy (PRD-07 section 12 step 3).
- The newcomer run (section 6) sits beside done item 6.

`ACCEPTANCE.md` (same folder) holds the A1 to A4 prompts as run (run 1's prompt only if the owner agreed it
may be committed), each run's row, its X lines, and the commands to repeat it.

### 14. Release steps (the owner's, after the report)

1. Read `MORNING.md`. Answer the owner questions or accept the defaults.
2. Check that no real run is open on production (PRD-07 section 12 step 3, the read-only query).
3. Merge `v1.0/integration` into `main` (merge commit). If `main` moved since 4b226de2, the merge result
   differs from the verified commit: the nine core rows and the Codex kill row rerun on it before the tag,
   or the release check refuses (PRD-14 G6).
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
author's; this PRD's replan role only appends Amendments notes, to any remaining PRD, PRD-00 and this PRD
included (PRD-00 ownership table).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-00 | Build order, slices, merge rule, done items, owner questions 6 to 8 (old 16 to 19), defaults, "Size budget", "Morning report template"; "Where the build runs" | Binding here |
| PRD-01 | Golden test and worked example green is the phase 0 exit; acceptance 3 (kill) and 4 (size) in phases 4 and 6; `start` keys `build` and `keep`; `held_bytes`, `held_peak` | Used here |
| PRD-02 | Kill test R1 (2 s, Codex) and the 5 s bound under A2's load (its defaults table); status `mode`; `GOALCUP_KEEP_PARTS`; `held_peak` for X3 | Used here |
| PRD-06 | U2, U7a and U7b lines in X5 and X9; `upload.json` `keep`; first staging deploy after its merge | Used here |
| PRD-07 | Staging deploy after its builder step 2 and PRD-06; `MIN_COMPETITION_VERSION = "1.0.0"` in both blocks from the first staging deploy; `fetch-run` and S13 `fetch.json` with its `heartbeats` list (X6); `failed` only while recording is off; the read-only queries; deploy order (section 12) | Used here |
| PRD-08 | `check`, `check --server --local`, `check --live`, `secrets`, `timeline`; tolerances owned by "PRD-16" in its defaults: kept as PRD-08 states them; `conn_bytes` as PRD-08 rules | Used here |
| PRD-09 | `usage --json` on open and fetched entries, for X8 and the pad's per-run spend check; a log reader per harness written from its phase 5 sample; the per-thread table in the report | Used here |
| PRD-11 | Connector data and the one host list the gateway and dodger rows rely on | Used here |
| PRD-12 | The pad, `selftest`, rows, `PAD-RESULTS.md`, `connectors/tiers.toml`, `--release-check`, the watchers' loop, the ledger, the per-model rate lines and the 5-minute spend check; `--prompt-file`, `--minutes`, `--keep-going`, `--kill-recorder`; P10 and P11 scheduled in phase 4 | Used here |
| PRD-13 | The phase 3 merge: four delete commits plus this PRD's workspace edit; the H+10 cut target and the W6 call if the gate is late | Done together |
| PRD-14 | `gate fast` and `gate full` in build boxes as `ubuntu`; build lock; CI on integration pushes; G1, G3, G4 in the build; G2, G5, G6, G7 at release; the release check in `release.sh tag`. Old crates are out of `members` from phase 0, so no exclude line | Used here |
| PRD-15 | Docs land as soon as green; README "The gate" numbers from G1; the newcomer run (acceptance 1, section 6) | Used here |
| PRD-17 | `LESSONS.md` by H+3 (PRD-00 R3-8); the deletion gate is the phase 2 exit and the precondition of the cut | Used here |

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
| N1 | **The core set is green on one commit.** Phase 4 | 9 PASS rows (3 harnesses x 3 cases: `direct`, `gateway` and `dodger`, all `full`, PRD-12 section 9) from a trusted pad on the same integration commit, every check (PRD-00 done item 1). Hermes passes C1 by the totals form. Dodger rows: verifier exit 1 from `listed_hosts_decrypted: named`, `wire_keeps_up` `named` (or `skip` "no per-call times" for Hermes), the end screen says "not recorded" (PRD-12 section 2). The Codex kill row PASS (done item 2) |
| N2 | **The long runs pass.** Phase 6 | A1, A2 and A3 pass C1 to C6 and X1 to X10; A1's table sits next to run 1's in `ACCEPTANCE.md` |
| N3 | **One system for gateways.** The gateway rows of N1, A3 and A4 | `ai-gateway.vercel.sh` is `decrypt`, `listed: shared`, `kind: gateway` in every `conn`; no connector or core line names a gateway; C1 passes from the record (no unexplained gap, the history check). Optional extra where a local log is read: wire matches the harness within tolerance |
| N4 | **Every merge is accounted for.** At report time | Each commit on integration that changes a path outside `docs/plans/` is a merge commit with a `merge` line, its `gate fast: PASS` line, at least two reviewer lines (three for core) and a green CI run; a lock refresh caused only by a reviewed `Cargo.toml` change says so |
| N5 | **The release check accepts the verified commit and refuses another.** PRD-14 G6, by a watcher | `scripts/pad/table --release-check <N1 commit>` exits 0; against a scratch commit that changes one line under `crates/` it exits 1. Nothing is tagged during the build; `release.sh tag` runs only at the owner's release |
| N6 | **Nothing leaked.** After phase 6 | `goalcup-verify secrets` with the build's known values, run in one check box from the build snapshot, `--non-persistent`, holding the full known-values file for the scan only and removed right after (a `decision` line: the one step where all keys meet a box): 0 hits over every kept record, server copy, `pad-out/`, `NIGHT-LOG.md`, the lane logs, `MORNING.md`, `ACCEPTANCE.md`, every `gc-pad-*` repository and the integration branch history |
| N7 | **Nothing touched production.** After phase 6 | The staging Worker was the only deploy (Cloudflare deployments list, read with an account-level read token, or the owner checks the production list); no row with a production event id was created by the build (read-only query); no request to the production host in any box's `goalcup.log` |
| N8 | **The report is complete.** At `REPORT_AT` | Every item of section 13 is filled, every claim links to a row, a gate line, a CI run or a commit; a reader who was away can decide in 10 minutes |

### Automated tests that guard them

None new. This PRD writes no code. Its guards are other PRDs' tests: PRD-12 `selftest` and `test_pad.py`
(the pad cannot pass a known fault), PRD-14 `test_gate.py` and `test_release.py` (the gate and the release
check), PRD-08's check tests, and CI on every integration push.

## Risks and blind spots

| Risk | What we do |
|---|---|
| 2 agents per workflow make the build crawl | Twelve lanes as parallel workflows; the three slices let the core slice prove the concept first; S2 reorders phase 5 and never drops a run for time. If the session cannot hold the lanes, the cloud-session fallback (section 1) |
| Many lanes on one repository step on each other | One owner per path, worktrees, seams as stubs first, every dependency in the scaffold, merges only through the integrator, lane logs off git |
| A seam changes late and three PRDs break | Seam changes only by the owner with a log line; users merge integration into their branch within the hour |
| A replan edits a PRD a builder is reading | Replan only appends an Amendments note; planners read it before each task |
| Vercel Sandbox limits: concurrent boxes, session length, egress | Concurrency for 6 pad boxes plus up to 13 build boxes is unverified; the conductor queues. A1 needs a box of about 2.5 hours: Pro allows 24 hours, Hobby 45 minutes (`VERCEL-COMPAT.md:41`), checked at H+0. Never turn on credential brokering for a model host (`VERCEL-COMPAT.md:46`) |
| Staging is shared by other people's tests | The 1.0 floor refuses 0.7 clients on staging from the first deploy; said in the report. Runs are told apart by run and entry id |
| The staging Worker writes into the shared website database | Only through its own calls, with today's tables (PRD-07). W3 calls the owner if the database looks hurt |
| A provider outage looks like 15 failures | PRD-12's `provider_down` and `provider_refused` come first; W5 only for a core key |
| A watcher "fixes" a check instead of the code | PRD-12 section 13 rule; `check changed` lines; `selftest` again before any row is trusted |
| A green pad today and a different harness version on 7 Nov | Rows name versions; connectors turn auto-update off; the 5 Nov rerun (section 14 step 11) |
| Run 1's prompt is not available | A1 runs the pad's `full` prompt for 2 hours and the report says the comparison is by load, not by prompt (PRD-00 owner question 6, old 16) |
| A2's 10 USD cap ends it before 15 subagents run | X2 INCONCLUSIVE, not PASS; the report asks for a higher cap and a rerun |
| The per-run spend check reads the open entry and misses the last frame | It errs low by at most one frame; the rate lines err high; between checks a run can overshoot by up to 5 minutes of spend (PRD-12 risk row); the after-run ledger is the record |
| DeepSeek at 6 boxes at once gets 429s anyway | The 5% rule in section 6 drops it to 2; `provider_errors` is in every row |
| The kill in A2 coincides with a burst and costs more than the gap shows | X4 runs the history check around both gaps; the harness's own log is an optional extra; PRD-02 owns the watchdog |
| Claude Code loses its terminal when the watchdog dies in A2 | X4 is INCONCLUSIVE as PRD-12's `kill-watchdog-recover` row rules, and the watcher follows PRD-10 acceptance 11; the minute-30 restart lines still count |
| The run certificate key sits on disk during the run (ledger R3b) | Accepted by the owner. X7 and N6 scan for it; PRD-02 removes it at the seal |
| Sleep, network change and hours of many subagents are untested | PRD-12 P11 runs `freeze` and `route-drop` on Codex in phase 4; A2 runs 60 minutes of up to 20 subagents before A1 |
| Linux only | The owner's Mac run (section 14 step 6); macOS-only paths run in PRD-14's release workflow on each target |
| The night log grows a secret by quoting an error | Section 12 rule; N6 scans it and the lane logs |
| The report is too long to read | Section 13 puts the decision on the first line and acceptance second |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Lanes | Twelve workflows in this session; cloud sessions only as fallback | Owner |
| `REPORT_AT` | H+16 | Owner |
| Cut target | H+10; later if the gate is green later, before `REPORT_AT`, with a W6 call at H+10 | Owner |
| Build spend report lines | 200 and 400 USD model, 75 and 150 USD sandboxes; no cap (ledger T6) | Owner |
| Per run cap | 10 USD (ledger T6, locked), checked every 5 minutes on every pad row and A1 to A3 | Owner (ledger) |
| A1, A2, A3, A4 length | 2 h, 60 min, 30 min, 20 min | Owner |
| Kill in A2 | recorder at minute 30; `kill-watchdog-recover@45` | PRD-02 |
| Idle before "keep going" | 3 minutes | Owner |
| Runs at once per key | 2 for Claude, OpenAI and gateway; A1 holds OpenAI alone; up to 6 for DeepSeek until 429s pass 5% | PRD-16 |
| Phase-miss reordering | after 2 hours late | Owner |
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
- `scripts/check-version.py` is red on today's `docs/plans/`: land PRD-14's change first after the docs
  merge, back to back with the first workspace merge (section 2 step 4).
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
`ACCEPTANCE.md`; in a check box, with the fetched copy and A1's kept local parts copied in, run
`goalcup-verify check --server <fetched A1> --local <A1 entry>` and compare the two SHA-256 sums; then run `scripts/pad/table --release-check <commit>` and see exit 0.

## Asks (ruled)

All ruled by PRD-00 revision 3 (R3-8, R3-9, R3-11) and applied: the phase hours and the DeepSeek exception
in PRD-00, H+10 in PRD-13, no conductor fallback in PRD-14, DeepSeek parallelism in PRD-12. None left; the
replan keeps new asks in Amendments notes.

## Review answers

Answers to the three reviews of 30 Oct (CC = consistency check, A = review A, B = review B), for items
that concern this PRD or where it is the natural owner. This section is history (revision 2): where it
differs from the body above, the ledger or "Alignment answers", those win. Owner question numbers here are
revision 2's, written "old N (now M)".

| Item | Answer |
|---|---|
| CC 1 Asks not absorbed | Accepted where it touches this PRD: PRD-00 and PRD-01 ruled on this PRD's asks; the remaining four are in "Asks" |
| CC 2 Night cap | Accepted then; revision 3 drops the build cap (ledger T6) and keeps report lines at 200 and 400 USD (PRD-00 owner question 8, old 18) |
| CC 13, B C17 Old crates during phases 0 to 3 | Accepted. This PRD's `exclude` plan is the one; PRD-13 and PRD-14 dropped theirs |
| CC 14, B O1 Phase 0 order | Accepted. Docs merge, then PRD-14's gate change, then the workspace edit (section 2) |
| CC 15, CC 104, B C19 18-CPU conductor | Accepted. Removed. Lanes are parallel workflows in this session at 2 agents each; cloud sessions only as fallback (section 1) |
| CC 16 Phase 6 and the 2-hour run | Accepted. A1 runs on staging before the hand-off; it informs the merge, it is not a done item; the owner may repeat it on production |
| CC 17, B A11 Dodger exit code | Accepted as PRD-12 rules: exit 1 from `listed_hosts_decrypted: named`; N1 lists the observable pass lines |
| CC 19 Server access | Accepted. Read-only credentials on the conductor only (section 1) |
| CC 22, B N14 Staging floor from the first deploy | CC 22 accepted. N14 rejected, as PRD-00 ruled: no 0.7 client runs on staging in this build, and it tests the refusal path |
| CC 23 New owner questions | Accepted. Run 1's prompt and "2 swp" are PRD-00 old 16 and 17 (now 6 and 8) |
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
| B verdict 2 Too big for one day | Partly. The slices, the deferred list (PRD-00) and narrower real-run phases take it in. Phase hours moved to H+8, H+10 and H+16, honest about the size; S2 reorders phase 5; nothing is dropped for time |
| B C19, B7 Reviewers | Accepted. Two reviewers per merge request, the breaker once per core merge request, not per commit (sections 1 and 4) |
| B A3 Owner questions in three places | Accepted for this PRD. Its open questions moved to PRD-00's list; the report reads that list |
| B A4 Phase 1a slice | Accepted. Phase 1a starts on PRD-00's core slice, `--server off` |
| B A10, M10 Replan edits PRDs mid-build | Partly, as PRD-00 ruled: Amendments notes appended to a PRD, never a rewrite of a section in use; no separate `AMENDMENTS.md` |
| B R1 Hermes cannot pass C1 | Accepted. N1 and X1 use the totals form for Hermes (PRD-00 old 19, now 7) |
| B B6 Real-run size | Accepted then. Gateway and dodger `smoke` in phase 4 (revision 3: all `full`, ledger T5); A3 30 minutes; A4 is Kilo's phase 5 `gateway` `full` row. Partly: A2 stays 60 minutes (review A asks for longer load before A1) |
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

## Alignment answers

What this PRD did with each fix of the alignment pass (fix lists g1, g2, g3). The ledger wins; where two
fix lists differ, the answer says which text was taken and why.

### Group 1

| Id | What was done |
|---|---|
| L5-01 | Applied, merged with G3-X11. Section 9: one per-run row for every pad row and A1 to A3, `pad` checks every 5 minutes in the box and seals at 10 USD; owner decisions, interfaces PRD-09 and PRD-12, risk row and defaults follow. Same as PRD-00 and PRD-12 |
| L5-02/L6-09 | Applied, merged with P00-1. No whole-build cap and no 2 USD `smoke` cap: section 9 rows become `spend` report lines (200 and 400 USD model, 75 and 150 USD sandboxes); "Why 400" becomes the expected spend; S4 kept as "(removed)" so ids hold; W4 and the defaults reworded |
| L6-05 | Applied, merged with G3-X1. Conductor row: `fetch-run` and read-only queries only, never `goalcup-verify`; pad box row holds the fetched copy and the per-run known-values file; X6, X7, X8, N6 and the watcher proof run in a box |
| F1 | Applied, merged with P00-8 and G3-X6. Section 5 "Cut target H+10", S7, W6, defaults and interfaces: owner called under W6, the cut lands in the same build once the gate is green, before `REPORT_AT`, else `MORNING.md` line 1 says "Not ready" |
| L5-03 | Applied with G3-X13. Phase 4 and N1: all nine core rows `full` |
| L5-04 | Applied. Phase 5 row: a PRD-09 reader from each harness's first `full` sample, one rerun, else "not verified: no log reader". Superseded on 30 Oct by ledger T4: a reader is optional and its absence is no tier reason |
| L6-01 | Applied, merged with G3-X2. Lanes paragraph: all lanes in parallel, this table replaces BUILD-PROMPT section 4's (R3-10), the ledger wins over both |
| L6-02 | Applied, merged with G2-04 and G3-X3 (Replan row and "Files it owns") |
| L6-03 | Applied, merged with G3-X5. Merge rule item 4: every finding fixed before the merge, a decline needs the raising reviewer's acceptance, the never-declined list includes the seven points and locked ledger items |
| L6-04 | Applied, merged with X3-2 and G3-X4. Section 2 opening paragraph and step 4; N4 follows |
| L6-07 | Applied (section 11 intro). BUILD-PROMPT's part is that file's |
| L6-08 | Applied. S2 reorders and never drops, S3 runs go on past `REPORT_AT`, risk row, defaults row, review answer B verdict 2. Section 11 notes S3's one end notice as the only call outside W1 to W7 |
| L6-10 | Partly. Taken: `GH_TOKEN` creates the repo on the conductor and never enters a box (section 1, section 8, W1). Rejected: the deploy key and private repos; PRD-12 (its L6-10 answer) keeps G3-X12's fine-grained push token and a public repo, because PRD-10's start path pushes over HTTPS with a `GH_TOKEN=` line and needs a public repo (point 7) |
| L4-7 | Applied. Section 13 owner notes list the M1 reading if not answered before H+0 |
| L6-11 | Applied. Reviewers row: at least two, each a fresh agent writing findings before seeing the other's |
| L6-12 | Applied. Builder row: task branches merged into the PRD branch by the lane |
| L6-13 | Applied. Phase 1a lists each second and third slice core feature; exact values only with the owner's yes (PRD-00 owner question 3) |
| L6-15 | Applied, with G3-X42 (owner decisions, phase 0 step 1) |
| L6-16 | Applied, with X3-16 and G3-X10 (H+3 in section 3, lanes and interfaces); "Asks" replaced, with G3-16b |
| L6-18 | Applied. S1, the watchers paragraph and the Watcher row: one named cause; the signature only groups rows |
| L6-19 | Applied (section 4, CI paragraph) |
| L6-21 | Applied (section 3, first staging deploy) |
| L7-07 | Applied. Stop table: "beyond the changes ledger section 7 lists" |
| L7-09 | Applied. Section 13 owner notes carry the email cron note (PRD-07 points here) |

### Group 2

| Id | What was done |
|---|---|
| G2-01 | Applied. Phase 1a precondition: PRD-05's core-slice masker merged, checked in the `start` event. Partly: "a `mask_rules` length above zero" became "names the shared kinds and not only `fail_closed`", because the S2 stub itself may list `fail_closed` and would pass a length check |
| P00-1 | Applied, merged with L5-02/L6-09 |
| X3-2 | Applied, merged with L6-04 and G3-X4 |
| 06-4 | Applied (X5) |
| G2-04 | Applied, merged with L6-02 and G3-X3. Replan runs per merge of a PRD branch, in the integrator lane, and amends this PRD and PRD-00 "Build order" as the build plan |
| G2-05 | Applied as G3-X27's wording (same rule, also PRD-00 phase 0 step 3) |
| G2-10 | Applied. Breaker row (40 then 200 synthetic subagents, no replay tool) and X3 (200-stream result reported, its `memory_guard` gaps a night-log line) |
| G2-38 | Applied. Replan posts `wc -l` against the PRD-00 "Size budget" row; reviewer B checks the row |
| P00-8 | Applied, merged with F1 and G3-X6 |
| X3-12 | Applied, with G3-X58 (lanes paragraph and Watcher row) |
| X3-13 | Applied (section 6, after the keys paragraph), with the release check's full path list from PRD-12 section 9 |
| X3-15 | Applied, merged with G3-16a. Step 7: rebuild the snapshot after step 6 and at each phase exit; other lock changes use `cargo fetch --locked` in the box, cheaper than a rebuild per merge |
| G2-12 | Applied, with G3-16c, against PRD-00's current list of twelve (not eight as the fix list says; PRD-00 revision 4 added 9 to 12): old 16 is 6, old 17 and 18 are 8, old 19 is 7 |
| X2-10 | Partly. Section 13 points to PRD-00 "Morning report template" as the one list and keeps only rules the template lacks (line 1 condition, acceptance line placement, links, owner notes, newcomer run). Kept because L4-7 and L7-09 put the owner notes here |
| X2-11 | Rejected for now. This fixer owns one file, and `REVIEW-HISTORY.md` does not exist; no other PRD has moved its section. Partly done: "Review answers" now opens with a history note (the body, the ledger and these answers win) and its stale lines are marked. "Asks" is reduced to one paragraph |
| X3-16 | Applied, with L6-16 |
| X3-24 | Applied. Step 6 names PRD-01's `Cargo.toml` and says builders never commit `Cargo.lock` |
| X3-25 | Partly. Step 3 is the one merge before `gate fast` can pass (G3-16h). Step 4 is not exempt: per G3-X27 its `gate fast` runs on the first workspace merge, and the two merge back to back |
| X3-26 | Applied (lanes paragraph) |

### Group 3

| Id | What was done |
|---|---|
| G3-X5 | Applied, merged with L6-03; reviewer A row names the seven points and locked items as blockers |
| G3-X1 | Applied, merged with L6-05 |
| G3-X27 | Applied (step 5 `members` bullet, step 4) |
| G3-16a | Applied, merged with X3-15 (steps 6 and 7) |
| G3-X3 | Applied, merged with L6-02 and G2-04. Reviewer A of the lane reads the note, since the Amendments commit is docs-only (G3-X4) |
| G3-X4 | Applied, merged with L6-04 and X3-2; N4 counts commits outside `docs/plans/` |
| G3-X11 | Partly. Taken: the 5-minute check, PRD-12's per-model rate lines, the seal at 10 USD. Rejected: "watcher, `full` rows only; `smoke` bounded by length". PRD-00 and PRD-12 (L5-01) run the check from `pad` on every run, since T6 is per run |
| G3-X6 | Applied, merged with F1 and P00-8. One change beyond the fix text: after a late cut, all nine core rows and the Codex kill row rerun (not only `direct`), because the release check compares `crates/` and `Cargo.lock` for every core row (PRD-12 section 9). PRD-00 phase 3 still says "`direct` rows"; the replan should align it |
| G3-X2 | Applied, merged with L6-01 |
| G3-X7 | Applied (X1, X2) |
| G3-X8 | Applied (X6). Partly: PRD-06 section 6 also sends `failed` when the writer is idle over 60 s while records wait, so X6 names both conditions, not `mode` null alone |
| G3-X26 | Applied (X6 reads `fetch.json` `heartbeats`; interfaces PRD-07) |
| G3-X13 | Applied, with L5-03 |
| G3-X30 | Applied (X8) |
| G3-X31 | Applied as a paragraph in section 6 (it is not an A run), with `README.md` and the guide as PRD-15 acceptance 1 states; section 13 and interfaces PRD-15 point to it |
| G3-X12 | Applied (section 1 hosts, section 8, W1); see L6-10 |
| G3-X28 | Applied (N5) |
| G3-X10 | Applied, with L6-16 |
| G3-16b | Applied. Header says revision 3; "Asks" says all are ruled and already applied in PRD-00, PRD-12, PRD-13 and PRD-14 (checked), so none is left to apply |
| G3-16c | Applied, with G2-12 |
| G3-16d | Applied. Checked: `Cargo.toml` lines 13 to 17 hold the comment |
| G3-X42 | Applied, with L6-15 |
| G3-16e | Applied (section 3 seam change, risk row) |
| G3-16f | Partly. Phase 2 exit runs the Codex kill row once, but on `full`, not `smoke`: the kill minutes 8 and 10 fall after a 3-minute `smoke` run ends (PRD-12 section 1) |
| G3-16g | Applied (conductor "Holds") |
| G3-16h | Applied (step 3, N7) |
| G3-X44 | Applied (build boxes row, step 7) |
| G3-X58 | Applied, with X3-12 |
| P16-19 | Not changed here: the commit attribution line goes to the owner (fix list g3) |
| P16-10 | No change needed (fix list g3) |

### Found on the re-read against the ledger

| What | Done |
|---|---|
| Section 13 named PRD-00 done items "1 to 7"; PRD-00 now has nine | Line 1 rule says 1 to 9 |
| X9 cited PRD-06 U7, now split into U7a and U7b | X9 and interfaces PRD-06 cite both |
| Section 14 step 3 reran only the core `direct` rows on a moved `main` | All nine core rows and the Codex kill row, as the release check needs |
| Section 6 listed fewer paths than the release check compares | The list now matches PRD-12 section 9 |

**R3a, R3b, T4 carried in.** A2 adds PRD-12's fault `kill-watchdog-recover@45` after the minute-30
recorder kill; X4 now expects two `recorder_down` gaps, no `null_mode` gap (R3a), the same harness pid,
port and run certificate after `goalcup recover` (R3b); X7 scans for the run certificate key. The proxy
record is the proof (T4): X1, X2 and N3 pass on the record (no unexplained gap, the history check, every
subagent recorded) and the harness's local log comparisons are optional extras; X8 states usage comes
from the record only and the run 1 ratio is informative; section 9 and W4 read no provider dashboard,
so no check calls an outside API. Points served, owner decisions, phase 1a, risks and defaults follow.
