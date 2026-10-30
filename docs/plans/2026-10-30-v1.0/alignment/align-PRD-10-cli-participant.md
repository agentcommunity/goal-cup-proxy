# Alignment review: PRD-10-cli-participant.md

Reviewed end to end (861 lines) against OWNER-DECISIONS.md (the ledger), VISION.md, AGENTS.md, BRIEF3, and
the parts of PRD-00, PRD-01, PRD-02, PRD-06, PRD-07, PRD-08, PRD-11, PRD-12, PRD-13 and PRD-17 that PRD-10
names as interfaces. 11 code facts spot-checked in `crates/goalcup-cli` and `crates/goalcup-platform`.

Counts: 1 blocker, 6 major, 16 minor.

## Findings

Format: id | file and section (lines) | what is wrong | ledger item, hard rule or point | severity | exact fix.

### Blocker

- **P10-01** | PRD-10 section 7 step 3 (lines 405-412), Asks C1 (line 770), Interfaces PRD-11 row (line 623) |
  The auto-update switch is bundled into the one no-prompt question: "n" keeps today's advice text, so a
  participant who declines the no-prompt (approval-off) settings also keeps auto-update on for Hermes,
  Gemini CLI and Qwen Code. PRD-10 also reads the switch from `no_prompts`, but PRD-11 (lines 157, 353,
  686, 746) defines a separate field `update_off`. A builder of `setup.rs` following PRD-10 would never
  read `update_off`, so the switch is never written. |
  Ledger H9 (locked: "Auto-updates off where the harness allows it"), H7 (locked: per harness data
  includes the auto-update switch). |
  blocker |
  Replace step 3's first sentence with: "goalcup always writes the connector's `update_off` values into
  that isolated home (no question: ledger H9). Then it asks one question about the `no_prompts` values:"
  and change the question text to drop "and to not update itself during the run". In the Interfaces
  PRD-11 row replace "`no_prompts` (incl. the settings-form auto-update switch)" with "`no_prompts`,
  `update_off`, `skills_dir`". Mark ask C1 "closed: PRD-11 added `update_off`". In section 1 setup row and
  acceptance 6 add "`--yes` and the interactive path both write `update_off`". PRD-00 line 436 (S3 field
  list) should add `update_off` and `skills_dir`.

### Major

- **P10-02** | Section 8 first bullet (line 455) vs acceptance 5 (line 677), Interfaces PRD-12 row (line
  624) and PRD-12 step 0:02 (PRD-12 line 166) |
  `--prompt FILE` is "real runs only", and line 98 defines a real run as no `--practice` and no `--test`.
  Acceptance 5 and the whole pad use `goalcup start --test --repo --prompt`. A builder following section
  8 refuses `--prompt` on every pad run, so no pad run can start as PRD-12 wrote it. |
  Ledger T4 and T5 (locked: the pad is the test), B3. |
  major |
  Line 455: replace "(new, real runs only)" with "(new, any run with `--repo`: competition and `--test`;
  with `--practice` or without `--repo` the file holds the prompt only, no push block)". Line 137: replace
  "a real run's `.env.local` has a plain `GH_TOKEN=` line" with "a run with `--repo` has a plain
  `GH_TOKEN=` line in `.env.local`".

- **P10-03** | Section 7 step 5 `Ready.` block (lines 423-425), help footer (line 522), section 3 step 2
  (lines 134-138) |
  The `Ready.` block tells the participant to run `goalcup start --repo ... --prompt prompt.txt` "in your
  empty project folder". A relative `prompt.txt` sits in that folder, and step 2 refuses any folder
  holding more than `.gitignore`, `.DS_Store` and `.env*` (RULES lines 142-145). The documented command
  refuses itself, or a builder exempts the file and the agent sees its prompt as a starter file. |
  Point 6 (tell plainly), point 7 (run the way people use it), RULES workspace rule. |
  major |
  Lines 424-425: change to "Your run, inside the start window, from your empty project folder, with the
  prompt saved outside it:" and "goalcup start --repo <you>/<repo> --prompt ~/goal-prompt.txt". Add to
  step 2: "The `--prompt` file must be outside the project folder; a prompt file inside it is named in
  the folder refusal with the fix 'move it out of the folder'." Add the case to `cli_folder.rs`.

- **P10-04** | Sections 4 and 5 (status and end screen), Asks A7, N7, N8 (lines 767-769) |
  The screens read facts that PRD-01 and PRD-02 do not define in their current text, although PRD-00
  rulings R3-2 and R3-4 accepted them: (a) `tunnel_hosts.<host>.listed` (PRD-01 COUNT example line 375
  has none); (b) notes `parse_stopped`, `native_missing`, `outlived` (missing from PRD-01's `what` list,
  line 465); (c) "first note of each `what`" and every COUNT key in `written` (PRD-02 line 307 still says
  "last 20"; its example lacks `req_holes`, `tunnel_hosts`, `mask_failclosed_bytes`); (d) `cut_files` in
  sealed `status.json` (absent from PRD-02). Also never asked anywhere: (e) gap counts by reason (rule 3
  "under heavy load only when the gap reasons are `memory_guard`"; COUNT has only `gaps` and
  `lost_bytes`); (f) `after_stop` connections and hosts (rule 6; only in the GAP 15 tail); (g) the last
  off span's start and end ("last: 15:43 to 15:45"; PRD-02 `off` is `spans`, `ms`, `since`). PRD-10 still
  lists A7, N7, N8 as "Left". A builder either cannot print these lines or adds counters in the CLI, which
  PRD-10's own trap forbids. |
  Ledger R6 (locked: counts are written counts), R8 (locked: record raw, the CLI does not parse),
  point 6. |
  major |
  In "Asks of other PRDs", move A7, N7, N8 to "ruled by PRD-00 R3-2 and R3-4; PRD-01 and PRD-02 must
  still apply them to their text". Add ask A8 of PRD-01: "COUNT and seal totals gain `gap_reasons`
  (`{reason: {n, lost}}`) and `after_stop` (`{conns, hosts}`)". Add to N8: "`off.last {start, end}` in
  `status.json`". Or, for less code, cut from the end screen the "(last: ... to ...)" span, the "under
  heavy load" qualifier and the after-stop host names, and say only what the existing keys hold.

- **P10-05** | Section 4 problem lines for `long_tunnel` (lines 260-261), section 4 first problem row
  (line 257) and section 6 (lines 378-383) |
  The fix printed live is "run: goalcup setup --record-host <host>, then goalcup continue". While the
  part records, `goalcup continue` returns `StillRecording` ("This part is still recording. To end it,
  run: goalcup seal", line 185), and the registered host only applies "from the next start". The
  participant follows the line and is refused; the model host stays a tunnel for the rest of the part. |
  Point 6 (tell plainly), point 1. |
  major |
  Replace the tail of both `long_tunnel` lines and the section 6 line with: "If it is a model service,
  run: goalcup setup --record-host <host>. goalcup records it from the next part: to start one now, run
  goalcup seal, then goalcup continue." Add to `cli_status.rs`: the printed fix names `seal` before
  `continue`.

- **P10-06** | Acceptance 2 (lines 666-670), builder notes "How a watcher proves it" (lines 809-813) |
  Not provable as written. The watcher saves `goalcup status --json` every 10 s, but the pass needs the
  human line ("Off:" within 2 s; "no status sample says Recording is off"). `--json` prints the files,
  not the lines, and a 10 s sample cannot show "within 2 s". PRD-12 also samples at 10 s (PRD-12 line
  219). |
  Ledger B3 (short real tests prove recording works), PRD-00 done items (real-run proofs). |
  major |
  Acceptance 2: "After each kill the watcher runs plain `goalcup status` once a second for 60 s and saves
  each output. Pass: the first saved output after the new boot's `status.json` `boot` changes shows the
  new 'Off:' count, within 2 s; none of the 60 saved outputs contains 'Recording is off'." Add the same
  1 s loop to the builder note.

- **P10-07** | Acceptance automated tests (lines 695-719) |
  PRD-17 routes lessons P14 and S7 to PRD-10 with PRD-10 as guard owner: P14 names
  "PRD-10 `cli_commands.rs` (no timer under 15 min before launch)" (PRD-17 line 251), S7 "a clock set back
  a day must not lengthen a segment" (line 277). PRD-10's test list has neither. PRD-17's deletion gate
  fails "a lesson with no test at all", so the old crates cannot be deleted. |
  Ledger L4 (locked: every lesson is an acceptance case the fresh core must pass). |
  major |
  Add to `cli_commands.rs`: "no timer before the launch is shorter than 15 minutes (P14): a fake device
  route that answers after 14 minutes on a paused clock still launches". Add `cli_start.rs`: "the
  deadline in the start facts comes from the server's `seconds_left` on the boot clock; a wall clock set
  back 24 h does not change it (S7)".

### Minor

- **P10-08** | Today table, false finish line (line 55) | "built at `:487-495`" is wrong: lines 485-496 are
  the sink shutdown; `recording_works_line` is at `finish.rs:500-510`. | Accuracy of code facts | minor |
  Replace ":487-495" with ":500-510".
- **P10-09** | Lines 43, 82-86, 113, 117, 405, 413, 442, 505, 643, 771 and others | IDs collide with ledger
  IDs: "H1", "H3", "H4", "H8", "H10", "H11" are appendix H items, not ledger H1 to H10; "F2" (disk) is
  ledger F2 (zstd); "F4", "F8"; "S3", "S7" (seam, PRD-17) vs ledger S3; ask "T1" vs ledger T1; "owner
  questions 8 to 11, 12, 15" use PRD-00's old numbers while PRD-00 question 8 is now the build budget. A
  reviewer checking against the ledger is misled. | Ledger header (the ledger is the reference) |
  minor | Write appendix items as "app. H 1", "app. F 8"; seams as "seam S3"; rename ask T1 to "K1";
  write owner questions as "PRD-00 question 2 (old 8 to 11)", "question 3 (old 12)", "question 4 (old
  15)", "question 1 (old 2)".
- **P10-10** | Risks, Size row (line 740) | Says "against PRD-00's 2,960"; PRD-00 now says 3,040 (line 766);
  PRD-13 line 467 still says 2,960. | Consistency | minor | Change to "matches PRD-00's 3,040; PRD-13's
  2,960 is stale and should read 3,040".
- **P10-11** | Interfaces PRD-08 row (line 622) vs PRD-08 line 305 | PRD-08 says `goalcup seal` prints a
  `goalcup-verify check` line; PRD-10 sends it to `goalcup.log` only. | Consistency | minor | Leave
  PRD-10; PRD-08 line 305 should read "the `goalcup-verify check` line goes to `goalcup.log` only".
- **P10-12** | Section 6 tier line (lines 386-389) | No line for an installed version that differs from the
  verified one; a builder may print "verified" for an untested version. | Ledger T3 (verified on a named
  version) | minor | Add: "`<harness> <version>: verified with goalcup on <v>, not on <version>.`".
- **P10-13** | Section 1 file connectors (line 101), acceptance 3 (line 674) | Only a `--connector` line is
  defined; a `--hosts`-only run (the dodger case) has none, yet acceptance 3 expects "the host-list path".
  PRD-12 line 153 expects the start line to say "not recorded", which PRD-10 never prints. | Point 6;
  consistency with PRD-12 | minor | Add "`Host list from a file: <path> (not verified).`"; PRD-12 line 153
  should drop "the start line".
- **P10-14** | Section 4 block and bullet (lines 234, 245-247), section 5 (line 303) | "(your agent's and
  its subagents')" is not true for a harness that keeps one file for all agents or totals only. | PRD-10
  goal "nothing on screen wider than the record holds", point 6 | minor | Drop the parenthesis, or print
  it only when the connector has a per-subagent native row.
- **P10-15** | Section 2 files (lines 118-120), section 4 `--json` (line 249) | Unclear whether the run
  credential is inside `server.json` or its own file. | Point 4 | minor | "the run credential in
  `runs/<run>/credential` (0600); `server.json` never holds it" and `"run":<server.json>`.
- **P10-16** | Section 3 step 8 (line 157) | PRD-11's loader allows `skills_dir` under `{workspace}`
  (PRD-11 line 163); PRD-10 would then write the skill into the project folder. | RULES "goalcup adds
  nothing to the folder"; hard rule on isolated homes | minor | Add: "Only under `{home}`; a `skills_dir`
  under `{workspace}` installs nothing and logs one line." Ask PRD-11 to allow `{home}` only for it.
- **P10-17** | Acceptance 1 (lines 661-665) | "exactly" cannot hold for MB and minutes on screen. |
  Provability | minor | "equals after the screen's rounding (bytes to MB with one decimal, times to the
  second)".
- **P10-18** | Acceptance (lines 656-693) | No real-run proof for `goalcup continue` (resume rule, version
  line) or for the live "No model request was written for 10 minutes" line, which PRD-00 names as how a
  missing host shows. | Point 6, point 1 | minor | Add to acceptance 3: "after minute 10 status shows the
  no-request `PROBLEM:` line". Add acceptance 10: a Codex `smoke` run, `goalcup seal`, `goalcup
  continue`; the harness opens with the connector's resume arguments.
- **P10-19** | Acceptance 7 (lines 686-688) | PRD-17 P11 cites "PRD-10 acceptance 8" for `goalcup` on PATH in
  a login shell; acceptance 8 is Version and acceptance 7 does not use a login shell. | Ledger L4 | minor |
  Acceptance 7: "once as `sh -lc 'goalcup time-left'` with the harness's `HOME` and env". PRD-17 P11 should
  cite acceptance 7.
- **P10-20** | Acceptance intro (line 658), builder notes | Says "Linux sandboxes"; does not say where
  `cli_*` tests compile and run. | Ledger B1 (locked) | minor | "Vercel Sandboxes (ledger B1); `cargo` builds
  and `tests/cli_*.rs` run there, never in the cloud coding environment."
- **P10-21** | Section 1 exit codes (lines 88-90), `cli_commands.rs` | clap exits 2 on a usage error by
  default; the test expects 1. Today's `main.rs:256` uses `try_parse` for that. | Builder misread | minor |
  Add a trap: "keep `Args::try_parse()` and map every parse error to exit 1".
- **P10-22** | Section 10 (line 498) | "the launch adds `GOALCUP_TIME_LEFT_FILE`": no owner; PRD-02 and PRD-11
  never mention it. | Builder misread | minor | "start step 9 adds it to `plan.env` before
  `watchdog::run`".
- **P10-23** | Section 8 (line 455), `cli_push.rs` | A prompt over 64 KiB is refused; RULES line 93 says a
  longer prompt "is not refused". | Point 3 (no refusal no point needs), less code | minor | Drop the size
  refusal and its default row; keep the UTF-8 check.

## Checked and aligned

- Ledger R6: every count on screen is `status.json` `written`; "Recording works" is gone; never "calls".
- R7: null mode row says "The server is told" (heartbeat `failed` is PRD-06 and PRD-07).
- R3, R4: restarting, off and losing states; every loss has a line with numbers; no failure budget.
- R5: only display limits (5 tunnel hosts, 20 notes); the full list goes to `goalcup.log`.
- R8 and the "record raw" rule: resume uses written counts, no session sniffing.
- L5: start hands over to `watchdog::run`; no helper stop or harness end in the CLI.
- L6, L7: `login`, `mcp`, `doctor`, `entries`, `--dev`, pins, disk handling, signing and 0.7 compat are cut.
- H1 to H5, H10: one pipeline; gateway shown like a direct key; dodger gets tunnel lines and its tier;
  vendor line; `--record-host` uses the one host check.
- M1 and point 4: `goalcup.log` takes no free string; push block holds no token; 0600 files.
- U1: "Removed here" names pruning. T3 tier words. T9 macOS by the owner after the build.
- B4: acceptance runs on staging. B5, B6: nothing here deploys or merges.
- Section 7 hard rules: no model base URL; D65 gone; D66 `time-left` kept; no harness names in
  `src/cli/`; nothing written into `~/.claude`, `~/.codex` or app-wide config (acceptance 6 checks it).
- PRD-00: seven commands, seams S11, S15, S16, the file list and phase order match. PRD-02: `status.json`
  modes, `rec/watchdog.pid` and SIGUSR1, `close_dead_entry` match. PRD-06: `server_word`, `upload_by`
  match. PRD-07: `/v1/health` exists (`index.ts:65`); floor in every mode matches.
- PRD-01: GAP reasons 1 to 15 and their numbers match the section 5 table; seal reasons match.
- Code facts checked and right: `main.rs:49-60` and `:81-240`, `login.rs:24-27`, `awake.rs:74-76` and
  `:93-106` (269 lines), `launch.rs:158-186` and `:181-182`, `status.rs:319-344`, `run/mod.rs:562-565`,
  `config.rs:39`, `server.rs:24`, `Cargo.toml:18`, RULES `:98-101` and `:142-151`, line counts of
  `folder_gate.rs`, `push_lines.rs`, `host_log.rs`, `skill.rs`, `mcp.rs`, `doctor.rs`. Wrong: P10-08.
- BUILD-PROMPT, PRD-16 and B1, B2, B2a text: not in scope for this file. PRD-10 carries no lane or
  reviewer rule that conflicts with them.
