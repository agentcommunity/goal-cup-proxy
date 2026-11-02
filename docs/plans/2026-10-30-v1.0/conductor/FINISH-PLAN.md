# Goal Cup 1.0: finishing plan

Written Sun 1 Nov 2026, 22:52Z (14:52 PT). For a fresh conductor session.

**Time zone note.** Daylight time ended at 02:00 local today. PT is now PST, UTC-8. Every PT time below uses UTC-8.

Sources: `origin/v1.0/integration` at `04044320`, `/root/gc-night/lanes/integrator.log` (to 22:27Z), `reviews/LATEST.md` (21:59Z), `backlog/next-pad-verify.md`, the pad `REPORT.txt` files. Where this plan infers rather than reads, it says so.

## 1. Where we are

1. The integration head is `04044320`, the masking package (exact-value masking sources 1-3), merged at 22:27Z after gate fast passed on the third try.
2. No pad row has run on this head. `/root/gc-night/builds/` is empty. Masking, the Hermes `state.db` drop, the model pin and the new C3 rules have never been tested together.
3. PRD-00 done items: 2, 3 and 4 are met. Item 1 is 4 of 9 core rows. Items 5 and 6 are open. Item 7 needs only a staging redeploy (no `server/` change since `e626de8f`). Item 8 is shown by tests only. Item 9 is provisional.
4. Only Claude Code and Codex have ever passed all six checks. Gemini passes all but C2. The other harnesses have reached "pass at micro" at best. `connectors/tiers.toml` says "not verified" for all 16.
5. Crush and Cline cannot run yet (permission dialog, sign-in screen). Amp and Droid need vendor accounts the owner does not have.
6. Already merged, so not Wave 0 work: G1 (`pad-run:424` reads case `pre_keys` first), P0a (index box, `pad:701-751`), P0b (`step_cleanup` keeps a clean `rec/` after a failed fetch), P1 (pad image with all 16 harnesses, `2157b7c3`), D1 and D2 (in `04044320`), V1 (`honesty.rs:384`). Still open in the rig: Codex exit key (R5-4), OpenClaw workspace, crushrc, prompt variant, D3.
7. The single-pass trie matcher for exact values is in progress on `v1.0/prd-05-masking-exact-trie-matcher` (`/root/gc-wt/builder-exact-trie`). The owner wants it before the 2-hour runs.
8. The macOS code (24 `cfg(target_os = "macos")` blocks) has never compiled in 1.0. The recover `/proc/<pid>/environ` reader is Linux only (a stated loss on Mac).
9. The record lags. `NIGHT-LOG.md` ends at 13:01Z, `MORNING.md` is the 09:59Z interim, `HANDOFF.md` is from 31 Oct, and the ledger lines after about 15:06Z are not in the repo. `OWNER-DECISIONS.md` misses about eight answers; agents may not edit it (owner only).
10. Model spend so far is about 27 USD billed. Sandbox minutes have never been measured.
11. Since mid-morning only one lane (masking) has been busy. Participant, core and docs have been idle for 8 to 14 hours. This plan goes back to several lanes at once.

## 2. Definition of done for 1.0

Each of the seven points needs evidence on the frozen head, not on an older commit.

| Point | Evidence needed |
|---|---|
| 1. Record every model call | Core set 9 of 9 (Claude Code, Codex, Hermes x direct, gateway, dodger) with C1 PASS on the frozen head. A1 and A2 (2-hour and 60-minute runs) pass C1. The HTTP/2-only run and `long-tunnel.sh` pass (done item 9). |
| 2. Record every subagent | C2 PASS on every core row. A2 records 20 subagents. Hermes passes on task logs with the plain note "subagents shown by their task logs, not tied to individual calls" (owner 22:03Z). |
| 3. Never block or slow the harness | Codex kill row and A2's `kill -9` at minute 30 plus watchdog recover at 45 pass. The masker cost (2 to 6.4 s at boot, 238 MB at 32 values) is written into PRD-02 and PRD-05, and a Mac measurement is under about 2 s or the value build moves after `ready`. |
| 4. Never record secrets | C3 PASS on every counted row (deltas-only shapes pass with the "stated loss" note). N6: `goalcup-verify secrets` over every kept record, the server copy, `pad-out/`, NIGHT-LOG, MORNING and lane logs, in one box, with 0 hits. |
| 5. Upload everything | C4 PASS on every staging row (fetch through the short-lived index box). Staging redeploy from the frozen head (done item 7). |
| 6. Tell the participant plainly | Participant text for exact-value masking and the Hermes limit is merged. W1 and W2 wording fixed. The newcomer docs run (PRD-15 acceptance 1) passes. |
| 7. Work with the harnesses people use | `tiers.toml` and the published table written from real rows (done items 5 and 6): every harness is "verified", "verified with a note" or "not verified: reason". Mac: `release.yml` green on both Mac targets and one owner Mac smoke. |

Also required: N5 (the release check accepts the verified commit and refuses a changed one), N8 (the report reads in 10 minutes), N9 (no stray boxes, no `gc-pad-*` repos, no key files kept), and the owner's call on the size overrun (about 39.9k Rust lines against an 18.1k budget).

## 3. Waves

Lanes run in parallel through the lane script (`gc-lane.js`), as at the start of the build. Pad waves run alongside the code lanes. Every merge goes through the merge queue (`mergeq.sh` driven by the conductor, gate fast in a box).

**Box rules for every wave.** One shared release build per integration commit in `/root/gc-night/builds/<commit>/`. At most 2 pad boxes at once. No probe boxes. Gate only on real merges. Smallest box size, short timeouts, stop stuck rows early. Reviews and reports never start a box. Claude Code runs on the pinned cheap model only. Keys only through `put-env` (0600), never on a command line.

### Wave 0: handover and one shared build

**When:** Sun 1 Nov 23:00Z to Mon 2 Nov 03:00Z (Sun 15:00 to 19:00 PT).

**Lanes (in parallel):**

| Lane | Work | Box |
|---|---|---|
| Conductor | Install `tools/`, run `snaps.mjs` once (lists snapshots and running boxes) and record the build snapshot (`build-snapshot.id`) and pad image (`scripts/pad/image.id`) ids and expiry. Change the hard-coded session trailer in `merge.sh`, `scribe.sh`, `handoff.sh` and `gc-lane.js`. Start the queue tick. Start the `spend` ledger line: box minutes per box, from the Sandbox list. Replay `04044320`'s `check.py` on the full3, full4 and gw3 Hermes folders (no box) to see if a clean Hermes row still gets C3 INCONCLUSIVE for `private_key` (review 6, item 3.6). | 0 (snapshot listing is an API call, not a box) |
| Scribe | Commit conductor state to `docs/plans/2026-10-30-v1.0/conductor/` (tools, ledger, reports, `STATE.md`, `branches.md`, `LESSONS-CONDUCTOR.md`, `SECRETS.md` with names only). Run `keyscan.py` over the tree first, 0 hits required. Bring `NIGHT-LOG.md` up to 22:27Z. Extend `handoff.sh` to copy `mq-driver.sh`, `package.json`, `package-lock.json`, `snaps*.mjs`, `build-snapshot.id`, one `keyscan.py`, and the ledger (today it copies only `box.mjs`, the four scripts and `gc-lane.js`). Write the sandbox proxy patch into `handoff-head.md`: it is written down nowhere on this machine (not in `gc-tools`, the lane logs or the repo). Draft the missing owner lines (about eight) in `conductor/owner-decisions-draft.md` for the owner to copy; never edit `OWNER-DECISIONS.md`. | 0 |
| Pad rig (PRD-12) | Data fixes still open for the mini-tests (one slice, one gate): Codex exit key (`/quit`, then Enter as a separate key), OpenClaw `agents.defaults.workspace` set to the run folder, crushrc permissions file mapped into `{home}/.config/crush/` (drop the `not_verified` line only if the file is in), the setup-script prompt variant as a settings file, D3 comment. A per-row `cap` setting is optional: today the cap is `cap_usd` in `pad.toml` only (10 USD, not a settings key). | 0 until merge |
| Cline search (PRD-11) | `npm pack cline@3.0.74` on the conductor into an empty folder; read it, never run it. Look for a config file or env route for the provider key. No key in argv. If none, write "not verified: needs sign-in". | 0 |
| Masking (PRD-05) | Continue the trie matcher. Not needed for the mini-tests. | 1 for its gate |

**Order matters if the new session is a fresh container.** `/root/gc-night/` and `/root/gc-tools/` exist only on the old machine. The scribe commit of conductor state (tools, ledger, reports) must be pushed to `v1.0/integration` by the old session before the new session starts, or the new session has no ledger. If `/root/gc-night/` exists when the new session starts, use it directly. A fresh container also needs, set by the owner in the environment settings before it starts: `VERCEL_TOKEN`, `VERCEL_TEAM_ID`, `VERCEL_PROJECT_ID`, `CLAUDE_API_KEY`, `OPENAI_API_KEY`, `DEEPSEEK_API_KEY`, `GEMINI_API_KEY`, `AI_GATEWAY_API_KEY`, `DATABASE_URL` (index box only), push access to `v1.0/*`, and network access to the Vercel API. Check them by name only.

**Inputs:** this plan, the handover survey, the backlog.

**Build early.** Pad data under `scripts/pad/` is read from the worktree, not compiled. Build H1 on `04044320` at the start of Wave 0 and run pad wave 1a at once (it needs none of the open rig fixes; Codex passed all six in gw3 with the current exit key). Rows on later rig-only commits reuse H1 only if the owner says yes to ask 8 and `git diff H1..head -- crates/ connectors/ Cargo.*` is empty, noted in the row; otherwise one new build.

**Exit criteria:** conductor state committed and keyscan-clean; rig data fixes merged; H1 built in `builds/<commit>/`; pad wave 1a result known.

**Box budget:** about 3 gate boxes (rig fixes, maybe trie), 1 build box.

**Model spend:** 0 USD, except pad wave 1a (about 1.9 USD, counted in Wave 1).

### Wave 1: 10-minute harness mini-tests, plus code lanes

**When:** pad wave 1a from about Sun 23:30Z (canary, inside Wave 0); 1b to 1g Mon 2 Nov 01:00Z to 05:00Z (Sun 17:00 to 21:00 PT), while the owner is awake to answer. Code lanes run from 23:00Z through Monday.

**Mini-test settings (one file per row):**

```
harness = "<name>-cli"
key     = "<per harness>"
case    = "direct"        # "gateway" for the Claude Code and Codex rows
mode    = "micro"
prompt  = "smoke"         # one subagent; the prompt-variant file for opencode, qwen, copilot
minutes = 10
server  = "staging"       # so C2, C4 and C5 are judged
where   = "sandbox"
steps   = ["keep-going"]
build   = "/root/gc-night/builds/<H1 commit>"
```

`key` is `claude`, `openai`, `deepseek`, `gemini` or `gateway` (`pad.toml [keys]`). Run with `GH_TOKEN` unset. The cap is `cap_usd` in `pad.toml`; for the mini-tests set it to 3 in the run worktree only (not committed), or use the per-row `cap` key if the rig slice adds it. In the same local copy set `box_extra_minutes` to 10 (default 30, so a 10-minute row holds a 40-minute box). `box_vcpus` stays 4 for 1a; try 2 from 1b if 1a shows headroom (owner rule: smallest box). Check the "Pinned model" line in the first Claude Code row: `claude-haiku-5-5` direct, `anthropic/claude-haiku-5.5` gateway; Codex gateway `anthropic/claude-haiku-5.5`; DeepSeek gateway `deepseek/deepseek-v4.1-flash`. Each row takes about 13 to 14 minutes wall.

**Pad waves (2 boxes at a time):**

| Pad wave | Rows | Why | Est. cost |
|---|---|---|---|
| 1a | claude-code (gateway), codex (gateway) | Canary for masking, model pin and C3 rules. **Stop everything if either fails.** | about 1.9 |
| 1b | hermes (direct), gemini (direct) | First proof of the `state.db` drop and task-log subagents; Gemini parent reader | about 0.5 |
| 1c | opencode2, opencode | opencode2 is the likeliest new clean pass | about 0.2 |
| 1d | kilo, qwen | Both near-passed at micro | about 0.25 |
| 1e | goose, copilot | Goose tests the headerless PEM ruling | about 0.15 |
| 1f | openclaw, kimi-code | Fixes landed, never run | about 0.25 |
| 1g | crush (only if crushrc merged), cline (only if a config route was found), one repeat of the worst failure | Least ready last | about 0.1 to 0.3 |

If the index box counts toward the 2-box limit (owner ask 1), stagger the two rows of each pad wave by about 6 minutes.

**Code lanes (in parallel with the pad):**

| Lane | Work | Size |
|---|---|---|
| Masking (PRD-05) | Trie matcher, review, merge | M, 2 to 3 h |
| Participant (PRD-10) | "N key values were not added to masking." on start, `goalcup status` and the end screen; section 12 wording; the Hermes limit line; W1 and W2 wording | S, about 2 h |
| Core docs (PRD-02 / PRD-05) | Write the masker's stated cost into both PRDs | S |
| Pad rig (PRD-12) | P3 pane copies out of the echo scan; P5 Kimi; G3 rung 4 kill timing; R5-2 and R5-3 (R5-1 L3 only after owner ask 15); H1 and H2. P1 is done (`2157b7c3`); rebuild the image only if a harness version changes. | M |
| Verify (PRD-08) | V2: OpenClaw `history_complete` orphans. Remove V1 from the backlog (fixed at `honesty.rs:384`). | M |
| Native (PRD-04) | N1: OpenClaw SQLite copies causing `source_changed` gaps | M |
| Format tests (PRD-14) | `e2e_run.rs`, then rerun G1 | M |
| Cuts (PRD-13) | Provenance comments that trip the reference guards; delete `check-links.py`. Fresh branch, do not queue the old one. | S |
| Usage (PRD-09) | Close findings A-2r, A-box, A-5 on `usage-job.sh`; model-id negative test | M |
| Mac (conductor) | With the owner's OK, start `release.yml` by hand with ref `v1.0/integration` (the `workflow_dispatch` on `main` allows it; builds four targets, runs the Rust guards, publishes nothing). Fix what breaks (likely `spawn.rs:253` `ps`, `clock.rs:72` boot id). | about 1 h runner time, no box; Mac minutes are metered at a higher rate |

**Exit criteria:** every runnable harness has one 10-minute row on H1 with a result per check; a short results table in `conductor/reports/mini-wave.md`; a list of fixes for Wave 2; Mac build result known.

**Box budget:** 13 to 14 pad rows on 2 boxes, plus index boxes, plus gate boxes for code merges (about 6 to 8). No image build.

**Model spend:** about 2 to 3.5 USD (D2 now prices gateway Haiku at the under-100k rate, so the estimate should sit near the bill; check it on 1a).

**Wall time:** pad about 2 h 10 min. Code lanes through Monday.

### Wave 2: fixes from Wave 1, full rows, and the 2-hour runs

**When:** Mon 2 Nov 16:00Z to Tue 3 Nov 08:00Z (Mon 08:00 PT to Tue 00:00 PT).

**Lanes:**

| Lane | Work |
|---|---|
| Fix lanes | One fresh branch per finding from Wave 1, in the owning PRD. Connector changes compile into the binary, so they merge before the next build. |
| Conductor | One build on the post-trie head (H2) once the fixes have merged. |
| Pad (2 boxes) | Core set on H2, all 9 rows `full`: Claude Code, Codex and Hermes, each direct, gateway and dodger (Hermes dodger has never run), plus `--kill-recorder` on the three direct rows. Then `full` rows for each phase 5 harness that passed its 10-minute row. Rung 9, null mode on a real run, rung 4 kill at 1. |
| Long runs | A2: Claude Code (Haiku), 60 minutes, 20 subagents, `kill -9` at minute 30, watchdog recover at 45, cap 15 USD, on H2 as the early check. A3: Hermes via the gateway (DeepSeek), 30 minutes. A4: Kilo via the gateway, 20 minutes, not a blocker. A1 (Codex direct, 2 hours, cap 15 USD) runs once, on HF in Wave 3, not on H2: fixes after Wave 2 will change `crates/`, which would force a rerun anyway. A1 and A2 start only after the owner answers the cap, and only after the trie merges. |
| Record | `PAD-RESULTS.md` and `ACCEPTANCE.md` drafts filled as rows land. A `spend` line in the ledger with box minutes per row. |

**Inputs:** Wave 1 results, owner answers 1 to 6 in section 5.

**Exit criteria:** trie merged; core rows pass on H2; each phase 5 harness has a full row or a plain "not verified: reason"; A2 and A3 have run with C1 to C6 and X1 to X10 judged.

**Box budget:** 1 build; about 10 core and kill rows; up to 10 phase 5 full rows; 3 long runs (A2 holds a box about 1.5 hours; run A3 and A4 in the other slot); gate boxes per merge.

**Model spend:** core and phase 5 rows about 12 USD; A2 up to 15 USD; A3 and A4 about 5 USD. Total about 32 USD. Top up gateway credit first (13.82 USD left at 21:59Z).

**Wall time:** about 16 hours, with A1 and A2 overnight Monday PT.

### Wave 3: freeze, final core reruns, reports, macOS

**Freeze:** propose **Tue 3 Nov 18:00Z (Tue 10:00 PT)**. That gives the owner a working morning in SF before it. After the freeze, only fixes for failed release rows merge, each named in the ledger.

**When:** Tue 3 Nov 18:00Z to Wed 4 Nov 22:00Z (Tue 10:00 PT to Wed 14:00 PT).

**Lanes:**

| Lane | Work |
|---|---|
| Conductor | Write the freeze in the ledger. `sh scripts/gate.sh full` on the frozen head (one box). One build on the frozen head (HF). Staging redeploy from HF (done item 7, owner action or approved route). |
| Pad (2 boxes) | Core set 9 of 9 on HF, Codex kill row, HTTP/2-only run, `long-tunnel.sh`. A1 on HF (2 hours, start it first; its table next to run 1's). If HF differs from H2 in `crates/` or `connectors/`, rerun A2 on HF; if only in docs or pad data, A2 to A4 stand. |
| Check | N5 release check on HF. N6 secrets scan in one box after all rows. N9 cleanup list. |
| Docs (PRD-15) | `tiers.toml` and the published table from the rows. Guide, README and SECURITY for exact-value masking and the Hermes text. The newcomer run. `LESSONS.md` RR1 to RR6. |
| Record | `PAD-RESULTS.md`, `ACCEPTANCE.md` (A1 to A4 prompts as run), full rewrite of `MORNING.md`, `NIGHT-LOG.md` to the end, `HANDOFF.md`, `STATUS-REPORT.md` refreshed or removed. |
| Mac | Owner runs one Mac smoke on HF (about 30 minutes) and measures masker boot time. |

**Exit criteria:** all of section 2 shown on HF, N6 at 0 hits, the report readable in 10 minutes, owner questions listed at the top of `MORNING.md`.

**Box budget:** 1 gate full, 1 build, 12 to 14 rows, 1 N6 box, A1, and A2 again if needed.

**Model spend:** about 16 USD plus A1 up to 15 USD, plus up to 15 USD if A2 must rerun.

### Wave 4: owner release steps

**When:** Wed 4 Nov 22:00Z to Fri 6 Nov 00:00Z (Wed 14:00 PT to Thu 16:00 PT). The event switch window is 6 Nov 00:00Z to 7 Nov 00:00Z (Thu 16:00 PT to Fri 16:00 PT).

Owner only, from PRD-16 section 14:

1. Read `MORNING.md`; answer its questions.
2. Check, read-only, that no real run is open on production.
3. Merge `v1.0/integration` into `main` (merge commit). If `main` moved since `4b226de2`, rerun the 9 core rows and the Codex kill row on the result.
4. `sh scripts/gate.sh full` on `main` (box).
5. `sh scripts/release.sh tag 1.0.0` (the release check runs first).
6. `release.yml` publishes four targets. Check `install.sh` on Linux and the owner's Mac; run the pad once on the Mac.
7. Production Worker deploy from the owner's device; version floor `1.0.0` last; then delete the `RECEIPT_SIGNING_KEY` secret.
8. Production smoke: one practice run sealed, fetched and checked.
9. Deferred checks: PRD-14 G2, PRD-12 P8.
10. `EVENT_ID` switch inside the window (`wrangler.toml:36-42`).
11. On 5 Nov, rerun the core set direct on that day's harness versions. A failure means a 1.0.1 fix forward.

The conductor never touches production, `main` or tags. Step 11 is pad rows on staging: the conductor may run it on the owner's word.

**Model spend:** about 5 USD for the 5 Nov rerun.

**Whole plan spend:** about 70 USD model in the likely case, about 85 USD if A2 repeats. Sandbox cost is unknown until box minutes are logged, which is why the spend line in Wave 0 matters.

### Biggest risks

- Phase 5 breadth with little room for fix loops. Mitigation: the 10-minute rows find the blockers in about 2 hours instead of a day.
- The Mac build has never compiled. Mitigation: `release.yml` by hand in Wave 1, not at the release.
- A1 and A2 wait on the cap answer and the trie. Mitigation: ask tonight; A1 runs once, on HF.
- A fresh container lacks the secrets, the proxy patch or the tools. Mitigation: the Wave 0 handover list and owner environment settings before the new session starts.
- The gate takes 130 to 450 s per merge and serialises. Mitigation: batch small fixes per lane.

## 4. Conductor rules and lessons

1. The conductor drives the merge queue. Waiters only read `<id>.res`. Every request has an id.
2. Before believing a lane's "not merged", check `git merge-base --is-ancestor` and the ledger.
3. A fresh branch per slice. Before queueing, diff the branch against integration and read its note. Never queue a held branch.
4. Every lane brief says: chat messages are for the conductor; ignore them.
5. Never edit `/home/user/goal-cup-proxy`. Check its `git status` and `git log origin..HEAD` at each wake-up.
6. Any non-docs edit by the conductor goes through `merge.sh`. Docs-only edits to plan-owned paths are the only exception.
7. Cross-PRD edits need a numbered ownership ruling in the ledger before the merge (next is OWN-12).
8. Check premises in code before telling the owner. Four corrections came from that mistake.
9. Cost: Claude Code on the pinned cheap model only; check "Pinned model" in the first row. No probe calls from the conductor. One build per commit. Max 2 pad boxes.
10. Environment: the sandbox SDK needs the proxy patch (write it down in `handoff-head.md`); boxes run as `vercel-sandbox` with Python 3.9 (install 3.11 for the pad); the conductor reaches the database only through the short-lived index box; keys only through `put-env`; unset `GH_TOKEN`, `DATABASE_URL`, `CLOUDFLARE_*` and the approval key in every pad box.
11. Run `scribe.sh` at every wake-up and before any handoff. Never let the ledger run 50 lines ahead of the repo again.
12. Never edit `OWNER-DECISIONS.md`; draft lines for the owner instead. Copy out only `REPORT.txt` files. Never commit `pad-out/`, `runs/`, `dbfetch/idx/`, `.env*` or key files. Run `keyscan.py` before every commit of conductor state.
13. Keep lanes busy. If a lane has been idle for more than 2 hours while work in its PRD is open, give it the next item.

## 5. Owner asks

Already answered, do not ask again: masker cap 32 kept (22:05Z), Hermes C2 on task logs (22:03Z), Codex gateway on `anthropic/claude-haiku-5.5` (merged 22:27Z), deltas-only stated loss (20:32Z). Ask 1, 5, 6, 7 and 8 first: they gate Waves 1 and 2.

1. Does the pad's short-lived index box count toward the 2-box limit?
2. Is a headerless PEM body (goose, kilo, openclaw) a goalcup miss, or a stated loss?
3. Is OpenClaw's `source_changed` a stated loss that passes with a note?
4. Masked-upstream C3 PASS (Hermes redacts itself): does it count for `--release-check`?
5. Per-run cap for A1 and A2: 15 USD each, yes or no; run them before Friday (A2 on H2, A1 once on HF)?
6. Freeze at Tue 3 Nov 18:00Z (10:00 PT): confirm or move.
7. OK to start `release.yml` by hand for the Mac build (metered Mac runner minutes, nothing published); and a slot for your Mac smoke on Tuesday or Wednesday.
8. May a row reuse a build from an earlier commit when no compiled input changed (`crates/`, `connectors/`, `Cargo.*`)? Saves one build per rig-only merge.
9. Top up gateway credit (13.82 USD left at 21:59Z) before Wave 2.
10. Gemini subagent Shell prompt: grade as "verified with a note" when the keep-going nudge works?
11. Mini-full prompt (10 minutes, 3 to 5 subagents): add it, or stay with the one-subagent smoke?
12. Size overrun (about 2.2 times the budget): accept or cut?
13. Rotate the two real keys named at 07:24Z (Gemini, Claude).
14. Put run 1's prompt in a file for A1.
15. Rung 5 setup L3 (launch shell outlives `goalcup start`): confirm (flagged 07:24Z, never answered).
16. Still open from before: S6 scope, the `gsk_` and `sgamp_` rules, PRD-11 setup switches, the cron email on `failed`, question 12.
