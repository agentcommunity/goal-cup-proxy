# Fix list for PRD-00-index.md (group 1)

From the alignment reviews L1 to L7, triaged against OWNER-DECISIONS.md (the ledger wins).
Files are under docs/plans/2026-10-30-v1.0/. Line numbers are from branch smart/intelligent-newton-0avrpi.
An item with several files carries the same id and the same change text in each file's list; make only the part for this file.

Counts: 5 blocker, 9 major, 6 minor.

## L2-01 (blocker)

- Section and lines: PRD-02 section 2 rule 1 (152-155), section 11 step 2 (533-535) and steps 3-4 (536-538); PRD-00 S1 rules (397), R3-3 (864)
- Change: PRD-02:154: 'A call after the seal's stop is a no-op that only counts (section 11).' becomes 'After the seal's stop, calls on proxy streams are no-ops that only count (section 11); the native thread's calls stay admitted until `NativeReader::finish` returns.' PRD-02:533: 'Every S1 call after this point is a no-op that only counts' becomes 'Every S1 call from a relay task after this point is a no-op that only counts. Calls from the native thread stay admitted until step 4 returns.' PRD-00:397: 'A call after the seal is a no-op.' becomes 'A relay call after the seal's stop is a no-op; native calls stay admitted until `NativeReader::finish` returns (R3-3).' Add a case to `pipeline_seal.rs` (PRD-02 730-732): a native file that grows during the seal and a `final` set are both in the record after the seal.
- Ledger item or rule: R1, R4 (locked), point 2; PRD-00 R3-3 not applied
- Source: align-L2 L2-01
- Also in: PRD-02-recorder-core

## L4-1 (blocker)

- Section and lines: PRD-00 owner question 3 (709-712), PRD table row (213), status (238-239), second slice (550); PRD-05 owner decisions (34-37), rule rows 17 and 20 (129, 132), section 9 (334-395), section 10 rows (405-406, 409, 415-417), section 12 (453-465), overridable defaults (594, 597), builder step 6 (621); PRD-11 `secret_files` and `secret_values` (151, 339-340, 361-366, each connector's `secret_files` line); PRD-10 mask values call (153), `--prompt` risk row (732); PRD-12 planted `sk_test_`, `whsec_` and the minute 5 canary (257-259, 265-266, 598, 806); PRD-15 D80 (340)
- Change: Build ledger M1 as written by default; the extra masking waits for the owner (owner list OW-01). PRD-00 question 3: 'Default: yes.' becomes 'Default: no until the owner answers. Built only if the owner adds it to the ledger as M1a.' PRD-05: in 'Owner decisions' replace 'default yes, built in the second slice' with 'default no; not built unless the owner says yes'; mark rows 17 (Stripe test keys) and 20 (`whsec_`) and section 9 sources 2 to 4 'only with owner yes'; the default build keeps the `GH_TOKEN` exact value only (as today); section 12's default text is the 'no' variant (463-465). PRD-11: `secret_files` and `secret_values` are not read in the default build (the data rows may stay). PRD-10:153: only `mask::workspace_values` for `GH_TOKEN`. PRD-12: plant `sk_test_`, `whsec_` and the canary only when the owner said yes; otherwise drop them from `.env.local` and the known-values file. PRD-15 D80: remove 'and the key values from the four sources ... (default, PRD-00 question 12)' unless the owner says yes. If the owner says yes before H+0, none of this changes and the second slice builds it as planned.
- Ledger item or rule: M1 (locked: the masking list); B2a (a locked item changes only with the owner)
- Source: align-L4 L4-1
- Also in: PRD-05-masking, PRD-11-connectors-harnesses, PRD-10-cli-participant, PRD-12-test-pad, PRD-15-docs-decisions

## L5-01 (blocker)

- Section and lines: PRD-12 section 7 'Per run' row (328), risk row (687), overridable defaults (699); PRD-16 section 9 (316-317); PRD-09 owner decisions (35-36); PRD-00 'Live spend pricing' row (578)
- Change: PRD-12 section 7, 'Per run' row: value '10 USD per run (ledger T6)'; 'What happens': 'During every run, `pad` runs `goalcup-verify usage <open entry> --json` in the box every 5 minutes with `sandbox exec`, prices the totals with the flat rate in `pad.toml`, and runs `goalcup seal --yes` at 10 USD. The row gets `over_cap: true` and the checks run on what was recorded. After the run the ledger line is written as now.' Risk row 687 names this live check. PRD-16:316: pad rows use the same check as 317, every 5 minutes for both. PRD-09:35-36: 'No rule, cap or budget reads these numbers.' becomes 'No contest rule reads these numbers (D26). The one reader that acts on them is the pad's test spend check (ledger T6, PRD-16 section 9), which reads `usage --json` on the open local entry in the box.' PRD-00:578 'Live spend pricing' row: 'Cut' becomes 'the pad's flat-rate check on `usage` (PRD-16), no `prices.toml`'. A `smoke` run has no lower cap of its own (see L5-02/L6-09).
- Ledger item or rule: T6 (locked: 10 USD per run); B8
- Source: align-L5 L5-01 (with align-PRD-09 P09-10 for the PRD-09 line)
- Also in: PRD-12-test-pad, PRD-16-orchestration-acceptance, PRD-09-usage-research-stats

## L5-02/L6-09 (blocker)

- Section and lines: PRD-12 section 7 'Whole build' row (329), overridable defaults (700); PRD-00 (639) and owner question on build spend (731-733); PRD-16 section 9 rows (316, 318-319), 'Why 400' (322-326), S4 (335), W4 (352), defaults (543-544)
- Change: No whole-build cap and no 2 USD smoke cap; only the 10 USD per-run cap stops a run. PRD-12:328 value '10 USD per run (ledger T6); a `smoke` run is expected near 2 USD, an estimate, not a cap'. PRD-12:329 and PRD-16:318: 'No cap (ledger T6). The scribe writes a `spend` line at 200 and 400 USD by the pad's ledger estimate and tells the owner; the report shows it; runs go on.' Remove 'A new run does not start past it' (PRD-12:329) and 'No new run starts' (PRD-16:318). PRD-16:319: 'No cap. A `spend` line at 75 and 150 USD estimated sandbox cost; builds go on.' PRD-16:322-326: 'The expected model spend is about 200 USD: phase 1a and selftest about 35, phase 4 up to 180 with reruns, phase 5 about 25 runs at 1 to 2 USD, acceptance up to 30.' Delete S4 (335). PRD-16:352 W4: 'A run past 1.5 times its 10 USD cap, a provider dashboard that keeps rising with no run going, or a sandbox that will not stop.' PRD-16:543-544 and PRD-12:700: 'Build spend report lines | 200 and 400 USD model, 75 and 150 USD sandboxes | Owner'. PRD-00:639 and the build-spend owner question (731-733): same report-line wording.
- Ledger item or rule: T6 (locked: cost otherwise not a constraint); T1, B3 (mass testing of all 15)
- Source: align-L5 L5-02, align-L6 L6-09
- Also in: PRD-12-test-pad, PRD-16-orchestration-acceptance

## L6-05 (blocker)

- Section and lines: PRD-16 Conductor row (81), X6 and X7 (278-279), N6 (502), builder notes (590); PRD-00 'Where the build runs' (485-486); PRD-12 section 1 table (140), section 3 rows 0:27-0:28 (171-172), keys (320-322), section 8 (342-349), selftest (505); PRD-07 'How a watcher proves it' (570-574); PRD-09 section 1 (98-100), `scripts/usage-job.sh` (442), risks (579-582)
- Change: The conductor only fetches, approves and runs read-only queries; every `goalcup-verify` run happens in a sandbox. PRD-16:81: '`scripts/pad/pad` (conductor half), `fetch-run`, `goalcup-verify` for checks. It never compiles or tests' becomes '`scripts/pad/pad` (conductor half: sandboxes, approval, fetch, read-only queries), `fetch-run`. It never compiles, tests or runs `goalcup-verify`; checks run in the pad box before it is removed (PRD-12 section 8)'. PRD-00:485-486: '`fetch-run` and server checks run there.' becomes '`fetch-run` and the read-only queries run there. The fetched copy is copied into the pad box and `goalcup-verify` runs there (ledger B1).' PRD-12:140: 'approves, fetches, checks, writes the row' becomes 'approves, fetches, copies the fetched entry and the per-run known-values file into the box, has `pad-run` run the checks there, writes the row from their JSON'. PRD-12:171-172: 0:27 conductor 'copy out ...; run `fetch-run`; `sandbox copy` the fetched entry and the per-run known-values file in; `sandbox exec` the four commands of section 8; copy out their JSON; `sandbox remove pad-<id>`'; 0:28 'write `row.json` and `report.txt`; scan the pad's output folder; append to the ledger'. PRD-12:320-322: 'The per-run known-values file holds the values that entered this box (its keys, its repo key), every planted value and the canary; it is made on the conductor, 0600, copied into the box for the checks, and removed with the box.' PRD-12:342: 'The conductor runs, for each run:' becomes '`pad-run` runs in the box, after the conductor copied in the fetched entry and the per-run known-values file:'. PRD-12:505: 'on the conductor' becomes 'in a build box'. PRD-07:570-574: 'run `goalcup-verify check --server <out>/<entry> --local <entry dir>`' becomes 'copy `<out>/<entry>` into the pad box and run `goalcup-verify check --server ... --local ...` there'. PRD-16 X6, X7 (278-279) and builder notes (590): add 'in the run's pad box'. PRD-16 N6 (502): add 'run in one check box from the build snapshot, `--non-persistent`, holding the full known-values file for the scan only and removed right after (a `decision` line: the one step where all keys meet a box)'. PRD-09:98-100: 'During the build the conductor does only the steps that need credentials: it lists sealed entries (read-only database), runs `fetch-run`, copies the fetched folder into a build box, and puts the three output files into `goalcup-usage-staging`. `goalcup-verify usage` itself runs in the build box, which holds no credential (ledger B1). The conductor never compiles or runs the reader. The production job is the owner's to start, on an organizer machine.' PRD-09:442 `scripts/usage-job.sh` and risks 579-582: the same split.
- Ledger item or rule: B1 (locked: builds, tests and real runs in Vercel Sandboxes; the cloud session orchestrates only)
- Source: align-L6 L6-05 (with align-PRD-09 P09-1)
- Also in: PRD-16-orchestration-acceptance, PRD-12-test-pad, PRD-07-worker-server, PRD-09-usage-research-stats

## F1 (major)

- Section and lines: PRD-16 section 5 'Cut-off H+10' (226-228) and S7 (338); PRD-13 section 5 (175-177) and risk 'The cut is skipped' (531); PRD-00 phase 3 row (563)
- Change: PRD-16:338 S7 'What happens': 'No cut (section 5)' becomes 'No cut yet (section 5). Calls the owner (W6): locked L3 cannot hold on time. The lessons lane keeps working; the cut lands in the same build as soon as the gate is green, before `MORNING.md`.' Same sentence in PRD-16:226-228, PRD-13:175-177 and :531. PRD-00:563: 'the end-of-build report says so' becomes 'the owner is called under W6 and the cut lands as soon as the gate is green'.
- Ledger item or rule: L3, L6 (locked: delete the old crates in the same build)
- Source: align-L1 F1
- Also in: PRD-16-orchestration-acceptance, PRD-13-cuts

## F6/L2-06 (major)

- Section and lines: PRD-02 section 8 step 9 (365-374), defaults (794), risks (755-756); PRD-00 rules of the shape (175-178)
- Change: PRD-02:369-370: 'After 5 minutes the watchdog tries a normal boot again ...; each failed try doubles the wait, up to 60 minutes' becomes 'After 5 minutes the watchdog tries a normal boot again ...; each failed try doubles the wait, up to 10 minutes.' Line 794: 'retry after 5 min, doubling to 10 min'. Risks 756: 'at most about six tries an hour'. PRD-00:175-178: 'on a growing timer' becomes 'on a timer that starts at 5 minutes and never passes 10 minutes'. (A 60 s retry, as align-L1 F6 offered, would reset every harness connection once a minute while a cause like a full disk lasts; that slows the harness, R2. Ten minutes keeps the restart fast without that.)
- Ledger item or rule: R3 (locked: restart fast, no failure budget), R2
- Source: align-L1 F6, align-L2 L2-06
- Also in: PRD-02-recorder-core

## L3-3 (major)

- Section and lines: PRD-00 Definition of done (601-628), third slice (551); PRD-12 section 9 `--release-check` (437-441)
- Change: PRD-00, after done item 2 add: '2a. **HTTP/2 works:** PRD-03 acceptance 3 passes in a pad box (an `h2`-only client to a listed host: one `conn` with `alpn: "h2"`, four decoded request and response pairs, zero `h2_unsupported`), and `tests/proxy_h2.rs` is green in `gate full`.' PRD-12:438: after 'and the Codex kill row are PASS' add 'and the `h2-parallel.sh` exec row of PRD-03 acceptance 3 passed'.
- Ledger item or rule: H6 (locked: HTTP/2 is supported)
- Source: align-L3 L3-3
- Also in: PRD-12-test-pad

## L4-3 (major)

- Section and lines: PRD-05 section 4 rule (193-194), why (205-209), cost (229), overridable defaults (592); PRD-00 'Owner questions'
- Change: Default the left guard class to the ledger's word boundary, `[A-Za-z0-9_]`; `-` is added only if the owner says yes (owner list OW-02). PRD-05:193-194 and 592: class `[A-Za-z0-9_]`; 'Who may change it: Owner' becomes 'Owner; ledger M1 says a letter or digit (a word boundary)'. PRD-05 'What it costs' (229): name `-` as the one possible addition and why (base64url blobs hold `-`). PRD-00 owner questions, add: 'Left guard class: `[A-Za-z0-9_]` (the ledger) or `[A-Za-z0-9_-]` (also `-`, so no guarded rule matches inside a base64url blob). Default: `[A-Za-z0-9_]`.'
- Ledger item or rule: M1 (locked: left word boundary; key after a letter or digit may be missed)
- Source: align-L4 L4-3
- Also in: PRD-05-masking

## L4-4 (major)

- Section and lines: PRD-05 section 7 step 2 (310-312), interfaces row PRD-09 (500), acceptance A3 (538), overridable defaults (600); PRD-00 'Owner questions'
- Change: Delete the sentence 'Exception: a name that contains `ratelimit` or `rate-limit` is not masked by the name rule ... in line with dropping the Codex rate-limit rules.' (310-312), the 'Rate-limit headers stay readable (section 7)' clause in the PRD-09 row (500), 'Rate-limit headers and' in A3 (538), and the row at 600. Add to PRD-00 owner questions: 'Unmask rate-limit headers for research? Default: no' (owner list OW-03).
- Ledger item or rule: M1 (locked: keep every secret rule)
- Source: align-L4 L4-4
- Also in: PRD-05-masking

## L5-03 (major)

- Section and lines: PRD-12 owner decisions (33-34), tier rule (427-430), `--release-check` (437-438), P3 (639), overridable defaults (708); PRD-00 done item 1; PRD-16 phase 4
- Change: PRD-12:427-430: 'the latest `gateway` and `dodger` runs, at least `smoke`, are PASS' becomes 'the latest `gateway` and `dodger` runs on `full` are PASS'. Same edit in 33-34, 437-438 ('nine core `full` rows'), P3 639 ('`direct`, `gateway` and `dodger` on `full`') and the 'Tier needs' default 708. Keep `smoke` for phase 1a, the build's short tests and watcher reproduction (section 13 step 3). PRD-16 phase 4 and PRD-00 done item 1 say the same.
- Ledger item or rule: T5 (locked pad workload), T4, B3
- Source: align-L5 L5-03
- Also in: PRD-12-test-pad, PRD-16-orchestration-acceptance

## L6-01 (major)

- Section and lines: BUILD-PROMPT section 1 item 5 (24-26), section 4 table (59-73); PRD-16 section 1 'Lanes' (91); PRD-00 ownership row `BUILD-PROMPT.md` (339), R3-10 (871)
- Change: BUILD-PROMPT:24-26: 'phases, merge rule, real runs, acceptance, autonomy, stop rules, wake list, morning report. Follow it, except where this prompt or the ledger says otherwise.' becomes 'lanes, phases, merge rule, real runs, acceptance, autonomy, stop rules, call list, end-of-build report. Follow it. Where this prompt and PRD-16 differ, PRD-16 wins (PRD-00 R3-10); where either differs from the ledger, the ledger wins. Re-read PRD-16's Amendments notes after every replan.' BUILD-PROMPT:59-73: replace the sentence at 61 and the table with 'Start the twelve lane workflows of PRD-16 section 1 in parallel, with the PRDs and start times that table gives. Each lane edits only the files the PRD-00 ownership map gives its PRDs.' PRD-16:91: '(BUILD-PROMPT section 4)' becomes '(ledger B2; all lanes run in parallel; this table replaces BUILD-PROMPT section 4's, PRD-00 R3-10)'. PRD-00:871 R3-10: 'Ledger B1, B2 and B2a are locked and win over this file, PRD-16 and BUILD-PROMPT.md. Within them the conductor starts PRD-16 section 1's twelve lanes with its start times and roles. BUILD-PROMPT section 4's table differs (Masking inside Format and core, Judge after PRD-01, Participant after PRD-02, PRD-14 inside Cut and docs, no Real runs lane) and is replaced by a pointer to PRD-16 before H+0. BUILD-PROMPT's per-PRD pipeline (plan, parallel builders in worktrees, at least two independent reviewers, fix, merge, replan) is the ledger's and stands.' Applied by: 'PRD-16; BUILD-PROMPT before H+0'. PRD-00:339: 'Edited before H+0 only (R3-10). Not edited during the build: the conductor holds the pasted text, so a later edit reaches no one. The replan changes the build plan through Amendments notes on PRD-16 (ledger B2a). Where the prompt and PRD-16 differ, PRD-16 wins; the ledger wins over both.'
- Ledger item or rule: B2, B2a (locked); settles the known R3-10 conflict
- Source: align-L6 L6-01
- Also in: BUILD-PROMPT, PRD-16-orchestration-acceptance

## L6-02 (major)

- Section and lines: PRD-16 section 1 Replan row (119); BUILD-PROMPT section 4 step 5 (90-93); PRD-00 ownership rows (339-340)
- Change: PRD-16:119 Replan row: Count 'one per merge of a PRD branch (each slice), as the last step of that PRD's lane pipeline'; Does 'Reads review findings, test and real-run results; appends an Amendments note to each remaining PRD it affects and to PRD-16 itself, which is the build plan (lanes, order, phases, runs, caps within the ledger); writes an `amendment` night-log line. Reviewer A of that merge reads the note against the ledger; the integrator commits it on integration. The conductor re-reads PRD-16's Amendments after each one and applies them to the lane queues'; Never 'Rewrites a section a builder is working from, edits BUILD-PROMPT.md, or changes a locked ledger item (that calls the owner, W6)'. BUILD-PROMPT:90-93 step 5: '**Replan.** After each merge of a PRD branch, an improvement agent reads what was learned (review findings, test results, real-run results) and appends Amendments notes to the remaining PRDs it affects and to PRD-16, the build plan, in a commit on the integration branch, with a night-log line. You re-read PRD-16's Amendments and adjust the lanes. This prompt is not edited during the build. A note may not change a locked ledger item; that calls me (W6).' PRD-00:340: after 'at the end of a PRD' add ', PRD-00 and PRD-16 included,'.
- Ledger item or rule: B2a (locked: updates the remaining PRDs and the build plan)
- Source: align-L6 L6-02
- Also in: PRD-16-orchestration-acceptance, BUILD-PROMPT

## L7-01 (major)

- Section and lines: PRD-12 section 1 output (132), 'Files it owns' (559-584); PRD-13 'Files it owns'; PRD-00 ownership table (no `.gitignore` row). Checked: repo `.gitignore` has no `pad-out` line today
- Change: PRD-12 'Files it owns', Change: add '`.gitignore`: add `/pad-out/` in the pad's first commit (phase 0), before any run writes there.' PRD-13 'Files it owns': add '`.gitignore`: remove the `spikes/*/out/` and `spikes/wire-pty/out/` lines in the commit that deletes `spikes/`.' PRD-00 ownership table: add a `.gitignore` row naming PRD-12 (the `pad-out` line) and PRD-13 (the `spikes` lines).
- Ledger item or rule: Section 7 'Never commit ... Stands'; point 4
- Source: align-L7 L7-01
- Also in: PRD-12-test-pad, PRD-13-cuts

## F5/L2-19 (minor)

- Section and lines: PRD-02 'Owner decisions' (34-35) and section 8 first paragraph (328); PRD-00 'Owner questions' (686 on)
- Change: PRD-02:35: 'A tiny watchdog that only restarts the recorder is allowed.' becomes 'A tiny watchdog is allowed (R3); in 1.0 it is the `goalcup start` or `continue` process, with the jobs in section 8 (PRD-00 owner question 9).' PRD-02 section 8 first paragraph, add: 'Its only recording duty is the restart loop. Everything else it does is what `goalcup start` already did (spawn, terminal, deadline, run CA, helpers); none of it records or touches recorded bytes.' PRD-00 owner questions, add 9: 'The watchdog is the start process itself. Beside restarting the recorder it holds the listen socket and the run CA, runs helper stops and starts and ends the harness, because those must survive a recorder crash. Default: yes.' (Owner list OW-05.)
- Ledger item or rule: R3, L5 (locked wording 'only restarts the recorder')
- Source: align-L1 F5, align-L2 L2-19
- Also in: PRD-02-recorder-core

## F10 (minor)

- Section and lines: PRD-00 'What 1.0 is' (56-58)
- Change: After 'A fresh recorder crate' add '(`crates/goalcup`; `crates/goalcup-format` is the format library it shares with `crates/goalcup-verify`, the verifier binary)'.
- Ledger item or rule: L3 (clarity: one new recorder crate)
- Source: align-L1 F10

## L3-5 (minor)

- Section and lines: PRD-00 'One system' (98-102); PRD-13 section 3 (103)
- Change: PRD-00 after line 102 add: 'Words: the ledger's "proxy dodgers (Amp, Cursor)" (H5) are the plan's vendor backends: listed, decrypted, recorded, tier "verified" or "not verified". Only a client that ignores `HTTPS_PROXY`, pins, or refuses the CA on its model host gets "not supported" (T3).' PRD-13:103: 'and the tier "not supported"' becomes 'and, when the harness's own model host is affected, the tier "not supported" (T3); Amp and Cursor are vendor backends, not this case'.
- Ledger item or rule: H5, H10 (wording); T3
- Source: align-L3 L3-5
- Also in: PRD-13-cuts

## L3-10 (minor)

- Section and lines: PRD-00 third slice row (551)
- Change: 'anything else is `h2_unsupported` and a tunnel' becomes 'anything else stops parsing for that connection with GAP `h2_unsupported` while it keeps relaying (PRD-03 section 5a)'.
- Ledger item or rule: H6 (clarity)
- Source: align-L3 L3-10

## L6-11 (minor)

- Section and lines: PRD-16 section 1 Reviewers row (117); PRD-00 lanes (512)
- Change: PRD-16:117 Count: 'at least two per merge request'; Does: append 'Each is a fresh agent, not the builder, and does not see the other reviewer's findings before writing its own.' PRD-00:512: 'two reviewers' becomes 'at least two independent reviewers'.
- Ledger item or rule: B2 (locked)
- Source: align-L6 L6-11
- Also in: PRD-16-orchestration-acceptance

## L7-07 (minor)

- Section and lines: PRD-00 'Consequences' (681-683); PRD-15 (254-256); PRD-16 stop table (299)
- Change: PRD-00:681-683 and PRD-15:254-256: 'because the ledger allows two changes only' becomes 'because ledger section 7 changes only the rows it lists; ending the harness is today's behaviour, raised as owner question 2 (old 11)'. PRD-16:299: 'beyond PRD-00's two changes' becomes 'beyond the changes ledger section 7 lists'.
- Ledger item or rule: Ledger section 7
- Source: align-L7 L7-07
- Also in: PRD-15-docs-decisions, PRD-16-orchestration-acceptance
