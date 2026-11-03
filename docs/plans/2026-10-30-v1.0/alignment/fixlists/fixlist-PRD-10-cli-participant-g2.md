# Fix list for PRD-10-cli-participant.md (triage group 2)

Source reviews: align-X1-run1, align-X2-scope, align-X3-builder, align-PRD-00 to align-PRD-06.
The ledger (OWNER-DECISIONS.md) wins. An id shared by several fix lists carries the same change text in each;
make in this file only the part that names this file. Items are ordered by severity, then id.

Counts: 0 blocker, 3 major, 4 minor, 7 in all.

## G2-07 (major)
- Sources: X3-7, P00-10
- Section and lines here: Interfaces (623)
- Same id also in: PRD-00-index
- Ledger item or rule: PRD-00 seams rule; ruling R3-11 (secret_values)
- Change:
  PRD-00 S3: replace the code block with "The final shape is PRD-11 'Design' lines 315-346: `load`, `plan`, `Env`, `LaunchPlan { connector, harness_version, program, argv, env, home, lock, before, after, hosts: HostList, native, mask_rules, secret_values, notes, refusal }`, `finish_env`; plus `HostList::decide`, `HostList::ca_subtrees` and `hosts::check` as below."
  PRD-10 line 623: use `connector::load` then `plan(&connector, &env, home)` (Amendments note if after H+0).

## P04-5 (major)
- Sources: P04-5
- Section and lines here: Status lines
- Same id also in: PRD-04-native-capture, PRD-01-record-format
- Ledger item or rule: Ledger R4 (no silent loss), point 6
- Change:
  PRD-04 section 9 bullet, append: "Each such folder gets one `note` `native_other_device` (`row`, the path relative to the root) per entry, so the miss is named." `native_walk.rs`: "a folder whose `st_dev` differs (an existing mount under the test root, or skipped with a printed reason when none exists) is not entered and gives one note". Builder notes: use `find <root> -xdev`.
  PRD-01: add `native_other_device` to the note list. PRD-10: one plain line for it.

## X2-03 (major)
- Sources: X2-03 (parts b, c, d; the sleep lock, part a, is owner item O-5)
- Section and lines here: Section 7 step 4 (413-415); section 8 (455-461); `cli_push.rs` (705); defaults (754); files table (593, 595, 597-598)
- Same id also in: PRD-00-index
- Ledger item or rule: Ledger L2; VISION 'Less code' and 'No caps of our own'
- Change:
  PRD-10 section 8: keep `--prompt` (owner question 2 default yes) but drop the 64 KiB refusal: "reads the file as UTF-8; no size limit". Remove the 64 KiB case from `cli_push.rs` and the `--prompt` size row from defaults.
  Line 415: delete "the model hosts of the chosen connector reachable (TCP, a warning only: F3);" and "on battery (H11);".
  Files table: `text.rs` 520 becomes about 420; recount the total (about 2,600 if the sleep lock also goes, O-5). PRD-00 CLI row follows.

## G2-33 (minor)
- Sources: X1-9, P02-29
- Section and lines here: End screen rule 3
- Same id also in: PRD-01-record-format, PRD-02-recorder-core
- Ledger item or rule: Ledger R6 ('a model call counts only once its bytes are in the record'), point 6
- Change:
  PRD-01 section 9, after the `req_whole` bullet: "The heartbeat, status and end screen show `req_whole` next to `model_requests` whenever `req_holes` is above 0." PRD-02 section 6: "Status shows `req_holes` and `req_whole` beside `model_requests`, so a request whose body is in a gap is never shown as whole." PRD-10 end screen rule 3: name `req_whole` as the "written whole" number.

## P05-15 (minor)
- Sources: P05-15 (applies only if exact values are built, G2-18)
- Section and lines here: Start screen
- Same id also in: PRD-05-masking
- Ledger item or rule: Points 4 and 6
- Change:
  PRD-05: count only the values past 32 in `skipped`; drop gate refusals from it. PRD-10: when `skipped` is above 0, the start screen says "N key values were not added to masking."

## X2-08 (minor)
- Sources: X2-08
- Section and lines here: Section 11 (529-531, the 0.7 MCP line)
- Same id also in: PRD-14-tests-gate, PRD-15-docs-decisions
- Ledger item or rule: Ledger L7 (no backward compatibility)
- Change:
  PRD-14: delete "on upgrade it deletes a `goalcup-daemon` left by 0.7 in the prefix and the 0.7 `share/goalcup/connectors/` folder;".
  PRD-15: delete guide appendix C and its exceptions (87, 614-617); risk row 632 says "0.7 is refused once the floor is set".
  PRD-10 section 11: delete the 0.7 `goalcup mcp` paragraph; one line in the release notes covers testers.

## X2-11 (minor)
- Sources: X2-11
- Section and lines here: 'Review answers' section and fully ruled 'Asks' sections
- Same id also in: PRD-00-index, PRD-01-record-format, PRD-02-recorder-core, PRD-03-proxy, PRD-04-native-capture, PRD-05-masking, PRD-06-uploader-compression, PRD-07-worker-server, PRD-08-verifier-checks, PRD-09-usage-research-stats, PRD-11-connectors-harnesses, PRD-12-test-pad, PRD-13-cuts, PRD-14-tests-gate, PRD-15-docs-decisions, PRD-16-orchestration-acceptance, PRD-17-lessons-old-code
- Ledger item or rule: Ledger L2; point 6 (plain, true text). Apply last, after every other fix, so line numbers in the fix lists stay valid
- Change:
  Move every "Review answers" section and every fully ruled "Asks" section into one file `docs/plans/2026-10-30-v1.0/REVIEW-HISTORY.md`, verbatim, one heading per PRD. Before moving, copy any ruling still binding into the PRD body. Each PRD keeps one line at its end: "Review history: REVIEW-HISTORY.md, section PRD-NN." The replan keeps new asks in the Amendments note only.
