# Local test day (handover to the local agent)

One working day, about 8 hours, on the owner's machines, to test branch `smart/zen-gates-d9tmbh`
(draft PR [#226](https://github.com/agentcommunity/goal-cup-tracer/pull/226)) with real harnesses
on staging. The owner is present for the first hours (sign-ins, approvals), then travels; after
that only a cloud agent works, and it sees only what is pushed. So: push results early and often.

Read first: [AGENTS.md](../../AGENTS.md), [LOCAL-CAMPAIGN](LOCAL-CAMPAIGN.md) (the kit, cells,
drills, pass lines, agent roles) and [LOCAL-NEXT-STEPS](LOCAL-NEXT-STEPS.md) (the checklist this
day works through). This page is the plan and the order; those are the reference.

## Ground rules

- **Staging only.** The scripts refuse production; never work around that.
- **Secrets are files, `chmod 600`,** never arguments, environment values, logs, chat or Git. Never
  print or copy a harness login (`auth.json`, `.credentials.json`, `.env`, `accounts.json`,
  `secrets.json`, Keychain). Never write `~/.codex`, `~/.claude`, `~/.hermes`, `~/.local/share/amp` or
  app-wide config. Goal Cup never sets a model base URL on a harness (the only wire path is the
  recording proxy). Hermes's own `config.yaml` does need `model.base_url`, its provider's address
  (D62): `goalcup prep` asks for it in Hermes's setup, and that is the participant's setting, not ours.
- **Cheapest models.** Codex `gpt-5.6-luna` at `low`; Claude Code `haiku`, thinking off; Hermes on
  the cheapest provider model the owner's key allows (DeepSeek flash was used before); Amp in its
  cheapest mode the owner approves. A run on a stronger model needs the owner's yes.
- **Goal Cup Proxy records, never manages.** Type each cell's prompt once. Never type "keep going",
  never restart a harness to fill time.
- **Raw traces and logs stay local.** Only aggregates and measured facts go into Git or to the
  cloud agent.
- **Keep every environment.** Do not remove Docker volumes, cell config folders, test repositories
  or staging test runs during the day. A fix must be retestable in the same place. Cleanup happens
  only at the very end, and only what the owner names ("At the end of the day").
- **Fix holistically, not by trial.** A failed cell is evidence: keep it. Reproduce the failure,
  find the root cause, decide whether it is code, script, docs or environment. Fix it with a test,
  run `sh scripts/gate.sh fast`, push, then retest in a new cell. If the fix is design-level or
  needs more than an hour, write it up for the cloud agent ("What to push for the cloud agent") instead.

## The plan, hour by hour

### 0:00 to 0:45: build and staging (owner present)

1. Pull the branch. `cargo fetch --locked`, `cargo build --offline --workspace`,
   `cargo build --release --workspace`. Put the release `target/release` first on `PATH`.
2. Start `sh scripts/gate.sh full` on the Mac as a normal user, in the background, at low priority
   (`nice`). It needs no network beyond loopback. Record its last line.
3. Staging, in this order (LOCAL-NEXT-STEPS step 3 and 4). **The database is shared with
   production, so the owner approves step c explicitly.**
   a. `sh server/worker/scripts/check-sql.sh` (needs PostgreSQL locally).
   b. Copy `server/worker/sql/pending/20261021100000_goal_cup_tracer_event_readiness.sql` into the
      website repo `supabase/migrations/`.
   c. Apply it once. Regenerate the mirror and check it matches.
   d. `wrangler secret put GITHUB_TOKEN --env staging` (any token that can call GitHub's API; the
      test repositories are public, so it needs no private access, see "Repositories").
   e. Deploy the staging Worker from this branch; keep `npx wrangler tail --env staging` open in a
      terminal the owner can see.
   f. Set `"test_runs_parallel": true` in every campaign config (test runs no longer replace each
      other once the migration is in).

### 0:30 to 1:30: sign-ins on every surface (owner at the browser)

Sign in once per cell, in the cell's own home. Never share or copy a login between cells (Codex
rotates its token; treat every harness the same way). The fewer cells, the fewer sign-ins: plan
the cells first, then sign in.

| Surface | How cells run | Harnesses |
|---|---|---|
| Mac host | `where: host`, one config folder per cell | codex-cli, claude-code-cli, amp-cli, hermes-cli |
| Docker on the Mac (Linux) | `where: docker`, one volume per cell | codex-cli, claude-code-cli, amp-cli, hermes-cli |
| Windows: WSL2 host | Goal Cup Proxy installed inside WSL2 Ubuntu, `where: host` | codex-cli, claude-code-cli (amp-cli and hermes-cli if time allows) |

- **Windows.** WSL2 is the supported path (README). It needs a Windows machine; without one, skip
  the WSL2 row and its cells and record "WSL2: not run (no Windows machine)" in the results.
  Docker Desktop on Windows runs the same Linux containers as Docker on the Mac, so it adds little;
  skip it unless a WSL2 host cell fails in a way Docker might isolate. Native Windows is not
  supported. The Windows machine needs its own local agent (or SSH into WSL2) running this page's
  WSL2 cells, pushing to the same branch.
- **Hermes.** The owner's API key is a file in the local repo. Put it into the cell's Hermes home
  `.env` by copying the file, never by echoing it (see [hermes-cli](../harnesses/hermes-cli.md),
  "The participant's own setup"). Set the provider and the cheapest model in that home's
  `config.yaml`. Check that `.env` is mode 600.
- **Amp.** The owner is signed in to Amp locally. Each Amp cell still signs in in its own home
  (`campaign.py login`). Record which mode the run used.
- **Docker cells** sign in with `python3 scripts/campaign.py login CONFIG <cell>`; host cells the
  same way (it runs `goalcup login` and `goalcup prep` in the cell's folder).
- Record each surface's facts as you go: where Claude Code keeps its login on Linux, whether Codex
  accepts `minimal` for Luna, whether Hermes or Amp load a skills folder (LOCAL-NEXT-STEPS step 6).

### Repositories

Two kinds of cells:

- **Infrastructure cells** (most of them): test runs on the empty public
  `agentcommunity/goalcup-matrix-empty`, with [campaign-workload.txt](campaign-workload.txt)
  (commit locally, never push). A test start sends the kit's GitHub token
  (`defaults.github_token_file`, LOCAL-CAMPAIGN "Cells") to GitHub to ask whether the repository is
  empty. The repository stays empty, so every such run may use it. That token must not be able to
  push to it: one agent push there would make every later test start refuse the repository.
- **Full-flow cells**: the agent pushes to a real repository, so the judged-commit path is tested
  end to end. A competition or test start refuses a repository that is not empty, so **each
  full-flow run needs its own new empty repository**. The orchestrator creates them as public
  repositories under one test owner (`gh repo create <owner>/gc-test-<cell>-<n> --public`, no
  README, no licence: `prep`, the clone and the Worker all refuse a private one), and lists them
  in the results file. The owner creates **one** fine-grained token for that test owner's
  repositories (Contents read and write), saved as a file. For each full-flow cell, run
  `goalcup prep --repo <url>` in the cell's config (host: with `GOALCUP_CONFIG_DIR`; Docker:
  inside the cell's container) and paste the token when asked. The kit keeps a prep'd repository
  and its token as they are (scripts/kit_settle.py); otherwise it settles the practice repository
  with the kit's token. Use [campaign-build-workload.txt](campaign-build-workload.txt): it pushes
  after each step, keeps `GOALCUP.md`, calls `goalcup time-left` and uses subagents.
  The kit was built for the practice repository and has not yet run a prep'd one: watch the first
  full-flow smoke closely (10 minutes, one cell) before starting more.
  Check afterwards: staging's judged commit equals GitHub's last push before the deadline. The
  repositories are public, so the staging Worker's `GITHUB_TOKEN` needs no access to them beyond
  GitHub's API.
- **Optional, for fun and realism**: a cell whose prompt asks the cheap agent to write a Goal Cup
  Proxy connector for another harness, following `skills/adding-a-capture-harness` (it clones this
  public repository into its workspace first). A good long, multi-file task.

### 1:00 to 1:45: 10-minute smoke of every cell

Every surface × harness, `duration_s: 600`, in parallel (`campaign.py run --parallel N`). Pass
lines: the run seals and uploads, `goalcup-verify` says integrity valid, the server copy equals
the local chain, the model check passes. Also, in at least one cell per harness, check by hand:
`goalcup time-left` answers inside the harness, `goalcup seal` refuses inside it and works from
another terminal, `goalcup status` shows disk, memory and CPU. Fix what fails before going on.

### 1:45 to 5:00: 1 to 2 hour cells, in parallel

| Cell kind | Where | Length | What it proves |
|---|---|---|---|
| Infrastructure, network cut | Docker codex and claude, Mac host codex | 2 h | ACK lag recovers, no gap unexplained |
| Infrastructure, pause or sleep | Docker codex (`pause`), Mac host (`pmset sleepnow`, prompted) | 2 h | the run survives a frozen machine |
| Disk pause (D65) | Mac host, small APFS volume (LOCAL-CAMPAIGN "Drills") | 1 h | pause below 1 GiB, resume above 3 GiB, one email |
| Full flow with subagents | one per harness: Mac amp, Docker claude, Docker codex, Mac hermes | 1 to 2 h | pushes, judged commit, GOALCUP.md, time-left, subagent messages in the record |
| Reboot then continue | Docker codex (`docker kill`, start, `goalcup continue`) | 1 h | a new part joins the run |
| WSL2 | WSL2 host codex and claude, network off by hand | 2 h | the Windows path works |

- **Subagents** are the main open question for Amp ([AMP-FOLLOWER](AMP-FOLLOWER.md)): check with
  `goalcup-verify --research-json` that the subagent's messages appear in the decoded record, for
  every harness. Claude Code's subagents must also be on haiku (the kit checks this).
- **Parallel limits:** CPU (a Docker cell takes up to 6), each harness's subscription quota (Claude
  Code hit its weekly limit before: start Claude cells small), and the owner's sign-in time.
- **The owner wants to see it.** Post a short status table in the chat every 20 to 30 minutes
  (`campaign.py status`: cell, state, elapsed, last sample age, ACK lag, problems), and keep
  `wrangler tail` visible.

### Any time: fix and retest

For each failure: reproduce, root-cause, fix with a test, `gate.sh fast`, push, retest in a new
cell next to the old one (keep both). Do not rerun a cell to get a pass without a change.

### 5:00 to 7:30: results, docs and the handover

Write everything down before the owner leaves (the two sections below). Longer cells may keep running;
the cloud agent cannot see them, so note where they are and when they end.

## What to push for the cloud agent

Push to `smart/zen-gates-d9tmbh` (pull with rebase first; the cloud agent may have pushed). Push a
progress commit at least every two hours, so a crashed session loses little.

- `docs/research/results/<date>-local-test-day.md` (aggregates only): every cell (surface,
  harness and version, model seen, workload, length, drills, PASS or FAIL, matrix row, how it
  ended), the gate verdicts, the staging steps done, the test repositories and volumes that still
  exist, and the cells still running with their end time.
- A **For the cloud agent** section: each open failure with how to reproduce it, the evidence
  (aggregates, never raw traces), the suspected cause and the files involved, in priority order.
- Measured facts into `docs/harnesses/<id>.md` and
  [coverage-matrix](coverage-matrix.md); tick [LOCAL-NEXT-STEPS](LOCAL-NEXT-STEPS.md) and update
  `TODO.md` status.
- Fix this page and LOCAL-CAMPAIGN wherever reality differed.

## At the end of the day

- Leave the environments running unless the owner says otherwise. List what exists: Docker
  volumes, cell config folders, test repositories, staging test runs.
- Delete only what the owner names. Test repositories: `gh repo delete` (owner's yes first).
  Staging runs: `server/worker/scripts/delete-test-runs.sh`, dry run first.
- Never delete a login or a trace that a failure's evidence depends on.
