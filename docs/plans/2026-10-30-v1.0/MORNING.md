# Goalcup 1.0 morning report

Written Monday 2 November 2026 at about 10:10 UTC for the owner. The conductor finalises it with the last rows. Full evidence is in `ACCEPTANCE.md`. The overnight log is `NIGHT-LOG.md`.

**Not ready: no release blocker set passes on one frozen head yet. Hermes has no PASS row, and no one-hour run is a PASS (best is INCONCLUSIVE).**

Acceptance line: A2, A3 and A5 have each run twice. The first pass failed on all three. The second pass is INCONCLUSIVE on all three, with C1 to C6 mostly PASS. A1 and A4 run after the release.

## Your decisions

Open asks, with the default in force until you answer. Numbers are those of the finishing plan, section 5.

| # | Ask | Default in force |
|---|---|---|
| 4 | Does a Hermes C3 that passes because Hermes redacts the key itself ("masked upstream") count for the release check? | No. Hermes then cannot reach a clean C3 PASS, because three secret kinds are never printed by the model. This is the main thing blocking a Hermes "verified". |
| 5 | Cap for each one-hour run | 15 USD. The six runs cost 13.87 USD together (pad estimate). |
| 6 | Freeze time | Tuesday 3 November, 18:00 UTC. |
| 7 | Mac release run (`release.yml` by hand, metered runner minutes, nothing published) and a slot for your own Mac smoke | Waits for the freeze. |
| 10 | Gemini CLI shell prompt | "Verified with a note" if the keep-going nudge works. In the last wave Gemini CLI stalled on a write confirmation even in unattended mode, so it stays "not verified". |
| 12 | Size overrun: about 39.9k Rust lines against an 18.1k budget (figure from the plan; I did not recount) | No default recorded. |
| 15 | Setup L3: the launching shell must outlive the watchdog for `goalcup recover` to work | Documented. Never answered. |
| - | Split echo: a known key split across streamed reply events is recorded raw. The pad scores it as a stated loss (C3 PASS with a note). This is the conductor's reading of your 2 Nov 02:00Z answer. Please confirm it. | Stated loss, keep shipping. |
| - | A bare git remote password restated by a model in its reasoning (OpenClaw, Goose) | Stated loss. The other choice is a new exact-value source for `.git/config` remote passwords. |
| - | OpenClaw `source_changed` gaps | Stated loss with a note. |
| - | Is the test member id a secret? It sits in 8 committed test fixtures and docs from before this session. | Not treated as a secret. Not added by this work. |
| - | Hermes dodger tier note wording (Hermes is not a dodger; the record is correct) | Wording still to settle. |
| - | Amp and Factory | Ship "not verified", no key. |
| - | Gateway key budget | See "Your steps", step 3. |

Already answered, not reopened: masker cap 32 kept, Hermes C2 on task logs with a plain note, Codex gateway on the pinned cheap model, stated loss for shapes in streamed replies, rotate the Gemini and DeepSeek keys later and keep testing with them now.

## What happened overnight

All times UTC, 2 November. Eight trains merged into `v1.0/integration`, each with one gate.

| Time | Train | Merge | Carried |
|---|---|---|---|
| 01:04 | handover | `45b47073` | tools, ledger, reports, the four finishing plans |
| 02:52 | the big train | `13ce0d62` | trie matcher, mini rig, release rows, longsum, participant text, seal wording, mini-wave fixes |
| 03:50 | pre-freeze | `f7048ed8` | pad trust fixes, fresh-clone compare |
| 04:46 | long-run fixes | `ad32a2a4` | headerless PEM body lines masked, pad kill pairing, busy one-hour runs |
| 05:21 | core-row fixes | `a437988d` | Hermes no longer called unreadable, seal end screen from the pane copy |
| 06:15 | exec retry | `aa36af92` | a lost sandbox reply is read back, never a second kill |
| 07:47 | subagent links | `284c2610` | readers for many harnesses, history matches renamed tool calls |
| 08:53 | wave fixes | `fabd82ec` | fetch retries on 429, work rule, Gemini CLI approvals |
| later | stall fixes | `7fdaf9a3` | wire subagent links count as a tool turn, Crush init answer, OpenClaw second Enter, Gemini write approval |

Gates: 11 gate runs up to `fabd82ec`, 8 passed; the stall-fix train then passed too. The three failures were slips caught by the gate (rustfmt, a rule check that found a harness name in CLI code, clippy). Each was fixed and rerun. A passing gate takes about 7 to 8 minutes. The test count went from 685 (train 5) to 707 (train 8).

Builds: six release builds (B0 `04044320`, H2 `13ce0d62`, `f7048ed8`, H4 `ad32a2a4`, H5 `a437988d`, H6 `284c2610`), each shared by all rows on it.

Waves, with the pad's own cost estimate in USD (it errs high):

| Wave | Build | Rows | Cost | Result |
|---|---|---|---|---|
| Mini wave | B0 | 13 boxes | 0.33 | 2 PASS, 7 FAIL, 4 INC, 2 not verified (no key) |
| Mini reruns | H2 | 7 | 0.22 | 0 PASS, 3 INC, 4 FAIL |
| Core set | `f7048ed8` | 9 | 8.18 | 5 PASS, 2 INC, 2 FAIL (before the PEM fix) |
| Core set again | H5 | 9 | 5.21 | 4 PASS, 4 INC, 1 FAIL (Hermes gateway, a verifier defect) |
| One-hour runs, pass 1 | H2 | 3 | 6.92 | all FAIL |
| One-hour runs, pass 2 | H4 | 3 | 6.95 | all INC, C1 to C6 mostly PASS |
| Paynest subagent wave 1 | H5 | 13 | 1.46 | all "did no work" |
| Paynest wave 2 | H6 | 13 | about 9.9 | 8 rows with data, 5 interrupted |
| Paynest wave 3 | H6 | 10 | 9.85 | 4 PASS (Kilo, Qwen, Goose, Kimi Code) |
| Paynest wave 4 | H6 binaries | 4 | 2.22 | 2 PASS (OpenCode 19 of 19 linked, Crush 4 of 4 linked), Gemini CLI and OpenClaw INC |

Model spend over the night: about 49 USD by the pad's estimate. Row boxes: about 80 boxes and about 1,600 box minutes (my sum of the spend files and the B0 ledger figure). Build and gate boxes, from the conductor's two spend lines: 10 boxes, 21.4 box minutes, 119.7 vCPU minutes (02:56 to 04:05) and 5 boxes, 16.7 box minutes, 122.1 vCPU minutes (04:49 to 05:56). The earlier Vercel infrastructure spend you named at 23:12Z on 1 November was about 100 USD.

Your calls today: 02:00 the key answer (W1: rotate Gemini and DeepSeek later, clean the server at the end), split echo recorded for later, 06:20 focus on recording and subagents over masking, DeepSeek topped up, budget about 20 USD for subagent reruns. The gateway key was still at its 10 USD limit at the last row.

## State of the code

- Integration head is `7fdaf9a3`. Gate fast PASS on both late trains (707 Rust, 30 script, 168 pad tests on the last). `crates/` is unchanged since H6, so the H6 binaries stand.
- The code is ready for the freeze on the guarded tree. Release rows on a frozen head are still owed.
- Not frozen. Guarded code changed in H4, H5 and H6, so every release row has to be redone on the frozen head.
- Production Worker untouched. The staging Worker holds the test rows.
- Only `f7048ed8` has a written pad trust read. The later builds have selftest files that ran all controls.

## What is proven

- Claude Code: all six checks pass on direct, gateway and dodger on H5, with 5 subagents per row.
- Codex: all six pass on all three cases on `f7048ed8`. On H5 the gateway row passes. Direct and dodger did no work because the OpenAI key is out of credit.
- The recorder survives a kill. Two recorder kills and one watchdog kill with `goalcup recover` ran in all six one-hour runs. The harness ran to its own end every time. Recorder gaps were 0.4 to 1.0 s, with one quiet kill of 14.4 s.
- Subagent links on a PASS row: OpenCode 2 28 of 28, Kimi Code 20 of 20, Kilo 18 of 18, OpenCode 19 of 19, Qwen 12 of 12, Goose 10 of 10, Crush 4 of 4. Hermes 12 of 12 by task logs (C3 INCONCLUSIVE only). Codex 10 of 10 linked (H5 wave). Claude Code 13 spawned (the gateway budget stopped the re-read) and linked on the H5 core rows. Not linked: Copilot (no reader, deferred). Cline, Amp, Factory: no rows.
- No secret in any kept record outside the stated losses. Keyscan of all pad output: 0 hits.
- No loss under load: no memory-guard gap in any one-hour run. Recorder memory peaked under 180 MB.
- Server copy equals local on every row that was fetched.

## What is not proven

- Hermes: no PASS row. C3 stays INCONCLUSIVE (ask 4). Its gateway C1 failure was a verifier defect, fixed and shown by replay only.
- Any one-hour run as a PASS. X2 (15 subagents, 10 at once) was not reached. Best at once: 6 for Claude Code, 5 for Codex. Hermes cannot show concurrency from task logs.
- Kill-gap bound under load: PASS once in three runs.
- Not verified: Gemini CLI (stalls on a build-file approval screen the settings do not skip), OpenClaw (no tool use in the 15-minute row), Copilot (subagents), Cline, Amp, Factory.
- macOS, the newcomer docs run and the clean-up check: no result yet.
- A known value split across stream events can still be stored raw.

## Problems that cost time

- Two real keys were recorded raw on the first mini wave (W1). Cause: the pad's setup script printed the environment. Fixed. Rotation is in your steps.
- OpenAI credit ran out during the second one-hour run and again in the H5 core rows.
- DeepSeek credit ran out at 07:20 and stopped 10 rows.
- The gateway key budget (10 USD) was exceeded twice. You raised it at 08:31, but the 10:07 ledger line still shows the key over 10 USD, and the Claude Code and Codex gateway rows did not run in wave 3.
- Staging storage returned HTTP 429 to the pad's fetch in wave 2. The retry merged and held.
- The wave 2 stop did not act in time. The 8 USD rule was passed (about 9.9 USD).
- Queue lesson: the merge-queue driver stops at its hour limit. A liveness check using `pgrep -f` on its own name matched itself and hid that it had stopped, which cost about an hour on train 10. Check the driver by its pid.

## Your steps

Before the release (the conductor does these on your word):

1. Answer the table above or accept the defaults. Especially ask 4, the freeze time and the split-echo reading.
2. Check the OpenAI balance. Without it Codex rows do no work.
3. Check the Vercel AI Gateway key budget. Make sure it really is above 20 USD, and top up DeepSeek if the balance is low. Without this the gateway rows stop at HTTP 402.
4. Say "freeze". The conductor then runs the full gate on the frozen head, one build, and the release rows (core set, one-hour runs).

Release (owner only, the plan's Task 12). The fast path is Tuesday 3 November at 17:00 UTC. The default is Wednesday 4 November at 17:00 UTC. The event opens Friday 6 November at 00:00 UTC.

5. Read this file. Confirm no real run is open on production.
6. Merge `v1.0/integration` at the frozen head into `main` as a merge commit. `main` is `4b226de2` at 0.7.0. The merge brings 1.0.0, so no `release.sh prepare`. If `main` moved under a guarded path, the conductor reruns the release rows on the merge result first.
7. Run `sh scripts/gate.sh full` on `main` in a build box.
8. Run `sh scripts/release.sh tag 1.0.0`. It runs `scripts/pad/table --release-check` first.
9. `release.yml` publishes four targets. Run `install.sh` from the latest release on Linux and on your Mac. Do one pad run on the Mac and note the masker boot time (target under about 2 s).
10. Deploy the production Worker from your own device, from the tagged commit: `cd server/worker && npx wrangler deploy`. Then `npx wrangler secret delete RECEIPT_SIGNING_KEY`.
11. Production smoke: one practice run sealed, fetched, checked.
12. Deferred checks: PRD-14 G2 and PRD-12 P8.
13. Set `EVENT_ID = "goalcup-cup-2026"` between 6 Nov 00:00Z and 7 Nov 00:00Z, in `server/worker/wrangler.toml`, with your deploy.

After the last test run:

14. Rotate the keys. Your 2 Nov 02:00Z answer names the Gemini key and the DeepSeek key. An older ask also named a Claude key. That one was not repeated in the later answer, so say if it still needs rotating.
15. Clean keys and personal information off the staging server, per your W1 answer. The two entries known to hold a raw key are `e9acb5c06a4bf2aae` and `ef100051f05c98367`. The local fetched copies of those two rows sit in locked folders on the conductor box.
16. Check that no boxes or `gc-pad-*` repositories are left (check N9).

## After the release

Plan Task 13, on your word and on staging only. A4 (Kilo through the gateway, 20 minutes). One `full` row per harness whose short composite row passed. A1 (two hours, Codex) only if you want the comparison to run 1. Results go into `ACCEPTANCE.md` and a "Big tests on 1.0.0" block here. Any new failure is a 1.0.1 fix, and tiers change only in 1.0.1.
