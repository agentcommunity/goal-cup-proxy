# Fix list for PRD-12-test-pad.md (triage group 2)

Source reviews: align-X1-run1, align-X2-scope, align-X3-builder, align-PRD-00 to align-PRD-06.
The ledger (OWNER-DECISIONS.md) wins. An id shared by several fix lists carries the same change text in each;
make in this file only the part that names this file. Items are ordered by severity, then id.

Counts: 1 blocker, 0 major, 3 minor, 4 in all.

## P00-1 (blocker)
- Sources: P00-1 (and X2 caps table)
- Section and lines here: Spend table (328-329); defaults (699)
- Same id also in: PRD-00-index, PRD-16-orchestration-acceptance
- Ledger item or rule: Ledger T6 (locked: 10 USD per run, cost otherwise not a constraint), B3
- Change:
  PRD-00 Spend row: replace the whole-build stop line with "No whole-build cap (ledger T6). The scribe reports running spend in the night log at 200 and 400 USD by the pad's ledger estimate and keeps going." Owner question 8: delete the build stop line sentence. "2 USD per `smoke` run" becomes "an estimate, not a cap".
  PRD-16 section 9 and defaults, PRD-12 lines 328-329 and 699: delete the whole-build model and sandbox rows; keep the 10 USD per-run cap (T6); `smoke` 2 USD is an estimate, not a cap.

## 06-17 (minor)
- Sources: 06-17
- Section and lines here: `cut-worker` stand-in (195)
- Same id also in: PRD-06-uploader-compression
- Ledger item or rule: Builder misread across PRD-06 and PRD-12
- Change:
  PRD-06 U5: "with the cut from minute 1 for 10 minutes (PRD-12 `cut-worker@1+10`)". Risks: "If `getent` shows the cut did not take, PRD-12's stand-in is used (a local listener that refuses or holds the connection; it never terminates TLS); else the row says 'not run: cut did not take'." Delete "so no forwarder" from B W5. PRD-12: the stand-in never terminates TLS.

## X2-07 (minor)
- Sources: X2-07
- Section and lines here: Section 3.1 (182-212); files table `exec/` (573); risk (681)
- Ledger item or rule: Ledger T4 (locked: one tool, three cases), L2
- Change:
  Keep the fault kinds `kill-recorder` (done item 2) and `cut-worker` (U1). Run PRD-02 R4, R5 and R7 once each through `--exec` scripts, not as fault kinds with their own expectations in `check.py`. Drop `route-drop` and `freeze`; PRD-02 R4 stays proven by `watchdog_rlimit.rs`. Files table: `pad-run` (300) and `check.py` (320) shrink by the fault branches (about 80 lines).

## X2-11 (minor)
- Sources: X2-11
- Section and lines here: 'Review answers' section and fully ruled 'Asks' sections
- Same id also in: PRD-00-index, PRD-01-record-format, PRD-02-recorder-core, PRD-03-proxy, PRD-04-native-capture, PRD-05-masking, PRD-06-uploader-compression, PRD-07-worker-server, PRD-08-verifier-checks, PRD-09-usage-research-stats, PRD-10-cli-participant, PRD-11-connectors-harnesses, PRD-13-cuts, PRD-14-tests-gate, PRD-15-docs-decisions, PRD-16-orchestration-acceptance, PRD-17-lessons-old-code
- Ledger item or rule: Ledger L2; point 6 (plain, true text). Apply last, after every other fix, so line numbers in the fix lists stay valid
- Change:
  Move every "Review answers" section and every fully ruled "Asks" section into one file `docs/plans/2026-10-30-v1.0/REVIEW-HISTORY.md`, verbatim, one heading per PRD. Before moving, copy any ruling still binding into the PRD body. Each PRD keeps one line at its end: "Review history: REVIEW-HISTORY.md, section PRD-NN." The replan keeps new asks in the Amendments note only.
