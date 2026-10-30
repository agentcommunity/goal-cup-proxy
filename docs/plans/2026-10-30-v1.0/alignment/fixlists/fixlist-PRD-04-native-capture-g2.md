# Fix list for PRD-04-native-capture.md (triage group 2)

Source reviews: align-X1-run1, align-X2-scope, align-X3-builder, align-PRD-00 to align-PRD-06.
The ledger (OWNER-DECISIONS.md) wins. An id shared by several fix lists carries the same change text in each;
make in this file only the part that names this file. Items are ordered by severity, then id.

Counts: 1 blocker, 7 major, 17 minor, 25 in all.

## G2-22 (blocker)
- Sources: P04-1, P02-3
- Section and lines here: Section 8 first paragraph (328-329); ask 2 (527-529)
- Same id also in: PRD-02-recorder-core, PRD-00-index
- Ledger item or rule: Ledger R4 (no silent loss), points 1 and 2, PRD-01 invariant 4
- Change:
  PRD-04 section 8 first paragraph and ask 2: replace "Its S1 calls are admitted until it returns (PRD-02's seal step 1 stops the proxy's payload, not this; see Interfaces)" with "The seal's stop (PRD-02 section 11 step 2) applies to wire streams only (`conn`, `http_req`, `http_resp`, `ws_up`, `ws_down`). `native` and `snapshot` streams are never stopped: the native loop's S1 calls stay admitted from the stop until `finish` returns. The drain (step 3) waits for wire bytes only."
  PRD-02 step 2: "**Stop the wire.** Every S1 call on a wire stream after this point is a no-op that only counts. `native` and `snapshot` calls stay admitted until `NativeReader::finish` returns (PRD-00 R3-3)." Step 3: the drain waits for wire bytes only.
  PRD-00 R3-3: "covers wire streams only; native and snapshot calls stay admitted until `finish` returns".

## G2-28 (major)
- Sources: P04-2, L3-4 (group 1)
- Section and lines here: Section 8 step 2, step 5 (336-343); defaults row 'Finish time guard' (515)
- Same id also in: PRD-02-recorder-core, PRD-00-index
- Ledger item or rule: Ledger R5 (no caps of our own), H8 (session files with no caps)
- Change:
  PRD-04 step 2: "Read every append file to the size seen in step 1's pass, and every unsettled snapshot as it is." Step 5: "Time guard: 180 s, only when the seal reason is `deadline`, `server_stop` or `signal` (today's finish budget, `control.rs:708`); it guards the end of the run window. On `harness_exit` and `seal_command`, finish reads every file to the size seen in step 1 with no time guard; status shows `sealing: reading session files` meanwhile." Defaults row: "180 s at `deadline`, `server_stop`, `signal`; none otherwise | Owner".
  PRD-02 section 11 step 4 and "The bound", PRD-00 S8: the same rule.

## G2-29 (major)
- Sources: P04-3, P02-24
- Section and lines here: Section 5 (237-242); defaults row 'Native share' (510); review answer 'Own check' (602)
- Same id also in: PRD-02-recorder-core
- Ledger item or rule: PRD-00 S1 (seam owned by PRD-02); builder misread
- Change:
  PRD-04 section 5 first bullet: replace "It keeps a reserve ... no new seam is needed." with "It reads at most `room()`. `room()` already answers against three quarters of the soft guard (PRD-00 S1, PRD-02 section 3), so native never pushes the wire into the band where it drops, and the reader subtracts nothing of its own." Defaults row: "Native share of the pipeline | `room()` (3/4 of the soft guard, PRD-02) | PRD-02". Delete the "Own check" row about the reserve.
  PRD-02 ask 5: mark ruled; PRD-02's text stands.

## P04-4 (major)
- Sources: P04-4
- Section and lines here: Section 4 table (204-211); section 1 (85-95); `native_tail.rs`
- Ledger item or rule: Ledger R4 (every loss writes a gap), point 2
- Change:
  Section 4, after the first paragraph: "A file's `native` stream is opened when the file is first taken, before any read, so a later loss always has a stream to carry its GAP. A file still unreadable at `finish` gets `gap(SourceChanged, start, Some(size - offset), {"unread_at_least": size - offset})` and END `source_changed` (PRD-01 GAP 9 row)." Add a `native_tail.rs` case: a file unreadable from the first pass to the seal gives an OPEN, one GAP `source_changed` and an END.

## P04-5 (major)
- Sources: P04-5
- Section and lines here: Section 9 'Same device only' (359-361); `native_walk.rs` (457); builder notes
- Same id also in: PRD-01-record-format, PRD-10-cli-participant
- Ledger item or rule: Ledger R4 (no silent loss), point 6
- Change:
  PRD-04 section 9 bullet, append: "Each such folder gets one `note` `native_other_device` (`row`, the path relative to the root) per entry, so the miss is named." `native_walk.rs`: "a folder whose `st_dev` differs (an existing mount under the test root, or skipped with a printed reason when none exists) is not entered and gives one note". Builder notes: use `find <root> -xdev`.
  PRD-01: add `native_other_device` to the note list. PRD-10: one plain line for it.

## P04-6 (major)
- Sources: P04-6
- Section and lines here: Section 2 (127-131); section 6 step 2 (257-276); `native_snapshot.rs`
- Ledger item or rule: Point 2; ledger T4 and T8 (Hermes must pass); H8
- Change:
  Section 6 step 2, add: "A set is taken whole: if any file of a `final` row's set passes the run-start rule, every file of that set is copied, whatever its own times." Add a `native_snapshot.rs` case: a database file older than the run start whose `-wal` is new is copied with its `-wal`.

## P04-7 (major)
- Sources: P04-7
- Section and lines here: Section 6 (257-260, 268-276)
- Ledger item or rule: Builder misread; PRD-01 section 4, PRD-09 interface
- Change:
  Step 2, first sentence: "A set is one database: the files of one row's matches that share a name once a `-wal`, `-shm` or `-journal` suffix is removed. Each set has its own `set` id, interval and retry." Step 3: replace "read the whole set once more; if it moved again, keep the second copy" with "END every stream of that copy `source_changed`, then read the whole set once more as a new `set`. If that copy also moved, END it `complete` and add a `note` `snapshot_unstable` with the path. Both copies stay in the record; PRD-09 takes the newest that opens."

## P04-8 (major)
- Sources: P04-8
- Section and lines here: Section 1 `NativeFinish` (82); section 8 step 5 (340-341)
- Same id also in: PRD-02-recorder-core
- Ledger item or rule: Point 6; PRD-10 interface and ruling R3-2
- Change:
  PRD-04 line 82: `pub struct NativeFinish { pub streams: u64, pub snapshots: u64, pub cut_files: u64, pub cut_bytes: u64, pub took_ms: u64 }`. Step 5: "The count and bytes go into `NativeFinish.cut_files` and `cut_bytes`, and PRD-02 writes them as sealed `status.json` `cut_files {files, bytes}` (PRD-10)." PRD-02 section 6: as P02-2.

## P04-10 (minor)
- Sources: P04-10
- Section and lines here: Section 1 (74); section 8 (329); ask 2
- Ledger item or rule: Builder misread
- Change:
  Line 74: "boot step 5". Line 329 and ask 2: "seal step 2 (the stop)" and "step 4 (native finish)".

## P04-11 (minor)
- Sources: P04-11
- Section and lines here: Section 2 'Two points for PRD-11' (158-162)
- Ledger item or rule: Clarity
- Change:
  Replace the Hermes half with "PRD-11 already gives Hermes's `manifest.json` its own `snapshot` row before the `task-*.log` row." Keep the Codex `*.sqlite*` half.

## P04-12 (minor)
- Sources: P04-12
- Section and lines here: Section 6 step 4 (277-281)
- Ledger item or rule: Builder misread
- Change:
  Replace the parenthesis with "(a seal the recorder runs with no live watchdog, for example the server's `stop`, or a harness the watchdog could not end)".

## P04-13 (minor)
- Sources: P04-13
- Section and lines here: Section 6 step 7 (293-295)
- Ledger item or rule: Clarity
- Change:
  "... before it sends the `seal` line, so they run before the seal calls `finish` (PRD-00 S8)".

## P04-14 (minor)
- Sources: P04-14
- Section and lines here: `native_safety.rs` (461)
- Ledger item or rule: AGENTS.md 'No harness names in core code'; ledger L6 (deny lists cut)
- Change:
  Reword the case: "the Hermes rows from PRD-11's embedded connector, run against a home holding `sessions/request_dump_x.json`, match no such file. `src/native/` holds no exclusion list and no harness name."

## P04-15 (minor)
- Sources: P04-15
- Section and lines here: Section 9 last bullet (362-363); builder notes (563)
- Same id also in: PRD-11-connectors-harnesses
- Ledger item or rule: Point 4; ledger L6 (no deny lists in code)
- Change:
  PRD-04 asks of PRD-11: "the connector test refuses a native row whose pattern matches any of the connector's own `secret_files`". No check in `src/native/`. PRD-11 adds that connector test.

## P04-16 (minor)
- Sources: P04-16
- Section and lines here: Section 2 'Rows' (124-125); section 7 ID (304)
- Same id also in: PRD-11-connectors-harnesses
- Ledger item or rule: Ledger H8 (the file path is the ID)
- Change:
  PRD-04 line 124: "Row names are unique within a connector (refused at load, PRD-11). Rows may overlap; a file is read once, by the first row (in file order) that matches it, compared by the file's absolute path under the resolved roots." PRD-11: refuse duplicate row names at load.

## P04-17 (minor)
- Sources: P04-17
- Section and lines here: Section 1 step 4 (93-95)
- Ledger item or rule: Point 2 (latency only)
- Change:
  Append: "A walk still runs once the pass interval has passed, even with unread bytes."

## P04-18 (minor)
- Sources: P04-18
- Section and lines here: Section 4 snapshot form (228-233)
- Ledger item or rule: Clarity
- Change:
  Replace "a new generation" for snapshots with "a new `snapshot` stream (snapshots carry no `gen`; `mtime_ns` and `fid` tell copies apart)".

## P04-19 (minor)
- Sources: P04-19
- Section and lines here: Acceptance 3 (438-441)
- Ledger item or rule: Acceptance provable from a real run; less code
- Change:
  "Pass: `goalcup-verify usage --json` (PRD-09) on the entry reports `readable: true`, `db_set` naming the seal set with `harness_exited: true`, and a thread whose parent comes from `sessions.parent_session_id` (column name unverified on 0.22.5)".

## P04-20 (minor)
- Sources: P04-20
- Section and lines here: Acceptance (428-450)
- Ledger item or rule: Acceptance provable from a real run
- Change:
  Add acceptance 7: "A1 (2-hour Codex, PRD-16): at least one periodic `final` set with `harness_exited: false` per changed Codex database, and a seal set. Logged, as acceptance 6."

## P04-21 (minor)
- Sources: P04-21
- Section and lines here: Builder notes, watcher proof (565-568)
- Ledger item or rule: Acceptance provable
- Change:
  "`find <root> -xdev -type f \( -newermt @<since> -o -newerct @<since> \)`, with `<since>` the run start in whole seconds".

## P04-22 (minor)
- Sources: P04-22
- Section and lines here: 'What is cut' net (423-424)
- Ledger item or rule: Clarity (arithmetic)
- Change:
  "about 3,400 src lines ... become about 780".

## P04-23 (minor)
- Sources: P04-23
- Section and lines here: Section 8 step 5 (342-343)
- Ledger item or rule: Clarity
- Change:
  "At PRD-05's 50 MB/s target (unmeasured), 180 s is about 9 GB; at 30 MB/s about 5 GB".

## P04-24 (minor)
- Sources: P04-24
- Section and lines here: Section 6 step 1 (262-267)
- Ledger item or rule: Ledger R5 wording; clarity
- Change:
  Add: "This is a cost guard on upload and storage, not a cap: every database is copied whole at every seal. The interval restarts at each boot (a restart may copy one interval late)."

## P04-9 (minor)
- Sources: P04-9
- Section and lines here: Section 2 `warn_missing` (115, 133-135); example (141-147)
- Ledger item or rule: Point 6; PRD-11 schema
- Change:
  Line 115: "`warn_missing` | Default `true` (PRD-11); `false` silences the note". Lines 133-135: "if no file has been taken 5 minutes after the first model request was written (`rec.counts()`), the reader writes one `note` ...". Add `warn_missing = false` to the `tool-results` and `meta` rows of the example, or say "abridged; PRD-11 section 6 is the source".

## X2-11 (minor)
- Sources: X2-11
- Section and lines here: 'Review answers' section and fully ruled 'Asks' sections
- Same id also in: PRD-00-index, PRD-01-record-format, PRD-02-recorder-core, PRD-03-proxy, PRD-05-masking, PRD-06-uploader-compression, PRD-07-worker-server, PRD-08-verifier-checks, PRD-09-usage-research-stats, PRD-10-cli-participant, PRD-11-connectors-harnesses, PRD-12-test-pad, PRD-13-cuts, PRD-14-tests-gate, PRD-15-docs-decisions, PRD-16-orchestration-acceptance, PRD-17-lessons-old-code
- Ledger item or rule: Ledger L2; point 6 (plain, true text). Apply last, after every other fix, so line numbers in the fix lists stay valid
- Change:
  Move every "Review answers" section and every fully ruled "Asks" section into one file `docs/plans/2026-10-30-v1.0/REVIEW-HISTORY.md`, verbatim, one heading per PRD. Before moving, copy any ruling still binding into the PRD body. Each PRD keeps one line at its end: "Review history: REVIEW-HISTORY.md, section PRD-NN." The replan keeps new asks in the Amendments note only.
