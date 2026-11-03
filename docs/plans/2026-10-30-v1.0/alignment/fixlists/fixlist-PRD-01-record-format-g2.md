# Fix list for PRD-01-record-format.md (triage group 2)

Source reviews: align-X1-run1, align-X2-scope, align-X3-builder, align-PRD-00 to align-PRD-06.
The ledger (OWNER-DECISIONS.md) wins. An id shared by several fix lists carries the same change text in each;
make in this file only the part that names this file. Items are ordered by severity, then id.

Counts: 1 blocker, 11 major, 16 minor, 28 in all.

## G2-20 (blocker)
- Sources: P01-1, P02-12
- Section and lines here: Overridable defaults (864); Risks (836-837); line 486
- Same id also in: PRD-06-uploader-compression, PRD-02-recorder-core
- Ledger item or rule: Ledger F2 (locked: zstd `--long=27`); B2a (a locked change needs the owner)
- Change:
  PRD-01 row 864 becomes two rows: "zstd level | 3 | PRD-06, with a measured reason" and "Window 2^27 with long matching | locked (ledger F2) | the owner only". Lines 836-837: "The 141 MiB compressor sits in the participant's process for the whole run. The window stays 2^27 (ledger F2, locked); a smaller window needs the owner." `PartOptions.window_log` stays a parameter for tests only (line 486).
  PRD-06 line 105: drop "PRD-01 allows window 26".
  PRD-02 R8 and risks line 748: "... or PRD-01 lowers the zstd level only; `window_log` 27 and long matching stay (ledger F2)."

## G2-05 (major)
- Sources: X3-5, P01-18
- Section and lines here: Files it owns, 'Must not touch' (748-768)
- Same id also in: PRD-16-orchestration-acceptance, PRD-00-index
- Ledger item or rule: PRD-00 merge rule (586-597) deadlocks otherwise; file ownership
- Change:
  PRD-16 section 2 step 5, replace the `members` bullet (PRD-00 phase 0 step 3 the same): "Add each new crate to `members` in the merge that creates it (PRD-01 adds `goalcup-format`, PRD-02's scaffold adds `goalcup`, PRD-08 adds `goalcup-verify`); the integrator applies that one `Cargo.toml` line inside the merge. The five old crates move to `exclude` in this commit."
  PRD-01 Must not touch, add: "the root `Cargo.toml` and `Cargo.lock` (the integrator adds `crates/goalcup-format` to `members` inside this PRD's first merge, PRD-16 section 2 step 5)".

## G2-09 (major)
- Sources: X3-9, P01-4
- Section and lines here: Section 7 (329-333); acceptance 3 (808-810)
- Same id also in: PRD-02-recorder-core
- Ledger item or rule: Ledger R3, point 3; PRD-00 done item 2 (2 s bound)
- Change:
  PRD-01 line 330 and PRD-02 line 410: replace "the last durable LINK" with "the last LINK named by an index line (the point recovery truncates to, PRD-01 section 12 step 1; after a plain process kill at most about 1 s before the kill)".
  PRD-01 acceptance 3: "During a Codex pad run (`full` prompt), `kill -9` the recorder twice, 2 minutes apart, while a response streams (no test hooks). Pass: the harness finishes its turn with no participant action; the entry decodes; exactly two `recorder_down` gaps, start before end, each under 2 s long; links verify across three boots; the server copy equals local."

## G2-21 (major)
- Sources: P02-1, P01-3
- Section and lines here: Section 9 (391-395); section 11 (446-447); section 12 step 3 (537-538); section 15 item 7 (634); invariant 7 (568)
- Same id also in: PRD-02-recorder-core
- Ledger item or rule: Ledger R6 (written counts), point 6; PRD-00 done item 1 (counts equal the recount); R3-2
- Change:
  PRD-02 section 5 step 3: "At every LINK, before its index line, append this boot's counters at that LINK to `counts.jsonl` (one compact line). At each durable point rewrite the file by atomic rename to one line per earlier boot plus this boot's latest." 9.2 step 4: "The recovery frame holds, before the `recorder_down` GAP, one COUNT for the dead boot taken from the `counts.jsonl` line whose `seq` equals the recovered last LINK, so that boot's last COUNT recounts every record it kept." Section 6: "`written` shows the counters at the last LINK (at most 1 s behind), so it never goes down across a crash."
  PRD-01 section 9, add: "A record belongs to the boot of the nearest PART before it, so a recovery frame belongs to the dead boot. Its COUNT (PRD-02 9.2 step 4) recounts that boot up to the last kept LINK. Entry totals = each boot's last COUNT, plus the `recorder_down` GAP of each recovery frame (`gaps`, `lost_unknown`, `off_ms`). The recovered seal carries the totals in `totals`." Section 12 step 3: the recovery frame writes that COUNT for a normal restart too, as it already does for a dead entry. Section 15 item 7: "Every COUNT is an exact recount of its boot up to that COUNT. Seal totals are the sum above; PRD-08 recounts with the same rule (`counts_match_seal`)." Invariant 7 reads per boot, the recovery frame's COUNT included.

## G2-26 (major)
- Sources: P01-8, P03-10
- Section and lines here: Section 3 after the GAP table (205-207); section 4 rules (241-244)
- Same id also in: PRD-03-proxy
- Ledger item or rule: Ledger R4 (lost bytes where known), point 6
- Change:
  PRD-01 section 3, after the GAP table: "`lost` is counted in the stream's offset space (decoded bytes for HTTP bodies, inflated bytes for WebSocket). When that size is unknown, as for `unsupported_encoding` and a body not decoded under `decoder_window`, `lost` is 0 and the tail gives `{"wire": <encoded bytes>}`."
  PRD-03 line 361: "`lost` 0, tail `{"guard":"decoder_window","limit":<room>,"wire":n}`".

## G2-27 (major)
- Sources: P01-6, P03-18
- Section and lines here: Note list (465); GAP table 'Written by' (214); COUNT `tunnel_hosts` (375, 384); interface row PRD-04 (776)
- Same id also in: PRD-03-proxy
- Ledger item or rule: PRD-00 ruling R3-4 and R3-5; builder misread
- Change:
  PRD-01 line 465 list: add `outlived`, `native_missing`, `parse_stopped`. After it: "`long_tunnel` facts: `host`, `why` (`stream` or `upload`), `conns`, `up`, `down`, `secs`. `parse_stopped`: `conn`, `reason`. `native_missing`: `row`. `outlived`: `count`, `programs`." Line 375: `"tunnel_hosts":{"github.com":{"conns":12,"up":40960,"down":2097152,"listed":null}}`; line 384: "a listed host's value adds `listed` (the GAP reason name)". Line 214 "Written by": "PRD-02, PRD-04". Line 776: add "GAP 10 `cut_at_seal`, note `native_missing`".
  PRD-03 asks 1 and 2: mark "ruled by PRD-00 R3-4 and R3-5; PRD-01 copies them (P01-6)".

## P01-2 (major)
- Sources: P01-2
- Section and lines here: Section 12 'Crash recovery' (524-546); invariant 1 (557); line 481
- Ledger item or rule: Ledger R3, R4; builder misread
- Change:
  Add after line 546: "**A boot that wrote no index line** (its part file holds no line, or no part file exists) leaves no trace: delete its part file; the next boot reuses its boot number (read from the last remaining PART, or 0 when none). The `recorder_down` GAP then covers every death since the last kept LINK: write it as the recovery frame of the last part file that has a line (step 3; a part file closed by an `R` line takes a second recovery frame); when the entry has no line at all, write it right after the new boot's `start` event, with start = the deleted boot's `start` time if the watchdog passed it, else the new boot's time, `lost` 0 and tail `{"cause":...,"exits":n}`." Invariant 1: "The stream starts with PART (part 0) and every part file starts with PART. The first PART has boot 0, or a higher boot only when every earlier boot died before its first LINK." Line 481: "plus at most one recovery frame" becomes "plus recovery frames".

## P01-5 (major)
- Sources: P01-5
- Section and lines here: Section 6 (316-317); section 3 GAP row (174); invariant 12 (576-577)
- Ledger item or rule: Ledger R4, R6 (off time), point 6
- Change:
  Section 6, add: "A GAP whose end was taken under a different `boot_id` than its start carries tail key `"boot_id"` (the new clock's id, 32 hex). For such a GAP, invariant 12 and off time use `start_wall` and `end_wall`." Invariant 12, add "(wall times when the GAP's tail names another `boot_id`)".

## P01-7 (major)
- Sources: P01-7
- Section and lines here: Section 3 MASK row (175); section 6 (296-299); worked example or `roundtrip.rs`
- Ledger item or rule: Point 5 (story by time); builder misread (PRD-15 writes an independent reader)
- Change:
  Line 296: "Every record but MASK has a time." Add: "MASK carries no time and does not move the running time: the next record's `dt` is from the record before the MASK." Add one MASK record to the worked example or to `roundtrip.rs`, with a check on the following record's time.

## P03-4 (major)
- Sources: P03-4
- Section and lines here: Section 4 tunnel reasons and listed-tunnel GAP list
- Same id also in: PRD-03-proxy, PRD-08-verifier-checks
- Ledger item or rule: Ledger R4 (named loss), point 6
- Change:
  PRD-03 section 2, after "Guard 64 KiB (`client_hello`); past it, a tunnel" add: "The CONNECT host decides. The `conn` gets the new tunnel reason `hello_too_big`; on a listed host its close GAP is `memory_guard` with `{"guard":"client_hello","limit":65536}`, `off` 0, `lost` = `up` + `down` + 1."
  PRD-01: add `hello_too_big` to the tunnel reasons and GAP 1 to the listed-tunnel GAP list. PRD-08: count it as `named`.

## P03-5 (major)
- Sources: P03-5
- Section and lines here: `conn` `reason` list
- Same id also in: PRD-03-proxy, PRD-00-index
- Ledger item or rule: Ledger H6; builder misread (a decrypted connection cannot fall back to a tunnel)
- Change:
  PRD-03 section 9: remove `h2_unsupported` from the tunnel reason list and "11 `h2_unsupported`" from the listed-tunnel GAP list. Section 6, add: "`h2_unsupported` is never a tunnel reason: the connection is already decrypted, so it keeps relaying unparsed (5a)."
  PRD-00 third slice: "anything else is GAP `h2_unsupported` and the connection relays unparsed (PRD-03 5a)". PRD-01: drop `h2_unsupported` from the `conn` `reason` list.

## P04-5 (major)
- Sources: P04-5
- Section and lines here: Note list
- Same id also in: PRD-04-native-capture, PRD-10-cli-participant
- Ledger item or rule: Ledger R4 (no silent loss), point 6
- Change:
  PRD-04 section 9 bullet, append: "Each such folder gets one `note` `native_other_device` (`row`, the path relative to the root) per entry, so the miss is named." `native_walk.rs`: "a folder whose `st_dev` differs (an existing mount under the test root, or skipped with a printed reason when none exists) is not entered and gives one note". Builder notes: use `find <root> -xdev`.
  PRD-01: add `native_other_device` to the note list. PRD-10: one plain line for it.

## G2-33 (minor)
- Sources: X1-9, P02-29
- Section and lines here: Section 9 (379-383)
- Same id also in: PRD-02-recorder-core, PRD-10-cli-participant
- Ledger item or rule: Ledger R6 ('a model call counts only once its bytes are in the record'), point 6
- Change:
  PRD-01 section 9, after the `req_whole` bullet: "The heartbeat, status and end screen show `req_whole` next to `model_requests` whenever `req_holes` is above 0." PRD-02 section 6: "Status shows `req_holes` and `req_whole` beside `model_requests`, so a request whose body is in a gap is never shown as whole." PRD-10 end screen rule 3: name `req_whole` as the "written whole" number.

## P01-11 (minor)
- Sources: P01-11
- Section and lines here: Section 11 `start` keys (458)
- Ledger item or rule: Builder misread
- Change:
  "`segment` (the entry's segment number from the server's run approval: 0 for `start`, then 1, 2 for each `continue`; PRD-07's `segment`)".

## P01-12 (minor)
- Sources: P01-12
- Section and lines here: Acceptance 1 (803)
- Ledger item or rule: Ledger T5 clarity
- Change:
  "A Claude Code direct-key pad run, `--prompt smoke` (PRD-12: one subagent, about 3 minutes), kept parts:".

## P01-13 (minor)
- Sources: P01-13
- Section and lines here: Section 1 'Durability' (113-116); section 12 step 1 (526-528)
- Ledger item or rule: Point 5 (server copy whole)
- Change:
  Add: "At each fsync the writer records the durable index length in `cursors.json` (atomic rename). When recovery runs under a new `boot_id`, it first drops index lines past that length."

## P01-14 (minor)
- Sources: P01-14
- Section and lines here: Section 4 rules (241-244)
- Ledger item or rule: Builder misread
- Change:
  "Extends the copied rule (`http.rs:18-26` undoes at most one coding): every coding in `content-encoding` is undone in order; any coding not in the list is a GAP `unsupported_encoding`."

## P01-15 (minor)
- Sources: P01-15
- Section and lines here: Section 11 (468); interface rows (783-784)
- Ledger item or rule: Ledger L6 clarity (pins are cut)
- Change:
  Replace "pin mark" and "pin marks" with "certificate-pinning flag (`pinned`)".

## P01-16 (minor)
- Sources: P01-16
- Section and lines here: Section 12 'Chunks' (505-508)
- Ledger item or rule: Seam with PRD-06 and PRD-07
- Change:
  Add: "Hashes in headers are 64 lowercase hex characters; `Goalcup-Prev` of an entry's first chunk is 64 zeros (`hash(-1)`, the database default `head`)."

## P01-17 (minor)
- Sources: P01-17
- Section and lines here: Section 9 (379-383)
- Ledger item or rule: Ledger R6; builder misread (the verifier recounts from this text)
- Change:
  Add: "`http_post` counts an `http_req` OPEN with method POST when that OPEN is written; `ws_up_msgs` counts a `ws_up` DATA with the end-of-message flag and a text or binary opcode when it is written."

## P01-19 (minor)
- Sources: P01-19
- Section and lines here: Section 8 (358-359); Risks (841)
- Same id also in: PRD-00-index
- Ledger item or rule: Ledger status words (default against locked)
- Change:
  PRD-01 line 359: "Accepted as a planner default (PRD-05); listed in PRD-00's defaults for the owner." Add "same-length masking (the length of a masked value stays visible)" to PRD-00's defaults list.

## P01-20 (minor)
- Sources: P01-20
- Section and lines here: 'Today' table (41)
- Ledger item or rule: Code fact accuracy
- Change:
  Cite `crates/goalcup-protocol/src/capture.rs:191-202` for the reasons and the `Gap` variant lines (about `:295-300`) for "one time, never an end".

## P01-9 (minor)
- Sources: P01-9
- Section and lines here: Section 6 (304-305); section 10 (409); table (63)
- Ledger item or rule: Builder misread
- Change:
  One sentence in section 10, cited from section 6 and the table: "A LINK is written 1 s after the last LINK if records were written since, sooner when 16 MiB raw wait, before rotation, at the seal, and after each COUNT (so at least once a minute)."

## P03-15 (minor)
- Sources: P03-15
- Section and lines here: Note list
- Same id also in: PRD-03-proxy
- Ledger item or rule: Point 5, R4
- Change:
  PRD-03: "Each non-`200` answer writes one `note` `connect_refused` `{host, status}` (host only when the head parsed), at most one per host and status per minute." PRD-01: add the note name.

## P03-9 (minor)
- Sources: P03-9
- Section and lines here: GAP 3 tail; note list
- Same id also in: PRD-03-proxy
- Ledger item or rule: Ledger R4, clarity
- Change:
  PRD-03 section 7: "GAP `unsupported_encoding` with tail `{"codings": [...]}` (each token at most 32 bytes, at most 4 tokens)." Remove `unsupported_coding` from the PRD-01 row. PRD-01: give GAP 3 the `codings` tail key.

## P05-11 (minor)
- Sources: P05-11
- Section and lines here: Section 8 (352)
- Same id also in: PRD-05-masking
- Ledger item or rule: Interface agreement
- Change:
  PRD-01 section 8: add "except a `fail_closed` span, which may cover quotes; such a body is not expected to parse". PRD-05 Open asks lists it.

## X1-7 (minor)
- Sources: X1-7
- Section and lines here: Section 14 'Window reach' (612-614)
- Same id also in: PRD-07-worker-server
- Ledger item or rule: Point 5, R5 (no cap, so plan the cost)
- Change:
  PRD-01 section 14, add a bullet: "**200 agents at once** (estimate, 10 times run 1's wire rate, ratio 3.2 to 1): about 8 GB raw and 2.5 GB compressed an hour, about 6 Mbit/s of upload, about 400 GB a week; a home uplink under that keeps the server hours behind, and parts wait on disk compressed." PRD-07 cost table: add the row "200 agents at once, a week: about 400 GB, about $6 a month".

## X2-11 (minor)
- Sources: X2-11
- Section and lines here: 'Review answers' section and fully ruled 'Asks' sections
- Same id also in: PRD-00-index, PRD-02-recorder-core, PRD-03-proxy, PRD-04-native-capture, PRD-05-masking, PRD-06-uploader-compression, PRD-07-worker-server, PRD-08-verifier-checks, PRD-09-usage-research-stats, PRD-10-cli-participant, PRD-11-connectors-harnesses, PRD-12-test-pad, PRD-13-cuts, PRD-14-tests-gate, PRD-15-docs-decisions, PRD-16-orchestration-acceptance, PRD-17-lessons-old-code
- Ledger item or rule: Ledger L2; point 6 (plain, true text). Apply last, after every other fix, so line numbers in the fix lists stay valid
- Change:
  Move every "Review answers" section and every fully ruled "Asks" section into one file `docs/plans/2026-10-30-v1.0/REVIEW-HISTORY.md`, verbatim, one heading per PRD. Before moving, copy any ruling still binding into the PRD body. Each PRD keeps one line at its end: "Review history: REVIEW-HISTORY.md, section PRD-NN." The replan keeps new asks in the Amendments note only.
