# Fix list for PRD-14-tests-gate.md (triage group 2)

Source reviews: align-X1-run1, align-X2-scope, align-X3-builder, align-PRD-00 to align-PRD-06.
The ledger (OWNER-DECISIONS.md) wins. An id shared by several fix lists carries the same change text in each;
make in this file only the part that names this file. Items are ordered by severity, then id.

Counts: 1 blocker, 1 major, 4 minor, 6 in all.

## X3-2 (blocker)
- Sources: X3-2
- Section and lines here: Builder notes step 1 (513)
- Same id also in: PRD-16-orchestration-acceptance
- Ledger item or rule: Ledger B2 (at least two independent reviewers read each diff, locked)
- Change:
  PRD-16 section 2, after "Every step is a commit": "Every step that changes a file outside `docs/plans/` (the gate change, the workspace edit, each lock refresh, the phase 3 edit) is a merge request on its own `v1.0/prd-16-<slug>` or `v1.0/prd-14-gate` branch, read by reviewers A and B before it lands. Log-only commits (`NIGHT-LOG.md`, `MORNING.md`) are exempt."
  PRD-14 line 513: "by the integrator" becomes "by PRD-14's lane, first in its queue, reviewed by reviewers A and B and merged by the integrator".

## X2-02 (major)
- Sources: X2-02
- Section and lines here: Section 3 (114, 122, 124-125)
- Same id also in: PRD-05-masking
- Ledger item or rule: Ledger L3 (copy the corpus), L2; VISION 'Less code'
- Change:
  PRD-05 line 479: "about 5,000" becomes "about 2,000". After line 482 add: "Tests copy the corpus as data, the split-at-every-byte property test run over the whole corpus, and only the old cases that a PRD-17 lesson names or that pin a rule's edge (PEM pieces, escapes, the left guard, `url_password`, fail-closed per message). Other old cases are not ported; the property test over the corpus covers them." Line 526: "to about 2,000".
  PRD-14 line 114: "2,000 (PRD-05)"; line 122 total "about 15,000"; line 124-125 keeps "Masking is the largest share" only if still true, else drop it.

## G2-35 (minor)
- Sources: X2-06, P05-13 (rename superseded)
- Section and lines here: Section 8 gate table (294); G1 (452)
- Same id also in: PRD-05-masking
- Ledger item or rule: Ledger L2; ledger section 7 test rule (a test stays if it proves a point)
- Change:
  PRD-14: delete the release-mode test row at 294; G1 drops "the `release_mask_bench` report".
  PRD-05: `mask_bench.rs` holds one `#[ignore]` test named `by_hand_mask_bench`, in no gate tier, run once by hand for PRD-02 R8 and A5, its number in the night log. Same words at 479, 505, 540 and ask 5.

## P04-25 (minor)
- Sources: P04-25 (the PRD-05 A4 part is G2-36)
- Section and lines here: Line 93
- Same id also in: PRD-00-index, PRD-13-cuts
- Ledger item or rule: Builder misread in other PRDs
- Change:
  PRD-00 line 977: "Routed to PRD-04 (hourly copies when changed; no copy in `close_dead_entry`, PRD-02 9.4)". PRD-13 lines 324 and 464: 780 src lines. PRD-14 line 93: drop `native_capture.rs`.

## X2-08 (minor)
- Sources: X2-08
- Section and lines here: Section 9 `install.sh` (348-350)
- Same id also in: PRD-15-docs-decisions, PRD-10-cli-participant
- Ledger item or rule: Ledger L7 (no backward compatibility)
- Change:
  PRD-14: delete "on upgrade it deletes a `goalcup-daemon` left by 0.7 in the prefix and the 0.7 `share/goalcup/connectors/` folder;".
  PRD-15: delete guide appendix C and its exceptions (87, 614-617); risk row 632 says "0.7 is refused once the floor is set".
  PRD-10 section 11: delete the 0.7 `goalcup mcp` paragraph; one line in the release notes covers testers.

## X2-11 (minor)
- Sources: X2-11
- Section and lines here: 'Review answers' section and fully ruled 'Asks' sections
- Same id also in: PRD-00-index, PRD-01-record-format, PRD-02-recorder-core, PRD-03-proxy, PRD-04-native-capture, PRD-05-masking, PRD-06-uploader-compression, PRD-07-worker-server, PRD-08-verifier-checks, PRD-09-usage-research-stats, PRD-10-cli-participant, PRD-11-connectors-harnesses, PRD-12-test-pad, PRD-13-cuts, PRD-15-docs-decisions, PRD-16-orchestration-acceptance, PRD-17-lessons-old-code
- Ledger item or rule: Ledger L2; point 6 (plain, true text). Apply last, after every other fix, so line numbers in the fix lists stay valid
- Change:
  Move every "Review answers" section and every fully ruled "Asks" section into one file `docs/plans/2026-10-30-v1.0/REVIEW-HISTORY.md`, verbatim, one heading per PRD. Before moving, copy any ruling still binding into the PRD body. Each PRD keeps one line at its end: "Review history: REVIEW-HISTORY.md, section PRD-NN." The replan keeps new asks in the Amendments note only.
