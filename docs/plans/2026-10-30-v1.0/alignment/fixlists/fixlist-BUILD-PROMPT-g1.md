# Fix list for BUILD-PROMPT.md (group 1)

From the alignment reviews L1 to L7, triaged against OWNER-DECISIONS.md (the ledger wins).
Files are under docs/plans/2026-10-30-v1.0/. Line numbers are from branch smart/intelligent-newton-0avrpi.
An item with several files carries the same id and the same change text in each file's list; make only the part for this file.

Counts: 0 blocker, 5 major, 6 minor.

## L6-01 (major)

- Section and lines: BUILD-PROMPT section 1 item 5 (24-26), section 4 table (59-73); PRD-16 section 1 'Lanes' (91); PRD-00 ownership row `BUILD-PROMPT.md` (339), R3-10 (871)
- Change: BUILD-PROMPT:24-26: 'phases, merge rule, real runs, acceptance, autonomy, stop rules, wake list, morning report. Follow it, except where this prompt or the ledger says otherwise.' becomes 'lanes, phases, merge rule, real runs, acceptance, autonomy, stop rules, call list, end-of-build report. Follow it. Where this prompt and PRD-16 differ, PRD-16 wins (PRD-00 R3-10); where either differs from the ledger, the ledger wins. Re-read PRD-16's Amendments notes after every replan.' BUILD-PROMPT:59-73: replace the sentence at 61 and the table with 'Start the twelve lane workflows of PRD-16 section 1 in parallel, with the PRDs and start times that table gives. Each lane edits only the files the PRD-00 ownership map gives its PRDs.' PRD-16:91: '(BUILD-PROMPT section 4)' becomes '(ledger B2; all lanes run in parallel; this table replaces BUILD-PROMPT section 4's, PRD-00 R3-10)'. PRD-00:871 R3-10: 'Ledger B1, B2 and B2a are locked and win over this file, PRD-16 and BUILD-PROMPT.md. Within them the conductor starts PRD-16 section 1's twelve lanes with its start times and roles. BUILD-PROMPT section 4's table differs (Masking inside Format and core, Judge after PRD-01, Participant after PRD-02, PRD-14 inside Cut and docs, no Real runs lane) and is replaced by a pointer to PRD-16 before H+0. BUILD-PROMPT's per-PRD pipeline (plan, parallel builders in worktrees, at least two independent reviewers, fix, merge, replan) is the ledger's and stands.' Applied by: 'PRD-16; BUILD-PROMPT before H+0'. PRD-00:339: 'Edited before H+0 only (R3-10). Not edited during the build: the conductor holds the pasted text, so a later edit reaches no one. The replan changes the build plan through Amendments notes on PRD-16 (ledger B2a). Where the prompt and PRD-16 differ, PRD-16 wins; the ledger wins over both.'
- Ledger item or rule: B2, B2a (locked); settles the known R3-10 conflict
- Source: align-L6 L6-01
- Also in: PRD-16-orchestration-acceptance, PRD-00-index

## L6-02 (major)

- Section and lines: PRD-16 section 1 Replan row (119); BUILD-PROMPT section 4 step 5 (90-93); PRD-00 ownership rows (339-340)
- Change: PRD-16:119 Replan row: Count 'one per merge of a PRD branch (each slice), as the last step of that PRD's lane pipeline'; Does 'Reads review findings, test and real-run results; appends an Amendments note to each remaining PRD it affects and to PRD-16 itself, which is the build plan (lanes, order, phases, runs, caps within the ledger); writes an `amendment` night-log line. Reviewer A of that merge reads the note against the ledger; the integrator commits it on integration. The conductor re-reads PRD-16's Amendments after each one and applies them to the lane queues'; Never 'Rewrites a section a builder is working from, edits BUILD-PROMPT.md, or changes a locked ledger item (that calls the owner, W6)'. BUILD-PROMPT:90-93 step 5: '**Replan.** After each merge of a PRD branch, an improvement agent reads what was learned (review findings, test results, real-run results) and appends Amendments notes to the remaining PRDs it affects and to PRD-16, the build plan, in a commit on the integration branch, with a night-log line. You re-read PRD-16's Amendments and adjust the lanes. This prompt is not edited during the build. A note may not change a locked ledger item; that calls me (W6).' PRD-00:340: after 'at the end of a PRD' add ', PRD-00 and PRD-16 included,'.
- Ledger item or rule: B2a (locked: updates the remaining PRDs and the build plan)
- Source: align-L6 L6-02
- Also in: PRD-16-orchestration-acceptance, PRD-00-index

## L6-03 (major)

- Section and lines: PRD-16 section 4 merge rule item 4 (204-208); BUILD-PROMPT section 4 step 3 last line (89)
- Change: PRD-16:204-208 item 4 becomes: 'At least two independent reviewers (A and B) read the merge request's diff, and for core parts the breaker too. Every finding is fixed in the merge request before the merge (ledger B2). A finding may be declined only with a one-line reason in the merge request that the reviewer who raised it accepts; reviewer B's less-code findings are declined when the change would not make the code smaller or clearer. Never declined: a path outside ownership; a break of a locked ledger item, a hard rule (AGENTS.md as PRD-00 restates it) or one of the seven points; a byte written to disk before masking; a drop with no GAP; a cap of our own; a harness name in core code; a secret in a fixture; a wrong count; a failing acceptance case of the PRD; a check made to pass without record evidence.' BUILD-PROMPT:89: 'Blockers per PRD-16 section 4; everything else is fixed if it makes the code smaller or clearer.' becomes 'Every finding is fixed before the merge; what may be declined, and what never may, is in PRD-16 section 4.'
- Ledger item or rule: B2 (locked: reviewers, fix, merge); R6, R7; points 1, 6
- Source: align-L6 L6-03
- Also in: PRD-16-orchestration-acceptance

## L6-07 (major)

- Section and lines: BUILD-PROMPT section 7 (123-127); PRD-16 section 11 (344-345)
- Change: BUILD-PROMPT:123-127: '## 7. Call me (a push notification and a top line in `NIGHT-LOG.md`) only for PRD-16 section 11's W1 to W7: a real secret found in any record, server copy, log, commit or pushed repository (all runs stop); anything that touched production (all runs stop); the website database looking hurt; spend out of control; a key the core set needs missing or refused; a locked ledger item that cannot hold, a hard rule that seems wrong, or Vercel Sandboxes down for 30 minutes; the core set still failing after the fix loops. Otherwise keep going.' PRD-16:345: 'Only these (BUILD-PROMPT section 7):' becomes 'Only these; BUILD-PROMPT section 7 lists the same seven:'.
- Ledger item or rule: Point 4; B4, B5, B8; S3
- Source: align-L6 L6-07
- Also in: PRD-16-orchestration-acceptance

## L6-10 (major)

- Section and lines: PRD-12 section 3 row 0:01 (165); PRD-16 section 8 (290-291); BUILD-PROMPT section 2 (39-40)
- Change: PRD-12:165: move repo creation to the conductor at 0:00 and replace 'a new empty public repo with the GitHub token ..., its token in `.env.local` 0600' with 'the conductor makes the repo (prefix `gc-pad-`) and a deploy key with write access to that repo only (GitHub API `POST /repos/{owner}/{repo}/keys`); the box gets only that deploy key, 0600, and it goes in the per-run known-values file. The account token never enters a box.' PRD-16:290-291: 'create public, empty GitHub repositories' becomes 'create empty GitHub repositories for pad runs (prefix `gc-pad-`), private unless the staging Worker's start-time repository check needs them public (PRD-07), each with its own deploy key'. BUILD-PROMPT:39-40: 'A pad box holds one harness key for the life of its run, in a 0600 file.' becomes 'A pad box holds its harness key (and for the gateway case the gateway key) and a deploy key for its one test repo, each in a 0600 file, for the life of its run; never the GitHub account token.'
- Ledger item or rule: Point 4; B8 (autonomy bounded by AGENTS.md)
- Source: align-L6 L6-10
- Also in: PRD-12-test-pad, PRD-16-orchestration-acceptance

## F9 (minor)

- Section and lines: BUILD-PROMPT section 4 step 3 reviewer A (84-85); section 6 (112-121)
- Change: Add to reviewer A at 85: 'nothing PRD-13's no-return guard names comes back (daemon or IPC for bytes, our own caps, disk handling, signing, pins, termination proof, research decoders), no compatibility shim for 0.7'. Add to section 6: 'One process: no IPC for recorded bytes, no failure budget. No backward compatibility.'
- Ledger item or rule: L5, L6, L7
- Source: align-L1 F9

## L6-12 (minor)

- Section and lines: PRD-16 section 1 Builder row (116); BUILD-PROMPT section 4 step 2 (79-80)
- Change: 'on `v1.0/prd-NN-<slug>`' becomes 'on `v1.0/prd-NN-<slug>-<task>` from the PRD branch, merged into `v1.0/prd-NN-<slug>` by the lane before its merge request' in both places.
- Ledger item or rule: B2 (parallel builders in worktrees)
- Source: align-L6 L6-12
- Also in: PRD-16-orchestration-acceptance

## L6-13 (minor)

- Section and lines: PRD-16 section 3 phase 1a (182-183); BUILD-PROMPT section 5 (97-99)
- Change: Replace the parenthesis in both with '(each core feature of PRD-00's second and third slices: native capture, uploader, watchdog restart and null mode, refusal learning, the gateway and dodger cases, Hermes `state.db`, HTTP/2, and exact-value masking only if the owner said yes, OW-01)'.
- Ledger item or rule: B3 (locked: one short real test after each new core feature)
- Source: align-L6 L6-13
- Also in: PRD-16-orchestration-acceptance

## L6-14 (minor)

- Section and lines: BUILD-PROMPT intro (6-7, 13) and section 7 title (123)
- Change: BUILD-PROMPT:13: 'leave me a morning report' becomes 'leave me the end-of-build report (`MORNING.md`, PRD-16 section 13)'. Section 7 title per L6-07. BUILD-PROMPT:6-7: 'the two staging values are set (the local-agent prompt from the planning session)' becomes '`GOALCUP_TEST_APPROVAL_KEY` and `GOALCUP_TEST_MEMBER_ID` are in the environment secrets and the staging Worker holds the matching `TEST_APPROVAL_KEY`'.
- Ledger item or rule: B7 (clarity)
- Source: align-L6 L6-14

## L6-17 (minor)

- Section and lines: BUILD-PROMPT section 5 'Mass testing' (104-106)
- Change: 'then the other 12 harnesses, then the acceptance runs A1 to A4 (PRD-16 section 7)' becomes 'then the other 12 harnesses and, beside them once the core set is green, the acceptance runs (A2 first, then A1 and A3; A4 is Kilo's gateway row) as PRD-16 sections 6 and 7 order them'.
- Ledger item or rule: B3; R3-10
- Source: align-L6 L6-17

## L6-20 (minor)

- Section and lines: BUILD-PROMPT section 4 step 3, third reviewer (87-88)
- Change: 'a run 1 replay (about 40 subagents, 1.5 MB bodies, rate-limit bursts, WebSockets)' becomes 'against run 1's shape (about 40 subagents, 1.5 MB bodies, rate-limit bursts, WebSockets) with PRD-02's `pipeline_guard.rs` and the latest pad smoke row, once per merge request; never a real 2-hour replay per diff'.
- Ledger item or rule: R3-10 consistency
- Source: align-L6 L6-20
