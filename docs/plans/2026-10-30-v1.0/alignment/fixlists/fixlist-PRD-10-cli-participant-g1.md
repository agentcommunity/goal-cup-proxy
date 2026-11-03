# Fix list for PRD-10-cli-participant.md (group 1)

From the alignment reviews L1 to L7, triaged against OWNER-DECISIONS.md (the ledger wins).
Files are under docs/plans/2026-10-30-v1.0/. Line numbers are from branch smart/intelligent-newton-0avrpi.
An item with several files carries the same id and the same change text in each file's list; make only the part for this file.

Counts: 1 blocker, 2 major, 4 minor.

## L4-1 (blocker)

- Section and lines: PRD-00 owner question 3 (709-712), PRD table row (213), status (238-239), second slice (550); PRD-05 owner decisions (34-37), rule rows 17 and 20 (129, 132), section 9 (334-395), section 10 rows (405-406, 409, 415-417), section 12 (453-465), overridable defaults (594, 597), builder step 6 (621); PRD-11 `secret_files` and `secret_values` (151, 339-340, 361-366, each connector's `secret_files` line); PRD-10 mask values call (153), `--prompt` risk row (732); PRD-12 planted `sk_test_`, `whsec_` and the minute 5 canary (257-259, 265-266, 598, 806); PRD-15 D80 (340)
- Change: Build ledger M1 as written by default; the extra masking waits for the owner (owner list OW-01). PRD-00 question 3: 'Default: yes.' becomes 'Default: no until the owner answers. Built only if the owner adds it to the ledger as M1a.' PRD-05: in 'Owner decisions' replace 'default yes, built in the second slice' with 'default no; not built unless the owner says yes'; mark rows 17 (Stripe test keys) and 20 (`whsec_`) and section 9 sources 2 to 4 'only with owner yes'; the default build keeps the `GH_TOKEN` exact value only (as today); section 12's default text is the 'no' variant (463-465). PRD-11: `secret_files` and `secret_values` are not read in the default build (the data rows may stay). PRD-10:153: only `mask::workspace_values` for `GH_TOKEN`. PRD-12: plant `sk_test_`, `whsec_` and the canary only when the owner said yes; otherwise drop them from `.env.local` and the known-values file. PRD-15 D80: remove 'and the key values from the four sources ... (default, PRD-00 question 12)' unless the owner says yes. If the owner says yes before H+0, none of this changes and the second slice builds it as planned.
- Ledger item or rule: M1 (locked: the masking list); B2a (a locked item changes only with the owner)
- Source: align-L4 L4-1
- Also in: PRD-00-index, PRD-05-masking, PRD-11-connectors-harnesses, PRD-12-test-pad, PRD-15-docs-decisions

## F3 (major)

- Section and lines: PRD-17 sections 2.9 to 2.13 (254-321) and gate check 1 (438-441); the receiving test lists in PRD-10, PRD-06 and PRD-11
- Change: Give every row in PRD-17 2.9 to 2.13 an `A: file::test` or `R: <pad row>`, or the fate `D` or `X` with the decision named. Concretely: X4 (HF4 after F4) `A: PRD-10 cli_commands.rs::second_start_on_one_home_refuses`; S7 owner PRD-06, `A: upload_beat.rs::clock_set_back_does_not_lengthen_a_segment`; X5 (HF5) `A: PRD-11 connector_launch.rs::codex_gets_no_dash_c_override`; X6 (HF6) `D` with 'participant text, PRD-10 section 11'. Change gate check 1 to 'Every row in section 2 has an `A:` guard, an `R:` row or a `D` or `X` fate; every `A:` name appears in `cargo nextest list`.' PRD-10, PRD-06 and PRD-11: add the named test to their acceptance or test list.
- Ledger item or rule: L4 (locked: each turned into an acceptance case)
- Source: align-L1 F3
- Also in: PRD-17-lessons-old-code, PRD-06-uploader-compression, PRD-11-connectors-harnesses

## L3-1 (major)

- Section and lines: PRD-11 schema row `update_off` (157), section 7 'What auto-update off can and cannot do' (685-690); PRD-10 setup step 3 (405-411), asks row C1 (770)
- Change: PRD-11:157: 'Written by `goalcup setup` into the isolated home in the same step as `no_prompts` (owner question 8)' becomes 'Written by `connector::plan` into the isolated home at every `goalcup start` and `continue`, before launch, whatever the setup answer. It is not part of the `no_prompts` question.' PRD-11:686-689: '`goalcup setup` writes it into goalcup's own isolated home ... may still update.' becomes '`plan` writes it into goalcup's own isolated home at every start (never the participant's real one). A harness that rewrites the file mid-run may still update; the `start` event records the version at every start.' PRD-11 overridable defaults (861): '`update_off` and `no_prompts` written by setup' becomes '`no_prompts` written by setup; `update_off` written by `plan` at every start'. PRD-10:405-411: drop 'and auto-update', 'which include the auto-update switch where it is a setting (Hermes, Gemini CLI, Qwen Code)' and 'and to not update itself during the run'. PRD-10:770: '`no_prompts` also holds the settings-form auto-update switch (Hermes, Gemini CLI, Qwen Code)' becomes '`update_off` is a separate key that `plan` applies at every start (PRD-11)'.
- Ledger item or rule: H9 (locked: auto-updates off where the harness allows it)
- Source: align-L3 L3-1
- Also in: PRD-11-connectors-harnesses

## F4 (minor)

- Section and lines: PRD-17 section 2.12 (295-306), fate codes (92-98), gate check 4 (447); citations PRD-11:205, :749; PRD-10:175, :195, :730
- Change: Rename harness-fact IDs X1 to X8 to HF1 to HF8 in PRD-17 and in every citation (PRD-11:205 'PRD-17 X4', :749 'X1 to X8'; PRD-10:175 'X4', :195 'X6', :730 'X6'). Gate check 4 then reads 'Every row with fate `X` or `I` names the owner decision or PRD that made it so.'
- Ledger item or rule: L4 (clarity)
- Source: align-L1 F4
- Also in: PRD-17-lessons-old-code, PRD-11-connectors-harnesses

## L2-12 (minor)

- Section and lines: PRD-02 section 11 step 6 (546-547); PRD-10 end screen and `goalcup status`
- Change: PRD-02:547: 'their count goes to `goalcup.log`' becomes 'their count and hosts go to `goalcup.log` and to `upload.json` (`after_seal_bytes`), and PRD-10's end screen and `goalcup status` name them in the `after_stop` line'. PRD-10: the `after_stop` line also shows `after_seal_bytes` when non-zero.
- Ledger item or rule: R4, point 6
- Source: align-L2 L2-12
- Also in: PRD-02-recorder-core

## L3-6 (minor)

- Section and lines: PRD-10 setup step 4 (414-415)
- Change: Replace 'the model hosts of the chosen connector reachable (TCP, a warning only: F3)' with 'the hosts that the connector's `host_hints` and `hint_env` name, if any, reachable (TCP, a warning only: F3)'.
- Ledger item or rule: H2 (one shared list; connectors hold no hosts)
- Source: align-L3 L3-6

## L3-11 (minor)

- Section and lines: PRD-11 schema `vendor` key (159), Amp connector (510), field lists (353, 746); PRD-10 vendor line
- Change: PRD-11: delete the `vendor` row (159), `vendor = true` (510), and `vendor` from the field lists at 353 and 746. PRD-10 keys its 'vendor backend' line off `conn` meta `kind: "vendor"` in the record, as PRD-03 section 12 (453) already does.
- Ledger item or rule: H1, H10, L2 (one source for one fact; less code)
- Source: align-L3 L3-11
- Also in: PRD-11-connectors-harnesses
