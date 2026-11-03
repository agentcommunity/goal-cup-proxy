# Fix list for PRD-02-recorder-core.md (triage group 2)

Source reviews: align-X1-run1, align-X2-scope, align-X3-builder, align-PRD-00 to align-PRD-06.
The ledger (OWNER-DECISIONS.md) wins. An id shared by several fix lists carries the same change text in each;
make in this file only the part that names this file. Items are ordered by severity, then id.

Counts: 3 blocker, 22 major, 19 minor, 44 in all.

## G2-01 (blocker)
- Sources: X3-1, P05-7
- Section and lines here: Builder notes step 2 (816-817)
- Same id also in: PRD-05-masking, PRD-16-orchestration-acceptance
- Ledger item or rule: Ledger M1 (masking before any byte reaches disk), point 4
- Change:
  PRD-02 builder note step 2: replace "that unlocks phase 1a" with "that unlocks phase 1a only once PRD-05's core-slice masker (option b, left guard, `boundary()`) is merged; the stub masker is for fake-upstream tests only".
  PRD-05 builder note step 1: "Publish the S2 stub, which stars every byte as `fail_closed` (same length, with `boundary()`), so no build ever writes a raw byte. Tests that need clear bytes use a test-only masker under `#[cfg(test)]`."
  PRD-16 section 3 Phase 1a, add: "Precondition: PRD-05's core slice is merged on the commit under test, checked by a `mask_rules` length above zero in the `start` event. No real key enters a pad box before that."

## G2-20 (blocker)
- Sources: P01-1, P02-12
- Section and lines here: Acceptance R8 (700); Risks (746-748)
- Same id also in: PRD-01-record-format, PRD-06-uploader-compression
- Ledger item or rule: Ledger F2 (locked: zstd `--long=27`); B2a (a locked change needs the owner)
- Change:
  PRD-01 row 864 becomes two rows: "zstd level | 3 | PRD-06, with a measured reason" and "Window 2^27 with long matching | locked (ledger F2) | the owner only". Lines 836-837: "The 141 MiB compressor sits in the participant's process for the whole run. The window stays 2^27 (ledger F2, locked); a smaller window needs the owner." `PartOptions.window_log` stays a parameter for tests only (line 486).
  PRD-06 line 105: drop "PRD-01 allows window 26".
  PRD-02 R8 and risks line 748: "... or PRD-01 lowers the zstd level only; `window_log` 27 and long matching stay (ledger F2)."

## G2-22 (blocker)
- Sources: P04-1, P02-3
- Section and lines here: Section 11 step 2 'Stop' (533-535), step 3 drain, step 4 (537-538)
- Same id also in: PRD-04-native-capture, PRD-00-index
- Ledger item or rule: Ledger R4 (no silent loss), points 1 and 2, PRD-01 invariant 4
- Change:
  PRD-04 section 8 first paragraph and ask 2: replace "Its S1 calls are admitted until it returns (PRD-02's seal step 1 stops the proxy's payload, not this; see Interfaces)" with "The seal's stop (PRD-02 section 11 step 2) applies to wire streams only (`conn`, `http_req`, `http_resp`, `ws_up`, `ws_down`). `native` and `snapshot` streams are never stopped: the native loop's S1 calls stay admitted from the stop until `finish` returns. The drain (step 3) waits for wire bytes only."
  PRD-02 step 2: "**Stop the wire.** Every S1 call on a wire stream after this point is a no-op that only counts. `native` and `snapshot` calls stay admitted until `NativeReader::finish` returns (PRD-00 R3-3)." Step 3: the drain waits for wire bytes only.
  PRD-00 R3-3: "covers wire streams only; native and snapshot calls stay admitted until `finish` returns".

## G2-06 (major)
- Sources: X3-6, P00-11
- Section and lines here: Builder notes step 1 (811-814)
- Same id also in: PRD-00-index, PRD-05-masking, PRD-11-connectors-harnesses
- Ledger item or rule: PRD-00 merge rule 1 and file ownership
- Change:
  PRD-00 phase 0 step 5: "PRD-02: the scaffold: `Cargo.toml` with every dependency the PRDs named, `lib.rs` with the full module list, S1 (with `for_tests` and `reserve`), S4 and S8 as compiling stubs, and an empty placeholder `mod.rs` in every other module folder of `crates/goalcup/src/`. This is the one ownership exception; each owner replaces its own placeholder in its first merge." Add: "PRD-05: the S2 stub in `src/mask/` by H+1 (it stars every byte, G2-01). PRD-11: the S3 stub in `src/connector/` by H+1, before the host list."
  PRD-02 builder note 1: add "placeholder module roots for every other folder (PRD-00 phase 0 step 5)".
  PRD-05 note 1 and PRD-11 note 1: "replace the scaffold's placeholder with the real S2 (S3) stub by H+1".

## G2-08 (major)
- Sources: X3-8, P02-23
- Section and lines here: Section 8 (332); control-line table `start` row (393); interfaces PRD-11 row (661)
- Same id also in: PRD-00-index, PRD-11-connectors-harnesses
- Ledger item or rule: Seam named but not defined; point 6 (notes reach the record)
- Change:
  PRD-00 Seams, add row "S17 `StartFacts` and the first control line: run, entry, deadline, server_time, local wall, workspace, connector ref, keep, build, `HostList`, `NativeSources`, mask rules, values, CA PEM, previous end, harness pid and start, `null_since`, `LaunchPlan.notes`; every type serializes with serde. Owner PRD-02; used by PRD-10, PRD-11, PRD-05; stable by H+1.5".
  PRD-11 S3: "`HostList`, `NativeSources`, `MaskRule` derive `Serialize` and `Deserialize`."
  PRD-02 control-line `start` row: list the S17 fields and add "`notes` from `LaunchPlan` (written with `Recorder::event` after `start` on boot 0 only)"; PRD-11 interfaces row: add `notes`.

## G2-09 (major)
- Sources: X3-9, P01-4
- Section and lines here: Section 9.1 table (410)
- Same id also in: PRD-01-record-format
- Ledger item or rule: Ledger R3, point 3; PRD-00 done item 2 (2 s bound)
- Change:
  PRD-01 line 330 and PRD-02 line 410: replace "the last durable LINK" with "the last LINK named by an index line (the point recovery truncates to, PRD-01 section 12 step 1; after a plain process kill at most about 1 s before the kill)".
  PRD-01 acceptance 3: "During a Codex pad run (`full` prompt), `kill -9` the recorder twice, 2 minutes apart, while a response streams (no test hooks). Pass: the harness finishes its turn with no participant action; the entry decodes; exactly two `recorder_down` gaps, start before end, each under 2 s long; links verify across three boots; the server copy equals local."

## G2-10 (major)
- Sources: X3-10, X1-1
- Section and lines here: Acceptance `pipeline_guard.rs` (707), `by_hand_load` (740), R8 (700)
- Same id also in: BUILD-PROMPT, PRD-16-orchestration-acceptance
- Ledger item or rule: Point 2 ('however many'), R5; ambiguous breaker instruction
- Change:
  BUILD-PROMPT lines 87-88: "a third reviewer who tries to break it against run 1's shape with PRD-02's `pipeline_guard.rs`, `pipeline_stall.rs` and `by_hand_load` (40 and then 200 streams, 1.5 MB bodies in bursts, a held writer, WebSockets) and the latest pad smoke row, once per merge request (PRD-16 section 1). There is no run 1 replay tool."
  PRD-02 `pipeline_guard.rs`: "40 streams, then 200 streams, each offer 2 GiB in 1.5 MB bodies". `by_hand_load`: "run twice: 40 clients, then 200 clients sending 1.5 MB POSTs in bursts (all at once every 60 s, as after a rate-limit window), plus 200 WebSockets at 1 message a second each". R8: report `held_peak`, the `memory_guard` gap count and S1 call time for both sizes.
  PRD-16 Breaker: "about 40 and about 200 synthetic subagents". X3: add "The 200-stream `by_hand_load` result is reported next to A1 and A2; its `memory_guard` gaps are a night-log line, not a FAIL".

## G2-16 (major)
- Sources: P00-2, P02-8, X1-3 (and L1 F6 of group 1)
- Section and lines here: Section 8 step 9 (365-371); defaults table (794)
- Same id also in: PRD-00-index
- Ledger item or rule: Ledger R3 (locked: restarts fast, no failure budget), point 1
- Change:
  PRD-02 step 9: "After 30 s the watchdog tries a normal boot again, then every 60 s after a failed try; the wait never grows. Null mode is not a failure budget: it only spaces out boots that crash at once, so each crash does not reset the harness's connections (point 3). An exit by SIGKILL (as from the kernel's OOM killer) does not count toward the 3 fast exits." Defaults row: "retry after 30 s, then every 60 s".
  PRD-00 lines 175-178: replace "retries a normal boot on a growing timer" with the same rule and why (ledger R3). Add the rule to the morning report's "Decisions agents took".

## G2-17 (major)
- Sources: P00-3, P02-9 (and L2-19 of group 1)
- Section and lines here: Section 8 intro (330-332); 'Owner decisions' (33-34)
- Same id also in: PRD-00-index
- Ledger item or rule: Ledger R3 ('a tiny watchdog that only restarts the recorder'), L5; also owner item O-4
- Change:
  PRD-00 after line 172 and PRD-02 after line 332, add: "Why this fits ledger R3: the watchdog is the `goalcup start` process the participant already runs. Spawning, shielding and ending the harness and the helper stops are today's launcher duties, not new ones. It holds the socket and the CA only so a restart keeps the same port and the same trust (R3: 'on the same port fast'). It never reads, masks, writes or uploads a recorded byte; all recording stays in the one recorder process (L5). This reading awaits the owner's confirmation (owner question, see O-4)." Add the reading to the morning report's "Decisions agents took".

## G2-19 (major)
- Sources: P00-7, P02-13, 06-21
- Section and lines here: S1 block (134-135); Files it owns (629-632); PRD-14 row (664)
- Same id also in: PRD-00-index, PRD-06-uploader-compression
- Ledger item or rule: Ledger L6 (locked: legacy and test seams cut), L2; also owner item O-3
- Change:
  PRD-00 item 86 and PRD-02 Files it owns: "Pending the owner (O-3): ledger L6 cut the test seams. The plan asks to keep one compile-time `test-hooks` feature in `crates/goalcup` only, off by default, never in a release build (PRD-14 greps the shipped binary for `GOALCUP_TEST_`), because restart, guard and rotation cannot be proven from a real run in the gate's time. No `GOALCUP_TEST_*` variable is read by a release build. `goalcup-verify` gets no hooks unless PRD-08 names one it needs." Add it to PRD-00's owner questions with this as the default.
  PRD-06 `upload_backoff.rs`: "`policy.rs` takes `now` as an argument; this case is a unit test of that pure function, with no new `GOALCUP_TEST_*` hook."

## G2-21 (major)
- Sources: P02-1, P01-3
- Section and lines here: Section 5 step 3 (255-257); section 6 'Entry totals' (281-283); 9.2 step 4 (438-443); builder notes watcher line (849-851); R3 (695)
- Same id also in: PRD-01-record-format
- Ledger item or rule: Ledger R6 (written counts), point 6; PRD-00 done item 1 (counts equal the recount); R3-2
- Change:
  PRD-02 section 5 step 3: "At every LINK, before its index line, append this boot's counters at that LINK to `counts.jsonl` (one compact line). At each durable point rewrite the file by atomic rename to one line per earlier boot plus this boot's latest." 9.2 step 4: "The recovery frame holds, before the `recorder_down` GAP, one COUNT for the dead boot taken from the `counts.jsonl` line whose `seq` equals the recovered last LINK, so that boot's last COUNT recounts every record it kept." Section 6: "`written` shows the counters at the last LINK (at most 1 s behind), so it never goes down across a crash."
  PRD-01 section 9, add: "A record belongs to the boot of the nearest PART before it, so a recovery frame belongs to the dead boot. Its COUNT (PRD-02 9.2 step 4) recounts that boot up to the last kept LINK. Entry totals = each boot's last COUNT, plus the `recorder_down` GAP of each recovery frame (`gaps`, `lost_unknown`, `off_ms`). The recovered seal carries the totals in `totals`." Section 12 step 3: the recovery frame writes that COUNT for a normal restart too, as it already does for a dead entry. Section 15 item 7: "Every COUNT is an exact recount of its boot up to that COUNT. Seal totals are the sum above; PRD-08 recounts with the same rule (`counts_match_seal`)." Invariant 7 reads per boot, the recovery frame's COUNT included.

## G2-23 (major)
- Sources: P03-1, X2-04 (part); P02-17 dropped as superseded
- Section and lines here: Section 2 rule 7 (173-176); Risks 'Memory' (749-752)
- Same id also in: PRD-03-proxy
- Ledger item or rule: Ledger R5 (one stated guard), L2; PRD-00 R3-1
- Change:
  PRD-03 section 7, replace the counter paragraph: "Before a coded body is decoded, call `Recorder::room()`; if the window (`1 << WBITS` for brotli, the zstd frame window up to 8 MiB, 32 KiB for gzip and deflate) fits, hold `Recorder::reserve(window)` until the body ends. If it does not fit, the body is not decoded: GAP `memory_guard` with `{"guard":"decoder_window","limit":<room at that moment>}` (lost as G2-26)." Drop "256 MiB process-wide" from sections 7 and 11 and from Overridable defaults; add `Recorder::reserve` and `room` to the PRD-02 interfaces row; change "with the decoder guard set low" to "with `for_tests(room)` small".
  PRD-02 keeps rule 7, `reserve` and ask 4. Risks "Memory": "decoder windows sit inside the 512 MiB guard through `reserve`".

## G2-24 (major)
- Sources: P03-2, P02-4, P03-12, X2-04 (part)
- Section and lines here: Section 11 step 1 'Grace' (527-532)
- Same id also in: PRD-03-proxy
- Ledger item or rule: Ledger R2 (seal timing), L2 (no unused surface); lesson P8
- Change:
  PRD-03 section 10: `ProxyHandle` offers one accessor, `quiet_since() -> Mono`: the `Recorder::now()` of the last byte any relay moved. Open but silent connections do not count. Delete `idle()`, `stop_accepting()` and `open_conns()` from the text, line 498, line 616, builder note 3 and asks 3. `proxy_restart.rs`: test that a silent open connection does not delay `quiet_since`.
  PRD-02 step 1: "The quiet time comes from `ProxyHandle::quiet_since()`: the grace ends when it is 5 s old, or 30 s after the `exit` line." `after_stop` hosts stay from the writer's table.

## G2-28 (major)
- Sources: P04-2, L3-4 (group 1)
- Section and lines here: Section 11 step 4 and 'The bound' (552)
- Same id also in: PRD-04-native-capture, PRD-00-index
- Ledger item or rule: Ledger R5 (no caps of our own), H8 (session files with no caps)
- Change:
  PRD-04 step 2: "Read every append file to the size seen in step 1's pass, and every unsettled snapshot as it is." Step 5: "Time guard: 180 s, only when the seal reason is `deadline`, `server_stop` or `signal` (today's finish budget, `control.rs:708`); it guards the end of the run window. On `harness_exit` and `seal_command`, finish reads every file to the size seen in step 1 with no time guard; status shows `sealing: reading session files` meanwhile." Defaults row: "180 s at `deadline`, `server_stop`, `signal`; none otherwise | Owner".
  PRD-02 section 11 step 4 and "The bound", PRD-00 S8: the same rule.

## G2-29 (major)
- Sources: P04-3, P02-24
- Section and lines here: Ask 5 (870-871)
- Same id also in: PRD-04-native-capture
- Ledger item or rule: PRD-00 S1 (seam owned by PRD-02); builder misread
- Change:
  PRD-04 section 5 first bullet: replace "It keeps a reserve ... no new seam is needed." with "It reads at most `room()`. `room()` already answers against three quarters of the soft guard (PRD-00 S1, PRD-02 section 3), so native never pushes the wire into the band where it drops, and the reader subtracts nothing of its own." Defaults row: "Native share of the pipeline | `room()` (3/4 of the soft guard, PRD-02) | PRD-02". Delete the "Own check" row about the reserve.
  PRD-02 ask 5: mark ruled; PRD-02's text stands.

## G2-31 (major)
- Sources: X1-4, X1-11, 06-10 (and L2-03, L2-07, L2-08 of group 1)
- Section and lines here: Section 3 hard guard (188, 211-217); section 5 steps 1 and 5 (247, 263); section 6 `mode` (305-306); defaults (787)
- Same id also in: PRD-06-uploader-compression, PRD-07-worker-server
- Ledger item or rule: Ledger R3, R4, R7 (heartbeat says `failed` when recording is off)
- Change:
  PRD-02 section 5 step 5: "Every second, a status thread (not the writer) writes `status.json` from the live atomics: `written`, `losing`, `held_bytes`, `writer_idle_ms`." Section 3: "Writer stall: when records wait and the writer has written nothing for 60 s of boot time, or when control items alone pass 2 x soft, the recorder exits 75 and the watchdog restarts it, so the stall becomes a `recorder_down` gap with cause `writer_stalled`. This is a recorder failure under R3, not an R5 guard; it is the one stall exit."
  PRD-06 section 6: compute `mode` from `Durable` age and the live atomics, not only `status.json`. `ok` row: remove "or `writer_slow`". `failed` row: "`mode` null, or `writer_idle_ms` over 60 s while records wait".
  PRD-07 line 197: replace "`mode` null (null mode), or the entry ended while off" with "recording is off: null mode, or a writer stalled past 60 s; a recovered entry is `gaps` (PRD-06 section 6)". Add `sealed` to its `mode` list (the final heartbeat sends `mode: sealed`).

## G2-32 (major)
- Sources: X1-6, 06-2
- Section and lines here: Section 9.2 step 1 (430); section 2 rule 1; section 12 P11 (575); acceptance test list (702-742)
- Same id also in: PRD-06-uploader-compression
- Ledger item or rule: Point 3 ('note a gap'), ledger R2, R3, R4
- Change:
  PRD-02 9.2 step 1: "A panic in the writer, the mask worker or the native thread exits 70. The uploader catches its own panics and never ends the process (PRD-06 section 1)." Section 2 rule 1: add "An S1 send whose receiver is gone exits the process with 70 (once, from the first caller), so the watchdog restarts it and a `recorder_down` gap covers the span." Test list: add "`pipeline_panic.rs`: kill the mask worker loop with a test hook; the recorder exits 70, the next boot writes `recorder_down`, and relays never hang".
  PRD-06 section 1: "Each upload round and each beat runs inside `catch_unwind`. A panic there writes one fixed-text line through `cli::log::line` (S11), sets `upload.json` `state` `waiting` with class `internal`, backs off by section 5 and never ends the process. Recording never depends on the uploader." `upload_resume.rs`: a scripted answer that panics the parser leaves the recorder running.

## P02-10 (major)
- Sources: P02-10
- Section and lines here: Section 8 step 10 (372-380); section 11 triggers (523-524); control table `server` row (397); 'Owner decisions' (37)
- Ledger item or rule: AGENTS.md hard rule 'records and never manages the agent' (ledger section 7); PRD-00 owner question 2 covers only the deadline and `goalcup seal`
- Change:
  Step 10: "On the server's `stop` with `server_reason: "deadline"` it ends the harness as at the deadline. On any other reason it seals the recording (`server_stop`, `harness: "running"`) and leaves the harness alone; PRD-10 says one line." Line 37: "(owner question 2, default yes)".

## P02-11 (major)
- Sources: P02-11
- Section and lines here: 9.1 'watchdog only' row (412); R5 (697); risks (762-764)
- Ledger item or rule: Point 3; acceptance R5 not provable as written
- Change:
  9.1 row: "Recording goes on; a non-TUI harness is unaffected; a TUI harness likely loses its terminal (the shell takes the foreground back), so it may stop or exit. The recorder seals when it exits." R5 pass line: "the recorder keeps writing while the harness runs; the watcher writes down whether the harness kept working; the entry seals with reason `harness_exit` once it exits." Add the same as a risks row.

## P02-2 (major)
- Sources: P02-2 (the `finish` part is G2-15)
- Section and lines here: S1 block (122-148); section 5 step 4; section 6 (274-311); interfaces PRD-06 and PRD-10 rows (658, 660); header (3-5)
- Ledger item or rule: PRD-00 S4 and ruling R3-2
- Change:
  Add to the S1 block `pub fn entry_totals(entry_dir: &Path) -> Counts; // S4, R3-2`. Section 5 step 4: "the counters at that LINK, as entry totals". Section 6: add "`written` carries every COUNT key (PRD-01 section 9) as entry totals; `notes` keeps the first note of each `what` beside the last 20; in `mode: sealed` the file also carries `seal {reason, workspace_head}`, `cut_files {files, bytes}` from `NativeFinish` (P04-8), and `outlived`; `close_dead_entry` writes the same sealed file."

## P02-5 (major)
- Sources: P02-5
- Section and lines here: Section 4 'Per item' (223-229)
- Ledger item or rule: Ledger M1 (masking before any byte reaches disk), point 4
- Change:
  Add to the per-item list: "EVENT runs `mask_meta` on its body, every event (cheap, and no caller has to know which ones carry upstream text). A GAP or END tail runs `mask_meta` too."

## P02-6 (major)
- Sources: P02-6
- Section and lines here: Section 8 step 9 (365-371); control-line table; section 11 triggers (522-525)
- Ledger item or rule: Ledger R3 (locked: never stops for good)
- Change:
  Step 9: "send the null recorder a `restart` control line; it exits 0 without sealing, then start a normal one". Control-line table: add "Watchdog to recorder: `restart` (null recorder only): exit without sealing". Section 11 triggers: SIGTERM stays the seal signal for outside callers only.

## P02-7 (major)
- Sources: P02-7
- Section and lines here: Section 11 (520-556); `watchdog_null.rs`
- Ledger item or rule: Point 5; ledger S1 (verifier needs a sealed record), R3
- Change:
  Add to section 11: "**In null mode** the null recorder seals through `close_dead_entry`'s recovery steps: recover the last part file, write in the recovery frame `GAP recorder_down` with cause `null_mode` up to now, the dead boot's COUNT (G2-21), the totals, the `seal` event with the real reason and `harness`, LINK, frame end, `E` line; then the uploader finishes as usual." Add a case to `watchdog_null.rs`: a harness exit during null mode gives a sealed entry that verifies.

## P04-8 (major)
- Sources: P04-8
- Section and lines here: Section 6 sealed `status.json`
- Same id also in: PRD-04-native-capture
- Ledger item or rule: Point 6; PRD-10 interface and ruling R3-2
- Change:
  PRD-04 line 82: `pub struct NativeFinish { pub streams: u64, pub snapshots: u64, pub cut_files: u64, pub cut_bytes: u64, pub took_ms: u64 }`. Step 5: "The count and bytes go into `NativeFinish.cut_files` and `cut_bytes`, and PRD-02 writes them as sealed `status.json` `cut_files {files, bytes}` (PRD-10)." PRD-02 section 6: as P02-2.

## P05-6 (major)
- Sources: P05-6
- Section and lines here: Section 4
- Same id also in: PRD-05-masking
- Ledger item or rule: Ledger R4 (lost bytes where known); PRD-01 invariant 4
- Change:
  Row "Masker panic" and PRD-02 section 4: "PRD-02 drops the piece and the held bytes, replaces the masker and writes GAP `mask_failed` (12) from the last released offset to the end of the dropped piece (held bytes included; the worker knows both offsets)."

## G2-12 (minor)
- Sources: X3-17, P00-14, P05-9, P02-16, 06-12
- Section and lines here: Lines 37, 796; line 120; header (3-5)
- Same id also in: PRD-16-orchestration-acceptance, PRD-05-masking, PRD-00-index, PRD-06-uploader-compression
- Ledger item or rule: Clarity; ledger is the source of truth
- Change:
  Replace revision 2 owner question numbers with PRD-00's current eight. Map: old 1, 2, 4, 6, 7 are question 1; old 8, 9, 10, 11, 13 are question 2; old 12 is 3; old 3, 14, 15 are 4; old 5 is 5; old 16 is 6; old 19 is 7; old 17 and 18 are 8. Write "owner question N (old M)" where a reader may meet the old number.
  PRD-00 line 681: "owner question 2". PRD-00 review answers: write each as "old N (now M)".
  PRD-02: "owner question 2" (line 37); line 120: "This is PRD-00's S1 block (revision 3, rulings R3-1 and R3-2)"; header: "Index: PRD-00 revision 3".
  PRD-06: every "owner question 6" and "owner question 2" becomes "PRD-00 owner question 1"; in "Owner decisions it carries out" tag each line with its ledger id: F2 (zstd), U1 (prune, small chunks, capped backoff that resets on progress), R5 (no caps), R6 and R7 (written counts, `failed`), L5 (one process), B4 (staging).

## G2-14 (minor)
- Sources: X3-21, P02-18
- Section and lines here: Acceptance R1 (693)
- Ledger item or rule: Acceptance a watcher cannot run
- Change:
  Replace "the watchdog log shows the first accept after each kill within 1 s" with "`goalcup.log` shows the next boot's `ready` within 1 s of each kill, and the next boot's first `conn` OPEN comes after its `recorder_down` gap end (`goalcup-verify dump`)".

## G2-15 (minor)
- Sources: X3-28, P02-2 (part)
- Section and lines here: Section 9.2 step 5 (445); section 11 step 6
- Same id also in: PRD-00-index, PRD-06-uploader-compression
- Ledger item or rule: Seam named but not defined; ruling R3-2
- Change:
  PRD-00 Seams, add "S18 `upload::spawn(ctx) -> UploadHandle`, `UploadHandle::finish(wrap_up, seal_facts { reason, workspace_head, totals })`, owner PRD-06, used by PRD-02, stable by H+3". R3-2 line 863 cites S18.
  PRD-02 section 11 step 6 and 9.2 step 5: "hand over with `UploadHandle::finish(wrap_up, seal_facts)`, `seal_facts` holding `reason`, `workspace_head` and `totals`".
  PRD-06 Files and Interfaces: the same signature.

## G2-33 (minor)
- Sources: X1-9, P02-29
- Section and lines here: Section 6 'Counted at write' (276-280)
- Same id also in: PRD-01-record-format, PRD-10-cli-participant
- Ledger item or rule: Ledger R6 ('a model call counts only once its bytes are in the record'), point 6
- Change:
  PRD-01 section 9, after the `req_whole` bullet: "The heartbeat, status and end screen show `req_whole` next to `model_requests` whenever `req_holes` is above 0." PRD-02 section 6: "Status shows `req_holes` and `req_whole` beside `model_requests`, so a request whose body is in a gap is never shown as whole." PRD-10 end screen rule 3: name `req_whole` as the "written whole" number.

## P02-14 (minor)
- Sources: P02-14
- Section and lines here: 'Owner decisions' (28)
- Ledger item or rule: Clarity
- Change:
  "(ledger L5)" in place of "(ledger L2)".

## P02-15 (minor)
- Sources: P02-15
- Section and lines here: 'Owner decisions' (36)
- Ledger item or rule: Ledger L6 (disk-full handling cut), section 7 (no D65)
- Change:
  "Disk full: no special handling. A write error ends the recorder like any crash; after three fast exits null mode keeps the harness working, unrecorded, and status says why. Nothing freezes the agent (D65 is gone)."

## P02-19 (minor)
- Sources: P02-19
- Section and lines here: Acceptance R4 (696)
- Ledger item or rule: Acceptance provable
- Change:
  "at least `min(4096, hard)` for the harness".

## P02-20 (minor)
- Sources: P02-20
- Section and lines here: Acceptance R5 (697)
- Ledger item or rule: Acceptance provable
- Change:
  "the seal starts within 31 s of the harness exit (`goalcup.log`) and the `seal` event is written within the section 11 bound".

## P02-21 (minor)
- Sources: P02-21
- Section and lines here: Acceptance R7 (699)
- Ledger item or rule: Acceptance provable
- Change:
  "per native path, the new entry's first `from` equals the old entry's last written offset (checked from both `goalcup-verify dump` outputs)".

## P02-22 (minor)
- Sources: P02-22
- Section and lines here: Section 11 step 5 (543-545)
- Ledger item or rule: Point 3 (never block the harness's own git)
- Change:
  "`git --no-optional-locks status --porcelain -z`".

## P02-25 (minor)
- Sources: P02-25
- Section and lines here: S1 `fds` (147); section 10 'Reserve' (510-515)
- Ledger item or rule: Interface (PRD-14 `proxy_connect.rs` calls it)
- Change:
  Add to `fds`: `pub fn raise(target: Target) -> u64` (used by the watchdog's `pre_exec` and by tests).

## P02-26 (minor)
- Sources: P02-26
- Section and lines here: Risks `PR_SET_DUMPABLE` (771-773)
- Ledger item or rule: Acceptance provable
- Change:
  Add: "PRD-12's open-file count of the recorder and watchdog needs root in the sandbox (or reads the `fd_limit` note and `status.json` instead)."

## P02-27 (minor)
- Sources: P02-27
- Section and lines here: Acceptance header (686); R8 (700)
- Ledger item or rule: Ledger B1
- Change:
  "Vercel Sandbox (Linux)" in both places.

## P02-28 (minor)
- Sources: P02-28
- Section and lines here: Builder notes order (809-824)
- Ledger item or rule: Ledger B3 (locked: one short real test after each new core feature)
- Change:
  Add after step 3: "After each item above that changes recording (mask worker, seal, restart, null mode, native resume), one short real `smoke` run in a sandbox (PRD-16 watchers)."

## P03-19 (minor)
- Sources: P03-19
- Section and lines here: Section 6 (status)
- Same id also in: PRD-03-proxy
- Ledger item or rule: Point 6
- Change:
  PRD-03 Asks 3, add: "PRD-02: set status `losing` while a `parse_stopped` connection is open, and clear it at that connection's close GAP." PRD-02 section 6: the same rule.

## X1-8 (minor)
- Sources: X1-8 (its macOS sentence dropped: T9 leaves macOS to the owner)
- Section and lines here: Section 10 (516)
- Ledger item or rule: Point 3, R2
- Change:
  Line 516, add: "Past that, with long-lived WebSockets, waiting connections may never be served: the recorder writes the `fd_limit` note and `goalcup status` and the end screen say `Too few open files: N agent connections waited.`"

## X2-05 (minor)
- Sources: X2-05
- Section and lines here: Section 11 step 3 (536); `pipeline_seal.rs` (730-732); Overridable defaults 'Seal drain'
- Ledger item or rule: Ledger R5 (guards never stop recording), R4
- Change:
  Line 536: "**Drain:** wait until `held` is 0 while the writer makes progress; stop early only when the writer wrote nothing for 60 s (5 s for `signal`). Bytes still queued then become `cut_at_seal` gaps." Lines 730-732: "past the drain budget" becomes "after the writer stalled for 60 s". Defaults row "Seal drain" the same.

## X2-11 (minor)
- Sources: X2-11
- Section and lines here: 'Review answers' section and fully ruled 'Asks' sections
- Same id also in: PRD-00-index, PRD-01-record-format, PRD-03-proxy, PRD-04-native-capture, PRD-05-masking, PRD-06-uploader-compression, PRD-07-worker-server, PRD-08-verifier-checks, PRD-09-usage-research-stats, PRD-10-cli-participant, PRD-11-connectors-harnesses, PRD-12-test-pad, PRD-13-cuts, PRD-14-tests-gate, PRD-15-docs-decisions, PRD-16-orchestration-acceptance, PRD-17-lessons-old-code
- Ledger item or rule: Ledger L2; point 6 (plain, true text). Apply last, after every other fix, so line numbers in the fix lists stay valid
- Change:
  Move every "Review answers" section and every fully ruled "Asks" section into one file `docs/plans/2026-10-30-v1.0/REVIEW-HISTORY.md`, verbatim, one heading per PRD. Before moving, copy any ruling still binding into the PRD body. Each PRD keeps one line at its end: "Review history: REVIEW-HISTORY.md, section PRD-NN." The replan keeps new asks in the Amendments note only.

## X3-22 (minor)
- Sources: X3-22, P05 'also small' (MaskError from a participant value)
- Section and lines here: Section 8 step 5 (349-350); control-line table (391-397)
- Same id also in: PRD-05-masking
- Ledger item or rule: Seam gap; point 6
- Change:
  PRD-02 section 8: "A recorder that cannot build its `Masker` from built-in shapes exits 78 before `ready`; the watchdog passes 78 to PRD-10, which shows PRD-05's fixed text."
  PRD-05: "A `MaskError` from a built-in shape fails `start` (exit 78). A `MaskError` from a participant value (for example a compile limit) skips and counts that value; it never fails `start`."
