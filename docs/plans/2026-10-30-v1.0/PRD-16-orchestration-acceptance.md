# PRD-16: Overnight orchestration and acceptance

Id: PRD-16. Version: goalcup 1.0. Status: plan for the overnight build. Date: 30 October 2026.
Base: main at 4b226de2. Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md).

## Goal

Run one night in which many agents build goalcup 1.0 from PRD-01 to PRD-17, prove it on real harnesses
in Vercel Sandboxes, and leave the owner one page in the morning that says "ready to merge" or "not, because".
Nothing reaches `main`, a tag or production without the owner.

## Points served

All seven, as the gate that decides whether the build serves them. 1 and 2: the pad and the acceptance
runs compare the wire and the session files with the harness's own log. 3: the kill drill and the load
runs. 4: secret scans of every record, server copy, log and pushed repository. 5: server copy checks on
staging. 6: counts the participant saw against the verifier's recount. 7: 15 harnesses run the way people
run them, gateways included.

## Owner decisions it carries out

- One overnight build, many parallel agents, shipped as **goalcup 1.0** (BRIEF3, 29 Oct evening and night).
  The relayed answers "core OK" and "version 1 sure" confirm the core set (Claude Code, Codex, Hermes) and
  the version. "I can fund one" is read as the one funded Google key (BRIEF3, 29 Oct night). "2 swp" is read
  as "the owner may swap harnesses in the list" (unverified; see Asks).
- Option B, one process; fresh core; old crates deleted in the same build after PRD-17's lessons pass.
- One system for every harness: a gateway is one more host on the one host list, so the gateway case uses the
  same proxy and the same checks (PRD-00, "One system for every harness"). Dodgers get tunnels, session files,
  a named gap and an honest label. This PRD proves that answer with real runs (acceptance A3 and A4).
- Real API runs in Vercel Sandboxes; watcher agents read failures and fix them; three failed fixes on one
  cause end that loop with a note.
- All overnight runs use the **staging** Worker; the test approval route (`TEST_APPROVAL_KEY`) approves with
  no browser. Agents may deploy the Worker to staging. Production deploy, production checks and the merge to
  `main` are the owner's. No website database schema migration overnight (BRIEF3, 30 Oct early).
- Spend: 10 USD per run. Pad runs are about 20 minutes of real work with many subagents; during the build,
  tests stay short. Cost is not a constraint for test runs; keys and budget are the owner's (BRIEF3).
- Autonomy, keys in 0600 files, separate keys per harness, read-only database and R2 access, macOS by the
  owner after the build (PRD-00 defaults).
- The long run uses run 1's Codex prompt, so the two runs compare.

## Today

| What | Where | Fact |
|---|---|---|
| Workspace members and version | `Cargo.toml:3-9`, `:18` | Five old crates; version `0.7.0`, the one place a version is written (`:12-17`) |
| Toolchain | `rust-toolchain.toml` | `1.99.0`, minimal profile, clippy and rustfmt |
| A dev profile for a crate that goes | `Cargo.toml:61` | `[profile.dev.package.goalcup-daemon]` |
| The gate refuses root | `scripts/gate.sh:32-39` | Some permission tests always fail as root. This container runs as `root` with 4 CPUs (`whoami`, `nproc`) |
| Release script | `scripts/release.sh:4-14`, `:56`, `:61-66`, `:74` | `prepare` opens a version PR (main is protected); `tag` tags `origin/main` only when its version matches; the next-step text names `linux-check.sh` |
| Version floor | `server/worker/wrangler.toml:51-55` (production), `:150` (staging); `server/worker/src/runs.ts:149-164`, `:228`; `device.ts:55` | `MIN_COMPETITION_VERSION = ""` in both; checked for competition starts only today. A dashboard edit is wiped by the next deploy |
| Staging Worker | `server/worker/wrangler.toml:107-164` | `goalcup-ingest-staging`, own bucket `goalcup-traces-staging`, own events, the only deployment that may hold `TEST_APPROVAL_KEY` (`:87-88`, `:108-109`), mail to Resend's test inbox (`:159-160`) |
| Event switch | `server/worker/wrangler.toml:36-42` | `EVENT_ID` becomes `goalcup-cup-2026` between 6 Nov 00:00 and 7 Nov 00:00 UTC, by the owner |
| Maintenance switch | `server/worker/wrangler.toml:68-70` | `MAINTENANCE` on holds clients, who keep their data and back off |
| Plan schedule | `docs/plans/2026-10-30-cut-first-plan.md:364-475` | Day-by-day PRs, a second 2-hour run on Monday 2 Nov, a release-candidate run on Wednesday 4 Nov. Superseded by one night |
| Run 1 | run report (scratchpad copy `run-report-74daaa03.md`), on `origin/smart/tender-clarke-qupne3` | 2 hours, Codex 0.165.0, about 20 subagents at once, 42 in all; wire stopped for good at 15:43:15Z; 473 calls (37.5%) and 15 of 42 subagents with no wire record; finish line said 1,736; server stored 1,766; 405 MB uploaded at `-3`, 100.8 MB at `--long=27`; about $1.50 by Codex's own record |
| Run 1 prompt | `docs/research/results/2026-10-30-two-hour-run/` on that branch | **Not in the repository.** `start-pane.txt` shows only the Codex banner; the report says only "as written by the owner" |
| Where the PRDs live | branch `smart/intelligent-newton-0avrpi`, 10 docs commits ahead of `main` 4b226de2 | The integration branch starts from `main`, so it has none of the PRDs until they are merged in |
| Agent concurrency | the workflow runner's own limit | At most min(16, CPUs minus 2) agents at once per workflow. On this 4-CPU container that is **2** |

## Design

### 1. Hosts and roles

| Host | What runs there | Holds |
|---|---|---|
| **Conductor host** | The night's one workflow script, the integrator, reviewers, the scribe, watchers (as agents), git worktrees, `scripts/pad/pad` (host half), `fetch-run`, `goalcup-verify` | The approval key, read-only database and R2 credentials, the GitHub token, the Cloudflare token for staging, all key files under `~/.pad-keys/` (0600). Never a production write credential |
| **Build boxes** | Vercel Sandboxes, 8 vCPU and 16 GB (the Pro maximum, `docs/research/VERCEL-COMPAT.md:50`), user `ubuntu`, from a build snapshot | The Rust toolchain from `rust-toolchain.toml`, nextest, zstd, Node, Python, PostgreSQL, a warm cargo cache. No key, no token |
| **Pad boxes** | Vercel Sandboxes from PRD-12's pad image, 4 vCPU, `--non-persistent` | One harness, its key file for the life of the run, goalcup and goalcup-verify of the build under test |
| **Staging Worker** | `goalcup-ingest-staging` on Cloudflare, 1.0 code from the integration branch | Its own bucket; the shared database through the Worker's upload functions only |

Why build boxes: the gate refuses root (`scripts/gate.sh:32-39`), the conductor runs as root, and 16
builders compiling a TLS, HTTP/2 and zstd stack on 4 shared CPUs would queue for hours. A build box is
non-root, has 8 CPUs, and is thrown away. Code goes in as a `git bundle` with `sandbox copy`, so no token
ever enters a box. JUnit and the gate's last line come back out.

**Conductor size.** The night needs about 16 agents at once in phase 1 (one per PRD) and about 8 in
phases 4 to 6 (integrator, reviewer, scribe, up to 6 watchers, one per running pad box). The runner allows
min(16, CPUs minus 2). Decision: run the conductor on a host with at least 18 CPUs. At H+0 the conductor
reads `nproc`. If it is under 18, builders run instead as separate cloud sessions, one per PRD, each in its
own container, started with the session tool and told their PRD, branch and the build-box rule; the
workflow drives them with messages and reads their state. Watchers and the integrator stay in the workflow.
The night log records which mode ran.

Roles:

| Role | Count | Does | Never |
|---|---|---|---|
| Conductor (the script) | 1 | Phases, timers, queues, caps, the wake list. Deterministic control flow | Writes code |
| Integrator | 1 | Phase 0 setup, merges, `gate fast` on every merge, `gate full` at phase exits, staging deploys, the build snapshot | Merges a branch that fails the rule in section 4 |
| Builder | one per PRD (01 to 15, 17) | Builds its PRD on `v1.0/prd-NN-<slug>` in its own worktree, inside its owned paths, runs its own tests in a build box | Edits a path its PRD does not own |
| Reviewer | 1 at a time | Reads each PRD branch's diff once before merge (AGENTS.md:105, "one reviewer reads the PR once") | Rewrites the code; it returns findings |
| Watcher | one per failing pad row, at most 6 at once | PRD-12 section 13: read, name the cause, reproduce, fix on the owning branch, rerun | Changes a check to make it pass without record evidence (PRD-12 section 13) |
| Scribe | 1 | Keeps `NIGHT-LOG.md` and writes `MORNING.md` and `ACCEPTANCE.md` | Copies a key, token or planted value into any file |

### 2. Phase 0: what the integrator does first (H+0 to H+1.5)

In this order. Every step is a commit on `v1.0/integration`, merge commits only, never a force push.

1. Branch `v1.0/integration` from `main` at 4b226de2 and push it. Pushes go only to `v1.0/*` branches.
2. Merge `smart/intelligent-newton-0avrpi` (docs only: the plan, the audit appendices, the PRDs). Check
   with `git diff --stat` that the merge touches nothing outside `docs/plans/`. Builders read the PRDs from
   the integration branch.
3. **PRD-14's gate change lands first** (PRD-14 ask 2): `scripts/check-version.py` is red today on
   `docs/plans/`, so no merge rule could pass before it.
4. **Workspace edit (this PRD's file)** in one commit:
   - `[workspace.package] version = "1.0.0"` (`Cargo.toml:18`). Every binary built tonight says
     `1.0.0`, so the staging floor of 1.0.0 lets them in.
   - `members` gains `crates/goalcup-format`, `crates/goalcup`, `crates/goalcup-verify`; the five old crates
     move from `members` to `exclude`. They stay on disk for PRD-17 and the copy-ins, and they are never
     built, linted or tested again. This settles PRD-13 asks 1 and 3 (the binary name collision) and PRD-14
     ask 4 with no rename and no `--exclude` line.
   - Dev profile: opt-level 3 for `zstd-sys`, `sha2`, `brotli-decompressor` and the deflate backend
     (PRD-14 section 4 and ask 8); delete `[profile.dev.package.goalcup-daemon]` (`Cargo.toml:61`).
   - `rust-toolchain.toml` unchanged.
5. Wait for PRD-01's format types and PRD-02's scaffold (both due by H+1.5, PRD-00 seams), then regenerate
   `Cargo.lock` once with `cargo generate-lockfile` in a build box and commit it. After that, any new
   dependency goes through PRD-02's `crates/goalcup/Cargo.toml` with a night-log note, and the integrator
   updates the lock in the merge.
6. Build the **build snapshot** (toolchain, nextest, zstd, Node 22, Python 3, PostgreSQL, `npm ci` for
   `server/worker`, a `cargo fetch` of the lock) and ask PRD-12 for the **pad image** (PRD-12 section 10).
   Rebuild the build snapshot at each phase exit so later builds start warm.
7. Create `NIGHT-LOG.md` with the start line: commit, conductor mode, CPUs, caps in force, the owner's
   overrides if any.

Exit gate (PRD-00): `cargo build` and `cargo test -p goalcup-format` green in a build box on integration.

### 3. Phase 1: parallel builders (H+1.5 to H+6)

- Each builder starts with: its PRD, PRD-00, PRD-01, the seams it uses, and the builder notes of its PRD.
  It works in its own git worktree on `v1.0/prd-NN-<slug>` from integration.
- **Build-box rule.** A builder never runs cargo on the conductor. It sends a bundle to its own build box
  and runs `cargo nextest run -p <its crate> -E <its tests>`, then `gate fast` before asking to merge.
- **Merge requests.** A builder asks to merge whenever a slice is green, not only at the end. Small merges
  early keep seams honest. The first merges are seams as stubs: S1, S2, S3, S8 by H+1.5; S4, S5 by H+2;
  S6 by H+3; S7 by H+4 (PRD-00 seams table).
- **Seam change.** Only the seam's owner changes a seam, with a night-log line `seam S<n>: <change>,
  users <PRDs>`. Users rebase within the hour.
- **Vendor docs first.** Every builder of a harness-specific part reads the vendor's current docs before
  building it and cites the page in the commit message (BRIEF3, 30 Oct early).
- **PRD-17 starts at H+0**, read only, and publishes `LESSONS.md` by H+3; `tests/lessons_*.rs` follow as
  the core lands.
- **PRD-07 starts at H+0** (it needs only PRD-01 section 12).

**Phase 1a, the early real test.** As soon as PRD-01, 02, 03 and 05 are merged and record one request, a
watcher runs PRD-12's P2: Claude Code and Codex, `direct`, `--prompt smoke`, `--server off`, in pad boxes.
Then one short real run after each later core feature lands (native capture, uploader, watchdog), so a broken
concept shows before the whole build is done (BRIEF3, "early quick real tests").

**First staging deploy.** As soon as PRD-07 and PRD-06 are merged, the integrator deploys staging:
`wrangler deploy --env staging` from the integration head, with `MIN_COMPETITION_VERSION = "1.0.0"` in
both blocks of `wrangler.toml` (PRD-07 section 6). From then on, smoke runs use `--server staging` and
check the server copy. Every staging deploy writes a night-log line with the commit and the Worker version id.
A 0.7 client used by anyone else on staging is refused from then on; that is test data and is said in the
morning report.

### 4. The merge rule (PRD-00, made concrete)

The integrator merges a PRD branch into `v1.0/integration` when all hold:

1. `git diff --name-only integration...branch` lists only paths that PRD owns (PRD-00 ownership map), or a
   seam change the seam owner approved in the night log.
2. The PRD's own tests pass in a build box.
3. `gate fast` passes on the merged result in a build box (non-root, `scripts/gate.sh:32-39`), and the last
   line `gate fast: PASS in <N>s` goes into the night log.
4. The reviewer read the diff once and found no blocker. Blockers are only: a path outside ownership, a
   hard-rule break (AGENTS.md:81-95 as PRD-00 restates them), a byte written to disk before masking, a drop
   with no GAP, a harness name in core code, or a secret in a fixture. Style is not a blocker.

Merge commits only. A conflict is fixed by the owner of the conflicting file. The push of each merge runs
CI `check` (PRD-14 section 9), an independent record; a red CI on integration stops further merges until it
is green or the night log names it as a runner fault.

### 5. Phase 2 and 3: integration and the cut (H+6 to H+9)

- **Phase 2 exit:** `gate fast` green; PRD-17's lessons tests green on the new core; the pad end to end
  against staging for Claude Code `smoke`; the kill test (PRD-01 acceptance 3, PRD-02 R1) on Codex
  `smoke`; PRD-12's `selftest` makes the pad trusted (PRD-12 section 12, P1). `gate full` timed (PRD-14 G1).
- **Phase 3, the cut:** PRD-13's branch, merged together with PRD-14's gate, CI and packaging edits and
  PRD-15's map and docs, in one merge (PRD-13 section 5). The integrator makes the matching workspace edit in
  the same merge: drop the `exclude` entries and every workspace dependency only the old crates used (checked
  with `cargo tree`), then refresh `Cargo.lock`. Exit: `gate full` green, PRD-13's reference guard clean,
  CI green. PRD-14 G2 (sabotage) and G3 (five clean runs) run here.
- If the lessons are not green by H+9, the cut does not happen. Old crates stay on disk, out of the
  workspace, and the morning report says so. Phases 4 to 6 still run, so the owner sees real results.

### 6. Phase 4, 5 and 6: real runs (H+9 to about H+14)

All runs use PRD-12's pad on staging, with the binaries built from the integration head in a build box.
The commit under test is written in every row. A fix merged during a phase starts a rerun of the rows it
can touch; the release check (PRD-14 section 9) later demands that the tagged commit equals the pad's
commit under `crates/`, `server/worker/src/`, `connectors/` and `Cargo.lock`.

| Phase | Runs | Parallel | Est. time |
|---|---|---|---|
| 4. Core set | Claude Code, Codex, Hermes x `direct`, `gateway`, `dodger`, `--prompt full`: 9 runs | 6 boxes, at most 2 per provider key (PRD-12 section 7) | about 1 h for two waves, plus fix loops |
| 5. The rest | The other 12 harnesses, each case that applies; no key means "not verified: no key" with no run (Amp, Factory Droid, OpenRouter cases) | 6 boxes, starts when phase 4's first wave is done and keys are free | about 2.5 h |
| 6. Acceptance | A1 to A4 (section 7) | Starts when the core set is green; shares the box limit with phase 5; A1 and A2 never share a key with a phase 5 run | about 2.5 h, A1 is the longest |

Rate limits: run 1 hit 2M tokens a minute on one OpenAI key with about 20 subagents (run report, measure
9). So A1 holds the OpenAI key alone while it runs; phase 5 runs that use the OpenAI cheap-model key use a
second OpenAI key if there is one, or wait.

**Watchers** follow PRD-12 section 13 exactly. A row that is FAIL gets one watcher, keyed by the signature
`<connector>|<case>|<check>|<first finding>`. INCONCLUSIVE rows are rerun once. A fix goes on the owning
PRD's branch in a worktree; the integrator merges it under section 4; the watcher reruns.

### 7. Final acceptance runs

Four real runs on staging after the core set is green, each longer than a pad run, each with the same
checks as the pad (C1 to C6, PRD-12 section 8) plus the extra lines below. The pad runs them with
`--prompt-file`, `--minutes` and `--keep-going` (ask of PRD-12, section "Interfaces"). Typing "keep going"
when the agent sits idle is what a participant may do (AGENTS.md:83-84); goalcup itself never does it.

| Id | Harness, case | Prompt | Length | Why |
|---|---|---|---|---|
| A1 | Codex, `direct`, OpenAI key, the same cheapest model and low reasoning setting as run 1, Codex at the pad's pinned version | **Run 1's prompt, word for word** (from the owner, see Asks), typed as one first message | 2 hours from the paste; "keep going" after 3 idle minutes | The same run as run 1, so every number compares |
| A2 | Claude Code, `direct`, Claude key | The pad's `full` prompt, with "use up to 20 subagents in parallel" in plain words | 60 minutes; one `kill -9` of the recorder at minute 30 | 20 subagents at once, and a restart under load |
| A3 | Hermes, `gateway` (Vercel AI Gateway, its built-in `ai-gateway` provider, PRD-12 section 2), chat TUI | The pad's `full` prompt asking for many subagents | 60 minutes | The owner's gateway question at length: one host list, same proxy, no gateway code |
| A4 | Kilo CLI, `gateway` (Vercel AI Gateway as an OpenAI-compatible provider, PRD-11:41-43) | The pad's `full` prompt | 20 minutes | The owner's own example, "AI Gateway under Kilo". Not a release blocker |

Pass lines beyond C1 to C6:

| # | Check | Pass |
|---|---|---|
| X1 | Wire against the harness, by time | Every 10-minute bucket: wire requests within PRD-08's tolerance (max(3 calls, 2%), 2-minute lag) of the harness's own calls. No bucket at 0 while the harness made calls. A1 compares against run 1's table (run report, section 4) |
| X2 | Subagents | Every subagent file on disk has a native stream, and every thread has wire calls. A1: at least 20 subagents spawned (run 1: 42); A2: at least 15, with 10 or more running at once. Fewer spawned is INCONCLUSIVE for load, never PASS |
| X3 | No loss under normal load | Zero `memory_guard` gaps in A1 and A3. One is a FAIL even though it is named: run 1's load is the normal load. Its guard name, value and peak `queue_bytes` go to PRD-02 |
| X4 | Restart | A2: exactly one `recorder_down` gap, start before end, under 5 s; the harness never noticed (no failed model call in its log in that window); `goalcup status` showed the off time; links verify across both boots |
| X5 | Upload keeps up | PRD-06 U2: at every minute the server is at most 6 minutes or 64 MiB raw behind the durable point; no wait over 120 s; after the seal, done within 2 minutes |
| X6 | Server copy | `fetch-run` from staging (read only); concatenated chunks equal the concatenated local parts (same SHA-256); `goalcup-verify check --server --local` has no `fail`; the database row's `model_requests` equals the written count; heartbeats say `failed` only for an off span of 60 s or more (PRD-07 section 4), checked in a `BEGIN READ ONLY` transaction |
| X7 | Secrets | `goalcup-verify secrets` with the known-values file: 0 hits in local parts, server copy, logs, the night log and the pushed repository |
| X8 | Usage | `goalcup-verify usage` on the server copy: tokens and model for every call, one row per subagent, `unattributed` 0 (PRD-09). A1: totals against run 1's (Codex's own record: 117.4M input, 410.6k output) as a ratio per hour |
| X9 | Size and memory | A1: raw and compressed bytes per hour, framing share under 2% (PRD-01 acceptance 4), against run 1's 405 MB uploaded in 49 recorded minutes; recorder RSS minus held bytes under 260 MiB (PRD-06 U7) |
| X10 | Counts on screen | The end screen's number equals the verifier's recount and the server row. No "recording works" line with any gap |

A1, A2 and A3 decide the morning's recommendation (section 10). A4 answers the owner's question either way.

### 8. Autonomy rules

Agents decide anything inside the seven points, AGENTS.md and the PRDs. In particular they may, without
asking: merge into `v1.0/integration` by section 4; push `v1.0/*` branches; deploy the Worker to staging;
create sandboxes; create public, empty GitHub repositories for pad runs (prefix `gc-pad-`); create a separate
R2 bucket for test runs if PRD-07 asks for one; change a default marked "PRD-16" in another PRD with a
night-log line; mark a harness "not verified" with a reason.

They stop that item and leave a night-log note (the night goes on) for:

| Stop | Examples |
|---|---|
| A hard-rule change | Setting a base URL from goalcup, writing into `~/.claude`, patching a harness, disabling pinning, any AGENTS.md rule beyond PRD-00's two changes |
| The website database | Any schema change, any write outside the staging Worker's own calls |
| Production | Any deploy, secret change or request to `ingest.agentcommunity.org` or bucket `goalcup-traces`; an approval bypass on production |
| `main` and tags | A merge to `main`, a tag, a release, a version PR |
| Spend past a cap | Section 9 |
| `VISION.md` | Never edited |

Commits and pushes are allowed only on `v1.0/*` branches, as part of this build (the owner's overnight
decision is the request). Commit messages end with the session's attribution lines.

### 9. Spend caps

| Cap | Value | On reaching it |
|---|---|---|
| Per pad run | 10 USD `full`, 2 USD `smoke` (owner; PRD-12 section 7) | `pad-run` seals at 80% of the live estimate; the row says `stopped_at_cap`; not a failure |
| Per acceptance run | 10 USD each (the owner's per-run cap). A2 with 20 subagents may reach it before 60 minutes | Seal at 80%; checks run on what was recorded; if X2 is INCONCLUSIVE for that reason, the morning report asks the owner for a higher cap and a rerun |
| Per night, model spend | 400 USD by the pad's ledger estimate | No new run starts. Running runs finish. A night-log line; the morning report shows it |
| Per night, sandboxes | 150 USD estimated at Pro rates ($0.128 per active CPU-hour, $0.0212 per GB-hour, `VERCEL-COMPAT.md:52`) | No new build box; builds queue on the boxes that exist |
| Boxes at once | 6 pad boxes, at most 2 per provider key; build boxes one per active builder plus one for the integrator | A queue in the conductor |

Why 400: phase 1a and selftest about 35 USD; phase 4 with one rerun per row up to 180; phase 5 about 25
runs on DeepSeek, expected 1 to 2 USD each, at most 250 if every one hit its cap; acceptance up to 40.
The expected total is well under half of that. The ledger is an estimate; the provider dashboards are the
truth (PRD-12 section 7). Agent tokens of the night itself are not in these caps; the workflow's token
budget applies if the owner set one, and the morning report states tokens used.

### 10. Stop rules

| # | Rule | What happens |
|---|---|---|
| S1 | Three failed fixes on one signature (PRD-12 section 13) | That loop ends. The row stays FAIL, the tier is "not verified", the night log holds the three attempts. For the core set this is a release blocker in the morning |
| S2 | A phase exit gate missed by 2 hours | The scribe notes it. The conductor narrows: phase 5 harnesses first become "not verified: no time", then A4 is dropped. The core set and A1 to A3 are never dropped for time |
| S3 | Morning time (`MORNING_AT`, default H+14) | No new run starts. `MORNING.md` is written from what is done; a run still going is listed as running, and the scribe updates the file when it ends |
| S4 | A night cap | No new run or box starts (section 9) |
| S5 | A stop of section 8 | That item waits for the owner; everything else goes on |
| S6 | A wake event (section 11) | Every pad box is sealed and removed; no new run starts until the owner answers |
| S7 | Lessons not green by H+9 | No cut (section 5) |
| S8 | Build boxes fail to start three times in 10 minutes (a Vercel fault) | Builds fall back to one at a time on the conductor as a non-root user under PRD-14's build lock; a night-log line |
| S9 | A red CI `check` on integration | No further merges until it is green or named as a runner fault |

### 11. What wakes the owner

A push notification to the owner's phone, one per cause, with a top line in `NIGHT-LOG.md`. Only these:

| # | Event | Why it cannot wait |
|---|---|---|
| W1 | A real secret (a provider key, the GitHub token, the approval key, a Cloudflare or database credential) found in any record, server copy, log, night log, pushed repository or commit | Only the owner can rotate it. All runs stop (S6) |
| W2 | Anything touched production: a request from a box or agent to the production Worker or bucket, a production deploy, a production secret change | Participants could be affected. All runs stop |
| W3 | The shared website database looks hurt: staging Worker database errors (5xx from the database, not 4xx from the Worker's own rules) more than 20 times in 10 minutes | The website shares it |
| W4 | Spend out of control: the ledger or a provider dashboard past 1.5 times a night cap, or a sandbox that will not stop | Money keeps leaving |
| W5 | A core-set key refused by its provider (401 or 403 on the first run) | The owner can fix it in minutes and save the night; otherwise the core set cannot pass |

Nothing else wakes the owner: failing checks, fix loops, late phases and missing keys for other harnesses
all wait for the morning.

### 12. The night log

`docs/plans/2026-10-30-v1.0/NIGHT-LOG.md`, append only, committed on integration by the scribe at least
every 30 minutes. One line per entry:

```
<HH:MM>Z | <role> <PRD or harness> | <kind> | <plain text, links to row.json, gate line, CI run, commit>
```

Kinds: `start`, `decision`, `seam`, `merge`, `gate`, `deploy`, `run`, `fix`, `stop`, `wake`, `spend`,
`check changed`, `owner question`. No key, token, planted value, model name or model id. A line that quotes
an error copies only the error code and the plain words, never a request body.

### 13. The morning report

`docs/plans/2026-10-30-v1.0/MORNING.md`, one screen first, details after. The owner reads it with coffee.

1. **One line:** "Ready to merge `v1.0/integration` at `<commit>`" or "Not ready: <the one main reason>".
   Ready means: PRD-00's definition of done items 1 to 7 hold, and A1, A2 and A3 pass.
2. **Core set:** the 9 rows (harness x case), each PASS or FAIL with its `why` line.
3. **Acceptance:** A1 to A4 with X1 to X10, and A1 next to run 1: wire calls against harness calls, subagents
   recorded, gaps and off time, raw and compressed bytes per hour, upload lag, recorder RSS, spend.
4. **The 15 harnesses:** the tier summary from `PAD-RESULTS.md` (PRD-12 section 9), with versions.
5. **The gateway answer:** the gateway rows (core set, A3, A4): same host list, same proxy, no gateway code,
   in two sentences with links.
6. **Blockers:** each with the owning PRD, the signature, the three attempts if S1 ended it.
7. **Decisions agents took** that the owner may reverse (from `decision` and `check changed` lines).
8. **Owner questions** (from `owner question` lines), each with the default the night used.
9. **Release checklist** (section 14) with the commit filled in.
10. **Spend and resources:** model spend by provider (estimate), sandbox cost, tokens used; every GitHub
    repository created (to delete), staging runs made, the staging Worker version, and a line that every
    sandbox was removed and every key file wiped.

`ACCEPTANCE.md` (same folder) holds the A1 to A4 prompts as run (with run 1's prompt only if the owner
agrees it may be in the repository), each run's row, its X lines, and the commands to repeat it.

### 14. Release steps (the owner's, in the morning)

1. Read `MORNING.md`. Answer the owner questions or accept the defaults.
2. Check that no real run is open on production (PRD-07 section 12 step 2, the read-only query).
3. Merge `v1.0/integration` into `main` (merge commit). If `main` moved since 4b226de2, the merge result
   differs from the verified commit: the integrator reruns the core set `direct` on it before the tag, or the
   release check refuses (PRD-14 G6).
4. `sh scripts/gate.sh full` on `main`, as a non-root user.
5. `sh scripts/release.sh tag 1.0.0`. The version is already `1.0.0` on `main` from phase 0, so no `prepare`
   PR is needed (`scripts/release.sh:56`). PRD-14's release check runs first: the core set PASS on the same
   commit with a trusted pad.
6. `release.yml` publishes the four targets. Check `install.sh` from `releases/latest` on Linux and on the
   owner's Mac (PRD-14 G7), and run the pad once on the Mac (PRD-12 P9).
7. **Production Worker, from the owner's own device, right after step 6:** `wrangler deploy` from the
   tagged commit. `MIN_COMPETITION_VERSION = "1.0.0"` is already in `wrangler.toml` (PRD-07 section 6), so
   from this deploy every mode refuses a 0.7 client. The floor goes last, so nobody is refused while 1.0
   cannot be downloaded (PRD-14 section 9). Between steps 5 and 7 a 1.0 client cannot use the 0.7 Worker;
   keep that window to minutes. Then delete the `RECEIPT_SIGNING_KEY` secret (PRD-07 section 12 step 5).
8. Production smoke: one practice run with 1.0, sealed, fetched, checked (PRD-07 section 12 step 6).
   Optional: repeat A1 on production for a production proof.
9. `EVENT_ID` switch between 6 Nov 00:00 and 7 Nov 00:00 UTC (`wrangler.toml:36-42`), the owner's deploy.
10. On 5 Nov, rerun the core set `direct` on the harness versions current that day (PRD-12 risk "a verified
    version is not the one people run"). A new failure there is a fix-forward release `1.0.1`.

No rollback to the 0.7 Worker once 1.0 clients run (PRD-07 section 12). A bad 1.0 Worker is held with
`MAINTENANCE` on and fixed forward.

## Files it owns

Change:

- `Cargo.toml` (workspace): version, members and `exclude`, dev profiles, workspace dependencies at the cut.
- `Cargo.lock`: regenerated at phase 0 and at the cut; updated by the integrator at merges that add a
  dependency.
- `rust-toolchain.toml`: unchanged unless a builder shows a need, with a night-log line.

Create:

- `docs/plans/2026-10-30-v1.0/NIGHT-LOG.md`
- `docs/plans/2026-10-30-v1.0/MORNING.md` (ask of PRD-00)
- `docs/plans/2026-10-30-v1.0/ACCEPTANCE.md` (ask of PRD-00)

No script in the repository. The night's workflow script lives where the workflow tool saves it; the night
log names its path and run id. Build-box and staging commands are run by the integrator by hand from this PRD.

Must not touch: every path another PRD owns, in particular `scripts/pad/**` and `PAD-RESULTS.md` (PRD-12),
`server/worker/**` including `wrangler.toml` (PRD-07), `scripts/gate.sh`, `scripts/release.sh` and
`.github/**` (PRD-14), `connectors/**` (PRD-11), `crates/**` (PRD-01 to PRD-10), `VISION.md` (the owner).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-01 | Golden test green is the phase 0 exit; acceptance 3 (kill) and 4 (size) run in phases 2 and 6 | Used here |
| PRD-02 | Kill test R1; status `mode`; `GOALCUP_KEEP_PARTS`; the silent drop hook for selftest (PRD-12 ask 1) | Used here |
| PRD-06 | U2 and U7 lines in X5 and X9; `upload.json` `keep` | Used here |
| PRD-07 | Staging deploy commands; `MIN_COMPETITION_VERSION = "1.0.0"` in both blocks of `wrangler.toml` from the first staging deploy; `fetch-run`; the read-only queries; deploy order (section 12) | Used here; PRD-07 section 12 step 1 changes (Asks) |
| PRD-08 | `check`, `check --server --local`, `check --live`, `secrets`, `usage` entry, tolerances owned by "PRD-16" in its overridable defaults: kept as PRD-08 states them tonight; `conn_bytes` stays `warn` until three pad runs | Used here |
| PRD-09 | Usage summary for X8 and the morning per-thread table | Used here |
| PRD-11 | Connector data and the host list the gateway rows rely on; tiers copied from `pad-out/tiers.json` (PRD-12 ask 5) | Used here |
| PRD-12 | The pad, selftest, rows, `PAD-RESULTS.md`, watchers' loop, ledger. **Asks:** `--prompt-file <path>`, `--minutes <n>` (seal after n minutes; box timeout n plus 30), `--keep-going <idle minutes>` (types "keep going" when the pane and the wire are idle that long), `--kill-recorder <minute list>` | Used here; the four flags are asked of PRD-12 |
| PRD-13 | The cut merge; the workspace edit is made here in the same merge (answers PRD-13 asks 1 and 3) | Done together |
| PRD-14 | `gate fast` and `gate full`; build lock; CI on integration pushes; G1 to G7; the release check in `release.sh tag`. Old crates are out of `members` from phase 0, so `$PKGS` needs no exclude line | Used here |
| PRD-15 | Docs merge in phase 3; README "The gate" numbers from G1 | Used here |
| PRD-17 | `LESSONS.md` by H+3; lessons tests green is the phase 2 exit and the precondition of the cut | Used here |

## What is cut

| Path | What | Lines |
|---|---|---:|
| `Cargo.toml:3-9` | The five old members (to `exclude` at phase 0, gone at the cut) | 5 |
| `Cargo.toml:61-62` | The daemon's dev profile | 2 |
| `Cargo.toml:64-132` | Workspace dependencies used only by the old crates, found with `cargo tree` at the cut (expected `ed25519-dalek`, `redb`, `axum`, PRD-13 section 5) | unverified until the cut |
| `docs/plans/2026-10-30-cut-first-plan.md:364-475` | The day-by-day schedule, the Monday 2-hour run and the Wednesday release-candidate run | Superseded by this PRD; the file stays as history (PRD-15) |

No tests go here; the tests of the old crates go with PRD-13.

## Acceptance

### Real-run proofs (first)

| # | Proof | Pass |
|---|---|---|
| N1 | **The core set is green on one commit.** Phase 4 | 9 PASS rows (3 harnesses x 3 cases) from a trusted pad on the same integration commit, every check, the dodger rows with exit 1 and `wire_keeps_up: named` |
| N2 | **The long runs pass.** Phase 6 | A1, A2 and A3 pass C1 to C6 and X1 to X10; A1's table sits next to run 1's in `ACCEPTANCE.md` |
| N3 | **One system for gateways.** The gateway rows of N1, A3 and A4 | `ai-gateway.vercel.sh` is `decrypt` and `listed: shared` in every `conn`; no connector or core line names a gateway; wire matches the harness within tolerance |
| N4 | **Every merge is accounted for.** At morning | Each merge commit on integration has a night-log `merge` line with its `gate fast: PASS` line, a reviewer line and a green CI run |
| N5 | **The release check accepts the verified commit and refuses another.** PRD-14 G6 on the morning commit | Accepts the commit of N1; refuses a commit that differs under `crates/` |
| N6 | **Nothing leaked.** After phase 6 | `goalcup-verify secrets` with the night's known values: 0 hits over every kept record, server copy, `pad-out/`, `NIGHT-LOG.md`, `MORNING.md`, `ACCEPTANCE.md`, every `gc-pad-*` repository and the integration branch history |
| N7 | **Nothing touched production.** After phase 6 | The staging Worker was the only deploy (Cloudflare deployments list); no row with a production event id was created by the night (read-only query); no request to the production host in any box's `goalcup.log` |
| N8 | **The morning report is complete.** At `MORNING_AT` | Every section of 13 is filled, every claim links to a row, a gate line, a CI run or a commit; a reader who was asleep can decide in 10 minutes |

### Automated tests that guard them

None new. This PRD writes no code. Its guards are other PRDs' tests: PRD-12 `selftest` and `test_pad.py`
(the pad cannot pass a known fault), PRD-14 `test_gate.py` and `test_release.py` (the gate and the release
check), PRD-08's check tests, and CI on every integration push.

## Risks and blind spots

| Risk | What we do |
|---|---|
| The conductor host has too few CPUs and the night crawls (2 agents at once on 4 CPUs) | Section 1: an 18-CPU host, or builders as cloud sessions. Whether cloud sessions can push to the repository is unverified; check it at H+0 with one branch push |
| 16 builders on one repository step on each other | One owner per path, worktrees, seams as stubs first, merges only through the integrator |
| A seam changes late and three PRDs break at H+5 | Seam changes only by the owner with a night-log line; users rebase within the hour; S2 narrows scope before the core set is touched |
| Vercel Sandbox limits: concurrent boxes, session length, egress | Concurrency for 6 pad boxes plus up to 17 build boxes is unverified; the conductor queues. A1 needs a box of about 2.5 hours: Hobby sessions end at 45 minutes, Pro at 24 hours (`VERCEL-COMPAT.md:41`), so check the plan at H+0. Never turn on credential brokering for a model host (`VERCEL-COMPAT.md:46`) |
| Staging is shared by other people's tests | The 1.0 floor refuses 0.7 clients on staging from the first deploy; said in the morning report. Runs are told apart by run and entry id |
| The staging Worker writes into the shared website database | Only through its own upload functions, with today's tables (PRD-07 section 11). W3 wakes the owner if the database looks hurt |
| A provider outage looks like 15 failures | PRD-12's `provider_down` and `provider_refused` causes come first; W5 only for a core key |
| A watcher "fixes" a check instead of the code | PRD-12 section 13 rule; `check changed` lines in the night log; `selftest` again before any row is trusted |
| A green pad at 3 a.m. and a different harness version on 7 Nov | Rows name versions; connectors turn auto-update off; the 5 Nov rerun (section 14 step 10) |
| Run 1's prompt is not available | A1 cannot compare word for word. Default: if the owner has not supplied it by H+9, A1 runs the pad's `full` prompt for 2 hours and the morning report says the comparison is by load, not by prompt |
| A2's 10 USD cap ends it before 20 subagents run | X2 INCONCLUSIVE, not PASS; the morning report asks for a higher cap |
| The kill in A2 coincides with a burst and costs more than the gap shows | X4 checks the harness's own log for failed calls in that window; PRD-02 owns the watchdog |
| Linux only | The owner's Mac run in the morning (section 14 step 6); macOS-only paths (clock, CA store, socket passing) run in PRD-14's release workflow on each target |
| The night log grows a secret by quoting an error | Section 12 rule; N6 scans it |
| The morning report is too long to read | Section 13 puts the decision on the first line and the core set second |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Conductor host | at least 18 CPUs, else builders as cloud sessions | Owner |
| `MORNING_AT` | H+14 | Owner |
| Night model spend cap | 400 USD estimate | Owner |
| Night sandbox cap | 150 USD estimate | Owner |
| Per acceptance run cap | 10 USD (the owner's per-run cap) | Owner; 40 USD suggested for A2 |
| A1 length, A2 and A3 length, A4 length | 2 h, 60 min, 20 min | Owner |
| Kill in A2 | one, at minute 30 | PRD-02 |
| Idle before "keep going" | 3 minutes | Owner |
| Phase-miss narrowing | after 2 hours late | Owner |
| Wake list | W1 to W5 | Owner |
| Night log commit | every 30 minutes | Integrator |
| Test bucket | the existing `goalcup-traces-staging` | PRD-07 |

## Builder notes

Order of work for the integrator and scribe:

1. H+0: read `nproc`, the Vercel plan and session limit, and that a `v1.0/*` push works. Write the start line.
2. Phase 0 steps 1 to 7 (section 2). Publish the build snapshot before H+1.
3. Start builders by PRD in this order: 01, 02, 11, 17 and 07 at once; 03, 04, 05, 06, 08, 09, 10, 12, 14, 15
   as soon as the scaffold and seams compile; 13 waits for phase 2.
4. Merge on request by section 4, seams first. First staging deploy as soon as 06 and 07 are in.
5. Phase 1a smoke runs; then one short run per new core feature.
6. Phase 2 exit, phase 3 merge, phases 4 to 6, the morning report.

Traps:

- The gate refuses root (`scripts/gate.sh:32-39`). Never set `GOALCUP_GATE_ALLOW_ROOT=1` to pass a merge:
  root hides permission bugs. Run it in a build box as `ubuntu`.
- `scripts/check-version.py` is red on today's `docs/plans/` (PRD-14 ask 2): land PRD-14's change first.
- Two packages building a `goalcup` binary would make `target/*/goalcup` ambiguous: phase 0 moves the old
  crates out of `members`, so this never happens.
- `scripts/release.sh tag` tags `origin/main` only and checks its version (`:56`, `:61`); it never tags a
  branch. Nothing is tagged overnight.
- The production floor is a file value (`wrangler.toml:51-55`); a dashboard edit is wiped by the next deploy.
- Never `--persistent` sandboxes: a stop keeps the disk with keys for 30 days (PRD-12 section 7).
- Codex: Enter after a delay; `$CODEX_HOME/.env` overrides proxy variables, so the gateway key file holds
  only the key line (PRD-12 builder notes).
- Hermes: stop any gateway for the same home first; run the chat TUI, because one-shot mode caps delegation
  at 2 children (PRD-12 builder notes).
- One OpenAI key with 20 subagents hit 2M tokens a minute in run 1: A1 holds its key alone.
- Never check a pruned folder: `GOALCUP_KEEP_PARTS=1` on every run (PRD-12 B5).

How a watcher proves this PRD in a real run: at morning, open `MORNING.md`, follow the first line's commit
to its 9 core rows in `PAD-RESULTS.md` and the A1 to A3 rows in `ACCEPTANCE.md`; run
`goalcup-verify check --server <fetched A1> --local <A1 entry>` and compare the two SHA-256 sums; then run
`scripts/pad/table --release-check <commit>` and see exit 0.

## Asks of PRD-00 and PRD-01

PRD-00:

1. **Defaults out of date (30 Oct early decisions).** "Pad server: a local Worker" and question 2 should say:
   agents deploy 1.0 to staging overnight and use the test approval route; production stays the owner's.
   PRD-07 section 12 step 1 ("overnight, nothing is deployed") follows. Per-run cap is 10 USD, not 5. Set the
   night cap to this PRD's 400 USD (PRD-12 ask 2 shows 50 USD would stop phase 5).
2. **Phase 6.** Change "After green, morning: the owner deploys the Worker and merges to main. Then the
   2-hour run" to: acceptance A1 to A4 run overnight on staging after the core set is green; the owner may
   repeat A1 on production after the deploy. State whether A1 to A3 are release blockers. This PRD treats
   them as the condition for a "ready to merge" line, and the owner still decides.
3. **Ownership.** Add `MORNING.md` and `ACCEPTANCE.md` to PRD-16. Record that PRD-16 moves the old crates
   from `members` to `exclude` in phase 0 and removes them at the cut (answers PRD-13 asks 1 and 3, PRD-14 ask
   4).
4. **Phase 0 needs the PRDs.** Add "merge the plan branch (docs only) into integration" before any builder
   starts; the integration branch from 4b226de2 has none of them.
5. **Concurrency.** The build order assumes about 16 agents at once. The workflow runner allows min(16, CPUs
   minus 2), which is 2 on this 4-CPU container. Add the conductor host size or the cloud-session fallback
   to the build order.
6. **Merge rule.** Add "`gate fast` runs as a non-root user" (`scripts/gate.sh:32-39`) and "a reviewer read
   the diff once" (AGENTS.md:105).
7. **A wake list** in the defaults (section 11). Today PRD-00 says only when to stop and leave a note.
8. **Run 1's prompt.** It is not in the repository or the evidence folder. Ask the owner to put it in a file
   on the conductor host before H+9, and say whether it may be committed in `ACCEPTANCE.md`.
9. **The relayed "2 swp".** This PRD reads it as "the owner may swap harnesses in the list" (unverified).
   PRD-00 should record what it answers.
10. **Question 8.** Add PRD-07's "the floor applies to every mode" and PRD-14's "the floor deploys after the
    release is published", and say that staging carries the 1.0.0 floor from the first overnight deploy.
11. **Server access overnight** (against PRD-07 ask 2). The keys list of 30 Oct includes a read-only database
    key and Cloudflare. Say: read-only database and R2 credentials on the conductor host only, never in a box.

PRD-01:

1. **Name the build in the start event.** Add `build` (the commit the binary was built from) next to
   `goalcup` in the `start` event (section 11). Then the record itself, and its server copy, prove which
   commit recorded each run. The release check (PRD-14 G6) and A1 to A3 rely on that, and a row's
   `"goalcup":"1.0.0+<commit>"` (PRD-12 section 8) then has a source in the record.
2. **`keep` in the start event**, as PRD-12 asks of PRD-01 (ask 2): A1 to A3 need to prove from the server copy
   alone that local parts were kept for the comparison.
