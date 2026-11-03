# Fix list for PRD-16-orchestration-acceptance.md (group 1)

From the alignment reviews L1 to L7, triaged against OWNER-DECISIONS.md (the ledger wins).
Files are under docs/plans/2026-10-30-v1.0/. Line numbers are from branch smart/intelligent-newton-0avrpi.
An item with several files carries the same id and the same change text in each file's list; make only the part for this file.

Counts: 3 blocker, 10 major, 11 minor.

## L5-01 (blocker)

- Section and lines: PRD-12 section 7 'Per run' row (328), risk row (687), overridable defaults (699); PRD-16 section 9 (316-317); PRD-09 owner decisions (35-36); PRD-00 'Live spend pricing' row (578)
- Change: PRD-12 section 7, 'Per run' row: value '10 USD per run (ledger T6)'; 'What happens': 'During every run, `pad` runs `goalcup-verify usage <open entry> --json` in the box every 5 minutes with `sandbox exec`, prices the totals with the flat rate in `pad.toml`, and runs `goalcup seal --yes` at 10 USD. The row gets `over_cap: true` and the checks run on what was recorded. After the run the ledger line is written as now.' Risk row 687 names this live check. PRD-16:316: pad rows use the same check as 317, every 5 minutes for both. PRD-09:35-36: 'No rule, cap or budget reads these numbers.' becomes 'No contest rule reads these numbers (D26). The one reader that acts on them is the pad's test spend check (ledger T6, PRD-16 section 9), which reads `usage --json` on the open local entry in the box.' PRD-00:578 'Live spend pricing' row: 'Cut' becomes 'the pad's flat-rate check on `usage` (PRD-16), no `prices.toml`'. A `smoke` run has no lower cap of its own (see L5-02/L6-09).
- Ledger item or rule: T6 (locked: 10 USD per run); B8
- Source: align-L5 L5-01 (with align-PRD-09 P09-10 for the PRD-09 line)
- Also in: PRD-12-test-pad, PRD-09-usage-research-stats, PRD-00-index

## L5-02/L6-09 (blocker)

- Section and lines: PRD-12 section 7 'Whole build' row (329), overridable defaults (700); PRD-00 (639) and owner question on build spend (731-733); PRD-16 section 9 rows (316, 318-319), 'Why 400' (322-326), S4 (335), W4 (352), defaults (543-544)
- Change: No whole-build cap and no 2 USD smoke cap; only the 10 USD per-run cap stops a run. PRD-12:328 value '10 USD per run (ledger T6); a `smoke` run is expected near 2 USD, an estimate, not a cap'. PRD-12:329 and PRD-16:318: 'No cap (ledger T6). The scribe writes a `spend` line at 200 and 400 USD by the pad's ledger estimate and tells the owner; the report shows it; runs go on.' Remove 'A new run does not start past it' (PRD-12:329) and 'No new run starts' (PRD-16:318). PRD-16:319: 'No cap. A `spend` line at 75 and 150 USD estimated sandbox cost; builds go on.' PRD-16:322-326: 'The expected model spend is about 200 USD: phase 1a and selftest about 35, phase 4 up to 180 with reruns, phase 5 about 25 runs at 1 to 2 USD, acceptance up to 30.' Delete S4 (335). PRD-16:352 W4: 'A run past 1.5 times its 10 USD cap, a provider dashboard that keeps rising with no run going, or a sandbox that will not stop.' PRD-16:543-544 and PRD-12:700: 'Build spend report lines | 200 and 400 USD model, 75 and 150 USD sandboxes | Owner'. PRD-00:639 and the build-spend owner question (731-733): same report-line wording.
- Ledger item or rule: T6 (locked: cost otherwise not a constraint); T1, B3 (mass testing of all 15)
- Source: align-L5 L5-02, align-L6 L6-09
- Also in: PRD-12-test-pad, PRD-00-index

## L6-05 (blocker)

- Section and lines: PRD-16 Conductor row (81), X6 and X7 (278-279), N6 (502), builder notes (590); PRD-00 'Where the build runs' (485-486); PRD-12 section 1 table (140), section 3 rows 0:27-0:28 (171-172), keys (320-322), section 8 (342-349), selftest (505); PRD-07 'How a watcher proves it' (570-574); PRD-09 section 1 (98-100), `scripts/usage-job.sh` (442), risks (579-582)
- Change: The conductor only fetches, approves and runs read-only queries; every `goalcup-verify` run happens in a sandbox. PRD-16:81: '`scripts/pad/pad` (conductor half), `fetch-run`, `goalcup-verify` for checks. It never compiles or tests' becomes '`scripts/pad/pad` (conductor half: sandboxes, approval, fetch, read-only queries), `fetch-run`. It never compiles, tests or runs `goalcup-verify`; checks run in the pad box before it is removed (PRD-12 section 8)'. PRD-00:485-486: '`fetch-run` and server checks run there.' becomes '`fetch-run` and the read-only queries run there. The fetched copy is copied into the pad box and `goalcup-verify` runs there (ledger B1).' PRD-12:140: 'approves, fetches, checks, writes the row' becomes 'approves, fetches, copies the fetched entry and the per-run known-values file into the box, has `pad-run` run the checks there, writes the row from their JSON'. PRD-12:171-172: 0:27 conductor 'copy out ...; run `fetch-run`; `sandbox copy` the fetched entry and the per-run known-values file in; `sandbox exec` the four commands of section 8; copy out their JSON; `sandbox remove pad-<id>`'; 0:28 'write `row.json` and `report.txt`; scan the pad's output folder; append to the ledger'. PRD-12:320-322: 'The per-run known-values file holds the values that entered this box (its keys, its repo key), every planted value and the canary; it is made on the conductor, 0600, copied into the box for the checks, and removed with the box.' PRD-12:342: 'The conductor runs, for each run:' becomes '`pad-run` runs in the box, after the conductor copied in the fetched entry and the per-run known-values file:'. PRD-12:505: 'on the conductor' becomes 'in a build box'. PRD-07:570-574: 'run `goalcup-verify check --server <out>/<entry> --local <entry dir>`' becomes 'copy `<out>/<entry>` into the pad box and run `goalcup-verify check --server ... --local ...` there'. PRD-16 X6, X7 (278-279) and builder notes (590): add 'in the run's pad box'. PRD-16 N6 (502): add 'run in one check box from the build snapshot, `--non-persistent`, holding the full known-values file for the scan only and removed right after (a `decision` line: the one step where all keys meet a box)'. PRD-09:98-100: 'During the build the conductor does only the steps that need credentials: it lists sealed entries (read-only database), runs `fetch-run`, copies the fetched folder into a build box, and puts the three output files into `goalcup-usage-staging`. `goalcup-verify usage` itself runs in the build box, which holds no credential (ledger B1). The conductor never compiles or runs the reader. The production job is the owner's to start, on an organizer machine.' PRD-09:442 `scripts/usage-job.sh` and risks 579-582: the same split.
- Ledger item or rule: B1 (locked: builds, tests and real runs in Vercel Sandboxes; the cloud session orchestrates only)
- Source: align-L6 L6-05 (with align-PRD-09 P09-1)
- Also in: PRD-00-index, PRD-12-test-pad, PRD-07-worker-server, PRD-09-usage-research-stats

## F1 (major)

- Section and lines: PRD-16 section 5 'Cut-off H+10' (226-228) and S7 (338); PRD-13 section 5 (175-177) and risk 'The cut is skipped' (531); PRD-00 phase 3 row (563)
- Change: PRD-16:338 S7 'What happens': 'No cut (section 5)' becomes 'No cut yet (section 5). Calls the owner (W6): locked L3 cannot hold on time. The lessons lane keeps working; the cut lands in the same build as soon as the gate is green, before `MORNING.md`.' Same sentence in PRD-16:226-228, PRD-13:175-177 and :531. PRD-00:563: 'the end-of-build report says so' becomes 'the owner is called under W6 and the cut lands as soon as the gate is green'.
- Ledger item or rule: L3, L6 (locked: delete the old crates in the same build)
- Source: align-L1 F1
- Also in: PRD-13-cuts, PRD-00-index

## L5-03 (major)

- Section and lines: PRD-12 owner decisions (33-34), tier rule (427-430), `--release-check` (437-438), P3 (639), overridable defaults (708); PRD-00 done item 1; PRD-16 phase 4
- Change: PRD-12:427-430: 'the latest `gateway` and `dodger` runs, at least `smoke`, are PASS' becomes 'the latest `gateway` and `dodger` runs on `full` are PASS'. Same edit in 33-34, 437-438 ('nine core `full` rows'), P3 639 ('`direct`, `gateway` and `dodger` on `full`') and the 'Tier needs' default 708. Keep `smoke` for phase 1a, the build's short tests and watcher reproduction (section 13 step 3). PRD-16 phase 4 and PRD-00 done item 1 say the same.
- Ledger item or rule: T5 (locked pad workload), T4, B3
- Source: align-L5 L5-03
- Also in: PRD-12-test-pad, PRD-00-index

## L5-04 (major)

- Section and lines: PRD-16 phase 5 (240); PRD-11 section 7 outlook column (662-678); PRD-12 risk row (677)
- Change: PRD-16:240: add 'As each harness's first `full` sample lands, a PRD-09 builder writes its reader from that sample (PRD-09 rule 250), then the row is rerun once. A harness with no reader by the end of phase 5 is "not verified: no log reader".' PRD-11 outlook column: append '; needs a PRD-09 reader written from its phase 5 sample' to every row except Claude Code, Codex and Hermes. PRD-12:677: 'unless PRD-09 writes a reader from their phase 5 sample in time' becomes 'until the PRD-16 phase 5 reader step writes one from their sample'.
- Ledger item or rule: T1, T3, T4 (wire calls match the harness's own log; usage stats present), S2
- Source: align-L5 L5-04
- Also in: PRD-11-connectors-harnesses, PRD-12-test-pad

## L6-01 (major)

- Section and lines: BUILD-PROMPT section 1 item 5 (24-26), section 4 table (59-73); PRD-16 section 1 'Lanes' (91); PRD-00 ownership row `BUILD-PROMPT.md` (339), R3-10 (871)
- Change: BUILD-PROMPT:24-26: 'phases, merge rule, real runs, acceptance, autonomy, stop rules, wake list, morning report. Follow it, except where this prompt or the ledger says otherwise.' becomes 'lanes, phases, merge rule, real runs, acceptance, autonomy, stop rules, call list, end-of-build report. Follow it. Where this prompt and PRD-16 differ, PRD-16 wins (PRD-00 R3-10); where either differs from the ledger, the ledger wins. Re-read PRD-16's Amendments notes after every replan.' BUILD-PROMPT:59-73: replace the sentence at 61 and the table with 'Start the twelve lane workflows of PRD-16 section 1 in parallel, with the PRDs and start times that table gives. Each lane edits only the files the PRD-00 ownership map gives its PRDs.' PRD-16:91: '(BUILD-PROMPT section 4)' becomes '(ledger B2; all lanes run in parallel; this table replaces BUILD-PROMPT section 4's, PRD-00 R3-10)'. PRD-00:871 R3-10: 'Ledger B1, B2 and B2a are locked and win over this file, PRD-16 and BUILD-PROMPT.md. Within them the conductor starts PRD-16 section 1's twelve lanes with its start times and roles. BUILD-PROMPT section 4's table differs (Masking inside Format and core, Judge after PRD-01, Participant after PRD-02, PRD-14 inside Cut and docs, no Real runs lane) and is replaced by a pointer to PRD-16 before H+0. BUILD-PROMPT's per-PRD pipeline (plan, parallel builders in worktrees, at least two independent reviewers, fix, merge, replan) is the ledger's and stands.' Applied by: 'PRD-16; BUILD-PROMPT before H+0'. PRD-00:339: 'Edited before H+0 only (R3-10). Not edited during the build: the conductor holds the pasted text, so a later edit reaches no one. The replan changes the build plan through Amendments notes on PRD-16 (ledger B2a). Where the prompt and PRD-16 differ, PRD-16 wins; the ledger wins over both.'
- Ledger item or rule: B2, B2a (locked); settles the known R3-10 conflict
- Source: align-L6 L6-01
- Also in: BUILD-PROMPT, PRD-00-index

## L6-02 (major)

- Section and lines: PRD-16 section 1 Replan row (119); BUILD-PROMPT section 4 step 5 (90-93); PRD-00 ownership rows (339-340)
- Change: PRD-16:119 Replan row: Count 'one per merge of a PRD branch (each slice), as the last step of that PRD's lane pipeline'; Does 'Reads review findings, test and real-run results; appends an Amendments note to each remaining PRD it affects and to PRD-16 itself, which is the build plan (lanes, order, phases, runs, caps within the ledger); writes an `amendment` night-log line. Reviewer A of that merge reads the note against the ledger; the integrator commits it on integration. The conductor re-reads PRD-16's Amendments after each one and applies them to the lane queues'; Never 'Rewrites a section a builder is working from, edits BUILD-PROMPT.md, or changes a locked ledger item (that calls the owner, W6)'. BUILD-PROMPT:90-93 step 5: '**Replan.** After each merge of a PRD branch, an improvement agent reads what was learned (review findings, test results, real-run results) and appends Amendments notes to the remaining PRDs it affects and to PRD-16, the build plan, in a commit on the integration branch, with a night-log line. You re-read PRD-16's Amendments and adjust the lanes. This prompt is not edited during the build. A note may not change a locked ledger item; that calls me (W6).' PRD-00:340: after 'at the end of a PRD' add ', PRD-00 and PRD-16 included,'.
- Ledger item or rule: B2a (locked: updates the remaining PRDs and the build plan)
- Source: align-L6 L6-02
- Also in: BUILD-PROMPT, PRD-00-index

## L6-03 (major)

- Section and lines: PRD-16 section 4 merge rule item 4 (204-208); BUILD-PROMPT section 4 step 3 last line (89)
- Change: PRD-16:204-208 item 4 becomes: 'At least two independent reviewers (A and B) read the merge request's diff, and for core parts the breaker too. Every finding is fixed in the merge request before the merge (ledger B2). A finding may be declined only with a one-line reason in the merge request that the reviewer who raised it accepts; reviewer B's less-code findings are declined when the change would not make the code smaller or clearer. Never declined: a path outside ownership; a break of a locked ledger item, a hard rule (AGENTS.md as PRD-00 restates it) or one of the seven points; a byte written to disk before masking; a drop with no GAP; a cap of our own; a harness name in core code; a secret in a fixture; a wrong count; a failing acceptance case of the PRD; a check made to pass without record evidence.' BUILD-PROMPT:89: 'Blockers per PRD-16 section 4; everything else is fixed if it makes the code smaller or clearer.' becomes 'Every finding is fixed before the merge; what may be declined, and what never may, is in PRD-16 section 4.'
- Ledger item or rule: B2 (locked: reviewers, fix, merge); R6, R7; points 1, 6
- Source: align-L6 L6-03
- Also in: BUILD-PROMPT

## L6-04 (major)

- Section and lines: PRD-16 section 2 phase 0 (130-131, steps 5 and 6 at 141-156); section 5 phase 3 workspace edit (219-223)
- Change: PRD-16:130-131: after 'never a force push.' add 'Steps 4 to 6 and the phase 3 workspace edit go through section 4 like any merge request: on branch `v1.0/prd-16-workspace`, two reviewers, `gate fast` in a build box. A `Cargo.lock` refresh whose only cause is a reviewed `Cargo.toml` change needs no reviewer. Only the docs merge of step 3 (the reviewed PRD set) and the scribe's log and report commits skip review; N6 scans them.'
- Ledger item or rule: B2 (locked: at least two reviewers read each diff)
- Source: align-L6 L6-04

## L6-07 (major)

- Section and lines: BUILD-PROMPT section 7 (123-127); PRD-16 section 11 (344-345)
- Change: BUILD-PROMPT:123-127: '## 7. Call me (a push notification and a top line in `NIGHT-LOG.md`) only for PRD-16 section 11's W1 to W7: a real secret found in any record, server copy, log, commit or pushed repository (all runs stop); anything that touched production (all runs stop); the website database looking hurt; spend out of control; a key the core set needs missing or refused; a locked ledger item that cannot hold, a hard rule that seems wrong, or Vercel Sandboxes down for 30 minutes; the core set still failing after the fix loops. Otherwise keep going.' PRD-16:345: 'Only these (BUILD-PROMPT section 7):' becomes 'Only these; BUILD-PROMPT section 7 lists the same seven:'.
- Ledger item or rule: Point 4; B4, B5, B8; S3
- Source: align-L6 L6-07
- Also in: BUILD-PROMPT

## L6-08 (major)

- Section and lines: PRD-16 section 10 S2 and S3 (333-334); risks (516); defaults 'Phase-miss narrowing' (550); review answer B verdict 2 (640)
- Change: PRD-16:333 S2: 'The scribe notes it. The conductor reorders, never drops: phase 5 runs on, harnesses with a key first; every harness and case gets its run before the report calls it "not verified". A4 runs as Kilo's phase 5 row. The core set and A1 to A3 keep their place.' PRD-16:334 S3: 'At `REPORT_AT` the scribe writes `MORNING.md` from what is done. Runs go on until every harness and case has had its run and A1 to A3 have run (ledger B3); the scribe updates `MORNING.md` as each ends and sends one call when the last ends.' PRD-16:516: 'S2 narrows phase 5' becomes 'S2 reorders phase 5'. PRD-16:550: 'Phase-miss narrowing' becomes 'Phase-miss reordering'. PRD-16:640: 'S2 still narrows phase 5 first' becomes 'S2 reorders phase 5; nothing is dropped for time'.
- Ledger item or rule: B3 (locked: mass testing of all harnesses at the end); T3, T4
- Source: align-L6 L6-08

## L6-10 (major)

- Section and lines: PRD-12 section 3 row 0:01 (165); PRD-16 section 8 (290-291); BUILD-PROMPT section 2 (39-40)
- Change: PRD-12:165: move repo creation to the conductor at 0:00 and replace 'a new empty public repo with the GitHub token ..., its token in `.env.local` 0600' with 'the conductor makes the repo (prefix `gc-pad-`) and a deploy key with write access to that repo only (GitHub API `POST /repos/{owner}/{repo}/keys`); the box gets only that deploy key, 0600, and it goes in the per-run known-values file. The account token never enters a box.' PRD-16:290-291: 'create public, empty GitHub repositories' becomes 'create empty GitHub repositories for pad runs (prefix `gc-pad-`), private unless the staging Worker's start-time repository check needs them public (PRD-07), each with its own deploy key'. BUILD-PROMPT:39-40: 'A pad box holds one harness key for the life of its run, in a 0600 file.' becomes 'A pad box holds its harness key (and for the gateway case the gateway key) and a deploy key for its one test repo, each in a 0600 file, for the life of its run; never the GitHub account token.'
- Ledger item or rule: Point 4; B8 (autonomy bounded by AGENTS.md)
- Source: align-L6 L6-10
- Also in: PRD-12-test-pad, BUILD-PROMPT

## L4-7 (minor)

- Section and lines: PRD-05 section 2 connector table (154), section 8 (329); PRD-16 `MORNING.md` owner steps
- Change: Add under PRD-05 section 8: 'Reading of ledger M1: the Codex rate-limit rules are the four plan, quota and credit rows (`connectors/codex-cli/harness.toml:110-113`); the three id rows stay. The owner confirms (owner list OW-04).' PRD-16's `MORNING.md` owner steps list it if not answered before H+0.
- Ledger item or rule: M1 (wording)
- Source: align-L4 L4-7
- Also in: PRD-05-masking

## L6-11 (minor)

- Section and lines: PRD-16 section 1 Reviewers row (117); PRD-00 lanes (512)
- Change: PRD-16:117 Count: 'at least two per merge request'; Does: append 'Each is a fresh agent, not the builder, and does not see the other reviewer's findings before writing its own.' PRD-00:512: 'two reviewers' becomes 'at least two independent reviewers'.
- Ledger item or rule: B2 (locked)
- Source: align-L6 L6-11
- Also in: PRD-00-index

## L6-12 (minor)

- Section and lines: PRD-16 section 1 Builder row (116); BUILD-PROMPT section 4 step 2 (79-80)
- Change: 'on `v1.0/prd-NN-<slug>`' becomes 'on `v1.0/prd-NN-<slug>-<task>` from the PRD branch, merged into `v1.0/prd-NN-<slug>` by the lane before its merge request' in both places.
- Ledger item or rule: B2 (parallel builders in worktrees)
- Source: align-L6 L6-12
- Also in: BUILD-PROMPT

## L6-13 (minor)

- Section and lines: PRD-16 section 3 phase 1a (182-183); BUILD-PROMPT section 5 (97-99)
- Change: Replace the parenthesis in both with '(each core feature of PRD-00's second and third slices: native capture, uploader, watchdog restart and null mode, refusal learning, the gateway and dodger cases, Hermes `state.db`, HTTP/2, and exact-value masking only if the owner said yes, OW-01)'.
- Ledger item or rule: B3 (locked: one short real test after each new core feature)
- Source: align-L6 L6-13
- Also in: BUILD-PROMPT

## L6-15 (minor)

- Section and lines: PRD-16 owner decisions (45-46); phase 0 step 1 (134-135)
- Change: PRD-16:45-46: 'approved with the staging test approval key' becomes 'approved with the staging test approval key and the owner's member id'. PRD-16:134-135: after 'which keys are present' add '(the staging approval key and member id included)'.
- Ledger item or rule: B4 (locked)
- Source: align-L6 L6-15

## L6-16 (minor)

- Section and lines: PRD-16 section 3 (177); interfaces PRD-17 row (478); 'Asks' (593-605)
- Change: PRD-16:177 and :478: 'by H+1.5' becomes 'by H+3 (R3-8)'. PRD-16:593-605: replace the four asks with 'All ruled by PRD-00 R3-8, R3-9 and R3-11; none left.'
- Ledger item or rule: PRD-00 R3-8 (consistency)
- Source: align-L6 L6-16

## L6-18 (minor)

- Section and lines: PRD-16 S1 (332); PRD-12 section 13 steps 6 and 7 (545-547)
- Change: PRD-16:332 and PRD-12 step 7: 'on one signature' becomes 'on one cause (the cause the watcher names in step 2; the signature only groups rows)'. PRD-12 step 6: 'A different signature: the first cause counts as fixed and a new loop starts' becomes 'A different signature with the same named cause is one failed fix; a different named cause starts a new loop'.
- Ledger item or rule: B3 (locked: three failed fixes on one cause)
- Source: align-L6 L6-18
- Also in: PRD-12-test-pad

## L6-19 (minor)

- Section and lines: PRD-16 section 4 last paragraph (210-212)
- Change: After 'an independent record' add 'on GitHub's runners; it is not a build host for agents, and no agent builds there (ledger B1)'.
- Ledger item or rule: B1 (clarity)
- Source: align-L6 L6-19

## L6-21 (minor)

- Section and lines: PRD-16 section 3 'First staging deploy' (186-192)
- Change: PRD-16:191-192: 'Before that deploy, runs use `--server off` or `--server local`.' becomes 'Before that deploy, runs use `--server off` or `--server local`: they upload nowhere, so B4 holds. The integrator deploys staging the hour PRD-07 step 2 and PRD-06 merge; no run ever targets production.'
- Ledger item or rule: B4 (clarity)
- Source: align-L6 L6-21

## L7-07 (minor)

- Section and lines: PRD-00 'Consequences' (681-683); PRD-15 (254-256); PRD-16 stop table (299)
- Change: PRD-00:681-683 and PRD-15:254-256: 'because the ledger allows two changes only' becomes 'because ledger section 7 changes only the rows it lists; ending the harness is today's behaviour, raised as owner question 2 (old 11)'. PRD-16:299: 'beyond PRD-00's two changes' becomes 'beyond the changes ledger section 7 lists'.
- Ledger item or rule: Ledger section 7
- Source: align-L7 L7-07
- Also in: PRD-00-index, PRD-15-docs-decisions

## L7-09 (minor)

- Section and lines: PRD-07 section 12 step 3 (384-387), risk rows (511-512); PRD-16 `MORNING.md` owner steps
- Change: PRD-07 section 12 step 3: add 'Note for the owner: with 1.0 the unchanged cron sends the `recording` email when a heartbeat says `failed`, and the idle notices count written requests. No email code changed; confirm this is wanted before the production deploy (owner list OW-12).' PRD-16 `MORNING.md` owner steps: add the same line.
- Ledger item or rule: Section 8 (email changes out of scope); R7
- Source: align-L7 L7-09
- Also in: PRD-07-worker-server
