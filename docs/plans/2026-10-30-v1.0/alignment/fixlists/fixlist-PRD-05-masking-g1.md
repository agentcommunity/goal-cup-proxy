# Fix list for PRD-05-masking.md (group 1)

From the alignment reviews L1 to L7, triaged against OWNER-DECISIONS.md (the ledger wins).
Files are under docs/plans/2026-10-30-v1.0/. Line numbers are from branch smart/intelligent-newton-0avrpi.
An item with several files carries the same id and the same change text in each file's list; make only the part for this file.

Counts: 1 blocker, 3 major, 1 minor.

## L4-1 (blocker)

- Section and lines: PRD-00 owner question 3 (709-712), PRD table row (213), status (238-239), second slice (550); PRD-05 owner decisions (34-37), rule rows 17 and 20 (129, 132), section 9 (334-395), section 10 rows (405-406, 409, 415-417), section 12 (453-465), overridable defaults (594, 597), builder step 6 (621); PRD-11 `secret_files` and `secret_values` (151, 339-340, 361-366, each connector's `secret_files` line); PRD-10 mask values call (153), `--prompt` risk row (732); PRD-12 planted `sk_test_`, `whsec_` and the minute 5 canary (257-259, 265-266, 598, 806); PRD-15 D80 (340)
- Change: Build ledger M1 as written by default; the extra masking waits for the owner (owner list OW-01). PRD-00 question 3: 'Default: yes.' becomes 'Default: no until the owner answers. Built only if the owner adds it to the ledger as M1a.' PRD-05: in 'Owner decisions' replace 'default yes, built in the second slice' with 'default no; not built unless the owner says yes'; mark rows 17 (Stripe test keys) and 20 (`whsec_`) and section 9 sources 2 to 4 'only with owner yes'; the default build keeps the `GH_TOKEN` exact value only (as today); section 12's default text is the 'no' variant (463-465). PRD-11: `secret_files` and `secret_values` are not read in the default build (the data rows may stay). PRD-10:153: only `mask::workspace_values` for `GH_TOKEN`. PRD-12: plant `sk_test_`, `whsec_` and the canary only when the owner said yes; otherwise drop them from `.env.local` and the known-values file. PRD-15 D80: remove 'and the key values from the four sources ... (default, PRD-00 question 12)' unless the owner says yes. If the owner says yes before H+0, none of this changes and the second slice builds it as planned.
- Ledger item or rule: M1 (locked: the masking list); B2a (a locked item changes only with the owner)
- Source: align-L4 L4-1
- Also in: PRD-00-index, PRD-11-connectors-harnesses, PRD-10-cli-participant, PRD-12-test-pad, PRD-15-docs-decisions

## L4-3 (major)

- Section and lines: PRD-05 section 4 rule (193-194), why (205-209), cost (229), overridable defaults (592); PRD-00 'Owner questions'
- Change: Default the left guard class to the ledger's word boundary, `[A-Za-z0-9_]`; `-` is added only if the owner says yes (owner list OW-02). PRD-05:193-194 and 592: class `[A-Za-z0-9_]`; 'Who may change it: Owner' becomes 'Owner; ledger M1 says a letter or digit (a word boundary)'. PRD-05 'What it costs' (229): name `-` as the one possible addition and why (base64url blobs hold `-`). PRD-00 owner questions, add: 'Left guard class: `[A-Za-z0-9_]` (the ledger) or `[A-Za-z0-9_-]` (also `-`, so no guarded rule matches inside a base64url blob). Default: `[A-Za-z0-9_]`.'
- Ledger item or rule: M1 (locked: left word boundary; key after a letter or digit may be missed)
- Source: align-L4 L4-3
- Also in: PRD-00-index

## L4-4 (major)

- Section and lines: PRD-05 section 7 step 2 (310-312), interfaces row PRD-09 (500), acceptance A3 (538), overridable defaults (600); PRD-00 'Owner questions'
- Change: Delete the sentence 'Exception: a name that contains `ratelimit` or `rate-limit` is not masked by the name rule ... in line with dropping the Codex rate-limit rules.' (310-312), the 'Rate-limit headers stay readable (section 7)' clause in the PRD-09 row (500), 'Rate-limit headers and' in A3 (538), and the row at 600. Add to PRD-00 owner questions: 'Unmask rate-limit headers for research? Default: no' (owner list OW-03).
- Ledger item or rule: M1 (locked: keep every secret rule)
- Source: align-L4 L4-4
- Also in: PRD-00-index

## L4-5 (major)

- Section and lines: PRD-01 `http_req` meta (232, 353); PRD-05 section 7 step 4 (317-318), what is cut row (516); PRD-03 request side (286)
- Change: Keep today's query drop (less code, no change to a secret protection). PRD-01:232: '`path` (query kept, credential values masked)' becomes '`path` (query dropped, as in 0.7)', and the same at 353. PRD-05 step 4: 'The query is kept, masked (PRD-01 `http_req` meta), so today's query drop (`metadata.rs:81-106`) goes.' becomes 'The query is dropped, as today (`metadata.rs:81-106`).' Delete row 516's 'Cut'. PRD-03:286: 'path with query' becomes 'path without query'.
- Ledger item or rule: M1 (locked: keep every secret rule, nothing loosened); point 4
- Source: align-L4 L4-5
- Also in: PRD-01-record-format, PRD-03-proxy

## L4-7 (minor)

- Section and lines: PRD-05 section 2 connector table (154), section 8 (329); PRD-16 `MORNING.md` owner steps
- Change: Add under PRD-05 section 8: 'Reading of ledger M1: the Codex rate-limit rules are the four plan, quota and credit rows (`connectors/codex-cli/harness.toml:110-113`); the three id rows stay. The owner confirms (owner list OW-04).' PRD-16's `MORNING.md` owner steps list it if not answered before H+0.
- Ledger item or rule: M1 (wording)
- Source: align-L4 L4-7
- Also in: PRD-16-orchestration-acceptance
