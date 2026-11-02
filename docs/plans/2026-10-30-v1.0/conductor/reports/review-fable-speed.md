# goalcup 1.0 finishing plans: speed, critical path and cost review

Independent reviewer, read only. Written Sun 1 Nov 2026 about 23:50Z (15:50 PT, PST). Read on `origin/v1.0/conductor-handover` at `e9dfd3b7`: `docs/superpowers/plans/README.md` and the four plans, `conductor/STATE.md`, `TOOLS.md`, `NEW-SESSION-PROMPT.md`, `state/WRITING-PLANS.md` (owner sections), `state/STATUS-REPORT.md`, `state/FINISH-PLAN.md`, `state/reviews/LATEST.md`, the last 200 lines of `state/lanes/integrator.log` (to 23:41Z), `scripts/pad/pad.toml`, `scripts/gate.sh`, `merge.sh`. No box started, nothing built, nothing edited in any repo.

## 1. Verdict in short

The wave order is right in shape (code, one train, one build, one composite wave, one fix round, one-hour runs, release, big tests) but it is laid out as a serial chain that idles the pad for 15 hours tonight and then idles everything for 17 hours between the one-hour runs and the freeze. Three facts the plans themselves state make a much faster path possible:

1. Composite 10-minute rows are diagnostic, never release rows (`mini` plan, Row procedure step 4: a micro row gives no tier; rows made before long-runs Task 1 merges "can never be trusted" anyway). So they do not need H1, a merged pad tree or trust. They need a binary with the masking package (`04044320`, already on integration) and a pad tree with the data fixes (read from the worktree, not compiled: FINISH-PLAN "Build early").
2. The trie matcher is required only "before the 2-hour runs" (owner 22:05Z), not before the 10-minute wave.
3. The 15 USD cap (ask 5) was proposed for 2-hour runs. The one-hour A2 and A5 fit under the existing `cap_usd = 10` (Haiku estimate 5 to 10 USD per 2 hours; Codex full rows cost 1.7 to 3.6 USD for 25 minutes), and A3 on DeepSeek via the gateway costs under 1 USD per hour against 13.82 USD of credit. Asks 5 and 9 do not gate Wave L.

Taken together: build `04044320` tonight, run the canary pair within the hour, run the whole composite wave tonight from the rig-slice worktree, fold the trie, Wave A, the rig slice, the handover and the fix batch into ONE train, build H2 once, and run the one-hour runs and the 9 core rows on H2 before the owner wakes on Monday. Builds stay at 3, gates at 4 or 5, rows at about 30, boxes at about 75. The release moves from Wed 14:00 PT to Tue 3 Nov (owner freeze permitting) or Wed morning at the latest, with Thursday as buffer before the event opens Thu 16:00 PT.

## 2. Ranked recommendations

Each: what to change, where, what it saves, the risk.

### R1. Build `04044320` now and run the composite wave tonight, not Monday 07:00 PT

- **Change:** `mini` plan Task 9 Step 0 runs now with `<H1>` = `04044320` (call it B0). Task 10 runs as soon as the rig slice's one box passes (Task 8 Step 1), from the rig worktree `/root/gc-wt/prd12-mini` with `build = /root/gc-night/builds/04044320`, before the train. H1 is dropped; the next build is H2 (train head: trie, Wave A, rig, handover, fixes). README wave table rows 3 and 4 move to Sun 17:00 to 20:00 PT.
- **Saves:** about 8 to 10 hours on the critical path. Builds unchanged (B0, H2, HF = 3). Zero extra boxes: H1 was a build that gets superseded by H2 within hours anyway.
- **Risk:** the Codex gateway pin (`codex-cli.toml:26`, Task 5) is not in `04044320`, so the Codex canary prints a `recorded model ..., pinned ...` why line; read it as expected, not a stop. Rows carry no `pad_commit`, which is fine because no mini row ever enters `pad-rows.jsonl`. Ask 8 (build reuse) is not needed: these rows claim nothing.

### R2. Run the canary pair first, tonight, from the integration worktree

- **Change:** before the rig slice is even reviewed, start `claude-code.toml` and `codex.toml` (gateway, composite) on B0 from a worktree at `04044320`. Neither needs a rig fix (Codex passed all six in gw3 with the current exit key; the exit key is only used on kill rows).
- **Saves:** the single most valuable fact of the night (does the masking package start, mask the planted values, pin Haiku, upload and pass C4 together) arrives about Sun 18:00 PT instead of Mon 07:30 PT. If it fails (review 6 item 3.1: boot with many values; READY_NS), the fix goes into the same train as everything else instead of forcing a second fix round on Monday.
- **Risk:** none beyond 2 rows (4 boxes, about 1.9 USD) already in the budget.

### R3. One train, not three; take the handover off the critical path

- **Change:** README "Wave order" rows 1 and 2 and the handover plan's Global Constraints ("ONE small train of their own ... merges first") and Task 10 Step 7. The handover branch joins the code-wave train (or merges any time later). The mini plan's Task 9 "handover tools installed" precondition is dropped: `padrun.sh`, `keyscan.py`, `proxy-preload.cjs` and `rowinfo.py` already exist under `/root/gc-night/gw3/` and `/root/gc-tools/`, and this session is not a fresh container. `row.sh` points at `/root/gc-night/gw3/padrun.sh` for tonight.
- **Saves:** one gate (about 8 minutes of 8 vCPU plus queue serialisation) and, more important, 2 to 6 hours of waiting for a 10-task TDD tooling plan before any row can start. Gates: 4 instead of 5.
- **Risk:** a bigger train bisects slower on a gate FAIL (see R7). The handover tools are for a future fresh session; nothing in Waves 1 to 3 needs them.

### R4. Run the 9 core release rows on H2 Monday morning, inside the idle gap, not after the freeze

- **Change:** long-runs Task 8 Steps 3 and 4 say "only the missing items" but then budget the 9 core rows as likely missing. Run `core-claude-code.toml`, `core-codex.toml`, `core-hermes.toml` on H2 right after A2, A5 and A3 end (Mon about 03:00 PT in the fast timeline; Mon 16:30 PT in the plan's own timeline). The release check compares the guarded tree (`crates/`, `connectors/`, `Cargo.*`, `server/worker/src/`), not the commit, so if every post-H2 merge is docs, pad data or `tiers.toml`, the H2 rows stand at HF and Wave F shrinks to one gate-full-plus-build box, re-stamp, table train, N6, newcomer, staging, Mac.
- **Saves:** about 1.5 hours of wall time off the post-freeze path and removes the largest post-freeze box burst (18 boxes) from the window where a failure costs most. Box count unchanged.
- **Risk:** a post-freeze `crates/` or `connectors/` fix invalidates them and they rerun, which the plan would do anyway. The participant-text seam (R8) changes `crates/`, so it must be in H2 or the rows are wasted: put it in the train.

### R5. Do not wait on asks 5 and 9 for Wave L; state defaults for asks 1 and 8 instead of asking

- **Change:** long-runs Task 7 Step 1 preconditions. A2 and A5 run under the existing cap 10 (the pad seals at the cap, which PRD-16 section 9 says is not a failure; `longsum` marks X2 INCONCLUSIVE only if the run was cut short of 15 subagents). A3 runs on the 13.82 USD gateway credit. Ask 1 (index box counts toward 8): decide "row boxes only" and say so; the index box is 1 vCPU for about 5 minutes. Ask 8 (build reuse): only `v1.0.0` in Task 13 Step 1 depends on it.
- **Saves:** the two most likely overnight stalls. Wave L can start the moment H2 is built and the selftest JSON is read.
- **Risk:** A2 sealed at 10 USD before 15 subagents gives INCONCLUSIVE and one rerun at 15 once the owner answers; Haiku full1 cost 7.33 USD on the default model and about 2.2 USD pinned, so a one-hour Haiku run under 10 USD is the likely case.

### R6. Start the Mac `release.yml` run tonight

- **Change:** long-runs Task 8 Step 9 moves from Wave F to tonight, on `04044320`; FINISH-PLAN Wave 1 "Mac" lane already said so. It needs ask 7 (metered Mac minutes, nothing published) and no box.
- **Saves:** the one failure that would force a post-freeze `crates/` fix, a new HF build and 18 rerun boxes (R4). Review 6 rated it the under-weighted high risk: 24 `cfg(target_os = "macos")` blocks never compiled in 1.0.
- **Risk:** Mac runner minutes (owner cost). A red job is a fix lane tonight, in the train.

### R7. Make the train gate pass first time

- **Change:** every slice box job (`mini` Task 8, matcher Task 2 Step 3, long-runs Task 5 Step 2, handover Task 10 Step 4) adds `cargo fmt --all --check` and the pad fixture tests before its own tests. Two of the three masking gates on Sunday failed on rustfmt and a missing fixture, not on logic (22:09Z, 22:18Z). The conductor's one local test merge (long-runs Task 5 Step 1) becomes the train assembly for all branches, pushed as one combined head to one `.req`.
- **Saves:** each avoided gate FAIL is 8 to 10 minutes plus a bisect of 2 to 3 more gates (30 minutes) on a 5-branch train.
- **Risk:** none.

### R8. Name the participant-text seam as a code-wave lane with a deadline

- **Change:** README lists "the other FINISH-PLAN code lanes" without naming them. The PRD-10 seam ("N key values were not added to masking." on start, status and the end screen; section 12 wording; the Hermes line) is point 6 evidence (FINISH-PLAN section 2) and changes `crates/goalcup/src/cli/text.rs`. It must be in the train before H2 or it costs a build and the H2 rows (R4). Size S, sonnet, one read-only review. Same for the PRD-13 provenance comments and `check-links.py` delete (comments only, no behaviour): fold into the same train or defer to 1.0.1; do not give them a slice box.
- **Saves:** one build and up to 18 rerun boxes if it would otherwise land after H2.
- **Risk:** low; wording only.

### R9. Time-box the fix round to 3 hours and ship the rest as "not verified: reason"

- **Change:** README row 5 gives the fix round Mon 08:00 to 14:30 PT (6.5 hours), the longest slot in the plan. Set a hard stop: every non-PASS row whose cause is not fixed in one batch within 3 hours ships "not verified: <screen or cause>" (long-runs open decision 3 already allows phase 5 tiers to move in 1.0.1). Known flaky causes (Gemini Shell prompt, Crush AGENTS.md init screen, Kimi approvals, OpenClaw `source_changed`) go straight to the reason column, not the fix list, unless the fix is data only.
- **Saves:** 3 to 4 hours of wall time and the second-round reruns the plan already forbids (S7).
- **Risk:** fewer verified phase 5 harnesses in 1.0.0. The seven points are proven on the core pair plus Hermes; breadth is a 1.0.1 item by the plan's own decision 3.

### R10. Cut the big tests to what cannot be learned before the tag

- **Change:** long-runs Task 13 Step 2 drops the 9-core-row repeat on `v1.0.0` (open decision 1: run the core set once, on HF). Keep A4 (Kilo gateway, 20 minutes), the extra `full` rows for phase 5 harnesses that passed their composite row (they feed 1.0.1 tiers), and A1 (2 hours, Codex) only if the owner wants the run 1 comparison; otherwise A1 is a post-event row.
- **Saves:** 18 boxes and about 10 USD model; with A1 dropped, 2 more boxes and up to 15 USD. Big tests: 10 to 20 boxes instead of 20 to 38.
- **Risk:** harness versions of 5 Nov differ from the row day; FINISH-PLAN Wave 4 step 11 covers that with a direct-only rerun on the day if the owner wants it.

### R11. Trim the two tooling plans to what the week needs

- **Handover plan (10 tasks, 9 red/green test files, `install.sh --night`, `fresh-clone.sh`, `spend.mjs`, `snaps`):** the 23:41Z push already put tools, ledger, reports, state, secret names, draft and prompt on a branch. Keep Task 7 (`keyscan.py`, used by every row), Task 5 (`padrun.sh`), the Task 3 patch note in `handoff-head.md`, the Task 1 `GC_SESSION` trailer, and `box.mjs --vcpus` from Task 2 (needed for the 2 and 4 vCPU rows). Defer `install.sh --night`, `fresh-clone.sh`, `spend.mjs`, `snaps`, `test_docs.sh` and the red/green replay to after the release unless a fresh container actually happens. One sonnet lane, one box.
- **longsum.py (long-runs Task 4, 8 tests, markdown, live projection):** the live projection duplicates the pad's own spend check (`spend_every_min = 5`, cap seal). Keep `summary` with X2, X3, X4 and `usd_per_hour`; drop `live`, `markdown` and the run 1 column (a reviewer can write the two-column table from the JSON). About half the code and tests.
- **mini Task 7 (Cline package search):** defer; Cline ships "not verified: needs sign-in" either way for 1.0.0.
- **Saves:** 4 to 8 agent-hours of sonnet work and reviewer time; 1 to 2 boxes. No wall-time change if these lanes truly run in parallel, but every lane competes for the conductor's attention and the train.
- **Risk:** a fresh container mid-week would need the manual TOOLS.md install (7 steps, documented).

### R12. Fewer most-capable reviewer passes

- **Change:** the matcher branch gets two default-model reviewers (Task 1), a default-model whole-branch review (Execution) and already had Reviewer A and the breaker on Sunday (ledger 23:17Z, 21:54Z). One correctness review of the new tests and the measured numbers is enough. Long-runs Tasks 1 to 4 (pad Python and a conductor script) list "two read-only reviewers per lane, default": one sonnet reviewer each. Keep the default model for: the rig-slice whole-branch review (it changes what rows mean), the trust first-output read, any recorder, masker or proxy fix, and the final whole-branch review before HR. Keep the three-hourly reviewer but let it read the ledger and row folders only, never start a lane.
- **Saves:** model spend for the heaviest tier (not in any plan's budget; the subagent swarm is the largest model cost of the finish) and reviewer latency on the critical path.
- **Risk:** none to the product; the gate and the rows are the real check.

### R13. Shorten boxes: `box_extra_minutes` and vCPUs

- **Change:** `pad.toml` default `box_extra_minutes = 30` holds a 20-minute full row's box for 50 minutes. The mini rows already set 10 (Task 1). Set 10 in `core-*.toml` and 15 in `a2/a3/a5.toml` too (a row stuck past its minutes plus 15 is a stuck row, S6). Builds and gates stay at 8 vCPU (Rust build time); pad rows 4 vCPU, reruns 2, index 1 (already planned).
- **Saves:** about 25 vCPU-hours through the release (section 3), roughly a quarter of the infra bill, at no box-count change.
- **Risk:** a slow harness exit at the end of a long run; the pad's cleanup copies `rec/` before the box ends, so set the long-run extra at 15, not 10.

### R14. Owner asks: one batched message tonight, with the default the conductor will use at a stated time

- **Change:** README "Owner decisions still open" lists 13 items with no defaults. Send one message now with: ask 7 (Mac run tonight: default yes, because it publishes nothing), ask 6 (freeze: propose Mon 2 Nov 18:00Z if the H2 evidence is complete, else Tue 18:00Z), ask 2 (headerless PEM: default "stated loss with a note" for Goose and Kilo until ruled), ask 5 (cap 15 only for a rerun), ask 9 (top-up before A4 and the big tests only), decision 1 (core set once). The conductor cannot decide owner matters, but it can run what does not depend on them (everything in R1 to R5) and state what it will do if no answer arrives by Mon 13:00Z.
- **Saves:** the owner-decision stalls, which are the only stalls the fast timeline cannot absorb.
- **Risk:** the owner overrides a default after rows ran; those are diagnostic rows, so the cost is a rerun, not a wasted release.

## 3. Boxes, builds, gates, rows and spend

Box time is what costs money, not the count: Sunday's 2,800 boxes for about 100 USD is about 3.5 cents a box, driven by about 120 eight-vCPU gates and pad boxes held 30 minutes past their rows. Count both.

### 3.1 The plans as written (README budget)

| Wave | Boxes | Builds | Gates | Rows |
|---|---|---|---|---|
| Handover | 1 to 2 | 0 | 1 | 0 |
| Code wave and train | about 10 | 0 | 1 | 0 |
| H1 and mini wave | about 28 | 1 | 0 | 13 to 14 |
| Fix round | about 14 | 1 | 1 | about 5 |
| One-hour runs | 6 | 0 | 0 | 3 |
| Freeze to release | about 22 | 1 | 2 | about 9 |
| Through release | about 80 | 3 (up to 5) | 5 | about 31 |
| Big tests | 20 to 38 | 0 or 1 | 0 | 10 to 19 |

### 3.2 With R1 to R13

| Wave | Boxes | Builds | Gates | Rows | vCPU-hours (approx.) |
|---|---|---|---|---|---|
| B0 build on `04044320` | 1 | 1 | 0 | 0 | 5 (8 vCPU, 35 min) |
| Canary pair tonight | 4 | 0 | 0 | 2 | 4 |
| Slice boxes (rig, matcher release box, Wave A, handover) | 4 to 6 | 0 | 0 | 0 | 8 |
| Rest of the composite wave (11 rows) | 22 | 0 | 0 | 11 | 19 |
| One train, one gate | 1 (2 to 4 on a bisect) | 0 | 1 | 0 | 1.5 |
| H2 build | 1 | 1 | 0 | 0 | 5 |
| Fix-round reruns (about 5 rows at 2 vCPU) | 10 | 0 | 0 | 5 | 4 |
| A2, A5, A3 | 6 | 0 | 0 | 3 | 16 (4 vCPU, 75 min with extra 15) |
| Core 9 rows on H2 (extra 10) | 18 | 0 | 0 | 9 | 21 |
| HF: gate full plus build (or gate only if the guarded tree equals H2) | 1 | 1 (0) | 1 | 0 | 6 |
| Table train gate | 1 | 0 | 1 | 0 | 1.5 |
| N6, newcomer | 2 | 0 | 0 | 1 | 3 |
| Pre-freeze fix train, if any | 1 | 0 | 1 | 0 | 1.5 |
| **Through release** | **about 72 to 78** | **3** | **4 to 5** | **about 31** | **about 95** |
| Big tests (R10: A4, up to 8 extra rows, A1 optional) | 10 to 20 | 0 to 1 | 0 | 5 to 10 | 15 to 30 |

Infra: about 95 vCPU-hours through the release, 110 to 125 with the big tests. At Vercel Sandbox list rates (roughly 0.13 USD per active vCPU-hour plus memory at about 2 GB per vCPU) that is about 15 to 20 USD through the release and 20 to 30 USD in all, against about 100 USD spent so far. Confirm the rate and the running total with `box.mjs spend --since 2026-11-01T23:00:00Z --ledger` after the canary pair, before trusting these numbers. Without R13 add about 25 vCPU-hours (about 4 USD).

Model (pad estimates, Claude Code pinned to Haiku): canary and composite wave 3.5 to 5 USD; fix reruns about 2; A2, A5, A3 about 10 (cap 30); core 9 rows about 12; big tests 10 to 25. About 40 USD likely through the release, 50 to 65 in all, within the FINISH-PLAN's 70 to 85. Not budgeted anywhere: the subagent swarm itself, which at Sunday's pace was the largest model line; R11 and R12 are the only levers on it.

### 3.3 Waste found

- H1 (a build superseded within hours by H2) and the 15 hours of pad idle time before it: R1, R2.
- A third train (handover) and its gate before any row: R3.
- 18 core-row boxes placed after the freeze, where a failure costs a rebuild and a rerun: R4.
- Boxes held 30 minutes past their row: R13.
- A 9-core-row repeat on `v1.0.0` that proves the same tree again: R10.
- Reviewer passes by the most capable model on already-reviewed or trivial diffs: R12.
- The fix round sized at 6.5 hours: R9.
- One real duplicate inside the rig: an index box per row (`pad-dbf-*`) where one index fetch per wave would do. It is a rig code change; at 1 vCPU for 5 minutes it is not worth a slice this week. Note it for 1.0.1.

## 4. Model per task

Sensible overall: default model for recorder, masker and proxy fixes, the breaker and final reviews; sonnet for implementation, rig data, docs, running rows and reading results; haiku for queue ticks, keyscans, copies and 5-minute spend ticks. Two corrections, both in R12: the matcher does not need three more most-capable reads after Sunday's two, and Wave A's pad Python does not need two default-model reviewers per lane. One addition: the three-hourly reviewer and the fix-round triage (deciding which non-PASS rows are fixable in 3 hours, R9) are the two judgement calls where the most capable model pays for itself; keep it there.

## 5. Over-engineered for the time left, and what is missing

Cut or defer (YAGNI): handover Tasks 8 to 10 beyond the keyscan, padrun, patch note and `--vcpus` (R11); `longsum.py` live projection and markdown (R11); mini Task 7 Cline search (R11); the `v1.0.0` core-set repeat (R10); matcher Wave B stays deferred (already right); PRD-13 provenance comment rewording and `check-links.py` delete to 1.0.1 unless they ride the train for free (R8).

Essential and present but buried: long-runs Task 1 (row identity: `goalcup`, `os`, `pad_trusted_at`). Review Focus 1 of that plan says every real row today carries nulls, so the release check can never pass. This is the most important code task of the finish and it sits as "Wave A" without a deadline. It must be in the train before H2, and the trust read (Task 6 Step 2) must happen on the first H2 row, or the core rows of R4 cannot be stamped.

Missing or under-named:

1. The PRD-10 participant-text seam (R8): point 6 evidence, changes `crates/`, no plan owns it.
2. A canary on `04044320` tonight (R2): the plan's first row on the masking package is Monday 07:00 PT, 33 hours after the merge.
3. Mac build tonight (R6).
4. The masker cost line in PRD-02 and PRD-05 (FINISH-PLAN "Core docs"): docs-only, any time before HR; name a lane.
5. `tiers.toml` is compiled into `goalcup` (`start.rs:26`), so the table train (Task 8 Step 6) changes the binary after HF. Either the release check must accept that `connectors/tiers.toml` is excluded from the guarded tree (Task 13 Step 1 already excludes it, so it probably is) or HR needs a build. Verify once in `table:126-167` before the freeze, not after.
6. Gate time is now 410 to 450 s (it was 140 to 250 s on Saturday). Nothing in the plans explains the growth; if it is the masking tests or the heavy group, a gate box at 8 vCPU with `--minutes 20` instead of 60 still fits and caps a hung gate.

## 6. Where it will stall

1. Owner decisions, in this order of damage: ask 7 (Mac tonight), ask 6 (freeze time), ask 2 (PEM: decides Goose and Kilo), decision 1 (core set once). Asks 5 and 9 do not stall Wave L (R5).
2. The canary on `04044320`: if the masking package fails to start or masks nothing, every row waits for a fix in the train; the fast timeline absorbs about 3 hours of this, the plan's timeline none (it would discover it Monday 07:30 PT).
3. The train gate: about 8 minutes per attempt; a FAIL on a 5-branch train is a 30-minute bisect. R7.
4. Flaky harnesses in the fix round: Gemini, Crush, Kimi, OpenClaw. R9 bounds it.
5. The trust read: a human-quality read of the selftest and the first H2 row blocks stamping; schedule the reviewer for the moment H2's first row ends.
6. Mac red after the freeze: new HF build, 18 rerun boxes. R6 removes it.
7. Gateway credit (13.82 USD) runs out during the big tests, not before (R5), unless A3 reruns.

## 7. Timeline to release

PT is PST (UTC-8). "Fast" assumes the owner answers ask 7 tonight and ask 6 by Mon 13:00Z; "Conservative" keeps the owner's proposed Tue 18:00Z freeze. Both keep 3 builds and 4 to 5 gates.

| Step | Fast PT | Fast UTC | Conservative PT | Conservative UTC |
|---|---|---|---|---|
| B0 build on `04044320`; code lanes start (rig, Wave A, matcher check, participant text, trimmed handover); batched owner message; Mac `release.yml` on ask 7 | Sun 16:00 | Mon 00:00 | same | same |
| Canary pair on B0 (Claude Code and Codex gateway, composite) | Sun 17:00 to 17:30 | Mon 01:00 to 01:30 | same | same |
| Rig slice box and review; composite wave of the other 11 rows on B0 from the rig worktree, pool 8 | Sun 18:30 to 19:30 | Mon 02:30 to 03:30 | same | same |
| Fix list (3-hour box); fix branches; one train assembled and gated | Sun 19:30 to 23:00 | Mon 03:30 to 07:00 | Sun 19:30 to Mon 01:00 | Mon 03:30 to 09:00 |
| H2 build with selftest; rerun failed rows at 2 vCPU; trust read on the first H2 row | Sun 23:00 to Mon 00:30 | Mon 07:00 to 08:30 | Mon 01:00 to 02:30 | Mon 09:00 to 10:30 |
| A2, A5, A3 one-hour runs in parallel (cap 10, gateway credit as is) | Mon 00:30 to 02:00 | Mon 08:30 to 10:00 | Mon 02:30 to 04:00 | Mon 10:30 to 12:00 |
| Judge; core 9 rows on H2 (two batches under the per-key limits) | Mon 02:00 to 04:00 | Mon 10:00 to 12:00 | Mon 04:00 to 06:00 | Mon 12:00 to 14:00 |
| Owner reads the morning note: H2 evidence complete or not; freeze decision | Mon 05:00 | Mon 13:00 | Mon 05:00 | Mon 13:00 |
| Pre-freeze fix train if Wave L or a core row failed (one gate, new H2 build only on a `crates/` change) | Mon 06:00 to 09:00 | Mon 14:00 to 17:00 | Mon 06:00 to Tue 09:00 | Mon 14:00 to Tue 17:00 |
| FREEZE; HF box (gate full plus build, or gate only); re-stamp and import; table train; N5; staging; Mac result read | Mon 10:00 to 13:00 | Mon 18:00 to 21:00 | Tue 10:00 to 13:00 | Tue 18:00 to 21:00 |
| N6 box, newcomer box, N7, N9 | Mon 13:00 to 14:30 | Mon 21:00 to 22:30 | Tue 13:00 to 14:30 | Tue 21:00 to 22:30 |
| ACCEPTANCE.md, MORNING.md, NIGHT-LOG, HANDOFF; N8 read; final whole-branch review | Mon 14:30 to 18:00 | Mon 22:30 to Tue 02:00 | Tue 14:30 to 18:00 | Tue 22:30 to Wed 02:00 |
| Owner release steps (merge to main, gate full box, tag, `release.yml`, Mac smoke, production deploy) | Tue 09:00 to 13:00 | Tue 17:00 to 21:00 | Wed 09:00 to 13:00 | Wed 17:00 to 21:00 |
| Big tests on `v1.0.0` (A4, extra rows, A1 optional), staging only | Tue 13:00 to 19:00 | Tue 21:00 to Wed 03:00 | Wed 13:00 to 19:00 | Wed 21:00 to Thu 03:00 |
| Buffer before the event opens Thu 5 Nov 16:00 PT (Fri 00:00Z) | Wed and Thu | | Thu | |

Against the README's Wed 4 Nov 14:00 PT release: the fast path releases Tue 3 Nov midday (about 26 hours earlier) and the conservative path Wed morning (about 5 hours earlier, with the whole proof done 30 hours before the freeze instead of 17). The difference between the two columns is one owner answer (ask 6), so the night's work is identical either way; the owner only chooses when to freeze.

## 8. What not to change

The composite row design, the 2-minute canary, the one fix round with failed-rows-only reruns, the per-key queue, the 8-row pool, keys through `put-env` only, reviewers that never start a box, Hermes C2 on task logs with the plain note, deltas-only stated-loss PASS, Wave B deferred, the N5 refuse check and the freeze rule (post-freeze merges only for failed release rows) are all right and cheap. Keep them.
