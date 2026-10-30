# Fix list for PRD-00-index.md (triage group 2)

Source reviews: align-X1-run1, align-X2-scope, align-X3-builder, align-PRD-00 to align-PRD-06.
The ledger (OWNER-DECISIONS.md) wins. An id shared by several fix lists carries the same change text in each;
make in this file only the part that names this file. Items are ordered by severity, then id.

Counts: 2 blocker, 17 major, 17 minor, 36 in all.

## G2-22 (blocker)
- Sources: P04-1, P02-3
- Section and lines here: Ruling R3-3 (864)
- Same id also in: PRD-04-native-capture, PRD-02-recorder-core
- Ledger item or rule: Ledger R4 (no silent loss), points 1 and 2, PRD-01 invariant 4
- Change:
  PRD-04 section 8 first paragraph and ask 2: replace "Its S1 calls are admitted until it returns (PRD-02's seal step 1 stops the proxy's payload, not this; see Interfaces)" with "The seal's stop (PRD-02 section 11 step 2) applies to wire streams only (`conn`, `http_req`, `http_resp`, `ws_up`, `ws_down`). `native` and `snapshot` streams are never stopped: the native loop's S1 calls stay admitted from the stop until `finish` returns. The drain (step 3) waits for wire bytes only."
  PRD-02 step 2: "**Stop the wire.** Every S1 call on a wire stream after this point is a no-op that only counts. `native` and `snapshot` calls stay admitted until `NativeReader::finish` returns (PRD-00 R3-3)." Step 3: the drain waits for wire bytes only.
  PRD-00 R3-3: "covers wire streams only; native and snapshot calls stay admitted until `finish` returns".

## P00-1 (blocker)
- Sources: P00-1 (and X2 caps table)
- Section and lines here: 'Defaults taken', Spend row (639); owner question 8 (731-733); risk row
- Same id also in: PRD-16-orchestration-acceptance, PRD-12-test-pad
- Ledger item or rule: Ledger T6 (locked: 10 USD per run, cost otherwise not a constraint), B3
- Change:
  PRD-00 Spend row: replace the whole-build stop line with "No whole-build cap (ledger T6). The scribe reports running spend in the night log at 200 and 400 USD by the pad's ledger estimate and keeps going." Owner question 8: delete the build stop line sentence. "2 USD per `smoke` run" becomes "an estimate, not a cap".
  PRD-16 section 9 and defaults, PRD-12 lines 328-329 and 699: delete the whole-build model and sandbox rows; keep the 10 USD per-run cap (T6); `smoke` 2 USD is an estimate, not a cap.

## G2-03 (major)
- Sources: X3-3, P00-4
- Section and lines here: Ruling R3-10 (871); ownership row BUILD-PROMPT (339)
- Same id also in: BUILD-PROMPT
- Ledger item or rule: Ledger B1, B2, B2a (locked, win over both texts); ALIGN known conflict R3-10
- Change:
  Edit before H+0 (the prompt is frozen only during the build).
  BUILD-PROMPT section 4: replace its own lane table with a pointer to PRD-16 section 1's twelve-row table (Judge, Participant and Masking start at H+0 against the stubs; PRD-14's gate change lands first).
  BUILD-PROMPT section 1 item 5: "Follow it. Where this prompt and PRD-16 differ, PRD-16 wins, except a locked ledger item, which wins over both."
  PRD-00 R3-10: replace with "Ledger B1, B2 and B2a win over BUILD-PROMPT.md and PRD-16. Within them, the lane table, start times and roles are PRD-16 section 1's."

## G2-04 (major)
- Sources: X3-4, P00-5, X3-19
- Section and lines here: Ownership rows BUILD-PROMPT and PRD-NN (339-340)
- Same id also in: BUILD-PROMPT, PRD-16-orchestration-acceptance
- Ledger item or rule: Ledger B2a (locked: replan updates the remaining PRDs and the build plan)
- Change:
  PRD-00 row 339: "Edited before H+0 only. The build plan of ledger B2a is PRD-16 plus PRD-00 'Build order'; during the build the replan updates them, and the remaining PRDs, by appending an Amendments note to each." Row 340: add "PRD-00 and PRD-16 included".
  BUILD-PROMPT step 5: "updates the remaining PRDs and the build plan (PRD-16 and PRD-00 'Build order') by appending an Amendments note to each, never editing this prompt".
  PRD-16 line 119: "runs after the last slice of each PRD merges, and after any merge that changes a seam; runs in the integrator lane; appends Amendments notes (PRD-00 row 339)".

## G2-05 (major)
- Sources: X3-5, P01-18
- Section and lines here: Phase 0 step 3 (523-525)
- Same id also in: PRD-16-orchestration-acceptance, PRD-01-record-format
- Ledger item or rule: PRD-00 merge rule (586-597) deadlocks otherwise; file ownership
- Change:
  PRD-16 section 2 step 5, replace the `members` bullet (PRD-00 phase 0 step 3 the same): "Add each new crate to `members` in the merge that creates it (PRD-01 adds `goalcup-format`, PRD-02's scaffold adds `goalcup`, PRD-08 adds `goalcup-verify`); the integrator applies that one `Cargo.toml` line inside the merge. The five old crates move to `exclude` in this commit."
  PRD-01 Must not touch, add: "the root `Cargo.toml` and `Cargo.lock` (the integrator adds `crates/goalcup-format` to `members` inside this PRD's first merge, PRD-16 section 2 step 5)".

## G2-06 (major)
- Sources: X3-6, P00-11
- Section and lines here: Phase 0 step 5 (528-530), step 7
- Same id also in: PRD-02-recorder-core, PRD-05-masking, PRD-11-connectors-harnesses
- Ledger item or rule: PRD-00 merge rule 1 and file ownership
- Change:
  PRD-00 phase 0 step 5: "PRD-02: the scaffold: `Cargo.toml` with every dependency the PRDs named, `lib.rs` with the full module list, S1 (with `for_tests` and `reserve`), S4 and S8 as compiling stubs, and an empty placeholder `mod.rs` in every other module folder of `crates/goalcup/src/`. This is the one ownership exception; each owner replaces its own placeholder in its first merge." Add: "PRD-05: the S2 stub in `src/mask/` by H+1 (it stars every byte, G2-01). PRD-11: the S3 stub in `src/connector/` by H+1, before the host list."
  PRD-02 builder note 1: add "placeholder module roots for every other folder (PRD-00 phase 0 step 5)".
  PRD-05 note 1 and PRD-11 note 1: "replace the scaffold's placeholder with the real S2 (S3) stub by H+1".

## G2-07 (major)
- Sources: X3-7, P00-10
- Section and lines here: S3 'final shape' (423-441, line 428)
- Same id also in: PRD-10-cli-participant
- Ledger item or rule: PRD-00 seams rule; ruling R3-11 (secret_values)
- Change:
  PRD-00 S3: replace the code block with "The final shape is PRD-11 'Design' lines 315-346: `load`, `plan`, `Env`, `LaunchPlan { connector, harness_version, program, argv, env, home, lock, before, after, hosts: HostList, native, mask_rules, secret_values, notes, refusal }`, `finish_env`; plus `HostList::decide`, `HostList::ca_subtrees` and `hosts::check` as below."
  PRD-10 line 623: use `connector::load` then `plan(&connector, &env, home)` (Amendments note if after H+0).

## G2-08 (major)
- Sources: X3-8, P02-23
- Section and lines here: Seams (347-366)
- Same id also in: PRD-11-connectors-harnesses, PRD-02-recorder-core
- Ledger item or rule: Seam named but not defined; point 6 (notes reach the record)
- Change:
  PRD-00 Seams, add row "S17 `StartFacts` and the first control line: run, entry, deadline, server_time, local wall, workspace, connector ref, keep, build, `HostList`, `NativeSources`, mask rules, values, CA PEM, previous end, harness pid and start, `null_since`, `LaunchPlan.notes`; every type serializes with serde. Owner PRD-02; used by PRD-10, PRD-11, PRD-05; stable by H+1.5".
  PRD-11 S3: "`HostList`, `NativeSources`, `MaskRule` derive `Serialize` and `Deserialize`."
  PRD-02 control-line `start` row: list the S17 fields and add "`notes` from `LaunchPlan` (written with `Recorder::event` after `start` on boot 0 only)"; PRD-11 interfaces row: add `notes`.

## G2-16 (major)
- Sources: P00-2, P02-8, X1-3 (and L1 F6 of group 1)
- Section and lines here: 'Rules of the shape', null mode (175-178); morning report template
- Same id also in: PRD-02-recorder-core
- Ledger item or rule: Ledger R3 (locked: restarts fast, no failure budget), point 1
- Change:
  PRD-02 step 9: "After 30 s the watchdog tries a normal boot again, then every 60 s after a failed try; the wait never grows. Null mode is not a failure budget: it only spaces out boots that crash at once, so each crash does not reset the harness's connections (point 3). An exit by SIGKILL (as from the kernel's OOM killer) does not count toward the 3 fast exits." Defaults row: "retry after 30 s, then every 60 s".
  PRD-00 lines 175-178: replace "retries a normal boot on a growing timer" with the same rule and why (ledger R3). Add the rule to the morning report's "Decisions agents took".

## G2-17 (major)
- Sources: P00-3, P02-9 (and L2-19 of group 1)
- Section and lines here: 'What 1.0 is' item 2 (59-61); diagram WD box (128); 'Rules of the shape', watchdog (166-172); S8 (443-460)
- Same id also in: PRD-02-recorder-core
- Ledger item or rule: Ledger R3 ('a tiny watchdog that only restarts the recorder'), L5; also owner item O-4
- Change:
  PRD-00 after line 172 and PRD-02 after line 332, add: "Why this fits ledger R3: the watchdog is the `goalcup start` process the participant already runs. Spawning, shielding and ending the harness and the helper stops are today's launcher duties, not new ones. It holds the socket and the CA only so a restart keeps the same port and the same trust (R3: 'on the same port fast'). It never reads, masks, writes or uploads a recorded byte; all recording stays in the one recorder process (L5). This reading awaits the owner's confirmation (owner question, see O-4)." Add the reading to the morning report's "Decisions agents took".

## G2-18 (major)
- Sources: P00-6, P00-23, P05-4
- Section and lines here: Owner question 3 (709-712); 'Smaller rulings' PRD-05 and PRD-11 (471); second slice (550)
- Same id also in: PRD-05-masking
- Ledger item or rule: Ledger M1 (locked masking scope; owner said 'nothing more'); VISION 'a limit says what it guards'; also owner item O-2
- Change:
  PRD-00 owner question 3: change "Default: yes." to "Not in the ledger. Built in the second slice only once the ledger records the owner's yes; without it, PRD-05 builder step 6 is skipped and MORNING.md lists it as not built." Smaller rulings: add "The 32-value limit guards `Masker::new` start time and memory (PRD-05 A5); the count of values past it is shown to the participant."
  PRD-05: at the top of section 9 and in "Owner decisions" add "Not yet in the ledger (PRD-00 owner question 3). Built in the second slice only after the owner's yes; until then builder step 6 waits." Mark rows 17 and 20 and the Stripe pair the same.

## G2-19 (major)
- Sources: P00-7, P02-13, 06-21
- Section and lines here: S1 shape (382-383); phase 0 step 5 (528-530); consistency item 86 (934); ownership row Cargo.toml (293)
- Same id also in: PRD-02-recorder-core, PRD-06-uploader-compression
- Ledger item or rule: Ledger L6 (locked: legacy and test seams cut), L2; also owner item O-3
- Change:
  PRD-00 item 86 and PRD-02 Files it owns: "Pending the owner (O-3): ledger L6 cut the test seams. The plan asks to keep one compile-time `test-hooks` feature in `crates/goalcup` only, off by default, never in a release build (PRD-14 greps the shipped binary for `GOALCUP_TEST_`), because restart, guard and rotation cannot be proven from a real run in the gate's time. No `GOALCUP_TEST_*` variable is read by a release build. `goalcup-verify` gets no hooks unless PRD-08 names one it needs." Add it to PRD-00's owner questions with this as the default.
  PRD-06 `upload_backoff.rs`: "`policy.rs` takes `now` as an argument; this case is a unit test of that pure function, with no new `GOALCUP_TEST_*` hook."

## G2-28 (major)
- Sources: P04-2, L3-4 (group 1)
- Section and lines here: S8 seal bound (443-460)
- Same id also in: PRD-04-native-capture, PRD-02-recorder-core
- Ledger item or rule: Ledger R5 (no caps of our own), H8 (session files with no caps)
- Change:
  PRD-04 step 2: "Read every append file to the size seen in step 1's pass, and every unsettled snapshot as it is." Step 5: "Time guard: 180 s, only when the seal reason is `deadline`, `server_stop` or `signal` (today's finish budget, `control.rs:708`); it guards the end of the run window. On `harness_exit` and `seal_command`, finish reads every file to the size seen in step 1 with no time guard; status shows `sealing: reading session files` meanwhile." Defaults row: "180 s at `deadline`, `server_stop`, `signal`; none otherwise | Owner".
  PRD-02 section 11 step 4 and "The bound", PRD-00 S8: the same rule.

## G2-38 (major)
- Sources: X2-01, P00-20
- Section and lines here: 'Size budget' (753-777)
- Same id also in: PRD-13-cuts, BUILD-PROMPT, PRD-16-orchestration-acceptance
- Ledger item or rule: Ledger L2 (locked: less code, cut before you add); VISION 'Less code'
- Change:
  PRD-00 lines 775-777: replace "The 18,700 is above ... says why in the night log." with "Each row is a ceiling, not an estimate. A builder who would pass its row first cuts what serves no point; past the row by more than 10% needs a replan `decision` line that names the point the extra lines serve. Reviewer B checks each merge request against its row and against the fresh design's row (audit B, B1.11 and B2.12). Targets after the cuts of X2-02, X2-03, X2-07, X2-08, G2-24 and G2-25: Rust src about 18,100, Rust tests about 15,000." Rust row: "about 18,100 / about 15,000". Use one pad number: "about 1,350 to 1,450 (PRD-12's estimate wins)".
  PRD-13 table: the same two numbers.
  BUILD-PROMPT reviewer B: add "and the PRD's size row (PRD-00 Size budget)".
  PRD-16 Replan: add "and posts `wc -l` of the merged PRD against its row".

## P00-12 (major)
- Sources: P00-12
- Section and lines here: 'One system', dodger bullet (98-102); done item 1 dodger bullet (612-615)
- Ledger item or rule: Ledger T3, T4 (locked); builder misread
- Change:
  Done item 1 dodger bullet, start with: "The dodger case is simulated: the pad passes a copy of the host list with the harness's model host marked `pinned` (`--hosts`, S16; PRD-12 section 2). It does not change the harness's tier." Line 101: "and, for a real dodger, the tier 'not supported'".

## P00-8 (major)
- Sources: P00-8
- Section and lines here: Phases table, phase 3 (563); ownership row PRD-13 (337)
- Same id also in: PRD-16-orchestration-acceptance
- Ledger item or rule: Ledger L3 (locked: delete the old crates in the same build), L4
- Change:
  PRD-00 phase 3: replace "If PRD-17's gate is not green by H+10 there is no cut ..." with "The cut runs as soon as PRD-17's gate is green, at any time before REPORT_AT. Until then old crates stay on disk, out of the workspace, and phases 4 to 6 still run. If the gate is never green, MORNING.md names L3 as not met and why." Row 337: "by H+10" becomes "once PRD-17's gate passes, before REPORT_AT". PRD-16 line 226 and S7 the same.

## P00-9 (major)
- Sources: P00-9
- Section and lines here: Definition of done (601-628)
- Ledger item or rule: Ledger R6, R7, H3, H6 (locked); points 1, 6
- Change:
  Done item 2, add: "After each kill, `status.json`, the end screen and the next heartbeat show the off time, and the gaps' total equals it." Add item 8: "`watchdog_null.rs` passes and a heartbeat sent in null mode says `failed` (PRD-02, PRD-07)." Add item 9: "PRD-03's `h2-parallel.sh` (acceptance 3) and `long-tunnel.sh` pass in a pad box against the release build."

## P03-5 (major)
- Sources: P03-5
- Section and lines here: Phase 1 table, third slice row (551)
- Same id also in: PRD-03-proxy, PRD-01-record-format
- Ledger item or rule: Ledger H6; builder misread (a decrypted connection cannot fall back to a tunnel)
- Change:
  PRD-03 section 9: remove `h2_unsupported` from the tunnel reason list and "11 `h2_unsupported`" from the listed-tunnel GAP list. Section 6, add: "`h2_unsupported` is never a tunnel reason: the connection is already decrypted, so it keeps relaying unparsed (5a)."
  PRD-00 third slice: "anything else is GAP `h2_unsupported` and the connection relays unparsed (PRD-03 5a)". PRD-01: drop `h2_unsupported` from the `conn` `reason` list.

## X2-03 (major)
- Sources: X2-03 (parts b, c, d; the sleep lock, part a, is owner item O-5)
- Section and lines here: Size budget, CLI row
- Same id also in: PRD-10-cli-participant
- Ledger item or rule: Ledger L2; VISION 'Less code' and 'No caps of our own'
- Change:
  PRD-10 section 8: keep `--prompt` (owner question 2 default yes) but drop the 64 KiB refusal: "reads the file as UTF-8; no size limit". Remove the 64 KiB case from `cli_push.rs` and the `--prompt` size row from defaults.
  Line 415: delete "the model hosts of the chosen connector reachable (TCP, a warning only: F3);" and "on battery (H11);".
  Files table: `text.rs` 520 becomes about 420; recount the total (about 2,600 if the sleep lock also goes, O-5). PRD-00 CLI row follows.

## 06-23 (minor)
- Sources: 06-23 (the review's note for PRD-00 and PRD-07)
- Section and lines here: 'Smaller interface rulings' (469)
- Same id also in: PRD-07-worker-server
- Ledger item or rule: Ruling R3-2
- Change:
  Where they say `goalcup upload` sends "the seal's totals read from the record", say "from `pipeline::entry_totals` (R3-2)".

## G2-12 (minor)
- Sources: X3-17, P00-14, P05-9, P02-16, 06-12
- Section and lines here: Line 681; review answers (909, 925, 956, 957, 962, 983, 984, 1017, 1027, 1040)
- Same id also in: PRD-16-orchestration-acceptance, PRD-05-masking, PRD-02-recorder-core, PRD-06-uploader-compression
- Ledger item or rule: Clarity; ledger is the source of truth
- Change:
  Replace revision 2 owner question numbers with PRD-00's current eight. Map: old 1, 2, 4, 6, 7 are question 1; old 8, 9, 10, 11, 13 are question 2; old 12 is 3; old 3, 14, 15 are 4; old 5 is 5; old 16 is 6; old 19 is 7; old 17 and 18 are 8. Write "owner question N (old M)" where a reader may meet the old number.
  PRD-00 line 681: "owner question 2". PRD-00 review answers: write each as "old N (now M)".
  PRD-02: "owner question 2" (line 37); line 120: "This is PRD-00's S1 block (revision 3, rulings R3-1 and R3-2)"; header: "Index: PRD-00 revision 3".
  PRD-06: every "owner question 6" and "owner question 2" becomes "PRD-00 owner question 1"; in "Owner decisions it carries out" tag each line with its ledger id: F2 (zstd), U1 (prune, small chunks, capped backoff that resets on progress), R5 (no caps), R6 and R7 (written counts, `failed`), L5 (one process), B4 (staging).

## G2-13 (minor)
- Sources: X3-18, P00-13
- Section and lines here: Review answers, consistency item 84 (932)
- Ledger item or rule: Ruling R3-7; 'Numbers and names' (279)
- Change:
  "Accepted. 95 to 99 rate-limited calls, a count (R3-7)."

## G2-15 (minor)
- Sources: X3-28, P02-2 (part)
- Section and lines here: Seams (347-366); R3-2 (863)
- Same id also in: PRD-02-recorder-core, PRD-06-uploader-compression
- Ledger item or rule: Seam named but not defined; ruling R3-2
- Change:
  PRD-00 Seams, add "S18 `upload::spawn(ctx) -> UploadHandle`, `UploadHandle::finish(wrap_up, seal_facts { reason, workspace_head, totals })`, owner PRD-06, used by PRD-02, stable by H+3". R3-2 line 863 cites S18.
  PRD-02 section 11 step 6 and 9.2 step 5: "hand over with `UploadHandle::finish(wrap_up, seal_facts)`, `seal_facts` holding `reason`, `workspace_head` and `totals`".
  PRD-06 Files and Interfaces: the same signature.

## G2-25 (minor)
- Sources: P03-13, X2-04 (part), X3-14 (part)
- Section and lines here: Size budget, Proxy row
- Same id also in: PRD-03-proxy
- Ledger item or rule: Ledger L2 (less code); H6 is met by `PreferH1` serving h2-only clients
- Change:
  PRD-03 section 4 item 4: replace "A switch `Alpn::Mirror` ... with a night-log line." with "Mirroring the client's full list is not built in 1.0; `PreferH1` is the only ALPN mode. It may be added after the build once HTTP/2 rows pass." `proxy_tls.rs`: the matrix under `PreferH1` only. Files table total "about 4,400" becomes "about 4,250" (with G2-24); PRD-00 Proxy row the same.

## P00-15 (minor)
- Sources: P00-15
- Section and lines here: 'AGENTS.md: what stands and what changes' (658-684)
- Ledger item or rule: Ledger section 7
- Change:
  Lines 671-672: "A test stays if it proves one of the seven points (ledger section 7)." Line 666: "Record raw, interpret later: the recorder captures bytes; the verifier and the usage reader judge them." Line 683: "because ledger section 7 lists the rule changes and this is not one of them." Move line 667 under a "Tests" note.

## P00-16 (minor)
- Sources: P00-16
- Section and lines here: 'Where the build runs', Conductor (477)
- Ledger item or rule: Clarity (ledger B2 names no number)
- Change:
  "at most 2 agents at once each (the environment's per-workflow limit, BRIEF3); several lanes in parallel get past it (ledger B2)."

## P00-17 (minor)
- Sources: P00-17
- Section and lines here: 'Where the build runs', Staging Worker (483)
- Ledger item or rule: Ledger B4
- Change:
  "approved with the staging test approval key and the owner's member id (ledger B4)."

## P00-18 (minor)
- Sources: P00-18
- Section and lines here: Ownership, connectors row (326)
- Ledger item or rule: Code fact (checked: `crates/goalcup-protocol/pins/` is tracked; no top-level `pins/`)
- Change:
  Replace "`pins/`" with "`crates/goalcup-protocol/pins/` (goes with the crate)".

## P00-19 (minor)
- Sources: P00-19
- Section and lines here: Today (51-52); consistency item 14 (901)
- Ledger item or rule: Code fact
- Change:
  "exits 1 with about 95 findings, all in `docs/plans/`".

## P00-21 (minor)
- Sources: P00-21
- Section and lines here: PRD table, PRD-13 row (221)
- Ledger item or rule: Ledger L6
- Change:
  Add "retired versions" after "legacy seams".

## P00-22 (minor)
- Sources: P00-22
- Section and lines here: Done item 2 (616-617)
- Ledger item or rule: Provable acceptance
- Change:
  "the harness unaffected" becomes "the harness finishes its turn with no participant action and its own log shows no failed call it did not retry (PRD-02 R1)".

## P01-19 (minor)
- Sources: P01-19
- Section and lines here: Defaults list
- Same id also in: PRD-01-record-format
- Ledger item or rule: Ledger status words (default against locked)
- Change:
  PRD-01 line 359: "Accepted as a planner default (PRD-05); listed in PRD-00's defaults for the owner." Add "same-length masking (the length of a masked value stays visible)" to PRD-00's defaults list.

## P04-25 (minor)
- Sources: P04-25 (the PRD-05 A4 part is G2-36)
- Section and lines here: Review answer (977)
- Same id also in: PRD-13-cuts, PRD-14-tests-gate
- Ledger item or rule: Builder misread in other PRDs
- Change:
  PRD-00 line 977: "Routed to PRD-04 (hourly copies when changed; no copy in `close_dead_entry`, PRD-02 9.4)". PRD-13 lines 324 and 464: 780 src lines. PRD-14 line 93: drop `native_capture.rs`.

## X1-10 (minor)
- Sources: X1-10
- Section and lines here: Acceptance runs table (817)
- Ledger item or rule: Point 6 (plain, true numbers); run report row 4
- Change:
  Change the run 1 cell "none written, 71 min lost" to "23 written (22 `queue_full`, 1 `limit_exceeded`); none covered the last 71 min".

## X2-11 (minor)
- Sources: X2-11
- Section and lines here: 'Review answers' section and fully ruled 'Asks' sections
- Same id also in: PRD-01-record-format, PRD-02-recorder-core, PRD-03-proxy, PRD-04-native-capture, PRD-05-masking, PRD-06-uploader-compression, PRD-07-worker-server, PRD-08-verifier-checks, PRD-09-usage-research-stats, PRD-10-cli-participant, PRD-11-connectors-harnesses, PRD-12-test-pad, PRD-13-cuts, PRD-14-tests-gate, PRD-15-docs-decisions, PRD-16-orchestration-acceptance, PRD-17-lessons-old-code
- Ledger item or rule: Ledger L2; point 6 (plain, true text). Apply last, after every other fix, so line numbers in the fix lists stay valid
- Change:
  Move every "Review answers" section and every fully ruled "Asks" section into one file `docs/plans/2026-10-30-v1.0/REVIEW-HISTORY.md`, verbatim, one heading per PRD. Before moving, copy any ruling still binding into the PRD body. Each PRD keeps one line at its end: "Review history: REVIEW-HISTORY.md, section PRD-NN." The replan keeps new asks in the Amendments note only.

## X3-27 (minor)
- Sources: X3-27
- Section and lines here: Seams S10 (359)
- Ledger item or rule: Order of work
- Change:
  S10 "stable by H+3, a stub type published with PRD-06's first merge"; PRD-02 builds its `server` line in the second slice.
