# Goal Cup 1.0: plan-wide stocktake

Written 2026-11-01, against `v1.0/integration` at `0bf25314`. Deadline: Friday 6 November 2026.
Sources: the per-PRD readings, PRD-16 (sections 6, 7, 13 and the N proofs), PRD-12 section 1a (the
ladder), the night log and the conductor's working folder.

## 1. Where the build stands

Most of the code is written and merged. The new recorder, proxy, native capture, masking, uploader,
Worker, verifier, connectors, CLI and rig all sit on integration, and the fast gate passes on every
merge. Real runs are thin. Only rung 1 (local capture) and rung 2 (upload to staging) have run, and only
for Claude Code and Codex. Both record, seal and upload with no failures and no key in any record. No
row has ever compared the server copy with the local copy, because the read-only database role cannot
read three tables. The last core piece (PRD-02 slice C: watchdog restart, `goalcup recover`, the server
word) is reviewed and green on its branch but not merged. Its "merge conflict" is in one plan document
only, not in code. Until it lands, `goalcup recover` and `goalcup continue` are not built, the
deletion gate cannot run, and the old crates cannot be cut. So the build is not blocked by hard
engineering. It is blocked by one docs merge, one database grant, a few keys, and then a lot of pad runs
that have not started.

## 2. PRD table

| PRD | Status | What is left | Blocked by |
|---|---|---|---|
| 00 Index | partial | Done items 1 to 4 not shown; 5 to 9 partial; MORNING.md and ACCEPTANCE.md not written | Slice C merge, DB grant, pad runs, the cut |
| 01 Format | nearly | Kept-parts decode recount, server copy byte check, kill test across three boots, framing share under 2% | DB grant; kill run; a Codex subagent run |
| 02 Recorder core | partial | Merge slice C part 1 (recover, Q6, restart test, size cuts). Then slice C part 2: pipeline_stall, many_streams, native_room, panic, no_latch, by_hand_load. Mask pool only if R8 says so | Docs-only conflict in the PRD-02 plan file; then pad kill cases |
| 03 Proxy | nearly | No code left. Pad acceptance 3 to 10 (h2 parallel, gateway, dodger, refusal, kill, long tunnel, load) | Pad runs; gateway key; slice C for the kill case |
| 04 Native capture | nearly | `SnapshotCursor.fid` (joint with PRD-02 `cursors.rs`). Pad acceptance 1 to 7, including Hermes `state.db` | PRD-02 file owner; Hermes key; pad runs |
| 05 Masking | nearly | Export `glued` to the verifier. Question 3 slice only if the owner says yes. Pad A1 to A5 | Owner answer on question 3; pad rows |
| 06 Uploader | nearly | Pad U1 to U8 and the sleep check. Fix the StillRecording and NotClosed wording with PRD-10 | DB grant (fetch-run); slice C for kill and dead-entry runs |
| 07 Worker | nearly | Real-run proofs 1 to 4. Production deploy and secret removal are the owner's | DB grant; slice C; A1 and A2 |
| 08 Verifier | nearly | Two nits (timeline off_ms, report clones). R1 to R7 on real rows; `server_equals_local` never run on a real copy | DB grant; Hermes and dodger rows; slice C |
| 09 Usage | partial | Merge `usage-job.sh` (branch, 5 commits). Six real-run proofs | DB grant (needs a fetched copy); usage bucket and token |
| 10 CLI | partial | Build `goalcup continue` and `goalcup recover` (slice 11) with their tests; HF4 second-start test; wording fixes; seal close-first stub | Slice C merge |
| 11 Connectors | nearly | Pad rows for Hermes, gateway, hints, dodgers, daemons and the other 12; tiers.toml still all "not verified" | Keys; pad runs |
| 12 Pad rig | partial | P1 first-output read; rungs 3 to 9; cycle and full rows; PAD-RESULTS.md | DB grant for C4; keys; slice C for rung 5 |
| 13 Cuts | not started | Delete five old crates, old scripts, spikes, old connector files; guards; size recount; two-binary build | PRD-17 deletion gate, which waits on slice C and PRD-10 slice 11 |
| 14 Tests and gate | partial | `e2e_run.rs`; heavy group entries; sabotage runs (G2); CI on the phase 3 push; check-links delete | Slice C; PRD-10; PRD-15's last link run |
| 15 Docs | nearly | Continue and recover pages; move HARNESS-SETUP.md to archive; tier column; four nits; one final check run; newcomer run | Slice C and PRD-10; pad results |
| 16 Orchestration | partial | Phases 2 to 6, A1 to A4, N1 to N10, MORNING.md, ACCEPTANCE.md | Everything above |
| 17 Lessons | nearly | Four guard names (watchdog_restart, pipeline_stall, cli_continue, second_start test); real-run rows RR1 to RR6; the deletion gate | Slice C; PRD-10 slice 11; pad runs |

## 3. The test ladder (PRD-12 section 1a)

| Rung | What it proves | State | What stops it |
|---|---|---|---|
| 1 Local capture | Recorder, proxy, masking, verifier on a live harness | PASS, Claude Code and Codex (8dc99969) | Hermes never run (no key confirmed) |
| 2 Upload | Open, chunks, heartbeat, seal on staging | Partly: uploads complete for both (3 chunks, 0 failures). C4 FAIL, C2 and C5 INC | DB grant (fetch-run) |
| 3 Subagents | Subagent threads on the wire and native | Never run on staging (smoke ran only at rung 1) | Nothing but scheduling; C4 stays INC until the grant |
| 4 Recorder crash | Watchdog restart, one closed gap | Never run | Restart loop is merged, so it can run now; the restart test lands with slice C |
| 5 Watchdog kill and recover | `goalcup recover` after the watchdog dies | Never run | Slice C merge and PRD-10 slice 11 |
| 6 Keep going | A participant nudge mid-run | Never run | Scheduling only |
| 7 Gateway | Vercel AI Gateway through the same proxy | Never run | Gateway key confirmed in the box key files |
| 8 Dodger | Pinned or proxy-ignoring harness is named, not hidden | Never run | Scheduling only |
| 9 HTTP/2 | Four parallel h2 streams | Never run | Scheduling only |
| Cycle | Every harness and case, 8 minutes | Never run | Rungs above; keys |
| Full | Tier rows, the core set (N1) | Never run | Cycle; the cut (post-cut core set) |
| A1 to A4 | Long runs and load | Never run | Core set green; slice C for A2 |

## 4. Critical path to "ready by Friday"

Ordered. Sizes are rough. "Lane" means the PRD's lane agent.

1. **Merge PRD-02 slice C part 1.** The conflict is only in `docs/plans/2026-10-30-v1.0/PRD-02-recorder-core.md` (two Amendments appends). Keep both, rerun gate fast, merge. Core lane or conductor. About 20 minutes.
2. **Owner grants the database role** (one SQL line, section 7). Then rerun fetch-run on the two rung 2b runs and check C4. Owner 5 minutes; conductor 30 minutes.
3. **Climb rungs 3, 4, 6, 8, 9 now, in parallel, on Claude Code and Codex.** They need no new code. C4 shows INC until step 2 lands, then rerun fetch and check. Pad lane and conductor. About 2 hours with fixes.
4. **PRD-10 slice 11: `continue` and `recover`,** with cli_continue, cli_recover and the HF4 test, plus the StillRecording and NotClosed wording. Participant lane. About half a day.
5. **PRD-02 slice C part 2:** pipeline_stall and the other named tests, by_hand_load. Core lane. About half a day, beside step 4.
6. **Rung 5** (watchdog kill and recover) once steps 1 and 4 are merged. Pad lane. About 1 hour.
7. **Keys land; rung 7 (gateway) and Hermes rows** at rungs 1 to 3. Owner, then pad lane. About 2 hours.
8. **PRD-17 deletion gate, then the PRD-13 cut** in one reviewed commit: old crates, scripts, spikes, old connector files; two binaries; Cargo.lock; CI edits. Cuts lane plus a gate full run. About 1 day including review.
9. **Cycle rows for every harness with a key,** then the **core set: 9 `full` rows on the post-cut commit** (N1), plus the Codex kill row (done item 2). Pad lane, many boxes in parallel. About half a day with fix loops.
10. **A2, then A1 and A3 beside it; A4 as Kilo's phase 5 row.** Conductor. About 3 hours of wall time.
11. **Phase 5: the other 12 harnesses,** one row each or "not verified: no key". Pad lane. Runs beside step 10.
12. **N4 to N10 checks, final docs pass, tiers.toml and README from PAD-RESULTS.md, MORNING.md and ACCEPTANCE.md.** Scribe, docs lane. About half a day.
13. **Owner reads MORNING.md and releases** (production Worker deploy, secret removal, tag). Owner.

Rough budget: steps 1 to 7 by Monday night, step 8 Tuesday, steps 9 to 11 Wednesday, step 12
Thursday, Friday as slack for the owner's release. There is no slack for a second cut or a second full
core-set wave.

## 5. Barriers and the way through each

1. **Slice C is not merged.** It looks like a code conflict but is a docs append conflict in the PRD-02 plan file. Way through: resolve by keeping both Amendments blocks, rerun gate fast, merge today. Then everything parked on it (PRD-10 slice 11, PRD-15 pages, PRD-17 guard names, the deletion gate) can move.
2. **No server copy can be checked.** The role `goalcup_trace_reader` has SELECT on `entries` and `chunks` only. Way through: the owner runs `GRANT SELECT ON goal_cup_tracer.runs, goal_cup_tracer.events, goal_cup_tracer.heartbeats TO goalcup_trace_reader;`. The approved short-lived fetch box already reaches the database, so no other route is needed. Until then, keep running rungs and mark C4 INC, and refetch every staging run afterwards in one batch.
3. **The cut has not started and sits late on the path.** Way through: prepare the cut branch now (deletions, Cargo, CI edits) so it only needs the deletion gate to turn green; run the core set once, after the cut, instead of before and after.
4. **Keys for Hermes and the gateway are unconfirmed, and the Claude key fragment incident is open.** Way through: the owner confirms the DeepSeek and Vercel gateway key files and decides on Claude key rotation; until then the pad runs Claude Code and Codex rungs only.
5. **Pad throughput.** Only 4 real rows have run in a day. Way through: run rungs 3 to 9 in parallel now (one change each from rung 3), per-key queue as PRD-12 says, and let fix loops run beside, not before, the next rung.

## 6. What can be cut or deferred without breaking the seven points

- **Mask pool** (PRD-02 section 4): build only if by_hand_load shows the mask stage is the bottleneck. One worker is correct, only slower.
- **Question 3 exact-value masking slice** (PRD-05): stays out under the default "no". Shape rules already keep known keys out (point 4); the loss is stated.
- **`usage-job.sh` and research stats** (PRD-09 slice 6): research output, not one of the seven points. Defer to after release if time runs short.
- **A4 (Kilo through the gateway)**: already not a release blocker; N3 is shown by the core gateway rows and A3.
- **Harnesses with no key** (Amp, Factory Droid, OpenRouter cases): ship "not verified: no key" by the PRD's own rule. OpenCode and OpenClaw are not release blocking.
- **macOS proof** (PRD-10 acceptance 9, PRD-12 P9): the owner's, after the build.
- **Review nits** across PRD-05, 08, 12, 15, 17 and the `SnapshotCursor.fid` same-size, same-mtime case: list in MORNING.md as known limits.
- **Size overruns** (PRD-02, 03, 06, 08, 10, 12, lessons tests): record in the report; do not spend build time on line cuts other than the PRD-13 cut itself, which is required.
- **Full-tier sabotage runs (PRD-14 G2)** can shrink to one sabotage per gate tier if time runs short.

Not cuttable: `recover` (point 3 under a dead watchdog), the server copy check (point 5), the core set
(points 1, 2, 7), the kill test (point 3), the N6 secret scan (point 4), the cut of the old crates
(done item 4).

## 7. Owner asks

1. Run the GRANT above for `goalcup_trace_reader` on `runs`, `events` and `heartbeats`.
2. Decide on rotating the Claude key after the fragment incident.
3. Confirm the DeepSeek key file for Hermes and the Vercel gateway key for the gateway rows.
4. Confirm the Vercel plan allows a 2.5-hour box for A1 (Pro, not Hobby).
5. Put run 1's prompt in a file for A1, or accept "compared by load" (question 6).
6. Answer question 3 (exact-value masking), or accept the default "no".
7. Confirm the 10 USD per-run cap, and accept a higher cap for an A2 rerun if X2 comes back INCONCLUSIVE.
8. Accept "not verified: no key" for Amp, Factory Droid and OpenRouter.
9. Confirm the unchanged cron emails on a `failed` heartbeat are wanted before the production deploy.
10. Answer question 12 (test hooks in the release binary) and the 14,200 total test line question, or accept the defaults.
11. Plan the macOS run on your Mac after the build, and the production deploy and secret removal at release.
