# Fix list for PRD-17-lessons-old-code.md (group 1)

From the alignment reviews L1 to L7, triaged against OWNER-DECISIONS.md (the ledger wins).
Files are under docs/plans/2026-10-30-v1.0/. Line numbers are from branch smart/intelligent-newton-0avrpi.
An item with several files carries the same id and the same change text in each file's list; make only the part for this file.

Counts: 0 blocker, 2 major, 1 minor.

## F2 (major)

- Section and lines: PRD-17 'Today' (46-65); risk 'The list is a sample' (450-453)
- Change: Add 'Today' rows for `crates/goalcup-daemon/src/runtime/capture/{writer,finish,sink}.rs`, `crates/goalcup-protocol/src/{chain,seal,export,capture_status}.rs`, `crates/goalcup-verifier/src/**` and `crates/goalcup-cli/src/commands/{start,continue_run,status}*`. Add sections '2.14 Format, writer and recovery (guards in PRD-01, PRD-02)', '2.15 Verifier (guards in PRD-08)' and '2.16 Start, continue and status (guards in PRD-10)'. Add a DECISIONS table: one line per D entry, either a lesson ID or 'no edge case: <why>'. Replace the risk text at 450-453 with 'Every old crate, its tests and every DECISIONS entry were read; a case a pad run finds later is added as a row.'
- Ledger item or rule: L4 (locked: every hard-won edge case)
- Source: align-L1 F2

## F3 (major)

- Section and lines: PRD-17 sections 2.9 to 2.13 (254-321) and gate check 1 (438-441); the receiving test lists in PRD-10, PRD-06 and PRD-11
- Change: Give every row in PRD-17 2.9 to 2.13 an `A: file::test` or `R: <pad row>`, or the fate `D` or `X` with the decision named. Concretely: X4 (HF4 after F4) `A: PRD-10 cli_commands.rs::second_start_on_one_home_refuses`; S7 owner PRD-06, `A: upload_beat.rs::clock_set_back_does_not_lengthen_a_segment`; X5 (HF5) `A: PRD-11 connector_launch.rs::codex_gets_no_dash_c_override`; X6 (HF6) `D` with 'participant text, PRD-10 section 11'. Change gate check 1 to 'Every row in section 2 has an `A:` guard, an `R:` row or a `D` or `X` fate; every `A:` name appears in `cargo nextest list`.' PRD-10, PRD-06 and PRD-11: add the named test to their acceptance or test list.
- Ledger item or rule: L4 (locked: each turned into an acceptance case)
- Source: align-L1 F3
- Also in: PRD-10-cli-participant, PRD-06-uploader-compression, PRD-11-connectors-harnesses

## F4 (minor)

- Section and lines: PRD-17 section 2.12 (295-306), fate codes (92-98), gate check 4 (447); citations PRD-11:205, :749; PRD-10:175, :195, :730
- Change: Rename harness-fact IDs X1 to X8 to HF1 to HF8 in PRD-17 and in every citation (PRD-11:205 'PRD-17 X4', :749 'X1 to X8'; PRD-10:175 'X4', :195 'X6', :730 'X6'). Gate check 4 then reads 'Every row with fate `X` or `I` names the owner decision or PRD that made it so.'
- Ledger item or rule: L4 (clarity)
- Source: align-L1 F4
- Also in: PRD-11-connectors-harnesses, PRD-10-cli-participant
