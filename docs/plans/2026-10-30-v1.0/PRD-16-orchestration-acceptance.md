# PRD-16: Build orchestration and acceptance

Id: PRD-16. Version: goalcup 1.0. Status: plan for the one-day build, revision 4 (ledger B3, B9, B10 and B11 carried in;
revision 3: PRD-00 rulings R3-8 to R3-11 and the alignment pass applied). Date: 30 October 2026. Base: main at 4b226de2. Index: [PRD-00](PRD-00-index.md). Ledger:
[OWNER-DECISIONS](OWNER-DECISIONS.md). Format: [PRD-01](PRD-01-record-format.md).

## Goal

Run one build day in which many agents build goalcup 1.0 from PRD-01 to PRD-17, prove it on real harnesses
in Vercel Sandboxes, and leave the owner one page at the end that says "ready to merge" or "not, because".
Nothing reaches `main`, a tag or production without the owner.

## Points served

All seven, as the gate that decides whether the build serves them. 1 and 2: the proxy record is the
proof (ledger T4): no unexplained gap, the history check, every subagent recorded; the harness's own
local log is an optional extra check where it keeps one. 3: the kill and fault runs, the crash and stand-back-up step
of every cycle run, `goalcup recover` after a watchdog kill, and the load runs. 4: secret scans of every record, server copy, log and pushed repository. 5: server copy
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
- **The test ladder, from tiny to massive** (ledger B3), every rung through PRD-12's rig (section 1a):
  (1) **micro tests** during phase 1: one harness, one key, a few calls, local capture first
  (`server = "off"`, nothing uploaded), then one layer at a time (section 3, phase 1a); (2) **cycle runs**
  per pad, a few minutes each, for every harness and case as soon as its connector lands, many in
  parallel (section 3, phase 1b); (3) **full runs** at the end: 20 minutes with the payments prompt, every
  pad, all in parallel, until they pass (section 6). Every run cleans up its sandbox, test repo and local
  files (PRD-12 section 3.2); every build box is removed when its job ends.
- **One small rig for agents** (ledger B9): every real run is `scripts/pad/pad <settings file>`. An agent
  writes the settings, runs the command and reads the table; it never drives a step by hand. The rig is
  trusted only after `selftest` fails it on run 1's known bugs and the rig reader has read its first
  outputs by hand against the record (section 3, "Trusting the rig").
- **Do not overbuild tests** (ledger B10). The rig's real runs are the main proof. Automated tests guard
  only the seven points, the format, masking and the never-stop rules. Reviewer B flags a test that pins
  an internal shape or wording, or takes longer to change than the code it guards (section 1).
- **All the parallelism we can afford** (ledger B11). Every build, test and experiment runs in Vercel
  Sandboxes, many at once, with no box count of our own: only the per-key queue (section 9) and the
  account's own limit. This session only drives agents: many subagents for tasks, reviews, rig runs and
  fixes, in as many workflows at once as the work needs.
- Watchers fix and rerun; three failed fixes on one cause end that loop with a note (S1, BUILD-PROMPT
  section 5).
- **Staging only**: every run uploads to the staging Worker, approved with the staging test approval key and the owner's member id;
  agents may deploy staging; production, the merge to `main` and tags are the owner's; no website
  database schema migration (ledger B4 to B6, S3).
- Spend: 10 USD per run (ledger T6), checked live by the pad every 5 minutes; no whole-build cap, since
  cost is otherwise not a constraint (section 9). Full pad runs are about 20 minutes of real work with many subagents
  (ledger T5); micro and cycle runs are shorter by design (ledger B3). Keys in 0600 files, separate keys per harness where possible, read-only database and R2
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
| Agent concurrency | the workflow runner's own limit | At most min(16, CPUs minus 2) agents at once per workflow. On this 4-CPU container that is **2** per workflow; so several workflows run at once (ledger B2), as many as the work needs (ledger B11) |
| Vercel Sandbox | `docs/research/VERCEL-COMPAT.md:41`, `:50`, `:52` | Sessions end at 45 minutes on Hobby, 24 hours on Pro; Pro boxes up to 8 vCPU and 16 GB; $0.128 per active CPU-hour, $0.0212 per GB-hour. The account's limit on boxes at once is unverified; read at H+0 |
| DeepSeek limits | https://api-docs.deepseek.com/quick_start/rate_limit | Concurrency limits per account, at least 500 requests at once, a 429 past it. So one DeepSeek key can feed six pad boxes at once |

## Design

### 1. Hosts, lanes and roles

| Host | What runs there | Holds |
|---|---|---|
| **Conductor** | This cloud session, 4 CPUs, root. The lane workflows, their agents, git worktrees, `scripts/pad/pad` (conductor half: sandboxes, approval, test repos, fetch, copy-in, read-only queries), `fetch-run`. `fetch-run` runs here because only the conductor holds the read-only credentials. It never compiles, tests or runs `goalcup` or `goalcup-verify`: it copies the fetched folder into the run's pad box (or a build box) with `sandbox copy`, and every check runs there (PRD-12 section 8). Nothing from the build under test executes on the conductor (ledger B1) | The approval key and the owner's member id, read-only database and R2 credentials, the `goalcup-usage-staging` write token (PRD-09), `GH_TOKEN` (creates test repos; never enters a box), the Cloudflare token for staging, the known-values files, all key files and push token files under `~/.pad-keys/` (0600) until copied into a box. Never a production write credential |
| **Build boxes** | Vercel Sandboxes, 8 vCPU and 16 GB, user `ubuntu`, from the build snapshot. One per builder task, test run, reviewer check or gate run, as many at once as the account allows (ledger B11); removed when its job ends | The Rust toolchain from `rust-toolchain.toml`, nextest, zstd, Node 22, Python 3, a warm cargo cache. No key, no token |
| **Pad boxes** | Vercel Sandboxes from PRD-12's pad image, 4 vCPU, `--non-persistent`. One per rig run, as many at once as the per-key queue allows (section 9); removed by the rig's `cleanup` step (PRD-12 section 3.2) | One harness; its key file, for `gateway` the gateway key file, and the run's push token (fine-grained, throwaway pad owner, Contents read and write; never `GH_TOKEN`), each 0600, for the life of the run; `goalcup` and `goalcup-verify` from the build under test; after the seal, the fetched copy and the per-run known-values file (only values that entered this box, deleted after the scan) for the checks |
| **Staging Worker** | `goalcup-ingest-staging` on Cloudflare, 1.0 code from the integration branch | Its own bucket; the shared database through the Worker's own calls only |

Why build boxes: the gate refuses root (`scripts/gate.sh:32-39`), the conductor runs as root, and many
builders compiling a TLS, HTTP/2 and zstd stack on 4 shared CPUs would queue for hours. A build box is
non-root, has 8 CPUs, and is thrown away. Code goes in as a `git bundle` with `sandbox copy`, so no token
ever enters a box. JUnit and the gate's last line come back out.

**Lanes.** Each lane is one workflow in this session, at most 2 agents at once, and all lanes run in
parallel (ledger B2). The real-runs lane runs as many workflows of 2 agents as there are rig runs and
watcher loops in flight (ledger B11); there is no count of our own. This table replaces BUILD-PROMPT section 4's older table (PRD-00 R3-10);
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
| Harnesses and pad | PRD-11, PRD-12 (the rig first, ledger B9) | H+0 |
| Participant | PRD-10 (`main.rs` stubs in phase 0) | H+0 |
| Test support and gate | PRD-14 (gate change first, `tests/common/`) | H+0 |
| Cut and docs | PRD-13 (read only, its copy ledger, until phase 3), PRD-15 | H+0 |
| Real runs | Rig runners, the rig reader and watchers: micro rungs (phase 1a), cycle runs per pad (phase 1b), full runs (phases 4 to 6) | when the core slice records |

**Roles inside the lanes:**

| Role | Count | Does | Never |
|---|---|---|---|
| Conductor (this session) | 1 | Starts and watches the lane workflows; phases, timers, queues, caps, the call list. Deterministic control flow in each workflow script | Writes product code |
| Integrator | 1 | Phase 0, merges by section 4, `gate fast` on every merge and `gate full` at phase exits in a build box, staging deploys, the build snapshot | Merges a branch that fails section 4 |
| Scribe | 1, in the integrator lane | Merges the lane logs into `NIGHT-LOG.md`; writes `MORNING.md` and `ACCEPTANCE.md` | Copies a key, token or planted value into any file |
| Planner | one per PRD | Reads the PRD, the ledger, its seams, the code and the PRD's Amendments note; writes a short task list, each task inside the PRD's owned paths | Plans a task outside ownership |
| Builder | one or more per PRD | Builds tasks on `v1.0/prd-NN-<slug>-<task>` from the PRD branch, in its own worktree, merged into `v1.0/prd-NN-<slug>` by the lane before its merge request;, runs its tests in a build box | Edits a path its PRD does not own |
| Reviewers A and B | at least two per merge request | A: correctness, the seven points, the ledger (a break of one of the seven points or of a locked ledger item is a blocker), masking before disk, no drop without a GAP. B: code quality and less code, the PRD's row in PRD-00 "Size budget", and overbuilt tests (ledger B10: a test that pins an internal shape or wording, or takes longer to change than the code it guards, is deleted or simplified). Given the diff and the PRD, not the builder's summary. Each is a fresh agent, not the builder, and writes its findings before it sees the other reviewer's | Rewrite the code; they return findings |
| Breaker | one per core merge request (PRD-01, 02, 03, 05, 06) | Tries to break it against run 1's shape (about 40 and then about 200 synthetic subagents, 1.5 MB bodies in bursts, a held writer, WebSockets) with PRD-02's `pipeline_guard.rs`, `pipeline_stall.rs` and `by_hand_load`, and the latest pad smoke row, once per merge request. There is no run 1 replay tool | Runs a real 2-hour replay per diff |
| Replan | one per merge of a PRD branch (each slice), the last step of that PRD's lane pipeline, run in the integrator lane | Reads review findings, test and real-run results; appends an "Amendments" note to each remaining PRD it affects and to this PRD, which with PRD-00 "Build order" is the build plan of ledger B2a (lanes, order, phases, runs); posts `wc -l` of the merged PRD against its PRD-00 "Size budget" row; one docs-only commit on integration with an `amendment` night-log line. Reviewer A of that lane reads the note against the ledger. The conductor re-reads this PRD's Amendments before each phase and each new lane task; planners read their PRD's before each task | Rewrites a section a builder is working from, edits BUILD-PROMPT.md, OWNER-DECISIONS.md or VISION.md, or changes a locked ledger item (that calls the owner, W6) |
| Rig runner | one per rig run in flight, no count of our own (ledger B11) | Writes one settings file (PRD-12 section 1), runs `scripts/pad/pad <file>` on the conductor, reads the table, writes a `run` log line with the row link; a FAIL or INCONCLUSIVE row goes to a watcher | Drives a step by hand, runs `goalcup` on the conductor, or leaves a box, repo or folder behind without a log line naming it |
| Rig reader | 1, a fresh agent in the real-runs lane | Reads the rig's first `micro` table and first `cycle` table by hand against the record (PRD-12 section 13 step 1's `dump`, in a check box) and says per step whether it tested what its line claims; again after any change under `scripts/pad/` (ledger B9) | Trusts the table's own words |
| Watcher | one per named cause, as many at once as there are causes (ledger B11), each its own real-runs workflow of 2 agents, started by the conductor | PRD-12 section 13: read, name the cause, reproduce, fix on the owning branch, rerun | Changes a check to make it pass without record evidence |

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

1. At H+0: the Vercel plan and session limit (Pro, or A1 cannot run), the account's limit on boxes at
   once (ledger B11 runs as many as it allows), that a `v1.0/*` push works, and which
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
- **Build-box rule.** No agent runs cargo on the conductor. A builder sends a bundle to its own build box
  and runs `cargo nextest run -p <its crate> -E <its tests>`, then `gate fast`, before asking to merge.
  Every builder, reviewer check and gate run gets its own box, many at once (ledger B11); the box is
  removed when the job ends.
- **Tests, not too many** (ledger B10). A builder writes automated tests only for the seven points, the
  format, masking and the never-stop rules, plus its PRD's acceptance cases. No test pins an internal
  shape or wording. The rig's real runs are the main proof.
- **Merge requests early.** A lane asks to merge whenever a slice of its PRD is green, not only at the end.
  Seams land first as compiling stubs by the "stable by" hours of PRD-00's seam table.
- **Seam change.** Only the seam's owner changes a seam, with a log line `seam S<n>: <change>, users
  <PRDs>`. Users merge integration into their branch within the hour (merge commits, no rebase of a
  pushed branch).
- **Vendor docs first.** Every builder of a harness-specific part reads the vendor's current docs and cites
  the page in the commit message (ledger T7).
- **PRD-17** publishes `LESSONS.md` by H+3 (PRD-00 R3-8, PRD-17 builder notes); `tests/lessons_*.rs` follow as
  modules land.

**Phase 1a, the micro rungs (ledger B3, step 1: micro tests).** As soon as the core slice records one
request (PRD-00 core slice), the real-runs lane climbs PRD-12 section 1a's ladder through the rig, one
rung at a time. Precondition: PRD-05's core-slice masker (option b, the left guard, `boundary()`) is
merged on the commit under test, checked in the `start` event, whose `mask_rules` names the shared kinds
and not only `fail_closed` (the S2 stub). No real key enters a pad box before that (ledger M1, point 4).

- **Rung 1, local capture** (PRD-12 P2): Claude Code with the Claude key, and Codex with the OpenAI key,
  each `mode = "micro"`, `case = "direct"`, `server = "off"`: nothing uploaded. Until the pad's sandbox
  lifecycle lands, PRD-12 builder notes' hand-made pad box with `where = "here"` stands in.
- **Then one layer at a time,** each rung as soon as the feature it needs is merged. Rungs 2 and 3 build
  on the rung before; rungs 4 to 9 each change one line of rung 3's settings, so a failure points at one
  layer: rung 2 upload to staging (after the first
  staging deploy), rung 3 subagents, rung 4 a recorder crash (watchdog restart), rung 5 a watchdog kill
  and `goalcup recover`, rung 6 "keep going", rung 7 the gateway case, rung 8 the dodger case, rung 9
  HTTP/2. Each rung runs first on one harness and one key, then on the other core harness in parallel.
- **Core features with no rung of their own** (native capture, refusal learning, Hermes `state.db`, and
  exact-value masking only if the owner said yes, PRD-00 owner question 3) get one micro run on a harness
  that uses them, the day they merge.
- When a rung fails, its layer is fixed before the next rung. If rung 1 fails, no lane adds a new feature
  until it is fixed (BUILD-PROMPT section 5). A micro run costs a few minutes; rerun freely.

**Trusting the rig (ledger B9).** `selftest` runs as soon as the rig and the checks it maps are merged
(its offline controls cost nothing; K5, K7 and K8 are short real runs), not first at the phase 2 exit, then again
after any change under `scripts/pad/`, `crates/goalcup-verify/` or a check threshold (PRD-12 section
12). The rig reader reads the first `micro` table and the first `cycle` table by hand against the record.
Until both hold, a rig row proves only that a piece is live and gives no tier (`pad_trusted_at` empty).

**Phase 1b, cycle runs per pad (ledger B3, step 2).** As soon as a harness's connector (PRD-11) and its
pad harness file (PRD-12 section 6) are merged, the pad runs in sandboxes (second slice) and staging is deployed (below),
the real-runs lane starts a `cycle` run for that harness with `case = "all"` (three sandboxes at once, each
case with a key) on staging. The rig walks it in about 12 minutes: setup, data flows, look at the
recorded data, a recorder kill at minute 3 and the wait for it to stand back up, "keep going" where the
harness file says `keep_going = true`, the seal at minute 8, upload, fetch, the server copy check,
cleanup. Many at once: every harness with a key, limited only by the per-key queue (section 9). The core
set first (Claude Code and Codex in the second slice, Hermes when its connector lands), the other 12 as
the third slice lands each connector. A cycle FAIL goes to a watcher at once (PRD-12 section 13), so most
fixes land before the full runs. A cycle row carries its crash, so it is a fault row and never gives a
tier (PRD-12 section 1a). It reruns after any merge that touches its harness or the core.

**First staging deploy.** As soon as PRD-07's builder step 2 (heartbeat and floor) and PRD-06 are merged
(PRD-07 section 12 step 2), the integrator deploys staging: `wrangler deploy --env staging` from the
integration head, with `MIN_COMPETITION_VERSION = "1.0.0"` in both blocks of `wrangler.toml` (PRD-07
section 6). From then on, rig runs use `server = "staging"` and check the server copy. Every staging deploy
writes a log line with the commit and the Worker version id. A 0.7 client on staging is refused from then
on; nobody runs one in this build, and it tests the refusal path. Before that deploy, runs use
`server = "off"` or `"local"`: they upload nowhere, so B4 holds. The integrator deploys staging the
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
  the core set's `cycle` rows PASS on staging (Claude Code, Codex and Hermes, every case: the pad end to
  end on staging); the Codex kill row (PRD-02 R1, PRD-00 phase 2)
  once, `direct` with `--kill-recorder 8,10` on `full`, because both kill minutes fall after a 3-minute
  `smoke` run ends; PRD-12's `selftest` reruns on the phase 2 commit and the pad is trusted there (PRD-12 P1; its first
  run was in phase 1, section 3); `gate full` timed (PRD-14 G1).
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

### 6. Phase 4, 5 and 6: full runs, all in parallel (H+10 to about H+15)

The top rung of the ladder (ledger B3): `mode = "full"`, 20 minutes, the payments prompt, for every pad,
all in parallel, until they pass. Phases 4 and 5 start together; the core set has first claim on the
Claude, OpenAI and gateway keys, and the other harnesses run beside it on their own keys. There is no box
count of our own (ledger B11); the per-key queue of section 9 is the only limit. A row that fails goes to
a watcher; after each fix merges, every row it can touch reruns, all at once, until every row passes or
S1 ends that loop. Every run ends in the rig's `cleanup` step.

All runs use PRD-12's pad on staging, with binaries built from the integration head in a build box. The
commit under test is in every row. A fix merged during a phase starts a rerun of the rows it can touch; the
release check (PRD-12 section 9, PRD-14) later demands that the tagged commit equals the pad's commit under
`crates/`, `server/worker/src/`, `connectors/` (except `connectors/tiers.toml`), `Cargo.lock`, `Cargo.toml`
and `rust-toolchain.toml`.

| Phase | Runs | Parallel | Est. time |
|---|---|---|---|
| 4. Core set | Claude Code, Codex, Hermes: `direct`, `gateway` and `dodger`, all `full`, the 20-minute payments prompt (ledger T5, PRD-00 done item 1): 9 rows. The kill rows `--kill-recorder 8,10` on all three `direct` `full` (PRD-12 P10). The fault rows once each (PRD-12 P11) | All 9 rows and the kill and fault rows at once; per key, section 9 | about 2 h with fix loops |
| 5. The rest | The other 12 harnesses, each case that applies; no key means "not verified: no key" with no run (Amp, Factory Droid, OpenRouter cases). All rows run `full`. Kilo's `gateway` row is A4. As each harness's first `full` sample lands, a PRD-09 builder may write its log reader from that sample (PRD-09) for the optional local log check. A harness with no reader is judged from the record (ledger T4); a missing reader is never a "not verified" reason | All at once, together with phase 4 (ledger B3: every pad in parallel); per key, section 9 | about 2 h with fix loops, beside phase 4 |
| 6. Acceptance | A1, A2, A3 (section 7) | Starts when the core set is green, beside phase 5; per key, section 9 | about 2.5 h, A1 is the longest |

Keys and rate limits: run 1 hit 2M tokens a minute on one OpenAI key with about 20 subagents (run report,
measure 9). So A1 holds the OpenAI key alone while it runs; a phase 5 run that would use the cheap OpenAI
model on the same key waits, or uses DeepSeek. The Claude key and the gateway key: at most 2 runs at once
each. DeepSeek: up to 6 runs at once (PRD-00 R3-9, PRD-12 section 7), because DeepSeek limits concurrency
per account at 500 or more requests (link in Today), far above six runs with 20 subagents each; a 429 share above 5% in a row's
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
`GOALCUP_SERVER`; the conductor approves it; about 15 minutes, a short prompt; the box is removed when it
ends (ledger B3). Its result sits in `MORNING.md` beside done item 6.

### 7. Final acceptance runs

Real runs on staging after the core set is green, each longer than a pad row, each with the pad's checks
(C1 to C6, PRD-12 section 8) plus the X lines below. The rig runs each from one settings file (`mode = "full"` with
`prompt`, `minutes` and the steps `keep-going` and `kill-recorder@m`; PRD-12 section 1 maps the old
flags `--prompt-file`, `--minutes`, `--keep-going` and `--kill-recorder`). Typing "keep going" when the agent sits idle is
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
| Boxes at once | No count of our own (ledger B11): as many pad and build boxes as the Vercel account allows, read at H+0. Per key: at most 2 runs at once per provider key, DeepSeek up to 6 (PRD-12 section 7); A1 holds the OpenAI key alone | A queue in the conductor per key; past the account's limit, runs wait in the queue |

Expected model spend is about 260 USD, at most about 360 (estimates, unverified): micro rungs and
selftest about 35; cycle runs about 60 (about 40 rows with reruns, most on DeepSeek); phase 4 up to 180
with fault rows and reruns; phase 5 about 25 runs on DeepSeek at 1 to 2 USD each; acceptance up to 30.
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
- **The ladder line**: per rung 1 to 9, the first PASS `micro` row; per harness, its last `cycle` row;
  and what cleanup left behind (N9, N10), each with a link.

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
| PRD-12 | The rig (`scripts/pad/pad <settings file>`), its modes `micro`, `cycle` and `full`, the ladder rungs 1 to 9 (section 1a), `cleanup` (section 3.2); `selftest` (its first run timed here, section 3), rows, `PAD-RESULTS.md`, `connectors/tiers.toml`, `--release-check`, the watchers' loop, the ledger, the per-model rate lines and the 5-minute spend check; `--prompt-file`, `--minutes`, `--keep-going`, `--kill-recorder`; P10 and P11 scheduled in phase 4 | Used here |
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
| N9 | **Every run cleaned up after itself** (ledger B3). At report time | No sandbox is left except those of rows still running (`sandbox list`); no `gc-pad-*` repository is left except those a cleanup line listed as "repo kept"; no key file in any kept run folder. Every leftover has a night-log line |
| N10 | **The ladder was climbed** (ledger B3). At report time | Rungs 1 to 9 each have a PASS `micro` row; every harness with a key has a `cycle` row before its first `full` row; the rig reader's two readings and the first `selftest` are in the log before any row gives a tier |
| N8 | **The report is complete.** At `REPORT_AT` | Every item of section 13 is filled, every claim links to a row, a gate line, a CI run or a commit; a reader who was away can decide in 10 minutes |

### Automated tests that guard them

None new (ledger B10: the rig's real runs are the main proof). This PRD writes no code. Its guards are
other PRDs' tests: PRD-12 `selftest` and `test_pad.py`
(the pad cannot pass a known fault), PRD-14's `gate fast` and `gate full` and PRD-12's
`table --release-check` (`test_gate.py` and `test_release.py` are deleted in phase 0, ledger B10), PRD-08's
check tests, and CI on every integration push.

## Risks and blind spots

| Risk | What we do |
|---|---|
| 2 agents per workflow make the build crawl | Twelve lanes as parallel workflows, and as many real-runs workflows as runs in flight (ledger B11); the three slices let the core slice prove the concept first; S2 reorders phase 5 and never drops a run for time. If the session cannot hold the lanes, the cloud-session fallback (section 1) |
| Many lanes on one repository step on each other | One owner per path, worktrees, seams as stubs first, every dependency in the scaffold, merges only through the integrator, lane logs off git |
| A seam changes late and three PRDs break | Seam changes only by the owner with a log line; users merge integration into their branch within the hour |
| A replan edits a PRD a builder is reading | Replan only appends an Amendments note; planners read it before each task |
| Vercel Sandbox limits: concurrent boxes, session length, egress | The account's limit on boxes at once is unverified; read at H+0. There is no count of our own (ledger B11); past the limit the conductor queues. A1 needs a box of about 2.5 hours: Pro allows 24 hours, Hobby 45 minutes (`VERCEL-COMPAT.md:41`), checked at H+0. Never turn on credential brokering for a model host (`VERCEL-COMPAT.md:46`) |
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
| The rig passes a run it should fail: it tests the wrong thing (ledger B9) | `selftest` must turn run 1's known bugs into FAIL; the rig reader reads the first `micro` and `cycle` tables by hand against the record; both again after any change under `scripts/pad/`; no tier before both |
| Many runs at once leave boxes, repos or folders behind | The rig's `cleanup` step always runs; boxes are `--non-persistent` with a timeout; at each phase exit the integrator lists sandboxes and `gc-pad-*` repositories and removes any not tied to a running row, with a log line; N9 |
| Tests grow until a one-line change costs three test edits (ledger B10) | Reviewer B flags such tests; they are deleted or simplified; the rig's real runs stay the main proof |
| The night log grows a secret by quoting an error | Section 12 rule; N6 scans it and the lane logs |
| The report is too long to read | Section 13 puts the decision on the first line and acceptance second |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Lanes | Twelve workflows in this session, plus as many real-runs workflows as runs in flight; cloud sessions only as fallback | Owner |
| Boxes at once | No count of our own; the Vercel account's limit and the per-key queue (ledger B11) | Owner (ledger) |
| Cycle run | 8 minutes to the seal, recorder kill at minute 3, `case = "all"` (PRD-12 section 1a) | PRD-12 |
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
5. Phase 1a micro rungs, one layer at a time; `selftest` and the rig reader as soon as the rig lands;
   phase 1b cycle runs per pad as each connector lands, many at once.
6. Phase 2 exit, phase 3 merge, phases 4 to 6 (full runs, all in parallel), the report.

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
- Never drive a rig step by hand: change a settings line and run the rig again (ledger B9).
- Every run ends in the rig's `cleanup`; at each phase exit, list sandboxes and `gc-pad-*` repositories
  and remove any not tied to a running row (N9).

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

**B3, B9, B10, B11 carried in.** B3: the test ladder now shapes the phases. Phase 1a is the micro rungs
of PRD-12 section 1a, local capture first (rung 1, `server = "off"`), then one layer at a time (upload,
subagents, recorder crash, watchdog kill and `goalcup recover`, keep going, gateway, dodger, HTTP/2);
new phase 1b runs `cycle` rows per pad, many in parallel, as soon as each connector lands; the phase 2
exit asks for the core set's `cycle` rows on staging; section 6 makes phases 4 and 5 full runs for every
pad, all in parallel, started together, until they pass (phase 5 no longer waits for phase 4's first
wave). Every run cleans up: build boxes are removed after each job, N9 checks nothing is left, a risk row
and a trap sweep leftovers at each phase exit. B9: every real run goes through `scripts/pad/pad
<settings file>`; new roles rig runner and rig reader; `selftest` runs first in phase 1 ("Trusting the
rig"), not only at the phase 2 exit; no tier before the hand reading; section 7 uses settings files; N10
checks the ladder. B10: reviewer B flags overbuilt tests; the build-box rule limits tests to the seven
points, the format, masking and the never-stop rules; a risk row. B11: no box count of our own anywhere
(the 6 pad boxes, the build box count and the six-watcher cap are gone), only the per-key queue and the
account's limit, read at H+0; watchers and rig runs get as many workflows as are in flight. The expected
spend adds about 60 USD for cycle runs. Left for others: PRD-00 phase 5 still says "from phase 4's first
wave" and "up to 6 pad boxes", and BUILD-PROMPT section 5 says the other 12 follow the core set's first
wave; the ledger wins. Done in the 30 Oct sweep: PRD-00 phases 1a, 2 and 5, BUILD-PROMPT section 5 and PRD-12's
watcher count now follow B3 and B11. S1 (three failed fixes) is
kept and now cites BUILD-PROMPT section 5, since B3 no longer carries it.

## Amendments

### 2026-10-30, replan after PRD-07 slice 1 (merge 4dcc1e7b)

- **First staging deploy now waits on PRD-06 alone.** PRD-07 builder step 2 is on integration
  (4dcc1e7b), with `MIN_COMPETITION_VERSION = "1.0.0"` in both `wrangler.toml` blocks. The workspace
  version bump the PRD-07 fixer named as an ordering seam (11:11Z) is also on integration: `Cargo.toml`
  reads `version = "1.0.0"`. So the deploy rule in section 3 holds as written.
- **Staging before PRD-07 step 3 still answers `/receipt` and `/abandon`** and still sends `ack` and
  `last_ack`. No 1.0 client calls or reads them, so a staging deploy between the two slices is safe;
  redeploy after step 3 merges.
- **X6 and every staging fetch pass `--bucket goalcup-traces-staging`.** `fetch-run.sh` picks the
  staging or production side of the index by the bucket name, and with no flag it reads production
  (`goalcup-traces`). Read only, but a staging run fetched without the flag ends with "the index holds
  no entry". PRD-12 section 3 already passes the flag.
- **What `fetch-run.sh` reads from the conductor's environment:** `DATABASE_URL` (psql, in a
  read-only transaction) and `CLOUDFLARE_ACCOUNT_ID` with `CLOUDFLARE_API_TOKEN` (R2 GETs). The
  conductor holds one Cloudflare token for the staging deploy; the script only reads with it, but the
  token's own rights are not narrowed by the script.
- **`--local` keeps only each entry's last heartbeat** (the local store holds `last_vitals` only). X6's
  check of every heartbeat row runs on a staging fetch, never on `--local`.
- **Build snapshot Python is 3.9** (lane logs 10:45Z to 10:53Z): `tomllib` is missing and
  `scripts/check-links.py` needs 3.10. Every Python step the gate or the pad runs in a box must work on
  3.9, or the snapshot gets 3.10 or later. `test_fetch_run.py` runs on 3.9 (17/17).
- **Size report, PRD-07 slice 1** (wc -l on integration at 4dcc1e7b): Worker src 3,848 against the
  "about 3,440" row (11.9% over; the row's src includes the out-of-scope `github.ts`, `mail.ts`,
  `notices.ts`, `ratelimit.ts` and `auth.ts`, as the 4,001 "today" figure did), down from 4,001.
  Worker tests 5,568 against "about 4,950" (12.5% over), down from 5,800. Step 3 still deletes
  `sign.ts` (116), the receipt, abandon and alias code, `sign.test.ts` (68) and `signing.json`
  (26). Scripts: `fetch-run.sh` 369 (was 349) and `test_fetch_run.py` 364 (was 695), 733 of the
  "about 900" shared with the gate, release and packaging. `docs/UPLOAD-API.md` 577 (was 596). By the
  11:20Z decision, overage alone is a nit.

### 2026-10-30, replan after PRD-17 phase 0 slice (merge 1423f8f9)

- **A heavy test binary's merge carries one line in PRD-14's `.config/nextest.toml`.** Group
  `heavy` starts empty (nextest refuses a filter that matches no binary; PRD-14 gate lane, integrator
  log 10:52Z), so the merge that brings `lessons_launch.rs`, a `watchdog_*`, `e2e_*`,
  `pipeline_restart`, `pipeline_stall`, `pipeline_many_streams`, `proxy_connect`, `proxy_restart`
  or `native_walk` binary adds its name to the heavy override. Under merge rule 1 that is a seam
  change: the integrator reads the PRD-14 lane's 10:52Z note as its approval for that one override
  line only; any other edit to the file needs a PRD-14 line in the log. Without it a kill test runs
  in parallel with the others.
- **Old-gate stall, now gone:** the docs-only `LESSONS.md` slice failed `gate fast` from 10:54Z to
  11:35Z on the old `scripts/check-version.py` over plan docs, not on its own file, until PRD-14's gate
  merged. With PRD-14 on integration, a docs-only slice passes (`gate fast: PASS in 59s` on 1423f8f9).
- **Findings raised on a fix after it merged** (A7, B7 on `LESSONS.md`, both nits) go to the owner
  lane's next slice through its Amendments note, not a separate merge request.
- **Size report, PRD-17 phase 0 slice** (`wc -l` on integration at 1423f8f9): 0 lines of PRD-17 code
  against the shared lessons and test-support tests row of 3,000 (PRD-00 "Size budget"); none of that
  row is on integration yet. `LESSONS.md` 197 lines of docs, outside the budget.

### 2026-10-30, replan after PRD-15 slice 1 (merge 98bbb7e9)

- **Size report, PRD-15 slice 1** (wc -l on integration at 98bbb7e9). PRD-15 has no row in PRD-00
  "Size budget"; docs are not counted there. `AGENTS.md` 111 (was 110). `docs/DECISIONS.md` 1,418 (was
  1,162; plus 256 for the header line, "Owner decisions in force", D75 to D86 and the status rows).
  `docs/research/results/2026-10-30-two-hour-run.md` 304 (new, a copy). Review B-1 names about 30 to 40
  DECISIONS lines that can go in the after-merge rewrite. No code.
- **The phase 3 order holds.** PRD-14 deletes `check-links.py` in the phase 3 merge only after
  PRD-15's last link run. That run leaves out the run 1 report by name (its 21 evidence links dangle by
  design, PRD-15 amendment) and runs on Python 3.10 or later in the box. Gate fast does not run
  `check-links.py`, so those links block no merge.
- **PRD-15's stale-term grep fails on `AGENTS.md` until PRD-15 slice 2 lands the map** (review B-4):
  the map and "Read next" still name the old crates, pins, CONTRACTS, TERMINATION and CHEAT-CHECKLIST.
  Read that check after slice 2, not before.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-17 upkeep slice 1 (merge 646737c6)

- **Phase 2 check 1 matches nextest's binary id, not a file path.** Rust guards live under
  `crates/*/tests/` (PRD-01's `recovery.rs` is in `goalcup-format`), and `LESSONS.md`'s next edit
  writes `file::test` without `.rs` (PRD-17 note). The check splits each entry on `::` and matches
  binary id suffix and test name in `cargo nextest list --workspace --message-format oneline`.
- **A green row's Commit is the commit the box proved** when the guarded crate changed after the
  test's merge; otherwise the merge that brought the test (review A-3).
- **Docs-only slices still pass the gate in about 60 s** (`gate fast: PASS in 60s` on 646737c6).
- **Size report, PRD-17 upkeep slice 1** (`wc -l` on integration at 646737c6): 0 lines of PRD-17
  code against the shared lessons and test-support tests row of 3,000 (PRD-00 "Size budget").
  `LESSONS.md` 197 lines of docs, outside the budget; 3 rows green (L-FW2, L-FW5, L-FW8 at
  `f5b0a6a2`), 6 rows still `(name pending)`.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-15 slice 2 (merge 21c0fd75)

- **Size report, PRD-15 slice 2** (`wc -l` on integration at 21c0fd75). PRD-15 has no row in PRD-00
  "Size budget"; docs are not counted there, and the slice has no code outside the doc. New
  `docs/RECORD-FORMAT.md` 324 lines, of which the stdlib Python reader is 51 (PRD-01 allows 60);
  PRD-15's own estimate for the page is about 280, and two sections are still to come (PRD-15
  amendment, review B5). `AGENTS.md` 111 (unchanged count, one sentence reworded). `docs/DECISIONS.md`
  1,416 (was 1,418).
- **Docs-only slices still pass the gate in about a minute** (`gate fast: PASS in 77s` on 21c0fd75).
- **Counting rule for the night's checks:** a count made by piping part files through the `zstd`
  command line tool is a lower bound on a live entry or one not yet recovered (review A, lane log
  11:56Z). Final acceptance counts on a live or crashed entry come from `goalcup-verify dump` or
  `check`, never from `zstd -dc` plus the reader. Sealed and recovered entries agree either way (box
  run 12:02Z: the reader matched the Rust dump on 11 format-crate entries).
- **Build snapshot Python 3.9 holds for the reader** (box runs 11:49Z, 12:02Z). `check-links.py`
  still needs 3.10 or later via `uv` in the box (earlier note).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-17 upkeep slice 2 (merge ae970618)

- **Phase 2 check 1 reads exact titles for file-level Worker guards** (PRD-17 note): a bare file name
  alone would pass after the guarded case left the file. The check matches the night-log title of
  L-S1, L-S2, L-S3 and L-S10 in the named file under `server/worker/test/`.
- **A lane merger that finds nothing new to join pushes nothing** (lessons log 11:58Z): no empty
  merge commit; the merge request goes on from the lane head as it is.
- **Box-only held, with one lapse logged** (lessons log 11:57Z): a docs structure script was once
  started on the conductor machine, its output discarded; the recorded proof is the box run. No
  rule change; scripts that check docs run in a box too.
- **Docs-only slices still pass the gate in about 60 s** (`gate fast: PASS in 59s` on ae970618).
- **Size report, PRD-17 upkeep slice 2** (`wc -l` on integration at ae970618): 0 lines of PRD-17
  code against the shared lessons and test-support tests row of 3,000 (PRD-00 "Size budget").
  `LESSONS.md` 203 lines of docs (was 197), outside the budget; 6 rows green, 6 rows still
  `(name pending)`.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-07 slice 2 (merge 2f4bc6a0)

- **Size report, PRD-07 slice 2** (`wc -l` on integration at 2f4bc6a0): Worker src 3,557 against the
  "about 3,440" row (3.4% over; out-of-scope files included as before), down from 3,848. Worker tests
  5,258 against "about 4,950" (6.2% over), down from 5,568. Scripts: `fetch-run.sh` 375 and
  `test_fetch_run.py` 366, 741 of the "about 900" shared with the gate, release and packaging.
  `server/worker/README.md` 562 lines of docs (absorbs SERVER-DESIGN), outside the budget.
- **Redeploy staging after this merge** (the slice 1 note): from 2f4bc6a0 on, staging answers
  `/receipt` and `/abandon` with `404 not_found` and sends no `ack` or `last_ack`.
- **PRD-13 Worker guard ran at PRD-07's cut merge**: clean in `src/`; one false match, the
  `wrangler.toml:83` comment about deleting `RECEIPT_SIGNING_KEY` after production, narrowed by the
  PRD-13 note to non-comment lines. The secret deletion stays an owner step after production; nothing
  in this build deletes it.
- **`--local` rerun against `wrangler dev` 4.149.0 passed** (lane log 11:49Z): three chunks plus the
  final heartbeat, `rec` equal to chunks, `complete` true, one heartbeat row.
- **Gate on PRD-07 slice 2:** `gate fast: PASS in 82s`; `test_fetch_run.py` still outside the gate
  (PRD-14 note).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-12 slice 2 (merge c7649890)

- **Merge bookkeeping.** c7649890, titled "PRD-12 slice 1", holds both PRD-12 slices (branch head
  62bd0ed3, slice 2 fixes included). The integrator log's 12:17Z line names 15a6963b as the slice 2
  merge, but 15a6963b is the PRD-07 replan docs commit: the branch was already in, and the merge step
  logged HEAD after a merge with nothing to do. The night log cites c7649890 for both slices. The
  merge step should say "already merged at <sha>" instead of logging HEAD as a merge.
- **Ladder rung 1 (PRD-12 P2) still waits.** The rig walks a no-key `micro` run with `server = "off"`
  in a real pad box (setup, start, wait-calls, look, seal, cleanup; box removed, no known file left),
  but no `goalcup` build is on integration, so `check` was skipped and no row was judged. P2's C3 PASS
  also waits on PRD-08's `secrets --json` `masks` key and on PRD-05's masker (PRD-08 and PRD-12 notes).
- **Open seams with PRD-08** (PRD-08 note): the `secrets --json` keys, the gap cause and length in
  `check --json`, `keep` in S6, and the stale C1 clause in PRD-08's PRD-12 interfaces row (PRD-12 and
  ledger T4 win: a history `skip` passes C1).
- **Seam keys read by the pad.** The `SEAM` table at the top of `scripts/pad/check.py` lists every key
  it reads from S4 (`mode`, `port`, `harness.pid`, `written.model_requests`, `written.off_ms`,
  `seal.reason`), S6, S7 and S13. A seam owner who renames one posts a seam line; only the pad lane
  edits `scripts/pad/`.
- **Size report, PRD-12** (`wc -l` on integration at c7649890): `scripts/pad/` is 1,771 lines
  (`pad` 419, `pad-run` 373, `check.py` 352, `test_pad.py` 214, `sbx.mjs` 133, `table` 84, `image` 54,
  `pad.toml` 17, harness files 92, prompts and settings 34), plus the generated `connectors/tiers.toml`
  (81). PRD-00's row is "about 1,350 to 1,450 with its tests" and PRD-12's own estimate about 1,400:
  22% over the top of the row with `selftest` (100), `exec/` (70), 13 harness files (about 195), the
  skill (100) and the second and third slice steps still to come, so at least about 2,140 if those land
  at their rows. Per the integrator's 11:20Z decision, overage on an owner estimate is reported, not
  held. The extra lines so far serve point 3 (every pad step has its own wait and limit, and a timed-out
  step never stops the rig), point 4 (known values never leave the box: plants, pane replacement and
  the output scan run in `pad-run`) and point 7 (`sbx.mjs` drives real Vercel Sandbox boxes, the way
  the rig is run). The next PRD-12 slice applies the three cuts in the PRD-12 note before adding lines.
  The end-of-build report carries the final figure for the owner.
- `gate fast` PASS in 76 s on c7649890 (and 75 s on the no-op second run). `test_pad.py` is not in
  the gate yet (PRD-14 note).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-15 slice 3 (merge 4e0ca793)

- **Size report, PRD-15 slice 3** (`wc -l` on integration at 4e0ca793). PRD-15 has no row in PRD-00
  "Size budget" (docs are not counted), and the slice has no code. `TODO.md` 59 lines (was 350;
  PRD-15 asks for under 60), `README.md` 318 (unchanged count, one link repointed).
- **Docs-only slices still pass the gate in under two minutes** (`gate fast: PASS in 98s` on
  4e0ca793).
- **`check-links.py` is not in the gate**, so a broken link merges silently. The PRD-11 box run
  (harnesses log 12:22Z) counted 33 listed lines with the new TODO overlaid, 30 after its fix. The
  final acceptance link check filters the history files by name (run 1's report,
  `docs/RESEARCH-TARGET.md`, `docs/research/**`) and must list nothing else for live docs.
- **After-merge purge order** (review A7, PRD-15 note): live docs stop linking purged history before
  the purge, and the purge change runs `check-links.py` itself. The TODO keep list stays at four
  files.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-07 slice 3 (merge 323b1a0f)

- **PRD-07 is done on integration.** Slice 3, the last, gave `delete-test-runs.sh` the two usage
  prefixes (S7 shape, built before PRD-09's job) and lists R2 before the database changes. No Worker
  src or test changed. `gate fast: PASS in 83s` on 323b1a0f. The fixer's 10 stubbed script cases ran
  once in a build box and are not in the gate; the script itself is never run in this build (owner
  tool, PRD-07 section 9), so acceptance cites the stubbed run and says the real-bucket run was not
  done.
- **Three nits are open** (RA-N2, B2-N1, B2-N2; PRD-07 note) and one hand-off to PRD-15 (RA-N1, the
  OPERATING test-runs lines; PRD-15 note). None blocks. If no later PRD-07 slice comes, the end-of-build
  report lists them as known.
- **Size report, PRD-07** (`wc -l` at 323b1a0f, against PRD-00 "Size budget", row "Worker (PRD-07)"):
  src (`server/worker/src`, all files) 3,557 against about 3,440, 3.4% over; tests (`server/worker/test`
  `*.ts` plus helpers) 5,258 against about 4,950, 6.2% over. Both within the 10% the row allows
  without a replan decision. Down from 4,001 and 5,775 before the build. Outside the row:
  `server/worker/scripts/delete-test-runs.sh` 122 (was 95) and `scripts/fetch-run.sh`, in the scripts
  row.
- **Seam to watch at PRD-09's job merge:** S7's key layout and the bucket-per-side rule are what the
  cleanup script relies on (PRD-09 note). The reviewer of that merge checks it.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-15 slice 4 (merge 7be8bf98)

- **Size report, PRD-15 slice 4** (`wc -l` on integration at 7be8bf98). PRD-15 has no row in PRD-00
  "Size budget" (docs are not counted), and the slice has no code. `docs/ARCHITECTURE.md` 349 (was
  350; target about 160 after its rewrite), `docs/README.md` 82 (was 83), `docs/OPERATING.md` 863 and
  `README.md` 318 (unchanged counts, one link each), `docs/archive/SERVER-DESIGN.md` 164 (moved,
  unchanged).
- **Docs-only slices still pass the gate in under two minutes** (`gate fast: PASS in 100s` on
  7be8bf98).
- **The final link check filters moved archive files by name** (PRD-15 note): 8 of the 40 broken
  links at bdcea297 are in `docs/archive/SERVER-DESIGN.md`. `check-links.py` is still not in the
  gate; acceptance cites the last box run and its filter list.
- **A merge-order point for the ARCHITECTURE rewrite** (review A-N3): its reviewer checks that no
  line says the server signs receipts or acknowledgements (lines 31, 276 and 277 today). If the
  third slice is not reached, the end-of-build report lists `SECURITY.md:69` as a known false line.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-07 slice 4 (merge c4ada44a)

- **PRD-07 is complete on integration.** Every builder step (4dcc1e7b, 2f4bc6a0, 323b1a0f) and the
  nit cleanup (c4ada44a) are merged. Two optional nits stay open with no slice planned: the script's
  `--help` header lacks the RA-N2 caveat, and a 2-line simplification of its Python parse (PRD-07
  note). Acceptance lists them as known, not as failures.
- **Size report, PRD-07** (`wc -l` on integration at c4ada44a), row "Worker (PRD-07)": src
  (`server/worker/src`, all files) 3,557 against about 3,440, 3.4% over; tests (`server/worker/test`
  `*.ts` plus helpers) 5,258 against about 4,950, 6.2% over. Both unchanged by this slice and within
  the 10% the row allows without a replan decision. Outside the row:
  `server/worker/scripts/delete-test-runs.sh` 121 (was 122).
- **Checks skipped in boxes, for acceptance to state as skipped:** `shellcheck` and `dash -n` (not in
  the box image) and `check-links.py` (box Python 3.9 lacks the 3.10 syntax it uses). The script's
  stubbed cases are not in the gate.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-05 phase 0 slice (merge 82484521)

- **Size report, PRD-05** (`wc -l` at 82484521, against PRD-00 "Size budget", row "Masking (PRD-05)",
  3,530 src and 2,000 tests): src 247 (`src/mask/mod.rs` without its tests), 7% of the row;
  `shapes.tsv` 32 is data, not counted; tests 367 (179 inline in `mod.rs`, 188 in
  `tests/mask_rules.rs`), 18% of the row. The corpus (86 files, 6,216,905 bytes) is data. The stub
  `mod.rs` is over PRD-05's own file estimate (247 against 160 for the final `mod.rs`); the core
  slice replaces the stub, and reviews B1 to B3 cut about 111 lines (PRD-05 note).
- `gate fast: PASS in 78s` on 82484521. The breaker's throwaway stress (40 and 200 streams, 1.5 MB
  bodies, WebSocket boundaries) lost no byte and released no raw byte, peak RSS 93 MB; it is not in
  the gate.
- **The no-real-key check works on the stub:** `mask_rules()` is `["fail_closed"]`, length 1, so the
  "`mask_rules` length above 1 in the `start` event" check tells a stub build from a core build, as
  designed. Any pad or acceptance row on a stub build is not evidence (PRD-12 note).
- **Next masking dependency:** S3 (PRD-11) on integration, for the concrete `Masker::new` type and
  the connector rules `meta.rs` reads. The core slice builds against S3's PRD-00 final shape meanwhile.
  PRD-13 may delete `spikes/` as far as PRD-05 is concerned (PRD-13 note).
- **Trailers:** merge commit df84011c (`v1.0/prd-05-masking-s12-shapes`) has no
  `Co-Authored-By` and `Claude-Session` lines. It stays (no force-push, no rebase) and the
  end-of-build report lists it. Mergers and builders add both lines to every merge commit, an
  integration-into-task merge included.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-10 slice 1 (merge d55030ee)

- `gate fast: PASS in 76s` on d55030ee. Before the merge, the merged-with-integration tree was green
  in a box: `fmt`, `clippy -D warnings`, `nextest` on `goalcup` 26 of 26, doctests (two
  `compile_fail`).
- **S11 is on integration** (due H+1.5), with a seventh `Fact` variant, `Io(io::ErrorKind)`, and a
  checked `cli::log::Host` (PRD-10 note). PRD-02, PRD-03 and PRD-06 build against it now.
- **Phase 0 did not include `start` with `GOALCUP_SERVER=off` against stubs** (PRD-10 builder notes
  step 1): `watchdog::run` (S8) and `connector::load` and `plan` (S3) are on no branch. It moves to
  PRD-10's core slice, which PRD-00's core slice row already lists, so the build order is unchanged.
  Anything that runs `goalcup start` (the pad's first `server = "off"` micro rung, PRD-14's e2e test)
  waits for S3 and the watchdog's `run`, then PRD-10 core. Every command is a stub that exits 3 until
  then; a pad or acceptance row against this build is not evidence.
- **Size report, PRD-10 slice 1** (`wc -l` at d55030ee, row "CLI (PRD-10)", about 2,900 src and 1,200
  tests): src 596 (`main.rs` 190, `cli/mod.rs` 152, `cli/log.rs` 202, `cli/text.rs` 52, inline
  test modules left out), 21% of the row; tests 308 (`tests/cli_commands.rs` 128, `tests/cli_log.rs`
  135, 45 inline in `main.rs` and `cli/mod.rs`), 26% of the row. Three files are over PRD-10's own
  file estimates (`main.rs` 190 against 160, `cli/mod.rs` 152 against 130, `cli/log.rs` 202 against
  140 shared with `skill.rs`); the row is the ceiling, and PRD-10's note names where to cut.
- **Open nits** A8, B13 and B14 go to PRD-10's next touch (PRD-10 note); acceptance lists them as
  known if they are still open at the end.
- **Trailers:** merge commits aab5cec4 and 2486380a on `v1.0/prd-10-cli` have no `Co-Authored-By` and
  `Claude-Session` lines. They stay (no force-push, no rebase) and the end-of-build report lists them
  beside df84011c.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-04 core slice part 1 (merge 403d9165)

- `gate fast: PASS in 94s` on 403d9165. Before the merge the lane was green in a box at 4fd26f3f:
  `fmt`, `clippy -D warnings`, `native_pattern` and `native_safety` 13 of 13. Linux only: no native
  test ran on macOS; reviewer A reported a macOS stub check passing in a box, which is not a macOS
  test run.
- **Size report, PRD-04** (`wc -l` at 403d9165, row "Native (PRD-04)", 780 src and 1,000 tests): src
  401 (`fsx.rs` 226, `pattern.rs` 167, `mod.rs` 8), 51% of the row from two of six files; tests 246
  (`tests/native_pattern.rs` 123, `tests/native_safety.rs` 123), 25% of the row. `fsx.rs` is 226
  against its 120 estimate and `pattern.rs` 167 against 100. Built to PRD-04's estimates the remaining
  files would put the area near 950, about 22% over the row; PRD-04's note sets the next slice to the
  379 src lines left, with review B9 and B8 as the first cuts.
- **Seam:** `NativeReader::finish` takes `guard: Option<Duration>` (PRD-00, PRD-02 and PRD-04 notes).
  The build order is unchanged: PRD-04's next slice (`start`, `finish`, walk, tail, snapshot, loop)
  waits for PRD-11's S3 stub.
- **Open nits** A8, B8, B9 and B10 go to PRD-04's next slice (PRD-04 note); acceptance lists them as
  known if they are still open at the end.
- Merge commits on `v1.0/prd-04-native` and 403d9165 carry both trailer lines.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-12 slice 3 (merge 9e259b4d)

- `gate fast: PASS in 82s` on 9e259b4d. The gate does not run `test_pad.py` (PRD-14 note); the 17 pad
  tests were green in a build box at 9e9e384a only. The merge and its 10 branch commits carry both
  trailer lines.
- **What slice 3 adds:** ladder rung 2 wiring with `server = "staging"` (headless approval, the
  approval key and member id read in the process from env vars named in `pad.toml`), wait-upload on
  S9, fetch-run into `fetched/<entry>/` with the copy-in, the section 8 server commands in `pad-run
  verify`, and `table` row mode with `--release-check`. No staging run has been made with it: rung 2
  waits for rung 1 (a no-key `micro` with `server = "off"`), which waits for PRD-10 core and a non-stub
  masker (PRD-05 note). A pad row on a stub build is not evidence.
- **P7 and `--release-check`.** Until PRD-12's nit B3 lands, `table --release-check <integration
  head>` reads the rows at the checkout's `HEAD`; run it from a checkout at that commit (PRD-12 and
  PRD-14 notes).
- **Size report, PRD-12** (`wc -l` at 9e259b4d, fixtures left out; PRD-00 row "about 1,350 to 1,450
  with its tests"): `scripts/pad/` is 2,077 lines (`pad` 500 of 320, `pad-run` 411 of 330, `check.py`
  358 of 320, `table` 192 of 150, `test_pad.py` 287 of 90, `sbx.mjs` 133 with no row, the rest 197),
  306 more than at c7649890 and 43% over the top of the row; the files still to come at their rows
  make about 2,440. The replan decision line (PRD-12 note) names points 3, 4 and 5 for the extra lines,
  does not raise the row, and has the next slice cut first (review nits B1 and B2, then `test_pad.py`
  toward its row). The end-of-build report lists PRD-12's size against its row; whether the row rises
  is the owner question already open in the pad log (12:53Z).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-11 slice 2 (merge b9fa50f8)

- `gate fast: PASS in 94s` on b9fa50f8. Before the merge the lane was green in build boxes: at
  2ed4bb82 `fmt`, build, `clippy -D warnings` and 15 of 15 connector tests; after the review fixes at
  111db2ed `fmt`, `clippy` and the 15 connector tests. Reviewers A and B each ran fmt, clippy and the
  connector tests green in a box. Linux only.
- **S3 is on integration as a skeleton** (PRD-11 note lists the stubs). Unblocked by its shape: PRD-02
  `watchdog::run`, PRD-03 slice 2, PRD-04's next slice, PRD-05's `MaskRule` switch, PRD-10's
  `From<Host>`. Real behaviour (host decisions, hints, lock, version, settings) comes with PRD-11's next
  slice, three parallel tasks (`hosts.rs`, `hints.rs`, `launch.rs`, one test each). The core slice's
  "launch plans for Claude Code and Codex", the 1a micro rungs and PRD-17's `lessons_hosts.rs` and
  `lessons_launch.rs` wait for it; a run before it tunnels every host and is not evidence.
- **Size report, PRD-11** (`wc -l` at b9fa50f8, row "Connector code (PRD-11)", 800 src and 400
  tests): src 1,149 (`mod.rs` 290, `schema.rs` 446, `hosts.rs` 229, `launch.rs` 136, `builtin.rs` 27,
  `hints.rs` 21), 144% of the row with three files still stubs; tests 384 (`connector_builtin.rs` 195,
  `connector_launch.rs` 147, `connector_no_model_address.rs` 42), 96% of the row. The merge passed the
  row by more than 10% without a decision line; the replan's decision (PRD-11 note) sets the ceiling to
  1,510 src and 620 tests with per-file ceilings and named cuts. That is 710 src and 220 tests over the
  row, on top of PRD-04's projected 22% and PRD-10's files over their estimates: the end-of-build report
  gives the Rust totals per row against about 18,100 src and at most about 14,200 tests.
- **Open nits** F5b, F1b, B12 and B13 go to PRD-11's next slice (launch task); acceptance lists them as
  known if they are still open at the end. B7 (one pattern check) is in the same slice.
- All commits in d8922216..b9fa50f8 carry both trailer lines.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-14 slice 1 (merge 22fe5926)

- `gate fast: PASS in 87s` on 22fe5926; it now runs 44 named script tests (`test_install.py`,
  `test_fetch_run.py`) and the 17 pad tests on python3.11, so the PRD-12 slice 3 note "the gate does
  not run `test_pad.py`" is closed. The merge and its branch commits carry both trailer lines.
- **CI.** `ci.yml` runs `gate fast` on pushes to `v1.0/integration`. Every run so far was cancelled by
  the next push (`cancel-in-progress: true` on one group per ref; the replan pushes docs-only commits
  within about two minutes of each merge): run 41060500797 for 22fe5926 was cancelled at 1m35s. The
  phase 3 exit needs "CI green, G1 and G4 in the log"; a cancelled run is neither. PRD-14's next CI
  edit cancels only pull request runs (PRD-14 note); a watcher counts only `completed success` runs.
  The "Today" row on `release.sh` ("the next-step text names `linux-check.sh`") is no longer true.
- **For every lane that writes tests in `crates/goalcup/tests/`:** declare the shared support as
  `#[allow(unused)] mod common;` (as `rule_code.rs` does), or `clippy -D warnings` fails on helpers the
  binary does not call. `common::upstream::Upstream::start` and `common::worker::FakeWorker::start` are
  still `todo!` until PRD-14 slice 2 (by H+3); a test that needs them before then keeps a private
  helper, removed at phase 3. `rule_code` now also bans connector `program` names in core string
  literals (PRD-14 and PRD-11 notes).
- **Size report, PRD-14** (`wc -l` at 22fe5926; PRD-00 row "Lessons (PRD-17), test support,
  `rule_code`, `e2e_run` (PRD-14)", 3,000 tests, of which PRD-14 section 3 gives PRD-14 1,700):
  `tests/common/` 740 of 1,480, `rule_code.rs` 213 of 80, `e2e_run.rs` 0 of 140, in all 953 of 1,700.
  `rule_code.rs` is past its line by more than 10%; the replan decision (PRD-14 note, and the
  test-support log) names point 7 and keeps the PRD-14 total within 1,870 without raising the row.
  Scripts (PRD-00 row "gate, release, packaging, fetch-run, about 900"): `gate.sh` 121, `ci.yml` 57,
  `release.sh` 111, with 384 lines deleted in the slice.
- **Open nit** B8 (`release.sh` next-step text written three times) goes to PRD-14's release slice
  (PRD-14 note); acceptance lists it as known if it is still open at the end.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-15 slice 5 (merge ea0a0fec)

- `gate fast: PASS in 83s` on ea0a0fec. Docs only: `README.md`, `docs/ARCHITECTURE.md`,
  `docs/CONNECTOR-SCHEMA.md`, `docs/HARNESS-ONBOARDING.md`, `docs/README.md` (+117, -1005). Box link
  check at c24908e4: no broken link from CONNECTOR-SCHEMA; 39 known others, of which one is new to
  this slice (`docs/CONTRACTS.md:615` to `#pin-evolution`, left for the CONTRACTS move). Lane box
  runs: em dash grep clean, stale-term grep clean on the page, key set equal both ways with the
  schema (22 keys with `mask`).
- **Size report, PRD-15.** PRD-15 merged no code, and PRD-00 "Size budget" has no docs row, so there
  is nothing to count against a row. Against PRD-15's own docs table ("CONNECTOR-SCHEMA 1,002 to 90"):
  the page is 113 lines (126%), the extra being the key table, the mask grammar and the eight
  **(next slice)** marks, which go when PRD-11's next slice merges.
- **Trailers.** Lane merge commit 16b62b8c ("Merge remote-tracking branch
  'origin/v1.0/prd-15-docs-connector-schema'") carries neither trailer line; the other six commits in
  3d56fffc..ea0a0fec carry both. It is pushed and is not rewritten; the end-of-build report lists it.
  Lane mergers write the two lines on merge commits too.
- **Open nits** A10 (`continue` file-flag rule) and B12 (two links to a page without the fact) go to
  PRD-15's next slice (PRD-15 note); acceptance lists them as known if they are still open at the end.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-08 slice 1 (merge d10feef7)

- `gate fast: PASS in 119s` on d10feef7 (83 tests, 16 of them `goalcup-verify`'s); the extra 32 to
  36 s against earlier merges is the crate's first compile with bundled SQLite. Box runs before the
  merge: fmt, clippy `-D warnings` and the crate's tests green at each fix round (judge 13:09Z to
  13:28Z). Workspace membership was added by the merge requester (94d204f1), as planned.
- **Size report, PRD-08** (`wc -l` at d10feef7, row "Verifier (PRD-08)", 1,400 src and 1,100 tests):
  src 1,635 (`integrity.rs` 1,089, `main.rs` 209, `input.rs` 192, `report.rs` 122, `usage/mod.rs`
  12 placeholder, `lib.rs` 11; `Cargo.toml` 27 not counted), 117% of the row with four of ten source
  files written; tests 1,471 (`tests/common/mod.rs` 907, `check_invariants.rs` 346,
  `check_named.rs` 218), 134% with three of seven files. The merge passed the row by more than 10%;
  the fixer's line (judge 13:09Z) named points 5 and 6, and the replan's decision (PRD-08 note) sets
  the ceiling to 2,400 src and 1,700 tests with per-file ceilings and named cuts. That is about 1,000
  src and 600 tests over the row, on top of PRD-11's 710 and 220 and PRD-04's projected 22%: the
  end-of-build report gives the Rust totals per row against about 18,100 src and at most about 14,200
  tests, and a test total past 14,200 is an owner question then.
- **Trailers.** Lane merge commit a5a4bf18 ("Merge remote-tracking branch 'origin/v1.0/integration'
  into v1.0/prd-08-verify") carries neither trailer line (review F10, declined because the branch was
  pushed); the other 15 commits in ea0a0fec..d10feef7 carry both. The end-of-build report lists it.
- **Open nit** B13 (test builder duplicate write path) goes to PRD-08's next slice (PRD-08 note);
  acceptance lists it as known if it is still open at the end.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-10 slice 2 (merge 56495dd6)

- `gate fast: PASS in 88s` on 56495dd6 (98 tests run in the nextest summary). Box runs before the
  merge: the builder ran fmt, clippy `-D warnings` and goalcup nextest (41/41) plus gate fast. The
  fixer ran goalcup nextest (69/69), and reviewer A re-checked on a local merge with 77e01fbb (fmt,
  workspace clippy, 69/69, doctests). The macOS liveness path has not been compiled in any box
  (skipped, no macOS box). Ten paths changed, all owned by PRD-10, with no `Cargo.toml` change.
- **Size report, PRD-10** (`wc -l` at 56495dd6, row "CLI (PRD-10)", about 2,900 src and 1,200 tests).
  Src is 1,959, 68% of the row with 6 of 18 files written: `text.rs` 613, `cli/mod.rs` 462,
  `status.rs` 341, `log.rs` 202, `main.rs` 197, `finish.rs` 144. The same six files have own
  estimates of 1,270, so they run at 154%, and the CLI projects to about 3,680 (127%). The replan
  decision (PRD-10 note) keeps the row, makes later slices build at their estimates, cuts B1, B3 and
  B4 first, and needs a decision line for any merge past 3,190. Tests are 965, 80% of the row with
  5 of the planned files (`cli_status.rs` 404, `cli_finish.rs` 274, `cli_log.rs` 136,
  `cli_commands.rs` 128, `tests/cli_common/mod.rs` 23). `cli_folder.rs`, `cli_start.rs`,
  `cli_continue.rs` and the setup tests are still to come, so the tests row will likely pass 1,200.
  They reuse `tests/cli_common/` and PRD-14's `common::home` and `common::worker`. The
  end-of-build report counts it against the Rust test total (a total past about 14,200 is an owner
  question then).
- **Trailers:** all 10 commits in 77e01fbb..56495dd6 carry both lines.
- **Ownership:** the integrator accepts `crates/goalcup/tests/cli_common/` as PRD-10's (PRD-00 note).
- **Open nits** B1 to B4 go to PRD-10's next slice (PRD-10 note). Acceptance lists them as known if
  they are still open at the end. The vendor problem row, end-screen rule 7's end time, the native
  rows line and the upload lines are not built yet. Acceptance does not count the section 4 problem
  table or the end screen as complete until they are built or a replan line rules them out.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-09 slice 1 (merge f5a529c8)

- `gate fast: PASS in 143s` on f5a529c8 (98 tests run, 98 passed). Box runs before the merge: the
  lane merger ran build, clippy `-D warnings`, fmt and 16/16 nextest; the fixer ran fmt, clippy
  `-D warnings`, doc `-D warnings`, 16/16 nextest, the fixture sha256 check against the old crate and
  a secret-pattern scan (0 hits) on ad1ce7e6; reviewer A's last re-check did not rerun the box (same
  sha). Ten paths changed, all owned by PRD-09 (`src/usage/mod.rs` and `tests/usage_fixtures/**`),
  with no `Cargo.toml` change.
- **Size report, PRD-09** (`wc -l` at f5a529c8, row "Usage reader (PRD-09)", 1,470 src and 200 tests).
  Src is 304, 21% of the row with 1 of 8 files written (`usage/mod.rs`, a stub: 89 doc-comment lines,
  no reader logic). Its own estimate is 250 for the whole `mod.rs` with `run`, the sink, the join,
  totals and output, so the file already runs at 122% of it on types alone; the final U1 types took
  the extra (PRD-09 note). The row stands; the core slice reports per file and needs a decision line
  past 1,617. Tests are 0 of 200 (`usage_run1.rs` is the core slice's). The fixtures are 555 lines of
  data, counted in neither row.
- **Trailers:** all 8 commits in ab939dd1..ad1ce7e6 carry both lines, and so does merge f5a529c8.
- **Seams:** U1 is on integration with its final names, so PRD-08's `legs.rs` and `history.rs` and
  its run 1 shape builder can build now (PRD-08 note). PRD-13's delete of the old verifier crate no
  longer waits on PRD-09's fixtures; the old `connectors` fixtures are not copied (PRD-13 note).
- **Open nit** B10 (one fact in `readable` plus `reason`) is taken in PRD-09's core slice unless
  PRD-08 reads the field first (PRD-09 note). Acceptance lists it as known if it is still open at the
  end.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-15 slice 6 (merge 9e89a22d)

- `gate fast: PASS in 86s` on 9e89a22d. Docs only: `docs/ARCHITECTURE.md`, `docs/CONNECTOR-SCHEMA.md`,
  `docs/HARNESS-ONBOARDING.md`, `docs/RECORD-FORMAT.md` (+69, -19), all PRD-15 owned. Box runs before
  the merge (lane 14:35Z to 14:48Z): `goalcup-verify dump` on the `zstd -dc` copy of the worked example
  equals the committed dump; `check --json` top-level and entry key sets, check ids, `inv` numbers and
  the skip list equal the page; em dash grep clean with a control hit; stale-term grep clean on the
  merged pages (old hits remain in ARCHITECTURE and HARNESS-ONBOARDING from integration, for their
  rewrites); link check: no new break (`docs/CONTRACTS.md` to `#pin-evolution` was already known).
  The reader's counts equal the dump per kind (box at 010c69cb); the fixer's box at 9e094019 did not
  rerun the reader.
- **Size report, PRD-15.** PRD-15 merged no code, and PRD-00 "Size budget" has no docs row, so
  `wc -l` of PRD-15 code is 0 and there is no row to count against. Against PRD-15's own docs table:
  `docs/RECORD-FORMAT.md` is 374 lines against about 280 (134%, the builder's decision line names the
  record tables, worked example, reader and S6 key tables), `docs/CONNECTOR-SCHEMA.md` 115 against
  about 90 (128%, the key table and the **(next slice)** marks).
- **Trailers:** all 11 commits in f5a529c8..9e89a22d carry both lines, the lane merge commits
  included.
- **Open nit** B-N4 (the reader intro's bare `zstd` pointer) goes to PRD-15's next slice that touches
  RECORD-FORMAT (PRD-15 note). Acceptance lists it as known if it is still open at the end. A10 and
  B12 are closed.
- **Doc follows code:** the `check --json` section and the `continue` mark depend on later PRD-08 and
  PRD-10 merges naming their changes (PRD-08 and PRD-10 notes). Acceptance checks the page against a
  final `check --json` run, not against this slice's skip list.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-15 slice 7 (merge 938f6733)

- `gate fast: PASS in 86s` on the lane branch at the merged head (gate log 15:09Z). Four paths changed,
  all PRD-15 owned (`AGENTS.md`, `README.md`, `docs/PARTICIPANT-GUIDE.md`, `docs/README.md`; +2, -8),
  no `Cargo.toml` change. Builder box (15:01Z): no `goal-cup-setup` link outside plans, archive and
  the skill; no U+2014 in the four files (control hit seen); `check-links.py` 39 broken at base and
  head, the same set, none from the four files.
- **Size report, PRD-15.** PRD-15 merged no code: `wc -l` of PRD-15 code is 0, and PRD-00 "Size
  budget" has no docs row to count it against. The slice made the docs 6 lines shorter.
- **Trailers:** all 3 commits in 04886295..938f6733 carry both lines.
- **Unblocked:** PRD-10's delete of `skills/goal-cup-setup/` (PRD-10 note). Link checks are not in
  the gate or CI on integration, so a broken link gates nothing; PRD-15's last `check-links.py` run is
  the only one, before PRD-14 deletes the script in the phase 3 merge.
- **Acceptance lists as known** the two broken links left in history docs on purpose:
  `docs/archive/PARTICIPANT-GUIDE-0.6.md:13` (the deleted skill, review A-N1) and `docs/CONTRACTS.md`
  to `#pin-evolution` (moves to the archive in PRD-15's third slice).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-14 slice 2 (merge 04886295)

- `gate fast: PASS in 122s` on the lane branch at the merged head (gate log 15:06Z). Six paths
  changed, all PRD-14 owned (`CODEOWNERS`, `ci.yml`, `tests/common/{mod,upstream,worker}.rs`,
  `release.sh`), no `Cargo.toml` change. `shellcheck` was skipped (not installable in the box).
- **Size report, PRD-14** (`wc -l` at 04886295, PRD-00 row "Lessons (PRD-17), test support,
  `rule_code`, `e2e_run` (PRD-14)", 0 src and 3,000 tests, of which PRD-14's line is 1,700):
  PRD-14 tests are 1,657 (`tests/common/` 1,444, `rule_code.rs` 213, `e2e_run.rs` 0), 97% of its
  1,700 with HTTP/2, `recorder::real` and `e2e_run.rs` still to come; the slice 3 projection is about
  1,870, the line plus 10% (PRD-14 note, with the decision on `worker.rs` at 334 of 250). Src 0.
  Scripts and CI: `release.sh` 121, `ci.yml` 60, `gate.sh` 121, `CODEOWNERS` 19 (was 57).
- **CI as the independent record:** from 04886295 each push to integration has its own group and runs
  to the end (PRD-14 note). Phase 3's "CI green" is read from a completed run on the phase 3 head,
  never from a cancelled one; runs before 04886295 may show `cancelled` and prove nothing.
- **Trailers:** all 14 commits in 900a24d7..04886295 carry both lines.
- **Seams:** `common::upstream` (HTTP/1.1, WebSocket) unblocks PRD-17's `lessons_decode` now (PRD-03
  leaf modules in 0e411074) and `lessons_http1` once `proxy::serve` lands; `common::worker` unblocks PRD-06's failing-PUT backoff case;
  PRD-04's `Calls` helpers and `recorder::real` go to PRD-14 slice 3 (PRD-04, PRD-06, PRD-17 notes).
  Release order needs nothing new: `release.sh tag` runs the pad's release check itself (PRD-12 note).
- **Housekeeping for the end:** remove `v1.0/prd-14-tests-lane-merge-wip` from origin.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-03 core slice 1 (merge 0e411074)

- `gate fast: PASS in 88s` on merge 0e411074 (gate log
  `/root/gc-night/gate-logs/v1.0_prd-03-proxy-1511.log`). Last fixer box at the merged head
  92c47bf6: fmt, workspace clippy `-D warnings`, goalcup doc `-D warnings`, goalcup nextest 31/31
  green. Paths are all PRD-03 owned (`crates/goalcup/src/proxy/`, `tests/proxy_*.rs`,
  `tests/proxy_fixtures/`); no `Cargo.toml` change.
- **Size report, PRD-03** (`wc -l` at 0e411074, row "Proxy (PRD-03)": 4,250 src, 2,000 tests). Src
  2,873: `decode.rs` 810, `ws.rs` 735, `connect.rs` 673, `ca.rs` 439, `tls.rs` 203, `mod.rs` 13;
  68% of the row after one of three slices, +19% on these files' own 2,410 estimate. Tests 1,029
  (`proxy_ws.rs` 427, `proxy_connect.rs` 244, `proxy_tls.rs` 180, `proxy_decode.rs` 178), 51% of
  the row for 4 of 7 files. With the remaining files at their 1,990 estimate the row would end near
  4,860 (+14%). Replan decision (wire-proxy log): the `decode.rs` and `ws.rs` extra serve points 3
  and 1 and is accepted; the remaining six files get at most 1,802 src lines so the row ends within
  10%; the four remaining test files share 971 (PRD-03 note).
- **Trailers:** all 15 non-merge commits in 938f6733..0e411074 carry both lines.
- **X3 and the decoder windows.** A `decoder_window` GAP in A1 to A3 is an X3 FAIL as written. Under
  the default soft guard `room()` fits about 23 open flushed `br` bodies if Anthropic sends WBITS 24
  windows (unverified, PRD-03 Risks), under run 1's about 40; A2 (20 subagents) is where it would
  show. Its report line goes to PRD-02 (soft default), never to a proxy cap (PRD-02 note).
- **Unblocked:** PRD-03 slice 2 (`serve`, `hosts`, `pump`, `tunnel`, `http1`) on the merged leaf
  modules and S3; PRD-17's `lessons_decode.rs` (all but D7) and `lessons_tls.rs` T1, T2, T4, T5 now
  (PRD-17 note).
- No locked ledger item is touched and no owner question comes from this slice (the 14:47Z one
  needs no W6, PRD-02 note).

### 2026-10-30, replan after PRD-06 slice 1 (merge e6bd8a50)

- `gate fast: PASS in 109s` on e6bd8a50 (nextest summary: 127 tests run, 127 passed). Box runs before
  the merge: the fixer and reviewer A at 3b94c4d4 ran fmt, clippy `-D warnings` and goalcup nextest
  (63/63). The breaker re-checked 3b94c4d4 merged with integration (63/63) and ran throwaway stress:
  8 writers doing 3,000 `upload.json` writes each gave 0 bad reads, and held and slow PUT servers ended
  in 123 s. A status-line trickle found BR1. BR1 was fixed at 79f22106 (fmt, clippy, 63/63; the https
  trickle was not run then). BR2 was fixed at 67a9e9c0, with a handshake probe ending at 30.0 s. No
  full nextest box run was logged at 67a9e9c0 itself; the merge gate covers it. Five paths changed,
  all owned by PRD-06, with no `Cargo.toml` change.
- **Size report, PRD-06** (`wc -l` at e6bd8a50, row "Upload (PRD-06)", 940 src and 1,000 tests). The src
  files total 1,359 (145% of the row): `client.rs` 692, `state.rs` 289, `policy.rs` 264, `mod.rs` 114.
  That includes 232 inline `#[cfg(test)]` lines; without them the src is 1,127 (120%). This is 4 of 9
  files. Their own estimates add up to 430 (262%), and the module projects to about 1,737 (185%) with
  the rest at their estimates. The 13:12Z decision line (points 5 and 6) covers this merge. The PRD-06
  note makes each later merge carry its own decision line and cut before it adds. Tests are
  `upload_backoff.rs` 95 plus the 232 inline lines, 327 of 1,000. The end-of-build report counts the
  overshoot against the Rust total (about 18,100). A total past that is the owner's question then.
- **Trailers:** all 16 commits in 2be5c719..e6bd8a50 carry both lines, the merge included.
- **Seams:** S9, S10 and S18 are on integration with PRD-00's final shapes (PRD-02, PRD-10 and PRD-12
  notes). No thread runs yet, so nothing is uploaded from a run, `finish` returns at once, and no
  `ServerWord` reaches the watchdog. Every acceptance proof that needs a server copy waits for PRD-06's
  send and beat slices.
- **Acceptance lists as known,** if still open at the end: BR2-r1 (JSON call worst case about 60 s and
  PUT handshake about 90 s on a peer that trickles bytes, against the 30 s and 600 s defaults), the
  every-other-PUT backoff case (it lands with `send.rs`), and the loose `server.json` parse.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-08 slice 2 (merge 669f5d92)

- On 669f5d92: `gate fast: PASS in 89s`, with a nextest summary of 136 tests run and 136 passed. Box
  runs before the merge: the builder at d1e7f065 ran fmt, clippy `-D warnings` and nextest (22/22).
  The lane merge at 2f77d55b ran the same plus a workspace build. Reviewers A and B ran it again, and
  reviewer A added three probes. The fixer ran 25/25 and a workspace build at 50d7c49a, the commit
  before the fmt-only 6225c707, which reviewer A re-checked with six probes. The fixer's last push,
  f174fd13, got fmt, clippy, a workspace build and 25/25, and both reviewers re-checked it. Nine paths
  changed, all owned by PRD-08, with no `Cargo.toml` change.
- **Size report, PRD-08** (`wc -l` at 669f5d92, row "Verifier (PRD-08)", 1,400 src and 1,100 tests;
  the slice 1 decision ceilings were 2,400 and 1,700). Src without PRD-09's `usage/` is 1,724, which
  is 123% of the row and 72% of 2,400. That is six of ten files: `integrity.rs` 982, `server.rs` 201,
  `main.rs` 206, `input.rs` 198, `report.rs` 126, `lib.rs` 11. Tests are 1,612, which is 147% of the
  row and 95% of 1,700: `tests/common/` 877, `check_invariants.rs` 329, `check_named.rs` 218,
  `server_compare.rs` 188. With every unwritten file at its ceiling, the crate projects to about
  2,503 src and 1,802 tests. PRD-08's decision line (PRD-08 note) sets the ceilings to 2,450 src and
  1,830 tests. It names point 5, server copy proven whole and equal by entry id, and point 3, review
  fixes that turned exit-3 crashes and false passes into named results. It also names the lessons
  tests L-FW3, L-FW9 and L-VR2. The end-of-build report nets these against the Rust totals (about
  18,100 src, at most about 14,200 tests). Only a total past those becomes the owner's question.
- **Open size nit:** A9 and B3 (`main.rs` 206 of 200). Slice 3 must fold it first, since it touches
  `main.rs`. Acceptance lists it as known if it is still open at the end.
- **Build order:** the slice 2 scope said `legs.rs` and `history.rs` wait for U1 on integration. U1's
  stub with final names has been there since f5a529c8, so PRD-08's honesty, legs and history slices
  need not wait for PRD-09. Only `readable()` as a method still does, and PRD-08 covers either order.
- **Integrator:** a stray branch `v1.0/prd-08-verify-lanemerge-tmp` (2f77d55b, the same as an older
  `v1.0/prd-08-verify` tip) was pushed by mistake. The remote refused its delete (judge 14:59Z). It
  is left for the integrator to remove.
- **Trailers:** all 12 commits in a8e284db..669f5d92 carry both lines, the merge included.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-11 slice 3 (merge c14d63b8)

- `gate fast: PASS in 88s` on c14d63b8 (nextest: 147 tests run, 147 passed, 0 skipped). Before the
  merge the lane was green in boxes after each fix round: fmt, clippy with and without `test-hooks`,
  and 80 of 80 connector, `rule_code` and `native_pattern` tests at 72a1d813, 5e143d9c and ccc77a4f;
  reviewer A's last box re-check at ccc77a4f ran eight extra YAML cases. Linux only. Nine paths
  changed, all PRD-11's, no `Cargo.toml`.
- **Size report, PRD-11** (`wc -l` at c14d63b8, PRD-00 row "Connector code (PRD-11)" 800 src and 400
  tests, replan ceiling 1,510 and 620): src 1,646 (`schema.rs` 423, `hints.rs` 303, `mod.rs` 301,
  `hosts.rs` 301, `launch.rs` 291, `builtin.rs` 27), 206% of the row and 109% of the ceiling;
  tests 721 (`connector_launch.rs` 223, `connector_hints.rs` 196, `connector_builtin.rs` 155,
  `connector_hosts.rs` 105, `connector_no_model_address.rs` 42), 180% of the row and 116% of the
  ceiling. The fixer's 14:59Z decision line named points 1, 6 and 7; this replan's decision (PRD-11
  note) sets the final ceiling to 1,700 src and 760 tests, 900 and 360 over the row. The end-of-build
  report counts it against the Rust totals (about 18,100 src, at most about 14,200 tests); a total
  past them is the owner's question then.
- **Unblocked:** PRD-17 `lessons_hosts.rs` and the HF rows guarded in `connector_launch.rs`; PRD-10's
  `start` wiring with `unread` and `log` (S3 addition, PRD-00 note), `--record-host` on the real
  check, and the setup settings test; PRD-15's CONNECTOR-SCHEMA pass; the core slice's launch plans
  for Claude Code and Codex now decide hosts for real. Still waiting on PRD-11's third slice: the
  `.profile` writer and the 12 remaining connector files.
- **Acceptance lists as known,** if still open at the end: numeric address spellings are not
  `local_model` notes, and the N6 lookup has no wait of its own (PRD-11 note).
- **Trailers:** all 18 commits in 669f5d92..c14d63b8 carry both lines, the merge included.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-10 slice 3 (merge f66ce4a5)

- `gate fast: PASS in 143s` on f66ce4a5 (nextest: 152 tests run, 152 passed, 0 skipped). Before the
  merge the lane was green in boxes after each round: fmt, clippy `-D warnings`, nextest goalcup
  73/73 at the lane merge, 75/75 after the A1 to A3 fixes, 74/74 at 91a7c294,
  doctests passing; both reviewers found no blocker. Linux only: the macOS `statfs` branch is not
  compiled. Ten paths changed, all PRD-10's, no `Cargo.toml`.
- **Size report, PRD-10** (`wc -l` at f66ce4a5 against PRD-00's "CLI (PRD-10)" row, about 2,900 src
  and 1,200 tests): src 2,298 in 8 files, 79% of the row with 9 files and most of `start.rs` still to
  write; at the file table's estimates the CLI projects to at least 3,801 (131%), past the 3,190 line.
  Tests 1,184, 99% of the row, with seven test files still to come. The PRD-10 note sets `folder.rs`
  at up to 240, asks the next slice to give back lines from `text.rs` (704 against 420) and `mod.rs`
  (464 against 130) first, and asks the slice that passes 3,190 src or 1,320 tests for its own
  decision line. The end-of-build report counts the CLI against the Rust totals.
- **Seam fixed for the start slice:** `connector::load`, then `folder::check`, then `connector::plan`
  (the plan writes into the isolated home; PRD-10 and PRD-17 notes). SC2's folder case depends on it.
- **Unblocked:** PRD-17 rows SC1 and SC2 (folder case) point at `cli_folder.rs`; PRD-15 can fix the
  guide's start appendix against `folder.rs` (PRD-15 note). **Still waiting:** `start` and `seal`
  with `GOALCUP_SERVER=off` on S8 `watchdog::run`; `continue` on `pipeline::close_dead_entry`;
  `setup --yes` (S15) can now use PRD-11's real `write_settings` and version probe; the upload lines
  and `time-left` can use S9's `upload::state::read`, with `server_word` null until PRD-06's sender.
- **Acceptance list as known,** if still open at the end: the WSL1 or Windows-drive check (PRD-10
  section 3 step 2) is not built.
- **Trailers:** all 13 commits in bab29a0c..f66ce4a5 carry both lines, the merge included.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-04 core slice part 2 (merge 01afcf22)

- `gate fast: PASS in 124s` on 01afcf22 (nextest: 182 tests run, 182 passed, 0 skipped). Before the
  merge the lane was green in boxes after each of four fix rounds: fmt, `clippy -D warnings`, native
  tests 40, 41, then 43 of 43 at 84b86ab0. Linux only; no native test ran on macOS. Eleven paths
  changed, all PRD-04's, no `Cargo.toml`. All 18 commits in e6dd0444..01afcf22 carry both trailer lines.
- **Size report, PRD-04** (`wc -l` at 01afcf22, row "Native (PRD-04)" 780 src and 1,000 tests): src
  909 (`tail.rs` 363, `fsx.rs` 213, `pattern.rs` 174, `walk.rs` 131, `mod.rs` 28), 117% of the row,
  with `start`, the loop, `finish` and `snapshot.rs` still to come; tests 1,236 (`native_tail.rs` 469,
  `native_safety.rs` 240, `native_walk.rs` 202, `native_pattern.rs` 132, `native_resume.rs` 106,
  `native_room.rs` 87), 124% of the row, with `native_snapshot.rs` and `native_finish.rs` to come.
  This replan's decision (PRD-04 note) names points 1 and 3 and sets the ceiling to 1,150 src and
  1,600 tests. The end-of-build report counts it against the Rust totals (about 18,100 src, at most
  about 14,200 tests); a total past them is the owner's question then.
- **Build order unchanged.** PRD-04's last slice (`start`, loop, `finish(self, harness_exited,
  guard)`, `native_missing`, `snapshot.rs`, `native_snapshot.rs`, `native_finish.rs`) is unblocked;
  PRD-02's seal and boot wiring call it once it lands.
- **Seams to close:** PRD-14 puts `native_walk` in the `heavy` group (PRD-14 note); PRD-02 adds
  `fid` to `SnapshotCursor` (PRD-02 note).
- **Acceptance lists as known,** if still open at the end: RB7 (dead guard in `tail.rs` `end()`), C2
  (nested root on another device), RB3 (no-exclusion-list half unguarded), and the missing snapshot
  `fid` on resume (PRD-04 note).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-09 slice 2 (merge dab0ce1f)

- `gate fast: PASS in 92s` on dab0ce1f (nextest: 188 tests run, 188 passed, 0 skipped; fmt and
  `clippy -D warnings` green). Before the merge the lane was green in boxes after each of three fix
  rounds (fmt, `clippy -D warnings`, `doc -D warnings`, nextest 22 of 22 at bce2f879, 31 of 31 on a
  probe merge with integration). Five paths changed, all PRD-09's, no `Cargo.toml`. All 17 commits in
  01afcf22..dab0ce1f carry both trailer lines.
- **Size report, PRD-09** (`wc -l` at dab0ce1f, row "Usage reader (PRD-09)" 1,470 src and 200
  tests): src 1,525 (`wire.rs` 601, `shapes.rs` 456, `mod.rs` 407, `sse.rs` 61), 104% of the row with
  `native.rs`, `sqlite.rs`, `hints.rs`, `out.rs`, the join and `run` still to come; tests 299
  (`usage_run1.rs`), 150% of the row. This replan's decision (PRD-09 note) names point 1 and ledger
  S2 and sets the ceiling to 2,250 src and 380 tests, with `usage table` and `--text` built only if
  they fit. The end-of-build report counts it against the Rust totals (about 18,100 src, at most
  about 14,200 tests); a total past them is the owner's question then.
- **Build order unchanged.** PRD-09's next slice (native readers, `sqlite.rs`, `hints.rs`, the join,
  `out.rs` core part, `usage::run`) unblocks PRD-12 C5, the T6 spend check and the watchers'
  `usage --json`; until then `usage` exits 3 and pad rows say usage was not read (PRD-12 note).
- **Acceptance lists as known,** if still open at the end: B12 (one way to build a create in
  `usage_run1.rs`), and `usage table` or `--text` if not built (cut, PRD-09 note).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-14 slice 3 (merge 00fac28d)

- `gate fast: PASS in 101s` on 00fac28d (nextest 188 of 188, Worker 253, `test_install.py` 28,
  pad 17). Twelve paths changed, all PRD-14 owned, no `Cargo.toml`. All 13 commits in
  e82f03de..00fac28d carry both trailer lines. `gate full` was not run on the merge; it fails by
  design at its new hooked-binary line until PRD-06 calls `Policy::from_env` (PRD-06, PRD-14 notes).
- **Size report, PRD-14** (`wc -l` at 00fac28d, PRD-00 row "Lessons (PRD-17), test support,
  `rule_code`, `e2e_run` (PRD-14)", 0 src and 3,000 tests, of which PRD-14's line is 1,700): PRD-14
  tests are 1,736 (`tests/common/` 1,523, `rule_code.rs` 213, `e2e_run.rs` 0), 102% of its 1,700 and
  within the 1,870 ceiling; `upstream.rs` 709 of 700 (decision line 16:08Z, point 1). Src 0. Scripts
  and CI: `install.sh` 355 (was 394), `test_install.py` 314 (was 582), `release.yml` 132,
  `gate.sh` 128, `release.sh` 121, `ci.yml` 60, `smoke-release.sh` 53, `package-release.sh` 40,
  `third-party-licenses.py` 231.
- **Release order:** `release.yml` now runs the workspace nextest on all four targets (macOS and Linux
  arm64 included) before it builds, with a 60-minute job limit, and refuses a hooked debug binary
  with no `GOALCUP_TEST_` text. So the release depends on PRD-06's `Policy::from_env` call as well as
  `gate full` and the pad's release check; a watcher does not tag before that merge.
- **Seams closed:** PRD-04's `Calls` helpers (PRD-04 deletes its copies next) and PRD-10's
  `status --json` in `install.sh` (PRD-10 note keeps the shape). **Open:** `recorder::real` (PRD-02's
  `Recorder::real_for_tests`), `native_walk` in the `heavy` group (next PRD-14 touch), `e2e_run.rs`
  (phase 2), the `check-links.py` delete (phase 3).
- **Acceptance lists as known,** if still open at the end: B12 (one "bytes left" closure in
  `h2_stream`, PRD-14 note).
- **Housekeeping for the end:** `v1.0/prd-14-tests-lane-merge-wip` is still to be removed from origin.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-12 slice 4 (merge 69f355c6)

- `gate fast: PASS in 137s` on 69f355c6 (nextest 188 of 188, 16 pad tests). Six paths changed, all
  under `scripts/pad/`, no `Cargo.toml`. All 10 non-merge commits in 00fac28d..69f355c6 carry both
  trailer lines. No real key run; proof by fake-binary probes in build boxes.
- **Size report, PRD-12** (`wc -l` at 69f355c6, PRD-00 row "the pad, about 1,350 to 1,450 with its
  tests"): `scripts/pad/` is 2,540 lines (`pad` 610 of 320, `pad-run` 658 of 330, `check.py` 429 of
  320, `table` 192 of 150, `test_pad.py` 308 of 90, `image` 54 of 60, `sbx.mjs` 133 with no row, the
  rest 156), 175% of the row's top and 463 more than at 9e259b4d. Still to land: `selftest`, `exec/`,
  13 harness files and the skill (about 465 at their rows), then six faults, `pre` and the queue. The
  slice 3 decision (points 3, 4, 5) stands and the 12:53Z owner question on the row stays open; the
  end-of-build report counts the pad against it.
- **Before the first pad, A1, A2 or A3 run with a real key,** check two things besides
  `start.mask_rules` holding more than one entry: `goalcup seal --yes` is built (on integration it
  exits 3 "not built", so the 10 USD seal of ledger T6 cannot happen), and `goalcup-verify usage` no
  longer exits 3 (else the spend check reads nothing). Until both, a paid run has no working cap.
- **Spend scribe:** `pad-out/spend.jsonl` has one line per run; sum `usd`, else `usd_live`, for the
  200 and 400 USD notices (PRD-12 note lists the keys).
- **Build order unchanged.** The cycle rungs (section 1a, 4 to 6) wait on PRD-10's `seal` and
  `recover`, PRD-05's masker, PRD-09's `usage` and PRD-02's `watchdog::run`.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-10 slice 4 (merge a5d7ddc3)

- `gate fast: PASS in 93s` on the merge (nextest 188 of 188, `test_install.py` 28, pad 16). Thirteen
  paths changed, all PRD-10 owned, no `Cargo.toml`. All 9 non-merge commits in
  c4160182..a5d7ddc3^2 carry both trailer lines. `gate full` was not run.
- **S15 is on integration** (late against H+4). The pad's setup step can now get its isolated home
  from `goalcup setup --harness <id> --yes`. The cap still cannot seal and the recover row is still
  not evidence: `seal` and `recover` exit 3 "not built" (PRD-12 note stands).
- **Size report, PRD-10** (`wc -l` at a5d7ddc3, PRD-00 row "CLI (PRD-10)" about 2,900 src and 1,200
  tests): src 2,722 (`cli/*.rs` 2,521 plus `main.rs` 201), 94% of the row; `text.rs` 827 against 420,
  `mod.rs` 467 against 130, `folder.rs` 243 against the 240 the slice 3 note allowed (a three-line cut
  is named, PRD-10 note), `setup.rs` 236 of 270, `awake.rs` 24 of 150. 1,248 lines are still to come
  at the table's estimates, projecting about 3,970 (137%), past the 3,190 line. The PRD-10 note keeps
  the row, asks each later slice to state `text.rs` and `mod.rs` counts in its merge note, and makes
  the crossing slice bring its own decision line. Tests 1,180 of 1,200 after the trim (697 to 495
  lines in `cli_status.rs`, `cli_finish.rs` and `cli_common/`).
- **Build order unchanged.** S8 `watchdog::run` is still on no branch, so `start` and `seal` with
  `GOALCUP_SERVER=off` still cannot be built. PRD-10's next slice is `time-left` and the upload lines
  (S9), then the interactive setup.
- **Acceptance lists as known,** if still open at the end: the WSL1 or Windows-drive check never run
  in a WSL box; the macOS `caffeinate` probe and `f_fstypename` branch never compiled in a macOS box;
  `native_walk old_files_are_skipped_and_the_new_one_found_among_five_thousand` failed 1 of 4 runs
  in a slice 4 box (not touched by that slice; PRD-04's test, in PRD-14's `heavy` group question).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-14 slice 4 (merge ec0d79d8)

- `gate fast: PASS in 134s` on ec0d79d8 (nextest 188 of 188, `native_walk`'s 7 in the `heavy` group).
  Three paths changed (`.config/nextest.toml`, `scripts/gate.sh`, `tests/common/upstream.rs`), all
  PRD-14 owned, no `Cargo.toml`. Both non-merge commits in 722b846b..ec0d79d8 carry both trailer
  lines. `gate full` was not run; it stays red until PRD-06 calls `Policy::from_env` (slice 3 note).
- **Size report, PRD-14** (`wc -l` at ec0d79d8, PRD-00 row "Lessons (PRD-17), test support,
  `rule_code`, `e2e_run` (PRD-14)", 0 src and 3,000 tests, of which PRD-14's line is 1,700): tests
  1,736 (`tests/common/` 1,523 with `upstream.rs` 709, `rule_code.rs` 213, `e2e_run.rs` 0), 102% of
  the 1,700 line and within the 1,870 ceiling, unchanged by this slice. Src 0. Scripts and CI:
  `gate.sh` 105 (was 128; section 8 says about 75, which section 8's own required steps rule out,
  PRD-14 note), `.config/nextest.toml` 21, `release.yml` 132, `release.sh` 121, `ci.yml` 60,
  `test_install.py` 314, `smoke-release.sh` 53, `package-release.sh` 40, `third-party-licenses.py`
  231, `install.sh` 355, all unchanged.
- **Heavy-group rule, refined** (the note after PRD-17 phase 0 slice): a binary joins in the merge
  that brings its heavy case, not its first case, and adds its name to the one override's
  alternation. `native_walk` is in. `proxy_connect` is on integration but waits for PRD-03's 2,000
  connection case (PRD-03 note). `lessons_launch`, `watchdog_*`, `e2e_*`, `pipeline_*` and
  `proxy_restart` join with their binaries.
- **Seams closed:** `native_walk` in `heavy` (PRD-04 note), B12. **Open:** `recorder::real`
  (PRD-02's `Recorder::real_for_tests`), `e2e_run.rs` (phase 2), the `check-links.py` delete (phase 3).
- **Acceptance lists as known,** if still open at the end: the slice 4 review nits on `gate.sh` and
  the nextest comment (A1 header format line, A2 the full tier's phase 2 note, A3 the "brings its
  heavy case" wording, A4 and B2 the script-test admission rule, B1 three behaviour-neutral cuts),
  all queued for PRD-14's next touch (PRD-14 note).
- **Housekeeping for the end:** besides `v1.0/prd-14-tests-lane-merge-wip`, the merged PRD-14 task
  branches (`v1.0/prd-14-tests-*`) are still on origin.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-11 slice 4, part 1 (merge a96d9f71)

- `gate fast: PASS in 96s` on the merge (nextest 189 of 189). Eleven paths changed, all PRD-11
  owned, no `Cargo.toml`. All 13 non-merge commits in ec0d79d8..a96d9f71^2 carry both trailer
  lines. Review rounds A and B closed with no open finding. `gate full` was not run.
- **Size report, PRD-11** (`wc -l` at a96d9f71, PRD-00 row "Connector code (PRD-11)" 800 src and
  400 tests): src 1,689 (211% of the row; 1,689 of the 1,700 ceiling set by the replan after slice
  3), tests 759 (190% of the row; 759 of the 760 ceiling). The PRD-11 note keeps both ceilings and
  allows one exception: review B9's `write_settings` change may pass 1,700 only by the lines PRD-10
  drops from `cli/setup.rs` in the same merge.
- **Build order, PRD-11:** slice 5 (amp, droid, goose, crush, kimi-code, cline with their six
  `entry!` lines; Amp and Droid are the first users of the `.profile` writer) can start now. The
  facts sheets were not rewritten in this part; all 15 move to PRD-11's last slice, which therefore
  waits for PRD-12's pad rows as well as the tier refresh. Ten of 16 connector files are on
  integration, so the pad can run rows for `copilot-cli`, `gemini-cli`, `openclaw-cli`,
  `opencode-cli`, `opencode2-cli` and `qwen-cli` where it has their logins.
- **Acceptance lists as known,** if still open at the end: a hand-written JSON5 `openclaw.json` is
  unread, so its proxy keys are not checked (named in `unread`); a `--version` line whose number is
  wrapped in punctuation refuses the start.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-06 slice 2 (merge a98f104c)

- `gate fast: PASS in 96s` on a98f104c (nextest summary: 207 tests run, 207 passed). Box runs before
  the merge: the lane merge at f2987846 (fmt, build, clippy `-D warnings`, goalcup nextest 160/160);
  reviewers A and B and the breaker at f2987846 and again at f1b4e9a6 (upload tests 25/25 and 26/26,
  goalcup 161/161); the fixer at 419bdbb6 (fmt, clippy, 175/175). The breaker's throwaway run-1-shape
  walk (40 then 200 streams, 1.5 MB bursts, a held writer, prune and lost answers) sent 674 MB raw
  byte for byte with a largest chunk of 26 MB and a peak RSS of 115 MB; it is not a committed test.
  Nine paths changed, all owned by PRD-06, with no `Cargo.toml` change. All 13 commits in
  a96d9f71..a98f104c carry both trailer lines.
- **Size report, PRD-06** (`wc -l` at a98f104c, row "Upload (PRD-06)", 940 src and 1,000 tests). Src
  1,724 with 176 inline test lines; 1,548 without them (165%), 6 of 9 files; projects to about
  1,868 (199%) with the last three at their estimates. Tests 984 in files plus 176 inline, 1,160
  (116%), before the sender, prune and manual tests. The 16:21Z decision line (points 5 and 6)
  covers this merge; the PRD-06 note asks slices 3 and 4 to cut the inline tests first. The
  end-of-build report counts the overshoot against the Rust total (about 18,100).
- **Closed since slice 1:** BR2-r1 (worst case now 30 s) and the loose `server.json` parse. Remove
  them from the known list of the slice 1 note.
- **Still open, in build order:** `Policy::from_env` is not called, so `gate full` stays red and no
  release can be cut until PRD-06 slice 3 merges (PRD-14 slice 3 note). `Recorder::live()` fills
  `held_bytes` only (PRD-02 note): the live heartbeat's `failed` for a stalled writer and its
  `losing` state wait on PRD-02, and acceptance proofs that read `recording` from a live run (a held
  writer, a stream in a gap) cannot pass before both PRD-06 slice 3 and that PRD-02 change are on
  integration. Every proof that needs a server copy still waits for slice 3.
- **Acceptance lists as known,** if still open at the end: the every-other-PUT backoff case (lands
  with `send.rs`), BK1 (a handshake loop with no I/O after a mid-handshake `close_notify`, one line in
  slice 3), and BK2 (prune deletes the part file before its `.idx`, slice 4).
- **Housekeeping for the end:** `v1.0/prd-06-upload-mergecheck-tmp` was pushed by mistake (lane log
  15:59Z) and is still to be removed from origin.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-04 core slice part 3 (merge a302d688)

- `gate fast: PASS in 87s` on a302d688 (nextest summary: 213 tests run, 213 passed, 0 skipped;
  `native_walk` ran in the `heavy` group). Before the merge, three review rounds closed every finding
  (F1, RB-1 to RB-5, A-R3-1, B-R4-1); the last box runs at 328cb905 were fmt, `clippy -D warnings`,
  native 49 of 49 three times and goalcup 150 of 150. Linux only; no native test ran on macOS. Five
  paths changed, all PRD-04's, no `Cargo.toml`. All 10 non-merge commits in a98f104c..a302d688^2 carry
  both trailer lines. `gate full` was not run.
- **Size report, PRD-04** (`wc -l` at a302d688, PRD-00 row "Native (PRD-04)" 780 src and 1,000 tests):
  src 1,144 (`tail.rs` 368, `mod.rs` 258, `fsx.rs` 213, `pattern.rs` 174, `walk.rs` 131), 147% of
  the row and 6 under the 1,150 ceiling of the part 2 note, with `snapshot.rs` still to come; tests
  1,474 (`native_tail.rs` 469, `native_safety.rs` 250, `native_finish.rs` 227, `native_walk.rs` 203,
  `native_pattern.rs` 132, `native_resume.rs` 106, `native_room.rs` 87), 147% of the row, 126 under
  the 1,600 ceiling. This replan's decision (PRD-04 note) names points 1 and 3 and sets the src
  ceiling to 1,260 (about 162% of the row); tests stay at 1,600. The end-of-build report counts it
  against the Rust totals (about 18,100 src, at most about 14,200 tests); a total past them is the
  owner's question then.
- **Build order:** PRD-04's last slice is `snapshot.rs` with `native_snapshot.rs` and the finish step 3
  hookup. PRD-02 can wire `native::start` (boot step 5) and `finish` (seal step 4) now; nothing calls
  them on integration yet, so no real run reads a session file until that PRD-02 slice merges. Pad
  rows that check native streams (PRD-12) wait for it.
- **Seams to close, in build order:** PRD-02 fills `Recorder::counts()` (at least `model_requests`),
  or `native_missing` never fires in a real run (PRD-02 note); PRD-02 adds `fid` to `SnapshotCursor`
  (part 2 note, still open).
- **Closed since part 2:** RB7, RB3 and the `native_walk` run-start flake. Remove them from the known
  list of the part 2 and PRD-14 slice 4 notes.
- **Acceptance lists as known,** if still open at the end: C2 (nested root on another device), the
  missing snapshot `fid` on resume, review nits N1, N2, N3 and N5 (PRD-04 note), and, until
  `snapshot.rs` merges, `final` rows copied only at the seal as single `snapshot` files.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-08 slice 3 (merge 7a995b6e)

- `gate fast: PASS in 105s` on 7a995b6e (nextest summary: 218 tests run, 218 passed, 0 skipped). Box
  runs before the merge: the builder at c42bfce1 (fmt, clippy `-D warnings`, workspace build,
  goalcup-verify 30 of 30); the lane merge at aaf7879b (36 of 36); reviewers A and B with probes
  (39 and 42 tests with their probes); the fixer at 3bd40ba6, fc81212c and a76e0bce (fmt, clippy,
  36 of 36 each), each re-checked by a reviewer; the last re-check left no open finding. Six paths
  changed, all owned by PRD-08, no `Cargo.toml`. All 5 non-merge commits in ed6f7bf6..7a995b6e and
  the merge carry both trailer lines. `gate full` was not run.
- **Size report, PRD-08** (`wc -l` at 7a995b6e, row "Verifier (PRD-08)", 1,400 src and 1,100 tests;
  the slice 2 ceilings were 2,450 and 1,830). Src without PRD-09's `usage/` is 2,235, 160% of the row
  and 91% of 2,450, seven of ten files: `integrity.rs` 982, `secrets.rs` 509, `server.rs` 201,
  `main.rs` 200, `input.rs` 198, `report.rs` 133, `lib.rs` 12. Tests are 1,826, 166% of the row and
  1,826 of 1,830: `tests/common/` 877, `check_invariants.rs` 329, `check_named.rs` 218,
  `secrets_scan.rs` 214, `server_compare.rs` 188. With the three unwritten files and two test files at
  their ceilings the crate projects to about 2,770 src and 1,986 tests. PRD-08's decision line
  (PRD-08 note) caps `secrets.rs` at 509 and sets the ceilings to 2,730 src (with the PRD-01 cut of
  about 40 lines of `input.rs`, now due before `legs.rs`) and 1,990 tests, naming points 4 and 3.
  The end-of-build report nets these against the Rust totals (about 18,100 src, at most about 14,200
  tests). Only a total past those becomes the owner's question.
- **Closed:** the A9/B3 `main.rs` nit (200 of 200). Remove it from the known list of the slice 2 note.
- **Owner-question line answered without the owner:** the fixer's 16:27Z line (kind owner question)
  asked for PRD-08 section 3's delta wording to add the Chat Completions keys. That is PRD text, not
  a ledger item; the PRD-08 note settles it. No W6 line.
- **What is unblocked:** X7 and N6 can run `goalcup-verify secrets` now. Both pass only paths that
  exist: one unreadable path on the command line exits 3 with no report, while one inside a walked
  folder only marks that input `partial`. `-` reads the project's git objects from stdin. PRD-12's C3
  is no longer held by PRD-08; it still waits on PRD-05's masker and a `goalcup` build, and on the
  pad's two small changes (PRD-12 note).
- **Build order, PRD-08:** honesty, legs, history and the run 1 shape builder remain, in this plan's
  order; none waits on PRD-09 (slice 2 note). The PRD-01 ask is cut first.
- **Acceptance lists as known,** if still open at the end: the PRD-08 slice 3 items in the PRD-08
  note (A5 nits 2 and 3, A7, A8, F11, the double count of a file named twice, the PRD-01 ask).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-14 slice 5 (merge b3747b6a)

- `gate fast: PASS in 126s` on b3747b6a (nextest 218 of 218, 0 skipped). Box runs before the merge:
  the builder's `gate fast` PASS in 59s at 1f201719 (213 of 213, root refusal and missing-prerequisite
  FAIL lines checked) and `sh -n release.sh` at 0eef7d25; the fixer's `gate fast` PASS at 7795b322;
  reviewer A's re-check left no open finding. Four paths changed (`.config/nextest.toml`,
  `packaging/README.md`, `scripts/gate.sh`, `scripts/release.sh`), all PRD-14 owned, no `Cargo.toml`.
  All 3 non-merge commits in 7a995b6e..7795b322 carry both trailer lines. `gate full` was not run; it
  stays red at the hooked-binary line until PRD-06 calls `Policy::from_env` (slice 3 note).
- **Size report, PRD-14** (`wc -l` at b3747b6a, PRD-00 row "Lessons (PRD-17), test support,
  `rule_code`, `e2e_run` (PRD-14)", 0 src and 3,000 tests, of which PRD-14's line is 1,700): tests
  1,736 (`tests/common/` 1,523 with `upstream.rs` 709, `rule_code.rs` 213, `e2e_run.rs` 0), 102% of
  the 1,700 line and within the 1,870 ceiling, unchanged by this slice. Src 0. Scripts and CI:
  `gate.sh` 103 (was 105), `.config/nextest.toml` 21, `packaging/README.md` 104 (was 103),
  `release.sh` 121, `release.yml` 132, `ci.yml` 60, `test_install.py` 314, `smoke-release.sh` 53,
  `package-release.sh` 40, `third-party-licenses.py` 231, `install.sh` 355.
- **Closed:** the slice 4 review nits A1, A2, A3, A4, B1 and B2. Remove them from the known list of
  the slice 4 note. **New known item** if still open at the end: review B-N4 (the signal trap in
  `gate.sh` is set only after the build lock, so a kill before then prints no FAIL line), queued for
  PRD-14's next touch of `gate.sh` (PRD-14 note).
- **Seams, unchanged:** `recorder::real` (PRD-02's `Recorder::real_for_tests`), `e2e_run.rs`
  (phase 2), the `check-links.py` delete (phase 3), `proxy_connect`'s heavy case (PRD-03).
- **Housekeeping for the end:** add `v1.0/prd-14-tests-gate-nits-nextest-comment` and
  `v1.0/prd-14-tests-packaging-readme-tag-check` to the merged PRD-14 task branches still on origin.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-09 slice 3 (merge 1a2c3537)

- `gate fast: PASS in 119s` on 1a2c3537 (nextest 219 of 219, 0 skipped); box runs and reviewer
  re-checks are in the PRD-09 note. `gate full` was not run.
- **Size report, PRD-09** (`wc -l` at 1a2c3537, row "Usage reader (PRD-09)", 1,470 src and 200
  tests; the slice 2 ceilings were 2,250 and 380): src 2,137, 145% of the row and 95% of 2,250
  (`wire.rs` 607, `mod.rs` 501, `shapes.rs` 456, `native.rs` 445, `hints.rs` 67, `sse.rs` 61); tests
  `usage_run1.rs` 381, 190% of the row and one past 380. With `sqlite.rs` 150, `out.rs` 120 and
  `run` 40 the crate projects to about 2,450. PRD-09's decision line (PRD-09 note) sets the
  ceilings to **2,450 src and 400 tests**, naming points 1 and 2. The end-of-build report nets this
  against the Rust totals (about 18,100 src, at most about 14,200 tests); only a total past those
  becomes the owner's question.
- **Build order, PRD-09:** the fourth slice is `sqlite.rs`, `out.rs` and `run`, then the CSV and
  `--text` only if they fit. Nothing else waits on it except PRD-12's spend check, which reads the
  summary keys already named.
- **Acceptance lists as known,** if still open at the end: the PRD-09 slice 3 nits A2-1, A2-2 and
  A2-3 (PRD-09 note).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-08 slice 4 (merge 1fdaf2c3)

- `gate fast: PASS in 92s` on 1fdaf2c3 (nextest 237 of 237, 0 skipped; Node 253 of 253). Box runs
  before the merge: the builder's two task branches (fmt, clippy `-D warnings`, 36 and 45 tests), the
  lane merge at 5c92ab79 (45 of 45), the fixers at 71abb3d8 (47 of 47) and 84eb26d3 (48 of 48 and
  the reviewer's probe); each re-checked by a reviewer, the last leaving no open finding. Six paths
  changed (`honesty.rs`, `integrity.rs`, `lib.rs`, `report.rs`, `tests/check_conn_bytes.rs`,
  `tests/check_named.rs`), all PRD-08 owned, no `Cargo.toml`. `gate full` was not run.
- **Trailers:** of the 7 non-merge commits in 5cc59b65..84eb26d3, three carry neither trailer line:
  71abb3d8 (rustfmt), ae15edae and efe3595f (test fixes). The branch is merged and not rewritten;
  the end-of-build report lists them as known.
- **Size report, PRD-08** (`wc -l` at 1fdaf2c3, row "Verifier (PRD-08)", 1,400 src and 1,100 tests;
  the slice 3 ceilings were 2,730 and 1,990). Src without PRD-09's `usage/` is 2,678, 191% of the
  row and 98% of 2,730 (`integrity.rs` 991, `secrets.rs` 509, `honesty.rs` 435, `server.rs` 201,
  `main.rs` 200, `input.rs` 198, `report.rs` 130, `lib.rs` 14). Tests are 2,083, 189% of the row and
  93 over 1,990 (`tests/common/` 877, `check_named.rs` 341, `check_invariants.rs` 329,
  `secrets_scan.rs` 214, `server_compare.rs` 188, `check_conn_bytes.rs` 134). With `legs.rs` 240,
  `history.rs` 155 and the `input.rs` cut of 40, and two test files of 80, the crate projects to
  about 3,033 src and 2,243 tests. PRD-08's decision line (PRD-08 note) names points 1, 2 and 4,
  keeps `honesty.rs` at 435, and sets the ceilings to **3,040 src and 2,245 tests**. The
  end-of-build report nets these against the Rust totals (about 18,100 src, at most about 14,200
  tests); only a total past those becomes the owner's question.
- **Build order, PRD-08:** `legs.rs`, `history.rs` and the run 1 shape builder remain, in this plan's
  order; the PRD-01 `input.rs` cut still comes first.
- **Seams:** the `conn_bytes` duties of the proxy are in the PRD-03 note (plain tails, `wire`,
  `conn` GAP on a parser fault); the pad's reading of the three ids is in the PRD-12 note.
- **Acceptance lists as known,** if still open at the end: the PRD-08 slice 4 nits A9, B-R1, B-R6
  and B-R7 (PRD-08 note), and the three commits without trailers above.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-12 slice 5 (merge 9e71bde0)

- `gate fast: PASS in 131s` on 9e71bde0; the gate runs the pad's tests (python3.11 step). Box runs
  before the merge: 31 pad tests at f7d585fa, 20 at 96df4f07 after the C-1 to C-5 cuts, each round
  re-checked by a reviewer, the last leaving no open finding but the A-1 and C-7 nits. Sixteen paths
  changed, all PRD-12 owned. All 16 non-merge commits carry both trailer lines. `gate full` was not
  run and no real-key run was made.
- **Size report, PRD-12** (`wc -l` at 9e71bde0, row "pad" about 1,350 to 1,450 with its tests):
  3,041 under `scripts/pad/` (`pad` 654, `pad-run` 801, `check.py` 583, `test_pad.py` 366, `table`
  192, `sbx.mjs` 133, `exec/` 96, `image` 54, `pad.toml` 31, prompts and runs 32, harness files 98,
  `image.id` 1), 110% over the top of the row. About 3,440 projected with `selftest`, 13 harness files
  and the skill, before the six faults and the queue. PRD-12's decision line (PRD-12 note) names points
  1, 3 and 4. The owner question on the row stays open (12:53Z); the end-of-build report lists the
  pad as over its row, with the per-file figures. Nothing blocks on it.
- **Acceptance lists as known,** if still open at the end: the PRD-12 slice 5 nit A-1 (`c3` `more`),
  the unproven live looks (`start-refused` `value_shown`, `daemon-check`) until the first real-run
  read, and the six faults that still answer skip.
- **Pad rungs 7 to 9 are in** (gateway, dodger, `exec` and `pre`); PRD-03 acceptance 3, 7 and 9 and
  PRD-11 acceptance 3, 4 and 5 and PRD-10 acceptance 7 now have their watcher scripts. They run only in
  a real-key row, so none is evidence before PRD-10's `seal` and `recover` merge and the first `micro`
  row is read by hand (P1, ledger T6).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-04 second slice (merge 7e8ed89b)

- `gate fast: PASS in 117s` on 7e8ed89b. Box runs before the merge, last at 0b8fd577: fmt, `clippy -D
  warnings`, native 55 of 55 three times, goalcup 175 of 175; the earlier rounds closed A1 to A9 and B1
  to B9 (B5, B6 and B9 declined as observations or taste, accepted), and the last re-check left only
  nit A10 (a PRD-00 ownership note, now written). Linux only; no native test ran on macOS. Ten paths
  changed, all PRD-04's (`src/native/`, `tests/native_*`, `tests/native_common/`, its PRD note); no
  `Cargo.toml`. All 7 non-merge commits carry both trailer lines. `gate full` was not run.
- **Size report, PRD-04** (`wc -l` at 7e8ed89b, PRD-00 row "Native (PRD-04)" 780 src and 1,000 tests):
  src 1,488 (`tail.rs` 410, `snapshot.rs` 289, `mod.rs` 271, `fsx.rs` 213, `pattern.rs` 174, `walk.rs`
  131), 191% of the row and 228 over the 1,260 ceiling of the part 3 note; tests 1,768 (`native_tail.rs`
  441, `native_snapshot.rs` 346, `native_safety.rs` 245, `native_walk.rs` 194, `native_finish.rs` 173,
  `native_pattern.rs` 132, `native_resume.rs` 106, `native_room.rs` 87, `native_common/mod.rs` 44),
  177% of the row and 168 over the 1,600 ceiling. The PRD-04 note's decision names points 1 and 3 and
  sets the ceilings to 1,500 src and 1,780 tests. PRD-04 is 720 src and 780 tests over its row; with
  PRD-11 (900 and 360 over, ceilings 1,700 and 760) and PRD-12 over its row, the end-of-build report
  must show the Rust totals (about 18,100 src, at most about 14,200 tests) against the sum of the
  ceilings. A total past them is the owner's question then.
- **Build order:** PRD-04 has no slice left. Its seam is waiting on PRD-02: wire `native::start` (boot
  step 5) and `finish` with the real `harness_exited` (seal step 4), fill `Recorder::counts()` and add
  `fid` to `SnapshotCursor`. Until that merges no real run reads a session file or copies a database,
  so no pad row that checks native streams or sets (PRD-12) is evidence yet, and PRD-02's `pipeline_seal.rs`
  can add the `form: "final"` and `set` assertions now.
- **Acceptance lists as known,** if still open at the end: C2 (nested root on another device), the
  missing snapshot `fid` on resume, and review nits N1, N2, N3 and N5. Closed: the part 3 line about
  `final` rows copied only at the seal. PRD-04 acceptance 1 (every subagent and database recorded) is
  proven in tests but not yet in any real run.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-02 core step 1 (merge 2fd96576)

- `gate fast: PASS in 125s` on 2fd96576 (integrator 00:07Z). Box runs before the merge, last at
  c1e65da6: fmt, `clippy -D warnings`, `nextest` 251 of 251 for the workspace, goalcup 201 of 201 and
  five reruns of the guard, gap, counts and seal tests; the breaker's stress at 40 and 200 streams,
  1.5 MB bodies, a held writer, 200 WebSockets and a seal under load found no lost byte without a GAP,
  no wait and no latch. The last round left only nits: B-3, B-4, B-5 and BRK8-N1 were declined and the
  declines accepted. Twelve paths changed under `crates/goalcup/src/pipeline/` and `tests/pipeline_*`, all
  PRD-02 owned, no `Cargo.toml`. All 27 non-merge commits carry both trailer lines. Linux only; no
  macOS run of the clock or open-file code. `gate full` was not run.
- **Size report, PRD-02** (`wc -l` at 2fd96576, row "Pipeline and watchdog (PRD-02)", 1,350 src and
  1,500 tests): src 3,485 (`pipeline/` 3,283 and `watchdog/mod.rs` 202), 258% of the row; tests 662
  (`pipeline_counts.rs` 598, `pipeline_secure.rs` 64), 44% of the row. The files still to build project
  to about 5,700 src. PRD-02's note (decision line, points 1, 3 and 6) sets the ceilings to **5,800
  src** and 1,500 tests. That is 4,450 src over the row. Added to PRD-04 (720 and 780 over), PRD-11
  (710 and 220 over), PRD-08 (1,640 and 1,145 over) and the pad, the ceilings of those four rows
  alone are about 7,500 src over their rows, against the Rust target of about 18,100 src, so the
  end-of-build report must show the measured total against the sum of the ceilings. Reviewer B holds each remaining PRD-02 slice to the 5,800. A total past the targets after
  the cuts is the owner's question then, not before.
- **Acceptance, X3 and the breaker.** `held_peak` can pass the soft guard by the control items admitted
  (OPEN, END, GAP, EVENT are never refused, PRD-02 section 3): X3 reads the `memory_guard` gap count, and
  the peak goes in the report. The breaker's check "`held_peak` under guard plus 1 MiB" reads "plus
  48 KiB per OPEN, END and GAP in the record" (PRD-02 note). `pipeline_stall.rs` was not part of this step and is not yet
  built.
- **Build order, PRD-02:** the rest is `writer.rs`, `mask_worker.rs`, then `boot.rs` with `native::start`
  and the `SnapshotCursor.fid` change with PRD-04, `seal.rs` with `native::finish(harness_exited, guard)`,
  `dead.rs`, then the watchdog. No pad row is evidence for a recorder that writes until the writer and
  boot land; PRD-12's `micro` rung waits on them.
- **Acceptance lists as known,** if still open at the end: `SnapshotCursor.fid` (PRD-02 note), the
  control-item weight nit (BRK9-N1) and the `open_gaps` comment (B-6) if the next touch did not fix them.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-11 slice 5 (merge 7a647ba4)

- `gate fast: PASS in 131s` on 7a647ba4 (integrator 00:12Z). Box runs before the merge: fmt,
  `clippy -D warnings`, 27 of 27 connector and `rule_code` tests. Nine paths changed (six connector files,
  `model-hosts.toml`, `builtin.rs`, the harness README), all PRD-11 owned, no `Cargo.toml`. Reviews A and
  B left no blocker; two fixes and four nits were closed, A-N3 declined (accepted), two new Goose nits
  moved into PRD-11's note. Linux only; no real harness run of the six; `gate full` not run.
- **Size report, PRD-11** (`wc -l` at 7a647ba4, row "Connector code (PRD-11)", 800 src and 400 tests):
  src 1,695 (`schema.rs` 423, `launch.rs` 324, `mod.rs` 317, `hints.rs` 303, `hosts.rs` 301,
  `builtin.rs` 27), 212% of the row and 5 under the 1,700 ceiling; tests 759 (`connector_launch.rs` 261,
  `connector_hints.rs` 196, `connector_builtin.rs` 155, `connector_hosts.rs` 105,
  `connector_no_model_address.rs` 42), 190% of the row and 1 under the 760 ceiling. That is 895 src and
  359 tests over the row, so the earlier "710 and 220 over" is out of date: use 895 and 359. The six new
  connectors added 6 src lines; the rest of the slice is data. The 1,700 and 760 ceilings stand. The last
  slice adds no src or test line; B9 stays the one allowed exception, unused.
- **Build order, PRD-11:** the only slice left is data, the Goose hint fix and the facts sheets with the
  tier refresh, after the pad's rows. Pad rows for the six new connectors are PRD-12's (note added);
  PRD-10 gets a one-line rule for Cline's empty `resume` (note added).
- **Acceptance lists as known,** if still open at the end: P11 on a real binary (Amp only, no key),
  Droid's daemon with no stop command, Crush's database below an existing `.crush`, schemeless Goose host
  variables, and the B9 `fill` copy.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-15 slice 9 (merge d56216f4)

- `gate fast` PASS on the four task branches (83 to 93 s) and on the fixed lane branch (120 s). Review A
  and B left no blocker; the fixes and nits closed, with four nits moved to PRD-15's note.
- **Size report, PRD-15** (`wc -l` at c262ef5a). PRD-00 "Size budget" has no docs row, so the
  comparison is with the PRD-15 outline estimates: `docs/OPERATING.md` 233 (estimate 280, 83%);
  `docs/RECORD-FORMAT.md` 419 (estimate 280, 150%, decision line in PRD-15's note, ceiling 430);
  `docs/CONNECTOR-SCHEMA.md` 115 (estimate 90, 128%); `scripts/README.md` 39. The slice's diff is 291
  lines in and 920 out over twelve files. No Rust, script or workflow line changed, so the Rust and
  Worker rows are unaffected.
- **Gate for PRD-16:** the docs gate step for this lane stays `gate fast`; links are not checked in CI
  (PRD-14 took `check-links.py` out of the gate), so the final link pass is PRD-15's own run before
  PRD-14 deletes the script. The known broken-link filter list is in PRD-15's note.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-09 slice 4 (merge c262ef5a)

- **Gate and box runs:** box runs before the merge were fmt, clippy `-D warnings`, doc `-D warnings`
  and nextest on the builder branch (39 of 39), the lane merge (51 of 51), the fixer round (52 of 52),
  and the re-check at 63003771 (fmt, clippy, nextest 52 of 52), each re-read by a reviewer, the last
  leaving no open fix or blocker. Review findings fixed: `path` kept out of `calls.jsonl`, `detail`
  once, `read_error` instead of a silent partial read, per-thread `models` and `effort`,
  `harness_version` with spaces, safe output file names. `gate full` was not run here.
- **Size report, PRD-09** (`wc -l` at c262ef5a, row "Usage reader (PRD-09)", 1,470 src and 200
  tests): src 2,396, 163% of the row and 94% of 2,550 (`mod.rs` 612, `wire.rs` 607, `shapes.rs` 456,
  `native.rs` 447, `out.rs` 146, `hints.rs` 67, `sse.rs` 61); tests `usage_run1.rs` 503, 252% of the
  row and 97% of 520. PRD-09's decision line (PRD-09 note) sets the ceilings to **2,550 src and 520
  tests**, naming points 1, 2 and 6. `sqlite.rs` (150) is the last src that fits; the CSV, `--text`
  and `run.json` are cut unless a reader needs them. The end-of-build report nets this against the
  Rust totals (about 18,100 src, at most about 14,200 tests); only a total past those becomes the
  owner's question.
- **Build order, PRD-09:** slice 5 is `sqlite.rs` with its one case and the `summary()` grouping nit
  (B2-1); slice 6 is `scripts/usage-job.sh` and whatever of the CSV, `--text` and `run.json` fits.
  PRD-12's spend check and the watchers' `usage --json` are unblocked now (PRD-12 note: `read_error`
  rule). `usage` no longer exits 3 on a readable entry, so the stale line in the slice 2 note ("until
  then `usage` exits 3") is out of date.
- **Seams:** the `read_error` rule is in the PRD-12 note; the five new `CallTable` fields are in the
  PRD-08 note; PRD-15's usage section can now be written from `usage/out.rs` (PRD-15 note).
- **Acceptance lists as known,** if still open at the end: the PRD-09 slice 4 nits B2-1, B2-2 and
  B2-3 (PRD-09 note), and the cut of `usage table` or `--text` if not built.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-06 slice 3 (merge a76bd11d)

- **Size report, PRD-06 slice 3** (`wc -l` at a76bd11d, row "Upload (PRD-06)", 940 src and 1,000
  tests): src 2,253 (client 644, send 417, beat 289, chunk 273, state 255, policy 232, mod 143), of
  which 176 inline tests; 2,077 without them, 221% of the row (240% with them). Tests 1,449
  (`upload_chunks` 516, `upload_beat` 498, `upload_resume` 343, `upload_backoff` 92) plus 176 inline,
  1,625, 163% of the row. 7 of 9 src files written; `prune.rs` and `manual.rs` remain. PRD-06's
  decision line (PRD-06 note, points 3, 5 and 6) sets the ceilings to **2,500 src and 1,900 tests**.
- **Rows over their size line, tracked for the end-of-build report:** connector (ceilings 1,510 src and
  620 tests against 800 and 400), usage (2,550 and 520 against 1,470 and 200), upload (2,500 and 1,900
  against 940 and 1,000). Only a Rust total past about 18,100 src or about 14,200 tests becomes the
  owner's question; the other rows' final counts decide it.
- **`gate full` stays red** until PRD-06 slice 4 calls `Policy::from_env()` from the threads (PRD-14
  slice 3 note; still uncalled at a76bd11d). No release is cut before it; phase 2 and 3 runs are not
  affected.
- **Slice 4 of PRD-06 is the last PRD-06 slice** and carries the open acceptance cases listed in the
  PRD-06 note (panic to `internal`, prune, manual, the null thread). The `upload_worker.rs` full-tier
  run waits for it.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-08 slice 5 (merge bfd244e3)

- **Size report, PRD-08 slice 5** (`wc -l` at bfd244e3, row "Verifier (PRD-08)", 1,400 src and 1,100
  tests, without PRD-09's `usage/` and `usage_run1.rs`): src 3,241, 231% of the row (`integrity.rs`
  1,011, `secrets.rs` 509, `honesty.rs` 498, `legs.rs` 477, `server.rs` 201, `main.rs` 200, `input.rs`
  198, `report.rs` 133, `lib.rs` 14); tests 2,627, 239% of the row (`common/mod.rs` 879,
  `check_named.rs` 341, `check_invariants.rs` 329, `run1.rs` 232, `secrets_scan.rs` 214,
  `server_compare.rs` 188, `check_run1_shape.rs` 165, `check_legs.rs` 145, `check_conn_bytes.rs` 134).
  PRD-08's decision line (PRD-08 note, points 1, 2 and 3) sets the ceilings to **3,460 src and 2,750
  tests**, to cover `history.rs`, `check_history.rs`, `timeline` and the manifest part.
- **Rows over their size line, updated for the end-of-build report:** connector (1,510 src and 620
  tests against 800 and 400), usage (2,550 and 520 against 1,470 and 200), upload (2,500 and 1,900
  against 940 and 1,000), verifier (3,460 and 2,750 against 1,400 and 1,100). Sum of the PRD-00 src rows
  is already 18,420; the four rows above add about 5,400 src if they end at their ceilings. The Rust
  total of about 18,100 src (at most about 14,200 tests) is therefore at real risk of being passed unless
  the unbuilt rows (proxy, masking, CLI, pipeline) come in well under theirs. Check the total at the
  next replan after PRD-03 or PRD-05 merges and write it as a `decision` or `owner question` line then;
  this note does not decide it. Only a total past those figures becomes the owner's question.
- **X1 reads `timeline`**, which is PRD-08 slice 7; the pad cannot read it before that merges. Order
  slice 7 before phase 4's first X1 reads. `wire_keeps_up`, `threads_match` and `subagents_present` are
  on integration, and the K2 fixtures follow in slice 6 (PRD-08 and PRD-12 notes).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-15 slice 10 (merge 524c6745)

- **Size report, PRD-15 slice 10.** PRD-15 writes docs, so PRD-00 "Size budget" has no row for it; the
  comparison is against the PRD-15 section 10 estimates (`wc -l` at 524c6745): RECORD-FORMAT 502 of 280
  (179%; the slice 9 ceiling of 430 was passed, and the PRD-15 note now sets **540** with the reason),
  OPERATING 235 of 280, CONNECTOR-SCHEMA 115 of 90, `scripts/README.md` 39. No Rust or script line
  count moved: the slice changed `docs/OPERATING.md` and `docs/RECORD-FORMAT.md` only. The Rust and
  Worker rows in the "Rows over their size line" list above are unchanged by this slice.
- **Build order, PRD-15:** the next RECORD-FORMAT edit follows PRD-08 slice 5 (`legs.rs`, merged at
  bfd244e3, not yet in the page) and PRD-08 slice 6 (`history.rs`) if that has merged by then. Slice 10
  was written against slice 4 and is stale on the legs checks (PRD-15 note). The remaining docs slices
  still wait for PRD-02 core, PRD-10 `start` and `continue`, and PRD-12's skill rewrite, as listed.
- **Seams:** none new. The `check --json` keys `harness_log`, `provider_errors`, `threads` and
  `counts.threads` are written by `legs.rs` and `report.rs` now (PRD-08 slice 5); `history` stays null
  until the history check is built.
- **Acceptance lists as known,** if still open at the end: the three nits A9, A10 and A11 (PRD-15 note)
  if the next RECORD-FORMAT edit does not take them, and the 540-line page ceiling as a recorded overage.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-11 slice 6 (merge 211ade46)

- **PRD-11 is done except one data-and-docs touch.** The code is on integration and was not changed by
  slice 6. `wc -l` at 211ade46 against the PRD-00 "Size budget" row (Connector code, 800 src, 400
  tests): src 1,695 (`schema.rs` 423, `launch.rs` 324, `mod.rs` 317, `hints.rs` 303, `hosts.rs` 301,
  `builtin.rs` 27), tests 759 (`connector_launch.rs` 261, `connector_hints.rs` 196,
  `connector_builtin.rs` 155, `connector_hosts.rs` 105, `connector_no_model_address.rs` 42). That is 895
  src and 359 tests over the row, and 5 and 1 under the ledger L2 ceilings of 1,700 and 760 that the
  slice 5 note records. The row was never updated; the over-row amount was named in the PRD-11 lane's
  `decision` lines and the connector set is the 16 data files plus the host list, not a code cost per
  harness. Count the ceilings, not the row, at the final size check, and list the 895 and 359 in the
  report next to the Rust total (the 18,100 and 14,200 totals are then about 900 and 350 low).
- **Add one step to the end-of-build list, after the pad's P7:** one PRD-11 lane refreshes the README
  Tier column (with each reason) and the 15 "Pad result" lines from `connectors/tiers.toml`
  (PRD-11 note items 1 to 3). It is docs only and must land before the final acceptance run's link
  check and before the purge. PRD-12's `table` does not do it.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-05 core slice (merge 2c95e8f1)

- **Size report, PRD-05** (`wc -l` at 8e886901, row "Masking (PRD-05)", 3,530 src and 2,000 tests): src
  4,030 lines of code, 114% of the row (`stream.rs` 2,327, `rules.rs` 834, `values.rs` 339, `meta.rs` 289,
  `mod.rs` 241; 4,692 with 662 inline test lines; `shapes.tsv` 32 data lines not counted); tests 2,782,
  139% of the row (6 `mask_*` files 1,712, corpus helpers 408, inline 662). The masking lane's decision
  line names points 4 and 3. Per ledger L2 and the 30 Oct size rule, overage alone is a nit; this note
  reports it and decides nothing.
- **Rows over their size line, updated for the end-of-build report:** masking joins connector, usage,
  upload and verifier, adding about 500 src and 780 tests. The sum of the PRD-00 src rows is 18,420; the
  five rows at their ceilings or measured counts (connector 895 over, usage 1,080, upload 1,560, verifier
  2,060, masking 500) add about 6,100, so the Rust src total is about 24,500 if the proxy, CLI and
  pipeline rows (not yet built) end at their rows, against the target of about 18,100 (tests at most about
  14,200). This is the check the PRD-08 slice 5 note asked for at the PRD-05 merge: the targets are the
  owners' estimates and the 30 Oct size rule makes overage a nit, so it goes into the end-of-build report
  as a plain figure and no owner question is asked now.
- **Build order:** with PRD-05 core merged, `lessons_ws.rs` needs only PRD-03 and `common::upstream`; the
  pad's C3 needs only PRD-08's `masks` key; the 50 MB/s mask throughput (A5) is PRD-02's pool, so the
  rung that measures masking in a run is after the pool, not before. Rung 1's stub check (more than one
  entry in `start.mask_rules`) now means something: it must pass on every build from 2c95e8f1 on.
- **Acceptance lists as known,** if still open at the end: the PRD-05 nits (PEM hand-off coverage case,
  the `Walk::absorbs` cut, the bench comment residue) and the question 3 shortfalls (32-value build 3.1 s
  and 1.4 MB/s, `env` and `harness` values counted `skipped`) if the owner answers question 3 no.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-12 slice 6 (merge 645f3219)

- **Gate:** `gate fast` passed in 98 s on the merge. It does not run the old `goalcup-protocol` tests or
  `scripts/test_check_connector.py`, and both read the deleted `connector-harness.toml` (PRD-13 and PRD-15
  notes). A full run before PRD-13 lands will fail those two; that is the known deletion, not a
  regression.
- **Size report, PRD-12** (`wc -l` at 645f3219, PRD-00 row "the pad, about 1,350 to 1,450 with its tests"):
  `scripts/pad/` is 4,096 lines; 3,546 in code files and tests (`pad-run` 993 of 330, `check.py` 743 of
  320, `pad` 664 of 320, `test_pad.py` 488 of 90, `table` 193 of 150, `selftest` 182 of 100, `sbx.mjs`
  133, `exec/` 96 of 70, `image` 54 of 60), 440 in harness files and snippets (11 connectors), 176 in the
  skill and templates, 79 in prompts, runs and `image.id`. The slice 7 work (five harness files, the
  queue, fixtures) comes on top. The decision line of the PRD-12 note names points 1, 3 and 7. The
  end-of-build report lists PRD-12 at its final size against its row; whether the row rises stays the
  owner question already logged.
- **Pad rows now possible by file:** the six faults of P11 have their expectations and code, so
  phase 4 can schedule them once a `goalcup` build with PRD-02 core, PRD-03 serve and PRD-10 `start` is on
  integration. Pad files cover 11 of 16 connectors; the other five (`cline-cli`, `crush-cli`, `droid-cli`,
  `goose-cli`, `kimi-code-cli`) are PRD-12 slice 7 and must precede any tier refresh row for them. A
  `cut-worker` row can end "not run: cut did not take"; count it as not run for PRD-06 U3 and U5, not as
  a pass, and list it with the open acceptance lines if the resolver ignores `/etc/hosts` in the build.
- **`selftest` has never run a control on a real fixture.** The K2 builder is on integration, but the
  fixtures are not committed and K7 and K11 wait on PRD-08 slice 6 and PRD-09. P1 ("the rig is trusted")
  stays open until slice 7 commits the fixtures and `selftest` exits 0 in a build box.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-11 slice 7 (merge 1c719d16)

- **The end-of-build step added by the slice 6 note (README Tier column and Pad result lines) has landed.**
  Remove it from the list. What stays is the final acceptance run's link check, which was not re-run after
  the last push of that slice.
- **Size report, PRD-11** (wc -l at 1c719d16, PRD-00 row "Connector code (PRD-11)", 800 src and 400 tests):
  src 1,695 (schema.rs 423, launch.rs 324, mod.rs 317, hints.rs 303, hosts.rs 301, builtin.rs 27), tests
  759. That is 895 and 359 over the row and 5 and 1 under the ledger L2 ceilings of 1,700 and 760. The slice
  changed no code, so the figures equal the slice 6 note. Count the ceilings at the final check and list
  the 895 and 359 beside the Rust total.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-09 slice 5 (merge 52922c12)

- **Size report, PRD-09** (`wc -l` at 52922c12, row "Usage reader (PRD-09)", 1,470 src and 200 tests):
  src 2,861 (195% of the row, 112% of the 2,550 ceiling), tests `usage_run1.rs` 676 (338% of the row,
  130% of the 520 ceiling). The overrun is the recorded slice 5 decision (`sqlite.rs` 393 against 150:
  WAL needs real files, checksum repair, column probing). Nothing else is planned for usage src. The
  end-of-build report nets this against the Rust totals; only a total past about 18,100 src or about
  14,200 tests becomes the owner question.
- **Build order:** PRD-09 slice 6 is the job script only; the CSV and `--text` are cut (PRD-09 note).
  The PRD-09 acceptance list as known, if still open: the A5r-2 test nit and the `out.rs` taste nit.
- Box results: fmt, clippy `-D warnings`, nextest 64 of 64 on the slice; `gate full` not run here.
- No locked ledger item is touched and no owner question comes from this slice.
