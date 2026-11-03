# Fix list for PRD-11-connectors-harnesses.md (group 1)

From the alignment reviews L1 to L7, triaged against OWNER-DECISIONS.md (the ledger wins).
Files are under docs/plans/2026-10-30-v1.0/. Line numbers are from branch smart/intelligent-newton-0avrpi.
An item with several files carries the same id and the same change text in each file's list; make only the part for this file.

Counts: 1 blocker, 6 major, 9 minor.

## L4-1 (blocker)

- Section and lines: PRD-00 owner question 3 (709-712), PRD table row (213), status (238-239), second slice (550); PRD-05 owner decisions (34-37), rule rows 17 and 20 (129, 132), section 9 (334-395), section 10 rows (405-406, 409, 415-417), section 12 (453-465), overridable defaults (594, 597), builder step 6 (621); PRD-11 `secret_files` and `secret_values` (151, 339-340, 361-366, each connector's `secret_files` line); PRD-10 mask values call (153), `--prompt` risk row (732); PRD-12 planted `sk_test_`, `whsec_` and the minute 5 canary (257-259, 265-266, 598, 806); PRD-15 D80 (340)
- Change: Build ledger M1 as written by default; the extra masking waits for the owner (owner list OW-01). PRD-00 question 3: 'Default: yes.' becomes 'Default: no until the owner answers. Built only if the owner adds it to the ledger as M1a.' PRD-05: in 'Owner decisions' replace 'default yes, built in the second slice' with 'default no; not built unless the owner says yes'; mark rows 17 (Stripe test keys) and 20 (`whsec_`) and section 9 sources 2 to 4 'only with owner yes'; the default build keeps the `GH_TOKEN` exact value only (as today); section 12's default text is the 'no' variant (463-465). PRD-11: `secret_files` and `secret_values` are not read in the default build (the data rows may stay). PRD-10:153: only `mask::workspace_values` for `GH_TOKEN`. PRD-12: plant `sk_test_`, `whsec_` and the canary only when the owner said yes; otherwise drop them from `.env.local` and the known-values file. PRD-15 D80: remove 'and the key values from the four sources ... (default, PRD-00 question 12)' unless the owner says yes. If the owner says yes before H+0, none of this changes and the second slice builds it as planned.
- Ledger item or rule: M1 (locked: the masking list); B2a (a locked item changes only with the owner)
- Source: align-L4 L4-1
- Also in: PRD-00-index, PRD-05-masking, PRD-10-cli-participant, PRD-12-test-pad, PRD-15-docs-decisions

## F3 (major)

- Section and lines: PRD-17 sections 2.9 to 2.13 (254-321) and gate check 1 (438-441); the receiving test lists in PRD-10, PRD-06 and PRD-11
- Change: Give every row in PRD-17 2.9 to 2.13 an `A: file::test` or `R: <pad row>`, or the fate `D` or `X` with the decision named. Concretely: X4 (HF4 after F4) `A: PRD-10 cli_commands.rs::second_start_on_one_home_refuses`; S7 owner PRD-06, `A: upload_beat.rs::clock_set_back_does_not_lengthen_a_segment`; X5 (HF5) `A: PRD-11 connector_launch.rs::codex_gets_no_dash_c_override`; X6 (HF6) `D` with 'participant text, PRD-10 section 11'. Change gate check 1 to 'Every row in section 2 has an `A:` guard, an `R:` row or a `D` or `X` fate; every `A:` name appears in `cargo nextest list`.' PRD-10, PRD-06 and PRD-11: add the named test to their acceptance or test list.
- Ledger item or rule: L4 (locked: each turned into an acceptance case)
- Source: align-L1 F3
- Also in: PRD-17-lessons-old-code, PRD-10-cli-participant, PRD-06-uploader-compression

## L3-1 (major)

- Section and lines: PRD-11 schema row `update_off` (157), section 7 'What auto-update off can and cannot do' (685-690); PRD-10 setup step 3 (405-411), asks row C1 (770)
- Change: PRD-11:157: 'Written by `goalcup setup` into the isolated home in the same step as `no_prompts` (owner question 8)' becomes 'Written by `connector::plan` into the isolated home at every `goalcup start` and `continue`, before launch, whatever the setup answer. It is not part of the `no_prompts` question.' PRD-11:686-689: '`goalcup setup` writes it into goalcup's own isolated home ... may still update.' becomes '`plan` writes it into goalcup's own isolated home at every start (never the participant's real one). A harness that rewrites the file mid-run may still update; the `start` event records the version at every start.' PRD-11 overridable defaults (861): '`update_off` and `no_prompts` written by setup' becomes '`no_prompts` written by setup; `update_off` written by `plan` at every start'. PRD-10:405-411: drop 'and auto-update', 'which include the auto-update switch where it is a setting (Hermes, Gemini CLI, Qwen Code)' and 'and to not update itself during the run'. PRD-10:770: '`no_prompts` also holds the settings-form auto-update switch (Hermes, Gemini CLI, Qwen Code)' becomes '`update_off` is a separate key that `plan` applies at every start (PRD-11)'.
- Ledger item or rule: H9 (locked: auto-updates off where the harness allows it)
- Source: align-L3 L3-1
- Also in: PRD-10-cli-participant

## L5-04 (major)

- Section and lines: PRD-16 phase 5 (240); PRD-11 section 7 outlook column (662-678); PRD-12 risk row (677)
- Change: PRD-16:240: add 'As each harness's first `full` sample lands, a PRD-09 builder writes its reader from that sample (PRD-09 rule 250), then the row is rerun once. A harness with no reader by the end of phase 5 is "not verified: no log reader".' PRD-11 outlook column: append '; needs a PRD-09 reader written from its phase 5 sample' to every row except Claude Code, Codex and Hermes. PRD-12:677: 'unless PRD-09 writes a reader from their phase 5 sample in time' becomes 'until the PRD-16 phase 5 reader step writes one from their sample'.
- Ledger item or rule: T1, T3, T4 (wire calls match the harness's own log; usage stats present), S2
- Source: align-L5 L5-04
- Also in: PRD-16-orchestration-acceptance, PRD-12-test-pad

## L5-05 (major)

- Section and lines: PRD-11 Claude Code `refuse_if_set` (387); Qwen Code `refuse_if_set` (577); acceptance 4
- Change: Claude Code (387): `refuse_if_set = [{ file = "{home}/settings.json", keys = [...] }, { file = "{workspace}/.claude/settings.json", keys = [...] }, { file = "{workspace}/.claude/settings.local.json", keys = [...] }]` with the same key list. Qwen Code (577): add `{ file = "{workspace}/.qwen/settings.json", keys = ["proxy"] }`. Acceptance 4: 'the same refusal for the workspace file'.
- Ledger item or rule: H7 (locked: the harness's own proxy setting); R4; point 1
- Source: align-L5 L5-05

## L5-06 (major)

- Section and lines: PRD-11 Factory Droid connector (599-613), Cline CLI connector (631-644), acceptance 5 (797-798), builder notes step 4 (879-880)
- Change: Droid: add `after = [["daemon", "stop"]]  # unverified: builder confirms the stop command from docs or --help`. Cline: add `before = [["hub", "stop"]]` and `after = [["hub", "stop"]]`, both marked unverified the same way. Acceptance 5: add 'Cline with a hub left running from a previous start, and OpenCode 2 with a background service left running: the run's model calls are decrypted and no old process made a model call.' If no stop command exists, the facts sheet says so and the row stays 'not verified'.
- Ledger item or rule: H7 (locked: background daemons to stop or bypass); R4; point 1
- Source: align-L5 L5-06

## L7-03 (major)

- Section and lines: PRD-11 schema table `env` (145), `no_prompts` (156), `update_off` (157); acceptance
- Change: PRD-11 schema text after the table: '`load` refuses a connector whose `env` names match (case-insensitive) `BASE_URL`, `API_BASE`, `ENDPOINT`, `_HOST` or `PROXY`, or whose `env`, `no_prompts` or `update_off` values contain `://`. Error: the connector sets a model address, which goalcup never does.' (`NODE_USE_ENV_PROXY` is a code default in section 3 step 4, not connector `env`, so it is not hit.) Add acceptance: '`connector_no_model_address`: each refused shape fails `load`; all 16 shipped connectors load.'
- Ledger item or rule: Hard rule 'Never set a model base URL' (section 7 row 1)
- Source: align-L7 L7-03

## F4 (minor)

- Section and lines: PRD-17 section 2.12 (295-306), fate codes (92-98), gate check 4 (447); citations PRD-11:205, :749; PRD-10:175, :195, :730
- Change: Rename harness-fact IDs X1 to X8 to HF1 to HF8 in PRD-17 and in every citation (PRD-11:205 'PRD-17 X4', :749 'X1 to X8'; PRD-10:175 'X4', :195 'X6', :730 'X6'). Gate check 4 then reads 'Every row with fate `X` or `I` names the owner decision or PRD that made it so.'
- Ledger item or rule: L4 (clarity)
- Source: align-L1 F4
- Also in: PRD-17-lessons-old-code, PRD-10-cli-participant

## L3-2 (minor)

- Section and lines: PRD-11 Claude Code connector `env` (386); overridable defaults (855-861)
- Change: Keep the variable until the owner answers (owner list OW-06), and add a row to the overridable defaults: '`CLAUDE_CODE_RETRY_WATCHDOG=1` / on / Owner (it changes how Claude Code retries; PRD-02 9.1 relies on it to ride over a recorder restart)'. If the owner says no, remove it from line 386.
- Ledger item or rule: H7 (per-harness data kinds); R2, R3
- Source: align-L3 L3-2 (removal itself goes to the owner)

## L3-11 (minor)

- Section and lines: PRD-11 schema `vendor` key (159), Amp connector (510), field lists (353, 746); PRD-10 vendor line
- Change: PRD-11: delete the `vendor` row (159), `vendor = true` (510), and `vendor` from the field lists at 353 and 746. PRD-10 keys its 'vendor backend' line off `conn` meta `kind: "vendor"` in the record, as PRD-03 section 12 (453) already does.
- Ledger item or rule: H1, H10, L2 (one source for one fact; less code)
- Source: align-L3 L3-11
- Also in: PRD-10-cli-participant

## L5-08 (minor)

- Section and lines: PRD-11 builder notes step 1 (867-869); PRD-12 builder notes step 3 (723-724)
- Change: PRD-11 step 1: after 'Write the Claude Code, Codex, Hermes and Kilo files' add 'after reading each vendor's current docs or `--help` of the pinned version, cited in the commit (ledger T7)'. PRD-12 step 3: after '(harness files for Claude Code, Codex and Hermes first)' add the same sentence.
- Ledger item or rule: T7 (locked)
- Source: align-L5 L5-08
- Also in: PRD-12-test-pad

## L5-09 (minor)

- Section and lines: PRD-11 shape table, Dodger row (62)
- Change: 'the label "not supported"' becomes 'the words "not recorded" on the start line; the tier is "not supported" only when the harness's `direct` case looks like this (PRD-12 section 9)'.
- Ledger item or rule: T3, T4
- Source: align-L5 L5-09

## L5-10 (minor)

- Section and lines: PRD-11 risk row 'macOS' (847)
- Change: 'the owner's Mac run checks Goose and Hermes' becomes 'Goose and Hermes on macOS stay "not verified on macOS" until the owner runs them' (no new work added to the owner's Mac run, which PRD-12 P9 keeps to Claude Code).
- Ledger item or rule: T9
- Source: align-L5 L5-10

## L5-11 (minor)

- Section and lines: PRD-11 section 3 step 1 (179-181); Claude Code `refuse_if_set` (387); Codex `refuse_if_set` (414)
- Change: Claude Code connector: add `unset = ["CLAUDE_CODE_CERT_STORE"]`; extend its `refuse_if_set` keys with `"env.no_proxy"` and `"env.NODE_EXTRA_CA_CERTS"`. Codex connector: add `"all_proxy"` and `"NODE_EXTRA_CA_CERTS"` to the `.env` keys.
- Ledger item or rule: H7 (CA variables and own proxy settings)
- Source: align-L5 L5-11

## L5-13 (minor)

- Section and lines: PRD-11 OpenClaw connector (476-493), section 7 row 5 (668); PRD-12 section 8 INCONCLUSIVE causes
- Change: PRD-11 row 5 outlook: append 'Subagents need a Gateway daemon (appendix O H5.4); 1.0 does not start one, so C2 is expected INCONCLUSIVE "no subagent" and the tier "not verified: subagents need the Gateway" (owner list OW-10).' PRD-12 section 8 INCONCLUSIVE causes: add `harness_subagents_unavailable` with that sign.
- Ledger item or rule: T4, H7
- Source: align-L5 L5-13
- Also in: PRD-12-test-pad

## L7-06 (minor)

- Section and lines: PRD-11 builder notes steps 1 and 2 (866-871)
- Change: Step 1: 'with `plan` returning the Claude Code plan' becomes 'with `plan` built from the Claude Code connector file (data), not from code'. Step 2: '`launch.rs` for Claude Code and Codex' becomes '`launch.rs` (one generic planner), proven first on the Claude Code and Codex connector files'.
- Ledger item or rule: Hard rule 'No harness names in core code'
- Source: align-L7 L7-06
