# Fix list for PRD-03-proxy.md (group 1)

From the alignment reviews L1 to L7, triaged against OWNER-DECISIONS.md (the ledger wins).
Files are under docs/plans/2026-10-30-v1.0/. Line numbers are from branch smart/intelligent-newton-0avrpi.
An item with several files carries the same id and the same change text in each file's list; make only the part for this file.

Counts: 0 blocker, 1 major, 3 minor.

## L4-5 (major)

- Section and lines: PRD-01 `http_req` meta (232, 353); PRD-05 section 7 step 4 (317-318), what is cut row (516); PRD-03 request side (286)
- Change: Keep today's query drop (less code, no change to a secret protection). PRD-01:232: '`path` (query kept, credential values masked)' becomes '`path` (query dropped, as in 0.7)', and the same at 353. PRD-05 step 4: 'The query is kept, masked (PRD-01 `http_req` meta), so today's query drop (`metadata.rs:81-106`) goes.' becomes 'The query is dropped, as today (`metadata.rs:81-106`).' Delete row 516's 'Cut'. PRD-03:286: 'path with query' becomes 'path without query'.
- Ledger item or rule: M1 (locked: keep every secret rule, nothing loosened); point 4
- Source: align-L4 L4-5
- Also in: PRD-01-record-format, PRD-05-masking

## L2-14 (minor)

- Section and lines: PRD-03 section 2 'Upstream TCP' (180-183)
- Change: Lines 181-183: 'Retry until 25 s when the CONNECT target is listed (...); one round otherwise, because retrying the agent's own traffic changes the run being measured (C6).' becomes 'One round for every target: retrying the agent's own traffic changes the run being measured and holds it longer than without us (C6). A failure is a `502` at once; the harness retries.'
- Ledger item or rule: R2 (never slow the harness), point 3; less code
- Source: align-L2 L2-14

## L2-15 (minor)

- Section and lines: PRD-03 section 5 (299-300), section 6 (337-338), section 11 (442-443)
- Change: Section 11, after the guard list: 'These three stop parsing only for the connection that passed them. `http_head` must not: past 64 KiB the parser keeps reading the head without storing it, writes GAP `memory_guard` (`http_head`) for the head bytes it dropped, and goes on with the body and later requests.' Leave `upgrade_copy` and `hpack_table` as they are (rare), and say so.
- Ledger item or rule: R5 (a guard writes a gap and goes on)
- Source: align-L2 L2-15

## L2-16 (minor)

- Section and lines: PRD-03 section 2 ClientHello (193), section 9 listed-host GAP list (394-397), section 11 (442); PRD-01 `conn` reasons (231)
- Change: PRD-03:193: 'Guard 64 KiB (`client_hello`); past it, a tunnel with `reason: memory_guard`, and when the CONNECT host is listed, a GAP `memory_guard` (`client_hello`) at close with `lost` = `up` + `down` + 1.' Add `memory_guard` to PRD-01:231's tunnel reasons and to PRD-03:394-397.
- Ledger item or rule: R4
- Source: align-L2 L2-16
- Also in: PRD-01-record-format
