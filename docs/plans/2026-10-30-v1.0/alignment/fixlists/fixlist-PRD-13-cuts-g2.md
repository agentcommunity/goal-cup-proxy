# Fix list for PRD-13-cuts.md (triage group 2)

Source reviews: align-X1-run1, align-X2-scope, align-X3-builder, align-PRD-00 to align-PRD-06.
The ledger (OWNER-DECISIONS.md) wins. An id shared by several fix lists carries the same change text in each;
make in this file only the part that names this file. Items are ordered by severity, then id.

Counts: 0 blocker, 1 major, 2 minor, 3 in all.

## G2-38 (major)
- Sources: X2-01, P00-20
- Section and lines here: 'Target size after 1.0' table (454-492, line 472)
- Same id also in: PRD-00-index, BUILD-PROMPT, PRD-16-orchestration-acceptance
- Ledger item or rule: Ledger L2 (locked: less code, cut before you add); VISION 'Less code'
- Change:
  PRD-00 lines 775-777: replace "The 18,700 is above ... says why in the night log." with "Each row is a ceiling, not an estimate. A builder who would pass its row first cuts what serves no point; past the row by more than 10% needs a replan `decision` line that names the point the extra lines serve. Reviewer B checks each merge request against its row and against the fresh design's row (audit B, B1.11 and B2.12). Targets after the cuts of X2-02, X2-03, X2-07, X2-08, G2-24 and G2-25: Rust src about 18,100, Rust tests about 15,000." Rust row: "about 18,100 / about 15,000". Use one pad number: "about 1,350 to 1,450 (PRD-12's estimate wins)".
  PRD-13 table: the same two numbers.
  BUILD-PROMPT reviewer B: add "and the PRD's size row (PRD-00 Size budget)".
  PRD-16 Replan: add "and posts `wc -l` of the merged PRD against its row".

## P04-25 (minor)
- Sources: P04-25 (the PRD-05 A4 part is G2-36)
- Section and lines here: Lines 324, 464
- Same id also in: PRD-00-index, PRD-14-tests-gate
- Ledger item or rule: Builder misread in other PRDs
- Change:
  PRD-00 line 977: "Routed to PRD-04 (hourly copies when changed; no copy in `close_dead_entry`, PRD-02 9.4)". PRD-13 lines 324 and 464: 780 src lines. PRD-14 line 93: drop `native_capture.rs`.

## X2-11 (minor)
- Sources: X2-11
- Section and lines here: 'Review answers' section and fully ruled 'Asks' sections
- Same id also in: PRD-00-index, PRD-01-record-format, PRD-02-recorder-core, PRD-03-proxy, PRD-04-native-capture, PRD-05-masking, PRD-06-uploader-compression, PRD-07-worker-server, PRD-08-verifier-checks, PRD-09-usage-research-stats, PRD-10-cli-participant, PRD-11-connectors-harnesses, PRD-12-test-pad, PRD-14-tests-gate, PRD-15-docs-decisions, PRD-16-orchestration-acceptance, PRD-17-lessons-old-code
- Ledger item or rule: Ledger L2; point 6 (plain, true text). Apply last, after every other fix, so line numbers in the fix lists stay valid
- Change:
  Move every "Review answers" section and every fully ruled "Asks" section into one file `docs/plans/2026-10-30-v1.0/REVIEW-HISTORY.md`, verbatim, one heading per PRD. Before moving, copy any ruling still binding into the PRD body. Each PRD keeps one line at its end: "Review history: REVIEW-HISTORY.md, section PRD-NN." The replan keeps new asks in the Amendments note only.
