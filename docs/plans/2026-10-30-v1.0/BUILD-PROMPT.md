# Build prompt for goalcup 1.0

Paste everything between the first line and the second line into a new Claude Code cloud session on
`agentcommunity/goal-cup-proxy`. It builds goalcup 1.0 in one big run from the PRD set. The owner opts in to
multi-agent orchestration here. The section after the second line ("Alignment answers") is not pasted.

Before you paste it: the keys are in the cloud environment secrets (names in section 3);
`GOALCUP_TEST_APPROVAL_KEY` and `GOALCUP_TEST_MEMBER_ID` are in the environment secrets, the member id has
accepted the current terms on staging, and the staging Worker holds the matching `TEST_APPROVAL_KEY`.

This file is edited before H+0 only. During the build the conductor holds the pasted text, so the build plan
changes through Amendments notes on PRD-16 (PRD-00 R3-10, ledger B2a), never through this file.

---

You are the conductor of the goalcup 1.0 build. This is a large, multi-agent build: use workflows, and run
several workflows in parallel in this session. I opt in to that scale. Build everything the PRDs describe, test it
with real API calls, and leave me the end-of-build report (`MORNING.md`, PRD-16 section 13). Do not stop at a
small update.

## 1. Read first, in this order

Check out `smart/intelligent-newton-0avrpi` first. All files below are on it until the integrator merges it into
`v1.0/integration`:

1. `VISION.md` (the seven points) and `AGENTS.md` (the hard rules).
2. `docs/plans/2026-10-30-v1.0/OWNER-DECISIONS.md`: the ledger. It wins over every PRD, this prompt and older
   docs. Locked items are not yours to change; raise a conflict instead.
3. `docs/plans/2026-10-30-v1.0/PRD-00-index.md`: architecture, the file ownership map, seams, build order.
4. `docs/plans/2026-10-30-v1.0/PRD-01-record-format.md`: the format everyone builds against.
5. `docs/plans/2026-10-30-v1.0/PRD-16-orchestration-acceptance.md`: lanes, phases, merge rule, real runs,
   acceptance, autonomy, stop rules, call list, end-of-build report. Follow it. Locked ledger items win over
   everything. Within them, PRD-16 wins over this prompt on lanes, phases, times and checks (PRD-00 R3-10);
   the per-PRD pipeline in section 4 below is ledger B2 and B2a and holds in both. Re-read PRD-16's
   Amendments notes before each phase and after every replan.
6. `docs/plans/2026-10-30-v1.0/PRD-17-lessons-old-code.md`, then each PRD as its lane starts.
7. Background only when a PRD points to it: `docs/plans/2026-10-30-cut-first-plan.md`, the audit appendices in
   `docs/plans/2026-10-30-audit/` (M: run 1 and its server copy; O: harness access; N: architecture;
   G: secrets; F: week-long risks), and the run 1 report on branch `origin/smart/tender-clarke-qupne3`.

Load the `workflow-authoring` skill before writing any workflow script.

## 2. Where things run

- **This session writes code, reviews and orchestrates. It never compiles or tests.** It has few CPUs and each
  workflow here runs only a couple of agents at once, so run several workflows in parallel, one per lane.
  Nothing from the build under test runs here: `goalcup-verify` checks and the usage reader run in the pad
  box or a build box on the fetched copy; only `fetch-run`, read-only queries, sandbox control, approval and
  git run in this session. If sandboxes fail, builds wait and retry; they never fall back to this session.
  The gate runs as `ubuntu`, never with `GOALCUP_GATE_ALLOW_ROOT=1`.
- **Every build, test and real run happens in Vercel Sandboxes** (build boxes and pad boxes, PRD-16 section 1).
  Code goes in as a git bundle. No key or token ever enters a build box. A pad box holds its harness key (for
  the gateway case, also the gateway key) and the push token for its one test repository, each in a 0600
  file, for the life of its run. `GH_TOKEN` never enters a box: the conductor creates the `gc-pad-*`
  repository. The push token is a fine-grained token of the throwaway pad owner, Contents read and write only
  (PRD-12 section 7).
- **Staging only.** Every run goes to the staging Worker, approved headlessly with the staging approval key and
  my member id. Never touch production. Never add an approval bypass to production. Every run of `goalcup` in
  a box sets `GOALCUP_SERVER` to the staging URL or `off`; an unset value means production. Phase 1a and any
  run before the first staging deploy use `off`, which uploads nowhere.
- Agents may deploy the Worker to staging. Test runs use the existing `goalcup-traces-staging`; agents may
  create `goalcup-usage-staging` (PRD-07, PRD-09). No website database schema migration. Read-only database
  access only.
- **Integration branch `v1.0/integration`.** Push only to `v1.0/*` branches. Nothing merges to main. Merge
  commits only, never a force push.

## 3. Keys (cloud environment secrets; never print, log or put one in argv)

`CLAUDE_API_KEY` (Claude Code), `OPENAI_API_KEY` (Codex, and the cheapest current OpenAI model for some
bring-your-own-key harnesses to spread rate limits), `DEEPSEEK_API_KEY` (every other bring-your-own-key harness),
`GEMINI_API_KEY` (Gemini CLI), `AI_GATEWAY_API_KEY` (gateway cases), `VERCEL_TOKEN` (and the team id),
`CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` (staging Worker deploy, R2), `DATABASE_URL` (read-only role),
`GOALCUP_TEST_APPROVAL_KEY` and `GOALCUP_TEST_MEMBER_ID` (staging approvals), `GH_TOKEN` (creates test repos for
pad runs, on the conductor only). The pad push token is my input; until it is there, rows run without a test
repo and say "push not tested" (PRD-12 section 7).
No keys for OpenRouter, Amp or Factory: those cases are "not verified: no key", built from vendor docs.
Before the first pad run, write each key from its secret into its own 0600 file under `~/.pad-keys/`
(`umask 077`, no echo, never in argv).
If a key is missing, say which one in the night log, skip what needs it, and go on with the rest.
Spend cap: 10 USD per run.

## 4. How to build: lanes, each its own workflow

Start the twelve lane workflows of PRD-16 section 1 in parallel, with the PRDs, start times and roles that
table gives (Judge, Participant and Masking start at H+0 against fixtures and stubs; PRD-14's gate change
lands first; PRD-13's cut waits for PRD-17's deletion gate). Each lane edits only the files the PRD-00
ownership map gives its PRDs. If this session cannot hold them, use PRD-16 section 1's fallback (separate
cloud sessions) and log which mode ran.

This holds for the integrator too: the gate change, the workspace `Cargo.toml` edits and the phase 3 workspace
edit each go on their own `v1.0/prd-14-gate` or `v1.0/prd-16-<slug>` branch and through reviewers A and B.
Only the docs merge of the PRD set, lock refreshes caused by a reviewed `Cargo.toml` change, the scribe's log
and report commits and replan notes skip review.

**Inside every lane, per PRD, use this pipeline:**

1. **Plan.** A planner agent reads the PRD, the ledger, its seams, the current code and the PRD's Amendments
   note, and writes a short task list. Each task names its files, all inside the PRD's owned paths.
2. **Build.** Builder agents take tasks in parallel, each in its own git worktree (`isolation: 'worktree'`) on
   `v1.0/prd-NN-<slug>-<task>` from the PRD branch, merged into `v1.0/prd-NN-<slug>` by the lane before its
   merge request. They compile and test in a build box, not here. Never commit `Cargo.lock` from a task
   branch; the integrator refreshes it at merge. Builders of a harness-specific part read the vendor's current
   docs first and cite the page in the commit message.
3. **Review every diff with at least two independent reviewers**, given the diff and the PRD, not the builder's
   summary. Each is a fresh agent and writes its findings before it sees the other's.
   - Reviewer A: correctness, the seven points, the ledger, masking before disk, no drop without a gap, no harness
     names in core code, no secret in fixtures, no path outside ownership; nothing PRD-13's no-return guard
     names comes back (daemon or IPC for bytes, our own caps, disk handling, signing, pins, termination proof,
     research decoders), and no compatibility shim for 0.7.
   - Reviewer B: code quality and less code: duplication, dead paths, needless caps, anything that serves no
     point, and the PRD's row in PRD-00 "Size budget".
   - Core parts (format, recorder core, proxy, masking, uploader) get a third reviewer, the breaker, once per
     merge request. It tries to break the diff against run 1's shape (about 40 and then about 200 synthetic
     subagents, 1.5 MB bodies in bursts, a held writer, WebSockets) with PRD-02's `pipeline_guard.rs`,
     `pipeline_stall.rs` and `by_hand_load`, and the latest pad smoke row. There is no run 1 replay tool; never
     build one or run a real 2-hour replay per diff.
   Every finding is fixed before the merge, unless the reviewer who raised it accepts a one-line reason to
   decline; a less-code finding may be declined when the change would not make the code smaller or clearer.
   Never declined: a break of one of the seven points or of a locked ledger item (for example backpressure on
   the wire, a recorder that stops for good, a count taken before its bytes are written, `failed` not sent
   while recording is off), a hard-rule break as ledger section 7 restates it, a path outside ownership, a
   byte written to disk before masking, a drop with no gap, a cap of our own, a harness name in core code, a
   secret in a fixture, a wrong count, a failing acceptance case of the PRD, or a check made to pass without
   record evidence (PRD-16 section 4 carries the same list).
4. **Fix**, re-test in the build box, then ask the integrator to merge. Merge small green slices early.
5. **Replan.** After each merge of a PRD branch (every slice), the integrator lane starts an improvement agent.
   It reads what was learned (review findings, test results, real-run results) and appends an Amendments note
   to each remaining PRD it affects and to PRD-16, which with PRD-00 "Build order" is the build plan (lanes,
   order, phases, runs). It posts `wc -l` of the merged PRD against its PRD-00 "Size budget" row. One
   docs-only commit on integration and an `amendment` night-log line. Reviewer A of that merge reads the note
   against the ledger. You re-read PRD-16's Amendments and adjust the lane queues. This prompt is never edited
   during the build. A note may not change a locked ledger item; that calls me (W6).

## 5. Real tests: short early, massive at the end

**Go wide.** Use all the parallelism we can afford (ledger B11). Every build, test and experiment runs in Vercel
Sandboxes, many at once; this session only drives agents. Run several workflows at the same time, with many
subagents for tasks, reviews, rig runs and fixes.

**The rig first (ledger B9).** Early in the build, make the pad (PRD-12) a small rig an agent can use without
driving it: one command, one small settings file (harness, key name, case, prompt, steps), built-in waits, and it
walks every step by itself (start, wait for calls, check the data, crash, wait for recovery, keep going, seal,
fetch, check, clean up), then prints one plain table. Keep it simple. Before you trust it: it must fail on run 1's
known bugs (PRD-12 selftest), and a reviewer reads its first outputs by hand to check it tests the right thing.
Every rig run cleans up its sandbox, test repo and local files.

**The test ladder (ledger B3).** Climb one rung at a time; when a rung fails, fix it before the next.
1. **Micro tests while building:** one harness, one key, a few calls, just to prove each piece is live. Local
   capture first (`GOALCUP_SERVER=off`, nothing uploaded). Then add one layer at a time: upload to staging,
   subagents asked for in the prompt, a recorder crash, a watchdog kill and `goalcup recover`, "keep going", the
   gateway case, the dodger case, HTTP/2.
2. **Cycle runs per pad,** a few minutes each, not 20: setup, data flows, look at the recorded data, crash
   something, check it stands back up, "keep going" on a few harnesses where it makes sense, seal, check the
   server copy, clean up. Run them for every harness and case as soon as its connector lands, many in parallel.
3. **Full runs at the end:** the 20-minute runs below, for every pad, all in parallel, until they pass.

**Do not overbuild tests (ledger B10).** The rig's real runs are the main proof. Automated tests guard only the
seven points, the format, masking and the never-stop rules. No tests that pin internal shapes or wording. If a
test takes longer to change than the code it guards, delete or simplify it.

- **Early proof.** As soon as format, recorder core, proxy and masking record one request end to end, climb rung 1
  through the rig (Claude Code and Codex, `mode = "micro"`, `GOALCUP_SERVER=off`) in pad boxes. Then one micro
  run after each new core feature (each core feature of PRD-00's second and third slices: native capture,
  uploader, watchdog restart and null mode, refusal learning, the gateway and dodger cases, Hermes `state.db`,
  HTTP/2, and exact-value masking only if I said yes, PRD-00 owner question 3). If the concept fails here, stop
  adding features and fix it first.
- **Pad workload** for full runs: about 20 minutes of real work. The prompt asks the agent to build a payments
  platform (like PayPal, under a made-up name) with admin and user accounts, an admin frontend and a user frontend,
  and to use many subagents in parallel. Ask for subagents in the prompt; never force them by config.
- **Mass testing at the end:** the core set (Claude Code, Codex, Hermes) on all three cases (direct key,
  gateway, dodger), each a `full` run. The other 12 harnesses run `full` at the same time, every pad in parallel (ledger B3);
  the core set has first claim on the Claude, OpenAI and gateway keys (PRD-16 section 6).
  When the core set is green, the acceptance runs start beside them: A2 first, then A1 and A3; A4 is Kilo's
  gateway `full` row (PRD-16 sections 6 and 7). Watchers fix failures and rerun; stop after three failed fixes
  on one cause and write it down. Start one watcher per named cause, each its own workflow of 2 agents, as
  many at once as there are causes (ledger B11).
- **Every real run** is judged by PRD-12's C1 to C6 and nothing looser. The proxy record is the proof (ledger
  T4): no unexplained gap and the history check finds no missed call (the dodger case by PRD-12 section 2); the
  harness's own session files are an optional extra check where it keeps them, and no outside API (no provider
  usage lookup) is called to verify; every subagent recorded; no silent loss (every loss is a gap with start
  and end); no secret in local parts, server copy, logs, the pad's output or the pushed repository; record and
  server copy whole, with parts kept (`GOALCUP_KEEP_PARTS=1`); usage stats (model, tokens and status per call
  and per subagent) present; the counts on screen, in `status.json` and on the server equal the verifier's
  recount, and the screen and the heartbeat say when recording was off.

## 6. Rules that never bend

- The seven points and the AGENTS.md hard rules, as the ledger section 7 restates them for 1.0.
- Never block or slow the harness. The recorder never stops for good: it restarts, writes a gap and marks the failure.
- One process: no IPC or second daemon for recorded bytes, no failure budget (ledger L5).
- No caps of our own; memory guards write a gap and go on.
- Masking before any byte reaches disk.
- Record raw, interpret later: the recorder never parses for rules or stats.
- No backward compatibility with 0.7 (ledger L7).
- goalcup never sets a model base URL; the pad may set one in the harness's isolated config, as a participant
  would (ledger section 7).
- Never write into a harness's real home or app-wide config; tests use a private `HOME`.
- No model names or ids in prose docs, commit messages or code comments. Config, fixtures and machine-read rows
  (`scripts/pad/*.toml`, `row.json`, usage JSON) may hold them. No em dashes in docs.
- Report outcomes as they are. A skipped check is said to be skipped.

## 7. Stops and calls

**Stop that item and leave a log note (the build goes on)** for PRD-16 section 8's stops: a locked ledger item
or a hard rule, the website database, production, `main` and tags, spend past a cap, and any edit to
`VISION.md` or `OWNER-DECISIONS.md`.

**Call me** (a push notification and a `call` line at the top of `NIGHT-LOG.md`, written at once, outside the
30-minute merge) only for PRD-16 section 11's W1 to W7: a real secret found in any record, server copy, log,
commit or pushed repository (all runs stop); anything that touched production (all runs stop); the website
database looking hurt; spend out of control (a run past 1.5 times its cap, or a sandbox that will not stop); a
key the core set needs missing or refused; a locked ledger item that cannot hold, a hard rule that seems
wrong, or Vercel Sandboxes down for 30 minutes; the core set still failing after the fix loops. Otherwise keep
going.

## 8. When you finish

Write `MORNING.md` and `ACCEPTANCE.md` on the integration branch (PRD-16 sections 12 and 13), `MORNING.md`
following PRD-00 "Morning report template". It ends with the steps that are mine, in PRD-16 section 14's
order: merge to main, `gate full` on main, release tag, the release workflow and the macOS run, the production
Worker deploy last (it carries the 1.0.0 floor already in `wrangler.toml`), the `EVENT_ID` switch.

---

## Alignment answers

Not part of the paste. Fix lists g1, g2 and g3 of the alignment pass, checked against the ledger, PRD-00,
PRD-12 and PRD-16.

### Group 1

| Id | What was done |
|---|---|
| L6-01 | Applied, merged with G2-03 and G3-X2. Section 1 item 5 now says PRD-16 wins within the ledger; section 4's own table is replaced by a pointer to PRD-16 section 1's twelve lanes |
| L6-02 | Applied, merged with G2-04 and G3-X3. Replan step 5 rewritten: every slice, integrator lane, Amendments notes to remaining PRDs and PRD-16, this prompt never edited during the build |
| L6-03 | Applied, merged with G3-X5. Section 4 step 3 carries the fix-every-finding rule and the never-declined list of PRD-16 section 4 |
| L6-07 | Applied, merged with G3-X46 and X3-20. Section 7 lists W1 to W7 |
| L6-10 | Partly. The pad box line and `GH_TOKEN` rule are fixed, but with PRD-12 and PRD-16's push token (fine-grained, throwaway pad owner, Contents read and write), not a deploy key: those two PRDs carry the push token, and this prompt follows them (G3-X12) |
| F9 | Applied. Reviewer A checks PRD-13's no-return guard and no 0.7 shim; section 6 has the one-process and no-compatibility rules |
| L6-12 | Applied. Task branches `v1.0/prd-NN-<slug>-<task>` |
| L6-13 | Applied. The early-proof list is PRD-00's second and third slices; exact-value masking only with the owner's yes (PRD-00 owner question 3), as PRD-16 phase 1a says. G3-BPc's unconditional wording was not used |
| L6-14 | Applied, merged with G3-BPi. Intro names the two staging secrets and the Worker's `TEST_APPROVAL_KEY`; the report is the end-of-build report |
| L6-17 | Applied, merged with X3-11 and G3-BPa |
| L6-20 | Applied, merged with G2-10 and G3-X47 |

### Group 2

| Id | What was done |
|---|---|
| G2-03 | Applied (see L6-01). Ledger B1, B2 and B2a stay above both texts |
| G2-04 | Applied (see L6-02). The build plan is PRD-16 with PRD-00 "Build order" |
| G2-10 | Applied. The breaker uses PRD-02's `pipeline_guard.rs`, `pipeline_stall.rs` and `by_hand_load` at about 40 and about 200 subagents; no run 1 replay tool |
| G2-38 | Applied. Reviewer B checks the PRD-00 "Size budget" row; the replan posts `wc -l` against it |
| X3-11 | Applied. Core set first; when green, A2, then A1 and A3, beside the other 12 harnesses; A4 is Kilo's gateway row |
| X2-10 | Applied. Section 8 points to PRD-00 "Morning report template" instead of its own list |
| X3-20 | Applied in section 7 |
| X3-23 | Applied in section 6 |
| X3-24 | Applied. Task branches never commit `Cargo.lock` |
| X3-29 | Applied in "Every real run" (Hermes per session after the seal, PRD-00 done item 1) |

### Group 3

| Id | What was done |
|---|---|
| G3-X5 | Applied (see L6-03) |
| G3-X1 | Applied in section 2 |
| G3-X2 | Applied (see L6-01), with the fallback to separate cloud sessions |
| G3-X3 | Applied (see L6-02) |
| G3-X4 | Applied. Integrator changes outside `docs/plans/` go through reviewers A and B |
| G3-X47 | Applied (see G2-10) |
| G3-X48 | Applied. Every real run is judged by PRD-12's C1 to C6 |
| G3-BPa | Applied (see X3-11). Superseded by ledger B3 and B11 (30 Oct): phases 4 and 5 start together, as PRD-16 section 6 now says |
| G3-X46 | Applied (see L6-07). Section 7 also lists PRD-16 section 8's stops, which leave a note and go on (ledger B8) |
| G3-X12 | Applied in sections 2 and 3. The push token has no secret name in any PRD, so section 3 names it as an owner input, as PRD-12 section 7 does |
| G3-BPb | Applied. `GOALCUP_SERVER` unset means production (PRD-10 section 2) |
| G3-BPc | Partly. Applied with exact-value masking conditional on the owner's yes (see L6-13) |
| G3-BPd | Applied in section 6 |
| G3-BPe | Applied (see X3-23). The attribution trailer stays an owner question |
| G3-BPf | Applied (see L6-12, X3-24) |
| G3-X58 | Applied in section 5 |
| G3-BPg | Partly. The `~/.pad-keys/` rule is in section 3. `VERCEL_TEAM_ID` was not named: no PRD gives the secret's name, so section 3 keeps "and the team id" |
| G3-BPh | Applied, in PRD-16 section 14's actual order (the macOS run goes with the release workflow, before the production deploy). `MIN_COMPETITION_VERSION` is no longer listed as an owner step: it is already in `wrangler.toml` |
| G3-BPi | Applied (see L6-14), with the terms precondition (PRD-10: terms not accepted show at the first practice start) |
| G3-BPj | Applied. Audit appendices G and F added |
| G3-BPk | Applied. Check out the planning branch first |
| G3-BPl | Applied. The existing `goalcup-traces-staging` for runs, as PRD-07 says; agents may create `goalcup-usage-staging` |
| B3, B9, B10, B11 sweep | Applied 30 Oct. Section 5 early proof is ladder rung 1 (`micro`) through the rig; the other 12 harnesses run `full` beside the core set, not after its first wave (B3); watchers are one per named cause with no count of our own (B11) |
