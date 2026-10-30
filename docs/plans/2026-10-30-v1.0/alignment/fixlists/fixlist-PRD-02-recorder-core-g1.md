# Fix list for PRD-02-recorder-core.md (group 1)

From the alignment reviews L1 to L7, triaged against OWNER-DECISIONS.md (the ledger wins).
Files are under docs/plans/2026-10-30-v1.0/. Line numbers are from branch smart/intelligent-newton-0avrpi.
An item with several files carries the same id and the same change text in each file's list; make only the part for this file.

Counts: 1 blocker, 6 major, 7 minor.

## L2-01 (blocker)

- Section and lines: PRD-02 section 2 rule 1 (152-155), section 11 step 2 (533-535) and steps 3-4 (536-538); PRD-00 S1 rules (397), R3-3 (864)
- Change: PRD-02:154: 'A call after the seal's stop is a no-op that only counts (section 11).' becomes 'After the seal's stop, calls on proxy streams are no-ops that only count (section 11); the native thread's calls stay admitted until `NativeReader::finish` returns.' PRD-02:533: 'Every S1 call after this point is a no-op that only counts' becomes 'Every S1 call from a relay task after this point is a no-op that only counts. Calls from the native thread stay admitted until step 4 returns.' PRD-00:397: 'A call after the seal is a no-op.' becomes 'A relay call after the seal's stop is a no-op; native calls stay admitted until `NativeReader::finish` returns (R3-3).' Add a case to `pipeline_seal.rs` (PRD-02 730-732): a native file that grows during the seal and a `final` set are both in the record after the seal.
- Ledger item or rule: R1, R4 (locked), point 2; PRD-00 R3-3 not applied
- Source: align-L2 L2-01
- Also in: PRD-00-index

## F6/L2-06 (major)

- Section and lines: PRD-02 section 8 step 9 (365-374), defaults (794), risks (755-756); PRD-00 rules of the shape (175-178)
- Change: PRD-02:369-370: 'After 5 minutes the watchdog tries a normal boot again ...; each failed try doubles the wait, up to 60 minutes' becomes 'After 5 minutes the watchdog tries a normal boot again ...; each failed try doubles the wait, up to 10 minutes.' Line 794: 'retry after 5 min, doubling to 10 min'. Risks 756: 'at most about six tries an hour'. PRD-00:175-178: 'on a growing timer' becomes 'on a timer that starts at 5 minutes and never passes 10 minutes'. (A 60 s retry, as align-L1 F6 offered, would reset every harness connection once a minute while a cause like a full disk lasts; that slows the harness, R2. Ten minutes keeps the restart fast without that.)
- Ledger item or rule: R3 (locked: restart fast, no failure budget), R2
- Source: align-L1 F6, align-L2 L2-06
- Also in: PRD-00-index

## L2-02 (major)

- Section and lines: PRD-02 section 4 masker panic (233-235); P13 row (577)
- Change: Lines 234-235: 'and the worker writes a GAP with reason `mask_failed` (12) and the piece's length' becomes 'and the worker writes one GAP with reason `mask_failed` (12) whose `off` is the first byte the old masker had not yet emitted, and whose `lost` is the held bytes plus the piece's length, plus one. Its start is the time of the earliest held byte, its end the panic.' Add to `pipeline_mask_panic.rs` (717-719): a panic while bytes are held leaves DATA plus GAP exactly contiguous (invariant 4).
- Ledger item or rule: R4 (locked: no silent loss)
- Source: align-L2 L2-02

## L2-03 (major)

- Section and lines: PRD-02 section 5 steps 1 and 5 (247, 263), section 6 `mode` (305-306), hard guard (188, 211-214); PRD-06 section 6 (258-261, 276-282)
- Change: PRD-02 section 5: move the status tick out of the writer loop: '5. Every second, a status thread (not the writer) writes `status.json` from the live atomics: `written`, `losing`, `held_bytes`, `writer_idle_ms`.' Add to section 3 or 8: 'Writer stall: when records wait and the writer has written nothing for 60 s on boot time, the recorder exits 75 and the watchdog restarts it, so the stall becomes a `recorder_down` gap with cause `writer_stalled`.' PRD-06 section 6 `recording` table: compute `mode` from `Durable` age and the live atomics, not only from `status.json`, and send `failed` when `writer_idle_ms` passes 60 s.
- Ledger item or rule: R3, R4, R7 (locked)
- Source: align-L2 L2-03
- Also in: PRD-06-uploader-compression

## L2-04 (major)

- Section and lines: PRD-02 section 5 step 3 (255-257), section 6 entry totals (281-283), builder note (849-851), sections 9.2 and 9.4; PRD-01 section 9 (391-392); PRD-08 `counts` row (173)
- Change: PRD-02 section 5 step 4: 'At each durable point, also append the counters at that LINK with its `seq` to `counts.jsonl`.' Sections 9.2 step 4 and 9.4 step 2: 'The recovery frame holds, before its LINK, a COUNT event with `boot` = the dead boot, taken from the last `counts.jsonl` line whose `seq` is at or below the recovered LINK.' PRD-02 section 6: `status.json` `written` shows counts at the durable point, so it never goes down. PRD-01:391: 'each boot's last COUNT' becomes 'each boot's last COUNT, which for a dead boot is the COUNT in its recovery frame', and state that this COUNT recounts records up to that line's LINK (invariant 7). PRD-08:173 `counts` row: add 'for a dead boot, the COUNT in its recovery frame (PRD-01 section 9)'.
- Ledger item or rule: R6 (locked: counts are written counts)
- Source: align-L2 L2-04
- Also in: PRD-01-record-format, PRD-08-verifier-checks

## L2-05 (major)

- Section and lines: PRD-02 section 8 step 9 (365-371), section 11 triggers (520-525)
- Change: Add step 9a: 'A `seal` line, a deadline or a server `stop` while in null mode: the watchdog stops the null recorder and starts one normal boot only to seal. Its recovery writes the `recorder_down` GAP with cause `null_mode` up to now, then the seal. If that boot also fails, the entry is left for `close_dead_entry` (seal reason `recovered`), and the null recorder's last heartbeat says `failed`.' Add a `watchdog_null.rs` case: harness exits in null mode; the entry ends with one `null_mode` gap and a seal.
- Ledger item or rule: R3, R4, R7 (locked)
- Source: align-L2 L2-05

## L3-4 (major)

- Section and lines: PRD-04 section 8 step 5 (336-343) and overridable defaults row 'Finish time guard'; PRD-02 section 11 step 4 (537-538)
- Change: PRD-04:336: '5. **Time guard: 180 s** (today's finish budget, `control.rs:708`).' becomes '5. **Time guard: 180 s, only when the seal reason is `deadline`, `server_stop` or `signal`** (today's finish budget, `control.rs:708`). On `harness_exit` and `seal_command`, finish reads every file to its end with no time guard; status shows `sealing: reading session files` meanwhile.' Same change in PRD-02 section 11 step 4 ('within PRD-04's 180 s guard' becomes 'within PRD-04's time guard where it applies') and in PRD-04's overridable defaults row.
- Ledger item or rule: H8 (locked: session files read with no caps); R5
- Source: align-L3 L3-4
- Also in: PRD-04-native-capture

## F5/L2-19 (minor)

- Section and lines: PRD-02 'Owner decisions' (34-35) and section 8 first paragraph (328); PRD-00 'Owner questions' (686 on)
- Change: PRD-02:35: 'A tiny watchdog that only restarts the recorder is allowed.' becomes 'A tiny watchdog is allowed (R3); in 1.0 it is the `goalcup start` or `continue` process, with the jobs in section 8 (PRD-00 owner question 9).' PRD-02 section 8 first paragraph, add: 'Its only recording duty is the restart loop. Everything else it does is what `goalcup start` already did (spawn, terminal, deadline, run CA, helpers); none of it records or touches recorded bytes.' PRD-00 owner questions, add 9: 'The watchdog is the start process itself. Beside restarting the recorder it holds the listen socket and the run CA, runs helper stops and starts and ends the harness, because those must survive a recorder crash. Default: yes.' (Owner list OW-05.)
- Ledger item or rule: R3, L5 (locked wording 'only restarts the recorder')
- Source: align-L1 F5, align-L2 L2-19
- Also in: PRD-00-index

## F7 (minor)

- Section and lines: PRD-02 'Owner decisions' (24-27)
- Change: '(ledger L2)' becomes '(ledger L5)'.
- Ledger item or rule: L5 (traceability)
- Source: align-L1 F7

## L2-07 (minor)

- Section and lines: PRD-02 section 3 hard guard (188, 211-217), defaults (787)
- Change: Line 188 'What passing it does': 'The recorder exits with code 75. This is a recorder failure under R3, not an R5 guard: past the soft guard only control items fill memory, so reaching 2 x soft means the writer is stuck.' Merge with the writer-stall rule of L2-03 so there is one stall exit, not two.
- Ledger item or rule: R5, R2
- Source: align-L2 L2-07

## L2-11 (minor)

- Section and lines: PRD-02 section 11 step 5 (540-542)
- Change: Line 541, after 'on `sid 0`': 'with start = the stop time (step 2), end = the seal time, and `lost` = the bytes counted after the stop plus one'.
- Ledger item or rule: R4 (every gap has a start and an end)
- Source: align-L2 L2-11

## L2-12 (minor)

- Section and lines: PRD-02 section 11 step 6 (546-547); PRD-10 end screen and `goalcup status`
- Change: PRD-02:547: 'their count goes to `goalcup.log`' becomes 'their count and hosts go to `goalcup.log` and to `upload.json` (`after_seal_bytes`), and PRD-10's end screen and `goalcup status` name them in the `after_stop` line'. PRD-10: the `after_stop` line also shows `after_seal_bytes` when non-zero.
- Ledger item or rule: R4, point 6
- Source: align-L2 L2-12
- Also in: PRD-10-cli-participant

## L2-13 (minor)

- Section and lines: PRD-02 section 9.1 table (408-413), risks (762-764)
- Change: Add a row to the 9.1 table: 'The watchdog, then later the recorder: harness calls refused; nothing sends a heartbeat; `goalcup status` says off (PRD-10 both-dead line); `continue` closes it `recovered`.' (The PRD-07 `fetch.json` heartbeat-age addition is not a fix; see owner list OW-11.)
- Ledger item or rule: R3, R7
- Source: align-L2 L2-13

## L2-17 (minor)

- Section and lines: PRD-02 section 6 (276-278); PRD-01 section 9 (379-383)
- Change: PRD-02:277: '`http_post` counts `http_req` streams with `POST` and `DECRYPTED` whose END is written' (a request counts once its body is in the record; `req_holes` still says which have holes). PRD-01 section 9: state the same counting point.
- Ledger item or rule: R6 (locked: a call counts only once its bytes are in the record)
- Source: align-L2 L2-17
- Also in: PRD-01-record-format
