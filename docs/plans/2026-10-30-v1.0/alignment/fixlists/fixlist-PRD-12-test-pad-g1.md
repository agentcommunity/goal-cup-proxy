# Fix list for PRD-12-test-pad.md (group 1)

From the alignment reviews L1 to L7, triaged against OWNER-DECISIONS.md (the ledger wins).
Files are under docs/plans/2026-10-30-v1.0/. Line numbers are from branch smart/intelligent-newton-0avrpi.
An item with several files carries the same id and the same change text in each file's list; make only the part for this file.

Counts: 4 blocker, 7 major, 5 minor.

## L4-1 (blocker)

- Section and lines: PRD-00 owner question 3 (709-712), PRD table row (213), status (238-239), second slice (550); PRD-05 owner decisions (34-37), rule rows 17 and 20 (129, 132), section 9 (334-395), section 10 rows (405-406, 409, 415-417), section 12 (453-465), overridable defaults (594, 597), builder step 6 (621); PRD-11 `secret_files` and `secret_values` (151, 339-340, 361-366, each connector's `secret_files` line); PRD-10 mask values call (153), `--prompt` risk row (732); PRD-12 planted `sk_test_`, `whsec_` and the minute 5 canary (257-259, 265-266, 598, 806); PRD-15 D80 (340)
- Change: Build ledger M1 as written by default; the extra masking waits for the owner (owner list OW-01). PRD-00 question 3: 'Default: yes.' becomes 'Default: no until the owner answers. Built only if the owner adds it to the ledger as M1a.' PRD-05: in 'Owner decisions' replace 'default yes, built in the second slice' with 'default no; not built unless the owner says yes'; mark rows 17 (Stripe test keys) and 20 (`whsec_`) and section 9 sources 2 to 4 'only with owner yes'; the default build keeps the `GH_TOKEN` exact value only (as today); section 12's default text is the 'no' variant (463-465). PRD-11: `secret_files` and `secret_values` are not read in the default build (the data rows may stay). PRD-10:153: only `mask::workspace_values` for `GH_TOKEN`. PRD-12: plant `sk_test_`, `whsec_` and the canary only when the owner said yes; otherwise drop them from `.env.local` and the known-values file. PRD-15 D80: remove 'and the key values from the four sources ... (default, PRD-00 question 12)' unless the owner says yes. If the owner says yes before H+0, none of this changes and the second slice builds it as planned.
- Ledger item or rule: M1 (locked: the masking list); B2a (a locked item changes only with the owner)
- Source: align-L4 L4-1
- Also in: PRD-00-index, PRD-05-masking, PRD-11-connectors-harnesses, PRD-10-cli-participant, PRD-15-docs-decisions

## L5-01 (blocker)

- Section and lines: PRD-12 section 7 'Per run' row (328), risk row (687), overridable defaults (699); PRD-16 section 9 (316-317); PRD-09 owner decisions (35-36); PRD-00 'Live spend pricing' row (578)
- Change: PRD-12 section 7, 'Per run' row: value '10 USD per run (ledger T6)'; 'What happens': 'During every run, `pad` runs `goalcup-verify usage <open entry> --json` in the box every 5 minutes with `sandbox exec`, prices the totals with the flat rate in `pad.toml`, and runs `goalcup seal --yes` at 10 USD. The row gets `over_cap: true` and the checks run on what was recorded. After the run the ledger line is written as now.' Risk row 687 names this live check. PRD-16:316: pad rows use the same check as 317, every 5 minutes for both. PRD-09:35-36: 'No rule, cap or budget reads these numbers.' becomes 'No contest rule reads these numbers (D26). The one reader that acts on them is the pad's test spend check (ledger T6, PRD-16 section 9), which reads `usage --json` on the open local entry in the box.' PRD-00:578 'Live spend pricing' row: 'Cut' becomes 'the pad's flat-rate check on `usage` (PRD-16), no `prices.toml`'. A `smoke` run has no lower cap of its own (see L5-02/L6-09).
- Ledger item or rule: T6 (locked: 10 USD per run); B8
- Source: align-L5 L5-01 (with align-PRD-09 P09-10 for the PRD-09 line)
- Also in: PRD-16-orchestration-acceptance, PRD-09-usage-research-stats, PRD-00-index

## L5-02/L6-09 (blocker)

- Section and lines: PRD-12 section 7 'Whole build' row (329), overridable defaults (700); PRD-00 (639) and owner question on build spend (731-733); PRD-16 section 9 rows (316, 318-319), 'Why 400' (322-326), S4 (335), W4 (352), defaults (543-544)
- Change: No whole-build cap and no 2 USD smoke cap; only the 10 USD per-run cap stops a run. PRD-12:328 value '10 USD per run (ledger T6); a `smoke` run is expected near 2 USD, an estimate, not a cap'. PRD-12:329 and PRD-16:318: 'No cap (ledger T6). The scribe writes a `spend` line at 200 and 400 USD by the pad's ledger estimate and tells the owner; the report shows it; runs go on.' Remove 'A new run does not start past it' (PRD-12:329) and 'No new run starts' (PRD-16:318). PRD-16:319: 'No cap. A `spend` line at 75 and 150 USD estimated sandbox cost; builds go on.' PRD-16:322-326: 'The expected model spend is about 200 USD: phase 1a and selftest about 35, phase 4 up to 180 with reruns, phase 5 about 25 runs at 1 to 2 USD, acceptance up to 30.' Delete S4 (335). PRD-16:352 W4: 'A run past 1.5 times its 10 USD cap, a provider dashboard that keeps rising with no run going, or a sandbox that will not stop.' PRD-16:543-544 and PRD-12:700: 'Build spend report lines | 200 and 400 USD model, 75 and 150 USD sandboxes | Owner'. PRD-00:639 and the build-spend owner question (731-733): same report-line wording.
- Ledger item or rule: T6 (locked: cost otherwise not a constraint); T1, B3 (mass testing of all 15)
- Source: align-L5 L5-02, align-L6 L6-09
- Also in: PRD-16-orchestration-acceptance, PRD-00-index

## L6-05 (blocker)

- Section and lines: PRD-16 Conductor row (81), X6 and X7 (278-279), N6 (502), builder notes (590); PRD-00 'Where the build runs' (485-486); PRD-12 section 1 table (140), section 3 rows 0:27-0:28 (171-172), keys (320-322), section 8 (342-349), selftest (505); PRD-07 'How a watcher proves it' (570-574); PRD-09 section 1 (98-100), `scripts/usage-job.sh` (442), risks (579-582)
- Change: The conductor only fetches, approves and runs read-only queries; every `goalcup-verify` run happens in a sandbox. PRD-16:81: '`scripts/pad/pad` (conductor half), `fetch-run`, `goalcup-verify` for checks. It never compiles or tests' becomes '`scripts/pad/pad` (conductor half: sandboxes, approval, fetch, read-only queries), `fetch-run`. It never compiles, tests or runs `goalcup-verify`; checks run in the pad box before it is removed (PRD-12 section 8)'. PRD-00:485-486: '`fetch-run` and server checks run there.' becomes '`fetch-run` and the read-only queries run there. The fetched copy is copied into the pad box and `goalcup-verify` runs there (ledger B1).' PRD-12:140: 'approves, fetches, checks, writes the row' becomes 'approves, fetches, copies the fetched entry and the per-run known-values file into the box, has `pad-run` run the checks there, writes the row from their JSON'. PRD-12:171-172: 0:27 conductor 'copy out ...; run `fetch-run`; `sandbox copy` the fetched entry and the per-run known-values file in; `sandbox exec` the four commands of section 8; copy out their JSON; `sandbox remove pad-<id>`'; 0:28 'write `row.json` and `report.txt`; scan the pad's output folder; append to the ledger'. PRD-12:320-322: 'The per-run known-values file holds the values that entered this box (its keys, its repo key), every planted value and the canary; it is made on the conductor, 0600, copied into the box for the checks, and removed with the box.' PRD-12:342: 'The conductor runs, for each run:' becomes '`pad-run` runs in the box, after the conductor copied in the fetched entry and the per-run known-values file:'. PRD-12:505: 'on the conductor' becomes 'in a build box'. PRD-07:570-574: 'run `goalcup-verify check --server <out>/<entry> --local <entry dir>`' becomes 'copy `<out>/<entry>` into the pad box and run `goalcup-verify check --server ... --local ...` there'. PRD-16 X6, X7 (278-279) and builder notes (590): add 'in the run's pad box'. PRD-16 N6 (502): add 'run in one check box from the build snapshot, `--non-persistent`, holding the full known-values file for the scan only and removed right after (a `decision` line: the one step where all keys meet a box)'. PRD-09:98-100: 'During the build the conductor does only the steps that need credentials: it lists sealed entries (read-only database), runs `fetch-run`, copies the fetched folder into a build box, and puts the three output files into `goalcup-usage-staging`. `goalcup-verify usage` itself runs in the build box, which holds no credential (ledger B1). The conductor never compiles or runs the reader. The production job is the owner's to start, on an organizer machine.' PRD-09:442 `scripts/usage-job.sh` and risks 579-582: the same split.
- Ledger item or rule: B1 (locked: builds, tests and real runs in Vercel Sandboxes; the cloud session orchestrates only)
- Source: align-L6 L6-05 (with align-PRD-09 P09-1)
- Also in: PRD-16-orchestration-acceptance, PRD-00-index, PRD-07-worker-server, PRD-09-usage-research-stats

## L3-3 (major)

- Section and lines: PRD-00 Definition of done (601-628), third slice (551); PRD-12 section 9 `--release-check` (437-441)
- Change: PRD-00, after done item 2 add: '2a. **HTTP/2 works:** PRD-03 acceptance 3 passes in a pad box (an `h2`-only client to a listed host: one `conn` with `alpn: "h2"`, four decoded request and response pairs, zero `h2_unsupported`), and `tests/proxy_h2.rs` is green in `gate full`.' PRD-12:438: after 'and the Codex kill row are PASS' add 'and the `h2-parallel.sh` exec row of PRD-03 acceptance 3 passed'.
- Ledger item or rule: H6 (locked: HTTP/2 is supported)
- Source: align-L3 L3-3
- Also in: PRD-00-index

## L5-03 (major)

- Section and lines: PRD-12 owner decisions (33-34), tier rule (427-430), `--release-check` (437-438), P3 (639), overridable defaults (708); PRD-00 done item 1; PRD-16 phase 4
- Change: PRD-12:427-430: 'the latest `gateway` and `dodger` runs, at least `smoke`, are PASS' becomes 'the latest `gateway` and `dodger` runs on `full` are PASS'. Same edit in 33-34, 437-438 ('nine core `full` rows'), P3 639 ('`direct`, `gateway` and `dodger` on `full`') and the 'Tier needs' default 708. Keep `smoke` for phase 1a, the build's short tests and watcher reproduction (section 13 step 3). PRD-16 phase 4 and PRD-00 done item 1 say the same.
- Ledger item or rule: T5 (locked pad workload), T4, B3
- Source: align-L5 L5-03
- Also in: PRD-00-index, PRD-16-orchestration-acceptance

## L5-04 (major)

- Section and lines: PRD-16 phase 5 (240); PRD-11 section 7 outlook column (662-678); PRD-12 risk row (677)
- Change: PRD-16:240: add 'As each harness's first `full` sample lands, a PRD-09 builder writes its reader from that sample (PRD-09 rule 250), then the row is rerun once. A harness with no reader by the end of phase 5 is "not verified: no log reader".' PRD-11 outlook column: append '; needs a PRD-09 reader written from its phase 5 sample' to every row except Claude Code, Codex and Hermes. PRD-12:677: 'unless PRD-09 writes a reader from their phase 5 sample in time' becomes 'until the PRD-16 phase 5 reader step writes one from their sample'.
- Ledger item or rule: T1, T3, T4 (wire calls match the harness's own log; usage stats present), S2
- Source: align-L5 L5-04
- Also in: PRD-16-orchestration-acceptance, PRD-11-connectors-harnesses

## L5-07 (major)

- Section and lines: PRD-12 section 3 step 0:01 (165), section 6 `login` comment (284), section 7 keys (311-318), section 2 gateway row (152)
- Change: Section 6: add a key `key_route` with three values: `login` (key file on stdin to the `login` command), `file` (`key_file = "<path in the isolated home>"` and `key_line = "<format>"`, for example Gemini `{home}/.gemini/.env` `GEMINI_API_KEY=...`, Copilot `{home}/providers.json`, Claude Code `apiKeyHelper` in `{home}/settings.json` that runs `cat` on a 0600 file), and `env` (the tmux shell runs `export NAME="$(cat <file>)"` before `goalcup start`, never echoed). Line 284: 'never an argument or a variable' becomes 'never an argument; a variable only through `key_route = "env"`'. Line 316: add 'or the tmux shell's environment for `key_route = "env"`'.
- Ledger item or rule: T2, T8 (Claude Code must pass); point 7
- Source: align-L5 L5-07

## L6-10 (major)

- Section and lines: PRD-12 section 3 row 0:01 (165); PRD-16 section 8 (290-291); BUILD-PROMPT section 2 (39-40)
- Change: PRD-12:165: move repo creation to the conductor at 0:00 and replace 'a new empty public repo with the GitHub token ..., its token in `.env.local` 0600' with 'the conductor makes the repo (prefix `gc-pad-`) and a deploy key with write access to that repo only (GitHub API `POST /repos/{owner}/{repo}/keys`); the box gets only that deploy key, 0600, and it goes in the per-run known-values file. The account token never enters a box.' PRD-16:290-291: 'create public, empty GitHub repositories' becomes 'create empty GitHub repositories for pad runs (prefix `gc-pad-`), private unless the staging Worker's start-time repository check needs them public (PRD-07), each with its own deploy key'. BUILD-PROMPT:39-40: 'A pad box holds one harness key for the life of its run, in a 0600 file.' becomes 'A pad box holds its harness key (and for the gateway case the gateway key) and a deploy key for its one test repo, each in a 0600 file, for the life of its run; never the GitHub account token.'
- Ledger item or rule: Point 4; B8 (autonomy bounded by AGENTS.md)
- Source: align-L6 L6-10
- Also in: PRD-16-orchestration-acceptance, BUILD-PROMPT

## L7-01 (major)

- Section and lines: PRD-12 section 1 output (132), 'Files it owns' (559-584); PRD-13 'Files it owns'; PRD-00 ownership table (no `.gitignore` row). Checked: repo `.gitignore` has no `pad-out` line today
- Change: PRD-12 'Files it owns', Change: add '`.gitignore`: add `/pad-out/` in the pad's first commit (phase 0), before any run writes there.' PRD-13 'Files it owns': add '`.gitignore`: remove the `spikes/*/out/` and `spikes/wire-pty/out/` lines in the commit that deletes `spikes/`.' PRD-00 ownership table: add a `.gitignore` row naming PRD-12 (the `pad-out` line) and PRD-13 (the `spikes` lines).
- Ledger item or rule: Section 7 'Never commit ... Stands'; point 4
- Source: align-L7 L7-01
- Also in: PRD-13-cuts, PRD-00-index

## L7-02 (major)

- Section and lines: PRD-12 section 6 pad harness file (273-296), 'Launch facts ... the fix goes to PRD-11' (292-296)
- Change: Section 6 TOML: add `[direct]` with `files = { ... }  # provider and model config copied into the isolated home, as a participant writes it` and `env = { ... }  # exported in the tmux shell before goalcup start, as a participant does`. After the block add: 'Provider, model and base URL settings are pad data here, never connector data: goalcup never sets a base URL (AGENTS.md hard rule 1).' Lines 292-296 become: 'Launch facts (CA variables, proxy settings that beat `HTTPS_PROXY`, daemons, auto-update switches, no-prompt values, session-file rows) are PRD-11's connector data. Provider, model and base URL settings are not: they stay in this file. If a pad run fails on a launch fact, the fix goes to PRD-11.'
- Ledger item or rule: Hard rule 'Never set a model base URL' (section 7 row 1); T2
- Source: align-L7 L7-02

## L5-08 (minor)

- Section and lines: PRD-11 builder notes step 1 (867-869); PRD-12 builder notes step 3 (723-724)
- Change: PRD-11 step 1: after 'Write the Claude Code, Codex, Hermes and Kilo files' add 'after reading each vendor's current docs or `--help` of the pinned version, cited in the commit (ledger T7)'. PRD-12 step 3: after '(harness files for Claude Code, Codex and Hermes first)' add the same sentence.
- Ledger item or rule: T7 (locked)
- Source: align-L5 L5-08
- Also in: PRD-11-connectors-harnesses

## L5-12 (minor)

- Section and lines: PRD-12 section 7 keys (310-312)
- Change: 'the owner puts each key in its own 0600 file' becomes '`pad` writes each key from the cloud environment secret named in BUILD-PROMPT section 3 into its own 0600 file under `~/.pad-keys/` once, at first use, with `umask 077` and no echo'.
- Ledger item or rule: T2, B7
- Source: align-L5 L5-12

## L5-13 (minor)

- Section and lines: PRD-11 OpenClaw connector (476-493), section 7 row 5 (668); PRD-12 section 8 INCONCLUSIVE causes
- Change: PRD-11 row 5 outlook: append 'Subagents need a Gateway daemon (appendix O H5.4); 1.0 does not start one, so C2 is expected INCONCLUSIVE "no subagent" and the tier "not verified: subagents need the Gateway" (owner list OW-10).' PRD-12 section 8 INCONCLUSIVE causes: add `harness_subagents_unavailable` with that sign.
- Ledger item or rule: T4, H7
- Source: align-L5 L5-13
- Also in: PRD-11-connectors-harnesses

## L6-18 (minor)

- Section and lines: PRD-16 S1 (332); PRD-12 section 13 steps 6 and 7 (545-547)
- Change: PRD-16:332 and PRD-12 step 7: 'on one signature' becomes 'on one cause (the cause the watcher names in step 2; the signature only groups rows)'. PRD-12 step 6: 'A different signature: the first cause counts as fixed and a new loop starts' becomes 'A different signature with the same named cause is one failed fix; a different named cause starts a new loop'.
- Ledger item or rule: B3 (locked: three failed fixes on one cause)
- Source: align-L6 L6-18
- Also in: PRD-16-orchestration-acceptance

## L7-04 (minor)

- Section and lines: PRD-12 P9 (645); risk row (691)
- Change: Replace the P9 tail and the risk row text with: 'Hermes on macOS reads the OS trust store (appendix O H3.3). goalcup never adds the run CA to a keychain; if Hermes cannot trust it there, Hermes on macOS is "not supported on macOS" with that reason.'
- Ledger item or rule: Hard rule 'Never write into any app-wide config' (section 7 row 4); T3
- Source: align-L7 L7-04
