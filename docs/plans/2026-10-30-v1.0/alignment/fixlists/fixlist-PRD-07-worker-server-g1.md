# Fix list for PRD-07-worker-server.md (group 1)

From the alignment reviews L1 to L7, triaged against OWNER-DECISIONS.md (the ledger wins).
Files are under docs/plans/2026-10-30-v1.0/. Line numbers are from branch smart/intelligent-newton-0avrpi.
An item with several files carries the same id and the same change text in each file's list; make only the part for this file.

Counts: 1 blocker, 0 major, 3 minor.

## L6-05 (blocker)

- Section and lines: PRD-16 Conductor row (81), X6 and X7 (278-279), N6 (502), builder notes (590); PRD-00 'Where the build runs' (485-486); PRD-12 section 1 table (140), section 3 rows 0:27-0:28 (171-172), keys (320-322), section 8 (342-349), selftest (505); PRD-07 'How a watcher proves it' (570-574); PRD-09 section 1 (98-100), `scripts/usage-job.sh` (442), risks (579-582)
- Change: The conductor only fetches, approves and runs read-only queries; every `goalcup-verify` run happens in a sandbox. PRD-16:81: '`scripts/pad/pad` (conductor half), `fetch-run`, `goalcup-verify` for checks. It never compiles or tests' becomes '`scripts/pad/pad` (conductor half: sandboxes, approval, fetch, read-only queries), `fetch-run`. It never compiles, tests or runs `goalcup-verify`; checks run in the pad box before it is removed (PRD-12 section 8)'. PRD-00:485-486: '`fetch-run` and server checks run there.' becomes '`fetch-run` and the read-only queries run there. The fetched copy is copied into the pad box and `goalcup-verify` runs there (ledger B1).' PRD-12:140: 'approves, fetches, checks, writes the row' becomes 'approves, fetches, copies the fetched entry and the per-run known-values file into the box, has `pad-run` run the checks there, writes the row from their JSON'. PRD-12:171-172: 0:27 conductor 'copy out ...; run `fetch-run`; `sandbox copy` the fetched entry and the per-run known-values file in; `sandbox exec` the four commands of section 8; copy out their JSON; `sandbox remove pad-<id>`'; 0:28 'write `row.json` and `report.txt`; scan the pad's output folder; append to the ledger'. PRD-12:320-322: 'The per-run known-values file holds the values that entered this box (its keys, its repo key), every planted value and the canary; it is made on the conductor, 0600, copied into the box for the checks, and removed with the box.' PRD-12:342: 'The conductor runs, for each run:' becomes '`pad-run` runs in the box, after the conductor copied in the fetched entry and the per-run known-values file:'. PRD-12:505: 'on the conductor' becomes 'in a build box'. PRD-07:570-574: 'run `goalcup-verify check --server <out>/<entry> --local <entry dir>`' becomes 'copy `<out>/<entry>` into the pad box and run `goalcup-verify check --server ... --local ...` there'. PRD-16 X6, X7 (278-279) and builder notes (590): add 'in the run's pad box'. PRD-16 N6 (502): add 'run in one check box from the build snapshot, `--non-persistent`, holding the full known-values file for the scan only and removed right after (a `decision` line: the one step where all keys meet a box)'. PRD-09:98-100: 'During the build the conductor does only the steps that need credentials: it lists sealed entries (read-only database), runs `fetch-run`, copies the fetched folder into a build box, and puts the three output files into `goalcup-usage-staging`. `goalcup-verify usage` itself runs in the build box, which holds no credential (ledger B1). The conductor never compiles or runs the reader. The production job is the owner's to start, on an organizer machine.' PRD-09:442 `scripts/usage-job.sh` and risks 579-582: the same split.
- Ledger item or rule: B1 (locked: builds, tests and real runs in Vercel Sandboxes; the cloud session orchestrates only)
- Source: align-L6 L6-05 (with align-PRD-09 P09-1)
- Also in: PRD-16-orchestration-acceptance, PRD-00-index, PRD-12-test-pad, PRD-09-usage-research-stats

## L2-08 (minor)

- Section and lines: PRD-07 section 4 (197)
- Change: '`mode` null (null mode), or the entry ended while off' becomes '`mode` null (null mode), and only then; a recovered entry is `gaps` (PRD-06 section 6)'.
- Ledger item or rule: R7 (one rule for two builders)
- Source: align-L2 L2-08

## L4-6 (minor)

- Section and lines: PRD-07 section 12 step 5 (391)
- Change: 'Create `goalcup-usage` if the usage job is in.' becomes 'Create `goalcup-usage` (the usage job is in this build, third slice).'
- Ledger item or rule: S2 (locked)
- Source: align-L4 L4-6

## L7-09 (minor)

- Section and lines: PRD-07 section 12 step 3 (384-387), risk rows (511-512); PRD-16 `MORNING.md` owner steps
- Change: PRD-07 section 12 step 3: add 'Note for the owner: with 1.0 the unchanged cron sends the `recording` email when a heartbeat says `failed`, and the idle notices count written requests. No email code changed; confirm this is wanted before the production deploy (owner list OW-12).' PRD-16 `MORNING.md` owner steps: add the same line.
- Ledger item or rule: Section 8 (email changes out of scope); R7
- Source: align-L7 L7-09
- Also in: PRD-16-orchestration-acceptance
