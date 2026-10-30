# Fix list for BUILD-PROMPT.md (triage group 2)

Source reviews: align-X1-run1, align-X2-scope, align-X3-builder, align-PRD-00 to align-PRD-06.
The ledger (OWNER-DECISIONS.md) wins. An id shared by several fix lists carries the same change text in each;
make in this file only the part that names this file. Items are ordered by severity, then id.

Counts: 0 blocker, 5 major, 5 minor, 10 in all.

## G2-03 (major)
- Sources: X3-3, P00-4
- Section and lines here: Section 1 item 5 (24-26); section 4 lane table (61-73)
- Same id also in: PRD-00-index
- Ledger item or rule: Ledger B1, B2, B2a (locked, win over both texts); ALIGN known conflict R3-10
- Change:
  Edit before H+0 (the prompt is frozen only during the build).
  BUILD-PROMPT section 4: replace its own lane table with a pointer to PRD-16 section 1's twelve-row table (Judge, Participant and Masking start at H+0 against the stubs; PRD-14's gate change lands first).
  BUILD-PROMPT section 1 item 5: "Follow it. Where this prompt and PRD-16 differ, PRD-16 wins, except a locked ledger item, which wins over both."
  PRD-00 R3-10: replace with "Ledger B1, B2 and B2a win over BUILD-PROMPT.md and PRD-16. Within them, the lane table, start times and roles are PRD-16 section 1's."

## G2-04 (major)
- Sources: X3-4, P00-5, X3-19
- Section and lines here: Section 4 step 5 (90-93)
- Same id also in: PRD-00-index, PRD-16-orchestration-acceptance
- Ledger item or rule: Ledger B2a (locked: replan updates the remaining PRDs and the build plan)
- Change:
  PRD-00 row 339: "Edited before H+0 only. The build plan of ledger B2a is PRD-16 plus PRD-00 'Build order'; during the build the replan updates them, and the remaining PRDs, by appending an Amendments note to each." Row 340: add "PRD-00 and PRD-16 included".
  BUILD-PROMPT step 5: "updates the remaining PRDs and the build plan (PRD-16 and PRD-00 'Build order') by appending an Amendments note to each, never editing this prompt".
  PRD-16 line 119: "runs after the last slice of each PRD merges, and after any merge that changes a seam; runs in the integrator lane; appends Amendments notes (PRD-00 row 339)".

## G2-10 (major)
- Sources: X3-10, X1-1
- Section and lines here: Section 4 step 3, third reviewer (86-88)
- Same id also in: PRD-02-recorder-core, PRD-16-orchestration-acceptance
- Ledger item or rule: Point 2 ('however many'), R5; ambiguous breaker instruction
- Change:
  BUILD-PROMPT lines 87-88: "a third reviewer who tries to break it against run 1's shape with PRD-02's `pipeline_guard.rs`, `pipeline_stall.rs` and `by_hand_load` (40 and then 200 streams, 1.5 MB bodies in bursts, a held writer, WebSockets) and the latest pad smoke row, once per merge request (PRD-16 section 1). There is no run 1 replay tool."
  PRD-02 `pipeline_guard.rs`: "40 streams, then 200 streams, each offer 2 GiB in 1.5 MB bodies". `by_hand_load`: "run twice: 40 clients, then 200 clients sending 1.5 MB POSTs in bursts (all at once every 60 s, as after a rate-limit window), plus 200 WebSockets at 1 message a second each". R8: report `held_peak`, the `memory_guard` gap count and S1 call time for both sizes.
  PRD-16 Breaker: "about 40 and about 200 synthetic subagents". X3: add "The 200-stream `by_hand_load` result is reported next to A1 and A2; its `memory_guard` gaps are a night-log line, not a FAIL".

## G2-38 (major)
- Sources: X2-01, P00-20
- Section and lines here: Section 4 step 3, reviewer B
- Same id also in: PRD-00-index, PRD-13-cuts, PRD-16-orchestration-acceptance
- Ledger item or rule: Ledger L2 (locked: less code, cut before you add); VISION 'Less code'
- Change:
  PRD-00 lines 775-777: replace "The 18,700 is above ... says why in the night log." with "Each row is a ceiling, not an estimate. A builder who would pass its row first cuts what serves no point; past the row by more than 10% needs a replan `decision` line that names the point the extra lines serve. Reviewer B checks each merge request against its row and against the fresh design's row (audit B, B1.11 and B2.12). Targets after the cuts of X2-02, X2-03, X2-07, X2-08, G2-24 and G2-25: Rust src about 18,100, Rust tests about 15,000." Rust row: "about 18,100 / about 15,000". Use one pad number: "about 1,350 to 1,450 (PRD-12's estimate wins)".
  PRD-13 table: the same two numbers.
  BUILD-PROMPT reviewer B: add "and the PRD's size row (PRD-00 Size budget)".
  PRD-16 Replan: add "and posts `wc -l` of the merged PRD against its row".

## X3-11 (major)
- Sources: X3-11
- Section and lines here: Section 5 'Mass testing' (104-106)
- Ledger item or rule: Order of work; acceptance at risk (A1 must finish before REPORT_AT)
- Change:
  Lines 104-105: "the core set (Claude Code, Codex, Hermes) on all three cases; when it is green, A2 then A1 and A3, in parallel with the other 12 harnesses; A4 is Kilo's gateway full row (PRD-16 sections 6 and 7)".

## X2-10 (minor)
- Sources: X2-10
- Section and lines here: Section 8
- Same id also in: PRD-16-orchestration-acceptance
- Ledger item or rule: Ledger L2; one place per fact
- Change:
  PRD-16 section 13: replace the numbered list with "The scribe fills PRD-00 'Morning report template'; that template is the one list." BUILD-PROMPT section 8: replace the list after "on the integration branch" with "following PRD-00 'Morning report template'".

## X3-20 (minor)
- Sources: X3-20
- Section and lines here: Section 7 (123-127)
- Ledger item or rule: Clarity; ledger B8 (stop and leave a note)
- Change:
  "call list (PRD-16 section 11): a push notification at once, and the scribe writes the `call` line to the top of `NIGHT-LOG.md` at once, outside the 30-minute merge".

## X3-23 (minor)
- Sources: X3-23
- Section and lines here: Section 6 (120)
- Ledger item or rule: Ledger T2 (cheapest current OpenAI model must be named in config); ambiguous instruction
- Change:
  "No model names or ids in prose docs, commit messages or code comments. Config files, fixtures and machine-read rows (`scripts/pad/*.toml`, `row.json`, usage JSON) may hold them."

## X3-24 (minor)
- Sources: X3-24
- Section and lines here: Section 4 step 2 (78-80)
- Same id also in: PRD-16-orchestration-acceptance
- Ledger item or rule: Ownership clarity; merge rule 1
- Change:
  BUILD-PROMPT step 2, add: "Never commit `Cargo.lock`; the integrator refreshes it at merge." PRD-16 step 6: add "(or PRD-01's `crates/goalcup-format/Cargo.toml`)" to the dependency entry points.

## X3-29 (minor)
- Sources: X3-29
- Section and lines here: Section 5 'Every real run checks' (107)
- Ledger item or rule: Ruling owner question 7 (Hermes per session); acceptance a watcher cannot run
- Change:
  After "per 10 minutes" add "(Hermes: per session after the seal, PRD-00 done item 1)".
