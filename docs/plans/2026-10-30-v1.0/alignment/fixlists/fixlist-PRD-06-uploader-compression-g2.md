# Fix list for PRD-06-uploader-compression.md (triage group 2)

Source reviews: align-X1-run1, align-X2-scope, align-X3-builder, align-PRD-00 to align-PRD-06.
The ledger (OWNER-DECISIONS.md) wins. An id shared by several fix lists carries the same change text in each;
make in this file only the part that names this file. Items are ordered by severity, then id.

Counts: 1 blocker, 8 major, 16 minor, 25 in all.

## G2-20 (blocker)
- Sources: P01-1, P02-12
- Section and lines here: Line 105
- Same id also in: PRD-01-record-format, PRD-02-recorder-core
- Ledger item or rule: Ledger F2 (locked: zstd `--long=27`); B2a (a locked change needs the owner)
- Change:
  PRD-01 row 864 becomes two rows: "zstd level | 3 | PRD-06, with a measured reason" and "Window 2^27 with long matching | locked (ledger F2) | the owner only". Lines 836-837: "The 141 MiB compressor sits in the participant's process for the whole run. The window stays 2^27 (ledger F2, locked); a smaller window needs the owner." `PartOptions.window_log` stays a parameter for tests only (line 486).
  PRD-06 line 105: drop "PRD-01 allows window 26".
  PRD-02 R8 and risks line 748: "... or PRD-01 lowers the zstd level only; `window_log` 27 and long matching stay (ledger F2)."

## 06-1 (major)
- Sources: 06-1
- Section and lines here: Design 1 'One queue of entries' (86-89); section 5 rows `refused`, `stuck` (233-234); section 11 `state` list
- Ledger item or rule: Point 5, ledger U1
- Change:
  Replace "oldest entry first, the live entry last." with: "Each round serves the live entry first, then one round for each older entry, oldest first. An entry that is `refused`, `stuck` or waiting on backoff never holds the queue: the thread moves on and comes back when its `next_try` is due. An entry whose `upload_by` has passed is dropped from the queue and its `upload.json` says `closed`. The upload thread (not the beat thread) sends the final heartbeat of every non-live entry." Add `closed` to the `state` list in section 11.

## 06-3 (major)
- Sources: 06-3
- Section and lines here: Acceptance U3 (518); section 5; 'How a watcher proves it' (648-652); interfaces table
- Ledger item or rule: Acceptance not provable from a real run
- Change:
  U3 (a): "`upload.json` shows `waiting` with class `network` within 1 minute of the first round or heartbeat due after the cut began (at most 6 minutes after it)." Section 5, add: "Every failed call writes one `goalcup.log` line through `cli::log::line` (S11): `upload: <class> <status or -> wait <s>s` (no URL, header or body)." U3 (b): "each wait is at most 2^n s for the n-th failure in a row, and none above 120 s", read from those lines. Add S11 to the interfaces table.

## 06-4 (major)
- Sources: 06-4
- Section and lines here: Acceptance U2 (517)
- Same id also in: PRD-16-orchestration-acceptance
- Ledger item or rule: Acceptance not provable
- Change:
  U2 and PRD-16 X5: "At every minute t of the watcher's log, `entries.upload_offset` is at or past the `durable_raw` the log shows at t minus 6 minutes, except while `upload.json` shows a network or server error. No byte-size line."

## 06-5 (major)
- Sources: 06-5
- Section and lines here: Design 1 (84-85); interfaces PRD-02 row (474); 'Traps' (631-646)
- Ledger item or rule: Points 1 and 3 (a held borrow stalls the writer)
- Change:
  "They copy `Durable` out with `*rx.borrow()` (or `borrow().clone()`) on their 1 s tick and drop the borrow at once; no `Ref` is held across a read, a hash or a call." Same line in "Traps".

## 06-6 (major)
- Sources: 06-6
- Section and lines here: Automated tests (531-558); Files it owns; 'What is cut' (496)
- Ledger item or rule: Point 5; PRD-14 interface (`upload_worker.rs` is PRD-06's)
- Change:
  Add to "Automated tests": "`upload_worker.rs` (full tier, ignored in fast): the real client against the real Worker under `wrangler dev` with its local database; a sealed entry with two parts; `fetch-run.sh --local`; `rec.gcr.zst` equals `cat rec-*.gcr.zst`; the entry is `sealed`." Add it to "Files it owns" and drop it from "What is cut".

## G2-19 (major)
- Sources: P00-7, P02-13, 06-21
- Section and lines here: `upload_backoff.rs` (538-540); interfaces PRD-14 row (482)
- Same id also in: PRD-00-index, PRD-02-recorder-core
- Ledger item or rule: Ledger L6 (locked: legacy and test seams cut), L2; also owner item O-3
- Change:
  PRD-00 item 86 and PRD-02 Files it owns: "Pending the owner (O-3): ledger L6 cut the test seams. The plan asks to keep one compile-time `test-hooks` feature in `crates/goalcup` only, off by default, never in a release build (PRD-14 greps the shipped binary for `GOALCUP_TEST_`), because restart, guard and rotation cannot be proven from a real run in the gate's time. No `GOALCUP_TEST_*` variable is read by a release build. `goalcup-verify` gets no hooks unless PRD-08 names one it needs." Add it to PRD-00's owner questions with this as the default.
  PRD-06 `upload_backoff.rs`: "`policy.rs` takes `now` as an argument; this case is a unit test of that pure function, with no new `GOALCUP_TEST_*` hook."

## G2-31 (major)
- Sources: X1-4, X1-11, 06-10 (and L2-03, L2-07, L2-08 of group 1)
- Section and lines here: Section 6 `recording` table (258-261, 276-282)
- Same id also in: PRD-02-recorder-core, PRD-07-worker-server
- Ledger item or rule: Ledger R3, R4, R7 (heartbeat says `failed` when recording is off)
- Change:
  PRD-02 section 5 step 5: "Every second, a status thread (not the writer) writes `status.json` from the live atomics: `written`, `losing`, `held_bytes`, `writer_idle_ms`." Section 3: "Writer stall: when records wait and the writer has written nothing for 60 s of boot time, or when control items alone pass 2 x soft, the recorder exits 75 and the watchdog restarts it, so the stall becomes a `recorder_down` gap with cause `writer_stalled`. This is a recorder failure under R3, not an R5 guard; it is the one stall exit."
  PRD-06 section 6: compute `mode` from `Durable` age and the live atomics, not only `status.json`. `ok` row: remove "or `writer_slow`". `failed` row: "`mode` null, or `writer_idle_ms` over 60 s while records wait".
  PRD-07 line 197: replace "`mode` null (null mode), or the entry ended while off" with "recording is off: null mode, or a writer stalled past 60 s; a recovered entry is `gaps` (PRD-06 section 6)". Add `sealed` to its `mode` list (the final heartbeat sends `mode: sealed`).

## G2-32 (major)
- Sources: X1-6, 06-2
- Section and lines here: Design 1 'Two plain threads' (81-85); `upload_resume.rs`
- Same id also in: PRD-02-recorder-core
- Ledger item or rule: Point 3 ('note a gap'), ledger R2, R3, R4
- Change:
  PRD-02 9.2 step 1: "A panic in the writer, the mask worker or the native thread exits 70. The uploader catches its own panics and never ends the process (PRD-06 section 1)." Section 2 rule 1: add "An S1 send whose receiver is gone exits the process with 70 (once, from the first caller), so the watchdog restarts it and a `recorder_down` gap covers the span." Test list: add "`pipeline_panic.rs`: kill the mask worker loop with a test hook; the recorder exits 70, the next boot writes `recorder_down`, and relays never hang".
  PRD-06 section 1: "Each upload round and each beat runs inside `catch_unwind`. A panic there writes one fixed-text line through `cli::log::line` (S11), sets `upload.json` `state` `waiting` with class `internal`, backs off by section 5 and never ends the process. Recording never depends on the uploader." `upload_resume.rs`: a scripted answer that panics the parser leaves the recorder running.

## 06-11 (minor)
- Sources: 06-11
- Section and lines here: Section 6 final heartbeat (295-305); section 11 `final` (416-417); interfaces PRD-07 row (476)
- Ledger item or rule: Agreement with S5 (PRD-00 R3-6, PRD-07 section 2)
- Change:
  Interfaces row: answer `{offset, next_seq, head, received_at, sealed}`. Lines 298-300: "After the last chunk is stored, if its answer did not say `sealed: true`, it sends the final heartbeat again ..." Section 11: "`sealed` (a heartbeat, chunk or position answer said `sealed: true`)".

## 06-13 (minor)
- Sources: 06-13
- Section and lines here: Acceptance U6 (521)
- Ledger item or rule: Builder misread of the check
- Change:
  "the final row's `vitals -> 'counts' ->> 'off_ms'`".

## 06-14 (minor)
- Sources: 06-14
- Section and lines here: Acceptance U1 (516)
- Ledger item or rule: Acceptance would fail on correct code under PRD-01's ruled fallback
- Change:
  Add to U1: "If PRD-01's recovery fallback is in force, decode part by part instead, using `fetch.json` `bytes` to split `rec.gcr.zst`; each part decodes, and only parts closed by a crash may end inside a frame at an index line."

## 06-15 (minor)
- Sources: 06-15
- Section and lines here: Section 9 tail window (357-361); section 11 `upload_by`; `upload_beat.rs`
- Ledger item or rule: Point 6 (told plainly and truly)
- Change:
  Add: "When a heartbeat answer says `stop` with reason `replaced`, `upload_by` becomes the answer's time plus 24 hours." Add a case to `upload_beat.rs`.

## 06-16 (minor)
- Sources: 06-16
- Section and lines here: Acceptance U4 (519)
- Ledger item or rule: Acceptance not provable (no Worker log access)
- Change:
  "`fetch.json` checks pass with no hole; the `goalcup.log` lines of 06-3 show at most one resend of the chunk that was in flight at the kill."

## 06-17 (minor)
- Sources: 06-17
- Section and lines here: Acceptance U5 (520); Risks 'The cut' (583-585); review answer B W5 (709)
- Same id also in: PRD-12-test-pad
- Ledger item or rule: Builder misread across PRD-06 and PRD-12
- Change:
  PRD-06 U5: "with the cut from minute 1 for 10 minutes (PRD-12 `cut-worker@1+10`)". Risks: "If `getent` shows the cut did not take, PRD-12's stand-in is used (a local listener that refuses or holds the connection; it never terminates TLS); else the row says 'not run: cut did not take'." Delete "so no forwarder" from B W5. PRD-12: the stand-in never terminates TLS.

## 06-18 (minor)
- Sources: 06-18
- Section and lines here: Section 9 first bullet (352-356)
- Ledger item or rule: Point 6
- Change:
  "Ctrl-C ends the watchdog's wait only; the recorder finishes the wrap-up alone (PRD-02 section 8 step 11). Whatever is left after 60 minutes waits on disk."

## 06-19 (minor)
- Sources: 06-19
- Section and lines here: Section 7 `keep` (331-332); defaults (607)
- Ledger item or rule: Builds what was not agreed (PRD-10 says no flag)
- Change:
  Delete "and may also offer a flag"; the defaults row's owner becomes "Owner".

## 06-20 (minor)
- Sources: 06-20
- Section and lines here: Acceptance U7 (522)
- Ledger item or rule: Acceptance not provable on its named run
- Change:
  "U7a (U2's run): RSS minus `held_bytes` under 260 MiB. U7b (A1, PRD-16 X9): no step of about 140 MiB at the 60-minute rotation."

## 06-22 (minor)
- Sources: 06-22
- Section and lines here: Today table `api.rs` row (440); section 12
- Ledger item or rule: Code fact
- Change:
  Change `api.rs:310-341` to `api.rs:18-61` (tests at `:310-341`).

## 06-7 (minor)
- Sources: 06-7
- Section and lines here: Section 5 (216-222); `upload_backoff.rs` (538-540)
- Ledger item or rule: Builder misread
- Change:
  Line 222: "So the nominal waits are 2, 4, 8 ... 120 s, each times a random 0.5 to 1.0, and never more than 120 s." U3 as 06-3.

## 06-8 (minor)
- Sources: 06-8
- Section and lines here: Section 5 table, 409 row (232); `upload_resume.rs`
- Ledger item or rule: Point 3 (CPU and network on the participant's machine)
- Change:
  Add: "A 409 whose re-read shows no forward move counts as a failure for the backoff." Add a case to `upload_resume.rs`.

## 06-9 (minor)
- Sources: 06-9
- Section and lines here: Section 6 (258); Traps (642-643)
- Ledger item or rule: Agreement with S5 (PRD-07 drops a bad `counts`, never a 400)
- Change:
  Line 258: "(PRD-07 drops anything else and records the heartbeat without it, so the loss is silent; `upload_beat.rs` guards the size)". Line 642: "or the server drops every `counts` silently."

## G2-12 (minor)
- Sources: X3-17, P00-14, P05-9, P02-16, 06-12
- Section and lines here: Lines 35, 38, 361, 569, 573, 598, 608, 660, 663, 684-685
- Same id also in: PRD-16-orchestration-acceptance, PRD-05-masking, PRD-02-recorder-core, PRD-00-index
- Ledger item or rule: Clarity; ledger is the source of truth
- Change:
  Replace revision 2 owner question numbers with PRD-00's current eight. Map: old 1, 2, 4, 6, 7 are question 1; old 8, 9, 10, 11, 13 are question 2; old 12 is 3; old 3, 14, 15 are 4; old 5 is 5; old 16 is 6; old 19 is 7; old 17 and 18 are 8. Write "owner question N (old M)" where a reader may meet the old number.
  PRD-00 line 681: "owner question 2". PRD-00 review answers: write each as "old N (now M)".
  PRD-02: "owner question 2" (line 37); line 120: "This is PRD-00's S1 block (revision 3, rulings R3-1 and R3-2)"; header: "Index: PRD-00 revision 3".
  PRD-06: every "owner question 6" and "owner question 2" becomes "PRD-00 owner question 1"; in "Owner decisions it carries out" tag each line with its ledger id: F2 (zstd), U1 (prune, small chunks, capped backoff that resets on progress), R5 (no caps), R6 and R7 (written counts, `failed`), L5 (one process), B4 (staging).

## G2-15 (minor)
- Sources: X3-28, P02-2 (part)
- Section and lines here: Files (453); Interfaces (474)
- Same id also in: PRD-00-index, PRD-02-recorder-core
- Ledger item or rule: Seam named but not defined; ruling R3-2
- Change:
  PRD-00 Seams, add "S18 `upload::spawn(ctx) -> UploadHandle`, `UploadHandle::finish(wrap_up, seal_facts { reason, workspace_head, totals })`, owner PRD-06, used by PRD-02, stable by H+3". R3-2 line 863 cites S18.
  PRD-02 section 11 step 6 and 9.2 step 5: "hand over with `UploadHandle::finish(wrap_up, seal_facts)`, `seal_facts` holding `reason`, `workspace_head` and `totals`".
  PRD-06 Files and Interfaces: the same signature.

## X2-11 (minor)
- Sources: X2-11
- Section and lines here: 'Review answers' section and fully ruled 'Asks' sections
- Same id also in: PRD-00-index, PRD-01-record-format, PRD-02-recorder-core, PRD-03-proxy, PRD-04-native-capture, PRD-05-masking, PRD-07-worker-server, PRD-08-verifier-checks, PRD-09-usage-research-stats, PRD-10-cli-participant, PRD-11-connectors-harnesses, PRD-12-test-pad, PRD-13-cuts, PRD-14-tests-gate, PRD-15-docs-decisions, PRD-16-orchestration-acceptance, PRD-17-lessons-old-code
- Ledger item or rule: Ledger L2; point 6 (plain, true text). Apply last, after every other fix, so line numbers in the fix lists stay valid
- Change:
  Move every "Review answers" section and every fully ruled "Asks" section into one file `docs/plans/2026-10-30-v1.0/REVIEW-HISTORY.md`, verbatim, one heading per PRD. Before moving, copy any ruling still binding into the PRD body. Each PRD keeps one line at its end: "Review history: REVIEW-HISTORY.md, section PRD-NN." The replan keeps new asks in the Amendments note only.
