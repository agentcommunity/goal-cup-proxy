# Fix list for PRD-13-cuts.md (group 1)

From the alignment reviews L1 to L7, triaged against OWNER-DECISIONS.md (the ledger wins).
Files are under docs/plans/2026-10-30-v1.0/. Line numbers are from branch smart/intelligent-newton-0avrpi.
An item with several files carries the same id and the same change text in each file's list; make only the part for this file.

Counts: 0 blocker, 2 major, 3 minor.

## F1 (major)

- Section and lines: PRD-16 section 5 'Cut-off H+10' (226-228) and S7 (338); PRD-13 section 5 (175-177) and risk 'The cut is skipped' (531); PRD-00 phase 3 row (563)
- Change: PRD-16:338 S7 'What happens': 'No cut (section 5)' becomes 'No cut yet (section 5). Calls the owner (W6): locked L3 cannot hold on time. The lessons lane keeps working; the cut lands in the same build as soon as the gate is green, before `MORNING.md`.' Same sentence in PRD-16:226-228, PRD-13:175-177 and :531. PRD-00:563: 'the end-of-build report says so' becomes 'the owner is called under W6 and the cut lands as soon as the gate is green'.
- Ledger item or rule: L3, L6 (locked: delete the old crates in the same build)
- Source: align-L1 F1
- Also in: PRD-16-orchestration-acceptance, PRD-00-index

## L7-01 (major)

- Section and lines: PRD-12 section 1 output (132), 'Files it owns' (559-584); PRD-13 'Files it owns'; PRD-00 ownership table (no `.gitignore` row). Checked: repo `.gitignore` has no `pad-out` line today
- Change: PRD-12 'Files it owns', Change: add '`.gitignore`: add `/pad-out/` in the pad's first commit (phase 0), before any run writes there.' PRD-13 'Files it owns': add '`.gitignore`: remove the `spikes/*/out/` and `spikes/wire-pty/out/` lines in the commit that deletes `spikes/`.' PRD-00 ownership table: add a `.gitignore` row naming PRD-12 (the `pad-out` line) and PRD-13 (the `spikes` lines).
- Ledger item or rule: Section 7 'Never commit ... Stands'; point 4
- Source: align-L7 L7-01
- Also in: PRD-12-test-pad, PRD-00-index

## F8 (minor)

- Section and lines: PRD-13 section 7 'What is not cut' (228-239)
- Change: Add a row: '`test-hooks` feature in `crates/goalcup` and `crates/goalcup-verify` (six `GOALCUP_TEST_*` hooks, `Recorder::for_tests`) / about 60 src / A new seam for tests that prove points 1 to 3 without real faults. Off by default; PRD-14's release smoke fails if `GOALCUP_TEST_` appears in a release binary; no other crate may declare it (no-return guard).'
- Ledger item or rule: L6 (legacy and test seams cut; a kept seam is justified)
- Source: align-L1 F8

## F11 (minor)

- Section and lines: PRD-13 section 5 (175) and interfaces row PRD-16 (268)
- Change: '(H+9 today)' becomes '(H+10, PRD-00 R3-8)' at 175; 'the H+9 rule for skipping the cut' becomes 'the H+10 rule' at 268 (and per F1 the rule calls the owner instead of skipping).
- Ledger item or rule: PRD-00 R3-8; L6
- Source: align-L1 F11

## L3-5 (minor)

- Section and lines: PRD-00 'One system' (98-102); PRD-13 section 3 (103)
- Change: PRD-00 after line 102 add: 'Words: the ledger's "proxy dodgers (Amp, Cursor)" (H5) are the plan's vendor backends: listed, decrypted, recorded, tier "verified" or "not verified". Only a client that ignores `HTTPS_PROXY`, pins, or refuses the CA on its model host gets "not supported" (T3).' PRD-13:103: 'and the tier "not supported"' becomes 'and, when the harness's own model host is affected, the tier "not supported" (T3); Amp and Cursor are vendor backends, not this case'.
- Ledger item or rule: H5, H10 (wording); T3
- Source: align-L3 L3-5
- Also in: PRD-00-index
