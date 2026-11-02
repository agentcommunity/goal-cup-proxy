# goalcup 1.0 finish: the four plans

Shape and budget follow the two owner sections at the end of `/root/gc-night/finish/WRITING-PLANS.md`, with the two 1 Nov reviews (correctness, speed) applied. Where they differ from `FINISH-PLAN.md`, the owner sections win. Times are rough: PT is PST (UTC-8). Plans name model classes (most capable, mid-size, smallest) under the owner's format; a model id appears only as a reference to a pad data file. Owner asks use FINISH-PLAN section 5 numbers, with the `conductor/STATE.md` number where it differs.

## Plans

| Plan | What it delivers | Branches |
|---|---|---|
| `2026-11-01-conductor-handover.md` | Portable conductor tools in `docs/plans/2026-10-30-v1.0/conductor/` (one layout: tools at the top, `ledger/`, `reports/`), `padrun.sh`, `keyscan.py`, `rowinfo.py`, `box.mjs --vcpus`, proxy patch, `handoff.sh`; installed to `/root/gc-tools` after the train. Off the critical path | `v1.0/conductor-handover` |
| `2026-11-01-exact-value-single-pass-matcher.md` | The PRD-05 trie matcher checked, measured, merged before the one-hour runs. Its Wave B (`LiteralTrack`) is deferred to after 1.0 | `v1.0/prd-05-masking-exact-trie-matcher` |
| `2026-11-01-harness-mini-test-wave.md` | Part A: the PRD-12 mini rig slice. Part B: build B0 tonight, the canary pair at once, every other composite row from the rig worktree beside the code wave, one 3-hour fix round inside the one train, build H2, rerun failed rows | `v1.0/prd-12-mini-rig`, `v1.0/prd-12-mini-fix` |
| `2026-11-01-long-runs-and-reports.md` | Wave A code (row identity, tier note, run files, `longsum.py`, PRD-10 participant text); Wave L one-hour runs and the 9 core rows on H2; freeze, release rows, reports, owner checklist, big tests | `v1.0/prd-12-release-rows`, `v1.0/conductor-longsum`, `v1.0/prd-10-participant-text`, `v1.0/prd-12-release-table` |

Heads: B0 = `04044320`, the current integration head with the masking package, built tonight for the diagnostic composite wave. H2 = the one train's head. HF = the freeze head (gate full only, reusing the H2 build, when the guarded tree did not move). HR = head after the table merge. Builds: `/root/gc-night/builds/<commit8>/` (`goalcup`, `goalcup-verify`, `commit`), made by the mini plan's Task 9 Step 0 job, which also runs the pad selftest.

## The one train

Lanes push and log `ready for the train`; they write no `.req`. `mergeq.sh` runs `merge.sh` once per request, and every `merge.sh` run is one gate, so four lane requests would be four gates. `merge.sh` takes a comma-joined branch list and merges all of it before its one gate, so the conductor writes ONE request (mini plan Task 11 Step 3) naming the trie, the rig slice, the release rows (row identity must be in it), `longsum`, the participant text, the handover and the fix branches. Bisect: requeue a comma-joined subset. There is no separate handover gate.

## Wave order

| # | Wave | Plan, tasks | PT | UTC |
|---|---|---|---|---|
| 1 | Build B0 (one box, selftest). Code lanes start: trie check, rig slice, Wave A, participant text, handover. One batched owner message. Mac `release.yml` on the owner's yes (ask 7) | mini 9 Step 0; matcher 1-3; mini 1-8; long-runs 1-4a, 8 Step 9; handover 1-10 | Sun 1 Nov 16:00 | Mon 2 Nov 00:00 |
| 2 | Canary pair on B0 from a worktree at `04044320` (Claude Code and Codex gateway, composite) | mini 9 Steps 1-3, 10 Step 1 | Sun 17:00 to 17:30 | Mon 01:00 to 01:30 |
| 3 | Rig slice box; the other 11 composite rows on B0 from the rig worktree, pool 8, diagnostic, beside the code wave | mini 8 Step 1, 10 Steps 2-4 | Sun 18:30 to 19:30 | Mon 02:30 to 03:30 |
| 4 | Fix round, hard stop 3 hours; the one train (one request, one gate) | mini 11 Steps 1-3; matcher 3; long-runs 4a, 5; handover 10 Step 7 | Sun 19:30 to 23:00 | Mon 03:30 to 07:00 |
| 5 | Build H2 (selftest); rerun failed rows at 2 vCPU; trust read on the first H2 row | mini 11 Steps 4-5; long-runs 6 | Sun 23:00 to Mon 00:30 | Mon 07:00 to 08:30 |
| 6 | A2, A5, A3 one-hour runs at once on H2 (needs asks 5 and 9) | long-runs 7 Steps 1-6 | Mon 2 Nov 00:30 to 02:00 | Mon 08:30 to 10:00 |
| 7 | The 9 core release rows on H2, in the gap before the freeze | long-runs 7 Step 7 | Mon 02:00 to 04:00 | Mon 10:00 to 12:00 |
| 8 | Owner reads the morning note and picks the freeze; `tiers.toml` guarded-tree check | long-runs 8 Step 0 | Mon 05:00 | Mon 13:00 |
| 9 | Pre-freeze fix train, only if a Wave L or core row failed | long-runs 7 Step 5 | Mon 06:00 to 09:00 | Mon 14:00 to 17:00 |
| 10 | FREEZE; HF gate full (plus build only if the guarded tree moved); re-stamp and import; table train; N5; staging; Mac result | long-runs 8 | fast: Mon 2 Nov 10:00 to 13:00; default: Tue 3 Nov 10:00 to 13:00 | fast: Mon 18:00 to 21:00; default: Tue 18:00 to 21:00 |
| 11 | N6, newcomer, N7, N9 | long-runs 9 | freeze + 3 h to + 4.5 h | same |
| 12 | Reports: `ACCEPTANCE.md`, `MORNING.md`, `NIGHT-LOG.md`, final whole-branch review | long-runs 10, 11 | freeze + 4.5 h to + 8 h | same |
| 13 | Release (owner steps) | long-runs 12 | fast: Tue 3 Nov 09:00; default: Wed 4 Nov 09:00 | fast: Tue 17:00; default: Wed 17:00 |
| 14 | Big tests on `v1.0.0` (A4, extra rows, A1 optional), staging only | long-runs 13 | release + 4 h to + 10 h | same |

The event opens Thu 5 Nov 16:00 PT (Fri 6 Nov 00:00Z). Tests never run on the conductor machine: every build and test runs in a box or in a merge-queue gate. Composite rows (waves 2, 3) are diagnostic: they claim no tier and no release row.

## Box budget (about 75 boxes, 3 builds, 4 to 5 gates, about 31 rows to the release)

| Wave | Boxes | Builds | Gates | Rows |
|---|---|---|---|---|
| B0 build | 1 | 1 | 0 | 0 |
| Canary pair and the other 11 composite rows | 26 | 0 | 0 | 13 |
| Slice boxes (rig, matcher, Wave A, participant text, handover) | 5 or 6 | 0 | 0 | 0 |
| The one train | 1 (2 to 4 on a bisect) | 0 | 1 | 0 |
| H2 build and fix-round reruns (about 5 rows at 2 vCPU) | 11 | 1 | 0 | 5 |
| A2, A5, A3 | 6 | 0 | 0 | 3 |
| 9 core rows on H2 (`box_extra_minutes = 10`) | 18 | 0 | 0 | 9 |
| HF gate full (build only if the guarded tree moved), table train gate, N6, newcomer | 4 or 5 | 0 or 1 | 2 | 1 |
| Pre-freeze or post-freeze fix train, if a row fails | 1 | 0 or 1 | 1 or 2 | 0 |
| **Through release** | **about 75** | **3 (B0, H2, HF or H2 reused)** | **4 to 5** | **about 31** |
| Big tests (after the release, no core-set repeat) | 10 to 20 | 0 or 1 | 0 | 5 to 10 |

Every row counts 2 boxes (the pad box and its index box). Concurrency: up to 8 row boxes at once. Boxes are held for their minutes plus `box_extra_minutes`: 10 on composite and core rows, 15 on the one-hour runs.

## Owner decisions still open

1. Freeze time (ask 6): fast path Mon 2 Nov 10:00 PT (18:00Z) if the H2 evidence is complete, or Tue 3 Nov 10:00 PT (18:00Z). Default Tue unless the owner says otherwise. The night's work is the same either way.
2. Mac `release.yml` started tonight on the current integration head (ask 7: metered Mac runner minutes, nothing published), and a Mac smoke slot. The owner confirms; without a yes it waits for the freeze.
3. Masked-upstream C3 (ask 4; STATE.md ask 6): does a row whose harness redacted the plants itself (`facts.masked_upstream`) count for the release check? `table --release-check` refuses it today (`table:142-146`); Hermes direct is a release blocker if it still sets it.
4. 15 USD cap for each one-hour run (ask 5) and a gateway credit top-up (ask 9): both gate Wave L.
5. Headerless PEM body: a goalcup miss or a stated loss? This decides the Goose and Kilo C3 results (ask 2).
6. "Up to 8 boxes": the plans count row boxes only, with each row's index box on top (16 at peak). Confirm, or the wave runs about 4 rows at once (ask 1).
7. If a canary fails after minute 2: kill the running rows, or let them finish for evidence?
8. Kill gap bound: `check.py` allows under 2 s, X4 allows 5 s under load. An A5 gap of 2 to 5 s costs one extra Codex kill row.
9. Phase 5 harnesses ship in 1.0.0 as "not verified: no full rows", and their tiers can move in 1.0.1. The fix round ships what it cannot fix in 3 hours as "not verified: <reason>".
10. Matcher: the 50 MB/s target holds for the matcher alone, not for one whole stream (14.6 MB/s from the shared rules). Accept? Keep the lane's own trie, or switch to the `aho-corasick` crate already in `Cargo.lock`?
11. STATE.md asks 16 (copy the draft lines into `OWNER-DECISIONS.md`; agents may not) and 17 (Amp and Factory accounts, or both ship "not verified, no key").
12. Settled by the reviews, unless the owner objects: the core set runs once, on H2 before the freeze, with no repeat on `v1.0.0`; A1 runs only if the owner wants the run 1 comparison.
13. Not blocking the waves: Gemini Shell prompt (ask 10), the mini-full prompt (ask 11), size overrun (ask 12), key rotation after the last run (ask 13), run 1's prompt for A1 (ask 14), rung 5 L3 (ask 15). Ask 8 (build reuse) now affects only the `v1.0.0` build in long-runs Task 13 Step 1.
