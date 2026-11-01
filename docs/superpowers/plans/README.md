# goalcup 1.0 finish: the four plans

Shape and budget follow the two owner sections at the end of `/root/gc-night/finish/WRITING-PLANS.md`. Where they differ from `FINISH-PLAN.md`, the owner sections win. Times are rough: PT is PST (UTC-8).

## Plans

| Plan | What it delivers | Branches |
|---|---|---|
| `2026-11-01-conductor-handover.md` | Portable conductor tools in `docs/plans/2026-10-30-v1.0/conductor/` (`padrun.sh`, `keyscan.py`, `box.mjs --vcpus`, proxy patch, `handoff.sh`), installed to `/root/gc-tools` | `v1.0/conductor-handover` (own small train) |
| `2026-11-01-exact-value-single-pass-matcher.md` | The PRD-05 trie matcher checked, measured, merged. Its Wave B (`LiteralTrack`) is deferred to after 1.0 | `v1.0/prd-05-masking-exact-trie-matcher` |
| `2026-11-01-harness-mini-test-wave.md` | Part A: the PRD-12 mini rig slice. Part B: build H1, one 10-minute composite row per harness, one fix round, build H2 | `v1.0/prd-12-mini-rig`, `v1.0/prd-12-mini-fix` |
| `2026-11-01-long-runs-and-reports.md` | Wave A code (row identity, tier note, run files, `longsum.py`); Wave L one-hour runs on H2; freeze, release rows, reports, owner checklist, big tests | `v1.0/prd-12-release-rows`, `v1.0/conductor-longsum`, `v1.0/prd-12-release-table` |

Heads: H1 = code-wave train head. H2 = fix-round train head (H2 = H1 if nothing needed fixing). HF = freeze head. HR = head after the table merge. Builds: `/root/gc-night/builds/<commit8>/` (`goalcup`, `goalcup-verify`, `commit`), made by the mini plan's Task 9 Step 0 job. That job also runs the pad selftest.

## Wave order

| # | Wave | Plan, tasks | PT | UTC |
|---|---|---|---|---|
| 1 | Code wave: every code lane in parallel, one end box per slice, read-only reviewers. The handover small train merges first and installs `/root/gc-tools` | handover 1-10; matcher 1-3; mini 1-8; long-runs 1-5; the other FINISH-PLAN code lanes | Sun 1 Nov 16:00 to Mon 2 Nov 04:00 | Mon 2 Nov 00:00 to 12:00 |
| 2 | Code-wave merge train: one gate on the combined head, bisect on failure | matcher 3; mini 8; long-runs 5 | Mon 04:00 to 06:00 | Mon 12:00 to 14:00 |
| 3 | Build H1 (one box, selftest included) | mini 9 Step 0 | Mon 06:00 to 06:45 | Mon 14:00 to 14:45 |
| 4 | Composite mini-test wave: about 14 rows, up to 8 at once, Claude Code and Codex 2 minutes ahead as canary | mini 9, 10 | Mon 07:00 to 07:45 | Mon 15:00 to 15:45 |
| 5 | One fix round: one batch, one gate, build H2, rerun failed rows only | mini 11, 12 | Mon 08:00 to 14:30 | Mon 16:00 to 22:30 |
| 6 | One-hour runs on H2: A2, A5, A3 at once; trust read beside them | long-runs 6, 7 | Mon 15:00 to 16:30 | Mon 23:00 to Tue 00:30 |
| 7 | Freeze, gate full and build on HF, only the release rows still missing, table, staging, Mac | long-runs 8, 9 | Tue 3 Nov 10:00 to Wed 4 Nov 04:00 | Tue 18:00 to Wed 12:00 |
| 8 | Reports: `ACCEPTANCE.md`, `MORNING.md`, `NIGHT-LOG.md` | long-runs 10, 11 | Tue 22:00 to Wed 10:00 | Wed 06:00 to 18:00 |
| 9 | Release (owner steps) | long-runs 12 | Wed 4 Nov 14:00 | Wed 22:00 |
| 10 | Big tests on `v1.0.0`, staging only | long-runs 13 | Thu 5 Nov 08:00 to 16:00 | Thu 16:00 to Fri 00:00 |

The event opens Thu 5 Nov 16:00 PT (Fri 6 Nov 00:00Z). Tests never run on the conductor machine: every build and test runs in a box or in a merge-queue gate.

## Box budget (target: about 5 builds, 5 gates, 30 rows, 50 to 80 boxes)

| Wave | Boxes | Builds | Gates | Rows |
|---|---|---|---|---|
| Handover | 1 or 2 | 0 | 1 | 0 |
| Code wave and train | about 10 (one end box per code slice, about 9, plus the gate) | 0 | 1 | 0 |
| H1 and mini wave | about 28 | 1 | 0 | 13 or 14 |
| Fix round | about 14 | 1 (H2) | 1 | about 5 |
| One-hour runs | 6 | 0 | 0 | 3 |
| Freeze to release | about 22 | 1 (HF) | 2 | about 9 |
| **Through release** | **about 80** | **3, up to 5 with a post-freeze fix or a `v1.0.0` rebuild** | **5** | **about 31** |
| Big tests (after the release) | 20 to 38 | 0 or 1 | 0 | 10 to 19 |

Every row counts 2 boxes (the pad box and its index box). Concurrency: up to 8 row boxes at once.

## Owner decisions still open

1. "Up to 8 boxes": the plans count row boxes only, with each row's index box on top (16 at peak). Confirm, or the wave runs about 4 rows at once (ask 1).
2. The big tests add 20 to 38 boxes beyond the target. Run the core set on HF and again on `v1.0.0`, or only once (long-runs open decision 1)?
3. 15 USD cap for each one-hour run (ask 5).
4. Freeze at Tue 3 Nov 10:00 PT, 18:00Z (ask 6).
5. Mac `release.yml` runner minutes and a Mac smoke slot (ask 7).
6. Gateway credit top-up before the one-hour runs (ask 9).
7. Headerless PEM body: a goalcup miss or a stated loss? This decides the Goose and Kilo C3 results (ask 2).
8. If a canary fails after minute 2: kill the running rows, or let them finish for evidence?
9. Kill gap bound: `check.py` allows under 2 s, X4 allows 5 s under load. An A5 gap of 2 to 5 s costs one extra Codex kill row.
10. Phase 5 harnesses ship in 1.0.0 as "not verified: no full rows", and their tiers can move in 1.0.1.
11. Matcher: the 50 MB/s target holds for the matcher alone, not for one whole stream (14.6 MB/s from the shared rules). Accept?
12. Matcher: keep the lane's own trie, or switch to the `aho-corasick` crate already in `Cargo.lock`?
13. Not blocking the waves: Gemini Shell prompt (ask 10), the mini-full prompt (ask 11), size overrun (ask 12), key rotation after the last run (ask 13), run 1's prompt for A1 (ask 14), rung 5 L3 (ask 15). Ask 8 (build reuse) now affects only the `v1.0.0` build in long-runs Task 13 Step 1.
