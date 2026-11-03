# Fix list for PRD-04-native-capture.md (group 1)

From the alignment reviews L1 to L7, triaged against OWNER-DECISIONS.md (the ledger wins).
Files are under docs/plans/2026-10-30-v1.0/. Line numbers are from branch smart/intelligent-newton-0avrpi.
An item with several files carries the same id and the same change text in each file's list; make only the part for this file.

Counts: 0 blocker, 1 major, 3 minor.

## L3-4 (major)

- Section and lines: PRD-04 section 8 step 5 (336-343) and overridable defaults row 'Finish time guard'; PRD-02 section 11 step 4 (537-538)
- Change: PRD-04:336: '5. **Time guard: 180 s** (today's finish budget, `control.rs:708`).' becomes '5. **Time guard: 180 s, only when the seal reason is `deadline`, `server_stop` or `signal`** (today's finish budget, `control.rs:708`). On `harness_exit` and `seal_command`, finish reads every file to its end with no time guard; status shows `sealing: reading session files` meanwhile.' Same change in PRD-02 section 11 step 4 ('within PRD-04's 180 s guard' becomes 'within PRD-04's time guard where it applies') and in PRD-04's overridable defaults row.
- Ledger item or rule: H8 (locked: session files read with no caps); R5
- Source: align-L3 L3-4
- Also in: PRD-02-recorder-core

## L2-09 (minor)

- Section and lines: PRD-04 section 5 (237-242), defaults (510)
- Change: Lines 237-241: '(S1, bytes free under the soft guard). It keeps a reserve of a quarter of the largest `room()` it has seen ... and reads at most `room()` minus that reserve.' becomes '(S1, bytes free under three quarters of the soft guard, PRD-02 section 3) and reads at most `room()`.' Line 510: '`room()` (PRD-02 already leaves a quarter free)'.
- Ledger item or rule: R4, R5
- Source: align-L2 L2-09

## L2-10 (minor)

- Section and lines: PRD-04 section 9 'Same device only' (359-361); PRD-01 note list
- Change: PRD-04:361, add: 'The first time a folder is skipped this way, write one `note` `native_other_device` with the row name and the folder path below the root, so status and the end screen name it.' Add `native_other_device` to PRD-01's note list.
- Ledger item or rule: R4, R1, point 2
- Source: align-L2 L2-10
- Also in: PRD-01-record-format

## L2-18 (minor)

- Section and lines: PRD-04 section 6 step 1 (262-267), defaults (512)
- Change: Line 266: 'the stated guard on upload and storage cost' becomes 'a cadence chosen for upload and storage cost; it never skips the seal copy and never drops a byte'.
- Ledger item or rule: R5 (only memory guards)
- Source: align-L2 L2-18
