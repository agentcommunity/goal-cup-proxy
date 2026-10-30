# Fix list for PRD-15-docs-decisions.md (triage group 2)

Source reviews: align-X1-run1, align-X2-scope, align-X3-builder, align-PRD-00 to align-PRD-06.
The ledger (OWNER-DECISIONS.md) wins. An id shared by several fix lists carries the same change text in each;
make in this file only the part that names this file. Items are ordered by severity, then id.

Counts: 0 blocker, 0 major, 3 minor, 3 in all.

## X2-08 (minor)
- Sources: X2-08
- Section and lines here: Rule 7 (87); guide appendix C (614-617); risk row (632)
- Same id also in: PRD-14-tests-gate, PRD-10-cli-participant
- Ledger item or rule: Ledger L7 (no backward compatibility)
- Change:
  PRD-14: delete "on upgrade it deletes a `goalcup-daemon` left by 0.7 in the prefix and the 0.7 `share/goalcup/connectors/` folder;".
  PRD-15: delete guide appendix C and its exceptions (87, 614-617); risk row 632 says "0.7 is refused once the floor is set".
  PRD-10 section 11: delete the 0.7 `goalcup mcp` paragraph; one line in the release notes covers testers.

## X2-09 (minor)
- Sources: X2-09
- Section and lines here: Section 2 rule 2 (75-76); guide harness table (97, 390-391)
- Same id also in: PRD-11-connectors-harnesses
- Ledger item or rule: Ledger L2; PRD-15 rule 2 (one place per fact); point 6
- Change:
  One harness table: `docs/harnesses/README.md`, tier columns generated from `connectors/tiers.toml`. PRD-15 line 97: "the harness table with tiers" becomes "a link to `docs/harnesses/README.md`"; delete the table text at 390-391. PRD-11 line 715: delete "plus the pad recipe (appendix O section .7 of each harness)"; fact pages hold facts and limits only; the pad recipe lives only in `scripts/pad/harnesses/<id>.toml`.

## X2-11 (minor)
- Sources: X2-11
- Section and lines here: 'Review answers' section and fully ruled 'Asks' sections
- Same id also in: PRD-00-index, PRD-01-record-format, PRD-02-recorder-core, PRD-03-proxy, PRD-04-native-capture, PRD-05-masking, PRD-06-uploader-compression, PRD-07-worker-server, PRD-08-verifier-checks, PRD-09-usage-research-stats, PRD-10-cli-participant, PRD-11-connectors-harnesses, PRD-12-test-pad, PRD-13-cuts, PRD-14-tests-gate, PRD-16-orchestration-acceptance, PRD-17-lessons-old-code
- Ledger item or rule: Ledger L2; point 6 (plain, true text). Apply last, after every other fix, so line numbers in the fix lists stay valid
- Change:
  Move every "Review answers" section and every fully ruled "Asks" section into one file `docs/plans/2026-10-30-v1.0/REVIEW-HISTORY.md`, verbatim, one heading per PRD. Before moving, copy any ruling still binding into the PRD body. Each PRD keeps one line at its end: "Review history: REVIEW-HISTORY.md, section PRD-NN." The replan keeps new asks in the Amendments note only.
