# Goal Cup 1.0 build: status report

As of Sunday 1 November 2026, 19:03 UTC. Deadline Friday 6 November 2026. Proposed freeze Tuesday 3 November 18:00 UTC (about 47 hours away). Event switch 6 November 00:00 UTC (about 101 hours away).

Sources: per-PRD findings gathered by reader agents, every pad `REPORT.txt` under `/root/gc-night/`, the latest review (`/root/gc-night/reviews/LATEST.md`, 18:54Z), and the conductor ledger (`/root/gc-night/lanes/integrator.log`, last line 18:56Z). No test was rerun for this report. No key, token or planted value appears below.

## 1. Summary

- The build is code complete on most PRDs and sits on integration head `a31a45e4`. Gate fast passed (419 s) and CI is green on it. Gate full passed (about 577 s) at `dacbc61b`.
- The old code is gone (763 paths deleted in `d7759fdc`), and the deletion gate held.
- What works: Claude Code and Codex record end to end. Direct and dodger rows pass all six checks on both. That covers proof, subagents, secrets, server copy, usage and counts. The Codex kill test passes on live streams, with gaps of 932 ms and 743 ms.
- Hermes records well and passes five of six checks with 5 subagents and 366 requests. It fails the secrets check. A planted test password reached staging raw inside Hermes's own search index.
- Core rows passing all six checks: 4 of 9. Hermes direct fails C3, Hermes dodger has never run, and the gateway rows are 0 of 3.
- The gateway is blocked. The Vercel AI Gateway key is free tier, and every model call through it has returned 403 since 06:49Z.
- Phase 5 has not started. The other 12 harnesses have 0 full rows, and the pad has run no row since 15:02Z.
- Exact-value masking, the owner's chosen fix for secrets, is built but on hold. As built, a large `.env` overflows the first control line (over 64 KiB), and the recorder fails to start on every boot.
- The rig would also read a masking miss as a pass until `check.py` treats a plain hit as a FAIL. Both fixes must merge together.
- Biggest risks: point 3 (recorder start with a large `.env`), point 4 (Hermes and other SQLite copies, plus the silent rig pass), point 2 at scale (no row above 5 subagents), point 7 breadth, and the gateway.
- Code size is about 2.2 times the PRD-00 budget (Rust src 39,908 lines against about 18,100). That needs an owner decision.
- Spend so far is about 25 USD of model cost, with every row under its cap.

## 2. PRD table

| PRD | Status | Done | Open | Blocked by | In flight |
|---|---|---|---|---|---|
| 00 index | partial | Done items 2, 3, 4 met; 1 at 4 of 9; 8 tests only; 9 provisional | Items 1, 5, 6, 7; size overrun; stale record | W5 gateway, masking package, phase 5 dispatch, owner asks | Masking fix and pad C3 package; Hermes parent investigation |
| 01 record format | complete | Codec, writer, recovery, reader, dump, golden; kill and server copy shown | Framing under 2% figure never reported | Pad row reporting framing | Nothing |
| 02 recorder core | nearly | All slices merged, all guards on integration; restart, recover and kill shown | Exit-74 case, by-hand load report, 20-subagent low fd row, size | Exit-74 hook ruling, per-run cap, owner size call | Nothing (masking fix touches its control line) |
| 03 proxy | nearly | Code complete through HTTP/2; direct, dodger and restart shown | HTTP/2 h2-only run, gateway, refusal fallback, long tunnel, load | W5, per-run cap, no lane owns watcher runs | Nothing |
| 04 native capture | nearly | Code complete; native-only usage works; Hermes `state.db` copied | OpenClaw `source_changed` gaps, CPU and fd logging, A1, Hermes `state.db` policy in code | Hermes parent reader, PRD-02 cursor resume | Hermes parent investigation |
| 05 masking | partial | Core masker, leak fix for new key shapes, Gemini key leak fixed | Exact-value masking unmerged with point 3 blocker; Hermes residual; CLI seam | Conductor HOLD, PRD-12 and PRD-10 lanes, owner | Blocker fix lane on `v1.0/prd-05-masking` |
| 06 uploader | nearly | Slices 1 to 7 merged; whole server copy on every row | Network cut, prune under cut, dead-entry upload, sleep, memory rows; PRD-10 text | Pad lane, PRD-10, per-run cap | Nothing |
| 07 Worker and database | nearly | Slices 1 to 5 merged; staging deployed; fetch-run works | Conductor fetch path in rig, size per hour, production release | Rig, A1/A2, owner release | Nothing |
| 08 verifier | nearly | Through slice 10; all invariant, honesty, legs and history checks | Hermes manifest parent reader, Gemini thread tie, H1 rule, exact-value seam, size | Per-run cap, verify lane idle, masking merge | Nothing directly |
| 09 usage | partial | Slices 1 to 5 merged; C5 passes on Claude Code, Codex, Hermes, dodger | `usage-job.sh` unmerged; gateway; A1; Hermes re-source; model-id negative test | Usage and R2 tokens, per-run cap, W5, masking merge | Nothing |
| 10 CLI | nearly | 14 slices merged; end screen, status, start, recover, continue | `launch_values` seam and key-count line; proofs 4 to 11 not run | Masking merge, pad rows, owner Mac | Nothing (seam not started) |
| 11 connectors | partial | 16 connectors, host lists, expected telemetry hosts wired | Phase 5 rows; Hermes leak; held docs request 182615; lane branch carries rejected drop | Pad dispatch, W5, Hermes reader, masking merge | Held request 182615 |
| 12 test pad | partial | Rig, selftest exit 0, 94 pad tests; 4 core rows and Codex kill PASS | Hermes, gateway, phase 5; ledger rules differ from `check.py`; plain-hit semantics | Owner W5 and cap, masking merge | C3 package lane started 18:57Z |
| 13 cuts | nearly | Old code deleted; gate full green after the fact | Provenance comments still hit guards; `check-links.py` delete; release tarball proof | PRD-15 sweep, owner size | Nothing |
| 14 tests and gate | partial | Gate, CI, heavy group, release packaging scripts | `e2e_run.rs` missing; G2, G5 to G7 not run; flake not clean | PRD-02, PRD-15, owner release | Nothing |
| 15 docs | nearly | 19 slices; all core docs on integration; greps clean | Newcomer run; record stale; exact-value and Hermes texts | Phase 5, masking merge, owner | Nothing |
| 16 orchestration and acceptance | partial | Orchestration live; ladder rungs 1 to 9 run; post-cut merged | N2, N3, N5, N6, N8, N9; final files | Owner W5 and cap, phase 6, scribe | Freeze plan; masking and pad lanes |
| 17 lessons | nearly | 8 lessons files; deletion gate green | Real-run lines RR1 to RR6 not recorded; about 30 rows open | Pad rows, owner Mac, size | Nothing |

## 3. Per-PRD notes

**PRD-00 index.** Done items 2, 3 and 4 are met: the Codex kill test, the deletion gate, and gate full plus CI. Item 1 stands at 4 of 9 core rows. Item 5 (15-harness tier table) and item 6 (docs match the build) are open. Item 7 is close, but staging was last deployed from `e626de8f`, not the current head. Item 8 (null mode) is shown by tests only, and item 9 (HTTP/2) is provisional.

**PRD-01 record format.** Done. The only gap is reporting: the framing-under-2% figure has never been quoted from a real Codex run.

**PRD-02 recorder core.** All slices and guards are merged. Restart, recover and the kill test are shown on real runs. Still open: the exit-74 case (needs a test hook), the by-hand load report, and the many-subagent rows. The masking blocker lands in this PRD's control line (`watchdog/mod.rs:45`).

**PRD-03 proxy.** Code complete and heavily reviewed. Direct, dodger and restart are shown. The HTTP/2 h2-only run, refusal fallback, long-tunnel watcher and load rows have never run, and no lane owns them. Gateway capture works, but every model call answers 403.

**PRD-04 native capture.** Code complete. OpenClaw SQLite copies cause 3 to 4 `source_changed` gaps per run. Hermes `state.db` stays copied until a parent reader from other Hermes files exists.

**PRD-05 masking.** Shape masking works on the core pair, and the Gemini key leak is fixed. Exact-value masking (sources 1 to 3 plus the Stripe rules) sits unmerged at `30847ac2`. It must not merge until two things are done. First, values are deduped and cut to 32 before they go on the control line. Second, the PRD-12 rule (plain hit = FAIL) merges with it. Exact-value masking does not cover the Hermes leak: the value is a git remote password in `.git/config`, and the index can store it with its first bytes cut.

**PRD-06 uploader.** Code complete. The server copy is whole on every row. The network cut, prune under cut, dead-entry upload and sleep rows have never run. The pad rig has no cut-worker row.

**PRD-07 Worker.** Done on staging. Production deploy, secret removal and the approve-route cut are owner release steps.

**PRD-08 verifier.** Ten slices merged. The history false alarm is fixed. The main gap is a manifest-based Hermes parent reader, which would let Hermes drop `state.db` without losing the subagents check.

**PRD-09 usage.** Usage is correct on Claude Code, Codex, Hermes and the dodger rows. The organizer job `usage-job.sh` stays unmerged with open findings. It is a known deferral.

**PRD-10 CLI.** 14 slices merged. The next slice, switching three call sites to `launch_values` and showing "N key values were not added to masking." on status and the end screen, has not started. It waits on the PRD-05 merge.

**PRD-11 connectors.** All 16 connector files are in. `tiers.toml` reads "not yet run on the pad" for every harness. The lane branch `v1.0/prd-11-connectors` carries a drop the owner rejected, so it must never be queued. Held request 182615 has text that will be wrong after PRD-05 merges.

**PRD-12 test pad.** The rig works and has produced every pass so far. The ledger and `check.py` have now differed four times. The masked-upstream PASS ruling for Hermes is still only in the ledger. Today a plain hit of the planted value counts as proof the exercise ran, so a masking regression would read as a C3 PASS.

**PRD-13 cuts.** The cut merged with its preconditions unmet. They were completed afterwards. Two items remain: provenance comments still trip the reference guards, and `check-links.py` is not yet deleted.

**PRD-14 tests and gate.** The gate and CI work. `e2e_run.rs` does not exist. Two gate fast failures were rerun green, and the `pipeline_guard` hang bound is a known load flake.

**PRD-15 docs.** The docs are written, but the repo record is stale. `NIGHT-LOG.md` ends at 13:01Z and `MORNING.md` is the 09:59Z interim. None of the three owner answers, full3, full4 or the Hermes finding is recorded there.

**PRD-16 acceptance.** Ladder rungs 1 to 9 have each run on the core pair. A1, A2, A3, A4, the release check, the secrets scan over kept artifacts, cleanup, `PAD-RESULTS.md`, `ACCEPTANCE.md` and the final `MORNING.md` are all not done.

**PRD-17 lessons.** The deletion gate is green. Real-run pass lines RR1 to RR6 are not recorded in `LESSONS.md`, although the full rows likely cover RR1, RR2, RR4 and RR5.

## 4. Tests

C1 = proof, C2 = subagents, C3 = secrets, C4 = server copy equals local copy, C5 = usage, C6 = counts. "Direct" means the harness calls its model host directly. "Dodger" means the model host is pinned in the host list, so it is named but not recorded. Spend figures are pad estimates.

### 4.1 Every run

| Date/time (UTC) | Folder | Harness | Rung / mode | Commit | C1-C6 | Faults | USD | Outcome |
|---|---|---|---|---|---|---|---|---|
| 31 Oct 19:00-19:20 | rung1 | Claude Code, Codex | Rung 1 micro and smoke, direct, server off | 633a4226 | Claude micro INC; smokes FAIL | n/a | 0.69 | Capture worked. 4 rig faults and a placeholder false positive blocked a pass |
| 1 Nov 02:10 | rung2 | Claude Code, Codex | Rung 1 rerun | 8dc99969 | Codex all PASS; Claude smoke PASS; Claude micro C3 FAIL | n/a | 0.55 | Rung 1 passes; the C3 FAIL was a redacted PEM echo |
| 1 Nov 02:10-02:40 | rung2 | Claude Code, Codex | Rung 2 staging | 8dc99969 | no run opened | n/a | 0 | Cloudflare 1010 refusal, then the Worker required `repo_url` |
| 1 Nov 02:54 | rung2b | Claude Code, Codex | Rung 1 recheck, rung 2 micro | 1455cd37 | C1, C3, C6 PASS; C4 FAIL; C2, C5 INC | n/a | 0.35 | Upload complete; C4 failed for lack of database reach |
| 1 Nov 05:2x-05:5x | rungs | Claude Code, Codex | Rungs 3, 4, 6, 8, 9 (10 rows) | 611d7a7d | C1, C3, C6 PASS on most; C4 FAIL | Rung 4 FAIL (rig) | ~2 | Rungs 3, 6, 9 hold locally; rung 4 restart works |
| 1 Nov 05:05-05:55 | rung1x | 12 other harnesses | Rung 1 micro | 0bf25314 | best: opencode2, kilo C1, C3, C5, C6 PASS | n/a | 0.30 | Gemini's real key was found unmasked (since fixed) |
| 1 Nov 06:23 | c4 | Claude Code, Codex | C4 refetch of 12 rows | cee85bdc | C4 FAIL (no local parts); C2, C5 PASS on 8 | n/a | 0 | Every server copy whole and hash-chained |
| 1 Nov 06:29-06:50 | rung7 | Claude Code, Codex | Gateway; rung 4 and 8 reruns | 5825d6d8 | Gateway INC; rung 4 kill@0 PASS; rung 8 C1 PASS | Rung 4 kill@1 FAIL (rig) | ~1 | Every gateway model call 403 (free tier) |
| 1 Nov 07:02-07:13 | rung5 | Claude Code, Codex | Watchdog kill and recover | 6fb5db63 | C1-C5 PASS, C6 INC (L3 variant) | Row as written INC | 0.54 | First real C4 PASS; recover works when the launch shell lives |
| 1 Nov 07:36 | rung3b | Claude Code, Codex | Rung 3 micro staging | f2f96f0e | All six PASS | none | 0.36 | First all-six rows |
| 1 Nov 08:42-09:27 | sweep2 | Gemini and 8 others | Rung 1 resweep; Gemini rung 3 | cafcdef4 | Gemini rung 3b: all PASS except C2 INC | n/a | 0.51 | Gemini key leak fixed, 0 known hits in 3 runs |
| 1 Nov ~10:00 | history-warn | investigation | n/a | 61ef1156 | n/a | n/a | 0 | History warning a false alarm; fixed in 64ff365a |
| 1 Nov ~10:10-10:50 | full1 | Claude Code | Full direct, 22 min | 27341339 | All six PASS | none | 7.33 | 172 requests, 6 threads, 5 subagents, history 132/132 |
| 1 Nov ~10:10-10:50 | full1 | Codex | Full direct, kill@8 and @10 | 27341339 | Run 1 C3 FAIL; run 2 all PASS | FAIL / INC | 3.60 | Kills hit an idle harness, so the kill test was not exercised |
| 1 Nov 11:21-11:44 | full2 | Codex | Full direct, kill@3 and @5 | 78618a77 | All six PASS | PASS | 1.71 | Kills on live streams; gaps 932 and 743 ms; 184 requests, 5 subagents |
| 1 Nov 11:2x | full2 | Gemini | Rung 3 micro | 78618a77 | C3 PASS; rest INC | n/a | 0.05 | Parent reader works; subagent stopped at a shell prompt |
| 1 Nov 11:2x | full2 | Hermes | Rung 3 micro | 78618a77 | n/a | n/a | 0 | Server refused the run (version null); fixed in e707e427 |
| 1 Nov 13:04-13:09 | full3 | Hermes | Rung 3 micro | 2702551e | All PASS except C3 FAIL | n/a | 0.01 | C3 failed by rig rule; 0 known hits |
| 1 Nov 13:11-13:34 | full3 | Hermes | Full direct, kill@8 and @10 | 2702551e | All PASS except C3 FAIL | PASS | 0.68 | 293 requests, 5 subagents, gaps 551 and 69 ms |
| 1 Nov 13:04-13:26 | full3 | Claude Code | Full dodger | 2702551e | All six PASS | n/a | 2.23 | Pinned host named plainly |
| 1 Nov 13:27-13:49 | full3 | Codex | Full dodger | 2702551e | All six PASS | n/a | 1.90 | Pinned host named plainly |
| 1 Nov 14:37-14:42 | full4 | Hermes | Rung 3 micro | 90a07d78 | All PASS except C3 FAIL | n/a | 0.01 | Planted test password raw in Hermes `state.db` on staging |
| 1 Nov 14:38-15:02 | full4 | Hermes | Full direct, kill@8 and @10 | 90a07d78 | All PASS except C3 FAIL | PASS | 0.76 | 366 requests, 5 subagents, gaps 498 and 672 ms |

Superseded results: the rung 1 placeholder C3 FAIL (fixed in 1455cd37), the Gemini key in the record (fixed in fc8df0e9), the C4 FAILs on the `rungs` and `rung7` rows (rung3b and rung5 pass), the full1 Codex kill rows (full2 passes), the Hermes start refusal (fixed in full3), and the history warning (fixed in 64ff365a).

### 4.2 The ladder

| Item | Claude Code | Codex | Others | Status |
|---|---|---|---|---|
| Rung 1, local capture | PASS | PASS | Gemini, opencode2, kilo at micro level | Pass |
| Rung 2, upload to staging | PASS | PASS | Gemini PASS | Pass |
| Rung 3, subagents | All six PASS | All six PASS | Gemini C2 INC; Hermes all but C3 | Pass on core pair |
| Rung 4, recorder crash | PASS at kill@0; kill@1 FAIL (rig) | PASS at kill@0 | n/a | Passes at kill@0 only |
| Rung 5, watchdog kill and recover | Works with L3 variant; C6 INC | Same | n/a | L3 variant not in the rig yet |
| Rung 6, keep going | Holds locally | Holds locally | n/a | Final C4 pending |
| Rung 7, gateway | 403 plus a rig key fault | 403 | n/a | Blocked on owner (W5) |
| Rung 8, dodger | PASS | PASS | n/a | Pass |
| Rung 9, HTTP/2 | Provisional | Provisional | n/a | Needs ALPN, stream ids and bodies confirmed |
| Cycle (parallel) | Never ran | Never ran | n/a | Never ran |
| Full core set | Direct and dodger PASS; gateway blocked | Direct (kill) and dodger PASS; gateway blocked | Hermes direct C3 FAIL; no dodger or gateway | 4 of 9 |
| A1 (Codex, 2 h) | n/a | Never ran | n/a | Needs a cap of about 40 USD |
| A2 (Claude Code, 60 min, 20 subagents) | Never ran | n/a | n/a | Needs a cap of about 50 USD |
| A3 (Hermes via gateway) | n/a | n/a | Never ran | Blocked by W5 |
| A4 (Kilo via gateway) | n/a | n/a | Never ran | Blocked by W5 |

### 4.3 Per harness

| Harness | Best result so far | What blocks a full pass |
|---|---|---|
| Claude Code 2.1.312 | Full direct and dodger PASS; rungs 1 to 3, 4 (kill@0), 5 (L3), 8 | Gateway (403, rig key fault); A2 not run |
| Codex 0.165.0 | Full direct kill row PASS; dodger PASS | Gateway (403); A1 not run; `/quit` exit key (cosmetic) |
| Gemini CLI 0.66.0 | Rung 3b all PASS except C2 INC | Subagent stops at a shell approval prompt despite YOLO; no full row |
| Hermes 0.22.5 | Full direct all PASS except C3; 5 subagents, 366 requests | C3 rig rule; `state.db` leak; no dodger or gateway row |
| opencode | C1, C5, C6 PASS | C3 INC (model refused the check script) |
| opencode2 | Micro pass | No run beyond micro |
| kilo | Micro pass on rerun | Headerless PEM body lines raw on first run; A4 blocked |
| qwen | C1, C5, C6 PASS | C3 INC or FAIL depending on run |
| goose | C1, C5, C6 PASS | Headerless PEM body raw in 3 of 3 runs; owner call |
| crush | None (4 requests) | Stuck at a tool permission dialog |
| cline | None (0 requests) | Sign-in screen; config shape wrong |
| kimi-code | INC | Approval dialog on every run |
| copilot | C1, C5, C6 PASS; C3 INC | Model ran only the safe parts of the check script |
| openclaw | C1, C5, C6 PASS (diagnostic) | C3 FAIL; `source_changed` gaps; works outside the project |
| amp | Never ran | Needs an Amp account |
| droid | Never ran | Needs a Factory account |

### 4.4 Spend (pad estimates)

| Folder | USD |
|---|---|
| rung1 | 0.69 |
| rung2 | 0.55 |
| rung2b | 0.35 |
| rungs | ~2.0 |
| rung1x | 0.30 |
| c4 | 0 |
| rung7 | ~1.0 |
| rung5 | 0.54 |
| rung3b | 0.36 |
| sweep2 | 0.51 |
| full1 | 10.93 |
| full2 | 1.76 |
| full3 | 4.82 |
| full4 | 0.77 |
| **Total** | **about 24.6 (review 5 says about 25.7)** |

Every row stayed under its cap. The ledger has no `spend` line, and box minutes are not measured.

## 5. The seven points

| Point | Verdict | Evidence |
|---|---|---|
| 1. Record every model call | Partly | Claude Code full: history 132/132, 172 requests, server copy equals local. Codex and Hermes C1 PASS. Not shown for the gateway (0 rows), for 12 harnesses at full level, or for long runs (A1). |
| 2. Record every subagent | Partly | 5 subagents with parents on Claude Code, Codex and Hermes. Hermes parents come only from `state.db`. Gemini parent reader merged but not rerun. No row above 5 subagents; A2 needs 15 to 20. |
| 3. Never block the harness | Partly | Kill and restart gaps close under 1 s, and recover works. But the unmerged exact-value change would stop the recorder on every boot with a large `.env`. The by-hand load report and the low fd row were never run. Null mode is shown by tests only. |
| 4. Never record secrets | Partly | 0 known hits on Claude Code and Codex rows; the Gemini key leak is fixed. A planted value went raw to staging inside Hermes `state.db`. Headerless PEM bodies leak on goose and kilo. A key split across streamed deltas is recorded unmasked (accepted risk). Exact-value masking is unmerged. |
| 5. Upload everything | Mostly shown | Server copy whole and equal to local on every core row since rung 5. Network cut, prune, dead-entry upload and sleep rows never ran. |
| 6. Tell the participant plainly | Partly | Pinned host named plainly; off time shown; version-unread said; telemetry hosts not flagged. The masked-key-count line, recover limits, Hermes limit text and the stale record are open. |
| 7. Work with real harnesses | Not shown beyond the core pair | Claude Code and Codex pass. Hermes passes all but C3. Gateway 0 of 3. 12 other harnesses at 0 full rows. Amp and droid need accounts. |

## 6. Critical path to Friday

1. **Now to Monday morning: land exact-value masking as one package.** PRD-05 dedupes and cuts values to 32 before they are sent, with tests at 200 values and 64 KiB. PRD-12 makes a plain hit FAIL and adds the masked-upstream PASS. PRD-10 switches the three call sites and adds the key-count line. Then run one Claude Code direct smoke and one Hermes direct full on the merged head. About 3 to 4 hours of work and about 2 USD.
2. **In parallel: phase 5 rows.** Order: Gemini, Copilot, Qwen, OpenCode, Kilo, then Goose and OpenClaw. About 2 to 3 hours and 10 to 25 USD. The pad has sat idle since 15:02Z, so this is the biggest unused resource.
3. **In parallel: Hermes parent reader.** Read the `delegation-manifest` snapshots already in the full3 and full4 records, at no model cost. If they carry parent ids, PRD-08 builds the reader, and Hermes can drop `state.db`.
4. **Monday 09:00Z: gateway plan B** if W5 is still unanswered. Use Cloudflare AI Gateway with the existing keys, or ship the gateway case as "not verified: plan".
5. **Before the freeze:** run the Hermes dodger row, refresh the record (`NIGHT-LOG.md`, `MORNING.md`, spend line), and decide on size.
6. **Freeze, Tuesday 3 Nov 18:00Z.** Run gate full on the frozen head, then one rerun of the 9 core rows and the Codex kill row.
7. **After the freeze:** A2, A1, A3 (if the cap is raised and W5 is solved). Then the secrets scan over kept artifacts, the production check, cleanup, `PAD-RESULTS.md`, `ACCEPTANCE.md` and the final `MORNING.md`.
8. **By Friday:** owner release steps. That means the macOS run, the production deploy with secret removal and the approve-route cut, the merge to main, and the tag.

## 7. Owner asks pending

1. Hermes `state.db`: confirm the 18:56Z rule (keep until a manifest reader lands, else "subagents not verified"). The fallback is to state the limit plainly at the freeze.
2. Record M1a (sources 1 to 3, the Stripe pair, no source 4) and the Hermes lines in `OWNER-DECISIONS.md`. Agents may not edit it.
3. W5 gateway: choose credits, a free-tier model, bring-your-own-key, or Cloudflare AI Gateway. Open since 09:57Z.
4. Per-run cap for A1 and A2: suggested 40 and 50 USD. The current cap is 10.
5. Deltas-only shape hit with 0 known values: stated loss with the row passing, or rerun. Default today is INCONCLUSIVE.
6. Rotate the Gemini and Claude API keys (W1). Open since 07:24Z.
7. Gemini 0.66 subagent shell prompt: the rig answers it, or ship "not verified".
8. Scope of a planted-value stop: does a planted fake value stop only that harness's rows? (A real secret stops every run.)
9. Size overrun, about 2.2 times the budget: accept or cut.
10. Confirm the `gsk_` and `sgamp_` masking rules.
11. Rung 5 setup L3 (adopt the launch-shell variant into the rig).
12. Goose and kilo headerless PEM body lines: count as a miss or a known limit.
13. Amp and Factory accounts, or ship both as "not verified, no key".
14. Smaller items: a new REPORT_AT, run 1's prompt kept in a file, the cron email on `failed`, question 12, PRD-11 setup switches.
15. Release steps: macOS run, production deploy and secret removal, merge to main, tag.
