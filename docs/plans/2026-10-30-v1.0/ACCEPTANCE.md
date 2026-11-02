# Goalcup 1.0 acceptance report

Written 2 November 2026, about 10:10 UTC, by the reports lane for the conductor. Numbers come from row files, `longsum` output and the ledger only. The conductor adds the last rows and finalises.

Integration head when updated: `7fdaf9a3`. The last two trains (fetch retry and work rule `fabd82ec`; wire links count as a tool turn, Crush init answer, OpenClaw second Enter, Gemini write approval `7fdaf9a3`) passed gate fast. `crates/` is unchanged since H6, so H6 binaries stand. Builds that rows ran on:

| Build | Commit | What it carried |
|---|---|---|
| B0 | `04044320` | masking package, first mini wave |
| H2 | `13ce0d62` | the big train: trie matcher, rig, release rows, participant text, seal wording |
| f7048ed8 | `f7048ed8` | pad trust fixes. The only build with a written trust read |
| H4 | `ad32a2a4` | headerless PEM body lines masked, long-run pad fixes |
| H5 | `a437988d` | task-log natives with no calls are not readable, seal end screen from the pane copy |
| H6 | `284c2610` | subagent links for many harnesses, history matches renamed tool calls |

No row has yet run on the frozen head. The freeze has not happened. Every verdict below is on the build named, and the release check will want the rows redone on the frozen head.

## 1. Verdicts in one table

C1 no missed call, C2 every subagent, C3 no secret, C4 server copy whole, C5 usage stats, C6 counts the participant saw.

| Harness | Release blocker | Verdict | Plain reason |
|---|---|---|---|
| Claude Code | yes | verified, with a note | All six checks pass on direct, gateway and dodger on H5. The one-hour kill and recover run is INCONCLUSIVE, not PASS (section 4). |
| Codex | yes | verified, with a note | All six pass on direct, gateway and dodger on f7048ed8. On H5 only the gateway row ran with work; direct and dodger did no work because the OpenAI key was out of credit. |
| Hermes | yes | not verified yet | C1, C2, C4, C5 and C6 pass, but C3 is INCONCLUSIVE on every Hermes row on current code. The model never touched three secret kinds, and Hermes redacts two kinds itself before the wire. A PASS row does not exist yet. |
| OpenCode | no | verified | Paynest pass 4: PASS C1 to C6, 19 subagents, 19 linked on the wire. Pass 3 was INCONCLUSIVE on the work rule, since fixed. |
| OpenCode 2 | no | verified | Paynest row PASS C1 to C6, 28 subagents, 28 linked. |
| Kilo | no | verified | Paynest row PASS C1 to C6, 18 of 18 linked. |
| Qwen Code | no | verified | Paynest row PASS C1 to C6, 12 of 12 linked, local log read. |
| Goose | no | verified | Paynest row PASS C1 to C6, 10 of 10 linked. |
| Kimi Code | no | verified | Paynest row PASS C1 to C6, 20 of 20 linked, local log read. |
| Gemini CLI | no | not verified: approval screen | Pass 4 stalled on a "Build File Modification, Apply this change?" screen the settings do not skip. 3 requests, no subagents. |
| Crush | no | verified | Paynest pass 4: PASS C1 to C6, 4 subagents, 4 linked. The init answer worked. |
| GitHub Copilot CLI | no | not verified for subagents | 450 requests recorded, no child thread, no reader. Recording works; subagent links do not exist. |
| OpenClaw | no | not verified: no tool use | Pass 4 submitted the prompt, 61 requests of reasoning, no tool call or subagent in 15 minutes. Earlier rows hit `source_changed` gaps. |
| Cline CLI | no | not verified | No row in the sources read. The ledger says it ships not verified (no subagent link). |
| Amp, Factory Droid | no | not verified | No key this cycle (owner decision T2). Built from docs. |

Rows in this table that say "verified" for the non-blockers are on H6 binaries and the Paynest prompt. Those rows are 15 minutes long, not the 20-minute core shape, and the ledger counts them as subagent evidence. The tier file is not written from them yet.

## 2. Release blockers, row by row

All rows below are 20 minutes, `full` mode, staging server, Linux box, keyscan 0 hits. Subagents are those the record shows.

### Claude Code (harness 2.1.312)

| Build | Case | Run | Entry | C1 to C6 | Subagents | Note |
|---|---|---|---|---|---|---|
| f7048ed8 | direct | r64b8d207e47e259e | e51f6ce826eeebedc | all PASS | 5 | stated loss: shape in streamed reply |
| f7048ed8 | dodger | rd25aaa27674537a1 | e16894f969d7ad7a2 | all PASS | 5 | 38 pinned-host gaps, named |
| f7048ed8 | gateway | r8cd26fc1b9c7b000 | e0364793f65aacb3e | C3 FAIL | 5 | 612 known hits: headerless PEM body lines. Fixed in H4 |
| H5 | direct | r463e47c24e590209 | e54514eaccf1be0ba | all PASS | 5 | 97 harness calls, 123 wire requests |
| H5 | dodger | r33389e2d4e577e34 | ebfac5b629384578f | all PASS | 5 | 85 harness calls, 37 pinned gaps |
| H5 | gateway | r9ca64cb17442a1e4 | ecb5738624a65309c | all PASS | 5 | 190 harness calls, 222 wire requests |

Paynest prompt, gateway: 13 subagent sessions in native files on H6, killed by the gateway key budget (HTTP 402), not by the harness. Row INCONCLUSIVE. Pass 3 could not start (402 on the first request).

### Codex (harness 0.165.0)

| Build | Case | Run | Entry | C1 to C6 | Subagents | Note |
|---|---|---|---|---|---|---|
| f7048ed8 | direct | r4ca433028d640aac | ee85ed8779b47484e | all PASS | 5 | 13 rate_limit errors, recorded |
| f7048ed8 | dodger | r338afcf263c88599 | e071a700221107ba9 | all PASS | 5 | 7 pinned gaps |
| f7048ed8 | gateway | r6d3c8f6977130e16 | e5b626f4bd291e52b | all PASS | 6 | |
| H5 | direct | radd23a6f15283466 | efcdddf0e7c0ba6ff | all INCONCLUSIVE | 0 | did no work: OpenAI key out of credit |
| H5 | dodger | r6318628a6857396a | e1354eebbf9b6338f | all INCONCLUSIVE | 0 | same reason |
| H5 | gateway | r2077674a3dc49dd1 | e53e57c7cf8d066a7 | all PASS | 5 | |

Owner direction (6:20Z, 2 Nov): Codex is already proven. The Paynest prompt made Codex spawn 10 and then 11 subagents (12 and 13 threads, 10 with a parent in the first pass).

### Hermes (harness 0.22.5)

| Build | Case | Run | Entry | Result | Subagents | Note |
|---|---|---|---|---|---|---|
| f7048ed8 | direct | r866302881b06fe79 | e03cb7fc5d6741bd1 | INCONCLUSIVE (C3) | 5 | others PASS |
| f7048ed8 | gateway | re6eabc0f32075b81 | e8294debecd25ede2 | INCONCLUSIVE (C3) | 5 | others PASS |
| f7048ed8 | dodger | rcc8d8c1db381b369 | eb0fc001533a24248 | FAIL C1, C5, C6 INC | 6 | verifier called Hermes unreadable; the record was right. Fixed in H5 |
| H5 | direct | r094de2c87f891500 | ea4c14191b6115027 | INCONCLUSIVE (C3) | 5 | others PASS; 15 provider errors recorded |
| H5 | dodger | r033a2f19f50e3adb | e48f84bdb0500ec63 | INCONCLUSIVE (C3) | 5 | C1, C2, C4, C5, C6 PASS; fix works |
| H5 | gateway | rf7540eac013f2308 | e3296158e3acb4a7d | FAIL C1 | 5 | history check: 44 of 368 orphans. Verifier defect, see below |

The H5 gateway C1 FAIL was not a missed call. Hermes renames parallel tool-call ids on resend (43 turns) and splits one multiplexed call (1 turn). The history check matched by id only. Fixed in H6 (`v1.0/prd-09-history-renamed-calls`). The re-verification of the recorded row (section 6) turns that C1 into a PASS. No fresh Hermes row has run on H6 binaries with the 20-minute prompt.

C3 on every Hermes row: PASS for openai_key and github_token as "masked upstream, before the wire" (Hermes redacts them itself), INCONCLUSIVE for google_key, jwt and private_key (the model never printed them, so no mask was exercised). Whether masked-upstream counts toward the release check is open owner ask 4. Default in force: it does not count.

C2 on Hermes carries the plain note: subagents are shown by their task logs, not tied to individual calls (owner, 22:03Z on 1 Nov). Paynest pass 2: 12 subagents, 12 of 12 linked by task logs.

## 3. The seven points

| Point | Evidence | Status |
|---|---|---|
| 1. Record every model call | C1 PASS on Claude Code (6 of 6 rows), Codex (4 of 4 rows that did work) and Hermes (4 of 6 rows; the two C1 results that failed were verifier defects, now fixed). Wire requests per 20-minute row: 102 to 375. The history check found no missed call on any row that did work. | Proven on three blockers, with the Hermes C1 fix proven only by replay |
| 2. Record every subagent | Core rows: 5 or 6 subagents per row on all three blockers. Paynest waves: Codex 10 and 11, Claude Code 13 (native files), Qwen 12 and 15, Kimi Code 20, Kilo 18, OpenCode 14, OpenCode 2 28, Goose 10, Hermes 12. Linked to the parent: 28 of 28, 20 of 20, 18 of 18, 14 of 14, 12 of 12 (twice), 10 of 10. No child thread shown: Copilot, Gemini CLI, Crush, OpenClaw. One-hour runs: 6 to 15 subagents (section 4). | Proven for 10 harnesses (OpenCode 19 of 19 and Crush 4 of 4 added in pass 4). Not proven for Copilot, Gemini CLI, OpenClaw. The owner target of 20 subagents or 10 at once in one hour is not reached in any one-hour run. |
| 3. Never block or slow the harness | One-hour runs killed the recorder twice and the watchdog once. On H4 the recorder gaps were 0.4 to 1.0 s, except one quiet kill of 14.4 s on Claude Code. The harness kept running to its own end in all six runs (seal reason: harness exit). `goalcup recover` worked in all three second-pass runs (ledger 05:57Z). The 2 s Codex bound and the 5 s load bound are not both measured: second-pass X4 is PASS on Hermes and INCONCLUSIVE on Claude Code and Codex. | Recovery shown. The kill-gap bound is shown on one run of three. |
| 4. Never record secrets | C3 PASS or stated-loss PASS on every row that exercised it except two FAILs, both fixed (headerless PEM body lines, section 5). Keyscan of every pad-out tree and every ledger-named folder: 0 hits. The W1 event is listed in section 5. | Proven with stated losses. Not proven for a value split across streamed reply events. |
| 5. Upload everything | C4 PASS (server copy equals local) on every core row that did work and every one-hour run. Upload state `sealed` in all. One-hour compressed size per hour: 2.3 to 2.9 MB (Claude Code), 6.1 to 6.5 MB (Codex), 10.8 to 18.6 MB (Hermes). Staging R2 returned HTTP 429 to the pad's fetch on 6 of 8 Paynest rows in pass 2; the fetch retry merged in `fabd82ec` and held on all 10 rows of pass 3. That was the pad's read, not the upload. | Proven on staging. Production not touched. |
| 6. Tell the participant plainly | Participant text for exact-value masking and the Hermes limit merged in the H2 train, with the seal wording fix. C6 (screen count equals verifier recount) PASS on all six Claude Code core rows, on Codex rows that did work, on all three Hermes H5 rows, and on the second-pass one-hour runs of Claude Code, Codex and Hermes. | Partly proven. The newcomer docs run (PRD-15) has no row in the sources read. |
| 7. Work with the harnesses people use | Harnesses ran as the real TUI binaries in sandbox boxes, with the pad typing the prompt, in direct, gateway and dodger cases where the harness has them. Linux only. 12 of 15 harnesses have rows; Amp, Factory and Cline have none. | Linux only. Mac not run. |

## 4. The one-hour runs (A2, A5, A3)

The composite runs replace PRD-16's separate A2 and A3 lengths, and A5 stands in for the separate Codex kill and exec rows (owner rule 23:12Z). Plan: kills near minutes 10 and 30, watchdog kill and recover near minute 45, cap 15 USD, 15 or more subagents.

A2 is Claude Code direct. A5 is Codex direct. A3 is Hermes through the gateway. Figures are `longsum` output. "Run 1" was the earlier failed Codex run, for comparison.

| Measure | A2 pass 1 (H2) | A2 pass 2 (H4) | A5 pass 1 | A5 pass 2 | A3 pass 1 | A3 pass 2 | Run 1 | A2 pass 3 (H6, Paynest) |
|---|---|---|---|---|---|---|---|---|
| Hours | 1.0 | 1.0 | 1.02 | 1.02 | 1.0 | 1.02 | 0.8 | 1.0 |
| USD | 2.08 | 1.95 | 2.87 | 2.90 | 1.97 | 2.10 | not given | 3.65 |
| Subagents | 6 | 6 | 9 | 6 | 12 | 15 | 42 (PRD-16) | 13 |
| Most at once | 5 | 6 | 3 | 5 | 0 | 0 | not given | 12 |
| Wire requests | 167 | 194 | 291 | 260 | 808 | 898 | not given | 309 |
| Raw MB per hour | 25.3 | 30.6 | 214.9 | 128.6 | 749.2 | 1044.2 | 495.9 | 45.1 |
| Compressed MB per hour | 2.3 | 2.9 | 6.1 | 6.5 | 10.8 | 18.6 | not given | 3.4 |
| Recorder RSS peak MB | 62.9 | 64.5 | 178.7 | 110.9 | 170.6 | 179.3 | not given | not given |
| Input tokens per hour | 48,044 | 40,626 | 329,559 | 283,806 | 1,116,961 | 1,257,886 | 143,755,102 | 132,782 |
| Output tokens per hour | 347,164 | 327,854 | 93,099 | 94,522 | 808,191 | 854,750 | 502,776 | 649,756 |
| X2 (subagents) | INC | INC | INC | INC | INC | INC | | INC |
| X3 (no memory guard loss) | PASS | PASS | PASS | PASS | PASS | PASS | | PASS |
| X4 (kill gaps) | FAIL | FAIL, line INC | FAIL | FAIL, line INC | FAIL | PASS | | FAIL, line INC |
| C1 to C6 | C6 INC, rest PASS | all PASS | C6 INC, rest PASS | all PASS | C3 FAIL, C6 INC | C3 INC, rest PASS | | all PASS |
| Row result | FAIL | INC | FAIL | INC | FAIL | INC | | INC |

What each pass shows:

- Pass 1 ran on H2 binaries (`13ce0d62`) with pad code at `f7048ed8`. Its X4 FAIL was a pad gap-count defect: the restarts were immediate (44 to 145 ms), and A2 idle was not a block (ledger 04:19Z). A3 C3 FAILED with 147 hits: headerless PEM body lines in `.env` files, masked in H4. C6 was INCONCLUSIVE on all three because the end screen did not parse after the new seal wording.
- Pass 2 ran on H4 (`ad32a2a4`). C1 to C6 pass on A2 and A5. A3 C3 is INCONCLUSIVE (three kinds not exercised, one known value split across stream events as a stated loss). Faults: A3 PASS; A2 INCONCLUSIVE (quiet kill of 14.4 s, recovery line not observed); A5 INCONCLUSIVE (recovery line not observed).
- A5 pass 2: OpenAI returned `credit_balance_exhausted` 7 times and `rate_limit_exceeded` 17 times, and spend stopped growing from about minute 25. The run still recorded 287 model requests and sealed.
- Gaps in pass 2, in order: A2 killed 987 ms, killed 14,382 ms, watchdog_lost 115,296 ms. A5 killed 424 ms, killed 768 ms, watchdog_lost 77,991 ms. A3 killed 663 ms, killed 512 ms, watchdog_lost 61,512 ms. I read `watchdog_lost` as the deliberate span between the watchdog kill and the recover command, not harness-visible loss.
- A2 pass 3 ran on H6 binaries (`284c2610`) with the Paynest prompt, cap 8 USD. C1 to C6 all PASS. 13 subagents, 12 at once: the 10-at-once target is met, the 15-subagent target is not (13). Gaps: killed 380 ms, killed 57,329 ms, watchdog_lost 119,788 ms. The 57 s gap is a quiet kill: no request was in flight, and the harness sent none until 57 s later, so it measures harness idle, not recorder downtime (recorder back in 2.4 s). Faults are INCONCLUSIVE because the pad took no status sample on both sides of each kill, a pad measurement gap, not a recorder fault. Keyscan: 0 hits. Evidence only, not a release row.
- No `memory_guard` gap in any run (X3 PASS seven times).
- Recorder memory peak stayed under 180 MB in all six, inside the 260 MiB line of PRD-06 U7a. The framing share and the minute-60 rotation step (X9) are not in the `longsum` output.
- Total spend of the six runs: 13.87 USD, against a cap of 15 USD each.

Not measured by `longsum`, so not stated here as X values: X1, X5, X6, X7, X8, X10 as separate lines. They are covered row by row by C1 (X1), C4 (X5 upload, X6 server copy), C3 (X7), C5 (X8) and C6 (X10), as in the tables above. X5 (upload keeps up within 6 minutes of durable) has no separate measurement in the sources.

A1 (two hours, Codex) and A4 (Kilo through the gateway) have not run. They wait for the release (plan Task 13).

## 5. Masking: the losses stated

- Known value found only in the joined streamed reply (model echo). Shape hits count as a stated loss (owner, 20:32Z on 1 Nov): C3 PASS with a note.
- A known value split across streamed reply events cannot be seen by the masker. Owner answer 2 Nov 02:00Z: record it for future improvement and keep shipping. The pad scores it as a stated loss: C3 PASS with a note. This is the W1 case.
- W1 event (2 Nov 01:57Z): in two B0 rows, the real Gemini key and the real DeepSeek key were recorded raw in streamed replies. The pad's setup script printed the environment, and the model echoed each key split across stream events. The request side was masked. Copies existed in the box records, two local fetched folders (mode 0700) and two staging entries. Owner answer: rotate both keys later, keep testing with them, clean the staging server at the end if needed. The setup script no longer prints the environment (merged in `13ce0d62`).
- Headerless PEM body lines, found on H2 (OpenCode 2, Goose, A3). Fixed in H4 by masking the key-file body lines by exact value. Claude Code gateway C3 went from 612 known hits (f7048ed8) to a PASS (H5).
- A bare git remote password restated in model reasoning (OpenClaw, Goose) is outside the exact-value sources. Default in force: a stated loss. Open owner ask.
- Masker boot cost and value cap: the cap of 32 values was kept by the owner (22:05Z on 1 Nov).

## 6. Subagent records for many harnesses, re-verified

The readers lane built subagent links for Gemini CLI, Qwen, Kimi, Crush and Goose, plus a wire-id table for several others, and a fix for renamed tool-call ids. On 12 real rows, old verifier against new (ledger 07:37Z):

- C2 went from INCONCLUSIVE or FAIL to PASS on Gemini CLI, Qwen, Kimi, OpenCode, OpenCode 2, Crush, Goose and Kilo.
- Gemini CLI C5 went from FAIL to PASS. Hermes history went from fail to pass, so C1 from FAIL to PASS.
- Codex and Claude Code on H5 were identical. Claude Code Paynest had all 37 pairs usage-equal.
- No regression in C1, C2 or C5. History coverage fell on Qwen (33 to 17) and Claude Code Paynest (63 to 41), because named threads now trigger the compaction skip.

This is a replay of recorded rows, not new rows. The new verifier merged as `284c2610`. The Paynest pass 3 rows then ran on it.

## 7. The Paynest subagent waves

Prompt: a payments platform, many subagents. Gateway key budget stopped Claude Code and Codex in passes 2 and 3 (HTTP 402). DeepSeek credit stopped 10 rows in pass 1 (402); the owner topped it up.

| Pass | Build | Rows | What it showed |
|---|---|---|---|
| 1 | H5 | 13 | All INCONCLUSIVE ("did no work"). Codex gateway 10 subagents, Claude Code 1. DeepSeek rows stalled on credit. |
| 2 | H6 | 13 | OpenCode 2 PASS (28 of 28 linked). Hermes 12 of 12. Qwen 15, Claude Code 13, Codex 11 sessions. 5 rows interrupted. |
| 3 | H6 | 10 of 11 | Kilo, Qwen, Goose, Kimi Code PASS C1 to C6. OpenCode 14 of 14 linked, row INC. Copilot, OpenClaw, Crush, Gemini CLI stalled or showed no child. |
| 4 | H6 binaries, pad `7fdaf9a3` | 4 | OpenCode PASS (19 of 19), Crush PASS (4 of 4). Gemini CLI and OpenClaw INC (2.22 USD est, keyscan 0). |

## 8. What is not proven

- No row ran on the frozen head, because the freeze has not happened. The release check wants rows on that exact head.
- No Hermes row is a PASS. C3 is INCONCLUSIVE on all of them.
- No one-hour run is a PASS. Best is INCONCLUSIVE. X2 never reached 15 subagents with 10 at once. Hermes cannot show concurrency from task logs (0 at once is how it reads).
- Kill-gap bound under load (5 s): PASS once (A3 pass 2). A2 pass 2 had one quiet 14.4 s kill gap.
- Only f7048ed8 has a written trust read. H4, H5 and H6 have selftest files with all controls run, but no separate trust note.
- A value split across streamed reply events can still be recorded raw (stated loss).
- Gemini CLI, Copilot (subagents), OpenClaw, Cline, Amp and Factory: not verified.
- macOS: not run. Linux only.
- Production Worker: not touched. All rows are staging.
- The newcomer docs run and the clean-up check (N9) have no result in the sources read.
- A1 and A4 have not run.
- Model spend is the pad's own estimate, which errs high. Billed spend is not known to me.
