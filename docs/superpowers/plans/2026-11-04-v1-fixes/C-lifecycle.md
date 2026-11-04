# Lane C: Watchdog, recover, macOS lifecycle tests, installer during a live run
**Wave:** 1   **Depends on:** none (C.11 runs last, after lanes A, E and F merge)   **Branch:** fix/lifecycle
**Covers findings:** cr-watchdog-1, arch-x1, cr-watchdog-2, cr-watchdog-3, cr-watchdog-7, arch-10, cr-watchdog-8, cr-watchdog-9, arch-1, arch-5, cr-scripts-2, cr-scripts-5, cr-scripts-6, MAC-LEARNINGS 7h
**Owns files:** crates/goalcup/src/watchdog/{mod,recover,spawn}.rs, crates/goalcup/src/cli/recover.rs (untouched unless C.11 needs it), crates/goalcup/src/cli/seal.rs (untouched: its dead-entry path already closes what C.3 leaves), crates/goalcup/src/pipeline/mod.rs, crates/goalcup/tests/{watchdog_end,watchdog_recover,watchdog_null,cli_recover}.rs, crates/goalcup/tests/watchdog_share/mod.rs, packaging/install.sh, scripts/test_install.py, docs/ARCHITECTURE.md line 95 only
**Shared files (only these parts):** packaging/README.md lines 32-34 (the `INSTALL_UNSURE` sentence, C.8). C.11 only: pipeline/boot.rs `start_event` lines 248 and 255-256; cli/start.rs StartFacts literal lines 375-376 and 382, `Part.local_wall` lines 316 and 345; cli/recover.rs:220; tests lessons_tls.rs:308,314, lessons_launch.rs:181,187, mask_values.rs:800,806 (one-line deletions each). scripts/gate.sh: no edit (see C.1).
**Produces (for other lanes):** the pinned exe: `crate::watchdog::EXE_FILE: &str = "goalcup"` (the file `<entry>/rec/goalcup`, present while a watchdog of that entry lives, removed when it ends) and `crate::watchdog::spawn::pin_exe(exe: &Path, rec: &Path) -> io::Result<PathBuf>`. Lane G (pad) and any lane that lists or copies `rec/` must skip `rec/goalcup` (it is never a part file: `part_files` matches `rec-*.gcr.zst` only). Lane L: ARCHITECTURE:95 text is C.7's.
**Consumes:** none

## Dropped
- arch-5: cosmetic. The recovered watchdog is dispatched by its `recover` first line (pipeline/mod.rs:798-802), `status` and `seal` read pid files, never process names; a second hidden subcommand adds a dispatch path and a test for a different `ps` line only.
- MAC-LEARNINGS 7h: by design, not a loss. With the watchdog gone the recorder records on and seals itself at a server stop, the deadline or the harness exit (pipeline/mod.rs:1001-1023); `goalcup seal` reaches it by SIGTERM (cli/seal.rs:47-51); if it then dies too, `goalcup recover` applies (it refuses only while a recorder lives). A "watchdog gone" status line needs pipeline/status.rs, outside this lane: owner question 3.
- cr-watchdog-1, install trigger on Linux: not real. `current_exe` is read once (watchdog/mod.rs:269) and install.sh renames a new file onto that path (install.sh:326-328), so a respawn runs the new binary (a version mix, fixed by C.2), not ENOENT. The wedge stays for any other spawn error (EMFILE, EAGAIN, ENOMEM): C.3.
- arch-x1, "version in the first control line, refuse a mismatch": cut. After C.2 every boot of an entry runs the same file.
- cr-watchdog-9, 256-byte values: already covered by mask_values.rs `the_start_line_with_32_long_values_fits` (mask/values.rs:611). The `--hosts` half stays: C.6.

## Review focus
1. The pin on another volume (EXDEV) or a full disk: the copy fails and the run must go on from the installed path. Test: C.2 `pin_exe_fails_cleanly`.
2. A recorder that cannot be spawned while the harness runs must never end the watchdog early, and must not wedge it after. Test: C.3 asserts the give-up comes only after the harness exit.
3. Reaping adopted zombies must never take the harness's or the recorder's exit status. Test: C.5 asserts the sealed `harness.exit` is `{"code":0}`.
4. A hung `status --json` during an install with a live run. Test: C.8.
5. A `goalcup start` of the same harness while a recovered watchdog runs must be refused (HomeLocked), not write into the live home. Test: C.4.

### Task C.1: The lifecycle suites run on macOS   (M, PARALLEL-OK)
**Files:** Modify crates/goalcup/tests/watchdog_end.rs (line 5-6 header and cfg, `gone` 26-31, orphan asserts in the seal-command and deadline tests), watchdog_recover.rs:5, cli_recover.rs:5, watchdog_share/mod.rs (`loopback_only` 53-70, new `gone`). lessons_launch.rs and pipeline_stall.rs stay Linux-only (TIOCSCTTY, `/proc/self/status`, `/proc/uptime`; not named by the finding). scripts/gate.sh needs no edit: its `cargo nextest run --locked --workspace` already runs every test binary on the Mac; the `#![cfg(target_os = "linux")]` lines are the only gate.
**Interfaces:** Produces (tests only) `watchdog_share::gone(pid: u32) -> bool`: Linux `/proc/<pid>/stat` missing or state `Z`; elsewhere `ps -o stat= -p <pid>` prints nothing or starts with `Z`. `loopback_only(port)`: Linux as now; elsewhere `lsof -nP -iTCP:<port> -sTCP:LISTEN -Fn` lines starting `n` equal `["n127.0.0.1:<port>"]`.
- [ ] Step 1: Delete the three crate-level `#![cfg(target_os = "linux")]` lines. Move `gone` from watchdog_end.rs to watchdog_share with the macOS branch. In `watchdog_end_stub_seal_command_ends_the_tree_only` and `watchdog_end_real_deadline_ends_the_tree`, check `orphan` only under `if cfg!(target_os = "linux")` (macOS has no subreaper, spawn.rs:156-166). Header line 5 becomes "The orphan checks are Linux only (the subreaper)."
- [ ] Step 2: On the owner's Mac before Step 1: `PATH="$HOME/.cargo/bin:$PATH" cargo nextest run -p goalcup --test watchdog_end --test watchdog_recover --test cli_recover` expect FAIL: "no tests to run" (exit 4).
- [ ] Step 3: Implement `gone` and `loopback_only` as above.
- [ ] Step 4: Same command on the Mac, expect PASS (every test in the three binaries); `cargo nextest run -p goalcup --test watchdog_end --test watchdog_recover --test cli_recover` on Linux, expect PASS.
- [ ] Step 5: `git add crates/goalcup/tests/watchdog_end.rs crates/goalcup/tests/watchdog_recover.rs crates/goalcup/tests/cli_recover.rs crates/goalcup/tests/watchdog_share/mod.rs; git commit -m "test(watchdog): end, recover and cli recover run on macOS"`

### Task C.2: Pin the recorder exe per run   (M)
**Files:** Modify crates/goalcup/src/watchdog/spawn.rs (new fn + unit tests), watchdog/mod.rs `run` (268-273) and a new `pub const EXE_FILE`, watchdog/recover.rs `recover` (64-67) and `run` (after `watch`, line 174). Test: spawn.rs `#[cfg(test)] mod tests`.
**Interfaces:** Produces `pub const EXE_FILE: &str = "goalcup";` in watchdog/mod.rs; `pub fn pin_exe(exe: &Path, rec: &Path) -> io::Result<PathBuf>` in spawn.rs: `fs::hard_link(exe, rec/goalcup.tmp)`, on any error `fs::copy` (keeps mode 0755), then `fs::rename` to `rec/goalcup`; removes the tmp on failure; returns `rec/goalcup`.
- [ ] Step 1: Write the failing tests in spawn.rs:
```rust
#[test]
fn pin_exe_survives_a_replace() {
    let t = tempfile::tempdir().unwrap();
    let (exe, rec) = (t.path().join("goalcup"), t.path().join("rec"));
    std::fs::create_dir(&rec).unwrap();
    std::fs::write(&exe, b"old").unwrap();
    let pinned = pin_exe(&exe, &rec).unwrap();
    assert_eq!(pinned, rec.join(super::super::EXE_FILE));
    std::fs::write(t.path().join("new"), b"new").unwrap();
    std::fs::rename(t.path().join("new"), &exe).unwrap(); // install.sh:328
    assert_eq!(std::fs::read(&pinned).unwrap(), b"old");
    pin_exe(&exe, &rec).unwrap(); // recover pins again
    assert_eq!(std::fs::read(&pinned).unwrap(), b"new");
    assert!(!rec.join("goalcup.tmp").exists());
}
#[test]
fn pin_exe_fails_cleanly() {
    let t = tempfile::tempdir().unwrap();
    let exe = t.path().join("goalcup");
    std::fs::write(&exe, b"x").unwrap();
    assert!(pin_exe(&exe, &t.path().join("no-such-dir")).is_err());
}
```
- [ ] Step 2: `cargo test -p goalcup --lib pin_exe` expect FAIL: `pin_exe` not found.
- [ ] Step 3: Implement `pin_exe`. `run`: after `current_exe`, `make_rec(&entry_dir/rec)`, then `pin_exe(&exe, &rec)`; on Err log `exe_pin_failed` and use `exe`; call `run_with` with the result; after it returns, `fs::remove_file(rec/EXE_FILE)` (ignore errors). `recover`: pin `current_exe` into `entry_dir/rec` the same way before `recover_with` (the recovered watchdog's own `current_exe` at recover.rs:145 is then the pin, so its boots use it); recover.rs `run` removes `rec/EXE_FILE` after `watch` returns. `run_with` and `recover_with` keep their signatures (tests pass wrappers).
- [ ] Step 4: `cargo test -p goalcup --lib pin_exe` expect PASS; `cargo nextest run -p goalcup --test watchdog_recover --test watchdog_end` expect PASS.
- [ ] Step 5: `git add crates/goalcup/src/watchdog/spawn.rs crates/goalcup/src/watchdog/mod.rs crates/goalcup/src/watchdog/recover.rs; git commit -m "fix(watchdog): every boot of a run starts the binary the run started with"`

### Task C.3: A recorder that cannot be spawned never wedges the watchdog   (M)
**Files:** Modify watchdog/mod.rs:563-582 (the inner `loop` in `watch`) and the consts near 232-260. Test: crates/goalcup/tests/watchdog_end.rs.
**Interfaces:** Produces `const SPAWN_GIVE_UP_NS: u64 = 30 * SEC;` and log words `recorder_spawn_failed` (once per failing spell, as now) and `recorder_spawn_gave_up`.
- [ ] Step 1: Write the failing test `watchdog_end_real_unspawnable_recorder_ends_after_the_harness`: the recorder is `d/rec.sh` = `#!/bin/sh\nrm -f "$0"\nexec "<CARGO_BIN_EXE_goalcup>" "$@"` (mode 0755), so every boot after the first fails with ENOENT. Driver: `until` status.json `pid` > 0 and `counts.jsonl` non-empty; SIGKILL that pid; `until` goalcup.log contains `recorder_spawn_failed`; sleep 2 s; note `stopped = Instant::now()`; touch `d/stop`. After `f.run` returns: `assert!(stopped.elapsed() >= Duration::from_secs(29))`, `assert!(stopped.elapsed() < Duration::from_secs(60))`, goalcup.log contains `recorder_spawn_gave_up`, `!cli::watchdog_alive(&f.entry_dir)`, and `goalcup::pipeline::close_dead_entry(&f.entry_dir)` is `Ok(Closed::Sealed(_))`. Kill `child.pid` and `orphan.pid` at the end as the other tests do. Run `f.run` on a thread and `recv_timeout(WAIT)` its end, so the wedge fails instead of hanging.
- [ ] Step 2: `cargo nextest run -p goalcup --test watchdog_end watchdog_end_real_unspawnable_recorder_ends_after_the_harness` expect FAIL: timed out (the loop at mod.rs:569 never breaks).
- [ ] Step 3: Implement: one `Boot::start` try per tick in place of the inner `loop`. On Ok: `boot = next; boots += 1;` clear `failing_since`. On Err: log once, set `failing_since` if None, leave the ended `boot` in place (its `next()` sleeps one tick on a closed channel). While `harness_pid.is_some()` keep trying; once it is None and `failing_since` is older than `SPAWN_GIVE_UP_NS`, log `recorder_spawn_gave_up` and set `left = true` (the entry goes to `close_dead_entry`, as a failed null seal does).
- [ ] Step 4: Same command, expect PASS; `cargo nextest run -p goalcup --test watchdog_end --test watchdog_restart --test watchdog_null` expect PASS (Linux), `--test watchdog_end` on the Mac.
- [ ] Step 5: `git add crates/goalcup/src/watchdog/mod.rs crates/goalcup/tests/watchdog_end.rs; git commit -m "fix(watchdog): a recorder that cannot start no longer stops the harness watch"`

### Task C.4: A recovered watchdog holds the home lock   (S)
**Files:** Modify watchdog/recover.rs `run` (after `write_pid`, line 142-144). Test: crates/goalcup/tests/watchdog_recover.rs.
**Interfaces:** Consumes `connector::HomeLock::take` (pub(crate), launch.rs:44) and `cli::isolated_home(home, id)`. The home is `facts.entry_dir.ancestors().nth(3)` (`$GOALCUP_HOME/runs/<run>/<entry>`), the id `facts.connector["id"]`.
- [ ] Step 1: Write the failing test `watchdog_recover_holds_the_home_lock`: `let mut d = setup(false);` `recover_with(...)` is `Ok`; open `d.tmp.path().join("g/homes/fake.goalcup.lock")` read+write+create; `assert!(matches!(f.try_lock(), Err(std::fs::TryLockError::WouldBlock)))`; kill and wait `d.harness`; `until` the sealed status.json; `until("the lock freed", || f.try_lock().ok())`.
- [ ] Step 2: `cargo nextest run -p goalcup --test watchdog_recover watchdog_recover_holds_the_home_lock` expect FAIL: `try_lock` returns Ok.
- [ ] Step 3: Implement: `let _home = HomeLock::take(&cli::isolated_home(home, id))`, held to the end of `run`; on `Err` (a start took it in the gap after `rebuild` dropped its lock, cli/recover.rs:195) log `home_locked` and go on.
- [ ] Step 4: Same command, expect PASS (Linux and Mac).
- [ ] Step 5: `git add crates/goalcup/src/watchdog/recover.rs crates/goalcup/tests/watchdog_recover.rs; git commit -m "fix(recover): the recovered watchdog holds the harness home lock"`

### Task C.5: Reap orphans the subreaper adopts (Linux)   (S)
**Files:** Modify watchdog/spawn.rs (new fn beside `end_tree`), watchdog/mod.rs `watch` loop. Test: watchdog_end.rs.
**Interfaces:** Produces `pub fn reap_orphans(keep: &dyn Fn(u32) -> bool)`: Linux only (no-op elsewhere); `waitpid(pid, WNOHANG)` for each `procs()` entry with `ppid == me`, `zombie`, `!keep(pid)`. `const REAP_NS: u64 = 10 * SEC;`
- [ ] Step 1: Write the failing test, `#[cfg(target_os = "linux")]` on the fn, `watchdog_end_stub_adopted_orphans_are_reaped`: a plan from `f.plan()` with `argv[1]` = `( sleep 0.2 & echo $! > {d}/quick.pid ); echo $$ > {d}/harness.pid; while [ ! -e {d}/stop ]; do sleep 0.05; done`. Driver: `until` quick.pid; `until("reaped", || (!Path::new(&format!("/proc/{pid}")).exists()).then_some(()))`; touch stop. After: `f.sealed_status()["harness"]["exit"] == json!({"code": 0})`.
- [ ] Step 2: `cargo nextest run -p goalcup --test watchdog_end watchdog_end_stub_adopted_orphans_are_reaped` expect FAIL: "timed out waiting for reaped" (the zombie stays until `end_tree`).
- [ ] Step 3: Implement: in `watch`, while `own.is_some() && harness_pid.is_some() && after.is_none()`, every `REAP_NS` of boot time call `spawn::reap_orphans(&|p| Some(p) == harness_pid || spare(p))`. `before` helpers are done by then and `after` helpers have not started, so the watchdog's only own children are the harness and the recorder.
- [ ] Step 4: Same command, expect PASS; full `--test watchdog_end` expect PASS.
- [ ] Step 5: `git add crates/goalcup/src/watchdog/spawn.rs crates/goalcup/src/watchdog/mod.rs crates/goalcup/tests/watchdog_end.rs; git commit -m "fix(watchdog): reap adopted orphans while the harness runs"`

### Task C.6: A start line too long is refused before anything starts   (S, PARALLEL-OK)
**Files:** Modify watchdog/mod.rs `run_with` (after `facts.ca = ca.export();`, line 304). Test: watchdog_end.rs.
**Interfaces:** Produces `fn start_line_fits(facts: &StartFacts) -> bool` (the serialized `ToRecorder::Start` plus newline is at most `MAX_LINE`) and the log word `watchdog_failed` `why=start_line`.
- [ ] Step 1: Write the failing test `watchdog_end_stub_oversized_start_line_starts_nothing`: `let big: String = (0..4000).map(|i| format!("\"h{i}.example.org\", ")).collect();` `let hosts = HostList::from_file(builtin::HOSTS.replacen("provider = [", &format!("provider = [{big}"), 1).as_bytes(), "big.toml").unwrap();` facts from `f.facts(&plan)` with `facts.hosts = to_value(&hosts).unwrap()`; `f.run_plan(plan, facts, &f.stub(), || {})`. Assert `!f.m("harness.pid").exists()` and goalcup.log contains `"start_line"`.
- [ ] Step 2: `cargo nextest run -p goalcup --test watchdog_end watchdog_end_stub_oversized_start_line_starts_nothing` expect FAIL: goalcup.log says `no_ready` (the recorder dropped the line, mod.rs:912).
- [ ] Step 3: Implement: `if !start_line_fits(&facts) { return failed("start_line"); }` before `write_key`.
- [ ] Step 4: Same command, expect PASS.
- [ ] Step 5: `git add crates/goalcup/src/watchdog/mod.rs crates/goalcup/tests/watchdog_end.rs; git commit -m "fix(watchdog): refuse a start line the recorder could not read"`

### Task C.7: A null-mode retry lets live traffic finish; ARCHITECTURE says what happens   (S, PARALLEL-OK)
**Files:** Modify pipeline/mod.rs:995 and a new method on `Run`; docs/ARCHITECTURE.md:95. Test: crates/goalcup/tests/watchdog_null.rs (Linux).
**Interfaces:** Produces `const QUIET_NS: u64 = 1_000_000_000; const NULL_DRAIN_NS: u64 = 8_000_000_000;` (under the watchdog's `RESTART_KILL_NS` 10 s, mod.rs:254) and `fn drain_quiet(&self)`: returns once `self.rec.now() - proxy.quiet_since() >= QUIET_NS` or after `NULL_DRAIN_NS`, polling every 100 ms. A real drain (stop accepting, overlap with the next boot) needs the proxy's accept loop (lane A/proxy files) and is post-event (REVIEW Architecture 6).
- [ ] Step 1: Write the failing test `watchdog_null_retry_lets_a_live_stream_finish`: `setup()`, run the watchdog as `watchdog_null_crash_loop_serves_and_retries` does; `crash_to_null(&dir)`; a local `TcpListener` thread that accepts once and writes `b"x"` 30 times, 200 ms apart, then closes; sleep 26 s; CONNECT through the proxy port to it (as `through_proxy`, without the GET) and read to EOF. `assert_eq!(got.len(), 30)`.
- [ ] Step 2: `cargo nextest run -p goalcup --test watchdog_null watchdog_null_retry_lets_a_live_stream_finish` expect FAIL: fewer than 30 bytes (cut by the exit at pipeline/mod.rs:995).
- [ ] Step 3: Implement: `Ok(Some(ToRecorder::Restart)) if self.null => { self.drain_quiet(); return ExitCode::SUCCESS; }`. ARCHITECTURE.md:95: replace "The harness keeps its connections." with "Each retry waits up to 8 seconds for a moment with no traffic; a connection still open then is cut, and the harness reconnects to the next recorder on the same port."
- [ ] Step 4: Same command, expect PASS; `grep -c "keeps its connections" docs/ARCHITECTURE.md` prints 0.
- [ ] Step 5: `git add crates/goalcup/src/pipeline/mod.rs crates/goalcup/tests/watchdog_null.rs docs/ARCHITECTURE.md; git commit -m "fix(recorder): a null-mode retry waits for a quiet moment; doc says what is cut"`

### Task C.8: The installer refuses when `status` hangs   (S, PARALLEL-OK)
**Files:** Modify packaging/install.sh:145-176; packaging/README.md:32-34. Test: scripts/test_install.py:302-310.
**Interfaces:** none. Keeps `INSTALL_UNSURE:` as the line prefix; only the hung case stops (exit 1) and names `--force`. Other unsure answers (exit 3, a goalcup from before 1.0) go on as now: after C.2 a replace no longer breaks a run that 1.0 started.
- [ ] Step 1: Replace `test_a_hanging_status_is_stopped_after_10_seconds_and_gives_the_unsure_line` by `test_a_hanging_status_is_stopped_after_10_seconds_and_refuses` (same prefix, AGENTS.md "Tests"): `prefix, _, before = self.guarded(hang=True)`; `assertEqual(result.returncode, 1)`; `assertIn(UNSURE, result.stderr)`; `assertIn("--force", result.stderr)`; `assertEqual(self.tree(prefix), before)`; the 9.5 s / 25 s bounds as now.
- [ ] Step 2: `python3 -m unittest scripts/test_install.py -k test_a_hanging_status` expect FAIL: returncode 0.
- [ ] Step 3: Implement: set `hung=1` in the kill branch (line 148-151); in the `unsure)` arm, when `hung=1` print `INSTALL_UNSURE: goalcup status did not answer in 10 seconds, so a run may be recording. Nothing was changed. Update after the run ends, or run the installer again with: curl -fsSL https://github.com/agentcommunity/goal-cup-proxy/releases/latest/download/install.sh | sh -s -- --force` and `exit 1`. README:33: "no answer within 10 s" moves to its own sentence: it stops, like `INSTALL_BLOCKED`, and names `--force`.
- [ ] Step 4: `python3 -m unittest scripts/test_install.py` expect PASS.
- [ ] Step 5: `git add packaging/install.sh packaging/README.md scripts/test_install.py; git commit -m "fix(install): a status that hangs stops the update"`

### Task C.9: wget keeps https and TLS 1.2   (S, PARALLEL-OK)
**Files:** Modify packaging/install.sh:67-68. Test: scripts/test_install.py (new test).
**Interfaces:** none.
- [ ] Step 1: Write the failing test `test_wget_keeps_https_and_tls12`: a bin dir with symlinks to `sh uname mktemp rm grep` (and `sysctl`, `ldd` when `shutil.which` finds them) and a fake `wget` script `echo "$@" > <log>; exit 1`; run `sh install.sh` with `PATH=<that dir>`, `HOME`, `GOALCUP_RELEASE_BASE=https://example.invalid/r`. Assert returncode 1, and the log holds `--https-only` and `--secure-protocol=TLSv1_2`.
- [ ] Step 2: `python3 -m unittest scripts/test_install.py -k test_wget_keeps_https` expect FAIL: flags missing.
- [ ] Step 3: Implement: `case "$1" in https://*) wget -q --https-only --secure-protocol=TLSv1_2 -O "$2" "$1" ...;; *) wget -q -O "$2" "$1" ...;; esac`, mirroring the curl arm (lines 63-66).
- [ ] Step 4: `python3 -m unittest scripts/test_install.py` expect PASS.
- [ ] Step 5: `git add packaging/install.sh scripts/test_install.py; git commit -m "fix(install): the wget fallback keeps https and TLS 1.2"`

### Task C.10: fish gets a line fish runs   (S, PARALLEL-OK)
**Files:** Modify packaging/install.sh `path_line` (216-230). Test: scripts/test_install.py (new test).
**Interfaces:** none.
- [ ] Step 1: Write the failing test `test_fish_gets_fish_add_path`: `self.install(make_release(self.scratch()), SHELL="/opt/homebrew/bin/fish")`; returncode 0; `assertIn("fish_add_path $HOME/.local/bin", result.stdout)`; `assertNotIn("export PATH", result.stdout)`.
- [ ] Step 2: `python3 -m unittest scripts/test_install.py -k test_fish_gets` expect FAIL.
- [ ] Step 3: Implement: before the `case ${SHELL:-}` profile arm, `*/fish) say "  fish_add_path $dir"; return 0 ;;` (fish 3.2+ keeps it for new shells itself).
- [ ] Step 4: `python3 -m unittest scripts/test_install.py` expect PASS.
- [ ] Step 5: `git add packaging/install.sh scripts/test_install.py; git commit -m "fix(install): fish users get fish_add_path"`

### Task C.11: Drop the always-null `build` and `server_time`   (S, last: after lanes A, E and F merge)
**Files:** Modify watchdog/mod.rs:142-155 (fields `server_time`, `local_wall`, `build`) and its test literal 1105-1111; pipeline/mod.rs:706-712; pipeline/boot.rs:248 (`"build"`) and 255-256 (`server_time`, `local_wall_at_server_time`); cli/start.rs:375-376, 382 and `Part.local_wall` (316, 345); cli/recover.rs:220; test literals watchdog_end.rs:123-129, lessons_launch.rs:181-187, lessons_tls.rs:308-314, mask_values.rs:800-806.
**Interfaces:** the `start` event loses keys `build`, `server_time`, `local_wall_at_server_time` (readers ignore them; no server sends a time). Lane D: RECORD-FORMAT.md:256 example and goalcup-verify/tests/common/mod.rs:729-730 may keep or drop them, both read fine.
- [ ] Step 1: Write the check: `grep -rnE "server_time|local_wall_at|build: (None|Some)|\"build\"" crates/goalcup/src crates/goalcup/tests` must print nothing.
- [ ] Step 2: Run it, expect FAIL (the lines above).
- [ ] Step 3: Delete the fields, the keys and every literal line; `cargo clippy --workspace --all-targets -- -D warnings` names any reader left.
- [ ] Step 4: The grep prints nothing; `sh scripts/gate.sh fast` expect PASS.
- [ ] Step 5: `git add -u crates/goalcup; git commit -m "chore: drop the start facts no one fills"`
