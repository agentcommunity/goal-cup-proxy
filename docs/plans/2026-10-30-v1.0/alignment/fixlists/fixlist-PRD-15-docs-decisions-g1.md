# Fix list for PRD-15-docs-decisions.md (group 1)

From the alignment reviews L1 to L7, triaged against OWNER-DECISIONS.md (the ledger wins).
Files are under docs/plans/2026-10-30-v1.0/. Line numbers are from branch smart/intelligent-newton-0avrpi.
An item with several files carries the same id and the same change text in each file's list; make only the part for this file.

Counts: 1 blocker, 0 major, 5 minor.

## L4-1 (blocker)

- Section and lines: PRD-00 owner question 3 (709-712), PRD table row (213), status (238-239), second slice (550); PRD-05 owner decisions (34-37), rule rows 17 and 20 (129, 132), section 9 (334-395), section 10 rows (405-406, 409, 415-417), section 12 (453-465), overridable defaults (594, 597), builder step 6 (621); PRD-11 `secret_files` and `secret_values` (151, 339-340, 361-366, each connector's `secret_files` line); PRD-10 mask values call (153), `--prompt` risk row (732); PRD-12 planted `sk_test_`, `whsec_` and the minute 5 canary (257-259, 265-266, 598, 806); PRD-15 D80 (340)
- Change: Build ledger M1 as written by default; the extra masking waits for the owner (owner list OW-01). PRD-00 question 3: 'Default: yes.' becomes 'Default: no until the owner answers. Built only if the owner adds it to the ledger as M1a.' PRD-05: in 'Owner decisions' replace 'default yes, built in the second slice' with 'default no; not built unless the owner says yes'; mark rows 17 (Stripe test keys) and 20 (`whsec_`) and section 9 sources 2 to 4 'only with owner yes'; the default build keeps the `GH_TOKEN` exact value only (as today); section 12's default text is the 'no' variant (463-465). PRD-11: `secret_files` and `secret_values` are not read in the default build (the data rows may stay). PRD-10:153: only `mask::workspace_values` for `GH_TOKEN`. PRD-12: plant `sk_test_`, `whsec_` and the canary only when the owner said yes; otherwise drop them from `.env.local` and the known-values file. PRD-15 D80: remove 'and the key values from the four sources ... (default, PRD-00 question 12)' unless the owner says yes. If the owner says yes before H+0, none of this changes and the second slice builds it as planned.
- Ledger item or rule: M1 (locked: the masking list); B2a (a locked item changes only with the owner)
- Source: align-L4 L4-1
- Also in: PRD-00-index, PRD-05-masking, PRD-11-connectors-harnesses, PRD-10-cli-participant, PRD-12-test-pad

## L4-8 (minor)

- Section and lines: PRD-15 D80 (340)
- Change: D80 names the left guard class: 'glue is a letter, a digit or `_`' (plus 'or `-`' only with owner yes, OW-02).
- Ledger item or rule: M1; point 6
- Source: align-L4 L4-8

## L7-05 (minor)

- Section and lines: PRD-14 section 6 rule 1 (220-227); PRD-15 AGENTS.md map paragraph
- Change: PRD-14 rule 1: scan `crates/goalcup/src/**` and `crates/goalcup-format/src/**`. PRD-15 AGENTS.md map paragraph (not the hard-rule line): add 'Core code is `crates/goalcup` and `crates/goalcup-format`. The usage reader in `crates/goalcup-verify` reads harness session formats after the run, chosen by connector id.'
- Ledger item or rule: Hard rule 'No harness names in core code' (section 7 row 3)
- Source: align-L7 L7-05
- Also in: PRD-14-tests-gate

## L7-07 (minor)

- Section and lines: PRD-00 'Consequences' (681-683); PRD-15 (254-256); PRD-16 stop table (299)
- Change: PRD-00:681-683 and PRD-15:254-256: 'because the ledger allows two changes only' becomes 'because ledger section 7 changes only the rows it lists; ending the harness is today's behaviour, raised as owner question 2 (old 11)'. PRD-16:299: 'beyond PRD-00's two changes' becomes 'beyond the changes ledger section 7 lists'.
- Ledger item or rule: Ledger section 7
- Source: align-L7 L7-07
- Also in: PRD-00-index, PRD-16-orchestration-acceptance

## L7-08 (minor)

- Section and lines: PRD-15 participant guide harness table, Claude Code 'Gateway' cell (396)
- Change: Replace the cell with 'You set `ANTHROPIC_BASE_URL` in the `settings.json` of goalcup's Claude Code home; goalcup never sets it'.
- Ledger item or rule: Hard rule 'Never set a model base URL'
- Source: align-L7 L7-08

## L7-11 (minor)

- Section and lines: PRD-15 new hard rule (220)
- Change: 'Never commit anything under `pad-out/`, `harnesses/` or `target/`' becomes 'Never commit anything under the repository root's `/pad-out/` or `/harnesses/`, or any `target/`, a key, or a real run's record.'
- Ledger item or rule: Hard rule 'Never commit ... Stands'
- Source: align-L7 L7-11
