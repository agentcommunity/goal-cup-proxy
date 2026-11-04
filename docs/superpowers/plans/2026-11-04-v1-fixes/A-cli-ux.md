# Lane A: Participant CLI usability
**Wave:** 1   **Depends on:** none   **Branch:** fix/cli-ux
**Covers findings:** usability-1, -2, -3, -4, -5, -6, -7, -8, -9, -10, -13, -14, -15, -16, cr-watchdog-6 (planned); usability-11, -12 (dropped)
**Owns files:** `crates/goalcup/src/cli/{finish,approve,awake,time_left,continue_run}.rs`, `crates/goalcup/src/main.rs`, README.md quick-start section, `server/worker/src/db-local.ts` window-refusal text only
**Shared files (only these parts):** `cli/start.rs` `check()` (163-237), `hand_over()` (101-146) and `first_message()` (402-429, plus one new `push_only`); `cli/text.rs` non-version messages only (not 176-182, not `tier_line` ~1158); `cli/setup.rs` lines 76-98 (tty/sign-in); `connector/mod.rs` `ConnectorError::UnknownId` and `load()` (~156-204, not ~242); `cli/status.rs` `run()`/`show()` (one line each, see A.7: not in the lane brief, flagged below). Tests under `crates/goalcup/tests/` and `server/worker/test/` that pin a changed behaviour.
**Produces (for other lanes):** `connector::resolve_id(arg: &str) -> Result<&'static str, ConnectorError>`; `ConnectorError::UnknownId { id: String, valid: String }`; `start::push_only(repo: &str, project: &Path, vars: &BTreeMap<String, String>) -> Vec<String>`; `time_left::line(upload, practice, deadline: Option<u64>, now)`.
**Consumes:** none. Lane E replaces the hand list in `builtin.rs` but keeps `builtin::CONNECTORS: &[(&str, &str)]`; A.4 only reads that constant.

## Dropped
- usability-11 (OpenCode wrong pick says "install that version"): lane F owns `text.rs` 176-182. A.4 only makes `--harness opencode` list both ids.
- usability-12 (tier line "not verified" with no action): lane F owns `tier_line`, lane G owns `tiers.toml`.
- cr-watchdog-6 half "`shield::hangup_seen()` is never called": not needed. Ignoring the write error fixes the panic and the lost next-step text (A.3).
- Early window refusal before the browser approval (second half of usability-6): the server sees the window only at `POST /v1/runs`. Making `POST /v1/device` refuse a competition request outside the window is a Worker change in lane cr-upload-server's files. See Open owner question 1.
- READY_MARK "Recording in chat mode" rename (part of usability-16): `scripts/pad/pad-run:419,431,476` and `test_pad.py` wait for that exact text. Kept; see A.16.

## Review focus
1. `--harness opencode` (two connectors share program `opencode`) must list both ids, not pick one (A.4).
2. A start with the harness missing must create nothing: no run dir, no `homes/<id>`, no call to the server (A.1 asserts all three).
3. Closed terminal during a real run: the end screen and the harness exit code must survive (A.3 spawns with a dead stdout).
4. `continue` after Ctrl-C-after-open in a competition run must still show the push lines (A.6); a plain Ctrl-C in practice shows none.
5. A run's `workspace` can be absent in old `server.json` (existing tests have none): A.15 must not refuse then.

### Task A.1: start refuses a harness that is not installed, before any approval   (size S, PARALLEL-OK)
**Files:** Modify `crates/goalcup/src/cli/start.rs` `check()` between the plan-free steps (insert before line 208 `let env = Env {`; the plan writes the harness home, so the test comes first). Test `crates/goalcup/tests/cli_start.rs`.
**Interfaces:** Consumes `connector::launch::resolve_program(&Path, &BTreeMap<String,String>) -> PathBuf`, `text::not_installed(&str)`. Produces none.
- [ ] Step 1: Write the failing test in `cli_start.rs`:
  ```rust
  #[test]
  fn a_harness_that_is_not_installed_stops_the_start_before_anything_is_created() {
      let fix = Fix::with_program("goalcup-no-such-harness-xyz", "{home}/skills", "");
      let w = FakeWorker::start();
      let stop = prepare(&fix.args(), &fix.ctx(Server::Url(w.url())), &mut Vec::new()).err().unwrap();
      assert_eq!(stop.code, 1);
      assert!(stop.text.contains("Tester is not installed"), "{}", stop.text);
      assert!(!fix.gc().exists());
      assert!(w.heartbeats().is_empty());
  }
  ```
  (Use the fake's recorded routes if `FakeWorker` exposes none besides `heartbeats`: add `pub fn calls(&self) -> usize` to `tests/common/worker.rs` counting every request, and assert `w.calls() == 0`.)
- [ ] Step 2: Run, expect FAIL (the start goes on to the approval and `fix.gc()` exists): `cargo nextest run -p goalcup --test cli_start a_harness_that_is_not_installed`
- [ ] Step 3: In `check()` after the folder problems and before `let env`, add: `if !resolve_program(&connector.program, &c.vars).is_absolute() { return Err(stop(text::not_installed(&connector.name))); }`. Change `text::not_installed` to end "Install it, then run the command again." (setup's caller keeps working: its text is only this sentence).
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test cli_start` and `cargo nextest run -p goalcup --test cli_setup yes_without_the_program_on_path_writes_nothing`
- [ ] Step 5: `git add crates/goalcup/src/cli/start.rs crates/goalcup/src/cli/text.rs crates/goalcup/tests/cli_start.rs crates/goalcup/tests/common/worker.rs && git commit -m "fix(cli): start refuses a harness that is not installed before the approval"`

### Task A.2: the end screen says why the harness did not start   (size S, PARALLEL-OK)
**Files:** Modify `crates/goalcup/src/cli/finish.rs` `closing()` (203-209) ; Modify `text.rs` (add `harness_not_started(name, kind: Option<&str>)`); Test `crates/goalcup/tests/cli_finish.rs`.
**Interfaces:** Produces private `fn spawn_failure(entry_dir: &Path) -> Option<String>` reading the last `harness_failed` line of `goalcup.log` (format `<time> harness_failed io=<Kind>`, written by `watchdog/mod.rs:360`).
- [ ] Step 1: Failing test in `cli_finish.rs`: sealed status with `"harness": null`, `goalcup.log` holding `2026-11-04T10:00:00Z harness_failed io=PermissionDenied\n`; assert `closing(&entry, "Agent", RUN, false, false)` contains `"PermissionDenied"` and still contains `text::PART_RECORDED`; a second case with no `harness_failed` line keeps the old "Agent did not start." and contains no "io=".
- [ ] Step 2: Run, expect FAIL: `cargo nextest run -p goalcup --test cli_finish closing_names_why_the_harness_did_not_start`
- [ ] Step 3: Implement `spawn_failure` (last line whose second word is `harness_failed`, value after `io=`) and use `text::harness_not_started` in place of the bare `format!("{name} did not start.\n\n")`. Text: "{name} did not start ({kind}). Check that it runs on its own, then run goalcup continue {run}." (needs `run`, already a `closing` argument).
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test cli_finish`
- [ ] Step 5: `git add crates/goalcup/src/cli/finish.rs crates/goalcup/src/cli/text.rs crates/goalcup/tests/cli_finish.rs && git commit -m "fix(cli): the end screen names the reason the harness did not start"`

### Task A.3: a closed terminal cannot panic the hand-over   (size S, PARALLEL-OK, Linux-only test)
**Files:** Modify `crates/goalcup/src/cli/start.rs` `hand_over()` lines 124, 138-144 (three `print!`/`println!`); Test `crates/goalcup/tests/cli_start.rs`.
**Interfaces:** Produces private `fn tell(line: &str)` = `let _ = writeln!(std::io::stdout(), "{line}")`.
- [ ] Step 1: Failing test next to `the_hand_over_passes_the_exit_code...`:
  ```rust
  #[cfg(target_os = "linux")]
  #[test]
  fn a_dead_stdout_does_not_panic_the_hand_over_and_the_exit_code_survives() {
      // spawn_start with stdout = write end of a pipe whose read end is already closed
      let out = spawn_start_with_dead_stdout(&fix_with_exit_7, &w.url());
      assert_eq!(out.status.code(), Some(7));
      assert!(!String::from_utf8_lossy(&out.stderr).contains("panicked"));
  }
  ```
  Refactor `spawn_start` to take a `stdout: Stdio`; dead stdout from `std::io::pipe()` with the reader dropped.
- [ ] Step 2: Run, expect FAIL (exit 101, stderr "failed printing to stdout"): `cargo nextest run -p goalcup --test cli_start a_dead_stdout`
- [ ] Step 3: Replace the three print sites in `hand_over` (`println!("{line}")` in the printer thread, `print!("{}", finish::closing(..))`, the `wrap_up` closure) with `tell`. The end-screen `print!` keeps its text; `tell` adds no newline for it (use `write!`).
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test cli_start`
- [ ] Step 5: `git add crates/goalcup/src/cli/start.rs crates/goalcup/tests/cli_start.rs && git commit -m "fix(cli): a closed terminal no longer panics the hand-over"`

### Task A.4: `--harness claude` works, and a wrong id lists the valid ones   (size M, PARALLEL-OK)
**Files:** Modify `crates/goalcup/src/connector/mod.rs` (`ConnectorError::UnknownId` ~156-157, `load()` ~190-204); Modify `crates/goalcup/src/main.rs` (help text of `--harness` on `Setup` and `Start`, lines 37-39, 49-51); Test `crates/goalcup/tests/connector_builtin.rs`, `crates/goalcup/tests/cli_commands.rs`.
**Interfaces:** Produces `pub fn resolve_id(arg: &str) -> Result<&'static str, ConnectorError>`; `UnknownId { id: String, valid: String }`, message `no built-in connector `{id}`. Choose one of: {valid}`. Rule: exact id wins; else the ids where `id == arg + "-cli"`, or the parsed connector's `program == arg`; exactly one match resolves, none or several is `UnknownId` (valid = the matches, else every id). `load()` calls it for non-path arguments. Consumes `builtin::CONNECTORS`.
- [ ] Step 1: Failing tests in `connector_builtin.rs`:
  ```rust
  #[test]
  fn a_program_name_picks_its_connector_and_an_ambiguous_one_lists_both() {
      assert_eq!(load("claude", None).unwrap().id, "claude-code-cli");
      assert_eq!(load("codex", None).unwrap().id, "codex-cli");
      assert_eq!(load("claude-code-cli", None).unwrap().id, "claude-code-cli");
      let both = load("opencode", None).unwrap_err().to_string();
      assert!(both.contains("opencode-cli") && both.contains("opencode2-cli"), "{both}");
      assert!(!both.contains("claude-code-cli"));
  }
  #[test]
  fn an_unknown_id_lists_every_valid_id() {
      let e = load("nonesuch", None).unwrap_err().to_string();
      for (id, _) in builtin::CONNECTORS { assert!(e.contains(id), "{e}"); }
  }
  ```
  In `cli_commands.rs`: `start --help` stdout contains `claude-code-cli` (help names the ids from `builtin::CONNECTORS`).
- [ ] Step 2: Run, expect FAIL: `cargo nextest run -p goalcup --test connector_builtin a_program_name`
- [ ] Step 3: Implement `resolve_id` and the new variant fields; build the help string once in `main.rs` with `format!` from `builtin::CONNECTORS` ids, passed as `long_help`.
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test connector_builtin --test cli_commands --test cli_setup`
- [ ] Step 5: `git add crates/goalcup/src/connector/mod.rs crates/goalcup/src/main.rs crates/goalcup/tests/connector_builtin.rs crates/goalcup/tests/cli_commands.rs && git commit -m "feat(cli): --harness takes the program name, and a wrong id lists the valid ones"`

### Task A.5: refusal texts name the right next step   (size S, PARALLEL-OK)
**Files:** Modify `crates/goalcup/src/cli/text.rs`: `opened_then_stopped` (281-286), `repo_not_empty` (308-312), `open_refused` (288-290), `approval_refused` (276-278), `refused` (1262-1264), `prompt_missing` (116-118); Test `crates/goalcup/tests/cli_approve.rs`, `cli_folder.rs`.
**Interfaces:** Produces private `fn sentence(said: &str) -> String` in `text.rs`: trims trailing `.` and spaces.
- [ ] Step 1: Failing tests. `cli_approve.rs`: the fake Worker answers `POST /v1/runs` 403 with message `"after the start window you can only continue."`; assert `stop.text` has no `".."`. A Ctrl-C after the open (existing test of `opened_then_stopped`, else call `text::opened_then_stopped("r0123456789abcdef")` directly) must contain `"goalcup continue r0123456789abcdef"` and must not contain `"start again"`. `cli_folder.rs`: `text::prompt_missing(Path::new("prompt.txt"))` contains `"project folder"` (a relative path is taken inside it). `text::repo_not_empty("a/b")` contains `"goalcup continue"`.
- [ ] Step 2: Run, expect FAIL: `cargo nextest run -p goalcup --test cli_approve --test cli_folder`
- [ ] Step 3: Reword: `opened_then_stopped` = "You pressed Ctrl-C after run {run} was opened and saved. {name} was not started. Its deadline keeps running. To go on, run: goalcup continue {run}" (drop "start again opens a new run"; a new start replaces the open run). `repo_not_empty` appends "To go on with a run you already started, run: goalcup continue." `prompt_missing` appends "A relative path is taken inside the project folder; give a path outside it, such as ~/goal-prompt.txt." Pass server messages through `sentence()` in `open_refused`, `approval_refused`, `refused`.
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test cli_approve --test cli_folder --test cli_continue`
- [ ] Step 5: `git add crates/goalcup/src/cli/text.rs crates/goalcup/tests/cli_approve.rs crates/goalcup/tests/cli_folder.rs && git commit -m "fix(cli): refusals name goalcup continue and stop doubling the full stop"`

### Task A.6: `continue` after a Ctrl-C-after-open still shows the push lines   (size M, depends on A.5 only for text)
**Files:** Modify `crates/goalcup/src/cli/start.rs` `first_message()` 414-422 (move the no-prompt branch into `pub(super) fn push_only`); Modify `crates/goalcup/src/cli/continue_run.rs` `first_message()` (326-333) and its caller (line 270); Test `crates/goalcup/tests/cli_continue.rs`.
**Interfaces:** Produces `start::push_only(repo: &str, project: &Path, vars: &BTreeMap<String, String>) -> Vec<String>` (PUSH_FRAME, then the block lines; copies the block to the clipboard). Consumes `ServerJson.repo`.
- [ ] Step 1: Failing test in `cli_continue.rs`: a `World` in competition mode with `repo = "a/b"`, one part with no `first-message.txt`, `.env.local` holding a token in the project; the stdout buffer passed to `prepare` contains `text::PUSH_FRAME`. A second case: practice (repo null) shows no `PUSH_FRAME`. A third: `resumed = true` shows none.
- [ ] Step 2: Run, expect FAIL: `cargo nextest run -p goalcup --test cli_continue the_push_lines_come_back`
- [ ] Step 3: Extract `push_only`; in `continue_run::first_message` fall back to it when the file is missing, `!resumed` and `sj.repo` is `Some`.
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test cli_continue --test cli_start`
- [ ] Step 5: `git add crates/goalcup/src/cli/start.rs crates/goalcup/src/cli/continue_run.rs crates/goalcup/tests/cli_continue.rs && git commit -m "fix(cli): continue shows the push lines when the first message was never written"`

### Task A.7: the first-message path comes back in `status`, and a never-launched part is said plainly   (size S, PARALLEL-OK)
**Files:** Modify `crates/goalcup/src/cli/status.rs` `run()` (~327-331: the unreadable-status branch) and `show()` (one extra line); Modify `text.rs` (add `NOT_LAUNCHED(run)`); Test `crates/goalcup/tests/cli_status.rs`. status.rs is not in the lane brief; the edit is two small hunks and merges clean unless the status owner touches the same lines (index: check).
**Interfaces:** Consumes `text::first_message`-style path wording; the file is `runs/<run>/first-message.txt`.
- [ ] Step 1: Failing tests: (a) a run dir with `server.json` and `first-message.txt` and a recording part: `show()` output contains the file's path; without the file, no such line. (b) a run whose newest part folder does not exist (the Ctrl-C-after-open case): `status` exits 1, stderr contains `"goalcup continue <run>"` and not `SUPPORT_ADDRESS`.
- [ ] Step 2: Run, expect FAIL: `cargo nextest run -p goalcup --test cli_status`
- [ ] Step 3: In `show()` append "First message: <path>" when the file exists and the part is the run's first. In `run()`, when `status.json` is absent and the entry dir does not exist, `fail(&text::not_launched(run))`: "Run {run} was opened but {name} was not started. To go on, run: goalcup continue {run}".
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test cli_status`
- [ ] Step 5: `git add crates/goalcup/src/cli/status.rs crates/goalcup/src/cli/text.rs crates/goalcup/tests/cli_status.rs && git commit -m "fix(cli): status shows the first-message path and a run that never launched"`

### Task A.8: README quick-start uses a prompt path outside the folder   (size S, PARALLEL-OK, doc-only)
**Files:** Modify `README.md` lines 40-44.
- [ ] Step 1: Replace `for example \`prompt.txt\`` with `for example \`~/goal-prompt.txt\`` and `--prompt prompt.txt` with `--prompt ~/goal-prompt.txt` (same value as `PARTICIPANT-GUIDE.md:213,218` and `text::READY`).
- [ ] Step 2: Verify: `grep -n "prompt.txt" README.md` prints nothing; `grep -n "goal-prompt.txt" README.md docs/PARTICIPANT-GUIDE.md` shows the same spelling in both.
- [ ] Step 3: `git add README.md && git commit -m "docs: quick-start prompt path is outside the project folder"`

### Task A.9: window refusal text reads as a sentence   (size S, PARALLEL-OK)
**Files:** Modify `server/worker/src/db-local.ts` lines 115-116; Test `server/worker/test/local-db.test.ts:169`, `server/worker/test/lifecycle.test.ts:100` (they pin the old string).
- [ ] Step 1: Update both tests to the new text first: `BEFORE_WINDOW` = "The start window has not opened yet." (unchanged); `AFTER_WINDOW` = "The start window has closed. A run you already started can still be continued: goalcup continue." and assert in `lifecycle.test.ts` that no message of the two ends with `..` after the client's `sentence()` (the Rust side is A.5).
- [ ] Step 2: Run, expect FAIL: `npm --prefix server/worker test`
- [ ] Step 3: Change the `AFTER_WINDOW` constant only.
- [ ] Step 4: Run, expect PASS: `npm --prefix server/worker test`
- [ ] Step 5: `git add server/worker/src/db-local.ts server/worker/test/local-db.test.ts server/worker/test/lifecycle.test.ts && git commit -m "fix(worker): the after-window refusal is a full sentence that names continue"`

### Task A.10: PROBLEM lines carry an action   (size S, PARALLEL-OK)
**Files:** Modify `crates/goalcup/src/cli/text.rs` `State::Null` (535-540) and `no_request_yet` (620-625); Test `crates/goalcup/tests/cli_status.rs`.
- [ ] Step 1: Failing test: for every `State` variant that renders with the `PROBLEM:` prefix, `state_line(state, run)` contains `"run:"` or `SUPPORT_ADDRESS` or "do nothing" (a loop over the variants the table already builds); `no_request_yet()` contains `"goalcup status"`.
- [ ] Step 2: Run, expect FAIL: `cargo nextest run -p goalcup --test cli_status every_problem_line_says_what_to_do`
- [ ] Step 3: Null: append "You need do nothing. If it stays off, write to {SUPPORT_ADDRESS}." `no_request_yet`: append "Give it a few minutes, then run goalcup status again. If this stays after 10 minutes of work, write to {SUPPORT_ADDRESS}."
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test cli_status`
- [ ] Step 5: `git add crates/goalcup/src/cli/text.rs crates/goalcup/tests/cli_status.rs && git commit -m "fix(cli): every PROBLEM line says what to do"`

### Task A.11: macOS lid and battery note   (size S, PARALLEL-OK)
**Files:** Modify `crates/goalcup/src/cli/awake.rs` `take()` (57-66); `text.rs` add `LID_NOTE`; Test unit test in `awake.rs`.
**Interfaces:** Produces `fn lid_note() -> Option<&'static str>` (`Some` only on macOS).
- [ ] Step 1: Failing unit test `lid_note_is_said_on_a_mac_only`: `assert_eq!(lid_note().is_some(), cfg!(target_os = "macos"))`, and on macOS `take()` with a held lock returns a warning containing "lid".
- [ ] Step 2: Run, expect FAIL (no `lid_note`): `cargo test -p goalcup lid_note_is_said`
- [ ] Step 3: `take()` returns `lid_note()` as the warning when the lock is held on macOS. Text: "On a laptop the lock does not keep the Mac awake on battery or with the lid closed: use mains power and keep the lid open." (`caffeinate -s` is AC-only.)
- [ ] Step 4: Run, expect PASS: `cargo test -p goalcup lid_note_is_said` and `cargo nextest run -p goalcup --test cli_start`
- [ ] Step 5: `git add crates/goalcup/src/cli/awake.rs crates/goalcup/src/cli/text.rs && git commit -m "feat(cli): say plainly that a Mac sleeps on battery or with the lid closed"`

### Task A.12: setup without a terminal does not say Ready   (size S, PARALLEL-OK)
**Files:** Modify `crates/goalcup/src/cli/setup.rs` 79-98 (the `else` branch at 84-87 and `fix == None` arm at 92-94); `text.rs` add `SETUP_NEEDS_TERMINAL`; Test `crates/goalcup/tests/cli_setup.rs:134`.
- [ ] Step 1: Change `no_terminal_asks_nothing_opens_nothing_writes_nothing`: expect exit 1, `all` contains `"goalcup setup"` and not `"Ready."`; snapshot still unchanged and no harness ran.
- [ ] Step 2: Run, expect FAIL (exit 0, "Ready." printed): `cargo nextest run -p goalcup --test cli_setup no_terminal_asks_nothing`
- [ ] Step 3: When neither `yes` nor `ask`, print the harness-found line and `fail(SETUP_NEEDS_TERMINAL)`: "Nothing was written and sign-in was not checked. Run goalcup setup in a terminal to sign in to {name}, or goalcup setup --yes to write its settings only."
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test cli_setup`
- [ ] Step 5: `git add crates/goalcup/src/cli/setup.rs crates/goalcup/src/cli/text.rs crates/goalcup/tests/cli_setup.rs && git commit -m "fix(cli): setup without a terminal no longer claims Ready"`

### Task A.13: parse errors keep clap's tip, and internal flags leave the help   (size S, PARALLEL-OK)
**Files:** Modify `crates/goalcup/src/main.rs` `parse_error()` (144-149) and the `test`, `connector`, `hosts` args of `Start`/`Continue` (`hide = true`); Test `main.rs` `mod tests` and `tests/cli_commands.rs`.
**Interfaces:** Produces private `fn one_line(rendered: &str) -> String`: first line, then ` (tip: ...)` from the first line whose trimmed text starts `tip:`.
- [ ] Step 1: Failing tests. In `main.rs`: `Args::try_parse_from(["goalcup","start","--hrness","x"])` error, `one_line(&e.to_string())` contains `"--harness"` and has no `\n`. In `cli_commands.rs`: `start --help` stdout contains none of `--test`, `--connector`, `--hosts`; `one_error_line` still holds for a typo'd flag.
- [ ] Step 2: Run, expect FAIL: `cargo test -p goalcup one_line_keeps_the_tip` and `cargo nextest run -p goalcup --test cli_commands`
- [ ] Step 3: Implement `one_line` and add `hide = true`.
- [ ] Step 4: Run, expect PASS: same commands.
- [ ] Step 5: `git add crates/goalcup/src/main.rs crates/goalcup/tests/cli_commands.rs && git commit -m "fix(cli): parse errors keep the did-you-mean, internal flags leave the help"`

### Task A.14: `time-left` falls back to the run's deadline   (size S, PARALLEL-OK)
**Files:** Modify `crates/goalcup/src/cli/time_left.rs` (`line` signature 68, callers 38, 58); Test `crates/goalcup/tests/cli_time_left.rs` (calls at lines 15, 31, 32).
**Interfaces:** Produces `pub fn line(upload: Option<&UploadJson>, practice: bool, deadline: Option<u64>, now: u64) -> Result<String, String>`; `deadline` is `ServerJson.deadline` via `unix_of` (the same value `status` shows).
- [ ] Step 1: Failing test: no upload word and `deadline = Some(T0 + 7200)`: `line(None, false, Some(T0 + 7200), T0)` is `Ok` and contains "stops on"; with `deadline = Some(T0 - 5)` it is `Ok(TIME_IS_UP)`; with `None` still `Err`. Update the three existing calls to pass `None`.
- [ ] Step 2: Run, expect FAIL (compile error then wrong result): `cargo nextest run -p goalcup --test cli_time_left`
- [ ] Step 3: In `line`, when no fresh word: use `deadline` (`TIME_IS_UP` if `<= now`, else `text::time_left`); otherwise the unknown error. Callers pass `run.deadline.as_deref().and_then(unix_of)`. The agent inside the harness gets the same answer.
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test cli_time_left`
- [ ] Step 5: `git add crates/goalcup/src/cli/time_left.rs crates/goalcup/tests/cli_time_left.rs && git commit -m "fix(cli): time-left uses the run's deadline when the server's word is stale"`

### Task A.15: `continue` refuses a different folder   (size S, PARALLEL-OK)
**Files:** Modify `crates/goalcup/src/cli/continue_run.rs` `prepare()` after the `(sj, last)` split (~line 124); `text.rs` add `wrong_folder(run, folder)`; Test `crates/goalcup/tests/cli_continue.rs`.
- [ ] Step 1: Failing tests. A `World` whose last part has `"workspace": <project>`; `prepare` from another directory is `Err` with code 1, text contains the recorded folder path, and nothing was written (`folders()` unchanged, `server.json` bytes equal). The same part with no `workspace` key (as every existing test) still prepares. Paths are compared after `canonicalize`.
- [ ] Step 2: Run, expect FAIL: `cargo nextest run -p goalcup --test cli_continue a_different_folder`
- [ ] Step 3: Compare `std::fs::canonicalize` of both, only when the recorded `workspace` is `Some`. Text: "Run {run} was started in \"{folder}\". Run goalcup continue there."
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test cli_continue`
- [ ] Step 5: `git add crates/goalcup/src/cli/continue_run.rs crates/goalcup/src/cli/text.rs crates/goalcup/tests/cli_continue.rs && git commit -m "fix(cli): continue refuses to launch the harness in a different folder"`

### Task A.16: plain words in terminal text   (size S, PARALLEL-OK)
**Files:** Modify `crates/goalcup/src/cli/text.rs`: masking sentence (322, "Goal Cup Proxy" to "goalcup"), `gap_words` "pinned" (464) and `no_request_written` (746-752) ("pins" to "passes it through unread"). Test `crates/goalcup/tests/cli_status.rs`.
- [ ] Step 1: Failing test: `text::masking(None)`, `text::gap_words("pinned")` and `text::no_request_written("Agent", Some("h.example"))` contain none of `"Goal Cup Proxy"`, `"pinned"`, `"pins"`. READY_MARK stays: the pad waits for it (Dropped).
- [ ] Step 2: Run, expect FAIL: `cargo nextest run -p goalcup --test cli_status no_jargon`
- [ ] Step 3: Reword the three strings. The product is "goalcup" in commands and "Goal Cup" for the event only.
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test cli_status`; also `grep -rn "Goal Cup Proxy" crates/goalcup/src/cli` prints nothing.
- [ ] Step 5: `git add crates/goalcup/src/cli/text.rs crates/goalcup/tests/cli_status.rs && git commit -m "fix(cli): plain words in terminal text"`

## Open owner questions
1. Refuse a competition `POST /v1/device` outside the start window (so an early start never spends a browser approval)? Needs a Worker change in the upload-server lane.
2. Rename the ready line "Recording in chat mode" (needs `scripts/pad/pad-run` and `test_pad.py` changed in the same release)?
3. `setup` without a terminal now exits 1 (A.12). Any script of yours that calls bare `goalcup setup` without `--yes`?

## Cross-lane notes
- Lane F: `text.rs` 176-182 and `tier_line` untouched; A.5 and A.16 edit other functions in the same file (merge-clean if F keeps to its ranges).
- Lane E: A.4 reads `builtin::CONNECTORS` and `schema::parse` only.
- cr-upload-server: A.9 changes the Worker message; Rust tests that match the old string live in `server/worker/test` only.
- status.rs: A.7 touches `run()` and `show()`; ask the index who else edits them.
