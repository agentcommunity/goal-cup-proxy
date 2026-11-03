# Fix list for PRD-16-orchestration-acceptance.md (triage group 2)

Source reviews: align-X1-run1, align-X2-scope, align-X3-builder, align-PRD-00 to align-PRD-06.
The ledger (OWNER-DECISIONS.md) wins. An id shared by several fix lists carries the same change text in each;
make in this file only the part that names this file. Items are ordered by severity, then id.

Counts: 3 blocker, 9 major, 7 minor, 19 in all.

## G2-01 (blocker)
- Sources: X3-1, P05-7
- Section and lines here: Section 3 'Phase 1a' (180-184)
- Same id also in: PRD-02-recorder-core, PRD-05-masking
- Ledger item or rule: Ledger M1 (masking before any byte reaches disk), point 4
- Change:
  PRD-02 builder note step 2: replace "that unlocks phase 1a" with "that unlocks phase 1a only once PRD-05's core-slice masker (option b, left guard, `boundary()`) is merged; the stub masker is for fake-upstream tests only".
  PRD-05 builder note step 1: "Publish the S2 stub, which stars every byte as `fail_closed` (same length, with `boundary()`), so no build ever writes a raw byte. Tests that need clear bytes use a test-only masker under `#[cfg(test)]`."
  PRD-16 section 3 Phase 1a, add: "Precondition: PRD-05's core slice is merged on the commit under test, checked by a `mask_rules` length above zero in the `start` event. No real key enters a pad box before that."

## P00-1 (blocker)
- Sources: P00-1 (and X2 caps table)
- Section and lines here: Section 9 (316-319); Overridable defaults (543-544); review answer CC 2 (615)
- Same id also in: PRD-00-index, PRD-12-test-pad
- Ledger item or rule: Ledger T6 (locked: 10 USD per run, cost otherwise not a constraint), B3
- Change:
  PRD-00 Spend row: replace the whole-build stop line with "No whole-build cap (ledger T6). The scribe reports running spend in the night log at 200 and 400 USD by the pad's ledger estimate and keeps going." Owner question 8: delete the build stop line sentence. "2 USD per `smoke` run" becomes "an estimate, not a cap".
  PRD-16 section 9 and defaults, PRD-12 lines 328-329 and 699: delete the whole-build model and sandbox rows; keep the 10 USD per-run cap (T6); `smoke` 2 USD is an estimate, not a cap.

## X3-2 (blocker)
- Sources: X3-2
- Section and lines here: Section 2 (130-160), section 5 phase 3 workspace edit (219-223)
- Same id also in: PRD-14-tests-gate
- Ledger item or rule: Ledger B2 (at least two independent reviewers read each diff, locked)
- Change:
  PRD-16 section 2, after "Every step is a commit": "Every step that changes a file outside `docs/plans/` (the gate change, the workspace edit, each lock refresh, the phase 3 edit) is a merge request on its own `v1.0/prd-16-<slug>` or `v1.0/prd-14-gate` branch, read by reviewers A and B before it lands. Log-only commits (`NIGHT-LOG.md`, `MORNING.md`) are exempt."
  PRD-14 line 513: "by the integrator" becomes "by PRD-14's lane, first in its queue, reviewed by reviewers A and B and merged by the integrator".

## 06-4 (major)
- Sources: 06-4
- Section and lines here: X5
- Same id also in: PRD-06-uploader-compression
- Ledger item or rule: Acceptance not provable
- Change:
  U2 and PRD-16 X5: "At every minute t of the watcher's log, `entries.upload_offset` is at or past the `durable_raw` the log shows at t minus 6 minutes, except while `upload.json` shows a network or server error. No byte-size line."

## G2-04 (major)
- Sources: X3-4, P00-5, X3-19
- Section and lines here: Roles 'Replan' (119); Files it owns (459-460)
- Same id also in: PRD-00-index, BUILD-PROMPT
- Ledger item or rule: Ledger B2a (locked: replan updates the remaining PRDs and the build plan)
- Change:
  PRD-00 row 339: "Edited before H+0 only. The build plan of ledger B2a is PRD-16 plus PRD-00 'Build order'; during the build the replan updates them, and the remaining PRDs, by appending an Amendments note to each." Row 340: add "PRD-00 and PRD-16 included".
  BUILD-PROMPT step 5: "updates the remaining PRDs and the build plan (PRD-16 and PRD-00 'Build order') by appending an Amendments note to each, never editing this prompt".
  PRD-16 line 119: "runs after the last slice of each PRD merges, and after any merge that changes a seam; runs in the integrator lane; appends Amendments notes (PRD-00 row 339)".

## G2-05 (major)
- Sources: X3-5, P01-18
- Section and lines here: Section 2 step 5 (141-148)
- Same id also in: PRD-00-index, PRD-01-record-format
- Ledger item or rule: PRD-00 merge rule (586-597) deadlocks otherwise; file ownership
- Change:
  PRD-16 section 2 step 5, replace the `members` bullet (PRD-00 phase 0 step 3 the same): "Add each new crate to `members` in the merge that creates it (PRD-01 adds `goalcup-format`, PRD-02's scaffold adds `goalcup`, PRD-08 adds `goalcup-verify`); the integrator applies that one `Cargo.toml` line inside the merge. The five old crates move to `exclude` in this commit."
  PRD-01 Must not touch, add: "the root `Cargo.toml` and `Cargo.lock` (the integrator adds `crates/goalcup-format` to `members` inside this PRD's first merge, PRD-16 section 2 step 5)".

## G2-10 (major)
- Sources: X3-10, X1-1
- Section and lines here: Roles 'Breaker' (118); X3 (274-275)
- Same id also in: BUILD-PROMPT, PRD-02-recorder-core
- Ledger item or rule: Point 2 ('however many'), R5; ambiguous breaker instruction
- Change:
  BUILD-PROMPT lines 87-88: "a third reviewer who tries to break it against run 1's shape with PRD-02's `pipeline_guard.rs`, `pipeline_stall.rs` and `by_hand_load` (40 and then 200 streams, 1.5 MB bodies in bursts, a held writer, WebSockets) and the latest pad smoke row, once per merge request (PRD-16 section 1). There is no run 1 replay tool."
  PRD-02 `pipeline_guard.rs`: "40 streams, then 200 streams, each offer 2 GiB in 1.5 MB bodies". `by_hand_load`: "run twice: 40 clients, then 200 clients sending 1.5 MB POSTs in bursts (all at once every 60 s, as after a rate-limit window), plus 200 WebSockets at 1 message a second each". R8: report `held_peak`, the `memory_guard` gap count and S1 call time for both sizes.
  PRD-16 Breaker: "about 40 and about 200 synthetic subagents". X3: add "The 200-stream `by_hand_load` result is reported next to A1 and A2; its `memory_guard` gaps are a night-log line, not a FAIL".

## G2-38 (major)
- Sources: X2-01, P00-20
- Section and lines here: Roles 'Replan' (section 1)
- Same id also in: PRD-00-index, PRD-13-cuts, BUILD-PROMPT
- Ledger item or rule: Ledger L2 (locked: less code, cut before you add); VISION 'Less code'
- Change:
  PRD-00 lines 775-777: replace "The 18,700 is above ... says why in the night log." with "Each row is a ceiling, not an estimate. A builder who would pass its row first cuts what serves no point; past the row by more than 10% needs a replan `decision` line that names the point the extra lines serve. Reviewer B checks each merge request against its row and against the fresh design's row (audit B, B1.11 and B2.12). Targets after the cuts of X2-02, X2-03, X2-07, X2-08, G2-24 and G2-25: Rust src about 18,100, Rust tests about 15,000." Rust row: "about 18,100 / about 15,000". Use one pad number: "about 1,350 to 1,450 (PRD-12's estimate wins)".
  PRD-13 table: the same two numbers.
  BUILD-PROMPT reviewer B: add "and the PRD's size row (PRD-00 Size budget)".
  PRD-16 Replan: add "and posts `wc -l` of the merged PRD against its row".

## P00-8 (major)
- Sources: P00-8
- Section and lines here: Section 5 'Cut-off H+10' (226); risk S7 (338)
- Same id also in: PRD-00-index
- Ledger item or rule: Ledger L3 (locked: delete the old crates in the same build), L4
- Change:
  PRD-00 phase 3: replace "If PRD-17's gate is not green by H+10 there is no cut ..." with "The cut runs as soon as PRD-17's gate is green, at any time before REPORT_AT. Until then old crates stay on disk, out of the workspace, and phases 4 to 6 still run. If the gate is never green, MORNING.md names L3 as not met and why." Row 337: "by H+10" becomes "once PRD-17's gate passes, before REPORT_AT". PRD-16 line 226 and S7 the same.

## X3-12 (major)
- Sources: X3-12
- Section and lines here: Section 1 Lanes (91); Roles 'Watcher' (120)
- Ledger item or rule: Ledger B2 (several workflows per lane); ambiguous instruction
- Change:
  Line 120: "at most 6 at once, run as up to three watcher workflows of 2 agents each, started by the conductor beside the real-runs lane".

## X3-13 (major)
- Sources: X3-13
- Section and lines here: Section 6 opening (232-235); N1 (497), N5 (501); section 14 step 5 (415-418)
- Ledger item or rule: PRD-00 done item 1 ('on one commit')
- Change:
  Section 6, add after the table: "After the last phase 5 merge that touches `crates/`, `connectors/` (except `tiers.toml`), `server/worker/src/` or `Cargo.lock`, the core set rows and the Codex kill row rerun on that head before `REPORT_AT`. The release check ignores `connectors/tiers.toml`."

## X3-15 (major)
- Sources: X3-15
- Section and lines here: Section 2 step 7 (157-158)
- Ledger item or rule: Order of work missing (offline gate after lock changes)
- Change:
  Step 7: "rebuild the build snapshot at once after step 6's lock regeneration and after every later lock refresh, and at each phase exit".

## G2-12 (minor)
- Sources: X3-17, P00-14, P05-9, P02-16, 06-12
- Section and lines here: Lines 29, 52, 265, 318, 526, 595, 615, 623, 645
- Same id also in: PRD-05-masking, PRD-02-recorder-core, PRD-00-index, PRD-06-uploader-compression
- Ledger item or rule: Clarity; ledger is the source of truth
- Change:
  Replace revision 2 owner question numbers with PRD-00's current eight. Map: old 1, 2, 4, 6, 7 are question 1; old 8, 9, 10, 11, 13 are question 2; old 12 is 3; old 3, 14, 15 are 4; old 5 is 5; old 16 is 6; old 19 is 7; old 17 and 18 are 8. Write "owner question N (old M)" where a reader may meet the old number.
  PRD-00 line 681: "owner question 2". PRD-00 review answers: write each as "old N (now M)".
  PRD-02: "owner question 2" (line 37); line 120: "This is PRD-00's S1 block (revision 3, rulings R3-1 and R3-2)"; header: "Index: PRD-00 revision 3".
  PRD-06: every "owner question 6" and "owner question 2" becomes "PRD-00 owner question 1"; in "Owner decisions it carries out" tag each line with its ledger id: F2 (zstd), U1 (prune, small chunks, capped backoff that resets on progress), R5 (no caps), R6 and R7 (written counts, `failed`), L5 (one process), B4 (staging).

## X2-10 (minor)
- Sources: X2-10
- Section and lines here: Section 13 (379-406)
- Same id also in: BUILD-PROMPT
- Ledger item or rule: Ledger L2; one place per fact
- Change:
  PRD-16 section 13: replace the numbered list with "The scribe fills PRD-00 'Morning report template'; that template is the one list." BUILD-PROMPT section 8: replace the list after "on the integration branch" with "following PRD-00 'Morning report template'".

## X2-11 (minor)
- Sources: X2-11
- Section and lines here: 'Review answers' section and fully ruled 'Asks' sections
- Same id also in: PRD-00-index, PRD-01-record-format, PRD-02-recorder-core, PRD-03-proxy, PRD-04-native-capture, PRD-05-masking, PRD-06-uploader-compression, PRD-07-worker-server, PRD-08-verifier-checks, PRD-09-usage-research-stats, PRD-10-cli-participant, PRD-11-connectors-harnesses, PRD-12-test-pad, PRD-13-cuts, PRD-14-tests-gate, PRD-15-docs-decisions, PRD-17-lessons-old-code
- Ledger item or rule: Ledger L2; point 6 (plain, true text). Apply last, after every other fix, so line numbers in the fix lists stay valid
- Change:
  Move every "Review answers" section and every fully ruled "Asks" section into one file `docs/plans/2026-10-30-v1.0/REVIEW-HISTORY.md`, verbatim, one heading per PRD. Before moving, copy any ruling still binding into the PRD body. Each PRD keeps one line at its end: "Review history: REVIEW-HISTORY.md, section PRD-NN." The replan keeps new asks in the Amendments note only.

## X3-16 (minor)
- Sources: X3-16
- Section and lines here: Section 3 (177); Interfaces PRD-17 row (478)
- Ledger item or rule: Ruling R3-8
- Change:
  Change both "by H+1.5" to "by H+3 (R3-8)".

## X3-24 (minor)
- Sources: X3-24
- Section and lines here: Section 2 step 6 (152-156)
- Same id also in: BUILD-PROMPT
- Ledger item or rule: Ownership clarity; merge rule 1
- Change:
  BUILD-PROMPT step 2, add: "Never commit `Cargo.lock`; the integrator refreshes it at merge." PRD-16 step 6: add "(or PRD-01's `crates/goalcup-format/Cargo.toml`)" to the dependency entry points.

## X3-25 (minor)
- Sources: X3-25
- Section and lines here: Section 2 step 3 (137-138)
- Ledger item or rule: Ambiguous instruction (merge rule 3)
- Change:
  Add: "This docs-only merge and step 4 are outside merge rule 3; from step 5 on every merge passes `gate fast`."

## X3-26 (minor)
- Sources: X3-26
- Section and lines here: Lanes (97)
- Ledger item or rule: Schedule clarity; ledger B2 (two reviewers, breaker optional)
- Change:
  Add: "A stub-only diff (phase 0 seams) gets reviewers A and B and no breaker, so the Format and core lane can ship PRD-01 types by H+1 and the PRD-02 scaffold by H+1.5."
