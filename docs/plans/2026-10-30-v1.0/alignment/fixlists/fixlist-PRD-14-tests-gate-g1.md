# Fix list for PRD-14-tests-gate.md (group 1)

From the alignment reviews L1 to L7, triaged against OWNER-DECISIONS.md (the ledger wins).
Files are under docs/plans/2026-10-30-v1.0/. Line numbers are from branch smart/intelligent-newton-0avrpi.
An item with several files carries the same id and the same change text in each file's list; make only the part for this file.

Counts: 1 blocker, 0 major, 1 minor.

## L6-06 (blocker)

- Section and lines: PRD-14 section 4 'Start' paragraph (280-283)
- Change: 'a build box used by the gate and PRD-12's rebuild step, or the fallback of PRD-16 S8 (builds one at a time on the conductor as a non-root user).' becomes 'a build box used by the gate and PRD-12's rebuild step. Nothing ever builds on the conductor (ledger B1, PRD-16 S8).'
- Ledger item or rule: B1 (locked)
- Source: align-L6 L6-06

## L7-05 (minor)

- Section and lines: PRD-14 section 6 rule 1 (220-227); PRD-15 AGENTS.md map paragraph
- Change: PRD-14 rule 1: scan `crates/goalcup/src/**` and `crates/goalcup-format/src/**`. PRD-15 AGENTS.md map paragraph (not the hard-rule line): add 'Core code is `crates/goalcup` and `crates/goalcup-format`. The usage reader in `crates/goalcup-verify` reads harness session formats after the run, chosen by connector id.'
- Ledger item or rule: Hard rule 'No harness names in core code' (section 7 row 3)
- Source: align-L7 L7-05
- Also in: PRD-15-docs-decisions
