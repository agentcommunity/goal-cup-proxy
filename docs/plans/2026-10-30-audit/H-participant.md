# Lane H: participant experience

Audit of what a participant does and sees, from install to the end of a run: every step they
take (the journey) and every message, warning and refusal the `goalcup` CLI can show them on the
terminal or in the run's `host.log` (the messages). Repo at commit 4b226de2 (branch
smart/intelligent-newton-0avrpi), read only. Nothing was built or run. Line counts are `wc -l`;
"src" is shipped code and "tests" is test code (inline `#[cfg(test)]` modules count as tests).
Anything not checked is marked "unverified".

The 2-hour run report (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on any
branch yet. No wait time below is measured from it, and no message is ranked by what that run showed.

## 1. Lane summary

Today a participant takes about 45 actions on macOS (49 on Windows with WSL2), with 3 browser
approvals and 2 hand edits of harness files, to do one practice run and one real run. The CLI can
put words in front of them from about 330 places, and a lean client for the seven points needs
about 60. Yet point 6 is not met: no message says plainly what was recorded, and nothing anywhere
counts session files or subagents. Several start refusals serve none of the seven points (disk
reserve, provider-host probe, `prep.json` gate, sync and Git folder checks), and the disk pause
stops the harness, against point 3. With the changes below the journey drops to about 26 to 31
actions, 2 approvals and 0 hand edits, and the end of every part says what was recorded.

## 2. Top findings

| # | Finding | Points | Verdict | Size | Ids below |
|---|---|---|---|---|---|
| 1 | No plain "what was recorded" line; no count of session or subagent files anywhere | 6, 2 | change: one "Recorded:" line at the end of each part and in `goalcup status` | about +25 src, +30 tests | F1, H12 |
| 2 | Disk reserve refuses starts; disk pause sends SIGSTOP to the harness | 3 | cut (owner decision) | about -90 src wording; `run/pause.rs` 211 src, 149 tests; `tests/full/t25_disk_full.rs` 1173 tests | F2, H2 |
| 3 | `goalcup login` is a whole browser approval that grants nothing | 5, 6 | cut (journey part); keep (messages part). See section 3 | about 170 src, 200 tests | H1 |
| 4 | Provider-host TCP probe refuses the start | 3, 7 | change to a warning | small | F3 |
| 5 | Recorder lost after 5 restarts refuses model connections; the advice is circular | 3 | change behaviour and words | other lane for behaviour | F7 |
| 6 | Two hand edits of harness settings, then a second `goalcup setup` | 7, 3 | change: seed no-prompt settings in goalcup's own home (owner call on D67) | similar size; about 60 doc lines go | H3 |
| 7 | Token file ritual: three OS variants and a strict line format | 4 | change: `start` asks for the token with hidden input; accept `export` and quotes | about +40 src | H5, F5 |
| 8 | `prep.json` gate, sync and Git folder refusals block starts | none | change to warnings or drop (parts disagree, section 3) | small | F4, H14 |
| 9 | Seven project-folder refusals stop at the first problem | rule | merge into one refusal that lists every problem | about -60 src | F5 |
| 10 | Technical lines (proxy admission, seal report, "Details:") on the terminal | none | move to `host.log`, print where it is | small, about 20 tests move | F6 |
| 11 | Time-left MCP tool is a manual step per harness | none | cut; keep `goalcup time-left` and the file | 273 src, 201 tests, 100 doc lines | H4 |
| 12 | Push block must be pasted at the start of the prompt | rule | change: write git setup into `.git/config` (owner call) | about 270 lines out, +30 in | H6 |

## 3. Where the two parts disagree

| Topic | Messages part | Journey part | Note |
|---|---|---|---|
| `goalcup login` | Keep: `approval_code`, `server_unreachable`, `login_done` (merged), "Keep the login check" (F4), and `--practice` should need "login only" (`src/commands/start/server.rs:37`) | Cut login entirely (H1); `start` uses the default server, a developer's local-only run takes an env var or flag | The shortest journey in section 4 assumes H1. If login stays, add 1 approval and 1 setup step on every OS. |
| Synced folder and Git work-tree checks | Change to warnings: they block start and serve no point (F4; `src/per_run_home.rs:299`, `:340`, `:356`; `project_synced`, `src/participant/refusals.rs:68-73`) | Keep the folder checks, synced folder and inside a repo included: "stop the token being published or uploaded", point 4 (H14; `start/project.rs:33-76`) | Owner call. Both agree the token must not reach a public repo; they differ on whether the participant's own sync service is a risk worth a refusal. |
| SSH-key warning | Cut setup's SSH lines and the `ssh_key`, `ssh_agent` doctor checks (F10; `src/ssh_keys.rs` 192 src, 49 tests) | Keep for Friday, cut later: the owner decided it on 26 Oct and it never blocks (H14) | The owner decision wins for Friday. |
| Disk after the cut | Cut every disk message, with no replacement (F2) | Keep one status line that says the disk is full (H2) | One line is cheap; it is not "special handling" of the run. |
| Push block | `prompt_block` (7 lines of git setup, `src/participant/launch.rs:158-186`): keep, it serves a rule | Change (owner call): write the remote and credential helper into `.git/config` (H6) | If the owner says no to H6, keep the block as is: it works. |
| Line numbers of the "Recording works" line | Printed at `src/commands/start/finish.rs:441-444`, built at `:487-495` | `finish.rs:497-507` | Both read the same commit; one cites the call, the other the builder body. Verify before editing. |
| Seal report lines | `src/commands/seal.rs:400-455` | `seal.rs:399-455` | Same block. |
| Token line form | Relax `token_line_form` to read `export GH_TOKEN=` and quoted values (F5) | Make `start` ask for the token and write `.env.local` itself (H5) | Compatible: do both. The by-hand path still works. |

## 4. The shortest journey per OS

This is the journey with H1, H3, H4, H5, H6, H7, H8, H10 and H11 done (journey part, section 4).
Counts are human actions on the happy path, one practice run, Claude Code as the harness.

### Common Goal Cup path (all OS)

1. Install: `curl ... | sh`; if told, paste the one PATH line. (1-2)
2. `goalcup setup`: pick the harness (Enter), it opens in its Goal Cup home with no-prompt settings
   already set, sign in, quit. (4: run, Enter, sign in, quit)
3. Practice: paste the printed line, approve in the browser, answer the harness screen, type a
   prompt, wait. (5)
4. GitHub: new empty public repo; a fine-grained token for it; `mkdir` and `cd`. (3)
5. Start: `goalcup start --repo you/name`, paste the token when asked, approve, answer the
   harness screen, paste the prompt. (5)
6. End: wait for "Upload done."; delete the token; log the harness out; hand in the video. (4)

### Per OS

| OS | Steps before Goal Cup | Goal Cup path | Total | Was | Extra notes |
|---|---|---|---|---|---|
| macOS | 4: account, verify email, accept terms, install the harness | 22-23 | **26-27** | 45 | I5 firewall approval only if one such as Little Snitch is installed |
| Linux desktop | 4: same | 22-24 (+1 if `xclip` or `wl-paste` is missing; after H5 the clipboard is not needed for the token) | **26-28** | 45-46 | native harness binary first on PATH |
| Linux headless (SSH) | 4: same | as Linux, plus `GOALCUP_NO_BROWSER=1` and opening the approval address on another device, and run inside tmux | about 27-29 (derived from the part's +1, not counted in the part) | 46-47 | |
| Windows (WSL2) | 8: the four above plus `wsl --install` in admin PowerShell (UAC), restart, first Ubuntu start with a Linux user, Windows sleep set to Never | 22-23 | **30-31** | 49 | the WSL carriage-return trap goes with H5 |

| | macOS | Linux | WSL |
|---|---|---|---|
| Browser approvals with a code | 2 (was 3) | 2 | 2 |
| Hand edits of harness files | 0 (was 2) | 0 | 0 |

If the owner keeps the push block (H6 no), add 1 step everywhere. If the owner keeps D67 (H3 no),
add 2. If login stays (section 3), add 1 approval and 1 setup step. The seven points still hold:
every model call and subagent file is recorded the same way (1, 2); nothing pauses the harness
after H2 (3); the token is in the folder at start and masked by value (4); every run is approved
and uploaded (5); the end block says plainly what was recorded (6, F1 and H12); the harness runs in
its normal chat mode (7).

What cannot go: the WSL install and restart (Microsoft's), the account and terms (the site's),
the GitHub repo and token (the event's submission rule), one approval per run (the account tie for
uploads), and the harness's own sign-in.

## 5. The message list to keep

About 60 messages, built from the "keep", "merge" and "change" verdicts in section 7. Paths are
under `crates/goalcup-cli/` unless noted. Words that scripts read must not change:
`READY_MARK` "Recording in chat mode", "shows the code XXXX-XXXX.", and `PROBLEM:`
(`scripts/matrix.sh`, `scripts/linux-matrix.sh`, `scripts/linux-harness-smoke.sh`,
`scripts/test_matrix.py`).

| Group | Keep | Source |
|---|---|---|
| Contract marks | `READY_MARK`, "shows the code", `PROBLEM:` | `src/participant.rs:76`; `src/participant/approval.rs:46-69`; `src/participant/status.rs:7-10` |
| Fragments | `CONTINUE_HINT`, `KEEP_GOING`, `STILL_OPEN`, `TIME_IS_UP`, `NOTHING_MORE`, `RUN_AGAIN`, `SUPPORT_ADDRESS`, `EARLIER_PART_UPLOADING`, `NOT_STARTED_HERE` | `src/participant.rs:79-110` |
| Approval | one-line intro (merged), `approval_code`, `replaces_open_run`, `APPROVED`, one "approval failed" builder with a reason (denied, expired, interrupted), `server_unreachable`; `login_done` only if login stays | `src/participant/approval.rs:16-126` |
| Launch | `launch_block` without the pre-0.7.1 branch, `PUSH_BLOCK_ON_CLIPBOARD`, `RUN_GOES_ON`, `prompt_block` (unless H6) | `src/participant/launch.rs:53-186` |
| Start refusals | `home_busy`, `still_recording`, `needs_terminal`, `project_of_open_run`, one project-folder refusal that lists every problem (F5), `cannot_record_model_service`, `repository_not_empty`, `dev_competes`, `unsupported_filesystem`, `server_answer_unusable`, `not_started_here`, `local_only`, `run_over`, one "project folder is gone" refusal, `last_part_closing`, `continued_elsewhere`, pinned connector files changed, "Goal Cup could not start recording, so <harness> was not started", harness not on PATH, WSL1 or Windows drive, leftover process, install problem (one "reinstall one release" line), run credential (two) | `src/participant/refusals.rs`; `src/commands/start/phases.rs:98`; `src/commands/start/proxy.rs:92`; `src/launch.rs:451`, `:470`; `src/folder_gate.rs:101`; `src/subreaper.rs:163`; `src/config.rs:112`, `:158`; `src/account/credential.rs` |
| Start warnings | `filesystem_warning`, default host warning, provider host unreachable (F3), synced folder or Git work tree (F4, if the owner agrees), shared home was open to others (rotate keys), Ctrl-C may end this command too | `src/participant/refusals.rs:201-221`; `src/commands/start/refusals.rs:223-229`; `src/home.rs:376`; `src/launch.rs:985` |
| Seal | `seal_inside_harness`, `seal_question`, `seal_not_confirmed`, `seal_needs_terminal`, `seal_done` (without disk words), three endings: "still finishing, run goalcup seal again", "could not finish, write to support", "already sealed" | `src/participant/seal.rs:7-79`; `src/commands/seal.rs:264-524` |
| End of a part | `helper_not_stopped`, `PRACTICE_NEXT`, `UPLOADING_REST`, `UPLOAD_DONE`, `ending_block` with the new "Recorded:" line, `after_upload`, the no-model-calls block, "Not recorded (passed through, unread): ...", "the model service could not be reached N times", "Details for support are in <host.log>." | `src/participant/ending.rs`; `src/commands/start/finish.rs:164-238`, `:446`, `:451` |
| Status | `STATUS_DETAILS`, `ends_line`, `status_header` (6 states plus "Recorded:"), `sending_words`, `status_next`, `no_run_yet`, `problem_state_stale`, recorder stopped (reworded, F7), `problem_recording_gaps`, `problem_recording_failed`, `CHECK_NETWORK`, one "upload is behind" line (F9), `problem_native_silent`, `problem_no_model_calls`, `problem_sleep`, `problem_agent_exited` (no disk branch), `agent_finished`, waiting for the recorder to reopen a run | `src/participant/status.rs`; `src/main.rs:761` |
| Upload | the 11 `upload_*` builders with `upload_not_opened` and `upload_local_run` merged, `entries_intro`, upload command lines | `src/participant/upload.rs:6-107`; `src/commands/upload.rs` |
| Setup | frame line, harness pick, home step, no-prompts line (or "what setup set", H3), battery and sleep as warnings, server and model service reachable, default hosts warning, git installed, WSL and filesystem checks, "Ready." block, "N of 4 steps need you" | `src/commands/prep.rs:358-1130` |
| Doctor | `manifest_pin` (shorter), `harness_manifest`, `harness_executable`, one recorder check, footer and exit codes | `src/preflight/gates.rs:108-160`; `src/commands/doctor.rs`; `src/main.rs:63-73` |
| Help | `AFTER_HELP` steps, guide, support address; per-command help | `src/main.rs:49-228` |
| Every error | `goalcup: <text>` | `src/main.rs:275` |

What goes: every disk message, `FREE_OUTSIDE`, `this_computer` vitals, the "Details:" block in the
default status view (kept under `--json`), proxy admission counters, the seal report, the
subreaper warning on the terminal, "harness exited: exit code N.", "uploaded: N bytes,
acknowledged, and the seal.", backward-compatibility messages, the `decoders` doctor check, and
"This command never exits with 2 or 4.".

## 6. The journey today (journey part)

Sources read: `packaging/install.sh`, `README.md`, `docs/PARTICIPANT-GUIDE.md`,
`docs/HARNESS-SETUP.md`, `docs/RULES.md`, `skills/goal-cup-setup/SKILL.md`,
`skills/goal-cup/SKILL.md`, `docs/research/PARTICIPANT-REVIEW-2026-10-21.md`, and the code of
`goalcup login`, `goalcup setup` and `goalcup start` (`crates/goalcup-cli/src/commands/login.rs`,
`commands/prep.rs`, `commands/start/*`, `commands/run_repo.rs`, `account/device.rs`,
`participant/approval.rs`, `participant/launch.rs`, `push_lines.rs`, `skill.rs`), plus
`server/worker/src/device.ts`. Wait times come from the code and docs only.

Example harness for the walk: Claude Code (`claude-code-cli`), a competition run with one practice
run first. Where another harness differs, the table "Per harness" says so.

### 6.1 Every step today

Kinds: **cmd** (type a command), **web** (do something in a browser), **appr** (browser approval
with a code), **sign** (sign in), **prompt** (answer a question in the terminal or harness),
**edit** (edit a file by hand), **wait**.

#### Before Goal Cup (outside this repo)

| # | Step | Kind | macOS | Linux | WSL | Evidence |
|---|---|---|---|---|---|---|
| P1 | Sign up for an Agent Community account | web, sign | yes | yes | yes | RULES.md:60 |
| P2 | Verify the email | wait, web | yes | yes | yes | RULES.md:60 |
| P3 | Accept the competition terms on the site | web | yes | yes | yes | RULES.md:63-64 |
| P4 | Install the harness (native binary first on PATH on Linux) | cmd | yes | yes | yes (inside Ubuntu) | README.md:185-187 |
| W1 | Admin PowerShell, `wsl --install` (UAC prompt) | cmd, appr | | | yes | README.md:190; [Microsoft](https://learn.microsoft.com/en-us/windows/wsl/install) |
| W2 | Restart Windows | wait | | | yes | [Microsoft](https://learn.microsoft.com/en-us/windows/wsl/install) |
| W3 | First Ubuntu start: wait to unpack, make a Linux user and password | wait, prompt | | | yes | [Microsoft](https://learn.microsoft.com/en-us/windows/wsl/install) |
| W4 | Set Windows sleep to Never (goalcup cannot hold a lock under WSL) | edit | | | yes | README.md:192-193; crates/goalcup-platform/src/awake.rs:24-26 |

#### Install

| # | Step | Kind | All OS | Evidence |
|---|---|---|---|---|
| I1 | `curl -fsSL .../install.sh \| sh` | cmd, wait | yes | install.sh:4, 290-300 |
| I2 | `goalcup --version` (the guide asks for it; the installer already prints the version) | cmd | yes | PARTICIPANT-GUIDE.md:28; install.sh:373 |
| I3 | `export PATH=...` when `~/.local/bin` is not on PATH | cmd | likely on a fresh Mac and on a fresh Ubuntu shell (unverified per machine) | install.sh:380-389 |
| I4 | Put the same line in the shell profile | edit | same as I3 | install.sh:387 |
| I5 | Allow `goalcup` in a firewall such as Little Snitch, again after each update | appr | macOS, only if one is installed | PARTICIPANT-GUIDE.md:64-65 |

#### `goalcup setup` (four steps)

| # | Step | Kind | Evidence |
|---|---|---|---|
| S1 | `goalcup setup` | cmd | prep.rs:355 |
| S2 | Login: the browser opens, sign in to Agent Community if needed, check the code, approve, go back to the terminal. Approval #1. Polls every 5 s, up to 15 min | appr, sign, wait | prep.rs:358-366; login.rs:51-57; device.rs:41-47, 216-277 |
| S3 | Pick the harness from a menu (type an id or press Enter) | prompt | prep.rs:595-634 |
| S4 | Read the guide link and the time-left lines to copy | read | prep.rs:715-723, 739-760 |
| S5 | "Press Enter to open" the harness in its Goal Cup home | prompt | prep.rs:819-826 |
| S6 | The harness's own first-run screens | prompt | HARNESS-SETUP.md:447-468 |
| S7 | Sign in to the harness (browser OAuth or paste an API key). On macOS Claude Code shares the login Keychain, so a signed-in user may skip it (measured per HARNESS-SETUP.md:409) | sign | prep.rs:826; HARNESS-SETUP.md:406-411 |
| S8 | Quit the harness | prompt | prep.rs:829-831 |
| S9 | Edit the harness settings file in the Goal Cup home to stop permission prompts (macOS path is `~/Library/Application Support/goalcup/capture-homes/...`) | edit | prep.rs:911-944; HARNESS-SETUP.md:41-44, 56-60, 126-133 |
| S10 | Add the time-left MCP tool by hand (a command or a file edit) | cmd or edit | prep.rs:739-760; HARNESS-SETUP.md:237-337 |
| S11 | Plug in a laptop: battery is a `todo` that holds back `Ready.` | prompt | prep.rs:968-972 |
| S12 | Optional: add a passphrase to an SSH key the `[warn]` names | cmd | prep.rs:431-441; ssh_keys.rs |
| S13 | Run `goalcup setup` again until `Ready.` | cmd | HARNESS-SETUP.md:44 |

#### Practice run (optional, strongly advised)

| # | Step | Kind | Evidence |
|---|---|---|---|
| R1 | Paste `d=$(mktemp -d ...) && cd "$d" && goalcup start --harness <id> --practice` | cmd | prep.rs:459-465 |
| R2 | Approval #2 in the browser, go back to the terminal | appr, wait | start/server.rs:110-118; approval.rs:16-41 |
| R3 | Claude Code asks "trust this folder" (every run), later "fullscreen renderer" | prompt | HARNESS-SETUP.md:460-462 |
| R4 | Type a prompt; ask the agent to call `time_left` | prompt | HARNESS-SETUP.md:48-49 |
| R5 | Wait about 10 minutes, then "Upload done." | wait | PARTICIPANT-GUIDE.md:77-79 |

#### Real run: GitHub and the project folder

| # | Step | Kind | macOS | Linux | WSL | Evidence |
|---|---|---|---|---|---|---|
| G1 | Make a new public empty repo on github.com/new | web | yes | yes | yes | PARTICIPANT-GUIDE.md:109-111 |
| G2 | Make a fine-grained token: about six choices (only this repo, Contents read and write, 30 days, copy) | web | yes | yes | yes | PARTICIPANT-GUIDE.md:112-119 |
| G3 | `mkdir -p ~/code/my-cup-run && cd ...` | cmd | yes | yes | yes | PARTICIPANT-GUIDE.md:137 |
| G4 | `printf '.env*\n' > .gitignore` | cmd | yes | yes | yes | PARTICIPANT-GUIDE.md:138 |
| G5 | Write `.env.local` from the clipboard: `pbpaste` / `xclip` or `wl-paste` / `powershell.exe Get-Clipboard \| tr -d '\r'` | cmd | pbpaste | may need to install xclip first (unverified per distro) | the CR-stripping variant | PARTICIPANT-GUIDE.md:146-157 |
| G6 | `grep -c ...` must print 1 | cmd | yes | yes | yes | PARTICIPANT-GUIDE.md:147 |
| G7 | Write the prompt in a file first | edit | yes | yes | yes | PARTICIPANT-GUIDE.md:210-211 |
| G8 | Wait for the start window, 7 Nov 00:00 UTC | wait | yes | yes | yes | RULES.md:44 |

#### Start

| # | Step | Kind | Evidence |
|---|---|---|---|
| T1 | `goalcup start --harness <id> --repo <user>/<name>` (`--harness` is required although setup saved it) | cmd | main.rs:127-130; prep_file.rs:29-31 |
| T2 | Folder check, then a public, unauthenticated GitHub check (repo public and empty) | wait | start/project.rs:33-76; run_repo.rs:66-165 |
| T3 | Approval #3, "Approving replaces" note if a competition run is open, go back to the terminal | appr, wait | approval.rs:72-78; start/mod.rs:21-35 |
| T4 | Harness screens: trust folder (Yes), maybe "No, keep bypass permissions", fullscreen offer, MCP auth | prompt | HARNESS-SETUP.md:458-463 |
| T5 | Wait for the chat input, then paste the push block (clipboard if a tool was found) | prompt | launch.rs:95-118; push_lines.rs:61-155 |
| T6 | Paste the prompt, press Enter | prompt | launch.rs:146-150 |
| T7 | Keep the terminal open, the machine awake and on power | wait | PARTICIPANT-GUIDE.md:242-244 |

#### During and after

| # | Step | Kind | Evidence |
|---|---|---|---|
| D1 | Optional: `goalcup status` in a second terminal | cmd | PARTICIPANT-GUIDE.md:214-223 |
| D2 | If the agent stopped: `goalcup continue`, then type only "keep going" | cmd, prompt | ending.rs:58-69 |
| E1 | Keep the terminal open until "Upload done." | wait | PARTICIPANT-GUIDE.md:248-251 |
| E2 | Delete the token on GitHub | web | PARTICIPANT-GUIDE.md:255 |
| E3 | Delete `.env.local` | cmd | PARTICIPANT-GUIDE.md:256 |
| E4 | Log the harness out or rotate its key | sign | PARTICIPANT-GUIDE.md:256 |
| E5 | Demo video and questionnaire on the site within 24 h (and the hour-24 video, VISION.md) | web | PARTICIPANT-GUIDE.md:257-258 |

#### Per harness (what changes the count)

| Harness | Setup differences | Every-start screens |
|---|---|---|
| Claude Code | 2 keys in `settings.json`; MCP by `claude mcp add` | Trust folder every run, "keep bypass", fullscreen offer (HARNESS-SETUP.md:460-462) |
| Codex | 2 lines in `config.toml`; MCP block with `env_vars`; native binary on Linux | None measured (codex-cli/harness.toml:38-42) |
| Hermes | `approvals.mode: "off"`, `max_turns`; setup wizard; may need `hermes-agent==0.19.0` and `[mcp]` extra | If not set up: wizard, Hermes closes, `goalcup continue` (hermes-cli/harness.toml:13) |
| Amp | No permission setting; install `rg` | Sign-in if missing (amp-cli/harness.toml:10) |

### 6.2 Counts today

Counted as human actions on the happy path, one practice run, Claude Code. Waits and approvals are
counted inside the totals and also shown alone. Optional error paths are not counted.

| | macOS | Linux desktop | Linux headless (SSH) | Windows (WSL2) |
|---|---|---|---|---|
| Before Goal Cup (P1-P4, W1-W4) | 4 | 4 | 4 | 8 |
| Install (I1-I4) | 4 | 4 | 4 | 4 |
| Setup (S1-S11, S13) | 12 | 12 | 12 (+1: `GOALCUP_NO_BROWSER=1`, open the address on another device) | 12 |
| Practice (R1-R5) | 5 | 5 | 5 | 5 |
| GitHub and folder (G1-G8) | 8 | 8 (+1 if xclip is missing) | 8 | 8 |
| Start (T1-T7) | 7 | 7 | 7 (run inside tmux) | 7 |
| End (E1-E5) | 5 | 5 | 5 | 5 |
| **Total** | **45** | **45-46** | **46-47** | **49** |
| Browser approvals with a code | 3 | 3 | 3 | 3 (+UAC) |
| Sign-ins | 2-3 (account, harness, maybe GitHub) | 2-3 | 2-3 | 2-3 |
| Hand edits of harness files | 2 | 2 | 2 | 2 |
| Waits over a minute | 6 (download, 3 approvals, practice 10 min, final upload) | 6 | 6 | 8 (+restart, unpack) |

The two steps most likely to go wrong, from the docs' own warnings: the permission settings
(PARTICIPANT-REVIEW B1, HARNESS-SETUP.md:341-344: a Codex run waited 8.7 of 10 min at a prompt)
and the push block ("Do not skip it", PARTICIPANT-GUIDE.md:183-186).

## 7. The messages today (messages part)

Scope: every message, warning and refusal a participant can see from the `goalcup` CLI
(`crates/goalcup-cli`), on the terminal or in the run's `host.log`. In this section all paths are
under `crates/goalcup-cli/` unless they start with `crates/`, `scripts/` or `docs/`. Line numbers
are exact for the file as it stands.

Method: the wording module (`src/participant.rs`, `src/participant/*.rs`) was read in full, then
every `CliError::Refused(...)` and `CliError::Internal(...)` outside test modules was extracted, with
every `say(out, ...)`, `host_log::say/say_raw/say_loud/log_only`, `eprintln!`, `println!` and
`writeln!`. Setup (`src/commands/prep.rs`), doctor (`src/commands/doctor.rs`,
`src/preflight/gates.rs`), status (`src/commands/status.rs`), the finish of `goalcup start`
(`src/commands/start/finish.rs`, `phases.rs`, `forked.rs`, `proxy.rs`) and the clap help in
`src/main.rs` were read.

### 7.1 Counts

| Surface | Where | How many | Notes |
|---|---|---|---|
| Wording module | `src/participant.rs` + 8 files in `src/participant/` | 87 `pub fn` + 26 `pub const`; about 75 distinct messages, many with branches | 1,788 src lines, 702 test lines (`src/participant/tests.rs`, 16 tests) |
| Branches inside those builders | `status_header` (13 states, 9 upload words), `status_next` (9), `ending_block` (5), `after_upload` (7), `launch_block` (about 12 branches) | about 55 extra variants | |
| Refusal and error sites outside the module | 40 files | 179 sites: 151 `Refused`, 28 `Internal` | 41 wrap a `participant::` builder; about 138 carry their own text; 56 of those use words the module bans for participants (daemon, entry, chain, capture, connector, seal.json, see `src/participant.rs:7-8`) |
| Lines said during a run | `say(out, ...)` and `host_log::*` | 68 sites: 34 stdout, 20 stderr or `host.log`, 2 raw, 1 loud, 11 log only | plus 4 `eprintln!` (`src/main.rs:557`, `:597`, `:761`; `src/launch.rs:985`) |
| `goalcup setup` | `src/commands/prep.rs:340-480`, `:522-1135` | about 35 messages over 4 steps, plus 2 SSH lines, 1 old-token notice, the "Ready." block and the "N of 4 steps need you" line | 1,301 src, 63 test lines |
| `goalcup doctor` | `src/commands/doctor.rs`, `src/preflight/gates.rs` | 7 check ids, 18 `Check` constructions | 471 + 171 src lines |
| `goalcup status` technical block | `src/commands/status.rs:582-700` | 8 lines plus 4 freshness variants | |
| Daemon errors passed through | `crates/goalcup-protocol/src/control.rs:785-810` | 10 prefixes (`conflict:`, `preflight failed:`, `this platform is not supported in M1:` ...) | |
| Help text | `src/main.rs:49-240` | 1 footer, 1 doctor footer, 12 command helps | |
| Test-build only (not shipped) | `src/main.rs:399`, `:410`, `:436`, `:508`, `:634`; `src/subreaper.rs:546` | 6 | ignore |

Rough total: about 330 places that can put words in front of a participant (75 module messages
plus 55 variants, 138 own-text refusals, 68 said lines, about 40 setup and doctor lines, 10 daemon
prefixes). A lean client for the seven points needs about 60.

### 7.2 Inventory and verdicts

Verdict words: **keep** (needed for a point), **merge** (say it once, in one place), **change**
(keep the job, change words or behaviour), **cut** (no point needs it). "Rule" means it enforces a
Goal Cup rule from `docs/RULES.md`, not one of the seven points.

#### The wording module

| Message | Path:line | Point | Verdict | Why |
|---|---|---|---|---|
| `READY_MARK` "Recording in chat mode" | `src/participant.rs:76` | 7 | keep | scripts wait for it (`scripts/matrix.sh:1762`, `scripts/linux-harness-smoke.sh:151`) |
| `CONTINUE_HINT`, `KEEP_GOING`, `STILL_OPEN`, `TIME_IS_UP`, `NOTHING_MORE`, `RUN_AGAIN`, `SUPPORT_ADDRESS` | `src/participant.rs:79-110` | 6 | keep | fragments reused everywhere |
| `DETAILS_HEADING` "Details for support" | `src/participant.rs:91` | none | change | heads a block of technical lines on the terminal; send that block to `host.log` and print one line that says where it is |
| `EARLIER_PART_UPLOADING` | `src/participant.rs:97` | 5 | keep | |
| `NOT_STARTED_HERE` | `src/participant.rs:100` | 5 | keep | |
| `FREE_OUTSIDE` (disk advice) | `src/participant.rs:106` | none | cut | owner: disk full gets no special handling |
| `approval_intro` (2 lines each) | `src/participant/approval.rs:16-41` | none | merge | "two approvals are normal" is said in the intro and again in `login_done:85`; one line is enough |
| `approval_code` | `approval.rs:46-69` | 5 | keep | "shows the code" is read by `scripts/matrix.sh:828`, `scripts/linux-matrix.sh:247` |
| `replaces_open_run` | `approval.rs:73-78` | rule | keep | prevents an accidental replacement |
| `login_done` | `approval.rs:82-93` | 5 | merge | repeats the intro's sentence (cut with login if H1 is taken) |
| `APPROVED` | `approval.rs:96` | 5 | keep | |
| `approval_denied`, `approval_expired`, `approval_interrupted` | `approval.rs:99-118` | 5 | merge | three refusals, one shape: "Run the command again. <why>. Nothing was created." One builder with a reason |
| `server_unreachable` | `approval.rs:121-126` | 5 | keep | |
| `details_heading` | `src/participant/ending.rs:7-13` | none | change | see `DETAILS_HEADING` |
| `helper_not_stopped` | `ending.rs:20-22` | 6 | keep | tells that open calls were cut, so not recorded |
| `PRACTICE_NEXT`, `UPLOADING_REST`, `UPLOAD_DONE` | `ending.rs:25-33` | 5, 6 | keep | |
| `ending_block` (5 endings) | `ending.rs:60-95` | 6 | keep, change | says how the agent ended, never what was recorded. Add the "Recorded:" line here (F1) |
| `after_upload` (7 variants) | `ending.rs:100-143` | 5 | keep | |
| `launch_block` | `src/participant/launch.rs:62-147` | 6, 7 | keep, cut one branch | the branch for "a run started before 0.7.1 kept no count" (`launch.rs:126-130`) is backward compatibility; cut |
| `PUSH_BLOCK_ON_CLIPBOARD`, `RUN_GOES_ON` | `launch.rs:53-58` | rule, 7 | keep | |
| `prompt_block` (7 lines of git setup) | `launch.rs:158-186` | rule | keep | the repository is a rule; not a point (the journey part would change this, H6) |
| `home_busy` | `src/participant/refusals.rs:14-27` | 7 | keep | real limit: one shared harness home |
| `still_recording` | `refusals.rs:30-36` | 7 | keep | |
| `needs_terminal` | `refusals.rs:39-45` | 7 | keep | chat-mode harness must own a terminal. Risk: the 2-hour run in a Vercel Sandbox needs a real TTY; unverified |
| `project_of_open_run` | `refusals.rs:54-64` | 5 | keep | one run per folder keeps IDs clean |
| `project_synced` | `refusals.rs:68-73` | none (4-adjacent) | change | turn into a warning; the sync service is the participant's own (the journey part keeps it, section 3) |
| `project_not_empty` | `refusals.rs:77-91` | rule | merge | see F5 |
| `cannot_record_model_service` | `refusals.rs:98-108` | 1 | keep | a local or plain-http model cannot be recorded |
| `repository_not_empty` | `refusals.rs:113-120` | rule | keep | the server refuses it too |
| `project_beside_goalcup` | `refusals.rs:124-132` | rule | merge | F5 |
| `project_inside_repository` | `refusals.rs:136-144` | rule | merge | F5 |
| `env_not_ignored` | `refusals.rs:151-160` | none (protects the token from the public repo) | merge | keep the check, fold into F5 |
| `token_line_form` | `refusals.rs:164-170` | none | change | exists only because the push line reads the token with `sed` (`launch.rs:181-182`); accept `export` and quotes, or warn |
| `token_line_missing` | `refusals.rs:175-188` | rule | merge | F5 (the journey part turns it into the token prompt, H5) |
| `dev_competes` | `refusals.rs:192-196` | rule | keep | saves an approval the server would refuse |
| `unsupported_filesystem`, `filesystem_warning` | `refusals.rs:201-221` | 5 | keep | warning only at start |
| `server_answer_unusable` | `refusals.rs:225-231` | 5 | keep | |
| `not_started_here`, `local_only`, `run_over` | `refusals.rs:234-256` | 5 | keep | |
| `project_missing`, `project_not_a_folder`, `project_unreadable` | `refusals.rs:266-296` | 5 | merge | three refusals, one fix ("put the folder back, or seal and upload the last part") |
| `last_part_closing`, `continued_elsewhere` | `refusals.rs:299-313` | 5 | keep | |
| `seal_inside_harness` | `src/participant/seal.rs:7-11` | rule | keep | |
| `seal_question`, `seal_not_confirmed`, `seal_needs_terminal` | `seal.rs:14-35` | 7 | keep | |
| `seal_done` | `seal.rs:52-79` | 6 | change | the "could not be sealed: the disk may be full" branch uses `FREE_OUTSIDE`; reword without the disk policy |
| `STATUS_DETAILS`, `PROBLEM` | `src/participant/status.rs:7-10` | 6 | keep | `PROBLEM:` is read by scripts |
| `ends_line` | `status.rs:105-111` | 6 | keep | |
| `status_header` (13 states) | `status.rs:124-193` | 6 | change | never says how many calls or subagent files were recorded (F1). The 13 state phrases can drop to 6 |
| `sending_words` | `status.rs:197-216` | 5 | keep | |
| `status_next` (9 variants) | `status.rs:244-268` | 6 | keep | |
| `no_run_yet` | `status.rs:272-276` | 6 | keep | |
| `problem_state_stale` | `status.rs:301-315` | 3, 6 | keep | |
| `problem_recorder_restarted` | `status.rs:319-325` | 3, 6 | change | says "If this repeats, run goalcup status", but this line is printed by `goalcup status` |
| `problem_recorder_lost` | `status.rs:329-333` | 3 | change | same circular advice; and the behaviour it reports blocks the harness (F7) |
| `problem_recording_gaps`, `problem_recording_failed` | `status.rs:335-344` | 3, 6 | keep | |
| `CHECK_NETWORK` | `status.rs:347-348` | 5 | keep | |
| `problem_upload` | `status.rs:352-364` | 5 | merge | F9 |
| `problem_server_refuses` | `status.rs:368-373` | 5 | merge | F9 |
| `problem_heartbeats` | `status.rs:376-378` | 5 | merge | F9 |
| `this_computer` (disk free, goalcup memory and CPU) | `status.rs:383-406` | none | cut | vitals; keep them in the heartbeat for the server only |
| `problem_disk`, `problem_disk_paused`, `disk_resumed`, `disk_resumed_unread` | `status.rs:408-431` | none | cut | owner: no disk handling (F2; the journey part would keep one "disk is full" line, section 3) |
| `NATIVE_SILENT_MARK`, `problem_native_silent` | `status.rs:435-453` | 2, 6 | keep | the only line that warns that session or subagent files are not being recorded |
| `NO_MODEL_CALLS_MARK`, `problem_no_model_calls` (running and ended forms) | `status.rs:456-509` | 1, 6 | keep | |
| `problem_sleep` | `status.rs:511-513` | none | keep (cheap) | a sleeping laptop ends the run's progress; one line |
| `problem_agent_exited` | `status.rs:516-543` | 6 | change | drop the `disk_stop` branch (`:528-531`) |
| `agent_finished` | `status.rs:546-552` | 6 | keep | |
| `time_left_unknown`, `no_open_run_time_left` | `src/participant/times.rs:158-169` | none | keep | serves the agent's time-left tool, not the seven points; out of this lane's message scope (see H4) |
| `upload_no_run` ... `upload_seal_missing` (11) | `src/participant/upload.rs:6-87` | 5 | keep; merge `upload_not_opened` + `upload_local_run`; change `upload_seal_missing` (drops disk words) | |
| `entries_intro` | `upload.rs:89-107` | 6 | keep | |

#### Refusals with their own text, outside the module

| File:lines | What they say | Point | Verdict |
|---|---|---|---|
| `src/launch.rs:451`, `:470` | harness not on PATH; script interpreter not on PATH | 7 | keep |
| `src/launch.rs:522`, `:537`, `:544`, `:566`, `:794`, `:819`, `:831`, `:893`, `:916`, `:1288`, `:1310` | path not UTF-8, symlink, not a file, quote in path, bad argument, unknown termination strategy, ARG_MAX, proxy URL form | none (internal guards) | change: one internal error text "goalcup could not launch <harness>: <why>. Write to support." |
| `src/launch.rs:869`, `:1550`, `:1568`, `:1778`, `:1782`, `:1789`, `:1869`, `:1897`, `:1907` | `Internal`: pid identity, spawn thread panicked, pre-start gate | none | keep as internal, shorten |
| `src/launch.rs:1032` | "could not be set up for recording" | 1 | keep |
| `src/ca_bundle.rs:181` | CA bundle problem | 1 | keep, shorten |
| `src/config.rs:112`, `:158`, `:309`, `:391`, `:407` | no config dir, no connectors dir, no daemon, control.json too large | none | keep `:112` and `:158` (install problems); `:407` is a cap on our own file, cut |
| `src/connector.rs:77-79`, `:100`, `:125` | unknown connector; two directories declare one id | 7 | keep `:100`; `:125` is developer text, change |
| `src/entries.rs:111`, `:202` | no record of entry; bad id | 5 | keep, reword "entry" to "part" |
| `src/folder_gate.rs:101` | WSL1 or Windows drive | 7 | keep |
| `src/home.rs:99`, `:240` | wrap `home_busy`, `still_recording` | 7 | keep |
| `src/home.rs:306`, `:389`, `:531`, `:569` | setup while a run records; unsafe shared home; internal; unknown home mode | 7 | keep `:306`; `:569` and `:531` internal |
| `src/model_host.rs:222` | model service cannot be recorded | 1 | keep |
| `src/per_run_home.rs:52`, `:72`, `:77`, `:188`, `:236`, `:257` | HOME unset, internal modes, not owner-private, read-back mismatch | 4, internal | keep `:52`, `:188`; others internal |
| `src/per_run_home.rs:299`, `:340`, `:356` | run directory inside a Git work tree or a synced folder | none (4-adjacent) | change to warnings; blocks start (F4) |
| `src/prep_file.rs:23`, `:52` | "This computer is not ready yet. Next: goalcup setup" | none | cut the gate; blocks start (F4) |
| `src/server.rs:65`, `:145`, `:183`, `:197`, `:203` | server URL, account handle, server.json mode, size, form | 5 | keep `:65`, merge the other four into "Run goalcup login again" |
| `src/subreaper.rs:163` | a process the harness left may still run | 3 | keep |
| `src/daemon/mod.rs:214`, `:246` | control API mismatch; non-loopback control URL | none | keep `:214` (install), shorten; `:246` internal |
| `src/daemon/spawn.rs:193`, `:489`, `:537`, `:565`, `:689`, `:701` | daemon from an older build, daemon binary missing, connectors dir mismatch, config path | none (install) | keep `:489`, `:701`; merge the rest into one "reinstall one release" line |
| `src/account/credential.rs:50`, `:57`, `:131`, `:143`, `:158`, `:165`, `:171` | run credential file problems | 5 | merge to two: "not on this computer" and "damaged, run from the computer that started it" |
| `src/account/device.rs:189`, `:191`, `:253`, `:261`, `:270`, `:281`, `:285`, `:303` | approval answers | 5 | keep (wrap the approval builders) |
| `src/account/github.rs:107` | "this clone was made by an older goalcup" | none | cut (backward compatibility) |
| `src/account/github.rs:121`, `:136`, `:142` | `github.json` from an older `goalcup prep` | none | cut (D69 says nothing uses it, `src/commands/prep.rs:437-446`) |
| `src/account/github.rs:190`, `:223`, `:230`, `:237`, `:313` | repo address form, empty or malformed token, repo not empty | rule | keep `:190`, `:313`; the token checks are for the old flow, check before cutting |
| `src/upload/mod.rs:132`, `:156`, `:202`, `:223`, `:281`, `:287` | server did not open or continue the run; receipt wrong | 5 | keep |
| `src/upload/uploader/chunk.rs:108`, `:113`, `:156`, `:167`, `:175`, `:200`, `:211` | chain and server copy disagree; chunk cap | 5 | keep as internal, reword "chain" |
| `src/upload/uploader/local.rs:34` | another goalcup uploads this part | 5 | keep |
| `src/run/mod.rs:1005`, `:1039`, `:1051` | bad run id; run file too large; no such run | 5 | keep `:1051`; `:1039` is a cap on our own file, cut |
| `src/run/reserve.rs:289-299` | "this run needs 3 GiB free plus 1 GiB" | none | cut; blocks start (F2) |
| `src/commands/continue_run.rs:74`, `:81`, `:116`, `:136`, `:174`, `:188`, `:212`, `:311` | continue refusals | 5 | keep; merge `:136` and `:188` (same advice) |
| `src/commands/doctor.rs:113`, `:175` | unknown connector; two directories | 7 | keep `:113` |
| `src/commands/login.rs:42`, `:60` | server.json unusable; server mixed up login and run | 5 | keep (cut with login if H1 is taken) |
| `src/commands/run_repo.rs:72` | repo question refusals | rule | keep |
| `src/commands/seal.rs:51`, `:75`, `:136` | wrap seal builders | 7 | keep |
| `src/commands/seal.rs:264`, `:480`, `:499`, `:503`, `:511`, `:518`, `:524` | recorder busy, torn trace, finish deadline, unreachable, unauthorized | 3, 5 | merge to three: "still finishing, run goalcup seal again", "could not finish, write to support", "already sealed" |
| `src/commands/status.rs:554`, `:570` | status refusals | 6 | keep |
| `src/commands/time_left.rs:47`, `:73`, `:108` | time-left refusals | none | out of lane |
| `src/commands/upload.rs:104`, `:106`, `:227`, `:230`, `:269`, `:334` | wrap upload builders | 5 | keep |
| `src/commands/start/finish.rs:77` | internal | none | keep |
| `src/commands/start/forked.rs:249` | launch sealed under it | 3 | keep |
| `src/commands/start/phases.rs:98` | pinned connector files changed | 1, 4 (the host and masking rules come from the connector) | keep, shorten the text in `src/preflight/gates.rs:126-160` |
| `src/commands/start/phases.rs:113` | provider host unreachable (TCP probe) | none | change to a warning; blocks start (F3) |
| `src/commands/start/phases.rs:131` | model service cannot be recorded | 1 | keep |
| `src/commands/start/phases.rs:389` | internal | none | keep |
| `src/commands/start/project.rs:46`, `:56`, `:69`, `:116`, `:120`, `:176`, `:198` | the project folder refusals | rule | merge into one (F5) |
| `src/commands/start/proxy.rs:92`, `:205`, `:214`, `:310` | recording proxy did not start; producer not admitted; status not signed | 1 | keep, say "Goal Cup could not start recording, so <harness> was not started" |
| `src/commands/start/refusals.rs:19`, `:29`, `:50`, `:63`, `:83` | interrupted; pin refusal on continue; needs terminal | 5, 7 | keep |
| `src/commands/start/server.rs:37` | "`--practice` needs `goalcup login` and `goalcup setup` first" | 5 | change: login only (or nothing, if H1 is taken) |
| `src/commands/start/server.rs:45`, `:168`, `:264`, `:409`, `:434`, `:443`, `:480` | dev competes, server mix-ups, run over, continued elsewhere | 5 | keep |

#### Lines said during a run

| Site | Says | Point | Verdict |
|---|---|---|---|
| `src/main.rs:275` | every error, `goalcup: <text>` | all | keep |
| `src/main.rs:557`, `:597` | "could not become a child subreaper ... (docs/TERMINATION.md)" | 3 | cut to `host.log`; nothing to do for the participant |
| `src/main.rs:761` | waiting for the daemon to reopen a run | 3 | keep |
| `src/launch.rs:985` | Ctrl-C may end this command too (setup) | 7 | keep |
| `src/home.rs:376` | shared home was open to other users, rotate keys | 4 | keep |
| `src/commands/start/phases.rs:103` | pin warning under `--dev` | none | keep for developers |
| `src/commands/start/phases.rs:135` | default host unreachable warning | 7 | keep |
| `src/commands/start/phases.rs:908`, `:918`, `:926` | launch block; ids and folders to `host.log` | 6 | keep |
| `src/commands/start/phases.rs:1111` | "(the upload could not start: ...)" | 5 | keep |
| `src/commands/start/phases.rs:1187` | harness stopped itself, continued at once | 3 | keep in log |
| `src/commands/start/phases.rs:1221` | "deadline: <reason>. The agent is stopped and the run sealed now." | 6 | merge with `TIME_IS_UP` block; today both print |
| `src/commands/start/phases.rs:1293`, `:1321`, `:1324` | details heading; swept processes line | 3 | log |
| `src/commands/start/phases.rs:1356` | after-exit commands not run | none | log only |
| `src/commands/start/phases.rs:1410`, `:1424` | abandon sentence, or "Part X could not be cleaned up" with recorder words | 5 | keep, reword |
| `src/commands/start/finish.rs:120` | "harness exited: exit code N." | 6 | cut; `ending_block` says the same (`src/participant/ending.rs:65`) |
| `src/commands/start/finish.rs:164`, `:181` | ending block; no-model-calls block | 1, 6 | keep |
| `src/commands/start/finish.rs:190-191` | "Uploading the rest of the run..." | 5 | keep |
| `src/commands/start/finish.rs:205-216` | "uploaded: N bytes, acknowledged, and the seal." or "upload incomplete ..." | 5 | merge into `after_upload`; the plain block after it says the same |
| `src/commands/start/finish.rs:225-233` | earlier parts uploaded or incomplete | 5 | merge with `src/commands/start/mod.rs:565`, `:579`, `:583`, which say the same for continue |
| `src/commands/start/finish.rs:238` | `after_upload` | 5 | keep |
| `src/commands/start/finish.rs:422` | helper not stopped | 6 | keep |
| `src/commands/start/finish.rs:437` + `src/commands/start/proxy.rs:232-288` | "proxy admission: accepted=... intercepted=..." up to 30 counters | none on the terminal | change: write to `host.log` only |
| `src/commands/start/finish.rs:443` + `:487-495` | "Recording works: N model call(s) recorded from hosts." | 6 | change: move out of "Details", add subagent files and gaps (F1) |
| `src/commands/start/finish.rs:446` + `src/proxy/outage.rs:126-140` | "proxy upstream outage: t1 to t3, 3 failed connects" | 3, 6 | log, and one plain line "the model service could not be reached N times" |
| `src/commands/start/finish.rs:451` + `src/proxy/tunnels.rs:142-190` | "sealed hosts (passed through unread): host:443 x3 12 KiB" | 6 | keep, plain words: "Not recorded (passed through, unread): ..." |
| `src/commands/start/finish.rs:482` | sealed but report not written | 5 | keep |
| `src/commands/seal.rs:400-455` | "entry X: finished locally; its terminal record is N and seal.json binds it", "anchor: ...", "harness termination: ...", "trace:", "verify: goalcup-verify --capture-integrity" | none | change: `host.log` only; keep `trace:` for support |
| `src/commands/start/forked.rs:126`, `:157-159`, `:231`, `:267` | terminal modes; details; launch ended under a seal; reserve not released | 3 | log; cut `:267` with the reserve |
| `src/commands/start/server.rs:164`, `:346`, `:371` | "Approved."; repo not kept; run not given back | 5 | keep `:164`, log the rest |
| `src/commands/start/refusals.rs:134` | filesystem warning | 5 | keep |
| `src/commands/continue_run.rs:256` | earlier part uploads in the background | 5 | keep |
| `src/commands/run_repo.rs:88-158` | the repository questions | rule | keep |
| `src/commands/upload.rs:89`, `:137`, `:151`, `:162`, `:202`, `:243` | upload command lines | 5 | keep |
| `src/upload/uploader/heartbeat.rs:114`, `:136` | heartbeat failures; server refuses | 5 | merge F9 |
| `src/upload/uploader/lifecycle.rs:307` | "<label>: <error>; trying again in Ns" | 5 | merge with `upload_waiting` |
| `src/upload/uploader/session.rs:322`, `:506`; `src/upload/mod.rs:155`, `:220`; `src/upload/uploader/finish.rs:273` | chunk stored; server refuses; raw answers | 5 | log only (already) |
| `src/run/watch.rs:157`, `:210` | disk watch stopped or could not start | none | cut with F2 |
| `src/run/watch.rs:351` | time-left file not written | none | log only |
| `src/run/watch.rs:501` | "The disk is almost full, so your agent is paused." (`say_loud`, over the harness) | none, against 3 | cut (F2) |
| `src/proxy/http.rs:1376` | unsupported content codings | 1 | log only |
| `src/proxy/mod.rs:1081`, `:1092`, `:1103` | recorder stopped, restarted, lost | 3 | keep the note, change the behaviour (F7) |
| `src/account/device.rs:217`, `:226` | approval lines | 5 | keep |

#### Setup output (`goalcup setup`, `src/commands/prep.rs`)

| Message | Line | Point | Verdict |
|---|---|---|---|
| "Goal Cup setup. 4 steps. Ctrl-C stops at any time" | 358 | none | keep (frame) |
| login step: connecting, ok, todo, fail | 363, 522-536 | 5 | keep (cut if H1 is taken) |
| harness step: installed list, "Which one?", unknown id, none in chat mode | 556-726 (575, 618, 654, 659, 667, 680) | 7 | keep |
| home step: opens the harness in its Goal Cup home; todo and ok states | 763-870 (813, 823) | 7 | keep |
| no-prompts advice (D67) | 905-943 | 3 in spirit (a harness that stops to ask would wait for a week) | keep (H3 would replace it with a one-line "what setup set") |
| battery: "plug it in, a run lasts a week" | 974-979 | none | keep as a warning (cheap; H11) |
| disk fail below 4 GiB, todo below 3.25 GiB, unreadable | 946-947, 985-1020 | none | cut (F2) |
| server and model service reachable or not | 1053-1069 | 5 | keep as the one network check |
| default hosts warning | 1070-1080 | 7 | keep |
| git installed or not | 1081-1090 | rule | keep |
| WSL and filesystem checks | 1095-1112 | 7 | keep |
| sleep lock | 1113-1130 | none | keep (one line) |
| SSH key warning, SSH agent note (D73) | 418-429, `src/ssh_keys.rs:49-66` | none | cut (the journey part keeps it for Friday, section 3) |
| old GitHub token notice | 437-446 | none | cut (backward compatibility) |
| "Ready." block with practice and real commands | 450-462 | 7 | keep |
| "N of 4 steps need you" | 463-477 | none | keep |

#### Doctor output (`goalcup doctor`)

| Check id | Where | Point | Verdict |
|---|---|---|---|
| `manifest_pin` | `src/preflight/gates.rs:108-160` | 1, 4 | keep, shorter text |
| `harness_manifest` | `src/commands/doctor.rs:200-203` | 7 | keep |
| `harness_executable` | `doctor.rs:227-239` | 7 | keep |
| `decoders` ("add a measured decoder, or retain the streams as capture-only") | `doctor.rs:290-313` | none | cut from the participant report; developer words |
| `daemon` (5 variants) | `doctor.rs:331-391` | none | keep one: "the recorder is installed and matches this goalcup" |
| `ssh_key`, `ssh_agent` | `doctor.rs:439-451` | none | cut with setup's SSH lines (the journey part keeps them for Friday) |
| doctor footer and exit codes | `src/main.rs:63-73` | none | keep |

#### Status view (`goalcup status`)

Participant block: `status_header`, `ends_line`, `this_computer`, `PROBLEM:` lines, `agent_finished`,
the push section and `status_next` (`src/commands/status.rs:225-460`). Technical block under
"Details:" (`src/commands/status.rs:582-700`): part id and connector, freshness (4 variants,
`:670-700`), `capture: ... run ..., analysis ...`, the daemon sentence, `workspace:`,
`storage: X of Y bytes used, Z reserved, durable through seq N`, `gaps: N; assistance recorded: yes/no`,
`ingest: N producer(s) consumed, M refused admission`.

Verdict: keep the participant block, change it (F1). Cut the "Details:" block from the default
view and keep it under `--json`: it repeats the stale state as freshness, uses wire words (capture,
analysis, seq, producer, ingest) and the one useful number in it (gaps) belongs in the plain block.

#### Help text (`src/main.rs`)

| Part | Line | Verdict |
|---|---|---|
| `AFTER_HELP` steps, guide, support address | 49-60 | keep |
| "This command never exits with 2 or 4." | 59-60 | cut |
| `GitCredential` hidden command, "clones that older versions made (D69)" | 229-240, 327-336 | cut (backward compatibility; also `src/account/github.rs:371` onward) |
| per-command help | 81-228 | keep; `Start` help says "Until the agent exits, goalcup prints nothing here", which is right |

### 7.3 Messages that block a start

A start refusal is fine when a point needs it (nothing can be recorded) or when the harness could
not run anyway. These do neither, except the last two rows:

| Refusal | Path:line | Point served | What could go wrong if relaxed |
|---|---|---|---|
| Disk: needs 3 GiB reserve plus 1 GiB free | `src/run/reserve.rs:20-23`, `:289-299`; setup `src/commands/prep.rs:946-1020` | none; the owner said disk full gets no special handling | a run fills the disk and the harness stops; owner accepted |
| Provider host must answer a TCP connect within the budget | `src/commands/start/phases.rs:107-140`, `src/proxy/preflight.rs:39-93` | none; the harness would show its own network error | someone starts offline; `problem_no_model_calls` catches it in minutes |
| `prep.json` must exist ("This computer is not ready yet") | `src/prep_file.rs:23`, `:52`; `src/commands/start/server.rs:48` | none; `--harness` names the harness and `server.json` holds the login | a participant skips the harness sign-in; the harness asks at its first screen, which `docs/RULES.md:120` allows |
| `--practice` and `--test` need setup as well as login | `src/commands/start/server.rs:37` | login serves 5; setup serves none | none |
| `GH_TOKEN` line must have no `export`, quotes or spaces | `src/commands/start/project.rs:85-120`, `src/participant/refusals.rs:164-170` | none; only our `sed` push line needs it | the agent's first push fails if the form is odd; read the common forms instead |
| Project or run directory in a synced folder or Git work tree | `src/per_run_home.rs:299`, `:340`, `:356` | none (keeps the token off the participant's own cloud) | the token reaches the participant's own sync service; warn instead (the journey part keeps the refusal, section 3) |
| Folder rules (empty, `.gitignore` covers `.env`, outside Git, not beside goalcup) | `src/commands/start/project.rs:32-78`, `:176`, `:198` | rule, not a point | keep, but show all problems in one refusal (F5) |
| Pinned connector files | `src/commands/start/phases.rs:95-103` | 1, 4 | keep |

## 8. Findings

F-numbers come from the messages part, H-numbers from the journey part. Where both parts found the
same thing, the finding is merged and carries both ids. Each finding gives the point, what the
code does today, the verdict, the lines and the risk.

### F1 and H12. Point 6 is not met: no message says plainly what was recorded

- **Points:** 6, 2.
- **Today:** the only positive line is "Recording works: N model call(s) recorded from hosts"
  (`src/commands/start/finish.rs:441-444`, built at `:487-495`; the journey part cites
  `finish.rs:497-507`), plus the sealed hosts (`src/proxy/tunnels.rs:174`). It prints only when
  N > 0, only at the end of a part, and under the "Details for support" heading, between the
  30-counter "proxy admission" line and the seal report ("entry ..., terminal record, seal.json,
  anchor", `src/commands/seal.rs:399-455`). No message anywhere counts session files or subagents
  (point 2): grep for "subagent" finds no participant string in `src/`. `goalcup status` shows
  neither the call count it keeps in `state.json` (`src/run/mod.rs:266`) nor the file count the
  daemon signs (`crates/goalcup-protocol/src/control.rs:349`, `native_files_admitted`); its only
  recording facts are "gaps: N; assistance recorded: yes/no" in the technical block
  (`src/commands/status.rs:665-668`).
- **Verdict: change.** One plain line, in `ending_block` and in `status_header`:
  "Recorded: 1,204 model calls (api.anthropic.com), 37 session files (main agent and subagents),
  2 gaps. Not recorded: 3 connections to github.com, passed through unread. Upload: done."
  Move the entry and anchor lines to `host.log`.
- **Lines:** messages part: about +25 src, +30 tests; journey part: about +20, -30 src. The data exists.
- **Risk:** a count that is wrong is worse than no count, and a local count may disagree with the
  verifier later. Take the numbers from the same sources the seal uses, say "so far" while the run
  records, and word it as "recorded on this computer".

### F2 and H2. Cut every disk message, the disk start refusal and the disk pause

- **Point:** 3 (never block or slow the harness). Direct conflict.
- **Today:** under 1 GiB free beyond the reservation the watch tick sends SIGSTOP to the harness
  tree and SIGCONT above 3 GiB (`run/pause.rs:1-11`). Start refuses without 3 GiB reserve plus
  1 GiB free (`src/run/reserve.rs:289-299`). Setup fails below the reservation plus margin
  (`src/commands/prep.rs:946-1020`). The participant sees it in README.md:79-80,
  PARTICIPANT-GUIDE.md:243-244, RULES.md:322-325 and status lines (PARTICIPANT-GUIDE.md:324-325).
- **Verdict: cut**, as the owner decided ("Disk full: no special handling"). Messages to cut:
  `src/run/reserve.rs:289-299` (start refusal), `src/commands/prep.rs:946-1020` (setup fail and
  todo), `src/participant/status.rs:408-431` (`problem_disk`, `problem_disk_paused`,
  `disk_resumed`, `disk_resumed_unread`), `:40`, `:165`, `:528-531` (stopped for disk),
  `src/participant.rs:105-107` (`FREE_OUTSIDE`), `src/run/watch.rs:157`, `:210`, `:501`,
  `src/commands/start/forked.rs:267`, and the disk words in `seal_done` and `upload_seal_missing`.
  The journey part would keep one status line that says the disk is full (section 3).
- **Lines:** about 90 src lines of wording. Behind them: `run/pause.rs` 211 src, 149 tests;
  `src/run/reserve.rs` 956 lines, mostly tests (other lanes own it); `src/run/watch.rs` 718 src,
  767 tests; `tests/full/t25_disk_full.rs` 1173 tests; about 30 doc lines.
- **Steps saved:** none on the happy path; removes a status message family and a setup `fail`,
  and unblocks starts on small disks.
- **Risk:** a full disk now ends the run with the harness's own error, or makes gaps. The trace
  may end without a seal and the participant must run `goalcup seal` and `goalcup upload` by hand.
  The owner accepts this.

### H1. `goalcup login` is a whole browser approval that grants nothing

- **Point:** 5 (tie uploads to an account) is already met by the per-run approval. 6 (plain).
- **Today:** login stores only the server address and the account handle, nothing secret
  (login.rs:24-27, 67-73; device.rs:7-8, 349-356). The run approval works on its own: the Worker
  takes `purpose` "login" or "run" with no link between them (server/worker/src/device.ts:51,
  109-110). Login's real job in `start` is a mode switch: with no `server.json`, a competition start
  is a local-only run (start/server.rs:35-41; start/mod.rs:419-427). Six places explain "two
  approvals are normal" (approval.rs:19-23, 36-39, 83-89; RULES.md:10-11; skills/goal-cup-setup
  SKILL.md:32-35; PARTICIPANT-GUIDE.md:45-46).
- **Verdict: cut** (journey part). `start` always uses the default server; a developer's
  local-only run takes an explicit env var or flag. Setup loses its login step. The messages part
  keeps login and merges its wording only (section 3).
- **Lines:** login.rs 84 src; prep.rs:356-366 and 520-541 (about 30); approval.rs login text
  (about 25); device.rs Purpose::Login paths (about 20); Worker login branch (about 10);
  tests/it/login.rs 159 tests plus login cases in tests/it/upload_server_device.rs (529 lines, part).
  About 170 src, about 200 tests.
- **Steps saved:** 1 approval on every OS (S2).
- **Risk:** an account that has not accepted the terms is told at the first practice start, not in
  setup. Organizers on staging need the server address from an env var. `goalcup status` and
  `upload` read `server.json` today and need the run's own record instead (check before cutting).

### F3. The provider-host probe should warn, not refuse

- **Points:** 3 and 7.
- **Today:** `src/commands/start/phases.rs:107-140` refuses the start when any hard provider host
  does not answer a TCP connect within the budget.
- **Verdict: change** to the existing warning path (`default_host_warning`,
  `src/commands/start/refusals.rs:223-229`) for every host. Saves the `continuing` refusal variant
  (`src/proxy/preflight.rs:80-93`) and its tests.
- **Risk:** an offline start records nothing for a while; the no-model-calls line
  (`src/participant/status.rs:467-509`) already tells the participant.

### F7. The recorder-lost message reports a block of the harness, and its advice is circular

- **Point:** 3.
- **Today:** after 5 restarts in a run (`src/proxy/mod.rs:1022`, a cap of our own) or one failed
  rebind (`:1098-1105`), "new connections to the model service are refused", which blocks the
  harness. `problem_recorder_lost` and `problem_recorder_restarted`
  (`src/participant/status.rs:319-333`) then tell the participant to "run goalcup status", inside
  `goalcup status`.
- **Verdict: change** the behaviour (another lane: pass through unrecorded and note a gap) and the
  words ("Recording stopped; the agent keeps running; this part has a gap. Write to support with
  run X.").
- **Risk:** a pass-through bypass is a cheat path; the gap note must say when and how long.

### H3. Two hand edits of harness settings, then a second `goalcup setup`

- **Points:** 7 (the harness must run a week with nobody there), 3 (a prompt stalls the run).
- **Today:** setup only advises; it reads the settings and prints what is missing
  (prep.rs:911-944, D67). The person finds the Goal Cup home (a path with a space on macOS,
  HARNESS-SETUP.md:58), edits JSON, TOML or YAML, and runs setup again (HARNESS-SETUP.md:41-44).
  This was the reviewers' top problem (PARTICIPANT-REVIEW B1).
- **Verdict: change.** When goalcup creates its own home for a harness, it writes the
  connector's `no_prompts` values there once (and Claude Code's per-folder trust flag before each
  launch, if the key is confirmed; unverified). It prints one line saying what it set and where to
  change it. It never touches `~/.claude` or `~/.codex`.
- **Lines:** replaces no_prompts_advice (about 35 src) with a writer of similar size; the
  connector tables already hold the keys (claude-code-cli/harness.toml:23-31,
  codex-cli/harness.toml:21-29, hermes-cli/harness.toml:29-32). About 60 doc lines go.
- **Steps saved:** 2 (S9, S13), and on Claude Code maybe 1 per start (T4 trust screen).
- **Risk:** reverses D67, so it is an owner call. Bypass mode lets the agent change anything the
  user can: the line must say so plainly. A harness may rewrite its file on first run (Hermes's
  wizard and `config.yaml`: unverified) and drop the keys; setup should re-read and warn.

### H5 and F5. The token ritual and the project-folder refusals

- **Points:** 4 (the token value must be known at start to be masked,
  PARTICIPANT-GUIDE.md:160-162); the folder rules are a rule, not a point.
- **Today:** the participant makes `.gitignore`, writes `.env.local` from the clipboard with a
  per-OS command, checks it with `grep`, and `start` refuses any other form
  (start/project.rs:98-145; PARTICIPANT-GUIDE.md:132-168). `token_line_form` exists only because the
  push line reads the token with `sed` (`src/participant/launch.rs:181-182`). Under WSL the Windows
  clipboard adds a carriage return that start refuses (PARTICIPANT-GUIDE.md:151-157).
  `refuse_unready_project` (`src/commands/start/project.rs:32-78`) stops at the first problem, so a
  participant with three problems needs three tries, each after reading a long sentence.
- **Verdict: change.** (H5) When a real run's folder has no `GH_TOKEN`, `start` asks for it with
  hidden input, writes `.env.local` (mode 0600) and `.gitignore` (`.env*`), then goes on. The
  by-hand path still works, and (F5) it reads `export GH_TOKEN=` and quoted values. (F5) One
  refusal lists every remaining folder problem with its fix, then "Nothing was created."
- **Lines:** H5 about +40 src and about -35 doc lines (the refusal `token_line_missing` becomes the
  prompt); F5 saves about 60 lines in `src/participant/refusals.rs:54-188`.
- **Steps saved:** 3 (G4, G5, G6), on every OS, and the WSL carriage-return trap.
- **Risk:** goalcup now handles a secret it asks for (it already reads it,
  start/project.rs:333-350). The terminal must not echo it; it must never reach host.log. The
  goal-cup-setup skill says Goal Cup asks for no token (SKILL.md:21-24): update it. The folder
  problem list gets long; cap it at the problems found.

### F4. Drop the `prep.json` gate and the sync and Git refusals

- **Point:** none.
- **Today:** `src/prep_file.rs:23-60`, `src/commands/start/server.rs:37`, `:48`;
  `src/per_run_home.rs:299`, `:340`, `:356` refuse a start.
- **Verdict: change.** Keep the login check (if login stays). Drop the `prep.json` gate; turn
  the sync and Git checks into warnings. The journey part keeps the sync and Git checks as
  refusals (section 3).
- **Risk:** low; setup still writes `prep.json` for its own use, for the repository suggestion and
  for H7's saved harness.

### H6. The push block must be pasted at the start of the one prompt

- **Point:** none of the seven (an event rule: the judged repo).
- **Today:** start prints the block, copies it to a clipboard tool if it finds one (pbcopy,
  wl-copy, xclip, clip.exe), writes `push-lines.txt`, shows it again in status, and re-shows it on a
  `continue` with no recorded calls (launch.rs:33-37, 95-118, 151-186; push_lines.rs:61-155).
  Goal Cup writes nothing in the folder by rule (RULES.md:148-151).
- **Verdict: change (owner call).** Before launch, goalcup runs `git init` in the empty folder and
  writes the remote, the credential helper and `user.name`/`user.email` into `.git/config`. No
  commit. The prompt then only needs "commit and push often". The messages part keeps
  `prompt_block` as is (section 3).
- **Lines:** push_lines.rs 152 src, 61 tests; launch.rs about 60; status re-show; about 40 doc lines.
  About +30 src for the git setup.
- **Steps saved:** 1 (T5), the step most often warned about.
- **Risk:** changes the "workspace starts empty" and "goalcup adds nothing" rules (RULES.md:142-151).
  An agent that deletes `.git` and re-inits loses the remote, as an agent today that never ran the
  block. If the owner says no, keep H6 as is: it works.

### H4. The time-left MCP tool is a manual step per harness

- **Point:** none of the seven.
- **Today:** `goalcup mcp` serves `time_left` over stdio (commands/mcp.rs). Setup prints lines to
  copy into a harness file (prep.rs:730-760); the guide needs about 100 lines for four harnesses
  (HARNESS-SETUP.md:237-337), and "the agent calling it" is not measured for any harness
  (HARNESS-SETUP.md:279-280, 293-295, 313-314, 337). The skill already falls back to
  `goalcup time-left` and `$GOALCUP_TIME_LEFT_FILE` (skills/goal-cup/SKILL.md:127-132).
- **Verdict: cut** the MCP server; keep `goalcup time-left` (168 lines incl. tests) and the file.
- **Lines:** mcp.rs 273 src, 201 tests; prep.rs about 30; `[participant.time_left_tool]` tables in
  four connectors; about 100 doc lines.
- **Steps saved:** 1 (S10) and the R4 check.
- **Risk:** an agent whose shell cannot find `goalcup` and ignores the env file has no deadline.
  Hermes and Amp do not get the skill (RULES.md:430-431), so the participant writes it in the
  prompt. Fairness across harnesses is unchanged from today, since the MCP path is unmeasured.

### F6. Move technical lines from the terminal to `host.log`

- **Point:** 6 (plain).
- **Today:** none of these is plain and none tells the participant what to do: "harness exited:
  ..." (`src/commands/start/finish.rs:120`, duplicates `src/participant/ending.rs:65`), "proxy
  admission: ..." (`src/commands/start/finish.rs:437`, `src/commands/start/proxy.rs:232-288`),
  "uploaded: N bytes, acknowledged, and the seal." (`finish.rs:205-216`, duplicates
  `after_upload`), the seal report (`src/commands/seal.rs:400-455`: entry, terminal record,
  seal.json, anchor, termination, verify), the subreaper warning (`src/main.rs:557`, `:597`), and
  the status "Details:" block (`src/commands/status.rs:582-700`).
- **Verdict: change.** Print one line: "Details for support are in <host.log>."
- **Lines:** small saving (the routing exists in `src/host_log.rs:170-200`); about 20 tests that
  assert these lines on stdout must move.
- **Risk:** support loses the lines if `host.log` cannot be opened; keep the terminal copy only in
  that case.

### F8. Cut backward-compatibility messages

- **Point:** none. No backward compatibility before Edition 01.
- **Today and verdict: cut** `src/account/github.rs:100-145` (old clone token, old `github.json`),
  `src/commands/prep.rs:437-446` (old token notice), the hidden `git-credential` command
  (`src/main.rs:229-240`, `:327-336`, `src/account/github.rs:371` onward), the pre-0.7.1 launch
  branch (`src/participant/launch.rs:126-130`).
- **Lines:** about 150 src (to verify with the account lane).
- **Risk:** a tester with an old clone gets a plain git auth failure.

### F9. Merge the five "upload is behind" messages

- **Point:** 5.
- **Today:** `problem_upload`, `problem_server_refuses`, `problem_heartbeats`
  (`src/participant/status.rs:352-378`), `upload_waiting` (`src/participant/upload.rs:44-49`) and
  the lifecycle retry line (`src/upload/uploader/lifecycle.rs:307`).
- **Verdict: merge** into one problem line: "Upload is behind by <time> (<why>). Nothing is lost;
  it waits on this computer. <fix if any>".
- **Lines:** saves about 30 lines and several tests.
- **Risk:** the HTTP-status case (server-side fault, nothing to do) must keep its "Nothing to do".

### F10. Cut small lines no point needs

- **Today and verdict: cut** `this_computer` vitals (`src/participant/status.rs:383-406`), SSH key
  and agent warnings (`src/ssh_keys.rs`, 192 src and 49 test lines; `src/commands/prep.rs:418-429`;
  `src/commands/doctor.rs:439-451`), the `decoders` doctor check (`src/commands/doctor.rs:290-313`),
  the "never exits with 2 or 4" sentence (`src/main.rs:59-60`).
- **Lines:** about 260 src lines with `ssh_keys.rs`.
- **Risk:** a participant whose agent could push with their SSH key loses the hint; the guide can
  say it. The journey part keeps the SSH warning for Friday because the owner decided it on 26 Oct
  and it never blocks (H14, section 3).

### F11. 56 own-text refusals use internal words

- **Point:** 6.
- **Today:** the module's rule (`src/participant.rs:7-8`) bans "entry, segment, chain, seal.json,
  daemon", yet `src/commands/seal.rs:264-530`, `src/daemon/*.rs`, `src/config.rs:391-407`,
  `src/entries.rs:111`, `src/upload/uploader/chunk.rs` and the 10 daemon prefixes
  (`crates/goalcup-protocol/src/control.rs:785-810`, "this platform is not supported in M1") use them.
- **Verdict: change.** Map internal errors to three plain endings ("run the command again", "run
  goalcup seal", "write to support with run X") and log the detail.
- **Risk:** support needs the detail; keep it in `host.log`.

### H7. `--harness` is required on every start

- **Point:** 7.
- **Today:** `start` requires `--harness` (main.rs:127-130) though setup saved the choice in
  `prep.json` (prep_file.rs:29-31).
- **Verdict: change.** Default to the saved harness; the approval intro already names it
  (approval.rs:26-35).
- **Lines:** about +5 src. **Steps saved:** none counted, one fewer thing to mistype.
- **Risk:** someone with two harnesses set up starts the wrong one; the name is on screen first.

### H8. Install: the PATH lines and the `--version` step

- **Point:** 7.
- **Today:** two lines when `~/.local/bin` is not on PATH (install.sh:380-389); the guide also asks
  for `goalcup --version` though the installer prints it (install.sh:373).
- **Verdict: change.** Print one line that both exports and appends to the right profile (the
  script still edits nothing itself); drop the `--version` step from the docs.
- **Lines:** about 5. **Steps saved:** 2 (I2, I4).
- **Risk:** low; a wrong profile file for an unusual shell.

### H9. The installer's recording-run guard

- **Points:** 3 and 5 (an upgrade must not break a week-long run; releases go on during the cup,
  RULES.md:38).
- **Today:** about 90 lines of shell parse `goalcup status --json` with sed and refuse while a run
  records (install.sh:104-195); 7 tests in scripts/test_install.py (614 lines in all).
- **Verdict: keep** for Friday. Later, a daemon that accepts the next CLI version would let it go.
- **Risk of cutting:** a mid-run upgrade leaves `status` and `continue` unable to talk to the old
  daemon.

### H10. `goalcup setup --json` and the goal-cup-setup skill

- **Point:** none.
- **Today:** a JSON mode for an agent to drive setup (prep.rs:36-41, 445-449) and a 106-line skill.
  After H1 and H3, setup has one human step (sign in to the harness) and nothing an agent can do.
  The hidden `prep` alias exists for old users (prep.rs:1).
- **Verdict: cut** the skill, `--json` and the `prep` alias.
- **Lines:** skill 106; prep.rs about 40 src; JSON cases in tests/it/prep.rs (1600 lines, part).
- **Risk:** low. People who liked agent-led setup lose it.

### H11. Battery holds back `Ready.`

- **Point:** 3 (nothing should block that the run does not need).
- **Today:** on battery the machine step is `todo`, so setup does not print `Ready.`
  (prep.rs:968-972). Nothing in `start` needs mains power.
- **Verdict: change** to a warning (the messages part agrees: "keep as a warning").
- **Lines:** 1. **Steps saved:** 1 (S11) for laptop users at setup time.
- **Risk:** a laptop that sleeps or dies mid-run; the start screen and status still warn.

### H13. The isolated harness home (keep for Friday, flag for later)

- **Points:** 7 partly against (people's own login, settings, skills, MCP servers and git login
  are not there); 2 and 4 served (only this run's session files are read, the user's own store is
  never read, RULES.md:274-280).
- **Today:** a second sign-in, settings in an odd folder, and an empty run HOME, which is why a
  GitHub token is needed at all (HARNESS-SETUP.md:430-432; PARTICIPANT-GUIDE.md:119-123).
  home.rs 995 and per_run_home.rs 489 lines.
- **Verdict: keep** for Friday. Rebuilding capture scoping in 8 days is too risky. After Edition 01,
  measure recording from the user's own home, scoped by start time and workspace.
- **Risk of changing:** recording the user's other sessions (point 4), two runs mixing.

### H14. Smaller keeps

| Part | Point | Verdict | Why |
|---|---|---|---|
| Per-run browser approval (device.rs:165-278) | 5 | keep | It is the one account tie after H1 |
| Repo check before the approval (run_repo.rs:66-165) | none | keep | Saves a wasted approval; the server checks again |
| Folder checks: empty kit, `.gitignore` covers `.env`, synced folder, inside a repo (start/project.rs:33-76) | 4 | keep | Stop the token being published or uploaded (the messages part would make the synced and Git checks warnings, section 3) |
| WSL1 and `/mnt/c` refusal (folder_gate.rs:58-107) | 5 | keep | A run cannot keep its records there |
| Browser opener with WSL fallback (goalcup-platform/src/machine.rs:515-534) | 7 | keep | Opens the Windows browser from WSL |
| `first_screen` hints in connectors | 6 | keep | Data, no code |
| SSH-key warning (ssh_keys.rs 192 src, 49 tests) | none (agent reach) | keep for Friday, cut later | Owner decided it on 26 Oct; it never blocks |

### H15. Point 7 gap outside this lane

Only four harnesses ship (RULES.md:403-408). OpenCode and OpenClaw facts sheets exist only on
origin/opencode-openclaw; they add no journey step today. Not a journey cut, but the biggest gap
against point 7.

## 9. Ranked list (both parts)

1. **F1 and H12** Add one plain "Recorded:" line (calls, hosts, session and subagent files, gaps,
   not recorded, upload) to the end block and to `goalcup status`. Points 6 and 2. About +25 src.
2. **F2 and H2** Cut the disk start refusal, the disk pause and every disk message (owner decision).
   About -90 src wording; 211 src, 149 + 1173 test lines behind it. Unblocks starts on small disks.
3. **H1** Cut `goalcup login`. One approval fewer on every OS; about 170 src and 200 test lines.
   Risk: terms problems show at the first practice start. The messages part would keep it.
4. **F3** Turn the provider-host probe refusal into a warning. Points 3 and 7. Unblocks starts.
5. **F7** Stop refusing model connections when the recorder is lost; fix the circular advice. Point 3.
6. **H3** Seed no-prompt settings in goalcup's own home. Removes the top cause of stalled runs and
   2 steps. Needs an owner ruling on D67.
7. **H5 and F5** Ask for the token in `start`; accept common token-line forms; one refusal that
   lists every project-folder problem. 3 steps and three OS variants gone; fixes the WSL carriage
   return trap.
8. **F4** Drop the `prep.json` gate; make the sync and Git refusals warnings (owner call, the parts
   disagree). Unblocks starts.
9. **H6** Write the git setup into `.git/config`. Removes the most-warned step and about 270 lines.
   Needs an owner ruling on "goalcup adds nothing to the folder".
10. **H4** Cut the time-left MCP tool. 273 src, 201 test lines, 100 doc lines, 1 step.
11. **F6** Send technical finish and status lines to `host.log`; print where they are.
12. **F8** Cut backward-compatibility messages and the hidden `git-credential` command. About -150 src.
13. **F9** Merge the five upload-behind messages into one.
14. **F10** Cut vitals, the decoders check and the exit-code sentence (and the SSH warnings after
    Friday). About -260 src with `ssh_keys.rs`.
15. **F11** Replace internal words in about 56 refusals with three plain endings.
16. **H7, H8, H11** Small defaults: saved harness, one PATH line, battery as a warning.
17. **H10** Cut setup `--json`, the setup skill and the `prep` alias.
18. **Keep:** H9 install guard, H13 isolated home (for Friday), per-run approval, folder checks,
    repo pre-check.

## 10. Blind spots and unverified

- No step was timed on a real machine. The PATH state of a fresh Mac and Ubuntu shell, xclip
  availability, the Claude Code trust key and the Hermes wizard's handling of `config.yaml` are
  unverified.
- Whether the Vercel Sandbox gives the 2-hour run a real terminal for `needs_terminal` is unverified.
- The 2-hour run report (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on any
  branch yet, so no measured wait is quoted and no message is ranked by what that run showed.
- The Linux headless count in section 4 is derived from the journey part's "+1" notes, not counted
  in the part itself.
