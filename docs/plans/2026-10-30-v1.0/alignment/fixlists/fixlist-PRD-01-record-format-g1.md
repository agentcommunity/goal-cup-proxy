# Fix list for PRD-01-record-format.md (group 1)

From the alignment reviews L1 to L7, triaged against OWNER-DECISIONS.md (the ledger wins).
Files are under docs/plans/2026-10-30-v1.0/. Line numbers are from branch smart/intelligent-newton-0avrpi.
An item with several files carries the same id and the same change text in each file's list; make only the part for this file.

Counts: 0 blocker, 3 major, 3 minor.

## L2-04 (major)

- Section and lines: PRD-02 section 5 step 3 (255-257), section 6 entry totals (281-283), builder note (849-851), sections 9.2 and 9.4; PRD-01 section 9 (391-392); PRD-08 `counts` row (173)
- Change: PRD-02 section 5 step 4: 'At each durable point, also append the counters at that LINK with its `seq` to `counts.jsonl`.' Sections 9.2 step 4 and 9.4 step 2: 'The recovery frame holds, before its LINK, a COUNT event with `boot` = the dead boot, taken from the last `counts.jsonl` line whose `seq` is at or below the recovered LINK.' PRD-02 section 6: `status.json` `written` shows counts at the durable point, so it never goes down. PRD-01:391: 'each boot's last COUNT' becomes 'each boot's last COUNT, which for a dead boot is the COUNT in its recovery frame', and state that this COUNT recounts records up to that line's LINK (invariant 7). PRD-08:173 `counts` row: add 'for a dead boot, the COUNT in its recovery frame (PRD-01 section 9)'.
- Ledger item or rule: R6 (locked: counts are written counts)
- Source: align-L2 L2-04
- Also in: PRD-02-recorder-core, PRD-08-verifier-checks

## L4-2 (major)

- Section and lines: PRD-01 risks (836-837), overridable defaults row 'zstd' (864); PRD-06 section 2 (104-106, table row 114), overridable defaults row 'zstd' (605)
- Change: PRD-01:837: 'On a small machine PRD-06 may drop to `window_log` 26 (73 MiB); the format allows any window up to 27.' becomes 'Window 27 is locked (ledger F2); a smaller window needs the owner.' PRD-01:864 and PRD-06:605: split into 'level: 3, PRD-06 with a measured reason (level 1 first)' and 'window 27 and long matching: locked (ledger F2), owner only'. PRD-06:105: 'PRD-01 allows window 26; we do not use it until a measured machine needs it.' becomes 'Window 27 is locked (ledger F2).' Drop the 73 MiB row at 114 or label it 'not allowed without the owner'.
- Ledger item or rule: F2 (locked: zstd `--long=27`)
- Source: align-L4 L4-2
- Also in: PRD-06-uploader-compression

## L4-5 (major)

- Section and lines: PRD-01 `http_req` meta (232, 353); PRD-05 section 7 step 4 (317-318), what is cut row (516); PRD-03 request side (286)
- Change: Keep today's query drop (less code, no change to a secret protection). PRD-01:232: '`path` (query kept, credential values masked)' becomes '`path` (query dropped, as in 0.7)', and the same at 353. PRD-05 step 4: 'The query is kept, masked (PRD-01 `http_req` meta), so today's query drop (`metadata.rs:81-106`) goes.' becomes 'The query is dropped, as today (`metadata.rs:81-106`).' Delete row 516's 'Cut'. PRD-03:286: 'path with query' becomes 'path without query'.
- Ledger item or rule: M1 (locked: keep every secret rule, nothing loosened); point 4
- Source: align-L4 L4-5
- Also in: PRD-05-masking, PRD-03-proxy

## L2-10 (minor)

- Section and lines: PRD-04 section 9 'Same device only' (359-361); PRD-01 note list
- Change: PRD-04:361, add: 'The first time a folder is skipped this way, write one `note` `native_other_device` with the row name and the folder path below the root, so status and the end screen name it.' Add `native_other_device` to PRD-01's note list.
- Ledger item or rule: R4, R1, point 2
- Source: align-L2 L2-10
- Also in: PRD-04-native-capture

## L2-16 (minor)

- Section and lines: PRD-03 section 2 ClientHello (193), section 9 listed-host GAP list (394-397), section 11 (442); PRD-01 `conn` reasons (231)
- Change: PRD-03:193: 'Guard 64 KiB (`client_hello`); past it, a tunnel with `reason: memory_guard`, and when the CONNECT host is listed, a GAP `memory_guard` (`client_hello`) at close with `lost` = `up` + `down` + 1.' Add `memory_guard` to PRD-01:231's tunnel reasons and to PRD-03:394-397.
- Ledger item or rule: R4
- Source: align-L2 L2-16
- Also in: PRD-03-proxy

## L2-17 (minor)

- Section and lines: PRD-02 section 6 (276-278); PRD-01 section 9 (379-383)
- Change: PRD-02:277: '`http_post` counts `http_req` streams with `POST` and `DECRYPTED` whose END is written' (a request counts once its body is in the record; `req_holes` still says which have holes). PRD-01 section 9: state the same counting point.
- Ledger item or rule: R6 (locked: a call counts only once its bytes are in the record)
- Source: align-L2 L2-17
- Also in: PRD-02-recorder-core
