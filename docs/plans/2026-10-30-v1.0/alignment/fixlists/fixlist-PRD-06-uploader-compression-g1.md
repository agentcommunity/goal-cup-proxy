# Fix list for PRD-06-uploader-compression.md (group 1)

From the alignment reviews L1 to L7, triaged against OWNER-DECISIONS.md (the ledger wins).
Files are under docs/plans/2026-10-30-v1.0/. Line numbers are from branch smart/intelligent-newton-0avrpi.
An item with several files carries the same id and the same change text in each file's list; make only the part for this file.

Counts: 0 blocker, 3 major, 0 minor.

## F3 (major)

- Section and lines: PRD-17 sections 2.9 to 2.13 (254-321) and gate check 1 (438-441); the receiving test lists in PRD-10, PRD-06 and PRD-11
- Change: Give every row in PRD-17 2.9 to 2.13 an `A: file::test` or `R: <pad row>`, or the fate `D` or `X` with the decision named. Concretely: X4 (HF4 after F4) `A: PRD-10 cli_commands.rs::second_start_on_one_home_refuses`; S7 owner PRD-06, `A: upload_beat.rs::clock_set_back_does_not_lengthen_a_segment`; X5 (HF5) `A: PRD-11 connector_launch.rs::codex_gets_no_dash_c_override`; X6 (HF6) `D` with 'participant text, PRD-10 section 11'. Change gate check 1 to 'Every row in section 2 has an `A:` guard, an `R:` row or a `D` or `X` fate; every `A:` name appears in `cargo nextest list`.' PRD-10, PRD-06 and PRD-11: add the named test to their acceptance or test list.
- Ledger item or rule: L4 (locked: each turned into an acceptance case)
- Source: align-L1 F3
- Also in: PRD-17-lessons-old-code, PRD-10-cli-participant, PRD-11-connectors-harnesses

## L2-03 (major)

- Section and lines: PRD-02 section 5 steps 1 and 5 (247, 263), section 6 `mode` (305-306), hard guard (188, 211-214); PRD-06 section 6 (258-261, 276-282)
- Change: PRD-02 section 5: move the status tick out of the writer loop: '5. Every second, a status thread (not the writer) writes `status.json` from the live atomics: `written`, `losing`, `held_bytes`, `writer_idle_ms`.' Add to section 3 or 8: 'Writer stall: when records wait and the writer has written nothing for 60 s on boot time, the recorder exits 75 and the watchdog restarts it, so the stall becomes a `recorder_down` gap with cause `writer_stalled`.' PRD-06 section 6 `recording` table: compute `mode` from `Durable` age and the live atomics, not only from `status.json`, and send `failed` when `writer_idle_ms` passes 60 s.
- Ledger item or rule: R3, R4, R7 (locked)
- Source: align-L2 L2-03
- Also in: PRD-02-recorder-core

## L4-2 (major)

- Section and lines: PRD-01 risks (836-837), overridable defaults row 'zstd' (864); PRD-06 section 2 (104-106, table row 114), overridable defaults row 'zstd' (605)
- Change: PRD-01:837: 'On a small machine PRD-06 may drop to `window_log` 26 (73 MiB); the format allows any window up to 27.' becomes 'Window 27 is locked (ledger F2); a smaller window needs the owner.' PRD-01:864 and PRD-06:605: split into 'level: 3, PRD-06 with a measured reason (level 1 first)' and 'window 27 and long matching: locked (ledger F2), owner only'. PRD-06:105: 'PRD-01 allows window 26; we do not use it until a measured machine needs it.' becomes 'Window 27 is locked (ledger F2).' Drop the 73 MiB row at 114 or label it 'not allowed without the owner'.
- Ledger item or rule: F2 (locked: zstd `--long=27`)
- Source: align-L4 L4-2
- Also in: PRD-01-record-format
