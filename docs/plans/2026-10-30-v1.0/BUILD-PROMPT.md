# Build prompt for goalcup 1.0

Paste everything below the line into a new Claude Code cloud session on `agentcommunity/goal-cup-proxy`. It
builds goalcup 1.0 in one big run from the PRD set. The owner opts in to multi-agent orchestration here.

Before you paste it: the keys are in the cloud environment secrets (names in section 3), and the two staging
values are set (the local-agent prompt from the planning session).

---

You are the conductor of the goalcup 1.0 build. This is a large, multi-agent build: use workflows, and run
several workflows in parallel in this session. I opt in to that scale. Build everything the PRDs describe, test it
with real API calls, and leave me a morning report. Do not stop at a small update.

## 1. Read first, in this order

All on branch `smart/intelligent-newton-0avrpi` until the integrator merges it into `v1.0/integration`:

1. `VISION.md` (the seven points) and `AGENTS.md` (the hard rules).
2. `docs/plans/2026-10-30-v1.0/OWNER-DECISIONS.md`: the ledger. It wins over every PRD and older doc. Locked items
   are not yours to change; raise a conflict instead.
3. `docs/plans/2026-10-30-v1.0/PRD-00-index.md`: architecture, the file ownership map, seams, build order.
4. `docs/plans/2026-10-30-v1.0/PRD-01-record-format.md`: the format everyone builds against.
5. `docs/plans/2026-10-30-v1.0/PRD-16-orchestration-acceptance.md`: phases, merge rule, real runs, acceptance,
   autonomy, stop rules, wake list, morning report. Follow it, except where this prompt or the ledger says
   otherwise.
6. `docs/plans/2026-10-30-v1.0/PRD-17-lessons-old-code.md`, then each PRD as its lane starts.
7. Background only when a PRD points to it: `docs/plans/2026-10-30-cut-first-plan.md`, the audit appendices in
   `docs/plans/2026-10-30-audit/` (M: run 1 and its server copy; O: harness access; N: architecture), and the
   run 1 report on branch `origin/smart/tender-clarke-qupne3`.

Load the `workflow-authoring` skill before writing any workflow script.

## 2. Where things run

- **This session writes code, reviews and orchestrates. It never compiles or tests.** It has few CPUs and each
  workflow here runs only a couple of agents at once, so run several workflows in parallel, one per lane.
- **Every build, test and real run happens in Vercel Sandboxes** (build boxes and pad boxes, PRD-16 section 1).
  Code goes in as a git bundle. No key or token ever enters a build box. A pad box holds one harness key for the
  life of its run, in a 0600 file.
- **Staging only.** Every run goes to the staging Worker, approved headlessly with the staging approval key and
  my member id. Never touch production. Never add an approval bypass to production.
- Agents may deploy the Worker to staging and may create a separate R2 bucket for test runs. No website database
  schema migration. Read-only database access only.
- **Integration branch `v1.0/integration`.** Push only to `v1.0/*` branches. Nothing merges to main. Merge
  commits only, never a force push.

## 3. Keys (cloud environment secrets; never print, log or put one in argv)

`CLAUDE_API_KEY` (Claude Code), `OPENAI_API_KEY` (Codex, and the cheapest current OpenAI model for some
bring-your-own-key harnesses to spread rate limits), `DEEPSEEK_API_KEY` (every other bring-your-own-key harness),
`GEMINI_API_KEY` (Gemini CLI), `AI_GATEWAY_API_KEY` (gateway cases), `VERCEL_TOKEN` (and team id),
`CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` (staging Worker deploy, R2), `DATABASE_URL` (read-only role),
`GOALCUP_TEST_APPROVAL_KEY` and `GOALCUP_TEST_MEMBER_ID` (staging approvals), `GH_TOKEN` (test repos for pad runs).
No keys for OpenRouter, Amp or Factory: those cases are "not verified: no key", built from vendor docs.
If a key is missing, say which one in the night log, skip what needs it, and go on with the rest.
Spend cap: 10 USD per run.

## 4. How to build: lanes, each its own workflow

Start these workflows in parallel. Each owns the PRDs listed and only the files the PRD-00 ownership map gives them.

| Lane | PRDs | Starts |
|---|---|---|
| Integrator | PRD-16 phase 0, merges, gate runs in build boxes, staging deploys, build snapshot | first, H+0 |
| Lessons | PRD-17 (read old code, write `LESSONS.md` and the lessons tests) | H+0 |
| Format and core | PRD-01, PRD-02, then PRD-05 masking | H+0 (format types and seams first) |
| Wire | PRD-03 proxy, PRD-04 native capture | after the PRD-01 and PRD-02 seams |
| Upload and server | PRD-06 uploader, PRD-07 Worker | PRD-07 at H+0; PRD-06 after seams |
| Judge | PRD-08 verifier, PRD-09 usage and research stats | after PRD-01 |
| Harnesses and pad | PRD-11 connectors for the 15 harnesses, PRD-12 test pad | H+0 (pad skeleton and connector data do not wait for the core) |
| Participant | PRD-10 CLI | after PRD-02 |
| Cut and docs | PRD-13 cuts, PRD-14 tests and gate, PRD-15 docs | the cut only after the lessons tests pass on the new core |

**Inside every lane, per PRD, use this pipeline:**

1. **Plan.** A planner agent reads the PRD, the ledger, its seams and the current code, and writes a short task
   list. Each task names its files, all inside the PRD's owned paths.
2. **Build.** Builder agents take tasks in parallel, each in its own git worktree (`isolation: 'worktree'`) on
   `v1.0/prd-NN-<slug>`. They compile and test in a build box, not here. Builders of a harness-specific part read
   the vendor's current docs first and cite the page in the commit message.
3. **Review every diff with at least two independent reviewers**, given the diff and the PRD, not the builder's
   summary:
   - Reviewer A: correctness, the seven points, the ledger, masking before disk, no drop without a gap, no harness
     names in core code, no secret in fixtures, no path outside ownership.
   - Reviewer B: code quality and less code: duplication, dead paths, needless caps, anything that serves no point.
   - Core parts (format, recorder core, proxy, masking, uploader) get a third reviewer who tries to break it:
     a run 1 replay (about 40 subagents, 1.5 MB bodies, rate-limit bursts, WebSockets).
   Blockers per PRD-16 section 4; everything else is fixed if it makes the code smaller or clearer.
4. **Fix**, re-test in the build box, then ask the integrator to merge. Merge small green slices early.
5. **Replan.** After each PRD merges, an improvement agent reads what was learned (review findings, test results,
   real-run results) and updates the remaining PRDs and this plan in a commit on the integration branch, with a
   night-log line. It may not change a locked ledger item; it writes the conflict to the wake list instead.

## 5. Real tests: short early, massive at the end

- **Early proof.** As soon as format, recorder core, proxy and masking record one request end to end, run a short
  real test (Claude Code and Codex, a smoke prompt) in pad boxes. Then one short real test after each new core
  feature (native capture, uploader, watchdog restart, HTTP/2). If the concept fails here, stop adding features
  and fix it first.
- **Pad workload** for full runs: about 20 minutes of real work. The prompt asks the agent to build a payments
  platform (like PayPal, under a made-up name) with admin and user accounts, an admin frontend and a user frontend,
  and to use many subagents in parallel. Ask for subagents in the prompt; never force them by config.
- **Mass testing at the end:** the core set (Claude Code, Codex, Hermes) on all three cases (direct key, gateway,
  dodger), then the other 12 harnesses, then the acceptance runs A1 to A4 (PRD-16 section 7). Watchers fix failures
  and rerun; stop after three failed fixes on one cause and write it down.
- **Every real run checks:** wire calls match the harness's own log per 10 minutes; every subagent recorded; no
  silent loss (every loss is a gap with start and end); no secret in local parts, server copy or logs; the server
  copy is whole; usage stats (model and tokens per call and per subagent) present; the screen and the heartbeat say
  when recording was off.

## 6. Rules that never bend

- The seven points and the AGENTS.md hard rules, as the ledger section 7 restates them for 1.0.
- Never block or slow the harness. The recorder never stops for good: it restarts, writes a gap and marks the failure.
- No caps of our own; memory guards write a gap and go on.
- Masking before any byte reaches disk.
- Record raw, interpret later: the recorder never parses for rules or stats.
- Never write into a harness's real home or app-wide config; tests use a private `HOME`.
- No model names or model ids in commits, docs or code comments. No em dashes in docs.
- Report outcomes as they are. A skipped check is said to be skipped.

## 7. Stop and wake me (write to the wake list in `NIGHT-LOG.md`) only for

A locked ledger item that cannot hold, a hard rule that seems wrong, anything needing production or the website
database, spend past a cap, a missing key that blocks the core set, or the core set still failing after the fix
loops. Otherwise keep going.

## 8. When you finish

Write `MORNING.md` and `ACCEPTANCE.md` on the integration branch (PRD-16 sections 12 and 13): what was built, the
pad table per harness and case (verified, not verified, not supported, with the version), A1 to A4 results with run
1 compared, open failures with causes, spend, the codebase size before and after, and the steps that are mine (merge
to main, production Worker deploy, `MIN_COMPETITION_VERSION`, release tag, the macOS run).
