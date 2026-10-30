# Fix list for PRD-08-verifier-checks.md (group 1)

From the alignment reviews L1 to L7, triaged against OWNER-DECISIONS.md (the ledger wins).
Files are under docs/plans/2026-10-30-v1.0/. Line numbers are from branch smart/intelligent-newton-0avrpi.
An item with several files carries the same id and the same change text in each file's list; make only the part for this file.

Counts: 0 blocker, 1 major, 0 minor.

## L2-04 (major)

- Section and lines: PRD-02 section 5 step 3 (255-257), section 6 entry totals (281-283), builder note (849-851), sections 9.2 and 9.4; PRD-01 section 9 (391-392); PRD-08 `counts` row (173)
- Change: PRD-02 section 5 step 4: 'At each durable point, also append the counters at that LINK with its `seq` to `counts.jsonl`.' Sections 9.2 step 4 and 9.4 step 2: 'The recovery frame holds, before its LINK, a COUNT event with `boot` = the dead boot, taken from the last `counts.jsonl` line whose `seq` is at or below the recovered LINK.' PRD-02 section 6: `status.json` `written` shows counts at the durable point, so it never goes down. PRD-01:391: 'each boot's last COUNT' becomes 'each boot's last COUNT, which for a dead boot is the COUNT in its recovery frame', and state that this COUNT recounts records up to that line's LINK (invariant 7). PRD-08:173 `counts` row: add 'for a dead boot, the COUNT in its recovery frame (PRD-01 section 9)'.
- Ledger item or rule: R6 (locked: counts are written counts)
- Source: align-L2 L2-04
- Also in: PRD-02-recorder-core, PRD-01-record-format
