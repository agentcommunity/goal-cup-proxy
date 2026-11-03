# Alignment review: BUILD-PROMPT.md

File: `docs/plans/2026-10-30-v1.0/BUILD-PROMPT.md` (134 lines, branch `smart/intelligent-newton-0avrpi`).
Checked against: OWNER-DECISIONS.md (the ledger), VISION.md, AGENTS.md as ledger section 7 restates it,
PRD-00 (lanes, ownership, "Where the build runs", merge rule, done items, R3-10), PRD-16 (whole), PRD-01,
PRD-02, PRD-12 (C1 to C6, cases, keys) and PRD-17 where the prompt names them. Line numbers are
BUILD-PROMPT's unless a row names another file. Earlier parts that found the same issue are named (L6 =
align-L6-build, P16 = align-PRD-16, X3 = align-X3-builder, P00 = align-PRD-00, L1 = align-L1-what, L5 =
align-L5-harness); the fixes here agree with them, and the text below is the one to apply to this file.

Counts: 1 blocker, 10 major, 12 minor.

## The R3-10 conflict, settled

1. Ledger B1, B2 and B2a are locked and win over BUILD-PROMPT, PRD-16 and PRD-00.
2. Inside them, PRD-16 section 1's twelve lanes, start times and roles win over BUILD-PROMPT section 4's
   nine-lane table. Both shapes meet B2. PRD-16's is the one PRD-00 "Build order" and the phase 0 steps
   already use (PRD-10 stubs, PRD-14's gate change, PRD-05 and the Judge lane at H+0 against stubs).
3. BUILD-PROMPT's per-PRD pipeline (section 4 steps 1 to 5: planner, parallel builders in worktrees, at
   least two independent reviewers, fix, merge, replan) is the ledger's own text. It stands in all three
   files.
4. The conductor holds only the pasted prompt, and BUILD-PROMPT:25-26 today says the prompt wins over
   PRD-16. So R3-10 does nothing unless BUILD-PROMPT is edited before it is pasted. PRD-00:339 freezes the
   file only during the build, so an edit before H+0 is allowed.

## Findings

| id | file and section (lines) | what is wrong | breaks | severity | exact fix |
|---|---|---|---|---|---|
| BP-1 | Section 4 step 3, last line (89) | "Blockers per PRD-16 section 4; everything else is fixed if it makes the code smaller or clearer." PRD-16's list (PRD-16:204-208) is closed. It leaves out a break of a seven point or a locked ledger item: backpressure on the wire, a recorder that stops for good, a count taken before its bytes are written, `failed` not sent while off. Such a finding from reviewer A merges unfixed whenever the fix adds lines. B2 says reviewers read, then fix, then merge. Same as P16-1, L6-03 | B2; L2; R2, R3, R6, R7; points 1 to 6 | blocker | Replace line 89 with: "Every finding is fixed before the merge. A finding may be declined only with a one-line reason the reviewer who raised it accepts; reviewer B's less-code findings are declined when the change would not make the code smaller or clearer. Never declined: a break of one of the seven points, a locked ledger item or a hard rule; a path outside ownership; a byte on disk before masking; a drop with no gap; a cap of our own; a harness name in core code; a secret in a fixture; a wrong count; a check made to pass without record evidence." PRD-16:204-208 carries the same list |
| BP-2 | Section 1 item 5 (24-26); section 4 opening and lane table (61-73) | The prompt says follow PRD-16 "except where this prompt or the ledger says otherwise", then gives its own nine lanes: PRD-05 behind PRD-01 and PRD-02 in one 2-agent lane, no Masking, Test support or Real runs lane, Wire "after the PRD-01 and PRD-02 seams" (it also needs PRD-11's S3), Judge "after PRD-01", Participant "after PRD-02" (phase 0 needs PRD-10's `main.rs` stubs at H+0), PRD-14 inside "Cut and docs" (its gate change must land before any merge). PRD-00 R3-10 says the reverse precedence and that "the order is the same", which is false. Same as L6-01, P16-9, X3-3, P00-4 | B2 (two answers to the conductor); PRD-00 phase 0 order | major | Lines 24-26: replace "phases, merge rule, real runs, acceptance, autonomy, stop rules, wake list, morning report. Follow it, except where this prompt or the ledger says otherwise." with "lanes, phases, merge rule, real runs, acceptance, autonomy, stop rules, call list, end-of-build report. Follow it. Locked ledger items win over everything. Within them, PRD-16 wins over this prompt on lanes, phases, times and checks (PRD-00 R3-10); the per-PRD pipeline in section 4 is ledger B2 and B2a and holds in both. Read PRD-16's Amendments notes before each phase." Lines 61-73: replace the sentence and table with "Start the twelve lane workflows of PRD-16 section 1 in parallel, with the PRDs, start times and roles that table gives. Each lane edits only the files the PRD-00 ownership map gives its PRDs. If this session cannot hold them, use PRD-16 section 1's fallback (separate cloud sessions) and log which mode ran." |
| BP-3 | Section 4 step 5 (91-93) | The replan "updates the remaining PRDs and this plan". PRD-00:339 says this file is not edited during the build, and PRD-16:119 lets the replan only append notes to "the remaining PRDs". No file is named as "the build plan" of B2a, so real findings never reach the lane order or the conductor's running scripts. "After each PRD merges" is unclear when a PRD merges in slices, and no lane runs the role. Same as L6-02, P16-5, X3-4, X3-19 | B2a | major | Replace step 5 with: "**Replan.** After each merge of a PRD branch (every slice), the integrator lane starts an improvement agent. It reads what was learned (review findings, test results, real-run results) and appends an Amendments note to each remaining PRD it affects and to PRD-16, which is the build plan (lanes, order, phases, runs). One docs-only commit on integration and an `amendment` night-log line. Reviewer A of that merge reads the note against the ledger. You re-read PRD-16's Amendments and adjust the lane queues. This prompt is never edited during the build. A note may not change a locked ledger item; that calls me (W6)." |
| BP-4 | Section 4, Integrator row (65) and the pipeline intro (75) | The pipeline runs "inside every lane, per PRD", but PRD-16 section 2 has the integrator commit its own code diffs straight onto integration: the gate change (PRD-14 builder note 1), the workspace `Cargo.toml`, `Cargo.lock` refreshes, the phase 3 dependency removal. None meets two reviewers. Same as X3-2, L6-04, P16-6 | B2 ("at least two independent reviewers read each diff") | major | After line 75 add: "This holds for the integrator too: the gate change, the workspace `Cargo.toml` edit and the phase 3 workspace edit each go on their own `v1.0/prd-14-gate` or `v1.0/prd-16-<slug>` branch and through reviewers A and B. Only the docs merge of the PRD set, lock refreshes caused by a reviewed `Cargo.toml` change, the scribe's log and report commits and replan notes skip review." |
| BP-5 | Section 4 step 3, third reviewer (87-88) | "Tries to break it: a run 1 replay (about 40 subagents, ...)". No replay tool exists, and the run 1 converter is cut (PRD-00:575). A breaker following the prompt would build or run one per diff. PRD-16:118 says the breaker uses PRD-02's `pipeline_guard.rs` and the latest pad smoke row, never a real replay. Same as X3-10, L6-20, P16-21 | L6 (cuts); "build nothing not agreed" | major | Replace lines 87-88 with: "Core parts (format, recorder core, proxy, masking, uploader) get a third reviewer, the breaker, once per merge request. It tries to break the diff against run 1's shape (about 40 subagents, 1.5 MB bodies, rate-limit bursts, WebSockets) with PRD-02's `pipeline_guard.rs` and `pipeline_stall.rs` and the latest pad smoke row. It never builds a run 1 replay or converter." |
| BP-6 | Section 5 "Every real run checks" (107-110) | A paraphrase of the checks that differs from PRD-12 C1 to C6, which the watchers actually run. (a) "Wire calls match the harness's own log per 10 minutes" fails every Hermes row (per-session totals only, PRD-00 done item 1) and every dodger row, where the pinned host is a tunnel by design. (b) It leaves out C6: counts shown to the participant equal the verifier's recount and `fetch.json`. (c) The secret scan leaves out the pushed repository (C3, X7). A watcher who follows the prompt fails good rows and passes a wrong count. Overlaps X3-29 | T4; R6; point 6; B3 | major | Replace lines 107-110 with: "**Every real run** is judged by PRD-12's C1 to C6 and nothing looser: wire calls match the harness's own log (per 10 minutes; Hermes per session after the seal; the dodger case by PRD-12 section 2); every subagent recorded; no secret in local parts, server copy, logs, the pad's output or the pushed repository; record and server copy whole, with parts kept (`GOALCUP_KEEP_PARTS=1`); usage stats (model, tokens and status per call and per subagent) present; the counts on screen, in `status.json` and on the server equal the verifier's recount, and the screen and the heartbeat say when recording was off." |
| BP-7 | Section 5 "Mass testing" (104-106) | Order is core set, "then the other 12 harnesses, then the acceptance runs A1 to A4". PRD-16 sections 6 and 7 start the acceptance runs as soon as the core set is green, beside phase 5, A2 first, and A4 is Kilo's phase 5 gateway row. Read in order, the 2-hour A1 starts last and likely misses `REPORT_AT`. Same as L6-17, X3-11 | B3; R3-10 | major | Replace lines 104-106 with: "**Mass testing at the end:** the core set (Claude Code, Codex, Hermes) on all three cases (direct key, gateway, dodger). When it is green, the acceptance runs start beside the other 12 harnesses: A2 first, then A1 and A3; A4 is Kilo's gateway `full` row (PRD-16 sections 6 and 7). Watchers fix failures and rerun; stop after three failed fixes on one cause and write it down." |
| BP-8 | Section 7 (123-127) | The "only for" list leaves out a real secret found (PRD-16 W1, all runs stop), anything that touched production (W2, all runs stop), the shared database looking hurt (W3) and Vercel down 30 minutes (W6). PRD-16:345 says its W1 to W7 are "Only these (BUILD-PROMPT section 7)". A leaked key would wait for the report. Same as L6-07, P16-10 | Point 4; B4, B5, B8; S3 | major | Replace section 7 with: "## 7. Call me (a push notification and a top line in `NIGHT-LOG.md`, written at once, outside the 30-minute merge) only for PRD-16 section 11's W1 to W7: a real secret found in any record, server copy, log, commit or pushed repository (all runs stop); anything that touched production (all runs stop); the website database looking hurt; spend past a cap or a sandbox that will not stop; a key the core set needs missing or refused; a locked ledger item that cannot hold, a hard rule that seems wrong, or Vercel Sandboxes down for 30 minutes; the core set still failing after the fix loops. Otherwise keep going." |
| BP-9 | Section 2, pad box line (39-40) | "A pad box holds one harness key for the life of its run." The gateway case also needs the gateway key, and PRD-12:165 has the box create a public repo with the GitHub account token and keep it in `.env.local` in the agent's workspace. The account token could reach the record or a public repo. Same as L6-10 | Point 4 | major | Replace with: "A pad box holds its harness key (for the gateway case, the gateway key) and a deploy key for its one test repository, each in a 0600 file, for the life of its run. The GitHub account token never enters a box: the conductor creates the `gc-pad-*` repository and its deploy key." PRD-12:165 and PRD-16:290-291 follow (L6-10) |
| BP-10 | Section 3, `CLOUDFLARE_API_TOKEN` "(staging Worker deploy, R2)" (53) | One token for two jobs. `scripts/fetch-run.sh:26-28` reads `CLOUDFLARE_API_TOKEN` as "an R2 token with read-only access", and wrangler also reads that name by default. A token that can deploy `goalcup-ingest-staging` can, as far as Cloudflare scopes Workers edit rights per account rather than per Worker (to confirm on Cloudflare's token docs at H+0), also deploy production `goalcup-ingest`. PRD-16:81 says the conductor holds "never a production write credential". Creating a test bucket (S3) needs R2 write rights, which a read-only token lacks | B5 (production is the owner's); PRD-16:81; S3 | major | Line 53: replace "`CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` (staging Worker deploy, R2)" with "`CLOUDFLARE_ACCOUNT_ID`; `CLOUDFLARE_DEPLOY_TOKEN` (Workers edit and R2 bucket create, used only as `CLOUDFLARE_API_TOKEN=... wrangler deploy --env staging` or to create a test bucket; it can reach production, so no other wrangler command runs with it, and N7 checks the deploy list); `CLOUDFLARE_API_TOKEN` (R2 read-only, for `fetch-run`)" |
| BP-11 | Section 2 "Staging only" (41-42) | The prompt says every run goes to staging but not how. Today's client defaults to production (`crates/goalcup-cli/src/server.rs:24`, `DEFAULT_SERVER_URL = "https://ingest.agentcommunity.org"`), and PRD-00:277 keeps "`GOALCUP_SERVER` unset: production" in 1.0. The pad sets it (PRD-12:166), but a watcher's "reproduce" step, a builder's manual run or a phase 1a run started by hand does not, and would reach production (W2) | B4; B5 | major | After line 42 add: "Every run of `goalcup` in a box sets `GOALCUP_SERVER` to the staging URL or `off`; an unset value means production. Phase 1a and any run before the first staging deploy use `off`, which uploads nowhere." |
| BP-12 | Section 5 "Early proof" (97-100) | The feature list after the core slice has four items (native capture, uploader, watchdog restart, HTTP/2). PRD-00's second and third slices (PRD-00:550-551) also have exact-value masking, null mode, refusal learning, the gateway and dodger cases and Hermes `state.db`, each "one short real run per feature". Same as L6-13 | B3 | minor | Replace the parenthesis at 98-99 with "(each core feature of PRD-00's second and third slices: native capture, exact-value masking, uploader, watchdog restart and null mode, refusal learning, the gateway and dodger cases, Hermes `state.db`, HTTP/2)" |
| BP-13 | Section 2 "Staging only" (41) against section 5 early proof (97-99) | "Every run goes to the staging Worker", yet phase 1a runs use `--server off` (PRD-16:180-192) because staging carries 1.0 only after PRD-06 and PRD-07 step 2 merge. Read literally, the prompt forbids phase 1a | B4 (clarity); B3 | minor | Covered by the sentence in BP-11 |
| BP-14 | Section 6 (114-121) | Does not name ledger L5 (one process, no IPC for recorded bytes, no failure budget) or L7 (no backward compatibility), and does not restate the base-URL nuance from ledger section 7: the pad configures a harness as a participant would (gateway case), goalcup never does. A reviewer working from this list may pass a returning hand-off or a 0.7 shim, or block the pad's gateway config. Overlaps L1 F9 | L5, L7; ledger section 7 | minor | Add three bullets: "One process: no IPC or second daemon for recorded bytes, no failure budget (ledger L5)." "No backward compatibility with 0.7 (ledger L7)." "goalcup never sets a model base URL; the pad may set one in the harness's isolated config, as a participant would (ledger section 7)." |
| BP-15 | Section 6 (120) | "No model names or model ids in commits, docs or code comments" collides with files the build must hold: the pad's model choice (T2), row and usage JSON with a `models` list (S2), A1's "same model as run 1" in `ACCEPTANCE.md`. Same as X3-23 | S2, T2 (clarity) | minor | "No model names or ids in prose docs, commit messages or code comments. Config, fixtures and machine-read rows (`scripts/pad/*.toml`, `row.json`, usage JSON) may hold them." |
| BP-16 | Section 4 step 2 (79-81) | Several builders of one PRD in separate worktrees cannot all check out `v1.0/prd-NN-<slug>`: git refuses a branch checked out in another worktree. Builders compiling in a box also rewrite `Cargo.lock`; a committed lock fails merge rule 1. Same as L6-12, X3-24 | B2 (clarity) | minor | "on `v1.0/prd-NN-<slug>-<task>` from the PRD branch, merged into `v1.0/prd-NN-<slug>` by the lane before its merge request. Never commit `Cargo.lock`; the integrator refreshes it at merge." |
| BP-17 | Section 2 (36-37) against PRD-16:120 | "Each workflow runs only a couple of agents at once", yet PRD-16 lets up to 6 watchers and 6 pad boxes run at once from one Real runs lane. Same as X3-12 | B2, B3 (clarity) | minor | Add to section 5: "Watchers may run as up to three workflows of 2 agents beside the real-runs lane, so six can work at once." |
| BP-18 | Section 3 (50-57) | Keys are environment secrets, but PRD-12:311 and :752 read 0600 files under `~/.pad-keys/`, and no step writes them. "`VERCEL_TOKEN` (and team id)" names no variable for the team id. Same as L5-12 | T2; B7 | minor | Add: "Before the first pad run, write each key from its secret into its own 0600 file under `~/.pad-keys/` (`umask 077`, no echo, never in argv). The team id is `VERCEL_TEAM_ID`." |
| BP-19 | Section 8 (133-134) | The owner's steps read "merge to main, production Worker deploy, `MIN_COMPETITION_VERSION`, release tag, the macOS run". PRD-16 section 14 and PRD-00 owner question 5 order them merge, gate, tag, release, production deploy last; and `MIN_COMPETITION_VERSION = "1.0.0"` is already in both `wrangler.toml` blocks from the first staging deploy (PRD-16:188), so it is not a separate step | B5 (clarity) | minor | "the steps that are mine, in PRD-16 section 14's order (merge to main, `gate full` on main, release tag, release workflow, production Worker deploy last, which carries the 1.0.0 floor already in `wrangler.toml`, the macOS run, the `EVENT_ID` switch)" |
| BP-20 | Intro (6-7, 13); section 7 title (123) | B7 starts the build in the morning with the owner awake, but the prompt says "leave me a morning report" and "Stop and wake me". "The two staging values ... (the local-agent prompt from the planning session)" names neither value. Same as L6-14 | B7 (clarity) | minor | Line 13: "leave me the end-of-build report (`MORNING.md`, PRD-16 section 13)". Lines 6-7: "`GOALCUP_TEST_APPROVAL_KEY` and `GOALCUP_TEST_MEMBER_ID` are in the environment secrets, the member id has accepted the current terms on staging, and the staging Worker holds the matching `TEST_APPROVAL_KEY`". Title per BP-8 |
| BP-21 | Section 1 item 7 (28-30) | Background lists audit appendices M, O and N only. G (secrets) and F (long runs) are the evidence for masking (PRD-05) and for A1 and A2 | Coverage (clarity) | minor | "(M: run 1 and its server copy; O: harness access; N: architecture; G: secrets; F: week-long risks)" |
| BP-22 | Section 1 heading line (17) | The session is created on the repository's default branch; nothing says to check out the plan branch first, so "read first" may find no `docs/plans/2026-10-30-v1.0/` | B7 (clarity) | minor | "Check out `smart/intelligent-newton-0avrpi` first. All files below are on it until the integrator merges it into `v1.0/integration`." |
| BP-23 | Section 2 (36-40) against PRD-16:81 | The prompt rightly says this session "never compiles or tests". PRD-16:81 still has the conductor run `goalcup-verify` for checks, a binary from the build under test, on a fetched A1 copy of about 100 MB. That is testing on the conductor. Same as L6-05, P16-4 | B1 | minor in this file (the prompt is right; PRD-16 must follow) | Add to line 40: "Nothing from the build under test runs here: `goalcup-verify` checks run in the pad box (or a build box) on the fetched copy; only `fetch-run`, read-only queries, sandbox control and git run in this session." |

## Text each file should carry for B1, B2 and B2a

So that BUILD-PROMPT, PRD-16 and PRD-00 say the same thing:

**B1 (builds, tests and real runs only in Vercel Sandboxes).**
- BUILD-PROMPT section 2: as today, plus the BP-23 sentence and the BP-11 `GOALCUP_SERVER` sentence.
- PRD-16:81 Conductor row: "The lane workflows, their agents, git worktrees, `scripts/pad/pad` (conductor
  half: sandboxes, approval, fetch, read-only queries), `fetch-run`. It never compiles, tests or runs a
  binary from the build; `goalcup-verify` runs in the pad box or a build box on the fetched copy." X6, X7
  and the watcher proof (PRD-16:278-279, 590) add "in a box". PRD-16:211: CI `check` is "an independent
  record on GitHub's runners, not a build host for agents".
- PRD-00:485-486: "`fetch-run` and the read-only queries run there. The fetched copy is copied into a box
  and `goalcup-verify` runs there (ledger B1)." PRD-14:283 drops the conductor fallback (L6-06).

**B2 (parallel workflows, planner, builders in worktrees, at least two independent reviewers, fix, merge).**
- BUILD-PROMPT: BP-2 precedence sentence and pointer to PRD-16's twelve lanes; BP-1 review rule; BP-4
  integrator diffs reviewed; BP-16 task branches.
- PRD-16:91: "Each lane is one workflow in this session (ledger B2), at most 2 agents at once. This table
  replaces BUILD-PROMPT section 4's older table (PRD-00 R3-10)." PRD-16:117 Reviewers row: Count "at least
  two per merge request"; Does adds "Each is a fresh agent, not the builder, and writes its findings before
  seeing the other's." PRD-16:204-208 carries the BP-1 list. PRD-16 section 2 carries the BP-4 sentence.
- PRD-00:512: "two reviewers" becomes "at least two independent reviewers". PRD-00:871 R3-10 becomes:
  "Ledger B1, B2 and B2a are locked and win over this file, PRD-16 and BUILD-PROMPT.md. Within them the
  conductor starts PRD-16 section 1's twelve lanes with its start times and roles. BUILD-PROMPT section 4's
  older table (Masking inside Format and core, Judge after PRD-01, Participant after PRD-02, PRD-14 inside
  Cut and docs, no Real runs lane) is replaced by a pointer to PRD-16 before H+0. BUILD-PROMPT's per-PRD
  pipeline (plan, parallel builders in worktrees, at least two independent reviewers, fix, merge, replan)
  is the ledger's and stands." Applied by: "PRD-16; BUILD-PROMPT before H+0".

**B2a (replan after each PRD; updates the remaining PRDs and the build plan).**
- BUILD-PROMPT step 5: the BP-3 text.
- PRD-16:119 Replan row: Count "one per merge of a PRD branch (each slice), run in the integrator lane";
  Does "Reads review findings, test and real-run results; appends an Amendments note to each remaining PRD
  it affects and to PRD-16 itself, which is the build plan; one docs-only commit on integration with an
  `amendment` line. Reviewer A of that merge reads the note against the ledger. The conductor re-reads
  PRD-16's Amendments before each phase and each new lane task; planners read their PRD's before each
  task"; Never "Rewrites a section a builder is working from, edits BUILD-PROMPT.md, OWNER-DECISIONS.md or
  VISION.md, or changes a locked ledger item (that calls the owner, W6)".
- PRD-00:339 BUILD-PROMPT row: "Edited before H+0 only (R3-10). Not edited during the build: the conductor
  holds the pasted text. The build plan of ledger B2a is PRD-16; the replan updates it through Amendments
  notes. Where the prompt and PRD-16 differ, PRD-16 wins; the ledger wins over both." PRD-00:340: after "at
  the end of a PRD" add ", PRD-00 and PRD-16 included,".

## Code facts spot-checked (5 plus 3)

1. `Cargo.toml:18` is `version = "0.7.0"` and `:3-9` lists the five old crates: matches ledger L1 and
   PRD-16 "Today". Aligned.
2. `scripts/gate.sh:32-39` refuses root unless `GOALCUP_GATE_ALLOW_ROOT=1`; this container is `root` with
   4 CPUs (`whoami`, `nproc`). Supports the prompt's "never compiles or tests here". Aligned.
3. `server/worker/wrangler.toml:108-114`, `:139`: staging Worker `goalcup-ingest-staging`, bucket
   `goalcup-traces-staging`, the only deployment that may hold `TEST_APPROVAL_KEY`. Aligned with section 2.
4. `server/worker/src/device.ts:120-133`: `POST /v1/device/approve` takes the bearer approval key plus
   `member_id` in the body, so "approved headlessly with the staging approval key and my member id" is
   right. The SQL needs that member to have accepted current terms (`goal-cup-tracer-upload.sql`,
   `has_current_terms`), hence the BP-20 wording.
5. `scripts/fetch-run.sh:26-28`, `:74-78`: needs `DATABASE_URL` (read-only role) and
   `CLOUDFLARE_API_TOKEN` described as an R2 read-only token. Conflicts with section 3's one token for
   deploy and R2 (BP-10).
6. `crates/goalcup-cli/src/server.rs:24`: the default server is production. Basis of BP-11.
7. `origin/smart/tender-clarke-qupne3` exists and holds
   `docs/research/results/2026-10-30-two-hour-run.md` and its folder: section 1 item 7 is right. There is
   no prompt file in that folder (only `prompt-sent-at`), as PRD-16 says.
8. `docs/plans/2026-10-30-cut-first-plan.md` and audit appendices M, O, N exist; the `workflow-authoring`
   skill exists in this session. Aligned.

## Checked and found aligned

- L1 (1.0, one big build, not a patch): 11-13 "Do not stop at a small update". L2: reviewer B (86), section
  6. L3 and L4: Lessons lane (66), the cut only after the lessons tests pass on the new core (73). L6: Cut
  and docs lane. L7: not stated (BP-14).
- R1, R2, R3, R5, R8, M1: section 6 (115-118). R4: section 5 "every loss is a gap with start and end". R6,
  R7: partly, section 5 (BP-6 adds the count check).
- S3: 43-44 (staging deploy, separate R2 bucket, no migration, read-only database). S1, S2, U1, F1, F2, H1
  to H10: not restated; the prompt defers to the PRDs, which is right for a conductor prompt.
- T2: 50-55 (Claude key, OpenAI key and the cheapest current OpenAI model, DeepSeek, one Google key, no keys
  for OpenRouter, Amp, Factory, "not verified: no key"). T3, T4: section 8 tiers; section 5 checks (BP-6).
  T5: 101-103, word for word. T6: 57. T7: 80-81. T8: core set in section 5 and 7. T9: macOS in section 8.
- B1: 36-40 (no compile or test here, build and pad boxes, git bundle, no token in a build box). B2: 11-12,
  36-37, 75-89 (planner, worktree builders, two reviewers with the ledger's remits, third on core parts).
  B2a: step 5 exists (BP-3 fixes it). B3: section 5 early proof and mass testing, three failed fixes rule
  (106). B4: 41-42, no production bypass. B5: 43 and 133-134. B6: 45-46 (`v1.0/*` only, merge commits, no
  force push, nothing to main). B7: 6-7. B8: section 7.
- Hard rules (ledger section 7): base URL, never manages the agent, no harness names in core code (85),
  never write into real homes (119), record raw (118), no committed `target/` and so on by reference (114).
- Seven points: 1 and 2 (section 5 wire and subagent checks), 3 (116), 4 (117, 108-109), 5 (server copy
  whole), 6 (screen and heartbeat, plus BP-6), 7 (15 harnesses, gateway and dodger cases).
- Builds nothing not agreed, except the "run 1 replay" reading (BP-5). The prompt has no whole-build spend
  stop line, which matches T6 better than PRD-16 section 9 (see L6-09).
- PRD-01, PRD-02, PRD-17: the early proof needs format, recorder core, proxy and masking together (97-98),
  which keeps raw bytes off disk in real runs (agrees with X3-1's fix to PRD-02). The Lessons lane matches
  PRD-17's deletion gate.
