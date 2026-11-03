# Fix list for PRD-07-worker-server.md (triage group 2)

Source reviews: align-X1-run1, align-X2-scope, align-X3-builder, align-PRD-00 to align-PRD-06.
The ledger (OWNER-DECISIONS.md) wins. An id shared by several fix lists carries the same change text in each;
make in this file only the part that names this file. Items are ordered by severity, then id.

Counts: 0 blocker, 1 major, 3 minor, 4 in all.

## G2-31 (major)
- Sources: X1-4, X1-11, 06-10 (and L2-03, L2-07, L2-08 of group 1)
- Section and lines here: Section 4 `failed` row (197); `mode` list
- Same id also in: PRD-02-recorder-core, PRD-06-uploader-compression
- Ledger item or rule: Ledger R3, R4, R7 (heartbeat says `failed` when recording is off)
- Change:
  PRD-02 section 5 step 5: "Every second, a status thread (not the writer) writes `status.json` from the live atomics: `written`, `losing`, `held_bytes`, `writer_idle_ms`." Section 3: "Writer stall: when records wait and the writer has written nothing for 60 s of boot time, or when control items alone pass 2 x soft, the recorder exits 75 and the watchdog restarts it, so the stall becomes a `recorder_down` gap with cause `writer_stalled`. This is a recorder failure under R3, not an R5 guard; it is the one stall exit."
  PRD-06 section 6: compute `mode` from `Durable` age and the live atomics, not only `status.json`. `ok` row: remove "or `writer_slow`". `failed` row: "`mode` null, or `writer_idle_ms` over 60 s while records wait".
  PRD-07 line 197: replace "`mode` null (null mode), or the entry ended while off" with "recording is off: null mode, or a writer stalled past 60 s; a recovered entry is `gaps` (PRD-06 section 6)". Add `sealed` to its `mode` list (the final heartbeat sends `mode: sealed`).

## 06-23 (minor)
- Sources: 06-23 (the review's note for PRD-00 and PRD-07)
- Section and lines here: Section 4
- Same id also in: PRD-00-index
- Ledger item or rule: Ruling R3-2
- Change:
  Where they say `goalcup upload` sends "the seal's totals read from the record", say "from `pipeline::entry_totals` (R3-2)".

## X1-7 (minor)
- Sources: X1-7
- Section and lines here: Cost table (222-223)
- Same id also in: PRD-01-record-format
- Ledger item or rule: Point 5, R5 (no cap, so plan the cost)
- Change:
  PRD-01 section 14, add a bullet: "**200 agents at once** (estimate, 10 times run 1's wire rate, ratio 3.2 to 1): about 8 GB raw and 2.5 GB compressed an hour, about 6 Mbit/s of upload, about 400 GB a week; a home uplink under that keeps the server hours behind, and parts wait on disk compressed." PRD-07 cost table: add the row "200 agents at once, a week: about 400 GB, about $6 a month".

## X2-11 (minor)
- Sources: X2-11
- Section and lines here: 'Review answers' section and fully ruled 'Asks' sections
- Same id also in: PRD-00-index, PRD-01-record-format, PRD-02-recorder-core, PRD-03-proxy, PRD-04-native-capture, PRD-05-masking, PRD-06-uploader-compression, PRD-08-verifier-checks, PRD-09-usage-research-stats, PRD-10-cli-participant, PRD-11-connectors-harnesses, PRD-12-test-pad, PRD-13-cuts, PRD-14-tests-gate, PRD-15-docs-decisions, PRD-16-orchestration-acceptance, PRD-17-lessons-old-code
- Ledger item or rule: Ledger L2; point 6 (plain, true text). Apply last, after every other fix, so line numbers in the fix lists stay valid
- Change:
  Move every "Review answers" section and every fully ruled "Asks" section into one file `docs/plans/2026-10-30-v1.0/REVIEW-HISTORY.md`, verbatim, one heading per PRD. Before moving, copy any ruling still binding into the PRD body. Each PRD keeps one line at its end: "Review history: REVIEW-HISTORY.md, section PRD-NN." The replan keeps new asks in the Amendments note only.
